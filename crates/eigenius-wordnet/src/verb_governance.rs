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

//! D97 slice 2 — the complements a verb sense takes beside WordNet's frames.
//!
//! SPECIALIST's `tran=` fields name, for a verb lemma, an object, PP arguments with their
//! prepositions and a finite clause ([`VerbComplement`]). The importer emits per WordNet sense, so
//! each lemma-level complement is placed on senses (D97 decision 1, revised for verbs 2026-10-01):
//! - a preposition: on the lemma's one sense, else on the senses the judge accepted ([`Placements`],
//!   resolved from its committed verdicts in `verb-senses.tsv`);
//! - an object or a clause: the same, but only where WordNet gives no sense of the lemma a frame of
//!   that kind. Where it gives some sense one, it has placed the complement per sense, and those
//!   items score the judge instead ([`VerbClassified::gold`]).
//!
//! WordNet's own frames stay beside these (decision 2, union), and its frames that name a
//! preposition are named relations: 12 and 27 `to`, 13 `on`. A SPECIALIST preposition placed on a
//! (sense, lemma) replaces WordNet's any-preposition frame (4, 22) there (decision 2, refined
//! 2026-10-01); [`crate::convert`] applies that.

use std::collections::{BTreeMap, BTreeSet};

use eigenius_kernel::dcg::category::prep_constructor;
pub use eigenius_specialist::VerbComplement;

use crate::convert::{classify, FrameKind};
use crate::governance::Placements;
#[cfg(test)]
use crate::wndb::Frame;
use crate::wndb::{Offset, Synset};

/// WordNet 3.0's verb frames (`dict/frames.vrb`), frame `n` at index `n - 1`.
const FRAMES: [&str; 35] = [
    "Something ----s",
    "Somebody ----s",
    "It is ----ing",
    "Something is ----ing PP",
    "Something ----s something Adjective/Noun",
    "Something ----s Adjective/Noun",
    "Somebody ----s Adjective",
    "Somebody ----s something",
    "Somebody ----s somebody",
    "Something ----s somebody",
    "Something ----s something",
    "Something ----s to somebody",
    "Somebody ----s on something",
    "Somebody ----s somebody something",
    "Somebody ----s something to somebody",
    "Somebody ----s something from somebody",
    "Somebody ----s somebody with something",
    "Somebody ----s somebody of something",
    "Somebody ----s something on somebody",
    "Somebody ----s somebody PP",
    "Somebody ----s something PP",
    "Somebody ----s PP",
    "Somebody's (body part) ----s",
    "Somebody ----s somebody to INFINITIVE",
    "Somebody ----s somebody INFINITIVE",
    "Somebody ----s that CLAUSE",
    "Somebody ----s to somebody",
    "Somebody ----s to INFINITIVE",
    "Somebody ----s whether INFINITIVE",
    "Somebody ----s somebody into V-ing something",
    "Somebody ----s something with something",
    "Somebody ----s INFINITIVE",
    "Somebody ----s VERB-ing",
    "It ----s that CLAUSE",
    "Something ----s INFINITIVE",
];

/// A complement as the placements file and the judge's verdicts name it: the preposition, `object`
/// or `clause`.
pub fn label(complement: &VerbComplement) -> &str {
    match complement {
        VerbComplement::Object => "object",
        VerbComplement::Clause => "clause",
        VerbComplement::Pp(p) => p,
    }
}

/// The complement [`label`] names.
pub fn from_label(label: &str) -> VerbComplement {
    match label {
        "object" => VerbComplement::Object,
        "clause" => VerbComplement::Clause,
        p => VerbComplement::Pp(p.to_string()),
    }
}

/// The complement WordNet's `frame` gives a sense, as the importer reads the frame. `None` for the
/// frames that give none of [`VerbComplement`]'s kinds and for the any-preposition frames (4, 22),
/// which name no preposition.
fn frame_gives(frame: u8) -> Option<VerbComplement> {
    match classify(frame)? {
        FrameKind::Transitive => Some(VerbComplement::Object),
        FrameKind::PpOblique(Some(p)) => Some(VerbComplement::Pp(p.to_string())),
        FrameKind::Clausal => Some(VerbComplement::Clause),
        _ => None,
    }
}

