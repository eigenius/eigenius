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
use crate::query::ast::{
    ClassRef, Clause, Expression, FiberClause, MatchPart, Name, OrderItem, ParamBinding,
    ParamValue, Parsed, Pattern, Program, PropertyPattern, Query, RelationId, Resolved, ReturnItem,
    RuleDefinition,
};
use crate::query::error::QueryError;
use std::collections::{BTreeMap, BTreeSet};

/// Is `iri` inside one of the explicitly imported namespace prefixes? A namespace is
/// matched by simple IRI-string prefix, verbatim from `USING NAMESPACE`.
fn in_imported_namespace(iri: &Iri, namespaces: &[String]) -> bool {
    let s = iri.as_str();
    namespaces.iter().any(|ns| s.starts_with(ns.as_str()))
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

    // **An explicit import shadows the implicit prelude.** Core and the `USING NAMESPACE`
    // prefixes used to be one flat candidate pool, so a name declared in both was an
    // ambiguity the query could not resolve: the error told the reader to import fewer
    // namespaces, and core cannot be un-imported. Five shipped ontologies collide with
    // core this way — `schema_org:description`, `notebook:description`,
    // `ingest:data_type`, `ingest:content_encoding` and `julia:intervals:domain` — so
    // `USING NAMESPACE "urn:schema_org:"` plus a `description` key was unanswerable.
    //
    // Naming a namespace is a statement about which vocabulary the query means, so it
    // wins over the prelude, the way an inner scope wins over an outer one. Ambiguity
    // WITHIN the imported set is still an error: those the reader can act on, by
    // importing fewer or writing the IRI.
    let mut imported: Vec<Iri> = Vec::new();
    let mut prelude: Vec<Iri> = Vec::new();
    for iri in typed_resource_iris(layer, metaclasses) {
        let is_imported = in_imported_namespace(&iri, namespaces);
        if !is_imported && !iri.as_str().starts_with(wk::CORE_NAMESPACE) {
            continue;
        }
        // Resolve through the head: merged top view + filters to this chain.
        let Some(res) = layer.resolve(&iri) else {
            continue;
        };
        if let Some(Value::String(sn)) = res.get(&short_prop) {
            if sn == short {
                if is_imported {
                    imported.push(iri);
                } else {
                    prelude.push(iri);
                }
            }
        }
    }

    let mut matches = if imported.is_empty() {
        prelude
    } else {
        imported
    };

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

// ─── Resolution: Program<Parsed> → Program<Resolved> (D92) ───────────

/// Resolve every reference a program writes, producing the stage the rest of the
/// pipeline consumes.
///
/// **A total function is the design.** Every parameterised position must be given a
/// resolved reference for the result to exist, so a position this pass forgets is a
/// compile error rather than a value that reaches evaluation unresolved. The defect that
/// motivated D92 was exactly that: a `match` that handled `Clause::Pattern` and let
/// `Clause::Fiber` fall through an `else { continue }`, leaving a dot-path inside a FIBER
/// param with its short names, type-checking clean, and failing at evaluation with a
/// message about this pass.
///
/// Errors accumulate rather than short-circuiting, so one run reports every unresolvable
/// name rather than the first.
pub struct ResolvedProgram {
    pub program: Program<Resolved>,
    /// What `stratify` worked out, carried rather than recomputed.
    ///
    /// **It travels with the program because it is only true OF that program.** An
    /// earlier version of this passed the strata to `evaluate` as a separate argument,
    /// which let the two disagree: handed an empty or foreign stratum list, the fixpoint
    /// loop never ran, `derived` stayed empty, and a `MATCH Rel(?x)` fell through to an
    /// untyped match-all over the whole chain — 1350 rows where 2 were right, with no
    /// error. That is the same shape D92 removes from name resolution, reintroduced one
    /// line below where the duplicate `stratify` call was deleted. A pair that cannot be
    /// mismatched is the fix; checking that it matches would be the guard.
    pub strata: Vec<crate::query::stratify::Stratum>,
    /// One id per distinct relation, by first appearance — see [`relation_ids`]. Carried
    /// for the same reason: `evaluate` keys derived facts by it and must agree with the
    /// ids `resolve` put in the `ClassRef`s.
    pub relation_ids: BTreeMap<String, RelationId>,
}

pub fn resolve(
    program: Program<Parsed>,
    strata: Vec<crate::query::stratify::Stratum>,
    layer: &Layer,
    index: &InstitutionIndex,
) -> Result<ResolvedProgram, Vec<QueryError>> {
    // One id per relation, assigned by first appearance. `stratify` has already run, so
    // the definition set is checked as well as fixed.
    let relations = crate::query::ast::relation_ids(&program.definitions);
    let relations_out = relations.clone();

    let mut errors = Vec::new();
    let definitions: Vec<Option<RuleDefinition<Resolved>>> = program
        .definitions
        .into_iter()
        .map(|d| {
            let body = resolve_part(d.body, layer, index, &relations, &mut errors);
            body.map(|body| RuleDefinition {
                name: d.name,
                variables: d.variables,
                body,
            })
        })
        .collect();

    let query = resolve_query(program.query, layer, index, &relations, &mut errors);

    if !errors.is_empty() {
        return Err(errors);
    }
    let definitions: Option<Vec<_>> = definitions.into_iter().collect();
    match (definitions, query) {
        (Some(definitions), Some(query)) => Ok(ResolvedProgram {
            program: Program { definitions, query },
            strata,
            relation_ids: relations_out,
        }),
        // Unreachable: every `None` above pushes an error, and the empty-error case
        // returned already. Reported rather than panicked, because "unreachable" is what
        // the guards this design removes also claimed.
        _ => Err(vec![QueryError::type_check(
            "resolution_incomplete",
            "a reference failed to resolve without reporting why",
        )]),
    }
}

/// Collect every element, then decide — rather than `Option`'s `FromIterator`, which
/// stops at the first `None`.
///
/// The difference is the number of errors one run reports. Each resolver pushes its own
/// error before returning `None`, so short-circuiting here loses every error after the
/// first: `MATCH C(?n) { nope_one: ?a, nope_two: ?b }` reported two before D92 and one
/// after, until this was fixed.
fn all<T>(items: impl IntoIterator<Item = Option<T>>) -> Option<Vec<T>> {
    let collected: Vec<Option<T>> = items.into_iter().collect();
    collected.into_iter().collect()
}

fn resolve_query(
    query: Query<Parsed>,
    layer: &Layer,
    index: &InstitutionIndex,
    relations: &BTreeMap<String, RelationId>,
    errors: &mut Vec<QueryError>,
) -> Option<Query<Resolved>> {
    let namespaces = query.body.using_namespaces.clone();
    let scope = build_scope(&query.body, layer, index, relations, errors);
    let body = resolve_part_with_scope(query.body, &scope, layer, index, errors);

    // `GROUP BY`, `RETURN` and `ORDER BY` read variables the query body bound, so they
    // resolve against the query body's scope.
    let group_by = all(query
        .group_by
        .into_iter()
        .map(|e| resolve_expression(e, &scope, layer, &namespaces, errors)));
    let result = all(query.result.into_iter().map(|item| {
        resolve_expression(item.expression, &scope, layer, &namespaces, errors).map(|expression| {
            ReturnItem {
                name: item.name,
                expression,
            }
        })
    }));
    let order_by = all(query.order_by.into_iter().map(|item| {
        resolve_expression(item.expression, &scope, layer, &namespaces, errors).map(|expression| {
            OrderItem {
                expression,
                direction: item.direction,
            }
        })
    }));

    // **`RETURN Class [] { … }` is a chain reference and is now checked.** It is stamped
    // as `is_a` on every result row, so a class the chain does not declare produces rows
    // asserting membership of nothing. Nothing asked this before D92.
    let result_classes = all(query.result_classes.into_iter().map(|name| {
        resolve_chain_name(
            &name,
            &[wk::CLASS],
            "result class",
            layer,
            &namespaces,
            errors,
        )
    }));

    Some(Query {
        body: body?,
        group_by: group_by?,
        result_classes: result_classes?,
        result: result?,
        order_by: order_by?,
        limit: query.limit,
        offset: query.offset,
        distinct: query.distinct,
        top: query.top,
    })
}

fn resolve_part(
    part: MatchPart<Parsed>,
    layer: &Layer,
    index: &InstitutionIndex,
    relations: &BTreeMap<String, RelationId>,
    errors: &mut Vec<QueryError>,
) -> Option<MatchPart<Resolved>> {
    let scope = build_scope(&part, layer, index, relations, errors);
    resolve_part_with_scope(part, &scope, layer, index, errors)
}

fn resolve_part_with_scope(
    part: MatchPart<Parsed>,
    scope: &Scope,
    layer: &Layer,
    index: &InstitutionIndex,
    errors: &mut Vec<QueryError>,
) -> Option<MatchPart<Resolved>> {
    let namespaces = part.using_namespaces.clone();
    // `USING INSTITUTION "<iri>" AS alias` — the alias table a FIBER clause resolves its
    // institution through.
    let aliases: BTreeMap<String, Iri> = part
        .using_institutions
        .iter()
        .map(|a| (a.alias.clone(), a.iri.clone()))
        .collect();

    let clauses = all(part
        .clauses
        .into_iter()
        .enumerate()
        .map(|(i, clause)| match clause {
            Clause::Pattern(p) => {
                resolve_pattern(p, i, scope, layer, &namespaces, errors).map(Clause::Pattern)
            }
            Clause::Fiber(fc) => {
                resolve_fiber(fc, i, scope, layer, index, &namespaces, &aliases, errors)
                    .map(Clause::Fiber)
            }
        }));
    let conditions = all(part
        .conditions
        .into_iter()
        .map(|e| resolve_expression(e, scope, layer, &namespaces, errors)));

    Some(MatchPart {
        using: part.using,
        using_institutions: part.using_institutions,
        using_namespaces: part.using_namespaces,
        clauses: clauses?,
        conditions: conditions?,
    })
}

fn resolve_pattern(
    pattern: Pattern<Parsed>,
    clause_index: usize,
    scope: &Scope,
    layer: &Layer,
    namespaces: &[String],
    errors: &mut Vec<QueryError>,
) -> Option<Pattern<Resolved>> {
    let vocab = scope
        .vocabularies
        .get(&pattern.subject.name)
        .cloned()
        .unwrap_or_default();
    // Read, not resolve: `build_scope` resolved this class and reported it if it could
    // not. A pattern that states a class the scope pass could not resolve has no entry,
    // and its error is already recorded.
    let class = match pattern.class {
        None => None,
        Some(_) => Some(scope.classes.get(&clause_index)?.clone()),
    };
    let properties = all(pattern.properties.into_iter().map(|pp| {
        resolve_property(&pp.property, &vocab, layer, namespaces, errors).map(|property| {
            PropertyPattern {
                property,
                object: pp.object,
            }
        })
    }));
    Some(Pattern {
        subject: pattern.subject,
        class,
        properties: properties?,
        negated: pattern.negated,
    })
}

/// A pattern class names a chain class or a `DEFINE` relation. The relation case was an
/// exemption inside `check_match_part`; here it is the answer.
fn resolve_pattern_class(
    name: &Name,
    layer: &Layer,
    namespaces: &[String],
    relations: &BTreeMap<String, RelationId>,
    errors: &mut Vec<QueryError>,
) -> Option<ClassRef> {
    match name {
        Name::FullIri(iri) => Some(ClassRef::Chain(iri.clone())),
        Name::ShortName(s) => {
            if let Some(id) = relations.get(s) {
                return Some(ClassRef::Relation(*id));
            }
            match resolve_scoped_name(layer, namespaces, &[wk::CLASS], s) {
                Ok(Some(iri)) => Some(ClassRef::Chain(iri)),
                Ok(None) => {
                    errors.push(QueryError::type_check(
                        "unknown_class",
                        format!(
                            "pattern class '{s}' does not resolve to a Class in the core namespace \
                             or any USING NAMESPACE, and names no DEFINE relation; add \
                             `USING NAMESPACE \"<prefix>\"` or use a full IRI"
                        ),
                    ));
                    None
                }
                Err(e) => {
                    errors.push(e);
                    None
                }
            }
        }
    }
}

/// Any reference that resolves to the IRI of a declared resource of one of `metaclasses`.
fn resolve_chain_name(
    name: &Name,
    metaclasses: &[&str],
    what: &str,
    layer: &Layer,
    namespaces: &[String],
    errors: &mut Vec<QueryError>,
) -> Option<Iri> {
    match name {
        Name::FullIri(iri) => Some(iri.clone()),
        Name::ShortName(s) => match resolve_scoped_name(layer, namespaces, metaclasses, s) {
            Ok(Some(iri)) => Some(iri),
            Ok(None) => {
                errors.push(QueryError::type_check(
                    "unresolved_reference",
                    format!(
                        "{what} '{s}' does not resolve in the core namespace or any \
                         USING NAMESPACE; add `USING NAMESPACE \"<prefix>\"` or use a full IRI"
                    ),
                ));
                None
            }
            Err(e) => {
                errors.push(e);
                None
            }
        },
    }
}

/// A FIBER clause names an institution (through an alias or inline), a query class, and a
/// param per declared property of that query class's input class.
#[allow(clippy::too_many_arguments)]
fn resolve_fiber(
    fc: FiberClause<Parsed>,
    clause_index: usize,
    scope: &Scope,
    layer: &Layer,
    index: &InstitutionIndex,
    namespaces: &[String],
    aliases: &BTreeMap<String, Iri>,
    errors: &mut Vec<QueryError>,
) -> Option<FiberClause<Resolved>> {
    // The institution is an alias bound by `USING INSTITUTION`, or an inline IRI.
    let institution = match &fc.institution {
        Name::FullIri(iri) => Some(iri.clone()),
        Name::ShortName(alias) => match aliases.get(alias) {
            Some(iri) => Some(iri.clone()),
            None => {
                errors.push(QueryError::type_check(
                    "undeclared_institution_alias",
                    format!(
                        "FIBER refers to undeclared institution alias '{alias}' — add \
                         `USING INSTITUTION \"...\" AS {alias}`, or use an inline IRI"
                    ),
                ));
                None
            }
        },
    };

    // Read, not resolve — `build_scope` already did, and reported.
    let query_class = scope.query_classes.get(&clause_index).cloned();

    // A param names a declared property of the QueryClass's INPUT class, which is a
    // vocabulary no pattern puts in scope — so it is resolved here rather than through
    // the subject scope. `evaluate/fiber.rs` used to rebuild this table at dispatch.
    let param_vocab = query_class
        .as_ref()
        .and_then(|qc| index.query_class(qc))
        .map(|entry| {
            let mut v = Vocabulary::default();
            v.add_class(&entry.query_class, layer);
            v
        })
        .unwrap_or_default();

    let params = all(fc.params.into_iter().map(|param| {
        let name = resolve_property(&param.name, &param_vocab, layer, namespaces, errors);
        let value = match param.value {
            ParamValue::Expression(e) => {
                resolve_expression(e, scope, layer, namespaces, errors).map(ParamValue::Expression)
            }
            // A comorphism is a declared resource like any other.
            ParamValue::Comorphism { name, source } => {
                let cm = resolve_chain_name(
                    &name,
                    &[wk::COMORPHISM],
                    "comorphism",
                    layer,
                    namespaces,
                    errors,
                );
                let src = resolve_expression(source, scope, layer, namespaces, errors);
                match (cm, src) {
                    (Some(name), Some(source)) => Some(ParamValue::Comorphism { name, source }),
                    _ => None,
                }
            }
        };
        match (name, value) {
            (Some(name), Some(value)) => Some(ParamBinding { name, value }),
            _ => None,
        }
    }));

    Some(FiberClause {
        institution: institution?,
        query_class: query_class?,
        params: params?,
        binding: fc.binding,
        into: fc.into,
    })
}

/// Every reference an expression holds — which is the dot-path segments, and whatever the
/// sub-expressions hold.
fn resolve_expression(
    expr: Expression<Parsed>,
    scope: &Scope,
    layer: &Layer,
    namespaces: &[String],
    errors: &mut Vec<QueryError>,
) -> Option<Expression<Resolved>> {
    let go = |e: Expression<Parsed>, errors: &mut Vec<QueryError>| {
        resolve_expression(e, scope, layer, namespaces, errors).map(Box::new)
    };
    Some(match expr {
        Expression::Literal(l) => Expression::Literal(l),
        Expression::Variable(v) => Expression::Variable(v),
        Expression::Binary { op, left, right } => {
            // Both sides, before deciding. `left?` would stop at the first failure and
            // `?a.nope_left = ?a.nope_right` would report one error where it names two.
            let left = go(*left, errors);
            let right = go(*right, errors);
            Expression::Binary {
                op,
                left: left?,
                right: right?,
            }
        }
        Expression::Unary { op, operand } => Expression::Unary {
            op,
            operand: go(*operand, errors)?,
        },
        Expression::VerdictPredicate { kind, operand } => Expression::VerdictPredicate {
            kind,
            operand: go(*operand, errors)?,
        },
        Expression::NotExists(operand) => Expression::NotExists(go(*operand, errors)?),
        Expression::FunctionCall { name, args } => Expression::FunctionCall {
            name,
            args: all(args
                .into_iter()
                .map(|a| resolve_expression(a, scope, layer, namespaces, errors)))?,
        },
        Expression::Aggregate { op, arg } => Expression::Aggregate {
            op,
            arg: go(*arg, errors)?,
        },
        Expression::DotPath { root, segments } => {
            // Each segment is scoped by the previous property's declared range; the first
            // by the root's own vocabulary.
            // A failed segment stops THIS path — the next segment's scope is the failed
            // one's range, so there is nothing to resolve it against — but the error is
            // recorded and the rest of the program still resolves.
            let mut vocab = scope
                .vocabularies
                .get(&root.name)
                .cloned()
                .unwrap_or_default();
            let mut resolved = Vec::with_capacity(segments.len());
            for segment in segments {
                let iri = resolve_property(&segment, &vocab, layer, namespaces, errors)?;
                vocab = range_vocabulary(&iri, layer);
                resolved.push(iri);
            }
            Expression::DotPath {
                root,
                segments: resolved,
            }
        }
        Expression::Array(elements) => Expression::Array(all(elements
            .into_iter()
            .map(|e| resolve_expression(e, scope, layer, namespaces, errors)))?),
        Expression::Similarity {
            property,
            query,
            hints,
        } => Expression::Similarity {
            property,
            query: go(*query, errors)?,
            hints,
        },
    })
}

// ─── Scope: which vocabulary a short name may be drawn from ──────────

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
/// A variable bound by two classed patterns carries both — resolution searches the union,
/// so `MATCH ?x Dog { … }, ?x Pet { … }` reaches either class's properties. A `DEFINE`
/// relation is not a class and contributes nothing; neither does a class that fails to
/// resolve, whose error the pattern's own resolution reports.
/// What one `MATCH` part's scope pass worked out: the vocabulary each subject carries,
/// and the reference each clause's class resolved to.
///
/// **Both come from one pass because they come from one lookup.** Building the vocabulary
/// requires resolving the pattern's class, and so does producing its `ClassRef`; doing
/// them in separate passes resolves each class name twice. That is the duplication D92
/// removes from the pipeline, and it would be odd to leave a copy of it inside the pass
/// that removes it.
#[derive(Default)]
struct Scope {
    vocabularies: BTreeMap<String, Vocabulary>,
    /// By clause index, for the pattern clauses whose class resolved.
    classes: BTreeMap<usize, ClassRef>,
    /// By clause index, for the FIBER clauses whose query class resolved.
    query_classes: BTreeMap<usize, Iri>,
}

/// Resolve every class in a match part, and build the vocabulary each subject carries.
///
/// A variable bound by two classed patterns carries both — resolution searches the union,
/// so `MATCH ?x Dog { … }, ?x Pet { … }` reaches either class's properties. A `DEFINE`
/// relation is not a class and contributes no vocabulary.
fn build_scope(
    part: &MatchPart<Parsed>,
    layer: &Layer,
    index: &InstitutionIndex,
    relations: &BTreeMap<String, RelationId>,
    errors: &mut Vec<QueryError>,
) -> Scope {
    let mut out = Scope::default();
    for (i, clause) in part.clauses.iter().enumerate() {
        match clause {
            Clause::Pattern(p) => {
                let Some(name) = &p.class else { continue };
                let Some(class) =
                    resolve_pattern_class(name, layer, &part.using_namespaces, relations, errors)
                else {
                    continue;
                };
                if let ClassRef::Chain(iri) = &class {
                    out.vocabularies
                        .entry(p.subject.name.clone())
                        .or_default()
                        .add_class(iri, layer);
                }
                out.classes.insert(i, class);
            }
            Clause::Fiber(fc) => {
                let Some(qc_iri) = resolve_chain_name(
                    &fc.query_class,
                    &[wk::QUERY_CLASS_CLASS],
                    "FIBER query class",
                    layer,
                    &part.using_namespaces,
                    errors,
                ) else {
                    continue;
                };
                // The binding's vocabulary is the QueryClass's output contract, which D90
                // closed and the kernel enforces at the dispatch boundary: what
                // `result_class` declares, plus whatever `result_properties` adds.
                if let Some(entry) = index.query_class(&qc_iri) {
                    let vocab = out.vocabularies.entry(fc.binding.name.clone()).or_default();
                    vocab.add_class(&entry.result_class, layer);
                    vocab
                        .properties
                        .extend(entry.result_properties.iter().cloned());
                }
                out.query_classes.insert(i, qc_iri);
            }
        }
    }
    out
}

/// Resolve one property reference: the class scope answers first, the imported namespaces
/// where it does not, and a full IRI needs neither.
fn resolve_property(
    name: &Name,
    vocab: &Vocabulary,
    layer: &Layer,
    namespaces: &[String],
    errors: &mut Vec<QueryError>,
) -> Option<Iri> {
    let short = match name {
        Name::FullIri(iri) => return Some(iri.clone()),
        Name::ShortName(s) => s,
    };
    let resolved = match scoped_property(short, vocab, layer) {
        Ok(found) => match found {
            Some(iri) => Some(iri),
            None => match resolve_scoped_name(layer, namespaces, &[wk::PROPERTY], short) {
                Ok(v) => v,
                Err(e) => {
                    errors.push(e);
                    return None;
                }
            },
        },
        Err(e) => {
            errors.push(e);
            return None;
        }
    };
    match resolved {
        Some(iri) => Some(iri),
        None => {
            errors.push(QueryError::type_check(
                "property_name_unresolved",
                if vocab.is_empty() {
                    format!(
                        "property '{short}' does not resolve to a declared property in the \
                         imported namespaces, and nothing in scope declares it — add \
                         `USING NAMESPACE`, give the pattern a class, or write the full \
                         property IRI"
                    )
                } else {
                    format!(
                        "property '{short}' is declared by neither {} nor the imported \
                         namespaces — write the full property IRI to reach a property the \
                         class does not declare",
                        vocab
                            .sources
                            .iter()
                            .map(|c| format!("'{c}'"))
                            .collect::<Vec<_>>()
                            .join(" nor ")
                    )
                },
            ));
            None
        }
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
                "short name '{short}' names {} distinct properties in the classes in scope \
                 ({}); write the full property IRI",
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
/// dot-path segment. Empty where the property declares none, which drops the next segment
/// to the namespace scope.
fn range_vocabulary(prop: &Iri, layer: &Layer) -> Vocabulary {
    let mut vocab = Vocabulary::default();
    let Some(res) = layer.resolve(prop) else {
        return vocab;
    };
    if let Some(v) = res.get(&wk::iri(wk::CLASS_TYPES)) {
        for class in v.as_iri_array() {
            vocab.add_class(&class, layer);
        }
    }
    vocab
}
