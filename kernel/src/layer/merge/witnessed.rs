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

//! Witness merge resolution (D20 §6.1).
//!
//! A witness resolution names a `MergeComorphism` resource whose
//! `merge_transformation` realises the universal arrow at the
//! conflicting IRI. This module covers:
//!
//! - [`MergeComorphismHandle`] — the resolved witness ready for
//!   application.
//! - [`resolve_merge_comorphism`] — chain walk + shape validation
//!   (runs at submission time; the four-tier search includes the
//!   ancestor's parent chain, both branches, and optionally
//!   user-named extra branches per D38 §4).
//! - [`apply_witness_resolution`] — type-check the transformation
//!   against `(A, A, Option<A>) → A`, evaluate in Pure mode, marshal
//!   the result back to a Resource.

use super::conflict::{ConflictKind, MergeSpan};
use super::lca::{find_in_span_chain, find_iri_in_chain, iter_iri_values};
use super::MergeError;
use crate::layer::handle::LayerTopology;
use crate::layer::LayerId;
use crate::ontology::iri::Iri;
use crate::ontology::resource::Resource;
use crate::ontology::well_known as wk;
use crate::storage::{PersistentBackend, StorageError};

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
    /// The IRI of the EigenTT term realising the universal arrow.
    /// Resolves to a resource committed earlier in the chain whose
    /// `is_a` includes one of the EigenTT expression classes
    /// (`program:Lambda`, etc.).
    pub transformation: Iri,
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
    expected_class: &Iri,
    span: &MergeSpan,
    extra_branches: &[String],
    topology: &LayerTopology,
    backend: &dyn PersistentBackend,
) -> Result<MergeComorphismHandle, MergeError> {
    let merge_comorphism_iri = Iri::parse(wk::MERGE_COMORPHISM).expect("MERGE_COMORPHISM IRI");
    let merge_transformation_iri =
        Iri::parse(wk::MERGE_TRANSFORMATION).expect("MERGE_TRANSFORMATION IRI");
    let merge_target_class_iri =
        Iri::parse(wk::MERGE_TARGET_CLASS).expect("MERGE_TARGET_CLASS IRI");

    // D20 §6.1 + D38 §4: four-tier search. Each branch's contributions
    // and the ancestor chain are the canonical locations. The
    // `extra_branches` list is consulted last so an unscoped witness
    // on a sibling branch can't shadow a "real" witness reachable
    // from the merge span; users opt into broader scope by naming
    // the branches.
    let mut resource_loc: Option<(LayerId, Resource)> =
        find_in_span_chain(iri, span, topology, backend).map_err(MergeError::Storage)?;
    if resource_loc.is_none() {
        for branch_name in extra_branches {
            // Skip blank entries — the wire surface tolerates a
            // possibly-empty trim from the picker UI.
            if branch_name.is_empty() {
                continue;
            }
            let branch_tip = match backend
                .get_branch(branch_name)
                .map_err(MergeError::Storage)?
            {
                Some(tip) => tip,
                None => continue, // unknown / deleted branch — silently skip
            };
            if let Some((layer_id, resource)) =
                find_iri_in_chain(&branch_tip, iri, topology, backend)
                    .map_err(MergeError::Storage)?
            {
                resource_loc = Some((layer_id, resource));
                break;
            }
        }
    }
    let (source_layer, resource) =
        resource_loc.ok_or_else(|| MergeError::MergeComorphismNotFound(iri.clone()))?;

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

    // D37 §6.2: every MergeComorphism declares the class A its
    // transformation operates on. Reject early if the conflict's
    // class doesn't match — the witness's term type-check would
    // eventually fail anyway, but the diagnostic surfaces deep
    // inside the evaluator with an opaque message; the typed
    // up-front error is much more actionable.
    // Accept both `ResourceRef` (canonical post-`canonicalise_resource_refs`)
    // and `String` (the shape that survives CBOR storage round-trips —
    // `Value::ResourceRef` serialises as a plain text node, so any
    // resource re-loaded from disk via `try_load_resource` comes back
    // with `Value::String` for IRI-typed properties). `Value::as_iri()`
    // unifies the two shapes.
    let target_class = match resource
        .get(&merge_target_class_iri)
        .and_then(|v| v.as_iri())
    {
        Some(c) => c,
        None => {
            let reason = if resource.get(&merge_target_class_iri).is_some() {
                "merge_target_class must be a Class IRI (ResourceRef or String)"
            } else {
                "merge_target_class property is required"
            };
            return Err(MergeError::MalformedMergeComorphism {
                iri: iri.clone(),
                reason: reason.to_string(),
            });
        }
    };
    if &target_class != expected_class {
        return Err(MergeError::MergeComorphismWrongClass {
            iri: iri.clone(),
            expected: expected_class.clone(),
            actual: target_class,
        });
    }

    // Same dual-shape acceptance as `merge_target_class` above —
    // storage-round-tripped IRIs deserialize as `Value::String`.
    let transformation = match resource
        .get(&merge_transformation_iri)
        .and_then(|v| v.as_iri())
    {
        Some(t) => t,
        None => {
            let reason = if resource.get(&merge_transformation_iri).is_some() {
                "merge_transformation must be an IRI reference to a EigenTT term"
            } else {
                "merge_transformation property is required"
            };
            return Err(MergeError::MalformedMergeComorphism {
                iri: iri.clone(),
                reason: reason.to_string(),
            });
        }
    };

    Ok(MergeComorphismHandle {
        iri: iri.clone(),
        source_layer,
        transformation,
    })
}

