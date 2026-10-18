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

//! `eigenius-lean-runtime` — authoring-side Lean 4 language runtime
//! for the Eigenius substrate. Implements
//! [`LanguageRuntime`](eigenius_runtime_substrate::language_runtime::LanguageRuntime)
//! so the Deno orchestrator (via the napi-rs Rust addon) can dispatch
//! `RuntimeScript` / `RuntimeMethodSignature` resources whose
//! `language = "lean"` to a Lake-driven Lean worker baked into a
//! deterministic OCI image.
//!
//! Mirrors the shape of [`eigenius-julia`](../eigenius-julia/) per
//! D26's substrate boundary; differs in: Dockerfile install commands
//! (`elan` + `lake`, not `juliaup` + `Pkg`), worker source language
//! (Lean, not Julia), and the authoring-specific resource shapes
//! (`LeanProject`, `LeanEnvironment` with `lean_permitted_axioms`).
//!
//! ## This crate vs. the verification side
//!
//! This crate is the **authoring** side: it produces proof bytes
//! (`lean_export`) the user commits as a `LeanProofTerm`. The
//! verification side ([`eigenius-lean`](../eigenius-lean/)) consumes
//! the bytes and runs nanoda in-process inside the kernel binary.
//! Different deployment surface (orchestrator binary vs. kernel
//! binary), different trust posture (substrate-mediated vs.
//! direct-call), different Lean integration (Lake project + `lake
//! exe` vs. vendored nanoda_lib) — per D28 §10.2.
//!
//! ## Phase 20a.5 status
//!
//! - **20a.5a (this milestone)**: Rust crate skeleton +
//!   Dockerfile fragments + ontology classes. `LanguageRuntime`
//!   trait impl returns `RunError::RuntimeError` with a "20a.5b
//!   pending" message — the worker doesn't exist yet, but the
//!   crate compiles and the chain knows about `LeanEnvironment` /
//!   `LeanProject`.
//! - **20a.5b**: Lake worker authored, Dockerfile actually
//!   installs Lean, napi-rs binding + orchestrator main.ts
//!   wiring, end-to-end `lean_export` round-trip against a built
//!   image.

pub mod conventions;
pub mod dockerfile;
pub mod mirror_gen;
pub mod runtime;

pub use dockerfile::{lean_dockerfile_fragments, LeanImagePlan};
pub use runtime::{build_target_constant, build_target_module, LeanLanguageRuntime};
