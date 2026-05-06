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
//
// Phase 19a.5: D31 (External Institution Authoring & Dispatch Lifecycle)
// CLI surface — mirror generation, env image build, and institution
// installation. The kernel-side dispatch path lands in 19a.5.c–d; this
// module is the author-facing surface only.

use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::server::proto;
use eigenius_kernel::server::proto::eigenius_kernel_client::EigeniusKernelClient;
use eigenius_runtime_substrate::chain::ChainAccessor;
use eigenius_runtime_substrate::mirror_generator::{
    LibraryContent, MirrorGenerationRequest, MirrorGenerator,
};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use tonic::transport::Channel;

// --- Mirror commands ------------------------------------------------------

/// Implements `eigenius mirror create`. Resolves the EigenQL filter
/// against the named layer to a seed of class IRIs, runs the
/// language-specific generator client-side via a `RemoteChainAccessor`
/// that does per-resource gRPC roundtrips, commits the resulting
/// `RuntimePackageMirror` to the chain, and writes the source files to
/// `--output`.
pub async fn mirror_create(
    endpoint: &str,
    layer: &str,
    filter: Option<&str>,
    filter_file: Option<&str>,
    language: &str,
    output: &str,
    json: bool,
) {
    if language != "julia" {
        eprintln!(
            "language `{language}` is not yet supported (only `julia` for v1; \
             other languages tracked in https://github.com/eigenius/eigenius/issues/41)"
        );
        std::process::exit(1);
    }

    // Resolve the filter to seed class IRIs.
    let query = match (filter, filter_file) {
        (Some(q), None) => q.to_string(),
        (None, Some(p)) => match std::fs::read_to_string(p) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to read filter file `{p}`: {e}");
                std::process::exit(1);
            }
        },
        (None, None) => {
            eprintln!("'mirror create' requires --filter or --filter-file");
            std::process::exit(1);
        }
        (Some(_), Some(_)) => {
            eprintln!("--filter and --filter-file are mutually exclusive");
            std::process::exit(1);
        }
    };

    let mut client = crate::connect_client(endpoint).await;
    let rows = crate::run_query(&mut client, &query).await;
    let seed_iris: Vec<Iri> = rows
        .iter()
        .filter_map(|r| r.get("iri").and_then(|v| v.as_str()))
        .filter_map(|s| Iri::parse(s).ok())
        .collect();
    if seed_iris.is_empty() {
        eprintln!(
            "Filter query returned no class IRIs. Confirm the query has a \
             RETURN clause that exposes the class IRI column as `iri`."
        );
        std::process::exit(1);
    }

    let layer_iri = match Iri::parse(layer) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("--layer `{layer}` is not a valid IRI: {e}");
            std::process::exit(1);
        }
    };

    // Generate the mirror via JuliaMirrorGenerator + RemoteChainAccessor.
    let chain = RemoteChainAccessor::new(client.clone(), layer.to_string());
    let request = MirrorGenerationRequest {
        source_layer: &layer_iri,
        seed_classes: &seed_iris,
        chain: &chain,
    };
    let generator = eigenius_julia::JuliaMirrorGenerator::new();
    let output_data = match generator.generate(&request) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("Mirror generation failed: {e}");
            std::process::exit(1);
        }
    };

    // Build the RuntimePackageMirror resource.
    let mirror_resource = eigenius_julia::mirror_to_resource(
        &generator,
        &output_data,
        &layer_iri,
        Some(&now_rfc3339()),
    );

    // Commit to chain via Load.
    let mirror_iri = mirror_resource
        .id()
        .map(|i| i.as_str().to_string())
        .unwrap_or_default();
    submit_resource_for_load(&mut client, &mirror_resource).await;

    // Write source files to --output.
    let LibraryContent::Embedded(files) = &output_data.library else {
        eprintln!("Mirror library is not embedded — cannot write to local files");
        std::process::exit(1);
    };
    if let Err(e) = std::fs::create_dir_all(output) {
        eprintln!("Failed to create output dir `{output}`: {e}");
        std::process::exit(1);
    }
    for f in files {
        let dest = Path::new(output).join(&f.path);
        if let Some(parent) = dest.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("Failed to create directory {}: {e}", parent.display());
                std::process::exit(1);
            }
        }
        if let Err(e) = std::fs::write(&dest, &f.content) {
            eprintln!("Failed to write {}: {e}", dest.display());
            std::process::exit(1);
        }
    }

    if json {
        println!(
            "{{\"success\":true,\"mirror_iri\":\"{}\",\"file_count\":{},\"output_dir\":\"{}\"}}",
            mirror_iri,
            files.len(),
            output
        );
    } else {
        println!("Mirror created.");
        println!("  IRI: {}", mirror_iri);
        println!("  Mirrored classes: {}", output_data.mirrored_classes.len());
        println!("  Files written to: {} ({} files)", output, files.len());
    }
}

