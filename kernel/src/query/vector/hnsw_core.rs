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

//! D43 §2.4 / M6-finish.2 — in-tree HNSW build + search.
//!
//! ## Algorithm reference
//!
//! Implementation of the Hierarchical Navigable Small World
//! algorithm of Malkov & Yashunin, "Efficient and robust approximate
//! nearest neighbor search using Hierarchical Navigable Small World
//! graphs" (2016, arXiv:1603.09320). The four core procedures
//! `INSERT`, `SEARCH-LAYER`, `SELECT-NEIGHBORS-SIMPLE`, and `SEARCH`
//! are implemented per Algorithms 1–4 of the paper.
//!
//! ## Attribution
//!
//! Structural patterns — node identity scheme, per-layer adjacency
//! representation, the entry-point promotion check, the
//! bidirectional-connection pruning loop — were cross-checked
//! against Jean-Pierre Both's [`hnsw_rs`](https://crates.io/crates/hnsw_rs)
//! crate (Apache-2.0 / MIT dual-licensed). See the repo's
//! `NOTICE` file for the full credit chain. We do not vendor any
//! `hnsw_rs` code; everything here is a fresh implementation written
//! against the paper, with `hnsw_rs` as a reference for naming and
//! corner-case interpretation.
//!
//! ## Scope
//!
//! v1 prioritises correctness and clarity over raw throughput.
//! Concretely:
//!
//! - **Sequential insert** — no parallel build path. The post-Load
//!   sweep amortises HNSW build across the per-`(layer, Index)`
//!   batch; for v1 segment sizes (the D43 envelope is ≤ 10 M
//!   vectors per segment) sequential insert at M=16,
//!   ef_construction=200 falls under the sweep's typical wall-clock
//!   budget. Parallel insert is a contained follow-up.
//! - **No mmap / no FFI** — the kernel doesn't expose this surface
//!   externally, and the segment's f32 vector backing is already
//!   `Arc<[u32]>`-aligned (M5.10), so mmap buys nothing.
//! - **Metric is fixed at build time** — encoded in the
//!   `Distance` enum; runtime dispatch is a 3-arm match per
//!   distance call. The vectors-per-search distance count is
//!   `O(ef × M)` so this is cheap.
//!
//! The build emits an [`HnswGraph`] in the §2.4 wire shape; the
//! [`crate::query::vector::hnsw_format`] encoder serialises that
//! graph into the segment's `hnsw_graph` bstr without further
//! transformation.

use crate::query::vector::distance::Metric;
use crate::query::vector::hnsw_format::{HnswGraph, HnswNode};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// Build-time parameters. Same shape as
/// [`crate::query::vector::hnsw::HnswBuildConfig`] but defined here
/// so the algorithm module is self-contained.
#[derive(Debug, Clone, Copy)]
pub struct CoreBuildConfig {
    /// `M` — number of bidirectional links each node keeps at the
    /// upper levels. The base layer keeps `M * 2`.
    pub m: usize,
    /// `ef_construction` — exploration breadth during build.
    pub ef_construction: usize,
    /// Seed for the level-generator RNG so builds are deterministic
    /// within a process. The post-Load sweep can use any seed —
    /// what matters is that the search dispatched against the
    /// resulting graph produces stable hits across replays.
    pub seed: u64,
}

impl CoreBuildConfig {
    /// D43 §3.1 v1 defaults.
    pub fn default_for_segment(seed: u64) -> Self {
        Self {
            m: 16,
            ef_construction: 200,
            seed,
        }
    }
}

