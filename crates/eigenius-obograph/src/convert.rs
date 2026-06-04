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

//! OBO-JSON → Eigon-JSON conversion (D43 M9 life-science fixture
//! pipeline).
//!
//! **IRI namespace remap.** OBO ontologies use HTTP IRIs as opaque
//! identifiers (`http://purl.obolibrary.org/obo/GO_0005634`), but
//! Eigenius's IRI convention is the `urn:` scheme (see CLAUDE.md /
//! `urn:eigenius:<namespace>:<local-name>`). The converter rewrites
//! every HTTP IRI it can recognise into a stable URN and records the
//! original under `urn:eigenius:core:source_irl` for provenance. The
//! rewrite is applied uniformly: node `@id`s, edge subject /
//! predicate / object IRIs, every `ResourceRef` value. Same input
//! IRI always rewrites to the same URN, so cross-references stay
//! coherent across the document.
//!
//! Rewrite rules ([`rewrite_iri`]):
//!
//! - `http://purl.obolibrary.org/obo/<PREFIX>_<LOCAL>` →
//!   `urn:obo:<PREFIX>:<LOCAL>` (preserves the canonical OBO CURIE
//!   `GO:0005634` shape biologists already use).
//! - `http://purl.obolibrary.org/obo/<PREFIX>#<frag>` →
//!   `urn:obo:<PREFIX>:<frag>` (the `#`-fragment form for
//!   intra-ontology subsets, synonym types, etc.).
//! - `http://www.geneontology.org/formats/oboInOwl#<X>` →
//!   `urn:obo:oboInOwl:<X>` — OBO's RDF schema annotations.
//! - `http://www.w3.org/2000/01/rdf-schema#<X>` → `urn:rdfs:<X>`,
//!   `http://www.w3.org/2002/07/owl#<X>` → `urn:owl:<X>`.
//! - Any other HTTP IRI or already-URN IRI passes through unchanged
//!   (no `source_irl` slot emitted in that case — the IRI is its
//!   own provenance).
//!
//! **Epistemic tagging.** Imported ontology Resources represent
//! curator-asserted scientific knowledge — declared, not derived.
//! Every Resource the converter emits is tagged
//! `is_a: [..., urn:eigenius:reflection:DeclaredResource]` and
//! carries `urn:eigenius:reflection:declared_by` pointing at the
//! source graph IRI (or a CLI-supplied override). This satisfies
//! the DeclaredResource `requires` slot and makes provenance-aware
//! EigenQL queries (filter by `declared_by`, exclude derived
//! facts, etc.) possible against the imported corpus.
//!
//! **Mapping summary.** Per node, an Eigon [`Resource`] is emitted
//! whose `@id` is the node IRI (after [`rewrite_iri`]). The Resource
//! carries `urn:eigenius:core:is_a` driven by the node `type`:
//!
//! - `CLASS` → `[core:Class]`
//! - `PROPERTY` → `[core:Property]` plus a derived `data_type`:
//!   `ANNOTATION` / `DATA` → `core:string`; `OBJECT` → `core:resource`
//! - `INDIVIDUAL` → `[core:Resource]` (the catch-all super-class)
//! - missing / unknown `type` → typeless Resource (no `is_a`); this
//!   covers OBO's "naked" nodes (subset declarations, synonym types)
//!   that are referenced but never typed by the ontology itself.
//!
//! `lbl` populates `core:short_name`; `meta.definition.val`
//! populates `core:description`; `meta.deprecated == true` adds
//! `core:deprecated: true`. Edges with `pred == "is_a"` extend the
//! subject's `is_a` array; edges with any other predicate set the
//! IRI-keyed property on the subject (as a resource_array — the
//! converter accumulates multiple obj values under the same
//! `(sub, pred)` pair).
//!
//! **Synthetic-IRI fan-out.** OBO uses bare-string predicates in two
//! places — edges (`is_a`, `inverseOf`, `subPropertyOf`, `type`) and
//! synonym scopes (`hasExactSynonym`, `hasRelatedSynonym`, etc.).
//! `is_a` / `type` / `subPropertyOf` fold into the kernel's
//! `core:is_a`. Everything else maps into the `urn:obo:*` IRI
//! namespace via [`synonym_scope_to_iri`] and
//! [`resolve_predicate_iri`]. After the node + edge pass,
//! [`ensure_synthetic_property_declarations`] walks the accumulated
//! Resources, finds every `urn:obo:*` slot that doesn't yet have a
//! Property declaration, and synthesises one — *unless* the IRI is
//! listed in [`META_DECLARED_IRIS`], in which case it's covered by
//! the shared `ontologies/obo/obo-meta-ontology.json` layer that the
//! kernel loads at bootstrap and re-emitting would just shadow the
//! authoritative declaration. Synonym scopes and `inverseOf` are
//! all in the meta layer; ad-hoc `urn:obo:*` predicates the
//! converter discovers in real data still get a per-document
//! declaration. Synonym slots get `data_type: core:string`; edge-
//! derived slots get `data_type: core:resource`.
//!
//! **Synonyms ingested in v1.** Each OBO node's `meta.synonyms` array
//! collapses into per-scope arrays of strings on the Resource:
//! `urn:obo:has_exact_synonym`, `urn:obo:has_related_synonym`,
//! `urn:obo:has_broad_synonym`, `urn:obo:has_narrow_synonym`.
//! Synonyms with an unrecognised scope predicate are dropped (rare;
//! OBO ontologies overwhelmingly use the four scopes above).
//!
//! **v1 explicit deferrals**, recorded so future passes don't
//! re-discover them:
//!
//! - `xrefs`, `comments`, `subsets`, `basicPropertyValues` are
//!   dropped. Each would need a corresponding `core:Property`
//!   declaration with its own data_type; the synonym path above is
//!   the proof-of-concept for the pattern.
//! - `equivalentNodesSets`, `logicalDefinitionAxioms`,
//!   `domainRangeAxioms`, `propertyChainAxioms` are dropped.
//!   They're OWL-shaped axioms that don't have a 1:1 Eigon
//!   counterpart and need a separate design pass.
//! - The `meta` block on edges is dropped — OBO uses it to carry
//!   provenance and evidence codes, which D43 doesn't ingest yet.

use std::collections::{BTreeMap, BTreeSet};

use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};

use crate::obo::{Edge, GraphDocument, Node};

// ─── well-known Eigon IRIs ─────────────────────────────────────────────
//
// Centralised here so the converter stays a pure function of the
// input document; the kernel's `well_known` module exposes the same
// strings, but we hold our own copies so the crate's dependency on
// the kernel stays scoped to the public Resource/IRI/Value types.

const IS_A: &str = "urn:eigenius:core:is_a";
const SHORT_NAME: &str = "urn:eigenius:core:short_name";
const DESCRIPTION: &str = "urn:eigenius:core:description";
const DATA_TYPE: &str = "urn:eigenius:core:data_type";
const DEPRECATED: &str = "urn:eigenius:core:deprecated";
const SOURCE_IRL: &str = "urn:eigenius:core:source_irl";

