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

//! The `proc:project_justification` OnDemand handler.
//!
//! The support ALGEBRA it calls lives in [`eigenius_kernel::justification`]. It moved to the
//! kernel because the well-foundedness check needs it there: that check is stated over a
//! term's support, and `core:mentions` cannot express support because it records `App` and
//! `Sum` edges undifferentiated.
//!
//! What is left here is institution surface — reading a `ProjectionRequest`, resolving its
//! subject on the chain, and rendering a `justification:Projection`. P7 deletes it with the
//! rest of the institution.

use std::collections::BTreeSet;

use eigenius_kernel::context::ExecutionContext;
use eigenius_kernel::institution::error::InstitutionError;
use eigenius_kernel::institution::runtime::QueryOutcome;
use eigenius_kernel::justification::{support, Ground, Leaf};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};

use crate::institution::iris;

/// `proc:project_justification` — the OnDemand handler behind `qc_project_justification`.
///
/// Reads the request's `subject_sentence`, resolves it on the chain, extracts its
/// `justification:Term`, and reports every slice of the term's support at once. Computing the
/// support is the whole cost; slicing it is free, so there is no projection-kind parameter.
///
/// Returns a `justification:Projection`, not a `Verdict`. This REPORTS what a conclusion
/// rests on; it does not judge it, and it carries no `canonical_proposition` because it asserts
/// nothing.
pub fn do_project_justification(
    input: &Resource,
    ctx: &ExecutionContext,
) -> Result<QueryOutcome, InstitutionError> {
    let subject = input
        .get(&Iri::parse(iris::PROP_SUBJECT_SENTENCE).expect("static IRI"))
        .and_then(|v| match v {
            Value::ResourceRef(i) => Some(i.clone()),
            Value::String(s) => Iri::parse(s).ok(),
            _ => None,
        })
        .ok_or_else(|| {
            InstitutionError::ComputationFailed(
                "ProjectionRequest missing required `subject_sentence`".to_string(),
            )
        })?;

    let sentence = ctx.head().resolve(&subject).ok_or_else(|| {
        InstitutionError::ComputationFailed(format!(
            "ProjectionRequest `subject_sentence` `{subject}` does not resolve on the chain"
        ))
    })?;

    // Reuse the ExportFormat path the validate handler uses, then read back the syntactic tree:
    // `support` walks constructor applications, which is what the term IS.
    let term = crate::extract::justification_exp(&sentence, ctx)?;

    let sets = support(&term).map_err(|e| {
        InstitutionError::ComputationFailed(format!("malformed justification: {e}"))
    })?;

    let counterfactual = input
        .get(&Iri::parse(iris::PROP_COUNTERFACTUAL_IRI).expect("static IRI"))
        .and_then(|v| v.as_str().map(str::to_string));

    Ok(QueryOutcome::from_output(projection_resource(
        &subject,
        &sets,
        counterfactual.as_deref(),
    )))
}

/// Build the `justification:Projection` result resource from a computed support.
fn projection_resource(
    subject: &Iri,
    sets: &[BTreeSet<Leaf>],
    counterfactual: Option<&str>,
) -> Resource {
    let iri = |s: &str| Iri::parse(s).expect("static IRI");
    let mut r = Resource::new_embedded();
    r.set(
        iri(eigenius_kernel::ontology::well_known::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(
            iris::JUSTIFICATION_PROJECTION,
        ))]),
    );
    r.set(
        iri(iris::PROP_SUBJECT_SENTENCE),
        Value::ResourceRef(subject.clone()),
    );
    r.set(
        iri(iris::PROP_SUPPORT_COUNT),
        Value::Integer(sets.len() as i64),
    );
    // Existential over alternatives — Sum is disjunctive.
    r.set(
        iri(iris::PROP_FULLY_VERIFIED),
        Value::Boolean(
            sets.iter()
                .any(|s| s.iter().all(|l| l.ground == Ground::Verified)),
        ),
    );

    // Union across alternatives: these are exposure questions.
    let grounds = |g: Ground| {
        let mut v: Vec<Value> = sets
            .iter()
            .flatten()
            .filter(|l| l.ground == g)
            .map(|l| l.iri.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(Value::String)
            .collect();
        v.shrink_to_fit();
        v
    };
    for (prop, g) in [
        (iris::PROP_DECLARED_GROUNDS, Ground::Declared),
        (iris::PROP_OBSERVED_GROUNDS, Ground::Observed),
        (iris::PROP_VERIFIED_GROUNDS, Ground::Verified),
    ] {
        let vals = grounds(g);
        if !vals.is_empty() {
            r.set(iri(prop), Value::Array(vals));
        }
    }

    if let Some(x) = counterfactual {
        r.set(
            iri(iris::PROP_COUNTERFACTUAL_IRI),
            Value::String(x.to_string()),
        );
        r.set(
            iri(iris::PROP_SURVIVES_WITHOUT),
            Value::Boolean(sets.iter().any(|s| !s.iter().any(|l| l.iri == x))),
        );
    }
    r
}
