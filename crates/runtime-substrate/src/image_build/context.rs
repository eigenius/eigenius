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

//! Build-context materialiser (D26 §9.2).
//!
//! Given a [`BuildContextSpec`] (provenance, package source trees, mirror
//! archive, language-supplied assets) and a Dockerfile string, write a
//! deterministic on-disk layout under a caller-supplied `work_dir`:
//!
//! ```text
//! work_dir/
//! ├── Dockerfile
//! ├── etc-eigenius-runtime-env/
//! │   ├── manifest-hash      ← lockfile content hash
//! │   ├── mirror-iri         ← IRI of the mirror baked in (or empty)
//! │   ├── included-pkgs      ← newline-separated package IRIs
//! │   └── built-at           ← caller-supplied stamp (deterministic input)
//! ├── packages/<pkg-name>/<file>...
//! ├── mirror/<file>...                   (only if a mirror was supplied)
//! └── language/<asset>...                (only if language assets supplied)
//! ```
//!
//! The materialiser is the dual of the [composer]: the composer emits
//! Dockerfile lines that reference these paths; the materialiser writes
//! the files at the same paths. The two are coupled through the layout
//! schema documented above — neither side carries it as data, both sides
//! enforce it in code.
//!
//! ## Determinism
//!
//! - `BTreeMap` is used for any iteration so disk-write order matches
//!   logical order regardless of caller iteration.
//! - `built_at` is a caller-supplied string: the substrate doesn't read
//!   the wall clock here, so byte-identical inputs produce byte-identical
//!   on-disk contents.
//! - File mtimes are *not* deterministic at write time — the OS sets
//!   them. Buildah's `--timestamp 0` flag normalises them at image-build
//!   time, which is the layer where determinism matters.
//!
//! [composer]: super::dockerfile::compose_dockerfile

use crate::error::BuildError;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Inputs to [`BuildContext::materialize`]. All fields are *content* —
/// no paths into existing directories, so the materialiser has full
/// control over the output layout.
#[derive(Debug, Clone, Default)]
pub struct BuildContextSpec {
    /// Composed Dockerfile (from [`super::dockerfile::compose_dockerfile`]).
    pub dockerfile: String,
    /// Lockfile content hash, written to `etc-eigenius-runtime-env/manifest-hash`.
    /// Cross-checked at worker startup against `EIGENIUS_RUNTIME_ENV_MANIFEST_HASH`
    /// (D26 §9.3).
    pub manifest_hash: String,
    /// IRI of the mirror baked into the image, written to
    /// `etc-eigenius-runtime-env/mirror-iri`. Empty string when no mirror.
    pub mirror_iri: String,
    /// IRIs of the included packages, written one-per-line to
    /// `etc-eigenius-runtime-env/included-pkgs`.
    pub included_pkg_iris: Vec<String>,
    /// Caller-supplied build stamp (e.g. `"1970-01-01T00:00:00Z"` or a
    /// hash-of-inputs string). Written to `etc-eigenius-runtime-env/built-at`.
    /// Caller must hand in a deterministic value if reproducible builds
    /// are required.
    pub built_at: String,
    /// Per-package source trees. Keys are package directory names
    /// (matching `IncludedPackage::name`); values are the file contents
    /// for that package.
    pub packages: BTreeMap<String, PackageMaterialization>,
    /// Mirror archive contents, materialised under `mirror/`.
    pub mirror: Option<MirrorMaterialization>,
    /// Language-supplied assets, materialised under `language/`.
    pub language_assets: Vec<LanguageAsset>,
}

/// File contents for a single included package, keyed by relative path
/// under `packages/<name>/`.
#[derive(Debug, Clone, Default)]
pub struct PackageMaterialization {
    pub files: BTreeMap<PathBuf, Vec<u8>>,
}

/// File contents for the mirror archive, keyed by relative path under
/// `mirror/`.
#[derive(Debug, Clone, Default)]
pub struct MirrorMaterialization {
    pub files: BTreeMap<PathBuf, Vec<u8>>,
}

