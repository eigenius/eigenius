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

//! Layer reconciliation — Phase 15 / D20.
//!
//! Two branches that diverge with overlapping IRI contributions form a
//! *span* `(ancestor → branch_a, ancestor → branch_b)`. The merge is the
//! pushout of that span; resolutions transform the span before the
//! pushout is taken (D20 §6).
//!
//! This module exists alongside [`crate::lattice::merge_independent_heads`]
//! — the pre-Phase-15 primitive that handles the trivial-merge fast
//! path (disjoint-IRI contributions, no resolution needed). When IRIs
//! overlap, lattice's `MergeCheck::Conflict { conflicting_iris }` is
//! the flat-list stub; this module is the typed-conflict surface that
//! replaces it.
//!
//! Two design calls anchor this code (decided 2026-05-13):
//!
//! - **Operate on Eigon resources directly.** D20 §4 frames the merge
//!   as a pushout in **Cat** + Σ pushforward in `[C_merged, Set]`, but
//!   we don't introduce a separate `CategoryPresentation` data
//!   structure — Eigon resources *are* the presentation, and the
//!   pushout reduces to "decide each shared IRI's body in the merged
//!   layer, then validate the result." The category-theoretic
//!   vocabulary motivates the design; it does not dictate an API.
//!
//! - **Open-world semantics narrows the conflict taxonomy.** D20 §5
//!   listed nine `SchemaConflict`/`EquationConflict`/`InstanceConflict`
//!   variants. Under Eigon's open-world reading, most of those
//!   collapse: `is_a` / `subclass_of` / `class_types` / `requires` /
//!   `recommends` additions are monotonically safe (the merged
//!   ontology stays valid; existing instances either keep satisfying
//!   the merged constraints or surface as cascade items for ack in
//!   15f). The genuinely structural cases are:
//!   - **Stage 1 — schema-shape:** `PropertyDataType` (single-valued
//!     primitive type disagrees), `KindMismatch` (same IRI declared
//!     as Class on one branch and Property on the other).
//!   - **Stage 2 — equation-closure:** `InheritanceCycle` (the merged
//!     `subclass_of` graph has a cycle that didn't exist in either
//!     branch). `DisjointnessViolation` and `PathEquationContradiction`
//!     keep their enum slots for forward compatibility but don't fire
//!     in v1 (Eigon has no disjointness declarations today; the
//!     "contradiction" cases are subsumed by `KindMismatch`).
//!   - **Stage 3 — instance:** `IriCollision` (same IRI, materially
//!     different resource bodies), `DeletionConflict` (one branch
//!     tombstoned an IRI the other modified).
//!
//! Sub-milestone 15a (this commit) lands the typed-conflict scaffolding
//! and the classifier. 15b–15e add the six resolution strategies;
//! 15f layers cascade impact analysis on top; 15g plumbs the surface
//! to gRPC + CLI + notebook.

