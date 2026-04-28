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

//! Institution support — both the legacy Phase-11d trait surface
//! (`FiberReasoner` etc., kept for now while the new D14 implementation
//! lands milestone-by-milestone) and the new D14 derived registry
//! (`registry::InstitutionIndex`).
//!
//! See design document D14 for the canonical specification. The legacy
//! types in this module file are removed in M3 once D14's `Institution`
//! trait + dispatch model take over.

pub mod comorphism;
pub mod error;
pub mod registry;

use crate::context::ExecutionContext;
use crate::institution::error::{InstitutionError, MorphismValidation};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use std::collections::BTreeMap;

/// The interface by which an institution exposes reasoning about
/// its internal fiber structure to the kernel.
///
/// Each method has a distinct role in the system lifecycle:
/// - `fiber_declaration`: called once at registration
/// - `validate_morphism`: called at load time when a morphism enters the graph
/// - `query`: called at evaluation time when a program needs fiber reasoning
/// - `discover_morphisms`: called on explicit request by a program
pub trait FiberReasoner: Send + Sync {
    /// Declare this institution's fiber structure.
    /// Called once at registration time.
    fn fiber_declaration(&self) -> FiberDeclaration;

    /// Execute a fiber query.
    /// The query is a typed Eigon resource (subclass of FiberQuery).
    /// Returns a typed result resource.
    fn query(&self, query: &Resource, ctx: &ExecutionContext)
        -> Result<Resource, InstitutionError>;

    /// Validate a claimed morphism against the institution's domain logic.
    /// Structural validation (required properties, types) is the kernel's job.
    /// This checks domain-specific validity.
    fn validate_morphism(
        &self,
        morphism: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<MorphismValidation, InstitutionError>;

    /// Discover morphisms not yet in the knowledge graph.
    /// Given resources in this institution's fiber, infer morphisms.
    /// Returns resources; the caller decides whether to commit them.
    fn discover_morphisms(
        &self,
        resources: &[Resource],
        ctx: &ExecutionContext,
    ) -> Result<Vec<Resource>, InstitutionError>;

    /// Decide a constraint predicate at check time
    /// (Phase 11c, life-science §16.3).
    ///
    /// Called when the kernel evaluates
    /// `Exp::NativeDecide(Constraint::Institution { iri, args }, v)`
    /// and the `iri` resolves to this institution. The `args` vector
    /// holds the user-supplied predicate arguments already marshalled
    /// from kernel `Val`s to resource `Value`s; institutions can
    /// pattern-match on scalar primitives, arrays, or embedded
    /// resources.
    ///
    /// Return `Holds` to reduce the constraint to `Refl(v)`, `Fails`
    /// to emit a failing neutral (blocking subsequent reduction —
    /// the type-checker surfaces this as a rejection), or
    /// `Undecidable` to leave the constraint as a passthrough
    /// neutral.
    ///
    /// Default implementation returns `Undecidable` — institutions
    /// that don't override `decide` opt out cleanly.
    fn decide(
        &self,
        constraint_iri: &Iri,
        args: &[crate::ontology::resource::Value],
        ctx: &ExecutionContext,
    ) -> Result<DecResult, InstitutionError> {
        let _ = (constraint_iri, args, ctx);
        Ok(DecResult::Undecidable)
    }

    /// Translate a resource across an institution boundary using a
    /// declared comorphism (Phase 11d, D10 §6).
    ///
    /// Called when the kernel evaluates
    /// `Exp::InstitutionInvoke { comorphism_iri, source }` or when
    /// Phase 14 reconciliation walks a merge witness. The
    /// `comorphism_iri` is looked up via
    /// [`InstitutionRegistry::institution_for_comorphism`]; the
    /// declaring institution's `translate` is called with the source
    /// resource and the current execution context.
    ///
    /// Default implementation returns an error — institutions that
    /// declare `comorphism_types` must override `translate` to
    /// produce translated resources.
    fn translate(
        &self,
        comorphism_iri: &Iri,
        source: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<Resource, InstitutionError> {
        let _ = (source, ctx);
        Err(InstitutionError::UnknownType(format!(
            "institution does not implement `translate` for comorphism `{comorphism_iri}`"
        )))
    }
}

/// Classification of an IRI by the institution-level capability it
/// dispatches to (Phase 11e, D10). Used by surface-language
/// compilers (ESL, EigenQL) to decide which kernel AST node to
/// emit for a function-call-shaped reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstitutionCapability {
    /// The IRI was declared as a decide predicate by some
    /// institution — emit `Exp::NativeDecide(Constraint::Institution {..}, _)`.
    DecidePredicate,
    /// The IRI was declared as a comorphism — emit
    /// `Exp::InstitutionInvoke { iri, source }`.
    Comorphism,
}

/// Result of an institution-registered constraint decision.
///
/// Three-valued so institutions can distinguish "I determined this
/// predicate is false" (`Fails`) from "I couldn't evaluate this at
/// check time, come back at runtime" (`Undecidable`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecResult {
    /// Predicate holds on the given args. The kernel reduces the
    /// surrounding `NativeDecide` to `Val::Refl(value)`.
    Holds,
    /// Predicate explicitly fails. The kernel emits a failing
    /// neutral — the type-checker's constraint path rejects this.
    Fails,
    /// Institution cannot determine the result (insufficient
    /// information, requires runtime). The `NativeDecide` stays as
    /// a passthrough neutral; later reduction may succeed.
    Undecidable,
}

