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
//! **The construction rules** — how each grammatical construction builds its result category and its
//! semantics: coordination, the relatives, the appositives, the reciprocal, the distributives,
//! type-raising, the kind shift, and the fronted participial.
//!
//! Each is a pure function of the operands' `(cat, sem)` — no chart, no lexicon, no config. The
//! *registry* ([`super::registry`]) says WHERE each fires and dispatches to them; the chart drivers
//! ([`super::super::chart`]) apply them. Splitting the two is what let one definition of each trigger
//! serve both drivers.
//!
//! These lived in `category.rs`, whose own module doc describes only the Cat *algebra* — the `⟦·⟧`
//! homomorphism, unification, subsumption, the feature meet. Twenty-one grammar rules had accumulated
//! behind that description. The algebra is a theory of categories; a construction is a fact about
//! English. They are different layers, and now they are different files.

use std::sync::Arc;

use crate::layer::Layer;
use crate::nbe::term::{list_decl, Exp, InductiveDecl, Patt};
use crate::ontology::iri::Iri;

use super::super::category::*;

/// Generalized conjunction/disjunction (Partee & Rooth): pointwise-lift the
/// connective `op` over a Prop-ending denotation. At `Prop`, build `op(a, b)`; at
/// an arrow, η-expand — `λx. coord(cod, a x, b x)`. So `S` conjoins to `op(P,Q)`,
/// `VP` to `λx. op(P x, Q x)`, `TV` to `λo.λs. op(P o s, Q o s)`.
fn generalized_coord(
    op: &Arc<InductiveDecl>,
    denote: &Exp,
    a: &Exp,
    b: &Exp,
    depth: usize,
) -> Option<Exp> {
    match denote {
        Exp::Sort(0) => {
            if std::env::var("EIGENIUS_TRACE_COORD").is_ok()
                && (matches!(a, Exp::Lam(..)) || matches!(b, Exp::Lam(..)))
            {
                eprintln!("  !! generalized_coord Sort(0) with a Lam argument: denote={denote:?}");
            }
            Some(Exp::InductiveType(op.clone(), vec![a.clone(), b.clone()]))
        }
        Exp::Arrow(_, cod) | Exp::Pi(_, _, cod) => {
            let var = format!("conj{depth}");
            let app = |f: &Exp| Exp::App(Box::new(f.clone()), Box::new(Exp::Var(var.clone())));
            let body = generalized_coord(op, cod, &app(a), &app(b), depth + 1)?;
            Some(Exp::Lam(Patt::Var(var), Box::new(body)))
        }
        _ => None,
    }
}

/// Two constituents coordinate iff their categories are the **same** (mutually
/// subsuming) and Prop-ending (`S`/`VP`/`TV`…). D63 §8.4 Phase 3.
pub fn cats_coordinate(x: &Exp, y: &Exp, layer: &Arc<Layer>) -> bool {
    unify_cat(x, y, layer).is_some()
        && unify_cat(y, x, layer).is_some()
        && denote_cat(x).map(|d| prop_ending(&d)).unwrap_or(false)
}

/// The sem of `a but not b` for same-category, Prop-ending constituents (D62 §2 #8): the
/// pointwise-lifted **contrastive** conjunction `a ∧ ¬b` — at `Prop`, `And(a, b→False)`; at an
/// arrow, η-expand and recurse (so two VPs give `λs. And(a s, ¬(b s))`, two object-raised GQs give
/// `λTV.λsubj. And(a TV subj, ¬(b TV subj))`). This is the general contrastive-ellipsis treatment —
/// the shared functor (verb / TV) applies affirmatively to `a` and negatively to the elided `b`,
/// covering determined-NP / GQ objects (`required the helicase activity but not its exonuclease
/// activity`), VP-level, and clause-level `but not`. `None` if `cat` isn't conjoinable or `logic:And`
/// / `logic:False` don't resolve. (Bare-NAME objects, which are not Prop-ending, use the
/// [`coordinate_but_not`] group path instead.)
pub fn coordinate_but_not_sem(cat: &Exp, a: &Exp, b: &Exp, layer: &Arc<Layer>) -> Option<Exp> {
    let denote = denote_cat(cat).ok()?;
    let and = resolve_inductive(layer, "urn:eigenius:logic:And")?;
    but_not_coord(&and, &denote, a, b, 0, layer)
}

fn but_not_coord(
    and: &Arc<InductiveDecl>,
    denote: &Exp,
    a: &Exp,
    b: &Exp,
    depth: usize,
    layer: &Arc<Layer>,
) -> Option<Exp> {
    match denote {
        Exp::Sort(0) => Some(Exp::InductiveType(
            and.clone(),
            vec![a.clone(), negate(b.clone(), layer)?],
        )),
        Exp::Arrow(_, cod) | Exp::Pi(_, _, cod) => {
            let var = format!("bn{depth}");
            let app = |f: &Exp| Exp::App(Box::new(f.clone()), Box::new(Exp::Var(var.clone())));
            let body = but_not_coord(and, cod, &app(a), &app(b), depth + 1, layer)?;
            Some(Exp::Lam(Patt::Var(var), Box::new(body)))
        }
        _ => None,
    }
}

/// The `Conn` ctor NAME on a coordination category's connective argument (`conn_and` / `conn_or` /
/// `conn_list` / `conn_but_not`).
fn conn_name_of(conn: &Exp) -> Option<&str> {
    match conn {
        Exp::InductiveCtor(_, n, _) => Some(n.as_str()),
        _ => None,
    }
}

/// Whether a sem is a completed coordination — an `And`/`Or` after peeling the pointwise λ's. A
/// `cat_coord` list sem (a `cons`/`nil` chain) is NOT one, so extending a list is unaffected; this only
/// blocks a *completed* coordination from re-entering `coordinate_prop` as a fresh conjunct.
///
/// **This is the one sem property a combination DECISION consults** (here in [`coordinate_prop`], and
/// in the `but not` rule's left-branching guard). It is therefore part of the packed-forest signature
/// ([`super::chart::forest::node_sig`]): two items that disagree on it do NOT behave identically under future
/// combination, so they must not share a node. See the invariant documented on `node_sig`.
pub(crate) fn sem_is_coordination(sem: &Exp) -> bool {
    let mut e = sem;
    while let Exp::Lam(_, body) = e {
        e = body;
    }
    matches!(e, Exp::InductiveType(d, _)
        if matches!(d.iri.as_str(), "urn:eigenius:logic:And" | "urn:eigenius:logic:Or"))
}

/// Build or extend a **prop-ending coordination list** `cat_coord(BaseCat, conn)` (D63 §8.4 Phase 3,
/// the list-with-operator model ported from core-en `conj.xsl`). This is the prop-side analogue of
/// [`coordinate_np`]: instead of folding `a <op> b` EAGERLY (the retired [`coordinate_sem`]), it
/// DEFERS — accumulating the conjunct sems in a `List` and marking the connective, which the trailing
/// `and`/`or` finalizes and [`complete_coord`] later folds. The left conjunct `l` is either a fresh
/// prop-ending constituent (`S` / `S\NP` / `S[adj]\NP` / `TV` — the first coordination) or an existing
/// `cat_coord` (extend — the left-branching n-ary case); the right `r` is always a single
/// non-`cat_coord` prop-ending constituent. A neutral `conn_list` left accepts ANY op (the trailing
/// `and`/`or` rebinds it); a FINALIZED left must share the op (no `X and Y or Z` mixing). `op_iri` is
/// `logic:And` / `logic:Or` / [`LIST_CONN`] (a comma). `None` unless `l`/`r` coordinate (same
/// category, prop-ending) and the connectives are compatible.
/// Raise a **bare-kind** `cat_np` conjunct to the OBJECT-GQ shape of its partner, so it can coordinate
/// with a determined quantifier — "HeLa affects **genes or a cell line**" (D63 §8.4).
///
/// Type-raising is motivated exactly HERE: coordinating unlike constituents. It is NOT needed for
/// ordinary argument filling, which is why the bare-kind shift yields a plain `cat_np` (core-en's
/// `bnp`, `n $1 → np $1`) — core-en likewise raises only `QuantNP`. Raising on demand keeps the plain
/// NP available for every argument slot (including the non-final ones type-raising cannot reach, the
/// ESSIVE / ditransitive frames) while restoring the one construction that genuinely needs the GQ.
///
/// `gq_cat` is the partner's object-GQ category `(S\NP)\((S\NP)/NP_T')`; the raise reuses its shape
/// with THIS conjunct's class substituted into the exposed object slot, so `common_cat` can then widen
/// the two indices to their `common_super` as it does for two determined GQs. Sem is the standard
/// object raise `λTV. λsubj. TV(kind, subj)` over the bare kind's `kind_of(t)` entity.
fn raise_kind_to_object_gq(np_cat: &Exp, np_sem: &Exp, gq_cat: &Exp) -> Option<(Exp, Exp)> {
    let [t, _num] = is_ctor(np_cat, "cat_np")? else {
        return None;
    };
    // The partner must be an object GQ: `(S\NP) \ ((S\NP)/NP)`.
    let (res_mode, res, inner) = slash_parts(gq_cat, "bwd")?;
    let (inner_mode, vp, obj) = slash_parts(inner, "fwd")?;
    let [_t_other, obj_num] = is_ctor(obj, "cat_np")? else {
        return None;
    };
    let (Exp::InductiveCtor(cat_decl, _, _), Exp::InductiveCtor(_, _, _)) = (gq_cat, obj) else {
        return None;
    };
    let new_obj = Exp::InductiveCtor(
        cat_decl.clone(),
        "cat_np".into(),
        vec![t.clone(), obj_num.clone()],
    );
    // Rebuilding the SAME category with a refined object: both slashes keep their own modality.
    let new_inner = Exp::InductiveCtor(
        cat_decl.clone(),
        "fwd".into(),
        vec![inner_mode.clone(), vp.clone(), new_obj],
    );
    let cat = Exp::InductiveCtor(
        cat_decl.clone(),
        "bwd".into(),
        vec![res_mode.clone(), res.clone(), new_inner],
    );
    let tv_app = Exp::App(
        Box::new(Exp::App(
            Box::new(Exp::Var("TV".into())),
            Box::new(np_sem.clone()),
        )),
        Box::new(Exp::Var("subj".into())),
    );
    let sem = Exp::Lam(
        Patt::Var("TV".into()),
        Box::new(Exp::Lam(Patt::Var("subj".into()), Box::new(tv_app))),
    );
    Some((cat, sem))
}

