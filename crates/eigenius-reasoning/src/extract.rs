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

//! `extract_typed` machinery for the Reasoning institution.
//!
//! `extract_typed` is the kernel's standard "lift a chain
//! resource into a typed `Val`" abstraction; every institution that
//! exposes its data to the kernel's term language goes through it.
//! The Reasoning institution's job in this file is to translate a
//! `justification:Term` chain-resident value (D32 §3.7 tagged-dict
//! shape on the `justification:term` property) into a kernel
//! `Val::InductiveVal` typed at `justification:Term`.
//!
//! Why this lives in the institution crate, not in the kernel:
//!
//! - D32 §3.7 specifies the *wire format* for inductive values, but
//!   not how to lift them into kernel `Val`. Numerical institutions
//!   (Symbolics, Catalyst, …) reify D32-shape values into their own
//!   runtime's representation (Julia structs) at the institution
//!   boundary; they never go through kernel `Val`. The Reasoning
//!   institution is different because its "runtime" *is* the kernel's
//!   NbE checker — there's no external worker to reify into, and the
//!   validate handler needs a `Val` to construct
//!   `justification:Certificate(justification, proposition)` for type-checking.
//! - Routing the lift through `extract_typed` (rather than a free
//!   function in the kernel) keeps the kernel surface scoped to
//!   abstractions it has specs for. The "chain inductive value → Val"
//!   bridge is Reasoning-institution-specific machinery; it belongs
//!   here.
//!
//! The lift goes through `Exp` as an intermediate: chain JSON →
//! `Exp::InductiveCtor` (a syntactic ctor application) → `Val` via
//! [`eigenius_kernel::nbe::eval::eval`]. The Exp step lets the kernel's
//! existing inductive machinery (positivity, recursor, etc.) see the
//! value uniformly with everything else it manipulates.

use eigenius_kernel::context::ExecutionContext;
use eigenius_kernel::institution::error::InstitutionError;
use eigenius_kernel::nbe::env::Rho;
use eigenius_kernel::nbe::eval::eval_env;
use eigenius_kernel::nbe::term::Exp;
use eigenius_kernel::nbe::val::Val;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::Resource;

use crate::institution::iris;

/// `extract_typed` handler for `proc:extract_justification`.
///
/// Projects the justification term out of the conclusion's judgement and
/// evaluates it to a `Val::InductiveVal` typed at `justification:Term`.
pub fn extract_justification(
    sentence: &Resource,
    ctx: &ExecutionContext,
) -> Result<Val, InstitutionError> {
    let exp = justification_exp(sentence, ctx)?;
    eval_env(
        &exp,
        &Rho::Nil,
        &eigenius_kernel::nbe::env_global::Env::of(ctx.head().clone()),
    )
    .map_err(|e| {
        InstitutionError::ComputationFailed(format!("failed to evaluate justification: {e:?}"))
    })
}

/// The same decode, stopped one step earlier: the SYNTACTIC `Exp::InductiveCtor` tree.
///
/// [`extract_justification`] evaluates this into a `Val` because the validate handler needs a value
/// to build `justification:Certificate(j, p)` from. The projections of D73 §1.2 want the opposite — they walk the
/// constructor application itself, since a `justification:Term` IS its tree and evaluating it only
/// obscures the shape (eigenius#204).
pub fn justification_exp(
    sentence: &Resource,
    ctx: &ExecutionContext,
) -> Result<Exp, InstitutionError> {
    // The justification term is no longer a slot: it is the FIRST index of the
    // certificate type inside the conclusion's judgement,
    // `holds(kernel, c, Certificate(j, P))`. Reading it from there is what
    // makes the term and the certificate provably about the same claim — as
    // separate slots nothing related them.
    //
    // Changed here rather than after the relocation this file is scheduled
    // for: moving a file and rewriting what it reads in one step makes the
    // move unreviewable.
    let stored = sentence
        .get(&Iri::parse(iris::PROP_JUDGEMENT).expect("static IRI"))
        .ok_or_else(|| {
            InstitutionError::ComputationFailed(
                "justification:Conclusion missing required `judgement` property".to_string(),
            )
        })?;
    let judgement =
        eigenius_kernel::program::eigentt_type_mirror::decode_judgement(stored, ctx.head())
            .map_err(|e| {
                InstitutionError::ComputationFailed(format!(
                    "conclusion judgement does not decode: {e}"
                ))
            })?;
    let (j_exp, _p) =
        eigenius_kernel::program::eigentt_type_mirror::certificate_indices(&judgement.typ)
            .ok_or_else(|| {
                InstitutionError::ComputationFailed(
                    "a conclusion's judgement must be checked against \
                     justification:Certificate(j, P)"
                        .to_string(),
                )
            })?;
    // The projection already IS an `Exp` — which is exactly what this function
    // returns — so no chain-value lift is needed.
    //
    // Re-encoding it would be actively wrong rather than merely wasteful:
    // `encode_type` emits the D47 form (a `CtorApp` naming a foreign
    // inductive), while a `justification:Term` value on the chain is a plain
    // D32 §3.7 tagged dict, which is what `chain_value_to_exp` reads. Sending
    // the term round the wrong codec reported `ctor CtorApp not declared on
    // inductive Term` — the two encodings meeting.
    Ok(j_exp.clone())
}

// The D32 §3.7 tagged-dict → `Exp` lift that used to live here is GONE, and the
// collapse of the conclusion's three slots into one judgement is why.
//
// A `justification:Term` used to sit in a slot of its own as a plain tagged
// dict, which no codec read — hence a bespoke ~330-line decoder here, plus its
// own error type and diagnostics. The term now rides inside the judgement,
// which is an `eigentt:Term`-ranged value, so the D47 codec decodes it and
// `justification_exp` projects an `Exp` straight out. One encoding where there
// were two, and the decoder that bridged them has no callers.
//
// This removes the stated reason this file had to be relocated rather than
// deleted: nothing else performed that lift, and now nothing performs it.
