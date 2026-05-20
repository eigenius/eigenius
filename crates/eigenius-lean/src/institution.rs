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

//! `LeanInstitution` — Lean 4 verification institution per
//! [D28](../../docs/design/d28-lean-4-as-institution.md).
//!
//! The kernel binary instantiates one of these at startup
//! ([`super::startup::register`]) and the chain-scan registration pass
//! (`kernel::capability::registration::register_in_process_institutions`)
//! wires it into [`kernel::institution::runtime::InstitutionRuntime`]
//! whenever it encounters the `lean:lean_institution` Institution
//! resource declared by [`ontologies/lean/lean-institution.eigon.json`].
//!
//! ## Surface in 20a.4
//!
//! - `query(proof_check, LeanProofTerm)` — extracts the
//!   referenced `LeanProofPayload`'s `payload_bytes`, reads the
//!   `target_name`, runs the v1 `check_proof` (axiom allowlist empty —
//!   `LeanEnvironment` integration arrives in 20a.5), and returns a
//!   `Verdict::Holds | Fails { diagnostic }` resource.
//! - `query(which_axioms, …)` — `NotImplemented` (the QueryClass is
//!   declared on chain so the procedure IRI is bound, but the v1
//!   institution doesn't compute the axiom list yet).
//! - `extract_typed(ef_lean_proof_payload, LeanProofTerm)` — returns
//!   the payload bytes wrapped as `Val::ResourceVal({core:string →
//!   bytes})`, matching the convention `kernel::nbe::eval::
//!   resource_value_to_val` uses for string-typed values.
//! - `reify` — `NotImplemented`. Lean has no `ImportFormat`s yet;
//!   construction is authoring-side via the chain-mirror translator,
//!   not via a kernel `reify` call.
//!
//! ## Correspondence is stubbed
//!
//! D28 §5.5's three-part correspondence check has only the
//! "proof-validity" part wired (nanoda's verdict). The other two
//! (mirror-anchor existence + structural correspondence between the
//! mirror type and the claim) are deferred to 20a.7 — the
//! `mirror_iri` / `claim_iri` fields are recorded but unread on the
//! verification path. The QueryClass's AutoOnLoad outcome is therefore
//! "the proof type-checks under nanoda," not "the proof discharges
//! the Eigon claim."

use std::sync::Arc;

use eigenius_kernel::context::ExecutionContext;
use eigenius_kernel::institution::error::InstitutionError;
use eigenius_kernel::institution::runtime::{Institution, QueryOutcome};
use eigenius_kernel::nbe::val::Val;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;

use crate::checker::{check_proof, Verdict};

/// Well-known IRIs the institution dispatches on. Keeping them in one
/// place so a downstream caller building a `LeanProofTerm` resource
/// can reach for the same strings without spelling them out.
pub mod iris {
    /// The institution itself.
    pub const INSTITUTION: &str = "urn:eigenius:lean:lean_institution";

    /// AutoOnLoad / OnDemand procedure: verify a `LeanProofTerm` and
    /// return a `Verdict`.
    pub const PROC_PROOF_CHECK: &str = "urn:eigenius:lean:proc:proof_check";

    /// OnDemand procedure: list axioms a proof transitively depends on.
    /// `NotImplemented` in v1 — see module docstring.
    pub const PROC_WHICH_AXIOMS: &str = "urn:eigenius:lean:proc:which_axioms";

    /// ExportFormat procedure: extract a `LeanProofTerm`'s referenced
    /// payload bytes as a `core:string`-typed Mini-TT value.
    pub const PROC_EXTRACT_PROOF_PAYLOAD: &str = "urn:eigenius:lean:proc:extract_proof_payload";

    /// Property: `LeanProofTerm.proof_payload` (resource ref →
    /// `LeanProofPayload`).
    pub const PROP_PROOF_PAYLOAD: &str = "urn:eigenius:lean:proof_payload";

    /// Property: `LeanProofPayload.payload_bytes` (string).
    pub const PROP_PAYLOAD_BYTES: &str = "urn:eigenius:lean:payload_bytes";

