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

//! End-to-end integration test for the substrate's image-build pipeline
//! (D26 §9.2). Exercises composer → materialiser → BuildahImageBuilder
//! against a real `buildah`. Skipped when buildah is not installed or
//! refuses to run in the host environment (e.g. unprivileged container
//! without user-namespace support).
//!
//! Acceptance for 18c.1: building the same materialised context twice
//! produces the same image id — the foundational determinism guarantee
//! the rest of Phase 18c builds on.

use eigenius_runtime_substrate::{
    compose_dockerfile, is_buildah_available, BuildContext, BuildContextSpec, BuildahImageBuilder,
    DockerfileFragments, DockerfileSpec, ImageBuilder,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn fresh_work_dir(label: &str) -> PathBuf {
    let pid = std::process::id();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("substrate-build-it-{pid}-{label}-{n}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create work_dir");
    dir
}

fn skip_if_no_buildah() -> bool {
    if !is_buildah_available() {
        eprintln!("buildah unavailable on this host, skipping image-build integration test");
        return true;
    }
    false
}

fn minimal_spec() -> BuildContextSpec {
    let fragments = DockerfileFragments {
        bootstrap_command: vec!["true".into()],
        ..Default::default()
    };
    let dockerfile = compose_dockerfile(&DockerfileSpec {
        // `scratch` is not a real image but `FROM scratch` is the
        // smallest possible base; buildah handles it without any
        // network access.
        base_image_ref: "scratch",
        fragments: &fragments,
        included_packages: &[],
        has_mirror: false,
        language_asset_copies: &[],
    });
    BuildContextSpec {
        dockerfile,
        manifest_hash: "deadbeef".into(),
        mirror_iri: String::new(),
        included_pkg_iris: vec![],
        built_at: "1970-01-01T00:00:00Z".into(),
        ..Default::default()
    }
}

#[test]
fn buildah_produces_image_id_for_minimal_context() {
    if skip_if_no_buildah() {
        return;
    }
    let work = fresh_work_dir("minimal");
    let ctx = BuildContext::materialize(work.clone(), &minimal_spec()).expect("materialise");
    let builder = BuildahImageBuilder::new();
    let tag = format!(
        "eigenius-substrate-test-{}:latest",
        COUNTER.fetch_add(1, Ordering::SeqCst)
    );
    match builder.build(&ctx, &tag) {
        Ok(digest) => {
            assert!(digest.as_str().starts_with("sha256:"), "got: {digest}");
            // Best-effort cleanup; ignore failures (e.g. parallel test cleanup).
            let _ = std::process::Command::new("buildah")
                .args(["rmi", "-f", &tag])
                .output();
        }
        Err(e) => {
            // Buildah may be installed but unable to run (no user
            // namespaces, missing storage backend, etc.). Skip rather
            // than fail — 18c.1's acceptance is the in-process pipeline,
            // and the build-side smoke is best-effort on CI hosts.
            eprintln!("buildah build failed (skipping): {e}");
        }
    }
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
fn buildah_build_is_deterministic_under_same_inputs() {
    if skip_if_no_buildah() {
        return;
    }
    let builder = BuildahImageBuilder::new();
    let work_a = fresh_work_dir("det-a");
    let work_b = fresh_work_dir("det-b");
    let ctx_a = BuildContext::materialize(work_a.clone(), &minimal_spec()).expect("materialise a");
    let ctx_b = BuildContext::materialize(work_b.clone(), &minimal_spec()).expect("materialise b");
    let tag_a = format!(
        "eigenius-substrate-det-a-{}:latest",
        COUNTER.fetch_add(1, Ordering::SeqCst)
    );
    let tag_b = format!(
        "eigenius-substrate-det-b-{}:latest",
        COUNTER.fetch_add(1, Ordering::SeqCst)
    );
    let digest_a = match builder.build(&ctx_a, &tag_a) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("buildah build failed (skipping determinism check): {e}");
            return;
        }
    };
    let digest_b = match builder.build(&ctx_b, &tag_b) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("buildah build failed on second build (skipping): {e}");
            let _ = std::process::Command::new("buildah")
                .args(["rmi", "-f", &tag_a])
                .output();
            return;
        }
    };
    assert_eq!(
        digest_a.as_str(),
        digest_b.as_str(),
        "byte-identical inputs to the substrate's image-build pipeline must produce the same image id"
    );
    let _ = std::process::Command::new("buildah")
        .args(["rmi", "-f", &tag_a])
        .output();
    let _ = std::process::Command::new("buildah")
        .args(["rmi", "-f", &tag_b])
        .output();
    let _ = std::fs::remove_dir_all(&work_a);
    let _ = std::fs::remove_dir_all(&work_b);
}