/// An institution's fiber declaration, provided at registration time.
pub struct FiberDeclaration {
    /// The institution's IRI (e.g., "urn:eigenius:institutions:fea")
    pub institution_iri: Iri,

    /// Human-readable name
    pub name: String,

    /// Morphism classes this institution defines.
    /// Each is a Class resource with source_type, target_type, and properties.
    pub morphism_types: Vec<Resource>,

    /// FiberQuery subclasses this institution can answer.
    pub query_types: Vec<Resource>,

    /// Advisory structural properties of morphisms.
    /// The kernel stores these but does not enforce them.
    pub structural_properties: Vec<Resource>,

    /// Comorphism resources declared by this institution (Phase 11d,
    /// D10 §6). Each is a `Comorphism`-class resource carrying
    /// `source_institution`, `target_institution`, and
    /// `translation_procedure`. The declaring institution implements
    /// [`FiberReasoner::translate`] to perform the actual
    /// translation when invoked via `Exp::InstitutionInvoke` or the
    /// Phase 14 reconciliation walker.
    pub comorphism_types: Vec<Resource>,

    /// Decide-predicate IRIs this institution answers (Phase 11e).
    /// Listed so the ESL compiler (and future EigenQL compiler) can
    /// classify a function-call IRI at compile time as a
    /// `FiberReasoner::decide`-dispatched predicate vs. a component
    /// or a comorphism. The IRIs go into
    /// [`InstitutionRegistry::decide_dispatch`] for O(log n) lookup.
    pub decide_procedures: Vec<Iri>,
}

impl FiberDeclaration {
    /// Build a minimal declaration with no morphisms, queries,
    /// structural properties, or comorphisms. Convenience for tests
    /// and institutions that don't need the full declaration surface.
    pub fn minimal(institution_iri: Iri, name: impl Into<String>) -> Self {
        Self {
            institution_iri,
            name: name.into(),
            morphism_types: Vec::new(),
            query_types: Vec::new(),
            structural_properties: Vec::new(),
            comorphism_types: Vec::new(),
            decide_procedures: Vec::new(),
        }
    }
}

/// Information about a registered institution (for introspection).
#[derive(Debug, Clone)]
pub struct InstitutionInfo {
    pub iri: Iri,
    pub name: String,
    pub morphism_type_iris: Vec<Iri>,
    pub query_type_iris: Vec<Iri>,
    pub comorphism_iris: Vec<Iri>,
    pub decide_procedure_iris: Vec<Iri>,
}

