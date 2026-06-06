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

//! Shared test fixtures for the per-variant test modules under
//! `crate::layer::merge`.
//!
//! The classifier, witness, rename, schema-quotient, restructure,
//! cascade, and merge-layer construction tests all build the same
//! shapes (a chain of `ancestor → branch_a / branch_b` layers, a
//! `MergeSpan` derived from them, optional `MergeComorphism`
//! resources for witness fixtures). Factoring the builders here
//! keeps the per-variant `mod tests` blocks focused on the variant
//! they exercise.

#![cfg(test)]

use super::conflict::MergeSpan;
use super::witnessed::{resolve_merge_comorphism, MergeComorphismHandle};
use crate::layer::{LayerBuilder, LayerStorage};
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use crate::storage::memory::MemoryPersistentBackend;
use crate::storage::PersistentBackend;
use std::sync::Arc;

pub(crate) fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}

/// Build a minimal `Resource` with the given is_a + property map.
pub(crate) fn make_resource(id: &str, is_a: &[&str], props: &[(&str, Value)]) -> Resource {
    let mut r = Resource::new(iri(id));
    let is_a_iri = Iri::parse(wk::IS_A).expect("IS_A IRI");
    let classes: Vec<Value> = is_a.iter().map(|c| Value::ResourceRef(iri(c))).collect();
    r.set(is_a_iri, Value::Array(classes));
    for (k, v) in props {
        r.set(iri(k), v.clone());
    }
    r
}

/// Build a span by committing ancestor / head_a / head_b layers
/// and computing per-head sources via the lattice's existing
/// `iri_sources_since`. Returns the span plus the backend.
pub(crate) fn build_span(
    ancestor_resources: Vec<Resource>,
    branch_a_resources: Vec<Resource>,
    branch_b_resources: Vec<Resource>,
) -> (MergeSpan, MemoryPersistentBackend) {
    let backend = MemoryPersistentBackend::new();
    let storage = LayerStorage::in_memory();

    let mut ab = LayerBuilder::new("ancestor", None);
    for r in ancestor_resources {
        ab.add_resource(r).unwrap();
    }
    let ancestor = Arc::new(ab.build(storage.clone()));
    backend.store_layer(&ancestor).unwrap();

    let mut a_builder = LayerBuilder::new("branch_a", Some(Arc::clone(&ancestor)));
    for r in branch_a_resources {
        a_builder.add_resource(r).unwrap();
    }
    let head_a = Arc::new(a_builder.build(storage.clone()));
    backend.store_layer(&head_a).unwrap();

    let mut b_builder = LayerBuilder::new("branch_b", Some(Arc::clone(&ancestor)));
    for r in branch_b_resources {
        b_builder.add_resource(r).unwrap();
    }
    let head_b = Arc::new(b_builder.build(storage));
    backend.store_layer(&head_b).unwrap();

    let topology = backend.load_topology().unwrap();
    let sources_a =
        crate::lattice::iri_sources_since(head_a.id(), ancestor.id(), &topology, &backend).unwrap();
    let sources_b =
        crate::lattice::iri_sources_since(head_b.id(), ancestor.id(), &topology, &backend).unwrap();

    let span = MergeSpan {
        ancestor: ancestor.id().clone(),
        head_a: head_a.id().clone(),
        head_b: head_b.id().clone(),
        sources_a,
        sources_b,
    };
    (span, backend)
}

/// Same as [`build_span`] but threads an `Arc<MemoryPersistentBackend>`
/// through `LayerStorage::with_persistent` so the apply path's
/// `build_chain` sees the same storage the test commits to.
/// Returns the span, the Arc-backed backend, and the storage
/// the test should pass to `apply_witness_resolution`.
pub(crate) fn build_span_arc(
    ancestor_resources: Vec<Resource>,
    branch_a_resources: Vec<Resource>,
    branch_b_resources: Vec<Resource>,
) -> (
    MergeSpan,
    Arc<MemoryPersistentBackend>,
    crate::layer::LayerStorage,
) {
    let backend: Arc<MemoryPersistentBackend> = Arc::new(MemoryPersistentBackend::new());
    let backend_dyn: Arc<dyn crate::storage::PersistentBackend> = backend.clone();
    let storage = crate::layer::LayerStorage::with_persistent(Arc::clone(&backend_dyn));

    let mut ab = LayerBuilder::new("ancestor", None);
    for r in ancestor_resources {
        ab.add_resource(r).unwrap();
    }
    let ancestor = Arc::new(ab.build(storage.clone()));
    backend.store_layer(&ancestor).unwrap();

    let mut a_builder = LayerBuilder::new("branch_a", Some(Arc::clone(&ancestor)));
    for r in branch_a_resources {
        a_builder.add_resource(r).unwrap();
    }
    let head_a = Arc::new(a_builder.build(storage.clone()));
    backend.store_layer(&head_a).unwrap();

    let mut b_builder = LayerBuilder::new("branch_b", Some(Arc::clone(&ancestor)));
    for r in branch_b_resources {
        b_builder.add_resource(r).unwrap();
    }
    let head_b = Arc::new(b_builder.build(storage.clone()));
    backend.store_layer(&head_b).unwrap();

    let topology = backend.load_topology().unwrap();
    let sources_a =
        crate::lattice::iri_sources_since(head_a.id(), ancestor.id(), &topology, &*backend)
            .unwrap();
    let sources_b =
        crate::lattice::iri_sources_since(head_b.id(), ancestor.id(), &topology, &*backend)
            .unwrap();

    let span = MergeSpan {
        ancestor: ancestor.id().clone(),
        head_a: head_a.id().clone(),
        head_b: head_b.id().clone(),
        sources_a,
        sources_b,
    };
    (span, backend, storage)
}

