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

//! P2 milestone: `RunRuntimeScript` end-to-end through the substrate's
//! `ServiceSpawner`. A `LocalServiceSpawner` launches the R worker (host
//! `Rscript`), the `RLanguageRuntime` dispatches an R `RuntimeScript`
//! through `ensure_service`/`attach_uds`, and the worker's output comes
//! back as a `RunOutcome`. The identical dispatch path runs under
//! `DockerServiceSpawner` with the pinned image (P3) — only the
//! `WorkerSpec` differs.
//!
//! Skips gracefully (passes) when `Rscript` is unavailable.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_r::RLanguageRuntime;
use eigenius_runtime_substrate::language_runtime::LanguageRuntime;
use eigenius_runtime_substrate::spawner::service::LocalServiceSpawner;

const PROP_SOURCE: &str = "urn:eigenius:runtime:source";
const PROP_SCRIPT_OUTPUT: &str = "urn:eigenius:runtime:script_output";

fn cdylib_path() -> PathBuf {
    let exe = std::env::current_exe().expect("test exe path");
    let profile_dir = exe
        .parent()
        .and_then(|deps| deps.parent())
        .expect("…/deps/.. = profile dir");
    let name = if cfg!(target_os = "macos") {
        "libeigenius_r_worker.dylib"
    } else {
        "libeigenius_r_worker.so"
    };
    profile_dir.join(name)
}

fn driver_path() -> PathBuf {
    // .../crates/eigenius-r → .../crates/eigenius-r-worker/r/EigeniusRWorker.R
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../eigenius-r-worker/r/EigeniusRWorker.R")
}

fn rscript_available() -> bool {
    Command::new("Rscript")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn resource_with(iri: &str, prop: &str, value: &str) -> Resource {
    let mut r = Resource::new(Iri::parse(iri).expect("iri"));
    r.set(
        Iri::parse(prop).expect("prop iri"),
        Value::String(value.to_string()),
    );
    r
}

#[test]
fn run_script_through_local_service_spawner() {
    if !rscript_available() {
        eprintln!("skipping run_script_through_local_service_spawner: Rscript not available");
        return;
    }
    let cdylib = cdylib_path();
    assert!(
        cdylib.exists(),
        "cdylib not built at {} (cargo builds eigenius-r-worker via dev-dep)",
        cdylib.display()
    );

    let depot = tempfile::tempdir().expect("depot tempdir");
    let spawner = Arc::new(LocalServiceSpawner::new(depot.path().to_path_buf()));
    let runtime = RLanguageRuntime::new(spawner, driver_path(), cdylib, depot.path().to_path_buf());

    // A RuntimeScript whose R source returns the output bytes (P1.2/P2
    // contract). `utf8ToInt` → the bytes of "hello-from-r".
    let script = resource_with(
        "urn:eigenius:test:rscript:hello",
        PROP_SOURCE,
        "as.raw(utf8ToInt(\"hello-from-r\"))",
    );
    let env = Resource::new(Iri::parse("urn:eigenius:test:renv").expect("env iri"));

    let outcome = runtime
        .run_script(&env, &script, &[])
        .expect("run_script succeeds end-to-end");

    let out = outcome
        .output
        .get(&Iri::parse(PROP_SCRIPT_OUTPUT).unwrap())
        .and_then(Value::as_str)
        .expect("output carries script_output");
    assert_eq!(
        out, "hello-from-r",
        "R evaluated the script and returned bytes"
    );

    // A second dispatch proves the per-invocation spawn/drain cycle is
    // re-runnable: sum(1:4) = 10 → byte 0x0a.
    let script2 = resource_with(
        "urn:eigenius:test:rscript:sum",
        PROP_SOURCE,
        "as.raw(sum(1:4))",
    );
    let outcome2 = runtime
        .run_script(&env, &script2, &[])
        .expect("second run_script succeeds");
    let out2 = outcome2
        .output
        .get(&Iri::parse(PROP_SCRIPT_OUTPUT).unwrap())
        .and_then(Value::as_str)
        .expect("output2");
    assert_eq!(out2.as_bytes(), &[10u8], "sum(1:4) = 10 → 0x0a");
}
