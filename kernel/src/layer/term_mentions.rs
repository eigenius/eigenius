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
use crate::ontology::resource::{Resource, Value};
use std::collections::BTreeSet;

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
pub fn json_mentions_of_value(v: &Value, layer: &crate::layer::Layer, out: &mut BTreeSet<Iri>) {
    match v {
        Value::Array(items) => items
            .iter()
            .for_each(|i| json_mentions_of_value(i, layer, out)),
        // STRICT (B6): a bare string is not a reference. Where one is, a declaration says so —
        // `spine_mentions` reads the constructor's argument types, and `is_a` is reached as a
        // class below. This is the narrowing: a `LitString` whose contents happen to be
        // IRI-shaped is data, and indexing it invented a dependency nothing declared.
        Value::String(_) => {}
        Value::Embedded(r) => {
            // The value names its constructor's class in `is_a`, and Rule 25 requires that class
            // to be declared — a real dependency, reached here rather than by string shape.
            for c in r.is_a() {
                out.insert(c.clone());
            }
            // B6 — an application spine whose head names a constructor can be read against that
            // constructor's DECLARED argument types instead of guessed at. When that read
            // succeeds it decides every argument; when it does not, fall through to the
            // structural walk below.
            if spine_mentions(r, layer, out) {
                return;
            }
            for (prop_iri, val) in r.properties() {
                mentions_under_property(prop_iri, val, layer, out);
            }
        }
        _ => {}
    }
}

/// The mentions carried by one property of a value resource, decided by that property's declared
/// `core:data_type` — the second half of B6, and the one that reaches a `ConstRef`.
///
/// A `ConstRef` is not an application: it is a value resource whose target sits on
/// `Term-ConstRef-iri`. `spine_mentions` cannot see it, so the property's own declaration is what
/// says the string is a reference. `ctor_classes` derives these properties from the inductive's
/// `core:ctors`, so the type is on the chain rather than in this reader.
fn mentions_under_property(
    prop_iri: &Iri,
    v: &Value,
    layer: &crate::layer::Layer,
    out: &mut BTreeSet<Iri>,
) {
    use crate::ontology::well_known as wk;
    let data_type = layer
        .resolve(prop_iri)
        .and_then(|p| {
            Iri::parse(wk::DATA_TYPE_PROP)
                .ok()
                .and_then(|k| p.get(&k).cloned())
        })
        .and_then(|v| v.as_str().map(str::to_string));
    match data_type.as_deref() {
        Some(wk::IRI_TYPE) | Some(wk::RESOURCE) | Some(wk::RESOURCE_ARRAY) => {
            push_iri_strings(v, out)
        }
        _ => json_mentions_of_value(v, layer, out),
    }
}

/// Every IRI-parsable string in `v`, one level of array included.
fn push_iri_strings(v: &Value, out: &mut BTreeSet<Iri>) {
    match v {
        Value::String(s) => {
            if let Ok(iri) = Iri::parse(s) {
                out.insert(iri);
            }
        }
        Value::Array(items) => items.iter().for_each(|i| push_iri_strings(i, out)),
        _ => {}
    }
}

