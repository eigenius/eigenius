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

/** A component (built-in, mock, or remote) was registered. */
export const COMPONENT_REGISTER = "orchestrator.component.register";
/** A component-dispatch RPC arrived from the kernel. */
export const COMPONENT_DISPATCH = "orchestrator.component.dispatch";
/** A `DispatchExternal` RPC arrived from the kernel (D31 §6.2 / 19a.5.c). */
export const EXTERNAL_DISPATCH = "orchestrator.component.external_dispatch";

// --- LLM ---

export const LLM_COMPLETE_TEXT = "orchestrator.llm.complete_text";
export const LLM_COMPLETE_JSON = "orchestrator.llm.complete_json";

// --- Native addons ---

/** Native addon load (presence / absence on startup) — runtime substrate. */
export const ADDON_LOAD = "orchestrator.addon_load";

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
export const KERNEL_PASSTHROUGH_LIST_BRANCHES =
  "orchestrator.kernel.list_branches";
export const KERNEL_PASSTHROUGH_GET_BRANCH = "orchestrator.kernel.get_branch";
export const KERNEL_PASSTHROUGH_CREATE_BRANCH =
  "orchestrator.kernel.create_branch";
export const KERNEL_PASSTHROUGH_DELETE_BRANCH =
  "orchestrator.kernel.delete_branch";
export const KERNEL_PASSTHROUGH_MERGE_BRANCHES =
  "orchestrator.kernel.merge_branches";
export const KERNEL_PASSTHROUGH_PREVIEW_MERGE =
  "orchestrator.kernel.preview_merge";
export const KERNEL_PASSTHROUGH_PREPARE_MERGE =
  "orchestrator.kernel.prepare_merge";
export const KERNEL_PASSTHROUGH_PREVIEW_CASCADE =
  "orchestrator.kernel.preview_cascade";
export const KERNEL_PASSTHROUGH_SUBMIT_RESOLUTION =
  "orchestrator.kernel.submit_resolution";
export const KERNEL_PASSTHROUGH_CONSOLIDATE_CHAIN =
  "orchestrator.kernel.consolidate_chain";
export const KERNEL_PASSTHROUGH_ESTIMATE_CONSOLIDATION =
  "orchestrator.kernel.estimate_consolidation";
export const KERNEL_PASSTHROUGH_LIST_TASKS = "orchestrator.kernel.list_tasks";
export const KERNEL_PASSTHROUGH_GET_TASK_STATUS =
  "orchestrator.kernel.get_task_status";
export const KERNEL_PASSTHROUGH_CANCEL_TASK = "orchestrator.kernel.cancel_task";
export const KERNEL_PASSTHROUGH_CREATE_TAG = "orchestrator.kernel.create_tag";
export const KERNEL_PASSTHROUGH_LIST_TAGS = "orchestrator.kernel.list_tags";
export const KERNEL_PASSTHROUGH_DELETE_TAG = "orchestrator.kernel.delete_tag";
export const KERNEL_PASSTHROUGH_ESTIMATE_GC = "orchestrator.kernel.estimate_gc";
export const KERNEL_PASSTHROUGH_RUN_GC = "orchestrator.kernel.run_gc";
