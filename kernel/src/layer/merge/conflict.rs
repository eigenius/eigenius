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

//! Conflict taxonomy + classifier + merge-span construction.
//!
//! The shared types every resolution variant builds on:
//!
//! - [`MergeSpan`] — the input shape (ancestor + two heads + per-side
//!   `iri_sources_since` maps), produced by [`build_merge_span`].
//! - [`TypedConflict`] / [`ConflictKind`] — the classifier's output
//!   alphabet, threaded through each resolution variant's apply
//!   function.
//! - [`MergeOutcome`] — the merge surface's result shape (Merged with
//!   a layer id, or NeedsResolution with the typed conflict list).
//! - [`classify_conflicts`] / [`detect_inheritance_cycles`] — the
//!   classifier surface.

use super::lca::{collect_iri_refs_into, find_iri_in_chain, iter_iri_values};
use super::MergeError;
use crate::layer::handle::LayerTopology;
use crate::layer::LayerId;
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use crate::ontology::well_known as wk;
use crate::storage::{PersistentBackend, StorageError};
use std::collections::{BTreeMap, BTreeSet};

// ─── Conflict taxonomy ─────────────────────────────────────────────────────

/// A merge-time conflict, paired with the identity needed for the
/// resolution protocol (D20 §7.1).
///
/// `ConflictId` is a stable identifier the kernel hands to clients so
/// they can submit a `MergeResolution` targeting a specific conflict.
/// v1 derives it from the IRI + a discriminator on the conflict kind;
/// future versions might index instead.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedConflict {
    pub id: ConflictId,
    pub kind: ConflictKind,
}

/// Stable handle on a single conflict within a merge attempt. Treat
/// as opaque on the wire; the kernel constructs it deterministically
/// so client retries against the same span get the same id back.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConflictId(pub String);

impl ConflictId {
    /// Construct a deterministic id from the conflict kind discriminator
    /// and the IRI(s) involved. The format is internal — clients should
    /// treat the string as opaque.
    pub(crate) fn from_iri(discriminator: &str, iri: &Iri) -> Self {
        Self(format!("{discriminator}:{iri}"))
    }

    /// Construct from a list of IRIs (for cycle-shaped conflicts).
    pub(crate) fn from_iris(discriminator: &str, iris: &[Iri]) -> Self {
        let joined = iris
            .iter()
            .map(|i| i.as_str())
            .collect::<Vec<_>>()
            .join(",");
        Self(format!("{discriminator}:{joined}"))
    }
}

/// Typed conflict kinds, narrowed for Eigon's open-world semantics.
///
/// Variants marked **(reserved)** keep stable wire positions but
/// never fire in v1 — they're carried forward for the cases where
/// Eigon gains additional structural constraints (e.g., D1-level
/// disjointness declarations) without churning the enum.
#[derive(Debug, Clone, PartialEq)]
pub enum ConflictKind {
    // ─── Stage 1: schema-shape ─────────────────────────────────────────
    /// A Property's `data_type` differs across branches. `data_type` is
    /// single-valued (a property has exactly one primitive type), so the
    /// merge has no monotonically-safe option — the user must pick or
    /// witness.
    PropertyDataType {
        property: Iri,
        branch_a: Iri,
        branch_b: Iri,
        /// The ancestor's value, if the property existed before the
        /// branches diverged.
        ancestor: Option<Iri>,
    },

    /// The same IRI is declared as a Class on one branch and a Property
    /// on the other. The kind of an Eigon resource is single-valued; no
    /// monotonic combination exists.
    KindMismatch {
        iri: Iri,
        branch_a_kind: ResourceKind,
        branch_b_kind: ResourceKind,
    },

    // ─── Stage 2: equation-closure ────────────────────────────────────
    /// The merged `subclass_of` graph contains a cycle that didn't exist
    /// in either branch alone. The `subclass_of` relation must be a DAG
    /// — even under open-world semantics, a cycle would make every
    /// class transitively a subclass of itself and trivialise the
    /// hierarchy.
    InheritanceCycle { cycle: Vec<Iri> },

    /// (reserved — does not fire in v1) The merged set contains
    /// instances violating a class-disjointness declaration. Eigon has
    /// no `disjoint_classes` declarations today; this variant is
    /// carried for forward compatibility.
    DisjointnessViolation {
        class_a: Iri,
        class_b: Iri,
        offending_iris: Vec<Iri>,
    },

    /// (reserved — does not fire in v1) The path-equation closure of
    /// the merged ontology produces a contradiction not implied by
    /// either branch's closure. v1 subsumes these cases under
    /// `KindMismatch`; the variant stays for indexed-closure work that
    /// catches non-trivial transitive contradictions.
    PathEquationContradiction {
        equation_a: String,
        equation_b: String,
    },

    // ─── Stage 3: instance-body ───────────────────────────────────────
    /// Same IRI, materially different resource bodies on the two
    /// branches. Body equality is structural: same `is_a`, same
    /// property → value map. Disagreements anywhere produce this
    /// kind. The user resolves via `Witness` (typed merge) or one of
    /// the schema-quotient strategies.
    IriCollision {
        iri: Iri,
        branch_a_body: ResourceBody,
        branch_b_body: ResourceBody,
        ancestor_body: Option<ResourceBody>,
    },

    /// One branch deleted (tombstoned) the IRI; the other modified it.
    /// Reserved for v1.5 once Eigon ships an explicit tombstone shape
    /// — D23's current write model has no tombstone, so this variant
    /// stays in the enum for forward compatibility.
    DeletionConflict {
        iri: Iri,
        modified_body: ResourceBody,
        deleting_side: Side,
    },
}

/// Which kind of ontology resource an IRI is declared as. Single-valued
/// per D1 §3; a `KindMismatch` is exactly the disagreement on this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    Class,
    Property,
    /// Anything else (Resource instance, ConditionalRequirement, etc.).
    /// The `KindMismatch` detector promotes Class ↔ Property as the
    /// structurally interesting case; other-vs-other is rare enough to
    /// fold into `Other` without a finer discriminator.
    Other,
}

