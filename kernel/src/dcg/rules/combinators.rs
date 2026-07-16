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

//! **The categorial combinators** — the sem-blind composition rules: forward/backward application,
//! composition (harmonic and crossed), the dependent determiner, and the nominal-modification family.
//!
//! Combinability is decided by [`combinable`], which receives only a
//! [`CategoryPayload`](super::super::item::CategoryPayload) and therefore *cannot* read a sem. That is
//! not a convention — it is the compile-time guarantee that makes the packed forest's
//! `(cat_shape, provenance)` signature sound, since two items sharing a signature must combine
//! identically. [`build`] then materialises the result, and is the only place a child sem is read.
//!
//! This file was `parser.rs`, and it never contained a parser: the chart drivers live in
//! `super::super::chart`, and this holds the rules they apply.

use std::sync::{Arc, LazyLock};

use crate::layer::Layer;
use crate::nbe::term::{Exp, Patt};

use super::super::category::{
    cat_subsumes, feat_meets, is_ctor, match_cat, subst_cat, unify_cat, CatPat, CatSubst,
};
use super::super::item::{CategoryPayload, Combinator, Cost, Item, COMPOUND_STEP_PENALTY};
use super::super::rules::constructions::{distribute, distribute_object};

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
    /// Datafied (Phase 1): [`combine_nominal_mod`] matched a [`RefineRule`] and carries its
    /// sem-`builder` plus the metavariable `binds` the pattern captured.
    Refine {
        builder: RefineBuilder,
        binds: CatSubst,
    },
    /// GQ-as-preposition-object raise: category `cat`; sem built from `L`/`R`.
    GqPrepObj { cat: Exp, kind: PrepObj },
    /// Close naming apposition (D63 §5.3): a SORTAL common noun + a proper NAME → the definite
    /// individual of the sortal kind bearing that name, `kind_of(Σx:sortal. named(x, name))`. `sortal`
    /// is the left `cat_n`'s class; the name is the right item's sem (its referent, used as the name
    /// token). Result category `cat_np(Entity, sg)` (a bare proper-name NP), built in [`build`].
    Name { sortal: Exp },
}

/// Application direction for [`SemRecipe::Apply`] (also fixes the provenance: forward ⇒ `ForwardApp`,
/// backward ⇒ `BackwardApp`).
#[derive(Clone, Copy)]
enum AppOrder {
    Fwd,
    Bwd,
}

/// A nominal-modification **sem-builder**: assembles the refined-noun [`Item`] from the metavariable
/// `binds` the trigger captured and the two child sems — the sem half of a datafied [`RefineRule`],
/// and (with [`build`]) the only place a child sem is read. One per rule (`refine_attrib`, …); they
/// are the extracted arms of the former `build_refine`.
type RefineBuilder = fn(&CatSubst, &Item, &Item, &Arc<Layer>) -> Item;

/// How a preposition-object combination `[prep] [raised-GQ]` attaches — decided by [`combinable`] from
/// categories, consumed by [`build`]. `PpMod` → a post-nominal `cat_pp` (noun modifier); `VpAdjunct` →
/// a `(S\NP)\(S\NP)` VP modifier; `ArgMarker` → an argument `cat_pp_arg` (the object entity itself, for a
/// verb that subcategorizes for a PP — "contributes to cancers").
#[derive(Clone, Copy)]
enum PrepObj {
    PpMod,
    VpAdjunct,
    ArgMarker,
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
    combine_determiner(left, right)
        .or_else(|| combine_universal(left, right, layer))
        .or_else(|| combine_nominal_mod(left, right))
        .or_else(|| combine_other_grammar(left, right))
}

/// **Dependent determiner application** (D63 §8.2 item 3) — a polymorphic `cat_forall(λT. R[T])`
/// consuming a common noun, binding `T := G`. A grammar-specific specialization of forward
/// application; tried first, preserving the original arm order (its `cat_forall` trigger is disjoint
/// from every other group, so the position is not load-bearing — the [`combinable`] split off the old
/// linear body is order-preserving by construction).
fn combine_determiner(left: &CategoryPayload, right: &CategoryPayload) -> Option<SemRecipe> {
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
    None
}

