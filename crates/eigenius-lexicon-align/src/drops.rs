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

//! **Verdicts → the drop set.** The other thing a `same=false` verdict can license, alongside a
//! merge: retracting a **junk atom** whose only contribution is a case-mangled collision with a
//! common word.
//!
//! The motivating case is `gENE` (UMLS `C5849123` = "Gross Extranodal Extension", an `NCI/SY` atom):
//! it lowercases to `gene`, so it seeds a second, wrong sense every time the word *gene* is parsed.
//! It is not the concept that is junk — "Gross Extranodal Extension" is a real finding — it is this
//! one **surface form**, an acronym written `gENE` that folds onto an unrelated common noun.
//!
//! Three gates, each doing distinct work (measured 2026-07-16, [`crate::candidates`] over MRCONSO):
//!
//! **1. The collision is the filter.** A drop is only ever considered for a [`Candidate`] — a UMLS
//! atom whose lowercased surface *is* a WordNet common-noun lemma. That clause (already computed by
//! candidate generation) is what makes case safe to use: `gENE`→`gene` collides, `alpha-Naphthylamine`
//! folds to nothing in WordNet and is never a candidate.
//!
//! **2. Case is the discriminator WITHIN the colliding set** ([`is_irregular_case`]). `TTY` cannot
//! separate junk from real — `gENE`, `DNA`, `MSH2`, `WRN` are all `SY`. The clean all-caps / CamelCase
//! atoms that legitimately collide (`CAT` the catalase gene, `SET` the oncogene) must be **kept**;
//! only the irregular casing (`gENE` — lowercase first letter, an uppercase later) marks a mangled
//! acronym. `TTY ∈ {SY, PEP}` is a conservative extra guard, never the discriminator.
//!
//! **3. Different-concept, at high confidence** ([`DROP_CONFIDENCE`]). We drop only when the
//! adjudicator was *confident these are different concepts* (`same=false`, conf ≥ threshold) — the
//! same trusted, gold-validated, recorded signal the merge path uses, in the opposite direction. A
//! surface that **merges** (the concept genuinely IS the WordNet class) is never dropped; the merge
//! redefines it instead. An unjudged or low-confidence pair is left alone — fail closed, prefer to
//! miss, exactly as [`crate::merge`] does.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::adjudicate::Verdict;
use crate::merge::MERGE_CONFIDENCE;
use crate::Candidate;

/// The confidence at or above which a `same=false` verdict may license a drop. Symmetric with
/// [`MERGE_CONFIDENCE`]: a low-confidence `same=false` is the adjudicator's "I cannot tell" default
/// (see the prompt), not evidence the atom is junk — so it does not drop.
pub const DROP_CONFIDENCE: f32 = 0.85;

/// Term types eligible for a drop. Never the preferred/abbreviation types (`PT`, `AB`, `ACR`, `MH`,
/// `PN`) — only loose synonyms (`SY`) and permuted entry points (`PEP`), where a source's
/// idiosyncratic mangled casing lives. A conservative guard, not the discriminator (that is case).
const DROP_TTYS: &[&str] = &["SY", "PEP"];

/// A mangled-acronym casing: **first character lowercase, some later character uppercase** (`gENE`).
///
/// This is the tight predicate, deliberately narrower than "any uppercase past position 0" (which
/// flags `RecQ`). It keeps:
/// - clean all-caps — `DNA`, `CAT`, `SET` (first char is *upper*),
/// - CamelCase gene symbols — `RecQ`, `MSH2` (first char is *upper*),
/// - ordinary words — `gene` (no later uppercase).
///
/// It flags biology's lower-prefix forms (`mRNA`, `siRNA`), but those fold to non-words (`mrna`) and
/// so are never [`Candidate`]s — the collision gate (rule 1) excludes them before case is consulted.
pub fn is_irregular_case(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() => chars.any(|c| c.is_ascii_uppercase()),
        _ => false,
    }
}

