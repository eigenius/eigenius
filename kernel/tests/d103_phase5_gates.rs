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

//! D103 Phase 5 — Multi-pass revision chain and negative-gate refusal tests.
//!
//! Uses purely synthetic identifiers (`SYN1`, `p.Leu101Val`, `c.301A>G`) with zero external corpus
//! material. Verifies that every structural failure mode (value mismatch, missing/blocked
//! observation, premise-failure propagation, dropped summary qualifier, invalid exemption) fails
//! admission for its exact structural cause, while Pass 1 -> Pass 2 -> Pass 3 amendments pass.

use std::collections::BTreeSet;
use std::sync::Arc;

use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::Iri;
use eigenius_kernel::validation::Validator;

fn compile_layer(name: &str, src: &str, parent: Arc<Layer>) -> Arc<Layer> {
    let resources = esl::compile(src, &parent).unwrap_or_else(|errs| {
        panic!(
            "{name} failed to compile: {}",
            errs.iter()
                .map(|e| format!("{e}"))
                .collect::<Vec<_>>()
                .join("; ")
        )
    });
    let mut b = LayerBuilder::new(name, Some(parent));
    for r in resources {
        b.add_resource(r).expect("add resource");
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

fn d103_base_chain() -> Arc<Layer> {
    let head = Arc::clone(
        eigenius_kernel::bootstrap::bootstrap()
            .expect("bootstrap")
            .head(),
    );
    let variant = compile_layer(
        "variant",
        include_str!("../../ontologies/variant/variant.esl"),
        head,
    );
    let clinical = compile_layer(
        "clinical",
        include_str!("../../ontologies/clinical/clinical.esl"),
        variant,
    );
    let acquisition = compile_layer(
        "acquisition",
        include_str!("../../ontologies/acquisition/acquisition.esl"),
        clinical,
    );
    let clinvar = compile_layer(
        "clinvar",
        include_str!("../../ontologies/clinvar/clinvar.esl"),
        acquisition,
    );
    let popgen = compile_layer(
        "popgen",
        include_str!("../../ontologies/popgen/popgen.esl"),
        clinvar,
    );
    let structure = compile_layer(
        "structure",
        include_str!("../../ontologies/structure/structure.esl"),
        popgen,
    );
    let predictors = compile_layer(
        "predictors",
        include_str!("../../ontologies/predictors/predictors.esl"),
        structure,
    );
    let sequence = compile_layer(
        "sequence",
        include_str!("../../ontologies/sequence/sequence.esl"),
        predictors,
    );
    compile_layer(
        "align",
        include_str!("../../ontologies/align/align.esl"),
        sequence,
    )
}

fn synthetic_data_layer() -> Arc<Layer> {
    let src = r#"
namespace core        = "urn:eigenius:core";
namespace eigentt     = "urn:eigenius:eigentt";
namespace prov        = "urn:eigenius:prov";
namespace agent       = "urn:eigenius:prov:agent";
namespace units       = "urn:eigenius:units";
namespace enc         = "urn:eigenius:encoding";
namespace variant     = "urn:eigenius:variant";
namespace clinical    = "urn:eigenius:clinical";
namespace acquisition = "urn:eigenius:acquisition";
namespace popgen      = "urn:eigenius:popgen";
namespace align       = "urn:eigenius:align";
namespace syn         = "urn:eigenius:test:d103p5";

resource syn:act_query : prov:Activity {
    core:short_name = "syn_query_activity";
}

resource syn:pg_rel : popgen:DatasetRelease {
    popgen:dataset_name     = "syn-pop";
    popgen:dataset_version  = "v4.1";
    popgen:reference_genome = "GRCh38";
    popgen:callset_kind     = popgen:callset_exome;
}

resource syn:nuc_301 : variant:NucleotideAllele {
    variant:reference = "NM_000001.1";
    variant:start     = 301;
    variant:end       = 301;
    variant:ref_state = "A";
    variant:alt       = "G";
}

resource syn:obs_count_4 : popgen:AlleleObservation {
    popgen:in_release       = syn:pg_rel;
    popgen:observed_allele  = syn:nuc_301;
    popgen:allele_count     = 4;
    popgen:allele_number    = 100000;
    popgen:hemizygous_count = 4;
    eigentt:proposition     = type_expr(
        popgen:ObservedHemizygousCount(syn:pg_rel, syn:nuc_301, units:mk_quantity(r"4", 0))
    );
}

resource syn:obs_count_4_trace : prov:ObservationTrace {
    prov:resource         = syn:obs_count_4;
    prov:was_generated_by = syn:act_query;
    prov:timestamp        = "2026-09-16T00:00:00Z";
}

// A resource with NO ObservationTrace (simulating a blocked / unavailable clinical report).
resource syn:missing_report : core:Resource {
    core:short_name     = "missing_clinical_report";
    eigentt:proposition = type_expr(syn:ReportStates("proband-genotype"));
}

axiom syn:ReportStates : core:string -> Prop
axiom syn:ClaimHolds   : core:string -> Prop

resource syn:unit_exempt : enc:DiscourseUnit {
    enc:prose     = "Standing scope note.";
    enc:unit_kind = enc:kind_prose;
    enc:locator   = "p0-s1";
}
"#;
    let layer = compile_layer("d103-p5-data", src, d103_base_chain());
    let errs = Validator::new(Arc::clone(&layer)).validate();
    assert!(errs.is_empty(), "synthetic data layer invalid: {errs:#?}");
    layer
}

fn check_compression_gate(layer: &Layer) -> Vec<String> {
    let cls_iri = Iri::parse("urn:eigenius:align:CompressionRecord").unwrap();
    let req_iri = Iri::parse("urn:eigenius:align:required_qualifiers").unwrap();
    let dis_iri = Iri::parse("urn:eigenius:align:discharged_qualifiers").unwrap();
    let mut violations = Vec::new();
    for (id, res) in layer.iter_resources() {
        if !res.is_instance_of(&cls_iri) {
            continue;
        }
        let required: Vec<Iri> = res
            .properties()
            .get(&req_iri)
            .map(|v| v.as_iri_array())
            .unwrap_or_default();
        let discharged: BTreeSet<Iri> = res
            .properties()
            .get(&dis_iri)
            .map(|v| v.as_iri_array().into_iter().collect())
            .unwrap_or_default();
        let missing: Vec<String> = required
            .into_iter()
            .filter(|q| !discharged.contains(q))
            .map(|q| q.as_str().to_string())
            .collect();
        if !missing.is_empty() {
            violations.push(format!(
                "{}: dropped required qualifier(s): [{}]",
                id.as_str(),
                missing.join(", ")
            ));
        }
    }
    violations
}

#[test]
fn pass1_observed_value_mismatch_is_refused() {
    let src = r#"
namespace core          = "urn:eigenius:core";
namespace eigentt       = "urn:eigenius:eigentt";
namespace units         = "urn:eigenius:units";
namespace justification = "urn:eigenius:justification";
namespace popgen        = "urn:eigenius:popgen";
namespace syn           = "urn:eigenius:test:d103p5";

resource syn:concl_bad_value : justification:Conclusion {
    justification:grounds_judgement =
        holds(eigentt:logic_kernel,
              type_expr(observed(
                  "urn:eigenius:test:d103p5:obs_count_4",
                  popgen:ObservedHemizygousCount(syn:pg_rel, syn:nuc_301, units:mk_quantity(r"9", 0)),
                  ())),
              type_expr(justification:Grounds(
                  popgen:ObservedHemizygousCount(syn:pg_rel, syn:nuc_301, units:mk_quantity(r"9", 0)))));
}
"#;
    let layer = compile_layer("p5-neg-mismatch", src, synthetic_data_layer());
    let joined: String = Validator::new(layer)
        .validate()
        .into_iter()
        .map(|e| format!("{e:?}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("no admitted IsObservedAs witness")
            && joined.contains("urn:eigenius:test:d103p5:obs_count_4"),
        "expected IsObservedAs witness refusal on mismatched count, got:\n{joined}"
    );
}

#[test]
fn pass1_blocked_missing_observation_is_refused() {
    let src = r#"
namespace eigentt       = "urn:eigenius:eigentt";
namespace justification = "urn:eigenius:justification";
namespace syn           = "urn:eigenius:test:d103p5";

resource syn:concl_blocked : justification:Conclusion {
    justification:grounds_judgement =
        holds(eigentt:logic_kernel,
              type_expr(observed(
                  "urn:eigenius:test:d103p5:missing_report",
                  syn:ReportStates("proband-genotype"),
                  ())),
              type_expr(justification:Grounds(syn:ReportStates("proband-genotype"))));
}
"#;
    let layer = compile_layer("p5-neg-blocked", src, synthetic_data_layer());
    let joined: String = Validator::new(layer)
        .validate()
        .into_iter()
        .map(|e| format!("{e:?}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("no admitted IsObservedAs witness")
            && joined.contains("urn:eigenius:test:d103p5:missing_report"),
        "expected blocked claim refusal due to absent ObservationTrace, got:\n{joined}"
    );
}

#[test]
fn pass1_marked_premise_failure_propagates_while_sibling_commits() {
    let src = r#"
namespace core          = "urn:eigenius:core";
namespace eigentt       = "urn:eigenius:eigentt";
namespace prov          = "urn:eigenius:prov";
namespace agent         = "urn:eigenius:prov:agent";
namespace units         = "urn:eigenius:units";
namespace justification = "urn:eigenius:justification";
namespace popgen        = "urn:eigenius:popgen";
namespace syn           = "urn:eigenius:test:d103p5";

resource syn:bridge_good : justification:Declaration {
    prov:was_attributed_to = agent:eigenius_core_team;
    eigentt:proposition    = type_expr(
        popgen:ObservedHemizygousCount(syn:pg_rel, syn:nuc_301, units:mk_quantity(r"4", 0))
        -> syn:ClaimHolds("sibling-good")
    );
}
resource syn:bridge_good_trace : prov:DeclarationTrace {
    prov:resource          = syn:bridge_good;
    prov:was_attributed_to = agent:eigenius_core_team;
    prov:timestamp         = "2026-09-16T00:00:00Z";
}
resource syn:concl_sibling_good : justification:Conclusion {
    justification:grounds_judgement =
        holds(eigentt:logic_kernel,
              type_expr(app(
                  declared("urn:eigenius:test:d103p5:bridge_good",
                           popgen:ObservedHemizygousCount(syn:pg_rel, syn:nuc_301, units:mk_quantity(r"4", 0))
                           -> syn:ClaimHolds("sibling-good"), ()),
                  observed("urn:eigenius:test:d103p5:obs_count_4",
                           popgen:ObservedHemizygousCount(syn:pg_rel, syn:nuc_301, units:mk_quantity(r"4", 0)), ()))),
              type_expr(justification:Grounds(syn:ClaimHolds("sibling-good"))));
}

resource syn:bridge_propagated : justification:Declaration {
    prov:was_attributed_to = agent:eigenius_core_team;
    eigentt:proposition    = type_expr(
        popgen:ObservedHemizygousCount(syn:pg_rel, syn:nuc_301, units:mk_quantity(r"4", 0))
        -> (syn:ReportStates("proband-genotype") -> syn:ClaimHolds("marked-bad"))
    );
}
resource syn:bridge_propagated_trace : prov:DeclarationTrace {
    prov:resource          = syn:bridge_propagated;
    prov:was_attributed_to = agent:eigenius_core_team;
    prov:timestamp         = "2026-09-16T00:00:00Z";
}
resource syn:concl_marked_bad : justification:Conclusion {
    justification:grounds_judgement =
        holds(eigentt:logic_kernel,
              type_expr(app(
                  app(declared("urn:eigenius:test:d103p5:bridge_propagated",
                               popgen:ObservedHemizygousCount(syn:pg_rel, syn:nuc_301, units:mk_quantity(r"4", 0))
                               -> (syn:ReportStates("proband-genotype") -> syn:ClaimHolds("marked-bad")), ()),
                      observed("urn:eigenius:test:d103p5:obs_count_4",
                               popgen:ObservedHemizygousCount(syn:pg_rel, syn:nuc_301, units:mk_quantity(r"4", 0)), ())),
                  observed("urn:eigenius:test:d103p5:missing_report",
                           syn:ReportStates("proband-genotype"), ()))),
              type_expr(justification:Grounds(syn:ClaimHolds("marked-bad"))));
}
"#;
    let layer = compile_layer("p5-neg-prop", src, synthetic_data_layer());
    let errs: Vec<String> = Validator::new(layer)
        .validate()
        .into_iter()
        .map(|e| format!("{e:?}"))
        .collect();
    assert!(
        errs.iter()
            .any(|e| e.contains("concl_marked_bad") && e.contains("missing_report")),
        "expected premise-failure propagation on concl_marked_bad naming missing_report, got:\n{errs:#?}"
    );
    assert!(
        !errs.iter().any(|e| e.contains("concl_sibling_good")),
        "sibling conclusion must remain error-free; got:\n{errs:#?}"
    );
}

#[test]
fn pass1_compressed_dropped_qualifier_is_refused_and_pass2_amendment_passes() {
    let pass1_src = r#"
namespace core  = "urn:eigenius:core";
namespace prov  = "urn:eigenius:prov";
namespace agent = "urn:eigenius:prov:agent";
namespace align = "urn:eigenius:align";
namespace syn   = "urn:eigenius:test:d103p5";

resource syn:qual_caveat : core:Resource {
    core:description = "Single-submitter review caveat that the summary must retain.";
}

resource syn:comp_pass1 : align:CompressionRecord {
    align:summary_claim         = syn:obs_count_4;
    align:compresses            = [syn:obs_count_4];
    align:required_qualifiers   = [syn:qual_caveat];
    align:discharged_qualifiers = [];
    prov:was_attributed_to      = agent:eigenius_core_team;
}
"#;
    let pass1 = compile_layer("p5-comp-pass1", pass1_src, synthetic_data_layer());
    // Logical validation admits it (D100), while the structural compression gate refuses it!
    assert!(Validator::new(Arc::clone(&pass1)).validate().is_empty());
    let gate_errs = check_compression_gate(&pass1);
    assert_eq!(gate_errs.len(), 1, "expected 1 dropped qualifier violation");
    assert!(gate_errs[0].contains("urn:eigenius:test:d103p5:qual_caveat"));

    // Pass-2 amendment supersedes Pass 1 and retains/discharges the qualifier.
    let pass2_src = r#"
namespace prov  = "urn:eigenius:prov";
namespace agent = "urn:eigenius:prov:agent";
namespace align = "urn:eigenius:align";
namespace syn   = "urn:eigenius:test:d103p5";

resource syn:comp_pass2 : align:CompressionRecord {
    align:summary_claim         = syn:obs_count_4;
    align:compresses            = [syn:obs_count_4];
    align:required_qualifiers   = [syn:qual_caveat];
    align:discharged_qualifiers = [syn:qual_caveat];
    align:supersedes            = syn:comp_pass1;
    prov:was_attributed_to      = agent:eigenius_core_team;
}
"#;
    let pass2 = compile_layer("p5-comp-pass2", pass2_src, pass1);
    assert!(Validator::new(Arc::clone(&pass2)).validate().is_empty());
    assert!(check_compression_gate(&pass2).is_empty());
}

#[test]
fn pass1_disputed_exemption_missing_reason_is_refused() {
    let src = r#"
namespace prov  = "urn:eigenius:prov";
namespace agent = "urn:eigenius:prov:agent";
namespace align = "urn:eigenius:align";
namespace syn   = "urn:eigenius:test:d103p5";

resource syn:bad_exempt : align:ExemptionRecord {
    align:exempt_unit      = syn:unit_exempt;
    prov:was_attributed_to = agent:eigenius_core_team;
}
"#;
    let layer = compile_layer("p5-neg-exempt", src, synthetic_data_layer());
    let joined: String = Validator::new(layer)
        .validate()
        .into_iter()
        .map(|e| format!("{e:?}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("exemption_reason"),
        "expected missing align:exemption_reason validation error, got:\n{joined}"
    );
}

#[test]
fn multipass_pass1_to_pass2_to_pass3_amendment_chain_validates() {
    let pass1_src = r#"
namespace core          = "urn:eigenius:core";
namespace eigentt       = "urn:eigenius:eigentt";
namespace prov          = "urn:eigenius:prov";
namespace agent         = "urn:eigenius:prov:agent";
namespace justification = "urn:eigenius:justification";
namespace syn           = "urn:eigenius:test:d103p5";

resource syn:p1_decl : justification:Declaration {
    prov:was_attributed_to = agent:eigenius_core_team;
    eigentt:proposition    = type_expr(syn:ClaimHolds("P1-C1"));
}
resource syn:p1_decl_trace : prov:DeclarationTrace {
    prov:resource          = syn:p1_decl;
    prov:was_attributed_to = agent:eigenius_core_team;
    prov:timestamp         = "2026-09-16T00:00:00Z";
}
resource syn:p1_concl : justification:Conclusion {
    justification:grounds_judgement =
        holds(eigentt:logic_kernel,
              type_expr(declared("urn:eigenius:test:d103p5:p1_decl", syn:ClaimHolds("P1-C1"), ())),
              type_expr(justification:Grounds(syn:ClaimHolds("P1-C1"))));
}
"#;
    let pass1 = compile_layer("p5-pass1", pass1_src, synthetic_data_layer());
    assert!(Validator::new(Arc::clone(&pass1)).validate().is_empty());

    let pass2_src = r#"
namespace core          = "urn:eigenius:core";
namespace eigentt       = "urn:eigenius:eigentt";
namespace prov          = "urn:eigenius:prov";
namespace agent         = "urn:eigenius:prov:agent";
namespace justification = "urn:eigenius:justification";
namespace align         = "urn:eigenius:align";
namespace syn           = "urn:eigenius:test:d103p5";

resource syn:p2_decl : justification:Declaration {
    prov:was_attributed_to = agent:eigenius_core_team;
    align:supersedes       = syn:p1_decl;
    eigentt:proposition    = type_expr(syn:ClaimHolds("P2-C1"));
}
resource syn:p2_decl_trace : prov:DeclarationTrace {
    prov:resource          = syn:p2_decl;
    prov:was_attributed_to = agent:eigenius_core_team;
    prov:timestamp         = "2026-09-17T00:00:00Z";
}
resource syn:p2_concl : justification:Conclusion {
    align:supersedes = syn:p1_concl;
    justification:grounds_judgement =
        holds(eigentt:logic_kernel,
              type_expr(declared("urn:eigenius:test:d103p5:p2_decl", syn:ClaimHolds("P2-C1"), ())),
              type_expr(justification:Grounds(syn:ClaimHolds("P2-C1"))));
}
resource syn:retire_p1 : align:RetirementRecord {
    align:retired_resource       = syn:p1_concl;
    align:superseded_by_resource = syn:p2_concl;
    align:retirement_reason      = "Replaced by amended wording in Pass 2.";
    prov:was_attributed_to       = agent:eigenius_core_team;
}
"#;
    let pass2 = compile_layer("p5-pass2", pass2_src, pass1);
    assert!(Validator::new(Arc::clone(&pass2)).validate().is_empty());

    let pass3_src = r#"
namespace core          = "urn:eigenius:core";
namespace eigentt       = "urn:eigenius:eigentt";
namespace prov          = "urn:eigenius:prov";
namespace agent         = "urn:eigenius:prov:agent";
namespace justification = "urn:eigenius:justification";
namespace align         = "urn:eigenius:align";
namespace syn           = "urn:eigenius:test:d103p5";

resource syn:p3_bridge : justification:Declaration {
    prov:was_attributed_to = agent:eigenius_core_team;
    eigentt:proposition    = type_expr(syn:ClaimHolds("P2-C1") -> syn:ClaimHolds("P3-C1"));
}
resource syn:p3_bridge_trace : prov:DeclarationTrace {
    prov:resource          = syn:p3_bridge;
    prov:was_attributed_to = agent:eigenius_core_team;
    prov:timestamp         = "2026-09-19T00:00:00Z";
}
resource syn:p3_concl : justification:Conclusion {
    justification:grounds_judgement =
        holds(eigentt:logic_kernel,
              type_expr(app(
                  declared("urn:eigenius:test:d103p5:p3_bridge",
                           syn:ClaimHolds("P2-C1") -> syn:ClaimHolds("P3-C1"), ()),
                  declared("urn:eigenius:test:d103p5:p2_decl", syn:ClaimHolds("P2-C1"), ()))),
              type_expr(justification:Grounds(syn:ClaimHolds("P3-C1"))));
}
"#;
    let pass3 = compile_layer("p5-pass3", pass3_src, pass2);
    let errs = Validator::new(Arc::clone(&pass3)).validate();
    assert!(
        errs.is_empty(),
        "Pass 1 -> Pass 2 -> Pass 3 chain failed: {errs:#?}"
    );
}
