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

//! `eigenius-julia` — production Julia language runtime for the
//! Eigenius substrate. Implements [`LanguageRuntime`] so a kernel
//! configured to register this crate can dispatch `RuntimeScript` /
//! `RuntimeMethodSignature` resources whose `language = "julia"` to a
//! Julia worker baked into a deterministic OCI image.
//!
//! ## Phase 19a status
//!
//! - **19a.1 (this milestone)**: ports the 18d capstone fixture
//!   (`TestLanguageRuntimeJulia`) into a real production crate.
//!   Per-invocation `DockerSpawner` path; `RunRuntimeScript` works;
//!   `CallRuntimeMethod` returns `Err(NotImplemented)`.
//! - **19a.2**: `ServiceSpawner` warm-pool path replaces per-invocation
//!   spawn; `LocalServiceSpawner` joins as a sibling backend.
//! - **19a.3**: mirror generator (substrate Rust code) walks the
//!   ontology layer, emits Julia struct source, commits
//!   `JuliaPackageMirror`, bakes precompiled mirror packages into the
//!   env image. `JuliaWorker.jl` boots with the mirror modules
//!   `using`-imported; method-IRI registry walks their exports.
//! - **19a.4**: `CallRuntimeMethod` lights up against typed mirror
//!   struct dispatch.
//!
//! [`LanguageRuntime`]: eigenius_runtime_substrate::language_runtime::LanguageRuntime

pub mod conventions;
pub mod dockerfile;
pub mod eigenius_common;
pub mod mirror_gen;
pub mod runtime;

pub use mirror_gen::{mirror_to_resource, JuliaMirrorGenerator};
pub use runtime::JuliaLanguageRuntime;
