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

//! D43 §M9.2 — end-to-end life-science integration test against real
//! Gene Ontology data with a **RocksDB-backed** kernel layer chain.
//!
//! Reads `data/GO/go-basic.json` from the repo root at test time,
//! runs it through the obograph converter, loads the result into a
//! persistent kernel layer (RocksDB temp dir) alongside a
//! `core:TextIndex` on `core:description`, and runs a similarity
//! (`~`) query for "nucleus" to verify the expected GO terms
//! surface. Skipped (with an `eprintln!` notice) when the data file
//! isn't present so CI without the dataset still passes.
//!
//! **Backend choice.** The kernel chain runs against
//! [`bootstrap_persistent`] with a fresh
//! [`eigenius_storage_rocksdb::RocksStore`] in a [`tempfile::TempDir`]
//! so the test exercises the same serialisation / CF-layout / write-
//! batch path production deployments use. The trade-off vs. an
//! in-memory bootstrap: a few-x load-time overhead for the
//! 52k-Resource ingest, but query-time stays close to RAM (block
//! cache warms during the build). Per-test timings are printed to
//! stderr (visible with `--nocapture`) so the test doubles as a
//! coarse performance check.
//!
//! Per-test invariants:
//!
//! - [`go_subset_converts_clean`] — convert real GO; assert no soft
//!   errors and the expected CLASS / PROPERTY counts. Pure
//!   in-memory conversion; no RocksDB involvement.
//! - [`go_subset_loads_into_kernel_layer`] — load every converted
//!   Resource into a fresh RocksDB-backed layer alongside a
//!   TextIndex declaration; assert the layer builds without panic
//!   and resolves `urn:obo:GO:0005634` (the nucleus class) back to
//!   a Resource with the expected `short_name`.
//! - [`go_subset_similarity_query_surfaces_nucleus`] — declare a
//!   TextIndex on `core:description`, run a `~` query for keywords
//!   from the nucleus definition, assert GO_0005634 appears in the
//!   top results. Exercises the full RocksDB read path through the
//!   text-search BM25 dispatcher.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use eigenius_kernel::bootstrap::bootstrap_persistent;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::query::evaluate::FiberRuntime;
use eigenius_kernel::query::execute_with;
use eigenius_kernel::storage::PersistentBackend;
use eigenius_obograph::{convert_document, ConvertReport, GraphDocument};
use eigenius_storage_rocksdb::RocksStore;
use tempfile::TempDir;

/// Path to the GO `obojson` dump, relative to the obograph crate's
/// `Cargo.toml`. Resolved at test time; missing-file triggers a
/// graceful skip so CI without the dataset still passes.
const GO_BASIC_RELATIVE_PATH: &str = "../../data/GO/go-basic.json";

/// Read + parse the GO obojson dump. Returns `None` (with an
/// `eprintln!` notice) when the file isn't present; the calling
/// `#[test]` then early-returns. The path is resolved relative to
/// the obograph crate manifest so the test runs from `cargo test`'s
/// usual working directory.
fn load_go_or_skip() -> Option<GraphDocument> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let path = PathBuf::from(manifest_dir).join(GO_BASIC_RELATIVE_PATH);
    let json = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => {
            eprintln!(
                "skipping GO integration test — `{}` not present. \
                 Place the GO obojson dump at that path to enable.",
                path.display()
            );
            return None;
        }
    };
    // Detect a git-lfs pointer file (the real GO dump is ~68 MB; a
    // pointer is ≤200 bytes starting with `version https://git-lfs.…`).
    // Treat it the same as "file missing" so contributors who haven't
    // run `git lfs pull` get a friendly skip instead of a JSON parse
    // panic.
    if json.starts_with("version https://git-lfs.") {
        eprintln!(
            "skipping GO integration test — `{}` is a git-lfs pointer, \
             not the actual data. Run `git lfs pull` to fetch the real file.",
            path.display()
        );
        return None;
    }
    Some(serde_json::from_str(&json).expect("go-basic.json parses"))
}

