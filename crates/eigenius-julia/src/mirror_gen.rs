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
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const GENERATOR_ID: &str = "eigon-julia-gen";
pub(crate) const TARGET_MODULE_NAME: &str = "EigeniusMirror";
const TARGET_FILE_PATH: &str = "src/EigeniusMirror.jl";
const TARGET_PROJECT_TOML_PATH: &str = "Project.toml";

/// Stable v4-shaped UUID for the generated `EigeniusMirror` Julia
/// package. Julia's `Pkg` requires every package to declare a `uuid`
/// in its `Project.toml`; the generator emits a single fixed value so
/// the produced mirror is byte-identical across runs. Refined to a
/// per-source-layer derived UUID once we generate multiple mirrors per
/// runtime (D27 §3.6 future-work).
const TARGET_PACKAGE_UUID: &str = "8a7b6c5d-4e3f-4a1b-9c8d-7e6f5a4b3c2d";

/// UUID of the hand-authored `EigeniusJuliaCommon` package the
/// generated mirror depends on. Must match
/// `julia/common/EigeniusJuliaCommon/Project.toml`.
const COMMON_PACKAGE_UUID: &str = "9c8e7a4e-1f2b-4c3d-9e5f-6a7b8c9d0e1f";

/// UUID of the third-party `CBOR.jl` package — the same registry
/// UUID the worker's project pins (see
/// `julia/runtime-worker/Project.toml`). The mirror module imports
/// `CBOR` only to reference `CBOR.Tag` in the inductive-decoder
/// tag-peeling overload (D32 §3.7 / Phase 19d.0.c); without it,
/// dispatching a chain-side `Value::Json` payload (which Eigon-CBOR
/// wraps in tag `27182`) into a `decode_<T>(d::AbstractDict)` method
/// fails to match.
const CBOR_PACKAGE_UUID: &str = "7f3e1038-61bc-5414-967e-017c9d82adda";

const TARGET_PACKAGE_VERSION: &str = "0.1.0";

// Core ontology IRIs the generator reads. Pinned as constants so a
// chain rename of the core ontology surfaces as a compile-time edit
// rather than a silent runtime drift.
const PROP_SHORT_NAME: &str = "urn:eigenius:core:short_name";
const PROP_REQUIRES: &str = "urn:eigenius:core:requires";
const PROP_RECOMMENDS: &str = "urn:eigenius:core:recommends";
const PROP_SUBCLASS_OF: &str = "urn:eigenius:core:subclass_of";
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
/// `data_type` IRI declaring a property carries an inductive value
/// (D32 §3.5). The property's `class_types` declares which
/// `core:InductiveType` the value is shaped against; the closure walker
/// mirrors that InductiveType into Julia (§4.2 of this module).
const TYPE_INDUCTIVE: &str = "urn:eigenius:core:inductive";

/// Property IRIs on `InductiveType` / `InductiveCtor` / `InductiveArgType`
/// that the mirror generator reads to emit Julia for an inductive
/// declaration. Pinned as constants so an ontology rename surfaces as
/// a compile-time edit rather than a silent drift.
const PROP_CTORS: &str = "urn:eigenius:core:ctors";
const PROP_CTOR_NAME: &str = "urn:eigenius:core:ctor_name";
const PROP_ARG_TYPES: &str = "urn:eigenius:core:arg_types";
const PROP_TYPE_NAME: &str = "urn:eigenius:core:type_name";
const PROP_ARG_NAME: &str = "urn:eigenius:core:arg_name";
const CLASS_INDUCTIVE_TYPE: &str = "urn:eigenius:core:InductiveType";

/// Property IRI we stamp on every encoded resource so the receiver can
/// re-validate the class. Mirrors the kernel's `is_a` convention.
const PROP_IS_A: &str = "urn:eigenius:core:is_a";

/// JSON-LD-shaped key for resource identity in the codec dict shape.
/// Per D29 §8.4 the mirror struct exposes this as the `_id` field
/// (a recommended-style optional slot) so a chain resource's IRI
/// round-trips through decode/encode.
const KEY_AT_ID: &str = "@id";

/// Reserved property short_name. The mirror generator owns this slot
/// for the `@id` round-trip; ontology authors must not declare a
/// property with this short_name. Pinned by D29 §11.1.
const RESERVED_FIELD_ID: &str = "_id";

/// Prefix for format IRIs in the core ontology. Format IRIs end in
/// the format short_name (e.g. `urn:eigenius:core:formats:date` →
/// `:date`).
const FORMAT_IRI_PREFIX: &str = "urn:eigenius:core:formats:";

// `RuntimePackageMirror` class + property IRIs from the runtime
// substrate ontology. Pinned as constants so any rename in the
// ontology surfaces as a compile-time edit rather than a silent drift.
const CLASS_RUNTIME_PACKAGE_MIRROR: &str = "urn:eigenius:runtime:RuntimePackageMirror";
const PROP_DESCRIPTION: &str = "urn:eigenius:core:description";
const PROP_MIRROR_LANGUAGE: &str = "urn:eigenius:runtime:language";
const PROP_MIRROR_SOURCE_LAYER: &str = "urn:eigenius:runtime:source_layer";
const PROP_MIRROR_GEN_ID: &str = "urn:eigenius:runtime:generator_identifier";
const PROP_MIRROR_GEN_VERSION: &str = "urn:eigenius:runtime:generator_version";
const PROP_MIRROR_GEN_CONTENT_HASH: &str = "urn:eigenius:runtime:generator_content_hash";
const PROP_MIRROR_LIB_CONTENT_HASH: &str = "urn:eigenius:runtime:library_content_hash";
const PROP_MIRROR_LIB_CONTENT: &str = "urn:eigenius:runtime:library_content";
const PROP_MIRRORED_CLASSES: &str = "urn:eigenius:runtime:mirrored_classes";
const PROP_MIRROR_GENERATED_AT: &str = "urn:eigenius:runtime:generated_at";

/// Language tag stamped on every produced mirror.
const LANGUAGE_JULIA: &str = "julia";

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
        // The ontology's `generator_content_hash` regex pins the value
        // to `^sha256:[a-f0-9]{64}$`. Until we wire up a real binary
        // hash (D26 §7.2 future-work), derive the hash deterministically
        // from `(generator_id, version)` so the integrity-chain shape
        // is correct and the value is stable across runs.
        let mut hasher = Sha256::new();
        hasher.update(GENERATOR_ID.as_bytes());
        hasher.update(b":");
        hasher.update(version.as_bytes());
        let content_hash = format!("sha256:{:x}", hasher.finalize());
        Self {
            version,
            content_hash,
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
        //    resource-typed properties' class_types and through every
        //    class's `subclass_of` ancestors (D29 §3.1, §3.2). The
        //    walker classifies each visited IRI as either a `Class`
        //    (mirrored as Julia structs, the existing pipeline) or an
        //    `InductiveType` (D32 §3.6 — mirrored as abstract +
        //    per-ctor structs in the parallel pipeline below).
        let ClosureResult {
            classes: closure,
            inductives: inductive_closure,
        } = walk_closure(request)?;

        // 2. Resolve each class in the closure and gather its property
        //    metadata. Indexed by class IRI for stable lookup.
        //    Multi-supertype classes are rejected at this step.
        let class_decls = resolve_class_declarations(request, &closure)?;

        // 2b. Same for inductives — pull each `InductiveType` resource
        //     into its `InductiveDecl` projection.
        let inductive_decls = resolve_inductive_declarations(request, &inductive_closure)?;

        // 3. Compute the transitive field layout per class (own +
        //    inherited via subclass_of, deduplicated by property IRI
        //    with first-declared winning, short_name conflicts
        //    rejected). Per D29 §11.1.
        let layouts = compute_class_layouts(&class_decls)?;

        // 4. Build the parent → concrete-descendants map driving
        //    codec helper emission (D29 §8.3) and union-type leaves.
        let concrete_descendants = compute_concrete_descendants(&class_decls);

        // 5. Topologically sort:
        //    - Abstract types: parent before child via subclass_of.
        //    - Concrete structs: a class's struct after every class
        //      its fields reference (Julia struct field types must
        //      resolve at parse time; `Abstract<X>` declarations are
        //      already emitted by Phase 1, so this only constrains
        //      struct-vs-struct referencing).
        let abstract_order = abstract_emission_order(&class_decls)?;
        let struct_order = topological_order(&class_decls)?;

        // 6. Emit the Julia source + the Project.toml that turns it
        //    into an installable Julia package.
        let source = emit_module(
            &class_decls,
            &layouts,
            &abstract_order,
            &struct_order,
            &concrete_descendants,
            &inductive_decls,
            request,
        );
        let project_toml = emit_project_toml();

        // The mirrored set is the union — Class struct order plus
        // every inductive's IRI in stable BTreeSet order. Both kinds
        // are content-addressed via the same library_content_hash.
        let mut mirrored = struct_order.to_vec();
        for ind in inductive_decls.values() {
            mirrored.push(ind.iri.clone());
        }
        Ok(MirrorGenerationOutput {
            mirrored_classes: mirrored,
            library: LibraryContent::Embedded(vec![
                LibraryFile {
                    path: TARGET_PROJECT_TOML_PATH.to_string(),
                    content: project_toml.into_bytes(),
                },
                LibraryFile {
                    path: TARGET_FILE_PATH.to_string(),
                    content: source.into_bytes(),
                },
            ]),
        })
    }
}

/// Emit the `Project.toml` for the generated mirror package. The
/// produced bytes are deterministic — same generator version produces
/// the same `Project.toml` byte-for-byte.
fn emit_project_toml() -> String {
    format!(
        "# Auto-generated by eigon-julia-gen — DO NOT EDIT.\n\
         name = \"{TARGET_MODULE_NAME}\"\n\
         uuid = \"{TARGET_PACKAGE_UUID}\"\n\
         authors = [\"The Eigenius Authors\"]\n\
         version = \"{TARGET_PACKAGE_VERSION}\"\n\
         \n\
         [deps]\n\
         EigeniusJuliaCommon = \"{COMMON_PACKAGE_UUID}\"\n\
         CBOR = \"{CBOR_PACKAGE_UUID}\"\n\
         \n\
         [compat]\n\
         julia = \"1.10\"\n",
    )
}

/// Construct the `RuntimePackageMirror` resource that anchors a
/// generated mirror in the chain. Required at image-build time per
/// D26 §5.4 / §7 — the resource IRI is what the env image's
/// `mirror-iri` provenance file points at.
///
/// `generated_at` is caller-supplied so the timestamp can be
/// deterministic in tests (`"1970-01-01T00:00:00Z"`) while production
/// callers stamp the wall clock. The mirror itself is byte-identical
/// without it; the property is recommended-only (audit-grade).
pub fn mirror_to_resource(
    generator: &dyn MirrorGenerator,
    output: &MirrorGenerationOutput,
    source_layer: &Iri,
    generated_at: Option<&str>,
) -> Resource {
    let library_content_hash = compute_library_content_hash(&output.library);
    let library_json = library_content_to_json(&output.library);
    let mirror_iri = derive_mirror_iri(&library_content_hash);

    let mut r = Resource::new(mirror_iri);
    r.set(
        Iri::parse(PROP_IS_A).expect("static IRI"),
        Value::Array(vec![Value::ResourceRef(
            Iri::parse(CLASS_RUNTIME_PACKAGE_MIRROR).expect("static IRI"),
        )]),
    );
    r.set(
        Iri::parse(PROP_SHORT_NAME).expect("static IRI"),
        Value::String(TARGET_MODULE_NAME.to_string()),
    );
    r.set(
        Iri::parse(PROP_DESCRIPTION).expect("static IRI"),
        Value::String(format!(
            "Generated Julia mirror covering {} class(es) from {}.",
            output.mirrored_classes.len(),
            source_layer.as_str(),
        )),
    );
    r.set(
        Iri::parse(PROP_MIRROR_LANGUAGE).expect("static IRI"),
        Value::String(LANGUAGE_JULIA.to_string()),
    );
    r.set(
        Iri::parse(PROP_MIRROR_SOURCE_LAYER).expect("static IRI"),
        Value::String(source_layer.as_str().to_string()),
    );
    r.set(
        Iri::parse(PROP_MIRROR_GEN_ID).expect("static IRI"),
        Value::String(generator.generator_identifier().to_string()),
    );
    r.set(
        Iri::parse(PROP_MIRROR_GEN_VERSION).expect("static IRI"),
        Value::String(generator.generator_version().to_string()),
    );
    r.set(
        Iri::parse(PROP_MIRROR_GEN_CONTENT_HASH).expect("static IRI"),
        Value::String(generator.generator_content_hash().to_string()),
    );
    r.set(
        Iri::parse(PROP_MIRROR_LIB_CONTENT_HASH).expect("static IRI"),
        Value::String(library_content_hash),
    );
    r.set(
        Iri::parse(PROP_MIRROR_LIB_CONTENT).expect("static IRI"),
        Value::Json(library_json),
    );
    r.set(
        Iri::parse(PROP_MIRRORED_CLASSES).expect("static IRI"),
        Value::Array(
            output
                .mirrored_classes
                .iter()
                .cloned()
                .map(Value::ResourceRef)
                .collect(),
        ),
    );
    if let Some(ts) = generated_at {
        r.set(
            Iri::parse(PROP_MIRROR_GENERATED_AT).expect("static IRI"),
            Value::String(ts.to_string()),
        );
    }
    r
}

/// SHA-256 over the library archive's bytes. For `Embedded`, the hash
/// covers each `(path, content)` pair in path-sorted order with a
/// length-prefix between fields so a path/content swap produces a
/// different digest. For `External`, the hash is the caller-supplied
/// `content_hash` (already SHA-256 of the referenced bytes).
fn compute_library_content_hash(library: &LibraryContent) -> String {
    match library {
        LibraryContent::Embedded(files) => {
            let mut sorted: Vec<&LibraryFile> = files.iter().collect();
            sorted.sort_by(|a, b| a.path.cmp(&b.path));
            let mut hasher = Sha256::new();
            for f in sorted {
                let path_bytes = f.path.as_bytes();
                hasher.update((path_bytes.len() as u64).to_be_bytes());
                hasher.update(path_bytes);
                hasher.update((f.content.len() as u64).to_be_bytes());
                hasher.update(&f.content);
            }
            format!("sha256:{:x}", hasher.finalize())
        }
        LibraryContent::External { content_hash, .. } => content_hash.clone(),
    }
}

/// Encode a `LibraryContent` as JSON for the `library_content`
/// property. Embedded archives become `{"kind": "embedded", "files":
/// [{"path": ..., "content_b64": ...}, ...]}`; external references
/// become `{"kind": "external", "reference": ..., "content_hash":
/// ...}`. The substrate's image-build pipeline parses this back when
/// it materialises the mirror.
fn library_content_to_json(library: &LibraryContent) -> serde_json::Value {
    match library {
        LibraryContent::Embedded(files) => {
            let mut sorted: Vec<&LibraryFile> = files.iter().collect();
            sorted.sort_by(|a, b| a.path.cmp(&b.path));
            let arr: Vec<serde_json::Value> = sorted
                .into_iter()
                .map(|f| {
                    serde_json::json!({
                        "path": f.path,
                        "content_b64": base64_encode(&f.content),
                    })
                })
                .collect();
            serde_json::json!({
                "kind": "embedded",
                "files": arr,
            })
        }
        LibraryContent::External {
            reference,
            content_hash,
        } => serde_json::json!({
            "kind": "external",
            "reference": reference,
            "content_hash": content_hash,
        }),
    }
}

