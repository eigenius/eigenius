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
// CLI surface — external institution installation, listing, and
// inspection (`institution install` / `list` / `inspect`).

use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::server::proto;

use crate::common::fetch_resource;

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
        // Default policy (Reject{100}) and no explicit tombstones —
        // this surface predates D41's policy wire-through.
        policy: None,
        explicit_tombstones: Vec::new(),
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