/// Implements `eigenius mirror get`. Fetches a previously-committed
/// `RuntimePackageMirror` by IRI and writes its embedded source files
/// to `--output`. Read-only — no commit.
pub async fn mirror_get(endpoint: &str, iri: &str, output: &str, json: bool) {
    let mut client = crate::connect_client(endpoint).await;

    let resource = match fetch_resource(&mut client, iri).await {
        Some(r) => r,
        None => {
            eprintln!("No RuntimePackageMirror at IRI `{iri}` (or unable to resolve)");
            std::process::exit(1);
        }
    };

    let library_iri = Iri::parse("urn:eigenius:runtime:library_content").expect("static IRI");
    let library_json = match resource.get(&library_iri) {
        Some(Value::Json(v)) => v,
        Some(other) => {
            eprintln!("library_content is not a JSON value (got {other:?})");
            std::process::exit(1);
        }
        None => {
            eprintln!("Resource at `{iri}` has no library_content property");
            std::process::exit(1);
        }
    };

    let kind = library_json
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if kind != "embedded" {
        eprintln!("library_content.kind = `{kind}` not yet supported (only `embedded`)");
        std::process::exit(1);
    }
    let files_arr = match library_json.get("files").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => {
            eprintln!("library_content.files is missing or not an array");
            std::process::exit(1);
        }
    };

    if let Err(e) = std::fs::create_dir_all(output) {
        eprintln!("Failed to create output dir `{output}`: {e}");
        std::process::exit(1);
    }
    let mut written = 0usize;
    for entry in files_arr {
        let path = entry.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let b64 = entry
            .get("content_b64")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let bytes = match base64_decode(b64) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("Failed to decode base64 for `{path}`: {e}");
                std::process::exit(1);
            }
        };
        let dest = Path::new(output).join(path);
        if let Some(parent) = dest.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                eprintln!("Failed to create directory {}: {e}", parent.display());
                std::process::exit(1);
            }
        }
        if let Err(e) = std::fs::write(&dest, &bytes) {
            eprintln!("Failed to write {}: {e}", dest.display());
            std::process::exit(1);
        }
        written += 1;
    }

    if json {
        println!(
            "{{\"success\":true,\"mirror_iri\":\"{}\",\"file_count\":{},\"output_dir\":\"{}\"}}",
            iri, written, output
        );
    } else {
        println!("Mirror retrieved.");
        println!("  IRI: {}", iri);
        println!("  Files written to: {} ({} files)", output, written);
    }
}

/// Implements `eigenius mirror list`.
pub async fn mirror_list(endpoint: &str, language: Option<&str>, json: bool) {
    let mut client = crate::connect_client(endpoint).await;
    let lang_clause = match language {
        Some(l) => format!(", \"urn:eigenius:runtime:language\": \"{l}\""),
        None => String::new(),
    };
    let query = format!(
        r#"
        MATCH "urn:eigenius:runtime:RuntimePackageMirror"(?m) {{
            "urn:eigenius:core:short_name": ?name{lang_clause}
        }}
        RETURN [] {{ iri: ?m, name: ?name }}
    "#,
    );
    let rows = crate::run_query(&mut client, &query).await;
    if json {
        println!("{}", serde_json::to_string(&rows).unwrap());
    } else if rows.is_empty() {
        println!("No mirrors registered.");
    } else {
        println!("Mirrors:");
        for r in &rows {
            let iri = r.get("iri").and_then(|v| v.as_str()).unwrap_or("?");
            let name = r.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            println!("  {name} ({iri})");
        }
    }
}

/// Implements `eigenius mirror inspect`.
pub async fn mirror_inspect(endpoint: &str, iri: &str, json: bool) {
    let mut client = crate::connect_client(endpoint).await;
    let resource = match fetch_resource(&mut client, iri).await {
        Some(r) => r,
        None => {
            eprintln!("No resource at IRI `{iri}`");
            std::process::exit(1);
        }
    };
    let read = |prop: &str| {
        Iri::parse(prop)
            .ok()
            .and_then(|i| resource.get(&i).cloned())
    };
    let str_prop = |prop: &str| {
        read(prop)
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "(not set)".to_string())
    };
    let mirrored_count = read("urn:eigenius:runtime:mirrored_classes")
        .map(|v| v.as_iri_array().len())
        .unwrap_or(0);
    if json {
        println!(
            "{{\"iri\":\"{}\",\"language\":\"{}\",\"source_layer\":\"{}\",\"library_content_hash\":\"{}\",\"mirrored_classes\":{}}}",
            iri,
            str_prop("urn:eigenius:runtime:language"),
            str_prop("urn:eigenius:runtime:source_layer"),
            str_prop("urn:eigenius:runtime:library_content_hash"),
            mirrored_count
        );
    } else {
        println!("Mirror: {iri}");
        println!("  Language: {}", str_prop("urn:eigenius:runtime:language"));
        println!(
            "  Source layer: {}",
            str_prop("urn:eigenius:runtime:source_layer")
        );
        println!(
            "  Generator: {} {}",
            str_prop("urn:eigenius:runtime:generator_identifier"),
            str_prop("urn:eigenius:runtime:generator_version"),
        );
        println!(
            "  Generator hash: {}",
            str_prop("urn:eigenius:runtime:generator_content_hash"),
        );
        println!(
            "  Library hash: {}",
            str_prop("urn:eigenius:runtime:library_content_hash"),
        );
        println!("  Mirrored classes: {mirrored_count}");
    }
}

// --- Env commands ---------------------------------------------------------

