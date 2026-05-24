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

//! `LeanMirrorGenerator` — substrate Rust code that walks the chain's
//! ontology layer and emits a Lean Lake package (`EigeniusFFI/`)
//! whose source faithfully translates each Eigon class into a Lean
//! `structure`. Output is committed back to the chain as a
//! `LeanPackageMirror` resource and baked into the `LeanEnvironment`
//! image.
//!
//! Spec: [D30 — Eigon → Lean Faithful Translation](../../../../../docs/design/d30-eigon-to-lean-faithful-translation.md).
//! Sibling: [`eigenius_julia::mirror_gen`] — same closure semantics
//! and topological-sort discipline, narrower v1 supported subset.
//!
//! ## What lands in this module
//!
//! Pipeline stages, top to bottom in the source:
//!
//! 1. **Types** — `LeanType`, `ClassDecl`, `PropertyDecl`,
//!    `PropertyConstraints`. The intermediate representation the
//!    closure walker hands to the emitters.
//! 2. **Closure walk** — `walk_closure` collects every class
//!    transitively reachable through `requires`/`recommends`
//!    `class_types` and `subclass_of` edges.
//! 3. **Resolution** — `resolve_class_declarations` turns each
//!    closure-member `Resource` into a `ClassDecl` with resolved
//!    parents and resolved properties.
//! 4. **Topological sort** — `topological_order` orders classes so
//!    every structure's field types are defined earlier in the
//!    module (Lean's `structure` declaration disallows forward
//!    references in v1).
//! 5. **Emitters** (subsequent commits) — turn the ordered
//!    `ClassDecl`s into Lean source.
//!
//! ## D30 v1 supported subset reminder
//!
//! `walk_closure` and the resolution layer fail (with
//! `MirrorGeneratorError::UnrepresentableClass`) on:
//! - cycles in property `class_types` or `subclass_of`
//! - empty `class_types` on a `resource` / `resource_array`
//! - non-Lean-identifier `short_name`s (capital-first for classes,
//!   lower-first for properties)
//! - reserved `_id` property name
//! - duplicate field `short_name`s across the inherited+own surface
//!
//! The emitter layer adds more shape checks (numeric constraint
//! type compatibility, pattern/format rendering) but the pre-emit
//! pipeline catches every error D30 §11.1 enumerates.

use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_runtime_substrate::mirror_generator::{MirrorGenerationRequest, MirrorGeneratorError};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) mod codec_emitter;
pub(crate) mod module_assembler;
pub(crate) mod structure_emitter;

use eigenius_runtime_substrate::mirror_generator::{
    LibraryContent, LibraryFile, MirrorGenerationOutput, MirrorGenerator,
};
use std::sync::OnceLock;

/// Stable identifier the mirror generator stamps on every emitted
/// `LeanPackageMirror` resource (D30 §10.2). Pinned across versions —
/// the identifier names the generator, not its release.
const GENERATOR_ID: &str = "eigon-ffi-gen";

/// Production `LanguageRuntime`-driven entry point for the Lean
/// mirror generator. Stateless across calls — every `generate()`
/// re-walks the supplied chain — but the per-instance content
/// hash is cached once at construction so successive `generate()`s
/// avoid the SHA-256 recompute.
pub struct LeanMirrorGenerator {
    version: &'static str,
    content_hash: OnceLock<String>,
}

impl LeanMirrorGenerator {
    pub fn new() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            content_hash: OnceLock::new(),
        }
    }

    /// Compute (and cache) the v1 placeholder generator-content
    /// hash. D30 §10.2: until we wire up a real binary hash, derive
    /// the value from `(generator_id, version)` so the
    /// integrity-chain shape matches the ontology's pinned regex
    /// (`^sha256:[a-f0-9]{64}$`).
    fn compute_content_hash(&self) -> &str {
        self.content_hash.get_or_init(|| {
            use sha2::Digest;
            let mut hasher = sha2::Sha256::new();
            hasher.update(GENERATOR_ID.as_bytes());
            hasher.update(b":");
            hasher.update(self.version.as_bytes());
            format!("sha256:{:x}", hasher.finalize())
        })
    }
}

impl Default for LeanMirrorGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl MirrorGenerator for LeanMirrorGenerator {
    fn generator_identifier(&self) -> &str {
        GENERATOR_ID
    }

    fn generator_version(&self) -> &str {
        self.version
    }

    fn generator_content_hash(&self) -> &str {
        self.compute_content_hash()
    }

    fn generate(
        &self,
        request: &MirrorGenerationRequest,
    ) -> Result<MirrorGenerationOutput, MirrorGeneratorError> {
        // Pipeline: closure walk → resolution → topological sort →
        // module assembly. Each stage's failure surface is its own;
        // the trait impl is just plumbing.
        let decls = build_decls(request)?;
        let lookup = class_name_lookup(&decls);
        let order = topological_emit_order(&decls)?;
        let files = module_assembler::assemble_mirror_package(
            &decls,
            &order,
            &lookup,
            request.source_layer,
            crate::conventions::LEAN_TOOLCHAIN_VERSION,
        );

        // Convert local `AssembledFile`s to the substrate-typed
        // `LibraryFile`s at the API boundary. The two structs are
        // path+content twins; the duplication keeps the assembler
        // module testable without dragging in the substrate's mirror
        // type at unit-test time.
        let library_files: Vec<LibraryFile> = files
            .into_iter()
            .map(|f| LibraryFile {
                path: f.path,
                content: f.content,
            })
            .collect();

        // D30 §10.1: mirrored_classes sorted by IRI for determinism.
        // `topological_emit_order` returns ordering for emission;
        // the resource-level list is independently sorted.
        let mut mirrored: Vec<Iri> = decls.keys().cloned().collect();
        mirrored.sort();

        Ok(MirrorGenerationOutput {
            mirrored_classes: mirrored,
            library: LibraryContent::Embedded(library_files),
        })
    }
}

// Re-export the integrity helpers + identifier for callers that need
// to round-trip a mirror through the chain (mirror_to_resource etc.
// land alongside the orchestrator-side commit pipeline).
pub use module_assembler::{derive_mirror_iri, library_content_hash, AssembledFile};

// ---------------------------------------------------------------------------
// Chain commit — `mirror_to_resource`
// ---------------------------------------------------------------------------

/// Built-in name the generator stamps on every `LeanPackageMirror`
/// resource's `short_name` property. Pinned because v1's package
/// layout (D30 §2) emits a fixed-name Lake package.
const TARGET_PACKAGE_NAME: &str = "EigeniusFFI";

/// The language tag the Lean mirror commits under
/// `runtime:language`. Matches the `LeanLanguageRuntime`'s
/// `language_id`.
const LANGUAGE_LEAN_TAG: &str = "lean";

// Substrate-pinned property IRIs the chain commit reads. Kept
// local to mirror Julia's pattern — diffing the two lists
// surfaces drift between language runtimes.

const CLASS_RUNTIME_PACKAGE_MIRROR: &str = "urn:eigenius:runtime:RuntimePackageMirror";
const PROP_IS_A_IRI: &str = "urn:eigenius:core:is_a";
const PROP_SHORT_NAME_IRI: &str = "urn:eigenius:core:short_name";
const PROP_DESCRIPTION_IRI: &str = "urn:eigenius:core:description";
const PROP_MIRROR_LANGUAGE: &str = "urn:eigenius:runtime:language";
const PROP_MIRROR_SOURCE_LAYER: &str = "urn:eigenius:runtime:source_layer";
const PROP_MIRROR_GEN_ID: &str = "urn:eigenius:runtime:generator_identifier";
const PROP_MIRROR_GEN_VERSION: &str = "urn:eigenius:runtime:generator_version";
const PROP_MIRROR_GEN_CONTENT_HASH: &str = "urn:eigenius:runtime:generator_content_hash";
const PROP_MIRROR_LIB_CONTENT_HASH: &str = "urn:eigenius:runtime:library_content_hash";
const PROP_MIRROR_LIB_CONTENT: &str = "urn:eigenius:runtime:library_content";
const PROP_MIRRORED_CLASSES: &str = "urn:eigenius:runtime:mirrored_classes";
const PROP_MIRROR_GENERATED_AT: &str = "urn:eigenius:runtime:generated_at";

