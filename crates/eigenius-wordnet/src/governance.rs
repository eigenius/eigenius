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

//! eigenius#263 / D97 slice 3a — the prepositions a gradable adjective sense governs.
//!
//! The attested sources (D97, "eigenius#263 folds in"):
//! - WordNet's ``followed by `p'`` convention, which names prepositions for one sense;
//! - SPECIALIST's `compl=pphr(p, …)` and the curated frames (`adjective-frames.tsv`), which name
//!   them for a lemma.
//!
//! A lemma-level preposition goes on the senses D97 decision 7 picks, among the lemma's gradable
//! senses (a relational adjective has no relational reading to carry it): the one sense, if the
//! lemma has one; else the senses the judge accepted ([`Placements`], resolved from its committed
//! verdicts). The derivational pointers are not evidence here: on the items they would decide, the
//! judge disagreed on 440 senses, and in a sample of 32 disagreements it was right 25 times and the
//! pointers 3 (decision 7, revised 2026-09-30).
//!
//! The verbs' complements are [`crate::verb_governance`]'s; [`build`] reads both.
//!
//! An item with several senses the judge has not placed stops the import
//! ([`Classified::governance`]). The gloss heuristic speaks only for a lemma no source attests,
//! and never names `as`: its `as` matches are equatives and definition wording. It proposes; the
//! judge places (decision 7, revised 2026-10-01): its pattern is often an adjunct or a passive's
//! agent («boggy under foot», «aggravated by passive resistance»), so its items go to the judge
//! even on a one-sense lemma. A preposition
//! outside `lexicon:Prep` is counted, not placed (D97 slice 2 brings its argument entries); `than`
//! belongs to the comparative.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use eigenius_kernel::dcg::category::prep_constructor;

use crate::verb_governance::{classify_verbs, VerbCounts, VerbGovernance};
use crate::wndb::{Offset, Synset};

/// The prepositions WordNet's convention names in a gloss: ``(usually) followed by `on'``,
/// ``(often followed by `of' or `to')``. Only prepositions `lexicon:Prep` names.
pub fn convention_prepositions(gloss: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for part in gloss.split("followed by").skip(1) {
        let clause = part.split([')', ';']).next().unwrap_or("");
        for quoted in clause.split('`').skip(1) {
            let word = quoted.split('\'').next().unwrap_or("").trim();
            if prep_constructor(word).is_some() {
                out.insert(word.to_string());
            }
        }
    }
    out
}

/// The lemma-in-gloss heuristic: the preposition directly after `lemma` in its own gloss or
/// examples (`proportional to the crime`), the first found; never `as`, whose matches are equatives
/// (`black as coal`) and definition wording (`accompanying as a consequence`).
pub fn heuristic_preposition(gloss: &str, lemma: &str) -> Option<String> {
    let g = gloss.to_lowercase();
    let key = format!("{} ", lemma.to_lowercase());
    let mut from = 0;
    while let Some(i) = g[from..].find(&key) {
        let next = g[from + i + key.len()..]
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_end_matches(|c: char| !c.is_ascii_alphabetic());
        if next != "as" && prep_constructor(next).is_some() {
            return Some(next.to_string());
        }
        from += i + key.len();
    }
    None
}

/// The judge's placements, resolved from its committed verdicts: for a (lemma, complement) on
/// several senses, the senses it goes on — none, a gap, where the judge said no sense fits. Read from
/// `adjective-senses.tsv` or `verb-senses.tsv`: `lemma <TAB> complement <TAB> offset,offset,…` or
/// `-`, with `#` comments. An adjective's complement is a preposition; a verb's a preposition,
/// `object` or `clause` ([`crate::verb_governance::label`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Placements {
    map: BTreeMap<(String, String), BTreeSet<Offset>>,
}

impl Placements {
    pub fn read(path: &Path) -> Result<Placements, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Placements, String> {
        let mut map = BTreeMap::new();
        for (i, line) in text.lines().enumerate() {
            let line = line.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let cols: Vec<&str> = line.split('\t').collect();
            let [lemma, prep, senses] = cols[..] else {
                return Err(format!("line {}: expected 3 tab-separated columns", i + 1));
            };
            let senses: BTreeSet<Offset> = if senses == "-" {
                BTreeSet::new()
            } else {
                senses.split(',').map(str::to_string).collect()
            };
            if senses
                .iter()
                .any(|s| s.len() != 8 || !s.bytes().all(|b| b.is_ascii_digit()))
            {
                return Err(format!("line {}: a sense is not an 8-digit offset", i + 1));
            }
            map.insert((lemma.to_string(), prep.to_string()), senses);
        }
        Ok(Placements { map })
    }

