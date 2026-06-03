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

//! Import OBO Graphs JSON ontologies into Eigon-JSON Resources.
//!
//! Surface:
//!
//! - [`obo::GraphDocument`] — serde struct for the top-level OBO-JSON
//!   document; deserialise with `serde_json::from_str` /
//!   `serde_json::from_reader`.
//! - [`convert::convert_document`] — pure function from a
//!   `GraphDocument` to a flat list of `eigenius_kernel::ontology::resource::Resource`
//!   plus a soft-error report.
//! - [`bin/obograph-import`] — CLI wrapping the above for the
//!   one-shot file-to-file conversion used by the D43 M9
//!   life-science fixture pipeline.
//!
//! See `convert.rs`'s module docstring for the full OBO→Eigon
//! mapping spec and the v1 deferral list.

pub mod convert;
pub mod obo;

pub use convert::{
    convert_document, convert_document_with, rewrite_iri, ConvertError, ConvertOptions,
    ConvertReport,
};
pub use obo::GraphDocument;

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test against the upstream `nucleus.json` example
    /// (committed into the obographs reference repo at
    /// `examples/nucleus.json`) — a hand-crafted ten-CLASS GO slice
    /// with the full meta/edges feature set the converter must
    /// handle. Loaded at runtime rather than `include_str!`'d so the
    /// fixture path stays a single source of truth and the test
    /// fails loud if the upstream layout changes.
    const NUCLEUS_FIXTURE_PATH: &str = "../../../obographs/examples/nucleus.json";

    #[test]
    fn nucleus_fixture_converts_all_classes() {
        let json = match std::fs::read_to_string(NUCLEUS_FIXTURE_PATH) {
            Ok(s) => s,
            Err(_) => {
                eprintln!(
                    "skipping nucleus fixture test — `{NUCLEUS_FIXTURE_PATH}` not found relative \
                     to crate root. Check out https://github.com/geneontology/obographs to \
                     re-enable."
                );
                return;
            }
        };
        let doc: GraphDocument = serde_json::from_str(&json).expect("nucleus.json parses");
        let report = convert_document(&doc);

        // The fixture contains 10 GO CLASS nodes plus a handful of
        // shorthand PROPERTY nodes (BFO_0000050 / IAO_0000115 / oboInOwl
        // annotation properties). Sanity-check both counts are
        // non-trivially > 0; tighter assertions live alongside the
        // hand-rolled doc tests in `convert.rs`.
        let class_count = report.counts_by_type.get("CLASS").copied().unwrap_or(0);
        let property_count = report.counts_by_type.get("PROPERTY").copied().unwrap_or(0);
        assert!(
            class_count >= 10,
            "expected ≥10 CLASS nodes, got {class_count}"
        );
        assert!(
            property_count >= 5,
            "expected ≥5 PROPERTY nodes, got {property_count}"
        );
        assert!(
            report.errors.is_empty(),
            "unexpected soft errors: {:?}",
            report.errors
        );

        // Spot-check: the nucleus class (GO_0005634) carries its
        // definition through to `core:description`. The Resource's
        // `@id` is the URN-rewritten form per [`convert::rewrite_iri`];
        // its `source_irl` slot carries the original HTTP IRI.
        let nucleus = report
            .resources
            .iter()
            .find(|r| {
                r.id()
                    .map(|i| i.as_str() == "urn:obo:GO:0005634")
                    .unwrap_or(false)
            })
            .expect("nucleus Resource emitted");
        let desc_iri =
            eigenius_kernel::ontology::iri::Iri::parse("urn:eigenius:core:description").unwrap();
        match nucleus.get(&desc_iri) {
            Some(eigenius_kernel::ontology::resource::Value::String(s)) => {
                assert!(
                    s.contains("membrane-bounded organelle"),
                    "nucleus description must round-trip the OBO definition; got `{s}`"
                );
            }
            other => panic!("expected description String, got {other:?}"),
        }
        // Provenance: original HTTP IRI is preserved under
        // `core:source_irl` so downstream auditors can join with
        // external OBO data that still uses the HTTP form.
        let src_iri =
            eigenius_kernel::ontology::iri::Iri::parse("urn:eigenius:core:source_irl").unwrap();
        match nucleus.get(&src_iri) {
            Some(eigenius_kernel::ontology::resource::Value::String(s)) => {
                assert_eq!(s, "http://purl.obolibrary.org/obo/GO_0005634");
            }
            other => panic!("expected source_irl String, got {other:?}"),
        }
        // Declared knowledge tagging: the nucleus Resource is
        // structurally a DeclaredResource, attributed to the source
        // graph IRI.
        let declared_by_iri =
            eigenius_kernel::ontology::iri::Iri::parse("urn:eigenius:reflection:declared_by")
                .unwrap();
        match nucleus.get(&declared_by_iri) {
            Some(eigenius_kernel::ontology::resource::Value::String(s)) => {
                assert!(
                    s.contains("go-test") || s.contains("go.owl"),
                    "declared_by must point at the source graph; got `{s}`"
                );
            }
            other => panic!("expected declared_by String, got {other:?}"),
        }
    }
}
