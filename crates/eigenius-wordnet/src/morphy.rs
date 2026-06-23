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

//! Morphy — a faithful Rust port of WordNet's morphological processor
//! (`references/WordNet-3.0/lib/morph.c`, documented in `morphy(7WN)`). Maps an
//! inflected surface form to its base lemma(s), so the lexicon lookup that feeds
//! the parser can match entries keyed by lemma. Deterministic, no LLM.
//!
//! Faithful to `morph.c`: the `sufx`/`addr` detachment tables, the exception
//! lists (`pos.exc`), the `ful` / `ss` / ≤2-char noun guards, the 15-preposition
//! list, and the verb+preposition collocation handling (`morphprep`). Every
//! candidate is validated by `morph.c`'s `is_defined` — here, membership in the
//! imported [`LemmaSet`].
//!
//! Adaptations (idiomatic, not line-by-line): the stateful "call again with
//! NULL for more base forms" iterator becomes a returned `Vec<String>`; the file
//! /env plumbing becomes in-memory [`ExcLists`] + [`LemmaSet`]. Like `morph.c`,
//! it does NOT undo stem changes (consonant doubling: `running ↛ run`) — those
//! are the exception list's job.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::wndb::{Offset, Pos, Synset};

// Detachment rules (`morph.c` sufx[]/addr[]): noun [0,8), verb [8,16), adj [16,20).
const SUFX: [&str; 20] = [
    "s", "ses", "xes", "zes", "ches", "shes", "men", "ies", // noun
    "s", "ies", "es", "es", "ed", "ed", "ing", "ing", // verb
    "er", "est", "er", "est", // adj
];
const ADDR: [&str; 20] = [
    "", "s", "x", "z", "ch", "sh", "man", "y", // noun
    "", "y", "e", "", "e", "", "e", "", // verb
    "", "", "e", "e", // adj
];

fn rules(pos: Pos) -> std::ops::Range<usize> {
    match pos {
        Pos::Noun => 0..8,
        Pos::Verb => 8..16,
        Pos::Adj => 16..20,
        Pos::Adv => 0..0, // no detachment rules for adverbs (exception list only)
    }
}

/// `morph.c` preposition list — used to detect verb+preposition collocations.
const PREPOSITIONS: [&str; 15] = [
    "to", "at", "of", "on", "off", "in", "out", "up", "down", "from", "with", "into", "for",
    "about", "between",
];

/// Normalize to `morph.c`'s internal form: lowercase, spaces → underscores.
fn norm(s: &str) -> String {
    s.trim().to_lowercase().replace(' ', "_")
}

/// The lemma-membership oracle — `morph.c`'s `is_defined`: is `lemma`
/// (underscore-joined, lowercase) a real lemma in `pos`? Built from the imported
/// synsets' member words.
#[derive(Debug, Default)]
pub struct LemmaSet {
    by_pos: BTreeMap<Pos, BTreeSet<String>>,
}

impl LemmaSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, lemma: &str, pos: Pos) {
        self.by_pos.entry(pos).or_default().insert(norm(lemma));
    }

    pub fn contains(&self, lemma: &str, pos: Pos) -> bool {
        self.by_pos.get(&pos).is_some_and(|s| s.contains(lemma))
    }

    /// Build from parsed `data.<pos>` indices — every member lemma of every synset.
    pub fn from_synsets<'a>(
        indices: impl IntoIterator<Item = &'a BTreeMap<Offset, Synset>>,
    ) -> Self {
        let mut ls = Self::new();
        for idx in indices {
            for syn in idx.values() {
                for w in &syn.words {
                    ls.insert(w, syn.pos);
                }
            }
        }
        ls
    }
}

/// Exception lists: `inflected → base form(s)`, per POS (the `pos.exc` files).
#[derive(Debug, Default)]
pub struct ExcLists {
    by_pos: BTreeMap<Pos, BTreeMap<String, Vec<String>>>,
}

impl ExcLists {
    /// Parse the four exception-file texts. Each line is
    /// `inflected base [base...]`, whitespace-separated, multiword joined by `_`.
    pub fn parse(noun: &str, verb: &str, adj: &str, adv: &str) -> Self {
        let mut e = Self::default();
        for (pos, text) in [
            (Pos::Noun, noun),
            (Pos::Verb, verb),
            (Pos::Adj, adj),
            (Pos::Adv, adv),
        ] {
            e.by_pos.insert(pos, parse_exc(text));
        }
        e
    }

    /// Load `noun.exc` / `verb.exc` / `adj.exc` / `adv.exc` from a dict directory.
    pub fn load(dict_dir: &Path) -> std::io::Result<Self> {
        let rd = |f: &str| std::fs::read_to_string(dict_dir.join(f));
        Ok(Self::parse(
            &rd("noun.exc")?,
            &rd("verb.exc")?,
            &rd("adj.exc")?,
            &rd("adv.exc")?,
        ))
    }

    fn get(&self, pos: Pos, word: &str) -> Option<&Vec<String>> {
        self.by_pos.get(&pos).and_then(|m| m.get(word))
    }
}

