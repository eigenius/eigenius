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

//! Pure Dockerfile composer for the substrate's image-build pipeline
//! (D26 §9.2).
//!
//! [`compose_dockerfile`] is a referentially transparent function from a
//! [`DockerfileSpec`] to a Dockerfile string — no I/O, no clocks, no
//! environment. The output is byte-identical for byte-identical input,
//! which is the load-bearing precondition for deterministic image builds.
//!
//! ## Section ordering
//!
//! The composer always emits sections in the same order, regardless of
//! which fragments the language crate populates:
//!
//! 1. `FROM <base_image_ref>`
//! 2. `install_runtime` lines
//! 3. `COPY language/<asset> <dest>` lines (when [`DockerfileSpec`]
//!    declares language assets)
//! 4. `COPY packages/<name>/ /opt/eigenius/packages/<name>/` for each
//!    `IncludedPackage`, name-sorted for determinism
//! 5. `install_packages` lines
//! 6. `COPY mirror/ /opt/eigenius/mirror/` (only if [`DockerfileSpec::has_mirror`])
//! 7. `install_mirror` lines
//! 8. `COPY etc-eigenius-runtime-env/ /etc/eigenius-runtime-env/` —
//!    substrate-managed in-image provenance (D26 §9.2 / §9.3)
//! 9. `CMD ["exe", "arg1", ...]` from `bootstrap_command` (exec form,
//!    so signals reach the worker as PID 1)
//!
//! The composer enforces ordering even when language fragments would
//! prefer to interleave — interleaving across the build-context layout
//! breaks layer caching and would force the materialiser and composer to
//! agree on a richer schema. The fixed ordering is the schema.

use crate::types::DockerfileFragments;
use std::path::PathBuf;

/// Inputs to [`compose_dockerfile`]. Borrowing — the composer copies
/// nothing.
#[derive(Debug, Clone)]
pub struct DockerfileSpec<'a> {
    /// Base image reference, e.g. `julia:1.10-bookworm@sha256:<digest>`.
    /// Pinned by digest in production for reproducibility; tag-only is
    /// acceptable in dev.
    pub base_image_ref: &'a str,
    /// Per-language fragments composed into the relevant sections.
    pub fragments: &'a DockerfileFragments,
    /// Packages whose source trees are materialised under
    /// `packages/<name>/` in the build context.
    pub included_packages: &'a [IncludedPackage],
    /// `true` when a `RuntimePackageMirror` archive is materialised under
    /// `mirror/` in the build context. The composer emits the
    /// corresponding COPY only when this is set.
    pub has_mirror: bool,
    /// Language-supplied assets to COPY into the image. Each tuple is
    /// `(path_under_language_dir_in_context, destination_in_image)`. The
    /// materialiser writes the file at `language/<source>`; the composer
    /// emits `COPY language/<source> <destination>`. Sorted by source
    /// path for determinism.
    pub language_asset_copies: &'a [LanguageAssetCopy],
}

/// A package whose source tree is baked into the image's
/// `/opt/eigenius/packages/<name>/` directory by the substrate-managed
/// COPY emitted from [`compose_dockerfile`].
#[derive(Debug, Clone)]
pub struct IncludedPackage {
    /// Directory name under `packages/` in the build context — also the
    /// destination under `/opt/eigenius/packages/` in the image.
    pub name: String,
}

/// A language-supplied asset COPY directive. Decoupled from
/// [`crate::image_build::context::LanguageAsset`] (which carries content)
/// because the composer doesn't need bytes — only the source path under
/// `language/` in the build context and the destination inside the image.
#[derive(Debug, Clone)]
pub struct LanguageAssetCopy {
    /// Path under `language/` in the build context. `language/JuliaWorker.jl`
    /// is `PathBuf::from("JuliaWorker.jl")` here.
    pub source: PathBuf,
    /// Destination path inside the image, e.g. `/opt/eigenius/JuliaWorker.jl`.
    pub destination: String,
}

/// Compose a deterministic Dockerfile from the spec.
///
/// Output starts with a substrate-managed header comment so a reader of
/// the materialised Dockerfile can tell at a glance that it is generated
/// rather than hand-edited.
pub fn compose_dockerfile(spec: &DockerfileSpec) -> String {
    let mut out = String::new();
    push_header(&mut out);
    push_from(&mut out, spec.base_image_ref);
    push_section(&mut out, "install_runtime", &spec.fragments.install_runtime);
    push_language_assets(&mut out, spec.language_asset_copies);
    push_package_copies(&mut out, spec.included_packages);
    push_section(
        &mut out,
        "install_packages",
        &spec.fragments.install_packages,
    );
    push_mirror_copy(&mut out, spec.has_mirror);
    push_section(&mut out, "install_mirror", &spec.fragments.install_mirror);
    push_provenance_copy(&mut out);
    push_cmd(&mut out, &spec.fragments.bootstrap_command);
    out
}

fn push_header(out: &mut String) {
    out.push_str("# Generated by Eigenius Runtime Substrate (D26 §9.2).\n");
    out.push_str("# Deterministic build: do not hand-edit.\n\n");
}

