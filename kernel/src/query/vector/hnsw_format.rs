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

//! D43 §2.4 / M6-finish.1 — library-independent HNSW graph wire format.
//!
//! The §2.4 design committed to a stable on-wire HNSW topology
//! independent of any specific library:
//!
//! > Per-node fields, packed sequentially: level (varint), level-0
//! > neighbour list (length-prefixed Vec<u32>), upper-level neighbour
//! > lists (sparse — only nodes at level ≥ L appear at each
//! > upper-level section). The on-wire format is stable across HNSW
//! > library choices so the decoder dispatches correctly regardless
//! > of which library produced the graph at build time.
//!
//! ## What this module owns
//!
//! Two pure-data types — [`HnswGraph`] and [`HnswNode`] — plus an
//! [`encode`] / [`decode`] round-trip pair. The format is the
//! authoritative shape that any HNSW build path (vendored algorithm,
//! external library adapter, future implementations) targets. The
//! vendored algorithm (M6-finish.2) emits [`HnswGraph`] directly;
//! the SegmentCache admission helper consumes one.
//!
//! ## What this module deliberately does *not* cover
//!
//! - **Vectors.** The graph is just topology — adjacency lists.
//!   Vectors live in the segment's `vectors` bstr (already aligned
//!   per M5.10) and are addressed by the node-id integers stored
//!   here.
//! - **Build parameters.** `M` and `ef_construction` live in the
//!   parent CBOR map alongside `hnsw_graph` (§2.4 `hnsw_params`
//!   block); they're not part of the graph bytes themselves so the
//!   same topology can be re-read under different runtime
//!   parameters.
//! - **Distance metric.** Travels in the segment's metadata.
//! - **Search state.** Per-search `ef`, candidate heaps, etc. are
//!   runtime concerns.
//!
//! ## Byte layout
//!
//! All integers stored little-endian. Variable-length integers use
//! the unsigned LEB128 encoding (one byte per 7 payload bits, MSB
//! continuation flag).
//!
//! ```text
//! header (fixed, 8 + varints):
//!   magic         u32 LE   = 0x53484745 ("EGHS" little-endian read)
//!   version       u8       = 1 (this revision)
//!   count         varint   - number of nodes (u32-bounded)
//!   entry_point   varint   - node id of the global entry point
//!   max_level     u8       - max level across all nodes
//!
//! per node, count times:
//!   level         u8       - this node's max level (0 = base layer only)
//!   per layer L in 0..=level:
//!     n           varint   - neighbour count
//!     ids         n × u32 LE - neighbour node ids at level L
//! ```
//!
//! ## Validation guarantees
//!
//! [`decode`] enforces:
//!
//! - magic + version match (rejects foreign / future formats);
//! - the input is not truncated mid-record;
//! - every neighbour id is `< count`;
//! - the entry_point is `< count` (unless `count == 0`);
//! - every node's declared `level <= max_level`.
//!
//! It does **not** enforce the bidirectionality of the graph (HNSW
//! makes bidirectionality a *build-time* invariant, not a
//! storage-format constraint). Builders that break it produce
//! correct-but-degraded search; this module reads what's written.

/// Magic at the start of every encoded graph. ASCII `EGHS` read
/// little-endian.
pub const MAGIC: u32 = 0x53484745;

/// Current format version. Bumped on any incompatible byte-layout
/// change.
pub const VERSION: u8 = 1;

/// Owned HNSW graph in the §2.4 wire shape. Holds the global
/// header data + per-node adjacency lists.
#[derive(Debug, Clone, PartialEq)]
pub struct HnswGraph {
    /// Global entry-point node id (the highest-level node). Used
    /// by the search loop's top-down descent.
    pub entry_point: u32,
    /// Maximum level across all nodes. `0` for a single-layer graph.
    pub max_level: u8,
    /// Per-node adjacency. `nodes[i]` is the i-th node's record.
    /// `nodes.len()` is the total count.
    pub nodes: Vec<HnswNode>,
}

