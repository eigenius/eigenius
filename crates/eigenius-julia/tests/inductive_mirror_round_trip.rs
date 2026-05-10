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

//! Phase 19d.0.c — inductive mirror round-trip e2e.
//!
//! Builds a chain carrying a hand-rolled `Nat = zero | succ(Nat)`,
//! generates a Julia mirror for it (D32 §3.6 — abstract type +
//! concrete-per-ctor structs + decode/encode), bakes the mirror into
//! an env image, and dispatches a Julia script that:
//!
//! 1. `using`s the generated `EigeniusMirror` module.
//! 2. Constructs a typed `Nat_succ(Nat_succ(Nat_zero()))` value.
//! 3. Encodes it via `encode_Nat(...)` to the chain wire shape.
//! 4. Decodes the wire shape back via `decode_Nat(...)`.
//! 5. Re-encodes and confirms byte-equality between the two
//!    encodings (the round-trip invariant).
//!
//! Skipped on hosts without buildah / Docker / a pullable Julia
//! base image — same gating as the substrate's 18d capstone.

use eigenius_julia::mirror_gen::{mirror_to_resource, JuliaMirrorGenerator};
use eigenius_julia::JuliaLanguageRuntime;
use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_runtime_substrate::chain::ChainAccessor;
use eigenius_runtime_substrate::language_runtime::LanguageRuntime;
use eigenius_runtime_substrate::mirror_generator::{MirrorGenerationRequest, MirrorGenerator};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const BASE_IMAGE_TAG: &str = "julia:1.12-bookworm";

const PROP_IS_A: &str = "urn:eigenius:core:is_a";
const PROP_SHORT_NAME: &str = "urn:eigenius:core:short_name";
const PROP_CTORS: &str = "urn:eigenius:core:ctors";
const PROP_CTOR_NAME: &str = "urn:eigenius:core:ctor_name";
const PROP_ARG_TYPES: &str = "urn:eigenius:core:arg_types";
const PROP_TYPE_NAME: &str = "urn:eigenius:core:type_name";
const PROP_ARG_NAME: &str = "urn:eigenius:core:arg_name";
const CLASS_INDUCTIVE_TYPE: &str = "urn:eigenius:core:InductiveType";
const CLASS_INDUCTIVE_CTOR: &str = "urn:eigenius:core:InductiveCtor";
const CLASS_INDUCTIVE_ARG_TYPE: &str = "urn:eigenius:core:InductiveArgType";

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}

// ─── Chain fixture ──────────────────────────────────────────────────────

/// In-memory chain with one hand-rolled inductive: `Nat = zero | succ(Nat)`.
struct NatChain {
    resources: HashMap<Iri, Resource>,
}

impl NatChain {
    fn new() -> Self {
        let mut resources = HashMap::new();

        // ctor `zero`: no args.
        let mut zero = Resource::new(iri("urn:eigenius:test:Nat:zero"));
        zero.set(
            iri(PROP_IS_A),
            Value::Array(vec![Value::ResourceRef(iri(CLASS_INDUCTIVE_CTOR))]),
        );
        zero.set(iri(PROP_CTOR_NAME), Value::String("zero".into()));
        zero.set(iri(PROP_ARG_TYPES), Value::Array(vec![]));

        // ctor `succ(pred: Nat)`.
        let mut succ_arg = Resource::new(iri("urn:eigenius:test:Nat:succ:pred"));
        succ_arg.set(
            iri(PROP_IS_A),
            Value::Array(vec![Value::ResourceRef(iri(CLASS_INDUCTIVE_ARG_TYPE))]),
        );
        succ_arg.set(iri(PROP_ARG_NAME), Value::String("pred".into()));
        succ_arg.set(
            iri(PROP_TYPE_NAME),
            Value::String("urn:eigenius:test:Nat".into()),
        );

        let mut succ = Resource::new(iri("urn:eigenius:test:Nat:succ"));
        succ.set(
            iri(PROP_IS_A),
            Value::Array(vec![Value::ResourceRef(iri(CLASS_INDUCTIVE_CTOR))]),
        );
        succ.set(iri(PROP_CTOR_NAME), Value::String("succ".into()));
        succ.set(
            iri(PROP_ARG_TYPES),
            Value::Array(vec![Value::Embedded(Box::new(succ_arg))]),
        );

        let mut nat = Resource::new(iri("urn:eigenius:test:Nat"));
        nat.set(
            iri(PROP_IS_A),
            Value::Array(vec![Value::ResourceRef(iri(CLASS_INDUCTIVE_TYPE))]),
        );
        nat.set(iri(PROP_SHORT_NAME), Value::String("Nat".into()));
        nat.set(
            iri(PROP_CTORS),
            Value::Array(vec![
                Value::Embedded(Box::new(zero)),
                Value::Embedded(Box::new(succ)),
            ]),
        );

        resources.insert(iri("urn:eigenius:test:Nat"), nat);
        Self { resources }
    }
}

