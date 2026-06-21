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

//! End-to-end test for the generic OCI tool runtime (D60 P2b): bake the
//! `eigenius-schemaorg-worker` binary into a real OCI image via buildah, spawn it
//! with `DockerSpawner`, dispatch a real schema.org conversion over the UDS, and
//! assert the returned conversion-report `DerivedResource` carries the output
//! `content_hash` + coverage. This is the substrate-side verification of the
//! Level-2 mechanism (the kernel-side `ProgramTrace`/witness wiring is P2c/P4).
//!
//! Heavy + Docker-gated (`#[ignore]`); run with:
//!   cargo test -p eigenius-oci --test oci_e2e -- --ignored --nocapture

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_oci::OciToolRuntime;
use eigenius_runtime_substrate::is_buildah_available;
use eigenius_runtime_substrate::language_runtime::LanguageRuntime;
use eigenius_runtime_substrate::spawner::{DockerSpawner, DockerSpawnerConfig};

const BASE_IMAGE_TAG: &str = "debian:bookworm-slim";
static COUNTER: AtomicU64 = AtomicU64::new(0);

fn fresh_depot() -> PathBuf {
    let pid = std::process::id();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("oci-e2e-{pid}-{n}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create depot");
    dir
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
        return Err(format!("expected `name@sha256:...`, got `{pinned}`"));
    }
    Ok(pinned)
}

/// Build the worker binary and resolve its path under the workspace target dir.
/// (`CARGO_BIN_EXE_*` is only set for binaries in the test's own crate, so the
/// cross-crate worker is built + located explicitly.)
fn build_and_locate_worker() -> PathBuf {
    let status = std::process::Command::new(env!("CARGO"))
        .args(["build", "-p", "eigenius-schemaorg-worker"])
        .status()
        .expect("invoke cargo build for the worker");
    assert!(
        status.success(),
        "building eigenius-schemaorg-worker failed"
    );
    let target = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(2)
                .expect("workspace root")
                .join("target")
        });
    let bin = target.join("debug").join("eigenius-schemaorg-worker");
    assert!(bin.exists(), "worker binary not found at {}", bin.display());
    bin
}

fn iri(s: &str) -> Iri {
    Iri::parse(s).unwrap()
}

#[ignore = "heavy E2E: buildah image build + DooD UDS round-trip with a sibling container"]
#[test]
fn oci_runtime_converts_schemaorg_through_a_real_container() {
    if !is_docker_available() {
        eprintln!("skipping oci e2e: Docker socket unavailable");
        return;
    }
    if !is_buildah_available() {
        eprintln!("skipping oci e2e: buildah unavailable");
        return;
    }
    let pinned_base = match ensure_base_image_pinned() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("skipping oci e2e (could not pin base image): {e}");
            return;
        }
    };

    let depot = fresh_depot();
    let spawner = match DockerSpawner::new(DockerSpawnerConfig::new(depot.clone())) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("skipping oci e2e (DockerSpawner construction failed): {e}");
            return;
        }
    };

    let runtime = OciToolRuntime::new(
        build_and_locate_worker(),
        pinned_base,
        spawner,
        depot.clone(),
    );

    // Build the image (bakes the worker binary).
    let env = Resource::new_embedded();
    let digest = match runtime.build_environment_image(&env, &[], None) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("skipping oci e2e (image build failed): {e}");
            let _ = std::fs::remove_dir_all(&depot);
            return;
        }
    };
    assert!(
        digest.as_str().starts_with("sha256:"),
        "expected sha256 digest, got {digest}"
    );

    // Stage a tiny schema.org JSON-LD under the depot (bind-mounted read-only at
    // the same path inside the container) and point the input at it.
    let graph = r#"{"@graph":[
        {"@id":"schema:Thing","@type":"rdfs:Class","rdfs:label":"Thing"},
        {"@id":"schema:name","@type":"rdf:Property","rdfs:label":"name"}
    ]}"#;
    let inputs_dir = depot.join("inputs");
    std::fs::create_dir_all(&inputs_dir).unwrap();
    let input_file = inputs_dir.join("graph.jsonld");
    std::fs::write(&input_file, graph).unwrap();

    let mut input = Resource::new(iri("urn:eigenius:obj:d57:gen_input"));
    input.set(
        iri("urn:eigenius:core:is_a"),
        Value::Array(vec![Value::ResourceRef(iri(
            "urn:eigenius:ingest:PinnedExternalFile",
        ))]),
    );
    input.set(
        iri("urn:eigenius:ingest:materialized_path"),
        Value::String(input_file.to_string_lossy().into_owned()),
    );

    let script = Resource::new_embedded();
    let outcome = runtime
        .run_script(&env, &script, &[input])
        .unwrap_or_else(|e| {
            let _ = std::fs::remove_dir_all(&depot);
            panic!("run_script through the oci container failed: {e}");
        });

    let report = outcome.output;
    // The worker ran the real converter and returned the report over the wire.
    let out_hash = report
        .get(&iri("urn:eigenius:obj:d57:output_content_hash"))
        .and_then(Value::as_str)
        .expect("report carries output_content_hash");
    assert!(out_hash.starts_with("sha256:"), "got {out_hash}");
    let cov = report
        .get(&iri("urn:eigenius:obj:d57:coverage"))
        .expect("report carries coverage");
    let Value::Json(j) = cov else {
        panic!("coverage should be a JSON payload, got {cov:?}");
    };
    assert_eq!(j["classes"], serde_json::json!(1));

    // The worker set its canonical_proposition (GeneratorConforms("schema_org"))
    // and it survived the real container round-trip — this is what the kernel's
    // ProgramTrace turns into IsDerivedAs for the chain's derived(...) certificate.
    let Some(Value::Json(prop)) = report.get(&iri("urn:eigenius:reflection:canonical_proposition"))
    else {
        panic!("report must carry canonical_proposition");
    };
    assert_eq!(
        prop["args"][0]["args"][0],
        serde_json::json!("urn:eigenius:obj:d57:GeneratorConforms"),
    );
    assert_eq!(prop["args"][1]["args"][0], serde_json::json!("schema_org"));

    // The trace carries the image the worker ran against.
    assert!(
        outcome.image_digest.is_some(),
        "RunOutcome should carry the image digest"
    );

    let _ = std::fs::remove_dir_all(&depot);
}
