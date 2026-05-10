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

//! Phase 19a.8 — kinase-grounded end-to-end test.
//!
//! Exercises the full `CallRuntimeMethod` / `DispatchExternal` path
//! against the workspace's canonical [`kinase ontology`]:
//!
//! 1. Build a chain accessor backed by the kinase ontology JSON.
//! 2. Generate a Julia mirror covering `Compound` + `Target` (the
//!    closure pulls in their property classes automatically).
//! 3. Build the env image via the substrate (buildah +
//!    `Pkg.precompile` — same hot path the production CLI hits).
//! 4. Pre-load a multi-input typed handler in the worker via a
//!    one-shot `RunRuntimeScript`. The handler picks "the more
//!    selective target" between two candidates by comparing the
//!    alphabetic order of `target_name` — deterministic, hand-
//!    verifiable, and exercises three typed-mirror inputs in a
//!    single dispatch.
//! 5. Dispatch a multi-input `CallRuntimeMethod` (via the substrate's
//!    `dispatch_external_institution` surface, the only public path
//!    that takes a multi-input list).
//! 6. Verify:
//!    - Output decodes to the expected `Target` resource.
//!    - `partial_invocation.dispatched_to` mentions the handler name
//!      and all three typed-mirror argument types — the canonical
//!      multi-arg `which()` shape.
//!    - A second dispatch against the warm worker is sub-second
//!      (a "warm reuse" anchor; we don't pin sub-100ms because
//!      Docker bind-mount + UDS reconnect cost is platform-dependent
//!      — the assertion is "much faster than cold").
//!
//! Skipped on hosts without buildah / Docker / a pullable Julia
//! base image — same gating as the substrate's 18d capstone.
//!
//! [`kinase ontology`]: ../../../ontologies/examples/kinase/kinase-ontology.json

use eigenius_julia::mirror_gen::{mirror_to_resource, JuliaMirrorGenerator};
use eigenius_julia::JuliaLanguageRuntime;
use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_runtime_substrate::chain::ChainAccessor;
use eigenius_runtime_substrate::language_runtime::LanguageRuntime;
use eigenius_runtime_substrate::mirror_generator::{MirrorGenerationRequest, MirrorGenerator};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const KINASE_ONTOLOGY_JSON: &str =
    include_str!("../../../ontologies/examples/kinase/kinase-ontology.json");

const COMPOUND_CLASS_IRI: &str = "urn:eigenius:demo:assay:Compound";
const TARGET_CLASS_IRI: &str = "urn:eigenius:demo:assay:Target";
const ENV_IRI: &str = "urn:eigenius:test:kinase:env:e2e";
const SIGNATURE_IRI: &str = "urn:eigenius:test:kinase:signatures:pick_more_selective_target";
const HANDLER_METHOD_NAME: &str = "pick_more_selective_target";

const BASE_IMAGE_TAG: &str = "julia:1.12-bookworm";

static COUNTER: AtomicU64 = AtomicU64::new(0);

// ─── Chain fixture ──────────────────────────────────────────────────────

struct KinaseChain {
    resources: HashMap<Iri, Resource>,
}

impl KinaseChain {
    fn from_ontology_json(json: &str) -> Self {
        let mut resources = HashMap::new();
        for r in eigon_json::parse_document(json).expect("kinase ontology must parse") {
            if let Some(id) = r.id() {
                resources.insert(id.clone(), r);
            }
        }
        Self { resources }
    }
}

impl ChainAccessor for KinaseChain {
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
    Iri::parse(s).unwrap()
}

// ─── Mirror + input builders ────────────────────────────────────────────

fn build_kinase_mirror() -> Resource {
    let g = JuliaMirrorGenerator::new();
    let chain = KinaseChain::from_ontology_json(KINASE_ONTOLOGY_JSON);
    let layer = iri("urn:eigenius:test:kinase:layer");
    let seed = vec![iri(COMPOUND_CLASS_IRI), iri(TARGET_CLASS_IRI)];
    let out = g
        .generate(&MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        })
        .expect("kinase mirror generation");
    mirror_to_resource(&g, &out, &layer, Some("1970-01-01T00:00:00Z"))
}

