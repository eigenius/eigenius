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

//! D76 Phase A — the order in which a layer's declarations must be processed.
//!
//! **Why this exists.** D76 §6 reads a layer as a `letrec` group: the
//! declarations are the bindings and references among them are dependency
//! edges. Prefix-visibility — a declaration sees its predecessors and not its
//! successors — is meaningless without an order that reflects those edges, and
//! there is none to inherit (§6.2a):
//!
//! - `LayerBuilder.resources` is a `BTreeMap<Iri, Resource>`, so a document's
//!   array order is discarded on load.
//! - Everything downstream iterates `defined_iris()`, which is
//!   **IRI-lexicographic**.
//!
//! Extending an environment while iterating `defined_iris()` would compile, run,
//! and produce a visibility rule determined by *IRI spelling*: `urn:x:Apple`
//! sees nothing, `urn:x:Zebra` sees everything, and renaming a declaration
//! changes what type-checks. Nothing would fail loudly. **So the sort has to
//! exist before anything depends on ordering**, which is why this is Phase A.
//!
//! **Scope: declarations, not instances.** The graph covers resources that
//! declare something the environment holds — classes, properties, inductives,
//! definitions, axioms — and not the millions of lexicon entries that merely
//! *use* them. That keeps the pass bounded by ontology size rather than chain
//! size, the same distinction that makes D78's class memo affordable.

use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use crate::ontology::well_known as wk;
use std::collections::{BTreeMap, BTreeSet};

/// Why a layer's declarations could not be ordered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderError {
    /// Two or more **inductive** declarations reference each other — a mutual
    /// inductive block (eigenius#20).
    ///
    /// Rejected rather than ordered, and rejected *loudly*, because today such a
    /// pair commits clean and lands in the chain uneliminable: no shared
    /// recursor exists, and `check_positivity` scans each declaration for
    /// occurrences of itself so a cross-type negative occurrence is not seen
    /// (`nbe::positivity::mutual_positivity_gap`).
    ///
    /// This is not a guard against expressible input. A mutual block *should* be
    /// expressible once #20 lands; what is wrong today is accepting it and
    /// producing something uneliminable. Fail-closed also makes #20's sequencing
    /// constraint enforceable: shipping `mutual … end` requires deliberately
    /// removing this rejection, which is where simultaneous positivity has to be
    /// present.
    MutualInductives(Vec<Iri>),
    /// A dependency cycle among declarations that are not all inductives.
    ///
    /// D76 §6.3: `letrec`'s signature-then-body separation works in ML because a
    /// type never depends on a term. In a dependent theory it can, so the
    /// general mutual case is not available and there is no reading under which
    /// this is admissible.
    Cycle(Vec<Iri>),
}

impl std::fmt::Display for OrderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names = |v: &Vec<Iri>| v.iter().map(|i| i.as_str()).collect::<Vec<_>>().join(", ");
        match self {
            Self::MutualInductives(iris) => write!(
                f,
                "mutual inductive types are not supported (eigenius#20): {} reference each other. \
                 Simultaneous positivity checking must land with the construct, not after it — \
                 see docs/design/d76-the-typing-environment.md §6.5.",
                names(iris)
            ),
            Self::Cycle(iris) => write!(
                f,
                "dependency cycle among declarations: {}. A layer is processed as a `letrec` \
                 group, and only mutually-recursive inductives may form a cycle (D76 §6.3).",
                names(iris)
            ),
        }
    }
}

impl std::error::Error for OrderError {}

/// Is this resource a **declaration** — something the typing environment holds?
fn is_declaration(r: &Resource) -> bool {
    r.is_a().iter().any(|c| {
        matches!(
            c.as_str(),
            wk::CLASS | wk::PROPERTY | wk::INDUCTIVE_TYPE | wk::DATA_TYPE
        )
    })
}

fn is_inductive(r: &Resource) -> bool {
    r.is_a().iter().any(|c| c.as_str() == wk::INDUCTIVE_TYPE)
}

