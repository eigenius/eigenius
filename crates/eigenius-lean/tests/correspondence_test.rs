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

//! Phase 20a.7 — D28 §5.5 three-part correspondence check.
//!
//! Each scenario builds a layer carrying:
//! - A `LeanProofPayload` (the toy `PUnit` export).
//! - A `LeanPackageMirror` (varying shape per scenario).
//! - A claim resource the proof discharges.
//! - A `LeanProofTerm` referencing the payload + mirror + claim.
//!
//! Then drives the kernel's AutoOnLoad dispatch through
//! `LeanInstitution`'s `qc_proof_check` handler and asserts the
//! verdict + diagnostic kind the spec calls for:
//!
//! - **Happy path** — mirror covers the claim's class, anchor is
//!   reachable, content hash is intact: `Verdict::Holds`.
//! - **FFI version mismatch** — mirror's `mirrored_classes` doesn't
//!   include the claim's class: `Verdict::Fails` carrying
//!   `FFIVersionMismatch`.
//! - **Anchor tamper** — mirror's declared `library_content_hash`
//!   doesn't match the actual content: `Verdict::Fails` carrying
//!   `AnchorContentHashMismatch`.
//!
//! Test fixtures use the same `toy_proof_holds.json` payload as the
//! end-to-end smoke test so the nanoda step always succeeds — the
//! variation under test is the correspondence layer, not the proof
//! validity.

use std::sync::Arc;

use eigenius_kernel::context::{ExecutionContext, ExecutionMode};
use eigenius_kernel::institution::runtime::Institution;
use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::ontology::well_known as wk;

use eigenius_lean::institution::iris as lean_iris;
use eigenius_lean::LeanInstitution;

/// Distinguish Holds vs Fails for tests reading the verdict
/// resource directly. Mirrors the kernel-side
/// `VerdictReading::{Holds, Fails}` shape but keeps the diagnostic
/// next to the result so per-scenario assertions stay tight.
#[derive(Debug)]
enum CheckOutcome {
    Holds,
    Fails(String),
}

const TOY_HOLDS: &[u8] = include_bytes!("../test_resources/toy_proof_holds.json");

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("static IRI must parse")
}

// ─── Mirror archive construction (matches the substrate-side
// `eigenius-lean-runtime::mirror_gen::library_content_to_json` shape) ─

/// One file in the mirror's embedded archive. `path` is relative
/// to the package root; `content` is the verbatim source bytes.
struct ArchiveFile {
    path: &'static str,
    content: &'static [u8],
}

/// Compute the library_content_hash over `files` using the same
/// length-prefixed framing as `eigenius-lean-runtime`'s assembler.
/// Path-sorted, SHA-256.
fn library_content_hash(files: &[ArchiveFile]) -> String {
    use sha2::{Digest, Sha256};
    let mut sorted: Vec<&ArchiveFile> = files.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(b.path));
    let mut hasher = Sha256::new();
    for f in sorted {
        hasher.update((f.path.len() as u64).to_be_bytes());
        hasher.update(f.path.as_bytes());
        hasher.update((f.content.len() as u64).to_be_bytes());
        hasher.update(f.content);
    }
    format!("sha256:{:x}", hasher.finalize())
}

