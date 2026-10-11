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

//! The notation glossary source (D101 §step 2), on the plain bootstrap chain.
//!
//! `Ile380Thr` blocked 20 units of the XIAP report and the apposition source never offered it.
//! What is pinned here is the TIERING, because the tiers are the whole design: a token that states
//! its reference accession becomes a `variant:Allele` with true identity; one that does not closes
//! the gap lexically with its parse recorded, and no accession is invented.

use std::sync::Arc;

use eigenius_kernel::dcg::notation_augmentation;
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::Iri;

fn bootstrap_head() -> Arc<Layer> {
    Arc::clone(
        eigenius_kernel::bootstrap::bootstrap()
            .expect("bootstrap")
            .head(),
    )
}

/// `variant` is NOT in BOOTSTRAP_CHAIN — it loads above the tip, after `lexicon`, because its
/// classes subclass `lexicon:Entity`. So tier 1 is only reachable on a chain that has it.
fn with_variant(head: Arc<Layer>) -> Arc<Layer> {
    let src = include_str!("../../ontologies/variant/variant.esl");
    let resources = esl::compile(src, &head).expect("variant compiles over the bootstrap tip");
    let mut b = LayerBuilder::new("variant", Some(head));
    for r in resources {
        b.add_resource(r).expect("add");
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("well-formed IRI")
}

#[test]
fn the_bare_change_closes_the_gap_without_inventing_an_accession() {
    let head = bootstrap_head();
    let aug = notation_augmentation(
        &head,
        "The bystander edit installs Ile380Thr in the UBA domain.",
    );

    assert!(
        !aug.added.is_empty(),
        "the gap must close — a unit carrying Ile380Thr returned missing-lexeme before this source"
    );
    let ni = aug
        .supporting
        .iter()
        .find(|r| {
            r.id()
                .map(|i| i.as_str())
                .unwrap_or("")
                .contains("ile380thr")
        })
        .expect("a doc-local individual for the recognised token");

    // Tier 2: typed lexicon:Entity, NOT an allele — the accession is not in the token, and
    // variant:Allele is identified by it ("one change on two references is two alleles").
    assert!(
        ni.is_instance_of(&iri("urn:eigenius:lexicon:Entity")),
        "tier 2 is a lexical entity, got {:?}",
        ni.is_a()
    );
    assert!(
        ni.get(&iri("urn:eigenius:variant:reference")).is_none(),
        "no accession may be invented for a bare change"
    );
    // And the provenance says which source did it.
    assert!(
        aug.added.iter().all(|b| matches!(
            b.provenance.method,
            eigenius_kernel::dcg::augment::ResolutionMethod::NotationRecognized
        )),
        "the binding is attributed to notation recognition"
    );
}

#[test]
fn a_qualified_notation_becomes_an_allele_with_true_identity() {
    let head = with_variant(bootstrap_head());
    let aug = notation_augmentation(
        &head,
        "The correction targets NM_001167.4:c.1141C>T directly.",
    );

    let ni = aug
        .supporting
        .iter()
        .find(|r| {
            r.id()
                .map(|i| i.as_str())
                .unwrap_or("")
                .contains("nm_001167")
        })
        .expect("a doc-local individual for the qualified notation");

    assert!(
        ni.is_instance_of(&iri("urn:eigenius:variant:NucleotideAllele")),
        "tier 1 is an allele, got {:?}",
        ni.is_a()
    );
    // The accession the NOTATION stated — not one chosen for it.
    assert_eq!(
        ni.get(&iri("urn:eigenius:variant:reference"))
            .and_then(|v| v.as_str().map(str::to_string))
            .as_deref(),
        Some("NM_001167.4")
    );
    // The parse, as typed fields. This is the point: nothing is carried by the name.
    assert_eq!(
        ni.get(&iri("urn:eigenius:variant:start"))
            .and_then(|v| v.as_integer()),
        Some(1141)
    );
    assert_eq!(
        ni.get(&iri("urn:eigenius:variant:ref_state"))
            .and_then(|v| v.as_str().map(str::to_string))
            .as_deref(),
        Some("C")
    );
    assert_eq!(
        ni.get(&iri("urn:eigenius:variant:alt"))
            .and_then(|v| v.as_str().map(str::to_string))
            .as_deref(),
        Some("T")
    );
}

#[test]
fn without_the_variant_layer_tier_one_degrades_rather_than_dangling() {
    // The glossary runs inside the kernel, whose chain ends at `encoding`. Emitting
    // variant:NucleotideAllele on a chain without the layer would be an unresolvable class.
    let head = bootstrap_head();
    let aug = notation_augmentation(
        &head,
        "The correction targets NM_001167.4:c.1141C>T directly.",
    );
    let ni = aug
        .supporting
        .iter()
        .find(|r| {
            r.id()
                .map(|i| i.as_str())
                .unwrap_or("")
                .contains("nm_001167")
        })
        .expect("still recognised, still closes the gap");
    assert!(
        ni.is_instance_of(&iri("urn:eigenius:lexicon:Entity")),
        "degrades to tier 2, got {:?}",
        ni.is_a()
    );
    assert!(ni.get(&iri("urn:eigenius:variant:reference")).is_none());
}

#[test]
fn ordinary_prose_and_product_names_yield_nothing() {
    let head = bootstrap_head();
    // Every token here is one the XIAP corpus contains and that must NOT be read as notation —
    // the editors, predictors and databases that remain OOV after this source.
    let aug = notation_augmentation(
        &head,
        "ABE9 and ABE8e were scored by SIFT4G, AlphaMissense, ClinPred and BayesDel against \
         MaveDB and HGMD at v4.1, which the paywalled text calls a meta-classifier.",
    );
    assert!(
        aug.added.is_empty() && aug.supporting.is_empty(),
        "no notation here, got {:?}",
        aug.supporting
            .iter()
            .map(|r| r.id().map(|i| i.as_str()).unwrap_or("").to_string())
            .collect::<Vec<_>>()
    );
}