/// A single file the language crate wants in the build context. The
/// materialiser writes it at `language/<source>`; the composer must have
/// emitted a matching `COPY language/<source> ...` line via
/// [`super::dockerfile::LanguageAssetCopy`].
#[derive(Debug, Clone)]
pub struct LanguageAsset {
    /// Path under `language/` in the build context.
    pub source: PathBuf,
    /// File contents.
    pub content: Vec<u8>,
    /// Optional explicit Unix mode (`0o755` etc.). Set when the asset
    /// must be executable in the image — Dockerfile `COPY` preserves
    /// the source-file mode, so worker binaries staged via this
    /// mechanism need their executable bit set in the build context.
    /// `None` falls back to whatever the platform's umask produces
    /// from `std::fs::write` (typically `0o644`).
    pub mode: Option<u32>,
}

/// A materialised build context — a `work_dir` populated with the layout
/// documented in the module header.
#[derive(Debug)]
pub struct BuildContext {
    work_dir: PathBuf,
}

impl BuildContext {
    /// Materialise `spec` under `work_dir`. The directory must already
    /// exist and be writable; the materialiser does not delete or
    /// recurse-clean it.
    pub fn materialize(work_dir: PathBuf, spec: &BuildContextSpec) -> Result<Self, BuildError> {
        if !work_dir.is_dir() {
            return Err(BuildError::EnvironmentBuildFailed(format!(
                "build context work_dir does not exist or is not a directory: {}",
                work_dir.display()
            )));
        }
        write_file(&work_dir.join("Dockerfile"), spec.dockerfile.as_bytes())?;
        write_provenance(&work_dir, spec)?;
        write_packages(&work_dir, &spec.packages)?;
        if let Some(mirror) = &spec.mirror {
            write_mirror(&work_dir, mirror)?;
        }
        write_language_assets(&work_dir, &spec.language_assets)?;
        Ok(Self { work_dir })
    }

    /// Path to the materialised build context — the directory passed as
    /// the build-context argument to `buildah`.
    pub fn work_dir(&self) -> &Path {
        &self.work_dir
    }
}

fn write_provenance(work_dir: &Path, spec: &BuildContextSpec) -> Result<(), BuildError> {
    let dir = work_dir.join("etc-eigenius-runtime-env");
    create_dir(&dir)?;
    write_file(&dir.join("manifest-hash"), spec.manifest_hash.as_bytes())?;
    write_file(&dir.join("mirror-iri"), spec.mirror_iri.as_bytes())?;
    let mut pkgs = spec.included_pkg_iris.clone();
    pkgs.sort();
    pkgs.dedup();
    let pkgs_body = pkgs.join("\n");
    let pkgs_body = if pkgs_body.is_empty() {
        String::new()
    } else {
        format!("{pkgs_body}\n")
    };
    write_file(&dir.join("included-pkgs"), pkgs_body.as_bytes())?;
    write_file(&dir.join("built-at"), spec.built_at.as_bytes())?;
    Ok(())
}

fn write_packages(
    work_dir: &Path,
    packages: &BTreeMap<String, PackageMaterialization>,
) -> Result<(), BuildError> {
    if packages.is_empty() {
        return Ok(());
    }
    let pkgs_root = work_dir.join("packages");
    create_dir(&pkgs_root)?;
    for (name, mat) in packages {
        let pkg_dir = pkgs_root.join(name);
        create_dir(&pkg_dir)?;
        write_files_under(&pkg_dir, &mat.files)?;
    }
    Ok(())
}

fn write_mirror(work_dir: &Path, mirror: &MirrorMaterialization) -> Result<(), BuildError> {
    let dir = work_dir.join("mirror");
    create_dir(&dir)?;
    write_files_under(&dir, &mirror.files)
}