/// Render an archive as the `library_content` JSON shape the
/// substrate's mirror-materialiser decodes — `{"kind": "embedded",
/// "files": [{"path", "content_b64"}]}` with path-sorted entries.
fn library_content_json(files: &[ArchiveFile]) -> serde_json::Value {
    let mut sorted: Vec<&ArchiveFile> = files.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(b.path));
    let arr: Vec<serde_json::Value> = sorted
        .iter()
        .map(|f| {
            serde_json::json!({
                "path": f.path,
                "content_b64": base64_encode(f.content),
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

// ─── Layer-building helpers ────────────────────────────────────────

/// Knobs for `build_test_layer` — each scenario varies a few fields.
struct ScenarioInputs<'a> {
    /// IRIs the mirror covers (`mirrored_classes`). Empty array means
    /// the mirror covers no claim — used by the FFI-mismatch scenario.
    mirrored_classes: &'a [&'a str],
    /// Class IRI to stamp onto the claim's `is_a`. Determines whether
    /// the claim falls inside the mirror's coverage.
    claim_class: &'a str,
    /// If `Some`, overrides the computed `library_content_hash` —
    /// used by the anchor-tamper scenario to plant a bad digest.
    tampered_hash: Option<String>,
    /// If `Some`, attach a chain-mirrored `lean:LeanExpr` proposition
    /// to the `LeanProofTerm`. Triggers the structural correspondence
    /// check (D28 §5.5 ¶2 final sentence). The default `None` keeps
    /// the proposition absent so v1's covering-only check applies.
    proposition: Option<serde_json::Value>,
}

impl Default for ScenarioInputs<'_> {
    fn default() -> Self {
        Self {
            mirrored_classes: &[],
            claim_class: "urn:eigenius:test:correspondence:Claim",
            tampered_hash: None,
            proposition: None,
        }
    }
}

/// Build a D40 §3.1 `lean:LeanName` tagged dict for a dotted name
/// like `EigeniusFFI.Patient`. Convenience for hand-rolling
/// `Const`-flavoured propositions.
fn lean_name(segments: &[&str]) -> serde_json::Value {
    let mut acc = serde_json::json!({"ctor": "Anon"});
    for seg in segments {
        acc = serde_json::json!({"ctor": "Str", "args": [acc, seg]});
    }
    acc
}

/// Construct a chain-mirrored `lean:LeanExpr.Const` referencing the
/// mirror type `EigeniusFFI.<class_short_name>` with an empty
/// universe-instantiation list. Smallest proposition value the
/// structural check accepts as a positive mirror-type reference.
fn lean_const_ref(class_short_name: &str) -> serde_json::Value {
    let name = lean_name(&["EigeniusFFI", class_short_name]);
    let levels = serde_json::json!({"ctor": "Nil"});
    serde_json::json!({"ctor": "Const", "args": [name, levels]})
}

/// Wrap an inner expression in a `Pi` binder. Used to verify the
/// walker descends through binder slots.
fn lean_pi(binder_type: serde_json::Value, body: serde_json::Value) -> serde_json::Value {
    let name = serde_json::json!({"ctor": "Anon"});
    serde_json::json!({
        "ctor": "Pi",
        "args": [name, "default", binder_type, body],
    })
}

/// Build a layer carrying a `LeanProofPayload`, a
/// `LeanPackageMirror`, a claim, and a `LeanProofTerm` wiring them
/// together. Returns (storage, head_layer, term_iri) ready for
/// `dispatch_auto_on_load_for_resource`.
fn build_test_layer(s: &ScenarioInputs) -> (LayerStorage, Arc<Layer>, Iri) {
    // The bootstrap chain head carries the ontology declaring
    // LeanProofTerm / LeanPackageMirror; we anchor here so the
    // dispatch index sees their class definitions.
    let ctx = eigenius_kernel::bootstrap::bootstrap().expect("bootstrap");
    let parent = Arc::clone(ctx.head());
    let storage = LayerStorage::in_memory();
    let parent_layer_id = parent.id().to_string();

    let mut builder = LayerBuilder::new("test_correspondence_layer", Some(parent));

    let payload_iri_str = "urn:eigenius:test:lean:corr_payload";
    let mirror_iri_str = "urn:eigenius:test:lean:corr_mirror";
    let claim_iri_str = "urn:eigenius:test:lean:corr_claim";
    let term_iri_str = "urn:eigenius:test:lean:corr_term";

    // LeanProofPayload — the verbatim toy export bytes.
    let payload_bytes = std::str::from_utf8(TOY_HOLDS).expect("toy fixture must be UTF-8");
    let mut payload = Resource::new(iri(payload_iri_str));
    payload.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(
            "urn:eigenius:lean:LeanProofPayload",
        ))]),
    );
    payload.set(
        iri(lean_iris::PROP_PAYLOAD_BYTES),
        Value::String(payload_bytes.to_string()),
    );
    builder.add_resource(payload).expect("add payload");

    // Claim — carries `is_a: [claim_class]`. The correspondence
    // check reads `is_a[0]` and compares against the mirror's
    // `mirrored_classes`.
    let mut claim = Resource::new(iri(claim_iri_str));
    claim.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(s.claim_class))]),
    );
    builder.add_resource(claim).expect("add claim");

    // Each class IRI the mirror covers (or, for structural-check
    // scenarios, every IRI the proposition references) needs an
    // accompanying `core:short_name` so the structural check's
    // `short_name → class_iri` lookup can map back. Auto-derive the
    // short name from the trailing IRI segment.
    for class_iri_str in s.mirrored_classes {
        let short = class_iri_str
            .rsplit(':')
            .next()
            .expect("non-empty IRI suffix");
        let mut class_def = Resource::new(iri(class_iri_str));
        class_def.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::CLASS))]),
        );
        class_def.set(iri(wk::SHORT_NAME), Value::String(short.to_string()));
        builder
            .add_resource(class_def)
            .expect("add class declaration");
    }

    // LeanPackageMirror — the substrate-committed mirror archive.
    // Single-file archive ("hello\n") is enough to exercise the
    // hash + JSON-shape paths; the institution doesn't actually
    // parse the Lean source, it just rehashes the bytes.
    let archive = [ArchiveFile {
        path: "EigeniusFFI/Mirror.lean",
        content: b"-- empty for correspondence test\n",
    }];
    let real_hash = library_content_hash(&archive);
    let declared_hash = s.tampered_hash.clone().unwrap_or(real_hash);
    let archive_json = library_content_json(&archive);
    let mut mirror = Resource::new(iri(mirror_iri_str));
    mirror.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(
            "urn:eigenius:runtime:RuntimePackageMirror",
        ))]),
    );
    mirror.set(
        iri(lean_iris::PROP_MIRROR_SOURCE_LAYER),
        // Anchor the mirror at the parent layer so the ancestry
        // walk from `head` (this layer) reaches it.
        Value::String(parent_layer_id),
    );
    mirror.set(
        iri(lean_iris::PROP_MIRROR_LIB_CONTENT_HASH),
        Value::String(declared_hash),
    );
    mirror.set(
        iri(lean_iris::PROP_MIRROR_LIB_CONTENT),
        Value::Json(archive_json),
    );
    mirror.set(
        iri(lean_iris::PROP_MIRRORED_CLASSES),
        Value::Array(
            s.mirrored_classes
                .iter()
                .map(|c| Value::ResourceRef(iri(c)))
                .collect(),
        ),
    );
    builder.add_resource(mirror).expect("add mirror");

    // LeanProofTerm — references payload + mirror + claim.
    let mut term = Resource::new(iri(term_iri_str));
    term.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(
            "urn:eigenius:lean:LeanProofTerm",
        ))]),
    );
    term.set(
        iri(lean_iris::PROP_PROOF_PAYLOAD),
        Value::ResourceRef(iri(payload_iri_str)),
    );
    term.set(
        iri(lean_iris::PROP_TARGET_NAME),
        Value::String("PUnit".to_string()),
    );
    term.set(
        iri(lean_iris::PROP_MIRROR_IRI),
        Value::String(mirror_iri_str.to_string()),
    );
    term.set(
        iri(lean_iris::PROP_CLAIM_IRI),
        Value::String(claim_iri_str.to_string()),
    );
    if let Some(prop_json) = &s.proposition {
        term.set(
            iri(lean_iris::PROP_PROPOSITION),
            Value::Json(prop_json.clone()),
        );
    }
    builder.add_resource(term).expect("add term");

    let layer = Arc::new(builder.build(storage.clone()));
    (storage, layer, iri(term_iri_str))
}

