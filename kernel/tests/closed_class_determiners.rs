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

//! D63 §8.3 — the COMMITTED closed-class determiner layer, end to end. The
//! determiners (`every`/`some`/`no`/`a`, subject + object) come from the
//! bootstrapped `ontologies/lexicon/closed-class.esl` — chain data, not test
//! fixtures — and compose with the demo domain (`experiments/lexicon`) through
//! the lookup bridge into kernel-checked propositions.

use std::sync::Arc;

use eigenius_kernel::bootstrap;
use eigenius_kernel::dcg::{is_ctor, Identity, LexicalIndex};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::nbe::check::{check_infer, CheckCtx};
use eigenius_kernel::nbe::env::Rho;
use eigenius_kernel::nbe::readback::readback_val;
use eigenius_kernel::nbe::term::Exp;

const DEMO: &str = include_str!("../../experiments/lexicon/lexicon.esl");

/// Bootstrap (which includes the lexicon schema + `closed-class` determiners),
/// then layer the demo domain (Gene/CellLine, `affects`, `primary`, HeLa, …) on
/// top — so the index sees the committed determiners *and* the demo content.
fn index_over_bootstrap() -> (Arc<Layer>, LexicalIndex) {
    let ctx = bootstrap::bootstrap().expect("bootstrap");
    let resources =
        esl::compile_against_layer(DEMO, ctx.head()).expect("demo compiles on bootstrap");
    let mut b = LayerBuilder::new("demo", Some(Arc::clone(ctx.head())));
    for r in resources {
        b.add_resource(r).expect("add demo resource");
    }
    let layer = Arc::new(b.build(LayerStorage::in_memory()));
    let index = LexicalIndex::build(Arc::clone(&layer));
    (layer, index)
}

fn assert_parses_to_prop(sentence: &str) {
    let (layer, index) = index_over_bootstrap();
    let forest = index.parse(sentence, &Identity);
    assert!(
        !forest.is_empty(),
        "'{sentence}' must yield at least one felicitous S:Prop parse from the committed determiners"
    );
    for p in &forest {
        assert!(
            is_ctor(&p.cat, "cat_s").is_some(),
            "'{sentence}': each parse is an S"
        );
        let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
        let ty = check_infer(&mut ctx, &p.sem)
            .unwrap_or_else(|e| panic!("'{sentence}' must type-check: {e}"));
        assert_eq!(
            readback_val(0, &ty),
            Exp::Sort(0),
            "'{sentence}' must inhabit Prop"
        );
    }
}

#[test]
fn committed_subject_determiners_parse() {
    // `every` / `no` (subject) from the committed closed-class layer.
    assert_parses_to_prop("every cell line affects HeLa"); // ∀c:CellLine. affects(HeLa, c)
    assert_parses_to_prop("no cell line affects HeLa"); //    ∀c:CellLine. ¬affects(HeLa, c)
}

#[test]
fn committed_object_determiner_parses() {
    // `a` (object, type-raised) from the committed closed-class layer.
    assert_parses_to_prop("HeLa affects a cell line"); // ∃c:CellLine. affects(c, HeLa)
}

#[test]
fn committed_determiners_compose_both_positions() {
    // Subject `every` + object `a`, both committed.
    assert_parses_to_prop("every cell line affects a cell line");
}

// ── D63 §8.4 Phase 3 — generalized conjunction (connectives) ─────────
// `and`/`or` are parser-level reserved words; coordination pointwise-lifts the
// connective (logic:And/Or) over same-category, Prop-ending conjuncts.

#[test]
fn sentence_coordination_parses() {
    // S and S: "HeLa affects BRCA1 and BRCA1 affects HeLa"
    //   → logic:And(affects(BRCA1, HeLa), affects(HeLa, BRCA1)) : Prop.
    assert_parses_to_prop("HeLa affects BRCA1 and BRCA1 affects HeLa");
}

#[test]
fn vp_coordination_parses() {
    // VP and VP (pointwise lift at S\NP): "HeLa affects BRCA1 and affects HeLa"
    //   → λs. And(affects(BRCA1, s), affects(HeLa, s)) applied to HeLa : Prop.
    assert_parses_to_prop("HeLa affects BRCA1 and affects HeLa");
}

#[test]
fn disjunction_parses() {
    // `or` → logic:Or, same generalized-conjunction machinery.
    assert_parses_to_prop("HeLa affects BRCA1 or BRCA1 affects HeLa");
}

// ── D63 §8.4 Phase 6 — NP coordination as `List`-groups (distributive) ─
// A coordinated NP is a member-retaining group (`cat_group(C, pl)` over `List C`);
// the distributive reading maps a one-place predicate over the members and
// ∧-folds — "X and Y affects Z" → affects(Z,X) ∧ affects(Z,Y).

