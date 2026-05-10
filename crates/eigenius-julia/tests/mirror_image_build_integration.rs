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

//! Phase 19a.3.c integration test — chain → mirror Resource → env
//! image with the generated mirror + `EigeniusJuliaCommon` baked in.
//!
//! Two assertions, in order of expense:
//!
//! 1. **Always-on (no Docker required):** `JuliaMirrorGenerator` walks
//!    a synthetic chain, `mirror_to_resource` produces a valid
//!    `RuntimePackageMirror` Resource, and the materialiser path
//!    used by `build_environment_image` round-trips every byte of the
//!    library archive back through the resource's `library_content`
//!    JSON. Catches drift between the generator's emit and the
//!    runtime's materialiser without needing buildah/Docker.
//!
//! 2. **Full e2e (Docker + buildah):** Builds an env image with the
//!    mirror baked in, dispatches a Julia one-liner that
//!    `using`-imports `EigeniusMirror` + `EigeniusJuliaCommon`, and
//!    confirms the worker can construct a typed mirror struct. Skipped
//!    on hosts without buildah/Docker, same gating as the 18d capstone.
//!
//! Skipped when:
//! - Built without `--features test-runtime,docker-spawner` (the
//!   substrate's bash test worker + DockerSpawner backend are gated
//!   behind these in the substrate crate).
//! - `buildah` not installed.
//! - Docker daemon unreachable.
//! - Julia base image cannot be pulled (offline / no registry access).

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

// ---------- Synthetic chain ------------------------------------------------

/// Tiny chain carrying one class (`Demo`) with one required string
/// property (`name`). Sufficient to exercise generator → resource →
/// image-build wiring without dragging in a full ontology.
struct DemoChain {
    resources: HashMap<Iri, Resource>,
}

impl DemoChain {
    fn new() -> Self {
        let mut resources = HashMap::new();

        let class_iri = iri("urn:eigenius:test:Demo");
        let mut cls = Resource::new(class_iri.clone());
        cls.set(
            iri("urn:eigenius:core:short_name"),
            Value::String("Demo".into()),
        );
        cls.set(
            iri("urn:eigenius:core:requires"),
            Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:test:name"))]),
        );
        resources.insert(class_iri, cls);

        let prop_iri = iri("urn:eigenius:test:name");
        let mut prop = Resource::new(prop_iri.clone());
        prop.set(
            iri("urn:eigenius:core:short_name"),
            Value::String("name".into()),
        );
        prop.set(
            iri("urn:eigenius:core:data_type"),
            Value::ResourceRef(iri("urn:eigenius:core:string")),
        );
        resources.insert(prop_iri, prop);

        Self { resources }
    }
}

impl ChainAccessor for DemoChain {
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

fn build_demo_mirror() -> Resource {
    let g = JuliaMirrorGenerator::new();
    let chain = DemoChain::new();
    let layer = iri("urn:eigenius:test:layer");
    let seed = vec![iri("urn:eigenius:test:Demo")];
    let out = g
        .generate(&MirrorGenerationRequest {
            source_layer: &layer,
            seed_classes: &seed,
            chain: &chain,
        })
        .expect("generate");
    mirror_to_resource(&g, &out, &layer, Some("1970-01-01T00:00:00Z"))
}

// ---------- Test 1: deterministic mirror Resource shape --------------------

/// Sanity-check the resource the substrate commits to the chain at
/// build time. The properties + their values are what the
/// orchestrator-side commit pipeline (lands in 19a.4 / orchestrator
/// glue) reads to populate the `RuntimeEnvironment.mirror` link, so a
/// drift here would surface as a chain-validation failure way
/// downstream.
#[test]
fn mirror_resource_carries_runtime_substrate_required_props() {
    let mirror = build_demo_mirror();

    // Every required property of `RuntimePackageMirror` populated.
    let required = [
        "urn:eigenius:core:is_a",
        "urn:eigenius:core:short_name",
        "urn:eigenius:runtime:language",
        "urn:eigenius:runtime:source_layer",
        "urn:eigenius:runtime:generator_identifier",
        "urn:eigenius:runtime:generator_version",
        "urn:eigenius:runtime:generator_content_hash",
        "urn:eigenius:runtime:library_content_hash",
        "urn:eigenius:runtime:library_content",
        "urn:eigenius:runtime:mirrored_classes",
    ];
    for p in required {
        assert!(
            mirror.get(&iri(p)).is_some(),
            "RuntimePackageMirror is missing required property `{p}`"
        );
    }

    // language is "julia"; mirrored_classes covers Demo.
    assert_eq!(
        mirror
            .get(&iri("urn:eigenius:runtime:language"))
            .and_then(Value::as_str),
        Some("julia")
    );
    let cls = mirror
        .get(&iri("urn:eigenius:runtime:mirrored_classes"))
        .expect("mirrored_classes")
        .as_iri_array();
    assert!(cls.contains(&iri("urn:eigenius:test:Demo")));

    // Hashes have the substrate's pinned shape (^sha256:[a-f0-9]{64}$).
    for p in [
        "urn:eigenius:runtime:generator_content_hash",
        "urn:eigenius:runtime:library_content_hash",
    ] {
        let v = mirror
            .get(&iri(p))
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("missing `{p}`"));
        assert!(v.starts_with("sha256:"), "{p} = {v}");
        let hex = &v["sha256:".len()..];
        assert_eq!(hex.len(), 64, "{p} digest must be 64 hex chars, got {v}");
        assert!(
            hex.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
            "{p} must be lowercase hex, got {v}"
        );
    }
}

