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

//! Well-known IRI constants for the Eigenius core ontology.
//!
//! These constants avoid repeated string parsing for frequently used IRIs.
//! They correspond to the resources defined in `ontologies/core/core-ontology.json`.

use crate::ontology::iri::Iri;

/// Parse a well-known constant into an [`Iri`]. Panics if the constant
/// isn't a valid IRI — by construction the strings in this module are.
pub fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("well-known IRI constants must be valid")
}

// --- Namespaces ---

/// The core-ontology IRI prefix. Core is the root layer present on every chain,
/// so its vocabulary acts as an always-available prelude — e.g. EigenQL short-name
/// resolution treats this namespace as implicitly imported (see
/// [`crate::query::resolve`]).
pub const CORE_NAMESPACE: &str = "urn:eigenius:core:";

// --- Classes ---

pub const CLASS: &str = "urn:eigenius:core:Class";
pub const PROPERTY: &str = "urn:eigenius:core:Property";
pub const DATA_TYPE: &str = "urn:eigenius:core:DataType";
pub const FORMAT: &str = "urn:eigenius:core:Format";
pub const ENCODING: &str = "urn:eigenius:core:Encoding";
pub const CONDITIONAL_REQUIREMENT: &str = "urn:eigenius:core:ConditionalRequirement";

// --- Layer reconciliation (D20 §6.1) ---
//
// A `MergeComorphism` is the typed witness for a `Witness`-strategy
// resolution: its `merge_transformation` Component has signature
// `(A, A, Option<A>) -> A` where `A` is the class of the IRI being
// merged. Distinct from the institution-layer `Comorphism` (which
// witnesses cross-institution translation); same triadic typing
// discipline, different application surface.

pub const MERGE_COMORPHISM: &str = "urn:eigenius:core:MergeComorphism";
pub const MERGE_TRANSFORMATION: &str = "urn:eigenius:core:merge_transformation";

// --- D43 §3.1 — Index Resource classes and their property slots ---

/// Class IRI for a `core:TextIndex` Resource — the first-class
/// Resource that declares an inverted-index target plus analyzer
/// configuration for D43 §2.3.
pub const TEXT_INDEX_CLASS: &str = "urn:eigenius:core:TextIndex";

/// Class IRI for a `core:VectorIndex` Resource — the first-class
/// Resource that declares an embedded-vector target plus model,
/// dimensionality, distance metric, strategy, and policy for
/// D43 §2.4 / §5.
pub const VECTOR_INDEX_CLASS: &str = "urn:eigenius:core:VectorIndex";

/// Class IRI for a `core:ValueIndex` Resource — the first-class
/// Resource that declares an EXACT value-index target (D65).
pub const VALUE_INDEX_CLASS: &str = "urn:eigenius:core:ValueIndex";

/// Property IRI for `target_property` — the Property an Index
/// Resource (TextIndex / VectorIndex / ValueIndex) targets for indexing.
/// Value is a resource reference to a `core:Property`.
pub const TARGET_PROPERTY: &str = "urn:eigenius:core:target_property";

/// Property IRI for `text_analyzer` — the analyzer ID for a
/// TextIndex (e.g. `"en-stem-v1"`).
pub const TEXT_ANALYZER: &str = "urn:eigenius:core:text_analyzer";

/// Property IRI for `value_normalizer` — the normalizer Resource a
/// ValueIndex applies to values before exact keying (D65). One of
/// `urn:eigenius:core:normalizers:{identity,lowercase,lowercase_trim}`;
/// default `identity` when omitted.
pub const VALUE_NORMALIZER: &str = "urn:eigenius:core:value_normalizer";

/// Property IRI for `vec_model` — the Embedder Component IRI a
/// VectorIndex uses.
pub const VEC_MODEL: &str = "urn:eigenius:core:vec_model";

/// Property IRI for `vec_dim` — declared output dimensionality.
pub const VEC_DIM: &str = "urn:eigenius:core:vec_dim";

