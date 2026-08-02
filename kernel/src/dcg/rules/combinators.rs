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
    cat_subsumes, feat_meets, is_ctor, match_cat, slash_parts, subst_cat, unify_cat, CatPat,
    CatSubst,
};
use super::super::item::{CategoryPayload, Combinator, Cost, Item, COMPOUND_STEP_PENALTY};
use super::super::pretty::pretty_term;
use super::super::rules::constructions::{distribute, distribute_object};

/// Combine two adjacent constituents (the CKY step). Combinability is decided **sem-blind** by
/// [`combinable`] (it receives only [`CategoryPayload`]s — the compile-time form of the packed-forest
/// soundness invariant: the packing signature `(cat_shape, ENF-prov)` is sound because the DECISION
/// is a function of the categories alone), then [`build`] materialises the item from the full items
/// (its sem, and — for the dependent nominal rules whose result TYPE embeds modifier meaning — its
/// category). The result's [`Item::cost`] is the **sum** of the two inputs' costs plus the
/// [`COMPOUND_STEP_PENALTY`] for a nominal-modification step (D63 §8.7).
pub fn apply(
    left: &Item,
    right: &Item,
    layer: &Arc<Layer>,
    rctx: super::RightContext,
) -> Option<Item> {
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
    apply_group(left, right, layer, rctx).map(|it| {
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
    /// A **datafied grammar rule** matched (Phase 1–2): a `combine_*` group matched a [`CatRule`] and
    /// carries its sem-`builder` plus the metavariable `binds` the pattern captured. `build` invokes
    /// the builder, the only place a child sem is read. Covers the nominal-modification family
    /// (attributive-Σ / N-N / named / PP compounds), close-naming apposition, and the
    /// GQ-as-preposition-object raise — everything that used to be a bespoke recipe variant + build
    /// arm is now this one variant plus a table row.
    Rule { builder: SemBuild, binds: CatSubst },
}

/// Application direction for [`SemRecipe::Apply`] (also fixes the provenance: forward ⇒ `ForwardApp`,
/// backward ⇒ `BackwardApp`).
#[derive(Clone, Copy)]
enum AppOrder {
    Fwd,
    Bwd,
}

/// A nominal-modification **sem-builder**: assembles the refined-noun [`Item`] from the metavariable
/// `binds` the trigger captured and the two child sems — the sem half of a datafied [`CatRule`],
/// and (with [`build`]) the only place a child sem is read. One per rule (`refine_attrib`, …); they
/// are the extracted arms of the former `build_refine`.
type SemBuild = fn(&CatSubst, &Item, &Item, &Arc<Layer>) -> Item;

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
    combine_universal(left, right, layer)
        .or_else(|| combine_nominal_mod(left, right))
        .or_else(|| combine_other_grammar(left, right))
}

/// A **universal combinator** — one of the category calculus's own composition rules (as opposed to
/// the grammar-specific structural rules). Unlike a [`CatRule`], its dispatch carries a *combination
/// constraint*: after destructuring a functor it `unify_cat`s an argument slot against the other
/// operand (subsumption + feature-meet), which may FAIL — so the combination is part of the decision,
/// not just the build. The SET of combinators is data; the calculus itself (destructure + `unify_cat`
/// + `subst_cat`) is [`CombKind::combine`]. See `docs/notes/grammar-formalization-plan.md` (Phase 2b).
struct CombRule {
    /// Rule identity — tracing / future on-chain naming; carried, not yet consumed.
    #[allow(dead_code)]
    name: &'static str,
    kind: CombKind,
    /// Eisner normal-form guards on the LEFT operand's provenance.
    prov_guards: &'static [ProvGuard],
}

/// The shape of a universal combinator.
enum CombKind {
    /// Application `slash(A, B) · other → A[σ]`: the `functor` operand is `slash(A, B)`; `unify_cat`
    /// its argument slot `B` against the other operand's whole category; the result is `A[σ]`. The sem
    /// applies functor to argument (order from `functor`).
    Apply {
        functor: Operand,
        slash: &'static str,
    },
    /// Composition `A/B ∘ B'/C → A/C`: both operands are `slash` functors; `unify_cat` the left's arg
    /// `B` against the right's result `B'`; the result is `slash(A, C)[σ]`.
    Compose { slash: &'static str },
    /// Dependent (polymorphic) application (the determiner, D63 §8.2 item 3): a
    /// `cat_forall(det_num, λT. body)` consuming `cat_n(T, noun_num)` by INSTANTIATING `T` (not slot
    /// unification) — feature-gated by `feat_meets`, with a Fst-projecting refined-noun branch.
    DepApply,
}

/// An Eisner normal-form provenance guard on the left operand (D63 §8.2 item 4).
// The shared `LeftNot` prefix is the point — every guard forbids a provenance on the LEFT operand.
#[allow(clippy::enum_variant_names)]
enum ProvGuard {
    /// The left operand is not itself a composition output (may not be a primary `>`/`>B` functor).
    LeftNotComposed,
    /// The left operand is not type-raised (a raised functor may only compose, not forward-apply).
    /// Covers the bare-kind raise ([`Combinator::KindRaised`]) for the same reason it covers
    /// [`Combinator::TypeRaised`]: with a plain `cat_np` available, a raised SUBJECT forward-applying to
    /// the VP would re-derive the reading plain backward application already gives.
    LeftNotRaised,
    /// The left operand — the backward-application ARGUMENT — is not a modal / do-support output
    /// ([`Combinator::Modal`]). On `backward_app` this blocks a VP-adjunct PP from attaching ABOVE a
    /// modal ("`can arise` `from LS`"), where it would escape the modal's `Possible(…)` scope; subject
    /// application is untouched (there the argument is the subject NP, not the modal VP).
    LeftNotModal,
    /// The right operand — the backward-application FUNCTOR — is not a raised BARE KIND
    /// ([`Combinator::KindRaised`]). The Eisner mirror of [`Self::LeftNotRaised`]: a raised category may
    /// only compose / coordinate, never do what plain application already does. A bare kind now has a
    /// plain `cat_np` (core-en's `bnp`), so the raised copy backward-applying to a transitive verb
    /// re-derives the identical reading — `HeLa affects genes` would close twice with the same sem. A
    /// DETERMINER quantifier (`a gene`) is unaffected: it has no plain-NP form, so its raise is the only
    /// derivation and carries a different provenance.
    RightNotKindRaised,
    /// The left operand — the primary COMPOSITION functor — is not a seed-time oblique participial
    /// lift ([`Combinator::ObliqueParticipial`]). The Eisner mirror of [`Self::LeftNotRaised`]: that
    /// guard bars a raised category from *applying* because composition is its only non-redundant use;
    /// this one bars a lifted post-nominal modifier from *composing* because application is. Both
    /// routes reach the same `cat_pp` over the same span with the same sem.
    LeftNotObliqueParticipial,
}

impl ProvGuard {
    fn holds(&self, left_prov: Combinator, right_prov: Combinator) -> bool {
        match self {
            ProvGuard::LeftNotComposed => !matches!(
                left_prov,
                Combinator::ForwardComp | Combinator::CrossedComp | Combinator::BackwardComp
            ),
            ProvGuard::LeftNotRaised => {
                !matches!(left_prov, Combinator::TypeRaised | Combinator::KindRaised)
            }
            ProvGuard::LeftNotModal => left_prov != Combinator::Modal,
            ProvGuard::RightNotKindRaised => right_prov != Combinator::KindRaised,
            ProvGuard::LeftNotObliqueParticipial => left_prov != Combinator::ObliqueParticipial,
        }
    }
}

impl CombKind {
    /// Try to combine the two operands under this combinator, **sem-blind** (categories + provenance
    /// only). Returns the [`SemRecipe`] `build` will materialise, or `None` if the combination
    /// constraint (`unify_cat` / `feat_meets`) fails.
    fn combine(
        &self,
        left: &CategoryPayload,
        right: &CategoryPayload,
        layer: &Arc<Layer>,
    ) -> Option<SemRecipe> {
        match self {
            CombKind::Apply { functor, slash } => {
                let (fun, arg) = match functor {
                    Operand::Left => (left, right),
                    Operand::Right => (right, left),
                };
                // APPLICATION is keyed to the lattice ROOT `⋆` (Baldridge (192): `X/⋆Y Y ⇒ X`), and
                // every modality is `⋆` or a subtype of it — so every slash applies, whatever its
                // mode, and there is no licensing test here. This is the one rule class modes never
                // restrict; it is precisely why `⋆` means "application ONLY".
                let (_mode, res, slot) = slash_parts(&fun.cat, slash)?;
                let subst = unify_cat(slot, &arg.cat, layer)?;
                Some(SemRecipe::Apply {
                    cat: subst_cat(res, &subst),
                    order: match functor {
                        Operand::Left => AppOrder::Fwd,
                        Operand::Right => AppOrder::Bwd,
                    },
                })
            }
            CombKind::Compose { slash } => {
                let (lm, l_res, l_arg) = slash_parts(&left.cat, slash)?;
                let (rm, r_res, r_arg) = slash_parts(&right.cat, slash)?;
                // HARMONIC composition, keyed to `⋄` (Baldridge (194): `X/⋄Y Y/⋄Z ⇒B X/⋄Z`). BOTH
                // slashes must license it, so a governed complement marked `⋆` cannot be composed
                // away from its head — the categorial statement of what `ProvGuard` approximates.
                if !harmonic_licenses(lm) || !harmonic_licenses(rm) {
                    return None;
                }
                let subst = unify_cat(l_arg, r_res, layer)?;
                let Exp::InductiveCtor(decl, _, _) = &left.cat else {
                    return None;
                };
                Some(SemRecipe::FwdComp {
                    // The result carries the rule's keyed modality, as `⇒B X/⋄Z` writes it.
                    cat: Exp::InductiveCtor(
                        decl.clone(),
                        (*slash).into(),
                        vec![
                            lm.clone(),
                            subst_cat(l_res, &subst),
                            subst_cat(r_arg, &subst),
                        ],
                    ),
                })
            }
            CombKind::DepApply => {
                let [det_num, Exp::Lam(Patt::Var(tvar), body)] = is_ctor(&left.cat, "cat_forall")?
                else {
                    return None;
                };
                let [t, noun_num] = is_ctor(&right.cat, "cat_n")? else {
                    return None;
                };
                if !feat_meets(det_num, noun_num) {
                    return None;
                }
                // Refined noun (attributive Σ): bind `T := C` (the component type) + Fst-project (in
                // `build`), only when `tvar` occurs in `body` (a GQ's predicate slot) — else the
                // predicate-nominal falls through to the plain application below.
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
                Some(SemRecipe::Apply {
                    cat: subst_cat(body, &subst),
                    order: AppOrder::Fwd,
                })
            }
        }
    }
}

/// The universal-combinator table (built once). Priority = order, mirroring the former arm order:
/// dependent determiner (its `cat_forall` trigger is disjoint from the rest, so its first position is
/// not load-bearing), then forward application, backward application, forward (harmonic) composition.
/// Eisner NF is enforced per rule by `prov_guards`.
fn comb_rules() -> &'static [CombRule] {
    static RULES: LazyLock<Vec<CombRule>> = LazyLock::new(|| {
        vec![
            CombRule {
                name: "dependent_determiner",
                kind: CombKind::DepApply,
                prov_guards: &[],
            },
            CombRule {
                name: "forward_app",
                kind: CombKind::Apply {
                    functor: Operand::Left,
                    slash: "fwd",
                },
                prov_guards: &[ProvGuard::LeftNotComposed, ProvGuard::LeftNotRaised],
            },
            CombRule {
                name: "backward_app",
                kind: CombKind::Apply {
                    functor: Operand::Right,
                    slash: "bwd",
                },
                // LeftNotModal: the ARGUMENT (left) may not be a modal/aux VP output — a VP-adjunct PP
                // must attach BELOW the modal, not above it (`Combinator::Modal`). Subject application
                // is unaffected (there the argument is the subject NP).
                prov_guards: &[ProvGuard::LeftNotModal, ProvGuard::RightNotKindRaised],
            },
            CombRule {
                name: "forward_comp",
                kind: CombKind::Compose { slash: "fwd" },
                prov_guards: &[
                    ProvGuard::LeftNotComposed,
                    ProvGuard::LeftNotObliqueParticipial,
                ],
            },
        ]
    });
    &RULES
}

/// The **universal CCG combinators** — the category calculus (application, composition, the dependent
/// determiner), now **data-driven** (Phase 2b). The interpreter over [`comb_rules`]: for each rule
/// whose Eisner provenance guards hold, try its combination; the first that succeeds wins. Replaces
/// the hand-written `combine_determiner` + `combine_universal` arms; the table order reproduces the
/// former linear order exactly.
fn combine_universal(
    left: &CategoryPayload,
    right: &CategoryPayload,
    layer: &Arc<Layer>,
) -> Option<SemRecipe> {
    for rule in comb_rules() {
        if rule
            .prov_guards
            .iter()
            .all(|g| g.holds(left.prov, right.prov))
        {
            if let Some(recipe) = rule.kind.combine(left, right, layer) {
                return Some(recipe);
            }
        }
    }
    None
}

/// The **nominal-modification family** (D63 §8.5/§8.13) — attributive adjective, named-entity and
/// N-N compounds, and post-nominal PP — now **data-driven** (Phase 1,
/// `docs/notes/grammar-formalization-plan.md`). Each rule is a [`CatRule`] (structural [`CatPat`]
/// triggers + sem-blind category guards + a sem-`builder`); this function is the interpreter: try the
/// rules in priority order, and on the first whose patterns match and guards hold, defer to its
/// builder. Sem-blind like all of [`combinable`] — a [`Guard`] reads only an operand's category
/// (its Σ type-index for `NotCompoundRefined`), never a sem.
fn combine_nominal_mod(left: &CategoryPayload, right: &CategoryPayload) -> Option<SemRecipe> {
    for rule in refine_rules() {
        let mut binds = CatSubst::new();
        if match_cat(&rule.left_pat, &left.cat, &mut binds)
            && match_cat(&rule.right_pat, &right.cat, &mut binds)
            && rule.guards.iter().all(|g| g.holds(&binds, left, right))
        {
            return Some(SemRecipe::Rule {
                builder: rule.build,
                binds,
            });
        }
    }
    None
}

/// The remaining grammar-specific binary rules — close-naming apposition and the
/// GQ-as-preposition-object raise — **data-driven** (Phase 2), same interpreter as
/// [`combine_nominal_mod`] over a separate table ([`other_grammar_rules`]). Tried last; the triggers
/// (`cat_n`+`cat_np`, `fwd`+`fwd`-with-raised-GQ) are disjoint from the earlier groups.
fn combine_other_grammar(left: &CategoryPayload, right: &CategoryPayload) -> Option<SemRecipe> {
    for rule in other_grammar_rules() {
        let mut binds = CatSubst::new();
        if match_cat(&rule.left_pat, &left.cat, &mut binds)
            && match_cat(&rule.right_pat, &right.cat, &mut binds)
            && rule.guards.iter().all(|g| g.holds(&binds, left, right))
        {
            return Some(SemRecipe::Rule {
                builder: rule.build,
                binds,
            });
        }
    }
    None
}

/// The **modal / do-support auxiliary** functor category `(S[dcl,fin]\NP)/(S[dcl,bse]\NP)` — a forward
/// functor from a BASE verbal VP to a FINITE one.
///
/// **No longer consulted by the grammar** (2026-07-27). Being scope-bearing is now DECLARED —
/// `lexicon:scope_bearing` on the entry, surfacing as [`Combinator::ScopeOperator`] on the leaf — and
/// [`build`] reads only that. Inferring it from the category could never be complete: sentential
/// negation is `fwd(VP[bse], VP[bse])` / `fwd(VP[adj], VP[adj])`, and the second is byte-identical to
/// the adverb adjective-modifier category, so no shape test can single it out.
///
/// Kept as a TEST-ONLY predicate because it still expresses a real completeness obligation: every
/// entry with this shape is an auxiliary and must carry the declaration.
/// `dcg::lexicon::scope_bearing_tests` asserts exactly that, so a modal added to
/// `closed-class.esl` without the flag fails CI instead of silently losing its `Modal` tag.
#[cfg(test)]
pub(crate) fn is_modal_functor(cat: &Exp) -> bool {
    /// `S[_,<fin>]\NP` — a verbal VP `bwd(cat_s(_, <fin>), NP)` with the given finiteness feature.
    fn is_vp_with_fin(cat: &Exp, fin: &str) -> bool {
        let Some((_m, s, _np)) = slash_parts(cat, "bwd") else {
            return false;
        };
        let Some([_mood, f]) = is_ctor(s, "cat_s") else {
            return false;
        };
        matches!(f, Exp::InductiveCtor(_, n, _) if n == fin)
    }
    let Some((_m, res, arg)) = slash_parts(cat, "fwd") else {
        return false;
    };
    is_vp_with_fin(res, "fin") && is_vp_with_fin(arg, "bse")
}

/// Materialise the [`Item`] for a [`SemRecipe`] from the two children's full items — the ONLY place a
/// child sem is read. For the dependent nominal rules ([`SemRecipe::Rule`]) the result CATEGORY
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
                AppOrder::Fwd => {
                    // A SCOPE-BEARING operator (sentential negation, a modal, declarative
                    // do-support) tags its output `Modal`, so a VP-adjunct cannot attach ABOVE it
                    // (`ProvGuard::LeftNotModal`) and escape the operator's scope.
                    //
                    // The property is DECLARED — `lexicon:scope_bearing` on the entry, arriving here
                    // as the functor's leaf provenance ([`Combinator::ScopeOperator`]) — not inferred
                    // from the category. Inference could never be complete: negation is
                    // `fwd(VP[bse], VP[bse])` / `fwd(VP[adj], VP[adj])`, the second byte-identical to
                    // the adverb adjective-modifier category. Completeness of the declaration is
                    // pinned in CI by `dcg::lexicon::scope_bearing_tests`, which fails if an entry
                    // with the auxiliary category shape lacks the flag.
                    let prov = if left.prov() == Combinator::ScopeOperator {
                        Combinator::Modal
                    } else {
                        Combinator::ForwardApp
                    };
                    (
                        Exp::App(Box::new(left.sem().clone()), Box::new(right.sem().clone())),
                        prov,
                    )
                }
                AppOrder::Bwd => (
                    Exp::App(Box::new(right.sem().clone()), Box::new(left.sem().clone())),
                    Combinator::BackwardApp,
                ),
            };
            // `EIGENIUS_TRACE_ARITY=1` — name the FUNCTOR whose sem arity exceeds its category's.
            //
            // `Item::from_parts` can flag that the RESULT is under-applied (a `Prop` category over a
            // sem that evaluates to a closure) but not who caused it: it has no view of the children.
            // Here both are in scope, so the functor's category is printable — and since every
            // downstream application inherits the defect, only the origin is worth reporting.
            if std::env::var("EIGENIUS_TRACE_ARITY").is_ok() {
                let (f, a) = match order {
                    AppOrder::Fwd => (left, right),
                    AppOrder::Bwd => (right, left),
                };
                // Compare the FUNCTOR's own arities, not the result's. Flagging the result only ever
                // names the application that tripped over the defect; flagging the functor names the
                // item that CARRIES it, and the first such item in a derivation is the origin.
                if let (Some(ca), Some(sa)) = (cat_arity(f.cat()), sem_arity(f.sem())) {
                    if sa > ca {
                        eprintln!(
                            "  !! ARITY functor={} cat_arity={ca} sem_arity={sa} fprov={:?} arg={}",
                            cat_brief(f.cat()),
                            f.prov(),
                            cat_brief(a.cat())
                        );
                    }
                }
            }
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
        SemRecipe::Rule { builder, binds } => builder(&binds, left, right, layer),
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
struct CatRule {
    /// Rule identity — for tracing and future on-chain naming; carried, not yet consumed.
    #[allow(dead_code)]
    name: &'static str,
    left_pat: CatPat,
    right_pat: CatPat,
    guards: &'static [Guard],
    build: SemBuild,
}

