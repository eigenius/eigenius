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

//! JSON Schema generation from ontology classes, served by the `GetSchema` RPC.
//!
//! A class becomes `{"type": "object", "properties": …, "required": …}`:
//!
//! - **Properties** are the class's `requires` (listed in `required`) and `recommends`, collected
//!   up the `subclass_of` chain. The ontology's meta-properties (`is_a`, `description`,
//!   `short_name`, `subclass_of`, `requires`, `recommends`, `conditional_requires`, `domain`,
//!   `source_irl`) are left out.
//! - **Keys** are each property's `short_name`, unique across the whole schema: a property without
//!   one fails with `SchemaError::MissingShortName`, two properties sharing one with
//!   `SchemaError::DuplicateShortName`. A property's `description` becomes the key's
//!   `description`.
//! - **Data types**: `string`, `integer`, `float` → `number`, `boolean`, `json` → `{}` (any value).
//!   `value_array` → an array of its `element_type`. `resource_array` → an array of objects.
//!   `resource` →
//!   - with `allows_only`: a string `enum` of the allowed values' short names (a value without a
//!     short name is left out);
//!   - with one `class_types` entry: that class's object schema, nested;
//!   - with several: `oneOf` the classes' object schemas, each with a required `_type` key whose
//!     `const` is the class's short name;
//!   - otherwise: a string (the referenced IRI).
//! - **Constraints**: integer `min_value` / `max_value` → `minimum` / `maximum`; `pattern` →
//!   `pattern`.
//!
//! Nesting deeper than four levels yields a bare `{"type": "object"}`; a class that reaches itself
//! fails with `SchemaError::CircularReference`.

use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::ontology::resource::Value;
use crate::ontology::well_known as wk;
use std::collections::{BTreeMap, BTreeSet};

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

/// Short name → property IRI, across the whole schema being generated: the JSON keys.
type ShortNames = BTreeMap<String, Iri>;

/// Generate a JSON Schema for a class.
pub fn schema_for_class(class_iri: &Iri, layer: &Layer) -> Result<serde_json::Value, SchemaError> {
    let mut keys = ShortNames::new();
    let mut visited = BTreeSet::new();
    generate_object_schema(class_iri, layer, &mut keys, &mut visited, 0)
}

