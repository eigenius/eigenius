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

//! Institution helpers for WASM fiber reasoners.
//!
//! Provides resource builders for the D14 institution declaration
//! vocabulary — `Institution`, `ExportFormat`, `ImportFormat`,
//! `QueryClass`, `Comorphism` — which a guest component constructs and
//! ships to the kernel as ordinary typed Resources (Eigon-JSON or
//! Eigon-CBOR), per D14 §3 / §4.
//!
//! The legacy `FiberDeclaration` struct + `MorphismValidation` enum
//! (used by the pre-D14 `eigenius-institution` WIT world) are retained
//! only so the kernel-side `FiberReasoner` trait still compiles while
//! the D14 migration is in flight; B4 of the retirement plan deletes
//! both ends together with the trait.

use crate::iri as wk;
use crate::{Resource, Value};

/// Declaration of a fiber reasoner's structure, matching the kernel's
/// `FiberDeclaration` struct. Serializes into a Resource with the
/// institution property names the kernel expects.
pub struct FiberDeclaration {
    pub institution_iri: String,
    pub name: String,
    pub morphism_types: Vec<Resource>,
    pub query_types: Vec<Resource>,
    pub structural_properties: Vec<Resource>,
}

impl FiberDeclaration {
    /// Convert this declaration into a CBOR-serializable Resource.
    pub fn into_resource(self) -> Resource {
        let mut r = Resource::new();
        r.set(
            "urn:eigenius:institution:institution_iri",
            Value::String(self.institution_iri),
        );
        r.set(
            "urn:eigenius:institution:institution_name",
            Value::String(self.name),
        );
        if !self.morphism_types.is_empty() {
            r.set(
                "urn:eigenius:institution:morphism_types",
                Value::Array(
                    self.morphism_types
                        .into_iter()
                        .map(|r| Value::Embedded(Box::new(r)))
                        .collect(),
                ),
            );
        }
        if !self.query_types.is_empty() {
            r.set(
                "urn:eigenius:institution:query_types",
                Value::Array(
                    self.query_types
                        .into_iter()
                        .map(|r| Value::Embedded(Box::new(r)))
                        .collect(),
                ),
            );
        }
        if !self.structural_properties.is_empty() {
            r.set(
                "urn:eigenius:institution:structural_properties",
                Value::Array(
                    self.structural_properties
                        .into_iter()
                        .map(|r| Value::Embedded(Box::new(r)))
                        .collect(),
                ),
            );
        }
        r
    }
}

/// Result of morphism validation. Matches the kernel's `MorphismValidation`
/// enum and the `validation-result` WIT enum.
#[derive(Debug, Clone, PartialEq)]
pub enum MorphismValidation {
    Valid,
    Invalid(String),
    Undecidable,
}

// ─── D14 declaration builders ────────────────────────────────────────────

/// How the kernel reaches an institution at runtime (D14 §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeKind {
    /// WASM Component Model binary hosted via Wasmtime.
    Wasm,
    /// External service reached via gRPC, LSP, etc.
    External,
    /// In-process Rust trait object linked into the kernel binary.
    InProcess,
}

impl RuntimeKind {
    pub fn iri(self) -> &'static str {
        match self {
            RuntimeKind::Wasm => wk::RUNTIME_WASM,
            RuntimeKind::External => wk::RUNTIME_EXTERNAL,
            RuntimeKind::InProcess => wk::RUNTIME_IN_PROCESS,
        }
    }
}

/// Operational profile of a `QueryClass` (D14 §4.4 / §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchRole {
    /// Explicit invocation: EigenQL FIBER clause or RunFiberQuery RPC.
    OnDemand,
    /// Fired automatically on Load when a resource of the bound query
    /// class enters the chain. Result class must be `Verdict`.
    AutoOnLoad,
    /// Fired during type-check reduction of `Exp::NativeDecide`.
    /// Result class must be `Verdict`.
    Decidable,
}

impl DispatchRole {
    pub fn iri(self) -> &'static str {
        match self {
            DispatchRole::OnDemand => wk::DISPATCH_ON_DEMAND,
            DispatchRole::AutoOnLoad => wk::DISPATCH_AUTO_ON_LOAD,
            DispatchRole::Decidable => wk::DISPATCH_DECIDABLE,
        }
    }
}

/// Builder for an `Institution` declaration resource.
///
/// ```ignore
/// let inst = InstitutionDecl::new("urn:example:dock", "Dock Institution")
///     .with_runtime(RuntimeKind::Wasm)
///     .with_description("Molecular docking institution.")
///     .build();
/// ```
pub struct InstitutionDecl {
    institution_iri: String,
    name: String,
    runtime: Option<RuntimeKind>,
    description: Option<String>,
}

