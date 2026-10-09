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

//! D100 rung 1 — claim C-0020 of the XIAP certification log, on the real bootstrap chain.
//!
//! «p.Ile380Val is carried by 4 hemizygous males.» Its subject is a PROTEIN allele; its predicate
//! is a count gnomAD computes over GENOTYPES at a nucleotide allele. The certificate's own
//! `reading` field resolves the one to the other by hand, which is the step a type system should
//! force rather than leave to a human.
//!
//! Chain: bootstrap → `variant` → this experiment's vocabulary. In-memory, no database.

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

fn chain() -> Arc<Layer> {
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
    let xiap = compile_layer(
        "xiap",
        include_str!("../../experiments/xiap-c0020/xiap.esl"),
        variant,
    );
    compile_layer(
        "xiap-observation",
        include_str!("../../experiments/xiap-c0020/observation.esl"),
        xiap,
    )
}

/// The vocabulary compiles and validates: two alleles at two resolutions, and a count predicate
/// indexed by the one the evidence is about.
#[test]
fn the_xiap_vocabulary_validates() {
    let layer = chain();
    let errors: Vec<String> = Validator::new(Arc::clone(&layer))
        .validate()
        .into_iter()
        .map(|e| format!("{e:?}"))
        .collect();
    assert!(
        errors.is_empty(),
        "the xiap layer validates against variant + bootstrap: {errors:#?}"
    );
}

/// Every validation error the chain at `head` reports about a resource under `urn:eigenius:uab:`,
/// as `(local name, message)`.
fn xiap_errors(layer: &Arc<Layer>) -> Vec<(String, String)> {
    Validator::new(Arc::clone(layer))
        .validate()
        .into_iter()
        .filter_map(|e| {
            let d = format!("{e:?}");
            let i = d
                .split("resource_id: Some(Iri(\"")
                .nth(1)?
                .split('"')
                .next()?
                .to_string();
            i.starts_with("urn:eigenius:uab:")
                .then(|| (i.rsplit(':').next().unwrap_or("").to_string(), d))
        })
        .collect()
}

/// **The sentence as the document writes it does not type.**
///
/// «p.Ile380Val is carried by 4 hemizygous males.» names a `variant:ProteinAllele` as the subject of
/// a count that `xiap:HemizygousMaleCount` indexes by `variant:NucleotideAllele` — because what
/// gnomAD counted was genotypes at X-123900531-A-G. `variant:TranslatesTo` relates the two and is
/// documented as NOT injective, so it supplies no route by which the count transfers.
///
/// This is experiment 01's residue-versus-variant refusal (#267) reproduced on real certified
/// material: the certificate's own `reading` field closes the gap by hand, which is the step a type
/// system should force rather than leave to a human.
#[test]
fn the_claim_as_written_does_not_type() {
    let as_written = compile_layer(
        "xiap-as-written",
        include_str!("../../experiments/xiap-c0020/as-written.esl"),
        chain(),
    );
    let errors = xiap_errors(&as_written);
    let on_claim: Vec<&String> = errors
        .iter()
        .filter(|(n, _)| n == "claim_c0020_as_written")
        .map(|(_, m)| m)
        .collect();
    assert!(
        !on_claim.is_empty(),
        "the protein-level claim must be refused; errors were: {errors:#?}"
    );
    let joined = on_claim
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("does not inhabit class urn:eigenius:variant:NucleotideAllele"),
        "refused for the RIGHT reason — a ProteinAllele is not a NucleotideAllele; got:\n{joined}"
    );
    // Nothing else is in error: the vocabulary and the observation stay clean.
    let rest: Vec<_> = errors
        .iter()
        .filter(|(n, _)| n != "claim_c0020_as_written")
        .collect();
    assert!(rest.is_empty(), "unexpected validation errors: {rest:#?}");
}