// 15a is internal scaffolding: every exported type below is consumed
// in 15b–15g (Witness application, classifier wiring into lattice,
// proto surface). Without this allow, clippy's "never used" pass
// flags the whole module under `-D warnings`. The allow shrinks as
// each downstream milestone wires up its callers.
#![allow(dead_code)]

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
    fn from_iri(discriminator: &str, iri: &Iri) -> Self {
        Self(format!("{discriminator}:{iri}"))
    }

    /// Construct from a list of IRIs (for cycle-shaped conflicts).
    fn from_iris(discriminator: &str, iris: &[Iri]) -> Self {
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

/// Walk the parent chain rooted at `head` looking for the topmost
/// layer that defines `iri`. Returns the layer id + the resource it
/// found, or `None` if no layer in the chain defines the IRI.
///
/// BFS over `LayerHandle.parents` so the shallowest (topmost-in-the-
/// chain) layer wins on multi-parent merges. Visited set prevents
/// re-entry on diamonds. Storage errors abort the walk and
/// propagate up.
fn find_iri_in_chain(
    head: &LayerId,
    iri: &Iri,
    topology: &LayerTopology,
    backend: &dyn PersistentBackend,
) -> Result<Option<(LayerId, Resource)>, StorageError> {
    use std::collections::VecDeque;
    let mut visited: BTreeSet<LayerId> = BTreeSet::new();
    let mut queue: VecDeque<LayerId> = VecDeque::new();
    queue.push_back(head.clone());
    while let Some(id) = queue.pop_front() {
        if !visited.insert(id.clone()) {
            continue;
        }
        if let Some(resource) = backend.try_load_resource(&id, iri)? {
            return Ok(Some((id, resource)));
        }
        if let Some(handle) = topology.get_layer(&id) {
            for parent in &handle.parents {
                if !visited.contains(parent) {
                    queue.push_back(parent.clone());
                }
            }
        }
    }
    Ok(None)
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

/// Yield every `Iri` referenced by a `Value`, recursing through
/// every nested container shape Eigon admits:
///
/// - `ResourceRef(iri)` — yield the IRI directly.
/// - `Array(items)` — recurse into each item (handles arrays of
///   refs, arrays of arrays, arrays of embeddeds).
/// - `Embedded(resource)` — recurse into each of the embedded
///   resource's property values; the embedded resource itself has
///   no `@id`, but its property values can mention any number of
///   IRIs (including its `is_a` class refs).
/// - Scalars (string / integer / float / boolean) and `Json` —
///   yield nothing.
///
/// Generic helper, not bound to any particular property. Callers
/// that want a specific subset (e.g., only `subclass_of` parents)
/// pass the property value directly; well-formed Eigon values for
/// most property shapes are flat arrays of refs, so recursion is a
/// no-op there and a safety net for the malformed-value cases the
/// classifier shouldn't silently ignore.
fn iter_iri_values(value: &crate::ontology::resource::Value) -> Vec<Iri> {
    let mut out = Vec::new();
    collect_iri_refs_into(value, &mut out);
    out
}

fn collect_iri_refs_into(value: &crate::ontology::resource::Value, out: &mut Vec<Iri>) {
    use crate::ontology::resource::Value;
    match value {
        Value::ResourceRef(iri) => out.push(iri.clone()),
        Value::Array(items) => {
            for v in items {
                collect_iri_refs_into(v, out);
            }
        }
        Value::Embedded(resource) => {
            for v in resource.properties().values() {
                collect_iri_refs_into(v, out);
            }
        }
        Value::String(_)
        | Value::Integer(_)
        | Value::Float(_)
        | Value::Boolean(_)
        | Value::Json(_) => {}
    }
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

// ─── Resolution surface ───────────────────────────────────────────────────

/// User-supplied resolution for a specific conflict.
///
/// Each variant transforms the merge span before the pushout is
/// taken (D20 §6). 15b ships `Witness`; 15c–15e land the rest.
#[derive(Debug, Clone, PartialEq)]
pub enum MergeResolution {
    /// Apply a `MergeComorphism` whose `merge_transformation` Component
    /// realises the universal arrow at the conflicting IRI. The
    /// transformation must have type `(A, A, Option<A>) → A` where
    /// `A` is the class of the conflict's IRI (D20 §6.1). The kernel
    /// resolves the comorphism IRI on the chain, validates the
    /// resource shape at submission time, and applies the
    /// transformation to produce the merged value.
    Witness {
        conflict: ConflictId,
        /// IRI of a `MergeComorphism` resource committed earlier in
        /// the chain. Must resolve through the ancestor's parent
        /// chain (or either branch's contributions).
        comorphism: Iri,
    },
    /// Apply an isomorphism functor renaming `old_iri` → `new_iri` on
    /// one side of the span before the pushout (D20 §6.2). The kernel
    /// (a) checks `new_iri` doesn't collide with anything else in the
    /// chain (the other branch's contributions, the ancestor's parent
    /// chain, the renamed branch's *other* contributions), and (b)
    /// rewrites every reference to `old_iri` within the renamed
    /// branch's slice so the rename is consistent. Useful for
    /// accidental IRI collisions — two teams independently choosing
    /// the same local name for genuinely different concepts.
    Rename {
        conflict: ConflictId,
        /// Which side of the span the rename is applied to.
        side: Side,
        /// The current IRI on `side` being renamed.
        old_iri: Iri,
        /// The replacement IRI. Must not collide with any other IRI
        /// in the merge span.
        new_iri: Iri,
    },
    /// Quotient the span at a schema-level conflict (D20 §6.3). Three
    /// flavors: `KeepBoth` admits the freely-combined pushout (only
    /// legal for conflicts where both contributions can coexist —
    /// none of v1's classified kinds qualify), `KeepOne { winner }`
    /// drops the loser's contribution at the conflict point, and
    /// `KeepNeither` collapses both contributions back to the
    /// ancestor's state. The kernel rejects strategies that don't
    /// apply to the conflict kind with a typed `QuotientNotApplicable`
    /// error rather than producing a merged ontology that won't load.
    SchemaQuotient {
        conflict: ConflictId,
        quotient: SchemaQuotient,
    },
    /// Augment the ancestor with new common structure and re-merge
    /// against it (D20 §6.4). The motivating shape: branch A added
    /// `Dog subclass_of Mammal`, branch B added `Dog subclass_of
    /// Reptile`. Restructure introduces a new `Animal` class, makes
    /// `Mammal` and `Reptile` subclass it, and the previously
    /// conflicting `Dog` class subclasses `Animal` only —
    /// sidestepping the original conflict by raising the
    /// abstraction. The kernel rejects synthesized parent IRIs (no
    /// `urn:eigenius:auto:*`); the user must name the new structure
    /// explicitly so the merged schema stays readable.
    Restructure {
        conflict: ConflictId,
        spec: RestructureSpec,
    },
    //
    // Each variant lands with its own sub-milestone; the enum grows
    // monotonically so callers built against one variant stay
    // working as the others light up.
}

/// The structural inputs to a `Restructure` resolution (D20 §6.4).
///
/// Kept as a sub-struct rather than inlined into the variant because
/// the resolution carries five logically-related fields and the apply
/// function threads them as a unit; bundling keeps the call surface
/// readable and the variant constructor terse.
#[derive(Debug, Clone, PartialEq)]
pub struct RestructureSpec {
    /// IRI of the class whose contradictory `subclass_of` arrows
    /// motivated the restructure. The kernel uses this both for
    /// downstream cascade analysis (15f) and for the
    /// `affected_class_under_new` toggle below.
    pub affected_class: Iri,
    /// Existing or new IRI for the parent class to introduce.
    pub new_parent: Iri,
    /// If `new_parent` is new (not yet in any layer of the span),
    /// its full `Class` resource definition. If `new_parent` already
    /// exists, must be `None` — supplying a definition for an
    /// existing IRI is a redeclaration that the apply path refuses
    /// to attempt.
    pub new_parent_def: Option<Resource>,
    /// Existing classes that should now subclass `new_parent`. Each
    /// IRI must resolve through the span. Empty is legal — the user
    /// may want a structural placeholder without immediate
    /// subclasses (e.g., creating `Animal` first, then letting
    /// follow-up commits attach `Mammal`/`Reptile`).
    pub classes_under_new: Vec<Iri>,
    /// Whether the conflicting class itself goes under `new_parent`.
    /// In the motivating example (`Dog`-under-`Mammal` vs
    /// `Dog`-under-`Reptile`), this is `true`.
    pub affected_class_under_new: bool,
}

/// Three ways to quotient a span at a schema-level conflict (D20 §6.3).
///
/// Applicability is conflict-kind-dependent and enforced by the kernel
/// at submission time — see [`apply_quotient_resolution`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// Variant names share `Keep*` by design — D20 §6.3 names the three
// strategies "KeepBoth", "KeepOne", "KeepNeither" and we mirror that
// vocabulary verbatim so resolution-surface clients can map UI labels
// to enum variants directly.
#[allow(clippy::enum_variant_names)]
pub enum SchemaQuotient {
    /// Accept the freely-combined pushout. Only legal when the conflict
    /// kind admits both sides' contributions structurally (Eigon's
    /// multi-class membership for `subclass_of` would qualify; none of
    /// v1's classified kinds do, because every kind we currently
    /// surface is single-valued or mutually-exclusive). Submitting
    /// `KeepBoth` against a current conflict kind always fails with
    /// `QuotientNotApplicable`; the variant is reserved for future
    /// taxonomies.
    KeepBoth,
    /// Quotient out the loser's contribution at the conflict point.
    /// Every arrow the loser added is dropped from the merge; the
    /// cascade analysis (15f) flags everything downstream that
    /// referenced it.
    KeepOne {
        /// Which side wins. The opposite side's contribution at the
        /// conflict point is dropped.
        winner: Side,
    },
    /// Collapse both contributions back to the ancestor's state.
    /// IRIs the ancestor didn't have are dropped entirely; IRIs the
    /// ancestor had keep the ancestor's body.
    KeepNeither,
}

/// A resolved `MergeComorphism` ready for application.
///
/// Produced by [`resolve_merge_comorphism`] at submission time;
/// carries everything the application path needs without re-walking
/// the chain. The actual term evaluation (applying
/// `merge_transformation` to the three branch values) lives in 15b
/// Step 2 — Step 1 only validates the resource shape.
#[derive(Debug, Clone)]
pub struct MergeComorphismHandle {
    /// The MergeComorphism resource's own IRI.
    pub iri: Iri,
    /// The layer where the resource was found. Useful for diagnostics
    /// when a resolution fails at application time.
    pub source_layer: LayerId,
    /// The IRI of the Mini-TT term realising the universal arrow.
    /// Resolves to a resource committed earlier in the chain whose
    /// `is_a` includes one of the Mini-TT expression classes
    /// (`program:Lambda`, etc.).
    pub transformation: Iri,
}

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

/// Attempt a merge with user-supplied resolutions.
///
/// Two distinct phases:
///
/// 1. **Classification.** Always runs first. Empty conflicts +
///    empty resolutions = clean merge (placeholder layer id for
///    now; lattice owns the actual layer-build path until 15g).
///    Non-empty conflicts + empty resolutions = `NeedsResolution`
///    surface for the client to fill in.
///
/// 2. **Resolution application.** Runs when `resolutions` is
///    non-empty. Each resolution is dispatched on its variant:
///    - `Witness` (15b): looks up the `MergeComorphism`, validates
///      the resource shape, and (currently) returns
///      `WitnessApplicationNotYetWired` because the evaluator
///      integration that produces the merged resource body is the
///      15b Step 2 deliverable.
///    - Other variants land with their respective sub-milestones.
///
/// On any resolution error the function fails the whole merge;
/// partial applications are not surfaced.
pub fn merge_with_resolutions(
    span: &MergeSpan,
    resolutions: Vec<MergeResolution>,
    backend: &dyn PersistentBackend,
) -> Result<MergeOutcome, MergeError> {
    let topology = backend.load_topology().map_err(MergeError::Storage)?;
    let conflicts = classify_conflicts(span, backend).map_err(MergeError::Storage)?;

    if resolutions.is_empty() {
        return if conflicts.is_empty() {
            // No structural conflicts — the merge proceeds. 15a
            // doesn't build the merge layer itself; that path stays
            // in `lattice::merge_independent_heads` for now. The
            // skeleton returns a placeholder layer id so callers
            // wire through the shape; the lattice layer's existing
            // path is the load-bearing producer.
            Ok(MergeOutcome::Merged {
                // Placeholder: lattice owns merge-layer construction;
                // 15g unifies the entry points.
                merge_layer: span.head_a.clone(),
            })
        } else {
            Ok(MergeOutcome::NeedsResolution {
                conflicts,
                candidate_chain: format!("{}+{}", span.head_a, span.head_b),
            })
        };
    }

    // Dispatch each resolution. Errors abort the whole merge so a
    // partial application never leaves the chain in an intermediate
    // state. Conflict-lookup table built once for O(1) targeting.
    let conflict_by_id: BTreeMap<&ConflictId, &TypedConflict> =
        conflicts.iter().map(|c| (&c.id, c)).collect();
    // 15b Step 1 short-circuits on the first Witness with
    // `WitnessApplicationNotYetWired` because the term evaluator
    // isn't wired yet; Step 2 turns this into a real loop that
    // accumulates resolved-and-applied state across resolutions.
    // The for-loop shape stays so Step 2 is a localised in-place
    // edit rather than a restructuring.
    #[allow(clippy::never_loop)]
    for resolution in &resolutions {
        match resolution {
            MergeResolution::Witness {
                conflict,
                comorphism,
            } => {
                let target = conflict_by_id
                    .get(conflict)
                    .ok_or_else(|| MergeError::ConflictNotFound(conflict.clone()))?;
                let _handle = resolve_merge_comorphism(comorphism, span, &topology, backend)?;
                // The handle is validated; Step 2 will pass it into
                // the term-evaluator + produce the merged resource
                // body for the conflict's IRI. Until that lands,
                // surface a typed "validated but not applied" error
                // rather than silently no-oping.
                let _ = target;
                return Err(MergeError::WitnessApplicationNotYetWired {
                    comorphism: comorphism.clone(),
                });
            }
            MergeResolution::Rename {
                conflict,
                side,
                old_iri,
                new_iri,
            } => {
                let target = conflict_by_id
                    .get(conflict)
                    .ok_or_else(|| MergeError::ConflictNotFound(conflict.clone()))?;
                let _ = target;
                // Validate the rename — collisions, missing target.
                // Application produces a renamed slice of the chosen
                // branch's contributions; merge-layer construction
                // (running the merge against the renamed branch and
                // committing the result) is 15g.
                let _renamed =
                    apply_rename_resolution(span, *side, old_iri, new_iri, &topology, backend)?;
                return Err(MergeError::RenameApplicationNotYetWired {
                    old_iri: old_iri.clone(),
                    new_iri: new_iri.clone(),
                });
            }
            MergeResolution::SchemaQuotient { conflict, quotient } => {
                let target = conflict_by_id
                    .get(conflict)
                    .ok_or_else(|| MergeError::ConflictNotFound(conflict.clone()))?;
                // Validate applicability + compute the per-side drop
                // sets. Actually applying them (rebuilding the merge
                // layer with the dropped contributions excluded) is 15g.
                let _application = apply_quotient_resolution(target, *quotient)?;
                return Err(MergeError::QuotientApplicationNotYetWired {
                    conflict_id: conflict.clone(),
                    quotient: *quotient,
                });
            }
            MergeResolution::Restructure { conflict, spec } => {
                let target = conflict_by_id
                    .get(conflict)
                    .ok_or_else(|| MergeError::ConflictNotFound(conflict.clone()))?;
                let _ = target;
                // Validate the restructure shape — synthesized
                // parent rejection, definition presence rules,
                // class-in-span checks. Building the augmented
                // ancestor + re-merging against it is 15g.
                let _application =
                    apply_restructure_resolution(conflict, spec, span, &topology, backend)?;
                return Err(MergeError::RestructureApplicationNotYetWired {
                    conflict_id: conflict.clone(),
                    new_parent: spec.new_parent.clone(),
                });
            }
        }
    }

    // Unreachable today — every `MergeResolution` variant either
    // returns Ok or Err inside the match. Kept as a defensive
    // fallthrough for when 15c–15e land their variants and one
    // accidentally falls through without producing an outcome.
    Err(MergeError::WitnessApplicationNotYetWired {
        comorphism: Iri::parse("urn:eigenius:placeholder:internal").expect("placeholder IRI"),
    })
}

/// Resolve a `MergeComorphism` IRI against the span and validate its
/// resource shape per the core ontology's class declaration:
///
/// 1. Walk every layer that could plausibly carry the comorphism
///    (each branch's contributions, then the ancestor's parent
///    chain) — D20 §6.1 says witnesses are "committed earlier in
///    the chain," which under the partial-order chain means
///    visible from the merge span's ancestor.
/// 2. Confirm the resource's `is_a` includes
///    `urn:eigenius:core:MergeComorphism`.
/// 3. Extract the `merge_transformation` property value; reject if
///    missing or if it isn't a `ResourceRef`.
///
/// On success, returns a [`MergeComorphismHandle`] the application
/// path consumes. On any structural failure, returns the matching
/// typed [`MergeError`] variant so callers can render a useful
/// message without parsing.
pub fn resolve_merge_comorphism(
    iri: &Iri,
    span: &MergeSpan,
    topology: &LayerTopology,
    backend: &dyn PersistentBackend,
) -> Result<MergeComorphismHandle, MergeError> {
    let merge_comorphism_iri = Iri::parse(wk::MERGE_COMORPHISM).expect("MERGE_COMORPHISM IRI");
    let merge_transformation_iri =
        Iri::parse(wk::MERGE_TRANSFORMATION).expect("MERGE_TRANSFORMATION IRI");

    // Search both branches' contributions before falling back to the
    // ancestor's chain. v1 doesn't require the comorphism to live
    // strictly under the ancestor — D20 §6.1 leaves the chain
    // location open as long as the resource is reachable from the
    // merge span. A comorphism committed on a branch is just as
    // valid as one on the ancestor.
    let resource_loc = find_in_span_chain(iri, span, topology, backend)
        .map_err(MergeError::Storage)?
        .ok_or_else(|| MergeError::MergeComorphismNotFound(iri.clone()))?;
    let (source_layer, resource) = resource_loc;

    if !resource.is_instance_of(&merge_comorphism_iri) {
        let is_a_iri = Iri::parse(wk::IS_A).expect("IS_A IRI");
        let found_classes: Vec<Iri> = resource
            .get(&is_a_iri)
            .map(iter_iri_values)
            .unwrap_or_default();
        return Err(MergeError::NotAMergeComorphism {
            iri: iri.clone(),
            found_classes,
        });
    }

    let transformation = match resource.get(&merge_transformation_iri) {
        Some(crate::ontology::resource::Value::ResourceRef(t)) => t.clone(),
        Some(_) => {
            return Err(MergeError::MalformedMergeComorphism {
                iri: iri.clone(),
                reason: "merge_transformation must be a ResourceRef to a Mini-TT term".to_string(),
            });
        }
        None => {
            return Err(MergeError::MalformedMergeComorphism {
                iri: iri.clone(),
                reason: "merge_transformation property is required".to_string(),
            });
        }
    };

    Ok(MergeComorphismHandle {
        iri: iri.clone(),
        source_layer,
        transformation,
    })
}

/// Walk the merge span looking for an IRI's definition. Searches
/// each branch's contributions first (those are the most-recent
/// commits and most-likely places for a freshly-committed witness)
/// before falling back to the ancestor's parent chain.
///
/// Returns `Some((layer_id, resource))` for the topmost layer that
/// defines `iri`; `None` if the IRI isn't reachable from any of the
/// span's heads. Storage errors propagate.
fn find_in_span_chain(
    iri: &Iri,
    span: &MergeSpan,
    topology: &LayerTopology,
    backend: &dyn PersistentBackend,
) -> Result<Option<(LayerId, Resource)>, StorageError> {
    if let Some(layer) = span.sources_a.get(iri) {
        if let Some(resource) = backend.try_load_resource(layer, iri)? {
            return Ok(Some((layer.clone(), resource)));
        }
    }
    if let Some(layer) = span.sources_b.get(iri) {
        if let Some(resource) = backend.try_load_resource(layer, iri)? {
            return Ok(Some((layer.clone(), resource)));
        }
    }
    find_iri_in_chain(&span.ancestor, iri, topology, backend)
}

// ─── Witness application (15b Step 2) ──────────────────────────────────────

/// Apply a validated `MergeComorphism` witness to a triple of
/// `(branch_a, branch_b, ancestor)` and produce the merged resource
/// body. Implements D20 §6.1's `(A, A, Option<A>) → A` signature
/// discipline end-to-end: type-check first, then evaluate.
///
/// Pipeline:
///  1. Build an in-memory chain for `handle.source_layer` so the
///     parser + evaluator can resolve references through it.
///  2. Look up the transformation Resource (the Mini-TT term the
///     comorphism points at). The lookup walks the chain — the
///     transformation may live at the witness's source layer, an
///     ancestor, or any layer reachable from the witness.
///  3. Parse the Resource into a Mini-TT `Exp` via
///     [`crate::program::expr::parse_expression`].
///  4. Build the expected witness type
///     `Π_:A. Π_:A. Π_:Option(A). A` and bidirectionally check the
///     parsed term against it. A type mismatch surfaces as
///     `WitnessTypeMismatch` and aborts before evaluation — the spec
///     mandates a commit-time signature check.
///  5. Evaluate in Pure mode (a merge witness must be deterministic +
///     side-effect-free — no IO, no chain mutation).
///  6. Apply the resulting function value to three arguments, each
///     wrapped as the appropriate `Val`:
///      - `branch_a` → `Val::ResourceVal(branch_a)`
///      - `branch_b` → `Val::ResourceVal(branch_b)`
///      - `ancestor` → `none A` or `some A r` as an `InductiveVal`
///        on [`crate::nbe::term::option_decl`].
///  7. Marshal the result back to an Eigon `Resource` via
///     [`crate::nbe::eval::val_to_resource_value`] — the inverse of
///     the `ResourceVal` wrap.
pub fn apply_witness_resolution(
    handle: &MergeComorphismHandle,
    class: &Iri,
    branch_a: Resource,
    branch_b: Resource,
    ancestor: Option<Resource>,
    storage: crate::layer::LayerStorage,
    backend: &dyn PersistentBackend,
) -> Result<Resource, MergeError> {
    use crate::nbe::check::{check, CheckCtx};
    use crate::nbe::env::Rho;
    use crate::nbe::eval::{eval_ctx, val_to_resource_value, EvalCtx};
    use crate::nbe::term::{option_decl, Exp, Patt};
    use crate::nbe::val::Val;
    use crate::ontology::resource::Value;
    use std::sync::Arc;

    // 1. Rebuild the witness's source layer's chain in memory.
    //    `parse_expression` walks references through this layer; the
    //    transformation IRI must be visible from it.
    let chain_info = backend
        .load_chain_from(&handle.source_layer)
        .map_err(MergeError::Storage)?
        .ok_or_else(|| {
            MergeError::Storage(StorageError::NotFound(format!(
                "witness source layer {} not in store",
                handle.source_layer
            )))
        })?;
    let layer = crate::layer::build_chain(chain_info, storage);

    // 2. Resolve the transformation IRI through the chain.
    let transformation_resource = layer.resolve(&handle.transformation).ok_or_else(|| {
        MergeError::TransformationNotFound {
            comorphism: handle.iri.clone(),
            transformation: handle.transformation.clone(),
        }
    })?;

    // 3. Parse the Resource into a Mini-TT Exp.
    let exp = crate::program::expr::parse_expression(&transformation_resource, &layer).map_err(
        |reason| MergeError::TransformationParseError {
            transformation: handle.transformation.clone(),
            reason,
        },
    )?;

    // 4. Build the expected type `Π_:A. Π_:A. Π_:Option(A). A` and
    //    type-check the witness term against it. Building as an `Exp`
    //    and evaluating in `Rho::Nil` keeps construction uniform with
    //    how the rest of the kernel produces Pi-chain Vals.
    let a_exp = Exp::EigonClass(class.clone());
    let option_a_exp = Exp::InductiveType(option_decl(), vec![a_exp.clone()]);
    let expected_exp = Exp::Pi(
        Patt::Unit,
        Box::new(a_exp.clone()),
        Box::new(Exp::Pi(
            Patt::Unit,
            Box::new(a_exp.clone()),
            Box::new(Exp::Pi(Patt::Unit, Box::new(option_a_exp), Box::new(a_exp))),
        )),
    );
    let mut check_ctx = CheckCtx::with_layer(Rho::Nil, Vec::new(), Arc::clone(&layer));
    let expected_val =
        check_ctx
            .eval(&expected_exp, &Rho::Nil)
            .map_err(|e| MergeError::WitnessTypeMismatch {
                transformation: handle.transformation.clone(),
                expected: "Π_:A. Π_:A. Π_:Option(A). A".to_string(),
                reason: format!("failed to build expected type: {e}"),
            })?;
    check(&mut check_ctx, &exp, &expected_val).map_err(|reason| {
        MergeError::WitnessTypeMismatch {
            transformation: handle.transformation.clone(),
            expected: format!("Π_:{class}. Π_:{class}. Π_:Option({class}). {class}"),
            reason,
        }
    })?;

    // 5. Evaluate in Pure mode — merge witnesses can't do IO.
    let ctx = EvalCtx::Pure;
    let term_val =
        eval_ctx(&exp, &Rho::Nil, &ctx).map_err(|e| MergeError::TransformationEvalError {
            transformation: handle.transformation.clone(),
            reason: e.to_string(),
        })?;

    // 6. Wrap each argument and apply. The transformation is
    //    `λ a. λ b. λ opt. ...` — three curried applications fold the
    //    merged value out. The ancestor lifts to `none A` or `some A r`
    //    on the canonical `option_decl()` so the witness can pattern-
    //    match it via Mini-TT's standard inductive elimination.
    let a_val = Val::EigonClass(class.clone());
    let val_a = Val::ResourceVal(Box::new(branch_a));
    let val_b = Val::ResourceVal(Box::new(branch_b));
    let val_opt = match ancestor {
        None => Val::InductiveVal {
            decl: option_decl(),
            ctor_name: "none".to_string(),
            args: vec![a_val.clone()],
        },
        Some(r) => Val::InductiveVal {
            decl: option_decl(),
            ctor_name: "some".to_string(),
            args: vec![a_val, Val::ResourceVal(Box::new(r))],
        },
    };
    let after_a = term_val
        .clone()
        .app_ctx(val_a, &ctx)
        .map_err(|e| witness_app_error(&handle.transformation, &term_val, e))?;
    let after_b = after_a
        .clone()
        .app_ctx(val_b, &ctx)
        .map_err(|e| witness_app_error(&handle.transformation, &after_a, e))?;
    let merged_val = after_b
        .clone()
        .app_ctx(val_opt, &ctx)
        .map_err(|e| witness_app_error(&handle.transformation, &after_b, e))?;

    // 7. Marshal back. `val_to_resource_value` returns a `Value`;
    //    the merge surface needs a `Resource`. `Embedded(box)` unwraps
    //    directly. Other shapes (scalars, refs) are wrapped into a
    //    fresh embedded Resource so callers always get a Resource —
    //    a single-string return is the CompleteText shortcut path
    //    `val_to_resource_value` produces for one-property resources.
    let result_value = val_to_resource_value(&merged_val);
    let merged_resource = match result_value {
        Value::Embedded(boxed) => *boxed,
        other => {
            // Wrap the scalar/ref into an embedded Resource so the
            // surface is uniform. The marshalling path's "extract
            // single property" shortcut is undone here.
            let mut wrapper = Resource::new_embedded();
            wrapper.set(
                Iri::parse("urn:eigenius:merge:result").expect("merge result IRI"),
                other,
            );
            wrapper
        }
    };

    Ok(merged_resource)
}

/// Translate an `EvalError` raised during witness application into a
/// typed `MergeError`. Distinguishes "the term wasn't a function" from
/// other evaluation failures so the caller can render a focused
/// diagnostic.
fn witness_app_error(
    transformation: &Iri,
    failing_val: &crate::nbe::val::Val,
    err: crate::nbe::eval::EvalError,
) -> MergeError {
    use crate::nbe::eval::EvalError;
    match err {
        EvalError::NotAFunction(_) => MergeError::WitnessTermNotAFunction {
            transformation: transformation.clone(),
            found: format!("{failing_val:?}"),
        },
        other => MergeError::TransformationEvalError {
            transformation: transformation.clone(),
            reason: other.to_string(),
        },
    }
}

// ─── Rename application (15c) ──────────────────────────────────────────────

/// The renamed slice of one branch's contributions, ready for the
/// pushout to be re-taken against. Produced by
/// [`apply_rename_resolution`] after validation.
///
/// `resources` is keyed by the *new* IRI — every resource that used to
/// live at `old_iri` (or referenced it) has been rewritten. Other
/// resources in the branch's slice that don't touch `old_iri` aren't
/// re-emitted here; the merge-layer construction path (15g) folds this
/// slice into the rest of the branch's contributions when committing.
#[derive(Debug, Clone, PartialEq)]
pub struct RenameApplication {
    /// Which side the rename was applied to.
    pub side: Side,
    /// The renamed-from IRI. Kept for diagnostics + cascade analysis
    /// (the cascade walker needs both to enumerate downstream effects).
    pub old_iri: Iri,
    /// The renamed-to IRI.
    pub new_iri: Iri,
    /// The transformed resources, keyed by their post-rename IRI. The
    /// target itself is keyed by `new_iri`; other resources on the
    /// branch that referenced `old_iri` are keyed by their own
    /// (unchanged) IRIs with their bodies rewritten.
    pub resources: BTreeMap<Iri, Resource>,
}

/// Validate and apply a `Rename` resolution against a `MergeSpan`.
///
/// Pipeline:
///  1. Verify `old_iri` is actually a contribution of the renamed
///     side. A rename targeting an IRI the side never touched is a
///     client-side error — there's nothing to transform.
///  2. Verify `new_iri` doesn't collide with anything else visible
///     from the span: the *other* branch's contributions, the
///     ancestor's parent chain, or the renamed branch's *own* other
///     contributions. A collision means the rename would silently
///     merge into another resource at the new IRI; reject it.
///  3. Walk the renamed branch's contributions, rewriting every
///     occurrence of `old_iri` (in `@id`, `ResourceRef`, nested
///     `Embedded` resources, and `Array` items) to `new_iri`.
///
/// Returns a [`RenameApplication`] carrying the transformed
/// resources. The actual merge-layer commit (running the merge
/// against the renamed branch) is 15g.
pub fn apply_rename_resolution(
    span: &MergeSpan,
    side: Side,
    old_iri: &Iri,
    new_iri: &Iri,
    topology: &LayerTopology,
    backend: &dyn PersistentBackend,
) -> Result<RenameApplication, MergeError> {
    if old_iri == new_iri {
        return Err(MergeError::RenameIdentity {
            iri: old_iri.clone(),
        });
    }

    let (this_sources, other_sources) = match side {
        Side::A => (&span.sources_a, &span.sources_b),
        Side::B => (&span.sources_b, &span.sources_a),
    };

    // 1. `old_iri` must be a contribution of `side`.
    if !this_sources.contains_key(old_iri) {
        return Err(MergeError::RenameTargetNotInBranch {
            old_iri: old_iri.clone(),
            side,
        });
    }

    // 2a. Collision against `side`'s other contributions. A rename to
    //     an IRI the same branch already touches would silently merge
    //     two resources into one.
    if this_sources.contains_key(new_iri) {
        return Err(MergeError::RenameCollision {
            new_iri: new_iri.clone(),
            location: RenameCollisionSite::SameBranch(side),
        });
    }

    // 2b. Collision against the *other* branch's contributions —
    //     renames don't dodge real conflicts by introducing new ones
    //     (D20 §6.2).
    if other_sources.contains_key(new_iri) {
        let other_side = match side {
            Side::A => Side::B,
            Side::B => Side::A,
        };
        return Err(MergeError::RenameCollision {
            new_iri: new_iri.clone(),
            location: RenameCollisionSite::OtherBranch(other_side),
        });
    }

    // 2c. Collision against the ancestor's parent chain.
    if find_iri_in_chain(&span.ancestor, new_iri, topology, backend)
        .map_err(MergeError::Storage)?
        .is_some()
    {
        return Err(MergeError::RenameCollision {
            new_iri: new_iri.clone(),
            location: RenameCollisionSite::AncestorChain,
        });
    }

    // 3. Walk this side's contributions, transforming every resource
    //    that mentions `old_iri`. Resources keyed at `old_iri` itself
    //    are re-keyed under `new_iri`; resources that *reference*
    //    `old_iri` from elsewhere are kept under their own keys with
    //    bodies rewritten.
    let mut resources: BTreeMap<Iri, Resource> = BTreeMap::new();
    for (iri, layer_id) in this_sources {
        let resource = backend
            .try_load_resource(layer_id, iri)
            .map_err(MergeError::Storage)?
            .ok_or_else(|| {
                MergeError::Storage(StorageError::NotFound(format!(
                    "rename: contribution {iri} not loadable from {layer_id}"
                )))
            })?;
        let mentions_old = resource_mentions_iri(&resource, old_iri);
        let is_target = iri == old_iri;
        if !mentions_old && !is_target {
            continue;
        }
        let renamed = substitute_iri_in_resource(&resource, old_iri, new_iri);
        let key = if is_target {
            new_iri.clone()
        } else {
            iri.clone()
        };
        resources.insert(key, renamed);
    }

    Ok(RenameApplication {
        side,
        old_iri: old_iri.clone(),
        new_iri: new_iri.clone(),
        resources,
    })
}

/// Indicates where the renamed-to IRI was found to clash.
///
/// Used inside [`MergeError::RenameCollision`]; lets the resolution UI
/// label the conflict source ("already on the other branch", "already
/// in the ancestor chain") without a stringly-typed reason field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameCollisionSite {
    /// The renamed branch already declares `new_iri` itself.
    SameBranch(Side),
    /// The opposite branch already declares `new_iri`.
    OtherBranch(Side),
    /// Some ancestor in the parent chain already declares `new_iri`.
    AncestorChain,
}

impl std::fmt::Display for RenameCollisionSite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenameCollisionSite::SameBranch(side) => {
                write!(f, "same branch ({side:?})")
            }
            RenameCollisionSite::OtherBranch(side) => {
                write!(f, "other branch ({side:?})")
            }
            RenameCollisionSite::AncestorChain => write!(f, "ancestor chain"),
        }
    }
}

