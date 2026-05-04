// Copyright 2026 The Eigenius Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Julia mirror generator — substrate Rust code that walks the
//! ontology layer and emits Julia struct source matching the
//! D27 §3.3 faithful-translation specification.
//!
//! ## Phase 19a.3.a scope (this commit)
//!
//! - **Class-walking pass.** From a seed of class IRIs, transitively
//!   collect all reachable classes (via resource-typed properties'
//!   `class_types`) and topologically sort so structs can be emitted
//!   in dependency order.
//! - **Per-class struct emitter.** Required properties → `field::Type`;
//!   recommended properties → `field::Union{Type, Nothing}`; type
//!   resolution per the D27 §3.3 mapping table.
//! - **Single-module output.** All structs in one Julia module file
//!   `EigeniusMirror.jl`. Subclass relationships and split-module
//!   layouts are deferred — flat ontologies (the kinase fixture)
//!   work fully.
//! - **Determinism.** Same input produces byte-identical output;
//!   property ordering is the BTreeMap order from the kernel's
//!   canonical Resource representation.
//!
//! ## Deferred to later sub-milestones
//!
//! - **19a.3.b**: `decode_*` / `encode_*` codec emitters; format-
//!   constraint validation in inner constructors;
//!   `EigeniusJuliaCommon` shared helpers.
//! - **19a.3.c**: `JuliaPackageMirror` chain commit; image-build
//!   wiring; precompile in env image.
//!
//! ## Why one module
//!
//! D27 §3 frames the mirror as a Julia *package* with one struct per
//! class. v1 collapses to one module file because Julia parses
//! per-file as a unit and one file is the simplest deterministic
//! emission target. Splitting per-class lands when the closure is
//! large enough that one file is unwieldy (not the kinase case).

use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_runtime_substrate::mirror_generator::{
    LibraryContent, LibraryFile, MirrorGenerationOutput, MirrorGenerationRequest, MirrorGenerator,
    MirrorGeneratorError,
};
use std::collections::{BTreeMap, BTreeSet};

const GENERATOR_ID: &str = "eigon-julia-gen";
const TARGET_MODULE_NAME: &str = "EigeniusMirror";
const TARGET_FILE_PATH: &str = "src/EigeniusMirror.jl";

// Core ontology IRIs the generator reads. Pinned as constants so a
// chain rename of the core ontology surfaces as a compile-time edit
// rather than a silent runtime drift.
const PROP_SHORT_NAME: &str = "urn:eigenius:core:short_name";
const PROP_REQUIRES: &str = "urn:eigenius:core:requires";
const PROP_RECOMMENDS: &str = "urn:eigenius:core:recommends";
const PROP_DATA_TYPE: &str = "urn:eigenius:core:data_type";
const PROP_CLASS_TYPES: &str = "urn:eigenius:core:class_types";
const PROP_ELEMENT_TYPE: &str = "urn:eigenius:core:element_type";
const PROP_MIN_VALUE: &str = "urn:eigenius:core:min_value";
const PROP_MAX_VALUE: &str = "urn:eigenius:core:max_value";
const PROP_MIN_LENGTH: &str = "urn:eigenius:core:min_length";
const PROP_MAX_LENGTH: &str = "urn:eigenius:core:max_length";
const PROP_PATTERN: &str = "urn:eigenius:core:pattern";
const PROP_FORMAT: &str = "urn:eigenius:core:format";

const TYPE_STRING: &str = "urn:eigenius:core:string";
const TYPE_INTEGER: &str = "urn:eigenius:core:integer";
const TYPE_FLOAT: &str = "urn:eigenius:core:float";
const TYPE_BOOLEAN: &str = "urn:eigenius:core:boolean";
const TYPE_RESOURCE: &str = "urn:eigenius:core:resource";
const TYPE_RESOURCE_ARRAY: &str = "urn:eigenius:core:resource_array";
const TYPE_VALUE_ARRAY: &str = "urn:eigenius:core:value_array";
const TYPE_JSON: &str = "urn:eigenius:core:json";

/// Property IRI we stamp on every encoded resource so the receiver can
/// re-validate the class. Mirrors the kernel's `is_a` convention.
const PROP_IS_A: &str = "urn:eigenius:core:is_a";

/// Prefix for format IRIs in the core ontology. Format IRIs end in
/// the format short_name (e.g. `urn:eigenius:core:formats:date` →
/// `:date`).
const FORMAT_IRI_PREFIX: &str = "urn:eigenius:core:formats:";

/// `MirrorGenerator` for Julia. Stateless — every `generate()` call
/// re-walks the supplied chain.
pub struct JuliaMirrorGenerator {
    version: &'static str,
    /// Stable content-hash anchor for the generator. Pinned to the
    /// crate version for v1 — refined to a real binary hash once the
    /// generator output stabilises and pinning to `Cargo.lock` digest
    /// pays off.
    content_hash: String,
}

impl JuliaMirrorGenerator {
    pub fn new() -> Self {
        let version = env!("CARGO_PKG_VERSION");
        Self {
            version,
            content_hash: format!("eigon-julia-gen:{version}"),
        }
    }
}

impl Default for JuliaMirrorGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl MirrorGenerator for JuliaMirrorGenerator {
    fn generator_identifier(&self) -> &str {
        GENERATOR_ID
    }

    fn generator_version(&self) -> &str {
        self.version
    }

    fn generator_content_hash(&self) -> &str {
        &self.content_hash
    }

    fn generate(
        &self,
        request: &MirrorGenerationRequest,
    ) -> Result<MirrorGenerationOutput, MirrorGeneratorError> {
        // 1. Collect closure: walk seed_classes transitively through
        //    resource-typed properties' class_types.
        let closure = walk_closure(request)?;

        // 2. Resolve each class in the closure and gather its property
        //    metadata. Indexed by class IRI for stable lookup.
        let class_decls = resolve_class_declarations(request, &closure)?;

        // 3. Topologically sort classes so a struct's referenced
        //    structs are declared before it. Stable on tie (by IRI).
        let order = topological_order(&class_decls);

        // 4. Emit the Julia source.
        let source = emit_module(&class_decls, &order, request);

        Ok(MirrorGenerationOutput {
            mirrored_classes: order.to_vec(),
            library: LibraryContent::Embedded(vec![LibraryFile {
                path: TARGET_FILE_PATH.to_string(),
                content: source.into_bytes(),
            }]),
        })
    }
}

/// Class declaration in the form the emitter consumes. `requires` and
/// `recommends` are pre-resolved to property declarations so the
/// emitter doesn't re-walk the chain.
struct ClassDecl {
    iri: Iri,
    short_name: String,
    requires: Vec<PropertyDecl>,
    recommends: Vec<PropertyDecl>,
}

/// One property's contribution to a struct field.
struct PropertyDecl {
    /// Property IRI — keys the `decode_*` / `encode_*` map lookups.
    iri: Iri,
    short_name: String,
    julia_type: JuliaType,
    constraints: PropertyConstraints,
}

/// Format / range constraints declared on a property in the
/// ontology. Drives the validating-inner-constructor emit. v1
/// captures the constraint primitives D1 spec carries on `Property`;
/// per-data-type semantics (e.g. `min_value` only meaningful for
/// integer / float properties) are enforced by the ontology validator
/// at commit time, not by the generator.
#[derive(Default, Debug)]
struct PropertyConstraints {
    min_value: Option<f64>,
    max_value: Option<f64>,
    min_length: Option<i64>,
    max_length: Option<i64>,
    pattern: Option<String>,
    /// Format short_name extracted from the format IRI's tail
    /// (e.g. `"date"` from `urn:eigenius:core:formats:date`).
    format: Option<String>,
}

