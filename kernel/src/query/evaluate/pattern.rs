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

/// A pattern's candidate rows: `(subject IRI, property map)` pairs collected from
/// the layer chain (and FIBER overlay) before brace refinement / join.
type Candidates = Vec<(Option<Iri>, BTreeMap<Iri, Value>)>;

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
    conditions: &[Expression],
    namespaces: &[String],
) -> Result<Vec<Binding>, QueryError> {
    // Subject-predicate pushdown: if a WHERE conjunct constrains this pattern's
    // subject to an IRI prefix (`LIKE "p%"`), a single IRI (`= "iri"`), or a set
    // (`IN [...]`), collect candidates by IRI instead of scanning the chain. The
    // WHERE still re-applies the predicate, so this only ever pre-filters — never
    // drops a valid row. Decisive for untyped `MATCH ?r {}` over a large chain.
    let subject_constraint = extract_subject_constraint(&pattern.subject.name, conditions);
    let candidates = collect_candidates(
        pattern,
        layer,
        derived,
        overlay,
        subject_constraint.as_ref(),
        namespaces,
    )?;
    let mut result = Vec::new();

    for binding in &existing {
        for (resource_iri, resource) in &candidates {
            result.extend(try_match_resource(pattern, resource, resource_iri, binding));
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
    namespaces: &[String],
) -> Result<Vec<Binding>, QueryError> {
    // No subject pushdown for negated patterns — narrowing the candidate set of a
    // `NOT` pattern would change its semantics. Always the full candidate view.
    let candidates = collect_candidates(pattern, layer, derived, overlay, None, namespaces)?;
    let mut result = Vec::new();

    for binding in &existing {
        let has_match = candidates
            .iter()
            .any(|(iri, resource)| !try_match_resource(pattern, resource, iri, binding).is_empty());
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
#[allow(clippy::too_many_arguments)]
fn collect_candidates<'a>(
    pattern: &Pattern,
    layer: &'a Layer,
    derived: &'a BTreeMap<String, Vec<Binding>>,
    overlay: &'a [(Iri, Resource)],
    subject_constraint: Option<&SubjectConstraint>,
    namespaces: &[String],
) -> Result<Candidates, QueryError> {
    // Check if this references a derived relation. Derived rows are stored as
    // positional tuples ("0" = the relation's first/subject column; see
    // `project_onto_head` in mod.rs). A pattern references a derived relation by
    // its subject only — the parser allows a single variable, e.g. `Reach(?n)` —
    // so bind that subject from column "0", and when the column value is a
    // resource IRI, resolve it to the REAL resource. That makes a brace
    // refinement (`Reach(?n) { prop: ?v }`) match the resource's actual
    // properties and join on shared variables, exactly like a concrete pattern.
    // A column IRI that doesn't resolve (a dangling reference) yields empty
    // properties: the subject still binds (the row is in the relation) but no
    // brace refinement can match it.
    if let Some(Name::ShortName(ref name)) = pattern.class {
        if let Some(derived_bindings) = derived.get(name) {
            return Ok(derived_bindings
                .iter()
                .filter_map(|b| {
                    // The subject column may be a `Value::String` (a subject
                    // binding, parse-time) or a `Value::ResourceRef` (a value read
                    // from a resource-valued property on a canonicalised chain).
                    // `as_iri` accepts both — a strict `Value::String` match would
                    // silently drop the ResourceRef case (e.g. a relation derived
                    // from `Objective { thesis: ?t }`).
                    let iri = b.get("0")?.as_iri()?;
                    let props = layer
                        .resolve(&iri)
                        .map(|r| r.properties().clone())
                        .unwrap_or_default();
                    Some((Some(iri), props))
                })
                .collect());
        }
    }

    let class_iri = match pattern.class.as_ref() {
        Some(n) => resolve_name(n, layer, namespaces)?,
        None => None,
    };
    let is_a_iri = Iri::parse(wk::IS_A).expect("well-known is_a IRI");

    // Indexed path: bound class + indexable is_a predicate.
    let mut candidates: Candidates = if let Some(ref class) = class_iri {
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
    } else if let Some(constraint) = subject_constraint {
        // Untyped pattern with a subject-bound WHERE conjunct: collect by IRI
        // (prefix range over `defined_iris`, or direct resolve) instead of
        // materialising the whole chain.
        collect_candidates_via_subject(layer, constraint)
    } else {
        // Untyped pattern, no usable constraint: fall back to the full scan.
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

    Ok(candidates)
}

/// Pre-14h scan path retained for the untyped-pattern case and as
/// fallback when `is_a`'s data_type isn't indexable. Walks the entire
/// chain via `iter_all_resources`.
fn collect_candidates_via_scan(layer: &Layer, class_iri: Option<&Iri>) -> Candidates {
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

/// A pushdown-able constraint on a pattern's subject variable, extracted from a
/// WHERE conjunct so an untyped `MATCH ?r {}` can collect candidates by IRI rather
/// than scanning the chain.
enum SubjectConstraint {
    /// `?r LIKE "p%"` — a pure trailing-`%` prefix (no other wildcards). Holds the
    /// prefix with the trailing `%` removed.
    Prefix(String),
    /// `?r = "iri"` or `?r IN ["iri", …]` — an explicit subject set.
    Iris(Vec<Iri>),
}

/// Extract a pushdown-able constraint on `subject` from the WHERE `conditions`
/// (top-level conjuncts). Returns the first usable one; the WHERE re-applies every
/// condition afterwards, so picking any single conjunct is sound. Only the simple
/// shapes `?subject LIKE "p%"`, `?subject = "iri"`, `?subject IN [...]` are
/// recognised — disjunctions, mid-string wildcards, and non-IRI literals are left to
/// the normal scan + WHERE filter.
fn extract_subject_constraint(
    subject: &str,
    conditions: &[Expression],
) -> Option<SubjectConstraint> {
    for cond in conditions {
        let Expression::Binary { op, left, right } = cond else {
            continue;
        };
        // Normalise to `var <op> lit` (also accept `lit = var` for Eq).
        let (var, lit) = match (left.as_ref(), right.as_ref()) {
            (Expression::Variable(v), other) => (v, other),
            (other, Expression::Variable(v)) if matches!(op, BinaryOp::Eq) => (v, other),
            _ => continue,
        };
        if var.name != subject {
            continue;
        }
        match op {
            BinaryOp::Like => {
                if let Expression::Literal(Literal::String(pat)) = lit {
                    if let Some(prefix) = like_pure_prefix(pat) {
                        return Some(SubjectConstraint::Prefix(prefix));
                    }
                }
            }
            BinaryOp::Eq => {
                if let Expression::Literal(Literal::String(s)) = lit {
                    if let Ok(iri) = Iri::parse(s) {
                        return Some(SubjectConstraint::Iris(vec![iri]));
                    }
                }
            }
            BinaryOp::In => {
                if let Expression::Array(items) = lit {
                    let iris: Vec<Iri> = items
                        .iter()
                        .filter_map(|e| match e {
                            Expression::Literal(Literal::String(s)) => Iri::parse(s).ok(),
                            _ => None,
                        })
                        .collect();
                    // Only push down a set that's entirely parseable IRI literals;
                    // anything mixed/empty falls back to scan + WHERE.
                    if !iris.is_empty() && iris.len() == items.len() {
                        return Some(SubjectConstraint::Iris(iris));
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// If `pat` is a pure prefix LIKE pattern — a non-empty literal followed by a single
/// trailing `%`, with no other `%`/`_` wildcards — return the literal prefix.
fn like_pure_prefix(pat: &str) -> Option<String> {
    let prefix = pat.strip_suffix('%')?;
    if prefix.is_empty() || prefix.contains('%') || prefix.contains('_') {
        return None;
    }
    Some(prefix.to_string())
}

/// Collect candidates for an untyped pattern via a subject constraint, never
/// materialising non-matching resources.
fn collect_candidates_via_subject(layer: &Layer, constraint: &SubjectConstraint) -> Candidates {
    let iris: BTreeSet<Iri> = match constraint {
        SubjectConstraint::Prefix(prefix) => {
            // Walk the chain gathering defined IRIs (metadata — no bodies) that share
            // the prefix; only those get resolved. A `BTreeSet` range would be
            // O(matches), but this prefix-filtered iteration is already O(chain-IRIs)
            // with zero body paging — the materialisation, not the IRI scan, was the
            // O(chain) cost.
            let mut out: BTreeSet<Iri> = BTreeSet::new();
            let mut current: Option<&Layer> = Some(layer);
            let mut visited: BTreeSet<crate::layer::LayerId> = BTreeSet::new();
            while let Some(l) = current {
                if !visited.insert(l.id().clone()) {
                    break;
                }
                for iri in l.defined_iris() {
                    if iri.as_str().starts_with(prefix.as_str()) {
                        out.insert(iri.clone());
                    }
                }
                current = l.parent().map(|p| p.as_ref());
            }
            out
        }
        SubjectConstraint::Iris(iris) => iris.iter().cloned().collect(),
    };
    iris.into_iter()
        .filter_map(|iri| {
            layer
                .resolve(&iri)
                .map(|r| (Some(iri), r.properties().clone()))
        })
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
///
/// Returns *all* extended bindings the match produces: empty on no match, one
/// for an ordinary match, and possibly many when an array pattern's `[... ?e ...]`
/// (Each) form iterates a property's elements (D59). Each property pattern is a
/// join step over the running frontier of partial bindings.
fn try_match_resource(
    pattern: &Pattern,
    resource_props: &BTreeMap<Iri, Value>,
    resource_iri: &Option<Iri>,
    existing: &Binding,
) -> Vec<Binding> {
    let mut base = existing.clone();

    // Bind the subject variable.
    let subject_name = &pattern.subject.name;
    if let Some(iri) = resource_iri {
        let iri_val = Value::String(iri.as_str().to_string());
        if let Some(existing_val) = base.get(subject_name) {
            if !values_equal(existing_val, &iri_val) {
                return Vec::new(); // conflict with existing binding
            }
        }
        base.insert(subject_name.clone(), iri_val);
    }

    // Match property patterns, threading a frontier of partial bindings.
    let mut frontier = vec![base];
    for prop_pat in &pattern.properties {
        let prop_iri = match &prop_pat.property {
            Name::ShortName(s) => match find_property_by_shortname(s, resource_props) {
                Some(iri) => iri,
                None => return Vec::new(),
            },
            Name::FullIri(iri) => iri.clone(),
        };
        let value = resource_props.get(&prop_iri);

        let mut next: Vec<Binding> = Vec::new();
        for b in frontier {
            match &prop_pat.object {
                ValueOrVariable::Variable(var) => {
                    // property absent → no match
                    if let Some(val) = value {
                        match b.get(&var.name) {
                            Some(existing_val) if !values_equal(existing_val, val) => {}
                            Some(_) => next.push(b),
                            None => {
                                let mut nb = b;
                                nb.insert(var.name.clone(), val.clone());
                                next.push(nb);
                            }
                        }
                    }
                }
                ValueOrVariable::Literal(lit) => {
                    let expected = literal_to_value(lit);
                    if let Some(val) = value {
                        if values_equal(val, &expected) {
                            next.push(b);
                        }
                    }
                }
                ValueOrVariable::Array(ap) => {
                    // property absent or not an array → no match
                    if let Some(Value::Array(items)) = value {
                        match_array_pattern(ap, items, b, &mut next);
                    }
                }
            }
        }
        frontier = next;
        if frontier.is_empty() {
            return Vec::new();
        }
    }

    frontier
}

/// Match an array pattern (D59) against a property's elements, pushing every
/// resulting binding (an `Each` pattern forks one per element) onto `out`.
fn match_array_pattern(ap: &ArrayPattern, elems: &[Value], base: Binding, out: &mut Vec<Binding>) {
    match ap {
        ArrayPattern::Exact(vars) => {
            if elems.len() == vars.len() {
                if let Some(b) = bind_positional(vars, elems, base) {
                    out.push(b);
                }
            }
        }
        ArrayPattern::AtLeast(vars) => {
            if elems.len() >= vars.len() {
                if let Some(b) = bind_positional(vars, &elems[..vars.len()], base) {
                    out.push(b);
                }
            }
        }
        ArrayPattern::Each(var) => {
            for el in elems {
                match base.get(&var.name) {
                    Some(existing_val) if !values_equal(existing_val, el) => {}
                    Some(_) => out.push(base.clone()),
                    None => {
                        let mut nb = base.clone();
                        nb.insert(var.name.clone(), el.clone());
                        out.push(nb);
                    }
                }
            }
        }
    }
}

/// Bind a positional run of variables to array elements (equi-join on any var
/// already bound). Returns None on a binding conflict.
fn bind_positional(vars: &[Variable], elems: &[Value], base: Binding) -> Option<Binding> {
    let mut b = base;
    for (var, el) in vars.iter().zip(elems.iter()) {
        match b.get(&var.name) {
            Some(existing_val) if !values_equal(existing_val, el) => return None,
            Some(_) => {}
            None => {
                b.insert(var.name.clone(), el.clone());
            }
        }
    }
    Some(b)
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

/// Check if a resource is a (subclass-)instance of a class, via the single
/// foundation authority [`Layer::is_subclass_of`].
fn is_subclass_instance(resource: &Resource, class_iri: &Iri, layer: &Layer) -> bool {
    resource
        .is_a()
        .iter()
        .any(|res_class| layer.is_subclass_of(res_class, class_iri))
}

/// Resolve a pattern-class `Name` to an IRI. `FullIri` passes through; a
/// `ShortName` resolves to a `core:Class` within the imported `namespaces`
/// (`USING NAMESPACE`) via the index-driven, namespace-scoped resolver —
/// never a whole-chain scan. Ambiguity (more than one match) is an error.
fn resolve_name(
    name: &Name,
    layer: &Layer,
    namespaces: &[String],
) -> Result<Option<Iri>, QueryError> {
    match name {
        Name::FullIri(iri) => Ok(Some(iri.clone())),
        Name::ShortName(s) => {
            crate::query::resolve::resolve_scoped_name(layer, namespaces, &[wk::CLASS], s)
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

    /// Subject-predicate pushdown (untyped `MATCH ?r {}` + a subject-bound WHERE):
    /// the optimized path must return EXACTLY the rows the full scan + WHERE would.
    #[test]
    fn subject_pushdown_equals_scan() {
        let layer = build_test_layer();
        let prefix = "urn:eigenius:example:";

        // Pull the bound subject IRI out of each RETURN row (single `{ iri: ?r }`).
        let row_iris = |rows: &[Resource]| -> std::collections::BTreeSet<String> {
            rows.iter()
                .filter_map(|r| {
                    r.properties()
                        .values()
                        .find_map(|v| v.as_iri().map(|i| i.as_str().to_string()))
                })
                .collect()
        };

        // Ground truth: scan everything (no constraint), filter by prefix client-side.
        let all = run_query(&layer, r#"MATCH ?r {} RETURN [] { iri: ?r }"#);
        let expected: std::collections::BTreeSet<String> = row_iris(&all)
            .into_iter()
            .filter(|i| i.starts_with(prefix))
            .collect();
        assert!(
            !expected.is_empty(),
            "fixture should have example: resources"
        );

        // LIKE-prefix pushdown — must equal the scan's prefix subset exactly.
        let liked = run_query(
            &layer,
            &format!(r#"MATCH ?r {{}} WHERE ?r LIKE "{prefix}%" RETURN [] {{ iri: ?r }}"#),
        );
        assert_eq!(row_iris(&liked), expected, "LIKE-prefix pushdown != scan");

        // Equality pushdown.
        let eq = run_query(
            &layer,
            r#"MATCH ?r {} WHERE ?r = "urn:eigenius:example:Dog" RETURN [] { iri: ?r }"#,
        );
        assert_eq!(eq.len(), 1);
        assert!(row_iris(&eq).contains("urn:eigenius:example:Dog"));

        // IN pushdown.
        let in_set = run_query(
            &layer,
            r#"MATCH ?r {} WHERE ?r IN ["urn:eigenius:example:Dog", "urn:eigenius:example:Animal"] RETURN [] { iri: ?r }"#,
        );
        assert_eq!(
            row_iris(&in_set),
            ["urn:eigenius:example:Animal", "urn:eigenius:example:Dog"]
                .iter()
                .map(|s| s.to_string())
                .collect()
        );

        // Non-prefix LIKE (suffix wildcard) is NOT pushed down — falls back to scan,
        // and must still be correct.
        let suffix = run_query(
            &layer,
            r#"MATCH ?r {} WHERE ?r LIKE "%:Dog" RETURN [] { iri: ?r }"#,
        );
        assert!(row_iris(&suffix).contains("urn:eigenius:example:Dog"));
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

    fn build_array_layer() -> Arc<Layer> {
        // team1 has members [a, b, c]; team0 has []. Members carry a name.
        let storage = crate::layer::LayerStorage::in_memory();
        let core_json = include_str!("../../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut core_builder = LayerBuilder::new("core", None);
        for r in core_resources {
            core_builder.add_resource(r).unwrap();
        }
        let core = Arc::new(core_builder.build(storage.clone()));
        let mut b = LayerBuilder::new("arr", Some(core));

        let iri = |s: &str| Iri::parse(s).unwrap();
        let sv = |s: &str| Value::String(s.into());
        for (id, nm) in [("a", "A"), ("b", "B"), ("c", "C")] {
            let mut m = Resource::new(iri(&format!("urn:eigenius:t:{id}")));
            m.set(iri("urn:eigenius:t:name"), sv(nm));
            b.add_resource(m).unwrap();
        }
        let mut team1 = Resource::new(iri("urn:eigenius:t:team1"));
        team1.set(
            iri("urn:eigenius:t:members"),
            Value::Array(vec![
                sv("urn:eigenius:t:a"),
                sv("urn:eigenius:t:b"),
                sv("urn:eigenius:t:c"),
            ]),
        );
        b.add_resource(team1).unwrap();
        let mut team0 = Resource::new(iri("urn:eigenius:t:team0"));
        team0.set(iri("urn:eigenius:t:members"), Value::Array(vec![]));
        b.add_resource(team0).unwrap();
        Arc::new(b.build(storage))
    }

    // D59 Item 2 — array element-iteration + cardinality patterns.

    fn array_query(q: &str) -> usize {
        let layer = build_array_layer();
        run_query(&layer, q).len()
    }

    #[test]
    fn array_each_iterates_elements() {
        // `[... ?m ...]` yields one binding per element: team1 → 3, team0 → 0.
        assert_eq!(
            array_query(
                r#"MATCH ?t { "urn:eigenius:t:members": [... ?m ...] } RETURN [] { "urn:eigenius:t:who": ?m }"#
            ),
            3
        );
    }

    #[test]
    fn array_exact_matches_cardinality() {
        // exactly three → team1 only; exactly two → neither (3 and 0).
        assert_eq!(
            array_query(
                r#"MATCH ?t { "urn:eigenius:t:members": [?x, ?y, ?z] } RETURN [] { "urn:eigenius:t:k": ?x }"#
            ),
            1
        );
        assert_eq!(
            array_query(
                r#"MATCH ?t { "urn:eigenius:t:members": [?x, ?y] } RETURN [] { "urn:eigenius:t:k": ?x }"#
            ),
            0
        );
    }

    #[test]
    fn array_empty_matches_empty_only() {
        // `[]` → only team0.
        assert_eq!(
            array_query(
                r#"MATCH ?t { "urn:eigenius:t:members": [] } RETURN [] { "urn:eigenius:t:t": ?t }"#
            ),
            1
        );
    }

    #[test]
    fn array_at_least_matches_prefix() {
        // `[?x, ?y, ...]` → team1 (3 ≥ 2); team0 (0 < 2) excluded.
        assert_eq!(
            array_query(
                r#"MATCH ?t { "urn:eigenius:t:members": [?x, ?y, ...] } RETURN [] { "urn:eigenius:t:k": ?x }"#
            ),
            1
        );
    }

    #[test]
    fn array_each_joins_to_element_resource() {
        // The Reachable shape: iterate elements, then join each back to its
        // resource's real property. team1's 3 members each resolve a name.
        assert_eq!(
            array_query(
                r#"MATCH ?t { "urn:eigenius:t:members": [... ?m ...] },
                         ?m { "urn:eigenius:t:name": ?n }
                   RETURN [] { "urn:eigenius:t:name": ?n }"#
            ),
            3
        );
    }

    // D59 Item 3 — the Reachable well-posedness check end to end: recursive
    // transitive closure over an array-valued `dep` edge (Item 2's `[... ?n ...]`)
    // through a derived-relation subject (Item 1's join), with a stratified-
    // negation `Unreachable` set. This is the D58 Reachable gate in miniature.
    fn build_objgraph_layer(include_orphan: bool) -> Arc<Layer> {
        let storage = crate::layer::LayerStorage::in_memory();
        let core_json = include_str!("../../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut core_builder = LayerBuilder::new("core", None);
        for r in core_resources {
            core_builder.add_resource(r).unwrap();
        }
        let core = Arc::new(core_builder.build(storage.clone()));
        let mut b = LayerBuilder::new("objgraph", Some(core));
        let iri = |s: &str| Iri::parse(s).unwrap();
        let sv = |s: &str| Value::String(s.into());
        let p = "urn:eigenius:t";

        // node(name) with a `dep` array edge and a `node` marker.
        let node = |id: &str, deps: Vec<&str>, bldr: &mut crate::layer::LayerBuilder| {
            let mut r = Resource::new(iri(&format!("{p}:{id}")));
            r.set(iri(&format!("{p}:node")), sv("y"));
            r.set(
                iri(&format!("{p}:dep")),
                Value::Array(deps.iter().map(|d| sv(&format!("{p}:{d}"))).collect()),
            );
            bldr.add_resource(r).unwrap();
        };
        node("thesis", vec!["m1", "m2"], &mut b);
        node("m1", vec!["ax"], &mut b);
        node("m2", vec![], &mut b);
        node("ax", vec![], &mut b);
        if include_orphan {
            node("orphan", vec![], &mut b); // a node nothing depends on
        }
        // The objective root carries the thesis pointer.
        let mut obj = Resource::new(iri(&format!("{p}:obj")));
        obj.set(iri(&format!("{p}:thesis")), sv(&format!("{p}:thesis")));
        b.add_resource(obj).unwrap();

        Arc::new(b.build(storage))
    }

    const REACHABLE_QUERY: &str = r#"
        DEFINE Reach(?t) FROM MATCH ?o { "urn:eigenius:t:thesis": ?t }
        DEFINE Reach(?n) FROM MATCH Reach(?m) { "urn:eigenius:t:dep": [... ?n ...] }
        DEFINE Node(?x) FROM MATCH ?x { "urn:eigenius:t:node": ?v }
        DEFINE Unreachable(?x) FROM MATCH Node(?x) {}, NOT Reach(?x) {}
        MATCH Unreachable(?x) {} RETURN [] { "urn:eigenius:t:x": ?x }
    "#;

    #[test]
    fn reachable_gate_well_posed_graph_has_no_unreachable() {
        let layer = build_objgraph_layer(false);
        assert_eq!(
            run_query(&layer, REACHABLE_QUERY).len(),
            0,
            "fully-connected graph should have no unreachable nodes"
        );
    }

    #[test]
    fn reachable_gate_flags_orphan() {
        let layer = build_objgraph_layer(true);
        // Only the orphan is unreachable from the thesis.
        assert_eq!(
            run_query(&layer, REACHABLE_QUERY).len(),
            1,
            "the disconnected orphan node must be flagged unreachable"
        );
    }

    #[test]
    fn derived_subject_from_resource_ref_property_resolves() {
        // Regression: a derived relation whose subject column comes from a
        // resource-VALUED property stored as `Value::ResourceRef` (the
        // chain-canonicalised shape) must resolve — not only `Value::String`
        // subject bindings. Reproduces the live `Reach(?t) FROM Objective {
        // thesis: ?t }` failure where `thesis` is a canonicalised ResourceRef.
        let storage = crate::layer::LayerStorage::in_memory();
        let core_json = include_str!("../../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut cb = LayerBuilder::new("core", None);
        for r in core_resources {
            cb.add_resource(r).unwrap();
        }
        let core = Arc::new(cb.build(storage.clone()));
        let mut b = LayerBuilder::new("rr", Some(core));
        let iri = |s: &str| Iri::parse(s).unwrap();
        let mut target = Resource::new(iri("urn:eigenius:t:target"));
        target.set(iri("urn:eigenius:t:name"), Value::String("T".into()));
        b.add_resource(target).unwrap();
        let mut root = Resource::new(iri("urn:eigenius:t:root"));
        // stored as a ResourceRef — the canonicalised shape a strict
        // Value::String match would drop.
        root.set(
            iri("urn:eigenius:t:points_to"),
            Value::ResourceRef(iri("urn:eigenius:t:target")),
        );
        b.add_resource(root).unwrap();
        let layer = Arc::new(b.build(storage));
        let results = run_query(
            &layer,
            r#"
            DEFINE Reach(?t) FROM MATCH ?r { "urn:eigenius:t:points_to": ?t }
            MATCH Reach(?x) { "urn:eigenius:t:name": ?n }
            RETURN [] { "urn:eigenius:t:name": ?n }
            "#,
        );
        assert_eq!(
            results.len(),
            1,
            "derived subject from a ResourceRef-valued property must resolve"
        );
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

    // D59 Item 1 — derived-relation binding/join. The three tests below would
    // each have failed before the positional-head-projection + resolve-subject
    // fix (unbound projection, empty/cross-product join, broken recursion).

    #[test]
    fn derived_subject_is_bound_and_projectable() {
        // A variable bound via a DEFINE relation must carry into the query and be
        // RETURN-able. Pre-fix this errored "unbound variable ?p" (run_query
        // would panic).
        let layer = build_hierarchy_layer();
        let results = run_query(
            &layer,
            r#"
            DEFINE Mgr(?x) FROM MATCH ?x { "urn:eigenius:test:reports_to": ?z }
            MATCH Mgr(?p) {}
            RETURN [] { "urn:eigenius:test:who": ?p }
            "#,
        );
        // Alice and Bob report to someone; Charlie does not.
        assert_eq!(
            results.len(),
            2,
            "expected the two reporters, got {}",
            results.len()
        );
    }

    #[test]
    fn derived_join_refinement_no_cross_product() {
        // A brace refinement on a derived-relation subject must match the REAL
        // resource's properties and equi-join on the bound subject — not
        // cross-product (the bug produced N× rows) and not return empty (the
        // pseudo-resource bug).
        let layer = build_hierarchy_layer();
        let results = run_query(
            &layer,
            r#"
            DEFINE Mgr(?x) FROM MATCH ?x { "urn:eigenius:test:reports_to": ?z }
            MATCH Mgr(?p) { "urn:eigenius:test:name": ?n }
            RETURN [] { "urn:eigenius:test:name": ?n }
            "#,
        );
        assert_eq!(
            results.len(),
            2,
            "expected exactly Alice+Bob, got {}",
            results.len()
        );
    }

    #[test]
    fn recursive_reach_single_arg_closure() {
        // The Reachable shape: a 1-arg relation whose recursive step refines the
        // derived subject through a real property. From Alice: direct = Bob,
        // transitive = Charlie. Exercises recursion + derived-subject join.
        let layer = build_hierarchy_layer();
        let results = run_query(
            &layer,
            r#"
            DEFINE Reach(?n) FROM
                MATCH ?s { "urn:eigenius:test:reports_to": ?n }
                WHERE ?s = "urn:eigenius:test:alice"
            DEFINE Reach(?n) FROM
                MATCH Reach(?m) { "urn:eigenius:test:reports_to": ?n }
            MATCH Reach(?p) { "urn:eigenius:test:name": ?nm }
            RETURN [] { "urn:eigenius:test:name": ?nm }
            "#,
        );
        // Reach(alice) = { bob, charlie }
        assert_eq!(
            results.len(),
            2,
            "expected Bob+Charlie reachable from Alice, got {}",
            results.len()
        );
    }
}
