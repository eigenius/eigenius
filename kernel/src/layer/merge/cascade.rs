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

//! Cascade impact analysis (D20 §8).
//!
//! Walks each resolution's drop / rename targets and surfaces every
//! downstream consequence as a [`CascadeItem`]. The kernel refuses
//! to commit a merge whose downstream consequences haven't been
//! explicitly acknowledged via [`CascadeAck`].
//!
//! v1 implements `OrphanedReference` and `OrphanedTyping` — the two
//! cascade kinds that fall out directly from resource references and
//! `is_a` membership. `InvalidatedSignature` (type-checker driven)
//! and `InvalidatedTrace` (trace-store driven) require integration
//! surfaces not yet stood up; they stay in the enum for forward
//! compat.

use super::conflict::{classify_conflicts, ConflictId, MergeSpan, Side};
use super::lca::ancestor_chain_iris;
use super::resolve::{apply_quotient_resolution, MergeResolution, SchemaQuotient};
use super::MergeError;
use crate::layer::handle::LayerTopology;
use crate::layer::LayerId;
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use crate::ontology::well_known as wk;
use crate::storage::PersistentBackend;
use std::collections::BTreeSet;
use std::fmt;

/// A path into a Resource's property graph, used to localise a
/// cascade item's reference site (D20 §8). A top-level reference is a
/// single-element path; references inside `Embedded` resources or
/// `Array` items carry the chain of property IRIs leading to the
/// reference site.
///
/// Used as data only — the kernel doesn't navigate paths back into
/// resources, just renders them for the resolution UI.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PropertyPath(pub Vec<Iri>);

impl fmt::Display for PropertyPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<&str> = self.0.iter().map(|i| i.as_str()).collect();
        write!(f, "{}", parts.join("/"))
    }
}

/// Deterministic identifier for a single cascade item. Computed from
/// the item's variant + the IRIs involved + the property path, so
/// re-running the cascade computation against the same span produces
/// the same id and acknowledgments stay stable across retries.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CascadeItemId(pub String);

/// A single downstream consequence of a resolution (D20 §8).
///
/// v1 implements `OrphanedReference` and `OrphanedTyping` — the two
/// cascade kinds that fall out directly from resource references and
/// `is_a` membership. `InvalidatedSignature` (type-checker driven)
/// and `InvalidatedTrace` (trace-store driven) require integration
/// surfaces this milestone doesn't yet light up; they stay in the
/// enum for forward compat.
#[derive(Debug, Clone, PartialEq)]
pub enum CascadeItem {
    /// A resource references something the resolution drops or moves.
    /// The user must acknowledge that the reference will no longer
    /// resolve to its original target after the merge.
    OrphanedReference {
        /// The resource carrying the reference.
        resource: Iri,
        /// The IRI that's being dropped or renamed.
        dropped_target: Iri,
        /// Path inside `resource` where the reference lives.
        location: PropertyPath,
    },
    /// A class's resources lose their typing — `is_a: [dropped_class,
    /// ...]` no longer resolves the dropped class.
    OrphanedTyping {
        /// The class IRI being dropped.
        class: Iri,
        /// Resources whose `is_a` includes `class`.
        affected_resources: Vec<Iri>,
    },
    /// (reserved — does not fire in v1) A program signature no
    /// longer type-checks after the resolution. Wiring the
    /// type-checker through cascade analysis is a separate surface
    /// not yet stood up.
    InvalidatedSignature {
        program: Iri,
        signature_problem: String,
    },
    /// (reserved — does not fire in v1) A trace references content
    /// that becomes inconsistent.
    InvalidatedTrace { trace: String, reason: String },
}

