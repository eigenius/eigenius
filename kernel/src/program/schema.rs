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

impl ShortNameTable {
    /// Serialize for cross-process transport. Phase 18e.2: the kernel
    /// embeds this on the `ComponentRequest` argument so the
    /// orchestrator's `CompleteJson` handler can translate LLM
    /// short-name output back to IRI-keyed shape before returning.
    /// Replaces the kernel-side post-hoc translation pattern.
    ///
    /// `class_iri` is the root class the LLM output instantiates;
    /// the orchestrator stamps it onto the translated resource as
    /// `urn:eigenius:core:is_a`. Without it the output would be a
    /// bare property bag with no class identity.
    ///
    /// Shape:
    /// ```json
    /// {
    ///   "class_iri": "urn:eigenius:foo:Person",
    ///   "properties": { "name": "urn:eigenius:foo:name", … },
    ///   "enums": [
    ///     ["urn:eigenius:foo:status", "active", "urn:eigenius:status:active"],
    ///     …
    ///   ]
    /// }
    /// ```
    pub fn to_json(&self, class_iri: &Iri) -> serde_json::Value {
        let properties: serde_json::Map<String, serde_json::Value> = self
            .properties
            .iter()
            .map(|(name, iri)| {
                (
                    name.clone(),
                    serde_json::Value::String(iri.as_str().to_string()),
                )
            })
            .collect();
        let enums: Vec<serde_json::Value> = self
            .enums
            .iter()
            .map(|((prop_iri, name), value_iri)| {
                serde_json::json!([prop_iri.as_str(), name, value_iri.as_str()])
            })
            .collect();
        serde_json::json!({
            "class_iri": class_iri.as_str(),
            "properties": properties,
            "enums": enums,
        })
    }
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

