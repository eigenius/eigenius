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

//! D53 §4 dataset-schema parsing + the checkable layout gate.
//!
//! A `DatasetSchema` resource binds a tabular file's internal structure to
//! graph types — dimensions (axes → classes), measures (values → properties),
//! attributes (per-component qualifiers), and a physical layout (wide matrix vs
//! long table). This module turns that resource into a typed Rust view and
//! provides the **checkable invariant gate** (§4.1: "checkable, not just
//! asserted"): a cheap header scan that validates the declared layout against a
//! file's actual columns *before* a recompute trusts it.
//!
//! Scope (Phase 3): the **tabular** profile against a delimited (CSV/TSV)
//! header — the format the WRN DepMap matrices actually use. Parquet/Arrow
//! carry their own column schema in-file; validating those substrate-side would
//! pull the heavy `parquet`/`arrow` crates (the same weight we declined for
//! `liboxen`), and the worker reads them with native tooling regardless, so the
//! columnar gate is deferred to the worker. The correctness root is unchanged
//! either way: `content_hash` (D53 §5) plus the worker honoring the schema
//! (§4.3). This gate is an *early, cheap* mismatch catch, not the trust root.

use eigenius_kernel::ontology::iri::Iri;
use eigenius_kernel::ontology::resource::{Resource, Value};

const PROP_DIMENSION: &str = "urn:eigenius:ingest:dimension";
const PROP_MEASURE: &str = "urn:eigenius:ingest:measure";
const PROP_ATTRIBUTE: &str = "urn:eigenius:ingest:attribute";
const PROP_LAYOUT: &str = "urn:eigenius:ingest:layout";
const PROP_MEMBER: &str = "urn:eigenius:ingest:member";
const PROP_NAME: &str = "urn:eigenius:ingest:name";
const PROP_CLASS: &str = "urn:eigenius:ingest:class";
const PROP_PROPERTY: &str = "urn:eigenius:ingest:property";
const PROP_CODE_LIST: &str = "urn:eigenius:ingest:code_list";
const PROP_DATA_TYPE: &str = "urn:eigenius:ingest:data_type";
const PROP_SOURCE: &str = "urn:eigenius:ingest:source";
const PROP_KIND: &str = "urn:eigenius:ingest:kind";
const PROP_ROW_KEY: &str = "urn:eigenius:ingest:row_key";
const PROP_ROW_KEY_BINDS: &str = "urn:eigenius:ingest:row_key_binds";
const PROP_COLUMN_DIMENSION: &str = "urn:eigenius:ingest:column_dimension";
const PROP_HEADER_PARSE: &str = "urn:eigenius:ingest:header_parse";
const PROP_CELL_MEASURE: &str = "urn:eigenius:ingest:cell_measure";
const PROP_MEMBER_DIMENSION: &str = "urn:eigenius:ingest:member_dimension";
const PROP_MEMBER_START_COLUMN: &str = "urn:eigenius:ingest:member_start_column";

/// A dimension — an identifying axis bound to a graph class (FK target).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Dimension {
    pub name: String,
    pub class: Option<String>,
    pub code_list: Option<String>,
    pub source: Option<String>,
}

/// A measure — a cube value bound to a graph property.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Measure {
    pub name: String,
    pub property: Option<String>,
    pub data_type: Option<String>,
    pub source: Option<String>,
}

/// An attribute — a per-component qualifier bound to a property.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Attribute {
    pub name: String,
    pub property: Option<String>,
    pub data_type: Option<String>,
    pub source: Option<String>,
}

/// The physical layout binding — how the semantic cube sits in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutKind {
    /// Row key × entity-per-column; cells carry the measure.
    WideMatrix,
    /// One column per component.
    LongTable,
    /// Ragged set→member-list (e.g. a `.gmt` gene-set file): each row is a
    /// named set, the row key binds the set dimension, and the variable-length
    /// trailing fields bind the member dimension (D53 §4/§10).
    Collection,
    /// An unrecognized kind string — surfaced as a validation issue.
    Other(String),
}

impl LayoutKind {
    fn parse(s: &str) -> Self {
        match s {
            "WideMatrix" => LayoutKind::WideMatrix,
            "LongTable" => LayoutKind::LongTable,
            "Collection" => LayoutKind::Collection,
            other => LayoutKind::Other(other.to_string()),
        }
    }
}

