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

//! `RuntimeInvocation` provenance assembly (D26 §5.5 / Phase 18c.5).
//!
//! The substrate captures the invocation-shaped facts it observes
//! during a dispatch — language id, image digest, timestamps,
//! `numerical_metadata` from the worker's `Health` response — and
//! emits them as a *partial* `RuntimeInvocation` Resource via
//! [`DispatchTrace::into_partial_invocation`]. The Resource is partial
//! because the IRI-typed properties (`script`, `environment`, `inputs`,
//! `output`) reference resources that get committed by the orchestrator
//! /  kernel, not by the substrate. The caller — the orchestrator-side
//! TS handler that already has access to the kernel commit machinery —
//! fills those in before committing the invocation.
//!
//! ## Why partial
//!
//! The substrate is invoked *from* the orchestrator and never talks to
//! the kernel directly. So at dispatch time the substrate doesn't know
//! the IRI under which the orchestrator will commit the output Resource;
//! it can only hand back the trace fields it observed. The orchestrator
//! commits the output, learns its IRI, then completes and commits the
//! `RuntimeInvocation` referencing the new IRI.
//!
//! ## Why JSON for `numerical_metadata`
//!
//! The ontology declares [`urn:eigenius:runtime:numerical_metadata`]
//! with `data_type: json` (D26 §5.5) — the field is intentionally
//! schema-less so per-language runtimes can record whatever
//! reproducibility-affecting context they observe (FMA flag for Julia,
//! GPU UUID for CUDA-backed runtimes, kernel build for low-level
//! diagnostic tools) without churning the ontology.
//!
//! [`urn:eigenius:runtime:numerical_metadata`]: https://example.invalid/

use crate::rpc::NumericalMetadata;
use crate::types::ImageDigest;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use std::time::SystemTime;

/// Property IRI for `RuntimeInvocation.language`.
pub const PROP_LANGUAGE: &str = "urn:eigenius:runtime:language";
/// Property IRI for `RuntimeInvocation.image_digest`.
pub const PROP_IMAGE_DIGEST: &str = "urn:eigenius:runtime:image_digest";
/// Property IRI for `RuntimeInvocation.started_at`.
pub const PROP_STARTED_AT: &str = "urn:eigenius:runtime:started_at";
/// Property IRI for `RuntimeInvocation.completed_at`.
pub const PROP_COMPLETED_AT: &str = "urn:eigenius:runtime:completed_at";
/// Property IRI for `RuntimeInvocation.numerical_metadata`.
pub const PROP_NUMERICAL_METADATA: &str = "urn:eigenius:runtime:numerical_metadata";
/// Property IRI for `RuntimeInvocation.dispatched_to`. Left unset by
/// the substrate in 18c.5 — populated when `CallRuntimeMethod`'s
/// method-resolution lands in Phase 19a (see implementation plan
/// Phase 19a — "Wire `dispatched_to`").
pub const PROP_DISPATCHED_TO: &str = "urn:eigenius:runtime:dispatched_to";

/// Substrate-captured facts about one dispatch. Caller (orchestrator)
/// adds the IRI bindings (`script`, `environment`, `inputs`, `output`)
/// before committing the resulting `RuntimeInvocation` resource.
///
/// Field choice: every property here is something the substrate
/// uniquely knows. IRIs the orchestrator already has are deliberately
/// excluded — the trace is "what the substrate adds to the commit",
/// not "everything that goes on a `RuntimeInvocation`".
#[derive(Debug, Clone, PartialEq)]
pub struct DispatchTrace {
    /// `RuntimeEnvironment.language` echoed from the dispatch argument
    /// — recorded so audits can re-route to the right `LanguageRuntime`
    /// without re-reading the script resource.
    pub language: String,
    /// Image the worker ran against. `None` under `LocalSpawner`
    /// (deployment shape (c) per D26 §10.1) where there is no built
    /// image. Always `Some` under `DockerSpawner`.
    pub image_digest: Option<ImageDigest>,
    /// RFC3339 timestamp of dispatch start. Captured *before*
    /// `run_script` / `call_method` is called.
    pub started_at: String,
    /// RFC3339 timestamp of dispatch completion. Captured *after* the
    /// language runtime returns, including the failure path so error
    /// invocations carry honest timing.
    pub completed_at: String,
    /// What the worker reported via `Request::Health`. Empty
    /// `NumericalMetadata` is valid (the bash test runtime, for
    /// instance, only populates `host_kernel`); the substrate does not
    /// fail dispatch on missing health data.
    pub numerical_metadata: NumericalMetadata,
    /// Worker-reported `dispatched_to` — the resolved method signature
    /// (e.g. Julia's `which(...)` output) for `CallRuntimeMethod`.
    /// `None` for `RunRuntimeScript` and for runtimes that don't
    /// implement method dispatch (D26 §4.2).
    pub dispatched_to: Option<String>,
}

