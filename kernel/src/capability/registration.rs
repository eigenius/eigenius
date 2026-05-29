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

//! Scan a Layer for WASM component/institution resources and register them.
//!
//! A resource declares WASM backing via:
//!
//! ```json
//! {
//!   "@id": "urn:example:components:MyComponent",
//!   "urn:eigenius:core:is_a": ["urn:eigenius:program:Component"],
//!   "urn:eigenius:program:component:implementation": "wasm",
//!   "urn:eigenius:program:component:wasm_binary": "<base64 bytes>",
//!   "urn:eigenius:program:component:capability_level": "urn:eigenius:program:capability_levels:pure"
//! }
//! ```
//!
//! Institutions use the `urn:eigenius:institution:` namespace for the same properties.
//! `wasm_binary_ref` (blob store IRI) is reserved for future use.

use super::external_institution::{ExternalInstitution, ExternalQueryHandler};
use super::wasm_component::{CapabilityLevel, WasmComponent, WasmComponentConfig};
use super::wasm_institution::WasmInstitution;
use crate::institution::in_process_registry::InProcessInstitutionRegistry;
use crate::institution::registry::{InstitutionIndex, RuntimeKind};
use crate::institution::runtime::{Institution, InstitutionRuntime};
use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use crate::program::component::ComponentRegistry;
use crate::server::proto::component_executor_client::ComponentExecutorClient;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::transport::Channel;

/// Protected namespace prefixes. Domain-supplied WASM modules cannot register
/// IRIs under these (D12 §8.4).
const PROTECTED_NAMESPACES: &[&str] = &[
    "urn:eigenius:core:",
    "urn:eigenius:program:",
    "urn:eigenius:reflection:",
    "urn:eigenius:institution:",
];

/// Exception: built-in components under program:components: can use the
/// program namespace (they're part of the built-in registry anyway).
const PROTECTED_NAMESPACE_EXCEPTIONS: &[&str] = &["urn:eigenius:program:components:"];

/// An error encountered while loading WASM extensions from a layer.
#[derive(Debug, Clone)]
pub struct RegistrationError {
    pub resource_iri: String,
    pub message: String,
}

impl std::fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.resource_iri, self.message)
    }
}

/// A warning emitted during a scan (non-fatal).
#[derive(Debug, Clone)]
pub struct RegistrationWarning {
    pub resource_iri: String,
    pub message: String,
}

impl std::fmt::Display for RegistrationWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] warning: {}", self.resource_iri, self.message)
    }
}

/// Summary of what was registered during a scan.
#[derive(Debug, Default)]
pub struct RegistrationReport {
    pub components_registered: Vec<String>,
    pub institutions_registered: Vec<String>,
    pub errors: Vec<RegistrationError>,
    pub warnings: Vec<RegistrationWarning>,
}

/// An IO-capability WASM component awaiting forwarding to the orchestrator.
/// The kernel can't host IO components itself (they need network/LLM access);
/// the caller ships these to the orchestrator via `RegisterWasmComponent`.
pub struct PendingIoComponent {
    pub resource_iri: String,
    pub wasm_binary: Vec<u8>,
    pub fuel_limit: u64,
    pub memory_limit_pages: u32,
}

/// Full scan result, including IO components that need orchestrator registration.
#[derive(Default)]
pub struct ScanResult {
    pub report: RegistrationReport,
    pub pending_io_components: Vec<PendingIoComponent>,
}

