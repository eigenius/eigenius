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

//! D49 ChainWitness synthesis: kernel-side inhabitation of
//! `JustifiedBy.*` predicate positions from the per-layer witness
//! index. Split from `check.rs`.

use super::CheckCtx;
use crate::nbe::readback::readback_val;
use crate::nbe::val::Val;

/// Check the arguments of an inductive constructor application against
/// the constructor's declared types.
///
/// Walks the constructor's Π-telescope, skipping the parameter prefix,
/// and checks each user-supplied argument against the corresponding
/// binder type evaluated in an environment that binds parameters to
/// the supplied param values and earlier args to their values (so a
/// constructor type like `cons : (A:Set) → A → List A → List A` can
/// have its second binder type `List A` reference the first param).
///
/// Used by both the bidirectional `check` arm and the inference path
/// for non-parametric constructors.
/// D49 Phase 6 hook — detect a ChainWitness-predicate expected type
/// at a constructor-arg position and synthesize the witness via the
/// layer's witness index. Returns `Some(witness_val)` on a successful
/// hit, `None` when the expected type isn't a ChainWitness predicate
/// (callers fall through to the standard type-check), and `Err` when
/// the expected type *is* a ChainWitness predicate but synthesis
/// fails (missing layer, missing trace, malformed iri arg).
pub(super) fn try_synthesize_chain_witness(
    ctx: &CheckCtx,
    expected_typ: &Val,
) -> Result<Option<Val>, String> {
    let (decl, indices) = match expected_typ {
        Val::InductiveType { decl, indices, .. } => (decl, indices),
        _ => return Ok(None),
    };
    let category = match chain_witness_category_for_short_name(&decl.name) {
        Some(c) => c,
        None => return Ok(None),
    };

    // The four ChainWitness predicates all have signature
    // `core:string -> Prop -> Prop` (2 indices: iri, P). Mismatch
    // means the chain ontology drifted from the kernel's expectation.
    if indices.len() != 2 {
        return Err(format!(
            "ChainWitness predicate `{}` expected 2 indices (iri, P), got {}",
            decl.name,
            indices.len()
        ));
    }

    let iri_str = match &indices[0] {
        Val::LitString(s) => s.clone(),
        other => {
            return Err(format!(
                "ChainWitness predicate `{}` iri index must be LitString, got {other:?}",
                decl.name
            ));
        }
    };
    let iri = crate::ontology::iri::Iri::parse(&iri_str)
        .map_err(|e| format!("ChainWitness `{}`: invalid iri `{iri_str}`: {e}", decl.name))?;

    let prop_exp = readback_val(ctx.rho.len(), &indices[1]);

    let layer = ctx.layer.as_ref().ok_or_else(|| {
        format!(
            "ChainWitness synthesis for `{}` requires a layer-attached CheckCtx; \
             pure-mode contexts cannot admit chain witnesses",
            decl.name
        )
    })?;

    let witness_val = crate::layer::synthesize_chain_witness(layer, category, &iri, &prop_exp)?;
    Ok(Some(witness_val))
}

/// Map an inductive's short name to its `WitnessCategory` if it is one
/// of the four ChainWitness predicates. The IRIs themselves live under
/// `urn:eigenius:reasoning:ChainWitness:Is*As`; the ESL compiler emits
/// the local-part short name (`IsDeclaredAs`, etc.) onto the
/// `InductiveDecl.name` slot, so the matching is by short name here.
fn chain_witness_category_for_short_name(name: &str) -> Option<crate::witness::WitnessCategory> {
    use crate::witness::WitnessCategory;
    match name {
        "IsDeclaredAs" => Some(WitnessCategory::Declared),
        "IsObservedAs" => Some(WitnessCategory::Observed),
        "IsDerivedAs" => Some(WitnessCategory::Derived),
        "IsVerifiedAs" => Some(WitnessCategory::Verified),
        _ => None,
    }
}