/// Which side of a span produced a particular value. Used by
/// `IriCollision` and `DeletionConflict` so the resolution UI can
/// label "branch A" and "branch B" consistently across conflicts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    A,
    B,
}

/// Snapshot of a `Resource` body for inclusion in conflict reports.
///
/// Carries the resource as the kernel sees it on each side, sortable
/// and serializable for wire surfacing. Cloned from the live
/// `Resource` at classifier time; the merge attempt that produces
/// the conflict is the only consumer (subsequent resolution
/// submissions re-load from the chain).
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceBody {
    /// The layer this body was sourced from. Useful for downstream
    /// cascade analysis and resolution debugging.
    pub source_layer: LayerId,
    /// The resource as committed at `source_layer`.
    pub resource: Resource,
}

// ─── Span representation ───────────────────────────────────────────────────

/// A merge span: the most-recent common ancestor, the two branch
/// tips, and per-branch maps from IRI to the layer that most
/// recently defined it (`iri_sources_since` shape).
///
/// `MergeSpan` is the input to every classifier and resolution applier
/// in this module. It is cheap to construct (no resource loading) and
/// trivially clonable for parallel classifier sub-passes.
#[derive(Debug, Clone)]
pub struct MergeSpan {
    pub ancestor: LayerId,
    pub head_a: LayerId,
    pub head_b: LayerId,
    pub sources_a: BTreeMap<Iri, LayerId>,
    pub sources_b: BTreeMap<Iri, LayerId>,
}

impl MergeSpan {
    /// IRIs that appear in both branches' contributions (i.e., that
    /// either side modified since the ancestor). These are the
    /// candidates for per-IRI conflict classification.
    pub fn shared_iris(&self) -> Vec<Iri> {
        let mut shared: Vec<Iri> = self
            .sources_a
            .keys()
            .filter(|i| self.sources_b.contains_key(*i))
            .cloned()
            .collect();
        shared.sort();
        shared
    }
}

/// Construct a [`MergeSpan`] from a pair of layer heads (D20 §7.2's
/// `(branch_head, candidate_chain)` input).
///
/// Finds the lowest common ancestor of the two heads via
/// [`crate::lattice::find_lca`], then computes each branch's
/// `sources_*` map via [`crate::lattice::iri_sources_since`]. The
/// result is the canonical merge-span input to every classifier and
/// resolution applier in this module.
///
/// Errors:
/// - `MergeError::NoCommonAncestor` if `head_a` and `head_b` share no
///   ancestor in the topology (unrelated DAG roots), or if either head
///   isn't in the topology — without an LCA there's no merge span to
///   build.
/// - `MergeError::Storage` on backend read failures.
///
/// `head_a == head_b` is valid: the LCA is the head itself, both
/// branches' `sources_*` are empty, and the resulting span describes
/// the trivial "merge with self" — every classifier returns no
/// conflicts.
pub fn build_merge_span(
    head_a: &LayerId,
    head_b: &LayerId,
    topology: &LayerTopology,
    backend: &dyn PersistentBackend,
) -> Result<MergeSpan, MergeError> {
    let ancestor = crate::lattice::find_lca(&[head_a.clone(), head_b.clone()], topology)
        .ok_or_else(|| MergeError::NoCommonAncestor {
            head_a: head_a.clone(),
            head_b: head_b.clone(),
        })?;
    let sources_a = crate::lattice::iri_sources_since(head_a, &ancestor, topology, backend)
        .map_err(MergeError::Storage)?;
    let sources_b = crate::lattice::iri_sources_since(head_b, &ancestor, topology, backend)
        .map_err(MergeError::Storage)?;
    Ok(MergeSpan {
        ancestor,
        head_a: head_a.clone(),
        head_b: head_b.clone(),
        sources_a,
        sources_b,
    })
}

// ─── Classifier ────────────────────────────────────────────────────────────

/// Classify the per-IRI disagreement at `iri` between the two branches.
///
/// Returns `None` if the disagreement is **monotonically safe** under
/// Eigon's open-world semantics (e.g., both branches added different
/// classes to a resource's `is_a` — multi-class membership is normal,
/// so the merge takes the union without flagging a conflict). Returns
/// `Some(kind)` for structural conflicts that need an explicit
/// resolution.
///
/// Three stages are decided per IRI:
///
///  1. **Kind** — if A typed `iri` as Class and B as Property (or any
///     other single-kind disagreement), surface `KindMismatch`. The
///     kind of an Eigon resource is single-valued (D1 §3), so this is
///     never monotonically safe.
///  2. **Schema shape** — for Property resources, single-valued
///     attributes that disagree produce a stage-1 conflict. The
///     foremost is `data_type`. Multi-valued attributes (`class_types`,
///     `domain`) combine monotonically and are NOT conflicts here.
///  3. **Instance body** — if neither stage 1 nor stage 2 produced a
///     conflict but the two resource bodies still differ materially,
///     surface `IriCollision`. The resource's class set (`is_a`) and
///     remaining property values are compared structurally.
pub fn classify_iri_disagreement(
    span: &MergeSpan,
    iri: &Iri,
    topology: &LayerTopology,
    backend: &dyn PersistentBackend,
) -> Result<Option<ConflictKind>, StorageError> {
    let layer_a = span.sources_a.get(iri).ok_or_else(|| {
        StorageError::NotFound(format!("iri {iri} missing from branch A sources"))
    })?;
    let layer_b = span.sources_b.get(iri).ok_or_else(|| {
        StorageError::NotFound(format!("iri {iri} missing from branch B sources"))
    })?;

    let resource_a = backend
        .try_load_resource(layer_a, iri)?
        .ok_or_else(|| StorageError::NotFound(format!("resource {iri} not in layer {layer_a}")))?;
    let resource_b = backend
        .try_load_resource(layer_b, iri)?
        .ok_or_else(|| StorageError::NotFound(format!("resource {iri} not in layer {layer_b}")))?;

    // Stage 1 — kind. Disagreement on Class vs Property is the
    // canonical kind mismatch; other-vs-other folds into `Other`
    // (rarely interesting in practice).
    let kind_a = classify_resource_kind(&resource_a);
    let kind_b = classify_resource_kind(&resource_b);
    if kind_a != kind_b {
        return Ok(Some(ConflictKind::KindMismatch {
            iri: iri.clone(),
            branch_a_kind: kind_a,
            branch_b_kind: kind_b,
        }));
    }

    // Stage 1 — Property's data_type (single-valued).
    if kind_a == ResourceKind::Property {
        if let Some(conflict) =
            check_property_data_type(iri, &resource_a, &resource_b, span, topology, backend)?
        {
            return Ok(Some(conflict));
        }
    }

    // Stage 3 — material body disagreement. Bodies that match
    // structurally (same is_a, same property → value map) are not a
    // conflict even when both branches modified the IRI — both
    // sides converged on the same value, which is the open-world
    // merge result.
    if !resource_bodies_equal(&resource_a, &resource_b) {
        let ancestor_body = load_ancestor_body(iri, span, topology, backend)?;
        return Ok(Some(ConflictKind::IriCollision {
            iri: iri.clone(),
            branch_a_body: ResourceBody {
                source_layer: layer_a.clone(),
                resource: resource_a,
            },
            branch_b_body: ResourceBody {
                source_layer: layer_b.clone(),
                resource: resource_b,
            },
            ancestor_body,
        }));
    }

    Ok(None)
}