const CLASS: &str = "urn:eigenius:core:Class";
const PROPERTY: &str = "urn:eigenius:core:Property";
const RESOURCE: &str = "urn:eigenius:core:Resource";
const STRING_DATA_TYPE: &str = "urn:eigenius:core:string";
const RESOURCE_DATA_TYPE: &str = "urn:eigenius:core:resource";

const DECLARED_RESOURCE: &str = "urn:eigenius:reflection:DeclaredResource";
const DECLARED_BY: &str = "urn:eigenius:reflection:declared_by";

/// Declared-by attribution for converter-synthesised Property
/// declarations. Distinct from the source-graph attribution used
/// for nodes/edges that came from the input ontology, so downstream
/// auditors can tell "this Property was inferred by the importer"
/// apart from "this Property was declared by the source curators."
const CONVERTER_DECLARED_BY: &str = "urn:obo:converter:eigenius-obograph";

/// HTTP IRI rewriting prefixes — paired (input, output).
const OBO_HTTP_PREFIX: &str = "http://purl.obolibrary.org/obo/";
const OBO_IN_OWL_PREFIX: &str = "http://www.geneontology.org/formats/oboInOwl#";
const RDFS_PREFIX: &str = "http://www.w3.org/2000/01/rdf-schema#";
const OWL_PREFIX: &str = "http://www.w3.org/2002/07/owl#";

/// Rewrite an OBO-style HTTP IRI into a stable Eigenius URN. Returns
/// `(rewritten, Some(original))` when a rewrite occurred and the
/// original needs to be preserved as `source_irl`; returns
/// `(unchanged, None)` for IRIs that are already URN, for unknown
/// HTTP namespaces, or for malformed inputs the converter can't
/// confidently rewrite.
///
/// `http://purl.obolibrary.org/obo/<PREFIX>_<LOCAL>` is the dominant
/// shape (`GO_0005634`, `CHEBI_15422`, `BFO_0000050`); it rewrites
/// to `urn:obo:<PREFIX>:<LOCAL>`. The `#`-fragment form
/// (`http://purl.obolibrary.org/obo/<PREFIX>#<frag>`) covers
/// intra-ontology subset declarations and synonym-type Resources;
/// it rewrites to `urn:obo:<PREFIX>:<frag>`. Hash takes precedence
/// over underscore when both appear (some `#`-fragment IRIs carry
/// underscores in their fragment that aren't separators).
pub fn rewrite_iri(input: &str) -> (String, Option<String>) {
    if !(input.starts_with("http://") || input.starts_with("https://")) {
        return (input.to_string(), None);
    }
    if let Some(rest) = input.strip_prefix(OBO_HTTP_PREFIX) {
        // Hash form has precedence: a fragment after `#` is the
        // separator regardless of what comes before it.
        if let Some(hash_idx) = rest.find('#') {
            let prefix = &rest[..hash_idx];
            let frag = &rest[hash_idx + 1..];
            if !prefix.is_empty() && !frag.is_empty() {
                return (format!("urn:obo:{prefix}:{frag}"), Some(input.to_string()));
            }
        }
        // Underscore form — the dominant `<PREFIX>_<LOCAL>` shape.
        if let Some(under_idx) = rest.find('_') {
            let prefix = &rest[..under_idx];
            let local = &rest[under_idx + 1..];
            if !prefix.is_empty() && !local.is_empty() {
                return (format!("urn:obo:{prefix}:{local}"), Some(input.to_string()));
            }
        }
        // Bare obo/<X> with no separator — uncommon but documented.
        return (format!("urn:obo:misc:{rest}"), Some(input.to_string()));
    }
    if let Some(rest) = input.strip_prefix(OBO_IN_OWL_PREFIX) {
        return (format!("urn:obo:oboInOwl:{rest}"), Some(input.to_string()));
    }
    if let Some(rest) = input.strip_prefix(RDFS_PREFIX) {
        return (format!("urn:rdfs:{rest}"), Some(input.to_string()));
    }
    if let Some(rest) = input.strip_prefix(OWL_PREFIX) {
        return (format!("urn:owl:{rest}"), Some(input.to_string()));
    }
    (input.to_string(), None)
}

/// OBO's bare-string `is_a` predicate; everywhere else the predicate
/// is a full IRI of a Property node.
const OBO_IS_A_PREDICATE: &str = "is_a";

/// Prefix every synthetic-IRI predicate shares. Used by the
/// post-pass to detect "this slot uses a synthetic predicate, so
/// the ontology won't have a declaration for it — synthesise one."
const SYNTHETIC_PREFIX: &str = "urn:obo:";

// Synonym scope predicates as they appear in OBO-JSON's
// `meta.synonyms[*].pred` slot, paired with the Eigon-side synthetic
// IRI each maps to. Restated here as fall-through `match` arms in
// [`synonym_scope_to_iri`] rather than a table so the lookup stays
// inlineable and zero-allocation.

const SYN_EXACT: &str = "urn:obo:has_exact_synonym";
const SYN_RELATED: &str = "urn:obo:has_related_synonym";
const SYN_BROAD: &str = "urn:obo:has_broad_synonym";
const SYN_NARROW: &str = "urn:obo:has_narrow_synonym";
const OBO_INVERSE_OF: &str = "urn:obo:inverseOf";

/// IRIs declared by the shared `ontologies/obo/obo-meta-ontology.json`
/// layer loaded by the kernel at bootstrap. The post-pass skips
/// synthesising declarations for these because the meta layer
/// already covers them — re-emitting them per-imported-document
/// would shadow the authoritative declarations with redundant
/// copies. Adding a new declaration to the meta ontology requires
/// adding its IRI here too (or rather, the omission silently lets
/// the per-doc shadow win, which masks the meta version).
const META_DECLARED_IRIS: &[&str] = &[
    SYN_EXACT,
    SYN_RELATED,
    SYN_BROAD,
    SYN_NARROW,
    OBO_INVERSE_OF,
];

/// Map one OBO synonym `pred` slot to the Eigon-side synthetic IRI
/// the converter stores its values under. Returns `None` for
/// unrecognised scopes — these are rare; OBO ontologies in practice
/// use only the four canonical scopes.
fn synonym_scope_to_iri(pred: &str) -> Option<&'static str> {
    match pred {
        "hasExactSynonym" => Some(SYN_EXACT),
        "hasRelatedSynonym" => Some(SYN_RELATED),
        "hasBroadSynonym" => Some(SYN_BROAD),
        "hasNarrowSynonym" => Some(SYN_NARROW),
        _ => None,
    }
}

/// Eigon-side `data_type` for the synthesised Property declaration
/// behind a `urn:obo:*` predicate. Synonym scopes carry strings;
/// every other synthetic predicate (typically `urn:obo:inverseOf`
/// and friends from RBox edges) carries IRIs and so gets
/// `core:resource`. Default-resource is the right call for unknown
/// synthetic predicates: they came from edges (the only other path
/// that emits synthetic IRIs), and edges always carry IRI objects.
fn synthetic_predicate_data_type(iri: &str) -> &'static str {
    match iri {
        SYN_EXACT | SYN_RELATED | SYN_BROAD | SYN_NARROW => STRING_DATA_TYPE,
        _ => RESOURCE_DATA_TYPE,
    }
}

