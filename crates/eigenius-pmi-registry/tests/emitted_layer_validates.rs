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

//! The converter's output compiles and validates on the bootstrap chain, under the `variant` and
//! `clinical` layers, and each conversion rule does what D99 decides.
//!
//! The input is `fixtures/registry-shapes.json`, one invented record per rule. WordNet and HPO
//! are not in the bootstrap chain: a stub layer declares the WordNet verb and the HP classes, and
//! the label check reads a two-term HPO built here.
//!
//! The UAB package is not in the repository (it is gitignored), so the check that
//! `experiments/pmi-registry/04-registry.esl` is the converter's output on it is ignored and runs
//! where the package and `references/hpo` are provisioned.

use std::path::Path;
use std::sync::Arc;

use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::validation::Validator;
use eigenius_pmi_registry::convert::{convert, report, Conversion, Disposition};
use eigenius_pmi_registry::hpo_history::HpoHistory;
use eigenius_pmi_registry::record::Package;

const SHAPES: &str = include_str!("fixtures/registry-shapes.json");

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

/// HPO 2026-09-01 as far as the fixture needs it: HP:0002353's label is not the fixture's, its
/// synonym is; HP:0004918 is another term, and the fixture's label names HP:0004906.
fn hpo() -> HpoHistory {
    let doc = r#"{ "graphs": [ {
        "meta": { "version": "http://purl.obolibrary.org/obo/hp/releases/2026-09-01/hp.json" },
        "nodes": [
          { "id": "http://purl.obolibrary.org/obo/HP_0000103", "lbl": "Polyuria", "type": "CLASS" },
          { "id": "http://purl.obolibrary.org/obo/HP_0002353", "lbl": "EEG abnormality", "type": "CLASS",
            "meta": { "synonyms": [ { "pred": "hasExactSynonym", "val": "Abnormal EEG" } ] } },
          { "id": "http://purl.obolibrary.org/obo/HP_0004906", "lbl": "Hypernatremic dehydration", "type": "CLASS" },
          { "id": "http://purl.obolibrary.org/obo/HP_0004918", "lbl": "Hyperchloremic metabolic acidosis", "type": "CLASS" }
        ] } ] }"#;
    HpoHistory::from_parts(doc, Vec::new()).expect("hpo")
}

const STUBS: &str = r#"namespace lexicon = "urn:eigenius:lexicon";
namespace wn = "urn:eigenius:wn";
namespace hp = "urn:obo:HP";
axiom wn:v00065370_t : lexicon:Entity -> lexicon:Entity -> Prop
class hp:'0000103' { description = "HP:0000103 (stub)"; }
class hp:'0002353' { description = "HP:0002353 (stub)"; }
"#;

/// bootstrap → `variant` → `clinical` → stubs → the converted fixture.
fn converted() -> (Conversion, Arc<Layer>) {
    let head = Arc::clone(
        eigenius_kernel::bootstrap::bootstrap()
            .expect("bootstrap")
            .head(),
    );
    let variant = compile_layer(
        "variant",
        include_str!("../../../ontologies/variant/variant.esl"),
        head,
    );
    let clinical = compile_layer(
        "clinical",
        include_str!("../../../ontologies/clinical/clinical.esl"),
        variant,
    );
    let stubs = compile_layer("stubs", STUBS, clinical);
    let package: Package = serde_json::from_str(SHAPES).unwrap();
    let conversion = convert(&package, &hpo(), "registry-shapes.json");
    let layer = compile_layer("registry", &conversion.esl, stubs);
    (conversion, layer)
}

fn errors(layer: &Arc<Layer>) -> Vec<String> {
    Validator::new(Arc::clone(layer))
        .validate()
        .into_iter()
        .map(|e| format!("{e:?}"))
        .collect()
}

fn has(layer: &Arc<Layer>, iri: &str) -> bool {
    layer.resolve(&Iri::parse(iri).unwrap()).is_some()
}

fn finding<'a>(c: &'a Conversion, d: Disposition, case: &str, item: &str) -> Option<&'a str> {
    c.findings
        .iter()
        .find(|f| f.disposition == d && f.case == case && f.item.contains(item))
        .map(|f| f.why.as_str())
}

#[test]
fn the_converted_layer_validates() {
    let (_, layer) = converted();
    assert_eq!(errors(&layer), Vec::<String>::new());
}

