//! JSON Schema generation from ontology classes.
//!
//! Generates a JSON Schema and a ShortNameTable for a class definition.
//! The schema uses short_name as JSON keys; the table maps them back to IRIs.
//! Used by CompleteJson for structured LLM output.
//!
//! See design document D8 for the full specification.

use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::ontology::resource::Value;
use crate::ontology::well_known as wk;
use std::collections::{BTreeMap, BTreeSet};

/// Mapping from short names back to IRIs for JSON → Eigon conversion.
#[derive(Debug, Clone)]
pub struct ShortNameTable {
    /// Property short_name → property IRI
    pub properties: BTreeMap<String, Iri>,
    /// (property IRI, enum short_name) → allowed value IRI
    pub enums: BTreeMap<(Iri, String), Iri>,
}

/// Errors during schema generation.
#[derive(Debug, Clone)]
pub enum SchemaError {
    ClassNotFound(String),
    PropertyNotFound(String),
    DuplicateShortName(String, String, String),
    CircularReference(String),
    MissingShortName(String),
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchemaError::ClassNotFound(iri) => write!(f, "class not found: {iri}"),
            SchemaError::PropertyNotFound(iri) => write!(f, "property not found: {iri}"),
            SchemaError::DuplicateShortName(name, iri1, iri2) => {
                write!(f, "duplicate short name '{name}': {iri1} and {iri2}")
            }
            SchemaError::CircularReference(iri) => {
                write!(f, "circular reference in class: {iri}")
            }
            SchemaError::MissingShortName(iri) => {
                write!(f, "property has no short_name: {iri}")
            }
        }
    }
}

impl std::error::Error for SchemaError {}

/// Generate a JSON Schema and ShortNameTable for a class.
pub fn schema_for_class(
    class_iri: &Iri,
    layer: &Layer,
) -> Result<(serde_json::Value, ShortNameTable), SchemaError> {
    let mut table = ShortNameTable {
        properties: BTreeMap::new(),
        enums: BTreeMap::new(),
    };
    let mut visited = BTreeSet::new();

    let schema = generate_object_schema(class_iri, layer, &mut table, &mut visited, 0)?;
    Ok((schema, table))
}

fn generate_object_schema(
    class_iri: &Iri,
    layer: &Layer,
    table: &mut ShortNameTable,
    visited: &mut BTreeSet<Iri>,
    depth: usize,
) -> Result<serde_json::Value, SchemaError> {
    if depth > 4 {
        return Ok(serde_json::json!({"type": "object"}));
    }
    if !visited.insert(class_iri.clone()) {
        return Err(SchemaError::CircularReference(
            class_iri.as_str().to_string(),
        ));
    }

    let _class_def = layer
        .resolve(class_iri)
        .ok_or_else(|| SchemaError::ClassNotFound(class_iri.as_str().to_string()))?;

    // Collect required and recommended properties
    let (required, recommended) = collect_properties(class_iri, layer);

    let mut properties = serde_json::Map::new();
    let mut required_names = Vec::new();

    // Process required properties
    for prop_iri in &required {
        let (short_name, prop_schema) =
            generate_property_schema(prop_iri, layer, table, visited, depth)?;
        // Check for duplicate short names
        if let Some(existing) = table.properties.get(&short_name) {
            if existing != prop_iri {
                return Err(SchemaError::DuplicateShortName(
                    short_name,
                    existing.as_str().to_string(),
                    prop_iri.as_str().to_string(),
                ));
            }
        }
        table
            .properties
            .insert(short_name.clone(), prop_iri.clone());
        properties.insert(short_name.clone(), prop_schema);
        required_names.push(serde_json::Value::String(short_name));
    }

    // Process recommended properties (optional — not in required array)
    for prop_iri in &recommended {
        if required.contains(prop_iri) {
            continue;
        }
        let (short_name, prop_schema) =
            generate_property_schema(prop_iri, layer, table, visited, depth)?;
        if let Some(existing) = table.properties.get(&short_name) {
            if existing != prop_iri {
                return Err(SchemaError::DuplicateShortName(
                    short_name,
                    existing.as_str().to_string(),
                    prop_iri.as_str().to_string(),
                ));
            }
        }
        table
            .properties
            .insert(short_name.clone(), prop_iri.clone());
        properties.insert(short_name, prop_schema);
    }

    visited.remove(class_iri);

    let mut schema = serde_json::json!({
        "type": "object",
        "properties": serde_json::Value::Object(properties),
    });
    if !required_names.is_empty() {
        schema["required"] = serde_json::Value::Array(required_names);
    }

    Ok(schema)
}