/// The operands of a left-branching connective chain (`op(op(a, b), c)` / `op(a,
/// b)`) for `conn` ∈ {"And", "Or"}, flattened left-to-right; `None` if `sem` is not
/// headed by that connective.
fn conn_chain(sem: &Exp, conn: &str) -> Option<Vec<Exp>> {
    match sem {
        Exp::InductiveType(decl, args) if decl.name == conn && args.len() == 2 => {
            let mut left = conn_chain(&args[0], conn).unwrap_or_else(|| vec![args[0].clone()]);
            left.push(args[1].clone());
            Some(left)
        }
        _ => None,
    }
}

fn and_conjuncts(sem: &Exp) -> Option<Vec<Exp>> {
    conn_chain(sem, "And")
}

#[test]
fn distributive_np_coordination_parses() {
    // "HeLa and BRCA1 affects HeLa": the coordinated subject is a group
    // [hela, brca1] : List Entity (CellLine ⊔ Gene = Entity); the predicate
    // `affects HeLa` = λs. affects(hela, s) distributes over the members →
    // affects(hela, hela) ∧ affects(hela, brca1) : Prop.
    assert_parses_to_prop("HeLa and BRCA1 affects HeLa");

    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa and BRCA1 affects HeLa", &Identity);
    assert_eq!(forest.len(), 1, "exactly one distributive parse");
    let conjuncts = and_conjuncts(&forest[0].sem)
        .expect("distributive sem is a logic:And of the per-member predications");
    assert_eq!(
        conjuncts.len(),
        2,
        "two members ⇒ two conjuncts; got {}",
        conjuncts.len()
    );
}

#[test]
fn disjunctive_np_coordination_distributes_with_or() {
    // "HeLa or BRCA1 affects HeLa": an `or`-group distributes with ∨ →
    // affects(hela, hela) ∨ affects(hela, brca1) : Prop.
    assert_parses_to_prop("HeLa or BRCA1 affects HeLa");

    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa or BRCA1 affects HeLa", &Identity);
    assert_eq!(
        forest.len(),
        1,
        "exactly one disjunctive-distributive parse"
    );
    let disjuncts = conn_chain(&forest[0].sem, "Or")
        .expect("disjunctive sem is a logic:Or of the per-member predications");
    assert_eq!(disjuncts.len(), 2, "two members ⇒ two disjuncts");
}

#[test]
fn distributive_object_coordination_parses() {
    // Object-position distribution: "HeLa affects BRCA1 and HeLa" — the object is a
    // group [brca1, hela]; the verb distributes over it →
    // affects(brca1, hela) ∧ affects(hela, hela) : Prop.
    assert_parses_to_prop("HeLa affects BRCA1 and HeLa");

    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa affects BRCA1 and HeLa", &Identity);
    assert_eq!(forest.len(), 1, "exactly one distributive-object parse");
    assert_eq!(
        and_conjuncts(&forest[0].sem).map(|c| c.len()),
        Some(2),
        "two object members ⇒ two conjuncts"
    );
}

#[test]
fn distributive_object_coordination_with_or_parses() {
    // Object distribution with `or`: "HeLa affects BRCA1 or HeLa" →
    // affects(brca1, hela) ∨ affects(hela, hela) : Prop.
    assert_parses_to_prop("HeLa affects BRCA1 or HeLa");
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa affects BRCA1 or HeLa", &Identity);
    assert_eq!(forest.len(), 1, "exactly one disjunctive-object parse");
    assert_eq!(
        conn_chain(&forest[0].sem, "Or").map(|c| c.len()),
        Some(2),
        "two object members ⇒ two disjuncts"
    );
}

/// The length of a `List` cons-chain sem (`cons(_, h, t)` / `nil`); `None` if not
/// a well-formed list.
fn cons_len(sem: &Exp) -> Option<usize> {
    match sem {
        Exp::InductiveCtor(_, n, args) if n == "nil" && args.is_empty() => Some(0),
        Exp::InductiveCtor(_, n, args) if n == "cons" && args.len() == 2 => {
            Some(1 + cons_len(&args[1])?)
        }
        _ => None,
    }
}

#[test]
fn collective_np_coordination_parses() {
    // "HeLa and BRCA1 form a complex": the collective verb is typed over the GROUP
    // (`S\Group(Entity)`, ⟦·⟧ = List Entity → Prop), so the coordinated subject —
    // the retained group [hela, brca1] : List Entity — is its argument directly →
    // forms_complex([hela, brca1]) : Prop. No mereological sum entity invented.
    assert_parses_to_prop("HeLa and BRCA1 form a complex");

    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa and BRCA1 form a complex", &Identity);
    assert_eq!(forest.len(), 1, "exactly one collective parse");
    match &forest[0].sem {
        Exp::App(_head, arg) => assert_eq!(
            cons_len(arg),
            Some(2),
            "the collective verb consumes the retained 2-member group list"
        ),
        other => panic!("collective sem must be V applied to the group list, got {other:?}"),
    }
}

