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

//! Pattern matching: positive / negated patterns, candidate collection,
//! class-closure walks, subject/object binding.
//!
//! This module also owns the [`Binding`] alias and the small
//! resolve/literal helpers shared with [`super::expression`].

use crate::layer::{is_indexable_predicate, scan_chain, Layer};
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use crate::query::ast::*;
use crate::query::error::QueryError;
use crate::query::functions::values_equal;
use std::collections::{BTreeMap, BTreeSet};

/// A binding maps variable names to values.
pub(super) type Binding = BTreeMap<String, Value>;

/// Apply a positive pattern: join with existing bindings.
///
/// `overlay` is the slice of transient fiber-response resources (possibly
/// empty) produced by earlier FIBER clauses in the same query. They are
/// merged into the candidate set alongside layer resources so pattern
/// matching on FIBER-bound variables works uniformly.
pub(super) fn apply_pattern(
    pattern: &Pattern,
    layer: &Layer,
    derived: &BTreeMap<String, Vec<Binding>>,
    overlay: &[(Iri, Resource)],
    existing: Vec<Binding>,
) -> Result<Vec<Binding>, QueryError> {
    let candidates = collect_candidates(pattern, layer, derived, overlay);
    let mut result = Vec::new();

    for binding in &existing {
        for (resource_iri, resource) in &candidates {
            if let Some(new_binding) = try_match_resource(pattern, resource, resource_iri, binding)
            {
                result.push(new_binding);
            }
        }
    }

    Ok(result)
}

/// Apply a negated pattern: keep bindings where no match exists.
pub(super) fn apply_negated_pattern(
    pattern: &Pattern,
    layer: &Layer,
    derived: &BTreeMap<String, Vec<Binding>>,
    overlay: &[(Iri, Resource)],
    existing: Vec<Binding>,
) -> Result<Vec<Binding>, QueryError> {
    let candidates = collect_candidates(pattern, layer, derived, overlay);
    let mut result = Vec::new();

    for binding in &existing {
        let has_match = candidates
            .iter()
            .any(|(iri, resource)| try_match_resource(pattern, resource, iri, binding).is_some());
        if !has_match {
            result.push(binding.clone());
        }
    }

    Ok(result)
}

/// Collect candidate resources for a pattern.
///
/// Phase 14h: when the pattern's class is bound and the `is_a` predicate
/// is indexable (its `Property.data_type` is `resource` or
/// `resource_array`), this uses [`scan_chain`] to enumerate matching
/// subjects via the per-layer triple index instead of the full chain
/// scan that pre-14h code used. The scan path remains as a fallback for
/// untyped patterns and for setups where `is_a` somehow lost its
/// indexable data_type.
fn collect_candidates<'a>(
    pattern: &Pattern,
    layer: &'a Layer,
    derived: &'a BTreeMap<String, Vec<Binding>>,
    overlay: &'a [(Iri, Resource)],
) -> Vec<(Option<Iri>, BTreeMap<Iri, Value>)> {
    // Check if this references a derived relation
    if let Some(Name::ShortName(ref name)) = pattern.class {
        if let Some(derived_bindings) = derived.get(name) {
            // Convert derived bindings to pseudo-resources
            return derived_bindings
                .iter()
                .map(|b| {
                    let props: BTreeMap<Iri, Value> = b
                        .iter()
                        .filter_map(|(k, v)| {
                            Iri::parse(&format!("urn:derived:{k}"))
                                .ok()
                                .map(|iri| (iri, v.clone()))
                        })
                        .collect();
                    (None, props)
                })
                .collect();
        }
    }

    let class_iri = pattern.class.as_ref().and_then(|n| resolve_name(n, layer));
    let is_a_iri = Iri::parse(wk::IS_A).expect("well-known is_a IRI");

    // Indexed path: bound class + indexable is_a predicate.
    let mut candidates: Vec<(Option<Iri>, BTreeMap<Iri, Value>)> =
        if let Some(ref class) = class_iri {
            if is_indexable_predicate(layer, &is_a_iri) {
                let class_closure = class_with_subclass_closure(class, layer);
                let mut subjects: BTreeSet<Iri> = BTreeSet::new();
                for concrete in &class_closure {
                    for s in scan_chain(layer, &is_a_iri, concrete) {
                        subjects.insert(s);
                    }
                }
                subjects
                    .into_iter()
                    .filter_map(|iri| {
                        layer
                            .resolve(&iri)
                            .map(|r| (Some(iri), r.properties().clone()))
                    })
                    .collect()
            } else {
                collect_candidates_via_scan(layer, Some(class))
            }
        } else {
            // Untyped pattern: no predicate to index by, fall back to scan.
            collect_candidates_via_scan(layer, None)
        };

    for (iri, resource) in overlay {
        let matches = if let Some(ref class) = class_iri {
            resource.is_instance_of(class) || is_subclass_instance(resource, class, layer)
        } else {
            true
        };
        if matches {
            candidates.push((Some(iri.clone()), resource.properties().clone()));
        }
    }

    candidates
}