/// Commit a generated mirror output as a `LeanPackageMirror`
/// Resource ready for chain insertion. Pins the D30 §10.2
/// integrity chain (`generator_identifier`, `generator_version`,
/// `generator_content_hash`, `library_content_hash`) plus the
/// substrate's mirror-archive JSON shape that
/// `runtime-substrate::image_build::context::MirrorMaterialization`
/// decodes at image-build time.
///
/// `generated_at` is caller-supplied so deterministic tests can
/// pin a constant timestamp; production callers stamp the wall
/// clock. The Resource's `@id` derives from the library content
/// hash via [`derive_mirror_iri`] — byte-identical mirrors land at
/// the same IRI, enabling chain dedupe.
pub fn mirror_to_resource(
    generator: &dyn MirrorGenerator,
    output: &MirrorGenerationOutput,
    source_layer: &Iri,
    generated_at: Option<&str>,
) -> Resource {
    use eigenius_kernel::ontology::resource::Value;

    let content_hash = compute_library_content_hash(&output.library);
    let library_json = library_content_to_json(&output.library);
    let mirror_iri = derive_mirror_iri(&content_hash);

    let mut r = Resource::new(mirror_iri);
    r.set(
        Iri::parse(PROP_IS_A_IRI).expect("static IRI"),
        Value::Array(vec![Value::ResourceRef(
            Iri::parse(CLASS_RUNTIME_PACKAGE_MIRROR).expect("static IRI"),
        )]),
    );
    r.set(
        Iri::parse(PROP_SHORT_NAME_IRI).expect("static IRI"),
        Value::String(TARGET_PACKAGE_NAME.to_string()),
    );
    r.set(
        Iri::parse(PROP_DESCRIPTION_IRI).expect("static IRI"),
        Value::String(format!(
            "Generated Lean mirror covering {} class(es) from {}.",
            output.mirrored_classes.len(),
            source_layer.as_str()
        )),
    );
    r.set(
        Iri::parse(PROP_MIRROR_LANGUAGE).expect("static IRI"),
        Value::String(LANGUAGE_LEAN_TAG.to_string()),
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
        Value::String(content_hash),
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

/// SHA-256 of the library archive's bytes. For `Embedded` libraries,
/// re-uses [`library_content_hash`] (the substrate-side
/// length-prefixed framing); for `External` libraries, returns the
/// caller-supplied content hash unchanged.
fn compute_library_content_hash(library: &LibraryContent) -> String {
    match library {
        LibraryContent::Embedded(files) => {
            let assembled: Vec<AssembledFile> = files
                .iter()
                .map(|f| AssembledFile {
                    path: f.path.clone(),
                    content: f.content.clone(),
                })
                .collect();
            library_content_hash(&assembled)
        }
        LibraryContent::External { content_hash, .. } => content_hash.clone(),
    }
}

/// Encode the library archive as the JSON shape the substrate's
/// `MirrorMaterialization` decoder accepts. Per
/// `runtime-substrate::mirror_generator::LibraryContent::Embedded`,
/// the shape is `{"kind": "embedded", "files": [{"path", "content_b64"}]}`.
/// Files are sorted by path so the JSON itself is deterministic.
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

/// Hand-rolled RFC 4648 §4 base64 encoder — same shape as Julia's
/// mirror_gen carries, kept local so the crate keeps a tight dep
/// set.
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

/// Emit one class's full per-class block — `structure` + `CoeOut`
/// instances + `decodeC` + `encodeC` — separated by single blank
/// lines per D30 §6. Public so the Lake-build integration test
/// drives the real emitters end-to-end; the module-assembly layer
/// will call this same function for each class in the topological
/// order.
///
/// `decls` must contain every class transitively referenced by
/// `decl` (the result of `walk_closure` + `resolve_class_declarations`);
/// `lookup` is the IRI→`short_name` table over the same set.
pub fn emit_class_block(
    decl: &ClassDecl,
    decls: &BTreeMap<Iri, ClassDecl>,
    lookup: &structure_emitter::ClassNameLookup,
) -> String {
    let mut out = structure_emitter::emit_structure_block(decl, lookup);
    out.push_str(&codec_emitter::emit_codec_block(decl, decls, lookup));
    out
}

/// Build the `IRI → short_name` table the emitter consumes.
/// Public for the same reasons as [`emit_class_block`].
pub fn class_name_lookup(decls: &BTreeMap<Iri, ClassDecl>) -> structure_emitter::ClassNameLookup {
    structure_emitter::class_name_lookup(decls)
}

/// Public re-export of the closure walker — drives the resolution
/// pipeline from a `MirrorGenerationRequest`. The integration test
/// uses this to build the `decls` map; the eventual trait impl
/// will call it the same way.
pub fn build_decls(
    request: &MirrorGenerationRequest,
) -> Result<BTreeMap<Iri, ClassDecl>, MirrorGeneratorError> {
    let closure = walk_closure(request)?;
    resolve_class_declarations(request, &closure)
}

/// Public re-export of the topological sort — orders the resolved
/// `decls` for emission. Cycles in `class_types` references surface
/// as `UnrepresentableClass`.
pub fn topological_emit_order(
    decls: &BTreeMap<Iri, ClassDecl>,
) -> Result<Vec<Iri>, MirrorGeneratorError> {
    topological_order(decls)
}

// ---------------------------------------------------------------------------
// Property IRI constants — pinned by the core ontology + D30 spec.
// Local to this module so the file is self-contained relative to
// `crates/eigenius-julia/src/mirror_gen.rs`; diffing the two lists
// surfaces any IRI drift between the Julia and Lean generators.
// ---------------------------------------------------------------------------

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
const PROP_DESCRIPTION: &str = "urn:eigenius:core:description";

const CORE_STRING: &str = "urn:eigenius:core:string";
const CORE_INTEGER: &str = "urn:eigenius:core:integer";
const CORE_FLOAT: &str = "urn:eigenius:core:float";
const CORE_BOOLEAN: &str = "urn:eigenius:core:boolean";
const CORE_JSON: &str = "urn:eigenius:core:json";
const CORE_RESOURCE: &str = "urn:eigenius:core:resource";
const CORE_RESOURCE_ARRAY: &str = "urn:eigenius:core:resource_array";
const CORE_VALUE_ARRAY: &str = "urn:eigenius:core:value_array";

const LANGUAGE_LEAN: &str = "lean";

/// Reserved field name in every generated structure — carries the
/// resource's `@id` through decode/encode round-trips (D30 §7.2).
/// A chain-side property whose `short_name` is `_id` produces a
/// resolution-time error.
const RESERVED_ID_FIELD: &str = "_id";

// ---------------------------------------------------------------------------
// Intermediate representation
// ---------------------------------------------------------------------------

/// Lean type the emitter renders for a property's field. Closed sum
/// type — every D30 §4 row is one variant. The emitter dispatches on
/// this and never re-parses the original `data_type` IRI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeanType {
    String,
    Int,
    Float,
    Bool,
    Json,
    /// `class_types: [C]` — singleton resource reference. Resolved
    /// `class_iri` is in the mirror closure.
    ClassRef(Iri),
    /// `class_types: [C₁, …, Cₙ]` (n ≥ 2) — multi-class polymorphic.
    /// Rendered as `EigeniusUnion [C₁, …, Cₙ]` (D30 §4.3). IRIs are
    /// sorted (canonical for determinism).
    Union(Vec<Iri>),
    /// `resource_array` with `class_types: [C]` — `List C`.
    ListClassRef(Iri),
    /// `resource_array` with `class_types: [C₁, …, Cₙ]` (n ≥ 2) —
    /// `List (EigeniusUnion [...])`.
    ListUnion(Vec<Iri>),
    /// `value_array` with primitive `element_type`. The boxed
    /// primitive type is one of `String`/`Int`/`Float`/`Bool`/`Json`.
    ListPrimitive(Box<LeanType>),
}

impl LeanType {
    /// IRIs of every class this type structurally references.
    /// Drives both the closure-walk reachability check and the
    /// topological sort's edge enumeration.
    fn class_refs(&self) -> Vec<&Iri> {
        match self {
            LeanType::ClassRef(c) | LeanType::ListClassRef(c) => vec![c],
            LeanType::Union(cs) | LeanType::ListUnion(cs) => cs.iter().collect(),
            LeanType::String
            | LeanType::Int
            | LeanType::Float
            | LeanType::Bool
            | LeanType::Json
            | LeanType::ListPrimitive(_) => Vec::new(),
        }
    }
}

/// Constraints attached to a property declaration (D30 §9). The
/// emitter consults these to decide whether a field's Lean type is
/// the bare type or a refinement subtype, and whether the decoder
/// chains in `validatePattern` / `validateFormat` calls.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PropertyConstraints {
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
    pub min_length: Option<u64>,
    pub max_length: Option<u64>,
    pub pattern: Option<String>,
    pub format: Option<String>,
}

