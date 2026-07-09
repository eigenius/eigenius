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
//! Eigon-CBOR via `codec/cbor.ts` (the cbor-x ↔ ciborium
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
use crate::invocation::{DispatchTrace, RunOutcome};
use crate::language_runtime::LanguageRuntime;
use crate::registry::{LanguageRuntimeRegistry, RegistryError};
use eigenius_kernel::ontology::eigon_cbor::{self, CborError};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use thiserror::Error;

const PROP_LANGUAGE: &str = "urn:eigenius:runtime:language";
const PROP_REQUIRES_ENVIRONMENT: &str = "urn:eigenius:runtime:requires_environment";
const PROP_IMAGE_DIGEST: &str = "urn:eigenius:runtime:image_digest";
const PROP_METHOD_NAME: &str = "urn:eigenius:runtime:method_name";

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
    /// Side-effect resources the language runtime emitted as artefacts
    /// of dispatch — each Eigon-CBOR-encoded. The kernel's
    /// `ExternalInstitution::query` decodes these into
    /// `QueryOutcome.derivations`, which the commit pipeline then
    /// emits as chain-resident
    /// `reflection:InstitutionEmittedDerivation` resources under the
    /// gated subject (D52 §6).
    pub derivations_cbor: Vec<Vec<u8>>,
    pub partial_invocation_cbor: Vec<u8>,
}

/// Substrate-side dispatcher. Holds the [`LanguageRuntimeRegistry`]
/// and exposes the two component entry points the napi addon calls.
#[derive(Default)]
pub struct SubstrateDispatcher {
    registry: LanguageRuntimeRegistry,
    /// Root under which `PinnedExternalFile` inputs are materialized
    /// (D53 §7). Set to a directory **under the depot** for containerized
    /// (Docker) deployments so the depot's read-only bind-mount makes the
    /// bytes visible to the worker at the same path. Left `None` for the
    /// same-host (local) spawner, where the verified source path is handed
    /// to the worker directly. See [`crate::external_file::prepare_input`].
    extfile_cache_root: Option<std::path::PathBuf>,
    /// Reject node-local `file://` inputs (D53 §3.1). Set in a distributed
    /// deployment without a network-shared volume — see
    /// [`crate::external_file::ResolveOptions::reject_node_local_files`].
    reject_node_local_files: bool,
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

    /// Set the depot-relative cache root for `PinnedExternalFile`
    /// materialization (D53 §7 / Phase 1.5). Callers that wire a
    /// depot-backed (Docker) spawner pass `<depot>/extfile-cache` here so
    /// materialized inputs land under the depot bind-mount; same-host
    /// deployments leave it unset.
    pub fn set_extfile_cache_root(&mut self, root: impl Into<std::path::PathBuf>) {
        self.extfile_cache_root = Some(root.into());
    }

    /// Reject node-local `file://` references (D53 §3.1). Set this in a
    /// distributed deployment without a network-shared volume so a `file://`
    /// that only exists on one host can't silently break when a worker lands
    /// elsewhere — such deployments must use `oxen://`.
    pub fn set_reject_node_local_files(&mut self, reject: bool) {
        self.reject_node_local_files = reject;
    }