/// Per the OBO Graphs OWL mapping (`README-owlmapping.md`), a handful
/// of predicates appear as bare identifiers rather than full IRIs:
/// `is_a`, `subPropertyOf`, `inverseOf`, `type`. The first three are
/// RBox-shaped (class / property hierarchy); the last is RDF type.
///
/// `is_a` and `type` both fold into [`IS_A`] on the Eigon side
/// (Eigon doesn't distinguish class-membership from class-subclass at
/// this layer). `subPropertyOf` likewise folds into [`IS_A`] because
/// Eigon Properties *are* Resources and inherit their hierarchy via
/// the same `is_a` slot. `inverseOf` and any other bare-string
/// predicate get a synthetic `urn:obo:<pred>` IRI so the triple
/// survives the import without colliding with the kernel's reserved
/// `urn:eigenius:*` namespace.
fn resolve_predicate_iri(pred: &str) -> Result<Iri, ()> {
    if pred == OBO_IS_A_PREDICATE || pred == "type" || pred == "subPropertyOf" {
        return Ok(Iri::parse(IS_A).expect("well-known IRI"));
    }
    // HTTP IRIs in predicates flow through the same rewriter as
    // node IDs so cross-references stay coherent: an edge whose
    // pred is `http://purl.obolibrary.org/obo/BFO_0000050` rewrites
    // to `urn:obo:BFO:0000050`, which matches the URN the BFO
    // Property declaration was rewritten to.
    let (rewritten, _source_irl) = rewrite_iri(pred);
    if let Ok(iri) = Iri::parse(&rewritten) {
        return Ok(iri);
    }
    let synthetic = format!("urn:obo:{pred}");
    Iri::parse(&synthetic).map_err(|_| ())
}

/// Errors surfaced by the converter. Almost every node-level failure
/// (malformed IRI, missing required slot) skips the offending node
/// and the converter continues — life-science ontologies invariably
/// carry a long tail of malformed entries that shouldn't sink the
/// whole import. These typed errors are returned alongside the
/// successful Resources via [`ConvertReport`].
#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    #[error("invalid IRI in node `{context}`: {iri}")]
    InvalidIri { context: String, iri: String },
}

/// Conversion outcome: the Resources produced plus a list of
/// soft errors that the caller can surface but that did not abort
/// the import.
#[derive(Debug, Default)]
pub struct ConvertReport {
    pub resources: Vec<Resource>,
    pub errors: Vec<ConvertError>,
    /// Per-`(node type)` count of resources emitted, for quick
    /// post-import sanity checks ("did we actually get 45k GO
    /// classes?").
    pub counts_by_type: BTreeMap<String, usize>,
}

/// Knobs the caller can tune without affecting the structural
/// mapping. All optional — `Default` gives the conservative
/// behaviour the v1 integration tests expect.
#[derive(Debug, Clone, Default)]
pub struct ConvertOptions {
    /// Override the `declared_by` attribution attached to every
    /// imported Resource. When `None`, defaults per-graph to the
    /// graph's own IRI (`graph.id`); if the graph also lacks an ID,
    /// the converter falls back to
    /// `urn:obo:converter:unknown-graph`. Set this when ingesting a
    /// dump whose graph-IRI doesn't unambiguously identify the
    /// curating authority (e.g., a community subset of GO).
    pub declared_by: Option<String>,
}

/// Convert a full [`GraphDocument`] into a flat list of Eigon
/// [`Resource`]s using the default conversion options. Multiple OBO
/// graphs in one document are flattened into one Eigon document —
/// the OBO graph identity is dropped on the Eigon side (graphs are
/// an OWL provenance construct; Eigon layers serve that role), but
/// per-graph attribution survives via `declared_by`.
pub fn convert_document(doc: &GraphDocument) -> ConvertReport {
    convert_document_with(doc, &ConvertOptions::default())
}

/// Conversion entry point with explicit options. See
/// [`ConvertOptions`] for the knobs.
pub fn convert_document_with(doc: &GraphDocument, opts: &ConvertOptions) -> ConvertReport {
    let mut report = ConvertReport::default();
    let mut by_iri: BTreeMap<String, Resource> = BTreeMap::new();

    for graph in &doc.graphs {
        // Per-graph declared_by, resolving in priority order: caller
        // override, the graph's own IRI, the converter fallback. The
        // fallback covers OBO dumps that omit graph.id (rare but the
        // schema allows it).
        let declared_by_value = opts
            .declared_by
            .clone()
            .or_else(|| graph.id.clone())
            .unwrap_or_else(|| "urn:obo:converter:unknown-graph".to_string());

        for node in &graph.nodes {
            let (urn_str, source_irl) = rewrite_iri(&node.id);
            let iri = match Iri::parse(&urn_str) {
                Ok(i) => i,
                Err(_) => {
                    report.errors.push(ConvertError::InvalidIri {
                        context: format!("node `{}`", node.lbl.as_deref().unwrap_or("<no-lbl>")),
                        iri: node.id.clone(),
                    });
                    continue;
                }
            };
            let resource = node_to_resource(node, iri, source_irl.as_deref(), &declared_by_value);
            let type_key = node.node_type.as_deref().unwrap_or("<untyped>").to_string();
            *report.counts_by_type.entry(type_key).or_insert(0) += 1;
            by_iri.insert(urn_str, resource);
        }

        for edge in &graph.edges {
            apply_edge(edge, &mut by_iri, &mut report, &declared_by_value);
        }
    }

    let synthetic_count = ensure_synthetic_property_declarations(&mut by_iri);
    if synthetic_count > 0 {
        *report
            .counts_by_type
            .entry("<synthetic-PROPERTY>".to_string())
            .or_insert(0) += synthetic_count;
    }

    report.resources = by_iri.into_values().collect();
    report
}

