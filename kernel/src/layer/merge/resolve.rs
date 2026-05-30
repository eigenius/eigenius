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

//! Resolution surface and merge-layer construction.
//!
//! Three resolution strategies live here — Rename (D20 §6.2),
//! SchemaQuotient (D20 §6.3), Restructure (D20 §6.4). The fourth
//! strategy (Witness, D20 §6.1) lives in [`super::witnessed`];
//! [`MergeResolution`] is the master enum that all four variants
//! share, and the merge-layer construction entry point
//! [`commit_resolutions_as_merge_layer`] dispatches per-variant.
//!
//! [`merge_with_resolutions`] is the front door: it classifies
//! conflicts, gates on cascade acknowledgments, then either returns
//! `NeedsResolution` (no resolutions supplied) or commits a merge
//! layer (resolutions supplied).

use super::cascade::{preview_cascade, verify_cascade_acknowledgments, CascadeAck};
use super::conflict::{
    classify_conflicts, conflict_kind_discriminator, ConflictId, ConflictKind, MergeOutcome,
    MergeSpan, Side, TypedConflict,
};
use super::lca::{find_in_span_chain, find_iri_in_chain};
use super::witnessed::{
    apply_witness_resolution, resolve_merge_comorphism, witness_target_class, witness_target_iri,
};
use super::MergeError;
use crate::layer::handle::LayerTopology;
use crate::layer::LayerId;
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use crate::ontology::well_known as wk;
use crate::storage::{PersistentBackend, StorageError};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

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

/// Attempt a merge with user-supplied resolutions.
///
/// Three distinct phases:
///
/// 1. **Classification.** Always runs first. Empty conflicts +
///    empty resolutions returns a no-op `Merged` outcome carrying
///    `head_a` as the merge layer id — the trivial-merge construction
///    lives in [`crate::lattice::merge_independent_heads`], which
///    owns the "no resolutions needed" path. Non-empty conflicts +
///    empty resolutions returns `NeedsResolution` for the client to
///    fill in.
///
/// 2. **Cascade acknowledgment gate (15f).** When `resolutions` is
///    non-empty, compute the cascade preview and verify every item
///    is acknowledged. The kernel refuses to commit a merge whose
///    downstream consequences haven't been explicitly seen
///    (D20 §8); missing acks surface as
///    `MergeError::IncompleteAcknowledgments`.
///
/// 3. **Merge-layer construction (15g).** Delegates to
///    [`commit_resolutions_as_merge_layer`], which builds a
///    multi-parent layer with both heads as parents and each
///    resolution's transformation applied on top. All six
///    resolution strategies — Witness, Rename, KeepBoth/KeepOne/
///    KeepNeither, Restructure — produce real committed merge
///    layers; the `merge_layer` field of `MergeOutcome::Merged`
///    carries the persisted id.
///
/// On any resolution error the function fails the whole merge;
/// partial applications are not surfaced.
pub fn merge_with_resolutions(
    span: &MergeSpan,
    resolutions: Vec<MergeResolution>,
    acknowledgments: Vec<CascadeAck>,
    extra_branches: Vec<String>,
    storage: crate::layer::LayerStorage,
    backend: &dyn PersistentBackend,
) -> Result<MergeOutcome, MergeError> {
    let conflicts = classify_conflicts(span, backend).map_err(MergeError::Storage)?;

    if resolutions.is_empty() {
        return if conflicts.is_empty() {
            // No structural conflicts — the merge proceeds. The
            // trivial-merge layer construction lives in
            // `lattice::merge_independent_heads`; this surface
            // returns the calling side's head as a no-op outcome so
            // callers that route through here don't fan out a second
            // multi-parent commit path for the trivial case.
            Ok(MergeOutcome::Merged {
                merge_layer: span.head_a.clone(),
            })
        } else {
            Ok(MergeOutcome::NeedsResolution {
                conflicts,
                candidate_chain: format!("{}+{}", span.head_a, span.head_b),
            })
        };
    }

    let layer_name = format!("merge:{}+{}", span.head_a, span.head_b);
    let merge_layer = commit_resolutions_as_merge_layer(
        span,
        &resolutions,
        &acknowledgments,
        &layer_name,
        &extra_branches,
        storage,
        backend,
    )?;
    Ok(MergeOutcome::Merged {
        merge_layer: merge_layer.id().clone(),
    })
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

impl fmt::Display for RenameCollisionSite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
/// recursively — same traversal shape as `iter_iri_values` but with
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
/// rewritten to `new_iri`.
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
pub(crate) fn quotient_target_iris(kind: &ConflictKind) -> Vec<Iri> {
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

// ─── Restructure application (15e) ─────────────────────────────────────────

/// Prefix that flags "synthesized" parent IRIs the kernel refuses for
/// `Restructure` resolutions. D20 §6.4 mandates user-supplied names so
/// the merged schema stays readable; auto-generated parents undermine
/// the structural intent of the resolution.
pub(crate) const SYNTHESIZED_PARENT_PREFIX: &str = "urn:eigenius:auto:";

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

impl fmt::Display for RestructureMissingRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RestructureMissingRole::AffectedClass => write!(f, "affected_class"),
            RestructureMissingRole::ClassUnderNew => write!(f, "classes_under_new entry"),
        }
    }
}

// ─── Merge-layer construction (15g step 1) ─────────────────────────────────

