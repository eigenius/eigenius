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

/// A parse item: a category (`lexicon:Cat` term), its assembled EigenTT sem, the
/// combinator [`Combinator`] that produced it (for Eisner normal form), and its
/// **cost** — an abstract additive weight summed by the combinators and used to
/// rank + cap the forest (D63 §8.7 Stage B). A leaf's cost is set by whoever builds
/// it (the lexical index sets it from the entry's `lexicon:sense_rank`, so a
/// lower-cost parse uses more-frequent WordNet senses); the kernel never learns the
/// cost *means* sense frequency — it only sums an opaque weight, keeping the engine
/// sense-agnostic (the §6 forest-returns boundary). Cost 0 throughout (closed-class
/// / demo entries) leaves single-parse ordering and the cap unaffected.
#[derive(Clone)]
pub struct Item {
    pub cat: Exp,
    pub sem: Exp,
    pub prov: Combinator,
    pub cost: u32,
}

impl Item {
    /// A leaf / non-combinatory item (a lexical seed, or any constituent not
    /// produced by a composition rule) — `prov = Other`, cost `0`. The default
    /// constructor for callers outside `apply`; set a non-zero cost with
    /// [`Item::with_cost`].
    pub fn new(cat: Exp, sem: Exp) -> Self {
        Item {
            cat,
            sem,
            prov: Combinator::Other,
            cost: 0,
        }
    }

    /// Same as [`Item::new`] but with an explicit leaf `cost` — used by the lexical
    /// index to stamp an entry's `sense_rank` onto its leaf item.
    pub fn with_cost(cat: Exp, sem: Exp, cost: u32) -> Self {
        Item {
            cat,
            sem,
            prov: Combinator::Other,
            cost,
        }
    }

    /// This item with its cost replaced (preserving cat/sem/prov) — for unary
    /// transforms (type-raise, number refinement) that carry a child's cost through.
    fn at_cost(mut self, cost: u32) -> Self {
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
                        cost: 0,
                    });
                }
                let mut subst = CatSubst::new();
                subst.insert(tvar.clone(), t.clone());
                return Some(Item {
                    cat: subst_cat(body, &subst),
                    sem: Exp::App(Box::new(left.sem.clone()), Box::new(right.sem.clone())),
                    prov: Combinator::ForwardApp,
                    cost: 0,
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
    let left_is_fwd_comp = left.prov == Combinator::ForwardComp;
    let left_is_raised = left.prov == Combinator::TypeRaised;
    if !left_is_fwd_comp && !left_is_raised {
        if let Some(args) = is_ctor(&left.cat, "fwd") {
            if args.len() == 2 {
                if let Some(subst) = unify_cat(&args[1], &right.cat, layer) {
                    return Some(Item {
                        cat: subst_cat(&args[0], &subst),
                        sem: Exp::App(Box::new(left.sem.clone()), Box::new(right.sem.clone())),
                        prov: Combinator::ForwardApp,
                        cost: 0,
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
                    cost: 0,
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
                            cost: 0,
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
                    cost: 0,
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
                    cost: 0,
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
                let x = "__refine_x";
                let sigma = Exp::Sig(
                    Patt::Var(x.into()),
                    Box::new(c.clone()),
                    Box::new(Exp::App(
                        Box::new(left.sem.clone()),
                        Box::new(Exp::Var(x.into())),
                    )),
                );
                return Some(Item {
                    cat: Exp::InductiveCtor(
                        decl.clone(),
                        "cat_n".into(),
                        vec![sigma.clone(), noun_num.clone()],
                    ),
                    sem: sigma,
                    prov: Combinator::Other,
                    cost: 0,
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
    None
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
        cost: 0,
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
