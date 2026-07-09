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

//! Shared proto-translation and small utility helpers used by the
//! per-domain RPC handler modules.
//!
//! Everything here is wire-shaped translation (proto ↔ kernel-internal
//! types), small wire-format encoders, and a handful of date/IRI
//! utilities. Anything used by 2+ handler files lives here. Handler
//! bodies live in their domain-specific siblings (load.rs, query.rs,
//! programs.rs, …).

use super::proto;
use super::proto::*;
use crate::commit::persister::PersistedLayerInfo;
use tonic::{Response, Status};

/// Current time in milliseconds since the Unix epoch. Used to stamp
/// `TaskRecord.{created_at, updated_at}`.
pub(super) fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Convert milliseconds since epoch to ISO 8601 string.
pub(super) fn millis_to_iso8601(ms: i64) -> String {
    use std::time::Duration;
    let d = Duration::from_millis(ms as u64);
    let secs = d.as_secs();
    let millis = d.subsec_millis();
    let days = secs / 86400;
    let rem = secs % 86400;
    let hours = rem / 3600;
    let minutes = (rem % 3600) / 60;
    let seconds = rem % 60;
    // Simple date calculation from days since epoch
    let (year, month, day) = days_to_ymd(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year, month, day, hours, minutes, seconds, millis
    )
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Civil calendar algorithm from Howard Hinnant
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1461 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Default branch name for requests that omit `branch`. Phase 14g.
pub const DEFAULT_BRANCH: &str = "main";

/// Resolve a request's branch field (empty → "main").
pub(super) fn resolve_branch_name(req_branch: &str) -> &str {
    if req_branch.is_empty() {
        DEFAULT_BRANCH
    } else {
        req_branch
    }
}

/// Build a wire-format [`proto::MergeInfo`] from an optional
/// [`PersistedLayerInfo`].
///
/// Resolves all post-persist states callers care about into the
/// proto's [`proto::MergeOutcome`] taxonomy:
///
/// - `None`: persist didn't run at all (commit attempted but errored
///   before reaching `persist_layer_if_backend`, e.g. backend I/O
///   error captured as a `ValidationError`). Emit `UNSPECIFIED`.
/// - **Different-position cache hit** (`info.cache_hit_different_position`):
///   `CACHED_DIFFERENT_POSITION` with `merge_layer_id = info.layer_id`
///   (the cached canonical layer's id). Branch did not advance.
/// - **Lattice CAS ran** (`info.merge_outcome = Some(_)`): map by
///   variant — FastForward / TrivialMerge / NeedsWitnessedMerge.
/// - **CAS skipped, no cache hit** (`info.merge_outcome = None`,
///   `!cache_hit_…`): the no-backend path. Emit `UNSPECIFIED`.
pub(super) fn merge_info_from_persist_info(info: Option<&PersistedLayerInfo>) -> proto::MergeInfo {
    use crate::lattice::UpdateOutcome;
    let Some(info) = info else {
        return proto::MergeInfo {
            outcome: proto::MergeOutcome::Unspecified as i32,
            merge_layer_id: String::new(),
            conflicting_iris: Vec::new(),
            current_head: String::new(),
            orphan_layer_id: String::new(),
        };
    };
    if info.cache_hit_different_position {
        return proto::MergeInfo {
            outcome: proto::MergeOutcome::CachedDifferentPosition as i32,
            merge_layer_id: info.layer_id.to_string(),
            conflicting_iris: Vec::new(),
            current_head: String::new(),
            orphan_layer_id: String::new(),
        };
    }
    match info.merge_outcome.as_ref() {
        None => proto::MergeInfo {
            outcome: proto::MergeOutcome::Unspecified as i32,
            merge_layer_id: String::new(),
            conflicting_iris: Vec::new(),
            current_head: String::new(),
            orphan_layer_id: String::new(),
        },
        Some(UpdateOutcome::FastForward) => proto::MergeInfo {
            outcome: proto::MergeOutcome::FastForward as i32,
            merge_layer_id: String::new(),
            conflicting_iris: Vec::new(),
            current_head: String::new(),
            orphan_layer_id: String::new(),
        },
        Some(UpdateOutcome::TrivialMerge { merge_layer }) => proto::MergeInfo {
            outcome: proto::MergeOutcome::TrivialMerge as i32,
            merge_layer_id: merge_layer.to_string(),
            conflicting_iris: Vec::new(),
            current_head: String::new(),
            orphan_layer_id: String::new(),
        },
        Some(UpdateOutcome::NeedsWitnessedMerge {
            current_head,
            conflicting_iris,
            orphan_head,
        }) => proto::MergeInfo {
            outcome: proto::MergeOutcome::NeedsWitnessedMerge as i32,
            merge_layer_id: String::new(),
            conflicting_iris: conflicting_iris
                .iter()
                .map(|iri| iri.as_str().to_string())
                .collect(),
            current_head: current_head.to_string(),
            orphan_layer_id: orphan_head.to_string(),
        },
    }
}

/// Convert a kernel-internal `ValidationError` to the proto-side one.
///
/// Used by the Load
/// handler (and other commit-shaped RPC handlers) to surface the
/// [`crate::commit::CommitOrchestrator`]'s per-layer / drain-hook /
/// pipeline errors back to gRPC clients.
///
/// D41 Phase E.
pub(super) fn kernel_validation_error_to_proto(
    err: &crate::validation::ValidationError,
) -> ValidationError {
    ValidationError {
        resource_iri: err
            .resource_id
            .as_ref()
            .map(|i| i.as_str().to_string())
            .unwrap_or_default(),
        property_iri: err
            .property
            .as_ref()
            .map(|i| i.as_str().to_string())
            .unwrap_or_default(),
        rule: format!("{:?}", err.rule),
        message: err.message.clone(),
        severity: "error".to_string(),
    }
}

/// Translate a [`crate::commit::CommitError`] to a list of proto
/// `ValidationError`s for surfacing to the gRPC caller.
///
/// The mapping per variant follows D41 Phase E:
///
/// - `Validation { errors, .. }` / `CascadeAbort { errors, .. }` —
///   the kernel-internal `ValidationError`s translate one-for-one.
/// - `Storage` / `Layer` / `WorkingSetExhausted` / `Persist` — wrap
///   the `Display` impl into a synthetic `ValidationError` with rule
///   `commit_error` so callers can distinguish it from a domain
///   rejection.
/// - `EmissionDepthExceeded { name, depth }` — synthetic
///   `ValidationError` with rule `emission_depth`. This should never
///   fire in practice today; surface it cleanly anyway.
///
/// D41 Phase E.
pub(super) fn commit_error_to_proto(err: &crate::commit::CommitError) -> Vec<ValidationError> {
    use crate::commit::CommitError;
    match err {
        CommitError::Validation { errors, .. } | CommitError::CascadeAbort { errors, .. } => {
            errors.iter().map(kernel_validation_error_to_proto).collect()
        }
        CommitError::Storage(_)
        | CommitError::Layer(_)
        | CommitError::WorkingSetExhausted(_)
        | CommitError::Persist(_) => vec![ValidationError {
            resource_iri: String::new(),
            property_iri: String::new(),
            rule: "commit_error".to_string(),
            message: err.to_string(),
            severity: "error".to_string(),
        }],
        CommitError::EmissionDepthExceeded { depth, layer_name } => vec![ValidationError {
            resource_iri: String::new(),
            property_iri: String::new(),
            rule: "emission_depth".to_string(),
            message: format!(
                "commit orchestrator: emission {layer_name:?} exceeded MAX_EMISSION_DEPTH at depth {depth}"
            ),
            severity: "error".to_string(),
        }],
    }
}

/// Extract the policy's `total_violations` from a [`crate::commit::CommitError`].
///
/// Only the `Validation` and `CascadeAbort` variants carry the true
/// violation count (D41 §3.3); every other variant resolves to `0`
/// because the failure didn't originate in the retroactive pass.
///
/// D41 §10.
pub(super) fn commit_error_total_violations(err: &crate::commit::CommitError) -> u32 {
    use crate::commit::CommitError;
    match err {
        CommitError::Validation {
            total_violations, ..
        }
        | CommitError::CascadeAbort {
            total_violations, ..
        } => *total_violations as u32,
        _ => 0,
    }
}

/// Translate a proto [`proto::CommitPolicy`] into the kernel's
/// [`crate::lattice::CommitPolicy`].
///
/// Mapping (D41 §3.3, §8):
/// - `None` or unset variant → [`CommitPolicy::default()`] (Reject{100}).
/// - `Reject { max_violations: 0 }` → `Reject { max_violations: 100 }` —
///   treat `0` as "use the kernel default" so clients can leave the
///   field at its proto zero-value and still get the default cap.
/// - `Reject { max_violations: n }` → `Reject { max_violations: n as usize }`.
/// - `CascadeTombstone` → `CascadeTombstone`.
///
/// D41 §10.1.
pub(super) fn commit_policy_from_proto(
    policy: Option<&proto::CommitPolicy>,
) -> crate::lattice::CommitPolicy {
    use crate::lattice::CommitPolicy;
    use proto::commit_policy::Variant;
    let Some(p) = policy else {
        return CommitPolicy::default();
    };
    match p.variant.as_ref() {
        None => CommitPolicy::default(),
        Some(Variant::Reject(r)) => {
            if r.max_violations == 0 {
                CommitPolicy::default()
            } else {
                CommitPolicy::Reject {
                    max_violations: r.max_violations as usize,
                }
            }
        }
        Some(Variant::CascadeTombstone(_)) => CommitPolicy::CascadeTombstone,
    }
}

/// Translate a [`crate::commit::LayerCommitOutcome`] into a proto
/// [`proto::CommittedLayer`] entry.
///
/// The `role` field maps the closed kernel
/// [`crate::commit::LayerRole`] taxonomy onto the wire-stable
/// [`proto::LayerRole`] enum; this is the field clients should match
/// on to identify the user / audit / institution-classes layers
/// (position-in-vec is unsafe on the Sibling-rescue path, and
/// string-comparing `name` is fragile). The `name` field carries the
/// free-form display label from the originating
/// [`crate::commit::LayerEmission::name`].
///
/// D41 §6 / §10.
pub(super) fn committed_layer_to_proto(
    outcome: &crate::commit::LayerCommitOutcome,
) -> proto::CommittedLayer {
    proto::CommittedLayer {
        role: layer_role_to_proto(outcome.role) as i32,
        name: outcome.name.to_string(),
        layer_id: outcome.persist.layer_id.to_string(),
        branch_advanced: outcome.persist.branch_advanced,
        merge: Some(merge_info_from_persist_info(Some(&outcome.persist))),
        cascade_tombstones: outcome
            .cascade_tombstones
            .iter()
            .map(|iri| iri.as_str().to_string())
            .collect(),
        cascade_iterations: outcome.cascade_iterations,
    }
}

/// Map the kernel-internal [`crate::commit::LayerRole`] onto the
/// wire-stable [`proto::LayerRole`]. Closed over every variant —
/// adding a new role to the kernel taxonomy will fail compilation
/// here until the proto enum and this mapping are updated together.
///
/// D41 §6.
pub(super) fn layer_role_to_proto(role: crate::commit::LayerRole) -> proto::LayerRole {
    match role {
        crate::commit::LayerRole::User => proto::LayerRole::User,
        crate::commit::LayerRole::AuditProvenance => proto::LayerRole::AuditProvenance,
        crate::commit::LayerRole::InstitutionClasses => proto::LayerRole::InstitutionClasses,
    }
}

/// Map `crate::institution::registry::RuntimeKind` → proto enum.
/// `None` (no `runtime` property on the resource) → `UNSPECIFIED`.
pub(super) fn runtime_kind_to_proto(
    kind: Option<crate::institution::registry::RuntimeKind>,
) -> proto::RuntimeKind {
    use crate::institution::registry::RuntimeKind as K;
    match kind {
        None => proto::RuntimeKind::Unspecified,
        Some(K::InProcess) => proto::RuntimeKind::InProcess,
        Some(K::External) => proto::RuntimeKind::External,
    }
}

/// Map `crate::institution::registry::DispatchRole` → proto enum.
pub(super) fn dispatch_role_to_proto(
    role: crate::institution::registry::DispatchRole,
) -> proto::DispatchRole {
    use crate::institution::registry::DispatchRole as R;
    match role {
        R::OnDemand => proto::DispatchRole::OnDemand,
        R::AutoOnLoad => proto::DispatchRole::AutoOnLoad,
        R::Decidable => proto::DispatchRole::Decidable,
    }
}

/// Parse a hex-encoded LayerId from the wire, returning a typed
/// `Status::invalid_argument` on malformed input.
#[allow(clippy::result_large_err)]
pub(super) fn parse_layer_id(hex_str: &str, field: &str) -> Result<crate::layer::LayerId, Status> {
    let bytes = hex::decode(hex_str)
        .map_err(|e| Status::invalid_argument(format!("{field} not valid hex: {e}")))?;
    if bytes.len() != 32 {
        return Err(Status::invalid_argument(format!(
            "{field} must be a 32-byte SHA-256 (64 hex chars)"
        )));
    }
    let mut id = [0u8; 32];
    id.copy_from_slice(&bytes);
    Ok(crate::layer::LayerId(id))
}

/// Convert a `TaskRecord` to the gRPC `TaskInfo` view.
pub(super) fn task_record_to_info(record: crate::task::TaskRecord) -> TaskInfo {
    TaskInfo {
        task_id: record.task_id.to_string(),
        session_id: record.session_id.to_string(),
        program_iri: record.program_iri,
        input_iri: record.input_iri,
        status: format!("{:?}", record.status),
        layer_head: hex::encode(record.layer_head.0),
        step_seq: record.step_seq,
        latest_trace_seq: record.latest_trace_seq,
        last_checkpoint_step: record
            .last_checkpoint
            .map(|n| n.to_string())
            .unwrap_or_default(),
        result_layer_head: record
            .result_layer_head
            .map(|id| hex::encode(id.0))
            .unwrap_or_default(),
        created_at_ms: record.created_at,
        updated_at_ms: record.updated_at,
        retain_forever: record.retain_forever,
    }
}

/// Encode a kernel `CascadeItem` as the wire shape. Mirrors the
/// enum variants one-for-one; `item_id` is the deterministic id the
/// kernel produces so clients can build acknowledgments from this
/// list directly.
pub(super) fn encode_cascade_item(
    item: &crate::layer::merge::CascadeItem,
) -> proto::CascadeItemWire {
    use crate::layer::merge::CascadeItem;
    let item_id = item.id().0;
    let kind = match item {
        CascadeItem::OrphanedReference {
            resource,
            dropped_target,
            location,
        } => proto::cascade_item_wire::Kind::OrphanedReference(proto::OrphanedReferenceItem {
            resource: resource.as_str().to_string(),
            dropped_target: dropped_target.as_str().to_string(),
            property_path: location.0.iter().map(|i| i.as_str().to_string()).collect(),
        }),
        CascadeItem::OrphanedTyping {
            class,
            affected_resources,
        } => proto::cascade_item_wire::Kind::OrphanedTyping(proto::OrphanedTypingItem {
            class: class.as_str().to_string(),
            affected_resources: affected_resources
                .iter()
                .map(|i| i.as_str().to_string())
                .collect(),
        }),
        CascadeItem::InvalidatedSignature {
            program,
            signature_problem,
        } => {
            proto::cascade_item_wire::Kind::InvalidatedSignature(proto::InvalidatedSignatureItem {
                program: program.as_str().to_string(),
                signature_problem: signature_problem.clone(),
            })
        }
        CascadeItem::InvalidatedTrace { trace, reason } => {
            proto::cascade_item_wire::Kind::InvalidatedTrace(proto::InvalidatedTraceItem {
                trace: trace.clone(),
                reason: reason.clone(),
            })
        }
    };
    proto::CascadeItemWire {
        item_id,
        kind: Some(kind),
    }
}

/// Encode a kernel `TypedConflict` as the wire shape (D36 §3.1).
/// Mirrors the `ConflictKind` enum one-for-one; the four reserved
/// stage-2/3 kinds (`DeletionConflict`, `DisjointnessViolation`,
/// `PathEquationContradiction`) don't fire in v1 and produce a
/// conflict with `kind = None` (the resolution UI surfaces that as
/// "internal kernel error — please report"). Once those kinds gain
/// wire shapes, add the corresponding `oneof` arms here.
pub(super) fn encode_typed_conflict(
    conflict: &crate::layer::merge::TypedConflict,
) -> proto::TypedConflictWire {
    use crate::layer::merge::ConflictKind;
    let kind = match &conflict.kind {
        ConflictKind::PropertyDataType {
            property,
            branch_a,
            branch_b,
            ancestor,
        } => Some(proto::typed_conflict_wire::Kind::PropertyDataType(
            proto::PropertyDataTypeConflict {
                property: property.as_str().to_string(),
                branch_a_type: branch_a.as_str().to_string(),
                branch_b_type: branch_b.as_str().to_string(),
                ancestor_type: ancestor
                    .as_ref()
                    .map(|i| i.as_str().to_string())
                    .unwrap_or_default(),
            },
        )),
        ConflictKind::KindMismatch {
            iri,
            branch_a_kind,
            branch_b_kind,
        } => Some(proto::typed_conflict_wire::Kind::KindMismatch(
            proto::KindMismatchConflict {
                iri: iri.as_str().to_string(),
                branch_a_kind: render_resource_kind(*branch_a_kind),
                branch_b_kind: render_resource_kind(*branch_b_kind),
            },
        )),
        ConflictKind::IriCollision {
            iri,
            branch_a_body,
            branch_b_body,
            ancestor_body,
        } => Some(proto::typed_conflict_wire::Kind::IriCollision(
            proto::IriCollisionConflict {
                iri: iri.as_str().to_string(),
                branch_a_body_json: serialize_body_as_json(&branch_a_body.resource),
                branch_b_body_json: serialize_body_as_json(&branch_b_body.resource),
                ancestor_body_json: ancestor_body
                    .as_ref()
                    .map(|b| serialize_body_as_json(&b.resource))
                    .unwrap_or_default(),
            },
        )),
        ConflictKind::InheritanceCycle { cycle } => Some(
            proto::typed_conflict_wire::Kind::InheritanceCycle(proto::InheritanceCycleConflict {
                cycle: cycle.iter().map(|i| i.as_str().to_string()).collect(),
            }),
        ),
        // Reserved kinds — no wire shape yet. The notebook should
        // never see these because the classifier doesn't surface
        // them in v1, but we leave `kind = None` rather than
        // panic'ing so a future kernel that does surface them gets
        // an unambiguous "wire shape missing" signal at the UI.
        ConflictKind::DeletionConflict { .. }
        | ConflictKind::DisjointnessViolation { .. }
        | ConflictKind::PathEquationContradiction { .. } => None,
    };
    let applicable_strategies = applicable_strategies_for(&conflict.kind)
        .into_iter()
        .map(|k| k as i32)
        .collect();
    proto::TypedConflictWire {
        id: conflict.id.0.clone(),
        kind,
        applicable_strategies,
    }
}

/// Render a `ResourceKind` as the wire string mirroring D36 §3.1's
/// `KindMismatchConflict.branch_*_kind` field comment.
fn render_resource_kind(k: crate::layer::merge::ResourceKind) -> String {
    use crate::layer::merge::ResourceKind;
    match k {
        ResourceKind::Class => "Class",
        ResourceKind::Property => "Property",
        ResourceKind::Other => "Other",
    }
    .to_string()
}

/// Serialize a `Resource` as Eigon-JSON for inclusion in the
/// `IriCollisionConflict` wire shape. The UI uses this to render a
/// body-diff without an additional resource-fetch round trip.
fn serialize_body_as_json(resource: &crate::ontology::resource::Resource) -> String {
    let value = crate::ontology::eigon_json::serialize_resource(resource);
    serde_json::to_string(&value).unwrap_or_default()
}

/// Compute the list of strategies whose applicability check passes
/// for a given `ConflictKind` (D36 §3.1, mirroring the kernel's
/// per-strategy rules from D20 §6).
///
/// **Witness, Rename, Restructure** apply to every kind in v1:
/// the witness term is type-checked against the kind's class at
/// commit time, rename rewrites references regardless of kind, and
/// restructure raises the abstraction by introducing a new parent
/// (which works for any class-shaped or even instance-shaped
/// conflict, though the user's `RestructureSpec` may not pass the
/// per-conflict validation — that's caught at commit time).
///
/// **SchemaQuotient** applicability is per the D20 §6.3 table:
/// every classified v1 kind is single-valued or mutually exclusive,
/// so `KeepBoth` is never applicable; `KeepOne` and `KeepNeither`
/// always are.
pub(super) fn applicable_strategies_for(
    kind: &crate::layer::merge::ConflictKind,
) -> Vec<proto::MergeStrategyKind> {
    use crate::layer::merge::ConflictKind;
    let mut out = vec![
        proto::MergeStrategyKind::Witness,
        proto::MergeStrategyKind::Rename,
    ];
    match kind {
        ConflictKind::PropertyDataType { .. }
        | ConflictKind::KindMismatch { .. }
        | ConflictKind::IriCollision { .. }
        | ConflictKind::InheritanceCycle { .. }
        | ConflictKind::DeletionConflict { .. }
        | ConflictKind::DisjointnessViolation { .. }
        | ConflictKind::PathEquationContradiction { .. } => {
            // KeepBoth is structurally inapplicable to every v1
            // kind (each is single-valued or mutually exclusive).
            // The wire-side picker greys it out per
            // `applicable_strategies` rather than per the editor's
            // own check, so the response is the source of truth.
            out.push(proto::MergeStrategyKind::KeepOne);
            out.push(proto::MergeStrategyKind::KeepNeither);
        }
    }
    out.push(proto::MergeStrategyKind::Restructure);
    out
}

/// Decode a list of wire-shaped `MergeResolutionWire` into kernel
/// `MergeResolution`s (D20 §7.2). Returns a human-readable diagnostic
/// on malformed input; the handler wraps it in
/// `SubmitResolutionErrorKind::MalformedResolution`.
pub(super) fn decode_resolutions(
    wire: &[proto::MergeResolutionWire],
) -> Result<Vec<crate::layer::merge::MergeResolution>, String> {
    use crate::layer::merge::{ConflictId, MergeResolution, RestructureSpec};

    let mut out = Vec::with_capacity(wire.len());
    for (idx, r) in wire.iter().enumerate() {
        let conflict_id = ConflictId(r.conflict_id.clone());
        let strategy = r
            .strategy
            .as_ref()
            .ok_or_else(|| format!("resolutions[{idx}]: missing strategy oneof"))?;
        let resolution = match strategy {
            proto::merge_resolution_wire::Strategy::Witness(w) => {
                let comorphism = crate::ontology::iri::Iri::parse(&w.comorphism_iri)
                    .map_err(|e| format!("resolutions[{idx}].comorphism_iri: {e}"))?;
                MergeResolution::Witness {
                    conflict: conflict_id,
                    comorphism,
                }
            }
            proto::merge_resolution_wire::Strategy::Rename(r) => {
                let side = decode_side(r.side, idx)?;
                let old_iri = crate::ontology::iri::Iri::parse(&r.old_iri)
                    .map_err(|e| format!("resolutions[{idx}].old_iri: {e}"))?;
                let new_iri = crate::ontology::iri::Iri::parse(&r.new_iri)
                    .map_err(|e| format!("resolutions[{idx}].new_iri: {e}"))?;
                MergeResolution::Rename {
                    conflict: conflict_id,
                    side,
                    old_iri,
                    new_iri,
                }
            }
            proto::merge_resolution_wire::Strategy::Quotient(q) => {
                let quotient = decode_quotient(q, idx)?;
                MergeResolution::SchemaQuotient {
                    conflict: conflict_id,
                    quotient,
                }
            }
            proto::merge_resolution_wire::Strategy::Restructure(spec) => {
                let affected_class = crate::ontology::iri::Iri::parse(&spec.affected_class)
                    .map_err(|e| format!("resolutions[{idx}].affected_class: {e}"))?;
                let new_parent = crate::ontology::iri::Iri::parse(&spec.new_parent)
                    .map_err(|e| format!("resolutions[{idx}].new_parent: {e}"))?;
                let new_parent_def = if spec.new_parent_def_json.is_empty() {
                    None
                } else {
                    Some(decode_eigon_json_resource(&spec.new_parent_def_json, idx)?)
                };
                let mut classes_under_new = Vec::with_capacity(spec.classes_under_new.len());
                for (j, cls) in spec.classes_under_new.iter().enumerate() {
                    let iri = crate::ontology::iri::Iri::parse(cls)
                        .map_err(|e| format!("resolutions[{idx}].classes_under_new[{j}]: {e}"))?;
                    classes_under_new.push(iri);
                }
                MergeResolution::Restructure {
                    conflict: conflict_id,
                    spec: RestructureSpec {
                        affected_class,
                        new_parent,
                        new_parent_def,
                        classes_under_new,
                        affected_class_under_new: spec.affected_class_under_new,
                    },
                }
            }
        };
        out.push(resolution);
    }
    Ok(out)
}

/// Decode an Eigon-JSON-encoded `Resource` from a wire string. Used
/// for `RestructureStrategy.new_parent_def_json`. The wire shape is
/// one top-level Class resource (with `@id`), so we wrap it in an
/// array for `parse_document` — `parse_embedded` rejects resources
/// that carry an `@id`. Returns a human-readable diagnostic on
/// parse failure; the caller wraps it in `MALFORMED_RESOLUTION`.
fn decode_eigon_json_resource(
    json: &str,
    idx: usize,
) -> Result<crate::ontology::resource::Resource, String> {
    let wrapped = format!("[{json}]");
    let mut resources = crate::ontology::eigon_json::parse_document(&wrapped)
        .map_err(|e| format!("resolutions[{idx}].new_parent_def_json: {e}"))?;
    if resources.len() != 1 {
        return Err(format!(
            "resolutions[{idx}].new_parent_def_json: expected exactly one resource, got {}",
            resources.len()
        ));
    }
    Ok(resources.remove(0))
}

fn decode_side(wire: i32, idx: usize) -> Result<crate::layer::merge::Side, String> {
    use crate::layer::merge::Side;
    match proto::MergeSide::try_from(wire) {
        Ok(proto::MergeSide::A) => Ok(Side::A),
        Ok(proto::MergeSide::B) => Ok(Side::B),
        Ok(proto::MergeSide::Unspecified) => Err(format!(
            "resolutions[{idx}]: side is MERGE_SIDE_UNSPECIFIED"
        )),
        Err(_) => Err(format!("resolutions[{idx}]: unknown MergeSide enum value")),
    }
}

fn decode_quotient(
    wire: &proto::QuotientStrategy,
    idx: usize,
) -> Result<crate::layer::merge::SchemaQuotient, String> {
    use crate::layer::merge::SchemaQuotient;
    match proto::MergeQuotientKind::try_from(wire.kind) {
        Ok(proto::MergeQuotientKind::KeepBoth) => Ok(SchemaQuotient::KeepBoth),
        Ok(proto::MergeQuotientKind::KeepOne) => {
            let winner = decode_side(wire.winner, idx)?;
            Ok(SchemaQuotient::KeepOne { winner })
        }
        Ok(proto::MergeQuotientKind::KeepNeither) => Ok(SchemaQuotient::KeepNeither),
        Ok(proto::MergeQuotientKind::Unspecified) => Err(format!(
            "resolutions[{idx}]: quotient kind is MERGE_QUOTIENT_KIND_UNSPECIFIED"
        )),
        Err(_) => Err(format!(
            "resolutions[{idx}]: unknown MergeQuotientKind enum value"
        )),
    }
}

/// Translate a `MergeError` into a `SubmitResolutionResponse` with
/// the right typed `error_kind`. Internal failures map to
/// `INTERNAL`; structural failures (cascade gate, conflict-not-found,
/// applicability) map to their dedicated variants.
pub(super) fn merge_error_to_submit_response(
    err: &crate::layer::merge::MergeError,
) -> Response<SubmitResolutionResponse> {
    use crate::layer::merge::MergeError;
    let (error_kind, missing) = match err {
        MergeError::IncompleteAcknowledgments { missing } => (
            proto::SubmitResolutionErrorKind::IncompleteAcknowledgments,
            missing.iter().map(|m| m.0.clone()).collect(),
        ),
        MergeError::ConflictNotFound(_) | MergeError::UnresolvedConflict { .. } => (
            proto::SubmitResolutionErrorKind::ConflictNotFound,
            Vec::new(),
        ),
        MergeError::NoCommonAncestor { .. } => (
            proto::SubmitResolutionErrorKind::NoCommonAncestor,
            Vec::new(),
        ),
        // `SUBMIT_RESOLUTION_ERROR_KIND_APPLICATION_PENDING` stays
        // on the wire as a reserved value for backward compat with
        // earlier kernel revisions; the kernel no longer constructs
        // it (every variant's commit shape is wired). Future
        // partially-supported resolution kinds would route here.
        MergeError::QuotientNotApplicable { .. }
        | MergeError::RenameTargetNotInBranch { .. }
        | MergeError::RenameCollision { .. }
        | MergeError::RenameIdentity { .. }
        | MergeError::MergeComorphismNotFound(_)
        | MergeError::NotAMergeComorphism { .. }
        | MergeError::MalformedMergeComorphism { .. }
        | MergeError::MergeComorphismWrongClass { .. }
        | MergeError::TransformationNotFound { .. }
        | MergeError::TransformationParseError { .. }
        | MergeError::TransformationEvalError { .. }
        | MergeError::WitnessTermNotAFunction { .. }
        | MergeError::WitnessTypeMismatch { .. }
        | MergeError::WitnessTargetNotResolvable { .. }
        | MergeError::RestructureSynthesizedParent { .. }
        | MergeError::RestructureParentRedeclaration { .. }
        | MergeError::RestructureParentMissingDefinition { .. }
        | MergeError::RestructureParentDefMismatch { .. }
        | MergeError::RestructureParentDefNotAClass { .. }
        | MergeError::RestructureClassNotInSpan { .. } => (
            proto::SubmitResolutionErrorKind::MalformedResolution,
            Vec::new(),
        ),
        MergeError::Storage(_) | MergeError::LayerBuild(_) => {
            (proto::SubmitResolutionErrorKind::Internal, Vec::new())
        }
    };
    Response::new(SubmitResolutionResponse {
        success: false,
        error: err.to_string(),
        error_kind: error_kind as i32,
        merge_layer_id: String::new(),
        branch_tip: String::new(),
        missing_acknowledgments: missing,
    })
}

pub(super) fn submit_resolution_internal_error(
    error: String,
) -> Response<SubmitResolutionResponse> {
    Response::new(SubmitResolutionResponse {
        success: false,
        error,
        error_kind: proto::SubmitResolutionErrorKind::Internal as i32,
        ..Default::default()
    })
}

/// Convert a `ConsolidateError` into the wire response. Both
/// `ConsolidateChain` and `EstimateConsolidation` use the same kind
/// enum + offending-layer/count fields; the two helpers differ only
/// in response shape (success-path fields are zeroed).
pub(super) fn consolidate_error_to_response(
    err: crate::layer::ConsolidateError,
) -> ConsolidateChainResponse {
    let (kind, error_layer, error_count) = consolidate_error_parts(&err);
    ConsolidateChainResponse {
        success: false,
        consolidated_layer: String::new(),
        collapsed_layer_count: 0,
        head_advanced: false,
        error_kind: kind as i32,
        error: err.to_string(),
        error_layer,
        error_count,
    }
}

pub(super) fn estimate_error_to_response(
    err: crate::layer::ConsolidateError,
) -> EstimateConsolidationResponse {
    let (kind, error_layer, error_count) = consolidate_error_parts(&err);
    EstimateConsolidationResponse {
        success: false,
        predicted_consolidated_layer: String::new(),
        collapsed_layer_count: 0,
        predicted_walk_entries: 0,
        actual_walk_entries: 0,
        error_kind: kind as i32,
        error: err.to_string(),
        error_layer,
        error_count,
    }
}

fn consolidate_error_parts(
    err: &crate::layer::ConsolidateError,
) -> (ConsolidateErrorKind, String, u64) {
    use crate::layer::ConsolidateError as E;
    match err {
        E::RangeNotAncestral { from, .. } => (
            ConsolidateErrorKind::RangeNotAncestral,
            hex::encode(from.0),
            0,
        ),
        E::BranchAdvancedConcurrently { observed_head, .. } => (
            ConsolidateErrorKind::BranchAdvanced,
            observed_head
                .as_ref()
                .map(|h| hex::encode(h.0))
                .unwrap_or_default(),
            0,
        ),
        E::RangeContainsMergeNode { merge_layer } => (
            ConsolidateErrorKind::RangeContainsMergeNode,
            hex::encode(merge_layer.0),
            0,
        ),
        E::RangeContainsTracePin {
            pinned_layer,
            trace_count,
        } => (
            ConsolidateErrorKind::RangeContainsTracePin,
            hex::encode(pinned_layer.0),
            *trace_count,
        ),
        E::CostExceedsCap { predicted_entries } => (
            ConsolidateErrorKind::CostExceedsCap,
            String::new(),
            *predicted_entries,
        ),
        E::ToNotReachableFromHead { observed_head, .. } => (
            ConsolidateErrorKind::ToNotReachableFromHead,
            hex::encode(observed_head.0),
            0,
        ),
        E::RangeCrossesExistingRedirect { offending_layer } => (
            ConsolidateErrorKind::RangeCrossesExistingRedirect,
            hex::encode(offending_layer.0),
            0,
        ),
        E::WriteFailed(_) | E::Internal(_) => (ConsolidateErrorKind::Internal, String::new(), 0),
    }
}

/// Build a `ConsolidateOpts` from the wire request. Pulls active task
/// pins from the session's task store (matches `delete_branch`'s
/// pattern). A `max_walk_entries` of 0 means "use the kernel default."
pub(super) async fn build_consolidate_opts(
    max_walk_entries: &u64,
    preserve_history: bool,
    service: &super::EigeniusService,
) -> Result<crate::layer::ConsolidateOpts, Status> {
    let mut opts = crate::layer::ConsolidateOpts::default();
    if *max_walk_entries > 0 {
        opts.max_walk_entries = *max_walk_entries;
    }
    opts.preserve_history = preserve_history;
    if let Some(store) = service.task_store.as_ref() {
        let session_id = service.session.read().await.session_id;
        match store.list_tasks(&session_id) {
            Ok(records) => {
                for record in records {
                    if record.status.is_terminal() {
                        continue;
                    }
                    *opts.pinned_layers.entry(record.layer_head).or_insert(0) += 1;
                }
            }
            Err(e) => return Err(Status::internal(format!("list_tasks failed: {e}"))),
        }
    }
    Ok(opts)
}

#[cfg(test)]
mod merge_info_tests {
    //! Unit tests for [`merge_info_from_persist_info`]. The lattice
    //! tests already pin the three [`UpdateOutcome`] variants on the
    //! production side; these tests pin the **conversion** to the
    //! proto wire format — making sure each persist-info shape maps
    //! to the correct enum value with the right side fields populated.
    //!
    //! Combined with the e2e tests in `storage/rocksdb/tests/`, this
    //! gives us defense in depth against regressions of D34 §G.1's
    //! silent-`NeedsWitnessedMerge` bug and the cache-hit conflation
    //! the `CachedDifferentPosition` variant resolves.
    use super::*;
    use crate::lattice::UpdateOutcome;
    use crate::layer::LayerId;
    use crate::ontology::iri::Iri;
    use hex::encode as hex_encode;
    use proto::MergeOutcome;
    fn lid(byte: u8) -> LayerId {
        LayerId([byte; 32])
    }
    fn pli(
        layer_id: LayerId,
        branch_advanced: bool,
        merge_outcome: Option<UpdateOutcome>,
        cache_hit_different_position: bool,
    ) -> PersistedLayerInfo {
        PersistedLayerInfo {
            layer_id,
            branch_advanced,
            merge_outcome,
            cache_hit_different_position,
        }
    }
    #[test]
    fn no_persist_info_maps_to_unspecified_with_empty_fields() {
        let info = merge_info_from_persist_info(None);
        assert_eq!(info.outcome, MergeOutcome::Unspecified as i32);
        assert!(info.merge_layer_id.is_empty());
        assert!(info.conflicting_iris.is_empty());
        assert!(info.current_head.is_empty());
    }
    #[test]
    fn no_cas_with_no_cache_hit_maps_to_unspecified() {
        // The no-backend path: persist ran, returned no merge_outcome,
        // and didn't hit the anchored-commit cache.
        let pi = pli(lid(0xFF), false, None, false);
        let info = merge_info_from_persist_info(Some(&pi));
        assert_eq!(info.outcome, MergeOutcome::Unspecified as i32);
        assert!(info.merge_layer_id.is_empty());
    }
    #[test]
    fn fast_forward_maps_with_empty_side_fields() {
        let pi = pli(lid(0x01), true, Some(UpdateOutcome::FastForward), false);
        let info = merge_info_from_persist_info(Some(&pi));
        assert_eq!(info.outcome, MergeOutcome::FastForward as i32);
        assert!(info.merge_layer_id.is_empty());
        assert!(info.conflicting_iris.is_empty());
        assert!(info.current_head.is_empty());
    }
    #[test]
    fn trivial_merge_carries_merge_layer_id_as_hex() {
        let merge_layer = lid(0xAB);
        let pi = pli(
            lid(0x01),
            true,
            Some(UpdateOutcome::TrivialMerge {
                merge_layer: merge_layer.clone(),
            }),
            false,
        );
        let info = merge_info_from_persist_info(Some(&pi));
        assert_eq!(info.outcome, MergeOutcome::TrivialMerge as i32);
        assert_eq!(info.merge_layer_id, hex_encode(merge_layer.0));
        assert!(info.conflicting_iris.is_empty());
        assert!(info.current_head.is_empty());
    }
    #[test]
    fn needs_witnessed_merge_carries_head_and_iris() {
        let current_head = lid(0xCD);
        let conflicting_iris = vec![
            Iri::parse("urn:eigenius:demo:A").unwrap(),
            Iri::parse("urn:eigenius:demo:B").unwrap(),
        ];
        let orphan = lid(0xEF);
        let pi = pli(
            lid(0x01),
            false,
            Some(UpdateOutcome::NeedsWitnessedMerge {
                current_head: current_head.clone(),
                conflicting_iris: conflicting_iris.clone(),
                orphan_head: orphan.clone(),
            }),
            false,
        );
        let info = merge_info_from_persist_info(Some(&pi));
        assert_eq!(info.outcome, MergeOutcome::NeedsWitnessedMerge as i32);
        assert!(info.merge_layer_id.is_empty());
        assert_eq!(info.current_head, hex_encode(current_head.0));
        assert_eq!(info.orphan_layer_id, hex_encode(orphan.0));
        assert_eq!(
            info.conflicting_iris,
            vec![
                "urn:eigenius:demo:A".to_string(),
                "urn:eigenius:demo:B".to_string()
            ]
        );
    }
    #[test]
    fn cache_hit_different_position_maps_with_cached_layer_id() {
        // Distinct from `UNSPECIFIED`: the persist short-circuited
        // because the content is canonical at the carried layer_id,
        // and the branch ref did **not** advance.
        let cached = lid(0x77);
        let pi = pli(cached.clone(), false, None, true);
        let info = merge_info_from_persist_info(Some(&pi));
        assert_eq!(info.outcome, MergeOutcome::CachedDifferentPosition as i32);
        assert_eq!(info.merge_layer_id, hex_encode(cached.0));
        assert!(info.conflicting_iris.is_empty());
        assert!(info.current_head.is_empty());
    }
    #[test]
    fn cache_hit_flag_dominates_over_any_merge_outcome() {
        // Defensive: if a caller ever sets both `cache_hit_different_position`
        // and `merge_outcome=Some(...)`, the cache-hit signal wins.
        // `persist_layer_if_backend` doesn't actually produce that
        // combination today, but pinning the precedence keeps the
        // mapping unambiguous.
        let cached = lid(0x55);
        let pi = pli(
            cached.clone(),
            false,
            Some(UpdateOutcome::FastForward),
            true,
        );
        let info = merge_info_from_persist_info(Some(&pi));
        assert_eq!(info.outcome, MergeOutcome::CachedDifferentPosition as i32);
        assert_eq!(info.merge_layer_id, hex_encode(cached.0));
    }
}

#[cfg(test)]
mod institution_enrichment_tests {
    //! Pin the runtime + dispatch-role enum mappings used by
    //! `list_institutions` (D34 §G.8 / §9.2). When the registry adds a
    //! `RuntimeKind` or `DispatchRole` variant, the matching proto
    //! branch must follow — the exhaustive-match in
    //! `runtime_kind_to_proto` / `dispatch_role_to_proto` catches it at
    //! compile time, and these tests catch any drift between the proto
    //! enum's numeric values and the kernel-side ordering.

    use super::*;
    use crate::institution::registry::{
        DispatchRole as KernelDispatchRole, RuntimeKind as KernelRuntimeKind,
    };

    #[test]
    fn runtime_kind_maps_every_variant() {
        assert_eq!(runtime_kind_to_proto(None), proto::RuntimeKind::Unspecified);
        assert_eq!(
            runtime_kind_to_proto(Some(KernelRuntimeKind::InProcess)),
            proto::RuntimeKind::InProcess
        );
        assert_eq!(
            runtime_kind_to_proto(Some(KernelRuntimeKind::External)),
            proto::RuntimeKind::External
        );
    }

    #[test]
    fn dispatch_role_maps_every_variant() {
        assert_eq!(
            dispatch_role_to_proto(KernelDispatchRole::OnDemand),
            proto::DispatchRole::OnDemand
        );
        assert_eq!(
            dispatch_role_to_proto(KernelDispatchRole::AutoOnLoad),
            proto::DispatchRole::AutoOnLoad
        );
        assert_eq!(
            dispatch_role_to_proto(KernelDispatchRole::Decidable),
            proto::DispatchRole::Decidable
        );
    }
}
