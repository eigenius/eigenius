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

//! Pipeline / orchestrator outcome shapes.
//!
//! - [`LayerCommitOutcome`] — one per pipeline run, returned by
//!   `CommitPipeline::run`.
//! - [`MultiLayerOutcome`] — one per orchestrator run, returned by
//!   `CommitOrchestrator::run`. Carries the per-layer outcomes plus any
//!   `didDrain` hook errors.
//! - [`LayerEmission`] — the unit of work the orchestrator drains. The
//!   root emission represents the RPC's primary layer; phases / hooks
//!   may queue further emissions as follow-up layers (verdict
//!   provenance, institution-classes, etc).
//! - [`DispatchEntry`] — one institution dispatch reading. Internal
//!   shape; the design doc deliberately leaves this open. Phase A
//!   provides a minimal record carrying the subject IRI, the queried
//!   QueryClass IRI, and the [`crate::institution::dispatch::VerdictReading`].
//!
//! See D41 §3 (`HookOutcome` is in `hooks.rs`), §4 (`CommitState`),
//! §6 (`LayerEmission`), §11 (`LayerCommitOutcome`).

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::institution::dispatch::VerdictReading;
use crate::layer::{Layer, LayerBuilder};
use crate::ontology::{Iri, Resource};
use crate::validation::ValidationError;

use super::persister::PersistedLayerInfo;
use super::pipeline::PipelineKind;

/// Kernel taxonomy of what produced a layer in a commit. Closed set:
/// every emission site in the kernel emits one of these. The proto
/// surface uses this enum directly (wire-stable). Free-form layer
/// names live separately on [`LayerEmission::name`] / [`Layer::name`]
/// for human display.
///
/// Use this — not a string compare on [`LayerEmission::name`] — to
/// route per-layer response shaping in handlers. The Sibling-rescue
/// path (the failing user-layer pipeline pushes nothing to
/// `MultiLayerOutcome.layers`, so `layers[0]` becomes the rescued
/// audit layer) makes position-in-vec assumptions unsafe; the role
/// is the structural discriminator that always identifies the
/// right entry.
///
/// D41 §6 / §3.4 / §3.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayerRole {
    /// The RPC caller's content — the root emission of any
    /// commit-shaped RPC (Load, Reflect, RunProgram, Query INTO, etc.).
    /// The orchestrator's [`MultiLayerOutcome::layers`] always has at
    /// most one of these.
    User,
    /// Audit Sibling emitted by `autoonload_dispatch` whenever any
    /// AutoOnLoad verdict (Holds, Undecidable, Fails) was produced.
    /// D31 §6.3 / D41 §3.4.
    AuditProvenance,
    /// Follow-up Child role for institution-class resources contributed
    /// by an institution-registration pass. Not emitted by any current
    /// backend — external and in-process institutions declare their
    /// classes statically in ontologies — but retained in the role
    /// taxonomy and wire protocol. D41 §3.6.
    InstitutionClasses,
}

/// Outcome of a single `CommitPipeline::run`.
///
/// One per layer landed (or attempted) in an orchestrator run. The
/// orchestrator's [`MultiLayerOutcome`] is a `Vec<LayerCommitOutcome>`
/// plus drain-hook accumulators.
///
/// D41 §4 / §11.
#[derive(Debug)]
pub struct LayerCommitOutcome {
    /// Kernel-internal role taxonomy of this layer. The closed
    /// [`LayerRole`] set is what handlers and proto consumers should
    /// match on; see the comment on [`LayerRole`] for why
    /// position-in-vec mapping is unsafe on the Sibling-rescue path.
    ///
    /// D41 §6.
    pub role: LayerRole,
    /// Free-form diagnostic / display name carried over from the
    /// originating [`LayerEmission::name`]. Useful for tracing and
    /// for surface-level layer labels (notebook layer-stack
    /// visualisation, audit log entries) but **not** for role
    /// identification — use [`Self::role`] for that.
    ///
    /// D41 §6.
    pub name: &'static str,
    /// The layer the `build` phase materialised. Identical across the
    /// outcome for cache-hit and CAS-loss paths — those are reflected
    /// in `persist.branch_advanced`, not by a different `layer`.
    pub layer: Arc<Layer>,
    /// Result of the `persist` phase. Drives the orchestrator's
    /// drain / revert decision and the `didPersist` hook gate.
    pub persist: PersistedLayerInfo,
    /// IRIs the cascade tombstoned beyond the caller's builder-level
    /// tombstones. Always empty for pipelines without
    /// `retroactive_with_cascade`.
    pub cascade_tombstones: BTreeSet<Iri>,
    /// Number of cascade fixpoint iterations. `0` if the phase didn't
    /// run or found no retroactive violations.
    pub cascade_iterations: u32,
    /// Per-subject institution dispatch readings collected by
    /// `autoonload_dispatch`. Empty for pipelines without that phase.
    pub dispatched_verdicts: Vec<DispatchEntry>,
    /// Follow-up emissions queued by phases / `didPersist` hooks. The
    /// orchestrator drains these in FIFO order; see D41 §6.2.
    pub emissions: Vec<LayerEmission>,
    /// Non-unwinding errors raised by `didPersist` hooks. The commit
    /// stands — the layer is on disk — but callers can surface these.
    /// See D41 §3.6.
    pub hook_errors: Vec<ValidationError>,
}

