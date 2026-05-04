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

//! Substrate dispatch entry point — what the orchestrator's napi addon
//! calls when a `RunRuntimeScript` or `CallRuntimeMethod` IO component
//! lands.
//!
//! The boundary uses Eigon-CBOR bytes — the same codec the kernel ↔
//! orchestrator gRPC path uses post-Phase-18e and the same codec the
//! worker RPC uses (D26 §8.1). The orchestrator-side TS handler
//! receives JS objects from `component_executor.ts`, encodes them to
//! Eigon-CBOR via `wasm/cbor.ts` (the existing cbor-x ↔ ciborium
//! bridge), and hands the bytes to the addon. The addon forwards
//! straight into this facade. No JSON in the substrate's data path.
//!
//! ## Phase 18a scope
//!
//! - The `argument` carries the inline script fields (`language`,
//!   `source`) directly. Chain-resolved scripts and the boundary check
//!   (D26 §7.5) land in 18b.
//! - The substrate does not yet commit `RuntimeInvocation` provenance
//!   resources. The output is a plain Resource produced by the
//!   language runtime; provenance commit lands when chain interaction
//!   arrives in 18b/c.

use crate::error::RunError;
use crate::invocation::DispatchTrace;
use crate::language_runtime::LanguageRuntime;
use crate::registry::{LanguageRuntimeRegistry, RegistryError};
use crate::rpc::NumericalMetadata;
use crate::types::WorkerHandle;
use eigenius_kernel::ontology::eigon_cbor::{self, CborError};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use thiserror::Error;

const PROP_LANGUAGE: &str = "urn:eigenius:runtime:language";
const PROP_REQUIRES_ENVIRONMENT: &str = "urn:eigenius:runtime:requires_environment";

/// Failure modes for [`SubstrateDispatcher::dispatch_run_runtime_script`]
/// and `dispatch_call_runtime_method`. Wraps lower-level errors with
/// boundary-codec failures (`InvalidCbor`) and dispatch-table lookup
/// failures (`UnknownLanguage`).
#[derive(Debug, Error)]
pub enum FacadeError {
    #[error("invalid Eigon-CBOR: {0}")]
    InvalidCbor(String),

    #[error("argument is missing the required `{0}` property")]
    MissingProperty(&'static str),

    #[error("argument's `{prop}` property has wrong type: expected {expected}")]
    WrongPropertyType {
        prop: &'static str,
        expected: &'static str,
    },

    #[error("no LanguageRuntime registered for language `{0}`")]
    UnknownLanguage(String),

    #[error(transparent)]
    Run(#[from] RunError),
}

impl From<CborError> for FacadeError {
    fn from(value: CborError) -> Self {
        Self::InvalidCbor(value.to_string())
    }
}

/// Output of a substrate dispatch: the language runtime's output Resource
/// (Eigon-CBOR bytes) plus a partial `RuntimeInvocation` Resource
/// carrying the substrate-captured trace fields (Eigon-CBOR bytes).
///
/// Two artifacts because the orchestrator needs both: the output flows
/// downstream as the component's logical result; the partial invocation
/// gets completed (with `script` / `environment` / `inputs` / `output`
/// IRIs the orchestrator knows from its commit machinery) and committed
/// to the chain as provenance. See [`crate::invocation`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchOutcome {
    pub output_cbor: Vec<u8>,
    pub partial_invocation_cbor: Vec<u8>,
}

/// Substrate-side dispatcher. Holds the [`LanguageRuntimeRegistry`]
/// and exposes the two component entry points the napi addon calls.
#[derive(Default)]
pub struct SubstrateDispatcher {
    registry: LanguageRuntimeRegistry,
}

impl SubstrateDispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_language_runtime(
        &mut self,
        runtime: Box<dyn LanguageRuntime>,
    ) -> Result<(), RegistryError> {
        self.registry.register(runtime)
    }

    pub fn registry(&self) -> &LanguageRuntimeRegistry {
        &self.registry
    }

