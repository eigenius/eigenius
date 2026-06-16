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
// Data commands (D53): `data attach`, `data list`, `data inspect`.
// Attaches an external file to the graph as a content-addressed
// `ingest:PinnedExternalFile` node — the bytes stay off-chain; only the
// reference + content_hash + media_type travel. Phase 0: `file://` local
// attach (read + hash + commit). Oxen + `verify` land in later phases.

use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};

use eigenius_kernel::server::proto::eigenius_kernel_client::EigeniusKernelClient;
use tonic::transport::Channel;

use crate::common::{fetch_resource, submit_resource_for_load};

const PINNED_FILE_CLASS: &str = "urn:eigenius:ingest:PinnedExternalFile";
const PROP_REFERENCE: &str = "urn:eigenius:ingest:reference";
const PROP_CONTENT_HASH: &str = "urn:eigenius:ingest:content_hash";
const PROP_MEDIA_TYPE: &str = "urn:eigenius:ingest:media_type";
const PROP_SOURCE: &str = "urn:eigenius:reflection:source";
const PROP_IS_A: &str = "urn:eigenius:core:is_a";
const PROP_SHORT_NAME: &str = "urn:eigenius:core:short_name";
const PROP_SCHEMA: &str = "urn:eigenius:ingest:schema";
const PROP_SCHEMAS: &str = "urn:eigenius:ingest:schemas";
const PROP_CONTENT_ENCODING: &str = "urn:eigenius:ingest:content_encoding";

/// Best-effort IANA media type from a file extension (D53 §3, §4.1).
fn media_type_for(file: &str) -> &'static str {
    let lower = file.to_ascii_lowercase();
    let lower = lower.strip_suffix(".gz").unwrap_or(&lower);
    if lower.ends_with(".parquet") {
        "application/vnd.apache.parquet"
    } else if lower.ends_with(".arrow") {
        "application/vnd.apache.arrow.file"
    } else if lower.ends_with(".csv") {
        "text/csv"
    } else if lower.ends_with(".tsv") || lower.ends_with(".gmt") {
        "text/tab-separated-values"
    } else if lower.ends_with(".json") {
        "application/json"
    } else if lower.ends_with(".xlsx") {
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    } else if lower.ends_with(".h5") || lower.ends_with(".hdf5") {
        "application/x-hdf5"
    } else if lower.ends_with(".rds") {
        "application/x-r-rds"
    } else {
        "application/octet-stream"
    }
}