/// Standard base64 (no padding stripping, no URL-safe alphabet — RFC
/// 4648 §4). Hand-rolled so the crate keeps a tiny dep set; the
/// alphabet is fixed and the encoder is one screen, so a dep would
/// just trade transitive churn for nothing.
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let b = &bytes[i..i + 3];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        out.push(ALPHABET[(n & 0x3f) as usize] as char);
        i += 3;
    }
    let rem = bytes.len() - i;
    if rem == 1 {
        let n = (bytes[i] as u32) << 16;
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rem == 2 {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8);
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        out.push('=');
    }
    out
}

/// Stable IRI for the generated mirror, derived from the library
/// content hash so byte-identical mirrors produce byte-identical
/// IRIs. The first 16 hex chars of the hash are enough for collision
/// safety in the practical generator-output universe; the full hash
/// stays on `library_content_hash` for full integrity checks.
fn derive_mirror_iri(library_content_hash: &str) -> Iri {
    let short = library_content_hash
        .strip_prefix("sha256:")
        .unwrap_or(library_content_hash)
        .chars()
        .take(16)
        .collect::<String>();
    Iri::parse(&format!("urn:eigenius:runtime:mirror:julia:{short}"))
        .expect("derived mirror IRI is well-formed")
}

/// Class declaration in the form the emitter consumes. `requires` and
/// `recommends` are pre-resolved to property declarations so the
/// emitter doesn't re-walk the chain. Multi-supertype is rejected at
/// resolution time (D29 §3.2 / §11.1) so `subclass_of` is at most one.
struct ClassDecl {
    iri: Iri,
    short_name: String,
    /// The class's directly-declared `requires` (in declaration
    /// order). The full inherited field set is computed separately
    /// in [`compute_class_layout`] using the abstract-class chain.
    requires: Vec<PropertyDecl>,
    /// The class's directly-declared `recommends` (in declaration
    /// order).
    recommends: Vec<PropertyDecl>,
    /// Direct supertype IRI, when this class declares
    /// `core:subclass_of`. At most one entry — multi-supertype is
    /// rejected at resolution time.
    subclass_of: Option<Iri>,
}

/// Inductive type declaration in the form the emitter consumes
/// (D32 §3.6). Built from a chain `core:InductiveType` resource by
/// [`resolve_inductive_declarations`]. A future parametric extension
/// would carry `type_params` here; v1 covers monomorphic inductives.
struct InductiveDecl {
    iri: Iri,
    short_name: String,
    ctors: Vec<InductiveCtorDecl>,
}

/// One ctor on an inductive type. `args` carries the ordered argument
/// declarations from the chain's `core:arg_types` list.
struct InductiveCtorDecl {
    /// Ctor name as it appears in the chain JSON (e.g. `"zero"`,
    /// `"succ"`). The emitted concrete struct's Julia name is
    /// derived from this — see [`inductive_ctor_struct_name`].
    ctor_name: String,
    args: Vec<InductiveArgDecl>,
}

/// One argument slot on an inductive ctor.
struct InductiveArgDecl {
    /// Optional readable name from `core:arg_name` (D32 §3.2). When
    /// absent, the emitter generates a positional fallback
    /// (`arg_0`, `arg_1`, …).
    arg_name: Option<String>,
    /// Verbatim `core:type_name` string. Resolves at emit time:
    /// primitive type IRI → Julia primitive (Float64, String, …);
    /// InductiveType IRI → the abstract Julia type for that inductive;
    /// Class IRI → `Any` for v1 (Class-typed arg values are not
    /// re-validated by the inductive's decoder, only by the chain
    /// validator).
    type_name: String,
}

/// One property's contribution to a struct field.
#[derive(Clone)]
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
#[derive(Default, Debug, Clone)]
struct PropertyConstraints {
    min_value: Option<f64>,
    max_value: Option<f64>,
    min_length: Option<i64>,
    max_length: Option<i64>,
    pattern: Option<String>,
    /// Format reference, when the property declares `core:format`.
    /// Renders to a Julia `Symbol` literal at validation time
    /// (D29 §9.3): standard `urn:eigenius:core:formats:<name>`
    /// IRIs become `:<name>` (e.g. `:date`); any other IRI is passed
    /// through as `Symbol("<full IRI>")` so the validator can raise
    /// loudly on unknown formats rather than the generator silently
    /// dropping the constraint.
    format: Option<FormatRef>,
}

#[derive(Debug, Clone)]
enum FormatRef {
    /// IRI under `urn:eigenius:core:formats:` — the tail (e.g.
    /// `"date"`) renders as `:date`.
    Standard(String),
    /// Any other format IRI — renders as `Symbol("<full IRI>")`.
    Custom(String),
}

impl FormatRef {
    /// Render as a Julia symbol expression for use as the third
    /// argument to `validate_format`.
    fn as_julia_symbol_expr(&self) -> String {
        match self {
            FormatRef::Standard(name) => format!(":{name}"),
            FormatRef::Custom(iri) => format!("Symbol({})", julia_string_literal(iri)),
        }
    }
}

/// The Julia type a property's `data_type` maps to. Format
/// constraints don't affect the type here (they're handled by the
/// validating constructors in §9 of D29).
#[derive(Debug, Clone)]
enum JuliaType {
    Primitive(&'static str),
    /// Reference to another mirror struct by class IRI.
    StructRef(Iri),
    /// Reference to a chain-committed `InductiveType` (D32 §3.6).
    /// Emitted for properties whose `data_type` is `core:inductive`.
    /// Renders as the inductive's bare short_name (e.g. `FormulaTerm`)
    /// — the Julia abstract type the per-ctor concrete structs
    /// extend, accepted polymorphically as a struct field type.
    InductiveRef(Iri),
    /// `Union{<C₁>, …, <Cₙ>}` — emitted when a `core:resource` (or
    /// the inner element of a `core:resource_array`) lists more than
    /// one class in `class_types`. IRIs are stored in IRI-sort order
    /// so the rendered Julia source is deterministic (D29 §4).
    UnionRef(Vec<Iri>),
    /// `Vector{<inner>}` — one level of nesting per v1.
    Vector(Box<JuliaType>),
}

impl JuliaType {
    /// Render to a Julia type expression — used in field declarations
    /// and constructor signatures. Class references render as the
    /// `Abstract<C>` slot from the abstract+struct pair (D29 §7), so
    /// fields uniformly accept any concrete subtype of the declared
    /// class.
    fn render(
        &self,
        class_lookup: &BTreeMap<Iri, String>,
        inductive_lookup: &BTreeMap<Iri, String>,
    ) -> String {
        match self {
            JuliaType::Primitive(s) => (*s).to_string(),
            JuliaType::StructRef(iri) => class_abstract_name(iri, class_lookup),
            JuliaType::InductiveRef(iri) => inductive_lookup
                .get(iri)
                .cloned()
                .unwrap_or_else(|| sanitise_for_identifier(iri.as_str())),
            JuliaType::UnionRef(iris) => {
                let inners: Vec<String> = iris
                    .iter()
                    .map(|i| class_abstract_name(i, class_lookup))
                    .collect();
                format!("Union{{{}}}", inners.join(", "))
            }
            JuliaType::Vector(inner) => {
                format!("Vector{{{}}}", inner.render(class_lookup, inductive_lookup))
            }
        }
    }

    /// Class IRIs the type references — drives the closure walker,
    /// topological sort, and Union-helper leaves enumeration.
    /// `InductiveRef` deliberately returns nothing: inductives don't
    /// gate the class topological sort (their forward-declared
    /// abstract types are emitted before any concrete struct).
    fn struct_refs(&self) -> Vec<Iri> {
        match self {
            JuliaType::Primitive(_) => Vec::new(),
            JuliaType::StructRef(iri) => vec![iri.clone()],
            JuliaType::InductiveRef(_) => Vec::new(),
            JuliaType::UnionRef(iris) => iris.clone(),
            JuliaType::Vector(inner) => inner.struct_refs(),
        }
    }

