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
//! recorded as bench:ToolArtifacts whose ProgramTrace is provenance only); the
//! Declared rules lift those readouts into the conclusions. No statistics
//! institution is needed here — the warrants are Declared + linked-external.

use std::sync::Arc;

use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;

/// The kernel-recomputed (statistics-institution) conclusions: their
/// plan declarations and input observations are committed by the statistics institution's
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
/// gate: every `justification:Conclusion` this layer adds MUST validate to
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
    let resources = esl::compile(source, parent).unwrap_or_else(|errs| {
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

    // STRUCTURAL validation, which `LayerBuilder::build` does not run and
    // `esl::compile_against_layer` does not either — it compiles, it does not
    // validate. Without it the chain is checked only for its conclusions'
    // certificates, so a resource-typed property naming something nothing
    // resolves, a required property left off, or a deleted class all commit
    // clean here and fail on a real load. `00-wrn-vocabulary.esl`'s header
    // records that costing two months.
    let structural = eigenius_kernel::validation::Validator::new(layer.clone()).validate();
    assert!(
        structural.is_empty(),
        "{name}: the layer must validate structurally — the live loader runs this and \
         these tests did not, which is how the chain silently stopped loading once before. \
         {} error(s): {:#?}",
        structural.len(),
        structural.iter().take(10).collect::<Vec<_>>()
    );

    // Every conclusion's judgement must type-check. This ran `do_validate_justification` and
    // read a `Verdict`; the check it was reading moved to commit at P2, so it now reads the
    // errors Rule 21 produces — same check, taken from where it lives. The `pending` skip is
    // preserved exactly: those conclusions' witnesses arrive out of band.
    let sentence_class = "urn:eigenius:justification:Conclusion";
    let all_errors = eigenius_kernel::validation::Validator::new(layer.clone()).validate();
    for r in &resources {
        if !r.is_a().iter().any(|c| c.as_str() == sentence_class) {
            continue;
        }
        let iri = r.id().map(|i| i.as_str().to_string()).unwrap_or_default();
        let diag = all_errors
            .iter()
            .filter(|e| e.resource_id.as_ref().is_some_and(|i| i.as_str() == iri))
            .map(|e| e.message.clone())
            .collect::<Vec<_>>()
            .join("\n");
        if !diag.is_empty() && !pending.iter().any(|p| *p == iri) {
            panic!(
                "esl_against({name}): conclusion `{iri}` did not type-check — the live \
                 AutoOnLoad gate would reject this layer, so a downstream lemma citation of it \
                 would be unsound. diagnostic: {diag}\n  If its witness is produced out of band \
                 (R runtime / statistics institution AutoOnLoad, not run in this harness), add \
                 its IRI to `pending`."
            );
        }
        // The VerificationTrace assertion that stood here is retired with the reasoning
        // institution. It pinned P3's narrowing — a trace is minted only alongside a
        // `justification:proof`, so a certificate-only conclusion owes none — against the
        // handler that minted them. That minter goes with the crate, and the Lean
        // institution becomes the producer of `Verified` witnesses (eigenius#160), so the
        // property this asserted now holds vacuously here and is #160's to re-pin.
    }
    layer
}

/// The conclusion's judgement type-checks — no validation error names it.
///
/// Read a `Verdict` off `do_validate_justification` until P7. The check moved to commit at
/// P2, so this reads Rule 21's errors instead: it decodes the judgement, checks its `type` is
/// a type, and checks its `term` against it.
fn assert_holds(ctx: &ExecutionContext, iri: &str) {
    ctx.resolve(&Iri::parse(iri).expect("sentence IRI"))
        .unwrap_or_else(|| panic!("sentence `{iri}` should be on the chain"));
    let diagnostic = eigenius_kernel::validation::Validator::new(ctx.head().clone())
        .validate()
        .into_iter()
        .filter(|e| e.resource_id.as_ref().is_some_and(|i| i.as_str() == iri))
        .map(|e| e.message)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        diagnostic.is_empty(),
        "expected `{iri}` to type-check; got: {diagnostic}"
    );
}