    // `data_type` canonicalises to `ResourceRef`; `as_iri` accepts
    // both that and the pre-canonical `String` shape.
    let dt_iri_owned = prop_def
        .get(&Iri::parse(wk::DATA_TYPE_PROP).unwrap())
        .and_then(|v| v.as_iri());
    let dt_str = dt_iri_owned.as_ref().map(|i| i.as_str()).unwrap_or("");

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
            // Check class_types (nested object or union)
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
                } else if classes.len() > 1 {
                    // Union type — oneOf with _type discriminator (D8 §3.6)
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
                            generate_object_schema(class_iri, layer, table, visited, depth + 1)?;

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
        // Check if this property has data_type: template. `data_type`
        // canonicalises to `ResourceRef`; `as_iri` accepts both
        // shapes for resilience against pre-canonical inputs.
        if let Some(prop_def) = layer.resolve(prop_iri) {
            let dt_iri = Iri::parse(wk::DATA_TYPE_PROP).unwrap();
            if let Some(dt) = prop_def.get(&dt_iri).and_then(|v| v.as_iri()) {
                if dt.as_str() == wk::TEMPLATE {
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

/// Validate output schemas referenced in component arguments.
///
/// Walks the program's expression tree looking for component arguments
/// that reference a class via `output_schema`. For each, calls
/// `schema_for_class` to verify the bijectivity invariant (D8 §4).
/// Returns errors for any class that fails schema generation.
pub fn validate_output_schemas(
    program: &crate::ontology::resource::Resource,
    layer: &Layer,
) -> Vec<SchemaError> {
    let mut errors = Vec::new();
    let body_prop = Iri::parse("urn:eigenius:program:body").unwrap();
    if let Some(Value::Embedded(body)) = program.get(&body_prop) {
        validate_output_schemas_walk(body, layer, &mut errors);
    }
    errors
}

fn validate_output_schemas_walk(
    resource: &crate::ontology::resource::Resource,
    layer: &Layer,
    errors: &mut Vec<SchemaError>,
) {
    let comp_arg_prop = Iri::parse("urn:eigenius:program:component_argument").unwrap();
    let output_schema_prop =
        Iri::parse("urn:eigenius:program:components:completion:output_schema").unwrap();

    // Check if this node has a component_argument with output_schema.
    // `output_schema` is `data_type: resource` (the class IRI), which
    // canonicalises to `ResourceRef`; `as_iri` keeps the legacy
    // `String` shape working too.
    if let Some(Value::Embedded(comp_arg)) = resource.get(&comp_arg_prop) {
        if let Some(class_iri) = comp_arg.get(&output_schema_prop).and_then(|v| v.as_iri()) {
            if let Err(e) = schema_for_class(&class_iri, layer) {
                errors.push(e);
            }
        }
    }

    // Recurse into all embedded children
    for val in resource.properties().values() {
        if let Value::Embedded(child) = val {
            validate_output_schemas_walk(child, layer, errors);
        }
    }
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
        let (schema, table) = schema_for_class(&iri, &layer).unwrap();

        // severity should be an enum with low/medium/high
        let severity = &schema["properties"]["severity"];
        assert_eq!(severity["type"], "string");
        let enum_vals = severity["enum"].as_array().unwrap();
        assert_eq!(enum_vals.len(), 3);
        assert!(enum_vals.contains(&serde_json::json!("low")));
        assert!(enum_vals.contains(&serde_json::json!("medium")));
        assert!(enum_vals.contains(&serde_json::json!("high")));

        // Enum table should have entries
        let sev_iri = Iri::parse("urn:eigenius:test:schema:severity").unwrap();
        assert_eq!(
            table.enums.get(&(sev_iri.clone(), "high".to_string())),
            Some(&Iri::parse("urn:eigenius:test:schema:severity:high").unwrap())
        );
    }

    #[test]
    fn schema_incident_has_nested_object() {
        let layer = build_schema_test_layer();
        let iri = Iri::parse("urn:eigenius:test:schema:Incident").unwrap();
        let (schema, _table) = schema_for_class(&iri, &layer).unwrap();

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
        let (schema, _table) = schema_for_class(&iri, &layer).unwrap();

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
        let (schema, _table) = schema_for_class(&iri, &layer).unwrap();

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
        let (schema, _table) = schema_for_class(&iri, &layer).unwrap();

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
        let (schema, _table) = schema_for_class(&iri, &layer).unwrap();

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
    fn schema_roundtrip_with_enum() {
        let layer = build_schema_test_layer();
        let iri = Iri::parse("urn:eigenius:test:schema:Incident").unwrap();
        let (_schema, table) = schema_for_class(&iri, &layer).unwrap();

        // Simulate an LLM response with short-name keys
        let json = serde_json::json!({
            "title": "Server outage",
            "severity": "high",
            "location": {"city": "Berlin", "country": "Germany"},
            "outcome": {"_type": "Resolved", "resolution_notes": "Rebooted"},
            "tags": ["infrastructure", "critical"]
        });

        let resource = convert_json_to_resource(&json, &table, &iri).unwrap();

        // title → string
        assert_eq!(
            resource
                .get(&Iri::parse("urn:eigenius:test:schema:title").unwrap())
                .unwrap()
                .as_str(),
            Some("Server outage")
        );
        // severity → enum IRI
        assert_eq!(
            resource
                .get(&Iri::parse("urn:eigenius:test:schema:severity").unwrap())
                .unwrap()
                .as_str(),
            Some("urn:eigenius:test:schema:severity:high")
        );
        // tags → array of strings
        let tags = resource
            .get(&Iri::parse("urn:eigenius:test:schema:tags").unwrap())
            .unwrap();
        match tags {
            Value::Array(arr) => {
                assert_eq!(arr.len(), 2);
            }
            _ => panic!("expected array for tags"),
        }
    }

    #[test]
    fn schema_class_not_found() {
        let layer = build_schema_test_layer();
        let iri = Iri::parse("urn:eigenius:test:schema:NonExistent").unwrap();
        let result = schema_for_class(&iri, &layer);
        assert!(matches!(result, Err(SchemaError::ClassNotFound(_))));
    }
}
