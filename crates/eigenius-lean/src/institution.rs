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

//! `LeanInstitution` — Lean 4 verification institution per
//! [D28](../../docs/design/d28-lean-4-as-institution.md).
//!
//! The kernel binary instantiates one of these at startup
//! ([`super::startup::register`]) and the chain-scan registration pass
//! (`kernel::capability::registration::register_in_process_institutions`)
//! wires it into [`kernel::institution::runtime::InstitutionRuntime`]
//! whenever it encounters the `lean:lean_institution` Institution
//! resource declared by [`ontologies/lean/lean-institution.eigon.json`].
//!
//! ## Surface in 20a.4
//!
//! - `query(proof_check, LeanProofTerm)` — extracts the
//!   referenced `LeanProofPayload`'s `payload_bytes`, reads the
//!   `target_name`, runs the v1 `check_proof` (axiom allowlist empty —
//!   `LeanEnvironment` integration arrives in 20a.5), and returns a
//!   `Verdict::Holds | Fails { diagnostic }` resource.
//! - `query(which_axioms, …)` — `NotImplemented` (the QueryClass is
//!   declared on chain so the procedure IRI is bound, but the v1
//!   institution doesn't compute the axiom list yet).
//! - `extract_typed(ef_lean_proof_payload, LeanProofTerm)` — returns
//!   the payload bytes wrapped as `Val::ResourceVal({core:string →
//!   bytes})`, matching the convention `kernel::nbe::eval::
//!   resource_value_to_val` uses for string-typed values.
//! - `reify` — `NotImplemented`. Lean has no `ImportFormat`s yet;
//!   construction is authoring-side via the chain-mirror translator,
//!   not via a kernel `reify` call.
//!
//! ## Correspondence check (D28 §5.5)
//!
//! Three checks run in order:
//!
//! 1. **Proof validity** — nanoda's `check_proof`. Same as 20a.4.
//! 2. **Mirror correspondence** — resolve `mirror_iri` to a
//!    `LeanPackageMirror`, verify its `source_layer` is reachable
//!    from `head` (proof anchored to an ancestor-or-equal of the
//!    layer the check runs against), and confirm the mirror covers
//!    the claim's class via `mirrored_classes`. Lacking either
//!    raises `FFIVersionMismatch`.
//! 3. **Anchor consistency** — recompute the
//!    `library_content_hash` over the embedded archive and confirm
//!    it matches the declared hash. Mismatch surfaces as
//!    `AnchorContentHashMismatch`.
//!
//! A `LeanProofTerm` without `mirror_iri` skips checks 2 + 3 — the
//! verdict reflects nanoda alone, matching the 20a.4 behavior for
//! proofs not yet pinned to a chain-level claim.
//!
//! ### Structural correspondence (D28 §5.5 ¶2 final sentence)
//!
//! When the `LeanProofTerm` carries a `proposition` — a
//! chain-mirrored `lean:LeanExpr` (D40) value — the check walks
//! that tree, collects every `Const` reference under the
//! `EigeniusFFI` namespace, and verifies at least one maps back
//! (via the mirror's `mirrored_classes` + each class's
//! `core:short_name`) to the claim's class IRI. Failure surfaces
//! as `PropositionMismatch` (D28 §9.1) with a diagnostic listing
//! what the proposition *does* reference.
//!
//! The proposition is recommended-not-required (D28 §6.3). Absent
//! → structural check is skipped; the covering check (class IRI ∈
//! `mirrored_classes`) is the only correspondence gate. Once the
//! orchestrator's commit pipeline guarantees `proposition`
//! population for every committed proof, a future spec version may
//! upgrade absent-proposition to a hard rejection.

use std::sync::Arc;

use eigenius_kernel::context::ExecutionContext;
use eigenius_kernel::institution::error::InstitutionError;
use eigenius_kernel::institution::runtime::{Institution, QueryOutcome};
use eigenius_kernel::nbe::val::Val;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;

use crate::checker::{check_proof, Verdict};

/// Well-known IRIs the institution dispatches on. Keeping them in one
/// place so a downstream caller building a `LeanProofTerm` resource
/// can reach for the same strings without spelling them out.
pub mod iris {
    /// The institution itself.
    pub const INSTITUTION: &str = "urn:eigenius:lean:lean_institution";

    /// AutoOnLoad / OnDemand procedure: verify a `LeanProofTerm` and
    /// return a `Verdict`.
    pub const PROC_PROOF_CHECK: &str = "urn:eigenius:lean:proc:proof_check";

    /// OnDemand procedure: list axioms a proof transitively depends on.
    /// `NotImplemented` in v1 — see module docstring.
    pub const PROC_WHICH_AXIOMS: &str = "urn:eigenius:lean:proc:which_axioms";

    /// ExportFormat procedure: extract a `LeanProofTerm`'s referenced
    /// payload bytes as a `core:string`-typed EigenTT value.
    pub const PROC_EXTRACT_PROOF_PAYLOAD: &str = "urn:eigenius:lean:proc:extract_proof_payload";

    /// Property: `LeanProofTerm.proof_payload` (resource ref →
    /// `LeanProofPayload`).
    pub const PROP_PROOF_PAYLOAD: &str = "urn:eigenius:lean:proof_payload";

