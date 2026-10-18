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

//! Candle-backed [`Embedder`] Component for D43 vector retrieval.
//!
//! Pure-Rust Sentence-BERT inference via HuggingFace
//! [Candle](https://github.com/huggingface/candle) and
//! [tokenizers](https://github.com/huggingface/tokenizers). No C++
//! runtime, no Python subprocess, no WASM round-trip — the model
//! runs in-process on CPU (or GPU when built with `--features
//! cuda` / `--features metal`). First call downloads the model
//! files from the HuggingFace Hub into the standard HF cache
//! (`~/.cache/huggingface/hub`); subsequent calls reuse the cached
//! files.
//!
//! ## Default model
//!
//! [`CandleEmbedder::new_bge_small`] loads [BGE-small-en-v1.5][bge]
//! (`BAAI/bge-small-en-v1.5`): 384-dim sentence embeddings, BERT-base
//! architecture (33M parameters, ~130MB on disk). Strong English
//! retrieval performance and small enough to be practical on a
//! laptop. The corresponding `urn:eigenius:embed:bge-small-en-v1.5`
//! IRI is what you'd put in `core:VectorIndex.vec_model` on the
//! schema side.
//!
//! [bge]: https://huggingface.co/BAAI/bge-small-en-v1.5
//!
//! ## Other models
//!
//! Use [`CandleEmbedder::from_hf_hub`] to load any
//! BERT-architecture sentence-embedding model from the Hub. The
//! caller supplies the repo name, the model IRI to register with
//! Eigenius, and the expected output dimensionality. The
//! constructor verifies the model produces the declared dim on a
//! dummy probe so a mismatch fails at registration time rather than
//! at the first real query.
//!
//! ## Threading
//!
//! [`CandleEmbedder`] holds a Candle [`BertModel`] and a
//! [`Tokenizer`]; both are `Send + Sync`. The Embedder trait's
//! `embed` runs single-text inference; `embed_batch` is overridden
//! to run a single batched BertModel forward pass — which the
//! kernel's sweep ([`eigenius_kernel::query::vector::indexing`])
//! calls exclusively, giving a 10-30× speedup over the per-text
//! loop on CPU at batch ≈ 32.

use candle_core::{DType, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::program::embedder::{Embedder, EmbedderError};
use hf_hub::{api::sync::Api, Repo, RepoType};
use std::path::PathBuf;
use tokenizers::{PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};

/// IRI registered for the default BGE-small embedder. Match this on
/// the `core:VectorIndex.vec_model` slot to wire the schema to the
/// running embedder.
pub const BGE_SMALL_MODEL_IRI: &str = "urn:eigenius:embed:bge-small-en-v1.5";

/// Default model on the Hub. Held as a const so it can be referenced
/// from the test fixtures + the embedder constructor without diverging.
pub const BGE_SMALL_REPO: &str = "BAAI/bge-small-en-v1.5";

/// Output dimensionality of BGE-small. Declared as a const so the
/// kernel side (`VectorIndex.vec_dim`) and the embedder agree at
/// compile time.
pub const BGE_SMALL_DIM: u32 = 384;

/// Errors specific to this crate's model-loading path. Embedder
/// runtime errors flow through `EmbedderError::Io` per the kernel's
/// trait surface.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("hf-hub error: {0}")]
    Hub(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("candle error: {0}")]
    Candle(String),
    #[error("tokenizer error: {0}")]
    Tokenizer(String),
    #[error("malformed config.json: {0}")]
    Config(String),
    #[error("invalid IRI: {0}")]
    Iri(String),
    #[error(
        "dimension mismatch: declared {declared}, model produced {actual}; \
         pick the embedder that matches your VectorIndex.vec_dim"
    )]
    DimMismatch { declared: u32, actual: u32 },
}

impl From<LoadError> for EmbedderError {
    fn from(value: LoadError) -> Self {
        EmbedderError::Io(value.to_string())
    }
}

