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

//! Parser for NCBI Gene's `gene_info` tab-delimited dump (the bulk projection of
//! the ASN.1 `Entrezgene` record).
//!
//! Column spec (verified against the Gene FTP README,
//! <https://ftp.ncbi.nih.gov/gene/DATA/README>):
//!
//! | # | column | notes |
//! |---|--------|-------|
//! | 1 | tax_id | NCBI Taxonomy id of the species |
//! | 2 | GeneID | the Entrez gene identifier (stable; `^\d+$`) |
//! | 3 | Symbol | the default symbol |
//! | 4 | LocusTag | locus tag, or `-` |
//! | 5 | Synonyms | bar-delimited unofficial symbols, or `-` |
//! | 6 | dbXrefs | bar-delimited cross-database ids, or `-` |
//! | 7 | chromosome | or `-` |
//! | 8 | map_location | or `-` |
//! | 9 | description | a descriptive name |
//! | 10 | type_of_gene | protein-coding / ncRNA / pseudo / … |
//! | 11 | Symbol_from_nomenclature_authority | or `-` |
//! | 12 | Full_name_from_nomenclature_authority | or `-` |
//! | 13 | Nomenclature_status | O / I / `-` |
//! | 14 | Other_designations | pipe-delimited, or `-` |
//! | 15 | Modification_date | YYYYMMDD |
//! | 16 | Feature_type | pipe-delimited, or `-` |
//!
//! A `-` denotes an absent value (NCBI's null convention).

/// One parsed `gene_info` row — the subset of fields the mirror keeps. Lossless for
/// what we model; columns we don't mirror (nomenclature status, modification date,
/// feature type) are dropped at parse time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneInfo {
    /// Column 1 — NCBI Taxonomy id (e.g. `9606` for Homo sapiens).
    pub tax_id: String,
    /// Column 2 — the Entrez GeneID (stable identifier).
    pub gene_id: String,
    /// Column 3 — the default gene symbol (the primary surface form).
    pub symbol: String,
    /// Column 4 — LocusTag, if present.
    pub locus_tag: Option<String>,
    /// Column 5 — unofficial alternate symbols (additional surface forms).
    pub synonyms: Vec<String>,
    /// Column 6 — cross-database identifiers (HGNC, Ensembl, MIM, …).
    pub dbxrefs: Vec<String>,
    /// Column 7 — chromosome, if present.
    pub chromosome: Option<String>,
    /// Column 8 — cytogenetic map location, if present.
    pub map_location: Option<String>,
    /// Column 9 — a descriptive name (the gloss).
    pub description: String,
    /// Column 10 — the NCBI gene type vocabulary value.
    pub type_of_gene: String,
    /// Column 12 — full name from the nomenclature authority, if present.
    pub full_name: Option<String>,
}

/// NCBI's null sentinel — a lone `-` means "no value".
fn dash(s: &str) -> Option<String> {
    if s == "-" || s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// Split a bar-delimited (`|`) NCBI list field; `-` ⇒ empty.
fn bar(s: &str) -> Vec<String> {
    match dash(s) {
        None => Vec::new(),
        Some(v) => v
            .split('|')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect(),
    }
}

/// Parse one `gene_info` line. Returns `None` for comment lines (`#…`) and rows
/// with too few columns (malformed / truncated). Fail-soft: a bad row is skipped,
/// never panics — the importer reports the kept count.
pub fn parse_line(line: &str) -> Option<GeneInfo> {
    if line.starts_with('#') {
        return None;
    }
    let f: Vec<&str> = line.split('\t').collect();
    if f.len() < 12 {
        return None;
    }
    let gene_id = f[1].trim();
    let symbol = f[2].trim();
    if gene_id.is_empty() || symbol.is_empty() || symbol == "-" {
        return None;
    }
    Some(GeneInfo {
        tax_id: f[0].trim().to_string(),
        gene_id: gene_id.to_string(),
        symbol: symbol.to_string(),
        locus_tag: dash(f[3].trim()),
        synonyms: bar(f[4]),
        dbxrefs: bar(f[5]),
        chromosome: dash(f[6].trim()),
        map_location: dash(f[7].trim()),
        description: f[8].trim().to_string(),
        type_of_gene: f[9].trim().to_string(),
        full_name: dash(f[11].trim()),
    })
}

/// Parse a whole `gene_info` document, keeping only rows for `tax_id`
/// (e.g. `"9606"`) — the dump may be multi-organism, the mirror is per-species.
pub fn parse_document(text: &str, tax_id: &str) -> Vec<GeneInfo> {
    text.lines()
        .filter_map(parse_line)
        .filter(|g| g.tax_id == tax_id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // A real-shape WRN row (human, GeneID 7486). 16 tab-separated columns.
    const WRN: &str = "9606\t7486\tWRN\t-\tRECQ3|RECQL2|RECQL3\tMIM:604611|HGNC:HGNC:12791|Ensembl:ENSG00000165392\t8\t8p12\tWerner syndrome RecQ like helicase\tprotein-coding\tWRN\tWerner syndrome RecQ like helicase\tO\tWerner syndrome, RecQ helicase-like\t20240101\t-";

    #[test]
    fn parses_a_real_wrn_row() {
        let g = parse_line(WRN).expect("WRN row parses");
        assert_eq!(g.gene_id, "7486");
        assert_eq!(g.symbol, "WRN");
        assert_eq!(g.tax_id, "9606");
        assert_eq!(g.synonyms, vec!["RECQ3", "RECQL2", "RECQL3"]);
        assert_eq!(g.description, "Werner syndrome RecQ like helicase");
        assert_eq!(g.type_of_gene, "protein-coding");
        assert_eq!(g.chromosome.as_deref(), Some("8"));
        assert_eq!(g.map_location.as_deref(), Some("8p12"));
        assert_eq!(g.locus_tag, None); // a lone `-`
        assert_eq!(g.dbxrefs.len(), 3);
        assert_eq!(
            g.full_name.as_deref(),
            Some("Werner syndrome RecQ like helicase")
        );
    }

    #[test]
    fn skips_comments_and_filters_by_taxon() {
        let doc = format!("#header line\n{WRN}\n9031\t999\tGGsym\t-\t-\t-\t-\t-\tchicken gene\tprotein-coding\t-\t-\t-\t-\t-\t-");
        let human = parse_document(&doc, "9606");
        assert_eq!(
            human.len(),
            1,
            "only the human row survives the taxon filter"
        );
        assert_eq!(human[0].symbol, "WRN");
    }
}