fn push_from(out: &mut String, base: &str) {
    out.push_str("FROM ");
    out.push_str(base);
    out.push_str("\n\n");
}

fn push_section(out: &mut String, label: &str, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    out.push_str("# ");
    out.push_str(label);
    out.push('\n');
    for line in lines {
        out.push_str(line);
        if !line.ends_with('\n') {
            out.push('\n');
        }
    }
    out.push('\n');
}

fn push_language_assets(out: &mut String, copies: &[LanguageAssetCopy]) {
    if copies.is_empty() {
        return;
    }
    let mut sorted: Vec<&LanguageAssetCopy> = copies.iter().collect();
    sorted.sort_by(|a, b| a.source.cmp(&b.source));
    out.push_str("# language assets\n");
    for c in sorted {
        out.push_str("COPY language/");
        out.push_str(&path_to_posix(&c.source));
        out.push(' ');
        out.push_str(&c.destination);
        out.push('\n');
    }
    out.push('\n');
}

fn push_package_copies(out: &mut String, packages: &[IncludedPackage]) {
    if packages.is_empty() {
        return;
    }
    let mut sorted: Vec<&IncludedPackage> = packages.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    out.push_str("# included_packages\n");
    for p in sorted {
        out.push_str("COPY packages/");
        out.push_str(&p.name);
        out.push_str("/ /opt/eigenius/packages/");
        out.push_str(&p.name);
        out.push_str("/\n");
    }
    out.push('\n');
}

fn push_mirror_copy(out: &mut String, has_mirror: bool) {
    if !has_mirror {
        return;
    }
    out.push_str("# mirror\n");
    out.push_str("COPY mirror/ /opt/eigenius/mirror/\n\n");
}

fn push_provenance_copy(out: &mut String) {
    out.push_str("# in-image provenance (D26 §9.2 / §9.3)\n");
    out.push_str("COPY etc-eigenius-runtime-env/ /etc/eigenius-runtime-env/\n\n");
}

fn push_cmd(out: &mut String, command: &[String]) {
    out.push_str("CMD [");
    for (i, arg) in command.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push('"');
        for ch in arg.chars() {
            match ch {
                '\\' => out.push_str("\\\\"),
                '"' => out.push_str("\\\""),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c => out.push(c),
            }
        }
        out.push('"');
    }
    out.push_str("]\n");
}

