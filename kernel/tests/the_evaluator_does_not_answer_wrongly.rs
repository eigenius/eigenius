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

//! Four ways EigenQL returned a wrong answer with no diagnostic.
//!
//! Each produced a plausible-looking result — an empty set, a number, an order — that was
//! simply not the answer to the question asked, and nothing said so. That is what makes
//! them worse than a crash: a query that fails gets debugged, and a query that quietly
//! lies gets believed.
//!
//! - **eigenius#126** — every evaluation failure in a `WHERE` condition was swallowed and
//!   the row dropped, so a condition that could not be a filter at all behaved exactly
//!   like a filter that excluded everything.
//! - **eigenius#123** — two aggregates of the same operator in one `RETURN` shared a
//!   binding key, so the second overwrote the first and both columns showed one number.
//! - **`ORDER BY`** over anything but a bare variable was a silent no-op, so D2 §8.8's own
//!   worked example returned an unordered result.
//! - **eigenius#172** — `-` was folded into a following digit without consulting
//!   whitespace, so `?a - 1` parsed and `?a -1` died at end of input.

use std::sync::Arc;

use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::query::{evaluate::FiberRuntime, execute_with};

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}

/// The Widget class and its two integer properties.
fn declare_vocabulary(b: &mut LayerBuilder) {
    let mut c = Resource::new(iri("urn:ex:Widget"));
    c.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(wk::CLASS.to_string())]),
    );
    c.set(iri(wk::DESCRIPTION), Value::String("probe class".into()));
    c.set(iri(wk::SHORT_NAME), Value::String("Widget".into()));
    b.add_resource(c).unwrap();

    for (prop, short) in [("urn:ex:size", "size"), ("urn:ex:weight", "weight")] {
        let mut p = Resource::new(iri(prop));
        p.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(wk::PROPERTY.to_string())]),
        );
        p.set(iri(wk::DESCRIPTION), Value::String("probe property".into()));
        p.set(iri(wk::SHORT_NAME), Value::String(short.into()));
        p.set(
            iri(wk::DATA_TYPE_PROP),
            Value::String("urn:eigenius:core:integer".into()),
        );
        b.add_resource(p).unwrap();
    }
}

fn widget(b: &mut LayerBuilder, id: &str, size: Option<i64>, weight: i64) {
    let mut r = Resource::new(iri(id));
    r.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String("urn:ex:Widget".into())]),
    );
    if let Some(sz) = size {
        r.set(iri("urn:ex:size"), Value::Integer(sz));
    }
    r.set(iri("urn:ex:weight"), Value::Integer(weight));
    b.add_resource(r).unwrap();
}

/// Three widgets. Sizes sum to 6 and weights to 60, so an aggregate over one is
/// unmistakably distinguishable from an aggregate over the other.
fn corpus() -> Arc<Layer> {
    let boot = eigenius_kernel::testing::bootstrap_context();
    let mut b = LayerBuilder::new("widgets", Some(Arc::clone(boot.head())));
    declare_vocabulary(&mut b);
    widget(&mut b, "urn:ex:w1", Some(1), 10);
    widget(&mut b, "urn:ex:w2", Some(2), 20);
    widget(&mut b, "urn:ex:w3", Some(3), 30);
    Arc::new(b.build(LayerStorage::in_memory()))
}

