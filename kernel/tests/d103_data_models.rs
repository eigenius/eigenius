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

//! D103 Phase 1 — validation and type-refusal tests for the six domain data-model ontologies
//! (`acquisition`, extended `clinvar`, `popgen`, `structure`, `predictors`, `sequence`) plus
//! `align` (post-parse entity alignment, direct table-row claims, and `compressed`/`exempt`
//! records).
//!
//! Uses synthetic gene/variant identifiers only — zero private corpus material.

use std::sync::Arc;

use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
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

fn d103_ontology_layers() -> Vec<(&'static str, Arc<Layer>)> {
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
        Arc::clone(&variant),
    );
    let acquisition = compile_layer(
        "acquisition",
        include_str!("../../ontologies/acquisition/acquisition.esl"),
        Arc::clone(&clinical),
    );
    let clinvar = compile_layer(
        "clinvar",
        include_str!("../../ontologies/clinvar/clinvar.esl"),
        Arc::clone(&acquisition),
    );
    let popgen = compile_layer(
        "popgen",
        include_str!("../../ontologies/popgen/popgen.esl"),
        Arc::clone(&clinvar),
    );
    let structure = compile_layer(
        "structure",
        include_str!("../../ontologies/structure/structure.esl"),
        Arc::clone(&popgen),
    );
    let predictors = compile_layer(
        "predictors",
        include_str!("../../ontologies/predictors/predictors.esl"),
        Arc::clone(&structure),
    );
    let sequence = compile_layer(
        "sequence",
        include_str!("../../ontologies/sequence/sequence.esl"),
        Arc::clone(&predictors),
    );
    let align = compile_layer(
        "align",
        include_str!("../../ontologies/align/align.esl"),
        Arc::clone(&sequence),
    );
    vec![
        ("acquisition", acquisition),
        ("clinvar", clinvar),
        ("popgen", popgen),
        ("structure", structure),
        ("predictors", predictors),
        ("sequence", sequence),
        ("align", align),
    ]
}

fn d103_ontology_chain() -> Arc<Layer> {
    d103_ontology_layers().pop().expect("non-empty").1
}

#[test]
fn all_d103_ontologies_compile_and_validate_cleanly() {
    for (name, layer) in d103_ontology_layers() {
        let errors: Vec<String> = Validator::new(layer)
            .validate()
            .into_iter()
            .map(|e| format!("{e:?}"))
            .collect();
        assert!(
            errors.is_empty(),
            "D103 ontology layer `{name}` must validate with 0 errors; got:\n{errors:#?}"
        );
    }
}

