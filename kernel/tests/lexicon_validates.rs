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

use eigenius_kernel::dcg::{
    apply, cat_subsumes, denote_cat, entry_to_item, gate_entry, is_ctor, resolve_sem,
    resolve_sem_value, subst_cat, type_eq, unify_cat, Identity, Item, Lemmatizer, LexicalIndex,
    Parser, Pos,
};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{
    normalize_value, resolve_active_value_indexes, Layer, LayerBuilder, LayerStorage,
};
use eigenius_kernel::nbe::check::{check, check_infer, CheckCtx};
use eigenius_kernel::nbe::env::Rho;
use eigenius_kernel::nbe::eval::eval;
use eigenius_kernel::nbe::readback::readback_val;
use eigenius_kernel::nbe::term::{Exp, MatchArm, Patt};
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

/// Compile a `.esl` file against `parent`, panicking with the errors if it is
/// not Expressible, and return the resulting layer.
fn esl_layer(name: &str, src: &str, parent: Arc<Layer>) -> Arc<Layer> {
    let resources = esl::compile_against_layer(src, &parent).unwrap_or_else(|errs| {
        panic!(
            "{name} failed to compile (not Expressible):\n{}",
            errs.into_iter()
                .map(|e| format!("  - {e:?}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
    let mut b = LayerBuilder::new(name, Some(parent));
    for r in &resources {
        b.add_resource(r.clone())
            .unwrap_or_else(|e| panic!("{name}: add_resource failed: {e:?}"));
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

/// The `logic` layer (ontologies/logic) over core→reflection — propositional
/// primitives (`logic:False`) the determiner/connective semantics build on
/// (D63 §8.3 Phase 0).
fn build_logic() -> Arc<Layer> {
    esl_layer(
        "logic",
        include_str!("../../ontologies/logic/logic.esl"),
        base_chain(),
    )
}

/// The lexicon SCHEMA layer (ontologies/lexicon) over core→reflection→logic.
fn build_schema() -> Arc<Layer> {
    esl_layer(
        "lexicon-schema",
        include_str!("../../ontologies/lexicon/lexicon-ontology.esl"),
        build_logic(),
    )
}

/// The `ontology` layer (ontologies/ontology) over the schema — `ontology:is_a` /
/// `ontology:subclass_of`, the opaque predicate-nominal relations (D63 §8.5 3c).
fn build_ontology() -> Arc<Layer> {
    esl_layer(
        "ontology",
        include_str!("../../ontologies/ontology/ontology.esl"),
        build_schema(),
    )
}

/// The committed closed-class determiner layer (ontologies/lexicon/closed-class)
/// over the ontology layer — the canonical determiners (`every`/`each`/`all`/`a`/
/// `some`/`no`, subject + object) + copula + wh-words the tests parse with (D63 §8.3).
fn build_closed_class() -> Arc<Layer> {
    esl_layer(
        "closed-class",
        include_str!("../../ontologies/lexicon/closed-class.esl"),
        build_ontology(),
    )
}

/// The `measurements` layer (ontologies/statistics) — the opaque float orderings
/// `measurements:gt`/`lt` (D52) the gradable-adjective comparatives reuse (D63 §8.12
/// 6-cmp). Layered over closed-class (it only needs core/reflection/institution below).
fn build_measurements() -> Arc<Layer> {
    esl_layer(
        "measurements",
        include_str!("../../ontologies/statistics/statistics.esl"),
        build_closed_class(),
    )
}

/// The worked demo DOMAIN (experiments/lexicon) over the closed-class + measurements
/// layers. A compile error here is the *Expressible* gate failing (the kernel cannot
/// carry the content).
fn build_lexicon() -> Arc<Layer> {
    esl_layer(
        "lexicon",
        include_str!("../../experiments/lexicon/lexicon.esl"),
        build_measurements(),
    )
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
    check(&mut ctx, &term, &typ_val).map_err(|e| e.to_string())
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

// ── The ⟦·⟧ recursor: the categorial → EigenTT-type homomorphism (D62 §8.6) ──
//
//   ⟦cat_s⟧ = Prop ;  ⟦cat_n⟧ = Set ;  ⟦cat_np(T)⟧ = T   (type-indexed entity)
//   ⟦A/B⟧ = ⟦A\B⟧ = ⟦B⟧ → ⟦A⟧   (direction is forgotten — it drives the parser,
//                                   not the type)
//
// This makes the felicity invariant `typeof(sem) = ⟦cat⟧` mechanical: an entry
// whose category and declared type disagree is now caught (the homogeneity /
// argument-order bug the bare-atom spike used to hide). The recursor
// (`denote_cat`) and `type_eq` are the kernel's `eigenius_kernel::dcg`
// engine, imported above — the tests below witness them, not redefine them.
fn decoded_field(layer: &Arc<Layer>, entry: &str, field: &str) -> Exp {
    let r = layer
        .resolve(&Iri::parse(entry).expect("entry iri"))
        .expect("entry resolves");
    let v = r
        .get(&Iri::parse(field).expect("field iri"))
        .unwrap_or_else(|| panic!("{entry} has no {field}"))
        .clone();
    decode_type(&v, layer).unwrap_or_else(|e| panic!("{entry}.{field} decode: {e}"))
}

#[test]
fn cat_denotation_matches_sem_type() {
    // The mechanized felicity invariant: for every entry, ⟦cat⟧ (derived from
    // the category by the recursor) is definitionally equal to the declared
    // sem_type. `cat` is now the checked source of truth — an entry whose
    // category and type disagree fails here.
    let lexicon = build_lexicon();
    for entry in [
        "urn:eigenius:lexicon:e_cell_line",
        "urn:eigenius:lexicon:e_brca1",
        "urn:eigenius:lexicon:e_hela",
        "urn:eigenius:lexicon:e_depends_on",
        "urn:eigenius:lexicon:e_primary",
    ] {
        let cat = decoded_field(&lexicon, entry, "urn:eigenius:lexicon:cat");
        let sem_type = decoded_field(&lexicon, entry, "urn:eigenius:lexicon:sem_type");
        let denoted = denote_cat(&cat).unwrap_or_else(|e| panic!("{entry}: {e}"));
        assert!(
            type_eq(&denoted, &sem_type),
            "{entry}: ⟦cat⟧ must equal sem_type.\n  ⟦cat⟧    = {denoted:?}\n  sem_type = {sem_type:?}"
        );
    }
}

#[test]
fn denotation_is_order_and_type_sensitive() {
    // ⟦(S\NP)/NP⟧ for "depends on" = Gene → CellLine → Prop. The recursor must
    // distinguish it from the argument-swapped and the homogeneous forms — the
    // two facets of the bare-atom bug it now forbids.
    let lexicon = build_lexicon();
    let denoted = denote_cat(&decoded_field(
        &lexicon,
        "urn:eigenius:lexicon:e_depends_on",
        "urn:eigenius:lexicon:cat",
    ))
    .expect("denote verb cat");

    let gene = || Exp::EigonClass(Iri::parse("urn:eigenius:lexicon:Gene").unwrap());
    let cell = || Exp::EigonClass(Iri::parse("urn:eigenius:lexicon:CellLine").unwrap());
    let ar = |a: Exp, b: Exp| Exp::Arrow(Box::new(a), Box::new(b));

    assert!(
        type_eq(&denoted, &ar(gene(), ar(cell(), Exp::Sort(0)))),
        "⟦cat⟧ should be Gene → CellLine → Prop, got {denoted:?}"
    );
    assert!(
        !type_eq(&denoted, &ar(cell(), ar(gene(), Exp::Sort(0)))),
        "⟦·⟧ must be argument-order sensitive"
    );
    assert!(
        !type_eq(&denoted, &ar(gene(), ar(gene(), Exp::Sort(0)))),
        "⟦·⟧ must distinguish entity types (the homogeneity bug is now rejected)"
    );
}

// ════════════════════════════════════════════════════════════════════
// The composition parser (D62 §2 stage 2): a CKY chart over categorial
// categories. Each step combines two items by forward/backward application —
// on the *category* (fwd/bwd) and, in lockstep, on the *sem* (App). The
// categorial type drives composition; the kernel confirms the assembled term
// is well-typed. The first prose-tokens → EigenTT-term → kernel-check loop.
// ════════════════════════════════════════════════════════════════════

// `Item`, `is_ctor`, `entry_to_item`, `apply` are the kernel's `eigenius_kernel::dcg` engine
// (imported above). The tests below drive it over the worked lexicon; they witness the engine, they do
// not redefine it.
//
// `cky_parse` is the exception, and it lives HERE rather than in the kernel: it is a bare CKY over
// `apply` alone, with no seeding, no token-keyed rules (coordination / relatives / appositives), no
// composed-cell shifts, and no beam — a strict subset of the real drivers
// (`dcg::lookup::chart_{packed,unpacked}`), which no production path uses. It is exactly the harness
// these lexicon tests want: it composes HAND-BUILT items so the assertions are about the LEXICON's
// categories and sems, not about the parse pipeline around them. Keeping it in the kernel would leave a
// second, production-shaped driver that nothing drives.
fn cky_parse(tokens: &[Item], layer: &Arc<Layer>) -> Vec<Item> {
    let n = tokens.len();
    if n == 0 {
        return Vec::new();
    }
    let mut chart: Vec<Vec<Vec<Item>>> = vec![vec![Vec::new(); n]; n];
    for (i, t) in tokens.iter().enumerate() {
        chart[i][i].push(t.clone());
    }
    for len in 2..=n {
        for i in 0..=(n - len) {
            let j = i + len - 1;
            let mut produced = Vec::new();
            for k in i..j {
                for l in &chart[i][k].clone() {
                    for r in &chart[k + 1][j].clone() {
                        if let Some(item) =
                            apply(l, r, layer, eigenius_kernel::dcg::RightContext::Other)
                        {
                            produced.push(item);
                        }
                    }
                }
            }
            chart[i][j] = produced;
        }
    }
    chart[0][n - 1].clone()
}
fn tokens_for(layer: &Arc<Layer>, forms: &[&str]) -> Vec<Item> {
    forms
        .iter()
        .map(|f| {
            let iri = Iri::parse(&format!("urn:eigenius:lexicon:{f}")).expect("entry iri");
            let r = layer
                .resolve(&iri)
                .unwrap_or_else(|| panic!("entry not found: {f}"));
            entry_to_item(layer, &r).unwrap_or_else(|e| panic!("{f}: {e}"))
        })
        .collect()
}

#[test]
fn parser_composes_sentence_to_checked_prop() {
    let lexicon = build_lexicon();
    // "HeLa depends on BRCA1" — subject HeLa (CellLine), verb, object BRCA1 (Gene).
    let tokens = tokens_for(&lexicon, &["e_hela", "e_depends_on", "e_brca1"]);
    let parses = cky_parse(&tokens, &lexicon);
    let sentences: Vec<&Item> = parses
        .iter()
        .filter(|it| is_ctor(it.cat(), "cat_s").is_some())
        .collect();
    assert_eq!(
        sentences.len(),
        1,
        "expected exactly one S parse; got cats {:?}",
        parses.iter().map(|i| i.cat()).collect::<Vec<_>>()
    );

    // The assembled sem must type-check — and to Prop. That is the felicity of
    // the *whole composed sentence*, confirmed by the kernel.
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], lexicon.clone());
    let ty = check_infer(&mut ctx, sentences[0].sem())
        .expect("composed sentence must type-check (felicity of the parse)");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "the composed sentence must inhabit Prop"
    );
}

#[test]
fn parser_rejects_type_mismatched_sentence() {
    let lexicon = build_lexicon();
    // "BRCA1 depends on HeLa" — the verb's object must be a Gene and its subject
    // a CellLine, but here they're swapped. The categories do not combine: no S
    // parse. The parse-time felicity filter, on the category alone.
    let tokens = tokens_for(&lexicon, &["e_brca1", "e_depends_on", "e_hela"]);
    let parses = cky_parse(&tokens, &lexicon);
    let s = parses
        .iter()
        .filter(|it| is_ctor(it.cat(), "cat_s").is_some())
        .count();
    assert_eq!(
        s, 0,
        "type-mismatched sentence must not parse to S; got {s}"
    );
}

// ════════════════════════════════════════════════════════════════════
// `gate_entry` — the callable felicity gate (D62 §8.6): the *trusted half* of
// the prose→trees engine. An untrusted LLM proposer drafts lexical entries as
// Eigon-JSON; the kernel admits or rejects each via this gate at ingestion.
// It enforces BOTH halves of felicity on one entry: ⟦cat⟧ ≡ sem_type AND the
// entry's `sem` actually inhabits ⟦cat⟧. The recursor tests above check the
// first half over the worked entries; the gate is the single callable that a
// generation tool runs every draft through.
// ════════════════════════════════════════════════════════════════════

#[test]
fn gate_admits_well_formed_entries() {
    let lexicon = build_lexicon();
    for entry in [
        "urn:eigenius:lexicon:e_cell_line",
        "urn:eigenius:lexicon:e_brca1",
        "urn:eigenius:lexicon:e_hela",
        "urn:eigenius:lexicon:e_depends_on",
        "urn:eigenius:lexicon:e_primary",
    ] {
        let r = lexicon
            .resolve(&Iri::parse(entry).expect("entry iri"))
            .unwrap_or_else(|| panic!("entry resolves: {entry}"));
        gate_entry(&lexicon, &r)
            .unwrap_or_else(|e| panic!("gate must admit well-formed entry {entry}: {e}"));
    }
}

// Drafts an LLM proposer might emit: each is per-field well-formed (so the
// commit gate / Rule 21, which checks each eigentt:TypeExpr slot in isolation,
// admits them) but FELICITY-inconsistent across fields — caught only by
// `gate_entry`. The gate is therefore doing real work the storage gate cannot.
const DRAFTS: &str = r#"
namespace lexicon   = "urn:eigenius:lexicon";
namespace epistemic = "urn:eigenius:reflection:epistemic";

// ⟦cat_np(Gene)⟧ = Gene, but sem_type claims CellLine — category and declared
// type disagree (the cross-field check the recursor proves for real entries).
resource lexicon:e_bad_type : lexicon:LexicalEntry {
    lexicon:form     = "bad-type";
    lexicon:cat      = type_expr( lexicon:cat_np(lexicon:Gene, lexicon:num_any) );
    lexicon:sem      = lexicon:brca1;
    lexicon:sem_type = type_expr( lexicon:CellLine );
    lexicon:grade    = epistemic:declared;
}

// cat and sem_type agree (Gene), but the `sem` points at a CellLine instance —
// the semantics does not inhabit ⟦cat⟧. The second half of the felicity check.
resource lexicon:e_bad_sem : lexicon:LexicalEntry {
    lexicon:form     = "bad-sem";
    lexicon:cat      = type_expr( lexicon:cat_np(lexicon:Gene, lexicon:num_any) );
    lexicon:sem      = lexicon:hela;
    lexicon:sem_type = type_expr( lexicon:Gene );
    lexicon:grade    = epistemic:declared;
}
"#;

fn drafts_layer() -> Arc<Layer> {
    let lexicon = build_lexicon();
    let resources = esl::compile_against_layer(DRAFTS, &lexicon)
        .expect("drafts compile (per-field well-formed; cross-field felicity is the gate's job)");
    let mut b = LayerBuilder::new("drafts", Some(lexicon));
    for r in &resources {
        b.add_resource(r.clone()).expect("add draft entry");
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

#[test]
fn gate_rejects_felicity_inconsistent_drafts() {
    let layer = drafts_layer();
    for (entry, why) in [
        ("urn:eigenius:lexicon:e_bad_type", "⟦cat⟧ ≠ sem_type"),
        (
            "urn:eigenius:lexicon:e_bad_sem",
            "sem does not inhabit ⟦cat⟧",
        ),
    ] {
        let r = layer
            .resolve(&Iri::parse(entry).expect("entry iri"))
            .unwrap_or_else(|| panic!("entry resolves: {entry}"));
        let verdict = gate_entry(&layer, &r);
        assert!(
            verdict.is_err(),
            "gate MUST reject {entry} ({why}), but admitted it: {verdict:?}"
        );
    }
}

// ════════════════════════════════════════════════════════════════════
// CN-as-types subsumption (Luo 2012; D62 §8.6): the checker honors the
// ontology's `core:subclass_of` lattice as the EigonClass subtype rule, so a
// GENERAL predicate typed at a supertype accepts subclass-typed arguments —
// "depends on relates entities, Gene/CellLine flow in" — with no new
// type-system machinery. Witnessed at the kernel boundary and end-to-end
// through the parser.
// ════════════════════════════════════════════════════════════════════

#[test]
fn kernel_honors_subclass_subsumption() {
    let lexicon = build_lexicon();
    let sem = |local: &str| {
        resolve_sem(
            &lexicon,
            &Iri::parse(&format!("urn:eigenius:lexicon:{local}")).unwrap(),
        )
    };
    let app = |f: Exp, x: Exp| Exp::App(Box::new(f), Box::new(x));

    // `affects : Entity -> Entity -> Prop` applied to brca1 : Gene and hela :
    // CellLine type-checks — Gene, CellLine <: Entity (the subsumption rule).
    let term = app(app(sem("affects"), sem("brca1")), sem("hela"));
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], lexicon.clone());
    let ty = check_infer(&mut ctx, &term)
        .expect("affects(Gene, CellLine) must type-check via subclass subsumption");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "the general-predicate application inhabits Prop"
    );

    // Subsumption is directional and sound: `depends_on : Gene -> CellLine ->
    // Prop` applied to hela : CellLine as its FIRST argument is still REJECTED
    // (CellLine is not a subclass of Gene) — siblings under Entity don't subsume.
    let bad = app(sem("depends_on"), sem("hela"));
    let mut ctx2 = CheckCtx::with_layer(Rho::Nil, vec![], lexicon.clone());
    assert!(
        check_infer(&mut ctx2, &bad).is_err(),
        "depends_on(CellLine, ..) MUST be rejected — CellLine is not a subclass of Gene"
    );
}

#[test]
fn parser_composes_general_verb_via_subsumption() {
    let lexicon = build_lexicon();
    // "HeLa affects BRCA1" — the general verb's `NP[Entity]` slots accept the
    // CellLine subject and the Gene object by subsumption. It composes to S and
    // the assembled term checks to Prop (kernel subsumption closes the parse).
    let tokens = tokens_for(&lexicon, &["e_hela", "e_affects", "e_brca1"]);
    let parses = cky_parse(&tokens, &lexicon);
    let sentences: Vec<&Item> = parses
        .iter()
        .filter(|it| is_ctor(it.cat(), "cat_s").is_some())
        .collect();
    assert_eq!(
        sentences.len(),
        1,
        "expected exactly one S parse for the general verb; got cats {:?}",
        parses.iter().map(|i| i.cat()).collect::<Vec<_>>()
    );

    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], lexicon.clone());
    let ty = check_infer(&mut ctx, sentences[0].sem())
        .expect("composed general-verb sentence must type-check via subsumption");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "the composed general-verb sentence must inhabit Prop"
    );
}

// ════════════════════════════════════════════════════════════════════
// The lookup bridge (D62 §8.8.1): string → the forest of typed parses.
// `Parser` builds a `form → entries` index over the committed lexicon;
// `parse` tokenizes, seeds multi-token spans via the `Lemmatizer` (`Identity`
// here — WordNet's Morphy is witnessed in the `eigenius-wordnet` crate), runs
// CKY, and keeps every full-span S whose assembled sem the kernel types to Prop.
// This joins lookup + multi-span MWE seeding + composition + the felicity oracle
// into the kernel-attached `string → tree(s)` library. The forest is returned
// whole (no selection, no commit — that is the encoding institution's job).
// ════════════════════════════════════════════════════════════════════

#[test]
fn index_covers_the_committed_entries() {
    // An INDEX test, not a parser test: it asserts on the lexicon's coverage, so it holds the lexicon.
    let index = LexicalIndex::build(build_lexicon());
    assert!(!index.is_empty());
    // the six spike entries (incl. the multiword forms "cell line", "depends on").
    assert!(
        index.len() >= 6,
        "index should cover the committed lexical entries; got {}",
        index.len()
    );
}

// D65 slice 1: the schema declares a `core:ValueIndex` on `lexicon:form` with a
// `lowercase` normalizer — the runtime substrate the lazy `Parser`
// (slice 2) probes (on a shared-storage chain) instead of eagerly scanning, and
// whose absence-from-the-local-index is the fallback signal in this fresh-
// storage-per-layer test harness. The generic active-discovery + build-time
// population over shared storage is proven by the slice-0 integration tests
// (kernel/tests/value_index_build_population.rs); here we validate the *schema
// declaration itself* is well-formed and discoverable as an active ValueIndex
// when its layer's index is consulted directly.
#[test]
fn form_value_index_is_declared_on_lexicon_form() {
    let schema = build_schema();

    // The declared resource resolves as a well-formed `core:ValueIndex`.
    let idx = schema
        .resolve(&Iri::parse("urn:eigenius:lexicon:form_index").unwrap())
        .expect("lexicon:form_index is committed in the schema layer");
    let is_a = idx
        .get(&Iri::parse("urn:eigenius:core:is_a").unwrap())
        .expect("form_index has is_a");
    let classes: Vec<&str> = match is_a {
        Value::Array(items) => items.iter().filter_map(|v| v.as_iri_str()).collect(),
        v => v.as_iri_str().into_iter().collect(),
    };
    assert!(
        classes.contains(&"urn:eigenius:core:ValueIndex"),
        "form_index is a core:ValueIndex; got {classes:?}"
    );

    // Discovery (over the schema layer's own index) yields exactly the lexicon
    // form index, targeting `lexicon:form` with the `lowercase` normalizer.
    let actives = resolve_active_value_indexes(&schema);
    let form_idx = actives
        .iter()
        .find(|a| a.iri.as_str() == "urn:eigenius:lexicon:form_index")
        .expect("the form ValueIndex is discoverable as active");
    assert_eq!(
        form_idx.target_property.as_str(),
        "urn:eigenius:lexicon:form"
    );
    assert_eq!(
        form_idx.normalizer.as_str(),
        "urn:eigenius:core:normalizers:lowercase"
    );

    // The normalizer the declaration names folds case as specified: the parser
    // and the index agree on the lookup key for "Cell Line" ⇒ "cell line".
    assert_eq!(
        normalize_value(&form_idx.normalizer, "Cell Line"),
        "cell line"
    );
}

// D65 §3 slice 3: a `lexicon:Lexicon` is a first-class Resource with a stable IRI
// + provenance metadata; each entry binds back via `lexicon:in_lexicon`. Because a
// lexicon is just a Resource, "available lexica" needs no new machinery — it is an
// ordinary EigenQL class-membership query over `lexicon:Lexicon` instances.
#[test]
fn lexicon_instances_validate_and_available_lexica_is_a_plain_query() {
    let src = r#"
        namespace lexicon = "urn:eigenius:lexicon";
        resource lexicon:wn : lexicon:Lexicon {
            lexicon:source   = "WordNet 3.0, Princeton University";
            lexicon:version  = "3.0";
            lexicon:language = "en";
            lexicon:domain   = "general";
        }
        resource lexicon:bio : lexicon:Lexicon {
            lexicon:source = "UMLS";
            lexicon:domain = "biomedical";
        }
    "#;
    let layer = esl_layer("lexica", src, build_schema());

    // Both instances satisfy the Lexicon class (requires lexicon:source).
    let errors = Validator::new(Arc::clone(&layer)).validate();
    assert!(
        errors.is_empty(),
        "lexicon instances must validate: {errors:?}"
    );

    // The descriptor + metadata resolve.
    let wn = layer
        .resolve(&Iri::parse("urn:eigenius:lexicon:wn").unwrap())
        .expect("lexicon:wn resolves");
    assert!(
        matches!(wn.get(&Iri::parse("urn:eigenius:lexicon:domain").unwrap()),
                 Some(Value::String(s)) if s == "general")
    );

    // "Available lexica" = every `lexicon:Lexicon` instance, via a plain query.
    let rows = eigenius_kernel::query::execute_with(
        r#"MATCH "urn:eigenius:lexicon:Lexicon"(?l) {} RETURN [] { l: ?l } LIMIT 10"#,
        &layer,
        eigenius_kernel::query::evaluate::FiberRuntime::default(),
    )
    .expect("available-lexica query runs");
    let row_count = rows
        .iter()
        .find_map(
            |r| match r.get(&Iri::parse("urn:eigenius:query:row_count").unwrap()) {
                Some(Value::Integer(n)) => Some(*n),
                _ => None,
            },
        )
        .expect("result carries a row_count");
    assert_eq!(row_count, 2, "both lexica surface as queryable instances");
}

// D65 §4 slice 4: a parse SCOPE (ordered Lexicon IRIs) filters the lexicon and
// ranks by lexicon precedence. Two competing "widget" entries — `CellLine` in
// `lex_a`, `Gene` in `lex_b` — let us observe filtering + precedence directly.
const SCOPED_LEXICA: &str = r#"
    namespace lexicon   = "urn:eigenius:lexicon";
    namespace epistemic = "urn:eigenius:reflection:epistemic";

    resource lexicon:lex_a : lexicon:Lexicon { lexicon:source = "A"; }
    resource lexicon:lex_b : lexicon:Lexicon { lexicon:source = "B"; }

    resource lexicon:e_widget_a : lexicon:LexicalEntry {
        lexicon:form       = "widget";
        lexicon:cat        = type_expr( lexicon:cat_n(lexicon:CellLine, lexicon:num_any) );
        lexicon:sem        = lexicon:CellLine;
        lexicon:sem_type   = type_expr( Set );
        lexicon:grade      = epistemic:declared;
        lexicon:in_lexicon = lexicon:lex_a;
    }
    resource lexicon:e_widget_b : lexicon:LexicalEntry {
        lexicon:form       = "widget";
        lexicon:cat        = type_expr( lexicon:cat_n(lexicon:Gene, lexicon:num_any) );
        lexicon:sem        = lexicon:Gene;
        lexicon:sem_type   = type_expr( Set );
        lexicon:grade      = epistemic:declared;
        lexicon:in_lexicon = lexicon:lex_b;
    }
"#;

/// Readback-normalized sem string — distinguishes the CellLine vs Gene reading.
fn sem_string(it: &Item) -> String {
    format!(
        "{:?}",
        readback_val(0, &eval(it.sem(), &Rho::Nil).expect("eval sem"))
    )
}

#[test]
fn parse_scope_filters_lexica_and_ranks_by_precedence() {
    let layer = esl_layer("scoped-lex", SCOPED_LEXICA, build_lexicon());
    let index = Parser::build(Arc::clone(&layer));
    let lex_a = Iri::parse("urn:eigenius:lexicon:lex_a").unwrap();
    let lex_b = Iri::parse("urn:eigenius:lexicon:lex_b").unwrap();
    let sentence = "every widget affects HeLa";

    // No scope: both widget readings parse (CellLine and Gene).
    let unscoped = index.parse(sentence, &Identity);
    assert_eq!(unscoped.len(), 2, "both readings present unscoped");

    // Scope to lex_a only → the lex_b (Gene) reading is filtered out.
    let only_a = index.parse_scoped(sentence, &Identity, Some(std::slice::from_ref(&lex_a)));
    assert_eq!(only_a.len(), 1, "lex_b reading filtered out");
    let only_b = index.parse_scoped(sentence, &Identity, Some(std::slice::from_ref(&lex_b)));
    assert_eq!(only_b.len(), 1, "lex_a reading filtered out");
    assert_ne!(
        sem_string(&only_a[0]),
        sem_string(&only_b[0]),
        "the two lexica give genuinely different readings"
    );

    // Both in scope, lex_a first → the lex_a reading ranks first (lexicon_order 0).
    let ab = index.parse_scoped(sentence, &Identity, Some(&[lex_a.clone(), lex_b.clone()]));
    assert_eq!(ab.len(), 2);
    assert!(
        ab[0].cost().lexicon_order <= ab[1].cost().lexicon_order,
        "forest is sorted by lexicon precedence"
    );
    assert_eq!(
        sem_string(&ab[0]),
        sem_string(&only_a[0]),
        "lex_a (first-listed) reading ranks first"
    );

    // Reverse precedence: lex_b first → the lex_b reading ranks first.
    let ba = index.parse_scoped(sentence, &Identity, Some(&[lex_b, lex_a]));
    assert_eq!(
        sem_string(&ba[0]),
        sem_string(&only_b[0]),
        "reversing the scope order flips which reading ranks first"
    );
}

#[test]
fn lexicon_profile_resolves_to_ordered_scope() {
    // D65 §4.1: a LexiconProfile names an ordered scope; resolve_lexicon_profile
    // returns its `lexica` array in declaration order (= resolution precedence).
    let snippet = r#"
        namespace lexicon = "urn:eigenius:lexicon";
        resource lexicon:lx1 : lexicon:Lexicon { lexicon:source = "1"; }
        resource lexicon:lx2 : lexicon:Lexicon { lexicon:source = "2"; }
        resource lexicon:prof : lexicon:LexiconProfile {
            lexicon:lexica = [ lexicon:lx2, lexicon:lx1 ];
        }
    "#;
    let layer = esl_layer("profile", snippet, build_schema());
    let scope = eigenius_kernel::dcg::resolve_lexicon_profile(
        &layer,
        &Iri::parse("urn:eigenius:lexicon:prof").unwrap(),
    )
    .expect("profile resolves to a scope");
    assert_eq!(
        scope,
        vec![
            Iri::parse("urn:eigenius:lexicon:lx2").unwrap(),
            Iri::parse("urn:eigenius:lexicon:lx1").unwrap(),
        ],
        "scope preserves the lexica array order (precedence)"
    );
}

#[test]
fn bridge_parses_mwe_sentence_to_prop() {
    let index = Parser::build(build_lexicon());
    // "HeLa depends on BRCA1": the verb is the multiword form "depends on" — one
    // entry seeded across two tokens (the multi-span MWE seed) — and the proper
    // nouns are single-token NP lookups. `parse` only returns S items whose sem
    // type-checks to Prop, so a non-empty forest is itself the felicity witness.
    let forest = index.parse("HeLa depends on BRCA1", &Identity);
    assert_eq!(
        forest.len(),
        1,
        "expected exactly one felicitous S parse for the MWE-verb sentence; got {}",
        forest.len()
    );
    assert!(
        is_ctor(forest[0].cat(), "cat_s").is_some(),
        "the parse is an S"
    );
}

#[test]
fn bridge_composes_general_verb_via_subsumption() {
    let index = Parser::build(build_lexicon());
    // "HeLa affects BRCA1" — the general verb's NP[Entity] slots accept the
    // CellLine subject and Gene object by subclass subsumption, through the bridge.
    let forest = index.parse("HeLa affects BRCA1", &Identity);
    assert_eq!(
        forest.len(),
        1,
        "the general verb must compose via subsumption; got {}",
        forest.len()
    );
}

#[test]
fn bridge_is_case_insensitive() {
    let index = Parser::build(build_lexicon());
    // Upper-cased input still resolves: the index is keyed by lowercased form and
    // the tokenizer lowercases.
    let forest = index.parse("HELA DEPENDS ON BRCA1", &Identity);
    assert_eq!(forest.len(), 1, "case-insensitive lookup must still parse");
}

#[test]
fn bridge_returns_empty_forest_for_unknown_words() {
    let index = Parser::build(build_lexicon());
    assert!(
        index.parse("xyzzy plugh frobnicate", &Identity).is_empty(),
        "no matching entries → no admissible parse (empty forest is a first-class outcome, not an error)"
    );
}

#[test]
fn bridge_yields_no_parse_for_type_mismatch() {
    let index = Parser::build(build_lexicon());
    // "BRCA1 depends on HeLa" — subject/object types swapped; the categories do
    // not combine, so the forest is empty (the felicity filter at the category level).
    assert!(
        index.parse("BRCA1 depends on HeLa", &Identity).is_empty(),
        "a type-mismatched sentence must produce no S parse"
    );
}

// ════════════════════════════════════════════════════════════════════
// Features on `lexicon:Cat` (D63 §5.1, Slice 1): atoms carry morphosyntactic
// features that `⟦·⟧` erases (Num/Fin) and `cat_subsumes` unifies by **meet**
// (`Any = ⊤`). The denotation tests above already witness erasure (⟦cat⟧ is
// unchanged by features); this witnesses the meet — the gate the spike's
// all-`num_any` entries can't exercise.
// ════════════════════════════════════════════════════════════════════

const FEAT: &str = r#"
namespace lexicon   = "urn:eigenius:lexicon";
namespace epistemic = "urn:eigenius:reflection:epistemic";
resource lexicon:f_n_sg : lexicon:LexicalEntry {
    lexicon:form = "f"; lexicon:cat = type_expr( lexicon:cat_n(lexicon:CellLine, lexicon:sg) );
    lexicon:sem = lexicon:CellLine; lexicon:sem_type = type_expr( Set ); lexicon:grade = epistemic:declared;
}
resource lexicon:f_n_pl : lexicon:LexicalEntry {
    lexicon:form = "f"; lexicon:cat = type_expr( lexicon:cat_n(lexicon:CellLine, lexicon:pl) );
    lexicon:sem = lexicon:CellLine; lexicon:sem_type = type_expr( Set ); lexicon:grade = epistemic:declared;
}
resource lexicon:f_n_any : lexicon:LexicalEntry {
    lexicon:form = "f"; lexicon:cat = type_expr( lexicon:cat_n(lexicon:CellLine, lexicon:num_any) );
    lexicon:sem = lexicon:CellLine; lexicon:sem_type = type_expr( Set ); lexicon:grade = epistemic:declared;
}
resource lexicon:f_np_ent_sg : lexicon:LexicalEntry {
    lexicon:form = "f"; lexicon:cat = type_expr( lexicon:cat_np(lexicon:Entity, lexicon:sg) );
    lexicon:sem = lexicon:brca1; lexicon:sem_type = type_expr( lexicon:Entity ); lexicon:grade = epistemic:declared;
}
resource lexicon:f_np_gene_sg : lexicon:LexicalEntry {
    lexicon:form = "f"; lexicon:cat = type_expr( lexicon:cat_np(lexicon:Gene, lexicon:sg) );
    lexicon:sem = lexicon:brca1; lexicon:sem_type = type_expr( lexicon:Gene ); lexicon:grade = epistemic:declared;
}
resource lexicon:f_np_gene_pl : lexicon:LexicalEntry {
    lexicon:form = "f"; lexicon:cat = type_expr( lexicon:cat_np(lexicon:Gene, lexicon:pl) );
    lexicon:sem = lexicon:brca1; lexicon:sem_type = type_expr( lexicon:Gene ); lexicon:grade = epistemic:declared;
}
"#;

#[test]
fn cat_subsumes_meets_features() {
    let lexicon = build_lexicon();
    let resources =
        esl::compile_against_layer(FEAT, &lexicon).expect("feature-bearing entries compile");
    let mut b = LayerBuilder::new("feat", Some(lexicon));
    for r in &resources {
        b.add_resource(r.clone()).expect("add feature entry");
    }
    let layer = Arc::new(b.build(LayerStorage::in_memory()));
    let cat = |local: &str| {
        decoded_field(
            &layer,
            &format!("urn:eigenius:lexicon:{local}"),
            "urn:eigenius:lexicon:cat",
        )
    };
    let (n_sg, n_pl, n_any) = (cat("f_n_sg"), cat("f_n_pl"), cat("f_n_any"));
    let (np_ent_sg, np_gene_sg, np_gene_pl) =
        (cat("f_np_ent_sg"), cat("f_np_gene_sg"), cat("f_np_gene_pl"));

    // cat_n number meet: `sg` fills `sg` or `Any`, never `pl`; `Any` fills anything.
    assert!(cat_subsumes(&n_sg, &n_sg, &layer));
    assert!(
        !cat_subsumes(&n_sg, &n_pl, &layer),
        "an `sg` slot must reject a `pl` argument"
    );
    assert!(
        cat_subsumes(&n_sg, &n_any, &layer),
        "an underspecified `Any` argument fills an `sg` slot (meet = sg)"
    );
    assert!(
        cat_subsumes(&n_any, &n_pl, &layer),
        "an `Any` slot accepts a `pl` argument"
    );

    // cat_np: subclass-subsume the type AND meet the number, jointly.
    assert!(
        cat_subsumes(&np_ent_sg, &np_gene_sg, &layer),
        "Gene ⊑ Entity and sg = sg ⇒ fills"
    );
    assert!(
        !cat_subsumes(&np_ent_sg, &np_gene_pl, &layer),
        "type ok (Gene ⊑ Entity) but number sg ≠ pl ⇒ reject"
    );
}

// ════════════════════════════════════════════════════════════════════
// #91 — the check-mode resource-inhabitation rule + the check-mode gate. A
// multi-class individual (`r : Gene, CellLine`) gate-admits a name entry at
// EACH of its classes — including the **non-first** one, which the old
// `check_infer`-`.first()` gate could not (it only ever saw `Gene`). Transitive
// (Entity) works via `is_subclass_of`.
// ════════════════════════════════════════════════════════════════════

const DUAL: &str = r#"
namespace core      = "urn:eigenius:core";
namespace epistemic = "urn:eigenius:reflection:epistemic";
namespace lexicon   = "urn:eigenius:lexicon";
resource lexicon:dual : lexicon:Gene, lexicon:CellLine {
    core:description = "an individual that is both a Gene and a CellLine";
}
resource lexicon:e_dual_gene : lexicon:LexicalEntry {
    lexicon:form = "dual"; lexicon:cat = type_expr( lexicon:cat_np(lexicon:Gene, lexicon:num_any) );
    lexicon:sem = lexicon:dual; lexicon:sem_type = type_expr( lexicon:Gene ); lexicon:grade = epistemic:declared;
}
resource lexicon:e_dual_cl : lexicon:LexicalEntry {
    lexicon:form = "dual"; lexicon:cat = type_expr( lexicon:cat_np(lexicon:CellLine, lexicon:num_any) );
    lexicon:sem = lexicon:dual; lexicon:sem_type = type_expr( lexicon:CellLine ); lexicon:grade = epistemic:declared;
}
resource lexicon:e_dual_ent : lexicon:LexicalEntry {
    lexicon:form = "dual"; lexicon:cat = type_expr( lexicon:cat_np(lexicon:Entity, lexicon:num_any) );
    lexicon:sem = lexicon:dual; lexicon:sem_type = type_expr( lexicon:Entity ); lexicon:grade = epistemic:declared;
}
"#;

#[test]
fn gate_admits_multi_class_resource_at_each_class() {
    let lexicon = build_lexicon();
    let resources = esl::compile_against_layer(DUAL, &lexicon).expect("dual entries compile");
    let mut b = LayerBuilder::new("dual", Some(lexicon));
    for r in &resources {
        b.add_resource(r.clone()).expect("add dual entry");
    }
    let layer = Arc::new(b.build(LayerStorage::in_memory()));
    for (entry, why) in [
        ("urn:eigenius:lexicon:e_dual_gene", "first class"),
        (
            "urn:eigenius:lexicon:e_dual_cl",
            "NON-first class (the #91 win)",
        ),
        (
            "urn:eigenius:lexicon:e_dual_ent",
            "transitive super (Gene ⊑ Entity)",
        ),
    ] {
        let r = layer
            .resolve(&Iri::parse(entry).expect("entry iri"))
            .unwrap_or_else(|| panic!("resolves: {entry}"));
        gate_entry(&layer, &r).unwrap_or_else(|e| {
            panic!("multi-class resource must gate-admit at {why} ({entry}): {e}")
        });
    }
}

// ════════════════════════════════════════════════════════════════════
// D63 §8.2 (Slice 2) — de-risking the expert-resolved determiner SEMANTICS
// before the category machinery (type-variables / unification / contravariance)
// is built. The polymorphic determiner sem `λA:Set. λV:A→Prop. ∀x:A. V(x)`:
//   (1) type-checks as a closed term against `ΠA:Set. (A→Prop)→Prop` (so it can
//       gate in isolation — the per-item felicity discipline holds); and
//   (2) applied to a CN (`Gene`) and a generic `Entity`-predicate (`q`), reduces
//       (NbE) to `∀x:Gene. q(x) : Prop` — the `Gene ⊑ Entity` coercion firing
//       under `∀` on the now-concrete `Gene`. This validates Option 2's typing
//       end-to-end; the remaining work is plumbing to PRODUCE these terms via
//       parsing.
// ════════════════════════════════════════════════════════════════════

const DET_SEMANTICS: &str = r#"
namespace lexicon = "urn:eigenius:lexicon";
axiom lexicon:q : lexicon:Entity -> Prop
resource lexicon:det_sem : lexicon:Sentence {
    lexicon:gloss = "the polymorphic determiner type ΠA:Set.(A→Prop)→Prop";
    lexicon:prop  = type_expr( forall (A : Set) => (A -> Prop) -> Prop );
}
"#;

/// The polymorphic determiner sem: `λA:Set. λV:A→Prop. ∀x:A. V(x)`.
fn det_sem_exp() -> Exp {
    let v_app = Exp::App(
        Box::new(Exp::Var("V".into())),
        Box::new(Exp::Var("x".into())),
    );
    let forall_x = Exp::Pi(
        Patt::Var("x".into()),
        Box::new(Exp::Var("A".into())),
        Box::new(v_app),
    );
    let lam_v = Exp::Lam(Patt::Var("V".into()), Box::new(forall_x));
    Exp::Lam(Patt::Var("A".into()), Box::new(lam_v))
}

fn det_layer() -> Arc<Layer> {
    let lexicon = build_lexicon();
    let resources =
        esl::compile_against_layer(DET_SEMANTICS, &lexicon).expect("determiner snippet compiles");
    let mut b = LayerBuilder::new("det", Some(lexicon));
    for r in &resources {
        b.add_resource(r.clone()).expect("add determiner resource");
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

#[test]
fn determiner_sem_inhabits_its_polymorphic_type() {
    // (1) The polymorphic determiner sem type-checks against ΠA:Set.(A→Prop)→Prop
    //     — so it can gate in isolation (the per-item felicity discipline holds).
    let layer = det_layer();
    let det_ty = decode_type(
        &proposition_of(&layer, "urn:eigenius:lexicon:det_sem"),
        &layer,
    )
    .expect("det type decodes");
    let ty_val = eval(&det_ty, &Rho::Nil).expect("eval det type");
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], layer.clone());
    check(&mut ctx, &det_sem_exp(), &ty_val)
        .expect("polymorphic determiner sem must inhabit ΠA:Set.(A→Prop)→Prop (gate-able)");
}

#[test]
fn every_gene_q_composes_and_reduces_to_prop() {
    // (2) The composed `det(Gene)(q)` NbE-reduces to `∀x:Gene. q(x) : Prop` — the
    //     Gene ⊑ Entity coercion firing under ∀ (q : Entity → Prop), on the now-
    //     concrete Gene. (Built directly here; producing it via parsing is the
    //     remaining DCG plumbing, §8.2.)
    let layer = det_layer();
    let gene = Exp::EigonClass(Iri::parse("urn:eigenius:lexicon:Gene").unwrap());
    let q = Exp::EigonAxiom(Iri::parse("urn:eigenius:lexicon:q").unwrap());
    let composed = Exp::App(
        Box::new(Exp::App(Box::new(det_sem_exp()), Box::new(gene))),
        Box::new(q),
    );
    let nf = readback_val(0, &eval(&composed, &Rho::Nil).expect("eval composed term"));
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], layer.clone());
    let ty = check_infer(&mut ctx, &nf)
        .expect("composed `every gene q` must type-check after NbE reduction");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "`every gene q` must inhabit Prop"
    );
}

