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

//! schema.org JSON-LD → Eigon-JSON, implementing the D57 meta-ontology
//! correspondence (docs/notes/d57-schemaorg-vs-core-metamodel.md):
//!
//! - Class (`rdfs:Class`) → `core:Class` + `core:subclass_of`.
//! - DataType (subclass of `DataType`) → a core scalar + a validating
//!   `core:format` for `URL`/`Date`/`DateTime`/`Time`; recorded, not emitted.
//! - Enumeration (subclass of `Enumeration`) → `core:Class`; its member instances
//!   → `DeclaredResource`s; a property ranging over it → `class_types=[E]` +
//!   `allows_only=[members]` (the closed set).
//! - Property (`rdf:Property`) → `core:Property`; `rangeIncludes` per §3.3 (class
//!   members → `class_types`; single DataType → scalar + format; all-Class union →
//!   `class_types`; mixed → entity-first). `domainIncludes` is *advisory*, so it is
//!   inverted into each domain class's `core:recommends` (not `core:domain`, which
//!   would restrict — schema.org doesn't); subclasses inherit.
//! - `subPropertyOf` / `inverseOf` / `supersededBy` / `equivalentClass|Property`
//!   and the Role pattern → not mapped (recorded as Tier-3 residual; no reasoner).
//!
//! Scope: `schema:`-prefixed terms only; `pending`/`meta` layers excluded (hosted
//! extensions kept). Every emitted resource is a `DeclaredResource` carrying
//! `source_irl = https://schema.org/<Term>` and `declared_by = urn:schema_org`.

use std::collections::{BTreeMap, BTreeSet};

use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};
use serde_json::Value as Json;

use crate::jsonld::{iri_refs, node_id, node_str, node_types};

// ── schema.org CURIEs / type markers ────────────────────────────────
const SCHEMA_DATATYPE: &str = "schema:DataType";
const SCHEMA_ENUMERATION: &str = "schema:Enumeration";
const T_CLASS: &str = "rdfs:Class";
const T_PROPERTY: &str = "rdf:Property";
const K_SUBCLASS_OF: &str = "rdfs:subClassOf";
const K_LABEL: &str = "rdfs:label";
const K_COMMENT: &str = "rdfs:comment";
const K_DOMAIN_INCLUDES: &str = "schema:domainIncludes";
const K_RANGE_INCLUDES: &str = "schema:rangeIncludes";
const K_IS_PART_OF: &str = "schema:isPartOf";
// Tier-3 relational keys (recorded, never mapped to active relations).
const TIER3_KEYS: &[&str] = &[
    "schema:supersededBy",
    "schema:inverseOf",
    "rdfs:subPropertyOf",
    "owl:equivalentClass",
    "owl:equivalentProperty",
];
// Excluded layers (pending = unstable; meta = schema.org's own metamodel).
const EXCLUDED_PARTS: &[&str] = &["https://pending.schema.org", "https://meta.schema.org"];

// ── core IRIs ───────────────────────────────────────────────────────
const IS_A: &str = "urn:eigenius:core:is_a";
const SUBCLASS_OF: &str = "urn:eigenius:core:subclass_of";
const SHORT_NAME: &str = "urn:eigenius:core:short_name";
const DESCRIPTION: &str = "urn:eigenius:core:description";
const DATA_TYPE: &str = "urn:eigenius:core:data_type";
const CLASS_TYPES: &str = "urn:eigenius:core:class_types";
const ALLOWS_ONLY: &str = "urn:eigenius:core:allows_only";
const RECOMMENDS: &str = "urn:eigenius:core:recommends";
const FORMAT: &str = "urn:eigenius:core:format";
const SOURCE_IRL: &str = "urn:eigenius:core:source_irl";
const CORE_CLASS: &str = "urn:eigenius:core:Class";
const CORE_PROPERTY: &str = "urn:eigenius:core:Property";
const DECLARED_RESOURCE: &str = "urn:eigenius:reflection:DeclaredResource";
const DECLARED_BY: &str = "urn:eigenius:reflection:declared_by";
const DECLARED_BY_VALUE: &str = "urn:schema_org";

