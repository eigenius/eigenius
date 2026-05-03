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
//! The boundary uses Eigon-JSON strings (the same shape the kernel and
//! orchestrator already speak across the existing `ComponentExecutor`
//! gRPC dispatch). Inside the dispatcher, we parse to `Resource`,
//! resolve the language runtime, and drive the [`LanguageRuntime`]
//! trait. Output goes back as an Eigon-JSON string.
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
use crate::language_runtime::LanguageRuntime;
use crate::registry::{LanguageRuntimeRegistry, RegistryError};
use eigenius_kernel::ontology::eigon_json::{self, ParseError};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use thiserror::Error;

const PROP_LANGUAGE: &str = "urn:eigenius:runtime:language";
const PROP_REQUIRES_ENVIRONMENT: &str = "urn:eigenius:runtime:requires_environment";

/// Failure modes for [`SubstrateDispatcher::dispatch_run_runtime_script`]
/// and `dispatch_call_runtime_method`. Wraps lower-level errors with
/// boundary-codec failures (`InvalidJson`) and dispatch-table lookup
/// failures (`UnknownLanguage`).
#[derive(Debug, Error)]
pub enum FacadeError {
    #[error("invalid Eigon-JSON: {0}")]
    InvalidJson(String),

    #[error("argument is missing the required `{0}` property")]
    MissingProperty(&'static str),

    #[error("argument's `{prop}` property has wrong type: expected {expected}")]
    WrongPropertyType {
        prop: &'static str,
        expected: &'static str,
    },

    #[error("no LanguageRuntime registered for language `{0}`")]
    UnknownLanguage(String),

    #[error("output Resource could not be serialized: {0}")]
    SerializeOutput(String),

    #[error(transparent)]
    Run(#[from] RunError),
}

impl From<ParseError> for FacadeError {
    fn from(value: ParseError) -> Self {
        Self::InvalidJson(value.to_string())
    }
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
    /// - `input_json` — Eigon-JSON string for the input Resource that
    ///   flows through the pipeline. Forwarded as the single input to
    ///   the language runtime.
    /// - `argument_json` — Eigon-JSON string for the argument
    ///   Resource. In Phase 18a this carries the inline `RuntimeScript`
    ///   fields (language, source).
    ///
    /// Returns the output Resource serialised as Eigon-JSON.
    pub fn dispatch_run_runtime_script(
        &self,
        input_json: &str,
        argument_json: &str,
    ) -> Result<String, FacadeError> {
        let input = parse_resource(input_json)?;
        let argument = parse_resource(argument_json)?;
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
        let output = runtime.run_script(&worker, script, &[input])?;
        serialize_resource(&output)
    }

    /// Dispatch a `CallRuntimeMethod` invocation. Same pattern as
    /// `dispatch_run_runtime_script` but routes to
    /// `LanguageRuntime::call_method` with the argument as the
    /// `RuntimeMethodSignature`.
    pub fn dispatch_call_runtime_method(
        &self,
        input_json: &str,
        argument_json: &str,
    ) -> Result<String, FacadeError> {
        let input = parse_resource(input_json)?;
        let argument = parse_resource(argument_json)?;
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
        let output = runtime.call_method(&worker, signature, &[input])?;
        serialize_resource(&output)
    }
}

/// Accepts both the top-level shape (a JSON object) and an array
/// containing one resource. Empty input is treated as an embedded
/// Resource with no properties.
fn parse_resource(json: &str) -> Result<Resource, FacadeError> {
    let trimmed = json.trim();
    if trimmed.is_empty() || trimmed == "{}" {
        return Ok(Resource::new_embedded());
    }
    eigon_json::parse_embedded(trimmed).map_err(FacadeError::from)
}

fn serialize_resource(r: &Resource) -> Result<String, FacadeError> {
    let value = eigon_json::serialize_resource(r);
    serde_json::to_string(&value).map_err(|e| FacadeError::SerializeOutput(e.to_string()))
}

fn read_string_property(r: &Resource, prop_iri: &str) -> Result<String, FacadeError> {
    let iri = Iri::parse(prop_iri).map_err(|e| FacadeError::InvalidJson(e.to_string()))?;
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

    #[test]
    fn unknown_language_returns_typed_error() {
        let d = SubstrateDispatcher::new();
        let argument = r#"{"urn:eigenius:runtime:language":"not-registered"}"#;
        let err = d
            .dispatch_run_runtime_script("{}", argument)
            .expect_err("should fail for unknown language");
        assert!(
            matches!(err, FacadeError::UnknownLanguage(ref l) if l == "not-registered"),
            "got {err:?}"
        );
    }

    #[test]
    fn missing_language_returns_typed_error() {
        let d = SubstrateDispatcher::new();
        let argument = r#"{"urn:eigenius:runtime:source":"echo nope"}"#;
        let err = d
            .dispatch_run_runtime_script("{}", argument)
            .expect_err("should fail when language is missing");
        assert!(
            matches!(err, FacadeError::MissingProperty(p) if p == PROP_LANGUAGE),
            "got {err:?}"
        );
    }

    #[test]
    fn malformed_json_returns_invalid_json_error() {
        let d = SubstrateDispatcher::new();
        let err = d
            .dispatch_run_runtime_script("{}", "{not-json}")
            .expect_err("should fail on malformed JSON");
        assert!(matches!(err, FacadeError::InvalidJson(_)), "got {err:?}");
    }
}
