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

//! `eigenius-r` — production R / Bioconductor language runtime for the
//! Eigenius substrate (D55).
//!
//! Implements [`LanguageRuntime`] over the substrate's
//! [`ServiceSpawner`] abstraction, exactly as `eigenius-julia` does — so
//! the *same* dispatch path runs the R worker as a host subprocess
//! (`LocalServiceSpawner`, dev) or inside a digest-pinned R image
//! (`DockerServiceSpawner`, prod). The reproducibility guarantee
//! (`ImageDigest` pinning, cross-check) is a spawner/`WorkerSpec`
//! configuration, not a separate code path.
//!
//! ## Phase status (D55)
//!
//! - **P2 (this milestone)**: `run_script` (`RunRuntimeScript`) end-to-end
//!   through `ensure_service` / `attach_uds`, dispatching the R source to
//!   the [`eigenius-r-worker`] cdylib via the shared `WorkerRpcClient`.
//!   `call_method` and `build_environment_image` are deferred (P4 / P3).
//! - **P3**: the pinned R OCI image + `DockerServiceSpawner` + cross-check.
//! - **P4**: the S4 mirror + typed `call_method`.
//!
//! [`LanguageRuntime`]: eigenius_runtime_substrate::LanguageRuntime
//! [`ServiceSpawner`]: eigenius_runtime_substrate::spawner::service::ServiceSpawner

pub mod conventions;
pub mod dockerfile;
pub mod runtime;

pub use dockerfile::{r_dockerfile_fragments, RImagePlan};
pub use runtime::{RImageBinding, RLanguageRuntime, DEFAULT_BASE_IMAGE};
