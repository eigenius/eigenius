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

//! Phase 20a.6 golden-file regression test.
//!
//! Drives `LeanMirrorGenerator::generate` against a small synthetic
//! ontology fixture and diffs the four emitted files (lakefile.lean,
//! lean-toolchain, EigeniusFFI/Basic.lean, EigeniusFFI/Mirror.lean)
//! against checked-in goldens. Pins D30 §10.1's determinism property
//! (same `(generator binary, source layer, seed classes)` →
//! byte-identical output) against accidental drift in the emitter
//! chain.
//!
//! ## Ontology shape
//!
//! Deliberately small: three classes covering each emitter dimension
//! at least once.
//!
//! - `Tag` — root class with one required `String` field.
//!   Exercises the structure + codec emit baseline.
//! - `Sample` — root with one required `Float` carrying a
//!   `min_value` + `max_value` (refinement subtype) and one
//!   optional `Tag` reference (recommended classref).
//! - `Document` — extends `Tag`, adds a required `Sample` field
//!   and a required value-array of strings. Exercises
//!   inheritance, classref, and primitive lists.
//!
//! No `EigeniusUnion` field in the fixture — multi-class
//! polymorphic dispatch has its own dedicated unit + Lake-build
//! tests; covering it here would double the golden's surface area
//! without surfacing distinct emitter shapes.
//!
//! ## Update workflow
//!
//! When the emitter intentionally changes:
//!
//! 1. Run the test, see the diff in the failure output:
//!    `cargo test -p eigenius-lean-runtime --test mirror_golden_test`.
//! 2. Inspect the new bytes for correctness — read the failure
//!    diff or run the test with `--nocapture` to peek.
//! 3. Accept the new output by re-running with the update env var:
//!    `EIGENIUS_UPDATE_GOLDEN=1 cargo test -p eigenius-lean-runtime --test mirror_golden_test`.
//! 4. Commit the updated `tests/golden/` files alongside the
//!    emitter change. The diff is the audit trail for the spec
//!    deviation (if any).

#![cfg(test)]

use std::collections::HashMap;
use std::path::PathBuf;

use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_lean_runtime::mirror_gen::{mirror_to_resource, LeanMirrorGenerator};
use eigenius_runtime_substrate::chain::ChainAccessor;
use eigenius_runtime_substrate::mirror_generator::{
    LibraryContent, MirrorGenerationRequest, MirrorGenerator,
};

const GOLDEN_DIR: &str = "tests/golden";

// ─── Synthetic chain ───────────────────────────────────────────────

struct InMemoryChain {
    resources: HashMap<Iri, Resource>,
}

impl InMemoryChain {
    fn new() -> Self {
        Self {
            resources: HashMap::new(),
        }
    }
    fn insert(&mut self, r: Resource) {
        self.resources
            .insert(r.id().expect("id required").clone(), r);
    }
}

impl ChainAccessor for InMemoryChain {
    fn resolve(&self, _claim_layer: &Iri, target: &Iri) -> Option<Resource> {
        self.resources.get(target).cloned()
    }
    fn is_ancestor_or_equal(&self, _: &Iri, _: &Iri) -> bool {
        true
    }
    fn class_unchanged_between(&self, _: &Iri, _: &Iri, _: &Iri) -> bool {
        true
    }
}

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("test IRI")
}

fn class_resource(
    iri_str: &str,
    short: &str,
    parents: &[&str],
    requires: &[&str],
    recommends: &[&str],
) -> Resource {
    let mut r = Resource::new(iri(iri_str));
    r.set(
        iri("urn:eigenius:core:short_name"),
        Value::String(short.to_string()),
    );
    if !parents.is_empty() {
        r.set(
            iri("urn:eigenius:core:subclass_of"),
            Value::Array(parents.iter().map(|p| Value::ResourceRef(iri(p))).collect()),
        );
    }
    if !requires.is_empty() {
        r.set(
            iri("urn:eigenius:core:requires"),
            Value::Array(
                requires
                    .iter()
                    .map(|p| Value::ResourceRef(iri(p)))
                    .collect(),
            ),
        );
    }
    if !recommends.is_empty() {
        r.set(
            iri("urn:eigenius:core:recommends"),
            Value::Array(
                recommends
                    .iter()
                    .map(|p| Value::ResourceRef(iri(p)))
                    .collect(),
            ),
        );
    }
    r
}

struct PropBuilder {
    r: Resource,
}

