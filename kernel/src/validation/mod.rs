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

pub mod retroactive;
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

/// FormulaTerm InductiveType IRI (D32 §4). Pinned here so the
/// formula-specific arity rule can short-circuit before doing any
/// chain resolution work.
const FORMULA_TERM_IRI: &str = "urn:eigenius:formulas:FormulaTerm";

/// Operator.operator_arity property IRI. Convenience integer; the
/// rank check uses it as a fast-path before the full
/// operator_signature walk (deferred to a follow-on landing).
const OPERATOR_ARITY_IRI: &str = "urn:eigenius:formulas:operator_arity";

/// Walk the left spine of an `App(App(App(head, a₃), a₂), a₁)` tree
/// and return `(head, [a₁, a₂, a₃])`. Spine args are emitted
/// **right-to-left** as the spine is traversed; the caller may want
/// to reverse if argument order matters semantically. For the arity
/// check, only the count matters so the order is irrelevant.
fn collect_app_spine(node: &serde_json::Value) -> (serde_json::Value, Vec<serde_json::Value>) {
    let mut spine = Vec::new();
    let mut cursor = node.clone();
    loop {
        let Some(obj) = cursor.as_object() else {
            return (cursor, spine);
        };
        let ctor = obj.get("ctor").and_then(|v| v.as_str()).unwrap_or("");
        if ctor != "App" {
            return (cursor, spine);
        }
        let args = obj
            .get("args")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if args.len() != 2 {
            return (cursor, spine);
        }
        let mut iter = args.into_iter();
        let head = iter.next().expect("len 2");
        let arg = iter.next().expect("len 2");
        spine.push(arg);
        cursor = head;
    }
}

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
    layer: Arc<Layer>,
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

        // Every `is_a` target must resolve in the layer chain.
        // `collect_effective_properties` silently skips unresolved
        // entries, which would mask a structurally invalid reference
        // (and let a resource declare `is_a: [SomethingNonExistent]`
        // without ever surfacing the broken IRI). Fire one error per
        // unresolved target. `is_a` targets that resolve but to a
        // resource without `class_types` containing `Class` /
        // `InductiveType` are caught elsewhere (Rule 14
        // `ClassTypeMismatch`); this check is the missing-link
        // counterpart to that rule.
        let is_a_iri = Iri::parse(wk::IS_A).expect("wk::IS_A is well-formed");
        for iri in &class_iris {
            if self.layer.resolve(iri).is_none() {
                errors.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: Some(is_a_iri.clone()),
                    rule: ValidationRule::UnresolvedClassReference,
                    message: format!("`is_a` references unresolved IRI '{iri}'"),
                });
            }
        }

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

        // Rule 14: Class-definition reference integrity (eigenius#26).
        // Verify that `requires` / `recommends` / `subclass_of` /
        // `class_types` / `data_type` IRIs declared by Class and
        // Property resources actually resolve to resources of the
        // expected kind in the layer chain. Forward references within
        // the same load batch are fine — by the time we run, the new
        // layer is fully assembled and `self.layer.resolve()` walks it
        // along with its parents.
        errors.extend(self.check_class_definition_references(resource, &res_id));

        // Rule 15: Comorphism well-formedness (D14 §4.5 / §5).
        // For Comorphism resources, verify that `export_format` and
        // `import_format` references resolve to ExportFormat /
        // ImportFormat resources, and that `transformation` resolves
        // to *some* resource in the chain. The full Mini-TT
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

    /// Collect effective `requires` and `recommends` from all classes and ancestors.
    fn collect_effective_properties(&self, class_iris: &[&Iri]) -> (BTreeSet<Iri>, BTreeSet<Iri>) {
        let mut required = BTreeSet::new();
        let mut recommended = BTreeSet::new();

        for class_iri in class_iris {
            self.collect_from_class(
                class_iri,
                &mut required,
                &mut recommended,
                &mut BTreeSet::new(),
            );
        }

        (required, recommended)
    }

    /// Recursively collect requires/recommends from a class and its ancestors.
    fn collect_from_class(
        &self,
        class_iri: &Iri,
        required: &mut BTreeSet<Iri>,
        recommended: &mut BTreeSet<Iri>,
        visited: &mut BTreeSet<Iri>,
    ) {
        if !visited.insert(class_iri.clone()) {
            return; // Already visited (handles cycles)
        }

        if let Some(class_def) = self.layer.resolve(class_iri) {
            // Collect requires
            if let Some(requires_val) = class_def.get(&iri(wk::REQUIRES)) {
                for prop_iri in requires_val.as_iri_array() {
                    required.insert(prop_iri);
                }
            }

            // Collect recommends
            if let Some(recommends_val) = class_def.get(&iri(wk::RECOMMENDS)) {
                for prop_iri in recommends_val.as_iri_array() {
                    recommended.insert(prop_iri);
                }
            }

            // Walk parent classes
            if let Some(parents_val) = class_def.get(&iri(wk::PARENT_CLASSES)) {
                for parent_iri in parents_val.as_iri_array() {
                    self.collect_from_class(&parent_iri, required, recommended, visited);
                }
            }
        }
    }

    /// Evaluate conditional_requires for all classes.
    fn evaluate_conditional_requires(
        &self,
        class_iris: &[&Iri],
        resource: &Resource,
    ) -> (BTreeSet<Iri>, BTreeSet<Iri>) {
        let mut required = BTreeSet::new();
        let mut recommended = BTreeSet::new();

        for class_iri in class_iris {
            if let Some(class_def) = self.layer.resolve(class_iri) {
                if let Some(conds) = class_def.get(&iri(wk::CONDITIONAL_REQUIRES)) {
                    if let Some(cond_array) = conds.as_array() {
                        for cond in cond_array {
                            if let Value::Embedded(cond_res) = cond {
                                self.evaluate_condition(
                                    cond_res,
                                    resource,
                                    &mut required,
                                    &mut recommended,
                                );
                            }
                        }
                    }
                }
            }
        }

        (required, recommended)
    }

    /// Evaluate a single ConditionalRequirement against a resource.
    fn evaluate_condition(
        &self,
        condition: &Resource,
        resource: &Resource,
        required: &mut BTreeSet<Iri>,
        recommended: &mut BTreeSet<Iri>,
    ) {
        // Get when_property — `data_type: resource`, so the canonical
        // shape is `ResourceRef`; `as_iri` also tolerates the
        // pre-canonical `String` shape.
        let when_prop = match condition
            .get(&iri(wk::WHEN_PROPERTY))
            .and_then(|v| v.as_iri())
        {
            Some(i) => i,
            None => return,
        };

        // Get has_value
        let has_values = match condition.get(&iri(wk::HAS_VALUE)) {
            Some(val) => val.as_iri_array(),
            None => return,
        };

        // Check if the resource's property value matches any has_value.
        // The value being matched is itself an IRI in either shape.
        let resource_value = match resource.get(&when_prop) {
            Some(v) => v,
            None => return,
        };
        let matches = resource_value
            .as_iri()
            .map(|val_iri| has_values.contains(&val_iri))
            .unwrap_or(false);

        if matches {
            // Apply then_requires
            if let Some(then_req) = condition.get(&iri(wk::THEN_REQUIRES)) {
                for prop_iri in then_req.as_iri_array() {
                    required.insert(prop_iri);
                }
            }
            // Apply then_recommends
            if let Some(then_rec) = condition.get(&iri(wk::THEN_RECOMMENDS)) {
                for prop_iri in then_rec.as_iri_array() {
                    recommended.insert(prop_iri);
                }
            }
        }
    }

    /// Rule 3: Type checking — value must match property's data_type.
    fn check_type(
        &self,
        prop_def: &Resource,
        value: &Value,
        prop_iri: &Iri,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let dt = match self.get_data_type_str(prop_def) {
            Some(dt) => dt,
            None => return vec![], // No data_type defined, skip
        };

        let ok = match dt.as_str() {
            wk::STRING => matches!(value, Value::String(_)),
            wk::INTEGER => matches!(value, Value::Integer(_)),
            wk::FLOAT => matches!(value, Value::Float(_) | Value::Integer(_)),
            wk::BOOLEAN => matches!(value, Value::Boolean(_)),
            wk::RESOURCE => {
                // Post-canonicalisation (see `LayerBuilder::build` ->
                // `canonicalise_resource_refs`), every resource-typed
                // value on a committed layer is either `ResourceRef`
                // (an IRI) or `Embedded` (an inlined Resource).
                // `Value::String` for a `data_type: resource`
                // property is a malformed declaration the
                // canonicaliser couldn't normalise (typically because
                // the property def is missing or its data_type isn't
                // resolvable) — flag it as a type mismatch rather
                // than silently accepting a non-canonical shape.
                //
                // When `class_types` declares an `InductiveType`, also
                // accept `Value::Json` — the tagged-dict carrier for
                // inductive values. The deeper structural check
                // (ctor / arg_types) runs in `check_class_types`,
                // mirroring the `core:inductive` split.
                if self.class_types_inductive_target(prop_def).is_some() {
                    matches!(
                        value,
                        Value::ResourceRef(_) | Value::Embedded(_) | Value::Json(_)
                    )
                } else {
                    matches!(value, Value::ResourceRef(_) | Value::Embedded(_))
                }
            }
            wk::RESOURCE_ARRAY => match value {
                Value::Array(arr) => {
                    if self.class_types_inductive_target(prop_def).is_some() {
                        arr.iter().all(|v| {
                            matches!(
                                v,
                                Value::ResourceRef(_) | Value::Embedded(_) | Value::Json(_)
                            )
                        })
                    } else {
                        arr.iter()
                            .all(|v| matches!(v, Value::ResourceRef(_) | Value::Embedded(_)))
                    }
                }
                _ => false,
            },
            wk::VALUE_ARRAY => matches!(value, Value::Array(_)),
            wk::JSON => true, // Any value is valid for JSON
            wk::INDUCTIVE => {
                // Wire-level shape check: an inductive value lands as
                // either a `Value::Json` carrying the tagged-dict tree
                // or a `Value::Embedded` resource. The deeper
                // structural type-check (ctor exists on declared
                // InductiveType, arg shapes match `arg_types`) lives in
                // `check_inductive_value` (rule 16) — same split as
                // `check_class_types` for `core:resource`.
                matches!(value, Value::Json(_) | Value::Embedded(_))
            }
            _ => true, // Unknown data type, skip
        };

        if ok {
            vec![]
        } else {
            vec![ValidationError {
                resource_id: res_id.clone(),
                property: Some(prop_iri.clone()),
                rule: ValidationRule::TypeMismatch,
                message: format!("expected data_type '{dt}', got incompatible value"),
            }]
        }
    }

    /// Rule 4: Format checking.
    fn check_format(
        &self,
        prop_def: &Resource,
        value: &Value,
        prop_iri: &Iri,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let format_str = match prop_def.get(&iri(wk::FORMAT_PROP)) {
            Some(Value::String(s)) => s.as_str(),
            _ => return vec![],
        };

        let string_val = match value.as_str() {
            Some(s) => s,
            None => return vec![], // Not a string, type checking handles this
        };

        let valid = match format_str {
            wk::FMT_DATE => is_valid_date(string_val),
            wk::FMT_DATETIME => is_valid_datetime(string_val),
            wk::FMT_TIME => is_valid_time(string_val),
            wk::FMT_IRI => Iri::parse(string_val).is_ok(),
            wk::FMT_UUID => is_valid_uuid(string_val),
            wk::FMT_REGEX => regex::Regex::new(string_val).is_ok(),
            _ => true, // Unknown format, skip
        };

        if valid {
            vec![]
        } else {
            vec![ValidationError {
                resource_id: res_id.clone(),
                property: Some(prop_iri.clone()),
                rule: ValidationRule::FormatViolation,
                message: format!("value '{string_val}' does not match format '{format_str}'"),
            }]
        }
    }

    /// Rule 5: Pattern checking.
    fn check_pattern(
        &self,
        prop_def: &Resource,
        value: &Value,
        prop_iri: &Iri,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let pattern_str = match prop_def.get(&iri(wk::PATTERN)) {
            Some(Value::String(s)) => s.as_str(),
            _ => return vec![],
        };

        let string_val = match value.as_str() {
            Some(s) => s,
            None => return vec![],
        };

        // Full match: wrap in ^...$
        let full_pattern = format!("^(?:{pattern_str})$");
        match regex::Regex::new(&full_pattern) {
            Ok(re) => {
                if re.is_match(string_val) {
                    vec![]
                } else {
                    vec![ValidationError {
                        resource_id: res_id.clone(),
                        property: Some(prop_iri.clone()),
                        rule: ValidationRule::PatternViolation,
                        message: format!(
                            "value '{string_val}' does not match pattern '{pattern_str}'"
                        ),
                    }]
                }
            }
            Err(_) => vec![], // Invalid regex in property def — skip (should be caught by format validation on the property itself)
        }
    }

    /// Rule 6: Range checking (min_value/max_value).
    fn check_range(
        &self,
        prop_def: &Resource,
        value: &Value,
        prop_iri: &Iri,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let num_val = match value {
            Value::Integer(n) => *n as f64,
            Value::Float(f) => *f,
            _ => return vec![],
        };

        let mut errors = Vec::new();

        if let Some(Value::Float(min)) = prop_def.get(&iri(wk::MIN_VALUE)) {
            if num_val < *min {
                errors.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: Some(prop_iri.clone()),
                    rule: ValidationRule::RangeViolation,
                    message: format!("value {num_val} is less than minimum {min}"),
                });
            }
        }
        if let Some(Value::Integer(min)) = prop_def.get(&iri(wk::MIN_VALUE)) {
            if num_val < *min as f64 {
                errors.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: Some(prop_iri.clone()),
                    rule: ValidationRule::RangeViolation,
                    message: format!("value {num_val} is less than minimum {min}"),
                });
            }
        }

        if let Some(Value::Float(max)) = prop_def.get(&iri(wk::MAX_VALUE)) {
            if num_val > *max {
                errors.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: Some(prop_iri.clone()),
                    rule: ValidationRule::RangeViolation,
                    message: format!("value {num_val} is greater than maximum {max}"),
                });
            }
        }
        if let Some(Value::Integer(max)) = prop_def.get(&iri(wk::MAX_VALUE)) {
            if num_val > *max as f64 {
                errors.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: Some(prop_iri.clone()),
                    rule: ValidationRule::RangeViolation,
                    message: format!("value {num_val} is greater than maximum {max}"),
                });
            }
        }

        errors
    }

    /// Rule 7: Length checking (min_length/max_length).
    fn check_length(
        &self,
        prop_def: &Resource,
        value: &Value,
        prop_iri: &Iri,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let len = match value {
            Value::String(s) => s.chars().count(),
            Value::Array(arr) => arr.len(),
            _ => return vec![],
        };

        let mut errors = Vec::new();

        if let Some(Value::Integer(min)) = prop_def.get(&iri(wk::MIN_LENGTH)) {
            if (len as i64) < *min {
                errors.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: Some(prop_iri.clone()),
                    rule: ValidationRule::LengthViolation,
                    message: format!("length {len} is less than minimum {min}"),
                });
            }
        }

        if let Some(Value::Integer(max)) = prop_def.get(&iri(wk::MAX_LENGTH)) {
            if (len as i64) > *max {
                errors.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: Some(prop_iri.clone()),
                    rule: ValidationRule::LengthViolation,
                    message: format!("length {len} is greater than maximum {max}"),
                });
            }
        }

        errors
    }

    /// Rule 8: Class type checking.
    ///
    /// `class_types` may name either a `Class` (the historical case —
    /// the value must be a ResourceRef/Embedded whose `is_a` matches)
    /// or an `InductiveType` (Option A unification — the value is a
    /// tagged-dict tree carried by `Value::Json`, and we dispatch to
    /// the inductive walker). Per the singleton constraint that
    /// already applies to `data_type: core:inductive`, an
    /// InductiveType `class_types` must be the sole entry; mixed
    /// Class/InductiveType lists are not a defined shape.
    fn check_class_types(
        &self,
        prop_def: &Resource,
        value: &Value,
        prop_iri: &Iri,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let allowed_classes = match prop_def.get(&iri(wk::CLASS_TYPES)) {
            Some(val) => val.as_iri_array(),
            None => return vec![],
        };

        if allowed_classes.is_empty() {
            return vec![];
        }

        // InductiveType branch: walk the tagged-dict tree(s).
        // Skipping non-Json elements here is intentional — wire-shape
        // (`check_type`) handles whether a Ref/Embedded is admissible
        // for the property's data_type; deep class-membership of
        // stored inductive instances is not a v1 use case.
        //
        // For `data_type: core:inductive`, `check_inductive_value`
        // (rule 16) owns the walk + the singleton precondition; we
        // defer to it to avoid duplicate diagnostics.
        let dt_is_inductive = self
            .get_data_type_str(prop_def)
            .map(|dt| dt == wk::INDUCTIVE)
            .unwrap_or(false);
        if !dt_is_inductive {
            if let Some(inductive_type) = self.class_types_inductive_target(prop_def) {
                let mut errors = Vec::new();
                match value {
                    Value::Json(_) => {
                        self.walk_inductive_value(
                            value,
                            &inductive_type,
                            prop_iri.as_str().to_string(),
                            res_id,
                            &mut errors,
                        );
                    }
                    Value::Array(arr) => {
                        for (i, v) in arr.iter().enumerate() {
                            if matches!(v, Value::Json(_)) {
                                let path = format!("{prop_iri}[{i}]");
                                self.walk_inductive_value(
                                    v,
                                    &inductive_type,
                                    path,
                                    res_id,
                                    &mut errors,
                                );
                            }
                        }
                    }
                    _ => {}
                }
                return errors;
            }
        }

        let allowed_refs: Vec<&Iri> = allowed_classes.iter().collect();

        let mut errors = Vec::new();
        let values_to_check = match value {
            Value::String(_) | Value::ResourceRef(_) | Value::Embedded(_) => vec![value],
            Value::Array(arr) => arr.iter().collect(),
            _ => return vec![],
        };

        for v in values_to_check {
            // Embedded resources are checked directly against the
            // allowed-class set; IRI references (in either canonical
            // `ResourceRef` or pre-canonical `String` shape) are
            // resolved through the chain first.
            if let Value::Embedded(embedded) = v {
                if !self.is_instance_of_any(embedded, &allowed_refs) {
                    let actual = format_is_a_list(embedded.is_a());
                    let allowed = format_iri_refs(&allowed_refs);
                    errors.push(ValidationError {
                        resource_id: res_id.clone(),
                        property: Some(prop_iri.clone()),
                        rule: ValidationRule::ClassTypeMismatch,
                        message: format!(
                            "embedded value on property '{prop_iri}' has is_a {actual} but must be an instance of one of: {allowed}"
                        ),
                    });
                }
                continue;
            }
            if let Some(ref_iri) = v.as_iri() {
                if let Some(referenced) = self.layer.resolve(&ref_iri) {
                    if !self.is_instance_of_any(&referenced, &allowed_refs) {
                        let actual = format_is_a_list(referenced.is_a());
                        let allowed = format_iri_refs(&allowed_refs);
                        errors.push(ValidationError {
                            resource_id: res_id.clone(),
                            property: Some(prop_iri.clone()),
                            rule: ValidationRule::ClassTypeMismatch,
                            message: format!(
                                "referenced resource '{ref_iri}' has is_a {actual} but must be an instance of one of: {allowed}"
                            ),
                        });
                    }
                }
                // If we can't resolve, skip — might be external.
            }
        }

        errors
    }

    /// Rule 9: Allowed values checking.
    fn check_allows_only(
        &self,
        prop_def: &Resource,
        value: &Value,
        prop_iri: &Iri,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let allowed = match prop_def.get(&iri(wk::ALLOWS_ONLY)) {
            Some(val) => val.as_iri_array(),
            None => return vec![],
        };

        if allowed.is_empty() {
            return vec![];
        }

        let allowed_set: BTreeSet<Iri> = allowed.into_iter().collect();
        let mut errors = Vec::new();

        // Collect candidate IRIs to test against the allows_only set.
        // Single-value properties hold one IRI directly; resource_array
        // properties hold a `Value::Array` of IRI elements. `as_iri`
        // accepts both canonical `ResourceRef` and pre-canonical
        // `String` shapes.
        let refs_to_check: Vec<Iri> = match value {
            Value::Array(arr) => arr.iter().filter_map(|v| v.as_iri()).collect(),
            single => single.as_iri().map(|i| vec![i]).unwrap_or_default(),
        };

        for ref_iri in refs_to_check {
            {
                if !allowed_set.contains(&ref_iri) {
                    errors.push(ValidationError {
                        resource_id: res_id.clone(),
                        property: Some(prop_iri.clone()),
                        rule: ValidationRule::AllowedValueViolation,
                        message: format!("value '{ref_iri}' is not in the allows_only set"),
                    });
                }
            }
        }

        errors
    }

    /// Rule 10: Domain checking.
    fn check_domain(
        &self,
        prop_def: &Resource,
        resource: &Resource,
        prop_iri: &Iri,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let domain_classes = match prop_def.get(&iri(wk::DOMAIN)) {
            Some(val) => val.as_iri_array(),
            None => return vec![], // No domain constraint
        };

        if domain_classes.is_empty() {
            return vec![];
        }

        let domain_refs: Vec<&Iri> = domain_classes.iter().collect();
        if self.is_instance_of_any(resource, &domain_refs) {
            vec![]
        } else {
            vec![ValidationError {
                resource_id: res_id.clone(),
                property: Some(prop_iri.clone()),
                rule: ValidationRule::DomainViolation,
                message: format!(
                    "property '{prop_iri}' is not allowed on this resource type (domain restriction)"
                ),
            }]
        }
    }

    /// Check if a resource is an instance of any of the given classes,
    /// considering subclass relationships.
    fn is_instance_of_any(&self, resource: &Resource, classes: &[&Iri]) -> bool {
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
    fn get_data_type_str(&self, prop_def: &Resource) -> Option<String> {
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

    /// Resolve `class_types` to an `InductiveType` resource when the
    /// property declares exactly one entry pointing to one. Returns
    /// `None` for the Class case (the original `class_types`
    /// semantics) and for mixed/empty lists. Powers the Option A
    /// unification across `core:resource`, `core:resource_array`,
    /// and (implicitly, via the singleton constraint) `core:inductive`.
    fn class_types_inductive_target(&self, prop_def: &Resource) -> Option<Arc<Resource>> {
        let class_iris = prop_def.get(&iri(wk::CLASS_TYPES))?.as_iri_array();
        if class_iris.len() != 1 {
            return None;
        }
        let target = self.layer.resolve(&class_iris[0])?;
        if target.is_instance_of(&iri(wk::INDUCTIVE_TYPE)) {
            Some(target)
        } else {
            None
        }
    }

    /// Rule 14: Class-definition reference integrity (eigenius#26).
    ///
    /// Rule 16: Inductive value type-checking (D32 §3.5).
    ///
    /// When a property has `data_type: core:inductive`, its `class_types`
    /// must declare exactly one `core:InductiveType`, and the value must
    /// be a tagged-dict tree (`{ "ctor": ..., "args": [...] }`) whose
    /// every node corresponds to a ctor declared on the inductive and
    /// whose every arg matches the ctor's declared `arg_types[i].type_name`.
    /// Errors carry structured paths so users see
    /// `term.args[0].args[1]: ctor 'foo' not declared on FormulaTerm`.
    fn check_inductive_value(
        &self,
        prop_def: &Resource,
        value: &Value,
        prop_iri: &Iri,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let dt = match self.get_data_type_str(prop_def) {
            Some(dt) => dt,
            None => return vec![],
        };
        if dt != wk::INDUCTIVE {
            return vec![];
        }

        let allowed = match prop_def.get(&iri(wk::CLASS_TYPES)) {
            Some(val) => val.as_iri_array(),
            None => Vec::new(),
        };
        if allowed.len() != 1 {
            return vec![ValidationError {
                resource_id: res_id.clone(),
                property: Some(prop_iri.clone()),
                rule: ValidationRule::TypeMismatch,
                message: format!(
                    "data_type 'core:inductive' requires exactly one `class_types` entry naming an InductiveType (got {})",
                    allowed.len()
                ),
            }];
        }
        let ind_iri = allowed.into_iter().next().expect("len 1");

        let ind_type = match self.layer.resolve(&ind_iri) {
            Some(r) => r,
            None => {
                return vec![ValidationError {
                    resource_id: res_id.clone(),
                    property: Some(prop_iri.clone()),
                    rule: ValidationRule::UnresolvedClassReference,
                    message: format!(
                        "inductive type '{ind_iri}' on `class_types` not found in chain"
                    ),
                }];
            }
        };
        if !ind_type.is_instance_of(&iri(wk::INDUCTIVE_TYPE)) {
            return vec![ValidationError {
                resource_id: res_id.clone(),
                property: Some(prop_iri.clone()),
                rule: ValidationRule::TypeMismatch,
                message: format!(
                    "`class_types` IRI '{ind_iri}' is not an InductiveType — `data_type: core:inductive` requires one"
                ),
            }];
        }

        let mut errors = Vec::new();
        self.walk_inductive_value(
            value,
            &ind_type,
            prop_iri.as_str().to_string(),
            res_id,
            &mut errors,
        );
        errors
    }

    /// Recursively type-check an inductive value tree against an
    /// `InductiveType` resource. `path` accumulates a structured trace
    /// (`term.args[0].args[1]`) for diagnostic clarity.
    fn walk_inductive_value(
        &self,
        value: &Value,
        inductive_type: &Resource,
        path: String,
        res_id: &Option<Iri>,
        out: &mut Vec<ValidationError>,
    ) {
        let json = match value {
            Value::Json(j) => j,
            other => {
                out.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: None,
                    rule: ValidationRule::InductiveValueMismatch,
                    message: format!(
                        "{path}: expected JSON tagged-dict for inductive value, got {other:?}"
                    ),
                });
                return;
            }
        };

        let obj = match json.as_object() {
            Some(o) => o,
            None => {
                out.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: None,
                    rule: ValidationRule::InductiveValueMismatch,
                    message: format!("{path}: inductive value must be a JSON object"),
                });
                return;
            }
        };

        let ctor_name = match obj.get("ctor").and_then(serde_json::Value::as_str) {
            Some(s) => s,
            None => {
                out.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: None,
                    rule: ValidationRule::InductiveValueMismatch,
                    message: format!("{path}: inductive value missing string `ctor` field"),
                });
                return;
            }
        };

        let args_array = obj
            .get("args")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();

        // Find the ctor declaration on the inductive type.
        let ctors_value = match inductive_type.get(&iri(wk::CTORS)) {
            Some(v) => v,
            None => {
                out.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: None,
                    rule: ValidationRule::InductiveValueMismatch,
                    message: format!(
                        "{path}: InductiveType `{}` has no `ctors` declared",
                        inductive_type.id().map(|i| i.as_str()).unwrap_or("?"),
                    ),
                });
                return;
            }
        };
        let ctor_arr = match ctors_value {
            Value::Array(a) => a,
            _ => return, // Earlier rules will have flagged this
        };

        let matching_ctor = ctor_arr.iter().find_map(|c| match c {
            Value::Embedded(r) => {
                let name = r
                    .get(&iri(wk::CTOR_NAME))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if name == ctor_name {
                    Some(r.as_ref())
                } else {
                    None
                }
            }
            _ => None,
        });

        let ctor = match matching_ctor {
            Some(c) => c,
            None => {
                out.push(ValidationError {
                    resource_id: res_id.clone(),
                    property: None,
                    rule: ValidationRule::InductiveValueMismatch,
                    message: format!(
                        "{path}: ctor `{ctor_name}` not declared on InductiveType `{}`",
                        inductive_type.id().map(|i| i.as_str()).unwrap_or("?"),
                    ),
                });
                return;
            }
        };

        let arg_types: Vec<Resource> = match ctor.get(&iri(wk::ARG_TYPES)) {
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|v| match v {
                    Value::Embedded(r) => Some(r.as_ref().clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };

        if args_array.len() != arg_types.len() {
            out.push(ValidationError {
                resource_id: res_id.clone(),
                property: None,
                rule: ValidationRule::InductiveValueMismatch,
                message: format!(
                    "{path}: ctor `{ctor_name}` expects {} arg(s), got {}",
                    arg_types.len(),
                    args_array.len(),
                ),
            });
            return;
        }

        for (i, (arg_value, arg_type_decl)) in args_array.iter().zip(arg_types.iter()).enumerate() {
            let type_name = arg_type_decl
                .get(&iri(wk::TYPE_NAME))
                .and_then(Value::as_str)
                .unwrap_or("");
            let child_path = format!("{path}.args[{i}]");
            self.check_inductive_arg(arg_value, type_name, child_path, res_id, out);
        }
    }

    /// Validate one argument value against the `type_name` declared on
    /// its `InductiveArgType`. Dispatches on whether `type_name` resolves
    /// to a primitive `DataType`, a `Class`, an `InductiveType`, or a
    /// bare type-parameter name (deferred — parameter-aware checking
    /// lands when parametric inductives have their first chain
    /// consumer; v1 callers use only monomorphic inductives).
    fn check_inductive_arg(
        &self,
        arg_value: &serde_json::Value,
        type_name: &str,
        path: String,
        res_id: &Option<Iri>,
        out: &mut Vec<ValidationError>,
    ) {
        // Try to parse as IRI and resolve. If it doesn't parse or
        // doesn't resolve, treat as an unbound parameter name and
        // skip (v1 deferral).
        let type_iri = match Iri::parse(type_name) {
            Ok(i) => i,
            Err(_) => return, // Bare parameter name; deferred per v1.
        };

        // Primitive type IRIs are well-known; check inline.
        let ok = match type_name {
            wk::STRING => arg_value.is_string(),
            wk::INTEGER => arg_value.is_i64(),
            wk::FLOAT => arg_value.is_number(),
            wk::BOOLEAN => arg_value.is_boolean(),
            _ => {
                // Resolve to a chain Resource.
                let referent = match self.layer.resolve(&type_iri) {
                    Some(r) => r,
                    None => return, // Treat as unbound parameter; deferred.
                };
                // InductiveType: recurse.
                if referent.is_instance_of(&iri(wk::INDUCTIVE_TYPE)) {
                    self.walk_inductive_value(
                        &Value::Json(arg_value.clone()),
                        &referent,
                        path,
                        res_id,
                        out,
                    );
                    return;
                }
                // Class: arg is an embedded resource ref or IRI string —
                // structural shape only; deeper class-type checking
                // would duplicate `check_class_types` and is deferred.
                // For v1, accept any string (IRI ref) or object (embedded).
                if referent.is_instance_of(&iri(wk::CLASS)) {
                    arg_value.is_string() || arg_value.is_object()
                } else {
                    // Unknown referent kind — skip silently.
                    true
                }
            }
        };

        if !ok {
            out.push(ValidationError {
                resource_id: res_id.clone(),
                property: None,
                rule: ValidationRule::InductiveValueMismatch,
                message: format!("{path}: value does not match declared `type_name` `{type_name}`"),
            });
        }
    }

    /// Rule 17: FormulaTerm App-spine arity check (D32 §5.4).
    ///
    /// When a property's value is a FormulaTerm whose outer ctor is
    /// `App`, walk the left spine to find the head. If the head is an
    /// `OpRef(iri)` whose target resolves to an `Operator` resource
    /// with a declared `operator_arity`, confirm the App spine
    /// supplies exactly that many arguments. This catches typos like
    /// `App(OpRef("add"), x)` (one arg short) at commit time rather
    /// than at dispatch.
    ///
    /// Type-of-each-arg checking against the operator's full
    /// `operator_signature` (a Pi chain over FormulaTerm) is a
    /// follow-on landing — v1 ships arity-only.
    fn check_formula_term_arity(
        &self,
        prop_def: &Resource,
        value: &Value,
        prop_iri: &Iri,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        // Only fire on properties whose value is a FormulaTerm.
        let dt = match self.get_data_type_str(prop_def) {
            Some(dt) => dt,
            None => return vec![],
        };
        if dt != wk::INDUCTIVE {
            return vec![];
        }
        let allowed = match prop_def.get(&iri(wk::CLASS_TYPES)) {
            Some(val) => val.as_iri_array(),
            None => return vec![],
        };
        if allowed.len() != 1 {
            return vec![];
        }
        if allowed[0].as_str() != FORMULA_TERM_IRI {
            return vec![];
        }

        let json = match value {
            Value::Json(j) => j,
            _ => return vec![],
        };

        let mut errors = Vec::new();
        self.walk_formula_term_app_arity(json, prop_iri.as_str().to_string(), res_id, &mut errors);
        errors
    }

    /// Walk a FormulaTerm value tree. When entering an `App` node,
    /// resolve the *whole* left spine to its head + spine args and
    /// arity-check once. Then recurse only into the spine args
    /// (each of which may itself be a sub-tree carrying nested
    /// applications). The intermediate `App` nodes inside the spine
    /// are *not* re-checked — they're partial applications, not
    /// complete operator invocations.
    ///
    /// For non-App nodes, recurse into every arg so nested
    /// applications buried in `Lam(_, ty, body)` etc. still get
    /// checked.
    fn walk_formula_term_app_arity(
        &self,
        node: &serde_json::Value,
        path: String,
        res_id: &Option<Iri>,
        out: &mut Vec<ValidationError>,
    ) {
        let obj = match node.as_object() {
            Some(o) => o,
            None => return,
        };
        let ctor = obj
            .get("ctor")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        if ctor == "App" {
            let (head, spine_args) = collect_app_spine(node);
            if let Some(head_obj) = head.as_object() {
                let head_ctor = head_obj
                    .get("ctor")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                if head_ctor == "OpRef" {
                    let op_iri_s = head_obj
                        .get("args")
                        .and_then(serde_json::Value::as_array)
                        .and_then(|a| a.first())
                        .and_then(serde_json::Value::as_str);
                    if let Some(op_iri_s) = op_iri_s {
                        if let Ok(op_iri) = Iri::parse(op_iri_s) {
                            if let Some(op_resource) = self.layer.resolve(&op_iri) {
                                if let Some(arity_value) = op_resource.get(&iri(OPERATOR_ARITY_IRI))
                                {
                                    if let Some(arity) = arity_value.as_integer() {
                                        if (arity as usize) != spine_args.len() {
                                            out.push(ValidationError {
                                                resource_id: res_id.clone(),
                                                property: None,
                                                rule: ValidationRule::OperatorArityMismatch,
                                                message: format!(
                                                    "{path}: operator `{op_iri_s}` declares arity {arity}; App spine supplies {} arg(s)",
                                                    spine_args.len(),
                                                ),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // Recurse only into spine args (NOT into intermediate
            // App heads — those are partial applications counted by
            // the spine, not separate invocations to arity-check).
            // `collect_app_spine` returns args right-to-left from
            // the deepest App; reverse so paths read left-to-right.
            let spine_left_to_right: Vec<&serde_json::Value> = spine_args.iter().rev().collect();
            for (i, arg) in spine_left_to_right.iter().enumerate() {
                let child_path = format!("{path}.args[{i}]");
                self.walk_formula_term_app_arity(arg, child_path, res_id, out);
            }
            return;
        }

        // Non-App node: recurse into every arg so nested applications
        // inside Lam/Pi bodies, OpRef IRIs (no recursion needed —
        // they're string args), etc. get checked too.
        let args = obj
            .get("args")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        for (i, arg) in args.iter().enumerate() {
            let child_path = format!("{path}.args[{i}]");
            self.walk_formula_term_app_arity(arg, child_path, res_id, out);
        }
    }

    /// For Class resources, every IRI in `requires` / `recommends` must
    /// resolve to a `core:Property` and every IRI in `subclass_of` must
    /// resolve to a `core:Class`. For Property resources, every IRI in
    /// `class_types` must resolve to a `core:Class` and `data_type` (if
    /// present) must resolve to a `core:DataType`.
    ///
    /// Without this, a typo in `requires patent:innovation_category`
    /// (vs `invention_category`) commits cleanly and only fails much
    /// later at instance validation or program execution time, far
    /// from the offending declaration.
    fn check_class_definition_references(
        &self,
        resource: &Resource,
        res_id: &Option<Iri>,
    ) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        let is_class = resource.is_instance_of(&iri(wk::CLASS));
        let is_property = resource.is_instance_of(&iri(wk::PROPERTY));
        if !is_class && !is_property {
            return errors;
        }

        if is_class {
            self.check_array_refs(
                resource,
                ReferenceCheck::REQUIRES_PROPERTY,
                res_id,
                &mut errors,
            );
            self.check_array_refs(
                resource,
                ReferenceCheck::RECOMMENDS_PROPERTY,
                res_id,
                &mut errors,
            );
            self.check_array_refs(
                resource,
                ReferenceCheck::SUBCLASS_OF_CLASS,
                res_id,
                &mut errors,
            );
        }

        if is_property {
            // class_types accepts BOTH Class and InductiveType IRIs
            // (D32 §3.5): `data_type: core:resource(_array)` properties
            // reference Classes; `data_type: core:inductive` properties
            // reference InductiveTypes. Walk the array and accept
            // either kind.
            if let Some(value) = resource.get(&iri(wk::CLASS_TYPES)) {
                for target in value.as_iri_array() {
                    match self.layer.resolve(&target) {
                        Some(t)
                            if t.is_instance_of(&iri(wk::CLASS))
                                || t.is_instance_of(&iri(wk::INDUCTIVE_TYPE)) => {}
                        Some(_) => errors.push(ValidationError {
                            resource_id: res_id.clone(),
                            property: Some(iri(wk::CLASS_TYPES)),
                            rule: ValidationRule::UnresolvedClassReference,
                            message: format!(
                                "class_types: '{target}' resolves to a resource that is not an instance of core:Class or core:InductiveType"
                            ),
                        }),
                        None => errors.push(ValidationError {
                            resource_id: res_id.clone(),
                            property: Some(iri(wk::CLASS_TYPES)),
                            rule: ValidationRule::UnresolvedClassReference,
                            message: format!(
                                "class_types: '{target}' does not resolve to any resource in the layer chain"
                            ),
                        }),
                    }
                }
            }
            // `data_type` is a single resource ref (not an array).
            if let Some(value) = resource.get(&iri(wk::DATA_TYPE_PROP)) {
                if let Some(target) = value_as_iri(value) {
                    self.check_resolves_to(&target, ReferenceCheck::DATA_TYPE, res_id, &mut errors);
                }
            }
        }
        errors
    }

    /// Rule 15: Comorphism well-formedness (D14 §4.5 / §5).
    ///
    /// For a Comorphism resource, the kernel checks that:
    ///
    /// - `export_format` resolves to a resource of class `ExportFormat`,
    /// - `import_format` resolves to a resource of class `ImportFormat`,
    /// - `transformation` resolves to *some* resource in the chain
    ///   (the full Mini-TT signature-equality check between the
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

    /// Walk an array-valued reference field on `resource` and verify
    /// every element resolves to a resource of the expected class.
    fn check_array_refs(
        &self,
        resource: &Resource,
        check: ReferenceCheck<'static>,
        res_id: &Option<Iri>,
        errors: &mut Vec<ValidationError>,
    ) {
        let Some(value) = resource.get(&iri(check.field_iri)) else {
            return;
        };
        for target in value.as_iri_array() {
            self.check_resolves_to(&target, check, res_id, errors);
        }
    }

    /// Verify a single referenced IRI resolves to a resource of the
    /// expected class. Reports unresolved or wrong-kind references
    /// against `ValidationRule::UnresolvedClassReference`.
    fn check_resolves_to(
        &self,
        target: &Iri,
        check: ReferenceCheck<'_>,
        res_id: &Option<Iri>,
        errors: &mut Vec<ValidationError>,
    ) {
        let expected_class = iri(check.expected_class_iri);
        match self.layer.resolve(target) {
            Some(target_resource) if target_resource.is_instance_of(&expected_class) => {}
            Some(_) => errors.push(ValidationError {
                resource_id: res_id.clone(),
                property: Some(iri(check.field_iri)),
                rule: ValidationRule::UnresolvedClassReference,
                message: format!(
                    "{}: '{target}' resolves to a resource that is not an instance of {}",
                    check.field_label, check.expected_class_label,
                ),
            }),
            None => errors.push(ValidationError {
                resource_id: res_id.clone(),
                property: Some(iri(check.field_iri)),
                rule: ValidationRule::UnresolvedClassReference,
                message: format!(
                    "{}: '{target}' does not resolve to any resource in the layer chain",
                    check.field_label,
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
                    message: format!("standalone Lambda body did not parse as Mini-TT: {reason}"),
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

// --- Format validation helpers ---

fn is_valid_date(s: &str) -> bool {
    let re = regex::Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap();
    if !re.is_match(s) {
        return false;
    }
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 {
        return false;
    }
    let year: u32 = parts[0].parse().unwrap_or(0);
    let month: u32 = parts[1].parse().unwrap_or(0);
    let day: u32 = parts[2].parse().unwrap_or(0);
    (1..=9999).contains(&year) && (1..=12).contains(&month) && (1..=31).contains(&day)
}

fn is_valid_datetime(s: &str) -> bool {
    // Accept ISO 8601 with timezone: YYYY-MM-DDTHH:MM:SSZ or +HH:MM
    let re = regex::Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$")
        .unwrap();
    re.is_match(s)
}

fn is_valid_time(s: &str) -> bool {
    let re = regex::Regex::new(r"^\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})?$").unwrap();
    re.is_match(s)
}

fn is_valid_uuid(s: &str) -> bool {
    let re = regex::Regex::new(
        r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$",
    )
    .unwrap();
    re.is_match(s)
}

/// Helper: parse a well-known constant into an Iri.
fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("well-known IRI constants must be valid")
}

/// Helper: extract a single resource-IRI from a Value. Accepts both
/// `Value::ResourceRef` (canonical) and `Value::String` (the JSON
/// parser stores all strings as `Value::String` — `data_type` is
/// frequently authored as a bare string in source ontologies).
fn value_as_iri(value: &Value) -> Option<Iri> {
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
fn format_is_a_list(classes: Vec<Iri>) -> String {
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
fn format_iri_refs(refs: &[&Iri]) -> String {
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

/// Bundle of "what we're checking" for the class-definition reference
/// validation pass (rule 14, eigenius#26). One value per field/expected-
/// class pair; the constants below cover the five sites the validator
/// inspects.
#[derive(Copy, Clone)]
struct ReferenceCheck<'a> {
    /// Field IRI on the source resource (e.g. `core:requires`).
    field_iri: &'a str,
    /// Human label for the field used in error messages (e.g. `requires`).
    field_label: &'a str,
    /// Class IRI the referenced resource must be an instance of
    /// (e.g. `core:Property`).
    expected_class_iri: &'a str,
    /// Human label for the expected class (e.g. `core:Property`).
    expected_class_label: &'a str,
}

impl ReferenceCheck<'static> {
    const REQUIRES_PROPERTY: Self = Self {
        field_iri: wk::REQUIRES,
        field_label: "requires",
        expected_class_iri: wk::PROPERTY,
        expected_class_label: "core:Property",
    };
    const RECOMMENDS_PROPERTY: Self = Self {
        field_iri: wk::RECOMMENDS,
        field_label: "recommends",
        expected_class_iri: wk::PROPERTY,
        expected_class_label: "core:Property",
    };
    const SUBCLASS_OF_CLASS: Self = Self {
        field_iri: wk::PARENT_CLASSES,
        field_label: "subclass_of",
        expected_class_iri: wk::CLASS,
        expected_class_label: "core:Class",
    };
    // class_types accepts both Class and InductiveType references
    // (D32 §3.5); the check is open-coded in
    // `check_class_definition_references` because `ReferenceCheck` is
    // single-class-only.
    const DATA_TYPE: Self = Self {
        field_iri: wk::DATA_TYPE_PROP,
        field_label: "data_type",
        expected_class_iri: wk::DATA_TYPE,
        expected_class_label: "core:DataType",
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::LayerBuilder;
    use crate::ontology::eigon_json;
    use std::sync::Arc;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn make_resource(id: &str, props: Vec<(&str, Value)>) -> Resource {
        let mut r = Resource::new(iri(id));
        for (k, v) in props {
            r.set(iri(k), v);
        }
        r
    }

    fn build_core_layer() -> Arc<Layer> {
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
    fn type_mismatch() {
        let core = build_core_layer();
        let mut builder = LayerBuilder::new("test", Some(core));
        // description should be a string, give it an integer
        builder
            .add_resource(make_resource(
                "urn:eigenius:test:bad_type",
                vec![
                    (
                        wk::IS_A,
                        Value::Array(vec![Value::String(wk::CLASS.to_string())]),
                    ),
                    (wk::DESCRIPTION, Value::Integer(42)), // Wrong type!
                    (wk::SHORT_NAME, Value::String("bad".into())),
                ],
            ))
            .unwrap();
        let layer = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));

        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        assert!(
            errors
                .iter()
                .any(|e| e.rule == ValidationRule::TypeMismatch),
            "expected TypeMismatch error; got: {errors:?}"
        );
    }

    #[test]
    fn allows_only_violation() {
        let core = build_core_layer();
        let mut builder = LayerBuilder::new("test", Some(core));
        // data_type has allows_only constraint — use an invalid value
        builder
            .add_resource(make_resource(
                "urn:eigenius:test:bad_dt",
                vec![
                    (
                        wk::IS_A,
                        Value::Array(vec![Value::String(wk::PROPERTY.to_string())]),
                    ),
                    (wk::DESCRIPTION, Value::String("bad data_type".into())),
                    (wk::SHORT_NAME, Value::String("bad_dt".into())),
                    (
                        wk::DATA_TYPE_PROP,
                        Value::String("urn:eigenius:core:nonexistent".to_string()),
                    ),
                ],
            ))
            .unwrap();
        let layer = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));

        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        assert!(errors
            .iter()
            .any(|e| e.rule == ValidationRule::AllowedValueViolation));
    }

    #[test]
    fn domain_violation() {
        let core = build_core_layer();
        let mut builder = LayerBuilder::new("test", Some(core));
        // 'requires' has domain [Class], but we put it on a non-Class resource
        builder
            .add_resource(make_resource(
                "urn:eigenius:test:not_a_class",
                vec![
                    (
                        wk::IS_A,
                        Value::Array(vec![Value::String(wk::PROPERTY.to_string())]),
                    ),
                    (wk::DESCRIPTION, Value::String("I'm a property".into())),
                    (wk::SHORT_NAME, Value::String("not_a_class".into())),
                    (wk::DATA_TYPE_PROP, Value::String(wk::STRING.to_string())),
                    // 'requires' is Class-only, but this is a Property
                    (
                        wk::REQUIRES,
                        Value::Array(vec![Value::String("urn:eigenius:test:foo".to_string())]),
                    ),
                ],
            ))
            .unwrap();
        let layer = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));

        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        assert!(errors
            .iter()
            .any(|e| e.rule == ValidationRule::DomainViolation));
    }

    #[test]
    fn inheritance_requires() {
        let core = build_core_layer();
        let mut builder = LayerBuilder::new("test", Some(core));

        // Define Animal class requiring 'name'
        builder
            .add_resource(make_resource(
                "urn:eigenius:test:Animal",
                vec![
                    (
                        wk::IS_A,
                        Value::Array(vec![Value::String(wk::CLASS.to_string())]),
                    ),
                    (wk::DESCRIPTION, Value::String("An animal".into())),
                    (wk::SHORT_NAME, Value::String("Animal".into())),
                    (
                        wk::REQUIRES,
                        Value::Array(vec![Value::String("urn:eigenius:test:name".to_string())]),
                    ),
                ],
            ))
            .unwrap();

        // Define Dog class extending Animal, requiring 'breed'
        builder
            .add_resource(make_resource(
                "urn:eigenius:test:Dog",
                vec![
                    (
                        wk::IS_A,
                        Value::Array(vec![Value::String(wk::CLASS.to_string())]),
                    ),
                    (wk::DESCRIPTION, Value::String("A dog".into())),
                    (wk::SHORT_NAME, Value::String("Dog".into())),
                    (
                        wk::PARENT_CLASSES,
                        Value::Array(vec![Value::String("urn:eigenius:test:Animal".to_string())]),
                    ),
                    (
                        wk::REQUIRES,
                        Value::Array(vec![Value::String("urn:eigenius:test:breed".to_string())]),
                    ),
                ],
            ))
            .unwrap();

        // Define the properties
        builder
            .add_resource(make_resource(
                "urn:eigenius:test:name",
                vec![
                    (
                        wk::IS_A,
                        Value::Array(vec![Value::String(wk::PROPERTY.to_string())]),
                    ),
                    (wk::DESCRIPTION, Value::String("Name".into())),
                    (wk::SHORT_NAME, Value::String("name".into())),
                    (wk::DATA_TYPE_PROP, Value::String(wk::STRING.to_string())),
                ],
            ))
            .unwrap();

        builder
            .add_resource(make_resource(
                "urn:eigenius:test:breed",
                vec![
                    (
                        wk::IS_A,
                        Value::Array(vec![Value::String(wk::PROPERTY.to_string())]),
                    ),
                    (wk::DESCRIPTION, Value::String("Breed".into())),
                    (wk::SHORT_NAME, Value::String("breed".into())),
                    (wk::DATA_TYPE_PROP, Value::String(wk::STRING.to_string())),
                ],
            ))
            .unwrap();

        // Create a Dog instance missing 'name' (inherited from Animal)
        builder
            .add_resource(make_resource(
                "urn:eigenius:test:rex",
                vec![
                    (
                        wk::IS_A,
                        Value::Array(vec![Value::String("urn:eigenius:test:Dog".to_string())]),
                    ),
                    (
                        "urn:eigenius:test:breed",
                        Value::String("German Shepherd".into()),
                    ),
                    // Missing 'name'!
                ],
            ))
            .unwrap();

        let layer = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));
        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();

        // Should have a MissingRequired for 'name' on rex
        let rex_errors: Vec<_> = errors
            .iter()
            .filter(|e| {
                e.resource_id.as_ref().map(|i| i.as_str()) == Some("urn:eigenius:test:rex")
                    && e.rule == ValidationRule::MissingRequired
            })
            .collect();
        assert!(
            !rex_errors.is_empty(),
            "Dog instance missing inherited 'name' should fail validation"
        );
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

    #[test]
    fn format_date_validation() {
        assert!(is_valid_date("2026-04-11"));
        assert!(!is_valid_date("2026-13-01"));
        assert!(!is_valid_date("not-a-date"));
    }

    #[test]
    fn format_datetime_validation() {
        assert!(is_valid_datetime("2026-04-11T14:30:00Z"));
        assert!(is_valid_datetime("2026-04-11T14:30:00+05:30"));
        assert!(!is_valid_datetime("2026-04-11"));
    }

    #[test]
    fn format_uuid_validation() {
        assert!(is_valid_uuid("550e8400-e29b-41d4-a716-446655440000"));
        assert!(!is_valid_uuid("not-a-uuid"));
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

    // --- eigenius#26: class-definition reference integrity ---

    /// A class that `requires` an IRI with no matching Property
    /// declaration anywhere in the chain must fail validation rather
    /// than commit cleanly.
    #[test]
    fn class_requires_unresolved_property_is_rejected() {
        let core = build_core_layer();
        let mut top = LayerBuilder::new("test", Some(core));

        // Class declaring a requires reference to a property that
        // doesn't exist (typo / forgotten-declaration scenario).
        let bad_class = make_resource(
            "urn:eigenius:test:Foo",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(wk::CLASS.into())]),
                ),
                (wk::SHORT_NAME, Value::String("Foo".into())),
                (wk::DESCRIPTION, Value::String("Test class.".into())),
                (
                    wk::REQUIRES,
                    Value::Array(vec![Value::String(
                        "urn:eigenius:test:totally_made_up_property".into(),
                    )]),
                ),
            ],
        );
        top.add_resource(bad_class).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let dangling: Vec<_> = errors
            .iter()
            .filter(|e| {
                e.rule == ValidationRule::UnresolvedClassReference
                    && e.message.contains("totally_made_up_property")
            })
            .collect();
        assert_eq!(
            dangling.len(),
            1,
            "expected exactly one UnresolvedClassReference for the missing property; got {errors:?}"
        );
    }

    /// Same class, but the referenced property is declared in the
    /// same load batch — must validate cleanly.
    #[test]
    fn class_requires_same_batch_property_is_accepted() {
        let core = build_core_layer();
        let mut top = LayerBuilder::new("test", Some(core));

        let prop = make_resource(
            "urn:eigenius:test:my_prop",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(wk::PROPERTY.into())]),
                ),
                (wk::SHORT_NAME, Value::String("my_prop".into())),
                (wk::DESCRIPTION, Value::String("A test property.".into())),
                (
                    wk::DATA_TYPE_PROP,
                    Value::String("urn:eigenius:core:string".into()),
                ),
            ],
        );
        let class = make_resource(
            "urn:eigenius:test:Foo",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(wk::CLASS.into())]),
                ),
                (wk::SHORT_NAME, Value::String("Foo".into())),
                (wk::DESCRIPTION, Value::String("Test class.".into())),
                (
                    wk::REQUIRES,
                    Value::Array(vec![Value::String("urn:eigenius:test:my_prop".into())]),
                ),
            ],
        );
        top.add_resource(prop).unwrap();
        top.add_resource(class).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let dangling: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::UnresolvedClassReference)
            .collect();
        assert!(
            dangling.is_empty(),
            "valid forward-reference should not surface UnresolvedClassReference; got {dangling:?}"
        );
    }

    /// A property whose `data_type` doesn't resolve must fail.
    #[test]
    fn property_data_type_unresolved_is_rejected() {
        let core = build_core_layer();
        let mut top = LayerBuilder::new("test", Some(core));

        let bad_prop = make_resource(
            "urn:eigenius:test:my_prop",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(wk::PROPERTY.into())]),
                ),
                (wk::SHORT_NAME, Value::String("my_prop".into())),
                (wk::DESCRIPTION, Value::String("Bad prop.".into())),
                (
                    wk::DATA_TYPE_PROP,
                    Value::String("urn:eigenius:test:not_a_real_type".into()),
                ),
            ],
        );
        top.add_resource(bad_prop).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let dangling: Vec<_> = errors
            .iter()
            .filter(|e| {
                e.rule == ValidationRule::UnresolvedClassReference
                    && e.message.contains("not_a_real_type")
            })
            .collect();
        assert_eq!(
            dangling.len(),
            1,
            "expected exactly one UnresolvedClassReference for the missing data_type; got {errors:?}"
        );
    }

    /// A property's `class_types` referencing a non-Class fails.
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

    #[test]
    fn property_class_types_pointing_at_non_class_is_rejected() {
        let core = build_core_layer();
        let mut top = LayerBuilder::new("test", Some(core));

        // A non-Class resource (just an instance of `core:Class`'s
        // base — actually use core:DataType, which is a Class but its
        // *instances* aren't classes themselves).
        let instance = make_resource(
            "urn:eigenius:test:not_a_class",
            vec![
                // is_a a DataType, NOT a Class.
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(wk::DATA_TYPE.into())]),
                ),
                (wk::SHORT_NAME, Value::String("not_a_class".into())),
                (wk::DESCRIPTION, Value::String("placeholder".into())),
            ],
        );
        let bad_prop = make_resource(
            "urn:eigenius:test:my_prop",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::String(wk::PROPERTY.into())]),
                ),
                (wk::SHORT_NAME, Value::String("my_prop".into())),
                (wk::DESCRIPTION, Value::String("Bad prop.".into())),
                (
                    wk::DATA_TYPE_PROP,
                    Value::String("urn:eigenius:core:resource".into()),
                ),
                (
                    wk::CLASS_TYPES,
                    Value::Array(vec![Value::String("urn:eigenius:test:not_a_class".into())]),
                ),
            ],
        );
        top.add_resource(instance).unwrap();
        top.add_resource(bad_prop).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let dangling: Vec<_> = errors
            .iter()
            .filter(|e| {
                e.rule == ValidationRule::UnresolvedClassReference
                    && e.message.contains("not_a_class")
            })
            .collect();
        assert_eq!(
            dangling.len(),
            1,
            "expected one UnresolvedClassReference for class_types pointing at a non-Class; got {errors:?}"
        );
    }

    // ──────────────────────────────────────────────────────────────────
    // Inductive value validation — D32 §3.5
    // ──────────────────────────────────────────────────────────────────

    /// Build a minimal `Nat = zero | succ(Nat)` ontology layer + a
    /// property `nat_value : core:inductive` with `class_types: [Nat]`,
    /// returning the chain layer ready to commit Nat values against.
    fn build_nat_layer() -> Arc<Layer> {
        let core = build_core_layer();
        let mut builder = LayerBuilder::new("test_nat", Some(core));

        // ctor `zero`: no args.
        let zero_ctor = make_resource(
            "urn:eigenius:test:Nat:zero",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(wk::INDUCTIVE_CTOR))]),
                ),
                (wk::CTOR_NAME, Value::String("zero".into())),
                (wk::ARG_TYPES, Value::Array(vec![])),
            ],
        );

        // ctor `succ(pred: Nat)`.
        let succ_arg = make_resource(
            "urn:eigenius:test:Nat:succ:pred",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(wk::INDUCTIVE_ARG_TYPE))]),
                ),
                (wk::ARG_NAME, Value::String("pred".into())),
                (wk::TYPE_NAME, Value::String("urn:eigenius:test:Nat".into())),
            ],
        );
        let succ_ctor = make_resource(
            "urn:eigenius:test:Nat:succ",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(wk::INDUCTIVE_CTOR))]),
                ),
                (wk::CTOR_NAME, Value::String("succ".into())),
                (
                    wk::ARG_TYPES,
                    Value::Array(vec![Value::Embedded(Box::new(succ_arg))]),
                ),
            ],
        );

        let nat = make_resource(
            "urn:eigenius:test:Nat",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(wk::INDUCTIVE_TYPE))]),
                ),
                (wk::SHORT_NAME, Value::String("Nat".into())),
                (
                    wk::CTORS,
                    Value::Array(vec![
                        Value::Embedded(Box::new(zero_ctor)),
                        Value::Embedded(Box::new(succ_ctor)),
                    ]),
                ),
            ],
        );

        // Property `nat_value : core:inductive` typed at Nat.
        let nat_value_prop = make_resource(
            "urn:eigenius:test:nat_value",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
                ),
                (wk::SHORT_NAME, Value::String("nat_value".into())),
                (wk::DATA_TYPE_PROP, Value::ResourceRef(iri(wk::INDUCTIVE))),
                (
                    wk::CLASS_TYPES,
                    Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:test:Nat"))]),
                ),
            ],
        );

        builder.add_resource(nat).unwrap();
        builder.add_resource(nat_value_prop).unwrap();
        Arc::new(builder.build(crate::layer::LayerStorage::in_memory()))
    }

    /// `succ(succ(zero))` as a JSON tagged-dict tree.
    fn nat_succ_succ_zero() -> serde_json::Value {
        serde_json::json!({
            "ctor": "succ",
            "args": [{
                "ctor": "succ",
                "args": [{
                    "ctor": "zero",
                    "args": []
                }]
            }]
        })
    }

    #[test]
    fn inductive_value_validates_succ_succ_zero() {
        let nat_layer = build_nat_layer();
        let mut top = LayerBuilder::new("test_top", Some(nat_layer));

        let holder = make_resource(
            "urn:eigenius:test:n2",
            vec![(
                "urn:eigenius:test:nat_value",
                Value::Json(nat_succ_succ_zero()),
            )],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let validator = Validator::new(Arc::clone(&layer));
        let errors = validator.validate();
        let inductive_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::InductiveValueMismatch)
            .collect();
        assert!(
            inductive_errors.is_empty(),
            "expected no InductiveValueMismatch on succ(succ(zero)); got {errors:?}"
        );
    }

    #[test]
    fn inductive_value_rejects_unknown_ctor() {
        let nat_layer = build_nat_layer();
        let mut top = LayerBuilder::new("test_top", Some(nat_layer));

        let bad = make_resource(
            "urn:eigenius:test:bad",
            vec![(
                "urn:eigenius:test:nat_value",
                Value::Json(serde_json::json!({
                    "ctor": "infinity",
                    "args": []
                })),
            )],
        );
        top.add_resource(bad).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        let mismatches: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::InductiveValueMismatch)
            .collect();
        assert_eq!(
            mismatches.len(),
            1,
            "expected exactly one InductiveValueMismatch for unknown ctor; got {errors:?}"
        );
        assert!(
            mismatches[0].message.contains("infinity"),
            "error must mention the offending ctor name: {}",
            mismatches[0].message
        );
    }

    #[test]
    fn inductive_value_rejects_arity_mismatch() {
        let nat_layer = build_nat_layer();
        let mut top = LayerBuilder::new("test_top", Some(nat_layer));

        // succ takes one arg; supply zero.
        let bad = make_resource(
            "urn:eigenius:test:bad_arity",
            vec![(
                "urn:eigenius:test:nat_value",
                Value::Json(serde_json::json!({
                    "ctor": "succ",
                    "args": []
                })),
            )],
        );
        top.add_resource(bad).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        let mismatches: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::InductiveValueMismatch)
            .collect();
        assert_eq!(
            mismatches.len(),
            1,
            "expected one InductiveValueMismatch for arity mismatch; got {errors:?}"
        );
        assert!(
            mismatches[0].message.contains("expects 1 arg"),
            "error must describe the arity mismatch: {}",
            mismatches[0].message
        );
    }

    // ──────────────────────────────────────────────────────────────────
    // FormulaTerm App-spine arity check — D32 §5.4 / Phase 19d.0.d
    // ──────────────────────────────────────────────────────────────────

    /// Build a layer chain rooted at the embedded core+formulas
    /// ontologies plus a property `formula_value : core:inductive`
    /// typed at FormulaTerm. Used by the arity-check tests.
    fn build_formula_layer() -> Arc<Layer> {
        // Reuse bootstrap so the formulas: layer (with FormulaTerm +
        // operator catalog) sits in the chain.
        let ctx = crate::bootstrap::bootstrap().expect("bootstrap with formulas layer");
        let formulas = Arc::clone(ctx.head().parent().expect("notebook has parent"));

        let mut builder = LayerBuilder::new("test_formula", Some(formulas));
        let prop = make_resource(
            "urn:eigenius:test:formula_value",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
                ),
                (wk::SHORT_NAME, Value::String("formula_value".into())),
                (wk::DATA_TYPE_PROP, Value::ResourceRef(iri(wk::INDUCTIVE))),
                (
                    wk::CLASS_TYPES,
                    Value::Array(vec![Value::ResourceRef(iri(
                        "urn:eigenius:formulas:FormulaTerm",
                    ))]),
                ),
            ],
        );
        builder.add_resource(prop).unwrap();
        Arc::new(builder.build(crate::layer::LayerStorage::in_memory()))
    }

    /// `App(App(OpRef("formulas:ops:add"), Var("x")), LitFloat(2.0))`
    /// — well-formed binary `add` invocation.
    fn add_x_2() -> serde_json::Value {
        serde_json::json!({
            "ctor": "App",
            "args": [
                {
                    "ctor": "App",
                    "args": [
                        {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:add"]},
                        {"ctor": "Var", "args": ["x"]}
                    ]
                },
                {"ctor": "LitFloat", "args": [2.0]}
            ]
        })
    }

    #[test]
    fn formula_term_well_formed_app_validates() {
        let layer = build_formula_layer();
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let holder = make_resource(
            "urn:eigenius:test:f1",
            vec![("urn:eigenius:test:formula_value", Value::Json(add_x_2()))],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        let arity_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::OperatorArityMismatch)
            .collect();
        assert!(
            arity_errors.is_empty(),
            "well-formed `add(x, 2)` must not raise OperatorArityMismatch; got {arity_errors:?}"
        );
    }

    #[test]
    fn formula_term_app_rejects_arity_short() {
        // `App(OpRef("add"), x)` — missing the second add argument.
        let underapplied = serde_json::json!({
            "ctor": "App",
            "args": [
                {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:add"]},
                {"ctor": "Var", "args": ["x"]}
            ]
        });

        let layer = build_formula_layer();
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let holder = make_resource(
            "urn:eigenius:test:bad_arity",
            vec![("urn:eigenius:test:formula_value", Value::Json(underapplied))],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        let arity_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::OperatorArityMismatch)
            .collect();
        assert_eq!(
            arity_errors.len(),
            1,
            "expected one OperatorArityMismatch for under-applied `add`; got {errors:?}"
        );
        assert!(
            arity_errors[0].message.contains("formulas:ops:add"),
            "error must name the offending operator: {}",
            arity_errors[0].message
        );
        assert!(
            arity_errors[0].message.contains("arity 2"),
            "error must mention the declared arity: {}",
            arity_errors[0].message
        );
    }

    #[test]
    fn formula_term_app_rejects_arity_long() {
        // `App(App(App(OpRef("neg"), x), y), z)` — `neg` is unary;
        // the spine supplies three args.
        let overapplied = serde_json::json!({
            "ctor": "App",
            "args": [
                {
                    "ctor": "App",
                    "args": [
                        {
                            "ctor": "App",
                            "args": [
                                {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:neg"]},
                                {"ctor": "Var", "args": ["x"]}
                            ]
                        },
                        {"ctor": "Var", "args": ["y"]}
                    ]
                },
                {"ctor": "Var", "args": ["z"]}
            ]
        });

        let layer = build_formula_layer();
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let holder = make_resource(
            "urn:eigenius:test:bad_arity_long",
            vec![("urn:eigenius:test:formula_value", Value::Json(overapplied))],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        let arity_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::OperatorArityMismatch)
            .collect();
        assert!(
            !arity_errors.is_empty(),
            "expected an OperatorArityMismatch for over-applied `neg`; got {errors:?}"
        );
    }

    // ──────────────────────────────────────────────────────────────────
    // Phase 20a.2 — D40 chain-mirrored Lean expressions
    // ──────────────────────────────────────────────────────────────────

    /// Build a layer chain rooted at the embedded bootstrap (which now
    /// carries `lean:LeanExpr` + siblings per D40) plus a property
    /// `proposition_value : core:inductive` typed at `lean:LeanExpr`.
    /// The chain looks like: core → program → reflection → institution
    /// → runtime → formulas → lean-expressions → <this test layer>.
    fn build_lean_expr_layer() -> Arc<Layer> {
        let ctx = crate::bootstrap::bootstrap().expect("bootstrap with lean-expressions layer");
        // After Phase 20a.4 the chain is:
        //   notebook → lean-institution → lean-expressions → formulas → …
        // We anchor at lean-institution (`ctx.head().parent()`) so the
        // test layer has both the lean:LeanExpr InductiveTypes
        // (resolved through `lean-expressions`) and the
        // institution-side classes (LeanProofTerm etc.) reachable —
        // notebook would also work, but anchoring above it keeps the
        // chain focused.
        let lean_layer = Arc::clone(
            ctx.head()
                .parent()
                .expect("head has lean-institution parent"),
        );

        let mut builder = LayerBuilder::new("test_lean_expr", Some(lean_layer));
        let prop = make_resource(
            "urn:eigenius:test:proposition_value",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
                ),
                (wk::SHORT_NAME, Value::String("proposition_value".into())),
                (wk::DATA_TYPE_PROP, Value::ResourceRef(iri(wk::INDUCTIVE))),
                (
                    wk::CLASS_TYPES,
                    Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:lean:LeanExpr"))]),
                ),
            ],
        );
        builder.add_resource(prop).unwrap();
        Arc::new(builder.build(crate::layer::LayerStorage::in_memory()))
    }

    /// `Lambda { binder_name = Str(Anon, "x"), binder_style = "default",
    ///           binder_type = Const(Str(Anon, "Nat"), Nil),
    ///           body = Var(0) }`
    /// ≈ `λ x : Nat, x` — the smallest non-trivial closed Lean term.
    fn lambda_x_in_nat() -> serde_json::Value {
        let anon = serde_json::json!({"ctor": "Anon"});
        let name_x = serde_json::json!({
            "ctor": "Str",
            "args": [anon.clone(), "x"]
        });
        let name_nat = serde_json::json!({
            "ctor": "Str",
            "args": [anon.clone(), "Nat"]
        });
        let nil = serde_json::json!({"ctor": "Nil"});
        serde_json::json!({
            "ctor": "Lambda",
            "args": [
                name_x,
                "default",
                {
                    "ctor": "Const",
                    "args": [name_nat, nil]
                },
                {"ctor": "Var", "args": [0]}
            ]
        })
    }

    #[test]
    fn lean_expr_lambda_x_in_nat_validates() {
        // Phase 20a.2 acceptance test: a hand-encoded `λ x : Nat, x`
        // value commits cleanly against the chain-mirrored LeanExpr
        // ontology — the chain-side type-check walks the tagged-dict
        // shape, dispatches on each ctor name, and recurses into
        // child arguments through LeanName / LeanLevelList / LeanExpr.
        // No validator errors means the inductive-value walker
        // successfully resolved every cross-reference and the
        // ontology layer is structurally consistent.
        let layer = build_lean_expr_layer();
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let holder = make_resource(
            "urn:eigenius:test:p1",
            vec![(
                "urn:eigenius:test:proposition_value",
                Value::Json(lambda_x_in_nat()),
            )],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        let inductive_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::InductiveValueMismatch)
            .collect();
        assert!(
            inductive_errors.is_empty(),
            "well-formed `λ x : Nat, x` must validate as a lean:LeanExpr; got {inductive_errors:?}"
        );
    }

    #[test]
    fn lean_expr_unknown_ctor_rejected() {
        // A value with a ctor name the LeanExpr inductive doesn't
        // declare must surface as `InductiveValueMismatch` — exercises
        // the per-ctor lookup in `walk_inductive_value`.
        let bogus = serde_json::json!({
            "ctor": "MetaVar",
            "args": [42]
        });
        let layer = build_lean_expr_layer();
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let holder = make_resource(
            "urn:eigenius:test:bad_ctor",
            vec![("urn:eigenius:test:proposition_value", Value::Json(bogus))],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        let inductive_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::InductiveValueMismatch)
            .collect();
        assert!(
            !inductive_errors.is_empty(),
            "unknown ctor `MetaVar` must trigger an InductiveValueMismatch; got {errors:?}"
        );
    }

    #[test]
    fn lean_expr_resolves_lean_layer_inductives() {
        // Sanity check: after bootstrap, every LeanExpr-related
        // InductiveType is reachable from the head as
        // `is_instance_of(core:InductiveType)`. Catches typos in the
        // ontology JSON or missing entries in the layer chain.
        let ctx = crate::bootstrap::bootstrap().expect("bootstrap");
        let head = Arc::clone(ctx.head());
        for ind_iri in &[
            "urn:eigenius:lean:LeanName",
            "urn:eigenius:lean:LeanLevel",
            "urn:eigenius:lean:LeanLevelList",
            "urn:eigenius:lean:LeanExpr",
        ] {
            let parsed = iri(ind_iri);
            let resolved = head.resolve(&parsed).unwrap_or_else(|| {
                panic!("`{ind_iri}` should resolve from the bootstrap chain head")
            });
            assert!(
                resolved.is_instance_of(&iri(wk::INDUCTIVE_TYPE)),
                "`{ind_iri}` should be an InductiveType"
            );
        }
    }

    /// Build a layer with a property `proposition_value` whose
    /// `data_type` is the caller-supplied IRI and whose `class_types`
    /// references `lean:LeanExpr`. Powers the Option A tests that
    /// exercise `core:resource` / `core:resource_array` carrying
    /// inductive values without going through `core:inductive`.
    fn build_lean_expr_property_layer(data_type_iri: &str) -> Arc<Layer> {
        let ctx = crate::bootstrap::bootstrap().expect("bootstrap with lean-expressions layer");
        let lean_layer = Arc::clone(
            ctx.head()
                .parent()
                .expect("head has lean-institution parent"),
        );

        let mut builder = LayerBuilder::new("test_lean_expr_resource", Some(lean_layer));
        let prop = make_resource(
            "urn:eigenius:test:proposition_value",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
                ),
                (wk::SHORT_NAME, Value::String("proposition_value".into())),
                (wk::DATA_TYPE_PROP, Value::ResourceRef(iri(data_type_iri))),
                (
                    wk::CLASS_TYPES,
                    Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:lean:LeanExpr"))]),
                ),
            ],
        );
        builder.add_resource(prop).unwrap();
        Arc::new(builder.build(crate::layer::LayerStorage::in_memory()))
    }

    /// Option A: `data_type: core:resource` with an `InductiveType`
    /// `class_types` accepts a single `Value::Json` carrying the
    /// inductive value, and the validator walks it the same way as
    /// `data_type: core:inductive`.
    #[test]
    fn option_a_resource_with_inductive_class_types_accepts_json() {
        let layer = build_lean_expr_property_layer(wk::RESOURCE);
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let holder = make_resource(
            "urn:eigenius:test:p_single",
            vec![(
                "urn:eigenius:test:proposition_value",
                Value::Json(lambda_x_in_nat()),
            )],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        assert!(
            errors.is_empty(),
            "`resource` + InductiveType class_types must accept a well-formed Json value; got {errors:?}"
        );
    }

    /// Option A: `data_type: core:resource_array` with an
    /// `InductiveType` `class_types` accepts an `Array` of
    /// `Value::Json` elements; each element is walked against the
    /// declared inductive.
    #[test]
    fn option_a_resource_array_with_inductive_class_types_accepts_json_array() {
        let layer = build_lean_expr_property_layer(wk::RESOURCE_ARRAY);
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let holder = make_resource(
            "urn:eigenius:test:p_array",
            vec![(
                "urn:eigenius:test:proposition_value",
                Value::Array(vec![
                    Value::Json(lambda_x_in_nat()),
                    Value::Json(lambda_x_in_nat()),
                ]),
            )],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        assert!(
            errors.is_empty(),
            "`resource_array` + InductiveType class_types must accept Array<Json>; got {errors:?}"
        );
    }

    /// Option A: malformed inductive value in a `resource_array`
    /// element surfaces as `InductiveValueMismatch` with a structured
    /// path indicating the bad index — the dispatch in
    /// `check_class_types` walks each Json element.
    #[test]
    fn option_a_resource_array_with_bad_ctor_rejects() {
        let layer = build_lean_expr_property_layer(wk::RESOURCE_ARRAY);
        let mut top = LayerBuilder::new("test_top", Some(layer));
        let bogus = serde_json::json!({"ctor": "DoesNotExist", "args": []});
        let holder = make_resource(
            "urn:eigenius:test:p_array_bad",
            vec![(
                "urn:eigenius:test:proposition_value",
                Value::Array(vec![Value::Json(lambda_x_in_nat()), Value::Json(bogus)]),
            )],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        let inductive_errors: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::InductiveValueMismatch)
            .collect();
        assert!(
            !inductive_errors.is_empty(),
            "bad ctor in array must surface as InductiveValueMismatch; got {errors:?}"
        );
        let saw_index_path = inductive_errors.iter().any(|e| e.message.contains("[1]"));
        assert!(
            saw_index_path,
            "error message should reference the failing array index `[1]`; got {inductive_errors:?}"
        );
    }

    /// Regression: a `resource_array` property with a Class
    /// `class_types` still rejects a `Value::Json` element at the
    /// wire-shape check — Option A only loosens the gate when
    /// `class_types` resolves to an `InductiveType`.
    #[test]
    fn option_a_resource_array_with_class_class_types_rejects_json() {
        let ctx = crate::bootstrap::bootstrap().expect("bootstrap");
        let head = Arc::clone(ctx.head());

        let mut builder = LayerBuilder::new("test_class_array", Some(head));
        // Declare a small Class so class_types resolves.
        let some_class = make_resource(
            "urn:eigenius:test:SomeClass",
            vec![(
                wk::IS_A,
                Value::Array(vec![Value::ResourceRef(iri(wk::CLASS))]),
            )],
        );
        let prop = make_resource(
            "urn:eigenius:test:class_array_prop",
            vec![
                (
                    wk::IS_A,
                    Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
                ),
                (wk::SHORT_NAME, Value::String("class_array_prop".into())),
                (
                    wk::DATA_TYPE_PROP,
                    Value::ResourceRef(iri(wk::RESOURCE_ARRAY)),
                ),
                (
                    wk::CLASS_TYPES,
                    Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:test:SomeClass"))]),
                ),
            ],
        );
        builder.add_resource(some_class).unwrap();
        builder.add_resource(prop).unwrap();
        let lay = Arc::new(builder.build(crate::layer::LayerStorage::in_memory()));

        let mut top = LayerBuilder::new("test_top", Some(lay));
        let holder = make_resource(
            "urn:eigenius:test:p_class",
            vec![(
                "urn:eigenius:test:class_array_prop",
                Value::Array(vec![Value::Json(serde_json::json!({"ctor": "Whatever"}))]),
            )],
        );
        top.add_resource(holder).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        let type_mismatches: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::TypeMismatch)
            .collect();
        assert!(
            !type_mismatches.is_empty(),
            "Class class_types must keep rejecting Json elements at the wire-shape gate; got {errors:?}"
        );
    }

    #[test]
    fn inductive_value_rejects_nested_arg_type_mismatch() {
        let nat_layer = build_nat_layer();
        let mut top = LayerBuilder::new("test_top", Some(nat_layer));

        // succ's arg should be a Nat; supply a JSON string.
        let bad = make_resource(
            "urn:eigenius:test:bad_nested",
            vec![(
                "urn:eigenius:test:nat_value",
                Value::Json(serde_json::json!({
                    "ctor": "succ",
                    "args": ["not_a_nat"]
                })),
            )],
        );
        top.add_resource(bad).unwrap();
        let layer = Arc::new(top.build(crate::layer::LayerStorage::in_memory()));

        let errors = Validator::new(Arc::clone(&layer)).validate();
        let mismatches: Vec<_> = errors
            .iter()
            .filter(|e| e.rule == ValidationRule::InductiveValueMismatch)
            .collect();
        assert!(
            !mismatches.is_empty(),
            "expected an InductiveValueMismatch for nested arg type mismatch; got {errors:?}"
        );
        // Path should mention args[0].
        let path_match = mismatches.iter().any(|e| e.message.contains("args[0]"));
        assert!(
            path_match,
            "error must include structured path `args[0]`: {mismatches:?}"
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
