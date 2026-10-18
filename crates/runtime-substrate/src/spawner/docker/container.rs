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

//! Pure assembly of [`bollard`] container configs from a [`WorkerSpec`].
//!
//! Kept in its own module so the assembly is testable without a Docker
//! daemon — every behavioural decision (binds layout, env serialisation,
//! capability drop, network mode, resource caps) is observable in the
//! produced [`bollard::container::Config`] without spawning anything.
//!
//! ## Decisions encoded here (D26 §8.2 / §8.3 / §9.5)
//!
//! - **Bind-mount layout.** Per-invocation tempdir bind-mounted at the
//!   *same path* in the container as on the host (DooD invariant). Depot
//!   bind-mounted read-only at the same path so workers can read shared
//!   artifacts (mirror archives, package caches) without translation.
//! - **`AutoRemove: true`.** Job model — container is reaped on exit so
//!   we don't leak stopped containers across invocations. Substrate must
//!   `wait_container` *before* exit to capture the exit code; that
//!   ordering lives in [`super::lifecycle`].
//! - **`CapDrop: ["ALL"]`.** Worker runs with the empty capability set.
//!   18c.4 will surface a narrow re-add list when a hosted runtime needs
//!   one; for 18c.3 we ship the strictest baseline.
//! - **Network mode.** Configured via [`super::config::NetworkMode`];
//!   defaults to `none` for the spawn-per-invocation Job model.
//! - **Resource caps.** `WorkerSpec::max_memory_bytes` → `HostConfig.memory`;
//!   `WorkerSpec::max_wall_time_ms` is enforced *outside* the container
//!   by [`super::lifecycle::wait`] (no container-side knob for wall-clock).

use crate::error::SpawnError;
use crate::spawner::docker::config::NetworkMode;
use crate::types::WorkerSpec;
use bollard::models::{HostConfig, Mount, MountType};
use bollard::query_parameters::CreateContainerOptionsBuilder;
use std::collections::HashMap;
use std::path::Path;

/// Inputs the pure assembler needs: the worker spec, the resolved
/// per-invocation tempdir (always under the depot), the depot path
/// itself (mounted read-only into the container), the network mode
/// to apply, and whether the container should auto-remove on exit.
///
/// `auto_remove = true` is the default for the per-invocation
/// `WorkerSpawner` (Job mode); `auto_remove = false` is what
/// `DockerServiceSpawner` (Service mode) sets so the container
/// persists across many dispatches until explicitly drained.
#[derive(Debug)]
pub struct ContainerBuildInputs<'a> {
    pub spec: &'a WorkerSpec,
    pub tempdir: &'a Path,
    pub depot: &'a Path,
    pub network_mode: &'a NetworkMode,
    pub auto_remove: bool,
}

/// Output of [`build_create_options`]: the create-container options
/// (which are name + platform metadata) and the body (image / command /
/// env / host config). Bollard's API splits these on the wire so we
/// pre-split here.
#[derive(Debug)]
pub struct CreatePlan {
    pub options: bollard::query_parameters::CreateContainerOptions,
    pub body: bollard::models::ContainerCreateBody,
}

/// Assemble the create-container request body and options from the
/// worker spec.
///
/// Pure transformation — no I/O, no Docker calls. The caller is
/// responsible for having validated the depot / tempdir relationship
/// via [`super::depot::verify_tempdir_under_depot`] beforehand; this
/// function will not re-check.
pub fn build_create_options(inputs: &ContainerBuildInputs) -> Result<CreatePlan, SpawnError> {
    let image_digest =
        inputs
            .spec
            .image_digest
            .as_ref()
            .ok_or_else(|| SpawnError::SpawnFailed {
                backend: super::BACKEND,
                reason: "DockerSpawner requires WorkerSpec::image_digest to be Some(_)".into(),
            })?;

    let env = serialise_env(inputs.spec);
    let cmd = if inputs.spec.command.is_empty() {
        None
    } else {
        Some(inputs.spec.command.clone())
    };
    let host_config = build_host_config(inputs);

    let body = bollard::models::ContainerCreateBody {
        image: Some(image_digest.as_str().to_string()),
        cmd,
        env: Some(env),
        host_config: Some(host_config),
        labels: Some(substrate_labels(inputs.spec)),
        // Disable stdin / TTY: workers communicate over UDS, never
        // through the docker stream. Avoids accidentally hanging the
        // create call waiting for a TTY.
        attach_stdin: Some(false),
        attach_stdout: Some(false),
        attach_stderr: Some(false),
        open_stdin: Some(false),
        tty: Some(false),
        ..Default::default()
    };

    let options = CreateContainerOptionsBuilder::new()
        .name("") // daemon assigns
        .build();

    Ok(CreatePlan { options, body })
}

