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

//! Categorial-type semantics: the `⟦·⟧` homomorphism, definitional equality, the
//! `lexicon:Cat` constructor accessor, and categorial subsumption.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::layer::Layer;
use crate::nbe::env::Rho;
use crate::nbe::eval::eval;
use crate::nbe::readback::readback_val;
use crate::nbe::term::{list_decl, Exp, InductiveDecl, Name, Patt};
use crate::nbe::val::Val;
use crate::ontology::iri::Iri;

/// A category type-variable binding: schematic `Exp::Var` name → concrete type.
/// `BTreeMap` for deterministic iteration (the project-wide convention).
pub type CatSubst = BTreeMap<Name, Exp>;

/// `⟦·⟧ : Cat → EigenTT type` — the categorial-to-type homomorphism. `Cat` is
/// type-indexed (`cat_np(T)` carries its class), so `⟦·⟧` is self-contained.
pub fn denote_cat(cat: &Exp) -> Result<Exp, String> {
    let Exp::InductiveCtor(_decl, name, args) = cat else {
        return Err(format!(
            "denote_cat: expected a lexicon:Cat constructor, got {cat:?}"
        ));
    };
    match (name.as_str(), args.as_slice()) {
        ("cat_s", [mood, _fin]) => denote_mood(mood), // ⟦S[m,_]⟧ = ⟦m⟧ (fin erased)
        ("cat_n", [_t, _num]) => Ok(Exp::Sort(1)),    // ⟦N(T)[_]⟧ = Set (type + num erased)
        ("cat_np", [t, _num]) => Ok(t.clone()),       // ⟦NP(T)[_]⟧ = T (num erased)
        // ⟦Group(C)[_,_]⟧ = List C — a coordinated group denotes the member-retaining
        // list over its common supertype C (D63 §8.4 Phase 6, the kernel `List`);
        // the connective and number are erased by ⟦·⟧.
        ("cat_group", [c, _conn, _num]) => Ok(Exp::InductiveType(list_decl(), vec![c.clone()])),
        // ⟦Q(T)⟧ = T → Prop — a wh-question denotes its answer-property (the
        // predicate the answer must satisfy), over the queried type T (D63 §8.5).
        ("cat_q", [t]) => Ok(Exp::Arrow(Box::new(t.clone()), Box::new(Exp::Sort(0)))),
        // ⟦Kind⟧ = Set — a kind-denoting NP denotes a type (the kind as a value of
        // `Set`); the predicate over it is `Set → Prop` (D63 §8.5, kind subjects).
        ("cat_kind", []) => Ok(Exp::Sort(1)),
        // ⟦CP⟧ = Prop — an embedded complement clause denotes the embedded proposition
        // (D63 §8.11, clausal complements); a clause-taking verb is `(S\NP)/cat_cp`.
        ("cat_cp", []) => Ok(Exp::Sort(0)),
        // ⟦PP[than]⟧ = Entity — the than-phrase supplies the comparison STANDARD, an
        // entity (D63 §8.12, comparatives). `than : cat_pp_than / cat_np(Entity)`.
        ("cat_pp_than", []) => Ok(Exp::EigonClass(
            Iri::parse("urn:eigenius:lexicon:Entity").map_err(|e| e.to_string())?,
        )),
        // ⟦PP[mod]⟧ = Entity → Prop — a noun-postmodifying PP is a predicate over the head
        // noun's entities (D63 §8.13, 6-mod). The post-nominal refine rule applies it under
        // a Σ; `of : cat_pp / cat_np(Entity)`, sem `λy.λx. prep_of(x, y)`.
        ("cat_pp", []) => Ok(Exp::Arrow(
            Box::new(Exp::EigonClass(
                Iri::parse("urn:eigenius:lexicon:Entity").map_err(|e| e.to_string())?,
            )),
            Box::new(Exp::Sort(0)),
        )),
        ("fwd", [a, b]) | ("bwd", [a, b]) => Ok(Exp::Arrow(
            Box::new(denote_cat(b)?),
            Box::new(denote_cat(a)?),
        )),
        // ⟦cat_forall(λT:Set. R)⟧ = ΠT:Set. ⟦R⟧ — the dependent forward over a
        // common-noun type binds T (the noun's type) as a Π; ⟦R⟧ may mention it
        // (`cat_np(T) → T`). This is the realization of D63 §8.2 item 3.
        ("cat_forall", [_num, body]) => {
            // The determiner's expected noun-number (`_num`) is syntactic — erased
            // by `⟦·⟧`, checked by `apply` against the noun (agreement).
            let Exp::Lam(patt, r) = body else {
                return Err(format!(
                    "denote_cat: cat_forall body must be a λ (Set -> Cat), got {body:?}"
                ));
            };
            Ok(Exp::Pi(
                patt.clone(),
                Box::new(Exp::Sort(1)),
                Box::new(denote_cat(r)?),
            ))
        }
        // ⟦cat_fin_forall(λf. R)⟧ = ⟦R⟧ / ⟦cat_num_forall(λn. R)⟧ = ⟦R⟧ (D63 §8.10):
        // a FEATURE binder is denotation-TRANSPARENT — features are erased by `⟦·⟧`, so
        // the bound `f`/`n` is free in `R` but never reached (every feature position is
        // discarded above), and `⟦R⟧` stays closed. Unlike `cat_forall` (a Π over the
        // noun TYPE), this binds no value — it only carries a unification variable the
        // parser instantiates from the consumed verb's real feature.
        ("cat_fin_forall", [body]) | ("cat_num_forall", [body]) => {
            let Exp::Lam(_patt, r) = body else {
                return Err(format!(
                    "denote_cat: {name} body must be a λ (Fin/Num -> Cat), got {body:?}"
                ));
            };
            denote_cat(r)
        }
        (n, a) => Err(format!(
            "denote_cat: unexpected ctor `{n}` of arity {}",
            a.len()
        )),
    }
}

