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

//! Render a joined UMLS [`Subset`] into an Eigon/ESL document: a faithful **typed
//! mirror** plus a **derived domain lexicon** (D65 §5).
//!
//! Three parts, one document, layered so the lexicon is a *view* of the mirror:
//!
//! 1. **Semantic-type classes.** Each used TUI → `class umlssty:<TUI> : lexicon:Entity`
//!    (the semantic network, flattened at the `Entity` top for v1 — the TUI ISA
//!    hierarchy is a follow-on). The TUI is the IRI local; the name is the description.
//! 2. **Concept classes.** Each CUI → `class umlscui:<CUI> : <its TUI classes>` — the
//!    `subclass_of` edges ARE the semantic typing (queryable structurally), reaching
//!    `lexicon:Entity` transitively. The CUI is the IRI local; the definition is the
//!    description. This parallels WordNet's common-noun synset (offset = IRI local,
//!    hypernym = subclass_of, gloss = description).
//! 3. **Lexicon (derived).** One `lexicon:Lexicon` (`lexicon:umls`) and, per concept,
//!    a common-noun **N** `lexicon:LexicalEntry` per English surface string —
//!    `cat_n(umlscui:<CUI>, num_any)`, `sem =` the concept class, `sem_type = Set`,
//!    `in_lexicon = lexicon:umls`.
//!
//! Because a concept is a *class* under `lexicon:Entity`, it is used as a **kind**
//! (the WordNet common-noun path) — a determiner quantifies it ("every Werner
//! syndrome …"), and it flows into general predicate slots by subsumption.

use crate::rrf::Subset;

/// The stable lexicon identity for this importer's output (D65 §3).
pub const UMLS_LEXICON: &str = "lexicon:umls";

/// Document header: the **UMLS license notice** (load-bearing — the redistribution
/// constraint flows to every downstream user) + namespace declarations. `{version}`
/// is the Metathesaurus release the import was built from.
fn esl_header(version: &str) -> String {
    format!(
        "\
// ════════════════════════════════════════════════════════════════════
// DERIVED FROM the UMLS Metathesaurus (U.S. National Library of Medicine),
// release {version}. This is a DERIVATIVE WORK governed by the UMLS
// Metathesaurus License Agreement:
//   https://uts.nlm.nih.gov/uts/assets/LicenseAgreement.pdf
//
// Redistribution of this artifact does NOT grant a UMLS license. Each
// downstream user MUST obtain their own UMLS license from the NLM
// (https://uts.nlm.nih.gov/uts/) before use.
//
// Only SRL-0 (Level 0 / Category 0) sources are included; sources with a
// higher Source Restriction Level (e.g. SNOMED CT, CPT) are EXCLUDED.
// ════════════════════════════════════════════════════════════════════
namespace core       = \"urn:eigenius:core\";
namespace reflection = \"urn:eigenius:reflection\";
namespace epistemic  = \"urn:eigenius:reflection:epistemic\";
namespace eigentt    = \"urn:eigenius:eigentt\";
namespace lexicon    = \"urn:eigenius:lexicon\";
namespace umlssty    = \"urn:eigenius:umlssty\";
namespace umlscui    = \"urn:eigenius:umlscui\";
"
    )
}

/// Coverage of one import run.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    /// `umls:SemanticType` classes emitted (one per used TUI).
    pub semantic_types: usize,
    /// `umls:Concept` classes emitted (one per CUI).
    pub concepts: usize,
    /// `lexicon:LexicalEntry` entries emitted (one per concept surface form).
    pub entries: usize,
    /// Additive `cat_n(C, mass)` entries emitted for mass concepts (RC-1 head-inheritance mass shim).
    pub mass_entries: usize,
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

/// Emit one semantic-type class (`umlssty:<TUI> : lexicon:Entity`).
fn push_semantic_type(buf: &mut String, tui: &str, name: &str) {
    buf.push_str(&format!(
        "class umlssty:{tui} : lexicon:Entity {{\n\
         \x20   description = \"UMLS Semantic Type {tui} — {name}.\";\n\
         }}\n\n",
        name = esc(name),
    ));
}

