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

/**
 * Standard field-name constants for the structured logging convention
 * (see `./mod.ts` for the convention overview).
 *
 * Names mirror the kernel's `kernel/src/observability/field.rs` so a
 * single log query (`error_kind = "validation_failed"`) works
 * identically against records emitted by either service. When adding
 * a field, update both files.
 */

/** Identifies what is being logged. Always present. Value is one of the `operation` constants. */
export const OPERATION = "operation";

// --- Identity ---

/** IRI of an IO component or capability being dispatched. */
export const COMPONENT_IRI = "component_iri";
/** IRI of a class — for example a target class for a CompleteJson schema. */
export const CLASS_IRI = "class_iri";
/** IRI of the program being type-checked, run, or otherwise referenced. */
export const PROGRAM_IRI = "program_iri";
/** IRI of a single resource — typically the subject of the operation. */
export const RESOURCE_IRI = "resource_iri";

// --- Session / task ---

/** Session identifier scoping a series of related RPCs. */
export const SESSION_ID = "session_id";
/** Per-task identifier (D21). */
export const TASK_ID = "task_id";
/** Per-RPC request correlator. */
export const REQUEST_ID = "request_id";

// --- Errors ---

/**
 * Stable category name for the error (e.g. `dispatch_failed`,
 * `llm_provider_error`, `parse_failed`). Use this rather than the
 * human message as the primary grouping key in dashboards.
 */
export const ERROR_KIND = "error_kind";
/** Free-form error message. Useful for humans, not for indexing. */
export const ERROR_MESSAGE = "error_message";

// --- Quantities / timings ---

/** Generic count — items processed, rows returned, components registered. */
export const COUNT = "count";
/** Wall-clock duration in milliseconds. */
export const LATENCY_MS = "latency_ms";
/** Size in bytes — request body, response body, serialized resource. */
export const SIZE_BYTES = "size_bytes";

// --- RPC-shape specifics ---

/** gRPC method name (e.g. `Dispatch`) for RPC entry/exit events. */
export const RPC_METHOD = "rpc_method";
/** Content type of an RPC payload. */
export const CONTENT_TYPE = "content_type";

// --- LLM specifics ---

/** Provider name (e.g. `anthropic`). */
export const PROVIDER = "provider";
/** Model name (e.g. `claude-sonnet-4-5`). */
export const MODEL = "model";
/** Tokens consumed by the call. */
export const TOKEN_COUNT = "token_count";
