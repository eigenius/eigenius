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

//! Retroactive validation pass.
//!
//! The per-new-layer validation (`Validator::validate`) only checks
//! resources defined directly in the committing layer. But a new
//! layer can change definitions that *lower-layer* resources rely on:
//! redefining a `core:Property`'s `data_type` or `class_types`,
//! tightening an `allows_only` enumeration, adding `requires` slots
//! to a `core:Class`, or simply introducing an IRI that lower-layer
//! property values already point at.
//!
//! This module enumerates the dependents and revalidates them against
//! the new chain (the new layer + ancestors). Errors land in
//! [`CommitWorkingSet::violations`]; the caller's [`CommitPolicy`]
//! decides whether to reject or cascade-tombstone.
//!
//! ## Dependent enumeration — three triggers
//!
//! For each IRI `i` defined in the new layer:
//!
//! 1. **`i` is_a `core:Class`** — every lower-layer resource whose
//!    `is_a` resolves to `i` *transitively via `subclass_of`* is a
//!    dependent (Rule 1's effective `requires` set, computed by the
//!    validator's recursive `collect_from_class`, may now include
//!    new entries). Enumerated by walking the full `subclass_of`
//!    closure rooted at `i` and scanning instances of every class in
//!    the closure.
//!
//! 2. **`i` is_a `core:Property`** — every lower-layer resource
//!    carrying property `i` is a dependent (Rules 3–10 may now reject
//!    the value under a tightened `data_type`, `pattern`, `min_value`,
//!    `class_types`, etc.). Enumerated via a chain walk in
//!    [`scan_chain_for_property_carriers`]; the triple index doesn't
//!    cover literal-typed values, so a walk is the v1 answer.
//!
//! 3. **`i` referenced as an IRI value by any property** — every
//!    lower-layer resource whose property value points at `i` is a
//!    dependent (Rule 8's `class_types` and Rule 9's `allows_only`
//!    may now reject the reference based on `i`'s new shape).
//!    Enumerated via triple-index scans over every IRI-typed
//!    Property declared in the chain (meta-ontology + user-defined),
//!    found dynamically by [`enumerate_iri_typed_predicates`].
//!
//! ## Phase 2 scope and limitations
//!
//! - Each violating resource is reported once even if multiple rules
//!   fire on it — the validator returns the full error vector and the
//!   collector preserves the ordering.

use crate::layer::Layer;
use crate::ontology::iri::Iri;
use crate::ontology::well_known as wk;
use crate::validation::{CommitWorkingSet, Validator, WorkingSetExhausted};
use std::sync::Arc;

/// Run the retroactive validation pass for `new_layer`. Populates
/// `ws.violations` with any errors found in lower-layer resources
/// that became invalid due to the new layer's content.
///
/// **Precondition.** `ws` is clean (use a freshly-allocated
/// [`CommitWorkingSet::in_memory`] or a pooled set acquired via
/// [`crate::validation::CommitWorkingSetPool::acquire`]).
///
/// **Returns `Err(WorkingSetExhausted)`** when any working-set
/// collection hits its capacity cap. The caller surfaces this as
/// [`crate::lattice::CommitError::WorkingSetExhausted`].
///
/// **Returns `Ok(())`** when the pass completes; the caller then
/// inspects `ws.violations.len()` to decide accept / reject /
/// cascade. The collector's truncation handles the policy's
/// `max_violations` cap.
pub fn retroactive_validate(
    new_layer: &Arc<Layer>,
    ws: &mut CommitWorkingSet,
) -> Result<(), WorkingSetExhausted> {
    enumerate_dependents(new_layer, ws)?;
    revalidate_pending(new_layer, ws)
}

/// Walk the new layer's `defined_iris`, classify each by role, and
/// enumerate dependents into `ws.pending`.
fn enumerate_dependents(
    new_layer: &Arc<Layer>,
    ws: &mut CommitWorkingSet,
) -> Result<(), WorkingSetExhausted> {
    // Cache the well-known IRIs once per pass.
    let Ok(core_class) = Iri::parse(wk::CLASS) else {
        return Ok(());
    };
    let Ok(core_property) = Iri::parse(wk::PROPERTY) else {
        return Ok(());
    };
    let Ok(is_a) = Iri::parse(wk::IS_A) else {
        return Ok(());
    };
    let Ok(subclass_of) = Iri::parse(wk::PARENT_CLASSES) else {
        return Ok(());
    };

    // Snapshot the new layer's defined IRIs — we'll iterate without
    // borrowing the layer for the duration of the walk.
    let new_iris: Vec<Iri> = new_layer.defined_iris().iter().cloned().collect();

    // Every IRI-typed Property declaration in the chain is a
    // candidate predicate for case (3). We enumerate via the triple
    // index on `is_a` against `core:Property`, then filter by
    // `data_type` resolving to `resource` / `resource_array`. Covers
    // both meta-ontology predicates (`is_a`, `subclass_of`, …) and
    // every user-defined IRI-typed property without a hard-coded
    // list.
    let iri_ref_predicates = enumerate_iri_typed_predicates(new_layer, &is_a);

    for iri in &new_iris {
        // Each new-layer resource's role determines enumeration:
        let Some(resource) = new_layer.get_resource(iri) else {
            continue;
        };
        let is_class = resource.is_instance_of(&core_class);
        let is_property = resource.is_instance_of(&core_property);

        // Reference integrity (Rule 22) makes a layer's validity a function of (its
        // content, the chain below it): every `is_a` / value reference — AND every
        // property *key* (Rule 22 §(c)) — must resolve same-or-lower at commit. So a
        // **brand-new** IRI — one not already defined in a lower layer — provably has NO
        // lower dependents for ANY of the three cases: no lower resource could be an
        // instance of, reference, or *carry as a property key* an IRI that did not exist
        // when it committed. We therefore run all three cases only for **redefinitions**
        // (an IRI that shadows an ancestor definition — a small set). This is what keeps
        // the otherwise O(chain) carrier scan (case 2) off the hot path: an additive
        // import's brand-new properties are skipped entirely; only a rare redefinition
        // pays. Collapses the retroactive pass from O(new_iris × predicates) to
        // ~O(redefs).
        let redefines = redefines_ancestor(new_layer, iri);

        if is_class && redefines {
            // Every direct or transitive subclass inherits this
            // Class's `requires` via the validator's recursive
            // `collect_from_class`. We enumerate the full closure,
            // then for each class in it scan instances. Pushing
            // instances (not the subclasses themselves) — subclass
            // resources are Class declarations; their own validity
            // is already covered by case (3) via `subclass_of`
            // references.
            let closure = collect_subclass_closure(new_layer, iri, &subclass_of);
            for class_iri in &closure {
                for subj in crate::layer::scan_chain(new_layer, &is_a, class_iri) {
                    ws.pending.push(subj)?;
                }
            }
        }
        if is_property && redefines {
            // Rules 3–10 dependents: every lower resource carrying this property must be
            // revalidated against the (re)definition. Only a REDEFINITION can have lower
            // carriers: reference integrity (Rule 22 §(c)) requires a property key to
            // resolve to a declared `core:Property` same-or-lower at commit, so a
            // brand-new property was unwritable in any lower layer and provably has no
            // lower carriers — the same closure that scopes cases (1)/(3) to
            // redefinitions. (This is what makes the otherwise O(chain) carrier scan
            // safe on a large chain: it never runs for an additive import's new
            // properties, only for the rare redefinition.)
            scan_chain_for_property_carriers(new_layer, iri, ws)?;
        }

        // Case (3): IRI is referenced as a value by lower-layer property values.
        // Only a redefinition can have lower referrers (closure: a brand-new IRI was
        // unreferenceable below). Enumerate via every IRI-typed predicate in the chain.
        if redefines {
            for pred in &iri_ref_predicates {
                for subj in crate::layer::scan_chain(new_layer, pred, iri) {
                    ws.pending.push(subj)?;
                }
            }
        }
    }

    Ok(())
}

