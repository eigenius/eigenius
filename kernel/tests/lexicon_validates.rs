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

//! D62 §8 — the drafted `lexicon` layer is **Expressible**, and the kernel is
//! the **felicity oracle** over composition.
//!
//! 1. `lexicon_layer_is_expressible` — the layer (the `LexicalEntry` schema,
//!    the inductive `lexicon:Cat`, the four archetype entries, the worked
//!    composition `s_gene_depends`) compiles against core→reflection(+eigentt)
//!    and the `Validator` reports 0 errors. The four categorial archetypes
//!    (common noun → `EigonClass`, named entity → `ResourceRef`, transitive
//!    verb / adjective → `EigonAxiom`) each map onto a kernel constructor.
//!
//! 2. `felicity_filter_*` — the Semantic Felicity Condition, demonstrated where
//!    it actually fires. A STORED `type_expr` proposition is lowered + encoded,
//!    not type-checked; the check fires only when a term is routed through the
//!    checker. So we route a composition through the proven `program → check`
//!    vehicle: a binary constructor `dep(Gene, CellLine)` mirroring the verb's
//!    argument structure. Well-typed `dep(Gene, CellLine)` type-checks; the
//!    argument-swapped `dep(CellLine, Gene)` is REJECTED. The two run the
//!    *identical* pipeline differing only in argument order, so the rejection
//!    is provably the type-checker — the kernel pruning an ill-typed
//!    derivation, the heart of D62's faithful-by-construction claim.

use std::sync::Arc;

use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::nbe::check::{check, check_infer, CheckCtx};
use eigenius_kernel::nbe::env::Rho;
use eigenius_kernel::nbe::eval::eval;
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::resource::Value;
use eigenius_kernel::ontology::Iri;
use eigenius_kernel::program::eigentt_type_mirror::decode_type;
use eigenius_kernel::program::expr::parse_program;
use eigenius_kernel::validation::Validator;