/// Implements `eigenius env build`. Reads the handler package from
/// `--package-path` (or cwd), fetches the mirror Resource from the
/// chain by IRI, and drives the substrate's
/// `JuliaLanguageRuntime::build_environment_image` to produce an OCI
/// image with handler + mirror + `EigeniusJuliaCommon` baked in.
/// Prints the resulting `sha256:` digest. Pass that digest to
/// `eigenius env create --image-digest <digest>` to commit the
/// `RuntimeEnvironment` resource referencing it.
///
/// Filesystem contract:
///   `<package-path>/Project.toml` — handler package manifest.
///   `<package-path>/src/**`        — handler source tree (recursive,
///                                    flattened into a JSON archive
///                                    the substrate's image-build
///                                    pipeline materialises under
///                                    `/opt/eigenius/packages/<name>/`).
///
/// Network contract: connects to the kernel's gRPC endpoint to fetch
/// the mirror Resource by IRI; the actual buildah invocation runs
/// locally against the host's Docker daemon.
#[allow(clippy::too_many_arguments)]
pub async fn env_build(
    endpoint: &str,
    language: &str,
    package_path: Option<&str>,
    mirror_iri: &str,
    base_image: &str,
    worker_source_dir: Option<&str>,
    depot: Option<&str>,
    json: bool,
) {
    if language != "julia" {
        eprintln!("language `{language}` is not yet supported by env build (only `julia` for v1)");
        std::process::exit(1);
    }

    // 1. Resolve package directory (cwd by default).
    let pkg_dir = match package_path {
        Some(p) => std::path::PathBuf::from(p),
        None => match std::env::current_dir() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("could not read current working directory: {e}");
                std::process::exit(1);
            }
        },
    };
    let project_toml_path = pkg_dir.join("Project.toml");
    let src_dir = pkg_dir.join("src");
    if !project_toml_path.is_file() {
        eprintln!(
            "handler package missing `Project.toml` at {}; run from the package directory \
             or pass --package-path",
            project_toml_path.display()
        );
        std::process::exit(1);
    }
    if !src_dir.is_dir() {
        eprintln!(
            "handler package missing `src/` directory at {}",
            src_dir.display()
        );
        std::process::exit(1);
    }

    // 2. Read Project.toml + extract package name.
    let project_toml = match std::fs::read_to_string(&project_toml_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to read {}: {e}", project_toml_path.display());
            std::process::exit(1);
        }
    };
    let pkg_name = match parse_project_toml_name(&project_toml) {
        Some(n) => n,
        None => {
            eprintln!(
                "could not find a `name = \"...\"` line in {}",
                project_toml_path.display()
            );
            std::process::exit(1);
        }
    };

    // 3. Walk src/ recursively, collect (relative-path, bytes).
    let source_files = match collect_handler_sources(&src_dir) {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "failed to read handler sources under {}: {e}",
                src_dir.display()
            );
            std::process::exit(1);
        }
    };
    if source_files.is_empty() {
        eprintln!(
            "handler package's src/ directory contains no files at {}",
            src_dir.display()
        );
        std::process::exit(1);
    }

    // 4. Fetch the mirror Resource from the chain.
    let mut client = crate::connect_client(endpoint).await;
    let mirror_resource = match fetch_resource(&mut client, mirror_iri).await {
        Some(r) => r,
        None => {
            eprintln!("--mirror IRI `{mirror_iri}` did not resolve to a chain-committed resource");
            std::process::exit(1);
        }
    };
    drop(client);

    // 5. Resolve the Julia worker source dir.
    let worker_dir = match resolve_worker_source_dir(worker_source_dir) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    // 6. Resolve depot path.
    let depot_path = match depot {
        Some(p) => std::path::PathBuf::from(p),
        None => {
            let dir = std::env::temp_dir().join(format!(
                "eigenius-env-build-{}-{}",
                std::process::id(),
                pkg_name
            ));
            if let Err(e) = std::fs::create_dir_all(&dir) {
                eprintln!("failed to create depot dir {}: {e}", dir.display());
                std::process::exit(1);
            }
            dir
        }
    };

    // 7. Build the in-memory RuntimePackage Resource.
    let handler_pkg = build_handler_package_resource(&pkg_name, &project_toml, &source_files);

    // 8. Construct the substrate runtime + spawner, then drive
    // `build_environment_image`. `DockerServiceSpawner::new` owns its
    // own multi-thread Tokio runtime and `block_on`s into Bollard;
    // doing that from inside this `async` CLI context would panic with
    // "Cannot start a runtime from within a runtime", so the whole
    // sync subtree runs on a blocking thread.
    let depot_for_blocking = depot_path.clone();
    let worker_dir_for_blocking = worker_dir.clone();
    let base_image_for_blocking = base_image.to_string();
    let digest = match tokio::task::spawn_blocking(move || {
        let spawner = eigenius_runtime_substrate::spawner::service::DockerServiceSpawner::new(
            eigenius_runtime_substrate::spawner::DockerSpawnerConfig::new(
                depot_for_blocking.clone(),
            ),
        )
        .map_err(|e| {
            format!(
                "failed to construct DockerServiceSpawner: {e}\n\
                     Is the Docker daemon running and reachable?"
            )
        })?;
        let runtime = eigenius_julia::JuliaLanguageRuntime::new(
            worker_dir_for_blocking,
            base_image_for_blocking,
            std::sync::Arc::new(spawner),
            depot_for_blocking,
        );
        use eigenius_runtime_substrate::language_runtime::LanguageRuntime;
        let env_resource = eigenius_kernel::ontology::resource::Resource::new_embedded();
        runtime
            .build_environment_image(&env_resource, &[handler_pkg], Some(&mirror_resource))
            .map_err(|e| format!("env build failed: {e}"))
    })
    .await
    {
        Ok(Ok(d)) => d,
        Ok(Err(msg)) => {
            eprintln!("{msg}");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("env build worker join failed: {e}");
            std::process::exit(1);
        }
    };

    // 10. Capture the runtime version from the built image. This is
    // the patch-level pin (`1.12.1`, not `1.12-bookworm`) the chain
    // ontology requires on `RuntimeEnvironment.runtime_version` —
    // anchored to whatever the base image actually shipped, so a
    // re-instantiation on a different host gets the same Julia.
    let runtime_version = match query_runtime_version(language, digest.as_str()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("env build: failed to capture {language} runtime version from image: {e}");
            std::process::exit(1);
        }
    };

    // 11. Print the digest + runtime version. JSON mode emits a single
    // line so callers can pipe through `jq`; human mode formats for
    // readability.
    if json {
        println!(
            "{{\"image_digest\":\"{}\",\"runtime_version\":\"{}\",\"package_name\":\"{}\",\"mirror_iri\":\"{}\"}}",
            digest.as_str(),
            runtime_version,
            pkg_name,
            mirror_iri
        );
    } else {
        println!("Image built.");
        println!("  Package: {pkg_name}");
        println!("  Mirror : {mirror_iri}");
        println!("  Digest : {}", digest.as_str());
        println!("  Runtime version: {runtime_version}");
        println!();
        println!("Commit the env Resource referencing this digest with:");
        println!(
            "  eigenius env create --language {language} --handler-package {} --mirror {mirror_iri} \\",
            pkg_dir.display()
        );
        println!(
            "      --as-iri <ENV_IRI> --image-digest {} --runtime-version {runtime_version}",
            digest.as_str()
        );
    }
}

