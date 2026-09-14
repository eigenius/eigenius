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

//! `NOT EXISTS` over a property, and the optional-column shape it completes (eigenius#124,
//! eigenius#33).
//!
//! Asking for the resources that LACK a property had no expression. `NOT (?n.title == "x")`
//! is also true when the title is "y", and `NOT EXISTS` tested whether a VARIABLE was bound
//! — which under a strictly conjunctive `MATCH` is always true, so it matched nothing.
//!
//! The other half of what optionality was meant to buy already works, and these tests pin
//! it: a brace key REQUIRES its property, a dot-path does not, and an absent column is
//! omitted from its row rather than excluding the row.

use std::sync::Arc;

use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_kernel::query::{evaluate::FiberRuntime, execute_with};

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}

/// Two notebooks: one titled and saved, one neither — the case eigenius#33 reports as
/// invisible.
fn notebooks() -> Arc<Layer> {
    let boot = eigenius_kernel::testing::bootstrap_context();
    let mut b = LayerBuilder::new("notebooks", Some(Arc::clone(boot.head())));

    let mut c = Resource::new(iri("urn:ex:Notebook"));
    c.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(wk::CLASS.to_string())]),
    );
    c.set(iri(wk::DESCRIPTION), Value::String("probe class".into()));
    c.set(iri(wk::SHORT_NAME), Value::String("Notebook".into()));
    b.add_resource(c).unwrap();

    for (prop, short) in [
        ("urn:ex:owner", "owner"),
        ("urn:ex:title", "title"),
        ("urn:ex:modified", "modified"),
    ] {
        let mut p = Resource::new(iri(prop));
        p.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(wk::PROPERTY.to_string())]),
        );
        p.set(iri(wk::DESCRIPTION), Value::String("probe property".into()));
        p.set(iri(wk::SHORT_NAME), Value::String(short.into()));
        p.set(
            iri(wk::DATA_TYPE_PROP),
            Value::String("urn:eigenius:core:string".into()),
        );
        b.add_resource(p).unwrap();
    }

    for (id, title, modified) in [
        ("urn:ex:n1", Some("Alpha"), Some("2026-01-01")),
        ("urn:ex:n2", None, None),
    ] {
        let mut r = Resource::new(iri(id));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String("urn:ex:Notebook".into())]),
        );
        r.set(iri("urn:ex:owner"), Value::String("hm".into()));
        if let Some(t) = title {
            r.set(iri("urn:ex:title"), Value::String(t.into()));
        }
        if let Some(m) = modified {
            r.set(iri("urn:ex:modified"), Value::String(m.into()));
        }
        b.add_resource(r).unwrap();
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

fn run(layer: &Arc<Layer>, q: &str) -> Vec<Resource> {
    execute_with(q, layer, FiberRuntime::default())
        .unwrap_or_else(|e| panic!("query failed: {e:?}"))
}

/// The IRIs one column holds, in row order.
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

/// **The resources that LACK a property are now askable for.**
///
/// There was no expression for this. `NOT EXISTS` tested variable binding, which a
/// conjunctive `MATCH` always satisfies, so it returned false for every row.
#[test]
fn not_exists_over_a_property_finds_the_resources_without_it() {
    let layer = notebooks();
    let rows = run(
        &layer,
        r#"
        USING "urn:ex:Notebook"
        USING NAMESPACE "urn:ex:"
        MATCH Notebook(?n) { }
        WHERE NOT EXISTS(?n.title)
        RETURN [] { n: ?n }
        "#,
    );
    assert_eq!(column(&rows, "n"), vec!["urn:ex:n2".to_string()]);
}

/// Its control: the complement is the other notebook, so the predicate is selecting rather
/// than matching everything or nothing.
#[test]
fn the_complement_finds_the_resources_that_have_it() {
    let layer = notebooks();
    let rows = run(
        &layer,
        r#"
        USING "urn:ex:Notebook"
        USING NAMESPACE "urn:ex:"
        MATCH Notebook(?n) { }
        WHERE NOT (NOT EXISTS(?n.title))
        RETURN [] { n: ?n }
        "#,
    );
    assert_eq!(column(&rows, "n"), vec!["urn:ex:n1".to_string()]);
}

/// **A pin, not a fix** — this already passes, and that is the finding.
///
/// **A brace key REQUIRES its property; a dot-path does not.**
///
/// This is eigenius#33's motivating case. Through a brace, every untitled notebook is
/// invisible — the report's exact complaint, worked around with a round trip per row.
/// Through a dot-path the row is present and the absent columns are simply not carried.
#[test]
fn a_dot_path_projects_an_optional_column_without_excluding_the_row() {
    let layer = notebooks();

    let via_brace = run(
        &layer,
        r#"
        USING "urn:ex:Notebook"
        USING NAMESPACE "urn:ex:"
        MATCH Notebook(?n) { "urn:ex:owner": ?o, "urn:ex:title": ?t }
        RETURN [] { n: ?n, t: ?t }
        "#,
    );
    assert_eq!(
        column(&via_brace, "n"),
        vec!["urn:ex:n1".to_string()],
        "a brace key excludes the notebook that has no title"
    );

    let via_dot_path = run(
        &layer,
        r#"
        USING "urn:ex:Notebook"
        USING NAMESPACE "urn:ex:"
        MATCH Notebook(?n) { }
        RETURN [] { n: ?n, t: ?n.title }
        "#,
    );
    assert_eq!(
        column(&via_dot_path, "n"),
        vec!["urn:ex:n1".to_string(), "urn:ex:n2".to_string()],
        "a dot-path keeps both rows"
    );
    assert_eq!(
        column(&via_dot_path, "t"),
        vec!["Alpha".to_string()],
        "and only the one that has a title carries the column"
    );
}