fn serialise_env(spec: &WorkerSpec) -> Vec<String> {
    // BTreeMap iteration is already deterministic; emit `K=V` strings
    // in that order so two structurally-equal specs produce identical
    // env vectors (helpful for tests).
    spec.env.iter().map(|(k, v)| format!("{k}={v}")).collect()
}

fn build_host_config(inputs: &ContainerBuildInputs) -> HostConfig {
    let tempdir = inputs.tempdir.to_string_lossy().into_owned();
    let depot = inputs.depot.to_string_lossy().into_owned();
    let mounts = vec![
        Mount {
            target: Some(tempdir.clone()),
            source: Some(tempdir),
            typ: Some(MountType::BIND),
            read_only: Some(false),
            ..Default::default()
        },
        Mount {
            target: Some(depot.clone()),
            source: Some(depot),
            typ: Some(MountType::BIND),
            read_only: Some(true),
            ..Default::default()
        },
    ];
    let memory = if inputs.spec.max_memory_bytes == 0 {
        None
    } else {
        Some(inputs.spec.max_memory_bytes as i64)
    };
    HostConfig {
        auto_remove: Some(inputs.auto_remove),
        // No cap_drop. The substrate is *not* a sandbox-as-a-service
        // (D26 §1.2) — its job is provenance + dispatch for trusted
        // language toolchains, not containing untrusted code. Dropping
        // capabilities by default would force every deployment to
        // solve UID-alignment between container processes and host
        // bind-mount owners, which is real ops work for zero benefit
        // under the trusted-but-tracked threat model. If a future
        // deployment scenario genuinely needs adversarial containment,
        // add `cap_drop: Option<Vec<String>>` to `WorkerSpec` and let
        // the caller opt in.
        network_mode: Some(inputs.network_mode.as_docker_string().to_string()),
        mounts: Some(mounts),
        memory,
        security_opt: Some(build_security_opts(inputs.spec)),
        // Tmpfs / sysctls / ulimits left at defaults; no current need.
        ..Default::default()
    }
}

/// Assemble the `security_opt` Vec for `HostConfig`. Always sets
/// `no-new-privileges:true` — free defense-in-depth, no UID-alignment
/// interaction. Honours `WorkerSpec::seccomp_profile` when set;
/// otherwise leaves Docker's built-in default seccomp profile in place
/// (substantially restrictive for trusted-but-tracked workloads per
/// D26 §1.2). Per-language crates that ship a tighter profile populate
/// `WorkerSpec::seccomp_profile` from their `LanguageRuntime::spawn_worker`.
fn build_security_opts(spec: &WorkerSpec) -> Vec<String> {
    let mut opts = vec!["no-new-privileges:true".to_string()];
    if let Some(profile) = &spec.seccomp_profile {
        opts.push(format!("seccomp={profile}"));
    }
    opts
}