/// Render a relative path as POSIX (forward-slash separated) for Dockerfile
/// COPY directives. Build contexts are POSIX-style on every platform.
fn path_to_posix(p: &std::path::Path) -> String {
    let mut s = String::new();
    let mut first = true;
    for comp in p.components() {
        use std::path::Component;
        let part = match comp {
            Component::Normal(os) => os.to_string_lossy().into_owned(),
            Component::CurDir => continue,
            Component::ParentDir => "..".to_string(),
            Component::RootDir => String::new(),
            Component::Prefix(_) => String::new(),
        };
        if first {
            first = false;
        } else {
            s.push('/');
        }
        s.push_str(&part);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn empty_fragments() -> DockerfileFragments {
        DockerfileFragments {
            install_runtime: vec![],
            install_packages: vec![],
            install_mirror: vec![],
            bootstrap_command: vec!["sleep".into(), "infinity".into()],
        }
    }

    #[test]
    fn emits_minimal_dockerfile_with_only_required_sections() {
        let fragments = empty_fragments();
        let out = compose_dockerfile(&DockerfileSpec {
            base_image_ref: "julia:1.10-bookworm",
            fragments: &fragments,
            included_packages: &[],
            has_mirror: false,
            language_asset_copies: &[],
        });
        assert!(out.starts_with("# Generated by Eigenius Runtime Substrate"));
        assert!(out.contains("FROM julia:1.10-bookworm\n"));
        assert!(out.contains("COPY etc-eigenius-runtime-env/ /etc/eigenius-runtime-env/\n"));
        assert!(out.trim_end().ends_with(r#"CMD ["sleep", "infinity"]"#));
        // No package or mirror noise when both are absent.
        assert!(!out.contains("COPY packages/"));
        assert!(!out.contains("COPY mirror/"));
        assert!(!out.contains("COPY language/"));
    }

    #[test]
    fn output_is_deterministic_under_input_reordering() {
        let fragments = empty_fragments();
        let pkgs_a = [
            IncludedPackage {
                name: "Beta".into(),
            },
            IncludedPackage {
                name: "Alpha".into(),
            },
        ];
        let pkgs_b = [
            IncludedPackage {
                name: "Alpha".into(),
            },
            IncludedPackage {
                name: "Beta".into(),
            },
        ];
        let assets_a = [
            LanguageAssetCopy {
                source: PathBuf::from("zeta.jl"),
                destination: "/opt/eigenius/zeta.jl".into(),
            },
            LanguageAssetCopy {
                source: PathBuf::from("alpha.jl"),
                destination: "/opt/eigenius/alpha.jl".into(),
            },
        ];
        let assets_b = [
            LanguageAssetCopy {
                source: PathBuf::from("alpha.jl"),
                destination: "/opt/eigenius/alpha.jl".into(),
            },
            LanguageAssetCopy {
                source: PathBuf::from("zeta.jl"),
                destination: "/opt/eigenius/zeta.jl".into(),
            },
        ];
        let a = compose_dockerfile(&DockerfileSpec {
            base_image_ref: "scratch",
            fragments: &fragments,
            included_packages: &pkgs_a,
            has_mirror: true,
            language_asset_copies: &assets_a,
        });
        let b = compose_dockerfile(&DockerfileSpec {
            base_image_ref: "scratch",
            fragments: &fragments,
            included_packages: &pkgs_b,
            has_mirror: true,
            language_asset_copies: &assets_b,
        });
        assert_eq!(a, b, "composer must sort packages and assets");
    }

    #[test]
    fn package_copies_appear_only_when_packages_present() {
        let fragments = empty_fragments();
        let pkgs = [IncludedPackage { name: "Foo".into() }];
        let out = compose_dockerfile(&DockerfileSpec {
            base_image_ref: "scratch",
            fragments: &fragments,
            included_packages: &pkgs,
            has_mirror: false,
            language_asset_copies: &[],
        });
        assert!(out.contains("COPY packages/Foo/ /opt/eigenius/packages/Foo/"));
    }

    #[test]
    fn mirror_copy_is_gated_on_has_mirror() {
        let fragments = empty_fragments();
        let with_mirror = compose_dockerfile(&DockerfileSpec {
            base_image_ref: "scratch",
            fragments: &fragments,
            included_packages: &[],
            has_mirror: true,
            language_asset_copies: &[],
        });
        let without_mirror = compose_dockerfile(&DockerfileSpec {
            base_image_ref: "scratch",
            fragments: &fragments,
            included_packages: &[],
            has_mirror: false,
            language_asset_copies: &[],
        });
        assert!(with_mirror.contains("COPY mirror/ /opt/eigenius/mirror/"));
        assert!(!without_mirror.contains("COPY mirror/"));
    }

    #[test]
    fn fragment_sections_emit_in_the_documented_order() {
        let fragments = DockerfileFragments {
            install_runtime: vec!["RUN echo runtime".into()],
            install_packages: vec!["RUN echo packages".into()],
            install_mirror: vec!["RUN echo mirror".into()],
            bootstrap_command: vec!["worker".into()],
        };
        let out = compose_dockerfile(&DockerfileSpec {
            base_image_ref: "scratch",
            fragments: &fragments,
            included_packages: &[],
            has_mirror: true,
            language_asset_copies: &[],
        });
        let pos_runtime = out.find("RUN echo runtime").expect("install_runtime");
        let pos_packages = out.find("RUN echo packages").expect("install_packages");
        let pos_mirror = out.find("RUN echo mirror").expect("install_mirror");
        let pos_provenance = out
            .find("COPY etc-eigenius-runtime-env/")
            .expect("provenance");
        let pos_cmd = out.find("CMD [").expect("CMD");
        assert!(pos_runtime < pos_packages);
        assert!(pos_packages < pos_mirror);
        assert!(pos_mirror < pos_provenance);
        assert!(pos_provenance < pos_cmd);
    }

    #[test]
    fn cmd_uses_exec_form_with_proper_json_escaping() {
        let fragments = DockerfileFragments {
            bootstrap_command: vec!["bin".into(), "arg \"with\" quotes".into()],
            ..Default::default()
        };
        let out = compose_dockerfile(&DockerfileSpec {
            base_image_ref: "scratch",
            fragments: &fragments,
            included_packages: &[],
            has_mirror: false,
            language_asset_copies: &[],
        });
        assert!(out.contains(r#"CMD ["bin", "arg \"with\" quotes"]"#));
    }

    #[test]
    fn empty_fragment_sections_emit_no_label() {
        let fragments = empty_fragments();
        let out = compose_dockerfile(&DockerfileSpec {
            base_image_ref: "scratch",
            fragments: &fragments,
            included_packages: &[],
            has_mirror: false,
            language_asset_copies: &[],
        });
        assert!(!out.contains("# install_runtime"));
        assert!(!out.contains("# install_packages"));
        assert!(!out.contains("# install_mirror"));
    }

    #[test]
    fn language_asset_copy_uses_relative_path_under_language_dir() {
        let fragments = empty_fragments();
        let assets = [LanguageAssetCopy {
            source: PathBuf::from("subdir/JuliaWorker.jl"),
            destination: "/opt/eigenius/JuliaWorker.jl".into(),
        }];
        let out = compose_dockerfile(&DockerfileSpec {
            base_image_ref: "scratch",
            fragments: &fragments,
            included_packages: &[],
            has_mirror: false,
            language_asset_copies: &assets,
        });
        assert!(out.contains("COPY language/subdir/JuliaWorker.jl /opt/eigenius/JuliaWorker.jl\n"));
    }
}