/// Apply a validated `MergeComorphism` witness to a triple of
/// `(branch_a, branch_b, ancestor)` and produce the merged resource
/// body. Implements D20 §6.1's `(A, A, Option<A>) → A` signature
/// discipline end-to-end: type-check first, then evaluate.
///
/// Pipeline:
///  1. Build an in-memory chain for `handle.source_layer` so the
///     parser + evaluator can resolve references through it.
///  2. Look up the transformation Resource (the EigenTT term the
///     comorphism points at). The lookup walks the chain — the
///     transformation may live at the witness's source layer, an
///     ancestor, or any layer reachable from the witness.
///  3. Parse the Resource into a EigenTT `Exp` via
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

    // 3. Parse the Resource into a EigenTT Exp.
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
            reason: reason.to_string(),
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
    //    match it via EigenTT's standard inductive elimination.
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

/// Return the IRI a Witness merge transforms. v1 supports the
/// single-IRI conflict kinds (`IriCollision`, `KindMismatch`,
/// `PropertyDataType`); the inheritance-cycle and reserved kinds
/// don't have a single witness target.
pub(crate) fn witness_target_iri(kind: &ConflictKind) -> Option<&Iri> {
    match kind {
        ConflictKind::IriCollision { iri, .. } => Some(iri),
        ConflictKind::KindMismatch { iri, .. } => Some(iri),
        ConflictKind::PropertyDataType { property, .. } => Some(property),
        ConflictKind::DeletionConflict { iri, .. } => Some(iri),
        _ => None,
    }
}

