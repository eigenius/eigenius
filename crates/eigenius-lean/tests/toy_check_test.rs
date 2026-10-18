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

//! Phase 20a.3 acceptance: `check_proof` admits a hand-vendored well-typed Lean export and
//! rejects a broken one, returning a structured diagnostic in both cases.
//!
//! `toy_proof_holds.json` is `PUnit`, its constructor `PUnit.unit` and its recursor `PUnit.rec`:
//! a minimal closed Lean environment holding the target name `PUnit`. **Regenerated from the
//! pinned toolchain `2026-09-07`** with
//! `cd lean/research/sigma-fixture && lake exe lean4export SigmaFixture -- PUnit`.
//!
//! It was previously the first 26 lines of `nanoda_lib`'s `test_resources/ProjFromProp/export`,
//! which declared Lean `4.27.0-rc1` while everything else in the tree pins `4.29.1`. Nothing was
//! wrong with it — the exported body is BYTE-IDENTICAL between the two versions, `PUnit` being
//! that stable — but `externalize_test.rs` uses this file as the environment it externalizes
//! propositions INTO, so it was asserting our Lean correspondence against a version we do not
//! ship. `checker::SUPPORTED_LEAN_MINOR` now refuses that, which is how the skew was found.
//! Regenerating also makes the fixture reproducible from a command in this repository rather
//! than a hand-truncated upstream artifact.
//!
//! `toy_proof_fails.json` stays vendored from `ProjFromProp/export` at `4.27.0-rc1`, and stays
//! that way deliberately: it is a hand-crafted UNSOUND export that no compiler emits, so there
//! is no toolchain to regenerate it from. It is only ever used for name-level checks, where no
//! correspondence of ours is involved and the version gate does not apply. It defines
//! `explosion_helper`/`explosion`, whose checking fails inside `infer_proj` for a `Prop`-valued
//! projection (the scenario nanoda's own `check_proj_from_prop` test exercises). The
//! type-checker panics; `check_proof` traps the panic and surfaces the diagnostic via
//! `Verdict::Fails`.

use eigenius_lean::{check_proof, Verdict};

const TOY_HOLDS: &[u8] = include_bytes!("../test_resources/toy_proof_holds.json");
const TOY_FAILS: &[u8] = include_bytes!("../test_resources/toy_proof_fails.json");

#[test]
fn toy_proof_holds_admits_well_typed_export() {
    let verdict = check_proof(TOY_HOLDS, "PUnit", &[], None).expect("infrastructure ok");
    assert_eq!(
        verdict,
        Verdict::Holds,
        "well-typed PUnit declaration must admit"
    );
}

#[test]
fn toy_proof_fails_rejects_broken_proof() {
    let verdict = check_proof(TOY_FAILS, "explosion", &[], None).expect("infrastructure ok");
    match verdict {
        Verdict::Fails { diagnostic } => {
            assert!(
                !diagnostic.is_empty(),
                "broken proof must surface a non-empty diagnostic"
            );
        }
        Verdict::Holds => panic!("ProjFromProp/explosion must not admit"),
    }
}

#[test]
fn missing_target_name_returns_fails_not_holds() {
    // Target name not declared in the export — the parser's
    // `unknown_pp_declar_hard_error` precondition catches this
    // before any checking runs.
    let verdict = check_proof(TOY_HOLDS, "DoesNotExist", &[], None).expect("infrastructure ok");
    match verdict {
        Verdict::Fails { diagnostic } => {
            assert!(
                diagnostic.contains("DoesNotExist")
                    || diagnostic.to_lowercase().contains("pp_declars")
                    || diagnostic.to_lowercase().contains("not found"),
                "missing-target diagnostic should reference the absent name; got: {diagnostic}"
            );
        }
        Verdict::Holds => panic!("absent target name must not admit"),
    }
}

#[test]
fn malformed_export_returns_fails() {
    // Garbage bytes — parser fails immediately. Confirms that we
    // surface parse errors through `Verdict::Fails`, not as a panic
    // or `CheckError`.
    let garbage = b"this is not valid lean4export JSON\n";
    let verdict = check_proof(garbage, "anything", &[], None).expect("infrastructure ok");
    assert!(
        matches!(verdict, Verdict::Fails { .. }),
        "malformed export must yield Fails, got {verdict:?}"
    );
}
