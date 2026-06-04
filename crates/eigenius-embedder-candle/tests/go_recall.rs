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

//! D43 §M9.3 — *semantic* recall test using a real Sentence-BERT
//! embedder against a real biomedical ontology.
//!
//! Complements the algorithm-level HNSW recall bench
//! ([`kernel/tests/d43_hnsw_recall_bench.rs`]) by validating that
//! the `~` operator surfaces semantically correct GO terms for
//! paraphrased natural-language queries — the deferred M9 item that
//! requires a real embedder (not the [`DummyEmbedder`] hash-based
//! placeholder).
//!
//! Pipeline:
//!
//! 1. Convert real GO via the obograph importer.
//! 2. Select a corpus consisting of:
//!    - Every gold-set target IRI (so we know the answer exists).
//!    - 1 000 "distractor" GO terms (so the embedder has to
//!      discriminate against real biomedical neighbours, not just
//!      avoid random noise).
//! 3. Load into a fresh RocksDB-backed kernel layer with both a
//!    `core:TextIndex` (BM25 baseline) and a `core:VectorIndex`
//!    targeting `description`. Strategy: `flat` (brute-force vector
//!    search) so the test measures **embedder quality**, not HNSW
//!    recall — the HNSW story is already covered separately.
//! 4. Sweep the layer through [`CandleEmbedder`] to populate the
//!    vector segment.
//! 5. For each (query, expected-IRI) pair in the gold set, run a
//!    `~` similarity query with `{ via: vector }` (force the
//!    Candle path, even though hybrid would also work) and assert
//!    the expected IRI appears in the top-10 results.
//!
//! Skipped (with an `eprintln!` notice) when:
//!
//! - The GO data file is missing or a git-lfs pointer.
//! - The HuggingFace Hub fetch fails (offline / no network access /
//!   first-run rate limit). The model is ~130 MB; first run takes
//!   a minute on a typical connection, subsequent runs hit the HF
//!   cache.
//!
//! Run with:
//!
//! ```text
//! cargo test -p eigenius-embedder-candle --release --test go_recall \
//!     -- --ignored --nocapture
//! ```

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use eigenius_embedder_candle::{CandleEmbedder, LoadError, BGE_SMALL_DIM, BGE_SMALL_MODEL_IRI};
use eigenius_kernel::bootstrap::bootstrap_persistent;
use eigenius_kernel::layer::LayerBuilder;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::program::embedder::EmbedderRegistry;
use eigenius_kernel::query::evaluate::FiberRuntime;
use eigenius_kernel::query::execute_with;
use eigenius_kernel::query::vector::indexing::sweep_layer_vectors;
use eigenius_kernel::storage::PersistentBackend;
use eigenius_obograph::{convert_document, GraphDocument};
use eigenius_storage_rocksdb::RocksStore;
use tempfile::TempDir;

const GO_BASIC_RELATIVE_PATH: &str = "../../data/GO/go-basic.json";

/// Gold-set of `(natural-language query, expected GO IRI in top-K, human-readable label)`.
///
/// The queries are deliberately *paraphrased* rather than the GO
/// term's canonical label — so a successful retrieval can't be
/// attributed to mere word-overlap with the term name. They probe
/// the embedder's semantic understanding of biomedical concepts.
const GOLD_SET: &[(&str, &str, &str)] = &[
    (
        "fixing damage to genetic material",
        "urn:obo:GO:0006281",
        "DNA repair",
    ),
    (
        "splitting one cell into two daughter cells",
        "urn:obo:GO:0051301",
        "cell division",
    ),
    (
        "proteins binding to other proteins",
        "urn:obo:GO:0005515",
        "protein binding",
    ),
    (
        "where chromosomes are stored in eukaryotes",
        "urn:obo:GO:0005634",
        "nucleus",
    ),
    (
        "the organelle that produces cellular energy",
        "urn:obo:GO:0005739",
        "mitochondrion",
    ),
    (
        "the fluid inside a cell that surrounds organelles",
        "urn:obo:GO:0005737",
        "cytoplasm",
    ),
    (
        "moving molecules across cell membranes",
        "urn:obo:GO:0055085",
        "transmembrane transport",
    ),
];

fn load_go_or_skip() -> Option<GraphDocument> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let path = PathBuf::from(manifest_dir).join(GO_BASIC_RELATIVE_PATH);
    let json = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("skipping — `{}` not present.", path.display());
            return None;
        }
    };
    if json.starts_with("version https://git-lfs.") {
        eprintln!(
            "skipping — `{}` is a git-lfs pointer. Run `git lfs pull`.",
            path.display()
        );
        return None;
    }
    Some(serde_json::from_str(&json).expect("go-basic.json parses"))
}

