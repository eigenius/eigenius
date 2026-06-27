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

//! Render parsed [`GeneInfo`] rows into an Eigon/ESL document: a faithful **typed
//! mirror** of NCBI Gene plus a **derived domain lexicon** (D65 §5).
//!
//! Two outputs, one document, layered so the lexicon is a *view* of the mirror:
//!
//! 1. **Mirror.** An `ncbi:Gene` class anchored `⊑ wn:gene.n.01` (the genetics
//!    synset — cistron/factor/gene) *and* `⊑ lexicon:Entity`, then one **witness**
//!    per gene (`ncbigene:<GeneID> : ncbi:Gene`) carrying the `gene_info` facts
//!    (symbol, description, type, organism, synonyms, dbxrefs, location).
//! 2. **Lexicon (derived).** One `lexicon:Lexicon` (`lexicon:ncbi_gene`) and, per
//!    gene, a named-entity **NP** `lexicon:LexicalEntry` for the symbol and each
//!    synonym — `cat_np(ncbi:Gene, sg)`, `sem =` the witness, `sem_type = ncbi:Gene`,
//!    `in_lexicon = lexicon:ncbi_gene`. This mirrors the WordNet proper-noun
//!    archetype (`Einstein : person.n.01`); a gene symbol's symbol+synonyms are the
//!    same "many forms → one sem" shape as a WordNet synset's lemmas.
//!
//! Because individual genes are witnesses (not classes) of `ncbi:Gene ⊑ Entity`,
//! they flow into general predicate slots by subsumption ("WRN affects TP53"), and
//! the `⊑ wn:gene.n.01` anchor makes them genes in WordNet's sense ("WRN is a gene";
//! "every gene …" ranges over them) — the whole point of aligning rather than
//! minting a parallel concept.

use crate::gene_info::GeneInfo;

/// The WordNet 3.0 genetics-sense gene synset — `gene.n.01` (lemmas cistron /
/// factor / gene, "a unit of heredity"), offset `05444328`, which the
/// `eigenius-wordnet` importer emits as `wn:n05444328`. `ncbi:Gene` is anchored
/// here. **Curated, citable anchor** (not a string match); verify it resolves to
/// the genetics synset in the provisioned WordNet layer.
pub const ANCHOR_GENE_SYNSET: &str = "wn:n05444328";

/// The stable lexicon identity for this importer's output (D65 §3).
pub const NCBI_GENE_LEXICON: &str = "lexicon:ncbi_gene";

/// Document header: license/provenance notice + namespace declarations.
pub const ESL_HEADER: &str = "\
// ════════════════════════════════════════════════════════════════════
// DERIVED FROM NCBI Gene (Entrez). NCBI databases are U.S. Government
// works in the public domain; see https://www.ncbi.nlm.nih.gov/home/about/policies/
// Source schema: gene_info (https://ftp.ncbi.nih.gov/gene/DATA/README).
// ════════════════════════════════════════════════════════════════════
namespace core       = \"urn:eigenius:core\";
namespace reflection = \"urn:eigenius:reflection\";
namespace epistemic  = \"urn:eigenius:reflection:epistemic\";
namespace eigentt    = \"urn:eigenius:eigentt\";
namespace lexicon    = \"urn:eigenius:lexicon\";
namespace wn         = \"urn:eigenius:wn\";
namespace ncbi       = \"urn:eigenius:ncbi\";
namespace ncbigene   = \"urn:eigenius:ncbigene\";
";