// ─── Witness fixture helpers ──────────────────────────────────────────────
//
// Used by the witness, cascade, and resolve test modules — every
// witness fixture starts from the same EigenTT lambda shape, so the
// builders live here.

/// Build an embedded-resource body for a EigenTT `Var <name>`
/// expression. Embedded (no `@id`) — `parse_var` reads
/// `program:name` from whatever resource it's handed.
pub(crate) fn make_var_resource(name: &str) -> Resource {
    let mut r = Resource::new_embedded();
    let is_a_iri = Iri::parse(wk::IS_A).unwrap();
    r.set(
        is_a_iri,
        Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:program:Var"))]),
    );
    r.set(
        iri("urn:eigenius:program:name"),
        Value::String(name.to_string()),
    );
    r
}

/// Build an embedded-resource body for a EigenTT
/// `Lambda <param> <body>` expression. `parse_lambda` reads
/// `program:parameter` + `program:body`.
pub(crate) fn make_lambda_resource(param: &str, body: Resource) -> Resource {
    let mut r = Resource::new_embedded();
    let is_a_iri = Iri::parse(wk::IS_A).unwrap();
    r.set(
        is_a_iri,
        Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:program:Lambda"))]),
    );
    r.set(
        iri("urn:eigenius:program:parameter"),
        Value::String(param.to_string()),
    );
    r.set(
        iri("urn:eigenius:program:body"),
        Value::Embedded(Box::new(body)),
    );
    r
}

/// Build a span with a `MergeComorphism` + `λ a. λ b. λ opt. <body>`
/// transformation committed on the ancestor side. Returns the
/// backend wrapped in an `Arc` so the `LayerStorage` and the
/// test's direct backend probes share the same storage instance
/// — without that, `Layer::resolve` walks a parallel empty
/// in-memory backend and finds nothing.
pub(crate) fn build_witness_fixture(
    body: Resource,
) -> (
    MergeSpan,
    Arc<MemoryPersistentBackend>,
    MergeComorphismHandle,
    LayerStorage,
) {
    let transformation_iri = "urn:test:term:identity_b";
    let witness_iri = "urn:test:witness";

    // Three nested Lambdas binding the spec's `a`, `b`, and `opt`
    // (the optional ancestor). Committed at a canonical top-level
    // IRI so `layer.resolve` finds it.
    let inner_opt = make_lambda_resource("opt", body);
    let inner_b = make_lambda_resource("b", inner_opt);
    let transformation = {
        let lam = make_lambda_resource("a", inner_b);
        let mut r = Resource::new(Iri::parse(transformation_iri).unwrap());
        for (k, v) in lam.properties() {
            r.set(k.clone(), v.clone());
        }
        r
    };

    let witness = make_resource(
        witness_iri,
        &[wk::MERGE_COMORPHISM],
        &[
            (
                wk::MERGE_TRANSFORMATION,
                Value::ResourceRef(iri(transformation_iri)),
            ),
            (
                wk::MERGE_TARGET_CLASS,
                Value::ResourceRef(iri("urn:test:Patient")),
            ),
        ],
    );

    let (span, backend, storage) = build_span_arc(
        vec![transformation, witness],
        vec![make_resource(
            "urn:test:patient_42",
            &["urn:test:Patient"],
            &[("urn:test:weight", Value::Integer(75))],
        )],
        vec![make_resource(
            "urn:test:patient_42",
            &["urn:test:Patient"],
            &[("urn:test:weight", Value::Integer(76))],
        )],
    );
    let topology = backend.load_topology().unwrap();
    let handle = resolve_merge_comorphism(
        &iri(witness_iri),
        &iri("urn:test:Patient"),
        &span,
        &[],
        &topology,
        &*backend,
    )
    .unwrap();
    (span, backend, handle, storage)
}