/// Commit a list of resolutions as a multi-parent merge layer and
/// persist it (D20 §7.2 step 6).
///
/// This is the layer-construction surface that turns validated
/// resolutions into a real merged layer. All six resolution
/// strategies have committable shapes: `Witness` evaluates the
/// merge term and commits its result; `Rename` commits the renamed
/// resources + shadows or tombstones the renamed-from IRI;
/// `SchemaQuotient` commits the winner's / ancestor's body or
/// tombstones; `Restructure` commits the new parent + reparents the
/// affected classes.
///
/// Pipeline:
///  1. Compute + verify the cascade preview (D20 §8).
///  2. Verify every classified conflict has a matching resolution
///     (`UnresolvedConflict` if not) and every resolution targets a
///     classified conflict (`ConflictNotFound` if not).
///  3. Resolve both heads to `Arc<Layer>` and build the multi-parent
///     base — `LayerBuilder::with_parents` against `[head_a, head_b]`.
///  4. Compute the union of conflict-target IRIs across all
///     resolutions, then commit every non-target contribution from
///     either branch directly to the merge layer (so branch B's
///     unique contributions remain reachable through the merge
///     layer, even though `Layer::resolve` only follows `parents.first()`).
///  5. Per-resolution dispatch applies each variant's transformation.
///     The merge layer's body takes precedence over the parents'
///     bodies on lookup, so each commit shadows the parent chain at
///     its target IRI.
///  6. Build the layer and persist it via `backend.store_layer`.
///     Return the `Arc<Layer>` so callers can immediately use it
///     (e.g., to CAS-advance a branch ref).
pub fn commit_resolutions_as_merge_layer(
    span: &MergeSpan,
    resolutions: &[MergeResolution],
    acknowledgments: &[CascadeAck],
    name: &str,
    extra_branches: &[String],
    storage: crate::layer::LayerStorage,
    backend: &dyn PersistentBackend,
) -> Result<std::sync::Arc<crate::layer::Layer>, MergeError> {
    use std::sync::Arc;

    // 1. Cascade gate. Same shape as merge_with_resolutions —
    //    inconsistencies surface before any layer construction
    //    happens.
    let preview = preview_cascade(span, resolutions, backend)?;
    verify_cascade_acknowledgments(&preview, acknowledgments)?;

    let conflicts = classify_conflicts(span, backend).map_err(MergeError::Storage)?;
    let conflict_by_id: BTreeMap<&ConflictId, &TypedConflict> =
        conflicts.iter().map(|c| (&c.id, c)).collect();

    // Pre-loop guard: every resolution must target a classified
    // conflict. Resolutions against bogus ids must fail with
    // `ConflictNotFound` before per-variant dispatch — otherwise a
    // typo'd id silently slides into the variant's `*NotYetWired`
    // error and the diagnostic chain becomes misleading.
    for resolution in resolutions {
        let conflict_id = resolution_conflict_id(resolution);
        if !conflict_by_id.contains_key(conflict_id) {
            return Err(MergeError::ConflictNotFound(conflict_id.clone()));
        }
    }

    // Coverage check: every classified conflict must be targeted by
    // exactly one resolution. Without this the merge commits a
    // partial outcome — branches' conflicting bodies would survive
    // unresolved and the resulting chain wouldn't satisfy the
    // classifier post-merge. Reject loudly.
    let resolution_by_conflict: BTreeMap<&ConflictId, &MergeResolution> = resolutions
        .iter()
        .map(|r| (resolution_conflict_id(r), r))
        .collect();
    for conflict in &conflicts {
        if !resolution_by_conflict.contains_key(&conflict.id) {
            return Err(MergeError::UnresolvedConflict {
                conflict_id: conflict.id.clone(),
            });
        }
    }

    // 2. Resolve both heads to `Arc<Layer>` so they can be wired in
    //    as parents. The chain-resolved Layer also gives us a
    //    handle for loading branch-side bodies during witness
    //    application.
    let head_a = load_head_layer(&span.head_a, storage.clone(), backend)?;
    let head_b = load_head_layer(&span.head_b, storage.clone(), backend)?;

    let topology = backend.load_topology().map_err(MergeError::Storage)?;
    let mut builder =
        crate::layer::LayerBuilder::with_parents(name, vec![head_a.clone(), head_b.clone()]);

    // 3. Compute the union of conflict target IRIs across all
    //    resolutions. Non-target contributions from either branch
    //    are committed directly to the merge layer in step 4 so
    //    they remain reachable post-merge (the resolve walker
    //    only follows `parents.first()`, so branch B's unique
    //    contributions aren't reachable through the merge layer's
    //    parent chain — we must replicate them here).
    let mut conflict_target_iris: BTreeSet<Iri> = BTreeSet::new();
    for resolution in resolutions {
        let conflict = conflict_by_id[resolution_conflict_id(resolution)];
        match resolution {
            MergeResolution::Witness { .. } => {
                if let Some(iri) = witness_target_iri(&conflict.kind) {
                    conflict_target_iris.insert(iri.clone());
                }
            }
            MergeResolution::Rename {
                old_iri, new_iri, ..
            } => {
                conflict_target_iris.insert(old_iri.clone());
                conflict_target_iris.insert(new_iri.clone());
            }
            MergeResolution::SchemaQuotient { .. } => {
                conflict_target_iris.extend(quotient_target_iris(&conflict.kind));
            }
            MergeResolution::Restructure { spec, .. } => {
                conflict_target_iris.insert(spec.affected_class.clone());
                conflict_target_iris.insert(spec.new_parent.clone());
                conflict_target_iris.extend(spec.classes_under_new.iter().cloned());
            }
        }
    }

    // 4. Commit non-target contributions from both branches. The
    //    intersection (shared IRIs not in `conflict_target_iris`)
    //    have structurally-equal bodies by construction — the
    //    classifier surfaces every disagreeing shared IRI as a
    //    conflict — so committing either branch's body produces the
    //    same merge view. We pick A's deterministically.
    let mut emitted: BTreeSet<Iri> = BTreeSet::new();
    for (iri, layer_id) in &span.sources_a {
        if conflict_target_iris.contains(iri) {
            continue;
        }
        let body = backend
            .try_load_resource(layer_id, iri)
            .map_err(MergeError::Storage)?
            .ok_or_else(|| {
                MergeError::Storage(StorageError::NotFound(format!(
                    "branch A body for {iri} not loadable from {layer_id}"
                )))
            })?;
        builder
            .add_resource(body)
            .map_err(|e| MergeError::LayerBuild(e.to_string()))?;
        emitted.insert(iri.clone());
    }
    for (iri, layer_id) in &span.sources_b {
        if conflict_target_iris.contains(iri) || emitted.contains(iri) {
            continue;
        }
        let body = backend
            .try_load_resource(layer_id, iri)
            .map_err(MergeError::Storage)?
            .ok_or_else(|| {
                MergeError::Storage(StorageError::NotFound(format!(
                    "branch B body for {iri} not loadable from {layer_id}"
                )))
            })?;
        builder
            .add_resource(body)
            .map_err(|e| MergeError::LayerBuild(e.to_string()))?;
        emitted.insert(iri.clone());
    }

    // 5. Per-resolution dispatch.
    for resolution in resolutions {
        match resolution {
            MergeResolution::Witness {
                conflict,
                comorphism,
            } => {
                let target = conflict_by_id
                    .get(conflict)
                    .ok_or_else(|| MergeError::ConflictNotFound(conflict.clone()))?;
                let conflict_iri = witness_target_iri(&target.kind)
                    .ok_or_else(|| MergeError::WitnessTargetNotResolvable {
                        conflict_id: conflict.clone(),
                    })?
                    .clone();
                let class = witness_target_class(&conflict_iri, span, &topology, backend)?
                    .ok_or_else(|| MergeError::WitnessTargetNotResolvable {
                        conflict_id: conflict.clone(),
                    })?;

                let body_a = backend
                    .try_load_resource(&span.sources_a[&conflict_iri], &conflict_iri)
                    .map_err(MergeError::Storage)?
                    .ok_or_else(|| {
                        MergeError::Storage(StorageError::NotFound(format!(
                            "branch A body for {conflict_iri} not loadable"
                        )))
                    })?;
                let body_b = backend
                    .try_load_resource(&span.sources_b[&conflict_iri], &conflict_iri)
                    .map_err(MergeError::Storage)?
                    .ok_or_else(|| {
                        MergeError::Storage(StorageError::NotFound(format!(
                            "branch B body for {conflict_iri} not loadable"
                        )))
                    })?;
                let ancestor_body =
                    find_iri_in_chain(&span.ancestor, &conflict_iri, &topology, backend)
                        .map_err(MergeError::Storage)?
                        .map(|(_, r)| r);

                let handle = resolve_merge_comorphism(
                    comorphism,
                    &class,
                    span,
                    extra_branches,
                    &topology,
                    backend,
                )?;
                let mut merged = apply_witness_resolution(
                    &handle,
                    &class,
                    body_a,
                    body_b,
                    ancestor_body,
                    storage.clone(),
                    backend,
                )?;
                // Witness terms operate on embedded bodies (the
                // `Resource → ResourceVal → Resource` round-trip
                // drops the `@id`); re-attach the conflict IRI so
                // the layer commit places the merged body at the
                // right key.
                merged.set_id(Some(conflict_iri.clone()));
                builder
                    .add_resource(merged)
                    .map_err(|e| MergeError::LayerBuild(e.to_string()))?;

                // D38 §3.2 step 4: copy the comorphism resource +
                // its transformation Lambda into the merge layer so
                // the provenance record's `merge_record_witness`
                // pointer is guaranteed to resolve on the merge
                // layer's own chain.
                //
                // Guard: only copy when the resource isn't already
                // reachable through the merge layer's parentage. The
                // merge layer's parents are both branch tips; their
                // chains reach back to the ancestor, so anything in
                // `sources_a` / `sources_b` / the ancestor's chain
                // (the merge span) is transitively pinned by the
                // merge layer's reachability already. Duplicating
                // into the contributions would waste storage with
                // no GC benefit. The off-span case — a witness on
                // a branch outside the span, surfaced via D38 §4's
                // `witness_search_branches` (PR 2) — is the one
                // that genuinely needs the copy: those source
                // branches aren't part of the merge layer's
                // parentage, so without the contribution the
                // record's IRI could go dangling if the source
                // branch is later deleted.
                //
                // Both writes (when emitted) are idempotent: the
                // bodies are deterministic and `LayerBuilder::add_resource`
                // is an upsert keyed by IRI, so re-committing the
                // same Witness resolution produces the same merge-
                // layer hash.
                if find_in_span_chain(&handle.iri, span, &topology, backend)
                    .map_err(MergeError::Storage)?
                    .is_none()
                {
                    let comorphism_body = backend
                        .try_load_resource(&handle.source_layer, &handle.iri)
                        .map_err(MergeError::Storage)?
                        .ok_or_else(|| {
                            MergeError::Storage(StorageError::NotFound(format!(
                                "comorphism {} not loadable from source layer {}",
                                handle.iri, handle.source_layer
                            )))
                        })?;
                    builder
                        .add_resource(comorphism_body)
                        .map_err(|e| MergeError::LayerBuild(e.to_string()))?;
                }
                if find_in_span_chain(&handle.transformation, span, &topology, backend)
                    .map_err(MergeError::Storage)?
                    .is_none()
                {
                    let (_, transformation_body) = find_iri_in_chain(
                        &handle.source_layer,
                        &handle.transformation,
                        &topology,
                        backend,
                    )
                    .map_err(MergeError::Storage)?
                    .ok_or_else(|| MergeError::TransformationNotFound {
                        comorphism: handle.iri.clone(),
                        transformation: handle.transformation.clone(),
                    })?;
                    builder
                        .add_resource(transformation_body)
                        .map_err(|e| MergeError::LayerBuild(e.to_string()))?;
                }

                // D38 §3.1 — emit the provenance record. Witness
                // strategy carries `merge_record_witness` (comorphism
                // IRI) and `merge_record_witness_source_layer` (the
                // original-author attribution, preserved after the
                // copy above).
                let record = build_merge_resolution_record(
                    target,
                    resolution,
                    span,
                    &topology,
                    backend,
                    Some(&handle.source_layer),
                )?;
                builder
                    .add_resource(record)
                    .map_err(|e| MergeError::LayerBuild(e.to_string()))?;
            }
            MergeResolution::Rename {
                side,
                old_iri,
                new_iri,
                ..
            } => {
                // Re-run rename validation + transformation. The
                // cascade gate has already passed, but
                // `apply_rename_resolution` also enforces the
                // collision / target-presence rules — surfacing them
                // here rather than letting them slide into a
                // half-committed layer.
                let application =
                    apply_rename_resolution(span, *side, old_iri, new_iri, &topology, backend)?;

                // Commit each renamed resource. The target body is
                // keyed at `new_iri`; references-in-other-resources
                // are keyed at their own (unchanged) IRIs with their
                // bodies rewritten. The builder upserts, so these
                // commits override the step-4 contributions for the
                // same IRIs (which carried the pre-rewrite bodies).
                for resource in application.resources.values() {
                    builder
                        .add_resource(resource.clone())
                        .map_err(|e| MergeError::LayerBuild(e.to_string()))?;
                }

                // Shadow the rename-side's `old_iri` body in the
                // parent chain. If the other branch has `old_iri`
                // (IRI-collision rename case), commit its body —
                // merge-layer precedence makes it the post-merge
                // representative. Otherwise tombstone so resolve
                // returns None for `old_iri` instead of surfacing
                // the renamed-away body via the parent walk.
                let other_sources = match side {
                    Side::A => &span.sources_b,
                    Side::B => &span.sources_a,
                };
                if let Some(other_layer) = other_sources.get(old_iri) {
                    let other_body = backend
                        .try_load_resource(other_layer, old_iri)
                        .map_err(MergeError::Storage)?
                        .ok_or_else(|| {
                            MergeError::Storage(StorageError::NotFound(format!(
                                "other-side body for {old_iri} not loadable from {other_layer}"
                            )))
                        })?;
                    builder
                        .add_resource(other_body)
                        .map_err(|e| MergeError::LayerBuild(e.to_string()))?;
                } else {
                    builder
                        .tombstone(old_iri.clone())
                        .map_err(|e| MergeError::LayerBuild(e.to_string()))?;
                }

                let target = conflict_by_id
                    .get(resolution_conflict_id(resolution))
                    .ok_or_else(|| {
                        MergeError::ConflictNotFound(resolution_conflict_id(resolution).clone())
                    })?;
                let record = build_merge_resolution_record(
                    target, resolution, span, &topology, backend, None,
                )?;
                builder
                    .add_resource(record)
                    .map_err(|e| MergeError::LayerBuild(e.to_string()))?;
            }
            MergeResolution::SchemaQuotient { conflict, quotient } => {
                let target = conflict_by_id
                    .get(conflict)
                    .ok_or_else(|| MergeError::ConflictNotFound(conflict.clone()))?;
                let application = apply_quotient_resolution(target, *quotient)?;
                match *quotient {
                    SchemaQuotient::KeepNeither => {
                        // For each dropped IRI: if the ancestor has a
                        // body, commit it in the merge layer (its
                        // body takes precedence over both branches'
                        // bodies via merge-layer precedence). If the
                        // ancestor has nothing, tombstone the IRI so
                        // the merge layer suppresses both branches'
                        // additions.
                        let mut dropped: BTreeSet<Iri> =
                            application.drop_from_branch_a.iter().cloned().collect();
                        dropped.extend(application.drop_from_branch_b.iter().cloned());
                        for iri in dropped {
                            match find_iri_in_chain(&span.ancestor, &iri, &topology, backend)
                                .map_err(MergeError::Storage)?
                            {
                                Some((_, ancestor_body)) => {
                                    builder
                                        .add_resource(ancestor_body)
                                        .map_err(|e| MergeError::LayerBuild(e.to_string()))?;
                                }
                                None => {
                                    builder
                                        .tombstone(iri)
                                        .map_err(|e| MergeError::LayerBuild(e.to_string()))?;
                                }
                            }
                        }
                    }
                    SchemaQuotient::KeepOne { winner } => {
                        // Commit the winner's body for each conflict
                        // IRI. Merge-layer precedence shadows the
                        // loser's body in the parent chain. If the
                        // winner doesn't have the IRI (it's only on
                        // the loser's side — possible for
                        // `KindMismatch`/`InheritanceCycle` shapes),
                        // tombstone it so the loser's body is
                        // suppressed.
                        let winner_sources = match winner {
                            Side::A => &span.sources_a,
                            Side::B => &span.sources_b,
                        };
                        let loser_sources = match winner {
                            Side::A => &span.sources_b,
                            Side::B => &span.sources_a,
                        };
                        // The application's drop set for the loser
                        // is the set of IRIs the loser contributed
                        // at the conflict point; iterate those.
                        let to_resolve = match winner {
                            Side::A => &application.drop_from_branch_b,
                            Side::B => &application.drop_from_branch_a,
                        };
                        for iri in to_resolve {
                            if let Some(layer_id) = winner_sources.get(iri) {
                                let body = backend
                                    .try_load_resource(layer_id, iri)
                                    .map_err(MergeError::Storage)?
                                    .ok_or_else(|| {
                                        MergeError::Storage(StorageError::NotFound(format!(
                                            "winner body for {iri} not loadable"
                                        )))
                                    })?;
                                builder
                                    .add_resource(body)
                                    .map_err(|e| MergeError::LayerBuild(e.to_string()))?;
                            } else if loser_sources.contains_key(iri) {
                                // Only the loser had it; tombstone
                                // to suppress.
                                builder
                                    .tombstone(iri.clone())
                                    .map_err(|e| MergeError::LayerBuild(e.to_string()))?;
                            }
                        }
                    }
                    SchemaQuotient::KeepBoth => {
                        // Unreachable: `apply_quotient_resolution`
                        // above rejects `KeepBoth` for every v1
                        // conflict kind with `QuotientNotApplicable`,
                        // so the `?` returns before the match. The
                        // arm exists for exhaustiveness; if a future
                        // taxonomy admits `KeepBoth`, this branch
                        // becomes the place to wire its commit shape
                        // and the panic guarantees we don't ship
                        // silent semantics.
                        unreachable!(
                            "KeepBoth is rejected by apply_quotient_resolution before this match"
                        );
                    }
                }

                let record = build_merge_resolution_record(
                    target, resolution, span, &topology, backend, None,
                )?;
                builder
                    .add_resource(record)
                    .map_err(|e| MergeError::LayerBuild(e.to_string()))?;
            }
            MergeResolution::Restructure { conflict, spec } => {
                // Re-validate + apply. The Restructure spec's
                // structural checks (synthesized-parent, def
                // presence, classes-in-span) run inside the apply
                // function — surface failures here, before any
                // partial commit.
                let application =
                    apply_restructure_resolution(conflict, spec, span, &topology, backend)?;

                // 1. Commit the new parent class definition, if the
                //    resolution supplied one (the parent was new to
                //    the span). When the parent already existed,
                //    `new_parent_resource` is `None` and we skip.
                if let Some(new_parent_resource) = &application.new_parent_resource {
                    builder
                        .add_resource(new_parent_resource.clone())
                        .map_err(|e| MergeError::LayerBuild(e.to_string()))?;
                }

                // 2. Reparent each affected class. Semantics differ
                //    between the affected class and the
                //    `classes_under_new` set:
                //    - Affected class: REPLACE `parent_classes` with
                //      `[new_parent]`. This is the "raise the
                //      abstraction" move — the original branch-side
                //      parent disagreement is sidestepped by
                //      pointing the conflicting class at the new
                //      common parent only.
                //    - Other classes (`classes_under_new`): ADD
                //      `new_parent` to existing parents. They keep
                //      their pre-restructure ancestry; the new
                //      parent layers on top.
                let parent_classes_iri =
                    Iri::parse(wk::PARENT_CLASSES).expect("PARENT_CLASSES IRI parses");
                for class_iri in &application.classes_to_reparent {
                    let mut body =
                        load_class_body_for_restructure(class_iri, span, &topology, backend)?;
                    use crate::ontology::resource::Value;
                    if class_iri == &spec.affected_class {
                        body.set(
                            parent_classes_iri.clone(),
                            Value::Array(vec![Value::ResourceRef(spec.new_parent.clone())]),
                        );
                    } else {
                        let mut parents: Vec<Iri> = body
                            .get(&parent_classes_iri)
                            .map(|v| v.as_iri_array())
                            .unwrap_or_default();
                        if !parents.iter().any(|p| p == &spec.new_parent) {
                            parents.push(spec.new_parent.clone());
                        }
                        body.set(
                            parent_classes_iri.clone(),
                            Value::Array(parents.into_iter().map(Value::ResourceRef).collect()),
                        );
                    }
                    builder
                        .add_resource(body)
                        .map_err(|e| MergeError::LayerBuild(e.to_string()))?;
                }

                let target = conflict_by_id
                    .get(conflict)
                    .ok_or_else(|| MergeError::ConflictNotFound(conflict.clone()))?;
                let record = build_merge_resolution_record(
                    target, resolution, span, &topology, backend, None,
                )?;
                builder
                    .add_resource(record)
                    .map_err(|e| MergeError::LayerBuild(e.to_string()))?;
            }
        }
    }

    let layer = Arc::new(builder.build(storage));
    backend.store_layer(&layer).map_err(MergeError::Storage)?;
    Ok(layer)
}

