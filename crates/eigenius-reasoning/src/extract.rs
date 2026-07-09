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
//! `JustificationTerm` chain-resident value (D32 §3.7 tagged-dict
//! shape on the `reasoning:justification` property) into a kernel
//! `Val::InductiveVal` typed at `reasoning:JustificationTerm`.
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
//!   `JustifiedBy(justification, proposition)` for type-checking.
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

use std::sync::Arc;

use eigenius_kernel::context::ExecutionContext;
use eigenius_kernel::institution::error::InstitutionError;
use eigenius_kernel::nbe::env::Rho;
use eigenius_kernel::nbe::eval::eval;
use eigenius_kernel::nbe::term::{Exp, InductiveDecl};
use eigenius_kernel::nbe::val::Val;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::program::ground::resolve_class_type;

use crate::institution::iris;

/// `extract_typed` handler for `proc:extract_justification`.
///
/// Reads the `justification` property off the supplied
/// `ReasoningSentence` resource, lifts the chain-resident inductive
/// value into a `Val::InductiveVal` typed at `JustificationTerm`.
pub fn extract_justification(
    sentence: &Resource,
    ctx: &ExecutionContext,
) -> Result<Val, InstitutionError> {
    let value = sentence
        .get(&Iri::parse(iris::PROP_JUSTIFICATION).expect("static IRI"))
        .ok_or_else(|| {
            InstitutionError::ComputationFailed(
                "ReasoningSentence missing required `justification` property".to_string(),
            )
        })?;

    let jt_iri = Iri::parse(iris::JUSTIFICATION_TERM).expect("static IRI");
    let jt_decl = match resolve_class_type(&jt_iri, ctx.head()) {
        Ok(Val::InductiveType { decl, .. }) => decl,
        Ok(other) => {
            return Err(InstitutionError::ComputationFailed(format!(
                "`{}` resolved to a non-inductive value: {other:?}",
                iris::JUSTIFICATION_TERM
            )));
        }
        Err(e) => {
            return Err(InstitutionError::ComputationFailed(format!(
                "failed to resolve JustificationTerm inductive: {e}"
            )));
        }
    };

    let exp = chain_value_to_exp(value, &jt_decl).map_err(|e| {
        InstitutionError::ComputationFailed(format!("malformed justification: {e}"))
    })?;
    eval(&exp, &Rho::Nil).map_err(|e| {
        InstitutionError::ComputationFailed(format!("failed to evaluate justification: {e:?}"))
    })
}

/// Decode a D32 §3.7-shaped chain inductive value into the kernel's
/// `Exp::InductiveCtor` form.
///
/// Args dispatch on the JSON primitive kind:
///
/// - **String** → [`Exp::LitString`].
/// - **Integer** → [`Exp::LitInt`].
/// - **Float** → [`Exp::LitFloat`].
/// - **Object with `ctor`/`args` fields** → recursive `InductiveCtor`
///   call. The recursion threads the same `decl` Arc; this assumes
///   recursive arg slots are of the same inductive type as the outer
///   ctor, which holds for `JustificationTerm` (every recursive arg
///   is itself a `JustificationTerm`). Heterogeneous inductives — a
///   ctor whose arg is a *different* inductive — would need a richer
///   decoder that consults per-arg declared types. Out of scope until
///   a Reasoning consumer needs it (see gh #74).
fn chain_value_to_exp(value: &Value, decl: &Arc<InductiveDecl>) -> Result<Exp, ChainDecodeError> {
    let json = match value {
        Value::Json(j) => j,
        other => {
            return Err(ChainDecodeError::NotJson(format!("{other:?}")));
        }
    };
    decode_json(json, decl, "<root>")
}

#[derive(Debug, Clone, PartialEq)]
enum ChainDecodeError {
    NotJson(String),
    NotObject(String),
    MissingCtor(String),
    MissingArgs(String),
    UnsupportedArg {
        path: String,
        details: String,
    },
    UnknownCtor {
        decl_name: String,
        ctor_name: String,
        available: Vec<String>,
    },
}

impl std::fmt::Display for ChainDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotJson(s) => write!(f, "expected Value::Json, got {s}"),
            Self::NotObject(p) => write!(f, "{p}: expected JSON object"),
            Self::MissingCtor(p) => write!(f, "{p}: missing string `ctor` field"),
            Self::MissingArgs(p) => write!(f, "{p}: missing array `args` field"),
            Self::UnsupportedArg { path, details } => write!(f, "{path}: {details}"),
            Self::UnknownCtor {
                decl_name,
                ctor_name,
                available,
            } => write!(
                f,
                "ctor `{ctor_name}` not declared on inductive `{decl_name}`; \
                 available ctors: {available:?}"
            ),
        }
    }
}

fn decode_json(
    json: &serde_json::Value,
    decl: &Arc<InductiveDecl>,
    path: &str,
) -> Result<Exp, ChainDecodeError> {
    let obj = json
        .as_object()
        .ok_or_else(|| ChainDecodeError::NotObject(path.to_string()))?;
    let ctor_name = obj
        .get("ctor")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ChainDecodeError::MissingCtor(path.to_string()))?;
    let args = obj
        .get("args")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| ChainDecodeError::MissingArgs(path.to_string()))?;

    // Last line of defense before the kernel type-checker — a clear
    // available-list diagnostic beats letting the type-checker crash
    // on a malformed `InductiveCtor`. Rule 16 should catch this at
    // commit, but the handler is dispatched after commit and may run
    // against a previously-malformed sentence.
    if !decl.ctors.iter().any(|c| c.name == ctor_name) {
        return Err(ChainDecodeError::UnknownCtor {
            decl_name: decl.name.clone(),
            ctor_name: ctor_name.to_string(),
            available: decl.ctors.iter().map(|c| c.name.clone()).collect(),
        });
    }

    let decoded_args: Result<Vec<Exp>, ChainDecodeError> = args
        .iter()
        .enumerate()
        .map(|(i, a)| decode_arg(a, decl, &format!("{path}.args[{i}]")))
        .collect();

    Ok(Exp::InductiveCtor(
        decl.clone(),
        ctor_name.to_string(),
        decoded_args?,
    ))
}

fn decode_arg(
    json: &serde_json::Value,
    decl: &Arc<InductiveDecl>,
    path: &str,
) -> Result<Exp, ChainDecodeError> {
    if let Some(s) = json.as_str() {
        return Ok(Exp::LitString(s.to_string()));
    }
    if let Some(n) = json.as_i64() {
        return Ok(Exp::LitInt(n));
    }
    if let Some(f) = json.as_f64() {
        // `as_f64` would also accept integers as `0.0`; check
        // `as_i64` first (above) so integers land on `LitInt`.
        return Ok(Exp::LitFloat(f));
    }
    if json.is_object() {
        return decode_json(json, decl, path);
    }
    Err(ChainDecodeError::UnsupportedArg {
        path: path.to_string(),
        details: format!(
            "arg slot must be a string, integer, float, or nested ctor object; got {json:?}"
        ),
    })
}
