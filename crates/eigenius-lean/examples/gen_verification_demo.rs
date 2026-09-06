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
//! and emits a self-contained Eigon-JSON document with the resources the audit chain walks
//! through:
//!
//! 1. `urn:eigenius:demo:lean:Patient` — class declaration.
//! 2. `urn:eigenius:demo:lean:Healthy` — the predicate the propositions apply.
//! 3. `urn:eigenius:demo:lean:patient_1` / `patient_2` — NAMED INDIVIDUALS. Chain axioms of type
//!    `Patient`, so a proposition can mention one; entities, so neither carries a proposition.
//! 4. `urn:eigenius:demo:lean:claim_patient_1_healthy` / `claim_patient_2_healthy` —
//!    `justification:Claim`s, each carrying `Healthy(<its subject>)`.
//! 5. `urn:eigenius:demo:lean:mirror` — `LeanPackageMirror` carrying the embedded Lake archive.
//! 6. `urn:eigenius:demo:lean:proof_payload` — `LeanProofPayload` with the verbatim
//!    `lean4export` bytes.
//! 7. `urn:eigenius:demo:lean:proof_term` — the `LeanProofTerm` that HOLDS: `healthy_patient_1`
//!    against the claim about `patient_1`.
//!
//! Plus a second document, `lean-verification-near-miss.eigon.json`, carrying one resource:
//! `urn:eigenius:demo:lean:proof_term_near_miss` — the same proof and the same target
//! declaration, bound to the claim about `patient_2`. It ships separately because an AutoOnLoad
//! gate returning `Fails` refuses the whole commit, so a near-miss inside the main document would
//! take the demo down with it. Separate, the refusal is the demonstration.
//!
//! ## What D87 §6 changed here, and why
//!
//! `patient_1` used to be a `Patient` INSTANCE carrying a `reflection:canonical_proposition`, and
//! that proposition was `∀ (p : Patient), Healthy(p) → Healthy(p)` — closed, universally
//! quantified, and never mentioning `patient_1`. So the witness the chain admitted paired a
//! resource IRI with a proposition that said nothing about that resource: *any* IRI would have
//! served equally. Two counts, and the second is the substantive one — a `Patient` instance is an
//! entity rather than an assertion, and the proposition was not about it.
//!
//! The demo therefore showed the plumbing running, not the check discriminating: the proof was a
//! tautology (`fun _ h => h`) about no one in particular. It now shows both. Each individual has
//! a claim ABOUT it, one proof exists, and the near-miss binds that proof to the other claim — a
//! proposition that is equally true and equally proved, so the refusal comes from `def_eq` and
//! nothing else. That is `Holds` meaning *"this proof proves THIS claim"* rather than *"a theorem
//! with this name type-checks"* (eigenius#159).
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
use eigenius_lean::institution::iris as lean_iris;
use eigenius_lean_runtime::mirror_gen::LeanMirrorGenerator;
use eigenius_runtime_substrate::mirror_generator::MirrorGenerator;

/// The theorem the demo's proof discharges: `Healthy patient_1`.
///
/// Not `patient_weight_nonneg`: D74's statement check manufactures the goal from the claim's
/// `reflection:canonical_proposition`, and `∀ p, 0.0 ≤ p.weight.val` is outside the §4 fragment
/// (a structure-field access, and `Float`).
///
/// Not `healthy_refl` either, since D87 §6. That one is `∀ p, Healthy p → Healthy p` — true of
/// every Patient and about none of them, so the claim it was checked against could name any
/// resource at all. This one names its subject.
const TARGET_THEOREM: &str = "healthy_patient_1";

/// The chain axiom the proposition applies — `demo:Healthy : demo:Patient -> Prop`.
const HEALTHY_IRI: &str = "urn:eigenius:demo:lean:Healthy";