/// Return the `ConflictId` a resolution targets. Used by the
/// merge-layer construction surface's pre-loop guard so that every
/// variant gets the same `ConflictNotFound` treatment.
fn resolution_conflict_id(resolution: &MergeResolution) -> &ConflictId {
    match resolution {
        MergeResolution::Witness { conflict, .. } => conflict,
        MergeResolution::Rename { conflict, .. } => conflict,
        MergeResolution::SchemaQuotient { conflict, .. } => conflict,
        MergeResolution::Restructure { conflict, .. } => conflict,
    }
}

/// Load a layer's full chain and return the head `Arc<Layer>` —
/// the same shape `apply_witness_resolution` builds internally. Used
/// by [`commit_resolutions_as_merge_layer`] so the resulting merge
/// layer can declare both heads as parents with their chains
/// already wired.
fn load_head_layer(
    head: &LayerId,
    storage: crate::layer::LayerStorage,
    backend: &dyn PersistentBackend,
) -> Result<std::sync::Arc<crate::layer::Layer>, MergeError> {
    let chain_info = backend
        .load_chain_from(head)
        .map_err(MergeError::Storage)?
        .ok_or_else(|| {
            MergeError::Storage(StorageError::NotFound(format!(
                "head layer {head} not in store"
            )))
        })?;
    Ok(crate::layer::build_chain(chain_info, storage))
}