/// Registry of institutions.
pub struct InstitutionRegistry {
    institutions: BTreeMap<Iri, Box<dyn FiberReasoner>>,
    /// Maps morphism class IRI → institution IRI for dispatch routing.
    morphism_dispatch: BTreeMap<Iri, Iri>,
    /// Maps query class IRI → institution IRI for dispatch routing.
    query_dispatch: BTreeMap<Iri, Iri>,
    /// Maps comorphism IRI → declaring institution IRI (Phase 11d).
    /// Looked up by `Exp::InstitutionInvoke` eval, Phase 14
    /// reconciliation, and user-facing introspection.
    comorphism_dispatch: BTreeMap<Iri, Iri>,
    /// Maps decide-predicate IRI → declaring institution IRI
    /// (Phase 11e). Used by ESL/EigenQL compilers to classify a
    /// function-call IRI as a `Constraint::Institution` predicate
    /// at compile time, and by the kernel to resolve which
    /// institution handles a given `decide` call.
    decide_dispatch: BTreeMap<Iri, Iri>,
    /// Info for each registered institution.
    info: BTreeMap<Iri, InstitutionInfo>,
}

impl InstitutionRegistry {
    pub fn new() -> Self {
        Self {
            institutions: BTreeMap::new(),
            morphism_dispatch: BTreeMap::new(),
            query_dispatch: BTreeMap::new(),
            comorphism_dispatch: BTreeMap::new(),
            decide_dispatch: BTreeMap::new(),
            info: BTreeMap::new(),
        }
    }

    /// Register an institution. Returns the ontology resources to commit
    /// (morphism types, query types, structural properties).
    pub fn register(&mut self, reasoner: Box<dyn FiberReasoner>) -> Result<Vec<Resource>, String> {
        self.register_inner(reasoner, /* publish */ true)
    }

    /// Register a reasoner whose declared classes are already in the layer
    /// chain (e.g., because they were persisted on the initial install).
    /// Builds dispatch tables, does not return any resources to commit.
    /// Used by the RESUME path in Phase 9a.
    pub fn register_rehydrated(&mut self, reasoner: Box<dyn FiberReasoner>) -> Result<(), String> {
        self.register_inner(reasoner, /* publish */ false)
            .map(|_| ())
    }

    fn register_inner(
        &mut self,
        reasoner: Box<dyn FiberReasoner>,
        publish: bool,
    ) -> Result<Vec<Resource>, String> {
        let decl = reasoner.fiber_declaration();
        let iri = decl.institution_iri.clone();

        // Build dispatch tables
        let mut morphism_iris = Vec::new();
        for mt in &decl.morphism_types {
            if let Some(mt_iri) = mt.id() {
                self.morphism_dispatch.insert(mt_iri.clone(), iri.clone());
                morphism_iris.push(mt_iri.clone());
            }
        }

        let mut query_iris = Vec::new();
        for qt in &decl.query_types {
            if let Some(qt_iri) = qt.id() {
                self.query_dispatch.insert(qt_iri.clone(), iri.clone());
                query_iris.push(qt_iri.clone());
            }
        }

        // Build comorphism dispatch: Comorphism resource IRI →
        // declaring institution IRI. Both the Comorphism's own IRI
        // and its `translation_procedure` IRI are indexed so lookups
        // work whether callers cite the comorphism or the procedure.
        let mut comorphism_iris = Vec::new();
        for cm in &decl.comorphism_types {
            let cm_iri = cm
                .id()
                .ok_or_else(|| format!("comorphism resource on institution `{iri}` missing @id"))?;
            self.comorphism_dispatch.insert(cm_iri.clone(), iri.clone());
            comorphism_iris.push(cm_iri.clone());
            // Also index by the translation_procedure IRI when set —
            // some declarations use a separate procedure identifier
            // from the resource IRI so multiple comorphisms can share
            // a procedure.
            let procedure_key = Iri::parse(crate::ontology::well_known::TRANSLATION_PROCEDURE)
                .expect("well-known IRI");
            if let Some(crate::ontology::resource::Value::String(proc_str)) = cm.get(&procedure_key)
            {
                if let Ok(proc_iri) = Iri::parse(proc_str) {
                    if proc_iri != *cm_iri {
                        self.comorphism_dispatch.insert(proc_iri, iri.clone());
                    }
                }
            }
        }

        // Build decide dispatch: procedure IRI → institution IRI.
        let mut decide_iris = Vec::new();
        for proc_iri in &decl.decide_procedures {
            self.decide_dispatch.insert(proc_iri.clone(), iri.clone());
            decide_iris.push(proc_iri.clone());
        }

        // Store info
        self.info.insert(
            iri.clone(),
            InstitutionInfo {
                iri: iri.clone(),
                name: decl.name.clone(),
                morphism_type_iris: morphism_iris,
                query_type_iris: query_iris,
                comorphism_iris,
                decide_procedure_iris: decide_iris,
            },
        );

        // Collect ontology resources to commit (only when publishing;
        // RESUME skips this because the resources are already persisted).
        let mut resources = Vec::new();
        if publish {
            resources.extend(decl.morphism_types);
            resources.extend(decl.query_types);
            resources.extend(decl.structural_properties);
            resources.extend(decl.comorphism_types);
        }

        self.institutions.insert(iri, reasoner);
        Ok(resources)
    }