// Demo namespace — distinct from `urn:eigenius:test:capstone:*` (the
// capstone integration-test scope) so a chain that has both committed
// concurrently doesn't collide.
const PATIENT_CLASS_IRI: &str = "urn:eigenius:demo:lean:Patient";
/// The subject the demo's claim is about, and the subject its proof names.
const PATIENT_1_IRI: &str = "urn:eigenius:demo:lean:patient_1";
/// The near-miss's subject. Equally real, equally healthy, and not what the proof proves.
const PATIENT_2_IRI: &str = "urn:eigenius:demo:lean:patient_2";
const CLAIM_1_IRI: &str = "urn:eigenius:demo:lean:claim_patient_1_healthy";
const CLAIM_2_IRI: &str = "urn:eigenius:demo:lean:claim_patient_2_healthy";
const MIRROR_IRI: &str = "urn:eigenius:demo:lean:mirror";
const PAYLOAD_IRI: &str = "urn:eigenius:demo:lean:proof_payload";
const TERM_IRI: &str = "urn:eigenius:demo:lean:proof_term";
const NEAR_MISS_TERM_IRI: &str = "urn:eigenius:demo:lean:proof_term_near_miss";

const OUTPUT_REL: &str = "notebooks/examples/lean-verification-demo.eigon.json";

/// The near-miss ships as its OWN document, and it has to.
///
/// An AutoOnLoad gate returning `Fails` refuses the whole commit — measured: the orchestrator
/// answers `InstitutionValidation … returned Fails` and lands no layer. So a near-miss inside the
/// main document would take the demo down with it. Loaded separately, the refusal is the point:
/// the notebook commits the verified claim, then commits this and watches the chain say no.
const NEAR_MISS_OUTPUT_REL: &str = "notebooks/examples/lean-verification-near-miss.eigon.json";

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

    let resources = vec![
        patient_class_resource(),
        healthy_axiom_resource(chain()),
        named_individual_resource(PATIENT_1_IRI, "patient_1", chain()),
        named_individual_resource(PATIENT_2_IRI, "patient_2", chain()),
        claim_resource(CLAIM_1_IRI, PATIENT_1_IRI, chain()),
        claim_resource(CLAIM_2_IRI, PATIENT_2_IRI, chain()),
        mirror_resource(lib_hash, lib_json, bootstrap_head_id),
        proof_payload_resource(&proof_bytes),
        proof_term_resource(TERM_IRI, CLAIM_1_IRI),
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

    // The near-miss, alone in its own document. `claim_patient_2_healthy` is already committed by
    // the file above, so this adds only the proof term that binds the wrong claim to the proof.
    let near_miss = vec![proof_term_resource(NEAR_MISS_TERM_IRI, CLAIM_2_IRI)];
    let near_miss_path = workspace.join(NEAR_MISS_OUTPUT_REL);
    let near_miss_doc = eigon_json::serialize_document(&near_miss);
    let near_miss_pretty =
        serde_json::to_string_pretty(&near_miss_doc).expect("pretty-print Eigon-JSON");
    std::fs::write(&near_miss_path, &near_miss_pretty).expect("write near-miss fixture");
    eprintln!(
        "Wrote {} ({} bytes, {} resource)",
        near_miss_path.display(),
        near_miss_pretty.len(),
        near_miss.len()
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
        Value::Array(vec![Value::iri(&iri(wk::CLASS))]),
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

/// A NAMED INDIVIDUAL: `axiom demo:lean:<name> : demo:lean:Patient`.
///
/// An `eigentt:Axiom` and not a `Patient` instance, and the difference is what D87 §6 is about.
/// The D47 decoder yields `EigonAxiom` only for that class, and an `EigonClass` cannot head an
/// application — so only an axiom-shaped individual can appear as an argument in `Healthy(x)`.
/// A resource that is merely `is_a: [Patient]` is an entity the term language cannot mention,
/// which is why the old fixture's proposition quantified over all Patients instead of naming one.
///
/// It carries no proposition. Patients do not assert things; the `justification:Claim` beside it
/// does, and this is what that claim is about.
fn named_individual_resource(
    individual_iri: &str,
    short_name: &str,
    chain: &Arc<eigenius_kernel::layer::Layer>,
) -> Resource {
    use eigenius_kernel::nbe::term::Exp;
    use eigenius_kernel::program::eigentt_type_mirror::{encode_type, CodecNames};

    let mut r = Resource::new(iri(individual_iri));
    r.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(
            "urn:eigenius:eigentt:Axiom".to_string(),
        )]),
    );
    r.set(iri(wk::SHORT_NAME), Value::String(short_name.to_string()));
    r.set(
        iri(wk::DESCRIPTION),
        Value::String(format!(
            "A named individual of demo:lean:Patient. Mirrored in Lean as `def \
             EigeniusFFI.eigenius.demo.lean.{short_name}`, so a proposition naming it \
             externalizes to a Const the export declares."
        )),
    );
    // The individual's TYPE is the class itself — this is an inhabitant, not a predicate.
    let names = CodecNames::from_layer(chain);
    r.set(
        iri("urn:eigenius:eigentt:axiom_statement"),
        encode_type(&Exp::EigonClass(iri(PATIENT_CLASS_IRI)), &names).expect("Patient encodes"),
    );
    r
}