    /// Dispatch a `RunRuntimeScript` invocation.
    ///
    /// - `input_cbor` — Eigon-CBOR bytes for the input Resource that
    ///   flows through the pipeline. Forwarded as the single input to
    ///   the language runtime.
    /// - `argument_cbor` — Eigon-CBOR bytes for the argument Resource.
    ///   In Phase 18a this carries the inline `RuntimeScript` fields
    ///   (language, source).
    ///
    /// Returns the output Resource and partial `RuntimeInvocation`
    /// trace, both serialised as Eigon-CBOR. See [`DispatchOutcome`].
    pub fn dispatch_run_runtime_script(
        &self,
        input_cbor: &[u8],
        argument_cbor: &[u8],
    ) -> Result<DispatchOutcome, FacadeError> {
        let input = parse_resource(input_cbor)?;
        let argument = parse_resource(argument_cbor)?;
        let language = read_string_property(&argument, PROP_LANGUAGE)?;
        let runtime = self
            .registry
            .get(&language)
            .ok_or_else(|| FacadeError::UnknownLanguage(language.clone()))?;

        let env = synthesize_env(&language, &argument);
        // Phase 18a treats the argument as the script Resource — the
        // boundary check + full chain resolution land in 18b/c.
        let script = &argument;

        let worker = runtime.spawn_worker(&env, None).map_err(|e| {
            FacadeError::Run(RunError::WorkerRpcFailed(format!("spawn_worker: {e}")))
        })?;
        let (numerical_metadata, image_digest) = capture_health(runtime, &worker);
        let started_at = DispatchTrace::now_rfc3339();
        let output = runtime.run_script(&worker, script, &[input])?;
        let completed_at = DispatchTrace::now_rfc3339();
        Ok(build_outcome(
            &output,
            &language,
            image_digest,
            started_at,
            completed_at,
            numerical_metadata,
        ))
    }

    /// Dispatch a `CallRuntimeMethod` invocation. Same pattern as
    /// `dispatch_run_runtime_script` but routes to
    /// `LanguageRuntime::call_method` with the argument as the
    /// `RuntimeMethodSignature`.
    pub fn dispatch_call_runtime_method(
        &self,
        input_cbor: &[u8],
        argument_cbor: &[u8],
    ) -> Result<DispatchOutcome, FacadeError> {
        let input = parse_resource(input_cbor)?;
        let argument = parse_resource(argument_cbor)?;
        let language = read_string_property(&argument, PROP_LANGUAGE)?;
        let runtime = self
            .registry
            .get(&language)
            .ok_or_else(|| FacadeError::UnknownLanguage(language.clone()))?;

        let env = synthesize_env(&language, &argument);
        let signature = &argument;

        let worker = runtime.spawn_worker(&env, None).map_err(|e| {
            FacadeError::Run(RunError::WorkerRpcFailed(format!("spawn_worker: {e}")))
        })?;
        let (numerical_metadata, image_digest) = capture_health(runtime, &worker);
        let started_at = DispatchTrace::now_rfc3339();
        let output = runtime.call_method(&worker, signature, &[input])?;
        let completed_at = DispatchTrace::now_rfc3339();
        Ok(build_outcome(
            &output,
            &language,
            image_digest,
            started_at,
            completed_at,
            numerical_metadata,
        ))
    }
}

/// Best-effort `Health` round-trip. Returns the substrate-relevant
/// fields from `HealthInfo`: the worker's reported `numerical_metadata`
/// and the in-image image digest (`env_digest_in_image`). A failure
/// here logs to stderr and yields empty fields rather than failing the
/// dispatch — trace integrity is best-effort, the dispatch contract is
/// not. Phase 18c.5 / D26 §5.5.
fn capture_health(
    runtime: &dyn LanguageRuntime,
    worker: &WorkerHandle,
) -> (NumericalMetadata, Option<crate::types::ImageDigest>) {
    match runtime.query_health(worker) {
        Ok(info) => {
            let digest = info
                .env_digest_in_image
                .as_deref()
                .and_then(|s| crate::types::ImageDigest::parse(s).ok());
            (info.numerical_metadata, digest)
        }
        Err(e) => {
            eprintln!(
                "eigenius-runtime-substrate: query_health failed for worker {} ({}): {e}; \
                 dispatch will continue with empty trace fields",
                worker.id, worker.backend
            );
            (NumericalMetadata::default(), None)
        }
    }
}

fn build_outcome(
    output: &Resource,
    language: &str,
    image_digest: Option<crate::types::ImageDigest>,
    started_at: String,
    completed_at: String,
    numerical_metadata: NumericalMetadata,
) -> DispatchOutcome {
    let trace = DispatchTrace {
        language: language.to_string(),
        image_digest,
        started_at,
        completed_at,
        numerical_metadata,
    };
    let partial = trace.into_partial_invocation();
    DispatchOutcome {
        output_cbor: eigon_cbor::serialize_resource(output),
        partial_invocation_cbor: eigon_cbor::serialize_resource(&partial),
    }
}

