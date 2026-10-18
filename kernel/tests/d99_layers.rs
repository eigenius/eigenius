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

//! The `variant` and `clinical` layers (D99 §1–§4, §6) on the real bootstrap chain.
//!
//! - **Resolutions are types.** `TranslatesTo` runs from a nucleotide allele to a protein allele
//!   and refuses the reverse.
//! - **`At` is computed.** It is the identity of reference and interval, so it is proved by `refl`
//!   and refused where a field differs.
//! - **Claim 6.** It is stated in the parser's existential encoding, and the layer above uses it
//!   through `instantiate`.
//! - **Carrying is at nucleotide resolution.** `Carries` takes a nucleotide allele. A row known
//!   only at protein resolution is an existential over the alleles that translate to its change,
//!   and `Carries` of a protein allele is refused.

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

/// bootstrap → `variant` → `clinical` → the SYNGENE fixture → the layer that uses claim 6.
fn chain() -> (Arc<Layer>, Arc<Layer>, Arc<Layer>) {
    let (variant, _, syngene, used) = full_chain();
    (variant, syngene, used)
}

/// As [`chain`], with the `clinical` layer and the case fixture as a fourth layer.
fn full_chain() -> (Arc<Layer>, Arc<Layer>, Arc<Layer>, Arc<Layer>) {
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
    let syngene = compile_layer(
        "syngene",
        include_str!("fixtures/d99_variant.esl"),
        Arc::clone(&clinical),
    );
    let used = compile_layer(
        "syngene-use",
        include_str!("fixtures/d99_variant_use.esl"),
        Arc::clone(&syngene),
    );
    let cases = compile_layer(
        "syngene-cases",
        include_str!("fixtures/d99_clinical.esl"),
        Arc::clone(&used),
    );
    (variant, clinical, syngene, cases)
}

fn all_errors(layer: &Arc<Layer>) -> Vec<String> {
    Validator::new(Arc::clone(layer))
        .validate()
        .into_iter()
        .map(|e| format!("{e:?}"))
        .collect()
}

fn errors(layer: &Arc<Layer>, local: &str) -> Vec<String> {
    let target = format!("urn:eigenius:test:syngene:{local}");
    Validator::new(Arc::clone(layer))
        .validate()
        .into_iter()
        .filter(|e| e.resource_id.as_ref().is_some_and(|i| i.as_str() == target))
        .map(|e| format!("{e:?}"))
        .collect()
}

#[test]
fn the_variant_layer_validates() {
    let (variant, _, _) = chain();
    assert_eq!(all_errors(&variant), Vec::<String>::new());
}

#[test]
fn the_alleles_and_locations_validate() {
    let (_, syngene, _) = chain();
    for local in [
        "SYNGENE",
        "NM_000010_7",
        "protein",
        "c_130C_T",
        "c_131T_C",
        "p_Leu44Phe",
        "p_Leu44Pro",
        "Leu44",
        "Leu45",
    ] {
        assert_eq!(errors(&syngene, local), Vec::<String>::new(), "{local}");
    }
}

#[test]
fn translation_runs_from_a_nucleotide_allele_to_a_protein_allele() {
    let (_, syngene, _) = chain();
    assert_eq!(errors(&syngene, "translates_131"), Vec::<String>::new());
    assert!(
        !errors(&syngene, "translates_backwards").is_empty(),
        "TranslatesTo(protein allele, nucleotide allele) must be ill-typed"
    );
}

/// `At` unfolds to the identity of reference and interval, so `refl` proves it where the fields
/// agree.
#[test]
fn a_protein_allele_is_proved_at_its_location() {
    let (_, syngene, _) = chain();
    assert_eq!(errors(&syngene, "pro_at_leu44"), Vec::<String>::new());
}

#[test]
fn a_protein_allele_is_not_proved_at_another_location() {
    let (_, syngene, _) = chain();
    let errs = errors(&syngene, "pro_at_leu45");
    assert!(
        errs.iter().any(|e| e.contains("LitInt(45) ≠ LitInt(44)")),
        "refl must not prove Id(44, 45), got {errs:?}"
    );
}

#[test]
fn claim_6_is_a_proposition_in_the_parsers_encoding() {
    let (_, syngene, _) = chain();
    assert_eq!(errors(&syngene, "claim_6"), Vec::<String>::new());
}

/// D99 §3's open point, settled: `instantiate` binds `T : Type 1`, and admits claim 6's `R : Prop`.
/// Using the claim at a named allele then needs a ground for its antecedent — every responsive
/// allele at Leu44 is that allele — which nothing supplies.
#[test]
fn claim_6_instantiates_at_a_proposition() {
    let (_, _, used) = chain();
    assert_eq!(errors(&used, "claim_6_at_pro"), Vec::<String>::new());
}

#[test]
fn the_clinical_layer_validates() {
    let (_, clinical, _, _) = full_chain();
    assert_eq!(all_errors(&clinical), Vec::<String>::new());
}

#[test]
fn a_registry_genotype_row_is_a_carries_declaration() {
    let (_, _, _, cases) = full_chain();
    for local in ["proband_002", "case_002", "p_Gly88Ser", "carries_131"] {
        assert_eq!(errors(&cases, local), Vec::<String>::new(), "{local}");
    }
}

/// D99 §2 — the row with a protein change and no nucleotide change is written at the resolution it
/// has.
#[test]
fn a_protein_only_row_is_an_existential_over_nucleotide_alleles() {
    let (_, _, _, cases) = full_chain();
    assert_eq!(errors(&cases, "carries_gly88ser"), Vec::<String>::new());
}

#[test]
fn a_person_does_not_carry_a_protein_allele() {
    let (_, _, _, cases) = full_chain();
    let errs = errors(&cases, "carries_protein");
    assert!(
        errs.iter()
            .any(|e| e.contains("does not inhabit class urn:eigenius:variant:NucleotideAllele")),
        "Carries(individual, protein allele, state) must be ill-typed, got {errs:?}"
    );
}