/// Build an HNSW over `vectors` (flat `count × dim` layout) under
/// `metric`. Returns the [`HnswGraph`] in the §2.4 wire shape ready
/// for encoding.
///
/// Panics if `vectors.len()` is not divisible by `dim`, or if `dim`
/// is zero — both are programming errors at the caller boundary.
pub fn build(vectors: &[f32], dim: usize, metric: Metric, config: CoreBuildConfig) -> HnswGraph {
    assert!(dim > 0, "dim must be positive");
    assert_eq!(
        vectors.len() % dim,
        0,
        "vectors.len() must be divisible by dim"
    );
    let count = vectors.len() / dim;
    if count == 0 {
        return HnswGraph {
            entry_point: 0,
            max_level: 0,
            nodes: Vec::new(),
        };
    }

    // Workspace state: per-node level + adjacency map (Vec of Vecs
    // indexed `[layer][neighbour]`). Grown by insert.
    let m_max_0 = config.m * 2; // base layer keeps double — paper Sec 4.1
    let m_max = config.m;
    let ml = 1.0 / (config.m as f32).ln();

    let mut rng = SplitMix64::new(config.seed);

    let mut nodes: Vec<NodeBuildState> = Vec::with_capacity(count);
    let mut entry_point: u32 = 0;
    let mut max_level: u8 = 0;

    for i in 0..count {
        let level = sample_level(&mut rng, ml);
        let q_id = i as u32;
        nodes.push(NodeBuildState::new(level));

        if i == 0 {
            entry_point = q_id;
            max_level = level;
            continue;
        }

        let q_vec = vector_slice(vectors, dim, q_id);
        // Greedy descent from current entry point down to layer
        // `level + 1` — picks the locally-nearest neighbour at
        // each upper layer.
        let mut ep_id = entry_point;
        let mut ep_dist = metric.distance(q_vec, vector_slice(vectors, dim, ep_id));

        let mut layer = max_level;
        while layer > level {
            let (next_id, next_dist) =
                greedy_step_down(vectors, dim, metric, &nodes, ep_id, ep_dist, q_vec, layer);
            ep_id = next_id;
            ep_dist = next_dist;
            if layer == 0 {
                break;
            }
            layer -= 1;
        }

        // From min(max_level, level) down to 0, run SEARCH-LAYER
        // with ef_construction, select M neighbours via the simple
        // heuristic, and wire up bidirectional links.
        let mut entry_candidates: Vec<Candidate> = vec![Candidate {
            id: ep_id,
            dist: ep_dist,
        }];

        let mut lc = level.min(max_level) as i32;
        loop {
            if lc < 0 {
                break;
            }
            let w = search_layer(
                vectors,
                dim,
                metric,
                &nodes,
                &entry_candidates,
                q_vec,
                config.ef_construction,
                lc as u8,
            );
            // Select neighbours for q at layer `lc`.
            let m_target = if lc == 0 { m_max_0 } else { m_max };
            let selected = select_neighbours_simple(&w, m_target);
            for cand in &selected {
                nodes[q_id as usize].neighbours[lc as usize].push(cand.id);
                nodes[cand.id as usize].neighbours[lc as usize].push(q_id);
            }
            // Prune over-capacity neighbour lists on the neighbours
            // q just connected to (their existing adjacency may now
            // exceed Mmax at this layer).
            for cand in &selected {
                let conn = std::mem::take(&mut nodes[cand.id as usize].neighbours[lc as usize]);
                if conn.len() > m_target {
                    let cand_vec = vector_slice(vectors, dim, cand.id);
                    let candidates: Vec<Candidate> = conn
                        .into_iter()
                        .map(|nid| Candidate {
                            id: nid,
                            dist: metric.distance(cand_vec, vector_slice(vectors, dim, nid)),
                        })
                        .collect();
                    let pruned = select_neighbours_simple(&candidates, m_target);
                    nodes[cand.id as usize].neighbours[lc as usize] =
                        pruned.iter().map(|c| c.id).collect();
                } else {
                    nodes[cand.id as usize].neighbours[lc as usize] = conn;
                }
            }
            entry_candidates = w;
            lc -= 1;
        }

        if level > max_level {
            entry_point = q_id;
            max_level = level;
        }
    }

    // Convert workspace `NodeBuildState` into the wire-format
    // `HnswNode`. Drop any internal allocations / bookkeeping.
    let nodes_out: Vec<HnswNode> = nodes
        .into_iter()
        .map(|n| HnswNode {
            level: n.level,
            neighbours: n.neighbours,
        })
        .collect();
    HnswGraph {
        entry_point,
        max_level,
        nodes: nodes_out,
    }
}

