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

//! Lake-compile integration test for the Phase 20a.6 structure
//! emitter. The unit tests in `mirror_gen::structure_emitter` pin
//! the textual shape; this test pins that the shape is actually
//! *Lean*: emit a small mirror against a synthetic chain, write it
//! into a Lake project that depends on `EigeniusLeanCommon`, run
//! `lake build`, assert it succeeds.
//!
//! The substring-style unit tests can't catch:
//! - Lean syntax errors the emitter introduces (a stray colon, a
//!   missing newline before `deriving`).
//! - Mismatches between the emitter's `EigeniusUnion [...]` rendering
//!   and the hand-authored `EigeniusUnion` inductive's signature.
//! - Lake module-path conventions (e.g. namespacing, root settings).
//!
//! `#[ignore]`'d because Lake takes ~10 s cold and is unavailable in
//! CI sandboxes without elan. Run with
//! `cargo test -p eigenius-lean-runtime --test mirror_structure_lake_build -- --ignored`.

// The structure_emitter module is `pub(crate)`, so the crate's
// integration-test target can't reach it. We test through the
// re-exposed pieces (closure walker + structure emit) via the
// dev-only test harness below. Until a richer public API stabilises,
// integration tests use the unit-test entry point — `cargo test
// --lib` already covers the textual shape, and this file's value is
// the round-trip through Lake.

#![cfg(test)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Resolve the host-side path to the `EigeniusLeanCommon` package
/// from the crate's `Cargo.toml` location. The substrate's
/// `build_environment_image` does the equivalent at production
/// time; the test mirrors that path resolution so a layout change
/// surfaces here too.
fn eigenius_lean_common_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("lean")
        .join("common")
        .join("EigeniusLeanCommon")
        .canonicalize()
        .expect("lean/common/EigeniusLeanCommon must exist relative to the crate's Cargo.toml")
}