/// Implements `eigenius data attach <file|file://…|oxen://…>`.
/// Computes the content hash from the bytes — streamed from a local file /
/// `file://`, or fetched once via the Oxen client for an `oxen://` reference —
/// mints the content-addressed `PinnedExternalFile` IRI, and commits the node.
/// The `reference` is the *durable* locator the substrate fetches from later;
/// the bytes stay off-chain. Idempotent — byte-identical files converge to one
/// IRI (D53 §3).
#[allow(clippy::too_many_arguments)]
pub async fn data_attach(
    endpoint: &str,
    file: &str,
    reference_override: Option<&str>,
    media_type_override: Option<&str>,
    name_override: Option<&str>,
    json: bool,
) {
    // Resolve the bytes-source by scheme → (content_hash, default reference,
    // a path-like string used to infer media type + short name).
    let (content_hash, default_reference, name_source) = if file.starts_with("oxen://") {
        attach_hash_oxen(file)
    } else {
        attach_hash_local(file)
    };

    let iri_str = match eigenius_runtime_substrate::pinned_external_file_iri(&content_hash) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Cannot mint IRI: {e}");
            std::process::exit(1);
        }
    };
    let iri = Iri::parse(&iri_str).expect("content-addressed IRI is well-formed");

    let reference = reference_override
        .map(str::to_string)
        .unwrap_or(default_reference);
    let media_type = media_type_override
        .map(str::to_string)
        .unwrap_or_else(|| media_type_for(&name_source).to_string());
    let short_name = name_override
        .map(str::to_string)
        .or_else(|| {
            std::path::Path::new(&name_source)
                .file_name()
                .and_then(|s| s.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "file".to_string());

    let mut node = Resource::new(iri.clone());
    let s = |p: &str| Iri::parse(p).expect("static IRI");
    node.set(
        s(PROP_IS_A),
        Value::Array(vec![Value::ResourceRef(s(PINNED_FILE_CLASS))]),
    );
    node.set(s(PROP_REFERENCE), Value::String(reference.clone()));
    node.set(s(PROP_CONTENT_HASH), Value::String(content_hash.clone()));
    node.set(s(PROP_MEDIA_TYPE), Value::String(media_type.clone()));
    node.set(s(PROP_SOURCE), Value::String(reference.clone()));
    node.set(s(PROP_SHORT_NAME), Value::String(short_name));

    let mut client = crate::connect_client(endpoint).await;
    submit_resource_for_load(&mut client, &node).await;

    if json {
        println!(
            "{{\"success\":true,\"iri\":\"{}\",\"content_hash\":\"{}\",\"reference\":\"{}\",\"media_type\":\"{}\"}}",
            iri.as_str(),
            content_hash,
            reference,
            media_type
        );
    } else {
        println!("PinnedExternalFile attached.");
        println!("  IRI:          {}", iri.as_str());
        println!("  content_hash: {content_hash}");
        println!("  reference:    {reference}");
        println!("  media_type:   {media_type}");
    }
}

/// Hash a local path or `file://` reference by streaming (never buffers the
/// whole file). Returns `(content_hash, file://abs reference, name source)`.
fn attach_hash_local(file: &str) -> (String, String, String) {
    let local = file.strip_prefix("file://").unwrap_or(file);
    let content_hash =
        match eigenius_runtime_substrate::content_hash_of_file(std::path::Path::new(local)) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("Failed to read file `{local}`: {e}");
                std::process::exit(1);
            }
        };
    let abs = std::fs::canonicalize(local).unwrap_or_else(|_| local.into());
    (
        content_hash,
        format!("file://{}", abs.display()),
        local.to_string(),
    )
}

/// Fetch an `oxen://` reference once via the Oxen client and hash the bytes.
/// Returns `(content_hash, the oxen reference verbatim, in-repo path)`.
fn attach_hash_oxen(file: &str) -> (String, String, String) {
    let oref = match eigenius_runtime_substrate::oxen::parse(file) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("Invalid oxen reference: {e}");
            std::process::exit(1);
        }
    };
    let tmp = std::env::temp_dir().join(format!("eig_attach_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);
    let path = match eigenius_runtime_substrate::oxen::download_into(&oref, &tmp) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Oxen download failed: {e}");
            let _ = std::fs::remove_dir_all(&tmp);
            std::process::exit(1);
        }
    };
    let content_hash = match eigenius_runtime_substrate::content_hash_of_file(&path) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Failed to hash downloaded file: {e}");
            let _ = std::fs::remove_dir_all(&tmp);
            std::process::exit(1);
        }
    };
    let _ = std::fs::remove_dir_all(&tmp);
    (content_hash, file.to_string(), oref.path)
}