impl InstitutionDecl {
    pub fn new(institution_iri: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            institution_iri: institution_iri.into(),
            name: name.into(),
            runtime: None,
            description: None,
        }
    }

    pub fn with_runtime(mut self, runtime: RuntimeKind) -> Self {
        self.runtime = Some(runtime);
        self
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    pub fn build(self) -> Resource {
        let mut r = Resource::with_id(&self.institution_iri);
        r.set_is_a([wk::INSTITUTION_CLASS]);
        r.set(wk::INSTITUTION_IRI, Value::String(self.institution_iri));
        r.set(wk::INSTITUTION_NAME, Value::String(self.name));
        if let Some(rt) = self.runtime {
            r.set(wk::RUNTIME, Value::String(rt.iri().to_string()));
        }
        if let Some(d) = self.description {
            r.set(wk::DESCRIPTION, Value::String(d));
        }
        r
    }
}

/// Builder for an `ExportFormat` declaration — the source-side typed
/// view a comorphism's `s` is dispatched to.
pub struct ExportFormatDecl {
    iri: String,
    from_class: String,
    payload_type: String,
    institution_ref: String,
    procedure: String,
    description: Option<String>,
}

impl ExportFormatDecl {
    pub fn new(
        iri: impl Into<String>,
        from_class: impl Into<String>,
        payload_type: impl Into<String>,
        institution_ref: impl Into<String>,
        procedure: impl Into<String>,
    ) -> Self {
        Self {
            iri: iri.into(),
            from_class: from_class.into(),
            payload_type: payload_type.into(),
            institution_ref: institution_ref.into(),
            procedure: procedure.into(),
            description: None,
        }
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    pub fn build(self) -> Resource {
        let mut r = Resource::with_id(&self.iri);
        r.set_is_a([wk::EXPORT_FORMAT_CLASS]);
        r.set(wk::FROM_CLASS, Value::String(self.from_class));
        r.set(wk::PAYLOAD_TYPE, Value::String(self.payload_type));
        r.set(wk::INSTITUTION_REF, Value::String(self.institution_ref));
        r.set(wk::PROCEDURE, Value::String(self.procedure));
        if let Some(d) = self.description {
            r.set(wk::DESCRIPTION, Value::String(d));
        }
        r
    }
}

/// Builder for an `ImportFormat` declaration — the target-side typed
/// constructor a comorphism's `t` is dispatched to.
pub struct ImportFormatDecl {
    iri: String,
    to_class: String,
    payload_type: String,
    institution_ref: String,
    procedure: String,
    description: Option<String>,
}

impl ImportFormatDecl {
    pub fn new(
        iri: impl Into<String>,
        to_class: impl Into<String>,
        payload_type: impl Into<String>,
        institution_ref: impl Into<String>,
        procedure: impl Into<String>,
    ) -> Self {
        Self {
            iri: iri.into(),
            to_class: to_class.into(),
            payload_type: payload_type.into(),
            institution_ref: institution_ref.into(),
            procedure: procedure.into(),
            description: None,
        }
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    pub fn build(self) -> Resource {
        let mut r = Resource::with_id(&self.iri);
        r.set_is_a([wk::IMPORT_FORMAT_CLASS]);
        r.set(wk::TO_CLASS, Value::String(self.to_class));
        r.set(wk::PAYLOAD_TYPE, Value::String(self.payload_type));
        r.set(wk::INSTITUTION_REF, Value::String(self.institution_ref));
        r.set(wk::PROCEDURE, Value::String(self.procedure));
        if let Some(d) = self.description {
            r.set(wk::DESCRIPTION, Value::String(d));
        }
        r
    }
}

/// Builder for a `QueryClass` declaration — a typed function in the
/// institution's fibre.
///
/// ```ignore
/// let qc = QueryClassDecl::new(
///     "urn:example:qc:check_dock",
///     "urn:example:DockingResult",
///     "urn:eigenius:institution:Verdict",
///     "urn:example:proc:check_dock",
///     "urn:example:dock",
/// )
/// .with_role(DispatchRole::AutoOnLoad)
/// .with_role(DispatchRole::OnDemand)
/// .build();
/// ```
pub struct QueryClassDecl {
    iri: String,
    query_class: String,
    result_class: String,
    query_handler: String,
    institution_ref: String,
    dispatch_roles: Vec<DispatchRole>,
    description: Option<String>,
}

impl QueryClassDecl {
    pub fn new(
        iri: impl Into<String>,
        query_class: impl Into<String>,
        result_class: impl Into<String>,
        query_handler: impl Into<String>,
        institution_ref: impl Into<String>,
    ) -> Self {
        Self {
            iri: iri.into(),
            query_class: query_class.into(),
            result_class: result_class.into(),
            query_handler: query_handler.into(),
            institution_ref: institution_ref.into(),
            dispatch_roles: Vec::new(),
            description: None,
        }
    }