/// Convert the parsed GO document into Eigon Resources. Verifies
/// the converter produced a usable report before the per-test logic
/// dives into kernel-side concerns.
fn convert_and_assert_basics(doc: &GraphDocument) -> ConvertReport {
    let report = convert_document(doc);
    assert!(
        report.errors.is_empty(),
        "converter soft errors on real GO: {:?}",
        report.errors
    );
    let class_count = report.counts_by_type.get("CLASS").copied().unwrap_or(0);
    assert!(
        class_count >= 50_000,
        "expected ≥50k CLASS nodes in GO; got {class_count}. counts_by_type: {:?}",
        report.counts_by_type
    );
    report
}

/// RocksDB-backed kernel chain + the temp dir owning the database
/// directory. The `TempDir` must outlive the `Layer` (otherwise the
/// database files vanish mid-test); the caller takes ownership of
/// both and lets the harness drop them together.
struct PersistentLayer {
    layer: Arc<Layer>,
    // RAII guards. Order doesn't matter for correctness here, but
    // the layer holds Arc references into the storage so it
    // structurally outlives them via Arc counting; the TempDir is
    // explicit and the only one whose Drop touches disk.
    _backend: Arc<dyn PersistentBackend>,
    _tmp: TempDir,
}

/// Build a fresh **RocksDB-backed** kernel layer chain rooted at
/// the persistent seed bootstrap, plus a child `go-corpus` layer
/// carrying (a) a `core:TextIndex` Resource targeting
/// `core:description` and (b) every Resource from `report`.
///
/// Prints per-phase timing to stderr — visible with
/// `cargo test -- --nocapture` — so the test doubles as a coarse
/// performance check of the RocksDB ingest + indexing path.
fn build_go_layer(report: &ConvertReport) -> PersistentLayer {
    let tmp = TempDir::new().expect("create rocks tempdir");
    eprintln!("rocksdb path: {}", tmp.path().display());

    let t_open = Instant::now();
    let store = Arc::new(RocksStore::open(tmp.path()).expect("open RocksStore"));
    let backend: Arc<dyn PersistentBackend> = store.clone();
    let ctx = bootstrap_persistent(Arc::clone(&backend)).expect("bootstrap_persistent");
    eprintln!(
        "rocksdb seed bootstrap: {:.2}s",
        t_open.elapsed().as_secs_f64()
    );

    let head = Arc::clone(ctx.head());
    // The `Layer::storage()` handle is the persistent
    // `LayerStorage` plumbed through bootstrap — its `text_index`,
    // `triple_index`, etc. all dispatch to the same RocksStore
    // CFs. We pass it to `LayerBuilder::build` so the new
    // `go-corpus` layer commits to disk through the same path.
    let storage: LayerStorage = head.storage().clone();
    let mut b = LayerBuilder::new("go-corpus", Some(head));

    // TextIndex on `core:description`. Auto-populates at build time
    // (see `query::text::indexing::populate_text_indexes`).
    let mut ti = Resource::new(Iri::parse("urn:obo:converter:go-test:ti_desc").unwrap());
    ti.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::ResourceRef(
            Iri::parse(wk::TEXT_INDEX_CLASS).unwrap(),
        )]),
    );
    ti.set(
        Iri::parse(wk::TARGET_PROPERTY).unwrap(),
        Value::ResourceRef(Iri::parse("urn:eigenius:core:description").unwrap()),
    );
    ti.set(
        Iri::parse(wk::TEXT_ANALYZER).unwrap(),
        Value::String("en-stem-v1".into()),
    );
    b.add_resource(ti).unwrap();

    // Every converted GO Resource — adding ~52k Resources to one
    // builder. Cloning here is one extra pass per Resource; could
    // be avoided by consuming `report` but the integration test
    // values readability over the 50ms saved.
    let t_add = Instant::now();
    for r in &report.resources {
        b.add_resource(r.clone()).unwrap();
    }
    eprintln!(
        "add_resource × {}: {:.2}s",
        report.resources.len(),
        t_add.elapsed().as_secs_f64()
    );

    let t_build = Instant::now();
    let layer = Arc::new(b.build(storage));
    // D65 index lifecycle: derived indexes (triple/text/value) are now
    // materialised at the **persist** step, not eagerly at build. Persist the
    // go-corpus layer so its `core:description` TextIndex is populated in the
    // backend before we query it (mirrors a real commit).
    backend.store_layer(&layer).expect("store go-corpus layer");
    eprintln!(
        "LayerBuilder::build + persist (bloom + triple + text index): {:.2}s",
        t_build.elapsed().as_secs_f64()
    );

    PersistentLayer {
        layer,
        _backend: backend,
        _tmp: tmp,
    }
}

