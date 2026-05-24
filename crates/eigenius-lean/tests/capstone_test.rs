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

//! Phase 20a.8 capstone — closed audit chain end-to-end.
//!
//! ## The audit chain this test walks
//!
//! Starting from the verified `LeanProofTerm` and reading backwards
//! through every property that anchored the verification verdict:
//!
//! 1. **Verdict::Holds** on the `LeanProofTerm` — the test's
//!    assertion. The kernel's AutoOnLoad dispatch fired
//!    `qc_proof_check`, the `LeanInstitution` returned
//!    `Verdict::Holds`, and the resource lands tagged *verified*
//!    per D31 §6.3.
//!
//! 2. **Proof validity (D28 §5.5 ¶1)** — nanoda_lib parsed the
//!    `LeanProofPayload.payload_bytes` and accepted the term as a
//!    proof of `patient_weight_nonneg`. The bytes are real
//!    `lean4export` output (`lean/research/capstone-proof/`); no
//!    hand-crafting at the proof layer.
//!
//! 3. **Mirror correspondence (D28 §5.5 ¶2)** — the proposition
//!    on the `LeanProofTerm` (decoded by `chain_mirror`) carries a
//!    `Const "EigeniusFFI.Patient"` reference. The
//!    `LeanPackageMirror` resource the proof anchors to declares
//!    `mirrored_classes: [urn:eigenius:test:capstone:Patient]`,
//!    whose chain-side `short_name` is `Patient`. Map matches.
//!    Structural check (20a.7.x) passes.
//!
//! 4. **Anchor consistency (D28 §5.5 ¶3)** — the
//!    `LeanPackageMirror.library_content_hash` matches the SHA-256
//!    of the committed archive (Capstone.lean + EigeniusFFI.lean +
//!    lakefile + toolchain) under D30 §10.2's length-prefixed
//!    framing. Tamper-detection passes.
//!
//! 5. **Mirror anchor reachable** — `LeanPackageMirror.source_layer`
//!    resolves up `head.parent()`. Same lineage check the unit
//!    tests pin.
//!
//! 6. **The chain-side class definition** — `urn:eigenius:test:
//!    capstone:Patient` is committed in the test layer with
//!    `short_name: "Patient"`. The `claim_iri` on the
//!    `LeanProofTerm` resolves to a Patient instance whose `is_a`
//!    is `Patient`. Closes the loop: every IRI in the chain
//!    resolves to a resource the verification check could read.
//!
//! ## Why no Docker
//!
//! The capstone is a *verification-side* test: it asserts the
//! kernel's AutoOnLoad path admits a real Lean proof through every
//! check in D28 §5.5. The substrate's authoring-side dispatch
//! (`build_environment_image` + `lean_export` against a baked
//! mirror) is already covered by
//! `crates/eigenius-lean-runtime/tests/lean_image_build_e2e.rs`'s
//! Docker e2e and the `mirror_structure_lake_build` Lake-build
//! integration. Re-running the Docker pipeline here would duplicate
//! 5–15 min of cold image-build cost without adding new coverage.
//!
//! The proof bytes (`test_resources/capstone_proof.json`) are
//! produced once by `lake exe lean4export` against
//! `lean/research/capstone-proof/Capstone.lean` and committed as a
//! fixture. Regeneration:
//!
//! ```sh
//! cd lean/research/capstone-proof
//! lake build
//! lake exe lean4export Capstone -- patient_weight_nonneg \
//!   > ../../../crates/eigenius-lean/test_resources/capstone_proof.json
//! ```

use std::sync::Arc;

use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::institution::runtime::Institution;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;

use eigenius_lean::chain_mirror::bytes_to_lean_expr;
use eigenius_lean::institution::iris as lean_iris;
use eigenius_lean::LeanInstitution;

/// `lean4export` output for `lean/research/capstone-proof/`'s
/// `patient_weight_nonneg` theorem. ~9 kLoC of JSON — large
/// because the theorem's transitive closure (Float, Subtype, ≤,
/// the EigeniusFFI.Patient structure declaration) drags in a chunk
/// of Lean stdlib. The full file lands in the resource graph as
/// the `LeanProofPayload.payload_bytes` string.
const CAPSTONE_PROOF_BYTES: &[u8] = include_bytes!("../test_resources/capstone_proof.json");

const TARGET_THEOREM: &str = "patient_weight_nonneg";

// ─── Chain-side identifiers ───────────────────────────────────────