/// Read an `App` spine against its head constructor's declared argument types.
///
/// **B6 / D88 §3.** Returns `true` when it handled the node — the spine's head is a `CtorApp`
/// naming a constructor whose argument types resolve — and `false` to fall back.
///
/// **Why the fallback is the heuristic and not silence.** A premise citation that stops reaching
/// the index is a dependency the well-foundedness check cannot see, and P6 enforces
/// well-foundedness over exactly these edges. Over-approximating is what this module does today;
/// under-approximating would be new, and wrong in the direction that loses data. So an
/// unresolvable constructor keeps the old behaviour for that subtree.
fn spine_mentions(r: &Resource, layer: &crate::layer::Layer, out: &mut BTreeSet<Iri>) -> bool {
    let Some((head, args)) = collect_spine(r) else {
        return false;
    };
    let Some((decl_iri, ctor_name)) = ctor_app_target(head) else {
        return false;
    };
    let Some(arg_types) = explicit_arg_types(layer, &decl_iri, &ctor_name) else {
        return false;
    };
    // The constructor and the inductive are both dependencies of any term naming them.
    out.insert(decl_iri);
    for c in head.is_a() {
        out.insert(c.clone());
    }
    // Spine arguments fill the explicit binders in order. There may be FEWER than the telescope
    // declares — an elided witness slot, an implicit binder — so a positional prefix is the
    // correspondence, and a spine longer than the telescope falls back rather than guessing.
    if args.len() > arg_types.len() {
        return false;
    }
    for (arg, ty) in args.iter().zip(arg_types.iter()) {
        match ty.as_str() {
            // Declared to hold a reference. The argument arrives as a `LitString` — the term
            // language has no IRI former — so its string IS the mention.
            crate::ontology::well_known::IRI_TYPE => lit_string_iri(arg, out),
            // Anything else is either another term (recurse) or data (nothing). Recursing into a
            // non-term is harmless: a literal's own value sits under a `core:string` property and
            // contributes nothing.
            _ => json_mentions_of_value(&Value::Embedded(Box::new(arg.clone())), layer, out),
        }
    }
    true
}

/// Flatten `App(App(App(h, a), b), c)` into `(h, [a, b, c])`, or `None` if `r` is not an `App`.
fn collect_spine(r: &Resource) -> Option<(&Resource, Vec<Resource>)> {
    fn head_arg(r: &Resource) -> Option<(&Resource, &Resource)> {
        let is_app = r.is_a().first()?.as_str().ends_with("-App");
        if !is_app {
            return None;
        }
        let mut head = None;
        let mut arg = None;
        for (k, v) in r.properties() {
            let Value::Embedded(inner) = v else { continue };
            if k.as_str().ends_with("-head") {
                head = Some(inner.as_ref());
            } else if k.as_str().ends_with("-arg") {
                arg = Some(inner.as_ref());
            }
        }
        Some((head?, arg?))
    }
    let (mut h, a) = head_arg(r)?;
    let mut args = vec![a.clone()];
    while let Some((inner_h, inner_a)) = head_arg(h) {
        args.push(inner_a.clone());
        h = inner_h;
    }
    args.reverse();
    Some((h, args))
}

/// `(inductive, ctor_name)` when `r` is a `CtorApp` value resource.
fn ctor_app_target(r: &Resource) -> Option<(Iri, String)> {
    if !r.is_a().first()?.as_str().ends_with("-CtorApp") {
        return None;
    }
    let mut decl = None;
    let mut name = None;
    for (k, v) in r.properties() {
        let Some(s) = v.as_str() else { continue };
        if k.as_str().ends_with("-decl_iri") {
            decl = Iri::parse(s).ok();
        } else if k.as_str().ends_with("-ctor_name") {
            name = Some(s.to_string());
        }
    }
    Some((decl?, name?))
}