/// ⟦mood⟧ (D63 §5.1, §8.5). A declarative `S[dcl]` denotes a `Prop`. A **polar**
/// question `S[q]` *also* denotes a `Prop` — the queried proposition (asked, not
/// asserted); the `q` tag is what distinguishes it for the consumer (Slice 5a). A
/// *wh*-question is NOT `cat_s(q, _)` — it is `cat_q(T)` (⟦·⟧ = T → Prop), so it
/// never reaches here. Imperatives remain deferred (fail closed, not silently
/// `Prop`).
fn denote_mood(mood: &Exp) -> Result<Exp, String> {
    let Exp::InductiveCtor(_, name, args) = mood else {
        return Err(format!(
            "denote_mood: expected a lexicon:Mood ctor, got {mood:?}"
        ));
    };
    match (name.as_str(), args.as_slice()) {
        ("dcl" | "q", []) => Ok(Exp::Sort(0)), // Prop (polar `q` = the queried Prop)
        ("imp", []) => Err(format!("⟦S[{name}]⟧ deferred to D63 Slice 5")),
        (n, _) => Err(format!("denote_mood: unexpected mood ctor `{n}`")),
    }
}

/// Definitional equality of two closed type expressions, via NbE normal forms
/// (so `A -> B` and `Pi _:A. B` compare equal).
pub fn type_eq(a: &Exp, b: &Exp) -> bool {
    let norm = |e: &Exp| eval(e, &Rho::Nil).map(|v| readback_val(0, &v));
    matches!((norm(a), norm(b)), (Ok(x), Ok(y)) if x == y)
}

/// If `cat` is the named `lexicon:Cat` constructor, return its arguments.
pub fn is_ctor<'a>(cat: &'a Exp, name: &str) -> Option<&'a [Exp]> {
    match cat {
        Exp::InductiveCtor(_, n, args) if n.as_str() == name => Some(args),
        _ => None,
    }
}