/// Invoke `LeanInstitution::query(proof_check, term)` directly and
/// read the verdict + diagnostic off the returned resource. The
/// kernel's `dispatch_auto_on_load_for_resource` path strips the
/// diagnostic when flattening to `VerdictReading`; the correspondence
/// tests need the diagnostic string to assert the failure *kind*
/// (D28 §9.1), so we bypass dispatch and call the handler directly.
/// The existing `end_to_end_smoke.rs` covers the dispatch path.
fn run_check(layer: &Arc<Layer>, storage: LayerStorage, term_iri: &Iri) -> CheckOutcome {
    let ctx = ExecutionContext::new(Arc::clone(layer), "test", ExecutionMode::ReadOnly, storage);
    let term = layer
        .resolve(term_iri)
        .expect("LeanProofTerm must resolve on the committed layer");
    let institution = LeanInstitution::new();
    let proc_iri = Iri::parse(lean_iris::PROC_PROOF_CHECK).expect("static IRI");
    let outcome = institution
        .query(&proc_iri, &term, &ctx)
        .expect("institution query");
    extract_verdict(&outcome.output)
}

/// Read the verdict ctor_name + optional diagnostic out of the
/// embedded Verdict resource the institution returned.
fn extract_verdict(output: &Resource) -> CheckOutcome {
    let ctor = output
        .get(&iri(wk::CTOR_NAME))
        .and_then(Value::as_str)
        .expect("Verdict resource must carry ctor_name");
    match ctor {
        "Holds" => CheckOutcome::Holds,
        "Fails" => {
            let diag = output
                .get(&iri(lean_iris::PROP_DIAGNOSTIC))
                .and_then(Value::as_str)
                .unwrap_or("<no diagnostic>")
                .to_string();
            CheckOutcome::Fails(diag)
        }
        other => panic!("unexpected ctor_name `{other}`"),
    }
}

