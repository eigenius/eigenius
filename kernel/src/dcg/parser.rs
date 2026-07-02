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
    /// A **nominal-modification** step (N-N compound / named-entity compound / PP-noun-modifier /
    /// attributive-adjective refinement) — every one builds a refined noun `cat_n(Σ…)`. Carried so
    /// [`apply`] can add a per-step **cost penalty** ([`COMPOUND_STEP_PENALTY`]): summed by the
    /// combinators, a DEEP noun-pile (many modification steps) then costs strictly more than the
    /// shallow correct parse, so the beam / forest cap keeps the real reading and thins the pile
    /// (GH#97 — the content-noun-compound explosion the cross-POS prune can't touch). ENF-inert.
    Compound,
    /// Any other producer (lexical leaf, coordination, group/distributive rules) —
    /// not a composition output, so ENF never constrains it.
    Other,
}

/// Per-step cost penalty for a nominal-modification ([`Combinator::Compound`]) output (GH#97). Added
/// to `Cost::sense_rank`, which is summed across a parse's steps — so cost grows with modification
/// DEPTH, ranking a deep noun-pile below the shallow correct parse. Small enough not to disturb the
/// lexicon-order primary key; large enough to dominate per-leaf sense-rank noise at a few steps.
pub const COMPOUND_STEP_PENALTY: u32 = 8;

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

/// The **category payload** of a parse item — everything the combination rules
/// ([`apply`]/[`apply_combine`]) are allowed to consume: the category (`lexicon:Cat`
/// term), the producing [`Combinator`] (Eisner normal form), and the additive [`Cost`]
/// rank key. Deliberately **carries no semantics**: the packed-forest design (D63
/// blueprint) requires combinability to be a function of `(category, prov)` alone, so
/// the sem is segregated into [`SemanticPayload`] and never reachable from a rule that
/// only holds a `CategoryPayload`. This is the compile-time form of the "postcondition
/// vs. carry" separation (Hopkins & Langmead 2009).
#[derive(Clone)]
pub struct CategoryPayload {
    pub cat: Exp,
    pub prov: Combinator,
    pub cost: Cost,
}

/// The **semantic payload** of a parse item — the assembled EigenTT sem. Segregated
/// from [`CategoryPayload`] so the combination rules cannot branch on it (the
/// packed-forest soundness invariant). In the eventual lazy design (Harper 1994
/// "Method 3") this becomes a deferred procedure call materialised on demand; today it
/// is the eagerly-built term.
#[derive(Clone)]
pub struct SemanticPayload {
    pub sem: Exp,
}

/// A parse item: a [`CategoryPayload`] (category + provenance + cost — all combination
/// consumes) paired with its [`SemanticPayload`] (the sem — never seen by combination).
/// A leaf's cost is set by whoever builds it (the lexical index from the entry's
/// `sense_rank`, the parse scope from the entry's lexicon position); the kernel only
/// sums opaque weights, staying sense-/lexicon-agnostic (the §6 forest-returns boundary).
#[derive(Clone)]
pub struct Item {
    pub category: CategoryPayload,
    pub semantics: SemanticPayload,
}

impl Item {
    /// Assemble an item from its parts — the single constructor the composition rules
    /// and lexical index build through (replacing the old `Item::from_parts(cat, sem, prov, cost)`
    /// literal now that the fields live in two payloads).
    pub fn from_parts(cat: Exp, sem: Exp, prov: Combinator, cost: Cost) -> Self {
        Item {
            category: CategoryPayload { cat, prov, cost },
            semantics: SemanticPayload { sem },
        }
    }

    /// A leaf / non-combinatory item (a lexical seed, or any constituent not
    /// produced by a composition rule) — `prov = Other`, cost zero.
    pub fn new(cat: Exp, sem: Exp) -> Self {
        Self::from_parts(cat, sem, Combinator::Other, Cost::ZERO)
    }

