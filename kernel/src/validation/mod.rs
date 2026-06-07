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

//! Validation engine for Eigon resources.
//!
//! Validates resources in a layer against definitions reachable through
//! the parent chain. Implements all validation rules from D1 §5.4.
//!
//! The per-rule check functions (Rules 3–10, 16, 17, and the
//! class-definition / `is_a` integrity checks) live under
//! `validation::rules` as `impl Validator` block extensions. The
//! `Validator` struct + driver loop, the shared chain-walking helpers
//! (`is_instance_of_any`, `is_subclass_of`, `get_data_type_str`), the
//! universe-stratification / merge-comorphism / lambda well-typedness
//! / comorphism well-formedness rules, and the public surface
//! (`ValidationError`, `ValidationRule`) stay here.

pub mod retroactive;
mod rules;
pub mod working_set;

pub use retroactive::retroactive_validate;
pub use working_set::{
    CommitWorkingSet, CommitWorkingSetPool, DrainedViolations, InMemoryIriQueue, InMemoryIriSet,
    InMemoryViolationCollector, IriQueue, IriSet, PooledWorkingSet, ViolationCollector,
    WorkingSetExhausted, DEFAULT_WORKING_SET_CAP,
};

use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

/// A validation error describing a constraint violation.
#[derive(Debug, Clone)]
pub struct ValidationError {
    pub resource_id: Option<Iri>,
    pub property: Option<Iri>,
    pub rule: ValidationRule,
    pub message: String,
}

/// The type of validation rule that was violated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationRule {
    MissingRequired,
    TypeMismatch,
    FormatViolation,
    PatternViolation,
    RangeViolation,
    LengthViolation,
    ClassTypeMismatch,
    AllowedValueViolation,
    DomainViolation,
    ConditionalRequirement,
    InstitutionValidation,
    UniverseStratificationViolation,
    /// A class or property declaration references an IRI that doesn't
    /// resolve to a resource of the expected kind in the layer chain.
    /// Examples: `is_a` referencing a missing class, `requires`
    /// referencing a missing `core:Property`, `class_types` referencing
    /// a missing `core:Class`, `data_type` referencing a missing
    /// `core:DataType`, `subclass_of` referencing a missing
    /// `core:Class`. See eigenius#26.
    UnresolvedClassReference,
    /// An inductive value carries a `ctor` not declared on its
    /// referenced `InductiveType`, an arity mismatch against the
    /// ctor's `arg_types`, or an arg whose value doesn't match its
    /// declared `type_name`. D32 §3.5.
    InductiveValueMismatch,
    /// A FormulaTerm `App` spine doesn't match the leftmost operator's
    /// declared arity. D32 §5.4 / Phase 19d.0.d.
    OperatorArityMismatch,
    /// A `MergeComorphism` resource is structurally inconsistent
    /// with its declared `merge_target_class` (D37 §5.2). Typical
    /// causes: the referenced `merge_transformation` isn't a
    /// Lambda, the Lambda chain doesn't have the 3-binder
    /// `(a, b, opt)` shape, or a binder's `parameter_type` slot
    /// disagrees with `target_class` / `Option<target_class>`.
    /// Commit-time enforcement of the witness signature contract;
    /// complements the resolver's apply-time class check.
    MergeComorphismShapeViolation,
    /// A standalone Lambda resource's body fails to type-check
    /// against its declared `program:type` Pi-term (D37 §5.1).
    /// Commit-time NbE-backed verification: the body evaluated
    /// against the typing context implied by the declared Pi-type
    /// produces a type error rather than the expected codomain.
    /// Catches witness bodies that reference unbound variables,
    /// return the wrong type, or apply operators with mismatched
    /// arities before the resource lands on the chain.
    LambdaTypeMismatch,
    /// A `reflection:canonical_proposition` property value (D49 §6)
    /// fails to decode through the D47 type-fragment codec. The
    /// resource is rejected at commit so malformed propositions never
    /// reach the per-Layer witness index; without this gate a malformed
    /// canonical proposition would silently absent the corresponding
    /// `ChainWitness` rather than surfacing a diagnostic.
    CanonicalPropositionMalformed,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(id) = &self.resource_id {
            write!(f, "[{id}] ")?;
        }
        if let Some(prop) = &self.property {
            write!(f, "{prop}: ")?;
        }
        write!(f, "{:?} — {}", self.rule, self.message)
    }
}

/// Validates resources in a layer against definitions reachable through
/// the parent chain (and within the layer itself).
///
/// Holds an `Arc<Layer>` rather than a borrow so the NbE-backed
/// checks (Rule 19: standalone Lambda well-typedness) can construct a
/// `CheckCtx` that owns the layer reference. Callers passing an
/// already-Arc'd layer should `Arc::clone` it; callers with a fresh
/// `Layer` should wrap in `Arc::new`.
pub struct Validator {
    pub(crate) layer: Arc<Layer>,
}

impl Validator {
    pub fn new(layer: Arc<Layer>) -> Self {
        Self { layer }
    }

