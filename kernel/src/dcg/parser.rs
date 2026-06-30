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

//! The composition parser: parse items, forward/backward application, and a CKY
//! chart over categorial categories. The categorial type drives composition; the
//! kernel confirms the assembled term is well-typed (the felicity oracle).

use std::sync::Arc;

use crate::layer::Layer;
use crate::nbe::term::{Exp, Patt};

use super::category::{
    cat_subsumes, distribute, distribute_object, feat_meets, is_ctor, subst_cat, unify_cat,
    CatSubst,
};

/// The combinator that produced a constituent — its **provenance**, tracked so the
/// **Eisner normal form** (D63 §8.5 Slice 5c, §8.9 Slice 6-T) can constrain a
/// derivation by how its inputs were built. ENF's forward constraint keys on
/// `ForwardComp` (a `>B` output may not be the primary functor of a subsequent
/// `>` / `>B`) and on `TypeRaised` (a raised functor may only *compose*, never
/// *apply*).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Combinator {
    /// Forward application (`>`) or the dependent `cat_forall` application.
    ForwardApp,
    /// Backward application (`<`).
    BackwardApp,
    /// Forward composition (`>B`) — the one ENF's forward constraint blocks as a functor.
    ForwardComp,
    /// Backward harmonic composition (`<B`, combinatory-core spike): `Y\Z · X\Y → X\Z`.
    BackwardComp,
    /// Crossed composition (`>Bx` / `<Bx`, combinatory-core spike): `A/B · B\C → A\C` and
    /// `Y/Z · X\Y → X/Z`. Like `ForwardComp`, an ENF-constrained functor (may not be the primary
    /// of a subsequent application/composition).
    CrossedComp,
    /// Forward bounded **type-raising** (`T`, D63 §8.9 Slice 6-T): an `NP_X` raised to
    /// `S/(S\NP_X)`. ENF blocks it from forward *application* — a raised functor may
    /// only *compose* (`>B`), which is what builds the object-extraction `S/NP` body
    /// of a relative clause. This kills the spurious `T`-application duplicate of plain
    /// backward application, keeping declaratives single-parse (the regression gate).
    TypeRaised,
    /// Any other producer (lexical leaf, coordination, group/distributive rules) —
    /// not a composition output, so ENF never constrains it.
    Other,
}

/// The 2-component additive **rank key** for a parse (D65 §4.2): lexicon
/// precedence (primary) then sense-frequency (secondary). The combinators **sum**
/// both components across a parse's leaves; the forest sorts **lexicographically**
/// by `(lexicon_order, sense_rank)` then caps. Derived `Ord` compares fields in
/// declaration order, giving exactly that lexicographic order.
///
/// The unordered, single-lexicon default leaves `lexicon_order = 0` everywhere —
/// behaviour-identical to the prior scalar `sense_rank` cost (D63 §8.7 Stage B).
/// The kernel never learns either component *means* anything — it sums opaque
/// weights, keeping the engine sense-/lexicon-agnostic (the §6 boundary).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cost {
    /// Σ of each leaf's position in the parse scope's ordered lexicon list
    /// (0 = first / most-preferred; 0 for the unordered default). Primary key.
    pub lexicon_order: u32,
    /// Σ of each leaf's `lexicon:sense_rank` (0 = most-frequent sense). Secondary key.
    pub sense_rank: u32,
}

impl Cost {
    /// The zero cost — the default for closed-class / unranked / unscoped leaves.
    pub const ZERO: Cost = Cost {
        lexicon_order: 0,
        sense_rank: 0,
    };

    /// A leaf cost from just a sense-frequency rank (`lexicon_order = 0`); the
    /// lexical index stamps this, and the scope (if any) overwrites `lexicon_order`.
    pub fn from_sense_rank(sense_rank: u32) -> Cost {
        Cost {
            lexicon_order: 0,
            sense_rank,
        }
    }

    /// Component-wise saturating sum — how the combinators aggregate child costs.
    pub fn saturating_add(self, other: Cost) -> Cost {
        Cost {
            lexicon_order: self.lexicon_order.saturating_add(other.lexicon_order),
            sense_rank: self.sense_rank.saturating_add(other.sense_rank),
        }
    }
}

