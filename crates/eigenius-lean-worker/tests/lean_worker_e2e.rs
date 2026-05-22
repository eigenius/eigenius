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
//
//! Phase 20a.5b end-to-end tests: spawn the Lake-built Lean worker,
//! drive it through the substrate protocol, and (in the real
//! `lean_export` case) round-trip the worker's output bytes
//! through nanoda's `check_proof` to confirm verdict consistency.
//!
//! All tests are `#[ignore]`'d by default since they need:
//! - `lean-runtime-worker` Lake binary built (`lake build` under
//!   `lean/runtime-worker/`)
//! - the local Lean toolchain on `PATH` (`elan` shims)
//! - the vendored `lean4export` Lake project's `.lake/build/bin/`
//!   populated (the test lazily runs `lake build` against the
//!   vendor on first invocation)
//!
//! Run via:
//!
//! ```text
//! cargo test -p eigenius-lean-worker --test lean_worker_e2e -- --ignored
//! ```

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_lean::{check_proof, Verdict};
use eigenius_runtime_substrate::rpc::client::WorkerRpcClient;
use eigenius_runtime_substrate::rpc::method::MethodInvocation;
use eigenius_runtime_substrate::rpc::protocol::{Request, Response, TargetKind};

// ---------------------------------------------------------------------------
// Path discovery + helpers
// ---------------------------------------------------------------------------

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent() // crates/
        .unwrap()
        .parent() // workspace root
        .unwrap()
        .to_path_buf()
}

fn locate_worker_binary() -> Option<PathBuf> {
    let p = workspace_root()
        .join("lean")
        .join("runtime-worker")
        .join(".lake")
        .join("build")
        .join("bin")
        .join("lean-runtime-worker");
    p.exists().then_some(p)
}

fn vendored_lean4export_path() -> PathBuf {
    workspace_root()
        .join("lean")
        .join("runtime-worker")
        .join("vendor")
        .join("lean4export")
}

/// Lazily build the vendored lean4export so the test's
/// `lake exe lean4export` invocation finds a cached binary.
/// Idempotent — `lake build` is a no-op when the artifacts are
/// already current.
///
/// Returns `Ok(())` only when the binary is verified at
/// `vendor/lean4export/.lake/build/bin/lean4export`. Any earlier
/// failure (lake not on PATH, missing vendor dir) propagates as an
/// `Err` so the caller can skip the test cleanly.
fn ensure_lean4export_built() -> Result<(), String> {
    let vendor = vendored_lean4export_path();
    if !vendor.exists() {
        return Err(format!(
            "vendored lean4export not found at {}",
            vendor.display()
        ));
    }
    let binary = vendor
        .join(".lake")
        .join("build")
        .join("bin")
        .join("lean4export");
    if binary.exists() {
        return Ok(());
    }
    let output = Command::new("lake")
        .arg("build")
        .current_dir(&vendor)
        .output()
        .map_err(|e| format!("`lake build` (vendor) failed to spawn: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "`lake build` (vendor) exited {:?}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    if !binary.exists() {
        return Err(format!(
            "lake build succeeded but lean4export binary still missing at {}",
            binary.display()
        ));
    }
    Ok(())
}

fn unique_uds_path() -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "eigenius-lean-worker-e2e-{}-{}.sock",
        std::process::id(),
        n
    ));
    path
}

fn spawn_worker(binary: &PathBuf, uds_path: &PathBuf) -> Child {
    Command::new(binary)
        .arg(uds_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::inherit())
        // Inherit stderr so the worker's `IO.eprintln` diagnostics
        // surface in `cargo test --nocapture` output — invaluable
        // when the worker dies unexpectedly during dispatch.
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .expect("spawn lean-runtime-worker")
}

fn connect_with_retry(path: &std::path::Path) -> UnixStream {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match UnixStream::connect(path) {
            Ok(s) => return s,
            Err(e) if Instant::now() < deadline => {
                if e.kind() != std::io::ErrorKind::NotFound
                    && e.kind() != std::io::ErrorKind::ConnectionRefused
                {
                    panic!("unexpected connect error: {e}");
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(e) => panic!("failed to connect to worker UDS within timeout: {e}"),
        }
    }
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(status)) => return status,
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(e) => panic!("try_wait failed: {e}"),
        }
    }
    let _ = child.kill();
    child.wait().expect("wait after kill")
}

// ---------------------------------------------------------------------------
// LeanProject CBOR construction
// ---------------------------------------------------------------------------

const LEAN_PROJECT_IRI: &str = "urn:eigenius:lean:LeanProject";
const PROP_IS_A: &str = "urn:eigenius:core:is_a";
const PROP_LAKEFILE: &str = "urn:eigenius:lean:lakefile";
const PROP_LAKE_MANIFEST: &str = "urn:eigenius:lean:lake_manifest";
const PROP_SOURCE_TREE: &str = "urn:eigenius:runtime:source_tree";

