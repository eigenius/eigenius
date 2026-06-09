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

//! Worker RPC message types per D26 §8.1.
//!
//! The five-verb protocol is encoded as serde-tagged enums so each
//! message round-trips through CBOR with a stable `verb` discriminator.
//! Payloads carrying Eigon resources keep them as opaque CBOR-encoded
//! byte strings ([`serde_bytes::ByteBuf`]) — the substrate's protocol
//! layer does not interpret resource bytes; the per-language worker
//! decodes them against its `RuntimePackageMirror`.

use serde::{Deserialize, Serialize};
use serde_bytes::ByteBuf;

/// Discriminator on [`Request::DispatchMethod`] selecting the worker's
/// dispatch path.
///
/// `Script` (the default for back-compat with the 19a.1 wire format)
/// treats `target` as a Julia source string and `eval`s it. `Method`
/// treats `target` as a [`crate::rpc::method::MethodInvocation`] and
/// performs typed multiple-dispatch over CBOR-decoded mirror struct
/// inputs — the path that lights up `CallRuntimeMethod` (D26 §4.1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    /// Script-eval path: `target` is CBOR-encoded source text the
    /// worker `eval`s. Output is the eval'd value, stringified.
    /// 18d capstone shape; preserved for `RunRuntimeScript`.
    #[default]
    Script,
    /// Method-call path: `target` is a CBOR-encoded
    /// [`MethodInvocation`](crate::rpc::method::MethodInvocation);
    /// `inputs` are CBOR-encoded mirror struct values; the worker
    /// decodes by `is_a`, dispatches via Julia multiple-dispatch, and
    /// returns the encoded result.
    Method,
}

/// A request from the substrate to a worker. Internal-tag layout: each
/// variant becomes a CBOR map with a `verb` key whose value is the
/// snake-case verb name, plus the variant's named fields at the same
/// level.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "verb", rename_all = "snake_case")]
pub enum Request {
    /// Liveness check + in-image cross-check (D26 §9.3). Workers
    /// answer with [`Response::Health`].
    Health,

    /// Boot the worker against the pinned environment. Called once
    /// per spawn (in v1's spawn-per-invocation model) or once per
    /// warm-worker reuse (Phase 19c).
    Instantiate {
        /// IRI of the `RuntimeEnvironment` resource.
        env_iri: String,
        /// Pinned image digest. `None` for `LocalSpawner`-only
        /// deployments where the env doesn't reference an image.
        image_digest: Option<String>,
    },

    /// Load a `RuntimePackageMirror`'s library archive into the
    /// runtime's package manager. Idempotent: re-registering the same
    /// mirror IRI is a no-op.
    RegisterMirror {
        mirror_iri: String,
        /// Mirror archive bytes (CBOR-encoded by the per-language
        /// generator — opaque to this protocol).
        library_content: ByteBuf,
    },

    /// Execute a script or method call. The substrate has already
    /// resolved the script / signature and inputs from the chain; this
    /// call passes the CBOR-encoded resources across the wire.
    DispatchMethod {
        /// Substrate-assigned correlation ID for the invocation.
        invocation_id: String,
        /// What `target` is — script source vs. method invocation
        /// directive. Defaults to `Script` for back-compat with the
        /// 19a.1 worker.
        #[serde(default)]
        target_kind: TargetKind,
        /// Payload whose meaning depends on `target_kind`.
        /// - `TargetKind::Script` — CBOR-encoded `String` carrying the
        ///   script source; the worker `eval`s it.
        /// - `TargetKind::Method` — CBOR-encoded
        ///   [`MethodInvocation`](crate::rpc::method::MethodInvocation)
        ///   carrying the function name + signature IRI; the worker
        ///   does typed multiple-dispatch on `inputs`.
        target: ByteBuf,
        /// CBOR-encoded input resources, in argument order. Each
        /// resource carries an `is_a` list the worker uses to dispatch
        /// to the matching mirror struct's decoder.
        inputs: Vec<ByteBuf>,
    },

    /// Graceful shutdown. The worker should drain in-flight work,
    /// release resources, and exit. Used by the warm-worker pool that
    /// lands in Phase 19c.
    Evict,
}

/// A response from a worker. Internally tagged the same way as
/// [`Request`] so the verb is the discriminator on both directions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "verb", rename_all = "snake_case")]
pub enum Response {
    /// Reply to [`Request::Health`].
    Health(HealthInfo),