/// The **universal CCG combinators** — forward/backward application and forward (harmonic)
/// composition, plus the Eisner normal-form guards. Category-generic: no ontology axiom, no nominal
/// knowledge. This is the calculus, not the grammar — the group Phase 1+ leaves hand-written. Its
/// `fwd`/`bwd`-keyed triggers are disjoint from the grammar-specific groups, so the split is
/// order-preserving.
fn combine_universal(
    left: &CategoryPayload,
    right: &CategoryPayload,
    layer: &Arc<Layer>,
) -> Option<SemRecipe> {
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
    None
}

/// The **nominal-modification family** (D63 §8.5/§8.13) — attributive adjective, named-entity and
/// N-N compounds, and post-nominal PP — now **data-driven** (Phase 1,
/// `docs/notes/grammar-formalization-plan.md`). Each rule is a [`RefineRule`] (structural [`CatPat`]
/// triggers + sem-blind category guards + a sem-`builder`); this function is the interpreter: try the
/// rules in priority order, and on the first whose patterns match and guards hold, defer to its
/// builder. Sem-blind like all of [`combinable`] — a [`Guard`] reads only an operand's category
/// (its Σ type-index for `NotCompoundRefined`), never a sem.
fn combine_nominal_mod(left: &CategoryPayload, right: &CategoryPayload) -> Option<SemRecipe> {
    for rule in refine_rules() {
        let mut binds = CatSubst::new();
        if match_cat(&rule.left_pat, &left.cat, &mut binds)
            && match_cat(&rule.right_pat, &right.cat, &mut binds)
            && rule.guards.iter().all(|g| g.holds(&left.cat, &right.cat))
        {
            return Some(SemRecipe::Refine {
                builder: rule.build,
                binds,
            });
        }
    }
    None
}

