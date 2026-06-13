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

//! Shared constants pinning the contract between the Rust side (this
//! crate, the substrate) and the R side (`EigeniusRWorker.R`). Mirrors
//! `eigenius_julia::conventions`.

/// `language_id` for dispatch + the `urn:eigenius:runtime:language` value
/// on `RuntimeScript` / `RuntimeEnvironment` resources this runtime owns.
pub const LANGUAGE: &str = "r";

/// Property IRI carrying the R source string on a `RuntimeScript` — the
/// input to `RunRuntimeScript`. (Language-agnostic runtime IRI, shared
/// with the Julia runtime.)
pub const PROP_SOURCE: &str = "urn:eigenius:runtime:source";

/// Property IRI for the language tag on output resources.
pub const PROP_LANGUAGE: &str = "urn:eigenius:runtime:language";

/// Property IRI under which `run_script` records the worker's output
/// (provisional shape, mirroring the Julia runtime's 19a.1 anchor; the
/// typed Eigon `DerivedResource` output lands with the matrix marshalling
/// in P5).
pub const PROP_SCRIPT_OUTPUT: &str = "urn:eigenius:runtime:script_output";

/// Env var the worker reads for the path to the `eigenius-r-worker`
/// cdylib it `dyn.load`s. The runtime sets it on the `WorkerSpec`.
pub const ENV_CDYLIB: &str = "EIGENIUS_R_WORKER_CDYLIB";
