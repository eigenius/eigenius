//! Validation engine for Eigon resources.
//!
//! Validates resources in a layer against definitions reachable through
//! the parent chain. Implements all validation rules from D1 §5.4.

use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use std::collections::BTreeSet;
use std::fmt;

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
pub struct Validator<'a> {
    layer: &'a Layer,
}

impl<'a> Validator<'a> {
    pub fn new(layer: &'a Layer) -> Self {
        Self { layer }
    }

    /// Validate all resources in this layer.
    pub fn validate(&self) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        for resource in self.layer.resources().values() {
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

            if let Some(prop_def) = prop_def {
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
            }
            // Rule 12 (open world): unknown properties are allowed
        }

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
        // Get when_property
        let when_prop = match condition.get(&iri(wk::WHEN_PROPERTY)) {
            Some(Value::String(s)) => match Iri::parse(s) {
                Ok(i) => i,
                Err(_) => return,
            },
            _ => return,
        };

        // Get has_value
        let has_values = match condition.get(&iri(wk::HAS_VALUE)) {
            Some(val) => val.as_iri_array(),
            None => return,
        };

        // Check if the resource's property value matches any has_value
        let resource_value = match resource.get(&when_prop) {
            Some(v) => v,
            None => return,
        };

        let matches = match resource_value {
            Value::String(s) => {
                if let Ok(val_iri) = Iri::parse(s) {
                    has_values.contains(&val_iri)
                } else {
                    false
                }
            }
            _ => false,
        };

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
                matches!(value, Value::String(_) | Value::Embedded(_))
            }
            wk::RESOURCE_ARRAY => match value {
                Value::Array(arr) => arr
                    .iter()
                    .all(|v| matches!(v, Value::String(_) | Value::Embedded(_))),
                _ => false,
            },
            wk::VALUE_ARRAY => matches!(value, Value::Array(_)),
            wk::JSON => true, // Any value is valid for JSON
            _ => true,        // Unknown data type, skip
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

        let allowed_refs: Vec<&Iri> = allowed_classes.iter().collect();

        let mut errors = Vec::new();
        let values_to_check = match value {
            Value::String(_) | Value::Embedded(_) => vec![value],
            Value::Array(arr) => arr.iter().collect(),
            _ => return vec![],
        };

        for v in values_to_check {
            match v {
                Value::String(ref_str) => {
                    if let Ok(ref_iri) = Iri::parse(ref_str) {
                        if let Some(referenced) = self.layer.resolve(&ref_iri) {
                            if !self.is_instance_of_any(referenced, &allowed_refs) {
                                errors.push(ValidationError {
                                    resource_id: res_id.clone(),
                                    property: Some(prop_iri.clone()),
                                    rule: ValidationRule::ClassTypeMismatch,
                                    message: format!(
                                        "referenced resource '{ref_iri}' is not an instance of any allowed class"
                                    ),
                                });
                            }
                        }
                        // If we can't resolve, skip — might be external
                    }
                }
                Value::Embedded(embedded) => {
                    if !self.is_instance_of_any(embedded, &allowed_refs) {
                        errors.push(ValidationError {
                            resource_id: res_id.clone(),
                            property: Some(prop_iri.clone()),
                            rule: ValidationRule::ClassTypeMismatch,
                            message: "embedded resource is not an instance of any allowed class"
                                .to_string(),
                        });
                    }
                }
                _ => {}
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

        let refs_to_check: Vec<&str> = match value {
            Value::String(s) => vec![s.as_str()],
            Value::Array(arr) => arr.iter().filter_map(|v| v.as_str()).collect(),
            _ => return vec![],
        };

        for ref_str in refs_to_check {
            if let Ok(ref_iri) = Iri::parse(ref_str) {
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
        match prop_def.get(&iri(wk::DATA_TYPE_PROP)) {
            Some(Value::String(s)) => Some(s.clone()),
            _ => None,
        }
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
        Arc::new(builder.build())
    }

    #[test]
    fn core_ontology_validates_against_itself() {
        let core = build_core_layer();
        let validator = Validator::new(&core);
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
        let layer = builder.build();

        let validator = Validator::new(&layer);
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
        let layer = builder.build();

        let validator = Validator::new(&layer);
        let errors = validator.validate();
        assert!(errors
            .iter()
            .any(|e| e.rule == ValidationRule::TypeMismatch));
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
        let layer = builder.build();

        let validator = Validator::new(&layer);
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
        let layer = builder.build();

        let validator = Validator::new(&layer);
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

        let layer = builder.build();
        let validator = Validator::new(&layer);
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

        let layer = builder.build();
        let validator = Validator::new(&layer);
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
    fn derived_resource_without_derivation_fails() {
        let base = build_full_bootstrap_layer();
        let mut builder = LayerBuilder::new("test", Some(base));

        // A resource claiming to be DerivedResource but missing 'derivation'
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

        let layer = builder.build();
        let validator = Validator::new(&layer);
        let errors = validator.validate();

        let derived_errors: Vec<_> = errors
            .iter()
            .filter(|e| {
                e.resource_id.as_ref().map(|i| i.as_str()) == Some("urn:eigenius:test:bad_derived")
                    && e.rule == ValidationRule::MissingRequired
            })
            .collect();
        assert!(
            !derived_errors.is_empty(),
            "DerivedResource without 'derivation' property should fail"
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

        let layer = builder.build();
        let validator = Validator::new(&layer);
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

        let layer = builder.build();
        let validator = Validator::new(&layer);
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

        let layer = builder.build();
        let validator = Validator::new(&layer);
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

        let layer = builder.build();
        let validator = Validator::new(&layer);
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
}