    /// Validate all resources in this layer.
    pub fn validate(&self) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        for arc_resource in self.layer.iter_resources().map(|(_, r)| r) {
            let resource: &Resource = &arc_resource;
            errors.extend(self.validate_resource(resource));
        }
        errors
    }

    /// Validate a single resource.
    pub fn validate_resource(&self, resource: &Resource) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        let res_id = resource.id().cloned();

        // Collect all classes this resource is an instance of
        let class_iris = resource.is_a();
        let class_refs: Vec<&Iri> = class_iris.iter().collect();

        // Rule 0: every resource must declare at least one `is_a`
        // class. (See `rules::is_a` for the full story.)
        errors.extend(self.check_missing_is_a(resource, &res_id));

        // Collect effective requires/recommends from all classes + ancestors
        let (required_props, _recommended_props) = self.collect_effective_properties(&class_refs);

        // Also collect conditional requirements
        let (conditional_required, _conditional_recommended) =
            self.evaluate_conditional_requires(&class_refs, resource);

        let all_required: BTreeSet<Iri> = required_props
            .into_iter()
            .chain(conditional_required)
            .collect();

        // Rule 1+2: Required properties (including inherited)
        for req_iri in &all_required {
            if !resource.has(req_iri) {
                errors.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: Some(req_iri.clone()),
                    rule: ValidationRule::MissingRequired,
                    message: format!("required property '{req_iri}' is missing"),
                });
            }
        }

        // Validate each property on the resource
        for (prop_iri, value) in resource.properties() {
            // Look up the property definition
            let prop_def = self.layer.resolve(prop_iri);

            if let Some(prop_def_arc) = prop_def {
                let prop_def: &Resource = &prop_def_arc;
                // Rule 10: Domain checking
                errors.extend(self.check_domain(prop_def, resource, prop_iri, &res_id));

                // Rule 3: Type checking
                errors.extend(self.check_type(prop_def, value, prop_iri, &res_id));

                // Rule 4: Format checking
                errors.extend(self.check_format(prop_def, value, prop_iri, &res_id));

                // Rule 5: Pattern checking
                errors.extend(self.check_pattern(prop_def, value, prop_iri, &res_id));

                // Rule 6: Range checking
                errors.extend(self.check_range(prop_def, value, prop_iri, &res_id));

                // Rule 7: Length checking
                errors.extend(self.check_length(prop_def, value, prop_iri, &res_id));

                // Rule 8: Class type checking
                errors.extend(self.check_class_types(prop_def, value, prop_iri, &res_id));

                // Rule 9: Allowed values checking
                errors.extend(self.check_allows_only(prop_def, value, prop_iri, &res_id));

                // Rule 16: Inductive value type-checking (D32 §3.5).
                errors.extend(self.check_inductive_value(prop_def, value, prop_iri, &res_id));

                // Rule 17: FormulaTerm App-spine rank check against
                // the leftmost operator's declared arity (D32 §5.4 /
                // Phase 19d.0.d). No-op for non-FormulaTerm values.
                errors.extend(self.check_formula_term_arity(prop_def, value, prop_iri, &res_id));
            }
            // Rule 12 (open world): unknown properties are allowed
        }

        // Rule 13: Universe stratification (D6b §7, Phase 10b)
        errors.extend(self.check_universe_stratification(resource, &res_id));

        // Rule 20: `reflection:canonical_proposition` decoder check
        // (D49 §6). When present on a Declared / Observed / Derived
        // resource, the value must decode cleanly through the D47
        // type-fragment codec. Malformed propositions are rejected at
        // commit so they never silently absent the corresponding
        // `ChainWitness` from the per-Layer witness index.
        errors.extend(self.check_canonical_proposition(resource, &res_id));

        // Rule 14: Class-definition reference integrity (eigenius#26).
        errors.extend(self.check_class_definition_references(resource, &res_id));

        // Rule 15: Comorphism well-formedness (D14 §4.5 / §5).
        // For Comorphism resources, verify that `export_format` and
        // `import_format` references resolve to ExportFormat /
        // ImportFormat resources, and that `transformation` resolves
        // to *some* resource in the chain. The full EigenTT
        // signature-equality check between transformation Component
        // and the export/import payload types is deferred until the
        // institution dispatch evaluator lands (M5 of the D14 plan).
        errors.extend(self.check_comorphism_well_formedness(resource, &res_id));

        // Rule 18: MergeComorphism shape (D37 §5.2). The witness
        // contract is `(A, A, Option<A>) -> A` where A is
        // `merge_target_class`; check that the referenced
        // `merge_transformation` is a Lambda chain matching that
        // shape. Catches mismatched witnesses at commit time rather
        // than at apply time.
        errors.extend(self.check_merge_comorphism_shape(resource, &res_id));

        // Rule 19: Standalone Lambda well-typedness (D37 §5.1).
        // When a Lambda resource is committed at a top-level IRI
        // and carries a declared `program:type`, NbE-check its
        // body against the declared Pi-term. The cheapest fail
        // mode is "binder count doesn't match Pi-arity", which the
        // checker catches as it walks the lambda chain alongside
        // the Pi. Body-internal errors (unbound var, wrong return
        // type, operator arity mismatch) surface as the
        // `nbe::check` diagnostic.
        errors.extend(self.check_standalone_lambda_well_typedness(resource, &res_id));

        errors
    }

    // ── Shared chain-walking helpers used across multiple rule files ──

    /// Check if a resource is an instance of any of the given classes,
    /// considering subclass relationships.
    pub(crate) fn is_instance_of_any(&self, resource: &Resource, classes: &[&Iri]) -> bool {
        let resource_classes = resource.is_a();
        for res_class in &resource_classes {
            for allowed_class in classes {
                if *res_class == **allowed_class || self.is_subclass_of(res_class, allowed_class) {
                    return true;
                }
            }
        }
        false
    }

    /// Check if `sub` is a subclass of `super_class` by walking subclass_of.
    fn is_subclass_of(&self, sub: &Iri, super_class: &Iri) -> bool {
        self.is_subclass_of_inner(sub, super_class, &mut BTreeSet::new())
    }

    fn is_subclass_of_inner(
        &self,
        sub: &Iri,
        super_class: &Iri,
        visited: &mut BTreeSet<Iri>,
    ) -> bool {
        if !visited.insert(sub.clone()) {
            return false; // Cycle
        }

        if let Some(class_def) = self.layer.resolve(sub) {
            if let Some(parents) = class_def.get(&iri(wk::PARENT_CLASSES)) {
                for parent in parents.as_iri_array() {
                    if parent == *super_class {
                        return true;
                    }
                    if self.is_subclass_of_inner(&parent, super_class, visited) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Extract the data_type IRI string from a property definition.
    pub(crate) fn get_data_type_str(&self, prop_def: &Resource) -> Option<String> {
        // `data_type` is a `data_type: resource` property, so after
        // `LayerBuilder::canonicalise_resource_refs` runs the value is
        // a `Value::ResourceRef`. Accept the (legacy) `Value::String`
        // shape too for resources read off the wire before
        // canonicalisation (RPC payloads, FIBER intermediates).
        prop_def
            .get(&iri(wk::DATA_TYPE_PROP))
            .and_then(|v| v.as_iri())
            .map(|i| i.as_str().to_string())
    }

    /// Rule 15: Comorphism well-formedness (D14 §4.5 / §5).
    ///
    /// For a Comorphism resource, the kernel checks that:
    ///
    /// - `export_format` resolves to a resource of class `ExportFormat`,
    /// - `import_format` resolves to a resource of class `ImportFormat`,
    /// - `transformation` resolves to *some* resource in the chain
    ///   (the full EigenTT signature-equality check between the
    ///   referenced Component and the export/import payload types
    ///   lands when the institution dispatch evaluator does — M5 of
    ///   the D14 plan).
    ///
    /// Existing rules (`check_class_types`) already flag references
    /// that resolve to *wrong-class* resources, but they deliberately
    /// skip *missing* references on instance properties (they may be
    /// forward references to be filled later in the same batch — see
    /// the comment in `check_class_types` line ≈628). For a Comorphism
    /// however, the export and import formats must already exist when
    /// the comorphism enters the chain — the kernel needs them to
    /// type-check the transformation, and Phase 9a rehydration won't
    /// recover anything we don't catch here. This rule closes that
    /// gap.
    fn check_comorphism_well_formedness(
        &self,
        resource: &Resource,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        let comorphism_class = iri(wk::COMORPHISM);
        if !resource.is_instance_of(&comorphism_class) {
            return errors;
        }

        // For both typed format references the kernel checks that
        // the referenced IRI both *resolves* and resolves to a resource
        // of the expected class. The existing `check_class_types` rule
        // handles wrong-class but skips missing references; this rule
        // tightens that for Comorphism specifically.
        for typed_ref in [
            ComorphismFormatRef {
                field_iri: wk::EXPORT_FORMAT,
                field_label: "Comorphism.export_format",
                expected_class_iri: wk::EXPORT_FORMAT_CLASS,
                expected_label: "ExportFormat",
            },
            ComorphismFormatRef {
                field_iri: wk::IMPORT_FORMAT,
                field_label: "Comorphism.import_format",
                expected_class_iri: wk::IMPORT_FORMAT_CLASS,
                expected_label: "ImportFormat",
            },
        ] {
            self.check_comorphism_typed_ref(resource, typed_ref, res_id, &mut errors);
        }

        // `transformation` is a generic IRI string; we only check that
        // the referenced resource exists in the chain. The full
        // Component-signature check waits for M5.
        if let Some(value) = resource.get(&iri(wk::TRANSFORMATION)) {
            if let Some(target) = value_as_iri(value) {
                if self.layer.resolve(&target).is_none() {
                    errors.push(ValidationError {
                        resource_id: res_id.clone(),
                        property: Some(iri(wk::TRANSFORMATION)),
                        rule: ValidationRule::UnresolvedClassReference,
                        message: format!(
                            "Comorphism.transformation: '{target}' does not resolve to any \
                             resource in the layer chain"
                        ),
                    });
                }
            }
        }

        errors
    }

    fn check_comorphism_typed_ref(
        &self,
        resource: &Resource,
        typed_ref: ComorphismFormatRef<'_>,
        res_id: &Option<Iri>,
        errors: &mut Vec<ValidationError>,
    ) {
        let Some(value) = resource.get(&iri(typed_ref.field_iri)) else {
            // Required-property absence is reported by Rule 1
            // (MissingRequired); don't double-flag here.
            return;
        };
        let Some(target) = value_as_iri(value) else {
            return; // Type errors caught elsewhere.
        };
        let expected = iri(typed_ref.expected_class_iri);
        match self.layer.resolve(&target) {
            Some(target_resource) if target_resource.is_instance_of(&expected) => {}
            Some(_) => errors.push(ValidationError {
                resource_id: res_id.clone(),
                property: Some(iri(typed_ref.field_iri)),
                rule: ValidationRule::UnresolvedClassReference,
                message: format!(
                    "{}: '{target}' resolves to a resource that is not an instance of {}",
                    typed_ref.field_label, typed_ref.expected_label
                ),
            }),
            None => errors.push(ValidationError {
                resource_id: res_id.clone(),
                property: Some(iri(typed_ref.field_iri)),
                rule: ValidationRule::UnresolvedClassReference,
                message: format!(
                    "{}: '{target}' does not resolve to any resource in the layer chain",
                    typed_ref.field_label,
                ),
            }),
        }
    }

    /// Rule 18: MergeComorphism shape (D37 §5.2).
    ///
    /// The witness contract is `(A, A, Option<A>) -> A` where A is
    /// the comorphism's declared `merge_target_class`. This check
    /// verifies — at commit time, before any merge attempt — that
    /// the referenced `merge_transformation`:
    ///
    /// 1. Resolves to a resource in the chain.
    /// 2. Is a `urn:eigenius:program:Lambda` resource.
    /// 3. Has exactly three nested-Lambda binders (the (a, b, opt)
    ///    triple).
    /// 4. Each binder's `parameter_type` (when populated) matches
    ///    the witness contract:
    ///    - binders 1 and 2: ResourceRef to `target_class`
    ///    - binder 3: embedded `InductiveArgType` with
    ///      `type_name = Option` and `type_args = [target_class]`
    ///
    /// The check is purely structural — full NbE-based body
    /// type-checking against the Pi-term is a follow-on once
    /// the elaborator surface is wired up. Even the cheap shape
    /// check catches the most-impactful failure modes (wrong
    /// target class, wrong arity, wrong parameter types) before
    /// the resource hits the merge path.
    fn check_merge_comorphism_shape(
        &self,
        resource: &Resource,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        let merge_class = iri(wk::MERGE_COMORPHISM);
        if !resource.is_instance_of(&merge_class) {
            return errors;
        }

        // `merge_target_class` is required by the core ontology; if
        // absent, Rule 1 already flagged it. Only proceed if present.
        let Some(target_class_value) = resource.get(&iri(wk::MERGE_TARGET_CLASS)) else {
            return errors;
        };
        let Some(target_class_iri_str) = target_class_value.as_iri_str() else {
            return errors;
        };
        let Ok(target_class) = Iri::parse(target_class_iri_str) else {
            return errors;
        };

        // `merge_transformation` is required; if absent, Rule 1
        // already flagged it. Only proceed if present.
        let Some(transformation_value) = resource.get(&iri(wk::MERGE_TRANSFORMATION)) else {
            return errors;
        };
        let Some(transformation_iri_str) = transformation_value.as_iri_str() else {
            return errors;
        };
        let Ok(transformation_iri) = Iri::parse(transformation_iri_str) else {
            return errors;
        };

        // Resolve the transformation in the chain.
        let Some(transformation_resource_arc) = self.layer.resolve(&transformation_iri) else {
            errors.push(ValidationError {
                resource_id: res_id.clone(),
                property: Some(iri(wk::MERGE_TRANSFORMATION)),
                rule: ValidationRule::MergeComorphismShapeViolation,
                message: format!(
                    "MergeComorphism.merge_transformation: '{transformation_iri}' does not \
                     resolve to any resource in the layer chain"
                ),
            });
            return errors;
        };
        let transformation_resource: &Resource = &transformation_resource_arc;

        // Must be a Lambda.
        let lambda_class = iri("urn:eigenius:program:Lambda");
        if !transformation_resource.is_instance_of(&lambda_class) {
            errors.push(ValidationError {
                resource_id: res_id.clone(),
                property: Some(iri(wk::MERGE_TRANSFORMATION)),
                rule: ValidationRule::MergeComorphismShapeViolation,
                message: format!(
                    "MergeComorphism.merge_transformation: '{transformation_iri}' resolves to a \
                     resource that is not a urn:eigenius:program:Lambda"
                ),
            });
            return errors;
        }

        // Walk the nested-Lambda chain. Expected shape: outer
        // Lambda(a, _, Lambda(b, _, Lambda(opt, _, body))). Collect
        // each binder's `parameter_type` (when populated) to check
        // against the witness contract.
        let parameter_type_iri = iri("urn:eigenius:program:parameter_type");
        let body_iri = iri("urn:eigenius:program:body");

        let mut current: &Resource = transformation_resource;
        // Hold a local arc to keep ownership alive across iterations
        // when we descend into an embedded body.
        let mut _hold_embedded: Option<Box<Resource>> = None;
        let mut binder_param_types: Vec<Option<Value>> = Vec::new();

        for depth in 0..3usize {
            // Each Lambda layer must carry `parameter` + `body`.
            if !current.is_instance_of(&lambda_class) {
                errors.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: Some(iri(wk::MERGE_TRANSFORMATION)),
                    rule: ValidationRule::MergeComorphismShapeViolation,
                    message: format!(
                        "MergeComorphism.merge_transformation: '{transformation_iri}' lambda \
                         chain truncates at depth {depth}; the witness signature \
                         (a, b, opt) requires three nested Lambda binders"
                    ),
                });
                return errors;
            }
            binder_param_types.push(current.get(&parameter_type_iri).cloned());
            let Some(body_value) = current.get(&body_iri) else {
                errors.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: Some(iri(wk::MERGE_TRANSFORMATION)),
                    rule: ValidationRule::MergeComorphismShapeViolation,
                    message: format!(
                        "MergeComorphism.merge_transformation: '{transformation_iri}' Lambda at \
                         depth {depth} is missing its `body` property"
                    ),
                });
                return errors;
            };
            // The inner body is either an embedded Lambda (when we
            // haven't yet hit the innermost) or a different
            // expression shape (Var, Construct, etc.) at the
            // innermost. After three layers we exit the loop.
            if depth == 2 {
                // Innermost — body is the actual witness expression.
                // Don't descend further.
                break;
            }
            match body_value {
                Value::Embedded(embedded) => {
                    _hold_embedded = Some(embedded.clone());
                    current = _hold_embedded.as_ref().unwrap().as_ref();
                }
                _ => {
                    errors.push(ValidationError {
                        resource_id: res_id.clone(),
                        property: Some(iri(wk::MERGE_TRANSFORMATION)),
                        rule: ValidationRule::MergeComorphismShapeViolation,
                        message: format!(
                            "MergeComorphism.merge_transformation: '{transformation_iri}' Lambda \
                             at depth {depth} body is not an embedded resource — the witness's \
                             nested-Lambda chain is malformed"
                        ),
                    });
                    return errors;
                }
            }
        }

        // Verify each binder's parameter_type when populated. We
        // suppress check on a missing slot — the parameter_type is
        // a recommends, not required — to keep this validator
        // additive over existing untyped lambdas. Future iteration
        // can require populated slots once the typed surface is the
        // primary authoring path.
        let class_iri_str = target_class.as_str();
        for (idx, pt) in binder_param_types.iter().enumerate() {
            let Some(value) = pt else { continue };
            let (label, ok) = match idx {
                0 | 1 => (
                    "the class A (merge_target_class)",
                    value.as_iri_str() == Some(class_iri_str),
                ),
                2 => (
                    "Option<A> (Option of merge_target_class)",
                    is_option_of_class(value, class_iri_str),
                ),
                _ => unreachable!(),
            };
            if !ok {
                errors.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: Some(iri(wk::MERGE_TRANSFORMATION)),
                    rule: ValidationRule::MergeComorphismShapeViolation,
                    message: format!(
                        "MergeComorphism.merge_transformation: '{transformation_iri}' binder #{} \
                         parameter_type does not match {label}",
                        idx + 1
                    ),
                });
            }
        }

        errors
    }

    /// Rule 19: Standalone Lambda well-typedness (D37 §5.1).
    ///
    /// When a resource is a `urn:eigenius:program:Lambda` committed
    /// at a top-level IRI (i.e., it has `@id`) and carries a
    /// declared `urn:eigenius:program:type`, NbE-check the body
    /// against the declared Pi-term using `nbe::check::check`. This
    /// is the deferred work from PR 2's initial cut — Rule 18 caught
    /// structural mismatches; this catches semantic ones (unbound
    /// vars, wrong return types, operator arity mismatches inside
    /// the body).
    ///
    /// Skip when:
    /// - The resource doesn't carry a top-level `@id` (embedded
    ///   lambdas inside `program` bodies infer their type from
    ///   context).
    /// - `program:type` is absent (recommends, not requires — keeps
    ///   the check additive over untyped lambdas).
    /// - Any preparatory step fails (decoder error, parse error,
    ///   eval error) — these get surfaced as a single typed error
    ///   rather than letting NbE crash.
    fn check_standalone_lambda_well_typedness(
        &self,
        resource: &Resource,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        // Only standalone (top-level) lambdas — embedded lambdas
        // have no @id and rely on surrounding-Pi inference.
        if res_id.is_none() {
            return errors;
        }
        let lambda_class = iri("urn:eigenius:program:Lambda");
        if !resource.is_instance_of(&lambda_class) {
            return errors;
        }
        let Some(type_value) = resource.get(&iri(wk::PROGRAM_TYPE)) else {
            return errors;
        };

        // Decode the declared Pi-term.
        let type_exp = match crate::program::expr::decode_program_type(type_value, &self.layer) {
            Ok(e) => e,
            Err(reason) => {
                errors.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: Some(iri(wk::PROGRAM_TYPE)),
                    rule: ValidationRule::LambdaTypeMismatch,
                    message: format!(
                        "standalone Lambda's `program:type` could not be decoded: {reason}"
                    ),
                });
                return errors;
            }
        };

        // Parse the lambda body.
        let lam_exp = match crate::program::expr::parse_expression(resource, &self.layer) {
            Ok(e) => e,
            Err(reason) => {
                errors.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: None,
                    rule: ValidationRule::LambdaTypeMismatch,
                    message: format!("standalone Lambda body did not parse as EigenTT: {reason}"),
                });
                return errors;
            }
        };

        // Evaluate the declared type to a Val so `check` can use it.
        let type_val = match crate::nbe::eval::eval(&type_exp, &crate::nbe::env::Rho::Nil) {
            Ok(v) => v,
            Err(eval_err) => {
                errors.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: Some(iri(wk::PROGRAM_TYPE)),
                    rule: ValidationRule::LambdaTypeMismatch,
                    message: format!(
                        "standalone Lambda's `program:type` failed to evaluate: {eval_err}"
                    ),
                });
                return errors;
            }
        };

        // Run the NbE check. The check arm for `(Lam, Pi)` walks
        // both chains and recurses on the body against the codomain.
        let mut ctx = crate::nbe::check::CheckCtx::with_layer(
            crate::nbe::env::Rho::Nil,
            Vec::new(),
            Arc::clone(&self.layer),
        );
        if let Err(reason) = crate::nbe::check::check(&mut ctx, &lam_exp, &type_val) {
            errors.push(ValidationError {
                resource_id: res_id.clone(),
                property: None,
                rule: ValidationRule::LambdaTypeMismatch,
                message: format!(
                    "standalone Lambda body fails to type-check against declared `program:type`: \
                     {reason}"
                ),
            });
        }
        errors
    }

    /// Rule 13: Universe stratification (D6b §7).
    ///
    /// A resource at universe level N may only reference resources at
    /// level N-1 or below. Domain resources (no `universe_level`) are
    /// always referenceable. This prevents circular meta-reasoning.
    fn check_universe_stratification(
        &self,
        resource: &Resource,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let this_level = match resource.get(&iri(wk::UNIVERSE_LEVEL)) {
            Some(Value::Integer(n)) => *n,
            _ => return vec![], // No universe_level → domain resource, skip
        };

        let mut errors = Vec::new();

        for (prop_iri, value) in resource.properties() {
            // Check the property definition's data_type
            let prop_def = match self.layer.resolve(prop_iri) {
                Some(d) => d,
                None => continue,
            };
            let dt = match self.get_data_type_str(&prop_def) {
                Some(s) => s,
                None => continue,
            };

            // Only check resource and resource_array properties
            let is_ref = dt == wk::RESOURCE || dt == wk::RESOURCE_ARRAY;
            if !is_ref {
                continue;
            }

            // Collect IRI references from the value, accepting both
            // canonical `ResourceRef` and pre-canonical `String`
            // shapes via `Value::as_iri` / `as_iri_array`.
            let ref_iris: Vec<Iri> = match value {
                Value::Array(_) => value.as_iri_array(),
                single => single.as_iri().map(|i| vec![i]).unwrap_or_default(),
            };

            for ref_iri in &ref_iris {
                if let Some(referenced) = self.layer.resolve(ref_iri) {
                    if let Some(Value::Integer(ref_level)) =
                        referenced.get(&iri(wk::UNIVERSE_LEVEL))
                    {
                        if *ref_level >= this_level {
                            errors.push(ValidationError {
                                resource_id: res_id.clone(),
                                property: Some(prop_iri.clone()),
                                rule: ValidationRule::UniverseStratificationViolation,
                                message: format!(
                                    "resource at universe level {} references '{}' at level {} \
                                     (must be strictly lower)",
                                    this_level, ref_iri, ref_level
                                ),
                            });
                        }
                    }
                    // No universe_level on referenced → domain resource → always OK
                }
            }
        }

        errors
    }
}