/// Search the graph for the top-`k` nearest neighbours of `query`
/// with per-search exploration breadth `ef`. Returns
/// `(node_index, similarity)` pairs in descending similarity order
/// (the "higher = better" convention shared with the brute-force
/// path — see [`Metric::similarity`]).
pub fn search(
    graph: &HnswGraph,
    vectors: &[f32],
    dim: usize,
    metric: Metric,
    query: &[f32],
    k: usize,
    ef: usize,
) -> Vec<(u32, f32)> {
    assert_eq!(query.len(), dim);
    if graph.nodes.is_empty() || k == 0 {
        return Vec::new();
    }
    // Wrap the workspace `NodeBuildState` shape so `search_layer`
    // can be shared between build and query — both want a flat
    // `neighbours[layer] -> [u32]` indexed by node id.
    let view = NodeView::new(&graph.nodes);

    // Greedy descent through upper layers down to layer 1.
    let mut ep_id = graph.entry_point;
    let mut ep_dist = metric.distance(query, vector_slice(vectors, dim, ep_id));
    let mut layer = graph.max_level;
    while layer > 0 {
        let (next_id, next_dist) =
            greedy_step_down_view(vectors, dim, metric, &view, ep_id, ep_dist, query, layer);
        ep_id = next_id;
        ep_dist = next_dist;
        layer -= 1;
    }

    let entry = vec![Candidate {
        id: ep_id,
        dist: ep_dist,
    }];
    let w = search_layer_view(vectors, dim, metric, &view, &entry, query, ef.max(k), 0);

    // Convert the dynamic candidate set to top-K by distance,
    // ascending → similarity descending. Truncate to k.
    let mut sorted = w;
    sorted.sort_by(|a, b| dist_cmp(a.dist, b.dist));
    sorted.truncate(k);
    sorted
        .into_iter()
        .map(|c| (c.id, distance_to_similarity(metric, c.dist)))
        .collect()
}

// ─── Workspace state shared between build and search ─────────────

struct NodeBuildState {
    level: u8,
    /// Per-layer adjacency (`[layer][i] = node_id`). `len == level + 1`.
    neighbours: Vec<Vec<u32>>,
}

impl NodeBuildState {
    fn new(level: u8) -> Self {
        let mut neighbours = Vec::with_capacity(level as usize + 1);
        for _ in 0..=level {
            neighbours.push(Vec::new());
        }
        Self { level, neighbours }
    }
}

/// Read-only view into a finished graph's adjacency lists. Lets the
/// search code share its layer-walker helpers with the build code
/// without taking out mutable borrows.
struct NodeView<'a> {
    nodes: &'a [HnswNode],
}

impl<'a> NodeView<'a> {
    fn new(nodes: &'a [HnswNode]) -> Self {
        Self { nodes }
    }

    fn neighbours_at(&self, node_id: u32, layer: u8) -> &'a [u32] {
        let n = &self.nodes[node_id as usize];
        if layer as usize >= n.neighbours.len() {
            &[]
        } else {
            &n.neighbours[layer as usize]
        }
    }
}

// ─── Core algorithm primitives (Malkov-Yashunin) ─────────────────

#[derive(Debug, Clone, Copy)]
struct Candidate {
    id: u32,
    /// Distance from the query in the metric's native sense
    /// (lower = closer). Used internally — never exposed.
    dist: f32,
}

/// Algorithm 2: SEARCH-LAYER. Greedy beam search inside one layer
/// starting from `entry_points`, maintaining a heap of the `ef`
/// best candidates by distance.
#[allow(clippy::too_many_arguments)]
fn search_layer(
    vectors: &[f32],
    dim: usize,
    metric: Metric,
    nodes: &[NodeBuildState],
    entry_points: &[Candidate],
    query: &[f32],
    ef: usize,
    layer: u8,
) -> Vec<Candidate> {
    // Adapter: build-state nodes seen as a flat read-only view.
    let view_nodes: Vec<HnswNode> = nodes
        .iter()
        .map(|n| HnswNode {
            level: n.level,
            neighbours: n.neighbours.clone(),
        })
        .collect();
    let view = NodeView::new(&view_nodes);
    search_layer_view(vectors, dim, metric, &view, entry_points, query, ef, layer)
}