/// One node's adjacency record.
#[derive(Debug, Clone, PartialEq)]
pub struct HnswNode {
    /// This node's max level. `neighbours.len()` == `level as usize + 1`.
    pub level: u8,
    /// Per-level neighbour id lists. `neighbours[L]` is the
    /// adjacency list at layer `L`, where `L` ranges
    /// `0..=level`. Index 0 is the base layer.
    pub neighbours: Vec<Vec<u32>>,
}

impl HnswGraph {
    /// Node count.
    pub fn count(&self) -> usize {
        self.nodes.len()
    }
}

/// Errors raised by [`decode`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FormatError {
    #[error("input truncated at offset {0}; need more bytes")]
    Truncated(usize),
    #[error("bad magic {found:#x} (expected {expected:#x})")]
    BadMagic { found: u32, expected: u32 },
    #[error("unsupported version {found} (decoder accepts {expected})")]
    UnsupportedVersion { found: u8, expected: u8 },
    #[error("varint at offset {0} overflows u32")]
    VarintOverflow(usize),
    #[error("entry_point {0} out of range (count = {1})")]
    EntryPointOutOfRange(u32, u32),
    #[error("node {0} declares level {1}, exceeding max_level {2}")]
    NodeLevelExceedsMax(u32, u8, u8),
    #[error("node {0} layer {1} has neighbour id {2} out of range (count = {3})")]
    NeighbourOutOfRange(u32, u8, u32, u32),
    #[error("trailing bytes after end of graph (extra = {0})")]
    TrailingBytes(usize),
}

// ─── Encoder ────────────────────────────────────────────────────

/// Encode a graph to the §2.4 wire format. Cannot fail — the
/// in-memory representation is by construction encodable.
pub fn encode(graph: &HnswGraph) -> Vec<u8> {
    let mut out = Vec::with_capacity(estimate_encoded_size(graph));
    out.extend_from_slice(&MAGIC.to_le_bytes());
    out.push(VERSION);
    write_varint(&mut out, graph.count() as u32 as u64);
    write_varint(&mut out, graph.entry_point as u64);
    out.push(graph.max_level);
    for node in &graph.nodes {
        out.push(node.level);
        debug_assert_eq!(
            node.neighbours.len(),
            node.level as usize + 1,
            "HnswNode.neighbours.len must equal level + 1"
        );
        for layer_nbrs in &node.neighbours {
            write_varint(&mut out, layer_nbrs.len() as u64);
            for &id in layer_nbrs {
                out.extend_from_slice(&id.to_le_bytes());
            }
        }
    }
    out
}

/// Conservative upper bound used to pre-size the output buffer.
/// Fixed header (8 bytes header + varints, capped at 5 bytes
/// each), plus per-node `1 + (level+1) * (5 + 4 * avg_neighbours)`.
fn estimate_encoded_size(graph: &HnswGraph) -> usize {
    let mut n = 4 + 1 + 5 + 5 + 1;
    for node in &graph.nodes {
        n += 1;
        for layer in &node.neighbours {
            n += 5 + 4 * layer.len();
        }
    }
    n
}

// ─── Decoder ────────────────────────────────────────────────────

/// Decode a graph from §2.4 wire bytes.
pub fn decode(bytes: &[u8]) -> Result<HnswGraph, FormatError> {
    let mut cur = Cursor::new(bytes);
    let magic = cur.read_u32_le()?;
    if magic != MAGIC {
        return Err(FormatError::BadMagic {
            found: magic,
            expected: MAGIC,
        });
    }
    let version = cur.read_u8()?;
    if version != VERSION {
        return Err(FormatError::UnsupportedVersion {
            found: version,
            expected: VERSION,
        });
    }
    let count = cur.read_varint()?;
    let entry_point = cur.read_varint()?;
    let max_level = cur.read_u8()?;

    if count > 0 && entry_point >= count {
        return Err(FormatError::EntryPointOutOfRange(entry_point, count));
    }

    let mut nodes: Vec<HnswNode> = Vec::with_capacity(count as usize);
    for node_idx in 0..count {
        let level = cur.read_u8()?;
        if level > max_level {
            return Err(FormatError::NodeLevelExceedsMax(node_idx, level, max_level));
        }
        let mut neighbours: Vec<Vec<u32>> = Vec::with_capacity(level as usize + 1);
        for layer_idx in 0..=level {
            let n = cur.read_varint()?;
            let mut layer_nbrs: Vec<u32> = Vec::with_capacity(n as usize);
            for _ in 0..n {
                let nid = cur.read_u32_le()?;
                if nid >= count {
                    return Err(FormatError::NeighbourOutOfRange(
                        node_idx, layer_idx, nid, count,
                    ));
                }
                layer_nbrs.push(nid);
            }
            neighbours.push(layer_nbrs);
        }
        nodes.push(HnswNode { level, neighbours });
    }
    if !cur.is_at_end() {
        return Err(FormatError::TrailingBytes(cur.remaining()));
    }
    Ok(HnswGraph {
        entry_point,
        max_level,
        nodes,
    })
}