fn parse_exc(text: &str) -> BTreeMap<String, Vec<String>> {
    let mut m = BTreeMap::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        if let Some(infl) = it.next() {
            let bases: Vec<String> = it.map(String::from).collect();
            if !bases.is_empty() {
                m.insert(infl.to_string(), bases);
            }
        }
    }
    m
}

/// Apply detachment rule `i` if `word`'s suffix matches (`morph.c` wordbase).
fn wordbase(word: &str, i: usize) -> Option<String> {
    let suf = SUFX[i];
    (word.len() > suf.len() && word.ends_with(suf))
        .then(|| format!("{}{}", &word[..word.len() - suf.len()], ADDR[i]))
}

/// Base form(s) of one word (`morph.c` morphword). Exception list is
/// authoritative; otherwise the detachment rules, each validated against
/// `lemmas`. (Returns all validated rule results, not just the first — a
/// completeness-favoring divergence harmless to the parser's chart.)
fn morphword(word: &str, pos: Pos, exc: &ExcLists, lemmas: &LemmaSet) -> Vec<String> {
    if let Some(bases) = exc.get(pos, word) {
        return bases.clone();
    }
    if pos == Pos::Adv {
        return Vec::new();
    }
    // `morph.c` noun guards: split off a trailing `ful`; skip `ss`-ending / ≤2-char.
    let (stem, end) = if pos == Pos::Noun && word.ends_with("ful") {
        (&word[..word.len() - 3], "ful")
    } else if pos == Pos::Noun && (word.ends_with("ss") || word.chars().count() <= 2) {
        return Vec::new();
    } else {
        (word, "")
    };
    let mut out = Vec::new();
    for i in rules(pos) {
        if let Some(base) = wordbase(stem, i) {
            let cand = format!("{base}{end}");
            if cand != word && lemmas.contains(&cand, pos) && !out.contains(&cand) {
                out.push(cand);
            }
        }
    }
    out
}

/// Word index (1-based, from word 2) of the first preposition, if any.
fn hasprep(words: &[&str]) -> bool {
    words.iter().skip(1).any(|w| PREPOSITIONS.contains(w))
}

/// Verb+preposition collocation (`morph.c` morphprep): morph the verb (first
/// word), keep the rest (preposition + …), validate. For >2 words also try
/// morphing the trailing noun.
fn morphprep(words: &[&str], exc: &ExcLists, lemmas: &LemmaSet) -> Vec<String> {
    let verb = words[0];
    if verb.is_empty() || !verb.chars().all(char::is_alphanumeric) {
        return Vec::new();
    }
    let rest = format!("_{}", words[1..].join("_"));
    let end: Option<String> = if words.len() > 2 {
        morphword(words[words.len() - 1], Pos::Noun, exc, lemmas)
            .into_iter()
            .next()
            .map(|lw| format!("_{}_{}", words[1..words.len() - 1].join("_"), lw))
    } else {
        None
    };

    let mut out = Vec::new();
    let try_base = |base: &str, out: &mut Vec<String>| {
        let r1 = format!("{base}{rest}");
        if lemmas.contains(&r1, Pos::Verb) {
            if !out.contains(&r1) {
                out.push(r1);
            }
            return;
        }
        if let Some(e) = &end {
            let r2 = format!("{base}{e}");
            if lemmas.contains(&r2, Pos::Verb) && !out.contains(&r2) {
                out.push(r2);
            }
        }
    };

    if let Some(bases) = exc.get(Pos::Verb, verb) {
        for b in bases {
            if b != verb {
                try_base(b, &mut out);
            }
        }
    }
    for i in rules(Pos::Verb) {
        if let Some(base) = wordbase(verb, i) {
            if base != verb {
                try_base(&base, &mut out);
            }
        }
    }
    out
}