    /// The resolver policy assembled from this dispatcher's configuration,
    /// applied to every `PinnedExternalFile` input before dispatch.
    fn resolve_options(&self) -> crate::external_file::ResolveOptions<'_> {
        crate::external_file::ResolveOptions {
            cache_root: self.extfile_cache_root.as_deref(),
            reject_node_local_files: self.reject_node_local_files,
        }
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
        self.dispatch_run_runtime_script_multi(input_cbor, &[], argument_cbor)
    }

    /// Multi-input form of [`Self::dispatch_run_runtime_script`] (D53 §4.3 /
    /// multi-file join). The primary `input_cbor` plus each of
    /// `additional_inputs_cbor` are prepared (D53 §5: `PinnedExternalFile`
    /// inputs are fetched + content-verified + materialized) and handed to the
    /// runtime as the ordered input list — so a script can read e.g. a
    /// dependency matrix + a sample-info bridge + an annotation table together
    /// (the worker binds them as `eigenius_inputs[[1..N]]`).
    pub fn dispatch_run_runtime_script_multi(
        &self,
        input_cbor: &[u8],
        additional_inputs_cbor: &[Vec<u8>],
        argument_cbor: &[u8],
    ) -> Result<DispatchOutcome, FacadeError> {
        // D53 §5: if an input is an `ingest:PinnedExternalFile`, the substrate
        // fetches + content-verifies + materializes it here and hands the
        // runtime a resource carrying `ingest:materialized_path`. Ordinary
        // chain-resident inputs pass through untouched.
        let opts = self.resolve_options();
        let mut inputs = Vec::with_capacity(1 + additional_inputs_cbor.len());
        inputs.push(crate::external_file::prepare_input(
            parse_resource(input_cbor)?,
            &opts,
        )?);
        for bytes in additional_inputs_cbor {
            inputs.push(crate::external_file::prepare_input(
                parse_resource(bytes)?,
                &opts,
            )?);
        }

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

        let outcome = runtime.run_script(&env, script, &inputs)?;
        Ok(build_outcome(outcome, &language))
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
        let input = crate::external_file::prepare_input(
            parse_resource(input_cbor)?,
            &self.resolve_options(),
        )?;
        let argument = parse_resource(argument_cbor)?;
        let language = read_string_property(&argument, PROP_LANGUAGE)?;
        let runtime = self
            .registry
            .get(&language)
            .ok_or_else(|| FacadeError::UnknownLanguage(language.clone()))?;

        let env = synthesize_env(&language, &argument);
        let signature = &argument;

        let outcome = runtime.call_method(&env, signature, &[input])?;
        Ok(build_outcome(outcome, &language))
    }

    /// Dispatch an external-institution invocation (D31 §6.2 /
    /// Phase 19a.5.c). Same dispatch shape as
    /// [`Self::dispatch_call_runtime_method`] but reached through the
    /// kernel's `DispatchExternal` gRPC path: the kernel sends the
    /// dispatch metadata as structured proto fields rather than as
    /// properties on a chain-resolved `RuntimeMethodSignature`. We
    /// synthesize the env + signature resources here so the
    /// `LanguageRuntime::call_method` boundary stays uniform — the
    /// runtime never has to know which surface the call came from.
    ///
    /// `input_cbors` is the multi-input list per D31 §6.5; for an
    /// AutoOnLoad / Decidable QueryClass dispatch this is exactly one
    /// element (the gated subject). Each element is parsed
    /// independently via [`eigon_cbor::parse_resource_lenient`].
    #[allow(clippy::too_many_arguments)]
    pub fn dispatch_external_institution(
        &self,
        language: &str,
        env_iri: &str,
        image_digest: &str,
        method_name: &str,
        signature_iri: &str,
        input_cbors: &[Vec<u8>],
    ) -> Result<DispatchOutcome, FacadeError> {
        let runtime = self
            .registry
            .get(language)
            .ok_or_else(|| FacadeError::UnknownLanguage(language.to_string()))?;

        let opts = self.resolve_options();
        let mut inputs = Vec::with_capacity(input_cbors.len());
        for bytes in input_cbors {
            inputs.push(crate::external_file::prepare_input(
                parse_resource(bytes)?,
                &opts,
            )?);
        }

        let signature =
            synthesize_signature(language, env_iri, image_digest, method_name, signature_iri);
        let env = synthesize_env(language, &signature);

        let outcome = runtime.call_method(&env, &signature, &inputs)?;
        Ok(build_outcome(outcome, language))
    }
}

