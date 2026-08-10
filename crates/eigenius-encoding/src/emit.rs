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
use eigenius_kernel::ontology::eigon_json::serialize_document;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::program::eigentt_type_mirror::encode_type;

use crate::select::Pin;

const CORE: &str = "urn:eigenius:core";
const REFL: &str = "urn:eigenius:reflection";
const ENC: &str = "urn:eigenius:encoding";

/// One sentence that parsed and whose pinned reading was selected.
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
    /// The pin the selection was made against.
    pub pin: &'a Pin,
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
        claim.set(
            iri(&format!("{CORE}:description")),
            Value::String(format!(
                "«{}» — the reading pinned as correct: {}",
                s.text, s.pin.skeleton
            )),
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
        // whose authority the reading was taken — the pin, not the pipeline.
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
        dp.set(
            iri(&format!("{REFL}:rationale")),
            Value::String(format!(
                "Reading selected by SKELETON PIN, not by the pipeline: the one reading whose \
                 sense-erased skeleton equals the human-verified pin. Structural disambiguation \
                 (D62 S4) is open work — this is declared selection, and it fails closed if the pin \
                 matches zero or several readings. Pin note: {}",
                if s.pin.note.is_empty() {
                    "(none)"
                } else {
                    &s.pin.note
                }
            )),
        );
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
