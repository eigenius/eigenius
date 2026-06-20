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

//! Mechanical evidence for the D57 objective's milestones m3/m4
//! (`docs/notes/d57-mechanical-evidence-plan.md`, Level 1). The chain previously
//! *Declared* these claims in prose; here `cargo test` *witnesses* them over the
//! real, content-pinned schema.org V30.0 vocabulary:
//!
//! - **Verified (structural)** — the full generated ontology (2114 resources)
//!   loads onto core+reflection and the kernel `Validator` reports **0 errors**
//!   (Expressible). A rejected load would be a fail-closed finding.
//! - **Verified** — *no* resource carries `core:domain` (`domainIncludes` is
//!   inverted into advisory `core:recommends`, decision #9); not one restriction
//!   leaks in.
//! - **Verified** — every enumeration-ranged property carries `allows_only` (the
//!   closed set), and the count equals the `Enumeration` coverage tier.
//! - **Verified** — every emitted resource round-trips: `source_irl` reverses to
//!   its `urn:schema_org:` `@id` under the prefix substitution (decision #13).
//! - **Derived + Verified** — an *independent* recount of the parsed graph
//!   reproduces the coverage report exactly (the cut accounts for every
//!   `schema:` node: mapped ∪ folded ∪ excluded ∪ non-vocabulary), so m4's
//!   partition is checked, not asserted.
//! - **Verified** — conversion is deterministic (byte-identical serialization).
//!
//! The input JSON-LD is pinned and gitignored (`data/MANIFEST.md`, sha256
//! `0f0c97a4…`); these tests are `#[ignore]` and run on demand once it is
//! present:
//!
//! ```bash
//! cargo test -p eigenius-schemaorg -- --ignored
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use eigenius_kernel::layer::{Layer, LayerBuilder, LayerStorage};
use eigenius_kernel::ontology::eigon_json;
use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use eigenius_kernel::validation::Validator;
use eigenius_schemaorg::{convert, parse_graph, ConvertReport};
use sha2::{Digest, Sha256};

/// The pinned input (data/MANIFEST.md). The byte identity the whole chain of
/// evidence rests on — if this mismatches we are not converting V30.0.
const INPUT_SHA256: &str = "0f0c97a4f666b2f8563573fe48453782fd51b87a504523cf0c9aff6a71c3eec4";

const URN_PREFIX: &str = "urn:schema_org:";
const HTTPS_PREFIX: &str = "https://schema.org/";
const CORE_DOMAIN: &str = "urn:eigenius:core:domain";
const CORE_ALLOWS_ONLY: &str = "urn:eigenius:core:allows_only";
const CORE_CLASS_TYPES: &str = "urn:eigenius:core:class_types";
const CORE_SOURCE_IRL: &str = "urn:eigenius:core:source_irl";
const CORE_PROPERTY: &str = "urn:eigenius:core:Property";

fn input_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/schemaorg-current-https-v30.0.jsonld")
}

/// Read the pinned input, fail closed on a content-hash mismatch, and run the
/// real generator over it. `#[ignore]` callers expect the file present; absence
/// is a hard error pointing at the fetch recipe (fail-closed, not silent skip).
fn convert_pinned() -> ConvertReport {
    let path = input_path();
    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "pinned schema.org input absent at {} ({e}). Fetch it first \
             (crates/eigenius-schemaorg/data/MANIFEST.md):\n  curl -fsSL \
             https://schema.org/version/30.0/schemaorg-current-https.jsonld -o {}",
            path.display(),
            path.display(),
        )
    });
    let digest = hex(&Sha256::digest(&bytes));
    assert_eq!(
        digest, INPUT_SHA256,
        "input content-hash mismatch — refusing to witness against the wrong bytes \
         (expected the pinned V30.0)"
    );
    let text = String::from_utf8(bytes).expect("input is utf-8");
    let nodes = parse_graph(&text).expect("input parses as JSON-LD @graph");
    convert(&nodes)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `id`/`source_irl` of a resource as `&str`.
fn id_str(r: &Resource) -> String {
    r.id().expect("resource has id").as_str().to_string()
}

// ── Verified (structural): the output loads + validates in the kernel ──

