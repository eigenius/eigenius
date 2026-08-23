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

//! Rule 21 step 3 (`wk::PROPOSITION_SLOTS`) fires **through a real commit**.
//!
//! Issue #175. `enc:EncodedClaim` is the artifact of record for the encoding
//! pipeline: an LLM proposes a formalization, the kernel accepts or rejects
//! it, and from there only `reflection:canonical_proposition` and the
//! derivations built on it carry weight. Rule 21 called `check_infer` and
//! discarded the type it returned, so the slot only had to be *well-typed* —
//! an integer literal decoded, inferred `core:integer`, and committed as the
//! proposition a claim asserts.
//!
//! The unit tests in `validation/rules/eigentt_value.rs` drive the validator
//! directly. This file goes through `commit_layer_default` against a real
//! bootstrapped chain, because the commit gate is the whole of the guarantee:
//! nothing downstream re-reads the prose, and no other check inspects the
//! sort of a decoded `TypeExpr`.

use eigenius_kernel::lattice::{commit_layer_default, CommitError};
use eigenius_kernel::layer::LayerStorage;
use eigenius_kernel::nbe::term::Exp;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::program::eigentt_type_mirror::encode_type;
use eigenius_kernel::storage::memory::MemoryPersistentBackend;
use eigenius_kernel::validation::ValidationRule;
use std::sync::Arc;

const UNIT: &str = "urn:eigenius:test:p175:unit";
const SCOPED: &str = "urn:eigenius:test:p175:scoped";
const CLAIM: &str = "urn:eigenius:test:p175:claim";

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("static IRI")
}

fn strings(values: &[&str]) -> Value {
    Value::Array(values.iter().map(|v| Value::String((*v).into())).collect())
}

/// The provenance chain `enc:EncodedClaim` requires: a `DiscourseUnit` (the
/// source span) wrapped in a `ScopedUnit` (the lexicon scope its parse was
/// ranked against).
fn source_unit_resources() -> Vec<Resource> {
    let mut unit = Resource::new(iri(UNIT));
    unit.set(
        iri(wk::IS_A),
        strings(&["urn:eigenius:encoding:DiscourseUnit"]),
    );
    unit.set(
        iri("urn:eigenius:encoding:prose"),
        Value::String("WRN is selectively essential in MSI cells.".into()),
    );
    unit.set(
        iri("urn:eigenius:encoding:unit_kind"),
        Value::String("urn:eigenius:encoding:kind_prose".into()),
    );

    let mut scoped = Resource::new(iri(SCOPED));
    scoped.set(
        iri(wk::IS_A),
        strings(&["urn:eigenius:encoding:ScopedUnit"]),
    );
    scoped.set(
        iri("urn:eigenius:encoding:unit"),
        Value::String(UNIT.into()),
    );

    vec![unit, scoped]
}

/// An `enc:EncodedClaim` asserting `proposition`.
fn claim(proposition: Value) -> Resource {
    let mut r = Resource::new(iri(CLAIM));
    r.set(
        iri(wk::IS_A),
        strings(&["urn:eigenius:encoding:EncodedClaim"]),
    );
    r.set(
        iri("urn:eigenius:encoding:from_unit"),
        Value::String(SCOPED.into()),
    );
    // REQUIRED since eigenius#201 made `enc:EncodedClaim` a `reflection:DeclaredResource`: a parse
    // establishes form, not warrant, so a landed claim must name the agent who asserts it.
    r.set(
        iri("urn:eigenius:reflection:declared_by"),
        Value::String("urn:eigenius:reflection:agent:unattributed".into()),
    );
    r.set(iri(wk::CANONICAL_PROPOSITION), proposition);
    r
}

/// `measurements:lt(1.0, 2.0)` — an axiom application, so a term at `Prop`.
fn a_real_proposition() -> Value {
    Value::Json(serde_json::json!({
        "ctor": "App",
        "args": [
            {"ctor": "App", "args": [
                {"ctor": "ConstRef", "args": ["urn:eigenius:measurements:lt"]},
                {"ctor": "LitFloat", "args": [1.0]}
            ]},
            {"ctor": "LitFloat", "args": [2.0]}
        ]
    }))
}

