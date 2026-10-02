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

//! **HPO ↔ UMLS** — which UMLS concepts denote which HPO class.
//!
//! The WordNet↔UMLS alignment had to *judge* each pair: its only evidence was a shared surface. Here
//! NLM states the correspondence itself — every HPO atom in `MRCONSO.RRF` carries its HP code in the
//! `CODE` column — so the mapping is read, not adjudicated. HPO is the canonical side: it carries
//! the phenotype hierarchy (`core:subclass_of`) and the current release; a UMLS concept is two levels
//! deep (`CUI → TUI → Entity`).
//!
//! A concept is merged when exactly one **live** HP code names it. It is left alone when
//!
//! - its entries already denote a WordNet synset (the D63 alignment) — WordNet stays canonical for
//!   them, so a concept is never split between two classes. Where one live HP code names such a
//!   concept, the HP class, the synset and the concept are declared EQUIVALENT instead
//!   (`core:EquivalentClasses`, D99 §11): the lexicon keeps one reading, and subsumption links the
//!   three;
//! - it carries several live HP codes — UMLS put distinct HPO terms in one concept, and choosing one
//!   would assert an identity NLM did not;
//! - none of its HP codes is live on the chain — obsolete in the current release, or newer than the
//!   chain's HPO (an obsolete term's replacement is not followed: that is a curation step).
//!
//! Several concepts naming one HP code is fine: their entries all denote that class.
//!
//! **Only HPO's own names move.** A UMLS concept gathers synonyms from many sources, and not all of
//! them name the HPO term: `C0521114` holds HPO's `Occasional` (`HP:0040283`, 29–5%) and NCI's `Rare`.
//! An entry is redefined only when its surface is one of the HP code's own HPO strings in the
//! concept; the concept's other surfaces keep denoting the UMLS concept. A wrong merge destroys the
//! correct reading, a missed one leaves things as they were.

use std::collections::{BTreeMap, BTreeSet};

/// HPO's atoms in UMLS: `CUI → HP code → {the code's own strings, lowercased}`, from `MRCONSO.RRF`
/// rows with `SAB = HPO`, an `HP:` code, and not obsolete (`SUPPRESS ≠ O`). Columns (0-based): CUI 0,
/// SAB 11, CODE 13, STR 14, SUPPRESS 16.
pub type HpoAtoms = BTreeMap<String, BTreeMap<String, BTreeSet<String>>>;

pub fn hpo_atoms<'a>(lines: impl IntoIterator<Item = &'a str>) -> HpoAtoms {
    let mut out: HpoAtoms = BTreeMap::new();
    for line in lines {
        let f: Vec<&str> = line.split('|').collect();
        if f.len() < 17 || f[11] != "HPO" || !f[13].starts_with("HP:") || f[16] == "O" {
            continue;
        }
        out.entry(f[0].to_string())
            .or_default()
            .entry(f[13].to_string())
            .or_default()
            .insert(f[14].to_lowercase());
    }
    out
}

/// Whether an entry with surface `form` names the HP code `code` in concept `cui` — one of the code's
/// own HPO strings there.
pub fn is_hpo_name(atoms: &HpoAtoms, cui: &str, code: &str, form: &str) -> bool {
    atoms
        .get(cui)
        .and_then(|codes| codes.get(code))
        .is_some_and(|names| names.contains(&form.to_lowercase()))
}

/// The merge set, and why the rest was left alone.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// `CUI → HP code`: the concept's entries will denote the HP class.
    pub merges: BTreeMap<String, String>,
    /// `CUI → HP code` for concepts WordNet took that exactly one live HP code names: the HP class
    /// is declared equivalent to the synset and the concept (D99 §11).
    pub left_to_wordnet: BTreeMap<String, String>,
    pub skipped_wordnet: usize,
    pub skipped_ambiguous: usize,
    pub skipped_not_live: usize,
}

/// Decide each concept. `is_live(code)` says whether `HP:…` resolves on the chain and is not
/// deprecated.
pub fn plan(
    atoms: &HpoAtoms,
    wordnet_cuis: &BTreeSet<String>,
    is_live: impl Fn(&str) -> bool,
) -> Plan {
    let mut p = Plan::default();
    for (cui, codes) in atoms {
        let live: Vec<&String> = codes.keys().filter(|c| is_live(c)).collect();
        if wordnet_cuis.contains(cui) {
            p.skipped_wordnet += 1;
            if let [one] = live.as_slice() {
                p.left_to_wordnet.insert(cui.clone(), (*one).clone());
            }
            continue;
        }
        match live.as_slice() {
            [] => p.skipped_not_live += 1,
            [one] => {
                p.merges.insert(cui.clone(), (*one).clone());
            }
            _ => p.skipped_ambiguous += 1,
        }
    }
    p
}

