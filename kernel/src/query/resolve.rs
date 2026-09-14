//! Namespace-scoped short-name resolution for EigenQL.
//!
//! A bare short name (a `Name::ShortName` — a class, property, or query-class
//! reference written without a full IRI) resolves against the vocabulary of the
//! namespaces the query *imports* via `USING NAMESPACE "<prefix>"`, **not** against
//! the whole knowledge graph. The **core namespace**
//! ([`wk::CORE_NAMESPACE`](crate::ontology::well_known::CORE_NAMESPACE)) is always
//! implicitly imported — core is the root layer on every chain, so its vocabulary
//! (`Class`, `Property`, `short_name`, `domain`, …) is the platform prelude and needs
//! no explicit `USING NAMESPACE`. This is both a correctness and a scaling property:
//!
//! - **Correctness** — resolution is scoped to declared vocabulary, so a short name
//!   can't accidentally bind to an unrelated resource elsewhere on the chain; a name
//!   that matches more than one imported-namespace resource is an *ambiguity error*
//!   rather than a silent first-wins pick.
//! - **Scaling** — discovery is index-driven ([`typed_resource_iris`]) and the
//!   candidate IRIs are filtered by namespace prefix *before* any body is resolved, so
//!   cost is O(imported-namespace vocab), independent of chain size. The old path
//!   `iter_all_resources()`-scanned the entire chain per short-name reference — seconds
//!   on a large chain (UMLS ≈ 281k resources). A global `short_name` value index was
//!   explicitly rejected: it would index 281k UMLS CUIs and invite false matches.
//!
//! See `docs/notes/chain-scaling-audit.md` (short-name resolution section).

use crate::institution::registry::InstitutionIndex;
use crate::layer::{typed_resource_iris, Layer};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Value;
use crate::ontology::well_known as wk;
use crate::query::ast::{Clause, Expression, MatchPart, Name, Program};
use crate::query::error::QueryError;
use std::collections::{BTreeMap, BTreeSet};

/// Is `iri` inside the implicit core namespace or one of the imported namespace
/// prefixes? A namespace is matched by simple IRI-string prefix (e.g. prefix
/// `urn:eigenius:core:` matches `urn:eigenius:core:Class`). The core namespace is
/// always in scope (the prelude); other prefixes come verbatim from `USING NAMESPACE`.
fn in_namespace(iri: &Iri, namespaces: &[String]) -> bool {
    let s = iri.as_str();
    s.starts_with(wk::CORE_NAMESPACE) || namespaces.iter().any(|ns| s.starts_with(ns.as_str()))
}