/// Categorial subsumption: may an `arg` category fill a `slot` category? Atoms
/// match by constructor, with these relaxations (D62 §8.6 / D63 §5.1, §8.2):
/// - an entity atom `cat_np(Sub, _)` fills `cat_np(Super, _)` when `Sub
///   subclass_of* Super` — CN-as-types subsumption (Luo 2012), so a general
///   verb's `NP[Entity]` slot accepts an `NP[Gene]` argument;
/// - the morphosyntactic **features** unify by **meet** (`Any = ⊤`): `sg` fills
///   `sg` or `Any`, never `pl`. Mood matches exactly (it is semantic);
/// - **functors** (`A/B`, `A\B`) subsume structurally with function variance —
///   covariant result, contravariant argument — so `S\NP_Entity` fills
///   `S\NP_Gene` (item 4).
///
/// Reflexive, so exact composition is the `Sub = Super`, equal-features case.
pub fn cat_subsumes(slot: &Exp, arg: &Exp, layer: &Arc<Layer>) -> bool {
    unify_cat(slot, arg, layer).is_some()
}

/// Categorial **unification** (D63 §8.2 item 2): can `arg` fill `slot`, and with
/// what binding of the slot's schematic type-variables? Generalizes
/// [`cat_subsumes`] (which is `unify_cat(..).is_some()`): a slot type-index that
/// is an `Exp::Var` (a polymorphic determiner's category variable `T`) **binds**
/// to the argument's concrete type; a concrete slot type must subsume per the
/// subclass lattice. The caller substitutes the returned binding through the
/// result category ([`subst_cat`]), so `every`+`gene` carries `T := Gene` into
/// `S/(S\NP_Gene)`.
pub fn unify_cat(slot: &Exp, arg: &Exp, layer: &Arc<Layer>) -> Option<CatSubst> {
    let mut subst = CatSubst::new();
    unify_into(slot, arg, layer, &mut subst).then_some(subst)
}

fn unify_into(slot: &Exp, arg: &Exp, layer: &Arc<Layer>, subst: &mut CatSubst) -> bool {
    // cat_np(T, num): unify the type-index (var-aware), unify the number (var-aware).
    if let (Some(s), Some(a)) = (is_ctor(slot, "cat_np"), is_ctor(arg, "cat_np")) {
        if s.len() == 2 && a.len() == 2 {
            return unify_type(&s[0], &a[0], layer, subst) && unify_feat(&s[1], &a[1], subst);
        }
    }
    // cat_n(T, num): unify the type-index (var-aware), unify the number (var-aware).
    if let (Some(s), Some(a)) = (is_ctor(slot, "cat_n"), is_ctor(arg, "cat_n")) {
        if s.len() == 2 && a.len() == 2 {
            return unify_type(&s[0], &a[0], layer, subst) && unify_feat(&s[1], &a[1], subst);
        }
    }
    // cat_group(C, conn, num): a group fills a COLLECTIVE verb's group slot
    // (D63 §8.4 Phase 6). Unify the member type-index (var-aware + subclass
    // subsumption), and unify the connective and number features. The connective
    // match is what restricts collective verbs to `and`-groups (no `conn_any`, so
    // `conn_and` accepts only `conn_and`); "X or Y form a complex" gets no parse.
    if let (Some(s), Some(a)) = (is_ctor(slot, "cat_group"), is_ctor(arg, "cat_group")) {
        if s.len() == 3 && a.len() == 3 {
            return unify_type(&s[0], &a[0], layer, subst)
                && unify_feat(&s[1], &a[1], subst)
                && unify_feat(&s[2], &a[2], subst);
        }
    }
    // cat_s(mood, fin): mood matches exactly (semantic); fin unifies (var-aware).
    if let (Some(s), Some(a)) = (is_ctor(slot, "cat_s"), is_ctor(arg, "cat_s")) {
        if s.len() == 2 && a.len() == 2 {
            return s[0] == a[0] && unify_feat(&s[1], &a[1], subst);
        }
    }
    // Higher-order functors `A/B` (`fwd`) and `A\B` (`bwd`), D63 §8.2 item 4:
    // structural subsumption with the standard function variance — the **result**
    // `A` is covariant, the **argument** `B` is contravariant. So an `S\NP_Entity`
    // VP fills an `S\NP_Gene` slot (`Gene ≤ Entity` ⇒ `Entity→Prop ≤ Gene→Prop`):
    // the argument check is run with operands SWAPPED. (Args are `[result, arg]` —
    // `⟦fwd(a,b)⟧ = ⟦b⟧ → ⟦a⟧`.) A functor only matches the same slash direction.
    for slash in ["fwd", "bwd"] {
        if let (Some(s), Some(a)) = (is_ctor(slot, slash), is_ctor(arg, slash)) {
            if s.len() == 2 && a.len() == 2 {
                return unify_into(&s[0], &a[0], layer, subst)   // result: covariant
                    && unify_into(&a[1], &s[1], layer, subst); // argument: contravariant
            }
        }
    }
    // Atoms of differing constructors / slashes of opposite direction never match.
    slot == arg
}