/// Whether a `Resource`'s body (excluding its own `@id`) contains any
/// reference to `iri`. Walks `ResourceRef`, `Embedded`, and `Array`
/// recursively — same traversal shape as [`iter_iri_values`] but with
/// an early-exit predicate.
fn resource_mentions_iri(resource: &Resource, iri: &Iri) -> bool {
    resource
        .properties()
        .values()
        .any(|v| value_mentions_iri(v, iri))
}

fn value_mentions_iri(value: &crate::ontology::resource::Value, iri: &Iri) -> bool {
    use crate::ontology::resource::Value;
    match value {
        Value::ResourceRef(r) => r == iri,
        Value::Array(items) => items.iter().any(|v| value_mentions_iri(v, iri)),
        Value::Embedded(resource) => resource_mentions_iri(resource, iri),
        _ => false,
    }
}

/// Produce a copy of `resource` with every reference to `old_iri`
/// (in `@id`, `ResourceRef`, nested `Embedded`, and `Array` items)
/// rewritten to `new_iri`. The shape mirrors [`iter_iri_values`] /
/// [`collect_iri_refs_into`] but maps values instead of collecting.
fn substitute_iri_in_resource(resource: &Resource, old_iri: &Iri, new_iri: &Iri) -> Resource {
    let mut out = match resource.id() {
        Some(id) if id == old_iri => Resource::new(new_iri.clone()),
        Some(id) => Resource::new(id.clone()),
        None => Resource::new_embedded(),
    };
    for (prop, value) in resource.properties() {
        out.set(
            prop.clone(),
            substitute_iri_in_value(value, old_iri, new_iri),
        );
    }
    out
}

fn substitute_iri_in_value(
    value: &crate::ontology::resource::Value,
    old_iri: &Iri,
    new_iri: &Iri,
) -> crate::ontology::resource::Value {
    use crate::ontology::resource::Value;
    match value {
        Value::ResourceRef(r) if r == old_iri => Value::ResourceRef(new_iri.clone()),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| substitute_iri_in_value(v, old_iri, new_iri))
                .collect(),
        ),
        Value::Embedded(resource) => Value::Embedded(Box::new(substitute_iri_in_resource(
            resource, old_iri, new_iri,
        ))),
        other => other.clone(),
    }
}

// ─── Schema-quotient application (15d) ─────────────────────────────────────

/// The drop-set produced by a `SchemaQuotient` resolution, ready for
/// the merge-layer construction path (15g) to apply.
///
/// `drop_from_branch_a` / `drop_from_branch_b` enumerate the IRIs each
/// branch's contribution should be excluded for at the conflict point.
/// `KeepBoth` produces empty sets (and is only legal when the kernel
/// finds the conflict kind admits it — see [`SchemaQuotient::KeepBoth`]
/// docs for why no current kind qualifies). `KeepOne { winner: A }`
/// drops the conflict's IRIs from branch B; `KeepNeither` drops them
/// from both branches.
#[derive(Debug, Clone, PartialEq)]
pub struct QuotientApplication {
    pub conflict_id: ConflictId,
    pub quotient: SchemaQuotient,
    /// IRIs whose branch-A contribution is dropped from the merge.
    pub drop_from_branch_a: Vec<Iri>,
    /// IRIs whose branch-B contribution is dropped from the merge.
    pub drop_from_branch_b: Vec<Iri>,
}

/// Validate and apply a `SchemaQuotient` resolution against a
/// pre-resolved conflict.
///
/// Checks the quotient is applicable to the conflict's kind and
/// produces the per-side drop sets. Callers in the merge surface
/// (e.g., [`merge_with_resolutions`]) resolve `ConflictId →
/// &TypedConflict` once via `classify_conflicts` and thread the
/// resolved conflict in here, avoiding a second classify pass. The
/// actual merge-layer commit (combining the drop sets with the rest
/// of the contributions to build the merged chain) is 15g.
///
/// **Applicability table** (D20 §6.3):
///
/// | Conflict kind             | `KeepBoth` | `KeepOne` | `KeepNeither` |
/// |---------------------------|------------|-----------|---------------|
/// | `PropertyDataType`        | ✗          | ✓         | ✓             |
/// | `KindMismatch`            | ✗          | ✓         | ✓             |
/// | `IriCollision`            | ✗          | ✓         | ✓             |
/// | `InheritanceCycle`        | ✗          | ✓         | ✓             |
/// | `DeletionConflict`        | ✗          | ✓         | ✓             |
/// | `DisjointnessViolation`   | ✗          | ✓         | ✓             |
/// | `PathEquationContradiction` | ✗        | ✓         | ✓             |
///
/// `KeepBoth` is never applicable to v1's classified kinds — every
/// kind currently surfaced is single-valued or mutually-exclusive.
/// It stays in the enum for forward compat with conflict taxonomies
/// that admit additive quotients (e.g., subclass-membership conflicts,
/// which open-world classification already treats as monotonically
/// safe and therefore doesn't surface).
pub fn apply_quotient_resolution(
    conflict: &TypedConflict,
    quotient: SchemaQuotient,
) -> Result<QuotientApplication, MergeError> {
    let conflict_iris = quotient_target_iris(&conflict.kind);

    let (drop_from_branch_a, drop_from_branch_b) = match quotient {
        SchemaQuotient::KeepBoth => {
            // No current kind admits KeepBoth — every classified kind
            // is single-valued or mutually-exclusive. Surface as a
            // typed error rather than producing a no-op application.
            return Err(MergeError::QuotientNotApplicable {
                conflict_id: conflict.id.clone(),
                conflict_kind: conflict_kind_discriminator(&conflict.kind).to_string(),
                quotient,
                reason: "KeepBoth requires a conflict kind that admits both contributions structurally; no v1 classified kind qualifies".to_string(),
            });
        }
        SchemaQuotient::KeepOne { winner } => match winner {
            Side::A => (Vec::new(), conflict_iris),
            Side::B => (conflict_iris, Vec::new()),
        },
        SchemaQuotient::KeepNeither => (conflict_iris.clone(), conflict_iris),
    };

    Ok(QuotientApplication {
        conflict_id: conflict.id.clone(),
        quotient,
        drop_from_branch_a,
        drop_from_branch_b,
    })
}

/// Enumerate the IRIs a quotient drops for the given conflict kind.
///
/// Single-IRI kinds (`PropertyDataType`, `KindMismatch`, `IriCollision`,
/// `DeletionConflict`) return a single-element vec. `InheritanceCycle`
/// returns every IRI in the cycle — dropping any one of them breaks
/// the cycle, and the user's `KeepOne` choice means "drop the loser's
/// edges in the cycle"; we conservatively drop all cycle-participating
/// IRIs on the loser side (cascade analysis surfaces what was actually
/// affected). Reserved kinds return their structural IRIs.
fn quotient_target_iris(kind: &ConflictKind) -> Vec<Iri> {
    match kind {
        ConflictKind::PropertyDataType { property, .. } => vec![property.clone()],
        ConflictKind::KindMismatch { iri, .. } => vec![iri.clone()],
        ConflictKind::IriCollision { iri, .. } => vec![iri.clone()],
        ConflictKind::DeletionConflict { iri, .. } => vec![iri.clone()],
        ConflictKind::InheritanceCycle { cycle } => cycle.clone(),
        ConflictKind::DisjointnessViolation {
            class_a,
            class_b,
            offending_iris,
        } => {
            let mut out = Vec::with_capacity(2 + offending_iris.len());
            out.push(class_a.clone());
            out.push(class_b.clone());
            out.extend(offending_iris.iter().cloned());
            out
        }
        ConflictKind::PathEquationContradiction { .. } => Vec::new(),
    }
}