pub fn coordinate_prop(
    op_iri: &str,
    l_cat: &Exp,
    l_sem: &Exp,
    r_cat: &Exp,
    r_sem: &Exp,
    layer: &Arc<Layer>,
) -> Option<(Exp, Exp)> {
    // Left-branching normal form: the right conjunct is a single constituent — neither a coordination
    // list (`cat_coord`) nor a completed coordination (an `And`/`Or` sem). So `A and B and C` parses
    // only as `(A and B) and C`.
    if is_ctor(r_cat, "cat_coord").is_some() || sem_is_coordination(r_sem) {
        return None;
    }
    let conn_name = match op_iri {
        "urn:eigenius:logic:And" => "conn_and",
        "urn:eigenius:logic:Or" => "conn_or",
        LIST_CONN => "conn_list",
        _ => return None,
    };
    // BARE KIND ⊕ determined quantifier: raise the bare kind's plain `cat_np` to the partner's
    // object-GQ shape ([`raise_kind_to_object_gq`]) so the two coordinate. Either side may be the
    // bare kind ("genes or a cell line" / "a cell line or genes"); the raised copies are local to
    // this coordination, so ordinary argument positions keep the plain NP and gain no duplicate.
    let (l_raised, r_raised);
    let (l_cat, l_sem, r_cat, r_sem) = if is_ctor(l_cat, "cat_np").is_some() {
        match raise_kind_to_object_gq(l_cat, l_sem, r_cat) {
            Some((c, m)) => {
                l_raised = (c, m);
                (&l_raised.0, &l_raised.1, r_cat, r_sem)
            }
            None => (l_cat, l_sem, r_cat, r_sem),
        }
    } else if is_ctor(r_cat, "cat_np").is_some() {
        match raise_kind_to_object_gq(r_cat, r_sem, l_cat) {
            Some((c, m)) => {
                r_raised = (c, m);
                (l_cat, l_sem, &r_raised.0, &r_raised.1)
            }
            None => (l_cat, l_sem, r_cat, r_sem),
        }
    } else {
        (l_cat, l_sem, r_cat, r_sem)
    };
    let (base_cat, members): (Exp, Vec<Exp>) = match is_ctor(l_cat, "cat_coord") {
        // Extend an existing list: a neutral `conn_list` accepts any op; a finalized one must match.
        Some([base, l_conn]) => {
            let lc = conn_name_of(l_conn)?;
            if lc != "conn_list" && lc != conn_name {
                return None;
            }
            (base.clone(), group_members(l_sem)?)
        }
        // First coordination: `l` is a fresh prop-ending constituent — NOT a completed coordination
        // (an `And`/`Or` sem). Blocking that keeps the left-branching normal form single-valued: a list
        // is built by EXTENDING the `cat_coord` (above), never by completing a sub-list and
        // re-coordinating it (which would double-derive `A and B and C`).
        _ => {
            if sem_is_coordination(l_sem)
                || !denote_cat(l_cat).map(|d| prop_ending(&d)).unwrap_or(false)
            {
                return None;
            }
            (l_cat.clone(), vec![l_sem.clone()])
        }
    };
    // The right conjunct coordinates with the base — EXACT (same category, prop-ending) or, for
    // type-raised quantifiers over DIFFERENT noun types (D63 §8.4: `a gene or a cell line`), at their
    // type-generalized common category ([`common_cat`]: exposed `cat_np` indices widened to
    // `common_super`, per-member sems preserved + folded pointwise). Only a prop-ending functor
    // generalizes — the pointwise fold needs a shared denotation; atoms stay exact.
    let base_cat = if cats_coordinate(&base_cat, r_cat, layer) {
        base_cat
    } else {
        match common_cat(&base_cat, r_cat, layer) {
            // Only OBJECT-GQs (backward-headed `(S\NP)\((S\NP)/NP)`) generalize: object coordination has
            // no subject–verb number agreement, so the pointwise generalized-conjunction fold is safe.
            // SUBJECT-GQs (`S/(S\NP)`, forward-headed) must NOT take this path — a coordinated subject
            // needs the plural-group promotion of the NP-list path (`coordinate_np`) so agreement bites
            // (`*HeLa and BRCA1 affects HeLa`). Gate on the object-GQ shape (top-level `bwd`).
            Some(gen)
                if is_ctor(&gen, "bwd").is_some()
                    && denote_cat(&gen).map(|d| prop_ending(&d)).unwrap_or(false) =>
            {
                gen
            }
            _ => return None,
        }
    };
    let Exp::InductiveCtor(cat_decl, _, _) = r_cat else {
        return None;
    };
    let mut all = members;
    all.push(r_sem.clone());
    let conn = Exp::InductiveCtor(
        resolve_inductive(layer, "urn:eigenius:lexicon:Conn")?,
        conn_name.into(),
        vec![],
    );
    let coord_cat = Exp::InductiveCtor(cat_decl.clone(), "cat_coord".into(), vec![base_cat, conn]);
    Some((coord_cat, list_term(&all)))
}

/// Coordinate pre-nominal **modifiers** (`cat_mod`) into a deferred `cat_coord(cat_mod, …)` list —
/// the attributive counterpart of [`coordinate_prop`] (D63 coordinated-modifier category, §6). A
/// coordinated attributive modifier is UNION over kinds — "gastric, endometrial and ovarian cancers"
/// is a cancer that is gastric OR endometrial OR ovarian — so the surface connective ("and" / "or" /
/// comma) is IRRELEVANT and the list always folds `Or` at completion ([`complete_coord`]). This is
/// what the category split buys: the SAME adjective coordinates predicatively ("X is gastric and
/// ovarian" — intersective `And`) via [`coordinate_prop`] on its `S[adj]\NP` form, and attributively
/// (union `Or`) here on its lifted `cat_mod` form. The category (`cat_mod` vs `S[adj]\NP`) is the
/// grammatical pivot. Left-branching NF: the right conjunct is a single `cat_mod` (never a list, never
/// an already-completed `Or`); the left is a fresh `cat_mod` (first coordination) or an existing
/// `cat_coord(cat_mod, …)` (extend). `None` otherwise.
pub fn coordinate_mod(
    l_cat: &Exp,
    l_sem: &Exp,
    r_cat: &Exp,
    r_sem: &Exp,
    layer: &Arc<Layer>,
) -> Option<(Exp, Exp)> {
    // Right conjunct: a single modifier — not a list, not an already-completed coordination.
    if is_ctor(r_cat, "cat_coord").is_some() || sem_is_coordination(r_sem) {
        return None;
    }
    is_ctor(r_cat, "cat_mod")?;
    let members: Vec<Exp> = match is_ctor(l_cat, "cat_coord") {
        // Extend a modifier list.
        Some([base, _conn]) => {
            is_ctor(base, "cat_mod")?;
            group_members(l_sem)?
        }
        // First coordination: a fresh `cat_mod`, not an already-completed `Or`.
        _ => {
            is_ctor(l_cat, "cat_mod")?;
            if sem_is_coordination(l_sem) {
                return None;
            }
            vec![l_sem.clone()]
        }
    };
    let mut all = members;
    all.push(r_sem.clone());
    // Neutral connective marker; `complete_coord` folds `Or` for a `cat_mod` base regardless (D63 §6).
    let conn = Exp::InductiveCtor(
        resolve_inductive(layer, "urn:eigenius:lexicon:Conn")?,
        "conn_list".into(),
        vec![],
    );
    let coord_cat = Exp::InductiveCtor(list_decl(), "cat_coord".into(), vec![r_cat.clone(), conn]);
    Some((coord_cat, list_term(&all)))
}

/// **List-completion** (D63 §8.4 Phase 3, core-en's `s-list` / `pred-adj-list` type-changing rules):
/// fold a prop-ending coordination `cat_coord(BaseCat, conn)` into its base category, applying the
/// operator pointwise over the accumulated members — `op(op(m₀, m₁), m₂)…` (left-branching normal
/// form, via [`generalized_coord`]). Needs ≥2 members. Returns `(BaseCat, folded_sem)`; `None` for an
/// ill-formed list or an unresolvable operator. Realized as a unary shift in both CKY paths (packable).
///
/// **A comma list folds only when it is COMPLETE** (2026-07-26), which is what `rctx` decides.
///
/// The comma is polarity-NEUTRAL by [`LIST_CONN`]'s specification and inherits the list's final
/// explicit connective, so `A, B, C or D` is a four-way `∨`. This arm used to default an unfinalized
/// `conn_list` to conjunction unconditionally, and since the chart offers every split, a comma-only
/// PREFIX could fold as `∧` and then coordinate with the trailing `or` — `Or(And(A, B), D)`, a
/// connective the surface never supplied, in a list that had already said which one it was. That was 52
/// of the germline unit's skeletons when first measured.
///
/// Neither obvious repair works, and both were tried:
///
/// - **Deleting the arm** removes those readings but breaks ASYNDETIC coordination, which the suite
///   pins deliberately: `comma_list_coordination_parses` (`A, B affect X`) and the packed/unpacked
///   oracle's `HeLa, BRCA1 affect HeLa`. Conjunction is the RIGHT reading for a complete asyndetic list.
/// - **Resolving the comma's inherited connective in the coordination trigger** is inert: the trigger
///   sees one CELL, and `MSH2 , MSH6` is built in a cell that ends before the `or`. Measured
///   byte-identical; widening the scan to the sentence mis-inherits ("A, B affect X or Y").
///
/// The distinction the two cases actually turn on is COMPLETENESS, not inheritance, and that is
/// span-local: a list is final iff no comma or coordinator follows its cell
/// ([`super::RightContext::list_is_final`]). `A, B` of `A, B, C or D` is followed by `,` and does not
/// fold; `A, B` of `A, B affect X` is followed by `affect` and folds as `∧`. Both pinned asyndetic
/// tests keep passing, which is the check that killed the earlier attempts.
///
/// Attributive MODIFIER coordination is unaffected either way: it folds union-`Or` over the restrictors
/// without consulting `conn` at all (D63 §6, the `cat_mod` arm below).
pub fn complete_coord(
    coord_cat: &Exp,
    coord_sem: &Exp,
    layer: &Arc<Layer>,
    rctx: super::RightContext,
) -> Option<(Exp, Exp)> {
    let [base_cat, conn] = is_ctor(coord_cat, "cat_coord")? else {
        return None;
    };
    let members = group_members(coord_sem)?;
    if members.len() < 2 {
        return None;
    }
    // Attributive modifier coordination (D63 §6): fold UNION `Or` over the restrictors, pointwise —
    // `[λx. P₀ x, …, λx. Pₙ x]` → `λx. Or(…Or(P₀ x, P₁ x)…, Pₙ x)`. The surface connective is
    // irrelevant (union over kinds), so `conn` is not consulted here.
    if is_ctor(base_cat, "cat_mod").is_some() {
        let or = resolve_inductive(layer, "urn:eigenius:logic:Or")?;
        let var = "conj0";
        let app = |p: &Exp| Exp::App(Box::new(p.clone()), Box::new(Exp::Var(var.into())));
        let mut iter = members.into_iter();
        let mut acc = app(&iter.next()?);
        for m in iter {
            acc = Exp::InductiveType(or.clone(), vec![acc, app(&m)]);
        }
        let body = Exp::Lam(Patt::Var(var.into()), Box::new(acc));
        return Some((base_cat.clone(), body));
    }
    let op_iri = match conn_name_of(conn)? {
        "conn_and" => "urn:eigenius:logic:And",
        "conn_or" => "urn:eigenius:logic:Or",
        // A comma list folds as conjunction only when it is COMPLETE. A prefix of a list that
        // continues has no connective at all — see the `rctx` discussion above.
        "conn_list" if rctx.list_is_final() => "urn:eigenius:logic:And",
        _ => return None,
    };
    let denote = denote_cat(base_cat).ok()?;
    let op = resolve_inductive(layer, op_iri)?;
    let mut iter = members.into_iter();
    let mut acc = iter.next()?;
    for m in iter {
        acc = generalized_coord(&op, &denote, &acc, &m, 0)?;
    }
    Some((base_cat.clone(), acc))
}

/// A `List` cons-chain term over `members`: `cons(m₀, cons(m₁, … nil))`, the
/// kernel built-in [`list_decl`]. The element type is the inductive's *parameter*
/// (`peel_ctor_telescope` strips it), NOT a constructor field — so the ctors carry
/// only their fields (`cons(head, tail)`, `nil()`); the element type is inferred
/// from the check-mode expected type (`List C` at the consuming verb's slot).
fn list_term(members: &[Exp]) -> Exp {
    let mut acc = Exp::InductiveCtor(list_decl(), "nil".into(), vec![]);
    for m in members.iter().rev() {
        acc = Exp::InductiveCtor(list_decl(), "cons".into(), vec![m.clone(), acc]);
    }
    acc
}

/// The members of a group sem (a `List` cons-chain), in order. `None` if the sem
/// is not a well-formed `cons`/`nil` chain.
fn group_members(sem: &Exp) -> Option<Vec<Exp>> {
    let mut out = Vec::new();
    let mut cur = sem;
    loop {
        if let Some(args) = is_ctor(cur, "nil") {
            return args.is_empty().then_some(out);
        }
        let args = is_ctor(cur, "cons")?;
        if args.len() != 2 {
            return None;
        }
        out.push(args[0].clone());
        cur = &args[1];
    }
}

/// The `Prop`-connective IRI a group's `Conn` feature distributes with: `conn_and`
/// → `logic:And`, `conn_or` → `logic:Or`. Reads the `Conn` ctor from a `cat_group`
/// category's second argument.
///
/// A neutral [`LIST_CONN`] group (`conn_list`) has **no operator** — it is an UNFINALIZED list, and
/// this returns `None` so nothing can consume it. See [`complete_coord`] for why the former
/// default-to-conjunction was a bug rather than a fallback.
fn group_conn_op(group_cat: &Exp, rctx: super::RightContext) -> Option<&'static str> {
    let [_c, conn, _num] = is_ctor(group_cat, "cat_group")? else {
        return None;
    };
    match conn {
        Exp::InductiveCtor(_, n, _) if n == "conn_and" => Some("urn:eigenius:logic:And"),
        Exp::InductiveCtor(_, n, _) if n == "conn_or" => Some("urn:eigenius:logic:Or"),
        // An ASYNDETIC list folds as conjunction — but only if it is COMPLETE ("A, B affect X"). A
        // `conn_list` prefix of a list that continues ("A, B" of "A, B, C or D") has no connective at
        // all; see [`complete_coord`].
        Exp::InductiveCtor(_, n, _) if n == "conn_list" && rctx.list_is_final() => {
            Some("urn:eigenius:logic:And")
        }
        _ => None,
    }
}