impl CascadeItem {
    /// Compute the stable id for this cascade item. Same item
    /// produced twice (e.g., across retries) yields the same id, so
    /// acknowledgments carry across resubmissions.
    pub fn id(&self) -> CascadeItemId {
        match self {
            CascadeItem::OrphanedReference {
                resource,
                dropped_target,
                location,
            } => CascadeItemId(format!(
                "orphaned_ref:{resource}:{dropped_target}:{location}"
            )),
            CascadeItem::OrphanedTyping { class, .. } => {
                CascadeItemId(format!("orphaned_typing:{class}"))
            }
            CascadeItem::InvalidatedSignature { program, .. } => {
                CascadeItemId(format!("invalidated_sig:{program}"))
            }
            CascadeItem::InvalidatedTrace { trace, .. } => {
                CascadeItemId(format!("invalidated_trace:{trace}"))
            }
        }
    }
}

/// The full set of downstream consequences for a list of
/// resolutions, computed deterministically against the span. Returned
/// from [`preview_cascade`]; the UI surfaces this between "user picked
/// a resolution" and "user commits."
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CascadePreview {
    pub items: Vec<CascadeItem>,
}

impl CascadePreview {
    /// Collect every item's id into a sorted, deduplicated vec.
    pub fn item_ids(&self) -> Vec<CascadeItemId> {
        let mut ids: Vec<CascadeItemId> = self.items.iter().map(|i| i.id()).collect();
        ids.sort();
        ids.dedup();
        ids
    }
}

/// User acknowledgment of a single cascade item. The kernel only
/// reads `item_id`; `user`/`acknowledged_at` are wire-only fields
/// preserved by the resolution submission surface for audit logs but
/// not consulted by the merge logic.
#[derive(Debug, Clone, PartialEq)]
pub struct CascadeAck {
    pub item_id: CascadeItemId,
}

/// Compute the cascade preview for a list of resolutions against a
/// span (D20 §7.3). Walks each resolution's drop / rename targets
/// and surfaces every downstream consequence as a `CascadeItem`. The
/// returned preview's items are deduplicated by id and sorted
/// deterministically so retries produce identical previews.
pub fn preview_cascade(
    span: &MergeSpan,
    resolutions: &[MergeResolution],
    backend: &dyn PersistentBackend,
) -> Result<CascadePreview, MergeError> {
    let topology = backend.load_topology().map_err(MergeError::Storage)?;
    let mut items: Vec<CascadeItem> = Vec::new();
    for resolution in resolutions {
        let mut sub = cascade_for_resolution(span, resolution, &topology, backend)?;
        items.append(&mut sub);
    }
    items.sort_by_key(|a| a.id());
    items.dedup_by(|a, b| a.id() == b.id());
    Ok(CascadePreview { items })
}

/// Dispatch a single resolution to its cascade-computation helper.
fn cascade_for_resolution(
    span: &MergeSpan,
    resolution: &MergeResolution,
    topology: &LayerTopology,
    backend: &dyn PersistentBackend,
) -> Result<Vec<CascadeItem>, MergeError> {
    match resolution {
        MergeResolution::Witness { .. } => {
            // A type-checked witness produces a value of the right
            // class by construction; v1 surfaces no cascade items.
            Ok(Vec::new())
        }
        MergeResolution::Rename { side, old_iri, .. } => {
            cascade_for_rename(span, *side, old_iri, topology, backend)
        }
        MergeResolution::SchemaQuotient { conflict, quotient } => {
            cascade_for_quotient(span, conflict, *quotient, topology, backend)
        }
        MergeResolution::Restructure { .. } => {
            // 15f v1: restructure's "subsumed subclass arrows" check
            // requires walking the path-equation closure under the
            // augmented ancestor, which isn't wired this milestone.
            // Until then, the apply path validates structurally and
            // cascade analysis surfaces nothing — the user takes
            // responsibility for the abstraction-raising effect.
            Ok(Vec::new())
        }
    }
}