/// Bootstrap on a memory backend and try to commit one claim carrying
/// `proposition`.
fn commit_claim(proposition: Value) -> Result<(), CommitError> {
    let backend = Arc::new(MemoryPersistentBackend::new());
    let mut ctx =
        eigenius_kernel::bootstrap::bootstrap_with_storage(LayerStorage::with_persistent(
            Arc::clone(&backend) as Arc<dyn eigenius_kernel::storage::PersistentBackend>,
        ))
        .expect("bootstrap");

    for r in source_unit_resources() {
        ctx.add_resource(r).expect("add source unit");
    }
    ctx.add_resource(claim(proposition)).expect("add claim");
    let working = ctx.take_working("p175").expect("take_working");
    commit_layer_default(working, ctx.storage().clone(), backend.as_ref()).map(|_| ())
}

fn rejection_errors(
    proposition: Value,
    why: &str,
) -> Vec<eigenius_kernel::validation::ValidationError> {
    match commit_claim(proposition) {
        Ok(()) => panic!("a claim whose canonical_proposition is {why} must not commit"),
        Err(CommitError::Validation { errors, .. }) => errors,
        Err(other) => panic!("expected a validation failure, got {other:?}"),
    }
}

/// The defect in #175, end to end: a literal in the proposition slot.
#[test]
fn integer_literal_claim_is_rejected_by_the_commit() {
    let errors = rejection_errors(
        encode_type(&Exp::LitInt(42)).expect("literal encodes"),
        "an integer literal",
    );
    let hit = errors
        .iter()
        .find(|e| e.rule == ValidationRule::TypeExprNotAProposition)
        .unwrap_or_else(|| panic!("no TypeExprNotAProposition among {errors:?}"));
    assert_eq!(
        hit.resource_id.as_ref().map(Iri::as_str),
        Some(CLAIM),
        "the diagnostic must name the claim: {hit:?}"
    );
    assert_eq!(
        hit.property.as_ref().map(Iri::as_str),
        Some(wk::CANONICAL_PROPOSITION)
    );
    assert!(
        hit.message.contains("Prop = Sort(0)"),
        "diagnostic should name the obligation: {}",
        hit.message
    );
}

/// `Prop` itself decodes and type-checks — it is a legitimate value for the
/// `eigentt:TypeExpr`-ranged slots that hold types. It asserts nothing, so it
/// is not a claim.
#[test]
fn a_type_in_the_proposition_slot_is_rejected_by_the_commit() {
    let errors = rejection_errors(encode_type(&Exp::sort(0)).expect("Prop encodes"), "a type");
    assert!(
        errors
            .iter()
            .any(|e| e.rule == ValidationRule::TypeExprNotAProposition),
        "no TypeExprNotAProposition among {errors:?}"
    );
}

/// Positive control: a well-formed claim still commits, so the rejections
/// above are measuring propositionhood and not the shape of the fixture.
#[test]
fn well_formed_claim_commits() {
    commit_claim(a_real_proposition()).expect("a claim asserting a Prop commits");
}

/// The residual route, pinned: issue #191. `check` admits `EigonClass` and
/// `EigonPrimitive` against **every** universe including `Sort(0)`
/// (`nbe/check/mod.rs`), while inference gives them `Sort(1)`. `Exp::Ann(e,
/// t)` checks `e` against `t` and then reports `t` as the inferred type, so
/// `(core:Class : Prop)` reaches this gate already wearing `Sort(0)` and the
/// gate has nothing to catch.
///
/// #175 closes the unannotated route; the annotated one needed #191, which lands in
/// the same commit series. Measured both ways: with #175 alone the annotated class
/// COMMITS, and this test fails; with #191 present it is rejected. That is why the two
/// had to land together, and this test is the evidence.
#[test]
fn a_class_annotated_as_a_proposition_is_rejected_by_the_commit() {
    let annotated = Value::Json(serde_json::json!({
        "ctor": "Ann",
        "args": [
            {"ctor": "ConstRef", "args": ["urn:eigenius:core:Class"]},
            {"ctor": "Sort", "args": [0]}
        ]
    }));
    let errors = rejection_errors(annotated, "a class annotated as Prop");
    assert!(
        errors
            .iter()
            .any(|e| e.rule == ValidationRule::TypeExprNotAProposition
                || e.rule == ValidationRule::TypeExprIllTyped),
        "no propositionhood or typing diagnostic among {errors:?}"
    );
}
