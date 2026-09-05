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

//! Emit the `nanoda_lib` revision this build actually links, as `EIGENIUS_NANODA_REV`.
//!
//! D87 §5 makes a Lean verdict re-decidable by pinning its inputs, and the checker's identity is
//! one of the two that were recorded nowhere. It is read off `Cargo.lock` rather than copied into
//! a const beside the dependency, so the value on a `prov:VerificationTrace` is the revision the
//! binary was built from — a hand-copied const agrees until someone bumps one and not the other,
//! which is the drift D86 §5 names as the reason to generate rather than to transcribe.

use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets this"));
    let lock = manifest
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/<name> sits two levels under the workspace root")
        .join("Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock.display());

    let text =
        std::fs::read_to_string(&lock).unwrap_or_else(|e| panic!("read `{}`: {e}", lock.display()));
    println!("cargo:rustc-env=EIGENIUS_NANODA_REV={}", nanoda_rev(&text));
}

/// The `rev=` fragment of `nanoda_lib`'s `source` line.
///
/// Panics rather than defaulting: an unknown checker identity written onto a trace as `unknown`
/// would be worse than a build failure, because it looks like a recorded fact.
fn nanoda_rev(lock: &str) -> String {
    let mut in_package = false;
    for line in lock.lines() {
        let line = line.trim();
        if line == "[[package]]" {
            in_package = false;
        } else if line == r#"name = "nanoda_lib""# {
            in_package = true;
        } else if in_package {
            if let Some(source) = line.strip_prefix(r#"source = ""#) {
                let rev = source
                    .split("rev=")
                    .nth(1)
                    .and_then(|r| r.split(['#', '"']).next())
                    .unwrap_or_else(|| {
                        panic!("`nanoda_lib`'s source line names no `rev=`: {line}")
                    });
                return rev.to_string();
            }
        }
    }
    panic!("`nanoda_lib` has no `source` line in Cargo.lock — is it still a git dependency?");
}