/// Extract the per-row subject IRIs from a wrapped query result.
/// Mirrors the helper used by the similarity end-to-end tests in
/// `kernel/src/query/evaluate/similarity.rs`; restated here so this
/// integration test is self-contained.
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
        .expect("result set Resource");
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

// ─── Tests ─────────────────────────────────────────────────────────────

#[test]
fn go_subset_converts_clean() {
    let doc = match load_go_or_skip() {
        Some(d) => d,
        None => return,
    };
    let report = convert_and_assert_basics(&doc);

    // Spot-check: nucleus class round-trips its description.
    let nucleus = report
        .resources
        .iter()
        .find(|r| {
            r.id()
                .map(|i| i.as_str() == "urn:obo:GO:0005634")
                .unwrap_or(false)
        })
        .expect("nucleus Resource emitted");
    match nucleus.get(&Iri::parse("urn:eigenius:core:description").unwrap()) {
        Some(Value::String(s)) => assert!(
            s.contains("membrane-bounded organelle"),
            "nucleus description must round-trip; got `{s}`"
        ),
        other => panic!("expected description String, got {other:?}"),
    }
    // Synonym round-trip — GO's `hasExactSynonym: "cell nucleus"`.
    match nucleus.get(&Iri::parse("urn:obo:has_exact_synonym").unwrap()) {
        Some(Value::Array(arr)) => {
            let strings: Vec<&str> = arr
                .iter()
                .filter_map(|v| match v {
                    Value::String(s) => Some(s.as_str()),
                    _ => None,
                })
                .collect();
            assert!(
                strings.contains(&"cell nucleus"),
                "expected 'cell nucleus' synonym; got {strings:?}"
            );
        }
        other => panic!("expected exact-synonym Array, got {other:?}"),
    }
}

#[test]
fn go_subset_loads_into_kernel_layer() {
    let doc = match load_go_or_skip() {
        Some(d) => d,
        None => return,
    };
    let report = convert_and_assert_basics(&doc);
    let persistent = build_go_layer(&report);

    // The layer chain resolves the nucleus Class back to its
    // Resource — full round-trip from OBO-JSON through the
    // converter, into the kernel's resource store (RocksDB), out via
    // chain-walking `resolve`.
    let nucleus_iri = Iri::parse("urn:obo:GO:0005634").unwrap();
    let resolved = persistent
        .layer
        .resolve(&nucleus_iri)
        .expect("nucleus Class resolves from kernel layer");
    match resolved.get(&Iri::parse(wk::SHORT_NAME).unwrap()) {
        Some(Value::String(s)) => assert_eq!(s, "nucleus"),
        other => panic!("expected short_name 'nucleus', got {other:?}"),
    }
}

/// End-to-end D43 similarity query against the loaded GO corpus.
/// The query keywords come straight from the nucleus class's
/// definition ("membrane-bounded organelle... chromosomes are
/// housed"). BM25 over the populated TextIndex should rank
/// GO_0005634 in the top results.
#[test]
fn go_subset_similarity_query_surfaces_nucleus() {
    let doc = match load_go_or_skip() {
        Some(d) => d,
        None => return,
    };
    let report = convert_and_assert_basics(&doc);
    let persistent = build_go_layer(&report);

    let t_query = Instant::now();
    let rows = execute_with(
        r#"
        MATCH ?c { "urn:eigenius:core:description": ?desc }
        WHERE ?desc ~ "membrane bounded organelle chromosomes"
        RETURN [] { c: ?c }
        TOP 10
        "#,
        &persistent.layer,
        FiberRuntime::default(),
    )
    .expect("query should succeed");
    eprintln!(
        "BM25 ~ query against 52k indexed docs: {:.3}s",
        t_query.elapsed().as_secs_f64()
    );
    let matched = matched_subject_iris(&rows, "c");
    assert!(
        !matched.is_empty(),
        "expected at least one match for nucleus-related query"
    );
    assert!(
        matched.iter().any(|s| s == "urn:obo:GO:0005634"),
        "expected GO_0005634 (nucleus) in TOP 10; got {matched:?}"
    );
}