/// The Julia type a property's `data_type` maps to. Only the cases
/// the v1 emitter needs; format constraints don't affect the type
/// here (they're handled by 19a.3.b's validating constructors).
#[derive(Debug, Clone)]
enum JuliaType {
    Primitive(&'static str),
    /// Reference to another mirror struct by class IRI.
    StructRef(Iri),
    /// `Vector{<inner>}` — only one level of nesting supported in v1.
    Vector(Box<JuliaType>),
}

impl JuliaType {
    /// Render to a Julia type expression. `class_lookup` resolves
    /// class IRIs to their short names (so `Compound` not the full
    /// IRI shows up in the source).
    fn render(&self, class_lookup: &BTreeMap<Iri, String>) -> String {
        match self {
            JuliaType::Primitive(s) => (*s).to_string(),
            JuliaType::StructRef(iri) => class_lookup
                .get(iri)
                .cloned()
                .unwrap_or_else(|| sanitise_for_identifier(iri.as_str())),
            JuliaType::Vector(inner) => {
                format!("Vector{{{}}}", inner.render(class_lookup))
            }
        }
    }

    /// Class IRIs the type references — drives the closure walker.
    fn struct_refs(&self) -> Vec<Iri> {
        match self {
            JuliaType::Primitive(_) => Vec::new(),
            JuliaType::StructRef(iri) => vec![iri.clone()],
            JuliaType::Vector(inner) => inner.struct_refs(),
        }
    }
}

fn walk_closure(request: &MirrorGenerationRequest) -> Result<BTreeSet<Iri>, MirrorGeneratorError> {
    let mut visited: BTreeSet<Iri> = BTreeSet::new();
    let mut queue: Vec<Iri> = request.seed_classes.to_vec();

    while let Some(class_iri) = queue.pop() {
        if !visited.insert(class_iri.clone()) {
            continue;
        }

        let class_def = request
            .chain
            .resolve(request.source_layer, &class_iri)
            .ok_or_else(|| MirrorGeneratorError::UnknownClass(class_iri.as_str().to_string()))?;

        // Walk required + recommended property class_types →
        // referenced classes.
        for prop_iri in iri_array(&class_def, PROP_REQUIRES)
            .into_iter()
            .chain(iri_array(&class_def, PROP_RECOMMENDS))
        {
            let prop_def = match request.chain.resolve(request.source_layer, &prop_iri) {
                Some(r) => r,
                None => continue,
            };
            let referenced = property_class_references(&prop_def);
            for r in referenced {
                if !visited.contains(&r) {
                    queue.push(r);
                }
            }
        }
    }

    Ok(visited)
}

fn property_class_references(prop_def: &Resource) -> Vec<Iri> {
    let dt = match resource_iri_value(prop_def, PROP_DATA_TYPE) {
        Some(iri) => iri,
        None => return Vec::new(),
    };
    match dt.as_str() {
        TYPE_RESOURCE | TYPE_RESOURCE_ARRAY => iri_array(prop_def, PROP_CLASS_TYPES),
        _ => Vec::new(),
    }
}

fn resolve_class_declarations(
    request: &MirrorGenerationRequest,
    closure: &BTreeSet<Iri>,
) -> Result<BTreeMap<Iri, ClassDecl>, MirrorGeneratorError> {
    let mut decls = BTreeMap::new();
    for class_iri in closure {
        let class_def = request
            .chain
            .resolve(request.source_layer, class_iri)
            .ok_or_else(|| MirrorGeneratorError::UnknownClass(class_iri.as_str().to_string()))?;

        let short_name = string_value(&class_def, PROP_SHORT_NAME).ok_or_else(|| {
            MirrorGeneratorError::UnrepresentableClass {
                class_iri: class_iri.as_str().to_string(),
                language: "julia".to_string(),
                reason: format!("class missing required `{}` property", PROP_SHORT_NAME),
            }
        })?;

        let requires = resolve_properties(request, &class_def, PROP_REQUIRES)?;
        let recommends = resolve_properties(request, &class_def, PROP_RECOMMENDS)?;

        decls.insert(
            class_iri.clone(),
            ClassDecl {
                iri: class_iri.clone(),
                short_name,
                requires,
                recommends,
            },
        );
    }
    Ok(decls)
}

fn resolve_properties(
    request: &MirrorGenerationRequest,
    class_def: &Resource,
    arity_prop: &str,
) -> Result<Vec<PropertyDecl>, MirrorGeneratorError> {
    let mut out = Vec::new();
    for prop_iri in iri_array(class_def, arity_prop) {
        let prop_def = request
            .chain
            .resolve(request.source_layer, &prop_iri)
            .ok_or_else(|| MirrorGeneratorError::UnknownClass(prop_iri.as_str().to_string()))?;
        let short_name = string_value(&prop_def, PROP_SHORT_NAME).ok_or_else(|| {
            MirrorGeneratorError::UnrepresentableClass {
                class_iri: prop_iri.as_str().to_string(),
                language: "julia".to_string(),
                reason: format!("property missing required `{}` property", PROP_SHORT_NAME),
            }
        })?;
        let julia_type = resolve_property_type(request, &prop_def, &prop_iri)?;
        let constraints = read_constraints(&prop_def);
        out.push(PropertyDecl {
            iri: prop_iri,
            short_name,
            julia_type,
            constraints,
        });
    }
    Ok(out)
}

fn read_constraints(prop_def: &Resource) -> PropertyConstraints {
    PropertyConstraints {
        min_value: numeric_value(prop_def, PROP_MIN_VALUE),
        max_value: numeric_value(prop_def, PROP_MAX_VALUE),
        min_length: integer_value(prop_def, PROP_MIN_LENGTH),
        max_length: integer_value(prop_def, PROP_MAX_LENGTH),
        pattern: string_value(prop_def, PROP_PATTERN),
        format: resource_iri_value(prop_def, PROP_FORMAT)
            .and_then(|iri| {
                iri.as_str()
                    .strip_prefix(FORMAT_IRI_PREFIX)
                    .map(str::to_string)
            }),
    }
}

/// Read a numeric property as f64. Tolerates `Value::Float` and
/// `Value::Integer` (the JSON parser keeps `0` as Integer and `0.0`
/// as Float; ontology authors write either).
fn numeric_value(r: &Resource, prop_iri: &str) -> Option<f64> {
    let iri = Iri::parse(prop_iri).ok()?;
    let v = r.get(&iri)?;
    v.as_float()
        .or_else(|| v.as_integer().map(|n| n as f64))
}

fn integer_value(r: &Resource, prop_iri: &str) -> Option<i64> {
    let iri = Iri::parse(prop_iri).ok()?;
    r.get(&iri).and_then(Value::as_integer)
}

fn resolve_property_type(
    _request: &MirrorGenerationRequest,
    prop_def: &Resource,
    prop_iri: &Iri,
) -> Result<JuliaType, MirrorGeneratorError> {
    let dt = resource_iri_value(prop_def, PROP_DATA_TYPE).ok_or_else(|| {
        MirrorGeneratorError::UnrepresentableClass {
            class_iri: prop_iri.as_str().to_string(),
            language: "julia".to_string(),
            reason: format!("property missing `{}`", PROP_DATA_TYPE),
        }
    })?;
    match dt.as_str() {
        TYPE_STRING => Ok(JuliaType::Primitive("String")),
        TYPE_INTEGER => Ok(JuliaType::Primitive("Int64")),
        TYPE_FLOAT => Ok(JuliaType::Primitive("Float64")),
        TYPE_BOOLEAN => Ok(JuliaType::Primitive("Bool")),
        TYPE_JSON => Ok(JuliaType::Primitive("Any")),
        TYPE_RESOURCE => {
            let class_types = iri_array(prop_def, PROP_CLASS_TYPES);
            // v1: pick the first; multi-class_types becomes a Union
            // type once we settle the runtime semantics. D27 §3.3
            // doesn't fully specify; flagged for 19a.3.b.
            class_types
                .into_iter()
                .next()
                .map(JuliaType::StructRef)
                .ok_or_else(|| MirrorGeneratorError::UnrepresentableClass {
                    class_iri: prop_iri.as_str().to_string(),
                    language: "julia".to_string(),
                    reason: format!(
                        "data_type `{TYPE_RESOURCE}` requires at least one `class_types` entry"
                    ),
                })
        }
        TYPE_RESOURCE_ARRAY => {
            let class_types = iri_array(prop_def, PROP_CLASS_TYPES);
            let inner = class_types
                .into_iter()
                .next()
                .map(JuliaType::StructRef)
                .ok_or_else(|| MirrorGeneratorError::UnrepresentableClass {
                    class_iri: prop_iri.as_str().to_string(),
                    language: "julia".to_string(),
                    reason: format!(
                        "data_type `{TYPE_RESOURCE_ARRAY}` requires at least one `class_types` entry"
                    ),
                })?;
            Ok(JuliaType::Vector(Box::new(inner)))
        }
        TYPE_VALUE_ARRAY => {
            let element_type = resource_iri_value(prop_def, PROP_ELEMENT_TYPE)
                .ok_or_else(|| MirrorGeneratorError::UnrepresentableClass {
                    class_iri: prop_iri.as_str().to_string(),
                    language: "julia".to_string(),
                    reason: format!(
                        "data_type `{TYPE_VALUE_ARRAY}` requires `{PROP_ELEMENT_TYPE}`"
                    ),
                })?;
            let inner = match element_type.as_str() {
                TYPE_STRING => JuliaType::Primitive("String"),
                TYPE_INTEGER => JuliaType::Primitive("Int64"),
                TYPE_FLOAT => JuliaType::Primitive("Float64"),
                TYPE_BOOLEAN => JuliaType::Primitive("Bool"),
                TYPE_JSON => JuliaType::Primitive("Any"),
                other => {
                    return Err(MirrorGeneratorError::UnrepresentableClass {
                        class_iri: prop_iri.as_str().to_string(),
                        language: "julia".to_string(),
                        reason: format!("value_array element_type `{other}` not supported"),
                    });
                }
            };
            Ok(JuliaType::Vector(Box::new(inner)))
        }
        other => Err(MirrorGeneratorError::UnrepresentableClass {
            class_iri: prop_iri.as_str().to_string(),
            language: "julia".to_string(),
            reason: format!("data_type `{other}` not supported in v1"),
        }),
    }
}

/// Topologically sort classes so a struct that references another is
/// declared after the referenced struct. Stable on tie (BTreeMap
/// iteration order = IRI sort).
fn topological_order(decls: &BTreeMap<Iri, ClassDecl>) -> Vec<Iri> {
    let mut visited: BTreeSet<Iri> = BTreeSet::new();
    let mut order: Vec<Iri> = Vec::new();

    fn visit(
        iri: &Iri,
        decls: &BTreeMap<Iri, ClassDecl>,
        visited: &mut BTreeSet<Iri>,
        order: &mut Vec<Iri>,
    ) {
        if visited.contains(iri) {
            return;
        }
        visited.insert(iri.clone());
        if let Some(decl) = decls.get(iri) {
            for prop in decl.requires.iter().chain(decl.recommends.iter()) {
                for ref_iri in prop.julia_type.struct_refs() {
                    visit(&ref_iri, decls, visited, order);
                }
            }
        }
        order.push(iri.clone());
    }

    // BTreeMap iteration is sorted by key — gives a stable starting
    // order, so the topological sort is deterministic.
    for iri in decls.keys() {
        visit(iri, decls, &mut visited, &mut order);
    }
    order
}

fn emit_module(
    decls: &BTreeMap<Iri, ClassDecl>,
    order: &[Iri],
    request: &MirrorGenerationRequest,
) -> String {
    let class_lookup: BTreeMap<Iri, String> = decls
        .values()
        .map(|d| (d.iri.clone(), d.short_name.clone()))
        .collect();

    let mut s = String::new();
    s.push_str("# Auto-generated by eigon-julia-gen — DO NOT EDIT.\n");
    s.push_str("# Regenerate via the substrate's image-build pipeline.\n");
    s.push_str(&format!("# source_layer: {}\n", request.source_layer));
    s.push_str("# mirrored_classes:\n");
    for iri in order {
        s.push_str(&format!("#   - {iri}\n"));
    }
    s.push('\n');
    s.push_str(&format!("module {TARGET_MODULE_NAME}\n\n"));

    s.push_str("using EigeniusJuliaCommon: validate_min_value, validate_max_value, ");
    s.push_str("validate_min_length, validate_max_length, validate_pattern, validate_format\n\n");

    for iri in order {
        let decl = decls.get(iri).expect("topological order references decls");
        emit_struct(&mut s, decl, &class_lookup);
        s.push('\n');
        emit_decoder(&mut s, decl, &class_lookup);
        s.push('\n');
        emit_encoder(&mut s, decl, &class_lookup);
        s.push('\n');
    }

    if !order.is_empty() {
        s.push_str("export ");
        let mut exports: Vec<String> = Vec::new();
        for iri in order {
            if let Some(d) = decls.get(iri) {
                exports.push(d.short_name.clone());
                exports.push(format!("decode_{}", d.short_name));
                exports.push(format!("encode_{}", d.short_name));
            }
        }
        s.push_str(&exports.join(", "));
        s.push('\n');
        s.push('\n');
    }

    s.push_str(&format!("end # module {TARGET_MODULE_NAME}\n"));
    s
}

fn emit_struct(
    out: &mut String,
    decl: &ClassDecl,
    class_lookup: &BTreeMap<Iri, String>,
) {
    out.push_str(&format!("struct {}\n", decl.short_name));
    for prop in &decl.requires {
        out.push_str(&format!(
            "    {}::{}\n",
            prop.short_name,
            prop.julia_type.render(class_lookup)
        ));
    }
    for prop in &decl.recommends {
        out.push_str(&format!(
            "    {}::Union{{{}, Nothing}}\n",
            prop.short_name,
            prop.julia_type.render(class_lookup)
        ));
    }
    out.push('\n');
    emit_inner_constructor(out, decl, class_lookup);
    out.push_str("end\n");
}

/// Inner constructor with format-constraint validation. Required
/// fields are positional; recommended fields are keyword args
/// defaulting to `nothing`. Each field's constraints (if any) are
/// checked before `new(...)`.
fn emit_inner_constructor(
    out: &mut String,
    decl: &ClassDecl,
    class_lookup: &BTreeMap<Iri, String>,
) {
    out.push_str(&format!("    function {}(\n", decl.short_name));

    let last_required = decl.requires.len().saturating_sub(1);
    let has_keyword = !decl.recommends.is_empty();

    // Positional args: required fields. The last one ends with `;`
    // (when keyword args follow) or `,` (when nothing follows or
    // only recommended fields follow without `;` form).
    for (i, prop) in decl.requires.iter().enumerate() {
        let trailer = if i == last_required && has_keyword {
            ";"
        } else {
            ","
        };
        out.push_str(&format!(
            "        {}::{}{trailer}\n",
            prop.short_name,
            prop.julia_type.render(class_lookup)
        ));
    }
    // Keyword args: recommended fields, default `nothing`.
    if has_keyword && decl.requires.is_empty() {
        // Edge case: only keyword args. Julia requires `;` to start
        // the keyword section even with no positional args.
        out.push_str("        ;\n");
    }
    for prop in &decl.recommends {
        out.push_str(&format!(
            "        {}::Union{{{}, Nothing}} = nothing,\n",
            prop.short_name,
            prop.julia_type.render(class_lookup)
        ));
    }
    out.push_str("    )\n");

    // Validation calls. Required props always; recommended props
    // gated on `isnothing(field) || …` so a missing recommended
    // field passes through without firing the validator.
    for prop in &decl.requires {
        emit_validations(out, prop, /* is_required = */ true);
    }
    for prop in &decl.recommends {
        emit_validations(out, prop, /* is_required = */ false);
    }

    // Construct.
    out.push_str("        new(");
    let all: Vec<&str> = decl
        .requires
        .iter()
        .chain(decl.recommends.iter())
        .map(|p| p.short_name.as_str())
        .collect();
    out.push_str(&all.join(", "));
    out.push_str(")\n");
    out.push_str("    end\n");
}

fn emit_validations(out: &mut String, prop: &PropertyDecl, is_required: bool) {
    let mut lines: Vec<String> = Vec::new();
    let c = &prop.constraints;
    let field = &prop.short_name;
    if let Some(min) = c.min_value {
        lines.push(format!(
            "validate_min_value(:{field}, {field}, {})",
            float_literal(min)
        ));
    }
    if let Some(max) = c.max_value {
        lines.push(format!(
            "validate_max_value(:{field}, {field}, {})",
            float_literal(max)
        ));
    }
    if let Some(n) = c.min_length {
        lines.push(format!("validate_min_length(:{field}, {field}, {n})"));
    }
    if let Some(n) = c.max_length {
        lines.push(format!("validate_max_length(:{field}, {field}, {n})"));
    }
    if let Some(pat) = &c.pattern {
        lines.push(format!(
            "validate_pattern(:{field}, {field}, {})",
            julia_string_literal(pat)
        ));
    }
    if let Some(fmt) = &c.format {
        lines.push(format!("validate_format(:{field}, {field}, :{fmt})"));
    }

    if lines.is_empty() {
        return;
    }

    if is_required {
        for line in &lines {
            out.push_str(&format!("        {line}\n"));
        }
    } else {
        // Skip validation when the recommended field was omitted.
        out.push_str(&format!("        if !isnothing({field})\n"));
        for line in &lines {
            out.push_str(&format!("            {line}\n"));
        }
        out.push_str("        end\n");
    }
}

/// Render an f64 as a Julia literal. `0` and `100` come out as `0.0`
/// / `100.0` so the validator-call type matches `Real` cleanly.
fn float_literal(v: f64) -> String {
    if v.fract() == 0.0 && v.is_finite() {
        format!("{v:.1}")
    } else {
        format!("{v}")
    }
}

/// Escape a string for embedding in a Julia double-quoted literal.
fn julia_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // `$` triggers Julia string interpolation; escape it.
            '$' => out.push_str("\\$"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn emit_decoder(
    out: &mut String,
    decl: &ClassDecl,
    class_lookup: &BTreeMap<Iri, String>,
) {
    let cls = &decl.short_name;
    out.push_str(&format!(
        "function decode_{cls}(m::AbstractDict)::{cls}\n"
    ));
    out.push_str(&format!("    {cls}(\n"));

    let last_required = decl.requires.len().saturating_sub(1);
    let has_keyword = !decl.recommends.is_empty();

    // Required positional args.
    for (i, prop) in decl.requires.iter().enumerate() {
        let trailer = if i == last_required && has_keyword {
            ";"
        } else {
            ","
        };
        out.push_str(&format!(
            "        {}{trailer}\n",
            decode_property_expr(prop, class_lookup, /* required = */ true)
        ));
    }
    // Keyword args for recommended.
    if has_keyword && decl.requires.is_empty() {
        out.push_str("        ;\n");
    }
    for prop in &decl.recommends {
        out.push_str(&format!(
            "        {} = {},\n",
            prop.short_name,
            decode_property_expr(prop, class_lookup, /* required = */ false)
        ));
    }
    out.push_str("    )\n");
    out.push_str("end\n");
}

fn decode_property_expr(
    prop: &PropertyDecl,
    class_lookup: &BTreeMap<Iri, String>,
    required: bool,
) -> String {
    let key = julia_string_literal(prop.iri.as_str());
    if required {
        decode_value_expr(&prop.julia_type, &format!("m[{key}]"), class_lookup)
    } else {
        // get(m, key, nothing); if nothing, pass through; else recurse.
        let raw = format!("get(m, {key}, nothing)");
        let inner = decode_value_expr(&prop.julia_type, "_v", class_lookup);
        format!(
            "(let _v = {raw}; isnothing(_v) ? nothing : ({inner}) end)"
        )
    }
}

/// Express the worker-side decode of `expr` (a CBOR-loaded value)
/// into the Julia type `t`. Resource-typed fields recurse via
/// `decode_<Class>`; primitives pass through.
fn decode_value_expr(
    t: &JuliaType,
    expr: &str,
    class_lookup: &BTreeMap<Iri, String>,
) -> String {
    match t {
        JuliaType::Primitive(_) => expr.to_string(),
        JuliaType::StructRef(iri) => {
            let cls = class_lookup
                .get(iri)
                .cloned()
                .unwrap_or_else(|| sanitise_for_identifier(iri.as_str()));
            format!("decode_{cls}({expr})")
        }
        JuliaType::Vector(inner) => match inner.as_ref() {
            JuliaType::Primitive(_) => expr.to_string(),
            JuliaType::StructRef(iri) => {
                let cls = class_lookup
                    .get(iri)
                    .cloned()
                    .unwrap_or_else(|| sanitise_for_identifier(iri.as_str()));
                format!("[decode_{cls}(_x) for _x in {expr}]")
            }
            JuliaType::Vector(_) => {
                // Nested vectors aren't reachable in v1 (no
                // value_array of resource_array); emit a passthrough
                // so the source still parses and add a TODO.
                format!("# TODO: nested Vector decode unsupported in v1\n        {expr}")
            }
        },
    }
}

fn emit_encoder(
    out: &mut String,
    decl: &ClassDecl,
    class_lookup: &BTreeMap<Iri, String>,
) {
    let cls = &decl.short_name;
    out.push_str(&format!(
        "function encode_{cls}(c::{cls})::Dict{{String, Any}}\n"
    ));
    out.push_str("    out = Dict{String, Any}(\n");
    out.push_str(&format!(
        "        {} => [{}],\n",
        julia_string_literal(PROP_IS_A),
        julia_string_literal(decl.iri.as_str())
    ));
    for prop in &decl.requires {
        let key = julia_string_literal(prop.iri.as_str());
        let value = encode_value_expr(
            &prop.julia_type,
            &format!("c.{}", prop.short_name),
            class_lookup,
        );
        out.push_str(&format!("        {key} => {value},\n"));
    }
    out.push_str("    )\n");
    for prop in &decl.recommends {
        let key = julia_string_literal(prop.iri.as_str());
        let field = &prop.short_name;
        let value = encode_value_expr(
            &prop.julia_type,
            &format!("c.{field}"),
            class_lookup,
        );
        out.push_str(&format!(
            "    isnothing(c.{field}) || (out[{key}] = {value})\n"
        ));
    }
    out.push_str("    return out\n");
    out.push_str("end\n");
}

fn encode_value_expr(
    t: &JuliaType,
    expr: &str,
    class_lookup: &BTreeMap<Iri, String>,
) -> String {
    match t {
        JuliaType::Primitive(_) => expr.to_string(),
        JuliaType::StructRef(iri) => {
            let cls = class_lookup
                .get(iri)
                .cloned()
                .unwrap_or_else(|| sanitise_for_identifier(iri.as_str()));
            format!("encode_{cls}({expr})")
        }
        JuliaType::Vector(inner) => match inner.as_ref() {
            JuliaType::Primitive(_) => expr.to_string(),
            JuliaType::StructRef(iri) => {
                let cls = class_lookup
                    .get(iri)
                    .cloned()
                    .unwrap_or_else(|| sanitise_for_identifier(iri.as_str()));
                format!("[encode_{cls}(_x) for _x in {expr}]")
            }
            JuliaType::Vector(_) => {
                format!("# TODO: nested Vector encode unsupported in v1\n        {expr}")
            }
        },
    }
}

// --- Resource readers --------------------------------------------------

fn string_value(r: &Resource, prop_iri: &str) -> Option<String> {
    let iri = Iri::parse(prop_iri).ok()?;
    r.get(&iri).and_then(Value::as_str).map(str::to_string)
}

/// Read a property value as a single resource IRI. Tolerates the
/// chain's two encodings of an IRI-typed value: `Value::ResourceRef`
/// (the canonical form) and `Value::String` (the JSON parser stores
/// IRIs as strings until the property's `data_type` is consulted).
fn resource_iri_value(r: &Resource, prop_iri: &str) -> Option<Iri> {
    let iri = Iri::parse(prop_iri).ok()?;
    let v = r.get(&iri)?;
    match v {
        Value::ResourceRef(i) => Some(i.clone()),
        Value::String(s) => Iri::parse(s).ok(),
        _ => None,
    }
}

/// Read a property value as a list of IRIs. Tolerates string-typed
/// elements from the JSON parser the same way `Value::as_iri_array`
/// does.
fn iri_array(r: &Resource, prop_iri: &str) -> Vec<Iri> {
    let iri = match Iri::parse(prop_iri) {
        Ok(i) => i,
        Err(_) => return Vec::new(),
    };
    r.get(&iri).map(Value::as_iri_array).unwrap_or_default()
}

fn sanitise_for_identifier(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut first = true;
    for c in s.chars() {
        let safe = if c.is_ascii_alphanumeric() || c == '_' {
            c
        } else {
            '_'
        };
        // First character must not be a digit.
        if first && safe.is_ascii_digit() {
            out.push('_');
        }
        out.push(safe);
        first = false;
    }
    if out.is_empty() {
        out.push('_');
    }
    out
}

// --- Tests --------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use eigenius_runtime_substrate::chain::ChainAccessor;
    use std::collections::HashMap;

    /// Synthetic chain backed by a flat IRI → Resource map. Resolves
    /// all IRIs at any layer. Sufficient for exercising the
    /// generator's class-walking + emission logic without standing up
    /// a real layer chain.
    struct FlatChain {
        resources: HashMap<Iri, Resource>,
    }

    impl FlatChain {
        fn new() -> Self {
            Self {
                resources: HashMap::new(),
            }
        }

        fn add(&mut self, iri: &str, r: Resource) {
            self.resources.insert(Iri::parse(iri).unwrap(), r);
        }
    }

    impl ChainAccessor for FlatChain {
        fn resolve(&self, _claim_layer: &Iri, target: &Iri) -> Option<Resource> {
            self.resources.get(target).cloned()
        }
        fn is_ancestor_or_equal(&self, _a: &Iri, _b: &Iri) -> bool {
            true
        }
        fn class_unchanged_between(&self, _: &Iri, _: &Iri, _: &Iri) -> bool {
            true
        }
    }

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn class_decl(short: &str, requires: &[&str], recommends: &[&str]) -> Resource {
        let mut r = Resource::new(iri(&format!("urn:eigenius:demo:assay:{short}")));
        r.set(iri(PROP_SHORT_NAME), Value::String(short.into()));
        let req: Vec<Value> = requires
            .iter()
            .map(|s| Value::ResourceRef(iri(s)))
            .collect();
        if !req.is_empty() {
            r.set(iri(PROP_REQUIRES), Value::Array(req));
        }
        let rec: Vec<Value> = recommends
            .iter()
            .map(|s| Value::ResourceRef(iri(s)))
            .collect();
        if !rec.is_empty() {
            r.set(iri(PROP_RECOMMENDS), Value::Array(rec));
        }
        r
    }

    fn property_decl(iri_str: &str, short: &str, data_type: &str) -> Resource {
        let mut r = Resource::new(iri(iri_str));
        r.set(iri(PROP_SHORT_NAME), Value::String(short.into()));
        r.set(iri(PROP_DATA_TYPE), Value::ResourceRef(iri(data_type)));
        r
    }

    fn property_resource(iri_str: &str, short: &str, class_iri: &str) -> Resource {
        let mut r = property_decl(iri_str, short, TYPE_RESOURCE);
        r.set(
            iri(PROP_CLASS_TYPES),
            Value::Array(vec![Value::ResourceRef(iri(class_iri))]),
        );
        r
    }

    /// Add `min_value` constraint to an existing property resource.
    fn with_min_value(mut r: Resource, min: f64) -> Resource {
        r.set(iri(PROP_MIN_VALUE), Value::Float(min));
        r
    }

    /// Add `format` constraint (the IRI's tail becomes the
    /// validation symbol — e.g. `date`).
    fn with_format(mut r: Resource, format_short: &str) -> Resource {
        r.set(
            iri(PROP_FORMAT),
            Value::ResourceRef(iri(&format!("{FORMAT_IRI_PREFIX}{format_short}"))),
        );
        r
    }

    /// Build a chain mirroring the kinase ontology's structure.
    fn build_kinase_chain() -> FlatChain {
        let mut chain = FlatChain::new();

        // Compound class — three required props + one recommended.
        chain.add(
            "urn:eigenius:demo:assay:Compound",
            class_decl(
                "Compound",
                &[
                    "urn:eigenius:demo:assay:compound_id",
                    "urn:eigenius:demo:assay:scaffold_class",
                    "urn:eigenius:demo:assay:molecular_weight",
                ],
                &["urn:eigenius:demo:assay:logp"],
            ),
        );
        chain.add(
            "urn:eigenius:demo:assay:compound_id",
            property_decl(
                "urn:eigenius:demo:assay:compound_id",
                "compound_id",
                TYPE_STRING,
            ),
        );
        chain.add(
            "urn:eigenius:demo:assay:scaffold_class",
            property_decl(
                "urn:eigenius:demo:assay:scaffold_class",
                "scaffold_class",
                TYPE_STRING,
            ),
        );
        chain.add(
            "urn:eigenius:demo:assay:molecular_weight",
            with_min_value(
                property_decl(
                    "urn:eigenius:demo:assay:molecular_weight",
                    "molecular_weight",
                    TYPE_FLOAT,
                ),
                0.0,
            ),
        );
        chain.add(
            "urn:eigenius:demo:assay:logp",
            property_decl("urn:eigenius:demo:assay:logp", "logp", TYPE_FLOAT),
        );

        // Target class — two required string props.
        chain.add(
            "urn:eigenius:demo:assay:Target",
            class_decl(
                "Target",
                &[
                    "urn:eigenius:demo:assay:target_name",
                    "urn:eigenius:demo:assay:target_family",
                ],
                &[],
            ),
        );
        chain.add(
            "urn:eigenius:demo:assay:target_name",
            property_decl(
                "urn:eigenius:demo:assay:target_name",
                "target_name",
                TYPE_STRING,
            ),
        );
        chain.add(
            "urn:eigenius:demo:assay:target_family",
            property_decl(
                "urn:eigenius:demo:assay:target_family",
                "target_family",
                TYPE_STRING,
            ),
        );

        // AssayProtocol — one string + one int.
        chain.add(
            "urn:eigenius:demo:assay:AssayProtocol",
            class_decl(
                "AssayProtocol",
                &[
                    "urn:eigenius:demo:assay:protocol_name",
                    "urn:eigenius:demo:assay:incubation_minutes",
                ],
                &[],
            ),
        );
        chain.add(
            "urn:eigenius:demo:assay:protocol_name",
            property_decl(
                "urn:eigenius:demo:assay:protocol_name",
                "protocol_name",
                TYPE_STRING,
            ),
        );
        chain.add(
            "urn:eigenius:demo:assay:incubation_minutes",
            with_min_value(
                property_decl(
                    "urn:eigenius:demo:assay:incubation_minutes",
                    "incubation_minutes",
                    TYPE_INTEGER,
                ),
                0.0,
            ),
        );

        // AssayResult — three resource-typed refs (Compound/Target/Protocol)
        // + numeric/string/boolean fields.
        chain.add(
            "urn:eigenius:demo:assay:AssayResult",
            class_decl(
                "AssayResult",
                &[
                    "urn:eigenius:demo:assay:compound",
                    "urn:eigenius:demo:assay:target",
                    "urn:eigenius:demo:assay:protocol",
                    "urn:eigenius:demo:assay:ic50_nm",
                    "urn:eigenius:demo:assay:replicate_count",
                    "urn:eigenius:demo:assay:measurement_date",
                    "urn:eigenius:demo:assay:passed_qc",
                ],
                &[
                    "urn:eigenius:demo:assay:ci_low_nm",
                    "urn:eigenius:demo:assay:ci_high_nm",
                ],
            ),
        );
        chain.add(
            "urn:eigenius:demo:assay:compound",
            property_resource(
                "urn:eigenius:demo:assay:compound",
                "compound",
                "urn:eigenius:demo:assay:Compound",
            ),
        );
        chain.add(
            "urn:eigenius:demo:assay:target",
            property_resource(
                "urn:eigenius:demo:assay:target",
                "target",
                "urn:eigenius:demo:assay:Target",
            ),
        );
        chain.add(
            "urn:eigenius:demo:assay:protocol",
            property_resource(
                "urn:eigenius:demo:assay:protocol",
                "protocol",
                "urn:eigenius:demo:assay:AssayProtocol",
            ),
        );
        chain.add(
            "urn:eigenius:demo:assay:ic50_nm",
            with_min_value(
                property_decl("urn:eigenius:demo:assay:ic50_nm", "ic50_nm", TYPE_FLOAT),
                0.0,
            ),
        );
        chain.add(
            "urn:eigenius:demo:assay:replicate_count",
            with_min_value(
                property_decl(
                    "urn:eigenius:demo:assay:replicate_count",
                    "replicate_count",
                    TYPE_INTEGER,
                ),
                1.0,
            ),
        );
        chain.add(
            "urn:eigenius:demo:assay:measurement_date",
            with_format(
                property_decl(
                    "urn:eigenius:demo:assay:measurement_date",
                    "measurement_date",
                    TYPE_STRING,
                ),
                "date",
            ),
        );
        chain.add(
            "urn:eigenius:demo:assay:passed_qc",
            property_decl(
                "urn:eigenius:demo:assay:passed_qc",
                "passed_qc",
                TYPE_BOOLEAN,
            ),
        );
        chain.add(
            "urn:eigenius:demo:assay:ci_low_nm",
            with_min_value(
                property_decl("urn:eigenius:demo:assay:ci_low_nm", "ci_low_nm", TYPE_FLOAT),
                0.0,
            ),
        );
        chain.add(
            "urn:eigenius:demo:assay:ci_high_nm",
            with_min_value(
                property_decl(
                    "urn:eigenius:demo:assay:ci_high_nm",
                    "ci_high_nm",
                    TYPE_FLOAT,
                ),
                0.0,
            ),
        );

        chain
    }

    fn run_kinase(seed: &[&str]) -> MirrorGenerationOutput {
        let chain = build_kinase_chain();
        let layer = iri("urn:eigenius:test:layer");
        let seed_iris: Vec<Iri> = seed.iter().map(|s| iri(s)).collect();
        let request = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed_iris,
            chain: &chain,
        };
        JuliaMirrorGenerator::new()
            .generate(&request)
            .expect("generate")
    }

