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

//! **A recorded draw survives the chain round-trip and replays with 0 misses** (D71 §9, slice 3).
//!
//! The claim slice 3 makes is that a formalization run can commit its proposer transcript to its
//! own `doc-<id>` branch and a later run can replay from the branch alone — no draw files, no LLM.
//! That rests on one property: `record → resource → layer → resource → record` must preserve the
//! REPLAY KEY exactly. A round-trip that loses or perturbs the key does not fail loudly; it fails as
//! a MISS, and a miss falls back to seed order, which is a different experiment wearing the same
//! numbers. So the assertion here is on hits and misses, not on the bytes.
//!
//! The draws also have to VALIDATE — `enc:ProposalDraw` requires its seam, key and record, and
//! `enc:draw_seam` is a closed enumeration — so the layer is validated rather than merely built.

use std::sync::Arc;

use eigenius_kernel::dcg::draw::{draw_resources, draws_from_layer, DrawSeam};
use eigenius_kernel::dcg::sense_ranker::{
    RecordingSenseRanker, ReplaySenseRanker, SenseCandidate, SenseRanker, WordSenses,
};
use eigenius_kernel::esl;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::validation::Validator;

fn json_layer(name: &str, parent: Option<Arc<Layer>>, sources: &[&str]) -> Arc<Layer> {
    let mut b = LayerBuilder::new(name, parent);
    for src in sources {
        for r in eigon_json::parse_document(src).expect("ontology parses") {
            b.add_resource(r).expect("ontology resource adds");
        }
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

fn esl_layer(name: &str, src: &str, parent: Arc<Layer>) -> Arc<Layer> {
    let resources = esl::compile_against_layer(src, &parent)
        .unwrap_or_else(|errs| panic!("{name} failed to compile: {errs:#?}"));
    let mut b = LayerBuilder::new(name, Some(parent));
    for r in &resources {
        b.add_resource(r.clone())
            .unwrap_or_else(|e| panic!("{name}: add_resource failed: {e:?}"));
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

/// The chain `encoding.esl` documents it loads after.
fn encoding_chain() -> Arc<Layer> {
    let core = json_layer(
        "core",
        None,
        &[include_str!("../../ontologies/core/core-ontology.json")],
    );
    let refl = json_layer(
        "reflection",
        Some(core),
        &[
            include_str!("../../ontologies/reflection/reflection-ontology.json"),
            include_str!("../../ontologies/eigentt/eigentt-type-fragment.json"),
            include_str!("../../ontologies/institution/institution-ontology.json"),
            include_str!("../../ontologies/ingest/ingest-ontology.json"),
        ],
    );
    let prov = esl_layer("prov", include_str!("../../ontologies/prov/prov.esl"), refl);
    let logic = esl_layer(
        "logic",
        include_str!("../../ontologies/logic/logic.esl"),
        prov,
    );
    let lexicon = esl_layer(
        "lexicon-schema",
        include_str!("../../ontologies/lexicon/lexicon-ontology.esl"),
        logic,
    );
    let reference = esl_layer(
        "reference",
        include_str!("../../ontologies/reference/reference.esl"),
        lexicon,
    );
    esl_layer(
        "encoding",
        include_str!("../../ontologies/encoding/encoding.esl"),
        reference,
    )
}

/// A ranker that reverses each word's candidate list — an answer that is never the seed order, so
/// "the replay actually answered" is distinguishable from "the replay fell through to the default".
struct ReverseRanker;
impl SenseRanker for ReverseRanker {
    fn rank(&self, _s: &str, _c: &str, words: &[WordSenses]) -> Option<Vec<Vec<usize>>> {
        Some(
            words
                .iter()
                .map(|w| (0..w.candidates.len()).rev().collect())
                .collect(),
        )
    }
}

fn cand(sense: &str) -> SenseCandidate {
    SenseCandidate {
        sense: sense.to_string(),
        sem: String::new(),
        gloss: String::new(),
    }
}

/// Two questions, each with a distinguishable answer.
fn ask(r: &dyn SenseRanker) -> Vec<Option<Vec<Vec<usize>>>> {
    let w1 = [WordSenses {
        surface: "Cancers",
        candidates: &[cand("n14247239"), cand("v02604760")],
    }];
    let w2 = [WordSenses {
        surface: "helicase",
        candidates: &[cand("C0018738"), cand("n05981230"), cand("n00001740")],
    }];
    vec![
        r.rank("Cancers exhibit defects.", "ctx one", &w1),
        r.rank("WRN has helicase activity.", "ctx two", &w2),
    ]
}

#[test]
fn a_recorded_draw_round_trips_through_the_chain_and_replays_with_zero_misses() {
    // 1. RECORD — the live arm.
    let recorder = RecordingSenseRanker::new(ReverseRanker);
    let live = ask(&recorder);
    assert_eq!(live[0], Some(vec![vec![1, 0]]));
    assert_eq!(live[1], Some(vec![vec![2, 1, 0]]));

    // 2. COMMIT — the draws become resources on a layer, and that layer VALIDATES.
    let draws = recorder.keyed_draws().expect("keyed draws");
    assert_eq!(draws.len(), 2, "two questions were asked and recorded");
    let resources = draw_resources(
        "urn:eigenius:test:doc",
        DrawSeam::SenseRank,
        &draws,
        Some("test-ranker"),
        "2026-08-19T00:00:00Z",
    )
    .expect("draw resources");

    let chain = encoding_chain();
    let mut b = LayerBuilder::new("doc-draws", Some(Arc::clone(&chain)));
    for r in resources {
        b.add_resource(r).expect("draw resource adds");
    }
    let branch = Arc::new(b.build(LayerStorage::in_memory()));
    let errors = Validator::new(Arc::clone(&branch)).validate();
    assert!(
        errors.is_empty(),
        "the draw layer validates with 0 errors, got {}:\n{errors:#?}",
        errors.len()
    );

    // 3. REPLAY FROM THE BRANCH ALONE — no file was written at any point above.
    let json = draws_from_layer(&branch, DrawSeam::SenseRank).expect("read draws back");
    let replay = ReplaySenseRanker::from_json(&json).expect("replay loads");
    let replayed = ask(&replay);

    assert_eq!(
        replay.misses(),
        0,
        "a miss means the key did not survive the round-trip; the run would silently fall back \
         to seed order and report itself as a reproduction"
    );
    assert_eq!(replay.hits(), 2);
    assert_eq!(
        replayed, live,
        "the chain-replayed answers are the recorded ones"
    );
}

/// The draw survives a CBOR encode/decode cycle.
///
/// **This is NOT the regression test for the shape bug, and saying so matters.** CBOR encoded
/// `Value::ResourceRef` as plain `Text`, so a decoded value was a `String` — but
/// `LayerBuilder::add_resource` RE-CANONICALISED it back to a ref against the ontology, which is
/// why this passed both before and after the fix. The persisted read path (`build_chain` over
/// stored bytes) never goes through the builder, so a draw read off a committed branch kept the
/// string shape, and `draws_from_layer` matching only the variant returned `[]` — a silent live
/// re-ask with no error anywhere. The variant is retired (D85 §6.2); the discipline it argued
/// for — read an IRI through an accessor — is what keeps this from recurring.
///
/// Reproducing that needs a real persisted store, so the witness is live rather than a unit test:
/// a second `eigenius formalize` on the same doc id reported `11 draw(s) recorded` before the fix
/// and `0 draw(s) recorded` after, with the four seams reading 7824 / 11977 / 2591 / 1536 bytes off
/// the branch (`2026-08-20`). What this test does cover is that the encode/decode cycle itself
/// preserves the draw, which is worth keeping.
#[test]
fn draws_survive_a_cbor_encode_decode_cycle() {
    let recorder = RecordingSenseRanker::new(ReverseRanker);
    let live = ask(&recorder);
    let resources = draw_resources(
        "urn:eigenius:test:doc",
        DrawSeam::SenseRank,
        &recorder.keyed_draws().expect("keyed draws"),
        None,
        "2026-08-19T00:00:00Z",
    )
    .expect("draw resources");

    let mut b = LayerBuilder::new("doc-draws", Some(encoding_chain()));
    for r in resources {
        let bytes = eigenius_kernel::ontology::eigon_cbor::serialize_resource(&r);
        let reloaded = eigenius_kernel::ontology::eigon_cbor::parse_resource(&bytes)
            .expect("draw round-trips through CBOR");
        b.add_resource(reloaded).expect("adds");
    }
    let branch = Arc::new(b.build(LayerStorage::in_memory()));

    let json = draws_from_layer(&branch, DrawSeam::SenseRank).expect("read back");
    assert_ne!(json, "[]");
    let replay = ReplaySenseRanker::from_json(&json).expect("replay loads");
    assert_eq!(ask(&replay), live);
    assert_eq!(replay.misses(), 0);
}

/// Seams do not bleed into each other: a chain carrying several runs' draws replays each seam's
/// set on its own. Without the filter, `from_json` would be handed another seam's record shape.
#[test]
fn reading_back_one_seam_ignores_the_others() {
    let recorder = RecordingSenseRanker::new(ReverseRanker);
    let _ = ask(&recorder);
    let rank_draws = recorder.keyed_draws().expect("keyed draws");

    let mut resources = draw_resources(
        "urn:eigenius:test:doc",
        DrawSeam::SenseRank,
        &rank_draws,
        None,
        "2026-08-19T00:00:00Z",
    )
    .expect("rank draws");
    // A foreign seam's draw, with a record shape `RankRecord` cannot parse.
    resources.extend(
        draw_resources(
            "urn:eigenius:test:doc",
            DrawSeam::DiscourseKind,
            &[eigenius_kernel::dcg::draw::KeyedDraw {
                key: "some sentence\u{1d}some gloss".to_string(),
                record: serde_json::json!({
                    "sentence": "s", "gloss": "g",
                    "kinds": ["urn:eigenius:encoding:Finding"]
                }),
            }],
            None,
            "2026-08-19T00:00:00Z",
        )
        .expect("kind draws"),
    );

    let chain = encoding_chain();
    let mut b = LayerBuilder::new("doc-draws", Some(chain));
    for r in resources {
        b.add_resource(r).expect("adds");
    }
    let branch = Arc::new(b.build(LayerStorage::in_memory()));

    let json = draws_from_layer(&branch, DrawSeam::SenseRank).expect("read back");
    let replay = ReplaySenseRanker::from_json(&json).expect("loads despite the foreign draw");
    let _ = ask(&replay);
    assert_eq!(replay.misses(), 0);
    assert_eq!(replay.hits(), 2);

    let kinds = draws_from_layer(&branch, DrawSeam::DiscourseKind).expect("read back");
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&kinds).expect("valid JSON");
    assert_eq!(parsed.len(), 1, "the kind seam has exactly its own draw");
}