#[test]
fn synthetic_end_to_end_records_and_grounds_validate() {
    let src = r#"
namespace core          = "urn:eigenius:core";
namespace lexicon       = "urn:eigenius:lexicon";
namespace eigentt       = "urn:eigenius:eigentt";
namespace prov          = "urn:eigenius:prov";
namespace agent         = "urn:eigenius:prov:agent";
namespace units         = "urn:eigenius:units";
namespace enc           = "urn:eigenius:encoding";
namespace justification = "urn:eigenius:justification";
namespace variant       = "urn:eigenius:variant";
namespace clinical      = "urn:eigenius:clinical";
namespace acquisition   = "urn:eigenius:acquisition";
namespace clinvar       = "urn:eigenius:clinvar";
namespace popgen        = "urn:eigenius:popgen";
namespace structure     = "urn:eigenius:structure";
namespace predictors    = "urn:eigenius:predictors";
namespace sequence      = "urn:eigenius:sequence";
namespace align         = "urn:eigenius:align";
namespace ex            = "urn:eigenius:test:d103";

// Synthetic gene, references, alleles, and locations
resource ex:gene_g1 : lexicon:Entity {
    core:short_name = "G1";
}

resource ex:tx_1 : variant:Transcript {
    variant:accession = "NM_000001.1";
    variant:gene      = ex:gene_g1;
}

resource ex:prot_1 : variant:ProteinProduct {
    variant:accession = "HGNC:1:p";
    variant:gene      = ex:gene_g1;
}

resource ex:nuc_c301 : variant:NucleotideAllele {
    variant:reference = "NM_000001.1";
    variant:start     = 301;
    variant:end       = 301;
    variant:ref_state = "A";
    variant:alt       = "G";
    variant:hgvs      = "c.301A>G";
}

resource ex:prot_101 : variant:ProteinAllele {
    variant:reference = "HGNC:1:p";
    variant:start     = 101;
    variant:end       = 101;
    variant:ref_state = "Ile";
    variant:alt       = "Val";
    variant:hgvs      = "p.Ile101Val";
}

resource ex:loc_101 : variant:Location {
    variant:reference = "HGNC:1:p";
    variant:start     = 101;
    variant:end       = 101;
    variant:ref_state = "Ile";
}

resource ex:loc_109 : variant:Location {
    variant:reference = "HGNC:1:p";
    variant:start     = 109;
    variant:end       = 109;
    variant:ref_state = "Phe";
}

// 1. Acquisition: HTTP 200 and HTTP 404 negative evidence
resource ex:fetch_popgen : acquisition:HttpExchange {
    acquisition:url           = "https://example.org/api/popgen";
    acquisition:http_method   = "POST";
    acquisition:outcome       = acquisition:outcome_ok;
    acquisition:http_status   = 200;
    acquisition:status_source = acquisition:status_observed;
    acquisition:body_sha256   = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    acquisition:byte_length   = 512;
}

resource ex:fetch_404 : acquisition:HttpExchange {
    acquisition:url           = "https://example.org/api/missing";
    acquisition:http_method   = "GET";
    acquisition:outcome       = acquisition:outcome_http_error;
    acquisition:http_status   = 404;
    acquisition:status_source = acquisition:status_observed;
    eigentt:proposition       = type_expr(acquisition:ReturnedStatus(ex:fetch_404, 404));
}

resource ex:archive_popgen : acquisition:ArchivedArtifact {
    acquisition:body_sha256   = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    acquisition:relative_path = "data/popgen.json";
    acquisition:media_type    = "application/json";
    acquisition:fetched_by    = ex:fetch_popgen;
}

// 2. ClinVar structured VariationRecord
resource ex:cv_rel : clinvar:Release {
    clinvar:release_date = "2026-09-01";
}

resource ex:vcv_1 : clinvar:VariationRecord {
    clinvar:vcv_accession   = "VCV000000001";
    clinvar:vcv_version     = 2;
    clinvar:record_release  = ex:cv_rel;
    clinvar:record_allele   = ex:nuc_c301;
    clinvar:classification  = clinical:benign;
    clinvar:review_status   = clinvar:single_submitter;
    clinvar:star_rating     = 1;
    clinvar:submitter_count = 1;
    clinvar:has_conflict    = false;
    eigentt:proposition     = type_expr(clinvar:HasReviewStars(ex:cv_rel, ex:nuc_c301, 1));
}

// 3. Popgen release, allele count observation (with ObservationTrace + Grounds), and RMC window
resource ex:pg_rel : popgen:DatasetRelease {
    popgen:dataset_name     = "synthetic-pop";
    popgen:dataset_version  = "v4.1";
    popgen:reference_genome = "GRCh38";
    popgen:callset_kind     = popgen:callset_joint;
}

resource ex:pg_obs : popgen:AlleleObservation {
    popgen:in_release       = ex:pg_rel;
    popgen:observed_allele  = ex:nuc_c301;
    popgen:allele_count     = 13;
    popgen:allele_number    = 1000000;
    popgen:hemizygous_count = 4;
    popgen:allele_frequency = r"13/1000000";
    popgen:filter_pass      = true;
    eigentt:proposition     = type_expr(
        popgen:ObservedHemizygousCount(ex:pg_rel, ex:nuc_c301, units:mk_quantity(r"4", 0))
    );
}

resource ex:pg_obs_trace : prov:ObservationTrace {
    prov:resource         = ex:pg_obs;
    prov:was_generated_by = ex:fetch_popgen;
    prov:timestamp        = "2026-09-16T00:00:00Z";
    prov:observed_at      = "2026-09-16T00:00:00Z";
}

resource ex:pg_concl : justification:Conclusion {
    justification:grounds_judgement = holds(
        eigentt:logic_kernel,
        type_expr(observed(
            "urn:eigenius:test:d103:pg_obs",
            popgen:ObservedHemizygousCount(ex:pg_rel, ex:nuc_c301, units:mk_quantity(r"4", 0)),
            ()
        )),
        type_expr(justification:Grounds(
            popgen:ObservedHemizygousCount(ex:pg_rel, ex:nuc_c301, units:mk_quantity(r"4", 0))
        ))
    );
}

resource ex:rmc_win : popgen:RegionalConstraintWindow {
    popgen:in_release          = ex:pg_rel;
    popgen:window_location     = ex:loc_101;
    popgen:obs_mis             = 20;
    popgen:exp_mis             = r"213/10";
    popgen:oe_ratio            = r"47/50";
    popgen:p_depletion         = r"9/20";
    popgen:passed_qc           = false;
    popgen:has_no_rmc_evidence = true;
}

// 4. Structure protein, model, metric, and meter-normalised atom contact
resource ex:prot_entry : structure:ProteinEntry {
    structure:uniprot_accession = "Q00001";
    structure:gene_symbol       = "G1";
    structure:sequence_length   = 500;
}

resource ex:model_1 : structure:StructureModel {
    structure:model_id            = "1XYZ";
    structure:model_source        = structure:source_pdb;
    structure:experimental_method = "SOLUTION NMR";
    structure:of_protein          = ex:prot_entry;
}

resource ex:contact_1 : structure:AtomContact {
    structure:in_model          = ex:model_1;
    structure:residue_a         = ex:loc_101;
    structure:residue_b         = ex:loc_109;
    structure:distance_meters   = r"173/500000000000";
    structure:includes_backbone = false;
    eigentt:proposition         = type_expr(
        structure:ContactsAtDistance(ex:model_1, ex:loc_101, ex:loc_109, units:mk_quantity(r"173/500000000000", 0))
    );
}

// 5. Predictors tool release & variant score
resource ex:tool_1 : predictors:ToolRelease {
    predictors:tool_name    = "SyntheticPred";
    predictors:tool_version = "1.0";
    predictors:aggregator   = "dbNSFP";
}

resource ex:score_1 : predictors:VariantScore {
    predictors:from_tool        = ex:tool_1;
    predictors:scored_allele    = ex:nuc_c301;
    predictors:metric_name      = "score";
    predictors:raw_score        = r"653/10000";
    predictors:categorical_call = "T";
    eigentt:proposition         = type_expr(
        predictors:HasScore(ex:tool_1, ex:nuc_c301, "score", units:mk_quantity(r"653/10000", 0))
    );
}

// 6. Sequence annotation, conservation, editing window, dbSNP
resource ex:tx_ann : sequence:TranscriptAnnotation {
    sequence:of_transcript = ex:tx_1;
    sequence:mane_select   = true;
    sequence:cds_length    = 1500;
    sequence:exon_count    = 6;
}

resource ex:cons_1 : sequence:OrthologConservation {
    sequence:at_location      = ex:loc_101;
    sequence:clade            = "mammals";
    sequence:species_count    = 50;
    sequence:conserved_count  = 48;
    sequence:alignment_source = "Ensembl Compara";
}

resource ex:win_1 : sequence:EditingWindow {
    sequence:of_transcript     = ex:tx_1;
    sequence:editor_name       = "SyntheticBE";
    sequence:protospacer_seq   = "ACGTACGTACGTACGTACGT";
    sequence:pam_seq           = "NGG";
    sequence:target_allele     = ex:nuc_c301;
    sequence:bystander_alleles = [ex:nuc_c301];
}

resource ex:dbsnp_1 : sequence:DbSnpRecord {
    sequence:rsid             = "rs1000001";
    sequence:dbsnp_allele     = ex:nuc_c301;
    sequence:reference_genome = "GRCh38";
}

// 7. Align: direct table-row claim, entity bridge, compression record, exemption record
resource ex:doc_unit_table : enc:DiscourseUnit {
    enc:prose     = "c.301A>G | Benign | 1 star";
    enc:unit_kind = enc:kind_table;
    enc:locator   = "table-1-row-1";
}

resource ex:doc_unit_exempt : enc:DiscourseUnit {
    enc:prose     = "Not medical advice.";
    enc:unit_kind = enc:kind_prose;
    enc:locator   = "caveat-1";
}

resource ex:table_claim_1 : align:TableRowClaim {
    align:source_unit      = ex:doc_unit_table;
    prov:was_attributed_to = agent:eigenius_core_team;
    eigentt:proposition    = type_expr(clinvar:ListedAs(ex:cv_rel, ex:nuc_c301, clinical:benign));
}

resource ex:doc_local_allele : lexicon:Entity {
    core:short_name = "ni_p_ile101val";
}

resource ex:align_decl : justification:Declaration {
    prov:was_attributed_to = agent:eigenius_core_team;
    eigentt:proposition    = type_expr(align:DenotesEntity(ex:doc_local_allele, ex:prot_101));
}

resource ex:desig_decl : justification:Declaration {
    prov:was_attributed_to = agent:eigenius_core_team;
    eigentt:proposition    = type_expr(align:DesignationDenotes(ex:prot_101, ex:nuc_c301));
}

resource ex:comp_rec : align:CompressionRecord {
    align:summary_claim       = ex:table_claim_1;
    align:compresses          = [ex:pg_obs];
    align:required_qualifiers = [ex:vcv_1];
    prov:was_attributed_to    = agent:eigenius_core_team;
}

resource ex:exempt_rec : align:ExemptionRecord {
    align:exempt_unit      = ex:doc_unit_exempt;
    align:exemption_reason = "Standing scope disclaimer.";
    prov:was_attributed_to = agent:eigenius_core_team;
}
"#;
    let layer = compile_layer("d103-synthetic", src, d103_ontology_chain());
    let errors: Vec<String> = Validator::new(layer)
        .validate()
        .into_iter()
        .map(|e| format!("{e:?}"))
        .collect();
    assert!(
        errors.is_empty(),
        "Synthetic D103 end-to-end instances must validate with 0 errors; got:\n{errors:#?}"
    );
}

