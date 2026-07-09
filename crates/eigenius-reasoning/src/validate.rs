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
//! 1. Read `proposition` and `certificate` from the ReasoningSentence
//!    (D47-encoded EigenTT terms) — decoded via the kernel's D47 codec.
//! 2. Lift the `justification` property into a typed `Val` via
//!    `extract_typed(ef_justification, sentence, ctx)` — the kernel's
//!    standard "chain resource → typed kernel value" surface, with
//!    the lifting logic in [`crate::extract`].
//! 3. Resolve the `JustifiedBy` inductive declaration from the layer.
//! 4. Type-check the proposition at `Prop` (= `Sort(0)`) and eval it
//!    to a `Val` to plug into the expected type's index slot.
//! 5. Construct the expected certificate type
//!    `Val::InductiveType { decl: JustifiedBy, params: [], indices:
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
use eigenius_kernel::nbe::eval::eval;
use eigenius_kernel::nbe::val::Val;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::program::eigentt_type_mirror::decode_type;
use eigenius_kernel::program::ground::resolve_class_type;

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
    // returns a `Val::InductiveVal` typed at `JustificationTerm`.
    let ef_proc = Iri::parse(iris::PROC_EXTRACT_JUSTIFICATION).expect("static IRI");
    let justification_val = match inst.extract_typed(&ef_proc, sentence, ctx) {
        Ok(v) => v,
        Err(InstitutionError::ComputationFailed(msg)) => {
            return Ok(verdict_fails(msg));
        }
        Err(e) => return Err(e),
    };

    // ── Step 3: resolve JustifiedBy inductive declaration ────────────
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
                "failed to resolve JustifiedBy inductive: {e}"
            )));
        }
    };

    // ── Step 4: type-check proposition at Prop = Sort(0), then eval ──
    let mut prop_ctx = CheckCtx::with_layer(Rho::Nil, Vec::new(), ctx.head().clone());
    if let Err(e) = check(&mut prop_ctx, &proposition_exp, &Val::Sort(0)) {
        return Ok(verdict_fails(format!(
            "proposition does not type-check at Prop: {e}"
        )));
    }
    let proposition_val = match eval(&proposition_exp, &Rho::Nil) {
        Ok(v) => v,
        Err(e) => {
            return Err(InstitutionError::ComputationFailed(format!(
                "failed to evaluate proposition: {e:?}"
            )));
        }
    };

    // ── Step 5: construct expected type `JustifiedBy(j, p)` as Val ───
    //
    // JustifiedBy has 0 params + 2 indices (per the D39 §5 declaration
    // `JustifiedBy : JustificationTerm -> Prop -> Type 0`). Building
    // the Val directly avoids an Exp roundtrip + eval — both index
    // sub-values are already in Val form (justification_val from
    // extract_typed, proposition_val from the eval above).
    let expected_type_val = Val::InductiveType {
        decl: jb_decl,
        params: Vec::new(),
        indices: vec![justification_val, proposition_val],
    };

    // ── Step 6: type-check certificate against JustifiedBy(j, p) ─────
    let mut cert_ctx = CheckCtx::with_layer(Rho::Nil, Vec::new(), ctx.head().clone());
    if let Err(e) = check(&mut cert_ctx, &certificate_exp, &expected_type_val) {
        return Ok(verdict_fails(format!(
            "certificate does not type-check against `JustifiedBy(justification, proposition)`: {e}"
        )));
    }

    Ok(QueryOutcome::from_output(verdict_resource(
        wk::VERDICT_HOLDS,
        None,
    )))
}

/// Read a required property off the ReasoningSentence; fail with a
/// `ComputationFailed` error if missing. The validator at commit time
/// (Rule 16 + the resource-class `requires` enforcement) should catch
/// this before we reach the handler, but the defensive check keeps the
/// failure mode legible if the institution dispatches against a
/// malformed input.
fn required_property(sentence: &Resource, prop_iri: &str) -> Result<Value, InstitutionError> {
    let iri = Iri::parse(prop_iri).expect("static IRI");
    sentence.get(&iri).cloned().ok_or_else(|| {
        InstitutionError::ComputationFailed(format!(
            "ReasoningSentence missing required `{prop_iri}` property"
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
