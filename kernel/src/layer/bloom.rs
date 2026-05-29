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

//! Per-layer shadowing bloom filter (D23 §5.2).
//!
//! Each persisted layer carries one `BloomFilter` over the IRIs it defines
//! directly. `Layer::resolve` (Phase 14b-iii) consults each ancestor's bloom
//! to skip layers without a matching definition, only probing the cache /
//! backend at layers the bloom flags as "maybe present". Walking the chain
//! head→root with the blooms produces shadowing semantics without the
//! per-head index originally proposed in D23.
//!
//! **Algorithm.** Standard bloom-filter construction with double hashing
//! (Kirsch & Mitzenmacher 2006): hash the IRI once to obtain `(h_a, h_b)`,
//! then probe positions `h_a + i * h_b` for `i ∈ 0..k`. Hash function is
//! SHA-256 of the IRI bytes — deterministic and well-distributed. Bit count
//! `m` is rounded up to a power of two so probe-position modulo is a single
//! bit-and (`& (m-1)`) instead of integer division.
//!
//! **Sizing.** Given `n` IRIs and target false-positive rate `p`:
//! - `m ≈ -n * ln(p) / (ln(2)^2)`
//! - `k ≈ (m / n) * ln(2)`
//!
//! Default `p = 0.01` (1%, per D23 §5.2.6). For `n = 10_000`, `p = 0.01`:
//! `m ≈ 95851 → 131072 bits = 16 KiB`, `k ≈ 7`.
//!
//! **Persistence.** Serializes via `serde` + `ciborium` to the same CBOR
//! byte stream as other kernel storage (D23 §6.2: `bloom:<layer_id>` keys).
//! Wire format is the field set below; `bit_count` and `hash_count` are
//! baked at build time and never change for a given layer.

use crate::ontology::iri::Iri;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

/// Default false-positive rate at the layer's actual IRI count. D23 §5.2.6.
pub const DEFAULT_FPR: f64 = 0.01;

/// Minimum bit count for the smallest practical bloom (covers empty layers
/// and the smallest non-empty layers without degenerate FPR).
const MIN_BIT_COUNT: u64 = 64;

/// Per-layer shadowing bloom filter.
///
/// Constructed once at layer commit time; immutable thereafter. Carries
/// the parameters baked in at build time so deserialization needs no
/// extra context. `iri_count` is diagnostic only — `might_contain` does
/// not use it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BloomFilter {
    /// Bit array, 64 bits per `u64`. Length is `bit_count / 64`.
    bits: Vec<u64>,
    /// Total bit count `m`. Always a power of two ≥ `MIN_BIT_COUNT`.
    bit_count: u64,
    /// Number of hash probes per insert/query, `k`.
    hash_count: u32,
    /// Number of IRIs inserted. Diagnostic only.
    iri_count: u64,
}

impl BloomFilter {
    /// Build a bloom from a single IRI set at the default FPR. Used
    /// by tests and any callsite that wants a bloom over an arbitrary
    /// set of IRIs (not a layer's visibility state). For per-layer
    /// shadowing blooms use [`BloomFilter::for_layer`].
    pub fn for_iris(iris: &BTreeSet<Iri>) -> Self {
        Self::for_iris_with_fpr(iris, DEFAULT_FPR)
    }

    /// Build a bloom with an explicit target FPR. Used by tests that
    /// want to verify FPR behavior at non-default settings.
    pub fn for_iris_with_fpr(iris: &BTreeSet<Iri>, fpr: f64) -> Self {
        assert!(fpr > 0.0 && fpr < 1.0, "FPR must be in (0, 1)");
        let n = iris.len() as u64;
        let (bit_count, hash_count) = size_params(n, fpr);
        let mut bloom = Self {
            bits: vec![0u64; (bit_count / 64) as usize],
            bit_count,
            hash_count,
            iri_count: n,
        };
        for iri in iris {
            bloom.insert(iri);
        }
        bloom
    }

    /// Build a per-layer shadowing bloom over the union of `defined`
    /// and `tombstoned`. The bloom is the master "should I consult
    /// this layer" gate during chain walks (D23 §5.2): every IRI the
    /// layer modifies the visibility of — by defining a body for it
    /// or by tombstoning a parent's body (D20 §6.2 / §6.3) — must
    /// produce `might_contain == true`. Walkers that skip a layer
    /// whose bloom returns `false` are guaranteed the layer changes
    /// nothing for that IRI.
    ///
    /// Pre-condition: `LayerBuilder` rejects layers that simultaneously
    /// define and tombstone the same IRI, so the two sets are disjoint
    /// and the bloom is sized for the exact union count.
    pub fn for_layer(defined: &BTreeSet<Iri>, tombstoned: &BTreeSet<Iri>) -> Self {
        let n = (defined.len() + tombstoned.len()) as u64;
        let (bit_count, hash_count) = size_params(n, DEFAULT_FPR);
        let mut bloom = Self {
            bits: vec![0u64; (bit_count / 64) as usize],
            bit_count,
            hash_count,
            iri_count: n,
        };
        for iri in defined.iter().chain(tombstoned.iter()) {
            bloom.insert(iri);
        }
        bloom
    }

