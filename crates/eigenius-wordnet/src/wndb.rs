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

//! Parser for the WordNet `data.<pos>` flat files (`wndb(5WN)`) — the structural
//! source for the D62 §8.7 mapper. It captures the fields the mapper needs:
//! the synset's **lemmas + gloss**, its **`@` hypernyms** (→ `subclass_of`), its
//! **`@i` instance-hypernyms** (→ the NP individual archetype, kept distinct from
//! subclass edges), and — for verbs — its **sentence frames** (→ categories).
//!
//! Record format (after the optional `" | gloss"` split, the definition tokens):
//! `offset lex_filenum ss_type w_cnt (word lex_id)+ p_cnt (sym offset pos st)* [f_cnt (+ fnum wnum)*]`
//! Pointer records are 4 tokens each; the verb frame block (3 tokens each) follows
//! the pointer block — so the `+` that marks a frame is disambiguated from the `+`
//! derivational-pointer symbol by position.

use std::collections::BTreeMap;
use std::path::Path;

/// Part of speech. WordNet's satellite-adjective `s` folds into `Adj`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Pos {
    Noun,
    Verb,
    Adj,
    Adv,
}

impl Pos {
    pub fn from_ss_char(c: &str) -> Option<Pos> {
        match c {
            "n" => Some(Pos::Noun),
            "v" => Some(Pos::Verb),
            "a" | "s" => Some(Pos::Adj),
            "r" => Some(Pos::Adv),
            _ => None,
        }
    }

    /// The single-letter tag used in IRIs / sense keys (`n`, `v`, `a`, `r`).
    pub fn tag(self) -> char {
        match self {
            Pos::Noun => 'n',
            Pos::Verb => 'v',
            Pos::Adj => 'a',
            Pos::Adv => 'r',
        }
    }

    pub fn index_file(self) -> &'static str {
        match self {
            Pos::Noun => "index.noun",
            Pos::Verb => "index.verb",
            Pos::Adj => "index.adj",
            Pos::Adv => "index.adv",
        }
    }

    pub fn data_file(self) -> &'static str {
        match self {
            Pos::Noun => "data.noun",
            Pos::Verb => "data.verb",
            Pos::Adj => "data.adj",
            Pos::Adv => "data.adv",
        }
    }
}

/// An 8-digit synset offset (the within-file locator).
pub type Offset = String;

/// One synset, reduced to what the mapper consumes.
#[derive(Debug, Clone)]
pub struct Synset {
    pub offset: Offset,
    pub pos: Pos,
    /// Member lemmas, underscores normalized to spaces.
    pub words: Vec<String>,
    pub gloss: String,
    /// `@` hypernym targets (same-pos offsets) → `core:subclass_of` (the
    /// class-as-type lattice). Instance synsets (`@i`) are kept separate in
    /// [`Self::instance_of`] — an individual is not a subclass.
    pub hypernyms: Vec<Offset>,
    /// `@i` **instance-hypernym** targets — this synset is a proper-noun
    /// individual (e.g. *Einstein* `@i` *physicist*), not a class. Non-empty ⇒
    /// the synset maps to the NP archetype (an `EigonResource`, §8.7.3), typed
    /// at these classes; empty ⇒ a common-noun class.
    pub instance_of: Vec<Offset>,
    /// Verb sentence-frame numbers (empty for non-verbs).
    pub frames: Vec<u8>,
    /// True if this synset has a `\` **pertainym** pointer — i.e. it is a *relational*
    /// adjective ("atomic" \→ "atom"), which is **non-gradable** (D63 §8.12 6-cmp). A
    /// descriptive (gradable) adjective lacks it. (On adverbs `\` means "derived from
    /// adjective"; only consumed for adjectives in `push_adj`.)
    pub relational: bool,
    /// `+` **derivational** pointer targets `(offset, pos-char)` — morphosemantic links
    /// (adjective `dependent` → noun `dependence`). A gradable adjective's `deg` is projected
    /// onto its nominalization as a `cat_measure` reading (C2, d63-comparative-phrasal.md §5.3).
    pub derivational: Vec<(Offset, String)>,
}

/// Strip a WordNet **adjective syntactic marker** — `(a)` attributive, `(p)` predicative,
/// `(ip)` immediately-postnominal — from a `data.adj` lemma. The marker records the adjective's
/// syntactic *position*, not its lemma, so `respective(a)` is the lemma `respective`; leaving it on
/// pollutes the emitted `lexicon:form`/`sense` and breaks lookup. Markers occur only in `data.adj`;
/// other POS lemmas never carry a trailing `(a)`/`(p)`/`(ip)`, so this is a no-op there.
fn strip_adj_marker(word: &str) -> String {
    for marker in ["(a)", "(ip)", "(p)"] {
        if let Some(stripped) = word.strip_suffix(marker) {
            return stripped.to_string();
        }
    }
    word.to_string()
}

