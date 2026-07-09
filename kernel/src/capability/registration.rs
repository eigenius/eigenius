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

//! Scan the layer chain for institution declarations and wire their
//! backends into the runtime: **in-process** Rust institutions (registered
//! at startup) and **external** institutions (dispatched over gRPC to the
//! orchestrator substrate). See D14 (institutions).

use super::external_institution::{ExternalInstitution, ExternalQueryHandler};
use crate::institution::in_process_registry::InProcessInstitutionRegistry;
use crate::institution::registry::{InstitutionIndex, RuntimeKind};
use crate::institution::runtime::InstitutionRuntime;
use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use crate::program::remote::OrchestratorTransport;
use crate::server::proto::component_executor_client::ComponentExecutorClient;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// An error encountered while loading institution declarations from a layer.
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
    let institution_class_str = "urn:eigenius:institution:Institution";
    let institution_class_iri = Iri::parse(institution_class_str).expect("IRI");
    let env_ref_prop = Iri::parse(wk::INSTITUTION_REQUIRES_ENVIRONMENT).expect("well-known IRI");
    let image_digest_prop = Iri::parse(wk::RUNTIME_IMAGE_DIGEST).expect("well-known IRI");
    let method_name_prop = Iri::parse(wk::RUNTIME_METHOD_NAME).expect("well-known IRI");
    let language_prop = Iri::parse(wk::RUNTIME_LANGUAGE).expect("well-known IRI");

    // Index-driven discovery (D23): only the chain's `Institution` declarations,
    // found via the triple index, rather than materialising the whole chain. The
    // single transitive caller is the commit-time rebuild hook (always core-rooted),
    // so `is_a` is indexable. `is_instance_of` below is then a redundant-but-cheap
    // guard.
    for resource in crate::layer::resolve_typed_resources(layer, &[institution_class_str]) {
        if !resource.is_instance_of(&institution_class_iri) {
            continue;
        }
        let Some(iri) = resource.id().cloned() else {
            continue;
        };
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
        // every institution declaration that anchors a runtime entry on this
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

/// Walk the chain for Institution declarations whose `runtime` is
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
    client: Arc<Mutex<ComponentExecutorClient<OrchestratorTransport>>>,
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

/// Walk the chain for Institution declarations whose `runtime` is
/// `urn:eigenius:institution:runtimes:in_process` (D28 §2.3) and
/// register the matching pre-registered impl from
/// `in_process_registry` into `runtime`. Phase 20a.1.
///
/// Unlike the External path, the institution implementation
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn in_process_registration_ignores_external_institutions() {
        // Construct a layer with a `runtime: external` institution and
        // verify the in-process pass skips it (external has its own
        // gRPC dispatch path).
        let ext_iri = "urn:eigenius:test:external_inst";
        let mut inst = Resource::new(Iri::parse(ext_iri).unwrap());
        inst.set(
            Iri::parse(wk::IS_A).expect("IS_A IRI"),
            Value::Array(vec![Value::ResourceRef(
                Iri::parse("urn:eigenius:institution:Institution").unwrap(),
            )]),
        );
        inst.set(
            Iri::parse("urn:eigenius:institution:institution_iri").unwrap(),
            Value::String(ext_iri.to_string()),
        );
        inst.set(
            Iri::parse("urn:eigenius:institution:institution_name").unwrap(),
            Value::String("External".to_string()),
        );
        inst.set(
            Iri::parse(wk::RUNTIME).unwrap(),
            Value::ResourceRef(Iri::parse(wk::RUNTIME_EXTERNAL).unwrap()),
        );
        let mut b = LayerBuilder::new("external_only", None);
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
            "external institutions should be skipped silently"
        );
        assert!(report.institutions_registered.is_empty());
        assert!(runtime.get(&Iri::parse(ext_iri).unwrap()).is_none());
    }
}
