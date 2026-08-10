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
pub const RPC_MERGE_BRANCHES: &str = "kernel.rpc.merge_branches";
pub const RPC_PREVIEW_MERGE: &str = "kernel.rpc.preview_merge";
pub const RPC_SUBMIT_RESOLUTION: &str = "kernel.rpc.submit_resolution";
pub const RPC_PREVIEW_CASCADE: &str = "kernel.rpc.preview_cascade";
pub const RPC_PREPARE_MERGE: &str = "kernel.rpc.prepare_merge";
pub const RPC_CONSOLIDATE_CHAIN: &str = "kernel.rpc.consolidate_chain";
pub const RPC_ESTIMATE_CONSOLIDATION: &str = "kernel.rpc.estimate_consolidation";
pub const RPC_CREATE_TAG: &str = "kernel.rpc.create_tag";
pub const RPC_LIST_TAGS: &str = "kernel.rpc.list_tags";
pub const RPC_DELETE_TAG: &str = "kernel.rpc.delete_tag";
pub const RPC_ESTIMATE_GC: &str = "kernel.rpc.estimate_gc";
pub const RPC_RUN_GC: &str = "kernel.rpc.run_gc";

// --- Layer ---

pub const LAYER_COMMIT: &str = "kernel.layer.commit";
/// D66 slice 1 — a stored `canonical_proposition` could not be decoded, so no `ChainWitness` can
/// be admitted for it. Emitted at the lookup site, which holds the specific resource.
pub const WITNESS_DECODE: &str = "kernel.layer.witness_decode";
pub const LAYER_TOPOLOGY: &str = "kernel.layer.topology";

// --- Commit pipeline (D41 §12) ---
//
// Phase events: `tracing::info!` with the corresponding constant.
// `COMMIT_CASCADE` fires once per cascade fixpoint iteration from
// inside `retroactive_with_cascade`.
//
// Hook + run spans: `tracing::info_span!` opened by the pipeline /
// orchestrator around the hook list or the entire drain.

pub const COMMIT_BUILD: &str = "kernel.commit.build";
pub const COMMIT_STRUCTURAL_VALIDATE: &str = "kernel.commit.structural_validate";
pub const COMMIT_RETROACTIVE: &str = "kernel.commit.retroactive";
pub const COMMIT_CASCADE: &str = "kernel.commit.cascade";
pub const COMMIT_AUTOONLOAD: &str = "kernel.commit.autoonload";
pub const COMMIT_PERSIST: &str = "kernel.commit.persist";
pub const COMMIT_DID_PERSIST: &str = "kernel.commit.did_persist";
pub const COMMIT_DID_DRAIN: &str = "kernel.commit.did_drain";
pub const COMMIT_PIPELINE_RUN: &str = "kernel.commit.pipeline_run";
pub const COMMIT_ORCHESTRATOR_RUN: &str = "kernel.commit.orchestrator_run";

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

// --- GC phases ---
//
// One event per phase per `collect` / `estimate` call. Lets dashboards
// distinguish "topology load took 200 ms" from "mark phase took 200 ms"
// from "sweep phase took 200 ms" without ad-hoc parsing.
//
// `GC_LOAD_TOPOLOGY` carries the topology size as `field::COUNT`
// (layer count) and `field::SIZE_BYTES` (sum of `LayerHandle.byte_size`
// — proxy for the topology's on-disk cost; in-memory cost is
// proportional). Use these to track the tripwires documented at
// [`crate::gc`]: when layer count crosses ~500k or `load_topology`
// p99 exceeds ~100 ms, time to revisit consolidation and (eventually)
// streaming topology iteration.

pub const GC_LOAD_TOPOLOGY: &str = "kernel.gc.load_topology";
pub const GC_MARK: &str = "kernel.gc.mark";
pub const GC_SWEEP: &str = "kernel.gc.sweep";

pub const RPC_PARSE_SENTENCE: &str = "kernel.rpc.parse_sentence";