/// Walk a Layer looking for WASM component resources.
///
/// - Pure/read components are loaded and registered in `components` (kernel host).
/// - IO components are collected as `PendingIoComponent` entries for the caller
///   to forward to the orchestrator (kernel can't host IO).
///
/// Resources without `implementation = "wasm"` are ignored — they are
/// handled by the regular built-in or remote component path.
///
/// Institution declarations under D14 are plain ontology resources scanned
/// elsewhere (see [`crate::institution::registry::InstitutionIndex`]); this
/// scanner does not handle them.
pub fn scan_and_register(layer: &Layer, components: &mut ComponentRegistry) -> ScanResult {
    let mut result = ScanResult::default();

    let impl_prop = Iri::parse("urn:eigenius:program:component:implementation").unwrap();

    for arc_resource in layer.iter_resources().map(|(_, r)| r) {
        let resource: &Resource = &arc_resource;
        let id = match resource.id() {
            Some(i) => i.as_str().to_string(),
            None => continue, // Only top-level resources can be registered
        };

        // Component: has program:component:implementation = "wasm"
        if let Some(Value::String(s)) = resource.get(&impl_prop) {
            if s == "wasm" {
                if let Err(e) = check_namespace(&id) {
                    result.report.errors.push(RegistrationError {
                        resource_iri: id,
                        message: e,
                    });
                    continue;
                }

                // Decide kernel-host (pure/read) vs orchestrator-host (io) by
                // inspecting the declared capability level on the resource.
                match classify_component_capability(resource) {
                    Ok(CapabilityClass::KernelHost) => match load_wasm_component(resource, layer) {
                        Ok(component) => {
                            let declared_iri = component.iri().to_string();
                            if declared_iri != id {
                                result.report.warnings.push(RegistrationWarning {
                                        resource_iri: id.clone(),
                                        message: format!(
                                            "WASM binary declares IRI '{declared_iri}' which differs from resource @id '{id}' — using binary's IRI",
                                        ),
                                    });
                            }
                            components.register(declared_iri.clone(), Box::new(component));
                            result.report.components_registered.push(declared_iri);
                        }
                        Err(e) => {
                            result.report.errors.push(RegistrationError {
                                resource_iri: id,
                                message: e,
                            });
                        }
                    },
                    Ok(CapabilityClass::OrchestratorHost) => {
                        match extract_pending_io(resource, &id, layer) {
                            Ok(pending) => {
                                result.pending_io_components.push(pending);
                                result.report.components_registered.push(id);
                            }
                            Err(e) => {
                                result.report.errors.push(RegistrationError {
                                    resource_iri: id,
                                    message: e,
                                });
                            }
                        }
                    }
                    Err(e) => {
                        result.report.errors.push(RegistrationError {
                            resource_iri: id,
                            message: e,
                        });
                    }
                }
                continue;
            }
        }
    }

    result
}

/// Walk the layer chain for D14 Institution declarations whose
/// `runtime` is `urn:eigenius:institution:runtimes:wasm` and build an
/// [`InstitutionRuntime`] populated with [`WasmInstitution`] instances
/// for each. In-process / external runtime declarations are skipped —
/// the caller is responsible for registering those programmatically.
///
/// Resources are merged across the chain; the topmost declaration for
/// each institution IRI wins (so a child layer can override a parent
/// layer's `runtime: in_process` declaration with `runtime: wasm` +
/// `wasm_binary`).
pub fn build_wasm_institution_runtime(layer: &Layer) -> (InstitutionRuntime, RegistrationReport) {
    let mut report = RegistrationReport::default();
    let mut runtime = InstitutionRuntime::new();

    let runtime_prop = Iri::parse(wk::RUNTIME).expect("well-known IRI");
    let institution_class_iri = Iri::parse("urn:eigenius:institution:Institution").expect("IRI");

    for (iri, resource) in layer.iter_all_resources() {
        if !resource.is_instance_of(&institution_class_iri) {
            continue;
        }
        // `runtime` is `data_type: resource`; canonicalises to
        // `ResourceRef`. `as_iri` accepts both that and the
        // pre-canonical `String` shape.
        let is_wasm = resource
            .get(&runtime_prop)
            .and_then(|v| v.as_iri())
            .is_some_and(|i| i.as_str() == wk::RUNTIME_WASM);
        if !is_wasm {
            continue; // not WASM-runtime — caller's responsibility
        }

        match load_wasm_institution(&resource, &iri, layer) {
            Ok(wasm_inst) => {
                let inst_iri = wasm_inst.institution_iri().clone();
                if let Err(e) = runtime.register(Box::new(wasm_inst)) {
                    report.errors.push(RegistrationError {
                        resource_iri: iri.as_str().to_string(),
                        message: format!("InstitutionRuntime::register failed: {e}"),
                    });
                    continue;
                }
                report
                    .institutions_registered
                    .push(inst_iri.as_str().to_string());
            }
            Err(e) => {
                report.errors.push(RegistrationError {
                    resource_iri: iri.as_str().to_string(),
                    message: e,
                });
            }
        }
    }

    (runtime, report)
}

/// One entry per external Institution declaration that resolves
/// cleanly against the chain. Returned by
/// [`validate_external_institution_chain`] for use both in install-
/// time cross-checks and in [`register_external_institutions`].
#[derive(Debug, Clone)]
pub struct ExternalInstitutionPlan {
    pub institution_iri: Iri,
    pub env_iri: Iri,
    pub image_digest: String,
    pub language: String,
    pub handlers: BTreeMap<Iri, ExternalQueryHandler>,
}

/// One install-time error produced by
/// [`validate_external_institution_chain`].
#[derive(Debug, Clone)]
pub struct ExternalInstitutionCheckError {
    pub institution_iri: String,
    pub message: String,
}

