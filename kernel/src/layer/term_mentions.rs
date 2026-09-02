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

//! **D79 §2.2 — what a D47-encoded term mentions.**
//!
//! After D76 the chain binds names to declarations, so a resource carrying a
//! proposition depends on every declaration its term names — a `ConstRef` inside an
//! encoded proposition, an inductive named in a `ctor_type`, an axiom cited in a
//! justification. Those dependencies are real and, before this module, unqueryable:
//! encoded terms live in `Value::Json`, and `extract_indexable_triples` emitted
//! triples only for IRI-shaped values under `resource` / `resource_array`
//! predicates, so a term contributed **no triples at all**.
//!
//! **One extraction, two consumers.** This is the third place that needed to walk a
//! term for references. `layer::supporting`'s walker skips `Value::Json` (correctly,
//! for its purpose); `layer::declaration_order` therefore had to hand-write a
//! descent, whose doc comment records that reusing the first *"would produce an
//! empty graph for precisely the case `OrderError::MutualInductives` exists to
//! catch, and would look like it worked"*. Rather than write a third,
//! `declaration_order` and the indexer now share this one.
//!
//! **Call this only on a value whose property is declared `core:inductive`.** That
//! restriction is the whole safety argument, not a caveat. A `core:json` value is
//! *"an opaque JSON value, not validated by the ontology"* — the Julia solver
//! payloads, the `*_kv` maps, an institution's witness blob — and an IRI-shaped
//! string inside one is **data, not a reference**. Treating it as a reference would
//! index a dependency that does not exist, and in a rewriting caller (a merge
//! rename) it would corrupt the payload. The indexer therefore reaches this only
//! from the `wk::INDUCTIVE` arm of `extract_indexable_triples`, never from
//! `wk::JSON`.
//!
//! This is why D79 §2.1's declaration cleanup is a **prerequisite** rather than a
//! tidy-up: before it, twenty-two term-valued properties were declared
//! `core:resource` and `core:ctor_type` was `core:json`, so the carrier's data type
//! could not tell a term from a blob and no caller could make this distinction
//! safely.
//!
//! **Within a term, structural and deliberately over-approximate.** Any string that
//! parses as a `urn:` IRI counts, rather than only `ConstRef`'s and `CtorApp`'s first
//! argument. Two reasons: it costs no decode, and it cannot go stale when the encoder
//! gains an IRI-bearing form — a walker enumerating the forms it knew would silently
//! stop seeing the new one, which is the failure mode this module exists to end. The
//! residual price is a `LitString` inside a term holding something IRI-shaped, which
//! is counted as a mention. Sound for a consumer asking "what might this depend on",
//! where a false positive costs an extra check and a false negative is a missed
//! invalidation.

use crate::ontology::iri::Iri;
use crate::ontology::resource::Value;
use std::collections::BTreeSet;

/// Every IRI a D47-encoded term names, at any depth.
pub fn json_mentions(j: &serde_json::Value, out: &mut BTreeSet<Iri>) {
    match j {
        serde_json::Value::String(s) => {
            if s.starts_with("urn:") {
                if let Ok(iri) = Iri::parse(s) {
                    out.insert(iri);
                }
            }
        }
        serde_json::Value::Array(items) => items.iter().for_each(|i| json_mentions(i, out)),
        serde_json::Value::Object(map) => map.values().for_each(|v| json_mentions(v, out)),
        _ => {}
    }
}