impl ChainAccessor for NatChain {
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

fn build_nat_mirror() -> Resource {
    let g = JuliaMirrorGenerator::new();
    let chain = NatChain::new();
    let layer = iri("urn:eigenius:test:Nat:layer");
    let seed = vec![iri("urn:eigenius:test:Nat")];
    let out = g
        .generate(&MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        })
        .expect("nat mirror generation");
    mirror_to_resource(&g, &out, &layer, Some("1970-01-01T00:00:00Z"))
}

// ─── Environment / skip gates ───────────────────────────────────────────

fn fresh_depot(label: &str) -> PathBuf {
    let pid = std::process::id();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("substrate-julia-nat-{pid}-{label}-{n}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create depot");
    dir
}

fn julia_project_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("julia")
        .join("runtime-worker")
        .canonicalize()
        .expect("julia/runtime-worker/ must exist relative to eigenius-julia's Cargo.toml")
}

fn is_docker_available() -> bool {
    Path::new("/var/run/docker.sock").exists()
}

fn ensure_base_image_pinned() -> Result<String, String> {
    let pull = std::process::Command::new("docker")
        .args(["pull", "--quiet", BASE_IMAGE_TAG])
        .output()
        .map_err(|e| format!("`docker pull` failed: {e}"))?;
    if !pull.status.success() {
        return Err(format!(
            "docker pull {BASE_IMAGE_TAG} exited {}: {}",
            pull.status,
            String::from_utf8_lossy(&pull.stderr)
        ));
    }
    let inspect = std::process::Command::new("docker")
        .args([
            "image",
            "inspect",
            "--format",
            "{{index .RepoDigests 0}}",
            BASE_IMAGE_TAG,
        ])
        .output()
        .map_err(|e| format!("`docker image inspect` failed: {e}"))?;
    if !inspect.status.success() {
        return Err(format!(
            "docker image inspect failed: {}",
            String::from_utf8_lossy(&inspect.stderr)
        ));
    }
    let pinned = String::from_utf8_lossy(&inspect.stdout).trim().to_string();
    if !pinned.contains('@') {
        return Err(format!(
            "expected RepoDigests entry like `julia@sha256:...`, got `{pinned}`"
        ));
    }
    Ok(if pinned.contains('/') {
        pinned
    } else {
        format!("docker.io/library/{pinned}")
    })
}

fn skip_unless_full_environment() -> Option<String> {
    if !is_docker_available() {
        return Some("Docker socket unavailable".into());
    }
    if !eigenius_runtime_substrate::is_buildah_available() {
        return Some("buildah unavailable".into());
    }
    None
}

fn build_argument(language: &str, source: &str) -> Vec<u8> {
    let mut arg = Resource::new_embedded();
    arg.set(
        iri("urn:eigenius:runtime:language"),
        Value::String(language.to_string()),
    );
    arg.set(
        iri("urn:eigenius:runtime:source"),
        Value::String(source.to_string()),
    );
    eigon_cbor::serialize_resource(&arg)
}

