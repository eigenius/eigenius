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

use super::{CheckCtx, CheckError};
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
) -> Result<Option<Val>, CheckError> {
    ctx.hooks
        .synthesize_chain_witness(expected_typ, ctx.rho.len(), ctx.layer.as_ref())
}