/// The ESL name of an HP class: `HP:0001250` → `hp:'0001250'` under `namespace hp = "urn:obo:HP"`.
/// Quoted because the local part starts with a digit, which a bare ESL name may not.
pub fn hp_qname(code: &str) -> String {
    format!("hp:'{}'", code.trim_start_matches("HP:"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(cui: &str, sab: &str, code: &str, name: &str, suppress: &str) -> String {
        // CUI|LAT|TS|LUI|STT|SUI|ISPREF|AUI|SAUI|SCUI|SDUI|SAB|TTY|CODE|STR|SRL|SUPPRESS|CVF|
        format!("{cui}|ENG|P|L1|PF|S1|Y|A1|||{code}|{sab}|PT|{code}|{name}|0|{suppress}||")
    }

    #[test]
    fn reads_hpo_atoms_and_drops_obsolete_ones() {
        let rows = [
            row("C1", "HPO", "HP:0001250", "Seizure", "N"),
            row("C1", "HPO", "HP:0001250", "Seizures", "Y"), // suppressible synonym — still a name
            row("C1", "HPO", "HP:0002373", "Febrile seizures", "O"), // obsolete atom
            row("C2", "MSH", "D012640", "Seizures", "N"),    // another source
        ];
        let a = hpo_atoms(rows.iter().map(String::as_str));
        assert_eq!(a.len(), 1);
        assert_eq!(
            a["C1"]["HP:0001250"],
            BTreeSet::from(["seizure".to_string(), "seizures".to_string()])
        );
        assert!(!a["C1"].contains_key("HP:0002373"));
    }

    /// `C0521114` holds HPO's `Occasional` and NCI's `Rare`: only HPO's own name moves.
    #[test]
    fn only_the_hp_codes_own_names_move() {
        let rows = [
            row("C0521114", "HPO", "HP:0040283", "Occasional", "N"),
            row("C0521114", "HPO", "HP:0040283", "Occasional (29-5%)", "N"),
            row("C0521114", "NCI", "C64954", "Rare", "N"),
        ];
        let a = hpo_atoms(rows.iter().map(String::as_str));
        assert!(is_hpo_name(&a, "C0521114", "HP:0040283", "occasional"));
        assert!(is_hpo_name(
            &a,
            "C0521114",
            "HP:0040283",
            "Occasional (29-5%)"
        ));
        assert!(!is_hpo_name(&a, "C0521114", "HP:0040283", "Rare"));
    }

    #[test]
    fn merges_one_live_code_and_says_why_the_rest_stayed() {
        let atoms: HpoAtoms = [
            ("C1", vec!["HP:0001250"]),               // one live code → merge
            ("C2", vec!["HP:0001250"]),               // a second concept for the same code → merge
            ("C3", vec!["HP:0000118", "HP:0000707"]), // two live codes → ambiguous
            ("C4", vec!["HP:0000003"]),               // obsolete in the chain's HPO
            ("C5", vec!["HP:0002664"]),               // WordNet already canonical
            ("C6", vec!["HP:0000003", "HP:0000708"]), // one of two is live → merge on that one
        ]
        .into_iter()
        .map(|(c, v)| {
            let codes = v
                .into_iter()
                .map(|code| (code.to_string(), BTreeSet::from(["name".to_string()])))
                .collect();
            (c.to_string(), codes)
        })
        .collect();
        let wordnet = BTreeSet::from(["C5".to_string()]);
        let live = |c: &str| c != "HP:0000003";
        let p = plan(&atoms, &wordnet, live);
        assert_eq!(
            p.merges,
            BTreeMap::from([
                ("C1".to_string(), "HP:0001250".to_string()),
                ("C2".to_string(), "HP:0001250".to_string()),
                ("C6".to_string(), "HP:0000708".to_string()),
            ])
        );
        assert_eq!(
            (p.skipped_wordnet, p.skipped_ambiguous, p.skipped_not_live),
            (1, 1, 1)
        );
        // WordNet's concept keeps its synset; its one live HP code is recorded for an equivalence.
        assert_eq!(
            p.left_to_wordnet,
            BTreeMap::from([("C5".to_string(), "HP:0002664".to_string())])
        );
    }

    #[test]
    fn an_hp_code_is_named_in_esl_by_its_quoted_local_part() {
        assert_eq!(hp_qname("HP:0001250"), "hp:'0001250'");
    }
}
