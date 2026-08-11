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

//! Emit the D62 pipeline record for a parsed document as Eigon-JSON.
//!
//! The vocabulary is the **committed** D62 ontology (`ontologies/encoding/encoding.esl`) — nothing
//! demo-specific. Per sentence:
//!
//! ```text
//!   enc:DiscourseUnit    prose + character span in the source
//!   enc:ScopedUnit       (thin — unscoped; the whole chain is the scope)
//!   enc:EncodedClaim     reflection:canonical_proposition = the parsed Prop, D47-encoded
//!   reflection:ProgramTrace  ──▶  IsDerivedAs claim_iri P     ← the witness downstream cites
//!   enc:DecisionPoint    which reading was taken, out of how many, and on whose authority
//! ```
//!
//! The `ProgramTrace` is what makes this **Derived**: a program (the parser) produced the claim from
//! a hashed input span. A certificate that cites `derived(claim_iri, P)` therefore breaks the moment
//! the prose changes and the parser derives a different `P` — which is the whole point.

use eigenius_kernel::dcg::item::Item;
use eigenius_kernel::dcg::skeleton::skeleton_of;
use eigenius_kernel::dcg::SelectionOutcome;
use eigenius_kernel::ontology::eigon_json::serialize_document;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::program::eigentt_type_mirror::encode_type;

use crate::select::Pin;

const CORE: &str = "urn:eigenius:core";
const REFL: &str = "urn:eigenius:reflection";
const ENC: &str = "urn:eigenius:encoding";

/// On whose authority a sentence's reading was taken — the emitted `DecisionPoint` records it.
pub enum SentenceSelection<'a> {
    /// A human-verified skeleton pin (the declared gate arm). Emits exactly the historical
    /// record shape — the committed demo artifacts regenerate byte-identically under this arm.
    Pinned(&'a Pin),
    /// The reading ranker's choice (live or replayed) — the computed-choice record
    /// (`d63-reading-selection.md` slice 5): authority individual, the ranker's rationale
    /// verbatim, and the runner-up skeletons it ranked the choice against.
    Ranked(&'a SelectionOutcome),
    /// The forest offered a single reading — no choice existed to make.
    Sole,
}

/// One sentence that parsed and whose reading was selected.
pub struct ParsedSentence<'a> {
    /// 1-based position in the document — the local-name key for every resource emitted for it.
    pub ordinal: usize,
    pub text: String,
    /// Character offsets of `text` in the source file.
    pub span: (usize, usize),
    /// The selected reading.
    pub item: &'a Item,
    /// How many closed readings the forest offered (`1` = the unit encoded on its own).
    pub candidates: usize,
    /// How the reading was selected — recorded on the chain via the `DecisionPoint`.
    pub selection: SentenceSelection<'a>,
}

#[derive(Debug)]
pub enum EmitError {
    /// The parsed `Prop` is outside the D47 chain-mirrored type fragment.
    Encode { ordinal: usize, detail: String },
}

impl std::fmt::Display for EmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Encode { ordinal, detail } => write!(
                f,
                "sentence {ordinal}: the parsed proposition is not expressible in the D47 \
                 chain-mirrored type fragment — {detail}"
            ),
        }
    }
}