/// Outcome of a [`crate::language_runtime::LanguageRuntime`] dispatch.
/// Bundles the result resource with the trace fields the runtime is
/// uniquely positioned to know (timestamps, image digest the worker
/// actually ran against, numerical metadata the worker reported).
///
/// The runtime owns its dispatch lifecycle (spawn, attach, dispatch,
/// cleanup) and produces this struct at the end. The substrate facade
/// adds the language tag (which it already knows from the dispatch
/// argument) and assembles the partial `RuntimeInvocation`.
#[derive(Debug)]
pub struct RunOutcome {
    /// The output resource produced by the dispatch.
    pub output: eigenius_kernel::ontology::resource::Resource,
    /// Side-effect resources the language runtime emitted as artefacts
    /// of the dispatch (per D52 §6 / institution-emitted derivations).
    /// Each becomes a chain-resident
    /// `reflection:InstitutionEmittedDerivation` carrying its own
    /// `canonical_proposition`. Empty for substrate-hosted institutions
    /// whose only job is the pass/fail gate.
    pub derivations: Vec<eigenius_kernel::ontology::resource::Resource>,
    /// Image digest the worker actually ran against. `None` under
    /// host-subprocess backends with no built image.
    pub image_digest: Option<ImageDigest>,
    /// RFC3339 timestamp captured immediately before the worker-side
    /// dispatch began.
    pub started_at: String,
    /// RFC3339 timestamp captured immediately after the worker-side
    /// dispatch returned (success or failure).
    pub completed_at: String,
    /// Worker-reported numerical metadata (Health RPC). Empty is valid.
    pub numerical_metadata: NumericalMetadata,
    /// Worker-reported `dispatched_to` — the resolved method signature
    /// for `CallRuntimeMethod`. `None` for `RunRuntimeScript` and for
    /// runtimes that don't implement method dispatch (Phase 19a.4
    /// lights this up for the Julia runtime).
    pub dispatched_to: Option<String>,
}

impl DispatchTrace {
    /// Format a `SystemTime` as RFC3339 with millisecond precision and
    /// the `Z` (UTC) suffix — the standard timestamp shape across the
    /// rest of Eigenius.
    pub fn now_rfc3339() -> String {
        humantime::format_rfc3339_millis(SystemTime::now()).to_string()
    }

    /// Build the partial `RuntimeInvocation` Resource carrying the
    /// substrate-captured trace. The result is an *embedded* Resource
    /// (no `@id`) — the orchestrator assigns the IRI when it commits.
    /// Required IRI-typed properties (`script`, `environment`,
    /// `inputs`, `output`) are deliberately absent; caller adds them
    /// before commit. The kernel will reject a commit attempt that's
    /// missing any required property — that rejection is the contract.
    pub fn into_partial_invocation(self) -> Resource {
        let mut r = Resource::new_embedded();
        r.set(parse_iri(PROP_LANGUAGE), Value::String(self.language));
        if let Some(digest) = self.image_digest {
            r.set(
                parse_iri(PROP_IMAGE_DIGEST),
                Value::String(digest.as_str().to_string()),
            );
        }
        r.set(parse_iri(PROP_STARTED_AT), Value::String(self.started_at));
        r.set(
            parse_iri(PROP_COMPLETED_AT),
            Value::String(self.completed_at),
        );
        r.set(
            parse_iri(PROP_NUMERICAL_METADATA),
            Value::Json(numerical_metadata_to_json(&self.numerical_metadata)),
        );
        if let Some(dt) = self.dispatched_to {
            r.set(parse_iri(PROP_DISPATCHED_TO), Value::String(dt));
        }
        r
    }
}

/// Serialise a [`NumericalMetadata`] as a JSON object suitable for the
/// `data_type: json` property value. Skips `None` fields rather than
/// emitting `null` so the on-graph value carries only what the worker
/// actually reported.
pub fn numerical_metadata_to_json(m: &NumericalMetadata) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    if let Some(v) = &m.blas_lib {
        obj.insert("blas_lib".to_string(), serde_json::Value::String(v.clone()));
    }
    if let Some(v) = &m.blas_version {
        obj.insert(
            "blas_version".to_string(),
            serde_json::Value::String(v.clone()),
        );
    }
    if let Some(v) = m.fma_enabled {
        obj.insert("fma_enabled".to_string(), serde_json::Value::Bool(v));
    }
    if let Some(v) = &m.host_kernel {
        obj.insert(
            "host_kernel".to_string(),
            serde_json::Value::String(v.clone()),
        );
    }
    if let Some(v) = &m.gpu_vendor {
        obj.insert(
            "gpu_vendor".to_string(),
            serde_json::Value::String(v.clone()),
        );
    }
    if let Some(v) = &m.gpu_driver_version {
        obj.insert(
            "gpu_driver_version".to_string(),
            serde_json::Value::String(v.clone()),
        );
    }
    serde_json::Value::Object(obj)
}