/// A parse item: a category (`lexicon:Cat` term), its assembled EigenTT sem, the
/// combinator [`Combinator`] that produced it (for Eisner normal form), and its
/// **cost** — the additive [`Cost`] rank key summed by the combinators and used to
/// rank + cap the forest (D63 §8.7 / D65 §4.2). A leaf's cost is set by whoever
/// builds it (the lexical index from the entry's `sense_rank`, the parse scope from
/// the entry's lexicon position); the kernel only sums opaque weights, staying
/// sense-/lexicon-agnostic (the §6 forest-returns boundary).
#[derive(Clone)]
pub struct Item {
    pub cat: Exp,
    pub sem: Exp,
    pub prov: Combinator,
    pub cost: Cost,
}

impl Item {
    /// A leaf / non-combinatory item (a lexical seed, or any constituent not
    /// produced by a composition rule) — `prov = Other`, cost zero. The default
    /// constructor for callers outside `apply`; set a non-zero cost with
    /// [`Item::with_cost`].
    pub fn new(cat: Exp, sem: Exp) -> Self {
        Item {
            cat,
            sem,
            prov: Combinator::Other,
            cost: Cost::ZERO,
        }
    }

    /// Same as [`Item::new`] but with an explicit [`Cost`] — used by the lexical
    /// index to stamp an entry's rank, and by the composition rules that sum costs.
    pub fn with_cost(cat: Exp, sem: Exp, cost: Cost) -> Self {
        Item {
            cat,
            sem,
            prov: Combinator::Other,
            cost,
        }
    }

    /// This item with its cost replaced (preserving cat/sem/prov) — for unary
    /// transforms (type-raise, number refinement) that carry a child's cost through.
    fn at_cost(mut self, cost: Cost) -> Self {
        self.cost = cost;
        self
    }
}

/// One combinatory step: forward (`A/B · B → A`) or backward (`B · A\B → A`),
/// assembling the sem by application in lockstep. The argument category need not
/// equal the slot — it must *unify into* it ([`unify_cat`]): a concrete slot
/// subsumes the argument (so `NP[Gene]` fills an `NP[Entity]` slot), and a
/// schematic slot variable binds to the argument's type. The resulting binding is
/// substituted through the result category ([`subst_cat`]), so a determiner's `T`
/// flows into the produced `S/(S\NP_Gene)`. A non-match returns `None` — the
/// parse-time felicity filter, on the category alone.
///
/// The result's [`Item::cost`] is the **sum** of the two inputs' costs (D63 §8.7
/// Stage B): every combination is binary, so a parse's cost is the sum of its
/// leaves' costs — the additive weight the forest is ranked + capped by. The inner
/// [`apply_combine`] builds the cat/sem/prov; this wrapper stamps the summed cost.
pub fn apply(left: &Item, right: &Item, layer: &Arc<Layer>) -> Option<Item> {
    apply_combine(left, right, layer).map(|it| it.at_cost(left.cost.saturating_add(right.cost)))
}

