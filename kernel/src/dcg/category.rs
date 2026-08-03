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
use crate::nbe::term::{list_decl, Exp, InductiveDecl, Name};
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
        // ⟦PP[arg](prep)⟧ = Entity — an argument (oblique-complement) PP supplies the verb's second
        // ENTITY argument (D63 verb+PP frames). The marker (`to`/`from`/`on`/`with`/…) is transparent
        // (`cat_pp_arg(prep) / cat_np(Entity)`, sem `λy. y`); a subcategorizing verb is
        // `(S\NP)/cat_pp_arg(any)`, sem `λy.λx. R(x, y)`. Distinct from a bare NP so only a PP-frame
        // verb accepts it. The `prep` feature (C3-precision) is erased by ⟦·⟧ — it only gates the
        // feature-meet during composition. Same denotation as `cat_pp_than`.
        ("cat_pp_arg", [_prep]) => Ok(Exp::EigonClass(
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
        // ⟦Measure⟧ = Entity → float — a 1-place measure maps an entity to its scalar value on a
        // dimension's opaque float scale (D63 §8.12 phrasal comparatives, d63-comparative-phrasal.md).
        // The comparative quantifier `greater`/`fewer` selects this; `μ(x) : float` feeds
        // `measurements:gt`. Distinct from `cat_pp` (Entity → Prop): a measure is a scalar.
        ("cat_measure", []) => Ok(Exp::Arrow(
            Box::new(Exp::EigonClass(
                Iri::parse("urn:eigenius:lexicon:Entity").map_err(|e| e.to_string())?,
            )),
            Box::new(Exp::EigonPrimitive(crate::nbe::term::PrimitiveType::Float)),
        )),
        // ⟦A/ₘB⟧ = ⟦A\ₘB⟧ = ⟦B⟧→⟦A⟧. The slash MODALITY `_m` is denotation-transparent: it
        // restricts which combinatory rules may consume the slash, never what it denotes.
        ("fwd", [_m, a, b]) | ("bwd", [_m, a, b]) => Ok(Exp::Arrow(
            Box::new(denote_cat(b)?),
            Box::new(denote_cat(a)?),
        )),
        // The multimodal migration's release-mode detector. `denote_cat` runs on every category,
        // so a construction site still building a 2-argument slash is caught here and NAMED,
        // rather than silently denoting as some other shape (D63 multimodal slashes).
        ("fwd", [_, _]) | ("bwd", [_, _]) => Err(format!(
            "`{name}` built with 2 arguments — expected 3 (mode, result, argument). A category \
             construction site was not migrated to multimodal slashes."
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

/// The parts of a **slash** category — `A/ₘB` (`dir = "fwd"`) or `A\ₘB` (`dir = "bwd"`) — as
/// `(mode, result, argument)`.
///
/// This is the ONLY sanctioned way to destructure a slash, and it exists because the multimodal
/// migration is not compiler-checked: `Exp::InductiveCtor` carries its constructor name as a
/// *string* and its arguments as a `Vec`, so a construction site left at the old 2-argument arity
/// produces a term Rust type-checks happily. The scattered `is_ctor(c, "fwd")` + `len() == 2`
/// idiom this replaces would have answered such a term with a silent "not a functor" — functor
/// subsumption would simply stop firing, corrupting derivations rather than failing. Routing every
/// destructuring through one accessor keeps that impossible.
pub fn slash_parts<'a>(cat: &'a Exp, dir: &str) -> Option<(&'a Exp, &'a Exp, &'a Exp)> {
    debug_assert!(
        dir == "fwd" || dir == "bwd",
        "slash_parts expects a slash constructor, got `{dir}`"
    );
    match is_ctor(cat, dir)? {
        [m, a, b] => Some((m, a, b)),
        // A stale 2-argument slash: report it, never treat it as a non-functor.
        stale => {
            debug_assert!(
                false,
                "`{dir}` built with arity {} — expected 3 (mode, result, argument). A category \
                 construction site was not migrated to multimodal slashes.",
                stale.len()
            );
            None
        }
    }
}

/// Whether `cat` is a slash in either direction, as `(dir, mode, result, argument)`.
pub fn as_slash(cat: &Exp) -> Option<(&'static str, &Exp, &Exp, &Exp)> {
    if let Some((m, a, b)) = slash_parts(cat, "fwd") {
        return Some(("fwd", m, a, b));
    }
    slash_parts(cat, "bwd").map(|(m, a, b)| ("bwd", m, a, b))
}

/// `·` — all rules. The most permissive point of the mode lattice (its BOTTOM, as an inheritance
/// hierarchy) and the migration default: it reproduces the pre-multimodal regime exactly, where
/// every slash was composable.
pub const MODE_ALL: &str = "m_all";
/// `⋆` — application only. The lattice ROOT, and the least permissive slash: composition,
/// type-raising and the crossed rules cannot consume it. core-en marks a phrasal verb's particle
/// slash this way (`v.xsl`'s `tv.phrasal` is `iv /▷ np[acc] /★ prt`), which is what stops a
/// governed complement from composing away from its head.
pub const MODE_APP: &str = "m_app";

/// A `lexicon:Mode` constructor value by name — see [`MODE_ALL`] / [`MODE_APP`].
/// `None` if the `lexicon:Mode` inductive doesn't resolve in the layer chain.
pub fn mode_value(layer: &Arc<Layer>, name: &str) -> Option<Exp> {
    Some(Exp::InductiveCtor(
        resolve_inductive(layer, "urn:eigenius:lexicon:Mode")?,
        name.to_string(),
        vec![],
    ))
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
    // cat_pp_arg(prep): the oblique-argument PP carries a preposition feature (D63 §5.3
    // C3-precision). Unify it by the feature-meet: a verb's `prep_any` wildcard meets any
    // marker's prep, while a gloss-governed adjective's specific `prep_on` meets only the `on`
    // marker — so `dependent on X` composes but `*dependent to X` fails.
    if let (Some(s), Some(a)) = (is_ctor(slot, "cat_pp_arg"), is_ctor(arg, "cat_pp_arg")) {
        if s.len() == 1 && a.len() == 1 {
            return unify_feat(&s[0], &a[0], subst);
        }
    }
    // Higher-order functors `A/B` (`fwd`) and `A\B` (`bwd`), D63 §8.2 item 4:
    // structural subsumption with the standard function variance — the **result**
    // `A` is covariant, the **argument** `B` is contravariant. So an `S\NP_Entity`
    // VP fills an `S\NP_Gene` slot (`Gene ≤ Entity` ⇒ `Entity→Prop ≤ Gene→Prop`):
    // the argument check is run with operands SWAPPED. (Args are `[result, arg]` —
    // `⟦fwd(a,b)⟧ = ⟦b⟧ → ⟦a⟧`.) A functor only matches the same slash direction.
    //
    // The slash MODALITY is deliberately NOT unified here. A mode keys which combinatory RULES may
    // consume a slash (Baldridge 2002 §5.2 — application is keyed to the root `⋆`, harmonic
    // composition to `⋄`, and so on); it says nothing about whether one category may FILL another's
    // argument slot. `A/⋆B` and `A/·B` both take a `B` and yield an `A`. Rule licensing is enforced
    // where rules fire (`rules::combinators`), not here.
    for slash in ["fwd", "bwd"] {
        if let (Some((_sm, s_res, s_arg)), Some((_am, a_res, a_arg))) =
            (slash_parts(slot, slash), slash_parts(arg, slash))
        {
            return unify_into(s_res, a_res, layer, subst)   // result: covariant
                && unify_into(a_arg, s_arg, layer, subst); // argument: contravariant
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

/// A **rule-dispatch pattern** — the structural matcher deciding WHICH grammar rule fires. It is a
/// third, distinct category operation, not [`unify_cat`] and not [`common_cat`]: those are the
/// grammar's subtype ALGEBRA (combination = subsumption + feature-meet; coordination = lattice join),
/// whereas dispatch is *exact structural matching* of a trigger. Crucially it binds metavariables in
/// ANY position — including whole subcategories, which [`unify_cat`] cannot (it binds only type-index
/// / feature slots, and *meets* features rather than matching them). See
/// `docs/notes/grammar-formalization-plan.md`.
#[derive(Debug, Clone)]
pub enum CatPat {
    /// Match an [`Exp::InductiveCtor`] by NAME + arity — the `decl` is ignored, exactly as
    /// [`is_ctor`] does — then match each argument against the corresponding sub-pattern.
    Ctor(&'static str, Vec<CatPat>),
    /// Bind the matched subterm to `name` (non-linear: a repeated name must bind an equal term). The
    /// reserved name `"_"` is an anonymous wildcard — matches anything, binds nothing, never checked.
    Var(&'static str),
}

/// Match `pat` against category `cat`, threading bindings into `binds` (so a rule matches its left
/// pattern then its right pattern into the SAME map — a metavariable shared across operands must bind
/// equal terms). Returns whether the whole pattern matched. Exact: no subsumption, no feature-meet.
/// See [`CatPat`].
pub fn match_cat(pat: &CatPat, cat: &Exp, binds: &mut CatSubst) -> bool {
    match pat {
        CatPat::Var(name) if *name == "_" => true,
        CatPat::Var(name) => match binds.get(*name) {
            Some(bound) => bound == cat,
            None => {
                binds.insert((*name).to_string(), cat.clone());
                true
            }
        },
        CatPat::Ctor(name, args) => {
            if let Exp::InductiveCtor(_, cname, cargs) = cat {
                cname == name
                    && cargs.len() == args.len()
                    && args.iter().zip(cargs).all(|(p, c)| match_cat(p, c, binds))
            } else {
                false
            }
        }
    }
}

/// An argument of type `sub` fills a slot of type `sup` iff `sub` is `sup` or a
/// reflexive-transitive subclass of it (the foundation authority
/// [`Layer::is_subclass_of`]); non-class atoms must match exactly.
pub(crate) fn type_subsumes(sup: &Exp, sub: &Exp, layer: &Arc<Layer>) -> bool {
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
/// Only ARGUMENT positions count — the `B` in `fwd(m, A, B)` / `bwd(m, A, B)`, recursively (a nested
/// functor argument, e.g. a VP-adjunct's `S\NP`, has its own arg slots). A plain noun leaf
/// `cat_n(Gene, sg)` is an *argument*, not a *slot*, so its concrete index does **not** flag. The
/// slash modality is not a category and never carries an index, so it is skipped.
pub fn cat_has_selectional_slot(cat: &Exp) -> bool {
    if let Some((_dir, _mode, res, arg)) = as_slash(cat) {
        return slot_is_concrete_nonentity(arg)
            || cat_has_selectional_slot(res)
            || cat_has_selectional_slot(arg);
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
    a == b || is_any_feat(a) || is_any_feat(b) || pred_subsumes_adj(a, b)
}

/// `pred ⊑ adj` — the ONE non-flat pair in the feature lattice, and the reason it exists.
///
/// A PREDICATE NOMINAL is a predicative complement, so everything that selects one must accept it:
/// the copula, negation, and — the open set — every WordNet adverb typed
/// `(S[adj]\NP)/(S[adj]\NP)`. Enumerating that set is not possible from the closed-class file, and
/// trying to (four `_prednom` copulas + `not_adj_prednom`) left `grammar-gap 1` on «These
/// observations suggest that WRN dependency is not simply a result of MMR deficiency.», where
/// `simply` is exactly such an adverb.
///
/// What `pred` must NOT do is license ATTRIBUTIVE use: English has no bare attributive predicate
/// nominal (*"a drug-target cancer"). That is a separate test — [`is_adjective_cat`] matches the
/// ctor name EXACTLY, so it keeps refusing `pred` no matter what the meet admits. Subsumption for
/// selection, exact match for attribution; the two questions are asked in different places and this
/// is the pair that makes them come apart.
fn pred_subsumes_adj(a: &Exp, b: &Exp) -> bool {
    fn name(e: &Exp) -> Option<&str> {
        match e {
            Exp::InductiveCtor(_, n, args) if args.is_empty() => Some(n.as_str()),
            _ => None,
        }
    }
    matches!(
        (name(a), name(b)),
        (Some("adj"), Some("pred")) | (Some("pred"), Some("adj"))
    )
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
        if args.is_empty() && matches!(name.as_str(), "num_any" | "fin_any" | "prep_any"))
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
/// 1. **pre-modifier, forward** `(S[f]\NP[n])/(S[f]\NP[n])` — "selectively essential", "highly
///    concordant" (`f = adj`), "commonly affects …" (`f = fin`);
/// 2. **VP modifier, backward** `(S[fin]\NP[n])\(S[fin]\NP[n])` — "arrest selectively".
///
/// The forward modifier BINDS the clause feature, so it returns exactly the feature it consumed.
/// It used to be two categories with the result feature FIXED — `adj` for the adjective modifier and
/// `fin` for the forward VP modifier — and the `adj` one LAUNDERED: once `pred ⊑ adj` entered
/// [`feat_meets`], it took a predicate nominal by subsumption and handed back a plain `adj`, which
/// [`is_adjective_cat`] then accepted for the attributive lift. That is the same shape as `fin_any`
/// laundering finiteness (`3ae672d`): a rule `X → X` over a feature it does not carry through.
/// Binding is the fix, and it also collapses the two into one — a bound `f` covers `fin` as well, so
/// keeping a separate `fin`-fixed forward category would only duplicate every VP-adverb derivation.
///
/// The BACKWARD modifier stays fixed at `fin`. It never laundered (it accepts `fin` and returns
/// `fin`), and post-adjectival adverbs are not attested on the reference page, so binding it would
/// widen coverage on no evidence.
///
/// The subject **number** is a free variable throughout, so agreement flows through the modifier
/// unchanged. `None` if the `lexicon:Cat`/`Mood`/`Fin` inductives don't resolve.
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
    // `m_all`: the predicative-adjective category keeps the pre-multimodal permissive regime.
    let m_all = mode_value(layer, MODE_ALL)?;
    Some(ctor(
        "bwd",
        vec![
            m_all,
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

    // Adverb modifier categories stay `m_all` — an adverb is exactly the case that SHOULD compose
    // freely (Baldridge's `skillfully` is `(s\◁np)/▷(s\◁np)`, both slashes permutative).
    let m_all = mode_value(layer, MODE_ALL)?;

    let nvar = Exp::Var("__adv_num".to_string());
    let clause = |feat: Exp| {
        ctor(
            "bwd",
            vec![
                m_all.clone(),
                ctor("cat_s", vec![dcl.clone(), feat]),
                ctor("cat_np", vec![entity.clone(), nvar.clone()]),
            ],
        )
    };

    // 1. Forward pre-modifier — the clause feature is BOUND, so the adverb hands back whatever it
    //    consumed (`adj`, `pred`, `fin`, …) instead of collapsing it to one value.
    let bound = clause(Exp::Var("__adv_fin".to_string()));
    let fwd_mod = ctor("fwd", vec![m_all.clone(), bound.clone(), bound]);

    // 2. Backward VP modifier — verbal only; returns the `fin` it accepts, so nothing to bind.
    let vp = clause(Exp::InductiveCtor(fin, "fin".to_string(), vec![]));
    let vp_mod_bwd = ctor("bwd", vec![m_all, vp.clone(), vp]);

    Some(vec![fwd_mod, vp_mod_bwd])
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
    let m_all = mode_value(layer, MODE_ALL)?;
    let fwd = Exp::InductiveCtor(
        cat.clone(),
        "fwd".to_string(),
        vec![m_all.clone(), s.clone(), s.clone()],
    );
    let bwd = Exp::InductiveCtor(cat, "bwd".to_string(), vec![m_all, s.clone(), s]);
    Some(vec![fwd, bwd])
}

/// A denotation is **conjoinable** iff it ends in `Prop` after peeling arrows
/// (`Prop`, `A→Prop`, `A→B→Prop`, …) — the Partee & Rooth conjoinable types.
pub(crate) fn prop_ending(d: &Exp) -> bool {
    match d {
        Exp::Sort(0) => true,
        Exp::Arrow(_, cod) => prop_ending(cod),
        Exp::Pi(_, _, cod) => prop_ending(cod),
        _ => false,
    }
}

/// Least common generalization of two categories that share STRUCTURE, widening each corresponding
/// `cat_np`/`cat_n` **type index** to its [`common_super`]; every other position (ctor, feature,
/// nested functor, mood) must match exactly. Used to coordinate **type-raised quantifiers over
/// different noun types** (D63 §8.4 — `a gene or a cell line`, whose object-GQ categories differ only
/// in the exposed object slot `cat_np(Gene)` vs `cat_np(CellLine)`): the coordinated category widens
/// that slot to `cat_np(Entity)` so a general verb still fills it, while the per-member semantics —
/// each quantifier over its own type — are preserved and folded pointwise ([`complete_coord`]). So
/// `V [a gene or a cell line]` yields `∃g:Gene.V(g) ∨ ∃c:CellLine.V(c)` (the two bound types stay
/// distinct; only the categorial selectional slot generalizes — a general verb accepts it, a
/// type-restricted verb over-generates, the documented corner). `None` if the structures differ or a
/// type pair has no common supertype.
pub(crate) fn common_cat(x: &Exp, y: &Exp, layer: &Arc<Layer>) -> Option<Exp> {
    if x == y {
        return Some(x.clone());
    }
    let (Exp::InductiveCtor(dx, nx, ax), Exp::InductiveCtor(_dy, ny, ay)) = (x, y) else {
        return None;
    };
    if nx != ny || ax.len() != ay.len() {
        return None;
    }
    // `cat_np(T, num)` / `cat_n(T, num)`: widen the type index to the common supertype; the number
    // feature must match (both raised object slots carry `num_any`, so this holds for the GQ case).
    if (nx == "cat_np" || nx == "cat_n") && ax.len() == 2 {
        if ax[1] != ay[1] {
            return None;
        }
        let t = common_super(&ax[0], &ay[0], layer)?;
        return Some(Exp::InductiveCtor(
            dx.clone(),
            nx.clone(),
            vec![t, ax[1].clone()],
        ));
    }
    // Structural (`fwd`/`bwd`/`cat_s`/…): ctor + arity already match; recurse on corresponding args.
    let mut args = Vec::with_capacity(ax.len());
    for (a, b) in ax.iter().zip(ay.iter()) {
        args.push(common_cat(a, b, layer)?);
    }
    Some(Exp::InductiveCtor(dx.clone(), nx.clone(), args))
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

// ── D1: the modifier-restrictor discriminator (nominal-modification NF) ──────────────────────────
//
// The Chatzikyriakidis & Luo intersective / subsective / gradable / privative test
// (`docs/notes/d63-nominal-modification-normal-form.md` §5, the in-repo Coq App. A7; §8 D1), run on
// our own EigenTT terms rather than a hand-maintained adjective list. The nominal-modification NF may
// reorder/collapse a modifier over a compound head ONLY when the modifier is *strictly intersective*
// (set intersection ⇒ bracketing-invariant, §5). A gradable adjective is covertly subsective (its
// standard is comparison-class-dependent — Kamp & Partee 1995), so it is screened. Keyed on the axiom
// vocabulary the importer actually emits (`measurements:gt`/`lt`, `deg_*`, `std_*`; `convert.rs`
// `push_adj`) and on term shape — pure, no layer lookup.

/// The semantic class of an attributive modifier, deciding NF collapse-eligibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifierClass {
    /// Plain, noun-type-independent predicate (`Entity → Prop`; C&L `Irish : Human → Prop`,
    /// `{h:> Human; I: Irish h}`). Intersective modification is predicate conjunction, so bracketing
    /// and order are invariant — the ONLY class the NF may collapse.
    Intersective,
    /// A degree compared against a standard: the positive `measurements:gt(deg_X(x), std_X)` or the
    /// comparative `…, deg_X(y))` (C&L `tall h := ge (height h) (STND …)`). Comparison-class-dependent
    /// ⇒ covertly subsective ⇒ NOT collapsible; the cheap first-cut screen (catches `attractive`).
    Gradable,
    /// CN-polymorphic predicate — the restrictor is parameterized by the head class (an `EigonClass`
    /// appears in it; C&L `skilful : ∀A:CN, A → Prop`, applied `skilful Man m`). Subsective ⇒ not
    /// collapsible. (The current WordNet/UMLS importer emits none — all adjectives are `Entity → Prop`;
    /// recognized for faithfulness + future lexica.)
    Subsective,
    /// Sum-membership (C&L `fake`, a `match … | inl | inr` over a disjoint sum). Privative ⇒ not
    /// collapsible.
    Privative,
    /// Unrecognized shape — conservatively NOT collapsible (fail-safe: the NF never collapses what it
    /// cannot prove intersective).
    Unknown,
}

impl ModifierClass {
    /// The NF may reorder/collapse a modifier over a compound head iff it is strictly intersective
    /// (§5). Every other class — gradable, subsective, privative, unrecognized — is left in place.
    pub fn is_collapsible(self) -> bool {
        matches!(self, ModifierClass::Intersective)
    }
}

/// Classify an attributive modifier's semantics — the predicate an adjective carries (the `left.sem()`
/// at the `Attrib` combine, an `Entity → Prop` functor) — by the shape of its restrictor. Priority:
/// gradable (the sharpest, cheapest marker — the degree machinery) → privative (disjoint-sum
/// elimination) → subsective (CN-polymorphic, head-class in the restrictor) → intersective (a clean
/// first-order predicate) → `Unknown` (fail-safe). If a modifier is already a conjunction of stacked
/// adjectives, any gradable/subsective conjunct dominates (the checks recurse).
pub fn modifier_class(adj_sem: &Exp) -> ModifierClass {
    // Strip the entity binder(s) to reach the predicate body (mirrors `sem_is_coordination`).
    let mut body = adj_sem;
    while let Exp::Lam(_, inner) = body {
        body = inner;
    }
    if exp_any(body, &is_degree_axiom) {
        return ModifierClass::Gradable;
    }
    if exp_any(body, &|e| matches!(e, Exp::Case(_) | Exp::Data(_))) {
        return ModifierClass::Privative;
    }
    if exp_any(body, &|e| matches!(e, Exp::EigonClass(_))) {
        return ModifierClass::Subsective;
    }
    if is_first_order_predicate(body) {
        return ModifierClass::Intersective;
    }
    ModifierClass::Unknown
}

/// A gradable adjective is built from the degree machinery: the opaque float ordering
/// `urn:eigenius:measurements:gt`/`lt`, a measure `…:deg_*`, or a standard `…:std_*`.
fn is_degree_axiom(e: &Exp) -> bool {
    matches!(e, Exp::EigonAxiom(iri)
        if iri.as_str() == "urn:eigenius:measurements:gt"
            || iri.as_str() == "urn:eigenius:measurements:lt"
            || iri.as_str().contains(":deg_")
            || iri.as_str().contains(":std_"))
}

/// True if `pred` holds at `e` or any of its subterms (the variants an adjective sem can contain).
fn exp_any(e: &Exp, pred: &dyn Fn(&Exp) -> bool) -> bool {
    if pred(e) {
        return true;
    }
    match e {
        Exp::App(a, b)
        | Exp::Pi(_, a, b)
        | Exp::Sig(_, a, b)
        | Exp::Arrow(a, b)
        | Exp::Times(a, b)
        | Exp::Pair(a, b)
        | Exp::Ann(a, b) => exp_any(a, pred) || exp_any(b, pred),
        Exp::Lam(_, b) | Exp::Fst(b) | Exp::Snd(b) | Exp::Con(_, b) | Exp::Refl(b) => {
            exp_any(b, pred)
        }
        Exp::InductiveType(_, args) | Exp::InductiveCtor(_, _, args) => {
            args.iter().any(|x| exp_any(x, pred))
        }
        Exp::Id(a, b, c) | Exp::DecEq(a, b, c) => {
            exp_any(a, pred) || exp_any(b, pred) || exp_any(c, pred)
        }
        _ => false,
    }
}

/// True if `e` is a first-order predicate expression — atoms (axioms, variables, resources,
/// literals) combined by application and the logical connectives — with no binder, type-former,
/// projection, or sum. This is the intersective shape: a plain `Entity → Prop` restrictor.
fn is_first_order_predicate(e: &Exp) -> bool {
    match e {
        Exp::EigonAxiom(_)
        | Exp::Var(_)
        | Exp::EigonResource(_)
        | Exp::EigonPrimitive(_)
        | Exp::LitString(_)
        | Exp::LitInt(_)
        | Exp::Unit => true,
        Exp::App(f, a) => is_first_order_predicate(f) && is_first_order_predicate(a),
        Exp::Con(_, b) => is_first_order_predicate(b),
        Exp::InductiveType(d, args) => {
            matches!(
                d.iri.as_str(),
                "urn:eigenius:logic:And" | "urn:eigenius:logic:Or" | "urn:eigenius:logic:Not"
            ) && args.iter().all(is_first_order_predicate)
        }
        _ => false,
    }
}

// ── Category predicates + sem builders that the RULES and the CHART ask about a category ──────────
//
// These lived in `lookup/mod.rs` — the bridge — which meant the chart drivers and the rule registry had
// to reach into the *parser* to ask a question about a *category*. None of them touches a lexicon, a
// chart, or a config; they are pure functions of a `Cat` (or of the type it indexes). They belong here,
// with the rest of the Cat algebra.

/// The head constructor of a determiner cat's `cat_forall(num, λT. body)` body — `"fwd"` for a
/// type-raised **subject** determiner (`S/(S\NP)`), `"bwd"` for an in-situ **object** determiner.
/// Selects the subject vs object deferred-quantifier sem in the bare-plural shift. `None` if `cat`
/// is not a `cat_forall(_, λ. <fwd|bwd>…)`.
pub(super) fn cat_forall_body_head(cat: &Exp) -> Option<&'static str> {
    if let Some([_num, Exp::Lam(_, inner)]) = is_ctor(cat, "cat_forall") {
        return match inner.as_ref() {
            Exp::InductiveCtor(_, name, _) if name == "fwd" => Some("fwd"),
            Exp::InductiveCtor(_, name, _) if name == "bwd" => Some("bwd"),
            _ => None,
        };
    }
    None
}

/// A `kind_of(A)` application — the class value `A` (a `Set`) realized as the `Entity` that is that
/// kind (Chierchia's ∩; the axiom `ontology:kind_of : Set -> Entity`, D63 kind-predication reshape).
pub(super) fn kind_of(a: Exp) -> Exp {
    Exp::App(
        Box::new(Exp::EigonAxiom(
            Iri::parse("urn:eigenius:ontology:kind_of").expect("static kind_of IRI"),
        )),
        Box::new(a),
    )
}

/// The base (non-refined) class of a common-noun type: peel `Σx:C. R` down to `C` (recursively, for
/// stacked refinements), else the type itself. A bare kind NP's raised category is indexed by this base
/// so it sits in the subsumption lattice (`C ≤ Entity`), while its sem nominalizes the WHOLE type
/// (`kind_of(Σx:C. R)`) — D63 kind-predication reshape §7.4 ([`LexicalIndex::kind_raised_nps`]).
pub(super) fn base_class(t: &Exp) -> Exp {
    match t {
        Exp::Sig(_, base, _) => base_class(base),
        other => other.clone(),
    }
}

/// Whether `cat` is a sentence PRE-modifier `S/S` (`fwd(cat_s, cat_s)`) — the category a fronted
/// transitional adverb / participial adjunct carries. Used by the fronted-modifier comma absorption.
pub(super) fn is_sentence_premod(cat: &Exp) -> bool {
    matches!(slash_parts(cat, "fwd"),
        Some((_m, a, b)) if is_ctor(a, "cat_s").is_some() && is_ctor(b, "cat_s").is_some())
}

/// Whether `cat` is a VP-adjunct preposition `((S\NP)\(S\NP))/NP` (`fwd(bwd(VP,VP), NP)`) — as
/// opposed to the `cat_pp / NP` noun-modifier reading. Used by pied-piping (#2B) to pick the prep
/// whose sem (`λx.λV.λs. And(V(s), prep(s,x))`) threads the fronted antecedent into the VP.
pub(super) fn is_vp_adjunct_prep(cat: &Exp) -> bool {
    matches!(slash_parts(cat, "fwd"),
        Some((_m, res, np)) if is_ctor(res, "bwd").is_some() && is_ctor(np, "cat_np").is_some())
}

/// Whether `cat` **governs a named preposition** — `X/cat_pp_arg(prep_R)` for a CONCRETE `prep_R`.
///
/// This is the lexical signature of a gloss-governed relational word: `concordant WITH`, `dependent
/// ON`, `essential FOR`, `associated WITH`. The importer writes the governance into the category —
/// [`push_adj`](eigenius_wordnet)'s relational adjective and the stative relational participle both
/// emit it — so the fact is readable here without consulting `adjective-frames.tsv`.
///
/// `prep_any` is EXCLUDED and that exclusion is the whole precision of this test. The wildcard is what
/// `FrameKind::PpOblique` emits for WordNet's preposition-AGNOSTIC PP frames ("----s PP"), where no
/// preposition is named and nothing is governed. Admitting it would match every oblique-PP verb in the
/// lexicon instead of the handful of words whose frame names a specific marker.
/// The RESULT must be a predicative ADJECTIVE (`S[adj]\NP`), which is what makes this an
/// adjective-governance test rather than a relational-word test. Dropping that requirement was
/// measured and REFUTED (2026-08-03): `X/cat_pp_arg(prep_R)` alone also matches relational NOUNS —
/// `deficiency in`, `dependency on`, `dependence on`, `result of`, `vulnerability to`,
/// `co-occurrence of` — whose nominal reading is the correct one, so pruning it took `grammar-gap`
/// 0 -> 9 and expected-hits 60 -> 49 on the reference page.
pub(super) fn governs_named_preposition(cat: &Exp) -> bool {
    let Some((_m, res, arg)) = slash_parts(cat, "fwd") else {
        return false;
    };
    if !is_adjective_cat(res) {
        return false;
    }
    let Some([prep]) = is_ctor(arg, "cat_pp_arg") else {
        return false;
    };
    matches!(prep, Exp::InductiveCtor(_, n, _) if n != "prep_any")
}

/// Whether a category is a **predicative adjective** `S[adj]\NP` — `bwd(cat_s(_, adj), _)`. Used to
/// confirm a derived `-ly` adverb's base is a known adjective (D62 Phase 3).
pub(super) fn is_adjective_cat(cat: &Exp) -> bool {
    if let Some((_m, s, _np)) = slash_parts(cat, "bwd") {
        if let Some([_mood, fin]) = is_ctor(s, "cat_s") {
            return matches!(fin, Exp::InductiveCtor(_, n, _) if n == "adj");
        }
    }
    false
}

/// Whether `cat` is a **binary relation** verb — `(S\NP)/NP` (transitive) or `(S\NP)/cat_pp_arg`
/// (argument-PP, e.g. `depend on`) — both carrying a raw 2-place `Entity → Entity → Prop` axiom as
/// their sem. Used by the denominal-suffix rule (D63 compound morphology §3b) to fetch each element's
/// relation from its verb lemma.
pub(super) fn is_binary_relation_cat(cat: &Exp) -> bool {
    let Some((_m, inner, obj)) = slash_parts(cat, "fwd") else {
        return false;
    };
    if is_ctor(obj, "cat_np").is_none() && is_ctor(obj, "cat_pp_arg").is_none() {
        return false;
    }
    let Some((_im, s, subj)) = slash_parts(inner, "bwd") else {
        return false;
    };
    is_ctor(s, "cat_s").is_some() && is_ctor(subj, "cat_np").is_some()
}

/// Compact category rendering for probe output: nested constructor names, no decl inlining.
pub fn pretty_cat_dbg(c: &Exp) -> String {
    match c {
        Exp::InductiveCtor(_, n, args) if args.is_empty() => n.clone(),
        Exp::InductiveCtor(_, n, args) => format!(
            "{n}({})",
            args.iter()
                .map(pretty_cat_dbg)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Exp::Var(v) => v.clone(),
        Exp::EigonClass(_) => "C".to_string(),
        Exp::Lam(_, b) => format!("λ.{}", pretty_cat_dbg(b)),
        Exp::Sig(..) => "Σ".to_string(),
        _ => "_".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbe::term::Patt;

    /// A GOVERNED preposition names a marker; `prep_any` names nothing.
    ///
    /// [`governs_named_preposition`] is the gate on the gloss-governed-relational nominal prune
    /// (`dcg::parse::seed`), and its entire precision is that `prep_any` is refused. The wildcard is
    /// what `FrameKind::PpOblique` emits for WordNet's preposition-AGNOSTIC PP frames, so admitting it
    /// would prune the nominal reading of every oblique-PP verb surface in the lexicon rather than the
    /// handful of relational words whose frame names a specific marker. Bootstrap-free: the categories
    /// are built directly so this pins the predicate, not a snapshot.
    #[test]
    fn governed_preposition_test_refuses_the_prep_any_wildcard() {
        fn ctor(name: &str, args: Vec<Exp>) -> Exp {
            // A minimal `lexicon:Cat` inductive — the predicate only reads ctor names.
            let decl = Arc::new(InductiveDecl {
                iri: Iri::parse("urn:eigenius:lexicon:Cat").expect("iri"),
                name: "Cat".into(),
                params: vec![],
                indices: vec![],
                sort: Exp::Sort(0),
                ctors: vec![],
            });
            Exp::InductiveCtor(decl, name.to_string(), args)
        }
        let vp = ctor(
            "bwd",
            vec![
                ctor("m_all", vec![]),
                ctor("cat_s", vec![ctor("dcl", vec![]), ctor("adj", vec![])]),
                ctor(
                    "cat_np",
                    vec![ctor("Entity", vec![]), ctor("num_any", vec![])],
                ),
            ],
        );
        let with_prep = |p: &str| {
            ctor(
                "fwd",
                vec![
                    ctor("m_all", vec![]),
                    vp.clone(),
                    ctor("cat_pp_arg", vec![ctor(p, vec![])]),
                ],
            )
        };
        // `concordant with` / `essential for` — the frame NAMES the marker.
        assert!(governs_named_preposition(&with_prep("prep_with")));
        assert!(governs_named_preposition(&with_prep("prep_for")));
        // `FrameKind::PpOblique` — the frame records no preposition, so nothing is governed.
        assert!(!governs_named_preposition(&with_prep("prep_any")));
        // A plain predicative adjective takes no PP argument at all.
        assert!(!governs_named_preposition(&vp));
    }

    /// Every `lexicon:Conn` constructor the coordination rules can CONSTRUCT must be DECLARED in the
    /// ontology.
    ///
    /// `conn_list` — the neutral connective a bare comma contributes ([`LIST_CONN`]) — was built by
    /// [`coordinate_prop`] / [`coordinate_np`] but was never declared in `data lexicon:Conn`. It
    /// survived only because `⟦·⟧` ERASES the connective, so a `Cat` term carrying it never reaches
    /// the type-checker — while `check` rejects an undeclared constructor outright ("no constructor
    /// `conn_list` in `Conn`", `nbe/check/inductive.rs`). So the parser was building a term the kernel
    /// would refuse, and only the erasure hid it. This pins the invariant so the gap cannot reopen —
    /// including for a connective added later.
    #[test]
    fn every_connective_the_parser_builds_is_declared_in_the_ontology() {
        let ctx = crate::bootstrap::bootstrap().expect("bootstrap");
        let decl =
            resolve_inductive(ctx.head(), "urn:eigenius:lexicon:Conn").expect("Conn resolves");
        let declared: Vec<&str> = decl.ctors.iter().map(|c| c.name.as_str()).collect();
        // The exhaustive set of names the coordination builders can emit: `coordinate_prop` /
        // `coordinate_np` map an `op_iri` to one of the first three; `coordinate_but_not` emits the last.
        for built in ["conn_and", "conn_or", "conn_list", "conn_but_not"] {
            assert!(
                declared.contains(&built),
                "the parser CONSTRUCTS `{built}`, but `data lexicon:Conn` declares only {declared:?} \
                 — `check` rejects an undeclared constructor, so this is live the moment anything \
                 type-checks a Cat term (today only `⟦·⟧`'s erasure hides it)"
            );
        }
    }

    /// An adverb must hand back the clause feature it consumed.
    ///
    /// `pred ⊑ adj` ([`pred_subsumes_adj`]) makes SELECTION permissive by design — the copula,
    /// negation and every adverb have to accept a predicate nominal. The cost is that any rule which
    /// accepts `adj` and returns a FIXED `adj` launders `pred` into `adj`, and [`is_adjective_cat`]
    /// then admits the laundered result for the attributive lift (`mod_lifts`), reopening exactly the
    /// hole `lexicon:pred` was introduced to close. The forward adverb modifier was that rule.
    ///
    /// The cure is a BOUND feature variable shared by result and argument, so this pins the structure
    /// rather than the measurement it produced (`invalid` 11 → 6, 2026-08-02).
    #[test]
    fn the_forward_adverb_modifier_binds_the_clause_feature_it_consumes() {
        let ctx = crate::bootstrap::bootstrap().expect("bootstrap");
        let cats = adverb_modifier_cats(ctx.head()).expect("adverb modifier cats resolve");

        /// `bwd(m, cat_s(mood, FEAT), cat_np(…))` → `FEAT`.
        fn clause_feat(c: &Exp) -> Option<&Exp> {
            let (_m, s, _np) = slash_parts(c, "bwd")?;
            match is_ctor(s, "cat_s")? {
                [_mood, feat] => Some(feat),
                _ => None,
            }
        }
        fn feat_name(f: &Exp) -> Option<&str> {
            match f {
                Exp::InductiveCtor(_, n, args) if args.is_empty() => Some(n.as_str()),
                _ => None,
            }
        }

        let fwd = cats
            .iter()
            .find_map(|c| slash_parts(c, "fwd"))
            .expect("a forward adverb modifier is seeded");
        let (_m, res, arg) = fwd;
        let rf = clause_feat(res).expect("the forward modifier's RESULT is a clause");
        let af = clause_feat(arg).expect("the forward modifier's ARGUMENT is a clause");
        assert!(
            matches!(rf, Exp::Var(_)),
            "the forward adverb's result clause feature must be a VARIABLE — a fixed value launders \
             every feature that reaches it by subsumption; got {rf:?}"
        );
        assert_eq!(
            rf, af,
            "result and argument must share ONE variable, or `unify_feat` has nothing to propagate"
        );

        // No seeded adverb category may FIX `adj`: that is the laundering shape, and `pred` reaches
        // any such slot through the meet.
        for c in &cats {
            let (_dir, _m, res, arg) = as_slash(c).expect("every adverb category is a slash");
            for side in [res, arg] {
                if let Some(f) = clause_feat(side).and_then(feat_name) {
                    assert_ne!(
                        f, "adj",
                        "adverb category {c:?} fixes the clause feature to `adj`, which `pred` \
                         subsumes into — it will launder a predicate nominal back into an \
                         attributive-capable adjective"
                    );
                }
            }
        }
    }

    // `denote_cat` matches on the constructor NAME + args (the decl Arc is erased),
    // so a directly-built `cat_group` ctor is faithful here.
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

    // ── D1 modifier-restrictor discriminator (nominal-modification NF §5/§8 D1) ──
    fn ax(iri: &str) -> Exp {
        Exp::EigonAxiom(Iri::parse(iri).expect("iri"))
    }
    fn lam(x: &str, body: Exp) -> Exp {
        Exp::Lam(Patt::Var(x.into()), Box::new(body))
    }
    fn app(f: Exp, a: Exp) -> Exp {
        Exp::App(Box::new(f), Box::new(a))
    }
    fn and2(a: Exp, b: Exp) -> Exp {
        // A minimal `logic:And` inductive — `modifier_class` only reads `decl.iri`.
        let and = Arc::new(InductiveDecl {
            iri: Iri::parse("urn:eigenius:logic:And").expect("iri"),
            name: "And".into(),
            params: vec![],
            indices: vec![],
            sort: Exp::Sort(0),
            ctors: vec![],
        });
        Exp::InductiveType(and, vec![a, b])
    }

    #[test]
    fn modifier_class_screens_gradable_positive_adjective() {
        // `attractive`: λx. measurements:gt(wn:deg_X(x), wn:std_X) — the `convert.rs` positive form.
        let sem = lam(
            "x",
            app(
                app(
                    ax("urn:eigenius:measurements:gt"),
                    app(ax("urn:eigenius:wn:deg_a00166146"), Exp::Var("x".into())),
                ),
                ax("urn:eigenius:wn:std_a00166146"),
            ),
        );
        assert_eq!(modifier_class(&sem), ModifierClass::Gradable);
        assert!(
            !modifier_class(&sem).is_collapsible(),
            "relative gradables (attractive) are screened, not collapsed"
        );
    }

    #[test]
    fn modifier_class_screens_gradable_comparative() {
        // `stronger`: λx. measurements:gt(wn:deg_X(x), wn:deg_X(anaphor)) — degree vs degree.
        let sem = lam(
            "x",
            app(
                app(
                    ax("urn:eigenius:measurements:gt"),
                    app(ax("urn:eigenius:wn:deg_a01"), Exp::Var("x".into())),
                ),
                app(
                    ax("urn:eigenius:wn:deg_a01"),
                    ax("urn:eigenius:lexicon:anaphor"),
                ),
            ),
        );
        assert_eq!(modifier_class(&sem), ModifierClass::Gradable);
    }

    #[test]
    fn modifier_class_collapses_intersective_boolean_adjective() {
        // C&L `Irish : Human → Prop`: a plain predicate. Applied and bare-axiom forms both collapse.
        let applied = lam(
            "x",
            app(ax("urn:eigenius:wn:is_irish"), Exp::Var("x".into())),
        );
        assert_eq!(modifier_class(&applied), ModifierClass::Intersective);
        assert!(modifier_class(&applied).is_collapsible());
        assert_eq!(
            modifier_class(&ax("urn:eigenius:wn:is_irish")),
            ModifierClass::Intersective,
            "a bare Entity→Prop axiom is an intersective predicate constant"
        );
    }

    #[test]
    fn modifier_class_conjunction_gradable_dominates() {
        let x = || Exp::Var("x".into());
        let p = app(ax("urn:eigenius:wn:is_human"), x());
        let q = app(ax("urn:eigenius:wn:is_colorectal"), x());
        // λx. And(is_human(x), is_colorectal(x)) — a stack of two intersectives stays collapsible.
        let and_ii = lam("x", and2(p.clone(), q));
        assert_eq!(modifier_class(&and_ii), ModifierClass::Intersective);
        // λx. And(is_human(x), gt(deg(x), std)) — one gradable conjunct screens the whole.
        let grad = app(
            app(
                ax("urn:eigenius:measurements:gt"),
                app(ax("urn:eigenius:wn:deg_a01"), x()),
            ),
            ax("urn:eigenius:wn:std_a01"),
        );
        assert_eq!(
            modifier_class(&lam("x", and2(p, grad))),
            ModifierClass::Gradable
        );
    }

    #[test]
    fn modifier_class_flags_subsective_cn_polymorphic() {
        // C&L `skilful Man m`: the restrictor carries the head CLASS (an EigonClass) — CN-dependent.
        let sem = lam(
            "x",
            app(
                app(
                    ax("urn:eigenius:wn:skilful"),
                    Exp::EigonClass(Iri::parse("urn:eigenius:lexicon:Man").expect("iri")),
                ),
                Exp::Var("x".into()),
            ),
        );
        assert_eq!(modifier_class(&sem), ModifierClass::Subsective);
        assert!(!modifier_class(&sem).is_collapsible());
    }

    #[test]
    fn modifier_class_flags_privative_sum() {
        // C&L `fake`: a disjoint-sum eliminator (`match … | inl | inr`) — a `Case` node.
        let sem = lam("x", Exp::Case(vec![]));
        assert_eq!(modifier_class(&sem), ModifierClass::Privative);
        assert!(!modifier_class(&sem).is_collapsible());
    }

    #[test]
    fn only_intersective_is_collapsible() {
        assert!(ModifierClass::Intersective.is_collapsible());
        for c in [
            ModifierClass::Gradable,
            ModifierClass::Subsective,
            ModifierClass::Privative,
            ModifierClass::Unknown,
        ] {
            assert!(!c.is_collapsible(), "{c:?} must not be collapsible");
        }
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
}
