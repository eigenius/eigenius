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

//! Dockerfile fragments emitted by `LeanLanguageRuntime`.
//!
//! Composed by the substrate's image-build pipeline (D26 §9.2) into
//! a final Dockerfile. The fragments reference paths from
//! [`crate::conventions`] — keep the two in sync.
//!
//! ## Lean vs. Julia
//!
//! Julia ships a single binary plus a built-in package manager
//! (`Pkg`). The substrate's `julia:` base image already includes the
//! interpreter, so `install_runtime` is empty and the heavy lifting
//! happens in `install_packages` (`Pkg.instantiate`).
//!
//! Lean's tooling is split: `elan` (the toolchain manager,
//! conceptually `rustup` for Lean) installs a specific Lean
//! toolchain, and `lake` (the build tool, conceptually `cargo` for
//! Lean) drives builds and dependency resolution from a
//! `lakefile.lean` + `lake-manifest.json`. There is no upstream
//! `lean:` Docker image we can rely on the way Julia uses
//! `julia:1.12-bookworm`, so `install_runtime` does the
//! elan-plus-toolchain install itself against a generic Debian base.

use crate::conventions::{ELAN_HOME, LEAN_TOOLCHAIN_VERSION, WORKER_PROJECT_DIR};
use eigenius_runtime_substrate::types::DockerfileFragments;

/// Path the substrate composer materialises included packages under
/// (`/opt/eigenius/packages/<name>/`). Mirrored from the substrate
/// composer — keeping the constant local so the fragments stay
/// self-contained.
const PACKAGES_IN_IMAGE: &str = "/opt/eigenius/packages";

/// Inputs to [`lean_dockerfile_fragments`]. Lets the build path
/// control whether the env image bakes in extra `LeanPackage`
/// dependencies (`included_packages` on the env resource). The
/// `include_mirror` flag is reserved for 20a.6 when the
/// `LeanMirrorGenerator` lands and the env image grows a generated
/// EigonFFI library.
#[derive(Debug, Clone, Default)]
pub struct LeanImagePlan {
    /// `true` when a `LeanPackageMirror` archive has been
    /// materialised under `/opt/eigenius/mirror/`. Reserved — 20a.5a
    /// always sets this to `false`; 20a.6 lights it up.
    pub include_mirror: bool,
    /// Names of additional `LeanPackage` resources baked under
    /// `/opt/eigenius/packages/<name>/` alongside the worker's own
    /// Lake project. Each gets a `lake update` + `lake build` pass so
    /// its dependencies resolve into the worker's manifest at
    /// instantiate time. Order is preserved for deterministic
    /// dockerfile output.
    pub handler_packages: Vec<String>,
}

/// Dockerfile fragments for a Lean env image. Extends a generic
/// Debian-slim base (the substrate composer's default) with:
///
/// 1. `install_runtime`: install `elan`, pin the Lean toolchain
///    version, install `git` + `curl` (needed to fetch Lean
///    dependencies via `lake update`). `elan` is fetched non-
///    interactively into [`crate::conventions::ELAN_HOME`].
/// 2. `install_packages`: drive `lake update` + `lake build` against
///    the worker's project to materialise the lockfile-pinned
///    Lean dependencies. Reads from `lakefile.lean` +
///    `lake-manifest.json` that the substrate composer has staged
///    under [`crate::conventions::WORKER_PROJECT_DIR`]. Handler
///    packages get a follow-up `lake build` each, in declaration
///    order.
/// 3. `install_mirror`: empty in 20a.5a — the
///    [`LeanMirrorGenerator`] lands in 20a.6 and will provision a
///    generated EigonFFI library here.
/// 4. `bootstrap_command`: launch the Lake worker binary as PID 1
///    inside the container. v1 invokes `lake exe lean-runtime-worker`
///    — the actual worker binary lands in 20a.5b; the bootstrap
///    command is recorded here so the Dockerfile composer's spec
///    test (which inspects the fragment shape) sees the production
///    target from day one.
pub fn lean_dockerfile_fragments(plan: &LeanImagePlan) -> DockerfileFragments {
    DockerfileFragments {
        install_runtime: install_runtime_lines(),
        install_packages: install_packages_lines(plan),
        install_mirror: install_mirror_lines(plan),
        bootstrap_command: vec![
            // Lake's `exe` form invokes a built binary declared in
            // `lakefile.lean`. The worker project (lean/runtime-worker/)
            // declares `lean_exe lean-runtime-worker := …`; the
            // resulting binary handles CBOR-framed RPC over UDS.
            "lake".to_string(),
            "--dir".to_string(),
            WORKER_PROJECT_DIR.to_string(),
            "exe".to_string(),
            "lean-runtime-worker".to_string(),
        ],
    }
}