const PATIENT_CLASS_IRI: &str = "urn:eigenius:test:capstone:Patient";
const PATIENT_INSTANCE_IRI: &str = "urn:eigenius:test:capstone:patient_1";
const PAYLOAD_IRI: &str = "urn:eigenius:test:capstone:proof_payload";
const MIRROR_IRI: &str = "urn:eigenius:test:capstone:mirror";
const TERM_IRI: &str = "urn:eigenius:test:capstone:proof_term";

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("static IRI must parse")
}

// ─── Mirror archive helpers (mirror eigenius-lean-runtime) ─────────

struct ArchiveFile {
    path: &'static str,
    content: Vec<u8>,
}

/// Read the Capstone Lake project's source files off disk. The
/// returned tuples become the `LeanPackageMirror.library_content`
/// archive — the actual bytes the verification side rehashes.
fn capstone_archive() -> Vec<ArchiveFile> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("lean")
        .join("research")
        .join("capstone-proof");
    let read = |rel: &'static str| ArchiveFile {
        path: rel,
        content: std::fs::read(root.join(rel))
            .unwrap_or_else(|e| panic!("read capstone source `{rel}`: {e}")),
    };
    vec![
        read("lakefile.lean"),
        read("lean-toolchain"),
        read("EigeniusFFI.lean"),
        read("Capstone.lean"),
    ]
}

/// SHA-256 over the archive's path-sorted length-prefixed framing —
/// D30 §10.2; matches the substrate-side digest the institution
/// rehashes during the anchor-consistency check.
fn library_content_hash(files: &[ArchiveFile]) -> String {
    use sha2::{Digest, Sha256};
    let mut sorted: Vec<&ArchiveFile> = files.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(b.path));
    let mut hasher = Sha256::new();
    for f in sorted {
        hasher.update((f.path.len() as u64).to_be_bytes());
        hasher.update(f.path.as_bytes());
        hasher.update((f.content.len() as u64).to_be_bytes());
        hasher.update(&f.content);
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn library_content_json(files: &[ArchiveFile]) -> serde_json::Value {
    let mut sorted: Vec<&ArchiveFile> = files.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(b.path));
    let arr: Vec<serde_json::Value> = sorted
        .iter()
        .map(|f| {
            serde_json::json!({
                "path": f.path,
                "content_b64": base64_encode(&f.content),
            })
        })
        .collect();
    serde_json::json!({ "kind": "embedded", "files": arr })
}

fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let b = &bytes[i..i + 3];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        out.push(ALPHABET[(n & 0x3f) as usize] as char);
        i += 3;
    }
    let rem = bytes.len() - i;
    if rem == 1 {
        let n = (bytes[i] as u32) << 16;
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rem == 2 {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8);
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        out.push('=');
    }
    out
}

// ─── Chain construction ────────────────────────────────────────────

