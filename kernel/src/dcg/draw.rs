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

//! **Proposer draws as chain resources** (D71 §9) — a run's LLM transcript, committed to its own
//! `doc-<id>` WORKING branch.
//!
//! A formalization run makes four kinds of untrusted call: sense ranking, reading selection,
//! anaphora proposal, discourse-kind classification. Each already records what it asked and what it
//! got, keyed on the question, so the run replays without an LLM. Until now that recording lived
//! only in JSON files travelling beside the source. This module puts it on the chain instead.
//!
//! ## Two records, two questions — neither subsumes the other
//!
//! An `enc:DecisionPoint` says WHAT WAS CHOSEN, for a reader of the graph, and travels in the
//! artifact to wherever the claims land. An `enc:ProposalDraw` says WHAT THE PROPOSER WAS ASKED AND
//! SAID, so the document re-runs LLM-free, and stays on the working branch, which is prunable. A
//! DecisionPoint carries no presented pool, no prior-selection context, and does not exist at all
//! for sense ranking or discourse-kind classification.
//!
//! ## Why the record is a transcript field, not a modelled structure
//!
//! `enc:draw_record` holds the seam's own Rust record type serialized verbatim. Those types ARE the
//! replay contract — each seam's key function reads them field by field — so modelling them a
//! second time in ESL would create two definitions of one contract with nothing keeping them in
//! step, and the first divergence would be a silent replay of the wrong answer.
//!
//! ## The consistency property this buys
//!
//! A draw is keyed on the PRESENTED POOL, and the pool is a function of the Stage-A glossary. On a
//! branch, the draw and the glossary that produced it are committed by the same run, so the
//! consistency the key enforces becomes structural rather than a filename convention. The
//! `2026-08-12` failure — a draw recorded against a different glossary, replayed here, answering a
//! different question — is not expressible in this arrangement.

use crate::layer::typed_resource_iris;
use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use sha2::{Digest, Sha256};

const CORE: &str = "urn:eigenius:core";
/// The provenance axis. Split out of `reflection`; see `ontologies/prov/prov.esl`.
const PROV: &str = "urn:eigenius:prov";
const ENC: &str = "urn:eigenius:encoding";

/// Which proposer an exchange came from. Mirrors the closed `enc:DrawSeam` enumeration — adding a
/// variant here means adding the individual there, deliberately, in the same change.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum DrawSeam {
    SenseRank,
    ReadingSelection,
    Anaphora,
    DiscourseKind,
}

impl DrawSeam {
    /// The `enc:` local name of the matching `DrawSeam` individual.
    pub fn local_name(self) -> &'static str {
        match self {
            Self::SenseRank => "seam_sense_rank",
            Self::ReadingSelection => "seam_reading_selection",
            Self::Anaphora => "seam_anaphora",
            Self::DiscourseKind => "seam_discourse_kind",
        }
    }

    /// A short slug for IRIs and log lines.
    pub fn slug(self) -> &'static str {
        match self {
            Self::SenseRank => "rank",
            Self::ReadingSelection => "selection",
            Self::Anaphora => "proposal",
            Self::DiscourseKind => "kind",
        }
    }

    pub fn from_local_name(name: &str) -> Option<Self> {
        Some(match name {
            "seam_sense_rank" => Self::SenseRank,
            "seam_reading_selection" => Self::ReadingSelection,
            "seam_anaphora" => Self::Anaphora,
            "seam_discourse_kind" => Self::DiscourseKind,
            _ => return None,
        })
    }

    pub fn all() -> [Self; 4] {
        [
            Self::SenseRank,
            Self::ReadingSelection,
            Self::Anaphora,
            Self::DiscourseKind,
        ]
    }
}

/// One exchange ready to commit: the replay key the seam computed, and the seam's record.
///
/// The seams produce these (each knows its own key function); this module knows nothing about what
/// is inside `record` beyond that it serializes.
pub struct KeyedDraw {
    pub key: String,
    pub record: serde_json::Value,
}

/// The draw's IRI — content-addressed on `(seam, key)`, so the same question recorded twice lands
/// at the same IRI and a re-run hits the anchored-commit cache instead of duplicating the branch.
fn draw_iri(ns: &str, seam: DrawSeam, key: &str) -> String {
    let mut h = Sha256::new();
    h.update(seam.local_name().as_bytes());
    h.update([0x1d]);
    h.update(key.as_bytes());
    format!(
        "{ns}:draw_{}_{:.16}",
        seam.slug(),
        hex::encode(h.finalize())
    )
}

