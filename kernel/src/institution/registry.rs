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

//! derived institution registry.
//!
//! [`InstitutionIndex`] is built by scanning the layer chain for
//! resources of the institution-vocabulary classes — `Institution`,
//! `ExportFormat`, `ImportFormat`, `QueryClass`, `Comorphism` — and
//! collecting them into typed dispatch tables. The registry is a *pure
//! derived index*: there is no parallel source of truth in code.
//! Phase 9a rehydration, Load-time commit, and bootstrap all rebuild
//! the index from whatever the chain currently contains.
//!
//! M2 of the implementation plan — pure indexing, no dispatch yet.
//! M3 will plug the index into the kernel's evaluator paths.

use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use std::collections::BTreeMap;

/// The `is_a` meta-classes the institution index dispatches on (see
/// [`InstitutionIndex::ingest`]). A resource contributes to the index iff its `is_a`
/// directly contains one of these — so these are exactly what
/// [`resolve_typed_resources`] scans the triple index for on the commit hot path.
/// Keep in sync with `ingest`'s dispatch arms.
const INSTITUTION_METACLASSES: &[&str] = &[
    wk::COMORPHISM,
    wk::EXPORT_FORMAT_CLASS,
    wk::IMPORT_FORMAT_CLASS,
    wk::QUERY_CLASS_CLASS,
    "urn:eigenius:institution:Institution",
];

use crate::layer::resolve_typed_resources;

// ─── Typed entries derived from declaration resources ──────────────────

/// How the kernel reaches an institution at runtime. Derived from the
/// `runtime` property on an `Institution` resource. Optional because
/// `runtime` is recommended-but-not-required.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeKind {
    /// External service (gRPC, LSP, etc.).
    External,
    /// In-process Rust trait object linked into the kernel binary.
    InProcess,
}

/// One registered institution.
#[derive(Debug, Clone)]
pub struct InstitutionEntry {
    pub iri: Iri,
    pub name: String,
    pub runtime: Option<RuntimeKind>,
    /// IRI of the `RuntimeEnvironment` this institution dispatches
    /// into. Carried for `runtime: external` institutions (D31 §5);
    /// `None` for the in-process kind. Resolved from the
    /// `requires_environment` property at index time.
    pub requires_environment: Option<Iri>,
}

/// One declared `ExportFormat` — a typed outbound view of a source
/// institution's resource class.
#[derive(Debug, Clone)]
pub struct ExportFormatEntry {
    pub iri: Iri,
    pub from_class: Iri,
    pub payload_type: Iri,
    pub institution_ref: Iri,
    pub procedure: Iri,
}

/// One declared `ImportFormat` — a typed inbound constructor for a
/// target institution's resource class.
#[derive(Debug, Clone)]
pub struct ImportFormatEntry {
    pub iri: Iri,
    pub to_class: Iri,
    pub payload_type: Iri,
    pub institution_ref: Iri,
    pub procedure: Iri,
}

/// Operational profile of a `QueryClass` (D14 §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DispatchRole {
    /// Explicit invocation — EigenQL FIBER clause (D2 v2 §3.5).
    OnDemand,
    /// Fired automatically on Load when a resource of the bound query
    /// class enters the chain. Result class must be `Verdict`.
    AutoOnLoad,
    /// Fired during type-check reduction of `Exp::NativeDecide`.
    /// Result class must be `Verdict`.
    Decidable,
}

/// One declared `QueryClass` — a typed function on resources in the
/// institution's fibre, with one or more dispatch roles.
#[derive(Debug, Clone)]
pub struct QueryClassEntry {
    pub iri: Iri,
    pub query_class: Iri,
    pub result_class: Iri,
    pub dispatch_roles: Vec<DispatchRole>,
    pub query_handler: Iri,
    pub institution_ref: Iri,
}

/// One declared `Comorphism` — the triadic translation across an
/// institution boundary.
#[derive(Debug, Clone)]
pub struct ComorphismEntry {
    pub iri: Iri,
    pub export_format: Iri,
    pub transformation: Iri,
    pub import_format: Iri,
    pub exact: bool,
}

/// What kind of procedure a procedure-IRI dispatches to. The kernel
/// uses this to route to the right institution method (boundary
/// extract / reify, or institution-runtime query).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcedureKind {
    /// Procedure on an `ExportFormat` — institution's `extract_typed`.
    Extract,
    /// Procedure on an `ImportFormat` — institution's `reify`.
    Reify,
    /// Procedure on a `QueryClass` whose `query_handler` is institution-
    /// runtime (not a kernel Component) — institution's `query`.
    Query,
}

// ─── The index ─────────────────────────────────────────────────────────

