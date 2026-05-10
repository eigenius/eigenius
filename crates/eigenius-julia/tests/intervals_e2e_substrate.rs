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

//! Phase 19a.6 stage 2a-iii — substrate-side e2e for the
//! IntervalArithmetic institution.
//!
//! Validates the engine path (no kernel, no orchestrator gRPC):
//!
//! 1. Generate a `RuntimePackageMirror` for the BoundedBy class via
//!    `JuliaMirrorGenerator`.
//! 2. Build a `RuntimePackage` Resource for the EigeniusIntervals
//!    handler from the on-disk handler sources.
//! 3. Stand up a `JuliaLanguageRuntime` against
//!    `julia/runtime-worker/` and a `DockerServiceSpawner`.
//! 4. Call `build_environment_image` with the mirror + handler
//!    package — exercises the 2a-i wiring end-to-end (image actually
//!    builds with EigeniusIntervals + EigeniusMirror + IntervalArithmetic.jl
//!    baked in).
//! 5. Dispatch via `SubstrateDispatcher::dispatch_external_institution`
//!    with a `BoundedBy(value, lower, upper)` CBOR payload. Assert the
//!    returned Verdict's `ctor_name` matches the expected
//!    Holds/Fails/Undecidable verdict.
//!
//! Expensive: cold runs pull Julia, install IntervalArithmetic.jl
//! from the registry, run `Pkg.precompile` for every baked package.
//! Skipped on hosts without buildah / Docker.

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

// ─── Source-of-truth artifacts (re-bundled from the chain side) ─────────

const INTERVALS_ONTOLOGY_JSON: &str = include_str!(
    "../../../julia/institutions/intervals/declarations/intervals-ontology.eigon.json"
);

const INTERVALS_HANDLER_PROJECT_TOML: &str =
    include_str!("../../../julia/institutions/intervals/EigeniusIntervals/Project.toml");

const INTERVALS_HANDLER_SOURCE_JL: &str = include_str!(
    "../../../julia/institutions/intervals/EigeniusIntervals/src/EigeniusIntervals.jl"
);

// ─── IRIs the test pins ─────────────────────────────────────────────────

const BOUNDED_BY_CLASS_IRI: &str = "urn:eigenius:intervals:BoundedBy";
const VALUE_PROP_IRI: &str = "urn:eigenius:intervals:value";
const LOWER_PROP_IRI: &str = "urn:eigenius:intervals:lower";
const UPPER_PROP_IRI: &str = "urn:eigenius:intervals:upper";
const ENV_IRI: &str = "urn:eigenius:intervals:env:test";
const SIGNATURE_IRI: &str = "urn:eigenius:intervals:signatures:validate_bounded_by";

const HANDLER_PACKAGE_NAME: &str = "EigeniusIntervals";
const HANDLER_METHOD_NAME: &str = "validate_bounded_by";

const BASE_IMAGE_TAG: &str = "julia:1.12-bookworm";

// ─── Helpers ────────────────────────────────────────────────────────────

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}

/// Tiny in-memory `ChainAccessor` carrying the BoundedBy class +
/// its three Float properties. Sufficient for `JuliaMirrorGenerator`
/// to walk the closure (after Stage 1's `is_core_meta_iri` filter, the
/// closure stops at `BoundedBy`).
struct IntervalsChain {
    resources: HashMap<Iri, Resource>,
}

impl IntervalsChain {
    fn from_ontology_json(json: &str) -> Self {
        let mut resources = HashMap::new();
        for r in eigon_json::parse_document(json).expect("intervals ontology must parse") {
            if let Some(id) = r.id() {
                resources.insert(id.clone(), r);
            }
        }
        Self { resources }
    }
}

impl ChainAccessor for IntervalsChain {
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

/// Generate the BoundedBy mirror Resource via `JuliaMirrorGenerator`,
/// then lift it through `mirror_to_resource` so the substrate's
/// build context can materialise it into the image.
fn build_intervals_mirror() -> Resource {
    let g = JuliaMirrorGenerator::new();
    let chain = IntervalsChain::from_ontology_json(INTERVALS_ONTOLOGY_JSON);
    let layer = iri("urn:eigenius:test:intervals:layer");
    let seed = vec![iri(BOUNDED_BY_CLASS_IRI)];
    let out = g
        .generate(&MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        })
        .expect("mirror generation");
    mirror_to_resource(&g, &out, &layer, Some("1970-01-01T00:00:00Z"))
}

