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

//! `ValidateJustification` handler (D39 §4.3).
//!
//! Algorithm:
//!
//! 1. Read `proposition` and `certificate` from the justification:Conclusion
//!    (D47-encoded EigenTT terms) — decoded via the kernel's D47 codec.
//! 2. Lift the `justification` property into a typed `Val` via
//!    `extract_typed(ef_justification, sentence, ctx)` — the kernel's
//!    standard "chain resource → typed kernel value" surface, with
//!    the lifting logic in [`crate::extract`].
//! 3. Resolve the `justification:Certificate` inductive declaration from the layer.
//! 4. Type-check the proposition at `Prop` (= `Sort(0)`) and eval it
//!    to a `Val` to plug into the expected type's index slot.
//! 5. Construct the expected certificate type
//!    `Val::InductiveType { decl: justification:Certificate, params: [], indices:
//!    [justification_val, proposition_val] }` directly at the Val
//!    layer — no Exp roundtrip needed.
//! 6. Type-check the certificate against that `Val` via the kernel's
//!    NbE checker.
//! 7. Return `Verdict::Holds` on success, `Verdict::Fails { diagnostic }`
//!    on any failure (with the kernel's type error string carried in
//!    the diagnostic).

use eigenius_kernel::context::ExecutionContext;
use eigenius_kernel::institution::error::InstitutionError;
use eigenius_kernel::institution::runtime::{Institution, QueryOutcome};
use eigenius_kernel::nbe::check::{check, CheckCtx};
use eigenius_kernel::nbe::env::Rho;
use eigenius_kernel::nbe::eval::eval_env;
use eigenius_kernel::nbe::val::Val;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::program::eigentt_type_mirror::{certificate_indices, decode_judgement};
use eigenius_kernel::program::ground::resolve_class_type;
use eigenius_kernel::server::helpers::{millis_to_iso8601, now_millis};

use crate::institution::iris;
use crate::institution::ReasoningInstitution;