/// The **neutral list connective** a comma contributes (D63 §8.4 Phase 6, Step 5b). English list
/// commas are polarity-neutral — `A, B, C or D` means `A ∨ B ∨ C ∨ D`, `A, B, C and D` means all-`∧`;
/// the comma inherits the list's FINAL explicit connective. So a comma builds a `conn_list` group that
/// the trailing `and`/`or` REBINDS (below). This is a PARSER-INTERNAL sentinel — never a logic op and
/// never a committed `lexicon:Conn` ctor (`denote_cat` erases the `Conn` argument, so it never reaches
/// the kernel), so no ontology change / reseed is needed.
///
/// A group still `conn_list` at fold time is UNFINALIZED and has no operator — [`group_conn_op`] and
/// [`complete_coord`] both return `None`, so it cannot be consumed. Neutrality is the whole point: if
/// the fold could read a connective off the comma, the comma would not be neutral, and in a list that
/// ends in `or` the read-off value contradicts the surface (see [`complete_coord`]).
pub(crate) const LIST_CONN: &str = "urn:eigenius:lexicon:conn_list";

/// Coordinate two NP-side constituents into a **group** (`cat_group(C, conn, pl)`
/// over a `List C` sem) under the connective `op_iri` (`logic:And`/`logic:Or`, or the neutral
/// [`LIST_CONN`] a comma contributes). Handles `NP·NP` (a fresh 2-member group) and `Group·NP`
/// (append, the left-branching n-ary case); the members are re-typed at the new common supertype
/// `C`. A **neutral `conn_list` left group** accepts ANY `op` — the trailing `and`/`or` rebinds the
/// whole group to `conn_and`/`conn_or` (list finalization); a FINALIZED (`and`/`or`) left group
/// requires `op` to match (no `X and Y or Z` mixing). Returns `(group_cat, group_sem)`, or `None` if
/// the constituents aren't NP/group, share no common type, mix finalized connectives, or `op_iri`
/// isn't a connective.
pub fn coordinate_np(
    op_iri: &str,
    l_cat: &Exp,
    l_sem: &Exp,
    r_cat: &Exp,
    r_sem: &Exp,
    layer: &Arc<Layer>,
) -> Option<(Exp, Exp)> {
    let conn_name = match op_iri {
        "urn:eigenius:logic:And" => "conn_and",
        "urn:eigenius:logic:Or" => "conn_or",
        LIST_CONN => "conn_list",
        _ => return None,
    };
    // The right conjunct is always a single NP (left-branching: a group never sits on the right —
    // enforced by the caller). A determined/named `cat_np` OR a bare-kind `cat_n`: coordination
    // LICENSES a bare kind as an argument even where a lone bare singular could not ("*gene is a
    // vulnerability" but "MSI and MMR deficiency create vulnerabilities"), so a kind conjunct is
    // realised as an entity via `kind_of`, matching the bare-nominal shift. Carries the shared
    // `cat`/`num` decls used to build the group.
    let (rt, r_member, cat_decl, num_decl) = np_conjunct(r_cat, r_sem)?;
    let (lt, members): (Exp, Vec<Exp>) = match l_cat {
        // A neutral `conn_list` left group takes ANY op (the trailing `and`/`or` rebinds it); a
        // finalized left group must share the op's connective (no `X and Y or Z` mixing).
        c if is_ctor(c, "cat_group").is_some() => {
            let left_conn = group_conn_name(c)?;
            if left_conn != "conn_list" && left_conn != conn_name {
                return None;
            }
            (is_ctor(c, "cat_group")?[0].clone(), group_members(l_sem)?)
        }
        // A single NP / bare kind starts a new group.
        _ => {
            let (lt, l_member, _, _) = np_conjunct(l_cat, l_sem)?;
            (lt, vec![l_member])
        }
    };
    // **A join at the `lexicon:Entity` TOP is not evidence of comparability, and refusing it is ONE
    // SENTENCE away from being correct** (measured 2026-07-26, reverted for coverage).
    //
    // Conjuncts sharing a semantic type join at THAT type — `CUI -> TUI` edges exist — so a ⊤ join
    // means the two share nothing narrower. That is how "Germline mutations in the MMR genes MSH2,
    // MSH6, PMS2 or MLH1 cause Lynch syndrome." coordinated a Germ-Line Mutation with three PROTEINS
    // at clause level, the last invalid family on that unit.
    //
    // Refusing a ⊤ join here scored, against the adopted recording:
    //   germline unit   9 -> 1   — ONLY the correct reading survives
    //   total-skeletons 281 -> 272,  encoded 10 -> 11
    //   grammar-gap     0 -> 1   ← the blocker, and it is exactly ONE unit
    //
    // The single casualty is the case [`np_conjunct`] already documents: "We hypothesized that MSI and
    // MMR deficiency may create vulnerabilities." — a phenomenon coordinated with a finding, which is
    // legitimate English and genuinely shares nothing narrower than `Entity` in this lexicon.
    //
    // So this is NOT "blocked on importing the UMLS semantic network". It is one named, tractable
    // knowledge gap: give `MSI` and `MMR deficiency` (or their semantic types) a common ancestor below
    // the top, and the refusal becomes shippable — worth ~9 skeletons and one `encoded` on this page,
    // and it removes a whole class of cross-kind coordination noise. Mind the 2026-07-11 lesson
    // (`d63-wordnet-umls-concept-unification.md` §2): lattice edges added wholesale broke parses and
    // were reverted, so the edge wanted here is a targeted one, not the ISA tree.
    // **EXPERIMENT (2026-07-28): conjunct-parallelism.** Refuse a coordination that pairs a
    // Σ-REFINED conjunct with an unrefined one. On the germline unit the invalid family coordinates
    // `Σx:Mutation. prep_in(x, …)` — the whole "germline mutations in the MMR genes MSH2" NP — with
    // three BARE gene names, which strands "germline mutations in" on the first disjunct and asserts
    // that MSH6/PMS2/MLH1 themselves cause Lynch syndrome. Measured against the `Entity`-top rule
    // this needs no semantic-type knowledge at all.
    // **Conjunct parallelism.** Refuse a coordination that pairs a Σ-REFINED member with a bare one.
    //
    // The germline unit's invalid family coordinates `kind_of(Σx:C0206530. … prep_in(x, the(MMR
    // genes MSH2)) …)` — the WHOLE "germline mutations in the MMR genes MSH2" NP — with three bare
    // gene names. The predicate then distributes, so "germline mutations in" is STRANDED on the
    // first disjunct and the reading asserts that MSH6/PMS2/MLH1 themselves cause Lynch syndrome.
    // False under every sense assignment: a gene does not cause the syndrome, mutations in it do.
    //
    // Read off the MEMBER SEMS, not the conjunct types: the types here are plain classes
    // (`C0206530`, `C1333234`, `T028`) and the refinement lives entirely in the sem, so a
    // type-level test cannot see it (measured — a `matches!(lt, Exp::Sig(..))` version never fired).
    // Consulting the sem is sanctioned for this rule: `Coordinate` is one of the two that declare
    // `reads_sem`, so its firing decision is already sem-dependent and packing carries the bit.
    fn refined_member(m: &Exp) -> bool {
        // `kind_of(Σ…)` — a kind realised from a REFINED common noun.
        match m {
            Exp::App(f, a) => {
                matches!(f.as_ref(), Exp::EigonAxiom(i) if i.as_str() == "urn:eigenius:ontology:kind_of")
                    && matches!(a.as_ref(), Exp::Sig(..))
            }
            _ => false,
        }
    }
    if members.iter().any(refined_member) != refined_member(&r_member) {
        return None;
    }
    let c = common_super(&lt, &rt, layer)?;
    let mut all = members;
    all.push(r_member);
    let conn = Exp::InductiveCtor(
        resolve_inductive(layer, "urn:eigenius:lexicon:Conn")?,
        conn_name.into(),
        vec![],
    );
    let pl = Exp::InductiveCtor(num_decl, "pl".into(), vec![]);
    let group_cat = Exp::InductiveCtor(cat_decl, "cat_group".into(), vec![c, conn, pl]);
    Some((group_cat, list_term(&all)))
}

/// **Modifier over a coordinated group** — `[cat_mod] [cat_group(C, conn, num)]` → a group whose
/// every member carries the modifier.
///
/// WHY IT IS NEEDED. RNR head distribution (`parse::seed::distribute_head`) emits, per conjunct, a
/// bare-kind `cat_np(C, num)` with sem `kind_of(C)`, which `coordinate_np` folds into a `cat_group`.
/// But `mod_apply`'s right pattern is `cat_n(C, num)` — a common noun. So a pre-nominal adjective
/// standing BEFORE a coordinated modifier list has no rule that can consume it:
///
/// ```text
///   "frequent insertion mutations"                 parses  (mod_apply over a cat_n)
///   "insertion or deletion mutations"              parses  (RNR -> a cat_group)
///   "frequent insertion or deletion mutations"     did NOT — nothing takes cat_mod + cat_group
/// ```
///
/// With no rule available, close apposition was the only remaining consumer of the stranded
/// adjective, and it produced `named(Frequently, kind_of(Insertion-Mutation))` — a KIND in the
/// naming-token slot, and the unit's ONLY reading. So the sentence had no admissible parse at all.
///
/// The distribution is the same move [`appose_group`] makes for a classifier: refine every member
/// rather than the group as a whole, and let the result ride the existing distributive machinery
/// unchanged. `frequent [insertion or deletion] mutations` ⟿
/// `Or(frequent-insertion-mutation, frequent-deletion-mutation)`, which is what the phrase means —
/// the adjective scopes over both conjuncts.
///
/// Deliberately NARROW: fires only when EVERY member is a bare kind `kind_of(K)`, i.e. exactly the
/// RNR-produced group. A group of named individuals or definite designations is left alone — an
/// adjective does not refine a name.
pub fn modify_group(
    mod_sem: &Exp,
    group_cat: &Exp,
    group_sem: &Exp,
    layer: &Arc<Layer>,
) -> Option<(Exp, Exp)> {
    is_ctor(group_cat, "cat_group")?;
    let members = group_members(group_sem)?;
    if members.is_empty() {
        return None;
    }
    let kind_of_iri = "urn:eigenius:ontology:kind_of";
    let mut out = Vec::with_capacity(members.len());
    for m in &members {
        // Every member must be `kind_of(K)`; anything else and the rule does not apply.
        let Exp::App(f, k) = m else { return None };
        if !matches!(f.as_ref(), Exp::EigonAxiom(i) if i.as_str() == kind_of_iri) {
            return None;
        }
        // `kind_of(Σx:K. restr(x))` — the restrictor left UN-REDUCED, as every modifier rule leaves
        // it (`is_adjective_refined` reads that shape to tell a modifier from an axiom restrictor).
        let x = "__modgrp_x";
        let restr = Exp::App(Box::new(mod_sem.clone()), Box::new(Exp::Var(x.into())));
        let sig = Exp::Sig(Patt::Var(x.into()), k.clone(), Box::new(restr));
        out.push(Exp::App(Box::new((**f).clone()), Box::new(sig)));
    }
    let _ = layer;
    // The group's type parameter is unchanged: each refined member is a Σ over its old base, hence
    // still below the group's common supertype.
    Some((group_cat.clone(), list_term(&out)))
}

/// One NP conjunct for [`coordinate_np`]: its **type**, its **entity sem**, and the shared `cat` /
/// `num` inductive decls. Handles a determined/named `cat_np` (sem is already an entity) and a bare
/// **kind** `cat_n` (its kind sem is realised as an entity via `kind_of` — the bare-nominal shift's
/// semantics — so coordinated bare kinds can be an argument). `None` for any other category.
fn np_conjunct(
    cat: &Exp,
    sem: &Exp,
) -> Option<(
    Exp,
    Exp,
    Arc<crate::nbe::term::InductiveDecl>,
    Arc<crate::nbe::term::InductiveDecl>,
)> {
    let Exp::InductiveCtor(cat_decl, n, args) = cat else {
        return None;
    };
    let ty = args.first()?.clone();
    let Exp::InductiveCtor(num_decl, _, _) = args.get(1)? else {
        return None;
    };
    match n.as_str() {
        "cat_np" => Some((ty, sem.clone(), cat_decl.clone(), num_decl.clone())),
        "cat_n" => {
            let kind_of = Exp::EigonAxiom(
                crate::ontology::iri::Iri::parse("urn:eigenius:ontology:kind_of").ok()?,
            );
            let entity = Exp::App(Box::new(kind_of), Box::new(sem.clone()));
            Some((ty, entity, cat_decl.clone(), num_decl.clone()))
        }
        _ => None,
    }
}