fn apply_combine(left: &Item, right: &Item, layer: &Arc<Layer>) -> Option<Item> {
    // Dependent forward application (the determiner case, D63 §8.2 item 3): a
    // polymorphic `cat_forall(λT:Set. R[T])` consumes a common noun `N_G` on its
    // right, binding the bound type `T := G` and yielding `R[G]`. The sem applies
    // the determiner to the noun's denotation (the type `G`, CN-as-type) in
    // lockstep — `det(G)`.
    if let Some([det_num, Exp::Lam(Patt::Var(tvar), body)]) = is_ctor(&left.cat, "cat_forall") {
        if let Some([t, noun_num]) = is_ctor(&right.cat, "cat_n") {
            // Determiner/noun number agreement: `every` (sg) ⊓ `gene` (sg) ✓, but
            // `every` ⊓ `genes` (pl) fails. `*_any` meets anything (D63 §5.1).
            if feat_meets(det_num, noun_num) {
                // Refined noun (attributive Σ, D63 §8.5 Slice 3b): the noun's type
                // index is a `Σx:C. P`. Bind `T := C` (the **component** type) for the
                // category, so the GQ composes with `Entity`-typed verbs normally; and
                // **Fst-project** the witness in the sem — `λV. det(Σ)(λz. V(Fst z))`.
                // By Σ/Π currying this yields the correct restrictor for *both*
                // quantifiers (∀z:Σ.V(Fst z) = ∀x:C. P(x)→V(x); ∃ likewise with ∧),
                // with no kernel coercion (the `Fst` is inserted here).
                //
                // GATE: the Fst projection is correct ONLY for a determiner whose result category
                // binds a **predicate over the noun type `T`** — i.e. `tvar` occurs in `body` (a
                // GQ's `cat_np(T, …)` VP/argument slot). The predicate-nominal `a_pred` does NOT
                // mention `T` in its body (`S[adj]\NP(Entity)`); `T` feeds only its `is_a(s, T)` sem,
                // and its 2nd argument is the *subject*, not a restrictor `V`. Applying the Fst
                // wrapper to it would land `λz.V(Fst z)` in `is_a`'s subject slot and bind the real
                // subject as a function (an ill-formed term — `NotAFunction` at readback). When
                // `tvar` is absent from `body`, fall through to the simple case (`T := Σ` directly:
                // `a_pred(Σ) = λs. is_a(s, Σ)`), which is correct for the predicate nominal.
                if crate::nbe::check::exp_mentions_var(body, tvar) {
                    if let Exp::Sig(_, comp, _) = t {
                        let mut subst = CatSubst::new();
                        subst.insert(tvar.clone(), (**comp).clone());
                        let (v, z) = ("__refine_v", "__refine_z");
                        let sem = Exp::Lam(
                            Patt::Var(v.into()),
                            Box::new(Exp::App(
                                Box::new(Exp::App(Box::new(left.sem.clone()), Box::new(t.clone()))),
                                Box::new(Exp::Lam(
                                    Patt::Var(z.into()),
                                    Box::new(Exp::App(
                                        Box::new(Exp::Var(v.into())),
                                        Box::new(Exp::Fst(Box::new(Exp::Var(z.into())))),
                                    )),
                                )),
                            )),
                        );
                        return Some(Item {
                            cat: subst_cat(body, &subst),
                            sem,
                            prov: Combinator::ForwardApp,
                            cost: Cost::ZERO,
                        });
                    }
                }
                let mut subst = CatSubst::new();
                subst.insert(tvar.clone(), t.clone());
                return Some(Item {
                    cat: subst_cat(body, &subst),
                    sem: Exp::App(Box::new(left.sem.clone()), Box::new(right.sem.clone())),
                    prov: Combinator::ForwardApp,
                    cost: Cost::ZERO,
                });
            }
        }
    }
    // Eisner normal form (D63 §8.5 Slice 5c, §8.9 Slice 6-T): a forward-composition
    // (`>B`) output may not be the primary (left) functor of `>` / `>B`, and a
    // **type-raised** (`T`) functor may not forward-*apply* (it may only compose).
    // This prunes the spurious composition / type-raise derivations while leaving the
    // extraction case (where the `>B` output is consumed as an *argument*, not a
    // functor) untouched.
    // ENF: a composition output (any of `>B`/`>Bx`/`<B`/`<Bx`) may not be the primary functor of a
    // subsequent application/composition. The crossed/backward variants only exist under the
    // combinatory-core flag, so flag-off behaviour is unchanged.
    let left_is_fwd_comp = matches!(
        left.prov,
        Combinator::ForwardComp | Combinator::CrossedComp | Combinator::BackwardComp
    );
    let left_is_raised = left.prov == Combinator::TypeRaised;
    if !left_is_fwd_comp && !left_is_raised {
        if let Some(args) = is_ctor(&left.cat, "fwd") {
            if args.len() == 2 {
                if let Some(subst) = unify_cat(&args[1], &right.cat, layer) {
                    return Some(Item {
                        cat: subst_cat(&args[0], &subst),
                        sem: Exp::App(Box::new(left.sem.clone()), Box::new(right.sem.clone())),
                        prov: Combinator::ForwardApp,
                        cost: Cost::ZERO,
                    });
                }
            }
        }
    }
    if let Some(args) = is_ctor(&right.cat, "bwd") {
        if args.len() == 2 {
            if let Some(subst) = unify_cat(&args[1], &left.cat, layer) {
                return Some(Item {
                    cat: subst_cat(&args[0], &subst),
                    sem: Exp::App(Box::new(right.sem.clone()), Box::new(left.sem.clone())),
                    prov: Combinator::BackwardApp,
                    cost: Cost::ZERO,
                });
            }
        }
    }
    // Forward composition B (D63 §8.5 Slice 5c): `A/B ∘ B'/C → A/C` when `B'` fills
    // `B` (the left's argument is the right's *result*). Sem is `λz. left(right(z))`.
    // This builds the `S[q]/NP` for a non-adjacent gap ("does HeLa affect" → the
    // sentence-missing-its-object), which the wh-word then consumes. ENF blocks it
    // when the left is itself a `>B` output (above).
    if !left_is_fwd_comp {
        if let (Some(l), Some(r)) = (is_ctor(&left.cat, "fwd"), is_ctor(&right.cat, "fwd")) {
            if l.len() == 2 && r.len() == 2 {
                if let Some(subst) = unify_cat(&l[1], &r[0], layer) {
                    if let Exp::InductiveCtor(decl, _, _) = &left.cat {
                        let result = Exp::InductiveCtor(
                            decl.clone(),
                            "fwd".into(),
                            vec![subst_cat(&l[0], &subst), subst_cat(&r[1], &subst)],
                        );
                        let z = "__comp_z";
                        let sem = Exp::Lam(
                            Patt::Var(z.into()),
                            Box::new(Exp::App(
                                Box::new(left.sem.clone()),
                                Box::new(Exp::App(
                                    Box::new(right.sem.clone()),
                                    Box::new(Exp::Var(z.into())),
                                )),
                            )),
                        );
                        return Some(Item {
                            cat: result,
                            sem,
                            prov: Combinator::ForwardComp,
                            cost: Cost::ZERO,
                        });
                    }
                }
            }
        }
    }
    // Distributive SUBJECT (D63 §8.4 Phase 6): a `cat_group(C, _, num)` subject
    // meeting a VP `S\NP_C'` (backward) distributes — `P` mapped over the members
    // and ⊕-folded (∧/∨ per the group's connective) — when each member fits the
    // predicate's slot (`C ≤ C'`) AND the group's number agrees with the verb
    // (D63 §8.10 6-agr: a plural group takes the plural-finite verb, so
    // `HeLa and BRCA1 affect …` ✓ / `*… affects …` ✗). This is the type-shift that
    // lets a coordinated NP serve as a distributive subject; ordinary backward
    // application can't (a `List C` group sem doesn't fill an individual `NP_C'`).
    if let (Some([c, _conn, gnum]), Some([result, slot])) =
        (is_ctor(&left.cat, "cat_group"), is_ctor(&right.cat, "bwd"))
    {
        let num_agrees =
            matches!(is_ctor(slot, "cat_np"), Some([_, snum]) if feat_meets(gnum, snum));
        if num_agrees && group_member_fits(slot, c, layer) {
            if let Some(sem) = distribute(&left.cat, &left.sem, &right.sem, layer) {
                return Some(Item {
                    cat: result.clone(),
                    sem,
                    prov: Combinator::Other,
                    cost: Cost::ZERO,
                });
            }
        }
    }
    // Distributive OBJECT (D63 §8.4 Phase 6): a transitive verb `V = (S\NP)/NP`
    // (forward) seeking a group object distributes — yielding a VP `λs. V(m₀, s) ⊕
    // V(m₁, s) ⊕ …`. Mirrors the subject case on the verb's object slot; ordinary
    // forward application can't consume a `cat_group` object.
    if let (Some([result, slot]), Some([c, ..])) =
        (is_ctor(&left.cat, "fwd"), is_ctor(&right.cat, "cat_group"))
    {
        if group_member_fits(slot, c, layer) {
            if let Some(sem) = distribute_object(&right.cat, &right.sem, &left.sem, layer) {
                return Some(Item {
                    cat: result.clone(),
                    sem,
                    prov: Combinator::Other,
                    cost: Cost::ZERO,
                });
            }
        }
    }
    // Attributive adjective (D63 §8.5 Slice 3b): an adjectival predicate
    // `S[dcl,adj]\NP` (left) modifying a common noun `cat_n(C)` (right) → the refined
    // noun `cat_n(Σx:C. adj(x))` (CN-as-types restriction, built over the *concrete*
    // `C` so `adj(x)` type-checks directly). Keyed on the `adj` clause form, so it
    // does not fire on (base/finite) verbs. The determiner-over-refined-noun rule
    // above then quantifies over the Σ-type with `Fst`-projection.
    if let (Some([adj_s, _adj_np]), Some([c, noun_num])) =
        (is_ctor(&left.cat, "bwd"), is_ctor(&right.cat, "cat_n"))
    {
        if is_adj_clause(adj_s) {
            if let Exp::InductiveCtor(decl, _, _) = &right.cat {
                // Refine the noun's type with the adjective restriction. If `C` is ALREADY a refined
                // noun `Σx:Base. P(x)` (a stacked adjective — "synthetic **lethal** vulnerability"),
                // CONJOIN over the SAME base: `Σx:Base. P(x) ∧ adj(x)` — a FLAT Σ where every
                // adjective applies to the base entity. Nesting (`Σy:Σ. adj(y)`) would apply `adj` to
                // the Σ *pair* (not `<: Entity`), which is ill-typed — why stacked attributive
                // adjectives didn't parse. The plain (first-adjective) case stays `Σx:C. adj(x)`.
                let sigma = match c {
                    Exp::Sig(Patt::Var(bx), base, p_body)
                        if super::category::resolve_inductive(layer, "urn:eigenius:logic:And")
                            .is_some() =>
                    {
                        let and =
                            super::category::resolve_inductive(layer, "urn:eigenius:logic:And")
                                .unwrap();
                        let adj_at =
                            Exp::App(Box::new(left.sem.clone()), Box::new(Exp::Var(bx.clone())));
                        Exp::Sig(
                            Patt::Var(bx.clone()),
                            base.clone(),
                            Box::new(Exp::InductiveType(and, vec![(**p_body).clone(), adj_at])),
                        )
                    }
                    _ => Exp::Sig(
                        Patt::Var("__refine_x".into()),
                        Box::new(c.clone()),
                        Box::new(Exp::App(
                            Box::new(left.sem.clone()),
                            Box::new(Exp::Var("__refine_x".into())),
                        )),
                    ),
                };
                return Some(Item {
                    cat: Exp::InductiveCtor(
                        decl.clone(),
                        "cat_n".into(),
                        vec![sigma.clone(), noun_num.clone()],
                    ),
                    sem: sigma,
                    prov: Combinator::Other,
                    cost: Cost::ZERO,
                });
            }
        }
    }
    // ── Nominal modification (D63 §8.13 Slice 6-mod) ──────────────────────
    // Two PRE-nominal compound rules: a modifier (left) + a head common noun `cat_n(C)`
    // (right) → the refined noun `cat_n(Σx:C. R(x, m))` over the concrete `C`, where the
    // relation `R` is OPAQUE (institution-mapped). Both reuse 3b's Σ + the
    // determiner-over-refined-noun `Fst` machinery. LEFT-BRANCHING normal form: a
    // compound's HEAD (right) may not itself be a compound result, so a 3+-noun chain has
    // the single bracketing `[[A B] C]` (no spurious `[A [B C]]`); an attributively-refined
    // head is still allowed (a distinct structure, not spurious ambiguity).
    if let Exp::InductiveCtor(decl, name, args) = &right.cat {
        if name == "cat_n" && !is_compound_refined(&right.cat) {
            if let [c, noun_num] = &args[..] {
                // Named-entity compound: `[cat_np] [cat_n(C)]` → Σx:C. compound(x, m), where
                // `m` is the modifier entity (the NP's sem). "BRCA1 cell line".
                if is_ctor(&left.cat, "cat_np").is_some() {
                    let restr = app2(
                        "urn:eigenius:ontology:compound",
                        COMPOUND_X,
                        left.sem.clone(),
                    );
                    return Some(refined_noun(decl, c, noun_num, restr));
                }
                // N-N kind compound: `[cat_n(M)] [cat_n(C)]` → Σx:C. compound_kind(x, M),
                // where the modifier `M` is the left noun's kind (its sem, a `Set` —
                // CN-as-types). "mutator load", "gene cell line".
                if is_ctor(&left.cat, "cat_n").is_some() {
                    let restr = app2(
                        "urn:eigenius:ontology:compound_kind",
                        COMPOUND_X,
                        left.sem.clone(),
                    );
                    return Some(refined_noun(decl, c, noun_num, restr));
                }
            }
        }
    }
    // PP-as-noun-modifier (post-nominal): `[cat_n(C)] [cat_pp]` → Σx:C. pp(x), where
    // ⟦cat_pp⟧ = Entity → Prop is the right's sem (a predicate over the head's entities).
    // "biomarker of WRN dependency" → Σx:Biomarker. prep_of(x, dependency). A category
    // distinct from a bare adjective (`S[adj]\NP`) means a post-nominal adjective never
    // spuriously refines, and distinct from the VP-adjunct preposition so the two
    // attachments are separate parses (PP-attachment ambiguity carried in the forest).
    if let Exp::InductiveCtor(decl, name, args) = &left.cat {
        if name == "cat_n" && is_ctor(&right.cat, "cat_pp").is_some() {
            if let [c, noun_num] = &args[..] {
                let restr = Exp::App(
                    Box::new(right.sem.clone()),
                    Box::new(Exp::Var(COMPOUND_X.into())),
                );
                return Some(refined_noun(decl, c, noun_num, restr));
            }
        }
    }
    // GQ-as-preposition-object (D62 §2): a `cat_pp / NP` preposition (left) consuming a
    // type-raised subject-form GQ `S/(S\NP)` (right) in its object slot — the in-situ scope
    // shift for a quantified or bare-plural NP as a preposition object ("within a gene",
    // "for tumours", "of inhibitors"). This is the parser-side analogue of the verb-object
    // raise (`a_obj`), polymorphic in the functor (the preposition) rather than minting a
    // per-determiner lexical entry: a name fills the prep's `cat_np` slot directly (plain
    // forward application), but a GQ — `λV. Q(A, V)` of type `(Entity→Prop)→Prop` — cannot,
    // so it scopes OVER the preposition instead. ⟦cat_pp⟧ = Entity → Prop; with the prep sem
    // `λy.λx. prep(x, y)` the result is `λx. GQ(λy. prep(x, y))`. The SAME rule covers a
    // closed GQ (`a/the/this gene` ⇒ a closed `cat_pp`) and a deferred bare-plural GQ
    // (`genes` ⇒ a `cat_pp` carrying the quantifier hole `Q`, discharged downstream), since
    // both surface as a subject-form `S/(S\NP)` item. Restricted to the `cat_pp` functor so
    // it never re-derives the verb-object raise (`(S\NP)/NP`, already `a_obj`).
    if let (Some([pp_res, pp_obj]), Some([gq_s, gq_vp])) =
        (is_ctor(&left.cat, "fwd"), is_ctor(&right.cat, "fwd"))
    {
        let prep_is_ppmod =
            is_ctor(pp_res, "cat_pp").is_some() && is_ctor(pp_obj, "cat_np").is_some();
        let gq_is_raised_subject = is_ctor(gq_s, "cat_s").is_some()
            && matches!(is_ctor(gq_vp, "bwd"),
                Some([s, np]) if is_ctor(s, "cat_s").is_some() && is_ctor(np, "cat_np").is_some());
        if prep_is_ppmod && gq_is_raised_subject {
            let (x, y) = ("__pobj_x", "__pobj_y");
            // λy. (prep y) x  — the prep's relation with the head entity `x` fixed, scoped by Q.
            let inner = Exp::Lam(
                Patt::Var(y.into()),
                Box::new(Exp::App(
                    Box::new(Exp::App(
                        Box::new(left.sem.clone()),
                        Box::new(Exp::Var(y.into())),
                    )),
                    Box::new(Exp::Var(x.into())),
                )),
            );
            let sem = Exp::Lam(
                Patt::Var(x.into()),
                Box::new(Exp::App(Box::new(right.sem.clone()), Box::new(inner))),
            );
            return Some(Item {
                cat: pp_res.clone(),
                sem,
                prov: Combinator::Other,
                cost: Cost::ZERO,
            });
        }
    }
    None
}

