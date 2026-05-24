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

//! Lake-compile integration test for the Phase 20a.6 structure
//! emitter. The unit tests in `mirror_gen::structure_emitter` pin
//! the textual shape; this test pins that the shape is actually
//! *Lean*: emit a small mirror against a synthetic chain, write it
//! into a Lake project that depends on `EigeniusLeanCommon`, run
//! `lake build`, assert it succeeds.
//!
//! The substring-style unit tests can't catch:
//! - Lean syntax errors the emitter introduces (a stray colon, a
//!   missing newline before `deriving`).
//! - Mismatches between the emitter's `EigeniusUnion [...]` rendering
//!   and the hand-authored `EigeniusUnion` inductive's signature.
//! - Lake module-path conventions (e.g. namespacing, root settings).
//!
//! `#[ignore]`'d because Lake takes ~10 s cold and is unavailable in
//! CI sandboxes without elan. Run with
//! `cargo test -p eigenius-lean-runtime --test mirror_structure_lake_build -- --ignored`.

// The early structure-shape tests below use *hand-rolled* Lean
// bodies that mirror the emitter's expected output, so a drift in
// the emitter surfaces here too (the unit tests pin shape; this
// file pins compile). The codec round-trip tests further down
// drive the real emitters end-to-end through the public
// `mirror_gen::*` surface.

#![cfg(test)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_lean_runtime::mirror_gen::{
    build_decls, class_name_lookup, emit_class_block, topological_emit_order, LeanMirrorGenerator,
};
use eigenius_runtime_substrate::chain::ChainAccessor;
use eigenius_runtime_substrate::mirror_generator::{MirrorGenerationRequest, MirrorGenerator};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Resolve the host-side path to the `EigeniusLeanCommon` package
/// from the crate's `Cargo.toml` location. The substrate's
/// `build_environment_image` does the equivalent at production
/// time; the test mirrors that path resolution so a layout change
/// surfaces here too.
fn eigenius_lean_common_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("lean")
        .join("common")
        .join("EigeniusLeanCommon")
        .canonicalize()
        .expect("lean/common/EigeniusLeanCommon must exist relative to the crate's Cargo.toml")
}