/// The `ncbi:Gene` class declaration. Always `⊑ lexicon:Entity` (so gene witnesses
/// compose in general `Entity` predicate slots, and the base import validates
/// standalone). When `anchor_to_wordnet` is set, *also* `⊑ wn:gene.n.01`
/// ([`ANCHOR_GENE_SYNSET`]) — the deeper grounding, valid only when the WordNet
/// layer is in the chain (the validator rejects an unresolved `subclass_of`).
fn gene_class_decl(anchor_to_wordnet: bool) -> String {
    let parents = if anchor_to_wordnet {
        format!("{ANCHOR_GENE_SYNSET}, lexicon:Entity")
    } else {
        "lexicon:Entity".to_string()
    };
    format!(
        "// ── The gene class, rooted in the lexicon:Entity lattice (D65 §5) ──\n\
         class ncbi:Gene : {parents} {{\n\
         \x20   description = \"An NCBI Gene (Entrez) gene concept. Individual genes are witnesses (instances) of this class; rooted at lexicon:Entity so they compose in general predicate slots{anchor_note}.\";\n\
         }}\n",
        anchor_note = if anchor_to_wordnet {
            ", and anchored to WordNet gene.n.01 (the genetics sense: cistron/factor/gene) so they count as genes in WordNet's sense"
        } else {
            ""
        },
    )
}

/// The NCBI-specific property declarations (the gene class is separate — see
/// [`gene_class_decl`]). Reuses `core:short_name` / `core:description` for the
/// symbol + gloss; only genuinely-NCBI fields get new properties.
const PROPERTIES: &str = "\
property ncbi:gene_id : core:string {
    description = \"The Entrez GeneID — the stable NCBI gene identifier. The witness IRI is ncbigene:g<GeneID>; this preserves the raw numeric id (and grounds to ncbigene:<GeneID> / http://purl.uniprot.org/geneid/<GeneID>).\";
    domain ncbi:Gene;
}
property ncbi:gene_type : core:string {
    description = \"NCBI type_of_gene value (protein-coding, ncRNA, pseudo, tRNA, …).\";
    domain ncbi:Gene;
}
property ncbi:in_taxon : core:string {
    description = \"The NCBI Taxonomy id of the organism the gene belongs to — gene_info tax_id (e.g. 9606 = Homo sapiens). Kept as the raw id for now; to be promoted to a resource link into an NCBITaxon mirror when that import lands (a separate follow-on).\";
    domain ncbi:Gene;
}
property ncbi:locus_tag : core:string {
    description = \"The gene's LocusTag (gene_info column 4), when present.\";
    domain ncbi:Gene;
}
property ncbi:chromosome : core:string {
    description = \"Chromosome the gene is placed on (gene_info column 7).\";
    domain ncbi:Gene;
}
property ncbi:map_location : core:string {
    description = \"Cytogenetic map location (gene_info column 8).\";
    domain ncbi:Gene;
}
property ncbi:synonym : core:value_array {
    element_type = core:string;
    description = \"Alternate gene symbols (gene_info Synonyms) — the additional surface forms the derived lexicon entries cover.\";
    domain ncbi:Gene;
}
property ncbi:dbxref : core:value_array {
    element_type = core:string;
    description = \"Cross-database identifiers (gene_info dbXrefs): HGNC / Ensembl / MIM / ….\";
    domain ncbi:Gene;
}
";

/// Coverage of one import run.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    /// Gene witnesses emitted.
    pub genes: usize,
    /// `lexicon:LexicalEntry` entries emitted (symbol + synonyms, deduped per gene).
    pub entries: usize,
}

/// Escape a string for an ESL double-quoted literal.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

/// Render an ESL array literal of quoted strings: `[ "a", "b" ]`.
fn str_array(items: &[String]) -> String {
    let inner: Vec<String> = items.iter().map(|s| format!("\"{}\"", esc(s))).collect();
    format!("[ {} ]", inner.join(", "))
}