/// Return the kind of an Eigon resource by inspecting its `is_a` field
/// for the well-known Class / Property markers. Resources typed into
/// neither (Resource instances, ConditionalRequirement, etc.) collapse
/// to `Other`.
fn classify_resource_kind(resource: &Resource) -> ResourceKind {
    let class_iri = Iri::parse(wk::CLASS).expect("CLASS IRI");
    let property_iri = Iri::parse(wk::PROPERTY).expect("PROPERTY IRI");
    if resource.is_instance_of(&class_iri) {
        ResourceKind::Class
    } else if resource.is_instance_of(&property_iri) {
        ResourceKind::Property
    } else {
        ResourceKind::Other
    }
}

/// Compare two Property resources' `data_type` declarations. Returns
/// `Some(PropertyDataType { ... })` if they disagree, `None` if both
/// agree (or neither declares it — unusual but the merge has nothing
/// to flag in that case).
fn check_property_data_type(
    property: &Iri,
    resource_a: &Resource,
    resource_b: &Resource,
    span: &MergeSpan,
    topology: &LayerTopology,
    backend: &dyn PersistentBackend,
) -> Result<Option<ConflictKind>, StorageError> {
    let data_type_iri = Iri::parse(wk::DATA_TYPE).expect("DATA_TYPE IRI");
    let type_a = resource_a.get(&data_type_iri).and_then(|v| v.as_iri());
    let type_b = resource_b.get(&data_type_iri).and_then(|v| v.as_iri());

    match (type_a, type_b) {
        (Some(a), Some(b)) if a != b => {
            let ancestor_body = load_ancestor_body(property, span, topology, backend)?;
            let ancestor_type = ancestor_body
                .as_ref()
                .and_then(|body| body.resource.get(&data_type_iri).and_then(|v| v.as_iri()));
            Ok(Some(ConflictKind::PropertyDataType {
                property: property.clone(),
                branch_a: a,
                branch_b: b,
                ancestor: ancestor_type,
            }))
        }
        _ => Ok(None),
    }
}

/// Structural equality on resource bodies — same `is_a` set, same
/// property keys, same values (recursively). Distinct from
/// `Resource: PartialEq` only in being explicit about the comparison
/// we want (today they coincide; calling out the dependency keeps
/// future `Resource` evolution from silently changing merge semantics).
fn resource_bodies_equal(a: &Resource, b: &Resource) -> bool {
    a == b
}

/// Best-effort load of the ancestor's body for an IRI. Returns `None`
/// if the IRI doesn't exist anywhere in the ancestor's parent chain
/// (i.e., both branches introduced it fresh) — distinct from a
/// storage error, which propagates.
///
/// Walks the ancestor's parent chain via [`find_iri_in_chain`]. The
/// `ResourceBackend::try_load_resource(layer_id, iri)` primitive is
/// a flat (layer, iri) lookup — it does NOT walk parents — so a
/// direct probe at the LCA misses IRIs defined deeper in the
/// ancestor's history. Walking the chain is the only correct shape.
fn load_ancestor_body(
    iri: &Iri,
    span: &MergeSpan,
    topology: &LayerTopology,
    backend: &dyn PersistentBackend,
) -> Result<Option<ResourceBody>, StorageError> {
    match find_iri_in_chain(&span.ancestor, iri, topology, backend)? {
        Some((source_layer, resource)) => Ok(Some(ResourceBody {
            source_layer,
            resource,
        })),
        None => Ok(None),
    }
}

// ─── Equation walker (stage 2) ─────────────────────────────────────────────

/// Detect inheritance cycles in the *merged* `subclass_of` graph that
/// don't exist in either branch alone (D20 §5.2 — the canonical
/// stage-2 conflict that survives the open-world narrowing).
///
/// Algorithm:
///  1. Materialise the candidate merged `subclass_of` graph: for each
///     Class IRI in either branch's contributions, take the union of
///     the two branches' `subclass_of` arrows. Existing classes from
///     the ancestor that neither branch touched stay untouched.
///  2. DFS for cycles. The first cycle encountered is reported; the
///     walker doesn't try to enumerate all cycles in v1 (typically
///     there's at most one and the user resolves it).
///
/// Returns `Vec<ConflictKind>` (one per cycle detected). Empty vec
/// means the merged graph is cycle-free.
pub fn detect_inheritance_cycles(
    span: &MergeSpan,
    topology: &LayerTopology,
    backend: &dyn PersistentBackend,
) -> Result<Vec<ConflictKind>, StorageError> {
    let merged_graph = build_merged_subclass_graph(span, topology, backend)?;
    let cycles = find_cycles(&merged_graph);
    Ok(cycles
        .into_iter()
        .map(|cycle| ConflictKind::InheritanceCycle { cycle })
        .collect())
}

