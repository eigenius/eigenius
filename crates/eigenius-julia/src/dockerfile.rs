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

//! Dockerfile fragments emitted by `JuliaLanguageRuntime`.
//!
//! Composed by the substrate's image-build pipeline (D26 §9.2) into
//! a final Dockerfile. The fragments reference paths from
//! [`crate::conventions`] — keep the two in sync.

use crate::conventions::WORKER_PROJECT_DIR;
use crate::eigenius_common::COMMON_PACKAGE_NAME;
use eigenius_runtime_substrate::types::DockerfileFragments;

/// Path the substrate composer materialises included packages under
/// (`/opt/eigenius/packages/<name>/`). Mirrored from
/// `crates/runtime-substrate/src/image_build/dockerfile.rs` —
/// substrate-side change here means a doc cross-link update; we keep
/// the constant local so the fragments stay self-contained.
const PACKAGES_IN_IMAGE: &str = "/opt/eigenius/packages";

/// Path the substrate composer materialises a `RuntimePackageMirror`
/// archive under (`/opt/eigenius/mirror/`). Same caveat as above.
const MIRROR_IN_IMAGE: &str = "/opt/eigenius/mirror";

/// Inputs to [`julia_dockerfile_fragments`]. Lets the build path
/// control whether the env image installs the shared
/// `EigeniusJuliaCommon` helper package and the generated mirror —
/// composition matters because the substrate's Dockerfile composer
/// orders `install_packages` *before* the mirror COPY but *after* the
/// included-packages COPY (D26 §9.2 / `image_build::dockerfile`).
#[derive(Debug, Clone, Default)]
pub struct JuliaImagePlan {
    /// `true` when the substrate has materialised the
    /// `EigeniusJuliaCommon` package under
    /// `/opt/eigenius/packages/EigeniusJuliaCommon/`. The fragment
    /// adds a `Pkg.develop` call to wire it into the worker's project.
    pub include_common: bool,
    /// `true` when a `RuntimePackageMirror` archive has been
    /// materialised under `/opt/eigenius/mirror/`. The fragment adds a
    /// post-instantiate `Pkg.develop` + `Pkg.precompile` so the mirror
    /// is precompiled at image-build time, not at first dispatch.
    pub include_mirror: bool,
    /// Names of institution handler packages baked under
    /// `/opt/eigenius/packages/<name>/` alongside `EigeniusJuliaCommon`.
    /// Each gets a `Pkg.develop` call so its `[deps]` (e.g.
    /// `IntervalArithmetic.jl` for `EigeniusIntervals`) resolve
    /// into the worker's manifest at instantiate time. Order is
    /// preserved for deterministic dockerfile output.
    pub handler_packages: Vec<String>,
}

/// Dockerfile fragments for a Julia env image extending an upstream
/// `julia:1.x-bookworm` (or pinned-digest equivalent) base.
///
/// `install_runtime` is empty because the `julia:` base image already
/// ships the Julia binary.
///
/// `install_packages` always runs `Pkg.instantiate; Pkg.precompile`
/// against the worker's project after the language-asset COPY. With
/// `include_common = true` it first calls `Pkg.develop` for
/// `EigeniusJuliaCommon` so the package the generated mirror imports
/// is on the load path before precompile.
///
/// `install_mirror` is empty when `include_mirror = false`. With it
/// set, the fragment runs `Pkg.develop(path = "<mirror>")` and a
/// follow-up `Pkg.precompile()`. The substrate composer COPYs the
/// mirror under `/opt/eigenius/mirror/` *after* `install_packages`,
/// so the mirror's installation belongs in `install_mirror` rather
/// than `install_packages`.
pub fn julia_dockerfile_fragments(plan: &JuliaImagePlan) -> DockerfileFragments {
    DockerfileFragments {
        install_runtime: vec![],
        install_packages: install_packages_lines(plan),
        install_mirror: install_mirror_lines(plan),
        bootstrap_command: vec![
            "julia".to_string(),
            format!("--project={WORKER_PROJECT_DIR}"),
            format!("{WORKER_PROJECT_DIR}/src/JuliaWorker.jl"),
        ],
    }
}