/// Short discriminator string for a ConflictKind, used in typed
/// errors so clients can branch on the conflict shape without
/// pattern-matching the full enum.
fn conflict_kind_discriminator(kind: &ConflictKind) -> &'static str {
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

// ─── Restructure application (15e) ─────────────────────────────────────────

/// Prefix that flags "synthesized" parent IRIs the kernel refuses for
/// `Restructure` resolutions. D20 §6.4 mandates user-supplied names so
/// the merged schema stays readable; auto-generated parents undermine
/// the structural intent of the resolution.
const SYNTHESIZED_PARENT_PREFIX: &str = "urn:eigenius:auto:";

/// The structural transformation produced by a validated
/// `Restructure` resolution, ready for the 15g merge-layer
/// construction path to commit.
///
/// `new_parent_resource` is `Some` when the user supplied a new
/// `Class` definition (the parent didn't exist anywhere in the span);
/// `None` when the parent already existed and the restructure only
/// re-attaches existing classes to it. `classes_to_reparent` is the
/// set of IRIs that gain `new_parent` in their `parent_classes`.
#[derive(Debug, Clone, PartialEq)]
pub struct RestructureApplication {
    pub conflict_id: ConflictId,
    pub new_parent: Iri,
    /// The new parent Class resource, only `Some` when the user
    /// supplied a `new_parent_def`. Carries the verbatim resource
    /// the user submitted, so the merge-layer construction path
    /// commits it without further transformation.
    pub new_parent_resource: Option<Resource>,
    /// Existing class IRIs that gain `new_parent` in their
    /// `parent_classes`. Includes the affected class iff
    /// `spec.affected_class_under_new`. Iteration order is
    /// deterministic (BTreeSet semantics) so downstream layer
    /// construction stays reproducible.
    pub classes_to_reparent: BTreeSet<Iri>,
}

/// Validate and produce the structural transformation for a
/// `Restructure` resolution (D20 §6.4).
///
/// Checks performed:
/// 1. `new_parent` is not synthesized — D20 §6.4's "the kernel
///    rejects synthesized parents like `urn:eigenius:auto:…`"
///    structural requirement.
/// 2. `new_parent`'s presence in the span and the presence of
///    `new_parent_def` are consistent: if the parent is new (not in
///    any branch nor the ancestor chain), `new_parent_def` must be
///    `Some`; if it exists, `new_parent_def` must be `None`.
/// 3. When supplied, `new_parent_def`'s `@id` matches `new_parent`
///    and its `is_a` declares it a `Class`.
/// 4. `spec.affected_class` and every IRI in `spec.classes_under_new`
///    resolve through the span.
///
/// Returns a [`RestructureApplication`] carrying the new parent
/// resource (if any) and the set of IRIs that gain `new_parent` in
/// their `parent_classes`. The actual merge-layer commit (rebuilding
/// the merge against the augmented ancestor + re-attached subclass
/// arrows) is 15g.
///
/// **Cascade-analysis interaction (15f).** D20 §6.4 also mandates a
/// "subsumed arrow" check: any `subclass_of` arrow the restructure
/// implicitly drops (because transitivity through the new parent
/// covers it) must be surfaced to the user, who explicitly
/// acknowledges the loss. That check lives in cascade analysis (15f)
/// — this apply step produces the structural transformation; the
/// cascade walker reads it and computes the implication.
pub fn apply_restructure_resolution(
    conflict_id: &ConflictId,
    spec: &RestructureSpec,
    span: &MergeSpan,
    topology: &LayerTopology,
    backend: &dyn PersistentBackend,
) -> Result<RestructureApplication, MergeError> {
    // 1. Reject synthesized parent IRIs. D20 §6.4 forbids
    //    `urn:eigenius:auto:*` so the merged schema retains
    //    human-readable names.
    if spec
        .new_parent
        .as_str()
        .starts_with(SYNTHESIZED_PARENT_PREFIX)
    {
        return Err(MergeError::RestructureSynthesizedParent {
            new_parent: spec.new_parent.clone(),
        });
    }

    // 2. Reconcile `new_parent`'s presence with `new_parent_def`.
    let parent_existing = find_in_span_chain(&spec.new_parent, span, topology, backend)
        .map_err(MergeError::Storage)?;
    match (&parent_existing, &spec.new_parent_def) {
        (Some(_), Some(_)) => {
            return Err(MergeError::RestructureParentRedeclaration {
                new_parent: spec.new_parent.clone(),
            });
        }
        (None, None) => {
            return Err(MergeError::RestructureParentMissingDefinition {
                new_parent: spec.new_parent.clone(),
            });
        }
        _ => {}
    }

    // 3. If a definition was supplied, validate it shape-wise.
    if let Some(def) = &spec.new_parent_def {
        match def.id() {
            Some(id) if id == &spec.new_parent => {}
            Some(id) => {
                return Err(MergeError::RestructureParentDefMismatch {
                    new_parent: spec.new_parent.clone(),
                    found: Some(id.clone()),
                });
            }
            None => {
                return Err(MergeError::RestructureParentDefMismatch {
                    new_parent: spec.new_parent.clone(),
                    found: None,
                });
            }
        }
        if !def.is_a().iter().any(|c| c.as_str() == wk::CLASS) {
            return Err(MergeError::RestructureParentDefNotAClass {
                new_parent: spec.new_parent.clone(),
            });
        }
    }

    // 4. Every IRI the restructure re-parents must resolve through
    //    the span — otherwise the merge would dangle subclass arrows
    //    against IRIs that don't exist.
    if find_in_span_chain(&spec.affected_class, span, topology, backend)
        .map_err(MergeError::Storage)?
        .is_none()
    {
        return Err(MergeError::RestructureClassNotInSpan {
            iri: spec.affected_class.clone(),
            role: RestructureMissingRole::AffectedClass,
        });
    }
    for cls in &spec.classes_under_new {
        if find_in_span_chain(cls, span, topology, backend)
            .map_err(MergeError::Storage)?
            .is_none()
        {
            return Err(MergeError::RestructureClassNotInSpan {
                iri: cls.clone(),
                role: RestructureMissingRole::ClassUnderNew,
            });
        }
    }

    // Build the reparent set deterministically. Including the
    // affected class is gated on the explicit toggle so the user
    // can express "introduce Animal as a sibling of Dog under
    // Mammal/Reptile" if they want a non-stretched hierarchy.
    let mut classes_to_reparent: BTreeSet<Iri> = spec.classes_under_new.iter().cloned().collect();
    if spec.affected_class_under_new {
        classes_to_reparent.insert(spec.affected_class.clone());
    }

    Ok(RestructureApplication {
        conflict_id: conflict_id.clone(),
        new_parent: spec.new_parent.clone(),
        new_parent_resource: spec.new_parent_def.clone(),
        classes_to_reparent,
    })
}

/// Which role a missing class IRI was filling in a `Restructure`
/// spec. Used inside [`MergeError::RestructureClassNotInSpan`] so
/// the resolution UI can render "the affected class isn't in the
/// span" differently from "the parent's subclass isn't in the span".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestructureMissingRole {
    AffectedClass,
    ClassUnderNew,
}

impl std::fmt::Display for RestructureMissingRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RestructureMissingRole::AffectedClass => write!(f, "affected_class"),
            RestructureMissingRole::ClassUnderNew => write!(f, "classes_under_new entry"),
        }
    }
}

// ─── Errors ────────────────────────────────────────────────────────────────

/// Errors specific to layer-reconciliation operations. Storage failures
/// propagate through `MergeError::Storage`; other variants are typed
/// kernel-level errors the resolution protocol returns to callers.
#[derive(Debug)]
pub enum MergeError {
    Storage(StorageError),
    /// A resolution targets a `ConflictId` that the classifier's most
    /// recent pass over the span did not surface. Either the
    /// resolution refers to a stale conflict id (the span moved on
    /// since the client read it) or to one the client invented.
    ConflictNotFound(ConflictId),
    /// A `Witness` resolution's `comorphism` IRI doesn't resolve to
    /// any resource in the merge span (neither branch's contributions
    /// nor the ancestor's parent chain). Common causes: the
    /// comorphism wasn't committed before the merge attempt, or its
    /// IRI was typoed.
    MergeComorphismNotFound(Iri),
    /// A `Witness` resolution's `comorphism` IRI resolved to a
    /// resource, but it isn't a `MergeComorphism` (its `is_a` doesn't
    /// include `urn:eigenius:core:MergeComorphism`). The kernel
    /// refuses to apply non-witness resources as witnesses.
    NotAMergeComorphism {
        iri: Iri,
        found_classes: Vec<Iri>,
    },
    /// A `MergeComorphism` resource is missing the required
    /// `merge_transformation` property, or the property's value isn't
    /// a `ResourceRef` to a Mini-TT term. Both shapes are required by
    /// the core ontology's class declaration; surfacing this as a
    /// typed error keeps the failure mode legible.
    MalformedMergeComorphism {
        iri: Iri,
        reason: String,
    },
    /// A `Witness` resolution validated cleanly but the merge-layer
    /// construction path (turning a per-conflict merged value into a
    /// committed layer) isn't yet wired (15g deliverable). The
    /// per-witness application is real — exercise it via
    /// [`apply_witness_resolution`] directly.
    WitnessApplicationNotYetWired {
        comorphism: Iri,
    },
    /// A `MergeComorphism`'s `merge_transformation` points at an IRI
    /// that doesn't resolve in the witness's source layer chain —
    /// the term was either uncommitted or lives in a parallel
    /// branch the merge can't see from here.
    TransformationNotFound {
        comorphism: Iri,
        transformation: Iri,
    },
    /// `parse_expression` failed to convert the transformation
    /// Resource into a Mini-TT `Exp`. The Resource is malformed
    /// against the program ontology — e.g., a Lambda missing its
    /// body, a Var without a binder name. Re-stringifies the parser's
    /// diagnostic for a flat error shape.
    TransformationParseError {
        transformation: Iri,
        reason: String,
    },
    /// The NbE evaluator returned an `EvalError` while applying the
    /// witness. Re-stringified because `EvalError` is not `PartialEq`
    /// and the merge surface wants a flat error shape.
    TransformationEvalError {
        transformation: Iri,
        reason: String,
    },
    /// The transformation evaluated to a non-function value —
    /// applying branch_a to it would fail, so we surface the typing
    /// gap up front instead of letting the evaluator's
    /// `NotAFunction` propagate without context.
    WitnessTermNotAFunction {
        transformation: Iri,
        found: String,
    },
    /// The witness term failed bidirectional type-checking against
    /// the spec signature `(A, A, Option<A>) → A`. Surfaces the
    /// checker's diagnostic verbatim alongside the rendered expected
    /// type so callers can show the witness author what was wrong.
    WitnessTypeMismatch {
        transformation: Iri,
        expected: String,
        reason: String,
    },
    /// A `Rename` resolution targets an IRI that isn't a contribution
    /// of the chosen side. The rename has nothing to transform.
    RenameTargetNotInBranch {
        old_iri: Iri,
        side: Side,
    },
    /// A `Rename` resolution's `new_iri` collides with another IRI
    /// visible from the merge span. Renames don't dodge real
    /// conflicts by introducing new ones (D20 §6.2).
    RenameCollision {
        new_iri: Iri,
        location: RenameCollisionSite,
    },
    /// A `Rename` resolution has `old_iri == new_iri`. The rename is
    /// a no-op; surfacing as a typed error keeps client intent
    /// explicit rather than silently accepting a malformed
    /// resolution.
    RenameIdentity {
        iri: Iri,
    },
    /// A `Rename` resolution validated cleanly and the renamed slice
    /// was produced, but the merge-layer construction path (running
    /// the merge against the renamed branch and committing the
    /// result) isn't yet wired (15g deliverable). Exercise the
    /// transformation directly via [`apply_rename_resolution`].
    RenameApplicationNotYetWired {
        old_iri: Iri,
        new_iri: Iri,
    },
    /// A `SchemaQuotient` resolution selected a strategy the
    /// conflict's kind doesn't admit (e.g., `KeepBoth` on a
    /// `PropertyDataType` conflict — a property can't carry two
    /// primitive types). The kernel refuses to apply incompatible
    /// quotients rather than producing a merged ontology that won't
    /// validate.
    QuotientNotApplicable {
        conflict_id: ConflictId,
        conflict_kind: String,
        quotient: SchemaQuotient,
        reason: String,
    },
    /// A `SchemaQuotient` resolution validated cleanly and the drop
    /// sets were computed, but the merge-layer construction path
    /// (rebuilding the merge with the dropped contributions
    /// excluded) isn't yet wired (15g deliverable). Exercise the
    /// drop-set computation directly via
    /// [`apply_quotient_resolution`].
    QuotientApplicationNotYetWired {
        conflict_id: ConflictId,
        quotient: SchemaQuotient,
    },
    /// A `Restructure` resolution's `new_parent` IRI uses the
    /// reserved `urn:eigenius:auto:` namespace. D20 §6.4 forbids
    /// synthesized parents so the merged schema retains
    /// human-readable names.
    RestructureSynthesizedParent {
        new_parent: Iri,
    },
    /// A `Restructure` resolution supplied a `new_parent_def` for a
    /// parent IRI that already exists in the span. Redeclaration
    /// would silently shadow the existing class; the kernel refuses
    /// to attempt it.
    RestructureParentRedeclaration {
        new_parent: Iri,
    },
    /// A `Restructure` resolution's `new_parent` doesn't exist
    /// anywhere in the span and no `new_parent_def` was supplied —
    /// the merge has nothing to attach the new subclasses to.
    RestructureParentMissingDefinition {
        new_parent: Iri,
    },
    /// A `Restructure` resolution's supplied `new_parent_def` has
    /// an `@id` that doesn't match `new_parent`, or has no `@id` at
    /// all. The definition must be self-consistent.
    RestructureParentDefMismatch {
        new_parent: Iri,
        found: Option<Iri>,
    },
    /// A `Restructure` resolution's supplied `new_parent_def` is
    /// not declared as a `Class`. The new parent must be a Class —
    /// subclass arrows can't target Properties or instances.
    RestructureParentDefNotAClass {
        new_parent: Iri,
    },
    /// A `Restructure` resolution references an IRI (the affected
    /// class or one of `classes_under_new`) that doesn't resolve
    /// anywhere in the span. The merge would dangle subclass
    /// arrows against a non-existent target.
    RestructureClassNotInSpan {
        iri: Iri,
        role: RestructureMissingRole,
    },
    /// A `Restructure` resolution validated cleanly and the
    /// transformation was produced, but the merge-layer construction
    /// path (committing the augmented ancestor + re-merging against
    /// it) isn't yet wired (15g deliverable). Exercise the
    /// transformation directly via [`apply_restructure_resolution`].
    RestructureApplicationNotYetWired {
        conflict_id: ConflictId,
        new_parent: Iri,
    },
}

