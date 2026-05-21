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

//! `LeanProject` Eigon-CBOR resource → staged directory layout.
//!
//! On a `DispatchMethod{function_name="lean_export"}` invocation,
//! the substrate ships a `LeanProject` resource as `input[0]`. The
//! worker's responsibility is to materialise that resource as a
//! real Lake project tree on disk so `lake exe lean4export` can run
//! against it. This module does the materialisation; the Lean side
//! (`Worker.Main.runLeanExport`) drives it via the
//! [`ei_lean_worker_stage_lean_project`](`crate::lean_ffi::ei_lean_worker_stage_lean_project`)
//! FFI symbol.
//!
//! ## Expected resource shape
//!
//! A `LeanProject` carries (per
//! [`ontologies/lean/lean-runtime-classes.eigon.json`](../../../ontologies/lean/lean-runtime-classes.eigon.json)):
//!
//! - `urn:eigenius:lean:lakefile` (string) — `lakefile.toml` /
//!   `lakefile.lean` content
//! - `urn:eigenius:lean:lake_manifest` (string) —
//!   `lake-manifest.json` content
//! - `urn:eigenius:runtime:source_tree` (JSON array) — list of
//!   `{path: string, content_base64: string}` records, the
//!   project's Lean sources
//! - `urn:eigenius:lean:lean_toolchain` (string, optional) —
//!   contents of `lean-toolchain`; falls back to a worker default
//!   if absent
//!
//! Missing required properties surface as
//! [`StagingError::MissingProperty`]; malformed values surface as
//! [`StagingError::MalformedProperty`].

use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};

/// Property IRIs the staging helper reads. Duplicated from
/// [`eigenius_lean_runtime::conventions`](../../eigenius-lean-runtime/src/conventions.rs)
/// to avoid pulling that crate's full dep tree into the worker
/// cdylib — the constants are stable string literals and the
/// duplication cost is one-line-per-property.
mod prop_iri {
    pub const LAKEFILE: &str = "urn:eigenius:lean:lakefile";
    pub const LAKE_MANIFEST: &str = "urn:eigenius:lean:lake_manifest";
    pub const SOURCE_TREE: &str = "urn:eigenius:runtime:source_tree";
    pub const LEAN_TOOLCHAIN: &str = "urn:eigenius:lean:lean_toolchain";
}

/// Failure modes for [`stage_lean_project`]. Surfaced through the
/// FFI as a non-zero return code; the Lean side maps the code to a
/// `DispatchFailed` diagnostic.
#[derive(Debug, thiserror::Error)]
pub enum StagingError {
    /// The input bytes don't decode as an Eigon-CBOR resource.
    #[error("LeanProject CBOR decode failed: {0}")]
    CborDecode(String),

    /// The decoded resource is missing a required property.
    #[error("LeanProject missing required property `{0}`")]
    MissingProperty(&'static str),

    /// A property's value doesn't have the expected shape (string,
    /// JSON array, etc.).
    #[error("LeanProject property `{property}` malformed: {reason}")]
    MalformedProperty {
        property: &'static str,
        reason: String,
    },

    /// I/O failure during staging (couldn't create directory, write
    /// file, etc.).
    #[error("filesystem error at `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// A `source_tree` entry's `path` escapes the staging directory
    /// (contains `..` or is absolute). We reject these to keep the
    /// worker from being tricked into writing outside its sandbox.
    #[error("source_tree entry `{0}` escapes the staging directory (absolute path or `..`)")]
    UnsafePath(String),
}

/// Default `lean-toolchain` content when the LeanProject doesn't
/// supply one. Matches the worker's own toolchain pin so `lake`
/// invocations against the staged project don't trigger a fresh
/// toolchain install.
const DEFAULT_LEAN_TOOLCHAIN: &str = "leanprover/lean4:v4.29.1\n";

/// Decode `cbor_bytes` as a [`LeanProject`] Eigon-CBOR resource and
/// stage its files under `dest_dir`. Creates `dest_dir` if missing.
///
/// Files written:
/// - `dest_dir/lakefile.toml` (or `lakefile.lean` — we use `.toml`
///   by default; the LeanProject's content determines which Lake
///   reads)
/// - `dest_dir/lake-manifest.json`
/// - `dest_dir/lean-toolchain`
/// - For each `source_tree` entry: `dest_dir/<path>` after
///   base64-decoding `content_base64`
///
/// The choice between `lakefile.toml` and `lakefile.lean` is made
/// by sniffing the `lakefile` string: if it parses as TOML (starts
/// with `name =`), we write `lakefile.toml`; otherwise
/// `lakefile.lean`. This sniff matches Lake's auto-detection — a
/// project may use either format and Lake reads whichever exists.
pub fn stage_lean_project(cbor_bytes: &[u8], dest_dir: &Path) -> Result<(), StagingError> {
    let resource: Resource = eigon_cbor::parse_resource_lenient(cbor_bytes)
        .map_err(|e| StagingError::CborDecode(format!("{e}")))?;

    let lakefile = string_property(&resource, prop_iri::LAKEFILE)?;
    let lake_manifest = string_property(&resource, prop_iri::LAKE_MANIFEST)?;
    let lean_toolchain = optional_string_property(&resource, prop_iri::LEAN_TOOLCHAIN)
        .unwrap_or_else(|| DEFAULT_LEAN_TOOLCHAIN.to_string());
    let source_tree = source_tree_property(&resource)?;

    fs::create_dir_all(dest_dir).map_err(|e| StagingError::Io {
        path: dest_dir.display().to_string(),
        source: e,
    })?;

    let lakefile_path = if lakefile.trim_start().starts_with("name =") {
        dest_dir.join("lakefile.toml")
    } else {
        dest_dir.join("lakefile.lean")
    };
    write_file(&lakefile_path, lakefile.as_bytes())?;
    write_file(
        &dest_dir.join("lake-manifest.json"),
        lake_manifest.as_bytes(),
    )?;
    write_file(&dest_dir.join("lean-toolchain"), lean_toolchain.as_bytes())?;

    for entry in source_tree {
        let staged = safe_join(dest_dir, &entry.path)?;
        if let Some(parent) = staged.parent() {
            fs::create_dir_all(parent).map_err(|e| StagingError::Io {
                path: parent.display().to_string(),
                source: e,
            })?;
        }
        write_file(&staged, &entry.content)?;
    }

    Ok(())
}

/// One `source_tree` entry after base64 decoding.
struct SourceEntry {
    path: String,
    content: Vec<u8>,
}

/// Extract a string-valued property from a resource. Required
/// shape: `Value::String(s)`.
fn string_property(resource: &Resource, iri_str: &'static str) -> Result<String, StagingError> {
    let iri = parse_iri(iri_str)?;
    match resource.get(&iri) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(other) => Err(StagingError::MalformedProperty {
            property: iri_str,
            reason: format!("expected string, got {other:?}"),
        }),
        None => Err(StagingError::MissingProperty(iri_str)),
    }
}