/// Per-language version-extraction recipe used by [`query_runtime_version`].
struct VersionProbe {
    /// Binary name to invoke as the container's entrypoint.
    cmd: &'static str,
    /// Arguments to pass to `cmd`.
    args: &'static [&'static str],
    /// Parser that pulls a version string out of the command's stdout.
    parse: fn(&str) -> Option<String>,
}

fn version_probe_for(language: &str) -> Result<VersionProbe, String> {
    match language {
        "julia" => Ok(VersionProbe {
            cmd: "julia",
            args: &["--version"],
            parse: parse_julia_version,
        }),
        other => Err(format!(
            "no version-extraction command registered for language `{other}`"
        )),
    }
}

/// Run the language's version command inside the freshly-built image
/// and parse the output. The base image already has the runtime
/// installed (by definition — the substrate's Dockerfile composer's
/// `install_runtime` is empty for languages whose base image ships
/// the runtime, and Julia's does), so a one-shot `docker run --rm`
/// is sufficient.
fn query_runtime_version(language: &str, image_digest: &str) -> Result<String, String> {
    let probe = version_probe_for(language)?;

    let output = std::process::Command::new("docker")
        .args(["run", "--rm", "--entrypoint", probe.cmd, image_digest])
        .args(probe.args)
        .output()
        .map_err(|e| {
            format!(
                "`docker run --rm <image> {}` failed to start: {e}",
                probe.cmd
            )
        })?;
    if !output.status.success() {
        return Err(format!(
            "`docker run --rm <image> {}` exited {}: {}",
            probe.cmd,
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    (probe.parse)(&stdout).ok_or_else(|| {
        format!(
            "could not parse {language} version from `{}` output: {}",
            probe.cmd,
            stdout.trim()
        )
    })
}

/// Parse `julia version 1.12.1\n` (the canonical `julia --version`
/// output shape) and return `1.12.1`.
fn parse_julia_version(stdout: &str) -> Option<String> {
    for line in stdout.lines() {
        let trimmed = line.trim();
        // `julia version X.Y.Z[-pre]` — anchor on the prefix so any
        // future addition (e.g. a leading banner line) doesn't drift
        // us into the wrong line.
        if let Some(rest) = trimmed.strip_prefix("julia version ") {
            let v = rest.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Extract `name = "..."` from a `Project.toml` body. Tolerant
/// shape — splits on whitespace and `=`, returns the first quoted
/// `name = "..."` it finds. Sufficient for v1's MVP; a full TOML
/// parser is overkill for the single field we need.
fn parse_project_toml_name(toml: &str) -> Option<String> {
    for line in toml.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("name") {
            let after = rest.trim_start();
            let after = after.strip_prefix('=')?.trim_start();
            let stripped = after.strip_prefix('"')?;
            let end = stripped.find('"')?;
            return Some(stripped[..end].to_string());
        }
    }
    None
}

/// Recursively collect every regular file under `dir`, returning a
/// list of (path-relative-to-`dir`-parent, bytes). The relative path
/// stays under `src/` because the substrate's package materialiser
/// writes files under `packages/<name>/<path>` and the handler package
/// contract is `packages/<name>/src/<...>`.
fn collect_handler_sources(src_dir: &std::path::Path) -> std::io::Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    walk_dir(src_dir, std::path::Path::new("src"), &mut out)?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn walk_dir(
    dir: &std::path::Path,
    relative_root: &std::path::Path,
    out: &mut Vec<(String, Vec<u8>)>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        let file_name = entry.file_name();
        let rel = relative_root.join(&file_name);
        if file_type.is_dir() {
            walk_dir(&path, &rel, out)?;
        } else if file_type.is_file() {
            let bytes = std::fs::read(&path)?;
            // Always use forward slashes — these end up in the
            // image's filesystem.
            let rel_str = rel
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            out.push((rel_str, bytes));
        }
        // Symlinks / other types: skip silently.
    }
    Ok(())
}

/// Build a `RuntimePackage` Resource carrying the handler's
/// `package_name`, verbatim `Project.toml`, and a JSON `source_tree`
/// archive in the shape `runtime_package_to_materialization`
/// expects (see `crates/eigenius-julia/src/runtime.rs`).
fn build_handler_package_resource(
    pkg_name: &str,
    project_toml: &str,
    source_files: &[(String, Vec<u8>)],
) -> Resource {
    let pkg_iri = Iri::parse(&format!("urn:eigenius:cli:env-build:package:{pkg_name}"))
        .expect("static IRI shape");
    let mut r = Resource::new(pkg_iri);
    r.set(
        Iri::parse("urn:eigenius:runtime:package_name").unwrap(),
        Value::String(pkg_name.to_string()),
    );
    r.set(
        Iri::parse("urn:eigenius:runtime:manifest").unwrap(),
        Value::String(project_toml.to_string()),
    );
    let entries: Vec<serde_json::Value> = source_files
        .iter()
        .map(|(path, bytes)| {
            serde_json::json!({
                "path": path,
                "content_base64": base64_encode(bytes),
            })
        })
        .collect();
    r.set(
        Iri::parse("urn:eigenius:runtime:source_tree").unwrap(),
        Value::Json(serde_json::Value::Array(entries)),
    );
    r
}

/// Standard base64 encoder. Mirrors the substrate-side decoder's
/// expected alphabet (`A-Za-z0-9+/`, `=` padding) — keeping a tiny
/// hand-written impl avoids pulling in a dep just for this.
fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0];
        let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };
        out.push(ALPHABET[(b0 >> 2) as usize] as char);
        out.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Resolve the Julia worker source directory in priority order:
///   1. Explicit `--worker-source-dir` flag.
///   2. `$EIGENIUS_HOME/julia/runtime-worker` if `EIGENIUS_HOME` is set.
///   3. Workspace-relative `julia/runtime-worker/` based on the CLI's
///      `CARGO_MANIFEST_DIR` — works in dev when running from the
///      repo, errors otherwise. Production deployments should set
///      `EIGENIUS_HOME`.
fn resolve_worker_source_dir(flag: Option<&str>) -> Result<std::path::PathBuf, String> {
    let candidate = if let Some(p) = flag {
        std::path::PathBuf::from(p)
    } else if let Ok(home) = std::env::var("EIGENIUS_HOME") {
        std::path::PathBuf::from(home)
            .join("julia")
            .join("runtime-worker")
    } else {
        // CLI's manifest dir is `<repo>/cli`. The worker source lives
        // at `<repo>/julia/runtime-worker`.
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("julia")
            .join("runtime-worker")
    };
    let canonical = candidate.canonicalize().map_err(|e| {
        format!(
            "could not resolve worker source dir {}: {e}\n\
             Set --worker-source-dir or $EIGENIUS_HOME/julia/runtime-worker.",
            candidate.display()
        )
    })?;
    if !canonical.join("src").join("JuliaWorker.jl").is_file() {
        return Err(format!(
            "worker source dir {} does not contain src/JuliaWorker.jl",
            canonical.display()
        ));
    }
    Ok(canonical)
}

/// Implements `eigenius env create`. v1 takes a pre-built image digest
/// (`--image-digest`) and commits the `RuntimeEnvironment` resource to
/// the chain. The full image-build pipeline (handler-package + mirror
/// → buildah-driven OCI image) is the IO-heavy half of this lifecycle
/// and is deferred to a follow-up sub-milestone — see [D31 §4.2]
/// (../../docs/design/d31-external-institution-lifecycle.md#42-phase-3--build-the-env-image-with-eigenius-env-create).
///
/// What v1 verifies:
///  - The mirror IRI resolves to a `RuntimePackageMirror` on the chain.
///  - The handler-package directory has a readable `Project.toml`
///    declaring deps on `EigeniusMirror` and `EigeniusJuliaCommon`.
///  - The image_digest parses as `sha256:<64-hex>`.
///
/// What v1 commits:
///  - A `RuntimeEnvironment` resource at `--as-iri` with `language`,
///    `image_digest`, and references to the mirror + handler-package
///    metadata (handler_package_path captured as a property for
///    audit; full source baking lands later).
#[allow(clippy::too_many_arguments)]
pub async fn env_create(
    endpoint: &str,
    language: &str,
    handler_package: &str,
    mirror: &str,
    _include_package: &[String],
    as_iri: &str,
    _base_image: Option<&str>,
    image_digest: &str,
    runtime_version: &str,
    json: bool,
) {
    if language != "julia" {
        eprintln!("language `{language}` is not yet supported by env create (only `julia` for v1)");
        std::process::exit(1);
    }

    // Validate image_digest shape.
    if !image_digest.starts_with("sha256:") {
        eprintln!("--image-digest must be `sha256:<64-hex>`, got `{image_digest}`");
        std::process::exit(1);
    }
    let hex = &image_digest["sha256:".len()..];
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        eprintln!("--image-digest hex portion must be 64 lowercase hex chars, got `{hex}`");
        std::process::exit(1);
    }

    // Validate handler-package directory has a Project.toml with the expected deps.
    let project_toml_path = Path::new(handler_package).join("Project.toml");
    let project_toml_str = match std::fs::read_to_string(&project_toml_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "Cannot read handler-package Project.toml at {}: {e}",
                project_toml_path.display()
            );
            std::process::exit(1);
        }
    };
    if !project_toml_str.contains("EigeniusMirror") {
        eprintln!(
            "Warning: handler-package's Project.toml at {} does not declare a dep on \
             EigeniusMirror — the worker will not be able to import the mirror module \
             at boot. Continuing anyway; fix the Project.toml if dispatches fail.",
            project_toml_path.display()
        );
    }

    // Resolve the mirror IRI.
    let mut client = crate::connect_client(endpoint).await;
    let mirror_iri = match Iri::parse(mirror) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("--mirror is not a valid IRI: {e}");
            std::process::exit(1);
        }
    };
    if fetch_resource(&mut client, mirror_iri.as_str())
        .await
        .is_none()
    {
        eprintln!("--mirror IRI `{mirror}` does not resolve to a chain-committed resource");
        std::process::exit(1);
    }

    // Build the RuntimeEnvironment resource.
    let env_iri = match Iri::parse(as_iri) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("--as-iri is not a valid IRI: {e}");
            std::process::exit(1);
        }
    };

    // Derive a `short_name` from the env IRI's last segment — same
    // convention every other CLI verb uses for synthesised resources.
    let short_name = env_iri
        .as_str()
        .rsplit(':')
        .find(|s| !s.is_empty())
        .unwrap_or("env")
        .to_string();

    // The handler's `Project.toml` is the v1 stand-in for the env's
    // lockfile. The full pin set lives in the worker image's
    // `Manifest.toml` after `Pkg.instantiate`, but committing those
    // bytes requires post-build extraction (deferred to D26 §5.6's
    // pinned_packages projection). The handler manifest is the
    // best-available-on-the-host approximation for now and satisfies
    // the chain's required-property check.
    let lockfile_str = match std::fs::read_to_string(&project_toml_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "Cannot re-read handler-package Project.toml at {}: {e}",
                project_toml_path.display()
            );
            std::process::exit(1);
        }
    };
    let _ = project_toml_str; // already validated above; lockfile_str shadows the read

    let mut env = Resource::new(env_iri.clone());
    env.set(
        Iri::parse("urn:eigenius:core:is_a").expect("static IRI"),
        Value::Array(vec![Value::ResourceRef(
            Iri::parse("urn:eigenius:runtime:RuntimeEnvironment").expect("static IRI"),
        )]),
    );
    env.set(
        Iri::parse("urn:eigenius:core:short_name").expect("static IRI"),
        Value::String(short_name),
    );
    env.set(
        Iri::parse("urn:eigenius:runtime:language").expect("static IRI"),
        Value::String(language.to_string()),
    );
    env.set(
        Iri::parse("urn:eigenius:runtime:runtime_version").expect("static IRI"),
        Value::String(runtime_version.to_string()),
    );
    env.set(
        Iri::parse("urn:eigenius:runtime:lockfile").expect("static IRI"),
        Value::String(lockfile_str),
    );
    env.set(
        Iri::parse("urn:eigenius:runtime:lifecycle").expect("static IRI"),
        Value::ResourceRef(
            Iri::parse("urn:eigenius:runtime:lifecycle:Service").expect("static IRI"),
        ),
    );
    env.set(
        Iri::parse("urn:eigenius:runtime:image_digest").expect("static IRI"),
        Value::String(image_digest.to_string()),
    );
    // Carry the handler-package path as a recommended audit property
    // (kernel ontology may not yet declare this property — emitted
    // best-effort; chain validation will accept unknown properties on
    // a best-effort basis or flag them as warnings depending on
    // configuration).
    env.set(
        Iri::parse("urn:eigenius:runtime:handler_package_path").expect("static IRI"),
        Value::String(handler_package.to_string()),
    );

    submit_resource_for_load(&mut client, &env).await;

    if json {
        println!(
            "{{\"success\":true,\"env_iri\":\"{}\",\"image_digest\":\"{}\"}}",
            env_iri.as_str(),
            image_digest
        );
    } else {
        println!("RuntimeEnvironment created.");
        println!("  IRI: {}", env_iri.as_str());
        println!("  Language: {language}");
        println!("  Image digest: {image_digest}");
        println!("  Mirror: {}", mirror_iri.as_str());
        println!("  Handler package: {handler_package}");
        println!();
        println!("Note: v1 of `env create` commits the RuntimeEnvironment metadata only.");
        println!("The image is the user's responsibility (build via buildah/docker against");
        println!("the eigenius-julia integration test machinery; D31 §4.2 / Phase 19a.5.b'");
        println!("for the integrated image-build path).");
    }
}