/// Emit one gene's witness resource (the mirror) into `buf`.
fn push_witness(buf: &mut String, g: &GeneInfo) {
    // Witness IRI: `ncbigene:g<GeneID>`. The `g` prefix keeps the local an
    // identifier (ESL forbids a pure-numeric `prefix:local`, like WordNet's
    // `n05444328`); the raw Entrez GeneID is preserved in `ncbi:gene_id`.
    buf.push_str(&format!(
        "resource ncbigene:g{id} : ncbi:Gene {{\n\
         \x20   core:short_name  = \"{sym}\";\n\
         \x20   core:description = \"{desc}\";\n\
         \x20   ncbi:gene_id     = \"{id}\";\n\
         \x20   ncbi:gene_type   = \"{ty}\";\n\
         \x20   ncbi:in_taxon    = \"{tax}\";\n",
        id = g.gene_id,
        sym = esc(&g.symbol),
        desc = esc(&g.description),
        ty = esc(&g.type_of_gene),
        tax = g.tax_id,
    ));
    if let Some(lt) = &g.locus_tag {
        buf.push_str(&format!("    ncbi:locus_tag   = \"{}\";\n", esc(lt)));
    }
    if let Some(ch) = &g.chromosome {
        buf.push_str(&format!("    ncbi:chromosome  = \"{}\";\n", esc(ch)));
    }
    if let Some(ml) = &g.map_location {
        buf.push_str(&format!("    ncbi:map_location = \"{}\";\n", esc(ml)));
    }
    if !g.synonyms.is_empty() {
        buf.push_str(&format!(
            "    ncbi:synonym     = {};\n",
            str_array(&g.synonyms)
        ));
    }
    if !g.dbxrefs.is_empty() {
        buf.push_str(&format!(
            "    ncbi:dbxref      = {};\n",
            str_array(&g.dbxrefs)
        ));
    }
    buf.push_str("}\n\n");
}

/// Emit the derived named-entity NP lexical entries for one gene — the symbol and
/// each distinct synonym, all `sem`-pointing at the gene's witness.
fn push_entries(buf: &mut String, g: &GeneInfo, rep: &mut Report) {
    // Symbol first, then synonyms; dedup (a synonym may repeat the symbol), keep order.
    let mut forms: Vec<&String> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for f in std::iter::once(&g.symbol).chain(g.synonyms.iter()) {
        if !f.is_empty() && seen.insert(f.as_str()) {
            forms.push(f);
        }
    }
    for (i, form) in forms.iter().enumerate() {
        buf.push_str(&format!(
            "resource ncbigene:e_{id}_{i} : lexicon:LexicalEntry {{\n\
             \x20   lexicon:form       = \"{form}\";\n\
             \x20   lexicon:cat        = type_expr( lexicon:cat_np(ncbi:Gene, lexicon:sg) );\n\
             \x20   lexicon:sem        = ncbigene:g{id};\n\
             \x20   lexicon:sem_type   = type_expr( ncbi:Gene );\n\
             \x20   lexicon:sense      = \"ncbigene:{id}\";\n\
             \x20   lexicon:grade      = epistemic:declared;\n\
             \x20   lexicon:in_lexicon = lexicon:ncbi_gene;\n\
             }}\n\n",
            id = g.gene_id,
            form = esc(form),
        ));
        rep.entries += 1;
    }
}

/// The `lexicon:ncbi_gene` descriptor (D65 §3) — the stable identity of this
/// domain lexicon. `tax_id` names the organism the import covers.
fn lexicon_descriptor(tax_id: &str) -> String {
    format!(
        "resource lexicon:ncbi_gene : lexicon:Lexicon {{\n\
         \x20   lexicon:source   = \"NCBI Gene (Entrez), gene_info; NCBITaxon:{tax}\";\n\
         \x20   lexicon:language = \"en\";\n\
         \x20   lexicon:domain   = \"biomedical\";\n\
         \x20   lexicon:license  = \"NCBI public domain (U.S. Government work)\";\n\
         }}\n\n",
        tax = tax_id,
    )
}