// ─── The test ───────────────────────────────────────────────────────────

#[ignore = "heavy E2E: Julia env image build for inductive mirror generator round-trip."]
#[test]
fn nat_inductive_round_trips_through_generated_mirror() {
    if let Some(reason) = skip_unless_full_environment() {
        eprintln!("skipping inductive mirror round-trip: {reason}");
        return;
    }
    let pinned_base = match ensure_base_image_pinned() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("skipping (could not pin base image): {e}");
            return;
        }
    };

    let depot = fresh_depot("rt");
    let spawner = match eigenius_runtime_substrate::spawner::service::DockerServiceSpawner::new(
        eigenius_runtime_substrate::spawner::DockerSpawnerConfig::new(depot.clone()),
    ) {
        Ok(s) => std::sync::Arc::new(s),
        Err(e) => {
            eprintln!("skipping (DockerServiceSpawner construction failed): {e}");
            return;
        }
    };

    let project_dir = julia_project_dir();
    let runtime = std::sync::Arc::new(JuliaLanguageRuntime::new(
        project_dir,
        pinned_base,
        spawner.clone(),
        depot.clone(),
    ));

    let mirror = build_nat_mirror();
    let env = Resource::new_embedded();
    let runtime_for_dispatch: Box<dyn LanguageRuntime> = Box::new(runtime.clone());
    let _digest = match runtime_for_dispatch.build_environment_image(&env, &[], Some(&mirror)) {
        Ok(d) => d,
        Err(e) => {
            let _ = runtime.drain();
            let _ = std::fs::remove_dir_all(&depot);
            panic!("build_environment_image: {e}");
        }
    };

    let mut dispatcher = eigenius_runtime_substrate::facade::SubstrateDispatcher::new();
    dispatcher
        .register_language_runtime(runtime_for_dispatch)
        .expect("register julia runtime");

    // Round-trip script:
    //   1. Construct a typed Nat tree (succ(succ(zero))).
    //   2. Encode it to the chain wire shape.
    //   3. Decode back to a typed Nat.
    //   4. Re-encode.
    //   5. Confirm both encodings agree, and that the decoded value
    //      sits in the right concrete-ctor type (`Nat_succ`).
    //
    // The script returns "ok:<typeof>" on success — non-empty so we
    // can assert against it; the typeof anchor proves the decoded
    // value really is in the typed hierarchy.
    let script = "begin; \
        using EigeniusMirror; \
        original = EigeniusMirror.Nat_succ(EigeniusMirror.Nat_succ(EigeniusMirror.Nat_zero())); \
        encoded_a = EigeniusMirror.encode_Nat(original); \
        decoded   = EigeniusMirror.decode_Nat(encoded_a); \
        encoded_b = EigeniusMirror.encode_Nat(decoded); \
        if encoded_a == encoded_b && decoded isa EigeniusMirror.Nat_succ \
            \"ok:\" * string(typeof(decoded)) \
        else \
            error(\"round-trip mismatch: encoded_a=$(encoded_a), encoded_b=$(encoded_b), decoded=$(decoded)\") \
        end \
        end";

    let argument = build_argument("julia", script);
    let outcome = match dispatcher.dispatch_run_runtime_script(&[], &argument) {
        Ok(o) => o,
        Err(e) => {
            let _ = runtime.drain();
            let _ = std::fs::remove_dir_all(&depot);
            panic!("dispatch failed: {e:?}");
        }
    };

    let output =
        eigon_cbor::parse_resource_lenient(&outcome.output_cbor).expect("decode output Resource");
    let script_output = output
        .get(&iri("urn:eigenius:runtime:script_output"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .expect("script_output property on output");

    assert!(
        script_output.starts_with("ok:") && script_output.contains("Nat_succ"),
        "round-trip script must succeed and return the concrete-ctor type; got {script_output:?}"
    );

    let _ = runtime.drain();
    let _ = std::fs::remove_dir_all(&depot);
}
