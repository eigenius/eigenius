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

//! Phase 20a.4 capstone: a hand-crafted `LeanProofTerm` carrying the
//! vendored toy `PUnit` export lands as *verified* — AutoOnLoad fires
//! `qc_proof_check`, the `LeanInstitution` dispatches via 20a.3's
//! `check_proof`, and the kernel records a `Verdict::Holds` for the
//! resource (per D31 §6.3's commit semantics).
//!
//! This is the integration shape D28 §11.1 commits to: the
//! verification verdict is a direct Rust function call inside the
//! kernel process — no IPC, no orchestrator round-trip. The test
//! drives the kernel-side dispatch path
//! (`dispatch_auto_on_load_for_resource`) so it doesn't need a live
//! gRPC server, but the institution + index + chain shape is exactly
//! what `cmd_serve` wires up at process startup.

use std::sync::Arc;

use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::institution::dispatch::{dispatch_auto_on_load_for_resource, VerdictReading};
use eigenius_kernel::institution::registry::InstitutionIndex;
use eigenius_kernel::institution::runtime::{Institution, InstitutionRuntime};
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;

use eigenius_lean::institution::iris as lean_iris;
use eigenius_lean::LeanInstitution;

const TOY_HOLDS: &[u8] = include_bytes!("../test_resources/toy_proof_holds.json");
const TOY_FAILS: &[u8] = include_bytes!("../test_resources/toy_proof_fails.json");

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("static IRI must parse")
}

/// Build a top-of-chain layer carrying a `LeanProofPayload` (the
/// `bytes_str` content) and a `LeanProofTerm` referencing it with
/// the given `target_name`. Returns the storage handle + the
/// committed layer so the caller can build an `ExecutionContext`.
fn build_proof_term_layer(
    bytes_str: &str,
    target_name: &str,
) -> (LayerStorage, Arc<eigenius_kernel::layer::Layer>) {
    // Anchor at the bootstrap chain head — that's where
    // `lean-institution` lives (the ontology declaring
    // `LeanProofTerm`, `LeanProofPayload`, and `qc_proof_check`).
    let ctx = eigenius_kernel::bootstrap::bootstrap().expect("bootstrap");
    let parent = Arc::clone(ctx.head());
    let storage = LayerStorage::in_memory();

    let mut builder = LayerBuilder::new("test_lean_proof_smoke", Some(parent));

    let payload_iri = "urn:eigenius:test:lean:p1_payload";
    let term_iri = "urn:eigenius:test:lean:p1_term";

    // LeanProofPayload — carries the export bytes as a UTF-8 string.
    let mut payload = Resource::new(iri(payload_iri));
    payload.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(
            "urn:eigenius:lean:LeanProofPayload",
        ))]),
    );
    payload.set(
        iri(lean_iris::PROP_PAYLOAD_BYTES),
        Value::String(bytes_str.to_string()),
    );
    builder.add_resource(payload).expect("add payload");

    // LeanProofTerm — references the payload by IRI and names the
    // target theorem. `proposition` / `mirror_iri` / `claim_iri` are
    // recommended-not-required; we omit them here because they're
    // unread on the verification path (correspondence is stubbed
    // until 20a.7).
    let mut term = Resource::new(iri(term_iri));
    term.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(
            "urn:eigenius:lean:LeanProofTerm",
        ))]),
    );
    term.set(
        iri(lean_iris::PROP_PROOF_PAYLOAD),
        Value::ResourceRef(iri(payload_iri)),
    );
    term.set(
        iri(lean_iris::PROP_TARGET_NAME),
        Value::String(target_name.to_string()),
    );
    builder.add_resource(term).expect("add term");

    let layer = Arc::new(builder.build(storage.clone()));
    (storage, layer)
}