    /// Reply to [`Request::Instantiate`]. `ready: true` means the
    /// worker is fully booted and accepting `dispatch_method` calls.
    Instantiated { ready: bool },

    /// Reply to [`Request::RegisterMirror`].
    MirrorRegistered { mirror_iri: String },

    /// Successful dispatch. The carried `output` is a CBOR-encoded
    /// Eigon resource that the substrate commits back to the chain
    /// alongside the `RuntimeInvocation` provenance record.
    DispatchOk {
        invocation_id: String,
        output: ByteBuf,
        /// Side-effect resources the language runtime emitted as
        /// artefacts of validation — each a CBOR-encoded Eigon
        /// resource that becomes a chain-resident
        /// `reflection:InstitutionEmittedDerivation` under the
        /// gated subject. Empty Vec for institutions whose only job
        /// is the pass/fail gate (D52 §6).
        #[serde(default)]
        derivations: Vec<ByteBuf>,
        /// The specific `RuntimeMethodSignature` IRI that handled the
        /// invocation (multiple-dispatch languages like Julia). Echoed
        /// onto `RuntimeInvocation.dispatched_to`.
        dispatched_to: Option<String>,
    },

    /// Failed dispatch. `error_kind` maps to a [`crate::error::RunError`]
    /// variant (e.g. `"runtime_error"`, `"sandbox_violation"`,
    /// `"resource_limit_exceeded"`); `message` carries the
    /// language-side diagnostic where available.
    DispatchFailed {
        invocation_id: String,
        error_kind: String,
        message: String,
    },

    /// Reply to [`Request::Evict`]. Sent immediately before the worker
    /// exits.
    Evicted,
}

/// Worker self-report bundling cross-check signals and the
/// `numerical_metadata` recorded on every `RuntimeInvocation`
/// (D26 §5.5 / §9.3).
///
/// Cross-check (D26 §9.3): the worker reports the digest and
/// manifest-hash it sees baked into its image at
/// `/etc/eigenius-runtime-env/`. The substrate compares these against
/// the values it passed via `EIGENIUS_RUNTIME_ENV_DIGEST` and
/// `EIGENIUS_RUNTIME_ENV_MANIFEST_HASH`. Disagreement triggers a
/// [`crate::error::SpawnError::WorkerCrossCheckFailed`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthInfo {
    /// Value of `/etc/eigenius-runtime-env/manifest-hash` as the
    /// worker sees it. Absent for `LocalSpawner` (no in-image
    /// provenance) and for misconfigured deployments.
    pub manifest_hash_in_image: Option<String>,
    /// Value of the `EIGENIUS_RUNTIME_ENV_DIGEST` env var as the
    /// worker received it. Echoed back so the substrate can confirm
    /// the worker actually saw the digest the substrate set.
    pub env_digest_in_image: Option<String>,
    /// Numerical-determinism context captured at worker bootstrap.
    pub numerical_metadata: NumericalMetadata,
}