/// Outcome of an orchestrator drain.
///
/// `layers` holds one [`LayerCommitOutcome`] per pipeline run, in the
/// order they drained (root → user-emitted children → hook-emitted
/// children, FIFO). `drain_hook_errors` collects non-unwinding errors
/// raised by `didDrain` hooks; see D41 §6.5.
///
/// **`error` shape (D41 Phase E).** The orchestrator returns
/// `MultiLayerOutcome` unconditionally — `error: None` is the all-Ok
/// path, `error: Some(_)` is the path where one of the pipeline runs
/// returned `Err`. The Err path is structurally distinct from a Sibling
/// rescue: when the user-layer pipeline returns `Err` but
/// `autoonload_dispatch` had queued a `verdict_provenance` Sibling
/// before failing, the orchestrator drains the Sibling, advances
/// `ctx.head` to it, and surfaces both facts to the caller — the
/// audit anchor in `layers` and the user-layer rejection in `error`.
///
/// Returning a single struct in both arms (instead of
/// `Result<MultiLayerOutcome, CommitError>`) makes that
/// "rejected-but-audited" shape representable: the handler can render
/// `success = false`, surface the error to the caller, *and* report
/// `branch_advanced = true` against the audit layer that landed.
/// Without the unified struct the audit's persist info has no place
/// to ride out the Err arm.
///
/// D41 §6 / Phase E.
#[derive(Debug)]
pub struct MultiLayerOutcome {
    /// Per-layer outcomes, in drain order. May contain entries even
    /// when `error` is `Some(_)` — the Sibling rescue path lands
    /// audit-anchor layers regardless of the surfaced error.
    pub layers: Vec<LayerCommitOutcome>,
    /// Non-unwinding errors from `didDrain` hooks. Surfaced to the
    /// caller; all layers in `layers` are durably on disk regardless.
    pub drain_hook_errors: Vec<ValidationError>,
    /// `None` on the all-Ok path. `Some(_)` if any pipeline run
    /// returned `Err`; the orchestrator records the *first* such
    /// error and continues draining to land any rescued Sibling
    /// emissions before returning the outcome to the caller.
    pub error: Option<crate::lattice::CommitError>,
}

/// Routing-kind discriminator on a [`LayerEmission`].
///
/// Distinguishes two follow-up-layer drain modes:
///
/// - [`EmissionKind::Child`] — the default. The emission is queued
///   only if its queuing pipeline succeeded with `branch_advanced`;
///   on `Err` or `!branch_advanced` it is silently dropped because
///   its parent (the queuing pipeline's layer) is not on disk.
/// - [`EmissionKind::Sibling`] — drained unconditionally, including
///   when the queuing pipeline returned `Err`. The orchestrator
///   re-queues rescued Siblings at depth 0 parented at the pre-run
///   head. Used for audit-anchor content (today: AutoOnLoad
///   `verdict_provenance`) that must land regardless of whether the
///   gated commit succeeded.
///
/// D41 §6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmissionKind {
    /// Drain iff the queuing pipeline succeeded with `branch_advanced`.
    Child,
    /// Drain unconditionally, even on `Err`. Re-rooted at `ctx.head`
    /// on the rescue path (§6.1).
    Sibling,
}

/// The unit of work the orchestrator drains.
///
/// The root emission represents the RPC's primary layer; phases /
/// hooks may push additional emissions onto `state.emissions` for
/// follow-up layers (verdict provenance after AutoOnLoad dispatch,
/// etc.).
///
/// `name` is a stable, static string used both for diagnostics and as
/// the `LayerBuilder` name when the orchestrator constructs a builder
/// for this emission. See D41 §6.
#[derive(Debug, Clone)]
pub struct LayerEmission {
    /// Kernel-internal role taxonomy. Drives proto routing and
    /// handler-side per-layer response shaping. See [`LayerRole`].
    pub role: LayerRole,
    /// Stable, diagnostic name (`"user"`, `"verdict_provenance"`,
    /// `"institution_classes"`, ...). Used as the builder name and
    /// surfaced to operators in logs / traces, but **not** the
    /// identifier handlers match on — that's [`Self::role`].
    pub name: &'static str,
    /// Which canned pipeline to run on this emission.
    pub pipeline: PipelineKind,
    /// Routing kind — Child (drop on parent failure) or Sibling
    /// (always-drain). See [`EmissionKind`].
    pub kind: EmissionKind,
    /// Resources to add to the emission's `LayerBuilder`. Followup
    /// emissions populate this from phase / hook output; the root
    /// emission populates it from the RPC request.
    pub resources: Vec<Resource>,
    /// Tombstones to apply to the emission's `LayerBuilder`. Followup
    /// emissions populate this from phase / hook output; the root
    /// emission populates it from the RPC's explicit tombstones
    /// (D41 §10.1).
    pub tombstones: BTreeSet<Iri>,
}