#[test]
fn collective_rejects_an_or_group() {
    // Collective is `and`-only: "HeLa or BRCA1 form a complex" has no parse — the
    // collective verb's `conn_and` group slot won't accept an `or`-group, and
    // `cat_group` doesn't distribute (its slot isn't `cat_np`).
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index
            .parse("HeLa or BRCA1 form a complex", &Identity)
            .is_empty(),
        "an or-group must not get a collective reading"
    );
}

#[test]
fn reciprocal_np_coordination_parses() {
    // "HeLa and BRCA1 affects each other": the verb is related over every ordered
    // distinct pair of the subject group's members → affects(brca1, hela) ∧
    // affects(hela, brca1) : Prop. ("each other" is a reserved reciprocal anaphor.)
    assert_parses_to_prop("HeLa and BRCA1 affects each other");

    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa and BRCA1 affects each other", &Identity);
    assert_eq!(forest.len(), 1, "exactly one reciprocal parse");
    // 2 members ⇒ 2 ordered distinct pairs ⇒ 2 conjuncts.
    assert_eq!(
        and_conjuncts(&forest[0].sem).map(|c| c.len()),
        Some(2),
        "two members ⇒ two ordered-pair conjuncts"
    );
}

#[test]
fn reciprocal_three_members_has_six_ordered_pairs() {
    // n members ⇒ n·(n−1) ordered distinct pairs: 3 members → 6 conjuncts.
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa and BRCA1 and HeLa affects each other", &Identity);
    assert_eq!(forest.len(), 1, "exactly one reciprocal parse");
    assert_eq!(
        and_conjuncts(&forest[0].sem).map(|c| c.len()),
        Some(6),
        "three members ⇒ 3·2 = 6 ordered-pair conjuncts"
    );
}

#[test]
fn reciprocal_rejects_an_or_group() {
    // Reciprocity is conjunctive — an `or`-group gets no reciprocal reading.
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index
            .parse("HeLa or BRCA1 affects each other", &Identity)
            .is_empty(),
        "an or-group must not get a reciprocal reading"
    );
}

// ── D63 §8.5 Slice 5b — subject wh-questions ──────────────────────────
// A wh-question denotes its answer-property ⟦Q(T)⟧ = T → Prop. A SUBJECT wh has
// its gap adjacent to the VP, so it composes by plain application (no extraction).

/// The queried type of a `cat_q(T)` result — `T` as an `EigonClass` IRI string.
fn cat_q_type(cat: &Exp) -> Option<String> {
    match is_ctor(cat, "cat_q")?.first()? {
        Exp::EigonClass(iri) => Some(iri.as_str().to_string()),
        _ => None,
    }
}

#[test]
fn subject_wh_what_parses_to_an_entity_answer_property() {
    // "what affects HeLa": the gap is the subject → λx:Entity. affects(hela, x) :
    // Entity → Prop. The result category is Q(Entity); the felicity filter confirms
    // the sem inhabits ⟦Q(Entity)⟧ = Entity → Prop (else the forest would be empty).
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("what affects HeLa", &Identity);
    assert_eq!(forest.len(), 1, "exactly one subject-wh parse");
    assert_eq!(
        cat_q_type(&forest[0].cat).as_deref(),
        Some("urn:eigenius:lexicon:Entity"),
        "'what' queries the Entity top"
    );
    assert!(
        matches!(&forest[0].sem, Exp::Lam(_, _)),
        "the answer-property is a λ (T → Prop), got {:?}",
        forest[0].sem
    );
}

#[test]
fn subject_wh_which_narrows_the_answer_type_to_the_noun() {
    // "which cell line affects HeLa": the restrictor narrows the answer to CellLine
    // → λx:CellLine. affects(hela, x) : CellLine → Prop. The Entity-typed verb fills
    // the `S\NP_CellLine` slot by the contravariant functor subsumption (§8.2 item 4),
    // and the η-expanded `which` sem binds `x:CellLine` so the answer type narrows
    // via the covariant application coercion (CellLine ≤ Entity).
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("which cell line affects HeLa", &Identity);
    assert_eq!(forest.len(), 1, "exactly one restricted subject-wh parse");
    assert_eq!(
        cat_q_type(&forest[0].cat).as_deref(),
        Some("urn:eigenius:lexicon:CellLine"),
        "'which cell line' narrows the queried type to CellLine"
    );
    assert!(
        matches!(&forest[0].sem, Exp::Lam(_, _)),
        "answer-property is a λ"
    );
}