/// One property on a class, after all `class_types` IRIs have been
/// resolved against the closure and the type has been classified.
#[derive(Debug, Clone)]
pub struct PropertyDecl {
    /// Chain-side property IRI (used by the decoder's `getObjValAs?`
    /// lookup; this is the JSON key).
    pub property_iri: Iri,
    /// `core:short_name` — the Lean field name on the structure.
    /// Validated to be a lowercase-first Lean identifier at
    /// resolution time.
    pub short_name: String,
    pub lean_type: LeanType,
    pub constraints: PropertyConstraints,
    /// `core:description` if the chain provided one — passed through
    /// as a Lean docstring (`/-- … -/`) above the field.
    pub description: Option<String>,
}

/// Resolved class declaration ready for emission. Carries the
/// fields in the order D30 §5 mandates (parents' fields elided
/// here — they're transitively materialised at emit time via the
/// `extends` clause).
#[derive(Debug, Clone)]
pub struct ClassDecl {
    pub class_iri: Iri,
    /// `core:short_name` — the Lean structure name. Validated to be
    /// a capital-first Lean identifier at resolution time.
    pub short_name: String,
    /// `core:description` — emitted as a docstring above the
    /// structure declaration.
    pub description: Option<String>,
    /// Parent class IRIs from `core:subclass_of`, in chain-declared
    /// order. All parents are in the closure (`walk_closure` pulls
    /// them in).
    pub parents: Vec<Iri>,
    /// Properties in `core:requires`, in chain-declared order.
    pub requires: Vec<PropertyDecl>,
    /// Properties in `core:recommends`, in chain-declared order.
    pub recommends: Vec<PropertyDecl>,
}

// ---------------------------------------------------------------------------
// Closure walk (D30 §3)
// ---------------------------------------------------------------------------

/// Result of [`walk_closure`] — the structural closure of the seed
/// over reachable classes.
#[derive(Debug, Default)]
pub(crate) struct ClosureResult {
    /// Every class IRI that should appear as a `structure` in the
    /// emitted module, in IRI-sorted order. Includes the seed and
    /// every transitively-reachable class.
    pub classes: BTreeSet<Iri>,
}

/// Walk the chain from `seed_classes`, collecting every class
/// reachable via `requires`/`recommends` `class_types` references
/// or `subclass_of` edges. Mirrors `eigenius_julia`'s
/// `walk_closure` modulo Lean v1's narrower subset (no
/// `InductiveType` bucket — those land with D30 v1.1).
///
/// Returns `UnknownClass` when a queued class IRI can't be resolved
/// at `source_layer`.
fn walk_closure(request: &MirrorGenerationRequest) -> Result<ClosureResult, MirrorGeneratorError> {
    let mut classes: BTreeSet<Iri> = BTreeSet::new();
    let mut queue: Vec<Iri> = request.seed_classes.to_vec();

    while let Some(iri) = queue.pop() {
        if classes.contains(&iri) {
            continue;
        }

        let def = request
            .chain
            .resolve(request.source_layer, &iri)
            .ok_or_else(|| MirrorGeneratorError::UnknownClass(iri.as_str().to_string()))?;

        classes.insert(iri.clone());

        // Walk properties — every `class_types` IRI is a closure edge.
        for prop_iri in iri_array(&def, PROP_REQUIRES)
            .into_iter()
            .chain(iri_array(&def, PROP_RECOMMENDS))
        {
            let prop_def = match request.chain.resolve(request.source_layer, &prop_iri) {
                Some(r) => r,
                None => continue,
            };
            for class_ref in property_class_references(&prop_def) {
                if classes.contains(&class_ref) {
                    continue;
                }
                queue.push(class_ref);
            }
        }

        // Walk `subclass_of` parents (D30 §3.2). Lean's multi-parent
        // `extends` makes this a straight enumeration; no
        // single-supertype gymnastics like Julia.
        for parent in iri_array(&def, PROP_SUBCLASS_OF) {
            if classes.contains(&parent) {
                continue;
            }
            queue.push(parent);
        }
    }

    Ok(ClosureResult { classes })
}

// ---------------------------------------------------------------------------
// Class & property resolution (D30 §§4–7)
// ---------------------------------------------------------------------------

/// Resolve each closure member to a `ClassDecl`. Per-class validation
/// follows D30 §11.1 — `short_name` shape, reserved `_id`, duplicate
/// field detection, supported `data_type`/`element_type` only.
fn resolve_class_declarations(
    request: &MirrorGenerationRequest,
    closure: &ClosureResult,
) -> Result<BTreeMap<Iri, ClassDecl>, MirrorGeneratorError> {
    let mut decls: BTreeMap<Iri, ClassDecl> = BTreeMap::new();
    for iri in &closure.classes {
        let def = request
            .chain
            .resolve(request.source_layer, iri)
            .ok_or_else(|| MirrorGeneratorError::UnknownClass(iri.as_str().to_string()))?;
        let decl = resolve_one_class(request, &closure.classes, iri, &def)?;
        decls.insert(iri.clone(), decl);
    }

    // Cross-class checks: duplicate-field detection across the
    // transitive inherited surface (D30 §11.1 — "the transitive
    // field set has unique property short_names").
    for iri in decls.keys().cloned().collect::<Vec<_>>() {
        validate_unique_inherited_fields(&decls, &iri)?;
    }

    Ok(decls)
}

fn resolve_one_class(
    request: &MirrorGenerationRequest,
    closure: &BTreeSet<Iri>,
    iri: &Iri,
    def: &Resource,
) -> Result<ClassDecl, MirrorGeneratorError> {
    let short_name = read_string_property(def, PROP_SHORT_NAME)
        .ok_or_else(|| MirrorGeneratorError::UnrepresentableClass {
            class_iri: iri.as_str().to_string(),
            language: LANGUAGE_LEAN.to_string(),
            reason: "class is missing `core:short_name` — generator needs a Lean structure name"
                .to_string(),
        })?
        .to_string();
    validate_class_identifier(iri, &short_name)?;

    let parents = iri_array(def, PROP_SUBCLASS_OF);
    // Every parent must already be in the closure (walk_closure puts
    // them there); a parent outside the closure is an internal bug.
    // Re-asserting here keeps the resolution layer hermetic.
    for parent in &parents {
        if !closure.contains(parent) {
            return Err(MirrorGeneratorError::UnrepresentableClass {
                class_iri: iri.as_str().to_string(),
                language: LANGUAGE_LEAN.to_string(),
                reason: format!(
                    "subclass_of parent `{}` not in closure; walk_closure should have included it",
                    parent.as_str()
                ),
            });
        }
    }

    let description = read_string_property(def, PROP_DESCRIPTION).map(String::from);

    let requires = resolve_properties(request, closure, iri, def, PROP_REQUIRES)?;
    let recommends = resolve_properties(request, closure, iri, def, PROP_RECOMMENDS)?;

    // Own-class shadow check — a property name can't appear in
    // both requires and recommends (D29/D30 §4.1 — listed in both
    // is malformed). Plus the reserved `_id` slot.
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for prop in requires.iter().chain(recommends.iter()) {
        if prop.short_name == RESERVED_ID_FIELD {
            return Err(MirrorGeneratorError::UnrepresentableClass {
                class_iri: iri.as_str().to_string(),
                language: LANGUAGE_LEAN.to_string(),
                reason: format!(
                    "property `{}` uses reserved short_name `_id` (D30 §7.2)",
                    prop.property_iri.as_str()
                ),
            });
        }
        if !seen.insert(prop.short_name.as_str()) {
            return Err(MirrorGeneratorError::UnrepresentableClass {
                class_iri: iri.as_str().to_string(),
                language: LANGUAGE_LEAN.to_string(),
                reason: format!(
                    "duplicate property short_name `{}` (appears in both requires and recommends, or twice in one)",
                    prop.short_name
                ),
            });
        }
    }

    Ok(ClassDecl {
        class_iri: iri.clone(),
        short_name,
        description,
        parents,
        requires,
        recommends,
    })
}

fn resolve_properties(
    request: &MirrorGenerationRequest,
    closure: &BTreeSet<Iri>,
    class_iri: &Iri,
    class_def: &Resource,
    list_iri: &str,
) -> Result<Vec<PropertyDecl>, MirrorGeneratorError> {
    let mut out = Vec::new();
    for prop_iri in iri_array(class_def, list_iri) {
        let prop_def = request
            .chain
            .resolve(request.source_layer, &prop_iri)
            .ok_or_else(|| MirrorGeneratorError::UnrepresentableClass {
                class_iri: class_iri.as_str().to_string(),
                language: LANGUAGE_LEAN.to_string(),
                reason: format!(
                    "property `{}` referenced from `{list_iri}` not found in chain at source_layer",
                    prop_iri.as_str()
                ),
            })?;
        let short_name = read_string_property(&prop_def, PROP_SHORT_NAME)
            .ok_or_else(|| MirrorGeneratorError::UnrepresentableClass {
                class_iri: class_iri.as_str().to_string(),
                language: LANGUAGE_LEAN.to_string(),
                reason: format!(
                    "property `{}` missing `core:short_name` — generator needs a Lean field name",
                    prop_iri.as_str()
                ),
            })?
            .to_string();
        validate_property_identifier(class_iri, &prop_iri, &short_name)?;
        let lean_type = resolve_property_type(class_iri, &prop_iri, &prop_def, closure)?;
        let constraints = read_constraints(&prop_def);
        let description = read_string_property(&prop_def, PROP_DESCRIPTION).map(String::from);
        out.push(PropertyDecl {
            property_iri: prop_iri,
            short_name,
            lean_type,
            constraints,
            description,
        });
    }
    Ok(out)
}