/// Walk the chain for `runtime: external` institutions and resolve
/// the metadata each one needs to dispatch (env IRI + image digest +
/// per-`query_handler` method name + signature IRI). Returns `(plans,
/// errors)`: every well-formed external institution lands in `plans`,
/// every malformed one in `errors`.
///
/// Pure data check — does **not** open a gRPC connection. Used by
/// the D41 commit pipeline's `autoonload_dispatch` phase to reject a
/// Load whose external-institution shape can't be wired up, and by
/// [`register_external_institutions`] to feed the registration loop
/// without re-walking the chain.
pub fn validate_external_institution_chain(
    layer: &Layer,
    index: &InstitutionIndex,
) -> (
    Vec<ExternalInstitutionPlan>,
    Vec<ExternalInstitutionCheckError>,
) {
    let mut plans = Vec::new();
    let mut errors = Vec::new();

    let runtime_prop = Iri::parse(wk::RUNTIME).expect("well-known IRI");
    let institution_class_iri = Iri::parse("urn:eigenius:institution:Institution").expect("IRI");
    let env_ref_prop = Iri::parse(wk::INSTITUTION_REQUIRES_ENVIRONMENT).expect("well-known IRI");
    let image_digest_prop = Iri::parse(wk::RUNTIME_IMAGE_DIGEST).expect("well-known IRI");
    let method_name_prop = Iri::parse(wk::RUNTIME_METHOD_NAME).expect("well-known IRI");
    let language_prop = Iri::parse(wk::RUNTIME_LANGUAGE).expect("well-known IRI");

    for (iri, resource) in layer.iter_all_resources() {
        if !resource.is_instance_of(&institution_class_iri) {
            continue;
        }
        // `runtime` is `data_type: resource` post-canonicalisation,
        // so the value is a `ResourceRef`. `Value::as_iri` accepts
        // both ResourceRef and (legacy/parse-time) String, so this
        // also handles intermediates that haven't been through
        // `canonicalise_resource_refs` yet.
        match resource.get(&runtime_prop).and_then(|v| v.as_iri()) {
            Some(i) if i.as_str() == wk::RUNTIME_EXTERNAL => {}
            _ => continue,
        }

        let inst_iri_str = iri.as_str().to_string();

        let env_iri = match resource.get(&env_ref_prop).and_then(|v| v.as_iri()) {
            Some(i) => i,
            None => {
                errors.push(ExternalInstitutionCheckError {
                    institution_iri: inst_iri_str,
                    message:
                        "external institution missing `requires_environment` — D31 §5 requires \
                        every external institution to declare an env it dispatches into"
                            .to_string(),
                });
                continue;
            }
        };

        let env_resource = match resolve_via_layer(layer, &env_iri) {
            Some(r) => r,
            None => {
                errors.push(ExternalInstitutionCheckError {
                    institution_iri: inst_iri_str,
                    message: format!(
                        "`requires_environment` -> `{env_iri}` did not resolve to a \
                         RuntimeEnvironment in the chain"
                    ),
                });
                continue;
            }
        };

        let image_digest = match env_resource.get(&image_digest_prop) {
            Some(Value::String(s)) => s.clone(),
            _ => {
                errors.push(ExternalInstitutionCheckError {
                    institution_iri: inst_iri_str,
                    message: format!(
                        "RuntimeEnvironment `{env_iri}` carries no `image_digest` — \
                         orchestrator cannot dispatch without one"
                    ),
                });
                continue;
            }
        };

        let language = match env_resource.get(&language_prop) {
            Some(Value::String(s)) => s.clone(),
            _ => {
                errors.push(ExternalInstitutionCheckError {
                    institution_iri: inst_iri_str,
                    message: format!(
                        "RuntimeEnvironment `{env_iri}` carries no `language` — orchestrator \
                         cannot route to a LanguageRuntime without one"
                    ),
                });
                continue;
            }
        };

        let mut handlers: BTreeMap<Iri, ExternalQueryHandler> = BTreeMap::new();
        let mut handler_errors: Vec<String> = Vec::new();

        // Harvest the procedure → method_name dispatch table from
        // every D14 declaration that anchors a runtime entry on this
        // institution: QueryClass.query_handler (FIBER / AutoOnLoad
        // dispatch), ExportFormat.procedure (comorphism source-side
        // extract_typed), ImportFormat.procedure (comorphism
        // target-side reify). All three resolve through the same
        // `RuntimeMethodSignature.method_name` property and dispatch
        // through the same `DispatchExternal` RPC — D14 §9.3 doesn't
        // distinguish source-side / target-side at the wire level.
        //
        // QueryClass entries are *required* — their AutoOnLoad /
        // OnDemand FIBER gates are the institution's published
        // surface, and missing dispatch metadata there is a
        // structural failure. ExportFormat / ImportFormat entries
        // are *recommended* (D14): the comorphism dispatch path
        // (`Exp::InstitutionInvoke`) errors with a clear
        // `UnknownType` at runtime when a procedure is missing here,
        // but rejecting the whole institution at registration time
        // would brick its working AutoOnLoad / FIBER gates over a
        // comorphism gap that may or may not be exercised on this
        // chain. Surface missing format procedures as warnings so
        // operators see them on each rebuild without losing the
        // institution.
        let mut handler_warnings: Vec<String> = Vec::new();
        let record_handler = |kind: &str,
                              owner_iri: &Iri,
                              signature_iri: &Iri,
                              handlers: &mut BTreeMap<Iri, ExternalQueryHandler>,
                              handler_errors: &mut Vec<String>,
                              handler_warnings: &mut Vec<String>,
                              tolerate_missing: bool| {
            if handlers.contains_key(signature_iri) {
                return;
            }
            let method_name = match resolve_via_layer(layer, signature_iri) {
                Some(sig) => match sig.get(&method_name_prop) {
                    Some(Value::String(s)) => s.clone(),
                    _ => {
                        let msg = format!(
                            "{kind} `{owner_iri}`: procedure -> `{signature_iri}` carries no \
                                 `method_name` (RuntimeMethodSignature property)"
                        );
                        if tolerate_missing {
                            handler_warnings.push(msg);
                        } else {
                            handler_errors.push(msg);
                        }
                        return;
                    }
                },
                None => {
                    let msg = format!(
                        "{kind} `{owner_iri}`: procedure -> `{signature_iri}` did not resolve \
                             to a RuntimeMethodSignature in the chain"
                    );
                    if tolerate_missing {
                        handler_warnings.push(msg);
                    } else {
                        handler_errors.push(msg);
                    }
                    return;
                }
            };
            handlers.insert(
                signature_iri.clone(),
                ExternalQueryHandler {
                    method_name,
                    signature_iri: signature_iri.clone(),
                },
            );
        };

        for qc in index.query_classes() {
            if qc.institution_ref.as_str() != iri.as_str() {
                continue;
            }
            record_handler(
                "QueryClass",
                &qc.iri,
                &qc.query_handler,
                &mut handlers,
                &mut handler_errors,
                &mut handler_warnings,
                /* tolerate_missing */ false,
            );
        }
        for ef in index.export_formats() {
            if ef.institution_ref.as_str() != iri.as_str() {
                continue;
            }
            record_handler(
                "ExportFormat",
                &ef.iri,
                &ef.procedure,
                &mut handlers,
                &mut handler_errors,
                &mut handler_warnings,
                /* tolerate_missing */ true,
            );
        }
        for f in index.import_formats() {
            if f.institution_ref.as_str() != iri.as_str() {
                continue;
            }
            record_handler(
                "ImportFormat",
                &f.iri,
                &f.procedure,
                &mut handlers,
                &mut handler_errors,
                &mut handler_warnings,
                /* tolerate_missing */ true,
            );
        }
        for w in &handler_warnings {
            tracing::warn!(
                institution_iri = %iri,
                "comorphism dispatch metadata gap: {w} — `Exp::InstitutionInvoke` calls through this procedure will fail at runtime; the institution's AutoOnLoad / FIBER gates remain operational",
            );
        }
        if !handler_errors.is_empty() {
            errors.push(ExternalInstitutionCheckError {
                institution_iri: inst_iri_str,
                message: format!(
                    "external institution dispatch metadata incomplete: {}",
                    handler_errors.join("; ")
                ),
            });
            continue;
        }

        plans.push(ExternalInstitutionPlan {
            institution_iri: iri.clone(),
            env_iri,
            image_digest,
            language,
            handlers,
        });
    }

    (plans, errors)
}