    fn extract_source(out: &MirrorGenerationOutput) -> String {
        match &out.library {
            LibraryContent::Embedded(files) => {
                let f = files
                    .iter()
                    .find(|f| f.path == TARGET_FILE_PATH)
                    .expect("module file present");
                String::from_utf8(f.content.clone()).expect("utf-8 source")
            }
            other => panic!("expected Embedded library, got {other:?}"),
        }
    }

    #[test]
    fn closure_pulls_in_referenced_classes() {
        // Seeded only on AssayResult; Compound/Target/AssayProtocol
        // must be discovered via the resource-typed properties.
        let out = run_kinase(&["urn:eigenius:demo:assay:AssayResult"]);
        let mirrored: Vec<String> = out
            .mirrored_classes
            .iter()
            .map(|iri| iri.as_str().to_string())
            .collect();
        assert!(mirrored.contains(&"urn:eigenius:demo:assay:Compound".to_string()));
        assert!(mirrored.contains(&"urn:eigenius:demo:assay:Target".to_string()));
        assert!(mirrored.contains(&"urn:eigenius:demo:assay:AssayProtocol".to_string()));
        assert!(mirrored.contains(&"urn:eigenius:demo:assay:AssayResult".to_string()));
    }

    #[test]
    fn topological_order_puts_referenced_first() {
        let out = run_kinase(&["urn:eigenius:demo:assay:AssayResult"]);
        let order: Vec<&str> = out
            .mirrored_classes
            .iter()
            .map(|iri| iri.as_str())
            .collect();
        let assay_idx = order
            .iter()
            .position(|s| *s == "urn:eigenius:demo:assay:AssayResult")
            .unwrap();
        for referenced in [
            "urn:eigenius:demo:assay:Compound",
            "urn:eigenius:demo:assay:Target",
            "urn:eigenius:demo:assay:AssayProtocol",
        ] {
            let i = order.iter().position(|s| *s == referenced).unwrap();
            assert!(
                i < assay_idx,
                "{referenced} must come before AssayResult, got order {order:?}"
            );
        }
    }

