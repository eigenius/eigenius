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

//! Integration tests for [`TestLanguageRuntime`] — the bash-c smoke
//! runtime that wraps the `eigenius-test-worker` binary as a
//! `LanguageRuntime` impl.
//!
//! These tests can't live in the lib's unit-test module because they
//! need `env!("CARGO_BIN_EXE_eigenius-test-worker")`, which Cargo only
//! sets for integration tests in `tests/`.

#![cfg(feature = "test-runtime")]

use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_runtime_substrate::error::{BuildError, RunError};
use eigenius_runtime_substrate::language_runtime::LanguageRuntime;
use eigenius_runtime_substrate::test_runtime::TestLanguageRuntime;
use std::path::PathBuf;

const PROP_LANGUAGE: &str = "urn:eigenius:runtime:language";
const PROP_SOURCE: &str = "urn:eigenius:runtime:source";
const PROP_TEST_BASH_STDOUT: &str = "urn:eigenius:test:bash_stdout";
const LANGUAGE: &str = "test";

fn worker_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_eigenius-test-worker"))
}

fn make_script(bash_command: &str) -> Resource {
    let iri = Iri::parse("urn:eigenius:test:script:hello").unwrap();
    let mut r = Resource::new(iri);
    r.set(
        Iri::parse(PROP_LANGUAGE).unwrap(),
        Value::String(LANGUAGE.to_string()),
    );
    r.set(
        Iri::parse(PROP_SOURCE).unwrap(),
        Value::String(bash_command.to_string()),
    );
    r
}

fn make_env() -> Resource {
    let iri = Iri::parse("urn:eigenius:test:env:bash").unwrap();
    let mut r = Resource::new(iri);
    r.set(
        Iri::parse(PROP_LANGUAGE).unwrap(),
        Value::String(LANGUAGE.to_string()),
    );
    r
}

#[test]
fn run_script_round_trips_through_real_worker() {
    let runtime = TestLanguageRuntime::with_worker_binary(worker_binary());
    let env = make_env();
    let script = make_script("echo runtime-trait-validated");

    let outcome = runtime.run_script(&env, &script, &[]).expect("run_script");

    let stdout = outcome
        .output
        .get(&Iri::parse(PROP_TEST_BASH_STDOUT).unwrap())
        .and_then(Value::as_str)
        .expect("bash_stdout property");
    assert_eq!(stdout.trim(), "runtime-trait-validated");
    let lang = outcome
        .output
        .get(&Iri::parse(PROP_LANGUAGE).unwrap())
        .and_then(Value::as_str)
        .expect("language property");
    assert_eq!(lang, LANGUAGE);
    assert!(!outcome.started_at.is_empty());
    assert!(!outcome.completed_at.is_empty());
}

#[test]
fn run_script_surfaces_dispatch_failed_as_runtime_error() {
    let runtime = TestLanguageRuntime::with_worker_binary(worker_binary());
    let env = make_env();
    let script = make_script("echo nope 1>&2; exit 5");

    let err = runtime
        .run_script(&env, &script, &[])
        .expect_err("expected runtime_error");
    match err {
        RunError::RuntimeError(msg) => {
            assert!(msg.contains("nope") && msg.contains('5'), "got `{msg}`");
        }
        other => panic!("expected RuntimeError, got {other:?}"),
    }
}

#[test]
fn run_script_rejects_missing_source() {
    let runtime = TestLanguageRuntime::with_worker_binary(worker_binary());
    let env = make_env();
    // Script without `source` property.
    let mut script = Resource::new(Iri::parse("urn:eigenius:test:script:bad").unwrap());
    script.set(
        Iri::parse(PROP_LANGUAGE).unwrap(),
        Value::String(LANGUAGE.to_string()),
    );

    let err = runtime
        .run_script(&env, &script, &[])
        .expect_err("expected method_signature_mismatch when source is missing");
    assert!(matches!(err, RunError::MethodSignatureMismatch(_)));
}

#[test]
fn call_method_is_unsupported_in_test_runtime() {
    let runtime = TestLanguageRuntime::with_worker_binary(worker_binary());
    let env = make_env();
    let signature = make_script("ignored");
    let err = runtime
        .call_method(&env, &signature, &[])
        .expect_err("call_method should not be supported");
    assert!(matches!(err, RunError::MethodSignatureMismatch(_)));
}

#[test]
fn build_environment_image_is_unsupported() {
    let runtime = TestLanguageRuntime::with_worker_binary(worker_binary());
    let env = make_env();
    let err = runtime
        .build_environment_image(&env, &[], None)
        .expect_err("build_environment_image should not be supported");
    assert!(matches!(err, BuildError::EnvironmentBuildFailed(_)));
}
