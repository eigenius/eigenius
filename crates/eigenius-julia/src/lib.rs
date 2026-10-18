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
//! ## What ships
//!
//! - **One dispatch path.** Every Julia dispatch runs in a Docker
//!   *sibling* container that the orchestrator starts over a
//!   bind-mounted socket. There is no host-Julia path and no
//!   per-invocation spawner path in production: `build_worker_spec`
//!   emits an **empty** `command` so the image's `CMD` is PID 1, and
//!   `LocalServiceSpawner` rejects an empty command outright
//!   ("`WorkerSpec.command` must be non-empty for
//!   `LocalServiceSpawner`"). `LocalServiceSpawner` is therefore not a
//!   usable backend for this crate; it exists for the substrate's own
//!   bash test worker.
//! - **Service, not pool.** `JuliaLanguageRuntime` holds an
//!   `Arc<dyn ServiceSpawner>` and reuses one container per image
//!   digest via `ensure_service`. That is a cache, not the warm pool
//!   D26 specifies: there is no idle timeout, no maximum size and no
//!   health-check eviction. `max_wall_time_ms` and `max_memory_bytes`
//!   are both zero and no seccomp profile is passed.
//! - **Mirror generator.** Substrate-side Rust walks the ontology
//!   layer, emits Julia struct source and commits a
//!   `RuntimePackageMirror`; precompiled mirror packages are baked
//!   into the env image at build time. `JuliaWorker.jl` boots with the
//!   mirror modules `using`-imported and resolves handlers through the
//!   `_eigenius_decoders` / `_eigenius_encoders` registries their
//!   exports carry.
//! - **`RunRuntimeScript` and `CallRuntimeMethod` both dispatch**
//!   against typed mirror structs over the CBOR-over-UDS wire.
//!
//! [`LanguageRuntime`]: eigenius_runtime_substrate::language_runtime::LanguageRuntime

pub mod conventions;
pub mod dockerfile;
pub mod eigenius_common;
pub mod mirror_gen;
pub mod runtime;

pub use mirror_gen::{mirror_to_resource, JuliaMirrorGenerator};
pub use runtime::JuliaLanguageRuntime;