/// Implements `eigenius data verify <iri>`. Fetches the node, re-fetches the
/// bytes by its `reference`, recomputes the hash, and checks it against the
/// pinned `content_hash` (fail closed — D53 §5).
pub async fn data_verify(endpoint: &str, iri: &str, json: bool) {
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
    };
    let (reference, content_hash) = match (read(PROP_REFERENCE), read(PROP_CONTENT_HASH)) {
        (Some(r), Some(h)) => (r, h),
        _ => {
            eprintln!("`{iri}` is missing reference or content_hash");
            std::process::exit(1);
        }
    };

    // Verify into a throwaway cache dir; we only care whether the hash matches.
    let tmp = std::env::temp_dir().join(format!("eig_verify_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);
    let opts = eigenius_runtime_substrate::ResolveOptions {
        cache_root: Some(&tmp),
        reject_node_local_files: false,
    };
    let outcome =
        eigenius_runtime_substrate::resolve_and_materialize(&reference, &content_hash, &opts);
    let _ = std::fs::remove_dir_all(&tmp);

    match outcome {
        Ok(_) => {
            if json {
                println!(
                    "{{\"verified\":true,\"iri\":\"{iri}\",\"content_hash\":\"{content_hash}\"}}"
                );
            } else {
                println!("✓ verified — bytes at {reference} match {content_hash}");
            }
        }
        Err(eigenius_runtime_substrate::RunError::ContentHashMismatch {
            expected, got, ..
        }) => {
            if json {
                println!(
                    "{{\"verified\":false,\"iri\":\"{iri}\",\"expected\":\"{expected}\",\"got\":\"{got}\"}}"
                );
            } else {
                eprintln!("✗ MISMATCH — {reference}\n  expected {expected}\n  got      {got}");
            }
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Verification could not fetch `{reference}`: {e}");
            std::process::exit(1);
        }
    }
}

/// Resolve a schema value (an embedded `DatasetSchema` or a `ResourceRef` to a
/// committed one) to a concrete resource.
async fn resolve_schema_value(
    client: &mut EigeniusKernelClient<Channel>,
    v: &Value,
) -> Option<Resource> {
    match v {
        Value::Embedded(b) => Some((**b).clone()),
        Value::ResourceRef(i) => fetch_resource(client, i.as_str()).await,
        // A schema reference can also arrive as a plain string IRI (e.g. when a
        // resource hasn't been ref-canonicalized on the read path).
        Value::String(s) => fetch_resource(client, s).await,
        _ => None,
    }
}

/// Collect a file's bound `DatasetSchema`s from `ingest:schema` (one) and
/// `ingest:schemas` (a set), resolving refs against the chain.
async fn collect_schemas(
    client: &mut EigeniusKernelClient<Channel>,
    resource: &Resource,
) -> Vec<Resource> {
    let mut out = Vec::new();
    for prop in [PROP_SCHEMA, PROP_SCHEMAS] {
        let Some(value) = Iri::parse(prop)
            .ok()
            .and_then(|i| resource.get(&i).cloned())
        else {
            continue;
        };
        match value {
            Value::Array(items) => {
                for v in &items {
                    if let Some(r) = resolve_schema_value(client, v).await {
                        out.push(r);
                    }
                }
            }
            other => {
                if let Some(r) = resolve_schema_value(client, &other).await {
                    out.push(r);
                }
            }
        }
    }
    out
}

/// Implements `eigenius data validate <iri>` — the D53 §4.1 checkable gate.
/// Fetches the file + its `DatasetSchema`(s), materializes the bytes (which also
/// re-verifies the content hash), reads the header, and checks each declared
/// layout against the actual columns. Tabular (CSV/TSV) only; columnar
/// (Parquet/Arrow) carry their own schema in-file and are validated worker-side.
pub async fn data_validate(endpoint: &str, iri: &str, json: bool) {
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
    };

    let schemas = collect_schemas(&mut client, &resource).await;
    if schemas.is_empty() {
        if json {
            println!(
                "{{\"valid\":true,\"iri\":\"{iri}\",\"note\":\"no schema bound (opaque file)\"}}"
            );
        } else {
            println!("No DatasetSchema bound to `{iri}` — opaque file, nothing to validate.");
        }
        return;
    }

    let (reference, content_hash) = match (read(PROP_REFERENCE), read(PROP_CONTENT_HASH)) {
        (Some(r), Some(h)) => (r, h),
        _ => {
            eprintln!("`{iri}` is missing reference or content_hash");
            std::process::exit(1);
        }
    };
    let media_type =
        read(PROP_MEDIA_TYPE).unwrap_or_else(|| "application/octet-stream".to_string());
    let content_encoding = read(PROP_CONTENT_ENCODING);

    // Materialize (and content-verify) into a throwaway dir.
    let tmp = std::env::temp_dir().join(format!("eig_validate_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);
    let opts = eigenius_runtime_substrate::ResolveOptions {
        cache_root: Some(&tmp),
        reject_node_local_files: false,
    };
    let path =
        match eigenius_runtime_substrate::resolve_and_materialize(&reference, &content_hash, &opts)
        {
            Ok(p) => p,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&tmp);
                eprintln!("Could not materialize `{reference}` for validation: {e}");
                std::process::exit(1);
            }
        };

    let delimited = matches!(
        media_type.as_str(),
        "text/csv" | "text/tab-separated-values"
    );
    if content_encoding.is_some() || !delimited {
        let _ = std::fs::remove_dir_all(&tmp);
        let reason = match &content_encoding {
            Some(enc) => format!("content_encoding `{enc}` not supported by the header gate"),
            None => format!("media_type `{media_type}` is not delimited text"),
        };
        if json {
            println!("{{\"valid\":true,\"iri\":\"{iri}\",\"note\":\"layout check deferred to worker: {reason}\"}}");
        } else {
            println!("✓ content verified. Layout check deferred to the worker ({reason}).");
        }
        return;
    }

    // Read the first non-empty lines: line 0 is the header for tabular
    // layouts; for a Collection (no header row) the lines are data rows.
    let lines: Vec<String> = {
        use std::io::BufRead;
        match std::fs::File::open(&path) {
            Ok(f) => std::io::BufReader::new(f)
                .lines()
                .map_while(Result::ok)
                .filter(|l| !l.trim().is_empty())
                .take(16)
                .collect(),
            Err(e) => {
                let _ = std::fs::remove_dir_all(&tmp);
                eprintln!("Could not read materialized file: {e}");
                std::process::exit(1);
            }
        }
    };
    let _ = std::fs::remove_dir_all(&tmp);

    // Validate each bound schema, dispatching on its layout kind.
    let mut all_ok = true;
    let mut report: Vec<(Option<String>, Vec<String>)> = Vec::new();
    for sr in &schemas {
        let parsed = eigenius_runtime_substrate::parse_dataset_schema(sr);
        // Member schemas describe an intra-file matrix (.rds member / sheet)
        // that a flat header can't represent — skip with a note.
        if parsed.member.is_some() {
            report.push((parsed.member.clone(), vec![]));
            continue;
        }
        let is_collection = matches!(
            parsed.layout.as_ref().map(|l| &l.kind),
            Some(eigenius_runtime_substrate::LayoutKind::Collection)
        );
        let issues = if is_collection {
            // Ragged: every line is a data row; split each into fields.
            let rows: Vec<Vec<String>> = lines
                .iter()
                .map(|l| eigenius_runtime_substrate::header_columns(l, &media_type))
                .collect();
            eigenius_runtime_substrate::validate_collection(&parsed, &rows)
        } else {
            let header = lines
                .first()
                .map(|l| eigenius_runtime_substrate::header_columns(l, &media_type))
                .unwrap_or_default();
            eigenius_runtime_substrate::validate_tabular(&parsed, &header)
        };
        if !issues.is_empty() {
            all_ok = false;
        }
        report.push((None, issues));
    }

    if json {
        let entries: Vec<String> = report
            .iter()
            .map(|(member, issues)| {
                let issues_json: Vec<String> = issues.iter().map(|i| format!("{i:?}")).collect();
                format!(
                    "{{\"member\":{},\"issues\":[{}]}}",
                    member
                        .as_ref()
                        .map(|m| format!("{m:?}"))
                        .unwrap_or_else(|| "null".to_string()),
                    issues_json.join(",")
                )
            })
            .collect();
        println!(
            "{{\"valid\":{all_ok},\"iri\":\"{iri}\",\"schemas\":[{}]}}",
            entries.join(",")
        );
    } else if all_ok {
        println!(
            "✓ valid — {}: layout matches the header for all bound schema(s).",
            iri
        );
        for (member, _) in &report {
            if let Some(m) = member {
                println!("  (member `{m}` schema — intra-file layout, not header-checkable here)");
            }
        }
    } else {
        eprintln!("✗ INVALID — {iri}");
        for (_, issues) in &report {
            for issue in issues {
                eprintln!("  - {issue}");
            }
        }
    }
    if !all_ok {
        std::process::exit(1);
    }
}