// core scalars + formats
const D_STRING: &str = "urn:eigenius:core:string";
const D_INTEGER: &str = "urn:eigenius:core:integer";
const D_FLOAT: &str = "urn:eigenius:core:float";
const D_BOOLEAN: &str = "urn:eigenius:core:boolean";
const D_RESOURCE: &str = "urn:eigenius:core:resource";
const F_IRI: &str = "urn:eigenius:core:formats:iri";
const F_DATE: &str = "urn:eigenius:core:formats:date";
const F_DATETIME: &str = "urn:eigenius:core:formats:datetime";
const F_TIME: &str = "urn:eigenius:core:formats:time";

/// Per-property mapping outcome, for the coverage report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RangeTier {
    /// single DataType → scalar, or single/all-Class → class_types.
    Clean,
    /// mixed literal+entity (entity-first), or all-DataType union → string.
    ByConvention,
    /// enumeration range → class_types + allows_only (closed set).
    Enumeration,
    /// no usable schema: range → defaulted to string.
    Defaulted,
}

/// What the import produced.
#[derive(Debug, Default, serde::Serialize)]
pub struct Coverage {
    pub classes: usize,
    pub enumeration_classes: usize,
    pub enumeration_members: usize,
    pub properties: usize,
    pub property_tiers: BTreeMap<String, usize>,
    /// Enumeration-tier properties whose enumeration class has no members
    /// anywhere in its subtree (a genuinely empty enumeration, e.g.
    /// `BusinessFunction` — members are defined in an external vocabulary). The
    /// `class_types` typing still applies, but no closed set (`allows_only`) can
    /// be formed, so the range stays open. Accounted, not silently dropped:
    /// `Enumeration` tier = (properties carrying `allows_only`) + `enumeration_open`.
    pub enumeration_open: usize,
    /// `property -> EmptyEnum` examples of the open enumerations above.
    pub enumeration_open_examples: Vec<String>,
    /// schema.org DataTypes folded into core scalars (not emitted as classes).
    pub datatypes_folded: Vec<String>,
    /// Excluded by layer (pending/meta).
    pub excluded_layer: usize,
    /// Tier-3 relational facts seen and intentionally NOT mapped (the cut).
    pub residual_relations: BTreeMap<String, usize>,
    pub residual_examples: Vec<String>,
}

#[derive(Debug, Default)]
pub struct ConvertReport {
    pub resources: Vec<Resource>,
    pub coverage: Coverage,
}

/// Map a `schema:Local` CURIE to `(urn:schema_org:Local, https://schema.org/Local)`.
/// Returns `None` for any non-`schema:` CURIE (external cross-references).
fn map_schema_id(curie: &str) -> Option<(String, String)> {
    let local = curie.strip_prefix("schema:")?;
    Some((
        format!("urn:schema_org:{local}"),
        format!("https://schema.org/{local}"),
    ))
}

fn rref(iri: &str) -> Value {
    Value::ResourceRef(Iri::parse(iri).expect("well-known IRI"))
}

/// Map a schema.org DataType CURIE to (core scalar, optional core:format).
fn scalar_of(curie: &str) -> (&'static str, Option<&'static str>) {
    match curie {
        "schema:Text" => (D_STRING, None),
        "schema:URL" => (D_STRING, Some(F_IRI)),
        "schema:Integer" => (D_INTEGER, None),
        "schema:Number" | "schema:Float" => (D_FLOAT, None),
        "schema:Boolean" => (D_BOOLEAN, None),
        "schema:Date" => (D_STRING, Some(F_DATE)),
        "schema:DateTime" => (D_STRING, Some(F_DATETIME)),
        "schema:Time" => (D_STRING, Some(F_TIME)),
        _ => (D_STRING, None),
    }
}

/// Compute the set of ids reachable downward (subclasses) from `root`,
/// inclusive, over the child→parent `subClassOf` edges (reversed here).
fn descendants_incl(root: &str, children: &BTreeMap<String, Vec<String>>) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![root.to_string()];
    while let Some(cur) = stack.pop() {
        if seen.insert(cur.clone()) {
            if let Some(kids) = children.get(&cur) {
                stack.extend(kids.iter().cloned());
            }
        }
    }
    seen
}