    /// Same as [`Item::new`] but with an explicit [`Cost`] — used by the lexical
    /// index to stamp an entry's rank, and by the composition rules that sum costs.
    pub fn with_cost(cat: Exp, sem: Exp, cost: Cost) -> Self {
        Self::from_parts(cat, sem, Combinator::Other, cost)
    }

    /// This item with its cost replaced (preserving cat/sem/prov) — for unary
    /// transforms (type-raise, number refinement) that carry a child's cost through.
    fn at_cost(mut self, cost: Cost) -> Self {
        self.category.cost = cost;
        self
    }

    /// The category term (`&self.category.cat`).
    pub fn cat(&self) -> &Exp {
        &self.category.cat
    }
    /// The assembled sem (`&self.semantics.sem`).
    pub fn sem(&self) -> &Exp {
        &self.semantics.sem
    }
    /// The producing combinator (Eisner normal form provenance).
    pub fn prov(&self) -> Combinator {
        self.category.prov
    }
    /// The additive rank key.
    pub fn cost(&self) -> Cost {
        self.category.cost
    }
    /// Replace the sem in place (the per-span hole freshening).
    pub fn set_sem(&mut self, sem: Exp) {
        self.semantics.sem = sem;
    }
}

/// Combine two adjacent constituents (the CKY step). Combinability is decided **sem-blind** by
/// [`combinable`] (it receives only [`CategoryPayload`]s — the compile-time form of the packed-forest
/// soundness invariant: the packing signature `(cat_shape, ENF-prov)` is sound because the DECISION
/// is a function of the categories alone), then [`build`] materialises the item from the full items
/// (its sem, and — for the dependent nominal rules whose result TYPE embeds modifier meaning — its
/// category). The result's [`Item::cost`] is the **sum** of the two inputs' costs plus the
/// [`COMPOUND_STEP_PENALTY`] for a nominal-modification step (D63 §8.7).
pub fn apply(left: &Item, right: &Item, layer: &Arc<Layer>) -> Option<Item> {
    if let Some(recipe) = combinable(&left.category, &right.category, layer) {
        let it = build(recipe, left, right, layer);
        let mut cost = left.cost().saturating_add(right.cost());
        // Compound-depth penalty (GH#97): each nominal-modification step costs more, so a deep
        // noun-pile ranks below the shallow correct parse and the beam/forest cap keeps the latter.
        if it.prov() == Combinator::Compound {
            cost.sense_rank = cost.sense_rank.saturating_add(COMPOUND_STEP_PENALTY);
        }
        return Some(it.at_cost(cost));
    }
    // Carve-out (Harper 1994 pitfall): the coordination/distributive rules DECIDE on the sem
    // (`group_members` reads the group's `cons/nil` list), so they are NOT sem-blind and are never
    // packed by (cat_shape, ENF-prov). They stay item-level, off the packed path.
    apply_group(left, right, layer).map(|it| {
        let cost = left.cost().saturating_add(right.cost());
        it.at_cost(cost)
    })
}

/// How [`build`] assembles a combined item — the "deferred procedure" (Harper 1994 "Method 3").
/// Each variant carries only CATEGORY-derived data (produced sem-blind by [`combinable`]); the child
/// *sems* are supplied at build time.
enum SemRecipe {
    /// Dependent determiner over a refined noun: category `cat`; sem `λv. L(t)(λz. v(Fst z))`.
    DetRefine { cat: Exp, t: Exp },
    /// Application: category `cat`; sem `L R` (forward) or `R L` (backward).
    Apply { cat: Exp, order: AppOrder },
    /// Forward composition: category `cat`; sem `λz. L(R z)`.
    FwdComp { cat: Exp },
    /// Nominal modification (attributive-Σ / N-N / named / PP): the result CATEGORY embeds the
    /// modifier's meaning (CN-as-types), so [`build`] constructs both category and sem from the sems.
    Refine {
        decl: Arc<crate::nbe::term::InductiveDecl>,
        c: Exp,
        noun_num: Exp,
        kind: RefineKind,
    },
    /// GQ-as-preposition-object raise: category `cat`; sem built from `L`/`R`.
    GqPrepObj { cat: Exp, ppmod: bool },
}