/// Turn one seam's recorded exchanges into `enc:ProposalDraw` resources.
///
/// `model` names who answered when the run was live; deterministic proposers pass `None`. The
/// resources are ready to add to a `LayerBuilder` for the run's `doc-<id>` branch.
pub fn draw_resources(
    ns: &str,
    seam: DrawSeam,
    draws: &[KeyedDraw],
    model: Option<&str>,
    timestamp: &str,
) -> Result<Vec<Resource>, String> {
    let iri = |s: &str| Iri::parse(s).map_err(|e| format!("bad IRI {s}: {e:?}"));
    let mut out = Vec::with_capacity(draws.len());
    for d in draws {
        let id = draw_iri(ns, seam, &d.key);
        let mut r = Resource::new(iri(&id)?);
        r.set(
            iri(&format!("{CORE}:is_a"))?,
            Value::Array(vec![Value::String(
                iri(&format!("{ENC}:ProposalDraw"))?.as_str().to_string(),
            )]),
        );
        r.set(
            iri(&format!("{ENC}:draw_seam"))?,
            Value::String(
                iri(&format!("{ENC}:{}", seam.local_name()))?
                    .as_str()
                    .to_string(),
            ),
        );
        r.set(
            iri(&format!("{ENC}:draw_key"))?,
            Value::String(d.key.clone()),
        );
        r.set(
            iri(&format!("{ENC}:draw_record"))?,
            Value::String(
                serde_json::to_string(&d.record)
                    .map_err(|e| format!("serialize draw record: {e}"))?,
            ),
        );
        if let Some(m) = model {
            r.set(
                iri(&format!("{ENC}:draw_model"))?,
                Value::String(m.to_string()),
            );
        }
        r.set(
            iri(&format!("{PROV}:timestamp"))?,
            Value::String(timestamp.to_string()),
        );
        out.push(r);
    }
    Ok(out)
}

/// Read one seam's draws back off a layer chain as the JSON array its `Replay*::from_json` expects.
///
/// Index-driven: the `enc:ProposalDraw` subjects come from the triple index, and only those bodies
/// are resolved. A full-chain scan here would be O(chain) over a lexicon of millions of resources —
/// the failure mode that took a day to find in `build_axiom_env`.
///
/// Records come back in draw-IRI order, which is content-hash order — stable across runs, which is
/// what makes a re-read deterministic, but carries no meaning.
pub fn draws_from_layer(layer: &Layer, seam: DrawSeam) -> Result<String, String> {
    let seam_iri = Iri::parse(&format!("{ENC}:{}", seam.local_name()))
        .map_err(|e| format!("bad seam IRI: {e:?}"))?;
    let record_prop = Iri::parse(&format!("{ENC}:draw_record"))
        .map_err(|e| format!("bad property IRI: {e:?}"))?;
    let seam_prop =
        Iri::parse(&format!("{ENC}:draw_seam")).map_err(|e| format!("bad property IRI: {e:?}"))?;

    let mut records: Vec<serde_json::Value> = Vec::new();
    for id in typed_resource_iris(layer, &[&format!("{ENC}:ProposalDraw")]) {
        let Some(r) = layer.resolve(&id) else {
            continue;
        };
        // Filter to this seam. A chain can carry draws from several runs and several seams; the
        // caller asked for one seam's replay set.
        //
        // `as_iri_str`, never a variant match: CBOR persistence collapsed `ResourceRef` into
        // `String`, so a draw read back off a committed branch carries the string shape while one
        // built in memory carries the ref. Matching only the ref made every persisted draw
        // invisible — `draws_from_layer` returned `[]`, the run silently re-asked the model, and
        // the whole point of putting draws on the branch was lost with no error anywhere. Caught
        // `2026-08-20` by a second `eigenius formalize` on the same doc id reporting 11 draws
        // recorded instead of 0. The same invariant is why Rule 3 accepts a String IRI for
        // resource-typed properties.
        match r.get(&seam_prop).and_then(|v| v.as_iri_str()) {
            Some(s) if s == seam_iri.as_str() => {}
            _ => continue,
        }
        let Some(Value::String(text)) = r.get(&record_prop) else {
            return Err(format!("{id} has no enc:draw_record string"));
        };
        records.push(
            serde_json::from_str(text)
                .map_err(|e| format!("{id}: enc:draw_record is not valid JSON: {e}"))?,
        );
    }
    serde_json::to_string(&records).map_err(|e| format!("serialize draw set: {e}"))
}