/// Is this node excluded by its `isPartOf` layer (pending / meta)?
fn excluded_by_layer(n: &Json) -> bool {
    iri_refs(n, K_IS_PART_OF)
        .iter()
        .any(|p| EXCLUDED_PARTS.contains(&p.as_str()))
}

/// Convert a parsed schema.org `@graph` to Eigon-JSON resources + coverage.
pub fn convert(nodes: &[Json]) -> ConvertReport {
    // ── Pass 1: index + subclass graph ──────────────────────────────
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut dual_datatypes: BTreeSet<String> = BTreeSet::new();
    for n in nodes {
        let Some(id) = node_id(n) else { continue };
        if !id.starts_with("schema:") {
            continue;
        }
        for parent in iri_refs(n, K_SUBCLASS_OF) {
            children.entry(parent).or_default().push(id.to_string());
        }
        if node_types(n).contains(&SCHEMA_DATATYPE) {
            dual_datatypes.insert(id.to_string());
        }
    }
    // DataTypes = DataType + every dual-typed (`@type schema:DataType`) term and
    // all their subtypes. Seeding from each dual datatype is load-bearing: most
    // datatypes (`Number`, `Text`, `Quantity`, …) carry `@type DataType` but no
    // `subClassOf DataType` edge, so their subtypes (`Integer`⊂`Number`,
    // `URL`⊂`Text`, `Distance`⊂`Quantity`) are only reachable from the dual node.
    let mut datatype_set = BTreeSet::new();
    for seed in dual_datatypes
        .iter()
        .cloned()
        .chain([SCHEMA_DATATYPE.to_string()])
    {
        datatype_set.extend(descendants_incl(&seed, &children));
    }
    let enum_set = descendants_incl(SCHEMA_ENUMERATION, &children);

    // kept_ids: the urns of every resource that WILL be emitted. Used to filter
    // outgoing references (subclass_of / class_types / domain / allows_only) so a
    // kept term never points at an out-of-scope target (a folded DataType, or a
    // pending/meta-excluded term) — which would be an unresolved reference.
    let mut kept_ids: BTreeSet<String> = BTreeSet::new();
    for n in nodes {
        let Some(id) = node_id(n) else { continue };
        if !id.starts_with("schema:") || excluded_by_layer(n) {
            continue;
        }
        let types = node_types(n);
        let emitted = if types.contains(&T_PROPERTY) {
            true
        } else if datatype_set.contains(id) {
            false
        } else if types.iter().any(|t| enum_set.contains(*t)) {
            true // enumeration member instance
        } else {
            types.contains(&T_CLASS)
        };
        if emitted {
            if let Some((u, _)) = map_schema_id(id) {
                kept_ids.insert(u);
            }
        }
    }

    // domainIncludes → recommends (inverted). schema.org's domainIncludes is
    // *advisory* ("this property is expected on these types") — that is
    // core:recommends (advisory), NOT core:domain (which *restricts* usage and
    // would over-enforce schema.org's permissive model). Invert to class →
    // {recommended properties}; emit on the direct domain class only (subclasses
    // inherit recommends from ancestors, mirroring schema.org's apply-to-subtypes).
    let mut class_recommends: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for n in nodes {
        let Some(id) = node_id(n) else { continue };
        if !id.starts_with("schema:")
            || excluded_by_layer(n)
            || !node_types(n).contains(&T_PROPERTY)
        {
            continue;
        }
        let Some((purn, _)) = map_schema_id(id) else {
            continue;
        };
        if !kept_ids.contains(&purn) {
            continue;
        }
        for d in iri_refs(n, K_DOMAIN_INCLUDES) {
            if let Some((curn, _)) = map_schema_id(&d) {
                if kept_ids.contains(&curn) {
                    class_recommends
                        .entry(curn)
                        .or_default()
                        .insert(purn.clone());
                }
            }
        }
    }

    // Enumeration members: a node whose @type is an enumeration class.
    // Build enum_class → [direct member urns].
    let mut enum_direct: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for n in nodes {
        let Some(id) = node_id(n) else { continue };
        if !id.starts_with("schema:") || excluded_by_layer(n) {
            continue;
        }
        for t in node_types(n) {
            if enum_set.contains(t) {
                if let (Some((murn, _)), Some((eurn, _))) = (map_schema_id(id), map_schema_id(t)) {
                    if kept_ids.contains(&murn) && kept_ids.contains(&eurn) {
                        enum_direct.entry(eurn).or_default().push(murn);
                    }
                }
            }
        }
    }
    // Closed set per enumeration class = the TRANSITIVE member closure: a member
    // of any sub-enumeration is a valid value for a property ranging on the
    // parent (schema.org's enumerations are a subclass hierarchy — members of
    // `QualitativeValue` live under its subtypes, members of `NonprofitType`
    // under its). Direct-only collection under-populated `allows_only` (it was
    // empty whenever the named enum's members lived in subclasses), so the closed
    // set leaked open. Walk each enum's descendants and union their direct members.
    let mut enum_members: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e_curie in &enum_set {
        let Some((eurn, _)) = map_schema_id(e_curie) else {
            continue;
        };
        if !kept_ids.contains(&eurn) {
            continue;
        }
        let mut members: BTreeSet<String> = BTreeSet::new();
        for d_curie in descendants_incl(e_curie, &children) {
            if let Some((durn, _)) = map_schema_id(&d_curie) {
                if let Some(ms) = enum_direct.get(&durn) {
                    members.extend(ms.iter().cloned());
                }
            }
        }
        if !members.is_empty() {
            enum_members.insert(eurn, members.into_iter().collect());
        }
    }

    // ── Pass 2: emit ────────────────────────────────────────────────
    let mut report = ConvertReport::default();
    for n in nodes {
        let Some(id) = node_id(n) else { continue };
        if !id.starts_with("schema:") {
            continue;
        }
        if excluded_by_layer(n) {
            report.coverage.excluded_layer += 1;
            continue;
        }
        record_residual(n, &mut report.coverage);

        let types = node_types(n);
        if types.contains(&T_PROPERTY) {
            emit_property(
                n,
                id,
                &datatype_set,
                &enum_set,
                &enum_members,
                &kept_ids,
                &mut report,
            );
        } else if datatype_set.contains(id) {
            // DataType: folded into a core scalar; not emitted as a class.
            report.coverage.datatypes_folded.push(id.to_string());
        } else if types.iter().any(|t| enum_set.contains(*t)) {
            // Enumeration member instance.
            emit_member(n, id, &types, &enum_set, &kept_ids, &mut report);
        } else if types.contains(&T_CLASS) {
            emit_class(n, id, &enum_set, &kept_ids, &class_recommends, &mut report);
        }
        // else: untyped / non-vocabulary node — skip.
    }
    report
}