/// Every IRI a term-valued property names, in either shape.
///
/// `Value::Embedded` IS descended. It was not until `2026-09-01`, on the reasoning that "an
/// embedded resource is validated and indexed as a resource in its own right" — true of
/// validation, false of the index: `extract_indexable_triples` iterates `layer.iter_resources()`,
/// which walks `defined_iris`, and a value resource has no `@id`. Nothing else would reach it.
///
/// This matters from D85 §6.1 onward, where a term stops being a `Value::Json` blob and becomes
/// a resource whose `is_a` names its constructor's class. Skipping `Embedded` would have emitted
/// zero mentions for every migrated value — the same silent hole this module was written to end,
/// arrived at from the other side.
///
/// The mention set is a superset of the old one: a value resource names its constructor class in
/// `is_a`, which the tagged form spelled as the bare string `"ConstRef"` and no consumer could
/// resolve. The class is a real dependency — Rule 25 requires it to be declared — so counting it
/// is the correct answer, not an over-approximation to apologise for.
pub fn json_mentions_of_value(v: &Value, out: &mut BTreeSet<Iri>) {
    match v {
        Value::Json(j) => json_mentions(j, out),
        Value::Array(items) => items.iter().for_each(|i| json_mentions_of_value(i, out)),
        // Same rule as inside a term: any string that parses as a `urn:` IRI counts.
        Value::String(s) if s.starts_with("urn:") => {
            if let Ok(iri) = Iri::parse(s) {
                out.insert(iri);
            }
        }
        Value::Embedded(r) => r
            .properties()
            .values()
            .for_each(|v| json_mentions_of_value(v, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_const_ref_inside_a_term_is_a_mention() {
        let mut out = BTreeSet::new();
        json_mentions(
            &serde_json::json!({"ctor": "ConstRef", "args": ["urn:eigenius:test:Nat"]}),
            &mut out,
        );
        assert_eq!(out.len(), 1);
        assert!(out.iter().any(|i| i.as_str() == "urn:eigenius:test:Nat"));
    }

    /// `CtorApp` names the inductive, not the constructor — constructors have no
    /// chain-resolvable identity (D79 §2.2.1), so the mention is to the type.
    #[test]
    fn a_ctor_app_mentions_the_inductive_not_the_constructor() {
        let mut out = BTreeSet::new();
        json_mentions(
            &serde_json::json!({"ctor": "CtorApp", "args": ["urn:eigenius:test:Nat", "succ"]}),
            &mut out,
        );
        assert_eq!(
            out.len(),
            1,
            "the ctor name `succ` is not an IRI and must not become one: {out:?}"
        );
        assert!(out.iter().any(|i| i.as_str() == "urn:eigenius:test:Nat"));
    }

    #[test]
    fn nested_applications_are_reached() {
        let mut out = BTreeSet::new();
        json_mentions(
            &serde_json::json!({"ctor": "App", "args": [
                {"ctor": "App", "args": [
                    {"ctor": "ConstRef", "args": ["urn:eigenius:lexicon:cat_np"]},
                    {"ctor": "ConstRef", "args": ["urn:eigenius:wn:n00001740"]}]},
                {"ctor": "ConstRef", "args": ["urn:eigenius:lexicon:num_sg"]}]}),
            &mut out,
        );
        assert_eq!(out.len(), 3, "every ConstRef in the spine: {out:?}");
    }

    /// D85 §6.1 — the shape the tagged dict became. The index reaches a value resource ONLY
    /// through this walker, so a missing arm here is a missing dependency edge, silently.
    #[test]
    fn a_value_resource_mentions_what_it_names() {
        use crate::ontology::resource::Resource;
        let mut inner = Resource::new_embedded();
        inner.set(
            Iri::parse("urn:eigenius:core:is_a").unwrap(),
            Value::Array(vec![Value::String(
                "urn:eigenius:eigentt:Term-ConstRef".into(),
            )]),
        );
        inner.set(
            Iri::parse("urn:eigenius:eigentt:Term-ConstRef-iri").unwrap(),
            Value::String("urn:eigenius:core:Level".into()),
        );
        let mut out = BTreeSet::new();
        json_mentions_of_value(&Value::Embedded(Box::new(inner)), &mut out);
        let got: Vec<&str> = out.iter().map(Iri::as_str).collect();
        assert_eq!(
            got,
            vec![
                "urn:eigenius:core:Level",
                "urn:eigenius:eigentt:Term-ConstRef"
            ],
            "the referenced constant AND the constructor class both count"
        );
    }

    #[test]
    fn a_term_with_no_iris_mentions_nothing() {
        let mut out = BTreeSet::new();
        json_mentions(
            &serde_json::json!({"ctor": "Sort", "args": [{"ctor": "Zero", "args": []}]}),
            &mut out,
        );
        assert!(out.is_empty(), "{out:?}");
    }
}