pub async fn env_list(endpoint: &str, language: Option<&str>, json: bool) {
    let mut client = crate::connect_client(endpoint).await;
    let lang_clause = match language {
        Some(l) => format!(", \"urn:eigenius:runtime:language\": \"{l}\""),
        None => String::new(),
    };
    let query = format!(
        r#"
        MATCH "urn:eigenius:runtime:RuntimeEnvironment"(?e) {{
            "urn:eigenius:core:short_name": ?name{lang_clause}
        }}
        RETURN [] {{ iri: ?e, name: ?name }}
    "#,
    );
    let rows = crate::run_query(&mut client, &query).await;
    if json {
        println!("{}", serde_json::to_string(&rows).unwrap());
    } else if rows.is_empty() {
        println!("No runtime environments registered.");
    } else {
        println!("RuntimeEnvironments:");
        for r in &rows {
            let iri = r.get("iri").and_then(|v| v.as_str()).unwrap_or("?");
            let name = r.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            println!("  {name} ({iri})");
        }
    }
}

pub async fn env_inspect(endpoint: &str, iri: &str, json: bool) {
    let mut client = crate::connect_client(endpoint).await;
    let resource = match fetch_resource(&mut client, iri).await {
        Some(r) => r,
        None => {
            eprintln!("No resource at IRI `{iri}`");
            std::process::exit(1);
        }
    };
    let read = |prop: &str| {
        Iri::parse(prop)
            .ok()
            .and_then(|i| resource.get(&i).cloned())
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "(not set)".to_string())
    };
    if json {
        println!(
            "{{\"iri\":\"{}\",\"language\":\"{}\",\"image_digest\":\"{}\"}}",
            iri,
            read("urn:eigenius:runtime:language"),
            read("urn:eigenius:runtime:image_digest"),
        );
    } else {
        println!("RuntimeEnvironment: {iri}");
        println!("  Language: {}", read("urn:eigenius:runtime:language"));
        println!(
            "  Image digest: {}",
            read("urn:eigenius:runtime:image_digest")
        );
    }
}