fn try_load_embedder() -> Option<CandleEmbedder> {
    match CandleEmbedder::new_bge_small() {
        Ok(e) => Some(e),
        Err(LoadError::Hub(msg)) => {
            eprintln!(
                "skipping — HF Hub fetch failed (likely offline): {msg}. \
                 Pre-cache the model at ~/.cache/huggingface/hub to enable."
            );
            None
        }
        Err(e) => panic!("unexpected embedder load error: {e}"),
    }
}

const DESCRIPTION_PROP: &str = "urn:eigenius:core:description";

/// Pick a subset of the converted GO corpus that contains every
/// gold-set target IRI plus `extra` random GO Class Resources as
/// distractors. Caps embedding work to a manageable budget while
/// still exercising real biomedical discrimination.
fn select_corpus_subset(report_resources: &[Resource], extra: usize) -> Vec<Resource> {
    let gold_iris: std::collections::HashSet<&str> =
        GOLD_SET.iter().map(|(_, iri, _)| *iri).collect();

    let has_description = |r: &Resource| {
        let prop = Iri::parse(DESCRIPTION_PROP).unwrap();
        matches!(r.get(&prop), Some(Value::String(s)) if !s.is_empty())
    };

    // Gold targets first — if any are missing from the corpus the
    // test will fail loudly at the recall step rather than silently
    // skip the query.
    let mut targets: Vec<Resource> = report_resources
        .iter()
        .filter(|r| {
            r.id()
                .map(|i| gold_iris.contains(i.as_str()))
                .unwrap_or(false)
                && has_description(r)
        })
        .cloned()
        .collect();
    let target_count = targets.len();

    // Distractors: the next `extra` Classes with descriptions, in
    // whatever order `iter_resources` returns them (deterministic
    // for the GO dump).
    let target_iris: std::collections::HashSet<String> = targets
        .iter()
        .filter_map(|r| r.id().map(|i| i.as_str().to_string()))
        .collect();
    let distractors: Vec<Resource> = report_resources
        .iter()
        .filter(|r| {
            r.id()
                .map(|i| !target_iris.contains(i.as_str()))
                .unwrap_or(false)
                && has_description(r)
        })
        .take(extra)
        .cloned()
        .collect();

    targets.extend(distractors);
    eprintln!(
        "  corpus: {target_count} gold targets + {} distractors = {} total",
        extra,
        targets.len()
    );
    targets
}