/// Every IRI this resource references.
///
/// **Descends into `Value::Json`, which the walker in `layer::supporting` does
/// not.** That one documents JSON as never carrying typed-reference semantics,
/// which is true for its purpose and false here: an inductive's constructor
/// argument types are stored as D47-encoded JSON —
/// `{"ctor": "ConstRef", "args": ["urn:…", []]}` — so a walker that skips `Json`
/// finds **no inductive-to-inductive edges at all**. Reusing it would produce an
/// empty graph for precisely the case [`OrderError::MutualInductives`] exists to
/// catch, and would look like it worked.
/// Is `iri` a constructor class or argument property that D85 §6.1 derived from an inductive
/// in this layer, rather than something an author declared?
///
/// A constructor class names its inductive in `subclass_of`; an argument property names that
/// class in `domain`. Both exist because `core:ctors` has an entry, so neither is an
/// independent declaration — which matters when classifying a cycle, since a mutual inductive
/// pair drags its derived classes in with it.
fn is_derived_from_inductive(layer: &Layer, iri: &Iri, decls: &BTreeMap<Iri, bool>) -> bool {
    let Some(r) = layer.get_resource(iri) else {
        return false;
    };
    if let Some(v) = r.get(&crate::ontology::well_known::iri(wk::PARENT_CLASSES)) {
        if v.as_iri_array().iter().any(|p| decls.get(p) == Some(&true)) {
            return true;
        }
    }
    if let Some(v) = r.get(&crate::ontology::well_known::iri(wk::DOMAIN)) {
        return v.as_iri_array().iter().any(|owner| {
            layer
                .get_resource(owner)
                .and_then(|c| {
                    c.get(&crate::ontology::well_known::iri(wk::PARENT_CLASSES))
                        .cloned()
                })
                .map(|p| p.as_iri_array().iter().any(|i| decls.get(i) == Some(&true)))
                .unwrap_or(false)
        });
    }
    false
}

fn references(r: &Resource, out: &mut BTreeSet<Iri>) {
    for (prop, value) in r.properties() {
        out.insert(prop.clone());
        // `core:domain` is NOT a dependency edge. It says where a property APPLIES; it does not
        // say the property needs that class declared first. Treating it as one makes every
        // ordinary class/property pair circular — a class `requires` the property, the property's
        // `domain` names the class back — which core has had all along (`core:Property requires
        // core:data_type`, `core:data_type domain core:Property`) and which D85 §6.1's derived
        // constructor classes make universal, since every inductive now yields such a pair.
        //
        // This is a constant, not a schema lookup: the walk stays usable before any schema is
        // resolvable, which is the reason it is schema-blind in the first place.
        if prop.as_str() == wk::DOMAIN {
            continue;
        }
        value_refs(value, out);
    }
}

fn value_refs(v: &Value, out: &mut BTreeSet<Iri>) {
    match v {
        // A reference is a string that parses as an IRI.
        //
        // This arm read `Value::ResourceRef` and ignored `Value::String`, which worked only
        // while a build-time pass upgraded one to the other — an upgrade that never survived
        // a storage round trip, so a reloaded chain contributed no edges here at all. The
        // shape test is what `json_mentions_of_value` already uses one line below ("any `urn:`-prefixed
        // string at any depth"), and having one answer is what lets `core:mentions` and
        // `MutualInductives` agree about what a value names.
        Value::String(s) => {
            if let Ok(iri) = Iri::parse(s) {
                out.insert(iri);
            }
        }
        Value::Array(items) => items.iter().for_each(|i| value_refs(i, out)),
        Value::Embedded(inner) => references(inner.as_ref(), out),
        Value::Integer(_)
        | Value::Float(_)
        | Value::Boolean(_)
        | Value::Json(_)
        | Value::Vector { .. } => {}
    }
}