/// Build a minimal `LeanProject` Eigon-CBOR resource carrying a
/// single-theorem Lean project. The lakefile depends on the
/// vendored `lean4export` via an absolute path; the
/// `lake-manifest.json` matches what `lake update` produces for
/// the same shape.
fn make_lean_project_cbor(target_theorem_source: &str) -> Vec<u8> {
    let vendor = vendored_lean4export_path();
    let vendor_str = vendor
        .to_str()
        .expect("vendor path must be UTF-8")
        .to_string();

    let lakefile = format!(
        "name = \"TestProject\"\n\
         defaultTargets = [\"TestProject\"]\n\
         \n\
         [[lean_lib]]\n\
         name = \"TestProject\"\n\
         \n\
         [[require]]\n\
         name = \"lean4export\"\n\
         path = \"{vendor_str}\"\n"
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
         \"dir\": \"{vendor_str}\",\n   \
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

    let mut r = Resource::new(Iri::parse("urn:eigenius:test:lean_project_1").unwrap());
    r.set(
        Iri::parse(PROP_IS_A).unwrap(),
        Value::Array(vec![Value::ResourceRef(
            Iri::parse(LEAN_PROJECT_IRI).unwrap(),
        )]),
    );
    r.set(Iri::parse(PROP_LAKEFILE).unwrap(), Value::String(lakefile));
    r.set(
        Iri::parse(PROP_LAKE_MANIFEST).unwrap(),
        Value::String(lake_manifest),
    );
    r.set(
        Iri::parse(PROP_SOURCE_TREE).unwrap(),
        Value::Json(source_tree),
    );
    eigon_cbor::serialize_resource(&r)
}

fn encode_method_invocation(function_name: &str) -> serde_bytes::ByteBuf {
    let mi = MethodInvocation {
        function_name: function_name.to_string(),
        signature_iri: format!("urn:eigenius:test:lean:methods:{function_name}"),
    };
    let mut buf = Vec::new();
    ciborium::into_writer(&mi, &mut buf).expect("encode");
    serde_bytes::ByteBuf::from(buf)
}

/// Build an Eigon-CBOR-encoded embedded Resource carrying a single
/// string property — the wire shape the Lake worker's
/// `decodeEigonStringProperty` reads for `lean_export`'s
/// `target_module` / `target_constant` inputs.
fn encode_string_property_resource(property_iri: &str, value: &str) -> Vec<u8> {
    let mut r = Resource::new_embedded();
    r.set(
        Iri::parse(property_iri).expect("static property IRI"),
        Value::String(value.to_string()),
    );
    eigon_cbor::serialize_resource(&r)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
#[ignore = "requires lake-built worker binary; run with --ignored after `lake build`"]
fn lean_worker_round_trips_health_evict() {
    let binary = match locate_worker_binary() {
        Some(p) => p,
        None => {
            eprintln!(
                "Lake-built worker binary not found — skipping. \
                 Run `(cd lean/runtime-worker && lake build)` first."
            );
            return;
        }
    };
    let uds_path = unique_uds_path();
    let mut child = spawn_worker(&binary, &uds_path);

    let stream = connect_with_retry(&uds_path);
    let mut client = WorkerRpcClient::new(stream);

    let health = client.call(&Request::Health).expect("health");
    assert!(matches!(health, Response::Health(_)));

    let evicted = client.call(&Request::Evict).expect("evict");
    assert!(matches!(evicted, Response::Evicted));
    drop(client);

    let status = wait_for_exit(&mut child, Duration::from_secs(5));
    assert!(status.success(), "worker should exit cleanly: {status:?}");
    let _ = std::fs::remove_file(&uds_path);
}

#[test]
#[ignore = "requires lake-built worker binary + vendored lean4export; run with --ignored"]
fn lean_worker_real_lean_export_round_trips_through_check_proof() {
    let binary = match locate_worker_binary() {
        Some(p) => p,
        None => {
            eprintln!(
                "Lake-built worker binary not found — skipping. \
                 Run `(cd lean/runtime-worker && lake build)` first."
            );
            return;
        }
    };
    if let Err(e) = ensure_lean4export_built() {
        eprintln!("lean4export pre-build failed: {e} — skipping");
        return;
    }

    let uds_path = unique_uds_path();
    let mut child = spawn_worker(&binary, &uds_path);
    let stream = connect_with_retry(&uds_path);
    let mut client = WorkerRpcClient::new(stream);

    // Build a real LeanProject with `theorem foo : True := True.intro`
    // and dispatch lean_export against `TestProject.Foo` (the module)
    // pinned to constant `foo`. The worker stages files, runs `lake
    // build` + `lake exe lean4export TestProject.Foo -- foo`, returns
    // the export bytes (≈1.6 KB; without the pinned constant
    // lean4export dumps the entire transitive env at ≈324 MB).
    let project_cbor = make_lean_project_cbor("theorem foo : True := True.intro\n");
    // Each input is an Eigon-CBOR Resource (the wire format every
    // `call_method` input takes). The Lake worker decodes each one
    // via its cdylib's `decodeEigonStringProperty` to read the
    // `module_name` / `constant_name` property out of inputs 1 and 2.
    let target_module =
        encode_string_property_resource("urn:eigenius:lean:module_name", "TestProject.Foo");
    let target_constant = encode_string_property_resource("urn:eigenius:lean:constant_name", "foo");

    let dispatch = client
        .call(&Request::DispatchMethod {
            invocation_id: "e2e-export-1".to_string(),
            target_kind: TargetKind::Method,
            target: encode_method_invocation("lean_export"),
            inputs: vec![
                serde_bytes::ByteBuf::from(project_cbor),
                serde_bytes::ByteBuf::from(target_module),
                serde_bytes::ByteBuf::from(target_constant),
            ],
        })
        .expect("dispatch");

    let export_bytes = match dispatch {
        Response::DispatchOk { output, .. } => output.into_vec(),
        Response::DispatchFailed {
            error_kind,
            message,
            ..
        } => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("worker failed lean_export: {error_kind}: {message}");
        }
        other => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("unexpected response: {other:?}");
        }
    };

    assert!(
        !export_bytes.is_empty(),
        "lean_export should produce non-empty bytes"
    );
    assert!(
        std::str::from_utf8(&export_bytes)
            .ok()
            .map(|s| s.starts_with("{\"meta\":"))
            .unwrap_or(false),
        "lean_export output should start with the lean4export metadata line"
    );

    // Round-trip: feed the exported bytes to nanoda's check_proof
    // with target "foo" and assert Holds. This is the real
    // verification proof the milestone is named for.
    let verdict = check_proof(&export_bytes, "foo", &[]).expect("check_proof infrastructure");
    match verdict {
        Verdict::Holds => {}
        Verdict::Fails { diagnostic } => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("nanoda rejected the exported proof: {diagnostic}");
        }
    }

    let _ = client.call(&Request::Evict);
    drop(client);
    let _ = wait_for_exit(&mut child, Duration::from_secs(5));
    let _ = std::fs::remove_file(&uds_path);
}