// ════════════════════════════════════════════════════════════════════
// D63 §8.2 item 2 — category type-variables + first-order unification.
//
// A determiner is polymorphic: its category `(S/(S\NP_T))/N_T` carries a
// schematic type-variable `T`. When it composes forward with a noun `N_Gene`,
// `apply` UNIFIES `T := Gene` and SUBSTITUTES that binding through the result,
// producing `S/(S\NP_Gene)`. (Authoring a free `T` in ESL is the item-5
// decision; here the polymorphic category is synthesized from a concrete
// determiner-shaped one — through the real decode path — by replacing the
// `Gene` leaves with a schematic `Var("T")`, so substituting `T := Gene` must
// recover the concrete category exactly.)
// ════════════════════════════════════════════════════════════════════

// A determiner-SHAPED category authored concretely over `Gene`: the forward
// functor `(S/(S\NP_Gene))/N_Gene`. (Not felicitous as an entry — its `sem` is a
// placeholder; only the `cat` field is read.)
const DET_SHAPE: &str = r#"
namespace lexicon   = "urn:eigenius:lexicon";
namespace epistemic = "urn:eigenius:reflection:epistemic";
resource lexicon:e_det_shape : lexicon:LexicalEntry {
    lexicon:form     = "every";
    lexicon:cat      = type_expr(
        lexicon:fwd(lexicon:m_all, 
            lexicon:fwd(lexicon:m_all, 
                lexicon:cat_s(lexicon:dcl, lexicon:fin_any),
                lexicon:bwd(lexicon:m_all, 
                    lexicon:cat_s(lexicon:dcl, lexicon:fin_any),
                    lexicon:cat_np(lexicon:Gene, lexicon:num_any)
                )
            ),
            lexicon:cat_n(lexicon:Gene, lexicon:num_any)
        )
    );
    lexicon:sem      = lexicon:Gene;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "x";
    lexicon:grade    = epistemic:declared;
}
"#;