/// A constructor's EXPLICIT argument types, in spine order, as the IRIs they name.
///
/// Two declaration forms, because the tree has both (D88 §3): a positional constructor carries
/// `core:arg_types`, and an indexed one carries its whole type as a `core:ctor_type` telescope.
/// Binders named in `core:implicit_args` (B1) are skipped — they are not written at the use site,
/// so they occupy no spine position.
fn explicit_arg_types(
    layer: &crate::layer::Layer,
    inductive: &Iri,
    ctor_name: &str,
) -> Option<Vec<String>> {
    use crate::ontology::well_known as wk;
    let ind = layer.resolve(inductive)?;
    let Value::Array(ctors) = ind.get(&Iri::parse(wk::CTORS).ok()?)? else {
        return None;
    };
    let ctor = ctors.iter().find_map(|c| match c {
        Value::Embedded(r)
            if r.get(&Iri::parse(wk::CTOR_NAME).ok()?)
                .and_then(|v| v.as_str())
                == Some(ctor_name) =>
        {
            Some(r.as_ref())
        }
        _ => None,
    })?;

    let implicit: BTreeSet<String> = match ctor.get(&Iri::parse(wk::IMPLICIT_ARGS).ok()?) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => BTreeSet::new(),
    };

    // Positional form.
    if let Some(Value::Array(args)) = ctor.get(&Iri::parse(wk::ARG_TYPES).ok()?) {
        return Some(
            args.iter()
                .filter_map(|a| match a {
                    Value::Embedded(r) => {
                        let name = r
                            .get(&Iri::parse(wk::ARG_NAME).ok()?)
                            .and_then(|v| v.as_str())
                            .unwrap_or_default();
                        if implicit.contains(name) {
                            return None;
                        }
                        Some(const_ref_iri(r.get(&Iri::parse(wk::TYPE_NAME).ok()?)?))
                    }
                    _ => None,
                })
                .collect(),
        );
    }

    // Telescope form: walk the `Pi` chain, taking each binder's domain.
    let ty = ctor.get(&Iri::parse(wk::CTOR_TYPE).ok()?)?;
    let mut out = Vec::new();
    let mut cur = match ty {
        Value::Embedded(r) => r.as_ref().clone(),
        _ => return None,
    };
    loop {
        if !cur.is_a().first().is_some_and(|c| {
            let c = c.as_str();
            c.ends_with("-Pi") || c.ends_with("-Arrow")
        }) {
            break;
        }
        let mut name = String::new();
        let mut dom = None;
        let mut body = None;
        for (k, v) in cur.properties() {
            let ks = k.as_str();
            if ks.ends_with("-name") {
                name = v.as_str().unwrap_or_default().to_string();
            } else if ks.ends_with("-dom") {
                dom = Some(v.clone());
            } else if ks.ends_with("-body") {
                if let Value::Embedded(b) = v {
                    body = Some(b.as_ref().clone());
                }
            }
        }
        if !implicit.contains(&name) {
            out.push(dom.as_ref().map(const_ref_iri).unwrap_or_default());
        }
        match body {
            Some(b) => cur = b,
            None => break,
        }
    }
    Some(out)
}