/// Application direction for [`SemRecipe::Apply`] (also fixes the provenance: forward ⇒ `ForwardApp`,
/// backward ⇒ `BackwardApp`).
#[derive(Clone, Copy)]
enum AppOrder {
    Fwd,
    Bwd,
}

/// Which nominal-modification rule a [`SemRecipe::Refine`] represents.
#[derive(Clone, Copy)]
enum RefineKind {
    /// Attributive adjective (D63 §8.5 Slice 3b): flat-Σ conjunction over the base.
    Attrib,
    /// Named-entity compound `[cat_np] [cat_n]` → `Σx:C. compound(x, m)`.
    NamedCompound,
    /// N-N kind compound `[cat_n] [cat_n]` → `Σx:C. compound_kind(x, M)`.
    KindCompound,
    /// PP-as-noun-modifier `[cat_n] [cat_pp]` → `Σx:C. pp(x)`.
    PpMod,
}

/// The **sem-blind combinability decision** (D63 packed-forest blueprint §4/§6): whether the two
/// constituents combine, and how, from their CATEGORIES alone. It is handed [`CategoryPayload`]s, so
/// it *cannot* read a sem — the compile-time guarantee that makes the packing signature `(cat_shape,
/// ENF-prov)` sound. Returns a [`SemRecipe`] carrying the category-derived data [`build`] needs. The
/// coordination/distributive rules are NOT here — they decide on the sem ([`apply_group`]).
fn combinable(
    left: &CategoryPayload,
    right: &CategoryPayload,
    layer: &Arc<Layer>,
) -> Option<SemRecipe> {
    // Dependent forward application (the determiner case, D63 §8.2 item 3): a polymorphic
    // `cat_forall(λT:Set. R[T])` consumes a common noun `N_G`, binding `T := G` → `R[G]`.
    if let Some([det_num, Exp::Lam(Patt::Var(tvar), body)]) = is_ctor(&left.cat, "cat_forall") {
        if let Some([t, noun_num]) = is_ctor(&right.cat, "cat_n") {
            if feat_meets(det_num, noun_num) {
                // Refined noun (attributive Σ): bind `T := C` (the component type) for the category
                // and Fst-project the witness in the sem (built in `build`). GATE: only when `tvar`
                // occurs in `body` (a GQ's predicate slot) — else the predicate-nominal falls through.
                if crate::nbe::check::exp_mentions_var(body, tvar) {
                    if let Exp::Sig(_, comp, _) = t {
                        let mut subst = CatSubst::new();
                        subst.insert(tvar.clone(), (**comp).clone());
                        return Some(SemRecipe::DetRefine {
                            cat: subst_cat(body, &subst),
                            t: t.clone(),
                        });
                    }
                }
                let mut subst = CatSubst::new();
                subst.insert(tvar.clone(), t.clone());
                return Some(SemRecipe::Apply {
                    cat: subst_cat(body, &subst),
                    order: AppOrder::Fwd,
                });
            }
        }
    }
    // Eisner normal form: a composition output may not be the primary functor of `>`/`>B`, and a
    // type-raised functor may only compose (not forward-apply).
    let left_is_fwd_comp = matches!(
        left.prov,
        Combinator::ForwardComp | Combinator::CrossedComp | Combinator::BackwardComp
    );
    let left_is_raised = left.prov == Combinator::TypeRaised;
    // Forward application (`A/B · B → A`).
    if !left_is_fwd_comp && !left_is_raised {
        if let Some(args) = is_ctor(&left.cat, "fwd") {
            if args.len() == 2 {
                if let Some(subst) = unify_cat(&args[1], &right.cat, layer) {
                    return Some(SemRecipe::Apply {
                        cat: subst_cat(&args[0], &subst),
                        order: AppOrder::Fwd,
                    });
                }
            }
        }
    }
    // Backward application (`B · A\B → A`).
    if let Some(args) = is_ctor(&right.cat, "bwd") {
        if args.len() == 2 {
            if let Some(subst) = unify_cat(&args[1], &left.cat, layer) {
                return Some(SemRecipe::Apply {
                    cat: subst_cat(&args[0], &subst),
                    order: AppOrder::Bwd,
                });
            }
        }
    }
    // Forward composition B (`A/B ∘ B'/C → A/C`); sem `λz. L(R z)` built in `build`.
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
                        return Some(SemRecipe::FwdComp { cat: result });
                    }
                }
            }
        }
    }
    // Attributive adjective (D63 §8.5 Slice 3b): `S[dcl,adj]\NP` (left) + `cat_n` (right). The
    // refined noun's TYPE embeds the adjective's predicate, so it is assembled in `build`.
    if let Some([adj_s, _adj_np]) = is_ctor(&left.cat, "bwd") {
        if is_adj_clause(adj_s) {
            if let Exp::InductiveCtor(decl, name, args) = &right.cat {
                if name == "cat_n" {
                    if let [c, noun_num] = &args[..] {
                        return Some(SemRecipe::Refine {
                            decl: decl.clone(),
                            c: c.clone(),
                            noun_num: noun_num.clone(),
                            kind: RefineKind::Attrib,
                        });
                    }
                }
            }
        }
    }
    // Pre-nominal compound (D63 §8.13): a modifier (left) + head common noun `cat_n(C)` (right) →
    // refined noun. LEFT-BRANCHING NF: the head may not itself be a compound result.
    if let Exp::InductiveCtor(decl, name, args) = &right.cat {
        if name == "cat_n" && !is_compound_refined(&right.cat) {
            if let [c, noun_num] = &args[..] {
                // Named-entity compound `[cat_np] [cat_n]`.
                if is_ctor(&left.cat, "cat_np").is_some() {
                    return Some(SemRecipe::Refine {
                        decl: decl.clone(),
                        c: c.clone(),
                        noun_num: noun_num.clone(),
                        kind: RefineKind::NamedCompound,
                    });
                }
                // N-N kind compound `[cat_n] [cat_n]`.
                if is_ctor(&left.cat, "cat_n").is_some() {
                    return Some(SemRecipe::Refine {
                        decl: decl.clone(),
                        c: c.clone(),
                        noun_num: noun_num.clone(),
                        kind: RefineKind::KindCompound,
                    });
                }
            }
        }
    }
    // PP-as-noun-modifier (post-nominal): `[cat_n(C)] [cat_pp]`.
    if let Exp::InductiveCtor(decl, name, args) = &left.cat {
        if name == "cat_n" && is_ctor(&right.cat, "cat_pp").is_some() {
            if let [c, noun_num] = &args[..] {
                return Some(SemRecipe::Refine {
                    decl: decl.clone(),
                    c: c.clone(),
                    noun_num: noun_num.clone(),
                    kind: RefineKind::PpMod,
                });
            }
        }
    }
    // GQ-as-preposition-object raise (D62 §2): a `cat_pp/NP` or VP-adjunct `(S\NP)\(S\NP)/NP`
    // preposition (left) consuming a type-raised subject-form GQ `S/(S\NP)` (right) in its object.
    if let (Some([pp_res, pp_obj]), Some([gq_s, gq_vp])) =
        (is_ctor(&left.cat, "fwd"), is_ctor(&right.cat, "fwd"))
    {
        let obj_is_np = is_ctor(pp_obj, "cat_np").is_some();
        let is_vp = |e: &Exp| {
            matches!(is_ctor(e, "bwd"),
                Some([s, np]) if is_ctor(s, "cat_s").is_some() && is_ctor(np, "cat_np").is_some())
        };
        let prep_is_ppmod = is_ctor(pp_res, "cat_pp").is_some() && obj_is_np;
        let prep_is_vpadjunct =
            obj_is_np && matches!(is_ctor(pp_res, "bwd"), Some([a, b]) if is_vp(a) && is_vp(b));
        let gq_is_raised_subject = is_ctor(gq_s, "cat_s").is_some()
            && matches!(is_ctor(gq_vp, "bwd"),
                Some([s, np]) if is_ctor(s, "cat_s").is_some() && is_ctor(np, "cat_np").is_some());
        if (prep_is_ppmod || prep_is_vpadjunct) && gq_is_raised_subject {
            return Some(SemRecipe::GqPrepObj {
                cat: pp_res.clone(),
                ppmod: prep_is_ppmod,
            });
        }
    }
    None
}