/// A `justification:Claim` carrying `Healthy(<subject>)` — a proposition ABOUT its subject.
///
/// `justification:Claim` and not `justification:Conclusion`: a Conclusion `requires
/// justification:judgement`, the kernel's own `holds(kernel, c, Certificate(j, P))`, and this
/// claim's warrant is a Lean proof rather than a certificate over chain grounds. D87 §6 reached
/// for `Conclusion` because `subject_iri` was the only way to say what a ∀-quantified proposition
/// was about; with the proposition naming its subject directly, `Claim` is the class the ontology
/// already describes — *"a chain-resident resource carrying a proposition … cited as a ground"*.
fn claim_resource(
    claim_iri: &str,
    subject_iri: &str,
    chain: &Arc<eigenius_kernel::layer::Layer>,
) -> Resource {
    use eigenius_kernel::nbe::term::Exp;
    use eigenius_kernel::program::eigentt_type_mirror::{encode_type, CodecNames};

    let mut r = Resource::new(iri(claim_iri));
    r.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(
            "urn:eigenius:justification:Claim".to_string(),
        )]),
    );
    r.set(
        iri("urn:eigenius:prov:was_attributed_to"),
        Value::String("urn:eigenius:prov:agent:eigenius_core_team".to_string()),
    );
    r.set(
        iri(wk::DESCRIPTION),
        Value::String(format!(
            "The claim that {subject_iri} is Healthy. Its proposition mentions its subject, which \
             is what makes the (resource, proposition) pairing mean something: swap the subject \
             and the proposition changes with it."
        )),
    );

    // `Healthy(subject)` — `App(EigonAxiom(Healthy), EigonAxiom(subject))`. Every node is inside
    // D74 §4.1, and it externalizes to `EigeniusFFI.…Healthy EigeniusFFI.…<subject>`.
    let prop = Exp::App(
        Box::new(Exp::EigonAxiom(iri(HEALTHY_IRI))),
        Box::new(Exp::EigonAxiom(iri(subject_iri))),
    );
    let names = CodecNames::from_layer(chain);
    r.set(
        iri(wk::CANONICAL_PROPOSITION),
        encode_type(&prop, &names).expect("the demo proposition is inside the D47 codec"),
    );
    r
}

