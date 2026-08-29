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
//! 1. Read `proposition` and `certificate` from the justification:Sentence
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
use eigenius_kernel::program::eigentt_type_mirror::decode_type;
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
    // ── Step 1: decode proposition + certificate via D47 codec ───────
    let proposition_value = required_property(sentence, iris::PROP_PROPOSITION)?;
    let certificate_value = required_property(sentence, iris::PROP_CERTIFICATE)?;
    let proposition_exp = match decode_type(&proposition_value, ctx.head()) {
        Ok(e) => e,
        Err(e) => return Ok(verdict_fails(format!("malformed proposition: {e:?}"))),
    };
    let certificate_exp = match decode_type(&certificate_value, ctx.head()) {
        Ok(e) => e,
        Err(e) => return Ok(verdict_fails(format!("malformed certificate: {e:?}"))),
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

    // ── Step 7: the certificate checked — emit the VerificationTrace ─
    //
    // eigenius#200. The kernel is a proof system, and a type-checked `justification:Certificate` certificate is a
    // proof term in it, so a passing check is a verification event like any other and gets the same
    // chain-side audit artifact the other three grounding families get. Without this the Verified
    // family was the one place D39 §5's invariant failed — `emit_from_reasoning_sentence`
    // synthesised a witness straight from the sentence, so every Verified witness on every chain
    // was traceless.
    //
    // The trace rides `outcome.derivations`, which the kernel commits alongside the Verdict only
    // when the gate Holds — so a `Fails` mints nothing, which is the point.
    let trace = match sentence.id() {
        Some(sentence_iri) => vec![verification_trace(sentence_iri)],
        // An embedded sentence has no IRI to attest; the gate still answers.
        None => Vec::new(),
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
/// this case: a `justification:Sentence` has no `ProgramTrace` to point at — D39 §4.2 satisfies its
/// inherited derivation requirement with the certificate field — and pointing the slot at itself to
/// satisfy a schema would be a fiction.
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

/// Read a required property off the justification:Sentence; fail with a
/// `ComputationFailed` error if missing. The validator at commit time
/// (Rule 16 + the resource-class `requires` enforcement) should catch
/// this before we reach the handler, but the defensive check keeps the
/// failure mode legible if the institution dispatches against a
/// malformed input.
fn required_property(sentence: &Resource, prop_iri: &str) -> Result<Value, InstitutionError> {
    let iri = Iri::parse(prop_iri).expect("static IRI");
    sentence.get(&iri).cloned().ok_or_else(|| {
        InstitutionError::ComputationFailed(format!(
            "justification:Sentence missing required `{prop_iri}` property"
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