/// Walk the chain for D14 Institution declarations whose `runtime` is
/// `urn:eigenius:institution:runtimes:external` (D31 §5) and register
/// an [`ExternalInstitution`] in `runtime` for each. Each registered
/// institution holds the env IRI + image digest resolved from the
/// chain plus a per-`query_handler` lookup of method-dispatch
/// metadata, all wired against the shared orchestrator gRPC `client`.
///
/// Institutions whose `requires_environment` cannot be resolved (or
/// whose env carries no `image_digest`) are skipped with an error
/// recorded in `report` — the kernel will not gate Loads against an
/// institution it cannot reach.
pub fn register_external_institutions(
    layer: &Layer,
    index: &InstitutionIndex,
    runtime: &mut InstitutionRuntime,
    client: Arc<Mutex<ComponentExecutorClient<Channel>>>,
    report: &mut RegistrationReport,
) {
    let (plans, errors) = validate_external_institution_chain(layer, index);
    for err in errors {
        report.errors.push(RegistrationError {
            resource_iri: err.institution_iri,
            message: err.message,
        });
    }
    for plan in plans {
        let inst = ExternalInstitution::new(
            plan.institution_iri.clone(),
            plan.env_iri,
            plan.image_digest,
            plan.language,
            plan.handlers,
            client.clone(),
        );
        let registered_iri = plan.institution_iri.as_str().to_string();
        runtime.replace(Box::new(inst));
        report.institutions_registered.push(registered_iri);
    }
}