/// Base lemma(s) of a word or collocation (`morph.c` morphstr). Returns
/// **space-joined** base forms (matching entry `form`s), de-duplicated.
pub fn morphstr(phrase: &str, pos: Pos, exc: &ExcLists, lemmas: &LemmaSet) -> Vec<String> {
    let s = norm(phrase);
    let words: Vec<&str> = s.split('_').collect();

    let mut out: Vec<String> = Vec::new();
    let push = |cand: &str, out: &mut Vec<String>| {
        let sp = cand.replace('_', " ");
        if !out.contains(&sp) {
            out.push(sp);
        }
    };

    // 1. exception list on the whole string (authoritative).
    if let Some(bases) = exc.get(pos, &s) {
        for b in bases {
            if *b != s {
                push(b, &mut out);
            }
        }
        if !out.is_empty() {
            return out;
        }
    }

    // 2. non-verb: morph the whole string (handles trailing inflection of a
    //    collocation, e.g. `cell_lines → cell_line`).
    if pos != Pos::Verb {
        for b in morphword(&s, pos, exc, lemmas) {
            push(&b, &mut out);
        }
        if !out.is_empty() {
            return out;
        }
    }

    // 3. verb + preposition collocation (`acts_on → act_on`).
    if pos == Pos::Verb && words.len() > 1 && hasprep(&words) {
        for b in morphprep(&words, exc, lemmas) {
            push(&b, &mut out);
        }
        return out;
    }

    // 4. otherwise morph each word, join, and validate (`took_a_breath →
    //    take_a_breath`; `attorneys_general → attorney_general`).
    let parts: Vec<String> = words
        .iter()
        .map(|w| {
            morphword(w, pos, exc, lemmas)
                .into_iter()
                .next()
                .unwrap_or_else(|| (*w).to_string())
        })
        .collect();
    let joined = parts.join("_");
    if joined != s && lemmas.contains(&joined, pos) {
        push(&joined, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> (ExcLists, LemmaSet) {
        let exc = ExcLists::parse(
            "mice mouse\nfeet foot\nattorneys_general attorney_general",
            "took take\nwent go",
            "",
            "",
        );
        let mut lemmas = LemmaSet::new();
        for n in [
            "mouse",
            "foot",
            "dog",
            "cell line",
            "breath",
            "attorney general",
        ] {
            lemmas.insert(n, Pos::Noun);
        }
        for v in ["take", "go", "depend", "act", "act on", "take a breath"] {
            lemmas.insert(v, Pos::Verb);
        }
        (exc, lemmas)
    }

    fn morph(s: &str, pos: Pos) -> Vec<String> {
        let (exc, lemmas) = fixtures();
        morphstr(s, pos, &exc, &lemmas)
    }

    #[test]
    fn exception_list_irregulars() {
        assert_eq!(morph("mice", Pos::Noun), ["mouse"]);
        assert_eq!(morph("feet", Pos::Noun), ["foot"]);
        assert_eq!(morph("took", Pos::Verb), ["take"]);
        assert_eq!(morph("went", Pos::Verb), ["go"]);
    }

    #[test]
    fn detachment_rules_regulars() {
        assert_eq!(morph("dogs", Pos::Noun), ["dog"]); // s → ""
        assert_eq!(morph("depended", Pos::Verb), ["depend"]); // ed → ""
                                                              // result must be a real lemma: "doges" → "doge"/"dog"? only "dog" is in
                                                              // the set, and only via "s"→"" on "dogs"; "frobs" has no lemma → empty.
        assert!(morph("frobs", Pos::Noun).is_empty());
    }

    #[test]
    fn collocation_trailing_inflection() {
        // whole-string rule on a compound noun.
        assert_eq!(morph("cell lines", Pos::Noun), ["cell line"]);
    }

    #[test]
    fn collocation_internal_inflection() {
        // per-word morph + join (irregular noun inside).
        assert_eq!(morph("attorneys general", Pos::Noun), ["attorney general"]);
        // verb collocation, no preposition.
        assert_eq!(morph("took a breath", Pos::Verb), ["take a breath"]);
    }

    #[test]
    fn verb_preposition_collocation() {
        // "acts on" → morph the verb, keep the preposition: "act on".
        assert_eq!(morph("acts on", Pos::Verb), ["act on"]);
    }

    #[test]
    fn no_undoubling_like_morph_c() {
        // Morphy does not undo consonant doubling; "running" ↛ "run".
        assert!(!morph("running", Pos::Verb).contains(&"run".to_string()));
    }

    #[test]
    fn parse_exc_handles_multiple_bases() {
        let m = parse_exc("ancones ancon ancone\nmice mouse");
        assert_eq!(m["ancones"], ["ancon", "ancone"]);
        assert_eq!(m["mice"], ["mouse"]);
    }

    /// Witness against the in-repo WordNet 3.0 corpus (real `.exc` + a `LemmaSet`
    /// built from `data.*`). Ignored by default — it reads ~15 MB.
    #[test]
    #[ignore = "loads the in-repo WordNet 3.0 corpus; run with --ignored"]
    fn morphs_against_the_real_corpus() {
        let dict = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../references/WordNet-3.0/dict");
        let exc = ExcLists::load(&dict).expect("load .exc");
        let noun = crate::wndb::read_data_file(&dict.join("data.noun")).unwrap();
        let verb = crate::wndb::read_data_file(&dict.join("data.verb")).unwrap();
        let adj = crate::wndb::read_data_file(&dict.join("data.adj")).unwrap();
        let lemmas = LemmaSet::from_synsets([&noun, &verb, &adj]);

        let m = |s: &str, p: Pos| morphstr(s, p, &exc, &lemmas);
        assert_eq!(m("mice", Pos::Noun), ["mouse"]); // noun.exc
        assert_eq!(m("geese", Pos::Noun), ["goose"]); // noun.exc
        assert_eq!(m("ran", Pos::Verb), ["run"]); // verb.exc
        assert!(m("dogs", Pos::Noun).contains(&"dog".to_string())); // rule s→""
        assert!(m("depends", Pos::Verb).contains(&"depend".to_string())); // rule s→""
                                                                          // is_defined gating: a non-word doesn't reduce to junk.
        assert!(m("frobnicates", Pos::Verb).is_empty());
    }
}