/// Emit one concept's chain node. A **concept class** (`class umlscui:<CUI> : <TUI classes>`) by
/// default; a **named individual** (`is_individual`, D62 — `docs/notes/d62-named-individual-typing.md`)
/// is instead an **instance** (`resource umlscui:<CUI> : <TUI classes>`) of its semantic-type
/// class(es), so a `cat_np` entry can name it.
fn push_concept(buf: &mut String, cui: &str, tuis: &[String], desc: &str, is_individual: bool) {
    let parents: Vec<String> = tuis.iter().map(|t| format!("umlssty:{t}")).collect();
    // A `class` body takes the bare `description` class-item keyword; a `resource` (instance) body
    // takes the qualified `core:description` property.
    let (keyword, desc_prop) = if is_individual {
        ("resource", "core:description")
    } else {
        ("class", "description")
    };
    buf.push_str(&format!(
        "{keyword} umlscui:{cui} : {} {{\n\
         \x20   {desc_prop} = \"{}\";\n\
         }}\n\n",
        parents.join(", "),
        esc(desc),
    ));
}

/// The concept's `description` text: the preferred name, the definition (with its
/// source) when present, and the CUI for provenance.
fn concept_description(
    preferred: &str,
    definition: Option<&(String, String)>,
    cui: &str,
) -> String {
    match definition {
        Some((sab, def)) => format!("{preferred} — {def} [{sab}] UMLS CUI {cui}."),
        None => format!("{preferred}. UMLS CUI {cui}."),
    }
}

/// The mass/uncountable-noun lookup set (lowercased lemmas) — the **shared** `--countability` lexicon
/// (Wiktionary `Category:English uncountable nouns` ∩ WordNet, `scripts/provision-countability.sh`) the
/// WordNet importer also consumes. Empty ⇒ no mass shim (count-only, the prior behaviour).
pub type MassNouns = std::collections::BTreeSet<String>;

/// **RC-1 mass shim (head-inheritance, D63 parse-gap-closure §4 Step 4).** A concept is mass iff its
/// preferred name's HEAD (last word) is an uncountable noun — "microsatellite instability" is mass because
/// its head "instability" is, so the abbreviation "MSI" inherits the mass reading. Lets a bare occurrence
/// (`MSI contributes to cancers`) shift to a mass NP subject/argument, exactly as bare "instability" does;
/// a bare *singular count* noun has no such reading (`MSI` / `gene` bare → grammar-gap). General — the
/// head governs compound countability — replacing the removed 5-acronym `MASS_FORMS` hardcode.
fn concept_is_mass(preferred_name: &str, mass: &MassNouns) -> bool {
    preferred_name
        .split_whitespace()
        .last()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .map(|head| mass.contains(&head))
        .unwrap_or(false)
}

/// Emit one `lexicon:LexicalEntry` for `form` under `cat` / `sem_type`, IRI `umlscui:e_{cui}_{i}{suffix}`.
fn emit_entry(
    buf: &mut String,
    cui: &str,
    i: usize,
    suffix: &str,
    form: &str,
    cat: &str,
    sem_type: &str,
) {
    buf.push_str(&format!(
        "resource umlscui:e_{cui}_{i}{suffix} : lexicon:LexicalEntry {{\n\
         \x20   lexicon:form       = \"{form}\";\n\
         \x20   lexicon:cat        = type_expr( {cat} );\n\
         \x20   lexicon:sem        = umlscui:{cui};\n\
         \x20   lexicon:sem_type   = type_expr( {sem_type} );\n\
         \x20   lexicon:sense      = \"umls:{cui}\";\n\
         \x20   lexicon:grade      = epistemic:declared;\n\
         \x20   lexicon:in_lexicon = lexicon:umls;\n\
         }}\n\n",
        form = esc(form),
    ));
}

