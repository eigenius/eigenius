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

//! Phase 20a.5b.4 integration test — full Docker-mode `lean_export`
//! round-trip against a substrate-built env image.
//!
//! Mirrors `eigenius-julia/tests/mirror_image_build_integration.rs`
//! (the closest analogue in the Julia institution). The Julia test
//! exercises mirror generation → image build → typed dispatch; this
//! test exercises image build → `lean_export` verb → ndjson output.
//!
//! Skipped when:
//! - `buildah` not installed.
//! - Docker daemon unreachable.
//! - Debian base image cannot be pulled (offline / no registry access).
//! - The host-side worker binary or cdylib hasn't been built.
//!
//! Cold run cost (full image build): ~5-15 minutes — elan downloads
//! the Lean toolchain, lake compiles lean4export against it, debian:
//! bookworm-slim pulls. Warm runs hit the substrate's image cache
//! (cached_digest) and complete in seconds.

use base64::Engine as _;
use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_lean_runtime::mirror_gen::{mirror_to_resource, LeanMirrorGenerator};
use eigenius_lean_runtime::{build_target_constant, build_target_module, LeanLanguageRuntime};
use eigenius_runtime_substrate::chain::ChainAccessor;
use eigenius_runtime_substrate::language_runtime::LanguageRuntime;
use eigenius_runtime_substrate::mirror_generator::{MirrorGenerationRequest, MirrorGenerator};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

// ─── Lean-runtime IRIs (mirror crate::conventions) ─────────────────

const PROP_IS_A: &str = "urn:eigenius:core:is_a";
const PROP_LANGUAGE: &str = "urn:eigenius:runtime:language";
const PROP_METHOD_NAME: &str = "urn:eigenius:runtime:method_name";
const PROP_SCRIPT_OUTPUT: &str = "urn:eigenius:runtime:script_output";

const LEAN_PROJECT_IRI: &str = "urn:eigenius:lean:LeanProject";
const PROP_LAKEFILE: &str = "urn:eigenius:lean:lakefile";
const PROP_LAKE_MANIFEST: &str = "urn:eigenius:lean:lake_manifest";
const PROP_SOURCE_TREE: &str = "urn:eigenius:runtime:source_tree";

/// In-image path of the vendored lean4export Lake project. The
/// Dockerfile composer COPYs the source tree here and pre-builds it
/// in `install_packages`. The `LeanProject` the test ships
/// references this absolute path as its `lean4export` require dep —
/// `lake build` inside the worker's temp dir resolves the path
/// against the in-image copy.
const LEAN4EXPORT_IN_IMAGE: &str = "/opt/lean4export";

// ─── Skip-gate helpers ─────────────────────────────────────────────

const BASE_IMAGE_TAG: &str = "debian:bookworm-slim";

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn fresh_depot(label: &str) -> PathBuf {
    let pid = std::process::id();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("substrate-lean-it-{pid}-{label}-{n}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create depot");
    dir
}

fn lean_project_dir() -> PathBuf {
    // crates/eigenius-lean-runtime/Cargo.toml → workspace root is
    // two up, then into lean/runtime-worker/.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("lean")
        .join("runtime-worker")
        .canonicalize()
        .expect("lean/runtime-worker/ must exist relative to eigenius-lean-runtime's Cargo.toml")
}

fn lean_common_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("lean")
        .join("common")
        .join("EigeniusLeanCommon")
        .canonicalize()
        .expect("lean/common/EigeniusLeanCommon/ must exist relative to the crate's Cargo.toml")
}

fn cdylib_path() -> PathBuf {
    // The cdylib lives in the workspace target dir. Cargo runs tests
    // with CARGO_TARGET_TMPDIR pointing into target/, but we want the
    // sibling debug/release dir holding libeigenius_lean_worker.so.
    // Resolve via CARGO_MANIFEST_DIR → workspace root → target/.
    //
    // Honors CARGO_TARGET_DIR when set (matches what the workspace's
    // own builds do).
    let target_dir = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("target")
        });
    // Prefer debug — the worker's lakefile.lean hard-codes
    // target/debug/ in its `-L` flag, so the binary is linked
    // against the debug cdylib. A future release-mode worker would
    // need a coordinated change there.
    target_dir.join("debug").join("libeigenius_lean_worker.so")
}

fn worker_binary_path() -> PathBuf {
    lean_project_dir()
        .join(".lake")
        .join("build")
        .join("bin")
        .join("lean-runtime-worker")
}

fn is_docker_available() -> bool {
    Path::new("/var/run/docker.sock").exists()
}