/// The raw `Conn` constructor name on a `cat_group` (`conn_and`/`conn_or`/`conn_but_not`).
fn group_conn_name(group_cat: &Exp) -> Option<&str> {
    let [_c, conn, _num] = is_ctor(group_cat, "cat_group")? else {
        return None;
    };
    match conn {
        Exp::InductiveCtor(_, n, _) => Some(n.as_str()),
        _ => None,
    }
}

/// Intuitionistic negation of a `Prop`: `prop → logic:False` (matching `closed-class.esl`'s
/// `neg_sem`, `λP.λs. P(s) → logic:False`). `None` if `logic:False` is unavailable.
fn negate(prop: Exp, layer: &Arc<Layer>) -> Option<Exp> {
    let f = resolve_inductive(layer, "urn:eigenius:logic:False")?;
    Some(Exp::Arrow(
        Box::new(prop),
        Box::new(Exp::InductiveType(f, vec![])),
    ))
}

/// Coordinate two NPs into a **contrastive `but not` group** `cat_group(C, conn_but_not, pl)`
/// (D62 §2 #8): `[O₁] but not [O₂]`. Binary (no n-ary chaining) — the second member is the
/// negated/elided one. The shared predicate is applied downstream by [`distribute`] /
/// [`distribute_object`], which negate every member after the first. `None` unless both sides are
/// `cat_np` sharing a common supertype.
pub fn coordinate_but_not(
    l_cat: &Exp,
    l_sem: &Exp,
    r_cat: &Exp,
    r_sem: &Exp,
    layer: &Arc<Layer>,
) -> Option<(Exp, Exp)> {
    let [lt, _ln] = is_ctor(l_cat, "cat_np")? else {
        return None;
    };
    let [rt, _rn] = is_ctor(r_cat, "cat_np")? else {
        return None;
    };
    let Exp::InductiveCtor(cat_decl, _, _) = l_cat else {
        return None;
    };
    let c = common_super(lt, rt, layer)?;
    let conn = Exp::InductiveCtor(
        resolve_inductive(layer, "urn:eigenius:lexicon:Conn")?,
        "conn_but_not".into(),
        vec![],
    );
    let num_decl = resolve_inductive(layer, "urn:eigenius:lexicon:Num")?;
    let pl = Exp::InductiveCtor(num_decl, "pl".into(), vec![]);
    let group_cat = Exp::InductiveCtor(cat_decl.clone(), "cat_group".into(), vec![c, conn, pl]);
    Some((group_cat, list_term(&[l_sem.clone(), r_sem.clone()])))
}

/// **Close nominal apposition** (D63 §8.4 Phase 6, RC-6): a definite/bare common-noun HEAD
/// immediately followed by a coreferential **name-group** — "the genes BRCA1 and MSH2", "the MMR
/// genes MSH2, MSH6, PMS2 or MLH1". In close apposition the head noun *classifies* the referents and
/// the named group *specifies* them, so the rule **distributes the classifier over the members**: each
/// designator `dᵢ` becomes the individual the classifier + that name picks out,
/// `the(Σx:classifier. named(x, dᵢ)).1`, and the result is a group of those at the classifier's base
/// class. The group then rides the existing distributive-subject / distributive-object machinery
/// unmodified (`distribute` / `distribute_object`), so "the genes BRCA1 and MSH2 affect cells" ⟿
/// `affect(cells, the(Σx:gene. named(x, brca1)).1) ∧ affect(cells, the(Σx:gene. named(x, msh2)).1)`.
///
/// **It used to pass the group through unchanged (until 2026-07-26), and that dropped the classifier.**
/// Two defects, both on the reference page's "Germline mutations in the MMR genes MSH2, MSH6, PMS2 or
/// MLH1 cause Lynch syndrome.":
///
/// - The classifier's own refinement went nowhere. The head is `Σg:Gene. compound_kind(g, MMR)`; a
///   pass-through keeps only the *members'* lexical types, so "MMR" left no trace in any reading and
///   no reading said the four genes are MMR genes. The maximum `named` count across that unit's 112
///   skeletons was **2** — the analysis typing all four designators did not exist in the forest.
/// - It made the DESIGNATOR's denotation the referent, which is exactly what
///   [`super::combinators`]'s `name` rule does not do for a single designator. `named` takes the
///   designator as a naming TOKEN ([`NAMED_AXIOM`]) precisely so that a name with an unrelated
///   lexical sense still names felicitously; a pass-through instead refers to Achilles the hero. The
///   arity of the designator list is not a reason for a different analysis, so both arities now build
///   the same [`naming_refinement`].
///
/// `head_cat` is a **subject GQ** `S/(S\NP_C)` (determined: "the genes") or a **bare common noun**
/// `cat_n(C, _)` (bare: "genes"); `group_cat` a `cat_group(D, conn, num)`. The felicity gate compares
/// the head's **base** class `⌊C⌋` (any Σ-refinement peeled — "MMR genes" is `Σx:Gene.
/// compound_kind(x, MMR)`, and whether each name is specifically an *MMR* gene is what the apposition
/// ASSERTS, not a precondition) with the group's base member type `⌊D⌋`, and passes iff **one subsumes
/// the other, EITHER direction**. Bidirectionality is required by cross-importer typing: a named
/// individual carries its broad UMLS **semantic type** (`umlssty:T028` "Gene or Genome"), while a
/// common noun carries its narrower **concept** (`umlscui:C0017337` "gene", emitted `: umlssty:T028`,
/// i.e. `C0017337 ≤ T028`). So "the genes BRCA1 and MSH2" has `⌊head⌋ = C0017337 ≤ T028 = ⌊D⌋` — the
/// head is a SUBTYPE of the members' type, not a supertype; a one-directional `⌊D⌋ ≤ ⌊C⌋` gate would
/// reject it. The check still rejects a genuine kind clash: "the cells BRCA1 and MSH2" has `⌊head⌋` a
/// cell concept and `⌊D⌋ = T028`, neither subsuming the other (UMLS semantic types are siblings under
/// `Entity`) ⇒ no parse. `None` if the shapes don't match or the gate fails.
///
/// The **group's type index is the classifier's base class** `⌊C⌋`, not the full refinement, matching
/// [`definite_designation`]: the index is what downstream selectional checks read
/// ([`type_subsumes`] compares classes and falls back to equality), while the refinement is semantic
/// content and travels in the members' sems. The head determiner's definiteness beyond the ι each
/// member now carries is still dropped — a first-cut approximation, parallel to the existential
/// treatment of `the` (a faithfulness refinement, D61).
pub fn appose_group(
    head_cat: &Exp,
    group_cat: &Exp,
    group_sem: &Exp,
    layer: &Arc<Layer>,
) -> Option<(Exp, Exp)> {
    let classifier = appositive_head_type(head_cat)?;
    // A classifier that is ITSELF already designated ("genes MSH2") takes no further designator list:
    // the second naming would sit inside the first's Σ. Cat-only, so the packed forest's `Sig` (which
    // already carries the designation bit) separates the two heads.
    if is_naming_refined(classifier) || is_pp_refined(classifier) {
        return None;
    }
    let head_ty = sigma_base(classifier);
    let [group_ty, conn, num] = is_ctor(group_cat, "cat_group")? else {
        return None;
    };
    let group_ty = sigma_base(group_ty);
    if !appositive_kinds_match(head_ty, group_ty, layer) {
        return None;
    }
    let Exp::InductiveCtor(decl, _, _) = group_cat else {
        return None;
    };
    // Distribute: every designator is classified, not just the first one a `[classifier designator]`
    // bracketing would reach.
    let members: Vec<Exp> = group_members(group_sem)?
        .iter()
        .map(|d| definite_individual(&naming_refinement(classifier, d)))
        .collect::<Option<_>>()?;
    let cat = Exp::InductiveCtor(
        decl.clone(),
        "cat_group".into(),
        vec![head_ty.clone(), conn.clone(), num.clone()],
    );
    Some((cat, list_term(&members)))
}

/// Whether a classifier and a designator group name the **same kind**, as far as the lexicon can say.
///
/// Subsumption in EITHER direction is felicitous, bridging the concept-vs-semantic-type granularity
/// gap *within* a vocabulary (`umlscui:C0017337 ≤ umlssty:T028`, so "the genes BRCA1 and MSH2" passes
/// with the head a SUBtype of the members' type).
///
/// A **cross-vocabulary** pair is also felicitous, because the absence of an edge between two
/// vocabularies is not evidence about kinds — the lattice is *specified* to have none.
/// `docs/notes/d63-wordnet-umls-concept-unification.md` §2: the WordNet↔UMLS alignment canonicalizes
/// by redefining which class an *entry* denotes and "adds **zero** new `subclass_of` edges … The
/// alignment must be inert for the lattice", after a 2026-07-11 branch that did add them (a supersense
/// parent plus the UMLS TUI ISA tree) broke parses and was reverted. So a common noun with an aligned
/// counterpart denotes a `wn:` class while a UMLS-only named individual keeps its `umlssty:` type
/// ("MSH2" has no synset, hence no pair), and [`type_subsumes`] across the two is false whether or not
/// the kinds match.
///
/// That is what left the reference page's germline unit with **no correct reading**: the properly
/// bracketed analysis — classifier `Σx:wn:n05436752. compound_kind(x, C1155661)` ("MMR genes") over
/// `cat_group(umlssty:T116)` — was refused 138 times on a single sentence (traced 2026-07-26), while
/// every apposition that DID fire had a group typed at the `lexicon:Entity` top, where the first branch
/// passes vacuously whatever the classifier is (that is how a MUTATION classified genes).
///
/// The permissiveness is scoped to **exactly the one pair whose absence is guaranteed** — WordNet
/// against UMLS. Everywhere else an absent edge is still informative and still rejects: two UMLS types
/// (a cell concept against a protein semantic type), and any hand-authored pair inside one layer chain,
/// where nothing stops the author from relating the classes and not relating them therefore means
/// something. A blanket "different namespaces ⇒ pass" is NOT equivalent and is wrong: it admitted the
/// `demo:WidgetKind`-group-against-gene-head clash that
/// `close_apposition_bridges_concept_and_semantic_type_granularity` pins as infelicitous.
fn appositive_kinds_match(head_ty: &Exp, group_ty: &Exp, layer: &Arc<Layer>) -> bool {
    if type_subsumes(head_ty, group_ty, layer) || type_subsumes(group_ty, head_ty, layer) {
        return true;
    }
    match (head_ty, group_ty) {
        (Exp::EigonClass(a), Exp::EigonClass(b)) => spans_wordnet_and_umls(a, b),
        _ => false,
    }
}

/// Whether two classes sit on opposite sides of the **WordNet/UMLS divide** — the one vocabulary
/// boundary the alignment is specified never to cross with a `subclass_of` edge, so an absent edge
/// across it is not evidence of a kind clash.
fn spans_wordnet_and_umls(a: &Iri, b: &Iri) -> bool {
    let (va, vb) = (vocabulary(a), vocabulary(b));
    matches!(
        (va, vb),
        (Some("wn"), Some("umls")) | (Some("umls"), Some("wn"))
    )
}

/// The imported **vocabulary** a class belongs to, or `None` for anything else (core, `lexicon:`, a
/// test/domain namespace). `umlscui:`/`umlssty:` are ONE vocabulary — concepts and their semantic types
/// are linked (`C0017337 ≤ T028`), so an absent edge between them is informative.
fn vocabulary(iri: &Iri) -> Option<&'static str> {
    match iri.namespace() {
        "urn:eigenius:wn:" => Some("wn"),
        "urn:eigenius:umlscui:" | "urn:eigenius:umlssty:" => Some("umls"),
        _ => None,
    }
}

/// The classifying **type index** of a close-apposition head: a subject GQ `S/(S\NP_C)` (a determined
/// head, "the genes") yields `C`; a bare common noun `cat_n(C, _)` yields `C`. A transitive verb
/// `(S\NP)/NP` or a preposition `cat_pp/cat_np` never matches — their `fwd` ARGUMENT is a bare
/// `cat_np` (object / prep-object), not a `S\NP` VP, so the inner `bwd` probe fails. `None` otherwise.
fn appositive_head_type(head_cat: &Exp) -> Option<&Exp> {
    // Determined subject GQ  S/(S\NP_C) = fwd(S, bwd(S, cat_np(C, _))): the ARGUMENT (arg 1) is the VP.
    if let Some((_m, _result, arg)) = slash_parts(head_cat, "fwd") {
        if let Some((_am, _s, np)) = slash_parts(arg, "bwd") {
            if let Some([ty, _num]) = is_ctor(np, "cat_np") {
                return Some(ty);
            }
        }
    }
    // Bare common noun  cat_n(C, _).
    if let Some([ty, _num]) = is_ctor(head_cat, "cat_n") {
        return Some(ty);
    }
    None
}

