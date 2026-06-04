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

//! Serde structs for the OBO Graphs JSON format
//! ([schema](https://github.com/geneontology/obographs/blob/master/schema/obographs-schema.json)).
//!
//! The schema is permissive — almost every field is optional, and
//! ontologies in the wild vary in which fields they populate. The
//! structs below mirror the schema literally with `#[serde(default)]`
//! everywhere so dropped fields parse cleanly and unknown extension
//! fields don't break ingestion.
//!
//! Field names match the OBO-JSON wire format (`lbl`, `sub`, `pred`,
//! etc.) so we never need a `#[serde(rename)]` per field — the rare
//! `propertyType` and `meta` keys aside, which would otherwise collide
//! with Rust idiom.

use serde::{Deserialize, Serialize};

/// Top-level OBO graphs document. Multiple graphs per document is
/// permitted by the schema but rare in practice — most ontology
/// dumps ship a single graph.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct GraphDocument {
    #[serde(default)]
    pub graphs: Vec<Graph>,
    /// Document-level metadata (version IRIs, etc.). Not consumed by
    /// the converter today; kept for diagnostic purposes.
    #[serde(default)]
    pub meta: Option<Meta>,
}

/// One named ontology graph within a document.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Graph {
    /// Graph IRI — `http://purl.obolibrary.org/obo/go.owl` etc.
    #[serde(default)]
    pub id: Option<String>,
    /// Optional human-readable label.
    #[serde(default)]
    pub lbl: Option<String>,
    #[serde(default)]
    pub meta: Option<Meta>,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    /// Equivalent-class axioms — pairs/sets of nodes that are
    /// asserted equivalent under OWL semantics. Not consumed by the
    /// converter today.
    #[serde(default)]
    pub equivalent_nodes_sets: Vec<serde_json::Value>,
    /// Logical-definition axioms (`C ≡ G and (P some D)`). Not
    /// consumed by the converter today.
    #[serde(default)]
    pub logical_definition_axioms: Vec<serde_json::Value>,
    /// Domain/range axioms on properties. Not consumed today.
    #[serde(default)]
    pub domain_range_axioms: Vec<serde_json::Value>,
    /// Property-chain axioms. Not consumed today.
    #[serde(default)]
    pub property_chain_axioms: Vec<serde_json::Value>,
}

/// One node — Class, Individual, or Property.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Node {
    /// Full IRI of the node — used as-is for the corresponding
    /// Eigon Resource's `@id`.
    pub id: String,
    /// Human-readable label, typically the class/property name in
    /// natural language.
    #[serde(default)]
    pub lbl: Option<String>,
    /// `CLASS` / `INDIVIDUAL` / `PROPERTY`. Defaults to the empty
    /// string when absent (treated as an unknown node — surfaced as
    /// a typeless Resource by the converter).
    #[serde(default, rename = "type")]
    pub node_type: Option<String>,
    /// For PROPERTY nodes: `ANNOTATION` / `OBJECT` / `DATA`.
    /// Drives the Eigon-side `data_type` mapping (`string` for
    /// annotation/data, `resource` for object).
    #[serde(default, rename = "propertyType")]
    pub property_type: Option<String>,
    #[serde(default)]
    pub meta: Option<Meta>,
}

/// One edge — typed triple between two nodes.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Edge {
    /// Subject node IRI.
    pub sub: String,
    /// Predicate — either the bare string `"is_a"` (the
    /// rdfs:subClassOf shorthand) or a full IRI of a Property node.
    pub pred: String,
    /// Object node IRI.
    pub obj: String,
    #[serde(default)]
    pub meta: Option<Meta>,
}

/// OBO meta block — attached to graphs, nodes, edges, axioms.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Meta {
    #[serde(default)]
    pub definition: Option<DefinitionPropertyValue>,
    #[serde(default)]
    pub comments: Vec<String>,
    #[serde(default)]
    pub subsets: Vec<String>,
    #[serde(default)]
    pub synonyms: Vec<SynonymPropertyValue>,
    #[serde(default)]
    pub xrefs: Vec<XrefPropertyValue>,
    #[serde(default, rename = "basicPropertyValues")]
    pub basic_property_values: Vec<BasicPropertyValue>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub deprecated: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct DefinitionPropertyValue {
    #[serde(default)]
    pub val: Option<String>,
    #[serde(default)]
    pub xrefs: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct SynonymPropertyValue {
    /// Synonym scope — typically `hasExactSynonym`,
    /// `hasRelatedSynonym`, `hasBroadSynonym`, `hasNarrowSynonym`.
    #[serde(default)]
    pub pred: Option<String>,
    #[serde(default)]
    pub val: Option<String>,
    #[serde(default)]
    pub xrefs: Vec<String>,
    #[serde(default, rename = "synonymType")]
    pub synonym_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct XrefPropertyValue {
    #[serde(default)]
    pub val: Option<String>,
    #[serde(default)]
    pub lbl: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct BasicPropertyValue {
    #[serde(default)]
    pub pred: Option<String>,
    #[serde(default)]
    pub val: Option<String>,
}