/// Walk the accumulated Resources, find every `urn:obo:*` slot
/// referenced anywhere, and synthesise a Property declaration for
/// each one that doesn't already have one. Returns the number of
/// declarations emitted so the report can surface it.
///
/// The synthesised Property carries:
///
/// - `is_a: [core:Property, urn:eigenius:reflection:DeclaredResource]`
///   — Property + Declared, since the converter itself is the
///   "declarer" for synthesised IRIs.
/// - `data_type` per [`synthetic_predicate_data_type`] —
///   `core:string` for synonym scopes, `core:resource` for
///   everything else.
/// - `short_name` derived from the trailing IRI fragment after
///   `urn:obo:` so kernel-side short-name resolution surfaces the
///   declaration without forcing every caller to know the full IRI.
/// - `declared_by: <CONVERTER_DECLARED_BY>` — attributes the
///   declaration to the importer rather than to the source graph
///   so auditors can tell converter-inferred Properties apart
///   from curator-declared ones.
fn ensure_synthetic_property_declarations(by_iri: &mut BTreeMap<String, Resource>) -> usize {
    let mut used: BTreeSet<String> = BTreeSet::new();
    for resource in by_iri.values() {
        for iri in resource.properties().keys() {
            let s = iri.as_str();
            if s.starts_with(SYNTHETIC_PREFIX) {
                used.insert(s.to_string());
            }
        }
    }

    let mut emitted = 0usize;
    for iri_str in used {
        if by_iri.contains_key(&iri_str) {
            // The ontology already declared this — unusual for the
            // `urn:obo:*` namespace, but possible if a future pass
            // hand-injects a Property under the same IRI; the
            // explicit declaration wins.
            continue;
        }
        if META_DECLARED_IRIS.contains(&iri_str.as_str()) {
            // Covered by the shared `obo-meta-ontology.json` layer
            // (loaded by the kernel ahead of any imported document
            // via `bootstrap`). Emitting an inline declaration here
            // would shadow the authoritative one, so skip.
            continue;
        }
        let iri = Iri::parse(&iri_str).expect("converter-synthesised IRI");
        let mut r = Resource::new(iri);
        r.set(
            Iri::parse(IS_A).expect("well-known IRI"),
            Value::Array(vec![
                Value::ResourceRef(Iri::parse(PROPERTY).expect("well-known IRI")),
                Value::ResourceRef(Iri::parse(DECLARED_RESOURCE).expect("well-known IRI")),
            ]),
        );
        r.set(
            Iri::parse(DECLARED_BY).expect("well-known IRI"),
            Value::String(CONVERTER_DECLARED_BY.to_string()),
        );
        let data_type = synthetic_predicate_data_type(&iri_str);
        r.set(
            Iri::parse(DATA_TYPE).expect("well-known IRI"),
            Value::ResourceRef(Iri::parse(data_type).expect("well-known IRI")),
        );
        if let Some(short) = iri_str.strip_prefix(SYNTHETIC_PREFIX) {
            r.set(
                Iri::parse(SHORT_NAME).expect("well-known IRI"),
                Value::String(short.to_string()),
            );
        }
        by_iri.insert(iri_str, r);
        emitted += 1;
    }
    emitted
}

/// Build a fresh [`Resource`] from one OBO node — the per-node
/// transform that ignores edges (those are applied in a second pass
/// so out-of-order definitions don't matter).
fn node_to_resource(
    node: &Node,
    iri: Iri,
    source_irl: Option<&str>,
    declared_by: &str,
) -> Resource {
    let mut r = Resource::new(iri);

    // is_a — driven by node type, then extended with
    // [`DECLARED_RESOURCE`] so every imported Resource is
    // structurally a declared one. PROPERTY nodes get a `data_type`
    // companion slot; INDIVIDUAL gets the catch-all `core:Resource`;
    // unknown/missing type still gets DeclaredResource (the only
    // tag).
    let is_a_iri = Iri::parse(IS_A).expect("well-known IRI");
    let (is_a_target, data_type_target): (Option<&str>, Option<&str>) =
        match node.node_type.as_deref() {
            Some("CLASS") => (Some(CLASS), None),
            Some("PROPERTY") => {
                let dt = match node.property_type.as_deref() {
                    Some("OBJECT") => RESOURCE_DATA_TYPE,
                    // ANNOTATION and DATA both carry string-typed values
                    // in OBO-JSON; the kernel's per-type checks downstream
                    // can specialise further when needed.
                    Some("ANNOTATION") | Some("DATA") | None => STRING_DATA_TYPE,
                    Some(_) => STRING_DATA_TYPE,
                };
                (Some(PROPERTY), Some(dt))
            }
            Some("INDIVIDUAL") => (Some(RESOURCE), None),
            _ => (None, None),
        };
    let mut is_a_array: Vec<Value> = Vec::new();
    if let Some(target) = is_a_target {
        is_a_array.push(Value::ResourceRef(
            Iri::parse(target).expect("well-known IRI"),
        ));
    }
    is_a_array.push(Value::ResourceRef(
        Iri::parse(DECLARED_RESOURCE).expect("well-known IRI"),
    ));
    r.set(is_a_iri, Value::Array(is_a_array));

    if let Some(target) = data_type_target {
        let dt_iri = Iri::parse(DATA_TYPE).expect("well-known IRI");
        let target_iri = Iri::parse(target).expect("well-known IRI");
        r.set(dt_iri, Value::ResourceRef(target_iri));
    }

    // Provenance + attribution. `source_irl` is only set when the
    // IRI was actually rewritten; for already-URN inputs the IRI is
    // its own provenance and the slot would be redundant.
    if let Some(src) = source_irl {
        r.set(
            Iri::parse(SOURCE_IRL).expect("well-known IRI"),
            Value::String(src.to_string()),
        );
    }
    r.set(
        Iri::parse(DECLARED_BY).expect("well-known IRI"),
        Value::String(declared_by.to_string()),
    );

    if let Some(lbl) = node.lbl.as_deref() {
        if !lbl.is_empty() {
            r.set(
                Iri::parse(SHORT_NAME).expect("well-known IRI"),
                Value::String(lbl.to_string()),
            );
        }
    }

    if let Some(meta) = node.meta.as_ref() {
        if let Some(def) = meta.definition.as_ref() {
            if let Some(val) = def.val.as_deref() {
                if !val.is_empty() {
                    r.set(
                        Iri::parse(DESCRIPTION).expect("well-known IRI"),
                        Value::String(val.to_string()),
                    );
                }
            }
        }
        if meta.deprecated {
            r.set(
                Iri::parse(DEPRECATED).expect("well-known IRI"),
                Value::Boolean(true),
            );
        }

        // Synonyms — group by scope predicate, accumulate values
        // into per-scope string arrays. Unrecognised scopes (rare in
        // practice) drop on the floor; entries with no `val` slot
        // (a partial entry the OBO emitter wrote and abandoned)
        // also drop.
        let mut by_scope: BTreeMap<&'static str, Vec<Value>> = BTreeMap::new();
        for syn in &meta.synonyms {
            let scope = match syn.pred.as_deref().and_then(synonym_scope_to_iri) {
                Some(s) => s,
                None => continue,
            };
            let val = match syn.val.as_deref() {
                Some(v) if !v.is_empty() => v,
                _ => continue,
            };
            by_scope
                .entry(scope)
                .or_default()
                .push(Value::String(val.to_string()));
        }
        for (scope_iri, vals) in by_scope {
            r.set(
                Iri::parse(scope_iri).expect("well-known IRI"),
                Value::Array(vals),
            );
        }
    }

    r
}

