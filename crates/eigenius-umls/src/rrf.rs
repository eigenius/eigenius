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

//! Parsers for the UMLS Rich Release Format (RRF) `|`-delimited files, plus the
//! multi-file join into the subset model the [`crate::convert`] renderer consumes.
//!
//! Files used (column specs verified against the UMLS Reference Manual / `MRFILES`
//! and the 2026AA data; only the fields the mirror keeps are extracted):
//!
//! - **MRCONSO** — atoms (one surface form per source/term-type): `CUI|LAT|TS|LUI|
//!   STT|SUI|ISPREF|AUI|SAUI|SCUI|SDUI|SAB|TTY|CODE|STR|SRL|SUPPRESS|CVF`.
//! - **MRSTY** — semantic types: `CUI|TUI|STN|STY|ATUI|CVF`.
//! - **MRSAB** — source metadata: the per-source **SRL** (col 14, the Source
//!   Restriction Level — `0` = redistribution-clean Level 0) keyed by RSAB (col 4).
//! - **MRRANK** — `RANK|SAB|TTY|SUPPRESS`; the precedence used to pick a concept's
//!   canonical name (higher RANK ⇒ more preferred).
//! - **MRDEF** — definitions: `CUI|AUI|ATUI|SATUI|SAB|DEF|SUPPRESS|CVF`.
//!
//! Every per-line parser is fail-soft (a malformed/short row → `None`, never panic),
//! so the importer can stream the multi-GB files and report the kept counts. The
//! [`ConceptBuilder`] accumulator joins the streams (feed STYs first, then atoms and
//! defs) and is shared by the streaming binary and the in-memory tests alike.

use std::collections::{BTreeMap, BTreeSet};

// ── Per-line record parsers ───────────────────────────────────────────

/// One MRCONSO atom — a surface form contributed by a source vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Atom {
    /// CUI — the concept this atom belongs to.
    pub cui: String,
    /// LAT — language of the term (`ENG`, `SPA`, …).
    pub lat: String,
    /// TS — term status (`P` = preferred LUI of the concept, `S` = synonym).
    pub ts: String,
    /// ISPREF — `Y` if this is the preferred atom of its string class.
    pub ispref: String,
    /// SAB — the source abbreviation (RSAB) this atom came from.
    pub sab: String,
    /// TTY — term type within the source (`PT`, `SY`, `MH`, …).
    pub tty: String,
    /// STR — the surface string itself.
    pub str_: String,
    /// SUPPRESS — `N` = not suppressed; `O`/`E`/`Y` = suppressed.
    pub suppress: String,
}

/// Parse one MRCONSO line. `None` for short rows.
pub fn parse_mrconso_line(line: &str) -> Option<Atom> {
    let f: Vec<&str> = line.split('|').collect();
    if f.len() < 17 {
        return None;
    }
    Some(Atom {
        cui: f[0].to_string(),
        lat: f[1].to_string(),
        ts: f[2].to_string(),
        ispref: f[6].to_string(),
        sab: f[11].to_string(),
        tty: f[12].to_string(),
        str_: f[14].to_string(),
        suppress: f[16].to_string(),
    })
}

/// One MRSTY row — a (concept, semantic-type) assignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sty {
    /// CUI — the concept.
    pub cui: String,
    /// TUI — the semantic type identifier (`T047`, `T028`, …).
    pub tui: String,
    /// STY — the semantic type's human name ("Disease or Syndrome").
    pub sty: String,
}

/// Parse one MRSTY line. `None` for short rows.
pub fn parse_mrsty_line(line: &str) -> Option<Sty> {
    let f: Vec<&str> = line.split('|').collect();
    if f.len() < 4 {
        return None;
    }
    if f[0].is_empty() || f[1].is_empty() {
        return None;
    }
    Some(Sty {
        cui: f[0].to_string(),
        tui: f[1].to_string(),
        sty: f[3].to_string(),
    })
}

/// One MRDEF row — a concept definition from a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Def {
    /// CUI — the concept defined.
    pub cui: String,
    /// SAB — the source the definition came from.
    pub sab: String,
    /// DEF — the definition text.
    pub def: String,
    /// SUPPRESS — `N` = usable.
    pub suppress: String,
}

