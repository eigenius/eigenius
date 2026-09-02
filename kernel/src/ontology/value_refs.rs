// Copyright 2026 the Eigenius authors.
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

//! The one walk over a `Value` for the IRIs it names (D85 R5a).
//!
//! Seven functions used to implement this walk — `declaration_order::value_refs`,
//! `supporting::collect_refs_from_value`, `merge/lca::collect_iri_refs_into`,
//! `merge/cascade::collect_orphaned_refs_in_value`, `merge/resolve::value_mentions_iri`,
//! `merge/resolve::substitute_iri_in_value` and `dcg/chart/attribute::value_refs` — plus the
//! JSON twin `term_mentions::json_mentions`, since deleted. They agreed about `String`,
//! `Integer` and `Array`, and disagreed about the two that matter:
//!
//! | variant | meaning | this walk |
//! |---|---|---|
//! | `Embedded` | a resource — an inline inductive value, or any other typed instance | descends; its IRIs are references |
//! | `Json` | `core:json`, and only that: opaque, "not validated by the ontology" | stops; an IRI inside opaque data is not a reference |
//!
//! The split existed because `Value::Json` carried both jobs at once — opaque payloads AND
//! inductive values, whose interior IRIs *are* references. D85 §6.1 separated them, so the
//! conditional some of these walks used (`if term_valued`, resolved against the property's
//! declared `data_type`) has nothing left to decide.
//!
//! **What varies between callers is the leaf action, not the walk.** So the walk is here once
//! and the action is a closure. A caller that wants only value leaves ignores
//! [`RefSite::Property`]; one that wants a property path reads the `path` argument; one that
//! rewrites uses [`map_refs`] instead of [`for_each_ref`].
//!
//! **A reference is a string that parses as an IRI.** There is no marker variant: `Value`'s
//! variants are shapes, not interpretations (D85 R5), and the one that used to mark a
//! reference — `ResourceRef` — was produced only at build time and never survived a storage
//! round trip, so a reloaded chain contributed no edges at all where it was relied on.

use super::iri::Iri;
use super::resource::{Resource, Value};

/// Where an IRI occurrence sits in a value tree.
///
/// Callers differ on whether a property KEY counts as a reference — `declaration_order` and
/// `supporting` count it, because a property's definition lives somewhere in the chain and so
/// is a dependency; the merge walks do not, because renaming a resource does not rename the
/// properties that point at it. The walk reports both and lets the caller decide.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RefSite {
    /// A property key on an embedded resource.
    Property,
    /// A string value that parses as an IRI.
    Value,
}

/// Visit every IRI a value names, in property order.
///
/// `f` receives the site, the IRI as it is written, and the property path from `value`'s owner
/// down to the occurrence — empty at the top level, one entry per `Embedded` descended. The
/// path is borrowed for the call and reused after it.
///
/// There is no early exit: a predicate caller sets a flag and lets the walk finish. Values are
/// small — a term of a few dozen nodes — and an exit path is a second control flow to keep
/// right.
pub fn for_each_ref(value: &Value, f: &mut impl FnMut(RefSite, &str, &[Iri])) {
    for_each_ref_where(value, &|_| true, f);
}

/// [`for_each_ref`], skipping the properties `descend` rejects — at every depth.
///
/// One caller needs this: `declaration_order` reads a layer for its dependency edges and
/// `core:domain` is not one. A property's `domain` says where it APPLIES; it does not say the
/// property needs that class declared first, and counting it makes every ordinary
/// class/property pair circular — the class `requires` the property, the property's `domain`
/// names the class back. The predicate is a filter on the walk rather than on its output
/// because the property key is skipped along with its value, and it applies inside an embedded
/// resource for the same reason it applies outside one.
pub fn for_each_ref_where(
    value: &Value,
    descend: &impl Fn(&Iri) -> bool,
    f: &mut impl FnMut(RefSite, &str, &[Iri]),
) {
    let mut path = Vec::new();
    walk_value(value, &mut path, descend, f);
}

/// Visit every IRI a resource's property values name, plus each property key.
///
/// The resource's own `@id` is not visited: it is the resource's identity, not a reference to
/// something else. A caller that rewrites identities handles it itself — see [`map_refs`],
/// whose `Embedded` arm does rewrite an id when one is present.
pub fn for_each_resource_ref(resource: &Resource, f: &mut impl FnMut(RefSite, &str, &[Iri])) {
    for_each_resource_ref_where(resource, &|_| true, f);
}