/// Materialise the [`Item`] for a [`SemRecipe`] from the two children's full items — the ONLY place a
/// child sem is read. For the dependent nominal rules ([`SemRecipe::Refine`]) the result CATEGORY
/// also embeds the modifier's meaning (CN-as-types), so it too is built here.
fn build(recipe: SemRecipe, left: &Item, right: &Item, layer: &Arc<Layer>) -> Item {
    match recipe {
        SemRecipe::DetRefine { cat, t } => {
            let (v, z) = ("__refine_v", "__refine_z");
            let sem = Exp::Lam(
                Patt::Var(v.into()),
                Box::new(Exp::App(
                    Box::new(Exp::App(Box::new(left.sem().clone()), Box::new(t))),
                    Box::new(Exp::Lam(
                        Patt::Var(z.into()),
                        Box::new(Exp::App(
                            Box::new(Exp::Var(v.into())),
                            Box::new(Exp::Fst(Box::new(Exp::Var(z.into())))),
                        )),
                    )),
                )),
            );
            Item::from_parts(cat, sem, Combinator::ForwardApp, Cost::ZERO)
        }
        SemRecipe::Apply { cat, order } => {
            let (sem, prov) = match order {
                AppOrder::Fwd => (
                    Exp::App(Box::new(left.sem().clone()), Box::new(right.sem().clone())),
                    Combinator::ForwardApp,
                ),
                AppOrder::Bwd => (
                    Exp::App(Box::new(right.sem().clone()), Box::new(left.sem().clone())),
                    Combinator::BackwardApp,
                ),
            };
            Item::from_parts(cat, sem, prov, Cost::ZERO)
        }
        SemRecipe::FwdComp { cat } => {
            let z = "__comp_z";
            let sem = Exp::Lam(
                Patt::Var(z.into()),
                Box::new(Exp::App(
                    Box::new(left.sem().clone()),
                    Box::new(Exp::App(
                        Box::new(right.sem().clone()),
                        Box::new(Exp::Var(z.into())),
                    )),
                )),
            );
            Item::from_parts(cat, sem, Combinator::ForwardComp, Cost::ZERO)
        }
        SemRecipe::Refine {
            decl,
            c,
            noun_num,
            kind,
        } => build_refine(decl, c, noun_num, kind, left, right, layer),
        SemRecipe::GqPrepObj { cat, ppmod } => {
            let sem = if ppmod {
                // Noun-modifier: `λx. Q(λy. (prep y) x)`.
                let (x, y) = ("__pobj_x", "__pobj_y");
                let inner = Exp::Lam(
                    Patt::Var(y.into()),
                    Box::new(Exp::App(
                        Box::new(Exp::App(
                            Box::new(left.sem().clone()),
                            Box::new(Exp::Var(y.into())),
                        )),
                        Box::new(Exp::Var(x.into())),
                    )),
                );
                Exp::Lam(
                    Patt::Var(x.into()),
                    Box::new(Exp::App(Box::new(right.sem().clone()), Box::new(inner))),
                )
            } else {
                // VP-adjunct: `λV.λs. Q(λx. prep_sem(x)(V)(s))`.
                let (x, v, s) = ("__pobj_x", "__pobj_V", "__pobj_s");
                let applied = Exp::App(
                    Box::new(Exp::App(
                        Box::new(Exp::App(
                            Box::new(left.sem().clone()),
                            Box::new(Exp::Var(x.into())),
                        )),
                        Box::new(Exp::Var(v.into())),
                    )),
                    Box::new(Exp::Var(s.into())),
                );
                let scoped = Exp::App(
                    Box::new(right.sem().clone()),
                    Box::new(Exp::Lam(Patt::Var(x.into()), Box::new(applied))),
                );
                Exp::Lam(
                    Patt::Var(v.into()),
                    Box::new(Exp::Lam(Patt::Var(s.into()), Box::new(scoped))),
                )
            };
            Item::from_parts(cat, sem, Combinator::Other, Cost::ZERO)
        }
    }
}