/// Read-only derived index of institution declarations.
///
/// Rebuilt on bootstrap, on Phase 9a rehydration, and on every Load
/// commit. The index does not own dispatch — M3 wires it into the
/// kernel evaluator. Until then this is pure data.
#[derive(Debug, Clone, Default)]
pub struct InstitutionIndex {
    institutions: BTreeMap<Iri, InstitutionEntry>,
    export_formats: BTreeMap<Iri, ExportFormatEntry>,
    import_formats: BTreeMap<Iri, ImportFormatEntry>,
    query_classes: BTreeMap<Iri, QueryClassEntry>,
    comorphisms: BTreeMap<Iri, ComorphismEntry>,

    /// query_class IRI → list of QueryClass IRIs whose `dispatch_role`
    /// includes `AutoOnLoad`. The Load handler iterates this list when
    /// a resource of the keyed class enters the chain.
    auto_on_load_by_class: BTreeMap<Iri, Vec<Iri>>,
    /// query_class IRI → list of QueryClass IRIs whose `dispatch_role`
    /// includes `OnDemand`. The EigenQL FIBER evaluator iterates this.
    on_demand_by_class: BTreeMap<Iri, Vec<Iri>>,
    /// query_class IRI → unique QueryClass IRI whose `dispatch_role`
    /// includes `Decidable`. `NativeDecide` resolves the constraint
    /// IRI through this map. (One Decidable QueryClass per input class
    /// — if the chain declares multiple, the index records the first
    /// and emits an error for the rest.)
    decidable_by_class: BTreeMap<Iri, Iri>,
    /// Procedure IRI → (declaring institution IRI, kind). Built from
    /// the `procedure` property on every ExportFormat / ImportFormat
    /// and the `query_handler` property on every QueryClass whose
    /// handler is *not* a kernel-registered Component (M3 will refine
    /// this — for now we record every handler IRI as `Query`).
    procedures: BTreeMap<Iri, (Iri, ProcedureKind)>,
}

/// One problem encountered while indexing — a malformed declaration
/// resource. The index keeps the well-formed entries it could parse and
/// returns the rest as errors so callers can surface them as
/// validation problems.
#[derive(Debug, Clone)]
pub struct IndexError {
    pub resource_iri: Option<Iri>,
    pub kind: &'static str,
    pub reason: String,
}

impl InstitutionIndex {
    /// Empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build an index by walking the chain rooted at `layer`, top-layer
    /// resources overriding parents. Returns the index plus any
    /// per-resource parse errors encountered. Well-formed entries are
    /// always indexed even when other entries fail.
    pub fn from_layer(layer: &Layer) -> (Self, Vec<IndexError>) {
        let mut idx = Self::new();
        let mut errors = Vec::new();

        for (_iri, resource) in layer.iter_all_resources() {
            idx.ingest(&resource, &mut errors);
        }

        (idx, errors)
    }

    /// Like [`from_layer`](Self::from_layer) but discovers the institution-related
    /// resources through the triple index ([`resolve_typed_resources`]) instead of
    /// materialising the entire chain. O(institution declarations), not O(chain) —
    /// the per-commit rebuild path (D14 / D23). Produces an identical index to
    /// `from_layer` on any chain rooted at the core ontology (proven by the
    /// `indexed_rebuild_matches_full_scan` test); use `from_layer` for bare fixture
    /// chains with no core (where `is_a` isn't indexed).
    pub fn from_layer_indexed(layer: &Layer) -> (Self, Vec<IndexError>) {
        let mut idx = Self::new();
        let mut errors = Vec::new();

        for resource in resolve_typed_resources(layer, INSTITUTION_METACLASSES) {
            idx.ingest(&resource, &mut errors);
        }

        (idx, errors)
    }

    /// Ingest a single resource, dispatching by its `is_a`.
    fn ingest(&mut self, resource: &Resource, errors: &mut Vec<IndexError>) {
        let is_a = is_a_iris(resource);

        if is_a.iter().any(|i| i == wk::COMORPHISM) {
            match parse_comorphism(resource) {
                Ok(entry) => {
                    self.comorphisms.insert(entry.iri.clone(), entry);
                }
                Err(reason) => errors.push(IndexError {
                    resource_iri: resource.id().cloned(),
                    kind: "Comorphism",
                    reason,
                }),
            }
        } else if is_a.iter().any(|i| i == wk::EXPORT_FORMAT_CLASS) {
            match parse_export_format(resource) {
                Ok(entry) => {
                    self.procedures.insert(
                        entry.procedure.clone(),
                        (entry.institution_ref.clone(), ProcedureKind::Extract),
                    );
                    self.export_formats.insert(entry.iri.clone(), entry);
                }
                Err(reason) => errors.push(IndexError {
                    resource_iri: resource.id().cloned(),
                    kind: "ExportFormat",
                    reason,
                }),
            }
        } else if is_a.iter().any(|i| i == wk::IMPORT_FORMAT_CLASS) {
            match parse_import_format(resource) {
                Ok(entry) => {
                    self.procedures.insert(
                        entry.procedure.clone(),
                        (entry.institution_ref.clone(), ProcedureKind::Reify),
                    );
                    self.import_formats.insert(entry.iri.clone(), entry);
                }
                Err(reason) => errors.push(IndexError {
                    resource_iri: resource.id().cloned(),
                    kind: "ImportFormat",
                    reason,
                }),
            }
        } else if is_a.iter().any(|i| i == wk::QUERY_CLASS_CLASS) {
            match parse_query_class(resource) {
                Ok(entry) => {
                    self.add_query_class(entry, errors);
                }
                Err(reason) => errors.push(IndexError {
                    resource_iri: resource.id().cloned(),
                    kind: "QueryClass",
                    reason,
                }),
            }
        } else if is_a
            .iter()
            .any(|i| i == "urn:eigenius:institution:Institution")
        {
            match parse_institution(resource) {
                Ok(entry) => {
                    self.institutions.insert(entry.iri.clone(), entry);
                }
                Err(reason) => errors.push(IndexError {
                    resource_iri: resource.id().cloned(),
                    kind: "Institution",
                    reason,
                }),
            }
        }
    }