/// Install `elan` + a pinned Lean toolchain on top of a Debian-slim
/// base. Single `RUN` so the layer cache key is deterministic —
/// splitting into multiple `RUN`s would let `apt-get install` and
/// `elan-init` produce different layer hashes even when the
/// downloaded bytes are identical.
fn install_runtime_lines() -> Vec<String> {
    // The toolchain version goes into `elan toolchain install` so
    // the install step pulls the bits at build time rather than
    // first-dispatch. `elan default` sets it as the default for
    // every subsequent `lake`/`lean` invocation.
    vec![format!(
        "RUN apt-get update \
            && apt-get install -y --no-install-recommends curl git ca-certificates \
            && rm -rf /var/lib/apt/lists/* \
            && curl -sSf https://raw.githubusercontent.com/leanprover/elan/master/elan-init.sh \
               -o /tmp/elan-init.sh \
            && ELAN_HOME={ELAN_HOME} sh /tmp/elan-init.sh -y --no-modify-path --default-toolchain {LEAN_TOOLCHAIN_VERSION} \
            && rm /tmp/elan-init.sh \
            && ln -s {ELAN_HOME}/bin/elan /usr/local/bin/elan \
            && ln -s {ELAN_HOME}/bin/lake /usr/local/bin/lake \
            && ln -s {ELAN_HOME}/bin/lean /usr/local/bin/lean"
    )]
}

fn install_packages_lines(plan: &LeanImagePlan) -> Vec<String> {
    // Single RUN so the lockfile resolution + worker build land in
    // one layer; matches Julia's `Pkg.instantiate(); Pkg.precompile()`
    // discipline. `lake update --keep-toolchain` resolves the
    // manifest, then `lake build` precompiles the worker target.
    let mut script = format!(
        "cd {WORKER_PROJECT_DIR} \
            && lake update --keep-toolchain \
            && lake build"
    );
    // Handler packages — each is its own Lake project under
    // `/opt/eigenius/packages/<name>/`. We resolve + build each in
    // declaration order so a later handler that depends on an
    // earlier one finds the build artifacts already on disk.
    for name in &plan.handler_packages {
        script.push_str(&format!(
            " && cd {PACKAGES_IN_IMAGE}/{name} \
              && lake update --keep-toolchain \
              && lake build"
        ));
    }
    vec![format!("RUN {script}")]
}

fn install_mirror_lines(_plan: &LeanImagePlan) -> Vec<String> {
    // 20a.6 lights this up: the LeanMirrorGenerator will produce an
    // EigonFFI library archive that the substrate composer COPYs
    // under `/opt/eigenius/mirror/`; this section will then `lake
    // develop` it into the worker's project and `lake build` for
    // precompilation. Empty in 20a.5a.
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_plan_emits_install_runtime_with_elan_and_toolchain() {
        // The runtime install step is the same regardless of plan —
        // elan + the pinned toolchain are the load-bearing prereqs
        // for `lake` to run at all. A missing toolchain version
        // would make first-dispatch try to download Lean on the wire
        // instead of using the baked image.
        let f = lean_dockerfile_fragments(&LeanImagePlan::default());
        assert_eq!(f.install_runtime.len(), 1, "single RUN for cache stability");
        let line = &f.install_runtime[0];
        assert!(line.contains("elan-init.sh"));
        assert!(line.contains(LEAN_TOOLCHAIN_VERSION));
        assert!(line.contains(ELAN_HOME));
    }

    #[test]
    fn default_plan_runs_lake_update_and_build() {
        let f = lean_dockerfile_fragments(&LeanImagePlan::default());
        assert_eq!(f.install_packages.len(), 1);
        let line = &f.install_packages[0];
        assert!(line.contains("lake update"));
        assert!(line.contains("lake build"));
        assert!(line.contains(WORKER_PROJECT_DIR));
    }

    #[test]
    fn install_mirror_empty_until_20a6() {
        // The mirror section stays empty for 20a.5a even when the
        // plan opts in — the field is reserved so 20a.6 can flip the
        // bit without changing the public API.
        let f = lean_dockerfile_fragments(&LeanImagePlan {
            include_mirror: true,
            handler_packages: Vec::new(),
        });
        assert!(f.install_mirror.is_empty());
    }

    #[test]
    fn handler_packages_appear_in_declaration_order() {
        let f = lean_dockerfile_fragments(&LeanImagePlan {
            include_mirror: false,
            handler_packages: vec!["A".to_string(), "B".to_string(), "C".to_string()],
        });
        let line = &f.install_packages[0];
        let a = line.find("packages/A").expect("A built");
        let b = line.find("packages/B").expect("B built");
        let c = line.find("packages/C").expect("C built");
        assert!(
            a < b && b < c,
            "handler packages must keep declaration order"
        );
    }

    #[test]
    fn bootstrap_command_runs_lake_exe_worker() {
        let f = lean_dockerfile_fragments(&LeanImagePlan::default());
        // The bootstrap command is the worker entry point — pinning
        // this assertion catches an accidental rename of the Lake
        // executable target landing in 20a.5b.
        assert!(
            f.bootstrap_command.iter().any(|s| s == "lake"),
            "bootstrap must invoke `lake`"
        );
        assert!(
            f.bootstrap_command
                .iter()
                .any(|s| s == "lean-runtime-worker"),
            "bootstrap must target the `lean-runtime-worker` Lake exe declared in lakefile.lean"
        );
    }
}
