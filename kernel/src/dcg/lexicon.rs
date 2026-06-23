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
use crate::nbe::check::{check_infer, CheckCtx};
use crate::nbe::env::Rho;
use crate::nbe::readback::readback_val;
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

    // The sem must actually inhabit ⟦cat⟧ (not merely match the declared type).
    let sem_target = match entry.get(&iri("urn:eigenius:lexicon:sem")) {
        Some(Value::ResourceRef(i)) => i.clone(),
        Some(Value::String(s)) => Iri::parse(s).map_err(|e| format!("sem iri: {e}"))?,
        other => return Err(format!("entry `sem` is not a reference: {other:?}")),
    };
    // The sem must actually inhabit ⟦cat⟧ (not merely match the declared type).
    let sem = resolve_sem(layer, &sem_target);
    let mut ctx = CheckCtx::with_layer(Rho::Nil, Vec::new(), Arc::clone(layer));
    let sem_ty =
        check_infer(&mut ctx, &sem).map_err(|e| format!("sem does not type-check: {e}"))?;
    if !type_eq(&readback_val(0, &sem_ty), &denoted) {
        return Err("typeof(sem) ≠ ⟦cat⟧".to_string());
    }
    Ok(denoted)
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
