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

//! Integration tests for the substrate dispatch facade — the entry
//! point the orchestrator's napi addon calls. Drives the full
//! Eigon-CBOR → Resource → LanguageRuntime → Resource → Eigon-CBOR
//! path through the bash-c test runtime.

#![cfg(feature = "test-runtime")]

use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_runtime_substrate::error::RunError;
use eigenius_runtime_substrate::facade::{FacadeError, SubstrateDispatcher};
use eigenius_runtime_substrate::test_runtime::TestLanguageRuntime;
use std::path::PathBuf;

fn worker_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_eigenius-test-worker"))
}

fn dispatcher_with_test_runtime() -> SubstrateDispatcher {
    let mut d = SubstrateDispatcher::new();
    d.register_language_runtime(Box::new(TestLanguageRuntime::with_worker_binary(
        worker_binary(),
    )))
    .expect("register test runtime");
    d
}

fn build_argument(language: &str, source: &str) -> Vec<u8> {
    let mut arg = Resource::new_embedded();
    arg.set(
        Iri::parse("urn:eigenius:runtime:language").unwrap(),
        Value::String(language.to_string()),
    );
    arg.set(
        Iri::parse("urn:eigenius:runtime:source").unwrap(),
        Value::String(source.to_string()),
    );
    eigon_cbor::serialize_resource(&arg)
}

#[test]
fn run_runtime_script_via_facade_round_trips_through_test_worker() {
    let d = dispatcher_with_test_runtime();
    let argument = build_argument("test", "echo facade-validated");
    let outcome = d
        .dispatch_run_runtime_script(&[], &argument)
        .expect("dispatch");
    let output = eigon_cbor::parse_resource_lenient(&outcome.output_cbor).expect("decode output");
    let stdout = output
        .get(&Iri::parse("urn:eigenius:test:bash_stdout").unwrap())
        .and_then(Value::as_str)
        .expect("bash_stdout property on output");
    assert_eq!(stdout.trim(), "facade-validated");

    // Phase 18c.5: the partial RuntimeInvocation carries the trace
    // fields the substrate observed during the dispatch.
    let inv = eigon_cbor::parse_resource_lenient(&outcome.partial_invocation_cbor)
        .expect("decode partial invocation");
    assert_eq!(
        inv.get(&Iri::parse("urn:eigenius:runtime:language").unwrap())
            .and_then(Value::as_str),
        Some("test")
    );
    let started = inv
        .get(&Iri::parse("urn:eigenius:runtime:started_at").unwrap())
        .and_then(Value::as_str)
        .expect("started_at present");
    let completed = inv
        .get(&Iri::parse("urn:eigenius:runtime:completed_at").unwrap())
        .and_then(Value::as_str)
        .expect("completed_at present");
    assert!(
        started <= completed,
        "started {started} <= completed {completed}"
    );
    let metadata = inv
        .get(&Iri::parse("urn:eigenius:runtime:numerical_metadata").unwrap())
        .expect("numerical_metadata present");
    match metadata {
        Value::Json(json) => {
            // The bash test worker reports `host_kernel = "test-runtime"`.
            assert_eq!(
                json.get("host_kernel").and_then(serde_json::Value::as_str),
                Some("test-runtime"),
            );
        }
        other => panic!("expected Value::Json for numerical_metadata, got {other:?}"),
    }
}

#[test]
fn call_runtime_method_with_test_runtime_returns_method_signature_mismatch() {
    let d = dispatcher_with_test_runtime();
    let argument = build_argument("test", "echo unused");
    let err = d
        .dispatch_call_runtime_method(&[], &argument)
        .expect_err("test runtime does not support call_method");
    assert!(
        matches!(err, FacadeError::Run(RunError::MethodSignatureMismatch(_))),
        "got {err:?}"
    );
}

// ── D53 §4.3 multi-input RunRuntimeScript ────────────────────────────

use eigenius_runtime_substrate::content_hash_of;
use std::sync::atomic::{AtomicU64, Ordering};

static MI_UNIQ: AtomicU64 = AtomicU64::new(0);

fn write_temp(name: &str, bytes: &[u8]) -> PathBuf {
    let n = MI_UNIQ.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("eig_facade_mi_{}_{n}_{name}", std::process::id()));
    std::fs::write(&p, bytes).unwrap();
    p
}

/// An `ingest:PinnedExternalFile` input resource (Eigon-CBOR), file:// backed.
fn pinned_file_input(reference: &str, content_hash: &str) -> Vec<u8> {
    let mut r = Resource::new_embedded();
    r.set(
        Iri::parse("urn:eigenius:core:is_a").unwrap(),
        Value::Array(vec![Value::ResourceRef(
            Iri::parse("urn:eigenius:ingest:PinnedExternalFile").unwrap(),
        )]),
    );
    r.set(
        Iri::parse("urn:eigenius:ingest:reference").unwrap(),
        Value::String(reference.to_string()),
    );
    r.set(
        Iri::parse("urn:eigenius:ingest:content_hash").unwrap(),
        Value::String(content_hash.to_string()),
    );
    r.set(
        Iri::parse("urn:eigenius:ingest:media_type").unwrap(),
        Value::String("text/csv".to_string()),
    );
    eigon_cbor::serialize_resource(&r)
}

#[test]
fn multi_input_materializes_and_verifies_additional_pinned_files() {
    let d = dispatcher_with_test_runtime();
    let argument = build_argument("test", "echo multi-ok");

    let bytes_a = b"a,b\n1,2\n";
    let bytes_b = b"x\n3\n";
    let pa = write_temp("a.csv", bytes_a);
    let pb = write_temp("b.csv", bytes_b);
    let add_a = pinned_file_input(
        &format!("file://{}", pa.display()),
        &content_hash_of(bytes_a),
    );
    let add_b = pinned_file_input(
        &format!("file://{}", pb.display()),
        &content_hash_of(bytes_b),
    );

    // Primary input empty; two content-verified additional file inputs.
    let outcome = d
        .dispatch_run_runtime_script_multi(&[], &[add_a, add_b], &argument)
        .expect("dispatch with two additional inputs (both content-verified)");
    let output = eigon_cbor::parse_resource_lenient(&outcome.output_cbor).expect("decode output");
    let stdout = output
        .get(&Iri::parse("urn:eigenius:test:bash_stdout").unwrap())
        .and_then(Value::as_str)
        .expect("worker ran after all inputs were prepared");
    assert_eq!(stdout.trim(), "multi-ok");
}

#[test]
fn multi_input_additional_file_fails_closed_on_tamper() {
    let d = dispatcher_with_test_runtime();
    let argument = build_argument("test", "echo should-not-run");
    let p = write_temp("tamper.csv", b"the real bytes\n");
    // Pinned with a wrong hash — the additional input must fail content
    // verification (in prepare_input) before the worker ever runs.
    let bad = pinned_file_input(&format!("file://{}", p.display()), "sha256:0000");
    let err = d
        .dispatch_run_runtime_script_multi(&[], &[bad], &argument)
        .expect_err("a tampered additional input must fail closed");
    assert!(
        matches!(err, FacadeError::Run(RunError::ContentHashMismatch { .. })),
        "got {err:?}"
    );
}