/// Unify a type-index position. A slot `Exp::Var` binds to the argument's type
/// (occurs-consistently: a repeated variable must bind the same type); a concrete
/// slot type must subsume the argument type per the subclass lattice.
fn unify_type(slot: &Exp, arg: &Exp, layer: &Arc<Layer>, subst: &mut CatSubst) -> bool {
    if let Exp::Var(name) = slot {
        match subst.get(name) {
            Some(bound) => bound == arg,
            None => {
                subst.insert(name.clone(), arg.clone());
                true
            }
        }
    } else {
        type_subsumes(slot, arg, layer)
    }
}

/// Substitute schematic category type-variables (`Exp::Var`) throughout a
/// category term — applied to the *result* category after [`unify_cat`] binds the
/// slot's variables (so the determiner's `T` flows into the produced category).
pub fn subst_cat(cat: &Exp, subst: &CatSubst) -> Exp {
    match cat {
        Exp::Var(name) => subst.get(name).cloned().unwrap_or_else(|| cat.clone()),
        Exp::InductiveCtor(decl, name, args) => Exp::InductiveCtor(
            decl.clone(),
            name.clone(),
            args.iter().map(|a| subst_cat(a, subst)).collect(),
        ),
        other => other.clone(),
    }
}

/// An argument of type `sub` fills a slot of type `sup` iff `sub` is `sup` or a
/// reflexive-transitive subclass of it (the foundation authority
/// [`Layer::is_subclass_of`]); non-class atoms must match exactly.
fn type_subsumes(sup: &Exp, sub: &Exp, layer: &Arc<Layer>) -> bool {
    match (sup, sub) {
        (Exp::EigonClass(sup), Exp::EigonClass(sub)) => layer.is_subclass_of(sub, sup),
        _ => sup == sub,
    }
}

/// Feature-meet (D63 §5.1): two feature values unify iff equal or either is the
/// underspecified top (`*_any`). `Any = ⊤`, unification = meet (`⊓`). Public so
/// `apply` can check determiner/noun number agreement on `cat_forall`.
pub fn feat_meets(a: &Exp, b: &Exp) -> bool {
    a == b || is_any_feat(a) || is_any_feat(b)
}

/// Feature **unification** (D63 §8.10) — the binding-aware generalization of
/// [`feat_meets`], parallel to [`unify_type`] for the type index. A feature
/// **variable** (`Exp::Var`, introduced by `cat_fin_forall` / `cat_num_forall` and
/// freed at seed time) binds — occurs-consistently — to the other side's feature,
/// and the binding propagates into the result via [`subst_cat`]; so the object
/// determiner carries the consumed verb's real finiteness / subject-number through
/// to the VP it produces, instead of laundering it to `*_any`. The variable may be
/// on EITHER side (the `bwd` argument check swaps operands — contravariance).
/// Concrete-vs-concrete falls back to the meet.
fn unify_feat(slot: &Exp, arg: &Exp, subst: &mut CatSubst) -> bool {
    for (var_side, other) in [(slot, arg), (arg, slot)] {
        if let Exp::Var(name) = var_side {
            return match subst.get(name) {
                Some(bound) => bound == other,
                None => {
                    subst.insert(name.clone(), other.clone());
                    true
                }
            };
        }
    }
    feat_meets(slot, arg)
}