/// Commit a run's draws onto its `doc-<id>` working branch, on top of whatever that branch already
/// holds (the doc-glossary layer the pipeline committed first).
///
/// This ADVANCES the branch rather than recreating it: the glossary layer beneath is what makes the
/// draws meaningful — a draw is keyed on the presented pool, and the pool is a function of that
/// glossary. Committing the draws anywhere else would reintroduce exactly the cross-glossary
/// mismatch the branch arrangement exists to prevent.
///
/// Returns the number of draws committed. An empty set is a no-op, not an empty layer: a run with
/// no LLM calls (every seam replaying, or every proposer deterministic) has no transcript to keep.
pub fn commit_draws(
    backend: &std::sync::Arc<dyn crate::storage::PersistentBackend>,
    doc_id: &str,
    parent: std::sync::Arc<Layer>,
    resources: Vec<Resource>,
) -> Result<usize, String> {
    use crate::commit::{BackendPersister, LayerPersister};
    if resources.is_empty() {
        return Ok(0);
    }
    let n = resources.len();
    let branch = format!("doc-{doc_id}");
    let mut b = crate::layer::LayerBuilder::new("doc-draws", Some(parent));
    for r in resources {
        b.add_resource(r).map_err(|e| format!("add draw: {e:?}"))?;
    }
    let layer = std::sync::Arc::new(b.build(crate::layer::LayerStorage::with_persistent(
        std::sync::Arc::clone(backend),
    )));
    let info = BackendPersister::new(Some(std::sync::Arc::clone(backend)))
        .persist(&branch, &layer)
        .map_err(|e| format!("persist draws to {branch}: {e:?}"))?;
    if !info.branch_advanced {
        // Not an error to re-run: draw IRIs are content-addressed on (seam, key), so an identical
        // re-record produces the identical layer and the anchored-commit cache declines to move the
        // branch. Say so rather than failing — idempotency is the intended behaviour.
        return Ok(0);
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draws() -> Vec<KeyedDraw> {
        vec![
            KeyedDraw {
                key: "sentence-a\u{1d}ctx".to_string(),
                record: serde_json::json!({"sentence": "a", "order": [1, 0]}),
            },
            KeyedDraw {
                key: "sentence-b\u{1d}ctx".to_string(),
                record: serde_json::json!({"sentence": "b", "order": [0]}),
            },
        ]
    }

    #[test]
    fn a_draw_carries_its_seam_key_and_verbatim_record() {
        let rs = draw_resources(
            "urn:eigenius:test:doc",
            DrawSeam::SenseRank,
            &draws(),
            Some("claude-opus-5"),
            "2026-08-19T00:00:00Z",
        )
        .expect("emits");
        assert_eq!(rs.len(), 2);
        let seam_prop = Iri::parse("urn:eigenius:encoding:draw_seam").unwrap();
        let rec_prop = Iri::parse("urn:eigenius:encoding:draw_record").unwrap();
        assert!(matches!(
            rs[0].get(&seam_prop),
            Some(Value::String(s)) if s.as_str().ends_with("seam_sense_rank")
        ));
        let Some(Value::String(text)) = rs[0].get(&rec_prop) else {
            panic!("record is a string")
        };
        let back: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(back["sentence"], "a", "the record round-trips verbatim");
    }

    /// The IRI is content-addressed on (seam, key): the same question recorded twice is the same
    /// resource, so a re-run hits the anchored-commit cache instead of growing the branch.
    #[test]
    fn the_same_question_lands_at_the_same_iri_and_a_different_seam_does_not() {
        let a = draw_iri("urn:x", DrawSeam::SenseRank, "q");
        let b = draw_iri("urn:x", DrawSeam::SenseRank, "q");
        let c = draw_iri("urn:x", DrawSeam::Anaphora, "q");
        let d = draw_iri("urn:x", DrawSeam::SenseRank, "q2");
        assert_eq!(a, b);
        assert_ne!(a, c, "the seam is part of the identity");
        assert_ne!(a, d, "the question is part of the identity");
    }

    #[test]
    fn seam_names_round_trip() {
        for s in DrawSeam::all() {
            assert_eq!(DrawSeam::from_local_name(s.local_name()), Some(s));
        }
        assert_eq!(DrawSeam::from_local_name("seam_nope"), None);
    }
}