/// Property IRI for `vec_distance` — distance-metric Resource ref
/// (one of `core:distances:cosine | l2 | dot`).
pub const VEC_DISTANCE: &str = "urn:eigenius:core:vec_distance";

/// Property IRI for `vec_strategy` — per-segment strategy Resource
/// ref (one of `core:strategies:flat | hnsw | auto`).
pub const VEC_STRATEGY: &str = "urn:eigenius:core:vec_strategy";

/// Property IRI for `vec_hnsw_m` — HNSW M parameter (optional).
pub const VEC_HNSW_M: &str = "urn:eigenius:core:vec_hnsw_m";

/// Property IRI for `vec_hnsw_ef_construction` — HNSW build-time
/// exploration depth (optional).
pub const VEC_HNSW_EF_CONSTRUCTION: &str = "urn:eigenius:core:vec_hnsw_ef_construction";

/// Property IRI for `vec_embedding_policy` — embedding-policy
/// Resource ref (one of `core:embedding_policies:eager_on_load | lazy_on_query | manual`).
pub const VEC_EMBEDDING_POLICY: &str = "urn:eigenius:core:vec_embedding_policy";
/// The class a `MergeComorphism` is declared for — its `A` in the
/// `(A, A, Option<A>) -> A` transformation signature (D37 §3.3, §6.1).
/// Required on every committed `MergeComorphism` so the witness path
/// can early-reject application to a mismatched conflict class and so
/// the notebook's WitnessEditor can show only applicable comorphisms
/// for the current conflict's class.
pub const MERGE_TARGET_CLASS: &str = "urn:eigenius:core:merge_target_class";

/// The declared Pi-type of a standalone `Lambda` resource (D37 §4.1,
/// §5.1). Carrying the type alongside the term lets the validator
/// commit-time-check the lambda's body against its declared signature
/// rather than deferring the check to apply time. Optional on
/// embedded lambdas inside `program` bodies (where the type is
/// inferred from the surrounding `Pi`), required on top-level Lambda
/// resources.
pub const PROGRAM_TYPE: &str = "urn:eigenius:program:type";

// --- Merge resolution records (D38 §3) ---
//
// One `MergeResolutionRecord` resource is committed alongside the
// resolved bodies in every merge layer, one record per resolved
// conflict. Required slots pin the strategy + conflict id; per-strategy
// optional slots capture the strategy-specific choices. For Witness
// resolutions the comorphism + transformation Lambda are also copied
// into the merge layer at their original IRIs so the record's pointer
// is guaranteed to resolve on the merge layer's own chain (D38 §3.2).

pub const MERGE_RESOLUTION_RECORD: &str = "urn:eigenius:core:MergeResolutionRecord";
pub const MERGE_RECORD_CONFLICT_ID: &str = "urn:eigenius:core:merge_record_conflict_id";
pub const MERGE_RECORD_STRATEGY: &str = "urn:eigenius:core:merge_record_strategy";
pub const MERGE_RECORD_BRANCH_A_SOURCE_LAYER: &str =
    "urn:eigenius:core:merge_record_branch_a_source_layer";
pub const MERGE_RECORD_BRANCH_B_SOURCE_LAYER: &str =
    "urn:eigenius:core:merge_record_branch_b_source_layer";
pub const MERGE_RECORD_ANCESTOR_SOURCE_LAYER: &str =
    "urn:eigenius:core:merge_record_ancestor_source_layer";
pub const MERGE_RECORD_WITNESS: &str = "urn:eigenius:core:merge_record_witness";
pub const MERGE_RECORD_WITNESS_SOURCE_LAYER: &str =
    "urn:eigenius:core:merge_record_witness_source_layer";