/// Top-level handler called by `ReasoningInstitution::query`. Routes
/// the per-step decoding through the standard kernel surfaces (D47
/// codec for type expressions, `extract_typed` for chain inductive
/// values) and builds the verdict.
pub fn do_validate_justification(
    inst: &ReasoningInstitution,
    sentence: &Resource,
    ctx: &ExecutionContext,
) -> Result<QueryOutcome, InstitutionError> {
    // ── Step 1: project proposition + certificate out of the judgement ──
    //
    // Both used to be slots of their own, and nothing required them to be
    // about the same claim. They are now the certificate term and the second
    // index of its type inside `holds(kernel, c, Certificate(j, P))`, so the
    // pairing this handler used to check by hand is checked at commit by the
    // uniform check-mode rule. What remains here is the institutional verdict,
    // which is provenance; the check itself is no longer this handler's to own
    // and the handler retires with the institution.
    let judgement_value = required_property(sentence, iris::PROP_JUDGEMENT)?;
    let judgement = match decode_judgement(&judgement_value, ctx.head()) {
        Ok(j) => j,
        Err(e) => return Ok(verdict_fails(format!("malformed judgement: {e:?}"))),
    };
    let certificate_exp = judgement.term.clone();
    let proposition_exp = match certificate_indices(&judgement.typ) {
        Some((_, p)) => p.clone(),
        None => {
            return Ok(verdict_fails(
                "a conclusion's judgement must be checked against \
                 justification:Certificate(j, P)"
                    .to_string(),
            ))
        }
    };

    // ── Step 2: lift justification via extract_typed ─────────────────
    //
    // Routes through the institution's own `extract_typed` so the
    // chain → Val translation rides on the kernel's standard surface rather
    // than a free kernel utility. The handler in `crate::extract`
    // returns a `Val::InductiveVal` typed at `justification:Term`.
    let ef_proc = Iri::parse(iris::PROC_EXTRACT_JUSTIFICATION).expect("static IRI");
    let justification_val = match inst.extract_typed(&ef_proc, sentence, ctx) {
        Ok(v) => v,
        Err(InstitutionError::ComputationFailed(msg)) => {
            return Ok(verdict_fails(msg));
        }
        Err(e) => return Err(e),
    };

    // ── Step 3: resolve justification:Certificate inductive declaration ────────────
    let jb_iri = Iri::parse(iris::JUSTIFIED_BY).expect("static IRI");
    let jb_decl = match resolve_class_type(&jb_iri, ctx.head()) {
        Ok(Val::InductiveType { decl, .. }) => decl,
        Ok(other) => {
            return Ok(verdict_fails(format!(
                "`{}` resolved to a non-inductive value: {other:?}",
                iris::JUSTIFIED_BY
            )));
        }
        Err(e) => {
            return Err(InstitutionError::ComputationFailed(format!(
                "failed to resolve justification:Certificate inductive: {e}"
            )));
        }
    };

    // ── Step 4: type-check proposition at Prop = Sort(0), then eval ──
    let mut prop_ctx = CheckCtx::with_layer(Rho::Nil, Vec::new(), ctx.head().clone());
    if let Err(e) = check(&mut prop_ctx, &proposition_exp, &Val::sort(0)) {
        return Ok(verdict_fails(format!(
            "proposition does not type-check at Prop: {e}"
        )));
    }
    // In the chain's environment, not an empty one: the proposition names
    // declarations, and an env-less eval would leave each a neutral (D76).
    let proposition_val = match eval_env(
        &proposition_exp,
        &Rho::Nil,
        &eigenius_kernel::nbe::env_global::Env::of(ctx.head().clone()),
    ) {
        Ok(v) => v,
        Err(e) => {
            return Err(InstitutionError::ComputationFailed(format!(
                "failed to evaluate proposition: {e:?}"
            )));
        }
    };

    // ── Step 5: construct expected type `justification:Certificate(j, p)` as Val ───
    //
    // justification:Certificate has 0 params + 2 indices (per the D39 §5 declaration
    // `justification:Certificate : justification:Term -> Prop -> Type 0`). Building
    // the Val directly avoids an Exp roundtrip + eval — both index
    // sub-values are already in Val form (justification_val from
    // extract_typed, proposition_val from the eval above).
    let expected_type_val = Val::InductiveType {
        decl: jb_decl,
        params: Vec::new(),
        indices: vec![justification_val, proposition_val],
    };

    // ── Step 6: type-check certificate against justification:Certificate(j, p) ─────
    let mut cert_ctx = CheckCtx::with_layer(Rho::Nil, Vec::new(), ctx.head().clone());
    if let Err(e) = check(&mut cert_ctx, &certificate_exp, &expected_type_val) {
        return Ok(verdict_fails(format!(
            "certificate does not type-check against `justification:Certificate(justification, proposition)`: {e}"
        )));
    }

    // ── Step 7: a VerificationTrace, but only for an actual PROOF ────
    //
    // The certificate type-checking is not verification of the proposition.
    // `Certificate(j, P)` says *j grounds a claim to P*; a checker confirming
    // that says nothing about `P` itself. A `VerificationTrace` is declared as
    // "a proof of a resource's proposition was checked by a proof system", so
    // minting one off the certificate check asserted the thing the two-layer
    // separation exists to deny — and it was the chain-side half of the same
    // defect the witness emitter had.
    //
    // eigenius#200 added this trace because Verified witnesses were otherwise
    // traceless. That reason survives, and so does the trace — it now
    // accompanies the judgement that actually establishes `P`,
    // `justification:proof`, which is also what the witness emitter keys off.
    // The two stay in step by construction.
    //
    // A conclusion with no proof mints nothing here. Its certificate check is
    // still recorded — by the Verdict, which is provenance of the
    // institutional act and is what that check actually produced.
    let trace = match (sentence.id(), sentence.get(&proof_iri())) {
        (Some(sentence_iri), Some(_)) => vec![verification_trace(sentence_iri)],
        // No proof term, or an embedded sentence with no IRI to attest. The
        // gate still answers; there is simply no verification to record.
        _ => Vec::new(),
    };
    Ok(QueryOutcome {
        output: verdict_resource(wk::VERDICT_HOLDS, None),
        derivations: trace,
        partial_invocation: None,
    })
}

