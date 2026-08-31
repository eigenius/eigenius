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

//! Rule 4 (`core:format`) fires **through a real commit**.
//!
//! Issue #118. `core:format` is declared `data_type: core:resource`, so
//! `canonicalise_resource_refs` — the first statement of
//! `LayerBuilder::build`, ahead of `structural_validate` — rewrites the
//! `format` slot on every property definition from `Value::String` to
//! `Value::ResourceRef`. `check_format` matched `Some(Value::String(s))`
//! alone, took the fall-through arm, and returned an empty diagnostic
//! vector: a malformed date committed with no diagnostic.
//!
//! The bug was shape-dependent, not universal. A definition rehydrated
//! from RocksDB comes back as `Value::String` (the CBOR layer normalises
//! `ResourceRef`), so the old match did fire for ancestor definitions on
//! a resumed store. It never fired for a definition in the layer being
//! committed, or anywhere on an in-memory chain.
//!
//! The unit tests already in `rules/format.rs` could not catch this: they
//! call `is_valid_date` and friends directly, so they never exercise the
//! read of the `format` slot. Only a commit does, because only
//! `LayerBuilder::build` canonicalises. Hence this file goes through
//! `commit_layer_default` rather than a hand-built `prop_def`.

use eigenius_kernel::lattice::commit_layer_default;
use eigenius_kernel::lattice::CommitError;
use eigenius_kernel::layer::{Layer, LayerStorage};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::storage::memory::MemoryPersistentBackend;
use eigenius_kernel::storage::PersistentBackend;
use eigenius_kernel::validation::{ValidationRule, Validator};
use std::sync::Arc;

const MEASURED_ON: &str = "urn:eigenius:test:fmt:measured_on";
const DATED_CLASS: &str = "urn:eigenius:test:fmt:Dated";

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("static IRI")
}

/// Bootstrap on a memory backend, then commit a layer declaring
/// `test:fmt:Dated` and a `test:fmt:measured_on` property carrying
/// `format = core:formats:date`.
fn chain_with_a_date_property() -> (
    eigenius_kernel::context::ExecutionContext,
    Arc<dyn PersistentBackend>,
) {
    chain_with_a_date_property_on(false)
}

/// `persist_bootstrap` seeds the whole bootstrap chain into the backend
/// (via `bootstrap_persistent`) so the chain can be reloaded from storage.
/// Only the round-trip test needs that; the others skip the seed cost.
fn chain_with_a_date_property_on(
    persist_bootstrap: bool,
) -> (
    eigenius_kernel::context::ExecutionContext,
    Arc<dyn PersistentBackend>,
) {
    let backend = Arc::new(MemoryPersistentBackend::new());
    let dyn_backend = Arc::clone(&backend) as Arc<dyn PersistentBackend>;
    let mut ctx = if persist_bootstrap {
        eigenius_kernel::bootstrap::bootstrap_persistent(Arc::clone(&dyn_backend))
            .expect("bootstrap_persistent")
    } else {
        eigenius_kernel::bootstrap::bootstrap_with_storage(LayerStorage::with_persistent(
            Arc::clone(&dyn_backend),
        ))
        .expect("bootstrap")
    };

    let mut class = Resource::new(iri(DATED_CLASS));
    class.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::String("urn:eigenius:core:Class".into())]),
    );
    class.set(
        iri("urn:eigenius:core:short_name"),
        Value::String("Dated".into()),
    );
    class.set(
        iri("urn:eigenius:core:description"),
        Value::String("Test class carrying a date-formatted property.".into()),
    );

    let mut prop = Resource::new(iri(MEASURED_ON));
    prop.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::String("urn:eigenius:core:Property".into())]),
    );
    prop.set(
        iri("urn:eigenius:core:short_name"),
        Value::String("measured_on".into()),
    );
    prop.set(
        iri("urn:eigenius:core:description"),
        Value::String("Date the measurement was taken.".into()),
    );
    prop.set(
        iri("urn:eigenius:core:data_type"),
        Value::String("urn:eigenius:core:string".into()),
    );
    // The slot under test. Written as a `Value::String`, exactly the
    // shape Eigon-JSON parsing produces — `LayerBuilder::build` turns it
    // into a `Value::ResourceRef` before the validator ever sees it.
    prop.set(
        iri("urn:eigenius:core:format"),
        Value::String("urn:eigenius:core:formats:date".into()),
    );

    ctx.add_resource(class).expect("add class");
    ctx.add_resource(prop).expect("add property");
    let working = ctx.take_working("fmt_decls").expect("take_working");
    let layer = commit_layer_default(working, ctx.storage().clone(), backend.as_ref())
        .expect("declaration layer commits");
    ctx.advance_head(layer, "fmt_decls").expect("advance_head");

    (ctx, dyn_backend)
}