/// Bits of the host environment that affect bit-identical
/// reproducibility but are not pinned by the image digest. D26 §5.5.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NumericalMetadata {
    pub blas_lib: Option<String>,
    pub blas_version: Option<String>,
    pub fma_enabled: Option<bool>,
    pub host_kernel: Option<String>,
    pub gpu_vendor: Option<String>,
    pub gpu_driver_version: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cbor_round_trip<T: Serialize + for<'de> Deserialize<'de>>(value: &T) -> T {
        let mut buf = Vec::new();
        ciborium::into_writer(value, &mut buf).expect("encode");
        ciborium::from_reader(&buf[..]).expect("decode")
    }

    #[test]
    fn health_request_round_trips() {
        let req = Request::Health;
        let after: Request = cbor_round_trip(&req);
        assert_eq!(req, after);
    }

    #[test]
    fn instantiate_request_round_trips() {
        let req = Request::Instantiate {
            env_iri: "urn:eigenius:test:env:julia-1.10".to_string(),
            image_digest: Some(
                "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                    .to_string(),
            ),
        };
        let after: Request = cbor_round_trip(&req);
        assert_eq!(req, after);
    }

    #[test]
    fn dispatch_method_request_round_trips() {
        let req = Request::DispatchMethod {
            invocation_id: "inv-42".to_string(),
            target_kind: TargetKind::Script,
            target: ByteBuf::from(vec![0xa0, 0x42, 0x01]),
            inputs: vec![ByteBuf::from(vec![0x01, 0x02]), ByteBuf::from(vec![0x03])],
        };
        let after: Request = cbor_round_trip(&req);
        assert_eq!(req, after);
    }

    #[test]
    fn dispatch_method_request_omits_target_kind_default() {
        // `target_kind` is `#[serde(default)]` so old workers receiving
        // payloads from a new substrate that omits the field still
        // decode correctly. Verify by encoding without target_kind and
        // decoding back to the new shape.
        let req = Request::DispatchMethod {
            invocation_id: "inv-old".to_string(),
            target_kind: TargetKind::Script,
            target: ByteBuf::from(vec![0xa0]),
            inputs: vec![],
        };
        let after: Request = cbor_round_trip(&req);
        match after {
            Request::DispatchMethod { target_kind, .. } => {
                assert_eq!(target_kind, TargetKind::Script);
            }
            other => panic!("expected DispatchMethod, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_method_request_with_target_kind_method_round_trips() {
        let req = Request::DispatchMethod {
            invocation_id: "inv-method".to_string(),
            target_kind: TargetKind::Method,
            target: ByteBuf::from(vec![0xa1]),
            inputs: vec![ByteBuf::from(vec![0x01])],
        };
        let after: Request = cbor_round_trip(&req);
        match after {
            Request::DispatchMethod { target_kind, .. } => {
                assert_eq!(target_kind, TargetKind::Method);
            }
            other => panic!("expected DispatchMethod, got {other:?}"),
        }
    }

    #[test]
    fn register_mirror_request_round_trips() {
        let req = Request::RegisterMirror {
            mirror_iri: "urn:eigenius:test:mirror:julia-core".to_string(),
            library_content: ByteBuf::from(vec![0xff; 16]),
        };
        let after: Request = cbor_round_trip(&req);
        assert_eq!(req, after);
    }

    #[test]
    fn evict_request_round_trips() {
        let req = Request::Evict;
        let after: Request = cbor_round_trip(&req);
        assert_eq!(req, after);
    }

    #[test]
    fn health_response_round_trips() {
        let resp = Response::Health(HealthInfo {
            manifest_hash_in_image: Some("abc123".to_string()),
            env_digest_in_image: Some("sha256:00".to_string()),
            numerical_metadata: NumericalMetadata {
                blas_lib: Some("openblas".to_string()),
                blas_version: Some("0.3.27".to_string()),
                fma_enabled: Some(true),
                host_kernel: Some("Linux 6.6.0".to_string()),
                gpu_vendor: None,
                gpu_driver_version: None,
            },
        });
        let after: Response = cbor_round_trip(&resp);
        assert_eq!(resp, after);
    }

    #[test]
    fn dispatch_ok_response_round_trips() {
        let resp = Response::DispatchOk {
            invocation_id: "inv-42".to_string(),
            output: ByteBuf::from(vec![0xff, 0xfe, 0xfd]),
            derivations: Vec::new(),
            dispatched_to: Some("urn:eigenius:test:method:foo".to_string()),
        };
        let after: Response = cbor_round_trip(&resp);
        assert_eq!(resp, after);
    }

    #[test]
    fn dispatch_failed_response_round_trips() {
        let resp = Response::DispatchFailed {
            invocation_id: "inv-42".to_string(),
            error_kind: "runtime_error".to_string(),
            message: "Julia: UndefVarError(:foo)".to_string(),
        };
        let after: Response = cbor_round_trip(&resp);
        assert_eq!(resp, after);
    }

    #[test]
    fn byte_payload_serializes_as_cbor_bytes_not_array() {
        // ByteBuf must encode as a CBOR byte-string (major type 2),
        // not a CBOR array of integers (major type 4) — that's the
        // only reason serde_bytes is in the dep graph.
        let req = Request::RegisterMirror {
            mirror_iri: "x".to_string(),
            library_content: ByteBuf::from(vec![0u8, 1, 2, 3]),
        };
        let mut buf = Vec::new();
        ciborium::into_writer(&req, &mut buf).expect("encode");
        // Find the byte string by length: 4 bytes preceded by the
        // CBOR major-type-2 length encoding. Major type 2 / length 4
        // is the single byte 0x44.
        assert!(
            buf.windows(5).any(|w| w == [0x44, 0x00, 0x01, 0x02, 0x03]),
            "expected CBOR byte-string `0x44 00 01 02 03` in encoded request, got {buf:02x?}"
        );
    }
}