/// Build the Eigon-JSON document for a parsed source file.
///
/// `ns` is the IRI prefix the emitted resources live under (e.g. `urn:eigenius:demo:prose`);
/// `source_sha256` and `source_path` pin *which bytes* were parsed, so a prose edit is visible on
/// the chain and not only in the propositions.
pub fn emit_document(
    ns: &str,
    source_path: &str,
    source_sha256: &str,
    timestamp: &str,
    sentences: &[ParsedSentence<'_>],
) -> Result<String, EmitError> {
    let mut out: Vec<Resource> = Vec::new();
    for s in sentences {
        let n = s.ordinal;
        let unit_iri = format!("{ns}:unit_{n}");
        let scoped_iri = format!("{ns}:scoped_{n}");
        let claim_iri = format!("{ns}:claim_{n}");

        let mut unit = res(&unit_iri, &[&format!("{ENC}:DiscourseUnit")]);
        unit.set(iri(&format!("{ENC}:prose")), Value::String(s.text.clone()));
        unit.set(
            iri(&format!("{ENC}:unit_kind")),
            Value::ResourceRef(iri(&format!("{ENC}:kind_prose"))),
        );
        unit.set(
            iri(&format!("{ENC}:span_start")),
            Value::Integer(s.span.0 as i64),
        );
        unit.set(
            iri(&format!("{ENC}:span_end")),
            Value::Integer(s.span.1 as i64),
        );
        unit.set(
            iri(&format!("{ENC}:section")),
            Value::String(format!("{source_path} (sha256 {source_sha256})")),
        );
        out.push(unit);

        let mut scoped = res(&scoped_iri, &[&format!("{ENC}:ScopedUnit")]);
        scoped.set(
            iri(&format!("{ENC}:unit")),
            Value::ResourceRef(iri(&unit_iri)),
        );
        out.push(scoped);

        let prop = encode_type(s.item.sem()).map_err(|e| EmitError::Encode {
            ordinal: n,
            detail: format!("{e:?}"),
        })?;
        let mut claim = res(&claim_iri, &[&format!("{ENC}:EncodedClaim")]);
        claim.set(iri(&format!("{REFL}:canonical_proposition")), prop);
        claim.set(
            iri(&format!("{ENC}:from_unit")),
            Value::ResourceRef(iri(&scoped_iri)),
        );
        let claim_desc = match &s.selection {
            SentenceSelection::Pinned(pin) => format!(
                "«{}» — the reading pinned as correct: {}",
                s.text, pin.skeleton
            ),
            SentenceSelection::Ranked(sel) => format!(
                "«{}» — the reading the ranker selected: {}",
                s.text, sel.chosen_skeleton
            ),
            SentenceSelection::Sole => format!(
                "«{}» — the sole surviving reading: {}",
                s.text,
                skeleton_of(s.item.sem())
            ),
        };
        claim.set(
            iri(&format!("{CORE}:description")),
            Value::String(claim_desc),
        );
        out.push(claim);

        // The witness. `reflection:resource` → the claim, so the emitter mints
        // `IsDerivedAs claim_iri P` where P is the claim's canonical_proposition.
        let mut trace = res(
            &format!("{ns}:trace_{n}"),
            &[&format!("{REFL}:ProgramTrace")],
        );
        trace.set(
            iri(&format!("{REFL}:resource")),
            Value::ResourceRef(iri(&claim_iri)),
        );
        trace.set(
            iri(&format!("{REFL}:source")),
            Value::String(format!(
                "eigenius-encoding prose-to-eigon: DCG parse (D63) of {source_path} \
                 chars {}..{} (source sha256 {source_sha256})",
                s.span.0, s.span.1
            )),
        );
        trace.set(
            iri(&format!("{REFL}:timestamp")),
            Value::String(timestamp.to_string()),
        );
        out.push(trace);

        // Selection is recorded even when the unit was unambiguous, so the chain always says on
        // whose authority the reading was taken. The PIN arm emits the exact historical shape
        // (no `selected_by`; its authority is stated in the rationale text) so the committed demo
        // artifacts regenerate byte-identically; the ranker and sole arms carry the
        // `SelectionAuthority` individual, and the ranker arm additionally the runner-up
        // skeletons — the choice's audit trail (there is no kernel veto on selection).
        let mut dp = res(
            &format!("{ns}:decision_{n}"),
            &[&format!("{ENC}:DecisionPoint")],
        );
        dp.set(
            iri(&format!("{ENC}:decision_unit")),
            Value::ResourceRef(iri(&scoped_iri)),
        );
        dp.set(
            iri(&format!("{ENC}:selected_claim")),
            Value::ResourceRef(iri(&claim_iri)),
        );
        dp.set(
            iri(&format!("{ENC}:candidate_count")),
            Value::Integer(s.candidates as i64),
        );
        match &s.selection {
            SentenceSelection::Pinned(pin) => {
                dp.set(
                    iri(&format!("{REFL}:rationale")),
                    Value::String(format!(
                        "Reading selected by SKELETON PIN, not by the pipeline: the one reading whose \
                         sense-erased skeleton equals the human-verified pin. Structural disambiguation \
                         (D62 S4) is open work — this is declared selection, and it fails closed if the pin \
                         matches zero or several readings. Pin note: {}",
                        if pin.note.is_empty() { "(none)" } else { &pin.note }
                    )),
                );
            }
            SentenceSelection::Ranked(sel) => {
                dp.set(
                    iri(&format!("{ENC}:selected_by")),
                    Value::ResourceRef(iri(&format!("{ENC}:authority_ranker"))),
                );
                if !sel.runner_up_skeletons.is_empty() {
                    dp.set(
                        iri(&format!("{ENC}:runner_up_skeletons")),
                        Value::Array(
                            sel.runner_up_skeletons
                                .iter()
                                .map(|s| Value::String(s.clone()))
                                .collect(),
                        ),
                    );
                }
                dp.set(
                    iri(&format!("{REFL}:rationale")),
                    Value::String(format!(
                        "Reading selected by the READING RANKER (d63-reading-selection): an \
                         untrusted choice in document context, recorded for audit — every \
                         candidate type-checks, so no kernel veto exists; the reading-adjudication \
                         ledger and this record are the controls. Ranker rationale: {}",
                        sel.rationale
                    )),
                );
            }
            SentenceSelection::Sole => {
                dp.set(
                    iri(&format!("{ENC}:selected_by")),
                    Value::ResourceRef(iri(&format!("{ENC}:authority_sole"))),
                );
                dp.set(
                    iri(&format!("{REFL}:rationale")),
                    Value::String(
                        "Sole surviving reading — the forest offered exactly one felicitous \
                         parse; no selection existed to make."
                            .to_string(),
                    ),
                );
            }
        }
        out.push(dp);
    }
    Ok(serde_json::to_string_pretty(&serialize_document(&out)).expect("serialize Eigon-JSON"))
}

/// The skeleton the parser actually produced for a reading — used by the driver's report.
pub fn reading_skeleton(item: &Item) -> String {
    skeleton_of(item.sem())
}

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("well-formed IRI")
}

