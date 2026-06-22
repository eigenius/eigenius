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

//! The D62 categorial composition engine over `lexicon:Cat` (design: D62 §8.6)
//! — the trusted half of the prose → typed-trees engine. Pure functions over
//! kernel types (`Exp`, `Layer`):
//!
//! - [`denote_cat`] — the homomorphism `⟦·⟧ : Cat → EigenTT type`
//!   (`⟦cat_s⟧ = Prop`, `⟦cat_n⟧ = Set`, `⟦cat_np(T)⟧ = T`,
//!   `⟦A/B⟧ = ⟦A\B⟧ = ⟦B⟧ → ⟦A⟧`).
//! - [`gate_entry`] — the **felicity gate**: admit a (draft) lexical entry iff
//!   `⟦cat⟧ ≡ sem_type` *and* the entry's `sem` inhabits `⟦cat⟧`. This is the
//!   trusted filter the *untrusted* LLM proposer's drafts pass through: an
//!   ad-hoc tool reads WordNet/VerbNet, has an LLM draft entries as Eigon-JSON,
//!   and the kernel admits or rejects each via this gate at ingestion.
//! - [`apply`] / [`cky_parse`] — categorial composition (forward/backward
//!   application on the category, `App` on the sem, in lockstep).
//!
//! The kernel is the felicity *oracle*; the LLM is never trusted, only gated.

use std::sync::Arc;

use crate::layer::Layer;
use crate::nbe::check::{check_infer, CheckCtx};
use crate::nbe::env::Rho;
use crate::nbe::eval::eval;
use crate::nbe::readback::readback_val;
use crate::nbe::term::Exp;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::Iri;
use crate::program::eigentt_type_mirror::decode_type;

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("valid lexicon iri")
}

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

/// Resolve an entry's `sem` reference to its EigenTT *value*: an axiom →
/// `EigonAxiom`, a class → `EigonClass`, an instance → `EigonResource`. Chain
/// entities become values the checker can type, not unbound `Var`s.
pub fn resolve_sem(layer: &Arc<Layer>, target: &Iri) -> Exp {
    let r = layer
        .resolve(target)
        .unwrap_or_else(|| panic!("sem target not found: {target}"));
    if r.is_instance_of(&iri("urn:eigenius:eigentt:Axiom")) {
        Exp::EigonAxiom(target.clone())
    } else if r.is_instance_of(&iri("urn:eigenius:core:Class")) {
        Exp::EigonClass(target.clone())
    } else {
        Exp::EigonResource(Box::new((*r).clone()))
    }
}

/// The felicity gate: admit a lexical entry iff its category and semantics
/// agree. Checks `⟦cat⟧ ≡ sem_type` and that the entry's `sem` actually
/// inhabits `⟦cat⟧`. Returns the derived `⟦cat⟧` on admit; a reason on reject.
/// This is the trusted filter every LLM-drafted entry must pass.
pub fn gate_entry(layer: &Arc<Layer>, entry: &Resource) -> Result<Exp, String> {
    let cat_v = entry
        .get(&iri("urn:eigenius:lexicon:cat"))
        .ok_or("entry has no `cat`")?;
    let st_v = entry
        .get(&iri("urn:eigenius:lexicon:sem_type"))
        .ok_or("entry has no `sem_type`")?;

    let cat = decode_type(cat_v, layer).map_err(|e| format!("cat decode: {e:?}"))?;
    let denoted = denote_cat(&cat)?;
    let sem_type = decode_type(st_v, layer).map_err(|e| format!("sem_type decode: {e:?}"))?;
    if !type_eq(&denoted, &sem_type) {
        return Err(format!(
            "⟦cat⟧ ≠ sem_type: ⟦cat⟧ = {denoted:?}, sem_type = {sem_type:?}"
        ));
    }

    // The sem must actually inhabit ⟦cat⟧ (not merely match the declared type).
    let sem_target = match entry.get(&iri("urn:eigenius:lexicon:sem")) {
        Some(Value::ResourceRef(i)) => i.clone(),
        Some(Value::String(s)) => Iri::parse(s).map_err(|e| format!("sem iri: {e}"))?,
        other => return Err(format!("entry `sem` is not a reference: {other:?}")),
    };
    let sem = resolve_sem(layer, &sem_target);
    let mut ctx = CheckCtx::with_layer(Rho::Nil, Vec::new(), Arc::clone(layer));
    let sem_ty =
        check_infer(&mut ctx, &sem).map_err(|e| format!("sem does not type-check: {e}"))?;
    if !type_eq(&readback_val(0, &sem_ty), &denoted) {
        return Err("typeof(sem) ≠ ⟦cat⟧".to_string());
    }
    Ok(denoted)
}

/// A parse item: a category (`lexicon:Cat` term) and its assembled EigenTT sem.
#[derive(Clone)]
pub struct Item {
    pub cat: Exp,
    pub sem: Exp,
}

/// If `cat` is the named `lexicon:Cat` constructor, return its arguments.
pub fn is_ctor<'a>(cat: &'a Exp, name: &str) -> Option<&'a [Exp]> {
    match cat {
        Exp::InductiveCtor(_, n, args) if n.as_str() == name => Some(args),
        _ => None,
    }
}

/// Build a parse item (category + resolved sem) from a committed lexical entry.
pub fn entry_to_item(layer: &Arc<Layer>, entry: &Resource) -> Result<Item, String> {
    let cat_v = entry
        .get(&iri("urn:eigenius:lexicon:cat"))
        .ok_or("entry has no `cat`")?;
    let cat = decode_type(cat_v, layer).map_err(|e| format!("cat decode: {e:?}"))?;
    let sem_target = match entry.get(&iri("urn:eigenius:lexicon:sem")) {
        Some(Value::ResourceRef(i)) => i.clone(),
        Some(Value::String(s)) => Iri::parse(s).map_err(|e| format!("sem iri: {e}"))?,
        other => return Err(format!("entry `sem` is not a reference: {other:?}")),
    };
    Ok(Item {
        cat,
        sem: resolve_sem(layer, &sem_target),
    })
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

/// One combinatory step: forward (`A/B · B → A`) or backward (`B · A\B → A`),
/// assembling the sem by application in lockstep. The argument category need not
/// equal the slot — it must *subsume into* it ([`cat_subsumes`]), so an
/// `NP[Gene]` fills an `NP[Entity]` slot. A non-match returns `None` — the
/// parse-time felicity filter, on the category alone.
pub fn apply(left: &Item, right: &Item, layer: &Arc<Layer>) -> Option<Item> {
    if let Some(args) = is_ctor(&left.cat, "fwd") {
        if args.len() == 2 && cat_subsumes(&args[1], &right.cat, layer) {
            return Some(Item {
                cat: args[0].clone(),
                sem: Exp::App(Box::new(left.sem.clone()), Box::new(right.sem.clone())),
            });
        }
    }
    if let Some(args) = is_ctor(&right.cat, "bwd") {
        if args.len() == 2 && cat_subsumes(&args[1], &left.cat, layer) {
            return Some(Item {
                cat: args[0].clone(),
                sem: Exp::App(Box::new(right.sem.clone()), Box::new(left.sem.clone())),
            });
        }
    }
    None
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