/// Build the candidate merged `subclass_of` graph from the span.
///
/// Returns a `child → [parent]` adjacency map covering every Class
/// referenced by either branch's contributions. Ancestor-only classes
/// are excluded unless they appear as a parent of a contributed
/// class; the cycle detection only needs the reachable subgraph.
fn build_merged_subclass_graph(
    span: &MergeSpan,
    topology: &LayerTopology,
    backend: &dyn PersistentBackend,
) -> Result<BTreeMap<Iri, Vec<Iri>>, StorageError> {
    let class_iri = Iri::parse(wk::CLASS).expect("CLASS IRI");
    let subclass_iri = Iri::parse(wk::PARENT_CLASSES).expect("SUBCLASS_OF IRI");

    let mut graph: BTreeMap<Iri, BTreeSet<Iri>> = BTreeMap::new();

    // Walk each branch's contributions and aggregate `subclass_of`
    // arrows. Union semantics — both branches' arrows survive into
    // the merged graph.
    for (sources, _label) in [(&span.sources_a, "A"), (&span.sources_b, "B")] {
        for (iri, layer) in sources {
            let resource = match backend.try_load_resource(layer, iri)? {
                Some(r) => r,
                None => continue,
            };
            if !resource.is_instance_of(&class_iri) {
                continue;
            }
            let entry = graph.entry(iri.clone()).or_default();
            if let Some(value) = resource.get(&subclass_iri) {
                for parent in iter_iri_values(value) {
                    entry.insert(parent);
                }
            }
        }
    }

    // Also pull in ancestor-defined `subclass_of` edges for any class
    // referenced as a parent but not itself contributed by either
    // branch — those arrows survive into the merge unchanged. We walk
    // the ancestor's parent chain (NOT just the LCA layer) because
    // the class may have been defined deeper in the history;
    // `find_iri_in_chain` does that walk and returns the topmost
    // definition. Missing-everywhere is fine — it means the parent
    // class was branch-introduced and not in the ancestor's history.
    let mut contributed: BTreeSet<Iri> = BTreeSet::new();
    contributed.extend(span.sources_a.keys().cloned());
    contributed.extend(span.sources_b.keys().cloned());
    for iri in &contributed {
        let only_in_ancestor =
            !span.sources_a.contains_key(iri) && !span.sources_b.contains_key(iri);
        if !only_in_ancestor {
            continue;
        }
        let ancestor_res = match find_iri_in_chain(&span.ancestor, iri, topology, backend)? {
            Some((_, r)) => r,
            None => continue,
        };
        if !ancestor_res.is_instance_of(&class_iri) {
            continue;
        }
        // Ancestor-only class — its arrows pass through to merged.
        let entry = graph.entry(iri.clone()).or_default();
        if let Some(value) = ancestor_res.get(&subclass_iri) {
            for parent in iter_iri_values(value) {
                entry.insert(parent);
            }
        }
    }

    Ok(graph
        .into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect())
}

/// Find cycles in a directed graph represented as `child → [parent]`.
///
/// Returns a `Vec<Vec<Iri>>` — one inner vec per cycle, in walk order
/// starting at the cycle's entry point. v1 emits *one* cycle per
/// strongly-connected component (the first discovered); the
/// user typically resolves cycles one at a time and re-attempts the
/// merge, so enumerating all cycles up front is wasted work.
fn find_cycles(graph: &BTreeMap<Iri, Vec<Iri>>) -> Vec<Vec<Iri>> {
    let mut cycles: Vec<Vec<Iri>> = Vec::new();
    let mut visited: BTreeSet<Iri> = BTreeSet::new();
    let mut in_stack: BTreeSet<Iri> = BTreeSet::new();
    let mut stack: Vec<Iri> = Vec::new();

    for start in graph.keys() {
        if !visited.contains(start) {
            dfs_cycle(
                start,
                graph,
                &mut visited,
                &mut in_stack,
                &mut stack,
                &mut cycles,
            );
        }
    }
    cycles
}

fn dfs_cycle(
    node: &Iri,
    graph: &BTreeMap<Iri, Vec<Iri>>,
    visited: &mut BTreeSet<Iri>,
    in_stack: &mut BTreeSet<Iri>,
    stack: &mut Vec<Iri>,
    cycles: &mut Vec<Vec<Iri>>,
) {
    visited.insert(node.clone());
    in_stack.insert(node.clone());
    stack.push(node.clone());

    if let Some(parents) = graph.get(node) {
        for parent in parents {
            if !visited.contains(parent) {
                dfs_cycle(parent, graph, visited, in_stack, stack, cycles);
            } else if in_stack.contains(parent) {
                // Found a back-edge — extract the cycle from `stack`
                // starting at `parent`.
                if let Some(cycle_start) = stack.iter().position(|n| n == parent) {
                    let cycle: Vec<Iri> = stack[cycle_start..].to_vec();
                    // Avoid recording the same cycle twice via
                    // different DFS entry points. The lexicographic
                    // minimum IRI normalises rotation; the cycle is
                    // stored starting at that minimum.
                    let normalised = normalise_cycle(&cycle);
                    if !cycles.contains(&normalised) {
                        cycles.push(normalised);
                    }
                }
            }
        }
    }

    in_stack.remove(node);
    stack.pop();
}

/// Rotate a cycle so it starts at its lexicographically smallest IRI.
/// This canonicalises cycle representation: two DFS walks that find
/// the "same" cycle at different rotations produce the same vec.
fn normalise_cycle(cycle: &[Iri]) -> Vec<Iri> {
    if cycle.is_empty() {
        return Vec::new();
    }
    let (min_idx, _) = cycle
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| a.cmp(b))
        .expect("non-empty cycle");
    let mut rotated = Vec::with_capacity(cycle.len());
    rotated.extend_from_slice(&cycle[min_idx..]);
    rotated.extend_from_slice(&cycle[..min_idx]);
    rotated
}

