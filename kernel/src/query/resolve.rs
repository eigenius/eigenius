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

use crate::layer::{typed_resource_iris, Layer};
use crate::ontology::iri::Iri;
use crate::ontology::resource::Value;
use crate::ontology::well_known as wk;
use crate::query::error::QueryError;

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
            Value::Array(vec![Value::ResourceRef(iri(wk::CLASS))]),
        );
        r.set(iri(wk::SHORT_NAME), Value::String(short.into()));
        r
    }

    /// A layer on the bootstrap core with two same-short-name `Widget` classes in
    /// two distinct namespaces plus one `Gadget` in `urn:a:` only.
    fn layer_with_classes() -> Arc<Layer> {
        let ctx = crate::bootstrap::bootstrap().expect("bootstrap");
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