fn det_shape_layer() -> Arc<Layer> {
    let lexicon = build_lexicon();
    let resources =
        esl::compile_against_layer(DET_SHAPE, &lexicon).expect("determiner-shape snippet compiles");
    let mut b = LayerBuilder::new("det-shape", Some(lexicon));
    for r in &resources {
        b.add_resource(r.clone()).expect("add det-shape resource");
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

/// Replace every `EigonClass(class)` leaf with a schematic `Var(var)` — turning a
/// concrete category into a polymorphic scheme (the inverse of the `subst_cat`
/// the engine performs).
fn polymorphize(cat: &Exp, class: &Iri, var: &str) -> Exp {
    match cat {
        Exp::EigonClass(iri) if iri == class => Exp::Var(var.to_string()),
        Exp::InductiveCtor(decl, name, args) => Exp::InductiveCtor(
            decl.clone(),
            name.clone(),
            args.iter().map(|a| polymorphize(a, class, var)).collect(),
        ),
        other => other.clone(),
    }
}

#[test]
fn determiner_unifies_type_var_and_substitutes_through_result() {
    let layer = det_shape_layer();
    let gene = Iri::parse("urn:eigenius:lexicon:Gene").unwrap();

    // The concrete determiner category `(S/(S\NP_Gene))/N_Gene`, decoded through
    // the real path; split into its result `S/(S\NP_Gene)` and its noun slot
    // `N_Gene`.
    let concrete = decoded_field(
        &layer,
        "urn:eigenius:lexicon:e_det_shape",
        "urn:eigenius:lexicon:cat",
    );
    let c_args = is_ctor(&concrete, "fwd").expect("determiner is a forward functor");
    let concrete_result = c_args[1].clone(); // S/(S\NP_Gene); [0] is the slash modality
    let noun_cat = c_args[2].clone(); // N_Gene  (= cat_n(Gene, num_any))

    // The polymorphic category: `Gene` leaves → schematic `T`.
    let poly = polymorphize(&concrete, &gene, "T");
    let p_args = is_ctor(&poly, "fwd").expect("polymorphic determiner is a forward functor");
    let poly_noun_slot = &p_args[2]; // N_T  (= cat_n(Var T, num_any)); [0] is the slash modality

    // (1) Unification: the `N_T` slot binds `T := Gene` against the concrete noun.
    let subst = unify_cat(poly_noun_slot, &noun_cat, &layer).expect("N_T unifies with N_Gene");
    assert_eq!(
        subst.get("T"),
        Some(&Exp::EigonClass(gene.clone())),
        "unification must bind T := Gene"
    );

    // (2) Substituting that binding through the polymorphic result recovers the
    //     concrete result exactly.
    assert_eq!(
        subst_cat(&p_args[1], &subst),
        concrete_result,
        "T := Gene must flow through the result category"
    );

    // (3) End-to-end via `apply`: `every` (polymorphic) ▸ `gene` (N_Gene) yields
    //     `S/(S\NP_Gene)` (variable resolved) and the sem `det(Gene)`.
    let det = Item::new(poly, det_sem_exp());
    let noun = Item::new(noun_cat, Exp::EigonClass(gene.clone()));
    let out = apply(
        &det,
        &noun,
        &layer,
        eigenius_kernel::dcg::RightContext::Other,
    )
    .expect("polymorphic determiner applies to its noun");
    assert_eq!(
        out.cat(),
        &concrete_result,
        "apply must resolve the category variable to Gene"
    );
    assert_eq!(
        out.sem(),
        &Exp::App(Box::new(det_sem_exp()), Box::new(Exp::EigonClass(gene))),
        "apply must build det(Gene) in lockstep"
    );
}

// ════════════════════════════════════════════════════════════════════
// D63 §8.2 item 3 — `cat_forall` denotes Π; the dependent application.
//
// A determiner's category is the CLOSED, kernel-checked value
// `cat_forall(λT:Set. S/(S\NP_T))` (the HOAS binder keeps it free-variable-
// free, so the commit-time felicity check — Rule 21 — admits it; the probe in
// eigenius#92's discussion confirmed the reflexive ctor type-checks). Its
// denotation is `denote_cat`-bound as the polymorphic determiner type
// `ΠT:Set. (T→Prop)→Prop`, so the felicity invariant ⟦cat⟧ ≡ sem_type holds IN
// ISOLATION; and `apply` instantiates it against a noun (T := the noun's type).
// ════════════════════════════════════════════════════════════════════

const DET_CAT_FORALL: &str = r#"
namespace lexicon   = "urn:eigenius:lexicon";
namespace epistemic = "urn:eigenius:reflection:epistemic";
namespace logic     = "urn:eigenius:logic";

// A general one-place predicate over entities — a stand-in VP semantics for the
// item-4 composition test (`Entity -> Prop`, so `Gene` flows in by subsumption).
axiom lexicon:q : lexicon:Entity -> Prop

// The DETERMINERS come from the committed closed-class layer
// (`ontologies/lexicon/closed-class.esl`), which this test's chain includes
// (`build_closed_class`); e.g. `every` is `lexicon:every_subj`. Only the demo
// scaffolding the engine unit tests reference is declared in this snippet.

// The common noun `N_Gene` the determiner consumes comes from the demo lexicon
// (`lexicon:e_gene`, `cat_n(Gene, num_any)`) — build_lexicon provides it, so no
// duplicate is declared here (two "gene" entries would double every parse).

// The supertype common noun `N_Entity` — the wider restrictor used in the FraCaS
// monotonicity check (`every entity …` ⊨ `every gene …`, since Gene ≤ Entity).
resource lexicon:e_entity_noun : lexicon:LexicalEntry {
    lexicon:form     = "entity";
    lexicon:cat      = type_expr( lexicon:cat_n(lexicon:Entity, lexicon:num_any) );
    lexicon:sem      = lexicon:Entity;
    lexicon:sem_type = type_expr( Set );
    lexicon:sense    = "x";
    lexicon:grade    = epistemic:declared;
}

// The expected result of `every ▸ gene`: the concrete `S/(S\NP_Gene)`.
resource lexicon:e_det_result : lexicon:LexicalEntry {
    lexicon:form     = "every gene";
    lexicon:cat      = type_expr(
        lexicon:fwd(lexicon:m_all, 
            lexicon:cat_s(lexicon:dcl, lexicon:fin_any),
            lexicon:bwd(lexicon:m_all, 
                lexicon:cat_s(lexicon:dcl, lexicon:fin),
                lexicon:cat_np(lexicon:Gene, lexicon:sg)
            )
        )
    );
    lexicon:sem      = lexicon:Gene;
    lexicon:sem_type = type_expr( (lexicon:Gene -> Prop) -> Prop );
    lexicon:sense    = "x";
    lexicon:grade    = epistemic:declared;
}
"#;

fn det_poly_layer() -> Arc<Layer> {
    let lexicon = build_lexicon();
    let resources = esl::compile_against_layer(DET_CAT_FORALL, &lexicon)
        .expect("cat_forall determiner snippet compiles");
    let mut b = LayerBuilder::new("det-poly", Some(lexicon));
    for r in &resources {
        b.add_resource(r.clone()).expect("add det-poly resource");
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

fn poly_cat(layer: &Arc<Layer>, entry: &str) -> Exp {
    decoded_field(layer, entry, "urn:eigenius:lexicon:cat")
}

#[test]
fn cat_forall_passes_commit_validation() {
    // The closed `cat_forall(λT:Set. …)` category type-checks at commit (Rule 21
    // decode + check_infer): the determiner entry is a legitimate chain Resource.
    let layer = det_poly_layer();
    let errors = Validator::new(layer).validate();
    assert!(
        errors.is_empty(),
        "the cat_forall determiner entry must validate cleanly. {} error(s):\n{}",
        errors.len(),
        errors
            .iter()
            .take(10)
            .map(|e| format!("  - {e}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

#[test]
fn determiner_entry_resolves_and_gates() {
    // Item 5: a complete determiner chain entry. Its `sem` references a committed
    // `SemTerm` whose `term` is the annotated λ-semantics. The entry passes the
    // felicity gate (⟦cat_forall⟧ ≡ sem_type AND the resolved λ-sem inhabits
    // ⟦cat⟧), and `resolve_sem_value` follows the ref + decodes the Ann'd term to
    // exactly the polymorphic determiner term `det_sem_exp()`.
    let layer = det_poly_layer();
    let entry = layer
        .resolve(&Iri::parse("urn:eigenius:lexicon:every_subj").unwrap())
        .expect("determiner entry resolves");

    gate_entry(&layer, &entry).expect("determiner entry must pass the felicity gate");

    let sem_v = entry
        .get(&Iri::parse("urn:eigenius:lexicon:sem").unwrap())
        .expect("entry has sem");
    let sem = resolve_sem_value(&layer, sem_v).expect("SemTerm-referenced λ-sem resolves");
    let got = readback_val(0, &eval(&sem, &Rho::Nil).expect("eval resolved sem"));
    let want = readback_val(
        0,
        &eval(&det_sem_exp(), &Rho::Nil).expect("eval det_sem_exp"),
    );
    assert_eq!(
        got, want,
        "the committed determiner sem must equal det_sem_exp()"
    );
}

#[test]
fn cat_forall_denotes_pi_and_matches_sem_type() {
    // ⟦cat_forall(λT. S/(S\NP_T))⟧ = ΠT:Set. (T→Prop)→Prop ≡ the declared sem_type.
    let layer = det_poly_layer();
    let cat = poly_cat(&layer, "urn:eigenius:lexicon:every_subj");
    let sem_type = decoded_field(
        &layer,
        "urn:eigenius:lexicon:every_subj",
        "urn:eigenius:lexicon:sem_type",
    );
    let denoted = denote_cat(&cat).expect("denote cat_forall");
    assert!(
        type_eq(&denoted, &sem_type),
        "⟦cat_forall⟧ must equal the polymorphic sem_type.\n  ⟦cat⟧    = {denoted:?}\n  sem_type = {sem_type:?}"
    );
}

#[test]
fn cat_forall_gates_in_isolation() {
    // The felicity gate IN ISOLATION: the polymorphic determiner sem inhabits
    // ⟦cat_forall⟧ = ΠT:Set.(T→Prop)→Prop — the per-item discipline holds.
    let layer = det_poly_layer();
    let cat = poly_cat(&layer, "urn:eigenius:lexicon:every_subj");
    let denoted = denote_cat(&cat).expect("denote cat_forall");
    let ty_val = eval(&denoted, &Rho::Nil).expect("eval ⟦cat⟧");
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], layer.clone());
    check(&mut ctx, &det_sem_exp(), &ty_val)
        .expect("determiner sem must inhabit ⟦cat_forall⟧ (gate-able in isolation)");
}

#[test]
fn cat_forall_dependent_application_instantiates_and_stays_felicitous() {
    // `every` (cat_forall) ▸ `gene` (N_Gene): T := Gene, category resolves to
    // `S/(S\NP_Gene)`, sem builds `det(Gene)`, and the produced sem still
    // inhabits the produced category's denotation (felicity preserved).
    let layer = det_poly_layer();
    let gene = Iri::parse("urn:eigenius:lexicon:Gene").unwrap();

    let det = Item::new(
        poly_cat(&layer, "urn:eigenius:lexicon:every_subj"),
        det_sem_exp(),
    );
    let noun = Item::new(
        poly_cat(&layer, "urn:eigenius:lexicon:e_gene"),
        Exp::EigonClass(gene.clone()),
    );
    let expected = poly_cat(&layer, "urn:eigenius:lexicon:e_det_result");

    let out = apply(
        &det,
        &noun,
        &layer,
        eigenius_kernel::dcg::RightContext::Other,
    )
    .expect("cat_forall determiner applies to its noun");
    assert_eq!(
        out.cat(),
        &expected,
        "cat_forall ▸ N_Gene must resolve to S/(S\\NP_Gene)"
    );
    assert_eq!(
        out.sem(),
        &Exp::App(Box::new(det_sem_exp()), Box::new(Exp::EigonClass(gene))),
        "sem must be det(Gene)"
    );

    // Felicity preserved across the step: the (NbE-reduced) produced sem inhabits
    // ⟦S/(S\NP_Gene)⟧ = (Gene→Prop)→Prop.
    let out_ty = eval(&denote_cat(out.cat()).expect("denote result"), &Rho::Nil)
        .expect("eval result denotation");
    let reduced_sem = readback_val(0, &eval(out.sem(), &Rho::Nil).expect("eval out sem"));
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], layer.clone());
    check(&mut ctx, &reduced_sem, &out_ty).expect("det(Gene) must inhabit ⟦S/(S\\NP_Gene)⟧");
}

// ════════════════════════════════════════════════════════════════════
// D63 §8.2 item 4 — contravariant structural subsumption for fwd/bwd.
//
// A functor `A/B` / `A\B` subsumes with function variance: covariant result,
// CONTRAVARIANT argument. So a general VP `S\NP_Entity` fills the determiner-
// result's `S\NP_Gene` slot (`Gene ≤ Entity` ⇒ `Entity→Prop ≤ Gene→Prop`) — the
// step that lets `every gene` compose with a general predicate.
// ════════════════════════════════════════════════════════════════════

#[test]
fn functor_subsumption_is_contravariant_in_the_argument() {
    let layer = det_poly_layer();
    // S\NP_Gene = the determiner-result's argument slot (from e_det_result).
    let det_result = poly_cat(&layer, "urn:eigenius:lexicon:e_det_result");
    let vp_gene = is_ctor(&det_result, "fwd").expect("fwd")[2].clone(); // S\NP_Gene (arg: [0]=mode)
                                                                        // S\NP_Entity = the general verb's VP (from e_affects).
    let affects = poly_cat(&layer, "urn:eigenius:lexicon:e_affects");
    let vp_entity = is_ctor(&affects, "fwd").expect("fwd")[1].clone(); // S\NP_Entity (result: [0]=mode)

    // Contravariant: the more-general `S\NP_Entity` fills the `S\NP_Gene` slot…
    assert!(
        cat_subsumes(&vp_gene, &vp_entity, &layer),
        "S\\NP_Entity must fill an S\\NP_Gene slot (contravariant: Gene ≤ Entity)"
    );
    // …but NOT the reverse — the asymmetry that proves it is contravariant, not
    // covariant (a `Gene`-only VP cannot stand in for an `Entity` VP).
    assert!(
        !cat_subsumes(&vp_entity, &vp_gene, &layer),
        "S\\NP_Gene must NOT fill an S\\NP_Entity slot"
    );
}

#[test]
fn every_gene_q_composes_via_apply_to_a_quantified_prop() {
    // The item-2/3/4 pieces end-to-end through `apply` (no hand-built terms): the
    // determiner-result `S/(S\NP_Gene)` (sem `det(Gene)`) applies forward to the
    // general VP `S\NP_Entity` (sem `q : Entity→Prop`), accepted by contravariant
    // functor subsumption, producing `S` whose sem reduces to `∀x:Gene. q(x) :
    // Prop`. (This is `every_gene_q_composes_and_reduces_to_prop` — but PRODUCED
    // by the parser combinators rather than assembled by hand.)
    let layer = det_poly_layer();
    let gene = Exp::EigonClass(Iri::parse("urn:eigenius:lexicon:Gene").unwrap());
    let q = Exp::EigonAxiom(Iri::parse("urn:eigenius:lexicon:q").unwrap());

    let det_result = Item::new(
        poly_cat(&layer, "urn:eigenius:lexicon:e_det_result"), // S/(S\NP_Gene)
        Exp::App(Box::new(det_sem_exp()), Box::new(gene)),     // det(Gene)
    );
    let affects = poly_cat(&layer, "urn:eigenius:lexicon:e_affects");
    let vp = Item::new(
        is_ctor(&affects, "fwd").expect("fwd")[1].clone(), // S\NP_Entity (result: [0]=mode)
        q,                                                 // q : Entity → Prop
    );

    let out = apply(
        &det_result,
        &vp,
        &layer,
        eigenius_kernel::dcg::RightContext::Other,
    )
    .expect("S/(S\\NP_Gene) must apply to S\\NP_Entity via contravariant subsumption");

    // The produced category is a declarative S → Prop.
    assert!(
        type_eq(&denote_cat(out.cat()).expect("denote S"), &Exp::Sort(0)),
        "the produced category must denote Prop"
    );
    // The produced sem NbE-reduces to a well-typed proposition (∀x:Gene. q(x)).
    let reduced = readback_val(0, &eval(out.sem(), &Rho::Nil).expect("eval sentence sem"));
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], layer.clone());
    let ty = check_infer(&mut ctx, &reduced).expect("composed sentence must type-check");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "`every gene q` (composed via apply) must inhabit Prop"
    );
}

// ════════════════════════════════════════════════════════════════════
// D63 §8.2 item 5 — the determiner milestone, end to end FROM CHAIN ENTRIES.
//
// "every cell line is primary" composed by the CKY parser from COMMITTED lexical
// entries (not hand-built items): the `cat_forall` determiner (its λ-sem resolved
// from its `SemTerm`), the `cell line` common noun, the copula `is`, and the BASE
// predicative `primary` (`S[dcl,bse]\NP_Entity`). The copula `is` supplies
// finiteness — `is primary : S[dcl,fin]\NP` (D63 §8.5 Slice 3a) — and the chart
// yields an `S` whose sem NbE-reduces to `∀c:CellLine. is_primary(c) : Prop`,
// kernel-checked. (`primary` now REQUIRES the copula: a bare `*every cell line
// primary` is not a finite root.)
// ════════════════════════════════════════════════════════════════════
#[test]
fn every_cell_line_is_primary_parses_from_entries_to_a_checked_prop() {
    let layer = det_poly_layer();
    let item = |iri: &str| {
        let r = layer
            .resolve(&Iri::parse(iri).unwrap())
            .unwrap_or_else(|| panic!("entry {iri} resolves"));
        entry_to_item(&layer, &r).unwrap_or_else(|e| panic!("{iri} -> item: {e}"))
    };

    // every · cell line · is · primary   (determiner · noun · copula · base adjective)
    let tokens = vec![
        item("urn:eigenius:lexicon:every_subj"),
        item("urn:eigenius:lexicon:e_cell_line"),
        item("urn:eigenius:lexicon:is_copula"),
        item("urn:eigenius:lexicon:e_primary"),
    ];

    let parses = cky_parse(&tokens, &layer);
    // A spanning parse whose category denotes Prop — a declarative sentence.
    let s = parses
        .iter()
        .find(|it| {
            denote_cat(it.cat())
                .map(|d| type_eq(&d, &Exp::Sort(0)))
                .unwrap_or(false)
        })
        .expect("CKY must yield a spanning S that denotes Prop");

    // Its sem NbE-reduces to a well-typed proposition: ∀c:CellLine. is_primary(c).
    let reduced = readback_val(0, &eval(s.sem(), &Rho::Nil).expect("eval parsed sem"));
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], layer.clone());
    let ty = check_infer(&mut ctx, &reduced).expect("parsed sentence must type-check");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "the parsed quantified sentence must inhabit Prop"
    );
}