    /// Get the fiber reasoner for a given institution IRI.
    pub fn get(&self, iri: &Iri) -> Option<&dyn FiberReasoner> {
        self.institutions.get(iri).map(|b| b.as_ref())
    }

    /// Find the institution that handles a given morphism class.
    pub fn institution_for_morphism(&self, morphism_class_iri: &Iri) -> Option<&dyn FiberReasoner> {
        let inst_iri = self.morphism_dispatch.get(morphism_class_iri)?;
        self.get(inst_iri)
    }

    /// Find the institution that handles a given query class.
    pub fn institution_for_query(&self, query_class_iri: &Iri) -> Option<&dyn FiberReasoner> {
        let inst_iri = self.query_dispatch.get(query_class_iri)?;
        self.get(inst_iri)
    }

    /// Find the institution declaring a given comorphism (Phase 11d).
    /// Accepts either the Comorphism resource's IRI or its
    /// `translation_procedure` IRI — both are indexed during
    /// registration.
    pub fn institution_for_comorphism(&self, comorphism_iri: &Iri) -> Option<&dyn FiberReasoner> {
        let inst_iri = self.comorphism_dispatch.get(comorphism_iri)?;
        self.get(inst_iri)
    }

    /// Return the institution IRI that declared the comorphism
    /// (non-reasoner view, useful for introspection or when the
    /// caller needs the institution identity rather than its
    /// implementation).
    pub fn comorphism_institution_iri(&self, comorphism_iri: &Iri) -> Option<&Iri> {
        self.comorphism_dispatch.get(comorphism_iri)
    }

    /// Find the institution that answers a given decide-predicate
    /// IRI (Phase 11e). `None` if no institution declared this
    /// procedure.
    pub fn institution_for_decide(&self, procedure_iri: &Iri) -> Option<&dyn FiberReasoner> {
        let inst_iri = self.decide_dispatch.get(procedure_iri)?;
        self.get(inst_iri)
    }

    /// Return the institution IRI that declared a decide procedure.
    pub fn decide_institution_iri(&self, procedure_iri: &Iri) -> Option<&Iri> {
        self.decide_dispatch.get(procedure_iri)
    }

    /// Classify an IRI by the capability kind it dispatches to
    /// (Phase 11e). Returns `None` if the registry doesn't know the
    /// IRI — the caller should fall through to non-institution
    /// lookups (component registry, class constructor, unbound
    /// variable).
    pub fn classify(&self, iri: &Iri) -> Option<InstitutionCapability> {
        if self.decide_dispatch.contains_key(iri) {
            Some(InstitutionCapability::DecidePredicate)
        } else if self.comorphism_dispatch.contains_key(iri) {
            Some(InstitutionCapability::Comorphism)
        } else {
            None
        }
    }

