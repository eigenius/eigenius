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

//! D74 — the statement-level check, end to end against a real `lean4export` payload.
//!
//! `notebook_demo_fixture_lands_holds` cannot cover this: its claim
//! (`urn:eigenius:demo:lean:patient_1`) carries only `is_a` and no
//! `reflection:canonical_proposition`, so `claim_proposition` returns `None` and the check is
//! skipped. A green run there says nothing about this path — which is the failure mode this
//! whole line of work keeps finding, so it is stated rather than left to be rediscovered.
//!
//! The toy export declares `PUnit : Sort u`, `PUnit.unit : PUnit.{u}` and `PUnit.rec`. That is
//! enough to drive the fragment's `One` / `Sort` / `Param` arms against terms nanoda parsed
//! itself.

use std::sync::Arc;

use eigenius_kernel::layer::Layer;
use eigenius_kernel::nbe::level::Level;
use eigenius_kernel::nbe::term::Exp;
use eigenius_lean::checker::{check_proof, ExpectedStatement, Verdict};

const TOY_HOLDS: &[u8] = include_bytes!("../test_resources/toy_proof_holds.json");

/// A layer is only consulted to resolve a chain IRI's `short_name`; these propositions name no
/// chain resources, so the bootstrap head serves.
fn head() -> Arc<Layer> {
    let ctx = eigenius_kernel::bootstrap::bootstrap().expect("bootstrap");
    Arc::clone(ctx.head())
}

fn check(target: &str, prop: &Exp) -> Verdict {
    let layer = head();
    check_proof(
        TOY_HOLDS,
        target,
        &[],
        Some(&ExpectedStatement {
            proposition: prop,
            layer: &layer,
        }),
    )
    .expect("infrastructure ok")
}

/// The pre-D74 behaviour, kept explicit: with no expected statement the call establishes that a
/// declaration named `target` type-checks, and says nothing about what it proves.
#[test]
fn without_an_expected_statement_the_check_is_name_level_only() {
    let v = check_proof(TOY_HOLDS, "PUnit", &[], None).expect("infrastructure ok");
    assert!(matches!(v, Verdict::Holds), "got {v:?}");
}

/// `PUnit.unit`'s type is `PUnit.{u}`. The fragment's `One` maps to Lean's `PUnit`, so this is
/// the smallest end-to-end statement check there is: externalize, compare under `def_eq`.
#[test]
fn the_claims_proposition_matching_the_target_type_holds() {
    let v = check("PUnit.unit", &Exp::One);
    assert!(
        matches!(v, Verdict::Holds),
        "`One` must externalize to the `PUnit` that `PUnit.unit` inhabits; got {v:?}"
    );
}

/// The point of the whole exercise: a proof that type-checks, bound to a proposition that is not
/// what it proves, must not reach `Holds`. Before D74 this returned `Holds` — check 1 passes,
/// and nothing related the named theorem to the claim (#159).
#[test]
fn a_proposition_the_target_does_not_prove_fails() {
    // `PUnit.unit : PUnit`, not `PUnit : Sort u`.
    let v = check("PUnit.unit", &Exp::Sort(Level::Zero));
    match v {
        Verdict::Fails { diagnostic } => assert!(
            diagnostic.contains("not the claim's proposition"),
            "diagnostic should name the mismatch; got {diagnostic}"
        ),
        other => panic!("a mismatched statement must not Hold; got {other:?}"),
    }
}

/// Refusal is typed and total (D74 §4.2): a proposition outside the fragment fails with the
/// variant named, rather than being approximated into a different theorem.
#[test]
fn a_proposition_outside_the_fragment_is_refused_by_name() {
    let v = check("PUnit.unit", &Exp::LitFloat(1.5));
    match v {
        Verdict::Fails { diagnostic } => {
            assert!(
                diagnostic.contains("LitFloat"),
                "the refusal must name the variant; got {diagnostic}"
            );
            assert!(
                diagnostic.contains("externalize"),
                "and say it could not be externalized; got {diagnostic}"
            );
        }
        other => panic!("an unrepresentable proposition must not Hold; got {other:?}"),
    }
}

/// D74 §6.5 — a `Level::Param` naming something the target does not declare is refused here
/// rather than left to `def_eq`, which would compare one parameter against a different one and
/// fail with nothing to say that universes were the cause.
#[test]
fn a_universe_param_the_target_does_not_declare_is_refused() {
    let v = check("PUnit.unit", &Exp::Sort(Level::Param("nonesuch".into())));
    match v {
        Verdict::Fails { diagnostic } => assert!(
            diagnostic.contains("nonesuch") && diagnostic.contains("universe parameter"),
            "the refusal must name the parameter; got {diagnostic}"
        ),
        other => panic!("an undeclared universe param must not Hold; got {other:?}"),
    }
}
