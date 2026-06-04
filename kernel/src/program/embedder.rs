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

//! D43 §5.2 / M4 — Embedder Component trait + registry.
//!
//! An `Embedder` is the IO-capability Component that turns a UTF-8
//! string into a fixed-dimensionality `f32` vector. It is the
//! dispatch target of [`crate::query::ast::Expression::FunctionCall`]
//! with name `EMBED` (M4.3) and of the post-Load vector-segment
//! sweep (M5, deferred).
//!
//! **Static dimensionality.** The dimensionality is declared on the
//! Component itself ([`Embedder::dim`]) so the typechecker (D43 §4.4,
//! follow-up) can verify model_iri / dim consistency at parse time
//! without dispatching the embedder. The IRI is the Component's
//! identity; model upgrades cut a new IRI rather than mutating an
//! existing Embedder's `dim`.
//!
//! **Non-determinism.** Per D43 §5.2, embedders are
//! `NonDeterministic` — hosted-API silent upgrades, non-deterministic
//! decoding strategies, and floating-point reproducibility across
//! hardware all defeat byte-equality on repeat embeds. The
//! content-addressed cache (M4 follow-up) is what makes repeated
//! embeds cheap; correctness does not depend on the embedder
//! re-producing the same bytes for the same input.
//!
//! **Why not extend `BuiltinComponent`.** The existing
//! [`crate::program::component::BuiltinComponent`] trait takes a
//! `Resource → Resource` signature. Embedders are `str → Vec<f32>`;
//! routing through a Resource wrapper for what is structurally a
//! typed primitive operation would add boxing and a structurally-
//! wrong reification of the output type. A sibling trait keeps the
//! type signature honest.

use crate::ontology::iri::Iri;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Errors returned by [`Embedder::embed`].
#[derive(Debug, thiserror::Error)]
pub enum EmbedderError {
    /// The embedder's IO dependency failed (network, hosted API,
    /// model load). Surfaces at query evaluation per D43 §5.8.
    #[error("embedder IO failure: {0}")]
    Io(String),
    /// The input text is structurally rejected (empty, too long,
    /// contains invalid characters for the model's tokenizer).
    /// Distinct from `Io` because the caller can sanitise input.
    #[error("invalid input for embedder: {0}")]
    InvalidInput(String),
}

/// A registered Embedder Component.
///
/// Implementations must be `Send + Sync` so a single registry can be
/// shared across query-handler threads. Determinism is not required
/// (the spec marks embedders `NonDeterministic`); implementations
/// that happen to be deterministic — like [`DummyEmbedder`] — are a
/// convenience for tests, not a contract.
pub trait Embedder: Send + Sync {
    /// The IRI identifying this Embedder Component. Same identity
    /// the typechecker (D43 §4.4) and the active VectorIndex
    /// Resource (D43 §3.1, `vec_model` slot) reference.
    fn model_iri(&self) -> &Iri;

    /// Output dimensionality. Static — does not change per call.
    /// The typechecker uses this to verify `EMBED(...)` results
    /// match the queried property's active VectorIndex's `vec_dim`.
    fn dim(&self) -> u32;

    /// Embed `text` into an `f32` vector of length `self.dim()`.
    /// Implementations must return a vector of exactly that length
    /// on success; a length mismatch is a programming error and the
    /// caller may panic. Failures surface as [`EmbedderError`].
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedderError>;

    /// Embed `texts` in one batched forward pass, returning a
    /// `Vec<Vec<f32>>` of length `texts.len()` with each inner vector
    /// of length `self.dim()`. The default implementation loops
    /// [`Self::embed`] one-at-a-time — correct, but slow for
    /// real ML backends. Embedders backed by a batched runtime
    /// (Candle, ORT, …) should override for the 10-30× speedup at
    /// batch ≈ 32. The sweep driver
    /// ([`crate::query::vector::indexing::sweep_one_index`]) calls
    /// this method exclusively; implementing only [`Self::embed`]
    /// still works but pays the per-text trip cost on the index
    /// path. Result ordering corresponds to input ordering — caller
    /// pairs them positionally.
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedderError> {
        texts.iter().map(|t| self.embed(t)).collect()
    }
}

/// Registry of [`Embedder`] implementations keyed by their declared
/// `model_iri`. Parallel to
/// [`crate::program::component::ComponentRegistry`] in structure
/// (parent-pointer chaining, `Arc`-shared, append-only).
pub struct EmbedderRegistry {
    embedders: BTreeMap<Iri, Arc<dyn Embedder>>,
    parent: Option<Arc<EmbedderRegistry>>,
}

impl EmbedderRegistry {
    /// Empty registry. Use [`Self::with_dummy`] in tests if you want
    /// the deterministic stub pre-registered.
    pub fn new() -> Self {
        Self {
            embedders: BTreeMap::new(),
            parent: None,
        }
    }