    pub fn insert(&mut self, lemma: &str, complement: &str, senses: BTreeSet<Offset>) {
        self.map
            .insert((lemma.to_string(), complement.to_string()), senses);
    }

    pub(crate) fn get(&self, lemma: &str, complement: &str) -> Option<&BTreeSet<Offset>> {
        self.map.get(&(lemma.to_string(), complement.to_string()))
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// A lemma-level preposition on a lemma with one gradable sense, placed there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecidedItem {
    pub lemma: String,
    pub preposition: String,
    pub sense: Offset,
}

/// A lemma-level preposition on a lemma with several gradable senses: the judge's item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenItem {
    pub lemma: String,
    pub preposition: String,
    /// The lemma's gradable senses.
    pub senses: Vec<Offset>,
}

/// What [`classify`] counted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts {
    /// Adjective lemmas with a lemma-level preposition (SPECIALIST or curated).
    pub lemmas_attested: usize,
    /// (lemma, preposition) items `lexicon:Prep` names.
    pub items: usize,
    /// Items naming a preposition outside `lexicon:Prep`, by preposition (`than` excluded).
    pub outside: BTreeMap<String, usize>,
    /// Items whose lemma has no gradable sense.
    pub no_gradable_sense: usize,
    /// Items placed on the lemma's one gradable sense.
    pub one_sense: usize,
    /// Items on several senses, for the judge.
    pub open: usize,
    /// (sense, lemma) pairs the convention names a preposition for.
    pub convention_senses: usize,
    /// (sense, lemma) pairs the heuristic names a preposition for.
    pub heuristic_senses: usize,
    /// Of `open`, the (lemma, preposition) items the heuristic proposes.
    pub heuristic_items: usize,
}

/// Every adjective lemma's attested prepositions, sorted into the items placed on a lemma's one sense
/// and the items the judge places.
#[derive(Debug, Clone, Default)]
pub struct Classified {
    convention: BTreeMap<(Offset, String), BTreeSet<String>>,
    lemma_facts: BTreeMap<String, BTreeSet<String>>,
    pub decided: Vec<DecidedItem>,
    pub open: Vec<OpenItem>,
    pub counts: Counts,
}

