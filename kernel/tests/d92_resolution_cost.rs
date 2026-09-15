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

//! D92 — what resolving each reference once instead of two or three times costs.
//!
//! `resolve_scoped_name` enumerates the typed-resource IRIs for a metaclass and resolves
//! each candidate through the chain to read its `short_name`, so the cost is O(vocabulary
//! × chain depth) per lookup. Before D92 a pattern class was resolved three times per
//! query, a FIBER query class twice, and the FIBER param table built twice.
//!
//! Run with:
//!
//! ```text
//! cargo test -p eigenius-kernel --test d92_resolution_cost --release -- --ignored --nocapture
//! ```

use std::sync::Arc;
use std::time::Instant;

use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}

fn strings(v: &[&str]) -> Value {
    Value::Array(v.iter().map(|s| Value::String((*s).to_string())).collect())
}

/// Bootstrap + one layer declaring `props` properties and a class that declares five of
/// them + `depth` filler layers, so every chain walk pays for the depth.
fn deep_chain(props: usize, depth: usize) -> Arc<Layer> {
    let boot = eigenius_kernel::testing::bootstrap_context();
    let mut b = LayerBuilder::new("vocab", Some(Arc::clone(boot.head())));
    for i in 0..props {
        let mut p = Resource::new(iri(&format!("urn:ex:p{i}")));
        p.set(iri(wk::IS_A), strings(&[wk::PROPERTY]));
        p.set(iri(wk::DESCRIPTION), Value::String("probe".into()));
        p.set(iri(wk::SHORT_NAME), Value::String(format!("p{i}")));
        p.set(
            iri(wk::DATA_TYPE_PROP),
            Value::String("urn:eigenius:core:string".into()),
        );
        b.add_resource(p).unwrap();
    }
    let mut c = Resource::new(iri("urn:ex:Big"));
    c.set(iri(wk::IS_A), strings(&[wk::CLASS]));
    c.set(iri(wk::DESCRIPTION), Value::String("probe class".into()));
    c.set(iri(wk::SHORT_NAME), Value::String("Big".into()));
    c.set(
        iri(wk::RECOMMENDS),
        strings(&[
            "urn:ex:p0",
            "urn:ex:p1",
            "urn:ex:p2",
            "urn:ex:p3",
            "urn:ex:p4",
        ]),
    );
    b.add_resource(c).unwrap();
    let mut head = Arc::new(b.build(LayerStorage::in_memory()));

    for d in 0..depth {
        let mut f = LayerBuilder::new(&format!("filler{d}"), Some(Arc::clone(&head)));
        let mut r = Resource::new(iri(&format!("urn:ex:filler{d}")));
        r.set(iri(wk::IS_A), strings(&[wk::CLASS]));
        r.set(iri(wk::DESCRIPTION), Value::String("filler".into()));
        r.set(iri(wk::SHORT_NAME), Value::String(format!("Filler{d}")));
        f.add_resource(r).unwrap();
        head = Arc::new(f.build(LayerStorage::in_memory()));
    }
    head
}

/// Parse, stratify, resolve and type-check — everything before evaluation.
fn front_end_millis(layer: &Arc<Layer>, q: &str) -> u128 {
    let started = Instant::now();
    let tokens = eigenius_kernel::query::lexer::tokenize(q).expect("lexes");
    let program = eigenius_kernel::query::parser::parse(tokens).expect("parses");
    let strata =
        eigenius_kernel::query::stratify::stratify(&program.definitions).expect("stratifies");
    let index =
        eigenius_kernel::institution::registry::InstitutionIndex::from_layer_indexed(layer).0;
    let resolved = eigenius_kernel::query::resolve::resolve(program, strata, layer, &index)
        .unwrap_or_else(|e| panic!("resolve: {e:?}"));
    let errors = eigenius_kernel::query::type_check::type_check(&resolved.program, layer, &index);
    assert!(errors.is_empty(), "type errors: {errors:?}");
    started.elapsed().as_millis()
}

const TYPED: &str = r#"
USING "urn:ex:Big"
MATCH "urn:ex:Big"(?x) { p0: ?a, p1: ?b, p2: ?c, p3: ?d, p4: ?e }
RETURN [] { a: ?a }
"#;

const UNTYPED: &str = r#"
USING NAMESPACE "urn:ex:"
MATCH ?x { p0: ?a, p1: ?b, p2: ?c, p3: ?d, p4: ?e }
RETURN [] { a: ?a }
"#;

const FULL_IRI: &str = r#"
MATCH ?x { "urn:ex:p0": ?a, "urn:ex:p1": ?b, "urn:ex:p2": ?c, "urn:ex:p3": ?d, "urn:ex:p4": ?e }
RETURN [] { a: ?a }
"#;