// ─── Internal: cursor + varint codec ────────────────────────────

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    fn is_at_end(&self) -> bool {
        self.pos == self.bytes.len()
    }

    fn read_u8(&mut self) -> Result<u8, FormatError> {
        let b = *self
            .bytes
            .get(self.pos)
            .ok_or(FormatError::Truncated(self.pos))?;
        self.pos += 1;
        Ok(b)
    }

    fn read_u32_le(&mut self) -> Result<u32, FormatError> {
        if self.pos + 4 > self.bytes.len() {
            return Err(FormatError::Truncated(self.pos));
        }
        let arr: [u8; 4] = self.bytes[self.pos..self.pos + 4]
            .try_into()
            .expect("slice length 4");
        self.pos += 4;
        Ok(u32::from_le_bytes(arr))
    }

    /// Read an unsigned LEB128 varint. Returns u32 because §2.4
    /// caps node counts at 2^32 - 1 (the segment-size envelope
    /// per §5.5 is ~10M, leaving comfortable headroom). Overflow
    /// surfaces as [`FormatError::VarintOverflow`].
    fn read_varint(&mut self) -> Result<u32, FormatError> {
        let start = self.pos;
        let mut result: u64 = 0;
        let mut shift = 0u32;
        loop {
            let b = self.read_u8()?;
            result |= ((b & 0x7f) as u64) << shift;
            if b & 0x80 == 0 {
                break;
            }
            shift += 7;
            if shift >= 35 {
                // 5 bytes max for u32 (5 × 7 = 35 payload bits).
                return Err(FormatError::VarintOverflow(start));
            }
        }
        if result > u32::MAX as u64 {
            return Err(FormatError::VarintOverflow(start));
        }
        Ok(result as u32)
    }
}

