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
        // ⟦Coord(B)[_]⟧ = List ⟦B⟧ — a coordinated PROP-ending group (clauses / VPs / predicative
        // adjectives / TVs) denotes the member-retaining list over its base category's denotation
        // (D63 §8.4 Phase 3, the list-with-operator model ported from core-en `conj.xsl`). The
        // connective is erased by ⟦·⟧; a list-completion (`complete_coord`) folds the members into
        // ⟦B⟧ with the operator. Parallel to `cat_group` (which lists an ENTITY type; this lists a
        // prop-ending category's denotation).
        ("cat_coord", [b, _conn]) => Ok(Exp::InductiveType(list_decl(), vec![denote_cat(b)?])),
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
        // ⟦PP[arg]⟧ = Entity — an argument (oblique-complement) PP supplies the verb's second
        // ENTITY argument (D63 verb+PP frames). The marker (`to`/`from`/`on`/`with`) is transparent
        // (`cat_pp_arg / cat_np(Entity)`, sem `λy. y`); a subcategorizing verb is `(S\NP)/cat_pp_arg`,
        // sem `λy.λx. R(x, y)`. Distinct from a bare NP so only a PP-frame verb accepts it (`affect`,
        // a plain `(S\NP)/NP`, still rejects `to X`). Same denotation as `cat_pp_than`.
        ("cat_pp_arg", []) => Ok(Exp::EigonClass(
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

/// The `lexicon:Entity` top type — the only *concrete* type index a functor argument slot may carry
/// while remaining index-INDEPENDENT (`type_subsumes(Entity, X)` holds for the whole noun lattice).
const ENTITY_TOP_IRI: &str = "urn:eigenius:lexicon:Entity";

/// Does this category impose a **selectional restriction** — a functor ARGUMENT slot whose type
/// index is a concrete class *other than* `Entity` (i.e. not a type variable and not the `Entity`
/// top)? Such a slot makes combinability **index-dependent** ([`unify_type`] does concrete
/// subsumption on it), so node-level packing by `cat_shape` — which erases the index — would be
/// UNSOUND (D63 packed-forest blueprint §4, Option A). The grammar-load guard flags a grammar with
/// any such slot and routes it to the unpacked CKY path; an index-independent grammar (every functor
/// arg is a variable or `Entity`, as the WordNet/UMLS importer emits) is safe to pack.
///
/// Only ARGUMENT positions count — the `B` in `fwd(A, B)` / `bwd(A, B)`, recursively (a nested
/// functor argument, e.g. a VP-adjunct's `S\NP`, has its own arg slots). A plain noun leaf
/// `cat_n(Gene, sg)` is an *argument*, not a *slot*, so its concrete index does **not** flag.
pub fn cat_has_selectional_slot(cat: &Exp) -> bool {
    if let Exp::InductiveCtor(_, name, args) = cat {
        if (name == "fwd" || name == "bwd") && args.len() == 2 {
            // args[0] = result (covariant, may nest functors); args[1] = the argument slot.
            return slot_is_concrete_nonentity(&args[1])
                || cat_has_selectional_slot(&args[0])
                || cat_has_selectional_slot(&args[1]);
        }
    }
    false
}

/// Whether `slot` is a `cat_np`/`cat_n` whose type index is a concrete class other than `Entity`
/// (a variable or the `Entity` top returns `false` — those are index-independent).
fn slot_is_concrete_nonentity(slot: &Exp) -> bool {
    for ctor in ["cat_np", "cat_n"] {
        if let Some([ty, _num]) = is_ctor(slot, ctor) {
            return matches!(ty, Exp::EigonClass(iri) if iri.as_str() != ENTITY_TOP_IRI);
        }
    }
    false
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
/// The **predicative adjective** category `S[adj]\NP` = `bwd(cat_s(dcl, adj), cat_np(Entity, num_any))`
/// — fixed `adj` / `num_any`, since predicative adjectives are uniform. Shared by the adverb
/// adjective-modifier cat ([`adverb_modifier_cats`]) and the D63 denominal `X-based` adjective
/// (`docs/notes/d63-compound-morphology.md` §3, Slice 2). `None` if the inductives don't resolve.
pub fn predicative_adjective_cat(layer: &Arc<Layer>) -> Option<Exp> {
    let cat = resolve_inductive(layer, "urn:eigenius:lexicon:Cat")?;
    let mood = resolve_inductive(layer, "urn:eigenius:lexicon:Mood")?;
    let fin = resolve_inductive(layer, "urn:eigenius:lexicon:Fin")?;
    let num = resolve_inductive(layer, "urn:eigenius:lexicon:Num")?;
    let entity = Exp::EigonClass(Iri::parse("urn:eigenius:lexicon:Entity").ok()?);
    let dcl = Exp::InductiveCtor(mood, "dcl".to_string(), vec![]);
    let adj = Exp::InductiveCtor(fin, "adj".to_string(), vec![]);
    let num_any = Exp::InductiveCtor(num, "num_any".to_string(), vec![]);
    let ctor = |n: &str, args: Vec<Exp>| Exp::InductiveCtor(cat.clone(), n.to_string(), args);
    Some(ctor(
        "bwd",
        vec![
            ctor("cat_s", vec![dcl, adj]),
            ctor("cat_np", vec![entity, num_any]),
        ],
    ))
}

pub fn adverb_modifier_cats(layer: &Arc<Layer>) -> Option<Vec<Exp>> {
    let cat = resolve_inductive(layer, "urn:eigenius:lexicon:Cat")?;
    let mood = resolve_inductive(layer, "urn:eigenius:lexicon:Mood")?;
    let fin = resolve_inductive(layer, "urn:eigenius:lexicon:Fin")?;
    let entity = Exp::EigonClass(Iri::parse("urn:eigenius:lexicon:Entity").ok()?);
    let dcl = Exp::InductiveCtor(mood, "dcl".to_string(), vec![]);
    let ctor = |n: &str, args: Vec<Exp>| Exp::InductiveCtor(cat.clone(), n.to_string(), args);

    // 1. Adjective modifier — over the uniform predicative-adjective cat `S[adj]\NP`.
    let adjp = predicative_adjective_cat(layer)?;
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
fn sem_is_coordination(sem: &Exp) -> bool {
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
    // The right conjunct must coordinate with the base category (same category, prop-ending).
    if !cats_coordinate(&base_cat, r_cat, layer) {
        return None;
    }
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
    // The right conjunct is always a single NP (left-branching: a group never sits
    // on the right — enforced by the caller, mirroring the `is_coordination` guard).
    let [rt, _rn] = is_ctor(r_cat, "cat_np")? else {
        return None;
    };
    let conn_name = match op_iri {
        "urn:eigenius:logic:And" => "conn_and",
        "urn:eigenius:logic:Or" => "conn_or",
        LIST_CONN => "conn_list",
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
        // A neutral `conn_list` left group takes ANY op (the trailing `and`/`or` rebinds it); a
        // finalized left group must share the op's connective (no `X and Y or Z` mixing).
        c if is_ctor(c, "cat_group").is_some() => {
            let left_conn = group_conn_name(c)?;
            if left_conn != "conn_list" && left_conn != conn_name {
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

    // ── cat_has_selectional_slot (D63 Option A grammar-load guard, blueprint §11 3b) ──
    fn np(ty: Exp) -> Exp {
        ctor("cat_np", vec![ty, ctor("num_any", vec![])])
    }
    fn cls(iri: &str) -> Exp {
        Exp::EigonClass(Iri::parse(iri).unwrap())
    }
    fn decl_s() -> Exp {
        ctor("cat_s", vec![ctor("dcl", vec![]), ctor("fin", vec![])])
    }

    #[test]
    fn generic_entity_verb_has_no_selectional_slot() {
        // `(S\NP_Entity)/NP_Entity` — the WordNet/UMLS importer's shape: index-INDEPENDENT.
        let entity = cls("urn:eigenius:lexicon:Entity");
        let vp = ctor("bwd", vec![decl_s(), np(entity.clone())]);
        let verb = ctor("fwd", vec![vp, np(entity)]);
        assert!(!cat_has_selectional_slot(&verb));
    }

    #[test]
    fn concrete_subtype_slot_is_selectional() {
        // `(S\NP_CellLine)/NP_Gene` — the demo `depends_on`: index-DEPENDENT ⇒ unpackable.
        let vp = ctor(
            "bwd",
            vec![decl_s(), np(cls("urn:eigenius:lexicon:CellLine"))],
        );
        let verb = ctor("fwd", vec![vp, np(cls("urn:eigenius:lexicon:Gene"))]);
        assert!(cat_has_selectional_slot(&verb));
    }

    #[test]
    fn type_variable_slot_is_not_selectional() {
        // A schematic slot (`Exp::Var`) binds to anything ⇒ index-independent.
        let vp = ctor("bwd", vec![decl_s(), np(Exp::Var("T".into()))]);
        let verb = ctor("fwd", vec![vp, np(Exp::Var("T".into()))]);
        assert!(!cat_has_selectional_slot(&verb));
    }

    #[test]
    fn plain_noun_leaf_is_an_argument_not_a_slot() {
        // `cat_n(Gene, sg)` is an ARGUMENT, not a functor arg SLOT ⇒ its concrete index must NOT flag.
        let noun = ctor(
            "cat_n",
            vec![cls("urn:eigenius:lexicon:Gene"), ctor("sg", vec![])],
        );
        assert!(!cat_has_selectional_slot(&noun));
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
