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

//! D43 §M9.3 — HNSW recall + latency benchmark against flat brute force.
//!
//! Synthetic vectors with controlled cluster structure: 50 cluster
//! centres uniformly distributed in `[-1, 1]^d`, with N/50 points
//! around each centre under a Gaussian-shaped perturbation. Query
//! vectors are drawn from the same distribution so the
//! ground-truth top-K is a well-defined neighbourhood, not a
//! random-noise floor.
//!
//! The bench compares the kernel's HNSW path
//! ([`HnswGraph::build`]/[`HnswGraph::search`]) against an inline
//! flat brute-force pass for `K = 10`, sweeping:
//!
//! - Dataset size N: 1 000, 10 000, 50 000.
//! - HNSW graph connectivity `m`: 16, 32, 48 (the v1 schema default
//!   is 16, declared in [`vec_defaults::HNSW_M`]; the
//!   sweep validates whether higher connectivity lifts the recall
//!   plateau observed at m = 16 with raised `ef`).
//! - HNSW search `ef`: 16, 64, 256.
//!
//! `ef_construction = 200` stays fixed throughout — varying it is
//! a separate axis the bench doesn't sweep.
//!
//! For each (N, m, ef) triple the bench reports:
//!
//! - Brute-force per-query latency (mean over 100 queries).
//! - HNSW per-query latency (mean over 100 queries).
//! - Mean recall@10 of HNSW against the brute-force top-10.
//!
//! Build time is reported per (N, m) since the graph is shared
//! across the three `ef` values at one `m`.
//!
//! Marked `#[ignore]` so `cargo test` doesn't run it by default —
//! it's a benchmark, not a correctness check. Run with:
//!
//! ```text
//! cargo test --release --test d43_hnsw_recall_bench -- --ignored --nocapture
//! ```
//!
//! Numbers from the benchmark feed the M9-pending "v1 operating
//! envelope" published in [d43-implementation-notes.md][notes].
//! Re-run when changing the HNSW graph builder, the distance
//! kernel, or the search loop.
//!
//! [notes]: ../../docs/notes/d43-implementation-notes.md

use std::time::Instant;

use eigenius_kernel::query::vector::distance::Metric;
use eigenius_kernel::query::vector::hnsw::{HnswBuildConfig, HnswGraph};

// ─── deterministic RNG ─────────────────────────────────────────────────

/// xorshift64* — deterministic, no external dependency, period 2^64−1.
/// Used as a fixed-seed source so the bench is reproducible across
/// runs; the only state the bench keeps is the seed.
#[derive(Clone)]
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform in `[-1, 1)`.
    fn next_unit(&mut self) -> f32 {
        let u = self.next_u64() >> 11; // top 53 bits → [0, 2^53)
        let f = (u as f64) * (2.0f64.powi(-53)); // [0, 1)
        (f as f32) * 2.0 - 1.0 // [-1, 1)
    }
    /// Standard-normal-ish via Box-Muller. Box-Muller is fine for
    /// the bench (no extreme-tail accuracy needed); we just want
    /// well-shaped clusters so flat top-K isn't dominated by
    /// boundary effects.
    fn next_gauss(&mut self) -> f32 {
        // Avoid `ln(0)` by clamping u1 away from 0.
        let mut u1 = (self.next_u64() >> 11) as f64 * 2.0f64.powi(-53);
        if u1 < 1e-12 {
            u1 = 1e-12;
        }
        let u2 = (self.next_u64() >> 11) as f64 * 2.0f64.powi(-53);
        let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
        z as f32
    }
}

// ─── corpus + query generation ─────────────────────────────────────────

const DIM: usize = 64;
const CLUSTERS: usize = 50;
const PERTURBATION: f32 = 0.15; // std-dev per coordinate around the centre

/// Sample the cluster centres separately so they can be shared
/// between corpus generation and query generation — queries are
/// drawn from the same distribution as the corpus so the
/// brute-force top-K is a meaningful neighbourhood rather than a
/// random-noise floor.
fn sample_centres(seed: u64) -> Vec<Vec<f32>> {
    let mut rng = Rng::new(seed);
    (0..CLUSTERS)
        .map(|_| (0..DIM).map(|_| rng.next_unit()).collect())
        .collect()
}

/// Synthesise `n` clustered vectors around the supplied centres.
/// Half the points cluster tightly (`PERTURBATION`); the other
/// half perturb more broadly (`PERTURBATION * 3`) to introduce
/// inter-cluster confusables so HNSW's recall isn't trivially 1.0
/// from clean separation alone. Returns a flat `n*dim` row-major
/// array.
fn build_corpus(centres: &[Vec<f32>], n: usize, seed: u64) -> Vec<f32> {
    let mut rng = Rng::new(seed);
    let mut data = vec![0.0_f32; n * DIM];
    for i in 0..n {
        let centre = &centres[i % CLUSTERS];
        let stride = if i < n / 2 {
            PERTURBATION
        } else {
            PERTURBATION * 3.0
        };
        let row = &mut data[i * DIM..(i + 1) * DIM];
        for (j, slot) in row.iter_mut().enumerate() {
            *slot = centre[j] + rng.next_gauss() * stride;
        }
    }
    data
}

/// Sample `count` queries from the *same* cluster distribution the
/// corpus was drawn from — each query is a small perturbation of a
/// randomly-chosen centre, so its true K-nearest neighbours in the
/// corpus are well-defined cluster members.
fn build_queries(centres: &[Vec<f32>], count: usize, seed: u64) -> Vec<Vec<f32>> {
    let mut rng = Rng::new(seed);
    (0..count)
        .map(|_| {
            let centre = &centres[(rng.next_u64() as usize) % CLUSTERS];
            (0..DIM)
                .map(|j| centre[j] + rng.next_gauss() * PERTURBATION)
                .collect()
        })
        .collect()
}