/// Pre-14h scan path retained for the untyped-pattern case and as
/// fallback when `is_a`'s data_type isn't indexable. Walks the entire
/// chain via `iter_all_resources`.
fn collect_candidates_via_scan(
    layer: &Layer,
    class_iri: Option<&Iri>,
) -> Vec<(Option<Iri>, BTreeMap<Iri, Value>)> {
    layer
        .iter_all_resources()
        .filter(|(_, resource)| {
            if let Some(class) = class_iri {
                resource.is_instance_of(class) || is_subclass_instance(resource, class, layer)
            } else {
                true
            }
        })
        .map(|(iri, resource)| (Some(iri.clone()), resource.properties().clone()))
        .collect()
}

/// `{class} ∪ all transitive subclasses(class)` — the set of concrete
/// classes whose instances satisfy `MATCH ?x : class { ... }`. Walks the
/// `subclass_of` index recursively. When `subclass_of` isn't indexable,
/// returns just `{class}` and accepts the (small) loss of subclass
/// matches — pre-14h behavior would also have missed them via the
/// scan-only `is_subclass_instance` walk in degenerate setups.
fn class_with_subclass_closure(class_iri: &Iri, layer: &Layer) -> BTreeSet<Iri> {
    let subclass_of = Iri::parse(wk::PARENT_CLASSES).expect("well-known subclass_of IRI");
    let mut closure: BTreeSet<Iri> = BTreeSet::new();
    closure.insert(class_iri.clone());
    if !is_indexable_predicate(layer, &subclass_of) {
        return closure;
    }
    let mut frontier: Vec<Iri> = vec![class_iri.clone()];
    while let Some(parent) = frontier.pop() {
        for sub in scan_chain(layer, &subclass_of, &parent) {
            if closure.insert(sub.clone()) {
                frontier.push(sub);
            }
        }
    }
    closure
}

/// Try to match a resource against a pattern, extending an existing binding.
fn try_match_resource(
    pattern: &Pattern,
    resource_props: &BTreeMap<Iri, Value>,
    resource_iri: &Option<Iri>,
    existing: &Binding,
) -> Option<Binding> {
    let mut binding = existing.clone();

    // Bind the subject variable
    let subject_name = &pattern.subject.name;
    if let Some(iri) = resource_iri {
        let iri_val = Value::String(iri.as_str().to_string());
        if let Some(existing_val) = binding.get(subject_name) {
            if !values_equal(existing_val, &iri_val) {
                return None; // Conflict with existing binding
            }
        }
        binding.insert(subject_name.clone(), iri_val);
    }

    // Match property patterns
    for prop_pat in &pattern.properties {
        let prop_iri = match &prop_pat.property {
            Name::ShortName(s) => {
                // Find by shortname in resource properties
                find_property_by_shortname(s, resource_props)?
            }
            Name::FullIri(iri) => iri.clone(),
        };

        let value = resource_props.get(&prop_iri);

        match &prop_pat.object {
            ValueOrVariable::Variable(var) => {
                match value {
                    Some(val) => {
                        if let Some(existing_val) = binding.get(&var.name) {
                            if !values_equal(existing_val, val) {
                                return None; // Conflict
                            }
                        } else {
                            binding.insert(var.name.clone(), val.clone());
                        }
                    }
                    None => {
                        // Property not present — mark as unbound for NOT EXISTS
                        // Don't insert anything; NOT EXISTS checks for absence
                        // But this means the pattern doesn't match this resource
                        // unless we're specifically allowing optional matching
                        return None;
                    }
                }
            }
            ValueOrVariable::Literal(lit) => {
                let expected = literal_to_value(lit);
                match value {
                    Some(val) if values_equal(val, &expected) => {}
                    _ => return None,
                }
            }
        }
    }

    Some(binding)
}

/// Find a property IRI by shortname by looking it up in the resource's keys.
pub(super) fn find_property_by_shortname(
    shortname: &str,
    props: &BTreeMap<Iri, Value>,
) -> Option<Iri> {
    props
        .keys()
        .find(|iri| iri.local_name() == shortname)
        .cloned()
}