/// Build a `DispatchOutcome` from the runtime's [`RunOutcome`] plus
/// the language tag the facade resolved from the dispatch argument.
///
/// Path-3 trait shape: the runtime owns spawn/dispatch/cleanup and
/// hands back the trace fields (timestamps, numerical_metadata,
/// image_digest). The facade only contributes the language tag and
/// the partial-invocation packaging.
fn build_outcome(outcome: RunOutcome, language: &str) -> DispatchOutcome {
    let RunOutcome {
        mut output,
        derivations,
        image_digest,
        started_at,
        completed_at,
        numerical_metadata,
        dispatched_to,
    } = outcome;
    // Epistemic category stamp: every resource produced by a runtime
    // is, by construction, derived (it was computed from inputs by a
    // typed program). The reflection-ontology pins this as
    // `DerivedResource` (D29 §8.4 cross-link, see the reflection
    // ontology). The mirror codec stamps only the structural class on
    // `is_a`; the substrate's commit pipeline owns the epistemic
    // categorization — applied here before the orchestrator commits.
    stamp_derived_epistemic_category(&mut output);
    let trace = DispatchTrace {
        language: language.to_string(),
        image_digest,
        started_at,
        completed_at,
        numerical_metadata,
        dispatched_to,
    };
    let partial = trace.into_partial_invocation();
    let derivations_cbor = derivations
        .iter()
        .map(eigon_cbor::serialize_resource)
        .collect();
    DispatchOutcome {
        output_cbor: eigon_cbor::serialize_resource(&output),
        derivations_cbor,
        partial_invocation_cbor: eigon_cbor::serialize_resource(&partial),
    }
}

/// IRI of the reflection ontology's class for resources produced by
/// computation. Stamped onto every runtime-substrate output's
/// `is_a` list so the chain auditor can distinguish runtime-produced
/// resources from declared / observed / verified ones (reflection
/// ontology §`DerivedResource`).
const PROP_IS_A: &str = "urn:eigenius:core:is_a";
const CLASS_DERIVED_RESOURCE: &str = "urn:eigenius:reflection:DerivedResource";