    /// Property: `LeanProofTerm.target_name` (string).
    pub const PROP_TARGET_NAME: &str = "urn:eigenius:lean:target_name";

    /// Property attached to a `Verdict::Fails` carrying the
    /// human-readable refusal reason (D31 §6.3 / institution ontology).
    pub const PROP_DIAGNOSTIC: &str = "urn:eigenius:institution:diagnostic";
}

/// In-process Lean 4 verification institution.
///
/// Stateless — every `query` call parses the proof from scratch via
/// `nanoda_lib`. A future revision may cache the parsed `ExportFile`
/// keyed by content hash to amortise repeated AutoOnLoad firings of
/// the same `LeanProofPayload`; the blanket
/// `impl Institution for Arc<I>` in
/// `kernel::institution::runtime` already permits per-process state
/// without rebuilding on every registry rebuild.
pub struct LeanInstitution {
    iri: Iri,
}

impl LeanInstitution {
    /// Construct a new institution with the canonical
    /// `urn:eigenius:lean:lean_institution` IRI.
    pub fn new() -> Self {
        Self {
            iri: Iri::parse(iris::INSTITUTION).expect("static institution IRI"),
        }
    }

    /// Wrap a fresh institution in an `Arc<dyn Institution>` ready to
    /// hand to
    /// `EigeniusService::register_in_process_institution`. Convenience
    /// constructor for the startup hook.
    pub fn arc() -> Arc<dyn Institution> {
        Arc::new(Self::new())
    }
}

impl Default for LeanInstitution {
    fn default() -> Self {
        Self::new()
    }
}

impl Institution for LeanInstitution {
    fn institution_iri(&self) -> &Iri {
        &self.iri
    }

