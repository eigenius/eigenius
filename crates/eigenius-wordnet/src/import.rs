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

//! Synset **selection** for the WordNet import (D62 §8.7 / D63 §8.7 Slice 7).
//!
//! Turns a [`SeedSpec`] (the `--seed`/`--limit`/`--all` flags, or a harness's
//! equivalent) into the set of [`Synset`]s to render — **closed under hypernymy**
//! (every `@`/`@i` ancestor + the `entity.n.01` root present) so the emitted
//! `subclass_of` lattice is rooted and self-consistent at any size. Shared by the
//! `wordnet-import` binary and the scale-up harness so both produce the *same*
//! synset set from the same spec.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use crate::convert::SenseRanks;
use crate::wndb::{read_data_file, Offset, Pos, Synset};

/// `entity.n.01` — the noun-lattice root; always pulled in so verb/adjective
/// argument slots (typed at the noun root) resolve.
pub const ENTITY_ROOT_OFFSET: &str = "00001740";

/// What to import: a bound (`all` / `limit` / `seeds`) over a set of POS. Mirrors
/// the `wordnet-import` CLI flags so the binary and a harness share one selector.
#[derive(Debug, Clone, Default)]
pub struct SeedSpec {
    /// Import ALL synsets of the requested POS (the full lexicon).
    pub all: bool,
    /// Cap the per-POS seed set to the first N synsets (then closed). Bounded.
    pub limit: Option<usize>,
    /// Seed lemma(s): import their synsets + the noun hypernym closure.
    pub seeds: Vec<String>,
    /// POS to import.
    pub pos: Vec<Pos>,
}

impl SeedSpec {
    /// A bounded spec: the first `n` synsets per POS (noun/verb/adj), closed.
    pub fn limit(n: usize) -> Self {
        Self {
            all: false,
            limit: Some(n),
            seeds: Vec::new(),
            pos: vec![Pos::Noun, Pos::Verb, Pos::Adj],
        }
    }

    /// A seeded spec: the given lemmas + the noun hypernym closure, over
    /// noun/verb/adj — controlled real-WordNet vocabulary for a battery.
    pub fn seeded<I, S>(seeds: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            all: false,
            limit: None,
            seeds: seeds.into_iter().map(Into::into).collect(),
            pos: vec![Pos::Noun, Pos::Verb, Pos::Adj],
        }
    }

    /// Whether at least one bound is set (else the selection is empty — an error
    /// for the CLI).
    pub fn is_bounded(&self) -> bool {
        self.all || self.limit.is_some() || !self.seeds.is_empty()
    }
}

/// Reflexive-transitive closure of `seeds` over the noun index along **both** `@`
/// hypernyms and `@i` instance-hypernyms — so an individual drags in the class(es)
/// it instantiates, and every class climbs to the `entity.n.01` root.
fn close_nouns(seeds: &BTreeSet<Offset>, noun: &BTreeMap<Offset, Synset>) -> BTreeSet<Offset> {
    let mut set = BTreeSet::new();
    let mut stack: Vec<Offset> = seeds.iter().cloned().collect();
    while let Some(o) = stack.pop() {
        if !set.insert(o.clone()) {
            continue;
        }
        if let Some(s) = noun.get(&o) {
            stack.extend(s.hypernyms.iter().cloned());
            stack.extend(s.instance_of.iter().cloned());
        }
    }
    set
}

/// Select seed offsets for one POS index per the spec (`all` ∪ `limit` ∪ `seeds`).
fn select_seeds(index: &BTreeMap<Offset, Synset>, spec: &SeedSpec) -> BTreeSet<Offset> {
    let mut seeds = BTreeSet::new();
    if spec.all {
        seeds.extend(index.keys().cloned());
    }
    if let Some(n) = spec.limit {
        seeds.extend(index.keys().take(n).cloned());
    }
    if !spec.seeds.is_empty() {
        let want: BTreeSet<&str> = spec.seeds.iter().map(String::as_str).collect();
        for (off, syn) in index {
            if syn.words.iter().any(|w| want.contains(w.as_str())) {
                seeds.insert(off.clone());
            }
        }
    }
    seeds
}