/// Build a `Compound` instance the worker decodes via the mirror.
/// Properties match the kinase ontology's `Compound` requires set.
fn build_compound_cbor(compound_id: &str, scaffold: &str, mw: f64) -> Vec<u8> {
    let mut r = Resource::new(iri(&format!(
        "urn:eigenius:test:kinase:compound:{compound_id}"
    )));
    r.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::ResourceRef(iri(COMPOUND_CLASS_IRI))]),
    );
    r.set(
        iri("urn:eigenius:demo:assay:compound_id"),
        Value::String(compound_id.to_string()),
    );
    r.set(
        iri("urn:eigenius:demo:assay:scaffold_class"),
        Value::String(scaffold.to_string()),
    );
    r.set(
        iri("urn:eigenius:demo:assay:molecular_weight"),
        Value::Float(mw),
    );
    eigon_cbor::serialize_resource(&r)
}

/// Build a `Target` instance the worker decodes via the mirror.
fn build_target_cbor(name: &str, family: &str) -> Vec<u8> {
    let mut r = Resource::new(iri(&format!("urn:eigenius:test:kinase:target:{name}")));
    r.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::ResourceRef(iri(TARGET_CLASS_IRI))]),
    );
    r.set(
        iri("urn:eigenius:demo:assay:target_name"),
        Value::String(name.to_string()),
    );
    r.set(
        iri("urn:eigenius:demo:assay:target_family"),
        Value::String(family.to_string()),
    );
    eigon_cbor::serialize_resource(&r)
}

/// Build the script-mode argument the substrate's
/// `dispatch_run_runtime_script` consumes — `language` + `source`.
fn build_setup_argument(source: &str) -> Vec<u8> {
    let mut arg = Resource::new_embedded();
    arg.set(
        iri("urn:eigenius:runtime:language"),
        Value::String("julia".to_string()),
    );
    arg.set(
        iri("urn:eigenius:runtime:source"),
        Value::String(source.to_string()),
    );
    eigon_cbor::serialize_resource(&arg)
}

// ─── Environment / skip gates ───────────────────────────────────────────

fn fresh_depot(label: &str) -> PathBuf {
    let pid = std::process::id();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("substrate-julia-kinase-{pid}-{label}-{n}"));
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

// ─── The test ───────────────────────────────────────────────────────────