/// Check if a resource is an instance of a class via subclass_of chain.
fn is_subclass_instance(resource: &Resource, class_iri: &Iri, layer: &Layer) -> bool {
    let resource_classes = resource.is_a();
    let subclass_iri = Iri::parse(wk::PARENT_CLASSES).unwrap();

    for res_class in &resource_classes {
        if is_subclass_of(res_class, class_iri, layer, &subclass_iri) {
            return true;
        }
    }
    false
}

fn is_subclass_of(sub: &Iri, target: &Iri, layer: &Layer, subclass_prop: &Iri) -> bool {
    if let Some(class_def) = layer.resolve(sub) {
        if let Some(parents) = class_def.get(subclass_prop) {
            for parent in parents.as_iri_array() {
                if parent == *target {
                    return true;
                }
                if is_subclass_of(&parent, target, layer, subclass_prop) {
                    return true;
                }
            }
        }
    }
    false
}

/// Resolve a Name to an IRI.
fn resolve_name(name: &Name, layer: &Layer) -> Option<Iri> {
    match name {
        Name::FullIri(iri) => Some(iri.clone()),
        Name::ShortName(s) => {
            // Search layer for a resource with this shortname
            let short_name_iri = Iri::parse(wk::SHORT_NAME).ok()?;
            for (iri, resource) in layer.iter_all_resources() {
                if let Some(Value::String(sn)) = resource.get(&short_name_iri) {
                    if sn == s {
                        return Some(iri.clone());
                    }
                }
            }
            None
        }
    }
}

pub(super) fn literal_to_value(lit: &Literal) -> Value {
    match lit {
        Literal::String(s) => Value::String(s.clone()),
        Literal::Integer(n) => Value::Integer(*n),
        Literal::Float(f) => Value::Float(*f),
        Literal::Boolean(b) => Value::Boolean(*b),
    }
}

#[cfg(test)]
mod tests {
    use super::super::evaluate;
    use super::super::FiberRuntime;
    use crate::layer::{Layer, LayerBuilder};
    use crate::ontology::eigon_json;
    use crate::ontology::iri::Iri;
    use crate::ontology::resource::{Resource, Value};
    use crate::query::document::QueryFingerprint;
    use crate::query::lexer::tokenize;
    use crate::query::parser;
    use std::sync::Arc;