/// Parse one MRDEF line. `None` for short rows.
pub fn parse_mrdef_line(line: &str) -> Option<Def> {
    let f: Vec<&str> = line.split('|').collect();
    if f.len() < 7 {
        return None;
    }
    Some(Def {
        cui: f[0].to_string(),
        sab: f[4].to_string(),
        def: f[5].to_string(),
        suppress: f[6].to_string(),
    })
}

/// Parse MRSAB into a `RSAB → SRL` map (the Source Restriction Level per source).
/// RSAB is col 4 (index 3), SRL is col 14 (index 13). The first occurrence of an
/// RSAB wins (versions of one source share an SRL).
pub fn parse_mrsab(text: &str) -> BTreeMap<String, u32> {
    let mut out: BTreeMap<String, u32> = BTreeMap::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('|').collect();
        if f.len() < 14 {
            continue;
        }
        let rsab = f[3].trim();
        if rsab.is_empty() {
            continue;
        }
        if let Ok(srl) = f[13].trim().parse::<u32>() {
            out.entry(rsab.to_string()).or_insert(srl);
        }
    }
    out
}

/// The set of RSABs whose Source Restriction Level is 0 (Level 0 / redistribution
/// clean) — the importer's allowlist; everything else is dropped to honor the UMLS
/// license, even if present in the input.
pub fn srl0_allowlist(srl: &BTreeMap<String, u32>) -> BTreeSet<String> {
    srl.iter()
        .filter(|(_, &lvl)| lvl == 0)
        .map(|(sab, _)| sab.clone())
        .collect()
}

/// Parse MRRANK into a `(SAB, TTY) → rank` map. RANK is col 1 (index 0, a 4-digit
/// string; higher = more preferred), SAB col 2, TTY col 3.
pub fn parse_mrrank(text: &str) -> BTreeMap<(String, String), u32> {
    let mut out: BTreeMap<(String, String), u32> = BTreeMap::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('|').collect();
        if f.len() < 3 {
            continue;
        }
        if let Ok(rank) = f[0].trim().parse::<u32>() {
            out.insert((f[1].trim().to_string(), f[2].trim().to_string()), rank);
        }
    }
    out
}

// ── The joined subset model ───────────────────────────────────────────

/// A UMLS semantic type used by the imported concepts → a `umls:SemanticType` class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticType {
    /// TUI — the type identifier (`T047`), used as the class IRI local.
    pub tui: String,
    /// The human name ("Disease or Syndrome") → the class `description`.
    pub name: String,
}

/// A UMLS concept → a `umls:Concept` class subclassed under its semantic type(s).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Concept {
    /// CUI — the concept identifier (`C0043119`), used as the class IRI local.
    pub cui: String,
    /// The concept's semantic types (TUIs), sorted — these become its `subclass_of`
    /// edges (and so reach `lexicon:Entity` transitively).
    pub tuis: Vec<String>,
    /// The canonical name (highest-ranked English atom) → drives the `description`.
    pub preferred_name: String,
    /// Distinct English surface strings (preferred name first, rest sorted) → one
    /// lexical entry each.
    pub forms: Vec<String>,
    /// A definition (best available SRL-0 source), with the source SAB, if any.
    pub definition: Option<(String, String)>,
    /// The concept's **proper-noun symbol** when it is a NAMED INDIVIDUAL (D62 —
    /// `docs/notes/d62-named-individual-typing.md`): present iff an atom from a nomenclature
    /// authority ([`NAMED_INDIVIDUAL_SABS`], e.g. HGNC for genes) supplied a symbol (`ACR`/`PT`).
    /// `Some` ⇒ emit as an **instance** of its semantic-type class with `cat_np` entries; `None` ⇒
    /// a concept **class** with `cat_n` entries (the default).
    pub symbol: Option<String>,
}

/// Nomenclature authorities whose symbols mark a concept a **named individual** (`cat_np`) rather
/// than a concept class (D62 — `docs/notes/d62-named-individual-typing.md`). HGNC is the gene-symbol
/// authority; extend deliberately (a short, auditable allow-list). A symbol-type atom (`ACR`/`PT`)
/// from such a source supplies the proper-noun form.
const NAMED_INDIVIDUAL_SABS: &[&str] = &["HGNC"];