/// D76 §6.2 — the order a layer's declarations must be processed in.
///
/// The topological order induced by the dependency relation, ties broken by IRI.
/// Deterministic and **canonical**: the same declaration set always yields the
/// same order, so a layer's verdict does not depend on which valid order the
/// sort happened to produce.
///
/// Kahn's algorithm with min-IRI selection — the same shape as
/// [`crate::nbe::term::Exp::record`]'s field ordering (D78 §1), on a different
/// graph. A cycle is exactly the sort failing to place every node.
pub fn declaration_order(layer: &Layer) -> Result<Vec<Iri>, OrderError> {
    let mut decls: BTreeMap<Iri, bool> = BTreeMap::new(); // iri → is_inductive
    let mut deps: BTreeMap<Iri, BTreeSet<Iri>> = BTreeMap::new();

    for iri in layer.defined_iris() {
        let Some(r) = layer.get_resource(iri) else {
            continue;
        };
        if !is_declaration(&r) {
            continue;
        }
        decls.insert(iri.clone(), is_inductive(&r));
        let mut refs = BTreeSet::new();
        references(&r, &mut refs);
        deps.insert(iri.clone(), refs);
    }

    // Keep only edges to siblings — a reference to a lower layer is already
    // resolved and imposes no ordering here.
    for (iri, refs) in deps.iter_mut() {
        refs.retain(|t| decls.contains_key(t) && t != iri);
    }

    let mut placed: BTreeSet<Iri> = BTreeSet::new();
    let mut order: Vec<Iri> = Vec::with_capacity(decls.len());
    while order.len() < decls.len() {
        // The IRI-least declaration whose dependencies are all placed. `decls`
        // is a `BTreeMap`, so iteration is already IRI-ordered and `find` takes
        // the least.
        let next = decls
            .keys()
            .find(|i| !placed.contains(*i) && deps[*i].iter().all(|d| placed.contains(d)))
            .cloned();
        match next {
            Some(i) => {
                placed.insert(i.clone());
                order.push(i);
            }
            None => {
                let stuck: Vec<Iri> = decls
                    .keys()
                    .filter(|i| !placed.contains(*i))
                    .cloned()
                    .collect();
                // A derived constructor class or argument property (D85 §6.1) is not an
                // independent declaration for this purpose — it is part of the inductive it
                // was projected from, and it is stuck only because that inductive is. Judge
                // the cycle by the declarations an author actually wrote, or a mutual pair
                // would report a generic cycle instead of naming eigenius#20.
                let authored: Vec<Iri> = stuck
                    .iter()
                    .filter(|i| !is_derived_from_inductive(layer, i, &decls))
                    .cloned()
                    .collect();
                return Err(
                    if !authored.is_empty() && authored.iter().all(|i| decls[i]) {
                        OrderError::MutualInductives(authored)
                    } else {
                        OrderError::Cycle(stuck)
                    },
                );
            }
        }
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::{LayerBuilder, LayerStorage};
    use std::sync::Arc;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    /// A class requiring the listed properties — `requires` is a `resource_array`,
    /// so these are ordinary references.
    fn class(id: &str, requires: &[&str]) -> Resource {
        let mut r = Resource::new(iri(id));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::iri(&iri(wk::CLASS))]),
        );
        r.set(
            iri(wk::SHORT_NAME),
            Value::String(id.rsplit(':').next().unwrap().into()),
        );
        r.set(iri(wk::DESCRIPTION), Value::String("t".into()));
        r.set(
            iri(wk::REQUIRES),
            Value::Array(requires.iter().map(|p| Value::iri(&iri(p))).collect()),
        );
        r
    }

    /// An inductive whose constructor argument type is a **D47-encoded**
    /// reference to `arg` — the shape the `supporting` walker would miss.
    fn inductive(id: &str, arg: &str) -> Resource {
        let mut r = Resource::new(iri(id));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(
                iri(wk::INDUCTIVE_TYPE).as_str().to_string(),
            )]),
        );
        r.set(
            iri(wk::SHORT_NAME),
            Value::String(id.rsplit(':').next().unwrap().into()),
        );
        r.set(iri(wk::DESCRIPTION), Value::String("t".into()));
        r.set(
            iri(wk::CTORS),
            Value::Array(vec![Value::Embedded(Box::new({
                let mut c = Resource::new_embedded();
                c.set(
                    iri(wk::CTOR_NAME),
                    Value::String(format!("mk{}", id.rsplit(':').next().unwrap())),
                );
                c.set(
                    iri("urn:eigenius:core:arg_types"),
                    Value::Array(vec![Value::Embedded(Box::new({
                        let mut a = Resource::new_embedded();
                        a.set(iri("urn:eigenius:core:arg_name"), Value::String("x".into()));
                        a.set(
                            iri("urn:eigenius:core:type_name"),
                            crate::testing::term_value(&serde_json::json!({
                                "ctor": "ConstRef", "args": [arg, []]
                            })),
                        );
                        a
                    }))]),
                );
                c
            }))]),
        );
        r
    }

    fn layer_of(resources: Vec<Resource>) -> Arc<Layer> {
        let mut b = LayerBuilder::new("order-test", None);
        for r in resources {
            b.add_resource(r).unwrap();
        }
        Arc::new(b.build(LayerStorage::in_memory()))
    }

    fn names(order: &[Iri]) -> Vec<String> {
        order.iter().map(|i| i.as_str().to_string()).collect()
    }

    #[test]
    fn the_order_respects_every_dependency_edge() {
        // `A` requires a property declared by `P`, so `P` must precede `A`
        // despite sorting after it alphabetically.
        let mut p = Resource::new(iri("urn:t:zprop"));
        p.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::iri(&iri(wk::PROPERTY))]),
        );
        p.set(iri(wk::SHORT_NAME), Value::String("zprop".into()));
        p.set(iri(wk::DESCRIPTION), Value::String("t".into()));
        p.set(iri(wk::DATA_TYPE_PROP), Value::iri(&iri(wk::STRING)));

        let order = declaration_order(&layer_of(vec![class("urn:t:aclass", &["urn:t:zprop"]), p]))
            .expect("acyclic");
        assert_eq!(
            names(&order),
            ["urn:t:zprop", "urn:t:aclass"],
            "a dependency outranks IRI order"
        );
    }

    #[test]
    fn the_order_is_canonical_under_input_permutation() {
        // The property that makes a layer's verdict independent of which valid
        // order the sort produced.
        let a = class("urn:t:a", &[]);
        let b = class("urn:t:b", &[]);
        let c = class("urn:t:c", &[]);
        let one = declaration_order(&layer_of(vec![a.clone(), b.clone(), c.clone()])).unwrap();
        let two = declaration_order(&layer_of(vec![c, a, b])).unwrap();
        assert_eq!(
            one, two,
            "the same declaration set must yield the same order"
        );
        assert_eq!(names(&one), ["urn:t:a", "urn:t:b", "urn:t:c"]);
    }

    #[test]
    fn a_mutual_inductive_pair_is_rejected_naming_the_issue() {
        // The gate D76 §6.5 added. These reference each other through
        // **D47-encoded** ctor argument types, which is why this module does not
        // reuse `layer::supporting`'s walker — that one skips `Value::Json` and
        // would find no edge at all here.
        let err = declaration_order(&layer_of(vec![
            inductive("urn:t:A", "urn:t:B"),
            inductive("urn:t:B", "urn:t:A"),
        ]))
        .expect_err("a mutual inductive pair must be rejected");

        match &err {
            OrderError::MutualInductives(iris) => {
                assert_eq!(iris.len(), 2, "both members reported: {iris:?}");
            }
            other => panic!("expected MutualInductives, got {other:?}"),
        }
        assert!(
            err.to_string().contains("eigenius#20"),
            "the diagnostic must name the issue: {err}"
        );
    }

    #[test]
    fn a_non_inductive_cycle_is_reported_as_a_cycle() {
        // Two classes each requiring a property the other declares would be the
        // realistic shape; a direct class↔class cycle is the minimal one.
        let mut a = class("urn:t:ca", &[]);
        a.set(
            iri(wk::PARENT_CLASSES),
            Value::Array(vec![Value::iri(&iri("urn:t:cb"))]),
        );
        let mut b = class("urn:t:cb", &[]);
        b.set(
            iri(wk::PARENT_CLASSES),
            Value::Array(vec![Value::iri(&iri("urn:t:ca"))]),
        );

        let err = declaration_order(&layer_of(vec![a, b])).expect_err("a cycle must be rejected");
        assert!(
            matches!(err, OrderError::Cycle(_)),
            "a class cycle is not a mutual inductive: {err:?}"
        );
    }

    #[test]
    fn instances_are_not_declarations_and_do_not_enter_the_graph() {
        // Scope: the pass is bounded by ontology size, not chain size. A lexicon
        // entry references its class but is not itself a declaration.
        let mut inst = Resource::new(iri("urn:t:instance"));
        inst.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::iri(&iri("urn:t:a"))]),
        );
        let order = declaration_order(&layer_of(vec![class("urn:t:a", &[]), inst])).unwrap();
        assert_eq!(
            names(&order),
            ["urn:t:a"],
            "only the declaration is ordered; the instance is not"
        );
    }

    #[test]
    fn a_d47_encoded_reference_is_an_edge() {
        // The reason this module has its own walker. `Inductive` references
        // `urn:t:target` only through a `Value::Json` ConstRef.
        let order = declaration_order(&layer_of(vec![
            inductive("urn:t:zind", "urn:t:atarget"),
            class("urn:t:atarget", &[]),
        ]))
        .expect("acyclic");
        // The layer also carries the constructor class and argument property `build` derives
        // from `zind`'s `core:ctors` (D85 §6.1). They are ordered after the inductive they come
        // from, which is right; what this test is about is the authored pair.
        let authored: Vec<String> = names(&order)
            .into_iter()
            .filter(|n| !n.contains("-mk"))
            .collect();
        assert_eq!(
            authored,
            ["urn:t:atarget", "urn:t:zind"],
            "the D47-encoded reference must order the inductive after its target"
        );
        let all = names(&order);
        let pos = |s: &str| all.iter().position(|n| n == s).expect(s);
        assert!(
            pos("urn:t:zind") < pos("urn:t:zind-mkzind"),
            "a derived constructor class must be ordered after its inductive"
        );
    }
}