/// Walk the chain for D14 Institution declarations whose `runtime` is
/// `urn:eigenius:institution:runtimes:in_process` (D28 §2.3) and
/// register the matching pre-registered impl from
/// `in_process_registry` into `runtime`. Phase 20a.1.
///
/// Unlike the WASM and External paths, the institution implementation
/// itself is **not** constructed from chain data — it's Rust code
/// linked into the orchestrator binary and pre-registered at startup
/// by each in-process institution crate. The chain-scan pass just
/// dispatches: for every `runtime: in_process` declaration, look up
/// the IRI in `in_process_registry`, register the looked-up `Arc<dyn
/// Institution>` into `runtime`.
///
/// Chain-declared `in_process` institutions that have no matching
/// pre-registered impl produce a `RegistrationError` — the
/// declaration is malformed (or the in-process crate isn't linked
/// into this build), and silently dropping it would surface as
/// unrelated `NotImplemented` errors at dispatch time.
pub fn register_in_process_institutions(
    index: &InstitutionIndex,
    runtime: &mut InstitutionRuntime,
    in_process_registry: &InProcessInstitutionRegistry,
    report: &mut RegistrationReport,
) {
    for entry in index.institutions() {
        if entry.runtime != Some(RuntimeKind::InProcess) {
            continue;
        }
        match in_process_registry.get(&entry.iri) {
            Some(arc_inst) => {
                let registered_iri = entry.iri.as_str().to_string();
                // Wrap the Arc<dyn Institution> in a Box via the
                // blanket impl on Arc<I>. Each rebuild gets a fresh
                // Box but the underlying Arc is shared with the
                // process-global registry — no per-rebuild state
                // reconstruction.
                runtime.replace(Box::new(arc_inst));
                report.institutions_registered.push(registered_iri);
            }
            None => {
                report.errors.push(RegistrationError {
                    resource_iri: entry.iri.as_str().to_string(),
                    message: format!(
                        "chain declares `runtime: in_process` for institution `{}` but no \
                         matching impl is registered in this build's \
                         InProcessInstitutionRegistry — link the appropriate \
                         in-process institution crate or change the declaration's runtime",
                        entry.iri
                    ),
                });
            }
        }
    }
}

fn resolve_via_layer(layer: &Layer, iri: &Iri) -> Option<Arc<Resource>> {
    layer.resolve(iri)
}

/// Load a WASM institution from an Institution resource declaring
/// `runtime: wasm` + `wasm_binary`.
fn load_wasm_institution(
    resource: &Resource,
    iri: &Iri,
    layer: &Layer,
) -> Result<WasmInstitution, String> {
    let bytes = extract_wasm_bytes(
        resource,
        "urn:eigenius:institution:wasm_binary",
        "urn:eigenius:institution:wasm_binary_ref",
        layer,
    )?;
    let config = extract_config(
        resource,
        "urn:eigenius:institution:fuel_limit",
        "urn:eigenius:institution:memory_limit_pages",
    );
    // The institution's IRI is its @id — same as the resource IRI.
    // The InstitutionRuntime keys by this; D14 dispatch resolves a
    // QueryClass's `institution_ref` against this key.
    WasmInstitution::from_bytes(iri.clone(), &bytes, config)
}

/// Load a WASM component from a resource.
fn load_wasm_component(resource: &Resource, layer: &Layer) -> Result<WasmComponent, String> {
    let bytes = extract_wasm_bytes(
        resource,
        "urn:eigenius:program:component:wasm_binary",
        "urn:eigenius:program:component:wasm_binary_ref",
        layer,
    )?;
    let level =
        extract_capability_level(resource, "urn:eigenius:program:component:capability_level")?;
    let config = extract_config(
        resource,
        "urn:eigenius:program:component:fuel_limit",
        "urn:eigenius:program:component:memory_limit_pages",
    );
    WasmComponent::from_bytes(&bytes, level, config)
}

