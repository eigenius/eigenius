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

//! Effect hooks — the seam between the pure NbE evaluator and the
//! Eigenius runtime (institution dispatch, IO component invocation).
//!
//! The evaluator core ([`super::eval_impl`]) is pure type theory; the
//! three effectful expression forms it can encounter —
//! `App` on a registered component, `InstitutionInvoke` (D14 comorphism
//! dispatch), and `NativeDecide` on an institution-bound constraint —
//! are delegated to an [`EffectHooks`] implementation carried by
//! [`EvalCtx::Effectful`](super::EvalCtx). The concrete engine lives
//! outside `nbe` (`crate::institution::eval_hooks`); the core knows
//! only this trait. §3.3 of `docs/notes/nbe-reorganization-analysis.md`.

use super::{EvalCtx, EvalError};
use crate::nbe::env::Rho;
use crate::nbe::term::Constraint;
use crate::nbe::val::Val;
use crate::ontology::iri::Iri;
use crate::program::trace::ComponentTrace;

/// Three-valued outcome of deciding an institution-bound constraint —
/// the NbE-owned mirror of `institution::DecResult`, so the evaluator
/// core doesn't depend on the institution crate for this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Holds,
    Fails,
    Undecidable,
}

/// Runtime effects the evaluator delegates out of the pure core.
///
/// One implementation per capability tier: the full IO engine
/// (component dispatch + comorphism pipeline + trace/produced-resource
/// collection) and the check-time engine (institution deciding only,
/// no component registry). Both live in `crate::institution`.
pub trait EffectHooks: Send + Sync {
    /// Whether `name` resolves to a registered IO component. The
    /// evaluator's `App` arm consults this *before* evaluating the
    /// argument so it can preserve evaluation order: a component call
    /// evaluates its argument first (then dispatches), an ordinary
    /// application evaluates the function first. Cheap registry lookup.
    fn is_component(&self, name: &str) -> bool;

    /// Dispatch a registered IO component (`is_component(name)` already
    /// held). `arg_val` is the evaluated argument — a `Pair(input, arg)`
    /// splits into the component's input and side argument, anything
    /// else is the input with no side argument. Returns the result plus
    /// the produced `ComponentTrace` (if any) for the D6b tree; the
    /// implementation also records the trace for the run-boundary drain.
    fn dispatch_component(
        &self,
        name: &str,
        arg_val: &Val,
    ) -> Result<(Val, Option<ComponentTrace>), EvalError>;

    /// D14 §9.3 comorphism dispatch. `Ok(None)` when no institution
    /// backing / registry is attached (the evaluator yields a
    /// passthrough neutral); `Ok(Some(v))` on a completed translation.
    fn institution_invoke(
        &self,
        comorphism_iri: &Iri,
        source: &Val,
        target_iri: Option<&Iri>,
    ) -> Result<Option<Val>, EvalError>;

    /// Decide an institution-bound constraint (D14 §9.2). Structural
    /// constraints (`MinValue`, `Pattern`, …) are handled in the pure
    /// core and never reach here — only `Constraint::Institution`
    /// does. `ctx` is the current effectful context, threaded so the
    /// implementation can re-enter the evaluator to reduce the
    /// constraint's argument expressions.
    fn decide_institution(
        &self,
        constraint: &Constraint,
        value: &Val,
        rho: &Rho,
        ctx: &EvalCtx,
    ) -> Result<Decision, EvalError>;
}
