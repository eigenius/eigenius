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

// --- Comorphism class (Phase 11d, D10 §6) ---

/// is_a marker for a cross-institution comorphism resource.
pub const COMORPHISM: &str = "urn:eigenius:institution:Comorphism";
/// The source institution IRI on a Comorphism resource.
pub const SOURCE_INSTITUTION: &str = "urn:eigenius:institution:source_institution";
/// The target institution IRI on a Comorphism resource.
pub const TARGET_INSTITUTION: &str = "urn:eigenius:institution:target_institution";
/// The translation procedure IRI on a Comorphism resource — the
/// identifier the source institution's `FiberReasoner::translate`
/// method dispatches on.
pub const TRANSLATION_PROCEDURE: &str = "urn:eigenius:institution:translation_procedure";

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