// ── The second drop path: metadata-artefact CONCEPTS ────────────────────────────────────────────
//
// The case path above retracts a junk SURFACE of a real concept (`gENE`). This path retracts a real
// surface of a junk CONCEPT: a UMLS entry that is not a lexical noun at all but an administrative
// code-table value or a SNOMED modifier, whose surface collides with a common word. `Specialty Type
// - cancer` (`C1547140`, an HL7 oncology-*specialty* code) seeds a spurious "cancer" reading against
// the disease; `Specific (qualifier value)` (`C0205369`) seeds a spurious noun against the adjective
// (the sense the 2026-07-20 reranker-gloss fix had to demote per sentence). The concept is
// identified from its UMLS preferred name — the same signal [`Candidate::umls_atoms`] documents as
// how a real merge is told from a metadata artefact — and the drop is gated on the SAME trusted
// evidence the case path uses (collision ⇒ WordNet covers the surface; a confident `same=false`
// verdict ⇒ not the WordNet sense; never a merged surface). Both sets are curated from the MRCONSO
// preferred-name distribution over the colliding, `same=false`-≥`DROP_CONFIDENCE` population
// (2026-07-20); neither pattern matches on any merged concept.

/// HL7 v2/v3 administrative code-table field names. A concept whose UMLS preferred name is
/// `<one of these> - <value>` is a code-table entry (`Specialty Type - cancer`, `Specimen Source
/// Codes - Bone`, `Act Class - act`), never a content concept: the `<value>` collides with a common
/// word but denotes the code, not the thing the word names.
const CODESYSTEM_PREFIXES: &[&str] = &[
    "specimen source codes",
    "specimen type",
    "specimen action code",
    "act class",
    "act code",
    "act priority",
    "role class",
    "role code",
    "kind of quantity",
    "specialty type",
    "what subject filter",
    "transaction counts and value totals",
    "transaction type",
    "message waiting priority",
    "quantity limited request",
    "query quantity unit",
    "value type",
    "organization unit type",
    "visit user code",
    "table cell vertical align",
    "table cell horizontal align",
    "authorization mode",
    "diagnostic service section id",
    "charge type",
    "parameterized data type",
    "degree of relationship",
    "mdf attribute type",
    "confidentiality",
    "processing id",
    "processing mode",
    "message structure",
    "administrative gender",
    "marital status",
    "patient class",
    "check digit scheme",
    "coordinate system data type",
    "amount type",
];

/// SNOMED CT metadata-hierarchy semantic tags. A concept whose UMLS preferred name ends
/// `(qualifier value)` / `(attribute)` / `(qualifier)` is a MODIFIER value in SNOMED's model, not an
/// entity — an adjective (`Specific`, `Common`, `Double`) reified as a code. Entity tags a real noun
/// carries (`(finding)`, `(procedure)`, `(substance)`, `(disorder)`, `(physical object)`) are
/// deliberately EXCLUDED: those mark genuine concepts, not scaffolding.
const METADATA_TAGS: &[&str] = &["qualifier value", "attribute", "qualifier"];

/// Semantic types that mark an **information / idea artefact** — a code, identifier, terminology,
/// database record, or software system, NOT a content entity. Combined with [`INFO_NAME_TOKENS`] this
/// catches the informational metadata the code-table PREFIX misses: `Protein Info` (`C1521746`, a
/// GenBank record), `Accession Number (identifier)`, `Acute - Triage Code`, `ARIA Oncology
/// Information System`. (The `cross-lexicon merge gap` turned out to be more of THIS, not unmerged
/// duplicates — the duplicates the adjudicator left are genuine distinct senses.)
const INFO_TUIS: &[&str] = &[
    "Intellectual Product",
    "Conceptual Entity",
    "Idea or Concept",
    "Classification",
];

/// Semantic types that are always a REAL content concept — never dropped even if the name looks
/// code-ish (`Alanine Transaminase` a.k.a. `ALT`, an Enzyme; a gene; a chemical). The safety floor.
const SUBSTANCE_TUIS: &[&str] = &[
    "Enzyme",
    "Amino Acid, Peptide, or Protein",
    "Organic Chemical",
    "Pharmacologic Substance",
    "Gene or Genome",
    "Nucleotide Sequence",
    "Biologically Active Substance",
    "Nucleic Acid, Nucleoside, or Nucleotide",
];

/// Name tokens marking an information/code artefact (word-boundary match).
const INFO_NAME_TOKENS: &[&str] = &[
    "code",
    "codes",
    "identifier",
    "info",
    "information",
    "terminology",
    "actclass",
    "nomenclature",
];