/// [`for_each_resource_ref`], skipping the properties `descend` rejects — see
/// [`for_each_ref_where`].
pub fn for_each_resource_ref_where(
    resource: &Resource,
    descend: &impl Fn(&Iri) -> bool,
    f: &mut impl FnMut(RefSite, &str, &[Iri]),
) {
    let mut path = Vec::new();
    walk_resource(resource, &mut path, descend, f);
}

fn walk_resource(
    resource: &Resource,
    path: &mut Vec<Iri>,
    descend: &impl Fn(&Iri) -> bool,
    f: &mut impl FnMut(RefSite, &str, &[Iri]),
) {
    for (prop, value) in resource.properties() {
        if !descend(prop) {
            continue;
        }
        f(RefSite::Property, prop.as_str(), path);
        path.push(prop.clone());
        walk_value(value, path, descend, f);
        path.pop();
    }
}

fn walk_value(
    value: &Value,
    path: &mut Vec<Iri>,
    descend: &impl Fn(&Iri) -> bool,
    f: &mut impl FnMut(RefSite, &str, &[Iri]),
) {
    match value {
        Value::String(s) => {
            if Iri::parse(s).is_ok() {
                f(RefSite::Value, s, path);
            }
        }
        Value::Array(items) => {
            for item in items {
                walk_value(item, path, descend, f);
            }
        }
        Value::Embedded(inner) => walk_resource(inner, path, descend, f),
        // See the module table: `Json` is opaque and every other variant is a scalar that
        // cannot hold a reference.
        Value::Integer(_) | Value::Float(_) | Value::Boolean(_) | Value::Json(_) => {}
    }
}

/// Rebuild a value, replacing every IRI-shaped string for which `f` returns `Some`.
///
/// The rebuilding half of the same walk: it visits exactly what [`for_each_ref`] visits, so a
/// caller cannot rewrite a site the read side does not see, or miss one it does. Property KEYS
/// are not rewritten — renaming a resource does not rename the properties pointing at it — but
/// an embedded resource's `@id` IS, because a rename that leaves the identity behind produces a
/// resource naming itself by its old IRI.
pub fn map_refs(value: &Value, f: &mut impl FnMut(&str) -> Option<String>) -> Value {
    match value {
        Value::String(s) => match Iri::parse(s).ok().and_then(|_| f(s)) {
            Some(replacement) => Value::String(replacement),
            None => value.clone(),
        },
        Value::Array(items) => Value::Array(items.iter().map(|v| map_refs(v, f)).collect()),
        Value::Embedded(inner) => Value::Embedded(Box::new(map_resource_refs(inner, f))),
        other => other.clone(),
    }
}