pub const MERGE_RECORD_RENAME_SIDE: &str = "urn:eigenius:core:merge_record_rename_side";
pub const MERGE_RECORD_RENAME_FROM_IRI: &str = "urn:eigenius:core:merge_record_rename_from_iri";
pub const MERGE_RECORD_RENAME_TO_IRI: &str = "urn:eigenius:core:merge_record_rename_to_iri";
pub const MERGE_RECORD_QUOTIENT_KIND: &str = "urn:eigenius:core:merge_record_quotient_kind";
pub const MERGE_RECORD_QUOTIENT_WINNER: &str = "urn:eigenius:core:merge_record_quotient_winner";
pub const MERGE_RECORD_RESTRUCTURE_NEW_PARENT: &str =
    "urn:eigenius:core:merge_record_restructure_new_parent";
pub const MERGE_RECORD_RESTRUCTURE_AFFECTED_CLASS: &str =
    "urn:eigenius:core:merge_record_restructure_affected_class";

/// Canonical optional/maybe inductive (Phase 15b step 3, D20 §6.1).
/// Used by `MergeComorphism` to type the optional ancestor argument
/// of a `(A, A, Option<A>) -> A` witness signature.
pub const OPTION: &str = "urn:eigenius:core:Option";

/// Canonical list inductive ([`crate::nbe::term::list_decl`]). A kernel
/// built-in (not a chain resource), so type-expression decoders
/// short-circuit this IRI to the built-in decl — as they do the
/// primitive datatypes — rather than resolving it against the chain.
pub const LIST: &str = "urn:eigenius:core:List";

// --- Inductive types (Phase 11b, D19) ---

pub const INDUCTIVE_TYPE: &str = "urn:eigenius:core:InductiveType";
pub const INDUCTIVE_CTOR: &str = "urn:eigenius:core:InductiveCtor";

/// D52 §12 — smart-constructor macro decl persisted as a chain
/// resource so child-file compiles can re-hydrate it via
/// `compile_against_layer`. The resource carries the macro's name,
/// parameter list, return type, and body — all serialized as JSON
/// blobs of the corresponding AST shapes. Macros emit no chain
/// resources at *call sites* (the call is fully inlined at compile
/// time); this resource is only the declaration's persistent form,
/// not the call expansion.
pub const MACRO: &str = "urn:eigenius:core:Macro";
/// Serialized macro-declaration body (JSON of `ast::MacroDecl`).
pub const MACRO_DECL_JSON: &str = "urn:eigenius:core:macro_decl_json";

/// D39 §4.1 atomic propositional inductive: `Asserts(iri) : Prop` —
/// uniform-parameter, zero-constructor inductive type declared in
/// `Sort(0)`. Different IRIs produce distinct propositions; the only
/// inhabitation paths are institutional dispatch or `eigentt:Axiom`
/// introduction (D46 §10). Used by the D49 witness emitter as the
/// default canonical proposition when a target resource carries no
/// explicit `reflection:canonical_proposition`. The well-known IRI is
/// pinned here so emission and the eventual `JustifiedBy.declared`
/// consumer share one source of truth.
pub const ASSERTS: &str = "urn:eigenius:core:Asserts";
pub const INDUCTIVE_ARG_TYPE: &str = "urn:eigenius:core:InductiveArgType";
pub const INDUCTIVE_PARAM: &str = "urn:eigenius:core:InductiveParam";
pub const CODATA_TYPE: &str = "urn:eigenius:core:CodataType";
pub const CTORS: &str = "urn:eigenius:core:ctors";
pub const TYPE_PARAMS: &str = "urn:eigenius:core:type_params";
pub const CTOR_NAME: &str = "urn:eigenius:core:ctor_name";
pub const ARG_TYPES: &str = "urn:eigenius:core:arg_types";
pub const TYPE_NAME: &str = "urn:eigenius:core:type_name";
pub const TYPE_ARGS: &str = "urn:eigenius:core:type_args";
pub const ARG_NAME: &str = "urn:eigenius:core:arg_name";
pub const PARAM_NAME: &str = "urn:eigenius:core:param_name";
pub const PARAM_KIND: &str = "urn:eigenius:core:param_kind";
pub const SET_KIND: &str = "urn:eigenius:core:Set";
/// Index telescope on an inductive-type resource (eigenius#72 Layer 2 /
/// D48). Array of `InductiveParam` resources, parallel to `type_params`
/// but for the indices that vary per constructor. Absent or empty for
/// non-indexed declarations (the default).
pub const INDICES: &str = "urn:eigenius:core:indices";
/// Result sort of an inductive type former (eigenius#72 Layer 2).
/// String of the form `"Prop"`, `"Set"`, or `"Type:N"` (with N a
/// non-negative integer). Absent defaults to `"Set"`.
pub const RESULT_SORT: &str = "urn:eigenius:core:result_sort";
/// Typed-ctor full Π-telescope encoded via the D47 type-fragment codec
/// (eigenius#72 Layer 2). Present on `InductiveCtor` resources that
/// were authored with the `name : <type-expr>` surface form. When
/// present, the kernel decoder uses this directly and ignores
/// `arg_types`. Required for ctors of indexed inductives (the
/// positional form cannot express conclusion indices).
pub const CTOR_TYPE: &str = "urn:eigenius:core:ctor_type";
/// Sized-type parameter kind (Phase 11b step 15h): inductive/codata
/// parameters typed at `Size` — the sort of size values — resolve to
/// `Exp::SizeSort` in the kernel, enabling bounded-binder-driven
/// termination/productivity checking.
pub const SIZE_KIND: &str = "urn:eigenius:core:Size";