/// Is this concept an **informational metadata artefact** — an `INFO_TUIS` type whose name is a code /
/// identifier / info-record reification — as opposed to a real substance/gene (`SUBSTANCE_TUIS`, kept)?
/// Validated over the 2026-07-20 population: 193 concepts, all codes/specimen-codes/info-systems, none
/// a merged pair, none a genuine content concept (the substance floor excludes `ALT`).
fn is_informational_metadata(umls_name: &str, tuis: &[String]) -> bool {
    if tuis.iter().any(|t| SUBSTANCE_TUIS.contains(&t.as_str())) {
        return false;
    }
    if !tuis.iter().any(|t| INFO_TUIS.contains(&t.as_str())) {
        return false;
    }
    let name = umls_name.to_lowercase();
    name.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|tok| INFO_NAME_TOKENS.contains(&tok))
        || name.contains("value set")
        || name.contains("code set")
}

/// Is this concept a metadata artefact — an administrative code-table entry, a SNOMED modifier value,
/// or an information/code artefact — as read from its UMLS preferred name (+ semantic type)? Neither
/// pattern set matches a merged concept in the 2026-07-20 population.
fn is_metadata_concept(umls_name: &str, tuis: &[String]) -> bool {
    let name = umls_name.to_lowercase();
    if let Some((prefix, _value)) = name.split_once(" - ") {
        if CODESYSTEM_PREFIXES.contains(&prefix.trim()) {
            return true;
        }
    }
    if let Some(open) = name.rfind('(') {
        if let Some(inner) = name[open + 1..].strip_suffix(')') {
            if METADATA_TAGS.contains(&inner.trim()) {
                return true;
            }
        }
    }
    is_informational_metadata(umls_name, tuis)
}

/// The colliding atom form(s) to drop for a metadata concept: every atom whose lowercase IS the
/// candidate surface. The importer keys its per-surface entry by the verbatim `lexicon:form` and
/// matches a drop by exact casing, so we emit each casing present (whichever survived first-seen
/// dedup is thereby covered). Empty when no atom collides exactly (a lemma-only collision) → no drop,
/// fail closed.
fn metadata_atom_forms(c: &Candidate) -> Vec<String> {
    if !is_metadata_concept(&c.umls_name, &c.tuis) {
        return Vec::new();
    }
    let mut forms: Vec<String> = c
        .umls_atoms
        .iter()
        .filter_map(|a| a.split_once('|').map(|(_tty, s)| s))
        .filter(|s| s.to_lowercase() == c.surface)
        .map(|s| s.to_string())
        .collect();
    forms.sort();
    forms.dedup();
    forms
}

/// The original-case atom form colliding on `c.surface`, if that atom is an irregular-cased `SY`/`PEP`
/// atom — the drop's target. `None` when no such atom is present (the colliding atom is clean-cased,
/// a preferred type, or simply not among the concept's retained atoms → fail closed, no drop).
fn irregular_atom_for(c: &Candidate) -> Option<String> {
    for row in &c.umls_atoms {
        let Some((tty, str_)) = row.split_once('|') else {
            continue;
        };
        if DROP_TTYS.contains(&tty) && str_.to_lowercase() == c.surface && is_irregular_case(str_) {
            return Some(str_.to_string());
        }
    }
    None
}

/// One row of `drops.json` — the importer's input. A `(cui, form)` atom the importer skips emitting.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Drop {
    pub cui: String,
    /// The **original-case** atom string (`gENE`). The importer keys its per-surface entries by the
    /// verbatim `lexicon:form`, so the drop must carry the exact casing, not the lowercased surface.
    pub form: String,
    /// The lowercased common-noun surface it collides on (`gene`) — provenance, for inspection.
    pub surface: String,
    /// The adjudicator's `same=false` confidence that licensed the drop.
    pub confidence: f32,
}

/// What [`resolve_drops`] did, so the driver can print it instead of guessing.
#[derive(Debug, Default, PartialEq)]
pub struct DropStats {
    /// `(cui, surface)` collisions that qualified on case + confidence but were **not** dropped
    /// because the concept merges into a WordNet class on that surface (rule 3). The merge owns it.
    pub merged_not_dropped: usize,
}