/// Priority of a term-type as a NAMED-INDIVIDUAL **symbol** (higher wins): `ACR` (the approved
/// acronym/symbol) over `PT` (preferred term, also the symbol for HGNC). `0` = not a symbol type.
fn symbol_tty_priority(tty: &str) -> u8 {
    match tty {
        "ACR" => 2,
        "PT" => 1,
        _ => 0,
    }
}

/// The result of the join: the concepts to mirror plus the semantic-type classes they
/// reference (every TUI appearing on any included concept — so no `subclass_of` is
/// left dangling).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Subset {
    /// Concept classes (sorted by CUI).
    pub concepts: Vec<Concept>,
    /// Semantic-type classes (sorted by TUI).
    pub semantic_types: Vec<SemanticType>,
}

/// Source vocabularies preferred for a concept's definition (lower index = better).
/// All are SRL-0; the list just picks a clean, readable gloss when several exist.
const DEF_SOURCE_PREFERENCE: &[&str] = &["MSH", "NCI", "CSP", "HPO", "MEDLINEPLUS", "PDQ"];

fn def_priority(sab: &str) -> usize {
    DEF_SOURCE_PREFERENCE
        .iter()
        .position(|s| *s == sab)
        .unwrap_or(DEF_SOURCE_PREFERENCE.len())
}

/// One distinct surface of a concept, keyed by its lowercase, with source provenance (D63 A2, the
/// CHV-redundant filter).
#[derive(Default)]
struct FormInfo {
    /// First display casing seen (first-seen wins).
    display: String,
    /// Some contributing source was **CHV** (Consumer Health Vocabulary — lay paraphrases).
    has_chv: bool,
    /// Some contributing source was **not** CHV (an authoritative vocabulary: MSH/NCI/…).
    has_nonchv: bool,
}

/// Per-concept name/form accumulator.
#[derive(Default)]
struct Forms {
    /// Best canonical-name candidate: (score, string).
    best: Option<(u32, String)>,
    /// Distinct surface strings, keyed by lowercase (first display casing wins), with provenance.
    by_key: BTreeMap<String, FormInfo>,
}

/// Accumulates the MRSTY/MRCONSO/MRDEF streams into a [`Subset`].
///
/// **Feed order matters**: all `add_sty` calls first (they decide which concepts are
/// selected and record every concept's full TUI set), then `add_atom` / `add_def`
/// (gated on the selected set). The streaming binary respects this; tests pass whole
/// slices in the same order.
pub struct ConceptBuilder {
    srl0: BTreeSet<String>,
    ranks: BTreeMap<(String, String), u32>,
    allow_tui: Option<BTreeSet<String>>,
    language: String,
    // Accumulators.
    tui_names: BTreeMap<String, String>,
    cui_tuis: BTreeMap<String, BTreeSet<String>>,
    selected: BTreeSet<String>,
    cui_forms: BTreeMap<String, Forms>,
    cui_def: BTreeMap<String, (usize, String, String)>, // (priority, sab, def)
    cui_symbol: BTreeMap<String, (u8, String)>, // named-individual symbol: (tty-priority, string)
    /// Lowercased surfaces that ANY concept carries from a **non-CHV** source — the coverage witness
    /// the A2 CHV-redundant filter checks before dropping (D63).
    surf_nonchv: BTreeSet<String>,
    /// A2: drop a concept's REDUNDANT multiword CHV-only alias (a compound surface it gets only from
    /// CHV, which an authoritative source already provides elsewhere). Off by default; opt-in.
    drop_chv_redundant: bool,
}

impl ConceptBuilder {
    /// `srl0` is the allowed-source set ([`srl0_allowlist`]); `ranks` from
    /// [`parse_mrrank`]; `allow_tui` is the semantic-type filter (`None` ⇒ all TUIs);
    /// `language` is the MRCONSO LAT to keep (e.g. `"ENG"`).
    pub fn new(
        srl0: BTreeSet<String>,
        ranks: BTreeMap<(String, String), u32>,
        allow_tui: Option<BTreeSet<String>>,
        language: impl Into<String>,
    ) -> Self {
        Self {
            srl0,
            ranks,
            allow_tui,
            language: language.into(),
            tui_names: BTreeMap::new(),
            cui_tuis: BTreeMap::new(),
            selected: BTreeSet::new(),
            cui_forms: BTreeMap::new(),
            cui_def: BTreeMap::new(),
            cui_symbol: BTreeMap::new(),
            surf_nonchv: BTreeSet::new(),
            drop_chv_redundant: false,
        }
    }

