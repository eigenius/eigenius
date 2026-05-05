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

// --- Classes ---

pub const CLASS: &str = "urn:eigenius:core:Class";
pub const PROPERTY: &str = "urn:eigenius:core:Property";
pub const DATA_TYPE: &str = "urn:eigenius:core:DataType";
pub const FORMAT: &str = "urn:eigenius:core:Format";
pub const ENCODING: &str = "urn:eigenius:core:Encoding";
pub const CONDITIONAL_REQUIREMENT: &str = "urn:eigenius:core:ConditionalRequirement";

// --- Inductive types (Phase 11b, D19) ---

pub const INDUCTIVE_TYPE: &str = "urn:eigenius:core:InductiveType";
pub const INDUCTIVE_CTOR: &str = "urn:eigenius:core:InductiveCtor";
pub const INDUCTIVE_ARG_TYPE: &str = "urn:eigenius:core:InductiveArgType";
pub const INDUCTIVE_PARAM: &str = "urn:eigenius:core:InductiveParam";
pub const CTORS: &str = "urn:eigenius:core:ctors";
pub const TYPE_PARAMS: &str = "urn:eigenius:core:type_params";
pub const CTOR_NAME: &str = "urn:eigenius:core:ctor_name";
pub const ARG_TYPES: &str = "urn:eigenius:core:arg_types";
pub const TYPE_NAME: &str = "urn:eigenius:core:type_name";
pub const TYPE_ARGS: &str = "urn:eigenius:core:type_args";
pub const PARAM_NAME: &str = "urn:eigenius:core:param_name";
pub const PARAM_KIND: &str = "urn:eigenius:core:param_kind";
pub const SET_KIND: &str = "urn:eigenius:core:Set";
/// Sized-type parameter kind (Phase 11b step 15h): inductive/codata
/// parameters typed at `Size` — the sort of size values — resolve to
/// `Exp::SizeSort` in the kernel, enabling bounded-binder-driven
/// termination/productivity checking.
pub const SIZE_KIND: &str = "urn:eigenius:core:Size";

// --- Institution-realisation vocabulary (D14) ---

/// is_a marker for a cross-institution comorphism resource. Under D14
/// the Comorphism class is declared in `institution-ontology.json` and
/// carries `export_format`, `transformation`, `import_format`, and
/// `exact` properties — see [`EXPORT_FORMAT`], [`TRANSFORMATION`],
/// [`IMPORT_FORMAT`], [`EXACT`].
pub const COMORPHISM: &str = "urn:eigenius:institution:Comorphism";

// --- D14 Comorphism triadic shape (s, m, t) ---

/// ExportFormat reference on a Comorphism — the source-side `s`.
pub const EXPORT_FORMAT: &str = "urn:eigenius:institution:export_format";
/// Mini-TT Component IRI implementing the comorphism's middle `m: S → T`.
pub const TRANSFORMATION: &str = "urn:eigenius:institution:transformation";
/// ImportFormat reference on a Comorphism — the target-side `t`.
pub const IMPORT_FORMAT: &str = "urn:eigenius:institution:import_format";
/// Exactness flag on a Comorphism (Diaconescu 2025, Thm. 14.15). Absent
/// or `false` is the safe default; only explicit `true` is an exactness
/// claim.
pub const EXACT: &str = "urn:eigenius:institution:exact";

// --- D14 ExportFormat / ImportFormat ---

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
/// Mini-TT type IRI of an ExportFormat / ImportFormat payload.
pub const PAYLOAD_TYPE: &str = "urn:eigenius:institution:payload_type";
/// Procedure IRI dispatched to the institution's `extract_typed` /
/// `reify` handler.
pub const PROCEDURE: &str = "urn:eigenius:institution:procedure";

// --- D14 QueryClass ---

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

// --- D14 RuntimeKind / DispatchRole / Verdict ---

/// is_a marker for a RuntimeKind resource on an Institution.
pub const RUNTIME_KIND_CLASS: &str = "urn:eigenius:institution:RuntimeKind";
/// `runtime` property on an Institution — IRI of the runtime kind.
pub const RUNTIME: &str = "urn:eigenius:institution:runtime";
/// WASM Component Model runtime.
pub const RUNTIME_WASM: &str = "urn:eigenius:institution:runtimes:wasm";
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
pub const DECLARED_BY: &str = "urn:eigenius:reflection:declared_by";
pub const DERIVATION: &str = "urn:eigenius:reflection:derivation";
pub const EPISTEMIC_STATUS: &str = "urn:eigenius:reflection:epistemic_status";
pub const EPISTEMIC_DERIVED: &str = "urn:eigenius:reflection:epistemic:derived";