fn generate_property_schema(
    prop_iri: &Iri,
    layer: &Layer,
    table: &mut ShortNameTable,
    visited: &mut BTreeSet<Iri>,
    depth: usize,
) -> Result<(String, serde_json::Value), SchemaError> {
    let prop_def = layer
        .resolve(prop_iri)
        .ok_or_else(|| SchemaError::PropertyNotFound(prop_iri.as_str().to_string()))?;

    let short_name = prop_def
        .get(&Iri::parse(wk::SHORT_NAME).unwrap())
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| SchemaError::MissingShortName(prop_iri.as_str().to_string()))?;

    let description = prop_def
        .get(&Iri::parse(wk::DESCRIPTION).unwrap())
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let dt_str = prop_def
        .get(&Iri::parse(wk::DATA_TYPE_PROP).unwrap())
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let mut schema = match dt_str {
        wk::STRING | wk::TEMPLATE => serde_json::json!({"type": "string"}),
        wk::INTEGER => serde_json::json!({"type": "integer"}),
        wk::FLOAT => serde_json::json!({"type": "number"}),
        wk::BOOLEAN => serde_json::json!({"type": "boolean"}),
        wk::JSON => serde_json::json!({}),
        wk::RESOURCE => {
            // Check allows_only (enum)
            let ao_iri = Iri::parse(wk::ALLOWS_ONLY).unwrap();
            if let Some(ao_val) = prop_def.get(&ao_iri) {
                let allowed = ao_val.as_iri_array();
                if !allowed.is_empty() {
                    let enum_values: Vec<serde_json::Value> = allowed
                        .iter()
                        .filter_map(|iri| {
                            let r = layer.resolve(iri)?;
                            let sn = r
                                .get(&Iri::parse(wk::SHORT_NAME).unwrap())?
                                .as_str()?
                                .to_string();
                            table
                                .enums
                                .insert((prop_iri.clone(), sn.clone()), iri.clone());
                            Some(serde_json::Value::String(sn))
                        })
                        .collect();
                    return Ok((
                        short_name,
                        add_description(
                            serde_json::json!({"type": "string", "enum": enum_values}),
                            &description,
                        ),
                    ));
                }
            }
            // Check class_types (nested object)
            let ct_iri = Iri::parse(wk::CLASS_TYPES).unwrap();
            if let Some(ct_val) = prop_def.get(&ct_iri) {
                let classes = ct_val.as_iri_array();
                if classes.len() == 1 {
                    return Ok((
                        short_name,
                        add_description(
                            generate_object_schema(&classes[0], layer, table, visited, depth + 1)?,
                            &description,
                        ),
                    ));
                }
            }
            serde_json::json!({"type": "string"})
        }
        wk::VALUE_ARRAY => {
            let et_iri = Iri::parse(wk::ELEMENT_TYPE).unwrap();
            let item_type = match prop_def.get(&et_iri).and_then(|v| v.as_str()) {
                Some(wk::STRING) => serde_json::json!({"type": "string"}),
                Some(wk::INTEGER) => serde_json::json!({"type": "integer"}),
                Some(wk::FLOAT) => serde_json::json!({"type": "number"}),
                Some(wk::BOOLEAN) => serde_json::json!({"type": "boolean"}),
                _ => serde_json::json!({}),
            };
            serde_json::json!({"type": "array", "items": item_type})
        }
        wk::RESOURCE_ARRAY => serde_json::json!({"type": "array", "items": {"type": "object"}}),
        _ => serde_json::json!({"type": "string"}),
    };

    // Add constraints
    if let Some(Value::Integer(min)) = prop_def.get(&Iri::parse(wk::MIN_VALUE).unwrap()) {
        schema["minimum"] = serde_json::json!(min);
    }
    if let Some(Value::Integer(max)) = prop_def.get(&Iri::parse(wk::MAX_VALUE).unwrap()) {
        schema["maximum"] = serde_json::json!(max);
    }
    if let Some(Value::String(pattern)) = prop_def.get(&Iri::parse(wk::PATTERN).unwrap()) {
        schema["pattern"] = serde_json::json!(pattern);
    }

    schema = add_description(schema, &description);

    Ok((short_name, schema))
}

fn add_description(
    mut schema: serde_json::Value,
    description: &Option<String>,
) -> serde_json::Value {
    if let Some(desc) = description {
        schema["description"] = serde_json::Value::String(desc.clone());
    }
    schema
}

