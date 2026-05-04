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

const TYPE_STRING: &str = "urn:eigenius:core:string";
const TYPE_INTEGER: &str = "urn:eigenius:core:integer";
const TYPE_FLOAT: &str = "urn:eigenius:core:float";
const TYPE_BOOLEAN: &str = "urn:eigenius:core:boolean";
const TYPE_RESOURCE: &str = "urn:eigenius:core:resource";
const TYPE_RESOURCE_ARRAY: &str = "urn:eigenius:core:resource_array";
const TYPE_VALUE_ARRAY: &str = "urn:eigenius:core:value_array";
const TYPE_JSON: &str = "urn:eigenius:core:json";

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
    /// Property IRI — needed by the codec emitter (19a.3.b) to key
    /// `decode_*` / `encode_*` lookups on the IRI-keyed CBOR map.
    /// Unused in 19a.3.a; the `#[allow]` removes when 19a.3.b lands.
    #[allow(dead_code)]
    iri: Iri,
    short_name: String,
    julia_type: JuliaType,
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
        out.push(PropertyDecl {
            iri: prop_iri,
            short_name,
            julia_type,
        });
    }
    Ok(out)
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

    for iri in order {
        let decl = decls.get(iri).expect("topological order references decls");
        emit_struct(&mut s, decl, &class_lookup);
        s.push('\n');
    }

    if !order.is_empty() {
        s.push_str("export ");
        let names: Vec<&str> = order
            .iter()
            .filter_map(|iri| decls.get(iri).map(|d| d.short_name.as_str()))
            .collect();
        s.push_str(&names.join(", "));
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
    out.push_str("end\n");
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
            property_decl(
                "urn:eigenius:demo:assay:molecular_weight",
                "molecular_weight",
                TYPE_FLOAT,
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
            property_decl(
                "urn:eigenius:demo:assay:incubation_minutes",
                "incubation_minutes",
                TYPE_INTEGER,
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
            property_decl(
                "urn:eigenius:demo:assay:ic50_nm",
                "ic50_nm",
                TYPE_FLOAT,
            ),
        );
        chain.add(
            "urn:eigenius:demo:assay:replicate_count",
            property_decl(
                "urn:eigenius:demo:assay:replicate_count",
                "replicate_count",
                TYPE_INTEGER,
            ),
        );
        chain.add(
            "urn:eigenius:demo:assay:measurement_date",
            property_decl(
                "urn:eigenius:demo:assay:measurement_date",
                "measurement_date",
                TYPE_STRING,
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
            property_decl(
                "urn:eigenius:demo:assay:ci_low_nm",
                "ci_low_nm",
                TYPE_FLOAT,
            ),
        );
        chain.add(
            "urn:eigenius:demo:assay:ci_high_nm",
            property_decl(
                "urn:eigenius:demo:assay:ci_high_nm",
                "ci_high_nm",
                TYPE_FLOAT,
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

struct AssayProtocol
    protocol_name::String
    incubation_minutes::Int64
end

struct Compound
    compound_id::String
    scaffold_class::String
    molecular_weight::Float64
    logp::Union{Float64, Nothing}
end

struct Target
    target_name::String
    target_family::String
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
end

export AssayProtocol, Compound, Target, AssayResult

end # module EigeniusMirror
";
        assert_eq!(
            src.as_str(),
            expected,
            "generated source diverged from snapshot:\n--- actual ---\n{src}\n--- expected ---\n{expected}"
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