fn is_any_feat(e: &Exp) -> bool {
    matches!(e, Exp::InductiveCtor(_, name, args)
        if args.is_empty() && matches!(name.as_str(), "num_any" | "fin_any"))
}

// ── Generalized coordination (D63 §8.4 Phase 3) ──────────────────────

/// Resolve an inductive (e.g. `logic:And` / `logic:Or`, or `lexicon:Conn`) from
/// the layer to its decl, so the combinator can build its terms.
pub(crate) fn resolve_inductive(layer: &Arc<Layer>, iri_str: &str) -> Option<Arc<InductiveDecl>> {
    let iri = Iri::parse(iri_str).ok()?;
    let resource = layer.resolve(&iri)?;
    match crate::program::ground::resolve_inductive_type(&iri, &resource, layer).ok()? {
        Val::InductiveType { decl, .. } => Some(decl),
        _ => None,
    }
}

/// The transparent **adverb modifier** categories (D62 Phase 3 — `docs/notes/d62-adverb-semantics-decision.md`).
/// A productive `-ly` adverb seeds these, each with an identity sem, so the clause composes and the
/// adverb contributes nothing to the claim `Prop` (the science-transparent default; the
/// measurement subset's obligation semantics is a later arm). Grounded in the WRN attachment
/// positions:
/// 1. **adjective modifier** `(S[adj]\NP)/(S[adj]\NP)` — "selectively essential", "highly concordant";
/// 2. **VP modifier, forward** `(S\NP)/(S\NP)` — "commonly affects …";
/// 3. **VP modifier, backward** `(S\NP)\(S\NP)` — "arrest selectively".
///
/// The VP modifier fixes the clause feature to `fin` (so it matches a *verbal* clause but not an
/// `adj` clause — keeping it disjoint from the adjective modifier, no spurious duplicate parses) and
/// keeps the subject **number** a free variable, so agreement flows through the modifier unchanged.
/// The adjective modifier is fixed (`adj`, `num_any`), since predicative adjectives are uniform.
/// `None` if the `lexicon:Cat`/`Mood`/`Fin`/`Num` inductives don't resolve.
pub fn adverb_modifier_cats(layer: &Arc<Layer>) -> Option<Vec<Exp>> {
    let cat = resolve_inductive(layer, "urn:eigenius:lexicon:Cat")?;
    let mood = resolve_inductive(layer, "urn:eigenius:lexicon:Mood")?;
    let fin = resolve_inductive(layer, "urn:eigenius:lexicon:Fin")?;
    let num = resolve_inductive(layer, "urn:eigenius:lexicon:Num")?;
    let entity = Exp::EigonClass(Iri::parse("urn:eigenius:lexicon:Entity").ok()?);
    let dcl = Exp::InductiveCtor(mood, "dcl".to_string(), vec![]);
    let ctor = |n: &str, args: Vec<Exp>| Exp::InductiveCtor(cat.clone(), n.to_string(), args);

    // 1. Adjective modifier — fixed `adj` / `num_any` (predicative adjectives are uniform).
    let adj = Exp::InductiveCtor(fin.clone(), "adj".to_string(), vec![]);
    let num_any = Exp::InductiveCtor(num.clone(), "num_any".to_string(), vec![]);
    let adjp = ctor(
        "bwd",
        vec![
            ctor("cat_s", vec![dcl.clone(), adj]),
            ctor("cat_np", vec![entity.clone(), num_any]),
        ],
    );
    let adj_mod = ctor("fwd", vec![adjp.clone(), adjp]);

    // 2/3. VP modifier — fixed `fin` clause (verbal, disjoint from `adj`), free subject number.
    let fin_c = Exp::InductiveCtor(fin, "fin".to_string(), vec![]);
    let nvar = Exp::Var("__adv_num".to_string());
    let vp = ctor(
        "bwd",
        vec![
            ctor("cat_s", vec![dcl, fin_c]),
            ctor("cat_np", vec![entity, nvar]),
        ],
    );
    let vp_mod_fwd = ctor("fwd", vec![vp.clone(), vp.clone()]);
    let vp_mod_bwd = ctor("bwd", vec![vp.clone(), vp]);

    Some(vec![adj_mod, vp_mod_fwd, vp_mod_bwd])
}