/// Extract WASM binary bytes from a resource, trying inline first then blob ref.
fn extract_wasm_bytes(
    resource: &Resource,
    inline_prop: &str,
    _ref_prop: &str,
    _layer: &Layer,
) -> Result<Vec<u8>, String> {
    let inline_iri = Iri::parse(inline_prop).unwrap();

    // Try inline binary (base64-encoded or hex-encoded raw bytes)
    if let Some(Value::String(s)) = resource.get(&inline_iri) {
        return decode_wasm_binary(s);
    }

    // Blob ref is reserved for future implementation
    Err(format!(
        "resource has no '{inline_prop}' (blob ref support is future work)"
    ))
}

/// Decode a WASM binary from a string. Supports:
/// - base64 encoding (standard)
/// - hex encoding (with "hex:" prefix)
fn decode_wasm_binary(s: &str) -> Result<Vec<u8>, String> {
    if let Some(hex_data) = s.strip_prefix("hex:") {
        hex::decode(hex_data).map_err(|e| format!("hex decode failed: {e}"))
    } else {
        base64_decode(s)
    }
}

/// Minimal standard base64 decode (RFC 4648, no padding tolerance).
fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut decode = [255u8; 256];
    for (i, &b) in ALPHA.iter().enumerate() {
        decode[b as usize] = i as u8;
    }

    let s = s.trim();
    let input: Vec<u8> = s
        .bytes()
        .filter(|&b| b != b'=' && !b.is_ascii_whitespace())
        .collect();
    let mut out = Vec::with_capacity(input.len() * 3 / 4);

    for chunk in input.chunks(4) {
        if chunk.is_empty() {
            break;
        }
        let mut buf = [0u8; 4];
        for (i, &b) in chunk.iter().enumerate() {
            let v = decode[b as usize];
            if v == 255 {
                return Err(format!("invalid base64 character: {}", b as char));
            }
            buf[i] = v;
        }
        if chunk.len() >= 2 {
            out.push((buf[0] << 2) | (buf[1] >> 4));
        }
        if chunk.len() >= 3 {
            out.push((buf[1] << 4) | (buf[2] >> 2));
        }
        if chunk.len() == 4 {
            out.push((buf[2] << 6) | buf[3]);
        }
    }
    Ok(out)
}

/// Where a WASM component should be hosted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityClass {
    /// Pure or read: hosted in the kernel via Wasmtime.
    KernelHost,
    /// IO: hosted in the orchestrator via jco + Deno WebAssembly.
    OrchestratorHost,
}

fn classify_component_capability(resource: &Resource) -> Result<CapabilityClass, String> {
    let iri = Iri::parse("urn:eigenius:program:component:capability_level").unwrap();
    let level_str = match resource.get(&iri) {
        Some(Value::String(s)) => s.as_str(),
        _ => return Ok(CapabilityClass::KernelHost), // Default: most restrictive (pure)
    };
    match level_str {
        "urn:eigenius:program:capability_levels:pure"
        | "urn:eigenius:program:capability_levels:read" => Ok(CapabilityClass::KernelHost),
        "urn:eigenius:program:capability_levels:io" => Ok(CapabilityClass::OrchestratorHost),
        other => Err(format!("unknown capability level: {other}")),
    }
}

fn extract_capability_level(resource: &Resource, prop: &str) -> Result<CapabilityLevel, String> {
    let iri = Iri::parse(prop).unwrap();
    let level_str = match resource.get(&iri) {
        Some(Value::String(s)) => s.as_str(),
        _ => return Ok(CapabilityLevel::Pure), // Default to most restrictive
    };

    match level_str {
        "urn:eigenius:program:capability_levels:pure" => Ok(CapabilityLevel::Pure),
        "urn:eigenius:program:capability_levels:read" => Ok(CapabilityLevel::Read),
        "urn:eigenius:program:capability_levels:io" => Err(
            "IO capability-level components must be classified for orchestrator hosting before reaching this function"
                .to_string(),
        ),
        other => Err(format!("unknown capability level: {other}")),
    }
}

/// Build a `PendingIoComponent` descriptor from the resource.
fn extract_pending_io(
    resource: &Resource,
    resource_iri: &str,
    layer: &Layer,
) -> Result<PendingIoComponent, String> {
    let bytes = extract_wasm_bytes(
        resource,
        "urn:eigenius:program:component:wasm_binary",
        "urn:eigenius:program:component:wasm_binary_ref",
        layer,
    )?;
    let config = extract_config(
        resource,
        "urn:eigenius:program:component:fuel_limit",
        "urn:eigenius:program:component:memory_limit_pages",
    );
    Ok(PendingIoComponent {
        resource_iri: resource_iri.to_string(),
        wasm_binary: bytes,
        fuel_limit: config.fuel_limit,
        memory_limit_pages: config.memory_limit_pages,
    })
}

