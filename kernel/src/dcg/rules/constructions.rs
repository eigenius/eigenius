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
        Exp::Sort(0) => Some(Exp::InductiveType(op.clone(), vec![a.clone(), b.clone()])),
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
/// form, via [`generalized_coord`]). A never-finalized `conn_list` (a bare comma list, no `and`/`or`)
/// defaults to conjunction. Needs ≥2 members. Returns `(BaseCat, folded_sem)`; `None` for an ill-formed
/// list or an unresolvable operator. Realized as a unary shift in both CKY paths (packable).
pub fn complete_coord(coord_cat: &Exp, coord_sem: &Exp, layer: &Arc<Layer>) -> Option<(Exp, Exp)> {
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
        "conn_and" | "conn_list" => "urn:eigenius:logic:And",
        "conn_or" => "urn:eigenius:logic:Or",
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
fn group_conn_op(group_cat: &Exp) -> Option<&'static str> {
    let [_c, conn, _num] = is_ctor(group_cat, "cat_group")? else {
        return None;
    };
    match conn {
        Exp::InductiveCtor(_, n, _) if n == "conn_and" => Some("urn:eigenius:logic:And"),
        Exp::InductiveCtor(_, n, _) if n == "conn_or" => Some("urn:eigenius:logic:Or"),
        // A never-finalized comma list (no trailing `and`/`or`) defaults to conjunction — "A, B, C
        // affect X" ⟿ `∧`. A finalized list carries `conn_and`/`conn_or` and never reaches here.
        Exp::InductiveCtor(_, n, _) if n == "conn_list" => Some("urn:eigenius:logic:And"),
        _ => None,
    }
}

/// The **neutral list connective** a comma contributes (D63 §8.4 Phase 6, Step 5b). English list
/// commas are polarity-neutral — `A, B, C or D` means `A ∨ B ∨ C ∨ D`, `A, B, C and D` means all-`∧`;
/// the comma inherits the list's FINAL explicit connective. So a comma builds a `conn_list` group that
/// the trailing `and`/`or` REBINDS (below). This is a PARSER-INTERNAL sentinel — never a logic op and
/// never a committed `lexicon:Conn` ctor (`denote_cat` erases the `Conn` argument, so it never reaches
/// the kernel), so no ontology change / reseed is needed. A group left `conn_list` at fold time (a bare
/// comma list, no explicit connective) defaults to conjunction ([`group_conn_op`]).
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
/// the named group *specifies* them; the names pick out exactly the members, so the group IS the
/// referent (the head's determiner quantification is overridden — "the poet Burns" refers to Burns,
/// not to some poet). We realize this by passing the **group through unchanged**, gated on the
/// felicity condition that the named members are of the head noun's kind. The result group then rides
/// the existing distributive-subject / distributive-object machinery unmodified (`distribute` /
/// `distribute_object`), so "the genes BRCA1 and MSH2 affect cells" ⟿ `affect(cells, brca1) ∧
/// affect(cells, msh2)` — exactly the bare-group reading, now licensed through the classifying head.
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
/// `Entity`) ⇒ no parse. `None` if the shapes don't match or the gate fails. The determiner's
/// definiteness and the head's type-assertion (`gene(brca1) ∧ …`, already lexically guaranteed by the
/// names' types) are dropped — a first-cut approximation, parallel to the existential treatment of
/// `the` (a faithfulness refinement, D61).
pub fn appose_group(
    head_cat: &Exp,
    group_cat: &Exp,
    group_sem: &Exp,
    layer: &Arc<Layer>,
) -> Option<(Exp, Exp)> {
    let head_ty = sigma_base(appositive_head_type(head_cat)?);
    let [group_ty, _conn, _num] = is_ctor(group_cat, "cat_group")? else {
        return None;
    };
    let group_ty = sigma_base(group_ty);
    // Felicity: the head noun and the named members must name the SAME KIND — one type subsumes the
    // other (either direction, to bridge the concept-vs-semantic-type granularity gap across importers).
    if !type_subsumes(head_ty, group_ty, layer) && !type_subsumes(group_ty, head_ty, layer) {
        return None;
    }
    Some((group_cat.clone(), group_sem.clone()))
}