/// The transparent **sentence modifier** categories `S/S` and `S\S` (D62 Phase 3) — for
/// *discourse* adverbs (`also`, `however`, `yet`) that attach at the clause level
/// (sentence-initial / sentence-final), as in `adv.xsl`'s `Adverb` Initial/Backward entries. The
/// clause feature is `fin_any` so they wrap any finite declarative. Identity sem (transparent).
/// Used in addition to [`adverb_modifier_cats`] for lexicalized discourse adverbs.
pub fn sentence_modifier_cats(layer: &Arc<Layer>) -> Option<Vec<Exp>> {
    let cat = resolve_inductive(layer, "urn:eigenius:lexicon:Cat")?;
    let mood = resolve_inductive(layer, "urn:eigenius:lexicon:Mood")?;
    let fin = resolve_inductive(layer, "urn:eigenius:lexicon:Fin")?;
    let dcl = Exp::InductiveCtor(mood, "dcl".to_string(), vec![]);
    let fin_any = Exp::InductiveCtor(fin, "fin_any".to_string(), vec![]);
    let s = Exp::InductiveCtor(cat.clone(), "cat_s".to_string(), vec![dcl, fin_any]);
    let fwd = Exp::InductiveCtor(cat.clone(), "fwd".to_string(), vec![s.clone(), s.clone()]);
    let bwd = Exp::InductiveCtor(cat, "bwd".to_string(), vec![s.clone(), s]);
    Some(vec![fwd, bwd])
}

/// A denotation is **conjoinable** iff it ends in `Prop` after peeling arrows
/// (`Prop`, `A→Prop`, `A→B→Prop`, …) — the Partee & Rooth conjoinable types.
fn prop_ending(d: &Exp) -> bool {
    match d {
        Exp::Sort(0) => true,
        Exp::Arrow(_, cod) => prop_ending(cod),
        Exp::Pi(_, _, cod) => prop_ending(cod),
        _ => false,
    }
}

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

/// The sem of `a <op> b` for same-category, Prop-ending constituents of category
/// `cat`: the pointwise-lifted `op` (`op_iri` = `logic:And` / `logic:Or`).
/// Returns `None` if `cat` is not conjoinable or the connective doesn't resolve.
pub fn coordinate_sem(
    op_iri: &str,
    cat: &Exp,
    a: &Exp,
    b: &Exp,
    layer: &Arc<Layer>,
) -> Option<Exp> {
    let denote = denote_cat(cat).ok()?;
    let op = resolve_inductive(layer, op_iri)?;
    generalized_coord(&op, &denote, a, b, 0)
}

// ── NP coordination as `List`-groups (D63 §8.4 Phase 6) ──────────────