impl std::fmt::Display for MergeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MergeError::Storage(e) => write!(f, "storage error during merge: {e}"),
            MergeError::ConflictNotFound(id) => {
                write!(f, "resolution targets unknown conflict id: {}", id.0)
            }
            MergeError::MergeComorphismNotFound(iri) => write!(
                f,
                "Witness comorphism IRI not found in the merge span: {iri}"
            ),
            MergeError::NotAMergeComorphism { iri, found_classes } => write!(
                f,
                "Witness comorphism {iri} is not a MergeComorphism (is_a: {found_classes:?})"
            ),
            MergeError::MalformedMergeComorphism { iri, reason } => {
                write!(f, "MergeComorphism {iri} is malformed: {reason}")
            }
            MergeError::WitnessApplicationNotYetWired { comorphism } => write!(
                f,
                "witness {comorphism} applied successfully but merge-layer construction is pending (15g)"
            ),
            MergeError::TransformationNotFound {
                comorphism,
                transformation,
            } => write!(
                f,
                "MergeComorphism {comorphism}'s transformation {transformation} not found in the chain"
            ),
            MergeError::TransformationParseError {
                transformation,
                reason,
            } => write!(
                f,
                "transformation {transformation} failed to parse as a Mini-TT term: {reason}"
            ),
            MergeError::TransformationEvalError {
                transformation,
                reason,
            } => write!(
                f,
                "transformation {transformation} failed during evaluation: {reason}"
            ),
            MergeError::WitnessTermNotAFunction {
                transformation,
                found,
            } => write!(
                f,
                "transformation {transformation} evaluated to a non-function value: {found}"
            ),
            MergeError::WitnessTypeMismatch {
                transformation,
                expected,
                reason,
            } => write!(
                f,
                "transformation {transformation} does not type-check against `{expected}`: {reason}"
            ),
            MergeError::RenameTargetNotInBranch { old_iri, side } => write!(
                f,
                "Rename target {old_iri} is not a contribution of side {side:?}"
            ),
            MergeError::RenameCollision { new_iri, location } => write!(
                f,
                "Rename destination {new_iri} collides with an existing IRI at {location}"
            ),
            MergeError::RenameIdentity { iri } => {
                write!(f, "Rename old_iri == new_iri ({iri}); rename is a no-op")
            }
            MergeError::RenameApplicationNotYetWired { old_iri, new_iri } => write!(
                f,
                "Rename {old_iri} → {new_iri} validated successfully but merge-layer construction is pending (15g)"
            ),
            MergeError::QuotientNotApplicable {
                conflict_id,
                conflict_kind,
                quotient,
                reason,
            } => write!(
                f,
                "SchemaQuotient {quotient:?} not applicable to {conflict_kind} conflict {}: {reason}",
                conflict_id.0
            ),
            MergeError::QuotientApplicationNotYetWired {
                conflict_id,
                quotient,
            } => write!(
                f,
                "SchemaQuotient {quotient:?} on conflict {} validated successfully but merge-layer construction is pending (15g)",
                conflict_id.0
            ),
            MergeError::RestructureSynthesizedParent { new_parent } => write!(
                f,
                "Restructure new_parent {new_parent} uses the reserved `{SYNTHESIZED_PARENT_PREFIX}` namespace; user must name the new structure explicitly (D20 §6.4)"
            ),
            MergeError::RestructureParentRedeclaration { new_parent } => write!(
                f,
                "Restructure new_parent {new_parent} already exists in the span; remove `new_parent_def` to attach to the existing class"
            ),
            MergeError::RestructureParentMissingDefinition { new_parent } => write!(
                f,
                "Restructure new_parent {new_parent} doesn't exist in the span and no `new_parent_def` was supplied"
            ),
            MergeError::RestructureParentDefMismatch { new_parent, found } => match found {
                Some(f_iri) => write!(
                    f,
                    "Restructure new_parent_def's @id {f_iri} doesn't match new_parent {new_parent}"
                ),
                None => write!(
                    f,
                    "Restructure new_parent_def has no @id; must match new_parent {new_parent}"
                ),
            },
            MergeError::RestructureParentDefNotAClass { new_parent } => write!(
                f,
                "Restructure new_parent_def for {new_parent} is not declared as a Class"
            ),
            MergeError::RestructureClassNotInSpan { iri, role } => write!(
                f,
                "Restructure {role} {iri} doesn't resolve anywhere in the merge span"
            ),
            MergeError::RestructureApplicationNotYetWired {
                conflict_id,
                new_parent,
            } => write!(
                f,
                "Restructure on conflict {} (new parent {new_parent}) validated successfully but merge-layer construction is pending (15g)",
                conflict_id.0
            ),
        }
    }
}

impl std::error::Error for MergeError {}