#[test]
fn popgen_refuses_protein_allele_in_nucleotide_slot() {
    let src = r#"
namespace core    = "urn:eigenius:core";
namespace eigentt = "urn:eigenius:eigentt";
namespace units   = "urn:eigenius:units";
namespace variant = "urn:eigenius:variant";
namespace popgen  = "urn:eigenius:popgen";
namespace ex      = "urn:eigenius:test:d103neg";

resource ex:pg_rel : popgen:DatasetRelease {
    popgen:dataset_name     = "synthetic-pop";
    popgen:dataset_version  = "v4.1";
    popgen:reference_genome = "GRCh38";
}

resource ex:prot_101 : variant:ProteinAllele {
    variant:reference = "HGNC:1:p";
    variant:start     = 101;
    variant:end       = 101;
    variant:ref_state = "Ile";
    variant:alt       = "Val";
}

resource ex:bad_prop : core:Resource {
    eigentt:proposition = type_expr(
        popgen:ObservedHemizygousCount(ex:pg_rel, ex:prot_101, units:mk_quantity(r"4", 0))
    );
}
"#;
    let layer = compile_layer("d103-neg-protein", src, d103_ontology_chain());
    let errors: Vec<String> = Validator::new(layer)
        .validate()
        .into_iter()
        .map(|e| format!("{e:?}"))
        .collect();
    assert!(
        errors
            .iter()
            .any(|e| e.contains("does not inhabit class urn:eigenius:variant:NucleotideAllele")),
        "popgen:ObservedHemizygousCount must reject a variant:ProteinAllele; got:\n{errors:#?}"
    );
}