/// The `reflection:VerificationTrace` a passing `ValidateJustification` mints.
///
/// `proof_system` is the kernel itself, which is what distinguishes this from a Lean / Coq / Agda
/// trace — the two are the same kind of artifact by different verifiers, so they are one class and
/// not two (eigenius#200). `proof_term` is the sentence's own IRI: the certificate lives on the
/// sentence, so the sentence IS the proof term's location, and unlike an external prover's blob it
/// is already chain-resident.
///
/// `derivation_trace` is deliberately absent. It is `recommends`, not `requires`, precisely for
/// this case: a `justification:Conclusion` has no `ProgramTrace` to point at — D39 §4.2 satisfies its
/// inherited derivation requirement with the certificate field — and pointing the slot at itself to
/// satisfy a schema would be a fiction.
/// The conclusion's optional proof judgement.
fn proof_iri() -> Iri {
    Iri::parse("urn:eigenius:justification:proof").expect("static IRI")
}

/// The chain-side audit artifact for a checked PROOF.
///
/// `proof_term` names the conclusion carrying the proof judgement the kernel
/// checked at `t : P`. It used to name the conclusion whose CERTIFICATE
/// type-checked, which is a different and weaker fact — the property's own
/// description still says so, and is corrected with the next bootstrap edit
/// rather than mid-reseed.
fn verification_trace(sentence_iri: &Iri) -> Resource {
    const KERNEL_PROOF_SYSTEM: &str = "urn:eigenius:kernel";
    let trace_iri = Iri::parse(&format!("{}:verification", sentence_iri.as_str()))
        .expect("a sentence IRI with a `:verification` suffix parses");
    let mut r = Resource::new(trace_iri);
    r.set(
        Iri::parse(wk::IS_A).expect("well-known IRI"),
        Value::Array(vec![Value::String(wk::VERIFICATION_TRACE.to_string())]),
    );
    r.set(
        Iri::parse(wk::REFLECTION_RESOURCE).expect("well-known IRI"),
        Value::String(sentence_iri.as_str().to_string()),
    );
    r.set(
        Iri::parse(wk::PROOF_SYSTEM).expect("well-known IRI"),
        Value::String(KERNEL_PROOF_SYSTEM.to_string()),
    );
    r.set(
        Iri::parse(wk::PROOF_TERM).expect("well-known IRI"),
        Value::String(sentence_iri.as_str().to_string()),
    );
    r.set(
        Iri::parse(wk::TIMESTAMP).expect("well-known IRI"),
        Value::String(millis_to_iso8601(now_millis())),
    );
    r
}

/// Read a required property off the justification:Conclusion; fail with a
/// `ComputationFailed` error if missing. The validator at commit time
/// (Rule 16 + the resource-class `requires` enforcement) should catch
/// this before we reach the handler, but the defensive check keeps the
/// failure mode legible if the institution dispatches against a
/// malformed input.
fn required_property(sentence: &Resource, prop_iri: &str) -> Result<Value, InstitutionError> {
    let iri = Iri::parse(prop_iri).expect("static IRI");
    sentence.get(&iri).cloned().ok_or_else(|| {
        InstitutionError::ComputationFailed(format!(
            "justification:Conclusion missing required `{prop_iri}` property"
        ))
    })
}

/// Build the chain-shaped Fails verdict carrying a diagnostic string.
fn verdict_fails(diagnostic: String) -> QueryOutcome {
    QueryOutcome::from_output(verdict_resource(wk::VERDICT_FAILS, Some(&diagnostic)))
}

/// Build the chain-shaped Undecidable verdict carrying a diagnostic
/// string. Used by EntailmentQuery / ConsistencyCheck handlers when
/// the v1 implementation can't decide.
pub(crate) fn verdict_undecidable(diagnostic: String) -> QueryOutcome {
    QueryOutcome::from_output(verdict_resource(wk::VERDICT_UNDECIDABLE, Some(&diagnostic)))
}

/// Build the Verdict::Holds | Fails | Undecidable resource shape the
/// kernel's commit pipeline expects. Mirrors
/// `LeanInstitution::verdict_resource`. Re-exported to sibling
/// handlers (entailment, consistency) that surface their own verdicts.
pub(crate) fn verdict_resource(ctor_name: &str, diagnostic: Option<&str>) -> Resource {
    const DIAGNOSTIC_IRI: &str = "urn:eigenius:institution:diagnostic";
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
            Iri::parse(DIAGNOSTIC_IRI).expect("static IRI"),
            Value::String(d.to_string()),
        );
    }
    r
}
