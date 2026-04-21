//! Grothendieck Institution Protocol.
//!
//! Domain-specific reasoning systems (institutions) register with the kernel,
//! declare their fiber structure (morphism types, query types, structural
//! properties), and participate in validation and reasoning.
//!
//! See design document D10 for the full specification.

pub mod comorphism;
pub mod error;

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
}

/// Information about a registered institution (for introspection).
#[derive(Debug, Clone)]
pub struct InstitutionInfo {
    pub iri: Iri,
    pub name: String,
    pub morphism_type_iris: Vec<Iri>,
    pub query_type_iris: Vec<Iri>,
}

/// Registry of institutions.
pub struct InstitutionRegistry {
    institutions: BTreeMap<Iri, Box<dyn FiberReasoner>>,
    /// Maps morphism class IRI → institution IRI for dispatch routing.
    morphism_dispatch: BTreeMap<Iri, Iri>,
    /// Maps query class IRI → institution IRI for dispatch routing.
    query_dispatch: BTreeMap<Iri, Iri>,
    /// Info for each registered institution.
    info: BTreeMap<Iri, InstitutionInfo>,
}

impl InstitutionRegistry {
    pub fn new() -> Self {
        Self {
            institutions: BTreeMap::new(),
            morphism_dispatch: BTreeMap::new(),
            query_dispatch: BTreeMap::new(),
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

        // Store info
        self.info.insert(
            iri.clone(),
            InstitutionInfo {
                iri: iri.clone(),
                name: decl.name.clone(),
                morphism_type_iris: morphism_iris,
                query_type_iris: query_iris,
            },
        );

        // Collect ontology resources to commit (only when publishing;
        // RESUME skips this because the resources are already persisted).
        let mut resources = Vec::new();
        if publish {
            resources.extend(decl.morphism_types);
            resources.extend(decl.query_types);
            resources.extend(decl.structural_properties);
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
}