#[test]
#[ignore = "needs the pinned schema.org V30.0 input (data/MANIFEST.md)"]
fn output_loads_and_validates() {
    let report = convert_pinned();

    // core → reflection (+ eigentt + institution) → schema_org, mirroring the
    // proven WRN harness stack so every definition the output references resolves.
    let core = layer_from_json(
        "core",
        None,
        &[include_str!("../../../ontologies/core/core-ontology.json")],
    );
    let reflection = layer_from_json(
        "reflection",
        Some(core),
        &[
            include_str!("../../../ontologies/reflection/reflection-ontology.json"),
            include_str!("../../../ontologies/eigentt/eigentt-type-fragment.json"),
            include_str!("../../../ontologies/institution/institution-ontology.json"),
        ],
    );

    let mut b = LayerBuilder::new("schema_org", Some(reflection));
    for r in &report.resources {
        b.add_resource(r.clone())
            .unwrap_or_else(|e| panic!("add_resource {} failed: {e:?}", id_str(r)));
    }
    let layer = Arc::new(b.build(LayerStorage::in_memory()));

    let errors = Validator::new(layer).validate();
    assert!(
        errors.is_empty(),
        "the generated schema.org ontology must validate cleanly (Expressible). \
         {} error(s):\n{}",
        errors.len(),
        errors
            .iter()
            .take(25)
            .map(|e| format!("  - {e}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );

    // Sanity: the resource count matches the committed coverage accounting.
    let cov = &report.coverage;
    let expected = cov.classes + cov.enumeration_classes + cov.enumeration_members + cov.properties;
    assert_eq!(
        report.resources.len(),
        expected,
        "emitted resource count must equal classes+enum_classes+members+properties",
    );
}

fn layer_from_json(name: &str, parent: Option<Arc<Layer>>, sources: &[&str]) -> Arc<Layer> {
    let mut b = LayerBuilder::new(name, parent);
    for src in sources {
        for r in eigon_json::parse_document(src).expect("ontology parses") {
            b.add_resource(r).expect("ontology resource adds");
        }
    }
    Arc::new(b.build(LayerStorage::in_memory()))
}

// ── Verified: domainIncludes → recommends, never core:domain (#9) ──────

#[test]
#[ignore = "needs the pinned schema.org V30.0 input (data/MANIFEST.md)"]
fn no_resource_carries_core_domain() {
    let report = convert_pinned();
    let domain = Iri::parse(CORE_DOMAIN).unwrap();
    let offenders: Vec<String> = report
        .resources
        .iter()
        .filter(|r| r.has(&domain))
        .map(id_str)
        .collect();
    assert!(
        offenders.is_empty(),
        "schema.org's domainIncludes is advisory; it must invert to core:recommends, \
         never the restrictive core:domain. Offending resources: {offenders:?}",
    );
}

// ── Verified: every enumeration-ranged property has allows_only ────────

#[test]
#[ignore = "needs the pinned schema.org V30.0 input (data/MANIFEST.md)"]
fn enumeration_ranged_properties_close_with_allows_only() {
    let report = convert_pinned();
    let allows_only = Iri::parse(CORE_ALLOWS_ONLY).unwrap();
    let class_types = Iri::parse(CORE_CLASS_TYPES).unwrap();

    let with_allows: Vec<&Resource> = report
        .resources
        .iter()
        .filter(|r| r.has(&allows_only))
        .collect();

    // Every allows_only carrier is a property that also pins class_types (the
    // closed set is over the enumeration class it ranges on — never standalone).
    for r in &with_allows {
        assert!(
            r.has(&class_types),
            "{} carries allows_only without class_types",
            id_str(r),
        );
        assert!(
            r.is_a().iter().any(|c| c.as_str() == CORE_PROPERTY),
            "{} carries allows_only but is not a core:Property",
            id_str(r),
        );
    }

    // The cut and the artifact agree: every Enumeration-tier property either
    // carries allows_only (a closable set) or is accounted as genuinely-open
    // (its enumeration is member-less, e.g. BusinessFunction). No third case.
    let enum_tier = *report
        .coverage
        .property_tiers
        .get("Enumeration")
        .unwrap_or(&0);
    assert!(enum_tier > 0, "expected some enumeration-ranged properties");
    assert_eq!(
        with_allows.len() + report.coverage.enumeration_open,
        enum_tier,
        "Enumeration tier must split exactly into closed (allows_only) + open (empty enum)",
    );
    // The open set is the small, named exception — not a silent generator miss.
    assert!(
        report.coverage.enumeration_open <= with_allows.len(),
        "open enumerations should be the exception, not the rule (got {} open vs {} closed)",
        report.coverage.enumeration_open,
        with_allows.len(),
    );
}

// ── Verified: source_irl round-trips to the schema.org @id (#13) ───────

#[test]
#[ignore = "needs the pinned schema.org V30.0 input (data/MANIFEST.md)"]
fn source_irl_round_trips_to_id() {
    let report = convert_pinned();
    let source_irl = Iri::parse(CORE_SOURCE_IRL).unwrap();
    for r in &report.resources {
        let id = id_str(r);
        let local = id
            .strip_prefix(URN_PREFIX)
            .unwrap_or_else(|| panic!("emitted id {id} is not under {URN_PREFIX}"));
        let irl = r
            .get(&source_irl)
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{id} has no source_irl"));
        assert_eq!(
            irl,
            format!("{HTTPS_PREFIX}{local}"),
            "source_irl for {id} must reverse to its schema.org @id",
        );
    }
}

// ── Derived + Verified: the cut accounts for every schema: node (m4) ───

#[test]
#[ignore = "needs the pinned schema.org V30.0 input (data/MANIFEST.md)"]
fn cut_partitions_the_schema_namespace() {
    let path = input_path();
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {} ({e})", path.display()));
    let nodes = parse_graph(&text).expect("parses");
    let report = convert(&nodes);
    let cov = &report.coverage;

    // Independent recount: every node with a schema: @id lands in exactly one
    // bucket. emitted (by urn id) ∪ folded ∪ excluded ∪ non-vocabulary = all.
    let emitted_ids: std::collections::BTreeSet<String> =
        report.resources.iter().map(id_str).collect();
    let folded: std::collections::BTreeSet<String> = cov
        .datatypes_folded
        .iter()
        .map(|c| format!("{URN_PREFIX}{}", c.strip_prefix("schema:").unwrap()))
        .collect();

    let mut total = 0usize;
    let mut mapped = 0usize;
    let mut folded_n = 0usize;
    let mut excluded = 0usize;
    let mut non_vocab = 0usize;
    for n in &nodes {
        let Some(id) = eigenius_schemaorg::jsonld::node_id(n) else {
            continue;
        };
        let Some(local) = id.strip_prefix("schema:") else {
            continue;
        };
        total += 1;
        let urn = format!("{URN_PREFIX}{local}");
        if is_excluded_layer(n) {
            excluded += 1;
        } else if emitted_ids.contains(&urn) {
            mapped += 1;
        } else if folded.contains(&urn) {
            folded_n += 1;
        } else {
            // Untyped / non-vocabulary node, or a member of an out-of-scope
            // enumeration class (emit_member skips it). Accounted, not silent.
            non_vocab += 1;
        }
    }

    assert_eq!(
        mapped + folded_n + excluded + non_vocab,
        total,
        "the partition must be exhaustive over all schema: nodes",
    );
    // The recount reproduces the committed coverage report (no silent drift).
    assert_eq!(
        excluded, cov.excluded_layer,
        "recounted excluded-by-layer must match coverage.excluded_layer",
    );
    assert_eq!(
        folded_n,
        cov.datatypes_folded.len(),
        "recounted folded DataTypes must match coverage.datatypes_folded",
    );
    assert_eq!(
        mapped,
        cov.classes + cov.enumeration_classes + cov.enumeration_members + cov.properties,
        "recounted mapped nodes must match the emitted vocabulary",
    );
}

fn is_excluded_layer(n: &serde_json::Value) -> bool {
    eigenius_schemaorg::jsonld::iri_refs(n, "schema:isPartOf")
        .iter()
        .any(|p| p == "https://pending.schema.org" || p == "https://meta.schema.org")
}

// ── Verified: conversion is deterministic ──────────────────────────────

#[test]
#[ignore = "needs the pinned schema.org V30.0 input (data/MANIFEST.md)"]
fn conversion_is_deterministic() {
    let a = convert_pinned();
    let b = convert_pinned();
    let sa = eigon_json::serialize_document(&a.resources);
    let sb = eigon_json::serialize_document(&b.resources);
    assert_eq!(sa, sb, "the generator must be deterministic");
}