/// The least common supertype of two category type-indices, walking the subclass
/// lattice (`core:subclass_of`). For two `EigonClass`es, BFS over the left's
/// ancestors (closest first) returns the first that the right is also `≤` — so
/// `common_super(CellLine, Gene) = Entity` when both sit under `Entity`. Non-class
/// indices (or a variable) match only if identical. `None` ⇒ the two NPs share no
/// common type, so they do not form a typed group.
pub fn common_super(t1: &Exp, t2: &Exp, layer: &Arc<Layer>) -> Option<Exp> {
    let (Exp::EigonClass(i1), Exp::EigonClass(i2)) = (t1, t2) else {
        return (t1 == t2).then(|| t1.clone());
    };
    let parent_prop = Iri::parse(crate::ontology::well_known::PARENT_CLASSES).ok()?;
    // BFS over i1's ancestors (i1 first), returning the first that i2 ≤ it.
    let mut queue = std::collections::VecDeque::from([i1.clone()]);
    let mut seen = std::collections::BTreeSet::new();
    while let Some(cur) = queue.pop_front() {
        if !seen.insert(cur.clone()) {
            continue;
        }
        if layer.is_subclass_of(i2, &cur) {
            return Some(Exp::EigonClass(cur));
        }
        if let Some(def) = layer.resolve(&cur) {
            if let Some(parents) = def.get(&parent_prop) {
                queue.extend(parents.as_iri_array());
            }
        }
    }
    None
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
        _ => None,
    }
}