fn resolve_property_type(
    class_iri: &Iri,
    prop_iri: &Iri,
    prop_def: &Resource,
    closure: &BTreeSet<Iri>,
) -> Result<LeanType, MirrorGeneratorError> {
    let data_type = resource_iri_value(prop_def, PROP_DATA_TYPE).ok_or_else(|| {
        MirrorGeneratorError::UnrepresentableClass {
            class_iri: class_iri.as_str().to_string(),
            language: LANGUAGE_LEAN.to_string(),
            reason: format!(
                "property `{}` missing `core:data_type` — D30 §4 requires a typed projection",
                prop_iri.as_str()
            ),
        }
    })?;

    match data_type.as_str() {
        CORE_STRING => Ok(LeanType::String),
        CORE_INTEGER => Ok(LeanType::Int),
        CORE_FLOAT => Ok(LeanType::Float),
        CORE_BOOLEAN => Ok(LeanType::Bool),
        CORE_JSON => Ok(LeanType::Json),
        CORE_RESOURCE => {
            let classes = resolve_class_types(class_iri, prop_iri, prop_def, closure)?;
            Ok(if classes.len() == 1 {
                LeanType::ClassRef(classes.into_iter().next().unwrap())
            } else {
                LeanType::Union(classes)
            })
        }
        CORE_RESOURCE_ARRAY => {
            let classes = resolve_class_types(class_iri, prop_iri, prop_def, closure)?;
            Ok(if classes.len() == 1 {
                LeanType::ListClassRef(classes.into_iter().next().unwrap())
            } else {
                LeanType::ListUnion(classes)
            })
        }
        CORE_VALUE_ARRAY => {
            let element_type = resource_iri_value(prop_def, PROP_ELEMENT_TYPE).ok_or_else(|| {
                MirrorGeneratorError::UnrepresentableClass {
                    class_iri: class_iri.as_str().to_string(),
                    language: LANGUAGE_LEAN.to_string(),
                    reason: format!(
                        "property `{}`: `core:value_array` requires `core:element_type`",
                        prop_iri.as_str()
                    ),
                }
            })?;
            let inner = match element_type.as_str() {
                CORE_STRING => LeanType::String,
                CORE_INTEGER => LeanType::Int,
                CORE_FLOAT => LeanType::Float,
                CORE_BOOLEAN => LeanType::Bool,
                CORE_JSON => LeanType::Json,
                other => {
                    return Err(MirrorGeneratorError::UnrepresentableClass {
                        class_iri: class_iri.as_str().to_string(),
                        language: LANGUAGE_LEAN.to_string(),
                        reason: format!(
                            "property `{}`: unsupported `element_type` `{other}` for value_array (v1 supports string/integer/float/boolean/json)",
                            prop_iri.as_str()
                        ),
                    });
                }
            };
            Ok(LeanType::ListPrimitive(Box::new(inner)))
        }
        other => Err(MirrorGeneratorError::UnrepresentableClass {
            class_iri: class_iri.as_str().to_string(),
            language: LANGUAGE_LEAN.to_string(),
            reason: format!(
                "property `{}`: unsupported `data_type` `{other}` (D30 §4 v1 supports only the documented table)",
                prop_iri.as_str()
            ),
        }),
    }
}

/// Read and validate a `class_types` list. Returns the IRIs in
/// **sorted** order — D30 §4.3 mandates canonical ordering for
/// `EigeniusUnion` determinism.
fn resolve_class_types(
    class_iri: &Iri,
    prop_iri: &Iri,
    prop_def: &Resource,
    closure: &BTreeSet<Iri>,
) -> Result<Vec<Iri>, MirrorGeneratorError> {
    let raw = iri_array(prop_def, PROP_CLASS_TYPES);
    if raw.is_empty() {
        return Err(MirrorGeneratorError::UnrepresentableClass {
            class_iri: class_iri.as_str().to_string(),
            language: LANGUAGE_LEAN.to_string(),
            reason: format!(
                "property `{}`: `core:resource` / `core:resource_array` requires non-empty `class_types` (D30 §4)",
                prop_iri.as_str()
            ),
        });
    }
    // Verify every referenced class is in the closure. walk_closure
    // pulls them in transitively, so a miss here is an internal
    // invariant violation surfaced as an UnknownClass error to keep
    // the failure mode legible.
    for c in &raw {
        if !closure.contains(c) {
            return Err(MirrorGeneratorError::UnknownClass(c.as_str().to_string()));
        }
    }
    // Sort + dedupe for canonical order.
    let mut sorted: Vec<Iri> = raw
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    sorted.sort();
    Ok(sorted)
}

fn read_constraints(prop_def: &Resource) -> PropertyConstraints {
    PropertyConstraints {
        min_value: numeric_value(prop_def, PROP_MIN_VALUE),
        max_value: numeric_value(prop_def, PROP_MAX_VALUE),
        min_length: integer_value(prop_def, PROP_MIN_LENGTH).and_then(|n| u64::try_from(n).ok()),
        max_length: integer_value(prop_def, PROP_MAX_LENGTH).and_then(|n| u64::try_from(n).ok()),
        pattern: read_string_property(prop_def, PROP_PATTERN).map(String::from),
        format: resource_iri_value(prop_def, PROP_FORMAT).map(|i| i.as_str().to_string()),
    }
}

/// D30 §11.1 — class `short_name`s must be valid Lean identifiers and
/// start with a capital letter (Lean's `structure` requires this).
fn validate_class_identifier(iri: &Iri, name: &str) -> Result<(), MirrorGeneratorError> {
    if name.is_empty() {
        return Err(MirrorGeneratorError::UnrepresentableClass {
            class_iri: iri.as_str().to_string(),
            language: LANGUAGE_LEAN.to_string(),
            reason: "class `short_name` is empty".to_string(),
        });
    }
    let first = name.chars().next().unwrap();
    if !first.is_ascii_uppercase() {
        return Err(MirrorGeneratorError::UnrepresentableClass {
            class_iri: iri.as_str().to_string(),
            language: LANGUAGE_LEAN.to_string(),
            reason: format!(
                "class `short_name` `{name}` must start with an ASCII capital letter (Lean structure names)"
            ),
        });
    }
    validate_lean_identifier_chars(iri, name)
}

/// D30 §11.1 — property `short_name`s must be valid Lean identifiers
/// and start with a lowercase letter (Lean field names).
fn validate_property_identifier(
    class_iri: &Iri,
    prop_iri: &Iri,
    name: &str,
) -> Result<(), MirrorGeneratorError> {
    if name.is_empty() {
        return Err(MirrorGeneratorError::UnrepresentableClass {
            class_iri: class_iri.as_str().to_string(),
            language: LANGUAGE_LEAN.to_string(),
            reason: format!("property `{}`: `short_name` is empty", prop_iri.as_str()),
        });
    }
    let first = name.chars().next().unwrap();
    if !(first.is_ascii_lowercase() || first == '_') {
        return Err(MirrorGeneratorError::UnrepresentableClass {
            class_iri: class_iri.as_str().to_string(),
            language: LANGUAGE_LEAN.to_string(),
            reason: format!(
                "property `{}`: `short_name` `{name}` must start with an ASCII lowercase letter or underscore (Lean field names)",
                prop_iri.as_str()
            ),
        });
    }
    validate_lean_identifier_chars(class_iri, name)
}

fn validate_lean_identifier_chars(iri: &Iri, name: &str) -> Result<(), MirrorGeneratorError> {
    for ch in name.chars() {
        if !(ch.is_ascii_alphanumeric() || ch == '_') {
            return Err(MirrorGeneratorError::UnrepresentableClass {
                class_iri: iri.as_str().to_string(),
                language: LANGUAGE_LEAN.to_string(),
                reason: format!(
                    "identifier `{name}` contains non-alphanumeric character `{ch}` (Lean identifier rule: ASCII alphanumeric + underscore only)"
                ),
            });
        }
    }
    Ok(())
}