/// A **sem-blind** dispatch guard — a predicate over an operand's CATEGORY and/or PROVENANCE (never
/// its sem: the packed-forest soundness invariant — [`Guard::holds`] receives a [`CategoryPayload`],
/// which carries only `cat` + `prov`, both part of the packing `Sig`). The predicate library the
/// datafied rules draw from.
#[derive(Clone, Copy)]
enum Guard {
    /// The named operand must NOT be an already-compound-refined noun — the left-branching normal
    /// form (D63 §8.13): a compound may not be a compound HEAD again. Negation of
    /// [`is_compound_refined`], which inspects only the category's Σ type-index.
    NotCompoundRefined(Operand),
    /// The named operand must NOT be an **adjective-refined** noun — the adjective-outside-compound
    /// normal form (D63 nominal-modification §3.3): a compound forms only over pure nouns, so an
    /// adjective always attaches OUTSIDE the compound core. Negation of [`is_adjective_refined`];
    /// category-only, like [`Self::NotCompoundRefined`].
    NotAdjectiveRefined(Operand),
    /// The named operand must NOT be a **bare-kind** NP (`Combinator::KindRaised`). Following core-en's
    /// `bnp` rule the bare-kind shift yields a PLAIN `cat_np` rather than the quantifier-style raise,
    /// so it can fill any argument slot — including a non-final one, which type-raising cannot reach.
    /// This guard keeps the invariant the raise used to enforce structurally: a bare kind stays
    /// **argument-only** and must not feed the compound rule, which would build a spurious
    /// `compound(x, kind_of(C))` duplicating the `compound_kind` classifier (D63 §7.5). Reads
    /// provenance, not the category — the same distinction `mod_lifts` already makes for `KindRaised`.
    NotKindRaised(Operand),
    /// The bound type-index metavar must be a **genuine proper-name class** — a concrete `EigonClass`
    /// other than the `Entity` top (D63 §5.3). Keeps close-naming apposition off a pronoun /
    /// bare-kind `cat_np(Entity)` right. Reads a category metavar, never a sem.
    ProperName(&'static str),
    /// The named operand must NOT be a **derived individual** (`Combinator::DerivedIndividual`) — a
    /// designator is a naming TOKEN, never a description, so `named(x, the(Σy. named(y, d)).1)` ("the
    /// gene named the gene named MSH2") is refused. Like [`Self::NotKindRaised`] this reads provenance:
    /// a designation's category is a plain concretely-typed `cat_np`, so it satisfies
    /// [`Self::ProperName`] and the type cannot tell it from a name.
    NotDerivedIndividual(Operand),
    /// The bound NUMBER metavar must not be plural — **classifier/designator cardinality agreement**
    /// (D63 §5.3). A close-apposition classifier takes as many designators as its number says: a
    /// SINGULAR classifier takes exactly one, which is what this binary rule supplies, while a PLURAL
    /// one needs a designator LIST and must therefore go through
    /// [`super::constructions::appose_group`] over a `cat_group`. "the gene MSH2" ✓, "the genes BRCA1
    /// and MSH2" ✓ (the group route), "*the genes MSH2" ✗.
    ///
    /// This is the constraint that kills **classifier capture**: in "the MMR genes MSH2, MSH6, PMS2 or
    /// MLH1" the string also admits `[[the MMR genes MSH2], MSH6, PMS2, MLH1]` — the classifier bound
    /// to the FIRST designator and that NP coordinated with the remaining three, so only one of four
    /// genes is classified. It was 24 of the reference page's germline-unit skeletons. The capture needs
    /// a plural classifier to take a single designator, so refusing that refuses the bracketing, and the
    /// group route (which classifies all four) is what remains. Purely morphosyntactic — no right
    /// context, no type lattice. `num_any` is underspecified and passes.
    NotPlural(&'static str),
    /// The named operand must NOT be a **PP-postmodified** noun
    /// ([`super::constructions::is_pp_refined`]) — the adjacency argument that already governs the
    /// group path: a designator sits immediately after the nominal head, so a PP postmodifier cannot
    /// intervene ("the gene MSH2 in humans", never "*the gene in humans MSH2"). Category-only.
    ///
    /// [`super::constructions::appose_group`] has enforced this since 2026-07-26; the singular rule did
    /// not, so "[Germline mutations in the MMR gene] [MSH2]" still captured the designator — traced live
    /// as `cat_n(Σ__cmp_x:n07425011. … prep_in …, sg)` reaching this rule.
    NotPpRefined(Operand),
}

/// Which operand a rule reads — a [`Guard`]'s target, or the functor side of a [`CombKind::Apply`].
#[derive(Clone, Copy)]
enum Operand {
    Left,
    Right,
}

impl Operand {
    fn pick<'a>(
        &self,
        left: &'a CategoryPayload,
        right: &'a CategoryPayload,
    ) -> &'a CategoryPayload {
        match self {
            Operand::Left => left,
            Operand::Right => right,
        }
    }
}

/// Audit-only projections of a [`Guard`], used by `grammar_rule_guard_matrix`.
#[cfg(test)]
impl Guard {
    /// A short label for the rule/guard AUDIT ([`grammar_rule_guard_matrix`]) — the audit exists
    /// because the recurring defect shape in this grammar is a constraint applied in ONE rule and not
    /// its sibling (PP-adjacency was in `appose_group` but not `kind_compound`; `NotKindRaised` on the
    /// `name` rule but not the group path; number refinement on nouns but not verbs). A matrix makes
    /// that visible instead of waiting for a sentence to expose it.
    fn label(&self) -> &'static str {
        match self {
            Guard::NotCompoundRefined(_) => "NotCompoundRefined",
            Guard::NotAdjectiveRefined(_) => "NotAdjectiveRefined",
            Guard::NotKindRaised(_) => "NotKindRaised",
            Guard::ProperName(_) => "ProperName",
            Guard::NotDerivedIndividual(_) => "NotDerivedIndividual",
            Guard::NotPlural(_) => "NotPlural",
            Guard::NotPpRefined(_) => "NotPpRefined",
        }
    }

    /// Which operand the guard constrains, for the audit ("L", "R", or "-" for a metavar guard).
    fn side(&self) -> &'static str {
        match self {
            Guard::NotCompoundRefined(o)
            | Guard::NotAdjectiveRefined(o)
            | Guard::NotKindRaised(o)
            | Guard::NotDerivedIndividual(o)
            | Guard::NotPpRefined(o) => match o {
                Operand::Left => "L",
                Operand::Right => "R",
            },
            Guard::ProperName(_) | Guard::NotPlural(_) => "-",
        }
    }
}

impl Guard {
    fn holds(&self, binds: &CatSubst, left: &CategoryPayload, right: &CategoryPayload) -> bool {
        match self {
            Guard::NotCompoundRefined(op) => !is_compound_refined(&op.pick(left, right).cat),
            Guard::NotAdjectiveRefined(op) => !is_adjective_refined(&op.pick(left, right).cat),
            Guard::NotKindRaised(op) => op.pick(left, right).prov != Combinator::KindRaised,
            Guard::NotDerivedIndividual(op) => {
                op.pick(left, right).prov != Combinator::DerivedIndividual
            }
            Guard::ProperName(meta) => matches!(
                binds.get(*meta),
                Some(Exp::EigonClass(iri)) if iri.as_str() != "urn:eigenius:lexicon:Entity"
            ),
            Guard::NotPlural(meta) => !matches!(
                binds.get(*meta),
                Some(Exp::InductiveCtor(_, n, _)) if n == "pl"
            ),
            // Checks `cat_np` as well as `cat_n`: the adjacency argument is about the SURFACE (a PP
            // postmodifier cannot sit between a modifier and its head), so it does not care whether the
            // modifier is a bare noun or a full NP. Inspecting only `cat_n` made this silently inert on
            // `named_compound`, whose left operand is a `cat_np` — found by `grammar_rule_guard_matrix`.
            Guard::NotPpRefined(op) => {
                let cat = &op.pick(left, right).cat;
                match is_ctor(cat, "cat_n").or_else(|| is_ctor(cat, "cat_np")) {
                    Some([ty, _num]) => !super::constructions::is_pp_refined(ty),
                    _ => true,
                }
            }
        }
    }
}