impl PropBuilder {
    fn new(iri_str: &str, short: &str, data_type: &str) -> Self {
        let mut r = Resource::new(iri(iri_str));
        r.set(
            iri("urn:eigenius:core:short_name"),
            Value::String(short.to_string()),
        );
        r.set(
            iri("urn:eigenius:core:data_type"),
            Value::ResourceRef(iri(data_type)),
        );
        Self { r }
    }
    fn class_types(mut self, classes: &[&str]) -> Self {
        self.r.set(
            iri("urn:eigenius:core:class_types"),
            Value::Array(classes.iter().map(|c| Value::ResourceRef(iri(c))).collect()),
        );
        self
    }
    fn element_type(mut self, et: &str) -> Self {
        self.r.set(
            iri("urn:eigenius:core:element_type"),
            Value::ResourceRef(iri(et)),
        );
        self
    }
    fn min_value(mut self, v: f64) -> Self {
        self.r
            .set(iri("urn:eigenius:core:min_value"), Value::Float(v));
        self
    }
    fn max_value(mut self, v: f64) -> Self {
        self.r
            .set(iri("urn:eigenius:core:max_value"), Value::Float(v));
        self
    }
    fn build(self) -> Resource {
        self.r
    }
}

fn fixture_chain() -> InMemoryChain {
    let mut c = InMemoryChain::new();

    // Tag — root, single required String.
    c.insert(class_resource(
        "urn:fixture:Tag",
        "Tag",
        &[],
        &["urn:fixture:tag_name"],
        &[],
    ));
    c.insert(
        PropBuilder::new(
            "urn:fixture:tag_name",
            "tag_name",
            "urn:eigenius:core:string",
        )
        .build(),
    );

    // Sample — root, refinement-typed Float + optional classref.
    c.insert(class_resource(
        "urn:fixture:Sample",
        "Sample",
        &[],
        &["urn:fixture:sample_weight"],
        &["urn:fixture:sample_tag"],
    ));
    c.insert(
        PropBuilder::new(
            "urn:fixture:sample_weight",
            "sample_weight",
            "urn:eigenius:core:float",
        )
        .min_value(0.0)
        .max_value(100.0)
        .build(),
    );
    c.insert(
        PropBuilder::new(
            "urn:fixture:sample_tag",
            "sample_tag",
            "urn:eigenius:core:resource",
        )
        .class_types(&["urn:fixture:Tag"])
        .build(),
    );

    // Document — extends Tag, classref to Sample + value_array of strings.
    c.insert(class_resource(
        "urn:fixture:Document",
        "Document",
        &["urn:fixture:Tag"],
        &["urn:fixture:doc_sample", "urn:fixture:doc_keywords"],
        &[],
    ));
    c.insert(
        PropBuilder::new(
            "urn:fixture:doc_sample",
            "doc_sample",
            "urn:eigenius:core:resource",
        )
        .class_types(&["urn:fixture:Sample"])
        .build(),
    );
    c.insert(
        PropBuilder::new(
            "urn:fixture:doc_keywords",
            "doc_keywords",
            "urn:eigenius:core:value_array",
        )
        .element_type("urn:eigenius:core:string")
        .build(),
    );

    c
}

/// Drive the full generator pipeline and return the emitted
/// `(path, content)` pairs sorted by path for deterministic
/// comparison against the goldens.
fn generate_fixture_files() -> Vec<(String, String)> {
    let chain = fixture_chain();
    let layer = iri("urn:fixture:layer");
    let seed = vec![iri("urn:fixture:Document"), iri("urn:fixture:Sample")];
    let req = MirrorGenerationRequest {
        source_layer: &layer,
        seed_classes: &seed,
        chain: &chain,
    };
    let g = LeanMirrorGenerator::new();
    let out = g.generate(&req).expect("generate");
    let LibraryContent::Embedded(files) = out.library else {
        panic!("fixture expected Embedded library");
    };
    let mut pairs: Vec<(String, String)> = files
        .into_iter()
        .map(|f| {
            let body = String::from_utf8(f.content).expect("utf-8 emit");
            (f.path, body)
        })
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    pairs
}

fn golden_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(GOLDEN_DIR)
        .join(rel)
}

/// Honour `EIGENIUS_UPDATE_GOLDEN=1` to rewrite goldens in place.
/// Without the env var, the test diffs and fails on mismatch — the
/// shape of CI-friendly regression assertions.
fn check_or_update_golden(rel: &str, actual: &str) {
    let path = golden_path(rel);
    if std::env::var("EIGENIUS_UPDATE_GOLDEN").as_deref() == Ok("1") {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir golden dir");
        }
        std::fs::write(&path, actual).expect("write golden");
        eprintln!("updated golden: {}", path.display());
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "golden file `{}` missing — regenerate with EIGENIUS_UPDATE_GOLDEN=1",
            path.display()
        )
    });
    if expected != actual {
        // Surface a compact diff hint without pulling in a diff
        // crate — file/line + the first divergent line is enough
        // for the developer to re-read the bytes.
        let first_diff_line = expected
            .lines()
            .zip(actual.lines())
            .enumerate()
            .find_map(|(i, (a, b))| if a != b { Some((i + 1, a, b)) } else { None });
        let summary = match first_diff_line {
            Some((line, exp, act)) => {
                format!("first diff at line {line}:\n  expected: {exp}\n  actual:   {act}")
            }
            None => "lengths differ (one file truncated relative to the other)".to_string(),
        };
        panic!(
            "golden mismatch for `{}`.\n{summary}\n\nTo accept the new output, run:\n  EIGENIUS_UPDATE_GOLDEN=1 cargo test -p eigenius-lean-runtime --test mirror_golden_test",
            path.display()
        );
    }
}

