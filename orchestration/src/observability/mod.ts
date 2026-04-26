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
 * Structured logging for the orchestrator. Mirrors the kernel's
 * `kernel/src/observability/` module by review-time discipline so a
 * single log query works against records emitted from either service.
 *
 * ## Convention
 *
 * Every log call follows the same shape:
 *
 * 1. **`operation`** — first positional arg, a stable dotted name
 *    pulled from `./operation.ts` (e.g. `orchestrator.llm.complete_text`).
 * 2. **A constant message string** — second positional arg. Never
 *    interpolated; everything variable goes through fields.
 * 3. **Field bag** — third (optional) arg, a record with keys from
 *    `./field.ts` (or its mirror `kernel/src/observability/field.rs`).
 *
 * Example:
 *
 * ```ts
 * import * as log from "./observability/mod.ts";
 * import * as op from "./observability/operation.ts";
 * import * as field from "./observability/field.ts";
 *
 * log.info(op.COMPONENT_REGISTER, "registered component", {
 *   [field.COMPONENT_IRI]: iri,
 *   host: "orchestrator",
 * });
 * ```
 *
 * Call `init()` once at process start to install env-driven
 * configuration (`EIGENIUS_LOG_LEVEL`, `EIGENIUS_LOG_FORMAT`).
 */

export { init } from "./init.ts";
export { debug, error, info, type LogLevel, warn } from "./log.ts";
export { type FailMark, withRpcGuard } from "./guard.ts";

export * as field from "./field.ts";
export * as operation from "./operation.ts";
