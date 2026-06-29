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

//! The lookup bridge (D62 §8.8.1): a surface string → the forest of typed
//! parses. It joins the three pieces already built — the [`Lemmatizer`] seam
//! ([`super::lemmatizer`]), the committed lexicon (`lexicon:LexicalEntry`
//! resources in a layer), and the CKY composition parser ([`super::parser`]) —
//! into the kernel-attached `string → tree(s)` library:
//!
//! 1. **tokenize** the input;
//! 2. **seed** the chart: for every token span (bounded by the longest multiword
//!    form), reduce the surface to candidate lemmas via the [`Lemmatizer`] and
//!    look them up in the [`LexicalIndex`] — so a multiword entry (`cell line`,
//!    `act on`) seeds a multi-token span *alongside* the single-token items for
//!    its parts (the MWE-vs-compositional ambiguity, §8.4, carried as competing
//!    chart edges, not resolved here);
//! 3. **compose** with CKY over the seeded chart ([`super::parser::apply`]);
//! 4. **filter** to every full-span `S` parse whose assembled sem type-checks to
//!    `Prop` — the kernel as the felicity oracle.
//!
//! The library returns the WHOLE forest (no selection, no commit). Selecting one
//! parse and committing it as a `lexicon:Sentence` is the encoding institution's
//! job (§8.8.2–8.8.3); an empty forest is a first-class outcome (no admissible
//! parse), not an error.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use crate::layer::{normalize_value, resolve_active_value_indexes, Layer};
use crate::nbe::check::{check, exp_mentions_var, CheckCtx};
use crate::nbe::env::{Gamma, Rho};
use crate::nbe::eval::eval;
use crate::nbe::readback::readback_val;
use crate::nbe::term::{Exp, Patt};
use crate::nbe::val::{Neut, Val};
use crate::ontology::resource::Value;
use crate::ontology::Iri;

use super::category::{
    adverb_modifier_cats, cats_coordinate, coordinate_np, coordinate_sem, denote_cat, is_ctor,
    reciprocate, relativize, sentence_modifier_cats, type_raise,
};
use super::lemmatizer::{Lemmatizer, Pos};
use super::lexicon::entry_to_item;
use super::parser::{apply, Combinator, Item};
use super::sense_ranker::{SenseCandidate, SenseRanker, WordSenses};

/// Default forest cap (D63 §8.7 Stage B): `parse` returns at most this many parses,
/// the lowest-cost (most-frequent-sense) first; the rest are dropped with a log line.
/// Chosen from the scale-up baselines — short sentences over full-WordNet polysemy
/// reach ~2k well-typed parses, so this bounds the forest while keeping every
/// plausible reading; it sits far above any closed-class / demo forest, so those are
/// unaffected (no truncation, order preserved by the stable cost-0 sort).
pub const DEFAULT_FOREST_CAP: usize = 256;

/// **Felicity-eval budget** (fail-closed OOM guard): the number of full-span candidates the
/// felicity loop will NbE-eval, after cost-sorting. The top chart cell is unbeamed (Lever B beams
/// only `len < n`), so with sub-cells beamed to `cell_beam` it can hold up to ~`cell_beam²·n`
/// candidates; over the full lexicon, widen-on-failure escalation makes that thousands, and each
/// felicity check is a full eval/readback/check of an **impredicative-∃** GQ sem — evaluating all of
/// them OOMs (witnessed: ~400 doubly-∃ candidates SIGKILL the process). Cost-sorting first and
/// classifying only the lowest-cost `CLASSIFY_BUDGET` bounds the work without changing the result
/// for normal forests (which have far fewer candidates): the kept readings are the most-frequent /
/// most-preferred, exactly what the forest cap would keep.
pub const CLASSIFY_BUDGET: usize = DEFAULT_FOREST_CAP;

/// Upper bound for widen-on-failure of the sense cap (GH #97): when a capped parse of an
/// all-known-vocabulary sentence yields nothing, the cap is doubled up to this many senses per
/// lemma, then the attempt is abandoned (rather than going uncapped, which would re-OOM long
/// sentences). The final β-level of bounded adaptive supertagging.
pub const SENSE_CAP_WIDEN_MAX: usize = 16;

/// Split prose into lowercased word tokens. Token-internal **separators** — em/en-dashes
/// (`—`/`–`), slashes, and brackets — are normalised to spaces first, so `"not—can"` →
/// `["not", "can"]` and `"and/or"` → `["and", "or"]` (D62 S0). Hyphens (`-`) are kept, so
/// hyphenated compounds (`"double-stranded"`) stay intact. Each token is then trimmed of
/// leading/trailing non-alphanumerics (so `"BRCA1,"` → `"brca1"`); empties are dropped.
/// Multiword forms are recovered by re-joining spans at lookup time, not here.
pub fn tokenize(text: &str) -> Vec<String> {
    // Bracket/dash/slash separators → spaces; the **comma** is preserved as a standalone `,` token
    // (D62 S0) so the parser can key multi-item list coordination on it. Other punctuation is still
    // trimmed off token edges.
    let mut spaced = String::with_capacity(text.len());
    for c in strip_bracketed_asides(text).chars() {
        match c {
            '—' | '–' | '‒' | '―' | '/' | '(' | ')' | '[' | ']' | '{' | '}' => {
                spaced.push(' ')
            }
            ',' => spaced.push_str(" , "),
            other => spaced.push(other),
        }
    }
    let mut toks: Vec<String> = spaced
        .split_whitespace()
        .filter_map(|t| {
            if t == "," {
                Some(",".to_string())
            } else {
                let s = t
                    .trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase();
                (!s.is_empty()).then_some(s)
            }
        })
        .collect();
    // A comma is only a separator BETWEEN content tokens: drop dangling (leading/trailing) commas
    // and collapse runs, so a stray `,` never blocks a full-span parse.
    while toks.first().is_some_and(|t| t == ",") {
        toks.remove(0);
    }
    while toks.last().is_some_and(|t| t == ",") {
        toks.pop();
    }
    toks.dedup_by(|a, b| a == "," && b == ",");
    toks
}

/// Drop **bracketed asides** before tokenizing (D62 S0): parenthetical `(…)`/`[…]`/`{…}` glosses
/// (depth-aware) and **em-dash-bracketed appositives** `—…—` (paired U+2014). These are droppable
/// for a *scientific claim* — an abbreviation gloss (`microsatellite instability (MSI)`), a figure
/// ref (`(Fig. 1a)`), or a defining appositive (`lethality—an interaction…—can be exploited`) leaves
/// the head + matrix asserting the same fact. A deliberate, recorded cut (apposition-as-renaming is
/// discourse-level, out of scope for the claim — `docs/notes/d62-grammar-gap-analysis.md`). Content
/// punctuation (commas/lists) is NOT dropped here — that is the marker-keyed list slice.
/// A single (unpaired) em-dash is left for the tokenizer to split (it isn't a bracketing pair).
fn strip_bracketed_asides(text: &str) -> String {
    // 1. Parentheticals/brackets, depth-aware (handles nesting like `poly(ADP(x))`).
    let mut no_parens = String::with_capacity(text.len());
    let mut depth = 0u32;
    for c in text.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => no_parens.push(c),
            _ => {}
        }
    }
    // 2. Paired em-dash appositives: with an even number of `—`, the bracketed asides are the
    // odd-indexed segments; keep the even-indexed matrix. An odd count (a lone `—`) is left as-is.
    let parts: Vec<&str> = no_parens.split('\u{2014}').collect();
    if parts.len() >= 3 && parts.len() % 2 == 1 {
        parts
            .iter()
            .step_by(2) // 0, 2, 4, … = the matrix segments
            .copied()
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        no_parens
    }
}

/// A `form → entries` lookup over a layer's committed `lexicon:LexicalEntry`
/// resources, each resolvable to a parse [`Item`] (category + sem). Built once per
/// layer; `parse` reuses it. Keys are **lowercased** forms (case-insensitive
/// lookup, the v1 choice; case-sensitive acronym disambiguation is a refinement).
///
/// Two backing modes (D65 §2.2, decisions D1/D2):
/// - **Lazy** — when a `core:ValueIndex` on `lexicon:form` is active at the layer
///   head (the production path: a shared-storage chain rooted at `bootstrap`). Form
///   lookups probe that exact index on demand and memoise per form, so `build` is
///   O(1) and a parse touches only the forms its sentence mentions — essential at
///   WordNet scale (325k entries), where the eager full-chain scan dominated.
/// - **Eager** — the fallback when no such index is active (e.g. an isolated-storage
///   chain, where [`scan_chain`](crate::layer)'s shared-index requirement keeps the
///   schema's declaration invisible from a child layer). Scans the whole chain once
///   into a materialised `form → items` map. This is the pre-D65 implementation,
///   retained verbatim as the no-index path; the two modes are behaviour-identical.
pub struct LexicalIndex {
    layer: Arc<Layer>,
    source: Source,
    /// Optional **sense cap** (adaptive supertagging, D63 parsing-scale plan / GH #97): seed at
    /// most this many entries per lemma, the lowest-`sense_rank` (most-frequent / highest-prior)
    /// first. `None` = uncapped (default; no behaviour change). Caps the WordNet sense-polysemy
    /// that drives the chart blow-up on long sentences; the closed class (1–few entries per form)
    /// is unaffected. Pair with widen-on-failure for completeness.
    sense_cap: Option<usize>,
    /// Optional **contextual sense reranker** (D63 parsing-scale plan / GH #97) — the *strong*
    /// form of the sense cap. When set (and a `sense_cap` is active), a per-sentence pre-pass asks
    /// the (untrusted) ranker to reorder each content word's candidate senses by contextual
    /// plausibility, so the senses the cap *keeps* are the ones most likely in this sentence — not
    /// merely the statically most-frequent (`sense_rank`). The ranker only reorders the seed beam;
    /// the kernel felicity gate still decides validity and widen-on-failure recovers a wrongly
    /// down-ranked sense, so a bad rank costs a re-parse, never a missed parse. `None` = the plain
    /// static `sense_rank` cap (no behaviour change).
    sense_ranker: Option<Box<dyn SenseRanker + Send + Sync>>,
    /// Optional **per-cell beam** (Lever B — D63 parsing-scale plan / GH #97). Each CKY chart cell
    /// is capped to this many lowest-`Cost` items after it is built. Bounds the chart's intermediate
    /// growth — the source of the full-lexicon OOM (sense-cap Lever A caps senses *per lemma* at the
    /// leaf; it does not stop a *fully-known* structurally-complex sentence's composed cells from
    /// blowing up over a dense lexicon). Applied to every non-top cell (`len < n`); leaf cells stay
    /// governed by `sense_cap` and the top cell by [`DEFAULT_FOREST_CAP`]. **Inexact** — like any
    /// beam it may drop a constituent the only full parse needed (the beam/A* tradeoff) — so it is
    /// opt-in; `None` = exact (unbounded) chart, the default (no behaviour change).
    cell_beam: Option<usize>,
}