#[ignore = "measurement, not a gate: builds a 2000-property chain at several depths"]
#[test]
fn resolution_cost_by_depth() {
    println!("\n── D92 front-end cost (parse + stratify + resolve + type-check) ──");
    println!("  2000 declared properties; 5 property keys per query\n");
    println!(
        "  {:<8} {:>12} {:>12} {:>12}",
        "depth", "typed", "untyped", "full-IRI"
    );
    for depth in [0usize, 25, 50, 100] {
        let layer = deep_chain(2000, depth);
        // One warm pass so the first measurement is not paying for lazily-built indexes.
        let _ = front_end_millis(&layer, TYPED);
        let typed = front_end_millis(&layer, TYPED);
        let untyped = front_end_millis(&layer, UNTYPED);
        let full = front_end_millis(&layer, FULL_IRI);
        println!("  {depth:<8} {typed:>10}ms {untyped:>10}ms {full:>10}ms");
    }
    println!();
}

/// The same, with the vocabulary weighted toward CLASSES rather than properties.
///
/// D92 removed duplicate resolutions of pattern classes and FIBER query classes, not of
/// properties, so a chain with few classes cannot show the difference however many
/// properties it has. `resolve_scoped_name` is O(vocabulary of the metaclass × depth),
/// and the metaclass here is `core:Class`.
#[ignore = "measurement, not a gate"]
#[test]
fn resolution_cost_with_a_large_class_vocabulary() {
    println!("\n── D92 front-end cost, 2000 CLASSES ──");
    println!("  the metaclass whose lookups D92 de-duplicated\n");
    println!("  {:<8} {:>12}", "depth", "typed");
    for depth in [0usize, 25, 50, 100] {
        let layer = class_heavy_chain(2000, depth);
        let _ = front_end_millis(&layer, TYPED_SHORT);
        let typed = front_end_millis(&layer, TYPED_SHORT);
        println!("  {depth:<8} {typed:>10}ms");
    }
    println!();
}

/// `MATCH Big(?x)` — a SHORT-name class, so the class lookup actually runs. The full-IRI
/// form resolves without a scan and would measure nothing.
const TYPED_SHORT: &str = r#"
USING NAMESPACE "urn:ex:"
MATCH Big(?x) { p0: ?a, p1: ?b, p2: ?c, p3: ?d, p4: ?e }
RETURN [] { a: ?a }
"#;

/// Bootstrap + one layer of `classes` class declarations (plus the five properties `Big`
/// recommends) + `depth` filler layers.
fn class_heavy_chain(classes: usize, depth: usize) -> Arc<Layer> {
    let boot = eigenius_kernel::testing::bootstrap_context();
    let mut b = LayerBuilder::new("vocab", Some(Arc::clone(boot.head())));
    for i in 0..5 {
        let mut p = Resource::new(iri(&format!("urn:ex:p{i}")));
        p.set(iri(wk::IS_A), strings(&[wk::PROPERTY]));
        p.set(iri(wk::DESCRIPTION), Value::String("probe".into()));
        p.set(iri(wk::SHORT_NAME), Value::String(format!("p{i}")));
        p.set(
            iri(wk::DATA_TYPE_PROP),
            Value::String("urn:eigenius:core:string".into()),
        );
        b.add_resource(p).unwrap();
    }
    for i in 0..classes {
        let mut c = Resource::new(iri(&format!("urn:ex:C{i}")));
        c.set(iri(wk::IS_A), strings(&[wk::CLASS]));
        c.set(iri(wk::DESCRIPTION), Value::String("filler class".into()));
        c.set(iri(wk::SHORT_NAME), Value::String(format!("C{i}")));
        b.add_resource(c).unwrap();
    }
    let mut c = Resource::new(iri("urn:ex:Big"));
    c.set(iri(wk::IS_A), strings(&[wk::CLASS]));
    c.set(iri(wk::DESCRIPTION), Value::String("probe class".into()));
    c.set(iri(wk::SHORT_NAME), Value::String("Big".into()));
    c.set(
        iri(wk::RECOMMENDS),
        strings(&[
            "urn:ex:p0",
            "urn:ex:p1",
            "urn:ex:p2",
            "urn:ex:p3",
            "urn:ex:p4",
        ]),
    );
    b.add_resource(c).unwrap();
    let mut head = Arc::new(b.build(LayerStorage::in_memory()));
    for d in 0..depth {
        let mut f = LayerBuilder::new(&format!("filler{d}"), Some(Arc::clone(&head)));
        let mut r = Resource::new(iri(&format!("urn:ex:filler{d}")));
        r.set(iri(wk::IS_A), strings(&[wk::PROPERTY]));
        r.set(iri(wk::DESCRIPTION), Value::String("filler".into()));
        r.set(iri(wk::SHORT_NAME), Value::String(format!("filler{d}")));
        r.set(
            iri(wk::DATA_TYPE_PROP),
            Value::String("urn:eigenius:core:string".into()),
        );
        f.add_resource(r).unwrap();
        head = Arc::new(f.build(LayerStorage::in_memory()));
    }
    head
}