/// The layout binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub kind: LayoutKind,
    pub row_key: Option<String>,
    pub row_key_binds: Option<String>,
    pub column_dimension: Option<String>,
    pub header_parse: Option<String>,
    pub cell_measure: Option<String>,
    /// Collection: the dimension name the list members bind to.
    pub member_dimension: Option<String>,
    /// Collection: the 0-based column index where a row's members begin.
    pub member_start_column: Option<String>,
}

/// A typed view of a `DatasetSchema` resource (D53 §4.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DatasetSchema {
    /// Intra-file selector (.rds member / .xlsx sheet) for a multi-matrix
    /// container, else `None`.
    pub member: Option<String>,
    pub dimensions: Vec<Dimension>,
    pub measures: Vec<Measure>,
    pub attributes: Vec<Attribute>,
    pub layout: Option<Layout>,
}

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("static IRI")
}

fn read_str(r: &Resource, prop: &str) -> Option<String> {
    r.get(&iri(prop)).and_then(|v| {
        v.as_str()
            .map(str::to_string)
            .or_else(|| v.as_iri_str().map(str::to_string))
    })
}

/// Read a property that holds one or more embedded component resources.
fn embedded_components(r: &Resource, prop: &str) -> Vec<Resource> {
    match r.get(&iri(prop)) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| match v {
                Value::Embedded(b) => Some((**b).clone()),
                _ => None,
            })
            .collect(),
        Some(Value::Embedded(b)) => vec![(**b).clone()],
        _ => Vec::new(),
    }
}

fn parse_dimension(r: &Resource) -> Dimension {
    Dimension {
        name: read_str(r, PROP_NAME).unwrap_or_default(),
        class: read_str(r, PROP_CLASS),
        code_list: read_str(r, PROP_CODE_LIST),
        source: read_str(r, PROP_SOURCE),
    }
}

fn parse_measure(r: &Resource) -> Measure {
    Measure {
        name: read_str(r, PROP_NAME).unwrap_or_default(),
        property: read_str(r, PROP_PROPERTY),
        data_type: read_str(r, PROP_DATA_TYPE),
        source: read_str(r, PROP_SOURCE),
    }
}

fn parse_attribute(r: &Resource) -> Attribute {
    Attribute {
        name: read_str(r, PROP_NAME).unwrap_or_default(),
        property: read_str(r, PROP_PROPERTY),
        data_type: read_str(r, PROP_DATA_TYPE),
        source: read_str(r, PROP_SOURCE),
    }
}

fn parse_layout(r: &Resource) -> Option<Layout> {
    let l = embedded_components(r, PROP_LAYOUT).into_iter().next()?;
    Some(Layout {
        kind: LayoutKind::parse(&read_str(&l, PROP_KIND).unwrap_or_default()),
        row_key: read_str(&l, PROP_ROW_KEY),
        row_key_binds: read_str(&l, PROP_ROW_KEY_BINDS),
        column_dimension: read_str(&l, PROP_COLUMN_DIMENSION),
        header_parse: read_str(&l, PROP_HEADER_PARSE),
        cell_measure: read_str(&l, PROP_CELL_MEASURE),
        member_dimension: read_str(&l, PROP_MEMBER_DIMENSION),
        member_start_column: read_str(&l, PROP_MEMBER_START_COLUMN),
    })
}

/// Parse a `DatasetSchema` resource into its typed view.
pub fn parse_dataset_schema(r: &Resource) -> DatasetSchema {
    DatasetSchema {
        member: read_str(r, PROP_MEMBER),
        dimensions: embedded_components(r, PROP_DIMENSION)
            .iter()
            .map(parse_dimension)
            .collect(),
        measures: embedded_components(r, PROP_MEASURE)
            .iter()
            .map(parse_measure)
            .collect(),
        attributes: embedded_components(r, PROP_ATTRIBUTE)
            .iter()
            .map(parse_attribute)
            .collect(),
        layout: parse_layout(r),
    }
}

/// Split the header line of a delimited file into column names. `media_type`
/// picks the delimiter: `text/tab-separated-values` → tab, else comma.
pub fn header_columns(first_line: &str, media_type: &str) -> Vec<String> {
    let delim = if media_type == "text/tab-separated-values" {
        '\t'
    } else {
        ','
    };
    first_line
        .trim_end_matches(['\r', '\n'])
        .split(delim)
        .map(|c| c.trim_matches('"').to_string())
        .collect()
}