/// D30 §11.1 — the transitive field surface (own + inherited via
/// `subclass_of`) must have unique `short_name`s. Walks the
/// parent chain once per leaf class and checks against the leaf's
/// own surface.
fn validate_unique_inherited_fields(
    decls: &BTreeMap<Iri, ClassDecl>,
    iri: &Iri,
) -> Result<(), MirrorGeneratorError> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    collect_field_names(decls, iri, &mut seen, &mut BTreeSet::new(), iri)?;
    Ok(())
}

fn collect_field_names(
    decls: &BTreeMap<Iri, ClassDecl>,
    iri: &Iri,
    seen: &mut BTreeSet<String>,
    visiting: &mut BTreeSet<Iri>,
    root: &Iri,
) -> Result<(), MirrorGeneratorError> {
    if !visiting.insert(iri.clone()) {
        // Subclass cycle — caught here for the integrity-check pass,
        // re-surfaced as UnrepresentableClass naming the root.
        return Err(MirrorGeneratorError::UnrepresentableClass {
            class_iri: root.as_str().to_string(),
            language: LANGUAGE_LEAN.to_string(),
            reason: format!(
                "subclass_of cycle reached via `{}`; D30 §11.1 forbids cyclic class hierarchies",
                iri.as_str()
            ),
        });
    }
    let Some(decl) = decls.get(iri) else {
        // Parent not in the closure decls — walk_closure should have
        // included it; treat as unknown to keep the surface uniform.
        return Err(MirrorGeneratorError::UnknownClass(iri.as_str().to_string()));
    };
    // Recurse into parents first so the inherited-fields-first
    // ordering (D30 §5) matches the dedupe order.
    for parent in &decl.parents {
        collect_field_names(decls, parent, seen, visiting, root)?;
    }
    for prop in decl.requires.iter().chain(decl.recommends.iter()) {
        if !seen.insert(prop.short_name.clone()) {
            return Err(MirrorGeneratorError::UnrepresentableClass {
                class_iri: root.as_str().to_string(),
                language: LANGUAGE_LEAN.to_string(),
                reason: format!(
                    "transitive field surface has duplicate `short_name` `{}` (own or inherited from `{}`)",
                    prop.short_name,
                    iri.as_str()
                ),
            });
        }
    }
    visiting.remove(iri);
    Ok(())
}

// ---------------------------------------------------------------------------
// Topological sort (D30 §3.3)
// ---------------------------------------------------------------------------

/// Order classes so every structure's referenced class types are
/// declared earlier in the emitted module. Lean v1 doesn't support
/// mutual `structure` (D30 §11.2 planned for v2), so a cycle in the
/// field-reference graph produces `UnrepresentableClass`.
///
/// The visitor is depth-first with three-state marks (unmarked /
/// in-progress / done). Iteration order over the input `BTreeMap`
/// is sorted by IRI — gives determinism even when two classes are
/// equally "ready" to emit.
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
                return Err(MirrorGeneratorError::UnrepresentableClass {
                    class_iri: iri.as_str().to_string(),
                    language: LANGUAGE_LEAN.to_string(),
                    reason: "class participates in a cycle of resource-typed property references; \
                             Lean v1 (D30 §3.3) does not emit mutually-recursive structures"
                        .to_string(),
                });
            }
            None => {}
        }
        marks.insert(iri.clone(), Mark::InProgress);
        if let Some(decl) = decls.get(iri) {
            // Walk every class the structure's fields depend on,
            // including parents (Lean's `extends` requires the parent
            // to be declared earlier) and field types.
            for parent in &decl.parents {
                visit(parent, decls, marks, order)?;
            }
            for prop in decl.requires.iter().chain(decl.recommends.iter()) {
                for class_ref in prop.lean_type.class_refs() {
                    visit(class_ref, decls, marks, order)?;
                }
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

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn read_string_property<'a>(r: &'a Resource, prop_iri: &str) -> Option<&'a str> {
    let iri = Iri::parse(prop_iri).expect("static property IRI is well-formed");
    r.get(&iri).and_then(Value::as_str)
}

fn iri_array(r: &Resource, prop_iri: &str) -> Vec<Iri> {
    let iri = Iri::parse(prop_iri).expect("static property IRI is well-formed");
    let Some(value) = r.get(&iri) else {
        return Vec::new();
    };
    value.as_iri_array()
}

fn resource_iri_value(r: &Resource, prop_iri: &str) -> Option<Iri> {
    let iri = Iri::parse(prop_iri).expect("static property IRI is well-formed");
    let value = r.get(&iri)?;
    value.as_iri()
}

fn numeric_value(r: &Resource, prop_iri: &str) -> Option<f64> {
    let iri = Iri::parse(prop_iri).expect("static property IRI is well-formed");
    let value = r.get(&iri)?;
    value
        .as_float()
        .or_else(|| value.as_integer().map(|n| n as f64))
}

fn integer_value(r: &Resource, prop_iri: &str) -> Option<i64> {
    let iri = Iri::parse(prop_iri).expect("static property IRI is well-formed");
    r.get(&iri)?.as_integer()
}