    fn add_query_class(&mut self, entry: QueryClassEntry, errors: &mut Vec<IndexError>) {
        // Update the dispatch sub-indexes from the parsed roles.
        for role in &entry.dispatch_roles {
            match role {
                DispatchRole::AutoOnLoad => self
                    .auto_on_load_by_class
                    .entry(entry.query_class.clone())
                    .or_default()
                    .push(entry.iri.clone()),
                DispatchRole::OnDemand => self
                    .on_demand_by_class
                    .entry(entry.query_class.clone())
                    .or_default()
                    .push(entry.iri.clone()),
                DispatchRole::Decidable => {
                    if let Some(existing) = self.decidable_by_class.get(&entry.query_class) {
                        if existing != &entry.iri {
                            errors.push(IndexError {
                                resource_iri: Some(entry.iri.clone()),
                                kind: "QueryClass",
                                reason: format!(
                                    "duplicate Decidable QueryClass for input class `{}`: \
                                     already bound to `{}`",
                                    entry.query_class, existing
                                ),
                            });
                            continue;
                        }
                    }
                    self.decidable_by_class
                        .insert(entry.query_class.clone(), entry.iri.clone());
                }
            }
        }
        // Record the handler IRI as a procedure dispatched to the
        // institution. M3 will distinguish Component-handler vs.
        // institution-handler at this point — until then every handler
        // is recorded as `Query` and the actual dispatch decision is
        // deferred.
        self.procedures.insert(
            entry.query_handler.clone(),
            (entry.institution_ref.clone(), ProcedureKind::Query),
        );
        self.query_classes.insert(entry.iri.clone(), entry);
    }

    // ─── Accessors ─────────────────────────────────────────────────

    pub fn institutions(&self) -> impl Iterator<Item = &InstitutionEntry> {
        self.institutions.values()
    }
    pub fn institution(&self, iri: &Iri) -> Option<&InstitutionEntry> {
        self.institutions.get(iri)
    }

    pub fn export_format(&self, iri: &Iri) -> Option<&ExportFormatEntry> {
        self.export_formats.get(iri)
    }
    pub fn import_format(&self, iri: &Iri) -> Option<&ImportFormatEntry> {
        self.import_formats.get(iri)
    }
    pub fn query_class(&self, iri: &Iri) -> Option<&QueryClassEntry> {
        self.query_classes.get(iri)
    }
    pub fn comorphism(&self, iri: &Iri) -> Option<&ComorphismEntry> {
        self.comorphisms.get(iri)
    }

    pub fn export_formats(&self) -> impl Iterator<Item = &ExportFormatEntry> {
        self.export_formats.values()
    }
    pub fn import_formats(&self) -> impl Iterator<Item = &ImportFormatEntry> {
        self.import_formats.values()
    }
    pub fn query_classes(&self) -> impl Iterator<Item = &QueryClassEntry> {
        self.query_classes.values()
    }
    pub fn comorphisms(&self) -> impl Iterator<Item = &ComorphismEntry> {
        self.comorphisms.values()
    }