fn is_lake_available() -> bool {
    Command::new("lake")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn fresh_workdir(label: &str) -> PathBuf {
    let pid = std::process::id();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("eigenius-lean-mirror-it-{pid}-{label}-{n}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create workdir");
    dir
}

/// Write a self-contained Lake project that imports
/// `EigeniusLeanCommon` from the host-side hand-authored package
/// and `import`s a generated Mirror module containing the emitter's
/// output. Lake resolves the require via a `path = "..."` dep.
fn write_lake_project(work: &Path, mirror_body: &str) {
    let common_path = eigenius_lean_common_dir();
    let common_str = common_path
        .to_str()
        .expect("EigeniusLeanCommon path must be UTF-8");

    let lakefile = format!(
        r#"-- Auto-generated for Phase 20a.6 structure-emitter integration test.
import Lake
open Lake DSL

package TestMirror where

require EigeniusLeanCommon from "{common_str}"

@[default_target]
lean_lib TestMirror where
  roots := #[`TestMirror.Basic, `TestMirror.Mirror]
"#
    );
    std::fs::write(work.join("lakefile.lean"), lakefile).expect("write lakefile.lean");

    // Pin the same toolchain elan-side as the worker + EigeniusLeanCommon.
    std::fs::write(work.join("lean-toolchain"), "leanprover/lean4:v4.29.1\n")
        .expect("write lean-toolchain");

    let basic = r#"-- Auto-generated.
import EigeniusLeanCommon

namespace TestMirror

-- The mirror module emits unqualified calls to the helpers defined
-- in EigeniusLeanCommon. Re-export them into this namespace so
-- `Mirror.lean` — which inhabits `namespace TestMirror` — can name
-- them without `EigeniusLeanCommon.` prefix.
--
-- The production emitter's Basic.lean (D30 §2.3) emits the same
-- export list against the EigeniusFFI namespace; this fixture
-- mirrors that contract.
export EigeniusLeanCommon (
  EigeniusUnion
  EigenValidationError
  validateMinValueFloat
  validateMaxValueFloat
  validateMinValueInt
  validateMaxValueInt
  validateMinLength
  validateMaxLength
  validatePattern
  validateFormat
  validateOptional
  withRefinement
  withOptionalRefinement
  decodeRequiredPrim
  decodeOptionalPrim
  decodeRequiredResource
  decodeOptionalResource
  decodeRequiredPrimList
  decodeRequiredResourceList
  isAHead
)

end TestMirror
"#;
    std::fs::create_dir_all(work.join("TestMirror")).expect("create TestMirror dir");
    std::fs::write(work.join("TestMirror").join("Basic.lean"), basic).expect("write Basic.lean");

    let mirror = format!(
        "-- Auto-generated mirror — emitter output under test.\nimport TestMirror.Basic\n\nnamespace TestMirror\n\n{mirror_body}\nend TestMirror\n"
    );
    std::fs::write(work.join("TestMirror").join("Mirror.lean"), mirror).expect("write Mirror.lean");
}

fn run_lake_build(work: &Path) -> Result<(), String> {
    let output = Command::new("lake")
        .current_dir(work)
        .arg("build")
        .output()
        .map_err(|e| format!("failed to invoke `lake build`: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "lake build failed (exit {:?}):\n--- stdout ---\n{}\n--- stderr ---\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

// ─── Test fixtures — direct invocations of the (pub-crate) emitter ──
//
// The structure_emitter module is `pub(crate)`, so we can't import
// it from this integration test. Instead, this file ships a *hand-
// rolled* version of what the emitter is expected to produce for
// each shape under test. The unit tests in
// `mirror_gen::structure_emitter::tests` pin that the emitter
// produces these bytes; this test pins that these bytes compile.
//
// Drift between the emitter and the hand-rolled bodies surfaces as
// a Lake build failure here (the emitter's output won't match Lean
// expectations) — the diagnostic is the build's stderr, not a
// string diff, but it still catches the structural error.
//
// When the codec emitter lands the integration test will hook into
// a pub-test helper instead so the round-trip is direct.

/// Hand-rolled equivalent of `emit_structure_block` for a root class
/// with one required field of each primitive type. Pinning this
/// exact text against the emitter output lets the unit tests catch
/// drift in the Rust→Lean lexical mapping.
fn handwritten_primitive_class() -> String {
    "structure Primitives where\n  _id : Option String := none\n  s : String\n  i : Int\n  f : Float\n  b : Bool\n  j : Lean.Json\n  deriving Repr\n".to_string()
}

fn handwritten_classref_pair() -> String {
    // Two classes — Doc has a field of type Person.
    "structure Person where\n  _id : Option String := none\n  name : String\n  deriving Repr\n\
     \n\
     structure Doc where\n  _id : Option String := none\n  author : Person\n  deriving Repr\n"
        .to_string()
}

fn handwritten_subclass_with_coercion() -> String {
    "structure Animal where\n  _id : Option String := none\n  deriving Repr\n\
     \n\
     structure Dog extends Animal where\n  breed : String\n  deriving Repr\n\
     \n\
     instance : CoeOut Dog Animal where\n  coe c := c.toAnimal\n"
        .to_string()
}

fn handwritten_list_and_union() -> String {
    "structure Apple where\n  _id : Option String := none\n  deriving Repr\n\
     \n\
     structure Zebra where\n  _id : Option String := none\n  deriving Repr\n\
     \n\
     structure Doc where\n  _id : Option String := none\n  tags : List String\n  contributor : EigeniusUnion [Apple, Zebra]\n  deriving Repr\n"
        .to_string()
}

#[test]
#[ignore = "requires lake + a built EigeniusLeanCommon olean cache"]
fn primitives_class_compiles_under_lake() {
    if !is_lake_available() {
        eprintln!("lake unavailable — skipping");
        return;
    }
    let work = fresh_workdir("primitives");
    write_lake_project(&work, &handwritten_primitive_class());
    run_lake_build(&work).expect("primitives mirror must lake-build");
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
#[ignore = "requires lake + a built EigeniusLeanCommon olean cache"]
fn classref_pair_compiles_under_lake() {
    if !is_lake_available() {
        eprintln!("lake unavailable — skipping");
        return;
    }
    let work = fresh_workdir("classref");
    write_lake_project(&work, &handwritten_classref_pair());
    run_lake_build(&work).expect("classref pair must lake-build");
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
#[ignore = "requires lake + a built EigeniusLeanCommon olean cache"]
fn subclass_with_coercion_compiles_under_lake() {
    if !is_lake_available() {
        eprintln!("lake unavailable — skipping");
        return;
    }
    let work = fresh_workdir("subclass");
    write_lake_project(&work, &handwritten_subclass_with_coercion());
    run_lake_build(&work).expect("subclass with coercion must lake-build");
    let _ = std::fs::remove_dir_all(&work);
}

// ─── End-to-end test driven through the real emitters ──────────────
//
// These tests build a synthetic chain, run `build_decls` +
// `topological_emit_order` + `emit_class_block` for each class,
// splice the concatenated output into a Lake project, and run
// `lake build`. Drift between the unit-test substring assertions and
// the actual Lean syntax surfaces here.

/// Minimal in-memory chain — same shape the unit tests use.
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
        self.resources.insert(
            r.id()
                .expect("synthetic chain entries must carry an IRI")
                .clone(),
            r,
        );
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

fn class_resource(iri_str: &str, short: &str, requires: &[&str]) -> Resource {
    let mut r = Resource::new(iri(iri_str));
    r.set(
        iri("urn:eigenius:core:short_name"),
        Value::String(short.to_string()),
    );
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
    r
}

fn primitive_property(iri_str: &str, short: &str, data_type: &str) -> Resource {
    let mut r = Resource::new(iri(iri_str));
    r.set(
        iri("urn:eigenius:core:short_name"),
        Value::String(short.to_string()),
    );
    r.set(
        iri("urn:eigenius:core:data_type"),
        Value::ResourceRef(iri(data_type)),
    );
    r
}

/// Build a property resource carrying numeric `min_value` /
/// `max_value` constraints.
fn ranged_property(
    iri_str: &str,
    short: &str,
    data_type: &str,
    min_value: Option<f64>,
    max_value: Option<f64>,
) -> Resource {
    let mut r = primitive_property(iri_str, short, data_type);
    if let Some(v) = min_value {
        r.set(iri("urn:eigenius:core:min_value"), Value::Float(v));
    }
    if let Some(v) = max_value {
        r.set(iri("urn:eigenius:core:max_value"), Value::Float(v));
    }
    r
}

/// Build a property resource carrying string-length + pattern +
/// format constraints — exercises every D30 §9.2 runtime check.
fn string_constrained_property(
    iri_str: &str,
    short: &str,
    min_length: Option<u64>,
    max_length: Option<u64>,
    pattern: Option<&str>,
    format_iri: Option<&str>,
) -> Resource {
    let mut r = primitive_property(iri_str, short, "urn:eigenius:core:string");
    if let Some(n) = min_length {
        r.set(
            iri("urn:eigenius:core:min_length"),
            Value::Integer(n as i64),
        );
    }
    if let Some(n) = max_length {
        r.set(
            iri("urn:eigenius:core:max_length"),
            Value::Integer(n as i64),
        );
    }
    if let Some(p) = pattern {
        r.set(
            iri("urn:eigenius:core:pattern"),
            Value::String(p.to_string()),
        );
    }
    if let Some(f) = format_iri {
        r.set(iri("urn:eigenius:core:format"), Value::ResourceRef(iri(f)));
    }
    r
}

fn classref_property(iri_str: &str, short: &str, class_iri: &str) -> Resource {
    let mut r = Resource::new(iri(iri_str));
    r.set(
        iri("urn:eigenius:core:short_name"),
        Value::String(short.to_string()),
    );
    r.set(
        iri("urn:eigenius:core:data_type"),
        Value::ResourceRef(iri("urn:eigenius:core:resource")),
    );
    r.set(
        iri("urn:eigenius:core:class_types"),
        Value::Array(vec![Value::ResourceRef(iri(class_iri))]),
    );
    r
}

/// Drive the real `mirror_gen` pipeline against a chain + seed,
/// return the concatenated per-class blocks ready to splice into a
/// Mirror module's namespace body.
fn emit_mirror_body(chain: &InMemoryChain, seed: &[Iri]) -> String {
    let layer = iri("urn:test:layer");
    let request = MirrorGenerationRequest {
        source_layer: &layer,
        seed_classes: seed,
        chain,
    };
    let decls = build_decls(&request).expect("build_decls");
    let lookup = class_name_lookup(&decls);
    let order = topological_emit_order(&decls).expect("topological order");
    let mut body = String::new();
    for (idx, iri) in order.iter().enumerate() {
        let decl = decls.get(iri).expect("decl");
        if idx > 0 {
            body.push('\n');
        }
        body.push_str(&emit_class_block(decl, &decls, &lookup));
    }
    body
}

#[test]
#[ignore = "requires lake + a built EigeniusLeanCommon olean cache"]
fn full_pipeline_primitive_class_compiles_under_lake() {
    if !is_lake_available() {
        eprintln!("lake unavailable — skipping");
        return;
    }
    // One class with one required String field — exercises the
    // smallest non-empty path through structure + decoder + encoder.
    let mut chain = InMemoryChain::new();
    chain.insert(class_resource(
        "urn:test:Person",
        "Person",
        &["urn:test:name"],
    ));
    chain.insert(primitive_property(
        "urn:test:name",
        "name",
        "urn:eigenius:core:string",
    ));
    let body = emit_mirror_body(&chain, &[iri("urn:test:Person")]);
    let work = fresh_workdir("e2e-primitive");
    write_lake_project(&work, &body);
    run_lake_build(&work).expect("primitive end-to-end mirror must lake-build");
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
#[ignore = "requires lake + a built EigeniusLeanCommon olean cache"]
fn full_pipeline_constraints_compile_under_lake() {
    if !is_lake_available() {
        eprintln!("lake unavailable — skipping");
        return;
    }
    // Single class with one constrained field of each kind covered
    // by D30 §9: Float with min/max range, Int with range, String
    // with length + pattern + format. The emitter's validator
    // chain has to compose every kind in the spec-mandated order
    // and the generated source has to lake-build against the real
    // EigeniusLeanCommon helpers.
    let mut chain = InMemoryChain::new();
    chain.insert(class_resource(
        "urn:test:Sample",
        "Sample",
        &["urn:test:weight", "urn:test:count", "urn:test:name"],
    ));
    chain.insert(ranged_property(
        "urn:test:weight",
        "weight",
        "urn:eigenius:core:float",
        Some(0.0),
        Some(100.0),
    ));
    chain.insert(ranged_property(
        "urn:test:count",
        "count",
        "urn:eigenius:core:integer",
        Some(1.0),
        Some(10.0),
    ));
    chain.insert(string_constrained_property(
        "urn:test:name",
        "name",
        Some(1),
        Some(64),
        Some("^[A-Za-z][A-Za-z0-9_-]*$"),
        Some("urn:eigenius:core:formats:iri"),
    ));
    let body = emit_mirror_body(&chain, &[iri("urn:test:Sample")]);
    let work = fresh_workdir("e2e-constraints");
    write_lake_project(&work, &body);
    run_lake_build(&work).expect("constraints end-to-end mirror must lake-build");
    let _ = std::fs::remove_dir_all(&work);
}

/// Drive the full `LeanMirrorGenerator::generate` pipeline against
/// a synthetic chain, materialise the four emitted files (lakefile,
/// toolchain, Basic, Mirror) verbatim into a workdir, and run
/// `lake build`. Differs from `emit_mirror_body`-based tests in
/// that:
///  - the lakefile is the assembler's output, not the test's hand
///    -written shell (so the path-vs-git require form, the
///    `EigeniusLeanCommon` tag pin, etc. all flow from
///    `module_assembler`);
///  - we rewrite the `require EigeniusLeanCommon from git "…" @ "…"`
///    line to `from "<local path>"` so the Lake build doesn't try to
///    fetch the package over the network. The path substitution is
///    test-fixture only — production builds resolve the git ref
///    against a registry mirror baked into the env image.
fn run_full_pipeline_under_lake(chain: &InMemoryChain, seed: &[Iri], label: &str) {
    let layer = iri("urn:test:layer");
    let request = MirrorGenerationRequest {
        source_layer: &layer,
        seed_classes: seed,
        chain,
    };
    let g = LeanMirrorGenerator::new();
    let output = g.generate(&request).expect("generate");

    let work = fresh_workdir(label);

    // The assembler emits the lakefile with a git-resolved require;
    // rewrite it to a path-resolved require for the offline test
    // run. The substitution preserves all other bytes so the rest
    // of the integrity chain (library_content_hash) still matches
    // what production would produce.
    let common_path = eigenius_lean_common_dir();
    let common_str = common_path
        .to_str()
        .expect("EigeniusLeanCommon path must be UTF-8");
    let path_require = format!("require EigeniusLeanCommon from \"{common_str}\"\n  ");

    let eigenius_runtime_substrate::mirror_generator::LibraryContent::Embedded(files) =
        &output.library
    else {
        panic!("expected Embedded library");
    };
    for file in files {
        let dest = work.join(&file.path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        let mut bytes = file.content.clone();
        if file.path == "lakefile.lean" {
            // Replace the git require with a local-path require so
            // the test doesn't depend on a Git remote.
            let src = String::from_utf8(bytes).expect("utf8 lakefile");
            let rewritten = src.replace(
                "require EigeniusLeanCommon from git \"https://github.com/eigenius/EigeniusLeanCommon.git\" @ \"v0.1.0\"\n",
                &path_require,
            );
            bytes = rewritten.into_bytes();
        }
        std::fs::write(&dest, bytes).expect("write file");
    }

    run_lake_build(&work).expect("full-pipeline mirror must lake-build");
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
#[ignore = "requires lake + a built EigeniusLeanCommon olean cache"]
fn full_pipeline_generate_lake_builds_primitive_class() {
    if !is_lake_available() {
        eprintln!("lake unavailable — skipping");
        return;
    }
    // Smallest end-to-end run through `LeanMirrorGenerator::generate`.
    // Verifies the four-file package assembled by `module_assembler`
    // is itself a buildable Lake project.
    let mut chain = InMemoryChain::new();
    chain.insert(class_resource(
        "urn:test:Person",
        "Person",
        &["urn:test:name"],
    ));
    chain.insert(primitive_property(
        "urn:test:name",
        "name",
        "urn:eigenius:core:string",
    ));
    run_full_pipeline_under_lake(&chain, &[iri("urn:test:Person")], "e2e-generate-primitive");
}

#[test]
#[ignore = "requires lake + a built EigeniusLeanCommon olean cache"]
fn full_pipeline_generate_lake_builds_with_refinement_constraints() {
    if !is_lake_available() {
        eprintln!("lake unavailable — skipping");
        return;
    }
    // End-to-end with refinement-typed fields — confirms the
    // assembler stitches the structure emitter's `{ x : T // pred }`
    // forms + the codec emitter's `withRefinement` calls into a
    // package Lake actually accepts.
    let mut chain = InMemoryChain::new();
    chain.insert(class_resource(
        "urn:test:Sample",
        "Sample",
        &["urn:test:weight", "urn:test:count", "urn:test:name"],
    ));
    chain.insert(ranged_property(
        "urn:test:weight",
        "weight",
        "urn:eigenius:core:float",
        Some(0.0),
        Some(100.0),
    ));
    chain.insert(ranged_property(
        "urn:test:count",
        "count",
        "urn:eigenius:core:integer",
        Some(1.0),
        Some(10.0),
    ));
    chain.insert(string_constrained_property(
        "urn:test:name",
        "name",
        Some(1),
        Some(64),
        Some("^[A-Za-z][A-Za-z0-9_-]*$"),
        Some("urn:eigenius:core:formats:iri"),
    ));
    run_full_pipeline_under_lake(&chain, &[iri("urn:test:Sample")], "e2e-generate-refinement");
}

#[test]
#[ignore = "requires lake + a built EigeniusLeanCommon olean cache"]
fn full_pipeline_classref_compiles_under_lake() {
    if !is_lake_available() {
        eprintln!("lake unavailable — skipping");
        return;
    }
    // Doc → Person classref. Tests that the topological order puts
    // Person before Doc and that the cross-class decoder/encoder
    // dispatch (`decodePerson`/`encodePerson`) resolves.
    let mut chain = InMemoryChain::new();
    chain.insert(class_resource(
        "urn:test:Person",
        "Person",
        &["urn:test:p_name"],
    ));
    chain.insert(primitive_property(
        "urn:test:p_name",
        "name",
        "urn:eigenius:core:string",
    ));
    chain.insert(class_resource("urn:test:Doc", "Doc", &["urn:test:author"]));
    chain.insert(classref_property(
        "urn:test:author",
        "author",
        "urn:test:Person",
    ));
    let body = emit_mirror_body(&chain, &[iri("urn:test:Doc")]);
    let work = fresh_workdir("e2e-classref");
    write_lake_project(&work, &body);
    run_lake_build(&work).expect("classref end-to-end mirror must lake-build");
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
#[ignore = "requires lake + a built EigeniusLeanCommon olean cache"]
fn list_and_union_compiles_under_lake() {
    if !is_lake_available() {
        eprintln!("lake unavailable — skipping");
        return;
    }
    let work = fresh_workdir("list-union");
    write_lake_project(&work, &handwritten_list_and_union());
    run_lake_build(&work).expect("list/union mirror must lake-build");
    let _ = std::fs::remove_dir_all(&work);
}