/// Resolve a bare short name to a unique IRI within the imported `namespaces`.
///
/// `metaclasses` restricts the candidate set by `is_a` (e.g. `core:Class` for a pattern
/// class, `core:Property` for a property, `institution:QueryClass` for a FIBER query
/// class). Returns:
/// - `Ok(Some(iri))` — exactly one imported-namespace resource of the given metaclass(es)
///   carries `short_name == short`.
/// - `Ok(None)` — no such resource (caller decides whether that is an error in context).
/// - `Err(..)` — more than one match: an ambiguity the user must disambiguate (use a full
///   IRI, or import fewer namespaces).
pub(crate) fn resolve_scoped_name(
    layer: &Layer,
    namespaces: &[String],
    metaclasses: &[&str],
    short: &str,
) -> Result<Option<Iri>, QueryError> {
    // Core is always in scope (the prelude), so an empty `namespaces` still resolves
    // core vocabulary; only non-core short names need an explicit `USING NAMESPACE`.
    let Ok(short_prop) = Iri::parse(wk::SHORT_NAME) else {
        return Ok(None);
    };

    let mut matches: Vec<Iri> = Vec::new();
    for iri in typed_resource_iris(layer, metaclasses) {
        if !in_namespace(&iri, namespaces) {
            continue;
        }
        // Resolve through the head: merged top view + filters to this chain.
        let Some(res) = layer.resolve(&iri) else {
            continue;
        };
        if let Some(Value::String(sn)) = res.get(&short_prop) {
            if sn == short {
                matches.push(iri);
            }
        }
    }

    match matches.len() {
        0 => Ok(None),
        1 => Ok(Some(matches.into_iter().next().unwrap())),
        _ => {
            matches.sort();
            Err(QueryError::type_check(
                "ambiguous_short_name",
                format!(
                    "short name '{short}' resolves to {} resources in the imported namespaces \
                     ({}); use a full IRI or import fewer namespaces",
                    matches.len(),
                    matches
                        .iter()
                        .map(|i| i.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::{Layer, LayerBuilder};
    use crate::ontology::resource::Resource;
    use std::sync::Arc;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    /// A `core:Class` resource at `id` with the given `short_name`.
    fn class_with_short_name(id: &str, short: &str) -> Resource {
        let mut r = Resource::new(iri(id));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::iri(&iri(wk::CLASS))]),
        );
        r.set(iri(wk::SHORT_NAME), Value::String(short.into()));
        r
    }

    /// A layer on the bootstrap core with two same-short-name `Widget` classes in
    /// two distinct namespaces plus one `Gadget` in `urn:a:` only.
    fn layer_with_classes() -> Arc<Layer> {
        let ctx = crate::testing::bootstrap_context();
        let head = Arc::clone(ctx.head());
        let storage = head.storage().clone();
        let mut b = LayerBuilder::new("vocab", Some(head));
        b.add_resource(class_with_short_name("urn:a:Widget", "Widget"))
            .unwrap();
        b.add_resource(class_with_short_name("urn:b:Widget", "Widget"))
            .unwrap();
        b.add_resource(class_with_short_name("urn:a:Gadget", "Gadget"))
            .unwrap();
        Arc::new(b.build(storage))
    }

    #[test]
    fn resolves_unique_short_name_in_imported_namespace() {
        let layer = layer_with_classes();
        let got = resolve_scoped_name(&layer, &["urn:a:".into()], &[wk::CLASS], "Gadget")
            .expect("no ambiguity");
        assert_eq!(got, Some(iri("urn:a:Gadget")));
    }

    #[test]
    fn unimported_namespace_fails_closed() {
        let layer = layer_with_classes();
        // `Gadget` lives in `urn:a:`; with nothing imported (core only), it does not
        // resolve — fail closed rather than scan the whole graph.
        let got = resolve_scoped_name(&layer, &[], &[wk::CLASS], "Gadget").expect("no ambiguity");
        assert_eq!(got, None);
    }

    #[test]
    fn ambiguous_short_name_across_imported_namespaces_errors() {
        let layer = layer_with_classes();
        let err = resolve_scoped_name(
            &layer,
            &["urn:a:".into(), "urn:b:".into()],
            &[wk::CLASS],
            "Widget",
        )
        .expect_err("two Widgets across imported namespaces must be ambiguous");
        assert_eq!(err.rule, "ambiguous_short_name");
    }

    #[test]
    fn single_namespace_disambiguates_collision() {
        let layer = layer_with_classes();
        // Importing only `urn:a:` narrows the otherwise-ambiguous `Widget` to one.
        let got = resolve_scoped_name(&layer, &["urn:a:".into()], &[wk::CLASS], "Widget")
            .expect("scoped to one namespace");
        assert_eq!(got, Some(iri("urn:a:Widget")));
    }

    #[test]
    fn core_namespace_is_implicitly_imported() {
        let layer = layer_with_classes();
        // `Class` is core vocabulary; it resolves with NO explicit USING NAMESPACE.
        let got = resolve_scoped_name(&layer, &[], &[wk::CLASS], "Class").expect("no ambiguity");
        assert_eq!(got, Some(iri(wk::CLASS)));
    }
}

// ─── Property-name resolution ────────────────────────────────────────

/// Resolve every property name the program writes — `MATCH` brace keys and
/// dot-path segments — to the property IRI it names, rewriting the AST in place.
///
/// **Why the AST is rewritten rather than re-resolved at evaluation.** A short name
/// is matched against a declared property's `core:short_name`; the evaluator used to
/// match it against the *local name* of whatever IRIs the resource happened to carry.
/// Two mechanisms for one name, and they disagree in both directions: a property
/// declared `urn:x:title_text` with `short_name "title"` type-checked and then matched
/// nothing, and a resource carrying an unrelated `urn:other:title` answered to a name
/// that meant `urn:x:title_text`. Both were silent. Resolving once, here, leaves the
/// evaluator a map lookup and no second opinion.
///
/// **Scope** (the rule, in full): a short name is admitted when the class is known or it
/// is derivable from a namespace declaration. The class scope is the pattern's declared
/// class — or, for a `FIBER … AS ?b` binding, the QueryClass output contract D90 closed;
/// the namespace scope is `USING NAMESPACE` plus the implicit core prelude. The class is
/// the narrower scope and answers first. A name in neither is an error rather than an
/// empty result set, and a full IRI — a quoted segment or key — reaches whatever neither
/// scope covers.
pub(crate) fn resolve_property_names(
    program: &mut Program,
    layer: &Layer,
    index: &InstitutionIndex,
    relation_names: &BTreeSet<String>,
) -> Vec<QueryError> {
    let mut errors = Vec::new();
    for def in &mut program.definitions {
        let scope = subject_vocabularies(&def.body, layer, index, relation_names);
        resolve_part(&mut def.body, &scope, layer, &mut errors);
    }
    let scope = subject_vocabularies(&program.query.body, layer, index, relation_names);
    let namespaces = program.query.body.using_namespaces.clone();
    resolve_part(&mut program.query.body, &scope, layer, &mut errors);
    // `GROUP BY`, `RETURN` and `ORDER BY` read variables the query body bound, so they
    // resolve against the query body's scope.
    for expr in &mut program.query.group_by {
        resolve_in_expression(expr, &scope, layer, &namespaces, &mut errors);
    }
    for item in &mut program.query.result {
        resolve_in_expression(
            &mut item.expression,
            &scope,
            layer,
            &namespaces,
            &mut errors,
        );
    }
    for item in &mut program.query.order_by {
        resolve_in_expression(
            &mut item.expression,
            &scope,
            layer,
            &namespaces,
            &mut errors,
        );
    }
    errors
}

/// The vocabulary a short name may be drawn from, for one pattern subject or dot-path
/// root.
#[derive(Default, Clone)]
struct Vocabulary {
    /// What put these properties in scope, named as the chain names it — for the error
    /// message when a short name is in none of them.
    sources: Vec<String>,
    properties: BTreeSet<Iri>,
}

impl Vocabulary {
    fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    fn add_class(&mut self, class: &Iri, layer: &Layer) {
        self.sources.push(class.as_str().to_string());
        self.properties.extend(layer.declared_properties(class));
    }
}

/// The vocabulary each pattern subject and FIBER binding carries.
///
/// A variable bound by two classed patterns carries both — resolution searches the
/// union, so `MATCH ?x Dog { … }, ?x Pet { … }` reaches either class's properties. A
/// `DEFINE` relation name is not a class and contributes nothing; neither does a class
/// that fails to resolve, whose dangling reference `check_match_part` reports on its own.
fn subject_vocabularies(
    part: &MatchPart,
    layer: &Layer,
    index: &InstitutionIndex,
    relation_names: &BTreeSet<String>,
) -> BTreeMap<String, Vocabulary> {
    let mut out: BTreeMap<String, Vocabulary> = BTreeMap::new();
    for clause in &part.clauses {
        match clause {
            Clause::Pattern(p) => {
                let Some(name) = &p.class else { continue };
                let iri = match name {
                    Name::FullIri(iri) => Some(iri.clone()),
                    Name::ShortName(s) if relation_names.contains(s) => None,
                    Name::ShortName(s) => {
                        resolve_scoped_name(layer, &part.using_namespaces, &[wk::CLASS], s)
                            .ok()
                            .flatten()
                    }
                };
                if let Some(iri) = iri {
                    out.entry(p.subject.name.clone())
                        .or_default()
                        .add_class(&iri, layer);
                }
            }
            Clause::Fiber(fc) => {
                // The binding's vocabulary is the QueryClass's output contract, which D90
                // closed and the kernel enforces at the dispatch boundary: what
                // `result_class` declares, plus whatever `result_properties` adds. D2 §5.8
                // step 10 called a non-Verdict response untyped for this purpose; it is
                // typed now, by a contract that is checked.
                let qc_iri = match &fc.query_class {
                    Name::FullIri(iri) => Some(iri.clone()),
                    Name::ShortName(s) => resolve_scoped_name(
                        layer,
                        &part.using_namespaces,
                        &[wk::QUERY_CLASS_CLASS],
                        s,
                    )
                    .ok()
                    .flatten(),
                };
                if let Some(entry) = qc_iri.as_ref().and_then(|i| index.query_class(i)) {
                    let vocab = out.entry(fc.binding.name.clone()).or_default();
                    vocab.add_class(&entry.result_class, layer);
                    vocab
                        .properties
                        .extend(entry.result_properties.iter().cloned());
                }
            }
        }
    }
    out
}

/// Brace keys in every pattern, then dot-paths in every condition.
fn resolve_part(
    part: &mut MatchPart,
    scope: &BTreeMap<String, Vocabulary>,
    layer: &Layer,
    errors: &mut Vec<QueryError>,
) {
    let namespaces = part.using_namespaces.clone();
    for clause in &mut part.clauses {
        let Clause::Pattern(p) = clause else { continue };
        let vocab = scope.get(&p.subject.name).cloned().unwrap_or_default();
        for pp in &mut p.properties {
            if let Err(e) = resolve_name_in_place(&mut pp.property, &vocab, layer, &namespaces) {
                errors.push(e);
            }
        }
    }
    for expr in &mut part.conditions {
        resolve_in_expression(expr, scope, layer, &namespaces, errors);
    }
}

/// Every dot-path reachable from `expr`.
fn resolve_in_expression(
    expr: &mut Expression,
    scope: &BTreeMap<String, Vocabulary>,
    layer: &Layer,
    namespaces: &[String],
    errors: &mut Vec<QueryError>,
) {
    match expr {
        Expression::DotPath { root, segments } => {
            let mut vocab = scope.get(&root.name).cloned().unwrap_or_default();
            for segment in segments.iter_mut() {
                match resolve_name_in_place(segment, &vocab, layer, namespaces) {
                    // The next segment is scoped by this property's declared range.
                    Ok(()) => vocab = range_vocabulary(segment, layer),
                    Err(e) => {
                        errors.push(e);
                        return;
                    }
                }
            }
        }
        Expression::Binary { left, right, .. } => {
            resolve_in_expression(left, scope, layer, namespaces, errors);
            resolve_in_expression(right, scope, layer, namespaces, errors);
        }
        Expression::Unary { operand, .. }
        | Expression::VerdictPredicate { operand, .. }
        | Expression::NotExists(operand)
        | Expression::Aggregate { arg: operand, .. } => {
            resolve_in_expression(operand, scope, layer, namespaces, errors);
        }
        Expression::FunctionCall { args, .. } => {
            for arg in args {
                resolve_in_expression(arg, scope, layer, namespaces, errors);
            }
        }
        Expression::Array(elements) => {
            for elem in elements {
                resolve_in_expression(elem, scope, layer, namespaces, errors);
            }
        }
        Expression::Object(pairs) => {
            for (_, v) in pairs {
                resolve_in_expression(v, scope, layer, namespaces, errors);
            }
        }
        Expression::Similarity { query, .. } => {
            resolve_in_expression(query, scope, layer, namespaces, errors);
        }
        Expression::Literal(_) | Expression::Variable(_) => {}
    }
}

/// Rewrite one property name to the IRI it names. A `FullIri` is already resolved.
fn resolve_name_in_place(
    name: &mut Name,
    vocab: &Vocabulary,
    layer: &Layer,
    namespaces: &[String],
) -> Result<(), QueryError> {
    let Name::ShortName(short) = name else {
        return Ok(());
    };
    let resolved = match scoped_property(short, vocab, layer)? {
        Some(iri) => Some(iri),
        None => resolve_scoped_name(layer, namespaces, &[wk::PROPERTY], short)?,
    };
    match resolved {
        Some(iri) => {
            *name = Name::FullIri(iri);
            Ok(())
        }
        None => Err(QueryError::type_check(
            "property_name_unresolved",
            if vocab.is_empty() {
                format!(
                    "property '{short}' does not resolve to a declared property in the imported \
                     namespaces, and nothing in scope declares it — add `USING NAMESPACE`, give \
                     the pattern a class, or write the full property IRI"
                )
            } else {
                format!(
                    "property '{short}' is declared by neither {} nor the imported namespaces — \
                     write the full property IRI to reach a property the class does not declare",
                    vocab
                        .sources
                        .iter()
                        .map(|c| format!("'{c}'"))
                        .collect::<Vec<_>>()
                        .join(" nor ")
                )
            },
        )),
    }
}

/// The one property `short` names in `vocab`, if any.
fn scoped_property(
    short: &str,
    vocab: &Vocabulary,
    layer: &Layer,
) -> Result<Option<Iri>, QueryError> {
    let short_prop = wk::iri(wk::SHORT_NAME);
    let mut matches: BTreeSet<Iri> = BTreeSet::new();
    for prop in &vocab.properties {
        let Some(res) = layer.resolve(prop) else {
            continue;
        };
        if let Some(Value::String(sn)) = res.get(&short_prop) {
            if sn == short {
                matches.insert(prop.clone());
            }
        }
    }
    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.into_iter().next()),
        _ => Err(QueryError::type_check(
            "ambiguous_short_name",
            format!(
                "short name '{short}' names {} distinct properties in the classes in scope ({}); \
                 write the full property IRI",
                matches.len(),
                matches
                    .iter()
                    .map(|i| i.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
    }
}

/// A property's declared range (`core:class_types`) — the vocabulary for the next
/// dot-path segment. Empty where the property declares none, which drops the next
/// segment to the namespace scope.
fn range_vocabulary(name: &Name, layer: &Layer) -> Vocabulary {
    let mut vocab = Vocabulary::default();
    let Name::FullIri(iri) = name else {
        return vocab;
    };
    let Some(res) = layer.resolve(iri) else {
        return vocab;
    };
    if let Some(v) = res.get(&wk::iri(wk::CLASS_TYPES)) {
        for class in v.as_iri_array() {
            vocab.add_class(&class, layer);
        }
    }
    vocab
}