fn collect_properties(class_iri: &Iri, layer: &Layer) -> (BTreeSet<Iri>, BTreeSet<Iri>) {
    let mut required = BTreeSet::new();
    let mut recommended = BTreeSet::new();
    let mut visited = BTreeSet::new();
    collect_props_inner(
        class_iri,
        layer,
        &mut required,
        &mut recommended,
        &mut visited,
    );
    (required, recommended)
}

fn collect_props_inner(
    class_iri: &Iri,
    layer: &Layer,
    required: &mut BTreeSet<Iri>,
    recommended: &mut BTreeSet<Iri>,
    visited: &mut BTreeSet<Iri>,
) {
    if !visited.insert(class_iri.clone()) {
        return;
    }
    let resource = match layer.resolve(class_iri) {
        Some(r) => r,
        None => return,
    };

    if let Some(req) = resource.get(&Iri::parse(wk::REQUIRES).unwrap()) {
        for iri in req.as_iri_array() {
            // Skip meta-properties
            if !is_meta_property(&iri) {
                required.insert(iri);
            }
        }
    }
    if let Some(rec) = resource.get(&Iri::parse(wk::RECOMMENDS).unwrap()) {
        for iri in rec.as_iri_array() {
            if !is_meta_property(&iri) {
                recommended.insert(iri);
            }
        }
    }
    if let Some(parents) = resource.get(&Iri::parse(wk::PARENT_CLASSES).unwrap()) {
        for parent in parents.as_iri_array() {
            collect_props_inner(&parent, layer, required, recommended, visited);
        }
    }
}

/// Check if a property is a meta-property (part of ontology infrastructure, not domain data).
fn is_meta_property(iri: &Iri) -> bool {
    let s = iri.as_str();
    matches!(
        s,
        wk::IS_A
            | wk::DESCRIPTION
            | wk::SHORT_NAME
            | wk::PARENT_CLASSES
            | wk::REQUIRES
            | wk::RECOMMENDS
            | wk::CONDITIONAL_REQUIRES
            | wk::DOMAIN
            | wk::SOURCE_IRL
    )
}

/// Convert a simple JSON object (short-name keys) back to an Eigon Resource
/// using the ShortNameTable.
pub fn convert_json_to_resource(
    json: &serde_json::Value,
    table: &ShortNameTable,
    class_iri: &Iri,
) -> Result<crate::ontology::resource::Resource, String> {
    let obj = json
        .as_object()
        .ok_or_else(|| "expected JSON object".to_string())?;

    let mut resource = crate::ontology::resource::Resource::new_embedded();
    resource.set(
        Iri::parse(wk::IS_A).unwrap(),
        Value::Array(vec![Value::String(class_iri.as_str().to_string())]),
    );

    for (key, val) in obj {
        if key == "_type" {
            continue; // Union discriminator — consumed, not stored
        }
        let prop_iri = table
            .properties
            .get(key)
            .ok_or_else(|| format!("unknown property short name: '{key}'"))?;

        let eigon_val = convert_json_value(val, prop_iri, table)?;
        resource.set(prop_iri.clone(), eigon_val);
    }

    Ok(resource)
}

fn convert_json_value(
    val: &serde_json::Value,
    prop_iri: &Iri,
    table: &ShortNameTable,
) -> Result<Value, String> {
    match val {
        serde_json::Value::String(s) => {
            // Check if this is an enum value
            if let Some(iri) = table.enums.get(&(prop_iri.clone(), s.clone())) {
                Ok(Value::String(iri.as_str().to_string()))
            } else {
                Ok(Value::String(s.clone()))
            }
        }
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Float(f))
            } else {
                Ok(Value::String(n.to_string()))
            }
        }
        serde_json::Value::Bool(b) => Ok(Value::Boolean(*b)),
        serde_json::Value::Array(arr) => {
            let items: Result<Vec<Value>, String> = arr
                .iter()
                .map(|v| convert_json_value(v, prop_iri, table))
                .collect();
            Ok(Value::Array(items?))
        }
        serde_json::Value::Object(_) => {
            // Nested object — would need recursive conversion with class info
            // For now, store as JSON
            Ok(Value::Json(val.clone()))
        }
        serde_json::Value::Null => Ok(Value::String(String::new())),
    }
}