// --- Institution-realisation vocabulary (D14) ---

/// is_a marker for a cross-institution comorphism resource. The Comorphism class is declared in `institution-ontology.json` and
/// carries `export_format`, `transformation`, `import_format`, and
/// `exact` properties — see [`EXPORT_FORMAT`], [`TRANSFORMATION`],
/// [`IMPORT_FORMAT`], [`EXACT`].
pub const COMORPHISM: &str = "urn:eigenius:institution:Comorphism";

// --- Comorphism triadic shape (s, m, t) ---

/// ExportFormat reference on a Comorphism — the source-side `s`.
pub const EXPORT_FORMAT: &str = "urn:eigenius:institution:export_format";
/// EigenTT Component IRI implementing the comorphism's middle `m: S → T`.
pub const TRANSFORMATION: &str = "urn:eigenius:institution:transformation";
/// ImportFormat reference on a Comorphism — the target-side `t`.
pub const IMPORT_FORMAT: &str = "urn:eigenius:institution:import_format";
/// Exactness flag on a Comorphism (Diaconescu 2025, Thm. 14.15). Absent
/// or `false` is the safe default; only explicit `true` is an exactness
/// claim.
pub const EXACT: &str = "urn:eigenius:institution:exact";

// --- ExportFormat / ImportFormat ---

/// is_a marker for an ExportFormat resource — a typed outbound view of
/// a source institution's resource class.
pub const EXPORT_FORMAT_CLASS: &str = "urn:eigenius:institution:ExportFormat";
/// is_a marker for an ImportFormat resource — a typed inbound
/// constructor for a target institution's resource class.
pub const IMPORT_FORMAT_CLASS: &str = "urn:eigenius:institution:ImportFormat";

/// Source class of an ExportFormat — the resource class it extracts from.
pub const FROM_CLASS: &str = "urn:eigenius:institution:from_class";
/// Target class of an ImportFormat — the resource class it constructs.
pub const TO_CLASS: &str = "urn:eigenius:institution:to_class";
/// EigenTT type IRI of an ExportFormat / ImportFormat payload.
pub const PAYLOAD_TYPE: &str = "urn:eigenius:institution:payload_type";
/// Procedure IRI dispatched to the institution's `extract_typed` /
/// `reify` handler.
pub const PROCEDURE: &str = "urn:eigenius:institution:procedure";

// --- QueryClass ---