fn res(id: &str, classes: &[&str]) -> Resource {
    let mut r = Resource::new(iri(id));
    r.set(
        iri(&format!("{CORE}:is_a")),
        Value::Array(classes.iter().map(|c| Value::ResourceRef(iri(c))).collect()),
    );
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use eigenius_kernel::dcg::{Combinator, Cost};
    use eigenius_kernel::nbe::term::Exp;

    /// A minimal encodable Item — an `EigonClass` sem is inside the D47 fragment, and the cat is
    /// never consulted by emission.
    fn item() -> Item {
        let cls = Exp::EigonClass(iri("urn:eigenius:demo:Thing"));
        Item::from_parts(cls.clone(), cls, Combinator::Other, Cost::ZERO)
    }

    fn sentence<'a>(item: &'a Item, selection: SentenceSelection<'a>) -> ParsedSentence<'a> {
        ParsedSentence {
            ordinal: 1,
            text: "A thing.".to_string(),
            span: (0, 8),
            item,
            candidates: 3,
            selection,
        }
    }

    fn emit(s: &[ParsedSentence<'_>]) -> String {
        emit_document(
            "urn:eigenius:test:doc",
            "test.txt",
            "deadbeef",
            "2026-08-11T00:00:00Z",
            s,
        )
        .expect("emits")
    }

    #[test]
    fn ranked_selection_emits_the_computed_choice_record() {
        let it = item();
        let sel = SelectionOutcome {
            chosen_skeleton: "skel-chosen".to_string(),
            chosen_sem: "sem-chosen".to_string(),
            chosen_gloss: "a thing".to_string(),
            rationale: "the document is about things".to_string(),
            runner_up_skeletons: vec!["skel-b".to_string(), "skel-c".to_string()],
            candidates: 3,
        };
        let json = emit(&[sentence(&it, SentenceSelection::Ranked(&sel))]);
        assert!(json.contains("urn:eigenius:encoding:authority_ranker"));
        assert!(json.contains("urn:eigenius:encoding:runner_up_skeletons"));
        assert!(json.contains("skel-b") && json.contains("skel-c"));
        assert!(json.contains("the document is about things"));
        assert!(json.contains("READING RANKER"));
        assert!(
            json.contains("the reading the ranker selected: skel-chosen"),
            "the claim description names the chosen skeleton"
        );
    }

    #[test]
    fn sole_selection_emits_the_sole_authority_and_no_runners_up() {
        let it = item();
        let json = emit(&[sentence(&it, SentenceSelection::Sole)]);
        assert!(json.contains("urn:eigenius:encoding:authority_sole"));
        assert!(!json.contains("runner_up_skeletons"));
        assert!(json.contains("Sole surviving reading"));
    }

    /// The PIN arm's record is BYTE-STABLE: no `selected_by`, the historical rationale text —
    /// the committed demo artifacts regenerate identically under it.
    #[test]
    fn pinned_selection_emits_the_historical_shape() {
        let it = item();
        let pin = crate::select::Pin {
            sentence: "A thing.".to_string(),
            skeleton: "skel-pinned".to_string(),
            note: "verified".to_string(),
        };
        let json = emit(&[sentence(&it, SentenceSelection::Pinned(&pin))]);
        assert!(json.contains("Reading selected by SKELETON PIN"));
        assert!(json.contains("Pin note: verified"));
        assert!(json.contains("the reading pinned as correct: skel-pinned"));
        assert!(
            !json.contains("selected_by") && !json.contains("authority_"),
            "the pin arm emits no SelectionAuthority — byte-stability of committed artifacts"
        );
    }
}
