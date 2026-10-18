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

//! Every property a query names — a `MATCH` brace key, a dot-path segment — resolves
//! against declared vocabulary: the class where the query states one, the imported
//! namespaces otherwise, and a full IRI where neither reaches it. D2 §5.4 and §5.6 said
//! so; the evaluator instead matched the short name against the LOCAL NAME of whatever
//! IRIs the resource happened to carry.
//!
//! Two mechanisms for one name disagree in both directions, and both directions are
//! silent. A property declared `urn:ex:title_text` with `short_name "title"` was named
//! `title` by a query that type-checked, and matched nothing. A resource carrying an
//! undeclared `urn:other:title` answered to `title` for a query that meant the declared
//! one. Neither reported anything: one returned no rows, the other returned the wrong
//! value.

use std::sync::Arc;

use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::query::{error::QueryError, evaluate::FiberRuntime, execute_with};

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}

fn strings(v: &[&str]) -> Value {
    Value::Array(v.iter().map(|s| Value::String((*s).to_string())).collect())
}

/// A `Notebook` whose declared title property is NOT named `title` at the end of its
/// IRI, and a notebook resource that also carries an undeclared `urn:other:title`.
/// The two properties hold different values, so any test that reads one can say which
/// mechanism answered.
fn notebooks() -> Arc<Layer> {
    let boot = eigenius_kernel::testing::bootstrap_context();
    let mut b = LayerBuilder::new("notebooks", Some(Arc::clone(boot.head())));

    let mut property = |id: &str, short: &str, range: Option<&str>| {
        let mut p = Resource::new(iri(id));
        p.set(iri(wk::IS_A), strings(&[wk::PROPERTY]));
        p.set(iri(wk::DESCRIPTION), Value::String("probe property".into()));
        p.set(iri(wk::SHORT_NAME), Value::String(short.into()));
        match range {
            Some(class) => {
                p.set(iri(wk::DATA_TYPE_PROP), Value::String(wk::RESOURCE.into()));
                p.set(iri(wk::CLASS_TYPES), strings(&[class]));
            }
            None => p.set(
                iri(wk::DATA_TYPE_PROP),
                Value::String("urn:eigenius:core:string".into()),
            ),
        }
        b.add_resource(p).unwrap();
    };
    // `title_text` is the declared one; its short name is `title` and its local name is
    // not. `urn:other:title` is the decoy: local name `title`, declared by no class the
    // query names and in no namespace the query imports.
    property("urn:ex:title_text", "title", None);
    property("urn:other:title", "title", None);
    property("urn:ex:person_name", "name", None);
    property("urn:ex:author", "author", Some("urn:ex:Person"));

    let mut class = |id: &str, short: &str, requires: &[&str]| {
        let mut c = Resource::new(iri(id));
        c.set(iri(wk::IS_A), strings(&[wk::CLASS]));
        c.set(iri(wk::DESCRIPTION), Value::String("probe class".into()));
        c.set(iri(wk::SHORT_NAME), Value::String(short.into()));
        c.set(iri(wk::REQUIRES), strings(requires));
        b.add_resource(c).unwrap();
    };
    class("urn:ex:Person", "Person", &["urn:ex:person_name"]);
    class(
        "urn:ex:Notebook",
        "Notebook",
        &["urn:ex:title_text", "urn:ex:author"],
    );

    let mut person = Resource::new(iri("urn:ex:p1"));
    person.set(iri(wk::IS_A), strings(&["urn:ex:Person"]));
    person.set(iri("urn:ex:person_name"), Value::String("Ada".into()));
    b.add_resource(person).unwrap();

    let mut n = Resource::new(iri("urn:ex:n1"));
    n.set(iri(wk::IS_A), strings(&["urn:ex:Notebook"]));
    n.set(iri("urn:ex:title_text"), Value::String("declared".into()));
    n.set(iri("urn:other:title"), Value::String("decoy".into()));
    n.set(iri("urn:ex:author"), Value::String("urn:ex:p1".into()));
    b.add_resource(n).unwrap();

    Arc::new(b.build(LayerStorage::in_memory()))
}