#[test]
fn fixture_emits_byte_identical_lakefile() {
    let files = generate_fixture_files();
    let (_, body) = files
        .iter()
        .find(|(p, _)| p == "lakefile.lean")
        .expect("lakefile present");
    check_or_update_golden("lakefile.lean", body);
}

#[test]
fn fixture_emits_byte_identical_toolchain() {
    let files = generate_fixture_files();
    let (_, body) = files
        .iter()
        .find(|(p, _)| p == "lean-toolchain")
        .expect("lean-toolchain present");
    check_or_update_golden("lean-toolchain", body);
}

#[test]
fn fixture_emits_byte_identical_basic_module() {
    let files = generate_fixture_files();
    let (_, body) = files
        .iter()
        .find(|(p, _)| p == "EigeniusFFI/Basic.lean")
        .expect("Basic present");
    check_or_update_golden("EigeniusFFI/Basic.lean", body);
}

#[test]
fn fixture_emits_byte_identical_mirror_module() {
    let files = generate_fixture_files();
    let (_, body) = files
        .iter()
        .find(|(p, _)| p == "EigeniusFFI/Mirror.lean")
        .expect("Mirror present");
    check_or_update_golden("EigeniusFFI/Mirror.lean", body);
}

#[test]
#[ignore = "requires lake + a built EigeniusLeanCommon olean cache"]
fn fixture_golden_lake_builds() {
    // Drive the goldens through `lake build` — pins that the
    // checked-in fixture isn't merely byte-stable but actually
    // valid Lean. Two failure modes this catches that the
    // byte-diff tests can't:
    //   - A future EigeniusLeanCommon refactor renames a helper
    //     the golden Mirror.lean still calls.
    //   - Lean toolchain bump silently breaks a syntax form the
    //     emitter relies on.
    if std::process::Command::new("lake")
        .arg("--version")
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true)
    {
        eprintln!("lake unavailable — skipping");
        return;
    }

    // Stage the golden files into a tempdir, rewrite the lakefile's
    // git-require to a path-require pointing at the local
    // EigeniusLeanCommon (same offline-build trick the
    // mirror_structure_lake_build tests use), run `lake build`.
    let pid = std::process::id();
    let work = std::env::temp_dir().join(format!("eigenius-lean-mirror-golden-{pid}"));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).expect("mkdir");

    let common = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("lean")
        .join("common")
        .join("EigeniusLeanCommon")
        .canonicalize()
        .expect("EigeniusLeanCommon present");
    let common_str = common.to_str().expect("utf-8 path");
    let path_require = format!("require EigeniusLeanCommon from \"{common_str}\"\n  ");

    let goldens = [
        "lakefile.lean",
        "lean-toolchain",
        "EigeniusFFI/Basic.lean",
        "EigeniusFFI/Mirror.lean",
    ];
    for rel in goldens {
        let src = std::fs::read_to_string(golden_path(rel)).expect("read golden");
        let body = if rel == "lakefile.lean" {
            src.replace(
                "require EigeniusLeanCommon from git \"https://github.com/eigenius/EigeniusLeanCommon.git\" @ \"v0.1.0\"\n",
                &path_require,
            )
        } else {
            src
        };
        let dest = work.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).expect("mkdir nested");
        }
        std::fs::write(&dest, body).expect("write golden");
    }

    let output = std::process::Command::new("lake")
        .current_dir(&work)
        .arg("build")
        .output()
        .expect("invoke lake");
    if !output.status.success() {
        panic!(
            "lake build of the golden fixture failed:\n--- stdout ---\n{}\n--- stderr ---\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
fn fixture_mirror_resource_id_pins_against_golden_digest() {
    // The mirror_to_resource IRI derives from library_content_hash.
    // If the emitter output drifts, the IRI changes — checking the
    // pinned IRI here doubles as a content-hash regression assertion.
    let chain = fixture_chain();
    let layer = iri("urn:fixture:layer");
    let seed = vec![iri("urn:fixture:Document"), iri("urn:fixture:Sample")];
    let req = MirrorGenerationRequest {
        source_layer: &layer,
        seed_classes: &seed,
        chain: &chain,
    };
    let g = LeanMirrorGenerator::new();
    let out = g.generate(&req).expect("generate");
    let resource = mirror_to_resource(&g, &out, &layer, None);
    let id = resource.id().expect("id present").as_str().to_string();
    check_or_update_golden("mirror_resource_id.txt", &id);
}