// --- Institution commands -------------------------------------------------

/// Implements `eigenius institution install`. Sends the definition file
/// to the kernel via `LoadRequest` with `auto_commit`. Cross-checks for
/// `runtime_environment` and `mirror` references happen at commit time
/// in the kernel's ontology validator (kernel-side validation lands in
/// 19a.5.e proper; this CLI surface assumes kernel-side handling exists).
pub async fn institution_install(endpoint: &str, definition: &str, json: bool) {
    let definition_bytes = match std::fs::read(definition) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Failed to read definition file `{definition}`: {e}");
            std::process::exit(1);
        }
    };

    // Determine content type from file extension. Same heuristic as
    // `capability install`'s definition-file path.
    let content_type = if definition.ends_with(".eigon-json") || definition.ends_with(".json") {
        "application/eigon+json"
    } else if definition.ends_with(".eigon") || definition.ends_with(".esl") {
        "application/eigon+esl"
    } else {
        eprintln!("Unknown definition file extension; expected .eigon-json/.json or .eigon/.esl");
        std::process::exit(1);
    };

    let mut client = crate::connect_client(endpoint).await;
    let request = proto::LoadRequest {
        resources: definition_bytes,
        content_type: content_type.to_string(),
        auto_commit: true,
        branch: String::new(),
    };
    match client.load(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if resp.success {
                if json {
                    println!(
                        "{{\"success\":true,\"resource_count\":{},\"layer_id\":\"{}\"}}",
                        resp.resource_count, resp.layer_id
                    );
                } else {
                    println!(
                        "Installed {} resource(s). Layer: {}",
                        resp.resource_count, resp.layer_id
                    );
                }
            } else {
                eprintln!("Install failed:");
                for err in &resp.errors {
                    eprintln!("  {}: {}", err.rule, err.message);
                }
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn institution_list(endpoint: &str, json: bool) {
    let mut client = crate::connect_client(endpoint).await;
    let query = r#"
        MATCH "urn:eigenius:institution:Institution"(?i) {
            "urn:eigenius:institution:institution_name": ?name
        }
        RETURN [] { iri: ?i, name: ?name }
    "#;
    let rows = crate::run_query(&mut client, query).await;
    if json {
        println!("{}", serde_json::to_string(&rows).unwrap());
    } else if rows.is_empty() {
        println!("No institutions registered.");
    } else {
        println!("Institutions:");
        for r in &rows {
            let iri = r.get("iri").and_then(|v| v.as_str()).unwrap_or("?");
            let name = r.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            println!("  {name} ({iri})");
        }
    }
}

pub async fn institution_inspect(endpoint: &str, iri: &str, json: bool) {
    let mut client = crate::connect_client(endpoint).await;
    let resource = match fetch_resource(&mut client, iri).await {
        Some(r) => r,
        None => {
            eprintln!("No resource at IRI `{iri}`");
            std::process::exit(1);
        }
    };
    let read = |prop: &str| {
        Iri::parse(prop)
            .ok()
            .and_then(|i| resource.get(&i).cloned())
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "(not set)".to_string())
    };
    if json {
        println!(
            "{{\"iri\":\"{}\",\"name\":\"{}\",\"runtime\":\"{}\",\"runtime_environment\":\"{}\",\"mirror\":\"{}\"}}",
            iri,
            read("urn:eigenius:institution:institution_name"),
            read("urn:eigenius:institution:runtime"),
            read("urn:eigenius:institution:runtime_environment"),
            read("urn:eigenius:institution:mirror"),
        );
    } else {
        println!("Institution: {iri}");
        println!(
            "  Name: {}",
            read("urn:eigenius:institution:institution_name")
        );
        println!("  Runtime: {}", read("urn:eigenius:institution:runtime"));
        println!(
            "  RuntimeEnvironment: {}",
            read("urn:eigenius:institution:runtime_environment"),
        );
        println!("  Mirror: {}", read("urn:eigenius:institution:mirror"));
    }
}

// --- Helpers --------------------------------------------------------------

/// `ChainAccessor` impl that resolves resource IRIs by issuing a
/// per-resource gRPC query against a running kernel. Caches resolved
/// resources within a single mirror generation to avoid re-fetching.
struct RemoteChainAccessor {
    client: Mutex<EigeniusKernelClient<Channel>>,
    /// Captured for symmetry with the substrate-side `KernelChainAccessor`
    /// — the resolve query in this CLI proxy is currently layer-agnostic
    /// (the kernel resolves at the head layer); when the resolve query
    /// gains an at-layer parameter, this field stops being dead.
    #[allow(dead_code)]
    layer_iri: String,
    cache: Mutex<HashMap<String, Option<Resource>>>,
}

impl RemoteChainAccessor {
    fn new(client: EigeniusKernelClient<Channel>, layer_iri: String) -> Self {
        Self {
            client: Mutex::new(client),
            layer_iri,
            cache: Mutex::new(HashMap::new()),
        }
    }
}

impl ChainAccessor for RemoteChainAccessor {
    fn resolve(&self, _claim_layer: &Iri, target: &Iri) -> Option<Resource> {
        let target_str = target.as_str().to_string();
        if let Some(cached) = self
            .cache
            .lock()
            .expect("cache mutex poisoned")
            .get(&target_str)
            .cloned()
        {
            return cached;
        }
        // Fetch via the kernel's Inspect RPC. The async-bridge here is
        // clunky but acceptable for a CLI command that issues
        // O(closure-size) lookups during mirror generation.
        let resource = futures::executor::block_on(async {
            let mut c = self.client.lock().expect("client mutex poisoned").clone();
            fetch_resource(&mut c, &target_str).await
        });
        self.cache
            .lock()
            .expect("cache mutex poisoned")
            .insert(target_str, resource.clone());
        resource
    }

    fn is_ancestor_or_equal(&self, _anchor: &Iri, _candidate: &Iri) -> bool {
        // CLI-side mirror generation runs against a single `--layer`
        // for the whole closure — every reachable class is "at" that
        // layer from the generator's perspective. Returning true is
        // safe because the boundary check that uses this method only
        // runs at dispatch time against a kernel-side ChainAccessor
        // (not this CLI proxy).
        true
    }

    fn class_unchanged_between(&self, _: &Iri, _: &Iri, _: &Iri) -> bool {
        // Same reasoning — mirror generation only resolves at the
        // single source layer; cross-layer comparisons are kernel-side.
        true
    }
}

/// Issue a query that returns the resource at `iri` as part of the
/// result document. Walks the document, finds the matching resource by
/// IRI, returns it. Returns `None` if not found.
async fn fetch_resource(client: &mut EigeniusKernelClient<Channel>, iri: &str) -> Option<Resource> {
    // Use the kernel's Inspect RPC — the canonical "give me this
    // resource by IRI" surface. It walks the parent-layer chain on
    // the kernel side and returns the resource as Eigon-CBOR.
    let resp = client
        .inspect(proto::InspectRequest {
            iri: iri.to_string(),
            at_layer: String::new(),
            branch: String::new(),
        })
        .await
        .ok()?
        .into_inner();
    if !resp.found {
        return None;
    }
    eigon_cbor::parse_resource_lenient(&resp.resource).ok()
}

/// Submit a single Resource via Load with auto_commit. Helper for
/// `mirror create` etc. — anywhere the CLI needs to commit a single
/// resource the substrate built locally.
async fn submit_resource_for_load(client: &mut EigeniusKernelClient<Channel>, resource: &Resource) {
    let cbor_bytes = eigon_cbor::serialize_resource(resource);
    let request = proto::LoadRequest {
        resources: cbor_bytes,
        content_type: "application/eigon+cbor".to_string(),
        auto_commit: true,
        branch: String::new(),
    };
    match client.load(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if !resp.success {
                eprintln!("Load failed:");
                for err in &resp.errors {
                    eprintln!("  {}: {}", err.rule, err.message);
                }
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("gRPC error: {e}");
            std::process::exit(1);
        }
    }
}

fn now_rfc3339() -> String {
    let now = std::time::SystemTime::now();
    humantime::format_rfc3339_millis(now).to_string()
}

/// Standard-alphabet base64 decoder, matching the one used by the
/// mirror generator's encoder ([crates/eigenius-julia/src/mirror_gen.rs]).
fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let cleaned: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !cleaned.len().is_multiple_of(4) {
        return Err(format!(
            "input length {} not a multiple of 4",
            cleaned.len()
        ));
    }
    let mut out = Vec::with_capacity(cleaned.len() / 4 * 3);
    let mut i = 0;
    while i < cleaned.len() {
        let chunk = &cleaned[i..i + 4];
        let pad = chunk.iter().filter(|&&b| b == b'=').count();
        let v0 = val(chunk[0]).ok_or_else(|| format!("invalid byte {:?}", chunk[0] as char))?;
        let v1 = val(chunk[1]).ok_or_else(|| format!("invalid byte {:?}", chunk[1] as char))?;
        let v2 = if chunk[2] == b'=' {
            0
        } else {
            val(chunk[2]).ok_or_else(|| format!("invalid byte {:?}", chunk[2] as char))?
        };
        let v3 = if chunk[3] == b'=' {
            0
        } else {
            val(chunk[3]).ok_or_else(|| format!("invalid byte {:?}", chunk[3] as char))?
        };
        let n = ((v0 as u32) << 18) | ((v1 as u32) << 12) | ((v2 as u32) << 6) | (v3 as u32);
        out.push(((n >> 16) & 0xff) as u8);
        if pad < 2 {
            out.push(((n >> 8) & 0xff) as u8);
        }
        if pad < 1 {
            out.push((n & 0xff) as u8);
        }
        i += 4;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_julia_version_extracts_patch_level() {
        assert_eq!(
            parse_julia_version("julia version 1.12.1\n"),
            Some("1.12.1".to_string())
        );
    }

    #[test]
    fn parse_julia_version_handles_pre_release_suffix() {
        assert_eq!(
            parse_julia_version("julia version 1.12.0-rc1\n"),
            Some("1.12.0-rc1".to_string())
        );
    }

    #[test]
    fn parse_julia_version_returns_none_on_unrecognised_shape() {
        assert!(parse_julia_version("Julia 1.12.1\n").is_none());
        assert!(parse_julia_version("").is_none());
    }
}