#[test]
fn a_full_row_is_carried_translated_classified_and_de_novo() {
    let (_, layer) = converted();
    for local in [
        "FIX_01_carries_NM_900001_1_131_C",
        "FIX_01_translates_NM_900001_1_131_C",
        "FIX_01_classified_NM_900001_1_131_C",
        "FIX_01_impact_NM_900001_1_131_C",
    ] {
        assert!(has(&layer, &format!("urn:eigenius:pmi:{local}")), "{local}");
    }
    assert!(has(
        &layer,
        "urn:eigenius:variant:allele:HGNC_900001_p_44_Pro"
    ));
}

#[test]
fn a_protein_only_row_is_an_existential_and_its_refseq_is_unsourced() {
    let (c, layer) = converted();
    let carries = layer
        .resolve(&Iri::parse("urn:eigenius:pmi:FIX_01_carries_HGNC_900002_p_88_Ser").unwrap())
        .expect("the existential");
    assert!(format!("{carries:?}").contains("had_primary_source"));
    let refseq = layer
        .resolve(&Iri::parse("urn:eigenius:pmi:FIX_01_refseq_HGNC_900002_p_88_Ser").unwrap())
        .expect("the RefSeq declaration");
    assert!(
        !format!("{refseq:?}").contains("had_primary_source"),
        "the RefSeq has no recorded source"
    );
    assert!(finding(&c, Disposition::Noted, "FIX-01", "RefSeq NM_900002.2").is_some());
    // Unknown impact writes nothing.
    assert!(!has(
        &layer,
        "urn:eigenius:pmi:FIX_01_impact_HGNC_900002_p_88_Ser"
    ));
}

#[test]
fn compound_heterozygous_rows_are_heterozygous_and_in_trans() {
    let (_, layer) = converted();
    assert!(has(
        &layer,
        "urn:eigenius:variant:allele:NM_900003_5_1105_del"
    ));
    assert!(has(
        &layer,
        "urn:eigenius:variant:allele:HGNC_900003_p_369_Leufs_12"
    ));
    assert!(has(&layer, "urn:eigenius:pmi:FIX_01_in_trans_FIXC"));
}

#[test]
fn rows_that_disagree_with_themselves_are_held_back() {
    let (c, layer) = converted();
    let hgvs = finding(&c, Disposition::Held, "FIX-01", "FIXD").expect("FIXD held");
    assert!(hgvs.contains("is not RefSeq:Genotype"), "{hgvs}");
    assert!(!has(&layer, "urn:eigenius:pmi:gene:FIXD"));
    let label = finding(&c, Disposition::Held, "FIX-01", "HP:0004918").expect("HP:0004918 held");
    assert!(label.contains("names HP:0004906"), "{label}");
    assert!(!has(&layer, "urn:eigenius:pmi:FIX_01_hp_0004918"));
    assert!(finding(&c, Disposition::Noted, "FIX-01", "HP:0002353").is_some());
    assert!(has(&layer, "urn:eigenius:pmi:FIX_01_hp_0002353"));
}

#[test]
fn consent_gates_the_case_and_its_variants() {
    let (c, layer) = converted();
    assert!(has(&layer, "urn:eigenius:pmi:FIX_02_hp_0000103"));
    assert!(!has(&layer, "urn:eigenius:pmi:gene:FIXE"));
    assert!(finding(&c, Disposition::Skipped, "FIX-02", "Gene Info").is_some());
    assert!(!has(&layer, "urn:eigenius:pmi:FIX_03"));
    assert!(finding(&c, Disposition::Skipped, "FIX-03", "the case").is_some());
    assert!(!has(&layer, "urn:eigenius:pmi:FIX_04"));
    assert!(finding(&c, Disposition::Held, "FIX-04", "the case").is_some());
}

/// `04-registry.esl` and its report are the converter's output on the UAB package.
#[test]
#[ignore = "needs the UAB package (experiments/uab) and references/hpo"]
fn the_committed_registry_layer_is_the_converters_output() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let input = "experiments/uab/UAB Round 1/02-synthetic-pmi-registry/synthetic-cases.json";
    let read = |rel: &str| {
        std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
    };
    let package: Package = serde_json::from_str(&read(input)).unwrap();
    let hpo = HpoHistory::load(
        &root.join("references/hpo/hp.json"),
        &root.join("references/hpo/history"),
    )
    .expect("HPO");
    let conversion = convert(&package, &hpo, input);
    assert_eq!(
        conversion.esl,
        read("experiments/pmi-registry/04-registry.esl"),
        "04-registry.esl is stale: regenerate it (experiments/pmi-registry/README.md)"
    );
    assert_eq!(
        report(&conversion, input, &hpo),
        read("experiments/pmi-registry/04-registry-report.md")
    );
}
