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

//! Dockerfile fragments emitted by `JuliaLanguageRuntime`.
//!
//! Composed by the substrate's image-build pipeline (D26 §9.2) into
//! a final Dockerfile. The fragments reference paths from
//! [`crate::conventions`] — keep the two in sync.

use crate::conventions::WORKER_PROJECT_DIR;
use eigenius_runtime_substrate::types::DockerfileFragments;

/// Dockerfile fragments for a Julia env image extending an upstream
/// `julia:1.x-bookworm` (or pinned-digest equivalent) base.
///
/// `install_runtime` is empty because the `julia:` base image already
/// ships the Julia binary. Project deps go in `install_packages`
/// because the composer's section ordering puts that section *after*
/// the language asset COPY (`Project.toml` / `Manifest.toml`); placing
/// the `Pkg.instantiate` call in `install_runtime` would silently be a
/// no-op (the project files don't exist yet).
pub fn julia_dockerfile_fragments() -> DockerfileFragments {
    DockerfileFragments {
        install_runtime: vec![],
        install_packages: vec![format!(
            "RUN JULIA_PKG_PRECOMPILE_AUTO=0 julia --project={WORKER_PROJECT_DIR} \
                 -e 'using Pkg; Pkg.instantiate(); Pkg.precompile()'"
        )],
        install_mirror: vec![],
        bootstrap_command: vec![
            "julia".to_string(),
            format!("--project={WORKER_PROJECT_DIR}"),
            format!("{WORKER_PROJECT_DIR}/src/JuliaWorker.jl"),
        ],
    }
}
