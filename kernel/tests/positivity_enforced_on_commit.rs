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

//! **Rule 23 reaches the commit path (eigenius#92).**
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
        .filter(|(r, _)| *r == ValidationRule::NonPositiveInductive)
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
        .filter(|(r, _)| *r == ValidationRule::NonPositiveInductive)
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
        .filter(|e| e.rule == ValidationRule::NonPositiveInductive)
        .map(|e| format!("{:?}: {}", e.resource_id, e.message))
        .collect();
    assert!(
        offenders.is_empty(),
        "the bootstrap must satisfy the rule it enforces:\n{}",
        offenders.join("\n")
    );
}