    /// Property: `LeanProofPayload.payload_bytes` (string).
    pub const PROP_PAYLOAD_BYTES: &str = "urn:eigenius:lean:payload_bytes";

    /// Property: `LeanProofTerm.target_name` (string).
    pub const PROP_TARGET_NAME: &str = "urn:eigenius:lean:target_name";

    /// Property: `LeanProofTerm.mirror_iri` — IRI of the
    /// `LeanPackageMirror` the proof's proposition is anchored to.
    /// Recommended on chain (20a.7+); absent means "verify under
    /// nanoda alone, no chain-claim correspondence".
    pub const PROP_MIRROR_IRI: &str = "urn:eigenius:lean:mirror_iri";

    /// Property: `LeanProofTerm.claim_iri` — IRI of the Eigon
    /// claim resource the proof discharges. v1 reads it for
    /// mirror-coverage matching: the claim's class must appear in
    /// the mirror's `mirrored_classes`.
    pub const PROP_CLAIM_IRI: &str = "urn:eigenius:lean:claim_iri";

    /// Property: `LeanProofTerm.proposition` — chain-mirrored
    /// `lean:LeanExpr` (D40 §3.4) tagged-dict tree carrying the
    /// theorem's *type*. v1 of the correspondence check walks this
    /// tree to confirm the proof actually reasons about the claim's
    /// class via a mirror type (D28 §5.5 ¶2 final sentence). Absent
    /// proposition → structural check is skipped.
    pub const PROP_PROPOSITION: &str = "urn:eigenius:lean:proposition";

    /// Property attached to a `Verdict::Fails` carrying the
    /// human-readable refusal reason (D31 §6.3 / institution ontology).
    pub const PROP_DIAGNOSTIC: &str = "urn:eigenius:institution:diagnostic";

    // ── LeanPackageMirror properties (D26 §5.4) — read by the
    // correspondence check. Constants mirror the substrate-side
    // properties that `mirror_to_resource` in `eigenius-lean-runtime`
    // stamps onto each generated mirror.
    pub const PROP_MIRROR_SOURCE_LAYER: &str = "urn:eigenius:runtime:source_layer";
    pub const PROP_MIRROR_LIB_CONTENT_HASH: &str = "urn:eigenius:runtime:library_content_hash";
    pub const PROP_MIRROR_LIB_CONTENT: &str = "urn:eigenius:runtime:library_content";
    pub const PROP_MIRRORED_CLASSES: &str = "urn:eigenius:runtime:mirrored_classes";

    // ── Diagnostic kinds (D28 §9.1). Prefixed onto the diagnostic
    // string so consumers can match by leading token. Single-string
    // shape matches the existing `PROP_DIAGNOSTIC` flat surface.
    pub(crate) const DIAG_FFI_VERSION_MISMATCH: &str = "FFIVersionMismatch";
    pub(crate) const DIAG_ANCHOR_CONTENT_HASH_MISMATCH: &str = "AnchorContentHashMismatch";
    pub(crate) const DIAG_PROPOSITION_MISMATCH: &str = "PropositionMismatch";

    /// Lean namespace the mirror generator emits structures under
    /// (D30 §2.4: `namespace EigeniusFFI`). A `Const` node in the
    /// proposition with a name like `EigeniusFFI.Patient` is a
    /// mirror-type reference; the structural correspondence check
    /// finds these and maps the suffix back to a chain class IRI.
    pub(crate) const MIRROR_NAMESPACE: &str = "EigeniusFFI";
}

/// IRI scheme prefix for wrapping a hex `LayerId` into a valid
/// `urn:eigenius:core:formats:iri`-conforming string. Used by
/// `RuntimePackageMirror.source_layer` and the institution's
/// ancestry check (D28 §5.5). Two-way mapping: stripping this
/// prefix from a stored `source_layer` value yields the hex form
/// the layer chain stores directly on `Layer::id()`.
const LAYER_IRI_PREFIX: &str = "urn:eigenius:layer:";

/// In-process Lean 4 verification institution.
///
/// Stateless — every `query` call parses the proof from scratch via
/// `nanoda_lib`. A future revision may cache the parsed `ExportFile`
/// keyed by content hash to amortise repeated AutoOnLoad firings of
/// the same `LeanProofPayload`; the blanket
/// `impl Institution for Arc<I>` in
/// `kernel::institution::runtime` already permits per-process state
/// without rebuilding on every registry rebuild.
pub struct LeanInstitution {
    iri: Iri,
}

impl LeanInstitution {
    /// Construct a new institution with the canonical
    /// `urn:eigenius:lean:lean_institution` IRI.
    pub fn new() -> Self {
        Self {
            iri: Iri::parse(iris::INSTITUTION).expect("static institution IRI"),
        }
    }

    /// Wrap a fresh institution in an `Arc<dyn Institution>` ready to
    /// hand to
    /// `EigeniusService::register_in_process_institution`. Convenience
    /// constructor for the startup hook.
    pub fn arc() -> Arc<dyn Institution> {
        Arc::new(Self::new())
    }
}

impl Default for LeanInstitution {
    fn default() -> Self {
        Self::new()
    }
}

impl Institution for LeanInstitution {
    fn institution_iri(&self) -> &Iri {
        &self.iri
    }