/// The remaining grammar-specific binary rules: close-naming apposition and the
/// GQ-as-preposition-object raise. Tried last; their triggers (`cat_n`+`cat_np`, `fwd`+`fwd`-with-
/// raised-GQ) are disjoint from the earlier groups, so their demotion below the universal combinators
/// is order-preserving.
fn combine_other_grammar(left: &CategoryPayload, right: &CategoryPayload) -> Option<SemRecipe> {
    // Close naming apposition (D63 §5.3): a SORTAL common noun `cat_n(Sortal)` (left) + a proper NAME
    // `cat_np(NameClass, sg)` (right) → the definite individual of the sortal kind bearing that name
    // ("Project Achilles", "the enzyme WRN"). The name's own class need NOT be the sortal (coining:
    // "Achilles" the hero names a Project), so this is distinct from `appose_group`'s KIND-checked
    // group apposition — it is the SINGLETON, un-type-checked naming case. Gated on a genuine proper
    // name (`NameClass ≠ Entity`) so it does not fire on a pronoun / bare-kind `cat_np(Entity)` right.
    if let (Some([sortal, _snum]), Some([name_ty, _nnum])) =
        (is_ctor(&left.cat, "cat_n"), is_ctor(&right.cat, "cat_np"))
    {
        if matches!(name_ty, Exp::EigonClass(iri) if iri.as_str() != "urn:eigenius:lexicon:Entity")
        {
            return Some(SemRecipe::Name {
                sortal: sortal.clone(),
            });
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
        let prep_is_argmarker = is_ctor(pp_res, "cat_pp_arg").is_some() && obj_is_np;
        let prep_is_vpadjunct =
            obj_is_np && matches!(is_ctor(pp_res, "bwd"), Some([a, b]) if is_vp(a) && is_vp(b));
        let gq_is_raised_subject = is_ctor(gq_s, "cat_s").is_some()
            && matches!(is_ctor(gq_vp, "bwd"),
                Some([s, np]) if is_ctor(s, "cat_s").is_some() && is_ctor(np, "cat_np").is_some());
        let kind = if prep_is_ppmod {
            Some(PrepObj::PpMod)
        } else if prep_is_argmarker {
            Some(PrepObj::ArgMarker)
        } else if prep_is_vpadjunct {
            Some(PrepObj::VpAdjunct)
        } else {
            None
        };
        if let (Some(kind), true) = (kind, gq_is_raised_subject) {
            return Some(SemRecipe::GqPrepObj {
                cat: pp_res.clone(),
                kind,
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
        SemRecipe::Refine { builder, binds } => builder(&binds, left, right, layer),
        SemRecipe::GqPrepObj { cat, kind } => {
            let sem = match kind {
                PrepObj::PpMod => {
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
                }
                PrepObj::VpAdjunct => {
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
                }
                PrepObj::ArgMarker => {
                    // Argument-PP: the object entity itself — `Q(prep_sem)`, the raised GQ applied to the
                    // transparent marker (`to` = `λy. y`). "to genes" (Q = `λV. V(kind_of(Gene))`) →
                    // `kind_of(Gene)`; a subcategorizing verb `(S\NP)/cat_pp_arg` then binds it.
                    Exp::App(Box::new(right.sem().clone()), Box::new(left.sem().clone()))
                }
            };
            Item::from_parts(cat, sem, Combinator::Other, Cost::ZERO)
        }
        SemRecipe::Name { sortal } => {
            // `Σx:sortal. named(x, name)` — the coined named individual's kind (the name referent is
            // the naming token); `kind_of` realizes it as an Entity, so "Project Achilles" is a bare
            // proper-name NP exactly like an ordinary name.
            let restr = app2(
                "urn:eigenius:ontology:named",
                COMPOUND_X,
                right.sem().clone(),
            );
            let sigma = Exp::Sig(
                Patt::Var(COMPOUND_X.into()),
                Box::new(sortal),
                Box::new(restr),
            );
            let kind_of = Exp::EigonAxiom(
                crate::ontology::iri::Iri::parse("urn:eigenius:ontology:kind_of")
                    .expect("kind_of iri"),
            );
            let sem = Exp::App(Box::new(kind_of), Box::new(sigma));
            // `cat_np(Entity, num)` — reuse the sortal `cat_n`'s Cat decl + the proper name's number.
            let (decl, num) = match (left.cat(), right.cat()) {
                (Exp::InductiveCtor(d, _, _), Exp::InductiveCtor(_, _, rargs))
                    if rargs.len() == 2 =>
                {
                    (d.clone(), rargs[1].clone())
                }
                _ => unreachable!("Name recipe requires a cat_n left + cat_np right"),
            };
            let entity = Exp::EigonClass(
                crate::ontology::iri::Iri::parse("urn:eigenius:lexicon:Entity")
                    .expect("entity iri"),
            );
            let cat = Exp::InductiveCtor(decl, "cat_np".into(), vec![entity, num]);
            Item::from_parts(cat, sem, Combinator::Compound, Cost::ZERO)
        }
    }
}

// ── The datafied nominal-modification family (Phase 1) ───────────────────────
//
// The four rules the imperative `combine_nominal_mod`/`build_refine` used to inline, expressed as
// data: a structural `CatPat` trigger per operand, sem-blind category guards, and a sem-builder.
// `combine_nominal_mod` interprets this table; each `build` is one arm of the former `build_refine`,
// lifted to a named function. See `docs/notes/grammar-formalization-plan.md` (Phase 1 slice).

/// One datafied nominal-modification rule: its structural trigger ([`CatPat`] over each operand),
/// sem-blind category `guards`, and the sem-`build`er. Priority is table order.
struct RefineRule {
    /// Rule identity — for tracing and future on-chain naming; carried, not yet consumed.
    #[allow(dead_code)]
    name: &'static str,
    left_pat: CatPat,
    right_pat: CatPat,
    guards: &'static [Guard],
    build: RefineBuilder,
}

/// A **sem-blind** dispatch guard — a predicate over an operand's CATEGORY (never its sem: the
/// packed-forest soundness invariant, enforced by the type — [`Guard::holds`] receives only
/// categories). This is the predicate library the datafied rules draw from.
#[derive(Clone, Copy)]
enum Guard {
    /// The named operand must NOT be an already-compound-refined noun — the left-branching normal
    /// form (D63 §8.13): a compound may not be a compound HEAD again. Negation of
    /// [`is_compound_refined`], which inspects only the category's Σ type-index.
    NotCompoundRefined(Operand),
}

/// Which operand a [`Guard`] reads. The complete two-sided vocabulary; the current family's only
/// guard reads `Right`, but a guard naming the `Left` operand is equally well-formed.
#[derive(Clone, Copy)]
enum Operand {
    #[allow(dead_code)]
    Left,
    Right,
}

impl Operand {
    fn pick<'a>(&self, left: &'a Exp, right: &'a Exp) -> &'a Exp {
        match self {
            Operand::Left => left,
            Operand::Right => right,
        }
    }
}

impl Guard {
    fn holds(&self, left: &Exp, right: &Exp) -> bool {
        match self {
            Guard::NotCompoundRefined(op) => !is_compound_refined(op.pick(left, right)),
        }
    }
}

/// The rule table (built once). Priority = order, mirroring the former linear arm order: attributive
/// adjective, then the pre-nominal compounds (named / N-N), then the post-nominal PP. Triggers are
/// pairwise disjoint by `(left_ctor, right_ctor)`, so order is not outcome-critical — it is kept for
/// a faithful differential against the hand-written path.
fn refine_rules() -> &'static [RefineRule] {
    static RULES: LazyLock<Vec<RefineRule>> = LazyLock::new(|| {
        use CatPat::{Ctor, Var};
        let cat_n = |a, b| Ctor("cat_n", vec![a, b]);
        vec![
            // Attributive adjective (D63 §8.5 Slice 3b): `S[_,adj]\NP` (left) + `cat_n` (right). The
            // `adj` fin literal in the pattern IS the adj-clause test — no guard needed.
            RefineRule {
                name: "attrib",
                left_pat: Ctor(
                    "bwd",
                    vec![Ctor("cat_s", vec![Var("_"), Ctor("adj", vec![])]), Var("_")],
                ),
                right_pat: cat_n(Var("C"), Var("num")),
                guards: &[],
                build: refine_attrib,
            },
            // Named-entity compound `[cat_np] [cat_n]` (D63 §8.13). Left-branching NF: the head may
            // not itself be a compound result.
            RefineRule {
                name: "named_compound",
                left_pat: Ctor("cat_np", vec![Var("_"), Var("_")]),
                right_pat: cat_n(Var("C"), Var("num")),
                guards: &[Guard::NotCompoundRefined(Operand::Right)],
                build: refine_named_compound,
            },
            // N-N kind compound `[cat_n] [cat_n]` (D63 §8.13). Same left-branching guard.
            RefineRule {
                name: "kind_compound",
                left_pat: cat_n(Var("_"), Var("_")),
                right_pat: cat_n(Var("C"), Var("num")),
                guards: &[Guard::NotCompoundRefined(Operand::Right)],
                build: refine_kind_compound,
            },
            // PP-as-noun-modifier (post-nominal): `[cat_n(C)] [cat_pp]`. Here the head noun is the
            // LEFT, so `C`/`num` bind from the left pattern.
            RefineRule {
                name: "pp_mod",
                left_pat: cat_n(Var("C"), Var("num")),
                right_pat: Ctor("cat_pp", vec![]),
                guards: &[],
                build: refine_pp_mod,
            },
        ]
    });
    &RULES
}

/// Pull the head noun's `decl` (from `noun`'s category ctor) and the `C` / `num` metavariables the
/// trigger bound — the shared preamble of the refine builders.
fn noun_parts(noun: &Item, binds: &CatSubst) -> (Arc<crate::nbe::term::InductiveDecl>, Exp, Exp) {
    let decl = match noun.cat() {
        Exp::InductiveCtor(d, _, _) => d.clone(),
        _ => unreachable!("a refine rule matched a non-inductive noun category"),
    };
    let c = binds.get("C").expect("refine trigger binds C").clone();
    let num = binds.get("num").expect("refine trigger binds num").clone();
    (decl, c, num)
}

/// Attributive adjective (D63 §8.5 Slice 3b). If `C` is ALREADY a refined noun `Σx:Base. P(x)` (a
/// stacked adjective), CONJOIN over the SAME base: `Σx:Base. P(x) ∧ adj(x)` — a FLAT Σ. Else
/// `Σx:C. adj(x)`. The head noun is the right operand.
fn refine_attrib(binds: &CatSubst, left: &Item, right: &Item, layer: &Arc<Layer>) -> Item {
    let (decl, c, noun_num) = noun_parts(right, binds);
    let sigma = match &c {
        Exp::Sig(Patt::Var(bx), base, p_body)
            if super::super::category::resolve_inductive(layer, "urn:eigenius:logic:And")
                .is_some() =>
        {
            let and =
                super::super::category::resolve_inductive(layer, "urn:eigenius:logic:And").unwrap();
            let adj_at = Exp::App(Box::new(left.sem().clone()), Box::new(Exp::Var(bx.clone())));
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

/// Named-entity compound `[cat_np] [cat_n]` → `Σx:C. compound(x, ⟦left⟧)`. Head noun is the right.
fn refine_named_compound(binds: &CatSubst, left: &Item, right: &Item, _layer: &Arc<Layer>) -> Item {
    let (decl, c, noun_num) = noun_parts(right, binds);
    let restr = app2(
        "urn:eigenius:ontology:compound",
        COMPOUND_X,
        left.sem().clone(),
    );
    refined_noun(&decl, &c, &noun_num, restr)
}

/// N-N kind compound `[cat_n] [cat_n]` → `Σx:C. compound_kind(x, ⟦left⟧)`. Head noun is the right.
fn refine_kind_compound(binds: &CatSubst, left: &Item, right: &Item, _layer: &Arc<Layer>) -> Item {
    let (decl, c, noun_num) = noun_parts(right, binds);
    let restr = app2(
        "urn:eigenius:ontology:compound_kind",
        COMPOUND_X,
        left.sem().clone(),
    );
    refined_noun(&decl, &c, &noun_num, restr)
}

/// Post-nominal PP modifier `[cat_n(C)] [cat_pp]` → `Σx:C. ⟦right⟧(x)`. Head noun is the LEFT.
fn refine_pp_mod(binds: &CatSubst, left: &Item, right: &Item, _layer: &Arc<Layer>) -> Item {
    let (decl, c, noun_num) = noun_parts(left, binds);
    let restr = Exp::App(
        Box::new(right.sem().clone()),
        Box::new(Exp::Var(COMPOUND_X.into())),
    );
    refined_noun(&decl, &c, &noun_num, restr)
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

/// The **number** argument of a `cat_n(_, num)` category (`sg` / `pl` / `mass` / `num_any`), or
/// `None` if `cat` is not a common noun. The multiword-preference cut compares only this — a bare-class
/// leaf and a `Σ`-refined compound noun of the SAME number fill the identical combinatorial slot and
/// differ only in denotation, so the compound is the one to drop.
pub(crate) fn cat_n_number(cat: &Exp) -> Option<&Exp> {
    if let Some([_, num]) = is_ctor(cat, "cat_n") {
        Some(num)
    } else {
        None
    }
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

// NOTE — there is deliberately NO chart driver here.
//
// `parser.rs` owns the RULES (`apply` / `apply_core` / `apply_group`); the CKY drivers live in
// `lookup/chart_packed.rs` and `lookup/chart_unpacked.rs`. That split is forced, not stylistic: several
// rules resolve their CATEGORY out of the lexicon at parse time — the bare-plural/mass kind shift
// borrows the determiner's raised category (`entries_for("a")` / `("these")`), the object appositive
// borrows `a_obj`, and pied-piping looks up the fronted preposition. A driver that must consult the
// lexicon cannot live in a module that has none, so the drivers hang off `Parser`.
//
// A bare `cky_parse(tokens, layer)` that only applied `apply` used to live here. It could not parse
// coordination, relatives, bare plurals, type-raising, or any composed-cell shift, so it was a strict
// subset of the real driver and no production path used it — a fossil of the grammar from before the
// lexicon-dependent rules existed. It survived only as a test harness and now lives with the tests
// that use it (`kernel/tests/lexicon_validates.rs`), so the engine has exactly one driver family.

#[cfg(test)]
mod nominal_mod_tests {
    //! **0b-lite golden characterization of the nominal-modification family** (the differential
    //! oracle for the Phase 1 datafication, `docs/notes/grammar-formalization-plan.md`). Each test
    //! constructs the two operand [`Item`]s and drives the real CKY step [`apply`], pinning the exact
    //! result category, sem, and provenance. When [`combine_nominal_mod`] is replaced by a data-driven
    //! table, these must still pass byte-identically — that is what makes "formalization changed
    //! nothing" a checked claim. The stacked-adjective flat-Σ `And` path needs a layer that resolves
    //! `logic:And`, so it is covered by the full-page `--no-llm` sweep differential, not here.
    use super::*;
    use crate::nbe::term::list_decl;
    use crate::ontology::iri::Iri;

    fn ct(name: &str, args: Vec<Exp>) -> Exp {
        Exp::InductiveCtor(list_decl(), name.into(), args)
    }
    fn cls(s: &str) -> Exp {
        Exp::EigonClass(Iri::parse(s).unwrap())
    }
    fn ax(s: &str) -> Exp {
        Exp::EigonAxiom(Iri::parse(s).unwrap())
    }
    fn mk_item(cat: Exp, sem: Exp) -> Item {
        Item::from_parts(cat, sem, Combinator::Other, Cost::ZERO)
    }
    fn layer() -> Arc<Layer> {
        Arc::new(
            crate::layer::LayerBuilder::new("combinators-nominal-mod-test", None)
                .build(crate::layer::LayerStorage::in_memory()),
        )
    }
    fn sg() -> Exp {
        ct("sg", vec![])
    }
    fn n(c: Exp) -> Exp {
        ct("cat_n", vec![c, sg()])
    }
    fn np(c: Exp) -> Exp {
        ct("cat_np", vec![c, sg()])
    }
    /// `Σx:base. restr` over the compound-family bound variable [`COMPOUND_X`].
    fn sigma_cmp(base: Exp, restr: Exp) -> Exp {
        Exp::Sig(
            Patt::Var(COMPOUND_X.into()),
            Box::new(base),
            Box::new(restr),
        )
    }
    /// `R(x, m)` — the 6-mod restrictor App-spine (mirrors [`app2`]).
    fn app2_x(axiom: &str, m: Exp) -> Exp {
        Exp::App(
            Box::new(Exp::App(
                Box::new(ax(axiom)),
                Box::new(Exp::Var(COMPOUND_X.into())),
            )),
            Box::new(m),
        )
    }

    #[test]
    fn kind_compound_is_sigma_over_compound_kind_axiom() {
        // `[cat_n] [cat_n]` → `Σx:C. compound_kind(x, ⟦left⟧)`.
        let modifier = ax("urn:eigenius:lexicon:mmr");
        let head = cls("urn:eigenius:lexicon:Gene");
        let l = mk_item(n(cls("urn:eigenius:lexicon:Mmr")), modifier.clone());
        let r = mk_item(n(head.clone()), head.clone());
        let got = apply(&l, &r, &layer()).expect("[cat_n][cat_n] → kind compound");
        let expected = sigma_cmp(
            head,
            app2_x("urn:eigenius:ontology:compound_kind", modifier),
        );
        assert_eq!(got.cat(), &n(expected.clone()), "result is cat_n(Σ, sg)");
        assert_eq!(got.sem(), &expected, "sem is the Σ (CN-as-types)");
        assert_eq!(got.prov(), Combinator::Compound);
        assert_eq!(
            got.cost().sense_rank,
            COMPOUND_STEP_PENALTY,
            "apply adds the compound-step penalty"
        );
    }

    #[test]
    fn named_compound_is_sigma_over_compound_axiom() {
        // `[cat_np] [cat_n]` → `Σx:C. compound(x, ⟦left⟧)`.
        let name_ref = ax("urn:eigenius:lexicon:achilles");
        let head = cls("urn:eigenius:lexicon:Project");
        let l = mk_item(np(cls("urn:eigenius:lexicon:Achilles")), name_ref.clone());
        let r = mk_item(n(head.clone()), head.clone());
        let got = apply(&l, &r, &layer()).expect("[cat_np][cat_n] → named compound");
        let expected = sigma_cmp(head, app2_x("urn:eigenius:ontology:compound", name_ref));
        assert_eq!(got.cat(), &n(expected.clone()));
        assert_eq!(got.sem(), &expected);
        assert_eq!(got.prov(), Combinator::Compound);
    }

    #[test]
    fn pp_mod_applies_the_pp_sem_to_the_bound_witness() {
        // `[cat_n] [cat_pp]` → `Σx:C. ⟦right⟧(x)` (un-reduced; the felicity gate normalizes later).
        let head = cls("urn:eigenius:lexicon:Protein");
        let pp_sem = Exp::Lam(
            Patt::Var("y".into()),
            Box::new(Exp::App(
                Box::new(ax("urn:eigenius:lexicon:in_nucleus")),
                Box::new(Exp::Var("y".into())),
            )),
        );
        let l = mk_item(n(head.clone()), head.clone());
        let r = mk_item(ct("cat_pp", vec![]), pp_sem.clone());
        let got = apply(&l, &r, &layer()).expect("[cat_n][cat_pp] → pp modifier");
        let expected = sigma_cmp(
            head,
            Exp::App(Box::new(pp_sem), Box::new(Exp::Var(COMPOUND_X.into()))),
        );
        assert_eq!(got.cat(), &n(expected.clone()));
        assert_eq!(got.sem(), &expected);
        assert_eq!(got.prov(), Combinator::Compound);
    }

    #[test]
    fn attrib_on_a_plain_noun_is_a_simple_sigma() {
        // `[S[adj]\NP] [cat_n]` with a NON-refined base → `Σx:C. ⟦adj⟧(x)`, bound var `__refine_x`.
        let adj_sem = Exp::Lam(
            Patt::Var("z".into()),
            Box::new(Exp::App(
                Box::new(ax("urn:eigenius:lexicon:large")),
                Box::new(Exp::Var("z".into())),
            )),
        );
        let adj_cat = ct(
            "bwd",
            vec![
                ct("cat_s", vec![ct("dcl", vec![]), ct("adj", vec![])]),
                np(cls("urn:eigenius:lexicon:Entity")),
            ],
        );
        let head = cls("urn:eigenius:lexicon:Cell");
        let l = mk_item(adj_cat, adj_sem.clone());
        let r = mk_item(n(head.clone()), head.clone());
        let got = apply(&l, &r, &layer()).expect("[S[adj]\\NP][cat_n] → attributive");
        let x = "__refine_x";
        let expected = Exp::Sig(
            Patt::Var(x.into()),
            Box::new(head),
            Box::new(Exp::App(Box::new(adj_sem), Box::new(Exp::Var(x.into())))),
        );
        assert_eq!(got.cat(), &n(expected.clone()));
        assert_eq!(got.sem(), &expected);
        assert_eq!(got.prov(), Combinator::Compound);
    }

    #[test]
    fn compound_refined_head_blocks_further_compounding() {
        // The left-branching NF cut (D63 §8.13): a head that is ALREADY a compound
        // (`Σx:Gene. compound_kind(x, m)`) may not be a compound HEAD again. No rule fires → `None`.
        let refined_head = sigma_cmp(
            cls("urn:eigenius:lexicon:Gene"),
            app2_x(
                "urn:eigenius:ontology:compound_kind",
                ax("urn:eigenius:lexicon:mmr"),
            ),
        );
        let l = mk_item(
            n(cls("urn:eigenius:lexicon:Repair")),
            ax("urn:eigenius:lexicon:repair"),
        );
        let r = mk_item(n(refined_head), cls("urn:eigenius:lexicon:Gene"));
        assert!(
            apply(&l, &r, &layer()).is_none(),
            "a compound-refined head is not a compound head a second time"
        );
    }
}