/// The base class under any Σ-refinements: `Σx:C. φ → ⌊C⌋` (recursively), else the type itself. A
/// compound / attributive / relative noun refines a base class with a Σ ("MMR genes" = `Σx:Gene.
/// compound_kind(x, MMR)`); apposition's felicity checks the named members against that BASE class.
fn sigma_base(ty: &Exp) -> &Exp {
    match ty {
        Exp::Sig(_, comp, _) => sigma_base(comp),
        other => other,
    }
}

/// Left-fold a non-empty list of `Prop`s with the connective `op` (`logic:And` /
/// `logic:Or`): `op(op(p₀, p₁), p₂)…` — the left-branching coordination normal
/// form. `None` if `preds` is empty.
fn fold_conn(op: &Arc<InductiveDecl>, preds: Vec<Exp>) -> Option<Exp> {
    if std::env::var("EIGENIUS_TRACE_COORD").is_ok()
        && preds.iter().any(|p| matches!(p, Exp::Lam(..)))
    {
        eprintln!(
            "  !! fold_conn over {} preds, at least one a Lam",
            preds.len()
        );
    }

    let mut iter = preds.into_iter();
    let mut acc = iter.next()?;
    for p in iter {
        acc = Exp::InductiveType(op.clone(), vec![acc, p]);
    }
    Some(acc)
}

/// The **distributive subject** reading: a group meeting a one-place predicate `P`
/// maps `P` over the members and folds with the group's connective (D63 §8.4
/// Phase 6) — `P(m₀) ⊕ P(m₁) ⊕ …` (⊕ = ∧ for `and`, ∨ for `or`). The members are
/// statically known (a literal coordination), so the map/fold is computed here,
/// yielding the bare connective chain (no `List`/`Reduce` residue). `None` for an
/// ill-formed group or an unresolvable connective.
pub fn distribute(
    group_cat: &Exp,
    group_sem: &Exp,
    pred_sem: &Exp,
    layer: &Arc<Layer>,
    rctx: super::RightContext,
) -> Option<Exp> {
    let members = group_members(group_sem)?;
    // Contrastive `but not` (D62 §2 #8): `P(m₀) ∧ ¬P(m₁) ∧ …` — first positive, rest negated,
    // ∧-folded. Otherwise the symmetric `conn_and`/`conn_or` fold.
    if group_conn_name(group_cat) == Some("conn_but_not") {
        let and = resolve_inductive(layer, "urn:eigenius:logic:And")?;
        let preds = but_not_preds(
            members,
            |m| Exp::App(Box::new(pred_sem.clone()), Box::new(m)),
            layer,
        )?;
        return fold_conn(&and, preds);
    }
    let op = resolve_inductive(layer, group_conn_op(group_cat, rctx)?)?;
    let preds = members
        .into_iter()
        .map(|m| Exp::App(Box::new(pred_sem.clone()), Box::new(m)))
        .collect();
    fold_conn(&op, preds)
}

/// Apply `mk` to each group member, negating every member AFTER the first — the `conn_but_not`
/// distribution (D62 §2 #8). `mk` builds the affirmative predicate-application for a member.
fn but_not_preds(
    members: Vec<Exp>,
    mk: impl Fn(Exp) -> Exp,
    layer: &Arc<Layer>,
) -> Option<Vec<Exp>> {
    members
        .into_iter()
        .enumerate()
        .map(|(idx, m)| {
            let p = mk(m);
            if idx == 0 {
                Some(p)
            } else {
                negate(p, layer)
            }
        })
        .collect()
}

/// The **distributive object** reading: a transitive verb `V : obj → subj → Prop`
/// (object-first) applied to a group object yields a VP `λs. V(m₀, s) ⊕ V(m₁, s) ⊕
/// …` — the predicate distributed over the object members and folded with the
/// group's connective (D63 §8.4 Phase 6). `None` for an ill-formed group or an
/// unresolvable connective.
pub fn distribute_object(
    group_cat: &Exp,
    group_sem: &Exp,
    tv_sem: &Exp,
    layer: &Arc<Layer>,
    rctx: super::RightContext,
) -> Option<Exp> {
    let members = group_members(group_sem)?;
    let s = Exp::Var("__dist_subj".into());
    let mk = |m: Exp| {
        Exp::App(
            Box::new(Exp::App(Box::new(tv_sem.clone()), Box::new(m))),
            Box::new(s.clone()),
        )
    };
    // Contrastive `but not` (D62 §2 #8): `V(m₀,s) ∧ ¬V(m₁,s) ∧ …`. Otherwise the symmetric fold.
    let body = if group_conn_name(group_cat) == Some("conn_but_not") {
        let and = resolve_inductive(layer, "urn:eigenius:logic:And")?;
        fold_conn(&and, but_not_preds(members, mk, layer)?)?
    } else {
        let op = resolve_inductive(layer, group_conn_op(group_cat, rctx)?)?;
        fold_conn(&op, members.into_iter().map(mk).collect())?
    };
    Some(Exp::Lam(Patt::Var("__dist_subj".into()), Box::new(body)))
}

/// The **reciprocal** reading "[group] V each other" (D63 §8.4 Phase 6): the
/// transitive verb `V` related over every **ordered distinct** pair of group
/// members, ∧-conjoined — `⋀_{i≠j} V(mⱼ, mᵢ)` ("mᵢ V's mⱼ"; `V` is object-first, so
/// the object `mⱼ` is applied first). For `[m₀, m₁]`: `V(m₁, m₀) ∧ V(m₀, m₁)`. A
/// reciprocal is conjunctive by nature, so it applies to **`and`-groups only**, and
/// needs ≥2 members. `tv_cat` must be a transitive verb `(S\NP)/NP`; the result is
/// its `S`. Members statically known ⇒ pairs enumerated here. `None` otherwise.
pub fn reciprocate(
    group_cat: &Exp,
    group_sem: &Exp,
    tv_cat: &Exp,
    tv_sem: &Exp,
    layer: &Arc<Layer>,
) -> Option<(Exp, Exp)> {
    // Reciprocity is inherently conjunctive — `and`-groups only.
    if group_conn_op(group_cat, super::RightContext::Other)? != "urn:eigenius:logic:And" {
        return None;
    }
    let members = group_members(group_sem)?;
    if members.len() < 2 {
        return None;
    }
    // `tv_cat` must be a transitive verb `(S\NP)/NP` = fwd(bwd(S, subj), obj); the
    // reciprocal sentence's category is that inner `S`.
    let (_m, vp, _obj) = slash_parts(tv_cat, "fwd")?;
    let (_vm, result, _subj) = slash_parts(vp, "bwd")?;
    let and = resolve_inductive(layer, "urn:eigenius:logic:And")?;
    let mut preds = Vec::new();
    for (i, subj) in members.iter().enumerate() {
        for (j, obj) in members.iter().enumerate() {
            if i == j {
                continue; // distinct pairs only — no self-relation
            }
            // "mᵢ V's mⱼ": object-first `V(obj=mⱼ)(subj=mᵢ)`.
            preds.push(Exp::App(
                Box::new(Exp::App(Box::new(tv_sem.clone()), Box::new(obj.clone()))),
                Box::new(subj.clone()),
            ));
        }
    }
    let sem = fold_conn(&and, preds)?;
    Some((result.clone(), sem))
}

/// Bare-plural → **kind-subject** shift (D63 §8.5 Slice 3c, kind subjects): a plural
/// common noun `cat_n(C, pl)` used bare denotes the **kind C** — a type-valued NP
/// `cat_kind` (⟦·⟧ = `Set`) whose sem is the class `C` itself. A kind predicate
/// (`are cell lines`) then relates it via `subclass_of`. `None` for non-plural /
/// non-noun (a singular bare noun is not a kind subject — it needs a determiner).
pub fn kind_subject(cat: &Exp, sem: &Exp) -> Option<(Exp, Exp)> {
    let Exp::InductiveCtor(decl, name, args) = cat else {
        return None;
    };
    if name != "cat_n" || args.len() != 2 {
        return None;
    }
    let Exp::InductiveCtor(_, num, _) = &args[1] else {
        return None;
    };
    if num != "pl" {
        return None;
    }
    Some((
        Exp::InductiveCtor(decl.clone(), "cat_kind".into(), vec![]),
        sem.clone(),
    ))
}

/// **Definite designation** (D63 §5.3, the bare half of the classifier+designator construction): a
/// common noun refined by a NAMING restrictor, `cat_n(Σx:C. named(x, d), num)`, shifts to the
/// individual that Σ uniquely picks out — `cat_np(C, num)` with sem `the(Σ…).1`, the ι operator
/// (`ontology:the`) applied and `Fst`-projected, the same shape the definite-NP pins carry.
///
/// This is the other half of moving [`super::combinators`]'s `name` rule from `cat_np` to `cat_n`
/// (2026-07-26), and neither half works alone. The rule used to jump straight to `cat_np`, which put
/// the whole construction out of a determiner's reach — `the`'s argument is a `cat_n` — so a DETERMINED
/// classifier + designator had no apposition route at all and fell through to the compound-noun
/// analysis, which heads the phrase on the NAME: `The gene MSH2 affects cells.` had exactly two
/// readings and both were `the(Σ G:C1333234. compound_kind(G, n05436752)).1`, "an MSH2 of the gene
/// kind". Emitting the refined noun instead lets `the` apply through the existing
/// determiner-over-refined-noun `Fst` machinery; this shift restores the bare use (`project Achilles`),
/// which no longer gets its `cat_np` from the rule directly.
///
/// **The naming restrictor is what licenses the shift.** A definite needs uniqueness, and `named(x, d)`
/// supplies it; a compound / adjective / PP restrictor does not, so those refined nouns keep needing a
/// real determiner. Hence the positive test for `ontology:named` rather than "any refined noun".
///
/// Fires only on a DIRECTLY-named Σ, so a further-modified appositive ("gene MSH2 in humans", whose
/// outer restrictor is the `prep_in`) is not covered — reachable now that the construction is a `cat_n`
/// at all, but a separate step.
pub fn definite_designation(cat: &Exp) -> Option<(Exp, Exp)> {
    let [ty, num] = is_ctor(cat, "cat_n")? else {
        return None;
    };
    if !is_naming_refined(ty) {
        return None;
    }
    let (Exp::Sig(_, base, _), Exp::InductiveCtor(decl, _, _)) = (ty, cat) else {
        return None;
    };
    let sem = definite_individual(ty)?;
    let cat = Exp::InductiveCtor(
        decl.clone(),
        "cat_np".into(),
        vec![(**base).clone(), num.clone()],
    );
    Some((cat, sem))
}

/// The opaque naming relation `ontology:named : Entity → Entity → Prop` — its second argument is the
/// designator's referent used as a naming TOKEN, not as the phrase's referent, which is why a name
/// whose lexical sense is unrelated (`Achilles` the hero) still felicitously names a Project.
const NAMED_AXIOM: &str = "urn:eigenius:ontology:named";

/// The **naming refinement** `Σx:classifier. named(x, designator)` — the one shape the
/// classifier+designator construction builds, in every arity. [`super::combinators`]'s `name` rule
/// builds it for a single designator ("gene MSH2"), [`appose_group`] builds one per member for a
/// designator GROUP ("the MMR genes MSH2, MSH6, PMS2 or MLH1"), and [`definite_designation`] /
/// [`is_naming_restrictor`] recognize it. Bound variable is the compound family's
/// [`super::combinators::COMPOUND_X`], so a refinement is a single canonical term rather than one
/// α-variant per construction site (the reading-dedup key is the term).
pub(crate) fn naming_refinement(classifier: &Exp, designator: &Exp) -> Exp {
    let named = Exp::EigonAxiom(Iri::parse(NAMED_AXIOM).expect("valid named axiom iri"));
    let x = super::combinators::COMPOUND_X;
    let restr = Exp::App(
        Box::new(Exp::App(Box::new(named), Box::new(Exp::Var(x.into())))),
        Box::new(designator.clone()),
    );
    Exp::Sig(
        Patt::Var(x.into()),
        Box::new(classifier.clone()),
        Box::new(restr),
    )
}

/// The **individual a uniquely-satisfied description picks out**: `the(Σ).1`, the ι operator
/// (`ontology:the`) applied to the refinement and `Fst`-projected — the shape the definite-NP pins
/// carry. `None` only if `ontology:the` is unavailable.
fn definite_individual(sigma: &Exp) -> Option<Exp> {
    let the = Exp::EigonAxiom(Iri::parse("urn:eigenius:ontology:the").ok()?);
    Some(Exp::Fst(Box::new(Exp::App(
        Box::new(the),
        Box::new(sigma.clone()),
    ))))
}