// ─── Outcome ───────────────────────────────────────────────────────────────

/// Outcome of a merge attempt, taxonomy from D20 §7.1.
///
/// This is the kernel-internal shape; the lattice's `MergeCheck` /
/// `UpdateOutcome` continue to carry the Phase 14e wire shape for
/// backward compatibility. 15g promotes `NeedsResolution` to the wire
/// once the rest of the resolution machinery is in place.
#[derive(Debug, Clone)]
pub enum MergeOutcome {
    /// The merge succeeded — no conflicts surfaced (either disjoint
    /// IRIs, or every shared IRI's disagreement was monotonically safe
    /// under open-world semantics).
    Merged { merge_layer: LayerId },
    /// Conflicts require user-supplied resolutions. The caller submits
    /// `MergeResolution`s targeting each `ConflictId` and re-attempts
    /// via `merge_with_resolutions`.
    NeedsResolution {
        conflicts: Vec<TypedConflict>,
        /// Identifier for the would-be-merged chain. v1 encodes the
        /// `(head_a, head_b)` pair as a deterministic string; future
        /// versions might persist the candidate-chain shape.
        candidate_chain: String,
    },
}

/// Classify the conflicts in a span and produce a typed report
/// suitable for the resolution submission protocol.
///
/// Empty `Vec<TypedConflict>` means the merge can proceed without
/// user resolution — the per-IRI classifier accepts every shared IRI
/// under Eigon's open-world semantics, and stage-2 walkers find no
/// structural problems. Non-empty means at least one conflict needs
/// an explicit resolution.
pub fn classify_conflicts(
    span: &MergeSpan,
    backend: &dyn PersistentBackend,
) -> Result<Vec<TypedConflict>, StorageError> {
    // Load the topology once and thread it through every per-IRI and
    // graph-level walker that needs to traverse the ancestor's
    // parent chain. Avoids reloading from the backend per call site;
    // the topology is bounded by layer count, not graph content
    // size, so this is cheap.
    let topology = backend.load_topology()?;
    let mut conflicts: Vec<TypedConflict> = Vec::new();

    // Stage 1 + 3 — per-IRI classifier over every shared IRI.
    for iri in span.shared_iris() {
        if let Some(kind) = classify_iri_disagreement(span, &iri, &topology, backend)? {
            let id = match &kind {
                ConflictKind::PropertyDataType { property, .. } => {
                    ConflictId::from_iri("property_data_type", property)
                }
                ConflictKind::KindMismatch { iri, .. } => {
                    ConflictId::from_iri("kind_mismatch", iri)
                }
                ConflictKind::IriCollision { iri, .. } => {
                    ConflictId::from_iri("iri_collision", iri)
                }
                ConflictKind::DeletionConflict { iri, .. } => ConflictId::from_iri("deletion", iri),
                // Stage-2 kinds don't surface from the per-IRI
                // classifier; they emerge from the graph walker below.
                ConflictKind::InheritanceCycle { cycle } => {
                    ConflictId::from_iris("inheritance_cycle", cycle)
                }
                ConflictKind::DisjointnessViolation {
                    class_a, class_b, ..
                } => ConflictId(format!("disjointness:{class_a}:{class_b}")),
                ConflictKind::PathEquationContradiction { .. } => {
                    ConflictId(format!("path_equation:{iri}"))
                }
            };
            conflicts.push(TypedConflict { id, kind });
        }
    }

    // Stage 2 — graph-level equation walker. v1 only emits
    // `InheritanceCycle`; `DisjointnessViolation` and
    // `PathEquationContradiction` are reserved.
    for kind in detect_inheritance_cycles(span, &topology, backend)? {
        let id = match &kind {
            ConflictKind::InheritanceCycle { cycle } => {
                ConflictId::from_iris("inheritance_cycle", cycle)
            }
            _ => unreachable!("detect_inheritance_cycles emits only InheritanceCycle"),
        };
        conflicts.push(TypedConflict { id, kind });
    }

    Ok(conflicts)
}

/// Short discriminator string for a ConflictKind, used in typed
/// errors so clients can branch on the conflict shape without
/// pattern-matching the full enum.
pub(crate) fn conflict_kind_discriminator(kind: &ConflictKind) -> &'static str {
    match kind {
        ConflictKind::PropertyDataType { .. } => "PropertyDataType",
        ConflictKind::KindMismatch { .. } => "KindMismatch",
        ConflictKind::IriCollision { .. } => "IriCollision",
        ConflictKind::DeletionConflict { .. } => "DeletionConflict",
        ConflictKind::InheritanceCycle { .. } => "InheritanceCycle",
        ConflictKind::DisjointnessViolation { .. } => "DisjointnessViolation",
        ConflictKind::PathEquationContradiction { .. } => "PathEquationContradiction",
    }
}

