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
//! Phase 15 sub-milestones in this module: 15a typed-conflict
//! scaffolding + classifier; 15b–15e six resolution strategies
//! (Witness, Rename, KeepBoth/KeepOne/KeepNeither schema-quotients,
//! Restructure); 15f cascade impact analysis with an ack gate; 15g
//! the multi-parent merge-layer construction, tombstone semantics,
//! gRPC surface (`SubmitResolution`, `PreviewCascade`), and CLI
//! wrappers. Every resolution's commit path produces a real merge
//! layer.
//!
//! ## Module layout
//!
//! - [`lca`] — chain-walking primitives shared by every variant.
//! - [`conflict`] — conflict taxonomy + classifier + `MergeSpan`.
//! - [`cascade`] — cascade impact analysis + ack gate.
//! - [`witnessed`] — Witness resolution (D20 §6.1).
//! - [`resolve`] — `MergeResolution` enum + Rename/SchemaQuotient/
//!   Restructure apply functions + merge-layer construction
//!   ([`commit_resolutions_as_merge_layer`], [`merge_with_resolutions`]).

pub mod cascade;
pub mod conflict;
pub mod lca;
pub mod resolve;
pub mod witnessed;

#[cfg(test)]
mod test_support;

// ─── Re-exports ────────────────────────────────────────────────────────────
//
// Preserve the pre-split surface: every `crate::layer::merge::X` path
// that resolved against the old single-file module continues to
// resolve through these re-exports.

pub use cascade::{
    preview_cascade, CascadeAck, CascadeItem, CascadeItemId, CascadePreview, PropertyPath,
};
pub use conflict::{
    build_merge_span, classify_conflicts, classify_iri_disagreement, detect_inheritance_cycles,
    ConflictId, ConflictKind, MergeOutcome, MergeSpan, ResourceBody, ResourceKind, Side,
    TypedConflict,
};
pub use resolve::{
    apply_quotient_resolution, apply_rename_resolution, apply_restructure_resolution,
    commit_resolutions_as_merge_layer, merge_with_resolutions, MergeResolution,
    QuotientApplication, RenameApplication, RenameCollisionSite, RestructureApplication,
    RestructureMissingRole, RestructureSpec, SchemaQuotient,
};
pub use witnessed::{apply_witness_resolution, resolve_merge_comorphism, MergeComorphismHandle};

// ─── Errors ────────────────────────────────────────────────────────────────

use crate::layer::LayerId;
use crate::ontology::iri::Iri;
use crate::storage::StorageError;

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
    /// a `ResourceRef` to a EigenTT term. Both shapes are required by
    /// the core ontology's class declaration; surfacing this as a
    /// typed error keeps the failure mode legible.
    MalformedMergeComorphism {
        iri: Iri,
        reason: String,
    },
    /// A `MergeComorphism` was applied to a conflict whose class
    /// doesn't match the comorphism's declared `merge_target_class`
    /// (D37 §6.2). The witness's transformation has signature
    /// `(A, A, Option<A>) -> A` where A is `merge_target_class` —
    /// applying it to a different class would fail downstream during
    /// term evaluation with an opaque diagnostic; this variant
    /// surfaces the mismatch up-front.
    MergeComorphismWrongClass {
        iri: Iri,
        expected: Iri,
        actual: Iri,
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
    /// Resource into a EigenTT `Exp`. The Resource is malformed
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
    /// The cascade preview surfaced items the user didn't
    /// acknowledge (D20 §8). The kernel refuses to commit a merge
    /// whose downstream consequences haven't been explicitly seen.
    IncompleteAcknowledgments {
        missing: Vec<CascadeItemId>,
    },
    /// A `Witness` resolution targets a conflict whose kind has no
    /// single IRI to merge at — `InheritanceCycle` and the reserved
    /// stage-2 kinds carry multi-IRI structure. Witnessing those
    /// requires a different resolution shape than the single
    /// `(A, A, Option<A>) → A` term.
    WitnessTargetNotResolvable {
        conflict_id: ConflictId,
    },
    /// The merge-layer construction path's `LayerBuilder` rejected a
    /// resource (e.g., missing `@id`, core-namespace violation). Wraps
    /// the underlying `LayerError` as a string to keep `MergeError`
    /// independent of the layer-builder error shape.
    LayerBuild(String),
    /// [`crate::layer::merge::build_merge_span`] couldn't find a
    /// common ancestor for the two heads — either they live on
    /// unrelated DAG roots, or one (or both) isn't present in the
    /// topology. Without an LCA there is no span to construct.
    NoCommonAncestor {
        head_a: LayerId,
        head_b: LayerId,
    },
    /// The classifier surfaced a conflict no resolution in the
    /// submitted list targets. Committing the merge would leave both
    /// branches' conflicting bodies in place — the resulting chain
    /// wouldn't satisfy the classifier post-merge. Callers must
    /// include a resolution per classified conflict.
    UnresolvedConflict {
        conflict_id: ConflictId,
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
            MergeError::MergeComorphismWrongClass {
                iri,
                expected,
                actual,
            } => write!(
                f,
                "MergeComorphism {iri} declared for class {actual} cannot be applied to a conflict on {expected}"
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
                "transformation {transformation} failed to parse as a EigenTT term: {reason}"
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
            MergeError::RestructureSynthesizedParent { new_parent } => write!(
                f,
                "Restructure new_parent {new_parent} uses the reserved `{}` namespace; user must name the new structure explicitly (D20 §6.4)",
                resolve::SYNTHESIZED_PARENT_PREFIX
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
            MergeError::IncompleteAcknowledgments { missing } => {
                let names: Vec<&str> = missing.iter().map(|m| m.0.as_str()).collect();
                write!(
                    f,
                    "cascade preview surfaced {} item(s) without acknowledgment: {}",
                    missing.len(),
                    names.join(", ")
                )
            }
            MergeError::WitnessTargetNotResolvable { conflict_id } => write!(
                f,
                "Witness resolution on conflict {} has no single-IRI merge target; this conflict kind needs a different resolution strategy",
                conflict_id.0
            ),
            MergeError::LayerBuild(reason) => {
                write!(f, "merge-layer builder rejected a resource: {reason}")
            }
            MergeError::NoCommonAncestor { head_a, head_b } => write!(
                f,
                "no common ancestor for heads {head_a} and {head_b}; cannot build a merge span"
            ),
            MergeError::UnresolvedConflict { conflict_id } => write!(
                f,
                "classified conflict {} has no matching resolution in the submitted list",
                conflict_id.0
            ),
        }
    }
}

impl std::error::Error for MergeError {}