fn install_packages_lines(plan: &JuliaImagePlan) -> Vec<String> {
    let common_path = format!("{PACKAGES_IN_IMAGE}/{COMMON_PACKAGE_NAME}");
    // Single RUN so layer cache stays predictable and Pkg state is
    // resolved once per build.
    let mut script = String::from("using Pkg; ");
    if plan.include_common {
        script.push_str(&format!("Pkg.develop(path=\"{common_path}\"); "));
    }
    // Handler packages typically depend on `EigeniusMirror` (the
    // generated mirror module is the typed input boundary). The
    // composer COPYs the mirror archive *after* `install_packages` and
    // before `install_mirror`, so when the mirror is in play the
    // handlers must wait for `install_mirror_lines` to develop them
    // — otherwise `Pkg.develop(handler)` resolves to an unsatisfiable
    // `EigeniusMirror` constraint. When there's no mirror (handlers
    // that import nothing mirror-shaped), the handler develop calls
    // move into this section so they get precompiled here.
    if !plan.include_mirror {
        for name in &plan.handler_packages {
            script.push_str(&format!(
                "Pkg.develop(path=\"{PACKAGES_IN_IMAGE}/{name}\"); "
            ));
        }
    }
    script.push_str("Pkg.instantiate(); Pkg.precompile()");
    vec![format!(
        "RUN JULIA_PKG_PRECOMPILE_AUTO=0 julia --project={WORKER_PROJECT_DIR} -e '{script}'"
    )]
}

