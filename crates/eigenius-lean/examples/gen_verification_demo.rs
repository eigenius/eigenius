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
//
//! Generator for the lean-verification demo notebook fixture.
//!
//! Reads the existing capstone proof bytes
//! (`crates/eigenius-lean/test_resources/capstone_proof.json`) plus
//! the capstone Lake project sources (`lean/research/capstone-proof/`)
//! and emits a self-contained Eigon-JSON document with the five
//! resources the audit chain walks through:
//!
//! 1. `urn:eigenius:demo:lean:Patient` — class declaration.
//! 2. `urn:eigenius:demo:lean:patient_1` — instance (the Eigon claim
//!    the proof discharges).
//! 3. `urn:eigenius:demo:lean:mirror` — `LeanPackageMirror` carrying
//!    the embedded Lake project archive.
//! 4. `urn:eigenius:demo:lean:proof_payload` — `LeanProofPayload`
//!    holding the verbatim `lean4export` bytes.
//! 5. `urn:eigenius:demo:lean:proof_term` — `LeanProofTerm` wiring
//!    everything together, including the chain-mirrored proposition.
//!
//! The output file is loaded by `lean-verification-setup.sh` before
//! the user opens the notebook in the browser. Regenerate any time
//! the Lean toolchain bumps or the capstone proof changes (see the
//! upgrade checklist in `docs/notes/lean-toolchain-upgrade.md`).
//!
//! Run from the workspace root:
//!
//! ```sh
//! cargo run --example gen_verification_demo
//! ```
//!
//! Output: `notebooks/examples/lean-verification-demo.eigon.json`.

use std::path::PathBuf;
use std::sync::Arc;

use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;
use eigenius_lean::chain_mirror::bytes_to_lean_expr;
use eigenius_lean::institution::iris as lean_iris;
use eigenius_lean_runtime::mirror_gen::LeanMirrorGenerator;
use eigenius_runtime_substrate::mirror_generator::MirrorGenerator;

const TARGET_THEOREM: &str = "patient_weight_nonneg";

// Demo namespace — distinct from `urn:eigenius:test:capstone:*` (the
// capstone integration-test scope) so a chain that has both committed
// concurrently doesn't collide.
const PATIENT_CLASS_IRI: &str = "urn:eigenius:demo:lean:Patient";
const PATIENT_INSTANCE_IRI: &str = "urn:eigenius:demo:lean:patient_1";
const MIRROR_IRI: &str = "urn:eigenius:demo:lean:mirror";
const PAYLOAD_IRI: &str = "urn:eigenius:demo:lean:proof_payload";
const TERM_IRI: &str = "urn:eigenius:demo:lean:proof_term";

const OUTPUT_REL: &str = "notebooks/examples/lean-verification-demo.eigon.json";

fn main() {
    let workspace = workspace_root();
    let proof_bytes_path =
        workspace.join("crates/eigenius-lean/test_resources/capstone_proof.json");
    let capstone_dir = workspace.join("lean/research/capstone-proof");
    let output_path = workspace.join(OUTPUT_REL);

    eprintln!(
        "Reading capstone proof bytes from {}",
        proof_bytes_path.display()
    );
    let proof_bytes = std::fs::read(&proof_bytes_path).unwrap_or_else(|e| {
        panic!(
            "read capstone proof bytes `{}`: {e}",
            proof_bytes_path.display()
        )
    });

    eprintln!(
        "Reading capstone Lake archive from {}",
        capstone_dir.display()
    );
    let archive = capstone_archive(&capstone_dir);

    let lib_hash = library_content_hash(&archive);
    eprintln!("Library content hash: {lib_hash}");
    let lib_json = library_content_json(&archive);

    // Bootstrap head is the universal ancestor of every chain layer.
    // Using it as `source_layer` means the mirror's anchor is
    // *somewhere* the claim's layer descends from, satisfying D28
    // §5.5's mirror-correspondence ancestral check regardless of how
    // much user state sits between bootstrap and the demo layer.
    // Deterministic across runs because bootstrap is deterministic.
    let bootstrap_head_id = bootstrap_head_layer_id();
    eprintln!("Bootstrap head layer ID: {bootstrap_head_id}");

    eprintln!("Decoding proposition for theorem `{TARGET_THEOREM}`");
    let proposition = bytes_to_lean_expr(&proof_bytes, TARGET_THEOREM)
        .expect("chain-mirror translator must decode the capstone proposition");

    let resources = vec![
        patient_class_resource(),
        patient_instance_resource(),
        mirror_resource(lib_hash, lib_json, bootstrap_head_id),
        proof_payload_resource(&proof_bytes),
        proof_term_resource(proposition),
    ];

    let doc = eigon_json::serialize_document(&resources);
    let pretty = serde_json::to_string_pretty(&doc).expect("pretty-print Eigon-JSON");

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).expect("create parent directory");
    }
    std::fs::write(&output_path, &pretty).expect("write fixture file");

    eprintln!(
        "Wrote {} ({} bytes, {} resources)",
        output_path.display(),
        pretty.len(),
        resources.len()
    );
}