// ─── ground truth ──────────────────────────────────────────────────────

/// Brute-force top-K under cosine similarity. Returns `(node_id,
/// similarity)` pairs ordered by descending similarity. O(n * dim)
/// per query — the ground-truth baseline HNSW recall is measured
/// against.
fn flat_top_k(data: &[f32], dim: usize, query: &[f32], k: usize) -> Vec<usize> {
    let count = data.len() / dim;
    let mut scored: Vec<(usize, f32)> = (0..count)
        .map(|i| {
            let row = &data[i * dim..(i + 1) * dim];
            (i, Metric::Cosine.similarity(row, query))
        })
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.iter().take(k).map(|(i, _)| *i).collect()
}

fn recall_at_k(hnsw: &[(usize, f32)], flat: &[usize], k: usize) -> f32 {
    let hnsw_ids: std::collections::HashSet<usize> = hnsw.iter().take(k).map(|(i, _)| *i).collect();
    let hit = flat.iter().take(k).filter(|i| hnsw_ids.contains(i)).count();
    hit as f32 / k as f32
}

// ─── bench harness ─────────────────────────────────────────────────────

const QUERY_COUNT: usize = 100;
const K: usize = 10;

/// HNSW graph-connectivity values to sweep. `16` is the v1 schema
/// default ([`vec_defaults::HNSW_M`]); the sweep checks whether
/// raising it lifts the recall plateau observed when `ef` is the
/// only knob varied.
const M_VALUES: &[usize] = &[16, 32, 48];

/// HNSW search exploration depths to sweep per built graph. The
/// published HNSW expectation is ~95 % recall@k at `ef = k * 2` and
/// ~99 % at `ef = k * 8`; here `K = 10` so 16 / 64 / 256 cover the
/// rule-of-thumb range plus a generous over-fetch.
const EF_VALUES: &[usize] = &[16, 64, 256];

fn run_bench_at_size(n: usize) {
    eprintln!("\n── HNSW recall + latency, N={n}, dim={DIM}, K={K} ──");
    let t = Instant::now();
    // One set of cluster centres shared between corpus and queries
    // so the brute-force top-K is a meaningful cluster
    // neighbourhood, not a random-noise floor.
    let centres = sample_centres(0xCE7E_55EE_D000);
    let corpus = build_corpus(&centres, n, 0xA11C_EBEE_F000);
    eprintln!(
        "  corpus gen:                       {:.3}s",
        t.elapsed().as_secs_f64()
    );

    let t = Instant::now();
    let queries = build_queries(&centres, QUERY_COUNT, 0xB0B);
    eprintln!(
        "  query gen ({QUERY_COUNT} queries):           {:.3}s",
        t.elapsed().as_secs_f64()
    );

    // Flat baseline runs once — independent of HNSW parameters,
    // so it doesn't belong inside the m / ef loop.
    let t = Instant::now();
    let flat_results: Vec<Vec<usize>> = queries
        .iter()
        .map(|q| flat_top_k(&corpus, DIM, q, K))
        .collect();
    let flat_total = t.elapsed().as_secs_f64();
    eprintln!(
        "  flat brute-force / query (mean):  {:.3}ms",
        (flat_total / QUERY_COUNT as f64) * 1000.0
    );

    // Sweep m on the outer loop — one HNSW build per m, then all
    // three ef values are measured against the same graph.
    for &m in M_VALUES {
        let config = HnswBuildConfig {
            m,
            ef_construction: 200,
            max_elements: n.max(16),
        };
        let t = Instant::now();
        let graph = HnswGraph::build(&corpus, DIM, Metric::Cosine, config);
        eprintln!(
            "\n  m={m:>2}  build (ef_c=200):              {:.3}s",
            t.elapsed().as_secs_f64()
        );

        for &ef in EF_VALUES {
            let t = Instant::now();
            let hnsw_results: Vec<Vec<(usize, f32)>> =
                queries.iter().map(|q| graph.search(q, K, ef)).collect();
            let hnsw_total = t.elapsed().as_secs_f64();
            let recall: f32 = hnsw_results
                .iter()
                .zip(flat_results.iter())
                .map(|(h, f)| recall_at_k(h, f, K))
                .sum::<f32>()
                / QUERY_COUNT as f32;
            eprintln!(
                "    ef={ef:>3} / query (mean):      {:>6.3}ms   recall@{K} = {:.3}",
                (hnsw_total / QUERY_COUNT as f64) * 1000.0,
                recall
            );
        }
    }
}

// ─── tests (bench, gated `#[ignore]`) ──────────────────────────────────

#[test]
#[ignore = "benchmark; run with `cargo test --release ... -- --ignored --nocapture`"]
fn bench_hnsw_recall_1k() {
    run_bench_at_size(1_000);
}

#[test]
#[ignore = "benchmark; run with `cargo test --release ... -- --ignored --nocapture`"]
fn bench_hnsw_recall_10k() {
    run_bench_at_size(10_000);
}

/// At 50 000 the in-tree HNSW builder dominates wall-clock
/// (~4 min) — separate `#[ignore]` so it can be selectively
/// run with
/// `cargo test ... -- --ignored bench_hnsw_recall_50k --nocapture`
/// or skipped with `--skip 50k`.
#[test]
#[ignore = "slow benchmark (~4 min build); skip with --skip 50k"]
fn bench_hnsw_recall_50k() {
    run_bench_at_size(50_000);
}