/// Build a `RuntimePackage` Resource for `EigeniusIntervals` from the
/// in-tree handler sources. Stage 2a-i's `runtime_package_to_materialization`
/// reads this exact shape.
fn build_intervals_handler_package() -> Resource {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    fn encode(input: &[u8]) -> String {
        let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
        for chunk in input.chunks(3) {
            let b0 = chunk[0];
            let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
            let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };
            out.push(ALPHABET[(b0 >> 2) as usize] as char);
            out.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
            if chunk.len() > 1 {
                out.push(ALPHABET[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
            } else {
                out.push('=');
            }
            if chunk.len() > 2 {
                out.push(ALPHABET[(b2 & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
        out
    }

    let mut r = Resource::new(iri(
        "urn:eigenius:test:intervals:handler-package:EigeniusIntervals",
    ));
    r.set(
        iri("urn:eigenius:runtime:package_name"),
        Value::String(HANDLER_PACKAGE_NAME.to_string()),
    );
    r.set(
        iri("urn:eigenius:runtime:manifest"),
        Value::String(INTERVALS_HANDLER_PROJECT_TOML.to_string()),
    );
    r.set(
        iri("urn:eigenius:runtime:source_tree"),
        Value::Json(serde_json::json!([{
            "path": "src/EigeniusIntervals.jl",
            "content_base64": encode(INTERVALS_HANDLER_SOURCE_JL.as_bytes()),
        }])),
    );
    r
}

/// Build a `BoundedBy` Eigon-CBOR payload the worker decodes into the
/// generated mirror's `BoundedBy` struct. Properties match what the
/// generator emits in `decode_BoundedBy`.
fn build_bounded_by_cbor(value: f64, lower: f64, upper: f64) -> Vec<u8> {
    let mut r = Resource::new(iri("urn:eigenius:test:intervals:input"));
    r.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::ResourceRef(iri(BOUNDED_BY_CLASS_IRI))]),
    );
    r.set(iri(VALUE_PROP_IRI), Value::Float(value));
    r.set(iri(LOWER_PROP_IRI), Value::Float(lower));
    r.set(iri(UPPER_PROP_IRI), Value::Float(upper));
    eigon_cbor::serialize_resource(&r)
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn fresh_depot(label: &str) -> PathBuf {
    let pid = std::process::id();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("substrate-julia-intervals-{pid}-{label}-{n}"));
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
    let qualified = if pinned.contains('/') {
        pinned
    } else {
        format!("docker.io/library/{pinned}")
    };
    Ok(qualified)
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

/// Read the Verdict's `ctor_name` off the substrate's output_cbor.
fn parse_verdict_ctor(output_cbor: &[u8]) -> String {
    let r = eigon_cbor::parse_resource_lenient(output_cbor).expect("decode Verdict resource");
    r.get(&iri("urn:eigenius:core:ctor_name"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .expect("Verdict must carry `core:ctor_name`")
}

// ─── The test ───────────────────────────────────────────────────────────

#[ignore = "heavy E2E: IntervalArithmetic.jl env image build."]
#[test]
fn intervals_substrate_dispatch_round_trip() {
    if let Some(reason) = skip_unless_full_environment() {
        eprintln!("skipping intervals stage 2a-iii: {reason}");
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

    // Inputs: BoundedBy mirror + the handler package as a
    // RuntimePackage Resource.
    let mirror = build_intervals_mirror();
    let handler_pkg = build_intervals_handler_package();
    let env = Resource::new_embedded();

    // Build the image via the engine — this exercises 2a-i (handler
    // package baked alongside Common + mirror, IntervalArithmetic.jl
    // resolved through `Pkg.instantiate`).
    let runtime_for_dispatch: Box<dyn LanguageRuntime> = Box::new(runtime.clone());
    let digest =
        match runtime_for_dispatch.build_environment_image(&env, &[handler_pkg], Some(&mirror)) {
            Ok(d) => d,
            Err(e) => {
                let _ = runtime.drain();
                let _ = std::fs::remove_dir_all(&depot);
                panic!("build_environment_image: {e}");
            }
        };
    assert!(
        digest.as_str().starts_with("sha256:"),
        "expected sha256-shaped digest, got {digest}"
    );

    let mut dispatcher = eigenius_runtime_substrate::facade::SubstrateDispatcher::new();
    dispatcher
        .register_language_runtime(runtime_for_dispatch)
        .expect("register julia runtime");

    // The worker's bootstrap doesn't auto-`using` the baked packages —
    // it only loads its own deps (CBOR, Sockets). To populate the
    // worker's `_eigenius_decoders` registry (so the dispatcher can
    // decode `BoundedBy` inputs) and to put `validate_bounded_by`
    // into `Main`, run a one-shot setup script via
    // `RunRuntimeScript`. Same pattern as the existing
    // `julia_call_runtime_method_dispatches_typed_handler` test in
    // `mirror_image_build_integration.rs`. A future `env build`
    // step (or a self-configuring worker) can fold this into the
    // image bootstrap; for the substrate-side e2e it stays explicit.
    // The worker's bootstrap doesn't auto-`using` the baked packages —
    // it only loads its own deps (CBOR, Sockets). Run a one-shot
    // setup script via `RunRuntimeScript` to put `validate_bounded_by`
    // into `Main` and load the mirror's `_eigenius_decoders` registry
    // (transitively, through `EigeniusIntervals`'s `using EigeniusMirror`).
    // The worker's `Core.eval(Main, fn_symbol)` lookup picks up the
    // new binding at the current world. Same pattern as
    // `julia_call_runtime_method_dispatches_typed_handler` in
    // `mirror_image_build_integration.rs`. A future `env build`
    // step (or self-configuring worker) can fold this into the image
    // bootstrap; for the substrate-side e2e it stays explicit.
    let setup_arg = {
        let mut arg = Resource::new_embedded();
        arg.set(
            iri("urn:eigenius:runtime:language"),
            Value::String("julia".into()),
        );
        arg.set(
            iri("urn:eigenius:runtime:source"),
            Value::String("begin; using EigeniusIntervals; nothing; end".into()),
        );
        eigon_cbor::serialize_resource(&arg)
    };
    if let Err(e) = dispatcher.dispatch_run_runtime_script(&[], &setup_arg) {
        let _ = runtime.drain();
        let _ = std::fs::remove_dir_all(&depot);
        panic!("setup script (using EigeniusIntervals) failed: {e}");
    }

    // Holds case: 2 ∈ [1, 3].
    let holds = match dispatcher.dispatch_external_institution(
        "julia",
        ENV_IRI,
        digest.as_str(),
        HANDLER_METHOD_NAME,
        SIGNATURE_IRI,
        &[build_bounded_by_cbor(2.0, 1.0, 3.0)],
    ) {
        Ok(o) => o,
        Err(e) => {
            let _ = runtime.drain();
            let _ = std::fs::remove_dir_all(&depot);
            panic!("dispatch_external_institution (Holds case): {e}");
        }
    };
    assert_eq!(parse_verdict_ctor(&holds.output_cbor), "Holds");
    assert!(
        !holds.partial_invocation_cbor.is_empty(),
        "partial RuntimeInvocation must ride alongside the Verdict"
    );

    // Fails case: 5 ∉ [1, 3].
    let fails = match dispatcher.dispatch_external_institution(
        "julia",
        ENV_IRI,
        digest.as_str(),
        HANDLER_METHOD_NAME,
        SIGNATURE_IRI,
        &[build_bounded_by_cbor(5.0, 1.0, 3.0)],
    ) {
        Ok(o) => o,
        Err(e) => {
            let _ = runtime.drain();
            let _ = std::fs::remove_dir_all(&depot);
            panic!("dispatch_external_institution (Fails case): {e}");
        }
    };
    assert_eq!(parse_verdict_ctor(&fails.output_cbor), "Fails");

    let _ = runtime.drain();
    let _ = std::fs::remove_dir_all(&depot);
}