/// Build the capstone test layer. Returns (storage, layer, term_iri)
/// ready for the institution to query.
fn build_capstone_layer() -> (LayerStorage, Arc<Layer>) {
    // Anchor on the bootstrap chain head so the institution index
    // sees the ontology declaring LeanProofTerm + LeanPackageMirror.
    let ctx = eigenius_kernel::bootstrap::bootstrap().expect("bootstrap");
    let parent = Arc::clone(ctx.head());
    let storage = LayerStorage::in_memory();
    let parent_layer_id = parent.id().to_string();

    let mut builder = LayerBuilder::new("test_capstone_layer", Some(parent));

    // ── Patient class declaration ─────────────────────────────────
    //
    // The "chain class" half of the audit chain. `short_name` is
    // load-bearing — the structural correspondence check maps from
    // the proposition's `EigeniusFFI.Patient` Const reference back
    // to this IRI via this property.
    let mut patient_class = Resource::new(iri(PATIENT_CLASS_IRI));
    patient_class.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(wk::CLASS))]),
    );
    patient_class.set(iri(wk::SHORT_NAME), Value::String("Patient".to_string()));
    builder
        .add_resource(patient_class)
        .expect("add patient class");

    // ── Patient instance ──────────────────────────────────────────
    //
    // The Eigon claim this proof discharges. `is_a[0]` is what the
    // correspondence check reads.
    let mut patient = Resource::new(iri(PATIENT_INSTANCE_IRI));
    patient.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(PATIENT_CLASS_IRI))]),
    );
    builder.add_resource(patient).expect("add patient instance");

    // ── LeanPackageMirror — the audit anchor for the proof ────────
    let archive = capstone_archive();
    let lib_hash = library_content_hash(&archive);
    let lib_json = library_content_json(&archive);
    let mut mirror = Resource::new(iri(MIRROR_IRI));
    mirror.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(
            "urn:eigenius:runtime:RuntimePackageMirror",
        ))]),
    );
    mirror.set(
        iri(lean_iris::PROP_MIRROR_SOURCE_LAYER),
        Value::String(parent_layer_id),
    );
    mirror.set(
        iri(lean_iris::PROP_MIRROR_LIB_CONTENT_HASH),
        Value::String(lib_hash),
    );
    mirror.set(
        iri(lean_iris::PROP_MIRROR_LIB_CONTENT),
        Value::Json(lib_json),
    );
    mirror.set(
        iri(lean_iris::PROP_MIRRORED_CLASSES),
        Value::Array(vec![Value::ResourceRef(iri(PATIENT_CLASS_IRI))]),
    );
    builder.add_resource(mirror).expect("add mirror");

    // ── LeanProofPayload — verbatim lean4export bytes ─────────────
    let bytes_str = std::str::from_utf8(CAPSTONE_PROOF_BYTES)
        .expect("capstone proof bytes must be valid UTF-8");
    let mut payload = Resource::new(iri(PAYLOAD_IRI));
    payload.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(
            "urn:eigenius:lean:LeanProofPayload",
        ))]),
    );
    payload.set(
        iri(lean_iris::PROP_PAYLOAD_BYTES),
        Value::String(bytes_str.to_string()),
    );
    builder.add_resource(payload).expect("add payload");

    // ── LeanProofTerm — wires everything together ────────────────
    //
    // The proposition is decoded from the proof bytes by the
    // chain-mirror translator. This is what the authoring-side
    // commit pipeline would do (D28 §6.3); we replicate it here so
    // the proposition reflects the proof's actual type rather than
    // a hand-fabricated tree.
    let proposition = bytes_to_lean_expr(CAPSTONE_PROOF_BYTES, TARGET_THEOREM)
        .expect("chain-mirror translator must decode the capstone proposition");
    let mut term = Resource::new(iri(TERM_IRI));
    term.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(
            "urn:eigenius:lean:LeanProofTerm",
        ))]),
    );
    term.set(
        iri(lean_iris::PROP_PROOF_PAYLOAD),
        Value::ResourceRef(iri(PAYLOAD_IRI)),
    );
    term.set(
        iri(lean_iris::PROP_TARGET_NAME),
        Value::String(TARGET_THEOREM.to_string()),
    );
    term.set(
        iri(lean_iris::PROP_MIRROR_IRI),
        Value::String(MIRROR_IRI.to_string()),
    );
    term.set(
        iri(lean_iris::PROP_CLAIM_IRI),
        Value::String(PATIENT_INSTANCE_IRI.to_string()),
    );
    term.set(iri(lean_iris::PROP_PROPOSITION), proposition);
    builder.add_resource(term).expect("add term");

    let layer = Arc::new(builder.build(storage.clone()));
    (storage, layer)
}

// ─── The capstone assertion ────────────────────────────────────────

#[test]
#[ignore = "heavy: parses ~9 kLoC of lean4export output via nanoda"]
fn capstone_proof_lands_verified_through_full_audit_chain() {
    let (storage, layer) = build_capstone_layer();
    let ctx = ExecutionContext::new(
        Arc::clone(&layer),
        "capstone",
        ExecutionMode::ReadOnly,
        storage,
    );
    let term = layer
        .resolve(&iri(TERM_IRI))
        .expect("LeanProofTerm must resolve on the test layer");

    // Drive the institution's `qc_proof_check` handler. This
    // exercises (in order): nanoda's `check_proof`, the anchor
    // consistency check, the mirror anchor reachability check, the
    // mirror class-coverage check, and the structural
    // correspondence check between the proposition and the claim.
    let institution = LeanInstitution::new();
    let proc_iri = iri(lean_iris::PROC_PROOF_CHECK);
    let outcome = institution
        .query(&proc_iri, &term, &ctx)
        .expect("institution query");

    let ctor = outcome
        .output
        .get(&iri(wk::CTOR_NAME))
        .and_then(Value::as_str)
        .expect("Verdict resource must carry ctor_name");

    if ctor != "Holds" {
        let diag = outcome
            .output
            .get(&iri(lean_iris::PROP_DIAGNOSTIC))
            .and_then(Value::as_str)
            .unwrap_or("<no diagnostic>");
        panic!(
            "capstone proof must Hold; got ctor_name=`{ctor}` diagnostic=`{diag}`. \
             If the diagnostic begins with `ProofDoesNotCheck`, nanoda rejected the bytes — \
             regenerate `test_resources/capstone_proof.json` per the module docstring. \
             If it begins with `PropositionMismatch`, the chain-mirror translator's output \
             diverges from what the structural check expects; inspect the proposition's \
             EigeniusFFI references vs. the mirror's mirrored_classes."
        );
    }
}
