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
//!   `target_name`, runs `check_proof` against the hard-coded
//!   [`DEFAULT_LEAN_AXIOMS`] allowlist, and returns a
//!   `Verdict::Holds | Fails { diagnostic }` resource. Nothing on this
//!   path reads a `LeanEnvironment`: `lean:lean_permitted_axioms` is
//!   read only by the authoring runtime, so a per-environment override
//!   has no effect on a verdict.
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
//! ## Correspondence check (D28 §5.5)
//!
//! Three checks run in order:
//!
//! 1. **Proof validity** — nanoda's `check_proof`. Same as 20a.4.
//! 2. **Mirror correspondence** — resolve `mirror_iri` to a
//!    `runtime:RuntimePackageMirror` (the design documents call it a
//!    `LeanPackageMirror`; no such class or type exists), verify its
//!    `source_layer` is reachable
//!    from `head` (proof anchored to an ancestor-or-equal of the
//!    layer the check runs against), and confirm the mirror covers
//!    the claim's class via `mirrored_classes`. Lacking either
//!    raises `FFIVersionMismatch`.
//! 3. **Anchor consistency** — recompute the
//!    `library_content_hash` over the embedded archive and confirm
//!    it matches the declared hash. Mismatch surfaces as
//!    `AnchorContentHashMismatch`.
//!
//! A `LeanProofTerm` without `mirror_iri` skips checks 2 + 3 — the
//! verdict reflects nanoda alone, matching the 20a.4 behavior for
//! proofs not yet pinned to a chain-level claim.
//!
//! ### Structural correspondence (D28 §5.5 ¶2 final sentence)
//!
//! When the `LeanProofTerm` carries a `proposition` — a
//! chain-mirrored `lean:LeanExpr` (D40) value — the check walks
//! that tree, collects every `Const` reference under the
//! `EigeniusFFI` namespace, and verifies at least one maps back
//! (via the mirror's `mirrored_classes` + each class's
//! `core:short_name`) to the claim's class IRI. Failure surfaces
//! as `PropositionMismatch` (D28 §9.1) with a diagnostic listing
//! what the proposition *does* reference.
//!
//! The proposition is recommended-not-required (D28 §6.3). Absent
//! → structural check is skipped; the covering check (class IRI ∈
//! `mirrored_classes`) is the only correspondence gate. Once the
//! orchestrator's commit pipeline guarantees `proposition`
//! population for every committed proof, a future spec version may
//! upgrade absent-proposition to a hard rejection.

use std::sync::Arc;

use eigenius_kernel::context::ExecutionContext;
use eigenius_kernel::institution::error::InstitutionError;
use eigenius_kernel::institution::runtime::{Institution, QueryOutcome};
use eigenius_kernel::nbe::term::Exp;
use eigenius_kernel::nbe::val::Val;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::program::eigentt_type_mirror::decode_type;

use crate::checker::{check_proof, ExpectedStatement, Verdict};

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
    /// payload bytes as a `core:string`-typed EigenTT value.
    pub const PROC_EXTRACT_PROOF_PAYLOAD: &str = "urn:eigenius:lean:proc:extract_proof_payload";

    /// Property: `LeanProofTerm.proof_payload` (resource ref →
    /// `LeanProofPayload`).
    pub const PROP_PROOF_PAYLOAD: &str = "urn:eigenius:lean:proof_payload";

    /// Property: `LeanProofPayload.payload_bytes` (string).
    pub const PROP_PAYLOAD_BYTES: &str = "urn:eigenius:lean:payload_bytes";

    /// Property: `LeanProofTerm.target_name` (string).
    pub const PROP_TARGET_NAME: &str = "urn:eigenius:lean:target_name";

    /// Property: `LeanProofTerm.claim_iri` — IRI of the Eigon
    /// claim resource the proof discharges. v1 reads it for
    /// mirror-coverage matching: the claim's class must appear in
    /// the mirror's `mirrored_classes`.
    pub const PROP_CLAIM_IRI: &str = "urn:eigenius:lean:claim_iri";

    /// Property attached to a `Verdict::Fails` carrying the
    /// human-readable refusal reason (D31 §6.3 / institution ontology).
    pub const PROP_DIAGNOSTIC: &str = "urn:eigenius:institution:diagnostic";

    // ── RuntimePackageMirror properties (D26 §5.4) — read by the
    // correspondence check. Constants mirror the substrate-side
    // properties that `mirror_to_resource` in `eigenius-lean-runtime`
    // stamps onto each generated mirror.
    pub const PROP_MIRROR_SOURCE_LAYER: &str = "urn:eigenius:runtime:source_layer";
    pub const PROP_MIRROR_LIB_CONTENT_HASH: &str = "urn:eigenius:runtime:library_content_hash";
    pub const PROP_MIRROR_LIB_CONTENT: &str = "urn:eigenius:runtime:library_content";
    pub const PROP_MIRRORED_CLASSES: &str = "urn:eigenius:runtime:mirrored_classes";

    // ── Diagnostic kinds (D28 §9.1). Prefixed onto the diagnostic
    // string so consumers can match by leading token. Single-string
    // shape matches the existing `PROP_DIAGNOSTIC` flat surface.
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
/// Default axiom allowlist when the `LeanProofTerm` doesn't anchor
/// to a `LeanEnvironment` that pins one. Matches D28 §7.1 — Lean's
/// four trust-the-compiler axioms. Even a trivial proof through
/// modern Lean stdlib pulls `Classical.choice` (via `Subtype`'s
/// projection helpers), so empty-allowlist is a footgun for any
/// real proof; the canonical default catches that case.
const DEFAULT_LEAN_AXIOMS: &[&str] = &[
    "propext",
    "Classical.choice",
    "Quot.sound",
    "Lean.trustCompiler",
];