/// Assemble a nominal-modification refined noun ([`SemRecipe::Refine`]). The result category is a
/// `cat_n(Σ…)` whose Σ body embeds the modifier's semantics (attributive predicate, compound kind,
/// or PP predicate) — the CN-as-types entanglement of category and sem in the nominal domain.
fn build_refine(
    decl: Arc<crate::nbe::term::InductiveDecl>,
    c: Exp,
    noun_num: Exp,
    kind: RefineKind,
    left: &Item,
    right: &Item,
    layer: &Arc<Layer>,
) -> Item {
    match kind {
        RefineKind::Attrib => {
            // If `C` is ALREADY a refined noun `Σx:Base. P(x)` (a stacked adjective), CONJOIN over the
            // SAME base: `Σx:Base. P(x) ∧ adj(x)` — a FLAT Σ. Else `Σx:C. adj(x)`.
            let sigma = match &c {
                Exp::Sig(Patt::Var(bx), base, p_body)
                    if super::category::resolve_inductive(layer, "urn:eigenius:logic:And")
                        .is_some() =>
                {
                    let and = super::category::resolve_inductive(layer, "urn:eigenius:logic:And")
                        .unwrap();
                    let adj_at =
                        Exp::App(Box::new(left.sem().clone()), Box::new(Exp::Var(bx.clone())));
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
                        Box::new(left.sem().clone()),
                        Box::new(Exp::Var("__refine_x".into())),
                    )),
                ),
            };
            Item::from_parts(
                Exp::InductiveCtor(decl, "cat_n".into(), vec![sigma.clone(), noun_num]),
                sigma,
                Combinator::Compound,
                Cost::ZERO,
            )
        }
        RefineKind::NamedCompound => {
            let restr = app2(
                "urn:eigenius:ontology:compound",
                COMPOUND_X,
                left.sem().clone(),
            );
            refined_noun(&decl, &c, &noun_num, restr)
        }
        RefineKind::KindCompound => {
            let restr = app2(
                "urn:eigenius:ontology:compound_kind",
                COMPOUND_X,
                left.sem().clone(),
            );
            refined_noun(&decl, &c, &noun_num, restr)
        }
        RefineKind::PpMod => {
            let restr = Exp::App(
                Box::new(right.sem().clone()),
                Box::new(Exp::Var(COMPOUND_X.into())),
            );
            refined_noun(&decl, &c, &noun_num, restr)
        }
    }
}