fn instance(local: &str, date: &str) -> Resource {
    let mut r = Resource::new(iri(&format!("urn:eigenius:test:fmt:{local}")));
    r.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::String(DATED_CLASS.into())]),
    );
    r.set(iri(MEASURED_ON), Value::String(date.into()));
    r
}

/// The declaration layer must canonicalise `format` to a `ResourceRef` —
/// this is the precondition that made Rule 4 unreachable. Asserted so a
/// future change to `canonicalise_resource_refs` that stops rewriting the
/// slot doesn't quietly turn the tests below into tautologies.
#[test]
fn format_slot_reads_as_an_iri_after_build() {
    let (ctx, _backend) = chain_with_a_date_property();
    let prop_def = ctx.head().resolve(&iri(MEASURED_ON)).expect("property");
    let slot = prop_def
        .get(&iri("urn:eigenius:core:format"))
        .expect("format slot");
    // This asserted `matches!(slot, Value::ResourceRef(_))` — that a build-time pass had
    // upgraded the parsed string. The pass is gone: it promised readers "one shape per
    // data_type" and could not keep it, because `ResourceRef` encodes to CBOR `Text` and
    // reads back as `String`. What a reader is entitled to is the IRI, through `as_iri`.
    assert_eq!(
        slot.as_iri().map(|i| i.as_str().to_string()).as_deref(),
        Some("urn:eigenius:core:formats:date"),
        "the format slot must read as an IRI, got {slot:?}"
    );
}

/// A malformed date must be rejected **by the commit**, not merely by a
/// helper predicate. Before the fix this commit succeeded silently.
#[test]
fn malformed_date_is_rejected_by_the_commit() {
    let (mut ctx, backend) = chain_with_a_date_property();
    ctx.add_resource(instance("bad", "2026-13-01"))
        .expect("add_resource");
    let working = ctx.take_working("fmt_bad").expect("take_working");
    let err = commit_layer_default(working, ctx.storage().clone(), backend.as_ref())
        .expect_err("a malformed date must not commit");

    let errors = match err {
        CommitError::Validation { errors, .. } => errors,
        other => panic!("expected a validation failure, got {other:?}"),
    };
    let hit = errors
        .iter()
        .find(|e| e.rule == ValidationRule::FormatViolation)
        .unwrap_or_else(|| panic!("no FormatViolation among {errors:?}"));
    assert_eq!(hit.property.as_ref().map(Iri::as_str), Some(MEASURED_ON));
    assert!(
        hit.message.contains("2026-13-01"),
        "diagnostic should name the offending value: {}",
        hit.message
    );
}

/// Positive control: a well-formed date still commits, so the test above
/// is measuring the format check and not a rule that rejects the shape of
/// the fixture.
#[test]
fn well_formed_date_commits() {
    let (mut ctx, backend) = chain_with_a_date_property();
    ctx.add_resource(instance("good", "2026-04-11"))
        .expect("add_resource");
    let working = ctx.take_working("fmt_good").expect("take_working");
    commit_layer_default(working, ctx.storage().clone(), backend.as_ref())
        .expect("a well-formed date commits");
}

/// Rule 4 must still fire when the property definition arrives from a
/// chain rehydrated out of storage rather than from the layer just built.
/// The in-memory backend hands the slot back in its `ResourceRef` shape;
/// the RocksDB backend's CBOR layer normalises `ResourceRef` into
/// `String` (pinned by `value_variants_round_trip_normalizations` in the
/// rocksdb store). `as_iri_str` reads both, so the rule holds across
/// backends.
#[test]
fn format_rule_fires_against_a_reloaded_chain() {
    let (ctx, backend) = chain_with_a_date_property_on(true);
    let head_id = ctx.head().id().clone();
    drop(ctx);

    let info = backend
        .load_chain_from(&head_id)
        .expect("load_chain_from")
        .expect("chain present");
    let reloaded: Arc<Layer> = eigenius_kernel::layer::build_chain(
        info,
        LayerStorage::with_persistent(Arc::clone(&backend)),
    );
    let prop_def = reloaded.resolve(&iri(MEASURED_ON)).expect("property");
    assert_eq!(
        prop_def
            .get(&iri("urn:eigenius:core:format"))
            .and_then(Value::as_iri_str),
        Some("urn:eigenius:core:formats:date"),
        "format slot must survive the reload in one of the two IRI shapes"
    );

    let validator = Validator::new(Arc::clone(&reloaded));
    let errors = validator.validate_resource(&instance("bad", "not-a-date"));
    assert!(
        errors
            .iter()
            .any(|e| e.rule == ValidationRule::FormatViolation),
        "reloaded chain must still enforce the format: {errors:?}"
    );
}