/// Whether a classifier type carries a **postmodifying PP** — a `prep_*` relation in its OWN
/// restrictor, at any depth short of a nested Σ (which belongs to some other noun).
///
/// This DISQUALIFIES a close-apposition classifier, on adjacency: the designator sits immediately
/// after the nominal head, so a PP postmodifier cannot intervene between them — "the gene MSH2 in
/// humans", never "*the gene in humans MSH2". Without the check the reference page's germline unit
/// bracketed "[Germline mutations in the MMR] [genes MSH2, MSH6, PMS2 or MLH1]" and classified four
/// genes as germline mutations, glossing "the a Germ-Line Mutation in the Mismatch Repair named MSH6
/// protein cause …". The felicity gate cannot catch it: every pair of UMLS classes in this lexicon
/// joins at the `lexicon:Entity` top (measured — `probe_cross_kind_np_coordination`), so
/// `type_subsumes(Entity, mutation)` holds and the gate passes vacuously whatever the classifier.
///
/// **A spine walk is not enough, and the difference was measured.** A PP whose object is a raised GQ
/// arrives un-β-reduced with the *verb* variable at the spine head and the preposition buried in an
/// argument — traced live, the classifier is
/// `Σ__cmp_x:C0206530. λ__pobj_x. λA. λV. V(the(A))(C1155661, λ__pobj_y. λy. λx. prep_in(x, y)(…))(__cmp_x)`,
/// whose spine head is `V`. Walking `App`/`Ann`/`Lam` heads (the [`is_naming_restrictor`] /
/// `is_adjective_refined` idiom) therefore matched nothing and the check was page-neutral: 522
/// skeletons before and after, byte-identical per unit. Hence a subterm search — stopped at a nested
/// `Sig` so a PP inside a compound's MODIFIER ("the MMR-pathway genes MSH2") is not mistaken for a
/// postmodifier of this classifier.
pub(crate) fn is_pp_refined(ty: &Exp) -> bool {
    let Exp::Sig(_, _, restr) = ty else {
        return false;
    };
    fn mentions_prep(e: &Exp) -> bool {
        match e {
            Exp::EigonAxiom(iri) => iri.as_str().starts_with("urn:eigenius:ontology:prep_"),
            // A nested refined noun is a different noun's restrictor — do not descend.
            Exp::Sig(..) => false,
            Exp::App(a, b)
            | Exp::Pi(_, a, b)
            | Exp::Arrow(a, b)
            | Exp::Times(a, b)
            | Exp::Pair(a, b)
            | Exp::Ann(a, b) => mentions_prep(a) || mentions_prep(b),
            Exp::Lam(_, b) | Exp::Fst(b) | Exp::Snd(b) | Exp::Con(_, b) | Exp::Refl(b) => {
                mentions_prep(b)
            }
            Exp::InductiveType(_, args) | Exp::InductiveCtor(_, _, args) => {
                args.iter().any(mentions_prep)
            }
            Exp::Id(a, b, c) | Exp::DecEq(a, b, c) => {
                mentions_prep(a) || mentions_prep(b) || mentions_prep(c)
            }
            _ => false,
        }
    }
    mentions_prep(restr)
}

/// Whether a type is a **naming refinement** — the `Σx:C. named(x, d)` [`naming_refinement`] builds,
/// i.e. a description already made unique by a designator. It licenses [`definite_designation`], and
/// it DISQUALIFIES a classifier in [`appose_group`]: what is already designated takes no second
/// designator.
fn is_naming_refined(ty: &Exp) -> bool {
    matches!(ty, Exp::Sig(Patt::Var(x), _, restr) if is_naming_restrictor(restr, x))
}

/// Whether a refined noun's restrictor is a **naming** of its own bound variable — `named(x, d)`, the
/// shape [`naming_refinement`] builds. Positive match on `ontology:named` over the Σ's variable, so a
/// compound / adjective / PP restrictor (which does not make the description unique) never licenses
/// [`definite_designation`].
fn is_naming_restrictor(restr: &Exp, x: &str) -> bool {
    let mut head = restr;
    let mut args: Vec<&Exp> = Vec::new();
    while let Exp::App(f, a) = head {
        args.push(a);
        head = f;
    }
    args.reverse();
    matches!(head, Exp::EigonAxiom(i) if i.as_str() == NAMED_AXIOM)
        && args.len() == 2
        && matches!(args[0], Exp::Var(v) if v == x)
}

/// Forward **bounded type-raising** `T` (D63 §8.9 Slice 6-T): an `NP_X` (a plain
/// `cat_np(X, num)` — a name; determined NPs are already lexically raised) lifts to
/// `S/(S\NP_X)` over the fixed target `S = cat_s(dcl, fin)` — the bound that makes the
/// unary closure terminating. The sem is `λV. V(x)` (apply the to-be-supplied VP to
/// the raised NP's witness). Returns `(raised_cat, raised_sem)`; the caller tags the
/// item `Combinator::TypeRaised`, so ENF lets it only **compose** (the object-gap
/// `S/NP` of a relative clause body), never forward-*apply*. `None` for a non-`NP`
/// (functors, groups, kinds, already-raised determiner NPs are not raised here).
pub fn type_raise(cat: &Exp, sem: &Exp, layer: &Arc<Layer>) -> Option<(Exp, Exp)> {
    let Exp::InductiveCtor(cat_decl, name, args) = cat else {
        return None;
    };
    if name != "cat_np" || args.len() != 2 {
        return None;
    }
    // The fixed target `S = cat_s(dcl, fin)` — a finite declarative clause (the body
    // of a restrictive relative). `Mood`/`Fin` are sibling inductives, resolved from
    // the layer (as `coordinate_np` resolves `Conn`); `cat_s`/`fwd`/`bwd` reuse the
    // `cat_np`'s own `Cat` decl.
    let mood = resolve_inductive(layer, "urn:eigenius:lexicon:Mood")?;
    let fin = resolve_inductive(layer, "urn:eigenius:lexicon:Fin")?;
    let s = Exp::InductiveCtor(
        cat_decl.clone(),
        "cat_s".into(),
        vec![
            Exp::InductiveCtor(mood, "dcl".into(), vec![]),
            Exp::InductiveCtor(fin, "fin".into(), vec![]),
        ],
    );
    // Type-raising `X ⇒T Y/i(Y\i X)` (Baldridge (196)) — the raised slashes take the permissive
    // `m_all`, matching the pre-multimodal regime; `i` is a mode VARIABLE in the source rule.
    let m_all = crate::dcg::category::mode_value(layer, crate::dcg::category::MODE_ALL)?;
    let vp = Exp::InductiveCtor(
        cat_decl.clone(),
        "bwd".into(),
        vec![m_all.clone(), s.clone(), cat.clone()],
    );
    let raised_cat = Exp::InductiveCtor(cat_decl.clone(), "fwd".into(), vec![m_all, s, vp]);
    let v = "__tr_v";
    let raised_sem = Exp::Lam(
        Patt::Var(v.into()),
        Box::new(Exp::App(
            Box::new(Exp::Var(v.into())),
            Box::new(sem.clone()),
        )),
    );
    Some((raised_cat, raised_sem))
}

/// The **relativizer** refine rule (D63 §8.9 Slice 6-rel): a common noun `cat_n(C,
/// num)` modified by a restrictive relative clause `[noun] that [body]` → the refined
/// noun `cat_n(Σx:C. body(x), num)`. The `body` is the relative clause's gap-abstracted
/// predicate — a subject relative VP `S\NP` ("that affects HeLa", sem `λx. affects(hela,
/// x)`) or an object relative `S/NP` ("that HeLa affects", built by `T`+`>B`, sem `λx.
/// affects(x, hela)`); both have sem `body : X → Prop`, so one rule covers them. The Σ
/// is built over the **concrete** `C` (so `body(x)` type-checks directly — the same
/// engine-level move as 3b's attributive Σ, dodging the abstract-`C` bounded-
/// quantification kernel gap). The refined noun then rides 3b's determiner-over-
/// refined-noun `Fst` machinery unchanged. `None` if the noun is not a `cat_n` or the
/// body is not a declarative-clause `S/NP` / `S\NP`.
pub fn relativize(noun_cat: &Exp, body_cat: &Exp, body_sem: &Exp) -> Option<(Exp, Exp)> {
    let [c, num] = is_ctor(noun_cat, "cat_n")? else {
        return None;
    };
    let Exp::InductiveCtor(decl, _, _) = noun_cat else {
        return None;
    };
    // The body is a clause missing one NP: `S/NP` (object relative) or `S\NP`
    // (subject relative), whose result `S` is a finite declarative clause.
    let (_bm, s, _np) = slash_parts(body_cat, "fwd").or_else(|| slash_parts(body_cat, "bwd"))?;
    if !is_decl_clause(s) {
        return None;
    }
    let x = "__rel_x";
    let sigma = Exp::Sig(
        Patt::Var(x.into()),
        Box::new(c.clone()),
        Box::new(Exp::App(
            Box::new(body_sem.clone()),
            Box::new(Exp::Var(x.into())),
        )),
    );
    let cat = Exp::InductiveCtor(
        decl.clone(),
        "cat_n".into(),
        vec![sigma.clone(), num.clone()],
    );
    Some((cat, sigma))
}

/// **Pied-piping** restrictive relativizer (D62 §2 #2B): `[noun] [prep] which [subject] [VP]`
/// ("the gene in which HeLa affects BRCA1", "the interaction through which the co-occurrence leads
/// to cell death") → the refined noun `cat_n(Σg:C. prep(g)(VP)(subj), num)`, i.e. the antecedent is
/// the FRONTED preposition's object, threaded into the clause as a VP-adjunct: with the VP-adjunct
/// prep sem `λx.λV.λs. And(V(s), prep(s,x))`, the restrictor is `And(VP(subj), prep(subj, g))`.
/// Reuses the VP-adjunct preposition's own sem (no PP-gap extraction / crossed-composition needed),
/// then rides the determiner-over-refined-noun `Fst` machinery. `prep_sem` is the VP-adjunct prep,
/// `subj_sem` the relative-clause subject, `vp_sem` its `S\NP` predicate. `None` if the antecedent
/// is not a `cat_n`.
pub fn pied_pipe(
    noun_cat: &Exp,
    prep_sem: &Exp,
    subj_sem: &Exp,
    vp_sem: &Exp,
) -> Option<(Exp, Exp)> {
    let [c, num] = is_ctor(noun_cat, "cat_n")? else {
        return None;
    };
    let Exp::InductiveCtor(decl, _, _) = noun_cat else {
        return None;
    };
    let g = "__pied_g";
    // restr(g) = prep_sem(g)(vp)(subj) = And(vp(subj), prep(subj, g)) — the VP-adjunct prep sem
    // builds the conjunction; the antecedent `g` fills the fronted preposition's object slot.
    let restr = Exp::App(
        Box::new(Exp::App(
            Box::new(Exp::App(
                Box::new(prep_sem.clone()),
                Box::new(Exp::Var(g.into())),
            )),
            Box::new(vp_sem.clone()),
        )),
        Box::new(subj_sem.clone()),
    );
    let sigma = Exp::Sig(Patt::Var(g.into()), Box::new(c.clone()), Box::new(restr));
    let cat = Exp::InductiveCtor(
        decl.clone(),
        "cat_n".into(),
        vec![sigma.clone(), num.clone()],
    );
    Some((cat, sigma))
}