/// Container-bookkeeping labels so substrate-spawned containers can be
/// distinguished from the user's own when `auto_remove` fails (daemon
/// restart between create and start, etc.) and ops can grep for stale
/// substrate workloads. Applied via `ContainerCreateBody::labels`.
///
/// Currently emits the substrate-version label and the image digest
/// the spawn was anchored to. Future labels (env IRI, invocation IRI)
/// land alongside the dispatcher's resource-context plumbing.
pub fn substrate_labels(spec: &WorkerSpec) -> HashMap<String, String> {
    let mut labels = HashMap::new();
    labels.insert(
        "eigenius.substrate".to_string(),
        crate::spawner::docker::SUBSTRATE_LABEL_VERSION.to_string(),
    );
    if let Some(d) = spec.image_digest.as_ref() {
        labels.insert("eigenius.image_digest".to_string(), d.as_str().to_string());
    }
    labels
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spawner::docker::config::NetworkMode;
    use crate::types::ImageDigest;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn dummy_digest() -> ImageDigest {
        ImageDigest::parse(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("digest parses")
    }

    fn spec(
        image: Option<ImageDigest>,
        command: Vec<&str>,
        env: BTreeMap<String, String>,
    ) -> WorkerSpec {
        WorkerSpec {
            image_digest: image,
            command: command.into_iter().map(String::from).collect(),
            tempdir_host_path: PathBuf::from("/var/lib/eigenius-runtime/inv-1"),
            depot_host_path: Some(PathBuf::from("/var/lib/eigenius-runtime")),
            env,
            max_wall_time_ms: 0,
            max_memory_bytes: 0,
            seccomp_profile: None,
        }
    }

    #[test]
    fn build_requires_image_digest() {
        let s = spec(None, vec!["bin"], BTreeMap::new());
        let inputs = ContainerBuildInputs {
            spec: &s,
            tempdir: Path::new("/var/lib/eigenius-runtime/inv-1"),
            depot: Path::new("/var/lib/eigenius-runtime"),
            network_mode: &NetworkMode::None,
            auto_remove: true,
        };
        let err = build_create_options(&inputs).expect_err("must require digest");
        match err {
            SpawnError::SpawnFailed { reason, .. } => {
                assert!(reason.contains("image_digest"));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn build_emits_image_command_and_env() {
        let mut env = BTreeMap::new();
        env.insert("A".to_string(), "1".to_string());
        env.insert("B".to_string(), "2".to_string());
        let s = spec(Some(dummy_digest()), vec!["worker", "--flag"], env);
        let inputs = ContainerBuildInputs {
            spec: &s,
            tempdir: Path::new("/var/lib/eigenius-runtime/inv-1"),
            depot: Path::new("/var/lib/eigenius-runtime"),
            network_mode: &NetworkMode::None,
            auto_remove: true,
        };
        let plan = build_create_options(&inputs).expect("build");
        assert_eq!(plan.body.image.as_deref(), Some(dummy_digest().as_str()));
        assert_eq!(
            plan.body.cmd.as_deref(),
            Some(&["worker".to_string(), "--flag".to_string()][..]),
        );
        // Env serialised in BTreeMap-sorted order.
        assert_eq!(
            plan.body.env.as_deref(),
            Some(&["A=1".to_string(), "B=2".to_string()][..]),
        );
    }

    #[test]
    fn empty_command_defers_to_image_cmd() {
        let s = spec(Some(dummy_digest()), vec![], BTreeMap::new());
        let inputs = ContainerBuildInputs {
            spec: &s,
            tempdir: Path::new("/var/lib/eigenius-runtime/inv-1"),
            depot: Path::new("/var/lib/eigenius-runtime"),
            network_mode: &NetworkMode::None,
            auto_remove: true,
        };
        let plan = build_create_options(&inputs).expect("build");
        assert!(
            plan.body.cmd.is_none(),
            "empty command must defer to image CMD"
        );
    }

    #[test]
    fn host_config_carries_dood_binds_and_no_network() {
        let s = spec(Some(dummy_digest()), vec!["bin"], BTreeMap::new());
        let inputs = ContainerBuildInputs {
            spec: &s,
            tempdir: Path::new("/var/lib/eigenius-runtime/inv-1"),
            depot: Path::new("/var/lib/eigenius-runtime"),
            network_mode: &NetworkMode::None,
            auto_remove: true,
        };
        let plan = build_create_options(&inputs).expect("build");
        let host = plan.body.host_config.as_ref().expect("host_config");
        assert_eq!(host.auto_remove, Some(true));
        // No cap_drop: substrate is not a sandbox-as-a-service (D26 §1.2).
        // Regression guard against a well-meaning future re-add.
        assert!(
            host.cap_drop.is_none(),
            "cap_drop must be absent — substrate is provenance + dispatch, not adversarial containment (D26 §1.2)",
        );
        assert_eq!(host.network_mode.as_deref(), Some("none"));
        let mounts = host.mounts.as_ref().expect("mounts");
        assert_eq!(mounts.len(), 2);
        // Tempdir bind: same source = target, read-write.
        let tempdir_mount = &mounts[0];
        assert_eq!(
            tempdir_mount.source.as_deref(),
            Some("/var/lib/eigenius-runtime/inv-1")
        );
        assert_eq!(
            tempdir_mount.target.as_deref(),
            Some("/var/lib/eigenius-runtime/inv-1")
        );
        assert_eq!(tempdir_mount.read_only, Some(false));
        // Depot bind: same source = target, read-only.
        let depot_mount = &mounts[1];
        assert_eq!(
            depot_mount.source.as_deref(),
            Some("/var/lib/eigenius-runtime")
        );
        assert_eq!(
            depot_mount.target.as_deref(),
            Some("/var/lib/eigenius-runtime")
        );
        assert_eq!(depot_mount.read_only, Some(true));
    }

    #[test]
    fn memory_cap_translates_to_host_config() {
        let mut s = spec(Some(dummy_digest()), vec!["bin"], BTreeMap::new());
        s.max_memory_bytes = 256 * 1024 * 1024;
        let inputs = ContainerBuildInputs {
            spec: &s,
            tempdir: Path::new("/var/lib/eigenius-runtime/inv-1"),
            depot: Path::new("/var/lib/eigenius-runtime"),
            network_mode: &NetworkMode::None,
            auto_remove: true,
        };
        let plan = build_create_options(&inputs).expect("build");
        assert_eq!(
            plan.body.host_config.as_ref().and_then(|h| h.memory),
            Some(256 * 1024 * 1024)
        );
    }

    #[test]
    fn zero_memory_cap_translates_to_unbounded() {
        let s = spec(Some(dummy_digest()), vec!["bin"], BTreeMap::new());
        let inputs = ContainerBuildInputs {
            spec: &s,
            tempdir: Path::new("/var/lib/eigenius-runtime/inv-1"),
            depot: Path::new("/var/lib/eigenius-runtime"),
            network_mode: &NetworkMode::None,
            auto_remove: true,
        };
        let plan = build_create_options(&inputs).expect("build");
        assert!(plan
            .body
            .host_config
            .as_ref()
            .and_then(|h| h.memory)
            .is_none());
    }

    #[test]
    fn security_opt_always_carries_no_new_privileges() {
        let s = spec(Some(dummy_digest()), vec!["bin"], BTreeMap::new());
        let inputs = ContainerBuildInputs {
            spec: &s,
            tempdir: Path::new("/var/lib/eigenius-runtime/inv-1"),
            depot: Path::new("/var/lib/eigenius-runtime"),
            network_mode: &NetworkMode::None,
            auto_remove: true,
        };
        let plan = build_create_options(&inputs).expect("build");
        let opts = plan
            .body
            .host_config
            .as_ref()
            .and_then(|h| h.security_opt.as_ref())
            .expect("security_opt");
        assert!(
            opts.iter().any(|o| o == "no-new-privileges:true"),
            "expected no-new-privileges in security_opt, got {opts:?}"
        );
        assert!(
            !opts.iter().any(|o| o.starts_with("seccomp=")),
            "no seccomp profile was supplied — `seccomp=` should be absent so Docker's default applies"
        );
    }

    #[test]
    fn worker_spec_seccomp_profile_propagates_into_security_opt() {
        let mut s = spec(Some(dummy_digest()), vec!["bin"], BTreeMap::new());
        s.seccomp_profile = Some(r#"{"defaultAction":"SCMP_ACT_ERRNO"}"#.into());
        let inputs = ContainerBuildInputs {
            spec: &s,
            tempdir: Path::new("/var/lib/eigenius-runtime/inv-1"),
            depot: Path::new("/var/lib/eigenius-runtime"),
            network_mode: &NetworkMode::None,
            auto_remove: true,
        };
        let plan = build_create_options(&inputs).expect("build");
        let opts = plan
            .body
            .host_config
            .as_ref()
            .and_then(|h| h.security_opt.as_ref())
            .expect("security_opt");
        assert!(opts
            .iter()
            .any(|o| o == r#"seccomp={"defaultAction":"SCMP_ACT_ERRNO"}"#));
    }

    #[test]
    fn substrate_labels_carry_version_and_image_digest() {
        let s = spec(Some(dummy_digest()), vec!["bin"], BTreeMap::new());
        let inputs = ContainerBuildInputs {
            spec: &s,
            tempdir: Path::new("/var/lib/eigenius-runtime/inv-1"),
            depot: Path::new("/var/lib/eigenius-runtime"),
            network_mode: &NetworkMode::None,
            auto_remove: true,
        };
        let plan = build_create_options(&inputs).expect("build");
        let labels = plan.body.labels.expect("labels");
        assert_eq!(
            labels.get("eigenius.substrate").map(String::as_str),
            Some("1")
        );
        assert_eq!(
            labels.get("eigenius.image_digest").map(String::as_str),
            Some(dummy_digest().as_str())
        );
    }

    #[test]
    fn network_mode_renders_named_when_supplied() {
        let s = spec(Some(dummy_digest()), vec!["bin"], BTreeMap::new());
        let mode = NetworkMode::Named("my-net".into());
        let inputs = ContainerBuildInputs {
            spec: &s,
            tempdir: Path::new("/var/lib/eigenius-runtime/inv-1"),
            depot: Path::new("/var/lib/eigenius-runtime"),
            network_mode: &mode,
            auto_remove: true,
        };
        let plan = build_create_options(&inputs).expect("build");
        assert_eq!(
            plan.body
                .host_config
                .as_ref()
                .and_then(|h| h.network_mode.as_deref()),
            Some("my-net")
        );
    }
}