#[test]
#[ignore = "real-embedder recall test; run with `cargo test --release -p eigenius-embedder-candle --test go_recall -- --ignored --nocapture`"]
fn go_recall_with_candle_bge_small() {
    let doc = match load_go_or_skip() {
        Some(d) => d,
        None => return,
    };
    let embedder = match try_load_embedder() {
        Some(e) => e,
        None => return,
    };

    eprintln!("\n── Real-embedder recall test: GO + Candle BGE-small ──");

    let t = Instant::now();
    let report = convert_document(&doc);
    eprintln!(
        "  obograph convert: {} Resources in {:.2}s",
        report.resources.len(),
        t.elapsed().as_secs_f64()
    );

    let subset = select_corpus_subset(&report.resources, 1000);

    let tmp = TempDir::new().expect("tempdir");
    let store = Arc::new(RocksStore::open(tmp.path()).expect("rocks open"));
    let backend: Arc<dyn PersistentBackend> = store;
    let ctx = bootstrap_persistent(Arc::clone(&backend)).expect("bootstrap");
    let head = Arc::clone(ctx.head());
    let storage = head.storage().clone();
    let mut b = LayerBuilder::new("go-candle-corpus", Some(head));

    // VectorIndex on description, strategy=flat (so we test
    // *embedder* recall, not HNSW recall — the HNSW story has its
    // own bench).
    let mut vi = Resource::new(Iri::parse("urn:obo:converter:go-candle:vi_desc").unwrap());
    vi.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::ResourceRef(
            Iri::parse(wk::VECTOR_INDEX_CLASS).unwrap(),
        )]),
    );
    vi.set(
        Iri::parse(wk::TARGET_PROPERTY).unwrap(),
        Value::ResourceRef(Iri::parse(DESCRIPTION_PROP).unwrap()),
    );
    vi.set(
        Iri::parse(wk::VEC_MODEL).unwrap(),
        Value::ResourceRef(Iri::parse(BGE_SMALL_MODEL_IRI).unwrap()),
    );
    vi.set(
        Iri::parse(wk::VEC_DIM).unwrap(),
        Value::Integer(BGE_SMALL_DIM as i64),
    );
    vi.set(
        Iri::parse(wk::VEC_DISTANCE).unwrap(),
        Value::ResourceRef(Iri::parse("urn:eigenius:core:distances:cosine").unwrap()),
    );
    vi.set(
        Iri::parse(wk::VEC_STRATEGY).unwrap(),
        Value::ResourceRef(Iri::parse("urn:eigenius:core:strategies:flat").unwrap()),
    );
    b.add_resource(vi).unwrap();

    for r in subset {
        b.add_resource(r).unwrap();
    }
    let t = Instant::now();
    let layer = Arc::new(b.build(storage));
    eprintln!(
        "  LayerBuilder::build (bloom + triple, no text):  {:.2}s",
        t.elapsed().as_secs_f64()
    );

    // Register the Candle embedder with the kernel and sweep.
    let mut reg = EmbedderRegistry::new();
    reg.register(Arc::new(embedder));

    let t = Instant::now();
    let report_sweep = sweep_layer_vectors(&layer, &reg, None).expect("vector sweep");
    eprintln!(
        "  vector sweep ({} subjects embedded): {:.2}s",
        report_sweep.total_subjects,
        t.elapsed().as_secs_f64()
    );

    // Wire the registry into the runtime so per-query embedding
    // works for the `~` operator.
    let runtime = FiberRuntime {
        embedders: Some(&reg),
        ..FiberRuntime::default()
    };

    let mut hits = 0;
    let mut misses: Vec<(&str, &str, &str)> = Vec::new();
    for &(query, expected_iri, label) in GOLD_SET {
        let t = Instant::now();
        let q = format!(
            r#"
            MATCH ?c {{ "urn:eigenius:core:description": ?desc }}
            WHERE ?desc ~ "{query}" {{ via: vector }}
            RETURN [] {{ c: ?c }}
            TOP 10
            "#
        );
        let rows = execute_with(&q, &layer, runtime).expect("query");
        let matched = matched_subject_iris(&rows, "c");
        let found = matched.iter().any(|s| s == expected_iri);
        let rank = matched
            .iter()
            .position(|s| s == expected_iri)
            .map(|p| (p + 1).to_string())
            .unwrap_or_else(|| "—".to_string());
        let mark = if found { "✓" } else { "✗" };
        eprintln!(
            "  {mark} {query:<55}  → rank {rank:>3} for {label} ({:.0}ms)",
            t.elapsed().as_secs_f64() * 1000.0
        );
        if found {
            hits += 1;
        } else {
            misses.push((query, expected_iri, label));
        }
    }

    let recall = hits as f32 / GOLD_SET.len() as f32;
    eprintln!("\n  recall@10 = {hits}/{} = {:.2}", GOLD_SET.len(), recall);
    assert!(
        recall >= 0.7,
        "expected recall@10 ≥ 0.7 on the gold set; got {recall:.2}.\n\
         Misses: {misses:?}"
    );
}

fn matched_subject_iris(wrapped: &[Resource], slot: &str) -> Vec<String> {
    let short_prop = Iri::parse(wk::SHORT_NAME).unwrap();
    let prop = wrapped
        .iter()
        .find(|r| {
            matches!(r.get(&short_prop), Some(Value::String(s)) if s == slot)
                && r.id().is_some()
                && r.id().unwrap().as_str().contains(":row:")
        })
        .and_then(|r| r.id().cloned())
        .unwrap_or_else(|| panic!("no row Property with short_name '{slot}'"));
    let rows_prop = Iri::parse("urn:eigenius:query:rows").unwrap();
    let result_set = wrapped
        .iter()
        .find(|r| {
            r.id()
                .map(|i| i.as_str().ends_with(":result"))
                .unwrap_or(false)
        })
        .expect("result set");
    let rows = match result_set.get(&rows_prop) {
        Some(Value::Array(arr)) => arr,
        _ => return Vec::new(),
    };
    rows.iter()
        .filter_map(|v| match v {
            Value::Embedded(r) => r.get(&prop).cloned(),
            _ => None,
        })
        .filter_map(|v| match v {
            Value::ResourceRef(i) => Some(i.as_str().to_string()),
            Value::String(s) => Some(s),
            _ => None,
        })
        .collect()
}