impl LayerEmission {
    /// Build a root [`LayerEmission`] from a populated [`LayerBuilder`].
    ///
    /// Handler callers (the gRPC Load / RunProgram / Reflect / Query
    /// INTO / SubmitResolution / CapabilityInstall handlers — D41 §10)
    /// use this to convert their accumulated working builder into the
    /// orchestrator's root emission. The builder's resources and
    /// tombstones are extracted by clone; the builder itself is
    /// consumed.
    ///
    /// `role`, `name`, `pipeline`, `kind` are caller-specified — the
    /// builder's own name is discarded because the orchestrator will
    /// rename the emission's builder back to `name` when it
    /// materialises (see [`Self::materialize`]). Root emissions are
    /// always [`EmissionKind::Child`] with [`LayerRole::User`]; the
    /// `kind` parameter is plumbed for symmetry with the
    /// constructed-by-phase emission shape.
    ///
    /// D41 §6 / Phase E.
    pub fn from_builder(
        role: LayerRole,
        name: &'static str,
        pipeline: PipelineKind,
        kind: EmissionKind,
        builder: LayerBuilder,
    ) -> Self {
        // LayerBuilder exposes `resources()` (by reference,
        // `&BTreeMap<Iri, Resource>`) and `tombstoned_iris()`
        // (`&BTreeSet<Iri>`). Both are private fields with no
        // `into_parts`-style consumer, so we clone. The volumes are
        // bounded by the originating RPC's payload — handler-side
        // builders never accumulate more than one batch's worth of
        // resources before the orchestrator runs.
        let resources: Vec<Resource> = builder.resources().values().cloned().collect();
        let tombstones: BTreeSet<Iri> = builder.tombstoned_iris().clone();
        Self {
            role,
            name,
            pipeline,
            kind,
            resources,
            tombstones,
        }
    }

    /// Materialise a fresh [`LayerBuilder`] for this emission, parented
    /// at `parent`. Adds the emission's resources and tombstones; the
    /// orchestrator passes the result into [`super::CommitPipeline::run`].
    ///
    /// Resource / tombstone insertion errors are surfaced as
    /// [`crate::lattice::CommitError::Layer`] so the orchestrator's
    /// drain loop can treat them uniformly with other commit-phase
    /// errors. The emission is consumed by this call.
    ///
    /// D41 §6.1.
    pub fn materialize(
        self,
        parent: &Arc<Layer>,
    ) -> Result<LayerBuilder, crate::lattice::CommitError> {
        let mut builder = LayerBuilder::new(self.name, Some(Arc::clone(parent)));
        for resource in self.resources {
            builder
                .add_resource(resource)
                .map_err(crate::lattice::CommitError::Layer)?;
        }
        for iri in self.tombstones {
            builder
                .tombstone(iri)
                .map_err(crate::lattice::CommitError::Layer)?;
        }
        Ok(builder)
    }
}

/// One institution dispatch reading collected by the
/// `autoonload_dispatch` phase.
///
/// The design doc deliberately leaves the interior shape open
/// (D41 §10 lists this as something the handler translates into
/// the response). Phase A picks a minimal record matching how the
/// handler already surfaces dispatch outcomes. Phase B / D may widen
/// it (e.g. to carry runtime invocation provenance) without breaking
/// the pipeline contract.
#[derive(Debug, Clone)]
pub struct DispatchEntry {
    /// IRI of the resource the gate was evaluated against. `None` only
    /// for gates that target whole-layer predicates (none exist today).
    pub subject_iri: Option<Iri>,
    /// IRI of the QueryClass that produced this reading. Stored as a
    /// `String` because the dispatch surface today (`InstitutionContext`
    /// snapshots) keeps it as a string; Phase B will move to `Iri` if
    /// the dispatch surface migrates.
    pub query_class_iri: String,
    /// Reading off the dispatch result resource.
    pub verdict: VerdictReading,
}

#[cfg(test)]
mod tests {
    //! D41 Phase F.5 — `LayerEmission::materialize` and
    //! `LayerEmission::from_builder` round-trip coverage.