// Used by future entry points that build `MergeSpan`s from raw
// branch tips. 15a doesn't expose a public constructor; callers
// supply spans constructed via `lattice::iri_sources_since`.
#[allow(dead_code)]
fn _topology_marker(_topology: &LayerTopology) {
    let _ = collect_iri_refs_into;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::merge::test_support::{build_span, iri, make_resource};
    use crate::layer::{LayerBuilder, LayerStorage};
    use crate::ontology::resource::Value;
    use crate::storage::memory::MemoryPersistentBackend;
    use std::sync::Arc;

    #[test]
    fn disjoint_branches_produce_no_conflicts() {
        // Branch A adds class X; branch B adds class Y. No overlap.
        // The pushout-of-trivial-span invariant: empty conflicts +
        // a successful merge outcome.
        let (span, backend) = build_span(
            Vec::new(),
            vec![make_resource("urn:test:X", &[wk::CLASS], &[])],
            vec![make_resource("urn:test:Y", &[wk::CLASS], &[])],
        );
        let conflicts = classify_conflicts(&span, &backend).unwrap();
        assert!(
            conflicts.is_empty(),
            "disjoint contributions must produce no conflicts; got {conflicts:?}"
        );
    }

    #[test]
    fn structural_body_equality_is_not_a_conflict() {
        // Both branches independently committed the same class body
        // at the same IRI. Under open-world semantics this is a
        // monotonically safe "merge to either" — no conflict needed.
        let class_x = make_resource("urn:test:X", &[wk::CLASS], &[]);
        let (span, backend) = build_span(Vec::new(), vec![class_x.clone()], vec![class_x.clone()]);
        let conflicts = classify_conflicts(&span, &backend).unwrap();
        assert!(
            conflicts.is_empty(),
            "equal bodies on both sides must NOT surface as IriCollision; got {conflicts:?}"
        );
    }

    #[test]
    fn property_data_type_disagreement_classified_as_stage_one() {
        // Branch A adds Property `weight` with data_type=integer;
        // branch B adds the same IRI with data_type=string. Different
        // single-valued primitive type — canonical stage-1 conflict.
        let prop_a = make_resource(
            "urn:test:weight",
            &[wk::PROPERTY],
            &[(wk::DATA_TYPE, Value::ResourceRef(iri(wk::INTEGER)))],
        );
        let prop_b = make_resource(
            "urn:test:weight",
            &[wk::PROPERTY],
            &[(wk::DATA_TYPE, Value::ResourceRef(iri(wk::STRING)))],
        );
        let (span, backend) = build_span(Vec::new(), vec![prop_a], vec![prop_b]);
        let conflicts = classify_conflicts(&span, &backend).unwrap();
        assert_eq!(
            conflicts.len(),
            1,
            "expected one conflict, got {conflicts:?}"
        );
        match &conflicts[0].kind {
            ConflictKind::PropertyDataType {
                property,
                branch_a,
                branch_b,
                ancestor,
            } => {
                assert_eq!(property.as_str(), "urn:test:weight");
                assert_eq!(branch_a.as_str(), wk::INTEGER);
                assert_eq!(branch_b.as_str(), wk::STRING);
                assert!(
                    ancestor.is_none(),
                    "property was branch-introduced, no ancestor value"
                );
            }
            other => panic!("expected PropertyDataType, got {other:?}"),
        }
    }

    #[test]
    fn kind_mismatch_class_vs_property() {
        // Same IRI declared as Class on A and Property on B. Kind is
        // single-valued per D1 §3 — no monotonic union exists.
        let class_x = make_resource("urn:test:X", &[wk::CLASS], &[]);
        let prop_x = make_resource("urn:test:X", &[wk::PROPERTY], &[]);
        let (span, backend) = build_span(Vec::new(), vec![class_x], vec![prop_x]);
        let conflicts = classify_conflicts(&span, &backend).unwrap();
        assert_eq!(conflicts.len(), 1);
        match &conflicts[0].kind {
            ConflictKind::KindMismatch {
                iri: i,
                branch_a_kind,
                branch_b_kind,
            } => {
                assert_eq!(i.as_str(), "urn:test:X");
                assert_eq!(*branch_a_kind, ResourceKind::Class);
                assert_eq!(*branch_b_kind, ResourceKind::Property);
            }
            other => panic!("expected KindMismatch, got {other:?}"),
        }
    }

    #[test]
    fn iri_collision_surfaces_when_bodies_differ() {
        // Both branches modified the same Resource (not Class /
        // Property) with different property values. Falls through
        // stages 1+2 and surfaces as a stage-3 IriCollision.
        let body_a = make_resource(
            "urn:test:patient_42",
            &["urn:test:Patient"],
            &[("urn:test:weight", Value::Integer(75))],
        );
        let body_b = make_resource(
            "urn:test:patient_42",
            &["urn:test:Patient"],
            &[("urn:test:weight", Value::Integer(76))],
        );
        let (span, backend) = build_span(Vec::new(), vec![body_a], vec![body_b]);
        let conflicts = classify_conflicts(&span, &backend).unwrap();
        assert_eq!(conflicts.len(), 1);
        match &conflicts[0].kind {
            ConflictKind::IriCollision {
                iri: i,
                ancestor_body,
                ..
            } => {
                assert_eq!(i.as_str(), "urn:test:patient_42");
                assert!(ancestor_body.is_none());
            }
            other => panic!("expected IriCollision, got {other:?}"),
        }
    }

    #[test]
    fn subclass_additions_are_monotonically_safe() {
        // Open-world invariant: branch A adds Dog `subclass_of` Mammal,
        // branch B adds Dog `subclass_of` Canine. The merged class
        // has both parents — no cycle, no kind mismatch, no body
        // collision (the bodies differ only in subclass_of, which is
        // multi-valued and combines monotonically).
        //
        // Today's classifier flags this as IriCollision because the
        // bodies aren't structurally equal. That's CORRECT for the
        // current stage — the cascade analysis in 15f decides whether
        // the user wants to KeepBoth, and the resolution-application
        // path in 15d folds the arrows into the merge. This test pins
        // the current shape so 15d's plumbing knows what classifier
        // output to expect.
        let dog_a = make_resource(
            "urn:test:Dog",
            &[wk::CLASS],
            &[(
                wk::PARENT_CLASSES,
                Value::Array(vec![Value::ResourceRef(iri("urn:test:Mammal"))]),
            )],
        );
        let dog_b = make_resource(
            "urn:test:Dog",
            &[wk::CLASS],
            &[(
                wk::PARENT_CLASSES,
                Value::Array(vec![Value::ResourceRef(iri("urn:test:Canine"))]),
            )],
        );
        let (span, backend) = build_span(Vec::new(), vec![dog_a], vec![dog_b]);
        let conflicts = classify_conflicts(&span, &backend).unwrap();
        // We expect ONE conflict (the IriCollision on the Dog body) —
        // 15d will resolve it via SchemaQuotient::KeepBoth.
        assert_eq!(
            conflicts.len(),
            1,
            "subclass-additions still surface as IriCollision (15d folds via KeepBoth); got {conflicts:?}"
        );
        assert!(matches!(
            conflicts[0].kind,
            ConflictKind::IriCollision { .. }
        ));
    }

    #[test]
    fn inheritance_cycle_detected_when_branches_combine() {
        // Branch A adds Dog `subclass_of` Mammal; branch B adds Mammal
        // `subclass_of` Dog. Neither branch alone has a cycle; the
        // merged subclass graph does. Canonical stage-2 conflict.
        let dog = make_resource(
            "urn:test:Dog",
            &[wk::CLASS],
            &[(
                wk::PARENT_CLASSES,
                Value::Array(vec![Value::ResourceRef(iri("urn:test:Mammal"))]),
            )],
        );
        let mammal = make_resource(
            "urn:test:Mammal",
            &[wk::CLASS],
            &[(
                wk::PARENT_CLASSES,
                Value::Array(vec![Value::ResourceRef(iri("urn:test:Dog"))]),
            )],
        );
        // Pre-existing ancestor declarations are required so the IRIs
        // exist before each branch redeclares them. (Without that
        // the classifier flags the IRIs as fresh additions on each
        // side and the cycle detector still finds them — but having
        // the ancestor structure mirrors realistic workflows.)
        let ancestor_dog = make_resource("urn:test:Dog", &[wk::CLASS], &[]);
        let ancestor_mammal = make_resource("urn:test:Mammal", &[wk::CLASS], &[]);
        let (span, backend) =
            build_span(vec![ancestor_dog, ancestor_mammal], vec![dog], vec![mammal]);
        let topology = backend.load_topology().unwrap();
        let cycles = detect_inheritance_cycles(&span, &topology, &backend).unwrap();
        assert_eq!(
            cycles.len(),
            1,
            "expected exactly one cycle; got {cycles:?}"
        );
        match &cycles[0] {
            ConflictKind::InheritanceCycle { cycle } => {
                assert_eq!(cycle.len(), 2, "cycle should be Dog→Mammal→Dog (2 nodes)");
                // Canonicalised — starts at the lexicographically
                // smallest IRI ("Dog" < "Mammal").
                assert_eq!(cycle[0].as_str(), "urn:test:Dog");
                assert_eq!(cycle[1].as_str(), "urn:test:Mammal");
            }
            other => panic!("expected InheritanceCycle, got {other:?}"),
        }
    }

    #[test]
    fn iter_iri_values_recurses_through_nested_arrays_and_embedded() {
        // Pure unit test of the helper — no chain involved. Pins the
        // contract that nested containers are walked, not silently
        // ignored. Adjacent to the cycle-detector use today; will be
        // load-bearing for 15f's cascade walker over arbitrary
        // property values.
        let inner = make_resource(
            "urn:test:inner",
            &[wk::CLASS],
            &[(
                wk::PARENT_CLASSES,
                Value::ResourceRef(iri("urn:test:DeepParent")),
            )],
        );
        let value = Value::Array(vec![
            Value::ResourceRef(iri("urn:test:DirectRef")),
            // Nested array — recursion into Array.
            Value::Array(vec![Value::ResourceRef(iri("urn:test:NestedRef"))]),
            // Embedded — recursion into the embedded resource's
            // property values yields BOTH `is_a` class refs AND
            // any IRI refs in its other properties.
            Value::Embedded(Box::new(inner)),
            // Scalars produce nothing.
            Value::String("not-an-iri".into()),
            Value::Integer(42),
        ]);

        let collected = iter_iri_values(&value);
        // Expected IRIs, in walk order:
        //   - DirectRef (top-level)
        //   - NestedRef (through nested array)
        //   - CLASS (from inner.is_a)
        //   - DeepParent (from inner.subclass_of)
        let collected_strs: Vec<&str> = collected.iter().map(|i| i.as_str()).collect();
        assert!(
            collected_strs.contains(&"urn:test:DirectRef"),
            "missing top-level ref; got {collected_strs:?}"
        );
        assert!(
            collected_strs.contains(&"urn:test:NestedRef"),
            "nested array not recursed; got {collected_strs:?}"
        );
        assert!(
            collected_strs.contains(&wk::CLASS),
            "embedded resource's is_a not walked; got {collected_strs:?}"
        );
        assert!(
            collected_strs.contains(&"urn:test:DeepParent"),
            "embedded resource's property values not walked; got {collected_strs:?}"
        );
    }

    #[test]
    fn property_data_type_walks_ancestor_chain_for_inherited_definitions() {
        // The chain shape:
        //
        //   root  (Property X declared with data_type: integer)
        //     │
        //   mid  (unrelated commit; does NOT redeclare X)
        //    ├── branch_a (Property X redeclared with data_type: string)
        //    └── branch_b (Property X redeclared with data_type: boolean)
        //
        // LCA = mid. X is defined at root (mid's parent). The
        // classifier must walk mid → root to find the ancestor's
        // value — a flat `try_load_resource(mid, X)` would miss it
        // and report `ancestor: None`, which is the gap this test
        // pins.
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();

        // root layer: declares Property X with data_type: integer.
        let mut root_b = LayerBuilder::new("root", None);
        root_b
            .add_resource(make_resource(
                "urn:test:weight",
                &[wk::PROPERTY],
                &[(wk::DATA_TYPE, Value::ResourceRef(iri(wk::INTEGER)))],
            ))
            .unwrap();
        let root = Arc::new(root_b.build(storage.clone()));
        backend.store_layer(&root).unwrap();

        // mid layer: an unrelated commit. Does not touch X.
        let mut mid_b = LayerBuilder::new("mid", Some(Arc::clone(&root)));
        mid_b
            .add_resource(make_resource("urn:test:Marker", &[wk::CLASS], &[]))
            .unwrap();
        let mid = Arc::new(mid_b.build(storage.clone()));
        backend.store_layer(&mid).unwrap();

        // branch_a: redeclares X with data_type: string.
        let mut a_b = LayerBuilder::new("branch_a", Some(Arc::clone(&mid)));
        a_b.add_resource(make_resource(
            "urn:test:weight",
            &[wk::PROPERTY],
            &[(wk::DATA_TYPE, Value::ResourceRef(iri(wk::STRING)))],
        ))
        .unwrap();
        let head_a = Arc::new(a_b.build(storage.clone()));
        backend.store_layer(&head_a).unwrap();

        // branch_b: redeclares X with data_type: boolean.
        let mut b_b = LayerBuilder::new("branch_b", Some(Arc::clone(&mid)));
        b_b.add_resource(make_resource(
            "urn:test:weight",
            &[wk::PROPERTY],
            &[(wk::DATA_TYPE, Value::ResourceRef(iri(wk::BOOLEAN)))],
        ))
        .unwrap();
        let head_b = Arc::new(b_b.build(storage));
        backend.store_layer(&head_b).unwrap();

        let topology = backend.load_topology().unwrap();
        let sources_a =
            crate::lattice::iri_sources_since(head_a.id(), mid.id(), &topology, &backend).unwrap();
        let sources_b =
            crate::lattice::iri_sources_since(head_b.id(), mid.id(), &topology, &backend).unwrap();

        let span = MergeSpan {
            ancestor: mid.id().clone(),
            head_a: head_a.id().clone(),
            head_b: head_b.id().clone(),
            sources_a,
            sources_b,
        };

        let conflicts = classify_conflicts(&span, &backend).unwrap();
        assert_eq!(
            conflicts.len(),
            1,
            "expected one PropertyDataType conflict; got {conflicts:?}"
        );
        match &conflicts[0].kind {
            ConflictKind::PropertyDataType {
                branch_a,
                branch_b,
                ancestor,
                ..
            } => {
                assert_eq!(branch_a.as_str(), wk::STRING);
                assert_eq!(branch_b.as_str(), wk::BOOLEAN);
                assert_eq!(
                    ancestor.as_ref().map(|i| i.as_str()),
                    Some(wk::INTEGER),
                    "ancestor's data_type should be resolved through mid → root chain, not None"
                );
            }
            other => panic!("expected PropertyDataType, got {other:?}"),
        }
    }

    #[test]
    fn build_merge_span_reconstructs_ancestor_and_sources_from_heads() {
        // Two diverged branches off a common ancestor. Calling
        // `build_merge_span` should produce a span identical (modulo
        // ordering) to the hand-assembled one from `build_span`.
        let ancestor_class = make_resource("urn:test:Base", &[wk::CLASS], &[]);
        let (hand_span, backend) = build_span(
            vec![ancestor_class],
            vec![make_resource("urn:test:A", &[wk::CLASS], &[])],
            vec![make_resource("urn:test:B", &[wk::CLASS], &[])],
        );
        let topology = backend.load_topology().unwrap();

        let span = build_merge_span(&hand_span.head_a, &hand_span.head_b, &topology, &backend)
            .expect("build_merge_span should succeed on a connected DAG");
        assert_eq!(span.ancestor, hand_span.ancestor);
        assert_eq!(span.head_a, hand_span.head_a);
        assert_eq!(span.head_b, hand_span.head_b);
        assert_eq!(span.sources_a, hand_span.sources_a);
        assert_eq!(span.sources_b, hand_span.sources_b);
    }

    #[test]
    fn build_merge_span_same_head_produces_empty_sources() {
        // Head merged with itself — LCA is the head; both sources
        // maps are empty (nothing changed since "ancestor").
        let (hand_span, backend) = build_span(
            vec![make_resource("urn:test:Base", &[wk::CLASS], &[])],
            vec![make_resource("urn:test:A", &[wk::CLASS], &[])],
            Vec::new(),
        );
        let topology = backend.load_topology().unwrap();

        let span = build_merge_span(&hand_span.head_a, &hand_span.head_a, &topology, &backend)
            .expect("same-head merge should succeed");
        assert_eq!(span.ancestor, hand_span.head_a);
        assert!(span.sources_a.is_empty());
        assert!(span.sources_b.is_empty());
    }

    #[test]
    fn build_merge_span_unrelated_roots_surface_no_common_ancestor() {
        // Two independently-rooted DAGs share no ancestor. v1's LCA
        // walker returns None, surfacing as `NoCommonAncestor`.
        let backend = MemoryPersistentBackend::new();
        let storage = crate::layer::LayerStorage::in_memory();

        let mut ab = LayerBuilder::new("root_a", None);
        ab.add_resource(make_resource("urn:test:RootA", &[wk::CLASS], &[]))
            .unwrap();
        let root_a = Arc::new(ab.build(storage.clone()));
        backend.store_layer(&root_a).unwrap();

        let mut bb = LayerBuilder::new("root_b", None);
        bb.add_resource(make_resource("urn:test:RootB", &[wk::CLASS], &[]))
            .unwrap();
        let root_b = Arc::new(bb.build(storage));
        backend.store_layer(&root_b).unwrap();

        let topology = backend.load_topology().unwrap();
        let result = build_merge_span(root_a.id(), root_b.id(), &topology, &backend);
        match result {
            Err(MergeError::NoCommonAncestor { head_a, head_b }) => {
                assert_eq!(&head_a, root_a.id());
                assert_eq!(&head_b, root_b.id());
            }
            other => panic!("expected NoCommonAncestor, got {other:?}"),
        }
    }

    #[test]
    fn build_merge_span_head_not_in_topology_surfaces_no_common_ancestor() {
        // Head id that doesn't exist in the topology — find_lca
        // returns None, surface as NoCommonAncestor.
        let (hand_span, backend) = build_span(
            vec![make_resource("urn:test:Base", &[wk::CLASS], &[])],
            vec![make_resource("urn:test:A", &[wk::CLASS], &[])],
            Vec::new(),
        );
        let topology = backend.load_topology().unwrap();
        let bogus = LayerId([0xCC; 32]);
        let result = build_merge_span(&hand_span.head_a, &bogus, &topology, &backend);
        match result {
            Err(MergeError::NoCommonAncestor { head_b, .. }) => {
                assert_eq!(head_b, bogus);
            }
            other => panic!("expected NoCommonAncestor, got {other:?}"),
        }
    }
}