/// Validate a tabular schema's declared layout against a file's actual header
/// columns. Returns a list of human-readable issues (empty ⇒ the layout is
/// consistent with the header). This is the §4.1 checkable gate — cheap, and
/// catches a mis-declared layout before a recompute trusts it.
pub fn validate_tabular(schema: &DatasetSchema, header: &[String]) -> Vec<String> {
    let mut issues = Vec::new();
    let has = |col: &str| header.iter().any(|h| h == col);

    let Some(layout) = &schema.layout else {
        // No layout (e.g. a member-only container schema); nothing to header-check.
        return issues;
    };

    match &layout.kind {
        LayoutKind::LongTable => {
            if let Some(rk) = &layout.row_key {
                if !has(rk) {
                    issues.push(format!("row_key column `{rk}` not found in header"));
                }
            }
            // Every component with a declared physical column must be present.
            for d in &schema.dimensions {
                check_source("dimension", &d.name, &d.source, &has, &mut issues);
            }
            for m in &schema.measures {
                check_source("measure", &m.name, &m.source, &has, &mut issues);
            }
            for a in &schema.attributes {
                check_source("attribute", &a.name, &a.source, &has, &mut issues);
            }
        }
        LayoutKind::WideMatrix => {
            if let Some(rk) = &layout.row_key {
                if !has(rk) {
                    issues.push(format!("row_key column `{rk}` not found in header"));
                }
            }
            // Entity-per-column: there must be data columns beyond the row key,
            // and (if a header_parse template is declared) at least one column
            // must match it.
            let data_cols: Vec<&String> = header
                .iter()
                .filter(|h| Some(h.as_str()) != layout.row_key.as_deref())
                .collect();
            if data_cols.is_empty() {
                issues.push("wide matrix has no data columns beyond the row key".to_string());
            } else if let Some(tmpl) = &layout.header_parse {
                if !data_cols.iter().any(|h| header_template_matches(tmpl, h)) {
                    issues.push(format!(
                        "no column matches the header_parse template `{tmpl}`"
                    ));
                }
            }
        }
        LayoutKind::Collection => {
            // A Collection is ragged (no header row) — validate with
            // `validate_collection` over data lines, not a header.
            issues.push(
                "Collection layout is not header-validatable — use validate_collection".to_string(),
            );
        }
        LayoutKind::Other(k) => {
            issues.push(format!("unrecognized layout kind `{k}` — cannot validate"));
        }
    }
    issues
}

/// Validate a `Collection` (ragged set→member-list, e.g. a `.gmt`) against
/// sampled data rows. Each `row` is the delimiter-split fields of one line.
/// Returns issues (empty ⇒ consistent). Cheap + structural: every sampled row
/// must carry at least one member past `member_start_column`, and the layout
/// must name a declared member dimension. Member *membership* against a bound
/// code-list is a separate check (see the code_list-resolution follow-up).
pub fn validate_collection(schema: &DatasetSchema, rows: &[Vec<String>]) -> Vec<String> {
    let mut issues = Vec::new();
    let Some(layout) = &schema.layout else {
        return issues;
    };
    if layout.kind != LayoutKind::Collection {
        issues.push("validate_collection called on a non-Collection layout".to_string());
        return issues;
    }

    // The member list begins at `member_start_column` (0-based). Default: one
    // past the set's row-key column (members right after the set id).
    let row_key_idx = layout
        .row_key
        .as_deref()
        .and_then(|s| s.parse::<usize>().ok());
    let member_start = layout
        .member_start_column
        .as_deref()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or_else(|| row_key_idx.map(|i| i + 1).unwrap_or(1));

    // The member dimension must be declared.
    match &layout.member_dimension {
        Some(md) if schema.dimensions.iter().any(|d| &d.name == md) => {}
        Some(md) => issues.push(format!(
            "member_dimension `{md}` is not a declared dimension"
        )),
        None => issues.push("Collection layout is missing member_dimension".to_string()),
    }

    if rows.is_empty() {
        issues.push("file is empty — no sets found".to_string());
        return issues;
    }
    for (i, fields) in rows.iter().enumerate() {
        if fields.len() <= member_start {
            issues.push(format!(
                "row {} has no members ({} field(s); members start at column {member_start})",
                i + 1,
                fields.len()
            ));
        }
    }
    issues
}