/// Look up the class of the conflict's IRI by reading any branch's
/// or the ancestor's body and pulling the first `is_a` entry. The
/// witness signature is `(A, A, Option<A>) → A` where `A` is this
/// class; supplying it to `apply_witness_resolution` enables the
/// commit-time signature check.
pub(crate) fn witness_target_class(
    iri: &Iri,
    span: &MergeSpan,
    topology: &LayerTopology,
    backend: &dyn PersistentBackend,
) -> Result<Option<Iri>, MergeError> {
    let pick_class = |r: &Resource| r.is_a().first().cloned();
    if let Some(layer_id) = span.sources_a.get(iri) {
        if let Some(r) = backend
            .try_load_resource(layer_id, iri)
            .map_err(MergeError::Storage)?
        {
            if let Some(c) = pick_class(&r) {
                return Ok(Some(c));
            }
        }
    }
    if let Some(layer_id) = span.sources_b.get(iri) {
        if let Some(r) = backend
            .try_load_resource(layer_id, iri)
            .map_err(MergeError::Storage)?
        {
            if let Some(c) = pick_class(&r) {
                return Ok(Some(c));
            }
        }
    }
    if let Some((_, r)) =
        find_iri_in_chain(&span.ancestor, iri, topology, backend).map_err(MergeError::Storage)?
    {
        if let Some(c) = pick_class(&r) {
            return Ok(Some(c));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::merge::conflict::{classify_conflicts, ConflictId};
    use crate::layer::merge::resolve::{merge_with_resolutions, MergeResolution};
    use crate::layer::merge::test_support::{
        build_span, build_span_arc, build_witness_fixture, build_witness_fixture_offspan, iri,
        make_lambda_resource, make_resource, make_var_resource,
    };
    use crate::ontology::resource::{Resource, Value};
    use crate::storage::memory::MemoryPersistentBackend;

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
    /// (placeholder) transformation IRI. The witness fixtures
    /// resolve against `urn:test:Patient` in every existing test, so
    /// that's the default class — call sites that need a different
    /// `merge_target_class` should construct the resource inline.
    fn make_merge_comorphism(iri: &str, transformation: &str) -> Resource {
        make_resource(
            iri,
            &[wk::MERGE_COMORPHISM],
            &[
                (
                    wk::MERGE_TRANSFORMATION,
                    Value::ResourceRef(Iri::parse(transformation).unwrap()),
                ),
                (
                    wk::MERGE_TARGET_CLASS,
                    Value::ResourceRef(Iri::parse("urn:test:Patient").unwrap()),
                ),
            ],
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
            Vec::new(),
            Vec::new(),
            crate::layer::LayerStorage::in_memory(),
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
            Vec::new(),
            Vec::new(),
            crate::layer::LayerStorage::in_memory(),
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
            Vec::new(),
            Vec::new(),
            crate::layer::LayerStorage::in_memory(),
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
    fn witness_for_wrong_class_is_rejected_early() {
        // D37 §6.2: a `MergeComorphism` declares `merge_target_class`;
        // applying it to a conflict on a different class must fail
        // up-front with `MergeComorphismWrongClass`, not deep inside
        // the term evaluator with an opaque type-mismatch.
        //
        // Construct a comorphism declared for `urn:test:Visit` and
        // try to apply it to the IriCollision on `urn:test:Patient`.
        let wrong_class_witness = make_resource(
            "urn:test:wrong_class_witness",
            &[wk::MERGE_COMORPHISM],
            &[
                (
                    wk::MERGE_TRANSFORMATION,
                    Value::ResourceRef(Iri::parse("urn:test:term_placeholder").unwrap()),
                ),
                (
                    wk::MERGE_TARGET_CLASS,
                    Value::ResourceRef(Iri::parse("urn:test:Visit").unwrap()),
                ),
            ],
        );
        let (span, backend) =
            build_span_with_iri_collision_and_optional_witness(Some(wrong_class_witness));
        let conflicts = classify_conflicts(&span, &backend).unwrap();
        let conflict_id = conflicts[0].id.clone();

        let result = merge_with_resolutions(
            &span,
            vec![MergeResolution::Witness {
                conflict: conflict_id,
                comorphism: iri("urn:test:wrong_class_witness"),
            }],
            Vec::new(),
            Vec::new(),
            crate::layer::LayerStorage::in_memory(),
            &backend,
        );
        match result {
            Err(MergeError::MergeComorphismWrongClass {
                iri: i,
                expected,
                actual,
            }) => {
                assert_eq!(i.as_str(), "urn:test:wrong_class_witness");
                assert_eq!(expected.as_str(), "urn:test:Patient");
                assert_eq!(actual.as_str(), "urn:test:Visit");
            }
            other => panic!("expected MergeComorphismWrongClass, got {other:?}"),
        }
    }

    #[test]
    fn witness_missing_merge_target_class_is_rejected() {
        // D37 §5.3: every `MergeComorphism` must carry
        // `merge_target_class`. The resolver enforces it at apply
        // time as a defensive check (the commit-time validator in
        // PR 2 catches it earlier, but this guards against pre-D37
        // resources that might still be reachable on disk).
        let no_class_witness = make_resource(
            "urn:test:no_class_witness",
            &[wk::MERGE_COMORPHISM],
            &[(
                wk::MERGE_TRANSFORMATION,
                Value::ResourceRef(Iri::parse("urn:test:term_placeholder").unwrap()),
            )],
        );
        let (span, backend) =
            build_span_with_iri_collision_and_optional_witness(Some(no_class_witness));
        let conflicts = classify_conflicts(&span, &backend).unwrap();
        let conflict_id = conflicts[0].id.clone();

        let result = merge_with_resolutions(
            &span,
            vec![MergeResolution::Witness {
                conflict: conflict_id,
                comorphism: iri("urn:test:no_class_witness"),
            }],
            Vec::new(),
            Vec::new(),
            crate::layer::LayerStorage::in_memory(),
            &backend,
        );
        match result {
            Err(MergeError::MalformedMergeComorphism { iri: i, reason }) => {
                assert_eq!(i.as_str(), "urn:test:no_class_witness");
                assert!(
                    reason.contains("merge_target_class"),
                    "reason should mention the missing property; got {reason:?}"
                );
            }
            other => panic!("expected MalformedMergeComorphism, got {other:?}"),
        }
    }

    #[test]
    fn witness_with_unresolvable_transformation_surfaces_transformation_not_found() {
        // The happy validation path with a placeholder transformation
        // IRI: conflict id exists, comorphism IRI resolves to a
        // well-formed `MergeComorphism`, but its `merge_transformation`
        // points at a term that wasn't committed. The witness
        // application step surfaces `TransformationNotFound` — the
        // real end-to-end happy path is exercised in
        // `merge_with_resolutions_commits_witness_resolution_to_real_layer`.
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
            Vec::new(),
            Vec::new(),
            crate::layer::LayerStorage::in_memory(),
            &backend,
        );
        match result {
            Err(MergeError::TransformationNotFound {
                comorphism,
                transformation,
            }) => {
                assert_eq!(comorphism.as_str(), "urn:test:witness");
                assert_eq!(transformation.as_str(), "urn:test:term_placeholder");
            }
            other => panic!("expected TransformationNotFound, got {other:?}"),
        }
    }

    #[test]
    fn malformed_merge_comorphism_missing_transformation_is_rejected() {
        // MergeComorphism resource lacks the required
        // `merge_transformation` property — the resolver detects
        // this rather than the application path discovering it at
        // evaluation time.
        // Carry the class field so the missing-transformation check
        // is what the resolver hits (otherwise the missing-class
        // check would fire first and the test's error variant would
        // be wrong).
        let malformed = make_resource(
            "urn:test:malformed_witness",
            &[wk::MERGE_COMORPHISM],
            &[(
                wk::MERGE_TARGET_CLASS,
                Value::ResourceRef(Iri::parse("urn:test:Patient").unwrap()),
            )],
        );
        let (span, backend) = build_span_with_iri_collision_and_optional_witness(Some(malformed));
        let conflicts = classify_conflicts(&span, &backend).unwrap();
        let conflict_id = conflicts[0].id.clone();

        let result = merge_with_resolutions(
            &span,
            vec![MergeResolution::Witness {
                conflict: conflict_id,
                comorphism: iri("urn:test:malformed_witness"),
            }],
            Vec::new(),
            Vec::new(),
            crate::layer::LayerStorage::in_memory(),
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

    #[test]
    fn witness_off_span_resolved_via_extra_branches() {
        // D38 §4: a `MergeComorphism` committed on `witness-library`
        // (a branch outside the merge span) is invisible to the
        // span-only walk but reachable when the caller names that
        // branch in `extra_branches`.
        let (span, backend, witness_iri, _storage) =
            build_witness_fixture_offspan(make_var_resource("b"));
        let topology = backend.load_topology().unwrap();

        // Span-only search misses it.
        let span_only = resolve_merge_comorphism(
            &witness_iri,
            &iri("urn:test:Patient"),
            &span,
            &[],
            &topology,
            &*backend,
        );
        assert!(
            matches!(span_only, Err(MergeError::MergeComorphismNotFound(_))),
            "span-only search must miss the off-span witness, got {span_only:?}",
        );

        // With the search branch, the fourth-tier walk finds it.
        let with_scope = resolve_merge_comorphism(
            &witness_iri,
            &iri("urn:test:Patient"),
            &span,
            &["witness-library".to_string()],
            &topology,
            &*backend,
        )
        .expect("extra_branches must surface the off-span witness");
        assert_eq!(with_scope.iri, witness_iri);
    }

    #[test]
    fn witness_off_span_unknown_branch_silently_skipped() {
        // Unknown branch names in `extra_branches` are skipped
        // (best-effort search). The error stays `MergeComorphismNotFound`
        // rather than turning into a per-branch lookup error — a
        // stale picker state shouldn't promote to a hard failure.
        let (span, backend, witness_iri, _storage) =
            build_witness_fixture_offspan(make_var_resource("b"));
        let topology = backend.load_topology().unwrap();
        let result = resolve_merge_comorphism(
            &witness_iri,
            &iri("urn:test:Patient"),
            &span,
            &["does-not-exist".to_string()],
            &topology,
            &*backend,
        );
        assert!(
            matches!(result, Err(MergeError::MergeComorphismNotFound(_))),
            "unknown branch must be skipped silently, got {result:?}",
        );
    }

    #[test]
    fn witness_off_span_wrong_class_still_rejected() {
        // The fourth-tier search relaxes *where* the witness can
        // live, not which witnesses are valid. A class mismatch
        // must still surface as `MergeComorphismWrongClass`.
        let (span, backend, witness_iri, _storage) =
            build_witness_fixture_offspan(make_var_resource("b"));
        let topology = backend.load_topology().unwrap();
        let result = resolve_merge_comorphism(
            &witness_iri,
            &iri("urn:test:Visit"), // wrong class
            &span,
            &["witness-library".to_string()],
            &topology,
            &*backend,
        );
        match result {
            Err(MergeError::MergeComorphismWrongClass {
                expected, actual, ..
            }) => {
                assert_eq!(expected.as_str(), "urn:test:Visit");
                assert_eq!(actual.as_str(), "urn:test:Patient");
            }
            other => panic!("expected MergeComorphismWrongClass, got {other:?}"),
        }
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
        // A transformation Resource that ISN'T a EigenTT term (no
        // recognised `is_a`) makes `parse_expression` fail. Surfaces
        // as `TransformationParseError` rather than a panic or
        // generic storage error.
        let transformation_iri = "urn:test:term:bogus";
        let bogus_term = make_resource(transformation_iri, &["urn:test:NotATerm"], &[]);
        let witness_iri = "urn:test:witness";
        let witness = make_resource(
            witness_iri,
            &[wk::MERGE_COMORPHISM],
            &[
                (
                    wk::MERGE_TRANSFORMATION,
                    Value::ResourceRef(iri(transformation_iri)),
                ),
                (
                    wk::MERGE_TARGET_CLASS,
                    Value::ResourceRef(iri("urn:test:Patient")),
                ),
            ],
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
        let handle = resolve_merge_comorphism(
            &iri(witness_iri),
            &iri("urn:test:Patient"),
            &span,
            &[],
            &topology,
            &*backend,
        )
        .unwrap();

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
            &[
                (
                    wk::MERGE_TRANSFORMATION,
                    Value::ResourceRef(iri(transformation_iri)),
                ),
                (
                    wk::MERGE_TARGET_CLASS,
                    Value::ResourceRef(iri("urn:test:Patient")),
                ),
            ],
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
        let handle = resolve_merge_comorphism(
            &iri(witness_iri),
            &iri("urn:test:Patient"),
            &span,
            &[],
            &topology,
            &*backend,
        )
        .unwrap();

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
}
