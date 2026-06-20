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

//! Core dispatch logic for the schema.org runtime worker (D60 §4.1, D57 Level 2).
//!
//! The worker is invoked by the substrate exactly like the R worker: spawned in
//! its pinned image, then sent `DispatchMethod { inputs }` over the UDS RPC. Each
//! input is an Eigon-CBOR `Resource`; the substrate has already fetched +
//! content-verified the pinned `PinnedExternalFile` and stamped its local path on
//! `ingest:materialized_path` (D53 §5/§7). This module is the body of that
//! dispatch — kept in the lib (not the bin's serve loop) so it is unit-testable
//! in-process without Docker. It returns the conversion-report `Resource` as
//! Eigon-CBOR (`Response::DispatchOk.output`); the kernel stamps the
//! invocation-declared `canonical_proposition` and applies the `ProgramTrace` /
//! `IsDerivedAs` witness.

use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::Value;
use eigenius_schemaorg::report::{build_report, RESULT_IRI};
use eigenius_schemaorg::{convert, parse_graph};
use serde_bytes::ByteBuf;
use sha2::{Digest, Sha256};

const MATERIALIZED_PATH: &str = "urn:eigenius:ingest:materialized_path";

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Run the conversion for one `DispatchMethod`: read the single pinned
/// schema.org JSON-LD input, convert it, and return the conversion-report
/// `Resource` as Eigon-CBOR. Errors are returned as strings the bin maps onto
/// `Response::DispatchFailed`.
pub fn run_conversion(inputs: &[ByteBuf]) -> Result<Vec<u8>, String> {
    let input = inputs.first().ok_or_else(|| {
        "schemaorg worker expects exactly one input (the schema.org JSON-LD)".to_string()
    })?;
    let resource = eigon_cbor::parse_resource_lenient(input)
        .map_err(|e| format!("input is not a valid Eigon-CBOR resource: {e}"))?;
    let path = match resource.get(&iri(MATERIALIZED_PATH)) {
        Some(Value::String(s)) => s.clone(),
        _ => {
            return Err(format!(
                "input resource has no `{MATERIALIZED_PATH}` — the substrate must \
                 provision the pinned file before dispatch"
            ))
        }
    };

    let bytes =
        std::fs::read(&path).map_err(|e| format!("cannot read materialized input {path}: {e}"))?;
    let input_sha256 = sha256_hex(&bytes);
    let text = String::from_utf8(bytes).map_err(|e| format!("input is not valid UTF-8: {e}"))?;
    let nodes = parse_graph(&text).map_err(|e| format!("input does not parse as JSON-LD: {e}"))?;

    let report = convert(&nodes);
    // The canonical (compact) ontology serialization is the artifact the chain
    // pins as `gen_output`; hash it to record the input→output provenance.
    let doc = eigon_json::serialize_document(&report.resources);
    let output_json = serde_json::to_string(&doc)
        .map_err(|e| format!("cannot serialize generated ontology: {e}"))?;
    let output_sha256 = sha256_hex(output_json.as_bytes());

    let result = build_report(RESULT_IRI, &input_sha256, &output_sha256, &report.coverage);
    Ok(eigon_cbor::serialize_resource(&result))
}

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("well-known IRI")
}

#[cfg(test)]
mod tests {
    use super::*;
    use eigenius_kernel::ontology::resource::Resource;

    fn input_resource_for(path: &str) -> ByteBuf {
        let mut r = Resource::new(iri("urn:eigenius:obj:d57:gen_input"));
        r.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::ResourceRef(iri(
                "urn:eigenius:ingest:PinnedExternalFile",
            ))]),
        );
        r.set(iri(MATERIALIZED_PATH), Value::String(path.to_string()));
        ByteBuf::from(eigon_cbor::serialize_resource(&r))
    }

    #[test]
    fn dispatch_reads_materialized_input_and_returns_report() {
        // A minimal schema.org-shaped @graph (one class, one property).
        let graph = r#"{"@graph":[
            {"@id":"schema:Thing","@type":"rdfs:Class","rdfs:label":"Thing"},
            {"@id":"schema:name","@type":"rdf:Property","rdfs:label":"name"}
        ]}"#;
        let dir =
            std::env::temp_dir().join(format!("schemaorg-worker-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("graph.jsonld");
        std::fs::write(&path, graph).unwrap();

        let inputs = vec![input_resource_for(path.to_str().unwrap())];
        let out_cbor = run_conversion(&inputs).expect("conversion succeeds");
        let report = eigon_cbor::parse_resource(&out_cbor).expect("report decodes");

        assert_eq!(report.id().map(|i| i.as_str()), Some(RESULT_IRI));
        // The report records the input→output content hashes…
        let in_hash = sha256_hex(graph.as_bytes());
        assert_eq!(
            report
                .get(&iri("urn:eigenius:obj:d57:input_content_hash"))
                .and_then(Value::as_str),
            Some(format!("sha256:{in_hash}").as_str()),
        );
        // …and embeds the coverage (one class mapped).
        let cov = report
            .get(&iri("urn:eigenius:obj:d57:coverage"))
            .expect("coverage");
        let Value::Json(j) = cov else {
            panic!("coverage json")
        };
        assert_eq!(j["classes"], serde_json::json!(1));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dispatch_without_materialized_path_errors() {
        let mut r = Resource::new(iri("urn:eigenius:obj:d57:gen_input"));
        r.set(
            iri("urn:eigenius:core:is_a"),
            Value::Array(vec![Value::ResourceRef(iri(
                "urn:eigenius:ingest:PinnedExternalFile",
            ))]),
        );
        let inputs = vec![ByteBuf::from(eigon_cbor::serialize_resource(&r))];
        let err =
            run_conversion(&inputs).expect_err("must fail closed without a materialized path");
        assert!(err.contains("materialized_path"));
    }
}