#[test]
fn subject_wh_which_requires_a_noun_restrictor() {
    // `which` is a determiner-shaped wh — it needs a common-noun restrictor on its
    // right; there is no bare-`which` subject reading.
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index.parse("which affects HeLa", &Identity).is_empty(),
        "'which' needs a common-noun restrictor"
    );
}

// ── D63 §8.5 Slice 5a — polar (yes/no) questions ──────────────────────
// Auxiliary inversion: `aux + subject + base-VP → S[q]`. ⟦S[q]⟧ = Prop (the queried
// proposition), `mood`-tagged `q`. Application-only; the aux selects a base VP.

/// The mood of a `cat_s` result (`dcl` / `q`).
fn sentence_mood(cat: &Exp) -> Option<String> {
    match is_ctor(cat, "cat_s")?.first()? {
        Exp::InductiveCtor(_, n, _) => Some(n.clone()),
        _ => None,
    }
}

#[test]
fn polar_question_parses_to_a_queried_prop() {
    // "does HeLa affect BRCA1?": aux inversion → the queried proposition
    // affect(brca1, hela) : Prop, tagged `mood = q` (asked, not asserted).
    let (layer, index) = index_over_bootstrap();
    let forest = index.parse("does HeLa affect BRCA1", &Identity);
    assert_eq!(forest.len(), 1, "exactly one polar parse");
    assert_eq!(
        sentence_mood(&forest[0].cat).as_deref(),
        Some("q"),
        "a polar question is tagged mood = q"
    );
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, &forest[0].sem).expect("polar sem type-checks");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "a polar question denotes Prop"
    );
}

#[test]
fn bare_base_clause_is_not_a_finite_root() {
    // "*HeLa affect BRCA1" — the base-form verb yields a base clause `S[_,bse]`,
    // which is not a standalone finite sentence (the finiteness root gate). Only
    // "HeLa affects BRCA1" (finite) or "does HeLa affect BRCA1" (aux) are roots.
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index.parse("HeLa affect BRCA1", &Identity).is_empty(),
        "a bare base clause must not parse as a finite root"
    );
    // the finite form still parses:
    assert!(
        !index.parse("HeLa affects BRCA1", &Identity).is_empty(),
        "the finite declarative still parses"
    );
}

#[test]
fn auxiliary_requires_a_base_form_complement() {
    // "*does HeLa affects BRCA1" — the aux selects a base VP (`S[dcl,bse]\NP`); the
    // finite "affects" fails the Fin-meet, so there is no parse.
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index.parse("does HeLa affects BRCA1", &Identity).is_empty(),
        "the auxiliary must reject a finite (non-base) complement"
    );
}

// ── D63 §8.5 Slice 5c — object wh-extraction (forward composition B + Eisner) ──
// The object gap is non-adjacent: forward composition builds `S[q]/NP` ("does HeLa
// ∘ affect"), the wh-word consumes it → the answer-property `T → Prop`.

#[test]
fn object_wh_what_extracts_via_composition() {
    // "what does HeLa affect?": `does HeLa` (S[q]/(S[bse]\NP)) >B `affect`
    // ((S[bse]\NP)/NP) → S[q]/NP (λz. affect(z, hela)); `what` consumes it →
    // λx:Entity. affect(x, hela) : Entity → Prop.
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("what does HeLa affect", &Identity);
    assert_eq!(forest.len(), 1, "exactly one object-wh parse");
    assert_eq!(
        cat_q_type(&forest[0].cat).as_deref(),
        Some("urn:eigenius:lexicon:Entity"),
        "object 'what' queries the Entity top"
    );
    assert!(
        matches!(&forest[0].sem, Exp::Lam(_, _)),
        "the answer-property is a λ"
    );
}

#[test]
fn object_wh_which_narrows_to_the_noun() {
    // "which cell line does HeLa affect?": the restrictor narrows the answer to
    // CellLine → λx:CellLine. affect(x, hela) : CellLine → Prop. The composed
    // `S[q]/NP_Entity` fills the `S[q]/NP_CellLine` slot by contravariant subsumption.
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("which cell line does HeLa affect", &Identity);
    assert_eq!(forest.len(), 1, "exactly one restricted object-wh parse");
    assert_eq!(
        cat_q_type(&forest[0].cat).as_deref(),
        Some("urn:eigenius:lexicon:CellLine"),
        "'which cell line' narrows the queried type to CellLine"
    );
}

// ── D63 §8.5 Slice 3a — copula + predicative adjective ────────────────
// `is`/`are` supply finiteness to a BASE adjective predicate ("HeLa is primary").

