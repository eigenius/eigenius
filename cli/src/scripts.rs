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
// Script commands (D26 §10): `script publish`, `script list`,
// `script inspect`, `script run`.

use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::server::proto;
use std::path::Path;

use crate::common::{fetch_resource, submit_resource_for_load};

// --- Script commands (D26 §10) --------------------------------------------

/// Map a script file extension to a substrate language identifier. The
/// language drives both IRI namespacing and worker dispatch (D26 §5.1).
fn language_for_extension(file: &str) -> Option<&'static str> {
    let lower = file.to_ascii_lowercase();
    if lower.ends_with(".r") {
        Some("r")
    } else if lower.ends_with(".jl") {
        Some("julia")
    } else if lower.ends_with(".py") {
        Some("python")
    } else if lower.ends_with(".lean") {
        Some("lean")
    } else {
        None
    }
}

/// Implements `eigenius script publish <file> --env <env-iri>`. Reads the
/// script source, mints the content-addressed `RuntimeScript` IRI (D26
/// §5.1), and commits the resource. A cheap graph commit — the heavy
/// operation is `env create`, which builds the image. Re-publishing an
/// identical script against the same environment is idempotent: the
/// content-addressed IRI collides and the chain deduplicates.
#[allow(clippy::too_many_arguments)]
pub async fn script_publish(
    endpoint: &str,
    file: &str,
    env_iri: &str,
    language_override: Option<&str>,
    entry_point: Option<&str>,
    description: Option<&str>,
    json: bool,
) {
    let source = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to read script file `{file}`: {e}");
            std::process::exit(1);
        }
    };

    let language = match language_override.or_else(|| language_for_extension(file)) {
        Some(l) => l.to_string(),
        None => {
            eprintln!(
                "Cannot infer language from `{file}` (expected .r/.jl/.py/.lean); pass --lang."
            );
            std::process::exit(1);
        }
    };

    let env = match Iri::parse(env_iri) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("--env is not a valid IRI: {e}");
            std::process::exit(1);
        }
    };

    // Content-addressed identity (D26 §5.1). `entry_point_signature` is
    // not exposed on this surface — top-level scripts have no typed
    // entry point; a script that needs one declares it via a published
    // RuntimeMethodSignature and the CallRuntimeMethod surface.
    let iri_str = eigenius_runtime_substrate::RuntimeScriptIdentity {
        language: &language,
        source: &source,
        entry_point,
        entry_point_signature: None,
        requires_environment: env.as_str(),
    }
    .content_addressed_iri();
    let script_iri = Iri::parse(&iri_str).expect("content-addressed IRI is well-formed");

    // Derive a `short_name` from the file stem — the same convention the
    // other runtime verbs use for synthesised resources.
    let short_name = Path::new(file)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("script")
        .to_string();

    let mut script = Resource::new(script_iri.clone());
    script.set(
        Iri::parse("urn:eigenius:core:is_a").expect("static IRI"),
        Value::Array(vec![Value::ResourceRef(
            Iri::parse("urn:eigenius:runtime:RuntimeScript").expect("static IRI"),
        )]),
    );
    script.set(
        Iri::parse("urn:eigenius:core:short_name").expect("static IRI"),
        Value::String(short_name),
    );
    script.set(
        Iri::parse("urn:eigenius:runtime:language").expect("static IRI"),
        Value::String(language.clone()),
    );
    script.set(
        Iri::parse("urn:eigenius:runtime:source").expect("static IRI"),
        Value::String(source),
    );
    script.set(
        Iri::parse("urn:eigenius:runtime:requires_environment").expect("static IRI"),
        Value::ResourceRef(env.clone()),
    );
    if let Some(ep) = entry_point {
        script.set(
            Iri::parse("urn:eigenius:runtime:entry_point").expect("static IRI"),
            Value::String(ep.to_string()),
        );
    }
    if let Some(desc) = description {
        script.set(
            Iri::parse("urn:eigenius:core:description").expect("static IRI"),
            Value::String(desc.to_string()),
        );
    }

    let mut client = crate::connect_client(endpoint).await;
    submit_resource_for_load(&mut client, &script).await;

    if json {
        println!(
            "{{\"success\":true,\"script_iri\":\"{}\",\"language\":\"{}\",\"environment\":\"{}\"}}",
            script_iri.as_str(),
            language,
            env.as_str()
        );
    } else {
        println!("RuntimeScript published.");
        println!("  IRI: {}", script_iri.as_str());
        println!("  Language: {language}");
        println!("  Environment: {}", env.as_str());
    }
}

/// Implements `eigenius script list [--lang <name>]`.
pub async fn script_list(endpoint: &str, language: Option<&str>, json: bool) {
    let mut client = crate::connect_client(endpoint).await;
    let lang_clause = match language {
        Some(l) => format!(", \"urn:eigenius:runtime:language\": \"{l}\""),
        None => String::new(),
    };
    let query = format!(
        r#"
        MATCH "urn:eigenius:runtime:RuntimeScript"(?s) {{
            "urn:eigenius:core:short_name": ?name,
            "urn:eigenius:runtime:language": ?lang{lang_clause}
        }}
        RETURN [] {{ iri: ?s, name: ?name, language: ?lang }}
    "#,
    );
    let rows = crate::run_query(&mut client, &query).await;
    if json {
        println!("{}", serde_json::to_string(&rows).unwrap());
    } else if rows.is_empty() {
        println!("No runtime scripts published.");
    } else {
        println!("RuntimeScripts:");
        for r in &rows {
            let iri = r.get("iri").and_then(|v| v.as_str()).unwrap_or("?");
            let name = r.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let lang = r.get("language").and_then(|v| v.as_str()).unwrap_or("?");
            println!("  {name} [{lang}] ({iri})");
        }
    }
}

