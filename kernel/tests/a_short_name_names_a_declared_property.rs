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

/// Type-check alone — the dispatch would need a live institution runtime, and resolution
/// is decided before any of that.
fn type_errors(layer: &Arc<Layer>, q: &str) -> Vec<QueryError> {
    let tokens = eigenius_kernel::query::lexer::tokenize(q).expect("lexes");
    let mut program = eigenius_kernel::query::parser::parse(tokens).expect("parses");
    eigenius_kernel::query::type_check::type_check(&mut program, layer)
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
    // The segment must actually be rewritten — a clean type-check that leaves a ShortName
    // behind is the defect, not the absence of an error.
    let tokens = eigenius_kernel::query::lexer::tokenize(&q).expect("lexes");
    let mut program = eigenius_kernel::query::parser::parse(tokens).expect("parses");
    let _ = eigenius_kernel::query::type_check::type_check(&mut program, &layer);
    let clause = program
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
    assert!(
        matches!(segments[0], eigenius_kernel::query::ast::Name::FullIri(_)),
        "the param's segment reaches the evaluator unresolved: {:?}",
        segments[0]
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