/// Cascade for a `Rename`: references to `old_iri` outside the
/// renamed side's slice (i.e., on the other branch or in the
/// ancestor chain) aren't rewritten by the rename apply and so will
/// silently re-bind post-merge. Each such reference becomes an
/// `OrphanedReference` item.
fn cascade_for_rename(
    span: &MergeSpan,
    side: Side,
    old_iri: &Iri,
    topology: &LayerTopology,
    backend: &dyn PersistentBackend,
) -> Result<Vec<CascadeItem>, MergeError> {
    let mut items = Vec::new();
    let other_sources = match side {
        Side::A => &span.sources_b,
        Side::B => &span.sources_a,
    };
    for (iri, layer_id) in other_sources {
        let resource = match backend
            .try_load_resource(layer_id, iri)
            .map_err(MergeError::Storage)?
        {
            Some(r) => r,
            None => continue,
        };
        collect_orphaned_refs(&resource, iri, old_iri, &mut Vec::new(), &mut items);
    }
    let ancestor_iris = ancestor_chain_iris(&span.ancestor, topology, backend)?;
    for (iri, layer_id) in ancestor_iris {
        let resource = match backend
            .try_load_resource(&layer_id, &iri)
            .map_err(MergeError::Storage)?
        {
            Some(r) => r,
            None => continue,
        };
        collect_orphaned_refs(&resource, &iri, old_iri, &mut Vec::new(), &mut items);
    }
    Ok(items)
}

/// Cascade for a `SchemaQuotient`: for each dropped IRI (per the
/// quotient's drop sets), surface (a) every reference from a
/// non-dropped resource as `OrphanedReference`, and (b) every
/// resource whose `is_a` lists the dropped IRI as `OrphanedTyping`.
fn cascade_for_quotient(
    span: &MergeSpan,
    conflict: &ConflictId,
    quotient: SchemaQuotient,
    topology: &LayerTopology,
    backend: &dyn PersistentBackend,
) -> Result<Vec<CascadeItem>, MergeError> {
    let conflicts = classify_conflicts(span, backend).map_err(MergeError::Storage)?;
    let typed = conflicts
        .iter()
        .find(|c| c.id == *conflict)
        .ok_or_else(|| MergeError::ConflictNotFound(conflict.clone()))?;
    let application = apply_quotient_resolution(typed, quotient)?;

    let mut items = Vec::new();
    let dropped_a: BTreeSet<&Iri> = application.drop_from_branch_a.iter().collect();
    let dropped_b: BTreeSet<&Iri> = application.drop_from_branch_b.iter().collect();
    let all_dropped: BTreeSet<&Iri> = dropped_a.union(&dropped_b).copied().collect();

    let surviving: Vec<(&Iri, &LayerId)> = span
        .sources_a
        .iter()
        .filter(|(iri, _)| !dropped_a.contains(iri))
        .chain(
            span.sources_b
                .iter()
                .filter(|(iri, _)| !dropped_b.contains(iri)),
        )
        .collect();
    for (iri, layer_id) in surviving {
        let resource = match backend
            .try_load_resource(layer_id, iri)
            .map_err(MergeError::Storage)?
        {
            Some(r) => r,
            None => continue,
        };
        for dropped in &all_dropped {
            collect_orphaned_refs(&resource, iri, dropped, &mut Vec::new(), &mut items);
        }
    }

    let ancestor_iris = ancestor_chain_iris(&span.ancestor, topology, backend)?;
    for (iri, layer_id) in &ancestor_iris {
        if all_dropped.contains(iri) {
            continue;
        }
        let resource = match backend
            .try_load_resource(layer_id, iri)
            .map_err(MergeError::Storage)?
        {
            Some(r) => r,
            None => continue,
        };
        for dropped in &all_dropped {
            collect_orphaned_refs(&resource, iri, dropped, &mut Vec::new(), &mut items);
        }
    }

    // OrphanedTyping: resources whose `is_a` includes a dropped class.
    let is_a_iri = Iri::parse(wk::IS_A).expect("IS_A IRI");
    for dropped in &all_dropped {
        let mut affected: Vec<Iri> = Vec::new();
        let sweep: Vec<(&Iri, &LayerId)> = span
            .sources_a
            .iter()
            .chain(span.sources_b.iter())
            .chain(ancestor_iris.iter().map(|(i, l)| (i, l)))
            .collect();
        for (iri, layer_id) in sweep {
            if &iri == dropped {
                continue;
            }
            let resource = match backend
                .try_load_resource(layer_id, iri)
                .map_err(MergeError::Storage)?
            {
                Some(r) => r,
                None => continue,
            };
            if let Some(value) = resource.get(&is_a_iri) {
                if value
                    .as_iri_array()
                    .iter()
                    .any(|class_iri| class_iri == *dropped)
                {
                    affected.push(iri.clone());
                }
            }
        }
        if !affected.is_empty() {
            affected.sort();
            affected.dedup();
            items.push(CascadeItem::OrphanedTyping {
                class: (*dropped).clone(),
                affected_resources: affected,
            });
        }
    }

    Ok(items)
}