fn extract_config(resource: &Resource, fuel_prop: &str, mem_prop: &str) -> WasmComponentConfig {
    let mut config = WasmComponentConfig::default();

    if let Some(Value::Integer(n)) = resource.get(&Iri::parse(fuel_prop).unwrap()) {
        if *n > 0 {
            config.fuel_limit = *n as u64;
        }
    }
    if let Some(Value::Integer(n)) = resource.get(&Iri::parse(mem_prop).unwrap()) {
        if *n > 0 {
            config.memory_limit_pages = *n as u32;
        }
    }
    config
}

/// Check that an IRI doesn't violate namespace protection.
fn check_namespace(iri: &str) -> Result<(), String> {
    for exception in PROTECTED_NAMESPACE_EXCEPTIONS {
        if iri.starts_with(exception) {
            return Ok(());
        }
    }
    for protected in PROTECTED_NAMESPACES {
        if iri.starts_with(protected) {
            return Err(format!(
                "cannot register WASM extension under protected namespace '{protected}'"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_check_rejects_core() {
        assert!(check_namespace("urn:eigenius:core:Foo").is_err());
        assert!(check_namespace("urn:eigenius:reflection:Foo").is_err());
        assert!(check_namespace("urn:eigenius:institution:Foo").is_err());
    }

    #[test]
    fn namespace_check_rejects_program_except_components() {
        // Plain program namespace: rejected
        assert!(check_namespace("urn:eigenius:program:Foo").is_err());
        // But program:components: is an exception (built-ins live there)
        assert!(check_namespace("urn:eigenius:program:components:Foo").is_ok());
    }

    #[test]
    fn namespace_check_accepts_user_namespaces() {
        assert!(check_namespace("urn:example:components:MyComp").is_ok());
        assert!(check_namespace("urn:customer:myapp:foo").is_ok());
    }

    #[test]
    fn base64_decode_known_vectors() {
        assert_eq!(base64_decode("").unwrap(), b"");
        assert_eq!(base64_decode("Zg==").unwrap(), b"f");
        assert_eq!(base64_decode("Zm8=").unwrap(), b"fo");
        assert_eq!(base64_decode("Zm9v").unwrap(), b"foo");
        assert_eq!(base64_decode("Zm9vYg==").unwrap(), b"foob");
        assert_eq!(base64_decode("Zm9vYmE=").unwrap(), b"fooba");
        assert_eq!(base64_decode("Zm9vYmFy").unwrap(), b"foobar");
    }

    #[test]
    fn base64_decode_tolerates_whitespace() {
        assert_eq!(base64_decode("Zm9v\nYmFy").unwrap(), b"foobar");
        assert_eq!(base64_decode("Zm9v YmFy").unwrap(), b"foobar");
    }

    #[test]
    fn base64_decode_rejects_invalid() {
        assert!(base64_decode("!!!!").is_err());
    }

    #[test]
    fn hex_prefix_decoding() {
        assert_eq!(decode_wasm_binary("hex:48656c6c6f").unwrap(), b"Hello");
    }

    // ─── Phase 20a.1 — InProcess runtime kind registration ───────────────

    use crate::institution::in_process_registry::{EchoInstitution, InProcessInstitutionRegistry};
    use crate::institution::registry::InstitutionIndex;
    use crate::institution::runtime::{Institution, InstitutionRuntime};
    use crate::layer::LayerBuilder;
    use crate::ontology::resource::{Resource, Value};
    use std::sync::Arc;

    /// Build a tiny layer carrying one Institution resource declaring
    /// the supplied IRI with `runtime: in_process`. Used by the
    /// in-process registration tests below.
    fn layer_with_in_process_institution(iri: &str) -> Arc<crate::layer::Layer> {
        let inst_iri = Iri::parse(iri).expect("test IRI");
        let mut inst = Resource::new(inst_iri.clone());
        inst.set(
            Iri::parse(wk::IS_A).expect("IS_A IRI"),
            Value::Array(vec![Value::ResourceRef(
                Iri::parse("urn:eigenius:institution:Institution").expect("IRI"),
            )]),
        );
        inst.set(
            Iri::parse("urn:eigenius:institution:institution_iri").expect("IRI"),
            Value::String(iri.to_string()),
        );
        inst.set(
            Iri::parse("urn:eigenius:institution:institution_name").expect("IRI"),
            Value::String("Echo".to_string()),
        );
        inst.set(
            Iri::parse(wk::RUNTIME).expect("RUNTIME IRI"),
            Value::ResourceRef(Iri::parse(wk::RUNTIME_IN_PROCESS).expect("RUNTIME_IN_PROCESS IRI")),
        );
        let mut b = LayerBuilder::new("in_process_test", None);
        b.add_resource(inst).unwrap();
        Arc::new(b.build(crate::layer::LayerStorage::in_memory()))
    }

    #[test]
    fn in_process_registration_succeeds_when_impl_is_pre_registered() {
        let iri_str = "urn:eigenius:test:echo_in_process";
        let layer = layer_with_in_process_institution(iri_str);
        let (index, errs) = InstitutionIndex::from_layer(&layer);
        assert!(errs.is_empty(), "unexpected index errors: {errs:?}");

        // Pre-register the EchoInstitution impl as a statically-linked
        // institution crate would.
        let in_process = InProcessInstitutionRegistry::new();
        let echo: Arc<dyn Institution> =
            Arc::new(EchoInstitution::new(Iri::parse(iri_str).unwrap()));
        in_process.register(echo);

        let mut runtime = InstitutionRuntime::new();
        let mut report = RegistrationReport::default();
        register_in_process_institutions(&index, &mut runtime, &in_process, &mut report);

        assert!(
            report.errors.is_empty(),
            "unexpected report errors: {:?}",
            report.errors
        );
        assert_eq!(
            report.institutions_registered,
            vec![iri_str.to_string()],
            "exactly one institution should land in the report's registered list"
        );

        // The dispatchable runtime must carry the institution under
        // the declared IRI.
        let registered = runtime
            .get(&Iri::parse(iri_str).unwrap())
            .expect("EchoInstitution should be in the InstitutionRuntime");
        assert_eq!(registered.institution_iri().as_str(), iri_str);
    }

    #[test]
    fn in_process_registration_errors_when_impl_is_missing() {
        let iri_str = "urn:eigenius:test:nothing_registered_for_this_iri";
        let layer = layer_with_in_process_institution(iri_str);
        let (index, errs) = InstitutionIndex::from_layer(&layer);
        assert!(errs.is_empty());

        // Empty registry — no impl pre-registered.
        let in_process = InProcessInstitutionRegistry::new();
        let mut runtime = InstitutionRuntime::new();
        let mut report = RegistrationReport::default();
        register_in_process_institutions(&index, &mut runtime, &in_process, &mut report);

        assert!(
            report.errors.iter().any(|e| e.resource_iri == iri_str
                && e.message.contains("no matching impl is registered")),
            "expected a registration error naming the missing impl; got {:?}",
            report.errors
        );
        assert!(
            report.institutions_registered.is_empty(),
            "no institution should land in the registered list"
        );
        assert!(
            runtime.get(&Iri::parse(iri_str).unwrap()).is_none(),
            "InstitutionRuntime should not carry the malformed declaration"
        );
    }

    #[test]
    fn in_process_registration_ignores_wasm_and_external_institutions() {
        // Construct a layer with a `runtime: wasm` institution and
        // verify the in-process pass skips it (Wasm has its own
        // registration path).
        let wasm_iri = "urn:eigenius:test:wasm_inst";
        let mut inst = Resource::new(Iri::parse(wasm_iri).unwrap());
        inst.set(
            Iri::parse(wk::IS_A).expect("IS_A IRI"),
            Value::Array(vec![Value::ResourceRef(
                Iri::parse("urn:eigenius:institution:Institution").unwrap(),
            )]),
        );
        inst.set(
            Iri::parse("urn:eigenius:institution:institution_iri").unwrap(),
            Value::String(wasm_iri.to_string()),
        );
        inst.set(
            Iri::parse("urn:eigenius:institution:institution_name").unwrap(),
            Value::String("Wasm".to_string()),
        );
        inst.set(
            Iri::parse(wk::RUNTIME).unwrap(),
            Value::ResourceRef(Iri::parse(wk::RUNTIME_WASM).unwrap()),
        );
        let mut b = LayerBuilder::new("wasm_only", None);
        b.add_resource(inst).unwrap();
        let layer = Arc::new(b.build(crate::layer::LayerStorage::in_memory()));

        let (index, errs) = InstitutionIndex::from_layer(&layer);
        assert!(errs.is_empty());

        let in_process = InProcessInstitutionRegistry::new();
        let mut runtime = InstitutionRuntime::new();
        let mut report = RegistrationReport::default();
        register_in_process_institutions(&index, &mut runtime, &in_process, &mut report);

        assert!(
            report.errors.is_empty(),
            "wasm institutions should be skipped silently"
        );
        assert!(report.institutions_registered.is_empty());
        assert!(runtime.get(&Iri::parse(wasm_iri).unwrap()).is_none());
    }
}