/// is_a marker for a QueryClass resource — a typed function on resources
/// in the institution's fibre, with one or more dispatch roles.
pub const QUERY_CLASS_CLASS: &str = "urn:eigenius:institution:QueryClass";

/// Input class of a QueryClass — dispatch keys on this IRI.
pub const QUERY_CLASS: &str = "urn:eigenius:institution:query_class";
/// Output class of a QueryClass — must be `Verdict` for AutoOnLoad /
/// Decidable roles.
pub const RESULT_CLASS: &str = "urn:eigenius:institution:result_class";
/// Array of dispatch role IRIs declaring how the kernel routes calls
/// to this QueryClass.
pub const DISPATCH_ROLE: &str = "urn:eigenius:institution:dispatch_role";
/// IRI of the QueryClass implementation — either a Component (the
/// kernel orchestrates extract → component → reify) or an
/// institution-runtime procedure dispatched to the institution's
/// `query` handler.
pub const QUERY_HANDLER: &str = "urn:eigenius:institution:query_handler";

// --- RuntimeKind / DispatchRole / Verdict ---

/// is_a marker for a RuntimeKind resource on an Institution.
pub const RUNTIME_KIND_CLASS: &str = "urn:eigenius:institution:RuntimeKind";
/// `runtime` property on an Institution — IRI of the runtime kind.
pub const RUNTIME: &str = "urn:eigenius:institution:runtime";
/// External service (gRPC, LSP, etc.) runtime.
pub const RUNTIME_EXTERNAL: &str = "urn:eigenius:institution:runtimes:external";
/// In-process Rust runtime (kernel-linked).
pub const RUNTIME_IN_PROCESS: &str = "urn:eigenius:institution:runtimes:in_process";

/// is_a marker for a DispatchRole resource on a QueryClass.
pub const DISPATCH_ROLE_CLASS: &str = "urn:eigenius:institution:DispatchRole";
/// Explicit-invocation dispatch (FIBER / RPC).
pub const DISPATCH_ON_DEMAND: &str = "urn:eigenius:institution:dispatch_roles:on_demand";
/// Auto-on-Load dispatch — fires when a resource of the bound query
/// class enters the chain via Load. Replaces the prior
/// `validate_morphism` mechanism.
pub const DISPATCH_AUTO_ON_LOAD: &str = "urn:eigenius:institution:dispatch_roles:auto_on_load";
/// Decidable dispatch — referenced from `Exp::NativeDecide`. Replaces
/// the prior `decide` mechanism.
pub const DISPATCH_DECIDABLE: &str = "urn:eigenius:institution:dispatch_roles:decidable";

/// `requires_environment` property on an Institution — IRI of a
/// `RuntimeEnvironment` resource the institution dispatches into.
/// Required for institutions whose `runtime` is `external` (D31 §5).
pub const INSTITUTION_REQUIRES_ENVIRONMENT: &str = "urn:eigenius:institution:requires_environment";

/// `image_digest` property on a `RuntimeEnvironment` — the
/// content-addressed worker image (`sha256:...`) the substrate
/// dispatches into.
pub const RUNTIME_IMAGE_DIGEST: &str = "urn:eigenius:runtime:image_digest";

/// `method_name` property on a `RuntimeMethodSignature` — the symbol
/// the worker resolves in `Main` after handler-package `using` import.
pub const RUNTIME_METHOD_NAME: &str = "urn:eigenius:runtime:method_name";

/// `language` property on a `RuntimeEnvironment` — the language
/// identifier (`"julia"`, `"python"`, …) the substrate dispatches
/// against its `LanguageRuntime` registry.
pub const RUNTIME_LANGUAGE: &str = "urn:eigenius:runtime:language";

/// is_a marker for the `Verdict` inductive type — the tri-state
/// outcome of an institution-bound predicate query (D14 §6.1).
pub const VERDICT: &str = "urn:eigenius:institution:Verdict";
/// `Verdict::Holds` constructor name.
pub const VERDICT_HOLDS: &str = "Holds";
/// `Verdict::Fails` constructor name.
pub const VERDICT_FAILS: &str = "Fails";
/// `Verdict::Undecidable` constructor name.
pub const VERDICT_UNDECIDABLE: &str = "Undecidable";