    /// QueryClasses whose `dispatch_role` includes `AutoOnLoad`, keyed
    /// by their input class IRI. The Load handler iterates this list
    /// when a resource of the keyed class enters the chain.
    pub fn auto_on_load_for(&self, query_class_iri: &Iri) -> &[Iri] {
        self.auto_on_load_by_class
            .get(query_class_iri)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// QueryClasses whose `dispatch_role` includes `OnDemand`, keyed
    /// by their input class IRI.
    pub fn on_demand_for(&self, query_class_iri: &Iri) -> &[Iri] {
        self.on_demand_by_class
            .get(query_class_iri)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// QueryClass IRI for a `Decidable`-dispatched constraint, keyed
    /// by the constraint's input class IRI. `None` when the chain
    /// declares no Decidable QueryClass for this class.
    pub fn decidable_for(&self, query_class_iri: &Iri) -> Option<&Iri> {
        self.decidable_by_class.get(query_class_iri)
    }

    /// Procedure IRI → (declaring institution IRI, kind). Used by the
    /// kernel to route boundary calls and institution-runtime queries.
    pub fn procedure(&self, procedure_iri: &Iri) -> Option<(&Iri, ProcedureKind)> {
        self.procedures
            .get(procedure_iri)
            .map(|(inst, kind)| (inst, *kind))
    }

    pub fn is_empty(&self) -> bool {
        self.institutions.is_empty()
            && self.export_formats.is_empty()
            && self.import_formats.is_empty()
            && self.query_classes.is_empty()
            && self.comorphisms.is_empty()
    }
}

// ─── Parsing helpers ──────────────────────────────────────────────────

fn is_a_iris(resource: &Resource) -> Vec<String> {
    resource.is_a().into_iter().map(|i| i.to_string()).collect()
}

fn require_iri_property(resource: &Resource, property: &str, label: &str) -> Result<Iri, String> {
    let prop_iri = Iri::parse(property).expect("well-known IRI");
    match resource.get(&prop_iri) {
        // `ResourceRef` is the canonical shape post-`canonicalise_resource_refs`;
        // `String` is accepted as a fallback for resources read off the wire
        // before canonicalisation (in-flight gRPC payloads, FIBER-synthesised
        // intermediates) where the layer's data_type pass hasn't run.
        Some(Value::ResourceRef(i)) => Ok(i.clone()),
        Some(Value::String(s)) => {
            Iri::parse(s).map_err(|e| format!("{label}: invalid IRI `{s}`: {e}"))
        }
        Some(other) => Err(format!("{label}: expected IRI reference, got {other:?}")),
        None => Err(format!("{label}: missing")),
    }
}

fn require_string_property(
    resource: &Resource,
    property: &str,
    label: &str,
) -> Result<String, String> {
    let prop_iri = Iri::parse(property).expect("well-known IRI");
    match resource.get(&prop_iri) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(other) => Err(format!("{label}: expected string, got {other:?}")),
        None => Err(format!("{label}: missing")),
    }
}

fn optional_iri_property(
    resource: &Resource,
    property: &str,
    label: &str,
) -> Result<Option<Iri>, String> {
    let prop_iri = Iri::parse(property).expect("well-known IRI");
    match resource.get(&prop_iri) {
        None => Ok(None),
        Some(Value::ResourceRef(i)) => Ok(Some(i.clone())),
        Some(Value::String(s)) => Iri::parse(s)
            .map(Some)
            .map_err(|e| format!("{label}: invalid IRI `{s}`: {e}")),
        Some(other) => Err(format!("{label}: expected IRI reference, got {other:?}")),
    }
}

fn require_id(resource: &Resource, label: &str) -> Result<Iri, String> {
    resource
        .id()
        .cloned()
        .ok_or_else(|| format!("{label}: resource has no @id"))
}

fn parse_institution(resource: &Resource) -> Result<InstitutionEntry, String> {
    let iri = require_id(resource, "Institution")?;
    let institution_iri = require_iri_property(
        resource,
        "urn:eigenius:institution:institution_iri",
        "Institution.institution_iri",
    )?;
    // Institution's own `@id` and the `institution_iri` property are
    // expected to agree; if they differ, the index uses the
    // `institution_iri` property as authoritative (matches the
    // dispatch keys used elsewhere).
    let _ = iri;
    let name = require_string_property(
        resource,
        "urn:eigenius:institution:institution_name",
        "Institution.institution_name",
    )?;
    let runtime = optional_iri_property(resource, wk::RUNTIME, "Institution.runtime")?
        .map(|i| parse_runtime_kind(i.as_str()))
        .transpose()?;
    let requires_environment = optional_iri_property(
        resource,
        wk::INSTITUTION_REQUIRES_ENVIRONMENT,
        "Institution.requires_environment",
    )?;
    Ok(InstitutionEntry {
        iri: institution_iri,
        name,
        runtime,
        requires_environment,
    })
}

fn parse_runtime_kind(s: &str) -> Result<RuntimeKind, String> {
    match s {
        wk::RUNTIME_EXTERNAL => Ok(RuntimeKind::External),
        wk::RUNTIME_IN_PROCESS => Ok(RuntimeKind::InProcess),
        other => Err(format!("unknown runtime IRI `{other}`")),
    }
}

fn parse_dispatch_role(s: &str) -> Result<DispatchRole, String> {
    match s {
        wk::DISPATCH_ON_DEMAND => Ok(DispatchRole::OnDemand),
        wk::DISPATCH_AUTO_ON_LOAD => Ok(DispatchRole::AutoOnLoad),
        wk::DISPATCH_DECIDABLE => Ok(DispatchRole::Decidable),
        other => Err(format!("unknown dispatch_role IRI `{other}`")),
    }
}

fn parse_export_format(resource: &Resource) -> Result<ExportFormatEntry, String> {
    let iri = require_id(resource, "ExportFormat")?;
    let from_class = require_iri_property(resource, wk::FROM_CLASS, "ExportFormat.from_class")?;
    let payload_type =
        require_iri_property(resource, wk::PAYLOAD_TYPE, "ExportFormat.payload_type")?;
    let institution_ref = require_iri_property(
        resource,
        "urn:eigenius:institution:institution_ref",
        "ExportFormat.institution_ref",
    )?;
    let procedure = require_iri_property(resource, wk::PROCEDURE, "ExportFormat.procedure")?;
    Ok(ExportFormatEntry {
        iri,
        from_class,
        payload_type,
        institution_ref,
        procedure,
    })
}

fn parse_import_format(resource: &Resource) -> Result<ImportFormatEntry, String> {
    let iri = require_id(resource, "ImportFormat")?;
    let to_class = require_iri_property(resource, wk::TO_CLASS, "ImportFormat.to_class")?;
    let payload_type =
        require_iri_property(resource, wk::PAYLOAD_TYPE, "ImportFormat.payload_type")?;
    let institution_ref = require_iri_property(
        resource,
        "urn:eigenius:institution:institution_ref",
        "ImportFormat.institution_ref",
    )?;
    let procedure = require_iri_property(resource, wk::PROCEDURE, "ImportFormat.procedure")?;
    Ok(ImportFormatEntry {
        iri,
        to_class,
        payload_type,
        institution_ref,
        procedure,
    })
}

fn parse_query_class(resource: &Resource) -> Result<QueryClassEntry, String> {
    let iri = require_id(resource, "QueryClass")?;
    let query_class = require_iri_property(resource, wk::QUERY_CLASS, "QueryClass.query_class")?;
    let result_class = require_iri_property(resource, wk::RESULT_CLASS, "QueryClass.result_class")?;
    let query_handler =
        require_iri_property(resource, wk::QUERY_HANDLER, "QueryClass.query_handler")?;
    let institution_ref = require_iri_property(
        resource,
        "urn:eigenius:institution:institution_ref",
        "QueryClass.institution_ref",
    )?;

    let prop_iri = Iri::parse(wk::DISPATCH_ROLE).expect("well-known IRI");
    let role_values = match resource.get(&prop_iri) {
        Some(Value::Array(items)) => items.clone(),
        Some(other) => {
            return Err(format!(
                "QueryClass.dispatch_role: expected resource_array, got {other:?}"
            ))
        }
        None => return Err("QueryClass.dispatch_role: missing".to_string()),
    };
    if role_values.is_empty() {
        return Err("QueryClass.dispatch_role: empty array".to_string());
    }
    let mut dispatch_roles = Vec::with_capacity(role_values.len());
    for v in &role_values {
        match v {
            // Post-canonicalisation, IRI references in resource_array
            // values are `ResourceRef`; `String` is a parse-time
            // fallback for intermediate (uncommitted) shapes.
            Value::ResourceRef(i) => dispatch_roles.push(parse_dispatch_role(i.as_str())?),
            Value::String(s) => dispatch_roles.push(parse_dispatch_role(s)?),
            other => {
                return Err(format!(
                    "QueryClass.dispatch_role: expected IRI reference, got {other:?}"
                ))
            }
        }
    }

    Ok(QueryClassEntry {
        iri,
        query_class,
        result_class,
        dispatch_roles,
        query_handler,
        institution_ref,
    })
}

fn parse_comorphism(resource: &Resource) -> Result<ComorphismEntry, String> {
    let iri = require_id(resource, "Comorphism")?;
    let export_format =
        require_iri_property(resource, wk::EXPORT_FORMAT, "Comorphism.export_format")?;
    let transformation =
        require_iri_property(resource, wk::TRANSFORMATION, "Comorphism.transformation")?;
    let import_format =
        require_iri_property(resource, wk::IMPORT_FORMAT, "Comorphism.import_format")?;

    let exact_iri = Iri::parse(wk::EXACT).expect("well-known IRI");
    let exact = match resource.get(&exact_iri) {
        Some(Value::Boolean(b)) => *b,
        Some(other) => return Err(format!("Comorphism.exact: expected boolean, got {other:?}")),
        None => false,
    };
    Ok(ComorphismEntry {
        iri,
        export_format,
        transformation,
        import_format,
        exact,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::LayerBuilder;
    use crate::ontology::resource::{Resource, Value};
    use std::collections::BTreeSet;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn set_str(r: &mut Resource, p: &str, v: &str) {
        r.set(iri(p), Value::String(v.to_string()));
    }

    fn set_is_a(r: &mut Resource, classes: &[&str]) {
        r.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(
                classes
                    .iter()
                    .map(|c| Value::String((*c).to_string()))
                    .collect(),
            ),
        );
    }

    /// Build a tiny test layer carrying one Institution declaration,
    /// one ExportFormat, one ImportFormat, one QueryClass, one
    /// Comorphism — all tied together. Returns the built layer.
    fn build_test_layer() -> std::sync::Arc<crate::layer::Layer> {
        build_test_layer_on(None, crate::layer::LayerStorage::in_memory())
    }

    /// Like [`build_test_layer`] but with an explicit parent + storage, so the same
    /// declarations can be committed onto a core-rooted chain (needed by the
    /// index-driven equivalence test, where `is_a` must be an indexable predicate).
    fn build_test_layer_on(
        parent: Option<std::sync::Arc<crate::layer::Layer>>,
        storage: crate::layer::LayerStorage,
    ) -> std::sync::Arc<crate::layer::Layer> {
        let mut b = LayerBuilder::new("test", parent);

        // Institution
        let mut inst = Resource::new(iri("urn:eigenius:test:inst:dock"));
        set_is_a(&mut inst, &["urn:eigenius:institution:Institution"]);
        set_str(
            &mut inst,
            "urn:eigenius:institution:institution_iri",
            "urn:eigenius:test:inst:dock",
        );
        set_str(
            &mut inst,
            "urn:eigenius:institution:institution_name",
            "Test Dock",
        );
        set_str(&mut inst, wk::RUNTIME, wk::RUNTIME_EXTERNAL);
        b.add_resource(inst).unwrap();

        let mut inst2 = Resource::new(iri("urn:eigenius:test:inst:assay"));
        set_is_a(&mut inst2, &["urn:eigenius:institution:Institution"]);
        set_str(
            &mut inst2,
            "urn:eigenius:institution:institution_iri",
            "urn:eigenius:test:inst:assay",
        );
        set_str(
            &mut inst2,
            "urn:eigenius:institution:institution_name",
            "Test Assay",
        );
        b.add_resource(inst2).unwrap();

        // ExportFormat
        let mut ef = Resource::new(iri("urn:eigenius:test:ef:dock_to_dg"));
        set_is_a(&mut ef, &[wk::EXPORT_FORMAT_CLASS]);
        set_str(&mut ef, wk::FROM_CLASS, "urn:eigenius:test:DockingResult");
        set_str(&mut ef, wk::PAYLOAD_TYPE, wk::FLOAT);
        set_str(
            &mut ef,
            "urn:eigenius:institution:institution_ref",
            "urn:eigenius:test:inst:dock",
        );
        set_str(&mut ef, wk::PROCEDURE, "urn:eigenius:test:proc:extract_dg");
        b.add_resource(ef).unwrap();

        // ImportFormat
        let mut imf = Resource::new(iri("urn:eigenius:test:if:assay_from_ic50"));
        set_is_a(&mut imf, &[wk::IMPORT_FORMAT_CLASS]);
        set_str(&mut imf, wk::TO_CLASS, "urn:eigenius:test:AssayPrediction");
        set_str(&mut imf, wk::PAYLOAD_TYPE, wk::FLOAT);
        set_str(
            &mut imf,
            "urn:eigenius:institution:institution_ref",
            "urn:eigenius:test:inst:assay",
        );
        set_str(&mut imf, wk::PROCEDURE, "urn:eigenius:test:proc:reify_ic50");
        b.add_resource(imf).unwrap();

        // QueryClass — auto-on-load Verdict-returning predicate on
        // DockingResult, plus an on-demand role.
        let mut qc = Resource::new(iri("urn:eigenius:test:qc:dock_validity"));
        set_is_a(&mut qc, &[wk::QUERY_CLASS_CLASS]);
        set_str(&mut qc, wk::QUERY_CLASS, "urn:eigenius:test:DockingResult");
        set_str(&mut qc, wk::RESULT_CLASS, wk::VERDICT);
        qc.set(
            iri(wk::DISPATCH_ROLE),
            Value::Array(vec![
                Value::String(wk::DISPATCH_AUTO_ON_LOAD.to_string()),
                Value::String(wk::DISPATCH_ON_DEMAND.to_string()),
            ]),
        );
        set_str(
            &mut qc,
            wk::QUERY_HANDLER,
            "urn:eigenius:test:proc:check_dock",
        );
        set_str(
            &mut qc,
            "urn:eigenius:institution:institution_ref",
            "urn:eigenius:test:inst:dock",
        );
        b.add_resource(qc).unwrap();

        // Comorphism — points at the export/import formats above,
        // plus a transformation Component IRI.
        let mut cm = Resource::new(iri("urn:eigenius:test:cm:dock_to_assay"));
        set_is_a(&mut cm, &[wk::COMORPHISM]);
        set_str(
            &mut cm,
            wk::EXPORT_FORMAT,
            "urn:eigenius:test:ef:dock_to_dg",
        );
        set_str(
            &mut cm,
            wk::TRANSFORMATION,
            "urn:eigenius:test:cm:arrhenius_component",
        );
        set_str(
            &mut cm,
            wk::IMPORT_FORMAT,
            "urn:eigenius:test:if:assay_from_ic50",
        );
        cm.set(iri(wk::EXACT), Value::Boolean(false));
        b.add_resource(cm).unwrap();

        std::sync::Arc::new(b.build(storage))
    }

    /// Safety net for the index-driven commit-time rebuild
    /// ([`InstitutionIndex::from_layer_indexed`]): on a realistic core-rooted chain
    /// it must produce the *same* index as the full-chain scan
    /// ([`InstitutionIndex::from_layer`]). A divergence would mean an institution
    /// kind silently stops dispatching after a commit. Core is required so `is_a` is
    /// an indexable predicate (every committed chain has it).
    #[test]
    fn indexed_rebuild_matches_full_scan() {
        let storage = crate::layer::LayerStorage::in_memory();
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let mut core = LayerBuilder::new("core", None);
        for r in crate::ontology::eigon_json::parse_document(core_json).unwrap() {
            core.add_resource(r).unwrap();
        }
        let core_layer = std::sync::Arc::new(core.build(storage.clone()));
        let layer = build_test_layer_on(Some(core_layer), storage);

        let (full, full_errs) = InstitutionIndex::from_layer(&layer);
        let (indexed, indexed_errs) = InstitutionIndex::from_layer_indexed(&layer);

        let inst = |idx: &InstitutionIndex| {
            idx.institutions()
                .map(|e| e.iri.clone())
                .collect::<BTreeSet<_>>()
        };
        let ef = |idx: &InstitutionIndex| {
            idx.export_formats()
                .map(|e| e.iri.clone())
                .collect::<BTreeSet<_>>()
        };
        let imf = |idx: &InstitutionIndex| {
            idx.import_formats()
                .map(|e| e.iri.clone())
                .collect::<BTreeSet<_>>()
        };
        let qc = |idx: &InstitutionIndex| {
            idx.query_classes()
                .map(|e| e.iri.clone())
                .collect::<BTreeSet<_>>()
        };
        let cm = |idx: &InstitutionIndex| {
            idx.comorphisms()
                .map(|e| e.iri.clone())
                .collect::<BTreeSet<_>>()
        };

        assert_eq!(inst(&full), inst(&indexed), "institutions diverge");
        assert_eq!(ef(&full), ef(&indexed), "export_formats diverge");
        assert_eq!(imf(&full), imf(&indexed), "import_formats diverge");
        assert_eq!(qc(&full), qc(&indexed), "query_classes diverge");
        assert_eq!(cm(&full), cm(&indexed), "comorphisms diverge");
        assert_eq!(full_errs.len(), indexed_errs.len(), "error counts diverge");

        // Sanity: the fixture actually has declarations (not a vacuous match).
        assert_eq!(inst(&full).len(), 2);
        assert_eq!(qc(&full).len(), 1);
        assert_eq!(cm(&full).len(), 1);
    }

    #[test]
    fn empty_layer_yields_empty_index() {
        let layer = std::sync::Arc::new(
            LayerBuilder::new("empty", None).build(crate::layer::LayerStorage::in_memory()),
        );
        let (idx, errors) = InstitutionIndex::from_layer(&layer);
        assert!(idx.is_empty());
        assert!(errors.is_empty());
    }

    #[test]
    fn full_round_trip_indexes_every_declaration() {
        let layer = build_test_layer();
        let (idx, errors) = InstitutionIndex::from_layer(&layer);
        assert!(
            errors.is_empty(),
            "expected no parse errors, got {errors:?}"
        );

        // Institutions
        assert_eq!(idx.institutions().count(), 2);
        let dock = idx
            .institution(&iri("urn:eigenius:test:inst:dock"))
            .unwrap();
        assert_eq!(dock.name, "Test Dock");
        assert_eq!(dock.runtime, Some(RuntimeKind::External));
        let assay = idx
            .institution(&iri("urn:eigenius:test:inst:assay"))
            .unwrap();
        assert_eq!(assay.runtime, None);

        // ExportFormat
        let ef = idx
            .export_format(&iri("urn:eigenius:test:ef:dock_to_dg"))
            .unwrap();
        assert_eq!(ef.from_class.as_str(), "urn:eigenius:test:DockingResult");
        assert_eq!(ef.payload_type.as_str(), wk::FLOAT);
        assert_eq!(ef.institution_ref.as_str(), "urn:eigenius:test:inst:dock");

        // ImportFormat
        let imf = idx
            .import_format(&iri("urn:eigenius:test:if:assay_from_ic50"))
            .unwrap();
        assert_eq!(imf.to_class.as_str(), "urn:eigenius:test:AssayPrediction");

        // QueryClass + dispatch sub-indexes
        let qc = idx
            .query_class(&iri("urn:eigenius:test:qc:dock_validity"))
            .unwrap();
        assert_eq!(qc.result_class.as_str(), wk::VERDICT);
        assert!(qc.dispatch_roles.contains(&DispatchRole::AutoOnLoad));
        assert!(qc.dispatch_roles.contains(&DispatchRole::OnDemand));
        let auto = idx.auto_on_load_for(&iri("urn:eigenius:test:DockingResult"));
        assert_eq!(auto.len(), 1);
        assert_eq!(auto[0].as_str(), "urn:eigenius:test:qc:dock_validity");
        let on_demand = idx.on_demand_for(&iri("urn:eigenius:test:DockingResult"));
        assert_eq!(on_demand.len(), 1);

        // Comorphism
        let cm = idx
            .comorphism(&iri("urn:eigenius:test:cm:dock_to_assay"))
            .unwrap();
        assert_eq!(cm.export_format.as_str(), "urn:eigenius:test:ef:dock_to_dg");
        assert!(!cm.exact);

        // Procedure dispatch table
        let (extract_inst, kind) = idx
            .procedure(&iri("urn:eigenius:test:proc:extract_dg"))
            .unwrap();
        assert_eq!(extract_inst.as_str(), "urn:eigenius:test:inst:dock");
        assert_eq!(kind, ProcedureKind::Extract);
        let (reify_inst, kind) = idx
            .procedure(&iri("urn:eigenius:test:proc:reify_ic50"))
            .unwrap();
        assert_eq!(reify_inst.as_str(), "urn:eigenius:test:inst:assay");
        assert_eq!(kind, ProcedureKind::Reify);
        let (q_inst, kind) = idx
            .procedure(&iri("urn:eigenius:test:proc:check_dock"))
            .unwrap();
        assert_eq!(q_inst.as_str(), "urn:eigenius:test:inst:dock");
        assert_eq!(kind, ProcedureKind::Query);
    }

    #[test]
    fn malformed_comorphism_yields_error_but_keeps_other_entries() {
        let mut b = LayerBuilder::new("test", None);

        // A well-formed Institution.
        let mut inst = Resource::new(iri("urn:eigenius:test:inst:ok"));
        set_is_a(&mut inst, &["urn:eigenius:institution:Institution"]);
        set_str(
            &mut inst,
            "urn:eigenius:institution:institution_iri",
            "urn:eigenius:test:inst:ok",
        );
        set_str(&mut inst, "urn:eigenius:institution:institution_name", "OK");
        b.add_resource(inst).unwrap();

        // A Comorphism missing its `transformation` property.
        let mut cm = Resource::new(iri("urn:eigenius:test:cm:bad"));
        set_is_a(&mut cm, &[wk::COMORPHISM]);
        set_str(&mut cm, wk::EXPORT_FORMAT, "urn:eigenius:test:ef:x");
        set_str(&mut cm, wk::IMPORT_FORMAT, "urn:eigenius:test:if:y");
        // transformation deliberately omitted
        b.add_resource(cm).unwrap();

        let layer = std::sync::Arc::new(b.build(crate::layer::LayerStorage::in_memory()));
        let (idx, errors) = InstitutionIndex::from_layer(&layer);

        // Well-formed Institution still indexed.
        assert!(idx.institution(&iri("urn:eigenius:test:inst:ok")).is_some());
        // Malformed Comorphism is *not* indexed.
        assert!(idx.comorphism(&iri("urn:eigenius:test:cm:bad")).is_none());
        // Error reported.
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, "Comorphism");
        assert!(errors[0].reason.contains("transformation"));
    }

    #[test]
    fn duplicate_decidable_for_same_class_reports_error() {
        let mut b = LayerBuilder::new("test", None);

        for i in 0..2 {
            let mut qc = Resource::new(iri(&format!("urn:eigenius:test:qc:dup{i}")));
            set_is_a(&mut qc, &[wk::QUERY_CLASS_CLASS]);
            set_str(&mut qc, wk::QUERY_CLASS, "urn:eigenius:test:Input");
            set_str(&mut qc, wk::RESULT_CLASS, wk::VERDICT);
            qc.set(
                iri(wk::DISPATCH_ROLE),
                Value::Array(vec![Value::String(wk::DISPATCH_DECIDABLE.to_string())]),
            );
            set_str(
                &mut qc,
                wk::QUERY_HANDLER,
                &format!("urn:eigenius:test:proc:dup{i}"),
            );
            set_str(
                &mut qc,
                "urn:eigenius:institution:institution_ref",
                "urn:eigenius:test:inst:x",
            );
            b.add_resource(qc).unwrap();
        }
        let layer = std::sync::Arc::new(b.build(crate::layer::LayerStorage::in_memory()));
        let (idx, errors) = InstitutionIndex::from_layer(&layer);

        // First Decidable wins; second emits an error.
        // (BTreeMap iteration order is by IRI — `dup0` < `dup1`, so
        // `dup0` is the first ingested and the conflict is on `dup1`.)
        assert!(idx.decidable_for(&iri("urn:eigenius:test:Input")).is_some());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].reason.contains("duplicate Decidable"));
    }
}
