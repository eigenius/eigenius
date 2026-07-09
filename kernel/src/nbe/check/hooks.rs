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

//! Check-time resolver hooks — the seam between the bidirectional type
//! checker and the chain-resident data it needs: resolving an
//! `EigonClass` IRI to its Sigma type, and synthesising a D49
//! `ChainWitness` inhabitant from the per-layer witness index.
//!
//! The checker core knows only this trait; the default implementation
//! (`crate::program::check_hooks`) supplies the `program::ground` /
//! `layer` / `witness` machinery. §3.3 of
//! `docs/notes/nbe-reorganization-analysis.md`.

use super::CheckError;
use crate::layer::Layer;
use crate::nbe::val::Val;
use crate::ontology::iri::Iri;
use std::sync::Arc;

/// Chain-resident resolution the type checker delegates out of the
/// pure core. Stateless: every method takes the layer it resolves
/// against, so a single shared instance serves all `CheckCtx`s.
pub trait CheckHooks: Send + Sync {
    /// Resolve an `EigonClass` IRI to its EigenTT Sigma type against
    /// the layer chain (D18 ontology-as-types).
    fn resolve_class(&self, iri: &Iri, layer: &Arc<Layer>) -> Result<Val, CheckError>;

    /// Synthesise a D49 `ChainWitness` inhabitant for a
    /// `JustifiedBy.*` predicate whose expected type is `expected_typ`
    /// (a `Val::InductiveType` over a witness-category inductive).
    /// Returns `Ok(None)` when the type is not a chain-witness
    /// predicate; `Err` when it *is* one but synthesis fails (no
    /// layer-attached context, malformed indices, missing trace).
    /// `level` is the current de Bruijn level (for reading back the
    /// predicate's `Prop` index).
    fn synthesize_chain_witness(
        &self,
        expected_typ: &Val,
        level: usize,
        layer: Option<&Arc<Layer>>,
    ) -> Result<Option<Val>, CheckError>;
}