    /// Add a dispatch role. May be called multiple times — the kernel
    /// accepts a single QueryClass declaring more than one role.
    pub fn with_role(mut self, role: DispatchRole) -> Self {
        if !self.dispatch_roles.contains(&role) {
            self.dispatch_roles.push(role);
        }
        self
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    pub fn build(self) -> Resource {
        let mut r = Resource::with_id(&self.iri);
        r.set_is_a([wk::QUERY_CLASS_CLASS]);
        r.set(wk::QUERY_CLASS, Value::String(self.query_class));
        r.set(wk::RESULT_CLASS, Value::String(self.result_class));
        r.set(wk::QUERY_HANDLER, Value::String(self.query_handler));
        r.set(wk::INSTITUTION_REF, Value::String(self.institution_ref));
        r.set(
            wk::DISPATCH_ROLE,
            Value::Array(
                self.dispatch_roles
                    .into_iter()
                    .map(|role| Value::String(role.iri().to_string()))
                    .collect(),
            ),
        );
        if let Some(d) = self.description {
            r.set(wk::DESCRIPTION, Value::String(d));
        }
        r
    }
}

/// Builder for a `Comorphism` declaration — the triadic translation
/// `(s, m, t)` across an institution boundary (D14 §5).
pub struct ComorphismDecl {
    iri: String,
    export_format: String,
    transformation: String,
    import_format: String,
    exact: bool,
    description: Option<String>,
}

impl ComorphismDecl {
    pub fn new(
        iri: impl Into<String>,
        export_format: impl Into<String>,
        transformation: impl Into<String>,
        import_format: impl Into<String>,
    ) -> Self {
        Self {
            iri: iri.into(),
            export_format: export_format.into(),
            transformation: transformation.into(),
            import_format: import_format.into(),
            exact: false,
            description: None,
        }
    }