/// Render the full mirror + derived lexicon document for `genes`. `tax_id` labels
/// the lexicon descriptor (all `genes` are expected to share it — see
/// [`crate::gene_info::parse_document`]). `anchor_to_wordnet` adds the
/// `ncbi:Gene ⊑ wn:gene.n.01` grounding edge — set it only when committing on a
/// chain that contains the WordNet layer (else the validator rejects the
/// unresolved `subclass_of`); off ⇒ rooted at `lexicon:Entity` only, which
/// validates standalone and still composes.
pub fn render_document(
    genes: &[GeneInfo],
    tax_id: &str,
    anchor_to_wordnet: bool,
) -> (String, Report) {
    let mut rep = Report::default();
    let mut body = String::new();

    body.push_str(&gene_class_decl(anchor_to_wordnet));
    body.push_str(PROPERTIES);
    body.push('\n');
    body.push_str(&lexicon_descriptor(tax_id));

    // Sort by GeneID (numeric) for deterministic output.
    let mut sorted: Vec<&GeneInfo> = genes.iter().collect();
    sorted.sort_by_key(|g| g.gene_id.parse::<u64>().unwrap_or(u64::MAX));

    for g in sorted {
        push_witness(&mut body, g);
        push_entries(&mut body, g, &mut rep);
        rep.genes += 1;
    }

    (format!("{ESL_HEADER}\n{body}"), rep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gene_info::parse_line;

    const WRN: &str = "9606\t7486\tWRN\t-\tRECQ3|RECQL2\tHGNC:HGNC:12791|Ensembl:ENSG00000165392\t8\t8p12\tWerner syndrome RecQ like helicase\tprotein-coding\tWRN\tWerner syndrome RecQ like helicase\tO\t-\t20240101\t-";
    const TP53: &str = "9606\t7157\tTP53\t-\tP53|LFS1\tHGNC:HGNC:11998\t17\t17p13.1\ttumor protein p53\tprotein-coding\tTP53\ttumor protein p53\tO\t-\t20240101\t-";

    fn genes() -> Vec<crate::gene_info::GeneInfo> {
        vec![parse_line(WRN).unwrap(), parse_line(TP53).unwrap()]
    }

    #[test]
    fn class_rooting_depends_on_the_wordnet_anchor_flag() {
        // Base (anchor off): rooted at lexicon:Entity only — validates standalone.
        let (base, _) = render_document(&genes(), "9606", false);
        assert!(base.contains("class ncbi:Gene : lexicon:Entity {"));
        assert!(!base.contains("wn:n05444328"));
        // Anchored: also ⊑ wn:gene.n.01 (the deeper grounding, needs WordNet).
        let (anchored, _) = render_document(&genes(), "9606", true);
        assert!(anchored.contains("class ncbi:Gene : wn:n05444328, lexicon:Entity {"));
        // The lexicon descriptor is present exactly once either way.
        assert_eq!(
            base.matches("resource lexicon:ncbi_gene : lexicon:Lexicon")
                .count(),
            1
        );
    }

    #[test]
    fn emits_witness_and_np_entries_per_gene() {
        let (doc, rep) = render_document(&genes(), "9606", false);
        assert_eq!(rep.genes, 2);
        // WRN witness + its facts (IRI g-prefixed; raw GeneID preserved).
        assert!(doc.contains("resource ncbigene:g7486 : ncbi:Gene {"));
        assert!(doc.contains("core:short_name  = \"WRN\";"));
        assert!(doc.contains("ncbi:gene_id     = \"7486\";"));
        assert!(doc.contains("ncbi:in_taxon    = \"9606\";"));
        assert!(doc.contains("ncbi:synonym     = [ \"RECQ3\", \"RECQL2\" ];"));
        // NP entry for the symbol — cat_np over the gene class, sem = the witness.
        assert!(doc.contains("lexicon:form       = \"WRN\";"));
        assert!(doc
            .contains("lexicon:cat        = type_expr( lexicon:cat_np(ncbi:Gene, lexicon:sg) );"));
        assert!(doc.contains("lexicon:sem        = ncbigene:g7486;"));
        assert!(doc.contains("lexicon:in_lexicon = lexicon:ncbi_gene;"));
        // Symbol + 2 synonyms for WRN, symbol + 2 for TP53 = 6 entries.
        assert_eq!(rep.entries, 6);
        // Every entry is tagged into the ncbi_gene lexicon.
        assert_eq!(
            doc.matches(": lexicon:LexicalEntry {").count(),
            doc.matches("lexicon:in_lexicon = lexicon:ncbi_gene;")
                .count()
        );
    }
}