/// Sort every adjective lemma's attested prepositions into placed and open items. `adjectives` is all
/// of WordNet's adjective synsets, so a lemma's senses are complete whatever the import selects;
/// `curated` is `adjective-frames.tsv`.
pub fn classify(
    adjectives: &BTreeMap<Offset, Synset>,
    specialist: Option<&eigenius_specialist::Lexicon>,
    curated: &BTreeMap<String, String>,
) -> Classified {
    let mut c = Classified::default();
    // A lemma's gradable senses, in offset order.
    let mut senses: BTreeMap<String, Vec<Offset>> = BTreeMap::new();
    for syn in adjectives.values().filter(|s| !s.relational) {
        let conv = convention_prepositions(&syn.gloss);
        for lemma in &syn.words {
            senses
                .entry(lemma.to_lowercase())
                .or_default()
                .push(syn.offset.clone());
            if !conv.is_empty() {
                c.convention
                    .insert((syn.offset.clone(), lemma.clone()), conv.clone());
            }
        }
    }
    c.counts.convention_senses = c.convention.len();
    // Lemma-level facts.
    for lemma in senses.keys() {
        let mut preps: BTreeSet<String> = BTreeSet::new();
        if let Some(lex) = specialist {
            for rec in lex.lookup(lemma, "adj") {
                preps.extend(rec.pp_prepositions("compl"));
            }
        }
        if let Some(p) = curated.get(lemma) {
            preps.insert(p.clone());
        }
        preps.remove("than");
        let (inside, outside): (BTreeSet<String>, BTreeSet<String>) = preps
            .into_iter()
            .partition(|p| prep_constructor(p).is_some());
        for p in outside {
            *c.counts.outside.entry(p).or_default() += 1;
        }
        if !inside.is_empty() {
            c.lemma_facts.insert(lemma.clone(), inside);
        }
    }
    c.counts.lemmas_attested = c.lemma_facts.len();
    for (lemma, preps) in &c.lemma_facts {
        let all = &senses[lemma];
        for p in preps {
            c.counts.items += 1;
            if all.is_empty() {
                c.counts.no_gradable_sense += 1;
                continue;
            }
            if all.len() == 1 {
                c.decided.push(DecidedItem {
                    lemma: lemma.clone(),
                    preposition: p.clone(),
                    sense: all[0].clone(),
                });
            } else {
                c.open.push(OpenItem {
                    lemma: lemma.clone(),
                    preposition: p.clone(),
                    senses: all.clone(),
                });
            }
        }
    }
    c.counts.one_sense = c.decided.len();
    // The heuristic, for a lemma no source attests: no lemma-level fact, and no sense of it carries
    // the convention. Its proposals are the judge's items, on the lemma's every gradable sense.
    let conventional: BTreeSet<String> =
        c.convention.keys().map(|(_, l)| l.to_lowercase()).collect();
    let mut proposed: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for syn in adjectives.values().filter(|s| !s.relational) {
        for lemma in &syn.words {
            let key = lemma.to_lowercase();
            if c.lemma_facts.contains_key(&key) || conventional.contains(&key) {
                continue;
            }
            if let Some(p) = heuristic_preposition(&syn.gloss, lemma) {
                c.counts.heuristic_senses += 1;
                proposed.entry(key).or_default().insert(p);
            }
        }
    }
    for (lemma, preps) in proposed {
        for p in preps {
            c.counts.heuristic_items += 1;
            c.open.push(OpenItem {
                lemma: lemma.clone(),
                preposition: p,
                senses: senses[&lemma].clone(),
            });
        }
    }
    c.counts.open = c.open.len();
    c
}

impl Classified {
    /// The governance table: every source, with the open items placed from `placements`. `Err`
    /// lists the open items `placements` does not place, or places on a sense that is not the
    /// lemma's — the import stops rather than guess.
    pub fn governance(&self, placements: &Placements) -> Result<Governance, Vec<String>> {
        let mut by_sense: BTreeMap<(Offset, String), BTreeSet<String>> = BTreeMap::new();
        for ((off, lemma), preps) in &self.convention {
            by_sense
                .entry((off.clone(), lemma.to_lowercase()))
                .or_default()
                .extend(preps.iter().cloned());
        }
        for item in &self.decided {
            by_sense
                .entry((item.sense.clone(), item.lemma.clone()))
                .or_default()
                .insert(item.preposition.clone());
        }
        let mut problems = Vec::new();
        let (mut judged, mut gaps) = (0, 0);
        for item in &self.open {
            match placements.get(&item.lemma, &item.preposition) {
                None => problems.push(format!(
                    "{} {}: no placement ({} senses)",
                    item.lemma,
                    item.preposition,
                    item.senses.len()
                )),
                Some(placed) if !placed.iter().all(|s| item.senses.contains(s)) => {
                    problems.push(format!(
                        "{} {}: placed on a sense that is not the lemma's gradable sense",
                        item.lemma, item.preposition
                    ))
                }
                Some(placed) => {
                    judged += 1;
                    gaps += usize::from(placed.is_empty());
                    for off in placed {
                        by_sense
                            .entry((off.clone(), item.lemma.clone()))
                            .or_default()
                            .insert(item.preposition.clone());
                    }
                }
            }
        }
        if !problems.is_empty() {
            return Err(problems);
        }
        Ok(Governance {
            by_sense,
            lemma_facts: self.lemma_facts.clone(),
            judged,
            gaps,
            verbs: VerbGovernance::default(),
        })
    }
}