fn json_layer(name: &str, parent: Option<Arc<Layer>>, sources: &[&str]) -> Arc<Layer> {
    let mut b = LayerBuilder::new(name, parent);
    for src in sources {
        for r in eigon_json::parse_document(src).expect("ontology parses") {
            b.add_resource(r).expect("ontology resource adds");
        }
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

/// core → reflection(+eigentt) — the lexicon's parent chain.
fn base_chain() -> Arc<Layer> {
    let core = json_layer(
        "core",
        None,
        &[include_str!("../../ontologies/core/core-ontology.json")],
    );
    json_layer(
        "reflection",
        Some(core),
        &[
            include_str!("../../ontologies/reflection/reflection-ontology.json"),
            include_str!("../../ontologies/eigentt/eigentt-type-fragment.json"),
            include_str!("../../ontologies/institution/institution-ontology.json"),
            include_str!("../../ontologies/ingest/ingest-ontology.json"),
        ],
    )
}

/// Compile the drafted lexicon layer against the base chain. A compile error
/// here is the *Expressible* gate failing (the kernel cannot carry the content).
fn build_lexicon() -> Arc<Layer> {
    let reflection = base_chain();
    let lexicon_src = include_str!("../../experiments/lexicon/lexicon.esl");
    let resources = esl::compile_against_layer(lexicon_src, &reflection).unwrap_or_else(|errs| {
        panic!(
            "lexicon.esl failed to compile (not Expressible):\n{}",
            errs.into_iter()
                .map(|e| format!("  - {e:?}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
    let mut b = LayerBuilder::new("lexicon", Some(reflection));
    for r in &resources {
        b.add_resource(r.clone())
            .unwrap_or_else(|e| panic!("lexicon: add_resource failed: {e:?}"));
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

#[test]
fn lexicon_layer_is_expressible() {
    let lexicon = build_lexicon();
    let errors = Validator::new(lexicon).validate();
    assert!(
        errors.is_empty(),
        "the drafted lexicon layer must validate cleanly (Expressible). \
         {} error(s):\n{}",
        errors.len(),
        errors
            .iter()
            .take(25)
            .map(|e| format!("  - {e}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

/// Route a composition through `program → check` and return the checker's
/// verdict. `dep : Gene -> CellLine -> Dep` mirrors the transitive verb's
/// argument structure; the program body applies it to two Constructed values.
/// Compile / build / parse succeed for both polarities (none type-check), so a
/// returned error is the `check` stage — the felicity filter — refusing it.
fn check_composition(src: &str) -> Result<(), String> {
    let lexicon = build_lexicon();
    let resources =
        esl::compile_against_layer(src, &lexicon).map_err(|errs| format!("compile: {errs:?}"))?;
    let mut b = LayerBuilder::new("composition", Some(lexicon));
    for r in &resources {
        b.add_resource(r.clone())
            .map_err(|e| format!("add: {e:?}"))?;
    }
    let layer = Arc::new(b.build(LayerStorage::in_memory()));

    let iri = Iri::parse("urn:eigenius:lexicon:compose").map_err(|e| format!("iri: {e:?}"))?;
    let resource = layer.resolve(&iri).ok_or("compose program not found")?;
    let (term, typ) = parse_program(&resource, &layer)?;

    let typ_val = eval(&typ, &Rho::Nil).map_err(|e| format!("eval type: {e:?}"))?;
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], layer.clone());
    check(&mut ctx, &term, &typ_val)
}

// Freshly-Constructed typed values (not chain ResourceRefs) so the check
// isolates the *type* match: a bare ResourceRef in a program body lowers to an
// unbound Var in the checker (chain entities are not free variables — a real
// D62 finding: named-entity references need explicit binding/resolution).
const COMPOSE_OK: &str = r#"
namespace core    = "urn:eigenius:core";
namespace lexicon = "urn:eigenius:lexicon";
data lexicon:Dep { dep(lexicon:Gene, lexicon:CellLine) }
program lexicon:compose : core:string -> lexicon:Dep {
    dep(Construct lexicon:Gene {}, Construct lexicon:CellLine {})
}
"#;

const COMPOSE_BAD: &str = r#"
namespace core    = "urn:eigenius:core";
namespace lexicon = "urn:eigenius:lexicon";
data lexicon:Dep { dep(lexicon:Gene, lexicon:CellLine) }
program lexicon:compose : core:string -> lexicon:Dep {
    dep(Construct lexicon:CellLine {}, Construct lexicon:Gene {})
}
"#;

#[test]
fn felicity_filter_accepts_well_typed_composition() {
    // dep(Gene, CellLine) — arguments in the categorially-required order; checks.
    check_composition(COMPOSE_OK)
        .expect("well-typed composition dep(Gene, CellLine) must type-check (felicity holds)");
}

#[test]
fn felicity_filter_rejects_swapped_arguments() {
    // dep(CellLine, Gene) — arguments swapped; the felicity filter must reject it.
    let verdict = check_composition(COMPOSE_BAD);
    assert!(
        verdict.is_err(),
        "argument-swapped composition dep(CellLine, Gene) MUST be rejected by the kernel's \
         felicity check (the composition oracle), but it was accepted: {verdict:?}"
    );
}

// ── Direct witness of the AXIOM-application path (decode → EigonAxiom → check) ──
//
// The composition tests above use a `data` constructor. These witness the
// transitive verb's actual `EigonAxiom` predicate end to end, and pin the exact
// gap from the Q1/Q2 analysis: a STORED type_expr proposition is encoded, not
// type-checked; `decode_type` only rebuilds the tree (Rule 20's commit check);
// `check_infer` is what actually enforces felicity. So a commit-time
// `check_infer` over proposition slots would catch what the decode-only gate
// misses.

/// Read a sentence's stored `lexicon:prop` (a type_expr-encoded proposition).
fn proposition_of(layer: &Arc<Layer>, sentence_iri: &str) -> Value {
    let resource = layer
        .resolve(&Iri::parse(sentence_iri).expect("sentence iri"))
        .expect("sentence resource resolves");
    let prop_iri = Iri::parse("urn:eigenius:lexicon:prop").expect("prop iri");
    resource
        .get(&prop_iri)
        .expect("sentence carries lexicon:prop")
        .clone()
}

#[test]
fn axiom_application_decodes_and_type_checks() {
    // `s_gene_depends` stores `forall (g:Gene, c:CellLine) => depends_on(g, c)`.
    let lexicon = build_lexicon();
    let prop = proposition_of(&lexicon, "urn:eigenius:lexicon:s_gene_depends");

    // Decode recovers the real predicate as `EigonAxiom` (eigentt:Axiom branch)...
    let exp = decode_type(&prop, &lexicon)
        .unwrap_or_else(|e| panic!("well-typed proposition must decode: {e}"));
    // ...and check_infer types it: a forall over a Prop is itself a Prop.
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], lexicon.clone());
    check_infer(&mut ctx, &exp).unwrap_or_else(|e| {
        panic!("axiom application depends_on(g, c) must type-check via decode→check_infer: {e}")
    });
}

const SWAPPED_SENTENCE: &str = r#"
namespace lexicon = "urn:eigenius:lexicon";
resource lexicon:s_swapped : lexicon:Sentence {
    lexicon:gloss = "ill-typed: depends_on with arguments swapped";
    lexicon:prop  = type_expr(
        forall (g : lexicon:Gene, c : lexicon:CellLine) => lexicon:depends_on(c, g)
    );
}
"#;

#[test]
fn ill_typed_axiom_application_decodes_but_check_infer_rejects() {
    let lexicon = build_lexicon();

    // Storage path: the swapped proposition COMPILES and commits cleanly —
    // encoding does not type-check (Finding 1).
    let resources = esl::compile_against_layer(SWAPPED_SENTENCE, &lexicon)
        .expect("swapped sentence compiles (storage encodes, does not type-check)");
    let mut b = LayerBuilder::new("swapped", Some(lexicon.clone()));
    for r in &resources {
        b.add_resource(r.clone()).expect("add swapped sentence");
    }
    let swapped_layer = Arc::new(b.build(LayerStorage::in_memory()));

    let prop = proposition_of(&swapped_layer, "urn:eigenius:lexicon:s_swapped");

    // Decode SUCCEEDS — the tree is well-formed and every ConstRef resolves, so
    // Rule 20's decode-only commit check would PASS this ill-typed proposition.
    let exp = decode_type(&prop, &swapped_layer)
        .unwrap_or_else(|e| panic!("ill-typed proposition still decodes (decode ≠ check): {e}"));

    // check_infer REJECTS it — the felicity check the decode-only gate misses.
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], swapped_layer.clone());
    let verdict = check_infer(&mut ctx, &exp);
    assert!(
        verdict.is_err(),
        "swapped axiom application depends_on(CellLine, Gene) MUST be rejected by check_infer \
         (this is exactly what a commit-time proposition type-check would add over decode-only): \
         {verdict:?}"
    );
}

#[test]
fn commit_gate_rejects_ill_typed_proposition() {
    // End-to-end witness of the generalized commit rule (Rule 21): an ill-typed
    // proposition stored in an `eigentt:TypeExpr` field is rejected by the
    // Validator itself — not just by a hand-invoked check_infer. This is the
    // decode-only gap, now closed for every type_expr slot.
    let lexicon = build_lexicon();
    let resources = esl::compile_against_layer(SWAPPED_SENTENCE, &lexicon)
        .expect("swapped sentence compiles (storage encodes, does not type-check)");
    let mut b = LayerBuilder::new("swapped", Some(lexicon.clone()));
    for r in &resources {
        b.add_resource(r.clone()).expect("add swapped sentence");
    }
    let swapped_layer = Arc::new(b.build(LayerStorage::in_memory()));

    let errors = Validator::new(swapped_layer).validate();
    assert!(
        errors.iter().any(|e| e
            .to_string()
            .contains("does not type-check against the chain")),
        "the commit gate must reject the ill-typed stored proposition (Rule 21), \
         but validate() reported: {errors:?}"
    );
}