/// **Combinatory-core spike** (porting core-en's `rules.xml`): the additional CCG composition
/// combinators not in [`apply_combine`] — **forward crossed** (`>Bx`: `A/B · B\C → A\C`), **backward
/// harmonic** (`<B`: `Y\Z · X\Y → X\Z`), and **backward crossed** (`<Bx`: `Y/Z · X\Y → X/Z`). Returns
/// ALL that apply (a pair may admit more than one), for the CKY to add alongside the hand-built rules
/// when the flag is set. Forward harmonic (`>B`) already lives in `apply_combine`. Sem is functional
/// composition `λz. f(g(z))` with the primary functor outermost. ENF: outputs carry a composition
/// provenance so they can't be a subsequent primary functor (the guard in `apply_combine`); a
/// composition output is also barred here from being a primary, mirroring that guard.
pub fn apply_core(left: &Item, right: &Item, layer: &Arc<Layer>) -> Vec<Item> {
    let mut out = Vec::new();
    let primary_blocked = |p: Combinator| {
        matches!(
            p,
            Combinator::ForwardComp | Combinator::CrossedComp | Combinator::BackwardComp
        )
    };
    let z = "__core_z";
    // λz. f(g(z)) — compose `f` (outer/primary) after `g` (inner/secondary).
    let compose_sem = |f: &Exp, g: &Exp| {
        Exp::Lam(
            Patt::Var(z.into()),
            Box::new(Exp::App(
                Box::new(f.clone()),
                Box::new(Exp::App(Box::new(g.clone()), Box::new(Exp::Var(z.into())))),
            )),
        )
    };
    let mk = |decl: &Arc<crate::nbe::term::InductiveDecl>, ctor: &str, a: Exp, b: Exp| {
        Exp::InductiveCtor(decl.clone(), ctor.into(), vec![a, b])
    };

    // Forward family: left is the primary functor `A/B` (fwd); not itself a composition output.
    if !primary_blocked(left.prov) {
        if let (Exp::InductiveCtor(decl, _, _), Some([a, b])) =
            (&left.cat, is_ctor(&left.cat, "fwd"))
        {
            // >Bx (crossed): `A/B · B\C → A\C`. left.arg(B) unifies right.result(B).
            if let Some([rr, rc]) = is_ctor(&right.cat, "bwd") {
                if let Some(subst) = unify_cat(b, rr, layer) {
                    out.push(Item {
                        cat: mk(decl, "bwd", subst_cat(a, &subst), subst_cat(rc, &subst)),
                        sem: compose_sem(&left.sem, &right.sem),
                        prov: Combinator::CrossedComp,
                        cost: Cost::ZERO,
                    });
                }
            }
        }
    }
    // Backward family: right is the primary functor `X\Y` (bwd); not itself a composition output.
    if !primary_blocked(right.prov) {
        if let (Exp::InductiveCtor(decl, _, _), Some([x, y])) =
            (&right.cat, is_ctor(&right.cat, "bwd"))
        {
            // <B (harmonic): `Y\Z · X\Y → X\Z`. left=Y\Z (bwd), unify left.result(Y) ~ right.arg(Y).
            if let Some([ly, lz]) = is_ctor(&left.cat, "bwd") {
                if let Some(subst) = unify_cat(ly, y, layer) {
                    out.push(Item {
                        cat: mk(decl, "bwd", subst_cat(x, &subst), subst_cat(lz, &subst)),
                        sem: compose_sem(&right.sem, &left.sem),
                        prov: Combinator::BackwardComp,
                        cost: Cost::ZERO,
                    });
                }
            }
            // <Bx (crossed): `Y/Z · X\Y → X/Z`. left=Y/Z (fwd), unify left.result(Y) ~ right.arg(Y).
            if let Some([ly, lz]) = is_ctor(&left.cat, "fwd") {
                if let Some(subst) = unify_cat(ly, y, layer) {
                    out.push(Item {
                        cat: mk(decl, "fwd", subst_cat(x, &subst), subst_cat(lz, &subst)),
                        sem: compose_sem(&right.sem, &left.sem),
                        prov: Combinator::CrossedComp,
                        cost: Cost::ZERO,
                    });
                }
            }
        }
    }
    out.into_iter()
        .map(|it| it.at_cost(left.cost.saturating_add(right.cost)))
        .collect()
}

