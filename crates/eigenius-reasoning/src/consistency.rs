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

//! `ConsistencyCheck` handler (D39 §4.3 Decidable).
//!
//! Asks: "are these committed ReasoningSentences internally
//! consistent under the institution's logic?" v1 of the handler
//! returns `Verdict::Undecidable` for every non-empty input — the
//! propositional-fragment decision procedure (let alone richer
//! fragments) is genuinely research-level work and outside Phase 7
//! scope. The handler exists so:
//!
//! - The QueryClass IRI is dispatch-bound: a FIBER call against
//!   `qc_consistency_check` routes here rather than landing on a
//!   "no handler" error.
//! - The input shape is *reserved*: the `ConsistencyRequest` class
//!   pins what a richer handler would consume, so the surface can
//!   evolve without forcing existing callers to rewrite.
//!
//! Empty-input requests get `Verdict::Holds` for free — the empty
//! sentence set is vacuously consistent — so callers that just want
//! to probe the dispatch surface aren't blocked by the v1 placeholder.

use eigenius_kernel::context::ExecutionContext;
use eigenius_kernel::institution::error::InstitutionError;
use eigenius_kernel::institution::runtime::QueryOutcome;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;

use crate::institution::iris;
use crate::validate::{verdict_resource, verdict_undecidable};

/// `query` handler for `proc:consistency_check`.
pub fn do_consistency_check(
    request: &Resource,
    _ctx: &ExecutionContext,
) -> Result<QueryOutcome, InstitutionError> {
    let set_iri = Iri::parse(iris::PROP_SENTENCE_SET).expect("static IRI");
    let set_value = request.get(&set_iri).ok_or_else(|| {
        InstitutionError::ComputationFailed(format!(
            "ConsistencyRequest missing required `{}` property",
            iris::PROP_SENTENCE_SET
        ))
    })?;

    // Empty sentence set is vacuously consistent — return Holds so
    // a caller probing the dispatch surface doesn't have to round-
    // trip a non-trivial input just to confirm the handler routes.
    let is_empty = match set_value {
        Value::Array(arr) => arr.is_empty(),
        _ => false,
    };
    if is_empty {
        return Ok(QueryOutcome::from_output(verdict_resource(
            wk::VERDICT_HOLDS,
            Some("empty sentence set is vacuously consistent"),
        )));
    }

    Ok(verdict_undecidable(
        "v1 of the ConsistencyCheck handler returns Undecidable for any non-empty input — \
         the propositional-fragment decision procedure is follow-on work. The QueryClass IRI \
         is dispatch-bound and the input shape is reserved so a richer handler can be plugged \
         in without surface churn"
            .to_string(),
    ))
}