    #[test]
    fn struct_for_compound_has_required_and_optional_fields() {
        let out = run_kinase(&["urn:eigenius:demo:assay:Compound"]);
        let src = extract_source(&out);
        // Required fields: bare types.
        assert!(src.contains("compound_id::String"));
        assert!(src.contains("scaffold_class::String"));
        assert!(src.contains("molecular_weight::Float64"));
        // Recommended field: Union{T, Nothing}.
        assert!(src.contains("logp::Union{Float64, Nothing}"));
    }

    #[test]
    fn struct_for_assay_result_uses_struct_refs_not_iris() {
        let out = run_kinase(&["urn:eigenius:demo:assay:AssayResult"]);
        let src = extract_source(&out);
        assert!(
            src.contains("compound::Compound"),
            "expected `compound::Compound`, got source:\n{src}"
        );
        assert!(src.contains("target::Target"));
        assert!(src.contains("protocol::AssayProtocol"));
        assert!(src.contains("ic50_nm::Float64"));
        assert!(src.contains("replicate_count::Int64"));
        assert!(src.contains("passed_qc::Bool"));
        assert!(src.contains("ci_low_nm::Union{Float64, Nothing}"));
    }

    #[test]
    fn module_exports_all_classes() {
        let out = run_kinase(&["urn:eigenius:demo:assay:AssayResult"]);
        let src = extract_source(&out);
        assert!(src.contains("export "));
        for name in ["Compound", "Target", "AssayProtocol", "AssayResult"] {
            assert!(src.contains(name), "expected {name} in source");
        }
    }