/// One resolved lexical entry at the **seed stage**: its parse [`Item`], its `lexicon:in_lexicon`
/// membership (the scope filter, D65 §4), and its `lexicon:sense` label (for contextual reranking).
/// The sense rides *only* here — once a leaf enters the chart its [`Item`] carries no sense (a
/// composed item has none), so the sense never bloats the hot CKY structure.
#[derive(Clone)]
struct SeedEntry {
    item: Item,
    in_lexicon: Option<Iri>,
    sense: Option<String>,
}

/// The resolved entries for one surface form (each a [`SeedEntry`]) — the unit a scope
/// filter (D65 §4) consumes to keep + rank entries by lexicon, and the sense cap /
/// contextual reranker consume to keep entries by sense.
type FormEntries = Vec<SeedEntry>;

/// The two backings behind [`LexicalIndex`] (lazy probe vs eager materialisation).
enum Source {
    /// Materialised `form → (item, in_lexicon)` — used when no form `ValueIndex`
    /// is active. The per-item `in_lexicon` (D65 §3) is the entry's `lexicon:Lexicon`
    /// membership, consumed at seed time for scope filtering + precedence ranking.
    Eager {
        by_form: BTreeMap<String, FormEntries>,
        /// Word count of the longest indexed form — the multi-span seeding window.
        max_words: usize,
    },
    /// On-demand probe of the active `lexicon:form` `ValueIndex`, memoised per form.
    Lazy {
        /// The `core:ValueIndex` Resource IRI its entries are keyed under.
        index_iri: Iri,
        /// The normalizer it declares — applied to a lookup key so it matches how
        /// the index was populated (D65: `lowercase`).
        normalizer: Iri,
        /// `normalized_form → resolved (item, in_lexicon)`. Presence = probed (an
        /// empty `Vec` records a probed miss, so a missing form is never re-probed).
        cache: Mutex<BTreeMap<String, FormEntries>>,
    },
}

/// The `lexicon:in_lexicon` membership of an entry resource (D65 §3), or `None`
/// for an untagged entry (always-available — e.g. the grammatical closed class).
fn read_in_lexicon(r: &crate::ontology::resource::Resource) -> Option<Iri> {
    r.get(&iri("urn:eigenius:lexicon:in_lexicon"))
        .and_then(|v| v.as_iri_str())
        .and_then(|s| Iri::parse(s).ok())
}