/// Fold one edge into the accumulating `by_iri` map. Two cases:
///
/// - `pred == "is_a"`: extend the subject's `is_a` array. If the
///   subject doesn't exist yet (an edge with a sub-IRI not in the
///   `nodes` list), a minimal typeless Resource is materialised so
///   the edge has a place to land.
/// - any other predicate: the predicate is interpreted as a full
///   IRI of a Property node. The subject's property at that IRI
///   gets the object IRI appended as a `Value::ResourceRef`.
///   Multi-edge support: repeated `(sub, pred)` pairs collapse to
///   an array of references.
fn apply_edge(
    edge: &Edge,
    by_iri: &mut BTreeMap<String, Resource>,
    report: &mut ConvertReport,
    declared_by: &str,
) {
    let (urn_sub, sub_source_irl) = rewrite_iri(&edge.sub);
    let (urn_obj, _obj_source_irl) = rewrite_iri(&edge.obj);

    let sub_iri = match Iri::parse(&urn_sub) {
        Ok(i) => i,
        Err(_) => {
            report.errors.push(ConvertError::InvalidIri {
                context: "edge.sub".to_string(),
                iri: edge.sub.clone(),
            });
            return;
        }
    };
    let obj_iri = match Iri::parse(&urn_obj) {
        Ok(i) => i,
        Err(_) => {
            report.errors.push(ConvertError::InvalidIri {
                context: "edge.obj".to_string(),
                iri: edge.obj.clone(),
            });
            return;
        }
    };

    let subject_entry = by_iri.entry(urn_sub).or_insert_with(|| {
        // Edge subject not declared as its own node — materialise a
        // typeless DeclaredResource so the edge has a place to land
        // and downstream auditors can still trace it back to the
        // source graph. The `is_a: [DeclaredResource]` seeding here
        // is consistent with [`node_to_resource`]: every Resource
        // the converter emits is structurally a declared one, even
        // the ones inferred from edges alone.
        let mut r = Resource::new(sub_iri);
        r.set(
            Iri::parse(IS_A).expect("well-known IRI"),
            Value::Array(vec![Value::ResourceRef(
                Iri::parse(DECLARED_RESOURCE).expect("well-known IRI"),
            )]),
        );
        if let Some(src) = sub_source_irl {
            r.set(
                Iri::parse(SOURCE_IRL).expect("well-known IRI"),
                Value::String(src),
            );
        }
        r.set(
            Iri::parse(DECLARED_BY).expect("well-known IRI"),
            Value::String(declared_by.to_string()),
        );
        r
    });

    let prop_iri = match resolve_predicate_iri(&edge.pred) {
        Ok(i) => i,
        Err(()) => {
            report.errors.push(ConvertError::InvalidIri {
                context: "edge.pred".to_string(),
                iri: edge.pred.clone(),
            });
            return;
        }
    };

    // Accumulate: every property is a resource_array. Newly-set
    // properties start as a one-element array; existing arrays gain
    // a fresh entry. A non-array existing value (which shouldn't
    // happen for is_a / object-property edges in valid input) is
    // upgraded to an array preserving the prior value.
    let existing = subject_entry.get(&prop_iri).cloned();
    let new_value = match existing {
        Some(Value::Array(mut arr)) => {
            arr.push(Value::ResourceRef(obj_iri));
            Value::Array(arr)
        }
        Some(other) => Value::Array(vec![other, Value::ResourceRef(obj_iri)]),
        None => Value::Array(vec![Value::ResourceRef(obj_iri)]),
    };
    subject_entry.set(prop_iri, new_value);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal hand-rolled graph exercising every branch of the
    /// node→Resource and edge→Resource mappings. Keeps the test
    /// hermetic without dragging in the full nucleus.json fixture
    /// (which is exercised by the integration tests in [`lib.rs`]).
    fn sample_doc() -> GraphDocument {
        serde_json::from_str(
            r#"{
            "graphs": [{
                "id": "http://example.org/g1",
                "nodes": [
                    {
                        "id": "http://example.org/Cell",
                        "lbl": "cell",
                        "type": "CLASS",
                        "meta": {"definition": {"val": "the basic structural unit"}}
                    },
                    {
                        "id": "http://example.org/part_of",
                        "lbl": "part of",
                        "type": "PROPERTY",
                        "propertyType": "OBJECT"
                    },
                    {
                        "id": "http://example.org/has_label",
                        "lbl": "has label",
                        "type": "PROPERTY",
                        "propertyType": "ANNOTATION"
                    },
                    {
                        "id": "http://example.org/Organelle",
                        "lbl": "organelle",
                        "type": "CLASS"
                    },
                    {
                        "id": "http://example.org/Mitochondrion",
                        "lbl": "mitochondrion",
                        "type": "CLASS",
                        "meta": {"deprecated": false}
                    }
                ],
                "edges": [
                    {"sub": "http://example.org/Mitochondrion", "pred": "is_a", "obj": "http://example.org/Organelle"},
                    {"sub": "http://example.org/Mitochondrion", "pred": "http://example.org/part_of", "obj": "http://example.org/Cell"}
                ]
            }]
        }"#,
        )
        .expect("hand-rolled doc parses")
    }

    fn find(report: &ConvertReport, iri: &str) -> Resource {
        report
            .resources
            .iter()
            .find(|r| r.id().map(|i| i.as_str() == iri).unwrap_or(false))
            .cloned()
            .unwrap_or_else(|| panic!("no resource with id `{iri}`"))
    }

    #[test]
    fn class_node_maps_to_is_a_class_with_short_name_and_description() {
        let report = convert_document(&sample_doc());
        let cell = find(&report, "http://example.org/Cell");
        // `is_a` now carries the node typing AND the
        // DeclaredResource tag — every imported Resource is
        // structurally a declared one. The typing target comes
        // first; DeclaredResource appended last.
        let is_a = cell
            .get(&Iri::parse(IS_A).unwrap())
            .expect("is_a present")
            .clone();
        let iris: Vec<String> = match is_a {
            Value::Array(arr) => arr
                .iter()
                .filter_map(|v| match v {
                    Value::ResourceRef(i) => Some(i.as_str().to_string()),
                    _ => None,
                })
                .collect(),
            other => panic!("expected Array, got {other:?}"),
        };
        assert_eq!(iris, vec![CLASS.to_string(), DECLARED_RESOURCE.to_string()]);
        match cell.get(&Iri::parse(SHORT_NAME).unwrap()) {
            Some(Value::String(s)) => assert_eq!(s, "cell"),
            other => panic!("expected short_name string, got {other:?}"),
        }
        match cell.get(&Iri::parse(DESCRIPTION).unwrap()) {
            Some(Value::String(s)) => assert_eq!(s, "the basic structural unit"),
            other => panic!("expected description string, got {other:?}"),
        }
        // `declared_by` defaults to the sample doc's graph IRI.
        match cell.get(&Iri::parse(DECLARED_BY).unwrap()) {
            Some(Value::String(s)) => assert_eq!(s, "http://example.org/g1"),
            other => panic!("expected declared_by string, got {other:?}"),
        }
    }

    #[test]
    fn property_node_object_type_gets_resource_data_type() {
        let report = convert_document(&sample_doc());
        let part_of = find(&report, "http://example.org/part_of");
        match part_of.get(&Iri::parse(DATA_TYPE).unwrap()) {
            Some(Value::ResourceRef(i)) => assert_eq!(i.as_str(), RESOURCE_DATA_TYPE),
            other => panic!("expected data_type ResourceRef, got {other:?}"),
        }
    }

    #[test]
    fn property_node_annotation_type_gets_string_data_type() {
        let report = convert_document(&sample_doc());
        let has_label = find(&report, "http://example.org/has_label");
        match has_label.get(&Iri::parse(DATA_TYPE).unwrap()) {
            Some(Value::ResourceRef(i)) => assert_eq!(i.as_str(), STRING_DATA_TYPE),
            other => panic!("expected data_type string, got {other:?}"),
        }
    }

    /// `pred == "is_a"` extends the subject's `is_a` array — and
    /// the subject's existing `[core:Class, DeclaredResource]` (from
    /// the node typing + DeclaredResource tag) must survive the
    /// merge. Mitochondrion ends up
    /// `is_a: [core:Class, DeclaredResource, Organelle]`.
    #[test]
    fn is_a_edge_extends_existing_is_a_array() {
        let report = convert_document(&sample_doc());
        let mito = find(&report, "http://example.org/Mitochondrion");
        let is_a = mito
            .get(&Iri::parse(IS_A).unwrap())
            .expect("is_a present")
            .clone();
        let array = match is_a {
            Value::Array(arr) => arr,
            other => panic!("expected Array, got {other:?}"),
        };
        let iris: Vec<String> = array
            .iter()
            .filter_map(|v| match v {
                Value::ResourceRef(i) => Some(i.as_str().to_string()),
                _ => None,
            })
            .collect();
        // Node typing seeded `[CLASS, DeclaredResource]`; the
        // `is_a` edge appends the Organelle URN.
        assert_eq!(
            iris,
            vec![
                CLASS.to_string(),
                DECLARED_RESOURCE.to_string(),
                "http://example.org/Organelle".to_string(),
            ]
        );
    }

    /// Object-property edges (`part_of`) populate the subject's
    /// property at the predicate IRI as a resource_array.
    #[test]
    fn object_property_edge_populates_predicate_iri_slot() {
        let report = convert_document(&sample_doc());
        let mito = find(&report, "http://example.org/Mitochondrion");
        let part_of_value = mito
            .get(&Iri::parse("http://example.org/part_of").unwrap())
            .expect("part_of slot present")
            .clone();
        match part_of_value {
            Value::Array(arr) => {
                assert_eq!(arr.len(), 1);
                match &arr[0] {
                    Value::ResourceRef(i) => assert_eq!(i.as_str(), "http://example.org/Cell"),
                    other => panic!("expected ResourceRef, got {other:?}"),
                }
            }
            other => panic!("expected Array, got {other:?}"),
        }
    }

    #[test]
    fn report_counts_by_type_reflects_node_distribution() {
        let report = convert_document(&sample_doc());
        assert_eq!(report.counts_by_type.get("CLASS"), Some(&3));
        assert_eq!(report.counts_by_type.get("PROPERTY"), Some(&2));
    }

    /// Bare-string OBO predicates (`type`, `subPropertyOf`,
    /// `inverseOf`, beyond `is_a`) used to soft-error and drop the
    /// edge. They now fold into `is_a` (for the hierarchy shorthand)
    /// or synthesise a `urn:obo:<pred>` IRI (for `inverseOf` and any
    /// other bare-string predicate the converter doesn't have a
    /// well-known mapping for). Three edges, three distinct
    /// destinations, no soft errors.
    #[test]
    fn bare_string_predicates_route_to_well_known_or_synthetic_iris() {
        let doc: GraphDocument = serde_json::from_str(
            r#"{"graphs":[{
                "nodes": [
                    {"id": "http://example.org/p1", "type": "PROPERTY", "propertyType": "OBJECT"},
                    {"id": "http://example.org/p2", "type": "PROPERTY", "propertyType": "OBJECT"},
                    {"id": "http://example.org/C1", "type": "CLASS"},
                    {"id": "http://example.org/C2", "type": "CLASS"},
                    {"id": "http://example.org/i1", "type": "INDIVIDUAL"}
                ],
                "edges": [
                    {"sub": "http://example.org/p1", "pred": "subPropertyOf", "obj": "http://example.org/p2"},
                    {"sub": "http://example.org/p1", "pred": "inverseOf",     "obj": "http://example.org/p2"},
                    {"sub": "http://example.org/i1", "pred": "type",          "obj": "http://example.org/C1"}
                ]
            }]}"#,
        ).unwrap();
        let report = convert_document(&doc);
        assert!(
            report.errors.is_empty(),
            "no soft errors: {:?}",
            report.errors
        );

        let p1 = find(&report, "http://example.org/p1");
        // subPropertyOf folds into is_a → p1.is_a contains both
        // [core:Property] (from node typing) and p2 (from the edge).
        let is_a = p1.get(&Iri::parse(IS_A).unwrap()).unwrap();
        let is_a_iris: Vec<String> = match is_a {
            Value::Array(arr) => arr
                .iter()
                .filter_map(|v| match v {
                    Value::ResourceRef(i) => Some(i.as_str().to_string()),
                    _ => None,
                })
                .collect(),
            _ => panic!("expected is_a Array"),
        };
        assert!(is_a_iris.iter().any(|i| i == "http://example.org/p2"));
        assert!(is_a_iris.iter().any(|i| i == PROPERTY));

        // inverseOf lands on `urn:obo:inverseOf`.
        let inverse_iri = Iri::parse("urn:obo:inverseOf").unwrap();
        let inv = p1.get(&inverse_iri).expect("inverseOf slot present");
        match inv {
            Value::Array(arr) => {
                assert_eq!(arr.len(), 1);
                if let Value::ResourceRef(i) = &arr[0] {
                    assert_eq!(i.as_str(), "http://example.org/p2");
                } else {
                    panic!("expected ResourceRef");
                }
            }
            _ => panic!("expected Array"),
        }

        // type also folds into is_a (rdf:type ≈ class membership).
        let i1 = find(&report, "http://example.org/i1");
        let i1_is_a = i1.get(&Iri::parse(IS_A).unwrap()).unwrap();
        let i1_iris: Vec<String> = match i1_is_a {
            Value::Array(arr) => arr
                .iter()
                .filter_map(|v| match v {
                    Value::ResourceRef(i) => Some(i.as_str().to_string()),
                    _ => None,
                })
                .collect(),
            _ => panic!("expected is_a Array"),
        };
        assert!(i1_iris.iter().any(|i| i == "http://example.org/C1"));
        assert!(i1_iris.iter().any(|i| i == RESOURCE));
        assert!(i1_iris.iter().any(|i| i == DECLARED_RESOURCE));
    }

    /// Synonyms in `meta.synonyms` collapse onto the Resource as
    /// per-scope string arrays. Both `hasExactSynonym` and
    /// `hasRelatedSynonym` populate independently — repeated entries
    /// at the same scope accumulate; entries with empty `val` drop.
    #[test]
    fn synonyms_populate_per_scope_arrays_on_resource() {
        let doc: GraphDocument = serde_json::from_str(
            r#"{"graphs":[{"nodes":[{
                "id": "http://example.org/Nucleus",
                "type": "CLASS",
                "lbl": "nucleus",
                "meta": {
                    "synonyms": [
                        {"pred": "hasExactSynonym",   "val": "cell nucleus"},
                        {"pred": "hasExactSynonym",   "val": "karyon"},
                        {"pred": "hasRelatedSynonym", "val": "nucleated structure"},
                        {"pred": "hasExactSynonym",   "val": ""},
                        {"pred": "ignoredScope",      "val": "should drop"}
                    ]
                }
            }]}]}"#,
        )
        .unwrap();
        let report = convert_document(&doc);
        let n = find(&report, "http://example.org/Nucleus");

        // Exact synonyms: ["cell nucleus", "karyon"] (empty dropped).
        let exact_iri = Iri::parse(SYN_EXACT).unwrap();
        let exact = match n.get(&exact_iri).expect("exact synonym slot") {
            Value::Array(arr) => arr
                .iter()
                .filter_map(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            other => panic!("expected Array, got {other:?}"),
        };
        assert_eq!(exact, vec!["cell nucleus", "karyon"]);

        // Related synonym: ["nucleated structure"].
        let related_iri = Iri::parse(SYN_RELATED).unwrap();
        let related = match n.get(&related_iri).expect("related synonym slot") {
            Value::Array(arr) => arr
                .iter()
                .filter_map(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            other => panic!("expected Array, got {other:?}"),
        };
        assert_eq!(related, vec!["nucleated structure"]);

        // ignoredScope drops on the floor — no slot under any
        // synthetic IRI for it.
        let unknown_iri = Iri::parse("urn:obo:ignoredScope").unwrap();
        assert!(n.get(&unknown_iri).is_none());
    }

    /// Meta-declared `urn:obo:*` IRIs (the four synonym scopes plus
    /// `inverseOf`) are skipped by the per-document synthesiser
    /// because `ontologies/obo/obo-meta-ontology.json` (loaded by
    /// the kernel at bootstrap) already declares them. The
    /// converter must not emit shadow copies.
    #[test]
    fn synthetic_property_declarations_skip_meta_layer_iris() {
        let doc: GraphDocument = serde_json::from_str(
            r#"{"graphs":[{
                "nodes": [
                    {
                        "id": "http://example.org/N1",
                        "type": "CLASS",
                        "meta": {"synonyms": [{"pred": "hasExactSynonym", "val": "n1 alias"}]}
                    },
                    {"id": "http://example.org/P1", "type": "PROPERTY", "propertyType": "OBJECT"},
                    {"id": "http://example.org/P2", "type": "PROPERTY", "propertyType": "OBJECT"}
                ],
                "edges": [
                    {"sub": "http://example.org/P1", "pred": "inverseOf", "obj": "http://example.org/P2"}
                ]
            }]}"#,
        ).unwrap();
        let report = convert_document(&doc);
        assert!(
            report.errors.is_empty(),
            "no soft errors: {:?}",
            report.errors
        );

        // Neither has_exact_synonym nor inverseOf gets re-declared
        // — both are in META_DECLARED_IRIS.
        assert!(
            !report.counts_by_type.contains_key("<synthetic-PROPERTY>"),
            "meta IRIs must not be re-synthesised; counts: {:?}",
            report.counts_by_type
        );
        assert!(
            report
                .resources
                .iter()
                .all(|r| r.id().map(|i| i.as_str() != SYN_EXACT).unwrap_or(true)),
            "obo:has_exact_synonym must not be re-declared by the converter"
        );
        assert!(
            report
                .resources
                .iter()
                .all(|r| r.id().map(|i| i.as_str() != OBO_INVERSE_OF).unwrap_or(true)),
            "obo:inverseOf must not be re-declared by the converter"
        );
    }

    /// Ad-hoc `urn:obo:*` predicates that aren't covered by the
    /// meta layer still get a per-document declaration so the
    /// kernel validator can resolve them. Synthesises one
    /// declaration per distinct uncovered predicate, attributing it
    /// to the converter.
    #[test]
    fn synthetic_property_declarations_emitted_for_ad_hoc_urn_obo_slots() {
        let doc: GraphDocument = serde_json::from_str(
            r#"{"graphs":[{
                "nodes": [
                    {"id": "http://example.org/N1", "type": "CLASS"},
                    {"id": "http://example.org/N2", "type": "CLASS"}
                ],
                "edges": [
                    {"sub": "http://example.org/N1", "pred": "hasAlternativeNamespace", "obj": "http://example.org/N2"}
                ]
            }]}"#,
        )
        .unwrap();
        let report = convert_document(&doc);
        assert!(
            report.errors.is_empty(),
            "no soft errors: {:?}",
            report.errors
        );
        // `hasAlternativeNamespace` is not in META_DECLARED_IRIS;
        // converter must synthesise a declaration for it.
        assert_eq!(
            report.counts_by_type.get("<synthetic-PROPERTY>"),
            Some(&1),
            "counts_by_type: {:?}",
            report.counts_by_type
        );
        let decl = find(&report, "urn:obo:hasAlternativeNamespace");
        match decl.get(&Iri::parse(DATA_TYPE).unwrap()) {
            Some(Value::ResourceRef(i)) => assert_eq!(i.as_str(), RESOURCE_DATA_TYPE),
            other => panic!("expected data_type ResourceRef, got {other:?}"),
        }
        match decl.get(&Iri::parse(SHORT_NAME).unwrap()) {
            Some(Value::String(s)) => assert_eq!(s, "hasAlternativeNamespace"),
            other => panic!("expected short_name, got {other:?}"),
        }
    }

    // ─── IRI rewriting + declared knowledge tagging ────────────────────

    #[test]
    fn rewrite_iri_obo_underscore_form() {
        let (urn, src) = rewrite_iri("http://purl.obolibrary.org/obo/GO_0005634");
        assert_eq!(urn, "urn:obo:GO:0005634");
        assert_eq!(
            src.as_deref(),
            Some("http://purl.obolibrary.org/obo/GO_0005634")
        );
    }

    #[test]
    fn rewrite_iri_chebi_underscore_form() {
        // ChEBI follows the same OBO underscore shape.
        let (urn, _) = rewrite_iri("http://purl.obolibrary.org/obo/CHEBI_15422");
        assert_eq!(urn, "urn:obo:CHEBI:15422");
    }

    #[test]
    fn rewrite_iri_obo_hash_fragment_form() {
        // Subsets / synonym types in OBO use the `#` fragment form
        // (`go-test#systematic_synonym`). The fragment takes
        // precedence over an underscore in the prefix.
        let (urn, src) = rewrite_iri("http://purl.obolibrary.org/obo/go-test#systematic_synonym");
        assert_eq!(urn, "urn:obo:go-test:systematic_synonym");
        assert!(src.is_some());
    }

    #[test]
    fn rewrite_iri_obo_in_owl_namespace() {
        let (urn, _) = rewrite_iri("http://www.geneontology.org/formats/oboInOwl#hasOBONamespace");
        assert_eq!(urn, "urn:obo:oboInOwl:hasOBONamespace");
    }

    #[test]
    fn rewrite_iri_rdfs_and_owl_namespaces() {
        let (rdfs, _) = rewrite_iri("http://www.w3.org/2000/01/rdf-schema#label");
        assert_eq!(rdfs, "urn:rdfs:label");
        let (owl, _) = rewrite_iri("http://www.w3.org/2002/07/owl#Thing");
        assert_eq!(owl, "urn:owl:Thing");
    }

    #[test]
    fn rewrite_iri_passes_through_unknown_http_and_urn() {
        // Unknown HTTP namespaces — keep as-is, no source_irl since
        // the IRI is its own provenance.
        let (out, src) = rewrite_iri("http://example.org/custom/X");
        assert_eq!(out, "http://example.org/custom/X");
        assert!(src.is_none());
        // Already-URN IRIs — strict pass-through.
        let (out, src) = rewrite_iri("urn:eigenius:core:Class");
        assert_eq!(out, "urn:eigenius:core:Class");
        assert!(src.is_none());
    }

    /// A real OBO-style document — Class with HTTP IRI, is_a edge
    /// referring to another HTTP IRI — must round-trip through
    /// rewriting consistently: both `@id` and the `is_a` target use
    /// the URN form, and the Resource carries `source_irl` +
    /// `declared_by`.
    #[test]
    fn obo_style_doc_rewrites_consistently_across_node_and_edge() {
        let doc: GraphDocument = serde_json::from_str(
            r#"{"graphs":[{
                "id": "http://purl.obolibrary.org/obo/go.owl",
                "nodes": [
                    {"id": "http://purl.obolibrary.org/obo/GO_0005634", "type": "CLASS", "lbl": "nucleus"},
                    {"id": "http://purl.obolibrary.org/obo/GO_0043231", "type": "CLASS", "lbl": "intracellular membrane-bounded organelle"}
                ],
                "edges": [
                    {"sub": "http://purl.obolibrary.org/obo/GO_0005634", "pred": "is_a", "obj": "http://purl.obolibrary.org/obo/GO_0043231"}
                ]
            }]}"#,
        )
        .unwrap();
        let report = convert_document(&doc);

        // Both Resources are keyed under the URN form.
        let nucleus = find(&report, "urn:obo:GO:0005634");
        let _parent = find(&report, "urn:obo:GO:0043231");

        // source_irl preserves the HTTP form.
        match nucleus.get(&Iri::parse(SOURCE_IRL).unwrap()) {
            Some(Value::String(s)) => {
                assert_eq!(s, "http://purl.obolibrary.org/obo/GO_0005634");
            }
            other => panic!("expected source_irl String, got {other:?}"),
        }

        // is_a contains the URN form of the parent (not the HTTP
        // form) — cross-references rewrite consistently.
        let is_a = nucleus.get(&Iri::parse(IS_A).unwrap()).unwrap();
        let iris: Vec<String> = match is_a {
            Value::Array(arr) => arr
                .iter()
                .filter_map(|v| match v {
                    Value::ResourceRef(i) => Some(i.as_str().to_string()),
                    _ => None,
                })
                .collect(),
            _ => panic!("expected Array"),
        };
        assert!(iris.iter().any(|i| i == "urn:obo:GO:0043231"));

        // declared_by defaults to the source graph IRI.
        match nucleus.get(&Iri::parse(DECLARED_BY).unwrap()) {
            Some(Value::String(s)) => {
                assert_eq!(s, "http://purl.obolibrary.org/obo/go.owl");
            }
            other => panic!("expected declared_by String, got {other:?}"),
        }
    }

    /// `--declared-by` override on the CLI / `ConvertOptions` takes
    /// precedence over the source graph's own IRI. Lets ingesters
    /// re-attribute when the graph IRI doesn't unambiguously
    /// identify the curating authority.
    #[test]
    fn declared_by_override_supersedes_graph_iri() {
        let doc: GraphDocument = serde_json::from_str(
            r#"{"graphs":[{
                "id": "http://purl.obolibrary.org/obo/go.owl",
                "nodes": [{"id": "http://purl.obolibrary.org/obo/GO_0005634", "type": "CLASS"}]
            }]}"#,
        )
        .unwrap();
        let opts = ConvertOptions {
            declared_by: Some("urn:eigenius:agents:go-curators".to_string()),
        };
        let report = convert_document_with(&doc, &opts);
        let nucleus = find(&report, "urn:obo:GO:0005634");
        match nucleus.get(&Iri::parse(DECLARED_BY).unwrap()) {
            Some(Value::String(s)) => assert_eq!(s, "urn:eigenius:agents:go-curators"),
            other => panic!("expected override declared_by, got {other:?}"),
        }
    }

    /// Synthesised Property declarations attribute themselves to the
    /// converter, not to the source graph — they're inferred by the
    /// importer, not declared by the curators.
    #[test]
    fn synthesised_property_attributes_to_converter() {
        // Use an ad-hoc bare-string predicate (`adhocLink`) that
        // isn't in META_DECLARED_IRIS — so the converter synthesises
        // a per-document declaration. Meta-covered synonyms wouldn't
        // produce a synthesised Resource to inspect.
        let doc: GraphDocument = serde_json::from_str(
            r#"{"graphs":[{
                "id": "http://example.org/g1",
                "nodes": [
                    {"id": "http://example.org/A", "type": "CLASS"},
                    {"id": "http://example.org/B", "type": "CLASS"}
                ],
                "edges": [
                    {"sub": "http://example.org/A", "pred": "adhocLink", "obj": "http://example.org/B"}
                ]
            }]}"#,
        )
        .unwrap();
        let report = convert_document(&doc);
        let decl = find(&report, "urn:obo:adhocLink");
        match decl.get(&Iri::parse(DECLARED_BY).unwrap()) {
            Some(Value::String(s)) => {
                assert_eq!(s, CONVERTER_DECLARED_BY);
            }
            other => panic!("expected converter declared_by, got {other:?}"),
        }
        // is_a includes both Property and DeclaredResource.
        let is_a_iris: Vec<String> = match decl.get(&Iri::parse(IS_A).unwrap()).unwrap() {
            Value::Array(arr) => arr
                .iter()
                .filter_map(|v| match v {
                    Value::ResourceRef(i) => Some(i.as_str().to_string()),
                    _ => None,
                })
                .collect(),
            _ => panic!("expected Array"),
        };
        assert!(is_a_iris.iter().any(|i| i == PROPERTY));
        assert!(is_a_iris.iter().any(|i| i == DECLARED_RESOURCE));
    }

    /// A `urn:obo:*` slot used in the data AND already explicitly
    /// declared as a Property node by the ontology must not be
    /// duplicated. The explicit declaration wins.
    #[test]
    fn explicit_property_declaration_wins_over_synthesised() {
        let doc: GraphDocument = serde_json::from_str(
            r#"{"graphs":[{
                "nodes": [
                    {"id": "urn:obo:custom_pred", "type": "PROPERTY", "propertyType": "OBJECT",
                     "lbl": "custom pred", "meta": {"definition": {"val": "user override"}}},
                    {"id": "http://example.org/A", "type": "CLASS"},
                    {"id": "http://example.org/B", "type": "CLASS"}
                ],
                "edges": [
                    {"sub": "http://example.org/A", "pred": "urn:obo:custom_pred", "obj": "http://example.org/B"}
                ]
            }]}"#,
        ).unwrap();
        let report = convert_document(&doc);
        // No <synthetic-PROPERTY> bucket because the ontology
        // pre-declared the predicate.
        assert!(
            !report.counts_by_type.contains_key("<synthetic-PROPERTY>"),
            "explicit declaration should suppress synthesis; counts: {:?}",
            report.counts_by_type
        );
        // The explicit Resource's properties survive intact.
        let r = find(&report, "urn:obo:custom_pred");
        match r.get(&Iri::parse(DESCRIPTION).unwrap()) {
            Some(Value::String(s)) => assert_eq!(s, "user override"),
            other => panic!("expected explicit description, got {other:?}"),
        }
    }
}