/// Walk every property of `resource`, emitting an `OrphanedReference`
/// item for each `ResourceRef(target)` hit. Recurses through nested
/// `Embedded` resources and `Array` items, tracking the property
/// path to the reference site.
fn collect_orphaned_refs(
    resource: &Resource,
    resource_iri: &Iri,
    target: &Iri,
    path: &mut Vec<Iri>,
    out: &mut Vec<CascadeItem>,
) {
    for (prop, value) in resource.properties() {
        path.push(prop.clone());
        collect_orphaned_refs_in_value(value, resource_iri, target, path, out);
        path.pop();
    }
}

fn collect_orphaned_refs_in_value(
    value: &crate::ontology::resource::Value,
    resource_iri: &Iri,
    target: &Iri,
    path: &mut Vec<Iri>,
    out: &mut Vec<CascadeItem>,
) {
    use crate::ontology::resource::Value;
    match value {
        Value::ResourceRef(r) if r == target => {
            out.push(CascadeItem::OrphanedReference {
                resource: resource_iri.clone(),
                dropped_target: target.clone(),
                location: PropertyPath(path.clone()),
            });
        }
        Value::Array(items) => {
            for v in items {
                collect_orphaned_refs_in_value(v, resource_iri, target, path, out);
            }
        }
        Value::Embedded(boxed) => {
            for (prop, inner_value) in boxed.properties() {
                path.push(prop.clone());
                collect_orphaned_refs_in_value(inner_value, resource_iri, target, path, out);
                path.pop();
            }
        }
        _ => {}
    }
}