/// The rule table (built once). Priority = order, mirroring the former linear arm order: attributive
/// adjective, then the pre-nominal compounds (named / N-N), then the post-nominal PP. Triggers are
/// pairwise disjoint by `(left_ctor, right_ctor)`, so order is not outcome-critical — it is kept for
/// a faithful differential against the hand-written path.
fn refine_rules() -> &'static [CatRule] {
    static RULES: LazyLock<Vec<CatRule>> = LazyLock::new(|| {
        use CatPat::{Ctor, Var};
        let cat_n = |a, b| Ctor("cat_n", vec![a, b]);
        vec![
            // Pre-nominal modifier application (D63 coordinated-modifier category,
            // `docs/notes/d63-coordinated-modifier-category.md`): a lifted `cat_mod` restrictor + head
            // `cat_n` → refined noun. This is the SOLE attributive-modifier path — an adjective reaches
            // it by lifting to `cat_mod` (`mod_lifts`, which also excludes `KindRaised`); a coordinated
            // attributive modifier reaches it as the `Or`-folded `cat_mod` that `coordinate_mod` +
            // `CoordComplete` produce (D63 §6). No `NotCompoundRefined` — an adjective may refine a
            // compound head ("primary cancer cell").
            CatRule {
                name: "mod_apply",
                left_pat: Ctor("cat_mod", Vec::new()),
                right_pat: cat_n(Var("C"), Var("num")),
                guards: &[],
                build: refine_mod_apply,
            },
            // Named-entity compound `[cat_np] [cat_n]` (D63 §8.13). Left-branching NF: the head may
            // not itself be a compound result. Adjective-outside NF (§3.3): nor an adjective-refined
            // one — an adjective on the head attaches OUTSIDE the compound, not before it.
            CatRule {
                name: "named_compound",
                left_pat: Ctor("cat_np", vec![Var("_"), Var("_")]),
                right_pat: cat_n(Var("C"), Var("num")),
                guards: &[
                    Guard::NotCompoundRefined(Operand::Right),
                    Guard::NotAdjectiveRefined(Operand::Right),
                    Guard::NotKindRaised(Operand::Left),
                    // Same PP-adjacency constraint its sibling `kind_compound` carries (32bfd21): a
                    // PP-postmodified modifier would need its PP to sit between it and the head.
                    Guard::NotPpRefined(Operand::Left),
                ],
                build: refine_named_compound,
            },
            // N-N kind compound `[cat_n] [cat_n]` (D63 §8.13). Left-branching guard on the head; the
            // adjective-outside NF (§3.3) additionally forbids an adjective-refined operand on EITHER
            // side, so a gradable adjective cannot float to an inner compound slot (`[specific repair]
            // proteins`) — it attaches outside the fully-formed compound core (`specific [repair
            // proteins]`). A genuine adjective-inside compound is a lexical unit (§4), not rebuilt here.
            CatRule {
                name: "kind_compound",
                left_pat: cat_n(Var("_"), Var("_")),
                right_pat: cat_n(Var("C"), Var("num")),
                guards: &[
                    Guard::NotCompoundRefined(Operand::Right),
                    Guard::NotAdjectiveRefined(Operand::Right),
                    Guard::NotAdjectiveRefined(Operand::Left),
                    // A PP-postmodified noun is not a PRE-nominal modifier: English puts the PP after
                    // the head it modifies, so "[mutations in the MMR] genes" would need the PP to sit
                    // between modifier and head. Same adjacency argument as
                    // [`super::constructions::appose_group`]'s, which has carried it since 2026-07-26;
                    // this rule did not, and the pile it built was the reference page's germline unit's
                    // last invalid family — `[[germline mutations in the MMR] genes] MSH2`, 24 of its
                    // 25 skeletons.
                    Guard::NotPpRefined(Operand::Left),
                ],
                build: refine_kind_compound,
            },
            // PP-as-noun-modifier (post-nominal): `[cat_n(C)] [cat_pp]`. Here the head noun is the
            // LEFT, so `C`/`num` bind from the left pattern.
            CatRule {
                name: "pp_mod",
                left_pat: cat_n(Var("C"), Var("num")),
                right_pat: Ctor("cat_pp", vec![]),
                guards: &[
                    // Left-branching NF for POSTnominal modification, the mirror of the guard
                    // `named_compound`/`kind_compound` carry on their head: a PP attaches to the bare
                    // nominal, and a pre-nominal compound modifier goes outside it. Both bracketings of
                    // "the MMR genes in humans" feed `refine_conjoin`, which flattens restrictors onto
                    // ONE Σ, so they are the same claim reached two ways — collapsing to one derivation
                    // removes spurious ambiguity, it does not choose a meaning. Found by
                    // `grammar_rule_guard_matrix` (this rule had no guards at all).
                    Guard::NotCompoundRefined(Operand::Left),
                ],
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

/// The **pre-nominal modifier** category `cat_mod` — a restrictor awaiting a head noun (D63,
/// `docs/notes/d63-coordinated-modifier-category.md`). Nullary: it carries **no** head type `C` (that
/// dependency lives in its SEM, an un-type-checked restrictor `λx. P(x)`), which is what lets modifiers
/// coordinate without introducing an abstract `C`. Rust-only ctor (categories are name-keyed on the
/// shared `list_decl`), so no ontology edit / reseed.
pub(crate) fn cat_mod_cat() -> Exp {
    Exp::InductiveCtor(crate::nbe::term::list_decl(), "cat_mod".into(), Vec::new())
}

/// **Modifier lift** (M1: adjectives): re-categorise a modifier-eligible item into a standalone
/// [`cat_mod_cat`], so a modifier can meet another modifier (coordinate) before it meets the head
/// noun. A genuine attributive adjective `S[adj]\NP` lifts with its sem UNCHANGED (already `λx. adj(x)`);
/// a `KindRaised` bare-noun predicative form does **not** lift — it stays out of modifier position,
/// reproducing the old `refine_attrib` `NotProv(KindRaised)` guard as a lift-time gate rather than an
/// application-time one.
///
/// A PREDICATE NOMINAL needs no guard here, and that is the point of `lexicon:pred`. It used to wear
/// `cat_s(dcl, adj)` and arrive by ordinary application from `a_pred`, so neither of the two things
/// this function may inspect — the category and the provenance — could tell it from a real
/// attributive adjective, and it lifted (d44dfda: 7 `invalid` ledger rows, including `is_a(WRN,
/// cancer)` from splitting the compound "MSI cancers"). Now it wears `cat_s(dcl, pred)` and
/// [`is_adjective_cat`] simply does not match it. Fires at leaf seeding (`seed.rs`) and on composed cells (the `ModLift` unary
/// shift), mirroring `bare_nominal_shifts`.
pub(crate) fn mod_lifts(it: &Item) -> Vec<Item> {
    if super::super::category::is_adjective_cat(it.cat()) && it.prov() != Combinator::KindRaised {
        return vec![Item::from_parts(
            cat_mod_cat(),
            it.sem().clone(),
            Combinator::Other,
            it.cost(),
        )];
    }
    Vec::new()
}

/// Pre-nominal attributive PAST PARTICIPLE lift — SEPARATE from [`mod_lifts`] so seeding can GATE it.
/// A transitive `(S[dcl,pss]\NP)/NP` → a reduced-passive modifier `cat_mod(λx. ∃a. TV(x, a))`
/// ("predicted deficiency" = a deficiency x that was predicted by some a). English forms this for ANY
/// transitive participle, but WordNet lists only some as adjectives ("increased"/"reduced" yes,
/// "predicted" no), so it is a RULE, not lexical coverage.
///
/// Two guards against double-seeding, layered: (1) [`super::super::parse::seed`] applies this at LEAVES
/// only when the surface has NO adjective entry — where WordNet already supplies the adjective the
/// redundant reading is never generated; (2) the compose-time `ModLift` shift applies it ungated (no
/// surface to check), where the cost PENALTY ([`PARTICIPIAL_MOD_PENALTY`]) keeps it below a real
/// adjective. Additive: the participle stays available for its passive-VP uses ("is predicted").
pub(crate) fn participial_lifts(it: &Item) -> Vec<Item> {
    match participial_restrictor(it.cat(), it.sem()) {
        Some(restr) => vec![Item::from_parts(
            cat_mod_cat(),
            restr,
            Combinator::Other,
            it.cost()
                .saturating_add(Cost::from_sense_rank(PARTICIPIAL_MOD_PENALTY)),
        )],
        None => Vec::new(),
    }
}

/// POST-nominal OBLIQUE participial lift — the counterpart of [`participial_lifts`], and the seed-time
/// half of the reduced-relative story (see [`super::constructions::reduced_relative`] for the full
/// diagnosis). An oblique participle STILL AWAITING its PP argument,
/// `(S[dcl,pss]\NP)/cat_pp_arg(P)`, lifts to a post-nominal modifier still awaiting the same argument,
/// `cat_pp/cat_pp_arg(P)` — so once the PP arrives ("compared **to MSS cell lines**") forward
/// application lands on `cat_pp`, which `pp_mod` attaches to the noun.
///
/// **The lift is on the FUNCTOR, and that is the whole point.** After saturation the oblique and the
/// transitive participle are indistinguishable — both are `S[dcl,pss]\NP` over an object-first
/// `Entity → Entity → Prop`, same category and same `ForwardApp` provenance (chart dump, 2026-07-27).
/// Before saturation they are not: the oblique's remaining argument is `cat_pp_arg`, the transitive's
/// is `cat_np`. That argument shape IS the agent test. `PpOblique` is 2-place with no distinct agent,
/// so its subject slot is exactly the one the modified noun should fill (`compared to X` →
/// `λsubj. compare(X, subj)`); `Transitive` puts the AGENT there (`induced DNA` →
/// `λsubj. induce(DNA, subj)`), which would be a reduced SUBJECT relative — ungrammatical in English
/// ("*the man ate the food" for "the man that ate"). Discriminating here is what lets
/// `reduced_relative` refuse the saturated `pss` case outright.
///
/// Witnessed at the leaf before it was written — `EIGENIUS_DUMP_CELL=16..16` on the `compared to MSS
/// cell lines` unit shows BOTH `fwd(bwd(cat_s(dcl,pss), NP), cat_pp_arg(prep_any))` and
/// `fwd(bwd(cat_s(dcl,pss), NP), cat_np(Entity, num_any))`, so the discriminator is available exactly
/// where this fires.
///
/// Seed-time, not a `UnaryShift`: both chart drivers run the shift table inside `for len in 2..=n`, so
/// a shift can never see a single-token cell — and `compared` is one. Category-only, cost unchanged,
/// matching the `reduced_relative` shift this route replaces.
pub(crate) fn oblique_participial_lifts(it: &Item) -> Vec<Item> {
    match oblique_participial_cat(it.cat()) {
        Some(cat) => vec![Item::from_parts(
            cat,
            it.sem().clone(),
            Combinator::ObliqueParticipial,
            it.cost(),
        )],
        None => Vec::new(),
    }
}

/// `(S[dcl,pss]\NP)/cat_pp_arg(P)` → `cat_pp/cat_pp_arg(P)`, or `None` for any other category.
/// The argument is carried through UNCHANGED, so the governed preposition (`prep_any` for WordNet's
/// preposition-agnostic PP frames) still has to be matched by whatever fills it.
fn oblique_participial_cat(cat: &Exp) -> Option<Exp> {
    use super::super::category::{is_ctor, slash_parts};
    let (mode, vp, arg) = slash_parts(cat, "fwd")?;
    is_ctor(arg, "cat_pp_arg")?;
    let (_vm, s, subj) = slash_parts(vp, "bwd")?;
    is_ctor(subj, "cat_np")?;
    let [mood, voice] = is_ctor(s, "cat_s")? else {
        return None;
    };
    if !matches!(mood, Exp::InductiveCtor(_, n, _) if n == "dcl") {
        return None;
    }
    if !matches!(voice, Exp::InductiveCtor(_, n, _) if n == "pss") {
        return None;
    }
    let Exp::InductiveCtor(decl, _, _) = cat else {
        return None;
    };
    Some(Exp::InductiveCtor(
        decl.clone(),
        "fwd".into(),
        vec![
            // The derived modifier INHERITS the source slash's modality — deriving a category
            // must not silently relax what rules may consume it.
            mode.clone(),
            Exp::InductiveCtor(decl.clone(), "cat_pp".into(), Vec::new()),
            arg.clone(),
        ],
    ))
}

/// Extra cost on a rule-derived participial modifier, so a real lexical adjective (e.g. WordNet's
/// `increased`/`reduced`) outranks it and the eventive reduced-passive reading only surfaces where no
/// adjective exists. > [`COMPOUND_STEP_PENALTY`] (8), a clear deprioritisation.
const PARTICIPIAL_MOD_PENALTY: u32 = 12;

/// The reduced-passive restrictor `λx. ∃a:Entity. TV(x, a)` of a transitive past participle
/// `(S[dcl,pss]\NP)/NP` (sem `TV`), or `None` for any other category. The `∃` is the impredicative
/// encoding used by `closed-class.esl`'s `passive_sem` (`∀C:Prop. (∀a. TV(x,a) → C) → C`), so the
/// modifier reading and the finite short passive denote identically.
fn participial_restrictor(cat: &Exp, tv: &Exp) -> Option<Exp> {
    use super::super::category::{is_ctor, slash_parts};
    let (_m, vp, obj) = slash_parts(cat, "fwd")?;
    is_ctor(obj, "cat_np")?;
    let (_vm, s, subj) = slash_parts(vp, "bwd")?;
    is_ctor(subj, "cat_np")?;
    let [_typ, voice] = is_ctor(s, "cat_s")? else {
        return None;
    };
    if !matches!(voice, Exp::InductiveCtor(_, n, _) if n == "pss") {
        return None;
    }
    let entity =
        Exp::EigonClass(crate::ontology::iri::Iri::parse("urn:eigenius:lexicon:Entity").ok()?);
    let (x, a, c) = ("__part_x", "__part_a", "__part_C");
    // TV(x, a) — object-first: the patient x (the modified noun) then the agent a.
    let tv_xa = Exp::App(
        Box::new(Exp::App(Box::new(tv.clone()), Box::new(Exp::Var(x.into())))),
        Box::new(Exp::Var(a.into())),
    );
    // ∀a:Entity. TV(x,a) → C
    let inner = Exp::Pi(
        Patt::Var(a.into()),
        Box::new(entity),
        Box::new(Exp::Arrow(Box::new(tv_xa), Box::new(Exp::Var(c.into())))),
    );
    // ∀C:Prop. (∀a. …) → C   [= ∃a. TV(x,a)]
    let exists = Exp::Pi(
        Patt::Var(c.into()),
        Box::new(Exp::Sort(0)),
        Box::new(Exp::Arrow(Box::new(inner), Box::new(Exp::Var(c.into())))),
    );
    Some(Exp::Lam(Patt::Var(x.into()), Box::new(exists)))
}

/// Apply a `cat_mod` modifier to a head noun: `cat_mod(restr) + cat_n(C, num) →
/// cat_n(Σx:C. And(P, restr(x)), num)`, via `refine_conjoin` over the CONCRETE `C` (so the restrictor
/// type-checks at `x:C` directly — no abstract `C`). The restrictor is `restr(x)` un-reduced (`App`),
/// and the stacked-modifier flatten is `refine_conjoin`'s job — so for an adjective this reproduces the
/// former `refine_attrib` byte-for-byte.
fn refine_mod_apply(binds: &CatSubst, left: &Item, right: &Item, layer: &Arc<Layer>) -> Item {
    let (decl, c, noun_num) = noun_parts(right, binds);
    refine_conjoin(&decl, &c, &noun_num, layer, |x| {
        Exp::App(Box::new(left.sem().clone()), Box::new(Exp::Var(x.into())))
    })
}

/// Named-entity compound `[cat_np] [cat_n]` → `Σx:C. compound(x, ⟦left⟧)`. Head noun is the right.
fn refine_named_compound(binds: &CatSubst, left: &Item, right: &Item, layer: &Arc<Layer>) -> Item {
    let (decl, c, noun_num) = noun_parts(right, binds);
    refine_conjoin(&decl, &c, &noun_num, layer, |x| {
        app2("urn:eigenius:ontology:compound", x, left.sem().clone())
    })
}

/// N-N kind compound `[cat_n] [cat_n]` → `Σx:C. compound_kind(x, ⟦left⟧)`. Head noun is the right.
fn refine_kind_compound(binds: &CatSubst, left: &Item, right: &Item, layer: &Arc<Layer>) -> Item {
    let (decl, c, noun_num) = noun_parts(right, binds);
    refine_conjoin(&decl, &c, &noun_num, layer, |x| {
        app2("urn:eigenius:ontology:compound_kind", x, left.sem().clone())
    })
}

/// Post-nominal PP modifier `[cat_n(C)] [cat_pp]` → `Σx:C. ⟦right⟧(x)`. Head noun is the LEFT.
fn refine_pp_mod(binds: &CatSubst, left: &Item, right: &Item, layer: &Arc<Layer>) -> Item {
    let (decl, c, noun_num) = noun_parts(left, binds);
    refine_conjoin(&decl, &c, &noun_num, layer, |x| {
        Exp::App(Box::new(right.sem().clone()), Box::new(Exp::Var(x.into())))
    })
}

// ── The datafied "other grammar" binary rules (Phase 2) ──────────────────────
//
// Close-naming apposition and the GQ-as-preposition-object raise, expressed in the same table as the
// nominal-modification family — `CatPat` triggers + guards + sem-builders. `combine_other_grammar`
// interprets this table; each builder is one arm of the former `build` match. See
// `docs/notes/grammar-formalization-plan.md` (Phase 2).

// ─────────────────────────── multimodal slash licensing (Baldridge 2002 ch. 5) ───────────────────
//
// A combinatory rule is KEYED to a modality and consumes a slash bearing that modality **or any
// SUBTYPE of it** ("a modality has all the powers of its supertypes", §5.2). The lattice is
// `lexicon:Mode`, declared in `lexicon-ontology.esl` from Figure 5.1 (p. 102):
//
//                      m_app (⋆)                    root — application only
//          ┌───────────────┼───────────────┐
//     m_cross_left     m_harm       m_cross_right
//         (◁×)          (⋄)             (×▷)
//          └───────┬────┴──────┬─────────┘
//            m_perm_left   m_perm_right
//                (◁)           (▷)
//                 └──────┬──────┘
//                     m_all (·)                     bottom — all rules
//
// APPLICATION is keyed to the ROOT `m_app`, so every slash applies — which is why there is no
// `application_licenses`: it would be constantly true.

/// The modality name a slash carries, or `None` if `cat` is not a slash.
fn slash_mode_name(mode: &Exp) -> Option<&str> {
    match mode {
        Exp::InductiveCtor(_, n, _) => Some(n.as_str()),
        _ => None,
    }
}

/// Whether a slash bearing `mode` may be consumed by the HARMONIC composition rules — keyed to
/// `⋄` (194), so it admits `m_harm` and its subtypes `m_perm_left`/`m_perm_right`/`m_all`, and
/// refuses `m_app` (application only) and the two crossed modes.
fn harmonic_licenses(mode: &Exp) -> bool {
    matches!(
        slash_mode_name(mode),
        Some("m_harm" | "m_perm_left" | "m_perm_right" | "m_all")
    )
}

/// Whether a slash bearing `mode` may be consumed by the CROSSED composition rules. Baldridge
/// states (200) over the bare `×` CLASS ("we employ the modalities ◁× and ×▷ in defining the
/// crossed composition rules"), not over one directional node — `×` is a rule-class shorthand with
/// no point in Figure 5.1, which is exactly why OpenCCG's mode table has eight entries where the
/// lattice has seven nodes. So the class is both crossed nodes plus their subtypes.
fn crossed_licenses(mode: &Exp) -> bool {
    matches!(
        slash_mode_name(mode),
        Some("m_cross_left" | "m_cross_right" | "m_perm_left" | "m_perm_right" | "m_all")
    )
}

/// An anonymous category-pattern wildcard (`?_`).
fn wild() -> CatPat {
    CatPat::Var("_")
}
/// `cat_np(?_, ?_)` — any noun phrase (shape-only).
fn any_np_pat() -> CatPat {
    CatPat::Ctor("cat_np", vec![wild(), wild()])
}
/// `cat_s(?_, ?_)` — any sentence (shape-only).
fn any_s_pat() -> CatPat {
    CatPat::Ctor("cat_s", vec![wild(), wild()])
}
/// A forward-slash pattern `A/ₘB`. The slash MODALITY is wildcarded: a `CatPat` is a structural
/// SHAPE test, and modality restricts which rules may fire, not what a category looks like. Rule
/// licensing by mode is enforced where rules fire ([`mode_licenses`]), never by pattern shape.
/// Going through this constructor is also what keeps the mode slot from being forgotten — a
/// hand-written `Ctor("fwd", vec![a, b])` would silently never match (D63 multimodal slashes).
fn fwd_pat(res: CatPat, arg: CatPat) -> CatPat {
    CatPat::Ctor("fwd", vec![wild(), res, arg])
}
/// A backward-slash pattern `A\ₘB` — mode wildcarded, as [`fwd_pat`].
fn bwd_pat(res: CatPat, arg: CatPat) -> CatPat {
    CatPat::Ctor("bwd", vec![wild(), res, arg])
}
/// A VP `S\NP` — `bwd(m, cat_s, cat_np)`.
fn vp_pat() -> CatPat {
    bwd_pat(any_s_pat(), any_np_pat())
}
/// A type-raised subject GQ `S/(S\NP)` — the right operand every GQ-prep rule consumes.
fn raised_gq_pat() -> CatPat {
    fwd_pat(any_s_pat(), vp_pat())
}

/// The agentive-passive `by`'s functor RESULT — `(S[pass]\NP) \ ((S[pss]\NP)/NP)`: a passive patient-VP
/// awaiting the unsaturated active participle on its left. Distinct from every `pp_res` above (it is a
/// `bwd` whose left is a VP and right is a `fwd`), so `gq_prep_passive_agent` stays trigger-disjoint.
fn passive_agent_res_pat() -> CatPat {
    bwd_pat(vp_pat(), fwd_pat(vp_pat(), any_np_pat()))
}

/// The "other grammar" rule table (built once). Priority = order: close-naming apposition first, then
/// the three GQ-as-prep-object kinds (distinguished by the preposition functor's result `pp_res`:
/// `cat_pp` / `cat_pp_arg` / `(S\NP)\(S\NP)` — disjoint ctors). Tried after the nominal-modification
/// family; all triggers are disjoint from the earlier groups.
fn other_grammar_rules() -> &'static [CatRule] {
    static RULES: LazyLock<Vec<CatRule>> = LazyLock::new(|| {
        use CatPat::{Ctor, Var};
        vec![
            // Close naming apposition (D63 §5.3): `cat_n(Sortal)` + a proper NAME `cat_np(≠Entity)`.
            CatRule {
                name: "name",
                left_pat: Ctor("cat_n", vec![Var("sortal"), Var("sortalnum")]),
                right_pat: Ctor("cat_np", vec![Var("namety"), wild()]),
                // `ProperName` alone is not enough: it only asks that the name's type index be a
                // CONCRETE class (≠ `Entity`), and a BARE KIND's plain `cat_np` (core-en `bnp`) is also
                // concretely typed — so "nucleotide repeat regions" was read as the sortal `nucleotide`
                // apposed to a *name* "repeat regions", i.e. "a nucleotide **named** a repeat region".
                // The type cannot tell a kind from a name; the PROVENANCE can.
                guards: &[
                    Guard::ProperName("namety"),
                    Guard::NotKindRaised(Operand::Right),
                    Guard::NotDerivedIndividual(Operand::Right),
                    Guard::NotPlural("sortalnum"),
                    Guard::NotPpRefined(Operand::Left),
                ],
                build: build_name,
            },
            // GQ-as-prep-object, PpMod: `[cat_pp/NP] [raised-GQ]` → a post-nominal `cat_pp` modifier.
            CatRule {
                name: "gq_prep_ppmod",
                left_pat: fwd_pat(Ctor("cat_pp", vec![]), any_np_pat()),
                right_pat: raised_gq_pat(),
                guards: &[],
                build: gq_prep_ppmod,
            },
            // GQ-as-prep-object, ArgMarker: `[cat_pp_arg/NP] [raised-GQ]` → an oblique argument marker.
            CatRule {
                name: "gq_prep_argmarker",
                left_pat: fwd_pat(Ctor("cat_pp_arg", vec![wild()]), any_np_pat()),
                right_pat: raised_gq_pat(),
                guards: &[],
                build: gq_prep_argmarker,
            },
            // GQ-as-prep-object, VpAdjunct: `[(S\NP)\(S\NP)/NP] [raised-GQ]` → a VP modifier.
            CatRule {
                name: "gq_prep_vpadjunct",
                left_pat: fwd_pat(bwd_pat(vp_pat(), vp_pat()), any_np_pat()),
                right_pat: raised_gq_pat(),
                guards: &[],
                build: gq_prep_vpadjunct,
            },
            // GQ-as-passive-agent: the agentive `by` (`fwd(passive-VP-result, NP_agent)`) takes a
            // type-raised GQ agent — `represented by [these data sets]`. Without it `by`'s forward slot
            // only takes a PLAIN `cat_np`, so a determined / pronoun / deep-compound agent (a raised GQ,
            // not a plain NP) has no passive-agent parse (the `#46` gap). Trigger-disjoint from the three
            // above (its `pp_res` is `bwd(VP, fwd(VP, NP))`, none of `cat_pp`/`cat_pp_arg`/`bwd(VP,VP)`).
            CatRule {
                name: "gq_prep_passive_agent",
                left_pat: fwd_pat(passive_agent_res_pat(), any_np_pat()),
                right_pat: raised_gq_pat(),
                guards: &[],
                build: gq_prep_passive_agent,
            },
        ]
    });
    &RULES
}

/// Close naming apposition (D63 §5.3) — the **classifier + designator** construction: a common-noun
/// CLASSIFIER followed by a proper NAME or identifier ("project Achilles", "project DRIVE", "gene
/// MSH2", "chromosome 7"). The classifier supplies the **type**; the designator supplies the
/// **identity**. The phrase denotes an INDIVIDUAL, and a uniquely-identifying one, so it is a definite
/// description: `the(Σx:sortal. named(x, ⟦right⟧)).1`, at category `cat_np(sortal, num)`.
///
/// **This replaced a kind-coercion (2026-07-25) and the change is the point.** It used to build
/// `kind_of(Σx:sortal. named(x, …))` at `cat_np(Entity, num)` — a KIND coerced to an entity, with the
/// classifier's class DISCARDED. Two consequences, both measured on the reference page:
///
/// - Every instance injected a `kind_of` wrapper, and wherever the phrase recurred in argument
///   position each occurrence could independently take this route or the glossary's minted `ni_*`
///   individual. That was the dominant ambiguity axis of the page's worst unit — "Project Achilles and
///   project DRIVE identified WRN as the top preferential dependency in MSI cell lines compared to MSS
///   cell lines." carried **204 skeletons (34% of the page)**, whose `kind_of` count ranged 2..9 across
///   ~3 argument positions × 2 coordinated projects.
/// - Typing the result `Entity` threw away exactly the information the construction exists to carry.
///   "project Achilles" is a *project*; the classifier is the type, not decoration.
///
/// NEITHER reference grammar covers this construction, which is why it needed designing rather than
/// mirroring (method note §3): core-en's `Name` family is a bare `np` with default sem and all its
/// apposition machinery is loose/comma-delimited (`RelPro-Appos`, `prep.appos`, the appositive comma);
/// CCGbank makes a name sequence an ordinary `N/N … N` modifier chain with NO semantics, resolved by a
/// supertagger. A straight CCGbank mirror would also put the head on the *name* ("Ms. Waite"), which
/// is wrong here — the referent of "project Achilles" is the project.
///
/// Safe to key on the category alone because the construction is **head-INITIAL** (classifier then
/// designator) while English premodification is head-final: "MSI cell lines" is modifier+head and can
/// never match this trigger, so no over-application risk of the kind a blanket "prefer the proper-name
/// reading" preference would carry.
///
/// **The number is the CLASSIFIER's** (2026-07-26). It used to be the designator's (`rargs[1]`), and
/// that inverted agreement exactly: a designator is a NAME, and a name has no grammatical number to
/// contribute — every UMLS named individual seeds `cat_np(T028, sg)` — so the whole phrase came out
/// `sg` however plural its classifier was, and the parser then accepted the ungrammatical string and
/// rejected the grammatical one:
///
/// | | before | after |
/// | --- | --- | --- |
/// | `The genes MSH2 affect cells.` | 0 | ✓ |
/// | `The genes MSH2 affects cells.` (*) | 2 | 0 |
/// | `The cell lines HeLa affects genes.` (*) | 15 | 0 |
///
/// Head-initial means the classifier is the head, so it carries both the type and the number; the
/// designator contributes identity only. The classifier's `num` is concrete by the time this fires —
/// [`super::super::parse::seed`] refines a common noun's `num_any` to `sg`/`pl` from the surface
/// morphology at seeding.
///
/// **The plural rows of that table no longer apply, and `The genes MSH2 affect cells.` is back to 0**
/// (2026-07-26) — by [`Guard::NotPlural`], not by the number bug. Cardinality agreement: a singular
/// classifier takes exactly one designator (this rule), a plural one takes a designator LIST and must
/// go through [`super::constructions::appose_group`] over a `cat_group`. `*The genes MSH2` is
/// ungrammatical either way, so the row was witnessing agreement with a string that should not parse;
/// what it actually established — that the number comes from the classifier — is unaffected and is now
/// pinned on a `num_any` classifier instead.
fn build_name(binds: &CatSubst, left: &Item, right: &Item, _layer: &Arc<Layer>) -> Item {
    let sortal = binds
        .get("sortal")
        .expect("name trigger binds sortal")
        .clone();
    let sigma = super::constructions::naming_refinement(&sortal, right.sem());
    // `cat_n(Σx:sortal. named(x, d), num)` — a REFINED COMMON NOUN, carrying the CLASSIFIER's class
    // and the classifier's number (this construction is head-initial, so the classifier heads both;
    // taking the number from the designator instead inverts agreement — see the table above). Cat and
    // sem are the same Σ, exactly as [`super::constructions::relativize`] and the named-compound rule
    // build theirs, and the `Compound` provenance already means "builds a refined noun `cat_n(Σ…)`".
    let (decl, num) = match left.cat() {
        Exp::InductiveCtor(d, _, largs) if largs.len() == 2 => (d.clone(), largs[1].clone()),
        _ => unreachable!("the name rule requires a cat_n left + cat_np right"),
    };
    let cat = Exp::InductiveCtor(decl, "cat_n".into(), vec![sigma.clone(), num]);
    Item::from_parts(cat, sigma, Combinator::Compound, Cost::ZERO)
}

/// The result category of a GQ-as-prep-object raise: the preposition functor's own result
/// (`pp_res` in `fwd(pp_res, cat_np)`), re-extracted from the left operand.
fn gq_pp_res(left: &Item) -> Exp {
    match slash_parts(left.cat(), "fwd") {
        Some((_m, res, _)) => res.clone(),
        _ => unreachable!("a gq-prep rule matched a non-fwd left"),
    }
}

/// GQ-as-prep-object, noun-modifier: `λx. Q(λy. (prep y) x)` — result category `cat_pp`.
fn gq_prep_ppmod(_binds: &CatSubst, left: &Item, right: &Item, _layer: &Arc<Layer>) -> Item {
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
    let sem = Exp::Lam(
        Patt::Var(x.into()),
        Box::new(Exp::App(Box::new(right.sem().clone()), Box::new(inner))),
    );
    Item::from_parts(gq_pp_res(left), sem, Combinator::Other, Cost::ZERO)
}

/// GQ-as-passive-agent: `by`'s sem is `λagent. λTV. λp. TV(p, agent)`; scope the GQ `Q` over the agent
/// slot — `λTV. λp. Q(λagent. by(agent)(TV)(p))` — and return `by`'s own result category. So
/// `represented by [these data sets]` closes: the raised-GQ agent is quantified in, exactly as the
/// other `gq_prep_*` rules quantify a preposition's object. Applies `by`'s sem opaquely (no assumption
/// about its body), so it stays correct if `by_agent_sem` changes.
fn gq_prep_passive_agent(
    _binds: &CatSubst,
    left: &Item,
    right: &Item,
    _layer: &Arc<Layer>,
) -> Item {
    let (agent, tv, p) = ("__agt_x", "__agt_TV", "__agt_p");
    // by(agent)(TV)(p) : Prop
    let by_applied = Exp::App(
        Box::new(Exp::App(
            Box::new(Exp::App(
                Box::new(left.sem().clone()),
                Box::new(Exp::Var(agent.into())),
            )),
            Box::new(Exp::Var(tv.into())),
        )),
        Box::new(Exp::Var(p.into())),
    );
    // Q(λagent. by(agent)(TV)(p)) : Prop
    let scoped = Exp::App(
        Box::new(right.sem().clone()),
        Box::new(Exp::Lam(Patt::Var(agent.into()), Box::new(by_applied))),
    );
    // λTV. λp. Q(…) — `by`'s passive-VP result sem.
    let sem = Exp::Lam(
        Patt::Var(tv.into()),
        Box::new(Exp::Lam(Patt::Var(p.into()), Box::new(scoped))),
    );
    Item::from_parts(gq_pp_res(left), sem, Combinator::Other, Cost::ZERO)
}

/// GQ-as-prep-object, VP-adjunct: `λV.λs. Q(λx. prep_sem(x)(V)(s))` — result `(S\NP)\(S\NP)`.
fn gq_prep_vpadjunct(_binds: &CatSubst, left: &Item, right: &Item, _layer: &Arc<Layer>) -> Item {
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
    let sem = Exp::Lam(
        Patt::Var(v.into()),
        Box::new(Exp::Lam(Patt::Var(s.into()), Box::new(scoped))),
    );
    Item::from_parts(gq_pp_res(left), sem, Combinator::Other, Cost::ZERO)
}

/// GQ-as-prep-object, argument marker: `Q(prep_sem)` — the raised GQ applied to the transparent
/// marker; result category `cat_pp_arg`.
fn gq_prep_argmarker(_binds: &CatSubst, left: &Item, right: &Item, _layer: &Arc<Layer>) -> Item {
    let sem = Exp::App(Box::new(right.sem().clone()), Box::new(left.sem().clone()));
    Item::from_parts(gq_pp_res(left), sem, Combinator::Other, Cost::ZERO)
}

/// Coordination/distributive rules — the packed-forest **carve-out** (Harper 1994 pitfall): these
/// DECIDE on the sem (`distribute`/`distribute_object` read the group's `cons/nil` list via
/// `group_members`), so unlike [`combinable`] they are not sem-blind and are never packed by
/// `(cat_shape, ENF-prov)`. Tried only after [`combinable`] returns `None` (the group categories
/// never match a sem-blind rule, so ordering is preserved).
fn apply_group(
    left: &Item,
    right: &Item,
    layer: &Arc<Layer>,
    rctx: super::RightContext,
) -> Option<Item> {
    // Distributive SUBJECT (D63 §8.4 Phase 6): a `cat_group` subject meeting a VP `S\NP` distributes.
    if let (Some([c, _conn, gnum]), Some((_m, result, slot))) = (
        is_ctor(left.cat(), "cat_group"),
        slash_parts(right.cat(), "bwd"),
    ) {
        let num_agrees =
            matches!(is_ctor(slot, "cat_np"), Some([_, snum]) if feat_meets(gnum, snum));
        if num_agrees && group_member_fits(slot, c, layer) {
            if let Some(sem) = distribute(left.cat(), left.sem(), right.sem(), layer, rctx) {
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
    if let (Some((_m, result, slot)), Some([c, ..])) = (
        slash_parts(left.cat(), "fwd"),
        is_ctor(right.cat(), "cat_group"),
    ) {
        if group_member_fits(slot, c, layer) {
            if let Some(sem) = distribute_object(right.cat(), right.sem(), left.sem(), layer, rctx)
            {
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
pub fn apply_core(
    left: &Item,
    right: &Item,
    layer: &Arc<Layer>,
    _rctx: super::RightContext,
) -> Vec<Item> {
    let mut out = Vec::new();
    // `ObliqueParticipial` joins the composition outputs here for the reason it is barred from
    // `forward_comp` in `comb_rules`: composing it re-derives what applying it already gives.
    let primary_blocked = |p: Combinator| {
        matches!(
            p,
            Combinator::ForwardComp
                | Combinator::CrossedComp
                | Combinator::BackwardComp
                | Combinator::ObliqueParticipial
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
    // The composed RESULT carries the rule's keyed modality, exactly as Baldridge writes it:
    // `X/⋄Y Y/⋄Z ⇒B X/⋄Z` (194), `X/×Y Y\×Z ⇒B X\×Z` (200a). It is NOT inherited from the inputs.
    let mk =
        |decl: &Arc<crate::nbe::term::InductiveDecl>, ctor: &str, mode: &Exp, a: Exp, b: Exp| {
            Exp::InductiveCtor(decl.clone(), ctor.into(), vec![mode.clone(), a, b])
        };

    // Forward family: left is the primary functor `A/B` (fwd); not itself a composition output.
    if !primary_blocked(left.prov()) {
        if let (Exp::InductiveCtor(decl, _, _), Some((lm, a, b))) =
            (left.cat(), slash_parts(left.cat(), "fwd"))
        {
            // >Bx (crossed): `A/B · B\C → A\C`. left.arg(B) unifies right.result(B).
            if let Some((rm, rr, rc)) = slash_parts(right.cat(), "bwd") {
                // BOTH slashes must license the crossed class — (200a) keys `X/×Y Y\×Z`.
                if crossed_licenses(lm) && crossed_licenses(rm) {
                    if let Some(subst) = unify_cat(b, rr, layer) {
                        out.push(Item::from_parts(
                            mk(decl, "bwd", lm, subst_cat(a, &subst), subst_cat(rc, &subst)),
                            compose_sem(left.sem(), right.sem()),
                            Combinator::CrossedComp,
                            Cost::ZERO,
                        ));
                    }
                }
            }
        }
    }
    // Backward family: right is the primary functor `X\Y` (bwd); not itself a composition output.
    if !primary_blocked(right.prov()) {
        if let (Exp::InductiveCtor(decl, _, _), Some((rm, x, y))) =
            (right.cat(), slash_parts(right.cat(), "bwd"))
        {
            // <B (harmonic): `Y\Z · X\Y → X\Z`. left=Y\Z (bwd), unify left.result(Y) ~ right.arg(Y).
            if let Some((lm, ly, lz)) = slash_parts(left.cat(), "bwd") {
                // BOTH slashes must license the harmonic class — (194b) keys `Y\⋄Z X\⋄Y`.
                if harmonic_licenses(lm) && harmonic_licenses(rm) {
                    if let Some(subst) = unify_cat(ly, y, layer) {
                        out.push(Item::from_parts(
                            mk(decl, "bwd", rm, subst_cat(x, &subst), subst_cat(lz, &subst)),
                            compose_sem(right.sem(), left.sem()),
                            Combinator::BackwardComp,
                            Cost::ZERO,
                        ));
                    }
                }
            }
            // <Bx (crossed): `Y/Z · X\Y → X/Z`. left=Y/Z (fwd), unify left.result(Y) ~ right.arg(Y).
            if let Some((lm, ly, lz)) = slash_parts(left.cat(), "fwd") {
                if crossed_licenses(lm) && crossed_licenses(rm) {
                    if let Some(subst) = unify_cat(ly, y, layer) {
                        out.push(Item::from_parts(
                            mk(decl, "fwd", rm, subst_cat(x, &subst), subst_cat(lz, &subst)),
                            compose_sem(right.sem(), left.sem()),
                            Combinator::CrossedComp,
                            Cost::ZERO,
                        ));
                    }
                }
            }
        }
    }
    out.into_iter()
        .map(|it| it.at_cost(left.cost().saturating_add(right.cost())))
        .collect()
}

/// The bound variable of every 6-mod Σ-refinement (D63 §8.13).
pub(crate) const COMPOUND_X: &str = "__cmp_x";

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

/// **Canonicalize a conjunction of noun restrictors** (residual structural-multiplicity fix). Flatten
/// the associative `And`-tree in `p_body`, append `new_restr`, sort the conjuncts into a DETERMINISTIC
/// order, and rebuild a left-nested `And`. `logic:And` is commutative+associative, so this is
/// meaning-preserving; making the order CANONICAL rather than CKY-derivation order is the point: the
/// two attachment orders of one modifier set (`And(compound, prep_of)` vs `And(prep_of, compound)`)
/// then emit the **byte-identical** Σ, so they pack into one forest node and `subsume_duplicates`
/// collapses the otherwise-spurious duplicate readings. Sort key is [`restrictor_key`]: `pretty_term`
/// of the conjunct's β-NORMAL form. Stability alone is not sufficient and keying on the raw term was
/// measured wrong — one restrictor has several un-reduced forms, which sort to different places and
/// then reduce to the same thing at readback; see [`restrictor_key`]. Depends on the bound variable
/// being the SAME across paths (all refined Σ use [`COMPOUND_X`]) — else two alpha-variants would
/// sort identically yet stay distinct terms.
/// The sort key [`conjoin_canonical`] orders restrictors by: `pretty_term` of the conjunct's
/// **β-normal form**.
///
/// Keying on the raw term is not enough, and the reason is measured. A restrictor is stored
/// UN-REDUCED (`refine_mod_apply` / `refine_pp_mod` build `App(sem, x)`, and
/// [`is_adjective_refined`] depends on that shape), and ONE restrictor has more than one un-reduced
/// form — the PP object arrives either directly or through a type-raised GQ:
///
/// ```text
/// λ__pobj_x. λV. V(kind_of(C))(λ__pobj_y. λy. λx. prep_to(x, y)(__pobj_y, __pobj_x))(__cmp_x)
/// λy. λx. prep_to(x, y)(kind_of(C), __cmp_x)
/// ```
///
/// Both β-reduce to `prep_to(__cmp_x, kind_of(C))` — the same claim — but as strings the first sorts
/// BEFORE `λx. gt(…)(__cmp_x)` and the second AFTER it, so an adjective and a PP on one noun came out
/// in either order. Readback then reduces both, leaving two Σ that differ ONLY in conjunct order.
/// Determinism was never the problem; the key has to be invariant under β, because β-equivalent
/// conjuncts are the same restrictor and must sort to the same place.
///
/// Sorting can only REORDER conjuncts, never drop one, so a key that under-normalizes costs a missed
/// collapse and nothing else — which is why [`beta_normalize`] is allowed to give up (below) rather
/// than risk an unsound reduction.
fn restrictor_key(e: &Exp) -> String {
    pretty_term(&beta_normalize(e))
}

/// β-normalize for [`restrictor_key`]. Reduces `(λx. b) a` and drops `Ann`, recursing through the
/// term formers a restrictor is built from; any other variant is returned as-is.
///
/// **Deliberately partial.** A redex whose argument shares a variable name with a binder inside the
/// body is left UNREDUCED instead of being renamed — capture-avoiding freshening is not worth writing
/// for a sort key, and the fallback is free: an unreduced conjunct just keys by its raw form, exactly
/// the old behaviour. The shadow test over-approximates (every `Var` in the argument against every
/// binder in the body), so it errs toward not reducing.
fn beta_normalize(e: &Exp) -> Exp {
    fn vars(e: &Exp, out: &mut std::collections::BTreeSet<String>) {
        if let Exp::Var(n) = e {
            out.insert(n.to_string());
        }
        for c in subterms(e) {
            vars(c, out);
        }
    }
    fn binders(e: &Exp, out: &mut std::collections::BTreeSet<String>) {
        match e {
            Exp::Lam(Patt::Var(n), _)
            | Exp::Pi(Patt::Var(n), _, _)
            | Exp::Sig(Patt::Var(n), _, _) => {
                out.insert(n.to_string());
            }
            _ => {}
        }
        for c in subterms(e) {
            binders(c, out);
        }
    }
    fn subterms(e: &Exp) -> Vec<&Exp> {
        match e {
            Exp::App(f, a) => vec![f, a],
            Exp::Lam(_, b) | Exp::Con(_, b) => vec![b],
            Exp::Pi(_, a, b) | Exp::Sig(_, a, b) | Exp::Arrow(a, b) | Exp::Times(a, b) => {
                vec![a, b]
            }
            Exp::Pair(a, b) => vec![a, b],
            Exp::Fst(a) | Exp::Snd(a) | Exp::Ann(a, _) => vec![a],
            Exp::InductiveType(_, args) | Exp::InductiveCtor(_, _, args) => args.iter().collect(),
            _ => Vec::new(),
        }
    }
    /// Replace free `name` by `arg`; stops at a binder that re-binds `name` (shadowing).
    fn subst(body: &Exp, name: &str, arg: &Exp) -> Exp {
        let rebinds = |p: &Patt| matches!(p, Patt::Var(n) if n.as_str() == name);
        match body {
            Exp::Var(n) if n.as_str() == name => arg.clone(),
            Exp::Lam(p, b) => Exp::Lam(
                p.clone(),
                Box::new(if rebinds(p) {
                    (**b).clone()
                } else {
                    subst(b, name, arg)
                }),
            ),
            Exp::Pi(p, a, b) => Exp::Pi(
                p.clone(),
                Box::new(subst(a, name, arg)),
                Box::new(if rebinds(p) {
                    (**b).clone()
                } else {
                    subst(b, name, arg)
                }),
            ),
            Exp::Sig(p, a, b) => Exp::Sig(
                p.clone(),
                Box::new(subst(a, name, arg)),
                Box::new(if rebinds(p) {
                    (**b).clone()
                } else {
                    subst(b, name, arg)
                }),
            ),
            Exp::App(f, a) => {
                Exp::App(Box::new(subst(f, name, arg)), Box::new(subst(a, name, arg)))
            }
            Exp::Con(n, b) => Exp::Con(n.clone(), Box::new(subst(b, name, arg))),
            Exp::Arrow(a, b) => {
                Exp::Arrow(Box::new(subst(a, name, arg)), Box::new(subst(b, name, arg)))
            }
            Exp::Times(a, b) => {
                Exp::Times(Box::new(subst(a, name, arg)), Box::new(subst(b, name, arg)))
            }
            Exp::Pair(a, b) => {
                Exp::Pair(Box::new(subst(a, name, arg)), Box::new(subst(b, name, arg)))
            }
            Exp::Fst(a) => Exp::Fst(Box::new(subst(a, name, arg))),
            Exp::Snd(a) => Exp::Snd(Box::new(subst(a, name, arg))),
            Exp::Ann(a, t) => Exp::Ann(Box::new(subst(a, name, arg)), t.clone()),
            Exp::InductiveType(d, args) => Exp::InductiveType(
                d.clone(),
                args.iter().map(|x| subst(x, name, arg)).collect(),
            ),
            Exp::InductiveCtor(d, n, args) => Exp::InductiveCtor(
                d.clone(),
                n.clone(),
                args.iter().map(|x| subst(x, name, arg)).collect(),
            ),
            other => other.clone(),
        }
    }
    match e {
        Exp::Ann(inner, _) => beta_normalize(inner),
        Exp::App(f, a) => {
            let (f, a) = (beta_normalize(f), beta_normalize(a));
            if let Exp::Lam(Patt::Var(n), body) = &f {
                let (mut av, mut bb) = (Default::default(), Default::default());
                vars(&a, &mut av);
                binders(body, &mut bb);
                if av.is_disjoint(&bb) {
                    return beta_normalize(&subst(body, n.as_str(), &a));
                }
            }
            Exp::App(Box::new(f), Box::new(a))
        }
        Exp::Lam(p, b) => Exp::Lam(p.clone(), Box::new(beta_normalize(b))),
        Exp::Con(n, b) => Exp::Con(n.clone(), Box::new(beta_normalize(b))),
        Exp::Pi(p, a, b) => Exp::Pi(
            p.clone(),
            Box::new(beta_normalize(a)),
            Box::new(beta_normalize(b)),
        ),
        Exp::Sig(p, a, b) => Exp::Sig(
            p.clone(),
            Box::new(beta_normalize(a)),
            Box::new(beta_normalize(b)),
        ),
        Exp::Arrow(a, b) => Exp::Arrow(Box::new(beta_normalize(a)), Box::new(beta_normalize(b))),
        Exp::Times(a, b) => Exp::Times(Box::new(beta_normalize(a)), Box::new(beta_normalize(b))),
        Exp::Pair(a, b) => Exp::Pair(Box::new(beta_normalize(a)), Box::new(beta_normalize(b))),
        Exp::Fst(a) => Exp::Fst(Box::new(beta_normalize(a))),
        Exp::Snd(a) => Exp::Snd(Box::new(beta_normalize(a))),
        Exp::InductiveType(d, args) => {
            Exp::InductiveType(d.clone(), args.iter().map(beta_normalize).collect())
        }
        Exp::InductiveCtor(d, n, args) => Exp::InductiveCtor(
            d.clone(),
            n.clone(),
            args.iter().map(beta_normalize).collect(),
        ),
        other => other.clone(),
    }
}

fn conjoin_canonical(
    and: &Arc<crate::nbe::term::InductiveDecl>,
    p_body: &Exp,
    new_restr: Exp,
) -> Exp {
    fn flatten(and_iri: &str, e: &Exp, out: &mut Vec<Exp>) {
        if let Exp::InductiveType(decl, args) = e {
            if decl.iri.as_str() == and_iri && args.len() == 2 {
                flatten(and_iri, &args[0], out);
                flatten(and_iri, &args[1], out);
                return;
            }
        }
        out.push(e.clone());
    }
    let mut conjuncts = Vec::new();
    flatten(and.iri.as_str(), p_body, &mut conjuncts);
    conjuncts.push(new_restr);
    conjuncts.sort_by_cached_key(restrictor_key);
    let mut it = conjuncts.into_iter();
    let mut acc = it.next().expect("conjoin_canonical: at least one conjunct");
    for c in it {
        acc = Exp::InductiveType(and.clone(), vec![acc, c]);
    }
    acc
}

/// Build a refined common noun `cat_n(Σx:C. restr(x), num)` for a modifier rule (D63 §8.13),
/// reusing the head noun's `decl` and number. **FLATTENS** when `C` is already a refined noun
/// `Σx:Base. P(x)`: conjoin the new restrictor over the SAME base → `Σx:Base. And(P(x), restr(x))`,
/// else the simple `Σx:C. restr(x)`. `mk_restr` receives the bound variable NAME and returns the
/// restrictor `Prop`. This mirrors how `refine_attrib` handles a stacked adjective, applied to EVERY
/// modifier rule so that a chain of modifiers on a compound noun stays a **flat** Σ rather than a
/// nested Σ-over-Σ — the flat form is what the downstream bare-plural kind-raise / predication
/// consumes (the bare-mass `And` over-generation used to be the only other flattener; see
/// `experiments/parsing/near-encoded-bucket-analysis.md`). Sem is the Σ itself; provenance `Compound`.
fn refine_conjoin(
    decl: &Arc<crate::nbe::term::InductiveDecl>,
    c: &Exp,
    noun_num: &Exp,
    layer: &Arc<Layer>,
    mk_restr: impl Fn(&str) -> Exp,
) -> Item {
    let sigma = match c {
        Exp::Sig(Patt::Var(bx), base, p_body)
            if super::super::category::resolve_inductive(layer, "urn:eigenius:logic:And")
                .is_some() =>
        {
            let and =
                super::super::category::resolve_inductive(layer, "urn:eigenius:logic:And").unwrap();
            Exp::Sig(
                Patt::Var(bx.clone()),
                base.clone(),
                Box::new(conjoin_canonical(&and, p_body, mk_restr(bx))),
            )
        }
        _ => Exp::Sig(
            Patt::Var(COMPOUND_X.into()),
            Box::new(c.clone()),
            Box::new(mk_restr(COMPOUND_X)),
        ),
    };
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

/// Whether `cat` is a common noun refined by a **gradable adjective** — a restrictor conjunct whose
/// predicate is the degree comparison `measurements:gt` / `lt` (`gt(deg_X(x), std_X)`, the form the
/// importer emits for `specific` / `notable` / `independent` / `attractive`; `category.rs`
/// `ModifierClass::Gradable`). Flattens the restrictor over `logic:And` and inspects each conjunct's
/// spine head; POSITIVELY matches the degree axioms rather than "anything not a compound", so a
/// non-modifier restrictor a compound noun legitimately carries — the essive `is_a`, `named`, a `pp`
/// (`prep_*`) — is never mistaken for an adjective (that mis-classification exploded `MSI as a
/// biomarker` under widen-on-failure).
///
/// The **adjective-outside-compound normal form** (D63 nominal-modification §3.3): a compound rule
/// refuses a gradable-adjective-refined operand, so the canonical derivation of a modifier stack over
/// a compound is `adj*(compound-core(N))` — the left-branching compound core forms first (pure nouns),
/// then adjectives apply as a flat conjunction on the outside. Collapses the spurious brackets where a
/// gradable adjective floats to an inner compound slot — `[specific repair] proteins`, `[independent
/// cancer] dependency data sets` — to the single adjective-outside tree.
///
/// Scope is gradable adjectives (the corpus's residual adjective form). Soundness for a gradable —
/// covertly subsective (§5) — rests on the compound's semantic HEAD being fixed regardless of the
/// adjective's attachment depth, so the inner-scope brackets are meaning-equivalent; a genuine
/// meaning-distinct adjective-inside compound (`red blood cell`) is licensed by the lexicon as a
/// multiword unit (§4), never rebuilt here. Intersective adjectives in compounds are not yet in scope
/// (absent from this corpus's residual). The adequacy battery witnesses no reading lost.
pub(super) fn is_adjective_refined(cat: &Exp) -> bool {
    let Some([Exp::Sig(_, _, body), _]) = is_ctor(cat, "cat_n") else {
        return false;
    };
    fn flatten_and<'a>(e: &'a Exp, out: &mut Vec<&'a Exp>) {
        if let Exp::InductiveType(decl, args) = e {
            if decl.iri.as_str() == "urn:eigenius:logic:And" && args.len() == 2 {
                flatten_and(&args[0], out);
                flatten_and(&args[1], out);
                return;
            }
        }
        out.push(e);
    }
    // The predicate an App-spine ultimately applies. A compound / PP / naming restrictor is a direct
    // axiom application `axiom(x, …)` (head = the axiom); a MODIFIER (adjective) restrictor from
    // `mod_apply` is left UN-REDUCED — `(λx. P(x)) x`, head = the `Lam` — so descend the annotation
    // and the binder to reach `P`'s head (`measurements:gt` for a gradable; `prep_*` for a PP, which
    // is thereby excluded).
    fn spine_head(mut e: &Exp) -> &Exp {
        loop {
            match e {
                Exp::App(f, _) => e = f,
                Exp::Ann(inner, _) => e = inner, // the mod-sem's `(e : T)`; pretty-print hides it
                Exp::Lam(_, body) => e = body,   // into the un-reduced `λx. P(x)` body
                _ => return e,
            }
        }
    }
    // Derived VERBAL modifiers (D63 compound morphology §3b): the denominal suffix `PCR-based` →
    // `base(x, PCR)` and the reduced-passive participial `predicted` → `∃a. predict(x, a)`. These are
    // prenominal modifiers exactly like a gradable adjective, so the same adjective-outside NF applies —
    // "PCR-based [MSI phenotyping]" is canonical and "[PCR-based MSI] phenotyping" is the spurious
    // bracketing (it is the PHENOTYPING that is PCR-based, not the MSI). They were missed because their
    // restrictor is a VERB relation, not a degree comparison.
    //
    // Matched POSITIVELY on the importer's verb-frame naming convention `v{offset}_{tag}`
    // (`crates/eigenius-wordnet/src/convert.rs`), which by construction excludes every restrictor a
    // compound noun legitimately carries — `compound_kind`, the essive `is_a`, `named`, a PP `prep_*`,
    // `gt`/`lt` — so the mis-classification that once exploded `MSI as a biomarker` cannot recur. The
    // reduced passive wraps its relation in a Π-CPS, so scan the whole conjunct, not just its spine.
    fn mentions_verb_frame(e: &Exp) -> bool {
        let is_frame = |iri: &crate::ontology::Iri| {
            let local = iri.as_str().rsplit(':').next().unwrap_or("");
            let mut cs = local.chars();
            cs.next() == Some('v')
                && local.contains('_')
                && cs.clone().next().is_some_and(|c| c.is_ascii_digit())
        };
        match e {
            Exp::EigonAxiom(iri) => is_frame(iri),
            Exp::App(f, x) => mentions_verb_frame(f) || mentions_verb_frame(x),
            Exp::Ann(a, b) | Exp::Arrow(a, b) | Exp::Times(a, b) | Exp::Pair(a, b) => {
                mentions_verb_frame(a) || mentions_verb_frame(b)
            }
            Exp::Lam(_, b) => mentions_verb_frame(b),
            Exp::Pi(_, a, b) | Exp::Sig(_, a, b) => {
                mentions_verb_frame(a) || mentions_verb_frame(b)
            }
            Exp::Fst(a) | Exp::Snd(a) => mentions_verb_frame(a),
            Exp::InductiveType(_, args) | Exp::InductiveCtor(_, _, args) => {
                args.iter().any(mentions_verb_frame)
            }
            _ => false,
        }
    }
    let mut conjuncts = Vec::new();
    flatten_and(body, &mut conjuncts);
    conjuncts.iter().any(|c| {
        matches!(spine_head(c), Exp::EigonAxiom(iri)
            if iri.as_str() == "urn:eigenius:measurements:gt"
                || iri.as_str() == "urn:eigenius:measurements:lt")
            || mentions_verb_frame(c)
    })
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

/// A one-line category summary for the arity probe: the constructor name plus its immediate
/// argument constructors, which is enough to recognise `fwd(cat_s, …)` shapes without dumping a
/// whole `InductiveCtor` tree.
fn cat_brief(c: &Exp) -> String {
    match c {
        Exp::InductiveCtor(_, n, args) => {
            let inner: Vec<String> = args
                .iter()
                .map(|a| match a {
                    Exp::InductiveCtor(_, m, _) => m.clone(),
                    _ => "_".to_string(),
                })
                .collect();
            format!("{n}({})", inner.join(","))
        }
        _ => "?".to_string(),
    }
}

/// Arrow-depth of a category's denotation — how many arguments it declares.
fn cat_arity(c: &Exp) -> Option<usize> {
    let mut t = super::super::category::denote_cat(c).ok()?;
    let mut n = 0;
    while let Exp::Arrow(_, cod) | Exp::Pi(_, _, cod) = t {
        t = *cod;
        n += 1;
    }
    Some(n)
}

/// Leading-λ count of a sem's VALUE — how many arguments it actually takes. Evaluated, because an
/// `Exp::App` given too few arguments is syntactically an application and only becomes a closure
/// under evaluation; a syntactic λ-count cannot see it.
fn sem_arity(sem: &Exp) -> Option<usize> {
    let mut v = crate::nbe::eval::eval(sem, &crate::nbe::env::Rho::Nil).ok()?;
    let mut n = 0;
    while let crate::nbe::val::Val::Lam(g) = v {
        v = g
            .apply(crate::nbe::val::Val::Nt(crate::nbe::val::Neut::Gen(
                n,
                format!("__arity{n}"),
            )))
            .ok()?;
        n += 1;
        if n > 8 {
            break;
        }
    }
    Some(n)
}

#[cfg(test)]
mod dispatch_tests {

    //! **Golden characterization of the datafied dispatch families** (the differential oracle for the
    //! Phase 1–2 datafication, `docs/notes/grammar-formalization-plan.md`): the nominal-modification
    //! family (`combine_nominal_mod`) and the "other grammar" binary rules — close-naming apposition
    //! and the GQ-as-preposition-object raise (`combine_other_grammar`). Each test constructs the two
    //! operand [`Item`]s and drives the real CKY step [`apply`], pinning the exact result category,
    //! sem, and provenance. When these `combine_*` functions became data-driven tables, the tests had
    //! to pass byte-identically — that is what makes "formalization changed nothing" a checked claim.
    //! The stacked-adjective flat-Σ `And` path needs a layer that resolves `logic:And`, so it is
    //! covered by the full-page `--no-llm` sweep differential, not here.
    use super::super::constructions::definite_designation;
    /// AUDIT — the rule x guard matrix, and which `Fin` features each rule mentions.
    ///
    /// Not an assertion: a REPORT, printed with `--nocapture`. It exists because the recurring defect
    /// shape in this grammar is *a constraint applied in one rule and not its sibling*, and that is
    /// invisible while the rules are read one at a time. Every fix on 2026-07-26/27 had this shape:
    /// PP-adjacency lived in `appose_group` but not `kind_compound` (24 skeletons); `NotKindRaised`
    /// guarded the singular `name` rule but not the group path; number refinement covered nouns but not
    /// verbs; `reduced_relative` consumed `pss` while the only producer of the patient-subject voice was
    /// `pass`.
    ///
    /// Read the matrix by COLUMN: a guard held by most rules of a family and missing from one is the
    /// thing to justify or fix. Read the feature census for producer/consumer mismatches.
    #[test]
    fn grammar_rule_guard_matrix() {
        fn feats(p: &CatPat, out: &mut Vec<String>) {
            match p {
                CatPat::Ctor(n, args) => {
                    if matches!(
                        *n,
                        "fin"
                            | "bse"
                            | "inf"
                            | "ger"
                            | "pss"
                            | "pass"
                            | "adj"
                            | "pred"
                            | "fin_any"
                            | "sg"
                            | "pl"
                            | "mass"
                            | "num_any"
                    ) {
                        out.push((*n).to_string());
                    }
                    for a in args {
                        feats(a, out);
                    }
                }
                CatPat::Var(_) => {}
            }
        }
        let families: [(&str, &[CatRule]); 2] =
            [("refine", refine_rules()), ("other", other_grammar_rules())];
        let mut cols: Vec<&'static str> = Vec::new();
        for (_, rs) in &families {
            for r in rs.iter() {
                for g in r.guards {
                    if !cols.contains(&g.label()) {
                        cols.push(g.label());
                    }
                }
            }
        }
        cols.sort_unstable();
        eprintln!("\n=== rule x guard ===");
        eprint!("{:<26}", "rule");
        for c in &cols {
            eprint!(" {:>21}", c);
        }
        eprintln!();
        for (fam, rs) in &families {
            for r in rs.iter() {
                eprint!("{:<26}", format!("{fam}/{}", r.name));
                for c in &cols {
                    let cell = r
                        .guards
                        .iter()
                        .find(|g| g.label() == *c)
                        .map(|g| g.side())
                        .unwrap_or("");
                    eprint!(" {cell:>21}");
                }
                eprintln!();
            }
        }
        eprintln!("\n=== Fin/Num features mentioned in each rule's operand patterns ===");
        for (fam, rs) in &families {
            for r in rs.iter() {
                let (mut l, mut rr) = (Vec::new(), Vec::new());
                feats(&r.left_pat, &mut l);
                feats(&r.right_pat, &mut rr);
                if !l.is_empty() || !rr.is_empty() {
                    eprintln!(
                        "{:<26} L={:<22} R={:?}",
                        format!("{fam}/{}", r.name),
                        format!("{l:?}"),
                        rr
                    );
                }
            }
        }
    }
    use super::*;
    use crate::nbe::term::list_decl;
    use crate::ontology::iri::Iri;

    fn ct(name: &str, args: Vec<Exp>) -> Exp {
        // A slash carries a leading MODALITY (D63 multimodal slashes). Tests build the permissive
        // `m_all` unless a case is specifically about mode licensing, so this helper injects it and
        // the call sites keep reading as `A/B` / `A\\B`.
        let args = if name == "fwd" || name == "bwd" {
            let mut v = vec![Exp::InductiveCtor(list_decl(), "m_all".into(), Vec::new())];
            v.extend(args);
            v
        } else {
            args
        };
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

    /// **`restrictor_key` must be invariant under β.** This is the property `conjoin_canonical`'s
    /// canonical order actually depends on, and keying on the raw term did not have it: a restrictor
    /// is stored UN-REDUCED, and one restrictor has several un-reduced forms (a PP object arriving
    /// directly vs through a type-raised GQ). Those sorted to different places and then reduced to
    /// the same term at readback, so one modifier set emitted two Σ differing only in conjunct order
    /// — 26 skeletons of the reference page, measured 2026-07-27.
    ///
    /// The three forms below are the shapes that actually occur: fully reduced, the direct
    /// application, and the raised-GQ detour `λV. V(obj)` applied to a continuation.
    #[test]
    fn restrictor_key_is_beta_invariant() {
        let (x, obj) = ("__cmp_x", cls("urn:eigenius:lexicon:Obj"));
        let prep = ax("urn:eigenius:ontology:prep_to");
        // prep_to(x, obj)
        let reduced = Exp::App(
            Box::new(Exp::App(
                Box::new(prep.clone()),
                Box::new(Exp::Var(x.into())),
            )),
            Box::new(obj.clone()),
        );
        // (λy. λz. prep_to(z, y)) obj x   — the direct route, un-reduced
        let direct = Exp::App(
            Box::new(Exp::App(
                Box::new(Exp::Lam(
                    Patt::Var("y".into()),
                    Box::new(Exp::Lam(
                        Patt::Var("z".into()),
                        Box::new(Exp::App(
                            Box::new(Exp::App(
                                Box::new(prep.clone()),
                                Box::new(Exp::Var("z".into())),
                            )),
                            Box::new(Exp::Var("y".into())),
                        )),
                    )),
                )),
                Box::new(obj.clone()),
            )),
            Box::new(Exp::Var(x.into())),
        );
        // (λV. V(obj)) (λw. λz. prep_to(z, w)) x   — the type-raised-object route
        let raised = Exp::App(
            Box::new(Exp::App(
                Box::new(Exp::Lam(
                    Patt::Var("V".into()),
                    Box::new(Exp::App(
                        Box::new(Exp::Var("V".into())),
                        Box::new(obj.clone()),
                    )),
                )),
                Box::new(Exp::Lam(
                    Patt::Var("w".into()),
                    Box::new(Exp::Lam(
                        Patt::Var("z".into()),
                        Box::new(Exp::App(
                            Box::new(Exp::App(Box::new(prep), Box::new(Exp::Var("z".into())))),
                            Box::new(Exp::Var("w".into())),
                        )),
                    )),
                )),
            )),
            Box::new(Exp::Var(x.into())),
        );

        let k = restrictor_key(&reduced);
        assert_eq!(
            k, "prep_to(__cmp_x, Obj)",
            "the reduced form keys by its normal form"
        );
        assert_eq!(
            restrictor_key(&direct),
            k,
            "direct application must key like its normal form"
        );
        assert_eq!(
            restrictor_key(&raised),
            k,
            "the raised-GQ detour must key like its normal form"
        );
        // The raw forms genuinely differ — otherwise this test would pass vacuously and the defect
        // it guards could not have happened.
        assert_ne!(pretty_term(&direct), pretty_term(&raised));
        assert_ne!(pretty_term(&direct), pretty_term(&reduced));
    }

    /// The β-normalizer GIVES UP rather than capture: reducing `(λf. λa. f(a)) a` would capture the
    /// argument `a` under the inner binder, so the redex is left alone. The key is then the raw form
    /// — a missed collapse, never a wrong one. Sorting only reorders conjuncts, so this costs
    /// nothing but a duplicate that survives.
    #[test]
    fn beta_normalize_refuses_to_capture() {
        let risky = Exp::App(
            Box::new(Exp::Lam(
                Patt::Var("f".into()),
                Box::new(Exp::Lam(
                    Patt::Var("a".into()),
                    Box::new(Exp::App(
                        Box::new(Exp::Var("f".into())),
                        Box::new(Exp::Var("a".into())),
                    )),
                )),
            )),
            Box::new(Exp::Var("a".into())),
        );
        assert_eq!(
            pretty_term(&beta_normalize(&risky)),
            pretty_term(&risky),
            "a capturing redex must be left unreduced, not renamed"
        );
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
        let got = apply(&l, &r, &layer(), crate::dcg::rules::RightContext::Other)
            .expect("[cat_n][cat_n] → kind compound");
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
    fn adjective_outside_compound_nf() {
        let protein = cls("urn:eigenius:lexicon:Protein");
        // A gradable-adjective restrictor `gt(deg_X(x), std_X)` (the `specific` / `notable` form).
        let grad = |x: &str| {
            Exp::App(
                Box::new(Exp::App(
                    Box::new(ax("urn:eigenius:measurements:gt")),
                    Box::new(Exp::App(
                        Box::new(ax("urn:eigenius:wordnet:deg_specific")),
                        Box::new(Exp::Var(x.into())),
                    )),
                )),
                Box::new(ax("urn:eigenius:wordnet:std_specific")),
            )
        };
        let adj_cat = n(sigma_cmp(protein.clone(), grad(COMPOUND_X)));
        assert!(
            is_adjective_refined(&adj_cat),
            "Σ. gt(deg,std) is (gradable-)adjective-refined"
        );
        assert!(!is_compound_refined(&adj_cat), "and NOT compound-refined");

        // Non-adjective refinements a compound noun legitimately carries must NOT be flagged — the
        // over-broad "anything not a compound" version wrongly matched these and exploded the parse.
        let comp_cat = n(sigma_cmp(
            protein.clone(),
            app2_x(
                "urn:eigenius:ontology:compound_kind",
                ax("urn:eigenius:lexicon:repair"),
            ),
        ));
        assert!(
            !is_adjective_refined(&comp_cat),
            "a pure compound is not adjective-refined"
        );

        // DERIVED VERBAL modifiers are modifiers too (D63 compound morphology §3b): the denominal
        // suffix `PCR-based` → `base(x, PCR)` and the reduced-passive participial `predicted` →
        // `∃a. predict(x, a)`. Their restrictor is a VERB relation, not a degree comparison, so the
        // degree-only guard missed them and BOTH bracketings of "PCR-based MSI phenotyping" survived
        // ("[PCR-based MSI] phenotyping" is spurious — it is the PHENOTYPING that is PCR-based).
        // Matched on the importer's verb-frame naming convention `v{offset}_{tag}`.
        let denominal_cat = n(sigma_cmp(
            protein.clone(),
            app2_x(
                "urn:eigenius:wordnet:v00636888_t",
                ax("urn:eigenius:wordnet:n_pcr"),
            ),
        ));
        assert!(
            is_adjective_refined(&denominal_cat),
            "a denominal `-based` verb-frame restrictor is modifier-refined"
        );
        // The reduced passive wraps its relation in a Π-CPS — scanned, not just the spine head.
        let participial_cat = n(sigma_cmp(
            protein.clone(),
            Exp::Pi(
                crate::nbe::term::Patt::Var("C".into()),
                Box::new(Exp::Sort(0)),
                Box::new(app2_x(
                    "urn:eigenius:wordnet:v00917772_t",
                    Exp::Var("C".into()),
                )),
            ),
        ));
        assert!(
            is_adjective_refined(&participial_cat),
            "a Π-wrapped reduced-passive participial restrictor is modifier-refined"
        );
        // A verb-frame axiom is required — a NON-frame relation on the same shape is not a modifier.
        let plain_rel = n(sigma_cmp(
            protein.clone(),
            app2_x(
                "urn:eigenius:ontology:prep_of",
                ax("urn:eigenius:lexicon:repair"),
            ),
        ));
        assert!(
            !is_adjective_refined(&plain_rel),
            "a PP restrictor is not modifier-refined (the naming convention excludes it)"
        );
        let essive_cat = n(sigma_cmp(
            protein.clone(),
            app2_x(
                "urn:eigenius:ontology:is_a",
                cls("urn:eigenius:lexicon:Biomarker"),
            ),
        ));
        assert!(
            !is_adjective_refined(&essive_cat),
            "an essive `is_a` restrictor is not an adjective"
        );

        // The adjective-outside NF: a compound may not form over a gradable-adjective-refined operand,
        // on EITHER side — so the adjective attaches outside the compound core.
        let adj_mod = mk_item(
            adj_cat.clone(),
            sigma_cmp(protein.clone(), grad(COMPOUND_X)),
        );
        let gene = mk_item(
            n(cls("urn:eigenius:lexicon:Gene")),
            cls("urn:eigenius:lexicon:Gene"),
        );
        assert!(
            apply(
                &adj_mod,
                &gene,
                &layer(),
                crate::dcg::rules::RightContext::Other
            )
            .is_none(),
            "adjective-refined LEFT (modifier) blocks kind_compound"
        );
        assert!(
            apply(
                &gene,
                &adj_mod,
                &layer(),
                crate::dcg::rules::RightContext::Other
            )
            .is_none(),
            "adjective-refined RIGHT (head) blocks kind_compound"
        );
        // A pure N-N compound still composes.
        let bare = mk_item(n(protein.clone()), protein.clone());
        assert!(
            apply(
                &bare,
                &gene,
                &layer(),
                crate::dcg::rules::RightContext::Other
            )
            .is_some(),
            "pure N-N compound still composes"
        );
    }

    #[test]
    fn named_compound_is_sigma_over_compound_axiom() {
        // `[cat_np] [cat_n]` → `Σx:C. compound(x, ⟦left⟧)`.
        let name_ref = ax("urn:eigenius:lexicon:achilles");
        let head = cls("urn:eigenius:lexicon:Project");
        let l = mk_item(np(cls("urn:eigenius:lexicon:Achilles")), name_ref.clone());
        let r = mk_item(n(head.clone()), head.clone());
        let got = apply(&l, &r, &layer(), crate::dcg::rules::RightContext::Other)
            .expect("[cat_np][cat_n] → named compound");
        let expected = sigma_cmp(head, app2_x("urn:eigenius:ontology:compound", name_ref));
        assert_eq!(got.cat(), &n(expected.clone()));
        assert_eq!(got.sem(), &expected);
        assert_eq!(got.prov(), Combinator::Compound);
    }

    #[test]
    fn transitive_past_participle_lifts_to_a_penalised_reduced_passive_modifier() {
        // `(S[dcl,pss]\NP)/NP` "predicted" → cat_mod(λx. ∃a. predict(x, a)), at a cost penalty so a real
        // lexical adjective outranks it where one exists (bounding the double-seeding ambiguity).
        let entity_np = np(cls("urn:eigenius:lexicon:Entity"));
        let s_pss = ct("cat_s", vec![ct("dcl", vec![]), ct("pss", vec![])]);
        let participle = ct(
            "fwd",
            vec![ct("bwd", vec![s_pss, entity_np.clone()]), entity_np],
        );
        let lifts = participial_lifts(&mk_item(participle, ax("urn:eigenius:lexicon:predict")));
        assert_eq!(lifts.len(), 1, "one participial modifier");
        assert_eq!(lifts[0].cat(), &cat_mod_cat(), "lifts to cat_mod");
        assert_eq!(
            lifts[0].cost().sense_rank,
            PARTICIPIAL_MOD_PENALTY,
            "carries the deprioritisation penalty"
        );
        assert!(
            matches!(lifts[0].sem(), Exp::Lam(..)),
            "the restrictor is λx. …"
        );
        // A plain NP is not a participle → no participial lift; and `mod_lifts` (adjectives) ignores it.
        assert!(
            participial_lifts(&mk_item(np(cls("urn:eigenius:lexicon:Gene")), Exp::Unit)).is_empty()
        );
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
        let got = apply(&l, &r, &layer(), crate::dcg::rules::RightContext::Other)
            .expect("[cat_n][cat_pp] → pp modifier");
        let expected = sigma_cmp(
            head,
            Exp::App(Box::new(pp_sem), Box::new(Exp::Var(COMPOUND_X.into()))),
        );
        assert_eq!(got.cat(), &n(expected.clone()));
        assert_eq!(got.sem(), &expected);
        assert_eq!(got.prov(), Combinator::Compound);
    }

    /// A PREDICATE NOMINAL must not become an ATTRIBUTIVE modifier. `mod_lifts` encodes that as
    /// `prov != KindRaised`, which covers the BARE kind-raise (`kind_raised_nps`) — but the
    /// predicative indefinite article `a_pred` reaches the SAME category `S[adj]\NP` with the SAME
    /// `is_a` sem by ordinary application, so it carries ordinary provenance and the guard never sees
    /// it. Same asymmetry `is_derived_individual` records for coordination: a shape the
    /// `NotKindRaised` guard never sees.
    ///
    /// This test WITNESSES the open route (it asserts today's behaviour, which is the defect).
    #[test]
    fn predicate_nominal_from_a_pred_still_lifts_to_an_attributive_modifier() {
        // `a promising drug target` — `a_pred` applied, so: S[adj]\NP, sem λsubj. is_a(subj, C).
        let target = cls("urn:eigenius:lexicon:Protein");
        let pred_nominal_sem = Exp::Lam(
            Patt::Var("subj".into()),
            Box::new(Exp::App(
                Box::new(Exp::App(
                    Box::new(ax("urn:eigenius:ontology:is_a")),
                    Box::new(Exp::Var("subj".into())),
                )),
                Box::new(target.clone()),
            )),
        );
        let pred_cat = ct(
            "bwd",
            vec![
                ct("cat_s", vec![ct("dcl", vec![]), ct("adj", vec![])]),
                np(cls("urn:eigenius:lexicon:Entity")),
            ],
        );
        // Ordinary application provenance — NOT `KindRaised`.
        let it = mk_item(pred_cat, pred_nominal_sem);
        assert_ne!(it.prov(), Combinator::KindRaised);

        // THE ASYMMETRY: the category says "adjective" and the provenance says "ordinary
        // application", so neither of the two things `mod_lifts` is allowed to look at can tell this
        // apart from a genuine attributive adjective. That is the defect, and it is a CATEGORY
        // problem — `cat_s(dcl, adj)` is being used for two things that do not distribute alike.
        assert!(
            super::super::super::category::is_adjective_cat(it.cat()),
            "a predicate nominal wears the adjective category"
        );
    }

    #[test]
    fn attrib_adjective_lifts_to_cat_mod_then_refines_a_plain_noun() {
        // A predicative `S[adj]\NP` does NOT refine a noun directly — `cat_mod` is the sole attributive
        // path (D63 §6). The adjective lifts to `cat_mod` (sem unchanged), then `cat_mod + [cat_n]` →
        // `Σx:C. ⟦adj⟧(x)` (bound var `COMPOUND_X`).
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
        // The direct `S[adj]\NP + cat_n` path is gone — the adjective must lift first.
        assert!(
            apply(&l, &r, &layer(), crate::dcg::rules::RightContext::Other).is_none(),
            "S[adj]\\NP no longer refines a noun directly; it must lift to cat_mod"
        );
        // Lift → cat_mod (sem unchanged), then apply → the refined noun.
        let mods = mod_lifts(&l);
        assert_eq!(mods.len(), 1, "one cat_mod lift");
        assert_eq!(mods[0].cat(), &cat_mod_cat());
        assert_eq!(
            mods[0].sem(),
            &adj_sem,
            "cat_mod carries the adjective sem unchanged"
        );
        let got = apply(
            &mods[0],
            &r,
            &layer(),
            crate::dcg::rules::RightContext::Other,
        )
        .expect("cat_mod + cat_n → refined noun");
        let x = COMPOUND_X;
        let expected = Exp::Sig(
            Patt::Var(x.into()),
            Box::new(head),
            Box::new(Exp::App(Box::new(adj_sem), Box::new(Exp::Var(x.into())))),
        );
        assert_eq!(got.cat(), &n(expected.clone()));
        assert_eq!(got.sem(), &expected);
        assert_eq!(got.prov(), Combinator::Compound);
    }

    /// A minimal `logic:And` declaration — the bare test `layer()` does not load `logic`, so build the
    /// decl directly (as the forest tests do) to exercise `conjoin_canonical`.
    fn and_decl() -> Arc<crate::nbe::term::InductiveDecl> {
        Arc::new(crate::nbe::term::InductiveDecl {
            iri: Iri::parse("urn:eigenius:logic:And").unwrap(),
            name: "And".to_string(),
            params: Vec::new(),
            indices: Vec::new(),
            sort: Exp::Sort(0),
            ctors: Vec::new(),
        })
    }

    #[test]
    fn conjoin_canonical_is_order_independent() {
        // The residual-multiplicity fix: two modifiers attached in EITHER CKY order must yield the
        // BYTE-IDENTICAL Σ restrictor, so the two derivations pack + subsume instead of surviving as
        // commutative `And` duplicates (the `And(compound, prep_of)` vs `And(prep_of, compound)` split).
        let and = and_decl();
        let a = app2_x(
            "urn:eigenius:ontology:compound_kind",
            cls("urn:eigenius:umlscui:C1"),
        );
        let b = app2_x(
            "urn:eigenius:ontology:prep_of",
            cls("urn:eigenius:umlscui:C2"),
        );
        assert_eq!(
            conjoin_canonical(&and, &a, b.clone()),
            conjoin_canonical(&and, &b, a.clone()),
            "two attachment orders of the same two modifiers must canonicalize identically",
        );

        // Three modifiers via two different derivation orders — flatten+sort collapses the
        // associativity too: (((a·b)·c)) and (((b·c)·a)) are the same canonical, FLAT, left-nested And.
        let c = app2_x(
            "urn:eigenius:ontology:compound",
            cls("urn:eigenius:umlscui:C3"),
        );
        let ab = conjoin_canonical(&and, &a, b.clone());
        let order1 = conjoin_canonical(&and, &ab, c.clone());
        let bc = conjoin_canonical(&and, &b, c.clone());
        let order2 = conjoin_canonical(&and, &bc, a.clone());
        assert_eq!(
            order1, order2,
            "3-modifier associativity must canonicalize identically"
        );

        // It is a FLAT left-nested And of 3 conjuncts (top And whose left operand is also an And),
        // not a nested Σ-over-Σ.
        let is_and = |e: &Exp| {
            matches!(e,
            Exp::InductiveType(d, args) if d.iri.as_str() == "urn:eigenius:logic:And" && args.len() == 2)
        };
        match &order1 {
            Exp::InductiveType(_, args) if is_and(&order1) => {
                assert!(is_and(&args[0]), "left-nested flat And of 3 conjuncts");
            }
            _ => panic!("expected a top-level And, got {}", pretty_term(&order1)),
        }
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
            apply(&l, &r, &layer(), crate::dcg::rules::RightContext::Other).is_none(),
            "a compound-refined head is not a compound head a second time"
        );
    }

    // ── other-grammar family (Phase 2): naming apposition + GQ-as-prep-object ──

    /// A type-raised subject GQ `S/(S\NP)` — the right operand of every GQ-prep rule.
    fn raised_gq(sem: Exp) -> Item {
        let s = ct("cat_s", vec![ct("dcl", vec![]), ct("fin", vec![])]);
        let vp = ct(
            "bwd",
            vec![s.clone(), np(cls("urn:eigenius:lexicon:Entity"))],
        );
        mk_item(ct("fwd", vec![s, vp]), sem)
    }

    /// The **classifier + designator** construction: the rule builds the REFINED NOUN
    /// `cat_n(Σx:Sortal. named(x, d), num)`, and [`definite_designation`] shifts that to the definite
    /// INDIVIDUAL `the(Σ…).1` at `cat_np(Sortal, num)`. Pins the whole chain, so it carries both
    /// corrections: 2026-07-25 (the sem is `the(Σ…).1`, NOT the kind coercion `kind_of(Σ…)`, and the
    /// type is the CLASSIFIER's, not `Entity` — the old shape injected a `kind_of` wrapper that
    /// multiplied across argument positions, 204 skeletons on one unit) and 2026-07-26 (the rule stops
    /// at `cat_n`, so a determiner can reach the construction at all).
    #[test]
    fn name_apposition_builds_a_refined_noun_that_shifts_to_a_definite_individual() {
        let sortal = cls("urn:eigenius:lexicon:Project");
        let name_ref = ax("urn:eigenius:lexicon:achilles_name");
        let l = mk_item(n(sortal.clone()), sortal.clone());
        let r = mk_item(
            np(cls("urn:eigenius:lexicon:AchillesHero")),
            name_ref.clone(),
        );
        let got = apply(&l, &r, &layer(), crate::dcg::rules::RightContext::Other)
            .expect("[cat_n][proper cat_np] → naming apposition");
        let sigma = sigma_cmp(
            sortal.clone(),
            app2_x("urn:eigenius:ontology:named", name_ref),
        );
        // Half one: the refined common noun — the shape `the` takes as its argument.
        assert_eq!(
            got.cat(),
            &n(sigma.clone()),
            "a refined COMMON NOUN `cat_n(Σx:Sortal. named(x, d))` — jumping straight to `cat_np` put \
             the construction out of a determiner's reach"
        );
        assert_eq!(got.sem(), &sigma, "cat and sem carry the same Σ");
        assert_eq!(got.prov(), Combinator::Compound);
        // Half two: the bare use shifts to the individual that Σ uniquely picks out.
        let (np_cat, np_sem) =
            definite_designation(got.cat()).expect("a naming-refined noun designates definitely");
        assert_eq!(
            np_cat,
            np(sortal),
            "the CLASSIFIER supplies the type — not lexicon:Entity, which discards it"
        );
        assert_eq!(
            np_sem,
            Exp::Fst(Box::new(Exp::App(
                Box::new(ax("urn:eigenius:ontology:the")),
                Box::new(sigma),
            ))),
            "a definite individual `the(Σ…).1`, not the kind coercion `kind_of(Σ…)`"
        );
    }

    /// The construction's NUMBER is the classifier's, not the designator's. Every UMLS named individual
    /// seeds `cat_np(…, sg)`, so sourcing the number from the designator made "the gene MSH2" agree as
    /// though the NAME set the number, and it must survive the definite shift too, since the shifted NP
    /// is what an argument slot agrees against.
    ///
    /// The operands carry DIFFERENT numbers — an underspecified `num_any` classifier against a `sg`
    /// designator — which is what makes the number's source observable. It used to be witnessed with a
    /// PLURAL classifier (`The genes MSH2 affect cells.`), and that witness is no longer available:
    /// [`Guard::NotPlural`] now refuses a plural classifier with a single designator, because a plural
    /// classifier's designators must arrive as a `cat_group` (cardinality agreement, 2026-07-26). The
    /// property under test is unchanged — only the pair that exhibits it.
    #[test]
    fn name_apposition_takes_the_classifiers_number_not_the_designators() {
        let sortal = cls("urn:eigenius:lexicon:Gene");
        let any = ct("num_any", vec![]);
        // An UNDERSPECIFIED classifier and a SINGULAR designator: if the number came from the name, the
        // phrase would be `sg`.
        let l = mk_item(
            ct("cat_n", vec![sortal.clone(), any.clone()]),
            sortal.clone(),
        );
        let r = mk_item(
            np(cls("urn:eigenius:lexicon:GeneOrGenome")),
            ax("urn:eigenius:lexicon:msh2_name"),
        );
        let got = apply(&l, &r, &layer(), crate::dcg::rules::RightContext::Other)
            .expect("[cat_n(num_any)][proper cat_np(sg)] → naming apposition");
        let Some([_ty, num]) = is_ctor(got.cat(), "cat_n") else {
            panic!("the rule yields a refined common noun")
        };
        assert_eq!(
            num, &any,
            "head-initial: the classifier is the head, so the phrase keeps the classifier's number — a \
             name carries no number of its own to contribute"
        );
        let (np_cat, _) = definite_designation(got.cat()).expect("designates definitely");
        assert_eq!(
            np_cat,
            ct("cat_np", vec![sortal.clone(), any]),
            "the classifier's number survives the definite shift — the shifted NP is what an \
             argument slot agrees against"
        );

        // Cardinality agreement: the SAME pair with a plural classifier is refused outright, because a
        // plural classifier needs a designator LIST and this rule can only supply one.
        let pl_l = mk_item(ct("cat_n", vec![sortal.clone(), ct("pl", vec![])]), sortal);
        assert!(
            apply(&pl_l, &r, &layer(), crate::dcg::rules::RightContext::Other).is_none(),
            "a plural classifier takes a designator GROUP (`appose_group`), never a single name"
        );
    }

    /// [`definite_designation`] fires on a NAMING restrictor only. A definite needs uniqueness, and
    /// `named(x, d)` supplies it; a compound restrictor does not, so a compound-refined noun keeps
    /// needing a real determiner rather than silently designating an individual.
    #[test]
    fn definite_designation_refuses_a_non_naming_restrictor() {
        let base = cls("urn:eigenius:lexicon:Project");
        let compound = sigma_cmp(
            base,
            app2_x(
                "urn:eigenius:ontology:compound",
                ax("urn:eigenius:lexicon:achilles"),
            ),
        );
        assert!(
            definite_designation(&n(compound)).is_none(),
            "a compound-refined noun is not uniquely identifying"
        );
    }

    #[test]
    fn name_apposition_skips_a_bare_entity_np() {
        // The `ProperName` guard rejects `cat_np(Entity)` (a pronoun / bare kind), and nothing else
        // matches (cat_n, cat_np) → `None`.
        let l = mk_item(
            n(cls("urn:eigenius:lexicon:Project")),
            cls("urn:eigenius:lexicon:Project"),
        );
        let r = mk_item(
            np(cls("urn:eigenius:lexicon:Entity")),
            ax("urn:eigenius:lexicon:it"),
        );
        assert!(
            apply(&l, &r, &layer(), crate::dcg::rules::RightContext::Other).is_none(),
            "close-naming apposition does not fire on a bare-kind cat_np(Entity)"
        );
    }

    #[test]
    fn gq_prep_object_ppmod_raises_into_a_cat_pp() {
        // `[cat_pp/NP] [raised GQ]` → `λx. Q(λy. (prep y) x)` : result category `cat_pp`.
        let prep = ax("urn:eigenius:lexicon:in_prep");
        let q = ax("urn:eigenius:lexicon:some_gq");
        let cat_pp = ct("cat_pp", vec![]);
        let left_cat = ct(
            "fwd",
            vec![cat_pp.clone(), np(cls("urn:eigenius:lexicon:Entity"))],
        );
        let l = mk_item(left_cat, prep.clone());
        let r = raised_gq(q.clone());
        let got = apply(&l, &r, &layer(), crate::dcg::rules::RightContext::Other)
            .expect("[cat_pp/NP][raised GQ] → GQ-prep PpMod");
        let (x, y) = ("__pobj_x", "__pobj_y");
        let inner = Exp::Lam(
            Patt::Var(y.into()),
            Box::new(Exp::App(
                Box::new(Exp::App(Box::new(prep), Box::new(Exp::Var(y.into())))),
                Box::new(Exp::Var(x.into())),
            )),
        );
        let expected_sem = Exp::Lam(
            Patt::Var(x.into()),
            Box::new(Exp::App(Box::new(q), Box::new(inner))),
        );
        assert_eq!(
            got.cat(),
            &cat_pp,
            "result is the preposition's cat_pp result"
        );
        assert_eq!(got.sem(), &expected_sem);
        assert_eq!(got.prov(), Combinator::Other);
    }

    #[test]
    fn gq_prep_object_argmarker_applies_gq_to_marker() {
        // `[cat_pp_arg/NP] [raised GQ]` → `Q(marker)` : result category `cat_pp_arg`.
        let marker = ax("urn:eigenius:lexicon:to_marker");
        let q = ax("urn:eigenius:lexicon:some_gq");
        let pp_arg = ct("cat_pp_arg", vec![ct("prep_any", vec![])]);
        let left_cat = ct(
            "fwd",
            vec![pp_arg.clone(), np(cls("urn:eigenius:lexicon:Entity"))],
        );
        let l = mk_item(left_cat, marker.clone());
        let r = raised_gq(q.clone());
        let got = apply(&l, &r, &layer(), crate::dcg::rules::RightContext::Other)
            .expect("[cat_pp_arg/NP][raised GQ] → GQ-prep ArgMarker");
        let expected_sem = Exp::App(Box::new(q), Box::new(marker));
        assert_eq!(
            got.cat(),
            &pp_arg,
            "result is the cat_pp_arg marker category"
        );
        assert_eq!(got.sem(), &expected_sem);
        assert_eq!(got.prov(), Combinator::Other);
    }

    #[test]
    fn gq_prep_object_vpadjunct_builds_vp_modifier() {
        // `[(S\NP)\(S\NP)/NP] [raised GQ]` → `λV.λs. Q(λx. prep(x)(V)(s))` : result `(S\NP)\(S\NP)`.
        let prep = ax("urn:eigenius:lexicon:during_prep");
        let q = ax("urn:eigenius:lexicon:some_gq");
        let ent = || np(cls("urn:eigenius:lexicon:Entity"));
        let s = || ct("cat_s", vec![ct("dcl", vec![]), ct("fin", vec![])]);
        let vp = || ct("bwd", vec![s(), ent()]);
        let vpadj = ct("bwd", vec![vp(), vp()]);
        let left_cat = ct("fwd", vec![vpadj.clone(), ent()]);
        let l = mk_item(left_cat, prep.clone());
        let r = raised_gq(q.clone());
        let got = apply(&l, &r, &layer(), crate::dcg::rules::RightContext::Other)
            .expect("[VP-adjunct/NP][raised GQ] → GQ-prep VpAdjunct");
        let (x, v, sv) = ("__pobj_x", "__pobj_V", "__pobj_s");
        let applied = Exp::App(
            Box::new(Exp::App(
                Box::new(Exp::App(Box::new(prep), Box::new(Exp::Var(x.into())))),
                Box::new(Exp::Var(v.into())),
            )),
            Box::new(Exp::Var(sv.into())),
        );
        let scoped = Exp::App(
            Box::new(q),
            Box::new(Exp::Lam(Patt::Var(x.into()), Box::new(applied))),
        );
        let expected_sem = Exp::Lam(
            Patt::Var(v.into()),
            Box::new(Exp::Lam(Patt::Var(sv.into()), Box::new(scoped))),
        );
        assert_eq!(
            got.cat(),
            &vpadj,
            "result is the VP-adjunct (S\\NP)\\(S\\NP)"
        );
        assert_eq!(got.sem(), &expected_sem);
        assert_eq!(got.prov(), Combinator::Other);
    }

    // ── universal combinators (Phase 2b): application, composition, dependent determiner ──

    fn s_fin() -> Exp {
        ct("cat_s", vec![ct("dcl", vec![]), ct("fin", vec![])])
    }
    fn ent_np() -> Exp {
        np(cls("urn:eigenius:lexicon:Entity"))
    }

    #[test]
    fn forward_application_applies_functor_to_argument() {
        // `A/B · B → A`, sem `App(L, R)`.
        let l = mk_item(
            ct("fwd", vec![s_fin(), ent_np()]),
            ax("urn:eigenius:lexicon:verb"),
        );
        let r = mk_item(ent_np(), ax("urn:eigenius:lexicon:subj"));
        let got =
            apply(&l, &r, &layer(), crate::dcg::rules::RightContext::Other).expect("A/B · B → A");
        assert_eq!(got.cat(), &s_fin(), "result is the functor's result A");
        assert_eq!(
            got.sem(),
            &Exp::App(
                Box::new(ax("urn:eigenius:lexicon:verb")),
                Box::new(ax("urn:eigenius:lexicon:subj"))
            )
        );
        assert_eq!(got.prov(), Combinator::ForwardApp);
    }

    #[test]
    fn backward_application_applies_functor_to_argument() {
        // `B · A\B → A`, sem `App(R, L)`.
        let l = mk_item(ent_np(), ax("urn:eigenius:lexicon:subj"));
        let r = mk_item(
            ct("bwd", vec![s_fin(), ent_np()]),
            ax("urn:eigenius:lexicon:vp"),
        );
        let got =
            apply(&l, &r, &layer(), crate::dcg::rules::RightContext::Other).expect("B · A\\B → A");
        assert_eq!(got.cat(), &s_fin());
        assert_eq!(
            got.sem(),
            &Exp::App(
                Box::new(ax("urn:eigenius:lexicon:vp")),
                Box::new(ax("urn:eigenius:lexicon:subj"))
            )
        );
        assert_eq!(got.prov(), Combinator::BackwardApp);
    }

    #[test]
    fn forward_composition_composes_two_functors() {
        // `A/B ∘ B/C → A/C`, sem `λz. L(R z)`.
        let pp = ct("cat_pp", vec![]);
        let l = mk_item(
            ct("fwd", vec![s_fin(), ent_np()]),
            ax("urn:eigenius:lexicon:f"),
        );
        let r = mk_item(
            ct("fwd", vec![ent_np(), pp.clone()]),
            ax("urn:eigenius:lexicon:g"),
        );
        let got = apply(&l, &r, &layer(), crate::dcg::rules::RightContext::Other)
            .expect("A/B ∘ B/C → A/C");
        assert_eq!(
            got.cat(),
            &ct("fwd", vec![s_fin(), pp]),
            "result is A/C = S/PP"
        );
        let z = "__comp_z";
        let expected = Exp::Lam(
            Patt::Var(z.into()),
            Box::new(Exp::App(
                Box::new(ax("urn:eigenius:lexicon:f")),
                Box::new(Exp::App(
                    Box::new(ax("urn:eigenius:lexicon:g")),
                    Box::new(Exp::Var(z.into())),
                )),
            )),
        );
        assert_eq!(got.sem(), &expected);
        assert_eq!(got.prov(), Combinator::ForwardComp);
    }

    /// A slash carrying an EXPLICIT modality — `ct` injects the permissive `m_all`, so mode-licensing
    /// tests need to name the mode themselves.
    fn ct_mode(name: &str, mode: &str, args: Vec<Exp>) -> Exp {
        let mut v = vec![Exp::InductiveCtor(list_decl(), mode.into(), Vec::new())];
        v.extend(args);
        Exp::InductiveCtor(list_decl(), name.into(), v)
    }

    // ── multimodal slash licensing (Baldridge 2002 §5.2) ─────────────────────────────────────────
    //
    // These three tests are the WITNESS that the mode machinery does what the lattice says, and they
    // are deliberately written BEFORE any lexical entry is tightened to `m_app`: the migration ships
    // with every slash at `m_all`, so without them nothing would exercise a non-permissive mode.

    #[test]
    fn application_only_slash_refuses_composition() {
        // `A/⋆B ∘ B/⋆C` must NOT compose: harmonic composition is keyed to `⋄` (194), and `m_app` is
        // the lattice ROOT — not a subtype of `⋄` — so it cannot serve as input. This is the
        // categorial statement of what a governed complement needs: `give rise TO` must not let `to`
        // compose away from its head.
        let pp = ct("cat_pp", vec![]);
        let l = mk_item(
            ct_mode("fwd", "m_app", vec![s_fin(), ent_np()]),
            ax("urn:eigenius:lexicon:f"),
        );
        let r = mk_item(
            ct_mode("fwd", "m_app", vec![ent_np(), pp]),
            ax("urn:eigenius:lexicon:g"),
        );
        assert!(
            apply(&l, &r, &layer(), crate::dcg::rules::RightContext::Other).is_none(),
            "an application-only slash must not compose"
        );
    }

    #[test]
    fn application_only_slash_still_applies() {
        // …but `m_app` means application ONLY, not application NEVER. Application is keyed to the
        // lattice root (192), so every modality — `m_app` included — still applies. If this ever
        // fails, `m_app` has become a category no rule can consume, and tightening a slash to it
        // would open a grammar gap rather than remove a spurious reading.
        let l = mk_item(
            ct_mode("fwd", "m_app", vec![s_fin(), ent_np()]),
            ax("urn:eigenius:lexicon:verb"),
        );
        let r = mk_item(ent_np(), ax("urn:eigenius:lexicon:subj"));
        let got = apply(&l, &r, &layer(), crate::dcg::rules::RightContext::Other)
            .expect("A/⋆B · B → A must still apply");
        assert_eq!(got.cat(), &s_fin(), "application is keyed to the root ⋆");
    }

    #[test]
    fn one_application_only_operand_is_enough_to_refuse() {
        // (194) keys BOTH slashes: `X/⋄Y Y/⋄Z`. A permissive primary does not rescue a `m_app`
        // secondary — otherwise marking the governed slash would be defeated by its head.
        let pp = ct("cat_pp", vec![]);
        let permissive = mk_item(
            ct("fwd", vec![s_fin(), ent_np()]),
            ax("urn:eigenius:lexicon:f"),
        );
        let restricted = mk_item(
            ct_mode("fwd", "m_app", vec![ent_np(), pp]),
            ax("urn:eigenius:lexicon:g"),
        );
        assert!(
            apply(
                &permissive,
                &restricted,
                &layer(),
                crate::dcg::rules::RightContext::Other
            )
            .is_none(),
            "a permissive primary must not license an application-only secondary"
        );
    }

    #[test]
    fn dependent_determiner_plain_noun_applies() {
        // `cat_forall(λT. cat_np(T)) · cat_n(Gene)` with a NON-Σ noun → plain forward application,
        // binding `T := Gene`.
        let forall = ct(
            "cat_forall",
            vec![
                ct("num_any", vec![]),
                Exp::Lam(Patt::Var("T".into()), Box::new(np(Exp::Var("T".into())))),
            ],
        );
        let l = mk_item(forall, ax("urn:eigenius:lexicon:det"));
        let r = mk_item(
            n(cls("urn:eigenius:lexicon:Gene")),
            ax("urn:eigenius:lexicon:noun"),
        );
        let got = apply(&l, &r, &layer(), crate::dcg::rules::RightContext::Other)
            .expect("cat_forall · plain cat_n → forward application");
        assert_eq!(
            got.cat(),
            &np(cls("urn:eigenius:lexicon:Gene")),
            "T bound to the noun type Gene"
        );
        assert_eq!(
            got.sem(),
            &Exp::App(
                Box::new(ax("urn:eigenius:lexicon:det")),
                Box::new(ax("urn:eigenius:lexicon:noun"))
            )
        );
        assert_eq!(got.prov(), Combinator::ForwardApp);
    }

    #[test]
    fn dependent_determiner_refined_noun_fst_projects() {
        // `cat_forall(λT. cat_np(T)) · cat_n(Σx:Gene. φ)` → DetRefine: bind `T := Gene` (component)
        // and Fst-project the witness in the sem.
        let forall = ct(
            "cat_forall",
            vec![
                ct("num_any", vec![]),
                Exp::Lam(Patt::Var("T".into()), Box::new(np(Exp::Var("T".into())))),
            ],
        );
        let comp = cls("urn:eigenius:lexicon:Gene");
        let sigma = Exp::Sig(
            Patt::Var("x".into()),
            Box::new(comp.clone()),
            Box::new(Exp::Sort(0)),
        );
        let l = mk_item(forall, ax("urn:eigenius:lexicon:det"));
        let r = mk_item(n(sigma.clone()), ax("urn:eigenius:lexicon:noun"));
        let got = apply(&l, &r, &layer(), crate::dcg::rules::RightContext::Other)
            .expect("cat_forall · refined cat_n → DetRefine");
        assert_eq!(
            got.cat(),
            &np(comp),
            "T bound to the component type Gene (not the whole Σ)"
        );
        // λv. det(Σ)(λz. v(Fst z))
        let (v, z) = ("__refine_v", "__refine_z");
        let expected = Exp::Lam(
            Patt::Var(v.into()),
            Box::new(Exp::App(
                Box::new(Exp::App(
                    Box::new(ax("urn:eigenius:lexicon:det")),
                    Box::new(sigma),
                )),
                Box::new(Exp::Lam(
                    Patt::Var(z.into()),
                    Box::new(Exp::App(
                        Box::new(Exp::Var(v.into())),
                        Box::new(Exp::Fst(Box::new(Exp::Var(z.into())))),
                    )),
                )),
            )),
        );
        assert_eq!(got.sem(), &expected);
        assert_eq!(got.prov(), Combinator::ForwardApp);
    }
}