fn parse_iri(s: &str) -> Iri {
    Iri::parse(s).expect("static substrate IRI must parse")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_digest() -> ImageDigest {
        ImageDigest::parse(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("digest parses")
    }

    fn sample_trace() -> DispatchTrace {
        DispatchTrace {
            language: "test".into(),
            image_digest: Some(dummy_digest()),
            started_at: "2026-05-03T12:00:00.000Z".into(),
            completed_at: "2026-05-03T12:00:00.250Z".into(),
            numerical_metadata: NumericalMetadata {
                host_kernel: Some("test-runtime".into()),
                fma_enabled: Some(true),
                ..Default::default()
            },
            dispatched_to: None,
        }
    }

    #[test]
    fn now_rfc3339_emits_z_suffix_and_millis() {
        let s = DispatchTrace::now_rfc3339();
        assert!(s.ends_with('Z'), "expected Z suffix, got {s}");
        // RFC3339 millis form: YYYY-MM-DDTHH:MM:SS.mmmZ — 24 chars.
        assert_eq!(s.len(), 24, "unexpected RFC3339 length: {s}");
    }

    #[test]
    fn into_partial_invocation_carries_all_substrate_known_fields() {
        let r = sample_trace().into_partial_invocation();
        assert!(r.id().is_none(), "partial invocation must be embedded");
        assert_eq!(
            r.get(&parse_iri(PROP_LANGUAGE)).and_then(Value::as_str),
            Some("test")
        );
        assert_eq!(
            r.get(&parse_iri(PROP_IMAGE_DIGEST)).and_then(Value::as_str),
            Some(dummy_digest().as_str())
        );
        assert_eq!(
            r.get(&parse_iri(PROP_STARTED_AT)).and_then(Value::as_str),
            Some("2026-05-03T12:00:00.000Z")
        );
        assert_eq!(
            r.get(&parse_iri(PROP_COMPLETED_AT)).and_then(Value::as_str),
            Some("2026-05-03T12:00:00.250Z")
        );
    }

    #[test]
    fn into_partial_invocation_skips_image_digest_when_absent() {
        let mut t = sample_trace();
        t.image_digest = None;
        let r = t.into_partial_invocation();
        assert!(r.get(&parse_iri(PROP_IMAGE_DIGEST)).is_none());
    }

    #[test]
    fn into_partial_invocation_omits_dispatched_to_in_phase_18c5() {
        // dispatched_to is left unset until Phase 19a wires
        // CallRuntimeMethod's resolved RuntimeMethodSignature through.
        // Regression guard against a well-meaning future edit.
        let r = sample_trace().into_partial_invocation();
        assert!(r.get(&parse_iri(PROP_DISPATCHED_TO)).is_none());
    }

    #[test]
    fn numerical_metadata_to_json_omits_none_fields() {
        let m = NumericalMetadata {
            host_kernel: Some("linux-6.6".into()),
            ..Default::default()
        };
        let json = numerical_metadata_to_json(&m);
        let obj = json.as_object().expect("object");
        assert_eq!(obj.len(), 1, "only set fields should appear");
        assert_eq!(
            obj.get("host_kernel").and_then(serde_json::Value::as_str),
            Some("linux-6.6")
        );
        assert!(obj.get("blas_lib").is_none());
    }

    #[test]
    fn numerical_metadata_to_json_carries_all_set_fields() {
        let m = NumericalMetadata {
            blas_lib: Some("openblas".into()),
            blas_version: Some("0.3.27".into()),
            fma_enabled: Some(true),
            host_kernel: Some("linux-6.6".into()),
            gpu_vendor: Some("NVIDIA".into()),
            gpu_driver_version: Some("550.78".into()),
        };
        let json = numerical_metadata_to_json(&m);
        let obj = json.as_object().expect("object");
        assert_eq!(obj.len(), 6);
        assert_eq!(
            obj.get("fma_enabled").and_then(serde_json::Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn numerical_metadata_value_serialises_into_resource_property() {
        let r = sample_trace().into_partial_invocation();
        let val = r
            .get(&parse_iri(PROP_NUMERICAL_METADATA))
            .expect("numerical_metadata present");
        match val {
            Value::Json(v) => {
                assert_eq!(
                    v.get("host_kernel").and_then(serde_json::Value::as_str),
                    Some("test-runtime")
                );
                assert_eq!(
                    v.get("fma_enabled").and_then(serde_json::Value::as_bool),
                    Some(true)
                );
            }
            other => panic!("expected Value::Json, got {other:?}"),
        }
    }
}
