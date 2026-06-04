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

//! D43 §M9.4 — performance benchmark against the real GO corpus.
//!
//! Measures the operating envelope of the D43 pipeline at
//! life-science scale: per-phase wall-clock for the convert / load /
//! index path against 52 000 Resources into a RocksDB-backed kernel
//! layer, plus cold and warm BM25 query latency for `~` operator
//! evaluation.
//!
//! Skipped (with an `eprintln!` notice) when the data file isn't
//! present *or* when it's a git-lfs pointer — same shape as
//! [`d43_go_subset_integration`]. Run with:
//!
//! ```text
//! cargo test -p eigenius-obograph --test d43_perf_bench --release \
//!     -- --ignored --nocapture
//! ```
//!
//! Output captured in [d43-implementation-notes.md][notes] as the
//! v1 operating envelope. Re-run when the converter, the text
//! indexer, the BM25 dispatcher, or the RocksDB schema change.
//!
//! [notes]: ../../docs/notes/d43-implementation-notes.md

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

const GO_BASIC_RELATIVE_PATH: &str = "../../data/GO/go-basic.json";

/// Read + parse the GO obojson dump or skip gracefully. Same logic
/// as the integration test in `d43_go_subset_integration.rs`;
/// restated here so the bench is self-contained.
fn load_go_or_skip() -> Option<GraphDocument> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let path = PathBuf::from(manifest_dir).join(GO_BASIC_RELATIVE_PATH);
    let json = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("skipping perf bench — `{}` not present.", path.display());
            return None;
        }
    };
    if json.starts_with("version https://git-lfs.") {
        eprintln!(
            "skipping perf bench — `{}` is a git-lfs pointer. Run `git lfs pull`.",
            path.display()
        );
        return None;
    }
    Some(serde_json::from_str(&json).expect("go-basic.json parses"))
}

/// Approximate process RSS via `/proc/self/status` on Linux.
/// Returns kilobytes (Linux convention). `None` on platforms where
/// the file isn't present (macOS, Windows) — the bench prints "n/a"
/// in that case rather than fabricate a number.
fn rss_kb() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let mut parts = rest.split_whitespace();
            return parts.next()?.parse().ok();
        }
    }
    None
}

fn fmt_rss(kb: Option<u64>) -> String {
    match kb {
        Some(k) if k >= 1024 * 1024 => format!("{:.2} GiB", k as f64 / (1024.0 * 1024.0)),
        Some(k) if k >= 1024 => format!("{:.1} MiB", k as f64 / 1024.0),
        Some(k) => format!("{k} KiB"),
        None => "n/a".to_string(),
    }
}

struct PersistentLayer {
    layer: Arc<Layer>,
    _backend: Arc<dyn PersistentBackend>,
    _tmp: TempDir,
}

fn build_go_layer_with_timing(report: &ConvertReport) -> PersistentLayer {
    let tmp = TempDir::new().expect("tempdir");

    let t = Instant::now();
    let store = Arc::new(RocksStore::open(tmp.path()).expect("open RocksStore"));
    let backend: Arc<dyn PersistentBackend> = store.clone();
    let ctx = bootstrap_persistent(Arc::clone(&backend)).expect("bootstrap_persistent");
    eprintln!(
        "  rocksdb seed bootstrap (11 layers):    {:.2}s",
        t.elapsed().as_secs_f64()
    );

    let head = Arc::clone(ctx.head());
    let storage: LayerStorage = head.storage().clone();
    let mut b = LayerBuilder::new("go-corpus", Some(head));

    let mut ti = Resource::new(Iri::parse("urn:obo:converter:go-perf:ti_desc").unwrap());
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

    let t = Instant::now();
    for r in &report.resources {
        b.add_resource(r.clone()).unwrap();
    }
    eprintln!(
        "  add_resource × {}:                {:.2}s",
        report.resources.len(),
        t.elapsed().as_secs_f64()
    );

    let t = Instant::now();
    let layer = Arc::new(b.build(storage));
    eprintln!(
        "  LayerBuilder::build (bloom + triple + text index):  {:.2}s",
        t.elapsed().as_secs_f64()
    );

    PersistentLayer {
        layer,
        _backend: backend,
        _tmp: tmp,
    }
}