// ─── Tests ─────────────────────────────────────────────────────────

#[test]
fn happy_path_correspondence_passes_when_mirror_covers_claim_class() {
    // Mirror covers `urn:eigenius:test:correspondence:Claim`, claim
    // is of that class, hash + ancestry intact → Holds.
    let claim_class = "urn:eigenius:test:correspondence:Claim";
    let (storage, layer, term_iri) = build_test_layer(&ScenarioInputs {
        mirrored_classes: &[claim_class],
        claim_class,
        ..Default::default()
    });
    let outcome = run_check(&layer, storage, &term_iri);
    assert!(
        matches!(outcome, CheckOutcome::Holds),
        "happy-path correspondence must yield Holds; got {outcome:?}"
    );
}

#[test]
fn ffi_version_mismatch_when_claim_class_is_not_in_mirrored_classes() {
    // Mirror covers one class; the claim is of a different class.
    // D28 §5.6's compositionality break.
    let (storage, layer, term_iri) = build_test_layer(&ScenarioInputs {
        mirrored_classes: &["urn:eigenius:test:correspondence:SomeOtherClass"],
        claim_class: "urn:eigenius:test:correspondence:Claim",
        ..Default::default()
    });
    let outcome = run_check(&layer, storage, &term_iri);
    let diag = match outcome {
        CheckOutcome::Fails(d) => d,
        CheckOutcome::Holds => panic!("uncovered class must yield Fails; got Holds"),
    };
    assert!(
        diag.starts_with("FFIVersionMismatch:"),
        "expected FFIVersionMismatch prefix; got: {diag}"
    );
    assert!(
        diag.contains("urn:eigenius:test:correspondence:Claim"),
        "diagnostic should name the missing class IRI; got: {diag}"
    );
}