// ─── Resource builders ─────────────────────────────────────────────────

fn patient_class_resource() -> Resource {
    // The chain-side class declaration. `short_name` is load-bearing —
    // the structural correspondence check maps from the proposition's
    // `EigeniusFFI.Patient` Const reference back to this IRI via this
    // property. `description` is required by the chain's `Class`
    // validator (every committed class must carry one).
    let mut r = Resource::new(iri(PATIENT_CLASS_IRI));
    r.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(wk::CLASS))]),
    );
    r.set(iri(wk::SHORT_NAME), Value::String("Patient".to_string()));
    r.set(
        iri(wk::DESCRIPTION),
        Value::String(
            "Demo Patient class for the lean-verification notebook. The Lean proof \
             `patient_weight_nonneg` discharges a claim about an instance of this class."
                .to_string(),
        ),
    );
    r
}

fn patient_instance_resource() -> Resource {
    // The Eigon claim the proof discharges. `is_a[0]` is what the
    // correspondence check reads.
    let mut r = Resource::new(iri(PATIENT_INSTANCE_IRI));
    r.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(PATIENT_CLASS_IRI))]),
    );
    r
}

fn mirror_resource(
    lib_hash: String,
    lib_json: serde_json::Value,
    source_layer_id: String,
) -> Resource {
    // Pull the canonical generator metadata off `LeanMirrorGenerator`
    // — `generator_identifier` / `generator_version` /
    // `generator_content_hash` are properties of the *generator*, not
    // its input, so they don't depend on the capstone Lake archive we
    // happen to be packaging. Sourcing them this way keeps the demo
    // fixture in lockstep with what a real chain-driven generation
    // would emit: a toolchain bump or generator-code change updates
    // the values automatically the next time this binary runs.
    let generator = LeanMirrorGenerator::new();

    let mut r = Resource::new(iri(MIRROR_IRI));
    r.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(
            "urn:eigenius:runtime:RuntimePackageMirror",
        ))]),
    );
    // `short_name` is required by `RuntimePackageMirror` and the
    // value matches what the real LeanMirrorGenerator emits: the
    // generated Lake package is always named `EigeniusFFI` (D30 §2.1).
    r.set(
        iri(wk::SHORT_NAME),
        Value::String("EigeniusFFI".to_string()),
    );
    r.set(
        iri(wk::DESCRIPTION),
        Value::String(
            "Lean mirror of demo classes for the lean-verification notebook demo. \
             Backed by the capstone Lake project (`lean/research/capstone-proof/`); \
             not produced by the LeanMirrorGenerator's chain-driven path, but carries \
             the same resource shape so the chain validator + Lean institution \
             treat it identically."
                .to_string(),
        ),
    );
    // `language` discriminates language-side runtime packages — the
    // institution + worker dispatch read this when resolving handlers.
    r.set(
        iri("urn:eigenius:runtime:language"),
        Value::String("lean".to_string()),
    );
    // Integrity-chain trio (D30 §10). Anchors the mirror against the
    // generator that *would* have produced it; chain-side consumers
    // can verify the generator code itself hasn't drifted by
    // recomputing `generator_content_hash` against a fresh build of
    // the LeanMirrorGenerator and comparing.
    r.set(
        iri("urn:eigenius:runtime:generator_identifier"),
        Value::String(generator.generator_identifier().to_string()),
    );
    r.set(
        iri("urn:eigenius:runtime:generator_version"),
        Value::String(generator.generator_version().to_string()),
    );
    r.set(
        iri("urn:eigenius:runtime:generator_content_hash"),
        Value::String(generator.generator_content_hash().to_string()),
    );
    r.set(
        iri(lean_iris::PROP_MIRROR_SOURCE_LAYER),
        Value::String(source_layer_id),
    );
    r.set(
        iri(lean_iris::PROP_MIRROR_LIB_CONTENT_HASH),
        Value::String(lib_hash),
    );
    r.set(
        iri(lean_iris::PROP_MIRROR_LIB_CONTENT),
        Value::Json(lib_json),
    );
    r.set(
        iri(lean_iris::PROP_MIRRORED_CLASSES),
        Value::Array(vec![Value::ResourceRef(iri(PATIENT_CLASS_IRI))]),
    );
    r
}