/// The **non-restrictive (appositive) relativizer** rule (D62 §2 #2A): a *referring* NP
/// `cat_np(C, num)` (a name, or any assembled NP) followed by a comma-set-off relative
/// `, which/that [body]` → the antecedent **type-raised to a conjoining quantifier**
/// `λP. logic:And(P(r), body(r))`, where `r` is the antecedent's referent (its sem) and
/// `body : X → Prop` is the gap-abstracted relative clause. Unlike the RESTRICTIVE
/// [`relativize`] (which Σ-*restricts* a common noun's denotation), a non-restrictive
/// relative is a **separate assertion** about an already-identified referent — core-en's
/// `RelPro-Appos` (`misc.xsl`: an `s\s` `Trib` contributory relation, *not* an `n\n`
/// restriction). We realize "separate assertion" by reusing the type-raise cat shape
/// (`S/(S\NP_C)`, so it composes exactly like any subject NP / GQ) with a sem that
/// conjoins `body(r)` alongside the matrix predicate `P(r)`. `None` if the antecedent is
/// not a `cat_np`, the body is not a declarative `S/NP` / `S\NP`, or `logic:And` is absent.
pub fn relativize_appos(
    np_cat: &Exp,
    np_sem: &Exp,
    body_cat: &Exp,
    body_sem: &Exp,
    layer: &Arc<Layer>,
) -> Option<(Exp, Exp)> {
    is_ctor(np_cat, "cat_np")?;
    // Body is a clause missing one NP (`S/NP` object-relative or `S\NP` subject-relative),
    // result a declarative clause — same shape the restrictive rule accepts.
    let (_bm, s, _np) = slash_parts(body_cat, "fwd").or_else(|| slash_parts(body_cat, "bwd"))?;
    if !is_decl_clause(s) {
        return None;
    }
    let and = resolve_inductive(layer, "urn:eigenius:logic:And")?;
    // Reuse the type-raise CAT (`S/(S\NP_C)`); swap its `λP. P(r)` sem for the conjoining
    // `λP. And(P(r), body(r))` — the appositive's separate assertion rides alongside.
    let (raised_cat, _) = type_raise(np_cat, np_sem, layer)?;
    let p = "__appos_p";
    let p_at_r = Exp::App(Box::new(Exp::Var(p.into())), Box::new(np_sem.clone()));
    let body_at_r = Exp::App(Box::new(body_sem.clone()), Box::new(np_sem.clone()));
    let sem = Exp::Lam(
        Patt::Var(p.into()),
        Box::new(Exp::InductiveType(and, vec![p_at_r, body_at_r])),
    );
    Some((raised_cat, sem))
}

/// Fronted **participial adjunct** (D62 §2 #5a): a subject-gapped present-participle VP
/// `cat_s(dcl, ger)\NP` ("affecting BRCA1", "hypothesizing that P") fronted as a sentence
/// pre-modifier `S/S`, asserting the participial proposition alongside the matrix —
/// `λm. logic:And(m, body(hole))`. The participle's subject is CONTROLLED: a referent hole
/// (the `lexicon:anaphor` placeholder, freshened per-span by the caller so it is typed
/// `Entity`/`EntityRef` at the felicity gate → an OPEN parse resolvable to the matrix subject,
/// D64). Reference grammar: core-en's `purp-i`/`tpc` fronted-`s` type-changes (`unary-rules.xsl`).
/// The resulting `S/S` then absorbs a trailing comma (CKY) and forward-applies to the matrix
/// clause. `None` unless `cat` is a subject-gapped `ger` VP, or `logic:And` is unavailable.
pub fn front_participial(cat: &Exp, sem: &Exp, layer: &Arc<Layer>) -> Option<(Exp, Exp)> {
    let Exp::InductiveCtor(cat_decl, _, _) = cat else {
        return None;
    };
    let (_m, s, _np) = slash_parts(cat, "bwd")?;
    let [mood, fin] = is_ctor(s, "cat_s")? else {
        return None;
    };
    if !matches!(mood, Exp::InductiveCtor(_, n, _) if n == "dcl") {
        return None;
    }
    if !matches!(fin, Exp::InductiveCtor(_, n, _) if n == "ger") {
        return None;
    }
    let and = resolve_inductive(layer, "urn:eigenius:logic:And")?;
    let mood_d = resolve_inductive(layer, "urn:eigenius:lexicon:Mood")?;
    let fin_d = resolve_inductive(layer, "urn:eigenius:lexicon:Fin")?;
    let dcl = Exp::InductiveCtor(mood_d, "dcl".into(), vec![]);
    let fin_any = Exp::InductiveCtor(fin_d, "fin_any".into(), vec![]);
    let s_full = Exp::InductiveCtor(cat_decl.clone(), "cat_s".into(), vec![dcl, fin_any]);
    let m_all_ss = crate::dcg::category::mode_value(layer, crate::dcg::category::MODE_ALL)?;
    let ss = Exp::InductiveCtor(
        cat_decl.clone(),
        "fwd".into(),
        vec![m_all_ss, s_full.clone(), s_full],
    );
    // The controlled-subject referent hole: the `lexicon:anaphor` placeholder (freshened by the
    // caller, exactly as a pronoun's sem is). `body(hole)` is the participial proposition.
    let anaphor = Exp::EigonAxiom(Iri::parse("urn:eigenius:lexicon:anaphor").ok()?);
    let body_at_hole = Exp::App(Box::new(sem.clone()), Box::new(anaphor));
    let m = "__front_m";
    let new_sem = Exp::Lam(
        Patt::Var(m.into()),
        Box::new(Exp::InductiveType(
            and,
            vec![Exp::Var(m.into()), body_at_hole],
        )),
    );
    Some((ss, new_sem))
}

/// **Reduced relative** (core-en `unary-rules.xsl`, the `rrel` type-changing rule): a subject-gapped
/// PASSIVE VP becomes a NOUN POST-MODIFIER — "the dependency [compared to MSS cells]", "the deficiency
/// [predicted by the model]".
///
/// `S[dcl,pss]\NP → cat_pp`, **sem unchanged**: both denote `Entity → Prop` (a VP is a predicate over
/// its subject; a `cat_pp` is a predicate over the noun it modifies), so the existing `pp_mod` rule
/// then conjoins it into the noun's Σ restrictor exactly as a PP modifier is. Reusing `cat_pp` is why
/// this needs no new binary rule.
///
/// **The trigger is `pass`, not `pss`** (fixed 2026-07-27). The WordNet importer emits ONE
/// past-participle category per frame, so a transitive verb's `pss` form is `(S[pss]\NP)/NP` — right
/// for the perfect ("has induced DNA", which keeps its object) and wrong for the passive (whose object
/// is promoted to subject). Once that category consumes its object the result is an ACTIVE VP wearing
/// a `pss` label, and firing this shift on it built a relativizer-less SUBJECT relative, which English
/// does not allow (it permits a reduced OBJECT relative, "the food the man ate", and a participial,
/// "the DNA induced by WRN", but never "*the man ate the food" for "the man that ate"):
///
/// ```text
/// "Depletion of WRN induced double-stranded DNA breaks."
///   -> cat_n(Σx:WRN. induced(DNA, x))      "WRN [that] induced DNA"   <- was built here
///   -> then `breaks` is read as the finite intransitive main verb
/// ```
///
/// `lexicon:Fin` already draws the distinction — `pss` is the past participle (active/perfect
/// transitive), `pass` the PASSIVE participle VP (patient-subject) — and a reduced relative predicates
/// over the PATIENT, so only `pass` can license one. Keying on `pass` is what this now does; the
/// producer that reaches it is `gq_prep_passive_agent` ("the deficiency predicted by the model").
///
/// **Narrowing the trigger alone was measured INERT, and the other half is the fix.** The oblique
/// participial ("compared to MSS cell lines") IS passive voice, but it only ever reached this rule by
/// riding the same `pss` route the active/perfect form came through — so refusing `pss` here without
/// giving it a route of its own removed a working analysis and measured 367 skeletons, byte-identical
/// to disabling the shift outright. Its route is [`super::combinators::oblique_participial_lifts`],
/// which lifts the still-UNSATURATED `(S[dcl,pss]\NP)/cat_pp_arg(P)` to `cat_pp/cat_pp_arg(P)`.
///
/// The two halves are one change because the discriminator only exists BEFORE saturation. After it,
/// oblique and transitive are the same category (`S[dcl,pss]\NP`) over the same object-first
/// `Entity → Entity → Prop` arrow, reached by the same `ForwardApp` — a provenance guard cannot
/// separate them (chart dump, 2026-07-27). Before it, the oblique's remaining argument is
/// `cat_pp_arg` and the transitive's is `cat_np`, and THAT is the agent test: `PpOblique` is 2-place
/// with no distinct agent, so its subject slot is the one the modified noun should fill
/// (`compared to X` → `λsubj. compare(X, subj)`), while `Transitive` puts the agent there
/// (`induced DNA` → `λsubj. induce(DNA, subj)`).
///
/// The lift must run at SEED time: both chart drivers run the shift table inside `for len in 2..=n`,
/// so a `UnaryShift` never sees a single-token cell, and `compared` is one.
///
/// Measured, replayed against the same snapshot and recording (grammar-gap 0, pins 44/45 throughout):
///
/// ```text
///                                          skeletons   readings
///   page                                    269 -> 263  1190 -> 1178
///   "Depletion of WRN induced …" (isolated)   8 ->   2    18 ->    6
///   "… compared to MSS cell lines"           64 ->  64   set-identical, 0 lost / 0 added
/// ```
///
/// core-en states the rule as `s[dcl]/np → n\n` with a `GenRel` relation. We take the SUBJECT-gap
/// (backward) form rather than its object-gap (forward) one, because that is the participial case —
/// the same asymmetry as [`front_participial`], which is this rule's sentence-level counterpart
/// (`ger` VP → `S/S`). Building one half of the participial story and not the other is why
/// "The deficiency predicted by the model was clear." parsed to 0 readings.
pub fn reduced_relative(cat: &Exp, layer: &Arc<Layer>) -> Option<Exp> {
    let Exp::InductiveCtor(cat_decl, _, _) = cat else {
        return None;
    };
    let (_m, s, _np) = slash_parts(cat, "bwd")?;
    let [mood, fin] = is_ctor(s, "cat_s")? else {
        return None;
    };
    if !matches!(mood, Exp::InductiveCtor(_, n, _) if n == "dcl") {
        return None;
    }
    if !matches!(fin, Exp::InductiveCtor(_, n, _) if n == "pass") {
        return None;
    }
    let _ = layer;
    Some(Exp::InductiveCtor(
        cat_decl.clone(),
        "cat_pp".into(),
        Vec::new(),
    ))
}

/// Elided-`than` standard defaulting (D63 §8.12): a comparative that is still awaiting its `than`
/// complement — category `X / cat_pp_than`, sem `λstd. body(std)` — completes with an ANAPHORIC
/// standard. It yields `X` with sem `body(anaphor)` (the `lexicon:anaphor` placeholder, freshened by
/// the caller like any referent hole → an OPEN parse the D64 resolver fills from discourse). This is
/// the general "`than` is optional" rule: "less dependent on WRN" (no `than`) resolves its comparison
/// target from context. It REPLACES the per-word bare degree entries (`more_deg_bare`/`less_deg_bare`)
/// with one shift; because only the analytic adjective comparative yields the outer shape
/// `(S[adj]\NP) / cat_pp_than` (the measure/cardinality comparatives nest `cat_pp_than` under a `bwd`),
/// it fires exactly where those entries did — a behaviour-preserving collapse, not a widening.
pub fn elided_than(cat: &Exp, sem: &Exp, _layer: &Arc<Layer>) -> Option<(Exp, Exp)> {
    let (_m, result, arg) = slash_parts(cat, "fwd")?;
    let [] = is_ctor(arg, "cat_pp_than")? else {
        return None;
    };
    let anaphor = Exp::EigonAxiom(Iri::parse("urn:eigenius:lexicon:anaphor").ok()?);
    let body_at_hole = Exp::App(Box::new(sem.clone()), Box::new(anaphor));
    Some((result.clone(), body_at_hole))
}