/// `demo:Healthy : demo:Patient -> Prop`, the predicate the proposition applies.
///
/// A chain `axiom`, mirrored in Lean as a `def` — see `EigeniusFFI.lean`. The externalizer maps
/// `EigonAxiom(iri)` to a `Const` under D30's mangling, so the two meet at
/// `EigeniusFFI.eigenius.demo.lean.Healthy`.
fn healthy_axiom_resource(chain: &Arc<eigenius_kernel::layer::Layer>) -> Resource {
    use eigenius_kernel::nbe::term::Exp;
    use eigenius_kernel::program::eigentt_type_mirror::{encode_type, CodecNames};

    let mut r = Resource::new(iri(HEALTHY_IRI));
    // An `eigentt:Axiom`, not a `core:Class` — the D47 decoder yields `EigonAxiom` only for
    // that class, and an `EigonClass` cannot head an application ("App spine applied to
    // non-parametric head").
    r.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(
            "urn:eigenius:eigentt:Axiom".to_string(),
        )]),
    );
    r.set(iri(wk::SHORT_NAME), Value::String("Healthy".to_string()));
    r.set(
        iri(wk::DESCRIPTION),
        Value::String(
            "A predicate over demo:Patient. Mirrored in Lean as a `def` returning `Prop`; the \
             demo's proposition only ever applies it, never unfolds it."
                .to_string(),
        ),
    );
    // `eigentt:Axiom` requires its statement — the axiom's TYPE: `Patient -> Prop`.
    let statement = Exp::Arrow(
        Box::new(Exp::EigonClass(iri(PATIENT_CLASS_IRI))),
        Box::new(Exp::Sort(eigenius_kernel::nbe::level::Level::Zero)),
    );
    let names = CodecNames::from_layer(chain);
    r.set(
        iri("urn:eigenius:eigentt:axiom_statement"),
        encode_type(&statement, &names).expect("Patient -> Prop encodes"),
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
        Value::Array(vec![Value::String(
            iri("urn:eigenius:runtime:RuntimePackageMirror")
                .as_str()
                .to_string(),
        )]),
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
        Value::Array(vec![Value::String(
            iri(PATIENT_CLASS_IRI).as_str().to_string(),
        )]),
    );
    r
}

fn proof_payload_resource(bytes: &[u8]) -> Resource {
    let bytes_str = std::str::from_utf8(bytes).expect("capstone proof bytes must be valid UTF-8");
    let mut r = Resource::new(iri(PAYLOAD_IRI));
    r.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(
            iri("urn:eigenius:lean:LeanProofPayload")
                .as_str()
                .to_string(),
        )]),
    );
    r.set(
        iri(lean_iris::PROP_PAYLOAD_BYTES),
        Value::String(bytes_str.to_string()),
    );
    r
}

/// A `LeanProofTerm` binding the one payload and the one target declaration to `claim_iri`.
///
/// Two are emitted, differing only in that last slot. `TARGET_THEOREM` proves `Healthy patient_1`,
/// so the term naming the `patient_1` claim Holds and the one naming the `patient_2` claim Fails —
/// on `def_eq` against the target's type, with both propositions true and both proved in the same
/// export. Nothing about name availability is what separates them.
fn proof_term_resource(term_iri: &str, claim_iri: &str) -> Resource {
    let mut r = Resource::new(iri(term_iri));
    r.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::String(
            iri("urn:eigenius:lean:LeanProofTerm").as_str().to_string(),
        )]),
    );
    r.set(
        iri(lean_iris::PROP_PROOF_PAYLOAD),
        Value::iri(&iri(PAYLOAD_IRI)),
    );
    r.set(
        iri(lean_iris::PROP_TARGET_NAME),
        Value::String(TARGET_THEOREM.to_string()),
    );
    r.set(
        iri(lean_iris::PROP_CLAIM_IRI),
        Value::String(claim_iri.to_string()),
    );
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

/// The bootstrap chain, for the D47 codec's constructor-argument names.
///
/// `CodecNames` reads each inductive's ctor argument names off the chain rather than carrying a
/// copy (D85 §6.1), so encoding the demo's proposition needs a layer.
fn chain() -> &'static Arc<eigenius_kernel::layer::Layer> {
    use std::sync::OnceLock;
    static CHAIN: OnceLock<Arc<eigenius_kernel::layer::Layer>> = OnceLock::new();
    CHAIN.get_or_init(|| {
        let ctx = eigenius_kernel::bootstrap::bootstrap().expect("bootstrap");
        Arc::clone(ctx.head())
    })
}

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