/// [`map_refs`] over a resource: its `@id`, then each property's value.
pub fn map_resource_refs(
    resource: &Resource,
    f: &mut impl FnMut(&str) -> Option<String>,
) -> Resource {
    let mut out = match resource.id() {
        Some(id) => match f(id.as_str()) {
            Some(replacement) => match Iri::parse(&replacement) {
                Ok(new_id) => Resource::new(new_id),
                // A replacement that is not an IRI cannot be an identity. Keeping the old one
                // is the only shape left, and the caller supplied the replacement, so it is
                // the caller's bug to find — not a reason to lose the resource.
                Err(_) => Resource::new(id.clone()),
            },
            None => Resource::new(id.clone()),
        },
        None => Resource::new_embedded(),
    };
    for (prop, value) in resource.properties() {
        out.set(prop.clone(), map_refs(value, f));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).expect("static iri")
    }

    fn refs(value: &Value) -> Vec<(RefSite, String, Vec<String>)> {
        let mut out = Vec::new();
        for_each_ref(value, &mut |site, s, path| {
            out.push((
                site,
                s.to_string(),
                path.iter().map(|p| p.as_str().to_string()).collect(),
            ))
        });
        out
    }

    /// **An IRI-shaped string is a reference; anything else is not.**
    #[test]
    fn a_string_that_parses_as_an_iri_is_a_reference() {
        assert_eq!(
            refs(&Value::String("urn:x:y".into())),
            vec![(RefSite::Value, "urn:x:y".to_string(), vec![])]
        );
        assert!(refs(&Value::String("not an iri".into())).is_empty());
        assert!(refs(&Value::Integer(3)).is_empty());
    }

    /// **`Json` is opaque.** An IRI-shaped string inside a `core:json` payload — a solver
    /// result, a `*_kv` map — is data. This is the arm the seven walks disagreed about.
    #[test]
    fn an_iri_inside_an_opaque_json_payload_is_not_a_reference() {
        let v = Value::Json(serde_json::json!({"target": "urn:x:y", "nested": ["urn:a:b"]}));
        assert!(refs(&v).is_empty(), "{:?}", refs(&v));
    }

    /// **`Embedded` is descended, keys and values both, with the path recorded.** This is the
    /// other arm they disagreed about, and the one D85 §6.1 made load-bearing: an inductive
    /// value is an embedded resource, and its interior IRIs are references.
    #[test]
    fn an_embedded_resource_is_descended_with_its_property_path() {
        let mut inner = Resource::new_embedded();
        inner.set(iri("urn:p:inner"), Value::String("urn:x:leaf".into()));
        let mut outer = Resource::new_embedded();
        outer.set(iri("urn:p:outer"), Value::Embedded(Box::new(inner)));

        assert_eq!(
            refs(&Value::Embedded(Box::new(outer))),
            vec![
                (RefSite::Property, "urn:p:outer".to_string(), vec![]),
                (
                    RefSite::Property,
                    "urn:p:inner".to_string(),
                    vec!["urn:p:outer".to_string()]
                ),
                (
                    RefSite::Value,
                    "urn:x:leaf".to_string(),
                    vec!["urn:p:outer".to_string(), "urn:p:inner".to_string()]
                ),
            ]
        );
    }

    /// **A skipped property is skipped at every depth.** The filter is on the walk, not on its
    /// output, so a property the caller excludes takes its value with it — inside an embedded
    /// resource as much as outside one. `declaration_order` is the caller that needs this, for
    /// `core:domain`, and it had it before the seven walks became one: its skip was recursive
    /// because its `Embedded` arm re-entered the same function.
    #[test]
    fn a_skipped_property_is_skipped_inside_an_embedded_resource_too() {
        let mut inner = Resource::new_embedded();
        inner.set(iri("urn:p:skipped"), Value::String("urn:hidden:one".into()));
        inner.set(iri("urn:p:kept"), Value::String("urn:visible:one".into()));
        let mut outer = Resource::new_embedded();
        outer.set(iri("urn:p:wrapper"), Value::Embedded(Box::new(inner)));

        let mut seen = Vec::new();
        for_each_resource_ref_where(
            &outer,
            &|prop| prop.as_str() != "urn:p:skipped",
            &mut |_site, s, _path| seen.push(s.to_string()),
        );
        assert!(
            !seen.iter().any(|s| s.contains("hidden")),
            "the skipped property's value must not be reported: {seen:?}"
        );
        assert!(
            !seen.iter().any(|s| s == "urn:p:skipped"),
            "nor its key: {seen:?}"
        );
        assert!(
            seen.iter().any(|s| s == "urn:visible:one"),
            "its sibling still is: {seen:?}"
        );
    }

    /// **A resource's own `@id` is not one of its references.** It is what the resource IS.
    #[test]
    fn a_resources_own_id_is_not_a_reference() {
        let mut r = Resource::new(iri("urn:me:self"));
        r.set(iri("urn:p:x"), Value::String("urn:other:thing".into()));
        let mut out = Vec::new();
        for_each_resource_ref(&r, &mut |site, s, _| out.push((site, s.to_string())));
        assert!(
            !out.iter().any(|(_, s)| s == "urn:me:self"),
            "the resource's own id must not be reported: {out:?}"
        );
    }

    /// **The rebuild visits exactly what the read visits.** Property keys are left alone; an
    /// embedded `@id` is rewritten; `Json` is untouched.
    #[test]
    fn the_rebuild_rewrites_value_leaves_and_ids_but_not_property_keys() {
        let mut inner = Resource::new(iri("urn:old:thing"));
        inner.set(iri("urn:old:thing"), Value::String("urn:old:thing".into()));
        inner.set(
            iri("urn:p:blob"),
            Value::Json(serde_json::json!("urn:old:thing")),
        );
        let v = Value::Embedded(Box::new(inner));

        let out = map_refs(&v, &mut |s| {
            (s == "urn:old:thing").then(|| "urn:new:thing".to_string())
        });
        let Value::Embedded(r) = &out else {
            panic!("still embedded")
        };
        assert_eq!(
            r.id().map(|i| i.as_str()),
            Some("urn:new:thing"),
            "the id is rewritten"
        );
        assert_eq!(
            r.get(&iri("urn:old:thing")).and_then(|v| v.as_str()),
            Some("urn:new:thing"),
            "the value leaf is rewritten under the UNCHANGED property key"
        );
        assert_eq!(
            r.get(&iri("urn:p:blob")),
            Some(&Value::Json(serde_json::json!("urn:old:thing"))),
            "an opaque payload is untouched"
        );
    }
}