/// Empty input is treated as an embedded Resource with no properties
/// — convenience for callers that don't need to pass an input (e.g.
/// the smoke test runtime). Otherwise the bytes are parsed as a
/// CBOR-encoded Resource via the lenient parser (allows embedded
/// resources without `@id`, which is the natural shape for component
/// arguments).
fn parse_resource(cbor: &[u8]) -> Result<Resource, FacadeError> {
    if cbor.is_empty() {
        return Ok(Resource::new_embedded());
    }
    eigon_cbor::parse_resource_lenient(cbor).map_err(FacadeError::from)
}

fn read_string_property(r: &Resource, prop_iri: &str) -> Result<String, FacadeError> {
    let iri = Iri::parse(prop_iri).map_err(|e| FacadeError::InvalidCbor(e.to_string()))?;
    match r.get(&iri) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(_) => Err(FacadeError::WrongPropertyType {
            prop: leak_property_name(prop_iri),
            expected: "string",
        }),
        None => Err(FacadeError::MissingProperty(leak_property_name(prop_iri))),
    }
}

/// FacadeError variants take `&'static str` for stable diagnostics.
/// Property-IRI constants are already 'static; this helper exists to
/// pin that against accidental dynamic strings.
fn leak_property_name(prop_iri: &str) -> &'static str {
    match prop_iri {
        PROP_LANGUAGE => PROP_LANGUAGE,
        PROP_REQUIRES_ENVIRONMENT => PROP_REQUIRES_ENVIRONMENT,
        _ => "<unknown>",
    }
}

/// Synthesize an environment Resource for v1's spawn-per-invocation
/// model. The TestLanguageRuntime ignores it; per-language runtimes
/// (Phase 19+) will replace this with a chain-resolved env.
fn synthesize_env(language: &str, argument: &Resource) -> Resource {
    let mut env = Resource::new_embedded();
    env.set(
        Iri::parse(PROP_LANGUAGE).expect("static IRI"),
        Value::String(language.to_string()),
    );
    // If the argument referenced a real env IRI, carry it forward so
    // language runtimes that need it can fetch the digest later
    // (no-op for TestLanguageRuntime).
    let env_prop = Iri::parse(PROP_REQUIRES_ENVIRONMENT).expect("static IRI");
    if let Some(v) = argument.get(&env_prop) {
        env.set(env_prop, v.clone());
    }
    env
}

#[cfg(test)]
mod tests {
    //! Pure tests that don't need the test worker binary live here.
    //! End-to-end tests using `TestLanguageRuntime` live in
    //! `tests/facade_integration.rs` because the binary path is only
    //! available via env!() in integration test crates.
    use super::*;

    fn argument_with(properties: &[(&str, &str)]) -> Vec<u8> {
        let mut r = Resource::new_embedded();
        for (iri, value) in properties {
            r.set(Iri::parse(iri).unwrap(), Value::String(value.to_string()));
        }
        eigon_cbor::serialize_resource(&r)
    }

    #[test]
    fn unknown_language_returns_typed_error() {
        let d = SubstrateDispatcher::new();
        let argument = argument_with(&[("urn:eigenius:runtime:language", "not-registered")]);
        let err = d
            .dispatch_run_runtime_script(&[], &argument)
            .expect_err("should fail for unknown language");
        assert!(
            matches!(err, FacadeError::UnknownLanguage(ref l) if l == "not-registered"),
            "got {err:?}"
        );
    }

    #[test]
    fn missing_language_returns_typed_error() {
        let d = SubstrateDispatcher::new();
        let argument = argument_with(&[("urn:eigenius:runtime:source", "echo nope")]);
        let err = d
            .dispatch_run_runtime_script(&[], &argument)
            .expect_err("should fail when language is missing");
        assert!(
            matches!(err, FacadeError::MissingProperty(p) if p == PROP_LANGUAGE),
            "got {err:?}"
        );
    }

    #[test]
    fn malformed_cbor_returns_invalid_cbor_error() {
        let d = SubstrateDispatcher::new();
        // 0xff alone is the CBOR break stop-code outside an
        // indefinite-length context — not a valid top-level value.
        let err = d
            .dispatch_run_runtime_script(&[], &[0xff])
            .expect_err("should fail on malformed CBOR");
        assert!(matches!(err, FacadeError::InvalidCbor(_)), "got {err:?}");
    }
}