    /// Enable the A2 CHV-redundant filter (D63): drop each concept's redundant multiword CHV-only
    /// alias in [`Self::finish`]. Off by default (a faithful import); the parse harness opts in.
    pub fn with_drop_chv_redundant(mut self, on: bool) -> Self {
        self.drop_chv_redundant = on;
        self
    }

    /// Whether a TUI passes the semantic-type filter.
    fn tui_allowed(&self, tui: &str) -> bool {
        self.allow_tui.as_ref().is_none_or(|a| a.contains(tui))
    }

    /// Record a semantic-type assignment. Builds the global TUI→name dictionary, the
    /// concept's full TUI set, and — if the TUI is allowed — marks the concept selected.
    pub fn add_sty(&mut self, sty: &Sty) {
        if !sty.sty.is_empty() {
            self.tui_names
                .entry(sty.tui.clone())
                .or_insert_with(|| sty.sty.clone());
        }
        self.cui_tuis
            .entry(sty.cui.clone())
            .or_default()
            .insert(sty.tui.clone());
        if self.tui_allowed(&sty.tui) {
            self.selected.insert(sty.cui.clone());
        }
    }

    /// Record an atom (surface form). Kept only for a selected concept, in the target
    /// language, from an SRL-0 source, not suppressed.
    pub fn add_atom(&mut self, a: &Atom) {
        if a.lat != self.language
            || a.suppress != "N"
            || !self.selected.contains(&a.cui)
            || !self.srl0.contains(&a.sab)
        {
            return;
        }
        let str_ = a.str_.trim();
        if str_.is_empty() {
            return;
        }
        // Canonical-name score: MRRANK precedence dominates, then preferred-LUI (TS=P)
        // and preferred-atom (ISPREF=Y) as tie-breakers.
        let rank = self
            .ranks
            .get(&(a.sab.clone(), a.tty.clone()))
            .copied()
            .unwrap_or(0);
        let score = rank * 4 + u32::from(a.ts == "P") * 2 + u32::from(a.ispref == "Y");

        // Source provenance for the A2 CHV-redundant filter (D63): CHV = Consumer Health Vocabulary.
        let is_chv = a.sab == "CHV";
        let lc = str_.to_lowercase();
        if !is_chv {
            self.surf_nonchv.insert(lc.clone());
        }
        let entry = self.cui_forms.entry(a.cui.clone()).or_default();
        match &entry.best {
            Some((s, _)) if *s >= score => {}
            _ => entry.best = Some((score, str_.to_string())),
        }
        let info = entry.by_key.entry(lc).or_insert_with(|| FormInfo {
            display: str_.to_string(),
            has_chv: false,
            has_nonchv: false,
        });
        if is_chv {
            info.has_chv = true;
        } else {
            info.has_nonchv = true;
        }

        // Named-individual symbol (D62): a nomenclature-authority atom (HGNC) with a symbol
        // term-type (`ACR`/`PT`) marks this concept a NAMED INDIVIDUAL and supplies its proper-noun
        // (`cat_np`) form. Keep the highest-priority symbol (ACR > PT) seen.
        if NAMED_INDIVIDUAL_SABS.contains(&a.sab.as_str()) {
            let prio = symbol_tty_priority(&a.tty);
            if prio > 0 {
                let slot = self
                    .cui_symbol
                    .entry(a.cui.clone())
                    .or_insert((0, String::new()));
                if prio > slot.0 {
                    *slot = (prio, str_.to_string());
                }
            }
        }
    }

    /// Record a definition. Kept for a selected concept from an SRL-0 source, not
    /// suppressed; the most-preferred source's definition wins.
    pub fn add_def(&mut self, d: &Def) {
        if d.suppress != "N" || !self.selected.contains(&d.cui) || !self.srl0.contains(&d.sab) {
            return;
        }
        let def = d.def.trim();
        if def.is_empty() {
            return;
        }
        let prio = def_priority(&d.sab);
        match self.cui_def.get(&d.cui) {
            Some((p, _, _)) if *p <= prio => {}
            _ => {
                self.cui_def
                    .insert(d.cui.clone(), (prio, d.sab.clone(), def.to_string()));
            }
        }
    }