    fn extract_typed(
        &self,
        procedure_iri: &Iri,
        resource: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<Val, InstitutionError> {
        if procedure_iri.as_str() == iris::PROC_EXTRACT_PROOF_PAYLOAD {
            let payload = resolve_payload(resource, ctx)?;
            let bytes = payload_bytes(&payload)?;
            // Match `kernel::nbe::eval::resource_value_to_val`'s
            // string convention: a `Val::ResourceVal` wrapping an
            // embedded Resource that carries the string under the
            // `core:string` property. The ExportFormat's
            // `payload_type` is `core:string`, so the consumer reads
            // the property by that IRI.
            let mut wrapper = Resource::new_embedded();
            wrapper.set(
                Iri::parse(wk::STRING).expect("well-known IRI"),
                Value::String(bytes),
            );
            Ok(Val::ResourceVal(Box::new(wrapper)))
        } else {
            Err(InstitutionError::NotImplemented(format!(
                "LeanInstitution has no extract_typed handler for `{procedure_iri}`"
            )))
        }
    }

    fn reify(
        &self,
        procedure_iri: &Iri,
        _value: &Val,
        _ctx: &ExecutionContext,
    ) -> Result<Resource, InstitutionError> {
        Err(InstitutionError::NotImplemented(format!(
            "LeanInstitution has no reify handler for `{procedure_iri}` \
             (Lean institution declares no ImportFormats in 20a.4 — construction \
              is authoring-side via the chain-mirror translator)"
        )))
    }

    fn query(
        &self,
        procedure_iri: &Iri,
        input: &Resource,
        ctx: &ExecutionContext,
    ) -> Result<QueryOutcome, InstitutionError> {
        match procedure_iri.as_str() {
            iris::PROC_PROOF_CHECK => do_proof_check(input, ctx),
            iris::PROC_WHICH_AXIOMS => Err(InstitutionError::NotImplemented(
                "LeanInstitution::query(which_axioms) is not implemented in 20a.4 — \
                 the QueryClass is declared on chain so the procedure IRI is bound, \
                 but axiom-list extraction lands opportunistically"
                    .to_string(),
            )),
            _ => Err(InstitutionError::NotImplemented(format!(
                "LeanInstitution has no query handler for procedure `{procedure_iri}`"
            ))),
        }
    }
}

/// Run the core proof-check procedure: read the LeanProofTerm's
/// payload bytes + target name, call `check_proof`, and lift the
/// resulting `Verdict` into a chain-shaped `Verdict::Holds | Fails`
/// resource.
///
/// Default axiom allowlist when the `LeanProofTerm` doesn't anchor
/// to a `LeanEnvironment` that pins one. Matches D28 §7.1 — Lean's
/// four trust-the-compiler axioms. Even a trivial proof through
/// modern Lean stdlib pulls `Classical.choice` (via `Subtype`'s
/// projection helpers), so empty-allowlist is a footgun for any
/// real proof; the canonical default catches that case.
const DEFAULT_LEAN_AXIOMS: &[&str] = &[
    "propext",
    "Classical.choice",
    "Quot.sound",
    "Lean.trustCompiler",
];

fn do_proof_check(
    input: &Resource,
    ctx: &ExecutionContext,
) -> Result<QueryOutcome, InstitutionError> {
    let payload = resolve_payload(input, ctx)?;
    let bytes = payload_bytes(&payload)?;
    let target_name = input
        .get(&Iri::parse(iris::PROP_TARGET_NAME).expect("static IRI"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            InstitutionError::ComputationFailed(
                "LeanProofTerm missing required `target_name` property".to_string(),
            )
        })?
        .to_string();

    // v1 uses the canonical default allowlist (D28 §7.1). When the
    // `LeanProofTerm` carries an `environment_iri` (D28 §6.3) the
    // institution will read that env's `lean_permitted_axioms`
    // property and use it instead — that wiring lands when the
    // authoring runtime's env-resource flow into the kernel
    // commit pipeline (currently the env IRI isn't on the chain).
    let permitted_axioms: Vec<String> = DEFAULT_LEAN_AXIOMS
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    let verdict = check_proof(bytes.as_bytes(), &target_name, &permitted_axioms)
        .map_err(|e| InstitutionError::ComputationFailed(format!("nanoda check_proof: {e}")))?;

    // Check 1 (proof validity) decided — short-circuit on nanoda
    // rejection. No correspondence check runs against a proof that
    // doesn't type-check; the diagnostic would be misleading.
    if let Verdict::Fails { diagnostic } = verdict {
        return Ok(QueryOutcome::from_output(verdict_resource(
            wk::VERDICT_FAILS,
            Some(&diagnostic),
        )));
    }

    // Checks 2 + 3 (D28 §5.5) — only run when the proof carries a
    // mirror anchor. `LeanProofTerm` without `mirror_iri` is the
    // 20a.4 shape ("verify under nanoda alone, no chain claim"); we
    // preserve that path so unanchored proofs don't regress.
    if let Some(failure) = do_correspondence_check(input, ctx)? {
        return Ok(QueryOutcome::from_output(verdict_resource(
            wk::VERDICT_FAILS,
            Some(&failure),
        )));
    }

    Ok(QueryOutcome::from_output(verdict_resource(
        wk::VERDICT_HOLDS,
        None,
    )))
}

/// Run the D28 §5.5 correspondence checks against a successfully
/// type-checked proof. Returns `Some(diagnostic)` on any failure;
/// `None` when the proof has no mirror anchor (skip) or every
/// check passes.
fn do_correspondence_check(
    proof_term: &Resource,
    ctx: &ExecutionContext,
) -> Result<Option<String>, InstitutionError> {
    // The `mirror_iri` property carries the IRI of the
    // `LeanPackageMirror` resource the proof's proposition is
    // anchored to. Absent → unanchored proof → skip (return None).
    let mirror_iri_str = match proof_term
        .get(&Iri::parse(iris::PROP_MIRROR_IRI).expect("static IRI"))
        .and_then(Value::as_str)
    {
        Some(s) => s.to_string(),
        None => return Ok(None),
    };
    let mirror_iri = Iri::parse(&mirror_iri_str).map_err(|e| {
        InstitutionError::ComputationFailed(format!(
            "LeanProofTerm `mirror_iri` is not a valid IRI: {e}"
        ))
    })?;

    // Resolve the mirror Resource. Missing → FFIVersionMismatch
    // (the proof points at a mirror the chain doesn't carry).
    let mirror = match ctx.resolve(&mirror_iri) {
        Some(r) => r,
        None => {
            return Ok(Some(format_diag(
                iris::DIAG_FFI_VERSION_MISMATCH,
                &format!(
                    "LeanPackageMirror `{mirror_iri_str}` does not resolve in the layer chain"
                ),
            )));
        }
    };

    // ─── Check 3: anchor consistency ──────────────────────────────
    //
    // Recompute `library_content_hash` from the embedded archive
    // and compare to the declared value. A mismatch means the
    // mirror was tampered with between commit and verification —
    // a hard reject regardless of any other check passing.
    if let Some(diag) = check_anchor_consistency(&mirror)? {
        return Ok(Some(diag));
    }

    // ─── Check 2: mirror correspondence ───────────────────────────
    //
    // Two sub-checks:
    //   (a) The mirror's `source_layer` is reachable from the
    //       verification context's head (proof anchored to an
    //       ancestor-or-equal of the layer the check runs against).
    //   (b) The claim's class is in the mirror's
    //       `mirrored_classes` set.
    //
    // (a) failure → FFIVersionMismatch ("the mirror was generated
    // against a layer outside this branch"). (b) failure →
    // FFIVersionMismatch ("the mirror doesn't cover this class").
    if let Some(diag) = check_mirror_anchor_reachable(&mirror, ctx) {
        return Ok(Some(diag));
    }
    if let Some(diag) = check_mirror_covers_claim_class(proof_term, &mirror, ctx)? {
        return Ok(Some(diag));
    }

    // ─── Check 2c: structural correspondence (D28 §5.5 ¶2 final) ──
    //
    // Walk the proposition's chain-mirrored `lean:LeanExpr` (D40)
    // and confirm at least one of the mirror types it references
    // maps back to the claim's class IRI. Skipped when the
    // proposition is absent — `LeanProofTerm.proposition` is
    // recommended, not required.
    if let Some(diag) = check_proposition_structural_correspondence(proof_term, &mirror, ctx)? {
        return Ok(Some(diag));
    }

    Ok(None)
}

/// Recompute SHA-256 over the mirror's embedded archive and verify
/// it matches the declared `library_content_hash`. Uses the same
/// length-prefixed framing as `eigenius_lean_runtime::mirror_gen::
/// library_content_hash` so the two computations agree.
fn check_anchor_consistency(mirror: &Resource) -> Result<Option<String>, InstitutionError> {
    let declared = mirror
        .get(&Iri::parse(iris::PROP_MIRROR_LIB_CONTENT_HASH).expect("static IRI"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            InstitutionError::ComputationFailed(
                "LeanPackageMirror missing `library_content_hash` property".to_string(),
            )
        })?;

    let lib_json = mirror
        .get(&Iri::parse(iris::PROP_MIRROR_LIB_CONTENT).expect("static IRI"))
        .ok_or_else(|| {
            InstitutionError::ComputationFailed(
                "LeanPackageMirror missing `library_content` property".to_string(),
            )
        })?;
    let lib_json = match lib_json {
        Value::Json(v) => v,
        other => {
            return Err(InstitutionError::ComputationFailed(format!(
                "`library_content` must be JSON, got {other:?}"
            )));
        }
    };

    let actual = recompute_library_content_hash(lib_json)?;
    if actual != declared {
        return Ok(Some(format_diag(
            iris::DIAG_ANCHOR_CONTENT_HASH_MISMATCH,
            &format!("library_content_hash mismatch: declared `{declared}`, recomputed `{actual}`"),
        )));
    }
    Ok(None)
}

/// Walk `head`'s parent chain looking for a layer whose `id()`
/// matches the mirror's `source_layer`. Source layers are stored
/// as a hex-encoded `LayerId` (`Display`-formatted) when committed
/// by the substrate's image-build pipeline.
fn check_mirror_anchor_reachable(mirror: &Resource, ctx: &ExecutionContext) -> Option<String> {
    let source_layer = match mirror
        .get(&Iri::parse(iris::PROP_MIRROR_SOURCE_LAYER).expect("static IRI"))
        .and_then(Value::as_str)
    {
        Some(s) => s.to_string(),
        None => {
            // Missing source_layer is malformed — surface as a
            // version mismatch with a descriptive message.
            return Some(format_diag(
                iris::DIAG_FFI_VERSION_MISMATCH,
                "LeanPackageMirror missing `source_layer` property",
            ));
        }
    };

    // `source_layer` may be one of three shapes, accumulated by
    // convention:
    //   (a) Bare hex `LayerId` string (the historical pattern used by
    //       capstone_test + correspondence_test).
    //   (b) Layer name (set when the substrate names its working
    //       layer something semantically meaningful).
    //   (c) `urn:eigenius:layer:<hex>` IRI form (required by the
    //       ontology's `format = urn:eigenius:core:formats:iri`
    //       constraint on `RuntimePackageMirror.source_layer`;
    //       Eigon-JSON loads going through `commit_with_validation`
    //       reject (a) and (b) at format-check time).
    //
    // We accept all three so existing in-process tests and the
    // documented IRI-form fixture both pass through the same check.
    let stripped_hex = source_layer
        .strip_prefix(LAYER_IRI_PREFIX)
        .map(str::to_string);
    let mut cur = Some(ctx.head().clone());
    while let Some(layer) = cur {
        let hex_id = layer.id().to_string();
        if hex_id == source_layer
            || layer.name() == source_layer
            || stripped_hex.as_deref() == Some(hex_id.as_str())
        {
            return None;
        }
        cur = layer.parent().cloned();
    }
    Some(format_diag(
        iris::DIAG_FFI_VERSION_MISMATCH,
        &format!(
            "mirror `source_layer = {source_layer}` is not in the verification context's ancestry"
        ),
    ))
}

/// Verify the claim's class IRI appears in the mirror's
/// `mirrored_classes` set. Resolves `claim_iri` → claim Resource
/// → `is_a` (first entry) → compares against the mirror's list.
///
/// v1 weaker check (D28 §5.5 ¶2): the *full* structural match
/// between the proposition's mirror type and the claim's class
/// requires walking the chain-mirrored `LeanExpr` (D40), which the
/// current handler treats as opaque. Covering-set membership is
/// strictly looser but catches the common case D28 §5.6 calls
/// out: a mirror anchored to L₀ that doesn't include a class the
/// claim references.
fn check_mirror_covers_claim_class(
    proof_term: &Resource,
    mirror: &Resource,
    ctx: &ExecutionContext,
) -> Result<Option<String>, InstitutionError> {
    let claim_iri_str = match proof_term
        .get(&Iri::parse(iris::PROP_CLAIM_IRI).expect("static IRI"))
        .and_then(Value::as_str)
    {
        Some(s) => s,
        None => {
            // mirror_iri without claim_iri is an authoring gap;
            // skip the class-coverage check rather than refusing.
            // The substrate's commit-time validator should reject
            // such resources before they reach AutoOnLoad anyway.
            return Ok(None);
        }
    };
    let claim_iri = Iri::parse(claim_iri_str).map_err(|e| {
        InstitutionError::ComputationFailed(format!(
            "LeanProofTerm `claim_iri` is not a valid IRI: {e}"
        ))
    })?;
    let claim = match ctx.resolve(&claim_iri) {
        Some(r) => r,
        None => {
            return Ok(Some(format_diag(
                iris::DIAG_FFI_VERSION_MISMATCH,
                &format!("claim resource `{claim_iri_str}` does not resolve"),
            )));
        }
    };

    let claim_class = match claim
        .get(&Iri::parse(wk::IS_A).expect("well-known IRI"))
        .map(Value::as_iri_array)
    {
        Some(arr) if !arr.is_empty() => arr[0].clone(),
        _ => {
            return Ok(Some(format_diag(
                iris::DIAG_FFI_VERSION_MISMATCH,
                &format!("claim resource `{claim_iri_str}` has no `is_a` class"),
            )));
        }
    };

    let mirrored = mirror
        .get(&Iri::parse(iris::PROP_MIRRORED_CLASSES).expect("static IRI"))
        .map(Value::as_iri_array)
        .unwrap_or_default();
    if !mirrored.iter().any(|i| i == &claim_class) {
        return Ok(Some(format_diag(
            iris::DIAG_FFI_VERSION_MISMATCH,
            &format!(
                "claim class `{}` is not in the mirror's `mirrored_classes` set",
                claim_class.as_str()
            ),
        )));
    }
    Ok(None)
}

/// Verify the proposition's mirror-typed references include the
/// claim's class (D28 §5.5 ¶2 final sentence — "the mirror type
/// referenced in the proposition must correspond structurally to
/// that class").
///
/// Algorithm:
///   1. Read `LeanProofTerm.proposition` — chain-mirrored
///      `lean:LeanExpr` (D40) value as `serde_json::Value` tagged
///      dict. Skip if absent.
///   2. Walk the tree collecting every `Const` whose `Name` lives
///      under the `EigeniusFFI` namespace — those are mirror-type
///      references. The collected names are the Lean short names
///      (e.g. `Patient` from `EigeniusFFI.Patient`).
///   3. Build a `short_name → class_iri` map by resolving every
///      IRI in `mirror.mirrored_classes` and reading
///      `core:short_name`.
///   4. Map collected short names through the table; verify the
///      claim's class IRI appears in the resulting set.
///
/// Failure → `PropositionMismatch` (D28 §9.1) with a diagnostic
/// naming the claim's class + the set of mirror types the
/// proposition actually references.
fn check_proposition_structural_correspondence(
    proof_term: &Resource,
    mirror: &Resource,
    ctx: &ExecutionContext,
) -> Result<Option<String>, InstitutionError> {
    let proposition = match proof_term.get(&Iri::parse(iris::PROP_PROPOSITION).expect("static IRI"))
    {
        Some(Value::Json(j)) => j,
        // Proposition is recommended-not-required (D28 §6.3). Absent
        // means the authoring side didn't run the chain-mirror
        // translator yet — the covering check alone has to suffice.
        // Future versions may upgrade this to a hard rejection once
        // the orchestrator's commit pipeline guarantees the
        // proposition's presence.
        None => return Ok(None),
        Some(other) => {
            return Err(InstitutionError::ComputationFailed(format!(
                "LeanProofTerm `proposition` must be JSON, got {other:?}"
            )));
        }
    };

    let referenced_short_names = collect_mirror_short_names(proposition);

    // Resolve the claim's class IRI — same path as the covering
    // check; duplicated here so the structural check is
    // self-contained.
    let claim_iri_str = match proof_term
        .get(&Iri::parse(iris::PROP_CLAIM_IRI).expect("static IRI"))
        .and_then(Value::as_str)
    {
        Some(s) => s,
        None => return Ok(None), // No claim → nothing to correspond to; covering check already skipped.
    };
    let claim_iri = Iri::parse(claim_iri_str)
        .map_err(|e| InstitutionError::ComputationFailed(format!("invalid claim_iri: {e}")))?;
    let claim = match ctx.resolve(&claim_iri) {
        Some(r) => r,
        None => return Ok(None), // covering check already raised FFIVersionMismatch
    };
    let claim_class = match claim
        .get(&Iri::parse(wk::IS_A).expect("well-known IRI"))
        .map(Value::as_iri_array)
    {
        Some(arr) if !arr.is_empty() => arr[0].clone(),
        _ => return Ok(None),
    };

    // Build the short_name → class_iri map from the mirror's
    // `mirrored_classes`. Each entry resolves to the class
    // Resource (in the same layer chain); we read its
    // `core:short_name` to determine the Lean side name the
    // generator would have stamped.
    let mut short_to_iri: std::collections::BTreeMap<String, Iri> =
        std::collections::BTreeMap::new();
    let mirrored = mirror
        .get(&Iri::parse(iris::PROP_MIRRORED_CLASSES).expect("static IRI"))
        .map(Value::as_iri_array)
        .unwrap_or_default();
    for class_iri in mirrored {
        let cls = match ctx.resolve(&class_iri) {
            Some(c) => c,
            None => continue, // unresolvable class — skip; covering check would have caught broader mirror issues
        };
        let short = cls
            .get(&Iri::parse(wk::SHORT_NAME).expect("well-known IRI"))
            .and_then(Value::as_str);
        if let Some(s) = short {
            short_to_iri.insert(s.to_string(), class_iri);
        }
    }

    // For each mirror-type short name the proposition references,
    // map back to the chain class IRI. Match against the claim's
    // class.
    let referenced_class_iris: std::collections::BTreeSet<&Iri> = referenced_short_names
        .iter()
        .filter_map(|s| short_to_iri.get(s))
        .collect();
    if referenced_class_iris.contains(&&claim_class) {
        return Ok(None);
    }

    // Build a descriptive diagnostic listing what the proposition
    // actually references so the user can see the version-skew or
    // wrong-proposition shape at a glance.
    let referenced_iri_strs: Vec<String> = referenced_class_iris
        .iter()
        .map(|i| i.as_str().to_string())
        .collect();
    let referenced_summary = if referenced_iri_strs.is_empty() {
        format!(
            "proposition references no mirror types (collected short names: [{}])",
            referenced_short_names
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else {
        format!(
            "proposition references mirror classes [{}]",
            referenced_iri_strs.join(", ")
        )
    };
    Ok(Some(format_diag(
        iris::DIAG_PROPOSITION_MISMATCH,
        &format!(
            "claim class `{}` not among the mirror types the proposition reasons about; {referenced_summary}",
            claim_class.as_str()
        ),
    )))
}

/// Walk a chain-mirrored `lean:LeanExpr` tree (D40 §3.4 tagged-dict
/// shape) collecting every `Const` whose decoded `Name` lives
/// under the `EigeniusFFI` namespace. Returns the suffix short
/// names — e.g. a `Const "EigeniusFFI.Patient"` yields
/// `"Patient"`.
///
/// The walker handles every D40 ctor that nests sub-expressions
/// (`App`, `Pi`, `Lambda`, `Let`, `Proj`) so a mirror-type
/// reference buried in a deeply-nested binder is still found.
/// Non-`EigeniusFFI` `Const` references (Lean stdlib types,
/// project-local types) are silently ignored — they're not in the
/// mirror's responsibility.
fn collect_mirror_short_names(value: &serde_json::Value) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    walk_lean_expr(value, &mut out);
    out
}

fn walk_lean_expr(value: &serde_json::Value, out: &mut std::collections::BTreeSet<String>) {
    let Some(ctor) = value.get("ctor").and_then(|v| v.as_str()) else {
        return;
    };
    let args = value.get("args").and_then(|v| v.as_array());
    match (ctor, args) {
        ("Const", Some(args)) if !args.is_empty() => {
            if let Some(short) = mirror_short_name_from_lean_name(&args[0]) {
                out.insert(short);
            }
            // levels (args[1]) carries LeanLevel values; those don't
            // reference mirror types so we don't recurse.
        }
        ("App", Some(args)) if args.len() == 2 => {
            walk_lean_expr(&args[0], out); // fun
            walk_lean_expr(&args[1], out); // arg
        }
        ("Pi" | "Lambda", Some(args)) if args.len() == 4 => {
            // [binder_name, binder_style, binder_type, body]
            walk_lean_expr(&args[2], out);
            walk_lean_expr(&args[3], out);
        }
        ("Let", Some(args)) if args.len() == 5 => {
            // [binder_name, binder_type, val, body, nondep]
            walk_lean_expr(&args[1], out);
            walk_lean_expr(&args[2], out);
            walk_lean_expr(&args[3], out);
        }
        ("Proj", Some(args)) if args.len() == 3 => {
            // [ty_name, idx, structure]
            walk_lean_expr(&args[2], out);
        }
        // Var / Sort / StringLit / NatLit have no nested LeanExpr
        // children; nothing to recurse into.
        _ => {}
    }
}

/// Decode a `lean:LeanName` tagged dict (D40 §3.1) into the
/// Lean-side short name **if** the name lives under the
/// `EigeniusFFI` namespace. Returns `None` for any other shape.
///
/// `Str(Str(Anon, "EigeniusFFI"), "Patient")` → `Some("Patient")`.
/// `Str(Anon, "Nat")` → `None` (not under EigeniusFFI).
/// `Num(...)` → `None` (mirror class names are string-suffixed).
fn mirror_short_name_from_lean_name(value: &serde_json::Value) -> Option<String> {
    let ctor = value.get("ctor")?.as_str()?;
    if ctor != "Str" {
        return None;
    }
    let args = value.get("args")?.as_array()?;
    if args.len() != 2 {
        return None;
    }
    let suffix = args[1].as_str()?.to_string();
    let prefix = &args[0];
    let prefix_ctor = prefix.get("ctor")?.as_str()?;
    match prefix_ctor {
        // Top-level (single-segment) name — `EigeniusFFI` itself
        // appears as `Str(Anon, "EigeniusFFI")` and isn't a class
        // reference; we filter it out here.
        "Anon" => None,
        "Str" => {
            // Two-segment names — confirm the leading segment is
            // `EigeniusFFI` and return the suffix.
            let prefix_args = prefix.get("args")?.as_array()?;
            if prefix_args.len() != 2 {
                return None;
            }
            let leading = prefix_args[1].as_str()?;
            if leading != iris::MIRROR_NAMESPACE {
                return None;
            }
            // The leading segment must itself be rooted at Anon
            // (single-segment namespace). Deeper namespaces like
            // `Project.EigeniusFFI.Patient` aren't generator output;
            // reject them here so a malicious authoring path can't
            // sneak past the check with a near-match name.
            let nested = prefix_args[0].get("ctor")?.as_str()?;
            if nested != "Anon" {
                return None;
            }
            Some(suffix)
        }
        _ => None,
    }
}

/// Compute the substrate-style `library_content_hash` over a
/// `library_content` JSON value (the `{"kind": "embedded", "files":
/// [{"path", "content_b64"}]}` shape). Mirrors
/// `eigenius_lean_runtime::mirror_gen::library_content_hash` —
/// path-sorted, length-prefixed framing, SHA-256. The two
/// implementations are deliberately duplicated to keep the
/// verification side dep-free; the test suite cross-checks them.
fn recompute_library_content_hash(
    lib_json: &serde_json::Value,
) -> Result<String, InstitutionError> {
    let kind = lib_json
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            InstitutionError::ComputationFailed(
                "`library_content` missing string `kind` field".to_string(),
            )
        })?;
    if kind != "embedded" {
        // Recomputing an external-archive hash requires fetching
        // the referenced bytes, which the verification path doesn't
        // do. Treat as a configuration error rather than a
        // correspondence failure — external libraries aren't a v1
        // surface (D26 §7.2 future-work).
        return Err(InstitutionError::ComputationFailed(format!(
            "library_content `kind = \"{kind}\"` cannot be rehashed in v1 (only `embedded` supported)"
        )));
    }
    let files = lib_json
        .get("files")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            InstitutionError::ComputationFailed(
                "`library_content.files` missing or not an array".to_string(),
            )
        })?;

    let mut sorted_pairs: Vec<(String, Vec<u8>)> = Vec::with_capacity(files.len());
    for (idx, f) in files.iter().enumerate() {
        let path = f.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            InstitutionError::ComputationFailed(format!(
                "`library_content.files[{idx}].path` missing or not a string"
            ))
        })?;
        let b64 = f
            .get("content_b64")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                InstitutionError::ComputationFailed(format!(
                    "`library_content.files[{idx}].content_b64` missing or not a string"
                ))
            })?;
        let bytes = base64_decode(b64).map_err(|e| {
            InstitutionError::ComputationFailed(format!(
                "library_content.files[{idx}].content_b64 (path `{path}`) is not valid base64: {e}"
            ))
        })?;
        sorted_pairs.push((path.to_string(), bytes));
    }
    sorted_pairs.sort_by(|a, b| a.0.cmp(&b.0));

    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for (path, bytes) in &sorted_pairs {
        hasher.update((path.len() as u64).to_be_bytes());
        hasher.update(path.as_bytes());
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(bytes);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

/// Standard base64 decoder (RFC 4648 §4) — mirrors the encoder in
/// `eigenius_lean_runtime::mirror_gen::base64_encode`.
fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let cleaned: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !cleaned.len().is_multiple_of(4) {
        return Err(format!(
            "input length {} not a multiple of 4",
            cleaned.len()
        ));
    }
    let mut out = Vec::with_capacity(cleaned.len() / 4 * 3);
    let mut i = 0;
    while i < cleaned.len() {
        let chunk = &cleaned[i..i + 4];
        let pad = chunk.iter().filter(|&&b| b == b'=').count();
        let v0 = val(chunk[0]).ok_or_else(|| format!("invalid byte {:?}", chunk[0] as char))?;
        let v1 = val(chunk[1]).ok_or_else(|| format!("invalid byte {:?}", chunk[1] as char))?;
        let v2 = if chunk[2] == b'=' {
            0
        } else {
            val(chunk[2]).ok_or_else(|| format!("invalid byte {:?}", chunk[2] as char))?
        };
        let v3 = if chunk[3] == b'=' {
            0
        } else {
            val(chunk[3]).ok_or_else(|| format!("invalid byte {:?}", chunk[3] as char))?
        };
        let n = ((v0 as u32) << 18) | ((v1 as u32) << 12) | ((v2 as u32) << 6) | (v3 as u32);
        out.push(((n >> 16) & 0xff) as u8);
        if pad < 2 {
            out.push(((n >> 8) & 0xff) as u8);
        }
        if pad < 1 {
            out.push((n & 0xff) as u8);
        }
        i += 4;
    }
    Ok(out)
}

