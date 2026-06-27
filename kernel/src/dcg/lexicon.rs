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

//! Lexical-entry handling: resolve an entry's `sem` reference to a value, build a
//! parse item from a committed entry, and the **felicity gate** — the trusted
//! filter every (LLM- or import-) produced entry must pass. The kernel is the
//! felicity oracle.

use std::sync::Arc;

use crate::layer::Layer;
use crate::nbe::check::{check, CheckCtx};
use crate::nbe::env::Rho;
use crate::nbe::eval::eval;
use crate::nbe::term::Exp;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::Iri;
use crate::program::eigentt_type_mirror::decode_type;

use super::category::{denote_cat, type_eq};
use super::parser::Item;

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("valid lexicon iri")
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

/// Resolve an entry's `sem` field *value* to its EigenTT term. `sem` is an
/// EigenTT term with two surface forms:
/// - a **reference** to a chain entity — the common case (a noun's class, a
///   verb's axiom, a named entity's resource), resolved by [`resolve_sem`]; the
///   reference is shorthand for that entity's `ConstRef` term;
/// - an **inline `type_expr` term** — a function word's λ-semantics, e.g. a
///   determiner's `λA:Set. λV:A→Prop. ∀x:A. V(x)`, which has no chain entity to
///   point at; decoded through the D47 codec.
pub fn resolve_sem_value(layer: &Arc<Layer>, sem_v: &Value) -> Result<Exp, String> {
    let target = match sem_v {
        Value::ResourceRef(i) => i.clone(),
        Value::String(s) => Iri::parse(s).map_err(|e| format!("sem iri: {e}"))?,
        // An inline EigenTT term value (rare — references are the norm).
        other => return decode_type(other, layer).map_err(|e| format!("sem decode: {e:?}")),
    };
    // A `lexicon:SemTerm` reference holds an inline λ-term: decode its `term`
    // field. (Any other reference — class / axiom / instance — goes through
    // `resolve_sem`'s entity dispatch.)
    let r = layer
        .resolve(&target)
        .ok_or_else(|| format!("sem target not found: {target}"))?;
    if r.is_instance_of(&iri("urn:eigenius:lexicon:SemTerm")) {
        let term_v = r
            .get(&iri("urn:eigenius:lexicon:term"))
            .ok_or("lexicon:SemTerm has no `term`")?;
        return decode_type(term_v, layer).map_err(|e| format!("sem term decode: {e:?}"));
    }
    Ok(resolve_sem(layer, &target))
}

/// The felicity gate: admit a lexical entry iff its category and semantics
/// agree. Checks `⟦cat⟧ ≡ sem_type` and that the entry's `sem` actually
/// inhabits `⟦cat⟧`. Returns the derived `⟦cat⟧` on admit; a reason on reject.
/// This is the trusted filter every drafted/imported entry must pass.
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

    // The sem must actually inhabit ⟦cat⟧. **Check-mode** (not `check_infer` +
    // exact `type_eq`): a lambda determiner sem checks against its `Pi` type, and a
    // (possibly multi-class) resource checks against its class via the full `is_a`
    // (#91) — neither of which `check_infer` can synthesize.
    let sem_v = entry
        .get(&iri("urn:eigenius:lexicon:sem"))
        .ok_or("entry has no `sem`")?;
    let sem = resolve_sem_value(layer, sem_v)?;
    let mut ctx = CheckCtx::with_layer(Rho::Nil, Vec::new(), Arc::clone(layer));
    let denoted_val = eval(&denoted, &Rho::Nil).map_err(|e| format!("⟦cat⟧ eval: {e}"))?;
    check(&mut ctx, &sem, &denoted_val).map_err(|e| format!("sem does not inhabit ⟦cat⟧: {e}"))?;
    Ok(denoted)
}

/// Build a parse item (category + resolved sem) from a committed lexical entry. The
/// leaf's **cost** is the entry's `lexicon:sense_rank` (D63 §8.7 Stage B) — a 0-based
/// WordNet sense-frequency rank (sense 1 → 0); absent ⇒ 0 (closed-class / demo
/// entries). The parser sums leaf costs, so a parse using more-frequent senses has a
/// lower cost and ranks higher.
pub fn entry_to_item(layer: &Arc<Layer>, entry: &Resource) -> Result<Item, String> {
    let cat_v = entry
        .get(&iri("urn:eigenius:lexicon:cat"))
        .ok_or("entry has no `cat`")?;
    let cat =
        strip_feature_binders(decode_type(cat_v, layer).map_err(|e| format!("cat decode: {e:?}"))?);
    let sem_v = entry
        .get(&iri("urn:eigenius:lexicon:sem"))
        .ok_or("entry has no `sem`")?;
    let sense_rank = entry
        .get(&iri("urn:eigenius:lexicon:sense_rank"))
        .and_then(Value::as_integer)
        .unwrap_or(0)
        .max(0) as u32;
    Ok(Item::with_cost(
        cat,
        resolve_sem_value(layer, sem_v)?,
        super::parser::Cost::from_sense_rank(sense_rank),
    ))
}

/// Peel `cat_fin_forall` / `cat_num_forall` binders off a leaf category (D63 §8.10),
/// leaving its feature variables FREE — so the parser's unifier binds them
/// (call-locally) from the consumed verb's real features and `subst_cat` propagates
/// them into the produced VP. Felicity gating reads the resource's full (binder-
/// wrapped) cat, where `⟦·⟧` erases the binder; only the parse item uses the
/// stripped form. The binder's `Exp::Lam` bound name appears as `Exp::Var` in the
/// body — which is exactly the free feature variable the parser then unifies.
fn strip_feature_binders(cat: Exp) -> Exp {
    if let Exp::InductiveCtor(_, name, args) = &cat {
        if (name.as_str() == "cat_fin_forall" || name.as_str() == "cat_num_forall")
            && args.len() == 1
        {
            if let Exp::Lam(_patt, body) = &args[0] {
                return strip_feature_binders((**body).clone());
            }
        }
    }
    cat
}