// A concept class yields common-noun `cat_n(C, num_any)` entries (`sem_type = Set`); a mass concept
// (head-inheritance, [`concept_is_mass`]) ALSO gets an additive `cat_n(C, mass)` reading per form, so a
// bare occurrence shifts to a mass NP (the RC-1 fix — bare `MSI` was a grammar-gap because UMLS typed it
// count-only). A named individual (gene symbol) yields proper-noun `cat_np(TUI, sg)` entries and is never
// mass-shimmed. `DNA`/`RNA` etc. that are English nouns get their mass reading here too (same lexicon the
// WordNet importer uses), a harmless duplicate of any WordNet mass entry (distinct entry IRIs).
fn push_entries(
    buf: &mut String,
    cui: &str,
    forms: &[String],
    named_tui: Option<&str>,
    is_mass: bool,
    rep: &mut Report,
) {
    let (cat, sem_type) = match named_tui {
        Some(tui) => (
            format!("lexicon:cat_np(umlssty:{tui}, lexicon:sg)"),
            format!("umlssty:{tui}"),
        ),
        None => (
            format!("lexicon:cat_n(umlscui:{cui}, lexicon:num_any)"),
            "Set".to_string(),
        ),
    };
    let mass_cat = format!("lexicon:cat_n(umlscui:{cui}, lexicon:mass)");
    for (i, form) in forms.iter().enumerate() {
        emit_entry(buf, cui, i, "", form, &cat, &sem_type);
        rep.entries += 1;
        if is_mass && named_tui.is_none() {
            emit_entry(buf, cui, i, "_mass", form, &mass_cat, "Set");
            rep.entries += 1;
            rep.mass_entries += 1;
        }
    }
}

/// The `lexicon:umls` descriptor (D65 §3) — the stable identity of this domain lexicon.
fn lexicon_descriptor(version: &str) -> String {
    format!(
        "resource lexicon:umls : lexicon:Lexicon {{\n\
         \x20   lexicon:source   = \"UMLS Metathesaurus {version} — Level 0 / SRL-0 sources only\";\n\
         \x20   lexicon:version  = \"{version}\";\n\
         \x20   lexicon:language = \"en\";\n\
         \x20   lexicon:domain   = \"biomedical\";\n\
         \x20   lexicon:license  = \"UMLS Metathesaurus License (NLM). This is a derivative work; redistribution requires each recipient to hold their own UMLS license — https://uts.nlm.nih.gov/uts/\";\n\
         }}\n\n",
    )
}

/// The document header (license notice + namespace declarations). Public so a
/// partitioned emit can prepend it to every chunk file — each chunk must carry the
/// UMLS license notice and the namespaces it references.
pub fn header(version: &str) -> String {
    esl_header(version)
}

/// Render the **base layer**: the semantic-type classes (`umlssty:*`) + the
/// `lexicon:umls` descriptor. In a partitioned import this is layer 0; every concept
/// chunk resolves its `subclass_of umlssty:*` and `in_lexicon lexicon:umls` against it.
/// Returns the document (header + base) and the count of semantic-type classes.
pub fn render_base(subset: &Subset, version: &str) -> (String, usize) {
    let mut body = String::from(
        "// ── Semantic-type classes (the UMLS semantic network, flat at Entity) ──\n",
    );
    for st in &subset.semantic_types {
        push_semantic_type(&mut body, &st.tui, &st.name);
    }
    body.push_str(&lexicon_descriptor(version));
    (
        format!("{}\n{body}", esl_header(version)),
        subset.semantic_types.len(),
    )
}

/// Render one concept's block — its class (the mirror) plus its derived common-noun
/// entries. No header; callers concatenate blocks into chunk bodies. Returns the
/// rendered text and the block's [`Report`] (lexical-entry + additive mass-entry counts).
pub fn render_concept_block(c: &crate::rrf::Concept, mass: &MassNouns) -> (String, Report) {
    let mut buf = String::new();
    let mut rep = Report::default();
    let desc = concept_description(&c.preferred_name, c.definition.as_ref(), &c.cui);
    // A named individual (a nomenclature symbol, e.g. an HGNC gene) is emitted as an INSTANCE of its
    // primary semantic-type class with `cat_np` entries; otherwise a concept class with `cat_n`.
    let named_tui: Option<&str> = c.symbol.as_ref().and(c.tuis.first()).map(|t| t.as_str());
    // RC-1 mass shim: a concept whose preferred-name head is uncountable gets an additive `mass` entry.
    let is_mass = concept_is_mass(&c.preferred_name, mass);
    push_concept(&mut buf, &c.cui, &c.tuis, &desc, named_tui.is_some());
    push_entries(&mut buf, &c.cui, &c.forms, named_tui, is_mass, &mut rep);
    (buf, rep)
}

