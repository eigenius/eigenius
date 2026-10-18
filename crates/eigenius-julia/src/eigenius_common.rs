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

//! `EigeniusJuliaCommon` — hand-authored shared validation package that
//! the generated mirror modules `using`-import. The package's source
//! lives at `julia/common/EigeniusJuliaCommon/` in the workspace and is
//! baked into this crate's binary via `include_str!` so the substrate
//! has zero runtime dependency on the workspace layout when building
//! env images.

use eigenius_runtime_substrate::image_build::PackageMaterialization;
use std::path::PathBuf;

/// Package directory name and Julia module name. Must match the
/// `name = "..."` field in `julia/common/EigeniusJuliaCommon/Project.toml`.
pub const COMMON_PACKAGE_NAME: &str = "EigeniusJuliaCommon";

/// `Project.toml` for the hand-authored common package.
const COMMON_PROJECT_TOML: &str =
    include_str!("../../../julia/common/EigeniusJuliaCommon/Project.toml");

/// Module source for the common package.
const COMMON_MODULE_JL: &str =
    include_str!("../../../julia/common/EigeniusJuliaCommon/src/EigeniusJuliaCommon.jl");

/// Build a [`PackageMaterialization`] carrying the hand-authored
/// `EigeniusJuliaCommon` package's source. The substrate's image-build
/// pipeline writes this under `packages/EigeniusJuliaCommon/` in the
/// build context; the generated Dockerfile then `Pkg.develop`-installs
/// it into the worker's project.
pub fn package_materialization() -> PackageMaterialization {
    let mut mat = PackageMaterialization::default();
    mat.files
        .insert(PathBuf::from("Project.toml"), COMMON_PROJECT_TOML.into());
    mat.files.insert(
        PathBuf::from("src/EigeniusJuliaCommon.jl"),
        COMMON_MODULE_JL.into(),
    );
    mat
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_toml_advertises_expected_name_and_uuid() {
        // Sanity-check the baked-in source. If the upstream package's
        // metadata changes, the embedded copy + the generated mirror's
        // dep declaration drift apart silently — this test catches it.
        assert!(COMMON_PROJECT_TOML.contains("name = \"EigeniusJuliaCommon\""));
        assert!(COMMON_PROJECT_TOML.contains("uuid = \"9c8e7a4e-1f2b-4c3d-9e5f-6a7b8c9d0e1f\""));
    }

    #[test]
    fn module_source_exports_validators() {
        // The generated mirror's `using EigeniusJuliaCommon: validate_*`
        // must resolve at Julia load time. Verify the export list at
        // build time so a missing export surfaces here instead of
        // at worker boot.
        for name in [
            "validate_min_value",
            "validate_max_value",
            "validate_min_length",
            "validate_max_length",
            "validate_pattern",
            "validate_format",
        ] {
            assert!(
                COMMON_MODULE_JL.contains(name),
                "EigeniusJuliaCommon source must export `{name}`"
            );
        }
    }

    #[test]
    fn package_materialization_carries_both_files() {
        let mat = package_materialization();
        assert!(mat.files.contains_key(&PathBuf::from("Project.toml")));
        assert!(mat
            .files
            .contains_key(&PathBuf::from("src/EigeniusJuliaCommon.jl")));
    }
}