    /// Fast probe: may return false positives, never false negatives.
    pub fn might_contain(&self, iri: &Iri) -> bool {
        let (h_a, h_b) = hash_pair(iri);
        let mask = self.bit_count - 1;
        for i in 0..self.hash_count as u64 {
            let pos = h_a.wrapping_add(i.wrapping_mul(h_b)) & mask;
            if !self.get_bit(pos) {
                return false;
            }
        }
        true
    }

    /// Number of IRIs that were inserted at build time. Diagnostic only.
    pub fn iri_count(&self) -> u64 {
        self.iri_count
    }

    /// Total bit count `m`.
    pub fn bit_count(&self) -> u64 {
        self.bit_count
    }

    /// Number of hash probes per query `k`.
    pub fn hash_count(&self) -> u32 {
        self.hash_count
    }

    /// Approximate in-memory size in bytes (bit array only).
    pub fn byte_size(&self) -> usize {
        self.bits.len() * 8
    }

    fn insert(&mut self, iri: &Iri) {
        let (h_a, h_b) = hash_pair(iri);
        let mask = self.bit_count - 1;
        for i in 0..self.hash_count as u64 {
            let pos = h_a.wrapping_add(i.wrapping_mul(h_b)) & mask;
            self.set_bit(pos);
        }
    }

    fn set_bit(&mut self, pos: u64) {
        let word = (pos / 64) as usize;
        let bit = pos % 64;
        self.bits[word] |= 1u64 << bit;
    }

    fn get_bit(&self, pos: u64) -> bool {
        let word = (pos / 64) as usize;
        let bit = pos % 64;
        (self.bits[word] & (1u64 << bit)) != 0
    }
}

/// Compute `(bit_count, hash_count)` for a target IRI count and FPR. The
/// bit count is rounded up to the next power of two so `& (m-1)` masking
/// works in `might_contain`. The hash count is clamped to `[1, 64]` —
/// no realistic (n, p) produces values outside this range.
fn size_params(n: u64, fpr: f64) -> (u64, u32) {
    if n == 0 {
        // Empty layer: smallest valid bloom, single probe. Reads always
        // miss (no bits set) which is the correct shadowing answer.
        return (MIN_BIT_COUNT, 1);
    }
    // m = -n * ln(p) / (ln(2)^2)
    let ln2 = std::f64::consts::LN_2;
    let m_ideal = -(n as f64) * fpr.ln() / (ln2 * ln2);
    let mut m = m_ideal.ceil() as u64;
    if m < MIN_BIT_COUNT {
        m = MIN_BIT_COUNT;
    }
    // Round up to next power of two.
    m = m.next_power_of_two();
    // k = (m / n) * ln(2)
    let k = ((m as f64 / n as f64) * ln2).round().clamp(1.0, 64.0) as u32;
    (m, k)
}