/// Verify the supplied acknowledgments cover every item in the
/// cascade preview. Returns `Ok(())` if every cascade id is acked;
/// `Err(MergeError::IncompleteAcknowledgments { missing })` otherwise.
pub(crate) fn verify_cascade_acknowledgments(
    preview: &CascadePreview,
    acks: &[CascadeAck],
) -> Result<(), MergeError> {
    let acked: BTreeSet<&CascadeItemId> = acks.iter().map(|a| &a.item_id).collect();
    let mut missing: Vec<CascadeItemId> = Vec::new();
    for id in preview.item_ids() {
        if !acked.contains(&id) {
            missing.push(id);
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(MergeError::IncompleteAcknowledgments { missing })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::merge::resolve::merge_with_resolutions;
    use crate::layer::merge::test_support::{
        build_span, build_span_arc, build_witness_fixture, iri, make_resource, make_var_resource,
    };
    use crate::ontology::resource::Value;

    #[test]
    fn cascade_for_witness_is_empty() {
        // Witnesses are type-checked at submission time, so the
        // merged value is well-typed by construction — no cascade
        // items for v1.
        let (_span, backend, handle, _storage) = build_witness_fixture(make_var_resource("b"));
        let conflict_id = ConflictId::from_iri("iri_collision", &iri("urn:test:patient_42"));
        let resolution = MergeResolution::Witness {
            conflict: conflict_id,
            comorphism: handle.iri.clone(),
        };
        // Use a fresh span to keep cascade analysis decoupled from
        // the witness fixture's branches (which have no inter-refs).
        let (clean_span, clean_backend) = build_span(Vec::new(), Vec::new(), Vec::new());
        let preview = preview_cascade(
            &clean_span,
            std::slice::from_ref(&resolution),
            &clean_backend,
        )
        .expect("preview_cascade should succeed");
        assert!(
            preview.items.is_empty(),
            "witness cascade should be empty; got {preview:?}"
        );
        // unused-but-must-stay-alive for the witness fixture's
        // Arc-backed backend; silences `Backend dropped while
        // resources still referenced`.
        drop(backend);
    }

    #[test]
    fn cascade_for_rename_surfaces_other_branch_orphaned_references() {
        // Branch A introduces `Patient`; branch B introduces a
        // `Profile` resource referencing `Patient`. Renaming A's
        // Patient → BillingPatient does NOT walk branch B's slice,
        // so the Profile's reference becomes an `OrphanedReference`
        // surfaced by cascade analysis.
        let patient_iri = "urn:project:Patient";
        let renamed_iri = "urn:project:billing:Patient";
        let profile_iri = "urn:project:profile";
        let profile_for_iri = "urn:project:profile_for";

        let patient = make_resource(patient_iri, &[wk::CLASS], &[]);
        let profile = make_resource(
            profile_iri,
            &[wk::CLASS],
            &[(profile_for_iri, Value::ResourceRef(iri(patient_iri)))],
        );
        let (span, backend, _storage) = build_span_arc(Vec::new(), vec![patient], vec![profile]);

        let resolution = MergeResolution::Rename {
            conflict: ConflictId::from_iri("iri_collision", &iri(patient_iri)),
            side: Side::A,
            old_iri: iri(patient_iri),
            new_iri: iri(renamed_iri),
        };
        let preview = preview_cascade(&span, std::slice::from_ref(&resolution), &*backend)
            .expect("preview_cascade should succeed");
        assert_eq!(
            preview.items.len(),
            1,
            "expected one orphaned ref; got {preview:?}"
        );
        match &preview.items[0] {
            CascadeItem::OrphanedReference {
                resource,
                dropped_target,
                location,
            } => {
                assert_eq!(resource.as_str(), profile_iri);
                assert_eq!(dropped_target.as_str(), patient_iri);
                assert_eq!(location.0.len(), 1);
                assert_eq!(location.0[0].as_str(), profile_for_iri);
            }
            other => panic!("expected OrphanedReference, got {other:?}"),
        }
    }

    #[test]
    fn cascade_for_rename_walks_into_embedded_resources() {
        // The ref to the renamed IRI lives inside an Embedded
        // resource nested in an Array. Cascade walker must descend
        // both shapes and carry the property path through them.
        let patient_iri = "urn:project:Patient";
        let renamed_iri = "urn:project:billing:Patient";
        let report_iri = "urn:project:report";
        let entries_iri = "urn:project:entries";
        let about_iri = "urn:project:about";

        let patient = make_resource(patient_iri, &[wk::CLASS], &[]);
        let mut embedded = Resource::new_embedded();
        embedded.set(iri(about_iri), Value::ResourceRef(iri(patient_iri)));
        let report = make_resource(
            report_iri,
            &[wk::CLASS],
            &[(
                entries_iri,
                Value::Array(vec![Value::Embedded(Box::new(embedded))]),
            )],
        );
        let (span, backend, _storage) = build_span_arc(Vec::new(), vec![patient], vec![report]);

        let resolution = MergeResolution::Rename {
            conflict: ConflictId::from_iri("iri_collision", &iri(patient_iri)),
            side: Side::A,
            old_iri: iri(patient_iri),
            new_iri: iri(renamed_iri),
        };
        let preview = preview_cascade(&span, std::slice::from_ref(&resolution), &*backend)
            .expect("preview_cascade should succeed");
        assert_eq!(preview.items.len(), 1);
        match &preview.items[0] {
            CascadeItem::OrphanedReference { location, .. } => {
                let path: Vec<&str> = location.0.iter().map(|i| i.as_str()).collect();
                assert_eq!(path, vec![entries_iri, about_iri]);
            }
            other => panic!("expected OrphanedReference, got {other:?}"),
        }
    }

    #[test]
    fn cascade_for_rename_is_empty_when_no_external_references() {
        // No other-branch / ancestor reference to old_iri → empty
        // cascade. (The rename apply walks the rename side, so its
        // own self-references are rewritten and don't show up here.)
        let patient_iri = "urn:project:Patient";
        let (span, backend, _storage) = build_span_arc(
            Vec::new(),
            Vec::new(),
            vec![make_resource(patient_iri, &[wk::CLASS], &[])],
        );
        let resolution = MergeResolution::Rename {
            conflict: ConflictId::from_iri("iri_collision", &iri(patient_iri)),
            side: Side::B,
            old_iri: iri(patient_iri),
            new_iri: iri("urn:project:billing:Patient"),
        };
        let preview = preview_cascade(&span, std::slice::from_ref(&resolution), &*backend)
            .expect("preview_cascade should succeed");
        assert!(preview.items.is_empty(), "cascade should be empty");
    }

    #[test]
    fn cascade_for_quotient_surfaces_orphaned_typing() {
        // `Animal` is dropped from branch A via `KeepOne`. The
        // ancestor declares `pet_42 is_a Animal`; that resource
        // surfaces as an `OrphanedTyping` item.
        let animal_iri = "urn:test:Animal";
        let pet_iri = "urn:test:pet_42";

        let animal = make_resource(animal_iri, &[wk::CLASS], &[]);
        let pet = make_resource(pet_iri, &[animal_iri], &[]);

        // Both branches modify `Animal` so it surfaces as a
        // KindMismatch conflict (Class vs Property) — gives us a
        // classified conflict to attach the quotient to.
        let animal_as_property = make_resource(animal_iri, &[wk::PROPERTY], &[]);
        let (span, backend) = build_span(vec![pet], vec![animal], vec![animal_as_property]);
        let conflicts = classify_conflicts(&span, &backend).unwrap();
        let conflict_id = conflicts[0].id.clone();

        // KeepOne winner=B drops `Animal` from branch A.
        let resolution = MergeResolution::SchemaQuotient {
            conflict: conflict_id,
            quotient: SchemaQuotient::KeepOne { winner: Side::B },
        };
        let preview = preview_cascade(&span, std::slice::from_ref(&resolution), &backend)
            .expect("preview_cascade should succeed");
        let typings: Vec<&CascadeItem> = preview
            .items
            .iter()
            .filter(|item| matches!(item, CascadeItem::OrphanedTyping { .. }))
            .collect();
        assert_eq!(
            typings.len(),
            1,
            "expected one OrphanedTyping; got {preview:?}"
        );
        match typings[0] {
            CascadeItem::OrphanedTyping {
                class,
                affected_resources,
            } => {
                assert_eq!(class.as_str(), animal_iri);
                assert_eq!(affected_resources.len(), 1);
                assert_eq!(affected_resources[0].as_str(), pet_iri);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn cascade_item_id_is_deterministic_across_calls() {
        // Same span + same resolution must produce the same cascade
        // item ids — acknowledgments depend on this for retries.
        let patient_iri = "urn:project:Patient";
        let profile_iri = "urn:project:profile";
        let patient = make_resource(patient_iri, &[wk::CLASS], &[]);
        let profile = make_resource(
            profile_iri,
            &[wk::CLASS],
            &[(
                "urn:project:profile_for",
                Value::ResourceRef(iri(patient_iri)),
            )],
        );
        let (span, backend, _storage) = build_span_arc(Vec::new(), vec![patient], vec![profile]);
        let resolution = MergeResolution::Rename {
            conflict: ConflictId::from_iri("iri_collision", &iri(patient_iri)),
            side: Side::A,
            old_iri: iri(patient_iri),
            new_iri: iri("urn:project:billing:Patient"),
        };
        let p1 = preview_cascade(&span, std::slice::from_ref(&resolution), &*backend).unwrap();
        let p2 = preview_cascade(&span, std::slice::from_ref(&resolution), &*backend).unwrap();
        assert_eq!(p1.item_ids(), p2.item_ids());
    }

    #[test]
    fn merge_with_resolutions_rejects_missing_cascade_acks() {
        // Rename produces a cascade item; commit without an ack
        // surfaces as `IncompleteAcknowledgments` with the missing
        // id.
        let patient_iri = "urn:project:Patient";
        let profile_iri = "urn:project:profile";
        let patient = make_resource(patient_iri, &[wk::CLASS], &[]);
        let profile = make_resource(
            profile_iri,
            &[wk::CLASS],
            &[(
                "urn:project:profile_for",
                Value::ResourceRef(iri(patient_iri)),
            )],
        );
        let (span, backend, storage) = build_span_arc(Vec::new(), vec![patient], vec![profile]);

        let resolutions = vec![MergeResolution::Rename {
            conflict: ConflictId::from_iri("iri_collision", &iri(patient_iri)),
            side: Side::A,
            old_iri: iri(patient_iri),
            new_iri: iri("urn:project:billing:Patient"),
        }];
        // No acks supplied — gate must reject.
        let result = merge_with_resolutions(
            &span,
            resolutions,
            Vec::new(),
            Vec::new(),
            storage,
            &*backend,
        );
        match result {
            Err(MergeError::IncompleteAcknowledgments { missing }) => {
                assert_eq!(missing.len(), 1);
                assert!(missing[0]
                    .0
                    .starts_with("orphaned_ref:urn:project:profile:urn:project:Patient"));
            }
            other => panic!("expected IncompleteAcknowledgments, got {other:?}"),
        }
    }

    #[test]
    fn merge_with_resolutions_accepts_acked_cascade_then_proceeds_to_apply() {
        // With every cascade item acked, the gate passes and the
        // surface continues to the per-resolution dispatch — which
        // (currently) short-circuits with the resolution's
        // `*NotYetWired` error. ConflictNotFound surfaces if the
        // synthetic id doesn't classify, so this test pins the
        // post-gate progression.
        let patient_iri = "urn:project:Patient";
        let profile_iri = "urn:project:profile";
        let patient = make_resource(patient_iri, &[wk::CLASS], &[]);
        let profile = make_resource(
            profile_iri,
            &[wk::CLASS],
            &[(
                "urn:project:profile_for",
                Value::ResourceRef(iri(patient_iri)),
            )],
        );
        let (span, backend, storage) = build_span_arc(Vec::new(), vec![patient], vec![profile]);

        let conflict_id = ConflictId::from_iri("iri_collision", &iri(patient_iri));
        let resolution = MergeResolution::Rename {
            conflict: conflict_id.clone(),
            side: Side::A,
            old_iri: iri(patient_iri),
            new_iri: iri("urn:project:billing:Patient"),
        };
        let preview = preview_cascade(&span, std::slice::from_ref(&resolution), &*backend).unwrap();
        let acks: Vec<CascadeAck> = preview
            .item_ids()
            .into_iter()
            .map(|item_id| CascadeAck { item_id })
            .collect();

        // Open-world classifier doesn't surface IriCollision for
        // disjoint branches, so the dispatch reports
        // ConflictNotFound after the gate passes.
        let result = merge_with_resolutions(
            &span,
            vec![resolution],
            acks,
            Vec::new(),
            storage,
            &*backend,
        );
        match result {
            Err(MergeError::ConflictNotFound(id)) => assert_eq!(id, conflict_id),
            other => panic!("expected ConflictNotFound after gate, got {other:?}"),
        }
    }

    #[test]
    fn verify_cascade_acks_against_empty_preview_succeeds() {
        // No cascade items + no acks = trivially OK. Pins the
        // "Witness produces empty cascade" composition path.
        let preview = CascadePreview::default();
        verify_cascade_acknowledgments(&preview, &[])
            .expect("empty preview + empty acks should pass");
    }
}
