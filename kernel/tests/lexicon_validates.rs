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
    cat_subsumes, cky_parse, denote_cat, entry_to_item, gate_entry, is_ctor, resolve_sem, type_eq,
    Identity, Item, LexicalIndex,
};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::nbe::check::{check, check_infer, CheckCtx};
use eigenius_kernel::nbe::env::Rho;
use eigenius_kernel::nbe::eval::eval;
use eigenius_kernel::nbe::readback::readback_val;
use eigenius_kernel::nbe::term::Exp;
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

/// The lexicon SCHEMA layer (ontologies/lexicon) over core→reflection.
fn build_schema() -> Arc<Layer> {
    esl_layer(
        "lexicon-schema",
        include_str!("../../ontologies/lexicon/lexicon-ontology.esl"),
        base_chain(),
    )
}

/// The worked demo DOMAIN (experiments/lexicon) over the schema. A compile error
/// here is the *Expressible* gate failing (the kernel cannot carry the content).
fn build_lexicon() -> Arc<Layer> {
    esl_layer(
        "lexicon",
        include_str!("../../experiments/lexicon/lexicon.esl"),
        build_schema(),
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

// `Item`, `is_ctor`, `entry_to_item`, `apply`, `cky_parse` are the kernel's
// `eigenius_kernel::dcg` engine (imported above). The tests below drive it
// over the worked lexicon; they witness the engine, they do not redefine it.
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
        .filter(|it| is_ctor(&it.cat, "cat_s").is_some())
        .collect();
    assert_eq!(
        sentences.len(),
        1,
        "expected exactly one S parse; got cats {:?}",
        parses.iter().map(|i| &i.cat).collect::<Vec<_>>()
    );

    // The assembled sem must type-check — and to Prop. That is the felicity of
    // the *whole composed sentence*, confirmed by the kernel.
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], lexicon.clone());
    let ty = check_infer(&mut ctx, &sentences[0].sem)
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
        .filter(|it| is_ctor(&it.cat, "cat_s").is_some())
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
        .filter(|it| is_ctor(&it.cat, "cat_s").is_some())
        .collect();
    assert_eq!(
        sentences.len(),
        1,
        "expected exactly one S parse for the general verb; got cats {:?}",
        parses.iter().map(|i| &i.cat).collect::<Vec<_>>()
    );

    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], lexicon.clone());
    let ty = check_infer(&mut ctx, &sentences[0].sem)
        .expect("composed general-verb sentence must type-check via subsumption");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "the composed general-verb sentence must inhabit Prop"
    );
}

// ════════════════════════════════════════════════════════════════════
// The lookup bridge (D62 §8.8.1): string → the forest of typed parses.
// `LexicalIndex` builds a `form → entries` index over the committed lexicon;
// `parse` tokenizes, seeds multi-token spans via the `Lemmatizer` (`Identity`
// here — WordNet's Morphy is witnessed in the `eigenius-wordnet` crate), runs
// CKY, and keeps every full-span S whose assembled sem the kernel types to Prop.
// This joins lookup + multi-span MWE seeding + composition + the felicity oracle
// into the kernel-attached `string → tree(s)` library. The forest is returned
// whole (no selection, no commit — that is the encoding institution's job).
// ════════════════════════════════════════════════════════════════════

#[test]
fn index_covers_the_committed_entries() {
    let index = LexicalIndex::build(build_lexicon());
    assert!(!index.is_empty());
    // the six spike entries (incl. the multiword forms "cell line", "depends on").
    assert!(
        index.len() >= 6,
        "index should cover the committed lexical entries; got {}",
        index.len()
    );
}

#[test]
fn bridge_parses_mwe_sentence_to_prop() {
    let index = LexicalIndex::build(build_lexicon());
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
        is_ctor(&forest[0].cat, "cat_s").is_some(),
        "the parse is an S"
    );
}

#[test]
fn bridge_composes_general_verb_via_subsumption() {
    let index = LexicalIndex::build(build_lexicon());
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
    let index = LexicalIndex::build(build_lexicon());
    // Upper-cased input still resolves: the index is keyed by lowercased form and
    // the tokenizer lowercases.
    let forest = index.parse("HELA DEPENDS ON BRCA1", &Identity);
    assert_eq!(forest.len(), 1, "case-insensitive lookup must still parse");
}

#[test]
fn bridge_returns_empty_forest_for_unknown_words() {
    let index = LexicalIndex::build(build_lexicon());
    assert!(
        index.parse("xyzzy plugh frobnicate", &Identity).is_empty(),
        "no matching entries → no admissible parse (empty forest is a first-class outcome, not an error)"
    );
}

#[test]
fn bridge_yields_no_parse_for_type_mismatch() {
    let index = LexicalIndex::build(build_lexicon());
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
    lexicon:form = "f"; lexicon:cat = type_expr( lexicon:cat_n(lexicon:sg) );
    lexicon:sem = lexicon:CellLine; lexicon:sem_type = type_expr( Set ); lexicon:grade = epistemic:declared;
}
resource lexicon:f_n_pl : lexicon:LexicalEntry {
    lexicon:form = "f"; lexicon:cat = type_expr( lexicon:cat_n(lexicon:pl) );
    lexicon:sem = lexicon:CellLine; lexicon:sem_type = type_expr( Set ); lexicon:grade = epistemic:declared;
}
resource lexicon:f_n_any : lexicon:LexicalEntry {
    lexicon:form = "f"; lexicon:cat = type_expr( lexicon:cat_n(lexicon:num_any) );
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