// ── Item 5 — the determiner milestone through the STRING bridge ──────
/// `parse("every gene affects HeLa")` via `Parser::parse`: tokenize →
/// seed (determiner / common noun / general verb / named entity) → CKY →
/// felicity filter. Subject quantification + a named object (no copula, no
/// object quantifier). The forest is the `∀g:Gene. affects(HeLa, g) : Prop`
/// reading — every returned parse is an S the kernel confirmed inhabits Prop.
#[test]
fn bridge_parses_every_gene_affects_hela_to_prop() {
    let layer = det_poly_layer();
    let index = Parser::build(layer.clone());
    let forest = index.parse("every gene affects HeLa", &Identity);
    assert!(
        !forest.is_empty(),
        "the determiner sentence must yield at least one felicitous S:Prop parse"
    );
    for p in &forest {
        assert!(is_ctor(p.cat(), "cat_s").is_some(), "each parse is an S");
        let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], layer.clone());
        let ty = check_infer(&mut ctx, p.sem()).expect("parsed (reduced) sem type-checks");
        assert_eq!(
            readback_val(0, &ty),
            Exp::Sort(0),
            "each parse inhabits Prop"
        );
    }
}

// ── Item 5 — OBJECT QUANTIFICATION milestone (the full target) ──────
/// `parse("every gene affects a cell line")` via the string bridge: subject
/// universal (`every gene`, a `cat_forall` GQ) + a transitive verb + an OBJECT
/// existential (`a cell line`, the type-raised object determiner `lexicon:a_obj`).
/// The chart composes `affects` ▸ `a cell line` (backward, contravariant functor
/// subsumption) into the VP, then `every gene` ▸ VP into the sentence. The forest
/// is the `∀g:Gene. ∃c:CellLine. affects(c, g) : Prop` reading — kernel-checked.
#[test]
fn bridge_parses_every_gene_affects_a_cell_line_to_prop() {
    let layer = det_poly_layer();
    let index = Parser::build(layer.clone());
    let forest = index.parse("every gene affects a cell line", &Identity);
    assert!(
        !forest.is_empty(),
        "the doubly-quantified sentence must yield at least one felicitous S:Prop parse"
    );
    for p in &forest {
        assert!(is_ctor(p.cat(), "cat_s").is_some(), "each parse is an S");
        let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], layer.clone());
        let ty = check_infer(&mut ctx, p.sem()).expect("parsed (reduced) sem type-checks");
        assert_eq!(
            readback_val(0, &ty),
            Exp::Sort(0),
            "each parse inhabits Prop"
        );
    }
}