/// Ensure the Debian base image is pulled + return its digest-pinned
/// reference. Pinning by digest keeps the env image build
/// reproducible — a tag like `debian:bookworm-slim` moves over time;
/// `debian@sha256:...` doesn't.
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
            "expected RepoDigests entry like `debian@sha256:...`, got `{pinned}`"
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
    if !worker_binary_path().exists() {
        return Some(format!(
            "Lake-built worker binary not found at `{}` — run `(cd lean/runtime-worker && lake build)` first",
            worker_binary_path().display()
        ));
    }
    if !cdylib_path().exists() {
        return Some(format!(
            "Rust cdylib not found at `{}` — run `cargo build -p eigenius-lean-worker` first",
            cdylib_path().display()
        ));
    }
    None
}

// ─── Test fixture construction ─────────────────────────────────────

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}

/// Build a minimal `LeanProject` Eigon-CBOR resource carrying a
/// single-theorem Lean project. The lakefile references
/// `/opt/lean4export` (the in-image path) — when the worker stages
/// the project files into its temp dir, `lake build` resolves the
/// `lean4export` require dep against the pre-built copy in
/// the image instead of trying to fetch + compile it from scratch.
fn make_lean_project_cbor(target_theorem_source: &str) -> Vec<u8> {
    let lakefile = format!(
        "name = \"TestProject\"\n\
         defaultTargets = [\"TestProject\"]\n\
         \n\
         [[lean_lib]]\n\
         name = \"TestProject\"\n\
         \n\
         [[require]]\n\
         name = \"lean4export\"\n\
         path = \"{LEAN4EXPORT_IN_IMAGE}\"\n"
    );
    let lake_manifest = format!(
        "{{\"version\": \"1.1.0\",\n \
         \"packagesDir\": \".lake/packages\",\n \
         \"packages\":\n \
         [{{\"type\": \"path\",\n   \
         \"scope\": \"\",\n   \
         \"name\": \"lean4export\",\n   \
         \"manifestFile\": \"lake-manifest.json\",\n   \
         \"inherited\": false,\n   \
         \"dir\": \"{LEAN4EXPORT_IN_IMAGE}\",\n   \
         \"configFile\": \"lakefile.toml\"}}],\n \
         \"name\": \"TestProject\",\n \
         \"lakeDir\": \".lake\"}}"
    );
    let source_tree = serde_json::json!([
        {
            "path": "TestProject.lean",
            "content_base64": base64::engine::general_purpose::STANDARD
                .encode("import TestProject.Foo\n"),
        },
        {
            "path": "TestProject/Foo.lean",
            "content_base64": base64::engine::general_purpose::STANDARD
                .encode(target_theorem_source),
        },
    ]);

    let mut r = Resource::new(iri("urn:eigenius:test:lean_project_e2e"));
    r.set(
        iri(PROP_IS_A),
        Value::Array(vec![Value::ResourceRef(iri(LEAN_PROJECT_IRI))]),
    );
    r.set(iri(PROP_LAKEFILE), Value::String(lakefile));
    r.set(iri(PROP_LAKE_MANIFEST), Value::String(lake_manifest));
    r.set(iri(PROP_SOURCE_TREE), Value::Json(source_tree));
    eigon_cbor::serialize_resource(&r)
}

/// Build a `RuntimeMethodSignature` resource pointing at a Lean
/// worker function by name. v1 just needs `language` + `method_name`
/// — input/output typing is validated by the kernel boundary check,
/// which is not in scope here.
fn build_method_signature(method_name: &str) -> Resource {
    let mut sig = Resource::new(iri(&format!(
        "urn:eigenius:test:method-signature:lean:{method_name}"
    )));
    sig.set(iri(PROP_LANGUAGE), Value::String("lean".to_string()));
    sig.set(
        iri(PROP_METHOD_NAME),
        Value::String(method_name.to_string()),
    );
    sig
}

// ─── The actual e2e test ───────────────────────────────────────────

