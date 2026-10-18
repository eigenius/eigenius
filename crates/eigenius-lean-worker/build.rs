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

//! Build script that compiles [`c/lean_bridge.c`] against the
//! installed Lean toolchain's `lean.h`.
//!
//! Lean's runtime header is heavy on `static inline` (~560 in
//! `lean.h`), so direct `extern "C"` bindings from Rust can't link
//! them. The C wrapper re-exposes each inline we need as a proper
//! linkable symbol; rustc then extern-declares those (see
//! [`crate::lean_sys`]).
//!
//! ## Discovering `lean.h`
//!
//! Lookup order:
//! 1. `EIGENIUS_LEAN_INCLUDE_DIR` env var — explicit override the
//!    build can pin in CI.
//! 2. `LEAN_SYSROOT` env var (set by Lake during builds) +
//!    `/include`.
//! 3. `lean --print-prefix` — works whenever `lean` is on `PATH`.
//!    The typical local-dev path: `elan` shims point at the
//!    active toolchain, and the active toolchain ships `lean.h`
//!    under `<prefix>/include/`.
//!
//! No fallback to system paths: `lean.h` is rarely installed
//! globally. If none of the above resolves we error out with a
//! clear remediation message.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    // docs.rs has no Lean toolchain. Setting `DOCS_RS=1` is its
    // convention for letting build scripts short-circuit cleanly; we
    // skip the C bridge compile entirely so rustdoc can still build the
    // crate's documentation from source. The published crate's
    // `[package.metadata.docs.rs]` doesn't need any extra flags — this
    // env check is the whole short-circuit.
    if env::var_os("DOCS_RS").is_some() {
        println!("cargo:warning=eigenius-lean-worker: DOCS_RS detected, skipping C bridge build");
        return;
    }

    let include_dir = match discover_lean_include_dir() {
        Ok(p) => p,
        Err(msg) => {
            panic!(
                "cannot locate Lean's include directory: {msg}\n\
                 Set EIGENIUS_LEAN_INCLUDE_DIR to a directory containing `lean/lean.h`, \
                 or ensure `lean` is on PATH so `lean --print-prefix` resolves."
            );
        }
    };

    // Tell cargo to re-run if the bridge source or the discovery
    // envs change. We also rebuild if the path itself moves —
    // `EIGENIUS_LEAN_INCLUDE_DIR` lets CI pin a specific toolchain.
    println!("cargo:rerun-if-changed=c/lean_bridge.c");
    println!("cargo:rerun-if-changed=c/lean_bridge.h");
    println!("cargo:rerun-if-env-changed=EIGENIUS_LEAN_INCLUDE_DIR");
    println!("cargo:rerun-if-env-changed=LEAN_SYSROOT");
    println!("cargo:rerun-if-env-changed=DOCS_RS");

    cc::Build::new()
        .file("c/lean_bridge.c")
        .include(&include_dir)
        // lean.h compiles cleanly under -std=c11.
        .flag_if_supported("-std=c11")
        // Lean's headers warn about a few things we don't control;
        // suppress to keep the build output focused on our errors.
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-unused-function")
        .compile("eigenius_lean_bridge");
}

fn discover_lean_include_dir() -> Result<PathBuf, String> {
    // 1. Explicit override.
    if let Ok(path) = env::var("EIGENIUS_LEAN_INCLUDE_DIR") {
        let p = PathBuf::from(path);
        if has_lean_h(&p) {
            return Ok(p);
        }
        return Err(format!(
            "EIGENIUS_LEAN_INCLUDE_DIR={} does not contain lean/lean.h",
            p.display()
        ));
    }

    // 2. LEAN_SYSROOT (Lake convention).
    if let Ok(sysroot) = env::var("LEAN_SYSROOT") {
        let p = PathBuf::from(sysroot).join("include");
        if has_lean_h(&p) {
            return Ok(p);
        }
    }

    // 3. `lean --print-prefix`.
    let output = Command::new("lean")
        .arg("--print-prefix")
        .output()
        .map_err(|e| format!("`lean --print-prefix` failed: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "`lean --print-prefix` exited non-zero: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let prefix = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let p = PathBuf::from(prefix).join("include");
    if has_lean_h(&p) {
        return Ok(p);
    }
    Err(format!(
        "`lean --print-prefix` returned `{}`, but `lean/lean.h` is not under `{}`",
        p.parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        p.display()
    ))
}

fn has_lean_h(include_dir: &std::path::Path) -> bool {
    include_dir.join("lean").join("lean.h").exists()
}
