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
// Phase 19a.5 (D31): mirror generation CLI surface — `mirror create`,
// `mirror get`, `mirror list`, `mirror inspect`.

use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::server::proto::eigenius_kernel_client::EigeniusKernelClient;
use eigenius_runtime_substrate::chain::ChainAccessor;
use eigenius_runtime_substrate::mirror_generator::{
    LibraryContent, MirrorGenerationRequest, MirrorGenerator,
};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use tonic::transport::Channel;

use crate::common::{fetch_resource, submit_resource_for_load};

// --- Mirror commands ------------------------------------------------------

/// Implements `eigenius mirror create`. Resolves the EigenQL filter
/// against the named layer to a seed of class IRIs, runs the
/// language-specific generator client-side via a `RemoteChainAccessor`
/// that does per-resource gRPC roundtrips, commits the resulting
/// `RuntimePackageMirror` to the chain, and writes the source files to
/// `--output`.
#[allow(clippy::too_many_arguments)]
pub async fn mirror_create(
    endpoint: &str,
    layer: &str,
    filter: Option<&str>,
    filter_file: Option<&str>,
    language: &str,
    output: &str,
    institution_file: Option<&str>,
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
    let mut seed_iris: Vec<Iri> = rows
        .iter()
        .filter_map(|r| r.get("iri").and_then(|v| v.as_str()))
        .filter_map(|s| Iri::parse(s).ok())
        .collect();
    if seed_iris.is_empty() && institution_file.is_none() {
        eprintln!(
            "Filter query returned no class IRIs. Confirm the query has a \
             RETURN clause that exposes the class IRI column as `iri`."
        );
        std::process::exit(1);
    }

    // Institution-aware seed augmentation. When `--institution-file
    // <path>` is set, parse the institution declaration file and pull
    // every `RuntimeMethodSignature.input_types` / `output_type`
    // class into the seed. Closes the gap that previously forced
    // notebook authors to manually list cross-institution return
    // classes (e.g. `OptimisationProblem` in a Symbolics mirror seed
    // because `frame_as_optimisation_problem` returns one).
    //
    // The augmentation reads the file rather than querying the chain
    // because, in the canonical flow, the institution declaration
    // only lands on the chain at `eigenius institution install` time
    // — which runs *after* mirror generation (env build bakes the
    // mirror in; the institution declaration references the env
    // IRI). The file is the source of truth at this stage.
    if let Some(file_path) = institution_file {
        let added = match augment_seed_from_institution_file(Path::new(file_path)) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("--institution-file `{file_path}`: {e}");
                std::process::exit(1);
            }
        };
        let before = seed_iris.len();
        for iri in added {
            if !seed_iris.contains(&iri) {
                seed_iris.push(iri);
            }
        }
        let after = seed_iris.len();
        if !json && after > before {
            eprintln!(
                "  +{} class(es) discovered from `{}` signature contracts",
                after - before,
                file_path
            );
        }
    }
    if seed_iris.is_empty() {
        eprintln!("Combined filter + institution augmentation returned no class IRIs.");
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
        // Fetch via the kernel's Inspect RPC. We're called
        // synchronously from inside the closure walker, which itself
        // runs inside the CLI's `#[tokio::main]` async context. Use
        // tokio's `block_in_place + Handle::current().block_on`
        // rather than `futures::executor::block_on`: the latter
        // doesn't drive tokio's reactor, so tonic's `tower::buffer`
        // worker task never gets polled and the gRPC future hangs
        // indefinitely while the calling thread spins at 100% CPU on
        // `ThreadNotify::wake_by_ref`. Confirmed by samply profile —
        // 100% of samples in `oneshot::Inner::poll_recv` with the
        // sender task never scheduled.
        let resource = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let mut c = self.client.lock().expect("client mutex poisoned").clone();
                fetch_resource(&mut c, &target_str).await
            })
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

/// Read the institution declaration file, walk every
/// `RuntimeMethodSignature` resource it carries, and collect the
/// class IRIs in those signatures' `input_types` + `output_type`
/// lists. Used by `mirror create --institution-file` to augment the
/// user-supplied filter seed with cross-institution classes the
/// institution's handlers must encode/decode (e.g. the JuMP
/// `OptimisationProblem` returned by Symbolics'
/// `frame_as_optimisation_problem`).
///
/// Reads the file rather than querying the chain because the
/// declarations only land on the chain at `eigenius institution
/// install` time — which runs *after* mirror generation in the
/// canonical setup flow (env build bakes the mirror in; the
/// institution declaration references the env IRI). The file is the
/// source of truth at this stage.
fn augment_seed_from_institution_file(file_path: &Path) -> Result<Vec<Iri>, String> {
    use eigenius_kernel::ontology::eigon_json;

    let extension = file_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if !matches!(extension, "json" | "eigon-json") {
        return Err(format!(
            "expected an Eigon-JSON file (.json or .eigon-json); got `.{extension}`"
        ));
    }

    let bytes =
        std::fs::read_to_string(file_path).map_err(|e| format!("failed to read file: {e}"))?;
    let document =
        eigon_json::parse_document(&bytes).map_err(|e| format!("failed to parse: {e}"))?;

    let signature_class_iri = "urn:eigenius:runtime:RuntimeMethodSignature";
    let is_a_iri = Iri::parse("urn:eigenius:core:is_a").expect("well-known IRI");
    let input_types_iri = Iri::parse("urn:eigenius:runtime:input_types").expect("well-known IRI");
    let output_type_iri = Iri::parse("urn:eigenius:runtime:output_type").expect("well-known IRI");

    let mut classes: Vec<Iri> = Vec::new();
    for resource in &document {
        // `is_a` is an array of class IRIs. Match against the
        // RuntimeMethodSignature class.
        let is_a_match = match resource.get(&is_a_iri) {
            Some(Value::Array(items)) => items.iter().any(|v| match v {
                Value::ResourceRef(i) => i.as_str() == signature_class_iri,
                Value::String(s) => s == signature_class_iri,
                _ => false,
            }),
            _ => false,
        };
        if !is_a_match {
            continue;
        }
        if let Some(value) = resource.get(&input_types_iri) {
            for c in iri_array_from_value(value) {
                if !classes.contains(&c) {
                    classes.push(c);
                }
            }
        }
        if let Some(value) = resource.get(&output_type_iri) {
            if let Some(c) = single_iri_from_value(value) {
                if !classes.contains(&c) {
                    classes.push(c);
                }
            }
        }
    }
    Ok(classes)
}

/// Extract every IRI-shaped element from a `core:resource_array`
/// property value. Tolerates the post-`canonicalise_resource_refs`
/// `ResourceRef` shape and the pre-canonical `String` shape.
fn iri_array_from_value(value: &eigenius_kernel::ontology::resource::Value) -> Vec<Iri> {
    use eigenius_kernel::ontology::resource::Value;
    match value {
        Value::Array(items) => items.iter().filter_map(single_iri_from_value).collect(),
        _ => Vec::new(),
    }
}

/// Extract an IRI from a `core:resource` property value (single).
fn single_iri_from_value(value: &eigenius_kernel::ontology::resource::Value) -> Option<Iri> {
    use eigenius_kernel::ontology::resource::Value;
    match value {
        Value::ResourceRef(i) => Some(i.clone()),
        Value::String(s) => Iri::parse(s).ok(),
        _ => None,
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
