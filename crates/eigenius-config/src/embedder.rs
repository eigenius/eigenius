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

//! Schema for the `[embedder]` section of `eigenius.toml`.
//!
//! Runtime tunables for the D43 vector-retrieval embedder pool that
//! the kernel service registers at startup. Build-time backend
//! selection (`--features cuda` / `--features metal` on the
//! consuming binary) is **orthogonal** to this config: the feature
//! flag decides which backends are compiled in; this config decides
//! which one to actually use at runtime. A CPU-only binary will
//! refuse `device = "cuda"`; a CUDA-built binary with `device =
//! "cpu"` runs CPU-only.

use serde::{Deserialize, Serialize};

/// Runtime configuration for the embedder pool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EmbedderConfig {
    /// Which built-in embedders to register at startup. Empty
    /// (default) → no embedders; vector retrieval is unavailable.
    /// The list entries name *built-in* embedder kinds compiled into
    /// the service binary; today the only registered kind is
    /// `"bge-small-en-v1.5"` (BGE-small via Candle, IRI
    /// `urn:eigenius:embed:bge-small-en-v1.5`). Unknown entries are
    /// rejected at config load.
    pub enabled: Vec<String>,

    /// Runtime device preference for embedders that can run on
    /// multiple backends. The build's feature flags decide which
    /// values are usable:
    ///
    /// - `"auto"` (default) — prefer the accelerator the binary was
    ///   compiled with (`cuda` → CUDA, `metal` → Metal, neither →
    ///   CPU); fall back to CPU on init failure.
    /// - `"cpu"` — force CPU even if the binary supports an
    ///   accelerator. Useful for debugging, shared-GPU contention,
    ///   or running on a host whose GPU is reserved for other work.
    /// - `"cuda"` — require CUDA. Fail at construction if the
    ///   binary wasn't built with `--features cuda` or no CUDA
    ///   device is visible.
    /// - `"metal"` — require Metal (Apple Silicon).
    pub device: DeviceSelection,

    /// Per-sweep batch size passed through to
    /// [`crate::query::vector::indexing::SweepOptions::batch_size`].
    /// 32 is the v1 default; tune up for GPU sweeps (memory permitting),
    /// down if peak RAM is a constraint.
    pub batch_size: usize,

    /// If `true` and any active VectorIndex Resource at the bootstrap
    /// / rehydrated head declares a `vec_model` IRI for which no
    /// embedder is registered, `start_server` refuses to start. If
    /// `false`, missing models surface as per-query errors instead.
    ///
    /// Defaults to `true` for production-shaped deployments — a
    /// service that quietly runs without the embedders its schema
    /// declares would be a silent correctness regression.
    pub fail_fast_on_missing_model: bool,
}

impl Default for EmbedderConfig {
    fn default() -> Self {
        Self {
            enabled: Vec::new(),
            device: DeviceSelection::Auto,
            batch_size: 32,
            fail_fast_on_missing_model: true,
        }
    }
}

/// Runtime device preference. See [`EmbedderConfig::device`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceSelection {
    #[default]
    Auto,
    Cpu,
    Cuda,
    Metal,
}

impl DeviceSelection {
    /// Parse a TOML string ("auto" | "cpu" | "cuda" | "metal").
    /// Used by the env-var loader where the value arrives as a
    /// `String` not a serde-typed enum.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "cpu" => Some(Self::Cpu),
            "cuda" => Some(Self::Cuda),
            "metal" => Some(Self::Metal),
            _ => None,
        }
    }

    /// Stable string for display in startup logs and error messages.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Cpu => "cpu",
            Self::Cuda => "cuda",
            Self::Metal => "metal",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_disabled() {
        let cfg = EmbedderConfig::default();
        assert!(cfg.enabled.is_empty());
        assert_eq!(cfg.device, DeviceSelection::Auto);
        assert_eq!(cfg.batch_size, 32);
        assert!(cfg.fail_fast_on_missing_model);
    }

    #[test]
    fn device_selection_parses_lowercase_and_uppercase() {
        for input in ["auto", "AUTO", "Auto"] {
            assert_eq!(DeviceSelection::parse(input), Some(DeviceSelection::Auto));
        }
        assert_eq!(DeviceSelection::parse("cpu"), Some(DeviceSelection::Cpu));
        assert_eq!(DeviceSelection::parse("cuda"), Some(DeviceSelection::Cuda));
        assert_eq!(
            DeviceSelection::parse("metal"),
            Some(DeviceSelection::Metal)
        );
        assert_eq!(DeviceSelection::parse("gpu"), None);
    }

    #[test]
    fn embedder_config_round_trips_via_toml() {
        let src = r#"
            enabled = ["bge-small-en-v1.5"]
            device = "cuda"
            batch_size = 64
            fail_fast_on_missing_model = false
        "#;
        let cfg: EmbedderConfig = toml::from_str(src).unwrap();
        assert_eq!(cfg.enabled, vec!["bge-small-en-v1.5".to_string()]);
        assert_eq!(cfg.device, DeviceSelection::Cuda);
        assert_eq!(cfg.batch_size, 64);
        assert!(!cfg.fail_fast_on_missing_model);
    }

    /// `#[serde(default)]` at every level lets a TOML file specify
    /// any subset of fields without explicit error.
    #[test]
    fn partial_toml_inherits_defaults_per_field() {
        let cfg: EmbedderConfig = toml::from_str(r#"enabled = ["bge-small-en-v1.5"]"#).unwrap();
        assert_eq!(cfg.enabled, vec!["bge-small-en-v1.5".to_string()]);
        assert_eq!(cfg.device, DeviceSelection::Auto);
        assert_eq!(cfg.batch_size, 32);
        assert!(cfg.fail_fast_on_missing_model);
    }
}
