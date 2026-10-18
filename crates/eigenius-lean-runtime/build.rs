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

//! Bring the workspace's pinned Lean toolchain version into the Rust
//! source as a `&'static str`. The single source of truth is
//! `lean/runtime-worker/lean-toolchain` — elan reads it natively when
//! Lake invocations happen locally; this build script reads the same
//! file so the Dockerfile composer (which bakes the toolchain into
//! the env image) and any other Rust-side caller see the identical
//! version, with no chance of drift.
//!
//! Bumping the pinned toolchain is a one-line edit to
//! `lean-toolchain`; the `rerun-if-changed` line below ensures Cargo
//! rebuilds this crate (and re-stamps the const) on the next build.

use std::path::PathBuf;

fn main() {
    // CARGO_MANIFEST_DIR is `<workspace>/crates/eigenius-lean-runtime/`;
    // the lean-toolchain file lives at `<workspace>/lean/runtime-worker/`.
    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .expect("CARGO_MANIFEST_DIR is set by cargo for every build script invocation"),
    );
    let toolchain_path = manifest_dir
        .join("..")
        .join("..")
        .join("lean")
        .join("runtime-worker")
        .join("lean-toolchain");

    println!("cargo:rerun-if-changed={}", toolchain_path.display());

    let raw = std::fs::read_to_string(&toolchain_path).unwrap_or_else(|e| {
        panic!(
            "failed to read Lean toolchain pin at `{}`: {e}",
            toolchain_path.display()
        )
    });
    let version = raw.trim();
    if version.is_empty() {
        panic!(
            "lean-toolchain file at `{}` is empty — expected one line like \
             `leanprover/lean4:v4.29.1`",
            toolchain_path.display()
        );
    }

    println!("cargo:rustc-env=EIGENIUS_LEAN_TOOLCHAIN_VERSION={version}");
}