// ── Slice-2 tail — determiner/noun NUMBER AGREEMENT (the Slice-1 deferral) ──
/// A minimal morphology that strips a plural `-s` (so `genes → gene`, marking the
/// surface plural). Enough to exercise the morphological-number seam without the
/// full WordNet Morphy (which the wordnet crate's bridge test covers).
struct PluralS;
impl Lemmatizer for PluralS {
    fn lemmas(&self, surface: &str, _pos: Pos) -> Vec<String> {
        let s = surface.trim().to_lowercase();
        match s.strip_suffix('s') {
            Some(base) if !base.is_empty() => vec![s.clone(), base.to_string()],
            _ => vec![s],
        }
    }
}

#[test]
fn determiner_noun_number_agreement_bites() {
    // The Num feature, made functional end to end: the lexicon stores nouns as
    // `num_any`; `Parser` refines the seed to the SURFACE number (`gene` sg,
    // `genes` pl); the determiner `every` carries `sg` on its `cat_forall`, and
    // `apply` checks agreement. So a singular determiner with a plural noun has
    // NO parse, while the singular agrees.
    let layer = det_poly_layer();
    let index = Parser::build(layer.clone());

    let ok = index.parse("every gene affects HeLa", &PluralS);
    assert!(
        !ok.is_empty(),
        "singular 'every gene affects HeLa' must parse (sg ⊓ sg)"
    );

    let bad = index.parse("every genes affects HeLa", &PluralS);
    assert!(
        bad.is_empty(),
        "'every genes ...' must NOT parse — sg determiner ⊓ pl noun fails agreement"
    );
}