/// The IRI a `ConstRef` domain names, or the empty string for any other shape. A domain that is
/// not a bare constant — `Prop`, an application, another Pi — is not a reference slot, and the
/// empty string routes it to the recursive arm.
fn const_ref_iri(v: &Value) -> String {
    match v {
        Value::Embedded(r)
            if r.is_a()
                .first()
                .is_some_and(|c| c.as_str().ends_with("-ConstRef")) =>
        {
            r.properties()
                .iter()
                .find(|(k, _)| k.as_str().ends_with("-iri"))
                .and_then(|(_, v)| v.as_str())
                .unwrap_or_default()
                .to_string()
        }
        Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

/// A `LitString`'s value, as a mention. The term language has no IRI former, so a slot declared
/// `core:iri` is filled by a `LitString` whose string is the reference.
fn lit_string_iri(r: &Resource, out: &mut BTreeSet<Iri>) {
    for c in r.is_a() {
        out.insert(c.clone());
    }
    for (k, v) in r.properties() {
        if k.as_str().ends_with("-value") {
            if let Some(s) = v.as_str() {
                if let Ok(i) = Iri::parse(s) {
                    out.insert(i);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_const_ref_inside_a_term_is_a_mention() {
        let mut out = BTreeSet::new();
        json_mentions_of_value(
            &crate::testing::term_value(
                &serde_json::json!({"ctor": "ConstRef", "args": ["urn:eigenius:test:Nat", []]}),
            ),
            crate::testing::term_chain(),
            &mut out,
        );
        // The referenced constant AND `Term-ConstRef`, the class the value states. Both are
        // real dependencies: the class must be declared for the value to be well formed.
        assert!(out.iter().any(|i| i.as_str() == "urn:eigenius:test:Nat"));
        assert!(out
            .iter()
            .any(|i| i.as_str() == "urn:eigenius:eigentt:Term-ConstRef"));
    }

    /// `CtorApp` names the inductive, not the constructor — constructors have no
    /// chain-resolvable identity (D79 §2.2.1), so the mention is to the type.
    #[test]
    fn a_ctor_app_mentions_the_inductive_not_the_constructor() {
        let mut out = BTreeSet::new();
        json_mentions_of_value(
            &crate::testing::term_value(
                &serde_json::json!({"ctor": "CtorApp", "args": ["urn:eigenius:test:Nat", "succ"]}),
            ),
            crate::testing::term_chain(),
            &mut out,
        );
        assert!(
            !out.iter().any(|i| i.as_str().ends_with(":succ")),
            "the ctor name `succ` is not an IRI and must not become one: {out:?}"
        );
        assert!(out.iter().any(|i| i.as_str() == "urn:eigenius:test:Nat"));
    }

    #[test]
    fn nested_applications_are_reached() {
        let mut out = BTreeSet::new();
        json_mentions_of_value(
            &crate::testing::term_value(&serde_json::json!({"ctor": "App", "args": [
                {"ctor": "App", "args": [
                    {"ctor": "ConstRef", "args": ["urn:eigenius:lexicon:cat_np", []]},
                    {"ctor": "ConstRef", "args": ["urn:eigenius:wn:n00001740", []]}]},
                {"ctor": "ConstRef", "args": ["urn:eigenius:lexicon:num_sg", []]}]})),
            crate::testing::term_chain(),
            &mut out,
        );
        for expected in [
            "urn:eigenius:lexicon:cat_np",
            "urn:eigenius:wn:n00001740",
            "urn:eigenius:lexicon:num_sg",
        ] {
            assert!(
                out.iter().any(|i| i.as_str() == expected),
                "every ConstRef in the spine is reached; missing {expected}: {out:?}"
            );
        }
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
        json_mentions_of_value(
            &Value::Embedded(Box::new(inner)),
            crate::testing::term_chain(),
            &mut out,
        );
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
    /// A term with no IRIs of its own still mentions the CLASSES it instantiates, and nothing
    /// else. `Sort(Zero)` names no constant, so the only mentions are `eigentt:Term-Sort` and
    /// `core:Level-Zero` — both declared, both real dependencies of the value.
    fn a_term_with_no_constants_mentions_only_its_classes() {
        let mut out = BTreeSet::new();
        json_mentions_of_value(
            &crate::testing::term_value(
                &serde_json::json!({"ctor": "Sort", "args": [{"ctor": "Zero", "args": []}]}),
            ),
            crate::testing::term_chain(),
            &mut out,
        );
        let got: Vec<&str> = out.iter().map(Iri::as_str).collect();
        assert_eq!(
            got,
            vec![
                "urn:eigenius:core:Level-Zero",
                "urn:eigenius:eigentt:Term-Sort"
            ],
            "only the constructor classes: {out:?}"
        );
    }

    /// **What B6 is for**: a string that merely LOOKS like an IRI, in a slot declared to hold
    /// data, is no longer a dependency.
    ///
    /// The predecessor matched `s.starts_with("urn:")` at any depth, so a `LitString` carrying an
    /// audit tag or an IRI-shaped payload was indexed as a reference to a declaration nothing
    /// named. `Term-LitString-value` is declared `core:string`; reading the declaration is what
    /// tells it apart from `Term-ConstRef-iri`, which is declared `core:iri`.
    #[test]
    fn a_lit_string_that_looks_like_an_iri_is_not_a_mention() {
        let mut out = BTreeSet::new();
        json_mentions_of_value(
            &crate::testing::term_value(
                &serde_json::json!({"ctor": "LitString", "args": ["urn:eigenius:test:NotADep"]}),
            ),
            crate::testing::term_chain(),
            &mut out,
        );
        assert!(
            !out.iter()
                .any(|i| i.as_str() == "urn:eigenius:test:NotADep"),
            "a `core:string` slot holds DATA — indexing its contents invents a dependency on a \
             declaration nothing referenced. Got {out:?}"
        );
        // The constructor's class is still a dependency: the value names it in `is_a`, and
        // Rule 25 requires it to be declared.
        assert!(out
            .iter()
            .any(|i| i.as_str() == "urn:eigenius:eigentt:Term-LitString"));
    }
}