    #[test]
    fn output_is_deterministic_under_repeated_runs() {
        let a = extract_source(&run_kinase(&["urn:eigenius:demo:assay:AssayResult"]));
        let b = extract_source(&run_kinase(&["urn:eigenius:demo:assay:AssayResult"]));
        assert_eq!(a, b, "repeated runs must produce byte-identical output");
    }

    #[test]
    fn output_is_deterministic_independent_of_seed_order() {
        // The same closure should produce the same output regardless of
        // the order seeds are passed in.
        let a = extract_source(&run_kinase(&[
            "urn:eigenius:demo:assay:Compound",
            "urn:eigenius:demo:assay:AssayResult",
        ]));
        let b = extract_source(&run_kinase(&[
            "urn:eigenius:demo:assay:AssayResult",
            "urn:eigenius:demo:assay:Compound",
        ]));
        assert_eq!(a, b, "seed-order independence");
    }

    /// Snapshot of the full emitted module for the kinase fixture —
    /// the canonical "this is what the generator produces" anchor.
    /// If you change the generator's output shape on purpose, update
    /// the expected string here intentionally; if a test fails
    /// unintentionally, the diff shows exactly what regressed.
    #[test]
    fn full_kinase_module_snapshot() {
        let out = run_kinase(&["urn:eigenius:demo:assay:AssayResult"]);
        let src = extract_source(&out);
        let expected = "\
# Auto-generated by eigon-julia-gen — DO NOT EDIT.
# Regenerate via the substrate's image-build pipeline.
# source_layer: urn:eigenius:test:layer
# mirrored_classes:
#   - urn:eigenius:demo:assay:AssayProtocol
#   - urn:eigenius:demo:assay:Compound
#   - urn:eigenius:demo:assay:Target
#   - urn:eigenius:demo:assay:AssayResult

module EigeniusMirror

using EigeniusJuliaCommon: validate_min_value, validate_max_value, validate_min_length, validate_max_length, validate_pattern, validate_format

struct AssayProtocol
    protocol_name::String
    incubation_minutes::Int64

    function AssayProtocol(
        protocol_name::String,
        incubation_minutes::Int64,
    )
        validate_min_value(:incubation_minutes, incubation_minutes, 0.0)
        new(protocol_name, incubation_minutes)
    end
end

function decode_AssayProtocol(m::AbstractDict)::AssayProtocol
    AssayProtocol(
        m[\"urn:eigenius:demo:assay:protocol_name\"],
        m[\"urn:eigenius:demo:assay:incubation_minutes\"],
    )
end

function encode_AssayProtocol(c::AssayProtocol)::Dict{String, Any}
    out = Dict{String, Any}(
        \"urn:eigenius:core:is_a\" => [\"urn:eigenius:demo:assay:AssayProtocol\"],
        \"urn:eigenius:demo:assay:protocol_name\" => c.protocol_name,
        \"urn:eigenius:demo:assay:incubation_minutes\" => c.incubation_minutes,
    )
    return out
end

struct Compound
    compound_id::String
    scaffold_class::String
    molecular_weight::Float64
    logp::Union{Float64, Nothing}

    function Compound(
        compound_id::String,
        scaffold_class::String,
        molecular_weight::Float64;
        logp::Union{Float64, Nothing} = nothing,
    )
        validate_min_value(:molecular_weight, molecular_weight, 0.0)
        new(compound_id, scaffold_class, molecular_weight, logp)
    end
end

function decode_Compound(m::AbstractDict)::Compound
    Compound(
        m[\"urn:eigenius:demo:assay:compound_id\"],
        m[\"urn:eigenius:demo:assay:scaffold_class\"],
        m[\"urn:eigenius:demo:assay:molecular_weight\"];
        logp = (let _v = get(m, \"urn:eigenius:demo:assay:logp\", nothing); isnothing(_v) ? nothing : (_v) end),
    )
end

function encode_Compound(c::Compound)::Dict{String, Any}
    out = Dict{String, Any}(
        \"urn:eigenius:core:is_a\" => [\"urn:eigenius:demo:assay:Compound\"],
        \"urn:eigenius:demo:assay:compound_id\" => c.compound_id,
        \"urn:eigenius:demo:assay:scaffold_class\" => c.scaffold_class,
        \"urn:eigenius:demo:assay:molecular_weight\" => c.molecular_weight,
    )
    isnothing(c.logp) || (out[\"urn:eigenius:demo:assay:logp\"] = c.logp)
    return out
end

struct Target
    target_name::String
    target_family::String

    function Target(
        target_name::String,
        target_family::String,
    )
        new(target_name, target_family)
    end
end

function decode_Target(m::AbstractDict)::Target
    Target(
        m[\"urn:eigenius:demo:assay:target_name\"],
        m[\"urn:eigenius:demo:assay:target_family\"],
    )
end

function encode_Target(c::Target)::Dict{String, Any}
    out = Dict{String, Any}(
        \"urn:eigenius:core:is_a\" => [\"urn:eigenius:demo:assay:Target\"],
        \"urn:eigenius:demo:assay:target_name\" => c.target_name,
        \"urn:eigenius:demo:assay:target_family\" => c.target_family,
    )
    return out
end

struct AssayResult
    compound::Compound
    target::Target
    protocol::AssayProtocol
    ic50_nm::Float64
    replicate_count::Int64
    measurement_date::String
    passed_qc::Bool
    ci_low_nm::Union{Float64, Nothing}
    ci_high_nm::Union{Float64, Nothing}

    function AssayResult(
        compound::Compound,
        target::Target,
        protocol::AssayProtocol,
        ic50_nm::Float64,
        replicate_count::Int64,
        measurement_date::String,
        passed_qc::Bool;
        ci_low_nm::Union{Float64, Nothing} = nothing,
        ci_high_nm::Union{Float64, Nothing} = nothing,
    )
        validate_min_value(:ic50_nm, ic50_nm, 0.0)
        validate_min_value(:replicate_count, replicate_count, 1.0)
        validate_format(:measurement_date, measurement_date, :date)
        if !isnothing(ci_low_nm)
            validate_min_value(:ci_low_nm, ci_low_nm, 0.0)
        end
        if !isnothing(ci_high_nm)
            validate_min_value(:ci_high_nm, ci_high_nm, 0.0)
        end
        new(compound, target, protocol, ic50_nm, replicate_count, measurement_date, passed_qc, ci_low_nm, ci_high_nm)
    end
end

function decode_AssayResult(m::AbstractDict)::AssayResult
    AssayResult(
        decode_Compound(m[\"urn:eigenius:demo:assay:compound\"]),
        decode_Target(m[\"urn:eigenius:demo:assay:target\"]),
        decode_AssayProtocol(m[\"urn:eigenius:demo:assay:protocol\"]),
        m[\"urn:eigenius:demo:assay:ic50_nm\"],
        m[\"urn:eigenius:demo:assay:replicate_count\"],
        m[\"urn:eigenius:demo:assay:measurement_date\"],
        m[\"urn:eigenius:demo:assay:passed_qc\"];
        ci_low_nm = (let _v = get(m, \"urn:eigenius:demo:assay:ci_low_nm\", nothing); isnothing(_v) ? nothing : (_v) end),
        ci_high_nm = (let _v = get(m, \"urn:eigenius:demo:assay:ci_high_nm\", nothing); isnothing(_v) ? nothing : (_v) end),
    )
end

function encode_AssayResult(c::AssayResult)::Dict{String, Any}
    out = Dict{String, Any}(
        \"urn:eigenius:core:is_a\" => [\"urn:eigenius:demo:assay:AssayResult\"],
        \"urn:eigenius:demo:assay:compound\" => encode_Compound(c.compound),
        \"urn:eigenius:demo:assay:target\" => encode_Target(c.target),
        \"urn:eigenius:demo:assay:protocol\" => encode_AssayProtocol(c.protocol),
        \"urn:eigenius:demo:assay:ic50_nm\" => c.ic50_nm,
        \"urn:eigenius:demo:assay:replicate_count\" => c.replicate_count,
        \"urn:eigenius:demo:assay:measurement_date\" => c.measurement_date,
        \"urn:eigenius:demo:assay:passed_qc\" => c.passed_qc,
    )
    isnothing(c.ci_low_nm) || (out[\"urn:eigenius:demo:assay:ci_low_nm\"] = c.ci_low_nm)
    isnothing(c.ci_high_nm) || (out[\"urn:eigenius:demo:assay:ci_high_nm\"] = c.ci_high_nm)
    return out
end

export AssayProtocol, decode_AssayProtocol, encode_AssayProtocol, Compound, decode_Compound, encode_Compound, Target, decode_Target, encode_Target, AssayResult, decode_AssayResult, encode_AssayResult

end # module EigeniusMirror
";
        assert_eq!(
            src.as_str(),
            expected,
            "generated source diverged from snapshot:\n--- actual ---\n{src}\n--- expected ---\n{expected}"
        );
    }

    #[test]
    fn min_value_constraint_emits_inline_validator() {
        let out = run_kinase(&["urn:eigenius:demo:assay:Compound"]);
        let src = extract_source(&out);
        assert!(
            src.contains("validate_min_value(:molecular_weight, molecular_weight, 0.0)"),
            "expected min_value validator, got source:\n{src}"
        );
    }

    #[test]
    fn format_date_constraint_emits_inline_validator() {
        let out = run_kinase(&["urn:eigenius:demo:assay:AssayResult"]);
        let src = extract_source(&out);
        assert!(
            src.contains("validate_format(:measurement_date, measurement_date, :date)"),
            "expected format validator, got source:\n{src}"
        );
    }

    #[test]
    fn recommended_field_validator_is_isnothing_gated() {
        // ci_low_nm has min_value=0 and is recommended; the validator
        // must be inside `if !isnothing(ci_low_nm) … end` so a
        // missing field doesn't fire it.
        let out = run_kinase(&["urn:eigenius:demo:assay:AssayResult"]);
        let src = extract_source(&out);
        assert!(
            src.contains("if !isnothing(ci_low_nm)\n            validate_min_value(:ci_low_nm"),
            "expected isnothing-gated validator, got source:\n{src}"
        );
    }

    #[test]
    fn decoder_recurses_into_resource_typed_fields() {
        let out = run_kinase(&["urn:eigenius:demo:assay:AssayResult"]);
        let src = extract_source(&out);
        assert!(
            src.contains("decode_Compound(m[\"urn:eigenius:demo:assay:compound\"])"),
            "expected nested decode_Compound call, got source:\n{src}"
        );
        assert!(src.contains("decode_Target(m[\"urn:eigenius:demo:assay:target\"])"));
        assert!(src.contains("decode_AssayProtocol(m[\"urn:eigenius:demo:assay:protocol\"])"));
    }

    #[test]
    fn encoder_recurses_into_resource_typed_fields() {
        let out = run_kinase(&["urn:eigenius:demo:assay:AssayResult"]);
        let src = extract_source(&out);
        assert!(src.contains("\"urn:eigenius:demo:assay:compound\" => encode_Compound(c.compound)"));
        assert!(src.contains("\"urn:eigenius:demo:assay:target\" => encode_Target(c.target)"));
        assert!(src
            .contains("\"urn:eigenius:demo:assay:protocol\" => encode_AssayProtocol(c.protocol)"));
    }

    #[test]
    fn encoder_stamps_is_a() {
        let out = run_kinase(&["urn:eigenius:demo:assay:Compound"]);
        let src = extract_source(&out);
        assert!(
            src.contains("\"urn:eigenius:core:is_a\" => [\"urn:eigenius:demo:assay:Compound\"]"),
            "expected is_a stamp, got source:\n{src}"
        );
    }

    #[test]
    fn encoder_skips_recommended_when_nothing() {
        let out = run_kinase(&["urn:eigenius:demo:assay:Compound"]);
        let src = extract_source(&out);
        // logp is recommended → conditional encode.
        assert!(
            src.contains("isnothing(c.logp) || (out[\"urn:eigenius:demo:assay:logp\"] = c.logp)"),
            "expected conditional encode for recommended field, got source:\n{src}"
        );
    }

    #[test]
    fn module_imports_eigenius_julia_common() {
        let out = run_kinase(&["urn:eigenius:demo:assay:AssayResult"]);
        let src = extract_source(&out);
        assert!(
            src.contains("using EigeniusJuliaCommon: validate_"),
            "expected `using EigeniusJuliaCommon: validate_…`, got source:\n{src}"
        );
    }

    #[test]
    fn unknown_seed_class_returns_unknown_class_error() {
        let chain = FlatChain::new();
        let layer = iri("urn:eigenius:test:layer");
        let seed = vec![iri("urn:eigenius:does:not:exist")];
        let request = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };
        let result = JuliaMirrorGenerator::new().generate(&request);
        match result {
            Err(MirrorGeneratorError::UnknownClass(_)) => {}
            Err(other) => panic!("expected UnknownClass, got {other:?}"),
            Ok(_) => panic!("expected unknown-class error, got Ok"),
        }
    }
}