// D63 §8.5 Slice 3c — kind-subject predicate nominals → `subclass_of`. A bare-plural
// common noun is a KIND subject (`cat_kind`, ⟦·⟧ = Set); the kind copula `are`
// relates it to the predicate noun via `ontology:subclass_of`. Distinct from
// "every gene is a cell line" (∀g. is_a(g, CellLine)) — the generic/kind reading.
#[test]
fn kind_subject_predicate_nominal_is_subclass_of() {
    let layer = det_poly_layer();
    let index = Parser::build(layer.clone());
    // "genes are cell lines" → subclass_of(Gene, CellLine) : Prop (opaque; truth is a
    // separate grounding judgment — felicity ≠ truth).
    let forest = index.parse("genes are cell lines", &PluralS);
    assert_eq!(forest.len(), 1, "exactly one kind-subject parse");
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], layer.clone());
    let ty = check_infer(&mut ctx, forest[0].sem()).expect("kind nominal type-checks");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "a kind predicate nominal denotes Prop"
    );
    // Structure: subclass_of(Gene, CellLine) = App(App(ontology:subclass_of, Gene), CellLine).
    match forest[0].sem() {
        Exp::App(f, _) => match &**f {
            Exp::App(g, _) => assert!(
                matches!(&**g, Exp::EigonAxiom(iri) if iri.as_str() == "urn:eigenius:ontology:subclass_of"),
                "head is ontology:subclass_of, got {g:?}"
            ),
            other => panic!("expected subclass_of application, got {other:?}"),
        },
        other => panic!("expected subclass_of(K, C), got {other:?}"),
    }
}