/// Whether WordNet's `frame` shows a complement of `complement`'s kind: a noun phrase right after
/// the verb (`----s something`, `----s somebody something`), a PP (`----s PP`, `----s to somebody`,
/// `----s somebody PP`), a that-clause.
fn of_kind(frame: u8, complement: &VerbComplement) -> bool {
    let Some(text) = usize::from(frame)
        .checked_sub(1)
        .and_then(|i| FRAMES.get(i))
    else {
        return false;
    };
    match complement {
        VerbComplement::Object => {
            text.contains("----s something") || text.contains("----s somebody")
        }
        VerbComplement::Pp(_) => {
            text.ends_with(" PP") || text.contains("----s to ") || text.contains("----s on ")
        }
        VerbComplement::Clause => text.contains("that CLAUSE"),
    }
}

/// The patterns the judge is shown for `lemma` in a sense when it decides `complement`: the frames
/// WordNet gives the lemma there ([`Synset::frames_of_lemma`]), in number order, except those that
/// show a complement of its kind — the gold is read from those.
pub fn shown_frames(syn: &Synset, lemma: &str, complement: &VerbComplement) -> Vec<&'static str> {
    syn.frames_of_lemma(lemma)
        .into_iter()
        .filter(|&f| !of_kind(f, complement))
        .filter_map(|f| FRAMES.get(usize::from(f).checked_sub(1)?).copied())
        .collect()
}

/// A lemma-level complement on a lemma with one verb sense, placed there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecidedVerbItem {
    pub lemma: String,
    pub complement: VerbComplement,
    pub sense: Offset,
}

/// A lemma-level complement on a lemma with several verb senses: the judge's item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenVerbItem {
    pub lemma: String,
    pub complement: VerbComplement,
    /// The lemma's verb senses, in offset order.
    pub senses: Vec<Offset>,
}

/// An item the judge is scored on: the senses WordNet's own frames give the complement, which it
/// must recover with those frames hidden ([`shown_frames`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoldVerbItem {
    pub item: OpenVerbItem,
    pub gold: BTreeSet<Offset>,
}

/// Items of one kind: placed on the lemma's one sense, or open for the judge.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KindCounts {
    pub one_sense: usize,
    pub open: usize,
    /// Objects and clauses WordNet already gives some sense of the lemma.
    pub met_by_wordnet: usize,
}

/// What [`classify_verbs`] counted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerbCounts {
    /// WordNet verb lemmas SPECIALIST names a complement for.
    pub lemmas_attested: usize,
    /// By kind: `preposition`, `object`, `clause`.
    pub by_kind: BTreeMap<&'static str, KindCounts>,
    /// Complements naming a preposition outside `lexicon:Prep`, by preposition.
    pub outside: BTreeMap<String, usize>,
}

fn kind_name(complement: &VerbComplement) -> &'static str {
    match complement {
        VerbComplement::Object => "object",
        VerbComplement::Clause => "clause",
        VerbComplement::Pp(_) => "preposition",
    }
}

/// Every verb lemma's SPECIALIST complements, sorted into the items placed on a lemma's one sense,
/// the items the judge places, and the gold the judge is scored on.
#[derive(Debug, Clone, Default)]
pub struct VerbClassified {
    pub decided: Vec<DecidedVerbItem>,
    pub open: Vec<OpenVerbItem>,
    /// Open PP items whose senses WordNet's named frames give the preposition (`to`, `on`), and the
    /// objects and clauses WordNet gives some senses of the lemma but not all.
    pub gold: Vec<GoldVerbItem>,
    pub counts: VerbCounts,
}