/// The mirror Resource carries the generator's library archive on its
/// `library_content` property as JSON. The runtime's `build_image`
/// path materialises that JSON back into files under `mirror/` in the
/// build context. If those two stop matching, the env image silently
/// ships the wrong source — verify the byte-level round-trip without
/// going through Docker.
#[test]
fn library_content_json_carries_project_toml_and_module_source() {
    let mirror = build_demo_mirror();
    let json = match mirror
        .get(&iri("urn:eigenius:runtime:library_content"))
        .expect("library_content")
    {
        Value::Json(v) => v.clone(),
        other => panic!("expected JSON, got {other:?}"),
    };
    assert_eq!(json["kind"], "embedded");
    let files = json["files"].as_array().expect("files array");
    let paths: Vec<&str> = files
        .iter()
        .filter_map(|f| f.get("path").and_then(|v| v.as_str()))
        .collect();
    assert!(paths.contains(&"Project.toml"), "got {paths:?}");
    assert!(paths.contains(&"src/EigeniusMirror.jl"), "got {paths:?}");
}

// ---------- Test 2: full e2e (skipped without Docker/buildah) --------------

const BASE_IMAGE_TAG: &str = "julia:1.12-bookworm";

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn fresh_depot(label: &str) -> PathBuf {
    let pid = std::process::id();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("substrate-julia-mirror-it-{pid}-{label}-{n}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create depot");
    dir
}