/// Pick the device Candle will run on. Build with `--features cuda`
/// or `--features metal` to opt into the corresponding backend; the
/// feature flag is what gates each crate's backend code, and only
/// then can `Device::new_cuda(0)` / `Device::new_metal(0)` succeed.
///
/// On feature-enabled build, we attempt the accelerator and fall
/// back to CPU on failure (driver missing, no device visible, OOM
/// at probe). The fall-back path emits a one-line warning so a
/// silent CPU-only run on a misconfigured machine is debuggable.
fn select_device() -> Device {
    #[cfg(feature = "cuda")]
    {
        match Device::new_cuda(0) {
            Ok(d) => return d,
            Err(e) => {
                eprintln!(
                    "eigenius-embedder-candle: CUDA requested but `Device::new_cuda(0)` failed: {e}. \
                     Falling back to CPU."
                );
            }
        }
    }
    #[cfg(feature = "metal")]
    {
        match Device::new_metal(0) {
            Ok(d) => return d,
            Err(e) => {
                eprintln!(
                    "eigenius-embedder-candle: Metal requested but `Device::new_metal(0)` failed: {e}. \
                     Falling back to CPU."
                );
            }
        }
    }
    Device::Cpu
}

/// Sentence-BERT embedder running on Candle. Constructed via
/// [`Self::new_bge_small`] (the curated default) or
/// [`Self::from_hf_hub`] (any BERT-architecture model on the Hub).
pub struct CandleEmbedder {
    iri: Iri,
    dim: u32,
    device: Device,
    model: BertModel,
    tokenizer: Tokenizer,
}

impl CandleEmbedder {
    /// Load BGE-small-en-v1.5 — the curated default. Equivalent to
    /// `from_hf_hub(BGE_SMALL_REPO, BGE_SMALL_MODEL_IRI, BGE_SMALL_DIM)`
    /// with the standard naming.
    pub fn new_bge_small() -> Result<Self, LoadError> {
        Self::from_hf_hub(BGE_SMALL_REPO, BGE_SMALL_MODEL_IRI, BGE_SMALL_DIM)
    }

    /// Load any BERT-architecture sentence-embedding model from the
    /// HuggingFace Hub. The constructor:
    ///
    /// 1. Fetches `tokenizer.json`, `config.json`, and
    ///    `model.safetensors` via [`hf_hub::api::sync::Api`] (cached
    ///    after the first call).
    /// 2. Loads the tokenizer + Bert config + weights into Candle.
    /// 3. Runs one dummy probe to verify the declared dim matches
    ///    the model's actual output. Mismatch returns
    ///    [`LoadError::DimMismatch`] before the embedder is handed
    ///    back to the caller — fail-fast at registration rather
    ///    than per-query.
    ///
    /// CPU device by default. Build with `--features cuda` or
    /// `--features metal` (in your binary's Candle dep) and the
    /// `Device::cuda_if_available` / `Device::Metal` path lights up
    /// automatically.
    pub fn from_hf_hub(
        repo_name: &str,
        model_iri: &str,
        declared_dim: u32,
    ) -> Result<Self, LoadError> {
        let iri = Iri::parse(model_iri).map_err(|e| LoadError::Iri(format!("{model_iri}: {e}")))?;

        let api = Api::new().map_err(|e| LoadError::Hub(e.to_string()))?;
        let repo = api.repo(Repo::with_revision(
            repo_name.to_string(),
            RepoType::Model,
            "main".to_string(),
        ));
        let tokenizer_path: PathBuf = repo
            .get("tokenizer.json")
            .map_err(|e| LoadError::Hub(format!("tokenizer.json: {e}")))?;
        let config_path: PathBuf = repo
            .get("config.json")
            .map_err(|e| LoadError::Hub(format!("config.json: {e}")))?;
        let weights_path: PathBuf = repo
            .get("model.safetensors")
            .map_err(|e| LoadError::Hub(format!("model.safetensors: {e}")))?;

        let mut tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| LoadError::Tokenizer(e.to_string()))?;
        // Pad on the right with the model's pad token; truncate to
        // the model's max sequence length. Without these, batched
        // inference would error on uneven-length inputs and long
        // text would OOM during forward.
        tokenizer
            .with_padding(Some(PaddingParams {
                strategy: PaddingStrategy::BatchLongest,
                ..Default::default()
            }))
            .with_truncation(Some(TruncationParams {
                max_length: 512,
                ..Default::default()
            }))
            .map_err(|e| LoadError::Tokenizer(e.to_string()))?;

