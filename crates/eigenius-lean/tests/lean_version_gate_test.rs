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

//! D86 §3.1 — the export declares which Lean produced it, and this build refuses the ones whose
//! `Float` semantics it was not written against.
//!
//! `nanoda_lib` gates the export FORMAT version and ignores the LEAN version. That asymmetry is
//! what made the `4.33.0` `Float.Model` change a silent hazard: the terms `externalize.rs`
//! builds still elaborate under `4.33`, so the skew would surface only as a defeq failure naming
//! no version. These tests pin the gate and pin it to the toolchain files.

use std::path::PathBuf;
use std::sync::Arc;

use eigenius_kernel::layer::Layer;
use eigenius_kernel::nbe::term::Exp;
use eigenius_lean::checker::{
    check_proof, lean_version_of, CheckError, ExpectedStatement, SUPPORTED_LEAN_MINOR,
};

/// `PUnit` and friends, exported at the pin. Small, and `PUnit.unit : PUnit` is a statement the
/// fragment can express (`Exp::One`), so the gate can be exercised through a real statement
/// check rather than a name-level one.
const FIXTURE: &[u8] = include_bytes!("../test_resources/toy_proof_holds.json");

/// A layer is consulted only to resolve chain IRIs' `short_name`; `Exp::One` names none, so the
/// bootstrap head serves.
fn head() -> Arc<Layer> {
    let ctx = eigenius_kernel::bootstrap::bootstrap().expect("bootstrap");
    Arc::clone(ctx.head())
}

/// A statement check against `bytes`, which is the configuration the gate applies to.
///
/// **Why every gate test goes through a statement check.** The gate fires only when `expected` is
/// `Some`, because that is exactly when `externalize` runs and hard-codes what Lean constants
/// mean. A name-level check asserts only that the export is internally sound and holds the
/// target — Lean's declarations against Lean's kernel rules — which no correspondence of ours
/// touches. Calling with `None` here would test nothing.
fn statement_check(bytes: &[u8]) -> Result<eigenius_lean::Verdict, CheckError> {
    let layer = head();
    check_proof(
        bytes,
        "PUnit.unit",
        &[],
        Some(&ExpectedStatement {
            proposition: &Exp::One,
            layer: &layer,
        }),
    )
}

/// The workspace root, from this crate's manifest directory.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root is two levels above this crate")
}

/// **The drift this suite exists to prevent.**
///
/// [`SUPPORTED_LEAN_MINOR`] is a second copy of a fact whose home is `lean/*/lean-toolchain`.
/// Two copies drift, so this fails the moment they disagree — which turns "bump the toolchain
/// and discover the skew months later from a confusing defeq failure" into "bump the toolchain
/// and this test tells you the correspondence needs review."
#[test]
fn lean_toolchain_pin_matches_the_supported_series() {
    let root = workspace_root();
    let mut checked = 0;

    for entry in walk(&root.join("lean")) {
        let text = std::fs::read_to_string(&entry).expect("a lean-toolchain file is readable");
        let pin = text.trim();
        // `leanprover/lean4:v4.29.1` -> `4.29.1`
        let version = pin
            .rsplit_once(":v")
            .unwrap_or_else(|| {
                panic!(
                    "{} reads `{pin}`, not `leanprover/lean4:vX.Y.Z`",
                    entry.display()
                )
            })
            .1;
        assert!(
            version.starts_with(&format!("{SUPPORTED_LEAN_MINOR}.")),
            "{} pins Lean {version}, but `SUPPORTED_LEAN_MINOR` is `{SUPPORTED_LEAN_MINOR}`. \
             Moving the toolchain means reviewing the numeric correspondence (D86 §3.1) — \
             `Float.lt`/`Float.le` changed from `Prop` to `Bool` in 4.33 — and updating both \
             in one change.",
            entry.display(),
        );
        checked += 1;
    }

    assert!(
        checked >= 4,
        "expected to find the lean-toolchain pins under `lean/`; found {checked}. If they moved, \
         this test is no longer guarding anything."
    );
}

/// Every `lean-toolchain` under `dir`, recursively.
fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else if path.file_name().is_some_and(|n| n == "lean-toolchain") {
            out.push(path);
        }
    }
    out
}