#[test]
fn copula_with_predicative_adjective_parses() {
    // "HeLa is primary": the copula lifts the base adjective `primary`
    // (S[dcl,bse]\NP) to a finite VP → is_primary(hela) : Prop.
    let (layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa is primary", &Identity);
    assert_eq!(forest.len(), 1, "exactly one copula parse");
    assert_eq!(
        sentence_mood(&forest[0].cat).as_deref(),
        Some("dcl"),
        "a copular predication is a declarative"
    );
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, &forest[0].sem).expect("copula sem type-checks");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "the predication denotes Prop"
    );
}

#[test]
fn bare_adjective_needs_the_copula() {
    // "*HeLa primary" — `primary` is a BASE predicate (S[dcl,bse]\NP); without the
    // copula the clause is non-finite, so it is not a standalone root.
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index.parse("HeLa primary", &Identity).is_empty(),
        "a bare predicative adjective is not a finite root without the copula"
    );
}

// ── D63 §8.5 Slice 3b — attributive adjectives (Σ-refinement, engine-level) ──
// "primary cell line" refines the noun to Σx:CellLine. is_primary(x); a determiner
// quantifies over the Σ-type with Fst-projection (correct restrictor for ∀ and ∃).

#[test]
fn attributive_adjective_existential_parses() {
    // "a primary cell line affects HeLa" → ∃z:(Σx:CellLine. is_primary(x)).
    // affects(Fst z, hela) ≡ ∃x:CellLine. is_primary(x) ∧ affects(x, hela) : Prop.
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("a primary cell line affects HeLa", &Identity);
    assert_eq!(
        forest.len(),
        1,
        "exactly one attributive (existential) parse"
    );
    assert_parses_to_prop("a primary cell line affects HeLa");
}

#[test]
fn attributive_adjective_universal_parses() {
    // "every primary cell line affects HeLa" → ∀z:(Σx:CellLine. is_primary(x)).
    // affects(Fst z, hela) ≡ ∀x:CellLine. is_primary(x) → affects(x, hela) : Prop.
    // (The Σ-type yields the implication restrictor for ∀ uniformly — no kernel
    // coercion; the Fst is engine-inserted.)
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("every primary cell line affects HeLa", &Identity);
    assert_eq!(forest.len(), 1, "exactly one attributive (universal) parse");
    assert_parses_to_prop("every primary cell line affects HeLa");
}

// ── D63 §8.5 Slice 3c — predicate nominals (opaque is_a) ──────────────
// "HeLa is a cell line" → ontology:is_a(hela, CellLine) : Prop — an opaque
// membership claim (the ontology's own relation, grounded downstream by ChainWitness).

#[test]
fn predicate_nominal_parses_to_is_a() {
    // The predicative `a` forms `λs. is_a(s, CellLine)` (an adjectival predicate);
    // the copula lifts it; the subject applies → is_a(hela, CellLine) : Prop.
    let (layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa is a cell line", &Identity);
    assert_eq!(forest.len(), 1, "exactly one predicate-nominal parse");
    assert_eq!(
        sentence_mood(&forest[0].cat).as_deref(),
        Some("dcl"),
        "a predicate nominal is a declarative"
    );
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, &forest[0].sem).expect("predicate-nominal sem type-checks");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "a predicate nominal denotes Prop"
    );
    // Structure: is_a(hela, CellLine) = App(App(ontology:is_a, hela), CellLine).
    match &forest[0].sem {
        Exp::App(f, _) => match &**f {
            Exp::App(g, _) => assert!(
                matches!(&**g, Exp::EigonAxiom(iri) if iri.as_str() == "urn:eigenius:ontology:is_a"),
                "predicate-nominal head is ontology:is_a, got {g:?}"
            ),
            other => panic!("expected is_a application, got {other:?}"),
        },
        other => panic!("expected is_a(s, C), got {other:?}"),
    }
}

#[test]
fn do_support_rejects_an_adjective() {
    // The `adj` category fix (Slice 3b step 1): do-support selects base VERBS, not
    // adjectives, so "*does HeLa primary" has no parse (the aux's `bse` slot rejects
    // the `adj` predicate). With the earlier `adj = bse` conflation this wrongly parsed.
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index.parse("does HeLa primary", &Identity).is_empty(),
        "do-support must reject an adjectival complement"
    );
}

#[test]
fn copula_rejects_a_verbal_complement() {
    // "*HeLa is affects HeLa" — the copula selects a BASE predicate; the finite verb
    // "affects" fails the Fin-meet, so this over-generation is blocked.
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index.parse("HeLa is affects HeLa", &Identity).is_empty(),
        "the copula must reject a finite verbal complement"
    );
}

// ── D63 §8.6 Slice 6-neg — negation (¬P := P → logic:False) ───────────

