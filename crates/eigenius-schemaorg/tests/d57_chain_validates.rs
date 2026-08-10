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

//! The D57 objective chain, kernel-type-checked end to end
//! (`experiments/objectives/d57-schema-org/chain/`). Builds
//! core → reflection → reasoning → reference → objective → 00…05 and replicates
//! the live AutoOnLoad gate: every `reasoning:ReasoningSentence` each layer adds
//! MUST validate to `Holds`, else the live loader would reject the layer and a
//! downstream lemma citation of it would be unsound.
//!
//! Until now the chain was validated only by loading it through the CLI/server.
//! This harness makes the chain's correctness itself a `cargo test` witness — so
//! an edit to a milestone certificate (e.g. the m3/m4 mechanical-evidence
//! re-discharge) cannot regress silently. Mirrors the proven
//! `eigenius-reasoning/tests/wrn_phase2.rs` harness.

use std::sync::Arc;

use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_reasoning::validate::do_validate_justification;
use eigenius_reasoning::ReasoningInstitution;

/// Build a layer from ESL against its parent, then assert every ReasoningSentence
/// it adds validates to `Holds` (the live AutoOnLoad gate).
fn esl_against(source: &str, parent: &Arc<Layer>, name: &str) -> Arc<Layer> {
    let resources = esl::compile_against_layer(source, parent).unwrap_or_else(|errs| {
        panic!(
            "{name} failed to compile:\n{}",
            errs.into_iter()
                .map(|e| format!("  - {e:?}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
    let mut b = LayerBuilder::new(name, Some(parent.clone()));
    for r in &resources {
        b.add_resource(r.clone()).unwrap();
    }
    let layer = Arc::new(b.build(LayerStorage::in_memory()));

    let ctx = ExecutionContext::new(
        layer.clone(),
        name,
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    );
    let inst = ReasoningInstitution::new();
    let sentence_class = "urn:eigenius:reasoning:ReasoningSentence";
    for r in &resources {
        if !r.is_a().iter().any(|c| c.as_str() == sentence_class) {
            continue;
        }
        let iri = r.id().map(|i| i.as_str().to_string()).unwrap_or_default();
        let outcome =
            do_validate_justification(&inst, r, &ctx).expect("validate handler returns outcome");
        let ctor = outcome
            .output
            .get(&Iri::parse(wk::CTOR_NAME).unwrap())
            .and_then(Value::as_str)
            .unwrap_or("<none>");
        if ctor != wk::VERDICT_HOLDS {
            let diag = outcome
                .output
                .get(&Iri::parse("urn:eigenius:institution:diagnostic").unwrap())
                .and_then(Value::as_str)
                .unwrap_or("");
            panic!(
                "esl_against({name}): conclusion `{iri}` did not Hold (got {ctor}) — the live \
                 AutoOnLoad gate would reject this layer. diagnostic: {diag}"
            );
        }
    }
    layer
}

fn json_layer(name: &str, parent: Option<Arc<Layer>>, sources: &[&str]) -> Arc<Layer> {
    let mut b = LayerBuilder::new(name, parent);
    for src in sources {
        for r in eigon_json::parse_document(src).expect("ontology parses") {
            b.add_resource(r).expect("ontology resource adds");
        }
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

fn assert_holds(ctx: &ExecutionContext, inst: &ReasoningInstitution, iri: &str) {
    let sentence = (*ctx
        .resolve(&Iri::parse(iri).expect("sentence IRI"))
        .unwrap_or_else(|| panic!("sentence `{iri}` should be on the chain")))
    .clone();
    let outcome =
        do_validate_justification(inst, &sentence, ctx).expect("validate handler returns outcome");
    let ctor = outcome
        .output
        .get(&Iri::parse(wk::CTOR_NAME).unwrap())
        .and_then(Value::as_str)
        .expect("verdict carries ctor_name")
        .to_string();
    let diagnostic = outcome
        .output
        .get(&Iri::parse("urn:eigenius:institution:diagnostic").unwrap())
        .and_then(Value::as_str)
        .map(str::to_owned);
    assert_eq!(
        ctor,
        wk::VERDICT_HOLDS,
        "expected Holds for `{iri}`; got {ctor}, diagnostic: {diagnostic:?}"
    );
}

#[test]
fn d57_objective_chain_validates() {
    let core = json_layer(
        "core",
        None,
        &[include_str!("../../../ontologies/core/core-ontology.json")],
    );
    let reflection = json_layer(
        "reflection",
        Some(core),
        &[
            include_str!("../../../ontologies/reflection/reflection-ontology.json"),
            include_str!("../../../ontologies/eigentt/eigentt-type-fragment.json"),
            include_str!("../../../ontologies/institution/institution-ontology.json"),
            include_str!("../../../ontologies/ingest/ingest-ontology.json"),
        ],
    );
    let reasoning = {
        let mut b = LayerBuilder::new("reasoning", Some(reflection));
        for r in esl::compile(include_str!("../../../ontologies/reasoning/reasoning.esl"))
            .expect("reasoning.esl compiles")
        {
            b.add_resource(r).unwrap();
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };
    let reference = esl_against(
        include_str!("../../../ontologies/reference/reference.esl"),
        &reasoning,
        "reference",
    );
    let objective = esl_against(
        include_str!("../../../ontologies/objective/objective-ontology.esl"),
        &reference,
        "objective",
    );

    let l00 = esl_against(
        include_str!("../../../experiments/objectives/d57-schema-org/chain/00-objective.esl"),
        &objective,
        "d57-00-objective",
    );
    let l01 = esl_against(
        include_str!("../../../experiments/objectives/d57-schema-org/chain/01-discipline.esl"),
        &l00,
        "d57-01-discipline",
    );
    let l02 = esl_against(
        include_str!("../../../experiments/objectives/d57-schema-org/chain/02-objective-typed.esl"),
        &l01,
        "d57-02-objective-typed",
    );
    let l03 = esl_against(
        include_str!("../../../experiments/objectives/d57-schema-org/chain/03-probe.esl"),
        &l02,
        "d57-03-probe",
    );
    // 04a-evidence: the pins (gen_input/gen_output) + rule + cut accounting, loaded
    // before the run (live: `gen_input` is then committed for the program to consume).
    let l04a = esl_against(
        include_str!("../../../experiments/objectives/d57-schema-org/chain/04a-evidence.esl"),
        &l03,
        "d57-04a-evidence",
    );
    // Model what `eigenius run` commits when the generator runs through the D60 `oci`
    // runtime: the worker's conversion-report DerivedResource (carrying
    // canonical_proposition = GeneratorConforms("schema_org"), built by the real
    // report builder) + a ProgramTrace over it. The witness index mints
    // IsDerivedAs(generate_result, GeneratorConforms), which 04b's concl_generator
    // discharges via derived(...). The live compose run commits these for real; this
    // makes the chain's Level-2 certificate cargo-test verifiable.
    let run_layer = {
        let result = eigenius_schemaorg::report::build_report(
            "urn:eigenius:obj:d57:generate_result",
            "0f0c97a4f666b2f8563573fe48453782fd51b87a504523cf0c9aff6a71c3eec4",
            "f4de231a3e32247509b000801e88a026a874bf3bf5a872a758f2227c5598c3fb",
            &eigenius_schemaorg::Coverage::default(),
        );
        let mut trace =
            Resource::new(Iri::parse("urn:eigenius:obj:d57:generate_result_trace").unwrap());
        trace.set(
            Iri::parse("urn:eigenius:core:is_a").unwrap(),
            Value::Array(vec![Value::ResourceRef(
                Iri::parse("urn:eigenius:reflection:ProgramTrace").unwrap(),
            )]),
        );
        trace.set(
            Iri::parse("urn:eigenius:reflection:resource").unwrap(),
            Value::ResourceRef(Iri::parse("urn:eigenius:obj:d57:generate_result").unwrap()),
        );
        trace.set(
            Iri::parse("urn:eigenius:reflection:source").unwrap(),
            Value::String("eigenius run: oci RunRuntimeScript -> eigenius-schemaorg-worker".into()),
        );
        trace.set(
            Iri::parse("urn:eigenius:reflection:timestamp").unwrap(),
            Value::String("2026-06-20T00:00:00Z".into()),
        );
        let mut b = LayerBuilder::new("d57-run", Some(l04a));
        b.add_resource(result).unwrap();
        b.add_resource(trace).unwrap();
        Arc::new(b.build(LayerStorage::in_memory()))
    };
    let l04 = esl_against(
        include_str!("../../../experiments/objectives/d57-schema-org/chain/04b-conclusions.esl"),
        &run_layer,
        "d57-04b-conclusions",
    );
    let l05 = esl_against(
        include_str!("../../../experiments/objectives/d57-schema-org/chain/05-synthesis.esl"),
        &l04,
        "d57-05-synthesis",
    );

    let ctx = ExecutionContext::new(
        l05,
        "d57-05-synthesis",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    );
    let inst = ReasoningInstitution::new();

    // The five milestone/thesis conclusions all Hold.
    for iri in [
        "urn:eigenius:obj:d57:concl_discipline",
        "urn:eigenius:obj:d57:concl_probe",
        "urn:eigenius:obj:d57:concl_generator",
        "urn:eigenius:obj:d57:concl_cut",
        "urn:eigenius:obj:d57:concl_main",
    ] {
        assert_holds(&ctx, &inst, iri);
    }
}