// ════════════════════════════════════════════════════════════════════
// Slice-2 tail — a FraCaS-style MONOTONICITY runner.
//
// FraCaS is an NL entailment battery. The kernel
// is a CHECKER, not a prover, so *generic* FraCaS entailment (which needs proof
// search) is out of scope. But the monotonicity inferences — the ones licensed
// by our generalized-quantifier semantics + coercive subtyping — have a
// CONSTRUCTIVE witness, which the kernel gates. The runner parses premise and
// hypothesis to Props through the bridge, then checks the supplied witness has
// type `⟦premise⟧ → ⟦hypothesis⟧`: a kernel-verified entailment.
// ════════════════════════════════════════════════════════════════════

fn first_prop(forest: &[Item], which: &str) -> Exp {
    assert!(
        !forest.is_empty(),
        "{which} must parse to at least one S:Prop"
    );
    forest[0].sem().clone()
}

/// Returns Ok iff the kernel confirms `witness : ⟦premise⟧ → ⟦hypothesis⟧` —
/// i.e. the entailment holds, witnessed and checked (not proof-searched).
fn treetest_entails(
    layer: &Arc<Layer>,
    index: &Parser,
    premise: &str,
    hypothesis: &str,
    witness: &Exp,
) -> Result<(), String> {
    let p = first_prop(&index.parse(premise, &Identity), "premise");
    let h = first_prop(&index.parse(hypothesis, &Identity), "hypothesis");
    let arrow = Exp::Arrow(Box::new(p), Box::new(h));
    let ty = eval(&arrow, &Rho::Nil).map_err(|e| format!("eval entailment type: {e}"))?;
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], layer.clone());
    check(&mut ctx, witness, &ty).map_err(|e| e.to_string())
}

/// The monotonicity witness `λp. λx. p(x)` — instantiate a universal at a
/// (coerced) element. Type-checks exactly when the quantifier step is licensed.
fn instantiation_witness() -> Exp {
    Exp::Lam(
        Patt::Var("p".into()),
        Box::new(Exp::Lam(
            Patt::Var("x".into()),
            Box::new(Exp::App(
                Box::new(Exp::Var("p".into())),
                Box::new(Exp::Var("x".into())),
            )),
        )),
    )
}