/// Format a diagnostic string: `<KIND>: <message>`. D28 §9.1 ships
/// diagnostics as a single field; the leading token lets consumers
/// pattern-match on the failure kind without growing the Verdict
/// resource's property surface.
fn format_diag(kind: &str, message: &str) -> String {
    format!("{kind}: {message}")
}

/// Resolve a LeanProofTerm's `proof_payload` reference into the
/// concrete `LeanProofPayload` resource. Accepts both
/// `Value::Embedded` (inline payload) and `Value::ResourceRef`
/// (referenced separately) shapes — the kernel canonicaliser may have
/// left either depending on whether the caller embedded the payload
/// or registered it as a top-level resource.
fn resolve_payload(term: &Resource, ctx: &ExecutionContext) -> Result<Resource, InstitutionError> {
    let prop_iri = Iri::parse(iris::PROP_PROOF_PAYLOAD).expect("static IRI");
    let value = term.get(&prop_iri).ok_or_else(|| {
        InstitutionError::ComputationFailed(
            "LeanProofTerm missing required `proof_payload` property".to_string(),
        )
    })?;
    match value {
        Value::Embedded(boxed) => Ok((**boxed).clone()),
        Value::ResourceRef(payload_iri) => ctx.resolve(payload_iri).map(|arc| (*arc).clone()).ok_or_else(|| {
            InstitutionError::MissingDependency(format!(
                "LeanProofPayload `{payload_iri}` referenced by `proof_payload` does not resolve in the layer chain"
            ))
        }),
        other => Err(InstitutionError::ComputationFailed(format!(
            "`proof_payload` has unexpected value shape: {other:?}"
        ))),
    }
}