#[test]
#[ignore = "requires lake-built worker binary; run with --ignored after `lake build`"]
fn lean_worker_lean_export_with_no_inputs_fails_cleanly() {
    // The Lean handler requires `inputs = [LeanProject, target_module]`.
    // Without inputs, the dispatch should fail with a descriptive
    // diagnostic — not crash, not stall.
    let binary = match locate_worker_binary() {
        Some(p) => p,
        None => return,
    };
    let uds_path = unique_uds_path();
    let mut child = spawn_worker(&binary, &uds_path);
    let stream = connect_with_retry(&uds_path);
    let mut client = WorkerRpcClient::new(stream);

    let resp = client
        .call(&Request::DispatchMethod {
            invocation_id: "e2e-export-no-inputs".to_string(),
            target_kind: TargetKind::Method,
            target: encode_method_invocation("lean_export"),
            inputs: vec![],
        })
        .expect("dispatch");

    match resp {
        Response::DispatchFailed {
            error_kind,
            message,
            ..
        } => {
            assert_eq!(error_kind, "lean_export_failed");
            assert!(
                message.contains("requires inputs"),
                "expected the missing-inputs diagnostic, got: {message}"
            );
        }
        other => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("expected DispatchFailed, got {other:?}");
        }
    }

    let _ = client.call(&Request::Evict);
    drop(client);
    let _ = wait_for_exit(&mut child, Duration::from_secs(5));
    let _ = std::fs::remove_file(&uds_path);
}

#[test]
#[ignore = "requires lake-built worker binary; run with --ignored after `lake build`"]
fn lean_worker_unknown_function_routes_to_dispatch_failed() {
    let binary = match locate_worker_binary() {
        Some(p) => p,
        None => return,
    };
    let uds_path = unique_uds_path();
    let mut child = spawn_worker(&binary, &uds_path);
    let stream = connect_with_retry(&uds_path);
    let mut client = WorkerRpcClient::new(stream);

    let resp = client
        .call(&Request::DispatchMethod {
            invocation_id: "e2e-unknown".to_string(),
            target_kind: TargetKind::Method,
            target: encode_method_invocation("compute_some_user_thing"),
            inputs: vec![],
        })
        .expect("dispatch");
    match resp {
        Response::DispatchFailed {
            error_kind,
            message,
            ..
        } => {
            assert_eq!(error_kind, "not_implemented");
            assert!(message.contains("compute_some_user_thing"));
        }
        other => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("expected DispatchFailed, got {other:?}");
        }
    }
    let _ = client.call(&Request::Evict);
    drop(client);
    let _ = wait_for_exit(&mut child, Duration::from_secs(5));
    let _ = std::fs::remove_file(&uds_path);
}