/// The checked-in fixture declares the version we support, so the gate is not vacuously passing.
#[test]
fn the_fixture_declares_the_supported_lean_version() {
    let found = lean_version_of(FIXTURE).expect("the fixture's first line is the meta header");
    assert!(
        found.starts_with(&format!("{SUPPORTED_LEAN_MINOR}.")),
        "fixture declares Lean {found}, expected {SUPPORTED_LEAN_MINOR}.x",
    );
}

/// **The gate fires.** A `4.33` export is refused before any checking work, naming both versions.
///
/// `4.33` specifically because that is where `Float` became a structure over `Float.Model` and
/// `Float.lt`/`Float.le` changed signature — the change that would otherwise pass silently.
#[test]
fn an_export_from_a_later_lean_is_refused_before_it_is_checked() {
    let doctored = retarget_lean_version(FIXTURE, "4.33.1");

    let err =
        statement_check(&doctored).expect_err("a version skew is a CheckError, not a verdict");

    match err {
        CheckError::UnsupportedLeanVersion { found, supported } => {
            assert_eq!(found, "4.33.1");
            assert_eq!(supported, SUPPORTED_LEAN_MINOR);
        }
        other => panic!("expected UnsupportedLeanVersion, got {other:?}"),
    }
}

/// A skew is NOT `Verdict::Fails`. `Fails` means "checked, and it does not hold" — the
/// institution records that as a refuted claim. An export we cannot judge must not produce it.
#[test]
fn a_version_skew_is_unjudgeable_rather_than_refuted() {
    let doctored = retarget_lean_version(FIXTURE, "4.33.1");
    assert!(
        statement_check(&doctored).is_err(),
        "a skewed export must not return a Verdict at all",
    );
}

/// A file with no metadata header is refused too, rather than defaulting to "supported".
#[test]
fn an_export_without_a_lean_version_is_refused() {
    let headerless = b"{\"in\":1,\"str\":{\"pre\":0,\"str\":\"Subtype\"}}\n".to_vec();
    match statement_check(&headerless) {
        Err(CheckError::MissingLeanVersion) => {}
        other => panic!("expected MissingLeanVersion, got {other:?}"),
    }
    assert_eq!(lean_version_of(&headerless), None);
}

/// Patch bumps pass. A definition's shape cannot change in a patch release, so gating the patch
/// component would fail loudly for no reason.
#[test]
fn a_patch_bump_within_the_supported_series_is_accepted() {
    let patched = retarget_lean_version(FIXTURE, &format!("{SUPPORTED_LEAN_MINOR}.99"));
    assert!(
        !matches!(
            statement_check(&patched),
            Err(CheckError::UnsupportedLeanVersion { .. })
        ),
        "a patch bump inside {SUPPORTED_LEAN_MINOR}.x must not trip the gate",
    );
}

/// Rewrite `meta.lean.version` in an export's header line, leaving the body untouched.
fn retarget_lean_version(bytes: &[u8], version: &str) -> Vec<u8> {
    let text = std::str::from_utf8(bytes).expect("the fixture is UTF-8");
    let (header, body) = text
        .split_once('\n')
        .expect("the fixture has a header line");
    let mut meta: serde_json::Value =
        serde_json::from_str(header).expect("the header line is JSON");
    meta["meta"]["lean"]["version"] = serde_json::Value::String(version.to_string());
    format!("{meta}\n{body}").into_bytes()
}

/// **The other half of the gate's contract: a name-level check is NOT gated.**
///
/// Without this, someone tightening the gate to fire unconditionally would break every
/// name-level check against a vendored `nanoda_lib` fixture — `toy_proof_fails.json` is a
/// hand-crafted unsound export at `4.27.0-rc1` that no toolchain can regenerate — and the suite
/// would not say why. The version matters when we externalize INTO an environment, not when we
/// ask whether that environment is internally sound.
#[test]
fn a_name_level_check_is_not_gated_on_the_lean_version() {
    let doctored = retarget_lean_version(FIXTURE, "4.33.1");
    assert!(
        check_proof(&doctored, "PUnit", &[], None).is_ok(),
        "a name-level check involves no correspondence, so no version gate applies",
    );
}