    /// Layered registry — local entries shadow parent entries with
    /// the same `model_iri`. Mirrors the `ComponentRegistry` parent
    /// pattern so chain-loaded embedders compose without cloning the
    /// underlying boxed objects.
    pub fn new_with_parent(parent: Arc<EmbedderRegistry>) -> Self {
        Self {
            embedders: BTreeMap::new(),
            parent: Some(parent),
        }
    }

    /// Register an [`Embedder`] implementation. The key is its
    /// declared `model_iri`; subsequent registrations with the same
    /// IRI overwrite local entries (parent entries are still
    /// shadowed via the lookup walk).
    pub fn register(&mut self, embedder: Arc<dyn Embedder>) {
        let iri = embedder.model_iri().clone();
        self.embedders.insert(iri, embedder);
    }

    /// Look up an embedder by its `model_iri`. Walks the parent
    /// chain on local miss.
    pub fn get(&self, model_iri: &Iri) -> Option<Arc<dyn Embedder>> {
        if let Some(e) = self.embedders.get(model_iri) {
            return Some(Arc::clone(e));
        }
        self.parent.as_ref().and_then(|p| p.get(model_iri))
    }

    /// Enumerate every registered Embedder's `model_iri`, local +
    /// inherited, deduplicated.
    pub fn list(&self) -> Vec<Iri> {
        let mut out: std::collections::BTreeSet<Iri> = self.embedders.keys().cloned().collect();
        if let Some(p) = &self.parent {
            for iri in p.list() {
                out.insert(iri);
            }
        }
        out.into_iter().collect()
    }
}

impl Default for EmbedderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience constructor used by tests: an [`EmbedderRegistry`]
/// pre-populated with one [`DummyEmbedder`] under the canonical
/// `urn:eigenius:embed:dummy:v1` IRI at 8-dim output. Kept small
/// (8 < `dim` < 16) so tests can print vectors during debugging.
pub fn registry_with_dummy() -> EmbedderRegistry {
    let mut reg = EmbedderRegistry::new();
    reg.register(Arc::new(DummyEmbedder::new(
        "urn:eigenius:embed:dummy:v1",
        8,
    )));
    reg
}

/// Deterministic reference embedder for tests. Produces a vector by
/// hashing `text` with blake3 and unpacking the digest into `dim`
/// `f32`s in `[-1.0, 1.0)`. Same input → byte-identical output, so
/// tests can assert on exact values; production embedders must not
/// rely on this property.
pub struct DummyEmbedder {
    model_iri: Iri,
    dim: u32,
}

impl DummyEmbedder {
    /// Construct a dummy embedder declaring the given IRI and
    /// dimensionality. Panics if the IRI doesn't parse — the call
    /// site is test-only, so the panic is the appropriate failure
    /// shape.
    pub fn new(iri: &str, dim: u32) -> Self {
        assert!(dim > 0, "DummyEmbedder dim must be > 0");
        Self {
            model_iri: Iri::parse(iri).expect("DummyEmbedder IRI must parse"),
            dim,
        }
    }
}

impl Embedder for DummyEmbedder {
    fn model_iri(&self) -> &Iri {
        &self.model_iri
    }