/// Implements `eigenius data provision <iri> [--cache-root <dir>]` — the D53 §7
/// provision step. Materializes a `PinnedExternalFile` into the local
/// content-addressed cache (`<cache>/<sha256-hex>/<name>`) the kernel reads for
/// native file-backed SampleSet recompute (D53 §6.1), fetching + content-
/// verifying via the §5 resolver. Run on the host whose depot the kernel reads
/// (co-located / per-host); the cache root defaults to
/// `$EIGENIUS_EXTFILE_CACHE_DIR`. `file://` on a shared volume needs no
/// provisioning (the kernel reads it directly) — this is for `oxen://` (and any
/// reference you want warmed into the cache).
pub async fn data_provision(endpoint: &str, iri: &str, cache_root: Option<&str>, json: bool) {
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
    };
    let (reference, content_hash) = match (read(PROP_REFERENCE), read(PROP_CONTENT_HASH)) {
        (Some(r), Some(h)) => (r, h),
        _ => {
            eprintln!("`{iri}` is missing reference or content_hash");
            std::process::exit(1);
        }
    };

    let cache = cache_root
        .map(str::to_string)
        .or_else(|| std::env::var("EIGENIUS_EXTFILE_CACHE_DIR").ok())
        .filter(|s| !s.trim().is_empty());
    let Some(cache) = cache else {
        eprintln!(
            "No cache root: pass --cache-root <dir> or set EIGENIUS_EXTFILE_CACHE_DIR \
             (the depot's extfile-cache the kernel reads)."
        );
        std::process::exit(1);
    };

    let opts = eigenius_runtime_substrate::ResolveOptions {
        cache_root: Some(std::path::Path::new(&cache)),
        reject_node_local_files: false,
    };
    match eigenius_runtime_substrate::resolve_and_materialize(&reference, &content_hash, &opts) {
        Ok(path) => {
            if json {
                println!(
                    "{{\"provisioned\":true,\"iri\":\"{iri}\",\"path\":{:?}}}",
                    path.to_string_lossy()
                );
            } else {
                println!("✓ provisioned {iri}");
                println!("  → {}", path.display());
            }
        }
        Err(e) => {
            eprintln!("Provision failed for `{reference}`: {e}");
            std::process::exit(1);
        }
    }
}

