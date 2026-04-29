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

//! Stable operation-name constants for the structured logging
//! convention (see [`crate::observability`] module docs).
//!
//! Naming: `<crate>.<area>.<verb>` — lowercase, dot-separated. Pick
//! a constant before adding a new log site; if no existing one fits,
//! add a new one here so call sites stay greppable and the vocabulary
//! stays small.

// --- gRPC handlers ---
//
// Fired at entry / exit of each RPC; pair with `field::REQUEST_ID`
// + `field::RPC_METHOD` so a single request threads through any
// sub-events.

pub const RPC_LOAD: &str = "kernel.rpc.load";
pub const RPC_QUERY: &str = "kernel.rpc.query";
pub const RPC_INSPECT: &str = "kernel.rpc.inspect";
pub const RPC_RUN_PROGRAM: &str = "kernel.rpc.run_program";
pub const RPC_RUN_PROGRAM_BY_IRI: &str = "kernel.rpc.run_program_by_iri";
pub const RPC_VALIDATE_PROGRAM: &str = "kernel.rpc.validate_program";
pub const RPC_REFLECT: &str = "kernel.rpc.reflect";
pub const RPC_HEALTH: &str = "kernel.rpc.health";
pub const RPC_FIBER_QUERY: &str = "kernel.rpc.fiber_query";
pub const RPC_DISCOVER_MORPHISMS: &str = "kernel.rpc.discover_morphisms";
pub const RPC_LAYER_TOPOLOGY: &str = "kernel.rpc.layer_topology";
pub const RPC_LIST_INSTITUTIONS: &str = "kernel.rpc.list_institutions";
pub const RPC_GET_SCHEMA: &str = "kernel.rpc.get_schema";
pub const RPC_LIST_TASKS: &str = "kernel.rpc.list_tasks";
pub const RPC_GET_TASK_STATUS: &str = "kernel.rpc.get_task_status";
pub const RPC_CANCEL_TASK: &str = "kernel.rpc.cancel_task";
pub const RPC_CAPABILITY_INSTALL: &str = "kernel.rpc.capability_install";
pub const RPC_CAPABILITY_LIST: &str = "kernel.rpc.capability_list";
pub const RPC_CAPABILITY_REMOVE: &str = "kernel.rpc.capability_remove";
pub const RPC_LIST_BRANCHES: &str = "kernel.rpc.list_branches";
pub const RPC_GET_BRANCH: &str = "kernel.rpc.get_branch";
pub const RPC_CREATE_BRANCH: &str = "kernel.rpc.create_branch";
pub const RPC_DELETE_BRANCH: &str = "kernel.rpc.delete_branch";

// --- Layer ---

pub const LAYER_COMMIT: &str = "kernel.layer.commit";
pub const LAYER_TOPOLOGY: &str = "kernel.layer.topology";

// --- Validation ---

pub const VALIDATE_RESOURCE: &str = "kernel.validate.resource";
pub const VALIDATE_LAYER: &str = "kernel.validate.layer";

// --- EigenQL ---

pub const QUERY_PARSE: &str = "kernel.query.parse";
pub const QUERY_TYPE_CHECK: &str = "kernel.query.type_check";
pub const QUERY_EVALUATE: &str = "kernel.query.evaluate";

// --- ESL compile ---

pub const ESL_COMPILE: &str = "kernel.esl.compile";

// --- NbE / type theory ---

pub const NBE_CHECK: &str = "kernel.nbe.check";
pub const NBE_EVAL: &str = "kernel.nbe.eval";

// --- Programs ---

pub const PROGRAM_RUN: &str = "kernel.program.run";
pub const PROGRAM_TYPE_CHECK: &str = "kernel.program.type_check";

// --- Institutions / capabilities ---

pub const INSTITUTION_REGISTER: &str = "kernel.institution.register";
pub const INSTITUTION_DISPATCH: &str = "kernel.institution.dispatch";
pub const CAPABILITY_INSTALL: &str = "kernel.capability.install";
pub const CAPABILITY_DISPATCH: &str = "kernel.capability.dispatch";
pub const CAPABILITY_REMOVE: &str = "kernel.capability.remove";

// --- Tasks (D21) ---

pub const TASK_START: &str = "kernel.task.start";
pub const TASK_RESUME: &str = "kernel.task.resume";
pub const TASK_CHECKPOINT: &str = "kernel.task.checkpoint";

// --- Server lifecycle ---

pub const SERVER_START: &str = "kernel.server.start";
pub const SERVER_SHUTDOWN: &str = "kernel.server.shutdown";
pub const BOOTSTRAP_LOAD: &str = "kernel.bootstrap.load";