// --- Module-level shared helpers ---

/// Helper: parse a well-known constant into an Iri.
///
/// Re-exported from [`crate::ontology::well_known::iri`] so the existing
/// validation-internal callers don't need updating. New code should
/// import directly from `ontology::well_known`.
pub(crate) use crate::ontology::well_known::iri;

/// Helper: extract a single resource-IRI from a Value. Accepts both
/// `Value::ResourceRef` (canonical) and `Value::String` (the JSON
/// parser stores all strings as `Value::String` — `data_type` is
/// frequently authored as a bare string in source ontologies).
pub(crate) fn value_as_iri(value: &Value) -> Option<Iri> {
    match value {
        Value::ResourceRef(i) => Some(i.clone()),
        Value::String(s) => Iri::parse(s).ok(),
        _ => None,
    }
}

/// Helper: check that `value` is an embedded `InductiveArgType`
/// resource representing `Option<class_iri>`. Used by the
/// MergeComorphism shape check (Rule 18) to verify the third
/// binder's parameter_type is `Option<A>`.
fn is_option_of_class(value: &Value, class_iri: &str) -> bool {
    let Value::Embedded(r) = value else {
        return false;
    };
    let resource: &Resource = r.as_ref();
    let inductive_arg_type = iri(wk::INDUCTIVE_ARG_TYPE);
    if !resource.is_instance_of(&inductive_arg_type) {
        return false;
    }
    let Some(name_value) = resource.get(&iri(wk::TYPE_NAME)) else {
        return false;
    };
    let Some(name_str) = name_value.as_iri_str() else {
        return false;
    };
    if name_str != wk::OPTION {
        return false;
    }
    let Some(Value::Array(args)) = resource.get(&iri(wk::TYPE_ARGS)) else {
        return false;
    };
    if args.len() != 1 {
        return false;
    }
    args[0].as_iri_str() == Some(class_iri)
}