/// Same as [`string_property`] but returns `None` for absent
/// properties instead of erroring.
fn optional_string_property(resource: &Resource, iri_str: &'static str) -> Option<String> {
    let iri = Iri::parse(iri_str).ok()?;
    match resource.get(&iri)? {
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// Extract `source_tree` — a JSON array of `{path, content_base64}`
/// records. The property is stored as `Value::Json(serde_json::Value)`
/// per the runtime ontology declaration.
fn source_tree_property(resource: &Resource) -> Result<Vec<SourceEntry>, StagingError> {
    let iri = parse_iri(prop_iri::SOURCE_TREE)?;
    let value = match resource.get(&iri) {
        Some(Value::Json(j)) => j,
        Some(other) => {
            return Err(StagingError::MalformedProperty {
                property: prop_iri::SOURCE_TREE,
                reason: format!("expected JSON array, got {other:?}"),
            });
        }
        // An absent source_tree is fine — a project may consist
        // entirely of the lakefile + transitive Lake-resolved
        // dependencies, with no user-authored source files.
        None => return Ok(Vec::new()),
    };

    let arr = value
        .as_array()
        .ok_or_else(|| StagingError::MalformedProperty {
            property: prop_iri::SOURCE_TREE,
            reason: "expected JSON array".to_string(),
        })?;

    let mut out = Vec::with_capacity(arr.len());
    for (i, item) in arr.iter().enumerate() {
        let obj = item
            .as_object()
            .ok_or_else(|| StagingError::MalformedProperty {
                property: prop_iri::SOURCE_TREE,
                reason: format!("entry [{i}] is not a JSON object"),
            })?;
        let path = obj
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| StagingError::MalformedProperty {
                property: prop_iri::SOURCE_TREE,
                reason: format!("entry [{i}] missing string `path`"),
            })?
            .to_string();
        let content_b64 = obj
            .get("content_base64")
            .and_then(|v| v.as_str())
            .ok_or_else(|| StagingError::MalformedProperty {
                property: prop_iri::SOURCE_TREE,
                reason: format!("entry [{i}] missing string `content_base64`"),
            })?;
        let content = base64::engine::general_purpose::STANDARD
            .decode(content_b64)
            .map_err(|e| StagingError::MalformedProperty {
                property: prop_iri::SOURCE_TREE,
                reason: format!("entry [{i}] base64 decode failed: {e}"),
            })?;
        out.push(SourceEntry { path, content });
    }
    Ok(out)
}

fn parse_iri(s: &'static str) -> Result<Iri, StagingError> {
    Iri::parse(s).map_err(|e| StagingError::MalformedProperty {
        property: s,
        reason: format!("static IRI `{s}` failed to parse: {e:?}"),
    })
}

/// Join a relative path into `base`, refusing if the result would
/// escape `base` (absolute path, `..` traversal, etc.).
fn safe_join(base: &Path, relative: &str) -> Result<PathBuf, StagingError> {
    let rel = Path::new(relative);
    if rel.is_absolute() {
        return Err(StagingError::UnsafePath(relative.to_string()));
    }
    for component in rel.components() {
        use std::path::Component;
        match component {
            Component::Normal(_) => {}
            Component::CurDir => {}
            _ => return Err(StagingError::UnsafePath(relative.to_string())),
        }
    }
    Ok(base.join(rel))
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), StagingError> {
    fs::write(path, bytes).map_err(|e| StagingError::Io {
        path: path.display().to_string(),
        source: e,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use eigenius_kernel::ontology::eigon_cbor::serialize_resource;
    use eigenius_kernel::ontology::iri::Iri;
    use eigenius_kernel::ontology::resource::{Resource, Value};

    /// Build a minimal `LeanProject` resource for the staging tests.
    fn make_project_resource(
        lakefile: &str,
        manifest: &str,
        source_tree: serde_json::Value,
    ) -> Resource {
        let mut r = Resource::new(Iri::parse("urn:eigenius:test:project1").unwrap());
        r.set(
            Iri::parse("urn:eigenius:core:is_a").unwrap(),
            Value::Array(vec![Value::ResourceRef(
                Iri::parse("urn:eigenius:lean:LeanProject").unwrap(),
            )]),
        );
        r.set(
            Iri::parse(prop_iri::LAKEFILE).unwrap(),
            Value::String(lakefile.to_string()),
        );
        r.set(
            Iri::parse(prop_iri::LAKE_MANIFEST).unwrap(),
            Value::String(manifest.to_string()),
        );
        r.set(
            Iri::parse(prop_iri::SOURCE_TREE).unwrap(),
            Value::Json(source_tree),
        );
        r
    }

    #[test]
    fn stages_minimal_lakefile_only_project() {
        let resource = make_project_resource(
            "name = \"TestProject\"\n",
            "{\"version\": \"1.1.0\", \"packages\": []}",
            serde_json::Value::Array(vec![]),
        );
        let cbor = serialize_resource(&resource);

        let tmp = tempfile::tempdir().expect("tempdir");
        stage_lean_project(&cbor, tmp.path()).expect("stage");

        assert!(tmp.path().join("lakefile.toml").exists());
        assert!(tmp.path().join("lake-manifest.json").exists());
        assert!(tmp.path().join("lean-toolchain").exists());
    }

    #[test]
    fn stages_source_tree_entries() {
        let source_tree = serde_json::json!([
            {
                "path": "TestProject.lean",
                "content_base64": base64::engine::general_purpose::STANDARD.encode("import TestProject.Foo\n"),
            },
            {
                "path": "TestProject/Foo.lean",
                "content_base64": base64::engine::general_purpose::STANDARD.encode("theorem foo : True := True.intro\n"),
            },
        ]);
        let resource = make_project_resource(
            "name = \"TestProject\"\n",
            "{\"version\": \"1.1.0\", \"packages\": []}",
            source_tree,
        );
        let cbor = serialize_resource(&resource);

        let tmp = tempfile::tempdir().expect("tempdir");
        stage_lean_project(&cbor, tmp.path()).expect("stage");

        assert_eq!(
            std::fs::read_to_string(tmp.path().join("TestProject.lean")).unwrap(),
            "import TestProject.Foo\n"
        );
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("TestProject").join("Foo.lean")).unwrap(),
            "theorem foo : True := True.intro\n"
        );
    }

    #[test]
    fn rejects_path_traversal_in_source_tree() {
        let source_tree = serde_json::json!([
            {
                "path": "../escape.lean",
                "content_base64": base64::engine::general_purpose::STANDARD.encode("bad"),
            },
        ]);
        let resource = make_project_resource(
            "name = \"TestProject\"\n",
            "{\"version\": \"1.1.0\", \"packages\": []}",
            source_tree,
        );
        let cbor = serialize_resource(&resource);
        let tmp = tempfile::tempdir().expect("tempdir");
        let err = stage_lean_project(&cbor, tmp.path()).expect_err("must reject ..");
        assert!(matches!(err, StagingError::UnsafePath(_)));
    }

    #[test]
    fn rejects_absent_lakefile() {
        let mut r = Resource::new(Iri::parse("urn:eigenius:test:project1").unwrap());
        r.set(
            Iri::parse(prop_iri::LAKE_MANIFEST).unwrap(),
            Value::String(String::new()),
        );
        let cbor = serialize_resource(&r);
        let tmp = tempfile::tempdir().expect("tempdir");
        let err =
            stage_lean_project(&cbor, tmp.path()).expect_err("must error on missing lakefile");
        match err {
            StagingError::MissingProperty(p) => assert_eq!(p, prop_iri::LAKEFILE),
            other => panic!("expected MissingProperty(lakefile), got {other:?}"),
        }
    }
}