/// Collect every class IRI referenced by a property definition's
/// `class_types`. Closure-walk-only — doesn't validate the shape
/// (`resolve_class_types` does that later with full context).
fn property_class_references(prop_def: &Resource) -> Vec<Iri> {
    iri_array(prop_def, PROP_CLASS_TYPES)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eigenius_kernel::ontology::resource::Resource;
    use eigenius_runtime_substrate::chain::ChainAccessor;
    use std::collections::HashMap;

    // ─── Synthetic chain ────────────────────────────────────────────

    /// Tiny in-memory chain for resolution tests. `resolve` returns
    /// whichever resource the test inserted; ancestor / class-unchanged
    /// helpers default to permissive answers (we don't exercise them).
    struct InMemoryChain {
        resources: HashMap<Iri, Resource>,
    }

    impl InMemoryChain {
        fn new() -> Self {
            Self {
                resources: HashMap::new(),
            }
        }
        fn insert(&mut self, r: Resource) {
            self.resources
                .insert(r.id().expect("test resources must carry an IRI").clone(), r);
        }
    }

    impl ChainAccessor for InMemoryChain {
        fn resolve(&self, _claim_layer: &Iri, target: &Iri) -> Option<Resource> {
            self.resources.get(target).cloned()
        }
        fn is_ancestor_or_equal(&self, _: &Iri, _: &Iri) -> bool {
            true
        }
        fn class_unchanged_between(&self, _: &Iri, _: &Iri, _: &Iri) -> bool {
            true
        }
    }

    fn iri(s: &str) -> Iri {
        Iri::parse(s).expect("test IRI is well-formed")
    }

    /// Build a class resource with `short_name`, optional parents,
    /// and required/recommended property IRIs.
    fn class_resource(
        iri_str: &str,
        short_name: &str,
        parents: &[&str],
        requires: &[&str],
        recommends: &[&str],
    ) -> Resource {
        let mut r = Resource::new(iri(iri_str));
        r.set(iri(PROP_SHORT_NAME), Value::String(short_name.to_string()));
        if !parents.is_empty() {
            r.set(
                iri(PROP_SUBCLASS_OF),
                Value::Array(parents.iter().map(|p| Value::ResourceRef(iri(p))).collect()),
            );
        }
        if !requires.is_empty() {
            r.set(
                iri(PROP_REQUIRES),
                Value::Array(
                    requires
                        .iter()
                        .map(|p| Value::ResourceRef(iri(p)))
                        .collect(),
                ),
            );
        }
        if !recommends.is_empty() {
            r.set(
                iri(PROP_RECOMMENDS),
                Value::Array(
                    recommends
                        .iter()
                        .map(|p| Value::ResourceRef(iri(p)))
                        .collect(),
                ),
            );
        }
        r
    }

    /// Build a property resource with `short_name` + `data_type` and
    /// optional `class_types` / `element_type`.
    fn property_resource(
        iri_str: &str,
        short_name: &str,
        data_type: &str,
        class_types: &[&str],
        element_type: Option<&str>,
    ) -> Resource {
        let mut r = Resource::new(iri(iri_str));
        r.set(iri(PROP_SHORT_NAME), Value::String(short_name.to_string()));
        r.set(iri(PROP_DATA_TYPE), Value::ResourceRef(iri(data_type)));
        if !class_types.is_empty() {
            r.set(
                iri(PROP_CLASS_TYPES),
                Value::Array(
                    class_types
                        .iter()
                        .map(|c| Value::ResourceRef(iri(c)))
                        .collect(),
                ),
            );
        }
        if let Some(et) = element_type {
            r.set(iri(PROP_ELEMENT_TYPE), Value::ResourceRef(iri(et)));
        }
        r
    }

    // ─── walk_closure ────────────────────────────────────────────────

    #[test]
    fn walk_closure_includes_seed_only_when_no_references() {
        let mut chain = InMemoryChain::new();
        chain.insert(class_resource("urn:test:A", "A", &[], &[], &[]));
        let layer = iri("urn:test:layer");
        let seed = vec![iri("urn:test:A")];
        let req = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };
        let closure = walk_closure(&req).expect("walk");
        assert_eq!(closure.classes.len(), 1);
        assert!(closure.classes.contains(&iri("urn:test:A")));
    }

    #[test]
    fn walk_closure_follows_class_types_into_referenced_class() {
        let mut chain = InMemoryChain::new();
        chain.insert(class_resource(
            "urn:test:A",
            "A",
            &[],
            &["urn:test:p_ref"],
            &[],
        ));
        chain.insert(property_resource(
            "urn:test:p_ref",
            "ref",
            CORE_RESOURCE,
            &["urn:test:B"],
            None,
        ));
        chain.insert(class_resource("urn:test:B", "B", &[], &[], &[]));

        let layer = iri("urn:test:layer");
        let seed = vec![iri("urn:test:A")];
        let req = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };
        let closure = walk_closure(&req).expect("walk");
        assert!(closure.classes.contains(&iri("urn:test:A")));
        assert!(
            closure.classes.contains(&iri("urn:test:B")),
            "class_types reference must pull B into closure"
        );
    }

    #[test]
    fn walk_closure_follows_subclass_of_transitively() {
        let mut chain = InMemoryChain::new();
        // A : B : C
        chain.insert(class_resource("urn:test:A", "A", &["urn:test:B"], &[], &[]));
        chain.insert(class_resource("urn:test:B", "B", &["urn:test:C"], &[], &[]));
        chain.insert(class_resource("urn:test:C", "C", &[], &[], &[]));

        let layer = iri("urn:test:layer");
        let seed = vec![iri("urn:test:A")];
        let req = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };
        let closure = walk_closure(&req).expect("walk");
        assert_eq!(
            closure.classes.len(),
            3,
            "transitive subclass_of must pull both ancestors in"
        );
    }

    #[test]
    fn walk_closure_errors_on_unresolvable_seed() {
        let chain = InMemoryChain::new();
        let layer = iri("urn:test:layer");
        let seed = vec![iri("urn:test:missing")];
        let req = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };
        match walk_closure(&req).expect_err("missing class must error") {
            MirrorGeneratorError::UnknownClass(s) => assert_eq!(s, "urn:test:missing"),
            other => panic!("expected UnknownClass, got {other:?}"),
        }
    }

    // ─── resolve_class_declarations + property typing ───────────────

    #[test]
    fn resolve_typed_property_classifies_primitives() {
        let mut chain = InMemoryChain::new();
        chain.insert(class_resource(
            "urn:test:Person",
            "Person",
            &[],
            &["urn:test:name", "urn:test:age", "urn:test:tags"],
            &[],
        ));
        chain.insert(property_resource(
            "urn:test:name",
            "name",
            CORE_STRING,
            &[],
            None,
        ));
        chain.insert(property_resource(
            "urn:test:age",
            "age",
            CORE_INTEGER,
            &[],
            None,
        ));
        chain.insert(property_resource(
            "urn:test:tags",
            "tags",
            CORE_VALUE_ARRAY,
            &[],
            Some(CORE_STRING),
        ));

        let layer = iri("urn:test:layer");
        let seed = vec![iri("urn:test:Person")];
        let req = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };
        let closure = walk_closure(&req).expect("walk");
        let decls = resolve_class_declarations(&req, &closure).expect("resolve");
        let person = decls.get(&iri("urn:test:Person")).expect("Person resolved");
        assert_eq!(person.short_name, "Person");
        assert_eq!(person.requires.len(), 3);
        assert_eq!(person.requires[0].lean_type, LeanType::String);
        assert_eq!(person.requires[1].lean_type, LeanType::Int);
        assert_eq!(
            person.requires[2].lean_type,
            LeanType::ListPrimitive(Box::new(LeanType::String))
        );
    }

    #[test]
    fn resolve_typed_property_classifies_resource_singleton_and_union() {
        let mut chain = InMemoryChain::new();
        chain.insert(class_resource(
            "urn:test:Doc",
            "Doc",
            &[],
            &["urn:test:author", "urn:test:contributor"],
            &[],
        ));
        // Singleton class_types → ClassRef.
        chain.insert(property_resource(
            "urn:test:author",
            "author",
            CORE_RESOURCE,
            &["urn:test:Person"],
            None,
        ));
        // Multi class_types → Union (sorted).
        chain.insert(property_resource(
            "urn:test:contributor",
            "contributor",
            CORE_RESOURCE_ARRAY,
            &["urn:test:Person", "urn:test:Bot"],
            None,
        ));
        chain.insert(class_resource("urn:test:Person", "Person", &[], &[], &[]));
        chain.insert(class_resource("urn:test:Bot", "Bot", &[], &[], &[]));

        let layer = iri("urn:test:layer");
        let seed = vec![iri("urn:test:Doc")];
        let req = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };
        let closure = walk_closure(&req).expect("walk");
        let decls = resolve_class_declarations(&req, &closure).expect("resolve");
        let doc = decls.get(&iri("urn:test:Doc")).expect("Doc resolved");
        assert_eq!(
            doc.requires[0].lean_type,
            LeanType::ClassRef(iri("urn:test:Person"))
        );
        // Union IRIs must be sorted (canonical for determinism).
        assert_eq!(
            doc.requires[1].lean_type,
            LeanType::ListUnion(vec![iri("urn:test:Bot"), iri("urn:test:Person")])
        );
    }

    #[test]
    fn resolve_rejects_reserved_id_property_name() {
        let mut chain = InMemoryChain::new();
        chain.insert(class_resource(
            "urn:test:Bad",
            "Bad",
            &[],
            &["urn:test:id_prop"],
            &[],
        ));
        chain.insert(property_resource(
            "urn:test:id_prop",
            "_id",
            CORE_STRING,
            &[],
            None,
        ));

        let layer = iri("urn:test:layer");
        let seed = vec![iri("urn:test:Bad")];
        let req = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };
        let closure = walk_closure(&req).expect("walk");
        let err = resolve_class_declarations(&req, &closure)
            .expect_err("reserved _id must fail resolution");
        match err {
            MirrorGeneratorError::UnrepresentableClass { reason, .. } => {
                assert!(
                    reason.contains("_id"),
                    "diagnostic should mention _id: {reason}"
                );
            }
            other => panic!("expected UnrepresentableClass, got {other:?}"),
        }
    }

    #[test]
    fn resolve_rejects_class_name_starting_with_lowercase() {
        let mut chain = InMemoryChain::new();
        chain.insert(class_resource("urn:test:bad", "bad", &[], &[], &[]));
        let layer = iri("urn:test:layer");
        let seed = vec![iri("urn:test:bad")];
        let req = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };
        let closure = walk_closure(&req).expect("walk");
        let err =
            resolve_class_declarations(&req, &closure).expect_err("lowercase class name must fail");
        match err {
            MirrorGeneratorError::UnrepresentableClass { reason, .. } => {
                assert!(reason.contains("capital"), "got: {reason}");
            }
            other => panic!("expected UnrepresentableClass, got {other:?}"),
        }
    }

    #[test]
    fn resolve_rejects_empty_class_types_on_resource_property() {
        let mut chain = InMemoryChain::new();
        chain.insert(class_resource("urn:test:C", "C", &[], &["urn:test:r"], &[]));
        chain.insert(property_resource(
            "urn:test:r",
            "r",
            CORE_RESOURCE,
            &[], // empty class_types — invalid per D30 §4
            None,
        ));
        let layer = iri("urn:test:layer");
        let seed = vec![iri("urn:test:C")];
        let req = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };
        let closure = walk_closure(&req).expect("walk");
        let err =
            resolve_class_declarations(&req, &closure).expect_err("empty class_types must fail");
        match err {
            MirrorGeneratorError::UnrepresentableClass { reason, .. } => {
                assert!(reason.contains("non-empty"), "got: {reason}");
            }
            other => panic!("expected UnrepresentableClass, got {other:?}"),
        }
    }

    #[test]
    fn resolve_rejects_duplicate_inherited_field_name() {
        let mut chain = InMemoryChain::new();
        // Both Parent and Child declare a field named `name`.
        // D30 §11.1: transitive field set must be unique.
        chain.insert(class_resource(
            "urn:test:Parent",
            "Parent",
            &[],
            &["urn:test:parent_name"],
            &[],
        ));
        chain.insert(property_resource(
            "urn:test:parent_name",
            "name",
            CORE_STRING,
            &[],
            None,
        ));
        chain.insert(class_resource(
            "urn:test:Child",
            "Child",
            &["urn:test:Parent"],
            &["urn:test:child_name"],
            &[],
        ));
        chain.insert(property_resource(
            "urn:test:child_name",
            "name", // SAME short_name as parent's → conflict
            CORE_STRING,
            &[],
            None,
        ));
        let layer = iri("urn:test:layer");
        let seed = vec![iri("urn:test:Child")];
        let req = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };
        let closure = walk_closure(&req).expect("walk");
        let err = resolve_class_declarations(&req, &closure)
            .expect_err("duplicate inherited field name must fail");
        match err {
            MirrorGeneratorError::UnrepresentableClass { reason, .. } => {
                assert!(
                    reason.contains("duplicate") && reason.contains("name"),
                    "got: {reason}"
                );
            }
            other => panic!("expected UnrepresentableClass, got {other:?}"),
        }
    }

    // ─── topological_order ──────────────────────────────────────────

    #[test]
    fn topological_order_emits_dependencies_before_dependents() {
        // A has a field of type B; B has a field of type C. Order
        // must be [C, B, A].
        let mut chain = InMemoryChain::new();
        chain.insert(class_resource(
            "urn:test:A",
            "A",
            &[],
            &["urn:test:p_b"],
            &[],
        ));
        chain.insert(property_resource(
            "urn:test:p_b",
            "b",
            CORE_RESOURCE,
            &["urn:test:B"],
            None,
        ));
        chain.insert(class_resource(
            "urn:test:B",
            "B",
            &[],
            &["urn:test:p_c"],
            &[],
        ));
        chain.insert(property_resource(
            "urn:test:p_c",
            "c",
            CORE_RESOURCE,
            &["urn:test:C"],
            None,
        ));
        chain.insert(class_resource("urn:test:C", "C", &[], &[], &[]));
        let layer = iri("urn:test:layer");
        let seed = vec![iri("urn:test:A")];
        let req = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };
        let closure = walk_closure(&req).expect("walk");
        let decls = resolve_class_declarations(&req, &closure).expect("resolve");
        let order = topological_order(&decls).expect("topo");
        assert_eq!(
            order,
            vec![iri("urn:test:C"), iri("urn:test:B"), iri("urn:test:A")]
        );
    }

    #[test]
    fn topological_order_rejects_property_class_types_cycle() {
        // A → B (via prop) → A (via prop) — cycle.
        let mut chain = InMemoryChain::new();
        chain.insert(class_resource(
            "urn:test:A",
            "A",
            &[],
            &["urn:test:p_b"],
            &[],
        ));
        chain.insert(property_resource(
            "urn:test:p_b",
            "b",
            CORE_RESOURCE,
            &["urn:test:B"],
            None,
        ));
        chain.insert(class_resource(
            "urn:test:B",
            "B",
            &[],
            &["urn:test:p_a"],
            &[],
        ));
        chain.insert(property_resource(
            "urn:test:p_a",
            "a",
            CORE_RESOURCE,
            &["urn:test:A"],
            None,
        ));
        let layer = iri("urn:test:layer");
        let seed = vec![iri("urn:test:A")];
        let req = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };
        let closure = walk_closure(&req).expect("walk");
        let decls = resolve_class_declarations(&req, &closure).expect("resolve");
        let err = topological_order(&decls).expect_err("cycle must error");
        match err {
            MirrorGeneratorError::UnrepresentableClass { reason, .. } => {
                assert!(reason.contains("cycle"), "got: {reason}");
            }
            other => panic!("expected UnrepresentableClass, got {other:?}"),
        }
    }

    // ─── union ordering canonicality ─────────────────────────────────

    #[test]
    fn class_types_iris_are_sorted_in_resolved_union() {
        // Input order intentionally reverse-alphabetical; resolved
        // union must come out sorted.
        let mut chain = InMemoryChain::new();
        chain.insert(class_resource(
            "urn:test:Doc",
            "Doc",
            &[],
            &["urn:test:p"],
            &[],
        ));
        chain.insert(property_resource(
            "urn:test:p",
            "p",
            CORE_RESOURCE,
            &["urn:test:Zebra", "urn:test:Apple", "urn:test:Mango"],
            None,
        ));
        chain.insert(class_resource("urn:test:Zebra", "Zebra", &[], &[], &[]));
        chain.insert(class_resource("urn:test:Apple", "Apple", &[], &[], &[]));
        chain.insert(class_resource("urn:test:Mango", "Mango", &[], &[], &[]));

        let layer = iri("urn:test:layer");
        let seed = vec![iri("urn:test:Doc")];
        let req = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };
        let closure = walk_closure(&req).expect("walk");
        let decls = resolve_class_declarations(&req, &closure).expect("resolve");
        let doc = decls.get(&iri("urn:test:Doc")).unwrap();
        match &doc.requires[0].lean_type {
            LeanType::Union(iris) => {
                assert_eq!(
                    iris,
                    &vec![
                        iri("urn:test:Apple"),
                        iri("urn:test:Mango"),
                        iri("urn:test:Zebra"),
                    ]
                );
            }
            other => panic!("expected Union, got {other:?}"),
        }
    }

    // ─── LeanMirrorGenerator trait impl ─────────────────────────────

    use eigenius_runtime_substrate::mirror_generator::{LibraryContent, MirrorGenerator};

    #[test]
    fn generator_identifier_is_eigon_ffi_gen() {
        let g = LeanMirrorGenerator::new();
        assert_eq!(g.generator_identifier(), GENERATOR_ID);
        assert_eq!(g.generator_identifier(), "eigon-ffi-gen");
    }

    #[test]
    fn generator_version_tracks_crate_version() {
        let g = LeanMirrorGenerator::new();
        assert_eq!(g.generator_version(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn generator_content_hash_matches_pinned_shape_and_caches() {
        let g = LeanMirrorGenerator::new();
        let h1 = g.generator_content_hash().to_string();
        assert!(h1.starts_with("sha256:"));
        assert_eq!(h1.len(), "sha256:".len() + 64);
        // Second call returns the cached value (OnceLock).
        let h2 = g.generator_content_hash();
        assert_eq!(h1, h2);
    }

    #[test]
    fn generate_returns_embedded_library_with_four_files() {
        // Smallest non-trivial pipeline run: one class with one
        // primitive field. Exercises closure → resolve → topological
        // sort → assembly + the full file-list shape.
        let mut chain = InMemoryChain::new();
        chain.insert(class_resource(
            "urn:test:Person",
            "Person",
            &[],
            &["urn:test:name"],
            &[],
        ));
        chain.insert(property_resource(
            "urn:test:name",
            "name",
            CORE_STRING,
            &[],
            None,
        ));
        let layer = iri("urn:test:layer");
        let seed = vec![iri("urn:test:Person")];
        let req = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };

        let g = LeanMirrorGenerator::new();
        let out = g.generate(&req).expect("generate");
        assert_eq!(out.mirrored_classes, vec![iri("urn:test:Person")]);
        let LibraryContent::Embedded(files) = &out.library else {
            panic!("expected Embedded library");
        };
        // D30 §2 — the four files in declaration order.
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "lakefile.lean",
                "lean-toolchain",
                "EigeniusFFI/Basic.lean",
                "EigeniusFFI/Mirror.lean",
            ]
        );
        // Spot-check that the Mirror module body actually contains
        // the Person structure + decoder + encoder.
        let mirror = files
            .iter()
            .find(|f| f.path == "EigeniusFFI/Mirror.lean")
            .expect("mirror file");
        let body = std::str::from_utf8(&mirror.content).expect("utf8");
        assert!(body.contains("structure Person where"));
        assert!(body.contains("def decodePerson"));
        assert!(body.contains("def encodePerson"));
        assert!(body.contains("def eigeniusDecoders"));
    }

    #[test]
    fn generate_is_deterministic_across_invocations() {
        // Same chain + seed must produce byte-identical library
        // archives. D30 §10.1 — the load-bearing precondition for
        // content-addressed integrity.
        let mut chain = InMemoryChain::new();
        chain.insert(class_resource(
            "urn:test:Person",
            "Person",
            &[],
            &["urn:test:name"],
            &[],
        ));
        chain.insert(property_resource(
            "urn:test:name",
            "name",
            CORE_STRING,
            &[],
            None,
        ));
        let layer = iri("urn:test:layer");
        let seed = vec![iri("urn:test:Person")];
        let req = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };

        let g = LeanMirrorGenerator::new();
        let a = g.generate(&req).expect("generate a");
        let b = g.generate(&req).expect("generate b");
        assert_eq!(a.mirrored_classes, b.mirrored_classes);
        match (&a.library, &b.library) {
            (LibraryContent::Embedded(fa), LibraryContent::Embedded(fb)) => {
                assert_eq!(fa, fb, "library archives must be byte-identical");
            }
            _ => panic!("expected Embedded libraries"),
        }
    }

    // ─── mirror_to_resource ─────────────────────────────────────────

    #[test]
    fn mirror_to_resource_carries_integrity_chain_properties() {
        // The chain-committed `LeanPackageMirror` must carry every
        // property the substrate's mirror-materialisation expects:
        // language tag, generator identity triple, library content
        // hash + archive, mirrored classes (D30 §10.2).
        use eigenius_kernel::ontology::resource::Value;

        let mut chain = InMemoryChain::new();
        chain.insert(class_resource(
            "urn:test:Person",
            "Person",
            &[],
            &["urn:test:name"],
            &[],
        ));
        chain.insert(property_resource(
            "urn:test:name",
            "name",
            CORE_STRING,
            &[],
            None,
        ));
        let layer = iri("urn:test:layer");
        let seed = vec![iri("urn:test:Person")];
        let req = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };

        let g = LeanMirrorGenerator::new();
        let out = g.generate(&req).expect("generate");
        let resource = mirror_to_resource(&g, &out, &layer, Some("1970-01-01T00:00:00Z"));

        for prop in [
            PROP_IS_A_IRI,
            PROP_SHORT_NAME_IRI,
            PROP_DESCRIPTION_IRI,
            PROP_MIRROR_LANGUAGE,
            PROP_MIRROR_SOURCE_LAYER,
            PROP_MIRROR_GEN_ID,
            PROP_MIRROR_GEN_VERSION,
            PROP_MIRROR_GEN_CONTENT_HASH,
            PROP_MIRROR_LIB_CONTENT_HASH,
            PROP_MIRROR_LIB_CONTENT,
            PROP_MIRRORED_CLASSES,
            PROP_MIRROR_GENERATED_AT,
        ] {
            assert!(
                resource.get(&Iri::parse(prop).unwrap()).is_some(),
                "LeanPackageMirror is missing required property `{prop}`"
            );
        }

        // language tag = "lean".
        assert_eq!(
            resource
                .get(&Iri::parse(PROP_MIRROR_LANGUAGE).unwrap())
                .and_then(Value::as_str),
            Some("lean")
        );
        // short_name pinned to "EigeniusFFI".
        assert_eq!(
            resource
                .get(&Iri::parse(PROP_SHORT_NAME_IRI).unwrap())
                .and_then(Value::as_str),
            Some(TARGET_PACKAGE_NAME)
        );
        // Generator identifier triple matches the impl.
        assert_eq!(
            resource
                .get(&Iri::parse(PROP_MIRROR_GEN_ID).unwrap())
                .and_then(Value::as_str),
            Some(GENERATOR_ID)
        );
        // content_hash is sha256:<64-hex>.
        let h = resource
            .get(&Iri::parse(PROP_MIRROR_LIB_CONTENT_HASH).unwrap())
            .and_then(Value::as_str)
            .unwrap();
        assert!(h.starts_with("sha256:"));
        assert_eq!(h.len(), "sha256:".len() + 64);
    }

    #[test]
    fn mirror_to_resource_id_derives_from_content_hash() {
        // Two byte-identical mirror runs produce identical IRIs —
        // chain dedupe at the commit layer (D30 §10.3).
        let mut chain = InMemoryChain::new();
        chain.insert(class_resource(
            "urn:test:Person",
            "Person",
            &[],
            &["urn:test:name"],
            &[],
        ));
        chain.insert(property_resource(
            "urn:test:name",
            "name",
            CORE_STRING,
            &[],
            None,
        ));
        let layer = iri("urn:test:layer");
        let seed = vec![iri("urn:test:Person")];
        let req = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };
        let g = LeanMirrorGenerator::new();
        let a = g.generate(&req).expect("a");
        let b = g.generate(&req).expect("b");
        let r_a = mirror_to_resource(&g, &a, &layer, None);
        let r_b = mirror_to_resource(&g, &b, &layer, None);
        assert_eq!(r_a.id(), r_b.id());
        let id_str = r_a.id().unwrap().as_str();
        assert!(id_str.starts_with("urn:eigenius:runtime:mirror:lean:"));
    }

    #[test]
    fn mirror_to_resource_library_content_round_trips_through_substrate_decoder() {
        // The library_content JSON must be the shape the
        // substrate's mirror-materialiser decodes — `{"kind": "embedded",
        // "files": [{"path", "content_b64"}]}`. Round-trip the JSON
        // through base64 to confirm the bytes the materialiser
        // would write equal the bytes the generator produced.
        use eigenius_kernel::ontology::resource::Value;

        let mut chain = InMemoryChain::new();
        chain.insert(class_resource(
            "urn:test:Person",
            "Person",
            &[],
            &["urn:test:name"],
            &[],
        ));
        chain.insert(property_resource(
            "urn:test:name",
            "name",
            CORE_STRING,
            &[],
            None,
        ));
        let layer = iri("urn:test:layer");
        let seed = vec![iri("urn:test:Person")];
        let req = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };
        let g = LeanMirrorGenerator::new();
        let out = g.generate(&req).expect("generate");
        let resource = mirror_to_resource(&g, &out, &layer, None);

        let lib_value = resource
            .get(&Iri::parse(PROP_MIRROR_LIB_CONTENT).unwrap())
            .expect("library_content present");
        let lib_json = match lib_value {
            Value::Json(v) => v,
            other => panic!("library_content must be JSON, got {other:?}"),
        };
        assert_eq!(lib_json["kind"], "embedded");
        let files = lib_json["files"].as_array().expect("files array");
        // The four-file package: lakefile, lean-toolchain, Basic, Mirror.
        // Sorted by path → EigeniusFFI/Basic.lean, EigeniusFFI/Mirror.lean,
        // lakefile.lean, lean-toolchain.
        let paths: Vec<&str> = files
            .iter()
            .filter_map(|f| f.get("path").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(
            paths,
            vec![
                "EigeniusFFI/Basic.lean",
                "EigeniusFFI/Mirror.lean",
                "lakefile.lean",
                "lean-toolchain",
            ]
        );
        // base64 of "lakefile.lean"'s content must decode to the
        // assembler's lakefile output bytes. Sanity-check the round-trip.
        let lakefile_entry = files
            .iter()
            .find(|f| f.get("path").and_then(|v| v.as_str()) == Some("lakefile.lean"))
            .expect("lakefile entry");
        let b64 = lakefile_entry["content_b64"].as_str().expect("b64");
        // Round-trip: decode the b64 back to bytes and confirm it
        // includes the pinned package directive.
        let decoded = base64_decode_for_test(b64);
        let decoded_str = std::str::from_utf8(&decoded).expect("utf8");
        assert!(decoded_str.contains("package EigeniusFFI"));
    }

    /// Hand-rolled base64 decoder for tests — symmetric inverse of
    /// the `base64_encode` helper in the module body.
    fn base64_decode_for_test(s: &str) -> Vec<u8> {
        fn v(c: u8) -> Option<u8> {
            match c {
                b'A'..=b'Z' => Some(c - b'A'),
                b'a'..=b'z' => Some(c - b'a' + 26),
                b'0'..=b'9' => Some(c - b'0' + 52),
                b'+' => Some(62),
                b'/' => Some(63),
                _ => None,
            }
        }
        let bytes: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
        let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
        let mut i = 0;
        while i + 4 <= bytes.len() {
            let pad = bytes[i..i + 4].iter().filter(|&&b| b == b'=').count();
            let v0 = v(bytes[i]).unwrap_or(0);
            let v1 = v(bytes[i + 1]).unwrap_or(0);
            let v2 = if bytes[i + 2] == b'=' {
                0
            } else {
                v(bytes[i + 2]).unwrap_or(0)
            };
            let v3 = if bytes[i + 3] == b'=' {
                0
            } else {
                v(bytes[i + 3]).unwrap_or(0)
            };
            let n = ((v0 as u32) << 18) | ((v1 as u32) << 12) | ((v2 as u32) << 6) | (v3 as u32);
            out.push((n >> 16) as u8);
            if pad < 2 {
                out.push((n >> 8) as u8);
            }
            if pad < 1 {
                out.push(n as u8);
            }
            i += 4;
        }
        out
    }

    #[test]
    fn generate_propagates_unknown_class_error() {
        // Seed references a class not in the chain — the trait
        // impl must surface UnknownClass, not panic.
        let chain = InMemoryChain::new();
        let layer = iri("urn:test:layer");
        let seed = vec![iri("urn:test:missing")];
        let req = MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        };
        let g = LeanMirrorGenerator::new();
        match g.generate(&req) {
            Err(MirrorGeneratorError::UnknownClass(s)) => assert_eq!(s, "urn:test:missing"),
            Err(other) => panic!("expected UnknownClass, got {other:?}"),
            Ok(_) => panic!("expected error for missing seed class"),
        }
    }
}
