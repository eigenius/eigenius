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
        ("cat_s", []) => Ok(Exp::Sort(0)), // Prop
        ("cat_n", []) => Ok(Exp::Sort(1)), // Set
        ("cat_np", [t]) => Ok(t.clone()),  // ⟦cat_np(T)⟧ = T
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
/// match exactly, except an entity atom `cat_np(Sub)` fills `cat_np(Super)` when
/// `Sub subclass_of* Super` — the CN-as-types subsumption (Luo 2012; D62 §8.6)
/// that lets a general verb's `NP[Entity]` slot accept an `NP[Gene]` argument.
/// Reflexive (`cat_np(T)` fills `cat_np(T)`), so exact composition is the
/// `Sub = Super` case.
pub fn cat_subsumes(slot: &Exp, arg: &Exp, layer: &Arc<Layer>) -> bool {
    if let (Some(s), Some(a)) = (is_ctor(slot, "cat_np"), is_ctor(arg, "cat_np")) {
        if s.len() == 1 && a.len() == 1 {
            return match (&s[0], &a[0]) {
                (Exp::EigonClass(sup), Exp::EigonClass(sub)) => layer.is_subclass_of(sub, sup),
                _ => s[0] == a[0],
            };
        }
    }
    slot == arg
}
