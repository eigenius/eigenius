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

use crate::conventions::{
    ELAN_HOME, LD_SO_CONF_PATH, LEAN4EXPORT_IN_IMAGE, LEAN_COMMON_IN_IMAGE, LEAN_TOOLCHAIN_VERSION,
    MIRROR_IN_IMAGE, WORKER_BIN_PATH, WORKER_LIB_DIR,
};
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
///    version, install `git` + `curl` (needed by `lake exe
///    lean4export` at dispatch time and for `lake build` of the
///    vendored `lean4export` below). `elan` is fetched non-
///    interactively into [`crate::conventions::ELAN_HOME`].
/// 2. `install_packages`: register [`crate::conventions::WORKER_LIB_DIR`]
///    with the glibc dynamic linker (via `/etc/ld.so.conf.d/` +
///    `ldconfig`) so the worker binary's host-side `DT_RUNPATH`
///    is silently bypassed by `ld.so.cache`, then `lake build` the
///    vendored `lean4export` so first dispatch reuses the cached
///    binary. The worker binary itself is staged via
///    [`LanguageAssetCopy`] entries the runtime crate emits — it's
///    pre-built on the host and COPY'd in, not built in the image.
/// 3. `install_mirror`: empty in 20a.5a — the
///    [`LeanMirrorGenerator`] lands in 20a.6 and will provision a
///    generated EigonFFI library here.
/// 4. `bootstrap_command`: launch the pre-built worker binary as PID 1.
///    The worker reads its UDS path from `EIGENIUS_TEST_WORKER_UDS`
///    (set by the substrate spawner) so no CMD args are needed.
pub fn lean_dockerfile_fragments(plan: &LeanImagePlan) -> DockerfileFragments {
    DockerfileFragments {
        install_runtime: install_runtime_lines(),
        install_packages: install_packages_lines(plan),
        install_mirror: install_mirror_lines(plan),
        bootstrap_command: vec![WORKER_BIN_PATH.to_string()],
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
    // Single RUN so dynamic-linker registration + lean4export
    // pre-build land in one layer.
    //
    // 1. Write `/etc/ld.so.conf.d/eigenius-lean.conf` carrying the
    //    cdylib directory and run `ldconfig` so the dynamic linker
    //    finds `libeigenius_lean_worker.so` when the worker binary
    //    starts. The worker's `DT_RUNPATH` was stamped by the
    //    host-side `cargo build` and points at the host workspace —
    //    it doesn't resolve inside the container, but `ld.so.cache`
    //    is consulted *before* `DT_RUNPATH` by glibc, so the cache
    //    entry wins and the stale RUNPATH is silently bypassed.
    //
    // 2. `lake build` the vendored lean4export so its compiled
    //    binary lands in this layer's cache. First-dispatch
    //    `lake exe lean4export` from a staged `LeanProject` then
    //    reuses the cached binary instead of recompiling
    //    (~5-10 s saved per dispatch). The lean4export source tree
    //    is staged at [`LEAN4EXPORT_IN_IMAGE`] by the COPY
    //    directives the runtime crate's `ensure_image` emits as
    //    `LanguageAssetCopy` entries.
    let mut script = format!(
        "echo {WORKER_LIB_DIR} > {LD_SO_CONF_PATH} \
            && ldconfig \
            && cd {LEAN4EXPORT_IN_IMAGE} \
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

fn install_mirror_lines(plan: &LeanImagePlan) -> Vec<String> {
    if !plan.include_mirror {
        return vec![];
    }
    // D30 §2.1 commits the mirror's lakefile with a git-require
    // pointing at upstream EigeniusLeanCommon. For in-image
    // dispatch we need an offline build, so rewrite that line to
    // point at the baked copy at `LEAN_COMMON_IN_IMAGE` before
    // running `lake build`. The single-line require form
    // (module_assembler::lakefile_content) keeps the sed
    // substitution one-shot — no multi-line awk gymnastics.
    //
    // `lake build` then compiles every `lean_lib` declared in the
    // mirror's lakefile (`EigeniusFFI.Basic` + `EigeniusFFI.Mirror`).
    // The resulting `.olean` files land under
    // `<MIRROR_IN_IMAGE>/.lake/build/lib/lean/` and are resolvable
    // by downstream user-side `LeanProject`s that `require
    // EigeniusFFI` against the mirror.
    let script = format!(
        "sed -i 's|^require EigeniusLeanCommon from git.*|require EigeniusLeanCommon from \"{LEAN_COMMON_IN_IMAGE}\"|' {MIRROR_IN_IMAGE}/lakefile.lean \
            && cd {MIRROR_IN_IMAGE} \
            && lake update --keep-toolchain \
            && lake build"
    );
    vec![format!("RUN {script}")]
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
    fn default_plan_registers_cdylib_and_builds_lean4export() {
        let f = lean_dockerfile_fragments(&LeanImagePlan::default());
        assert_eq!(f.install_packages.len(), 1);
        let line = &f.install_packages[0];
        // The worker binary's stale host-side DT_RUNPATH only gets
        // silently bypassed if ldconfig's cache carries WORKER_LIB_DIR.
        // A regression that drops the conf file or skips ldconfig
        // would surface as the worker failing to find its cdylib at
        // PID-1 startup — pin both.
        assert!(line.contains(LD_SO_CONF_PATH));
        assert!(line.contains(WORKER_LIB_DIR));
        assert!(line.contains("ldconfig"));
        // lean4export still needs to be pre-built so first dispatch
        // doesn't pay the ~5-10 s compile cost.
        assert!(line.contains(LEAN4EXPORT_IN_IMAGE));
        assert!(line.contains("lake build"));
    }

    #[test]
    fn install_packages_registers_cdylib_before_running_lake() {
        // ldconfig must run before any `lake build` step — Lake's
        // own compilation pipeline doesn't pull in our cdylib, but
        // ordering this way keeps the script readable: linker
        // setup first, build steps after.
        let f = lean_dockerfile_fragments(&LeanImagePlan::default());
        let line = &f.install_packages[0];
        let ldconfig_pos = line.find("ldconfig").expect("ldconfig in install_packages");
        let lake_build_pos = line
            .find("lake build")
            .expect("lake build in install_packages");
        assert!(
            ldconfig_pos < lake_build_pos,
            "ldconfig registration must precede lake build steps"
        );
    }

    #[test]
    fn install_mirror_empty_when_no_mirror_planned() {
        // No mirror → no install_mirror step. The substrate
        // composer also skips the `COPY mirror/` directive in this
        // case (gated on `has_mirror`); the dockerfile fragment
        // shape stays minimal for mirrorless deployments.
        let f = lean_dockerfile_fragments(&LeanImagePlan {
            include_mirror: false,
            handler_packages: Vec::new(),
        });
        assert!(f.install_mirror.is_empty());
    }

    #[test]
    fn install_mirror_rewrites_lakefile_and_lake_builds_when_mirror_planned() {
        // 20a.6.x: when a mirror is baked, the install_mirror step
        // (a) rewrites the chain-committed git-require to a
        // path-require pointing at the baked EigeniusLeanCommon
        // and (b) runs `lake build` so the mirror's .olean files
        // land in the image's layer cache. Both steps must appear
        // in the single RUN line (one layer per logical action,
        // matching the rest of the dockerfile fragment discipline).
        let f = lean_dockerfile_fragments(&LeanImagePlan {
            include_mirror: true,
            handler_packages: Vec::new(),
        });
        assert_eq!(
            f.install_mirror.len(),
            1,
            "single RUN line for layer-cache stability"
        );
        let line = &f.install_mirror[0];
        // sed substitution of the lakefile's require — anchors on
        // the line prefix so a stray match elsewhere in the file
        // doesn't get rewritten.
        assert!(line.contains("sed -i 's|^require EigeniusLeanCommon from git.*|"));
        assert!(line.contains(LEAN_COMMON_IN_IMAGE));
        assert!(line.contains(&format!("{MIRROR_IN_IMAGE}/lakefile.lean")));
        // Build invocation.
        assert!(line.contains(&format!("cd {MIRROR_IN_IMAGE}")));
        assert!(line.contains("lake build"));
        // sed precedes lake build — rewrite has to happen first or
        // lake will try to resolve the git require over the wire.
        let sed_pos = line.find("sed -i").expect("sed step");
        let build_pos = line.find("lake build").expect("lake build step");
        assert!(sed_pos < build_pos);
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
    fn bootstrap_command_runs_prebuilt_worker_binary() {
        let f = lean_dockerfile_fragments(&LeanImagePlan::default());
        // The worker binary is COPY'd in pre-built — bootstrap
        // invokes it directly rather than going through `lake exe`,
        // which would require an in-image build environment.
        assert_eq!(f.bootstrap_command, vec![WORKER_BIN_PATH.to_string()]);
    }
}
