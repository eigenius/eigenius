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

//! Kernel-checked disequality (D99 §10): two resources are distinct when a field of theirs holds
//! distinct literals.
//!
//! The proof is a chain term — congruence over the field (`eigentt:J` on `eigentt:field`), closed
//! by `eigentt:apart` — carried as a `justification:proof_judgement`, which admits a `Verified`
//! witness that a later conclusion cites. Built on the real bootstrap chain: the fixture uses
//! `lexicon:Entity`, `logic:False` and the justification vocabulary as they ship.
//!
//! The refusals matter as much as the proof. Two resources that agree on every field are not
//! proved distinct, two names are never apart by themselves, and an apartness about the wrong
//! literals does not connect to the fields — so nothing here assumes unique names.

use std::sync::Arc;

use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::validation::Validator;

/// The bootstrap chain, the fixture layer, and the citing layer above it.
fn chain() -> (Arc<Layer>, Arc<Layer>) {
    let head = Arc::clone(
        eigenius_kernel::bootstrap::bootstrap()
            .expect("bootstrap")
            .head(),
    );
    let layer = |name: &str, src: &str, parent: Arc<Layer>| {
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
    };
    let alleles = layer(
        "apart-alleles",
        include_str!("fixtures/disequality_by_field.esl"),
        head,
    );
    let cite = layer(
        "apart-cite",
        include_str!("fixtures/disequality_by_field_cite.esl"),
        Arc::clone(&alleles),
    );
    (alleles, cite)
}

/// Every validation error `layer` reports on `urn:eigenius:test:apart:<local>`.
fn errors(layer: &Arc<Layer>, local: &str) -> Vec<String> {
    let target = format!("urn:eigenius:test:apart:{local}");
    Validator::new(Arc::clone(layer))
        .validate()
        .into_iter()
        .filter(|e| e.resource_id.as_ref().is_some_and(|i| i.as_str() == target))
        .map(|e| format!("{e:?}"))
        .collect()
}

#[test]
fn distinct_start_positions_prove_two_alleles_distinct() {
    let (alleles, _) = chain();
    assert_eq!(errors(&alleles, "by_start"), Vec::<String>::new());
}

#[test]
fn distinct_alternate_states_prove_two_alleles_distinct() {
    let (alleles, _) = chain();
    assert_eq!(errors(&alleles, "by_alt"), Vec::<String>::new());
}

/// The proof admits a `Verified` witness: a conclusion one layer up cites it through
/// `Grounds.verified`, and the citation checks.
#[test]
fn the_disequality_grounds_a_citing_conclusion_as_verified() {
    let (_, cite) = chain();
    assert_eq!(errors(&cite, "cites_by_start"), Vec::<String>::new());
}

/// The citation can fail: a resource carrying no proof admits no `Verified` witness.
#[test]
fn citing_a_resource_without_a_proof_is_refused() {
    let (_, cite) = chain();
    let errs = errors(&cite, "cites_unproved");
    assert!(
        errs.iter().any(|e| e.contains("IsVerifiedAs")),
        "expected a missing Verified witness, got {errs:?}"
    );
}

/// `c131_twin` agrees with `c131` on every field. The congruence lands on `Id(131, 131)`, which
/// `apart` refuses — no unique-names assumption makes the two distinct.
#[test]
fn resources_that_agree_on_every_field_are_not_proved_distinct() {
    let (alleles, _) = chain();
    let errs = errors(&alleles, "twin");
    assert!(
        errs.iter()
            .any(|e| e.contains("not distinct canonical literals")),
        "expected the apartness side condition to refuse, got {errs:?}"
    );
}

#[test]
fn two_names_are_never_apart_by_themselves() {
    let (alleles, _) = chain();
    let errs = errors(&alleles, "by_name");
    assert!(
        errs.iter()
            .any(|e| e.contains("not distinct canonical literals")),
        "expected names to be refused as non-literals, got {errs:?}"
    );
}

/// `apart(130, 132)` is a sound proof of `Id(130, 132) -> False`, but the congruence yields
/// `Id(130, 131)`: the proof does not connect, and the judgement fails.
#[test]
fn an_apartness_about_the_wrong_literals_does_not_connect() {
    let (alleles, _) = chain();
    let errs = errors(&alleles, "disconnected");
    assert!(
        errs.iter().any(|e| e.contains("TermIllTyped")),
        "expected the proof to be ill-typed, got {errs:?}"
    );
}