/// Read each requested POS's `index.<pos>` and build the sense-frequency ranks
/// (D63 §8.7 Stage B): WordNet lists a lemma's synsets in `index.<pos>` in decreasing
/// frequency (sense 1 = most frequent), so the i-th offset gets rank `i` (0-based). The
/// key is the entry `sense` (`wn:{lemma}.{tag}.{offset}`), matching
/// [`crate::convert::render_document`]'s lookup. Unranked lemmas (and the
/// case-mismatched proper nouns, which `index` lowercases) simply default to rank 0.
pub fn read_sense_ranks(dict: &Path, pos_set: &[Pos]) -> std::io::Result<SenseRanks> {
    let mut ranks = SenseRanks::new();
    for &pos in pos_set {
        let content = match fs::read_to_string(dict.join(pos.index_file())) {
            Ok(c) => c,
            Err(_) => continue, // a missing index for a POS is non-fatal (ranks stay 0)
        };
        for line in content.lines() {
            // The license preamble lines begin with two spaces, not a lemma.
            if line.starts_with("  ") {
                continue;
            }
            let tok: Vec<&str> = line.split_whitespace().collect();
            // `lemma pos synset_cnt p_cnt [ptrs…] sense_cnt tagsense_cnt offset…`
            let Some(syn_cnt) = tok.get(2).and_then(|n| n.parse::<usize>().ok()) else {
                continue;
            };
            if syn_cnt == 0 || tok.len() < syn_cnt {
                continue;
            }
            let lemma = tok[0];
            // The synset offsets are the trailing `syn_cnt` tokens, in sense order.
            for (i, off) in tok[tok.len() - syn_cnt..].iter().enumerate() {
                ranks.insert(format!("wn:{lemma}.{}.{off}", pos.tag()), i as u32);
            }
        }
    }
    Ok(ranks)
}

/// Read the dict and gather the synsets to render for `spec` — noun selection
/// closed under hypernymy, the `entity.n.01` root added, and verb/adjective
/// synsets at their seeds (typed at the noun root). The result is the input to
/// [`crate::convert::render_document`].
pub fn select_synsets(dict: &Path, spec: &SeedSpec) -> std::io::Result<Vec<Synset>> {
    // The noun index is always needed (closure + entity root for verb/adj typing).
    let noun = read_data_file(&dict.join(Pos::Noun.data_file()))?;
    let load = |p: Pos| -> std::io::Result<BTreeMap<Offset, Synset>> {
        if spec.pos.contains(&p) {
            read_data_file(&dict.join(p.data_file()))
        } else {
            Ok(BTreeMap::new())
        }
    };
    let verb = load(Pos::Verb)?;
    let adj = load(Pos::Adj)?;

    let mut chosen: Vec<Synset> = Vec::new();
    if spec.pos.contains(&Pos::Noun) {
        let seeds = select_seeds(&noun, spec);
        let mut closed = close_nouns(&seeds, &noun);
        closed.insert(ENTITY_ROOT_OFFSET.to_string());
        chosen.extend(closed.iter().filter_map(|o| noun.get(o).cloned()));
    } else if !verb.is_empty() || !adj.is_empty() {
        // verbs/adjs type at the noun root → it must be present even if nouns
        // weren't requested.
        if let Some(root) = noun.get(ENTITY_ROOT_OFFSET) {
            chosen.push(root.clone());
        }
    }
    for (index, p) in [(&verb, Pos::Verb), (&adj, Pos::Adj)] {
        if spec.pos.contains(&p) {
            let seeds = select_seeds(index, spec);
            chosen.extend(seeds.iter().filter_map(|o| index.get(o).cloned()));
        }
    }
    Ok(chosen)
}