/// Turn the adjudicator's verdicts into the drop set. Deterministic; no LLM. Mutually exclusive with
/// [`crate::merge::resolve`]: a surface that merges is never dropped.
pub fn resolve_drops(candidates: &[Candidate], verdicts: &[Verdict]) -> (Vec<Drop>, DropStats) {
    // Index the verdict by the CONCEPT PAIR, keeping the most confident (mirrors `merge::resolve`).
    let mut by_pair: BTreeMap<(&str, &str), &Verdict> = BTreeMap::new();
    for v in verdicts {
        let key = (v.cui.as_str(), v.offset.as_str());
        match by_pair.get(&key) {
            Some(prev) if prev.confidence >= v.confidence => {}
            _ => {
                by_pair.insert(key, v);
            }
        }
    }

    // Two drop paths, one gate. A `same=false`-≥`DROP_CONFIDENCE` witness licenses a drop of either
    // (1) an irregular-cased colliding atom — a junk SURFACE of a real concept (`gENE`), or (2) every
    // colliding atom of a metadata-artefact CONCEPT (`Specialty Type - cancer`). Keyed by `(cui,
    // form)` — a metadata concept can carry several colliding casings, each its own drop. `merged`
    // tracks `(cui, surface)` so a surface the concept merges on is never dropped (the merge owns it).
    let mut merged: BTreeSet<(String, String)> = BTreeSet::new();
    let mut cand: BTreeMap<(String, String), (String, f32)> = BTreeMap::new();

    for c in candidates {
        let Some(v) = by_pair.get(&(c.cui.as_str(), c.offset.as_str())) else {
            continue; // unjudged — fail closed
        };
        let surface = c.surface.to_lowercase();
        if v.same {
            if v.confidence >= MERGE_CONFIDENCE {
                merged.insert((c.cui.clone(), surface));
            }
            continue;
        }
        // same == false
        if v.confidence < DROP_CONFIDENCE {
            continue; // "cannot tell" default — not evidence of junk
        }
        let mut forms: Vec<String> = Vec::new();
        if let Some(f) = irregular_atom_for(c) {
            forms.push(f);
        }
        forms.extend(metadata_atom_forms(c));
        for form in forms {
            let key = (c.cui.clone(), form);
            match cand.get(&key) {
                Some((_, conf)) if *conf >= v.confidence => {}
                _ => {
                    cand.insert(key, (surface.clone(), v.confidence));
                }
            }
        }
    }

    let mut stats = DropStats::default();
    let mut out = Vec::new();
    for ((cui, form), (surface, confidence)) in cand {
        if merged.contains(&(cui.clone(), surface.clone())) {
            stats.merged_not_dropped += 1;
            continue; // the merge owns this surface — redefined, not dropped
        }
        out.push(Drop {
            cui,
            form,
            surface,
            confidence,
        });
    }
    out.sort_by(|a, b| (&a.cui, &a.form).cmp(&(&b.cui, &b.form)));
    (out, stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(surface: &str, cui: &str, offset: &str, atoms: &[&str]) -> Candidate {
        Candidate {
            surface: surface.into(),
            cui: cui.into(),
            offset: offset.into(),
            umls_atoms: atoms.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }
    /// A candidate carrying a UMLS preferred name — the metadata-path discriminator.
    fn cand_named(surface: &str, cui: &str, offset: &str, name: &str, atoms: &[&str]) -> Candidate {
        Candidate {
            umls_name: name.into(),
            ..cand(surface, cui, offset, atoms)
        }
    }
    fn verdict(cui: &str, offset: &str, surface: &str, same: bool, confidence: f32) -> Verdict {
        Verdict {
            cui: cui.into(),
            offset: offset.into(),
            surface: surface.into(),
            same,
            confidence,
            reason: String::new(),
        }
    }

    #[test]
    fn irregular_case_flags_mangled_acronyms_only() {
        assert!(is_irregular_case("gENE")); // the junk: lower-first, later upper
        assert!(is_irregular_case("mRNA")); // flagged, but folds to a non-word (never a candidate)
        assert!(!is_irregular_case("DNA")); // clean all-caps
        assert!(!is_irregular_case("CAT")); // legit all-caps gene symbol that collides with "cat"
        assert!(!is_irregular_case("RecQ")); // CamelCase — first char upper
        assert!(!is_irregular_case("MSH2")); // CamelCase + digit — first char upper
        assert!(!is_irregular_case("gene")); // ordinary word — no later uppercase
        assert!(!is_irregular_case(""));
    }

    /// The motivating case, with its real MRCONSO atoms and recorded verdict (`same=false`, 0.99).
    #[test]
    fn the_gene_junk_atom_is_dropped() {
        let cands = [cand(
            "gene",
            "C5849123",
            "05436752",
            &[
                "PT|Gross Extranodal Extension",
                "PN|Gross Extranodal Extension",
                "SY|gENE",
            ],
        )];
        let verdicts = [verdict("C5849123", "05436752", "gene", false, 0.99)];
        let (drops, stats) = resolve_drops(&cands, &verdicts);
        assert_eq!(drops.len(), 1);
        assert_eq!(drops[0].cui, "C5849123");
        assert_eq!(drops[0].form, "gENE"); // ORIGINAL case — what the importer keys on
        assert_eq!(drops[0].surface, "gene");
        assert_eq!(stats.merged_not_dropped, 0);
    }

    /// A clean all-caps atom that legitimately collides (`CAT` the catalase gene vs the animal) is
    /// KEPT — case is what separates it from `gENE`, and `TTY=SY` is shared by both.
    #[test]
    fn a_clean_cased_colliding_atom_is_not_dropped() {
        let cands = [cand(
            "cat",
            "C1412461",
            "02121808",
            &["SY|CAT", "PT|CAT Gene"],
        )];
        let verdicts = [verdict("C1412461", "02121808", "cat", false, 0.98)];
        let (drops, _) = resolve_drops(&cands, &verdicts);
        assert!(drops.is_empty());
    }

    /// A surface that MERGES (the concept genuinely is the WordNet class) is never dropped, even if
    /// some other synset's verdict is a confident `same=false` with an irregular atom.
    #[test]
    fn a_merged_surface_is_never_dropped() {
        let cands = [
            cand("gene", "C0017337", "05436752", &["SY|gene", "PT|Gene"]),
            cand("gene", "C0017337", "99999999", &["SY|gene", "PT|Gene"]),
        ];
        let verdicts = [
            verdict("C0017337", "05436752", "gene", true, 0.97), // merges here
            verdict("C0017337", "99999999", "gene", false, 0.9),
        ];
        // (no irregular atom anyway, but assert the merge-exclusion path holds)
        let (drops, _) = resolve_drops(&cands, &verdicts);
        assert!(drops.is_empty());
    }

    /// A confident `same=false` on an irregular atom is dropped, UNLESS the same surface also merges.
    #[test]
    fn merge_beats_drop_on_the_same_surface() {
        let cands = [
            cand("gene", "C5849123", "05436752", &["SY|gENE"]),
            cand("gene", "C5849123", "05436752", &["SY|gENE"]), // second synset row
        ];
        // First: a merge verdict; second: a drop-eligible verdict. Merge must win.
        let mut cands2 = cands.to_vec();
        cands2[1].offset = "11111111".into();
        let verdicts = [
            verdict("C5849123", "05436752", "gene", true, 0.95),
            verdict("C5849123", "11111111", "gene", false, 0.99),
        ];
        let (drops, stats) = resolve_drops(&cands2, &verdicts);
        assert!(drops.is_empty());
        assert_eq!(stats.merged_not_dropped, 1);
    }

    /// Low-confidence `same=false` is the adjudicator's "cannot tell" default — not a drop.
    #[test]
    fn low_confidence_different_does_not_drop() {
        let cands = [cand("gene", "C5849123", "05436752", &["SY|gENE"])];
        let verdicts = [verdict("C5849123", "05436752", "gene", false, 0.5)];
        let (drops, _) = resolve_drops(&cands, &verdicts);
        assert!(drops.is_empty());
    }

    /// An unjudged candidate is left alone (fail closed), never dropped.
    #[test]
    fn an_unjudged_candidate_is_not_dropped() {
        let cands = [cand("gene", "C5849123", "05436752", &["SY|gENE"])];
        let (drops, _) = resolve_drops(&cands, &[]);
        assert!(drops.is_empty());
    }

    // ── The metadata-concept path ───────────────────────────────────────────────────────────────

    #[test]
    fn metadata_name_patterns_flag_scaffolding_only() {
        let no_tui: &[String] = &[];
        // Administrative code-table entries.
        assert!(is_metadata_concept("Specialty Type - cancer", no_tui));
        assert!(is_metadata_concept("Specimen Source Codes - Bone", no_tui));
        assert!(is_metadata_concept("Act Class - act", no_tui));
        // SNOMED modifier tags.
        assert!(is_metadata_concept("Specific (qualifier value)", no_tui));
        assert!(is_metadata_concept("Adherence (attribute)", no_tui));
        assert!(is_metadata_concept(
            "Arbitrary (property) (qualifier value)",
            no_tui
        )); // trailing tag wins
            // NOT scaffolding: a real dashed concept whose prefix is not a code-system, and real SNOMED
            // entity tags a genuine noun carries.
        assert!(!is_metadata_concept(
            "Blood - brain barrier anatomy",
            no_tui
        ));
        assert!(!is_metadata_concept("Beans - dietary", no_tui));
        assert!(!is_metadata_concept("Impaired health (finding)", no_tui));
        assert!(!is_metadata_concept("Biopsy (procedure)", no_tui));
        assert!(!is_metadata_concept("4-aminobenzoic acid", no_tui));
    }

    #[test]
    fn informational_metadata_dropped_but_substances_kept() {
        let ip = &["Intellectual Product".to_string()];
        let ce = &["Conceptual Entity".to_string()];
        // Information / code artefacts (an INFO_TUIS type + a code/info/identifier name).
        assert!(is_informational_metadata("Protein Info", ce)); // C1521746, the GenBank record
        assert!(is_informational_metadata("Acute - Triage Code", ip));
        assert!(is_informational_metadata(
            "Accession Number (identifier)",
            ip
        ));
        assert!(is_informational_metadata(
            "ARIA Oncology Information System",
            ip
        ));
        assert!(is_informational_metadata("Basophil Specimen Code", ip));
        // Substance floor: never drop a real enzyme/gene/chemical, even if the name looks code-ish.
        let enzyme = &["Enzyme".to_string()];
        assert!(!is_informational_metadata("Alanine Transaminase", enzyme)); // ALT
                                                                             // Needs BOTH an info TUI and an info name — a plain content concept is not caught.
        assert!(!is_informational_metadata("Protein", ce)); // no info token
        assert!(!is_informational_metadata(
            "Triage Code",
            &["Finding".to_string()]
        )); // not an info TUI
    }

    /// The WRN-page culprit: `Specialty Type - cancer` (`C1547140`, an HL7 oncology-specialty code)
    /// seeds a spurious "cancer" reading against the disease. Its colliding atom `Cancer` is dropped.
    #[test]
    fn a_metadata_codesystem_concept_is_dropped() {
        let cands = [cand_named(
            "cancer",
            "C1547140",
            "14239918",
            "Specialty Type - cancer",
            &["PT|Cancer", "PN|Specialty Type - cancer"],
        )];
        let verdicts = [verdict("C1547140", "14239918", "cancer", false, 0.98)];
        let (drops, _) = resolve_drops(&cands, &verdicts);
        assert_eq!(drops.len(), 1);
        assert_eq!(drops[0].cui, "C1547140");
        assert_eq!(drops[0].form, "Cancer"); // the colliding atom, original case
        assert_eq!(drops[0].surface, "cancer");
    }

    /// A SNOMED `(qualifier value)` concept — an adjective reified as a code — is dropped, the sense
    /// the reranker-gloss fix had to demote per sentence. The WordNet noun/adjective survive it.
    #[test]
    fn a_metadata_qualifier_value_concept_is_dropped() {
        let cands = [cand_named(
            "specific",
            "C0205369",
            "00003553",
            "Specific (qualifier value)",
            &["PT|Specific"],
        )];
        let verdicts = [verdict("C0205369", "00003553", "specific", false, 0.9)];
        let (drops, _) = resolve_drops(&cands, &verdicts);
        assert_eq!(drops.len(), 1);
        assert_eq!(drops[0].form, "Specific");
    }

    /// A concept whose UMLS name merely CONTAINS a spaced dash but whose prefix is not a code-system
    /// (`Blood - brain barrier anatomy`) is a real concept, never a metadata drop.
    #[test]
    fn a_real_dashed_concept_is_not_dropped() {
        let cands = [cand_named(
            "blood",
            "C0005902",
            "05405324",
            "Blood - brain barrier anatomy",
            &["PT|Blood"],
        )];
        let verdicts = [verdict("C0005902", "05405324", "blood", false, 0.95)];
        let (drops, _) = resolve_drops(&cands, &verdicts);
        assert!(drops.is_empty());
    }

    /// Merge precedence holds for the metadata path too: a metadata-named concept that the
    /// adjudicator MERGED on the surface is redefined, not dropped.
    #[test]
    fn a_merged_metadata_concept_is_not_dropped() {
        let cands = [cand_named(
            "emergency",
            "C0175673",
            "07357388",
            "Emergency (qualifier value)",
            &["PT|Emergency"],
        )];
        let verdicts = [verdict("C0175673", "07357388", "emergency", true, 0.95)];
        let (drops, stats) = resolve_drops(&cands, &verdicts);
        assert!(drops.is_empty());
        assert_eq!(stats.merged_not_dropped, 0); // never even a drop candidate
    }
}