/// Coordinate two NP-side constituents into a **group** (`cat_group(C, conn, pl)`
/// over a `List C` sem) under the connective `op_iri` (`logic:And`/`logic:Or`).
/// Handles `NP·NP` (a fresh 2-member group) and `Group·NP` (append, the
/// left-branching n-ary case); the members are re-typed at the new common supertype
/// `C`. Returns `(group_cat, group_sem)`, or `None` if the constituents aren't
/// NP/group, share no common type, mix connectives, or `op_iri` isn't a connective.
pub fn coordinate_np(
    op_iri: &str,
    l_cat: &Exp,
    l_sem: &Exp,
    r_cat: &Exp,
    r_sem: &Exp,
    layer: &Arc<Layer>,
) -> Option<(Exp, Exp)> {
    // The right conjunct is always a single NP (left-branching: a group never sits
    // on the right — enforced by the caller, mirroring the `is_coordination` guard).
    let [rt, _rn] = is_ctor(r_cat, "cat_np")? else {
        return None;
    };
    let conn_name = match op_iri {
        "urn:eigenius:logic:And" => "conn_and",
        "urn:eigenius:logic:Or" => "conn_or",
        _ => return None,
    };
    let (cat_decl, num_decl) = match l_cat {
        Exp::InductiveCtor(d, n, args) if n == "cat_np" || n == "cat_group" => {
            // `cat_np`'s number is arg 1; `cat_group`'s is arg 2 (after Conn).
            let num = if n == "cat_group" {
                args.get(2)
            } else {
                args.get(1)
            }?;
            let Exp::InductiveCtor(nd, _, _) = num else {
                return None;
            };
            (d.clone(), nd.clone())
        }
        _ => return None,
    };
    let (lt, members): (Exp, Vec<Exp>) = match l_cat {
        c if is_ctor(c, "cat_np").is_some() => {
            (is_ctor(c, "cat_np")?[0].clone(), vec![l_sem.clone()])
        }
        // A left group must share the connective (no `X and Y or Z` mixing).
        c if is_ctor(c, "cat_group").is_some() => {
            if group_conn_op(c)? != op_iri {
                return None;
            }
            (is_ctor(c, "cat_group")?[0].clone(), group_members(l_sem)?)
        }
        _ => return None,
    };
    let c = common_super(&lt, rt, layer)?;
    let mut all = members;
    all.push(r_sem.clone());
    let conn = Exp::InductiveCtor(
        resolve_inductive(layer, "urn:eigenius:lexicon:Conn")?,
        conn_name.into(),
        vec![],
    );
    let pl = Exp::InductiveCtor(num_decl, "pl".into(), vec![]);
    let group_cat = Exp::InductiveCtor(cat_decl, "cat_group".into(), vec![c, conn, pl]);
    Some((group_cat, list_term(&all)))
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
    let op = resolve_inductive(layer, group_conn_op(group_cat)?)?;
    let preds = members
        .into_iter()
        .map(|m| Exp::App(Box::new(pred_sem.clone()), Box::new(m)))
        .collect();
    fold_conn(&op, preds)
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
    let op = resolve_inductive(layer, group_conn_op(group_cat)?)?;
    let s = Exp::Var("__dist_subj".into());
    let preds = members
        .into_iter()
        .map(|m| {
            Exp::App(
                Box::new(Exp::App(Box::new(tv_sem.clone()), Box::new(m))),
                Box::new(s.clone()),
            )
        })
        .collect();
    let body = fold_conn(&op, preds)?;
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

    // `denote_cat` matches on the constructor NAME + args (the decl Arc is erased),
    // so a directly-built `cat_group` ctor is faithful here.
    fn ctor(name: &str, args: Vec<Exp>) -> Exp {
        Exp::InductiveCtor(list_decl(), name.into(), args)
    }

    #[test]
    fn feature_variable_binds_meets_and_is_occurs_consistent() {
        // D63 §8.10 — `unify_feat`: a feature VARIABLE binds to the other side's
        // concrete feature (either side — contravariance), occurs-consistently; a
        // concrete pair falls back to the `*_any` meet.
        let f = Exp::Var("f".into());
        let (fin, bse, sg, any) = (
            ctor("fin", vec![]),
            ctor("bse", vec![]),
            ctor("sg", vec![]),
            ctor("num_any", vec![]),
        );

        let mut subst = CatSubst::new();
        assert!(unify_feat(&f, &fin, &mut subst), "var binds to concrete");
        assert_eq!(subst.get("f"), Some(&fin));
        assert!(
            unify_feat(&f, &fin, &mut subst),
            "rebinding the same value is consistent"
        );
        assert!(
            !unify_feat(&f, &bse, &mut subst),
            "f is bound to fin — it cannot also be bse"
        );

        // The variable may be on the ARGUMENT side (the bwd contravariant swap).
        let mut s2 = CatSubst::new();
        assert!(unify_feat(&fin, &Exp::Var("g".into()), &mut s2));
        assert_eq!(s2.get("g"), Some(&fin));

        // Concrete vs concrete → the meet: `*_any` = ⊤, distinct values fail.
        let mut s3 = CatSubst::new();
        assert!(unify_feat(&any, &sg, &mut s3), "num_any meets sg");
        assert!(!unify_feat(&fin, &bse, &mut s3), "fin does not meet bse");
    }

    #[test]
    fn feature_binder_is_denotation_transparent() {
        // ⟦cat_fin_forall(λf. cat_s(dcl, f))⟧ = ⟦cat_s(dcl, _)⟧ = Prop — the binder is
        // erased by `⟦·⟧` (features never appear in the denotation), so it never adds a
        // Π and the determiner's sem_type is unchanged.
        let inner = ctor("cat_s", vec![ctor("dcl", vec![]), Exp::Var("f".into())]);
        let cat = ctor(
            "cat_fin_forall",
            vec![Exp::Lam(Patt::Var("f".into()), Box::new(inner))],
        );
        assert_eq!(
            denote_cat(&cat).expect("feature binder denotes"),
            Exp::Sort(0),
            "the feature binder must be denotation-transparent (⟦·⟧ = Prop)"
        );
    }

    #[test]
    fn group_denotes_a_list_of_its_common_type() {
        // ⟦cat_group(C, conn, num)⟧ = List C — connective + number erased. (Guards
        // the arity of the `cat_group` denotation arm against the 3-arg ctor.)
        let gene = Exp::EigonClass(Iri::parse("urn:eigenius:lexicon:Gene").unwrap());
        let group = ctor(
            "cat_group",
            vec![gene.clone(), ctor("conn_and", vec![]), ctor("pl", vec![])],
        );
        assert_eq!(
            denote_cat(&group).expect("group denotes"),
            Exp::InductiveType(list_decl(), vec![gene]),
            "⟦cat_group(Gene, _, _)⟧ must be List Gene"
        );
    }
}