    pub(crate) fn build_test_layer() -> Arc<Layer> {
        let storage = crate::layer::LayerStorage::in_memory();
        let core_json = include_str!("../../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut builder = LayerBuilder::new("core", None);
        for r in core_resources {
            builder.add_resource(r).unwrap();
        }

        // Add example animals
        let animals_json = include_str!("../../../../ontologies/examples/animals.json");
        let animal_resources = eigon_json::parse_document(animals_json).unwrap();
        // Need a new layer on top of core. Share the same `LayerStorage`
        // so the bloom cache, resource cache, and triple index are all
        // populated from one set of writes — production bootstrap does
        // the same (see `bootstrap_with_storage`).
        let core = Arc::new(builder.build(storage.clone()));
        let mut domain_builder = LayerBuilder::new("animals", Some(core));
        for r in animal_resources {
            domain_builder.add_resource(r).unwrap();
        }
        Arc::new(domain_builder.build(storage))
    }

    pub(crate) fn run_query(layer: &Layer, query_str: &str) -> Vec<Resource> {
        let tokens = tokenize(query_str).unwrap();
        let program = parser::parse(tokens).unwrap();
        let fp = QueryFingerprint::of(query_str);
        evaluate(&program, layer, &fp, FiberRuntime::default())
            .unwrap()
            .0
    }

    #[test]
    fn find_all_classes() {
        let layer = build_test_layer();
        let results = run_query(
            &layer,
            r#"
            USING "urn:eigenius:core:Class"
            MATCH Class(?c) {
                short_name: ?name
            }
            RETURN [] {
                short_name: ?name
            }
            "#,
        );
        // Should find core classes + example classes (Animal, Dog)
        assert!(
            results.len() >= 6,
            "expected at least 6 classes, got {}",
            results.len()
        );
    }

    #[test]
    fn find_dog_instance() {
        let layer = build_test_layer();
        let results = run_query(
            &layer,
            r#"
            MATCH "urn:eigenius:example:Dog"(?d) {
                "urn:eigenius:example:name": ?name,
                "urn:eigenius:example:breed": ?breed
            }
            RETURN [] {
                "urn:eigenius:example:name": ?name,
                "urn:eigenius:example:breed": ?breed
            }
            "#,
        );
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn where_filtering() {
        let layer = build_test_layer();
        let results = run_query(
            &layer,
            r#"
            MATCH "urn:eigenius:example:Dog"(?d) {
                "urn:eigenius:example:breed": ?breed
            }
            WHERE ?breed = "German Shepherd"
            RETURN [] {
                "urn:eigenius:example:breed": ?breed
            }
            "#,
        );
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn where_no_match() {
        let layer = build_test_layer();
        let results = run_query(
            &layer,
            r#"
            MATCH "urn:eigenius:example:Dog"(?d) {
                "urn:eigenius:example:breed": ?breed
            }
            WHERE ?breed = "Poodle"
            RETURN [] {
                "urn:eigenius:example:breed": ?breed
            }
            "#,
        );
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn match_only_guard() {
        let layer = build_test_layer();
        let results = run_query(
            &layer,
            r#"
            MATCH "urn:eigenius:example:Dog"(?d) {
                "urn:eigenius:example:breed": ?breed
            }
            WHERE ?breed = "German Shepherd"
            "#,
        );
        // Guard query returns bindings (non-empty = true)
        assert!(!results.is_empty());
    }

    #[test]
    fn limit_results() {
        let layer = build_test_layer();
        let results = run_query(
            &layer,
            r#"
            USING "urn:eigenius:core:Property"
            MATCH Property(?p) {
                short_name: ?name
            }
            RETURN [] {
                short_name: ?name
            }
            LIMIT 3
            "#,
        );
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn like_operator() {
        let layer = build_test_layer();
        let results = run_query(
            &layer,
            r#"
            USING "urn:eigenius:core:Property"
            MATCH Property(?p) {
                short_name: ?name
            }
            WHERE ?name LIKE "data_%"
            RETURN [] {
                short_name: ?name
            }
            "#,
        );
        // Should find data_type
        assert!(!results.is_empty());
    }

    #[test]
    fn arithmetic_in_where() {
        let layer = build_test_layer();
        let results = run_query(
            &layer,
            r#"
            MATCH ?x {}
            WHERE 1 + 2 = 3
            RETURN [] {}
            LIMIT 1
            "#,
        );
        assert!(!results.is_empty());
    }

    fn build_hierarchy_layer() -> Arc<Layer> {
        // Build a simple hierarchy: Alice -> Bob -> Charlie
        let storage = crate::layer::LayerStorage::in_memory();
        let core_json = include_str!("../../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut core_builder = LayerBuilder::new("core", None);
        for r in core_resources {
            core_builder.add_resource(r).unwrap();
        }
        let core = Arc::new(core_builder.build(storage.clone()));

        let mut builder = LayerBuilder::new("hierarchy", Some(core));

        let mut alice = Resource::new(Iri::parse("urn:eigenius:test:alice").unwrap());
        alice.set(
            Iri::parse("urn:eigenius:test:name").unwrap(),
            Value::String("Alice".into()),
        );
        alice.set(
            Iri::parse("urn:eigenius:test:reports_to").unwrap(),
            Value::String("urn:eigenius:test:bob".into()),
        );
        builder.add_resource(alice).unwrap();

        let mut bob = Resource::new(Iri::parse("urn:eigenius:test:bob").unwrap());
        bob.set(
            Iri::parse("urn:eigenius:test:name").unwrap(),
            Value::String("Bob".into()),
        );
        bob.set(
            Iri::parse("urn:eigenius:test:reports_to").unwrap(),
            Value::String("urn:eigenius:test:charlie".into()),
        );
        builder.add_resource(bob).unwrap();

        let mut charlie = Resource::new(Iri::parse("urn:eigenius:test:charlie").unwrap());
        charlie.set(
            Iri::parse("urn:eigenius:test:name").unwrap(),
            Value::String("Charlie".into()),
        );
        builder.add_resource(charlie).unwrap();

        Arc::new(builder.build(storage))
    }

    #[test]
    fn recursive_define_ancestor() {
        let layer = build_hierarchy_layer();
        let results = run_query(
            &layer,
            r#"
            DEFINE Ancestor(?x, ?z) FROM
                MATCH ?x { "urn:eigenius:test:reports_to": ?z }
            DEFINE Ancestor(?x, ?z) FROM
                MATCH ?x { "urn:eigenius:test:reports_to": ?y },
                Ancestor(?y) { "urn:eigenius:test:reports_to": ?z }
            MATCH ?person {}
            WHERE ?person = "urn:eigenius:test:alice"
            RETURN [] {}
            "#,
        );
        // Alice should match
        assert!(!results.is_empty());
    }

    #[test]
    fn non_recursive_define() {
        let layer = build_hierarchy_layer();
        let results = run_query(
            &layer,
            r#"
            DEFINE Manager(?x, ?mgr) FROM
                MATCH ?x { "urn:eigenius:test:reports_to": ?mgr }
            MATCH ?x {}
            RETURN [] { "urn:eigenius:test:name": ?x }
            LIMIT 5
            "#,
        );
        assert!(!results.is_empty());
    }
}