/// Append `urn:eigenius:reflection:DerivedResource` to the output's
/// `is_a` list, preserving any structural class the mirror codec
/// stamped. Idempotent: a second call is a no-op. Resources without
/// any prior `is_a` (a primitive output, or a worker that didn't
/// stamp) get a single-element list.
fn stamp_derived_epistemic_category(output: &mut Resource) {
    let is_a_iri = Iri::parse(PROP_IS_A).expect("static IRI");
    let derived_iri = Iri::parse(CLASS_DERIVED_RESOURCE).expect("static IRI");

    let mut entries: Vec<Value> = match output.get(&is_a_iri) {
        Some(Value::Array(arr)) => arr.clone(),
        Some(other) => {
            // Defensive: an unexpected shape (single ResourceRef or
            // String). Promote into an array so the rule "is_a is a
            // list" is preserved post-stamp.
            vec![other.clone()]
        }
        None => Vec::new(),
    };
    let already_present = entries.iter().any(|v| match v {
        Value::ResourceRef(i) => i == &derived_iri,
        Value::String(s) => s == derived_iri.as_str(),
        _ => false,
    });
    if !already_present {
        entries.push(Value::ResourceRef(derived_iri));
    }
    output.set(is_a_iri, Value::Array(entries));
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

/// Synthesize a `RuntimeMethodSignature` resource for the
/// `dispatch_external_institution` path. The kernel sends D31 §6.2's
/// structured fields (env_iri, image_digest, method_name,
/// signature_iri, language) — we wrap them in a Resource so the
/// existing `LanguageRuntime::call_method` boundary keeps its single
/// shape. The synthesised signature carries `@id = signature_iri` so
/// per-language runtimes that record the resolved signature IRI keep
/// matching against the kernel's chain-side resource.
fn synthesize_signature(
    language: &str,
    env_iri: &str,
    image_digest: &str,
    method_name: &str,
    signature_iri: &str,
) -> Resource {
    let mut sig = match Iri::parse(signature_iri) {
        Ok(iri) => Resource::new(iri),
        // Defensive fallback: a malformed signature IRI from the wire
        // is a kernel-side bug, but we still need a usable Resource so
        // the caller surfaces an UnknownLanguage / RunError rather
        // than panicking on IRI parse.
        Err(_) => Resource::new_embedded(),
    };
    sig.set(
        Iri::parse(PROP_LANGUAGE).expect("static IRI"),
        Value::String(language.to_string()),
    );
    sig.set(
        Iri::parse(PROP_METHOD_NAME).expect("static IRI"),
        Value::String(method_name.to_string()),
    );
    sig.set(
        Iri::parse(PROP_IMAGE_DIGEST).expect("static IRI"),
        Value::String(image_digest.to_string()),
    );
    if let Ok(env) = Iri::parse(env_iri) {
        sig.set(
            Iri::parse(PROP_REQUIRES_ENVIRONMENT).expect("static IRI"),
            Value::ResourceRef(env),
        );
    }
    sig
}

/// Synthesize an environment Resource for v1's spawn-per-invocation
/// model. The TestLanguageRuntime ignores it; per-language runtimes
/// (Phase 19+) will replace this with a chain-resolved env.
///
/// The env carries `image_digest` when the argument has one — that
/// is, when the dispatch came through `dispatch_external_institution`
/// (whose synthesised signature pins the digest the kernel sent in
/// the `DispatchExternal` request). Per-language runtimes
/// (`JuliaLanguageRuntime` in particular) read the env's digest at
/// dispatch time so a single runtime instance can serve multiple
/// envs concurrently — no per-runtime cached digest, one
/// `ServiceHandle` keyed per digest.
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
    // Forward the image digest if the argument carries one. The
    // synthesised RuntimeMethodSignature for the external-dispatch
    // path always sets it; `RunRuntimeScript` and
    // `CallRuntimeMethod` arguments don't, and those callers fall
    // back to the runtime's lazy `ensure_image` path.
    let digest_prop = Iri::parse(PROP_IMAGE_DIGEST).expect("static IRI");
    if let Some(v) = argument.get(&digest_prop) {
        env.set(digest_prop, v.clone());
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

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    #[test]
    fn stamp_appends_derived_resource_to_empty_is_a() {
        let mut r = Resource::new_embedded();
        stamp_derived_epistemic_category(&mut r);
        let is_a = r.get(&iri(PROP_IS_A)).expect("is_a present").as_iri_array();
        assert_eq!(is_a, vec![iri(CLASS_DERIVED_RESOURCE)]);
    }

    #[test]
    fn stamp_preserves_existing_is_a_classes() {
        // Output came from a mirror codec carrying its own structural
        // class — the stamp must not drop that.
        let mut r = Resource::new_embedded();
        r.set(
            iri(PROP_IS_A),
            Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:test:Demo"))]),
        );
        stamp_derived_epistemic_category(&mut r);
        let is_a = r.get(&iri(PROP_IS_A)).expect("is_a present").as_iri_array();
        assert!(is_a.contains(&iri("urn:eigenius:test:Demo")));
        assert!(is_a.contains(&iri(CLASS_DERIVED_RESOURCE)));
    }

    #[test]
    fn stamp_is_idempotent() {
        let mut r = Resource::new_embedded();
        stamp_derived_epistemic_category(&mut r);
        stamp_derived_epistemic_category(&mut r);
        let is_a = r.get(&iri(PROP_IS_A)).expect("is_a present").as_iri_array();
        // Exactly one entry — no duplicates from the second stamp.
        assert_eq!(is_a, vec![iri(CLASS_DERIVED_RESOURCE)]);
    }

    #[test]
    fn stamp_promotes_non_array_is_a_to_array() {
        // Defensive: some workers might emit a single ResourceRef
        // rather than a list (older codec shapes). Stamp must still
        // produce a valid is_a list.
        let mut r = Resource::new_embedded();
        r.set(
            iri(PROP_IS_A),
            Value::ResourceRef(iri("urn:eigenius:test:OldShape")),
        );
        stamp_derived_epistemic_category(&mut r);
        let is_a = r.get(&iri(PROP_IS_A)).expect("is_a present").as_iri_array();
        assert!(is_a.contains(&iri("urn:eigenius:test:OldShape")));
        assert!(is_a.contains(&iri(CLASS_DERIVED_RESOURCE)));
    }
}