    /// Finish the join. `limit` caps the number of concepts (sorted by CUI) — `None`
    /// for the full set. Concepts with no kept English atom are dropped (nothing to
    /// name them by). The returned `semantic_types` are exactly those referenced by
    /// the emitted concepts.
    pub fn finish(self, limit: Option<usize>) -> Subset {
        let mut concepts = Vec::new();
        let mut used_tuis: BTreeSet<String> = BTreeSet::new();
        let mut chv_dropped = 0usize; // A2: redundant multiword CHV-only aliases skipped

        for cui in &self.selected {
            let Some(forms_acc) = self.cui_forms.get(cui) else {
                continue; // no SRL-0 English atom survived — un-nameable, skip.
            };
            let Some((_, preferred)) = &forms_acc.best else {
                continue;
            };
            let tuis: Vec<String> = self
                .cui_tuis
                .get(cui)
                .map(|s| s.iter().cloned().collect())
                .unwrap_or_default();
            if tuis.is_empty() {
                continue;
            }
            for t in &tuis {
                used_tuis.insert(t.clone());
            }

            // Forms: preferred name first, then the rest sorted (deduped by lowercase).
            let mut forms = vec![preferred.clone()];
            let pref_key = preferred.to_lowercase();
            for (key, info) in &forms_acc.by_key {
                if *key == pref_key {
                    continue;
                }
                // A2 (D63): drop a REDUNDANT multiword CHV-only alias — a compound surface this
                // concept gets ONLY from CHV, where an authoritative source already provides that
                // surface for some concept (so it stays seedable — coverage-safe). It removes the
                // spurious second concept-reading of a compound that is otherwise one concept
                // (`C0610268` "DNA Helicase A" aliasing `dna helicase`, which MSH gives the enzyme).
                // The preferred name is never dropped. Multiword only: single-word CHV collisions are
                // the A1 (LLM-adjudicated) class.
                if self.drop_chv_redundant
                    && key.contains(' ')
                    && info.has_chv
                    && !info.has_nonchv
                    && self.surf_nonchv.contains(key)
                {
                    chv_dropped += 1;
                    continue;
                }
                forms.push(info.display.clone());
            }

            concepts.push(Concept {
                cui: cui.clone(),
                tuis,
                preferred_name: preferred.clone(),
                forms,
                definition: self
                    .cui_def
                    .get(cui)
                    .map(|(_, sab, def)| (sab.clone(), def.clone())),
                symbol: self.cui_symbol.get(cui).map(|(_, s)| s.clone()),
            });
        }

        if let Some(n) = limit {
            concepts.truncate(n);
            // Recompute used TUIs over the retained concepts so no semantic-type class
            // is emitted that no surviving concept references (and vice-versa).
            used_tuis.clear();
            for c in &concepts {
                for t in &c.tuis {
                    used_tuis.insert(t.clone());
                }
            }
        }

        let semantic_types = used_tuis
            .into_iter()
            .map(|tui| SemanticType {
                name: self
                    .tui_names
                    .get(&tui)
                    .cloned()
                    .unwrap_or_else(|| tui.clone()),
                tui,
            })
            .collect();

        if self.drop_chv_redundant {
            eprintln!("A2 CHV-redundant multiword aliases dropped: {chv_dropped}");
        }

        Subset {
            concepts,
            semantic_types,
        }
    }
}