/// Format a resource's `is_a` list for inclusion in an error message.
/// `[]` for empty, otherwise `[a, b, c]`.
pub(crate) fn format_is_a_list(classes: Vec<Iri>) -> String {
    if classes.is_empty() {
        "[]".to_string()
    } else {
        let joined = classes
            .iter()
            .map(|i| i.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        format!("[{joined}]")
    }
}

/// Format an `&[&Iri]` slice for inclusion in an error message.
pub(crate) fn format_iri_refs(refs: &[&Iri]) -> String {
    let joined = refs
        .iter()
        .map(|i| i.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{joined}]")
}

/// Bundle of "what we're checking" for a Comorphism's typed-reference
/// fields (rule 15, D14 §4.5). Two such values cover `export_format`
/// and `import_format`.
#[derive(Copy, Clone)]
struct ComorphismFormatRef<'a> {
    /// Field IRI on the Comorphism resource.
    field_iri: &'a str,
    /// Human label for the field used in error messages.
    field_label: &'a str,
    /// Class IRI the referenced resource must be an instance of.
    expected_class_iri: &'a str,
    /// Human label for the expected class.
    expected_label: &'a str,
}

#[cfg(test)]
mod tests {
    //! Driver-loop and cross-rule integration tests, plus the test
    //! helpers (`build_core_layer`, `make_resource`, …) re-used by
    //! the per-rule test modules. Rule-specific test cases live
    //! alongside their rule under `validation::rules::*`.

    use super::*;
    use crate::layer::LayerBuilder;
    use crate::ontology::eigon_json;
    use std::sync::Arc;

    pub(in crate::validation) fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    pub(in crate::validation) fn make_resource(id: &str, props: Vec<(&str, Value)>) -> Resource {
        let mut r = Resource::new(iri(id));
        for (k, v) in props {
            r.set(iri(k), v);
        }
        r
    }

    pub(in crate::validation) fn build_core_layer() -> Arc<Layer> {
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let resources = eigon_json::parse_document(core_json).unwrap();
        let mut builder = LayerBuilder::new("core", None);
        for r in resources {
            builder.add_resource(r).unwrap();
        }
        Arc::new(builder.build(crate::layer::LayerStorage::in_memory()))
    }

    #[test]
    fn core_ontology_validates_against_itself() {
        let core = build_core_layer();
        let validator = Validator::new(Arc::clone(&core));
        let errors = validator.validate();
        for e in &errors {
            eprintln!("  {e}");
        }
        assert!(
            errors.is_empty(),
            "core ontology should validate against itself"
        );
    }