/// The bound variable of every 6-mod Σ-refinement (D63 §8.13).
const COMPOUND_X: &str = "__cmp_x";

/// Apply an opaque binary modifier axiom `R` to `(Var(arg0), arg1)` — the restrictor of a
/// 6-mod Σ. `R(x, m)` where the bound `x` (`arg0`) ranges over the head noun's concrete
/// type and `m` (`arg1`) is the modifier.
fn app2(axiom_iri: &str, arg0: &str, arg1: Exp) -> Exp {
    let r = Exp::EigonAxiom(
        crate::ontology::iri::Iri::parse(axiom_iri).expect("valid modifier axiom iri"),
    );
    Exp::App(
        Box::new(Exp::App(Box::new(r), Box::new(Exp::Var(arg0.into())))),
        Box::new(arg1),
    )
}

/// Build a refined common noun `cat_n(Σx:C. restr, num)` for a 6-mod rule (D63 §8.13),
/// reusing the head noun's `decl` and number; `restr` is the restrictor `Prop` over the
/// bound `COMPOUND_X`. Sem is the Σ itself (CN-as-types); provenance `Other`.
fn refined_noun(
    decl: &Arc<crate::nbe::term::InductiveDecl>,
    c: &Exp,
    noun_num: &Exp,
    restr: Exp,
) -> Item {
    let sigma = Exp::Sig(
        Patt::Var(COMPOUND_X.into()),
        Box::new(c.clone()),
        Box::new(restr),
    );
    Item {
        cat: Exp::InductiveCtor(
            decl.clone(),
            "cat_n".into(),
            vec![sigma.clone(), noun_num.clone()],
        ),
        sem: sigma,
        prov: Combinator::Other,
        cost: Cost::ZERO,
    }
}