/// Build a `MergeResolutionRecord` resource for a single resolved
/// conflict (D38 §3). One record is committed per resolution by
/// [`commit_resolutions_as_merge_layer`]. The record's `@id` is the
/// content-hash of its canonical Eigon-CBOR with `@id` cleared —
/// deterministic resolutions of the same conflict produce the same
/// record IRI across runs.
///
/// `witness_source_layer` is set only for `Witness` resolutions; the
/// commit path threads in `handle.source_layer` so the original
/// authoring attribution survives the witness copy into the merge
/// layer (D38 §3.2).
fn build_merge_resolution_record(
    conflict: &TypedConflict,
    resolution: &MergeResolution,
    span: &MergeSpan,
    topology: &LayerTopology,
    backend: &dyn PersistentBackend,
    witness_source_layer: Option<&LayerId>,
) -> Result<Resource, MergeError> {
    use crate::ontology::resource::Value;

    let mut record = Resource::new_embedded();
    record.set(
        Iri::parse(wk::IS_A).expect("IS_A IRI"),
        Value::Array(vec![Value::ResourceRef(
            Iri::parse(wk::MERGE_RESOLUTION_RECORD).expect("MERGE_RESOLUTION_RECORD IRI"),
        )]),
    );
    record.set(
        Iri::parse(wk::MERGE_RECORD_CONFLICT_ID).expect("MERGE_RECORD_CONFLICT_ID IRI"),
        Value::String(conflict.id.0.clone()),
    );
    record.set(
        Iri::parse(wk::MERGE_RECORD_STRATEGY).expect("MERGE_RECORD_STRATEGY IRI"),
        Value::String(merge_resolution_strategy_name(resolution).to_string()),
    );

    // Branch / ancestor source-layer slots — populated when the
    // conflict has a single primary IRI for which per-side bodies
    // exist. Cycle-shaped conflicts (InheritanceCycle) involve
    // multiple IRIs across both branches; for those the slots stay
    // absent.
    if let Some(target_iri) = primary_record_iri(resolution, &conflict.kind) {
        if let Some(layer_id) = span.sources_a.get(target_iri) {
            record.set(
                Iri::parse(wk::MERGE_RECORD_BRANCH_A_SOURCE_LAYER)
                    .expect("MERGE_RECORD_BRANCH_A_SOURCE_LAYER IRI"),
                Value::String(layer_id.to_string()),
            );
        }
        if let Some(layer_id) = span.sources_b.get(target_iri) {
            record.set(
                Iri::parse(wk::MERGE_RECORD_BRANCH_B_SOURCE_LAYER)
                    .expect("MERGE_RECORD_BRANCH_B_SOURCE_LAYER IRI"),
                Value::String(layer_id.to_string()),
            );
        }
        if let Some((ancestor_layer, _)) =
            find_iri_in_chain(&span.ancestor, target_iri, topology, backend)
                .map_err(MergeError::Storage)?
        {
            record.set(
                Iri::parse(wk::MERGE_RECORD_ANCESTOR_SOURCE_LAYER)
                    .expect("MERGE_RECORD_ANCESTOR_SOURCE_LAYER IRI"),
                Value::String(ancestor_layer.to_string()),
            );
        }
    }

    match resolution {
        MergeResolution::Witness { comorphism, .. } => {
            record.set(
                Iri::parse(wk::MERGE_RECORD_WITNESS).expect("MERGE_RECORD_WITNESS IRI"),
                Value::ResourceRef(comorphism.clone()),
            );
            if let Some(layer_id) = witness_source_layer {
                record.set(
                    Iri::parse(wk::MERGE_RECORD_WITNESS_SOURCE_LAYER)
                        .expect("MERGE_RECORD_WITNESS_SOURCE_LAYER IRI"),
                    Value::String(layer_id.to_string()),
                );
            }
        }
        MergeResolution::Rename {
            side,
            old_iri,
            new_iri,
            ..
        } => {
            record.set(
                Iri::parse(wk::MERGE_RECORD_RENAME_SIDE).expect("MERGE_RECORD_RENAME_SIDE IRI"),
                Value::String(side_label(*side).to_string()),
            );
            record.set(
                Iri::parse(wk::MERGE_RECORD_RENAME_FROM_IRI)
                    .expect("MERGE_RECORD_RENAME_FROM_IRI IRI"),
                Value::ResourceRef(old_iri.clone()),
            );
            record.set(
                Iri::parse(wk::MERGE_RECORD_RENAME_TO_IRI).expect("MERGE_RECORD_RENAME_TO_IRI IRI"),
                Value::ResourceRef(new_iri.clone()),
            );
        }
        MergeResolution::SchemaQuotient { quotient, .. } => {
            let (kind_name, winner) = match quotient {
                SchemaQuotient::KeepBoth => ("KeepBoth", None),
                SchemaQuotient::KeepOne { winner } => ("KeepOne", Some(*winner)),
                SchemaQuotient::KeepNeither => ("KeepNeither", None),
            };
            record.set(
                Iri::parse(wk::MERGE_RECORD_QUOTIENT_KIND).expect("MERGE_RECORD_QUOTIENT_KIND IRI"),
                Value::String(kind_name.to_string()),
            );
            if let Some(winner) = winner {
                record.set(
                    Iri::parse(wk::MERGE_RECORD_QUOTIENT_WINNER)
                        .expect("MERGE_RECORD_QUOTIENT_WINNER IRI"),
                    Value::String(side_label(winner).to_string()),
                );
            }
        }
        MergeResolution::Restructure { spec, .. } => {
            record.set(
                Iri::parse(wk::MERGE_RECORD_RESTRUCTURE_NEW_PARENT)
                    .expect("MERGE_RECORD_RESTRUCTURE_NEW_PARENT IRI"),
                Value::ResourceRef(spec.new_parent.clone()),
            );
            record.set(
                Iri::parse(wk::MERGE_RECORD_RESTRUCTURE_AFFECTED_CLASS)
                    .expect("MERGE_RECORD_RESTRUCTURE_AFFECTED_CLASS IRI"),
                Value::ResourceRef(spec.affected_class.clone()),
            );
        }
    }

    record.set_id(Some(compute_merge_record_iri(&record)));
    Ok(record)
}

/// Strategy-name string for `merge_record_strategy`. Mirrors the
/// `MergeResolution` variant names.
fn merge_resolution_strategy_name(resolution: &MergeResolution) -> &'static str {
    match resolution {
        MergeResolution::Witness { .. } => "Witness",
        MergeResolution::Rename { .. } => "Rename",
        MergeResolution::SchemaQuotient { .. } => "SchemaQuotient",
        MergeResolution::Restructure { .. } => "Restructure",
    }
}

/// Short label for a `Side` value used in record fields
/// (`merge_record_rename_side`, `merge_record_quotient_winner`).
fn side_label(side: Side) -> &'static str {
    match side {
        Side::A => "a",
        Side::B => "b",
    }
}

/// Choose the IRI whose branch source layers go in
/// `merge_record_branch_*_source_layer`. For single-IRI conflicts
/// this is the conflict's target IRI; for Rename it's the renamed-
/// from IRI (the conflict point); for Restructure it's the affected
/// class. Returns `None` for cycle-shaped conflicts that have no
/// single primary IRI.
fn primary_record_iri<'a>(
    resolution: &'a MergeResolution,
    kind: &'a ConflictKind,
) -> Option<&'a Iri> {
    match resolution {
        MergeResolution::Witness { .. } => witness_target_iri(kind),
        MergeResolution::Rename { old_iri, .. } => Some(old_iri),
        MergeResolution::SchemaQuotient { .. } => witness_target_iri(kind),
        MergeResolution::Restructure { spec, .. } => Some(&spec.affected_class),
    }
}

/// Compute the content-hash `@id` for a `MergeResolutionRecord`
/// (D38 §3.1). Mirrors `compute_witness_lambda_iri` in
/// `esl/compile.rs`: serialize the canonical Eigon-CBOR of the
/// resource with `@id` cleared, hash with SHA-256, format as
/// `urn:eigenius:auto:merge-record:<hex>`. Deterministic across
/// runs so re-commits of identical resolutions produce identical
/// IRIs.
fn compute_merge_record_iri(resource: &Resource) -> Iri {
    use sha2::{Digest, Sha256};
    let mut canonical = resource.clone();
    canonical.set_id(None);
    let bytes = crate::ontology::eigon_cbor::serialize_resource(&canonical);
    let digest = Sha256::digest(&bytes);
    let hex = format!("{digest:x}");
    Iri::parse(&format!("urn:eigenius:auto:merge-record:{hex}"))
        .expect("synthesised merge-record IRI must be valid")
}