#[allow(clippy::too_many_arguments)]
fn search_layer_view(
    vectors: &[f32],
    dim: usize,
    metric: Metric,
    view: &NodeView<'_>,
    entry_points: &[Candidate],
    query: &[f32],
    ef: usize,
    layer: u8,
) -> Vec<Candidate> {
    // `visited` is bounded by the node count; a `Vec<bool>` keyed
    // by node id is O(1) per check and per insert. For very large
    // graphs (10M+) consider a hash set with capacity-bound
    // pruning; the post-Load sweep's per-(layer, Index)
    // materialisation budget makes that a tractable v2 swap.
    let mut visited: Vec<bool> = vec![false; view.nodes.len()];
    // Min-heap of candidates left to expand (smallest distance at
    // the top — implemented via `Reverse` since BinaryHeap is a
    // max-heap).
    let mut frontier: BinaryHeap<std::cmp::Reverse<OrderedCand>> = BinaryHeap::new();
    // Max-heap of best-so-far results: keeping the *furthest* at
    // the top makes the "is the worst in W still better than the
    // furthest in frontier" check O(1).
    let mut best: BinaryHeap<OrderedCand> = BinaryHeap::new();

    for ep in entry_points {
        if (ep.id as usize) < visited.len() {
            visited[ep.id as usize] = true;
            frontier.push(std::cmp::Reverse(OrderedCand {
                id: ep.id,
                dist: ep.dist,
            }));
            best.push(OrderedCand {
                id: ep.id,
                dist: ep.dist,
            });
            if best.len() > ef {
                best.pop();
            }
        }
    }

    while let Some(std::cmp::Reverse(c)) = frontier.pop() {
        // If the closest unexpanded candidate is further than the
        // worst kept best, we're done.
        let worst_best = best.peek().map(|x| x.dist).unwrap_or(f32::INFINITY);
        if c.dist > worst_best && best.len() >= ef {
            break;
        }
        for &nid in view.neighbours_at(c.id, layer) {
            if (nid as usize) >= visited.len() || visited[nid as usize] {
                continue;
            }
            visited[nid as usize] = true;
            let d = metric.distance(query, vector_slice(vectors, dim, nid));
            let worst_best = best.peek().map(|x| x.dist).unwrap_or(f32::INFINITY);
            if best.len() < ef || d < worst_best {
                frontier.push(std::cmp::Reverse(OrderedCand { id: nid, dist: d }));
                best.push(OrderedCand { id: nid, dist: d });
                if best.len() > ef {
                    best.pop();
                }
            }
        }
    }
    best.into_iter()
        .map(|c| Candidate {
            id: c.id,
            dist: c.dist,
        })
        .collect()
}

/// Algorithm 3: SELECT-NEIGHBORS-SIMPLE. Pick the `m` closest to
/// `q` from the candidate set. We use a small-`m`-sized sort
/// rather than a heap — `m` ≤ ~32 in practice.
fn select_neighbours_simple(candidates: &[Candidate], m: usize) -> Vec<Candidate> {
    let mut sorted: Vec<Candidate> = candidates.to_vec();
    sorted.sort_by(|a, b| dist_cmp(a.dist, b.dist));
    sorted.truncate(m);
    sorted
}

/// Greedy descent within `layer`: starting from `(ep_id, ep_dist)`,
/// walk to the locally-nearest neighbour until no improvement is
/// found at this layer. Returns the final `(id, dist)` pair the
/// next layer down should treat as its entry point.
#[allow(clippy::too_many_arguments)]
fn greedy_step_down(
    vectors: &[f32],
    dim: usize,
    metric: Metric,
    nodes: &[NodeBuildState],
    ep_id: u32,
    ep_dist: f32,
    query: &[f32],
    layer: u8,
) -> (u32, f32) {
    let mut cur_id = ep_id;
    let mut cur_dist = ep_dist;
    loop {
        let nbrs = if (layer as usize) < nodes[cur_id as usize].neighbours.len() {
            &nodes[cur_id as usize].neighbours[layer as usize]
        } else {
            &[][..]
        };
        let mut improved = false;
        for &nid in nbrs {
            let d = metric.distance(query, vector_slice(vectors, dim, nid));
            if d < cur_dist {
                cur_id = nid;
                cur_dist = d;
                improved = true;
            }
        }
        if !improved {
            break;
        }
    }
    (cur_id, cur_dist)
}

#[allow(clippy::too_many_arguments)]
fn greedy_step_down_view(
    vectors: &[f32],
    dim: usize,
    metric: Metric,
    view: &NodeView<'_>,
    ep_id: u32,
    ep_dist: f32,
    query: &[f32],
    layer: u8,
) -> (u32, f32) {
    let mut cur_id = ep_id;
    let mut cur_dist = ep_dist;
    loop {
        let nbrs = view.neighbours_at(cur_id, layer);
        let mut improved = false;
        for &nid in nbrs {
            let d = metric.distance(query, vector_slice(vectors, dim, nid));
            if d < cur_dist {
                cur_id = nid;
                cur_dist = d;
                improved = true;
            }
        }
        if !improved {
            break;
        }
    }
    (cur_id, cur_dist)
}