/// Whether `iri` already resolves in the chain **below** `new_layer` — i.e. the new
/// layer *redefines* (shadows) an ancestor definition rather than introducing a
/// brand-new IRI. Bloom-gated `resolve`, so this is ~O(chain depth), not O(resources).
/// Under reference integrity (Rule 22), only a redefinition can have lower instances or
/// referrers, so this gates the expensive case-(1)/(3) scans.
fn redefines_ancestor(new_layer: &Arc<Layer>, iri: &Iri) -> bool {
    new_layer.parents().iter().any(|p| p.resolve(iri).is_some())
}

/// Drain `ws.pending`; for each IRI not already revalidated, look up
/// the resource via the new chain's resolve walk and validate it.
/// Skips IRIs that are themselves defined in the new layer — those
/// were validated by the per-new-layer pass already.
fn revalidate_pending(
    new_layer: &Arc<Layer>,
    ws: &mut CommitWorkingSet,
) -> Result<(), WorkingSetExhausted> {
    let validator = Validator::new(Arc::clone(new_layer));
    while let Some(iri) = ws.pending.pop() {
        if !ws.revalidated.insert(iri.clone())? {
            continue;
        }
        // Skip the new layer's own resources — already validated.
        if new_layer.defined_iris().contains(&iri) {
            continue;
        }
        if let Some(resource) = new_layer.resolve(&iri) {
            let errors = validator.validate_resource(resource.as_ref());
            for err in errors {
                ws.violations.push(err);
            }
        }
        // If resolve returns None, the IRI is tombstoned or otherwise
        // not reachable. Nothing to validate. (A tombstoned IRI's
        // dangling references show up via Rule 14 on the referring
        // resource, which is itself a separate dependent we'll catch
        // through case (3).)
    }
    Ok(())
}

/// Walk the new layer's merged view; for each resource carrying
/// `property`, push it into `ws.pending`. Used for Phase 2's
/// literal-valued property dependent enumeration (the triple index
/// only covers IRI-valued properties).
fn scan_chain_for_property_carriers(
    new_layer: &Arc<Layer>,
    property: &Iri,
    ws: &mut CommitWorkingSet,
) -> Result<(), WorkingSetExhausted> {
    for (iri, resource) in new_layer.iter_all_resources() {
        if resource.has(property) {
            ws.pending.push(iri)?;
        }
    }
    Ok(())
}

/// Collect the transitive `subclass_of` closure rooted at `root`,
/// inclusive of `root` itself. Each returned IRI is a class `C` such
/// that `C subclass_of* root` — instances of `C` inherit `root`'s
/// `requires` via the validator's recursive `collect_from_class` and
/// must therefore be revalidated when `root`'s definition changes.
///
/// BFS over the `subclass_of` triple index ordering. Dedup via the
/// `seen` set guards against cycles (the chain shouldn't have any,
/// but defending against malformed cycles costs nothing).
fn collect_subclass_closure(new_layer: &Arc<Layer>, root: &Iri, subclass_of: &Iri) -> Vec<Iri> {
    let mut closure = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(root.clone());
    while let Some(class_iri) = queue.pop_front() {
        if !seen.insert(class_iri.clone()) {
            continue;
        }
        closure.push(class_iri.clone());
        // Direct subclasses of this class — they recurse via the queue.
        for sub in crate::layer::scan_chain(new_layer, subclass_of, &class_iri) {
            if !seen.contains(&sub) {
                queue.push_back(sub);
            }
        }
    }
    closure
}