/// Whether `cat` is an already-compound-refined common noun — `cat_n(Σ. body, _)` whose
/// restrictor's App-spine head is `ontology:compound` / `compound_kind`. The left-branching
/// normal form (D63 §8.13) forbids such a noun as a compound HEAD, collapsing the spurious
/// bracketings of a 3+-noun compound chain to the single left-branching tree. An
/// *attributively*-refined noun is NOT compound-refined, so adjective+compound still composes.
fn is_compound_refined(cat: &Exp) -> bool {
    if let Some([Exp::Sig(_, _, body), _]) = is_ctor(cat, "cat_n") {
        let mut head = &**body;
        while let Exp::App(f, _) = head {
            head = f;
        }
        return matches!(head, Exp::EigonAxiom(iri)
            if iri.as_str() == "urn:eigenius:ontology:compound"
                || iri.as_str() == "urn:eigenius:ontology:compound_kind");
    }
    false
}

/// Whether `s` is an **adjectival** clause `cat_s(_, adj)` — the predicative
/// adjective form (D63 §8.5 Slice 3b), distinct from verbal `fin`/`bse`.
fn is_adj_clause(s: &Exp) -> bool {
    matches!(is_ctor(s, "cat_s"), Some([_, fin])
        if matches!(fin, Exp::InductiveCtor(_, n, _) if n == "adj"))
}

