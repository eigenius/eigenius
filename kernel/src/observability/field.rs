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

//! Standard field-name constants for the structured logging
//! convention (see [`crate::observability`] module docs).
//!
//! Centralising these lets log-aggregator queries and dashboards be
//! written against a fixed vocabulary, and lets the orchestrator
//! mirror the same names by review-time discipline. Adding a new
//! field is a kernel-and-orchestrator change you should make
//! deliberately — pick a name here, then use the constant at every
//! call site.

/// Identifies *what* is being logged. Always present. Value is one
/// of the [`crate::observability::operation`] constants.
pub const OPERATION: &str = "operation";

// --- Identity ---

/// Hex-encoded `LayerId` (sha-256 of the layer's CBOR). Used by every
/// layer-related event.
pub const LAYER_ID: &str = "layer_id";
/// IRI of a single resource — typically the subject of the operation.
pub const RESOURCE_IRI: &str = "resource_iri";
/// IRI of the program being type-checked, run, or otherwise referenced.
pub const PROGRAM_IRI: &str = "program_iri";
/// IRI of an IO component or capability being dispatched.
pub const COMPONENT_IRI: &str = "component_iri";
/// IRI of the institution being dispatched into.
pub const INSTITUTION_IRI: &str = "institution_iri";
/// IRI of a class — for example the class an instance is being
/// validated against, or a class targeted by a query.
pub const CLASS_IRI: &str = "class_iri";
/// IRI of a property — for example the property a validation rule
/// fired against.
pub const PROPERTY_IRI: &str = "property_iri";

// --- Session / task ---

/// Session identifier scoping a series of related RPCs.
pub const SESSION_ID: &str = "session_id";
/// Per-task identifier (D21).
pub const TASK_ID: &str = "task_id";
/// Per-RPC request correlator. Set on the gRPC handler entry point
/// and propagated through any sub-events fired during the request.
pub const REQUEST_ID: &str = "request_id";

// --- Errors ---

/// Stable category name for the error (e.g. `validation_failed`,
/// `parse_error`, `capability_panic`). Use this rather than the
/// human-readable message as the primary grouping key in dashboards.
pub const ERROR_KIND: &str = "error_kind";
/// Free-form error message. Useful for humans, not for indexing.
pub const ERROR_MESSAGE: &str = "error_message";

// --- Quantities / timings ---

/// Generic count — items processed, rows returned, resources committed.
/// Pair with `OPERATION` to disambiguate.
pub const COUNT: &str = "count";
/// Wall-clock duration in milliseconds.
pub const LATENCY_MS: &str = "latency_ms";
/// Size in bytes — request body, response body, serialized resource.
pub const SIZE_BYTES: &str = "size_bytes";

// --- RPC-shape specifics ---

/// gRPC method name (`Load`, `Query`, `Inspect`, …) for RPC entry/exit
/// events.
pub const RPC_METHOD: &str = "rpc_method";
/// Content type of an RPC payload (`application/eigon+json`,
/// `application/x-esl`, …).
pub const CONTENT_TYPE: &str = "content_type";