    #[test]
    fn missing_required_property() {
        let core = build_core_layer();

        // Create a domain layer with a resource that's an instance of Property
        // but missing the required 'data_type' property
        let mut builder = LayerBuilder::new("test", Some(core));
        builder
            .add_resource(make_resource(
                "urn:eigenius:test:bad_prop",
                vec![
                    (
                        wk::IS_A,
                        Value::Array(vec![Value::String(wk::PROPERTY.to_string())]),
                    ),
                    (wk::DESCRIPTION, Value::String("missing data_type".into())),
                    (wk::SHORT_NAME, Value::String("bad_prop".into())),
                    // data_type is missing!
                ],
            ))
            .unwrap();
        let layer = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));

        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        assert!(errors
            .iter()
            .any(|e| e.rule == ValidationRule::MissingRequired));
    }

    #[test]
    fn valid_resource_passes() {
        let core = build_core_layer();
        let mut builder = LayerBuilder::new("test", Some(core));

        builder
            .add_resource(make_resource(
                "urn:eigenius:test:my_prop",
                vec![
                    (
                        wk::IS_A,
                        Value::Array(vec![Value::String(wk::PROPERTY.to_string())]),
                    ),
                    (wk::DESCRIPTION, Value::String("A valid property".into())),
                    (wk::SHORT_NAME, Value::String("my_prop".into())),
                    (wk::DATA_TYPE_PROP, Value::String(wk::STRING.to_string())),
                ],
            ))
            .unwrap();

        let layer = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let my_errors: Vec<_> = errors
            .iter()
            .filter(|e| {
                e.resource_id.as_ref().map(|i| i.as_str()) == Some("urn:eigenius:test:my_prop")
            })
            .collect();
        assert!(my_errors.is_empty(), "valid resource should have no errors");
    }

    // --- Epistemic base class validation (Step 6) ---

    fn build_full_bootstrap_layer() -> Arc<Layer> {
        let ctx = crate::bootstrap::bootstrap().unwrap();
        // Return the head layer (reflection, on top of program, on top of core)
        ctx.head().clone()
    }

    #[test]
    fn derived_resource_without_derivation_passes_with_recommendation() {
        // Per the reflection ontology, `derivation` is *recommended*
        // (not required) on `DerivedResource`. A resource carrying the
        // epistemic stamp without a chain-resident trace IRI still
        // validates — substrate-produced resources from FIBER ... INTO
        // commits and post-translation comorphism reify outputs are
        // derived by construction but may not have a kernel-generated
        // ProgramTrace yet (D14 §9.3 chain reinsertion). When the
        // kernel does generate a trace (RunProgram, AutoOnLoad fires),
        // it sets `derivation` so the audit trail is complete.
        let base = build_full_bootstrap_layer();
        let mut builder = LayerBuilder::new("test", Some(base));
        builder
            .add_resource(make_resource(
                "urn:eigenius:test:bad_derived",
                vec![(
                    wk::IS_A,
                    Value::Array(vec![Value::String(
                        "urn:eigenius:reflection:DerivedResource".to_string(),
                    )]),
                )],
            ))
            .unwrap();

        let layer = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();

        let derived_errors: Vec<_> = errors
            .iter()
            .filter(|e| {
                e.resource_id.as_ref().map(|i| i.as_str()) == Some("urn:eigenius:test:bad_derived")
                    && e.rule == ValidationRule::MissingRequired
            })
            .collect();
        assert!(
            derived_errors.is_empty(),
            "DerivedResource without 'derivation' should validate (derivation is recommended, not required), got: {derived_errors:?}"
        );
    }

    #[test]
    fn declared_resource_with_declared_by_passes() {
        let base = build_full_bootstrap_layer();
        let mut builder = LayerBuilder::new("test", Some(base));

        builder
            .add_resource(make_resource(
                "urn:eigenius:test:good_declared",
                vec![
                    (
                        wk::IS_A,
                        Value::Array(vec![Value::String(
                            "urn:eigenius:reflection:DeclaredResource".to_string(),
                        )]),
                    ),
                    (
                        "urn:eigenius:reflection:declared_by",
                        Value::String("test user".into()),
                    ),
                ],
            ))
            .unwrap();

        let layer = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();

        let declared_errors: Vec<_> = errors
            .iter()
            .filter(|e| {
                e.resource_id.as_ref().map(|i| i.as_str())
                    == Some("urn:eigenius:test:good_declared")
            })
            .collect();
        assert!(
            declared_errors.is_empty(),
            "DeclaredResource with 'declared_by' should pass: {declared_errors:?}"
        );
    }

    #[test]
    fn declared_resource_without_declared_by_fails() {
        let base = build_full_bootstrap_layer();
        let mut builder = LayerBuilder::new("test", Some(base));

        builder
            .add_resource(make_resource(
                "urn:eigenius:test:bad_declared",
                vec![(
                    wk::IS_A,
                    Value::Array(vec![Value::String(
                        "urn:eigenius:reflection:DeclaredResource".to_string(),
                    )]),
                )],
            ))
            .unwrap();

        let layer = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();

        assert!(
            errors.iter().any(|e| {
                e.resource_id.as_ref().map(|i| i.as_str()) == Some("urn:eigenius:test:bad_declared")
                    && e.rule == ValidationRule::MissingRequired
            }),
            "DeclaredResource without 'declared_by' should fail"
        );
    }

    #[test]
    fn observed_resource_without_source_fails() {
        let base = build_full_bootstrap_layer();
        let mut builder = LayerBuilder::new("test", Some(base));

        builder
            .add_resource(make_resource(
                "urn:eigenius:test:bad_observed",
                vec![(
                    wk::IS_A,
                    Value::Array(vec![Value::String(
                        "urn:eigenius:reflection:ObservedResource".to_string(),
                    )]),
                )],
            ))
            .unwrap();

        let layer = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();

        assert!(
            errors.iter().any(|e| {
                e.resource_id.as_ref().map(|i| i.as_str()) == Some("urn:eigenius:test:bad_observed")
                    && e.rule == ValidationRule::MissingRequired
            }),
            "ObservedResource without 'source' should fail"
        );
    }

    #[test]
    fn verified_resource_requires_both_derivation_and_verification() {
        let base = build_full_bootstrap_layer();
        let mut builder = LayerBuilder::new("test", Some(base));

        // VerifiedResource subclasses DerivedResource, so needs both
        // 'derivation' (from DerivedResource) and 'verification' (its own)
        builder
            .add_resource(make_resource(
                "urn:eigenius:test:bad_verified",
                vec![(
                    wk::IS_A,
                    Value::Array(vec![Value::String(
                        "urn:eigenius:reflection:VerifiedResource".to_string(),
                    )]),
                )],
            ))
            .unwrap();

        let layer = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();

        let verified_errors: Vec<_> = errors
            .iter()
            .filter(|e| {
                e.resource_id.as_ref().map(|i| i.as_str()) == Some("urn:eigenius:test:bad_verified")
                    && e.rule == ValidationRule::MissingRequired
            })
            .collect();
        // Should require both 'derivation' and 'verification'
        assert!(
            verified_errors.len() >= 2,
            "VerifiedResource should require both 'derivation' and 'verification', got {} errors: {verified_errors:?}",
            verified_errors.len()
        );
    }

    // --- Universe stratification tests (Phase 10b) ---

    /// Build a layer with the reflection ontology for stratification tests.
    /// Includes a `ref_prop` property with data_type=resource so the
    /// stratification checker has something to inspect.
    fn build_stratification_layer() -> Arc<Layer> {
        let base = build_full_bootstrap_layer();
        let mut builder = LayerBuilder::new("strat_test", Some(base));

        // Add a resource-typed property for referencing other resources
        builder
            .add_resource(make_resource(
                "urn:eigenius:test:ref_prop",
                vec![
                    (
                        wk::IS_A,
                        Value::Array(vec![Value::String(wk::PROPERTY.to_string())]),
                    ),
                    (
                        wk::DESCRIPTION,
                        Value::String("A reference property".into()),
                    ),
                    (wk::SHORT_NAME, Value::String("ref_prop".into())),
                    (wk::DATA_TYPE_PROP, Value::String(wk::RESOURCE.to_string())),
                ],
            ))
            .unwrap();

        Arc::new(builder.build(crate::layer::LayerStorage::in_memory()))
    }

    #[test]
    fn stratification_level1_referencing_level0_passes() {
        let base = build_stratification_layer();
        let mut builder = LayerBuilder::new("test", Some(base));

        // Level-0 resource
        builder
            .add_resource(make_resource(
                "urn:eigenius:test:level0",
                vec![(wk::UNIVERSE_LEVEL, Value::Integer(0))],
            ))
            .unwrap();

        // Level-1 resource referencing level-0 — should pass
        builder
            .add_resource(make_resource(
                "urn:eigenius:test:level1",
                vec![
                    (wk::UNIVERSE_LEVEL, Value::Integer(1)),
                    (
                        "urn:eigenius:test:ref_prop",
                        Value::String("urn:eigenius:test:level0".to_string()),
                    ),
                ],
            ))
            .unwrap();

        let layer = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();

        let strat_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::UniverseStratificationViolation)
            .collect();
        assert!(
            strat_errors.is_empty(),
            "level-1 referencing level-0 should pass: {strat_errors:?}"
        );
    }

    #[test]
    fn stratification_level1_referencing_level1_rejected() {
        let base = build_stratification_layer();
        let mut builder = LayerBuilder::new("test", Some(base));

        // Two level-1 resources, one referencing the other — should fail
        builder
            .add_resource(make_resource(
                "urn:eigenius:test:peer_a",
                vec![(wk::UNIVERSE_LEVEL, Value::Integer(1))],
            ))
            .unwrap();

        builder
            .add_resource(make_resource(
                "urn:eigenius:test:peer_b",
                vec![
                    (wk::UNIVERSE_LEVEL, Value::Integer(1)),
                    (
                        "urn:eigenius:test:ref_prop",
                        Value::String("urn:eigenius:test:peer_a".to_string()),
                    ),
                ],
            ))
            .unwrap();

        let layer = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();

        let strat_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::UniverseStratificationViolation)
            .collect();
        assert!(
            !strat_errors.is_empty(),
            "level-1 referencing level-1 should be rejected"
        );
    }

    #[test]
    fn stratification_domain_resources_always_referenceable() {
        let base = build_stratification_layer();
        let mut builder = LayerBuilder::new("test", Some(base));

        // A domain resource with no universe_level
        builder
            .add_resource(make_resource(
                "urn:eigenius:test:domain_thing",
                vec![(wk::DESCRIPTION, Value::String("just a thing".into()))],
            ))
            .unwrap();

        // Level-1 resource referencing domain resource — should pass
        builder
            .add_resource(make_resource(
                "urn:eigenius:test:meta1",
                vec![
                    (wk::UNIVERSE_LEVEL, Value::Integer(1)),
                    (
                        "urn:eigenius:test:ref_prop",
                        Value::String("urn:eigenius:test:domain_thing".to_string()),
                    ),
                ],
            ))
            .unwrap();

        let layer = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();

        let strat_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::UniverseStratificationViolation)
            .collect();
        assert!(
            strat_errors.is_empty(),
            "referencing domain resources (no universe_level) should always pass: {strat_errors:?}"
        );
    }

    #[test]
    fn stratification_level2_referencing_level1_passes() {
        let base = build_stratification_layer();
        let mut builder = LayerBuilder::new("test", Some(base));

        builder
            .add_resource(make_resource(
                "urn:eigenius:test:trace",
                vec![(wk::UNIVERSE_LEVEL, Value::Integer(1))],
            ))
            .unwrap();

        builder
            .add_resource(make_resource(
                "urn:eigenius:test:meta_trace",
                vec![
                    (wk::UNIVERSE_LEVEL, Value::Integer(2)),
                    (
                        "urn:eigenius:test:ref_prop",
                        Value::String("urn:eigenius:test:trace".to_string()),
                    ),
                ],
            ))
            .unwrap();

        let layer = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();

        let strat_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::UniverseStratificationViolation)
            .collect();
        assert!(
            strat_errors.is_empty(),
            "level-2 referencing level-1 should pass: {strat_errors:?}"
        );
    }

    #[test]
    fn stratification_universe_level_3_rejected_by_range() {
        // universe_level has max_value=2 in the reflection ontology
        let base = build_full_bootstrap_layer();
        let mut builder = LayerBuilder::new("test", Some(base));

        builder
            .add_resource(make_resource(
                "urn:eigenius:test:too_high",
                vec![(wk::UNIVERSE_LEVEL, Value::Integer(3))],
            ))
            .unwrap();

        let layer = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();

        let range_errors: Vec<_> = errors
            .iter()
            .filter(|e| {
                e.resource_id.as_ref().map(|i| i.as_str()) == Some("urn:eigenius:test:too_high")
                    && e.rule == ValidationRule::RangeViolation
            })
            .collect();
        assert!(
            !range_errors.is_empty(),
            "universe_level=3 should be rejected by max_value=2 range check"
        );
    }

    // --- ProgramTrace validation tests (Phase 10b) ---

    #[test]
    fn program_trace_with_all_required_fields_passes() {
        let base = build_full_bootstrap_layer();
        let mut builder = LayerBuilder::new("test", Some(base));

        // A ProgramTrace with all four required fields
        builder
            .add_resource(make_resource(
                "urn:eigenius:test:good_trace",
                vec![
                    (
                        wk::IS_A,
                        Value::Array(vec![Value::String(
                            "urn:eigenius:reflection:ProgramTrace".to_string(),
                        )]),
                    ),
                    (
                        "urn:eigenius:reflection:program",
                        Value::String("urn:eigenius:test:some_program".to_string()),
                    ),
                    (
                        "urn:eigenius:reflection:trace_tree",
                        Value::Embedded(Box::new(Resource::new_embedded())),
                    ),
                    (
                        "urn:eigenius:reflection:started_at",
                        Value::String("2026-04-23T12:00:00Z".to_string()),
                    ),
                    (
                        "urn:eigenius:reflection:completed_at",
                        Value::String("2026-04-23T12:00:01Z".to_string()),
                    ),
                ],
            ))
            .unwrap();

        let layer = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();

        let trace_errors: Vec<_> = errors
            .iter()
            .filter(|e| {
                e.resource_id.as_ref().map(|i| i.as_str()) == Some("urn:eigenius:test:good_trace")
                    && e.rule == ValidationRule::MissingRequired
            })
            .collect();
        assert!(
            trace_errors.is_empty(),
            "ProgramTrace with all required fields should pass: {trace_errors:?}"
        );
    }

    #[test]
    fn program_trace_missing_required_fields_fails() {
        let base = build_full_bootstrap_layer();
        let mut builder = LayerBuilder::new("test", Some(base));

        // A ProgramTrace missing started_at and completed_at. (Note:
        // `trace_tree` is *recommended*, not required — pure-leaf
        // programs like a `Var`-body identity produce no trace tree,
        // and the reflection ontology accepts traces without one. See
        // `Trace` enum's leaf comment in kernel/src/program/trace.rs.)
        builder
            .add_resource(make_resource(
                "urn:eigenius:test:bad_trace",
                vec![
                    (
                        wk::IS_A,
                        Value::Array(vec![Value::String(
                            "urn:eigenius:reflection:ProgramTrace".to_string(),
                        )]),
                    ),
                    (
                        "urn:eigenius:reflection:program",
                        Value::String("urn:eigenius:test:some_program".to_string()),
                    ),
                    // Missing: started_at, completed_at
                ],
            ))
            .unwrap();

        let layer = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();

        let missing_errors: Vec<_> = errors
            .iter()
            .filter(|e| {
                e.resource_id.as_ref().map(|i| i.as_str()) == Some("urn:eigenius:test:bad_trace")
                    && e.rule == ValidationRule::MissingRequired
            })
            .collect();
        // Two required fields are missing: started_at, completed_at.
        assert!(
            missing_errors.len() >= 2,
            "ProgramTrace missing 2 required fields should have >= 2 errors, got {}: {missing_errors:?}",
            missing_errors.len()
        );
        // Pin which fields are actually flagged — guards against a
        // regression where `trace_tree` accidentally moves back to
        // `requires` (or where started_at / completed_at quietly
        // disappear from the requires list).
        let missing_props: std::collections::BTreeSet<&str> = missing_errors
            .iter()
            .filter_map(|e| e.property.as_ref().map(|i| i.as_str()))
            .collect();
        assert!(
            missing_props.contains("urn:eigenius:reflection:started_at"),
            "expected `started_at` to be flagged missing; flagged set = {missing_props:?}",
        );
        assert!(
            missing_props.contains("urn:eigenius:reflection:completed_at"),
            "expected `completed_at` to be flagged missing; flagged set = {missing_props:?}",
        );
        assert!(
            !missing_props.contains("urn:eigenius:reflection:trace_tree"),
            "`trace_tree` is recommended, not required — a missing trace_tree must not surface as a MissingRequired error: {missing_props:?}",
        );
    }

    // --- Comorphism well-formedness tests (Rule 15, D14 §4.5 / §5) ---

    /// Build a layer chain containing the bootstrap ontologies (core +
    /// institution + program + reflection + notebook). Used by tests
    /// that exercise the Comorphism well-formedness rule (D14 §4.5)
    /// since Comorphism / ExportFormat / ImportFormat are declared in
    /// the institution ontology, not core.
    fn build_bootstrap_layer() -> Arc<Layer> {
        Arc::clone(crate::bootstrap::bootstrap().unwrap().head())
    }

    fn comorphism_format_ref(
        id: &str,
        is_a_class: &str,
        institution_ref: &str,
        procedure: &str,
    ) -> Resource {
        let from_to_field = if is_a_class == wk::EXPORT_FORMAT_CLASS {
            "urn:eigenius:institution:from_class"
        } else {
            "urn:eigenius:institution:to_class"
        };
        make_resource(
            id,
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(is_a_class.into())]),
                ),
                (
                    from_to_field,
                    Value::String("urn:eigenius:test:SomeClass".into()),
                ),
                (
                    "urn:eigenius:institution:payload_type",
                    Value::String("urn:eigenius:core:float".into()),
                ),
                (
                    "urn:eigenius:institution:institution_ref",
                    Value::String(institution_ref.into()),
                ),
                (
                    "urn:eigenius:institution:procedure",
                    Value::String(procedure.into()),
                ),
            ],
        )
    }

    fn comorphism_with(
        id: &str,
        export_format: &str,
        transformation: &str,
        import_format: &str,
    ) -> Resource {
        make_resource(
            id,
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(wk::COMORPHISM.into())]),
                ),
                (wk::EXPORT_FORMAT, Value::String(export_format.into())),
                (wk::TRANSFORMATION, Value::String(transformation.into())),
                (wk::IMPORT_FORMAT, Value::String(import_format.into())),
                (wk::EXACT, Value::Boolean(false)),
            ],
        )
    }

    #[test]
    fn well_formed_comorphism_passes_typing_check() {
        let bootstrap = build_bootstrap_layer();
        let mut top = LayerBuilder::new("test", Some(bootstrap));

        // A target resource the transformation can resolve to. The
        // current rule only requires this to exist; M5 will tighten
        // it to require a typed Component.
        top.add_resource(make_resource(
            "urn:eigenius:test:transform",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(wk::CLASS.into())]),
                ),
                (wk::SHORT_NAME, Value::String("transform".into())),
                (wk::DESCRIPTION, Value::String("placeholder".into())),
            ],
        ))
        .unwrap();

        top.add_resource(comorphism_format_ref(
            "urn:eigenius:test:ef",
            wk::EXPORT_FORMAT_CLASS,
            "urn:eigenius:test:inst",
            "urn:eigenius:test:proc:extract",
        ))
        .unwrap();
        top.add_resource(comorphism_format_ref(
            "urn:eigenius:test:imf",
            wk::IMPORT_FORMAT_CLASS,
            "urn:eigenius:test:inst",
            "urn:eigenius:test:proc:reify",
        ))
        .unwrap();
        top.add_resource(comorphism_with(
            "urn:eigenius:test:cm",
            "urn:eigenius:test:ef",
            "urn:eigenius:test:transform",
            "urn:eigenius:test:imf",
        ))
        .unwrap();

        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));
        let validator = Validator::new(Arc::clone(&layer));
        let comorphism_errors: Vec<_> = validator
            .validate()
            .into_iter()
            .filter(|e| e.message.contains("Comorphism"))
            .collect();
        assert!(
            comorphism_errors.is_empty(),
            "well-formed Comorphism should pass; got {comorphism_errors:?}"
        );
    }

    #[test]
    fn comorphism_with_missing_export_format_is_rejected() {
        let bootstrap = build_bootstrap_layer();
        let mut top = LayerBuilder::new("test", Some(bootstrap));

        top.add_resource(make_resource(
            "urn:eigenius:test:transform",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(wk::CLASS.into())]),
                ),
                (wk::SHORT_NAME, Value::String("transform".into())),
                (wk::DESCRIPTION, Value::String("placeholder".into())),
            ],
        ))
        .unwrap();
        top.add_resource(comorphism_format_ref(
            "urn:eigenius:test:imf",
            wk::IMPORT_FORMAT_CLASS,
            "urn:eigenius:test:inst",
            "urn:eigenius:test:proc:reify",
        ))
        .unwrap();
        // Comorphism references an ExportFormat that isn't in the chain.
        top.add_resource(comorphism_with(
            "urn:eigenius:test:cm",
            "urn:eigenius:test:missing_ef",
            "urn:eigenius:test:transform",
            "urn:eigenius:test:imf",
        ))
        .unwrap();

        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let dangling: Vec<_> = errors
            .iter()
            .filter(|e| {
                e.rule == ValidationRule::UnresolvedClassReference
                    && e.message.contains("Comorphism.export_format")
            })
            .collect();
        assert_eq!(
            dangling.len(),
            1,
            "expected one UnresolvedClassReference on Comorphism.export_format; got {errors:?}"
        );
    }

    #[test]
    fn comorphism_with_export_format_pointing_at_wrong_class_is_rejected() {
        let bootstrap = build_bootstrap_layer();
        let mut top = LayerBuilder::new("test", Some(bootstrap));

        top.add_resource(make_resource(
            "urn:eigenius:test:transform",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(wk::CLASS.into())]),
                ),
                (wk::SHORT_NAME, Value::String("transform".into())),
                (wk::DESCRIPTION, Value::String("placeholder".into())),
            ],
        ))
        .unwrap();

        // An ImportFormat resource accidentally referenced from the
        // Comorphism's `export_format` slot.
        top.add_resource(comorphism_format_ref(
            "urn:eigenius:test:wrong",
            wk::IMPORT_FORMAT_CLASS,
            "urn:eigenius:test:inst",
            "urn:eigenius:test:proc:reify",
        ))
        .unwrap();
        top.add_resource(comorphism_format_ref(
            "urn:eigenius:test:imf",
            wk::IMPORT_FORMAT_CLASS,
            "urn:eigenius:test:inst",
            "urn:eigenius:test:proc:reify",
        ))
        .unwrap();
        top.add_resource(comorphism_with(
            "urn:eigenius:test:cm",
            "urn:eigenius:test:wrong",
            "urn:eigenius:test:transform",
            "urn:eigenius:test:imf",
        ))
        .unwrap();

        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let mismatched: Vec<_> = errors
            .iter()
            .filter(|e| {
                e.rule == ValidationRule::UnresolvedClassReference
                    && e.message.contains("Comorphism.export_format")
                    && e.message.contains("not an instance of ExportFormat")
            })
            .collect();
        assert_eq!(
            mismatched.len(),
            1,
            "expected one UnresolvedClassReference flagging the wrong-class export_format; \
             got {errors:?}"
        );
    }

    #[test]
    fn comorphism_with_unresolvable_transformation_is_rejected() {
        let bootstrap = build_bootstrap_layer();
        let mut top = LayerBuilder::new("test", Some(bootstrap));

        top.add_resource(comorphism_format_ref(
            "urn:eigenius:test:ef",
            wk::EXPORT_FORMAT_CLASS,
            "urn:eigenius:test:inst",
            "urn:eigenius:test:proc:extract",
        ))
        .unwrap();
        top.add_resource(comorphism_format_ref(
            "urn:eigenius:test:imf",
            wk::IMPORT_FORMAT_CLASS,
            "urn:eigenius:test:inst",
            "urn:eigenius:test:proc:reify",
        ))
        .unwrap();
        // Transformation IRI points at nothing.
        top.add_resource(comorphism_with(
            "urn:eigenius:test:cm",
            "urn:eigenius:test:ef",
            "urn:eigenius:test:absent_transform",
            "urn:eigenius:test:imf",
        ))
        .unwrap();

        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let dangling: Vec<_> = errors
            .iter()
            .filter(|e| {
                e.rule == ValidationRule::UnresolvedClassReference
                    && e.message.contains("Comorphism.transformation")
            })
            .collect();
        assert_eq!(
            dangling.len(),
            1,
            "expected one UnresolvedClassReference for unresolvable transformation; got {errors:?}"
        );
    }

    // --- D37 §5.2: MergeComorphism shape (Rule 18) ---

    /// Builds a layer with the core ontology + a `Patient` class and
    /// the supplied resources. The Patient class gives the
    /// MergeComorphism a real `merge_target_class` target.
    fn build_d37_layer(extras: Vec<Resource>) -> Arc<Layer> {
        let core = build_core_layer();
        let mut builder = LayerBuilder::new("d37-test", Some(core));
        // Minimal Patient class so target_class references resolve.
        let mut patient = Resource::new(iri("urn:test:Patient"));
        patient.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::CLASS))]),
        );
        patient.set(iri(wk::SHORT_NAME), Value::String("Patient".to_string()));
        builder.add_resource(patient).unwrap();
        for r in extras {
            builder.add_resource(r).unwrap();
        }
        Arc::new(builder.build(crate::layer::LayerStorage::in_memory()))
    }

    /// Build a 3-binder Lambda chain `λ a. λ b. λ opt. body` with
    /// the given parameter types and body expression. Pass `None`
    /// for any binder to omit its `parameter_type` (untyped binder).
    fn make_witness_lambda_chain(
        iri_str: &str,
        param_types: [Option<Value>; 3],
        body: Resource,
    ) -> Resource {
        let mut current = body;
        let params = ["a", "b", "opt"];
        for i in (0..3).rev() {
            let mut lam = Resource::new_embedded();
            lam.set(
                iri(wk::IS_A),
                Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:program:Lambda"))]),
            );
            lam.set(
                iri("urn:eigenius:program:parameter"),
                Value::String(params[i].to_string()),
            );
            if let Some(pt) = &param_types[i] {
                lam.set(iri("urn:eigenius:program:parameter_type"), pt.clone());
            }
            lam.set(
                iri("urn:eigenius:program:body"),
                Value::Embedded(Box::new(current)),
            );
            current = lam;
        }
        // Outermost lambda is the top-level resource — set its IRI.
        current.set_id(Some(iri(iri_str)));
        current
    }

    /// Build a `Var "b"` body — the simplest witness body.
    fn make_var_b_body() -> Resource {
        let mut r = Resource::new_embedded();
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:program:Var"))]),
        );
        r.set(
            iri("urn:eigenius:program:name"),
            Value::String("b".to_string()),
        );
        r
    }

    fn make_option_of(class_iri: &str) -> Value {
        let mut r = Resource::new_embedded();
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::INDUCTIVE_ARG_TYPE))]),
        );
        r.set(iri(wk::TYPE_NAME), Value::String(wk::OPTION.to_string()));
        r.set(
            iri(wk::TYPE_ARGS),
            Value::Array(vec![Value::ResourceRef(iri(class_iri))]),
        );
        Value::Embedded(Box::new(r))
    }

    fn make_merge_comorphism(iri_str: &str, target_class: &str, transformation: &str) -> Resource {
        let mut r = Resource::new(iri(iri_str));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::MERGE_COMORPHISM))]),
        );
        r.set(
            iri(wk::MERGE_TARGET_CLASS),
            Value::ResourceRef(iri(target_class)),
        );
        r.set(
            iri(wk::MERGE_TRANSFORMATION),
            Value::ResourceRef(iri(transformation)),
        );
        r
    }

    #[test]
    fn merge_comorphism_with_well_typed_witness_validates_clean() {
        let lambda = make_witness_lambda_chain(
            "urn:test:take_b_term",
            [
                Some(Value::ResourceRef(iri("urn:test:Patient"))),
                Some(Value::ResourceRef(iri("urn:test:Patient"))),
                Some(make_option_of("urn:test:Patient")),
            ],
            make_var_b_body(),
        );
        let comorphism = make_merge_comorphism(
            "urn:test:take_b",
            "urn:test:Patient",
            "urn:test:take_b_term",
        );
        let layer = build_d37_layer(vec![lambda, comorphism]);
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let shape_violations: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::MergeComorphismShapeViolation)
            .collect();
        assert!(
            shape_violations.is_empty(),
            "well-typed witness should not produce a shape violation; got {shape_violations:?}"
        );
    }

    #[test]
    fn merge_comorphism_with_unresolved_transformation_is_rejected() {
        // Transformation points at an IRI that doesn't resolve.
        let comorphism = make_merge_comorphism(
            "urn:test:take_b",
            "urn:test:Patient",
            "urn:test:missing_term",
        );
        let layer = build_d37_layer(vec![comorphism]);
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let matches: Vec<_> = errors
            .iter()
            .filter(|e| {
                e.rule == ValidationRule::MergeComorphismShapeViolation
                    && e.message.contains("does not resolve")
            })
            .collect();
        assert!(
            !matches.is_empty(),
            "missing transformation should be flagged; got errors {errors:?}"
        );
    }

    #[test]
    fn merge_comorphism_with_non_lambda_transformation_is_rejected() {
        // Transformation points at the Patient class (a Class, not a
        // Lambda). The shape check rejects it.
        let comorphism =
            make_merge_comorphism("urn:test:take_b", "urn:test:Patient", "urn:test:Patient");
        let layer = build_d37_layer(vec![comorphism]);
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let matches: Vec<_> = errors
            .iter()
            .filter(|e| {
                e.rule == ValidationRule::MergeComorphismShapeViolation
                    && e.message.contains("not a urn:eigenius:program:Lambda")
            })
            .collect();
        assert!(
            !matches.is_empty(),
            "non-Lambda transformation should be flagged; got errors {errors:?}"
        );
    }

    #[test]
    fn merge_comorphism_with_wrong_parameter_type_is_rejected() {
        // First binder's parameter_type is `urn:test:Visit` instead
        // of the comorphism's `merge_target_class` (Patient).
        let lambda = make_witness_lambda_chain(
            "urn:test:wrong_param_term",
            [
                Some(Value::ResourceRef(iri("urn:test:Visit"))),
                Some(Value::ResourceRef(iri("urn:test:Patient"))),
                Some(make_option_of("urn:test:Patient")),
            ],
            make_var_b_body(),
        );
        let comorphism = make_merge_comorphism(
            "urn:test:take_b",
            "urn:test:Patient",
            "urn:test:wrong_param_term",
        );
        let layer = build_d37_layer(vec![lambda, comorphism]);
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let matches: Vec<_> = errors
            .iter()
            .filter(|e| {
                e.rule == ValidationRule::MergeComorphismShapeViolation
                    && e.message.contains("binder #1")
            })
            .collect();
        assert!(
            !matches.is_empty(),
            "binder #1 parameter_type mismatch should be flagged; got errors {errors:?}"
        );
    }

    #[test]
    fn merge_comorphism_with_wrong_option_type_is_rejected() {
        // Third binder is `Option<Visit>` instead of `Option<Patient>`.
        let lambda = make_witness_lambda_chain(
            "urn:test:wrong_opt_term",
            [
                Some(Value::ResourceRef(iri("urn:test:Patient"))),
                Some(Value::ResourceRef(iri("urn:test:Patient"))),
                Some(make_option_of("urn:test:Visit")),
            ],
            make_var_b_body(),
        );
        let comorphism = make_merge_comorphism(
            "urn:test:take_b",
            "urn:test:Patient",
            "urn:test:wrong_opt_term",
        );
        let layer = build_d37_layer(vec![lambda, comorphism]);
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let matches: Vec<_> = errors
            .iter()
            .filter(|e| {
                e.rule == ValidationRule::MergeComorphismShapeViolation
                    && e.message.contains("binder #3")
            })
            .collect();
        assert!(
            !matches.is_empty(),
            "binder #3 parameter_type mismatch should be flagged; got errors {errors:?}"
        );
    }

    #[test]
    fn merge_comorphism_with_too_few_binders_is_rejected() {
        // Transformation is a SINGLE-binder Lambda instead of three
        // nested. The shape check rejects at depth 1.
        let mut single_lambda = Resource::new(iri("urn:test:single_term"));
        single_lambda.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:program:Lambda"))]),
        );
        single_lambda.set(
            iri("urn:eigenius:program:parameter"),
            Value::String("a".to_string()),
        );
        single_lambda.set(
            iri("urn:eigenius:program:body"),
            Value::Embedded(Box::new(make_var_b_body())),
        );
        let comorphism = make_merge_comorphism(
            "urn:test:take_b",
            "urn:test:Patient",
            "urn:test:single_term",
        );
        let layer = build_d37_layer(vec![single_lambda, comorphism]);
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let matches: Vec<_> = errors
            .iter()
            .filter(|e| {
                e.rule == ValidationRule::MergeComorphismShapeViolation
                    && e.message.contains("truncates")
            })
            .collect();
        assert!(
            !matches.is_empty(),
            "truncated lambda chain should be flagged; got errors {errors:?}"
        );
    }

    // --- D37 §5.1: Standalone Lambda well-typedness (Rule 19) ---

    /// Build a witness Lambda resource at `iri_str` carrying the
    /// canonical `pi a : C, b : C, opt : Option<C> => C` `program:type`
    /// for the supplied target class, plus its parameter_type slots,
    /// and the supplied innermost body. Used to test the body-vs-type
    /// check path end-to-end.
    fn make_witness_lambda_with_program_type(
        iri_str: &str,
        target_class: &str,
        body: Resource,
    ) -> Resource {
        let class_value = Value::ResourceRef(iri(target_class));
        let option_value = make_option_of(target_class);
        let param_types = [
            Some(class_value.clone()),
            Some(class_value.clone()),
            Some(option_value.clone()),
        ];
        let mut lambda = make_witness_lambda_chain(iri_str, param_types.clone(), body);

        // Build the Pi-term: `pi a : C, b : C, opt : Option<C> => C`.
        // Nested TypeBinderArrow resources.
        let mut pi_acc: Value = class_value;
        let params = ["a", "b", "opt"];
        let kinds = [
            param_types[0].as_ref().unwrap(),
            param_types[1].as_ref().unwrap(),
            param_types[2].as_ref().unwrap(),
        ];
        for i in (0..3).rev() {
            let mut ar = Resource::new_embedded();
            ar.set(
                iri(wk::IS_A),
                Value::Array(vec![Value::ResourceRef(iri(wk::TYPE_BINDER_ARROW))]),
            );
            ar.set(iri(wk::BINDER_NAME), Value::String(params[i].to_string()));
            ar.set(iri(wk::BINDER_KIND), kinds[i].clone());
            ar.set(iri(wk::BINDER_BODY), pi_acc);
            pi_acc = Value::Embedded(Box::new(ar));
        }
        lambda.set(iri(wk::PROGRAM_TYPE), pi_acc);
        lambda
    }

    #[test]
    fn standalone_lambda_with_well_typed_body_validates_clean() {
        // Body is `Var "b"`. The witness contract says the body's
        // type must match the codomain (Patient). `b` is bound at
        // type Patient — well-typed.
        let lambda = make_witness_lambda_with_program_type(
            "urn:test:take_b_term",
            "urn:test:Patient",
            make_var_b_body(),
        );
        let layer = build_d37_layer(vec![lambda]);
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let lambda_errs: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::LambdaTypeMismatch)
            .collect();
        assert!(
            lambda_errs.is_empty(),
            "well-typed witness body should validate clean; got {lambda_errs:?}"
        );
    }

    #[test]
    fn standalone_lambda_with_unbound_var_body_is_rejected() {
        // Body references `unbound_var` which isn't in scope. NbE
        // catches it.
        let mut bad_body = Resource::new_embedded();
        bad_body.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:program:Var"))]),
        );
        bad_body.set(
            iri("urn:eigenius:program:name"),
            Value::String("unbound_var".to_string()),
        );
        let lambda = make_witness_lambda_with_program_type(
            "urn:test:bad_term",
            "urn:test:Patient",
            bad_body,
        );
        let layer = build_d37_layer(vec![lambda]);
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let lambda_errs: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::LambdaTypeMismatch)
            .collect();
        assert!(
            !lambda_errs.is_empty(),
            "unbound-var witness body should be flagged; got errors {errors:?}"
        );
    }

    #[test]
    fn standalone_lambda_without_program_type_is_skipped() {
        // Lambda has no `program:type` declared — the validator
        // skips the body check (the check is additive over untyped
        // lambdas). No error should fire from Rule 19.
        let lambda = make_witness_lambda_chain(
            "urn:test:untyped_term",
            [None, None, None],
            make_var_b_body(),
        );
        let layer = build_d37_layer(vec![lambda]);
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let lambda_errs: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::LambdaTypeMismatch)
            .collect();
        assert!(
            lambda_errs.is_empty(),
            "untyped lambda should not fire Rule 19; got {lambda_errs:?}"
        );
    }

    #[test]
    fn embedded_lambda_without_id_is_skipped() {
        // Lambda inside a `program` body has no top-level @id; the
        // check skips it (the surrounding Pi handles its type
        // inference). We verify by committing a synthetic embedded
        // lambda as part of a wrapper resource — but since the
        // validator iterates top-level resources, the embedded
        // lambda never gets directly visited. This test pins that
        // behaviour structurally: no error should fire for any
        // committed embedded resource.
        let outer_iri = "urn:test:outer_with_embedded";
        let inner_lambda = {
            let mut lam = Resource::new_embedded();
            lam.set(
                iri(wk::IS_A),
                Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:program:Lambda"))]),
            );
            lam.set(
                iri("urn:eigenius:program:parameter"),
                Value::String("x".to_string()),
            );
            lam.set(
                iri("urn:eigenius:program:body"),
                Value::Embedded(Box::new(make_var_b_body())),
            );
            // No @id — embedded.
            lam
        };
        // Wrap in a generic top-level resource (Class) — just to
        // commit something containing an embedded lambda.
        let mut wrapper = Resource::new(iri(outer_iri));
        wrapper.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::CLASS))]),
        );
        wrapper.set(iri(wk::SHORT_NAME), Value::String("outer".to_string()));
        // Stash the embedded lambda on a benign property — even if
        // the validator picks it up, the lambda has no @id so Rule
        // 19 skips. The wrapper class itself doesn't get Rule 19.
        wrapper.set(
            iri(wk::DESCRIPTION),
            Value::Embedded(Box::new(inner_lambda)),
        );
        let layer = build_d37_layer(vec![wrapper]);
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let lambda_errs: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::LambdaTypeMismatch)
            .collect();
        assert!(
            lambda_errs.is_empty(),
            "embedded lambdas (no top-level @id) should not fire Rule 19; got {lambda_errs:?}"
        );
    }

    #[test]
    fn compiler_output_validates_clean_end_to_end() {
        // End-to-end smoke: compile a `merge_comorphism` inline body
        // through the ESL pipeline (which emits both the synthesised
        // Lambda and the MergeComorphism), load into a layer
        // alongside the Patient class, and verify the validator
        // accepts the result. This closes the compiler↔validator
        // loop: anything the compiler produces should pass the
        // validator's shape check.
        let resources = crate::esl::compile(
            r#"
            namespace ex = "urn:test";
            merge_comorphism ex:take_b for ex:Patient {
                (a, b, opt) => b
            }
            "#,
        )
        .unwrap();
        let layer = build_d37_layer(resources);
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let shape_violations: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::MergeComorphismShapeViolation)
            .collect();
        assert!(
            shape_violations.is_empty(),
            "compiler-produced witness must pass the shape validator; got {shape_violations:?}"
        );
    }

    #[test]
    fn merge_comorphism_with_untyped_binders_still_validates() {
        // parameter_type is optional today (a recommends, not
        // requires). When binders carry no parameter_type, the
        // shape check passes — only present-but-wrong slots are
        // flagged. This keeps the validator additive over existing
        // untyped lambdas.
        let lambda = make_witness_lambda_chain(
            "urn:test:untyped_term",
            [None, None, None],
            make_var_b_body(),
        );
        let comorphism = make_merge_comorphism(
            "urn:test:take_b",
            "urn:test:Patient",
            "urn:test:untyped_term",
        );
        let layer = build_d37_layer(vec![lambda, comorphism]);
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let shape_violations: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::MergeComorphismShapeViolation)
            .collect();
        assert!(
            shape_violations.is_empty(),
            "untyped binders should pass (only present-but-wrong slots are flagged); got {shape_violations:?}"
        );
    }
}