/// Coordination/distributive rules — the packed-forest **carve-out** (Harper 1994 pitfall): these
/// DECIDE on the sem (`distribute`/`distribute_object` read the group's `cons/nil` list via
/// `group_members`), so unlike [`combinable`] they are not sem-blind and are never packed by
/// `(cat_shape, ENF-prov)`. Tried only after [`combinable`] returns `None` (the group categories
/// never match a sem-blind rule, so ordering is preserved).
fn apply_group(left: &Item, right: &Item, layer: &Arc<Layer>) -> Option<Item> {
    // Distributive SUBJECT (D63 §8.4 Phase 6): a `cat_group` subject meeting a VP `S\NP` distributes.
    if let (Some([c, _conn, gnum]), Some([result, slot])) = (
        is_ctor(left.cat(), "cat_group"),
        is_ctor(right.cat(), "bwd"),
    ) {
        let num_agrees =
            matches!(is_ctor(slot, "cat_np"), Some([_, snum]) if feat_meets(gnum, snum));
        if num_agrees && group_member_fits(slot, c, layer) {
            if let Some(sem) = distribute(left.cat(), left.sem(), right.sem(), layer) {
                return Some(Item::from_parts(
                    result.clone(),
                    sem,
                    Combinator::Other,
                    Cost::ZERO,
                ));
            }
        }
    }
    // Distributive OBJECT (D63 §8.4 Phase 6): a transitive verb seeking a `cat_group` object.
    if let (Some([result, slot]), Some([c, ..])) = (
        is_ctor(left.cat(), "fwd"),
        is_ctor(right.cat(), "cat_group"),
    ) {
        if group_member_fits(slot, c, layer) {
            if let Some(sem) = distribute_object(right.cat(), right.sem(), left.sem(), layer) {
                return Some(Item::from_parts(
                    result.clone(),
                    sem,
                    Combinator::Other,
                    Cost::ZERO,
                ));
            }
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
    if !primary_blocked(left.prov()) {
        if let (Exp::InductiveCtor(decl, _, _), Some([a, b])) =
            (left.cat(), is_ctor(left.cat(), "fwd"))
        {
            // >Bx (crossed): `A/B · B\C → A\C`. left.arg(B) unifies right.result(B).
            if let Some([rr, rc]) = is_ctor(right.cat(), "bwd") {
                if let Some(subst) = unify_cat(b, rr, layer) {
                    out.push(Item::from_parts(
                        mk(decl, "bwd", subst_cat(a, &subst), subst_cat(rc, &subst)),
                        compose_sem(left.sem(), right.sem()),
                        Combinator::CrossedComp,
                        Cost::ZERO,
                    ));
                }
            }
        }
    }
    // Backward family: right is the primary functor `X\Y` (bwd); not itself a composition output.
    if !primary_blocked(right.prov()) {
        if let (Exp::InductiveCtor(decl, _, _), Some([x, y])) =
            (right.cat(), is_ctor(right.cat(), "bwd"))
        {
            // <B (harmonic): `Y\Z · X\Y → X\Z`. left=Y\Z (bwd), unify left.result(Y) ~ right.arg(Y).
            if let Some([ly, lz]) = is_ctor(left.cat(), "bwd") {
                if let Some(subst) = unify_cat(ly, y, layer) {
                    out.push(Item::from_parts(
                        mk(decl, "bwd", subst_cat(x, &subst), subst_cat(lz, &subst)),
                        compose_sem(right.sem(), left.sem()),
                        Combinator::BackwardComp,
                        Cost::ZERO,
                    ));
                }
            }
            // <Bx (crossed): `Y/Z · X\Y → X/Z`. left=Y/Z (fwd), unify left.result(Y) ~ right.arg(Y).
            if let Some([ly, lz]) = is_ctor(left.cat(), "fwd") {
                if let Some(subst) = unify_cat(ly, y, layer) {
                    out.push(Item::from_parts(
                        mk(decl, "fwd", subst_cat(x, &subst), subst_cat(lz, &subst)),
                        compose_sem(right.sem(), left.sem()),
                        Combinator::CrossedComp,
                        Cost::ZERO,
                    ));
                }
            }
        }
    }
    out.into_iter()
        .map(|it| it.at_cost(left.cost().saturating_add(right.cost())))
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
    Item::from_parts(
        Exp::InductiveCtor(
            decl.clone(),
            "cat_n".into(),
            vec![sigma.clone(), noun_num.clone()],
        ),
        sigma,
        Combinator::Compound,
        Cost::ZERO,
    )
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
