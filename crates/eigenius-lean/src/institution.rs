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
//!   `target_name` and the claim's proposition, runs `check_proof` against the hard-coded
//!   [`DEFAULT_LEAN_AXIOMS`] allowlist, and returns a
//!   `Verdict::Holds | Fails { diagnostic }` resource plus, on `Holds`, a
//!   `prov:VerificationTrace`. Nothing on this
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
//! ## What a `Holds` means
//!
//! Two checks, both mandatory (D74 §6.3, eigenius#159):
//!
//! 1. **Proof validity** — nanoda type-checks every declaration in the export and refuses any
//!    axiom outside the permitted set.
//! 2. **Statement correspondence** — the claim named by `lean:claim_iri` carries a
//!    `reflection:canonical_proposition`; it is externalized to a Lean `Expr`
//!    ([`crate::externalize`]) and compared to the target declaration's type with nanoda's
//!    `def_eq`. Without this, `Holds` would mean only "a theorem with this name type-checks".
//!
//! D28 §5.5's three-part correspondence check is gone, along with `lean:mirror_iri` and
//! `lean:proposition` (D74 §6.3.1). `def_eq` against the claim's own proposition subsumes it: the
//! mirror-coverage check asked whether the committed proposition *mentioned* the claim's class, a
//! proxy for what the comparison answers directly, and the version-skew check is implied — a
//! moved mirror makes the externalized `Const` names disagree with the export's.
//!
//! ## What a `Holds` produces
//!
//! A `prov:VerificationTrace` naming the claim, emitted beside the Verdict and committed by the
//! kernel into the `verdict_provenance` layer (eigenius#160). That trace is what makes
//! `layer_admits_witness` answer `Verified` for the claim's proposition; the verdict resource
//! itself grounds nothing. On `Fails`, no trace.

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
use eigenius_kernel::server::helpers::{millis_to_iso8601, now_millis};

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