/// Enumerate every IRI-typed predicate declared anywhere in the
/// chain rooted at `new_layer`. Used by [`enumerate_dependents`] to
/// find lower-layer resources that reference a new-layer IRI via any
/// IRI-typed property.
///
/// **Algorithm.** `scan_chain(is_a, core:Property)` returns every
/// resource whose `is_a` includes `core:Property` — the set of
/// Property declarations in the chain (including the meta-ontology's
/// own `is_a`, `subclass_of`, `class_types`, etc., plus any
/// user-defined properties). For each, resolve and check the
/// `data_type` slot — keep those that resolve to `core:resource` or
/// `core:resource_array`.
///
/// **Cost.** One triple-index prefix scan + one resolve per Property
/// declaration. For typical ontologies with O(100) properties this
/// completes in microseconds.
fn enumerate_iri_typed_predicates(new_layer: &Arc<Layer>, is_a: &Iri) -> Vec<Iri> {
    let Ok(core_property) = Iri::parse(wk::PROPERTY) else {
        return Vec::new();
    };
    let Ok(data_type_prop) = Iri::parse(wk::DATA_TYPE_PROP) else {
        return Vec::new();
    };

    let mut iri_typed = Vec::new();
    for prop_iri in crate::layer::scan_chain(new_layer, is_a, &core_property) {
        let Some(prop_def) = new_layer.resolve(&prop_iri) else {
            continue;
        };
        let Some(dt) = prop_def.get(&data_type_prop).and_then(|v| v.as_iri_str()) else {
            continue;
        };
        if dt == wk::RESOURCE || dt == wk::RESOURCE_ARRAY {
            iri_typed.push(prop_iri);
        }
    }
    iri_typed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::{LayerBuilder, LayerStorage};
    use crate::ontology::resource::{Resource, Value};
    use crate::storage::memory::MemoryPersistentBackend;
    use crate::storage::PersistentBackend;
    use crate::validation::CommitWorkingSet;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    /// Build a minimal bootstrap chain: core ontology + a domain
    /// layer that defines a Class and some instances. Returns the
    /// head layer.
    fn build_chain_with_class_instances() -> (Arc<Layer>, LayerStorage, Arc<MemoryPersistentBackend>)
    {
        let backend = Arc::new(MemoryPersistentBackend::new());
        let storage = LayerStorage::with_persistent(backend.clone() as _);

        // Root: core ontology shape — just enough Class/Property/is_a
        // declarations to exercise the validator.
        let mut root_b = LayerBuilder::new("core", None);

        // core:Class
        let mut core_class = Resource::new(iri(wk::CLASS));
        core_class.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::CLASS))]),
        );
        core_class.set(
            iri(wk::DESCRIPTION),
            Value::String("self-describing".into()),
        );
        core_class.set(iri(wk::SHORT_NAME), Value::String("Class".into()));
        core_class.set(
            iri(wk::REQUIRES),
            Value::Array(vec![
                Value::ResourceRef(iri(wk::IS_A)),
                Value::ResourceRef(iri(wk::DESCRIPTION)),
                Value::ResourceRef(iri(wk::SHORT_NAME)),
            ]),
        );
        root_b.add_resource(core_class).unwrap();

        // core:Property
        let mut core_property = Resource::new(iri(wk::PROPERTY));
        core_property.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::CLASS))]),
        );
        core_property.set(iri(wk::DESCRIPTION), Value::String("property class".into()));
        core_property.set(iri(wk::SHORT_NAME), Value::String("Property".into()));
        core_property.set(
            iri(wk::REQUIRES),
            Value::Array(vec![
                Value::ResourceRef(iri(wk::IS_A)),
                Value::ResourceRef(iri(wk::DESCRIPTION)),
                Value::ResourceRef(iri(wk::SHORT_NAME)),
                Value::ResourceRef(iri(wk::DATA_TYPE_PROP)),
            ]),
        );
        root_b.add_resource(core_property).unwrap();

        // core:is_a property
        let mut is_a = Resource::new(iri(wk::IS_A));
        is_a.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
        );
        is_a.set(iri(wk::DESCRIPTION), Value::String("is_a".into()));
        is_a.set(iri(wk::SHORT_NAME), Value::String("is_a".into()));
        is_a.set(
            iri(wk::DATA_TYPE_PROP),
            Value::ResourceRef(iri(wk::RESOURCE_ARRAY)),
        );
        is_a.set(
            iri(wk::CLASS_TYPES),
            Value::Array(vec![Value::ResourceRef(iri(wk::CLASS))]),
        );
        root_b.add_resource(is_a).unwrap();

        // core:description
        let mut desc = Resource::new(iri(wk::DESCRIPTION));
        desc.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
        );
        desc.set(iri(wk::DESCRIPTION), Value::String("description".into()));
        desc.set(iri(wk::SHORT_NAME), Value::String("description".into()));
        desc.set(iri(wk::DATA_TYPE_PROP), Value::ResourceRef(iri(wk::STRING)));
        root_b.add_resource(desc).unwrap();

        // core:short_name
        let mut sn = Resource::new(iri(wk::SHORT_NAME));
        sn.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
        );
        sn.set(iri(wk::DESCRIPTION), Value::String("short_name".into()));
        sn.set(iri(wk::SHORT_NAME), Value::String("short_name".into()));
        sn.set(iri(wk::DATA_TYPE_PROP), Value::ResourceRef(iri(wk::STRING)));
        root_b.add_resource(sn).unwrap();

        // core:string data type
        let mut string_dt = Resource::new(iri(wk::STRING));
        string_dt.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::CLASS))]),
        );
        string_dt.set(iri(wk::DESCRIPTION), Value::String("string".into()));
        string_dt.set(iri(wk::SHORT_NAME), Value::String("string".into()));
        root_b.add_resource(string_dt).unwrap();

        // core:resource_array data type
        let mut ra_dt = Resource::new(iri(wk::RESOURCE_ARRAY));
        ra_dt.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::CLASS))]),
        );
        ra_dt.set(iri(wk::DESCRIPTION), Value::String("resource_array".into()));
        ra_dt.set(iri(wk::SHORT_NAME), Value::String("resource_array".into()));
        root_b.add_resource(ra_dt).unwrap();

        // core:resource data type — needed by demo:color in the
        // allows_only test below.
        let mut res_dt = Resource::new(iri(wk::RESOURCE));
        res_dt.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::CLASS))]),
        );
        res_dt.set(iri(wk::DESCRIPTION), Value::String("resource".into()));
        res_dt.set(iri(wk::SHORT_NAME), Value::String("resource".into()));
        root_b.add_resource(res_dt).unwrap();

        // core:data_type property
        let mut dt_prop = Resource::new(iri(wk::DATA_TYPE_PROP));
        dt_prop.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
        );
        dt_prop.set(iri(wk::DESCRIPTION), Value::String("data_type".into()));
        dt_prop.set(iri(wk::SHORT_NAME), Value::String("data_type".into()));
        dt_prop.set(
            iri(wk::DATA_TYPE_PROP),
            Value::ResourceRef(iri(wk::RESOURCE_ARRAY)),
        );
        root_b.add_resource(dt_prop).unwrap();

        // core:subclass_of property — IRI-typed; the triple index
        // needs this to be a Property with `data_type: resource_array`
        // so the retroactive pass can scan_chain subclasses.
        let mut sub_prop = Resource::new(iri(wk::PARENT_CLASSES));
        sub_prop.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
        );
        sub_prop.set(iri(wk::DESCRIPTION), Value::String("subclass_of".into()));
        sub_prop.set(iri(wk::SHORT_NAME), Value::String("subclass_of".into()));
        sub_prop.set(
            iri(wk::DATA_TYPE_PROP),
            Value::ResourceRef(iri(wk::RESOURCE_ARRAY)),
        );
        root_b.add_resource(sub_prop).unwrap();

        // core:requires property — used by mk_class via `requires` slot.
        let mut req_prop = Resource::new(iri(wk::REQUIRES));
        req_prop.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
        );
        req_prop.set(iri(wk::DESCRIPTION), Value::String("requires".into()));
        req_prop.set(iri(wk::SHORT_NAME), Value::String("requires".into()));
        req_prop.set(
            iri(wk::DATA_TYPE_PROP),
            Value::ResourceRef(iri(wk::RESOURCE_ARRAY)),
        );
        root_b.add_resource(req_prop).unwrap();

        let root = Arc::new(root_b.build(storage.clone()));
        // Commit (persist + index) the lower layer the way production does, so
        // retroactive validation's `scan_chain` sees its triple entries.
        backend.store_layer(&root).unwrap();

        (root, storage, backend)
    }

    /// Smoke: a commit that doesn't trigger any retroactive
    /// violations leaves `ws.violations` empty.
    #[test]
    fn no_dependents_means_no_violations() {
        let (root, storage, _backend) = build_chain_with_class_instances();
        // Child layer defines a new IRI nobody references. No
        // retroactive validation should produce violations.
        let mut b = LayerBuilder::new("child", Some(Arc::clone(&root)));
        let mut r = Resource::new(iri("urn:eigenius:demo:Standalone"));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::CLASS))]),
        );
        r.set(iri(wk::DESCRIPTION), Value::String("alone".into()));
        r.set(iri(wk::SHORT_NAME), Value::String("Standalone".into()));
        b.add_resource(r).unwrap();
        let child = Arc::new(b.build(storage));

        let mut ws = CommitWorkingSet::in_memory();
        retroactive_validate(&child, &mut ws).unwrap();
        assert_eq!(ws.violations.len(), 0);
    }

    /// Build a Class with a `requires` set. Used for the
    /// "class redef adds requires" regression test.
    fn mk_class(id: &str, short: &str, requires: Vec<&str>) -> Resource {
        let mut r = Resource::new(iri(id));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::CLASS))]),
        );
        r.set(iri(wk::DESCRIPTION), Value::String("class".into()));
        r.set(iri(wk::SHORT_NAME), Value::String(short.into()));
        let req_vals: Vec<Value> = requires
            .into_iter()
            .map(|s| Value::ResourceRef(iri(s)))
            .collect();
        if !req_vals.is_empty() {
            r.set(iri(wk::REQUIRES), Value::Array(req_vals));
        }
        r
    }

    /// Build an instance: `is_a: [class_iri]`, plus any extra
    /// (prop, value) pairs the caller specifies.
    fn mk_instance(id: &str, class_iri: &str, props: Vec<(&str, Value)>) -> Resource {
        let mut r = Resource::new(iri(id));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(class_iri))]),
        );
        for (p, v) in props {
            r.set(iri(p), v);
        }
        r
    }

    /// Regression: a new layer redefines an existing Class to add a
    /// new `requires` slot. Lower-layer instances of that class are
    /// missing the new required property → they violate Rule 1.
    /// The retroactive pass must surface those violations.
    #[test]
    fn class_redef_with_new_requires_invalidates_instances() {
        let (root, storage, backend) = build_chain_with_class_instances();

        // Mid layer: defines `demo:Foo` (a Class) with no requires
        // beyond the meta-ontology baseline, plus an instance `foo_1`
        // that only carries `is_a` (no extra properties).
        let mid = {
            let mut b = LayerBuilder::new("mid", Some(Arc::clone(&root)));
            b.add_resource(mk_class("urn:eigenius:demo:Foo", "Foo", vec![]))
                .unwrap();
            b.add_resource(mk_instance(
                "urn:eigenius:demo:foo_1",
                "urn:eigenius:demo:Foo",
                vec![],
            ))
            .unwrap();
            Arc::new(b.build(storage.clone()))
        };
        backend.store_layer(&mid).unwrap();
        // Confirm baseline: mid is valid.
        let mut ws = CommitWorkingSet::in_memory();
        retroactive_validate(&mid, &mut ws).unwrap();
        assert_eq!(ws.violations.len(), 0);

        // New layer redefines `demo:Foo` to require `core:description`.
        // The instance `foo_1` (in mid) doesn't have description, so
        // the retroactive pass must flag it.
        let new = {
            let mut b = LayerBuilder::new("new", Some(Arc::clone(&mid)));
            b.add_resource(mk_class(
                "urn:eigenius:demo:Foo",
                "Foo",
                vec![wk::DESCRIPTION],
            ))
            .unwrap();
            Arc::new(b.build(storage.clone()))
        };

        let mut ws = CommitWorkingSet::in_memory();
        retroactive_validate(&new, &mut ws).unwrap();
        assert!(
            !ws.violations.is_empty(),
            "class redef adding required property must invalidate the existing instance"
        );
    }

    /// Regression: case (3) enumerates user-defined IRI-typed
    /// predicates dynamically, not from a hard-coded meta-ontology
    /// list. Scenario: a lower-layer resource references
    /// `demo:Target` via a *user-defined* property `demo:references`
    /// whose `class_types` is `[demo:GoodKind]`. A new layer adds
    /// `demo:Target` with `is_a: [demo:BadKind]`. Rule 8 should flag
    /// the lower-layer reference.
    #[test]
    fn user_defined_iri_typed_predicate_triggers_case_3() {
        let (root, storage, backend) = build_chain_with_class_instances();

        // Mid layer: declares `demo:GoodKind` and `demo:BadKind` classes, a
        // user-defined property `demo:references` (class_types: [demo:GoodKind]),
        // `demo:Target` AS A GoodKind, and `demo:caller` referencing Target. Under
        // reference integrity (Rule 22) the caller's reference must resolve same-or-
        // lower, so Target exists here and the reference is valid — no dangling. The
        // case-(3) trigger is a *redefinition* of Target below, not a late definition.
        let mid = {
            let mut b = LayerBuilder::new("mid", Some(Arc::clone(&root)));
            b.add_resource(mk_class("urn:eigenius:demo:GoodKind", "GoodKind", vec![]))
                .unwrap();
            b.add_resource(mk_class("urn:eigenius:demo:BadKind", "BadKind", vec![]))
                .unwrap();

            let mut prop = Resource::new(iri("urn:eigenius:demo:references"));
            prop.set(
                iri(wk::IS_A),
                Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
            );
            prop.set(
                iri(wk::DESCRIPTION),
                Value::String("user-defined IRI ref".into()),
            );
            prop.set(iri(wk::SHORT_NAME), Value::String("references".into()));
            prop.set(
                iri(wk::DATA_TYPE_PROP),
                Value::ResourceRef(iri(wk::RESOURCE)),
            );
            prop.set(
                iri(wk::CLASS_TYPES),
                Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:demo:GoodKind"))]),
            );
            b.add_resource(prop).unwrap();

            // Target as GoodKind — the reference below will be valid.
            let mut target = mk_instance(
                "urn:eigenius:demo:Target",
                "urn:eigenius:demo:GoodKind",
                vec![],
            );
            target.set(iri(wk::DESCRIPTION), Value::String("good target".into()));
            target.set(iri(wk::SHORT_NAME), Value::String("Target".into()));
            b.add_resource(target).unwrap();

            // demo:caller references demo:Target (a GoodKind) — valid, closure-clean.
            let mut caller = mk_instance(
                "urn:eigenius:demo:caller",
                wk::CLASS,
                vec![(
                    "urn:eigenius:demo:references",
                    Value::ResourceRef(iri("urn:eigenius:demo:Target")),
                )],
            );
            caller.set(iri(wk::DESCRIPTION), Value::String("caller".into()));
            caller.set(iri(wk::SHORT_NAME), Value::String("caller".into()));
            b.add_resource(caller).unwrap();
            Arc::new(b.build(storage.clone()))
        };
        backend.store_layer(&mid).unwrap();
        let mut ws = CommitWorkingSet::in_memory();
        retroactive_validate(&mid, &mut ws).unwrap();
        assert_eq!(
            ws.violations.len(),
            0,
            "mid is valid in isolation (Target is GoodKind, caller's reference is allowed)"
        );

        // New layer REDEFINES demo:Target as is_a: [demo:BadKind]. demo:caller's
        // `demo:references = demo:Target` now resolves (shadow-resolves) to a BadKind,
        // so Rule 8 should reject — and case (3) must fire because Target *redefines*
        // an ancestor (the brand-new-IRI fast path does not skip redefinitions).
        let new = {
            let mut b = LayerBuilder::new("new", Some(Arc::clone(&mid)));
            let mut target = mk_instance(
                "urn:eigenius:demo:Target",
                "urn:eigenius:demo:BadKind",
                vec![],
            );
            target.set(iri(wk::DESCRIPTION), Value::String("bad target".into()));
            target.set(iri(wk::SHORT_NAME), Value::String("Target".into()));
            b.add_resource(target).unwrap();
            Arc::new(b.build(storage.clone()))
        };

        let mut ws = CommitWorkingSet::in_memory();
        retroactive_validate(&new, &mut ws).unwrap();
        assert!(
            !ws.violations.is_empty(),
            "redefining Target as the wrong class must surface Rule 8 violation on caller"
        );
        let drained = ws.violations.drain(100);
        assert!(
            drained
                .errors
                .iter()
                .any(|e| matches!(e.rule, crate::validation::ValidationRule::ClassTypeMismatch)),
            "expected ClassTypeMismatch via user-defined demo:references predicate; got {:?}",
            drained.errors
        );
    }

    /// The precondition that lets case (2) skip brand-new properties: a resource may
    /// NOT carry a property key that isn't defined same-or-lower (reference integrity,
    /// Rule 22 §(c)). Open-world (Rule 12) only frees a resource from its classes'
    /// requires/recommends sets — it does NOT permit an undeclared property IRI as a
    /// key. So the old "carry an undeclared property, declare it later" scenario is
    /// rejected at commit, which means a brand-new property provably has no lower
    /// carrier and retroactive validation correctly does not scan for one.
    #[test]
    fn undeclared_property_key_is_rejected() {
        let (root, storage, _backend) = build_chain_with_class_instances();

        let mid = {
            let mut b = LayerBuilder::new("mid", Some(Arc::clone(&root)));
            let mut carrier = mk_instance(
                "urn:eigenius:demo:carrier",
                wk::CLASS,
                vec![("urn:eigenius:demo:rank", Value::String("not-an-int".into()))],
            );
            carrier.set(iri(wk::DESCRIPTION), Value::String("carrier".into()));
            carrier.set(iri(wk::SHORT_NAME), Value::String("carrier".into()));
            b.add_resource(carrier).unwrap();
            Arc::new(b.build(storage.clone()))
        };

        // Structural validation rejects the undeclared `demo:rank` key (Rule 22 §(c)).
        let errors = crate::validation::Validator::new(Arc::clone(&mid)).validate();
        assert!(
            errors.iter().any(|e| matches!(
                e.rule,
                crate::validation::ValidationRule::UnresolvedClassReference
            ) && e.property.as_ref().map(|p| p.as_str())
                == Some("urn:eigenius:demo:rank")),
            "carrying an undeclared property key (demo:rank) must be rejected; got {errors:?}"
        );
    }

    /// Regression: transitive `subclass_of` closure. When a new
    /// layer redefines a Class `A` with new `requires`, every
    /// instance of every class that `subclass_of*` `A` must be
    /// revalidated — not just direct instances and not just direct
    /// subclasses' instances. Catches the case where the
    /// enumeration walks the closure properly.
    ///
    /// Setup:
    /// - mid: `demo:A` (no requires), `demo:B subclass_of [demo:A]`,
    ///   `demo:C subclass_of [demo:B]`, plus instances:
    ///   `inst_A : A`, `inst_B : B`, `inst_C : C`.
    /// - new: redefine `demo:A` to require `core:description`.
    /// - Expected: violations for at least `inst_A`, `inst_B`,
    ///   `inst_C` — all three lack `description`.
    #[test]
    fn transitive_subclass_closure_enumerates_indirect_instances() {
        let (root, storage, backend) = build_chain_with_class_instances();

        let mid = {
            let mut b = LayerBuilder::new("mid", Some(Arc::clone(&root)));
            // A: no requires.
            b.add_resource(mk_class("urn:eigenius:demo:A", "A", vec![]))
                .unwrap();
            // B subclass_of [A]
            let mut bcls = mk_class("urn:eigenius:demo:B", "B", vec![]);
            bcls.set(
                iri(wk::PARENT_CLASSES),
                Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:demo:A"))]),
            );
            b.add_resource(bcls).unwrap();
            // C subclass_of [B]
            let mut ccls = mk_class("urn:eigenius:demo:C", "C", vec![]);
            ccls.set(
                iri(wk::PARENT_CLASSES),
                Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:demo:B"))]),
            );
            b.add_resource(ccls).unwrap();
            // Instances at each level — none carry `description`.
            // Validator's Rule 1 won't fire on any of them yet
            // because A doesn't require description here.
            b.add_resource(mk_instance(
                "urn:eigenius:demo:inst_A",
                "urn:eigenius:demo:A",
                vec![],
            ))
            .unwrap();
            b.add_resource(mk_instance(
                "urn:eigenius:demo:inst_B",
                "urn:eigenius:demo:B",
                vec![],
            ))
            .unwrap();
            b.add_resource(mk_instance(
                "urn:eigenius:demo:inst_C",
                "urn:eigenius:demo:C",
                vec![],
            ))
            .unwrap();
            Arc::new(b.build(storage.clone()))
        };
        backend.store_layer(&mid).unwrap();
        let mut ws = CommitWorkingSet::in_memory();
        retroactive_validate(&mid, &mut ws).unwrap();
        assert_eq!(
            ws.violations.len(),
            0,
            "mid valid in isolation (A has no requires yet)"
        );

        // Redefine A to require core:description.
        let new = {
            let mut b = LayerBuilder::new("new", Some(Arc::clone(&mid)));
            b.add_resource(mk_class("urn:eigenius:demo:A", "A", vec![wk::DESCRIPTION]))
                .unwrap();
            Arc::new(b.build(storage.clone()))
        };

        let mut ws = CommitWorkingSet::in_memory();
        retroactive_validate(&new, &mut ws).unwrap();
        let total = ws.violations.len();
        let drained = ws.violations.drain(usize::MAX);

        // All three instances (direct, one-hop subclass's, two-hop
        // subclass's) must be flagged. We assert by checking each
        // expected resource_id appears in the error list.
        let violating_ids: std::collections::BTreeSet<String> = drained
            .errors
            .iter()
            .filter_map(|e| e.resource_id.as_ref().map(|i| i.as_str().to_string()))
            .collect();
        assert!(
            violating_ids.contains("urn:eigenius:demo:inst_A"),
            "direct instance inst_A must be flagged; violations: {:?}",
            violating_ids
        );
        assert!(
            violating_ids.contains("urn:eigenius:demo:inst_B"),
            "one-hop subclass instance inst_B must be flagged; violations: {:?}",
            violating_ids
        );
        assert!(
            violating_ids.contains("urn:eigenius:demo:inst_C"),
            "two-hop subclass instance inst_C must be flagged (transitive closure); \
             violations: {:?}",
            violating_ids
        );
        assert!(
            total >= 3,
            "expected at least 3 violations (one per instance); got {total}"
        );
    }

    /// Cascade smoke: a Property redef invalidates one lower-layer
    /// resource. With `CascadeTombstone` policy, `commit_layer`
    /// tombstones the violator and reaches fixpoint, committing
    /// the layer with `cascade_tombstones = {violator_iri}` and
    /// `cascade_iterations = 2` (one to discover + tombstone, one
    /// to confirm fixpoint).
    #[test]
    fn cascade_tombstones_violator_and_reaches_fixpoint() {
        use crate::lattice::{commit_layer, CommitPolicy};
        use crate::storage::PersistentBackend;
        let (root, storage, backend) = build_chain_with_class_instances();

        // Mid: defines `demo:Foo` (Class, no requires) and an
        // instance `inst_foo : Foo` lacking `description`.
        let mid = {
            let mut b = LayerBuilder::new("mid", Some(Arc::clone(&root)));
            b.add_resource(mk_class("urn:eigenius:demo:Foo", "Foo", vec![]))
                .unwrap();
            // Instance without description — currently valid (Foo has
            // no requires).
            let mut inst = Resource::new(iri("urn:eigenius:demo:inst_foo"));
            inst.set(
                iri(wk::IS_A),
                Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:demo:Foo"))]),
            );
            inst.set(iri(wk::SHORT_NAME), Value::String("inst_foo".into()));
            b.add_resource(inst).unwrap();
            Arc::new(b.build(storage.clone()))
        };
        backend.store_layer(&mid).unwrap();

        // New layer redefines Foo to require `description`. inst_foo
        // doesn't have it → cascade tombstones inst_foo.
        let mut new_b = LayerBuilder::new("new", Some(Arc::clone(&mid)));
        new_b
            .add_resource(mk_class(
                "urn:eigenius:demo:Foo",
                "Foo",
                vec![wk::DESCRIPTION],
            ))
            .unwrap();

        let mut ws = CommitWorkingSet::in_memory();
        let outcome = commit_layer(
            new_b,
            storage.clone(),
            backend.as_ref(),
            CommitPolicy::CascadeTombstone,
            &mut ws,
        )
        .expect("cascade should succeed by tombstoning inst_foo");

        assert!(
            outcome
                .cascade_tombstones
                .contains(&iri("urn:eigenius:demo:inst_foo")),
            "expected cascade to tombstone inst_foo; got {:?}",
            outcome.cascade_tombstones
        );
        assert_eq!(outcome.cascade_iterations, 2);
        // The committed layer carries the cascade tombstone.
        assert!(outcome
            .layer
            .tombstoned_iris()
            .contains(&iri("urn:eigenius:demo:inst_foo")));
        // And resolve from the committed layer reflects the suppression.
        assert!(outcome
            .layer
            .resolve(&iri("urn:eigenius:demo:inst_foo"))
            .is_none());
    }

    /// Cascade abort: a new-layer Class references a lower-layer
    /// class via `subclass_of`; the cascade would tombstone that
    /// lower-layer class (because a Class-membership redef
    /// invalidates it), leaving the new-layer Class with a
    /// dangling reference (Rule 14: `UnresolvedClassReference`).
    /// The cascade must abort and surface the new-layer breakage.
    #[test]
    fn cascade_aborts_when_tombstone_would_break_new_layer() {
        use crate::lattice::{commit_layer, CommitError, CommitPolicy};
        use crate::storage::PersistentBackend;
        let (root, storage, backend) = build_chain_with_class_instances();

        // Mid: defines a custom `demo:Tagged` class with one requires
        // slot (`demo:tag`), plus `demo:tag` itself (a Property), plus
        // `demo:LowerC` is_a: [demo:Tagged] with the tag — valid.
        let mid = {
            let mut b = LayerBuilder::new("mid", Some(Arc::clone(&root)));

            // demo:tag property (string-valued)
            let mut tag_prop = Resource::new(iri("urn:eigenius:demo:tag"));
            tag_prop.set(
                iri(wk::IS_A),
                Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
            );
            tag_prop.set(iri(wk::DESCRIPTION), Value::String("tag".into()));
            tag_prop.set(iri(wk::SHORT_NAME), Value::String("tag".into()));
            tag_prop.set(iri(wk::DATA_TYPE_PROP), Value::ResourceRef(iri(wk::STRING)));
            b.add_resource(tag_prop).unwrap();

            // demo:Tagged class — initially requires only base meta-ontology
            // slots, *not* demo:tag.
            b.add_resource(mk_class(
                "urn:eigenius:demo:Tagged",
                "Tagged",
                vec![wk::IS_A, wk::DESCRIPTION, wk::SHORT_NAME],
            ))
            .unwrap();

            // demo:LowerC is both a Class (so subclass_of references
            // to it pass Rule 14) AND an instance of demo:Tagged
            // (so it inherits Tagged's requires set). Valid against
            // the initial Tagged definition.
            let mut lower_c = Resource::new(iri("urn:eigenius:demo:LowerC"));
            lower_c.set(
                iri(wk::IS_A),
                Value::Array(vec![
                    Value::ResourceRef(iri(wk::CLASS)),
                    Value::ResourceRef(iri("urn:eigenius:demo:Tagged")),
                ]),
            );
            lower_c.set(iri(wk::DESCRIPTION), Value::String("lower c".into()));
            lower_c.set(iri(wk::SHORT_NAME), Value::String("LowerC".into()));
            b.add_resource(lower_c).unwrap();

            Arc::new(b.build(storage.clone()))
        };
        backend.store_layer(&mid).unwrap();

        // New layer:
        // 1. Redefines demo:Tagged to ALSO require demo:tag.
        //    → demo:LowerC (lower layer) lacks demo:tag → cascade
        //      will want to tombstone demo:LowerC.
        // 2. Defines demo:UpperX with subclass_of: [demo:LowerC],
        //    where UpperX itself has demo:tag set (so per-new-layer
        //    validates).
        //    → After cascade tombstones LowerC, UpperX's subclass_of
        //      reference dangles → Rule 14 → CascadeAbort.
        let mut new_b = LayerBuilder::new("new", Some(Arc::clone(&mid)));
        new_b
            .add_resource(mk_class(
                "urn:eigenius:demo:Tagged",
                "Tagged",
                vec![
                    wk::IS_A,
                    wk::DESCRIPTION,
                    wk::SHORT_NAME,
                    "urn:eigenius:demo:tag",
                ],
            ))
            .unwrap();
        let mut upper_x = mk_class(
            "urn:eigenius:demo:UpperX",
            "UpperX",
            vec![wk::IS_A, wk::DESCRIPTION, wk::SHORT_NAME],
        );
        upper_x.set(
            iri(wk::PARENT_CLASSES),
            Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:demo:LowerC"))]),
        );
        upper_x.set(iri("urn:eigenius:demo:tag"), Value::String("good".into()));
        new_b.add_resource(upper_x).unwrap();

        let mut ws = CommitWorkingSet::in_memory();
        let result = commit_layer(
            new_b,
            storage.clone(),
            backend.as_ref(),
            CommitPolicy::CascadeTombstone,
            &mut ws,
        );

        match result {
            Err(CommitError::CascadeAbort {
                cascade_tombstones,
                errors,
                ..
            }) => {
                // Cascade had accumulated demo:LowerC in its tombstone
                // set before realizing it would break demo:UpperX.
                assert!(
                    cascade_tombstones.contains(&iri("urn:eigenius:demo:LowerC")),
                    "expected demo:LowerC in cascade set; got {:?}",
                    cascade_tombstones
                );
                // The breakage error is on demo:UpperX (new-layer).
                let breakage_ids: std::collections::BTreeSet<String> = errors
                    .iter()
                    .filter_map(|e| e.resource_id.as_ref().map(|i| i.as_str().to_string()))
                    .collect();
                assert!(
                    breakage_ids.contains("urn:eigenius:demo:UpperX"),
                    "expected demo:UpperX in breakage errors; got {:?}",
                    breakage_ids
                );
            }
            Err(other) => panic!("expected CascadeAbort, got {other:?}"),
            Ok(outcome) => panic!(
                "expected CascadeAbort, but cascade succeeded with tombstones {:?}",
                outcome.cascade_tombstones
            ),
        }
    }

    /// Regression: a new layer redefines a Property to narrow its
    /// `allows_only` set so an existing value is no longer admissible.
    /// Resources carrying that property are dependents — the pass
    /// must walk the chain (literal-presence scan, since the triple
    /// index doesn't cover this) and surface the violation.
    #[test]
    fn property_allows_only_narrowing_invalidates_carriers() {
        let (root, storage, _backend) = build_chain_with_class_instances();

        // Mid: define a Property `demo:color` with allows_only
        // [demo:red, demo:blue], plus a resource using `demo:blue`.
        let mid = {
            let mut b = LayerBuilder::new("mid", Some(Arc::clone(&root)));
            // demo:red, demo:blue as resources for the IRI references
            // to resolve against.
            for (id, name) in [
                ("urn:eigenius:demo:red", "red"),
                ("urn:eigenius:demo:blue", "blue"),
            ] {
                let mut r = mk_class(id, name, vec![]);
                r.set(iri(wk::DESCRIPTION), Value::String("color value".into()));
                b.add_resource(r).unwrap();
            }
            // demo:color property with allows_only = [red, blue]
            let mut color_prop = Resource::new(iri("urn:eigenius:demo:color"));
            color_prop.set(
                iri(wk::IS_A),
                Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
            );
            color_prop.set(iri(wk::DESCRIPTION), Value::String("color".into()));
            color_prop.set(iri(wk::SHORT_NAME), Value::String("color".into()));
            color_prop.set(
                iri(wk::DATA_TYPE_PROP),
                Value::ResourceRef(iri(wk::RESOURCE)),
            );
            color_prop.set(
                iri(wk::ALLOWS_ONLY),
                Value::Array(vec![
                    Value::ResourceRef(iri("urn:eigenius:demo:red")),
                    Value::ResourceRef(iri("urn:eigenius:demo:blue")),
                ]),
            );
            b.add_resource(color_prop).unwrap();

            // demo:thing_1 carries demo:color = demo:blue
            let mut thing = mk_instance(
                "urn:eigenius:demo:thing_1",
                wk::CLASS, // simplest valid class membership
                vec![(
                    "urn:eigenius:demo:color",
                    Value::ResourceRef(iri("urn:eigenius:demo:blue")),
                )],
            );
            // Add required properties for Class instance.
            thing.set(iri(wk::DESCRIPTION), Value::String("blue thing".into()));
            thing.set(iri(wk::SHORT_NAME), Value::String("thing_1".into()));
            b.add_resource(thing).unwrap();
            Arc::new(b.build(storage.clone()))
        };
        let mut ws = CommitWorkingSet::in_memory();
        retroactive_validate(&mid, &mut ws).unwrap();
        assert_eq!(
            ws.violations.len(),
            0,
            "mid layer should be retroactively valid"
        );

        // New layer narrows demo:color's allows_only to [red] only.
        // thing_1's `demo:color = demo:blue` is no longer admissible.
        let new = {
            let mut b = LayerBuilder::new("new", Some(Arc::clone(&mid)));
            let mut color_prop = Resource::new(iri("urn:eigenius:demo:color"));
            color_prop.set(
                iri(wk::IS_A),
                Value::Array(vec![Value::ResourceRef(iri(wk::PROPERTY))]),
            );
            color_prop.set(iri(wk::DESCRIPTION), Value::String("color".into()));
            color_prop.set(iri(wk::SHORT_NAME), Value::String("color".into()));
            color_prop.set(
                iri(wk::DATA_TYPE_PROP),
                Value::ResourceRef(iri(wk::RESOURCE)),
            );
            color_prop.set(
                iri(wk::ALLOWS_ONLY),
                Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:demo:red"))]),
            );
            b.add_resource(color_prop).unwrap();
            Arc::new(b.build(storage.clone()))
        };

        let mut ws = CommitWorkingSet::in_memory();
        retroactive_validate(&new, &mut ws).unwrap();
        assert!(
            !ws.violations.is_empty(),
            "narrowing allows_only must invalidate carriers of the dropped value"
        );
        // The violation should mention the allows_only rule.
        let drained = ws.violations.drain(100);
        assert!(
            drained.errors.iter().any(|e| matches!(
                e.rule,
                crate::validation::ValidationRule::AllowedValueViolation
            )),
            "expected AllowedValueViolation; got {:?}",
            drained.errors
        );
    }
}
