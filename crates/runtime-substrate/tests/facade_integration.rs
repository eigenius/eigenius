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
    let output_cbor = d
        .dispatch_run_runtime_script(&[], &argument)
        .expect("dispatch");
    let output = eigon_cbor::parse_resource_lenient(&output_cbor).expect("decode output");
    let stdout = output
        .get(&Iri::parse("urn:eigenius:test:bash_stdout").unwrap())
        .and_then(Value::as_str)
        .expect("bash_stdout property on output");
    assert_eq!(stdout.trim(), "facade-validated");
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
