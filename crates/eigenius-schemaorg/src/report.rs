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

//! The conversion-report `DerivedResource` the schema.org tool emits as its
//! Eigon-CBOR result (D60 §4.1, Level 2).
//!
//! Under the generic `oci` tool-runtime contract the tool is a *pure transform*:
//! it reads the pinned schema.org input, runs [`crate::convert`], and returns
//! **only the bare data** of the run — the generated ontology's `content_hash`
//! plus the coverage accounting — encoded as Eigon-CBOR. It does **not** set the
//! `canonical_proposition` (that is invocation-declared and kernel-stamped), nor
//! the `ProgramTrace` / `IsDerivedAs` witness (kernel-applied). Keeping the
//! proposition off the tool is what lets the runtime stay language-agnostic: a
//! `Prop` is a D47-encoded term a generic containerized tool has no business
//! constructing.

use eigenius_kernel::ontology::eigon_cbor;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};

use crate::convert::Coverage;

/// Default IRI of the conversion-report result for the standalone tool. The
/// program-run path may instead assign a deterministic output IRI (D56); this is
/// the default when the tool is invoked directly.
pub const RESULT_IRI: &str = "urn:eigenius:obj:d57:generate_result";

const IS_A: &str = "urn:eigenius:core:is_a";
const DERIVED_RESOURCE: &str = "urn:eigenius:reflection:DerivedResource";
const SOURCE: &str = "urn:eigenius:reflection:source";
const OUTPUT_CONTENT_HASH: &str = "urn:eigenius:obj:d57:output_content_hash";
const INPUT_CONTENT_HASH: &str = "urn:eigenius:obj:d57:input_content_hash";
const COVERAGE: &str = "urn:eigenius:obj:d57:coverage";
const CANONICAL_PROPOSITION: &str = "urn:eigenius:reflection:canonical_proposition";
/// The `obj:GeneratorConforms` predicate the chain's m3 conformance leg uses.
const GENERATOR_CONFORMS: &str = "urn:eigenius:obj:d57:GeneratorConforms";
/// The subject the schema.org objective is about.
pub const SUBJECT: &str = "schema_org";

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("well-known IRI")
}

/// Build the conversion-report `DerivedResource`: the tool's bare-data result.
///
/// `input_sha256` / `output_sha256` are the hex digests (no `sha256:` prefix) of
/// the pinned input JSON-LD and the generated ontology; this records the
/// input→output provenance the kernel's `RuntimeInvocation` corroborates. The
/// coverage is embedded as an opaque JSON payload (the m4 cut accounting).
pub fn build_report(
    id: &str,
    input_sha256: &str,
    output_sha256: &str,
    coverage: &Coverage,
) -> Resource {
    let mut r = Resource::new(iri(id));
    r.set(
        iri(IS_A),
        Value::Array(vec![Value::ResourceRef(iri(DERIVED_RESOURCE))]),
    );
    r.set(
        iri(INPUT_CONTENT_HASH),
        Value::String(format!("sha256:{input_sha256}")),
    );
    r.set(
        iri(OUTPUT_CONTENT_HASH),
        Value::String(format!("sha256:{output_sha256}")),
    );
    r.set(
        iri(COVERAGE),
        Value::Json(serde_json::to_value(coverage).expect("Coverage serializes to JSON")),
    );
    r.set(
        iri(SOURCE),
        Value::String(format!(
            "schemaorg-import: convert(schema.org JSON-LD sha256:{input_sha256}) \
             -> ontology sha256:{output_sha256}"
        )),
    );
    // The worker is Eigenius-aware (links the kernel), so it sets its own
    // canonical_proposition — `obj:GeneratorConforms("schema_org")` — exactly as
    // the WRN R worker sets `r_eigon_set_proposition` (D55/D56). The committed
    // ProgramTrace then mints `IsDerivedAs(<this resource>, GeneratorConforms)`,
    // which the chain's `concl_generator` discharges via `derived(...)` (D60 §4.1
    // tool-set path; the generic invocation-declared path is for non-Eigenius
    // tools). The term shape is the D47 App-spine the reasoning institution reads:
    // `App(ConstRef(pred), LitString(arg))`.
    r.set(
        iri(CANONICAL_PROPOSITION),
        Value::Json(serde_json::json!({
            "ctor": "App",
            "args": [
                {"ctor": "ConstRef", "args": [GENERATOR_CONFORMS]},
                {"ctor": "LitString", "args": [SUBJECT]}
            ]
        })),
    );
    r
}

/// Serialize the report as Eigon-CBOR — the `oci` runtime's result wire format.
pub fn report_to_cbor(report: &Resource) -> Vec<u8> {
    eigon_cbor::serialize_resource(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_coverage() -> Coverage {
        let mut c = Coverage {
            classes: 683,
            enumeration_classes: 51,
            enumeration_members: 250,
            properties: 1130,
            enumeration_open: 4,
            excluded_layer: 848,
            ..Default::default()
        };
        c.property_tiers.insert("Enumeration".into(), 66);
        c
    }

    #[test]
    fn report_round_trips_through_eigon_cbor() {
        let cov = sample_coverage();
        let report = build_report(RESULT_IRI, "0f0c97a4", "f4de231a", &cov);
        let bytes = report_to_cbor(&report);
        let back = eigon_cbor::parse_resource(&bytes).expect("report decodes from CBOR");

        // The result is identified and typed as a derivation result.
        assert_eq!(back.id().map(|i| i.as_str()), Some(RESULT_IRI));
        assert!(back.is_a().iter().any(|c| c.as_str() == DERIVED_RESOURCE));

        // The input→output provenance survives the round trip exactly.
        assert_eq!(
            back.get(&iri(OUTPUT_CONTENT_HASH)).and_then(Value::as_str),
            Some("sha256:f4de231a"),
        );
        assert_eq!(
            back.get(&iri(INPUT_CONTENT_HASH)).and_then(Value::as_str),
            Some("sha256:0f0c97a4"),
        );

        // The worker sets its own canonical_proposition — GeneratorConforms("schema_org")
        // as the D47 App-spine — so the program-run's IsDerivedAs matches the chain's
        // derived(...) certificate.
        let Some(Value::Json(prop)) = back.get(&iri(CANONICAL_PROPOSITION)) else {
            panic!("report must carry canonical_proposition");
        };
        assert_eq!(prop["ctor"], serde_json::json!("App"));
        assert_eq!(
            prop["args"][0]["args"][0],
            serde_json::json!(GENERATOR_CONFORMS)
        );
        assert_eq!(prop["args"][1]["args"][0], serde_json::json!(SUBJECT));

        // The coverage payload round-trips and carries the m4 accounting.
        let cov_back = back.get(&iri(COVERAGE)).expect("coverage present");
        let Value::Json(j) = cov_back else {
            panic!("coverage should round-trip as an opaque JSON payload, got {cov_back:?}");
        };
        assert_eq!(j["classes"], serde_json::json!(683));
        assert_eq!(j["enumeration_open"], serde_json::json!(4));
        assert_eq!(j["property_tiers"]["Enumeration"], serde_json::json!(66));
    }
}
