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

//! `EntailmentQuery` handler (D39 §4.3 OnDemand).
//!
//! Asks: "does the chain warrant this candidate proposition?" v1 of
//! the handler is a *lookup-based* search — it walks the layer chain
//! for committed `justification:Conclusion` resources and returns
//! `Verdict::Holds` when it finds one whose proposition matches the
//! query candidate (syntactic `Exp` equality after D47 decode).
//!
//! Bounded-depth proof search over `justification:Term` constructors —
//! the spec's full algorithm — is follow-on work. v1's surface
//! intentionally does the useful-but-trivial case ("have I already
//! committed a sentence claiming this?"), and reports `Undecidable`
//! when no matching sentence exists.
//!
//! `Undecidable` (not `Fails`) is the honest answer on a miss:
//! absence of a matching sentence isn't proof that no warrant
//! exists, only that the v1 handler couldn't find one.

use eigenius_kernel::context::ExecutionContext;
use eigenius_kernel::institution::error::InstitutionError;
use eigenius_kernel::institution::runtime::QueryOutcome;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::Resource;
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::program::eigentt_type_mirror::{
    certificate_indices, decode_judgement, decode_type,
};

use crate::institution::iris;
use crate::validate::{verdict_resource, verdict_undecidable};

/// `query` handler for `proc:entailment_query`.
pub fn do_entailment_query(
    request: &Resource,
    ctx: &ExecutionContext,
) -> Result<QueryOutcome, InstitutionError> {
    // ── Step 1: read + decode the candidate proposition ──────────────
    let candidate_iri = Iri::parse(iris::PROP_CANDIDATE_PROPOSITION).expect("static IRI");
    let candidate_value = request.get(&candidate_iri).ok_or_else(|| {
        InstitutionError::ComputationFailed(format!(
            "EntailmentRequest missing required `{}` property",
            iris::PROP_CANDIDATE_PROPOSITION
        ))
    })?;
    let candidate_exp = match decode_type(candidate_value, ctx.head()) {
        Ok(e) => e,
        Err(e) => {
            return Ok(verdict_undecidable(format!(
                "candidate proposition does not decode through the D47 codec: {e:?}"
            )));
        }
    };

    // ── Step 2: walk the layer chain for ReasoningSentences ──────────
    //
    // `iter_all_resources` returns the merged-view chain set; the
    // top-of-chain wins for duplicate IRIs, which is the right
    // semantics here (most recently committed proposition is
    // authoritative).
    let sentence_class = Iri::parse("urn:eigenius:justification:Conclusion").expect("static IRI");
    let judgement_iri = Iri::parse(iris::PROP_JUDGEMENT).expect("static IRI");

    for (iri, resource) in ctx.head().iter_all_resources() {
        if !resource.is_instance_of(&sentence_class) {
            continue;
        }
        // The proposition is projected out of the conclusion's judgement —
        // it has no slot of its own. A conclusion missing or malforming it
        // should have been rejected at commit by the requires-list and the
        // uniform check-mode rule; skip defensively rather than fail the whole
        // query on one bad row.
        let Some(stored) = resource.get(&judgement_iri) else {
            continue;
        };
        let Ok(judgement) = decode_judgement(stored, ctx.head()) else {
            continue;
        };
        let Some((_, prop)) = certificate_indices(&judgement.typ) else {
            continue;
        };
        let prop_exp = prop.clone();
        if prop_exp == candidate_exp {
            // Holds — citation goes in the diagnostic so a caller can
            // recover the witnessing sentence's IRI without parsing
            // the Verdict resource's structure.
            return Ok(QueryOutcome::from_output(verdict_resource(
                wk::VERDICT_HOLDS,
                Some(&format!(
                    "candidate proposition matches committed sentence `{iri}`"
                )),
            )));
        }
    }

    // No matching sentence found. Undecidable is the honest answer —
    // a fully-correct bounded-depth proof search might still find a
    // composite warrant. v1 doesn't attempt that.
    Ok(verdict_undecidable(
        "no committed justification:Conclusion's proposition syntactically matches the candidate; \
         v1's lookup-based search does not attempt bounded-depth proof composition"
            .to_string(),
    ))
}