/// The claim's `reflection:canonical_proposition`, decoded to an `Exp` (D74 §2).
///
/// `None` when the proof term names no claim, or the claim carries no proposition — the
/// statement-level check has nothing to compare against and the caller falls back to the
/// name-level check alone. An unresolvable `claim_iri`, or a proposition that will not decode,
/// is an error rather than a skip: the author meant to bind a claim and the binding is broken.
fn claim_proposition(
    input: &Resource,
    ctx: &ExecutionContext,
) -> Result<Option<Exp>, InstitutionError> {
    let Some(claim_iri) = input
        .get(&Iri::parse(iris::PROP_CLAIM_IRI).expect("static IRI"))
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };
    let claim_iri = Iri::parse(claim_iri).map_err(|e| {
        InstitutionError::ComputationFailed(format!("`claim_iri` is not an IRI: {e}"))
    })?;
    let claim = ctx.resolve(&claim_iri).ok_or_else(|| {
        InstitutionError::ComputationFailed(format!(
            "`claim_iri` `{claim_iri}` does not resolve in the verification context"
        ))
    })?;
    // REJECT, do not skip. The whole of #159 is that a `Holds` must mean "this proof proves THIS
    // claim"; a claim carrying no proposition leaves nothing to compare against, and returning
    // `None` here would fall back to the name-level check — "a theorem with this name
    // type-checks" — which is the verdict the issue opened against.
    //
    // This cannot be expressed in the ontology: `reflection:canonical_proposition` is a
    // `reflection:` property on an arbitrary claim class, and `lean:LeanProofTerm` cannot require
    // a property of a resource it merely references. So the institution enforces it, and D74
    // §6.3's `claim_iri: requires` is necessary but not sufficient on its own.
    let Some(value) = claim.get(&Iri::parse(wk::CANONICAL_PROPOSITION).expect("well-known IRI"))
    else {
        return Err(InstitutionError::ComputationFailed(format!(
            "claim `{claim_iri}` carries no `reflection:canonical_proposition`, so there is \
             nothing to check the proof against; a Lean verdict must not rest on the target \
             name alone (D74 / eigenius#159)"
        )));
    };
    let exp = decode_type(value, ctx.head()).map_err(|e| {
        InstitutionError::ComputationFailed(format!(
            "`{claim_iri}`'s canonical_proposition does not decode: {e:?}"
        ))
    })?;
    Ok(Some(exp))
}

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

    // v1 uses the canonical default allowlist (D28 §7.1). When the
    // `LeanProofTerm` carries an `environment_iri` (D28 §6.3) the
    // institution will read that env's `lean_permitted_axioms`
    // property and use it instead — that wiring lands when the
    // authoring runtime's env-resource flow into the kernel
    // commit pipeline (currently the env IRI isn't on the chain).
    let permitted_axioms: Vec<String> = DEFAULT_LEAN_AXIOMS
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    // D74 / #159 — the statement-level check beside the name-level one. The claim's own
    // proposition becomes the Lean goal, so `Holds` means "this proof proves THIS claim" rather
    // than "a theorem with this name type-checks".
    //
    // `claim_iri` is `recommends` today, so an absent one still reaches the name-level check
    // alone. D74 §6.3 promotes it to `requires`; until that ontology edit lands this is the one
    // place the check can be skipped, and it is skipped loudly in the diagnostic rather than
    // silently.
    let expected = claim_proposition(input, ctx)?;
    let verdict = check_proof(
        bytes.as_bytes(),
        &target_name,
        &permitted_axioms,
        expected
            .as_ref()
            .map(|prop| ExpectedStatement {
                proposition: prop,
                layer: ctx.head(),
            })
            .as_ref(),
    )
    .map_err(|e| InstitutionError::ComputationFailed(format!("nanoda check_proof: {e}")))?;

    // Check 1 (proof validity) decided — short-circuit on nanoda
    // rejection. No correspondence check runs against a proof that
    // doesn't type-check; the diagnostic would be misleading.
    if let Verdict::Fails { diagnostic } = verdict {
        return Ok(QueryOutcome::from_output(verdict_resource(
            wk::VERDICT_FAILS,
            Some(&diagnostic),
        )));
    }

    // Checks 2 and 3 (D28 §5.5) are gone with `mirror_iri` and `lean:proposition`
    // (D74 §6.3.1). `def_eq` against the claim's own externalized proposition subsumes them:
    // 2c asked whether the committed proposition MENTIONED the claim's class, a proxy for what
    // the comparison answers directly, and 2a's version-skew check is implied — a moved mirror
    // makes the externalized `Const` names disagree with the export's.

    Ok(QueryOutcome::from_output(verdict_resource(
        wk::VERDICT_HOLDS,
        None,
    )))
}

/// Resolve a LeanProofTerm's `proof_payload` reference into the
/// concrete `LeanProofPayload` resource. Accepts both
/// `Value::Embedded` (inline payload) and an IRI reference
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
        Value::String(payload_iri) => {
            let parsed = Iri::parse(payload_iri).map_err(|e| {
                InstitutionError::ComputationFailed(format!(
                    "`proof_payload` holds `{payload_iri}`, which is not an IRI: {e}"
                ))
            })?;
            ctx.resolve(&parsed)
                .map(|arc| (*arc).clone())
                .ok_or_else(|| {
                    InstitutionError::MissingDependency(format!(
                        "LeanProofPayload `{payload_iri}` referenced by `proof_payload` does not \
                     resolve in the layer chain"
                    ))
                })
        }
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
        Value::Array(vec![Value::String(
            Iri::parse(wk::VERDICT)
                .expect("well-known IRI")
                .as_str()
                .to_string(),
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
