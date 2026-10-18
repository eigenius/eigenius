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

// ── In-image worker asset paths (P3, DockerServiceSpawner) ──────────────
// Where the image-build composer bakes the R worker's assets. The image's
// CMD (`bootstrap_command`) runs `Rscript DRIVER_IN_IMAGE`; the driver
// `dyn.load`s `CDYLIB_IN_IMAGE` (via `ENV_CDYLIB`), and `install_packages`
// runs `renv::restore` against `RENV_LOCK_IN_IMAGE`.

/// Directory the R worker assets (driver, cdylib, renv.lock) are baked at.
pub const WORKER_DIR_IN_IMAGE: &str = "/opt/eigenius/r-worker";
/// In-image path of `EigeniusRWorker.R` (the image CMD runs this).
pub const DRIVER_IN_IMAGE: &str = "/opt/eigenius/r-worker/EigeniusRWorker.R";
/// In-image path of the `eigenius-r-worker` cdylib (env `ENV_CDYLIB`).
pub const CDYLIB_IN_IMAGE: &str = "/opt/eigenius/r-worker/libeigenius_r_worker.so";
/// In-image path of the pinned `renv.lock` (`install_packages` restores it).
pub const RENV_LOCK_IN_IMAGE: &str = "/opt/eigenius/r-worker/renv.lock";
/// Path the substrate composer materialises a `RPackageMirror` archive
/// under (P4).
pub const MIRROR_IN_IMAGE: &str = "/opt/eigenius/mirror";
