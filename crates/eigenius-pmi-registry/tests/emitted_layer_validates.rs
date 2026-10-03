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

//! The converted synthetic registry compiles and validates on the real bootstrap chain, under
//! the `variant` and `clinical` layers.
//!
//! WordNet and HPO are not in the bootstrap chain, so a stub layer declares the one WordNet verb
//! and the HP classes the cases code; the HPO the label check reads is built from the cases'
//! own labels, so every phenotype row converts. The test against the loaded lexicon is
//! `crates/eigenius-encoding/tests/uab_registry_case.rs`, against a snapshot.

use std::sync::Arc;

use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::validation::Validator;
use eigenius_pmi_registry::convert::{convert, Conversion, Disposition};
use eigenius_pmi_registry::hpo_history::HpoHistory;
use eigenius_pmi_registry::record::Package;

const CASES: &str = include_str!(
    "../../../experiments/uab/UAB Round 1/02-synthetic-pmi-registry/synthetic-cases.json"
);

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

fn hp_ids(package: &Package) -> Vec<String> {
    let mut ids: Vec<String> = package
        .cases
        .iter()
        .flat_map(|c| c.linked_records.hp_terms.iter().map(|t| t.hp_id.clone()))
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// An HPO whose only release labels each coded term as the registry does.
fn agreeing_hpo(package: &Package) -> HpoHistory {
    let nodes: Vec<serde_json::Value> = package
        .cases
        .iter()
        .flat_map(|c| &c.linked_records.hp_terms)
        .map(|t| {
            serde_json::json!({
                "id": format!("http://purl.obolibrary.org/obo/HP_{}", &t.hp_id[3..]),
                "lbl": t.phenotype,
                "type": "CLASS",
            })
        })
        .collect();
    let doc = serde_json::json!({ "graphs": [ {
        "meta": { "version": "http://purl.obolibrary.org/obo/hp/releases/2026-09-01/hp.json" },
        "nodes": nodes,
    } ] });
    HpoHistory::from_parts(&doc.to_string(), Vec::new()).expect("hpo")
}

fn stubs(package: &Package) -> String {
    let mut s = String::from(
        "namespace lexicon = \"urn:eigenius:lexicon\";\n\
         namespace wn = \"urn:eigenius:wn\";\n\
         namespace hp = \"urn:obo:HP\";\n\
         axiom wn:v00065370_t : lexicon:Entity -> lexicon:Entity -> Prop\n",
    );
    for id in hp_ids(package) {
        s.push_str(&format!(
            "class hp:'{}' {{ description = \"{id} (stub)\"; }}\n",
            &id[3..]
        ));
    }
    s
}

/// bootstrap → `variant` → `clinical` → stubs → the converted registry.
fn converted(package: &Package) -> (Conversion, Arc<Layer>) {
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
    let stubs = compile_layer("stubs", &stubs(package), clinical);
    let conversion = convert(package, &agreeing_hpo(package), "synthetic-cases.json");
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

#[test]
fn the_synthetic_registry_validates() {
    let package: Package = serde_json::from_str(CASES).unwrap();
    let (conversion, layer) = converted(&package);
    assert_eq!(errors(&layer), Vec::<String>::new());
    assert!(conversion
        .findings
        .iter()
        .all(|f| f.disposition != Disposition::Held));
    assert!(has(
        &layer,
        "urn:eigenius:pmi:SYN_26_002_carries_NM_000010_7_131_C"
    ));
    assert!(has(&layer, "urn:eigenius:pmi:SYN_26_002_hp_0000103"));
    // SYN-25-003 has no genomic-data-sharing consent: its case converts, its variants do not.
    assert!(has(&layer, "urn:eigenius:pmi:SYN_25_003_hp_0003201"));
    assert!(!has(&layer, "urn:eigenius:pmi:gene:SYNMET"));
}

#[test]
fn compound_heterozygous_rows_are_heterozygous_and_in_trans() {
    let mut package: Package = serde_json::from_str(CASES).unwrap();
    for case in &mut package.cases {
        case.genomic_consent = true;
    }
    let (_, layer) = converted(&package);
    assert_eq!(errors(&layer), Vec::<String>::new());
    assert!(has(
        &layer,
        "urn:eigenius:variant:allele:NM_000020_5_1105_del"
    ));
    assert!(has(
        &layer,
        "urn:eigenius:variant:allele:HGNC_000020_p_369_Leufs_12"
    ));
    assert!(has(&layer, "urn:eigenius:pmi:SYN_25_003_in_trans_SYNMET"));
}