/// Name of a named constructor-argument binder (Phase 11b step 15h).
/// Presence on an `InductiveArgType` resource flags the arg as a
/// Π/SizedPi binder rather than an anonymous positional type.
pub const BINDER_NAME: &str = "urn:eigenius:core:binder_name";

/// Upper bound for a bounded size binder (Phase 11b step 15h).
/// Only meaningful alongside `binder_name` with kind `Size`; carries
/// the rigid size variable or `Inf` the binder is strictly below.
pub const BINDER_BOUND: &str = "urn:eigenius:core:binder_bound";

// --- TypeExpr resource shapes for codata observation types (Phase 11b step 15h.3) ---

/// is_a marker for a non-dependent arrow `A -> B` in a codata
/// observation type.
pub const TYPE_ARROW: &str = "urn:eigenius:core:TypeArrow";
/// is_a marker for a size-binder arrow `{j < i} -> body` or
/// `{j : Kind} -> body` in a codata observation type.
pub const TYPE_BINDER_ARROW: &str = "urn:eigenius:core:TypeBinderArrow";

/// Domain of a `TypeArrow` — embedded TypeExpr resource (or string).
pub const ARROW_DOMAIN: &str = "urn:eigenius:core:arrow_domain";
/// Codomain of a `TypeArrow`.
pub const ARROW_CODOMAIN: &str = "urn:eigenius:core:arrow_codomain";

/// Kind of a size-binder arrow's bound variable. Qualified-name
/// string ("urn:eigenius:core:Size" or bare "Size").
pub const BINDER_KIND: &str = "urn:eigenius:core:binder_kind";
/// Body of a size-binder arrow — embedded TypeExpr resource or
/// string.
pub const BINDER_BODY: &str = "urn:eigenius:core:binder_body";

// --- Properties ---

pub const IS_A: &str = "urn:eigenius:core:is_a";
pub const DESCRIPTION: &str = "urn:eigenius:core:description";
pub const SHORT_NAME: &str = "urn:eigenius:core:short_name";
pub const PARENT_CLASSES: &str = "urn:eigenius:core:subclass_of";
pub const REQUIRES: &str = "urn:eigenius:core:requires";
pub const RECOMMENDS: &str = "urn:eigenius:core:recommends";
pub const DATA_TYPE_PROP: &str = "urn:eigenius:core:data_type";
pub const FORMAT_PROP: &str = "urn:eigenius:core:format";
pub const PATTERN: &str = "urn:eigenius:core:pattern";
pub const DOMAIN: &str = "urn:eigenius:core:domain";
pub const CLASS_TYPES: &str = "urn:eigenius:core:class_types";
pub const ALLOWS_ONLY: &str = "urn:eigenius:core:allows_only";
pub const ELEMENT_TYPE: &str = "urn:eigenius:core:element_type";
pub const CONDITIONAL_REQUIRES: &str = "urn:eigenius:core:conditional_requires";
pub const WHEN_PROPERTY: &str = "urn:eigenius:core:when_property";
pub const HAS_VALUE: &str = "urn:eigenius:core:has_value";
pub const THEN_REQUIRES: &str = "urn:eigenius:core:then_requires";
pub const THEN_RECOMMENDS: &str = "urn:eigenius:core:then_recommends";
pub const MIN_VALUE: &str = "urn:eigenius:core:min_value";
pub const MAX_VALUE: &str = "urn:eigenius:core:max_value";
pub const MIN_LENGTH: &str = "urn:eigenius:core:min_length";
pub const MAX_LENGTH: &str = "urn:eigenius:core:max_length";
pub const CONTENT_TYPE: &str = "urn:eigenius:core:content_type";
pub const CONTENT_ENCODING: &str = "urn:eigenius:core:content_encoding";
pub const SOURCE_IRL: &str = "urn:eigenius:core:source_irl";