/// The governance the importer builds: every adjective and verb synset in `dict`, SPECIALIST's
/// `LEXICON` at `specialist`, the curated frames, and the judge's placements for adjectives
/// (`placements`) and verbs (`verb_placements`). SPECIALIST is required: without it the import would
/// be a different lexicon from the one the judge's placements were made for. `Err` names what is
/// missing, each unplaced open item among it.
pub fn build(
    dict: &Path,
    specialist: &Path,
    placements: &Placements,
    verb_placements: &Placements,
) -> Result<(Governance, Counts, VerbCounts), String> {
    let read = |pos: &str| {
        crate::wndb::read_data_file(&dict.join(format!("data.{pos}")))
            .map_err(|e| format!("{}: {e}", dict.join(format!("data.{pos}")).display()))
    };
    let adjectives = read("adj")?;
    let lexicon = eigenius_specialist::Lexicon::read(specialist).map_err(|e| {
        format!(
            "{}: {e} — run scripts/provision-specialist.sh",
            specialist.display()
        )
    })?;
    let classified = classify(
        &adjectives,
        Some(&lexicon),
        crate::convert::adjective_frames(),
    );
    let unplaced = |part: &str, unplaced: Vec<String>| {
        format!(
            "{} open item(s) with no placement from the {part} sense judge, e.g. {}",
            unplaced.len(),
            unplaced
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        )
    };
    let mut governance = classified
        .governance(placements)
        .map_err(|u| unplaced("adjective", u))?;
    let verbs = classify_verbs(&read("verb")?, &lexicon);
    governance.verbs = verbs
        .governance(verb_placements)
        .map_err(|u| unplaced("verb", u))?;
    Ok((governance, classified.counts, verbs.counts))
}

/// The prepositions each (gradable adjective sense, lemma) governs, and the complements placed on
/// each (verb sense, lemma).
#[derive(Debug, Clone, Default)]
pub struct Governance {
    by_sense: BTreeMap<(Offset, String), BTreeSet<String>>,
    lemma_facts: BTreeMap<String, BTreeSet<String>>,
    /// Open adjective items the judge's placements placed.
    pub judged: usize,
    /// Of those, the gaps: placed on no sense.
    pub gaps: usize,
    /// The verbs' (D97 slice 2).
    pub verbs: VerbGovernance,
}