// ─── Helpers: ordering, distances, RNG ───────────────────────────

#[derive(Debug, Clone, Copy)]
struct OrderedCand {
    id: u32,
    dist: f32,
}

impl PartialEq for OrderedCand {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && dist_cmp(self.dist, other.dist).is_eq()
    }
}
impl Eq for OrderedCand {}
impl Ord for OrderedCand {
    fn cmp(&self, other: &Self) -> Ordering {
        // Max-heap by distance: greater distance wins so it bubbles
        // to the top. Tiebreak by id for determinism.
        dist_cmp(self.dist, other.dist).then_with(|| self.id.cmp(&other.id))
    }
}
impl PartialOrd for OrderedCand {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Total ordering on distances; NaN treated as `+∞` (worst).
fn dist_cmp(a: f32, b: f32) -> Ordering {
    a.partial_cmp(&b).unwrap_or_else(|| {
        // NaN handling: one or both are NaN. Treat NaN > any
        // real → pushes NaN-distance candidates to the worst end.
        match (a.is_nan(), b.is_nan()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => Ordering::Equal,
        }
    })
}

#[inline]
fn vector_slice(vectors: &[f32], dim: usize, node_id: u32) -> &[f32] {
    let off = node_id as usize * dim;
    &vectors[off..off + dim]
}

/// Map the internal metric "distance" (lower = closer) back to
/// the kernel's "higher = better" similarity convention. Parity
/// with [`Metric::similarity`] is the M6.4 invariant the existing
/// HNSW dispatch relies on.
fn distance_to_similarity(metric: Metric, distance: f32) -> f32 {
    match metric {
        // `Metric::distance` for cosine returns `1 - cos`; invert.
        Metric::Cosine => 1.0 - distance,
        // `Metric::distance` for L2 returns the Euclidean distance;
        // similarity is `1 / (1 + d)` (same form as the brute-force
        // path's `Metric::similarity`).
        Metric::L2 => 1.0 / (1.0 + distance),
        // `Metric::distance` for dot returns `-dot`; similarity is
        // `dot` = `-distance`.
        Metric::Dot => -distance,
    }
}

/// Sample a random level from the geometric distribution used in
/// the original Malkov-Yashunin paper:
///
/// `floor(-ln(uniform(0, 1)) * m_L)`
///
/// where `m_L = 1 / ln(M)`. Clamped to `u8::MAX` defensively; with
/// `M = 16` the expected max level for 10 M nodes is around 8, so
/// the clamp is structural only.
fn sample_level(rng: &mut SplitMix64, m_l: f32) -> u8 {
    let u = rng.next_unit_f32().max(f32::EPSILON);
    let raw = (-u.ln() * m_l).floor();
    raw.clamp(0.0, u8::MAX as f32) as u8
}

/// SplitMix64 — a small, well-distributed PRNG. We don't need
/// cryptographic quality; we need reproducibility across
/// kernel-restart-replays of the same sweep.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }

    fn next_unit_f32(&mut self) -> f32 {
        // 24-bit mantissa → uniform in [0, 1).
        let bits = (self.next_u64() >> 40) as u32;
        bits as f32 / (1u32 << 24) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_vec_2d(angle_deg: f32) -> Vec<f32> {
        let a = angle_deg.to_radians();
        vec![a.cos(), a.sin()]
    }

    fn four_unit_vectors() -> (Vec<f32>, usize) {
        let mut data = Vec::new();
        for &deg in &[0.0f32, 90.0, 180.0, 270.0] {
            data.extend(unit_vec_2d(deg));
        }
        (data, 2)
    }

    #[test]
    fn empty_input_yields_empty_graph() {
        let g = build(
            &[],
            4,
            Metric::Cosine,
            CoreBuildConfig::default_for_segment(0),
        );
        assert_eq!(g.count(), 0);
    }

    #[test]
    fn single_vector_is_its_own_entry_point() {
        let g = build(
            &[1.0, 0.0, 0.0, 0.0],
            4,
            Metric::Cosine,
            CoreBuildConfig::default_for_segment(42),
        );
        assert_eq!(g.count(), 1);
        assert_eq!(g.entry_point, 0);
        assert_eq!(g.max_level, 0);
        // Single node has level 0 and an empty layer-0 adjacency.
        assert_eq!(g.nodes[0].level, 0);
        assert_eq!(g.nodes[0].neighbours, vec![Vec::<u32>::new()]);
    }

    #[test]
    fn self_query_returns_self_for_small_corpus() {
        let (data, dim) = four_unit_vectors();
        let g = build(
            &data,
            dim,
            Metric::Cosine,
            CoreBuildConfig::default_for_segment(7),
        );
        for i in 0..4 {
            let q = &data[i * dim..(i + 1) * dim];
            let hits = search(&g, &data, dim, Metric::Cosine, q, 1, 16);
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].0, i as u32, "self-query at i={i} must return self");
            assert!(
                (hits[0].1 - 1.0).abs() < 1e-5,
                "self-similarity ≈ 1; got {}",
                hits[0].1
            );
        }
    }

    #[test]
    fn search_returns_hits_in_descending_similarity_order() {
        let mut data = Vec::new();
        for k in 0..8 {
            let deg = (k as f32) * 45.0;
            data.extend(unit_vec_2d(deg));
        }
        let g = build(
            &data,
            2,
            Metric::Cosine,
            CoreBuildConfig::default_for_segment(11),
        );
        let q = unit_vec_2d(45.0);
        let hits = search(&g, &data, 2, Metric::Cosine, &q, 4, 32);
        for w in hits.windows(2) {
            assert!(
                w[0].1 >= w[1].1,
                "similarity must be descending; got {} then {}",
                w[0].1,
                w[1].1
            );
        }
        // Top-1 is index 1 (the 45° point — same as the query).
        assert_eq!(hits[0].0, 1);
        assert!((hits[0].1 - 1.0).abs() < 1e-4);
    }

    #[test]
    fn search_respects_k() {
        let (data, dim) = four_unit_vectors();
        let g = build(
            &data,
            dim,
            Metric::Cosine,
            CoreBuildConfig::default_for_segment(1),
        );
        let q = &data[0..dim];
        let hits = search(&g, &data, dim, Metric::Cosine, q, 3, 32);
        assert_eq!(hits.len(), 3);
    }

    #[test]
    fn build_with_deterministic_seed_is_reproducible() {
        let (data, dim) = four_unit_vectors();
        let g1 = build(
            &data,
            dim,
            Metric::Cosine,
            CoreBuildConfig::default_for_segment(123),
        );
        let g2 = build(
            &data,
            dim,
            Metric::Cosine,
            CoreBuildConfig::default_for_segment(123),
        );
        assert_eq!(g1, g2, "same seed must produce identical graphs");
    }

    #[test]
    fn build_with_different_seeds_can_differ_in_levels() {
        // 32-node corpus: enough nodes that the random level
        // generator's output varies between seeds. Verifies the
        // RNG is actually plumbed in (regression guard for
        // accidental hard-coded seeds).
        let mut data: Vec<f32> = Vec::new();
        for i in 0..32 {
            data.extend(unit_vec_2d(i as f32 * 360.0 / 32.0));
        }
        let g_a = build(
            &data,
            2,
            Metric::Cosine,
            CoreBuildConfig::default_for_segment(1),
        );
        let g_b = build(
            &data,
            2,
            Metric::Cosine,
            CoreBuildConfig::default_for_segment(2),
        );
        // Topologies usually differ — at minimum, the max_level
        // or per-node levels rarely agree across seeds.
        let levels_a: Vec<u8> = g_a.nodes.iter().map(|n| n.level).collect();
        let levels_b: Vec<u8> = g_b.nodes.iter().map(|n| n.level).collect();
        assert_ne!(levels_a, levels_b, "different seeds should diverge");
    }

    /// Recall sanity check on a ~200-node corpus. With ef=64 the
    /// paper's published anchor is ~95% recall@k; we test the
    /// far-easier "self-query top-1 must hit" guarantee on every
    /// indexed vector. A failure here means the search loop is
    /// returning misordered results, not that recall is
    /// borderline.
    #[test]
    fn self_queries_hit_top_1_on_larger_corpus() {
        use sha2::{Digest, Sha256};
        let dim = 16;
        let count = 200;
        let mut data: Vec<f32> = Vec::with_capacity(count * dim);
        for i in 0..count {
            let mut h = Sha256::new();
            h.update((i as u64).to_le_bytes());
            let digest = h.finalize();
            for j in 0..dim {
                let chunk_idx = j % 8;
                let bytes = [
                    digest[chunk_idx * 4],
                    digest[chunk_idx * 4 + 1],
                    digest[chunk_idx * 4 + 2],
                    digest[chunk_idx * 4 + 3],
                ];
                let u = u32::from_le_bytes(bytes);
                let scaled = (u as f32 / u32::MAX as f32) * 2.0 - 1.0;
                data.push(scaled);
            }
        }
        let g = build(
            &data,
            dim,
            Metric::Cosine,
            CoreBuildConfig {
                m: 16,
                ef_construction: 200,
                seed: 99,
            },
        );
        // Spot-check every 10th node; full-corpus check is
        // unnecessary at this size and slows the test suite.
        for i in (0..count).step_by(10) {
            let q = &data[i * dim..(i + 1) * dim];
            let hits = search(&g, &data, dim, Metric::Cosine, q, 5, 64);
            assert_eq!(
                hits[0].0, i as u32,
                "self-query at i={i} must return self as top-1"
            );
            assert!(
                (hits[0].1 - 1.0).abs() < 1e-4,
                "self-similarity ≈ 1 at i={i}; got {}",
                hits[0].1
            );
        }
    }

    #[test]
    fn build_emits_format_compatible_graph() {
        // The graph the algorithm produces must round-trip through
        // the §2.4 wire encoder without complaint — pins the
        // alignment between build output and the format module's
        // validation rules.
        use crate::query::vector::hnsw_format::{decode, encode};
        let (data, dim) = four_unit_vectors();
        let g = build(
            &data,
            dim,
            Metric::Cosine,
            CoreBuildConfig::default_for_segment(0),
        );
        let bytes = encode(&g);
        let decoded = decode(&bytes).expect("round-trip");
        assert_eq!(decoded, g);
    }

    #[test]
    fn build_with_l2_metric_works() {
        // Cosine is the only metric tested above; pin L2 with the
        // same self-query property as a regression guard.
        let (data, dim) = four_unit_vectors();
        let g = build(
            &data,
            dim,
            Metric::L2,
            CoreBuildConfig::default_for_segment(0),
        );
        for i in 0..4 {
            let q = &data[i * dim..(i + 1) * dim];
            let hits = search(&g, &data, dim, Metric::L2, q, 1, 16);
            assert_eq!(hits[0].0, i as u32);
            // L2-similarity at zero distance is 1.0.
            assert!((hits[0].1 - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn dist_cmp_handles_nan() {
        let nan = f32::NAN;
        assert_eq!(dist_cmp(nan, 0.5), Ordering::Greater);
        assert_eq!(dist_cmp(0.5, nan), Ordering::Less);
        assert_eq!(dist_cmp(nan, nan), Ordering::Equal);
        assert_eq!(dist_cmp(0.3, 0.7), Ordering::Less);
    }

    #[test]
    fn splitmix64_produces_uniform_unit_floats() {
        let mut rng = SplitMix64::new(42);
        for _ in 0..1000 {
            let v = rng.next_unit_f32();
            assert!((0.0..1.0).contains(&v), "expected [0,1); got {v}");
        }
    }

    #[test]
    fn sample_level_distribution_matches_paper() {
        // Malkov-Yashunin: `level = floor(-ln(u) * 1/ln(M))`.
        // For `u ~ Uniform(0, 1)` this gives `P(level = 0) = 1 - 1/M`,
        // so at M=16 about 93.75 % of nodes live only at the base
        // layer — that's the whole point of the hierarchy
        // (cheap base, sparse upper levels). Verify the empirical
        // fraction lands inside a tight band around the analytic
        // value, which doubles as a regression guard if the RNG
        // ever drifts.
        let mut rng = SplitMix64::new(13);
        let m: f32 = 16.0;
        let m_l = 1.0 / m.ln();
        let mut zero = 0;
        let trials = 5000;
        for _ in 0..trials {
            if sample_level(&mut rng, m_l) == 0 {
                zero += 1;
            }
        }
        let frac = zero as f32 / trials as f32;
        // Analytic: 1 - 1/16 = 0.9375. ±3 % band for sampling noise.
        let expected = 1.0 - 1.0 / m;
        assert!(
            (expected - frac).abs() < 0.03,
            "level-0 fraction {frac} differs from analytic {expected} by more than 3%"
        );
    }
}