/// Render the full mirror + derived lexicon as a SINGLE document for `subset`.
/// `version` labels the header notice and the lexicon descriptor (e.g. `"2026AA"`).
/// For large imports use the partitioned emit (the binary's `--out-dir`) instead, so
/// each layer stays under the gRPC message-size limit.
pub fn render_document(subset: &Subset, version: &str, mass: &MassNouns) -> (String, Report) {
    let mut rep = Report::default();
    let (base, sty) = render_base(subset, version);
    rep.semantic_types = sty;

    let mut body = base;
    body.push_str("\n// ── Concept classes (the mirror) + derived common-noun entries ──\n");
    for c in &subset.concepts {
        let (block, brep) = render_concept_block(c, mass);
        body.push_str(&block);
        rep.entries += brep.entries;
        rep.mass_entries += brep.mass_entries;
        rep.concepts += 1;
    }

    (body, rep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rrf::{Concept, SemanticType};

    fn werner_subset() -> Subset {
        Subset {
            semantic_types: vec![SemanticType {
                tui: "T047".to_string(),
                name: "Disease or Syndrome".to_string(),
            }],
            concepts: vec![Concept {
                cui: "C0043119".to_string(),
                tuis: vec!["T047".to_string()],
                preferred_name: "Werner Syndrome".to_string(),
                forms: vec![
                    "Werner Syndrome".to_string(),
                    "Werner's Syndrome".to_string(),
                ],
                definition: Some((
                    "MSH".to_string(),
                    "An autosomal recessive disorder.".to_string(),
                )),
                symbol: None, // a disease concept → stays a class (cat_n)
            }],
        }
    }

    /// The WRN **gene** (HGNC) — a NAMED INDIVIDUAL: `symbol = Some("WRN")`, TUI T028.
    fn wrn_gene_subset() -> Subset {
        Subset {
            semantic_types: vec![SemanticType {
                tui: "T028".to_string(),
                name: "Gene or Genome".to_string(),
            }],
            concepts: vec![Concept {
                cui: "C1337007".to_string(),
                tuis: vec!["T028".to_string()],
                preferred_name: "WRN".to_string(),
                forms: vec![
                    "WRN".to_string(),
                    "Werner syndrome RecQ like helicase".to_string(),
                ],
                definition: None,
                symbol: Some("WRN".to_string()),
            }],
        }
    }

    #[test]
    fn renders_mirror_and_lexicon() {
        let (doc, rep) = render_document(&werner_subset(), "2026AA", &MassNouns::new());
        assert_eq!(rep.semantic_types, 1);
        assert_eq!(rep.concepts, 1);
        assert_eq!(rep.entries, 2);

        // Semantic-type class, rooted at Entity.
        assert!(doc.contains("class umlssty:T047 : lexicon:Entity {"));
        // Concept class subclassed under its semantic type (typing IS the edge).
        assert!(doc.contains("class umlscui:C0043119 : umlssty:T047 {"));
        // Definition + CUI folded into the description.
        assert!(doc.contains(
            "Werner Syndrome — An autosomal recessive disorder. [MSH] UMLS CUI C0043119."
        ));
        // Common-noun (cat_n) entry, sem = the concept class, sem_type = Set.
        assert!(doc.contains("lexicon:form       = \"Werner Syndrome\";"));
        assert!(doc.contains(
            "lexicon:cat        = type_expr( lexicon:cat_n(umlscui:C0043119, lexicon:num_any) );"
        ));
        assert!(doc.contains("lexicon:sem        = umlscui:C0043119;"));
        assert!(doc.contains("lexicon:sem_type   = type_expr( Set );"));
        assert!(doc.contains("lexicon:in_lexicon = lexicon:umls;"));

        // The lexicon descriptor appears exactly once.
        assert_eq!(
            doc.matches("resource lexicon:umls : lexicon:Lexicon")
                .count(),
            1
        );
        // Every LexicalEntry carries the lexicon tag.
        assert_eq!(
            doc.matches(": lexicon:LexicalEntry {").count(),
            doc.matches("lexicon:in_lexicon = lexicon:umls;").count()
        );
    }

    #[test]
    fn named_individual_gene_renders_as_instance_with_cat_np_entries() {
        // A gene (HGNC symbol → named individual, D62) is an INSTANCE of its semantic-type class
        // with PROPER-NOUN (cat_np) entries — so it works as both a bare NP and a prenominal modifier.
        let (doc, _) = render_document(&wrn_gene_subset(), "2026AA", &MassNouns::new());
        // The CUI is a `resource` (instance), NOT a `class`, typed by its semantic type.
        assert!(doc.contains("resource umlscui:C1337007 : umlssty:T028 {"));
        assert!(!doc.contains("class umlscui:C1337007"));
        // Proper-noun (cat_np) entry over the semantic-type class, sem = the instance, sg.
        assert!(doc.contains("lexicon:form       = \"WRN\";"));
        assert!(doc.contains(
            "lexicon:cat        = type_expr( lexicon:cat_np(umlssty:T028, lexicon:sg) );"
        ));
        assert!(doc.contains("lexicon:sem        = umlscui:C1337007;"));
        assert!(doc.contains("lexicon:sem_type   = type_expr( umlssty:T028 );"));
        // No common-noun (cat_n) entry for a named individual.
        assert!(!doc.contains("lexicon:cat_n(umlscui:C1337007"));
        // Every form is a name of the individual (both cat_np).
        assert!(doc.contains("lexicon:form       = \"Werner syndrome RecQ like helicase\";"));
    }

    #[test]
    fn header_carries_the_umls_license_and_redistribution_constraint() {
        let (doc, _) = render_document(&werner_subset(), "2026AA", &MassNouns::new());
        assert!(doc.contains("UMLS Metathesaurus"));
        assert!(doc.contains("MUST obtain their own UMLS license"));
        assert!(doc.contains("SRL-0"));
        assert!(doc.contains("2026AA"));
    }

    /// An MSI-like concept: preferred head `instability` is uncountable → the concept + its abbreviation
    /// `MSI` are mass (C0920269, "Microsatellite Instability", a T047 phenomenon, no nomenclature symbol).
    fn msi_subset() -> Subset {
        Subset {
            semantic_types: vec![SemanticType {
                tui: "T047".to_string(),
                name: "Disease or Syndrome".to_string(),
            }],
            concepts: vec![Concept {
                cui: "C0920269".to_string(),
                tuis: vec!["T047".to_string()],
                preferred_name: "Microsatellite Instability".to_string(),
                forms: vec!["MSI".to_string(), "Microsatellite Instability".to_string()],
                definition: None,
                symbol: None,
            }],
        }
    }

    #[test]
    fn mass_concept_gets_additive_mass_entry_by_head_inheritance() {
        // RC-1 (§4 Step 4): head `instability` ∈ the countability set ⇒ every form gets an ADDITIVE
        // `cat_n(C, mass)` entry alongside its `num_any` one — so bare `MSI` shifts to a mass subject.
        let mut mass = MassNouns::new();
        mass.insert("instability".to_string());
        let (doc, rep) = render_document(&msi_subset(), "2026AA", &mass);
        assert_eq!(rep.entries, 4, "2 forms × (count + mass)");
        assert_eq!(rep.mass_entries, 2);
        assert!(doc.contains("lexicon:cat_n(umlscui:C0920269, lexicon:num_any)"));
        assert!(doc.contains("lexicon:cat_n(umlscui:C0920269, lexicon:mass)"));
        // The abbreviation `MSI` (form 0) itself carries the mass reading — the fix's whole point.
        assert!(doc.contains("resource umlscui:e_C0920269_0_mass : lexicon:LexicalEntry {"));
        assert!(doc.contains("lexicon:form       = \"MSI\";"));
    }

    #[test]
    fn count_head_concept_gets_no_mass_entry() {
        // `Werner Syndrome` head `syndrome` is countable ⇒ no mass entry, even with a populated set.
        let mut mass = MassNouns::new();
        mass.insert("instability".to_string()); // present, but `syndrome` is not
        let (doc, rep) = render_document(&werner_subset(), "2026AA", &mass);
        assert_eq!(rep.mass_entries, 0);
        assert!(!doc.contains("lexicon:mass"));
    }
}
