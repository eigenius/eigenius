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

use std::sync::Arc;

use crate::layer::Layer;
use crate::nbe::env::Rho;
use crate::nbe::eval::eval;
use crate::nbe::readback::readback_val;
use crate::nbe::term::Exp;

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
        ("cat_n", [_num]) => Ok(Exp::Sort(1)),        // ⟦N[_]⟧ = Set (num erased)
        ("cat_np", [t, _num]) => Ok(t.clone()),       // ⟦NP(T)[_]⟧ = T (num erased)
        ("fwd", [a, b]) | ("bwd", [a, b]) => Ok(Exp::Arrow(
            Box::new(denote_cat(b)?),
            Box::new(denote_cat(a)?),
        )),
        (n, a) => Err(format!(
            "denote_cat: unexpected ctor `{n}` of arity {}",
            a.len()
        )),
    }
}

/// ⟦mood⟧ — the only feature that alters the denotation (D63 §5.1). A declarative
/// is a `Prop`; question / imperative denotations are deferred (D63 §5.3, Slice 5)
/// and fail closed rather than silently denoting `Prop`.
fn denote_mood(mood: &Exp) -> Result<Exp, String> {
    let Exp::InductiveCtor(_, name, args) = mood else {
        return Err(format!(
            "denote_mood: expected a lexicon:Mood ctor, got {mood:?}"
        ));
    };
    match (name.as_str(), args.as_slice()) {
        ("dcl", []) => Ok(Exp::Sort(0)), // Prop
        ("q" | "imp", []) => Err(format!("⟦S[{name}]⟧ deferred to D63 Slice 5")),
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
/// match by constructor, with two relaxations (D62 §8.6 / D63 §5.1):
/// - an entity atom `cat_np(Sub, _)` fills `cat_np(Super, _)` when `Sub
///   subclass_of* Super` — CN-as-types subsumption (Luo 2012), so a general
///   verb's `NP[Entity]` slot accepts an `NP[Gene]` argument;
/// - the morphosyntactic **features** unify by **meet** (`Any = ⊤`): `sg` fills
///   `sg` or `Any`, never `pl`. Mood matches exactly (it is semantic).
///
/// Reflexive, so exact composition is the `Sub = Super`, equal-features case.
pub fn cat_subsumes(slot: &Exp, arg: &Exp, layer: &Arc<Layer>) -> bool {
    // cat_np(T, num): subclass-subsume the type, meet the number.
    if let (Some(s), Some(a)) = (is_ctor(slot, "cat_np"), is_ctor(arg, "cat_np")) {
        if s.len() == 2 && a.len() == 2 {
            return type_subsumes(&s[0], &a[0], layer) && feat_meets(&s[1], &a[1]);
        }
    }
    // cat_n(num): meet the number.
    if let (Some(s), Some(a)) = (is_ctor(slot, "cat_n"), is_ctor(arg, "cat_n")) {
        if s.len() == 1 && a.len() == 1 {
            return feat_meets(&s[0], &a[0]);
        }
    }
    // cat_s(mood, fin): mood matches exactly (semantic); fin meets.
    if let (Some(s), Some(a)) = (is_ctor(slot, "cat_s"), is_ctor(arg, "cat_s")) {
        if s.len() == 2 && a.len() == 2 {
            return s[0] == a[0] && feat_meets(&s[1], &a[1]);
        }
    }
    // Higher-order (fwd/bwd) argument slots: structural equality for now
    // (refined alongside the T/B combinators in D63 Slice 2+).
    slot == arg
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
/// underspecified top (`*_any`). `Any = ⊤`, unification = meet (`⊓`).
fn feat_meets(a: &Exp, b: &Exp) -> bool {
    a == b || is_any_feat(a) || is_any_feat(b)
}

fn is_any_feat(e: &Exp) -> bool {
    matches!(e, Exp::InductiveCtor(_, name, args)
        if args.is_empty() && matches!(name.as_str(), "num_any" | "fin_any"))
}