/// Widgets grouped so the counts differ: one of size 1, two of size 2, three of size 3.
/// Ordering by the COUNT is then distinguishable from every other order.
fn grouped_corpus() -> Arc<Layer> {
    let boot = eigenius_kernel::testing::bootstrap_context();
    let mut b = LayerBuilder::new("grouped", Some(Arc::clone(boot.head())));
    declare_vocabulary(&mut b);
    let mut n = 0;
    for size in 1..=3i64 {
        for _ in 0..size {
            n += 1;
            widget(&mut b, &format!("urn:ex:g{n}"), Some(size), 10 * n);
        }
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

/// The values one column of the result set holds, in row order.
fn column(rows: &[Resource], name: &str) -> Vec<Value> {
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
        .unwrap_or_else(|| panic!("no column `{name}` in the result document"));
    match result_set.get(&iri("urn:eigenius:query:rows")) {
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| match v {
                Value::Embedded(r) => r.get(&prop).cloned(),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

// ── eigenius#126 ─────────────────────────────────────────────────────

/// **A `WHERE` condition that is not boolean fails the query.**
///
/// Every failure inside a condition was swallowed by `unwrap_or(false)` and the row
/// dropped, so a condition that could never be a filter — an integer, a string — behaved
/// exactly like a filter that excluded everything. An empty result set was the answer to
/// both "nothing matched" and "this query is not a question".
#[test]
fn a_non_boolean_condition_is_reported_rather_than_dropping_every_row() {
    let layer = corpus();
    let err = execute_with(
        r#"
        USING "urn:ex:Widget"
        USING NAMESPACE "urn:ex:"
        MATCH Widget(?w) { "urn:ex:size": ?s }
        WHERE ?s
        RETURN [] { s: ?s }
        "#,
        &layer,
        FiberRuntime::default(),
    )
    .expect_err("an integer is not a condition");
    assert!(
        err.iter().any(|e| e.message.contains("boolean")),
        "the diagnostic should say what was wrong: {err:?}"
    );
}

/// **A dot-path on something that is not a resource fails the query.**
///
/// The same swallow, a different fault: walking `.size` off an integer is a query that
/// cannot mean anything, and it returned an empty set indistinguishable from a real
/// filter.
#[test]
fn a_dot_path_on_a_non_resource_is_reported() {
    let layer = corpus();
    let err = execute_with(
        r#"
        USING "urn:ex:Widget"
        USING NAMESPACE "urn:ex:"
        MATCH Widget(?w) { "urn:ex:size": ?s }
        WHERE ?s.size > 0
        RETURN [] { s: ?s }
        "#,
        &layer,
        FiberRuntime::default(),
    )
    .expect_err("an integer has no properties to walk");
    assert!(
        err.iter().any(|e| e.message.contains("not a resource IRI")),
        "the diagnostic should name the fault: {err:?}"
    );
}

/// **The control, and the reason the fix is a distinction rather than a blanket failure.**
///
/// A resource that does not carry the property is data ABSENCE: the condition is not
/// satisfied, that row drops, and the query succeeds. A heterogeneous chain produces this
/// constantly, so making it an error would fail most real queries. Only this case stays
/// silent; every fault above is now reported.
///
/// **A known limit this pins rather than fixes.** Absence is decided from the resource's
/// own properties, so a MISTYPED property name — one no resource carries — is
/// indistinguishable from one that legitimately no resource has, and both drop the row
/// silently. Telling them apart means checking the name against the declared vocabulary,
/// which needs the query's namespaces and so belongs at type-check time, not in per-row
/// evaluation where a chain-wide scan is the only alternative.
#[test]
fn a_resource_missing_the_property_drops_its_row_without_failing() {
    let boot = eigenius_kernel::testing::bootstrap_context();
    let mut b = LayerBuilder::new("partial", Some(Arc::clone(boot.head())));
    declare_vocabulary(&mut b);
    widget(&mut b, "urn:ex:w1", Some(1), 10);
    widget(&mut b, "urn:ex:w2", Some(2), 20);
    widget(&mut b, "urn:ex:w3", Some(3), 30);
    // The fourth carries no size at all.
    widget(&mut b, "urn:ex:w4", None, 40);
    let layer = Arc::new(b.build(LayerStorage::in_memory()));

    let rows = execute_with(
        r#"
        USING "urn:ex:Widget"
        USING NAMESPACE "urn:ex:"
        MATCH Widget(?w) { "urn:ex:weight": ?wt }
        WHERE ?w.size > 0
        RETURN [] { w: ?w }
        "#,
        &layer,
        FiberRuntime::default(),
    )
    .expect("absence is not a fault");
    assert_eq!(
        column(&rows, "w").len(),
        3,
        "the three with a size match; the one without drops, silently and correctly"
    );
}

// ── eigenius#123 ─────────────────────────────────────────────────────

/// **Two aggregates of the same operator are two columns, not one.**
///
/// The binding key was the operator alone, so `SUM(?s)` and `SUM(?wt)` both wrote to
/// `AGG#Sum` and the second won for both reads: two columns showing one number, with no
/// diagnostic. The sums here are 6 and 60, so a collision is unmistakable.
#[test]
fn two_sums_in_one_return_do_not_collide() {
    let layer = corpus();
    let rows = execute_with(
        r#"
        USING "urn:ex:Widget"
        USING NAMESPACE "urn:ex:"
        MATCH Widget(?w) { "urn:ex:size": ?s, "urn:ex:weight": ?wt }
        RETURN [] { total_size: SUM(?s), total_weight: SUM(?wt) }
        "#,
        &layer,
        FiberRuntime::default(),
    )
    .expect("query should succeed");
    assert_eq!(column(&rows, "total_size"), vec![Value::Integer(6)]);
    assert_eq!(column(&rows, "total_weight"), vec![Value::Integer(60)]);
}

// ── ORDER BY ─────────────────────────────────────────────────────────

/// **`ORDER BY` over an aggregate actually orders.**
///
/// Sort-value extraction matched only a bare `Expression::Variable` and returned `None`
/// for everything else, so for an aggregate both operands were `None`, the comparison was
/// skipped, and the order was left untouched. D2 §8.8's own worked example — group,
/// count, order by the count — therefore returned an unordered result with no error.
///
/// Ordering by a bare variable always worked, which is why this orders by the COUNT: a
/// test over the case that already worked would pass against the defect.
#[test]
fn order_by_an_aggregate_is_not_a_no_op() {
    let layer = grouped_corpus();
    let rows = execute_with(
        r#"
        USING "urn:ex:Widget"
        USING NAMESPACE "urn:ex:"
        MATCH Widget(?w) { "urn:ex:size": ?s }
        GROUP BY ?s
        RETURN [] { s: ?s, n: COUNT(?w) }
        ORDER BY COUNT(?w) DESC
        "#,
        &layer,
        FiberRuntime::default(),
    )
    .expect("query should succeed");
    // Group sizes are 3, 2 and 1, so descending by count is sizes 3, 2, 1 — the reverse
    // of the order the groups are built in.
    assert_eq!(
        column(&rows, "n"),
        vec![Value::Integer(3), Value::Integer(2), Value::Integer(1)],
        "descending by count"
    );
    assert_eq!(
        column(&rows, "s"),
        vec![Value::Integer(3), Value::Integer(2), Value::Integer(1)]
    );
}

/// Its control: ascending is the exact reverse, so the test above measures the sort rather
/// than agreeing with whatever order the groups happened to come out in.
#[test]
fn the_aggregate_sort_direction_is_honoured() {
    let layer = grouped_corpus();
    let rows = execute_with(
        r#"
        USING "urn:ex:Widget"
        USING NAMESPACE "urn:ex:"
        MATCH Widget(?w) { "urn:ex:size": ?s }
        GROUP BY ?s
        RETURN [] { s: ?s, n: COUNT(?w) }
        ORDER BY COUNT(?w) ASC
        "#,
        &layer,
        FiberRuntime::default(),
    )
    .expect("query should succeed");
    assert_eq!(
        column(&rows, "n"),
        vec![Value::Integer(1), Value::Integer(2), Value::Integer(3)]
    );
}

// ── eigenius#172 ─────────────────────────────────────────────────────

/// **Whitespace around `-` does not change what a query means.**
///
/// The lexer folded the minus into a following digit without consulting whitespace, so
/// `?s - 1` was a subtraction and `?s -1` was a variable beside a literal with no operator
/// between them. The additive loop stopped, the leftover number was carried past every
/// later clause, and the query died at end of input with a token complaint that named
/// neither the minus nor the place it went wrong.
#[test]
fn a_minus_means_the_same_with_or_without_a_space() {
    let layer = corpus();
    let run = |cond: &str| {
        execute_with(
            &format!(
                r#"
                USING "urn:ex:Widget"
                USING NAMESPACE "urn:ex:"
                MATCH Widget(?w) {{ "urn:ex:size": ?s }}
                WHERE {cond}
                RETURN [] {{ s: ?s }}
                "#
            ),
            &layer,
            FiberRuntime::default(),
        )
        .map(|rows| column(&rows, "s"))
    };
    let spaced = run("?s - 1 > 1").expect("the spaced form always parsed");
    let tight = run("?s -1 > 1").expect("the tight form must parse too");
    assert_eq!(spaced, tight, "spacing must not change the meaning");
    assert_eq!(
        tight,
        vec![Value::Integer(3)],
        "only size 3 satisfies s - 1 > 1"
    );
}

/// A negative literal is still a negative literal: the parser folds the sign the lexer
/// stopped folding.
#[test]
fn a_negative_literal_still_compares_as_one() {
    let layer = corpus();
    let rows = execute_with(
        r#"
        USING "urn:ex:Widget"
        USING NAMESPACE "urn:ex:"
        MATCH Widget(?w) { "urn:ex:size": ?s }
        WHERE ?s > -1
        RETURN [] { s: ?s }
        "#,
        &layer,
        FiberRuntime::default(),
    )
    .expect("query should succeed");
    assert_eq!(column(&rows, "s").len(), 3, "every size is above -1");
}