/// Parse one `data.<pos>` line, or `None` for the license preamble (which begins
/// with two spaces, not an 8-digit offset).
pub fn parse_data_line(line: &str) -> Option<Synset> {
    let b = line.as_bytes();
    if b.len() < 9 || !b[..8].iter().all(u8::is_ascii_digit) || b[8] != b' ' {
        return None;
    }
    let (defs, gloss) = match line.split_once(" | ") {
        Some((d, g)) => (d, g.trim().to_string()),
        None => (line, String::new()),
    };
    let tok: Vec<&str> = defs.split_whitespace().collect();

    let offset = tok.first()?.to_string();
    let pos = Pos::from_ss_char(tok.get(2)?)?;
    let w_cnt = usize::from_str_radix(tok.get(3)?, 16).ok()?;

    let mut i = 4;
    let mut words = Vec::with_capacity(w_cnt);
    for _ in 0..w_cnt {
        words.push(strip_adj_marker(&tok.get(i)?.replace('_', " ")));
        i += 2; // skip the lex_id following each word
    }

    let p_cnt: usize = tok.get(i)?.parse().ok()?;
    i += 1;
    let mut hypernyms = Vec::new();
    let mut instance_of = Vec::new();
    let mut relational = false;
    let mut derivational = Vec::new();
    for _ in 0..p_cnt {
        match *tok.get(i)? {
            "@" => hypernyms.push(tok.get(i + 1)?.to_string()),
            "@i" => instance_of.push(tok.get(i + 1)?.to_string()),
            "\\" => relational = true, // pertainym → relational (non-gradable) adjective
            // `+` derivational: (target offset, target pos-char) — the nominalization link.
            "+" => derivational.push((tok.get(i + 1)?.to_string(), tok.get(i + 2)?.to_string())),
            _ => {}
        }
        i += 4; // sym, offset, pos, source/target
    }

    let mut frames = Vec::new();
    if pos == Pos::Verb {
        if let Some(f_cnt) = tok.get(i).and_then(|s| s.parse::<usize>().ok()) {
            i += 1;
            for _ in 0..f_cnt {
                // each frame is `+ f_num w_num`
                if let Some(fnum) = tok.get(i + 1).and_then(|s| s.parse::<u8>().ok()) {
                    frames.push(fnum);
                }
                i += 3;
            }
        }
    }

    Some(Synset {
        offset,
        pos,
        words,
        gloss,
        hypernyms,
        instance_of,
        frames,
        relational,
        derivational,
    })
}

