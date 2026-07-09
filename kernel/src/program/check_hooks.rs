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

//! Default [`CheckHooks`] implementation — the chain-resident resolution
//! the type checker delegates out of its pure core: `EigonClass` →
//! Sigma type (via `program::ground`) and D49 `ChainWitness` synthesis
//! (via the per-layer witness index in `layer` / `witness`). §3.3 of
//! `docs/notes/nbe-reorganization-analysis.md`.

use crate::layer::Layer;
use crate::nbe::check::{CheckError, CheckHooks};
use crate::nbe::readback::readback_val;
use crate::nbe::val::Val;
use crate::ontology::iri::Iri;
use crate::witness::WitnessCategory;
use std::sync::Arc;

/// Stateless resolver wiring `program::ground` + the witness index into
/// the type checker. A single shared instance serves every `CheckCtx`.
pub struct DefaultCheckHooks;

impl CheckHooks for DefaultCheckHooks {
    fn resolve_class(&self, iri: &Iri, layer: &Arc<Layer>) -> Result<Val, CheckError> {
        crate::program::ground::resolve_class_type(iri, layer).map_err(CheckError::from)
    }

    fn synthesize_chain_witness(
        &self,
        expected_typ: &Val,
        level: usize,
        layer: Option<&Arc<Layer>>,
    ) -> Result<Option<Val>, CheckError> {
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
            return Err(CheckError::IllFormed(format!(
                "ChainWitness predicate `{}` expected 2 indices (iri, P), got {}",
                decl.name,
                indices.len()
            )));
        }

        let iri_str = match &indices[0] {
            Val::LitString(s) => s.clone(),
            other => {
                return Err(CheckError::IllFormed(format!(
                    "ChainWitness predicate `{}` iri index must be LitString, got {other:?}",
                    decl.name
                )));
            }
        };
        let iri = Iri::parse(&iri_str)
            .map_err(|e| format!("ChainWitness `{}`: invalid iri `{iri_str}`: {e}", decl.name))?;

        let prop_exp = readback_val(level, &indices[1]);

        let layer = layer.ok_or_else(|| {
            format!(
                "ChainWitness synthesis for `{}` requires a layer-attached CheckCtx; \
                 pure-mode contexts cannot admit chain witnesses",
                decl.name
            )
        })?;

        let witness_val = crate::layer::synthesize_chain_witness(layer, category, &iri, &prop_exp)?;
        Ok(Some(witness_val))
    }
}

/// Map an inductive's short name to its `WitnessCategory` if it is a
/// D49 chain-witness predicate; `None` otherwise.
fn chain_witness_category_for_short_name(name: &str) -> Option<WitnessCategory> {
    match name {
        "IsDeclaredAs" => Some(WitnessCategory::Declared),
        "IsObservedAs" => Some(WitnessCategory::Observed),
        "IsDerivedAs" => Some(WitnessCategory::Derived),
        "IsVerifiedAs" => Some(WitnessCategory::Verified),
        _ => None,
    }
}
