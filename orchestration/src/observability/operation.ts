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
 * Stable operation-name constants for the structured logging
 * convention (see `./mod.ts`).
 *
 * Naming: `<service>.<area>.<verb>` — lowercase, dot-separated.
 * Pick a constant before adding a new log site; if no existing one
 * fits, add a new one here so call sites stay greppable and the
 * vocabulary stays small.
 *
 * Mirrors `kernel/src/observability/operation.rs`'s `kernel.*`
 * naming. Orchestrator constants begin with `orchestrator.*`.
 */

// --- Server lifecycle ---

export const SERVER_START = "orchestrator.server.start";
export const SERVER_SHUTDOWN = "orchestrator.server.shutdown";

// --- Component / capability ---

/** A component (built-in, mock, WASM, or remote) was registered. */
export const COMPONENT_REGISTER = "orchestrator.component.register";
/** A component-dispatch RPC arrived from the kernel. */
export const COMPONENT_DISPATCH = "orchestrator.component.dispatch";

// --- LLM ---

export const LLM_COMPLETE_TEXT = "orchestrator.llm.complete_text";
export const LLM_COMPLETE_JSON = "orchestrator.llm.complete_json";

// --- WASM ---

/** WASM IO addon load (presence / absence on startup). */
export const WASM_ADDON_LOAD = "orchestrator.wasm.addon_load";
/** A WASM IO component was registered with the orchestrator. */
export const WASM_COMPONENT_REGISTER = "orchestrator.wasm.component_register";
/** A WASM IO component was invoked. */
export const WASM_DISPATCH = "orchestrator.wasm.dispatch";

// --- MCP ---

export const MCP_SERVER_START = "orchestrator.mcp.server_start";
export const MCP_TOOL_INVOKE = "orchestrator.mcp.tool_invoke";

// --- Notebook static-file route ---

export const NOTEBOOK_STATIC_REQUEST = "orchestrator.notebook.static_request";

// --- Notebook RPC service (browser-facing) ---

export const NOTEBOOK_LAYER_TOPOLOGY = "orchestrator.notebook.layer_topology";

// --- EigeniusKernel passthrough (browser-facing proxy of the kernel surface) ---

export const KERNEL_PASSTHROUGH_INSPECT = "orchestrator.kernel.inspect";
export const KERNEL_PASSTHROUGH_QUERY = "orchestrator.kernel.query";
export const KERNEL_PASSTHROUGH_LOAD = "orchestrator.kernel.load";
export const KERNEL_PASSTHROUGH_RUN_PROGRAM_BY_IRI =
  "orchestrator.kernel.run_program_by_iri";
export const KERNEL_PASSTHROUGH_LAYER_TOPOLOGY =
  "orchestrator.kernel.layer_topology";
export const KERNEL_PASSTHROUGH_GET_SCHEMA = "orchestrator.kernel.get_schema";
export const KERNEL_PASSTHROUGH_LIST_INSTITUTIONS =
  "orchestrator.kernel.list_institutions";
export const KERNEL_PASSTHROUGH_HEALTH = "orchestrator.kernel.health";