/// D38 §4 off-span fixture. Builds the same IriCollision span as
/// `build_witness_fixture` but commits the `MergeComorphism` +
/// transformation Lambda on a separate `witness-library` branch
/// rooted at the merge span's ancestor — so neither resource is
/// reachable from the merge span itself, and the resolver only
/// finds the comorphism if `extra_branches = ["witness-library"]`
/// is supplied. Registers the branch ref via `put_branch` so the
/// resolver's `backend.get_branch(...)` call returns the tip.
pub(crate) fn build_witness_fixture_offspan(
    body: Resource,
) -> (
    MergeSpan,
    Arc<MemoryPersistentBackend>,
    Iri, // witness IRI (caller passes via extra_branches lookup)
    LayerStorage,
) {
    let transformation_iri = "urn:test:term:identity_b_offspan";
    let witness_iri = "urn:test:witness_offspan";

    let inner_opt = make_lambda_resource("opt", body);
    let inner_b = make_lambda_resource("b", inner_opt);
    let transformation = {
        let lam = make_lambda_resource("a", inner_b);
        let mut r = Resource::new(Iri::parse(transformation_iri).unwrap());
        for (k, v) in lam.properties() {
            r.set(k.clone(), v.clone());
        }
        r
    };
    let witness = make_resource(
        witness_iri,
        &[wk::MERGE_COMORPHISM],
        &[
            (
                wk::MERGE_TRANSFORMATION,
                Value::ResourceRef(iri(transformation_iri)),
            ),
            (
                wk::MERGE_TARGET_CLASS,
                Value::ResourceRef(iri("urn:test:Patient")),
            ),
        ],
    );

    let backend: Arc<MemoryPersistentBackend> = Arc::new(MemoryPersistentBackend::new());
    let backend_dyn: Arc<dyn PersistentBackend> = backend.clone();
    let storage = LayerStorage::with_persistent(Arc::clone(&backend_dyn));

    // Ancestor — empty (no witness, no resources). The branches
    // diverge from here.
    let ancestor = Arc::new(LayerBuilder::new("ancestor", None).build(storage.clone()));
    backend.store_layer(&ancestor).unwrap();

    // Branch A / Branch B — patient_42 with different weights,
    // producing the IriCollision the witness will resolve.
    let mut a_builder = LayerBuilder::new("branch_a", Some(Arc::clone(&ancestor)));
    a_builder
        .add_resource(make_resource(
            "urn:test:patient_42",
            &["urn:test:Patient"],
            &[("urn:test:weight", Value::Integer(75))],
        ))
        .unwrap();
    let head_a = Arc::new(a_builder.build(storage.clone()));
    backend.store_layer(&head_a).unwrap();

    let mut b_builder = LayerBuilder::new("branch_b", Some(Arc::clone(&ancestor)));
    b_builder
        .add_resource(make_resource(
            "urn:test:patient_42",
            &["urn:test:Patient"],
            &[("urn:test:weight", Value::Integer(76))],
        ))
        .unwrap();
    let head_b = Arc::new(b_builder.build(storage.clone()));
    backend.store_layer(&head_b).unwrap();

    // Witness-library — separate sibling branch holding the
    // comorphism + transformation. Not reachable from the merge
    // span (sources_a / sources_b / ancestor's chain).
    let mut lib_builder = LayerBuilder::new("witness-library", Some(Arc::clone(&ancestor)));
    lib_builder.add_resource(transformation).unwrap();
    lib_builder.add_resource(witness).unwrap();
    let lib_head = Arc::new(lib_builder.build(storage.clone()));
    backend.store_layer(&lib_head).unwrap();
    backend
        .put_branch("witness-library", lib_head.id())
        .unwrap();

    let topology = backend.load_topology().unwrap();
    let sources_a =
        crate::lattice::iri_sources_since(head_a.id(), ancestor.id(), &topology, &*backend)
            .unwrap();
    let sources_b =
        crate::lattice::iri_sources_since(head_b.id(), ancestor.id(), &topology, &*backend)
            .unwrap();

    let span = MergeSpan {
        ancestor: ancestor.id().clone(),
        head_a: head_a.id().clone(),
        head_b: head_b.id().clone(),
        sources_a,
        sources_b,
    };
    (span, backend, iri(witness_iri), storage)
}