/// The claim this proof discharges: its IRI, and its `reflection:canonical_proposition`
/// decoded to an `Exp` (D74 §2).
///
/// `None` only when the proof term names no claim at all. `lean:claim_iri` is `requires` on
/// `LeanProofTerm` (D74 §6.3), so validation rejects that shape before dispatch and the arm is
/// unreachable through the commit pipeline; it stays for a direct `query` call. An unresolvable
/// `claim_iri`, a claim with no proposition, or a proposition that will not decode is an error
/// rather than a skip: the author meant to bind a claim and the binding is broken.
fn claim_proposition(
    input: &Resource,
    ctx: &ExecutionContext,
) -> Result<Option<(Iri, Exp)>, InstitutionError> {
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
    Ok(Some((claim_iri, exp)))
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
    //
    // This set is the trusted computing base of the verdict, and the trace records it
    // (`prov:permitted_axioms`, D87 §5). Until it did, two proofs — one leaning on
    // `Classical.choice` and one not — produced byte-identical traces.
    let permitted_axioms: Vec<String> = DEFAULT_LEAN_AXIOMS
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    // D74 / #159 — the statement-level check beside the name-level one. The claim's own
    // proposition becomes the Lean goal, so `Holds` means "this proof proves THIS claim" rather
    // than "a theorem with this name type-checks".
    let claim = claim_proposition(input, ctx)?;
    let verdict = check_proof(
        bytes.as_bytes(),
        &target_name,
        &permitted_axioms,
        claim
            .as_ref()
            .map(|(_, prop)| ExpectedStatement {
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

    // eigenius#160 — a `Holds` that promotes nothing is the verdict this institution existed to
    // avoid. The trace is what carries the check off this call and onto the chain: committed
    // beside the Verdict, it makes `layer_admits_witness` answer `Verified` for the claim's own
    // proposition (`witness_index::emit_from_trace`), which is the grade D28 §1 names as the
    // reason a proof-checking institution exists.
    let mut outcome = QueryOutcome::from_output(verdict_resource(wk::VERDICT_HOLDS, None));
    if let Some((claim_iri, proposition)) = claim.as_ref() {
        match verification_trace(
            input,
            &payload,
            claim_iri,
            proposition,
            &permitted_axioms,
            ctx.head(),
        ) {
            Ok(Some(trace)) => outcome.derivations.push(trace),
            Ok(None) => {}
            // The check held; the trace could not be built. Refusing the whole dispatch is right
            // and a silent `Holds` would not be: the trace is what carries the result onto the
            // chain, so a `Holds` without one is the verdict-that-promotes-nothing eigenius#160
            // existed to eliminate, arriving by a different route.
            Err(e) => return Err(e),
        }
    }
    Ok(outcome)
}

/// The checker's identity, as a *kind* and a value (D87 §9.3).
///
/// `image_digest` when the deployment injected `EIGENIUS_IMAGE_DIGEST` — the registry digest of
/// the running image, which is the only value binding the BINARY. `source_pin` otherwise: the
/// `nanoda_lib` revision this crate links plus the Lean toolchain whose export format it parses,
/// which binds the checker's source and not the build. Absence of the variable is itself
/// informative — "not deployed by digest" — so there is nothing to fall back past.
///
/// Self-reported either way: the kernel repeats what it was told. It is provenance, not warrant,
/// and becomes load-bearing only when something outside the process vouches for the binding.
fn checker_identity() -> (&'static str, String) {
    match std::env::var("EIGENIUS_IMAGE_DIGEST") {
        Ok(d) if !d.trim().is_empty() => ("image_digest", d),
        _ => (
            "source_pin",
            format!(
                "nanoda_lib@{} lean@{}",
                env!("EIGENIUS_NANODA_REV"),
                eigenius_lean_runtime::conventions::LEAN_TOOLCHAIN_VERSION
            ),
        ),
    }
}

/// `prov:proof_system` for a proof this institution checked. The property exists so a
/// `VerificationTrace` from an external prover and one from the kernel's own certificate checker
/// are the same class, told apart by value rather than by kind.
const PROOF_SYSTEM_LEAN4: &str = "lean4";

/// The `eigentt:Logic` individual naming this checker — "Lean 4, re-checked in process by the
/// `nanoda_lib` kernel reimplementation". The `logic` argument of every judgement this
/// institution emits.
const LOGIC_LEAN4: &str = "urn:eigenius:eigentt:logic_lean4";

/// The `prov:VerificationTrace` recording that this proof was checked against this claim.
///
/// Two halves, and the split is the paper's: the **judgement** is what the check ESTABLISHED, and
/// everything else is provenance about the occasion.
///
/// `prov:judgement` holds `holds(logic_lean4, Checked(payload), P)` — nanoda verified the artifact
/// at `payload` against `P` (D87 §2). It is what `witness_index::emit_from_trace` reads to admit a
/// `Verified` witness, keyed off this judgement's own `type`. Before it existed the checker's
/// result was computed and discarded, and the trace was a note that a check RAN, with `Verified`
/// admitted on the strength of the note.
///
/// `prov:permitted_axioms` and the checker-identity pair are the two inputs a verdict is a
/// deterministic function of that nothing recorded (D87 §5). With them pinned any party can re-run
/// `check_proof` and obtain the same verdict, which is stronger than a receipt: a receipt says "I
/// checked", this says "check it yourself".
///
/// `None` when the proof term has no `@id`: the trace's IRI is derived from it, and a dispatch over
/// an embedded resource commits nothing to derive from. `finalize_emitted_resource` drops such an
/// emission for the same reason. `Err` when the judgement will not encode — a `Holds` whose result
/// cannot be written down must not commit as a bare note that a check happened.
#[allow(clippy::result_large_err)]
fn verification_trace(
    term: &Resource,
    payload: &Resource,
    claim_iri: &Iri,
    proposition: &Exp,
    permitted_axioms: &[String],
    layer: &Arc<eigenius_kernel::layer::Layer>,
) -> Result<Option<Resource>, InstitutionError> {
    use eigenius_kernel::program::eigentt_type_mirror::{
        encode_judgement, encode_type, CodecNames,
    };

    let Some(term_iri) = term.id() else {
        return Ok(None);
    };
    let proof_term_iri = payload.id().unwrap_or(term_iri);
    let Ok(trace_iri) = Iri::parse(&format!("{term_iri}:verification")) else {
        return Ok(None);
    };

    // `Checked` names the ARTIFACT nanoda examined, so the term anchors to the bytes. It is not an
    // `eigentt:Axiom`: that is "a closed term whose type the kernel admits without checking the
    // term itself", the opposite of what is being recorded (D87 §4.1).
    let names = CodecNames::from_layer(layer);
    let encode_err = |what: &str, e: String| {
        InstitutionError::ComputationFailed(format!(
            "the proof checked out, but its {what} would not encode, so the result could not be \
             written to the chain: {e}"
        ))
    };
    let checked = encode_type(&Exp::Checked(proof_term_iri.clone()), &names)
        .map_err(|e| encode_err("checked-term reference", format!("{e:?}")))?;
    let prop = encode_type(proposition, &names)
        .map_err(|e| encode_err("proposition", format!("{e:?}")))?;
    let judgement = encode_judgement(LOGIC_LEAN4, &checked, &prop, &names)
        .map_err(|e| encode_err("judgement", format!("{e:?}")))?;

    let (identity_kind, identity) = checker_identity();

    let mut trace = Resource::new(trace_iri);
    trace.set(
        Iri::parse(wk::IS_A).expect("well-known IRI"),
        Value::Array(vec![Value::String(wk::VERIFICATION_TRACE.to_string())]),
    );
    trace.set(
        Iri::parse(wk::REFLECTION_RESOURCE).expect("well-known IRI"),
        Value::iri(claim_iri),
    );
    trace.set(
        Iri::parse(wk::PROV_JUDGEMENT).expect("well-known IRI"),
        judgement,
    );
    trace.set(
        Iri::parse(wk::PROOF_SYSTEM).expect("well-known IRI"),
        Value::String(PROOF_SYSTEM_LEAN4.to_string()),
    );
    trace.set(
        Iri::parse(wk::PROOF_TERM).expect("well-known IRI"),
        Value::String(proof_term_iri.as_str().to_string()),
    );
    trace.set(
        Iri::parse(wk::PERMITTED_AXIOMS).expect("well-known IRI"),
        Value::Array(
            permitted_axioms
                .iter()
                .map(|a| Value::String(a.clone()))
                .collect(),
        ),
    );
    trace.set(
        Iri::parse(wk::CHECKER_IDENTITY_KIND).expect("well-known IRI"),
        Value::String(identity_kind.to_string()),
    );
    trace.set(
        Iri::parse(wk::CHECKER_IDENTITY).expect("well-known IRI"),
        Value::String(identity),
    );
    trace.set(
        Iri::parse(wk::TIMESTAMP).expect("well-known IRI"),
        Value::String(millis_to_iso8601(now_millis())),
    );
    Ok(Some(trace))
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