fn common_meta(r: &mut Resource, n: &Json, https: &str) {
    if let Some(label) = node_str(n, K_LABEL) {
        r.set(iri(SHORT_NAME), Value::String(label.to_string()));
    }
    if let Some(comment) = node_str(n, K_COMMENT) {
        r.set(iri(DESCRIPTION), Value::String(comment.to_string()));
    }
    r.set(iri(SOURCE_IRL), Value::String(https.to_string()));
    r.set(
        iri(DECLARED_BY),
        Value::String(DECLARED_BY_VALUE.to_string()),
    );
}

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("well-known IRI")
}

fn emit_class(
    n: &Json,
    id: &str,
    enum_set: &BTreeSet<String>,
    kept: &BTreeSet<String>,
    class_recommends: &BTreeMap<String, BTreeSet<String>>,
    report: &mut ConvertReport,
) {
    let (urn, https) = map_schema_id(id).expect("schema: id");
    let mut r = Resource::new(iri(&urn));
    r.set(
        iri(IS_A),
        Value::Array(vec![rref(CORE_CLASS), rref(DECLARED_RESOURCE)]),
    );
    // subclass_of: in-scope schema: parents only (drop external cross-refs,
    // folded DataTypes, and out-of-scope/pending parents).
    let parents: Vec<Value> = iri_refs(n, K_SUBCLASS_OF)
        .iter()
        .filter_map(|p| map_schema_id(p).map(|(u, _)| u))
        .filter(|u| kept.contains(u))
        .map(|u| rref(&u))
        .collect();
    if !parents.is_empty() {
        r.set(iri(SUBCLASS_OF), Value::Array(parents));
    }
    // recommends: the properties whose schema.org domainIncludes names this class
    // (advisory; subclasses inherit). The faithful mapping of domainIncludes.
    if let Some(props) = class_recommends.get(&urn) {
        r.set(
            iri(RECOMMENDS),
            Value::Array(props.iter().map(|p| rref(p)).collect()),
        );
    }
    common_meta(&mut r, n, &https);
    report.resources.push(r);
    if enum_set.contains(id) {
        report.coverage.enumeration_classes += 1;
    } else {
        report.coverage.classes += 1;
    }
}