fn write_language_assets(work_dir: &Path, assets: &[LanguageAsset]) -> Result<(), BuildError> {
    if assets.is_empty() {
        return Ok(());
    }
    let dir = work_dir.join("language");
    create_dir(&dir)?;
    let mut sorted: Vec<&LanguageAsset> = assets.iter().collect();
    sorted.sort_by(|a, b| a.source.cmp(&b.source));
    for asset in sorted {
        if asset.source.is_absolute()
            || asset.source.components().any(|c| {
                matches!(
                    c,
                    std::path::Component::ParentDir | std::path::Component::Prefix(_)
                )
            })
        {
            return Err(BuildError::EnvironmentBuildFailed(format!(
                "language asset source must be a relative path under language/, got `{}`",
                asset.source.display()
            )));
        }
        let dest = dir.join(&asset.source);
        if let Some(parent) = dest.parent() {
            create_dir(parent)?;
        }
        write_file(&dest, &asset.content)?;
        if let Some(mode) = asset.mode {
            apply_mode(&dest, mode)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn apply_mode(p: &Path, mode: u32) -> Result<(), BuildError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).map_err(|e| {
        BuildError::EnvironmentBuildFailed(format!(
            "failed to set mode {mode:o} on {}: {e}",
            p.display()
        ))
    })
}

// Non-Unix targets (Windows / WASI) currently can't be substrate
// build hosts — buildah requires a Linux host — so this branch is a
// no-op rather than a build error.
#[cfg(not(unix))]
fn apply_mode(_p: &Path, _mode: u32) -> Result<(), BuildError> {
    Ok(())
}

fn write_files_under(base: &Path, files: &BTreeMap<PathBuf, Vec<u8>>) -> Result<(), BuildError> {
    for (rel, content) in files {
        if rel.is_absolute()
            || rel.components().any(|c| {
                matches!(
                    c,
                    std::path::Component::ParentDir | std::path::Component::Prefix(_)
                )
            })
        {
            return Err(BuildError::EnvironmentBuildFailed(format!(
                "file path must be a relative path inside the build context, got `{}`",
                rel.display()
            )));
        }
        let dest = base.join(rel);
        if let Some(parent) = dest.parent() {
            create_dir(parent)?;
        }
        write_file(&dest, content)?;
    }
    Ok(())
}

fn create_dir(p: &Path) -> Result<(), BuildError> {
    std::fs::create_dir_all(p).map_err(|e| {
        BuildError::EnvironmentBuildFailed(format!(
            "failed to create build-context directory {}: {e}",
            p.display()
        ))
    })
}

fn write_file(p: &Path, content: &[u8]) -> Result<(), BuildError> {
    std::fs::write(p, content).map_err(|e| {
        BuildError::EnvironmentBuildFailed(format!(
            "failed to write build-context file {}: {e}",
            p.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn fresh_work_dir(label: &str) -> PathBuf {
        let pid = std::process::id();
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("substrate-build-{pid}-{label}-{n}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create work_dir");
        dir
    }

    fn read(p: &Path) -> Vec<u8> {
        std::fs::read(p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
    }

    fn read_str(p: &Path) -> String {
        String::from_utf8(read(p)).expect("utf8")
    }

    #[test]
    fn materialise_writes_dockerfile_and_provenance() {
        let work = fresh_work_dir("provenance");
        let spec = BuildContextSpec {
            dockerfile: "FROM scratch\n".into(),
            manifest_hash: "deadbeef".into(),
            mirror_iri: "urn:eigenius:mirror:m1".into(),
            included_pkg_iris: vec!["urn:eigenius:pkg:b".into(), "urn:eigenius:pkg:a".into()],
            built_at: "1970-01-01T00:00:00Z".into(),
            ..Default::default()
        };
        let ctx = BuildContext::materialize(work.clone(), &spec).expect("materialise");
        assert_eq!(ctx.work_dir(), work.as_path());
        assert_eq!(read_str(&work.join("Dockerfile")), "FROM scratch\n");
        let prov = work.join("etc-eigenius-runtime-env");
        assert_eq!(read_str(&prov.join("manifest-hash")), "deadbeef");
        assert_eq!(read_str(&prov.join("mirror-iri")), "urn:eigenius:mirror:m1");
        assert_eq!(read_str(&prov.join("built-at")), "1970-01-01T00:00:00Z");
        // included-pkgs sorted, newline-terminated.
        assert_eq!(
            read_str(&prov.join("included-pkgs")),
            "urn:eigenius:pkg:a\nurn:eigenius:pkg:b\n"
        );
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn empty_pkg_list_writes_empty_provenance_file() {
        let work = fresh_work_dir("empty-pkgs");
        let spec = BuildContextSpec {
            dockerfile: "FROM scratch\n".into(),
            built_at: "stamp".into(),
            ..Default::default()
        };
        BuildContext::materialize(work.clone(), &spec).expect("materialise");
        let body = read_str(&work.join("etc-eigenius-runtime-env").join("included-pkgs"));
        assert!(body.is_empty());
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn writes_packages_under_packages_dir() {
        let work = fresh_work_dir("packages");
        let mut foo = PackageMaterialization::default();
        foo.files
            .insert(PathBuf::from("Project.toml"), b"name = \"Foo\"\n".to_vec());
        foo.files
            .insert(PathBuf::from("src/Foo.jl"), b"module Foo end\n".to_vec());
        let mut packages = BTreeMap::new();
        packages.insert("Foo".to_string(), foo);
        let spec = BuildContextSpec {
            dockerfile: "FROM scratch\n".into(),
            built_at: "stamp".into(),
            packages,
            ..Default::default()
        };
        BuildContext::materialize(work.clone(), &spec).expect("materialise");
        assert_eq!(
            read_str(&work.join("packages/Foo/Project.toml")),
            "name = \"Foo\"\n"
        );
        assert_eq!(
            read_str(&work.join("packages/Foo/src/Foo.jl")),
            "module Foo end\n"
        );
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn writes_mirror_only_when_supplied() {
        let work_with = fresh_work_dir("mirror-with");
        let work_without = fresh_work_dir("mirror-without");
        let mut mirror = MirrorMaterialization::default();
        mirror
            .files
            .insert(PathBuf::from("Mirror.jl"), b"# mirror".to_vec());
        BuildContext::materialize(
            work_with.clone(),
            &BuildContextSpec {
                dockerfile: "FROM scratch\n".into(),
                built_at: "stamp".into(),
                mirror: Some(mirror),
                ..Default::default()
            },
        )
        .expect("materialise");
        BuildContext::materialize(
            work_without.clone(),
            &BuildContextSpec {
                dockerfile: "FROM scratch\n".into(),
                built_at: "stamp".into(),
                ..Default::default()
            },
        )
        .expect("materialise");
        assert!(work_with.join("mirror/Mirror.jl").exists());
        assert!(!work_without.join("mirror").exists());
        let _ = std::fs::remove_dir_all(&work_with);
        let _ = std::fs::remove_dir_all(&work_without);
    }

    #[test]
    fn writes_language_assets_under_language_dir() {
        let work = fresh_work_dir("language");
        let assets = vec![LanguageAsset {
            source: PathBuf::from("subdir/JuliaWorker.jl"),
            content: b"# worker".to_vec(),
            mode: None,
        }];
        BuildContext::materialize(
            work.clone(),
            &BuildContextSpec {
                dockerfile: "FROM scratch\n".into(),
                built_at: "stamp".into(),
                language_assets: assets,
                ..Default::default()
            },
        )
        .expect("materialise");
        assert_eq!(
            read_str(&work.join("language/subdir/JuliaWorker.jl")),
            "# worker"
        );
        let _ = std::fs::remove_dir_all(&work);
    }

    #[cfg(unix)]
    #[test]
    fn applies_explicit_mode_to_language_asset() {
        use std::os::unix::fs::PermissionsExt;
        let work = fresh_work_dir("language-mode");
        let assets = vec![LanguageAsset {
            source: PathBuf::from("worker"),
            content: b"#!/bin/sh\necho ok\n".to_vec(),
            mode: Some(0o755),
        }];
        BuildContext::materialize(
            work.clone(),
            &BuildContextSpec {
                dockerfile: "FROM scratch\n".into(),
                built_at: "stamp".into(),
                language_assets: assets,
                ..Default::default()
            },
        )
        .expect("materialise");
        let meta = std::fs::metadata(work.join("language/worker")).expect("stat");
        assert_eq!(
            meta.permissions().mode() & 0o7777,
            0o755,
            "language asset mode must be applied verbatim",
        );
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn rejects_absolute_or_traversing_paths() {
        let work = fresh_work_dir("traversal");
        let mut foo = PackageMaterialization::default();
        foo.files.insert(PathBuf::from("../escape"), b"x".to_vec());
        let mut packages = BTreeMap::new();
        packages.insert("Foo".to_string(), foo);
        let err = BuildContext::materialize(
            work.clone(),
            &BuildContextSpec {
                dockerfile: "FROM scratch\n".into(),
                built_at: "stamp".into(),
                packages,
                ..Default::default()
            },
        )
        .expect_err("must reject ..");
        match err {
            BuildError::EnvironmentBuildFailed(msg) => {
                assert!(msg.contains("relative path"), "got: {msg}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn errors_when_work_dir_does_not_exist() {
        let missing = std::env::temp_dir().join("substrate-build-does-not-exist-xyz-1234");
        let _ = std::fs::remove_dir_all(&missing);
        let err = BuildContext::materialize(
            missing,
            &BuildContextSpec {
                dockerfile: "FROM scratch\n".into(),
                built_at: "stamp".into(),
                ..Default::default()
            },
        )
        .expect_err("must reject missing work_dir");
        assert!(matches!(err, BuildError::EnvironmentBuildFailed(_)));
    }
}