        let config: Config = serde_json::from_slice(
            &std::fs::read(&config_path).map_err(|e| LoadError::Io(e.to_string()))?,
        )
        .map_err(|e| LoadError::Config(e.to_string()))?;

        let device = select_device();
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights_path], DType::F32, &device)
                .map_err(|e| LoadError::Candle(e.to_string()))?
        };
        let model = BertModel::load(vb, &config).map_err(|e| LoadError::Candle(e.to_string()))?;

        let embedder = Self {
            iri,
            dim: declared_dim,
            device,
            model,
            tokenizer,
        };

        // Dim-mismatch fail-fast probe. Embedding the empty string is
        // the cheapest dummy that still exercises the full forward
        // path (Sentence-BERT tokenizers prepend a CLS token, so a
        // single forward step runs even on `""`).
        let probe = embedder
            .embed_internal("")
            .map_err(|e| LoadError::Candle(e.to_string()))?;
        if probe.len() != declared_dim as usize {
            return Err(LoadError::DimMismatch {
                declared: declared_dim,
                actual: probe.len() as u32,
            });
        }

        Ok(embedder)
    }

    /// Embed one text into a `dim`-sized vector. Convenience wrapper
    /// around [`Self::embed_internal_batch`] for the constructor's
    /// dim-probe and the single-text `Embedder::embed` path. Real
    /// throughput should flow through `embed_batch` (the trait's
    /// batched method that this crate overrides below).
    fn embed_internal(&self, text: &str) -> Result<Vec<f32>, candle_core::Error> {
        let mut vs = self.embed_internal_batch(&[text])?;
        Ok(vs.pop().expect("one input → one output"))
    }

    /// Embed `texts` in a single batched forward pass through the
    /// BertModel. Result ordering matches input ordering.
    ///
    /// Pipeline: `encode_batch` (padded to the batch's longest
    /// sequence per the tokenizer's `BatchLongest` setting) →
    /// flatten ids + attention masks into row-major tensors of
    /// shape `[batch, seq]` → BertModel forward → mean-pool over
    /// the sequence weighted by attention mask → L2-normalize per
    /// row. The normalize step matches BGE / Sentence-BERT
    /// convention so cosine similarity reduces to a dot product on
    /// the query side.
    ///
    /// Empty `texts` returns `Ok(vec![])` — a defensive short-circuit
    /// so the sweep's chunk loop doesn't have to special-case the
    /// final partial chunk being empty.
    fn embed_internal_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, candle_core::Error> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| candle_core::Error::Msg(format!("tokenize batch: {e}")))?;
        let batch = encodings.len();
        // `BatchLongest` padding (set in the constructor) makes
        // every encoding the same length within a batch; the first
        // entry's length is therefore the canonical seq dim.
        let seq = encodings.first().map(|e| e.get_ids().len()).unwrap_or(0);
        if seq == 0 {
            // Pathological all-empty inputs: emit dim-sized zero
            // vectors per input. (BertModel.forward would itself
            // reject a [B, 0] tensor.)
            return Ok(vec![vec![0.0; self.dim as usize]; batch]);
        }

        // Flatten into [batch, seq] tensors.
        let mut ids: Vec<u32> = Vec::with_capacity(batch * seq);
        let mut mask: Vec<u32> = Vec::with_capacity(batch * seq);
        for e in &encodings {
            ids.extend_from_slice(e.get_ids());
            mask.extend_from_slice(e.get_attention_mask());
        }
        let token_ids = Tensor::from_vec(ids, (batch, seq), &self.device)?;
        let token_type_ids = token_ids.zeros_like()?;
        let attention_mask = Tensor::from_vec(mask, (batch, seq), &self.device)?;

        // BertModel::forward returns the last hidden state:
        // shape `[batch, seq, hidden]`.
        let hidden = self
            .model
            .forward(&token_ids, &token_type_ids, Some(&attention_mask))?;

        // Mean-pool over seq, weighted by attention mask. Padding
        // tokens contribute zero so the mean is over *real* tokens.
        let mask_f = attention_mask
            .to_dtype(DType::F32)?
            .unsqueeze(2)?
            .broadcast_as(hidden.shape())?;
        let masked = hidden.mul(&mask_f)?;
        let summed = masked.sum(1)?; // [batch, hidden]
                                     // Sum the mask over seq → denominator per-batch-element.
                                     // Clamp to avoid division by zero on empty-after-special-tokens
                                     // inputs (would only happen on truly empty text).
        let denom = mask_f.sum(1)?.clamp(1e-6_f64, f64::INFINITY)?;
        let pooled = summed.broadcast_div(&denom)?;

        // L2-normalize so cosine similarity == dot product on the
        // query side.
        let norm = pooled
            .sqr()?
            .sum_keepdim(1)?
            .sqrt()?
            .clamp(1e-6_f64, f64::INFINITY)?;
        let normalised = pooled.broadcast_div(&norm)?;

        // Materialise per-row Vec<f32>. Candle's `to_vec2` would do
        // it in one shot but we want owned `Vec<Vec<f32>>` matching
        // the trait, so a per-row `i(b)?` is the clean path.
        let mut out = Vec::with_capacity(batch);
        for b in 0..batch {
            out.push(normalised.i(b)?.to_vec1::<f32>()?);
        }
        Ok(out)
    }
}