/// Build a subset from whole-file text slices (the in-memory path — tests and small
/// inputs). The binary streams the big files line-by-line through a [`ConceptBuilder`]
/// directly; this is the convenience wrapper that mirrors that join.
#[allow(clippy::too_many_arguments)]
pub fn build_subset(
    mrsab: &str,
    mrrank: &str,
    mrsty: &str,
    mrconso: &str,
    mrdef: &str,
    allow_tui: Option<BTreeSet<String>>,
    language: &str,
    limit: Option<usize>,
) -> Subset {
    let srl0 = srl0_allowlist(&parse_mrsab(mrsab));
    let ranks = parse_mrrank(mrrank);
    let mut b = ConceptBuilder::new(srl0, ranks, allow_tui, language);
    for line in mrsty.lines() {
        if let Some(s) = parse_mrsty_line(line) {
            b.add_sty(&s);
        }
    }
    for line in mrconso.lines() {
        if let Some(a) = parse_mrconso_line(line) {
            b.add_atom(&a);
        }
    }
    for line in mrdef.lines() {
        if let Some(d) = parse_mrdef_line(line) {
            b.add_def(&d);
        }
    }
    b.finish(limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real-shape MRSAB rows: two SRL-0 sources (MSH, NCI) and one restricted source
    // (SNOMEDCT_US, SRL=9) — the filter must drop the restricted one's atoms.
    // RRF MRSAB: RSAB is col 4 (index 3), SRL is col 14 (index 13).
    const MRSAB: &str = "C1|C1|MSH2026|MSH|MeSH|MSH|2026|||||||0|685|630|FULL|MH||ENG|UTF-8|Y|Y|MeSH|;|
C2|C2|NCI2026|NCI|NCI Thesaurus|NCI|2026|||||||0|1|1|FULL|PT||ENG|UTF-8|Y|Y|NCI|;|
C9|C9|SNOMEDCT_US_2026|SNOMEDCT_US|SNOMED CT|SNOMEDCT_US|2026|||||||9|1|1|FULL|PT||ENG|UTF-8|Y|Y|SNOMEDCT|;|";

    const MRRANK: &str = "0500|MSH|MH|N|
0490|NCI|PT|N|
0100|SNOMEDCT_US|PT|N|";

    // Werner syndrome (T047) and Microsatellite Instability (T049).
    const MRSTY: &str = "C0043119|T047|B2.2.1.2.1|Disease or Syndrome|AT1||
C0920269|T049|A1.2.2.2|Cell or Molecular Dysfunction|AT2||";

    // Atoms: MSH/NCI (SRL-0) kept; a SNOMEDCT_US atom must be filtered; a Spanish
    // atom must be filtered; a suppressed atom must be filtered.
    const MRCONSO: &str = "C0043119|ENG|P|L1|PF|S1|Y|A1||||MSH|MH|D014898|Werner Syndrome|0|N||
C0043119|ENG|S|L2|VO|S2|N|A2||||NCI|SY|C1|Werner's Syndrome|0|N||
C0043119|ENG|S|L3|VO|S3|N|A3||||SNOMEDCT_US|PT|111|Werner syndrome (disorder)|0|N||
C0043119|SPA|S|L4|VO|S4|N|A4||||MSH|MH|D1|Síndrome de Werner|0|N||
C0043119|ENG|S|L5|VO|S5|N|A5||||MSH|ET|D2|Suppressed Form|0|O||
C0920269|ENG|P|L6|PF|S6|Y|A6||||MSH|MH|D053842|Microsatellite Instability|0|N||
C0920269|ENG|S|L7|VO|S7|N|A7||||NCI|SY|C2|Microsatellite Instability Positive|0|N||";

    const MRDEF: &str =
        "C0043119|A1|AT10||MSH|An autosomal recessive disorder of premature aging.|N||
C0043119|A2|AT11||NCI|A rare syndrome caused by WRN mutations.|N||
C0920269|A6|AT12||SNOMEDCT_US|should be filtered (restricted source).|N||";

    #[test]
    fn srl0_filter_excludes_restricted_sources() {
        let allow = srl0_allowlist(&parse_mrsab(MRSAB));
        assert!(allow.contains("MSH"));
        assert!(allow.contains("NCI"));
        assert!(
            !allow.contains("SNOMEDCT_US"),
            "SRL=9 SNOMED CT must be excluded from the SRL-0 allowlist"
        );
    }

    #[test]
    fn join_builds_concepts_with_filtered_forms_and_preferred_name() {
        let sub = build_subset(MRSAB, MRRANK, MRSTY, MRCONSO, MRDEF, None, "ENG", None);
        assert_eq!(sub.concepts.len(), 2, "two concepts");
        assert_eq!(sub.semantic_types.len(), 2, "T047 + T049");

        let werner = sub.concepts.iter().find(|c| c.cui == "C0043119").unwrap();
        assert_eq!(werner.tuis, vec!["T047"]);
        // MSH MH outranks NCI SY ⇒ canonical name from MeSH.
        assert_eq!(werner.preferred_name, "Werner Syndrome");
        // English MSH/NCI forms kept; SNOMED, Spanish, and suppressed forms dropped.
        assert!(werner.forms.contains(&"Werner Syndrome".to_string()));
        assert!(werner.forms.contains(&"Werner's Syndrome".to_string()));
        assert!(
            !werner.forms.iter().any(|f| f.contains("disorder")),
            "the SRL>0 SNOMED form must be filtered"
        );
        assert!(
            !werner.forms.iter().any(|f| f.contains("Síndrome")),
            "non-English atoms must be filtered"
        );
        assert!(
            !werner.forms.iter().any(|f| f == "Suppressed Form"),
            "suppressed atoms must be filtered"
        );
        // Definition prefers MSH over NCI; the SNOMED def on the other concept is dropped.
        let (sab, def) = werner.definition.as_ref().unwrap();
        assert_eq!(sab, "MSH");
        assert!(def.contains("premature aging"));

        let msi = sub.concepts.iter().find(|c| c.cui == "C0920269").unwrap();
        assert_eq!(msi.tuis, vec!["T049"]);
        assert!(
            msi.definition.is_none(),
            "MSI's only definition was from a restricted source ⇒ dropped"
        );
    }

    #[test]
    fn semantic_type_allowlist_subsets_concepts() {
        let allow: BTreeSet<String> = ["T047".to_string()].into_iter().collect();
        let sub = build_subset(
            MRSAB,
            MRRANK,
            MRSTY,
            MRCONSO,
            MRDEF,
            Some(allow),
            "ENG",
            None,
        );
        assert_eq!(sub.concepts.len(), 1, "only the T047 concept is selected");
        assert_eq!(sub.concepts[0].cui, "C0043119");
        assert_eq!(sub.semantic_types.len(), 1);
        assert_eq!(sub.semantic_types[0].tui, "T047");
    }

    #[test]
    fn limit_caps_concepts_and_prunes_unused_semantic_types() {
        let sub = build_subset(MRSAB, MRRANK, MRSTY, MRCONSO, MRDEF, None, "ENG", Some(1));
        assert_eq!(sub.concepts.len(), 1);
        // Sorted by CUI ⇒ C0043119 (T047) is the one kept; T049 must be pruned.
        assert_eq!(sub.concepts[0].cui, "C0043119");
        assert_eq!(sub.semantic_types.len(), 1);
        assert_eq!(sub.semantic_types[0].tui, "T047");
    }

    #[test]
    fn non_nomenclature_concepts_are_not_named_individuals() {
        // The Werner disease + MSI concepts have no HGNC atom ⇒ stay concept classes (symbol None).
        let sub = build_subset(MRSAB, MRRANK, MRSTY, MRCONSO, MRDEF, None, "ENG", None);
        for c in &sub.concepts {
            assert!(
                c.symbol.is_none(),
                "{} should not be a named individual",
                c.cui
            );
        }
    }

    #[test]
    fn hgnc_symbol_marks_a_named_individual_with_its_symbol() {
        // An HGNC gene concept: the ACR atom is the symbol ⇒ named individual (D62). The descriptive
        // name (a non-symbol TTY) does NOT override the symbol; the ACR wins.
        const SAB: &str =
            "C1|C1|HGNC2026|HGNC|HGNC|HGNC|2026|||||||0|1|1|FULL|ACR||ENG|UTF-8|Y|Y|HGNC|;|";
        const RANK: &str = "0300|HGNC|ACR|N|\n0290|HGNC|NA|N|";
        const STY: &str = "C1337007|T028|A1.2.3.5|Gene or Genome|AT1||";
        const CONSO: &str = "C1337007|ENG|P|L1|PF|S1|Y|A1||||HGNC|ACR|HGNC:12791|WRN|0|N||\n\
C1337007|ENG|S|L2|VO|S2|N|A2||||HGNC|NA|HGNC:12791|Werner syndrome RecQ like helicase|0|N||";
        let sub = build_subset(SAB, RANK, STY, CONSO, "", None, "ENG", None);
        let gene = sub.concepts.iter().find(|c| c.cui == "C1337007").unwrap();
        assert_eq!(
            gene.symbol.as_deref(),
            Some("WRN"),
            "HGNC ACR atom marks the gene a named individual with symbol WRN"
        );
        assert_eq!(gene.tuis, vec!["T028"]);
    }

    /// A2 CHV-redundant filter (D63): a concept's REDUNDANT multiword CHV-only alias is dropped
    /// (`C0610268` "DNA Helicase A" aliasing `dna helicase`, which MSH gives the real enzyme), while
    /// a CHV-UNIQUE compound and every single-word CHV alias are KEPT (coverage-safe).
    #[test]
    fn a2_drops_redundant_multiword_chv_only_alias_only() {
        // Two SRL-0 sources: MSH (authoritative) and CHV (consumer paraphrases).
        const MRSAB: &str = "C1|C1|MSH|MSH|MeSH|MSH|2026|||||||0|1|1|FULL|MH||ENG|UTF-8|Y|Y|MeSH|;|
C2|C2|CHV|CHV|Consumer Health Vocabulary|CHV|2026|||||||0|1|1|FULL|SY||ENG|UTF-8|Y|Y|CHV|;|";
        const MRRANK: &str = "0500|MSH|MH|N|
0490|MSH|SY|N|
0100|CHV|SY|N|";
        const MRSTY: &str = "CA|T116|A1.1|Amino Acid, Peptide, or Protein|AT1||
CB|T116|A1.1|Amino Acid, Peptide, or Protein|AT2||";
        // CA "DNA Helicase A": 'dna helicase' ONLY from CHV (redundant — CB has it from MSH);
        //   'sugar diabetes' ONLY from CHV, CHV-UNIQUE; 'helicase' ONLY from CHV, single word.
        // CB "DNA helicase": the real enzyme, 'dna helicase' + 'helicase' from MSH.
        const MRCONSO: &str = "CA|ENG|P|L1|PF|S1|Y|A1||||MSH|MH|D1|DNA Helicase A|0|N||
CA|ENG|S|L2|VO|S2|N|A2||||CHV|SY|D2|dna helicase|0|N||
CA|ENG|S|L3|VO|S3|N|A3||||CHV|SY|D3|sugar diabetes|0|N||
CA|ENG|S|L4|VO|S4|N|A4||||CHV|SY|D4|helicase|0|N||
CB|ENG|P|L5|PF|S5|Y|A5||||MSH|MH|D5|DNA helicase|0|N||
CB|ENG|S|L6|VO|S6|N|A6||||MSH|SY|D6|helicase|0|N||";

        let build = |flag: bool| -> Subset {
            let srl0 = srl0_allowlist(&parse_mrsab(MRSAB));
            let mut b = ConceptBuilder::new(srl0, parse_mrrank(MRRANK), None, "ENG")
                .with_drop_chv_redundant(flag);
            for l in MRSTY.lines() {
                if let Some(s) = parse_mrsty_line(l) {
                    b.add_sty(&s);
                }
            }
            for l in MRCONSO.lines() {
                if let Some(a) = parse_mrconso_line(l) {
                    b.add_atom(&a);
                }
            }
            b.finish(None)
        };

        let forms_of = |s: &Subset, cui: &str| -> Vec<String> {
            s.concepts
                .iter()
                .find(|c| c.cui == cui)
                .map(|c| c.forms.iter().map(|f| f.to_lowercase()).collect())
                .unwrap_or_default()
        };

        // Flag OFF (faithful import): CA keeps its CHV 'dna helicase' alias.
        let off = forms_of(&build(false), "CA");
        assert!(
            off.contains(&"dna helicase".to_string()),
            "off: CHV alias kept"
        );

        // Flag ON: the redundant multiword CHV-only alias is dropped; everything else stays.
        let on = forms_of(&build(true), "CA");
        assert!(
            !on.contains(&"dna helicase".to_string()),
            "redundant CHV compound DROPPED"
        );
        assert!(
            on.contains(&"dna helicase a".to_string()),
            "preferred name kept"
        );
        assert!(
            on.contains(&"sugar diabetes".to_string()),
            "CHV-UNIQUE compound kept (coverage)"
        );
        assert!(
            on.contains(&"helicase".to_string()),
            "single-word CHV alias kept (multiword only)"
        );
        // CB (the authoritative enzyme) is untouched — the surface still seeds.
        assert!(forms_of(&build(true), "CB").contains(&"dna helicase".to_string()));
    }
}