/// Construct an `ExecutionContext` + populated `InstitutionIndex` +
/// `InstitutionRuntime` with `LeanInstitution` registered. This is
/// the kernel-side dispatch surface — what `cmd_serve` wires up
/// before the gRPC listener starts.
fn build_dispatch_setup(
    layer: Arc<eigenius_kernel::layer::Layer>,
    storage: LayerStorage,
) -> (
    Arc<InstitutionIndex>,
    Arc<InstitutionRuntime>,
    ExecutionContext,
) {
    let (index, errors) = InstitutionIndex::from_layer(&layer);
    assert!(
        errors.is_empty(),
        "InstitutionIndex from bootstrap + smoke layer must build cleanly; got {errors:?}"
    );
    let index = Arc::new(index);

    let mut runtime = InstitutionRuntime::new();
    let lean: Arc<dyn Institution> = LeanInstitution::arc();
    runtime
        .register(Box::new(Arc::clone(&lean)))
        .expect("register LeanInstitution");
    let runtime = Arc::new(runtime);

    let ctx = ExecutionContext::new(
        Arc::clone(&layer),
        "test",
        ExecutionMode::ReadWrite,
        storage,
    );
    (index, runtime, ctx)
}

#[test]
fn lean_proof_term_with_well_typed_payload_lands_holds() {
    let bytes_str = std::str::from_utf8(TOY_HOLDS).expect("toy proof fixture must be valid UTF-8");
    let (storage, layer) = build_proof_term_layer(bytes_str, "PUnit");
    let (index, runtime, ctx) = build_dispatch_setup(Arc::clone(&layer), storage);

    // Pull the LeanProofTerm back off the committed layer so the
    // AutoOnLoad dispatch sees the canonicalised shape (e.g. `is_a`
    // entries as `ResourceRef`s, not raw `String`s).
    let term_iri = iri("urn:eigenius:test:lean:p1_term");
    let term = layer
        .resolve(&term_iri)
        .expect("LeanProofTerm must resolve on the committed layer");

    let outcome = dispatch_auto_on_load_for_resource(&term, &index, &runtime, &ctx);

    assert!(
        outcome.errors.is_empty(),
        "AutoOnLoad on a well-typed LeanProofTerm must not surface errors; got {:?}",
        outcome.errors
    );
    assert_eq!(
        outcome.dispatches.len(),
        1,
        "exactly one qc_proof_check dispatch should fire on a LeanProofTerm; got {} dispatches",
        outcome.dispatches.len()
    );
    let dispatch = &outcome.dispatches[0];
    assert!(
        matches!(dispatch.verdict, VerdictReading::Holds),
        "well-typed proof must yield Verdict::Holds; got {:?}",
        dispatch.verdict
    );
    assert!(
        dispatch.partial_invocation.is_none(),
        "in-process institution must not populate partial_invocation"
    );
}

#[test]
fn lean_proof_term_with_broken_payload_lands_fails() {
    // Same dispatch path, but the payload is the known-broken
    // `ProjFromProp` export. nanoda panics during checking; the
    // institution's panic-catch in `check_proof` (20a.3) converts
    // that into a `Verdict::Fails` carrying the diagnostic.
    let bytes_str = std::str::from_utf8(TOY_FAILS).expect("toy proof fixture must be valid UTF-8");
    let (storage, layer) = build_proof_term_layer(bytes_str, "explosion");
    let (index, runtime, ctx) = build_dispatch_setup(Arc::clone(&layer), storage);

    let term_iri = iri("urn:eigenius:test:lean:p1_term");
    let term = layer
        .resolve(&term_iri)
        .expect("LeanProofTerm must resolve on the committed layer");

    let outcome = dispatch_auto_on_load_for_resource(&term, &index, &runtime, &ctx);

    assert!(
        outcome.errors.is_empty(),
        "Fails verdicts are well-formed dispatches, not errors; got {:?}",
        outcome.errors
    );
    assert_eq!(outcome.dispatches.len(), 1);
    assert!(
        matches!(outcome.dispatches[0].verdict, VerdictReading::Fails),
        "broken proof must yield Verdict::Fails; got {:?}",
        outcome.dispatches[0].verdict
    );
}