// --- DataType IRIs ---

pub const STRING: &str = "urn:eigenius:core:string";
pub const INTEGER: &str = "urn:eigenius:core:integer";
pub const FLOAT: &str = "urn:eigenius:core:float";
pub const BOOLEAN: &str = "urn:eigenius:core:boolean";
pub const RESOURCE: &str = "urn:eigenius:core:resource";
pub const RESOURCE_ARRAY: &str = "urn:eigenius:core:resource_array";
pub const VALUE_ARRAY: &str = "urn:eigenius:core:value_array";
pub const JSON: &str = "urn:eigenius:core:json";
pub const INDUCTIVE: &str = "urn:eigenius:core:inductive";
pub const TEMPLATE: &str = "urn:eigenius:core:template";

// --- Format IRIs ---

pub const FMT_DATE: &str = "urn:eigenius:core:formats:date";
pub const FMT_DATETIME: &str = "urn:eigenius:core:formats:datetime";
pub const FMT_TIME: &str = "urn:eigenius:core:formats:time";
pub const FMT_IRI: &str = "urn:eigenius:core:formats:iri";
pub const FMT_UUID: &str = "urn:eigenius:core:formats:uuid";
pub const FMT_REGEX: &str = "urn:eigenius:core:formats:regex";

// --- Encoding IRIs ---

pub const ENC_BASE64: &str = "urn:eigenius:core:encodings:base64";

// --- Reflection namespace (D6b, Phase 10b) ---

pub const UNIVERSE_LEVEL: &str = "urn:eigenius:reflection:universe_level";
pub const DECLARED_RESOURCE: &str = "urn:eigenius:reflection:DeclaredResource";
pub const DERIVED_RESOURCE: &str = "urn:eigenius:reflection:DerivedResource";
pub const OBSERVED_RESOURCE: &str = "urn:eigenius:reflection:ObservedResource";
pub const VERIFIED_RESOURCE: &str = "urn:eigenius:reflection:VerifiedResource";
/// `reflection:InstitutionEmittedDerivation` — marker subclass of
/// `DerivedResource` for resources the kernel commits as side-effects of
/// AutoOnLoad institution dispatches. The witness emitter walks these
/// directly (no ProgramTrace required) to admit
/// `IsDerivedAs(derivation_iri, canonical_proposition)` per D49 §6.
pub const INSTITUTION_EMITTED_DERIVATION: &str =
    "urn:eigenius:reflection:InstitutionEmittedDerivation";
/// `reflection:from_subject` — the analysis/claim IRI that triggered
/// the emission of an `InstitutionEmittedDerivation`. Bidirectional
/// navigability between an analysis and its derivations.
pub const FROM_SUBJECT: &str = "urn:eigenius:reflection:from_subject";
/// `reflection:runtime_invocation` — back-pointer to the producing
/// `RuntimeInvocation` on an `InstitutionEmittedDerivation`.
pub const RUNTIME_INVOCATION: &str = "urn:eigenius:reflection:runtime_invocation";
pub const DECLARED_BY: &str = "urn:eigenius:reflection:declared_by";
pub const DERIVATION: &str = "urn:eigenius:reflection:derivation";
pub const EPISTEMIC_STATUS: &str = "urn:eigenius:reflection:epistemic_status";
pub const EPISTEMIC_DERIVED: &str = "urn:eigenius:reflection:epistemic:derived";

// --- D49 ChainWitness: Trace event classes + canonical proposition ---
//
// The four Trace classes are the chain-side artifacts whose commits the
// D49 witness emitter projects into the per-Layer witness index. Their
// IRIs are looked up by class name during `build_witness_index`.