    fn dim(&self) -> u32 {
        self.dim
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedderError> {
        use sha2::{Digest, Sha256};
        if text.is_empty() {
            return Err(EmbedderError::InvalidInput(
                "DummyEmbedder: empty input".into(),
            ));
        }
        // SHA-256 produces a 32-byte digest per round. Rehash with a
        // counter prefix to fill any `dim` deterministically. Each
        // 4-byte window is reinterpreted as a `u32` and normalised
        // into `[-1.0, 1.0)`.
        let mut out = Vec::with_capacity(self.dim as usize);
        let mut counter: u32 = 0;
        let dim = self.dim as usize;
        while out.len() < dim {
            let mut hasher = Sha256::new();
            hasher.update(text.as_bytes());
            hasher.update(counter.to_le_bytes());
            let digest = hasher.finalize();
            for chunk in digest.chunks_exact(4) {
                if out.len() == dim {
                    break;
                }
                let u = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                let scaled = (u as f32 / u32::MAX as f32) * 2.0 - 1.0;
                out.push(scaled);
            }
            counter += 1;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    #[test]
    fn dummy_embedder_returns_declared_dim() {
        let e = DummyEmbedder::new("urn:eigenius:embed:dummy:v1", 8);
        let v = e.embed("hello").unwrap();
        assert_eq!(v.len(), 8);
        assert_eq!(e.dim(), 8);
    }

    #[test]
    fn dummy_embedder_is_deterministic() {
        let e = DummyEmbedder::new("urn:eigenius:embed:dummy:v1", 16);
        let a = e.embed("the quick brown fox").unwrap();
        let b = e.embed("the quick brown fox").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn dummy_embedder_differs_on_different_inputs() {
        let e = DummyEmbedder::new("urn:eigenius:embed:dummy:v1", 8);
        let a = e.embed("alpha").unwrap();
        let b = e.embed("beta").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn dummy_embedder_rejects_empty_input() {
        let e = DummyEmbedder::new("urn:eigenius:embed:dummy:v1", 8);
        match e.embed("") {
            Err(EmbedderError::InvalidInput(_)) => (),
            other => panic!("expected InvalidInput; got {other:?}"),
        }
    }

    #[test]
    fn dummy_embedder_values_in_unit_interval() {
        let e = DummyEmbedder::new("urn:eigenius:embed:dummy:v1", 32);
        let v = e.embed("anything").unwrap();
        assert!(v.iter().all(|f| (-1.0..1.0).contains(f)));
    }

    #[test]
    fn registry_register_and_get_round_trips() {
        let mut reg = EmbedderRegistry::new();
        let model = "urn:eigenius:embed:test:m1";
        reg.register(Arc::new(DummyEmbedder::new(model, 4)));
        let got = reg.get(&iri(model)).expect("should be registered");
        assert_eq!(got.dim(), 4);
        assert_eq!(got.model_iri().as_str(), model);
    }

    #[test]
    fn registry_get_unknown_returns_none() {
        let reg = EmbedderRegistry::new();
        assert!(reg.get(&iri("urn:eigenius:embed:missing")).is_none());
    }

    #[test]
    fn registry_parent_lookup_works() {
        let mut parent = EmbedderRegistry::new();
        parent.register(Arc::new(DummyEmbedder::new(
            "urn:eigenius:embed:parent:m1",
            4,
        )));
        let child = EmbedderRegistry::new_with_parent(Arc::new(parent));
        let got = child.get(&iri("urn:eigenius:embed:parent:m1"));
        assert!(got.is_some(), "child should see parent's embedder");
    }

    #[test]
    fn registry_local_shadows_parent() {
        let mut parent = EmbedderRegistry::new();
        let model = "urn:eigenius:embed:shadow:m1";
        parent.register(Arc::new(DummyEmbedder::new(model, 4)));
        let mut child = EmbedderRegistry::new_with_parent(Arc::new(parent));
        // Re-register the same IRI at the child with a different dim;
        // the child's lookup must hit the local entry first.
        child.register(Arc::new(DummyEmbedder::new(model, 16)));
        assert_eq!(child.get(&iri(model)).unwrap().dim(), 16);
    }

    #[test]
    fn registry_list_includes_inherited_entries() {
        let mut parent = EmbedderRegistry::new();
        parent.register(Arc::new(DummyEmbedder::new("urn:eigenius:embed:p:a", 4)));
        let mut child = EmbedderRegistry::new_with_parent(Arc::new(parent));
        child.register(Arc::new(DummyEmbedder::new("urn:eigenius:embed:c:b", 4)));
        let list: Vec<String> = child
            .list()
            .iter()
            .map(|i| i.as_str().to_string())
            .collect();
        assert!(list.contains(&"urn:eigenius:embed:p:a".to_string()));
        assert!(list.contains(&"urn:eigenius:embed:c:b".to_string()));
    }

    #[test]
    fn registry_with_dummy_constructor_works() {
        let reg = registry_with_dummy();
        assert!(reg
            .get(&iri("urn:eigenius:embed:dummy:v1"))
            .is_some_and(|e| e.dim() == 8));
    }

    /// The default `embed_batch` impl must return per-text results
    /// in input order and each be byte-identical to the
    /// per-text `embed` call. This pins the trait contract any
    /// override (e.g. the Candle batched path) has to honour.
    #[test]
    fn embed_batch_default_impl_matches_per_text_embed() {
        let e = DummyEmbedder::new("urn:eigenius:embed:dummy:v1", 16);
        let texts = ["alpha", "beta gamma", "the quick brown fox"];
        let batched = e.embed_batch(&texts).unwrap();
        assert_eq!(batched.len(), texts.len());
        for (i, t) in texts.iter().enumerate() {
            let single = e.embed(t).unwrap();
            assert_eq!(batched[i], single, "row {i} ({t:?}) mismatched");
        }
    }

    /// Empty batch must yield empty output without dispatching to the
    /// embedder — sweep's chunked dispatch relies on this for the
    /// degenerate final-chunk case.
    #[test]
    fn embed_batch_empty_input_returns_empty_output() {
        let e = DummyEmbedder::new("urn:eigenius:embed:dummy:v1", 8);
        let out = e.embed_batch(&[]).unwrap();
        assert!(out.is_empty());
    }

    /// One bad input in a batch must surface its `InvalidInput`
    /// diagnostic — the default impl propagates the first per-text
    /// error eagerly, which the sweep relies on for attribution
    /// (the sweep's `embed_batch_with_retry` then falls back to
    /// per-text dispatch to localise which subject is broken).
    #[test]
    fn embed_batch_default_impl_propagates_per_text_error() {
        let e = DummyEmbedder::new("urn:eigenius:embed:dummy:v1", 8);
        let texts = ["ok one", "", "ok two"];
        match e.embed_batch(&texts) {
            Err(EmbedderError::InvalidInput(_)) => (),
            other => panic!("expected InvalidInput from empty entry; got {other:?}"),
        }
    }
}