fn emit_member(
    n: &Json,
    id: &str,
    types: &[&str],
    enum_set: &BTreeSet<String>,
    kept: &BTreeSet<String>,
    report: &mut ConvertReport,
) {
    let (urn, https) = map_schema_id(id).expect("schema: id");
    let mut r = Resource::new(iri(&urn));
    // is_a = [<every in-scope enum class it instantiates>..., DeclaredResource]
    let mut is_a: Vec<Value> = types
        .iter()
        .filter(|t| enum_set.contains(**t))
        .filter_map(|t| map_schema_id(t).map(|(u, _)| u))
        .filter(|u| kept.contains(u))
        .map(|u| rref(&u))
        .collect();
    if is_a.is_empty() {
        return; // its enumeration class is out of scope — skip the member
    }
    is_a.push(rref(DECLARED_RESOURCE));
    r.set(iri(IS_A), Value::Array(is_a));
    common_meta(&mut r, n, &https);
    report.resources.push(r);
    report.coverage.enumeration_members += 1;
}

#[allow(clippy::too_many_arguments)]
fn emit_property(
    n: &Json,
    id: &str,
    datatype_set: &BTreeSet<String>,
    enum_set: &BTreeSet<String>,
    enum_members: &BTreeMap<String, Vec<String>>,
    kept: &BTreeSet<String>,
    report: &mut ConvertReport,
) {
    let (urn, https) = map_schema_id(id).expect("schema: id");
    let mut r = Resource::new(iri(&urn));
    r.set(
        iri(IS_A),
        Value::Array(vec![rref(CORE_PROPERTY), rref(DECLARED_RESOURCE)]),
    );
    // NB: schema.org `domainIncludes` is NOT emitted as `core:domain` (which would
    // restrict usage); it is inverted into each domain class's `core:recommends`
    // (see `convert`), faithfully preserving schema.org's advisory stance.

    // Partition the range. Entity (class/enum) targets must be in scope; an
    // out-of-scope target is dropped (and a literal target → data_type/format).
    let ranges = iri_refs(n, K_RANGE_INCLUDES);
    let mut entity_urns: Vec<String> = Vec::new();
    let mut entity_enum_curies: Vec<String> = Vec::new();
    let mut entity_nonenum = false;
    let mut dt_curies: Vec<String> = Vec::new();
    for c in &ranges {
        if !c.starts_with("schema:") {
            continue; // external range target — skip (cross-ref)
        }
        if datatype_set.contains(c) {
            dt_curies.push(c.clone());
        } else if let Some((u, _)) = map_schema_id(c) {
            if !kept.contains(&u) {
                continue; // out-of-scope (pending/meta) entity target — drop
            }
            entity_urns.push(u);
            if enum_set.contains(c) {
                entity_enum_curies.push(c.clone());
            } else {
                entity_nonenum = true;
            }
        }
    }

    let tier;
    if !entity_urns.is_empty() {
        r.set(iri(DATA_TYPE), rref(D_RESOURCE));
        r.set(
            iri(CLASS_TYPES),
            Value::Array(entity_urns.iter().map(|u| rref(u)).collect()),
        );
        // Closed-set enforcement only when every entity range is an enumeration.
        if !entity_enum_curies.is_empty() && !entity_nonenum {
            let mut members: BTreeSet<String> = BTreeSet::new();
            for e in &entity_enum_curies {
                if let Some((eurn, _)) = map_schema_id(e) {
                    if let Some(ms) = enum_members.get(&eurn) {
                        members.extend(ms.iter().cloned());
                    }
                }
            }
            if members.is_empty() {
                // Every ranged enumeration is genuinely member-less (members
                // defined in an external vocabulary, e.g. BusinessFunction). The
                // class_types typing stands, but no closed set can be formed —
                // the range stays open. Accounted explicitly (the cut), not silent.
                report.coverage.enumeration_open += 1;
                if report.coverage.enumeration_open_examples.len() < 20 {
                    report
                        .coverage
                        .enumeration_open_examples
                        .push(format!("{id} -> {}", entity_enum_curies.join(",")));
                }
            } else {
                r.set(
                    iri(ALLOWS_ONLY),
                    Value::Array(members.iter().map(|m| rref(m)).collect()),
                );
            }
            tier = RangeTier::Enumeration;
        } else if !dt_curies.is_empty() {
            tier = RangeTier::ByConvention; // mixed literal+entity → entity-first
        } else {
            tier = RangeTier::Clean; // single/all-Class
        }
    } else if dt_curies.len() == 1 {
        let (dt, fmt) = scalar_of(&dt_curies[0]);
        r.set(iri(DATA_TYPE), rref(dt));
        if let Some(f) = fmt {
            r.set(iri(FORMAT), rref(f));
        }
        tier = RangeTier::Clean;
    } else if dt_curies.len() > 1 {
        // format-spanning literal union → plain string (§3.2).
        r.set(iri(DATA_TYPE), rref(D_STRING));
        tier = RangeTier::ByConvention;
    } else {
        r.set(iri(DATA_TYPE), rref(D_STRING));
        tier = RangeTier::Defaulted;
    }

    common_meta(&mut r, n, &https);
    report.resources.push(r);
    report.coverage.properties += 1;
    *report
        .coverage
        .property_tiers
        .entry(format!("{tier:?}"))
        .or_default() += 1;
}

