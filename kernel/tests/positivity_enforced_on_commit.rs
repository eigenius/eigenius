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

//! **Rule 23 reaches the commit path (eigenius#92, eigenius#188).**
//!
//! The strict-positivity checker existed for the whole of Phase 11b and was never called on a
//! declaration: `check_positivity` runs from `check_type`'s `Exp::Inductive` arm, the TERM form,
//! while an ESL `data` declaration becomes a `core:InductiveType` RESOURCE. eigenius#92 witnessed
//! the consequence by pushing a probe declaration through `Validator::validate()` and getting zero
//! errors.
//!
//! These tests take that same path — ESL source, compiled onto the real bootstrap chain, validated
//! — because that is the path that was broken. A test calling `check_positivity` directly would
//! have passed throughout the period the gate was open, which is precisely why the defect survived
//! (`nbe/positivity.rs` has nine such tests, all green, all of them missing this).
//!
//! Same shape as `short_name_pattern_enforced.rs`, which pins Rule 4 for the same reason.
//!
//! Rule 23 is no longer a positivity rule. It routes the declaration through `check_type`, of
//! which strict positivity is one component — telescope well-typedness and constructor-conclusion
//! validation are the others. The last test here covers the second.

use eigenius_kernel::bootstrap::bootstrap_with_storage;
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::validation::{ValidationRule, Validator};
use std::sync::Arc;

/// Compile ESL onto the real bootstrap chain and return the validation errors.
fn validate_esl(source: &str) -> Vec<(ValidationRule, String)> {
    let storage = LayerStorage::in_memory();
    let ctx = bootstrap_with_storage(storage.clone()).expect("bootstrap builds");
    let resources = eigenius_kernel::esl::compile_against_layer(source, ctx.head())
        .unwrap_or_else(|e| panic!("ESL must compile; positivity is a VALIDATION defect: {e:?}"));
    let mut b = LayerBuilder::new("probe", Some(Arc::clone(ctx.head())));
    for r in resources {
        b.add_resource(r).expect("add_resource");
    }
    let layer = Arc::new(b.build(storage));
    Validator::new(Arc::clone(&layer))
        .validate()
        .into_iter()
        .map(|e| (e.rule, e.message))
        .collect()
}

/// The unsound shape: `Bad` in the DOMAIN of an argument's function type. This is the declaration
/// eigenius#92 said "would equally be accepted" — and it was.
#[test]
fn negative_occurrence_is_rejected_at_commit() {
    let errors = validate_esl(
        r#"
        namespace core = "urn:eigenius:core";
        namespace probe = "urn:eigenius:probe";

        data probe:Bad : Type 1 {
            mk : (probe:Bad -> core:boolean) -> probe:Bad
        }
    "#,
    );
    let hits: Vec<&(ValidationRule, String)> = errors
        .iter()
        .filter(|(r, _)| *r == ValidationRule::InductiveDeclInadmissible)
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "`(Bad -> boolean) -> Bad` must be rejected by Rule 23; got: {errors:?}"
    );
    assert!(
        hits[0].1.contains("DOMAIN"),
        "the diagnostic must say WHERE the occurrence is, so an author can act on it: {}",
        hits[0].1
    );
}

/// The bootstrap's own shape, and the reason widening the criterion had to come first: this is
/// `lexicon:Cat`'s `cat_forall` with the parameter dropped. Wiring Rule 23 in under the
/// pre-eigenius#92 criterion would have rejected `ontologies/lexicon/lexicon-ontology.esl` and the
/// bootstrap would not load.
#[test]
fn higher_order_positive_occurrence_is_admitted_at_commit() {
    let errors = validate_esl(
        r#"
        namespace core = "urn:eigenius:core";
        namespace probe = "urn:eigenius:probe";

        data probe:Good : Type 1 {
            base : probe:Good,
            rall : (Set -> probe:Good) -> probe:Good
        }
    "#,
    );
    let hits: Vec<&(ValidationRule, String)> = errors
        .iter()
        .filter(|(r, _)| *r == ValidationRule::InductiveDeclInadmissible)
        .collect();
    assert!(
        hits.is_empty(),
        "`(Set -> Good) -> Good` is strictly positive and must commit: {hits:?}"
    );
}

/// The bootstrap chain itself, end to end: 42 `core:InductiveType` declarations, three of them
/// higher-order positive. If Rule 23 ever regresses to the direct-only criterion this fails here
/// rather than at the next reseed.
#[test]
fn the_bootstrap_chain_has_no_non_positive_declaration() {
    let storage = LayerStorage::in_memory();
    let ctx = bootstrap_with_storage(storage).expect("bootstrap builds");
    let offenders: Vec<String> = Validator::new(Arc::clone(ctx.head()))
        .validate()
        .into_iter()
        .filter(|e| e.rule == ValidationRule::InductiveDeclInadmissible)
        .map(|e| format!("{:?}: {}", e.resource_id, e.message))
        .collect();
    assert!(
        offenders.is_empty(),
        "the bootstrap must satisfy the rule it enforces:\n{}",
        offenders.join("\n")
    );
}