/// Parse a whole `data.<pos>` file into an offset → synset index.
pub fn read_data_file(path: &Path) -> std::io::Result<BTreeMap<Offset, Synset>> {
    let text = std::fs::read_to_string(path)?;
    let mut out = BTreeMap::new();
    for line in text.lines() {
        if let Some(s) = parse_data_line(line) {
            out.insert(s.offset.clone(), s);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real WordNet 3.1 `data.<pos>` lines.
    const ENTITY_N: &str = "00001740 03 n 01 entity 0 003 ~ 00001930 n 0000 ~ 00002137 n 0000 ~ 04431553 n 0000 | that which is perceived or known or inferred to have its own distinct existence  ";
    // Real gene synset (words + `@` hypernym), p_cnt trimmed to the 3 pointers shown.
    const GENE_N: &str = "05444328 08 n 03 gene 0 cistron 0 factor 0 003 @ 08476263 n 0000 #p 14854534 n 0000 #p 05449707 n 0000 | (genetics) a segment of DNA  ";
    const BREATHE_V: &str = "00001740 29 v 04 breathe 0 take_a_breath 0 respire 0 suspire 3 021 * 00005041 v 0000 * 00004227 v 0000 + 03121972 a 0301 + 00832852 n 0303 + 04087945 n 0301 + 04257960 n 0105 + 00832852 n 0101 ^ 00004227 v 0103 ^ 00005041 v 0103 $ 00002325 v 0000 $ 00002573 v 0000 ~ 00002573 v 0000 ~ 00002724 v 0000 ~ 00002942 v 0000 ~ 00003826 v 0000 ~ 00004032 v 0000 ~ 00004227 v 0000 ~ 00005041 v 0000 ~ 00006697 v 0000 ~ 00007328 v 0000 ~ 00017024 v 0000 02 + 02 00 + 08 00 | draw air into, and expel out of, the lungs";
    const EAT_V: &str = "00275082 30 v 03 corrode 1 eat 0 rust 1 007 @ 00259743 v 0000 + 14913630 n 0301 + 13573473 n 0301 + 00590069 a 0102 + 13474601 n 0101 + 13474601 n 0102 $ 00274762 v 0000 01 + 11 00 | cause to deteriorate";
    // Real instance synset: Einstein `@i` physicist (an individual, not a class);
    // the `+` is a derivational pointer, not a hypernym.
    const EINSTEIN_N: &str = "10954498 18 n 02 Einstein 0 Albert_Einstein 0 002 @i 10428004 n 0000 + 03031247 a 0301 | physicist born in Germany who formulated the theory of relativity (1879-1955)  ";
    const PREAMBLE: &str = "  1 This software and database is being provided to you  ";

    // Real adjective satellite: `respective(a)` / `several(a)` / `various(a)` — each carries the
    // attributive `(a)` syntactic marker, which is NOT part of the lemma.
    const RESPECTIVE_A: &str =
        "00494409 00 s 03 respective(a) 0 several(a) 0 various(a) 0 001 & 00493460 a 0000 | ";

    #[test]
    fn adjective_syntactic_markers_are_stripped_from_lemmas() {
        let s = parse_data_line(RESPECTIVE_A).unwrap();
        assert_eq!(s.pos, Pos::Adj);
        // (a) attributive markers stripped → clean lemmas usable as `lexicon:form`.
        assert_eq!(s.words, ["respective", "several", "various"]);
    }

    #[test]
    fn strip_adj_marker_handles_each_position_marker_and_leaves_others() {
        assert_eq!(strip_adj_marker("respective(a)"), "respective");
        assert_eq!(strip_adj_marker("asleep(p)"), "asleep");
        assert_eq!(strip_adj_marker("elect(ip)"), "elect");
        // No marker / other POS lemmas are untouched (no trailing (a)/(p)/(ip)).
        assert_eq!(strip_adj_marker("common"), "common");
        assert_eq!(strip_adj_marker("gene"), "gene");
    }

    #[test]
    fn noun_root_has_no_hypernym() {
        let s = parse_data_line(ENTITY_N).unwrap();
        assert_eq!(s.offset, "00001740");
        assert_eq!(s.pos, Pos::Noun);
        assert_eq!(s.words, ["entity"]);
        assert!(s.hypernyms.is_empty(), "entity.n.01 is the root");
        assert!(s.gloss.starts_with("that which is perceived"));
    }

    #[test]
    fn noun_hypernym_extracted() {
        let s = parse_data_line(GENE_N).unwrap();
        assert_eq!(s.words, ["gene", "cistron", "factor"]);
        // `@ 08476263` is the hypernym; `#p` meronyms are NOT subclass edges.
        assert_eq!(s.hypernyms, ["08476263"]);
        assert!(s.instance_of.is_empty()); // a common noun, not an individual
    }

    #[test]
    fn instance_hypernym_separated_from_subclass() {
        let s = parse_data_line(EINSTEIN_N).unwrap();
        assert_eq!(s.words, ["Einstein", "Albert Einstein"]);
        // `@i` is an instance edge (the NP individual archetype), NOT a subclass.
        assert!(s.hypernyms.is_empty(), "@i must not land in hypernyms");
        assert_eq!(s.instance_of, ["10428004"]); // @i → physicist.n.01
    }

    #[test]
    fn verb_frames_after_the_pointer_block() {
        let s = parse_data_line(BREATHE_V).unwrap();
        assert_eq!(s.pos, Pos::Verb);
        assert_eq!(s.words, ["breathe", "take a breath", "respire", "suspire"]);
        // frame 2 = "Somebody ----s" (intransitive), 8 = "Somebody ----s something".
        assert_eq!(s.frames, [2, 8]);
        assert!(s.hypernyms.is_empty()); // breathe has no `@` hypernym in this synset
    }

    #[test]
    fn verb_hypernym_and_frame() {
        let s = parse_data_line(EAT_V).unwrap();
        assert_eq!(s.hypernyms, ["00259743"]); // `@` (troponymy)
        assert_eq!(s.frames, [11]); // "Something ----s something" (transitive)
    }

    #[test]
    fn preamble_lines_are_skipped() {
        assert!(parse_data_line(PREAMBLE).is_none());
        assert!(parse_data_line("").is_none());
    }
}