impl Governance {
    /// The prepositions `lemma` governs in the sense `offset`, in order.
    pub fn of(&self, offset: &str, lemma: &str) -> Vec<String> {
        self.by_sense
            .get(&(offset.to_string(), lemma.to_lowercase()))
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Whether `lemma` is a multiword adjective `X P` whose base `X` governs `P` by a lemma-level
    /// source: it restates a frame the grammar composes, and competing with it strands the
    /// preposition's object (`dependent on`, D63).
    pub fn restates_frame(&self, lemma: &str) -> bool {
        let Some((base, prep)) = lemma.rsplit_once(' ') else {
            return false;
        };
        self.lemma_facts
            .get(&base.to_lowercase())
            .is_some_and(|preps| preps.contains(&prep.to_lowercase()))
    }

    /// (sense, lemma) pairs with a governed preposition, and the prepositions over all of them.
    pub fn totals(&self) -> (usize, usize) {
        (
            self.by_sense.len(),
            self.by_sense.values().map(BTreeSet::len).sum(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adj(offset: &str, words: &[&str], gloss: &str) -> Synset {
        Synset {
            offset: offset.to_string(),
            pos: crate::wndb::Pos::Adj,
            words: words.iter().map(|w| w.to_string()).collect(),
            gloss: gloss.to_string(),
            hypernyms: vec![],
            instance_of: vec![],
            frames: vec![],
            relational: false,
            derivational: BTreeSet::new(),
        }
    }

    const SPECIALIST: &str = "{base=dependent
entry=E1
\tcat=adj
\tcompl=pphr(on,np)
\tcompl=pphr(upon,np)
\tnominalization=dependence|noun|E2
}
{base=proportional
entry=E3
\tcat=adj
\tcompl=pphr(to,np)
}
{base=responsible
entry=E4
\tcat=adj
\tcompl=pphr(for,np)
\tcompl=pphr(to,np)
\tcompl=pphr(than,np)
\tcompl=pphr(beneath,np)
}
";

    fn fixture() -> BTreeMap<Offset, Synset> {
        [
            // `dependent`: two senses: open.
            adj("00000001", &["dependent"], "relying on something"),
            adj("00000002", &["dependent"], "hanging down"),
            // `proportional`: one sense.
            adj(
                "00000003",
                &["proportional"],
                "(usually followed by `to') in proportion",
            ),
            // `responsible`: two senses: open.
            adj("00000004", &["responsible"], "worthy of trust"),
            adj("00000005", &["responsible"], "being the cause"),
            // `addicted`: no lemma-level source; the heuristic reads its gloss.
            adj(
                "00000006",
                &["addicted"],
                "compulsively; `she is addicted to chocolate'",
            ),
            // `black`: the heuristic never names `as`.
            adj("00000007", &["black"], "as black as coal"),
        ]
        .into_iter()
        .map(|s| (s.offset.clone(), s))
        .collect()
    }

    #[test]
    fn the_convention_names_every_preposition_it_quotes() {
        assert_eq!(
            convention_prepositions("(often followed by `of' or `to') liable"),
            ["of", "to"].map(String::from).into()
        );
        assert!(convention_prepositions("followed by `that' clause").is_empty());
    }

    #[test]
    fn the_heuristic_never_names_as() {
        assert_eq!(
            heuristic_preposition("she is addicted to chocolate", "addicted").as_deref(),
            Some("to")
        );
        assert_eq!(heuristic_preposition("as black as coal", "black"), None);
    }

    #[test]
    fn a_lemma_level_preposition_goes_on_the_one_sense_and_the_judge_places_the_rest() {
        let lex = eigenius_specialist::Lexicon::parse(SPECIALIST).unwrap();
        let c = classify(&fixture(), Some(&lex), &BTreeMap::new());
        let decided: BTreeSet<(&str, &str, &str)> = c
            .decided
            .iter()
            .map(|d| (d.lemma.as_str(), d.preposition.as_str(), d.sense.as_str()))
            .collect();
        assert_eq!(
            decided,
            BTreeSet::from([("proportional", "to", "00000003")])
        );
        let open: BTreeSet<(&str, &str)> = c
            .open
            .iter()
            .map(|o| (o.lemma.as_str(), o.preposition.as_str()))
            .collect();
        assert_eq!(
            open,
            BTreeSet::from([
                ("addicted", "to"),
                ("dependent", "on"),
                ("dependent", "upon"),
                ("responsible", "for"),
                ("responsible", "to"),
            ])
        );
        assert_eq!(
            (c.counts.heuristic_senses, c.counts.heuristic_items),
            (1, 1)
        );
        assert_eq!(
            c.counts.outside,
            BTreeMap::from([("beneath".to_string(), 1)])
        );
    }

    #[test]
    fn an_open_item_without_a_placement_stops_the_import() {
        let lex = eigenius_specialist::Lexicon::parse(SPECIALIST).unwrap();
        let c = classify(&fixture(), Some(&lex), &BTreeMap::new());
        let err = c.governance(&Placements::default()).unwrap_err();
        assert_eq!(
            err.len(),
            5,
            "addicted to, the heuristic's, is the judge's too: {err:?}"
        );
        let one = |o: &str| BTreeSet::from([o.to_string()]);
        let mut placements = Placements::default();
        placements.insert("dependent", "on", one("00000001"));
        placements.insert("dependent", "upon", one("00000001"));
        placements.insert("responsible", "for", one("00000005"));
        placements.insert("responsible", "to", one("00000009"));
        placements.insert("addicted", "to", one("00000006"));
        let err = c.governance(&placements).unwrap_err();
        assert_eq!(err.len(), 1, "a sense that is not the lemma's: {err:?}");
        placements.insert("responsible", "to", one("00000004"));
        let g = c.governance(&placements).unwrap();
        assert_eq!(g.of("00000005", "responsible"), ["for"]);
        assert_eq!(g.of("00000004", "responsible"), ["to"]);
        assert_eq!(g.of("00000001", "dependent"), ["on", "upon"]);
        assert!(g.of("00000002", "dependent").is_empty());
        assert_eq!(g.of("00000003", "proportional"), ["to"]);
        assert_eq!(g.of("00000006", "addicted"), ["to"]);
        assert!(g.of("00000007", "black").is_empty());
        assert!(g.restates_frame("dependent on"));
        assert!(!g.restates_frame("contingent on"));
        assert_eq!(g.judged, 5);
    }

    #[test]
    fn placements_parse_and_refuse_a_malformed_sense() {
        let p = Placements::parse("# resolved\nresponsible\tfor\t00000005,00000004\n").unwrap();
        assert_eq!(p.len(), 1);
        assert!(Placements::parse("responsible\tfor\ta0000005\n").is_err());
        assert!(Placements::parse("responsible\tfor\n").is_err());
    }
}