/// Whether `sem` is a negation `… → logic:False` (an arrow/Π whose codomain is
/// `logic:False`).
fn is_negation(sem: &Exp) -> bool {
    let cod = match sem {
        Exp::Arrow(_, c) => c,
        Exp::Pi(_, _, c) => c,
        _ => return false,
    };
    matches!(&**cod, Exp::InductiveType(decl, _) if decl.name == "False")
}

#[test]
fn verbal_negation_parses() {
    // "HeLa does not affect BRCA1": declarative do-support + `not` over the base VP
    // → affect(brca1, hela) → logic:False : Prop.
    let (layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa does not affect BRCA1", &Identity);
    assert_eq!(forest.len(), 1, "exactly one verbal-negation parse");
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, &forest[0].sem).expect("negation sem type-checks");
    assert_eq!(readback_val(0, &ty), Exp::Sort(0), "negation denotes Prop");
    assert!(
        is_negation(&forest[0].sem),
        "sem is ¬(…) = … → logic:False, got {:?}",
        forest[0].sem
    );
}

#[test]
fn copular_negation_parses() {
    // "HeLa is not primary": copula + `not` over the adjectival predicate →
    // is_primary(hela) → logic:False : Prop.
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa is not primary", &Identity);
    assert_eq!(forest.len(), 1, "exactly one copular-negation parse");
    assert!(
        is_negation(&forest[0].sem),
        "sem is ¬is_primary(hela), got {:?}",
        forest[0].sem
    );
}

// ── D63 §8.9 Slice 6-aux — progressive + perfect auxiliaries ──────────
// Aspect auxiliaries are finiteness-lifters (λP.P): progressive `be` over a present-
// participle (`ger`) VP, perfect `have` over a past-participle (`pss`) VP. Tense/aspect
// erased, so the proposition equals the plain declarative's.

#[test]
fn progressive_auxiliary_parses() {
    // "HeLa is affecting BRCA1": is_prog lifts the `ger` VP "affecting BRCA1"
    // (S[ger]\NP) to a finite VP → affects(brca1, hela) : Prop (aspect erased).
    let (layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa is affecting BRCA1", &Identity);
    assert_eq!(forest.len(), 1, "exactly one progressive parse");
    assert_eq!(
        sentence_mood(&forest[0].cat).as_deref(),
        Some("dcl"),
        "a progressive clause is a finite declarative"
    );
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, &forest[0].sem).expect("progressive sem type-checks");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "progressive denotes Prop"
    );
}

#[test]
fn perfect_auxiliary_parses() {
    // "HeLa has affected BRCA1": has_perf lifts the `pss` VP "affected BRCA1"
    // (S[pss]\NP) to a finite VP → affects(brca1, hela) : Prop (tense erased).
    assert_parses_to_prop("HeLa has affected BRCA1");
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa has affected BRCA1", &Identity);
    assert_eq!(forest.len(), 1, "exactly one perfect parse");
}

#[test]
fn short_passive_parses_with_existential_agent() {
    // "BRCA1 is affected": the passive `be` takes the unsaturated past-participle TV
    // and closes the agent → ∃a:Entity. affects(brca1, a) : Prop (BRCA1 is the patient).
    let (layer, index) = index_over_bootstrap();
    let forest = index.parse("BRCA1 is affected", &Identity);
    assert_eq!(forest.len(), 1, "exactly one short-passive parse");
    assert_eq!(
        sentence_mood(&forest[0].cat).as_deref(),
        Some("dcl"),
        "a passive clause is a finite declarative"
    );
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, &forest[0].sem).expect("passive sem type-checks");
    assert_eq!(readback_val(0, &ty), Exp::Sort(0), "passive denotes Prop");
    // The agent is existentially closed — impredicative ∃ is `∀C:Prop. (…→C) → C`,
    // i.e. the sem is a Π/→ whose ultimate codomain is the bound `C` (a Var), not a
    // bare predicate application. (Distinguishes it from the active "affects".)
    assert!(
        matches!(&forest[0].sem, Exp::Pi(_, _, _) | Exp::Arrow(_, _)),
        "short passive closes the agent with an (impredicative) ∃, got {:?}",
        forest[0].sem
    );
}

// ── D63 §8.9 Slice 6-aux — modal auxiliaries (opaque ◇/□) ─────────────
// Modals wrap the proposition with the opaque logic-layer operators: can/could/may/
// might → Possible, must → Necessary. Do-support-shaped aux over a BASE VP.

/// The head modal operator of `op(P)` = App(EigonAxiom(logic:Possible|Necessary), _).
fn modal_head(sem: &Exp) -> Option<String> {
    match sem {
        Exp::App(f, _) => match &**f {
            Exp::EigonAxiom(iri) => Some(iri.as_str().to_string()),
            _ => None,
        },
        _ => None,
    }
}