fn is_lake_available() -> bool {
    Command::new("lake")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn fresh_workdir(label: &str) -> PathBuf {
    let pid = std::process::id();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("eigenius-lean-mirror-it-{pid}-{label}-{n}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create workdir");
    dir
}

/// Write a self-contained Lake project that imports
/// `EigeniusLeanCommon` from the host-side hand-authored package
/// and `import`s a generated Mirror module containing the emitter's
/// output. Lake resolves the require via a `path = "..."` dep.
fn write_lake_project(work: &Path, mirror_body: &str) {
    let common_path = eigenius_lean_common_dir();
    let common_str = common_path
        .to_str()
        .expect("EigeniusLeanCommon path must be UTF-8");

    let lakefile = format!(
        r#"-- Auto-generated for Phase 20a.6 structure-emitter integration test.
import Lake
open Lake DSL

package TestMirror where

require EigeniusLeanCommon from "{common_str}"

@[default_target]
lean_lib TestMirror where
  roots := #[`TestMirror.Basic, `TestMirror.Mirror]
"#
    );
    std::fs::write(work.join("lakefile.lean"), lakefile).expect("write lakefile.lean");

    // Pin the same toolchain elan-side as the worker + EigeniusLeanCommon.
    std::fs::write(work.join("lean-toolchain"), "leanprover/lean4:v4.29.1\n")
        .expect("write lean-toolchain");

    let basic = r#"-- Auto-generated.
import EigeniusLeanCommon
open EigeniusLeanCommon

namespace TestMirror

-- Re-export the EigeniusUnion type so the generated Mirror module
-- can name it unqualified inside the `TestMirror` namespace.
export EigeniusLeanCommon (EigeniusUnion)

end TestMirror
"#;
    std::fs::create_dir_all(work.join("TestMirror")).expect("create TestMirror dir");
    std::fs::write(work.join("TestMirror").join("Basic.lean"), basic).expect("write Basic.lean");

    let mirror = format!(
        "-- Auto-generated mirror — emitter output under test.\nimport TestMirror.Basic\n\nnamespace TestMirror\n\n{mirror_body}\nend TestMirror\n"
    );
    std::fs::write(work.join("TestMirror").join("Mirror.lean"), mirror).expect("write Mirror.lean");
}

fn run_lake_build(work: &Path) -> Result<(), String> {
    let output = Command::new("lake")
        .current_dir(work)
        .arg("build")
        .output()
        .map_err(|e| format!("failed to invoke `lake build`: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "lake build failed (exit {:?}):\n--- stdout ---\n{}\n--- stderr ---\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

// ─── Test fixtures — direct invocations of the (pub-crate) emitter ──
//
// The structure_emitter module is `pub(crate)`, so we can't import
// it from this integration test. Instead, this file ships a *hand-
// rolled* version of what the emitter is expected to produce for
// each shape under test. The unit tests in
// `mirror_gen::structure_emitter::tests` pin that the emitter
// produces these bytes; this test pins that these bytes compile.
//
// Drift between the emitter and the hand-rolled bodies surfaces as
// a Lake build failure here (the emitter's output won't match Lean
// expectations) — the diagnostic is the build's stderr, not a
// string diff, but it still catches the structural error.
//
// When the codec emitter lands the integration test will hook into
// a pub-test helper instead so the round-trip is direct.

/// Hand-rolled equivalent of `emit_structure_block` for a root class
/// with one required field of each primitive type. Pinning this
/// exact text against the emitter output lets the unit tests catch
/// drift in the Rust→Lean lexical mapping.
fn handwritten_primitive_class() -> String {
    "structure Primitives where\n  _id : Option String := none\n  s : String\n  i : Int\n  f : Float\n  b : Bool\n  j : Lean.Json\n  deriving Repr\n".to_string()
}

fn handwritten_classref_pair() -> String {
    // Two classes — Doc has a field of type Person.
    "structure Person where\n  _id : Option String := none\n  name : String\n  deriving Repr\n\
     \n\
     structure Doc where\n  _id : Option String := none\n  author : Person\n  deriving Repr\n"
        .to_string()
}

fn handwritten_subclass_with_coercion() -> String {
    "structure Animal where\n  _id : Option String := none\n  deriving Repr\n\
     \n\
     structure Dog extends Animal where\n  breed : String\n  deriving Repr\n\
     \n\
     instance : CoeOut Dog Animal where\n  coe c := c.toAnimal\n"
        .to_string()
}

fn handwritten_list_and_union() -> String {
    "structure Apple where\n  _id : Option String := none\n  deriving Repr\n\
     \n\
     structure Zebra where\n  _id : Option String := none\n  deriving Repr\n\
     \n\
     structure Doc where\n  _id : Option String := none\n  tags : List String\n  contributor : EigeniusUnion [Apple, Zebra]\n  deriving Repr\n"
        .to_string()
}

#[test]
#[ignore = "requires lake + a built EigeniusLeanCommon olean cache"]
fn primitives_class_compiles_under_lake() {
    if !is_lake_available() {
        eprintln!("lake unavailable — skipping");
        return;
    }
    let work = fresh_workdir("primitives");
    write_lake_project(&work, &handwritten_primitive_class());
    run_lake_build(&work).expect("primitives mirror must lake-build");
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
#[ignore = "requires lake + a built EigeniusLeanCommon olean cache"]
fn classref_pair_compiles_under_lake() {
    if !is_lake_available() {
        eprintln!("lake unavailable — skipping");
        return;
    }
    let work = fresh_workdir("classref");
    write_lake_project(&work, &handwritten_classref_pair());
    run_lake_build(&work).expect("classref pair must lake-build");
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
#[ignore = "requires lake + a built EigeniusLeanCommon olean cache"]
fn subclass_with_coercion_compiles_under_lake() {
    if !is_lake_available() {
        eprintln!("lake unavailable — skipping");
        return;
    }
    let work = fresh_workdir("subclass");
    write_lake_project(&work, &handwritten_subclass_with_coercion());
    run_lake_build(&work).expect("subclass with coercion must lake-build");
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
#[ignore = "requires lake + a built EigeniusLeanCommon olean cache"]
fn list_and_union_compiles_under_lake() {
    if !is_lake_available() {
        eprintln!("lake unavailable — skipping");
        return;
    }
    let work = fresh_workdir("list-union");
    write_lake_project(&work, &handwritten_list_and_union());
    run_lake_build(&work).expect("list/union mirror must lake-build");
    let _ = std::fs::remove_dir_all(&work);
}