/// Load the most-recent body for a class IRI somewhere in the merge
/// span — branch A's contribution first, then branch B's, then the
/// ancestor's chain. Used by the Restructure commit path to fetch a
/// starting body for each reparented class so the merge layer's
/// commit can layer the new parent edge on top. Branch A first is a
/// deterministic tie-breaker; classes that agree across branches
/// produce the same body either way.
fn load_class_body_for_restructure(
    iri: &Iri,
    span: &MergeSpan,
    topology: &LayerTopology,
    backend: &dyn PersistentBackend,
) -> Result<Resource, MergeError> {
    if let Some(layer_id) = span.sources_a.get(iri) {
        if let Some(r) = backend
            .try_load_resource(layer_id, iri)
            .map_err(MergeError::Storage)?
        {
            return Ok(r);
        }
    }
    if let Some(layer_id) = span.sources_b.get(iri) {
        if let Some(r) = backend
            .try_load_resource(layer_id, iri)
            .map_err(MergeError::Storage)?
        {
            return Ok(r);
        }
    }
    if let Some((_, r)) =
        find_iri_in_chain(&span.ancestor, iri, topology, backend).map_err(MergeError::Storage)?
    {
        return Ok(r);
    }
    // `apply_restructure_resolution` already verified every class
    // resolves through the span, so reaching this branch implies a
    // storage / topology inconsistency. Surface as `Storage` rather
    // than `unreachable!()` so the operator sees a typed error.
    Err(MergeError::Storage(StorageError::NotFound(format!(
        "restructure: class {iri} not loadable from span"
    ))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::merge::cascade::CascadeAck;
    use crate::layer::merge::test_support::{
        build_span, build_span_arc, build_witness_fixture, build_witness_fixture_offspan, iri,
        make_resource, make_var_resource,
    };
    use crate::ontology::resource::{Resource, Value};
    use crate::storage::memory::MemoryPersistentBackend;
    use std::collections::BTreeSet;

    #[test]
    fn empty_resolutions_with_empty_span_yields_clean_merge() {
        // Sanity baseline: no conflicts + no resolutions = the
        // skeleton placeholder Merged outcome. Pins the 15a path
        // through the new resolution dispatcher.
        let (span, backend) = build_span(Vec::new(), Vec::new(), Vec::new());
        let result = merge_with_resolutions(
            &span,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            crate::layer::LayerStorage::in_memory(),
            &backend,
        );
        match result {
            Ok(MergeOutcome::Merged { .. }) => {}
            other => panic!("expected Merged, got {other:?}"),
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
    fn merge_with_resolutions_rename_unknown_conflict_id() {
        // A Rename resolution targets a synthetic conflict id the
        // classifier didn't produce — under open-world, single-side
        // contributions never surface as conflicts. The pre-loop
        // guard inside `commit_resolutions_as_merge_layer` rejects
        // with `ConflictNotFound` before the Rename dispatch fires.
        let patient_iri = "urn:project:Patient";
        let renamed_iri = "urn:project:billing:Patient";

        let (span, backend, storage) = build_span_arc(
            Vec::new(),
            Vec::new(),
            vec![make_resource(patient_iri, &[wk::CLASS], &[])],
        );

        let conflict = rename_conflict_id(patient_iri);
        let resolutions = vec![MergeResolution::Rename {
            conflict: conflict.clone(),
            side: Side::B,
            old_iri: iri(patient_iri),
            new_iri: iri(renamed_iri),
        }];
        let result = merge_with_resolutions(
            &span,
            resolutions,
            Vec::new(),
            Vec::new(),
            storage,
            &*backend,
        );
        match result {
            Err(MergeError::ConflictNotFound(id)) => {
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
        let result = merge_with_resolutions(
            &span,
            resolutions,
            Vec::new(),
            Vec::new(),
            crate::layer::LayerStorage::in_memory(),
            &backend,
        );
        match result {
            Err(MergeError::ConflictNotFound(id)) => {
                assert_eq!(id, bogus_id);
            }
            other => panic!("expected ConflictNotFound, got {other:?}"),
        }
    }

    #[test]
    fn merge_with_resolutions_keep_one_commits_real_merge_layer() {
        // End-to-end through `merge_with_resolutions`: a KeepOne
        // resolution against a PropertyDataType conflict commits a
        // real merge layer carrying the winner's body.
        let (span, backend, conflict) = span_with_property_data_type_conflict();
        let resolutions = vec![MergeResolution::SchemaQuotient {
            conflict: conflict.id.clone(),
            quotient: SchemaQuotient::KeepOne { winner: Side::A },
        }];
        let result = merge_with_resolutions(
            &span,
            resolutions,
            Vec::new(),
            Vec::new(),
            crate::layer::LayerStorage::in_memory(),
            &backend,
        );
        match result {
            Ok(MergeOutcome::Merged { merge_layer }) => {
                // The merge layer must be persisted and resolvable.
                assert!(backend.load_chain_from(&merge_layer).unwrap().is_some());
            }
            other => panic!("expected Merged, got {other:?}"),
        }
    }

    #[test]
    fn merge_with_resolutions_quotient_surfaces_applicability_error() {
        // KeepBoth on a PropertyDataType conflict — single-valued
        // primitive types can't admit both contributions, so the
        // applicability validator rejects before any commit work.
        let (span, backend, conflict) = span_with_property_data_type_conflict();
        let resolutions = vec![MergeResolution::SchemaQuotient {
            conflict: conflict.id.clone(),
            quotient: SchemaQuotient::KeepBoth,
        }];
        let result = merge_with_resolutions(
            &span,
            resolutions,
            Vec::new(),
            Vec::new(),
            crate::layer::LayerStorage::in_memory(),
            &backend,
        );
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
            classes_under_new: Vec::new(),
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
        // Definition's @id is `urn:test:WrongAnimal`, but we declare
        // new_parent = `urn:test:Animal`.
        let bad_def = make_resource("urn:test:WrongAnimal", &[wk::CLASS], &[]);
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
            Err(MergeError::RestructureParentDefMismatch {
                new_parent,
                found: Some(f),
            }) => {
                assert_eq!(new_parent.as_str(), "urn:test:Animal");
                assert_eq!(f.as_str(), "urn:test:WrongAnimal");
            }
            other => panic!("expected RestructureParentDefMismatch, got {other:?}"),
        }
    }

    #[test]
    fn restructure_rejects_parent_def_that_is_not_a_class() {
        let (span, backend) = span_for_restructure();
        let topology = backend.load_topology().unwrap();
        // Definition's @id matches but it's typed as Property, not
        // Class.
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
        let result = merge_with_resolutions(
            &span,
            resolutions,
            Vec::new(),
            Vec::new(),
            crate::layer::LayerStorage::in_memory(),
            &backend,
        );
        match result {
            Err(MergeError::ConflictNotFound(id)) => assert_eq!(id, bogus),
            other => panic!("expected ConflictNotFound, got {other:?}"),
        }
    }

    #[test]
    fn commit_restructure_introduces_animal_and_reparents_classes() {
        // D20 §6.4 motivating example, committed end-to-end. Both
        // branches contribute `Dog` with structurally-different
        // bodies (so the classifier surfaces an IriCollision that
        // the Restructure resolution targets). Ancestor has Mammal
        // and Reptile; the resolution introduces Animal, makes
        // Mammal/Reptile subclass it, and points Dog at Animal only.
        let mammal = make_resource("urn:test:Mammal", &[wk::CLASS], &[]);
        let reptile = make_resource("urn:test:Reptile", &[wk::CLASS], &[]);
        // Different parent_classes on each branch → IriCollision
        // (structural body inequality at `Dog`).
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
        let (span, backend, storage) =
            build_span_arc(vec![mammal, reptile], vec![dog_a], vec![dog_b]);
        let conflicts = classify_conflicts(&span, &*backend).unwrap();
        assert_eq!(conflicts.len(), 1, "expected one IriCollision on Dog");
        let conflict_id = conflicts[0].id.clone();

        let resolutions = vec![MergeResolution::Restructure {
            conflict: conflict_id,
            spec: RestructureSpec {
                affected_class: iri("urn:test:Dog"),
                new_parent: iri("urn:test:Animal"),
                new_parent_def: Some(animal_class_def()),
                classes_under_new: vec![iri("urn:test:Mammal"), iri("urn:test:Reptile")],
                affected_class_under_new: true,
            },
        }];
        let merge_layer = commit_resolutions_as_merge_layer(
            &span,
            &resolutions,
            &[],
            "merge:restructure_animal",
            &[],
            storage,
            &*backend,
        )
        .expect("Restructure should commit cleanly");

        // 1. The new Animal Class lives in the merge layer.
        assert!(
            merge_layer.resolve(&iri("urn:test:Animal")).is_some(),
            "Animal should be reachable post-merge"
        );

        // 2. Dog's `parent_classes` is replaced with [Animal] —
        //    the original disagreement is sidestepped by raising
        //    the abstraction.
        let dog = merge_layer
            .resolve(&iri("urn:test:Dog"))
            .expect("Dog should resolve");
        let dog_parents: Vec<String> = dog
            .get(&iri(wk::PARENT_CLASSES))
            .map(|v| v.as_iri_array())
            .unwrap_or_default()
            .iter()
            .map(|i| i.as_str().to_string())
            .collect();
        assert_eq!(dog_parents, vec!["urn:test:Animal".to_string()]);

        // 3. Mammal and Reptile gain Animal in their parents
        //    (additive — keeping any pre-existing ancestry intact).
        let mammal = merge_layer
            .resolve(&iri("urn:test:Mammal"))
            .expect("Mammal should resolve");
        let mammal_parents: Vec<String> = mammal
            .get(&iri(wk::PARENT_CLASSES))
            .map(|v| v.as_iri_array())
            .unwrap_or_default()
            .iter()
            .map(|i| i.as_str().to_string())
            .collect();
        assert!(
            mammal_parents.contains(&"urn:test:Animal".to_string()),
            "Mammal should subclass Animal; got {mammal_parents:?}"
        );
        let reptile = merge_layer
            .resolve(&iri("urn:test:Reptile"))
            .expect("Reptile should resolve");
        let reptile_parents: Vec<String> = reptile
            .get(&iri(wk::PARENT_CLASSES))
            .map(|v| v.as_iri_array())
            .unwrap_or_default()
            .iter()
            .map(|i| i.as_str().to_string())
            .collect();
        assert!(
            reptile_parents.contains(&"urn:test:Animal".to_string()),
            "Reptile should subclass Animal; got {reptile_parents:?}"
        );
    }

    // ─── Merge-layer construction (15g step 1) ─────────────────────────────

    #[test]
    fn commit_witness_resolution_produces_multi_parent_merge_layer() {
        // End-to-end happy path: build the witness fixture, classify
        // (surfaces IriCollision for patient_42), submit a Witness
        // resolution + ack the empty cascade, commit. Verify the
        // returned layer has both heads as parents and the merged
        // body lives at the conflict's IRI.
        let (span, backend, handle, storage) = build_witness_fixture(make_var_resource("b"));

        let conflicts = classify_conflicts(&span, &*backend).unwrap();
        assert_eq!(
            conflicts.len(),
            1,
            "expected one IriCollision; got {conflicts:?}"
        );
        let conflict_id = conflicts[0].id.clone();

        let resolutions = vec![MergeResolution::Witness {
            conflict: conflict_id,
            comorphism: handle.iri.clone(),
        }];
        // Witness cascade is empty by design (well-typed by
        // construction). No acks needed.
        let merge_layer = commit_resolutions_as_merge_layer(
            &span,
            &resolutions,
            &[],
            "merge:patient_test",
            &[],
            storage.clone(),
            &*backend,
        )
        .expect("commit should succeed");

        // Multi-parent topology: both heads should be the layer's
        // immediate parents.
        let parent_ids: BTreeSet<LayerId> = merge_layer
            .parents()
            .iter()
            .map(|l| l.id().clone())
            .collect();
        let mut expected = BTreeSet::new();
        expected.insert(span.head_a.clone());
        expected.insert(span.head_b.clone());
        assert_eq!(parent_ids, expected);

        // The merged body lives in the merge layer at the conflict
        // IRI. The `λa.λb.λopt.b` witness returns branch B's body,
        // so weight should match 76 (branch B's value).
        let patient_iri = iri("urn:test:patient_42");
        let weight_iri = iri("urn:test:weight");
        let merged_resource = merge_layer
            .resolve(&patient_iri)
            .expect("merged body should be reachable from merge layer");
        assert_eq!(merged_resource.get(&weight_iri), Some(&Value::Integer(76)));

        // Verify the layer was persisted — re-load it from the backend.
        let reloaded = backend
            .load_chain_from(merge_layer.id())
            .unwrap()
            .expect("layer should be in store");
        assert!(
            reloaded.handles.iter().any(|h| h.id == *merge_layer.id()),
            "persisted chain should include the merge layer"
        );
    }

    #[test]
    fn commit_witness_rejects_missing_cascade_acks() {
        // If the cascade preview surfaces items (e.g., from a
        // witness fixture that introduces external refs), missing
        // acks must surface before any layer is built. The witness
        // fixture's branches don't have cross-references so the
        // preview is empty — to exercise the gate, we synthesize a
        // missing ack by passing one that doesn't match anything in
        // the (empty) preview. Witness cascade is empty → no acks
        // needed → call succeeds, NOT the right test shape.
        //
        // Instead: pin that a Rename resolution submitted alongside
        // a witness (the Rename surfaces cascade items) errors out
        // with IncompleteAcknowledgments before commit. This also
        // pins that the gate runs before per-resolution dispatch.
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
        let result = commit_resolutions_as_merge_layer(
            &span,
            &resolutions,
            &[],
            "merge:should_not_build",
            &[],
            storage,
            &*backend,
        );
        match result {
            Err(MergeError::IncompleteAcknowledgments { missing }) => {
                assert!(!missing.is_empty());
            }
            other => panic!("expected IncompleteAcknowledgments, got {other:?}"),
        }
    }

    #[test]
    fn merge_with_resolutions_commits_witness_resolution_to_real_layer() {
        // End-to-end through the unified `merge_with_resolutions`
        // surface (15g step 1 wiring): a Witness resolution against a
        // classified IriCollision conflict produces a real
        // `MergeOutcome::Merged` with a multi-parent layer id —
        // confirming the placeholder Merged path is replaced by
        // `commit_resolutions_as_merge_layer` whenever resolutions
        // are non-empty.
        let (span, backend, handle, storage) = build_witness_fixture(make_var_resource("b"));
        let conflicts = classify_conflicts(&span, &*backend).unwrap();
        let conflict_id = conflicts[0].id.clone();

        let result = merge_with_resolutions(
            &span,
            vec![MergeResolution::Witness {
                conflict: conflict_id,
                comorphism: handle.iri.clone(),
            }],
            Vec::new(),
            Vec::new(),
            storage,
            &*backend,
        );
        match result {
            Ok(MergeOutcome::Merged { merge_layer }) => {
                // Layer id must be reachable from the backend and
                // declare both heads as parents.
                let chain = backend
                    .load_chain_from(&merge_layer)
                    .unwrap()
                    .expect("merge layer should be persisted");
                let head_handle = chain
                    .handles
                    .iter()
                    .find(|h| h.id == merge_layer)
                    .expect("chain should include the merge layer");
                let parents: BTreeSet<LayerId> = head_handle.parents.iter().cloned().collect();
                let mut expected = BTreeSet::new();
                expected.insert(span.head_a.clone());
                expected.insert(span.head_b.clone());
                assert_eq!(parents, expected);
            }
            other => panic!("expected Merged with real layer id, got {other:?}"),
        }
    }

    // ─── D38 §3 — Merge resolution records ──────────────────────────────────

    /// Find the single MergeResolutionRecord defined directly in
    /// `merge_layer` and return a clone of its resolved body. Panics
    /// if zero or more than one record IRI is defined — the tests
    /// assume one-record-per-resolution. Resolves through the layer
    /// chain rather than reading `local_resources` so the assertions
    /// run against the same shape clients see (post canonicalisation).
    fn single_record_in(merge_layer: &crate::layer::Layer) -> Resource {
        let record_iris: Vec<Iri> = merge_layer
            .defined_iris()
            .iter()
            .filter(|i| i.as_str().starts_with("urn:eigenius:auto:merge-record:"))
            .cloned()
            .collect();
        assert_eq!(
            record_iris.len(),
            1,
            "expected exactly one merge-record IRI in merge layer; got {record_iris:?}"
        );
        let body = merge_layer
            .resolve(&record_iris[0])
            .expect("record IRI must resolve");
        (*body).clone()
    }

    #[test]
    fn merge_records_witness_emits_record_with_strategy_fields() {
        // D38 §3: a Witness resolution emits a `MergeResolutionRecord`
        // with `strategy = "Witness"`, `witness = <comorphism IRI>`,
        // and `witness_source_layer` carrying the original committing
        // layer id.
        let (span, backend, handle, storage) = build_witness_fixture(make_var_resource("b"));
        let conflicts = classify_conflicts(&span, &*backend).unwrap();
        let conflict_id = conflicts[0].id.clone();
        let comorphism_source_layer = handle.source_layer.to_string();

        let resolutions = vec![MergeResolution::Witness {
            conflict: conflict_id.clone(),
            comorphism: handle.iri.clone(),
        }];
        let merge_layer = commit_resolutions_as_merge_layer(
            &span,
            &resolutions,
            &[],
            "merge:record_witness",
            &[],
            storage,
            &*backend,
        )
        .expect("witness commit");

        let record = single_record_in(&merge_layer);
        assert_eq!(
            record
                .get(&iri(wk::MERGE_RECORD_STRATEGY))
                .and_then(|v| v.as_str()),
            Some("Witness"),
        );
        assert_eq!(
            record
                .get(&iri(wk::MERGE_RECORD_CONFLICT_ID))
                .and_then(|v| v.as_str()),
            Some(conflict_id.0.as_str()),
        );
        match record.get(&iri(wk::MERGE_RECORD_WITNESS)) {
            Some(Value::ResourceRef(r)) => assert_eq!(r, &handle.iri),
            other => panic!("merge_record_witness should be a ResourceRef, got {other:?}"),
        }
        assert_eq!(
            record
                .get(&iri(wk::MERGE_RECORD_WITNESS_SOURCE_LAYER))
                .and_then(|v| v.as_str()),
            Some(comorphism_source_layer.as_str()),
        );
    }

    #[test]
    fn merge_records_witness_off_span_copies_into_merge_layer() {
        // D38 §3.2 step-4 guard, off-span branch: when the witness
        // lives outside the merge span (reached via the fourth-tier
        // `extra_branches` walk), the commit path copies both the
        // comorphism and its transformation Lambda into the merge
        // layer's contributions at their original IRIs — so the
        // resolution trace stays resolvable even if the source
        // branch is later deleted. Also pins that `merge_record_witness_source_layer`
        // captures the original-author attribution.
        let (span, backend, witness_iri, storage) =
            build_witness_fixture_offspan(make_var_resource("b"));
        let topology = backend.load_topology().unwrap();
        let comorphism_source_layer = backend
            .get_branch("witness-library")
            .unwrap()
            .expect("witness-library branch should be registered");

        // Confirm the resolver can find the witness via extra_branches.
        let extra = vec!["witness-library".to_string()];
        let _ = resolve_merge_comorphism(
            &witness_iri,
            &iri("urn:test:Patient"),
            &span,
            &extra,
            &topology,
            &*backend,
        )
        .expect("off-span resolve must succeed with extra_branches");

        let conflicts = classify_conflicts(&span, &*backend).unwrap();
        let conflict_id = conflicts[0].id.clone();
        let resolutions = vec![MergeResolution::Witness {
            conflict: conflict_id,
            comorphism: witness_iri.clone(),
        }];
        let merge_layer = commit_resolutions_as_merge_layer(
            &span,
            &resolutions,
            &[],
            "merge:off_span_copy",
            &extra,
            storage,
            &*backend,
        )
        .expect("off-span witness commit");

        // The off-span comorphism + its transformation are now
        // contributions of the merge layer — guarantees the merge
        // layer is self-contained if `witness-library` is deleted.
        assert!(
            merge_layer.defined_iris().contains(&witness_iri),
            "off-span comorphism should be copied into the merge layer's contributions"
        );
        let transformation_iri = iri("urn:test:term:identity_b_offspan");
        assert!(
            merge_layer.defined_iris().contains(&transformation_iri),
            "off-span transformation should be copied into the merge layer's contributions"
        );

        // Record carries the source-layer attribution.
        let record = single_record_in(&merge_layer);
        assert_eq!(
            record
                .get(&iri(wk::MERGE_RECORD_WITNESS_SOURCE_LAYER))
                .and_then(|v| v.as_str()),
            Some(comorphism_source_layer.to_string().as_str()),
        );
    }

    #[test]
    fn merge_records_witness_in_span_skips_copy_but_stays_resolvable() {
        // D38 §3.2 guard: when the comorphism + transformation are
        // reachable through the merge span (sources_a / sources_b /
        // ancestor chain), the commit path skips the copy — the
        // merge layer's parent chain already pins them transitively,
        // so a contribution would just duplicate the body. The
        // `merge_record_witness` pointer must still resolve through
        // the merge layer (via the parent walk). In `build_witness_fixture`
        // the comorphism + transformation both live on the ancestor,
        // i.e., in-span, so neither should appear in `defined_iris`.
        let (span, backend, handle, storage) = build_witness_fixture(make_var_resource("b"));
        let conflicts = classify_conflicts(&span, &*backend).unwrap();
        let conflict_id = conflicts[0].id.clone();

        let resolutions = vec![MergeResolution::Witness {
            conflict: conflict_id,
            comorphism: handle.iri.clone(),
        }];
        let merge_layer = commit_resolutions_as_merge_layer(
            &span,
            &resolutions,
            &[],
            "merge:in_span_skip_copy",
            &[],
            storage,
            &*backend,
        )
        .expect("witness commit");

        assert!(
            !merge_layer.defined_iris().contains(&handle.iri),
            "in-span comorphism should not be duplicated into the merge layer's contributions"
        );
        assert!(
            !merge_layer.defined_iris().contains(&handle.transformation),
            "in-span transformation should not be duplicated into the merge layer's contributions"
        );
        assert!(
            merge_layer.resolve(&handle.iri).is_some(),
            "comorphism must still resolve through the merge layer's parent chain"
        );
        assert!(
            merge_layer.resolve(&handle.transformation).is_some(),
            "transformation must still resolve through the merge layer's parent chain"
        );
    }

    #[test]
    fn merge_records_witness_recommit_is_idempotent() {
        // Re-committing the same Witness resolution against the same
        // span yields a merge layer with the same id. The record's
        // `@id` is content-hashed, the witness copy is content-keyed,
        // and the merged body is deterministic — all three are
        // covered by the same hash-equality assertion.
        let (span, backend, handle, storage) = build_witness_fixture(make_var_resource("b"));
        let conflicts = classify_conflicts(&span, &*backend).unwrap();
        let conflict_id = conflicts[0].id.clone();
        let resolutions = vec![MergeResolution::Witness {
            conflict: conflict_id,
            comorphism: handle.iri.clone(),
        }];

        let layer_a = commit_resolutions_as_merge_layer(
            &span,
            &resolutions,
            &[],
            "merge:idempotent_witness",
            &[],
            storage.clone(),
            &*backend,
        )
        .expect("first commit");
        let layer_b = commit_resolutions_as_merge_layer(
            &span,
            &resolutions,
            &[],
            "merge:idempotent_witness",
            &[],
            storage,
            &*backend,
        )
        .expect("second commit");
        assert_eq!(
            layer_a.id(),
            layer_b.id(),
            "re-committing the same Witness resolution must produce the same merge-layer id",
        );
    }

    #[test]
    fn merge_records_rename_emits_record_with_strategy_fields() {
        // Rename records carry rename_side / rename_from_iri /
        // rename_to_iri alongside the base required slots.
        let patient_iri = "urn:project:Patient";
        let renamed_iri = "urn:project:billing:Patient";
        let (span, backend, storage) = build_span_arc(
            Vec::new(),
            vec![make_resource(
                patient_iri,
                &[wk::CLASS],
                &[("urn:project:label", Value::String("A".to_string()))],
            )],
            vec![make_resource(
                patient_iri,
                &[wk::CLASS],
                &[("urn:project:label", Value::String("B".to_string()))],
            )],
        );
        let conflicts = classify_conflicts(&span, &*backend).unwrap();
        let conflict_id = conflicts[0].id.clone();
        let resolutions = vec![MergeResolution::Rename {
            conflict: conflict_id.clone(),
            side: Side::A,
            old_iri: iri(patient_iri),
            new_iri: iri(renamed_iri),
        }];
        let merge_layer = commit_resolutions_as_merge_layer(
            &span,
            &resolutions,
            &[],
            "merge:record_rename",
            &[],
            storage,
            &*backend,
        )
        .expect("rename commit");

        let record = single_record_in(&merge_layer);
        assert_eq!(
            record
                .get(&iri(wk::MERGE_RECORD_STRATEGY))
                .and_then(|v| v.as_str()),
            Some("Rename"),
        );
        assert_eq!(
            record
                .get(&iri(wk::MERGE_RECORD_RENAME_SIDE))
                .and_then(|v| v.as_str()),
            Some("a"),
        );
        match record.get(&iri(wk::MERGE_RECORD_RENAME_FROM_IRI)) {
            Some(Value::ResourceRef(r)) => assert_eq!(r.as_str(), patient_iri),
            other => panic!("rename_from_iri should be ResourceRef, got {other:?}"),
        }
        match record.get(&iri(wk::MERGE_RECORD_RENAME_TO_IRI)) {
            Some(Value::ResourceRef(r)) => assert_eq!(r.as_str(), renamed_iri),
            other => panic!("rename_to_iri should be ResourceRef, got {other:?}"),
        }
        let _ = conflict_id;
    }

    #[test]
    fn merge_records_quotient_keep_one_emits_kind_and_winner() {
        // KeepOne records carry `quotient_kind = "KeepOne"` plus
        // `quotient_winner` ("a" / "b"). KeepNeither carries only
        // the kind.
        let body_a = make_resource(
            "urn:test:X",
            &["urn:test:Thing"],
            &[("urn:test:weight", Value::Integer(1))],
        );
        let body_b = make_resource(
            "urn:test:X",
            &["urn:test:Thing"],
            &[("urn:test:weight", Value::Integer(2))],
        );
        let (span, backend, storage) = build_span_arc(Vec::new(), vec![body_a], vec![body_b]);
        let conflicts = classify_conflicts(&span, &*backend).unwrap();
        let conflict_id = conflicts[0].id.clone();
        let resolutions = vec![MergeResolution::SchemaQuotient {
            conflict: conflict_id,
            quotient: SchemaQuotient::KeepOne { winner: Side::B },
        }];
        let merge_layer = commit_resolutions_as_merge_layer(
            &span,
            &resolutions,
            &[],
            "merge:record_quotient",
            &[],
            storage,
            &*backend,
        )
        .expect("quotient commit");

        let record = single_record_in(&merge_layer);
        assert_eq!(
            record
                .get(&iri(wk::MERGE_RECORD_STRATEGY))
                .and_then(|v| v.as_str()),
            Some("SchemaQuotient"),
        );
        assert_eq!(
            record
                .get(&iri(wk::MERGE_RECORD_QUOTIENT_KIND))
                .and_then(|v| v.as_str()),
            Some("KeepOne"),
        );
        assert_eq!(
            record
                .get(&iri(wk::MERGE_RECORD_QUOTIENT_WINNER))
                .and_then(|v| v.as_str()),
            Some("b"),
        );
    }

    #[test]
    fn merge_records_restructure_emits_new_parent_and_affected() {
        // Restructure records carry restructure_new_parent +
        // restructure_affected_class.
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
        let (span, backend, storage) =
            build_span_arc(vec![mammal, reptile], vec![dog_a], vec![dog_b]);
        let conflicts = classify_conflicts(&span, &*backend).unwrap();
        let conflict_id = conflicts[0].id.clone();
        let resolutions = vec![MergeResolution::Restructure {
            conflict: conflict_id,
            spec: RestructureSpec {
                affected_class: iri("urn:test:Dog"),
                new_parent: iri("urn:test:Animal"),
                new_parent_def: Some(animal_class_def()),
                classes_under_new: vec![iri("urn:test:Mammal"), iri("urn:test:Reptile")],
                affected_class_under_new: true,
            },
        }];
        let merge_layer = commit_resolutions_as_merge_layer(
            &span,
            &resolutions,
            &[],
            "merge:record_restructure",
            &[],
            storage,
            &*backend,
        )
        .expect("restructure commit");

        let record = single_record_in(&merge_layer);
        assert_eq!(
            record
                .get(&iri(wk::MERGE_RECORD_STRATEGY))
                .and_then(|v| v.as_str()),
            Some("Restructure"),
        );
        match record.get(&iri(wk::MERGE_RECORD_RESTRUCTURE_NEW_PARENT)) {
            Some(Value::ResourceRef(r)) => assert_eq!(r.as_str(), "urn:test:Animal"),
            other => panic!("restructure_new_parent should be ResourceRef, got {other:?}"),
        }
        match record.get(&iri(wk::MERGE_RECORD_RESTRUCTURE_AFFECTED_CLASS)) {
            Some(Value::ResourceRef(r)) => assert_eq!(r.as_str(), "urn:test:Dog"),
            other => panic!("restructure_affected_class should be ResourceRef, got {other:?}"),
        }
    }

    #[test]
    fn commit_rename_iri_collision_keeps_other_side_at_old_iri() {
        // IRI-collision case: both branches contribute `Patient`
        // with different bodies. Renaming A's Patient →
        // billing:Patient should produce a merge layer where
        // `Patient` resolves to B's body (the un-renamed side) and
        // `billing:Patient` resolves to A's renamed body. Pins the
        // "shadow old_iri via other-side body" path.
        let patient_iri = "urn:project:Patient";
        let renamed_iri = "urn:project:billing:Patient";
        let (span, backend, storage) = build_span_arc(
            Vec::new(),
            vec![make_resource(
                patient_iri,
                &[wk::CLASS],
                &[("urn:project:label", Value::String("A".to_string()))],
            )],
            vec![make_resource(
                patient_iri,
                &[wk::CLASS],
                &[("urn:project:label", Value::String("B".to_string()))],
            )],
        );
        let conflicts = classify_conflicts(&span, &*backend).unwrap();
        let conflict_id = conflicts[0].id.clone();
        let resolutions = vec![MergeResolution::Rename {
            conflict: conflict_id,
            side: Side::A,
            old_iri: iri(patient_iri),
            new_iri: iri(renamed_iri),
        }];
        let merge_layer = commit_resolutions_as_merge_layer(
            &span,
            &resolutions,
            &[],
            "merge:rename_collision",
            &[],
            storage,
            &*backend,
        )
        .expect("Rename should commit cleanly");

        // `Patient` post-merge: B's body (other side's).
        let resolved = merge_layer
            .resolve(&iri(patient_iri))
            .expect("Patient should resolve to B's body");
        assert_eq!(
            resolved
                .get(&iri("urn:project:label"))
                .and_then(|v| v.as_str()),
            Some("B"),
        );
        // `billing:Patient` post-merge: A's renamed body.
        let renamed = merge_layer
            .resolve(&iri(renamed_iri))
            .expect("renamed body should be reachable");
        assert_eq!(
            renamed
                .get(&iri("urn:project:label"))
                .and_then(|v| v.as_str()),
            Some("A"),
        );
    }

    #[test]
    fn commit_rename_rewrites_external_references() {
        // Branch A contributes both `Patient` (the rename target)
        // and `Profile` (referencing `Patient`). Branch B also has
        // `Patient` with a different body (so the classifier
        // surfaces IriCollision and the rename can target it).
        // After the rename → `billing:Patient`, the post-merge
        // `Profile` body must carry the rewritten reference.
        let patient_iri = "urn:project:Patient";
        let renamed_iri = "urn:project:billing:Patient";
        let profile_iri = "urn:project:profile";
        let profile_for_iri = "urn:project:profile_for";

        let patient_a = make_resource(
            patient_iri,
            &[wk::CLASS],
            &[("urn:project:label", Value::String("A".to_string()))],
        );
        let patient_b = make_resource(
            patient_iri,
            &[wk::CLASS],
            &[("urn:project:label", Value::String("B".to_string()))],
        );
        let profile = make_resource(
            profile_iri,
            &[wk::CLASS],
            &[(profile_for_iri, Value::ResourceRef(iri(patient_iri)))],
        );
        let (span, backend, storage) =
            build_span_arc(Vec::new(), vec![patient_a, profile], vec![patient_b]);
        let conflicts = classify_conflicts(&span, &*backend).unwrap();
        let conflict_id = conflicts[0].id.clone();

        // Cascade preview will surface an orphaned ref against
        // `Profile` (the rename apply doesn't walk branch B's
        // contributions — but here the ref lives on the renamed
        // side, which IS walked, so the cascade item comes from
        // branch B's *unmodified* view of Profile in the
        // ancestor-chain walk. Cover by acking it.
        let resolutions = vec![MergeResolution::Rename {
            conflict: conflict_id,
            side: Side::A,
            old_iri: iri(patient_iri),
            new_iri: iri(renamed_iri),
        }];
        let preview = preview_cascade(&span, &resolutions, &*backend).unwrap();
        let acks: Vec<CascadeAck> = preview
            .item_ids()
            .into_iter()
            .map(|item_id| CascadeAck { item_id })
            .collect();
        let merge_layer = commit_resolutions_as_merge_layer(
            &span,
            &resolutions,
            &acks,
            "merge:rename_rewrite",
            &[],
            storage,
            &*backend,
        )
        .expect("Rename with rewrite should commit cleanly");

        // Profile's reference now points at the renamed IRI.
        let resolved_profile = merge_layer
            .resolve(&iri(profile_iri))
            .expect("Profile should resolve");
        match resolved_profile.get(&iri(profile_for_iri)) {
            Some(Value::ResourceRef(r)) => {
                assert_eq!(
                    r.as_str(),
                    renamed_iri,
                    "Profile.profile_for should point at the renamed IRI"
                );
            }
            other => panic!("expected ResourceRef to renamed IRI, got {other:?}"),
        }
    }

    // ─── Tombstones (15g step 3) ───────────────────────────────────────────

    #[test]
    fn keep_neither_tombstones_iri_when_ancestor_absent() {
        // Both branches add `urn:test:X` (different bodies → IriCollision),
        // ancestor has nothing. KeepNeither produces a merge layer
        // that tombstones the conflict IRI so neither branch's body
        // is reachable post-merge.
        let body_a = make_resource(
            "urn:test:X",
            &["urn:test:Thing"],
            &[("urn:test:weight", Value::Integer(1))],
        );
        let body_b = make_resource(
            "urn:test:X",
            &["urn:test:Thing"],
            &[("urn:test:weight", Value::Integer(2))],
        );
        let (span, backend, storage) = build_span_arc(Vec::new(), vec![body_a], vec![body_b]);
        let conflicts = classify_conflicts(&span, &*backend).unwrap();
        assert_eq!(conflicts.len(), 1);
        let conflict_id = conflicts[0].id.clone();

        let resolutions = vec![MergeResolution::SchemaQuotient {
            conflict: conflict_id,
            quotient: SchemaQuotient::KeepNeither,
        }];
        let merge_layer = commit_resolutions_as_merge_layer(
            &span,
            &resolutions,
            &[],
            "merge:keep_neither",
            &[],
            storage,
            &*backend,
        )
        .expect("KeepNeither should commit cleanly");

        // The merge layer's tombstone shadows both branches' bodies.
        assert!(merge_layer.tombstoned_iris().contains(&iri("urn:test:X")));
        assert!(merge_layer.resolve(&iri("urn:test:X")).is_none());
    }

    #[test]
    fn keep_neither_restores_ancestor_body_when_present() {
        // Ancestor has `urn:test:X` (the canonical pre-divergence
        // body). Both branches modified it. KeepNeither produces a
        // merge layer that commits ANCESTOR's body — overriding both
        // branches' bodies via merge-layer precedence. No tombstone
        // because the canonical body resolves to ancestor's version.
        let ancestor_body = make_resource(
            "urn:test:X",
            &["urn:test:Thing"],
            &[("urn:test:weight", Value::Integer(0))],
        );
        let body_a = make_resource(
            "urn:test:X",
            &["urn:test:Thing"],
            &[("urn:test:weight", Value::Integer(1))],
        );
        let body_b = make_resource(
            "urn:test:X",
            &["urn:test:Thing"],
            &[("urn:test:weight", Value::Integer(2))],
        );
        let (span, backend, storage) =
            build_span_arc(vec![ancestor_body], vec![body_a], vec![body_b]);
        let conflicts = classify_conflicts(&span, &*backend).unwrap();
        let conflict_id = conflicts[0].id.clone();

        let resolutions = vec![MergeResolution::SchemaQuotient {
            conflict: conflict_id,
            quotient: SchemaQuotient::KeepNeither,
        }];
        let merge_layer = commit_resolutions_as_merge_layer(
            &span,
            &resolutions,
            &[],
            "merge:keep_neither_ancestor",
            &[],
            storage,
            &*backend,
        )
        .expect("KeepNeither with ancestor body should commit cleanly");

        assert!(merge_layer.tombstoned_iris().is_empty());
        let resolved = merge_layer
            .resolve(&iri("urn:test:X"))
            .expect("ancestor body should be reachable");
        assert_eq!(
            resolved.get(&iri("urn:test:weight")),
            Some(&Value::Integer(0)),
            "merged body should match the ancestor's"
        );
    }

    #[test]
    fn keep_one_commits_winner_body() {
        // KeepOne winner=A commits A's body at the conflict IRI; the
        // merge-layer-precedence rule means resolve sees A's value
        // even though both branches modified the IRI.
        let body_a = make_resource(
            "urn:test:X",
            &["urn:test:Thing"],
            &[("urn:test:weight", Value::Integer(1))],
        );
        let body_b = make_resource(
            "urn:test:X",
            &["urn:test:Thing"],
            &[("urn:test:weight", Value::Integer(2))],
        );
        let (span, backend, storage) = build_span_arc(Vec::new(), vec![body_a], vec![body_b]);
        let conflicts = classify_conflicts(&span, &*backend).unwrap();
        let conflict_id = conflicts[0].id.clone();

        let resolutions = vec![MergeResolution::SchemaQuotient {
            conflict: conflict_id,
            quotient: SchemaQuotient::KeepOne { winner: Side::A },
        }];
        let merge_layer = commit_resolutions_as_merge_layer(
            &span,
            &resolutions,
            &[],
            "merge:keep_one",
            &[],
            storage,
            &*backend,
        )
        .expect("KeepOne winner=A should commit cleanly");
        let resolved = merge_layer
            .resolve(&iri("urn:test:X"))
            .expect("winner body should be reachable");
        assert_eq!(
            resolved.get(&iri("urn:test:weight")),
            Some(&Value::Integer(1)),
            "merge layer should expose branch A's body"
        );
        assert!(
            merge_layer.tombstoned_iris().is_empty(),
            "KeepOne shouldn't tombstone — the winner's body is present"
        );
    }

    #[test]
    fn merge_commits_non_conflict_contributions_from_both_branches() {
        // Branch A adds `urn:test:OnlyA`; branch B adds `urn:test:OnlyB`
        // (non-overlapping). Both branches also modify `urn:test:X`
        // (the conflict the resolution targets). The merge layer's
        // resolve must see all three IRIs — without committing
        // non-conflict contributions, branch B's OnlyB would be
        // unreachable through the merge layer (the resolve walker
        // only follows `parents.first()` = branch A).
        let only_a = make_resource(
            "urn:test:OnlyA",
            &["urn:test:Thing"],
            &[("urn:test:label", Value::String("a".into()))],
        );
        let only_b = make_resource(
            "urn:test:OnlyB",
            &["urn:test:Thing"],
            &[("urn:test:label", Value::String("b".into()))],
        );
        let conflict_a = make_resource(
            "urn:test:X",
            &["urn:test:Thing"],
            &[("urn:test:weight", Value::Integer(1))],
        );
        let conflict_b = make_resource(
            "urn:test:X",
            &["urn:test:Thing"],
            &[("urn:test:weight", Value::Integer(2))],
        );
        let (span, backend, storage) = build_span_arc(
            Vec::new(),
            vec![only_a, conflict_a],
            vec![only_b, conflict_b],
        );
        let conflicts = classify_conflicts(&span, &*backend).unwrap();
        let conflict_id = conflicts[0].id.clone();
        let resolutions = vec![MergeResolution::SchemaQuotient {
            conflict: conflict_id,
            quotient: SchemaQuotient::KeepOne { winner: Side::A },
        }];
        let merge_layer = commit_resolutions_as_merge_layer(
            &span,
            &resolutions,
            &[],
            "merge:full_view",
            &[],
            storage,
            &*backend,
        )
        .expect("merge should commit");

        assert!(
            merge_layer.resolve(&iri("urn:test:OnlyA")).is_some(),
            "branch A's unique contribution should be reachable"
        );
        assert!(
            merge_layer.resolve(&iri("urn:test:OnlyB")).is_some(),
            "branch B's unique contribution should be reachable through the merge layer"
        );
        assert!(merge_layer.resolve(&iri("urn:test:X")).is_some());
    }

    #[test]
    fn commit_rejects_unresolved_conflict() {
        // Two conflicts surface; only one resolution submitted. The
        // commit path rejects rather than producing a partial outcome.
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
        let other_prop_a = make_resource(
            "urn:test:height",
            &[wk::PROPERTY],
            &[(wk::DATA_TYPE, Value::ResourceRef(iri(wk::INTEGER)))],
        );
        let other_prop_b = make_resource(
            "urn:test:height",
            &[wk::PROPERTY],
            &[(wk::DATA_TYPE, Value::ResourceRef(iri(wk::STRING)))],
        );
        let (span, backend, storage) = build_span_arc(
            Vec::new(),
            vec![prop_a, other_prop_a],
            vec![prop_b, other_prop_b],
        );
        let conflicts = classify_conflicts(&span, &*backend).unwrap();
        assert_eq!(conflicts.len(), 2);
        // Resolve only the first one.
        let resolutions = vec![MergeResolution::SchemaQuotient {
            conflict: conflicts[0].id.clone(),
            quotient: SchemaQuotient::KeepOne { winner: Side::A },
        }];
        let result = commit_resolutions_as_merge_layer(
            &span,
            &resolutions,
            &[],
            "merge:partial",
            &[],
            storage,
            &*backend,
        );
        match result {
            Err(MergeError::UnresolvedConflict { conflict_id }) => {
                assert_eq!(conflict_id, conflicts[1].id);
            }
            other => panic!("expected UnresolvedConflict, got {other:?}"),
        }
    }
}