/// Five BM25-flavoured queries drawn from the nucleus class's
/// definition vocabulary so the bench exercises both common
/// (high-`tf`) and uncommon (low-`tf`) terms.
const QUERIES: &[&str] = &[
    "nucleus chromosome housing",
    "membrane bounded organelle",
    "cell division mitosis",
    "ribosome translation initiation",
    "transcription factor regulation",
];

#[test]
#[ignore = "benchmark; run with `cargo test --release ... -- --ignored --nocapture`"]
fn bench_go_perf_envelope() {
    let doc = match load_go_or_skip() {
        Some(d) => d,
        None => return,
    };

    eprintln!("\n── D43 perf envelope: GO basic + RocksDB ──");
    let baseline_rss = rss_kb();
    eprintln!(
        "  baseline RSS:                          {}",
        fmt_rss(baseline_rss)
    );

    let t = Instant::now();
    let report = convert_document(&doc);
    let convert_secs = t.elapsed().as_secs_f64();
    eprintln!(
        "  obograph convert ({} Resources):    {:.2}s",
        report.resources.len(),
        convert_secs
    );
    let convert_rss = rss_kb();
    eprintln!(
        "  RSS after convert:                     {}",
        fmt_rss(convert_rss)
    );

    let persistent = build_go_layer_with_timing(&report);
    let loaded_rss = rss_kb();
    eprintln!(
        "  RSS after load + index:                {}",
        fmt_rss(loaded_rss)
    );

    eprintln!("  ─── BM25 ~ query latency ───");
    eprintln!(
        "  Each query: `MATCH ?c {{ description: ?desc }} WHERE ?desc ~ \"<text>\" \
         TOP 10`"
    );

    // Cold pass — each query runs against a freshly cold block
    // cache (in practice the kernel has been doing builds + scans
    // so cache isn't strictly cold, but it's the earliest
    // measurement we can take).
    eprintln!("  Cold pass (first run per query):");
    for q in QUERIES {
        let query = format!(
            r#"
            MATCH ?c {{ "urn:eigenius:core:description": ?desc }}
            WHERE ?desc ~ "{q}"
            RETURN [] {{ c: ?c }}
            TOP 10
            "#
        );
        let t = Instant::now();
        let _ =
            execute_with(&query, &persistent.layer, FiberRuntime::default()).expect("query runs");
        eprintln!("    {q:<40}  {:>6.0}ms", t.elapsed().as_secs_f64() * 1000.0);
    }

    // Warm pass — same queries again, block cache populated by the
    // cold pass. Gives the kernel's steady-state per-query cost.
    eprintln!("  Warm pass (cache primed):");
    for q in QUERIES {
        let query = format!(
            r#"
            MATCH ?c {{ "urn:eigenius:core:description": ?desc }}
            WHERE ?desc ~ "{q}"
            RETURN [] {{ c: ?c }}
            TOP 10
            "#
        );
        let t = Instant::now();
        let _ =
            execute_with(&query, &persistent.layer, FiberRuntime::default()).expect("query runs");
        eprintln!("    {q:<40}  {:>6.0}ms", t.elapsed().as_secs_f64() * 1000.0);
    }

    let final_rss = rss_kb();
    eprintln!(
        "  RSS at end of bench:                   {}",
        fmt_rss(final_rss)
    );
    if let (Some(base), Some(end)) = (baseline_rss, final_rss) {
        eprintln!(
            "  net RSS delta (load + index + queries): {}",
            fmt_rss(Some(end.saturating_sub(base)))
        );
    }
}