/// Resource recording that a resource was declared by a human/agent.
/// Carries `reflection:resource` (target IRI). Per D49 §6, a successful
/// commit emits an `IsDeclaredAs` witness for the target resource.
pub const DECLARATION_TRACE: &str = "urn:eigenius:reflection:DeclarationTrace";
/// Resource recording that a resource was observed from external reality.
/// Carries `reflection:resource` and `reflection:source`. Per D49 §6, a
/// successful commit emits an `IsObservedAs` witness.
pub const OBSERVATION_TRACE: &str = "urn:eigenius:reflection:ObservationTrace";
/// Resource recording a complete program execution. The output resource's
/// IRI is the witness `iri` key; per D49 §6 commit emits an `IsDerivedAs`
/// witness.
pub const PROGRAM_TRACE: &str = "urn:eigenius:reflection:ProgramTrace";
/// Resource recording that a formal proof was attached to a resource. Per
/// D49 §6 + §7, the `IsVerifiedAs` witness is admitted only after the
/// `Lean → Reasoning` comorphism's reify produces a
/// `reasoning:VerifiedPropositionView` whose `canonical_proposition` is
/// the EigenTT form of the proved proposition.
pub const VERIFICATION_TRACE: &str = "urn:eigenius:reflection:VerificationTrace";

/// `reflection:resource` — the target IRI a Trace points at. Common to
/// all four Trace classes (semantically; for `ProgramTrace` the role is
/// played by the output resource's own IRI, not a separate property).
pub const REFLECTION_RESOURCE: &str = "urn:eigenius:reflection:resource";

/// `reflection:canonical_proposition` — the optional `Prop`-typed
/// proposition a resource asserts (per D49 §6). Carries a D47-encoded
/// `eigentt:TypeExpr` payload. Absent value defaults to `Asserts(iri)`
/// at witness-emission time. Type-checked at `Prop` at commit by the
/// validator extension shipped in D49 Phase 5.
pub const CANONICAL_PROPOSITION: &str = "urn:eigenius:reflection:canonical_proposition";

// --- D49 ChainWitness: predicate-type IRIs ---
//
// The four kernel-internal `ChainWitness.IsXxAs : core:iri → Prop → Prop`
// predicate types. ESL has no constructors for their inhabitants; the
// kernel synthesises `Val::ChainWitness` values at `JustifiedBy.*`
// constructor type-check time via the per-Layer witness-index lookup.
// The IRIs are referenced from the `reasoning:JustifiedBy` indexed
// inductive's constructor signatures (D39 §5) and from the witness-
// synthesis hook in `kernel/src/nbe/check.rs` (D49 §5).

pub const CHAIN_WITNESS_IS_DECLARED_AS: &str = "urn:eigenius:reasoning:ChainWitness:IsDeclaredAs";
pub const CHAIN_WITNESS_IS_OBSERVED_AS: &str = "urn:eigenius:reasoning:ChainWitness:IsObservedAs";
pub const CHAIN_WITNESS_IS_DERIVED_AS: &str = "urn:eigenius:reasoning:ChainWitness:IsDerivedAs";
pub const CHAIN_WITNESS_IS_VERIFIED_AS: &str = "urn:eigenius:reasoning:ChainWitness:IsVerifiedAs";

/// Helper: map a class IRI for one of the four `ChainWitness.IsXxAs`
/// predicate types to its `WitnessCategory`, or `None` if the IRI is
/// not a ChainWitness predicate.
pub fn chain_witness_category_for_iri(iri: &str) -> Option<crate::witness::WitnessCategory> {
    use crate::witness::WitnessCategory;
    match iri {
        CHAIN_WITNESS_IS_DECLARED_AS => Some(WitnessCategory::Declared),
        CHAIN_WITNESS_IS_OBSERVED_AS => Some(WitnessCategory::Observed),
        CHAIN_WITNESS_IS_DERIVED_AS => Some(WitnessCategory::Derived),
        CHAIN_WITNESS_IS_VERIFIED_AS => Some(WitnessCategory::Verified),
        _ => None,
    }
}