/// The queries below name the class by full IRI rather than as `Notebook`. That keeps
/// `urn:ex:` out of the namespace scope — a `USING NAMESPACE "urn:ex:"` would resolve
/// these short names on its own and the tests would stop saying anything about the class.
fn run(layer: &Arc<Layer>, q: &str) -> Vec<Resource> {
    execute_with(q, layer, FiberRuntime::default())
        .unwrap_or_else(|e| panic!("query failed: {e:?}"))
}

fn errors(layer: &Arc<Layer>, q: &str) -> Vec<QueryError> {
    match execute_with(q, layer, FiberRuntime::default()) {
        Ok(_) => Vec::new(),
        Err(e) => e,
    }
}

/// The values one column holds, in row order.
fn column(rows: &[Resource], name: &str) -> Vec<String> {
    let result_set = rows
        .iter()
        .find(|r| {
            r.id()
                .map(|i| i.as_str().ends_with(":result"))
                .unwrap_or(false)
        })
        .expect("a result set");
    let prop = rows
        .iter()
        .filter_map(|r| r.id())
        .find(|i| i.as_str().ends_with(&format!(":{name}")))
        .cloned()
        .unwrap_or_else(|| panic!("no column `{name}`"));
    match result_set.get(&iri("urn:eigenius:query:rows")) {
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| match v {
                Value::Embedded(r) => r.get(&prop).and_then(|x| x.as_str().map(str::to_owned)),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// **A short name is the declared `short_name`, not the tail of the IRI.**
///
/// `urn:ex:title_text` declares `short_name "title"` and `Notebook` requires it. Matching
/// the local name instead found `urn:other:title` — a property of the same resource that
/// the class does not declare — and returned its value.
#[test]
fn a_dot_path_segment_resolves_to_the_property_the_class_declares() {
    let layer = notebooks();
    let rows = run(
        &layer,
        r#"
        USING "urn:ex:Notebook"
        MATCH "urn:ex:Notebook"(?n) { }
        RETURN [] { t: ?n.title }
        "#,
    );
    assert_eq!(column(&rows, "t"), vec!["declared".to_string()]);
}

/// The same, for a brace key. `MATCH` resolved its keys by local name too.
#[test]
fn a_brace_key_resolves_to_the_property_the_class_declares() {
    let layer = notebooks();
    let rows = run(
        &layer,
        r#"
        USING "urn:ex:Notebook"
        MATCH "urn:ex:Notebook"(?n) { title: ?t }
        RETURN [] { t: ?t }
        "#,
    );
    assert_eq!(column(&rows, "t"), vec!["declared".to_string()]);
}

/// **A mistyped segment is an error, not an empty result set.**
///
/// Absence was decided from the resource's own properties, so a typo and a property no
/// resource happens to carry were the same answer: no rows, no diagnostic.
#[test]
fn a_mistyped_dot_path_segment_is_a_type_error() {
    let layer = notebooks();
    let errs = errors(
        &layer,
        r#"
        USING "urn:ex:Notebook"
        MATCH "urn:ex:Notebook"(?n) { }
        RETURN [] { t: ?n.titel }
        "#,
    );
    assert!(
        errs.iter().any(|e| e.rule == "property_name_unresolved"),
        "expected property_name_unresolved, got {errs:?}"
    );
}

/// The same hole, one construct over: a mistyped brace key matched nothing and said
/// nothing. Nothing in the type-checker asked whether a brace key resolved.
#[test]
fn a_mistyped_brace_key_is_a_type_error() {
    let layer = notebooks();
    let errs = errors(
        &layer,
        r#"
        USING "urn:ex:Notebook"
        MATCH "urn:ex:Notebook"(?n) { titel: ?t }
        RETURN [] { t: ?t }
        "#,
    );
    assert!(
        errs.iter().any(|e| e.rule == "property_name_unresolved"),
        "expected property_name_unresolved, got {errs:?}"
    );
}

/// **The escape hatch.** `urn:other:title` is declared by no class the query names and
/// sits in no imported namespace, so no short name reaches it — a full IRI does. A
/// dot-path segment could only be a bare identifier before, which is why scoping a
/// segment was unsound without this: the rule's other half had no syntax.
#[test]
fn a_full_iri_segment_reaches_a_property_outside_every_scope() {
    let layer = notebooks();
    let rows = run(
        &layer,
        r#"
        USING "urn:ex:Notebook"
        MATCH "urn:ex:Notebook"(?n) { }
        RETURN [] { t: ?n."urn:other:title" }
        "#,
    );
    assert_eq!(column(&rows, "t"), vec!["decoy".to_string()]);
}

/// **An intermediate segment scopes the next one by its declared range.**
///
/// `urn:ex:author` declares `class_types [urn:ex:Person]`, and `Person` requires
/// `urn:ex:person_name` with `short_name "name"`. Nothing on the query names `Person`.
#[test]
fn a_later_segment_resolves_against_the_previous_property_s_range() {
    let layer = notebooks();
    let rows = run(
        &layer,
        r#"
        USING "urn:ex:Notebook"
        MATCH "urn:ex:Notebook"(?n) { }
        RETURN [] { a: ?n.author.name }
        "#,
    );
    assert_eq!(column(&rows, "a"), vec!["Ada".to_string()]);
}

/// **The namespace half of the rule.** With no class on the pattern, `USING NAMESPACE
/// "urn:ex:"` is what puts `title` in scope — and scope is what picks `urn:ex:title_text`
/// over `urn:other:title`, which is in no imported namespace. Matching by local name
/// consulted no namespace at all and returned the decoy from the unimported one.
#[test]
fn a_namespace_import_resolves_a_short_name_with_no_class_in_scope() {
    let layer = notebooks();
    let rows = run(
        &layer,
        r#"
        USING NAMESPACE "urn:ex:"
        MATCH ?n { title: ?t }
        RETURN [] { t: ?t }
        "#,
    );
    assert_eq!(column(&rows, "t"), vec!["declared".to_string()]);
}

/// A short name with neither scope to draw on is an error naming both remedies. Without
/// `USING NAMESPACE` and without a class, `title` is a bare word.
#[test]
fn a_short_name_with_no_scope_at_all_is_a_type_error() {
    let layer = notebooks();
    let errs = errors(
        &layer,
        r#"
        MATCH ?n { title: ?t }
        RETURN [] { t: ?t }
        "#,
    );
    assert!(
        errs.iter().any(|e| e.rule == "property_name_unresolved"),
        "expected property_name_unresolved, got {errs:?}"
    );
}

// ─── The FIBER binding's scope ───────────────────────────────────────
//
// A FIBER response never appears as a pattern subject, so the query text says nothing
// about its class. Its vocabulary is the QueryClass's output contract — `result_class`
// plus `result_properties` — which D90 closed and the kernel enforces against the
// institution at the dispatch boundary. The two lists are therefore the same list: a
// property the query may name on `?b` is a property the institution may set on it.
//
// The live case is `crates/eigenius-julia`'s `?bound.lower` / `?bound.upper` against
// `qc_compute_bounds`, whose `result_class` is `intervals:BoundedBy` and which requires
// `intervals:lower` and `intervals:upper`. That test is `#[ignore]`d behind a Julia env
// image build, so without what follows this scope has no CI coverage at all.

const INST: &str = "urn:eigenius:test:fiberscope:Institution";
const QC: &str = "urn:eigenius:test:fiberscope:qc";
const INPUT_CLASS: &str = "urn:ex:BoundsRequest";
const RESULT_CLASS: &str = "urn:ex:BoundedBy";

/// A chain carrying one OnDemand QueryClass whose result class declares `lower`, plus an
/// `urn:other:` property the contract does not mention.
fn with_a_query_class() -> Arc<Layer> {
    let boot = eigenius_kernel::testing::bootstrap_context();
    let mut b = LayerBuilder::new("fiberscope", Some(Arc::clone(boot.head())));

    let mut property = |id: &str, short: &str| {
        let mut p = Resource::new(iri(id));
        p.set(iri(wk::IS_A), strings(&[wk::PROPERTY]));
        p.set(iri(wk::DESCRIPTION), Value::String("probe property".into()));
        p.set(iri(wk::SHORT_NAME), Value::String(short.into()));
        p.set(
            iri(wk::DATA_TYPE_PROP),
            Value::String("urn:eigenius:core:float".into()),
        );
        b.add_resource(p).unwrap();
    };
    property("urn:ex:lower", "lower");
    property("urn:ex:expr", "expr");
    property("urn:other:witness", "witness");

    let mut class = |id: &str, short: &str, requires: &[&str]| {
        let mut c = Resource::new(iri(id));
        c.set(iri(wk::IS_A), strings(&[wk::CLASS]));
        c.set(iri(wk::DESCRIPTION), Value::String("probe class".into()));
        c.set(iri(wk::SHORT_NAME), Value::String(short.into()));
        c.set(iri(wk::REQUIRES), strings(requires));
        b.add_resource(c).unwrap();
    };
    class(INPUT_CLASS, "BoundsRequest", &["urn:ex:expr"]);
    class(RESULT_CLASS, "BoundedBy", &["urn:ex:lower"]);

    let mut institution = Resource::new(iri(INST));
    institution.set(
        iri(wk::IS_A),
        strings(&["urn:eigenius:institution:Institution"]),
    );
    institution.set(
        iri("urn:eigenius:institution:institution_iri"),
        Value::String(INST.into()),
    );
    institution.set(
        iri("urn:eigenius:institution:institution_name"),
        Value::String("FiberScopeProbe".into()),
    );
    b.add_resource(institution).unwrap();

    let mut qc = Resource::new(iri(QC));
    qc.set(iri(wk::IS_A), strings(&[wk::QUERY_CLASS_CLASS]));
    qc.set(iri(wk::QUERY_CLASS), Value::String(INPUT_CLASS.into()));
    qc.set(iri(wk::RESULT_CLASS), Value::String(RESULT_CLASS.into()));
    qc.set(iri(wk::DISPATCH_ROLE), strings(&[wk::DISPATCH_ON_DEMAND]));
    qc.set(
        iri(wk::QUERY_HANDLER),
        Value::String("urn:eigenius:test:fiberscope:proc".into()),
    );
    qc.set(
        iri("urn:eigenius:institution:institution_ref"),
        Value::String(INST.into()),
    );
    qc.set(iri(wk::SHORT_NAME), Value::String("qc".into()));
    b.add_resource(qc).unwrap();

    Arc::new(b.build(LayerStorage::in_memory()))
}

/// Resolve and type-check, stopping before evaluation — the dispatch would need a live
/// institution runtime, and every question here is decided before any of that.
fn type_errors(layer: &Arc<Layer>, q: &str) -> Vec<QueryError> {
    let index =
        eigenius_kernel::institution::registry::InstitutionIndex::from_layer_indexed(layer).0;
    match resolve_program(layer, q) {
        Ok(resolved) => {
            eigenius_kernel::query::type_check::type_check(&resolved.program, layer, &index)
        }
        Err(errors) => errors,
    }
}

/// The resolved program, or the errors resolution reported.
fn resolve_program(
    layer: &Arc<Layer>,
    q: &str,
) -> Result<eigenius_kernel::query::resolve::ResolvedProgram, Vec<QueryError>> {
    let tokens = eigenius_kernel::query::lexer::tokenize(q).expect("lexes");
    let program = eigenius_kernel::query::parser::parse(tokens).expect("parses");
    let strata =
        eigenius_kernel::query::stratify::stratify(&program.definitions).expect("stratifies");
    let index =
        eigenius_kernel::institution::registry::InstitutionIndex::from_layer_indexed(layer).0;
    eigenius_kernel::query::resolve::resolve(program, strata, layer, &index)
}

fn fiber_query(projection: &str) -> String {
    format!(
        r#"
USING INSTITUTION "{INST}" AS cap
FIBER cap:"{QC}" {{ "urn:ex:expr": "x" }} AS ?b
RETURN [] {{ v: {projection} }}
"#
    )
}

/// **The QueryClass's `result_class` is the binding's scope.** Nothing else in the query
/// says what `?b` is.
///
/// A guard against over-restriction, not a catch: it asserts an error's ABSENCE, and no
/// such error existed before the rule, so it passes either way. The two below it are the
/// discriminating pair — every other test in this file fails against the unfixed code.
#[test]
fn a_fiber_binding_resolves_against_the_declared_result_class() {
    let layer = with_a_query_class();
    let errs = type_errors(&layer, &fiber_query("?b.lower"));
    assert!(
        !errs.iter().any(|e| e.rule == "property_name_unresolved"),
        "`lower` is required by the result class, so it is in scope: {errs:?}"
    );
}

/// A name the output contract does not carry is refused. Without the contract as a scope
/// this could only be answered at evaluation, by whether the institution happened to
/// return something ending in `witness`.
#[test]
fn a_name_outside_the_output_contract_is_refused() {
    let layer = with_a_query_class();
    let errs = type_errors(&layer, &fiber_query("?b.witness"));
    assert!(
        errs.iter().any(|e| e.rule == "property_name_unresolved"),
        "`witness` is declared by neither the result class nor an import: {errs:?}"
    );
}

/// The escape hatch reaches it anyway, which is what keeps the rule above from being a
/// restriction on what a query can ask.
#[test]
fn a_full_iri_reaches_past_the_output_contract() {
    let layer = with_a_query_class();
    let errs = type_errors(&layer, &fiber_query(r#"?b."urn:other:witness""#));
    assert!(
        !errs.iter().any(|e| e.rule == "property_name_unresolved"),
        "a full IRI needs no scope: {errs:?}"
    );
}

// ─── What a code review found ────────────────────────────────────────

/// **A FIBER param value is an expression, and its names need resolving too.**
///
/// `resolve_part` matched only `Clause::Pattern`, so `Clause::Fiber` was skipped entirely
/// and a dot-path in a param kept its `ShortName` segments. The query then type-checked
/// with zero errors and failed at EVALUATION with "was never resolved to a property IRI"
/// — a message about the compiler pass, shown to someone who wrote a valid query. D2 §3.5
/// makes `param_value ::= expression`, and this shape worked before the rule.
#[test]
fn a_dot_path_inside_a_fiber_param_is_resolved() {
    let layer = with_a_query_class();
    let q = format!(
        r#"
USING INSTITUTION "{INST}" AS cap
USING NAMESPACE "urn:ex:"
MATCH ?n {{ }}
FIBER cap:"{QC}" {{ "urn:ex:expr": ?n.lower }} AS ?b
RETURN [] {{ v: ?b.lower }}
"#
    );
    let errs = type_errors(&layer, &q);
    assert!(
        !errs.iter().any(|e| e.rule == "property_name_unresolved"),
        "`lower` is in the imported namespace: {errs:?}"
    );
    // The segment must actually be resolved. Under D92 a `Program<Resolved>` cannot hold
    // an unresolved one — the type is the assertion — so this reads the IRI to confirm it
    // is the property the namespace scope names, not merely that something is there.
    let resolved = resolve_program(&layer, &q).expect("resolves");
    let clause = resolved
        .program
        .query
        .body
        .clauses
        .iter()
        .find_map(|c| match c {
            eigenius_kernel::query::ast::Clause::Fiber(f) => Some(f),
            _ => None,
        })
        .expect("a FIBER clause");
    let param = &clause.params[0];
    let eigenius_kernel::query::ast::ParamValue::Expression(
        eigenius_kernel::query::ast::Expression::DotPath { segments, .. },
    ) = &param.value
    else {
        panic!("expected a dot-path param value, got {:?}", param.value);
    };
    assert_eq!(
        segments[0].as_str(),
        "urn:ex:lower",
        "the param's segment resolved to the wrong property"
    );
}

/// **An explicit `USING NAMESPACE` shadows the implicit core prelude.**
///
/// Core and the imported prefixes were one flat candidate pool, so a short name declared
/// in both was `ambiguous_short_name` — and the error's remedy, "import fewer namespaces",
/// cannot be taken, because core is never imported in the first place. Five shipped
/// ontologies collide with core exactly this way, `schema_org:description` among them.
///
/// Naming a namespace says which vocabulary the query means, so it wins, the way an inner
/// scope wins over an outer one.
#[test]
fn an_imported_namespace_wins_over_the_core_prelude() {
    let boot = eigenius_kernel::testing::bootstrap_context();
    let mut b = LayerBuilder::new("shadowing", Some(Arc::clone(boot.head())));

    // Same short name as `core:description`, which every chain carries.
    let mut p = Resource::new(iri("urn:ex:description"));
    p.set(iri(wk::IS_A), strings(&[wk::PROPERTY]));
    p.set(iri(wk::DESCRIPTION), Value::String("a rival".into()));
    p.set(iri(wk::SHORT_NAME), Value::String("description".into()));
    p.set(
        iri(wk::DATA_TYPE_PROP),
        Value::String("urn:eigenius:core:string".into()),
    );
    b.add_resource(p).unwrap();

    let mut r = Resource::new(iri("urn:ex:thing"));
    r.set(iri(wk::IS_A), strings(&[wk::CLASS]));
    r.set(iri(wk::SHORT_NAME), Value::String("Thing".into()));
    r.set(iri(wk::DESCRIPTION), Value::String("core's slot".into()));
    r.set(
        iri("urn:ex:description"),
        Value::String("the import's".into()),
    );
    b.add_resource(r).unwrap();
    let layer = Arc::new(b.build(LayerStorage::in_memory()));

    let rows = run(
        &layer,
        r#"
        USING NAMESPACE "urn:ex:"
        MATCH ?t { description: ?d }
        RETURN [] { d: ?d }
        "#,
    );
    assert_eq!(
        column(&rows, "d"),
        vec!["the import's".to_string()],
        "the imported namespace names the property, not the prelude"
    );
}

// ─── What the D92 review found ───────────────────────────────────────

/// **A relation defined by several rules is ONE relation.**
///
/// `RelationId` first indexed `Program::definitions`, which splits a base case and a
/// recursive case into two relations: the derived facts never meet and the closure never
/// accumulates. The reachability tests caught it behaviourally — 2 unreachable nodes
/// where 1 was right — and this names the invariant so the next reader does not have to
/// infer it from a graph fixture.
#[test]
fn several_rules_defining_one_relation_share_an_id() {
    use eigenius_kernel::query::ast::{relation_ids, RelationId};
    let q = r#"
        DEFINE Reach(?t) FROM MATCH ?o { "urn:ex:seed": ?t }
        DEFINE Reach(?n) FROM MATCH Reach(?m) { "urn:ex:dep": ?n }
        DEFINE Other(?x) FROM MATCH ?x { "urn:ex:seed": ?v }
        MATCH Reach(?r) {} RETURN [] { r: ?r }
    "#;
    let tokens = eigenius_kernel::query::lexer::tokenize(q).expect("lexes");
    let program = eigenius_kernel::query::parser::parse(tokens).expect("parses");
    assert_eq!(program.definitions.len(), 3, "three rules");

    let ids = relation_ids(&program.definitions);
    assert_eq!(ids.len(), 2, "but two relations");
    assert_eq!(
        ids["Reach"],
        RelationId(0),
        "both Reach rules share the first id"
    );
    assert_eq!(ids["Other"], RelationId(1));
}

/// **`RETURN Class [] { … }` names a chain class, and it is checked.**
///
/// Nothing asked this before D92: the bare string was stamped into the row class's
/// `is_a` and `subclass_of`, so a `RETURN` naming a class the chain does not declare
/// produced rows asserting membership of nothing.
#[test]
fn a_return_result_class_that_resolves_nowhere_is_refused() {
    let layer = notebooks();
    let errs = errors(
        &layer,
        r#"
        USING "urn:ex:Notebook"
        MATCH "urn:ex:Notebook"(?n) { }
        RETURN [NoSuchClass] { t: ?n."urn:ex:title_text" }
        "#,
    );
    assert!(
        errs.iter()
            .any(|e| e.rule == "unresolved_reference" && e.message.contains("NoSuchClass")),
        "expected the result class to be reported unresolvable: {errs:?}"
    );
}

/// The same position, resolving: a `RETURN` class in an imported namespace is accepted,
/// so the rule above rejects unresolvable names rather than the construct.
#[test]
fn a_return_result_class_resolves_through_the_namespace_scope() {
    let layer = notebooks();
    let errs = errors(
        &layer,
        r#"
        USING NAMESPACE "urn:ex:"
        MATCH ?n { title: ?t }
        RETURN [Notebook] { t: ?t }
        "#,
    );
    assert!(
        !errs.iter().any(|e| e.rule == "unresolved_reference"),
        "`Notebook` is declared in the imported namespace: {errs:?}"
    );
}

/// **Resolution reports every unresolvable name, not the first.**
///
/// The pass's own doc said so and `Option`'s `FromIterator` did not: it stops at the
/// first `None`, so one run named one bad key where the code it replaced named both.
#[test]
fn resolution_reports_every_unresolvable_name() {
    let layer = notebooks();
    let errs = errors(
        &layer,
        r#"
        USING "urn:ex:Notebook"
        MATCH "urn:ex:Notebook"(?n) { nope_one: ?a, nope_two: ?b }
        RETURN [] { a: ?a }
        "#,
    );
    let unresolved: Vec<_> = errs
        .iter()
        .filter(|e| e.rule == "property_name_unresolved")
        .collect();
    assert_eq!(
        unresolved.len(),
        2,
        "both keys are unresolvable and both should be reported: {errs:?}"
    );
    assert!(errs.iter().any(|e| e.message.contains("nope_one")));
    assert!(errs.iter().any(|e| e.message.contains("nope_two")));
}

/// **A full-IRI FIBER param is not gated on the input class.**
///
/// It never was: this pass accepted any full-IRI param and checked its shape, not its
/// membership. A D92 draft made every param go through the input class's vocabulary,
/// which rejected `{ "urn:ex:sidecar": … }` that used to dispatch — a language change
/// D92 did not argue for.
#[test]
fn a_full_iri_fiber_param_is_not_gated_on_the_input_class() {
    let layer = with_a_query_class();
    let q = format!(
        r#"
USING INSTITUTION "{INST}" AS cap
FIBER cap:"{QC}" {{ "urn:ex:expr": "x", "urn:other:witness": "y" }} AS ?b
RETURN [] {{ v: ?b.lower }}
"#
    );
    let errs = type_errors(&layer, &q);
    assert!(
        !errs
            .iter()
            .any(|e| e.rule == "fiber_param_short_name_unresolved"),
        "a full-IRI param is not required to be declared by the input class: {errs:?}"
    );
}

/// **A full-IRI FIBER query class that is not an indexed QueryClass is still refused.**
///
/// Resolution accepts a full IRI without asking what it names — that is what a full IRI
/// means — so this rule is type-check's, and it is the check D92 left `type_check` doing
/// after it stopped resolving. Its only test was retargeted to the resolution error when
/// the staging changed, which left both of its two reachable paths uncovered.
#[test]
fn a_fiber_query_class_that_is_not_indexed_is_refused() {
    let layer = with_a_query_class();
    let q = format!(
        r#"
USING INSTITUTION "{INST}" AS cap
FIBER cap:"urn:ex:not_a_query_class" {{ "urn:ex:expr": "x" }} AS ?b
RETURN [] {{ v: ?b."urn:ex:lower" }}
"#
    );
    let errs = type_errors(&layer, &q);
    assert!(
        errs.iter()
            .any(|e| e.rule == "fiber_query_class_not_query_class"),
        "a full IRI naming no indexed QueryClass must be refused: {errs:?}"
    );
}