#[test]
fn structure_contact_refuses_wrong_dimension_where_meters_required() {
    let src = r#"
namespace core      = "urn:eigenius:core";
namespace eigentt   = "urn:eigenius:eigentt";
namespace units     = "urn:eigenius:units";
namespace variant   = "urn:eigenius:variant";
namespace structure = "urn:eigenius:structure";
namespace ex        = "urn:eigenius:test:d103dim";

resource ex:model_1 : structure:StructureModel {
    structure:model_id     = "1XYZ";
    structure:model_source = structure:source_pdb;
}

resource ex:loc_101 : variant:Location {
    variant:reference = "HGNC:1:p";
    variant:start     = 101;
    variant:end       = 101;
}

resource ex:loc_109 : variant:Location {
    variant:reference = "HGNC:1:p";
    variant:start     = 109;
    variant:end       = 109;
}

// Passing a mass quantity (Quantity(u"kg")) into a length slot (Quantity(u"m")) must fail.
resource ex:bad_dim : core:Resource {
    eigentt:proposition = type_expr(
        structure:ContactsAtDistance(ex:model_1, ex:loc_101, ex:loc_109, units:quantity(5, "kg"))
    );
}
"#;
    let layer = compile_layer("d103-neg-dim", src, d103_ontology_chain());
    let errors: Vec<String> = Validator::new(layer)
        .validate()
        .into_iter()
        .map(|e| format!("{e:?}"))
        .collect();
    assert!(
        !errors.is_empty(),
        "structure:ContactsAtDistance must reject a dimensionless Quantity(u\"1\") in a Quantity(u\"m\") slot"
    );
}