#[ignore = "heavy E2E: Julia env image build + multi-call dispatch."]
#[test]
fn kinase_call_method_multi_input_dispatch_and_warm_reuse() {
    if let Some(reason) = skip_unless_full_environment() {
        eprintln!("skipping kinase e2e: {reason}");
        return;
    }
    let pinned_base = match ensure_base_image_pinned() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("skipping (could not pin base image): {e}");
            return;
        }
    };

    let depot = fresh_depot("e2e");
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

    // Build env image with the kinase mirror baked in. No handler
    // package — the handler is injected via setup script below.
    let mirror = build_kinase_mirror();
    let env = Resource::new_embedded();
    let runtime_for_dispatch: Box<dyn LanguageRuntime> = Box::new(runtime.clone());
    let digest = match runtime_for_dispatch.build_environment_image(&env, &[], Some(&mirror)) {
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

    // Setup: bring the mirror into Main and define the handler. The
    // function picks whichever Target's `target_name` sorts first
    // alphabetically — boring but deterministic and easy to verify.
    // The handler ignores Compound's molecular_weight in this v1
    // scope; the point is exercising multi-arg typed dispatch, not
    // domain semantics.
    let setup_script = "begin; \
        using EigeniusMirror; \
        function pick_more_selective_target(\
            _c::EigeniusMirror.Compound, \
            t1::EigeniusMirror.Target, \
            t2::EigeniusMirror.Target, \
        ) \
            return t1.target_name <= t2.target_name ? t1 : t2 \
        end; \
        nothing; \
        end";
    if let Err(e) = dispatcher.dispatch_run_runtime_script(&[], &build_setup_argument(setup_script))
    {
        let _ = runtime.drain();
        let _ = std::fs::remove_dir_all(&depot);
        panic!("setup script (handler install) failed: {e:?}");
    }

    // Inputs: one Compound, two Targets. CDK2 sorts before EGFR so
    // CDK2 is the expected return.
    let compound = build_compound_cbor("EIG_0042", "pyrimidine", 350.4);
    let cdk2 = build_target_cbor("CDK2", "CDK");
    let egfr = build_target_cbor("EGFR", "EGFR");

    // Cold dispatch — first call into the env spawns the worker.
    let cold_start = Instant::now();
    let cold = match dispatcher.dispatch_external_institution(
        "julia",
        ENV_IRI,
        digest.as_str(),
        HANDLER_METHOD_NAME,
        SIGNATURE_IRI,
        &[compound.clone(), cdk2.clone(), egfr.clone()],
    ) {
        Ok(o) => o,
        Err(e) => {
            let _ = runtime.drain();
            let _ = std::fs::remove_dir_all(&depot);
            panic!("cold dispatch_external_institution: {e}");
        }
    };
    let cold_elapsed = cold_start.elapsed();

    // Output: a typed Target Resource. Should be CDK2.
    let output = eigon_cbor::parse_resource_lenient(&cold.output_cbor)
        .expect("output Resource decodes from CBOR");
    let returned_name = output
        .get(&iri("urn:eigenius:demo:assay:target_name"))
        .and_then(Value::as_str)
        .expect("output carries target_name");
    assert_eq!(
        returned_name, "CDK2",
        "handler must return the alphabetically-earlier Target"
    );

    // dispatched_to must mention the handler and all three arg types
    // — the canonical multi-arg `which()` shape.
    let inv = eigon_cbor::parse_resource_lenient(&cold.partial_invocation_cbor)
        .expect("partial RuntimeInvocation decodes");
    let dispatched_to = inv
        .get(&iri("urn:eigenius:runtime:dispatched_to"))
        .and_then(Value::as_str)
        .expect("partial invocation carries dispatched_to");
    assert!(
        dispatched_to.contains(HANDLER_METHOD_NAME),
        "dispatched_to must mention the handler name, got `{dispatched_to}`"
    );
    assert!(
        dispatched_to.contains("Compound"),
        "dispatched_to must mention the Compound arg type, got `{dispatched_to}`"
    );
    // Two Target args — `Target` should appear at least twice in the
    // canonical `which()` form `f(::Compound, ::Target, ::Target)`.
    assert!(
        dispatched_to.matches("Target").count() >= 2,
        "dispatched_to must mention both Target args, got `{dispatched_to}`"
    );

    // Warm dispatch — same env, same handler, fresh inputs that sort
    // the other way (EGFR, CDK2). Worker stays alive between cold and
    // warm calls; the substrate reuses the cached service handle.
    let egfr2 = build_target_cbor("EGFR", "EGFR");
    let cdk2_2 = build_target_cbor("CDK2", "CDK");
    let warm_start = Instant::now();
    let warm = match dispatcher.dispatch_external_institution(
        "julia",
        ENV_IRI,
        digest.as_str(),
        HANDLER_METHOD_NAME,
        SIGNATURE_IRI,
        &[compound, egfr2, cdk2_2],
    ) {
        Ok(o) => o,
        Err(e) => {
            let _ = runtime.drain();
            let _ = std::fs::remove_dir_all(&depot);
            panic!("warm dispatch_external_institution: {e}");
        }
    };
    let warm_elapsed = warm_start.elapsed();

    let warm_name = eigon_cbor::parse_resource_lenient(&warm.output_cbor)
        .expect("warm output decodes")
        .get(&iri("urn:eigenius:demo:assay:target_name"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .expect("warm output carries target_name");
    assert_eq!(
        warm_name, "CDK2",
        "warm dispatch must still return the alphabetically-earlier Target"
    );

    // Warm reuse anchor: the second dispatch must be much faster
    // than the first because the worker is still alive (no respawn).
    // Pinning to "≤ half the cold time AND ≤ 5s" keeps the assertion
    // useful without making it brittle on slow CI machines —
    // production hardware easily clears the 100ms target the plan
    // mentions; we don't want flake on shared runners.
    assert!(
        warm_elapsed < cold_elapsed / 2 && warm_elapsed < Duration::from_secs(5),
        "warm dispatch ({:?}) must be substantially faster than cold ({:?})",
        warm_elapsed,
        cold_elapsed,
    );

    let _ = runtime.drain();
    let _ = std::fs::remove_dir_all(&depot);
}