    /// Mark this comorphism as exact. Default is `false` (the safe
    /// default). Only set to `true` when the transformation Component
    /// is provably correct in the sense of Diaconescu (2025, Thm.
    /// 14.15) — preserving the model amalgamation pullback square.
    pub fn exact(mut self, exact: bool) -> Self {
        self.exact = exact;
        self
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    pub fn build(self) -> Resource {
        let mut r = Resource::with_id(&self.iri);
        r.set_is_a([wk::COMORPHISM_CLASS]);
        r.set(wk::EXPORT_FORMAT, Value::String(self.export_format));
        r.set(wk::TRANSFORMATION, Value::String(self.transformation));
        r.set(wk::IMPORT_FORMAT, Value::String(self.import_format));
        r.set(wk::EXACT, Value::Boolean(self.exact));
        if let Some(d) = self.description {
            r.set(wk::DESCRIPTION, Value::String(d));
        }
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fiber_declaration_roundtrips() {
        let decl = FiberDeclaration {
            institution_iri: "urn:example:inst".into(),
            name: "Example".into(),
            morphism_types: vec![],
            query_types: vec![],
            structural_properties: vec![],
        };
        let r = decl.into_resource();
        assert_eq!(
            r.get_string("urn:eigenius:institution:institution_iri"),
            Some("urn:example:inst")
        );
        assert_eq!(
            r.get_string("urn:eigenius:institution:institution_name"),
            Some("Example")
        );
    }

    #[test]
    fn fiber_declaration_with_morphism_types() {
        let mut morphism = Resource::with_id("urn:example:Refinement");
        morphism.set_is_a(["urn:eigenius:core:Class"]);
        morphism.set(
            "urn:eigenius:core:short_name",
            Value::String("Refinement".into()),
        );

        let decl = FiberDeclaration {
            institution_iri: "urn:example:inst".into(),
            name: "Example".into(),
            morphism_types: vec![morphism],
            query_types: vec![],
            structural_properties: vec![],
        };

        let r = decl.into_resource();
        let morphisms = r
            .get("urn:eigenius:institution:morphism_types")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(morphisms.len(), 1);
    }

    // ─── D14 declaration-builder tests ──────────────────────────────

    fn first_is_a(r: &Resource) -> &str {
        let v = r.get(wk::IS_A).unwrap().as_array().unwrap();
        match &v[0] {
            Value::String(s) => s.as_str(),
            other => panic!("expected is_a string, got {other:?}"),
        }
    }

    #[test]
    fn institution_decl_with_runtime_round_trip() {
        let inst = InstitutionDecl::new("urn:example:dock", "Dock Institution")
            .with_runtime(RuntimeKind::Wasm)
            .with_description("Molecular docking.")
            .build();

        assert_eq!(first_is_a(&inst), wk::INSTITUTION_CLASS);
        assert_eq!(
            inst.get_string(wk::INSTITUTION_IRI),
            Some("urn:example:dock")
        );
        assert_eq!(
            inst.get_string(wk::INSTITUTION_NAME),
            Some("Dock Institution")
        );
        assert_eq!(inst.get_string(wk::RUNTIME), Some(wk::RUNTIME_WASM));
    }

    #[test]
    fn institution_decl_runtime_optional() {
        let inst = InstitutionDecl::new("urn:example:no_runtime", "X").build();
        assert!(inst.get(wk::RUNTIME).is_none());
    }

    #[test]
    fn export_format_decl_carries_required_properties() {
        let ef = ExportFormatDecl::new(
            "urn:example:ef:dock_to_dg",
            "urn:example:DockingResult",
            "urn:eigenius:core:float",
            "urn:example:dock",
            "urn:example:proc:extract_dg",
        )
        .build();
        assert_eq!(first_is_a(&ef), wk::EXPORT_FORMAT_CLASS);
        assert_eq!(
            ef.get_string(wk::FROM_CLASS),
            Some("urn:example:DockingResult")
        );
        assert_eq!(
            ef.get_string(wk::PAYLOAD_TYPE),
            Some("urn:eigenius:core:float")
        );
        assert_eq!(ef.get_string(wk::INSTITUTION_REF), Some("urn:example:dock"));
        assert_eq!(
            ef.get_string(wk::PROCEDURE),
            Some("urn:example:proc:extract_dg")
        );
    }

    #[test]
    fn import_format_decl_carries_required_properties() {
        let imf = ImportFormatDecl::new(
            "urn:example:if:assay_from_ic50",
            "urn:example:AssayPrediction",
            "urn:eigenius:core:float",
            "urn:example:assay",
            "urn:example:proc:reify_ic50",
        )
        .build();
        assert_eq!(first_is_a(&imf), wk::IMPORT_FORMAT_CLASS);
        assert_eq!(
            imf.get_string(wk::TO_CLASS),
            Some("urn:example:AssayPrediction")
        );
    }

    #[test]
    fn query_class_decl_supports_multiple_roles() {
        let qc = QueryClassDecl::new(
            "urn:example:qc:check_dock",
            "urn:example:DockingResult",
            wk::VERDICT,
            "urn:example:proc:check_dock",
            "urn:example:dock",
        )
        .with_role(DispatchRole::AutoOnLoad)
        .with_role(DispatchRole::OnDemand)
        // Duplicate roles are deduplicated:
        .with_role(DispatchRole::AutoOnLoad)
        .build();

        let roles = qc.get(wk::DISPATCH_ROLE).unwrap().as_array().unwrap();
        assert_eq!(roles.len(), 2);
        let role_iris: Vec<&str> = roles
            .iter()
            .map(|v| match v {
                Value::String(s) => s.as_str(),
                _ => panic!("expected string"),
            })
            .collect();
        assert!(role_iris.contains(&wk::DISPATCH_AUTO_ON_LOAD));
        assert!(role_iris.contains(&wk::DISPATCH_ON_DEMAND));
        assert_eq!(qc.get_string(wk::RESULT_CLASS), Some(wk::VERDICT));
    }

    #[test]
    fn comorphism_decl_defaults_to_inexact() {
        let cm = ComorphismDecl::new(
            "urn:example:cm:dock_to_assay",
            "urn:example:ef:dock_to_dg",
            "urn:example:cm:arrhenius_component",
            "urn:example:if:assay_from_ic50",
        )
        .with_description("IC50 ≈ exp(-ΔG / RT)")
        .build();

        assert_eq!(first_is_a(&cm), wk::COMORPHISM_CLASS);
        assert_eq!(
            cm.get_string(wk::EXPORT_FORMAT),
            Some("urn:example:ef:dock_to_dg")
        );
        assert_eq!(
            cm.get_string(wk::TRANSFORMATION),
            Some("urn:example:cm:arrhenius_component")
        );
        assert_eq!(
            cm.get(wk::EXACT),
            Some(&Value::Boolean(false)),
            "default exactness is `false`"
        );
    }

    #[test]
    fn comorphism_decl_with_exact_true() {
        let cm = ComorphismDecl::new("urn:example:cm:id", "urn:e:ef", "urn:e:cm", "urn:e:if")
            .exact(true)
            .build();
        assert_eq!(cm.get(wk::EXACT), Some(&Value::Boolean(true)));
    }
}