#[test]
fn modal_can_wraps_the_proposition_in_possible() {
    // "HeLa can affect BRCA1" → Possible(affects(brca1, hela)) : Prop.
    let (layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa can affect BRCA1", &Identity);
    assert_eq!(forest.len(), 1, "exactly one modal parse");
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, &forest[0].sem).expect("modal sem type-checks");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "a modal claim denotes Prop"
    );
    assert_eq!(
        modal_head(&forest[0].sem).as_deref(),
        Some("urn:eigenius:logic:Possible"),
        "`can` wraps the proposition in the opaque ◇ (logic:Possible)"
    );
}

#[test]
fn modal_must_wraps_the_proposition_in_necessary() {
    // "HeLa must affect BRCA1" → Necessary(affects(brca1, hela)) : Prop.
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa must affect BRCA1", &Identity);
    assert_eq!(forest.len(), 1, "exactly one modal parse");
    assert_eq!(
        modal_head(&forest[0].sem).as_deref(),
        Some("urn:eigenius:logic:Necessary"),
        "`must` wraps the proposition in the opaque □ (logic:Necessary)"
    );
}

#[test]
fn modal_selects_a_base_vp() {
    // The modal aux selects a BASE VP (`S[bse]\NP`), like do-support — so the finite
    // "affects" is rejected: "*HeLa can affects BRCA1" has no parse (Fin-meet failure).
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index.parse("HeLa can affects BRCA1", &Identity).is_empty(),
        "a modal must reject a finite complement (it selects the base form)"
    );
}

#[test]
fn agentive_long_passive_parses_with_the_by_agent() {
    // "BRCA1 is affected by HeLa": `by HeLa` supplies the agent and tags the patient-VP
    // with the `pass` voice feature; passive `be` lifts it → affects(brca1, hela) : Prop
    // — the SAME proposition as active "HeLa affects BRCA1" (agent supplied, not closed).
    let (layer, index) = index_over_bootstrap();
    let forest = index.parse("BRCA1 is affected by HeLa", &Identity);
    assert_eq!(forest.len(), 1, "exactly one agentive-passive parse");
    let mut ctx = CheckCtx::with_layer(Rho::Nil, vec![], Arc::clone(&layer));
    let ty = check_infer(&mut ctx, &forest[0].sem).expect("agentive passive type-checks");
    assert_eq!(
        readback_val(0, &ty),
        Exp::Sort(0),
        "agentive passive denotes Prop"
    );
    // Unlike the short passive (an impredicative ∃ = Π/→), the agent is supplied, so the
    // sem is a direct predication `affects(brca1, hela)` = App(App(affects, _), _).
    match &forest[0].sem {
        Exp::App(f, _) => assert!(
            matches!(&**f, Exp::App(g, _)
                if matches!(&**g, Exp::EigonAxiom(iri) if iri.as_str() == "urn:eigenius:lexicon:affects")),
            "agentive passive is affects(patient, agent), got head {f:?}"
        ),
        other => panic!("expected a direct predication affects(_, _), got {other:?}"),
    }
}

#[test]
fn passive_be_rejects_a_saturated_participle() {
    // The over-generation guard: passive `be` takes the TV *before* its object slot is
    // filled, so once the object is supplied ("affected BRCA1" : S[pss]\NP) it no longer
    // matches — `*HeLa is affected BRCA1` has no parse (no spurious active reading).
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index.parse("HeLa is affected BRCA1", &Identity).is_empty(),
        "passive be must not consume a saturated participle (no `*X is affected Y`)"
    );
}

#[test]
fn aspect_auxiliaries_select_the_right_participle() {
    // The `ger`/`pss` complement slots are exact: the progressive `be` rejects a base
    // verb ("*HeLa is affect BRCA1") and the perfect `have` rejects a gerund
    // ("*HeLa has affecting BRCA1") — finiteness/form mismatch, no parse (fail-closed).
    let (_layer, index) = index_over_bootstrap();
    assert!(
        index.parse("HeLa is affect BRCA1", &Identity).is_empty(),
        "progressive be must reject a base verb (needs a present participle)"
    );
    assert!(
        index
            .parse("HeLa has affecting BRCA1", &Identity)
            .is_empty(),
        "perfect have must reject a gerund (needs a past participle)"
    );
}

#[test]
fn eisner_keeps_polar_single_despite_available_composition() {
    // With forward composition B now globally available, "does HeLa affect BRCA1"
    // could be derived a *second* way (`does HeLa ∘ affect` → S[q]/NP, then apply
    // BRCA1). Eisner normal form blocks that — a `>B` output may not be the functor
    // of `>` — so the application derivation is the *only* parse. This is the
    // regression witness that composition didn't reintroduce spurious ambiguity.
    let (_layer, index) = index_over_bootstrap();
    assert_eq!(
        index.parse("does HeLa affect BRCA1", &Identity).len(),
        1,
        "Eisner NF keeps the polar question a single parse despite B being available"
    );
}