/// Write an unsigned LEB128 varint. `value` is `u64` to share the
/// helper with the cursor type, but callers must keep payloads in
/// the u32 range — the format is `u32`-bounded per the §2.4 v1
/// envelope.
fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    debug_assert!(value <= u32::MAX as u64, "varint payload exceeds u32");
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small_graph() -> HnswGraph {
        HnswGraph {
            entry_point: 1,
            max_level: 1,
            nodes: vec![
                HnswNode {
                    level: 0,
                    neighbours: vec![vec![1, 2]],
                },
                HnswNode {
                    level: 1,
                    neighbours: vec![vec![0, 2], vec![2]],
                },
                HnswNode {
                    level: 0,
                    neighbours: vec![vec![0, 1]],
                },
            ],
        }
    }

    // ─── Header round-trip ──────────────────────────────────────

    #[test]
    fn round_trip_preserves_graph_exactly() {
        let g = small_graph();
        let bytes = encode(&g);
        let decoded = decode(&bytes).expect("decode");
        assert_eq!(decoded, g);
    }

    #[test]
    fn encoded_starts_with_magic_and_version() {
        let g = small_graph();
        let bytes = encode(&g);
        assert!(bytes.len() > 5);
        let magic = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        assert_eq!(magic, MAGIC);
        assert_eq!(bytes[4], VERSION);
    }

    #[test]
    fn empty_graph_round_trips() {
        let g = HnswGraph {
            entry_point: 0,
            max_level: 0,
            nodes: Vec::new(),
        };
        let bytes = encode(&g);
        let decoded = decode(&bytes).expect("decode empty");
        assert_eq!(decoded, g);
    }

    #[test]
    fn single_node_no_neighbours_round_trips() {
        let g = HnswGraph {
            entry_point: 0,
            max_level: 0,
            nodes: vec![HnswNode {
                level: 0,
                neighbours: vec![vec![]],
            }],
        };
        let bytes = encode(&g);
        let decoded = decode(&bytes).expect("decode single");
        assert_eq!(decoded, g);
    }

    #[test]
    fn large_node_id_round_trips() {
        // Stress the u32 LE encoding with values near the upper end
        // of the dim range we expect HNSW segments to ever hit.
        let g = HnswGraph {
            entry_point: 0,
            max_level: 0,
            nodes: vec![HnswNode {
                level: 0,
                neighbours: vec![vec![100_000, 1_000_000, 9_999_999]],
            }],
        };
        // count = 1 here, so the in-bounds check would reject ids
        // ≥ 1. Instead, use a graph where the id space matches.
        let n_nodes = 10_000_000u32;
        let mut nodes: Vec<HnswNode> = (0..n_nodes)
            .map(|_| HnswNode {
                level: 0,
                neighbours: vec![vec![]],
            })
            .collect();
        nodes[0] = HnswNode {
            level: 0,
            neighbours: vec![vec![100_000, 1_000_000, 9_999_999]],
        };
        let _ = g; // suppress unused-binding lint on the discarded fixture
        let big = HnswGraph {
            entry_point: 0,
            max_level: 0,
            nodes,
        };
        let bytes = encode(&big);
        let decoded = decode(&bytes).expect("decode large");
        assert_eq!(
            decoded.nodes[0].neighbours[0],
            vec![100_000, 1_000_000, 9_999_999]
        );
        assert_eq!(decoded.count(), n_nodes as usize);
    }

    #[test]
    fn upper_level_sparse_neighbours_round_trip() {
        // Node 1 has 3 levels (0, 1, 2). Node 0 has only level 0.
        // Verifies the per-node loop iterates `0..=level` correctly.
        let g = HnswGraph {
            entry_point: 1,
            max_level: 2,
            nodes: vec![
                HnswNode {
                    level: 0,
                    neighbours: vec![vec![1]],
                },
                HnswNode {
                    level: 2,
                    neighbours: vec![vec![0], vec![], vec![]],
                },
            ],
        };
        let bytes = encode(&g);
        let decoded = decode(&bytes).expect("decode multi-level");
        assert_eq!(decoded, g);
    }

    // ─── Validation errors ──────────────────────────────────────

    #[test]
    fn bad_magic_rejected() {
        let mut bytes = encode(&small_graph());
        bytes[0] = 0xff;
        let err = decode(&bytes).unwrap_err();
        assert!(matches!(err, FormatError::BadMagic { .. }));
    }

    #[test]
    fn unsupported_version_rejected() {
        let mut bytes = encode(&small_graph());
        bytes[4] = 99;
        let err = decode(&bytes).unwrap_err();
        assert!(matches!(
            err,
            FormatError::UnsupportedVersion { found: 99, .. }
        ));
    }

    #[test]
    fn truncated_header_rejected() {
        let bytes = encode(&small_graph());
        let truncated = &bytes[..3];
        let err = decode(truncated).unwrap_err();
        assert!(matches!(err, FormatError::Truncated(_)));
    }

    #[test]
    fn truncated_mid_node_rejected() {
        let bytes = encode(&small_graph());
        // Lop off the last byte — should leave a partial neighbour id.
        let truncated = &bytes[..bytes.len() - 1];
        let err = decode(truncated).unwrap_err();
        assert!(matches!(err, FormatError::Truncated(_)));
    }

    #[test]
    fn entry_point_out_of_range_rejected() {
        let g = HnswGraph {
            entry_point: 5,
            max_level: 0,
            nodes: vec![HnswNode {
                level: 0,
                neighbours: vec![vec![]],
            }],
        };
        let bytes = encode(&g);
        let err = decode(&bytes).unwrap_err();
        assert!(matches!(err, FormatError::EntryPointOutOfRange(5, 1)));
    }

    #[test]
    fn neighbour_id_out_of_range_rejected() {
        // Hand-build bytes where node 0's only neighbour is id 7
        // but count = 2.
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend_from_slice(&MAGIC.to_le_bytes());
        bytes.push(VERSION);
        write_varint(&mut bytes, 2); // count
        write_varint(&mut bytes, 0); // entry
        bytes.push(0); // max_level
                       // node 0: level 0, 1 neighbour at layer 0, id = 7 (out of range)
        bytes.push(0);
        write_varint(&mut bytes, 1);
        bytes.extend_from_slice(&7u32.to_le_bytes());
        // node 1: level 0, 0 neighbours
        bytes.push(0);
        write_varint(&mut bytes, 0);
        let err = decode(&bytes).unwrap_err();
        assert!(matches!(err, FormatError::NeighbourOutOfRange(0, 0, 7, 2)));
    }

    #[test]
    fn node_level_exceeds_max_level_rejected() {
        let g = HnswGraph {
            entry_point: 0,
            max_level: 1,
            nodes: vec![HnswNode {
                level: 5, // bogus — exceeds max_level
                neighbours: vec![vec![], vec![], vec![], vec![], vec![], vec![]],
            }],
        };
        let bytes = encode(&g);
        let err = decode(&bytes).unwrap_err();
        assert!(matches!(err, FormatError::NodeLevelExceedsMax(0, 5, 1)));
    }

    #[test]
    fn trailing_bytes_rejected() {
        let mut bytes = encode(&small_graph());
        bytes.extend_from_slice(&[0u8, 1, 2, 3]);
        let err = decode(&bytes).unwrap_err();
        assert!(matches!(err, FormatError::TrailingBytes(4)));
    }

    // ─── Varint codec ───────────────────────────────────────────

    #[test]
    fn varint_encodes_small_values_compactly() {
        let mut bytes = Vec::new();
        write_varint(&mut bytes, 0);
        assert_eq!(bytes, vec![0]);
        let mut bytes = Vec::new();
        write_varint(&mut bytes, 127);
        assert_eq!(bytes, vec![127]);
        let mut bytes = Vec::new();
        write_varint(&mut bytes, 128);
        assert_eq!(bytes, vec![0x80, 0x01]);
    }

    #[test]
    fn varint_round_trip_at_boundaries() {
        for &v in &[0u32, 1, 127, 128, 16_383, 16_384, 100_000, u32::MAX] {
            let mut bytes = Vec::new();
            write_varint(&mut bytes, v as u64);
            let mut cur = Cursor::new(&bytes);
            assert_eq!(cur.read_varint().expect("decode"), v);
            assert!(cur.is_at_end());
        }
    }

    #[test]
    fn varint_overflow_rejected() {
        // 6-byte varint payload — exceeds the 5-byte u32 budget.
        let bytes = [0xff, 0xff, 0xff, 0xff, 0xff, 0x01];
        let mut cur = Cursor::new(&bytes);
        let err = cur.read_varint().unwrap_err();
        assert!(matches!(err, FormatError::VarintOverflow(0)));
    }

    #[test]
    fn encoded_size_grows_with_neighbour_count() {
        let small = HnswGraph {
            entry_point: 0,
            max_level: 0,
            nodes: vec![HnswNode {
                level: 0,
                neighbours: vec![vec![]],
            }],
        };
        let with_nbrs = HnswGraph {
            entry_point: 0,
            max_level: 0,
            nodes: vec![HnswNode {
                level: 0,
                neighbours: vec![(0..16).collect()],
            }],
        };
        // count=1 so all 16 neighbour ids point to node 0 — they all
        // dedup-ID to 0, but the format keeps the multiplicity.
        let real = HnswGraph {
            nodes: vec![HnswNode {
                level: 0,
                neighbours: vec![vec![0; 16]],
            }],
            ..with_nbrs
        };
        assert!(encode(&real).len() > encode(&small).len());
    }
}