    use super::*;
    use crate::layer::LayerStorage;
    use crate::ontology::resource::Value;
    use crate::ontology::well_known;

    /// Build a minimal root core layer to serve as parent for emission
    /// materialize / from_builder tests. The parent layer's content is
    /// irrelevant to these tests — only its identity matters because
    /// `LayerBuilder::new` clones an `Arc<Layer>` reference.
    fn build_root_layer() -> Arc<Layer> {
        let storage = LayerStorage::in_memory();
        let builder = LayerBuilder::new("root", None);
        Arc::new(builder.build(storage))
    }

    /// Build a child-namespace resource with `is_a = Class` (passes the
    /// add_resource path; emission tests don't care about validation).
    fn make_resource(local: &str) -> Resource {
        let mut r = Resource::new(Iri::parse(&format!("urn:eigenius:user:{local}")).unwrap());
        r.set(
            Iri::parse(well_known::IS_A).unwrap(),
            Value::Array(vec![Value::String(well_known::CLASS.into())]),
        );
        r
    }

    #[test]
    fn layer_emission_materialize_constructs_builder_with_resources_and_tombstones() {
        // Three resources, two tombstones (both in user namespace so
        // `tombstone` doesn't reject as core-namespace).
        let parent = build_root_layer();
        let resources = vec![
            make_resource("alpha"),
            make_resource("beta"),
            make_resource("gamma"),
        ];
        let mut tombstones = BTreeSet::new();
        tombstones.insert(Iri::parse("urn:eigenius:user:dead1").unwrap());
        tombstones.insert(Iri::parse("urn:eigenius:user:dead2").unwrap());

        let emission = LayerEmission {
            role: LayerRole::User,
            name: "test_emission",
            pipeline: PipelineKind::StructuralFollowup,
            kind: EmissionKind::Child,
            resources: resources.clone(),
            tombstones: tombstones.clone(),
        };

        let builder = emission
            .materialize(&parent)
            .expect("materialize on well-formed inputs succeeds");

        // Resources by IRI.
        assert_eq!(builder.resources().len(), 3);
        for r in &resources {
            let iri = r.id().expect("resources have IDs");
            assert!(
                builder.has_resource(iri),
                "builder must carry resource {iri}"
            );
        }
        // Tombstones equal.
        assert_eq!(builder.tombstoned_iris(), &tombstones);
    }

    #[test]
    fn layer_emission_materialize_handles_empty_resources_and_tombstones() {
        let parent = build_root_layer();
        let emission = LayerEmission {
            role: LayerRole::User,
            name: "empty",
            pipeline: PipelineKind::StructuralFollowup,
            kind: EmissionKind::Child,
            resources: Vec::new(),
            tombstones: BTreeSet::new(),
        };
        let builder = emission
            .materialize(&parent)
            .expect("materialize empty emission");
        assert!(builder.resources().is_empty());
        assert!(builder.tombstoned_iris().is_empty());
    }

    #[test]
    fn layer_emission_from_builder_round_trip() {
        let parent = build_root_layer();
        // Construct a builder with content; convert to emission and
        // back via materialize.
        let mut original = LayerBuilder::new("source", Some(Arc::clone(&parent)));
        let r1 = make_resource("alpha");
        let r2 = make_resource("beta");
        original.add_resource(r1.clone()).unwrap();
        original.add_resource(r2.clone()).unwrap();
        let tomb = Iri::parse("urn:eigenius:user:tombed").unwrap();
        original.tombstone(tomb.clone()).unwrap();

        let original_resources: BTreeSet<Iri> = original.resources().keys().cloned().collect();
        let original_tombstones = original.tombstoned_iris().clone();

        let emission = LayerEmission::from_builder(
            LayerRole::User,
            "round_trip",
            PipelineKind::WithRetroactive,
            EmissionKind::Child,
            original,
        );

        // Emission carries the same content.
        assert_eq!(emission.role, LayerRole::User);
        assert_eq!(emission.name, "round_trip");
        assert_eq!(emission.pipeline, PipelineKind::WithRetroactive);
        assert_eq!(emission.kind, EmissionKind::Child);
        assert_eq!(emission.resources.len(), 2);
        assert_eq!(emission.tombstones, original_tombstones);

        // Round-tripping through materialize recovers a builder with
        // the same IRI sets.
        let rebuilt = emission
            .materialize(&parent)
            .expect("materialize round-trip");
        let rebuilt_iris: BTreeSet<Iri> = rebuilt.resources().keys().cloned().collect();
        assert_eq!(rebuilt_iris, original_resources);
        assert_eq!(rebuilt.tombstoned_iris(), &original_tombstones);
    }
}