/// Sort every WordNet verb lemma's SPECIALIST complements. `verbs` is all of WordNet's verb synsets,
/// so a lemma's senses are complete whatever the import selects; a multiword lemma is looked up as
/// written (`give rise`).
pub fn classify_verbs(
    verbs: &BTreeMap<Offset, Synset>,
    specialist: &eigenius_specialist::Lexicon,
) -> VerbClassified {
    let mut c = VerbClassified::default();
    let mut senses: BTreeMap<String, Vec<Offset>> = BTreeMap::new();
    for syn in verbs.values() {
        for lemma in &syn.words {
            let all = senses.entry(lemma.to_lowercase()).or_default();
            if all.last() != Some(&syn.offset) {
                all.push(syn.offset.clone());
            }
        }
    }
    for (lemma, all) in &senses {
        let complements: BTreeSet<VerbComplement> = specialist
            .lookup(lemma, "verb")
            .flat_map(|rec| rec.verb_complements())
            .collect();
        if complements.is_empty() {
            continue;
        }
        c.counts.lemmas_attested += 1;
        for complement in complements {
            if let VerbComplement::Pp(p) = &complement {
                if prep_constructor(p).is_none() {
                    *c.counts.outside.entry(p.clone()).or_default() += 1;
                    continue;
                }
            }
            let kind = c.counts.by_kind.entry(kind_name(&complement)).or_default();
            let given: BTreeSet<Offset> = all
                .iter()
                .filter(|o| {
                    verbs[*o]
                        .frames_of_lemma(lemma)
                        .into_iter()
                        .any(|f| frame_gives(f).as_ref() == Some(&complement))
                })
                .cloned()
                .collect();
            let item = OpenVerbItem {
                lemma: lemma.clone(),
                complement: complement.clone(),
                senses: all.clone(),
            };
            if !matches!(complement, VerbComplement::Pp(_)) && !given.is_empty() {
                kind.met_by_wordnet += 1;
                if given.len() < all.len() {
                    c.gold.push(GoldVerbItem { item, gold: given });
                }
                continue;
            }
            if all.len() == 1 {
                kind.one_sense += 1;
                c.decided.push(DecidedVerbItem {
                    lemma: lemma.clone(),
                    complement,
                    sense: all[0].clone(),
                });
                continue;
            }
            kind.open += 1;
            if !given.is_empty() {
                c.gold.push(GoldVerbItem {
                    item: item.clone(),
                    gold: given,
                });
            }
            c.open.push(item);
        }
    }
    c
}

impl VerbClassified {
    /// The verb governance: the decided items, and the open items placed from `placements`. `Err`
    /// lists the open items `placements` does not place, or places on a sense that is not the
    /// lemma's — the import stops rather than guess.
    pub fn governance(&self, placements: &Placements) -> Result<VerbGovernance, Vec<String>> {
        let mut g = VerbGovernance::default();
        for item in &self.decided {
            g.by_sense
                .entry((item.sense.clone(), item.lemma.clone()))
                .or_default()
                .insert(item.complement.clone());
        }
        let mut problems = Vec::new();
        for item in &self.open {
            let name = label(&item.complement);
            match placements.get(&item.lemma, name) {
                None => problems.push(format!(
                    "{} {name}: no placement ({} senses)",
                    item.lemma,
                    item.senses.len()
                )),
                Some(placed) if !placed.iter().all(|s| item.senses.contains(s)) => {
                    problems.push(format!(
                        "{} {name}: placed on a sense that is not the lemma's",
                        item.lemma
                    ))
                }
                Some(placed) => {
                    g.judged += 1;
                    g.gaps += usize::from(placed.is_empty());
                    for off in placed {
                        g.by_sense
                            .entry((off.clone(), item.lemma.clone()))
                            .or_default()
                            .insert(item.complement.clone());
                    }
                }
            }
        }
        if problems.is_empty() {
            Ok(g)
        } else {
            Err(problems)
        }
    }
}

/// The SPECIALIST complements placed on each (verb sense, lemma).
#[derive(Debug, Clone, Default)]
pub struct VerbGovernance {
    by_sense: BTreeMap<(Offset, String), BTreeSet<VerbComplement>>,
    /// Open items the judge's placements placed.
    pub judged: usize,
    /// Of those, the gaps: placed on no sense.
    pub gaps: usize,
}

impl VerbGovernance {
    /// The complements placed on `lemma` in the sense `offset`.
    pub fn of(&self, offset: &str, lemma: &str) -> BTreeSet<VerbComplement> {
        self.by_sense
            .get(&(offset.to_string(), lemma.to_lowercase()))
            .cloned()
            .unwrap_or_default()
    }