/// The classifying **type index** of a close-apposition head: a subject GQ `S/(S\NP_C)` (a determined
/// head, "the genes") yields `C`; a bare common noun `cat_n(C, _)` yields `C`. A transitive verb
/// `(S\NP)/NP` or a preposition `cat_pp/cat_np` never matches — their `fwd` ARGUMENT is a bare
/// `cat_np` (object / prep-object), not a `S\NP` VP, so the inner `bwd` probe fails. `None` otherwise.
fn appositive_head_type(head_cat: &Exp) -> Option<&Exp> {
    // Determined subject GQ  S/(S\NP_C) = fwd(S, bwd(S, cat_np(C, _))): the ARGUMENT (arg 1) is the VP.
    if let Some([_result, arg]) = is_ctor(head_cat, "fwd") {
        if let Some([_s, np]) = is_ctor(arg, "bwd") {
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
    let op = resolve_inductive(layer, group_conn_op(group_cat)?)?;
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
        let op = resolve_inductive(layer, group_conn_op(group_cat)?)?;
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
    if group_conn_op(group_cat)? != "urn:eigenius:logic:And" {
        return None;
    }
    let members = group_members(group_sem)?;
    if members.len() < 2 {
        return None;
    }
    // `tv_cat` must be a transitive verb `(S\NP)/NP` = fwd(bwd(S, subj), obj); the
    // reciprocal sentence's category is that inner `S`.
    let [vp, _obj] = is_ctor(tv_cat, "fwd")? else {
        return None;
    };
    let [result, _subj] = is_ctor(vp, "bwd")? else {
        return None;
    };
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
    let vp = Exp::InductiveCtor(cat_decl.clone(), "bwd".into(), vec![s.clone(), cat.clone()]);
    let raised_cat = Exp::InductiveCtor(cat_decl.clone(), "fwd".into(), vec![s, vp]);
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
    let body_args = is_ctor(body_cat, "fwd").or_else(|| is_ctor(body_cat, "bwd"))?;
    let [s, _np] = body_args else {
        return None;
    };
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
    let body_args = is_ctor(body_cat, "fwd").or_else(|| is_ctor(body_cat, "bwd"))?;
    let [s, _np] = body_args else {
        return None;
    };
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
    let [s, _np] = is_ctor(cat, "bwd")? else {
        return None;
    };
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
    let ss = Exp::InductiveCtor(cat_decl.clone(), "fwd".into(), vec![s_full.clone(), s_full]);
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
    fn close_apposition_passes_group_through_gated_on_head_kind() {
        let layer = Arc::new(
            crate::layer::LayerBuilder::new("appos-test", None)
                .build(crate::layer::LayerStorage::in_memory()),
        );
        let gene = cls("urn:eigenius:lexicon:Gene");
        // The name-group `BRCA1 and MSH2` : cat_group(Gene, conn_and, pl); its sem is a cons-chain.
        let group = ctor(
            "cat_group",
            vec![gene.clone(), ctor("conn_and", vec![]), ctor("pl", vec![])],
        );
        let group_sem = ctor("cons", vec![Exp::Unit, ctor("nil", vec![])]);
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
        assert_eq!(cat, group, "the group category passes through unchanged");
        assert_eq!(sem, group_sem, "the group sem passes through unchanged");

        // Bare common-noun head "genes" : cat_n(Gene, pl) — same pass-through.
        let bare = ctor("cat_n", vec![gene.clone(), ctor("pl", vec![])]);
        assert!(
            appose_group(&bare, &group, &group_sem, &layer).is_some(),
            "a bare common-noun head also apposes"
        );

        // Compound-Σ head "the MMR genes" : S/(S\NP_{Σx:Gene. φ}) — the BASE class (Gene) peels out.
        let sigma = Exp::Sig(
            Patt::Var("x".into()),
            Box::new(gene.clone()),
            Box::new(Exp::Sort(0)),
        );
        let mmr_genes = ctor(
            "fwd",
            vec![s_finany.clone(), ctor("bwd", vec![decl_s(), np(sigma)])],
        );
        assert!(
            appose_group(&mmr_genes, &group, &group_sem, &layer).is_some(),
            "the compound-Σ head's base class (Gene) licenses the apposition"
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
        let (base, folded) = complete_coord(&c2_cat, &c2_sem, &layer).expect("completes");
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