/// Validate template references in a component argument against an input class.
///
/// Scans all properties with `data_type: template` in the component argument,
/// extracts `{{iri}}` references, and verifies each exists on the input class.
/// Returns errors for missing properties.
pub fn validate_component_templates(
    component_arg: &crate::ontology::resource::Resource,
    input_class_iri: &Iri,
    layer: &Layer,
) -> Vec<SchemaError> {
    let mut errors = Vec::new();

    // Collect all template strings from the component argument
    let mut template_refs: BTreeSet<Iri> = BTreeSet::new();

    for (prop_iri, value) in component_arg.properties() {
        // Check if this property has data_type: template
        if let Some(prop_def) = layer.resolve(prop_iri) {
            let dt_iri = Iri::parse(wk::DATA_TYPE_PROP).unwrap();
            if let Some(Value::String(dt)) = prop_def.get(&dt_iri) {
                if dt == wk::TEMPLATE {
                    // This is a template property — extract references
                    if let Value::String(template_str) = value {
                        for ref_str in parse_template_references(template_str) {
                            if let Ok(iri) = Iri::parse(&ref_str) {
                                template_refs.insert(iri);
                            }
                        }
                    }
                }
            }
        }
    }

    if template_refs.is_empty() {
        return errors;
    }

    // Collect all properties available on the input class
    let (required, recommended) = collect_properties(input_class_iri, layer);
    let available: BTreeSet<Iri> = required.union(&recommended).cloned().collect();

    // Check each template reference
    for ref_iri in &template_refs {
        if !available.contains(ref_iri) {
            errors.push(SchemaError::PropertyNotFound(format!(
                "template references property '{}' which is not on class '{}'",
                ref_iri, input_class_iri
            )));
        }
    }

    errors
}

/// Parse a template string and extract {{iri}} references.
pub fn parse_template_references(template: &str) -> Vec<String> {
    let re = regex::Regex::new(r"\{\{(\S+?)\}\}").unwrap();
    re.captures_iter(template)
        .map(|c| c[1].to_string())
        .filter(|s| s != "string") // {{string}} is special — no property reference
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap;

    #[test]
    fn schema_for_core_property() {
        let ctx = bootstrap::bootstrap().unwrap();
        let iri = Iri::parse("urn:eigenius:core:Property").unwrap();
        let (schema, table) = schema_for_class(&iri, ctx.head()).unwrap();

        // Property requires: is_a, description, short_name, data_type
        // But is_a, description, short_name are meta-properties (filtered)
        // So only data_type should appear
        assert!(schema["properties"].is_object());
        assert!(!table.properties.is_empty());
    }

    #[test]
    fn parse_template_refs() {
        let refs = parse_template_references(
            "Summarize {{urn:eigenius:demo:text}} by {{urn:eigenius:demo:author}}",
        );
        assert_eq!(refs.len(), 2);
        assert!(refs.contains(&"urn:eigenius:demo:text".to_string()));
        assert!(refs.contains(&"urn:eigenius:demo:author".to_string()));
    }

    #[test]
    fn parse_template_string_special() {
        let refs = parse_template_references("Do something with {{string}}");
        assert!(refs.is_empty()); // {{string}} is special, not a property ref
    }

    #[test]
    fn convert_simple_json() {
        let table = ShortNameTable {
            properties: BTreeMap::from([
                ("name".to_string(), Iri::parse("urn:test:name").unwrap()),
                ("age".to_string(), Iri::parse("urn:test:age").unwrap()),
            ]),
            enums: BTreeMap::new(),
        };

        let json = serde_json::json!({"name": "Alice", "age": 30});
        let class_iri = Iri::parse("urn:test:Person").unwrap();
        let resource = convert_json_to_resource(&json, &table, &class_iri).unwrap();

        assert_eq!(
            resource
                .get(&Iri::parse("urn:test:name").unwrap())
                .unwrap()
                .as_str(),
            Some("Alice")
        );
        assert_eq!(
            resource
                .get(&Iri::parse("urn:test:age").unwrap())
                .unwrap()
                .as_integer(),
            Some(30)
        );
        // Should have is_a
        let is_a = resource.is_a();
        assert_eq!(is_a[0].as_str(), "urn:test:Person");
    }

    #[test]
    fn convert_with_enum() {
        let mut table = ShortNameTable {
            properties: BTreeMap::from([(
                "severity".to_string(),
                Iri::parse("urn:test:severity").unwrap(),
            )]),
            enums: BTreeMap::new(),
        };
        table.enums.insert(
            (Iri::parse("urn:test:severity").unwrap(), "high".to_string()),
            Iri::parse("urn:test:severity:high").unwrap(),
        );

        let json = serde_json::json!({"severity": "high"});
        let class_iri = Iri::parse("urn:test:Issue").unwrap();
        let resource = convert_json_to_resource(&json, &table, &class_iri).unwrap();

        // Should be the full IRI, not the short name
        assert_eq!(
            resource
                .get(&Iri::parse("urn:test:severity").unwrap())
                .unwrap()
                .as_str(),
            Some("urn:test:severity:high")
        );
    }
}