/// Whether a group's member type `c` fits a predicate's `NP_C'` `slot` — i.e.
/// `C ≤ C'` via the subclass lattice (checked by building a member NP at `c`,
/// reusing the slot's number, and running categorial subsumption).
fn group_member_fits(slot: &Exp, c: &Exp, layer: &Arc<Layer>) -> bool {
    if let Exp::InductiveCtor(decl, name, slot_args) = slot {
        if name == "cat_np" && slot_args.len() == 2 {
            let member_np = Exp::InductiveCtor(
                decl.clone(),
                "cat_np".into(),
                vec![c.clone(), slot_args[1].clone()],
            );
            return cat_subsumes(slot, &member_np, layer);
        }
    }
    false
}

/// CKY: `chart[i][j]` holds every item spanning tokens `i..=j`; returns the
/// items spanning the whole input.
pub fn cky_parse(tokens: &[Item], layer: &Arc<Layer>) -> Vec<Item> {
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
                let lefts = chart[i][k].clone();
                let rights = chart[k + 1][j].clone();
                for l in &lefts {
                    for r in &rights {
                        if let Some(item) = apply(l, r, layer) {
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

#[cfg(test)]
mod tests {
    use super::Cost;

    #[test]
    fn cost_sorts_lexicon_order_before_sense_rank() {
        // D65 §4.2: the rank key is lexicographic — lexicon precedence dominates,
        // sense-frequency tie-breaks within a precedence level.
        let mut v = vec![
            Cost {
                lexicon_order: 1,
                sense_rank: 0,
            }, // preferred lexicon? no — order 1
            Cost {
                lexicon_order: 0,
                sense_rank: 9,
            },
            Cost {
                lexicon_order: 0,
                sense_rank: 1,
            },
        ];
        v.sort();
        assert_eq!(
            v,
            vec![
                Cost {
                    lexicon_order: 0,
                    sense_rank: 1
                }, // order 0 beats order 1 …
                Cost {
                    lexicon_order: 0,
                    sense_rank: 9
                }, // … even at a much worse sense_rank
                Cost {
                    lexicon_order: 1,
                    sense_rank: 0
                },
            ]
        );
    }

    #[test]
    fn cost_saturating_add_is_componentwise() {
        let a = Cost {
            lexicon_order: 2,
            sense_rank: 3,
        };
        let b = Cost {
            lexicon_order: 1,
            sense_rank: 4,
        };
        assert_eq!(
            a.saturating_add(b),
            Cost {
                lexicon_order: 3,
                sense_rank: 7
            }
        );
        // Saturates each component independently, no overflow panic.
        let big = Cost {
            lexicon_order: u32::MAX,
            sense_rank: 0,
        };
        assert_eq!(big.saturating_add(a).lexicon_order, u32::MAX);
    }
}