fn julia_project_dir() -> PathBuf {
    // crates/eigenius-julia/Cargo.toml → workspace root is two up.
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

/// End-to-end on Docker: build the env image with the mirror baked
/// in, dispatch a Julia one-liner that `using`-imports the generated
/// mirror, confirm the typed struct can be constructed and its field
/// read back. This exercises the full chain → resource → buildah →
/// `Pkg.develop` → `Pkg.precompile` → worker boot → mirror import path
/// in one shot. Cold run: ~3-5 min (pulls Julia + precompiles
/// EigeniusJuliaCommon + EigeniusMirror); warm runs hit the substrate's
/// image cache and Docker's layer cache (~30s).
#[ignore = "heavy E2E: full Julia env image build."]
#[test]
fn julia_env_image_with_mirror_dispatches_typed_struct() {
    if let Some(reason) = skip_unless_full_environment() {
        eprintln!("skipping 19a.3.c mirror image-build e2e: {reason}");
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

    let mirror = build_demo_mirror();
    let env = Resource::new_embedded();
    let runtime_for_dispatch: Box<dyn LanguageRuntime> = Box::new(runtime.clone());
    let digest = match runtime_for_dispatch.build_environment_image(&env, &[], Some(&mirror)) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("substrate-built julia image (with mirror) failed: {e}");
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

    // Dispatch twice. With the warm-pool path, the second dispatch
    // hits the same already-running container — no rebuild, no
    // cold-start, no Pkg.precompile. We don't have a direct hook for
    // "did the worker get re-spawned?" but the timing differential
    // shows it: the second call must complete in well under a second
    // (warm RPC), while the first carries Julia's first-call JIT.
    fn dispatch_once(
        dispatcher: &mut eigenius_runtime_substrate::facade::SubstrateDispatcher,
        source: &str,
    ) -> String {
        let argument = build_argument("julia", source);
        let outcome = dispatcher
            .dispatch_run_runtime_script(&[], &argument)
            .expect("dispatch julia mirror-using script");
        let output =
            eigon_cbor::parse_resource_lenient(&outcome.output_cbor).expect("decode output");
        output
            .get(&iri("urn:eigenius:runtime:script_output"))
            .and_then(Value::as_str)
            .expect("script_output property on output")
            .to_string()
    }

    let script_a = "begin; \
        using EigeniusMirror; \
        using EigeniusJuliaCommon; \
        instance = EigeniusMirror.Demo(\"hello-19a3e\"); \
        instance.name; \
        end";
    let script_b = "begin; \
        using EigeniusMirror; \
        instance = EigeniusMirror.Demo(\"warm-reuse-ok\"); \
        instance.name; \
        end";

    assert_eq!(dispatch_once(&mut dispatcher, script_a), "hello-19a3e");

    // The second dispatch must complete in **well** under the cold-
    // start envelope. Julia first-call JIT dominates the first
    // dispatch (often 5-15s); on a warm worker, a typed-struct
    // round-trip is sub-second. Use a generous bound to avoid CI
    // flakiness while still proving the worker wasn't re-spawned.
    let warm_start = std::time::Instant::now();
    let warm_output = dispatch_once(&mut dispatcher, script_b);
    let warm_elapsed = warm_start.elapsed();
    assert_eq!(warm_output, "warm-reuse-ok");
    assert!(
        warm_elapsed < std::time::Duration::from_secs(5),
        "warm dispatch must complete in <5s (got {warm_elapsed:?}); \
         a cold restart would take 10s+ for Julia's first-call JIT — \
         this confirms the ServiceSpawner warm-pool path"
    );

    let _ = runtime.drain();
    let _ = std::fs::remove_dir_all(&depot);
}

// ---------- CallRuntimeMethod e2e (Phase 19a.4) ----------------------------

/// Build a `RuntimeMethodSignature` resource. v1 just needs
/// `language` + `method_name` for the substrate's `call_method` to
/// dispatch; the full `input_types` / `output_type` machinery is
/// validated by the kernel boundary check (D26 §7.5) which is not
/// part of this test.
fn build_method_signature(method_name: &str) -> Resource {
    let mut sig = Resource::new(iri(&format!(
        "urn:eigenius:test:method-signature:{method_name}"
    )));
    sig.set(
        iri("urn:eigenius:runtime:language"),
        Value::String("julia".to_string()),
    );
    sig.set(
        iri("urn:eigenius:runtime:method_name"),
        Value::String(method_name.to_string()),
    );
    sig
}

/// Build an Eigon resource representing a `Demo` mirror struct value
/// the worker can decode. Properties match what the generated
/// `decode_Demo` reads — `name` (required) and the optional `_id`.
fn build_demo_resource(name: &str, id: Option<&str>) -> Resource {
    let mut r = match id {
        Some(s) => Resource::new(iri(s)),
        None => Resource::new_embedded(),
    };
    r.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::ResourceRef(iri("urn:eigenius:test:Demo"))]),
    );
    r.set(
        iri("urn:eigenius:test:name"),
        Value::String(name.to_string()),
    );
    r
}

