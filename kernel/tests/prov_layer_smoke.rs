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

//! The `prov` bootstrap layer seeds and every declaration in it resolves.
//!
//! `prov` carries the provenance axis — Agent, Activity, the four provenance
//! Traces, and the relations between them — split out of `reflection` because
//! that ontology had come to hold two unrelated families under one word:
//! `reflection:Trace` with `LetTrace` / `MapTrace` / `CaseTrace` records how a
//! PROGRAM EVALUATED, while the parentless `DeclarationTrace` /
//! `ObservationTrace` / `ProductionTrace` / `VerificationTrace` record HOW A
//! RESOURCE CAME TO EXIST.
//!
//! The layer sits ABOVE `reflection` and that direction is forced:
//! `prov:ProgramTrace` points into the evaluation family through
//! `prov:trace_tree` and `reflection:output`, and nothing in `reflection` points
//! back.

use eigenius_kernel::ontology::iri::Iri;

#[test]
fn every_prov_declaration_resolves() {
    let ctx = eigenius_kernel::bootstrap::bootstrap().expect("bootstrap seeds");
    for iri in [
        // agents
        "urn:eigenius:prov:Agent",
        "urn:eigenius:prov:Person",
        "urn:eigenius:prov:Organization",
        "urn:eigenius:prov:agent:unattributed",
        "urn:eigenius:prov:agent:eigenius_core_team",
        // activities
        "urn:eigenius:prov:Activity",
        "urn:eigenius:prov:used",
        "urn:eigenius:prov:was_associated_with",
        "urn:eigenius:prov:started_at",
        "urn:eigenius:prov:completed_at",
        // the core relations
        "urn:eigenius:prov:was_attributed_to",
        "urn:eigenius:prov:was_generated_by",
        "urn:eigenius:prov:had_primary_source",
        "urn:eigenius:prov:rationale",
        "urn:eigenius:prov:timestamp",
        // traces
        "urn:eigenius:prov:Trace",
        "urn:eigenius:prov:DeclarationTrace",
        "urn:eigenius:prov:ObservationTrace",
        "urn:eigenius:prov:ProductionTrace",
        "urn:eigenius:prov:ProgramTrace",
        "urn:eigenius:prov:VerificationTrace",
        "urn:eigenius:prov:resource",
        "urn:eigenius:prov:proof_system",
        "urn:eigenius:prov:proof_term",
        "urn:eigenius:prov:trace_tree",
    ] {
        let parsed = Iri::parse(iri).expect("well-formed IRI");
        assert!(
            ctx.resolve(&parsed).is_some(),
            "the prov layer must resolve `{iri}`"
        );
    }
}

/// A run's output carries an `ObservationTrace`, and the chain admits `Observed` for it.
///
/// This pins the contract between what `server::programs::execute_program` emits and what
/// `witness_index::emit_from_trace` reads — the part of kernel-run-records §2 that can
/// silently break. `execute_program` is `pub(super)` behind the gRPC service, so the trace
/// is built here exactly as that code builds it: `is_a: [prov:ObservationTrace]`,
/// `prov:resource` at the output, `prov:was_generated_by` at the run activity, and
/// `prov:timestamp` — the three the class requires.
///
/// The output carries no `reflection:canonical_proposition`, so the witness keys on D39
/// §4.1's default `Asserts(iri)`, which is what an unannotated program output asserts.
///
/// **Why `Observed` and not nothing.** A run's outcome is *sampled*: the paper's criterion
/// is whether the plan formalizes a deterministic function, and nothing asserts that —
/// 0 of 21 `stats:StatisticalAnalysisPlan` resources carry a `DeclarationTrace`. The
/// `ProgramTrace` beside this one stays provenance and grounds nothing.
#[test]
fn a_run_output_with_an_observation_trace_admits_observed() {
    use eigenius_kernel::layer::{lookup_chain_witness, LayerBuilder, LayerStorage};
    use eigenius_kernel::ontology::resource::{Resource, Value};
    use eigenius_kernel::witness::{WitnessCategory, WitnessKey};
    use std::sync::Arc;

    let ctx = eigenius_kernel::bootstrap::bootstrap().expect("bootstrap seeds");
    let mut b = LayerBuilder::new("run-output", Some(Arc::clone(ctx.head())));

    let out_iri = Iri::parse("urn:eigenius:test:run:output").unwrap();
    b.add_resource(Resource::new(out_iri.clone()))
        .expect("add the run output");

    let mut obs = Resource::new(Iri::parse("urn:eigenius:trace:exec-t:observed").unwrap());
    obs.set(
        Iri::parse("urn:eigenius:core:is_a").unwrap(),
        Value::Array(vec![Value::String(
            "urn:eigenius:prov:ObservationTrace".to_string(),
        )]),
    );
    obs.set(
        Iri::parse("urn:eigenius:prov:resource").unwrap(),
        Value::iri(&out_iri),
    );
    obs.set(
        Iri::parse("urn:eigenius:prov:was_generated_by").unwrap(),
        Value::String("urn:eigenius:prov:activity:kernel_run_program".to_string()),
    );
    obs.set(
        Iri::parse("urn:eigenius:prov:timestamp").unwrap(),
        Value::String("2026-09-04T00:00:00.000Z".to_string()),
    );
    b.add_resource(obs).expect("add the observation trace");

    let layer = Arc::new(b.build(LayerStorage::in_memory()));

    let prop_hash = eigenius_kernel::layer::default_asserts_proposition_hash(&layer, &out_iri)
        .expect("the default Asserts(iri) proposition hashes");
    let key = WitnessKey {
        category: WitnessCategory::Observed,
        iri: out_iri,
        prop_hash,
    };

    assert!(
        lookup_chain_witness(&layer, &key),
        "an ObservationTrace on a run's output must admit `Observed` — this is the leaf a \
         sampled outcome is owed (kernel-run-records §2)"
    );
}