fn install_mirror_lines(plan: &JuliaImagePlan) -> Vec<String> {
    if !plan.include_mirror {
        return vec![];
    }
    let mut script = String::from("using Pkg; ");
    script.push_str(&format!("Pkg.develop(path=\"{MIRROR_IN_IMAGE}\"); "));
    // Develop handler packages after the mirror so a handler's
    // `[deps] EigeniusMirror = "..."` resolves to the just-developed
    // path package. `Pkg.instantiate` then pulls in any registry
    // deps the handler declared (e.g. `IntervalArithmetic.jl`).
    for name in &plan.handler_packages {
        script.push_str(&format!(
            "Pkg.develop(path=\"{PACKAGES_IN_IMAGE}/{name}\"); "
        ));
    }
    script.push_str("Pkg.instantiate(); Pkg.precompile()");
    vec![format!(
        "RUN JULIA_PKG_PRECOMPILE_AUTO=0 julia --project={WORKER_PROJECT_DIR} -e '{script}'"
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_plan_omits_pkg_develop_calls() {
        let f = julia_dockerfile_fragments(&JuliaImagePlan::default());
        assert!(f.install_runtime.is_empty());
        assert!(f.install_mirror.is_empty());
        assert_eq!(f.install_packages.len(), 1);
        assert!(!f.install_packages[0].contains("Pkg.develop"));
        assert!(f.install_packages[0].contains("Pkg.instantiate"));
    }

    #[test]
    fn common_only_emits_one_develop_in_install_packages() {
        let f = julia_dockerfile_fragments(&JuliaImagePlan {
            include_common: true,
            include_mirror: false,
            handler_packages: Vec::new(),
        });
        assert!(f.install_mirror.is_empty());
        assert!(f.install_packages[0]
            .contains("Pkg.develop(path=\"/opt/eigenius/packages/EigeniusJuliaCommon\")"));
    }

    #[test]
    fn mirror_install_uses_install_mirror_section() {
        // The mirror COPY happens after install_packages in the
        // composer's ordering (D26 §9.2) — the mirror's Pkg.develop
        // must therefore live in install_mirror, not install_packages.
        let f = julia_dockerfile_fragments(&JuliaImagePlan {
            include_common: true,
            include_mirror: true,
            handler_packages: Vec::new(),
        });
        assert_eq!(f.install_mirror.len(), 1);
        assert!(f.install_mirror[0].contains("Pkg.develop(path=\"/opt/eigenius/mirror\")"));
        // install_packages never references the mirror path.
        assert!(!f.install_packages[0].contains("/opt/eigenius/mirror"));
    }

    #[test]
    fn handler_packages_without_mirror_develop_in_install_packages() {
        // No mirror in the plan → handler develop calls go in
        // `install_packages`. Pinning this case so handlers that
        // don't import a mirror (uncommon but valid) still get baked.
        let f = julia_dockerfile_fragments(&JuliaImagePlan {
            include_common: true,
            include_mirror: false,
            handler_packages: vec!["StandaloneHandler".to_string()],
        });
        let line = &f.install_packages[0];
        assert!(line.contains("Pkg.develop(path=\"/opt/eigenius/packages/EigeniusJuliaCommon\")"));
        assert!(line.contains("Pkg.develop(path=\"/opt/eigenius/packages/StandaloneHandler\")"));
        // Common comes before handler packages so handlers that
        // import it find the load path already populated.
        let common_pos = line.find("EigeniusJuliaCommon").unwrap();
        let handler_pos = line.find("StandaloneHandler").unwrap();
        assert!(common_pos < handler_pos);
        assert!(line.contains("Pkg.instantiate()"));
        // No mirror RUN line.
        assert!(f.install_mirror.is_empty());
    }

    #[test]
    fn handler_packages_with_mirror_develop_after_mirror_in_install_mirror() {
        // The composer COPYs the mirror archive *after*
        // `install_packages` and *before* `install_mirror`. Handler
        // packages that depend on `EigeniusMirror` (the typical case
        // — the mirror is the typed input boundary) must therefore
        // wait for `install_mirror` to develop them, otherwise
        // `Pkg.develop(handler)` resolves to an unsatisfiable
        // EigeniusMirror constraint at install_packages time.
        let f = julia_dockerfile_fragments(&JuliaImagePlan {
            include_common: true,
            include_mirror: true,
            handler_packages: vec!["EigeniusIntervals".to_string()],
        });
        let install_pkgs = &f.install_packages[0];
        // Common goes in install_packages.
        assert!(install_pkgs
            .contains("Pkg.develop(path=\"/opt/eigenius/packages/EigeniusJuliaCommon\")"));
        // Handler does NOT go in install_packages when a mirror is
        // present — it would fail to resolve.
        assert!(
            !install_pkgs.contains("EigeniusIntervals"),
            "handler develop must not appear in install_packages when a mirror is present"
        );

        // Both mirror.develop and handler.develop go in install_mirror,
        // mirror first.
        assert_eq!(f.install_mirror.len(), 1);
        let install_mirror = &f.install_mirror[0];
        let mirror_pos = install_mirror.find("/opt/eigenius/mirror\"").unwrap();
        let handler_pos = install_mirror
            .find("EigeniusIntervals\"")
            .expect("handler develop in install_mirror");
        assert!(
            mirror_pos < handler_pos,
            "mirror must develop before handler so EigeniusMirror is on the load path"
        );
        // Pkg.instantiate runs after both develops, picking up
        // registry deps the handler declared.
        assert!(install_mirror.contains("Pkg.instantiate()"));
    }

    #[test]
    fn handler_packages_listed_in_declaration_order() {
        let f = julia_dockerfile_fragments(&JuliaImagePlan {
            include_common: true,
            include_mirror: false,
            handler_packages: vec!["A".to_string(), "B".to_string(), "C".to_string()],
        });
        let line = &f.install_packages[0];
        let a = line.find("packages/A\"").unwrap();
        let b = line.find("packages/B\"").unwrap();
        let c = line.find("packages/C\"").unwrap();
        assert!(
            a < b && b < c,
            "handler packages must keep declaration order"
        );
    }

    #[test]
    fn bootstrap_command_runs_worker_jl() {
        let f = julia_dockerfile_fragments(&JuliaImagePlan::default());
        assert!(f
            .bootstrap_command
            .iter()
            .any(|s| s.contains("JuliaWorker.jl")));
    }
}