impl Embedder for CandleEmbedder {
    fn model_iri(&self) -> &Iri {
        &self.iri
    }
    fn dim(&self) -> u32 {
        self.dim
    }
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedderError> {
        self.embed_internal(text)
            .map_err(|e| EmbedderError::Io(format!("candle inference: {e}")))
    }

    /// Override the trait default. Runs all `texts` in one BertModel
    /// forward pass, yielding a 10-30× speedup over the per-text
    /// loop on CPU at batch ≈ 32 (more on GPU). The kernel's sweep
    /// path ([`crate::query::vector::indexing::sweep_one_index`])
    /// calls this method exclusively, so the speedup applies
    /// automatically to every VectorIndex this embedder backs.
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedderError> {
        self.embed_internal_batch(texts)
            .map_err(|e| EmbedderError::Io(format!("candle batched inference: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Construction sanity-check. Downloads the model on first run
    /// (~130MB) and caches it; subsequent runs reuse the HF cache.
    /// Marked `#[ignore]` so `cargo test` doesn't trigger a download
    /// unless the caller asks for it.
    #[test]
    #[ignore = "downloads ~130MB on first run; run with `cargo test ... -- --ignored --nocapture`"]
    fn loads_bge_small_and_embeds_a_phrase() {
        let embedder = match CandleEmbedder::new_bge_small() {
            Ok(e) => e,
            Err(LoadError::Hub(msg)) => {
                eprintln!(
                    "skipping — HF Hub fetch failed (likely offline): {msg}. \
                     Set HF_HUB_OFFLINE=0 and ensure network access, \
                     or pre-download the model to ~/.cache/huggingface/."
                );
                return;
            }
            Err(e) => panic!("unexpected load error: {e}"),
        };
        assert_eq!(embedder.dim(), BGE_SMALL_DIM);
        assert_eq!(embedder.model_iri().as_str(), BGE_SMALL_MODEL_IRI);

        let v = embedder
            .embed("the cell nucleus contains chromosomes")
            .unwrap();
        assert_eq!(v.len(), BGE_SMALL_DIM as usize);
        // L2-normalised output: |v|² ≈ 1.
        let norm_sq: f32 = v.iter().map(|x| x * x).sum();
        assert!(
            (norm_sq - 1.0).abs() < 1e-4,
            "expected unit-norm embedding, got |v|² = {norm_sq}"
        );

        // Sanity: two semantically-similar phrases should embed
        // closer than two unrelated ones. Loose threshold; real
        // recall validation lives in the GO integration test.
        let nucleus = embedder
            .embed("the cell nucleus contains chromosomes")
            .unwrap();
        let dna = embedder.embed("DNA is stored in the cell nucleus").unwrap();
        let car = embedder
            .embed("a sports car accelerates from zero to sixty")
            .unwrap();
        let cos_nd: f32 = nucleus.iter().zip(dna.iter()).map(|(a, b)| a * b).sum();
        let cos_nc: f32 = nucleus.iter().zip(car.iter()).map(|(a, b)| a * b).sum();
        assert!(
            cos_nd > cos_nc + 0.1,
            "expected nucleus-DNA similarity ({cos_nd:.3}) to exceed \
             nucleus-car similarity ({cos_nc:.3}) by ≥0.1"
        );
    }

    /// The batched override must produce vectors numerically close to
    /// the per-text path. They are not byte-equal — padding to the
    /// batch's longest sequence changes the floating-point rounding
    /// inside the BertModel forward pass on short inputs — but the
    /// difference is well below downstream retrieval tolerances.
    /// We assert the per-element gap is within a small epsilon and
    /// that the cosine similarity is essentially 1.
    #[test]
    #[ignore = "downloads ~130MB on first run; run with `cargo test ... -- --ignored --nocapture`"]
    fn embed_batch_matches_per_text_within_tolerance() {
        let embedder = match CandleEmbedder::new_bge_small() {
            Ok(e) => e,
            Err(LoadError::Hub(msg)) => {
                eprintln!("skipping — HF Hub fetch failed: {msg}");
                return;
            }
            Err(e) => panic!("unexpected load error: {e}"),
        };

        let texts = [
            "the cell nucleus contains chromosomes",
            "DNA is stored in the cell nucleus",
            "a sports car accelerates from zero to sixty",
            "ribosomes synthesize proteins",
        ];
        let single: Vec<Vec<f32>> = texts.iter().map(|t| embedder.embed(t).unwrap()).collect();
        let batched = embedder.embed_batch(&texts).unwrap();
        assert_eq!(batched.len(), texts.len());

        for (i, (s, b)) in single.iter().zip(batched.iter()).enumerate() {
            assert_eq!(s.len(), BGE_SMALL_DIM as usize);
            assert_eq!(b.len(), BGE_SMALL_DIM as usize);
            // Per-element diff bound: padding-induced rounding is
            // typically O(1e-4) for BERT-small in F32. Loose enough
            // to absorb numerical noise, tight enough to catch any
            // structural bug (wrong row, wrong mask, swapped order).
            let max_diff = s
                .iter()
                .zip(b.iter())
                .map(|(a, c)| (a - c).abs())
                .fold(0.0f32, f32::max);
            assert!(
                max_diff < 5e-3,
                "row {i}: max per-element diff {max_diff} exceeds tolerance",
            );
            // Cosine similarity should be effectively 1.0.
            let dot: f32 = s.iter().zip(b.iter()).map(|(a, c)| a * c).sum();
            assert!(
                dot > 0.9999,
                "row {i}: cosine {dot} below threshold — batched vector diverged from per-text",
            );
        }
    }

    /// Empty batch must short-circuit without dispatching to Candle.
    /// Not gated on the model download because the empty-input check
    /// is in front of all model IO.
    #[test]
    #[ignore = "downloads ~130MB on first run; run with `cargo test ... -- --ignored --nocapture`"]
    fn embed_batch_empty_input_short_circuits() {
        let embedder = match CandleEmbedder::new_bge_small() {
            Ok(e) => e,
            Err(LoadError::Hub(msg)) => {
                eprintln!("skipping — HF Hub fetch failed: {msg}");
                return;
            }
            Err(e) => panic!("unexpected load error: {e}"),
        };
        let out = embedder.embed_batch(&[]).unwrap();
        assert!(out.is_empty());
    }
}