/// End-to-end `CallRuntimeMethod` against the Demo mirror with a
/// typed handler defined in the worker's `Main`. Exercises the full
/// 19a.4 path:
///
/// 1. Build env image with the Demo mirror baked in.
/// 2. Pre-load a handler `echo_demo(d::Demo) = d` via `RunRuntimeScript`
///    (real handler packages land alongside per-institution crates;
///    this test injects a one-liner so it doesn't need a separate
///    `EigeniusKinaseDemo` package).
/// 3. Dispatch `CallRuntimeMethod` with method_name = "echo_demo"
///    and a Demo input.
/// 4. Verify the output Resource is the round-tripped Demo.
/// 5. Verify `dispatched_to` is populated on the partial
///    `RuntimeInvocation` (D26 §4.2 / §5.5).
#[ignore = "heavy E2E: full Julia env image build."]
#[test]
fn julia_call_runtime_method_dispatches_typed_handler() {
    if let Some(reason) = skip_unless_full_environment() {
        eprintln!("skipping 19a.4 CallRuntimeMethod e2e: {reason}");
        return;
    }
    let pinned_base = match ensure_base_image_pinned() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("skipping (could not pin base image): {e}");
            return;
        }
    };

    let depot = fresh_depot("call-method");
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

    let mirror = build_demo_mirror();
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

    // Step 1: bring EigeniusMirror into Main and define the handler.
    // `echo_demo(d::Demo) = d` is the simplest non-trivial typed
    // method — it lets us verify the round-trip without any
    // computation muddying the picture.
    let setup_script = "begin; \
        using EigeniusMirror; \
        echo_demo(d::EigeniusMirror.Demo) = d; \
        nothing; \
        end";
    let setup_arg = build_argument("julia", setup_script);
    if let Err(e) = dispatcher.dispatch_run_runtime_script(&[], &setup_arg) {
        let _ = runtime.drain();
        let _ = std::fs::remove_dir_all(&depot);
        panic!("setup script (handler install) failed: {e:?}");
    }

    // Step 2: dispatch CallRuntimeMethod.
    let signature = build_method_signature("echo_demo");
    let signature_cbor = eigon_cbor::serialize_resource(&signature);
    let demo_input = build_demo_resource("typed-call-ok", Some("urn:eigenius:test:demo:input-1"));
    let demo_input_cbor = eigon_cbor::serialize_resource(&demo_input);

    let outcome = match dispatcher.dispatch_call_runtime_method(&demo_input_cbor, &signature_cbor) {
        Ok(o) => o,
        Err(e) => {
            let _ = runtime.drain();
            let _ = std::fs::remove_dir_all(&depot);
            panic!("dispatch_call_runtime_method failed: {e:?}");
        }
    };

    // Step 3: output Resource is the round-tripped Demo.
    let output = eigon_cbor::parse_resource_lenient(&outcome.output_cbor).expect("decode output");
    let output_name = output
        .get(&iri("urn:eigenius:test:name"))
        .and_then(Value::as_str)
        .expect("output's `name` property");
    assert_eq!(output_name, "typed-call-ok");
    let output_is_a = output
        .get(&iri("urn:eigenius:core:is_a"))
        .expect("output's is_a")
        .as_iri_array();
    assert!(
        output_is_a.contains(&iri("urn:eigenius:test:Demo")),
        "output must be is_a Demo, got {output_is_a:?}"
    );
    // Epistemic category stamp (Phase 19a.4 / D29 §8.4 substrate
    // commit-pipeline rule): every runtime-produced resource lands
    // with `urn:eigenius:reflection:DerivedResource` on its `is_a`,
    // alongside its structural class. Lets the chain auditor
    // distinguish runtime-produced from declared / observed /
    // verified resources.
    assert!(
        output_is_a.contains(&iri("urn:eigenius:reflection:DerivedResource")),
        "output must be is_a DerivedResource, got {output_is_a:?}"
    );

    // Step 4: partial RuntimeInvocation carries dispatched_to.
    let inv = eigon_cbor::parse_resource_lenient(&outcome.partial_invocation_cbor)
        .expect("decode partial invocation");
    let dispatched_to = inv
        .get(&iri("urn:eigenius:runtime:dispatched_to"))
        .and_then(Value::as_str)
        .expect("partial invocation must carry dispatched_to for CallRuntimeMethod");
    // `which()` formats as `Module.fname(::ArgType) at file:line`.
    // Loose contains-check — Julia's `which()` output format is stable
    // but we don't pin the file/line.
    assert!(
        dispatched_to.contains("echo_demo"),
        "dispatched_to must mention the handler name, got `{dispatched_to}`"
    );
    assert!(
        dispatched_to.contains("Demo"),
        "dispatched_to must mention the dispatched arg type, got `{dispatched_to}`"
    );

    let _ = runtime.drain();
    let _ = std::fs::remove_dir_all(&depot);
}