#[test]
fn wrn_phase2_validation_chain_validates() {
    let core = {
        let mut b = LayerBuilder::new("core", None);
        for r in
            eigon_json::parse_document(include_str!("../../ontologies/core/core-ontology.json"))
                .unwrap()
        {
            b.add_resource(r).unwrap();
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };
    let reflection = {
        let mut b = LayerBuilder::new("reflection", Some(core));
        for src in [
            include_str!("../../ontologies/reflection/reflection-ontology.json"),
            include_str!("../../ontologies/eigentt/eigentt-type-fragment.json"),
            include_str!("../../ontologies/institution/institution-ontology.json"),
            include_str!("../../ontologies/ingest/ingest-ontology.json"),
        ] {
            for r in eigon_json::parse_document(src).unwrap() {
                b.add_resource(r).unwrap();
            }
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };
    // `prov` — the provenance axis, above reflection and below everything that
    // names an agent, a trace or an attribution.
    let prov = {
        let mut b = LayerBuilder::new("prov", Some(reflection));
        for r in esl::compile(
            include_str!("../../ontologies/prov/prov.esl"),
            &eigenius_kernel::layer::Layer::empty(),
        )
        .unwrap()
        {
            b.add_resource(r).unwrap();
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };
    let reasoning = {
        let mut b = LayerBuilder::new("reasoning", Some(prov));
        for r in esl::compile(
            include_str!("../../ontologies/justification/justification.esl"),
            &eigenius_kernel::layer::Layer::empty(),
        )
        .expect("reasoning.esl compiles")
        {
            b.add_resource(r).unwrap();
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };
    let statistics = esl_against(
        include_str!("../../ontologies/statistics/statistics.esl"),
        &reasoning,
        "statistics",
    );
    // `reference` — the literature layer uses reference:Citation, and this chain
    // never carried the ontology declaring it. Nothing noticed because the layers
    // were built without structural validation.
    let reference = esl_against(
        include_str!("../../ontologies/reference/reference.esl"),
        &statistics,
        "reference",
    );
    let bench_core = esl_against(
        include_str!("../../experiments/benchmark/base-ontologies/bench-core.esl"),
        &reference,
        "bench-core",
    );
    let harness = esl_against(
        include_str!("../../experiments/benchmark/harness-ontology.esl"),
        &bench_core,
        "harness",
    );
    let onco = esl_against(
        include_str!("../../experiments/publications/wrn-helicase/chain/01-onco.esl"),
        &harness,
        "onco",
    );
    // Literature layer: references + imported-claim warrants (reference:Citation).
    // Loaded before the chain so phase2/phase3 rules can compose the literature
    // warrants (e.g. WRNActivitiesSeparable [14]) as premises.
    let literature = esl_against(
        include_str!("../../experiments/publications/wrn-helicase/chain/02-literature.esl"),
        &onco,
        "wrn-literature",
    );
    // D54 two-phase load: plans (emitters) before conclusions (consumers).
    let recompute_plans = esl_against(
        include_str!(
            "../../experiments/publications/wrn-helicase/chain/03-phase1-recompute-plans.esl"
        ),
        &literature,
        "wrn-recompute-plans",
    );
    let recompute = esl_against_pending(
        include_str!(
            "../../experiments/publications/wrn-helicase/chain/04-phase1-recompute-conclusions.esl"
        ),
        &recompute_plans,
        "wrn-recompute-conclusions",
        STATS_RECOMPUTED,
    );
    let phase1 = esl_against(
        include_str!("../../experiments/publications/wrn-helicase/chain/05-phase1-discovery.esl"),
        &recompute,
        "wrn-phase1",
    );
    let phase2 = esl_against(
        include_str!("../../experiments/publications/wrn-helicase/chain/07-phase2-validation.esl"),
        &phase1,
        "wrn-phase2",
    );

    let ctx = ExecutionContext::new(
        phase2,
        "wrn-phase2",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    );

    // C-VAL is now kernel-recomputed (concl_val_recomputed, statistics layer);
    // the linked-external concl_val it replaced is retired. The Declared
    // experimental-design conclusions remain here:
    assert_holds(&ctx, "urn:eigenius:pub:wrn:concl_ontarget");
    assert_holds(&ctx, "urn:eigenius:pub:wrn:concl_helicase_required");
    assert_holds(&ctx, "urn:eigenius:pub:wrn:concl_exo_dispensable");
}