/// The `lexicon:sense` label of an entry resource — the sense key (e.g. `wn:bank.n.05`) the
/// contextual reranker reorders by. `None` for an entry that carries no sense (closed class).
fn read_sense(r: &crate::ontology::resource::Resource) -> Option<String> {
    match r.get(&iri("urn:eigenius:lexicon:sense")) {
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

/// The human-readable gloss of a chain entity (its `core:description`) — what the reranker reasons
/// over for a candidate sense. `None` if the entity has no description.
fn read_description(r: &crate::ontology::resource::Resource) -> Option<String> {
    match r.get(&iri("urn:eigenius:core:description")) {
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Resolve a `lexicon:LexiconProfile` IRI to its ordered scope — the
/// `lexicon:lexica` array of `lexicon:Lexicon` IRIs, in declaration order =
/// resolution precedence (D65 §4.1). The result is ready to pass as the `scope`
/// to [`LexicalIndex::parse_scoped`]. Returns `None` if the IRI doesn't resolve or
/// carries no `lexica`. Resolved against `layer`'s chain so a profile committed
/// anywhere below the parse head is visible.
pub fn resolve_lexicon_profile(layer: &Layer, profile: &Iri) -> Option<Vec<Iri>> {
    let r = layer.resolve(profile)?;
    match r.get(&iri("urn:eigenius:lexicon:lexica"))? {
        Value::Array(items) => Some(
            items
                .iter()
                .filter_map(|v| v.as_iri_str().and_then(|s| Iri::parse(s).ok()))
                .collect(),
        ),
        v => v
            .as_iri_str()
            .and_then(|s| Iri::parse(s).ok())
            .map(|i| vec![i]),
    }
}

impl LexicalIndex {
    /// Build the lookup over `layer`. Prefers the **lazy** path — a declared, active
    /// `core:ValueIndex` on `lexicon:form` — and falls back to the **eager** full-
    /// chain scan when none is active. Entries whose `cat`/`sem` fail to resolve are
    /// skipped (the felicity gate caught them at import; a parse cannot use them).
    pub fn build(layer: Arc<Layer>) -> Self {
        let form_prop = iri("urn:eigenius:lexicon:form");
        if let Some(active) = resolve_active_value_indexes(&layer)
            .into_iter()
            .find(|a| a.target_property == form_prop)
        {
            return LexicalIndex {
                layer,
                source: Source::Lazy {
                    index_iri: active.iri,
                    normalizer: active.normalizer,
                    cache: Mutex::new(BTreeMap::new()),
                },
                sense_cap: None,
                sense_ranker: None,
                cell_beam: None,
            };
        }
        let (by_form, max_words) = Self::scan_eager(&layer);
        LexicalIndex {
            layer,
            source: Source::Eager { by_form, max_words },
            sense_cap: None,
            sense_ranker: None,
            cell_beam: None,
        }
    }

    /// Set the per-lemma **sense cap** (adaptive supertagging — GH #97): keep at most `n` entries
    /// per lemma, lowest `sense_rank` first. Cuts WordNet sense-polysemy at the seed to keep the
    /// chart tractable on long sentences. Builder-style; default (unset) is uncapped.
    pub fn with_sense_cap(mut self, n: usize) -> Self {
        self.sense_cap = Some(n);
        self
    }

    /// Set the **per-cell beam** (Lever B — GH #97): cap every non-top CKY cell to `n`
    /// lowest-`Cost` items, bounding the chart's intermediate growth so a fully-known
    /// structurally-complex sentence doesn't OOM over a dense lexicon (where the per-lemma
    /// `sense_cap` alone is insufficient). Inexact (may drop a constituent the only full parse
    /// needed); builder-style, default (unset) is the exact unbounded chart.
    pub fn with_cell_beam(mut self, n: usize) -> Self {
        self.cell_beam = Some(n);
        self
    }

    /// Set the **contextual sense reranker** (GH #97) — the strong form of the sense cap. With a
    /// cap active, a per-sentence pre-pass asks `ranker` to reorder each content word's candidate
    /// senses by contextual plausibility, so the cap keeps the senses most likely *in this
    /// sentence*, not merely the statically most-frequent. No-op without a [`Self::with_sense_cap`]
    /// (the ranker only influences which senses the cap drops). Builder-style; default is the plain
    /// static `sense_rank` cap.
    pub fn with_sense_ranker(mut self, ranker: Box<dyn SenseRanker + Send + Sync>) -> Self {
        self.sense_ranker = Some(ranker);
        self
    }

    /// The pre-D65 eager scan: walk the chain (`iter_all_resources`, which follows
    /// parent `Arc` pointers — storage-sharing independent) and materialise
    /// `form → items`, tracking the longest form's word count for span seeding.
    fn scan_eager(layer: &Arc<Layer>) -> (BTreeMap<String, FormEntries>, usize) {
        let entry_class = iri("urn:eigenius:lexicon:LexicalEntry");
        let form_prop = iri("urn:eigenius:lexicon:form");
        let mut by_form: BTreeMap<String, FormEntries> = BTreeMap::new();
        let mut max_words = 1;
        for (_id, r) in layer.iter_all_resources() {
            if !r.is_instance_of(&entry_class) {
                continue;
            }
            let Some(Value::String(form)) = r.get(&form_prop) else {
                continue;
            };
            let Ok(item) = entry_to_item(layer, r.as_ref()) else {
                continue;
            };
            let key = form.trim().to_lowercase();
            if key.is_empty() {
                continue;
            }
            max_words = max_words.max(key.split_whitespace().count());
            by_form.entry(key).or_default().push(SeedEntry {
                item,
                in_lexicon: read_in_lexicon(r.as_ref()),
                sense: read_sense(r.as_ref()),
            });
        }
        (by_form, max_words)
    }

    /// Items for one exact, already-lowercased form key. **Eager**: a map lookup.
    /// **Lazy**: a memoised probe of the active `lexicon:form` `ValueIndex` —
    /// `value_index.lookup(index, normalize(form))` yields candidate `(subject,
    /// layer)` across the DAG; each distinct subject is resolved chain-nearest (via
    /// [`Layer::resolve`](crate::layer::Layer::resolve), which filters to the head's
    /// chain and shadow-resolves), re-checked to be a `LexicalEntry` whose form
    /// still normalizes to the key, then turned into an [`Item`].
    fn entries_for(&self, form_lc: &str) -> FormEntries {
        match &self.source {
            Source::Eager { by_form, .. } => by_form.get(form_lc).cloned().unwrap_or_default(),
            Source::Lazy {
                index_iri,
                normalizer,
                cache,
            } => {
                let key = normalize_value(normalizer, form_lc);
                if let Some(hit) = cache.lock().expect("LexicalIndex cache poisoned").get(&key) {
                    return hit.clone();
                }
                let items = self.probe_form(index_iri, normalizer, &key);
                cache
                    .lock()
                    .expect("LexicalIndex cache poisoned")
                    .insert(key, items.clone());
                items
            }
        }
    }

    /// Probe the active value index for a normalized form key (lazy path).
    fn probe_form(&self, index_iri: &Iri, normalizer: &Iri, norm_key: &str) -> FormEntries {
        let entry_class = iri("urn:eigenius:lexicon:LexicalEntry");
        let form_prop = iri("urn:eigenius:lexicon:form");
        let mut seen: BTreeSet<Iri> = BTreeSet::new();
        let mut items = Vec::new();
        for hit in self.layer.storage().value_index.lookup(index_iri, norm_key) {
            let Ok((subject, _defining)) = hit else {
                continue;
            };
            if !seen.insert(subject.clone()) {
                continue; // a subject can be hit once per defining layer; resolve once
            }
            // Resolve the chain-nearest definition (None ⇒ out of this head's chain).
            let Some(r) = self.layer.resolve(&subject) else {
                continue;
            };
            if !r.is_instance_of(&entry_class) {
                continue;
            }
            // Shadow safety: the resolved (nearest) definition's form must still
            // normalize to the queried key — a closer layer may have redefined it.
            let Some(Value::String(form)) = r.get(&form_prop) else {
                continue;
            };
            if normalize_value(normalizer, form) != norm_key {
                continue;
            }
            let Ok(item) = entry_to_item(&self.layer, r.as_ref()) else {
                continue;
            };
            items.push(SeedEntry {
                item,
                in_lexicon: read_in_lexicon(r.as_ref()),
                sense: read_sense(r.as_ref()),
            });
        }
        items
    }

    /// The multi-span seeding window: how far a lexical span may reach from token
    /// `i`. **Eager** knows the longest indexed form (`max_words`); **lazy** seeds
    /// every span up to the sentence length `n` (D65 §2.3 / D3 — no `max_words`
    /// stat; an over-long span is a cheap empty probe, memoised).
    fn span_limit(&self, n: usize) -> usize {
        match &self.source {
            Source::Eager { max_words, .. } => *max_words,
            Source::Lazy { .. } => n,
        }
    }

    /// Number of distinct indexed forms. **Eager**: the total materialised forms.
    /// **Lazy**: the forms probed into the cache so far (forms are discovered on
    /// demand, so the full count is not known without enumerating the value index).
    pub fn len(&self) -> usize {
        match &self.source {
            Source::Eager { by_form, .. } => by_form.len(),
            Source::Lazy { cache, .. } => cache.lock().expect("LexicalIndex cache poisoned").len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether any lexical entry exists for `surface` — the raw lowercased surface, or
    /// any lemma the [`Lemmatizer`] yields across the parts of speech. Scope-independent.
    ///
    /// This is the **missing-lexeme signal** the encoding pipeline (D62 §7.6a) keys lazy
    /// lexical recovery off: when a parse comes back empty, a token for which this is
    /// `false` is an unknown word (route to lexical recovery / search+inject), whereas an
    /// empty parse with all tokens known is a grammar gap (route to reformulation).
    pub fn has_token(&self, surface: &str, lemmatizer: &dyn Lemmatizer) -> bool {
        let s_lc = surface.trim().to_lowercase();
        // Coordinating conjunctions (`and`/`or`/`but`) are consumed by the parser's
        // coordination rule, not a lexical entry — known, not missing (D63 §8.4).
        if coord_connective(&s_lc).is_some() {
            return true;
        }
        if !self.entries_for(&s_lc).is_empty() {
            return true;
        }
        for pos in [Pos::Noun, Pos::Verb, Pos::Adj, Pos::Adv] {
            for lemma in lemmatizer.lemmas(surface, pos) {
                if !self.entries_for(&lemma.trim().to_lowercase()).is_empty() {
                    return true;
                }
            }
        }
        // A productive `-ly` adverb whose adjective base is known, or a lexicalized discourse
        // adverb, is parseable (D62 Phase 3) — *known*, not a missing lexeme.
        self.is_derived_adverb(&s_lc) || is_lexicalized_adverb(&s_lc)
    }

    /// Apply the per-parse lexicon **scope** (D65 §4) to one form's resolved
    /// `(item, in_lexicon)` pairs, returning the surviving [`Item`]s with their
    /// leaf `cost.lexicon_order` stamped from the scope:
    ///
    /// - `scope = None` (default) — keep everything, `lexicon_order` stays 0
    ///   (behaviour-preserving, unordered whole chain);
    /// - `scope = Some(order)` — keep an entry iff its `in_lexicon` is in `order`
    ///   (its position becomes `lexicon_order`, the primary rank key), **or** it is
    ///   untagged (`in_lexicon = None` ⇒ always-available, e.g. the closed class).
    ///   A tagged entry whose lexicon is outside the scope is dropped.
    fn scoped(&self, entries: FormEntries, scope: Option<&[Iri]>) -> Vec<SeedEntry> {
        entries
            .into_iter()
            .filter_map(|mut e| match scope {
                None => Some(e),
                Some(order) => match &e.in_lexicon {
                    None => Some(e), // untagged = always available
                    Some(lx) => order.iter().position(|s| s == lx).map(|pos| {
                        e.item.cost.lexicon_order = pos as u32;
                        e
                    }),
                },
            })
            .collect()
    }

    /// Every candidate lemma string for a surface: the raw lowercased surface plus every lemma the
    /// [`Lemmatizer`] yields across all parts of speech, de-duplicated. The shared seam used by
    /// both [`Self::lookup_span`] (seeding) and [`Self::contextual_sense_ranks`] (the rerank
    /// pre-pass), so the two see exactly the same candidate set.
    fn candidate_lemmas(&self, surface: &str, lemmatizer: &dyn Lemmatizer) -> BTreeSet<String> {
        let mut candidates: BTreeSet<String> = BTreeSet::new();
        candidates.insert(surface.trim().to_lowercase());
        for pos in [Pos::Noun, Pos::Verb, Pos::Adj, Pos::Adv] {
            for lemma in lemmatizer.lemmas(surface, pos) {
                candidates.insert(lemma.trim().to_lowercase());
            }
        }
        candidates
    }

    /// The lexical items for one token span's surface: the raw surface plus every
    /// lemma the [`Lemmatizer`] yields across all parts of speech (so an inflected
    /// or collocated form resolves to its base entries). Candidate strings are
    /// de-duplicated before lookup. `scope` filters + ranks by lexicon (§4). `ranks`, when
    /// present, is the per-sentence contextual sense ranking (`sense → rank`) that overrides the
    /// static `sense_rank` ordering when the cap drops senses.
    fn lookup_span(
        &self,
        surface: &str,
        lemmatizer: &dyn Lemmatizer,
        scope: Option<&[Iri]>,
        cap: Option<usize>,
        ranks: Option<&BTreeMap<String, u32>>,
    ) -> Vec<Item> {
        let s_lc = surface.trim().to_lowercase();
        let candidates = self.candidate_lemmas(surface, lemmatizer);
        let mut out = Vec::new();
        for c in &candidates {
            let mut entries = self.scoped(self.entries_for(c), scope);
            if entries.is_empty() {
                continue;
            }
            // Adaptive-supertagging sense cap (GH #97): keep at most `cap` entries for this lemma —
            // by contextual plausibility first (the reranker's `ranks`, when present), falling back
            // to the static `sense_rank` (most-frequent first) — cutting WordNet polysemy at the
            // seed. The closed class (≤ cap entries) is untouched. A stable sort preserves seed
            // order within a rank. (`cap` is the per-attempt cap from the widen loop.)
            if let Some(cap) = cap {
                if entries.len() > cap {
                    entries.sort_by_key(|e| sense_cap_key(e, ranks));
                    entries.truncate(cap);
                }
            }
            // Morphological number (D63 §5.1, the Slice-1 deferral): a surface
            // that morphology *reduced* to this lemma was inflected (plural,
            // for nouns); a surface equal to the lemma is singular. Refine the
            // common noun's underspecified `num_any` to that number so
            // determiner/noun agreement (`every gene` ✓ / `every genes` ✗)
            // bites at composition.
            let num = if *c == s_lc { "sg" } else { "pl" };
            out.extend(entries.iter().map(|e| with_noun_num(&e.item, num)));
        }
        // Bare-plural → kind-subject shift (D63 §8.5 Slice 3c): a plural common noun
        // also seeds a `cat_kind` edge (the kind it denotes), so "genes" can serve as
        // a kind subject ("Genes are cell lines" → subclass_of(Gene, CellLine))
        // alongside its ordinary common-noun reading.
        let kinds: Vec<Item> = out
            .iter()
            .filter_map(|it| {
                crate::dcg::kind_subject(&it.cat, &it.sem)
                    .map(|(cat, sem)| Item::with_cost(cat, sem, it.cost))
            })
            .collect();
        out.extend(kinds);
        out
    }

    /// Transparent `-ly` **adverb** items (D62 Phase 3 — `docs/notes/d62-adverb-semantics-decision.md`).
    /// If `surface` is a single `-ly` form whose adjective base is **known to the lexicon**
    /// (data-driven probe — no hardcoded adverb list; WordNet doesn't store productive `-ly`
    /// adverbs), seed identity-sem modifier items at the WRN attachment categories
    /// ([`adverb_modifier_cats`]). The adverb composes and contributes nothing to the claim `Prop`
    /// — the science-transparent default; the measurement subset's obligation semantics is a later
    /// arm. Empty when the surface isn't an `-ly` form, no adjective base resolves, or the `Cat`
    /// inductives are unavailable.
    /// Whether `surface` is a productive `-ly` adverb whose adjective base is **known to the
    /// lexicon** (the data-driven recognition gate, D62 Phase 3). Shared by [`Self::adverb_items`]
    /// (seeding) and [`Self::has_token`] (the missing-lexeme diagnostic), so a derived adverb counts
    /// as *known* — not routed to lexical recovery.
    fn is_derived_adverb(&self, surface: &str) -> bool {
        let s = surface.trim().to_lowercase();
        adverb_bases(&s).iter().any(|b| {
            self.entries_for(b)
                .iter()
                .any(|e| is_adjective_cat(&e.item.cat))
        })
    }

    fn adverb_items(&self, surface: &str) -> Vec<Item> {
        let s = surface.trim().to_lowercase();
        let lexicalized = is_lexicalized_adverb(&s);
        if !lexicalized && !self.is_derived_adverb(&s) {
            return Vec::new();
        }
        // Manner positions (adjective + VP modifier) for every transparent adverb; discourse
        // adverbs (`also`/`however`/`yet`) ALSO attach at the clause level (`S/S`, `S\S`).
        let mut cats = adverb_modifier_cats(&self.layer).unwrap_or_default();
        if lexicalized {
            cats.extend(sentence_modifier_cats(&self.layer).unwrap_or_default());
        }
        if cats.is_empty() {
            return Vec::new();
        }
        // Identity sem `λx. x`: forward/backward application leaves the modified phrase's sem
        // unchanged (β-reduces away at felicity), so the claim `Prop` is exactly the unmodified one.
        let ident = Exp::Lam(
            Patt::Var("__adv_x".to_string()),
            Box::new(Exp::Var("__adv_x".to_string())),
        );
        cats.into_iter()
            .map(|cat| Item::new(cat, ident.clone()))
            .collect()
    }

    /// Parse prose into the forest of typed sentence parses: every full-span `S`
    /// derivation whose assembled sem type-checks to `Prop`. Returns the WHOLE
    /// forest (ambiguity included); an empty `Vec` means no admissible parse.
    ///
    /// Unscoped (the whole composed chain, unordered) — see [`Self::parse_scoped`]
    /// for the per-parse lexicon scope (D65 §4).
    pub fn parse(&self, text: &str, lemmatizer: &dyn Lemmatizer) -> Vec<Item> {
        self.parse_scoped(text, lemmatizer, None)
    }

    /// Parse with an optional **lexicon scope** (D65 §4): an ordered list of
    /// `lexicon:Lexicon` IRIs. Only entries whose `lexicon:in_lexicon` is in the
    /// scope (or untagged — always-available, e.g. the closed class) seed the
    /// chart, and each entry's position in the list becomes its leaf
    /// `lexicon_order` — the **primary** rank key, so earlier-listed lexica rank
    /// first (soft precedence; later lexica stay in the forest, no shadowing).
    /// `scope = None` is the unordered whole chain (backward-compatible).
    pub fn parse_scoped(
        &self,
        text: &str,
        lemmatizer: &dyn Lemmatizer,
        scope: Option<&[Iri]>,
    ) -> Vec<Item> {
        self.parse_scoped_open(text, lemmatizer, scope).0
    }

    /// Parse with optional scope, returning **both** the closed forest and the **open**
    /// (hole-bearing) forest (D64 open-parse carrier). A pronoun seeds a referent *hole*
    /// (a fresh free variable); a full-span `S` whose felicitous sem still carries holes is
    /// an [`OpenParse`] — type-checked (each hole bound to `Entity`) but not a closed final
    /// parse, awaiting the D64 resolver. The closed forest is identical to what
    /// [`Self::parse_scoped`] returns; `parse` / `parse_scoped` are thin closed-only wrappers.
    pub fn parse_open(
        &self,
        text: &str,
        lemmatizer: &dyn Lemmatizer,
    ) -> (Vec<Item>, Vec<OpenParse>) {
        self.parse_scoped_open(text, lemmatizer, None)
    }

    /// Parse with optional scope, returning the closed + open forests. Applies the **sense cap**
    /// (`with_sense_cap`) with **widen-on-failure** (adaptive supertagging, GH #97): try the cap;
    /// if it yields *no* parse at all (closed and open both empty) **and** the failure could be a
    /// cap-dropped sense — i.e. every (prose) token is lexically known, so it is not an OOV miss —
    /// retry with a doubled cap, up to [`SENSE_CAP_WIDEN_MAX`]. So the cap never *loses* a parse a
    /// known-vocabulary sentence would otherwise get, while OOV-blocked sentences (the WRN case)
    /// don't waste retries (widening can't supply a missing word).
    pub fn parse_scoped_open(
        &self,
        text: &str,
        lemmatizer: &dyn Lemmatizer,
        scope: Option<&[Iri]>,
    ) -> (Vec<Item>, Vec<OpenParse>) {
        // Contextual sense ranking (GH #97): computed ONCE up front (one ranker call per parse,
        // not per widen iteration), then threaded into every capped attempt below.
        let ranks = self.contextual_sense_ranks(text, lemmatizer, scope);
        let mut cap = self.sense_cap;
        loop {
            let (closed, open) = self.parse_at_cap(text, lemmatizer, scope, cap, ranks.as_ref());
            if !closed.is_empty() || !open.is_empty() {
                return (closed, open);
            }
            match cap {
                // Widen only if the cap could be the cause (no OOV token) and we're under the bound.
                Some(c)
                    if c < SENSE_CAP_WIDEN_MAX && self.all_prose_tokens_known(text, lemmatizer) =>
                {
                    cap = Some((c * 2).min(SENSE_CAP_WIDEN_MAX));
                }
                _ => return (closed, open),
            }
        }
    }

    /// Whether every prose token (non-`is_nonprose`) of `text` is lexically known
    /// ([`Self::has_token`]). Used to gate widen-on-failure: an OOV miss is not a cap miss.
    fn all_prose_tokens_known(&self, text: &str, lemmatizer: &dyn Lemmatizer) -> bool {
        tokenize(text)
            .iter()
            .filter(|t| !super::is_nonprose(t))
            .all(|t| self.has_token(t, lemmatizer))
    }

    /// The per-sentence **contextual sense ranking** (GH #97): for each content-word span with
    /// more candidate senses than the cap (the only words the cap actually truncates), ask the
    /// (untrusted) [`SenseRanker`] to reorder its senses by contextual plausibility, and fold the
    /// result into a flat `sense → rank` map the seed cap then sorts by. Returns `None` — i.e. the
    /// plain static `sense_rank` cap — when no ranker or no cap is configured, when the sentence
    /// has no over-cap polysemous word, or when the ranker reply is malformed (it only reorders a
    /// beam; a bad reply degrades to the static order, never a missed parse).
    ///
    /// Run ONCE per parse (before the widen loop), against the *initial* cap: widening only raises
    /// the cap (fewer words need ranking), so a map computed at the initial cap stays valid — its
    /// extra entries simply go unused. The ranker reasons over each sense's `core:description`
    /// gloss, resolved from the entry's `sem` entity.
    fn contextual_sense_ranks(
        &self,
        text: &str,
        lemmatizer: &dyn Lemmatizer,
        scope: Option<&[Iri]>,
    ) -> Option<BTreeMap<String, u32>> {
        let ranker = self.sense_ranker.as_deref()?;
        let cap = self.sense_cap?; // ranking only matters when the cap can drop senses
        let tokens = tokenize(text);
        let n = tokens.len();
        if n == 0 {
            return None;
        }
        let span_limit = self.span_limit(n);

        // Gather, per over-cap span, its pooled candidate senses (deduped by sense key).
        let mut surfaces: Vec<String> = Vec::new();
        let mut cands: Vec<Vec<SenseCandidate>> = Vec::new();
        for i in 0..n {
            let last = (i + span_limit).min(n);
            for j in i..last {
                let surface = tokens[i..=j].join(" ");
                let mut senses: Vec<SenseCandidate> = Vec::new();
                let mut seen: BTreeSet<String> = BTreeSet::new();
                for c in self.candidate_lemmas(&surface, lemmatizer) {
                    for e in self.scoped(self.entries_for(&c), scope) {
                        let Some(sense) = e.sense else { continue };
                        if !seen.insert(sense.clone()) {
                            continue;
                        }
                        let gloss = self.sem_gloss(&e.item.sem).unwrap_or_default();
                        senses.push(SenseCandidate { sense, gloss });
                    }
                }
                // Only words the cap would actually truncate are worth ranking.
                if senses.len() > cap {
                    surfaces.push(surface);
                    cands.push(senses);
                }
            }
        }
        if cands.is_empty() {
            return None;
        }

        let words: Vec<WordSenses> = surfaces
            .iter()
            .zip(&cands)
            .map(|(s, c)| WordSenses {
                surface: s,
                candidates: c,
            })
            .collect();
        let rankings = ranker.rank(text, &words);
        if rankings.len() != words.len() {
            return None; // malformed reply ⇒ degrade to the static cap
        }
        // Flatten to `sense → rank`. A sense shared across overlapping spans keeps its best (min)
        // contextual rank.
        let mut map: BTreeMap<String, u32> = BTreeMap::new();
        for (ranking, word_cands) in rankings.iter().zip(&cands) {
            for (pos, &ci) in ranking.iter().enumerate() {
                if let Some(c) = word_cands.get(ci) {
                    map.entry(c.sense.clone())
                        .and_modify(|r| *r = (*r).min(pos as u32))
                        .or_insert(pos as u32);
                }
            }
        }
        Some(map)
    }

    /// The `core:description` gloss of a leaf item's `sem` entity — the text the reranker reasons
    /// over for that sense. An `EigonResource` carries its resource inline; a class/axiom is
    /// resolved by IRI in the chain. `None` for an inline λ-term sem (function words) or a
    /// description-less entity.
    fn sem_gloss(&self, sem: &Exp) -> Option<String> {
        match sem {
            Exp::EigonResource(r) => read_description(r),
            Exp::EigonClass(i) | Exp::EigonAxiom(i) => {
                read_description(self.layer.resolve(i)?.as_ref())
            }
            _ => None,
        }
    }

    fn parse_at_cap(
        &self,
        text: &str,
        lemmatizer: &dyn Lemmatizer,
        scope: Option<&[Iri]>,
        cap: Option<usize>,
        ranks: Option<&BTreeMap<String, u32>>,
    ) -> (Vec<Item>, Vec<OpenParse>) {
        let tokens = tokenize(text);
        let n = tokens.len();
        if n == 0 {
            return (Vec::new(), Vec::new());
        }
        // Parse-failure instrumentation (set `EIGENIUS_PARSE_DEBUG=1`): per-cell stats, flushed, so
        // the last line before an OOM/SIGKILL localizes the blow-up cell + cap level.
        let debug = std::env::var("EIGENIUS_PARSE_DEBUG").is_ok();

        // chart[i][j] = every item spanning tokens i..=j.
        let mut chart: Vec<Vec<Vec<Item>>> = vec![vec![Vec::new(); n]; n];

        // Coordinator positions (D63 §8.4 Phase 3): `and`/`or` are parser-level
        // reserved words (NOT lexical entries — coordination is polymorphic over
        // `Cat`, which `⟦·⟧` can't denote), handled by the coordination rule below.
        let coord_op: Vec<Option<&str>> = tokens
            .iter()
            .map(|t| coord_connective(t.as_str()))
            .collect();

        // 1. Seed lexical spans (multi-span MWE seeding). A multiword form at
        //    [i,j] is seeded ALONGSIDE the items of its parts, so both readings
        //    survive into the chart.
        let span_limit = self.span_limit(n);
        for i in 0..n {
            let last = (i + span_limit).min(n);
            for j in i..last {
                let surface = tokens[i..=j].join(" ");
                for mut it in self.lookup_span(&surface, lemmatizer, scope, cap, ranks) {
                    // Referent-hole freshening (D64 open-parse carrier): the placeholder
                    // `lexicon:anaphor` (a bare pronoun's whole sem, or the possessor NESTED
                    // inside a possessive determiner's λ) is replaced with a fresh,
                    // per-occurrence free variable so distinct occurrences are distinct holes.
                    // The freshened var rides through CKY and is typed (`Entity`) at felicity.
                    it.sem = freshen_anaphor(&it.sem, &hole_base(i, j));
                    chart[i][j].push(it);
                }
                // Derived `-ly` adverbs (D62 Phase 3): transparent modifier items for a single
                // `-ly` token whose adjective base is known. Single-token spans only (adverbs are
                // one word); the identity sem carries no holes, so no freshening.
                if i == j {
                    for it in self.adverb_items(&surface) {
                        chart[i][j].push(it);
                    }
                }
            }
        }

        // Forward bounded type-raising `T` (D63 §8.9 Slice 6-T) at the LEAF cells: a
        // name `NP` lifts to `S/(S\NP)`, so it can forward-compose into a relative
        // clause's object-extraction body `S/NP` ("HeLa affects [gap]"). Applied once
        // per leaf cell here; multi-token / composed cells are raised once each in the
        // CKY loop below. ENF (the `TypeRaised` provenance) keeps these inert outside
        // extraction — a raised functor may only compose, never apply.
        // Lever B (per-cell beam, GH #97): drop-count accumulated across every beamed cell, logged
        // once below. Leaf cells are beamed here (after type-raise), composed cells in the loop.
        let mut beam_drops = 0usize;
        for (i, row) in chart.iter_mut().enumerate() {
            let raised = raise_nps(&row[i], &self.layer);
            row[i].extend(raised);
            // A leaf cell is non-top iff the sentence has >1 token (else it is the full-span root,
            // left to the forest cap). `sense_cap` already bounds it per-lemma; the beam caps it
            // across all candidate lemmas/POS of the token.
            if n > 1 {
                if let Some(beam) = self.cell_beam {
                    beam_drops += beam_cell(&mut row[i], beam);
                }
            }
        }

        // 2. CKY composition, appending combined items to each cell's seeds (so a
        //    multiword leaf and a compositional derivation of the same span both
        //    remain available).
        for len in 2..=n {
            for i in 0..=(n - len) {
                let j = i + len - 1;
                let mut produced = Vec::new();
                for k in i..j {
                    let lefts = &chart[i][k];
                    let rights = &chart[k + 1][j];
                    for l in lefts {
                        for r in rights {
                            if let Some(item) = apply(l, r, &self.layer) {
                                produced.push(item);
                            }
                        }
                    }
                }
                // Coordination: `[X] and/or [Y] → [X]` for same-category,
                // Prop-ending conjuncts, with the generalized-conjunction sem
                // (pointwise-lifted connective — D63 §8.4 Phase 3).
                for c in (i + 1)..j {
                    let Some(op) = coord_op[c] else { continue };
                    let lefts = &chart[i][c - 1];
                    let rights = &chart[c + 1][j];
                    for l in lefts {
                        for r in rights {
                            // Left-branching normal form (the spurious-ambiguity
                            // control our grammar needs — D63 §8.4 Phase 4): a
                            // coordination's RIGHT conjunct may not itself be a
                            // coordination, so `A and B and C` parses *only* as
                            // `(A and B) and C`. (Classic Eisner — composition /
                            // type-raising normal forms — does not apply: we have
                            // application + lexical type-raising + coordination,
                            // no composition rule. It returns when one lands.)
                            if is_coordination(&r.sem) {
                                continue;
                            }
                            if cats_coordinate(&l.cat, &r.cat, &self.layer) {
                                if let Some(sem) =
                                    coordinate_sem(op, &l.cat, &l.sem, &r.sem, &self.layer)
                                {
                                    produced.push(Item::with_cost(
                                        l.cat.clone(),
                                        sem,
                                        l.cost.saturating_add(r.cost),
                                    ));
                                }
                            } else if let Some((cat, sem)) =
                                coordinate_np(op, &l.cat, &l.sem, &r.cat, &r.sem, &self.layer)
                            {
                                // NP coordination → a member-retaining `cat_group`
                                // tagged with its connective (D63 §8.4 Phase 6).
                                // Distinct from the Prop-ending generalized
                                // conjunction above: a coordinated NP is not
                                // conjoinable as a `Prop`; it denotes the group
                                // `List C`, distributed (∧ for `and`, ∨ for `or`) or
                                // collected at the verb. `coordinate_np` requires the
                                // right conjunct to be a plain NP, keeping groups
                                // left-branching for the n-ary case (the
                                // `is_coordination` analogue here).
                                produced.push(Item::with_cost(
                                    cat,
                                    sem,
                                    l.cost.saturating_add(r.cost),
                                ));
                            }
                        }
                    }
                }
                // Reciprocal: `[group] <TV> each other → S` (D63 §8.4 Phase 6).
                // "each other" is a reserved anaphor (not a lexical entry); the verb
                // is related over every ordered distinct pair of the subject group's
                // members. Keyed on the trailing "each other" tokens, mirroring how
                // coordination keys on `and`/`or`. The subject must be a (conjunctive)
                // group — a reciprocal needs ≥2 distinct participants.
                if j >= 3 && tokens[j - 1] == "each" && tokens[j] == "other" {
                    // Verb spans [s, j-2]; subject group spans [i, s-1].
                    for s in (i + 1)..=(j - 2) {
                        let subjects = &chart[i][s - 1];
                        let verbs = &chart[s][j - 2];
                        for subj in subjects {
                            for tv in verbs {
                                if let Some((cat, sem)) =
                                    reciprocate(&subj.cat, &subj.sem, &tv.cat, &tv.sem, &self.layer)
                                {
                                    produced.push(Item::with_cost(
                                        cat,
                                        sem,
                                        subj.cost.saturating_add(tv.cost),
                                    ));
                                }
                            }
                        }
                    }
                }
                // Relative clause (D63 §8.9 Slice 6-rel): `[noun] that/which [body]` → a
                // **refined noun** `cat_n(Σx:C. body(x))`. `that`/`which` are reserved
                // relativizers (like `and`/`or`, `each other`); the body is a
                // subject-relative VP `S\NP` ("that affects HeLa") or an object-relative
                // `S/NP` ("that HeLa affects", built by `T` + forward `B`). Both have
                // sem `body : X → Prop`, so a single rule Σ-refines the noun over the
                // concrete `C` (reusing 3b). The noun spans `[i, c-1]`, the body
                // `[c+1, j]`. The refined noun then rides 3b's determiner+`Fst` rule.
                // `which` covers the restrictive `which`-relative; the non-restrictive
                // (comma) reading collapses to the same refinement here (the comma is S0-
                // stripped — the contrast is semantic, deferred). A sentence-initial
                // wh-`which` never matches (no noun spans `[i, c-1]`).
                for c in (i + 1)..j {
                    if !matches!(tokens[c].as_str(), "that" | "which") {
                        continue;
                    }
                    let nouns = &chart[i][c - 1];
                    let bodies = &chart[c + 1][j];
                    for noun in nouns {
                        for body in bodies {
                            if let Some((cat, sem)) = relativize(&noun.cat, &body.cat, &body.sem) {
                                produced.push(Item::with_cost(
                                    cat,
                                    sem,
                                    noun.cost.saturating_add(body.cost),
                                ));
                            }
                        }
                    }
                }
                let produced_n = produced.len();
                chart[i][j].extend(produced);
                // Type-raise `T` (D63 §8.9 Slice 6-T) the cell's name NPs (after its
                // composition + relativizer items are in place), so a non-leaf / composed
                // NP can also seed an extraction body. Raised once per cell.
                let raised = raise_nps(&chart[i][j], &self.layer);
                chart[i][j].extend(raised);
                // Lever B: beam this composed cell (non-top; the top cell `len == n` is left to the
                // forest cap). Done after type-raise so the raised items compete in the beam too.
                if len < n {
                    if let Some(beam) = self.cell_beam {
                        beam_drops += beam_cell(&mut chart[i][j], beam);
                    }
                }
                if debug {
                    eprintln!(
                        "  [parse-debug cap={cap:?}] cell[{i}..{j}] len={len} produced={produced_n} kept={}",
                        chart[i][j].len()
                    );
                }
            }
        }
        if beam_drops > 0 {
            eprintln!(
                "dcg::parse: cell-beam (Lever B) dropped {beam_drops} items \
                 (beam={})",
                self.cell_beam.unwrap_or(0),
            );
        }

        // 3. The forest: full-span `S` items whose assembled sem — once **NbE-
        //    reduced** (the determiner lambdas β-apply away to a normal form) — the
        //    kernel confirms inhabits `Prop`. Reducing first is essential: a
        //    composed determiner sentence is a redex-heavy `App(λ…, …)` tree, and
        //    `check_infer` cannot synthesize a bare lambda's type.
        // The referent-hole context for classification: every per-position hole base name,
        // and the `Entity` type each hole inhabits (Slice-1: all holes are `Entity`).
        let hole_bases: Vec<String> = (0..n)
            .flat_map(|i| (i..n).map(move |j| hole_base(i, j)))
            .collect();
        let entity = eval(&Exp::EigonClass(iri(ENTITY_IRI)), &Rho::Nil).ok();

        // Full-span candidates: a **finite** declarative/polar `S` (denotes `Prop`) or a
        // wh-question `Q(T)` (denotes `T → Prop`, D63 §8.5). The finiteness gate rejects a bare
        // base/infinitival clause as a standalone root; partial functors are dropped.
        // FAIL-CLOSED OOM GUARD: cost-sort and keep only the lowest-cost [`CLASSIFY_BUDGET`] BEFORE
        // the felicity loop — the top cell is unbeamed and can hold thousands of candidates over the
        // full lexicon, and each felicity check NbE-evals an impredicative-∃ GQ sem, so classifying
        // all of them OOMs. (Normal forests have far fewer candidates → no-op.)
        let mut candidates: Vec<&Item> = chart[0][n - 1]
            .iter()
            .filter(|it| {
                // Complete results: a **finite** declarative/polar `S` (denotes `Prop`) or a
                // wh-question `Q(T)` (denotes `T → Prop`, D63 §8.5). The finiteness gate rejects a
                // bare base/infinitival clause (`S[_,bse]`) as a standalone root; partial functors
                // are dropped. NOTE: the sem shape cannot discriminate here — a well-formed
                // determiner-subject clause is an unreduced `App` redex (subject-GQ applied to the
                // VP), structurally identical to a pathological reading; only β-reduction in the
                // felicity gate below tells them apart.
                is_finite_clause(&it.cat) || is_ctor(&it.cat, "cat_q").is_some()
            })
            .collect();
        let n_candidates = candidates.len();
        candidates.sort_by_key(|it| it.cost);
        candidates.truncate(CLASSIFY_BUDGET);
        if debug && n_candidates > candidates.len() {
            eprintln!(
                "  [parse-debug cap={cap:?}] full-span candidates {n_candidates} → felicity-checking \
                 {} (CLASSIFY_BUDGET)",
                candidates.len()
            );
        }

        // Split into the CLOSED forest (felicitous closed `Prop`) and the OPEN forest (felicitous
        // but carrying unresolved referent holes — D64).
        let mut forest: Vec<Item> = Vec::new();
        let mut open: Vec<OpenParse> = Vec::new();
        for (k, it) in candidates.into_iter().enumerate() {
            if debug {
                eprintln!(
                    "  [parse-debug cap={cap:?}] classify candidate {k}/{n_candidates}\n      cat={}\n      sem={}",
                    super::pretty_term(&it.cat),
                    super::pretty_term(&it.sem)
                );
            }
            match entity.as_ref() {
                Some(e) => match self.classify_felicitous(it, &hole_bases, e) {
                    Some(FelicitousOutcome::Closed(c)) => forest.push(c),
                    Some(FelicitousOutcome::Open(o)) => open.push(o),
                    None => {}
                },
                // Entity type unavailable (should not happen): closed path only.
                None => {
                    if let Some(c) = self.reduced_felicitous(it) {
                        forest.push(c);
                    }
                }
            }
        }

        // RANK + CAP (D63 §8.7 Stage B): order each forest by ascending cost — the sum
        // of the parse's leaf `sense_rank`s — so the most-frequent-sense readings come
        // first, then cap to [`DEFAULT_FOREST_CAP`]. WordNet sense-polysemy yields
        // 100s–1000s of well-typed parses for a short sentence (the felicity gate prunes
        // none of it), so an unbounded forest is unusable; the cap bounds it without
        // silent loss — the dropped tail is logged. Stable sort + cost 0 everywhere
        // (closed-class / demo entries) ⇒ no ranking or cap effect there (order
        // preserved, sizes well under the cap), so exact-count tests are unaffected.
        forest.sort_by_key(|it| it.cost);
        if forest.len() > DEFAULT_FOREST_CAP {
            let dropped = forest.len() - DEFAULT_FOREST_CAP;
            eprintln!(
                "dcg::parse: ranked forest capped {} → {DEFAULT_FOREST_CAP} \
                 (dropped {dropped} higher-cost / rarer-sense parses)",
                forest.len(),
            );
            forest.truncate(DEFAULT_FOREST_CAP);
        }
        open.sort_by_key(|o| o.item.cost);
        if open.len() > DEFAULT_FOREST_CAP {
            open.truncate(DEFAULT_FOREST_CAP);
        }
        (forest, open)
    }

    /// Normalize `it.sem` (NbE β-reduction → a normal form) and keep the item —
    /// carrying the reduced sem — only if the kernel confirms it **inhabits `⟦cat⟧`**:
    /// `Prop` for a declarative `S`, `T → Prop` for a wh-question `Q(T)`. Uses
    /// check-mode (not `check_infer`) so a wh-question's answer-property *lambda* —
    /// which `check_infer` cannot synthesize — is checked against its expected Π/→.
    fn reduced_felicitous(&self, it: &Item) -> Option<Item> {
        let expected = denote_cat(&it.cat).ok()?;
        let expected_val = eval(&expected, &Rho::Nil).ok()?;
        let nf = readback_val(0, &eval(&it.sem, &Rho::Nil).ok()?);
        let mut ctx = CheckCtx::with_layer(Rho::Nil, Vec::new(), Arc::clone(&self.layer));
        check(&mut ctx, &nf, &expected_val).ok()?;
        Some(Item {
            cat: it.cat.clone(),
            sem: nf,
            prov: it.prov,
            cost: it.cost,
        })
    }

    /// Classify a full-span candidate as a CLOSED felicitous parse or an OPEN one carrying
    /// unresolved referent holes (D64), or reject it. Generalizes [`Self::reduced_felicitous`]
    /// to hole-bearing sems: a referent hole is a free variable, so it must be bound in `rho`
    /// to a generic neutral (else Pure `eval` errors `UnboundVariable`) and in `gamma` to its
    /// type (`Entity`, Slice-1) so `check` types it. `Neut::Gen(0, h)` reads back as
    /// `Var("{h}0")`, so the gamma key and the reported hole name use that readback form. With
    /// no holes present this is exactly `reduced_felicitous` (empty `rho`/`gamma`) — the closed
    /// path is unchanged.
    fn classify_felicitous(
        &self,
        it: &Item,
        hole_bases: &[String],
        entity: &Val,
    ) -> Option<FelicitousOutcome> {
        // Referent holes carried by this parse (tested on the raw, pre-reduction sem).
        let present: Vec<String> = hole_bases
            .iter()
            .filter(|h| exp_mentions_var(&it.sem, h))
            .cloned()
            .collect();
        let expected = denote_cat(&it.cat).ok()?;
        let expected_val = eval(&expected, &Rho::Nil).ok()?;
        // Evaluate the assembled sem with each freshened hole base bound to a generic neutral
        // (else Pure eval errors on the free var). `Neut::Gen(0, base)` reads back as
        // `Var("{base}0")`, so the holes in the normal form carry that suffixed name.
        let mut eval_rho = Rho::Nil;
        for h in &present {
            eval_rho = eval_rho.extend(Patt::Var(h.clone()), Val::Nt(Neut::Gen(0, h.clone())));
        }
        // STEP-TIMING instrumentation (set `EIGENIUS_PARSE_DEBUG=1`): each step is flushed BEFORE
        // it runs, so the last line printed before an OOM/SIGKILL names the exploding step
        // (eval / readback / check) — the felicity gate is the witnessed full-lexicon blow-up site.
        let dbg = std::env::var("EIGENIUS_PARSE_DEBUG").is_ok();
        if dbg {
            eprintln!("    [felicity] eval start");
        }
        let evaled = eval(&it.sem, &eval_rho).ok()?;
        if dbg {
            eprintln!("    [felicity] readback start");
        }
        let nf = readback_val(0, &evaled);
        // Check the normal form under a context binding each (readback-named) hole in BOTH
        // `rho` (a neutral value — `check` evaluates subterms, which would otherwise error on
        // the free var) and `gamma` (its type — `Entity`, Slice-1).
        let holes: Vec<String> = present.iter().map(|h| format!("{h}0")).collect();
        let mut chk_rho = Rho::Nil;
        let mut gamma: Gamma = Vec::new();
        for hn in &holes {
            chk_rho = chk_rho.extend(Patt::Var(hn.clone()), Val::Nt(Neut::Gen(0, hn.clone())));
            gamma.push((hn.clone(), entity.clone()));
        }
        let mut ctx = CheckCtx::with_layer(chk_rho, gamma, Arc::clone(&self.layer));
        if dbg {
            eprintln!("    [felicity] check start");
        }
        check(&mut ctx, &nf, &expected_val).ok()?;
        let item = Item {
            cat: it.cat.clone(),
            sem: nf,
            prov: it.prov,
            cost: it.cost,
        };
        if holes.is_empty() {
            Some(FelicitousOutcome::Closed(item))
        } else {
            // Slice 1: every hole is an `Entity`-typed referent (`EntityRef`). `ty` carries the
            // Entity class so a resolver can type-filter candidates; `ProofObligation` holes
            // (factive) will carry a parse-computed `Prop` here instead.
            let infos = holes
                .into_iter()
                .map(|var| HoleInfo {
                    var,
                    ty: Exp::EigonClass(iri(ENTITY_IRI)),
                    kind: HoleKind::EntityRef,
                })
                .collect();
            Some(FelicitousOutcome::Open(OpenParse { item, holes: infos }))
        }
    }

    /// Resolve an [`OpenParse`] by substituting each hole with a proposed antecedent and
    /// **re-gating** through the kernel (D64 §4 — the trusted half of anaphora resolution; the
    /// untrusted proposer only ever *suggests* antecedents). `bindings` maps a hole's
    /// [`HoleInfo::var`] to its antecedent term (e.g. `EigonResource`/`EigonClass` for a chain
    /// entity). Each hole is bound to the antecedent's *value* during evaluation, so the
    /// resulting normal form is **closed**; it is then checked to inhabit `⟦cat⟧`. Returns the
    /// resolved closed [`Item`] iff every hole is bound and the closed term type-checks — a
    /// type-mismatched antecedent (e.g. a `Gene` where the predicate needs a `CellLine`) makes
    /// the check fail and yields `None`, exactly the kernel veto that keeps the LLM from having
    /// the last word. A leftover (unbound) hole likewise fails closed.
    pub fn resolve_open(&self, open: &OpenParse, bindings: &[(String, Exp)]) -> Option<Item> {
        let mut rho = Rho::Nil;
        for (var, ante) in bindings {
            let v = eval(ante, &Rho::Nil).ok()?;
            rho = rho.extend(Patt::Var(var.clone()), v);
        }
        let nf = readback_val(0, &eval(&open.item.sem, &rho).ok()?);
        let expected = denote_cat(&open.item.cat).ok()?;
        let expected_val = eval(&expected, &Rho::Nil).ok()?;
        // Closed re-gate: empty Γ, so any leftover hole is an unbound variable ⇒ fail closed.
        let mut ctx = CheckCtx::with_layer(Rho::Nil, Vec::new(), Arc::clone(&self.layer));
        check(&mut ctx, &nf, &expected_val).ok()?;
        Some(Item {
            cat: open.item.cat.clone(),
            sem: nf,
            prov: open.item.prov,
            cost: open.item.cost,
        })
    }

    /// Resolve **every** hole of an [`OpenParse`] via an (untrusted) [`Proposer`], substituting
    /// and re-gating through the kernel (D64 §4, the resolve loop). For each hole the proposer
    /// is asked, given the sentence and the in-scope `candidates`, for a **ranked** list of
    /// antecedent IRIs; the loop searches those assignments (depth-first, bounded by the
    /// proposer's list lengths) and returns the first whole-parse assignment the kernel re-gates
    /// to a closed `Prop`. **Fail-closed**: a hole the proposer leaves empty, or whose every
    /// candidate the kernel vetoes (type mismatch), yields `None` — no committed parse. The
    /// proposer never decides felicity; [`Self::resolve_open`] (the kernel) does.
    pub fn resolve_with(
        &self,
        open: &OpenParse,
        sentence: &str,
        candidates: &[Candidate],
        proposer: &dyn Proposer,
    ) -> Option<Item> {
        let mut ranked: Vec<Vec<Exp>> = Vec::with_capacity(open.holes.len());
        for hole in &open.holes {
            let picks = proposer.propose(&ProposeCtx {
                sentence,
                hole,
                candidates,
            });
            let antes: Vec<Exp> = picks
                .iter()
                .filter_map(|iri| self.antecedent_exp(iri))
                .collect();
            if antes.is_empty() {
                return None; // unresolvable / unknown antecedent ⇒ fail closed
            }
            ranked.push(antes);
        }
        self.search_resolve(open, &ranked, &mut Vec::new())
    }

    /// Depth-first search over per-hole ranked antecedents: assign one antecedent per hole, then
    /// re-gate the whole assignment via [`Self::resolve_open`]; the first that type-checks closed
    /// wins, and a kernel veto backtracks to the next candidate (the trust boundary driving
    /// retry). Bounded by the proposer's list lengths.
    fn search_resolve(
        &self,
        open: &OpenParse,
        ranked: &[Vec<Exp>],
        acc: &mut Vec<(String, Exp)>,
    ) -> Option<Item> {
        let i = acc.len();
        if i == ranked.len() {
            return self.resolve_open(open, acc);
        }
        for ante in &ranked[i] {
            acc.push((open.holes[i].var.clone(), ante.clone()));
            if let Some(it) = self.search_resolve(open, ranked, acc) {
                return Some(it);
            }
            acc.pop();
        }
        None
    }

    /// The antecedent term for a chain-entity IRI: an `EigonResource` (named entity), `EigonClass`
    /// (a class), or `EigonAxiom`, per the entity's kind. `None` if the IRI does not resolve in
    /// the chain (so a hallucinated antecedent fails closed before re-gating).
    fn antecedent_exp(&self, iri: &Iri) -> Option<Exp> {
        self.layer.resolve(iri)?;
        Some(super::lexicon::resolve_sem(&self.layer, iri))
    }
}

/// A candidate antecedent for anaphora resolution (D64 §4): an in-scope committed chain entity,
/// with its surface form for the proposer to rank against. The resolver assembles these from the
/// discourse context; the (untrusted) [`Proposer`] ranks/selects, and the kernel re-gates.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub iri: Iri,
    pub surface: String,
}

/// The context handed to a [`Proposer`] for one referent hole: the sentence, the hole (its type
/// + kind), and the in-scope candidate antecedents.
pub struct ProposeCtx<'a> {
    pub sentence: &'a str,
    pub hole: &'a HoleInfo,
    pub candidates: &'a [Candidate],
}

/// The **untrusted** anaphora proposer (D64 §4): given a hole and the in-scope candidates, return
/// a **ranked** list of antecedent IRIs (most-preferred first; empty ⇒ unresolvable). It only
/// *suggests*; the kernel re-gates every suggestion ([`LexicalIndex::resolve_open`]). Impls: a
/// deterministic mock (tests), a feature-gated live LLM client (`allms`), and the production
/// orchestrator bridge — all behind this one trait, so the algorithm is impl-agnostic.
pub trait Proposer {
    fn propose(&self, ctx: &ProposeCtx) -> Vec<Iri>;
}

/// IRI of the referent-hole placeholder constant (`axiom lexicon:anaphor : lexicon:Entity`):
/// a pronoun entry stores this, and the lookup bridge freshens it into a per-occurrence free
/// variable at parse time (D64 open-parse carrier).
const ANAPHOR_IRI: &str = "urn:eigenius:lexicon:anaphor";
/// IRI of the universal entity class — the type of a (Slice-1) referent hole.
const ENTITY_IRI: &str = "urn:eigenius:lexicon:Entity";

/// Base name of the referent-hole free variable for a pronoun/possessive spanning tokens
/// `[i, j]`. Position-keyed, so distinct occurrences are distinct holes.
fn hole_base(i: usize, j: usize) -> String {
    format!("$anaphor${i}_{j}")
}

/// Replace every `lexicon:anaphor` placeholder in `exp` with the free variable `fresh` (the
/// referent-hole freshening, D64). The anaphor is a leaf constant (no binders to capture), so
/// this is a plain structural replace. It appears only in authored pronoun sems (the whole
/// sem) and possessive-determiner sems (nested inside the λ — `poss_of(A, x, anaphor)`); the
/// compound forms those traverse are covered below, and every other form is returned
/// unchanged (no anaphor occurs there).
fn freshen_anaphor(exp: &Exp, fresh: &str) -> Exp {
    let go = |e: &Exp| freshen_anaphor(e, fresh);
    match exp {
        Exp::EigonAxiom(a) if a.as_str() == ANAPHOR_IRI => Exp::Var(fresh.to_string()),
        Exp::App(f, x) => Exp::App(Box::new(go(f)), Box::new(go(x))),
        Exp::Lam(p, b) => Exp::Lam(p.clone(), Box::new(go(b))),
        Exp::Pi(p, a, b) => Exp::Pi(p.clone(), Box::new(go(a)), Box::new(go(b))),
        Exp::Sig(p, a, b) => Exp::Sig(p.clone(), Box::new(go(a)), Box::new(go(b))),
        Exp::Arrow(a, b) => Exp::Arrow(Box::new(go(a)), Box::new(go(b))),
        Exp::Times(a, b) => Exp::Times(Box::new(go(a)), Box::new(go(b))),
        Exp::Fst(e) => Exp::Fst(Box::new(go(e))),
        Exp::Snd(e) => Exp::Snd(Box::new(go(e))),
        Exp::Pair(a, b) => Exp::Pair(Box::new(go(a)), Box::new(go(b))),
        Exp::Ann(e, t) => Exp::Ann(Box::new(go(e)), Box::new(go(t))),
        other => other.clone(),
    }
}

/// What a referent hole dispatches to once resolved (the carrier's resolver tag — D64). Slice 1
/// produces only `EntityRef` (pronoun/possessive referents → the D64 anaphora resolver);
/// `ProofObligation` (factive presupposition → grounding) is the planned second arm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HoleKind {
    /// An unresolved entity referent (a pronoun / possessor), resolved by substituting a chain
    /// antecedent and re-gating.
    EntityRef,
}

/// One referent hole in an [`OpenParse`]: the free variable standing in the sem, the EigenTT
/// type it must inhabit (Slice 1: `Entity`), and its resolver [`HoleKind`]. This is what a
/// `Proposer` consumes (to filter/rank antecedents) and what [`LexicalIndex::resolve_open`]
/// fills.
#[derive(Clone, Debug)]
pub struct HoleInfo {
    pub var: String,
    pub ty: Exp,
    pub kind: HoleKind,
}

/// An **open** parse (D64): a felicitous full-span `S` whose sem still carries unresolved
/// referent holes (free variables). Each [`HoleInfo`] is a slot the D64 resolver fills (by
/// substituting a chain antecedent + re-gating — [`LexicalIndex::resolve_open`]). The kernel
/// type-checked `item.sem` with each hole bound to its type; it is NOT a closed final parse.
#[derive(Clone)]
pub struct OpenParse {
    pub item: Item,
    pub holes: Vec<HoleInfo>,
}

/// The outcome of classifying a full-span candidate (see [`LexicalIndex::classify_felicitous`]).
enum FelicitousOutcome {
    Closed(Item),
    Open(OpenParse),
}

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("valid lexicon iri")
}

/// Candidate adjective bases for a productive `-ly` adverb (D62 Phase 3 derivational rule).
/// Orthographic reverse-derivation; each candidate is probed against the lexicon (data-driven — no
/// hardcoded adverb list), so over-generated noise (a non-word base) simply fails the probe.
fn adverb_bases(surface: &str) -> Vec<String> {
    // Non-adverb `-ly` tokens: chemistry/maths fragments mis-split by S0 (`poly(ADP-ribose)` →
    // `poly`) that happen to end in `-ly` but are never adverbs. An explicit exception so they are
    // not derived even if a base accidentally resolves.
    const NON_ADVERB_LY: &[&str] = &["poly"];
    if surface.len() < 4 || !surface.ends_with("ly") || NON_ADVERB_LY.contains(&surface) {
        return Vec::new();
    }
    let stem = &surface[..surface.len() - 2]; // strip "ly": commonly→common, selectively→selective
    let mut bases = vec![stem.to_string(), format!("{stem}le")]; // stem+le: simply→simple
    if let Some(p) = surface.strip_suffix("ily") {
        bases.push(format!("{p}y")); // -ily→-y: easily→easy
    }
    if let Some(p) = surface.strip_suffix("bly") {
        bases.push(format!("{p}ble")); // -bly→-ble: favourably→favourable
    }
    match surface {
        "truly" => bases.push("true".to_string()),
        "fully" => bases.push("full".to_string()),
        "wholly" => bases.push("whole".to_string()),
        _ => {}
    }
    bases.sort();
    bases.dedup();
    bases
}

/// Lexicalized (non-`-ly`) transparent **discourse adverbs** (D62 connectives batch): closed-class
/// adverbs that don't derive from an adjective but are inert for a scientific claim, so they get the
/// same transparent treatment as `-ly` adverbs (plus clause-level `S/S`/`S\S` attachment).
fn is_lexicalized_adverb(surface: &str) -> bool {
    const LEXICALIZED_ADVERBS: &[&str] = &["also", "however", "yet"];
    LEXICALIZED_ADVERBS.contains(&surface)
}

/// Whether a category is a **predicative adjective** `S[adj]\NP` — `bwd(cat_s(_, adj), _)`. Used to
/// confirm a derived `-ly` adverb's base is a known adjective (D62 Phase 3).
fn is_adjective_cat(cat: &Exp) -> bool {
    if let Some([s, _np]) = is_ctor(cat, "bwd") {
        if let Some([_mood, fin]) = is_ctor(s, "cat_s") {
            return matches!(fin, Exp::InductiveCtor(_, n, _) if n == "adj");
        }
    }
    false
}

/// The sort key the per-lemma sense cap (D63 §8.7 / GH #97) truncates by: contextually-ranked
/// senses first (ordered by the reranker's `ranks` position), then the rest by static `sense_rank`
/// (most-frequent first). The leading `bool` puts `Some(ctx)` (`false`) ahead of unranked
/// (`true`). With `ranks = None` every sense is unranked, collapsing to the pure-`sense_rank`
/// order — the behaviour-identical static cap.
/// Cap a CKY chart cell to its `beam` lowest-[`Cost`] items (Lever B — per-cell beam, GH #97),
/// returning how many were dropped. A **stable** sort by `Cost` keeps the cheapest
/// (most-frequent-sense / preferred-lexicon) derivations and preserves insertion order within a
/// cost tie (so closed-class / cost-0 cells are order-preserved and deterministic). Inexact: a
/// dropped constituent may have been the only route to a full parse — the beam/A* tradeoff, why the
/// beam is opt-in.
fn beam_cell(cell: &mut Vec<Item>, beam: usize) -> usize {
    if cell.len() <= beam {
        return 0;
    }
    let dropped = cell.len() - beam;
    cell.sort_by_key(|it| it.cost);
    cell.truncate(beam);
    dropped
}

fn sense_cap_key(e: &SeedEntry, ranks: Option<&BTreeMap<String, u32>>) -> (bool, u32, u32) {
    let ctx = e
        .sense
        .as_ref()
        .and_then(|s| ranks.and_then(|m| m.get(s).copied()));
    (ctx.is_none(), ctx.unwrap_or(0), e.item.cost.sense_rank)
}

/// Forward bounded type-raise (D63 §8.9 Slice 6-T) every name `NP` in a cell's items
/// to `S/(S\NP)`, tagged `Combinator::TypeRaised` so ENF lets it only *compose*.
/// Non-`NP` items (functors, groups, kinds, determined NPs) yield nothing.
fn raise_nps(items: &[Item], layer: &Arc<Layer>) -> Vec<Item> {
    items
        .iter()
        .filter_map(|it| {
            type_raise(&it.cat, &it.sem, layer).map(|(cat, sem)| Item {
                cat,
                sem,
                prov: Combinator::TypeRaised,
                cost: it.cost,
            })
        })
        .collect()
}

/// A complete clause root must be **finite**: `cat_s(_, fin | fin_any)`. A base /
/// infinitival clause (`cat_s(_, bse)` — the VP an auxiliary selects) is never a
/// standalone sentence (D63 §8.5, Slice 5a). Non-`cat_s` categories are not clauses.
fn is_finite_clause(cat: &Exp) -> bool {
    match is_ctor(cat, "cat_s") {
        Some([_mood, fin]) => {
            matches!(fin, Exp::InductiveCtor(_, n, _) if n == "fin" || n == "fin_any")
        }
        _ => false,
    }
}

/// The `Prop`-connective IRI a coordinating conjunction contributes, or `None` if the
/// token is not a coordinator. These words are consumed by the parser's **coordination
/// rule** (D63 §8.4), not via lexical entries — coordination is polymorphic over `Cat`,
/// which `⟦·⟧` cannot denote. This is the single source of truth shared by the chart's
/// `coord_op` table and the missing-lexeme signal [`LexicalIndex::has_token`] (so the
/// pipeline never routes a structurally-handled connective to lexical recovery).
///
/// The symmetric coordinators `and`/`or`, plus the **list comma** `,` (D62 S0): a comma in a
/// multi-item list (`A, B, C and D`) is a conjunctive separator, so it maps to `And` and the existing
/// left-branching coordination builds the member group (`A, B` → group, `… and D` closes it). This is
/// **structural** coverage; a mixed-connective list (`A, B or C`) has its comma-joined members
/// approximated as `And` (the disjunction surfaces only at the final `or`) — a recorded approximation,
/// faithful per-list connective propagation (core-en's list-completion, `conj.xsl`) is the follow-on.
///
/// Contrastive `but` is NOT here: core-en gives it its own family with a distinct `but(Arg1, Arg2)`
/// relation — collapsing it to `And` would drop the adversative relation; deferred to the
/// subordinator/discourse-connective work.
pub fn coord_connective(token: &str) -> Option<&'static str> {
    match token {
        "and" | "," => Some("urn:eigenius:logic:And"),
        "or" => Some("urn:eigenius:logic:Or"),
        _ => None,
    }
}

/// Whether a sem was produced by the coordination rule — i.e. (after peeling any
/// `λ`s from a `VP`/`TV` pointwise lift) it is headed by `logic:And`/`logic:Or`.
/// In this grammar those connectives arise *only* from coordination, so this is
/// the derivation marker the left-branching normal form keys on (D63 §8.4 Ph 4).
fn is_coordination(sem: &Exp) -> bool {
    let mut e = sem;
    while let Exp::Lam(_, body) = e {
        e = body;
    }
    matches!(e, Exp::InductiveType(decl, _)
        if matches!(decl.iri.as_str(), "urn:eigenius:logic:And" | "urn:eigenius:logic:Or"))
}

/// Instantiate a common noun's underspecified `num_any` with the surface number.
/// Only a `cat_n(T, num_any)` item is refined (to `cat_n(T, <num>)`); verbs,
/// names, and multiword leaves pass through unchanged. The `lexicon:Num` decl is
/// reused from the existing `num_any` ctor, so no decl lookup is needed.
fn with_noun_num(it: &Item, num_name: &str) -> Item {
    if let Exp::InductiveCtor(decl, name, args) = &it.cat {
        if name == "cat_n" && args.len() == 2 {
            if let Exp::InductiveCtor(num_decl, n, _) = &args[1] {
                if n == "num_any" {
                    let num =
                        Exp::InductiveCtor(num_decl.clone(), num_name.to_string(), Vec::new());
                    return Item {
                        cat: Exp::InductiveCtor(
                            decl.clone(),
                            name.clone(),
                            vec![args[0].clone(), num],
                        ),
                        sem: it.sem.clone(),
                        prov: it.prov,
                        cost: it.cost,
                    };
                }
            }
        }
    }
    it.clone()
}

#[cfg(test)]
mod tests {
    use super::tokenize;

    #[test]
    fn tokenize_lowercases_and_strips_edge_punctuation() {
        assert_eq!(
            tokenize("HeLa depends on BRCA1."),
            ["hela", "depends", "on", "brca1"]
        );
        // The comma between content tokens is preserved as a `,` token (D62 S0 list coordination).
        assert_eq!(tokenize("  A,  b!  "), ["a", ",", "b"]);
        assert!(tokenize("   ").is_empty());
        assert!(tokenize("").is_empty());
    }

    #[test]
    fn tokenize_preserves_list_commas_and_drops_dangling() {
        // Internal commas survive as separators; leading/trailing/duplicate commas are dropped.
        assert_eq!(
            tokenize("a, b, c and d"),
            ["a", ",", "b", ",", "c", "and", "d"]
        );
        assert_eq!(tokenize("a,, b,"), ["a", ",", "b"]); // collapsed run + trailing dropped
        assert_eq!(tokenize(", a"), ["a"]); // leading dropped
    }

    #[test]
    fn tokenize_keeps_internal_alphanumerics() {
        // intra-token digits/letters survive; only the edges are trimmed. The `(BRCA1)` is now a
        // dropped parenthetical aside (D62 S0), so only `p53` survives.
        assert_eq!(tokenize("p53, (BRCA1)"), ["p53"]);
    }

    #[test]
    fn tokenize_drops_bracketed_asides() {
        // Parenthetical gloss dropped, head + matrix kept.
        assert_eq!(
            tokenize("microsatellite instability (MSI) results"),
            ["microsatellite", "instability", "results"]
        );
        // Nested parens dropped wholesale.
        assert_eq!(
            tokenize("poly(ADP(x)-ribose) polymerase"),
            ["poly", "polymerase"]
        );
        // Paired em-dash appositive dropped; head + matrix kept.
        assert_eq!(
            tokenize("lethality\u{2014}an interaction here\u{2014}can be exploited"),
            ["lethality", "can", "be", "exploited"]
        );
        // A single (unpaired) em-dash is NOT a bracket pair → split, both sides kept.
        assert_eq!(tokenize("not\u{2014}can"), ["not", "can"]);
    }
}