#[test]
fn anchor_content_hash_mismatch_when_declared_hash_is_tampered() {
    // Mirror covers the claim's class — but its declared content
    // hash doesn't match the recomputed value. Anchor consistency
    // (check 3) catches this; the dispatch must Fail with
    // AnchorContentHashMismatch *before* falling through to the
    // class-coverage check.
    let claim_class = "urn:eigenius:test:correspondence:Claim";
    let (storage, layer, term_iri) = build_test_layer(&ScenarioInputs {
        mirrored_classes: &[claim_class],
        claim_class,
        tampered_hash: Some(
            "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        ),
        ..Default::default()
    });
    let outcome = run_check(&layer, storage, &term_iri);
    let diag = match outcome {
        CheckOutcome::Fails(d) => d,
        CheckOutcome::Holds => panic!("tampered content hash must yield Fails"),
    };
    assert!(
        diag.starts_with("AnchorContentHashMismatch:"),
        "expected AnchorContentHashMismatch prefix; got: {diag}"
    );
}

#[test]
fn unanchored_proof_skips_correspondence_and_falls_back_to_nanoda_verdict() {
    // Backward-compatibility: a LeanProofTerm without `mirror_iri`
    // is the 20a.4 shape ("verify under nanoda alone, no chain
    // claim"). The new check path must skip and yield the nanoda
    // verdict unchanged.
    let bytes_str = std::str::from_utf8(TOY_HOLDS).expect("toy fixture must be UTF-8");
    let ctx = eigenius_kernel::bootstrap::bootstrap().expect("bootstrap");
    let parent = Arc::clone(ctx.head());
    let storage = LayerStorage::in_memory();
    let mut builder = LayerBuilder::new("test_unanchored", Some(parent));

    let payload_iri_str = "urn:eigenius:test:lean:una_payload";
    let term_iri_str = "urn:eigenius:test:lean:una_term";

    let mut payload = Resource::new(iri(payload_iri_str));
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

    let mut term = Resource::new(iri(term_iri_str));
    term.set(
        iri(wk::IS_A),
        Value::Array(vec![Value::ResourceRef(iri(
            "urn:eigenius:lean:LeanProofTerm",
        ))]),
    );
    term.set(
        iri(lean_iris::PROP_PROOF_PAYLOAD),
        Value::ResourceRef(iri(payload_iri_str)),
    );
    term.set(
        iri(lean_iris::PROP_TARGET_NAME),
        Value::String("PUnit".to_string()),
    );
    // mirror_iri deliberately omitted.
    builder.add_resource(term).expect("add term");

    let layer = Arc::new(builder.build(storage.clone()));
    let outcome = run_check(&layer, storage, &iri(term_iri_str));
    assert!(
        matches!(outcome, CheckOutcome::Holds),
        "unanchored proof with valid bytes must still verify; got {outcome:?}"
    );
}

// ─── Structural correspondence (D28 §5.5 ¶2 final sentence) ────────

#[test]
fn structural_correspondence_passes_when_proposition_references_claim_class_via_mirror() {
    // Proposition: `∀ _ : EigeniusFFI.Claim, EigeniusFFI.Claim`
    // (nonsensical as a real theorem but exercises the walker's
    // descent through `Pi` binder slots). The proposition
    // references `EigeniusFFI.Claim`; the mirror covers `Claim`;
    // the claim's class is `Claim` → structural check passes.
    let claim_class = "urn:eigenius:test:correspondence:Claim";
    let proposition = lean_pi(lean_const_ref("Claim"), lean_const_ref("Claim"));
    let (storage, layer, term_iri) = build_test_layer(&ScenarioInputs {
        mirrored_classes: &[claim_class],
        claim_class,
        proposition: Some(proposition),
        ..Default::default()
    });
    let outcome = run_check(&layer, storage, &term_iri);
    assert!(
        matches!(outcome, CheckOutcome::Holds),
        "structural correspondence with a matching mirror reference must Hold; got {outcome:?}"
    );
}

#[test]
fn proposition_mismatch_when_proposition_references_only_a_different_mirror_class() {
    // The mirror covers both `Claim` and `Other`. The claim's
    // class is `Claim`. But the proposition reasons about `Other`
    // only — exactly the "wrong proposition" shape D28 §9.1's
    // PropositionMismatch diagnostic exists for.
    let claim_class = "urn:eigenius:test:correspondence:Claim";
    let other_class = "urn:eigenius:test:correspondence:Other";
    let proposition = lean_pi(lean_const_ref("Other"), lean_const_ref("Other"));
    let (storage, layer, term_iri) = build_test_layer(&ScenarioInputs {
        mirrored_classes: &[claim_class, other_class],
        claim_class,
        proposition: Some(proposition),
        ..Default::default()
    });
    let outcome = run_check(&layer, storage, &term_iri);
    let diag = match outcome {
        CheckOutcome::Fails(d) => d,
        CheckOutcome::Holds => panic!("wrong-target proposition must yield Fails"),
    };
    assert!(
        diag.starts_with("PropositionMismatch:"),
        "expected PropositionMismatch prefix; got: {diag}"
    );
    assert!(
        diag.contains("urn:eigenius:test:correspondence:Claim"),
        "diagnostic should name the claim class; got: {diag}"
    );
    assert!(
        diag.contains("urn:eigenius:test:correspondence:Other"),
        "diagnostic should name what the proposition actually references; got: {diag}"
    );
}

#[test]
fn proposition_mismatch_when_proposition_references_no_mirror_classes_at_all() {
    // The proposition reasons only about Lean core types (no
    // `EigeniusFFI.*` Const references). The mirror covers the
    // claim's class, but the proposition isn't *about* it.
    // Diagnostic mentions the empty mirror-reference set.
    let claim_class = "urn:eigenius:test:correspondence:Claim";
    // `∀ _ : Nat, Nat` — no EigeniusFFI namespace touches.
    let nat_const = {
        let name = lean_name(&["Nat"]);
        let levels = serde_json::json!({"ctor": "Nil"});
        serde_json::json!({"ctor": "Const", "args": [name, levels]})
    };
    let proposition = lean_pi(nat_const.clone(), nat_const);
    let (storage, layer, term_iri) = build_test_layer(&ScenarioInputs {
        mirrored_classes: &[claim_class],
        claim_class,
        proposition: Some(proposition),
        ..Default::default()
    });
    let outcome = run_check(&layer, storage, &term_iri);
    let diag = match outcome {
        CheckOutcome::Fails(d) => d,
        CheckOutcome::Holds => panic!("non-mirror-typed proposition must Fail"),
    };
    assert!(
        diag.starts_with("PropositionMismatch:"),
        "expected PropositionMismatch prefix; got: {diag}"
    );
    assert!(
        diag.contains("no mirror types"),
        "diagnostic should call out the empty mirror-reference set; got: {diag}"
    );
}

#[test]
fn structural_walker_finds_mirror_const_buried_in_nested_app_and_pi() {
    // The walker has to descend through every D40 ctor that nests
    // sub-expressions. Build a proposition like
    //   `App (Pi _ : Nat, EigeniusFFI.Claim) (Var 0)`
    // — the mirror reference sits inside a Pi's body, the Pi is
    // wrapped in an App. A walker that only inspected the root
    // would miss it.
    let claim_class = "urn:eigenius:test:correspondence:Claim";
    let nat_const = {
        let name = lean_name(&["Nat"]);
        let levels = serde_json::json!({"ctor": "Nil"});
        serde_json::json!({"ctor": "Const", "args": [name, levels]})
    };
    let inner_pi = lean_pi(nat_const, lean_const_ref("Claim"));
    let proposition = serde_json::json!({
        "ctor": "App",
        "args": [inner_pi, {"ctor": "Var", "args": [0]}],
    });
    let (storage, layer, term_iri) = build_test_layer(&ScenarioInputs {
        mirrored_classes: &[claim_class],
        claim_class,
        proposition: Some(proposition),
        ..Default::default()
    });
    let outcome = run_check(&layer, storage, &term_iri);
    assert!(
        matches!(outcome, CheckOutcome::Holds),
        "walker must find Const buried inside App(Pi(..., Const), Var); got {outcome:?}"
    );
}

#[test]
fn structural_walker_ignores_classes_outside_the_eigeniusffi_namespace() {
    // A `Const` whose name is `OtherProject.Claim` looks like a
    // mirror reference at first glance but isn't — the
    // `EigeniusFFI.` namespace gate must reject it. Otherwise we'd
    // pass propositions that reference *unrelated* types whose Lean
    // short_name happens to match a mirrored class.
    let claim_class = "urn:eigenius:test:correspondence:Claim";
    let name = lean_name(&["OtherProject", "Claim"]);
    let levels = serde_json::json!({"ctor": "Nil"});
    let other_const = serde_json::json!({"ctor": "Const", "args": [name, levels]});
    let proposition = lean_pi(other_const.clone(), other_const);
    let (storage, layer, term_iri) = build_test_layer(&ScenarioInputs {
        mirrored_classes: &[claim_class],
        claim_class,
        proposition: Some(proposition),
        ..Default::default()
    });
    let outcome = run_check(&layer, storage, &term_iri);
    let diag = match outcome {
        CheckOutcome::Fails(d) => d,
        CheckOutcome::Holds => panic!(
            "OtherProject.Claim must not be confused with EigeniusFFI.Claim — \
             walker namespace gate is the soundness floor here"
        ),
    };
    assert!(
        diag.starts_with("PropositionMismatch:"),
        "expected PropositionMismatch; got: {diag}"
    );
}