    /// Inner element when this type is a `Vector{...}` (one level).
    /// Used by the codec to peel `Vector` and produce array-comp
    /// expressions over the inner type's leaves.
    fn vector_inner(&self) -> Option<&JuliaType> {
        match self {
            JuliaType::Vector(inner) => Some(inner.as_ref()),
            _ => None,
        }
    }
}

/// Concrete struct name for a class IRI — the name of the actual
/// `struct C` in the emitted source, used in codec function names
/// (`encode_C` / `decode_C`) and `isa` checks. Per D29 §7 this is the
/// class's `short_name` verbatim.
fn class_short_name(iri: &Iri, class_lookup: &BTreeMap<Iri, String>) -> String {
    class_lookup
        .get(iri)
        .cloned()
        .unwrap_or_else(|| sanitise_for_identifier(iri.as_str()))
}

/// Abstract type name for a class IRI — the `Abstract<C>` slot in
/// the abstract+struct pair, used in field-type positions (D29 §7).
fn class_abstract_name(iri: &Iri, class_lookup: &BTreeMap<Iri, String>) -> String {
    format!("Abstract{}", class_short_name(iri, class_lookup))
}

/// Set of concrete class IRIs a value of `t` can hold at runtime —
/// the union of `concrete_descendants[c]` for each `c` in `t`'s
/// struct_refs. Drives codec helper emission (D29 §8.3): one leaf →
/// direct `decode_C` / `encode_C` call; multiple leaves → emit a
/// per-field `_decode_<C>_<f>` / `_encode_<C>_<f>` dispatcher.
fn type_leaves(
    t: &JuliaType,
    concrete_descendants: &BTreeMap<Iri, BTreeSet<Iri>>,
) -> BTreeSet<Iri> {
    let mut leaves = BTreeSet::new();
    for class_iri in t.struct_refs() {
        if let Some(d) = concrete_descendants.get(&class_iri) {
            leaves.extend(d.iter().cloned());
        } else {
            // Class not in closure — defensive; closure walk should
            // have pulled it. Treat as singleton leaf.
            leaves.insert(class_iri);
        }
    }
    leaves
}

/// Closure walk result — separates `Class` IRIs (mirrored as Julia
/// structs) from `InductiveType` IRIs (mirrored as Julia abstract +
/// per-ctor concrete structs, D32 §3.6). The two flow through parallel
/// emission pipelines and are split here so emit-time logic doesn't
/// have to re-classify.
struct ClosureResult {
    classes: BTreeSet<Iri>,
    inductives: BTreeSet<Iri>,
}

fn walk_closure(request: &MirrorGenerationRequest) -> Result<ClosureResult, MirrorGeneratorError> {
    let mut classes: BTreeSet<Iri> = BTreeSet::new();
    let mut inductives: BTreeSet<Iri> = BTreeSet::new();
    let mut queue: Vec<Iri> = request.seed_classes.to_vec();

    while let Some(iri) = queue.pop() {
        // Already visited (in either bucket)?
        if classes.contains(&iri) || inductives.contains(&iri) {
            continue;
        }

        let def = request
            .chain
            .resolve(request.source_layer, &iri)
            .ok_or_else(|| MirrorGeneratorError::UnknownClass(iri.as_str().to_string()))?;

        // Classify by `is_a`. An InductiveType lives in a parallel
        // emit pipeline; everything else is treated as a Class.
        let is_inductive = iri_array(&def, PROP_IS_A)
            .iter()
            .any(|t| t.as_str() == CLASS_INDUCTIVE_TYPE);

        if is_inductive {
            inductives.insert(iri.clone());
            // Walk arg_types[].type_name to pull in transitively-
            // referenced inductives or classes (e.g. FormulaTerm's
            // ctors reference FormulaTerm itself; OpRef references
            // a Class via its iri arg). Self-references are absorbed
            // by the visited check.
            for ctor in resource_array(&def, PROP_CTORS) {
                for arg_type in resource_array(&ctor, PROP_ARG_TYPES) {
                    if let Some(tn) = string_value(&arg_type, PROP_TYPE_NAME) {
                        if let Ok(target) = Iri::parse(&tn) {
                            if !is_core_meta_iri(&target)
                                && !is_core_primitive_iri(&target)
                                && !classes.contains(&target)
                                && !inductives.contains(&target)
                            {
                                queue.push(target);
                            }
                        }
                    }
                    // Recurse into nested type_args for parametric
                    // applications — best-effort; a fully-fledged
                    // parametric walk lands when the first parametric
                    // chain consumer materialises.
                }
            }
            continue;
        }

        classes.insert(iri.clone());

        // Walk required + recommended property class_types →
        // referenced classes / inductives.
        for prop_iri in iri_array(&def, PROP_REQUIRES)
            .into_iter()
            .chain(iri_array(&def, PROP_RECOMMENDS))
        {
            let prop_def = match request.chain.resolve(request.source_layer, &prop_iri) {
                Some(r) => r,
                None => continue,
            };
            for r in property_class_references(&prop_def) {
                if is_core_meta_iri(&r) || classes.contains(&r) || inductives.contains(&r) {
                    continue;
                }
                queue.push(r);
            }
        }

        // Walk `subclass_of` ancestors transitively (D29 §3.2).
        for parent in iri_array(&def, PROP_SUBCLASS_OF) {
            if is_core_meta_iri(&parent)
                || classes.contains(&parent)
                || inductives.contains(&parent)
            {
                continue;
            }
            queue.push(parent);
        }
    }

    Ok(ClosureResult {
        classes,
        inductives,
    })
}

/// True for the seven primitive type IRIs the validator and emitter
/// special-case. Used by the closure walker to skip primitive-type
/// references (they don't need mirror emission).
fn is_core_primitive_iri(iri: &Iri) -> bool {
    matches!(
        iri.as_str(),
        TYPE_STRING
            | TYPE_INTEGER
            | TYPE_FLOAT
            | TYPE_BOOLEAN
            | TYPE_RESOURCE
            | TYPE_RESOURCE_ARRAY
            | TYPE_VALUE_ARRAY
            | TYPE_JSON
            | TYPE_INDUCTIVE
    )
}

/// Read a property whose value is an array of embedded resources.
/// Empty when the property is missing or its value isn't an array.
fn resource_array(r: &Resource, prop_iri: &str) -> Vec<Resource> {
    let iri = match Iri::parse(prop_iri) {
        Ok(i) => i,
        Err(_) => return Vec::new(),
    };
    match r.get(&iri) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| match v {
                Value::Embedded(r) => Some(r.as_ref().clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// True when `iri` lives under the `urn:eigenius:core:` namespace —
/// i.e. it's a core-ontology meta-class or meta-property (`Class`,
/// `Property`, `is_a`, `ConditionalRequirement`, …) rather than a
/// user-data class. The mirror generator emits Julia struct types for
/// user-data classes; meta classes never get a struct type because
/// the kernel's type-system shape is implicitly known to the
/// hand-authored `EigeniusJuliaCommon` package the generated mirror
/// imports. Skipping these in the closure walk avoids both redundant
/// code emission and the recursion that pulls
/// `ConditionalRequirement.has_value` (a `value_array` with no
/// `class_types`, which is correct for its meta role but incompatible
/// with the generator's expectation of mirrored properties).
fn is_core_meta_iri(iri: &Iri) -> bool {
    iri.as_str().starts_with("urn:eigenius:core:")
}

fn property_class_references(prop_def: &Resource) -> Vec<Iri> {
    let dt = match resource_iri_value(prop_def, PROP_DATA_TYPE) {
        Some(iri) => iri,
        None => return Vec::new(),
    };
    match dt.as_str() {
        // `data_type: core:inductive` (D32 §3.5) routes the same as
        // resource-typed properties — `class_types` declares the
        // referent (an `InductiveType`), and the closure walker pulls
        // it into the inductive bucket via its `is_a` discriminator.
        TYPE_RESOURCE | TYPE_RESOURCE_ARRAY | TYPE_INDUCTIVE => {
            iri_array(prop_def, PROP_CLASS_TYPES)
        }
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
        let subclass_of = resolve_subclass_of(&class_def, class_iri)?;

        decls.insert(
            class_iri.clone(),
            ClassDecl {
                iri: class_iri.clone(),
                short_name,
                requires,
                recommends,
                subclass_of,
            },
        );
    }
    Ok(decls)
}

/// Read `core:subclass_of` from a class declaration.
///
/// D29 §3.2: at most one supertype is permitted in v1.1 — Julia has
/// single-inheritance abstract types and multi-inheritance has no
/// faithful Julia mapping. Multiple entries → `UnrepresentableClass`.
fn resolve_subclass_of(
    class_def: &Resource,
    class_iri: &Iri,
) -> Result<Option<Iri>, MirrorGeneratorError> {
    let parents = iri_array(class_def, PROP_SUBCLASS_OF);
    match parents.len() {
        0 => Ok(None),
        1 => {
            let parent = parents.into_iter().next().expect("len 1");
            if &parent == class_iri {
                return Err(MirrorGeneratorError::UnrepresentableClass {
                    class_iri: class_iri.as_str().to_string(),
                    language: "julia".to_string(),
                    reason: "class declares itself as its own supertype".to_string(),
                });
            }
            Ok(Some(parent))
        }
        _ => Err(MirrorGeneratorError::UnrepresentableClass {
            class_iri: class_iri.as_str().to_string(),
            language: "julia".to_string(),
            reason: format!(
                "class declares {} supertypes via `subclass_of`; Julia's single-inheritance \
                 abstract types support at most one (D29 §3.2 / §11.1)",
                parents.len()
            ),
        }),
    }
}

/// A class's effective field set after walking its `subclass_of`
/// chain. Properties from ancestors come first (root-to-leaf), then
/// the class's own properties; deduplicated by property IRI with
/// first-declared winning. Pinned by D29 §3.2 / §7.
#[derive(Clone)]
struct ClassLayout {
    requires: Vec<PropertyDecl>,
    recommends: Vec<PropertyDecl>,
}

/// Resolve every `InductiveType` IRI in `closure` into an
/// [`InductiveDecl`] suitable for the emitter (D32 §3.6). Each
/// inductive's `ctors` and per-ctor `arg_types` are pulled in
/// declaration order; `arg_name` is read when present (`None` falls
/// back to positional names at emit time).
fn resolve_inductive_declarations(
    request: &MirrorGenerationRequest,
    closure: &BTreeSet<Iri>,
) -> Result<BTreeMap<Iri, InductiveDecl>, MirrorGeneratorError> {
    let mut decls = BTreeMap::new();
    for ind_iri in closure {
        let def = request
            .chain
            .resolve(request.source_layer, ind_iri)
            .ok_or_else(|| MirrorGeneratorError::UnknownClass(ind_iri.as_str().to_string()))?;

        let short_name = string_value(&def, PROP_SHORT_NAME).ok_or_else(|| {
            MirrorGeneratorError::UnrepresentableClass {
                class_iri: ind_iri.as_str().to_string(),
                language: "julia".to_string(),
                reason: "InductiveType missing `core:short_name`".into(),
            }
        })?;

        let mut ctors = Vec::new();
        for ctor_res in resource_array(&def, PROP_CTORS) {
            let ctor_name = string_value(&ctor_res, PROP_CTOR_NAME).ok_or_else(|| {
                MirrorGeneratorError::UnrepresentableClass {
                    class_iri: ind_iri.as_str().to_string(),
                    language: "julia".to_string(),
                    reason: format!(
                        "InductiveCtor on `{}` missing `core:ctor_name`",
                        ind_iri.as_str()
                    ),
                }
            })?;

            let mut args = Vec::new();
            for arg_res in resource_array(&ctor_res, PROP_ARG_TYPES) {
                let type_name = string_value(&arg_res, PROP_TYPE_NAME).ok_or_else(|| {
                    MirrorGeneratorError::UnrepresentableClass {
                        class_iri: ind_iri.as_str().to_string(),
                        language: "julia".to_string(),
                        reason: format!(
                            "InductiveArgType on ctor `{ctor_name}` missing `core:type_name`"
                        ),
                    }
                })?;
                args.push(InductiveArgDecl {
                    arg_name: string_value(&arg_res, PROP_ARG_NAME),
                    type_name,
                });
            }
            ctors.push(InductiveCtorDecl { ctor_name, args });
        }

        decls.insert(
            ind_iri.clone(),
            InductiveDecl {
                iri: ind_iri.clone(),
                short_name,
                ctors,
            },
        );
    }
    Ok(decls)
}

/// Compute the transitive field layout for every class in `decls` by
/// walking each class's `subclass_of` chain root-to-leaf and unioning
/// `requires`/`recommends` with first-declared dedup on property IRI.
/// Detects:
/// - `subclass_of` cycles → `UnrepresentableClass`.
/// - `short_name` conflicts within a class's transitive field set
///   (D29 §11.1) → `UnrepresentableClass`.
fn compute_class_layouts(
    decls: &BTreeMap<Iri, ClassDecl>,
) -> Result<BTreeMap<Iri, ClassLayout>, MirrorGeneratorError> {
    enum Mark {
        InProgress,
        Done,
    }
    let mut layouts: BTreeMap<Iri, ClassLayout> = BTreeMap::new();
    let mut marks: BTreeMap<Iri, Mark> = BTreeMap::new();

    fn compute(
        iri: &Iri,
        decls: &BTreeMap<Iri, ClassDecl>,
        layouts: &mut BTreeMap<Iri, ClassLayout>,
        marks: &mut BTreeMap<Iri, Mark>,
    ) -> Result<ClassLayout, MirrorGeneratorError> {
        if let Some(l) = layouts.get(iri) {
            return Ok(l.clone());
        }
        match marks.get(iri) {
            Some(Mark::Done) => return Ok(layouts.get(iri).expect("done implies layout").clone()),
            Some(Mark::InProgress) => {
                return Err(MirrorGeneratorError::UnrepresentableClass {
                    class_iri: iri.as_str().to_string(),
                    language: "julia".to_string(),
                    reason: "class participates in a `subclass_of` cycle (D29 §3.2 / §11.1)"
                        .to_string(),
                });
            }
            None => {}
        }
        marks.insert(iri.clone(), Mark::InProgress);

        let decl = decls.get(iri).expect("layout target must be in decls");
        let mut requires: Vec<PropertyDecl> = Vec::new();
        let mut recommends: Vec<PropertyDecl> = Vec::new();
        let mut seen_iris: BTreeSet<Iri> = BTreeSet::new();

        // Inherit from parent first (root-to-leaf order).
        if let Some(parent) = &decl.subclass_of {
            let parent_layout = compute(parent, decls, layouts, marks)?;
            for p in &parent_layout.requires {
                if seen_iris.insert(p.iri.clone()) {
                    requires.push(p.clone());
                }
            }
            for p in &parent_layout.recommends {
                if seen_iris.insert(p.iri.clone()) {
                    recommends.push(p.clone());
                }
            }
        }
        // Own declarations come last; dedup against ancestors.
        for p in &decl.requires {
            if seen_iris.insert(p.iri.clone()) {
                requires.push(p.clone());
            }
        }
        for p in &decl.recommends {
            if seen_iris.insert(p.iri.clone()) {
                recommends.push(p.clone());
            }
        }

        // D29 §11.1: short_name uniqueness across the transitive
        // field set. Two distinct property IRIs with the same
        // short_name produce invalid Julia (duplicate struct field
        // name) — reject loudly so the chain author can fix the
        // ontology.
        let mut name_to_iri: BTreeMap<String, Iri> = BTreeMap::new();
        for p in requires.iter().chain(recommends.iter()) {
            if let Some(prev) = name_to_iri.get(&p.short_name) {
                return Err(MirrorGeneratorError::UnrepresentableClass {
                    class_iri: iri.as_str().to_string(),
                    language: "julia".to_string(),
                    reason: format!(
                        "two distinct properties resolve to the same short_name `{}` on \
                         this class's transitive field set: `{}` and `{}` (D29 §11.1)",
                        p.short_name,
                        prev.as_str(),
                        p.iri.as_str(),
                    ),
                });
            }
            name_to_iri.insert(p.short_name.clone(), p.iri.clone());
        }

        marks.insert(iri.clone(), Mark::Done);
        let layout = ClassLayout {
            requires,
            recommends,
        };
        layouts.insert(iri.clone(), layout.clone());
        Ok(layout)
    }

    for iri in decls.keys() {
        compute(iri, decls, &mut layouts, &mut marks)?;
    }
    Ok(layouts)
}

/// Build a map from each class IRI in the closure to its inclusive
/// concrete-descendants set (the class itself plus all classes whose
/// `subclass_of` chain reaches it transitively). Used by codec
/// emission to enumerate the leaves of an `Abstract<C>`-typed field.
/// IRIs in the result set are sorted by `BTreeSet` ordering.
fn compute_concrete_descendants(decls: &BTreeMap<Iri, ClassDecl>) -> BTreeMap<Iri, BTreeSet<Iri>> {
    // Inverse relation: parent IRI → list of direct child IRIs.
    let mut children: BTreeMap<Iri, Vec<Iri>> = BTreeMap::new();
    for decl in decls.values() {
        if let Some(parent) = &decl.subclass_of {
            children
                .entry(parent.clone())
                .or_default()
                .push(decl.iri.clone());
        }
    }
    // For each class, BFS down the inverse-edge graph.
    let mut out: BTreeMap<Iri, BTreeSet<Iri>> = BTreeMap::new();
    for iri in decls.keys() {
        let mut set: BTreeSet<Iri> = BTreeSet::new();
        let mut stack: Vec<Iri> = vec![iri.clone()];
        while let Some(c) = stack.pop() {
            if !set.insert(c.clone()) {
                continue;
            }
            if let Some(kids) = children.get(&c) {
                stack.extend(kids.iter().cloned());
            }
        }
        out.insert(iri.clone(), set);
    }
    out
}

/// Topologically order classes by `subclass_of` for abstract-type
/// emission: parent's abstract type is declared before the child's.
/// Tie-breaking by IRI sort (BTreeMap iteration). Cycles → already
/// rejected by [`compute_class_layouts`]; this pass returns an
/// equivalent error if seen, defending the contract locally.
fn abstract_emission_order(
    decls: &BTreeMap<Iri, ClassDecl>,
) -> Result<Vec<Iri>, MirrorGeneratorError> {
    enum Mark {
        InProgress,
        Done,
    }
    let mut marks: BTreeMap<Iri, Mark> = BTreeMap::new();
    let mut order: Vec<Iri> = Vec::new();

    fn visit(
        iri: &Iri,
        decls: &BTreeMap<Iri, ClassDecl>,
        marks: &mut BTreeMap<Iri, Mark>,
        order: &mut Vec<Iri>,
    ) -> Result<(), MirrorGeneratorError> {
        match marks.get(iri) {
            Some(Mark::Done) => return Ok(()),
            Some(Mark::InProgress) => {
                return Err(MirrorGeneratorError::UnrepresentableClass {
                    class_iri: iri.as_str().to_string(),
                    language: "julia".to_string(),
                    reason: "subclass_of cycle while ordering abstract emission".to_string(),
                });
            }
            None => {}
        }
        marks.insert(iri.clone(), Mark::InProgress);
        if let Some(decl) = decls.get(iri) {
            if let Some(parent) = &decl.subclass_of {
                visit(parent, decls, marks, order)?;
            }
        }
        marks.insert(iri.clone(), Mark::Done);
        order.push(iri.clone());
        Ok(())
    }

    for iri in decls.keys() {
        visit(iri, decls, &mut marks, &mut order)?;
    }
    Ok(order)
}

fn resolve_properties(
    request: &MirrorGenerationRequest,
    class_def: &Resource,
    arity_prop: &str,
) -> Result<Vec<PropertyDecl>, MirrorGeneratorError> {
    let mut out = Vec::new();
    for prop_iri in iri_array(class_def, arity_prop) {
        // Skip core-namespace properties — `is_a`, `short_name`,
        // `description`, `requires`, … are meta-shape properties the
        // codec emitter already handles (the encoder stamps `is_a`
        // automatically; `@id` rides through the reserved `_id`
        // slot). Surfacing them as data fields would both produce
        // redundant struct fields and recurse into core meta-classes
        // (`is_a.class_types = [core:Class]`) that the closure
        // walker correctly excludes from the decl set.
        if is_core_meta_iri(&prop_iri) {
            continue;
        }
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
        if short_name == RESERVED_FIELD_ID {
            return Err(MirrorGeneratorError::UnrepresentableClass {
                class_iri: prop_iri.as_str().to_string(),
                language: "julia".to_string(),
                reason: format!(
                    "property short_name `{RESERVED_FIELD_ID}` is reserved by the mirror \
                     generator for the @id round-trip slot (D29 §11.1)"
                ),
            });
        }
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
        format: resource_iri_value(prop_def, PROP_FORMAT).map(|iri| {
            match iri.as_str().strip_prefix(FORMAT_IRI_PREFIX) {
                Some(name) => FormatRef::Standard(name.to_string()),
                None => FormatRef::Custom(iri.as_str().to_string()),
            }
        }),
    }
}

/// Read a numeric property as f64. Tolerates `Value::Float` and
/// `Value::Integer` (the JSON parser keeps `0` as Integer and `0.0`
/// as Float; ontology authors write either).
fn numeric_value(r: &Resource, prop_iri: &str) -> Option<f64> {
    let iri = Iri::parse(prop_iri).ok()?;
    let v = r.get(&iri)?;
    v.as_float().or_else(|| v.as_integer().map(|n| n as f64))
}

fn integer_value(r: &Resource, prop_iri: &str) -> Option<i64> {
    let iri = Iri::parse(prop_iri).ok()?;
    r.get(&iri).and_then(Value::as_integer)
}

/// Resolve a `class_types` array to a single `JuliaType`: a `StructRef`
/// when it lists exactly one class, a `UnionRef` (with IRIs sorted) when
/// it lists two or more, an error when it's empty. Used for both
/// `core:resource` (scalar) and `core:resource_array` (inner of the
/// `Vector{...}` wrapper).
fn struct_or_union_ref(
    prop_def: &Resource,
    prop_iri: &Iri,
    declared_data_type: &str,
) -> Result<JuliaType, MirrorGeneratorError> {
    let mut class_types = iri_array(prop_def, PROP_CLASS_TYPES);
    match class_types.len() {
        0 => Err(MirrorGeneratorError::UnrepresentableClass {
            class_iri: prop_iri.as_str().to_string(),
            language: "julia".to_string(),
            reason: format!(
                "data_type `{declared_data_type}` requires at least one `class_types` entry"
            ),
        }),
        1 => Ok(JuliaType::StructRef(class_types.remove(0))),
        _ => {
            // D29 §4: Union variants are emitted in IRI-sort order so
            // the rendered Julia source is deterministic regardless of
            // the chain's class_types declaration order.
            class_types.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            class_types.dedup();
            Ok(JuliaType::UnionRef(class_types))
        }
    }
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
        TYPE_RESOURCE => Ok(struct_or_union_ref(prop_def, prop_iri, TYPE_RESOURCE)?),
        TYPE_RESOURCE_ARRAY => {
            let inner = struct_or_union_ref(prop_def, prop_iri, TYPE_RESOURCE_ARRAY)?;
            Ok(JuliaType::Vector(Box::new(inner)))
        }
        TYPE_INDUCTIVE => {
            // D32 §3.5: a `core:inductive` property's `class_types`
            // declares exactly one InductiveType IRI. The Julia field
            // is typed at that inductive's abstract type — concrete
            // ctor structs subtype it polymorphically.
            let mut class_types = iri_array(prop_def, PROP_CLASS_TYPES);
            if class_types.len() != 1 {
                return Err(MirrorGeneratorError::UnrepresentableClass {
                    class_iri: prop_iri.as_str().to_string(),
                    language: "julia".to_string(),
                    reason: format!(
                        "data_type `{TYPE_INDUCTIVE}` requires exactly one `{PROP_CLASS_TYPES}` entry, got {}",
                        class_types.len(),
                    ),
                });
            }
            Ok(JuliaType::InductiveRef(class_types.remove(0)))
        }
        TYPE_VALUE_ARRAY => {
            let element_type =
                resource_iri_value(prop_def, PROP_ELEMENT_TYPE).ok_or_else(|| {
                    MirrorGeneratorError::UnrepresentableClass {
                        class_iri: prop_iri.as_str().to_string(),
                        language: "julia".to_string(),
                        reason: format!(
                            "data_type `{TYPE_VALUE_ARRAY}` requires `{PROP_ELEMENT_TYPE}`"
                        ),
                    }
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
/// Topologically sort classes so a struct that references another is
/// declared after the referenced struct. Stable on tie (BTreeMap
/// iteration order = IRI sort).
///
/// Cycles in the class graph (D29 §3.3) are rejected with
/// `UnrepresentableClass`: Julia's `struct` declaration form requires
/// forward references to be resolved at parse time, and the
/// mutable-struct + forward-declaration workaround is deferred to a
/// future spec version. v1 callers must factor cycles out of the seed
/// (typically by extracting an interface class).
fn topological_order(decls: &BTreeMap<Iri, ClassDecl>) -> Result<Vec<Iri>, MirrorGeneratorError> {
    enum Mark {
        InProgress,
        Done,
    }
    let mut marks: BTreeMap<Iri, Mark> = BTreeMap::new();
    let mut order: Vec<Iri> = Vec::new();

    fn visit(
        iri: &Iri,
        decls: &BTreeMap<Iri, ClassDecl>,
        marks: &mut BTreeMap<Iri, Mark>,
        order: &mut Vec<Iri>,
    ) -> Result<(), MirrorGeneratorError> {
        match marks.get(iri) {
            Some(Mark::Done) => return Ok(()),
            Some(Mark::InProgress) => {
                // Re-entered the same class while it's still being
                // resolved → cycle. Report the offending class IRI;
                // the caller adds context to the error chain.
                return Err(MirrorGeneratorError::UnrepresentableClass {
                    class_iri: iri.as_str().to_string(),
                    language: "julia".to_string(),
                    reason: "class participates in a cycle of resource-typed property references; \
                         Julia struct definitions cannot forward-reference and v1 of D29 does not \
                         emit mutable structs (see D29 §3.3)"
                        .to_string(),
                });
            }
            None => {}
        }
        marks.insert(iri.clone(), Mark::InProgress);
        if let Some(decl) = decls.get(iri) {
            for prop in decl.requires.iter().chain(decl.recommends.iter()) {
                for ref_iri in prop.julia_type.struct_refs() {
                    visit(&ref_iri, decls, marks, order)?;
                }
            }
        }
        marks.insert(iri.clone(), Mark::Done);
        order.push(iri.clone());
        Ok(())
    }

    // BTreeMap iteration is sorted by key — gives a stable starting
    // order, so the topological sort is deterministic.
    for iri in decls.keys() {
        visit(iri, decls, &mut marks, &mut order)?;
    }
    Ok(order)
}

fn emit_module(
    decls: &BTreeMap<Iri, ClassDecl>,
    layouts: &BTreeMap<Iri, ClassLayout>,
    abstract_order: &[Iri],
    struct_order: &[Iri],
    concrete_descendants: &BTreeMap<Iri, BTreeSet<Iri>>,
    inductive_decls: &BTreeMap<Iri, InductiveDecl>,
    request: &MirrorGenerationRequest,
) -> String {
    let class_lookup: BTreeMap<Iri, String> = decls
        .values()
        .map(|d| (d.iri.clone(), d.short_name.clone()))
        .collect();
    // Parallel lookup for InductiveType IRIs → their Julia abstract
    // names (e.g. `formulas:FormulaTerm` → `"FormulaTerm"`). Used by
    // `JuliaType::InductiveRef::render` for `core:inductive`-typed
    // class fields.
    let inductive_lookup: BTreeMap<Iri, String> = inductive_decls
        .values()
        .map(|d| (d.iri.clone(), d.short_name.clone()))
        .collect();

    let mut s = String::new();
    s.push_str("# Auto-generated by eigon-julia-gen — DO NOT EDIT.\n");
    s.push_str("# Regenerate via the substrate's image-build pipeline.\n");
    s.push_str(&format!("# source_layer: {}\n", request.source_layer));
    s.push_str("# mirrored_classes:\n");
    for iri in struct_order {
        s.push_str(&format!("#   - {iri}\n"));
    }
    s.push('\n');
    s.push_str(&format!("module {TARGET_MODULE_NAME}\n\n"));

    s.push_str("using EigeniusJuliaCommon: validate_min_value, validate_max_value, ");
    s.push_str("validate_min_length, validate_max_length, validate_pattern, validate_format\n");
    // `CBOR` is needed by the inductive-decoder tag-peeling overload
    // (D32 §3.7) — `Value::Json` payloads land wrapped in
    // `CBOR.Tag(27182, ...)` per `eigon_cbor::EIGENIUS_JSON_TAG`, and
    // the inductive `decode_<T>(t::CBOR.Tag)` method peels them.
    s.push_str("using CBOR\n\n");

    // Phase 1: emit abstract type declarations in subclass_of-topo
    // order. Every class C produces `abstract type AbstractC end`
    // (with `<: AbstractParent` when C declares subclass_of). The
    // hierarchy is closed before any concrete struct is emitted, so
    // struct field types referencing AbstractX are always in scope.
    for iri in abstract_order {
        let decl = decls.get(iri).expect("abstract order references decls");
        let parent_clause = match &decl.subclass_of {
            Some(parent) => format!(" <: {}", class_abstract_name(parent, &class_lookup)),
            None => String::new(),
        };
        s.push_str(&format!(
            "abstract type {}{parent_clause} end\n",
            class_abstract_name(&decl.iri, &class_lookup)
        ));
    }
    if !abstract_order.is_empty() {
        s.push('\n');
    }

    // Phase 1b (D32 §3.6): emit inductive type abstracts + concrete
    // per-ctor structs + decode/encode functions. Inductives come
    // *before* class structs so a class field whose `data_type` is
    // `core:inductive` can reference the inductive's abstract type.
    // Order is BTreeMap iteration (IRI sort) — stable but doesn't
    // pre-resolve cross-inductive references; mutually recursive
    // inductives use the abstract type which Julia accepts as a
    // forward reference within a module.
    if !inductive_decls.is_empty() {
        // Two passes so all abstract-type forward declarations are
        // visible before any concrete struct references them.
        for ind in inductive_decls.values() {
            s.push_str(&format!("abstract type {} end\n", ind.short_name));
        }
        s.push('\n');
        for ind in inductive_decls.values() {
            emit_inductive(&mut s, ind);
        }
    }

    // Phase 2: emit concrete struct + helpers + codecs per class in
    // field-dependency topo order.
    for iri in struct_order {
        let decl = decls.get(iri).expect("struct order references decls");
        let layout = layouts.get(iri).expect("layout for every class");
        emit_struct(&mut s, decl, layout, &class_lookup, &inductive_lookup);
        s.push('\n');
        // D29 §8.3: emit per-field codec helpers for any property
        // whose type has more than one concrete leaf in the closure.
        let any_helper_needed = layout
            .requires
            .iter()
            .chain(layout.recommends.iter())
            .any(|p| {
                let t = property_codec_type(p);
                type_leaves(t, concrete_descendants).len() > 1
            });
        if any_helper_needed {
            emit_union_helpers(&mut s, decl, layout, concrete_descendants, &class_lookup);
            s.push('\n');
        }
        emit_decoder(
            &mut s,
            decl,
            layout,
            concrete_descendants,
            &class_lookup,
            &inductive_lookup,
        );
        s.push('\n');
        emit_encoder(
            &mut s,
            decl,
            layout,
            concrete_descendants,
            &class_lookup,
            &inductive_lookup,
        );
        s.push('\n');
    }

    // Codec registries (D29 §8.5 — added in v1.1 to support
    // worker-side typed-method dispatch). Two constants exported by
    // every mirror module:
    //
    // - `_eigenius_decoders` — class IRI → `decode_<C>` function. The
    //   worker reads `m["urn:eigenius:core:is_a"]` on each input and
    //   dispatches to the matching decoder.
    // - `_eigenius_encoders` — concrete struct type → `encode_<C>`
    //   function. The worker dispatches on `typeof(result)` to
    //   produce the output dict.
    let any_emission = !struct_order.is_empty() || !inductive_decls.is_empty();
    if any_emission {
        s.push_str("const _eigenius_decoders = Dict{String, Function}(\n");
        for iri in struct_order {
            if let Some(d) = decls.get(iri) {
                s.push_str(&format!(
                    "    {} => decode_{},\n",
                    julia_string_literal(d.iri.as_str()),
                    d.short_name
                ));
            }
        }
        // Inductive types: keyed on the InductiveType IRI, decoder
        // dispatches on the value tree's `ctor` field.
        for ind in inductive_decls.values() {
            s.push_str(&format!(
                "    {} => decode_{},\n",
                julia_string_literal(ind.iri.as_str()),
                ind.short_name
            ));
        }
        s.push_str(")\n\n");

        s.push_str("const _eigenius_encoders = Dict{DataType, Function}(\n");
        for iri in struct_order {
            if let Some(d) = decls.get(iri) {
                s.push_str(&format!(
                    "    {} => encode_{},\n",
                    d.short_name, d.short_name
                ));
            }
        }
        // Inductives: every concrete per-ctor struct dispatches into
        // its inductive's encode_* function via Julia's typeof().
        for ind in inductive_decls.values() {
            for ctor in &ind.ctors {
                s.push_str(&format!(
                    "    {} => encode_{},\n",
                    inductive_ctor_struct_name(&ind.short_name, &ctor.ctor_name),
                    ind.short_name
                ));
            }
        }
        s.push_str(")\n\n");
    }

    if any_emission {
        s.push_str("export ");
        let mut exports: Vec<String> = Vec::new();
        for iri in struct_order {
            if let Some(d) = decls.get(iri) {
                // Both the abstract and the concrete struct are
                // exported so user code can dispatch on either.
                exports.push(class_abstract_name(&d.iri, &class_lookup));
                exports.push(d.short_name.clone());
                exports.push(format!("decode_{}", d.short_name));
                exports.push(format!("encode_{}", d.short_name));
            }
        }
        for ind in inductive_decls.values() {
            // Inductive abstract + every concrete ctor + the
            // encode/decode pair.
            exports.push(ind.short_name.clone());
            for ctor in &ind.ctors {
                exports.push(inductive_ctor_struct_name(&ind.short_name, &ctor.ctor_name));
            }
            exports.push(format!("decode_{}", ind.short_name));
            exports.push(format!("encode_{}", ind.short_name));
        }
        // Codec registries are part of the worker dispatch contract;
        // export them so the worker's introspection finds them.
        exports.push("_eigenius_decoders".to_string());
        exports.push("_eigenius_encoders".to_string());
        s.push_str(&exports.join(", "));
        s.push('\n');
        s.push('\n');
    }

    s.push_str(&format!("end # module {TARGET_MODULE_NAME}\n"));
    s
}

/// Return the inner type used for codec leaf-counting on a property.
/// `Vector{<inner>}` reduces to `<inner>` for the helper-or-direct
/// decision (the array comprehension is independent of element-codec
/// shape).
fn property_codec_type(p: &PropertyDecl) -> &JuliaType {
    p.julia_type.vector_inner().unwrap_or(&p.julia_type)
}

/// Per-ctor concrete struct name. Convention: `<InductiveName>_<CtorName>`.
/// Concatenation rather than camel-casing keeps the chain `ctor_name`
/// recoverable from the Julia struct name without a normalisation table.
fn inductive_ctor_struct_name(inductive_short: &str, ctor_name: &str) -> String {
    format!("{inductive_short}_{ctor_name}")
}

/// Map a chain `type_name` string to the Julia type it materialises
/// into. Primitive type IRIs get the standard primitive types; an
/// InductiveType IRI gets the abstract type (the concrete sub-type
/// satisfies the field). Anything else (Class IRIs, parameter names,
/// unresolved IRIs) falls back to `Any` — Class-typed args inside an
/// inductive ctor aren't re-typechecked at the inductive layer
/// (they're typechecked by the chain validator on the surrounding
/// resource); parameter-name handling lands when the first parametric
/// inductive consumer materialises.
fn inductive_arg_julia_type(
    type_name: &str,
    inductive_decls: &BTreeMap<Iri, InductiveDecl>,
) -> String {
    match type_name {
        TYPE_STRING => "String".to_string(),
        TYPE_INTEGER => "Int64".to_string(),
        TYPE_FLOAT => "Float64".to_string(),
        TYPE_BOOLEAN => "Bool".to_string(),
        other => {
            if let Ok(iri) = Iri::parse(other) {
                if let Some(ind) = inductive_decls.get(&iri) {
                    return ind.short_name.clone();
                }
            }
            "Any".to_string()
        }
    }
}

/// Emit the Julia for one [`InductiveDecl`] — concrete per-ctor
/// structs (the abstract type was emitted in the forward-declaration
/// pass) plus `decode_<T>(d::Dict)::<T>` and `encode_<T>(v::<T>)::Dict`.
/// Both functions delegate per-ctor to recursive calls into other
/// inductives' decoders/encoders, looked up via the closure's
/// declaration map.
///
/// All callers operate on `inductive_decls` already on hand at
/// emission time so type lookups for arg types are local.
fn emit_inductive(out: &mut String, ind: &InductiveDecl) {
    // Empty inductive (no ctors) is a degenerate case but valid as a
    // declaration of an unconstructable type. Emit decoder/encoder
    // stubs so the registry still resolves.
    let inductive_decls_local = BTreeMap::from_iter([(
        ind.iri.clone(),
        InductiveDecl {
            iri: ind.iri.clone(),
            short_name: ind.short_name.clone(),
            ctors: Vec::new(),
        },
    )]);

    // Concrete structs per ctor.
    for ctor in &ind.ctors {
        let struct_name = inductive_ctor_struct_name(&ind.short_name, &ctor.ctor_name);
        out.push_str(&format!(
            "struct {struct_name} <: {parent}\n",
            parent = ind.short_name,
        ));
        for (i, arg) in ctor.args.iter().enumerate() {
            let field_name = arg.arg_name.clone().unwrap_or_else(|| format!("arg_{i}"));
            let jty = inductive_arg_julia_type(&arg.type_name, &inductive_decls_local);
            out.push_str(&format!("    {field_name}::{jty}\n"));
        }
        out.push_str("end\n\n");
    }

    // Tag-peeling overload: Eigon-CBOR wraps `Value::Json` payloads
    // (which is how chain-side inductive values land on the wire) in
    // `CBOR.Tag(27182, ...)`. The class-decoder path doesn't see this
    // because chain Resources serialize as plain CBOR maps; inductive
    // values do because the codec uses the tag to distinguish opaque
    // JSON from typed Resources. Peel the wrapper here so callers
    // can hand us either shape — see `EIGENIUS_JSON_TAG` in
    // `kernel/src/ontology/eigon_cbor.rs`.
    out.push_str(&format!(
        "decode_{name}(t::CBOR.Tag)::{name} = decode_{name}(t.data)\n\n",
        name = ind.short_name,
    ));

    // Decoder: dispatches on `d["ctor"]`, recurses into args via the
    // worker-side `_eigenius_decoders` registry (keyed by IRI) for
    // any nested inductive arg types, primitive accessors for
    // primitives.
    out.push_str(&format!(
        "function decode_{name}(d::AbstractDict)::{name}\n",
        name = ind.short_name,
    ));
    out.push_str("    ctor = d[\"ctor\"]\n");
    out.push_str("    args = get(d, \"args\", Any[])\n");
    let mut first = true;
    for ctor in &ind.ctors {
        let struct_name = inductive_ctor_struct_name(&ind.short_name, &ctor.ctor_name);
        let kw = if first { "if" } else { "elseif" };
        first = false;
        out.push_str(&format!(
            "    {kw} ctor == {ctor_lit}\n",
            ctor_lit = julia_string_literal(&ctor.ctor_name),
        ));
        // Build the constructor-call arg expressions.
        let arg_exprs: Vec<String> = ctor
            .args
            .iter()
            .enumerate()
            .map(|(i, arg)| inductive_decode_arg_expr(&arg.type_name, i, &ind.short_name))
            .collect();
        out.push_str(&format!(
            "        return {struct_name}({})\n",
            arg_exprs.join(", "),
        ));
    }
    if !first {
        out.push_str("    else\n");
        out.push_str(&format!(
            "        error(\"unknown ctor `$ctor` for inductive {}\")\n",
            ind.short_name,
        ));
        out.push_str("    end\n");
    } else {
        out.push_str(&format!(
            "    error(\"inductive {} has no declared ctors\")\n",
            ind.short_name,
        ));
    }
    out.push_str("end\n\n");

    // Encoder: dispatches on `typeof(v)` against each concrete ctor
    // struct. Recurses into nested inductive args via the same
    // `encode_<T>` family.
    out.push_str(&format!(
        "function encode_{name}(v::{name})::Dict{{String, Any}}\n",
        name = ind.short_name,
    ));
    let mut first = true;
    for ctor in &ind.ctors {
        let struct_name = inductive_ctor_struct_name(&ind.short_name, &ctor.ctor_name);
        let kw = if first { "if" } else { "elseif" };
        first = false;
        out.push_str(&format!("    {kw} v isa {struct_name}\n"));
        let arg_exprs: Vec<String> = ctor
            .args
            .iter()
            .enumerate()
            .map(|(i, arg)| {
                let field_name = arg.arg_name.clone().unwrap_or_else(|| format!("arg_{i}"));
                inductive_encode_arg_expr(&arg.type_name, &field_name)
            })
            .collect();
        out.push_str(&format!(
            "        return Dict{{String, Any}}(\"ctor\" => {ctor_lit}, \"args\" => Any[{args}])\n",
            ctor_lit = julia_string_literal(&ctor.ctor_name),
            args = arg_exprs.join(", "),
        ));
    }
    if !first {
        out.push_str("    else\n");
        out.push_str(&format!(
            "        error(\"unknown concrete type for inductive {}: $(typeof(v))\")\n",
            ind.short_name,
        ));
        out.push_str("    end\n");
    } else {
        out.push_str(&format!(
            "    error(\"inductive {} has no encoders (no ctors declared)\")\n",
            ind.short_name,
        ));
    }
    out.push_str("end\n\n");
}

/// Build the Julia expression that decodes one ctor argument from
/// `args[i+1]` (Julia is 1-indexed). Primitive types get the value
/// passed through; inductives get a recursive `decode_<T>(args[i+1])`
/// call. Class-typed args (Any) pass the value through unchanged.
fn inductive_decode_arg_expr(type_name: &str, idx: usize, parent_inductive: &str) -> String {
    let one_indexed = idx + 1;
    if let Ok(iri) = Iri::parse(type_name) {
        // Self-reference?
        if iri.as_str() == format!("urn:eigenius:formulas:{parent_inductive}")
            || type_name.ends_with(&format!(":{parent_inductive}"))
        {
            return format!("decode_{parent_inductive}(args[{one_indexed}])");
        }
        // Primitive types: structural pass-through.
        match type_name {
            TYPE_STRING => return format!("convert(String, args[{one_indexed}])"),
            TYPE_INTEGER => return format!("convert(Int64, args[{one_indexed}])"),
            TYPE_FLOAT => return format!("convert(Float64, args[{one_indexed}])"),
            TYPE_BOOLEAN => return format!("convert(Bool, args[{one_indexed}])"),
            _ => {}
        }
        // Other IRI — assume it's an inductive sibling. The worker's
        // `_eigenius_decoders` registry resolves it at call time;
        // cheaper for v1 to defer the lookup to the registry rather
        // than thread a global lookup through the emit path.
        format!(
            "Base.invokelatest(_eigenius_decoders[{}], args[{one_indexed}])",
            julia_string_literal(type_name)
        )
    } else {
        // Bare parameter name (or unresolvable): pass through.
        format!("args[{one_indexed}]")
    }
}

/// Inverse of [`inductive_decode_arg_expr`] — emit the Julia that
/// re-encodes a struct field for the output dict.
fn inductive_encode_arg_expr(type_name: &str, field_name: &str) -> String {
    if let Ok(_iri) = Iri::parse(type_name) {
        match type_name {
            TYPE_STRING | TYPE_INTEGER | TYPE_FLOAT | TYPE_BOOLEAN => {
                return format!("v.{field_name}");
            }
            _ => {}
        }
        // Recursive case: encode via the registry. A self-encoder call
        // would also work but registry-routed keeps the symmetry with
        // the decode path.
        format!("Base.invokelatest(_eigenius_encoders[typeof(v.{field_name})], v.{field_name})")
    } else {
        format!("v.{field_name}")
    }
}

fn emit_struct(
    out: &mut String,
    decl: &ClassDecl,
    layout: &ClassLayout,
    class_lookup: &BTreeMap<Iri, String>,
    inductive_lookup: &BTreeMap<Iri, String>,
) {
    let parent_clause = format!(" <: {}", class_abstract_name(&decl.iri, class_lookup));
    out.push_str(&format!("struct {}{}\n", decl.short_name, parent_clause));
    for prop in &layout.requires {
        out.push_str(&format!(
            "    {}::{}\n",
            prop.short_name,
            prop.julia_type.render(class_lookup, inductive_lookup)
        ));
    }
    for prop in &layout.recommends {
        out.push_str(&format!(
            "    {}::Union{{{}, Nothing}}\n",
            prop.short_name,
            prop.julia_type.render(class_lookup, inductive_lookup)
        ));
    }
    // D29 §8.4: `_id` is the mirror-managed @id round-trip slot.
    // Emitted on every struct, last in declaration order so the
    // user-facing required+recommended fields stay grouped at the top.
    out.push_str(&format!(
        "    {RESERVED_FIELD_ID}::Union{{String, Nothing}}\n"
    ));
    out.push('\n');
    emit_inner_constructor(out, decl, layout, class_lookup, inductive_lookup);
    out.push_str("end\n");
}

/// Inner constructor with format-constraint validation. Required
/// fields are positional; recommended fields and `_id` are keyword
/// args defaulting to `nothing`. Each field's constraints (if any)
/// are checked before `new(...)`. Per D29 §8.4 `_id` is always the
/// last keyword arg — it's mirror-managed metadata and stays out of
/// the way of the user-facing recommended properties.
fn emit_inner_constructor(
    out: &mut String,
    decl: &ClassDecl,
    layout: &ClassLayout,
    class_lookup: &BTreeMap<Iri, String>,
    inductive_lookup: &BTreeMap<Iri, String>,
) {
    out.push_str(&format!("    function {}(\n", decl.short_name));

    let last_required = layout.requires.len().saturating_sub(1);
    // `_id` is always a keyword arg, so the constructor *always* has
    // at least one keyword section — the `;` separator after the last
    // required arg is unconditional.
    let has_keyword = true;

    // Positional args: required fields. The last one ends with `;`
    // because the keyword section (carrying at minimum `_id`) always
    // follows.
    for (i, prop) in layout.requires.iter().enumerate() {
        let trailer = if i == last_required && has_keyword {
            ";"
        } else {
            ","
        };
        out.push_str(&format!(
            "        {}::{}{trailer}\n",
            prop.short_name,
            prop.julia_type.render(class_lookup, inductive_lookup)
        ));
    }
    // Edge case: zero required fields. Julia needs `;` to open the
    // keyword section even with no positional args.
    if layout.requires.is_empty() {
        out.push_str("        ;\n");
    }
    for prop in &layout.recommends {
        out.push_str(&format!(
            "        {}::Union{{{}, Nothing}} = nothing,\n",
            prop.short_name,
            prop.julia_type.render(class_lookup, inductive_lookup)
        ));
    }
    // `_id` last among kwargs (D29 §8.4).
    out.push_str(&format!(
        "        {RESERVED_FIELD_ID}::Union{{String, Nothing}} = nothing,\n"
    ));
    out.push_str("    )\n");

    // Validation calls. Required props always; recommended props
    // gated on `isnothing(field) || …` so a missing recommended
    // field passes through without firing the validator. `_id` has
    // no validators in v1 (the kernel handles IRI well-formedness
    // upstream when one is present at all).
    for prop in &layout.requires {
        emit_validations(out, prop, /* is_required = */ true);
    }
    for prop in &layout.recommends {
        emit_validations(out, prop, /* is_required = */ false);
    }

    // Construct. Field order: required, recommended, _id (matches
    // struct declaration order).
    out.push_str("        new(");
    let mut all: Vec<&str> = layout
        .requires
        .iter()
        .chain(layout.recommends.iter())
        .map(|p| p.short_name.as_str())
        .collect();
    all.push(RESERVED_FIELD_ID);
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
        lines.push(format!(
            "validate_format(:{field}, {field}, {})",
            fmt.as_julia_symbol_expr()
        ));
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
    layout: &ClassLayout,
    concrete_descendants: &BTreeMap<Iri, BTreeSet<Iri>>,
    class_lookup: &BTreeMap<Iri, String>,
    inductive_lookup: &BTreeMap<Iri, String>,
) {
    let cls = &decl.short_name;
    out.push_str(&format!("function decode_{cls}(m::AbstractDict)::{cls}\n"));
    out.push_str(&format!("    {cls}(\n"));

    let last_required = layout.requires.len().saturating_sub(1);
    // `_id` is always a kwarg, so kwarg section is always present.
    let has_keyword = true;

    // Required positional args.
    for (i, prop) in layout.requires.iter().enumerate() {
        let trailer = if i == last_required && has_keyword {
            ";"
        } else {
            ","
        };
        out.push_str(&format!(
            "        {}{trailer}\n",
            decode_property_expr(
                prop,
                cls,
                class_lookup,
                inductive_lookup,
                concrete_descendants,
                /* required = */ true
            )
        ));
    }
    // Keyword args. Empty required → leading `;` opens the kwarg
    // section before `_id` and any recommended.
    if layout.requires.is_empty() {
        out.push_str("        ;\n");
    }
    for prop in &layout.recommends {
        out.push_str(&format!(
            "        {} = {},\n",
            prop.short_name,
            decode_property_expr(
                prop,
                cls,
                class_lookup,
                inductive_lookup,
                concrete_descendants,
                /* required = */ false
            )
        ));
    }
    // `_id` last (D29 §8.4): read m["@id"] when present, nothing
    // otherwise. No type recursion — IRIs are bare strings.
    out.push_str(&format!(
        "        {RESERVED_FIELD_ID} = (let _v = get(m, {}, nothing); isnothing(_v) ? nothing : _v end),\n",
        julia_string_literal(KEY_AT_ID)
    ));
    out.push_str("    )\n");
    out.push_str("end\n");
}

fn decode_property_expr(
    prop: &PropertyDecl,
    class_short: &str,
    class_lookup: &BTreeMap<Iri, String>,
    inductive_lookup: &BTreeMap<Iri, String>,
    concrete_descendants: &BTreeMap<Iri, BTreeSet<Iri>>,
    required: bool,
) -> String {
    let key = julia_string_literal(prop.iri.as_str());
    if required {
        decode_value_expr(
            &prop.julia_type,
            &format!("m[{key}]"),
            class_short,
            &prop.short_name,
            class_lookup,
            inductive_lookup,
            concrete_descendants,
        )
    } else {
        // get(m, key, nothing); if nothing, pass through; else recurse.
        let raw = format!("get(m, {key}, nothing)");
        let inner = decode_value_expr(
            &prop.julia_type,
            "_v",
            class_short,
            &prop.short_name,
            class_lookup,
            inductive_lookup,
            concrete_descendants,
        );
        format!("(let _v = {raw}; isnothing(_v) ? nothing : ({inner}) end)")
    }
}

/// Express the worker-side decode of `expr` (a CBOR-loaded value)
/// into the Julia type `t`. Decision tree (D29 §8.3):
/// - Primitive → pass `expr` through verbatim.
/// - Single concrete leaf → direct `decode_<C>(expr)` call.
/// - Multiple concrete leaves → call the per-field
///   `_decode_<class>_<field>` helper emitted by [`emit_union_helpers`].
/// - Vector wrapper → array comprehension over the inner type.
fn decode_value_expr(
    t: &JuliaType,
    expr: &str,
    class_short: &str,
    field_short: &str,
    class_lookup: &BTreeMap<Iri, String>,
    inductive_lookup: &BTreeMap<Iri, String>,
    concrete_descendants: &BTreeMap<Iri, BTreeSet<Iri>>,
) -> String {
    if let JuliaType::Primitive(name) = t {
        // Coerce to the declared Julia type. CBOR's compact-float
        // optimisation ships a Float64 source value as Float16/Float32
        // when it fits losslessly; the mirror struct's typed `::Float64`
        // field would reject that without coercion. Same for Int sizes
        // and for substring/symbol-like inputs that decode as
        // non-`String` text. Coercion is lossless for matching CBOR
        // major types and surfaces a clear `MethodError` if the wire
        // shape doesn't match the declared primitive.
        return primitive_coerce_expr(name, expr);
    }
    if let JuliaType::InductiveRef(iri) = t {
        // D32 §3.6: inductive-typed field decodes via the inductive's
        // emitted `decode_<T>` (which dispatches on the value tree's
        // `ctor`). The decoder is in the same module, so a direct
        // call resolves at compile time.
        let short = inductive_lookup
            .get(iri)
            .cloned()
            .unwrap_or_else(|| sanitise_for_identifier(iri.as_str()));
        return format!("decode_{short}({expr})");
    }
    if let JuliaType::Vector(inner) = t {
        if let JuliaType::Primitive(name) = inner.as_ref() {
            return format!("[{} for _x in {expr}]", primitive_coerce_expr(name, "_x"));
        }
        if matches!(inner.as_ref(), JuliaType::Vector(_)) {
            return format!("# TODO: nested Vector decode unsupported in v1\n        {expr}");
        }
        if let JuliaType::InductiveRef(iri) = inner.as_ref() {
            // Inductive `decode_<T>` is typed `::T` where `T` is the
            // abstract umbrella, so `[decode_T(_x) for _x in expr]`
            // already lands as `Vector{T}` — matches the field type.
            let short = inductive_lookup
                .get(iri)
                .cloned()
                .unwrap_or_else(|| sanitise_for_identifier(iri.as_str()));
            return format!("[decode_{short}(_x) for _x in {expr}]");
        }
        // For class-typed Vectors the field is rendered as
        // `Vector{Abstract<C>}`, but `decode_<C>(_x)` returns the
        // *concrete* struct type. Julia's parametric types are
        // invariant, so an unannotated comprehension produces
        // `Vector{C}` and the constructor rejects it. Type-annotate
        // the comprehension with the abstract element type so the
        // resulting array's element type lines up with the field.
        let abstract_element = inner.render(class_lookup, inductive_lookup);
        let leaves = type_leaves(inner.as_ref(), concrete_descendants);
        if leaves.len() == 1 {
            let only = leaves.iter().next().expect("len 1");
            let cls = class_short_name(only, class_lookup);
            return format!("{abstract_element}[decode_{cls}(_x) for _x in {expr}]");
        }
        return format!(
            "{abstract_element}[_decode_{class_short}_{field_short}(_x) for _x in {expr}]"
        );
    }
    // Scalar struct ref / union ref.
    let leaves = type_leaves(t, concrete_descendants);
    if leaves.len() == 1 {
        let only = leaves.iter().next().expect("len 1");
        let cls = class_short_name(only, class_lookup);
        return format!("decode_{cls}({expr})");
    }
    format!("_decode_{class_short}_{field_short}({expr})")
}

/// Wrap `expr` in a primitive-type constructor so the decoder
/// accepts any CBOR-compatible width and converts to the struct's
/// declared type. `Any` passes through unchanged because Julia has
/// no `Any(x)` constructor — the field accepts whatever the wire
/// shape produced (intentional for `data_type: json` properties).
fn primitive_coerce_expr(name: &str, expr: &str) -> String {
    match name {
        "Any" => expr.to_string(),
        other => format!("{other}({expr})"),
    }
}

fn emit_encoder(
    out: &mut String,
    decl: &ClassDecl,
    layout: &ClassLayout,
    concrete_descendants: &BTreeMap<Iri, BTreeSet<Iri>>,
    class_lookup: &BTreeMap<Iri, String>,
    inductive_lookup: &BTreeMap<Iri, String>,
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
    for prop in &layout.requires {
        let key = julia_string_literal(prop.iri.as_str());
        let value = encode_value_expr(
            &prop.julia_type,
            &format!("c.{}", prop.short_name),
            cls,
            &prop.short_name,
            class_lookup,
            inductive_lookup,
            concrete_descendants,
        );
        out.push_str(&format!("        {key} => {value},\n"));
    }
    out.push_str("    )\n");
    for prop in &layout.recommends {
        let key = julia_string_literal(prop.iri.as_str());
        let field = &prop.short_name;
        let value = encode_value_expr(
            &prop.julia_type,
            &format!("c.{field}"),
            cls,
            field,
            class_lookup,
            inductive_lookup,
            concrete_descendants,
        );
        out.push_str(&format!(
            "    isnothing(c.{field}) || (out[{key}] = {value})\n"
        ));
    }
    // `_id` last: stamp the @id key when the struct carries one
    // (D29 §8.4).
    out.push_str(&format!(
        "    isnothing(c.{RESERVED_FIELD_ID}) || (out[{}] = c.{RESERVED_FIELD_ID})\n",
        julia_string_literal(KEY_AT_ID)
    ));
    out.push_str("    return out\n");
    out.push_str("end\n");
}

fn encode_value_expr(
    t: &JuliaType,
    expr: &str,
    class_short: &str,
    field_short: &str,
    class_lookup: &BTreeMap<Iri, String>,
    inductive_lookup: &BTreeMap<Iri, String>,
    concrete_descendants: &BTreeMap<Iri, BTreeSet<Iri>>,
) -> String {
    if let JuliaType::Primitive(_) = t {
        return expr.to_string();
    }
    if let JuliaType::InductiveRef(iri) = t {
        let short = inductive_lookup
            .get(iri)
            .cloned()
            .unwrap_or_else(|| sanitise_for_identifier(iri.as_str()));
        // Wrap inductive payloads in `CBOR.Tag(EIGENIUS_JSON_TAG, …)`
        // so the kernel's CBOR decoder treats them as `Value::Json`
        // (opaque) rather than trying to parse the bare-keyed
        // `{ctor, args, …}` shape as an embedded Resource. The worker
        // generates `decode_<T>(t::CBOR.Tag)` overloads to peel the
        // wrapper on incoming traffic, so the round-trip stays
        // symmetric. See `kernel/src/ontology/eigon_cbor.rs:330`
        // (`EIGENIUS_JSON_TAG`).
        return format!("CBOR.Tag(27182, encode_{short}({expr}))");
    }
    if let JuliaType::Vector(inner) = t {
        if matches!(inner.as_ref(), JuliaType::Primitive(_)) {
            return expr.to_string();
        }
        if matches!(inner.as_ref(), JuliaType::Vector(_)) {
            return format!("# TODO: nested Vector encode unsupported in v1\n        {expr}");
        }
        if let JuliaType::InductiveRef(iri) = inner.as_ref() {
            let short = inductive_lookup
                .get(iri)
                .cloned()
                .unwrap_or_else(|| sanitise_for_identifier(iri.as_str()));
            // Same tag-wrapping rationale as the scalar branch above.
            return format!("[CBOR.Tag(27182, encode_{short}(_x)) for _x in {expr}]");
        }
        let leaves = type_leaves(inner.as_ref(), concrete_descendants);
        if leaves.len() == 1 {
            let only = leaves.iter().next().expect("len 1");
            let cls = class_short_name(only, class_lookup);
            return format!("[encode_{cls}(_x) for _x in {expr}]");
        }
        return format!("[_encode_{class_short}_{field_short}(_x) for _x in {expr}]");
    }
    let leaves = type_leaves(t, concrete_descendants);
    if leaves.len() == 1 {
        let only = leaves.iter().next().expect("len 1");
        let cls = class_short_name(only, class_lookup);
        return format!("encode_{cls}({expr})");
    }
    format!("_encode_{class_short}_{field_short}({expr})")
}

/// Emit `_encode_<C>_<field>` and `_decode_<C>_<field>` helpers for
/// every property on `decl` whose effective concrete-leaf set has
/// more than one element — i.e. either a `Union` of struct refs OR a
/// single `class_types: [C]` where C has descendants in the closure.
/// The helpers dispatch by `typeof` (encode) and by the input dict's
/// `is_a` list (decode), per D29 §8.3.
fn emit_union_helpers(
    out: &mut String,
    decl: &ClassDecl,
    layout: &ClassLayout,
    concrete_descendants: &BTreeMap<Iri, BTreeSet<Iri>>,
    class_lookup: &BTreeMap<Iri, String>,
) {
    for prop in layout.requires.iter().chain(layout.recommends.iter()) {
        let codec_type = property_codec_type(prop);
        let leaves: Vec<Iri> = type_leaves(codec_type, concrete_descendants)
            .into_iter()
            .collect();
        if leaves.len() <= 1 {
            continue;
        }
        emit_one_union_helper_pair(
            out,
            &decl.short_name,
            &prop.short_name,
            &leaves,
            class_lookup,
        );
    }
}

fn emit_one_union_helper_pair(
    out: &mut String,
    class_short: &str,
    field_short: &str,
    iris: &[Iri],
    class_lookup: &BTreeMap<Iri, String>,
) {
    // Encoder: dispatch by typeof.
    out.push_str(&format!(
        "function _encode_{class_short}_{field_short}(v)\n"
    ));
    for (i, iri) in iris.iter().enumerate() {
        let cls = class_short_name(iri, class_lookup);
        let kw = if i == 0 { "if" } else { "elseif" };
        out.push_str(&format!("    {kw} v isa {cls}\n"));
        out.push_str(&format!("        return encode_{cls}(v)\n"));
    }
    out.push_str("    else\n");
    out.push_str(&format!(
        "        throw(ArgumentError(\"unexpected type $(typeof(v)) for field {field_short} of {class_short}\"))\n"
    ));
    out.push_str("    end\n");
    out.push_str("end\n\n");

    // Decoder: dispatch by is_a list.
    out.push_str(&format!(
        "function _decode_{class_short}_{field_short}(m::AbstractDict)\n"
    ));
    out.push_str(&format!(
        "    is_a = get(m, {}, String[])\n",
        julia_string_literal(PROP_IS_A)
    ));
    for (i, iri) in iris.iter().enumerate() {
        let cls = class_short_name(iri, class_lookup);
        let kw = if i == 0 { "if" } else { "elseif" };
        out.push_str(&format!(
            "    {kw} {} in is_a\n",
            julia_string_literal(iri.as_str())
        ));
        out.push_str(&format!("        return decode_{cls}(m)\n"));
    }
    out.push_str("    else\n");
    out.push_str(&format!(
        "        throw(ArgumentError(\"no matching decoder for is_a $(is_a) on field {field_short} of {class_short}\"))\n"
    ));
    out.push_str("    end\n");
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
    fn struct_for_assay_result_uses_abstract_class_refs() {
        // D29 §4 / §7: a property's `class_types: [C]` renders the
        // field type as `AbstractC` (not `C`) so any concrete subtype
        // of C is assignable. Even leaves with no subclasses go
        // through the abstract slot — uniform shape.
        let out = run_kinase(&["urn:eigenius:demo:assay:AssayResult"]);
        let src = extract_source(&out);
        assert!(
            src.contains("compound::AbstractCompound"),
            "expected `compound::AbstractCompound`, got source:\n{src}"
        );
        assert!(src.contains("target::AbstractTarget"));
        assert!(src.contains("protocol::AbstractAssayProtocol"));
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
using CBOR

abstract type AbstractAssayProtocol end
abstract type AbstractAssayResult end
abstract type AbstractCompound end
abstract type AbstractTarget end

struct AssayProtocol <: AbstractAssayProtocol
    protocol_name::String
    incubation_minutes::Int64
    _id::Union{String, Nothing}

    function AssayProtocol(
        protocol_name::String,
        incubation_minutes::Int64;
        _id::Union{String, Nothing} = nothing,
    )
        validate_min_value(:incubation_minutes, incubation_minutes, 0.0)
        new(protocol_name, incubation_minutes, _id)
    end
end

function decode_AssayProtocol(m::AbstractDict)::AssayProtocol
    AssayProtocol(
        String(m[\"urn:eigenius:demo:assay:protocol_name\"]),
        Int64(m[\"urn:eigenius:demo:assay:incubation_minutes\"]);
        _id = (let _v = get(m, \"@id\", nothing); isnothing(_v) ? nothing : _v end),
    )
end

function encode_AssayProtocol(c::AssayProtocol)::Dict{String, Any}
    out = Dict{String, Any}(
        \"urn:eigenius:core:is_a\" => [\"urn:eigenius:demo:assay:AssayProtocol\"],
        \"urn:eigenius:demo:assay:protocol_name\" => c.protocol_name,
        \"urn:eigenius:demo:assay:incubation_minutes\" => c.incubation_minutes,
    )
    isnothing(c._id) || (out[\"@id\"] = c._id)
    return out
end

struct Compound <: AbstractCompound
    compound_id::String
    scaffold_class::String
    molecular_weight::Float64
    logp::Union{Float64, Nothing}
    _id::Union{String, Nothing}

    function Compound(
        compound_id::String,
        scaffold_class::String,
        molecular_weight::Float64;
        logp::Union{Float64, Nothing} = nothing,
        _id::Union{String, Nothing} = nothing,
    )
        validate_min_value(:molecular_weight, molecular_weight, 0.0)
        new(compound_id, scaffold_class, molecular_weight, logp, _id)
    end
end

function decode_Compound(m::AbstractDict)::Compound
    Compound(
        String(m[\"urn:eigenius:demo:assay:compound_id\"]),
        String(m[\"urn:eigenius:demo:assay:scaffold_class\"]),
        Float64(m[\"urn:eigenius:demo:assay:molecular_weight\"]);
        logp = (let _v = get(m, \"urn:eigenius:demo:assay:logp\", nothing); isnothing(_v) ? nothing : (Float64(_v)) end),
        _id = (let _v = get(m, \"@id\", nothing); isnothing(_v) ? nothing : _v end),
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
    isnothing(c._id) || (out[\"@id\"] = c._id)
    return out
end

struct Target <: AbstractTarget
    target_name::String
    target_family::String
    _id::Union{String, Nothing}

    function Target(
        target_name::String,
        target_family::String;
        _id::Union{String, Nothing} = nothing,
    )
        new(target_name, target_family, _id)
    end
end

function decode_Target(m::AbstractDict)::Target
    Target(
        String(m[\"urn:eigenius:demo:assay:target_name\"]),
        String(m[\"urn:eigenius:demo:assay:target_family\"]);
        _id = (let _v = get(m, \"@id\", nothing); isnothing(_v) ? nothing : _v end),
    )
end

function encode_Target(c::Target)::Dict{String, Any}
    out = Dict{String, Any}(
        \"urn:eigenius:core:is_a\" => [\"urn:eigenius:demo:assay:Target\"],
        \"urn:eigenius:demo:assay:target_name\" => c.target_name,
        \"urn:eigenius:demo:assay:target_family\" => c.target_family,
    )
    isnothing(c._id) || (out[\"@id\"] = c._id)
    return out
end

struct AssayResult <: AbstractAssayResult
    compound::AbstractCompound
    target::AbstractTarget
    protocol::AbstractAssayProtocol
    ic50_nm::Float64
    replicate_count::Int64
    measurement_date::String
    passed_qc::Bool
    ci_low_nm::Union{Float64, Nothing}
    ci_high_nm::Union{Float64, Nothing}
    _id::Union{String, Nothing}

    function AssayResult(
        compound::AbstractCompound,
        target::AbstractTarget,
        protocol::AbstractAssayProtocol,
        ic50_nm::Float64,
        replicate_count::Int64,
        measurement_date::String,
        passed_qc::Bool;
        ci_low_nm::Union{Float64, Nothing} = nothing,
        ci_high_nm::Union{Float64, Nothing} = nothing,
        _id::Union{String, Nothing} = nothing,
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
        new(compound, target, protocol, ic50_nm, replicate_count, measurement_date, passed_qc, ci_low_nm, ci_high_nm, _id)
    end
end

function decode_AssayResult(m::AbstractDict)::AssayResult
    AssayResult(
        decode_Compound(m[\"urn:eigenius:demo:assay:compound\"]),
        decode_Target(m[\"urn:eigenius:demo:assay:target\"]),
        decode_AssayProtocol(m[\"urn:eigenius:demo:assay:protocol\"]),
        Float64(m[\"urn:eigenius:demo:assay:ic50_nm\"]),
        Int64(m[\"urn:eigenius:demo:assay:replicate_count\"]),
        String(m[\"urn:eigenius:demo:assay:measurement_date\"]),
        Bool(m[\"urn:eigenius:demo:assay:passed_qc\"]);
        ci_low_nm = (let _v = get(m, \"urn:eigenius:demo:assay:ci_low_nm\", nothing); isnothing(_v) ? nothing : (Float64(_v)) end),
        ci_high_nm = (let _v = get(m, \"urn:eigenius:demo:assay:ci_high_nm\", nothing); isnothing(_v) ? nothing : (Float64(_v)) end),
        _id = (let _v = get(m, \"@id\", nothing); isnothing(_v) ? nothing : _v end),
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
    isnothing(c._id) || (out[\"@id\"] = c._id)
    return out
end

const _eigenius_decoders = Dict{String, Function}(
    \"urn:eigenius:demo:assay:AssayProtocol\" => decode_AssayProtocol,
    \"urn:eigenius:demo:assay:Compound\" => decode_Compound,
    \"urn:eigenius:demo:assay:Target\" => decode_Target,
    \"urn:eigenius:demo:assay:AssayResult\" => decode_AssayResult,
)

const _eigenius_encoders = Dict{DataType, Function}(
    AssayProtocol => encode_AssayProtocol,
    Compound => encode_Compound,
    Target => encode_Target,
    AssayResult => encode_AssayResult,
)

export AbstractAssayProtocol, AssayProtocol, decode_AssayProtocol, encode_AssayProtocol, AbstractCompound, Compound, decode_Compound, encode_Compound, AbstractTarget, Target, decode_Target, encode_Target, AbstractAssayResult, AssayResult, decode_AssayResult, encode_AssayResult, _eigenius_decoders, _eigenius_encoders

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

    #[test]
    fn project_toml_is_emitted_alongside_module_source() {
        let out = run_kinase(&["urn:eigenius:demo:assay:Compound"]);
        match &out.library {
            LibraryContent::Embedded(files) => {
                let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
                assert!(paths.contains(&"Project.toml"), "got {paths:?}");
                assert!(paths.contains(&"src/EigeniusMirror.jl"), "got {paths:?}");
            }
            other => panic!("expected Embedded, got {other:?}"),
        }
    }

    #[test]
    fn project_toml_declares_eigenius_julia_common_dep() {
        let out = run_kinase(&["urn:eigenius:demo:assay:Compound"]);
        let LibraryContent::Embedded(files) = &out.library else {
            panic!("expected Embedded library");
        };
        let toml = files
            .iter()
            .find(|f| f.path == "Project.toml")
            .expect("Project.toml present");
        let body = std::str::from_utf8(&toml.content).expect("utf-8");
        assert!(body.contains("name = \"EigeniusMirror\""), "got:\n{body}");
        assert!(
            body.contains("EigeniusJuliaCommon = \"9c8e7a4e-1f2b-4c3d-9e5f-6a7b8c9d0e1f\""),
            "Project.toml must pin the hand-authored Common's UUID, got:\n{body}"
        );
    }

    #[test]
    fn generator_content_hash_has_sha256_shape() {
        // Ontology pins generator_content_hash to ^sha256:[a-f0-9]{64}$
        // so the resource validates at chain commit.
        let g = JuliaMirrorGenerator::new();
        let h = g.generator_content_hash();
        assert!(h.starts_with("sha256:"));
        let hex = &h["sha256:".len()..];
        assert_eq!(hex.len(), 64);
        assert!(hex
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));
    }

    #[test]
    fn mirror_to_resource_carries_required_properties() {
        let g = JuliaMirrorGenerator::new();
        let out = run_kinase(&["urn:eigenius:demo:assay:Compound"]);
        let layer = iri("urn:eigenius:test:layer:l0");
        let r = mirror_to_resource(&g, &out, &layer, Some("1970-01-01T00:00:00Z"));

        let s = |p: &str| {
            r.get(&iri(p))
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| panic!("missing string property `{p}`"))
        };
        assert_eq!(s(PROP_MIRROR_LANGUAGE), "julia");
        assert_eq!(s(PROP_MIRROR_SOURCE_LAYER), "urn:eigenius:test:layer:l0");
        assert_eq!(s(PROP_MIRROR_GEN_ID), "eigon-julia-gen");
        assert_eq!(s(PROP_MIRROR_GEN_VERSION), env!("CARGO_PKG_VERSION"));
        assert_eq!(s(PROP_SHORT_NAME), "EigeniusMirror");
        let h = s(PROP_MIRROR_LIB_CONTENT_HASH);
        assert!(h.starts_with("sha256:"));
        // is_a points at RuntimePackageMirror.
        let is_a = r.get(&iri(PROP_IS_A)).expect("is_a present").as_iri_array();
        assert_eq!(
            is_a,
            vec![iri(CLASS_RUNTIME_PACKAGE_MIRROR)],
            "is_a must point at RuntimePackageMirror"
        );
        // generated_at present when provided.
        assert_eq!(s(PROP_MIRROR_GENERATED_AT), "1970-01-01T00:00:00Z");
        // mirrored_classes lists the class IRIs.
        let cls = r
            .get(&iri(PROP_MIRRORED_CLASSES))
            .expect("mirrored_classes present")
            .as_iri_array();
        assert!(cls.contains(&iri("urn:eigenius:demo:assay:Compound")));
    }

    #[test]
    fn mirror_to_resource_iri_is_derived_from_library_hash() {
        let g = JuliaMirrorGenerator::new();
        // Same closure → same derived IRI.
        let out_a = run_kinase(&["urn:eigenius:demo:assay:Compound"]);
        let out_b = run_kinase(&["urn:eigenius:demo:assay:Compound"]);
        let layer = iri("urn:eigenius:test:layer");
        let ra = mirror_to_resource(&g, &out_a, &layer, None);
        let rb = mirror_to_resource(&g, &out_b, &layer, None);
        assert_eq!(ra.id(), rb.id(), "deterministic mirror IRI");
        // IRI starts with the substrate's mirror namespace.
        assert!(ra
            .id()
            .unwrap()
            .as_str()
            .starts_with("urn:eigenius:runtime:mirror:julia:"));
    }

    #[test]
    fn library_content_json_round_trips_files() {
        // The on-resource JSON must carry every file the generator
        // produced so the substrate's image-build pipeline can
        // materialise the mirror by reading the resource alone.
        let g = JuliaMirrorGenerator::new();
        let out = run_kinase(&["urn:eigenius:demo:assay:Compound"]);
        let layer = iri("urn:eigenius:test:layer");
        let r = mirror_to_resource(&g, &out, &layer, None);
        let json = match r
            .get(&iri(PROP_MIRROR_LIB_CONTENT))
            .expect("library_content")
        {
            Value::Json(v) => v.clone(),
            other => panic!("expected JSON value, got {other:?}"),
        };
        assert_eq!(json["kind"], "embedded");
        let files = json["files"].as_array().expect("files array");
        let paths: Vec<&str> = files
            .iter()
            .filter_map(|f| f.get("path").and_then(|v| v.as_str()))
            .collect();
        assert!(paths.contains(&"Project.toml"));
        assert!(paths.contains(&"src/EigeniusMirror.jl"));
        // Decoded base64 must equal the original bytes.
        for f in files {
            let path = f["path"].as_str().unwrap();
            let b64 = f["content_b64"].as_str().unwrap();
            let original = match &out.library {
                LibraryContent::Embedded(fs) => fs
                    .iter()
                    .find(|x| x.path == path)
                    .expect("path matches generator output")
                    .content
                    .clone(),
                _ => panic!("expected embedded"),
            };
            // Decode base64 here to confirm round-trip — uses the
            // same alphabet as base64_encode in this module.
            let decoded = decode_b64_for_test(b64);
            assert_eq!(decoded, original, "base64 round-trip for `{path}`");
        }
    }

    fn decode_b64_for_test(s: &str) -> Vec<u8> {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut idx = [0u8; 256];
        for (i, &b) in ALPHABET.iter().enumerate() {
            idx[b as usize] = i as u8;
        }
        let bytes = s.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let chunk = &bytes[i..i + 4];
            let pad = chunk.iter().filter(|&&b| b == b'=').count();
            let v0 = idx[chunk[0] as usize] as u32;
            let v1 = idx[chunk[1] as usize] as u32;
            let v2 = if chunk[2] == b'=' {
                0
            } else {
                idx[chunk[2] as usize] as u32
            };
            let v3 = if chunk[3] == b'=' {
                0
            } else {
                idx[chunk[3] as usize] as u32
            };
            let n = (v0 << 18) | (v1 << 12) | (v2 << 6) | v3;
            out.push(((n >> 16) & 0xff) as u8);
            if pad < 2 {
                out.push(((n >> 8) & 0xff) as u8);
            }
            if pad < 1 {
                out.push((n & 0xff) as u8);
            }
            i += 4;
        }
        out
    }

    #[test]
    fn base64_encoder_round_trips_arbitrary_bytes() {
        let cases: [&[u8]; 5] = [b"", b"f", b"fo", b"foo", b"hello world\n\xff\x00\x10"];
        for input in &cases {
            let encoded = base64_encode(input);
            let decoded = decode_b64_for_test(&encoded);
            assert_eq!(&decoded, input, "round-trip for {input:?}");
        }
    }

    // ---- D29 v1.1 conformance tests ----------------------------------------

    fn property_resource_multi(iri_str: &str, short: &str, class_iris: &[&str]) -> Resource {
        let mut r = property_decl(iri_str, short, TYPE_RESOURCE);
        r.set(
            iri(PROP_CLASS_TYPES),
            Value::Array(
                class_iris
                    .iter()
                    .map(|s| Value::ResourceRef(iri(s)))
                    .collect(),
            ),
        );
        r
    }

    fn class_with_subclass_of(
        short: &str,
        parents: &[&str],
        requires: &[&str],
        recommends: &[&str],
    ) -> Resource {
        let mut r = class_decl(short, requires, recommends);
        if !parents.is_empty() {
            r.set(
                iri(PROP_SUBCLASS_OF),
                Value::Array(parents.iter().map(|p| Value::ResourceRef(iri(p))).collect()),
            );
        }
        r
    }

    /// Build a tiny chain with `Animal` as a parent and `Dog`, `Cat`
    /// as concrete subclasses. Animal has property `name`; Dog adds
    /// `breed`; Cat adds `indoor`. Used to exercise the abstract+
    /// struct hierarchy emission and Union dispatch.
    fn build_animal_chain() -> FlatChain {
        let mut chain = FlatChain::new();
        chain.add(
            "urn:eigenius:demo:assay:Animal",
            class_with_subclass_of("Animal", &[], &["urn:eigenius:demo:assay:name"], &[]),
        );
        chain.add(
            "urn:eigenius:demo:assay:name",
            property_decl("urn:eigenius:demo:assay:name", "name", TYPE_STRING),
        );
        chain.add(
            "urn:eigenius:demo:assay:Dog",
            class_with_subclass_of(
                "Dog",
                &["urn:eigenius:demo:assay:Animal"],
                &["urn:eigenius:demo:assay:breed"],
                &[],
            ),
        );
        chain.add(
            "urn:eigenius:demo:assay:breed",
            property_decl("urn:eigenius:demo:assay:breed", "breed", TYPE_STRING),
        );
        chain.add(
            "urn:eigenius:demo:assay:Cat",
            class_with_subclass_of(
                "Cat",
                &["urn:eigenius:demo:assay:Animal"],
                &["urn:eigenius:demo:assay:indoor"],
                &[],
            ),
        );
        chain.add(
            "urn:eigenius:demo:assay:indoor",
            property_decl("urn:eigenius:demo:assay:indoor", "indoor", TYPE_BOOLEAN),
        );
        chain
    }

    fn run_with_chain(chain: &FlatChain, seed: &[&str]) -> MirrorGenerationOutput {
        let layer = iri("urn:eigenius:test:layer");
        let seed_iris: Vec<Iri> = seed.iter().map(|s| iri(s)).collect();
        let request = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed_iris,
            chain,
        };
        JuliaMirrorGenerator::new()
            .generate(&request)
            .expect("generate")
    }

    #[test]
    fn subclass_emission_walks_subclass_of_into_closure() {
        // D29 §3.1: subclass_of ancestors are pulled into the closure
        // automatically. Seeding only on Dog should still pull in Animal.
        let chain = build_animal_chain();
        let out = run_with_chain(&chain, &["urn:eigenius:demo:assay:Dog"]);
        let mirrored: Vec<&str> = out.mirrored_classes.iter().map(|i| i.as_str()).collect();
        assert!(mirrored.contains(&"urn:eigenius:demo:assay:Animal"));
        assert!(mirrored.contains(&"urn:eigenius:demo:assay:Dog"));
    }

    #[test]
    fn subclass_struct_extends_abstract_parent() {
        // D29 §7: `struct Sub <: AbstractParent` for direct supertype.
        let chain = build_animal_chain();
        let out = run_with_chain(&chain, &["urn:eigenius:demo:assay:Dog"]);
        let src = extract_source(&out);
        assert!(src.contains("abstract type AbstractAnimal end"));
        assert!(src.contains("abstract type AbstractDog <: AbstractAnimal end"));
        assert!(src.contains("struct Animal <: AbstractAnimal"));
        assert!(src.contains("struct Dog <: AbstractDog"));
    }

    #[test]
    fn subclass_struct_inherits_parent_fields_first() {
        // D29 §3.2 / §11.1: Dog's struct carries Animal's `name` first
        // (root-to-leaf order), then Dog's own `breed`. _id is last.
        let chain = build_animal_chain();
        let out = run_with_chain(&chain, &["urn:eigenius:demo:assay:Dog"]);
        let src = extract_source(&out);
        let dog_idx = src.find("struct Dog").expect("Dog struct present");
        let dog_block = &src[dog_idx..src[dog_idx..].find("end\n").unwrap() + dog_idx + 4];
        let name_idx = dog_block.find("name::String").expect("Dog has name");
        let breed_idx = dog_block.find("breed::String").expect("Dog has breed");
        assert!(
            name_idx < breed_idx,
            "ancestor's `name` field must precede own `breed` field, got:\n{dog_block}"
        );
    }

    #[test]
    fn polymorphic_union_emits_union_type_and_helpers() {
        // D29 §4 + §8.3: a property with `class_types: [C1, C2]`
        // produces a `Union{AbstractC1, AbstractC2}` field type AND
        // per-field encode/decode dispatch helpers.
        let mut chain = FlatChain::new();
        // Two leaf classes.
        chain.add(
            "urn:eigenius:demo:assay:Foo",
            class_decl("Foo", &["urn:eigenius:demo:assay:foo_name"], &[]),
        );
        chain.add(
            "urn:eigenius:demo:assay:foo_name",
            property_decl("urn:eigenius:demo:assay:foo_name", "foo_name", TYPE_STRING),
        );
        chain.add(
            "urn:eigenius:demo:assay:Bar",
            class_decl("Bar", &["urn:eigenius:demo:assay:bar_name"], &[]),
        );
        chain.add(
            "urn:eigenius:demo:assay:bar_name",
            property_decl("urn:eigenius:demo:assay:bar_name", "bar_name", TYPE_STRING),
        );
        // Owner has a polymorphic field referencing both.
        chain.add(
            "urn:eigenius:demo:assay:Owner",
            class_decl("Owner", &["urn:eigenius:demo:assay:thing"], &[]),
        );
        chain.add(
            "urn:eigenius:demo:assay:thing",
            property_resource_multi(
                "urn:eigenius:demo:assay:thing",
                "thing",
                &["urn:eigenius:demo:assay:Foo", "urn:eigenius:demo:assay:Bar"],
            ),
        );

        let out = run_with_chain(&chain, &["urn:eigenius:demo:assay:Owner"]);
        let src = extract_source(&out);
        // IRI sort: Bar < Foo, so Union renders Bar first.
        assert!(
            src.contains("thing::Union{AbstractBar, AbstractFoo}"),
            "expected Union field type in IRI sort order, got source:\n{src}"
        );
        // Helpers emitted for the polymorphic field.
        assert!(src.contains("function _encode_Owner_thing(v)"));
        assert!(src.contains("function _decode_Owner_thing(m::AbstractDict)"));
        // Encoder dispatches via typeof; decoder via is_a list.
        assert!(src.contains("if v isa Bar"));
        assert!(src.contains("\"urn:eigenius:demo:assay:Bar\" in is_a"));
    }

    #[test]
    fn id_field_round_trips_through_decode_and_encode() {
        // D29 §8.4: every struct exposes `_id` as a Union{String,
        // Nothing} kwarg; decode reads m["@id"], encode stamps
        // out["@id"] when present.
        let out = run_kinase(&["urn:eigenius:demo:assay:Compound"]);
        let src = extract_source(&out);
        // Struct field present.
        assert!(src.contains("_id::Union{String, Nothing}"));
        // Constructor kwarg, last in the list.
        assert!(src.contains("_id::Union{String, Nothing} = nothing"));
        // Decoder reads m["@id"].
        assert!(
            src.contains(
                "_id = (let _v = get(m, \"@id\", nothing); isnothing(_v) ? nothing : _v end)"
            ),
            "expected decoder to read m[\"@id\"], got source:\n{src}"
        );
        // Encoder stamps out["@id"].
        assert!(
            src.contains("isnothing(c._id) || (out[\"@id\"] = c._id)"),
            "expected encoder to stamp out[\"@id\"], got source:\n{src}"
        );
    }

    #[test]
    fn reserved_id_short_name_is_rejected() {
        // D29 §11.1: property short_name `_id` is reserved.
        let mut chain = FlatChain::new();
        chain.add(
            "urn:eigenius:demo:assay:Bad",
            class_decl("Bad", &["urn:eigenius:demo:assay:reserved"], &[]),
        );
        chain.add(
            "urn:eigenius:demo:assay:reserved",
            property_decl("urn:eigenius:demo:assay:reserved", "_id", TYPE_STRING),
        );
        let layer = iri("urn:eigenius:test:layer");
        let seed = vec![iri("urn:eigenius:demo:assay:Bad")];
        let request = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };
        let result = JuliaMirrorGenerator::new().generate(&request);
        match result {
            Err(MirrorGeneratorError::UnrepresentableClass { reason, .. }) => {
                assert!(
                    reason.contains("_id"),
                    "expected `_id` reserved-name error, got reason: {reason}"
                );
            }
            Err(other) => panic!("expected UnrepresentableClass for reserved `_id`, got {other:?}"),
            Ok(_) => panic!("expected UnrepresentableClass for reserved `_id`, got Ok"),
        }
    }

    #[test]
    fn multi_supertype_is_rejected() {
        // D29 §3.2 / §11.1: Julia abstract types are single-inheritance.
        // A class with two `subclass_of` entries cannot be faithfully
        // mirrored.
        let mut chain = FlatChain::new();
        chain.add(
            "urn:eigenius:demo:assay:A",
            class_decl("A", &["urn:eigenius:demo:assay:p_a"], &[]),
        );
        chain.add(
            "urn:eigenius:demo:assay:p_a",
            property_decl("urn:eigenius:demo:assay:p_a", "p_a", TYPE_STRING),
        );
        chain.add(
            "urn:eigenius:demo:assay:B",
            class_decl("B", &["urn:eigenius:demo:assay:p_b"], &[]),
        );
        chain.add(
            "urn:eigenius:demo:assay:p_b",
            property_decl("urn:eigenius:demo:assay:p_b", "p_b", TYPE_STRING),
        );
        chain.add(
            "urn:eigenius:demo:assay:Multi",
            class_with_subclass_of(
                "Multi",
                &["urn:eigenius:demo:assay:A", "urn:eigenius:demo:assay:B"],
                &[],
                &[],
            ),
        );
        let layer = iri("urn:eigenius:test:layer");
        let seed = vec![iri("urn:eigenius:demo:assay:Multi")];
        let request = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };
        let result = JuliaMirrorGenerator::new().generate(&request);
        match result {
            Err(MirrorGeneratorError::UnrepresentableClass { reason, .. }) => {
                assert!(
                    reason.contains("supertype"),
                    "expected multi-supertype error, got: {reason}"
                );
            }
            Err(other) => {
                panic!("expected UnrepresentableClass for multi-supertype, got {other:?}")
            }
            Ok(_) => panic!("expected UnrepresentableClass for multi-supertype, got Ok"),
        }
    }

    #[test]
    fn cycle_in_property_refs_is_rejected() {
        // D29 §3.3: cyclic class graphs aren't representable as Julia
        // structs (no forward references). Reject loudly.
        let mut chain = FlatChain::new();
        chain.add(
            "urn:eigenius:demo:assay:Loop1",
            class_decl("Loop1", &["urn:eigenius:demo:assay:to2"], &[]),
        );
        chain.add(
            "urn:eigenius:demo:assay:to2",
            property_resource(
                "urn:eigenius:demo:assay:to2",
                "to2",
                "urn:eigenius:demo:assay:Loop2",
            ),
        );
        chain.add(
            "urn:eigenius:demo:assay:Loop2",
            class_decl("Loop2", &["urn:eigenius:demo:assay:to1"], &[]),
        );
        chain.add(
            "urn:eigenius:demo:assay:to1",
            property_resource(
                "urn:eigenius:demo:assay:to1",
                "to1",
                "urn:eigenius:demo:assay:Loop1",
            ),
        );
        let layer = iri("urn:eigenius:test:layer");
        let seed = vec![iri("urn:eigenius:demo:assay:Loop1")];
        let request = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };
        let result = JuliaMirrorGenerator::new().generate(&request);
        match result {
            Err(MirrorGeneratorError::UnrepresentableClass { reason, .. }) => {
                assert!(
                    reason.contains("cycle"),
                    "expected cycle-detection error, got: {reason}"
                );
            }
            Err(other) => panic!("expected UnrepresentableClass for cycle, got {other:?}"),
            Ok(_) => panic!("expected UnrepresentableClass for cycle, got Ok"),
        }
    }

    #[test]
    fn cycle_in_subclass_of_is_rejected() {
        // D29 §11.1: subclass_of cycles produce `UnrepresentableClass`.
        let mut chain = FlatChain::new();
        chain.add(
            "urn:eigenius:demo:assay:CycleA",
            class_with_subclass_of("CycleA", &["urn:eigenius:demo:assay:CycleB"], &[], &[]),
        );
        chain.add(
            "urn:eigenius:demo:assay:CycleB",
            class_with_subclass_of("CycleB", &["urn:eigenius:demo:assay:CycleA"], &[], &[]),
        );
        let layer = iri("urn:eigenius:test:layer");
        let seed = vec![iri("urn:eigenius:demo:assay:CycleA")];
        let request = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };
        let result = JuliaMirrorGenerator::new().generate(&request);
        match result {
            Err(MirrorGeneratorError::UnrepresentableClass { reason, .. }) => {
                assert!(
                    reason.contains("cycle") || reason.contains("subclass_of"),
                    "expected subclass_of cycle error, got: {reason}"
                );
            }
            Err(other) => {
                panic!("expected UnrepresentableClass for subclass_of cycle, got {other:?}")
            }
            Ok(_) => panic!("expected UnrepresentableClass for subclass_of cycle, got Ok"),
        }
    }

    #[test]
    fn short_name_conflict_in_inherited_field_set_is_rejected() {
        // D29 §11.1: two distinct property IRIs with the same
        // short_name in a class's transitive field set is invalid.
        let mut chain = FlatChain::new();
        chain.add(
            "urn:eigenius:demo:assay:Parent",
            class_decl("Parent", &["urn:eigenius:demo:assay:weight_kg"], &[]),
        );
        chain.add(
            "urn:eigenius:demo:assay:weight_kg",
            property_decl("urn:eigenius:demo:assay:weight_kg", "weight", TYPE_FLOAT),
        );
        chain.add(
            "urn:eigenius:demo:assay:Child",
            class_with_subclass_of(
                "Child",
                &["urn:eigenius:demo:assay:Parent"],
                &["urn:eigenius:demo:assay:weight_lbs"],
                &[],
            ),
        );
        chain.add(
            "urn:eigenius:demo:assay:weight_lbs",
            property_decl("urn:eigenius:demo:assay:weight_lbs", "weight", TYPE_FLOAT),
        );
        let layer = iri("urn:eigenius:test:layer");
        let seed = vec![iri("urn:eigenius:demo:assay:Child")];
        let request = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };
        let result = JuliaMirrorGenerator::new().generate(&request);
        match result {
            Err(MirrorGeneratorError::UnrepresentableClass { reason, .. }) => {
                assert!(
                    reason.contains("short_name"),
                    "expected short_name conflict error, got: {reason}"
                );
            }
            Err(other) => {
                panic!("expected UnrepresentableClass for short_name conflict, got {other:?}")
            }
            Ok(_) => panic!("expected UnrepresentableClass for short_name conflict, got Ok"),
        }
    }

    #[test]
    fn custom_format_iri_passes_through_to_validator() {
        // D29 §9.3: format IRIs outside `urn:eigenius:core:formats:`
        // pass through as `Symbol("<full IRI>")` rather than being
        // silently dropped.
        let mut chain = FlatChain::new();
        let mut prop = property_decl("urn:eigenius:demo:assay:p", "p", TYPE_STRING);
        prop.set(
            iri(PROP_FORMAT),
            Value::ResourceRef(iri("urn:my:custom:format:foo")),
        );
        chain.add("urn:eigenius:demo:assay:p", prop);
        chain.add(
            "urn:eigenius:demo:assay:Bag",
            class_decl("Bag", &["urn:eigenius:demo:assay:p"], &[]),
        );
        let out = run_with_chain(&chain, &["urn:eigenius:demo:assay:Bag"]);
        let src = extract_source(&out);
        assert!(
            src.contains("validate_format(:p, p, Symbol(\"urn:my:custom:format:foo\"))"),
            "expected full-IRI format passthrough, got source:\n{src}"
        );
    }

    // ──────────────────────────────────────────────────────────────────
    // Inductive emission — D32 §3.6 / Phase 19d.0.c
    // ──────────────────────────────────────────────────────────────────

    /// Build a chain carrying a hand-rolled `Nat = zero | succ(Nat)`
    /// and run the generator. Used by the inductive emission tests.
    fn run_with_nat() -> MirrorGenerationOutput {
        let mut chain = FlatChain::new();

        // ctor `zero`: no args.
        let mut zero = Resource::new(iri("urn:eigenius:test:Nat:zero"));
        zero.set(
            iri(PROP_IS_A),
            Value::Array(vec![Value::ResourceRef(iri(
                "urn:eigenius:core:InductiveCtor",
            ))]),
        );
        zero.set(iri(PROP_CTOR_NAME), Value::String("zero".into()));
        zero.set(iri(PROP_ARG_TYPES), Value::Array(vec![]));

        // ctor `succ(pred: Nat)`.
        let mut succ_arg = Resource::new(iri("urn:eigenius:test:Nat:succ:pred"));
        succ_arg.set(
            iri(PROP_IS_A),
            Value::Array(vec![Value::ResourceRef(iri(
                "urn:eigenius:core:InductiveArgType",
            ))]),
        );
        succ_arg.set(iri(PROP_ARG_NAME), Value::String("pred".into()));
        succ_arg.set(
            iri(PROP_TYPE_NAME),
            Value::String("urn:eigenius:test:Nat".into()),
        );

        let mut succ = Resource::new(iri("urn:eigenius:test:Nat:succ"));
        succ.set(
            iri(PROP_IS_A),
            Value::Array(vec![Value::ResourceRef(iri(
                "urn:eigenius:core:InductiveCtor",
            ))]),
        );
        succ.set(iri(PROP_CTOR_NAME), Value::String("succ".into()));
        succ.set(
            iri(PROP_ARG_TYPES),
            Value::Array(vec![Value::Embedded(Box::new(succ_arg))]),
        );

        let mut nat = Resource::new(iri("urn:eigenius:test:Nat"));
        nat.set(
            iri(PROP_IS_A),
            Value::Array(vec![Value::ResourceRef(iri(CLASS_INDUCTIVE_TYPE))]),
        );
        nat.set(iri(PROP_SHORT_NAME), Value::String("Nat".into()));
        nat.set(
            iri(PROP_CTORS),
            Value::Array(vec![
                Value::Embedded(Box::new(zero)),
                Value::Embedded(Box::new(succ)),
            ]),
        );

        chain.add("urn:eigenius:test:Nat", nat);

        let layer = iri("urn:eigenius:test:layer");
        let seed = vec![iri("urn:eigenius:test:Nat")];
        JuliaMirrorGenerator::new()
            .generate(&MirrorGenerationRequest {
                source_layer: &layer,
                seed_classes: &seed,
                chain: &chain,
            })
            .expect("nat mirror generation")
    }

    #[test]
    fn inductive_emits_abstract_type_and_concrete_ctor_structs() {
        let out = run_with_nat();
        let src = extract_source(&out);

        assert!(
            src.contains("abstract type Nat end"),
            "missing `abstract type Nat end`; got:\n{src}"
        );
        assert!(
            src.contains("struct Nat_zero <: Nat"),
            "missing `struct Nat_zero <: Nat`; got:\n{src}"
        );
        assert!(
            src.contains("struct Nat_succ <: Nat"),
            "missing `struct Nat_succ <: Nat`; got:\n{src}"
        );
        // `pred` field on Nat_succ is typed `Nat` (the abstract).
        assert!(
            src.contains("pred::Nat"),
            "Nat_succ.pred must be typed `Nat`; got:\n{src}"
        );
    }

    #[test]
    fn inductive_emits_decoder_dispatching_on_ctor_string() {
        let src = extract_source(&run_with_nat());
        assert!(
            src.contains("function decode_Nat(d::AbstractDict)::Nat"),
            "missing decoder signature; got:\n{src}"
        );
        assert!(
            src.contains("ctor == \"zero\""),
            "decoder must dispatch on `zero`; got:\n{src}"
        );
        assert!(
            src.contains("ctor == \"succ\""),
            "decoder must dispatch on `succ`; got:\n{src}"
        );
        assert!(
            src.contains("decode_Nat(args[1])"),
            "succ branch must recurse via decode_Nat; got:\n{src}"
        );
    }

    #[test]
    fn inductive_emits_encoder_dispatching_on_concrete_struct_type() {
        let src = extract_source(&run_with_nat());
        assert!(
            src.contains("function encode_Nat(v::Nat)::Dict{String, Any}"),
            "missing encoder signature; got:\n{src}"
        );
        assert!(
            src.contains("v isa Nat_zero"),
            "encoder must isa-dispatch on Nat_zero; got:\n{src}"
        );
        assert!(
            src.contains("v isa Nat_succ"),
            "encoder must isa-dispatch on Nat_succ; got:\n{src}"
        );
        assert!(
            src.contains("\"ctor\" => \"succ\""),
            "encoder must produce the chain ctor name; got:\n{src}"
        );
    }

    #[test]
    fn inductive_registers_in_decoders_and_encoders_maps() {
        let src = extract_source(&run_with_nat());
        // Decoder map is keyed on the InductiveType IRI.
        assert!(
            src.contains("\"urn:eigenius:test:Nat\" => decode_Nat"),
            "decoder map missing Nat IRI entry; got:\n{src}"
        );
        // Encoder map has one entry per concrete ctor struct.
        assert!(
            src.contains("Nat_zero => encode_Nat"),
            "encoder map missing Nat_zero entry; got:\n{src}"
        );
        assert!(
            src.contains("Nat_succ => encode_Nat"),
            "encoder map missing Nat_succ entry; got:\n{src}"
        );
    }

    #[test]
    fn inductive_exports_abstract_concrete_decode_encode() {
        let src = extract_source(&run_with_nat());
        let export_line = src
            .lines()
            .find(|l| l.starts_with("export "))
            .expect("export line present");
        for token in ["Nat", "Nat_zero", "Nat_succ", "decode_Nat", "encode_Nat"] {
            assert!(
                export_line.contains(token),
                "export line missing `{token}`: {export_line}"
            );
        }
    }
}