/// Implements `eigenius data list [--media-type <mt>]`.
pub async fn data_list(endpoint: &str, media_type: Option<&str>, json: bool) {
    let mut client = crate::connect_client(endpoint).await;
    let mt_clause = match media_type {
        Some(m) => format!(", \"{PROP_MEDIA_TYPE}\": \"{m}\""),
        None => String::new(),
    };
    let query = format!(
        r#"
        MATCH "{PINNED_FILE_CLASS}"(?f) {{
            "{PROP_MEDIA_TYPE}": ?mt,
            "{PROP_REFERENCE}": ?ref{mt_clause}
        }}
        RETURN [] {{ iri: ?f, media_type: ?mt, reference: ?ref }}
    "#,
    );
    let rows = crate::run_query(&mut client, &query).await;
    if json {
        println!("{}", serde_json::to_string(&rows).unwrap());
    } else if rows.is_empty() {
        println!("No pinned external files attached.");
    } else {
        println!("PinnedExternalFiles:");
        for r in &rows {
            let iri = r.get("iri").and_then(|v| v.as_str()).unwrap_or("?");
            let mt = r.get("media_type").and_then(|v| v.as_str()).unwrap_or("?");
            let rf = r.get("reference").and_then(|v| v.as_str()).unwrap_or("?");
            println!("  {iri}  [{mt}]  {rf}");
        }
    }
}

/// Implements `eigenius data inspect <iri>`.
pub async fn data_inspect(endpoint: &str, iri: &str, json: bool) {
    let mut client = crate::connect_client(endpoint).await;
    let resource = match fetch_resource(&mut client, iri).await {
        Some(r) => r,
        None => {
            eprintln!("No resource at IRI `{iri}`");
            std::process::exit(1);
        }
    };
    if json {
        println!(
            "{}",
            serde_json::to_string(&eigon_json::serialize_resource(&resource)).unwrap()
        );
        return;
    }
    let read = |prop: &str| {
        Iri::parse(prop)
            .ok()
            .and_then(|i| resource.get(&i).cloned())
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "(not set)".to_string())
    };
    println!("PinnedExternalFile: {iri}");
    println!("  reference:    {}", read(PROP_REFERENCE));
    println!("  content_hash: {}", read(PROP_CONTENT_HASH));
    println!("  media_type:   {}", read(PROP_MEDIA_TYPE));
    println!("  schema:       {}", read("urn:eigenius:ingest:schema"));
    println!("  source:       {}", read(PROP_SOURCE));
}