/// End-to-end: build the Lean env image with the substrate, dispatch
/// `lean_export` against a single-theorem `LeanProject`, verify the
/// returned bytes are valid lean4export ndjson. This is the
/// production target for Phase 20a.5b.4 — every layer (image build,
/// worker boot, lake build, lean4export invocation, CBOR-framed RPC)
/// exercises a real implementation, not a stub.
#[ignore = "heavy E2E: full Lean env image build (cold: 5-15 min)"]
#[test]
fn lean_env_image_dispatches_lean_export_round_trip() {
    if let Some(reason) = skip_unless_full_environment() {
        eprintln!("skipping 20a.5b.4 Lean env-image e2e: {reason}");
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
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("skipping (DockerServiceSpawner construction failed): {e}");
            return;
        }
    };

    let runtime = Arc::new(LeanLanguageRuntime::new(
        lean_project_dir(),
        cdylib_path(),
        lean_common_dir(),
        pinned_base,
        spawner.clone(),
        depot.clone(),
    ));

    let env = Resource::new_embedded();
    let runtime_for_dispatch: Box<dyn LanguageRuntime> = Box::new(runtime.clone());
    let digest = match runtime_for_dispatch.build_environment_image(&env, &[], None) {
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

    // Build inputs for `lean_export`. Every input ships as an
    // Eigon-CBOR Resource — the cross-runtime wire format the
    // substrate uses for typed Resources. The Lake worker decodes
    // each one via its cdylib's `decodeEigonStringProperty` (which
    // hosts the workspace Eigon-CBOR codec) to read the relevant
    // string property out of inputs 1 and 2.
    let project_cbor = make_lean_project_cbor("theorem foo : True := True.intro\n");
    let project = eigon_cbor::parse_resource_lenient(&project_cbor)
        .expect("LeanProject must round-trip through parse_resource_lenient");
    let target_module = build_target_module("TestProject.Foo");
    let target_constant = build_target_constant("foo");
    let signature = build_method_signature("lean_export");

    let outcome = match runtime_for_dispatch.call_method(
        &env,
        &signature,
        &[project, target_module, target_constant],
    ) {
        Ok(o) => o,
        Err(e) => {
            let _ = runtime.drain();
            let _ = std::fs::remove_dir_all(&depot);
            panic!("call_method(lean_export): {e:?}");
        }
    };

    // The worker returns ndjson bytes; the runtime wraps them as a
    // `script_output` string on the output Resource.
    let output_text = outcome
        .output
        .get(&iri(PROP_SCRIPT_OUTPUT))
        .and_then(Value::as_str)
        .expect("output Resource must carry script_output");
    assert!(
        !output_text.is_empty(),
        "lean_export must produce non-empty ndjson"
    );
    assert!(
        output_text.starts_with("{\"meta\":"),
        "lean_export ndjson must begin with the metadata line; got prefix: `{}`",
        &output_text[..output_text.len().min(80)]
    );
    // The constant we pinned must appear in the dumped environment —
    // sanity check that we exported the right thing, not just *some*
    // bytes.
    assert!(
        output_text.contains("\"foo\""),
        "lean_export ndjson must mention the pinned constant `foo`"
    );

    let _ = runtime.drain();
    let _ = std::fs::remove_dir_all(&depot);
}

// ─── Mirror-baking Docker e2e (Phase 20a.6.x) ──────────────────────

/// Minimal in-memory chain for the mirror-baking test. Carries a
/// single class declaration so `LeanMirrorGenerator` has something
/// to mirror.
struct MirrorTestChain {
    resources: std::collections::HashMap<Iri, Resource>,
}

impl MirrorTestChain {
    fn for_patient() -> Self {
        let mut resources = std::collections::HashMap::new();
        let class_iri = iri("urn:eigenius:test:image_mirror:Patient");
        let mut cls = Resource::new(class_iri.clone());
        cls.set(
            iri("urn:eigenius:core:short_name"),
            Value::String("Patient".into()),
        );
        cls.set(
            iri("urn:eigenius:core:requires"),
            Value::Array(vec![Value::ResourceRef(iri(
                "urn:eigenius:test:image_mirror:weight",
            ))]),
        );
        resources.insert(class_iri, cls);

        let prop_iri = iri("urn:eigenius:test:image_mirror:weight");
        let mut prop = Resource::new(prop_iri.clone());
        prop.set(
            iri("urn:eigenius:core:short_name"),
            Value::String("weight".into()),
        );
        prop.set(
            iri("urn:eigenius:core:data_type"),
            Value::ResourceRef(iri("urn:eigenius:core:float")),
        );
        resources.insert(prop_iri, prop);
        Self { resources }
    }
}