fn generate_object_schema(
    class_iri: &Iri,
    layer: &Layer,
    keys: &mut ShortNames,
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
            generate_property_schema(prop_iri, layer, keys, visited, depth)?;
        // Check for duplicate short names
        if let Some(existing) = keys.get(&short_name) {
            if existing != prop_iri {
                return Err(SchemaError::DuplicateShortName(
                    short_name,
                    existing.as_str().to_string(),
                    prop_iri.as_str().to_string(),
                ));
            }
        }
        keys.insert(short_name.clone(), prop_iri.clone());
        properties.insert(short_name.clone(), prop_schema);
        required_names.push(serde_json::Value::String(short_name));
    }

    // Process recommended properties (optional — not in required array)
    for prop_iri in &recommended {
        if required.contains(prop_iri) {
            continue;
        }
        let (short_name, prop_schema) =
            generate_property_schema(prop_iri, layer, keys, visited, depth)?;
        if let Some(existing) = keys.get(&short_name) {
            if existing != prop_iri {
                return Err(SchemaError::DuplicateShortName(
                    short_name,
                    existing.as_str().to_string(),
                    prop_iri.as_str().to_string(),
                ));
            }
        }
        keys.insert(short_name.clone(), prop_iri.clone());
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
    keys: &mut ShortNames,
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

    // `data_type` is `data_type: resource`, so an IRI string; `as_iri` reads it.
    let dt_iri_owned = prop_def
        .get(&Iri::parse(wk::DATA_TYPE_PROP).unwrap())
        .and_then(|v| v.as_iri());
    let dt_str = dt_iri_owned.as_ref().map(|i| i.as_str()).unwrap_or("");

    let mut schema = match dt_str {
        wk::STRING => serde_json::json!({"type": "string"}),
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
            // Check class_types (nested object or union)
            let ct_iri = Iri::parse(wk::CLASS_TYPES).unwrap();
            if let Some(ct_val) = prop_def.get(&ct_iri) {
                let classes = ct_val.as_iri_array();
                if classes.len() == 1 {
                    return Ok((
                        short_name,
                        add_description(
                            generate_object_schema(&classes[0], layer, keys, visited, depth + 1)?,
                            &description,
                        ),
                    ));
                } else if classes.len() > 1 {
                    // Union type — oneOf with a _type discriminator
                    let mut variants = Vec::new();
                    for class_iri in &classes {
                        let class_def = layer.resolve(class_iri).ok_or_else(|| {
                            SchemaError::ClassNotFound(class_iri.as_str().to_string())
                        })?;
                        let class_short = class_def
                            .get(&Iri::parse(wk::SHORT_NAME).unwrap())
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                SchemaError::MissingShortName(class_iri.as_str().to_string())
                            })?
                            .to_string();

                        let mut variant_schema =
                            generate_object_schema(class_iri, layer, keys, visited, depth + 1)?;

                        // Add _type discriminator to the variant
                        if let Some(props) = variant_schema["properties"].as_object_mut() {
                            props.insert(
                                "_type".to_string(),
                                serde_json::json!({"type": "string", "const": class_short}),
                            );
                        }
                        if let Some(req) = variant_schema["required"].as_array_mut() {
                            req.insert(0, serde_json::Value::String("_type".to_string()));
                        } else {
                            variant_schema["required"] = serde_json::json!(["_type"]);
                        }

                        variants.push(variant_schema);
                    }
                    return Ok((
                        short_name,
                        add_description(serde_json::json!({"oneOf": variants}), &description),
                    ));
                }
            }
            serde_json::json!({"type": "string"})
        }
        wk::VALUE_ARRAY => {
            let et_iri = Iri::parse(wk::ELEMENT_TYPE).unwrap();
            let element_iri = prop_def.get(&et_iri).and_then(|v| v.as_iri());
            let item_type = match element_iri.as_ref().map(|i| i.as_str()) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap;

    #[test]
    fn schema_for_core_property() {
        let ctx = bootstrap::bootstrap().unwrap();
        let iri = Iri::parse("urn:eigenius:core:Property").unwrap();
        let schema = schema_for_class(&iri, ctx.head()).unwrap();

        // Property requires: is_a, description, short_name, data_type
        // But is_a, description, short_name are meta-properties (filtered)
        // So only data_type should appear
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("data_type"));
    }

    // --- Tests using the schema-test.json ontology ---

    fn build_schema_test_layer() -> std::sync::Arc<crate::layer::Layer> {
        let ctx = bootstrap::bootstrap().unwrap();
        let test_json = include_str!("../../../ontologies/examples/schema-test.json");
        let resources = crate::ontology::eigon_json::parse_document(test_json).unwrap();
        let mut builder = crate::layer::LayerBuilder::new("schema-test", Some(ctx.head().clone()));
        for r in resources {
            builder.add_resource(r).unwrap();
        }
        std::sync::Arc::new(builder.build(crate::layer::LayerStorage::in_memory()))
    }

    #[test]
    fn schema_incident_has_enum() {
        let layer = build_schema_test_layer();
        let iri = Iri::parse("urn:eigenius:test:schema:Incident").unwrap();
        let schema = schema_for_class(&iri, &layer).unwrap();

        // severity should be an enum with low/medium/high
        let severity = &schema["properties"]["severity"];
        assert_eq!(severity["type"], "string");
        let enum_vals = severity["enum"].as_array().unwrap();
        assert_eq!(enum_vals.len(), 3);
        assert!(enum_vals.contains(&serde_json::json!("low")));
        assert!(enum_vals.contains(&serde_json::json!("medium")));
        assert!(enum_vals.contains(&serde_json::json!("high")));
    }

    #[test]
    fn schema_incident_has_nested_object() {
        let layer = build_schema_test_layer();
        let iri = Iri::parse("urn:eigenius:test:schema:Incident").unwrap();
        let schema = schema_for_class(&iri, &layer).unwrap();

        // location should be a nested object with city, country (required) and building (optional)
        let location = &schema["properties"]["location"];
        assert_eq!(location["type"], "object");
        let loc_props = location["properties"].as_object().unwrap();
        assert!(loc_props.contains_key("city"));
        assert!(loc_props.contains_key("country"));
        assert!(loc_props.contains_key("building"));

        let loc_required = location["required"].as_array().unwrap();
        assert!(loc_required.contains(&serde_json::json!("city")));
        assert!(loc_required.contains(&serde_json::json!("country")));
        // building is recommended, not required
        assert!(!loc_required.contains(&serde_json::json!("building")));
    }

    #[test]
    fn schema_incident_has_union() {
        let layer = build_schema_test_layer();
        let iri = Iri::parse("urn:eigenius:test:schema:Incident").unwrap();
        let schema = schema_for_class(&iri, &layer).unwrap();

        // outcome should be oneOf with _type discriminator
        let outcome = &schema["properties"]["outcome"];
        let one_of = outcome["oneOf"].as_array().unwrap();
        assert_eq!(one_of.len(), 2);

        // Each variant should have _type in required
        for variant in one_of {
            let req = variant["required"].as_array().unwrap();
            assert!(req.contains(&serde_json::json!("_type")));
            let props = variant["properties"].as_object().unwrap();
            assert!(props.contains_key("_type"));
            let type_field = &props["_type"];
            assert_eq!(type_field["type"], "string");
            // Should have a const value (Resolved or Escalated)
            assert!(type_field.get("const").is_some());
        }

        // Verify the specific variants
        let variant_types: Vec<&str> = one_of
            .iter()
            .filter_map(|v| v["properties"]["_type"]["const"].as_str())
            .collect();
        assert!(variant_types.contains(&"Resolved"));
        assert!(variant_types.contains(&"Escalated"));
    }

    #[test]
    fn schema_incident_has_array() {
        let layer = build_schema_test_layer();
        let iri = Iri::parse("urn:eigenius:test:schema:Incident").unwrap();
        let schema = schema_for_class(&iri, &layer).unwrap();

        // tags should be an array of strings (recommended, so present but not required)
        let tags = &schema["properties"]["tags"];
        assert_eq!(tags["type"], "array");
        assert_eq!(tags["items"]["type"], "string");

        // tags is recommended, not in required array
        let required = schema["required"].as_array().unwrap();
        assert!(!required.contains(&serde_json::json!("tags")));
    }

    #[test]
    fn schema_incident_required_fields() {
        let layer = build_schema_test_layer();
        let iri = Iri::parse("urn:eigenius:test:schema:Incident").unwrap();
        let schema = schema_for_class(&iri, &layer).unwrap();

        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&serde_json::json!("title")));
        assert!(required.contains(&serde_json::json!("severity")));
        assert!(required.contains(&serde_json::json!("location")));
        assert!(required.contains(&serde_json::json!("outcome")));
    }

    #[test]
    fn schema_incident_constraints() {
        let layer = build_schema_test_layer();
        let iri = Iri::parse("urn:eigenius:test:schema:Incident").unwrap();
        let schema = schema_for_class(&iri, &layer).unwrap();

        // escalation_level in the Escalated variant should have min/max
        let outcome = &schema["properties"]["outcome"];
        let one_of = outcome["oneOf"].as_array().unwrap();
        let escalated = one_of
            .iter()
            .find(|v| v["properties"]["_type"]["const"] == "Escalated")
            .unwrap();
        let esc_level = &escalated["properties"]["escalation_level"];
        assert_eq!(esc_level["type"], "integer");
        assert_eq!(esc_level["minimum"], 1);
        assert_eq!(esc_level["maximum"], 5);
    }

    #[test]
    fn schema_duplicate_short_name_rejected() {
        let layer = build_schema_test_layer();
        let iri = Iri::parse("urn:eigenius:test:schema:DuplicateShortNameClass").unwrap();
        let result = schema_for_class(&iri, &layer);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, SchemaError::DuplicateShortName(ref name, _, _) if name == "name"),
            "expected DuplicateShortName error, got: {err}"
        );
    }

    #[test]
    fn schema_class_not_found() {
        let layer = build_schema_test_layer();
        let iri = Iri::parse("urn:eigenius:test:schema:NonExistent").unwrap();
        let result = schema_for_class(&iri, &layer);
        assert!(matches!(result, Err(SchemaError::ClassNotFound(_))));
    }
}
