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

//! Shared constants for the Julia language runtime.
//!
//! These pin the contract between the Rust side (this crate, the
//! substrate, the orchestrator) and the Julia side (`JuliaWorker.jl`,
//! generated mirrors landing in 19a.3, institution handlers landing
//! in 19d–19h). Putting them in one module keeps the two sides from
//! drifting silently — the worker's bootstrap code reads literal
//! string paths, and a mismatch produces a cross-check failure
//! rather than a typed error.

use std::time::Duration;

/// `language_id` for `LanguageRuntime` dispatch and the value of the
/// `urn:eigenius:runtime:language` property on `RuntimeScript` /
/// `RuntimeEnvironment` / `RuntimeMethodSignature` resources this
/// runtime owns.
pub const LANGUAGE: &str = "julia";

/// Property IRI carrying the Julia source string on a `RuntimeScript`
/// resource — the input to `RunRuntimeScript`.
pub const PROP_SOURCE: &str = "urn:eigenius:runtime:source";

/// Property IRI for the language tag on output resources.
pub const PROP_LANGUAGE: &str = "urn:eigenius:runtime:language";

/// Property IRI under which `run_script` records the script's textual
/// output. Provisional shape — 19a.4's `CallRuntimeMethod` work
/// settles the proper output-resource shape (a full `RuntimeInvocation`
/// projection); for 19a.1 this single property is the regression
/// anchor matching what the 18d capstone test asserts on.
pub const PROP_SCRIPT_OUTPUT: &str = "urn:eigenius:runtime:script_output";

/// In-image path where the worker's `Project.toml` / `Manifest.toml` /
/// `src/JuliaWorker.jl` are copied. Bound by the Dockerfile composer
/// (see [`crate::dockerfile`]) and read by the worker's bootstrap.
pub const WORKER_PROJECT_DIR: &str = "/opt/eigenius/julia-worker";

/// Wall-clock deadline for the substrate to connect to the worker's
/// UDS after spawn — covers Julia's cold-start (precompile JIT) plus
/// container startup.
pub const UDS_CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
