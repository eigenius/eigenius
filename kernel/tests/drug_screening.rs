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

//! End-to-end D39 / D49 / D52 demo: a drug-screening scenario authored
//! entirely in ESL via the `type_expr(...)` surface.
//!
//! The fixture at [`tests/fixtures/drug_screening.esl`](fixtures/drug_screening.esl)
//! commits five artifacts:
//!
//! 1. Domain vocabulary (`HasLowIC50`, `StrongInhibitor` predicates),
//!    both marked `is_a stats:PopulationLevel` per D52 §7.4 so the
//!    statistics institution's epistemic-scope check admits the claim
//!    under BiologicalReplication.
//! 2. A literature rule as a `DeclaredResource` + `DeclarationTrace`,
//!    with `canonical_proposition` = `HasLowIC50(EIG_0291) ->
//!    StrongInhibitor(EIG_0291)`.
//! 3. A `stats:SampleSetResource` carrying the three raw IC50
//!    replicate readings (72, 85, 100 nM) + an `ObservationTrace`.
//! 4. A `stats:StatisticalAnalysisPlan` referencing the SampleSet with the
//!    universal-claim schema (alpha = 0.05, effect_size = Absolute(100,
//!    "nM"), TwoSided, WelchUnequal, Identity exclusion) + a
//!    `ProgramTrace`. The claim's `canonical_proposition` is
//!    `HasLowIC50(EIG_0291)` — the proposition the computed ground exposes
//!    to D39 reasoning.
//! 5. A `justification:Conclusion` claiming `StrongInhibitor(EIG_0291)`,
//!    justified by `App(Declared(rule), App(Declared(plan), Observed(s)))`,
//!    with a `justification:Certificate.app` certificate composing
//!    `justification:Certificate.declared` + `justification:Certificate.derived`.
//!
//! This is the modernization of the original fixture, which committed
//! the bench measurement as a plain `reflection:ObservedResource` whose
//! `canonical_proposition` was a methodological assertion ("85.0 < 100
//! ⇒ HasLowIC50 holds") and cited it via `Observed`. The D52
//! statistics institution turns that author-asserted bridge into a
//! mechanical recomputation: the SampleSet carries the raw replicates,
//! the StatisticalAnalysisPlan asserts the parameters, and the verifier
//! computes the proposition. The justification:Conclusion then cites the
//! claim as `App(Declared(plan), Observed(sampleset))` and inherits its
//! auditable provenance.
//!
//! This test compiles the fixture, builds the layer chain (core →
//! reflection → reasoning → statistics → fixture), walks the D49
//! witness index, runs the D39 ValidateJustification handler, and
//! asserts `Verdict::Holds`. The statistics institution itself is not
//! registered in the test runtime — D49 §6 admits `IsDeclaredAs` from
//! the StatisticalAnalysisPlan's ProgramTrace + canonical_proposition pair,
//! independent of whether AutoOnLoad has fired the verifier. The
//! verifier's recomputation path is exercised separately in the
//! eigenius-statistics crate's tests.

use std::sync::Arc;

use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;