fn check_source(
    role: &str,
    name: &str,
    source: &Option<String>,
    has: &impl Fn(&str) -> bool,
    issues: &mut Vec<String>,
) {
    if let Some(s) = source {
        if !has(s) {
            issues.push(format!(
                "{role} `{name}` source column `{s}` not found in header"
            ));
        }
    }
}

/// Whether a wide-matrix column header matches an `ingest:header_parse` template
/// like `<symbol> (<entrez>)`. The template's `<…>` placeholders match ≥1 char;
/// the literal chunks between them must appear in order, and the header must be
/// longer than the literals alone (so each placeholder captured something).
fn header_template_matches(template: &str, header: &str) -> bool {
    // Split the template into literal chunks around `<…>` placeholders.
    let mut literals: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_placeholder = false;
    for c in template.chars() {
        match c {
            '<' if !in_placeholder => {
                in_placeholder = true;
                literals.push(std::mem::take(&mut cur));
            }
            '>' if in_placeholder => in_placeholder = false,
            _ if !in_placeholder => cur.push(c),
            _ => {}
        }
    }
    literals.push(cur);

    // `literals[0]` is the prefix (before the first placeholder); a placeholder
    // sits between each consecutive pair. Walk the header consuming each literal
    // in order; every placeholder must capture ≥1 char.
    let Some(mut rest) = header.strip_prefix(literals[0].as_str()) else {
        return false;
    };
    for (i, lit) in literals.iter().enumerate().skip(1) {
        let is_last = i == literals.len() - 1;
        if lit.is_empty() {
            // Trailing placeholder (template ends with `<…>`): it must capture
            // ≥1 char, i.e. there must be something left.
            if is_last {
                return !rest.is_empty();
            }
            // Adjacent placeholders with no literal between — require ≥1 char.
            if rest.is_empty() {
                return false;
            }
            continue;
        }
        match rest.find(lit.as_str()) {
            // `pos >= 1` ⇒ the preceding placeholder captured ≥1 char.
            Some(pos) if pos >= 1 => rest = &rest[pos + lit.len()..],
            _ => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn embedded(props: &[(&str, Value)]) -> Value {
        let mut r = Resource::new_embedded();
        for (p, v) in props {
            r.set(iri(p), v.clone());
        }
        Value::Embedded(Box::new(r))
    }

    fn s(v: &str) -> Value {
        Value::String(v.to_string())
    }

    fn rref(v: &str) -> Value {
        Value::ResourceRef(iri(v))
    }

    /// Build the CERES wide-matrix schema from the worked example.
    fn ceres_schema() -> Resource {
        let mut schema = Resource::new(iri("urn:eigenius:pub:wrn:ceres_matrix_schema"));
        schema.set(
            iri(PROP_DIMENSION),
            Value::Array(vec![
                embedded(&[
                    (PROP_NAME, s("cell_line")),
                    (PROP_CLASS, rref("urn:eigenius:onco:CellLine")),
                ]),
                embedded(&[
                    (PROP_NAME, s("gene")),
                    (PROP_CLASS, rref("urn:eigenius:onco:Gene")),
                ]),
            ]),
        );
        schema.set(
            iri(PROP_MEASURE),
            Value::Array(vec![embedded(&[
                (PROP_NAME, s("ceres")),
                (PROP_PROPERTY, rref("urn:eigenius:onco:dependency_score")),
                (PROP_DATA_TYPE, rref("urn:eigenius:core:float")),
            ])]),
        );
        schema.set(
            iri(PROP_LAYOUT),
            embedded(&[
                (PROP_KIND, s("WideMatrix")),
                (PROP_ROW_KEY, s("DepMap_ID")),
                (PROP_ROW_KEY_BINDS, s("cell_line")),
                (PROP_COLUMN_DIMENSION, s("gene")),
                (PROP_HEADER_PARSE, s("<symbol> (<entrez>)")),
                (PROP_CELL_MEASURE, s("ceres")),
            ]),
        );
        schema
    }

    #[test]
    fn parses_wide_matrix_schema() {
        let parsed = parse_dataset_schema(&ceres_schema());
        assert_eq!(parsed.dimensions.len(), 2);
        assert_eq!(parsed.dimensions[0].name, "cell_line");
        assert_eq!(
            parsed.dimensions[0].class.as_deref(),
            Some("urn:eigenius:onco:CellLine")
        );
        assert_eq!(parsed.measures.len(), 1);
        assert_eq!(
            parsed.measures[0].property.as_deref(),
            Some("urn:eigenius:onco:dependency_score")
        );
        let layout = parsed.layout.unwrap();
        assert_eq!(layout.kind, LayoutKind::WideMatrix);
        assert_eq!(layout.row_key.as_deref(), Some("DepMap_ID"));
        assert_eq!(layout.header_parse.as_deref(), Some("<symbol> (<entrez>)"));
    }

    #[test]
    fn wide_matrix_header_validates() {
        let parsed = parse_dataset_schema(&ceres_schema());
        let header = header_columns("DepMap_ID,WRN (7486),BRCA1 (672)", "text/csv");
        assert_eq!(validate_tabular(&parsed, &header), Vec::<String>::new());
    }

    #[test]
    fn wide_matrix_missing_row_key_flagged() {
        let parsed = parse_dataset_schema(&ceres_schema());
        let header = header_columns("WRN (7486),BRCA1 (672)", "text/csv");
        let issues = validate_tabular(&parsed, &header);
        assert!(issues.iter().any(|i| i.contains("row_key")), "{issues:?}");
    }

    #[test]
    fn wide_matrix_header_parse_mismatch_flagged() {
        let parsed = parse_dataset_schema(&ceres_schema());
        // Columns present but none match `<symbol> (<entrez>)`.
        let header = header_columns("DepMap_ID,WRN,BRCA1", "text/csv");
        let issues = validate_tabular(&parsed, &header);
        assert!(
            issues.iter().any(|i| i.contains("header_parse")),
            "{issues:?}"
        );
    }

    fn long_schema() -> Resource {
        let mut schema = Resource::new(iri("urn:eigenius:pub:wrn:supp_table_1_schema"));
        schema.set(
            iri(PROP_DIMENSION),
            embedded(&[
                (PROP_NAME, s("cell_line")),
                (PROP_CLASS, rref("urn:eigenius:onco:CellLine")),
                (PROP_SOURCE, s("CCLE_ID")),
            ]),
        );
        schema.set(
            iri(PROP_MEASURE),
            Value::Array(vec![embedded(&[
                (PROP_NAME, s("avg_WRN_dep")),
                (PROP_SOURCE, s("avg_WRN_dep")),
            ])]),
        );
        schema.set(
            iri(PROP_ATTRIBUTE),
            Value::Array(vec![embedded(&[
                (PROP_NAME, s("CCLE_MSI")),
                (PROP_SOURCE, s("CCLE_MSI")),
            ])]),
        );
        schema.set(
            iri(PROP_LAYOUT),
            embedded(&[(PROP_KIND, s("LongTable")), (PROP_ROW_KEY, s("CCLE_ID"))]),
        );
        schema
    }

    #[test]
    fn long_table_validates_and_flags_missing_columns() {
        let parsed = parse_dataset_schema(&long_schema());
        let ok = header_columns("CCLE_ID,avg_WRN_dep,CCLE_MSI,extra", "text/csv");
        assert_eq!(validate_tabular(&parsed, &ok), Vec::<String>::new());

        let missing = header_columns("CCLE_ID,CCLE_MSI", "text/csv");
        let issues = validate_tabular(&parsed, &missing);
        assert!(
            issues.iter().any(|i| i.contains("avg_WRN_dep")),
            "{issues:?}"
        );
    }

    #[test]
    fn header_template_matcher() {
        assert!(header_template_matches("<symbol> (<entrez>)", "WRN (7486)"));
        assert!(header_template_matches(
            "<symbol> (<entrez>)",
            "BRCA1 (672)"
        ));
        assert!(!header_template_matches("<symbol> (<entrez>)", "WRN")); // no " (...)"
        assert!(!header_template_matches("<symbol> (<entrez>)", "WRN ()")); // empty entrez
        assert!(header_template_matches("ENSG<id>", "ENSG000001"));
        assert!(!header_template_matches("ENSG<id>", "ENSG")); // placeholder empty
    }

    #[test]
    fn tsv_delimiter_and_quote_stripping() {
        let cols = header_columns("\"a\"\t\"b c\"\t\"d\"", "text/tab-separated-values");
        assert_eq!(cols, vec!["a", "b c", "d"]);
    }

    /// A Hallmark-style `.gmt` collection schema: sets bound by row key (col 0),
    /// gene members from col 2 on.
    fn gmt_schema() -> Resource {
        let mut schema = Resource::new(iri("urn:eigenius:pub:wrn:hallmark_schema"));
        schema.set(
            iri(PROP_DIMENSION),
            Value::Array(vec![
                embedded(&[
                    (PROP_NAME, s("gene_set")),
                    (PROP_CLASS, rref("urn:eigenius:onco:GeneSet")),
                ]),
                embedded(&[
                    (PROP_NAME, s("gene")),
                    (PROP_CLASS, rref("urn:eigenius:onco:Gene")),
                ]),
            ]),
        );
        schema.set(
            iri(PROP_LAYOUT),
            embedded(&[
                (PROP_KIND, s("Collection")),
                (PROP_ROW_KEY, s("0")),
                (PROP_ROW_KEY_BINDS, s("gene_set")),
                (PROP_MEMBER_DIMENSION, s("gene")),
                (PROP_MEMBER_START_COLUMN, s("2")),
            ]),
        );
        schema
    }

    fn split_rows(lines: &[&str]) -> Vec<Vec<String>> {
        lines
            .iter()
            .map(|l| header_columns(l, "text/tab-separated-values"))
            .collect()
    }

    #[test]
    fn parses_collection_schema() {
        let parsed = parse_dataset_schema(&gmt_schema());
        let layout = parsed.layout.unwrap();
        assert_eq!(layout.kind, LayoutKind::Collection);
        assert_eq!(layout.member_dimension.as_deref(), Some("gene"));
        assert_eq!(layout.member_start_column.as_deref(), Some("2"));
    }

    #[test]
    fn collection_validates_ragged_rows() {
        let parsed = parse_dataset_schema(&gmt_schema());
        // set \t description \t members…  (ragged: different lengths, all ≥1 member)
        let rows = split_rows(&[
            "HALLMARK_DNA_REPAIR\thttp://...\tWRN\tBRCA1\tATM",
            "HALLMARK_APOPTOSIS\thttp://...\tCASP3\tBAX",
        ]);
        assert_eq!(validate_collection(&parsed, &rows), Vec::<String>::new());
    }

    #[test]
    fn collection_flags_memberless_row() {
        let parsed = parse_dataset_schema(&gmt_schema());
        // Second row has set + description but no members.
        let rows = split_rows(&[
            "HALLMARK_DNA_REPAIR\tdesc\tWRN\tBRCA1",
            "HALLMARK_EMPTY\tdesc",
        ]);
        let issues = validate_collection(&parsed, &rows);
        assert!(issues.iter().any(|i| i.contains("row 2")), "{issues:?}");
    }

    #[test]
    fn collection_flags_undeclared_member_dimension() {
        let mut schema = Resource::new(iri("urn:eigenius:demo:bad_collection"));
        schema.set(
            iri(PROP_DIMENSION),
            Value::Array(vec![embedded(&[(PROP_NAME, s("gene_set"))])]),
        );
        schema.set(
            iri(PROP_LAYOUT),
            embedded(&[
                (PROP_KIND, s("Collection")),
                (PROP_MEMBER_DIMENSION, s("gene")), // not declared
                (PROP_MEMBER_START_COLUMN, s("2")),
            ]),
        );
        let parsed = parse_dataset_schema(&schema);
        let rows = split_rows(&["S1\td\tA\tB"]);
        let issues = validate_collection(&parsed, &rows);
        assert!(
            issues.iter().any(|i| i.contains("member_dimension")),
            "{issues:?}"
        );
    }

    #[test]
    fn validate_tabular_rejects_collection_kind() {
        let parsed = parse_dataset_schema(&gmt_schema());
        let issues = validate_tabular(&parsed, &["0".to_string()]);
        assert!(
            issues.iter().any(|i| i.contains("validate_collection")),
            "{issues:?}"
        );
    }

    #[test]
    fn no_layout_is_vacuously_ok() {
        let mut schema = Resource::new_embedded();
        schema.set(iri(PROP_MEMBER), s("GE"));
        let parsed = parse_dataset_schema(&schema);
        assert_eq!(parsed.member.as_deref(), Some("GE"));
        assert_eq!(
            validate_tabular(&parsed, &["anything".to_string()]),
            Vec::<String>::new()
        );
    }
}
