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

//! WRN Phase-5: C-MMR (causal dissection) and C-MAIN (the thesis).
//!
//! C-MAIN reaches `SyntheticLethal(WRN, MSI)` by modus ponens over a
//! Declared synthesis implication (`SVD → IVD → RA → DL → CD →
//! SyntheticLethal`) applied to the five findings (C-VAL, C-VIVO,
//! D-HELICASE, C-MECH, C-MMR). Each antecedent is discharged by its own
//! warrant inlined into the certificate — a proven sentence is the
//! antecedent of the implication, not an evidence atom. (The lemma-citation
//! mechanism that would let C-MAIN reference the phase conclusions directly
//! — D39's planned `Asserts` wrapper — is a separate follow-up.)

use std::sync::Arc;

use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;

/// Statistics-institution-recomputed conclusions (their plan declarations and input
/// observations are
/// committed by the statistics layer's own AutoOnLoad, which this harness does not
/// run); validated in wrn_phase1_recompute.rs. See esl_against_pending.
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
/// `Holds`, else the live loader would reject it (and a downstream lemma citation
/// would be unsound). Panics on a non-`Holds` sentence unless its IRI is in
/// `pending` (witnesses committed out of band — the statistics institution's AutoOnLoad).
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
    }
    layer
}

/// The validation errors naming this conclusion, joined. Empty is what the handler used to
/// report as `Holds`; the check moved to commit at P2 and is Rule 21's.
fn judgement_diagnostic(ctx: &ExecutionContext, iri: &str) -> String {
    ctx.resolve(&Iri::parse(iri).expect("sentence IRI"))
        .unwrap_or_else(|| panic!("sentence `{iri}` should be on the chain"));
    eigenius_kernel::validation::Validator::new(ctx.head().clone())
        .validate()
        .into_iter()
        .filter(|e| e.resource_id.as_ref().is_some_and(|i| i.as_str() == iri))
        .map(|e| e.message)
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_ctx() -> ExecutionContext {
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
    // Literature layer: phase2/phase3 rules compose its warrants as premises.
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
    // 08a commits the out-of-band programs' inputs (as observations) and their
    // reproducibility claims (as declarations). It must precede 08, whose
    // conclusions cite both: `emit_from_trace` resolves a trace's target on the
    // chain, so the targets have to be in an ancestor layer.
    // The programs' real inputs: content-addressed PinnedExternalFiles with
    // reference + content_hash + media_type, declared beside each program.
    // 08a used to mint hash-less stand-ins whose IRIs were TRUNCATED PREFIXES of
    // these — `wrn:input_8d26fbb8aafb610a` for
    // `ingest:file:8d26fbb8aafb610a4952…` — so an observation named 16 hex digits
    // of a hash instead of pointing at the bytes.
    let program_inputs = {
        let mut b = LayerBuilder::new("wrn-program-inputs", Some(phase2.clone()));
        for src in [
            include_str!("../../experiments/publications/wrn-helicase/programs/invivo/xenograft-input.json"),
            include_str!("../../experiments/publications/wrn-helicase/programs/invivo/km12-competition-input.json"),
            include_str!("../../experiments/publications/wrn-helicase/programs/mechanism/foci-ed6-input.json"),
            include_str!("../../experiments/publications/wrn-helicase/programs/mechanism/gh2ax-foci-input.json"),
            include_str!("../../experiments/publications/wrn-helicase/programs/mechanism/gh2ax-intensity-input.json"),
            include_str!("../../experiments/publications/wrn-helicase/programs/mechanism/if-ed5-input.json"),
            include_str!("../../experiments/publications/wrn-helicase/programs/mechanism/patm-foci-input.json"),
            include_str!("../../experiments/publications/wrn-helicase/programs/specificity/paralog-ed9a-input.json"),
        ] {
            for r in eigon_json::parse_document(src).unwrap() {
                b.add_resource(r).unwrap();
            }
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    };
    let provenance = esl_against(
        include_str!(
            "../../experiments/publications/wrn-helicase/chain/08a-program-provenance.esl"
        ),
        &program_inputs,
        "wrn-08a",
    );
    let phase3 = esl_against_pending(
        include_str!(
            "../../experiments/publications/wrn-helicase/chain/08-phase3-invivo-mechanism.esl"
        ),
        &provenance,
        "wrn-phase3",
        // No pending allowance — see the same note in wrn_phase3.rs. A computed
        // conclusion is `App(Declared(plan), Observed(input))` and both witnesses are
        // chain-resident facts 08a commits, so nothing here needs a runtime.
        &[],
    );
    let phase5 = esl_against(
        include_str!("../../experiments/publications/wrn-helicase/chain/09-phase5-synthesis.esl"),
        &phase3,
        "wrn-phase5",
    );
    ExecutionContext::new(
        phase5,
        "wrn-phase5",
        ExecutionMode::ReadOnly,
        LayerStorage::in_memory(),
    )
}

#[test]
fn wrn_phase5_cmmr_and_cmain_validate() {
    let ctx = build_ctx();

    let diag = judgement_diagnostic(&ctx, "urn:eigenius:pub:wrn:concl_mmr");
    assert!(diag.is_empty(), "C-MMR should type-check; got: {diag}");

    // C-MAIN: the thesis, by modus ponens over the synthesis implication.
    let diag = judgement_diagnostic(&ctx, "urn:eigenius:pub:wrn:concl_main");
    assert!(diag.is_empty(), "C-MAIN should type-check; got: {diag}");
}