impl ChainAccessor for MirrorTestChain {
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

/// Inspect the built image's filesystem layer to confirm
/// `install_mirror` produced compiled `.olean` files. `docker run
/// --rm <image> sh -c "ls <path>"` is the cheapest way to ask
/// without standing up a worker.
fn list_image_path(image_tag: &str, path: &str) -> Result<String, String> {
    let out = std::process::Command::new("docker")
        .args(["run", "--rm", "--entrypoint", "ls", image_tag, "-1", path])
        .output()
        .map_err(|e| format!("docker run failed: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "docker run exited {}: stdout={}, stderr={}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// End-to-end: build an env image with a mirror baked in, assert
/// the install_mirror step rewrote the lakefile + lake-built the
/// mirror to .olean files. Closes the Phase 20a.6.x production
/// gap — chain-committed mirrors are usable inside the image
/// without dispatch-time compilation.
#[ignore = "heavy E2E: full Lean env image build with mirror baked in"]
#[test]
fn lean_env_image_with_baked_mirror_lake_builds_the_mirror_in_image() {
    if let Some(reason) = skip_unless_full_environment() {
        eprintln!("skipping 20a.6.x mirror-bake e2e: {reason}");
        return;
    }
    let pinned_base = match ensure_base_image_pinned() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("skipping (could not pin base image): {e}");
            return;
        }
    };

    // Generate a mirror for a synthetic Patient class via the real
    // LeanMirrorGenerator. The mirror's lakefile carries the
    // chain-committed git-require for EigeniusLeanCommon; the
    // install_mirror step rewrites that to a path-require pointing
    // at the baked copy at LEAN_COMMON_IN_IMAGE.
    let chain = MirrorTestChain::for_patient();
    let layer_iri = iri("urn:eigenius:test:image_mirror:layer");
    let seed = vec![iri("urn:eigenius:test:image_mirror:Patient")];
    let generator = LeanMirrorGenerator::new();
    let mirror_output = generator
        .generate(&MirrorGenerationRequest {
            source_layer: &layer_iri,
            seed_classes: &seed,
            chain: &chain,
        })
        .expect("mirror generation");
    let mirror_resource = mirror_to_resource(&generator, &mirror_output, &layer_iri, None);

    let depot = fresh_depot("e2e-mirror");
    let spawner = match eigenius_runtime_substrate::spawner::service::DockerServiceSpawner::new(
        eigenius_runtime_substrate::spawner::DockerSpawnerConfig::new(depot.clone()),
    ) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("skipping (DockerServiceSpawner construction failed): {e}");
            return;
        }
    };

    let runtime = Arc::new(LeanLanguageRuntime::new(
        lean_project_dir(),
        cdylib_path(),
        lean_common_dir(),
        pinned_base,
        spawner.clone(),
        depot.clone(),
    ));

    let env = Resource::new_embedded();
    let runtime_for_build: Box<dyn eigenius_runtime_substrate::language_runtime::LanguageRuntime> =
        Box::new(runtime.clone());
    let digest = match runtime_for_build.build_environment_image(&env, &[], Some(&mirror_resource))
    {
        Ok(d) => d,
        Err(e) => {
            let _ = runtime.drain();
            let _ = std::fs::remove_dir_all(&depot);
            panic!("build_environment_image (with mirror): {e}");
        }
    };
    assert!(digest.as_str().starts_with("sha256:"));

    // Confirm the lake-build step landed compiled output where the
    // worker can find it. Two assertions:
    //
    //   (a) `EigeniusLeanCommon.olean` lives under the baked
    //       EigeniusLeanCommon dir — proves the lean-common stage
    //       wasn't dropped + lake compiled it as a transitive dep
    //       during the mirror build.
    //
    //   (b) The mirror's own `.olean` files (EigeniusFFI.Basic +
    //       EigeniusFFI.Mirror) live under the mirror's
    //       `.lake/build/lib/lean/EigeniusFFI/` — proves the sed
    //       rewrite worked and lake produced output.
    //
    // Both checks shell out to `docker run --rm <image> ls <path>`
    // which is the cheapest "did the in-image build succeed?" probe.
    let image_tag = "eigenius-lean-dockeriolibrarydebiansha:latest";
    let common_oleans = list_image_path(
        image_tag,
        "/opt/eigenius/lean-common/EigeniusLeanCommon/.lake/build/lib/lean",
    )
    .expect("list common .oleans");
    assert!(
        common_oleans.contains("EigeniusLeanCommon.olean"),
        "EigeniusLeanCommon must lake-build during install_mirror; got listing:\n{common_oleans}"
    );

    let mirror_oleans = list_image_path(
        image_tag,
        "/opt/eigenius/mirror/.lake/build/lib/lean/EigeniusFFI",
    )
    .expect("list mirror .oleans");
    assert!(
        mirror_oleans.contains("Basic.olean"),
        "EigeniusFFI.Basic must lake-build; got listing:\n{mirror_oleans}"
    );
    assert!(
        mirror_oleans.contains("Mirror.olean"),
        "EigeniusFFI.Mirror must lake-build; got listing:\n{mirror_oleans}"
    );

    let _ = runtime.drain();
    let _ = std::fs::remove_dir_all(&depot);
}
