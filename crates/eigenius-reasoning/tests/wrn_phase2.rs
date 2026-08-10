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

//! WRN Phase-2 wet-lab validation chain — the Declared experimental-design
//! reasoning (the rescue / control logic), kernel-type-checked.
//!
//! Builds core → reflection → reasoning → statistics → bench-core → harness
//! → onco → wrn-phase1-recompute-{plans,conclusions} → wrn-phase1 →
//! wrn-phase2, then runs
//! ValidateJustification on the four Phase-2 conclusions and asserts Holds:
//! - C-VAL `SelectiveViabilityDependence(WRN, MSI)`
//! - D-ONTARGET `OnTarget(WRN, MSI_viability)` (sgWRN-EIJ rescue logic)
//! - D-HELICASE `RequiresActivity(WRN, helicase)` (K577M fails to rescue)
//! - D-HELICASE `DispensableActivity(WRN, exonuclease)` (E84A rescues)
//!
//! Phase-2 statistics are linked-external (the authors' wet-lab assays,
//! recorded as bench:ToolArtifacts with ProgramTrace → IsDerivedAs); the
//! Declared rules lift those readouts into the conclusions. No statistics
//! institution is needed here — the warrants are Declared + linked-external.

use std::sync::Arc;

use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::Value;
use eigenius_kernel::ontology::well_known as wk;
use eigenius_reasoning::validate::do_validate_justification;
use eigenius_reasoning::ReasoningInstitution;

/// The kernel-recomputed (statistics-institution) conclusions: their
/// `DerivedEvidence` witnesses are emitted by the statistics institution's
/// AutoOnLoad, which this reasoning-only harness does not run, so they cannot
/// validate in-process. They are validated for real in
/// `eigenius-statistics/tests/wrn_phase1_recompute.rs`. Listed `pending` here so
/// the gate tolerates them while still enforcing every other conclusion.
const STATS_RECOMPUTED: &[&str] = &[
    "urn:eigenius:pub:wrn:concl_wrn_selective_recomputed",
    "urn:eigenius:pub:wrn:concl_refine_recomputed",
    "urn:eigenius:pub:wrn:concl_lineage_mutator_recomputed",
    "urn:eigenius:pub:wrn:concl_coloc_recomputed",
    "urn:eigenius:pub:wrn:concl_apop_shrna_recomputed",
    "urn:eigenius:pub:wrn:concl_hcr_recomputed",
    "urn:eigenius:pub:wrn:concl_recq_recomputed",
    "urn:eigenius:pub:wrn:concl_biomarker_recomputed",
    "urn:eigenius:pub:wrn:concl_p53_modulates",
    "urn:eigenius:pub:wrn:concl_val_recomputed",
    "urn:eigenius:pub:wrn:concl_cellcycle_recomputed",
    "urn:eigenius:pub:wrn:concl_apoptosis_recomputed",
    "urn:eigenius:pub:wrn:concl_mmr_restoration_recomputed",
    "urn:eigenius:pub:wrn:concl_rescue_wt_recomputed",
    "urn:eigenius:pub:wrn:concl_rescue_e84a_recomputed",
];

fn esl_against(source: &str, parent: &Arc<Layer>, name: &str) -> Arc<Layer> {
    esl_against_pending(source, parent, name, &[])
}