    fn extract_typed(
        &self,
        procedure_iri: &Iri,
        resource: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<Val, InstitutionError> {
        if procedure_iri.as_str() == iris::PROC_EXTRACT_PROOF_PAYLOAD {
            let payload = resolve_payload(resource, ctx)?;
            let bytes = payload_bytes(&payload)?;
            // Match `kernel::nbe::eval::resource_value_to_val`'s
            // string convention: a `Val::ResourceVal` wrapping an
            // embedded Resource that carries the string under the
            // `core:string` property. The ExportFormat's
            // `payload_type` is `core:string`, so the consumer reads
            // the property by that IRI.
            let mut wrapper = Resource::new_embedded();
            wrapper.set(
                Iri::parse(wk::STRING).expect("well-known IRI"),
                Value::String(bytes),
            );
            Ok(Val::ResourceVal(Box::new(wrapper)))
        } else {
            Err(InstitutionError::NotImplemented(format!(
                "LeanInstitution has no extract_typed handler for `{procedure_iri}`"
            )))
        }
    }

    fn reify(
        &self,
        procedure_iri: &Iri,
        _value: &Val,
        _ctx: &ExecutionContext,
    ) -> Result<Resource, InstitutionError> {
        Err(InstitutionError::NotImplemented(format!(
            "LeanInstitution has no reify handler for `{procedure_iri}` \
             (Lean institution declares no ImportFormats in 20a.4 — construction \
              is authoring-side via the chain-mirror translator)"
        )))
    }

    fn query(
        &self,
        procedure_iri: &Iri,
        input: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<QueryOutcome, InstitutionError> {
        match procedure_iri.as_str() {
            iris::PROC_PROOF_CHECK => do_proof_check(input, ctx),
            iris::PROC_WHICH_AXIOMS => Err(InstitutionError::NotImplemented(
                "LeanInstitution::query(which_axioms) is not implemented in 20a.4 — \
                 the QueryClass is declared on chain so the procedure IRI is bound, \
                 but axiom-list extraction lands opportunistically"
                    .to_string(),
            )),
            _ => Err(InstitutionError::NotImplemented(format!(
                "LeanInstitution has no query handler for procedure `{procedure_iri}`"
            ))),
        }
    }
}

/// Run the core proof-check procedure: read the LeanProofTerm's
/// payload bytes + target name, call `check_proof`, and lift the
/// resulting `Verdict` into a chain-shaped `Verdict::Holds | Fails`
/// resource.
///
/// Empty axiom allowlist for v1 — `LeanEnvironment.permitted_axioms`
/// integration arrives with the authoring runtime (20a.5).
fn do_proof_check(
    input: &Resource,
    ctx: &ExecutionContext,
) -> Result<QueryOutcome, InstitutionError> {
    let payload = resolve_payload(input, ctx)?;
    let bytes = payload_bytes(&payload)?;
    let target_name = input
        .get(&Iri::parse(iris::PROP_TARGET_NAME).expect("static IRI"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            InstitutionError::ComputationFailed(
                "LeanProofTerm missing required `target_name` property".to_string(),
            )
        })?
        .to_string();

    let verdict = check_proof(bytes.as_bytes(), &target_name, &[])
        .map_err(|e| InstitutionError::ComputationFailed(format!("nanoda check_proof: {e}")))?;

    let output = match verdict {
        Verdict::Holds => verdict_resource(wk::VERDICT_HOLDS, None),
        Verdict::Fails { diagnostic } => verdict_resource(wk::VERDICT_FAILS, Some(&diagnostic)),
    };
    Ok(QueryOutcome::from_output(output))
}

/// Resolve a LeanProofTerm's `proof_payload` reference into the
/// concrete `LeanProofPayload` resource. Accepts both
/// `Value::Embedded` (inline payload) and `Value::ResourceRef`
/// (referenced separately) shapes — the kernel canonicaliser may have
/// left either depending on whether the caller embedded the payload
/// or registered it as a top-level resource.
fn resolve_payload(term: &Resource, ctx: &ExecutionContext) -> Result<Resource, InstitutionError> {
    let prop_iri = Iri::parse(iris::PROP_PROOF_PAYLOAD).expect("static IRI");
    let value = term.get(&prop_iri).ok_or_else(|| {
        InstitutionError::ComputationFailed(
            "LeanProofTerm missing required `proof_payload` property".to_string(),
        )
    })?;
    match value {
        Value::Embedded(boxed) => Ok((**boxed).clone()),
        Value::ResourceRef(payload_iri) => ctx.resolve(payload_iri).map(|arc| (*arc).clone()).ok_or_else(|| {
            InstitutionError::MissingDependency(format!(
                "LeanProofPayload `{payload_iri}` referenced by `proof_payload` does not resolve in the layer chain"
            ))
        }),
        other => Err(InstitutionError::ComputationFailed(format!(
            "`proof_payload` has unexpected value shape: {other:?}"
        ))),
    }
}

/// Extract the `payload_bytes` string from a `LeanProofPayload`
/// resource.
fn payload_bytes(payload: &Resource) -> Result<String, InstitutionError> {
    payload
        .get(&Iri::parse(iris::PROP_PAYLOAD_BYTES).expect("static IRI"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            InstitutionError::ComputationFailed(
                "LeanProofPayload missing required `payload_bytes` property".to_string(),
            )
        })
}

/// Build the embedded Verdict resource the kernel's commit pipeline
/// expects: `is_a: [Verdict]`, `ctor_name: "Holds"|"Fails"`, and an
/// optional `diagnostic` string. Matches the shape
/// `kernel::institution::in_process_registry::EchoInstitution::query`
/// constructs.
fn verdict_resource(ctor_name: &str, diagnostic: Option<&str>) -> Resource {
    let mut r = Resource::new_embedded();
    r.set(
        Iri::parse(wk::IS_A).expect("well-known IRI"),
        Value::Array(vec![Value::ResourceRef(
            Iri::parse(wk::VERDICT).expect("well-known IRI"),
        )]),
    );
    r.set(
        Iri::parse(wk::CTOR_NAME).expect("well-known IRI"),
        Value::String(ctor_name.to_string()),
    );
    if let Some(d) = diagnostic {
        r.set(
            Iri::parse(iris::PROP_DIAGNOSTIC).expect("static IRI"),
            Value::String(d.to_string()),
        );
    }
    r
}