#[test]
fn treetest_every_is_downward_monotone_in_its_restrictor() {
    // FraCaS §1 (generalized quantifiers), monotonicity. `every` is DOWNWARD
    // monotone in its restrictor: narrowing the restrictor preserves truth.
    // `Gene ≤ Entity`, so  "every entity affects HeLa"  ⊨  "every gene affects
    // HeLa".  Witness: λp. λg. p(g) — apply the Entity-universal at the coerced g.
    let layer = det_poly_layer();
    let index = Parser::build(layer.clone());
    treetest_entails(
        &layer,
        &index,
        "every entity affects HeLa",
        "every gene affects HeLa",
        &instantiation_witness(),
    )
    .expect("downward-monotone entailment must be kernel-verified (Holds)");
}

#[test]
fn treetest_rejects_the_invalid_upward_restrictor_step() {
    // The CONVERSE is invalid: "every gene affects HeLa" does NOT entail "every
    // entity affects HeLa" (widening the restrictor of a universal). The same
    // instantiation witness fails to type-check — `p : ∀g:Gene…` applied to an
    // `Entity` has no coercion (Entity ⊄ Gene). The runner reports no entailment.
    let layer = det_poly_layer();
    let index = Parser::build(layer.clone());
    let verdict = treetest_entails(
        &layer,
        &index,
        "every gene affects HeLa",
        "every entity affects HeLa",
        &instantiation_witness(),
    );
    assert!(
        verdict.is_err(),
        "the invalid upward-restrictor step must NOT be kernel-verifiable"
    );
}

/// The ∃-monotonicity witness `λe. λC. λk. e(C)(λg. k(g))` — lift a witness of
/// `∃x:A. P(x)` to `∃x:B. P(x)` when `A ≤ B` (the impredicative ∃ from
/// `exists_sem`): feed the (coerced) A-witness to the wider continuation `k`.
fn exists_monotone_witness() -> Exp {
    let v = |n: &str| Exp::Var(n.into());
    Exp::Lam(
        Patt::Var("e".into()),
        Box::new(Exp::Lam(
            Patt::Var("c".into()),
            Box::new(Exp::Lam(
                Patt::Var("k".into()),
                Box::new(Exp::App(
                    Box::new(Exp::App(Box::new(v("e")), Box::new(v("c")))),
                    Box::new(Exp::Lam(
                        Patt::Var("g".into()),
                        Box::new(Exp::App(Box::new(v("k")), Box::new(v("g")))),
                    )),
                )),
            )),
        )),
    )
}

#[test]
fn treetest_some_is_upward_monotone_in_its_restrictor() {
    // `some` is UPWARD monotone in its restrictor: widening preserves truth.
    // `Gene ≤ Entity`, so  "some gene affects HeLa"  ⊨  "some entity affects
    // HeLa".  Witness: lift the existential witness from Gene to Entity.
    let layer = det_poly_layer();
    let index = Parser::build(layer.clone());
    treetest_entails(
        &layer,
        &index,
        "some gene affects HeLa",
        "some entity affects HeLa",
        &exists_monotone_witness(),
    )
    .expect("upward-monotone existential entailment must be kernel-verified");
}

#[test]
fn treetest_rejects_the_invalid_downward_existential_step() {
    // The converse is invalid: "some entity affects HeLa" does NOT entail "some
    // gene affects HeLa" (narrowing an existential's restrictor). The lift
    // witness fails — `k : ∀x:Gene…` cannot consume the Entity-witness.
    let layer = det_poly_layer();
    let index = Parser::build(layer.clone());
    let verdict = treetest_entails(
        &layer,
        &index,
        "some entity affects HeLa",
        "some gene affects HeLa",
        &exists_monotone_witness(),
    );
    assert!(
        verdict.is_err(),
        "the invalid downward-existential step must NOT be kernel-verifiable"
    );
}

#[test]
fn treetest_no_is_downward_monotone_in_its_restrictor() {
    // `no` is DOWNWARD monotone in its restrictor (`no = ∀x. ¬…`, so narrowing
    // preserves): "no entity affects HeLa" ⊨ "no gene affects HeLa". Witness:
    // the same instantiation `λp. λg. p(g)` (a universal instantiated at coerced g).
    let layer = det_poly_layer();
    let index = Parser::build(layer.clone());
    treetest_entails(
        &layer,
        &index,
        "no entity affects HeLa",
        "no gene affects HeLa",
        &instantiation_witness(),
    )
    .expect("downward-monotone negative entailment must be kernel-verified");
}

/// First projection `λm. match m { conj p q => p }` — the conjunction-elimination
/// witness (`P ∧ Q ⊨ P`). `logic:And` is declared with `P, Q` as **parameters**
/// (sort-typed at `Prop`), so they are fixed across the recursor motive; the
/// `conj` arm binds only `p : P`, `q : Q` (two bindings, not four), and the
/// `Match` synthesizes the constant motive `λ_. P` — a `Prop`-valued motive, which
/// is the always-admissible (subsingleton) elimination. This is exactly Lean's
/// `And.left`; it is a plain parametric recursor, no index abstraction.
fn conjunction_elim_witness() -> Exp {
    Exp::Lam(
        Patt::Var("m".into()),
        Box::new(Exp::Match {
            scrutinee: Box::new(Exp::Var("m".into())),
            arms: vec![MatchArm {
                ctor_name: "conj".into(),
                bindings: vec![Patt::Var("p".into()), Patt::Var("q".into())],
                body: Exp::Var("p".into()),
            }],
        }),
    )
}

#[test]
fn treetest_conjunction_elimination_holds() {
    // FraCaS conjunction elimination: a coordinated `S` entails either conjunct.
    // "HeLa affects BRCA1 and BRCA1 affects HeLa"  ⊨  "HeLa affects BRCA1".
    // Premise sem is `logic:And(P, Q)`; the witness `λm. match m { conj p q => p }`
    // is the kernel-checked proof of `⟦premise⟧ → ⟦hypothesis⟧` = `And(P,Q) → P`.
    let layer = det_poly_layer();
    let index = Parser::build(layer.clone());
    treetest_entails(
        &layer,
        &index,
        "HeLa affects BRCA1 and BRCA1 affects HeLa",
        "HeLa affects BRCA1",
        &conjunction_elim_witness(),
    )
    .expect("conjunction elimination must be kernel-verified (Holds)");
}

#[test]
fn treetest_rejects_a_non_conjunct_as_a_conjunction_consequence() {
    // The projection only licenses the actual conjuncts: "HeLa affects BRCA1 and
    // BRCA1 affects HeLa" does NOT entail an unrelated "HeLa affects HeLa" — the
    // first projection yields `P` (= affects(BRCA1, HeLa)), which is not the
    // hypothesis prop, so the witness fails to type-check.
    let layer = det_poly_layer();
    let index = Parser::build(layer.clone());
    let verdict = treetest_entails(
        &layer,
        &index,
        "HeLa affects BRCA1 and BRCA1 affects HeLa",
        "HeLa affects HeLa",
        &conjunction_elim_witness(),
    );
    assert!(
        verdict.is_err(),
        "first projection must not license a non-conjunct consequence"
    );
}

#[test]
fn treetest_rejects_the_invalid_upward_negative_step() {
    // The converse is invalid: "no gene affects HeLa" does NOT entail "no entity
    // affects HeLa" (widening a negative's restrictor).
    let layer = det_poly_layer();
    let index = Parser::build(layer.clone());
    let verdict = treetest_entails(
        &layer,
        &index,
        "no gene affects HeLa",
        "no entity affects HeLa",
        &instantiation_witness(),
    );
    assert!(
        verdict.is_err(),
        "the invalid upward-negative step must NOT be kernel-verifiable"
    );
}

// ── Determiner build-out (§8.3 Phase 2) — new quantifiers parse to checked Prop ──
fn assert_parses_to_prop(layer: &Arc<Layer>, index: &Parser, sentence: &str) {
    let forest = index.parse(sentence, &Identity);
    assert!(
        !forest.is_empty(),
        "'{sentence}' must yield an S:Prop parse"
    );
    for p in &forest {
        assert!(
            is_ctor(p.cat(), "cat_s").is_some(),
            "'{sentence}': each parse is an S"
        );
        let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], layer.clone());
        let ty = check_infer(&mut ctx, p.sem())
            .unwrap_or_else(|e| panic!("'{sentence}' must type-check: {e}"));
        assert_eq!(
            readback_val(0, &ty),
            Exp::Sort(0),
            "'{sentence}' must inhabit Prop"
        );
    }
}

#[test]
fn buildout_some_and_no_determiners_parse_to_prop() {
    let layer = det_poly_layer();
    let index = Parser::build(layer.clone());
    // some (subject existential):  ∃g:Gene. affects(HeLa, g)
    assert_parses_to_prop(&layer, &index, "some gene affects HeLa");
    // no (subject negative):       ∀g:Gene. ¬affects(HeLa, g)
    assert_parses_to_prop(&layer, &index, "no gene affects HeLa");
    // no (object negative, via logic:False under ∃-less ∀¬):
    //   every gene affects no cell line  →  ∀g:Gene. ∀c:CellLine. ¬affects(c, g)
    assert_parses_to_prop(&layer, &index, "every gene affects no cell line");
}

/// P2.2a — `the` and the demonstratives `this`/`that` are determiners (closed-class
/// function words gating real prose), modeled on `a` as a definite/demonstrative ≈
/// existential first-cut (proper iota/deixis deferred). They must parse in both
/// subject and object position, so the WRN-style NPs they head leave the OOV stream.
#[test]
fn buildout_definite_and_demonstrative_determiners_parse_to_prop() {
    let layer = det_poly_layer();
    let index = Parser::build(layer.clone());
    // the (subject + object)
    assert_parses_to_prop(&layer, &index, "the gene affects HeLa");
    assert_parses_to_prop(&layer, &index, "every gene affects the cell line");
    // this (subject + object)
    assert_parses_to_prop(&layer, &index, "this gene affects HeLa");
    assert_parses_to_prop(&layer, &index, "every gene affects this cell line");
    // that (demonstrative — distinct from the relativizer `that`; subject + object)
    assert_parses_to_prop(&layer, &index, "that gene affects HeLa");
    assert_parses_to_prop(&layer, &index, "every gene affects that cell line");
    // an (pre-vocalic spelling of the indefinite article `a`; subject + object —
    // a/an phonology is orthographic, so the parser treats `an` exactly as `a`)
    assert_parses_to_prop(&layer, &index, "an gene affects HeLa");
    assert_parses_to_prop(&layer, &index, "every gene affects an cell line");
}
