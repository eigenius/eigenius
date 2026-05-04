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

//! Deterministic image-build pipeline (D26 §9.2).
//!
//! Three layers, in order of dependency:
//!
//! - [`dockerfile`] — pure composer: `(base, fragments, packages, mirror,
//!   language assets) -> Dockerfile string`. No I/O. Trivially testable.
//! - [`context`] — materialiser: write the build-context tempdir layout
//!   (Dockerfile + provenance + package trees + mirror + language assets)
//!   in deterministic order.
//! - [`builder`] — backend abstraction: [`ImageBuilder`] trait with
//!   [`BuildahImageBuilder`] as the production impl. Shells out to
//!   `buildah bud --timestamp 0 --layers --jobs 1 --format oci`.
//!
//! The build path is `buildah`-driven, never via the run-side container
//! client (D26 §9.2). Build and run are independent: this module produces
//! images deterministically; [`crate::spawner`] consumes them by digest.

pub mod builder;
pub mod context;
pub mod dockerfile;

pub use builder::{is_buildah_available, BuildahImageBuilder, ImageBuilder};
pub use context::{
    BuildContext, BuildContextSpec, LanguageAsset, MirrorMaterialization, PackageMaterialization,
};
pub use dockerfile::{compose_dockerfile, DockerfileSpec, IncludedPackage};
