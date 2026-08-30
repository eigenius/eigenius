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

//! The four obligations D81 recorded as **declared but unchecked**.
//!
//! Rule 21 used to end in `check_infer` and DISCARD the type it returned. A
//! slot therefore only had to hold a term with *some* type — nothing asserted
//! it was the intended one. D81 named four slots where that gap was visible,
//! and each is now covered by a property-level declaration the rule reads:
//!
//! | slot | declares | what used to commit |
//! |---|---|---|
//! | `lexicon:cat` | `expected_type lexicon:Cat` | any well-typed term at all |
//! | `lexicon:sem_type` | `is_a_type` | a term that is not a type |
//! | `eigentt:axiom_statement` | `is_a_type` | ditto |
//! | `eigentt:definition_type` | `is_a_type` | ditto |
//!
//! These drive the validator against a real bootstrapped chain rather than
//! constructing a synthetic property, so what is under test is the DECLARATION
//! shipped in the ontology and not a fixture that happens to agree with it.
//! A slot whose declaration were dropped would fail here.

use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::nbe::term::Exp;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::program::eigentt_type_mirror::encode_type;
use eigenius_kernel::validation::{ValidationRule, Validator};
use std::sync::Arc;

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("valid IRI")
}

/// Validate one resource carrying `value` at `prop`, against the real
/// bootstrap chain, and return only Rule 21's diagnostics.
fn rule21_errors(class: &str, prop: &str, value: Value) -> Vec<ValidationRule> {
    let head = Arc::clone(
        eigenius_kernel::bootstrap::bootstrap()
            .expect("bootstrap")
            .head(),
    );
    let mut b = LayerBuilder::new("obligations", Some(head));
    let mut r = Resource::new(iri("urn:eigenius:test:subject"));
    r.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(class.to_string())]),
    );
    r.set(iri(prop), value);
    b.add_resource(r).expect("add subject");
    Validator::new(Arc::new(b.build(LayerStorage::in_memory())))
        .validate()
        .into_iter()
        .filter(|e| {
            e.property.as_ref().map(Iri::as_str) == Some(prop)
                && matches!(
                    e.rule,
                    ValidationRule::TermMalformed
                        | ValidationRule::TermIllTyped
                        | ValidationRule::TermNotAProposition
                )
        })
        .map(|e| e.rule)
        .collect()
}

/// A term that is well-typed but is NOT a type: the float literal `1.0`.
/// Inference succeeds on it, which is exactly why the old rule let it through.
fn a_well_typed_non_type() -> Value {
    encode_type(&Exp::LitFloat(1.0)).expect("literal encodes")
}

#[test]
fn a_lexicon_cat_slot_rejects_a_value_that_is_not_a_cat() {
    // Well-typed, and not a `lexicon:Cat`. Under `check_infer`-then-discard
    // this committed: it inferred `core:float` and the result was thrown away.
    let errs = rule21_errors(
        "urn:eigenius:lexicon:LexicalEntry",
        "urn:eigenius:lexicon:cat",
        a_well_typed_non_type(),
    );
    assert!(
        !errs.is_empty(),
        "lexicon:cat declares `expected_type lexicon:Cat`, so a non-Cat value must be \
         rejected; got no Rule 21 diagnostic"
    );
}

#[test]
fn a_lexicon_cat_slot_accepts_a_real_cat() {
    // The other half: the declaration must not reject what the lexicon
    // actually stores. `cat_n(Set, num_any)` is the shape 2,062,659 committed
    // entries carry.
    let cat = encode_type(&Exp::InductiveCtor(
        iri("urn:eigenius:lexicon:Cat"),
        "cat_n".into(),
        vec![
            // `cat_n : Set -> Num -> Cat` takes a MEMBER of Set — a class —
            // not `Set` itself. Committed entries pass a WordNet synset class
            // here; `lexicon:Entity` stands in for one.
            Exp::EigonClass(iri("urn:eigenius:lexicon:Entity")),
            Exp::InductiveCtor(iri("urn:eigenius:lexicon:Num"), "num_any".into(), vec![]),
        ],
    ))
    .expect("cat encodes");
    let errs = rule21_errors(
        "urn:eigenius:lexicon:LexicalEntry",
        "urn:eigenius:lexicon:cat",
        cat,
    );
    assert!(
        errs.is_empty(),
        "a real cat_n value must pass lexicon:cat; got {errs:?}"
    );
}

#[test]
fn a_lexicon_sem_type_slot_rejects_a_value_that_is_not_a_type() {
    let errs = rule21_errors(
        "urn:eigenius:lexicon:LexicalEntry",
        "urn:eigenius:lexicon:sem_type",
        a_well_typed_non_type(),
    );
    assert!(
        !errs.is_empty(),
        "lexicon:sem_type declares `is_a_type`, so a non-type value must be rejected"
    );
}

#[test]
fn an_axiom_statement_rejects_a_value_that_is_not_a_type() {
    let errs = rule21_errors(
        "urn:eigenius:eigentt:Axiom",
        "urn:eigenius:eigentt:axiom_statement",
        a_well_typed_non_type(),
    );
    assert!(
        !errs.is_empty(),
        "eigentt:axiom_statement declares `is_a_type`, so a non-type value must be rejected"
    );
}

#[test]
fn a_definition_type_rejects_a_value_that_is_not_a_type() {
    let errs = rule21_errors(
        "urn:eigenius:eigentt:Definition",
        "urn:eigenius:eigentt:definition_type",
        a_well_typed_non_type(),
    );
    assert!(
        !errs.is_empty(),
        "eigentt:definition_type declares `is_a_type`, so a non-type value must be rejected"
    );
}