/// Whether `s` is a declarative clause `cat_s(dcl, _)` — the result type a relative
/// clause body abstracts over (D63 §8.9). The finiteness is irrelevant here (a VP
/// result is `fin`, an object-extraction `S/NP` result is the `T` target's `fin`);
/// the mood must be declarative.
fn is_decl_clause(s: &Exp) -> bool {
    matches!(is_ctor(s, "cat_s"), Some([mood, _fin])
        if matches!(mood, Exp::InductiveCtor(_, n, _) if n == "dcl"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbe::term::list_decl;
    use crate::ontology::iri::Iri;

    fn ctor(name: &str, args: Vec<Exp>) -> Exp {
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
    fn cls(iri: &str) -> Exp {
        Exp::EigonClass(Iri::parse(iri).unwrap())
    }
    fn np(t: Exp) -> Exp {
        ctor("cat_np", vec![t, ctor("num_any", vec![])])
    }
    fn decl_s() -> Exp {
        ctor("cat_s", vec![ctor("dcl", vec![]), ctor("fin", vec![])])
    }

    // ── coordinate_np: a bare-KIND conjunct is realised as an entity via `kind_of` ──
    #[test]
    fn np_conjunct_realises_a_bare_kind_via_kind_of() {
        let gene = cls("urn:eigenius:lexicon:Gene");
        // A determined/named `cat_np`: its sem is already an entity, passed through unchanged.
        let np_cat = ctor("cat_np", vec![gene.clone(), ctor("num_any", vec![])]);
        let np_sem = cls("urn:eigenius:lexicon:Achilles");
        let (t, member, _, _) = np_conjunct(&np_cat, &np_sem).expect("cat_np is an NP conjunct");
        assert_eq!(t, gene, "type is the NP's type");
        assert_eq!(member, np_sem, "a determined NP's sem is already an entity");

        // A bare `cat_n` KIND ("WRN"/"MSI"): its kind sem is realised as an entity via `kind_of`, so
        // coordinated bare kinds can be an argument (a lone bare singular could not).
        let n_cat = ctor("cat_n", vec![gene.clone(), ctor("num_any", vec![])]);
        let n_sem = cls("urn:eigenius:lexicon:Wrn");
        let (t2, member2, _, _) = np_conjunct(&n_cat, &n_sem).expect("cat_n is a kind NP conjunct");
        assert_eq!(t2, gene);
        let wrapped = matches!(&member2, Exp::App(f, x)
            if matches!(f.as_ref(), Exp::EigonAxiom(i) if i.as_str().ends_with(":kind_of"))
            && x.as_ref() == &n_sem);
        assert!(wrapped, "a bare kind is wrapped in kind_of: {member2:?}");

        // A non-NP category is not an NP conjunct.
        assert!(np_conjunct(&decl_s(), &Exp::Unit).is_none());
    }

    // ── appose_group (D63 §8.4 Phase 6, RC-6 close nominal apposition) ──
    #[test]
    fn close_apposition_distributes_the_classifier_over_every_group_member() {
        let layer = Arc::new(
            crate::layer::LayerBuilder::new("appos-test", None)
                .build(crate::layer::LayerStorage::in_memory()),
        );
        let gene = cls("urn:eigenius:lexicon:Gene");
        // The name-group `BRCA1 and MSH2` : cat_group(Gene, conn_and, pl); its sem is a cons-chain.
        let (brca1, msh2) = (
            cls("urn:eigenius:lexicon:Brca1"),
            cls("urn:eigenius:lexicon:Msh2"),
        );
        let group = ctor(
            "cat_group",
            vec![gene.clone(), ctor("conn_and", vec![]), ctor("pl", vec![])],
        );
        let group_sem = list_term(&[brca1.clone(), msh2.clone()]);
        let s_finany = ctor("cat_s", vec![ctor("dcl", vec![]), ctor("fin_any", vec![])]);
        // Head "the genes" — a subject GQ  S/(S\NP_Gene) = fwd(S, bwd(S, cat_np(Gene, _))).
        let the_genes = ctor(
            "fwd",
            vec![
                s_finany.clone(),
                ctor("bwd", vec![decl_s(), np(gene.clone())]),
            ],
        );
        let (cat, sem) = appose_group(&the_genes, &group, &group_sem, &layer)
            .expect("a gene-typed group apposes a gene-typed head");
        assert_eq!(
            cat, group,
            "a class-typed classifier reproduces the group category"
        );
        // EVERY member is classified — not just the first, which is all a `[classifier designator]`
        // bracketing reaches. A pass-through returned `group_sem` here and left the classifier nowhere.
        let designate = |d: &Exp| {
            definite_individual(&naming_refinement(&gene, d)).expect("ontology:the is a valid iri")
        };
        assert_eq!(
            sem,
            list_term(&[designate(&brca1), designate(&msh2)]),
            "each designator becomes `the(Σx:classifier. named(x, dᵢ)).1`"
        );

        // Bare common-noun head "genes" : cat_n(Gene, pl) — same distribution.
        let bare = ctor("cat_n", vec![gene.clone(), ctor("pl", vec![])]);
        assert_eq!(
            appose_group(&bare, &group, &group_sem, &layer).map(|(_, s)| s),
            Some(sem),
            "a bare common-noun head classifies its members identically"
        );

        // Compound-Σ head "the MMR genes" : S/(S\NP_{Σx:Gene. φ}). The BASE class (Gene) peels out for
        // the felicity gate and indexes the group; the FULL refinement reaches every member's sem —
        // that is the information the pass-through dropped ("MMR" left no trace in any reading).
        let sigma = Exp::Sig(
            Patt::Var("x".into()),
            Box::new(gene.clone()),
            Box::new(Exp::Sort(0)),
        );
        let mmr_genes = ctor(
            "fwd",
            vec![
                s_finany.clone(),
                ctor("bwd", vec![decl_s(), np(sigma.clone())]),
            ],
        );
        let (mmr_cat, mmr_sem) = appose_group(&mmr_genes, &group, &group_sem, &layer)
            .expect("the compound-Σ head's base class (Gene) licenses the apposition");
        assert_eq!(
            mmr_cat, group,
            "the group is indexed at the classifier's BASE class, so selectional checks still see a \
             class (`type_subsumes` falls back to equality on a Σ)"
        );
        let designate_mmr = |d: &Exp| {
            definite_individual(&naming_refinement(&sigma, d)).expect("ontology:the is a valid iri")
        };
        assert_eq!(
            mmr_sem,
            list_term(&[designate_mmr(&brca1), designate_mmr(&msh2)]),
            "every member is an MMR gene, not merely a gene"
        );

        // Felicity reject: "the cells BRCA1 and MSH2" — genes are not cells (no lattice link).
        let the_cells = ctor(
            "fwd",
            vec![
                s_finany,
                ctor(
                    "bwd",
                    vec![decl_s(), np(cls("urn:eigenius:lexicon:CellLine"))],
                ),
            ],
        );
        assert!(
            appose_group(&the_cells, &group, &group_sem, &layer).is_none(),
            "a gene-typed group does not appose a cell-typed head"
        );

        // A transitive verb `(S\NP)/NP` is NOT an apposition head (its fwd-arg is an object NP).
        let verb = ctor(
            "fwd",
            vec![ctor("bwd", vec![decl_s(), np(gene.clone())]), np(gene)],
        );
        assert!(
            appose_group(&verb, &group, &group_sem, &layer).is_none(),
            "a verb's fwd-argument is an object NP, not a VP — no apposition head type"
        );
    }

    /// A classifier already carrying a **postmodifying PP** or an already-designated one is refused:
    /// the designator is adjacent to the nominal head, so neither can intervene. Pins both halves of
    /// the normal form [`is_pp_refined`] / [`is_naming_refined`] enforce.
    #[test]
    fn close_apposition_refuses_a_pp_modified_or_already_named_classifier() {
        let layer = Arc::new(
            crate::layer::LayerBuilder::new("appos-nf-test", None)
                .build(crate::layer::LayerStorage::in_memory()),
        );
        let gene = cls("urn:eigenius:lexicon:Gene");
        let group = ctor(
            "cat_group",
            vec![gene.clone(), ctor("conn_and", vec![]), ctor("pl", vec![])],
        );
        let group_sem = list_term(&[cls("urn:eigenius:lexicon:Brca1")]);
        let refined = |restr: Exp| {
            ctor(
                "cat_n",
                vec![
                    Exp::Sig(
                        Patt::Var(crate::dcg::rules::combinators::COMPOUND_X.into()),
                        Box::new(gene.clone()),
                        Box::new(restr),
                    ),
                    ctor("pl", vec![]),
                ],
            )
        };
        let x = || Exp::Var(crate::dcg::rules::combinators::COMPOUND_X.into());
        let app2 = |iri: &str, a: Exp| {
            Exp::App(
                Box::new(Exp::App(
                    Box::new(Exp::EigonAxiom(Iri::parse(iri).unwrap())),
                    Box::new(x()),
                )),
                Box::new(a),
            )
        };
        // "mutations in the MMR" + "genes BRCA1" — the PP would have to sit between classifier and
        // designator ("*the gene in humans MSH2"), so the head is not an apposition classifier.
        let pp_head = refined(app2(
            "urn:eigenius:ontology:prep_in",
            cls("urn:eigenius:lexicon:Mmr"),
        ));
        assert!(
            appose_group(&pp_head, &group, &group_sem, &layer).is_none(),
            "a PP-postmodified classifier is refused"
        );
        // The un-REDUCED form the PP-noun-modifier rule actually builds — `(λx. prep_in(x, y)) x`.
        // Matching only the `App` spine reaches the `Lam` and misses this, which made the check inert.
        let unreduced = Exp::App(
            Box::new(Exp::Lam(
                Patt::Var(crate::dcg::rules::combinators::COMPOUND_X.into()),
                Box::new(app2(
                    "urn:eigenius:ontology:prep_in",
                    cls("urn:eigenius:lexicon:Mmr"),
                )),
            )),
            Box::new(x()),
        );
        assert!(
            appose_group(&refined(unreduced), &group, &group_sem, &layer).is_none(),
            "the un-reduced PP restrictor is refused too"
        );
        // An already-designated classifier ("genes MSH2") takes no second designator list.
        let named_head = refined(app2(
            "urn:eigenius:ontology:named",
            cls("urn:eigenius:lexicon:Msh2"),
        ));
        assert!(
            appose_group(&named_head, &group, &group_sem, &layer).is_none(),
            "an already-named classifier is refused"
        );
        // Control: a COMPOUND-refined classifier ("MMR genes") is the licensed case and must pass.
        let compound_head = refined(app2(
            "urn:eigenius:ontology:compound_kind",
            cls("urn:eigenius:lexicon:Mmr"),
        ));
        assert!(
            appose_group(&compound_head, &group, &group_sem, &layer).is_some(),
            "a compound-refined classifier still apposes — that is the MMR-genes case"
        );
    }

    // ── coordinate_prop / complete_coord (D63 §8.4 Phase 3, list-with-operator) ──
    #[test]
    fn prop_coordination_builds_a_list_and_completes_by_folding() {
        // The prop-side list-with-operator model: comma builds a neutral `conn_list` list, the trailing
        // `or` rebinds the whole list, and `complete_coord` folds it left-branching all-`∨`. Needs the
        // real `logic:And/Or` + `lexicon:Conn` inductives ⇒ bootstrap.
        let ctx = crate::bootstrap::bootstrap().expect("bootstrap");
        let layer = Arc::clone(ctx.head());
        // A prop-ending base category — a declarative clause `S[dcl,fin]` (⟦·⟧ = Prop).
        let s = ctor("cat_s", vec![ctor("dcl", vec![]), ctor("fin", vec![])]);
        let (a, b, c) = (
            Exp::Var("A".into()),
            Exp::Var("B".into()),
            Exp::Var("C".into()),
        );
        // `A , B` → cat_coord(S, conn_list) over [A, B].
        let (c1_cat, c1_sem) =
            coordinate_prop(LIST_CONN, &s, &a, &s, &b, &layer).expect("comma builds a coord list");
        assert!(
            matches!(is_ctor(&c1_cat, "cat_coord"), Some([base, conn])
                if *base == s && conn_name_of(conn) == Some("conn_list")),
            "the comma yields a neutral conn_list list over the base clause"
        );
        // `... or C` → the `or` rebinds the whole list to conn_or, appending C.
        let (c2_cat, c2_sem) =
            coordinate_prop("urn:eigenius:logic:Or", &c1_cat, &c1_sem, &s, &c, &layer)
                .expect("or finalizes the list");
        assert!(
            matches!(is_ctor(&c2_cat, "cat_coord"), Some([_, conn]) if conn_name_of(conn) == Some("conn_or")),
            "the trailing `or` rebinds the neutral list to conn_or"
        );
        // Completion folds left-branching: Or(Or(A, B), C).
        let (base, folded) =
            complete_coord(&c2_cat, &c2_sem, &layer, crate::dcg::RightContext::Other)
                .expect("completes");
        assert_eq!(base, s, "completion returns the base clause category");
        let expect = |op: &Exp, args: &[Exp]| {
            matches!(op, Exp::InductiveType(d, a)
            if d.iri.as_str() == "urn:eigenius:logic:Or" && a.as_slice() == args)
        };
        match &folded {
            Exp::InductiveType(d, args)
                if d.iri.as_str() == "urn:eigenius:logic:Or" && args.len() == 2 =>
            {
                assert!(expect(&args[0], &[a, b]), "inner Or(A, B): {folded:?}");
                assert_eq!(args[1], c, "outer right conjunct is C");
            }
            other => panic!("expected Or(Or(A,B),C), got {other:?}"),
        }
        // Mixing a FINALIZED list rejects: `(A or B) and C` — conn_or left, `and` op.
        assert!(
            coordinate_prop("urn:eigenius:logic:And", &c2_cat, &c2_sem, &s, &c, &layer).is_none(),
            "a finalized conn_or list does not accept a following `and` (no X or Y and Z mixing)"
        );
    }
}