fn proof_payload_resource(bytes: &[u8]) -> Resource {
    let bytes_str = std::str::from_utf8(bytes).expect("capstone proof bytes must be valid UTF-8");
    let mut r = Resource::new(iri(PAYLOAD_IRI));
    r.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(
            "urn:eigenius:lean:LeanProofPayload",
        ))]),
    );
    r.set(
        iri(lean_iris::PROP_PAYLOAD_BYTES),
        Value::String(bytes_str.to_string()),
    );
    r
}

fn proof_term_resource(proposition: Value) -> Resource {
    let mut r = Resource::new(iri(TERM_IRI));
    r.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(
            "urn:eigenius:lean:LeanProofTerm",
        ))]),
    );
    r.set(
        iri(lean_iris::PROP_PROOF_PAYLOAD),
        Value::ResourceRef(iri(PAYLOAD_IRI)),
    );
    r.set(
        iri(lean_iris::PROP_TARGET_NAME),
        Value::String(TARGET_THEOREM.to_string()),
    );
    r.set(
        iri(lean_iris::PROP_MIRROR_IRI),
        Value::String(MIRROR_IRI.to_string()),
    );
    r.set(
        iri(lean_iris::PROP_CLAIM_IRI),
        Value::String(PATIENT_INSTANCE_IRI.to_string()),
    );
    r.set(iri(lean_iris::PROP_PROPOSITION), proposition);
    r
}

// ─── Mirror archive helpers (D30 §10.2) ─────────────────────────────────
//
// Mirror of the same helpers in `crates/eigenius-lean/tests/capstone_test.rs`.
// Inlined rather than pulled from a shared crate so this example
// binary's dep tree stays minimal (no `eigenius-lean-runtime` import,
// no `base64` crate). If a third consumer needs them, promote to a
// pub helper in `eigenius-lean-runtime::mirror_gen` (which already
// has the matching `library_content_hash` function).

struct ArchiveFile {
    path: &'static str,
    content: Vec<u8>,
}

fn capstone_archive(root: &std::path::Path) -> Vec<ArchiveFile> {
    let read = |rel: &'static str| ArchiveFile {
        path: rel,
        content: std::fs::read(root.join(rel))
            .unwrap_or_else(|e| panic!("read capstone source `{rel}`: {e}")),
    };
    // Order is irrelevant — `library_content_hash` sorts internally —
    // but matching the capstone test's order keeps diffs minimal.
    vec![
        read("lakefile.lean"),
        read("lean-toolchain"),
        read("EigeniusFFI.lean"),
        read("Capstone.lean"),
    ]
}

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

// ─── Bootstrap layer ID ─────────────────────────────────────────────────

fn bootstrap_head_layer_id() -> String {
    // Wrap the hex `LayerId` in the `urn:eigenius:layer:<hex>` IRI
    // scheme so the value satisfies the ontology's
    // `format = urn:eigenius:core:formats:iri` constraint on
    // `RuntimePackageMirror.source_layer`. The Lean institution's
    // ancestry check strips this prefix before comparing against
    // `Layer::id().to_string()`. The capstone test's bare-hex
    // pattern bypasses commit-time validation by going through
    // `LayerBuilder::build()` directly — Eigon-JSON loads via
    // `eigenius load` don't, so the demo fixture has to use the
    // valid-IRI form.
    let ctx = eigenius_kernel::bootstrap::bootstrap()
        .expect("bootstrap must succeed (kernel ontology must compile)");
    format!("urn:eigenius:layer:{}", Arc::clone(ctx.head()).id())
}

// ─── Workspace path resolution ──────────────────────────────────────────

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR points at `<workspace>/crates/eigenius-lean/`
    // for this example binary. Walk up two segments to reach the
    // workspace root.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .expect("CARGO_MANIFEST_DIR must have ancestor segments")
}

// ─── Small free helper ─────────────────────────────────────────────────

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("static IRI must parse")
}