    /// (sense, lemma) pairs with a placed complement, and the complements over all of them.
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

    fn verb(offset: &str, words: &[&str], frames: &[u8], gloss: &str) -> Synset {
        let frames = frames.iter().map(|&f| Frame::all(f)).collect();
        Synset {
            offset: offset.to_string(),
            pos: crate::wndb::Pos::Verb,
            words: words.iter().map(|w| w.to_string()).collect(),
            gloss: gloss.to_string(),
            hypernyms: vec![],
            instance_of: vec![],
            frames,
            relational: false,
            derivational: BTreeSet::new(),
        }
    }

    const SPECIALIST: &str = "{base=respond
entry=E1
\tcat=verb
\tintran
\ttran=pphr(to,np);nopass
\ttran=pphr(with,np);nopass
\ttran=fincomp(t);nopass
}
{base=incubate
entry=E2
\tcat=verb
\ttran=np
}
{base=depend
entry=E3
\tcat=verb
\ttran=pphr(on,np)
}
{base=report
entry=E4
\tcat=verb
\ttran=np
\ttran=fincomp(t)
}
";

    fn fixture() -> BTreeMap<Offset, Synset> {
        [
            // `respond`: two senses; the first has WordNet's `to` (27) and the any frame (22).
            verb("00000001", &["respond"], &[2, 22, 27], "show a reaction"),
            verb(
                "00000002",
                &["respond"],
                &[1, 2],
                "respond favorably; `the cancer responded to the therapy'",
            ),
            // `incubate`: two senses, neither transitive in WordNet: an open object item.
            verb("00000003", &["incubate"], &[1], "grow under conditions"),
            verb("00000004", &["incubate"], &[2], "sit on eggs"),
            // `depend`: one sense: decided.
            verb("00000005", &["depend"], &[22], "be contingent on"),
            // `report`: WordNet gives one of two senses an object and a clause: gold, not open.
            verb("00000006", &["report"], &[8, 26], "announce as the result"),
            verb("00000007", &["report"], &[2], "present oneself"),
        ]
        .into_iter()
        .map(|s| (s.offset.clone(), s))
        .collect()
    }

    fn classified() -> VerbClassified {
        let lex = eigenius_specialist::Lexicon::parse(SPECIALIST).unwrap();
        classify_verbs(&fixture(), &lex)
    }

    #[test]
    fn a_complement_is_decided_open_or_met_by_wordnet() {
        let c = classified();
        let decided: Vec<(&str, &str, &str)> = c
            .decided
            .iter()
            .map(|d| (d.lemma.as_str(), label(&d.complement), d.sense.as_str()))
            .collect();
        assert_eq!(decided, [("depend", "on", "00000005")]);
        let open: Vec<(&str, &str)> = c
            .open
            .iter()
            .map(|o| (o.lemma.as_str(), label(&o.complement)))
            .collect();
        assert_eq!(
            open,
            [
                ("incubate", "object"),
                ("respond", "to"),
                ("respond", "with"),
                ("respond", "clause"),
            ]
        );
        let gold: Vec<(&str, &str, Vec<&str>)> = c
            .gold
            .iter()
            .map(|g| {
                (
                    g.item.lemma.as_str(),
                    label(&g.item.complement),
                    g.gold.iter().map(String::as_str).collect(),
                )
            })
            .collect();
        assert_eq!(
            gold,
            [
                ("report", "object", vec!["00000006"]),
                ("report", "clause", vec!["00000006"]),
                ("respond", "to", vec!["00000001"]),
            ]
        );
        assert_eq!(c.counts.by_kind["object"].met_by_wordnet, 1);
        assert_eq!(c.counts.by_kind["preposition"].open, 2);
    }

    #[test]
    fn the_judge_is_not_shown_the_frames_of_the_kind_it_decides() {
        let syn = verb("00000001", &["respond"], &[2, 22, 27, 8], "");
        assert_eq!(
            shown_frames(&syn, "respond", &VerbComplement::Pp("to".into())),
            ["Somebody ----s", "Somebody ----s something"]
        );
        assert_eq!(
            shown_frames(&syn, "respond", &VerbComplement::Object),
            [
                "Somebody ----s",
                "Somebody ----s PP",
                "Somebody ----s to somebody"
            ]
        );
        // Every frame with a noun phrase after the verb is hidden for an object, a ditransitive's
        // too; a PP after an object is hidden for a preposition.
        let give = verb("00000002", &["give"], &[8, 14, 21, 26], "");
        assert_eq!(
            shown_frames(&give, "give", &VerbComplement::Object),
            ["Somebody ----s that CLAUSE"]
        );
        assert_eq!(
            shown_frames(&give, "give", &VerbComplement::Pp("to".into())),
            [
                "Somebody ----s something",
                "Somebody ----s somebody something",
                "Somebody ----s that CLAUSE"
            ]
        );
    }

    #[test]
    fn an_open_item_without_a_placement_stops_the_import() {
        let c = classified();
        assert_eq!(c.governance(&Placements::default()).unwrap_err().len(), 4);
        let mut placements = Placements::default();
        let one = |o: &str| BTreeSet::from([o.to_string()]);
        placements.insert("incubate", "object", one("00000003"));
        placements.insert("respond", "to", one("00000009"));
        placements.insert("respond", "with", BTreeSet::new());
        placements.insert("respond", "clause", one("00000001"));
        assert_eq!(c.governance(&placements).unwrap_err().len(), 1);
        placements.insert(
            "respond",
            "to",
            BTreeSet::from(["00000001".to_string(), "00000002".to_string()]),
        );
        let g = c.governance(&placements).unwrap();
        let pp = |p: &str| VerbComplement::Pp(p.into());
        assert_eq!(g.of("00000002", "respond"), BTreeSet::from([pp("to")]));
        assert_eq!(
            g.of("00000001", "respond"),
            BTreeSet::from([pp("to"), VerbComplement::Clause])
        );
        assert_eq!(g.of("00000005", "depend"), BTreeSet::from([pp("on")]));
        assert_eq!(
            g.of("00000003", "incubate"),
            BTreeSet::from([VerbComplement::Object])
        );
        assert!(g.of("00000004", "incubate").is_empty());
        assert_eq!((g.judged, g.gaps), (4, 1));
    }

    /// WordNet gives frame 8 in 00630380 to `chew over` only: SPECIALIST's object for `reflect` is
    /// not met there, so the item is open, and `chew over`'s is met.
    #[test]
    fn a_frame_restricted_to_another_word_does_not_give_the_complement() {
        let mut ponder = verb(
            "00630380",
            &["chew over", "reflect"],
            &[22],
            "reflect deeply",
        );
        ponder.frames.push(Frame { number: 8, word: 1 });
        let other = verb("00000009", &["reflect"], &[1], "throw back light");
        let verbs: BTreeMap<Offset, Synset> = [ponder, other]
            .into_iter()
            .map(|s| (s.offset.clone(), s))
            .collect();
        let lex = eigenius_specialist::Lexicon::parse(
            "{base=reflect\nentry=E5\n\tcat=verb\n\ttran=np\n}\n{base=chew over\nentry=E6\n\tcat=verb\n\ttran=np\n}\n",
        )
        .unwrap();
        let c = classify_verbs(&verbs, &lex);
        let open: Vec<(&str, &str)> = c
            .open
            .iter()
            .map(|o| (o.lemma.as_str(), label(&o.complement)))
            .collect();
        assert_eq!(open, [("reflect", "object")]);
        assert_eq!(c.counts.by_kind["object"].met_by_wordnet, 1, "chew over");
        assert_eq!(
            shown_frames(
                &verbs["00630380"],
                "reflect",
                &VerbComplement::Pp("on".into())
            ),
            Vec::<&str>::new()
        );
        assert_eq!(
            shown_frames(
                &verbs["00630380"],
                "chew over",
                &VerbComplement::Pp("on".into())
            ),
            ["Somebody ----s something"]
        );
    }

    #[test]
    fn labels_round_trip() {
        for c in [
            VerbComplement::Object,
            VerbComplement::Clause,
            VerbComplement::Pp("out of".into()),
        ] {
            assert_eq!(from_label(label(&c)), c);
        }
    }
}
