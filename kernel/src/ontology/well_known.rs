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