// Used by future entry points that build `MergeSpan`s from raw
// branch tips. 15a doesn't expose a public constructor; callers
// supply spans constructed via `lattice::iri_sources_since`.
#[allow(dead_code)]
fn _topology_marker(_topology: &LayerTopology) {}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::{LayerBuilder, LayerStorage};
    use crate::ontology::resource::Value;
    use crate::storage::memory::MemoryPersistentBackend;
    use std::sync::Arc;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    /// Build a minimal `Resource` with the given is_a + property map.
    fn make_resource(id: &str, is_a: &[&str], props: &[(&str, Value)]) -> Resource {
        let mut r = Resource::new(iri(id));
        let is_a_iri = Iri::parse(wk::IS_A).expect("IS_A IRI");
        let classes: Vec<Value> = is_a.iter().map(|c| Value::ResourceRef(iri(c))).collect();
        r.set(is_a_iri, Value::Array(classes));
        for (k, v) in props {
            r.set(iri(k), v.clone());
        }
        r
    }

    /// Build a span by committing ancestor / head_a / head_b layers
    /// and computing per-head sources via the lattice's existing
    /// `iri_sources_since`. Returns the span plus the backend.
    fn build_span(
        ancestor_resources: Vec<Resource>,
        branch_a_resources: Vec<Resource>,
        branch_b_resources: Vec<Resource>,
    ) -> (MergeSpan, MemoryPersistentBackend) {
        let backend = MemoryPersistentBackend::new();
        let storage = LayerStorage::in_memory();

        let mut ab = LayerBuilder::new("ancestor", None);
        for r in ancestor_resources {
            ab.add_resource(r).unwrap();
        }
        let ancestor = Arc::new(ab.build(storage.clone()));
        backend.store_layer(&ancestor).unwrap();

        let mut a_builder = LayerBuilder::new("branch_a", Some(Arc::clone(&ancestor)));
        for r in branch_a_resources {
            a_builder.add_resource(r).unwrap();
        }
        let head_a = Arc::new(a_builder.build(storage.clone()));
        backend.store_layer(&head_a).unwrap();

        let mut b_builder = LayerBuilder::new("branch_b", Some(Arc::clone(&ancestor)));
        for r in branch_b_resources {
            b_builder.add_resource(r).unwrap();
        }
        let head_b = Arc::new(b_builder.build(storage));
        backend.store_layer(&head_b).unwrap();

        let topology = backend.load_topology().unwrap();
        let sources_a =
            crate::lattice::iri_sources_since(head_a.id(), ancestor.id(), &topology, &backend)
                .unwrap();
        let sources_b =
            crate::lattice::iri_sources_since(head_b.id(), ancestor.id(), &topology, &backend)
                .unwrap();

        let span = MergeSpan {
            ancestor: ancestor.id().clone(),
            head_a: head_a.id().clone(),
            head_b: head_b.id().clone(),
            sources_a,
            sources_b,
        };
        (span, backend)
    }

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
    fn empty_resolutions_with_empty_span_yields_clean_merge() {
        // Sanity baseline: no conflicts + no resolutions = the
        // skeleton placeholder Merged outcome. Pins the 15a path
        // through the new resolution dispatcher.
        let (span, backend) = build_span(Vec::new(), Vec::new(), Vec::new());
        let result = merge_with_resolutions(&span, Vec::new(), &backend);
        match result {
            Ok(MergeOutcome::Merged { .. }) => {}
            other => panic!("expected Merged, got {other:?}"),
        }
    }

    // ─── 15b·1: Witness resolution validation ──────────────────────────

    /// Helper for the four Witness tests: build a span with one
    /// `IriCollision` conflict on `urn:test:patient_42` (so resolutions
    /// have a real target). Optionally commits a `MergeComorphism`
    /// resource on the ancestor side so the chain walk can find it.
    fn build_span_with_iri_collision_and_optional_witness(
        witness: Option<Resource>,
    ) -> (MergeSpan, MemoryPersistentBackend) {
        let ancestor_resources = witness.into_iter().collect();
        build_span(
            ancestor_resources,
            vec![make_resource(
                "urn:test:patient_42",
                &["urn:test:Patient"],
                &[("urn:test:weight", Value::Integer(75))],
            )],
            vec![make_resource(
                "urn:test:patient_42",
                &["urn:test:Patient"],
                &[("urn:test:weight", Value::Integer(76))],
            )],
        )
    }

    /// Make a well-formed MergeComorphism resource pointing at a
    /// (placeholder) transformation IRI. Step 1 only validates the
    /// resource shape; the transformation IRI doesn't need to point
    /// at a real Mini-TT term until Step 2 wires the evaluator.
    fn make_merge_comorphism(iri: &str, transformation: &str) -> Resource {
        make_resource(
            iri,
            &[wk::MERGE_COMORPHISM],
            &[(
                wk::MERGE_TRANSFORMATION,
                Value::ResourceRef(Iri::parse(transformation).unwrap()),
            )],
        )
    }

    #[test]
    fn witness_with_unknown_conflict_id_is_rejected() {
        // Resolution targets a conflict id the classifier didn't
        // surface — common cause: stale read against the span. Must
        // produce a typed `ConflictNotFound` rather than silently
        // succeeding or panicking.
        let (span, backend) = build_span_with_iri_collision_and_optional_witness(Some(
            make_merge_comorphism("urn:test:witness", "urn:test:term_placeholder"),
        ));
        let result = merge_with_resolutions(
            &span,
            vec![MergeResolution::Witness {
                conflict: ConflictId("does_not_exist".to_string()),
                comorphism: iri("urn:test:witness"),
            }],
            &backend,
        );
        match result {
            Err(MergeError::ConflictNotFound(id)) => {
                assert_eq!(id.0, "does_not_exist");
            }
            other => panic!("expected ConflictNotFound, got {other:?}"),
        }
    }

    #[test]
    fn witness_with_missing_comorphism_iri_is_rejected() {
        // Comorphism IRI doesn't resolve anywhere in the span.
        // Common cause: typo, or the witness wasn't committed
        // before the merge attempt. Surfaces as
        // `MergeComorphismNotFound`.
        let (span, backend) = build_span_with_iri_collision_and_optional_witness(None);
        let conflicts = classify_conflicts(&span, &backend).unwrap();
        let conflict_id = conflicts[0].id.clone();

        let result = merge_with_resolutions(
            &span,
            vec![MergeResolution::Witness {
                conflict: conflict_id,
                comorphism: iri("urn:test:missing_witness"),
            }],
            &backend,
        );
        match result {
            Err(MergeError::MergeComorphismNotFound(i)) => {
                assert_eq!(i.as_str(), "urn:test:missing_witness");
            }
            other => panic!("expected MergeComorphismNotFound, got {other:?}"),
        }
    }

    #[test]
    fn witness_pointing_at_non_merge_comorphism_is_rejected() {
        // The IRI resolves to a resource — but it's a plain Class,
        // not a MergeComorphism. The kernel refuses to apply
        // arbitrary resources as witnesses; surfaces as
        // `NotAMergeComorphism` with the actual `is_a` list so the
        // caller can render a useful diagnostic.
        let bogus_witness = make_resource("urn:test:not_a_witness", &[wk::CLASS], &[]);
        let (span, backend) =
            build_span_with_iri_collision_and_optional_witness(Some(bogus_witness));
        let conflicts = classify_conflicts(&span, &backend).unwrap();
        let conflict_id = conflicts[0].id.clone();

        let result = merge_with_resolutions(
            &span,
            vec![MergeResolution::Witness {
                conflict: conflict_id,
                comorphism: iri("urn:test:not_a_witness"),
            }],
            &backend,
        );
        match result {
            Err(MergeError::NotAMergeComorphism {
                iri: i,
                found_classes,
            }) => {
                assert_eq!(i.as_str(), "urn:test:not_a_witness");
                assert!(
                    found_classes.iter().any(|c| c.as_str() == wk::CLASS),
                    "expected `is_a` list to include Class, got {found_classes:?}"
                );
            }
            other => panic!("expected NotAMergeComorphism, got {other:?}"),
        }
    }

    #[test]
    fn valid_witness_progresses_to_application_stub() {
        // The happy validation path: conflict id exists, comorphism
        // IRI resolves to a well-formed MergeComorphism with a
        // `merge_transformation` ResourceRef. Step 2 will plug the
        // evaluator in here; Step 1 surfaces a typed
        // `WitnessApplicationNotYetWired` so a deployment that
        // tries to use witnesses against the in-progress kernel
        // gets an honest error rather than a silent merge failure.
        let (span, backend) = build_span_with_iri_collision_and_optional_witness(Some(
            make_merge_comorphism("urn:test:witness", "urn:test:term_placeholder"),
        ));
        let conflicts = classify_conflicts(&span, &backend).unwrap();
        let conflict_id = conflicts[0].id.clone();

        let result = merge_with_resolutions(
            &span,
            vec![MergeResolution::Witness {
                conflict: conflict_id,
                comorphism: iri("urn:test:witness"),
            }],
            &backend,
        );
        match result {
            Err(MergeError::WitnessApplicationNotYetWired { comorphism }) => {
                assert_eq!(comorphism.as_str(), "urn:test:witness");
            }
            other => panic!(
                "expected WitnessApplicationNotYetWired (15b Step 2 deliverable), got {other:?}"
            ),
        }
    }

    #[test]
    fn malformed_merge_comorphism_missing_transformation_is_rejected() {
        // MergeComorphism resource lacks the required
        // `merge_transformation` property — the resolver detects
        // this rather than the application path discovering it at
        // evaluation time.
        let malformed = make_resource("urn:test:malformed_witness", &[wk::MERGE_COMORPHISM], &[]);
        let (span, backend) = build_span_with_iri_collision_and_optional_witness(Some(malformed));
        let conflicts = classify_conflicts(&span, &backend).unwrap();
        let conflict_id = conflicts[0].id.clone();

        let result = merge_with_resolutions(
            &span,
            vec![MergeResolution::Witness {
                conflict: conflict_id,
                comorphism: iri("urn:test:malformed_witness"),
            }],
            &backend,
        );
        match result {
            Err(MergeError::MalformedMergeComorphism { iri: i, reason }) => {
                assert_eq!(i.as_str(), "urn:test:malformed_witness");
                assert!(
                    reason.contains("merge_transformation"),
                    "reason should mention the missing property; got {reason:?}"
                );
            }
            other => panic!("expected MalformedMergeComorphism, got {other:?}"),
        }
    }

    // ─── 15b·2: Witness term evaluation ────────────────────────────────

    /// Build an embedded-resource body for a Mini-TT `Var <name>`
    /// expression. Embedded (no `@id`) — `parse_var` reads
    /// `program:name` from whatever resource it's handed.
    fn make_var_resource(name: &str) -> Resource {
        let mut r = Resource::new_embedded();
        let is_a_iri = Iri::parse(wk::IS_A).unwrap();
        r.set(
            is_a_iri,
            Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:program:Var"))]),
        );
        r.set(
            iri("urn:eigenius:program:name"),
            Value::String(name.to_string()),
        );
        r
    }

    /// Build an embedded-resource body for a Mini-TT
    /// `Lambda <param> <body>` expression. `parse_lambda` reads
    /// `program:parameter` + `program:body`.
    fn make_lambda_resource(param: &str, body: Resource) -> Resource {
        let mut r = Resource::new_embedded();
        let is_a_iri = Iri::parse(wk::IS_A).unwrap();
        r.set(
            is_a_iri,
            Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:program:Lambda"))]),
        );
        r.set(
            iri("urn:eigenius:program:parameter"),
            Value::String(param.to_string()),
        );
        r.set(
            iri("urn:eigenius:program:body"),
            Value::Embedded(Box::new(body)),
        );
        r
    }

    /// Build a span with a `MergeComorphism` + `λ a. λ b. λ opt. <body>`
    /// transformation committed on the ancestor side. Returns the
    /// backend wrapped in an `Arc` so the `LayerStorage` and the
    /// test's direct backend probes share the same storage instance
    /// — without that, `Layer::resolve` walks a parallel empty
    /// in-memory backend and finds nothing.
    fn build_witness_fixture(
        body: Resource,
    ) -> (
        MergeSpan,
        std::sync::Arc<MemoryPersistentBackend>,
        MergeComorphismHandle,
        crate::layer::LayerStorage,
    ) {
        let transformation_iri = "urn:test:term:identity_b";
        let witness_iri = "urn:test:witness";

        // Three nested Lambdas binding the spec's `a`, `b`, and `opt`
        // (the optional ancestor). Committed at a canonical top-level
        // IRI so `layer.resolve` finds it.
        let inner_opt = make_lambda_resource("opt", body);
        let inner_b = make_lambda_resource("b", inner_opt);
        let transformation = {
            let lam = make_lambda_resource("a", inner_b);
            let mut r = Resource::new(Iri::parse(transformation_iri).unwrap());
            for (k, v) in lam.properties() {
                r.set(k.clone(), v.clone());
            }
            r
        };

        let witness = make_resource(
            witness_iri,
            &[wk::MERGE_COMORPHISM],
            &[(
                wk::MERGE_TRANSFORMATION,
                Value::ResourceRef(iri(transformation_iri)),
            )],
        );

        let (span, backend, storage) = build_span_arc(
            vec![transformation, witness],
            vec![make_resource(
                "urn:test:patient_42",
                &["urn:test:Patient"],
                &[("urn:test:weight", Value::Integer(75))],
            )],
            vec![make_resource(
                "urn:test:patient_42",
                &["urn:test:Patient"],
                &[("urn:test:weight", Value::Integer(76))],
            )],
        );
        let topology = backend.load_topology().unwrap();
        let handle =
            resolve_merge_comorphism(&iri(witness_iri), &span, &topology, &*backend).unwrap();
        (span, backend, handle, storage)
    }

    /// Same as [`build_span`] but threads an `Arc<MemoryPersistentBackend>`
    /// through `LayerStorage::with_persistent` so the apply path's
    /// `build_chain` sees the same storage the test commits to.
    /// Returns the span, the Arc-backed backend, and the storage
    /// the test should pass to `apply_witness_resolution`.
    fn build_span_arc(
        ancestor_resources: Vec<Resource>,
        branch_a_resources: Vec<Resource>,
        branch_b_resources: Vec<Resource>,
    ) -> (
        MergeSpan,
        std::sync::Arc<MemoryPersistentBackend>,
        crate::layer::LayerStorage,
    ) {
        use std::sync::Arc;
        let backend: Arc<MemoryPersistentBackend> = Arc::new(MemoryPersistentBackend::new());
        let backend_dyn: Arc<dyn crate::storage::PersistentBackend> = backend.clone();
        let storage = crate::layer::LayerStorage::with_persistent(Arc::clone(&backend_dyn));

        let mut ab = LayerBuilder::new("ancestor", None);
        for r in ancestor_resources {
            ab.add_resource(r).unwrap();
        }
        let ancestor = Arc::new(ab.build(storage.clone()));
        backend.store_layer(&ancestor).unwrap();

        let mut a_builder = LayerBuilder::new("branch_a", Some(Arc::clone(&ancestor)));
        for r in branch_a_resources {
            a_builder.add_resource(r).unwrap();
        }
        let head_a = Arc::new(a_builder.build(storage.clone()));
        backend.store_layer(&head_a).unwrap();

        let mut b_builder = LayerBuilder::new("branch_b", Some(Arc::clone(&ancestor)));
        for r in branch_b_resources {
            b_builder.add_resource(r).unwrap();
        }
        let head_b = Arc::new(b_builder.build(storage.clone()));
        backend.store_layer(&head_b).unwrap();

        let topology = backend.load_topology().unwrap();
        let sources_a =
            crate::lattice::iri_sources_since(head_a.id(), ancestor.id(), &topology, &*backend)
                .unwrap();
        let sources_b =
            crate::lattice::iri_sources_since(head_b.id(), ancestor.id(), &topology, &*backend)
                .unwrap();

        let span = MergeSpan {
            ancestor: ancestor.id().clone(),
            head_a: head_a.id().clone(),
            head_b: head_b.id().clone(),
            sources_a,
            sources_b,
        };
        (span, backend, storage)
    }

    #[test]
    fn witness_returning_second_argument_produces_branch_b_resource() {
        // Happy-path test: a `λ a. λ b. λ opt. b` witness should
        // produce branch B's body when applied. Pins the round-trip:
        // Resource → Val::ResourceVal → eval → val_to_resource_value
        // → Resource. The merged body's `weight` should match
        // branch B's (76). Ancestor is `None` — the witness ignores
        // its third argument.
        let (_span, backend, handle, storage) = build_witness_fixture(make_var_resource("b"));

        let branch_a = make_resource(
            "urn:test:patient_42",
            &["urn:test:Patient"],
            &[("urn:test:weight", Value::Integer(75))],
        );
        let branch_b = make_resource(
            "urn:test:patient_42",
            &["urn:test:Patient"],
            &[("urn:test:weight", Value::Integer(76))],
        );

        let class = iri("urn:test:Patient");
        let merged = apply_witness_resolution(
            &handle,
            &class,
            branch_a,
            branch_b.clone(),
            None,
            storage,
            &*backend,
        )
        .expect("witness should apply cleanly");

        // `val_to_resource_value` round-trips a `ResourceVal` to
        // `Value::Embedded(resource)`; the wrapper inside
        // `apply_witness_resolution` unboxes that to a `Resource`.
        // The merged body should structurally match branch_b.
        assert_eq!(
            merged.properties().len(),
            branch_b.properties().len(),
            "merged should have the same property count as branch_b; got {merged:?}"
        );
        let weight_iri = iri("urn:test:weight");
        assert_eq!(
            merged.get(&weight_iri),
            branch_b.get(&weight_iri),
            "merged weight should equal branch_b's"
        );
    }

    #[test]
    fn witness_referencing_unbound_variable_surfaces_type_error() {
        // A `λ a. λ b. λ opt. <unknown_var>` witness — the body
        // references a variable name that's not bound by any lambda.
        // Step 3's commit-time type-check catches this before
        // evaluation: the var lookup in `check_infer` fails and the
        // diagnostic is rewrapped as `WitnessTypeMismatch`.
        let (_span, backend, handle, storage) =
            build_witness_fixture(make_var_resource("not_bound_anywhere"));

        let branch_a = make_resource("urn:test:patient_42", &["urn:test:Patient"], &[]);
        let branch_b = make_resource("urn:test:patient_42", &["urn:test:Patient"], &[]);

        let class = iri("urn:test:Patient");
        let result = apply_witness_resolution(
            &handle, &class, branch_a, branch_b, None, storage, &*backend,
        );
        match result {
            Err(MergeError::WitnessTypeMismatch {
                transformation,
                reason,
                ..
            }) => {
                assert_eq!(transformation.as_str(), "urn:test:term:identity_b");
                assert!(
                    reason.contains("not_bound_anywhere"),
                    "reason should mention the unbound variable; got {reason:?}"
                );
            }
            other => panic!("expected WitnessTypeMismatch, got {other:?}"),
        }
    }

    #[test]
    fn apply_witness_resolution_rejects_unparseable_transformation() {
        // A transformation Resource that ISN'T a Mini-TT term (no
        // recognised `is_a`) makes `parse_expression` fail. Surfaces
        // as `TransformationParseError` rather than a panic or
        // generic storage error.
        let transformation_iri = "urn:test:term:bogus";
        let bogus_term = make_resource(transformation_iri, &["urn:test:NotATerm"], &[]);
        let witness_iri = "urn:test:witness";
        let witness = make_resource(
            witness_iri,
            &[wk::MERGE_COMORPHISM],
            &[(
                wk::MERGE_TRANSFORMATION,
                Value::ResourceRef(iri(transformation_iri)),
            )],
        );

        let (span, backend, storage) = build_span_arc(
            vec![bogus_term, witness],
            vec![make_resource(
                "urn:test:patient_42",
                &["urn:test:Patient"],
                &[],
            )],
            vec![make_resource(
                "urn:test:patient_42",
                &["urn:test:Patient"],
                &[],
            )],
        );
        let topology = backend.load_topology().unwrap();
        let handle =
            resolve_merge_comorphism(&iri(witness_iri), &span, &topology, &*backend).unwrap();

        let branch_a = make_resource("urn:test:patient_42", &["urn:test:Patient"], &[]);
        let branch_b = make_resource("urn:test:patient_42", &["urn:test:Patient"], &[]);

        let class = iri("urn:test:Patient");
        let result = apply_witness_resolution(
            &handle, &class, branch_a, branch_b, None, storage, &*backend,
        );
        match result {
            Err(MergeError::TransformationParseError { transformation, .. }) => {
                assert_eq!(transformation.as_str(), transformation_iri);
            }
            other => panic!("expected TransformationParseError, got {other:?}"),
        }
    }

    #[test]
    fn witness_with_wrong_arity_fails_type_check() {
        // A `λ a. a` witness — only one binder, missing the b/opt
        // binders. The expected type is `Π_:A. Π_:A. Π_:Option(A). A`,
        // so check::check fails as soon as it tries to match the
        // body (`a`) against `Π_:A. Π_:Option(A). A` — `a : A` is
        // not a function. Step 3's commit-time check catches this
        // before evaluation.
        let transformation_iri = "urn:test:term:wrong_arity";
        let witness_iri = "urn:test:witness";

        // Build `λ a. a` only (no inner b/opt binders). The body
        // `a` is just a Var resource referring to the outer binder.
        let transformation = {
            let lam = make_lambda_resource("a", make_var_resource("a"));
            let mut r = Resource::new(Iri::parse(transformation_iri).unwrap());
            for (k, v) in lam.properties() {
                r.set(k.clone(), v.clone());
            }
            r
        };
        let witness = make_resource(
            witness_iri,
            &[wk::MERGE_COMORPHISM],
            &[(
                wk::MERGE_TRANSFORMATION,
                Value::ResourceRef(iri(transformation_iri)),
            )],
        );

        let (span, backend, storage) = build_span_arc(
            vec![transformation, witness],
            vec![make_resource(
                "urn:test:patient_42",
                &["urn:test:Patient"],
                &[],
            )],
            vec![make_resource(
                "urn:test:patient_42",
                &["urn:test:Patient"],
                &[],
            )],
        );
        let topology = backend.load_topology().unwrap();
        let handle =
            resolve_merge_comorphism(&iri(witness_iri), &span, &topology, &*backend).unwrap();

        let branch_a = make_resource("urn:test:patient_42", &["urn:test:Patient"], &[]);
        let branch_b = make_resource("urn:test:patient_42", &["urn:test:Patient"], &[]);

        let class = iri("urn:test:Patient");
        let result = apply_witness_resolution(
            &handle, &class, branch_a, branch_b, None, storage, &*backend,
        );
        match result {
            Err(MergeError::WitnessTypeMismatch {
                transformation,
                expected,
                ..
            }) => {
                assert_eq!(transformation.as_str(), transformation_iri);
                assert!(
                    expected.contains("Option"),
                    "expected-type rendering should mention Option; got {expected:?}"
                );
            }
            other => panic!("expected WitnessTypeMismatch, got {other:?}"),
        }
    }

    // ─── Rename (15c) ──────────────────────────────────────────────────────

    /// Build a synthetic ConflictId targeting an IRI. The 15c surface
    /// doesn't yet exercise the conflict-id<->classifier round trip
    /// (IriCollision doesn't fire under open-world today); tests
    /// build deterministic ids and feed them in.
    fn rename_conflict_id(iri_str: &str) -> ConflictId {
        ConflictId::from_iri("iri_collision", &iri(iri_str))
    }

    #[test]
    fn rename_walks_id_and_resource_refs() {
        // Branch B introduces `urn:project:Patient` plus a Profile
        // resource that references Patient via `urn:project:profile_for`.
        // Renaming Patient → BillingPatient must update both the
        // resource at the old IRI *and* the reference inside the
        // Profile resource.
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
        let (span, backend, _storage) =
            build_span_arc(Vec::new(), Vec::new(), vec![patient, profile]);
        let topology = backend.load_topology().unwrap();

        let result = apply_rename_resolution(
            &span,
            Side::B,
            &iri(patient_iri),
            &iri(renamed_iri),
            &topology,
            &*backend,
        )
        .expect("rename should validate + apply cleanly");

        assert_eq!(result.side, Side::B);
        assert_eq!(result.old_iri.as_str(), patient_iri);
        assert_eq!(result.new_iri.as_str(), renamed_iri);

        // Target re-keyed under new IRI; its body is unchanged
        // structurally but its `@id` is rewritten.
        let renamed_patient = result
            .resources
            .get(&iri(renamed_iri))
            .expect("renamed target should be present under new IRI");
        assert_eq!(
            renamed_patient.id().map(|i| i.as_str()),
            Some(renamed_iri),
            "target's @id should be rewritten"
        );

        // Profile re-keyed under its own (unchanged) IRI but with
        // the inner `profile_for` reference rewritten.
        let renamed_profile = result
            .resources
            .get(&iri(profile_iri))
            .expect("profile referencing the renamed target should be re-emitted");
        let profile_for = renamed_profile
            .get(&iri(profile_for_iri))
            .expect("profile_for ref should still exist");
        match profile_for {
            Value::ResourceRef(r) => {
                assert_eq!(r.as_str(), renamed_iri, "ref should be rewritten");
            }
            other => panic!("expected ResourceRef, got {other:?}"),
        }
    }

    #[test]
    fn rename_walks_nested_embedded_and_arrays() {
        // The target IRI is referenced inside an Array containing an
        // Embedded resource whose body references it. The walker
        // must descend through both shapes to find and rewrite the
        // inner ResourceRef.
        let patient_iri = "urn:project:Patient";
        let renamed_iri = "urn:project:billing:Patient";
        let report_iri = "urn:project:report";

        let patient = make_resource(patient_iri, &[wk::CLASS], &[]);
        let mut embedded = Resource::new_embedded();
        embedded.set(
            iri("urn:project:about"),
            Value::ResourceRef(iri(patient_iri)),
        );
        let report = make_resource(
            report_iri,
            &[wk::CLASS],
            &[(
                "urn:project:entries",
                Value::Array(vec![Value::Embedded(Box::new(embedded))]),
            )],
        );

        let (span, backend, _storage) =
            build_span_arc(Vec::new(), Vec::new(), vec![patient, report]);
        let topology = backend.load_topology().unwrap();

        let result = apply_rename_resolution(
            &span,
            Side::B,
            &iri(patient_iri),
            &iri(renamed_iri),
            &topology,
            &*backend,
        )
        .expect("nested rename should succeed");

        let renamed_report = result
            .resources
            .get(&iri(report_iri))
            .expect("report should be re-emitted");
        let entries = renamed_report
            .get(&iri("urn:project:entries"))
            .expect("entries should still be present");
        let inner = match entries {
            Value::Array(items) => items.first().expect("one entry expected"),
            other => panic!("expected Array, got {other:?}"),
        };
        let inner_resource = match inner {
            Value::Embedded(boxed) => boxed.as_ref(),
            other => panic!("expected Embedded, got {other:?}"),
        };
        let about = inner_resource
            .get(&iri("urn:project:about"))
            .expect("nested about ref should still exist");
        match about {
            Value::ResourceRef(r) => assert_eq!(r.as_str(), renamed_iri),
            other => panic!("expected ResourceRef, got {other:?}"),
        }
    }

    #[test]
    fn rename_rejects_collision_with_other_branch() {
        // Branch A introduces `urn:project:billing:Patient`; branch B
        // introduces `urn:project:Patient`. Renaming B's Patient →
        // billing:Patient would silently merge with A's contribution,
        // which is exactly what D20 §6.2 forbids.
        let conflicting_iri = "urn:project:billing:Patient";
        let patient_iri = "urn:project:Patient";

        let a_resources = vec![make_resource(conflicting_iri, &[wk::CLASS], &[])];
        let b_resources = vec![make_resource(patient_iri, &[wk::CLASS], &[])];
        let (span, backend, _storage) = build_span_arc(Vec::new(), a_resources, b_resources);
        let topology = backend.load_topology().unwrap();

        let result = apply_rename_resolution(
            &span,
            Side::B,
            &iri(patient_iri),
            &iri(conflicting_iri),
            &topology,
            &*backend,
        );
        match result {
            Err(MergeError::RenameCollision {
                new_iri,
                location: RenameCollisionSite::OtherBranch(other_side),
            }) => {
                assert_eq!(new_iri.as_str(), conflicting_iri);
                assert_eq!(other_side, Side::A);
            }
            other => panic!("expected RenameCollision::OtherBranch, got {other:?}"),
        }
    }

    #[test]
    fn rename_rejects_collision_with_ancestor_chain() {
        // The ancestor already has `urn:project:billing:Patient`.
        // Branch B introduces `urn:project:Patient`. Renaming B's
        // Patient → billing:Patient would shadow / silently merge
        // with the ancestor's resource.
        let conflicting_iri = "urn:project:billing:Patient";
        let patient_iri = "urn:project:Patient";

        let (span, backend, _storage) = build_span_arc(
            vec![make_resource(conflicting_iri, &[wk::CLASS], &[])],
            Vec::new(),
            vec![make_resource(patient_iri, &[wk::CLASS], &[])],
        );
        let topology = backend.load_topology().unwrap();

        let result = apply_rename_resolution(
            &span,
            Side::B,
            &iri(patient_iri),
            &iri(conflicting_iri),
            &topology,
            &*backend,
        );
        match result {
            Err(MergeError::RenameCollision {
                new_iri,
                location: RenameCollisionSite::AncestorChain,
            }) => {
                assert_eq!(new_iri.as_str(), conflicting_iri);
            }
            other => panic!("expected RenameCollision::AncestorChain, got {other:?}"),
        }
    }

    #[test]
    fn rename_rejects_collision_with_same_branch_contribution() {
        // Branch B introduces both `urn:project:Patient` and
        // `urn:project:billing:Patient`. Renaming Patient →
        // billing:Patient would silently merge the two within the
        // same branch.
        let patient_iri = "urn:project:Patient";
        let billing_iri = "urn:project:billing:Patient";
        let (span, backend, _storage) = build_span_arc(
            Vec::new(),
            Vec::new(),
            vec![
                make_resource(patient_iri, &[wk::CLASS], &[]),
                make_resource(billing_iri, &[wk::CLASS], &[]),
            ],
        );
        let topology = backend.load_topology().unwrap();

        let result = apply_rename_resolution(
            &span,
            Side::B,
            &iri(patient_iri),
            &iri(billing_iri),
            &topology,
            &*backend,
        );
        match result {
            Err(MergeError::RenameCollision {
                new_iri,
                location: RenameCollisionSite::SameBranch(s),
            }) => {
                assert_eq!(new_iri.as_str(), billing_iri);
                assert_eq!(s, Side::B);
            }
            other => panic!("expected RenameCollision::SameBranch, got {other:?}"),
        }
    }

    #[test]
    fn rename_rejects_target_not_in_branch() {
        // Branch A introduces `urn:project:Patient`. Asking to
        // rename it via Side::B is nonsense — B never touched it,
        // so there's nothing to transform.
        let patient_iri = "urn:project:Patient";
        let (span, backend, _storage) = build_span_arc(
            Vec::new(),
            vec![make_resource(patient_iri, &[wk::CLASS], &[])],
            Vec::new(),
        );
        let topology = backend.load_topology().unwrap();

        let result = apply_rename_resolution(
            &span,
            Side::B,
            &iri(patient_iri),
            &iri("urn:project:billing:Patient"),
            &topology,
            &*backend,
        );
        match result {
            Err(MergeError::RenameTargetNotInBranch { old_iri, side }) => {
                assert_eq!(old_iri.as_str(), patient_iri);
                assert_eq!(side, Side::B);
            }
            other => panic!("expected RenameTargetNotInBranch, got {other:?}"),
        }
    }

    #[test]
    fn rename_identity_is_rejected() {
        // old_iri == new_iri makes the rename a no-op. Surface as a
        // typed error so client intent stays explicit.
        let patient_iri = "urn:project:Patient";
        let (span, backend, _storage) = build_span_arc(
            Vec::new(),
            Vec::new(),
            vec![make_resource(patient_iri, &[wk::CLASS], &[])],
        );
        let topology = backend.load_topology().unwrap();

        let result = apply_rename_resolution(
            &span,
            Side::B,
            &iri(patient_iri),
            &iri(patient_iri),
            &topology,
            &*backend,
        );
        match result {
            Err(MergeError::RenameIdentity { iri: i }) => {
                assert_eq!(i.as_str(), patient_iri);
            }
            other => panic!("expected RenameIdentity, got {other:?}"),
        }
    }

    #[test]
    fn rename_skips_branch_contributions_that_do_not_mention_target() {
        // Branch B introduces `Patient` plus an unrelated `Visit`
        // resource that doesn't reference Patient. After rename,
        // only the renamed Patient should be in the output — the
        // unrelated Visit isn't re-emitted (the merge-layer
        // construction path will pick it up from the original
        // contribution unchanged).
        let patient_iri = "urn:project:Patient";
        let renamed_iri = "urn:project:billing:Patient";
        let visit_iri = "urn:project:Visit";

        let (span, backend, _storage) = build_span_arc(
            Vec::new(),
            Vec::new(),
            vec![
                make_resource(patient_iri, &[wk::CLASS], &[]),
                make_resource(visit_iri, &[wk::CLASS], &[]),
            ],
        );
        let topology = backend.load_topology().unwrap();

        let result = apply_rename_resolution(
            &span,
            Side::B,
            &iri(patient_iri),
            &iri(renamed_iri),
            &topology,
            &*backend,
        )
        .expect("rename should succeed");

        assert!(result.resources.contains_key(&iri(renamed_iri)));
        assert!(
            !result.resources.contains_key(&iri(visit_iri)),
            "unrelated Visit shouldn't be re-emitted; got resources {:?}",
            result.resources.keys().collect::<Vec<_>>()
        );
        assert_eq!(result.resources.len(), 1);
    }

    #[test]
    fn merge_with_resolutions_rename_validates_then_reports_pending() {
        // End-to-end through `merge_with_resolutions`: a Rename
        // resolution targets a synthetic conflict id, validation
        // passes, and the surface short-circuits with
        // `RenameApplicationNotYetWired` until 15g lands the
        // merge-layer construction path.
        let patient_iri = "urn:project:Patient";
        let renamed_iri = "urn:project:billing:Patient";

        let (span, backend, _storage) = build_span_arc(
            Vec::new(),
            Vec::new(),
            vec![make_resource(patient_iri, &[wk::CLASS], &[])],
        );

        // Synthesize a conflict so the surface accepts the resolution
        // — classify_conflicts under open-world doesn't yet surface
        // IriCollision, so we feed the resolution against the same
        // discriminator scheme `ConflictId::from_iri` produces.
        let conflict = rename_conflict_id(patient_iri);
        let resolutions = vec![MergeResolution::Rename {
            conflict: conflict.clone(),
            side: Side::B,
            old_iri: iri(patient_iri),
            new_iri: iri(renamed_iri),
        }];
        let result = merge_with_resolutions(&span, resolutions, &*backend);
        match result {
            Err(MergeError::ConflictNotFound(id)) => {
                // Open-world classifier doesn't surface this IRI as
                // a conflict yet — verifies the surface threading.
                assert_eq!(id, conflict);
            }
            other => panic!(
                "expected ConflictNotFound (classifier doesn't yet surface IriCollision); got {other:?}"
            ),
        }
    }

    // ─── SchemaQuotient (15d) ──────────────────────────────────────────────

    /// Build a span with a `PropertyDataType` conflict on
    /// `urn:test:weight`. Branch A = integer, branch B = string.
    fn span_with_property_data_type_conflict() -> (MergeSpan, MemoryPersistentBackend, TypedConflict)
    {
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
        (span, backend, conflicts.into_iter().next().unwrap())
    }

    /// Build a span with a `KindMismatch` conflict on `urn:test:X`.
    fn span_with_kind_mismatch_conflict() -> (MergeSpan, MemoryPersistentBackend, TypedConflict) {
        let class_x = make_resource("urn:test:X", &[wk::CLASS], &[]);
        let prop_x = make_resource("urn:test:X", &[wk::PROPERTY], &[]);
        let (span, backend) = build_span(Vec::new(), vec![class_x], vec![prop_x]);
        let conflicts = classify_conflicts(&span, &backend).unwrap();
        (span, backend, conflicts.into_iter().next().unwrap())
    }

    #[test]
    fn quotient_keep_both_rejected_on_property_data_type() {
        // `KeepBoth` requires the conflict kind to admit both
        // contributions structurally. `PropertyDataType` is
        // single-valued — a property can't have two primitive types.
        let (_span, _backend, conflict) = span_with_property_data_type_conflict();
        let result = apply_quotient_resolution(&conflict, SchemaQuotient::KeepBoth);
        match result {
            Err(MergeError::QuotientNotApplicable {
                conflict_id: id,
                conflict_kind,
                quotient,
                ..
            }) => {
                assert_eq!(id, conflict.id);
                assert_eq!(conflict_kind, "PropertyDataType");
                assert_eq!(quotient, SchemaQuotient::KeepBoth);
            }
            other => panic!("expected QuotientNotApplicable, got {other:?}"),
        }
    }

    #[test]
    fn quotient_keep_both_rejected_on_kind_mismatch() {
        // Kind is single-valued per D1 §3 — same rejection shape as
        // PropertyDataType.
        let (_span, _backend, conflict) = span_with_kind_mismatch_conflict();
        let result = apply_quotient_resolution(&conflict, SchemaQuotient::KeepBoth);
        assert!(
            matches!(result, Err(MergeError::QuotientNotApplicable { .. })),
            "expected QuotientNotApplicable, got {result:?}"
        );
    }

    #[test]
    fn quotient_keep_one_winner_a_drops_property_from_branch_b() {
        let (_span, _backend, conflict) = span_with_property_data_type_conflict();
        let application =
            apply_quotient_resolution(&conflict, SchemaQuotient::KeepOne { winner: Side::A })
                .expect("KeepOne is applicable to PropertyDataType");
        assert_eq!(application.conflict_id, conflict.id);
        assert_eq!(
            application.quotient,
            SchemaQuotient::KeepOne { winner: Side::A }
        );
        assert!(
            application.drop_from_branch_a.is_empty(),
            "winner A — nothing dropped from A; got {:?}",
            application.drop_from_branch_a
        );
        assert_eq!(application.drop_from_branch_b.len(), 1);
        assert_eq!(
            application.drop_from_branch_b[0].as_str(),
            "urn:test:weight"
        );
    }

    #[test]
    fn quotient_keep_one_winner_b_drops_property_from_branch_a() {
        let (_span, _backend, conflict) = span_with_property_data_type_conflict();
        let application =
            apply_quotient_resolution(&conflict, SchemaQuotient::KeepOne { winner: Side::B })
                .expect("KeepOne winner=B is applicable");
        assert!(application.drop_from_branch_b.is_empty());
        assert_eq!(application.drop_from_branch_a.len(), 1);
        assert_eq!(
            application.drop_from_branch_a[0].as_str(),
            "urn:test:weight"
        );
    }

    #[test]
    fn quotient_keep_neither_drops_property_from_both() {
        let (_span, _backend, conflict) = span_with_property_data_type_conflict();
        let application = apply_quotient_resolution(&conflict, SchemaQuotient::KeepNeither)
            .expect("KeepNeither is applicable to PropertyDataType");
        assert_eq!(application.drop_from_branch_a.len(), 1);
        assert_eq!(application.drop_from_branch_b.len(), 1);
        assert_eq!(
            application.drop_from_branch_a[0],
            application.drop_from_branch_b[0]
        );
        assert_eq!(
            application.drop_from_branch_a[0].as_str(),
            "urn:test:weight"
        );
    }

    #[test]
    fn quotient_keep_one_on_kind_mismatch_drops_the_iri() {
        let (_span, _backend, conflict) = span_with_kind_mismatch_conflict();
        let application =
            apply_quotient_resolution(&conflict, SchemaQuotient::KeepOne { winner: Side::A })
                .expect("KeepOne is applicable to KindMismatch");
        assert_eq!(application.drop_from_branch_b.len(), 1);
        assert_eq!(application.drop_from_branch_b[0].as_str(), "urn:test:X");
        assert!(application.drop_from_branch_a.is_empty());
    }

    #[test]
    fn merge_with_resolutions_quotient_rejects_unknown_conflict_id() {
        // The merge dispatch resolves ConflictId → TypedConflict via
        // the classifier-derived index. An id that doesn't classify
        // surfaces as `ConflictNotFound` before reaching the apply
        // function. `apply_quotient_resolution` itself takes a
        // resolved `&TypedConflict` and so cannot return this error.
        let (span, backend, _conflict) = span_with_property_data_type_conflict();
        let bogus_id = ConflictId("nonexistent:foo".to_string());
        let resolutions = vec![MergeResolution::SchemaQuotient {
            conflict: bogus_id.clone(),
            quotient: SchemaQuotient::KeepNeither,
        }];
        let result = merge_with_resolutions(&span, resolutions, &backend);
        match result {
            Err(MergeError::ConflictNotFound(id)) => {
                assert_eq!(id, bogus_id);
            }
            other => panic!("expected ConflictNotFound, got {other:?}"),
        }
    }

    #[test]
    fn merge_with_resolutions_quotient_validates_then_reports_pending() {
        // End-to-end through `merge_with_resolutions`: a KeepOne
        // resolution against a PropertyDataType conflict validates,
        // produces drop sets, and the surface short-circuits with
        // `QuotientApplicationNotYetWired` until 15g lands.
        let (span, backend, conflict) = span_with_property_data_type_conflict();
        let resolutions = vec![MergeResolution::SchemaQuotient {
            conflict: conflict.id.clone(),
            quotient: SchemaQuotient::KeepOne { winner: Side::A },
        }];
        let result = merge_with_resolutions(&span, resolutions, &backend);
        match result {
            Err(MergeError::QuotientApplicationNotYetWired {
                conflict_id: id,
                quotient,
            }) => {
                assert_eq!(id, conflict.id);
                assert_eq!(quotient, SchemaQuotient::KeepOne { winner: Side::A });
            }
            other => panic!("expected QuotientApplicationNotYetWired, got {other:?}"),
        }
    }

    #[test]
    fn merge_with_resolutions_quotient_surfaces_applicability_error() {
        // KeepBoth on a PropertyDataType conflict — the validator
        // rejects before the "not yet wired" short-circuit fires.
        let (span, backend, conflict) = span_with_property_data_type_conflict();
        let resolutions = vec![MergeResolution::SchemaQuotient {
            conflict: conflict.id.clone(),
            quotient: SchemaQuotient::KeepBoth,
        }];
        let result = merge_with_resolutions(&span, resolutions, &backend);
        assert!(
            matches!(result, Err(MergeError::QuotientNotApplicable { .. })),
            "expected QuotientNotApplicable from merge surface, got {result:?}"
        );
    }

    // ─── Restructure (15e) ─────────────────────────────────────────────────

    /// Build the Dog/Mammal/Reptile motivating span from D20 §6.4.
    /// Ancestor has `Mammal` and `Reptile` as classes; branch A
    /// adds `Dog subclass_of Mammal`, branch B adds `Dog subclass_of
    /// Reptile`. Under open-world the classifier doesn't surface
    /// this as a conflict (the union is monotonically combined), so
    /// tests synthesize a ConflictId off `Dog`'s IRI when exercising
    /// merge-dispatch end-to-end.
    fn span_for_restructure() -> (MergeSpan, MemoryPersistentBackend) {
        let mammal = make_resource("urn:test:Mammal", &[wk::CLASS], &[]);
        let reptile = make_resource("urn:test:Reptile", &[wk::CLASS], &[]);
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
                Value::Array(vec![Value::ResourceRef(iri("urn:test:Reptile"))]),
            )],
        );
        build_span(vec![mammal, reptile], vec![dog_a], vec![dog_b])
    }

    fn restructure_conflict_id() -> ConflictId {
        ConflictId::from_iri("subclass_conflict", &iri("urn:test:Dog"))
    }

    /// Build a fresh-style `Animal` Class resource for use as the
    /// `new_parent_def` in restructure tests.
    fn animal_class_def() -> Resource {
        make_resource("urn:test:Animal", &[wk::CLASS], &[])
    }

    #[test]
    fn restructure_rejects_synthesized_parent() {
        let (span, backend) = span_for_restructure();
        let topology = backend.load_topology().unwrap();
        let spec = RestructureSpec {
            affected_class: iri("urn:test:Dog"),
            new_parent: iri("urn:eigenius:auto:CommonParent_42"),
            new_parent_def: None,
            classes_under_new: vec![iri("urn:test:Mammal"), iri("urn:test:Reptile")],
            affected_class_under_new: true,
        };
        let id = restructure_conflict_id();
        let result = apply_restructure_resolution(&id, &spec, &span, &topology, &backend);
        match result {
            Err(MergeError::RestructureSynthesizedParent { new_parent }) => {
                assert_eq!(new_parent.as_str(), "urn:eigenius:auto:CommonParent_42");
            }
            other => panic!("expected RestructureSynthesizedParent, got {other:?}"),
        }
    }

    #[test]
    fn restructure_rejects_redeclaration_of_existing_parent() {
        // Mammal already exists in the ancestor — supplying a def
        // for it is a silent redeclaration.
        let (span, backend) = span_for_restructure();
        let topology = backend.load_topology().unwrap();
        let spec = RestructureSpec {
            affected_class: iri("urn:test:Dog"),
            new_parent: iri("urn:test:Mammal"),
            new_parent_def: Some(make_resource("urn:test:Mammal", &[wk::CLASS], &[])),
            classes_under_new: Vec::new(),
            affected_class_under_new: false,
        };
        let id = restructure_conflict_id();
        let result = apply_restructure_resolution(&id, &spec, &span, &topology, &backend);
        match result {
            Err(MergeError::RestructureParentRedeclaration { new_parent }) => {
                assert_eq!(new_parent.as_str(), "urn:test:Mammal");
            }
            other => panic!("expected RestructureParentRedeclaration, got {other:?}"),
        }
    }

    #[test]
    fn restructure_rejects_missing_definition_for_new_parent() {
        // Animal isn't anywhere in the span; user forgot the def.
        let (span, backend) = span_for_restructure();
        let topology = backend.load_topology().unwrap();
        let spec = RestructureSpec {
            affected_class: iri("urn:test:Dog"),
            new_parent: iri("urn:test:Animal"),
            new_parent_def: None,
            classes_under_new: vec![iri("urn:test:Mammal")],
            affected_class_under_new: false,
        };
        let id = restructure_conflict_id();
        let result = apply_restructure_resolution(&id, &spec, &span, &topology, &backend);
        match result {
            Err(MergeError::RestructureParentMissingDefinition { new_parent }) => {
                assert_eq!(new_parent.as_str(), "urn:test:Animal");
            }
            other => panic!("expected RestructureParentMissingDefinition, got {other:?}"),
        }
    }

    #[test]
    fn restructure_rejects_parent_def_with_mismatched_id() {
        let (span, backend) = span_for_restructure();
        let topology = backend.load_topology().unwrap();
        let bad_def = make_resource("urn:test:NotAnimal", &[wk::CLASS], &[]);
        let spec = RestructureSpec {
            affected_class: iri("urn:test:Dog"),
            new_parent: iri("urn:test:Animal"),
            new_parent_def: Some(bad_def),
            classes_under_new: Vec::new(),
            affected_class_under_new: false,
        };
        let id = restructure_conflict_id();
        let result = apply_restructure_resolution(&id, &spec, &span, &topology, &backend);
        match result {
            Err(MergeError::RestructureParentDefMismatch { new_parent, found }) => {
                assert_eq!(new_parent.as_str(), "urn:test:Animal");
                assert_eq!(
                    found.map(|i| i.as_str().to_string()),
                    Some("urn:test:NotAnimal".to_string())
                );
            }
            other => panic!("expected RestructureParentDefMismatch, got {other:?}"),
        }
    }

    #[test]
    fn restructure_rejects_parent_def_that_is_not_a_class() {
        let (span, backend) = span_for_restructure();
        let topology = backend.load_topology().unwrap();
        // Declared as Property, not Class.
        let bad_def = make_resource("urn:test:Animal", &[wk::PROPERTY], &[]);
        let spec = RestructureSpec {
            affected_class: iri("urn:test:Dog"),
            new_parent: iri("urn:test:Animal"),
            new_parent_def: Some(bad_def),
            classes_under_new: Vec::new(),
            affected_class_under_new: false,
        };
        let id = restructure_conflict_id();
        let result = apply_restructure_resolution(&id, &spec, &span, &topology, &backend);
        match result {
            Err(MergeError::RestructureParentDefNotAClass { new_parent }) => {
                assert_eq!(new_parent.as_str(), "urn:test:Animal");
            }
            other => panic!("expected RestructureParentDefNotAClass, got {other:?}"),
        }
    }

    #[test]
    fn restructure_rejects_affected_class_not_in_span() {
        let (span, backend) = span_for_restructure();
        let topology = backend.load_topology().unwrap();
        let spec = RestructureSpec {
            affected_class: iri("urn:test:Unicorn"),
            new_parent: iri("urn:test:Animal"),
            new_parent_def: Some(animal_class_def()),
            classes_under_new: Vec::new(),
            affected_class_under_new: false,
        };
        let id = restructure_conflict_id();
        let result = apply_restructure_resolution(&id, &spec, &span, &topology, &backend);
        match result {
            Err(MergeError::RestructureClassNotInSpan { iri, role }) => {
                assert_eq!(iri.as_str(), "urn:test:Unicorn");
                assert_eq!(role, RestructureMissingRole::AffectedClass);
            }
            other => panic!("expected RestructureClassNotInSpan, got {other:?}"),
        }
    }

    #[test]
    fn restructure_rejects_classes_under_new_not_in_span() {
        let (span, backend) = span_for_restructure();
        let topology = backend.load_topology().unwrap();
        let spec = RestructureSpec {
            affected_class: iri("urn:test:Dog"),
            new_parent: iri("urn:test:Animal"),
            new_parent_def: Some(animal_class_def()),
            classes_under_new: vec![iri("urn:test:Mammal"), iri("urn:test:Phoenix")],
            affected_class_under_new: true,
        };
        let id = restructure_conflict_id();
        let result = apply_restructure_resolution(&id, &spec, &span, &topology, &backend);
        match result {
            Err(MergeError::RestructureClassNotInSpan { iri, role }) => {
                assert_eq!(iri.as_str(), "urn:test:Phoenix");
                assert_eq!(role, RestructureMissingRole::ClassUnderNew);
            }
            other => panic!("expected RestructureClassNotInSpan, got {other:?}"),
        }
    }

    #[test]
    fn restructure_motivating_example_succeeds_with_dog_under_animal() {
        // The canonical D20 §6.4 case: introduce Animal as a new
        // parent for Mammal, Reptile, and Dog.
        let (span, backend) = span_for_restructure();
        let topology = backend.load_topology().unwrap();
        let animal_def = animal_class_def();
        let spec = RestructureSpec {
            affected_class: iri("urn:test:Dog"),
            new_parent: iri("urn:test:Animal"),
            new_parent_def: Some(animal_def.clone()),
            classes_under_new: vec![iri("urn:test:Mammal"), iri("urn:test:Reptile")],
            affected_class_under_new: true,
        };
        let id = restructure_conflict_id();
        let application = apply_restructure_resolution(&id, &spec, &span, &topology, &backend)
            .expect("canonical Animal/Mammal/Reptile/Dog restructure should succeed");

        assert_eq!(application.conflict_id, id);
        assert_eq!(application.new_parent.as_str(), "urn:test:Animal");
        assert_eq!(application.new_parent_resource, Some(animal_def));
        let names: Vec<&str> = application
            .classes_to_reparent
            .iter()
            .map(|i| i.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["urn:test:Dog", "urn:test:Mammal", "urn:test:Reptile"]
        );
    }

    #[test]
    fn restructure_can_keep_affected_class_outside_new_parent() {
        // Same span, but the user wants Animal as a sibling of Dog
        // (introduced alongside) rather than as Dog's parent.
        // affected_class_under_new = false → Dog is not in the
        // reparent set.
        let (span, backend) = span_for_restructure();
        let topology = backend.load_topology().unwrap();
        let spec = RestructureSpec {
            affected_class: iri("urn:test:Dog"),
            new_parent: iri("urn:test:Animal"),
            new_parent_def: Some(animal_class_def()),
            classes_under_new: vec![iri("urn:test:Mammal"), iri("urn:test:Reptile")],
            affected_class_under_new: false,
        };
        let id = restructure_conflict_id();
        let application = apply_restructure_resolution(&id, &spec, &span, &topology, &backend)
            .expect("Animal-as-sibling restructure should also succeed");
        let names: Vec<&str> = application
            .classes_to_reparent
            .iter()
            .map(|i| i.as_str())
            .collect();
        assert_eq!(names, vec!["urn:test:Mammal", "urn:test:Reptile"]);
        assert!(!application
            .classes_to_reparent
            .iter()
            .any(|i| i.as_str() == "urn:test:Dog"));
    }

    #[test]
    fn restructure_attaches_to_existing_parent_when_no_def_supplied() {
        // Mammal already exists in the ancestor; the user wants
        // Reptile re-parented under Mammal without redeclaring it.
        // Tests the "parent exists, no def" branch.
        let (span, backend) = span_for_restructure();
        let topology = backend.load_topology().unwrap();
        let spec = RestructureSpec {
            affected_class: iri("urn:test:Dog"),
            new_parent: iri("urn:test:Mammal"),
            new_parent_def: None,
            classes_under_new: vec![iri("urn:test:Reptile")],
            affected_class_under_new: false,
        };
        let id = restructure_conflict_id();
        let application = apply_restructure_resolution(&id, &spec, &span, &topology, &backend)
            .expect("attach-to-existing restructure should succeed");
        assert!(application.new_parent_resource.is_none());
        let names: Vec<&str> = application
            .classes_to_reparent
            .iter()
            .map(|i| i.as_str())
            .collect();
        assert_eq!(names, vec!["urn:test:Reptile"]);
    }

    #[test]
    fn merge_with_resolutions_restructure_rejects_unknown_conflict_id() {
        let (span, backend) = span_for_restructure();
        let bogus = ConflictId("nonexistent:foo".to_string());
        let resolutions = vec![MergeResolution::Restructure {
            conflict: bogus.clone(),
            spec: RestructureSpec {
                affected_class: iri("urn:test:Dog"),
                new_parent: iri("urn:test:Animal"),
                new_parent_def: Some(animal_class_def()),
                classes_under_new: Vec::new(),
                affected_class_under_new: true,
            },
        }];
        let result = merge_with_resolutions(&span, resolutions, &backend);
        match result {
            Err(MergeError::ConflictNotFound(id)) => assert_eq!(id, bogus),
            other => panic!("expected ConflictNotFound, got {other:?}"),
        }
    }
}