/// Stand up the standard reasoning chain (core → reflection → eigentt
/// → institution → reasoning → statistics) plus a user layer compiled
/// from the drug-screening fixture. The fixture's `type_expr(...)`
/// certificates reference reasoning-layer ctors (`app`, `declared`,
/// `observed`, `App`, `Declared`, `Observed`) and its
/// resource bodies reference statistics-layer smart constructors
/// (`stats:SingleSampleEstimate(...)`, `BiologicalReplication()`,
/// `Absolute(...)`, etc.), so the fixture must be compiled with
/// [`esl::compile_against_layer`] — that seeds the compiler's ctor
/// table from the full parent chain.
fn build_drug_screening_chain() -> ExecutionContext {
    let core_json = include_str!("../../ontologies/core/core-ontology.json");
    let core_resources = eigon_json::parse_document(core_json).unwrap();
    let mut core_builder = LayerBuilder::new("core", None);
    for r in core_resources {
        core_builder.add_resource(r).unwrap();
    }
    let core = Arc::new(core_builder.build(LayerStorage::in_memory()));

    let reflection_json = include_str!("../../ontologies/reflection/reflection-ontology.json");
    let reflection_resources = eigon_json::parse_document(reflection_json).unwrap();
    let mut reflection_builder = LayerBuilder::new("reflection", Some(core));
    for r in reflection_resources {
        reflection_builder.add_resource(r).unwrap();
    }
    let eigentt_json = include_str!("../../ontologies/eigentt/eigentt-type-fragment.json");
    let eigentt_resources = eigon_json::parse_document(eigentt_json).unwrap();
    for r in eigentt_resources {
        reflection_builder.add_resource(r).unwrap();
    }
    let institution_json = include_str!("../../ontologies/institution/institution-ontology.json");
    let institution_resources = eigon_json::parse_document(institution_json).unwrap();
    for r in institution_resources {
        reflection_builder.add_resource(r).unwrap();
    }
    let reflection = Arc::new(reflection_builder.build(LayerStorage::in_memory()));

    let reasoning_source = include_str!("../../ontologies/justification/justification.esl");
    let reasoning_resources =
        esl::compile(reasoning_source, &eigenius_kernel::layer::Layer::empty())
            .expect("reasoning.esl compiles");
    // `prov` (P5). These fixtures carry `prov:` properties and trace classes; without this
    // layer none of them resolve, and the chain reports a dozen `UnresolvedClassReference`s
    // that no assertion here was looking at. Sits above `reflection` and below
    // `justification`, matching `BOOTSTRAP_CHAIN`.
    let prov_resources = esl::compile(include_str!("../../ontologies/prov/prov.esl"), &reflection)
        .expect("prov.esl compiles");
    let mut prov_builder = LayerBuilder::new("prov", Some(reflection));
    for r in prov_resources {
        prov_builder.add_resource(r).unwrap();
    }
    let prov = Arc::new(prov_builder.build(LayerStorage::in_memory()));

    let mut reasoning_builder = LayerBuilder::new("reasoning", Some(prov));
    for r in reasoning_resources {
        reasoning_builder.add_resource(r).unwrap();
    }
    let reasoning = Arc::new(reasoning_builder.build(LayerStorage::in_memory()));

    // Statistics layer — provides the SampleSet / StatisticalAnalysisPlan /
    // axis-enum machinery the fixture's SampleSetResource and
    // StatisticalAnalysisPlan reference. Compiled against `reasoning` so the
    // statistics ontology can pick up reasoning-layer types if it ever
    // needs to (it currently doesn't, but parking it above keeps the
    // dependency direction honest: reasoning is a sibling of
    // statistics, both above reflection).
    let stats_source = include_str!("../../ontologies/statistics/statistics.esl");
    let stats_resources = esl::compile(stats_source, &reasoning)
        .expect("statistics.esl compiles against reasoning layer");
    let mut stats_builder = LayerBuilder::new("statistics", Some(reasoning));
    for r in stats_resources {
        stats_builder.add_resource(r).unwrap();
    }
    let stats_layer = Arc::new(stats_builder.build(LayerStorage::in_memory()));

    // The fixture compiles AGAINST the statistics layer so its
    // `type_expr(...)` bodies can reference reasoning-layer ctors
    // (`app`, `declared`, `derived`, `App`, `Declared`,
    // `Observed`) AND statistics-layer smart constructors
    // (`SingleSampleEstimate`, `BiologicalReplication`, `Absolute`,
    // `TwoSided`, `WelchUnequal`, `Identity`) by their short names.
    // The ctor table seed walks the full chain unambiguously per
    // gh #75's IRI-discipline split.
    let fixture_source = include_str!("fixtures/drug_screening.esl");
    let fixture_resources = esl::compile(fixture_source, &stats_layer).unwrap_or_else(|errs| {
        panic!(
            "drug_screening.esl failed to compile: {}",
            errs.into_iter()
                .map(|e| format!("{e:?}"))
                .collect::<Vec<_>>()
                .join("; ")
        )
    });
    let mut fixture_builder = LayerBuilder::new("drug-screening-demo", Some(stats_layer));
    for r in fixture_resources {
        fixture_builder.add_resource(r).unwrap();
    }
    let fixture_layer = Arc::new(fixture_builder.build(LayerStorage::in_memory()));

    // Force the witness index to populate from the three trace
    // resources the fixture committed: the rule's DeclarationTrace
    // admits `IsDeclaredAs(rule_iri, rule_prop)`; the SampleSet's
    // ObservationTrace admits `IsObservedAs(sampleset_iri, …)`; and
    // the StatisticalAnalysisPlan's ProgramTrace admits
    // `IsDeclaredAs(plan_iri, Asserts(s) -> lt(mean_of(s), 100.0))` — the witness
    // the certificate's `derived(...)` constructor consumes.

    ExecutionContext::new(
        fixture_layer,
        "drug-screening-demo",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    )
}

#[test]
fn drug_screening_scenario_validates_to_holds() {
    let ctx = build_drug_screening_chain();

    // Fetch the justification:Conclusion the fixture authored, by IRI.
    let sentence_iri =
        Iri::parse("urn:eigenius:demo:screen:concl_eig0291_strong").expect("sentence IRI");
    ctx.resolve(&sentence_iri)
        .unwrap_or_else(|| panic!("sentence `{sentence_iri}` should be on the chain"));

    // The check is Rule 21's since P2 moved it to commit: it decodes the judgement, checks
    // its `type` is a type, and checks its `term` against it. Read the errors it reports for
    // this conclusion; an empty set is what the handler used to report as `Holds`.
    let diagnostic = eigenius_kernel::validation::Validator::new(ctx.head().clone())
        .validate()
        .into_iter()
        .filter(|e| e.resource_id.as_ref().is_some_and(|i| *i == sentence_iri))
        .map(|e| e.message)
        .collect::<Vec<_>>()
        .join(
            "
",
        );

    assert!(diagnostic.is_empty(), "expected Holds; got: {diagnostic}");
}