/// Record Tier-3 relational facts present on a node (the cut — not mapped).
fn record_residual(n: &Json, cov: &mut Coverage) {
    let id = node_id(n).unwrap_or("");
    for key in TIER3_KEYS {
        let refs = iri_refs(n, key);
        if !refs.is_empty() {
            *cov.residual_relations
                .entry((*key).to_string())
                .or_default() += 1;
            if cov.residual_examples.len() < 20 {
                cov.residual_examples
                    .push(format!("{id} {key} {}", refs.join(",")));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn graph() -> Vec<Json> {
        vec![
            json!({"@id":"schema:Thing","@type":"rdfs:Class","rdfs:label":"Thing"}),
            json!({"@id":"schema:CreativeWork","@type":"rdfs:Class","rdfs:subClassOf":{"@id":"schema:Thing"}}),
            json!({"@id":"schema:Dataset","@type":"rdfs:Class","rdfs:label":"Dataset","rdfs:subClassOf":{"@id":"schema:CreativeWork"}}),
            json!({"@id":"schema:Person","@type":"rdfs:Class","rdfs:subClassOf":{"@id":"schema:Thing"}}),
            json!({"@id":"schema:Organization","@type":"rdfs:Class","rdfs:subClassOf":{"@id":"schema:Thing"}}),
            json!({"@id":"schema:Text","@type":["rdfs:Class","schema:DataType"]}),
            json!({"@id":"schema:URL","@type":"rdfs:Class","rdfs:subClassOf":{"@id":"schema:Text"}}),
            json!({"@id":"schema:Enumeration","@type":"rdfs:Class","rdfs:subClassOf":{"@id":"schema:Thing"}}),
            json!({"@id":"schema:DayOfWeek","@type":"rdfs:Class","rdfs:subClassOf":{"@id":"schema:Enumeration"}}),
            json!({"@id":"schema:Monday","@type":"schema:DayOfWeek","rdfs:label":"Monday"}),
            json!({"@id":"schema:Secret","@type":"rdfs:Class","schema:isPartOf":{"@id":"https://pending.schema.org"}}),
            json!({"@id":"schema:name","@type":"rdf:Property","schema:rangeIncludes":{"@id":"schema:Text"}}),
            json!({"@id":"schema:url","@type":"rdf:Property","schema:rangeIncludes":{"@id":"schema:URL"}}),
            json!({"@id":"schema:author","@type":"rdf:Property","schema:domainIncludes":{"@id":"schema:CreativeWork"},"schema:rangeIncludes":[{"@id":"schema:Person"},{"@id":"schema:Organization"}]}),
            json!({"@id":"schema:about","@type":"rdf:Property","schema:rangeIncludes":[{"@id":"schema:Thing"},{"@id":"schema:Text"}]}),
            json!({"@id":"schema:dow","@type":"rdf:Property","schema:rangeIncludes":{"@id":"schema:DayOfWeek"}}),
            // Parent enumeration whose members live in a SUBCLASS enum — the
            // closed set must be the transitive member closure (regression for
            // the direct-only-members bug).
            json!({"@id":"schema:MedicalEnum","@type":"rdfs:Class","rdfs:subClassOf":{"@id":"schema:Enumeration"}}),
            json!({"@id":"schema:SurgicalSpec","@type":"rdfs:Class","rdfs:subClassOf":{"@id":"schema:MedicalEnum"}}),
            json!({"@id":"schema:Cardiac","@type":"schema:SurgicalSpec","rdfs:label":"Cardiac"}),
            json!({"@id":"schema:spec","@type":"rdf:Property","schema:rangeIncludes":{"@id":"schema:MedicalEnum"}}),
            // Genuinely empty enumeration (members defined in an external vocab) —
            // stays open (class_types only) and is accounted in enumeration_open.
            json!({"@id":"schema:EmptyEnum","@type":"rdfs:Class","rdfs:subClassOf":{"@id":"schema:Enumeration"}}),
            json!({"@id":"schema:emptyRanged","@type":"rdf:Property","schema:rangeIncludes":{"@id":"schema:EmptyEnum"}}),
            json!({"@id":"schema:rel","@type":"rdf:Property","schema:rangeIncludes":{"@id":"schema:Thing"},"schema:supersededBy":{"@id":"schema:about"}}),
            json!({"@id":"schema:badref","@type":"rdf:Property","schema:rangeIncludes":{"@id":"schema:Secret"}}),
        ]
    }

    fn find<'a>(rep: &'a ConvertReport, urn: &str) -> Option<&'a Resource> {
        rep.resources
            .iter()
            .find(|r| r.id().map(|i| i.as_str()) == Some(urn))
    }
    fn refs(r: &Resource, prop: &str) -> Vec<String> {
        match r.get(&iri(prop)) {
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|v| v.as_iri_str().map(String::from))
                .collect(),
            Some(v) => v
                .as_iri_str()
                .map(|s| vec![s.to_string()])
                .unwrap_or_default(),
            None => Vec::new(),
        }
    }

    #[test]
    fn class_maps_with_subclass_and_grade() {
        let rep = convert(&graph());
        let d = find(&rep, "urn:schema_org:Dataset").expect("Dataset emitted");
        assert_eq!(refs(d, IS_A), vec![CORE_CLASS, DECLARED_RESOURCE]);
        assert_eq!(refs(d, SUBCLASS_OF), vec!["urn:schema_org:CreativeWork"]);
        assert_eq!(
            d.get(&iri(SOURCE_IRL)).and_then(|v| v.as_str()),
            Some("https://schema.org/Dataset")
        );
    }

    #[test]
    fn datatypes_folded_not_emitted() {
        let rep = convert(&graph());
        assert!(find(&rep, "urn:schema_org:Text").is_none(), "Text folded");
        assert!(find(&rep, "urn:schema_org:URL").is_none(), "URL folded");
        assert!(rep
            .coverage
            .datatypes_folded
            .iter()
            .any(|d| d == "schema:URL"));
    }

    #[test]
    fn single_datatype_ranges() {
        let rep = convert(&graph());
        let name = find(&rep, "urn:schema_org:name").unwrap();
        assert_eq!(refs(name, DATA_TYPE), vec![D_STRING]);
        assert!(name.get(&iri(FORMAT)).is_none());
        let url = find(&rep, "urn:schema_org:url").unwrap();
        assert_eq!(refs(url, DATA_TYPE), vec![D_STRING]);
        assert_eq!(refs(url, FORMAT), vec![F_IRI]); // URL → string + format=iri
    }

    #[test]
    fn all_class_union_and_entity_first() {
        let rep = convert(&graph());
        let author = find(&rep, "urn:schema_org:author").unwrap();
        assert_eq!(refs(author, DATA_TYPE), vec![D_RESOURCE]);
        assert_eq!(
            refs(author, CLASS_TYPES),
            vec!["urn:schema_org:Person", "urn:schema_org:Organization"]
        );
        // mixed Thing|Text → entity-first: class_types=[Thing], Text dropped.
        let about = find(&rep, "urn:schema_org:about").unwrap();
        assert_eq!(refs(about, CLASS_TYPES), vec!["urn:schema_org:Thing"]);
    }

    #[test]
    fn domain_becomes_class_recommends_not_domain() {
        let rep = convert(&graph());
        // author's domainIncludes CreativeWork → CreativeWork recommends author.
        let cw = find(&rep, "urn:schema_org:CreativeWork").unwrap();
        assert!(refs(cw, RECOMMENDS).contains(&"urn:schema_org:author".to_string()));
        // and core:domain is NOT emitted (schema.org doesn't restrict).
        let author = find(&rep, "urn:schema_org:author").unwrap();
        assert!(author.get(&iri("urn:eigenius:core:domain")).is_none());
    }

    #[test]
    fn enumeration_range_closes_with_allows_only() {
        let rep = convert(&graph());
        assert!(
            find(&rep, "urn:schema_org:Monday").is_some(),
            "member emitted"
        );
        let dow = find(&rep, "urn:schema_org:dow").unwrap();
        assert_eq!(refs(dow, CLASS_TYPES), vec!["urn:schema_org:DayOfWeek"]);
        assert_eq!(refs(dow, ALLOWS_ONLY), vec!["urn:schema_org:Monday"]);
    }

    #[test]
    fn enumeration_closure_is_transitive_and_open_enums_accounted() {
        let rep = convert(&graph());
        // spec ranges on the PARENT enum; its only member lives in a subclass —
        // the closed set must still include it (transitive closure).
        let spec = find(&rep, "urn:schema_org:spec").unwrap();
        assert_eq!(refs(spec, CLASS_TYPES), vec!["urn:schema_org:MedicalEnum"]);
        assert_eq!(refs(spec, ALLOWS_ONLY), vec!["urn:schema_org:Cardiac"]);
        // emptyRanged ranges on a member-less enum: typed, but no closable set.
        let empty = find(&rep, "urn:schema_org:emptyRanged").unwrap();
        assert_eq!(refs(empty, CLASS_TYPES), vec!["urn:schema_org:EmptyEnum"]);
        assert!(empty.get(&iri(ALLOWS_ONLY)).is_none());
        assert_eq!(rep.coverage.enumeration_open, 1);
        assert!(rep
            .coverage
            .enumeration_open_examples
            .iter()
            .any(|e| e.contains("schema:emptyRanged")));
    }

    #[test]
    fn pending_excluded_and_refs_dropped() {
        let rep = convert(&graph());
        assert!(
            find(&rep, "urn:schema_org:Secret").is_none(),
            "pending excluded"
        );
        assert!(rep.coverage.excluded_layer >= 1);
        // a property ranging only over the pending class → ref dropped → string.
        let bad = find(&rep, "urn:schema_org:badref").unwrap();
        assert!(bad.get(&iri(CLASS_TYPES)).is_none());
        assert_eq!(refs(bad, DATA_TYPE), vec![D_STRING]);
    }

    #[test]
    fn tier3_recorded_not_mapped() {
        let rep = convert(&graph());
        assert_eq!(
            rep.coverage.residual_relations.get("schema:supersededBy"),
            Some(&1)
        );
        let rel = find(&rep, "urn:schema_org:rel").unwrap();
        // supersededBy is NOT emitted as a property on the resource.
        assert!(rel.get(&iri("urn:schema_org:supersededBy")).is_none());
    }

    #[test]
    fn deterministic() {
        let a = convert(&graph());
        let b = convert(&graph());
        let da = eigenius_kernel::ontology::eigon_json::serialize_document(&a.resources);
        let db = eigenius_kernel::ontology::eigon_json::serialize_document(&b.resources);
        assert_eq!(da, db, "conversion must be deterministic");
    }
}
