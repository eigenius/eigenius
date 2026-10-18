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

//! Loader precedence tests: defaults → file → env → override.
//!
//! Each test pins a synthetic env-var provider so the developer's
//! actual `$EIGENIUS_*` settings can't leak in. File loads use a
//! `tempfile::tempdir`-shaped scratch dir created inside `target/`
//! so cargo's cleanup discipline catches leftovers.

use eigenius_config::Loader;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Make a fresh empty scratch dir under the workspace's `target/`. A
/// tiny hand-rolled tempdir helper avoids pulling `tempfile` in for
/// just this test crate; the dirs are wiped at the start of the test
/// to keep reruns deterministic.
fn scratch_dir(label: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "eigenius-config-test-{}-{label}-{n}",
        std::process::id(),
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn empty_env() -> impl Fn(&str) -> Option<String> + Send + Sync + 'static {
    |_: &str| None
}

fn env_map(
    pairs: &[(&'static str, &'static str)],
) -> impl Fn(&str) -> Option<String> + Send + Sync + 'static {
    let map: HashMap<&'static str, &'static str> = pairs.iter().copied().collect();
    move |name: &str| map.get(name).map(|v| v.to_string())
}

#[test]
fn defaults_only_when_no_file_no_env_no_overrides() {
    let cfg = Loader::new()
        .no_search_path()
        .with_env_provider(empty_env())
        .load()
        .expect("load defaults");
    assert_eq!(
        cfg.substrate.docker.daemon_socket,
        PathBuf::from("/var/run/docker.sock")
    );
    assert_eq!(cfg.substrate.local.julia_binary, PathBuf::from("julia"));
    assert!(cfg.substrate.image.registry_url.is_none());
}

#[test]
fn file_overrides_defaults() {
    let dir = scratch_dir("file_over_defaults");
    let path = dir.join("eigenius.toml");
    std::fs::write(
        &path,
        r#"
[substrate.docker]
daemon_socket = "/run/user/1000/docker.sock"

[substrate.image]
registry_url = "registry.example.com:5000"
"#,
    )
    .unwrap();

    let cfg = Loader::new()
        .with_file(path)
        .with_env_provider(empty_env())
        .load()
        .expect("load file");
    assert_eq!(
        cfg.substrate.docker.daemon_socket,
        PathBuf::from("/run/user/1000/docker.sock")
    );
    assert_eq!(
        cfg.substrate.image.registry_url.as_deref(),
        Some("registry.example.com:5000")
    );
    // Field the file didn't touch keeps the default.
    assert_eq!(cfg.substrate.local.julia_binary, PathBuf::from("julia"));
}

#[test]
fn env_overrides_file() {
    let dir = scratch_dir("env_over_file");
    let path = dir.join("eigenius.toml");
    std::fs::write(
        &path,
        r#"
[substrate.docker]
daemon_socket = "/from/file.sock"
"#,
    )
    .unwrap();

    let cfg = Loader::new()
        .with_file(path)
        .with_env_provider(env_map(&[(
            "EIGENIUS_DOCKER_DAEMON_SOCKET",
            "/from/env.sock",
        )]))
        .load()
        .expect("load env over file");
    assert_eq!(
        cfg.substrate.docker.daemon_socket,
        PathBuf::from("/from/env.sock")
    );
}

#[test]
fn override_beats_env_and_file() {
    let dir = scratch_dir("override_beats");
    let path = dir.join("eigenius.toml");
    std::fs::write(
        &path,
        r#"
[substrate.docker]
daemon_socket = "/from/file.sock"
"#,
    )
    .unwrap();

    let cfg = Loader::new()
        .with_file(path)
        .with_env_provider(env_map(&[(
            "EIGENIUS_DOCKER_DAEMON_SOCKET",
            "/from/env.sock",
        )]))
        .with_overrides(|c| {
            c.substrate.docker.daemon_socket = PathBuf::from("/from/override.sock");
        })
        .load()
        .expect("load with overrides");
    assert_eq!(
        cfg.substrate.docker.daemon_socket,
        PathBuf::from("/from/override.sock")
    );
}

#[test]
fn missing_file_via_search_path_falls_through_to_defaults() {
    // Empty env (no $EIGENIUS_CONFIG, no $HOME) + a working
    // directory unlikely to contain `eigenius.toml`. The loader
    // should not error; it should fall through to defaults.
    let cfg = Loader::new()
        .with_env_provider(empty_env())
        .no_search_path() // belt and suspenders — also tests the "nothing on disk" path
        .load()
        .expect("load with nothing on disk");
    assert_eq!(
        cfg.substrate.docker.daemon_socket,
        PathBuf::from("/var/run/docker.sock")
    );
}

#[test]
fn explicit_file_missing_is_an_error() {
    let dir = scratch_dir("explicit_missing");
    let path = dir.join("does-not-exist.toml");
    let err = Loader::new()
        .with_file(path)
        .with_env_provider(empty_env())
        .load()
        .expect_err("explicit-but-missing must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("could not read config file") && msg.contains("does-not-exist.toml"),
        "got: {msg}"
    );
}

#[test]
fn malformed_file_surfaces_parse_error() {
    let dir = scratch_dir("malformed");
    let path = dir.join("eigenius.toml");
    std::fs::write(&path, "this is = not valid =\ntoml [\n").unwrap();
    let err = Loader::new()
        .with_file(path)
        .with_env_provider(empty_env())
        .load()
        .expect_err("malformed must fail");
    assert!(err.to_string().contains("config parse error"), "got: {err}");
}

#[test]
fn empty_env_var_unsets_optional_string() {
    // Setting `EIGENIUS_IMAGE_REGISTRY_URL=""` is the documented way
    // to override a file's value back to "no registry".
    let dir = scratch_dir("empty_env_unsets");
    let path = dir.join("eigenius.toml");
    std::fs::write(
        &path,
        r#"
[substrate.image]
registry_url = "registry.example.com"
"#,
    )
    .unwrap();
    let cfg = Loader::new()
        .with_file(path)
        .with_env_provider(env_map(&[("EIGENIUS_IMAGE_REGISTRY_URL", "")]))
        .load()
        .expect("load");
    assert!(cfg.substrate.image.registry_url.is_none());
}
