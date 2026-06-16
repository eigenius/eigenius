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

use crate::common::{fetch_resource, submit_resource_for_load};

const PINNED_FILE_CLASS: &str = "urn:eigenius:ingest:PinnedExternalFile";
const PROP_REFERENCE: &str = "urn:eigenius:ingest:reference";
const PROP_CONTENT_HASH: &str = "urn:eigenius:ingest:content_hash";
const PROP_MEDIA_TYPE: &str = "urn:eigenius:ingest:media_type";
const PROP_SOURCE: &str = "urn:eigenius:reflection:source";
const PROP_IS_A: &str = "urn:eigenius:core:is_a";
const PROP_SHORT_NAME: &str = "urn:eigenius:core:short_name";

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

/// Implements `eigenius data attach <file> [--reference …] [--media-type …]`.
/// Reads the local file, computes its content hash, mints the
/// content-addressed `PinnedExternalFile` IRI, and commits the node. The
/// `reference` is the *durable* locator the substrate will later fetch from
/// (defaults to a `file://` URL of the absolute path); the local `<file>` is
/// only read here to compute the hash. Idempotent — byte-identical files
/// converge to one IRI (D53 §3).
#[allow(clippy::too_many_arguments)]
pub async fn data_attach(
    endpoint: &str,
    file: &str,
    reference_override: Option<&str>,
    media_type_override: Option<&str>,
    name_override: Option<&str>,
    json: bool,
) {
    let bytes = match std::fs::read(file) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Failed to read file `{file}`: {e}");
            std::process::exit(1);
        }
    };
    let content_hash = eigenius_runtime_substrate::content_hash_of(&bytes);
    let iri_str = match eigenius_runtime_substrate::pinned_external_file_iri(&content_hash) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Cannot mint IRI: {e}");
            std::process::exit(1);
        }
    };
    let iri = Iri::parse(&iri_str).expect("content-addressed IRI is well-formed");

    // Durable locator: default to a file:// URL of the absolute path.
    let reference = match reference_override {
        Some(r) => r.to_string(),
        None => {
            let abs = std::fs::canonicalize(file).unwrap_or_else(|_| file.into());
            format!("file://{}", abs.display())
        }
    };
    let media_type = media_type_override
        .map(str::to_string)
        .unwrap_or_else(|| media_type_for(file).to_string());
    let short_name = name_override
        .map(str::to_string)
        .or_else(|| {
            std::path::Path::new(file)
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