// ── D63 §8.9 Slice 6-T + 6-rel — type-raising + restrictive relatives ──
// `that` is a reserved relativizer; `[noun] that [body]` Σ-refines the noun. A
// SUBJECT relative body is a VP `S\NP` (application only); an OBJECT relative body
// is `S/NP`, built by bounded type-raising `T` (NP → S/(S\NP)) + forward `B`. The
// refined noun then rides the 3b determiner+`Fst` machinery into a full sentence.

#[test]
fn subject_relative_clause_parses() {
    // "every cell line that affects HeLa is primary": the subject relative body
    // "affects HeLa" is a VP `S\NP` (sem λx. affects(hela, x)); the relativizer refines
    // → Σx:CellLine. affects(hela, x); `every`+`Fst` quantifies; `is primary` predicates
    // → ∀z:(Σx:CellLine. affects(hela, x)). is_primary(Fst z) : Prop.
    assert_parses_to_prop("every cell line that affects HeLa is primary");
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("every cell line that affects HeLa is primary", &Identity);
    assert_eq!(forest.len(), 1, "exactly one subject-relative parse");
}

#[test]
fn object_relative_clause_parses() {
    // "every cell line that HeLa affects is primary": the object relative body "HeLa
    // affects" has an object gap — built by type-raising HeLa (NP → S/(S\NP)) then
    // forward-composing with `affects` → `S/NP` (sem λx. affects(x, hela)); the
    // relativizer refines → Σx:CellLine. affects(x, hela) → a kernel-checked Prop.
    assert_parses_to_prop("every cell line that HeLa affects is primary");
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("every cell line that HeLa affects is primary", &Identity);
    assert_eq!(forest.len(), 1, "exactly one object-relative parse");
}

#[test]
fn relative_clause_refines_the_noun_to_a_sigma_over_its_base_type() {
    // The refined noun's restrictor is a Σ over the noun's CONCRETE base type CellLine
    // (the 3b move — so `body(x)` type-checks without kernel bounded quantification),
    // existentially this time: "a cell line that affects HeLa is primary".
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("a cell line that affects HeLa is primary", &Identity);
    assert_eq!(
        forest.len(),
        1,
        "exactly one existential subject-relative parse"
    );
    assert_parses_to_prop("a cell line that affects HeLa is primary");
}

#[test]
fn type_raising_keeps_plain_declaratives_single() {
    // Regression gate (the Eisner `TypeRaised`-can't-apply clause): with `T` now
    // globally available, "HeLa affects BRCA1" could be re-derived as T(HeLa) applied
    // to the VP "affects BRCA1". ENF blocks a raised functor from forward-applying, so
    // the canonical backward-application derivation is the ONLY parse.
    let (_layer, index) = index_over_bootstrap();
    assert_eq!(
        index.parse("HeLa affects BRCA1", &Identity).len(),
        1,
        "type-raising must not reintroduce a spurious declarative parse"
    );
    // And the polar question stays single too (T + the 5c composition both available).
    assert_eq!(
        index.parse("does HeLa affect BRCA1", &Identity).len(),
        1,
        "type-raising must not perturb the polar question's single parse"
    );
}

#[test]
fn nary_distributive_group_is_left_branching_single_parse() {
    // n-ary NP coordination builds a single left-branching group (the Phase-4
    // normal form, here enforced by `coordinate_np` requiring a plain-NP right
    // conjunct): "HeLa and BRCA1 and HeLa affects HeLa" → one parse, three
    // conjuncts.
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse("HeLa and BRCA1 and HeLa affects HeLa", &Identity);
    assert_eq!(
        forest.len(),
        1,
        "n-ary distributive group must have a single parse; got {}",
        forest.len()
    );
    assert_eq!(
        and_conjuncts(&forest[0].sem).map(|c| c.len()),
        Some(3),
        "three members ⇒ three conjuncts"
    );
}

#[test]
fn nary_coordination_has_a_single_left_branching_parse() {
    // Spurious-ambiguity control (D63 §8.4 Phase 4): without a normal form,
    // `A and B and C` yields two logically-equivalent parses (left- vs right-
    // branching `And`). The left-branching normal form keeps exactly one.
    let (_layer, index) = index_over_bootstrap();
    let forest = index.parse(
        "HeLa affects BRCA1 and BRCA1 affects HeLa and HeLa affects BRCA1",
        &Identity,
    );
    assert_eq!(
        forest.len(),
        1,
        "n-ary coordination must have a single (left-branching) parse; got {}",
        forest.len()
    );
}