/// Implements `eigenius script inspect <iri>`.
pub async fn script_inspect(endpoint: &str, iri: &str, json: bool) {
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
    let source = read("urn:eigenius:runtime:source");
    if json {
        println!(
            "{}",
            serde_json::to_string(&eigon_json::serialize_resource(&resource)).unwrap()
        );
    } else {
        println!("RuntimeScript: {iri}");
        println!("  Language: {}", read("urn:eigenius:runtime:language"));
        println!(
            "  Environment: {}",
            read("urn:eigenius:runtime:requires_environment")
        );
        println!(
            "  Entry point: {}",
            read("urn:eigenius:runtime:entry_point")
        );
        println!("  ---- source ----");
        println!("{source}");
    }
}

/// Implements `eigenius script run <iri> --inputs <iri,...>`. Builds a
/// `RunRuntimeScript` program that references the published script by IRI
/// (the kernel resolves source/env from the graph at execution, D26
/// §6.2) and dispatches it against the named graph-resident input.
///
/// v1 supports exactly one input: the substrate's `dispatch_run_runtime_script`
/// boundary is single-input (`&[input]`). Multi-input dispatch is a
/// substrate-boundary extension tracked separately.
pub async fn script_run(
    endpoint: &str,
    script_iri: &str,
    inputs: &[String],
    branch: Option<&str>,
    json: bool,
) {
    if inputs.len() != 1 {
        eprintln!(
            "`script run` v1 takes exactly one --inputs IRI (got {}); multi-input dispatch is a substrate-boundary extension.",
            inputs.len()
        );
        std::process::exit(1);
    }
    let input_iri = &inputs[0];

    let mut client = crate::connect_client(endpoint).await;

    // Resolve the input resource to (a) confirm it is graph-resident and
    // (b) read its class for the program's `input_type`.
    let input_resource = match fetch_resource(&mut client, input_iri).await {
        Some(r) => r,
        None => {
            eprintln!("Input resource `{input_iri}` not found on the chain.");
            std::process::exit(1);
        }
    };
    let input_type = input_resource
        .get(&Iri::parse("urn:eigenius:core:is_a").expect("static IRI"))
        .and_then(|v| match v {
            Value::Array(items) => items.first().and_then(|i| i.as_iri_str()),
            other => other.as_iri_str(),
        })
        .map(str::to_string)
        .unwrap_or_else(|| {
            eprintln!("Input resource `{input_iri}` has no `is_a` class.");
            std::process::exit(1);
        });

    // Build the program: Apply(RunRuntimeScript, Var "input") with a
    // component_argument referencing the published script by IRI. A
    // stable program @id derived from the script keeps repeated runs of
    // the same script anchored-commit-cacheable across inputs.
    let program_iri = format!("{script_iri}:run");
    let program = serde_json::json!([{
        "@id": program_iri,
        "urn:eigenius:core:is_a": ["urn:eigenius:program:Program"],
        "urn:eigenius:program:input_type": input_type,
        "urn:eigenius:program:output_type": "urn:eigenius:reflection:DerivedResource",
        "urn:eigenius:program:body": {
            "urn:eigenius:core:is_a": ["urn:eigenius:program:Apply"],
            "urn:eigenius:program:function": "urn:eigenius:program:components:RunRuntimeScript",
            "urn:eigenius:program:argument": {
                "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
                "urn:eigenius:program:name": "input"
            },
            "urn:eigenius:program:component_argument": {
                "urn:eigenius:runtime:script": script_iri
            }
        }
    }]);
    let program_bytes = serde_json::to_vec(&program).expect("program JSON serialises");

    // The input payload is the graph-resident resource, re-encoded as an
    // Eigon-JSON document (single-element array), exactly as `eigenius
    // run` feeds an input file.
    let input_json = eigon_json::serialize_resource(&input_resource);
    let input_bytes =
        serde_json::to_vec(&serde_json::json!([input_json])).expect("input JSON serialises");

    let request = proto::RunProgramRequest {
        program: program_bytes,
        input: input_bytes,
        content_type: "application/eigon+json".to_string(),
        branch: branch.unwrap_or("").to_string(),
    };

    match client.run_program(request).await {
        Ok(response) => {
            let resp = response.into_inner();
            if resp.success {
                let resource =
                    eigon_cbor::parse_resource_lenient(&resp.output).unwrap_or_else(|e| {
                        eprintln!("Failed to parse output: {e}");
                        std::process::exit(1);
                    });
                let out_json = eigon_json::serialize_resource(&resource);
                if json {
                    println!("{}", serde_json::to_string(&out_json).unwrap());
                } else {
                    println!("{}", serde_json::to_string_pretty(&out_json).unwrap());
                }
                if !resp.branch_advanced {
                    eprintln!(
                        "Note: trace layer reused from anchored-commit cache (branch unchanged)."
                    );
                }
            } else {
                eprintln!("Script run failed:");
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