/// Extract the `payload_bytes` string from a `LeanProofPayload`
/// resource.
fn payload_bytes(payload: &Resource) -> Result<String, InstitutionError> {
    payload
        .get(&Iri::parse(iris::PROP_PAYLOAD_BYTES).expect("static IRI"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            InstitutionError::ComputationFailed(
                "LeanProofPayload missing required `payload_bytes` property".to_string(),
            )
        })
}

/// Build the embedded Verdict resource the kernel's commit pipeline
/// expects: `is_a: [Verdict]`, `ctor_name: "Holds"|"Fails"`, and an
/// optional `diagnostic` string. Matches the shape
/// `kernel::institution::in_process_registry::EchoInstitution::query`
/// constructs.
fn verdict_resource(ctor_name: &str, diagnostic: Option<&str>) -> Resource {
    let mut r = Resource::new_embedded();
    r.set(
        Iri::parse(wk::IS_A).expect("well-known IRI"),
        Value::Array(vec![Value::ResourceRef(
            Iri::parse(wk::VERDICT).expect("well-known IRI"),
        )]),
    );
    r.set(
        Iri::parse(wk::CTOR_NAME).expect("well-known IRI"),
        Value::String(ctor_name.to_string()),
    );
    if let Some(d) = diagnostic {
        r.set(
            Iri::parse(iris::PROP_DIAGNOSTIC).expect("static IRI"),
            Value::String(d.to_string()),
        );
    }
    r
}