/// Hash an IRI to two independent u64 seeds for double-hashing. Stable
/// across runs — same IRI always hashes to the same pair, which is what
/// makes the bloom deterministic and content-addressable.
fn hash_pair(iri: &Iri) -> (u64, u64) {
    let mut hasher = Sha256::new();
    hasher.update(iri.as_str().as_bytes());
    let bytes = hasher.finalize();
    let h_a = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
    let h_b = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
    // Ensure h_b is odd so successive probes (h_a + i*h_b) cover the full
    // bit space modulo a power-of-two `m` — even h_b would skip half the
    // positions. Setting the low bit cheaply guarantees this.
    (h_a, h_b | 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    #[test]
    fn empty_bloom_returns_false_for_any_iri() {
        let bloom = BloomFilter::for_iris(&BTreeSet::new());
        assert!(!bloom.might_contain(&iri("urn:eigenius:test:x")));
        assert!(!bloom.might_contain(&iri("urn:eigenius:test:y")));
        assert_eq!(bloom.iri_count(), 0);
    }

    #[test]
    fn no_false_negatives() {
        // Every inserted IRI must report `might_contain == true`.
        let iris: BTreeSet<Iri> = (0..1000)
            .map(|i| iri(&format!("urn:eigenius:test:{i}")))
            .collect();
        let bloom = BloomFilter::for_iris(&iris);
        for i in &iris {
            assert!(
                bloom.might_contain(i),
                "false negative for {i} (bloom must never lose inserted entries)"
            );
        }
    }

    #[test]
    fn fpr_within_budget_at_default() {
        // Build with 10K IRIs at default 1% FPR; query 10K disjoint IRIs;
        // the empirical false-positive count should be in the same order
        // of magnitude as the budget. We allow 3× the budget as headroom
        // for the random variance — flake protection, not loose pass.
        let inserted: BTreeSet<Iri> = (0..10_000)
            .map(|i| iri(&format!("urn:eigenius:test:in:{i}")))
            .collect();
        let bloom = BloomFilter::for_iris(&inserted);

        let mut false_positives = 0u32;
        let probes = 10_000u32;
        for i in 0..probes {
            let candidate = iri(&format!("urn:eigenius:test:out:{i}"));
            if bloom.might_contain(&candidate) {
                false_positives += 1;
            }
        }
        let observed_fpr = false_positives as f64 / probes as f64;
        assert!(
            observed_fpr < DEFAULT_FPR * 3.0,
            "observed FPR {observed_fpr} exceeded 3× the {DEFAULT_FPR} budget \
             ({false_positives} false positives over {probes} probes)"
        );
    }

    #[test]
    fn deterministic_across_builds() {
        // Same IRI set must produce byte-identical blooms — content
        // addressing of `bloom:<layer_id>` depends on this.
        let iris: BTreeSet<Iri> = (0..200)
            .map(|i| iri(&format!("urn:eigenius:test:{i}")))
            .collect();
        let a = BloomFilter::for_iris(&iris);
        let b = BloomFilter::for_iris(&iris);
        assert_eq!(a, b);
    }

    #[test]
    fn cbor_round_trip() {
        let iris: BTreeSet<Iri> = (0..500)
            .map(|i| iri(&format!("urn:eigenius:test:{i}")))
            .collect();
        let bloom = BloomFilter::for_iris(&iris);

        let mut bytes = Vec::new();
        ciborium::into_writer(&bloom, &mut bytes).unwrap();
        let decoded: BloomFilter = ciborium::from_reader(bytes.as_slice()).unwrap();

        assert_eq!(bloom, decoded);
        for i in &iris {
            assert!(decoded.might_contain(i));
        }
    }

    #[test]
    fn bit_count_is_power_of_two() {
        for n in [0u64, 1, 100, 1_000, 100_000] {
            let (m, _) = size_params(n, 0.01);
            assert!(
                m.is_power_of_two(),
                "bit_count must be power of two for fast modulo (got {m} for n={n})"
            );
            assert!(m >= MIN_BIT_COUNT);
        }
    }

    #[test]
    fn for_layer_unions_defined_and_tombstoned() {
        // The bloom must report `might_contain == true` for both
        // defined IRIs and tombstoned IRIs — tombstones are
        // visibility-modifying changes the chain walk must not skip
        // past.
        let mut defined = BTreeSet::new();
        defined.insert(iri("urn:eigenius:test:def"));
        let mut tombstoned = BTreeSet::new();
        tombstoned.insert(iri("urn:eigenius:test:tomb"));

        let bloom = BloomFilter::for_layer(&defined, &tombstoned);
        assert!(bloom.might_contain(&iri("urn:eigenius:test:def")));
        assert!(bloom.might_contain(&iri("urn:eigenius:test:tomb")));
        // Untouched IRI doesn't have to miss (1% FPR budget), so we
        // only assert the union members are present.
        assert_eq!(bloom.iri_count(), 2);
    }

    #[test]
    fn for_layer_with_empty_tombstones_matches_for_iris() {
        // When tombstoned is empty, `for_layer` and `for_iris` must
        // produce byte-identical blooms — the on-disk shape for
        // layers without tombstones can't have churned.
        let mut defined = BTreeSet::new();
        for i in 0..50 {
            defined.insert(iri(&format!("urn:eigenius:test:{i}")));
        }
        let tombstoned: BTreeSet<Iri> = BTreeSet::new();

        let from_layer = BloomFilter::for_layer(&defined, &tombstoned);
        let from_iris = BloomFilter::for_iris(&defined);
        assert_eq!(from_layer, from_iris);
    }

    #[test]
    fn distinct_iris_set_distinct_bits() {
        // Two single-IRI blooms over different IRIs should not be equal —
        // basic sanity that hash_pair separates inputs.
        let mut s_a = BTreeSet::new();
        s_a.insert(iri("urn:eigenius:test:a"));
        let mut s_b = BTreeSet::new();
        s_b.insert(iri("urn:eigenius:test:b"));
        assert_ne!(BloomFilter::for_iris(&s_a), BloomFilter::for_iris(&s_b));
    }
}