/// Build a layer from ESL, then replicate the live commit pipeline's AutoOnLoad
/// gate: every `reasoning:ReasoningSentence` this layer adds MUST validate to
/// `Holds`, else the live loader would reject the layer (so a downstream lemma
/// citation of it would be unsound). Panics on a non-`Holds` sentence unless its
/// IRI is in `pending` — the documented exceptions whose witnesses are produced
/// out of band (the R runtime, or the statistics institution's AutoOnLoad, which
/// this in-process reasoning harness does not run). Without this gate a layer
/// could commit a never-validated conclusion (e.g. one whose rule references an
/// unloaded ontology) and a later sentence would trust it by IRI — the gap that
/// let wrn_phase5 pass without wrn-literature.
fn esl_against_pending(
    source: &str,
    parent: &Arc<Layer>,
    name: &str,
    pending: &[&str],
) -> Arc<Layer> {
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
        if ctor != wk::VERDICT_HOLDS && !pending.iter().any(|p| *p == iri) {
            let diag = outcome
                .output
                .get(&Iri::parse("urn:eigenius:institution:diagnostic").unwrap())
                .and_then(Value::as_str)
                .unwrap_or("");
            panic!(
                "esl_against({name}): conclusion `{iri}` did not Hold (got {ctor}) — the live \
                 AutoOnLoad gate would reject this layer, so a downstream lemma citation of it \
                 would be unsound. diagnostic: {diag}\n  If its witness is produced out of band \
                 (R runtime / statistics institution AutoOnLoad, not run in this harness), add \
                 its IRI to `pending`."
            );
        }
    }
    layer
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
fn wrn_phase2_validation_chain_validates() {
    let core = {
        let mut b = LayerBuilder::new("core", None);
        for r in
            eigon_json::parse_document(include_str!("../../../ontologies/core/core-ontology.json"))
                .unwrap()
        {
            b.add_resource(r).unwrap();
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };
    let reflection = {
        let mut b = LayerBuilder::new("reflection", Some(core));
        for src in [
            include_str!("../../../ontologies/reflection/reflection-ontology.json"),
            include_str!("../../../ontologies/eigentt/eigentt-type-fragment.json"),
            include_str!("../../../ontologies/institution/institution-ontology.json"),
        ] {
            for r in eigon_json::parse_document(src).unwrap() {
                b.add_resource(r).unwrap();
            }
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };
    let reasoning = {
        let mut b = LayerBuilder::new("reasoning", Some(reflection));
        for r in esl::compile(include_str!("../../../ontologies/reasoning/reasoning.esl"))
            .expect("reasoning.esl compiles")
        {
            b.add_resource(r).unwrap();
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };
    let statistics = esl_against(
        include_str!("../../../ontologies/statistics/statistics.esl"),
        &reasoning,
        "statistics",
    );
    let bench_core = esl_against(
        include_str!("../../../experiments/benchmark/base-ontologies/bench-core.esl"),
        &statistics,
        "bench-core",
    );
    let harness = esl_against(
        include_str!("../../../experiments/benchmark/harness-ontology.esl"),
        &bench_core,
        "harness",
    );
    let onco = esl_against(
        include_str!("../../../experiments/publications/wrn-helicase/chain/01-onco.esl"),
        &harness,
        "onco",
    );
    // Literature layer: references + imported-claim warrants (reference:Citation).
    // Loaded before the chain so phase2/phase3 rules can compose the literature
    // warrants (e.g. WRNActivitiesSeparable [14]) as premises.
    let literature = esl_against(
        include_str!("../../../experiments/publications/wrn-helicase/chain/02-literature.esl"),
        &onco,
        "wrn-literature",
    );
    // D54 two-phase load: plans (emitters) before conclusions (consumers).
    let recompute_plans = esl_against(
        include_str!(
            "../../../experiments/publications/wrn-helicase/chain/03-phase1-recompute-plans.esl"
        ),
        &literature,
        "wrn-recompute-plans",
    );
    let recompute = esl_against_pending(
        include_str!(
            "../../../experiments/publications/wrn-helicase/chain/04-phase1-recompute-conclusions.esl"
        ),
        &recompute_plans,
        "wrn-recompute-conclusions",
        STATS_RECOMPUTED,
    );
    let phase1 = esl_against(
        include_str!(
            "../../../experiments/publications/wrn-helicase/chain/05-phase1-discovery.esl"
        ),
        &recompute,
        "wrn-phase1",
    );
    let phase2 = esl_against(
        include_str!(
            "../../../experiments/publications/wrn-helicase/chain/07-phase2-validation.esl"
        ),
        &phase1,
        "wrn-phase2",
    );

    let ctx = ExecutionContext::new(
        phase2,
        "wrn-phase2",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    );
    let inst = ReasoningInstitution::new();

    // C-VAL is now kernel-recomputed (concl_val_recomputed, statistics layer);
    // the linked-external concl_val it replaced is retired. The Declared
    // experimental-design conclusions remain here:
    assert_holds(&ctx, &inst, "urn:eigenius:pub:wrn:concl_ontarget");
    assert_holds(&ctx, &inst, "urn:eigenius:pub:wrn:concl_helicase_required");
    assert_holds(&ctx, &inst, "urn:eigenius:pub:wrn:concl_exo_dispensable");
}