    /// List all registered institutions.
    pub fn list(&self) -> Vec<&InstitutionInfo> {
        self.info.values().collect()
    }

    /// Check if any institutions are registered.
    pub fn is_empty(&self) -> bool {
        self.institutions.is_empty()
    }
}

impl Default for InstitutionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal test institution: an ordering/refinement institution
    /// that validates transitivity of refinement morphisms.
    struct OrderingInstitution;

    impl FiberReasoner for OrderingInstitution {
        fn fiber_declaration(&self) -> FiberDeclaration {
            let refinement_class = {
                let mut r =
                    Resource::new(Iri::parse("urn:eigenius:test:institution:Refinement").unwrap());
                r.set(
                    Iri::parse("urn:eigenius:core:is_a").unwrap(),
                    crate::ontology::resource::Value::Array(vec![
                        crate::ontology::resource::Value::String(
                            "urn:eigenius:core:Class".to_string(),
                        ),
                    ]),
                );
                r.set(
                    Iri::parse("urn:eigenius:core:description").unwrap(),
                    crate::ontology::resource::Value::String(
                        "A refinement morphism between two results.".to_string(),
                    ),
                );
                r.set(
                    Iri::parse("urn:eigenius:core:short_name").unwrap(),
                    crate::ontology::resource::Value::String("Refinement".to_string()),
                );
                r.set(
                    Iri::parse("urn:eigenius:core:requires").unwrap(),
                    crate::ontology::resource::Value::Array(vec![
                        crate::ontology::resource::Value::String(
                            "urn:eigenius:test:institution:source".to_string(),
                        ),
                        crate::ontology::resource::Value::String(
                            "urn:eigenius:test:institution:target".to_string(),
                        ),
                        crate::ontology::resource::Value::String(
                            "urn:eigenius:test:institution:delta".to_string(),
                        ),
                    ]),
                );
                r
            };

            let query_class = {
                let mut r = Resource::new(
                    Iri::parse("urn:eigenius:test:institution:ConvergenceQuery").unwrap(),
                );
                r.set(
                    Iri::parse("urn:eigenius:core:is_a").unwrap(),
                    crate::ontology::resource::Value::Array(vec![
                        crate::ontology::resource::Value::String(
                            "urn:eigenius:core:Class".to_string(),
                        ),
                    ]),
                );
                r.set(
                    Iri::parse("urn:eigenius:core:description").unwrap(),
                    crate::ontology::resource::Value::String(
                        "Query whether a refinement chain has converged.".to_string(),
                    ),
                );
                r.set(
                    Iri::parse("urn:eigenius:core:short_name").unwrap(),
                    crate::ontology::resource::Value::String("ConvergenceQuery".to_string()),
                );
                r
            };

            FiberDeclaration {
                institution_iri: Iri::parse("urn:eigenius:test:institution:ordering").unwrap(),
                name: "Test Ordering Institution".to_string(),
                morphism_types: vec![refinement_class],
                query_types: vec![query_class],
                structural_properties: vec![],
                comorphism_types: vec![],
                decide_procedures: vec![],
            }
        }

        fn query(
            &self,
            query: &Resource,
            _ctx: &ExecutionContext,
        ) -> Result<Resource, InstitutionError> {
            // Check if the query is a ConvergenceQuery
            let is_a = query.is_a();
            if is_a
                .iter()
                .any(|i| i.as_str() == "urn:eigenius:test:institution:ConvergenceQuery")
            {
                let mut result = Resource::new_embedded();
                result.set(
                    Iri::parse("urn:eigenius:test:institution:converged").unwrap(),
                    crate::ontology::resource::Value::Boolean(true),
                );
                Ok(result)
            } else {
                Err(InstitutionError::UnknownType(format!(
                    "unknown query type: {:?}",
                    is_a
                )))
            }
        }

        fn validate_morphism(
            &self,
            morphism: &Resource,
            _ctx: &ExecutionContext,
        ) -> Result<MorphismValidation, InstitutionError> {
            // Check that delta is positive (refinement must improve)
            let delta_iri = Iri::parse("urn:eigenius:test:institution:delta").unwrap();
            match morphism.get(&delta_iri) {
                Some(crate::ontology::resource::Value::Float(d)) if *d > 0.0 => {
                    Ok(MorphismValidation::Valid)
                }
                Some(crate::ontology::resource::Value::Float(d)) => Ok(
                    MorphismValidation::Invalid(format!("delta must be positive, got {d}")),
                ),
                Some(crate::ontology::resource::Value::Integer(d)) if *d > 0 => {
                    Ok(MorphismValidation::Valid)
                }
                Some(crate::ontology::resource::Value::Integer(d)) => Ok(
                    MorphismValidation::Invalid(format!("delta must be positive, got {d}")),
                ),
                _ => Ok(MorphismValidation::Invalid(
                    "missing delta property".to_string(),
                )),
            }
        }

        fn discover_morphisms(
            &self,
            _resources: &[Resource],
            _ctx: &ExecutionContext,
        ) -> Result<Vec<Resource>, InstitutionError> {
            // Test institution doesn't discover morphisms
            Ok(vec![])
        }
    }

    #[test]
    fn register_institution() {
        let mut registry = InstitutionRegistry::new();
        let resources = registry.register(Box::new(OrderingInstitution)).unwrap();

        // Should produce 2 resources: Refinement class + ConvergenceQuery class
        assert_eq!(resources.len(), 2);

        // Registry should have 1 institution
        assert_eq!(registry.list().len(), 1);
        assert_eq!(registry.list()[0].name, "Test Ordering Institution");
    }

    #[test]
    fn dispatch_morphism_validation() {
        let mut registry = InstitutionRegistry::new();
        registry.register(Box::new(OrderingInstitution)).unwrap();

        // Look up institution by morphism class
        let refinement_iri = Iri::parse("urn:eigenius:test:institution:Refinement").unwrap();
        let reasoner = registry.institution_for_morphism(&refinement_iri);
        assert!(reasoner.is_some());

        // Validate a valid morphism
        let mut morphism = Resource::new_embedded();
        morphism.set(
            Iri::parse("urn:eigenius:test:institution:delta").unwrap(),
            crate::ontology::resource::Value::Float(0.05),
        );

        let ctx = crate::bootstrap::bootstrap().unwrap();
        let result = reasoner
            .unwrap()
            .validate_morphism(&morphism, &ctx)
            .unwrap();
        assert!(matches!(result, MorphismValidation::Valid));
    }

    #[test]
    fn reject_invalid_morphism() {
        let mut registry = InstitutionRegistry::new();
        registry.register(Box::new(OrderingInstitution)).unwrap();

        let refinement_iri = Iri::parse("urn:eigenius:test:institution:Refinement").unwrap();
        let reasoner = registry.institution_for_morphism(&refinement_iri).unwrap();

        // Negative delta — should be rejected
        let mut morphism = Resource::new_embedded();
        morphism.set(
            Iri::parse("urn:eigenius:test:institution:delta").unwrap(),
            crate::ontology::resource::Value::Float(-0.1),
        );

        let ctx = crate::bootstrap::bootstrap().unwrap();
        let result = reasoner.validate_morphism(&morphism, &ctx).unwrap();
        assert!(matches!(result, MorphismValidation::Invalid(_)));
    }

    #[test]
    fn dispatch_fiber_query() {
        let mut registry = InstitutionRegistry::new();
        registry.register(Box::new(OrderingInstitution)).unwrap();

        let query_iri = Iri::parse("urn:eigenius:test:institution:ConvergenceQuery").unwrap();
        let reasoner = registry.institution_for_query(&query_iri).unwrap();

        // Build a convergence query
        let mut query = Resource::new_embedded();
        query.set(
            Iri::parse("urn:eigenius:core:is_a").unwrap(),
            crate::ontology::resource::Value::Array(vec![
                crate::ontology::resource::Value::String(
                    "urn:eigenius:test:institution:ConvergenceQuery".to_string(),
                ),
            ]),
        );

        let ctx = crate::bootstrap::bootstrap().unwrap();
        let result = reasoner.query(&query, &ctx).unwrap();

        // Should return converged = true
        let converged = result
            .get(&Iri::parse("urn:eigenius:test:institution:converged").unwrap())
            .unwrap()
            .as_boolean();
        assert_eq!(converged, Some(true));
    }

    #[test]
    fn unknown_morphism_class_returns_none() {
        let registry = InstitutionRegistry::new();
        let iri = Iri::parse("urn:eigenius:nonexistent:Foo").unwrap();
        assert!(registry.institution_for_morphism(&iri).is_none());
    }

    // --- Phase 11d: comorphism declaration + translate ---

    /// Test institution that declares one comorphism and implements
    /// `translate` to produce a constant "translated" resource.
    struct ComorphismDeclarer {
        institution_iri: Iri,
        comorphism_iri: Iri,
    }

    impl FiberReasoner for ComorphismDeclarer {
        fn fiber_declaration(&self) -> FiberDeclaration {
            let mut cm = Resource::new(self.comorphism_iri.clone());
            cm.set(
                Iri::parse(crate::ontology::well_known::IS_A).unwrap(),
                crate::ontology::resource::Value::Array(vec![
                    crate::ontology::resource::Value::String(
                        crate::ontology::well_known::COMORPHISM.to_string(),
                    ),
                ]),
            );
            cm.set(
                Iri::parse(crate::ontology::well_known::SOURCE_INSTITUTION).unwrap(),
                crate::ontology::resource::Value::String(self.institution_iri.as_str().to_string()),
            );
            cm.set(
                Iri::parse(crate::ontology::well_known::TARGET_INSTITUTION).unwrap(),
                crate::ontology::resource::Value::String(
                    "urn:eigenius:test:target_institution".to_string(),
                ),
            );
            cm.set(
                Iri::parse(crate::ontology::well_known::TRANSLATION_PROCEDURE).unwrap(),
                crate::ontology::resource::Value::String(self.comorphism_iri.as_str().to_string()),
            );
            FiberDeclaration {
                institution_iri: self.institution_iri.clone(),
                name: "ComorphismDeclarer".to_string(),
                morphism_types: vec![],
                query_types: vec![],
                structural_properties: vec![],
                comorphism_types: vec![cm],
                decide_procedures: vec![],
            }
        }

        fn query(
            &self,
            _q: &Resource,
            _ctx: &ExecutionContext,
        ) -> Result<Resource, InstitutionError> {
            unreachable!()
        }
        fn validate_morphism(
            &self,
            _m: &Resource,
            _ctx: &ExecutionContext,
        ) -> Result<MorphismValidation, InstitutionError> {
            unreachable!()
        }
        fn discover_morphisms(
            &self,
            _rs: &[Resource],
            _ctx: &ExecutionContext,
        ) -> Result<Vec<Resource>, InstitutionError> {
            unreachable!()
        }
        fn translate(
            &self,
            comorphism_iri: &Iri,
            source: &Resource,
            _ctx: &ExecutionContext,
        ) -> Result<Resource, InstitutionError> {
            // Produce a "translated" resource whose id carries the
            // source's id + the comorphism suffix, so tests can
            // observe the call flowed through.
            let source_id_str = source
                .id()
                .map(|i| i.as_str().to_string())
                .unwrap_or_else(|| "anon".to_string());
            let out_iri = Iri::parse(&format!(
                "urn:eigenius:test:translated:{}:{}",
                comorphism_iri.as_str().replace(':', "_"),
                source_id_str.replace(':', "_")
            ))
            .unwrap();
            let out = Resource::new(out_iri);
            Ok(out)
        }
    }

    #[test]
    fn register_declares_comorphisms_in_dispatch_table() {
        let institution_iri = Iri::parse("urn:eigenius:test:declarer").unwrap();
        let comorphism_iri = Iri::parse("urn:eigenius:test:my_comorphism").unwrap();
        let decl = ComorphismDeclarer {
            institution_iri: institution_iri.clone(),
            comorphism_iri: comorphism_iri.clone(),
        };

        let mut reg = InstitutionRegistry::new();
        let resources = reg.register(Box::new(decl)).unwrap();
        // The Comorphism resource is returned for committing to the layer.
        assert!(
            resources
                .iter()
                .any(|r| r.id().map(|i| i == &comorphism_iri).unwrap_or(false)),
            "comorphism resource should be published among registration outputs"
        );

        // Dispatch table resolves comorphism IRI → institution.
        assert_eq!(
            reg.comorphism_institution_iri(&comorphism_iri),
            Some(&institution_iri)
        );
        // institution_for_comorphism returns the reasoner.
        assert!(reg.institution_for_comorphism(&comorphism_iri).is_some());
    }

    #[test]
    fn translate_dispatches_to_declaring_institution() {
        let institution_iri = Iri::parse("urn:eigenius:test:declarer2").unwrap();
        let comorphism_iri = Iri::parse("urn:eigenius:test:comorphism2").unwrap();
        let decl = ComorphismDeclarer {
            institution_iri: institution_iri.clone(),
            comorphism_iri: comorphism_iri.clone(),
        };

        let mut reg = InstitutionRegistry::new();
        reg.register(Box::new(decl)).unwrap();

        let reasoner = reg
            .institution_for_comorphism(&comorphism_iri)
            .expect("registered");
        let src = Resource::new(Iri::parse("urn:eigenius:test:src_resource").unwrap());
        let layer =
            std::sync::Arc::new(crate::layer::LayerBuilder::new("test_layer", None).build());
        let exec =
            ExecutionContext::new(layer, "test_exec", crate::context::ExecutionMode::ReadOnly);
        let result = reasoner
            .translate(&comorphism_iri, &src, &exec)
            .expect("translate");
        let id_str = result.id().unwrap().as_str();
        assert!(
            id_str.contains("comorphism2") && id_str.contains("src_resource"),
            "translated resource id should carry the comorphism+source suffix, got: {id_str}"
        );
    }

    #[test]
    fn default_translate_returns_unknown_type_error() {
        use crate::institution::DecResult;
        struct NoTranslate;
        impl FiberReasoner for NoTranslate {
            fn fiber_declaration(&self) -> FiberDeclaration {
                FiberDeclaration::minimal(
                    Iri::parse("urn:eigenius:test:no_translate").unwrap(),
                    "NoTranslate",
                )
            }
            fn query(
                &self,
                _q: &Resource,
                _ctx: &ExecutionContext,
            ) -> Result<Resource, InstitutionError> {
                unreachable!()
            }
            fn validate_morphism(
                &self,
                _m: &Resource,
                _ctx: &ExecutionContext,
            ) -> Result<MorphismValidation, InstitutionError> {
                unreachable!()
            }
            fn discover_morphisms(
                &self,
                _rs: &[Resource],
                _ctx: &ExecutionContext,
            ) -> Result<Vec<Resource>, InstitutionError> {
                unreachable!()
            }
        }

        let inst = NoTranslate;
        let iri = Iri::parse("urn:eigenius:test:nonexistent_comorphism").unwrap();
        let src = Resource::new(Iri::parse("urn:eigenius:test:src").unwrap());
        let layer =
            std::sync::Arc::new(crate::layer::LayerBuilder::new("test_layer", None).build());
        let exec =
            ExecutionContext::new(layer, "test_exec", crate::context::ExecutionMode::ReadOnly);
        let err = inst.translate(&iri, &src, &exec).unwrap_err();
        assert!(
            matches!(err, InstitutionError::UnknownType(_)),
            "default translate should return UnknownType, got {err:?}"
        );
        // DecResult is unrelated — reference to keep the import used.
        let _ = DecResult::Holds;
    }
}