/// **Rule 23 checks the telescope, not just positivity (eigenius#188 / N4).**
///
/// `check_type`'s `Exp::Inductive` arm ran `check_positivity` and
/// `validate_indexed_ctor_conclusions` over the declaration's structured `params` / `indices` /
/// `ctors` fields, and never applied the Π typing rule to them — so a parameter kind or
/// constructor argument type that was not a type at all was admitted.
/// `references/nanoda_lib` gets the check for free, because a declaration there is one Π-chain
/// `Expr` and inferring it checks every binder domain (`src/tc.rs:165`, `src/inductive.rs:900`).
///
/// The probe declares `A : B` before `B` is bound. Positivity has no opinion about it (`Bad` does
/// not occur in either kind) and the constructor conclusion is well-formed, so this passes both of
/// the arm's original checks. It is caught only by walking the parameters IN ORDER and requiring
/// each kind to be a type in the scope of the ones before it.
#[test]
fn parameter_kind_referring_to_a_later_parameter_is_rejected_at_commit() {
    let errors = validate_esl(
        r#"
        namespace probe = "urn:eigenius:probe";

        data probe:Bad(A : B, B : Set) {
            mk(A),
        }
    "#,
    );
    let hits: Vec<&(ValidationRule, String)> = errors
        .iter()
        .filter(|(r, _)| *r == ValidationRule::InductiveDeclInadmissible)
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "a parameter kind naming a parameter declared after it must be rejected by Rule 23; \
         got: {errors:?}"
    );
    assert!(
        hits[0].1.contains('B'),
        "the diagnostic must name the offending reference: {}",
        hits[0].1
    );
}

/// **A constructor argument may not exceed its inductive's universe (eigenius#188).**
///
/// Port of nanoda's `check_ctor` universe check (`src/inductive.rs:904`). An inductive at `Sort n`
/// storing something from `Sort m` with `m > n` smuggles a large type into a small one, and
/// Girard's paradox follows. This is the check that forced `justification:Certificate` from `Type 0` to
/// `Type 2` — it binds `T : Type 1` in `spec_poly`, and nothing enforced the constraint before.
///
/// The probe that measured this before it rejected anything found exactly one violating
/// declaration across the whole workspace, which is why the ontology edit was a one-token change.
#[test]
fn constructor_argument_above_the_inductives_universe_is_rejected_at_commit() {
    let errors = validate_esl(
        r#"
        namespace probe = "urn:eigenius:probe";

        data probe:Big : Type 0 {
            mk : Type 1 -> probe:Big,
        }
    "#,
    );
    let hits: Vec<&(ValidationRule, String)> = errors
        .iter()
        .filter(|(r, _)| *r == ValidationRule::InductiveDeclInadmissible)
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "an inductive at `Type 0` must not store a `Type 1`; got: {errors:?}"
    );
    assert!(
        hits[0].1.contains("too large"),
        "the diagnostic must say the argument is too large, and for what: {}",
        hits[0].1
    );
}

/// **…unless the inductive is a `Prop`, which is the point of impredicativity.**
///
/// nanoda's `st.is_zero ||` (`src/inductive.rs:904`), where `is_zero(l)` is `leq(l, Zero)`
/// (`src/level.rs:264`). A proposition may quantify over anything without leaving `Prop`, because
/// it has no computational content to smuggle out. The same declaration that is rejected above is
/// admitted here with only its result sort changed — so the test pins the EXEMPTION, not merely
/// that something passes.
#[test]
fn a_prop_may_store_an_argument_from_any_universe() {
    let errors = validate_esl(
        r#"
        namespace probe = "urn:eigenius:probe";

        data probe:BigProp : Prop {
            mk : Type 1 -> probe:Big,
        }
    "#,
    );
    let hits: Vec<&(ValidationRule, String)> = errors
        .iter()
        .filter(|(r, _)| *r == ValidationRule::InductiveDeclInadmissible)
        .collect();
    assert!(
        hits.is_empty(),
        "`Prop` is impredicative — a proposition may quantify over any universe; got: {errors:?}"
    );
}

/// **A constructor's parameter prefix must match its inductive's (eigenius#219).**
///
/// Port of nanoda's `assert_def_eq(binder_type, local_param)`
/// (`references/nanoda_lib/src/inductive.rs:892`), which runs over the parameter prefix before its
/// constructor loop begins.
///
/// This only bites on the `core:ctor_type` path — the typed constructor form `mk : <type-expr>`,
/// where the author writes the whole Π chain including the parameters. The positional
/// `core:arg_types` form builds its prefix FROM `core:type_params`, so it agrees by construction
/// and could never have disagreed.
///
/// Measured before it rejected anything: zero prefix disagreements across the bootstrap chain and
/// the whole workspace.
#[test]
fn constructor_parameter_prefix_disagreeing_with_the_declaration_is_rejected() {
    let errors = validate_esl(
        r#"
        namespace probe = "urn:eigenius:probe";

        data probe:Box(A : Set) {
            mk : forall (A : Prop) => A -> probe:Box(A),
        }
    "#,
    );
    let hits: Vec<&(ValidationRule, String)> = errors
        .iter()
        .filter(|(r, _)| *r == ValidationRule::InductiveDeclInadmissible)
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "`data Box(A : Set)` with `mk : forall (A : Prop) => ...` must be rejected; got: {errors:?}"
    );
    assert!(
        hits[0].1.contains("parameter prefix") || hits[0].1.contains("parameter #0"),
        "the diagnostic must say WHICH parameter disagrees: {}",
        hits[0].1
    );
}
