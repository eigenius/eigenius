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
    adverb_modifier_cats, cats_coordinate, coordinate_but_not, coordinate_but_not_sem,
    coordinate_np, coordinate_sem, denote_cat, front_participial, is_ctor, pied_pipe, reciprocate,
    relativize, relativize_appos, sentence_modifier_cats, subst_cat, type_raise, CatSubst,
};
use super::lemmatizer::{Lemmatizer, Pos};
use super::lexicon::entry_to_item;
use super::parser::{apply, apply_core, Combinator, Item};
use super::reserved;
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

/// Upper bound for widen-on-failure of the **cell beam** (GH #97 Lever 2): when a capped parse of an
/// all-known-vocabulary sentence yields nothing, the per-cell beam is doubled (alongside the sense
/// cap) up to this many items per cell, then the attempt is abandoned. This pays the wider beam ONLY
/// for known sentences that need the structural headroom (measured: the CNL's grammar-complete
/// sentences cross at beam 128–256), while sentences that parse at the base beam never widen — so the
/// base beam stays the OOM defense for the long-sentence common case. Bounded (not uncapped) so a
/// genuinely intractable sentence can't re-OOM the chart.
pub const CELL_BEAM_WIDEN_MAX: usize = 512;

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
    /// **Combinatory-core spike** (porting core-en's full rule set): when set, the CKY also applies
    /// the additional CCG combinators — crossed composition (`>Bx`/`<Bx`), backward harmonic
    /// composition (`<B`), and generalized type-raising — alongside the hand-built rules, to measure
    /// how much of the composition long tail the general combinators subsume. Default `false` = the
    /// established rule-by-rule path, byte-identical. (Spike: normal-form control is partial; expect
    /// extra ambiguity until the NF is rebuilt.)
    combinatory_core: bool,

    /// **Cross-POS prune** experiment (GH#97): when a surface token has a CLOSED-class (grammatical,
    /// `in_lexicon = None`) reading — it's a known function word — drop its open-class **nominal**
    /// (`cat_n`/`cat_np`) readings, the dense-lexicon noise that feeds the compound rule (`can`→
    /// container, `for`→noun, `is`→beryllium) into the sentence-spanning noun-pile. Open-class VERB/
    /// ADJ readings are KEPT (so `is`→the `be`-verb copula survives — the case blanket closed-class-
    /// wins wrongly killed). Acts at seed time, so widen-on-failure can't re-admit the dropped nouns.
    /// Opt-in; default off.
    pos_prune: bool,

    /// **Packed-forest parsing** (D63 blueprint, GH#97 Option A): when set (and the grammar is
    /// index-independent — no selectional functor slots — and `combinatory_core` is off), a parse is
    /// routed to the node-level packed CKY + cube-pruning extractor instead of the flat beamed chart.
    /// Packing collapses the same-`cat_shape` sense-product into one node per `(cat_shape, ENF-prov)`,
    /// so combination is O(1) per node-pair; selectional restrictions (if any) are deferred to the
    /// felicity pop-filter. The router falls back to the unpacked path for selectional grammars
    /// (the guard, [`Self::seeds_have_selectional_slot`]). Opt-in; default off (no behaviour change).
    packing: bool,
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
                combinatory_core: false,
                pos_prune: false,
                packing: false,
            };
        }
        let (by_form, max_words) = Self::scan_eager(&layer);
        LexicalIndex {
            layer,
            source: Source::Eager { by_form, max_words },
            sense_cap: None,
            sense_ranker: None,
            cell_beam: None,
            combinatory_core: false,
            pos_prune: false,
            packing: false,
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

    /// Enable the **combinatory-core spike**: apply the additional CCG combinators (crossed +
    /// backward-harmonic composition, generalized type-raising) alongside the hand-built rules.
    /// Builder-style; default off (the established rule-by-rule path). For the A/B port measurement.
    pub fn with_combinatory_core(mut self, on: bool) -> Self {
        self.combinatory_core = on;
        self
    }

    /// Enable the **cross-POS prune** experiment (GH#97): drop a function word's open-class nominal
    /// readings (see the `pos_prune` field doc). Builder-style; default off.
    pub fn with_pos_prune(mut self, on: bool) -> Self {
        self.pos_prune = on;
        self
    }

    /// Enable **packed-forest parsing** ([`Self::packing`]) — node-level packing + cube-pruning
    /// extraction, gated at parse time on the grammar being index-independent. Builder-style; default off.
    pub fn with_packing(mut self, on: bool) -> Self {
        self.packing = on;
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

    /// Diagnostic (D62/GH#97 function-word-noise analysis): every resolved entry for `surface`
    /// (raw lowercased + each lemma across POS), tagged **closed-class** (`in_lexicon = None`, the
    /// grammatical core) vs **open-class** (a wordnet/umls sense). Returns `(closed_class, cat,
    /// sense)` per entry. Used to enumerate the spurious open-class noun senses that function words
    /// (`is`/`an`/`a`/`between`) pick up from the dense lexicon and feed into the compound rule.
    pub fn debug_form_entries(
        &self,
        surface: &str,
        lemmatizer: &dyn Lemmatizer,
    ) -> Vec<(bool, String, String)> {
        let mut out = Vec::new();
        let mut seen: BTreeSet<(bool, String, String)> = BTreeSet::new();
        for cand in self.candidate_lemmas(surface, lemmatizer) {
            for e in self.scoped(self.entries_for(&cand), None) {
                let row = (
                    e.in_lexicon.is_none(),
                    super::pretty_term(e.item.cat()),
                    e.sense.clone().unwrap_or_default(),
                );
                if seen.insert(row.clone()) {
                    out.push(row);
                }
            }
        }
        out
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
                        e.item.category.cost.lexicon_order = pos as u32;
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
        // Cross-POS prune (GH#97): a surface that is a known function word is a lexicon artifact when
        // read as a noun; drop the open-class NOMINAL readings of all its candidate lemmas (the
        // compound-rule noise), keeping closed-class + open-class verb/adj/etc. A surface counts as a
        // function word if (a) it ITSELF carries a closed-class grammatical reading, OR (b) it is a
        // MULTI-TOKEN surface every token of which does — i.e. a multi-word UMLS entry spanning only
        // function words (e.g. the concept "Do not" = `cat_n(C3840725)` over the tokens "do"+"not").
        // Case (b) is the whole-sentence noun-pile BRIDGE (GH#97 dump 2026-06-30): per-token prune
        // kills each function word's noun reading, but a bilexical stop-word concept bypassed the
        // single-surface check, so every clause with such a span folded into one giant compound noun.
        // The LLM reranker cannot remove it (a sole reading is never cap-truncated), so the prune must.
        let has_closed = |surf: &str| {
            self.entries_for(surf)
                .iter()
                .any(|e| e.in_lexicon.is_none())
        };
        let surface_is_function = self.pos_prune
            && (has_closed(&s_lc) || {
                let toks: Vec<&str> = s_lc.split_whitespace().collect();
                toks.len() > 1 && toks.iter().all(|t| has_closed(t))
            });
        let mut out = Vec::new();
        for c in &candidates {
            let mut entries = self.scoped(self.entries_for(c), scope);
            if surface_is_function {
                entries.retain(|e| {
                    e.in_lexicon.is_none()
                        || !(is_ctor(e.item.cat(), "cat_n").is_some()
                            || is_ctor(e.item.cat(), "cat_np").is_some())
                });
            }
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
                crate::dcg::kind_subject(it.cat(), it.sem())
                    .map(|(cat, sem)| Item::with_cost(cat, sem, it.cost()))
            })
            .collect();
        out.extend(kinds);
        // Bare-plural NP shift (D62 — core-en `bnp`, `docs/notes/d62-bare-plural-quantification.md`):
        // a plural common noun also serves as an ARGUMENT NP (subject + object) whose quantifier is
        // deferred to a `Quantification` hole — `det=nil`. (The `kind_subject` edge above is only the
        // copula-subclass reading; this is the general argument-position reading.)
        let bare: Vec<Item> = out.iter().flat_map(|it| self.bare_plural_nps(it)).collect();
        out.extend(bare);
        // Bare-MASS NP shift (D62 CNL): a mass noun `cat_n(C, mass)` ("MSI", "DNA", "apoptosis") is
        // a bare argument too, grammatically singular, with the same deferred quantifier.
        let bare_mass: Vec<Item> = out.iter().flat_map(|it| self.bare_mass_nps(it)).collect();
        out.extend(bare_mass);
        out
    }

    /// Bare-MASS NP shift (D62 CNL): a **mass** common noun `cat_n(C, mass)` serves as a bare argument
    /// NP (subject + object), grammatically **singular**, with a deferred [`HoleKind::Quantification`]
    /// hole `Q` — the mass/uncountable analogue of [`Self::bare_plural_nps`]. The mass noun is
    /// presented as **singular** to the existential determiner (`a`) shapes (mass meets neither sg nor
    /// pl, so it can't use `these`/`a` directly), reusing their subject- + object-raised categories
    /// with the deferred sem. `Q` is freshened per span and typed at the felicity gate ⇒ an open parse.
    fn bare_mass_nps(&self, noun: &Item) -> Vec<Item> {
        let Some([c, num]) = is_ctor(noun.cat(), "cat_n") else {
            return Vec::new();
        };
        if !matches!(num, Exp::InductiveCtor(_, n, _) if n == "mass") {
            return Vec::new();
        }
        // Present the mass noun as singular (so the `a` determiner's sg cat composes).
        let Exp::InductiveCtor(num_decl, _, _) = num else {
            return Vec::new();
        };
        let Exp::InductiveCtor(cat_decl, _, _) = noun.cat() else {
            return Vec::new();
        };
        let sg = Exp::InductiveCtor(num_decl.clone(), "sg".into(), vec![]);
        let sg_cat = Exp::InductiveCtor(cat_decl.clone(), "cat_n".into(), vec![c.clone(), sg]);
        let sg_noun = Item::with_cost(sg_cat, noun.sem().clone(), noun.cost());
        let subj = deferred_quant_subj_sem();
        let obj = deferred_quant_obj_sem();
        self.entries_for("a")
            .iter()
            .filter_map(|det| {
                let sem = match cat_forall_body_head(det.item.cat())? {
                    "fwd" => subj.clone(),
                    "bwd" => obj.clone(),
                    _ => return None,
                };
                let synthetic = Item::with_cost(det.item.cat().clone(), sem, det.item.cost());
                apply(&synthetic, &sg_noun, &self.layer)
            })
            .collect()
    }

    /// Bare-plural NP shift (D62 — core-en's `bnp` unary rule; `det=nil`). A **plural** common noun
    /// `cat_n(C, pl)` also serves as an argument NP whose quantifier is **deferred**: a higher-order
    /// [`HoleKind::Quantification`] hole `Q`. Built by applying the plural-existential determiner's
    /// *category* (`these`) to the noun, but with the determiner sem replaced by
    /// [`deferred_quant_det_sem`] (`λA.λV. Q(A,V)` — ∃ replaced by the hole), so the NP sem is
    /// `λV. Q(C, V)`. Returns the subject + object NP items (`these` carries both raised cats). Gated
    /// on `pl` — core-en's `pl-or-mass` minus mass (a later feature); a bare *singular* count noun
    /// (`*gene is a vulnerability`) correctly does not shift. The `Q` sentinel is freshened per-span
    /// at chart placement and typed at the felicity gate.
    fn bare_plural_nps(&self, noun: &Item) -> Vec<Item> {
        let Some([_c, num]) = is_ctor(noun.cat(), "cat_n") else {
            return Vec::new();
        };
        if !matches!(num, Exp::InductiveCtor(_, n, _) if n == "pl") {
            return Vec::new();
        }
        // The plural-existential determiner (`these`) supplies the subject + object NP categories;
        // we keep its cat and swap in the matching deferred-quantifier sem. The subject determiner's
        // (post-`cat_forall`) body is headed `fwd` (a type-raised `S/(S\NP)`), the object's `bwd`
        // (the in-situ object raise) — so the body head selects the subject vs object deferred sem.
        // Depends on `these` being a loaded plural existential determiner (closed class).
        let subj = deferred_quant_subj_sem();
        let obj = deferred_quant_obj_sem();
        self.entries_for("these")
            .iter()
            .filter_map(|det| {
                let sem = match cat_forall_body_head(det.item.cat())? {
                    "fwd" => subj.clone(),
                    "bwd" => obj.clone(),
                    _ => return None,
                };
                let synthetic = Item::with_cost(det.item.cat().clone(), sem, det.item.cost());
                apply(&synthetic, noun, &self.layer)
            })
            .collect()
    }

    /// Object-position non-restrictive (appositive) relative NP (D62 §2 #2A, object slot): the
    /// antecedent NP `cat_np(C, _)` + a comma-set-off relative `, which/that [body]` raised into a
    /// transitive verb's OBJECT slot (mirroring `a_obj`), conjoining the appositive assertion —
    /// `(S\NP)\((S\NP)/NP)` with sem `λTV. λs. logic:And(TV(r)(s), body(r))`. Reuses the `a` object
    /// determiner's raised cat (instantiating its bound `T := C`), as [`Self::bare_plural_nps`]
    /// reuses `these`, so it composes with any transitive verb. The SUBJECT-position appositive is
    /// [`relativize_appos`] (type-raised `S/(S\NP)`); prep-object position rides that subject form
    /// through the GQ-as-preposition-object rule. `None` unless the antecedent is a `cat_np`, the body
    /// a declarative `S/NP`/`S\NP`, the `a_obj` cat is loaded, and `logic:And` resolves.
    fn appositive_obj(&self, ante: &Item, body: &Item) -> Option<Item> {
        let [c, _num] = is_ctor(ante.cat(), "cat_np")? else {
            return None;
        };
        let body_args = is_ctor(body.cat(), "fwd").or_else(|| is_ctor(body.cat(), "bwd"))?;
        let [s, _np] = body_args else {
            return None;
        };
        if !matches!(is_ctor(s, "cat_s"),
            Some([mood, _]) if matches!(mood, Exp::InductiveCtor(_, n, _) if n == "dcl"))
        {
            return None;
        }
        let and = super::category::resolve_inductive(&self.layer, "urn:eigenius:logic:And")?;
        // The `a` object determiner's raised cat `cat_forall(sg, λT. (S\NP)\((S\NP)/NP_T))` (the
        // `bwd`-headed body); instantiate `T := C` for this antecedent's class.
        let entries = self.entries_for("a");
        let det = entries
            .iter()
            .find(|d| cat_forall_body_head(d.item.cat()) == Some("bwd"))?;
        let [_dnum, body_lam] = is_ctor(det.item.cat(), "cat_forall")? else {
            return None;
        };
        let Exp::Lam(Patt::Var(tvar), obj_body) = body_lam else {
            return None;
        };
        let mut subst = CatSubst::new();
        subst.insert(tvar.clone(), c.clone());
        let cat = subst_cat(obj_body, &subst);
        // sem: λTV. λsubj. And(TV(r)(subj), body(r)) — the in-situ object raise conjoining the
        // appositive assertion on the antecedent referent `r`.
        let (tv, sj) = ("__appos_tv", "__appos_s");
        let r = ante.sem().clone();
        let tv_r_s = Exp::App(
            Box::new(Exp::App(Box::new(Exp::Var(tv.into())), Box::new(r.clone()))),
            Box::new(Exp::Var(sj.into())),
        );
        let body_r = Exp::App(Box::new(body.sem().clone()), Box::new(r));
        let sem = Exp::Lam(
            Patt::Var(tv.into()),
            Box::new(Exp::Lam(
                Patt::Var(sj.into()),
                Box::new(Exp::InductiveType(and, vec![tv_r_s, body_r])),
            )),
        );
        Some(Item::with_cost(
            cat,
            sem,
            ante.cost().saturating_add(body.cost()),
        ))
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
                .any(|e| is_adjective_cat(e.item.cat()))
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

    /// Degree-modified adverb items (D62 §2 #5b — `more commonly`, `most notably`, `less frequently`):
    /// a degree word (`more`/`most`/`less`) over a known adverb (derived `-ly` or lexicalized) forms a
    /// transparent **sentence** adverb. Two-token span; the degree contributes nothing to the claim
    /// (transparent, like the bare adverb), so the whole phrase reuses [`sentence_modifier_cats`] +
    /// [`adverb_modifier_cats`] with the identity sem. Empty unless `w0` is a degree word and `w1` a
    /// recognized adverb.
    fn degree_adverb_items(&self, w0: &str, w1: &str) -> Vec<Item> {
        let d = w0.trim().to_lowercase();
        if !matches!(d.as_str(), "more" | "most" | "less" | "least") {
            return Vec::new();
        }
        let a = w1.trim().to_lowercase();
        if !self.is_derived_adverb(&a) && !is_lexicalized_adverb(&a) {
            return Vec::new();
        }
        let mut cats = adverb_modifier_cats(&self.layer).unwrap_or_default();
        cats.extend(sentence_modifier_cats(&self.layer).unwrap_or_default());
        if cats.is_empty() {
            return Vec::new();
        }
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
    /// (`with_sense_cap`) and **cell beam** (`with_cell_beam`) with **widen-on-failure** (GH #97): try
    /// at the base cap+beam; if it yields *no* parse at all (closed and open both empty) **and** the
    /// failure could be a pruning artifact — i.e. every (prose) token is lexically known, so it is not
    /// an OOV miss — retry with **both** doubled (sense cap up to [`SENSE_CAP_WIDEN_MAX`], cell beam up
    /// to [`CELL_BEAM_WIDEN_MAX`]). So neither the cap (a dropped sense) nor the beam (a dropped
    /// structural constituent — the dominant blocker for the grammar-complete CNL sentences, which
    /// cross at beam 128–256) ever *loses* a parse a known-vocabulary sentence would get, while
    /// OOV-blocked sentences don't waste retries and sentences that parse at the base settings never
    /// pay the wider ones. Escalating both each round bounds the retries to ~log2 of the wider span.
    pub fn parse_scoped_open(
        &self,
        text: &str,
        lemmatizer: &dyn Lemmatizer,
        scope: Option<&[Iri]>,
    ) -> (Vec<Item>, Vec<OpenParse>) {
        // ROUTER (D63 Option A, blueprint §11 3b.3): route to the packed CKY + cube-pruning extractor
        // when packing is enabled, the combinatory-core spike is off, and this sentence is
        // index-independent (the guard — no selectional functor slot, no coordination). Otherwise the
        // unpacked beamed path (also the fallback for selectional lexicons and the oracle baseline).
        if self.packing
            && !self.combinatory_core
            && !self.parse_needs_unpacked(&tokenize(text), lemmatizer, scope)
        {
            return self.parse_packed(text, lemmatizer, scope);
        }
        self.parse_unpacked(text, lemmatizer, scope)
    }

    /// Per-parse index-independence guard (D63 Option A, blueprint §11 3b.2). Returns `true` if this
    /// sentence must use the UNPACKED path. As of §11 3g.3 the packed CKY mirrors every token-keyed
    /// sem-reading construct (coordination, the reciprocal, `but not`, the restrictive relative, the
    /// appositive, the fronted-modifier comma) plus the wh-determiner `which` (an ordinary leaf), so
    /// only two fail-closed carve-outs remain:
    /// 1. **pied-piping** (`[prep] which [subj] [VP]`) — a ternary rule with no packing benefit (a
    ///    rare, non-piling construct), detected structurally (a `which` right after a VP-adjunct
    ///    preposition) and routed to the proven unpacked path rather than given a ternary edge;
    /// 2. a seeded functor with a concrete SELECTIONAL argument slot
    ///    ([`super::category::cat_has_selectional_slot`]) — combinability would be index-dependent, so
    ///    node-level packing by `cat_shape` is unsound.
    ///
    /// The seed scan is over this sentence's spans only (feasible for the lazy index) and uncapped so
    /// no beyond-cap selectional entry slips through.
    fn parse_needs_unpacked(
        &self,
        tokens: &[String],
        lemmatizer: &dyn Lemmatizer,
        scope: Option<&[Iri]>,
    ) -> bool {
        let n = tokens.len();
        // (1) Pied-piping `[prep] which`: the antecedent noun-pile isn't collapsed by packing (the
        // construct is ternary and rare), so route it unpacked. A `which` right after a VP-adjunct
        // preposition is pied-piping; a `which` after a noun is the packed which-relative, and a
        // sentence-initial / post-determiner `which` is the packed wh-determiner.
        for p in 1..n {
            if tokens[p] != reserved::WHICH {
                continue;
            }
            if self
                .lookup_span(&tokens[p - 1], lemmatizer, scope, None, None)
                .iter()
                .any(|it| is_vp_adjunct_prep(it.cat()))
            {
                return true;
            }
        }
        // (2) A seeded functor with a concrete SELECTIONAL argument slot — combinability would be
        // index-dependent, so node-level packing by `cat_shape` is unsound.
        let span_limit = self.span_limit(n);
        for i in 0..n {
            let last = (i + span_limit).min(n);
            for j in i..last {
                let surface = tokens[i..=j].join(" ");
                for it in self.lookup_span(&surface, lemmatizer, scope, None, None) {
                    if super::category::cat_has_selectional_slot(it.cat()) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Whether an (unscoped) parse of `text` would take the **packed** path (D63 Option A): packing
    /// enabled, no combinatory-core spike, and the sentence index-independent + construct-free (the
    /// [`Self::parse_needs_unpacked`] guard). The routing decision is otherwise unobservable —
    /// packed ≡ unpacked by construction (the differential oracle) — so this exposes it for tests to
    /// assert *which* path a sentence takes (blueprint §11 3f.2).
    pub fn routes_packed(&self, text: &str, lemmatizer: &dyn Lemmatizer) -> bool {
        self.packing
            && !self.combinatory_core
            && !self.parse_needs_unpacked(&tokenize(text), lemmatizer, None)
    }

    /// Packed-forest parse (D63 Option A, blueprint §11 3d): build the packed shared forest
    /// ([`Self::build_forest`]) and extract the top-span k-best via cube pruning ([`Self::kbest`]),
    /// then apply the felicity pop-filter ([`Self::classify_felicitous`]) — routing each survivor to
    /// the closed or open forest, exactly as [`Self::parse_at_cap`] does. Reached only for
    /// index-independent, construct-free sentences (the router's guard), so it is equivalent to
    /// [`Self::parse_unpacked`] on those (the differential oracle, 3f). No widen loop — packing never
    /// drops the needed constituent.
    fn parse_packed(
        &self,
        text: &str,
        lemmatizer: &dyn Lemmatizer,
        scope: Option<&[Iri]>,
    ) -> (Vec<Item>, Vec<OpenParse>) {
        let tokens = tokenize(text);
        let n = tokens.len();
        if n == 0 {
            return (Vec::new(), Vec::new());
        }
        let ranks = self.contextual_sense_ranks(text, lemmatizer, scope);
        let forest = self.build_forest(&tokens, lemmatizer, scope, self.sense_cap, ranks.as_ref());
        let mut memo: Vec<Option<Vec<Item>>> = vec![None; forest.nodes.len()];

        // Top-span candidates: finite-clause / wh-question nodes spanning the whole sentence.
        let top: Vec<super::packed::NodeId> = forest.cells[0][n - 1]
            .values()
            .copied()
            .filter(|&id| {
                let c = forest.nodes[id].rep.cat();
                is_finite_clause(c) || is_ctor(c, "cat_q").is_some()
            })
            .collect();
        let mut candidates: Vec<Item> = Vec::new();
        for id in top {
            candidates.extend(self.kbest(&forest, id, DEFAULT_FOREST_CAP, &mut memo));
        }
        candidates.sort_by_key(|it| it.cost());
        candidates.truncate(CLASSIFY_BUDGET);
        if std::env::var("EIGENIUS_PARSE_DEBUG").is_ok() {
            eprintln!(
                "dcg::parse (packed): {:?} forest nodes={} finite candidates={}",
                text,
                forest.nodes.len(),
                candidates.len()
            );
        }

        // Hole context for classification — identical to the unpacked path.
        let entity_ty = Exp::EigonClass(iri(ENTITY_IRI));
        let quant_ty = quant_hole_type();
        let types_ok = eval(&entity_ty, &Rho::Nil).is_ok() && eval(&quant_ty, &Rho::Nil).is_ok();
        let mut hole_specs: Vec<(String, Exp, HoleKind)> = Vec::new();
        if types_ok {
            for i in 0..n {
                for j in i..n {
                    hole_specs.push((hole_base(i, j), entity_ty.clone(), HoleKind::EntityRef));
                    hole_specs.push((
                        quant_hole_base(i, j),
                        quant_ty.clone(),
                        HoleKind::Quantification,
                    ));
                }
            }
        }

        // Felicity pop-filter → closed / open forests (the only type-check, at the top span).
        let mut forest_out: Vec<Item> = Vec::new();
        let mut open: Vec<OpenParse> = Vec::new();
        for it in &candidates {
            if types_ok {
                match self.classify_felicitous(it, &hole_specs) {
                    Some(FelicitousOutcome::Closed(c)) => forest_out.push(c),
                    Some(FelicitousOutcome::Open(o)) => open.push(o),
                    None => {}
                }
            } else if let Some(c) = self.reduced_felicitous(it) {
                forest_out.push(c);
            }
        }
        forest_out.sort_by_key(|it| it.cost());
        forest_out.truncate(DEFAULT_FOREST_CAP);
        (forest_out, open)
    }

    /// Lazy k-best extraction from a packed-forest node (D63 §11 3d). Merges the node's edges — `Leaf`
    /// (the item), `Combine` (cube pruning over the two children's k-best, materialised by `apply` per
    /// pop in `(cost, li, ri)` order, bounded by `max_pops`), `Unary` (the composed-cell shift applied
    /// to each child item) — then cost-sorts and keeps `k`. Memoised per node (the forest is a DAG by
    /// span). **No felicity here** — the felicity pop-filter runs once at the top span, matching the
    /// unpacked path (which type-checks only the full span).
    fn kbest(
        &self,
        forest: &super::packed::Forest,
        node_id: super::packed::NodeId,
        k: usize,
        memo: &mut Vec<Option<Vec<Item>>>,
    ) -> Vec<Item> {
        if let Some(cached) = &memo[node_id] {
            return cached.clone();
        }
        memo[node_id] = Some(Vec::new()); // DAG re-entrancy guard (no cycles expected).
        let span = forest.nodes[node_id].span;
        let mut cands: Vec<Item> = Vec::new();
        for e in 0..forest.nodes[node_id].edges.len() {
            match &forest.nodes[node_id].edges[e] {
                super::packed::Edge::Leaf(it) => cands.push(it.clone()),
                super::packed::Edge::Combine { left, right } => {
                    let (l, r) = (*left, *right);
                    let lk = self.kbest(forest, l, k, memo);
                    let rk = self.kbest(forest, r, k, memo);
                    let layer = &self.layer;
                    self.cube(&lk, &rk, k, &mut cands, |l, r| apply(l, r, layer));
                }
                super::packed::Edge::Binary { left, right, rule } => {
                    let (l, r, rule) = (*left, *right, *rule);
                    let lk = self.kbest(forest, l, k, memo);
                    let rk = self.kbest(forest, r, k, memo);
                    self.cube(&lk, &rk, k, &mut cands, |l, r| {
                        self.apply_bin_rule(rule, l, r)
                    });
                }
                super::packed::Edge::Unary { child, kind } => {
                    let (child, kind) = (*child, *kind);
                    let ck = self.kbest(forest, child, k, memo);
                    for it in &ck {
                        self.materialize_unary(it, kind, span, &mut cands);
                    }
                }
            }
        }
        cands.sort_by_key(|it| it.cost());
        cands.truncate(k);
        memo[node_id] = Some(cands.clone());
        cands
    }

    /// Cube pruning (Huang & Chiang 2005) over a binary edge: enumerate `combine(lk[li], rk[ri])`
    /// best-first by combined child cost, pushing the two grid neighbours after each pop, until `k`
    /// results or the `max_pops` circuit-breaker trips (a dense pocket of non-combining pairs — the
    /// child lists are already combinability-homogeneous under index-independence, so this rarely
    /// fires). `combine` is the edge's binary rule (`apply` for `Combine`, `relativize` for
    /// `Relativize`). Appends materialised items to `out`.
    fn cube<F: Fn(&Item, &Item) -> Option<Item>>(
        &self,
        lk: &[Item],
        rk: &[Item],
        k: usize,
        out: &mut Vec<Item>,
        combine: F,
    ) {
        use super::packed::CubeCandidate;
        use std::collections::{BTreeSet, BinaryHeap};
        if lk.is_empty() || rk.is_empty() {
            return;
        }
        let mut heap: BinaryHeap<CubeCandidate> = BinaryHeap::new();
        let mut seen: BTreeSet<(usize, usize)> = BTreeSet::new();
        heap.push(CubeCandidate {
            cost: lk[0].cost().saturating_add(rk[0].cost()),
            li: 0,
            ri: 0,
        });
        seen.insert((0, 0));
        let (mut kept, mut pops) = (0usize, 0usize);
        let max_pops = k.saturating_mul(10).max(64);
        while let Some(cc) = heap.pop() {
            pops += 1;
            if pops > max_pops {
                // Circuit-breaker (a dense pocket of non-combining pairs). Never silent — log the
                // shortfall so a partial cube is visible (D63 §11 3d.3).
                eprintln!(
                    "dcg::parse (packed): cube max_pops={max_pops} hit ({kept} kept of a \
                     {}×{} grid) — extraction may be partial",
                    lk.len(),
                    rk.len(),
                );
                break;
            }
            if let Some(item) = combine(&lk[cc.li], &rk[cc.ri]) {
                out.push(item);
                kept += 1;
                if kept >= k {
                    break;
                }
            }
            if cc.li + 1 < lk.len() && seen.insert((cc.li + 1, cc.ri)) {
                heap.push(CubeCandidate {
                    cost: lk[cc.li + 1].cost().saturating_add(rk[cc.ri].cost()),
                    li: cc.li + 1,
                    ri: cc.ri,
                });
            }
            if cc.ri + 1 < rk.len() && seen.insert((cc.li, cc.ri + 1)) {
                heap.push(CubeCandidate {
                    cost: lk[cc.li].cost().saturating_add(rk[cc.ri + 1].cost()),
                    li: cc.li,
                    ri: cc.ri + 1,
                });
            }
        }
    }

    /// Materialise a token-keyed [`super::packed::BinRule`] for one (left, right) item-pair — the
    /// combiner the `cube` calls for a `Binary` edge. Each mirrors the corresponding unpacked CKY
    /// rule exactly; the DECISION (whether it returns `Some`) is category-based (so it is consistent
    /// across a packed node's items), and the sem is built here per pair.
    fn apply_bin_rule(&self, rule: super::packed::BinRule, l: &Item, r: &Item) -> Option<Item> {
        use super::packed::BinRule;
        let cost = l.cost().saturating_add(r.cost());
        match rule {
            BinRule::Relativize => relativize(l.cat(), r.cat(), r.sem())
                .map(|(cat, sem)| Item::with_cost(cat, sem, cost)),
            BinRule::Coordinate(op) => {
                // Left-branching NF: the right conjunct may not itself be a coordination.
                if is_coordination(r.sem()) {
                    return None;
                }
                if cats_coordinate(l.cat(), r.cat(), &self.layer) {
                    coordinate_sem(op, l.cat(), l.sem(), r.sem(), &self.layer)
                        .map(|sem| Item::with_cost(l.cat().clone(), sem, cost))
                } else {
                    coordinate_np(op, l.cat(), l.sem(), r.cat(), r.sem(), &self.layer)
                        .map(|(cat, sem)| Item::with_cost(cat, sem, cost))
                }
            }
            BinRule::ButNot => {
                if cats_coordinate(l.cat(), r.cat(), &self.layer) {
                    if is_coordination(r.sem()) {
                        return None;
                    }
                    coordinate_but_not_sem(l.cat(), l.sem(), r.sem(), &self.layer)
                        .map(|sem| Item::with_cost(l.cat().clone(), sem, cost))
                } else {
                    coordinate_but_not(l.cat(), l.sem(), r.cat(), r.sem(), &self.layer)
                        .map(|(cat, sem)| Item::with_cost(cat, sem, cost))
                }
            }
            BinRule::Reciprocal => reciprocate(l.cat(), l.sem(), r.cat(), r.sem(), &self.layer)
                .map(|(cat, sem)| Item::with_cost(cat, sem, cost)),
            BinRule::AppositiveSubj => {
                relativize_appos(l.cat(), l.sem(), r.cat(), r.sem(), &self.layer)
                    .map(|(cat, sem)| Item::with_cost(cat, sem, cost))
            }
            BinRule::AppositiveObj => self.appositive_obj(l, r),
        }
    }

    /// Collect the [`Edge::Binary`] derivations for `rule` over a left span `ls = (i, k)` and a right
    /// span `rs = (k', j)` (both `(start, end)` inclusive cell coordinates), the token-keyed reserved
    /// word(s) between/after them having no node. For each `(left, right)` node-pair whose
    /// REPRESENTATIVES combine under [`Self::apply_bin_rule`], appends `(result-Sig, result-item,
    /// left, right, rule)` to `out` — the caller inserts them as [`Edge::Binary`] edges once the
    /// forest borrow is released. Sound under index-independence: the decision is representative-based.
    fn binary_edges(
        &self,
        forest: &super::packed::Forest,
        ls: (usize, usize),
        rs: (usize, usize),
        rule: super::packed::BinRule,
        out: &mut Vec<(
            super::packed::Sig,
            Item,
            super::packed::NodeId,
            super::packed::NodeId,
            super::packed::BinRule,
        )>,
    ) {
        let lefts: Vec<super::packed::NodeId> =
            forest.cells[ls.0][ls.1].values().copied().collect();
        let rights: Vec<super::packed::NodeId> =
            forest.cells[rs.0][rs.1].values().copied().collect();
        for lid in lefts {
            for &rid in &rights {
                if let Some(item) =
                    self.apply_bin_rule(rule, &forest.nodes[lid].rep, &forest.nodes[rid].rep)
                {
                    out.push((super::packed::node_sig(&item), item, lid, rid, rule));
                }
            }
        }
    }

    /// Materialise a `Unary` edge for one child item — the composed-cell shift for [`UnaryKind`],
    /// with span-pure hole re-freshening (`$quant$i_j` / `$anaphor$i_j`). Mirrors the unpacked path's
    /// per-item shifts ([`Self::seed_leaves`] / the CKY loop). Appends to `out`.
    fn materialize_unary(
        &self,
        it: &Item,
        kind: super::packed::UnaryKind,
        span: (usize, usize),
        out: &mut Vec<Item>,
    ) {
        use super::packed::UnaryKind;
        let (i, j) = span;
        match kind {
            UnaryKind::BareNp => {
                let mut v = self.bare_plural_nps(it);
                v.extend(self.bare_mass_nps(it));
                for mut np in v {
                    np.set_sem(freshen_quant(np.sem(), &quant_hole_base(i, j)));
                    out.push(np);
                }
            }
            UnaryKind::Raise => out.extend(raise_nps(std::slice::from_ref(it), &self.layer)),
            UnaryKind::FrontParticipial => {
                if let Some((cat, sem)) = front_participial(it.cat(), it.sem(), &self.layer) {
                    let sem = freshen_anaphor(&sem, &hole_base(i, j));
                    out.push(Item::with_cost(cat, sem, it.cost()));
                }
            }
            // Comma absorption carries the sentence-premodifier through unchanged (it now spans the
            // trailing comma). The child is already `is_sentence_premod` (checked at forest build), so
            // no re-check is needed here; the span widens but the cat/sem/cost are identical.
            UnaryKind::AbsorbComma => out.push(it.clone()),
        }
    }

    /// Build the **packed shared forest** over a sentence (D63 blueprint §11 3c.3/3c.4). Seeds the
    /// leaf cells (shared [`Self::seed_leaves`], `beam = None` — packing bounds via k-best), groups
    /// each cell's items into [`super::packed::PNode`]s by [`super::packed::node_sig`], then runs a
    /// node-level CKY loop: for each adjacent node-pair, `apply` on their REPRESENTATIVE items decides
    /// combinability + the result signature ONCE (the O(1)-per-node-pair win — sound because the
    /// packing router gated on the grammar being index-independent), recorded as an
    /// [`super::packed::Edge::Combine`] hyperedge. The differing item-pairs are materialised lazily by
    /// the cube-pruning extractor (3d).
    ///
    /// After each cell's binary combinations come the **token-keyed sem-reading binary rules** (§11
    /// 3g.3) — relatives, coordination, `but not`, the reciprocal, appositives — as
    /// [`super::packed::Edge::Binary`] edges (materialised per item-pair at extraction via
    /// [`Self::apply_bin_rule`]), then the composed-cell UNARY shifts (3c.4b) as
    /// [`super::packed::Edge::Unary`] edges, in the unpacked CKY's order: bare-plural/mass NP shift,
    /// type-raising (which sees the shifted NPs), the fronted participial, and the fronted-modifier
    /// comma absorption. The packed CKY now mirrors every construct the unpacked CKY has, so the router
    /// ([`Self::parse_needs_unpacked`]) only diverts pied-piping (`[prep] which`) and selectional
    /// lexicons — everything else is packed and gated on the differential oracle (3f).
    fn build_forest(
        &self,
        tokens: &[String],
        lemmatizer: &dyn Lemmatizer,
        scope: Option<&[Iri]>,
        cap: Option<usize>,
        ranks: Option<&BTreeMap<String, u32>>,
    ) -> super::packed::Forest {
        use super::packed::{node_sig, BinRule, Edge, Forest, NodeId, Sig, UnaryKind};
        let n = tokens.len();
        let (leaves, _drops) = self.seed_leaves(tokens, lemmatizer, scope, cap, ranks, None);
        let mut forest = Forest::new(n);
        // Group leaf items into nodes (one `Leaf` edge each; same-`Sig` items share a node).
        for (i, row) in leaves.iter().enumerate() {
            for (j, cell) in row.iter().enumerate().skip(i) {
                for it in cell {
                    let id = forest.get_or_create(i, j, node_sig(it), it);
                    forest.push_edge(id, Edge::Leaf(it.clone()));
                }
            }
        }
        // Node-level CKY: decide each node-pair ONCE via `apply` on representatives.
        for len in 2..=n {
            for i in 0..=(n - len) {
                let j = i + len - 1;
                // Collect combinations first (immutable borrow of `forest`), then insert.
                let mut edges: Vec<(Sig, Item, NodeId, NodeId)> = Vec::new();
                for k in i..j {
                    let lefts: Vec<NodeId> = forest.cells[i][k].values().copied().collect();
                    let rights: Vec<NodeId> = forest.cells[k + 1][j].values().copied().collect();
                    for &l in &lefts {
                        for &r in &rights {
                            let lrep = forest.nodes[l].rep.clone();
                            let rrep = forest.nodes[r].rep.clone();
                            if let Some(result) = apply(&lrep, &rrep, &self.layer) {
                                edges.push((node_sig(&result), result, l, r));
                            }
                        }
                    }
                }
                for (sig, result, l, r) in edges {
                    let id = forest.get_or_create(i, j, sig, &result);
                    forest.push_edge(id, Edge::Combine { left: l, right: r });
                }

                // Token-keyed sem-reading binary rules (§11 3g.3): relative clauses, coordination,
                // `but not`, appositives, and the reciprocal — each combines two sub-cell node-spans
                // via a CAT-based decision (mirroring the unpacked CKY), recorded as `Binary` edges
                // and materialised per item-pair at extraction ([`Self::apply_bin_rule`]). Run before
                // the unary shifts so a resulting refined noun / group can shift or feed larger cells.
                let mut bin: Vec<(Sig, Item, NodeId, NodeId, BinRule)> = Vec::new();
                #[allow(clippy::needless_range_loop)] // `c` indexes tokens AND the sub-cells
                for c in (i + 1)..j {
                    // Relative: [noun] that/which [body].
                    if reserved::is_relativizer(tokens[c].as_str()) {
                        self.binary_edges(
                            &forest,
                            (i, c - 1),
                            (c + 1, j),
                            BinRule::Relativize,
                            &mut bin,
                        );
                    }
                    // Coordination: [X] and/or/`,` [Y].
                    if let Some(op) = coord_connective(tokens[c].as_str()) {
                        self.binary_edges(
                            &forest,
                            (i, c - 1),
                            (c + 1, j),
                            BinRule::Coordinate(op),
                            &mut bin,
                        );
                    }
                    // Contrastive: [O₁] but not [O₂].
                    if tokens[c] == reserved::BUT
                        && tokens.get(c + 1).map(String::as_str) == Some(reserved::NOT)
                        && c + 2 <= j
                    {
                        self.binary_edges(
                            &forest,
                            (i, c - 1),
                            (c + 2, j),
                            BinRule::ButNot,
                            &mut bin,
                        );
                    }
                }
                // Appositive: [NP] , that/which [body] [,] — a comma BEFORE the relativizer.
                #[allow(clippy::needless_range_loop)]
                for c in (i + 2)..=j {
                    if reserved::is_relativizer(tokens[c].as_str())
                        && tokens[c - 1] == reserved::COMMA
                    {
                        let body_end = if tokens[j] == reserved::COMMA {
                            j - 1
                        } else {
                            j
                        };
                        if c < body_end {
                            self.binary_edges(
                                &forest,
                                (i, c - 2),
                                (c + 1, body_end),
                                BinRule::AppositiveSubj,
                                &mut bin,
                            );
                            self.binary_edges(
                                &forest,
                                (i, c - 2),
                                (c + 1, body_end),
                                BinRule::AppositiveObj,
                                &mut bin,
                            );
                        }
                    }
                }
                // Reciprocal: [group] <TV> each other → S (the trailing "each other").
                if j >= 3 && tokens[j - 1] == reserved::EACH && tokens[j] == reserved::OTHER {
                    for s in (i + 1)..=(j - 2) {
                        self.binary_edges(
                            &forest,
                            (i, s - 1),
                            (s, j - 2),
                            BinRule::Reciprocal,
                            &mut bin,
                        );
                    }
                }
                for (sig, item, left, right, rule) in bin {
                    let id = forest.get_or_create(i, j, sig, &item);
                    forest.push_edge(id, Edge::Binary { left, right, rule });
                }

                // Composed-cell UNARY shifts (§11 3c.4b), applied per node's representative and
                // recorded as `Unary` edges (3d re-applies them per item at extraction). Order matches
                // the unpacked CKY: (1) bare-plural/mass NP shift, (2) type-raise over the updated
                // cell (so it sees the shifted NPs), (3) fronted participial. Freshening only touches
                // the sem, never `cat_shape`, so it does not affect the signature — but it is applied
                // here so the representative sems stay consistent with the unpacked path.
                let mut unary: Vec<(Sig, Item, NodeId, UnaryKind)> = Vec::new();
                for id in forest.cells[i][j].values().copied().collect::<Vec<_>>() {
                    let rep = forest.nodes[id].rep.clone();
                    let mut shifted = self.bare_plural_nps(&rep);
                    shifted.extend(self.bare_mass_nps(&rep));
                    for mut np in shifted {
                        np.set_sem(freshen_quant(np.sem(), &quant_hole_base(i, j)));
                        unary.push((node_sig(&np), np, id, UnaryKind::BareNp));
                    }
                }
                for (sig, item, child, kind) in unary.drain(..) {
                    let nid = forest.get_or_create(i, j, sig, &item);
                    forest.push_edge(nid, Edge::Unary { child, kind });
                }
                for id in forest.cells[i][j].values().copied().collect::<Vec<_>>() {
                    let rep = forest.nodes[id].rep.clone();
                    for raised in raise_nps(std::slice::from_ref(&rep), &self.layer) {
                        unary.push((node_sig(&raised), raised, id, UnaryKind::Raise));
                    }
                }
                for (sig, item, child, kind) in unary.drain(..) {
                    let nid = forest.get_or_create(i, j, sig, &item);
                    forest.push_edge(nid, Edge::Unary { child, kind });
                }
                for id in forest.cells[i][j].values().copied().collect::<Vec<_>>() {
                    let rep = forest.nodes[id].rep.clone();
                    if let Some((cat, sem)) = front_participial(rep.cat(), rep.sem(), &self.layer) {
                        let sem = freshen_anaphor(&sem, &hole_base(i, j));
                        let item = Item::with_cost(cat, sem, rep.cost());
                        unary.push((node_sig(&item), item, id, UnaryKind::FrontParticipial));
                    }
                }
                for (sig, item, child, kind) in unary.drain(..) {
                    let nid = forest.get_or_create(i, j, sig, &item);
                    forest.push_edge(nid, Edge::Unary { child, kind });
                }
                // Fronted-modifier comma absorption (§11 3g.3): a sentence-initial `S/S` pre-modifier
                // at `[0, j-1]` carries over a trailing comma at `j` to span `[0, j]`, so it can then
                // forward-apply across the node-less comma to the matrix clause. Keyed on `i == 0` (so
                // it never competes with list-coordination commas); the child keeps its `Sig`, so the
                // absorbed node packs identically. Mirrors the unpacked CKY's comma-absorption.
                if i == 0 && j >= 1 && tokens[j] == reserved::COMMA {
                    for cid in forest.cells[0][j - 1].values().copied().collect::<Vec<_>>() {
                        let rep = forest.nodes[cid].rep.clone();
                        if is_sentence_premod(rep.cat()) {
                            unary.push((node_sig(&rep), rep, cid, UnaryKind::AbsorbComma));
                        }
                    }
                    for (sig, item, child, kind) in unary.drain(..) {
                        let nid = forest.get_or_create(i, j, sig, &item);
                        forest.push_edge(nid, Edge::Unary { child, kind });
                    }
                }
            }
        }
        forest
    }

    fn parse_unpacked(
        &self,
        text: &str,
        lemmatizer: &dyn Lemmatizer,
        scope: Option<&[Iri]>,
    ) -> (Vec<Item>, Vec<OpenParse>) {
        // Contextual sense ranking (GH #97): computed ONCE up front (one ranker call per parse,
        // not per widen iteration), then threaded into every capped attempt below.
        let ranks = self.contextual_sense_ranks(text, lemmatizer, scope);
        let mut cap = self.sense_cap;
        let mut beam = self.cell_beam;
        loop {
            let (closed, open) =
                self.parse_at_cap(text, lemmatizer, scope, cap, ranks.as_ref(), beam);
            if !closed.is_empty() || !open.is_empty() {
                return (closed, open);
            }
            // Widen only if a pruning artifact could be the cause (no OOV token).
            if !self.all_prose_tokens_known(text, lemmatizer) {
                return (closed, open);
            }
            // Escalate **beam-first**: grow the cell beam (keeping the sense cap LOW) until it maxes,
            // then grow the sense cap. Raising the cap admits more senses per lemma, which re-crowds
            // the chart and can beam out the very constituent a wider beam was meant to keep — so a
            // beam-limited sentence is best recovered at a low cap + wide beam, not both wide at once.
            let grew_beam = match beam {
                Some(b) if b < CELL_BEAM_WIDEN_MAX => {
                    beam = Some((b * 2).min(CELL_BEAM_WIDEN_MAX));
                    true
                }
                _ => false,
            };
            let widened = grew_beam
                || match cap {
                    Some(c) if c < SENSE_CAP_WIDEN_MAX => {
                        cap = Some((c * 2).min(SENSE_CAP_WIDEN_MAX));
                        true
                    }
                    _ => false,
                };
            if !widened {
                return (closed, open);
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
                        let gloss = self.sem_gloss(e.item.sem()).unwrap_or_default();
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

    /// Seed the LEAF cells of a CKY chart — the shared front-end of both the unpacked path
    /// ([`Self::parse_at_cap`]) and the packed forest (D63 blueprint §11 3c.3). Multi-span MWE
    /// [`Self::lookup_span`] + hole-freshening (`$anaphor$`/`$quant$`) + `-ly`/degree adverbs +
    /// fronted participials + leaf forward type-raising, optionally per-cell beamed. Returns the
    /// `n × n` chart (only leaf spans `[i,j]` populated) and the accumulated beam-drop count.
    /// Behaviour-identical to the inline seeding it replaces — the packed path calls it with
    /// `beam = None` (packing bounds via k-best, not a beam).
    fn seed_leaves(
        &self,
        tokens: &[String],
        lemmatizer: &dyn Lemmatizer,
        scope: Option<&[Iri]>,
        cap: Option<usize>,
        ranks: Option<&BTreeMap<String, u32>>,
        beam: Option<usize>,
    ) -> (Vec<Vec<Vec<Item>>>, usize) {
        let debug = std::env::var("EIGENIUS_PARSE_DEBUG").is_ok();
        let n = tokens.len();
        // chart[i][j] = every item spanning tokens i..=j.
        let mut chart: Vec<Vec<Vec<Item>>> = vec![vec![Vec::new(); n]; n];

        // 1. Seed lexical spans (multi-span MWE seeding). A multiword form at [i,j] is seeded
        //    ALONGSIDE the items of its parts, so both readings survive into the chart.
        let span_limit = self.span_limit(n);
        for i in 0..n {
            let last = (i + span_limit).min(n);
            for j in i..last {
                let surface = tokens[i..=j].join(" ");
                for mut it in self.lookup_span(&surface, lemmatizer, scope, cap, ranks) {
                    // Referent-hole freshening (D64): the `lexicon:anaphor` placeholder becomes a
                    // fresh per-occurrence free var (typed `Entity` at felicity).
                    it.set_sem(freshen_anaphor(it.sem(), &hole_base(i, j)));
                    // Quantification-hole freshening (D62 bare-plural shift): the `$quanthole$`
                    // sentinel becomes a fresh per-span free var (typed `Quantification`).
                    it.set_sem(freshen_quant(it.sem(), &quant_hole_base(i, j)));
                    chart[i][j].push(it);
                }
                // Derived `-ly` adverbs (D62 Phase 3): transparent modifier items for a single `-ly`
                // token whose adjective base is known. Single-token spans; identity sem, no holes.
                if i == j {
                    for it in self.adverb_items(&surface) {
                        chart[i][j].push(it);
                    }
                    // Fronted participial from a single-token (intransitive) `ger` VP ("arising, …").
                    let fronted: Vec<Item> = chart[i][j]
                        .iter()
                        .filter_map(|it| {
                            front_participial(it.cat(), it.sem(), &self.layer).map(|(c, s)| {
                                Item::with_cost(c, freshen_anaphor(&s, &hole_base(i, j)), it.cost())
                            })
                        })
                        .collect();
                    chart[i][j].extend(fronted);
                }
                // Degree-modified adverb (`more commonly`): a 2-token transparent sentence adverb.
                if j == i + 1 {
                    for it in self.degree_adverb_items(&tokens[i], &tokens[j]) {
                        chart[i][j].push(it);
                    }
                }
            }
        }

        // Forward bounded type-raising `T` (D63 §8.9 Slice 6-T) at the LEAF cells: a name `NP` lifts
        // to `S/(S\NP)` so it can forward-compose into a relative clause's object-extraction body.
        // ENF (`TypeRaised` provenance) keeps these inert outside extraction. Composed cells are
        // raised in the CKY loop.
        let mut beam_drops = 0usize;
        for (i, row) in chart.iter_mut().enumerate() {
            let raised = raise_nps(&row[i], &self.layer);
            row[i].extend(raised);
            // A leaf cell is non-top iff the sentence has >1 token; the beam caps it across all
            // candidate lemmas/POS of the token (`sense_cap` already bounds it per-lemma).
            if n > 1 {
                if let Some(b) = beam {
                    beam_drops += beam_cell(&mut row[i], b);
                }
            }
            if debug {
                eprintln!(
                    "  [parse-debug leaf] cell[{i}..{i}] tok={:?} | {}",
                    tokens[i],
                    cell_histogram(&row[i])
                );
            }
        }
        (chart, beam_drops)
    }

    fn parse_at_cap(
        &self,
        text: &str,
        lemmatizer: &dyn Lemmatizer,
        scope: Option<&[Iri]>,
        cap: Option<usize>,
        ranks: Option<&BTreeMap<String, u32>>,
        beam: Option<usize>,
    ) -> (Vec<Item>, Vec<OpenParse>) {
        let tokens = tokenize(text);
        let n = tokens.len();
        if n == 0 {
            return (Vec::new(), Vec::new());
        }
        // Parse-failure instrumentation (set `EIGENIUS_PARSE_DEBUG=1`): per-cell stats, flushed, so
        // the last line before an OOM/SIGKILL localizes the blow-up cell + cap level.
        let debug = std::env::var("EIGENIUS_PARSE_DEBUG").is_ok();

        // Coordinator positions (D63 §8.4 Phase 3): `and`/`or` are parser-level
        // reserved words (NOT lexical entries — coordination is polymorphic over
        // `Cat`, which `⟦·⟧` can't denote), handled by the coordination rule below.
        let coord_op: Vec<Option<&str>> = tokens
            .iter()
            .map(|t| coord_connective(t.as_str()))
            .collect();

        // 1. Seed the leaf cells (shared with the packed path, §11 3c.3).
        let (mut chart, mut beam_drops) =
            self.seed_leaves(&tokens, lemmatizer, scope, cap, ranks, beam);

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
                            // Combinatory-core spike: the extra CCG combinators (crossed + backward
                            // composition), applied alongside the hand-built rules when enabled.
                            if self.combinatory_core {
                                produced.extend(apply_core(l, r, &self.layer));
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
                            if is_coordination(r.sem()) {
                                continue;
                            }
                            if cats_coordinate(l.cat(), r.cat(), &self.layer) {
                                if let Some(sem) =
                                    coordinate_sem(op, l.cat(), l.sem(), r.sem(), &self.layer)
                                {
                                    produced.push(Item::with_cost(
                                        l.cat().clone(),
                                        sem,
                                        l.cost().saturating_add(r.cost()),
                                    ));
                                }
                            } else if let Some((cat, sem)) =
                                coordinate_np(op, l.cat(), l.sem(), r.cat(), r.sem(), &self.layer)
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
                                    l.cost().saturating_add(r.cost()),
                                ));
                            }
                        }
                    }
                }
                // Contrastive `but not` (D62 §2 #8): `[O₁] but not [O₂]` builds a `conn_but_not`
                // group (binary), which the verb then distributes over as `V(O₁) ∧ ¬V(O₂)` (the
                // shared predicate applies affirmatively to O₁ and negatively to the elided O₂). A
                // two-token reserved coordinator (`but` + `not`), keyed like `and`/`or` but matched as
                // a sequence; `but` alone stays the sentential `but_subord`, so no conflict.
                for c in (i + 1)..j {
                    if tokens[c] != reserved::BUT
                        || tokens.get(c + 1).map(String::as_str) != Some(reserved::NOT)
                    {
                        continue;
                    }
                    if c + 2 > j {
                        continue;
                    }
                    let lefts = &chart[i][c - 1];
                    let rights = &chart[c + 2][j];
                    for l in lefts {
                        for r in rights {
                            // Prop-ending constituents (determined-NP / GQ objects, VPs, clauses):
                            // the general contrastive conjunction `a ∧ ¬b` — covers the WRN case
                            // `required the helicase activity but not its exonuclease activity`.
                            if cats_coordinate(l.cat(), r.cat(), &self.layer) {
                                if !is_coordination(r.sem()) {
                                    if let Some(sem) = coordinate_but_not_sem(
                                        l.cat(),
                                        l.sem(),
                                        r.sem(),
                                        &self.layer,
                                    ) {
                                        produced.push(Item::with_cost(
                                            l.cat().clone(),
                                            sem,
                                            l.cost().saturating_add(r.cost()),
                                        ));
                                    }
                                }
                            } else if let Some((cat, sem)) =
                                coordinate_but_not(l.cat(), l.sem(), r.cat(), r.sem(), &self.layer)
                            {
                                // Bare-NAME objects (not Prop-ending): the `conn_but_not` group the
                                // verb then distributes over as `V(O₁) ∧ ¬V(O₂)`.
                                produced.push(Item::with_cost(
                                    cat,
                                    sem,
                                    l.cost().saturating_add(r.cost()),
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
                if j >= 3 && tokens[j - 1] == reserved::EACH && tokens[j] == reserved::OTHER {
                    // Verb spans [s, j-2]; subject group spans [i, s-1].
                    for s in (i + 1)..=(j - 2) {
                        let subjects = &chart[i][s - 1];
                        let verbs = &chart[s][j - 2];
                        for subj in subjects {
                            for tv in verbs {
                                if let Some((cat, sem)) = reciprocate(
                                    subj.cat(),
                                    subj.sem(),
                                    tv.cat(),
                                    tv.sem(),
                                    &self.layer,
                                ) {
                                    produced.push(Item::with_cost(
                                        cat,
                                        sem,
                                        subj.cost().saturating_add(tv.cost()),
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
                    if !reserved::is_relativizer(tokens[c].as_str()) {
                        continue;
                    }
                    let nouns = &chart[i][c - 1];
                    let bodies = &chart[c + 1][j];
                    for noun in nouns {
                        for body in bodies {
                            if let Some((cat, sem)) = relativize(noun.cat(), body.cat(), body.sem())
                            {
                                produced.push(Item::with_cost(
                                    cat,
                                    sem,
                                    noun.cost().saturating_add(body.cost()),
                                ));
                            }
                        }
                    }
                }
                // Non-restrictive (appositive) relative (D62 §2 #2A): `[NP] , which/that [body] [,]` →
                // the antecedent NP type-raised to a CONJOINING quantifier (`λP. And(P(r), body(r))`) —
                // a SEPARATE assertion on an already-referring NP, NOT a Σ-restriction (core-en
                // `RelPro-Appos`: `s\s`+`Trib`). Signalled by the comma BEFORE the relativizer (so it
                // never competes with the restrictive rule, whose noun must be relativizer-adjacent). A
                // trailing comma after the clause is absorbed into this span so the appositive NP is
                // adjacent to the matrix VP.
                for c in (i + 2)..=j {
                    if !reserved::is_relativizer(tokens[c].as_str()) {
                        continue;
                    }
                    if tokens[c - 1] != reserved::COMMA {
                        continue;
                    }
                    let ante_end = c - 2; // antecedent NP is [i, c-2] (before the comma)
                    let body_end = if tokens[j] == reserved::COMMA {
                        j - 1
                    } else {
                        j
                    };
                    if c + 1 > body_end {
                        continue;
                    }
                    let antes = &chart[i][ante_end];
                    let bodies = &chart[c + 1][body_end];
                    for ante in antes {
                        for body in bodies {
                            // Subject-position (type-raised `S/(S\NP)`); prep-object rides this form
                            // through the GQ-as-preposition-object rule.
                            if let Some((cat, sem)) = relativize_appos(
                                ante.cat(),
                                ante.sem(),
                                body.cat(),
                                body.sem(),
                                &self.layer,
                            ) {
                                produced.push(Item::with_cost(
                                    cat,
                                    sem,
                                    ante.cost().saturating_add(body.cost()),
                                ));
                            }
                            // Verb-object position (in-situ object raise, mirroring `a_obj`).
                            if let Some(it) = self.appositive_obj(ante, body) {
                                produced.push(it);
                            }
                        }
                    }
                }
                // Pied-piping restrictive relative (D62 §2 #2B): `[noun] [prep] which [subj] [VP]` →
                // refine the noun with the clause + the FRONTED preposition relating the antecedent to
                // the clause subject (`Σg:C. And(VP(subj), prep(subj,g))`). Reuses the VP-adjunct prep
                // sem (no PP-gap extraction). The clause after `prep which` is decomposed into its
                // subject NP + `S\NP` VP at every split, so it handles the ordinary subject-predicate
                // clause; `which` here is the fronted prep's object, distinct from the bare relativizer.
                for p in (i + 1)..j {
                    if tokens.get(p + 1).map(|t| t.as_str()) != Some(reserved::WHICH) {
                        continue;
                    }
                    if p < i + 1 || p + 2 > j {
                        continue;
                    }
                    let preps: Vec<Exp> = self
                        .entries_for(tokens[p].as_str())
                        .iter()
                        .filter(|e| is_vp_adjunct_prep(e.item.cat()))
                        .map(|e| e.item.sem().clone())
                        .collect();
                    if preps.is_empty() {
                        continue;
                    }
                    for k in (p + 2)..j {
                        for noun in &chart[i][p - 1] {
                            if is_ctor(noun.cat(), "cat_n").is_none() {
                                continue;
                            }
                            for subj in &chart[p + 2][k] {
                                if is_ctor(subj.cat(), "cat_np").is_none() {
                                    continue;
                                }
                                for vp in &chart[k + 1][j] {
                                    // VP must be `S\NP` (a clause missing its subject).
                                    if !matches!(is_ctor(vp.cat(), "bwd"),
                                        Some([s, _]) if is_ctor(s, "cat_s").is_some())
                                    {
                                        continue;
                                    }
                                    for prep_sem in &preps {
                                        if let Some((cat, sem)) =
                                            pied_pipe(noun.cat(), prep_sem, subj.sem(), vp.sem())
                                        {
                                            produced.push(Item::with_cost(
                                                cat,
                                                sem,
                                                noun.cost()
                                                    .saturating_add(subj.cost())
                                                    .saturating_add(vp.cost()),
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                let produced_n = produced.len();
                chart[i][j].extend(produced);
                // Bare-NP shift for COMPOSED common nouns (D62 — N-N compounds like "MSI cancer
                // models", adjective-refined nouns like "novel therapies" / "synthetic lethality"):
                // the leaf shift in `lookup_span` only covers lexical nouns, so a *composed*
                // `cat_n(_, pl)` (plural) or `cat_n(_, mass)` (uncountable) cell needs the shift here
                // too — else such a compound/adjective-modified noun can never be a bare argument NP.
                // BOTH the plural and mass shifts apply, symmetric with the leaf path (which runs both);
                // the mass arm was missing here, so `synthetic lethality` / `deficient repair` (adj +
                // mass/plural head) gapped while the bare leaf `lethality` shifted. The quant sentinel
                // is freshened with THIS span's `quant_hole_base` (distinct hole per span).
                let bare: Vec<Item> = chart[i][j]
                    .iter()
                    .flat_map(|it| {
                        let mut v = self.bare_plural_nps(it);
                        v.extend(self.bare_mass_nps(it));
                        v
                    })
                    .map(|mut np| {
                        np.set_sem(freshen_quant(np.sem(), &quant_hole_base(i, j)));
                        np
                    })
                    .collect();
                chart[i][j].extend(bare);
                // Type-raise `T` (D63 §8.9 Slice 6-T) the cell's name NPs (after its
                // composition + relativizer items are in place), so a non-leaf / composed
                // NP can also seed an extraction body. Raised once per cell.
                let raised = raise_nps(&chart[i][j], &self.layer);
                chart[i][j].extend(raised);
                // Fronted participial adjunct (D62 §2 #5a): a subject-gapped `ger` VP in this cell
                // ("affecting BRCA1", "hypothesizing that P") also serves as a sentence pre-modifier
                // `S/S` asserting the participial proposition with a CONTROLLED-subject referent hole
                // (freshened with this span's `hole_base`, so it is the open-parse controller). The
                // comma absorption below then carries it over a trailing comma to front the matrix.
                let fronted: Vec<Item> = chart[i][j]
                    .iter()
                    .filter_map(|it| {
                        front_participial(it.cat(), it.sem(), &self.layer).map(|(cat, sem)| {
                            Item::with_cost(cat, freshen_anaphor(&sem, &hole_base(i, j)), it.cost())
                        })
                    })
                    .collect();
                chart[i][j].extend(fronted);
                // Fronted-modifier comma absorption (D62 §2 #5): a SENTENCE-INITIAL `S/S` modifier
                // (`Thus,` / `More commonly,` / later a fronted participial) absorbs a trailing comma
                // so it can then forward-apply to the matrix clause. The comma is otherwise a reserved
                // coordinator with no chart item, leaving a gap the modifier can't bridge. Restricted to
                // `i == 0` (sentence-initial) to avoid competing with list-coordination commas.
                if i == 0 && len >= 2 && tokens[j] == reserved::COMMA {
                    let absorbed: Vec<Item> = chart[i][j - 1]
                        .iter()
                        .filter(|it| is_sentence_premod(it.cat()))
                        .cloned()
                        .collect();
                    chart[i][j].extend(absorbed);
                }
                // Lever B: beam this composed cell (non-top; the top cell `len == n` is left to the
                // forest cap). Done after type-raise so the raised items compete in the beam too.
                if len < n {
                    if let Some(b) = beam {
                        beam_drops += beam_cell(&mut chart[i][j], b);
                    }
                }
                if debug {
                    eprintln!(
                        "  [parse-debug cap={cap:?}] cell[{i}..{j}] len={len} produced={produced_n} kept={} | {}",
                        chart[i][j].len(),
                        cell_histogram(&chart[i][j])
                    );
                }
                // Targeted dump (set `EIGENIUS_DUMP_CELL=i..j`): print the FULL category (indices
                // intact) + provenance of a sample of this cell's items, to see exactly which
                // sense/derivation combinations accumulate.
                if let Ok(want) = std::env::var("EIGENIUS_DUMP_CELL") {
                    if want == format!("{i}..{j}") {
                        eprintln!(
                            "  ===== DUMP cell[{i}..{j}] ({} items, sample 20) =====",
                            chart[i][j].len()
                        );
                        for it in chart[i][j].iter().take(20) {
                            eprintln!(
                                "    [{:?} cost={:?}] {}",
                                it.prov(),
                                it.cost(),
                                super::pretty_term(it.cat())
                            );
                        }
                    }
                }
            }
        }
        if beam_drops > 0 {
            eprintln!(
                "dcg::parse: cell-beam (Lever B) dropped {beam_drops} items \
                 (beam={})",
                beam.unwrap_or(0),
            );
        }

        // 3. The forest: full-span `S` items whose assembled sem — once **NbE-
        //    reduced** (the determiner lambdas β-apply away to a normal form) — the
        //    kernel confirms inhabits `Prop`. Reducing first is essential: a
        //    composed determiner sentence is a redex-heavy `App(λ…, …)` tree, and
        //    `check_infer` cannot synthesize a bare lambda's type.
        // The hole context for classification (D64 carrier, generalized to per-hole type+kind): for
        // every span `[i,j]`, both a referent hole (`Entity`/`EntityRef`, a pronoun/possessor) and a
        // quantification hole (`Π(A:Set).(A→Prop)→Prop`/`Quantification`, a bare plural's deferred
        // determiner — `docs/notes/d62-bare-plural-quantification.md`). A candidate mentions only the
        // hole vars it actually carries; `classify_felicitous` filters to those.
        let entity_ty = Exp::EigonClass(iri(ENTITY_IRI));
        let quant_ty = quant_hole_type();
        // Degenerate guard (preserved): if the hole types can't even be evaluated, fall back to the
        // closed-only path. Normally both eval fine.
        let types_ok = eval(&entity_ty, &Rho::Nil).is_ok() && eval(&quant_ty, &Rho::Nil).is_ok();
        let mut hole_specs: Vec<(String, Exp, HoleKind)> = Vec::new();
        if types_ok {
            for i in 0..n {
                for j in i..n {
                    hole_specs.push((hole_base(i, j), entity_ty.clone(), HoleKind::EntityRef));
                    hole_specs.push((
                        quant_hole_base(i, j),
                        quant_ty.clone(),
                        HoleKind::Quantification,
                    ));
                }
            }
        }

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
                is_finite_clause(it.cat()) || is_ctor(it.cat(), "cat_q").is_some()
            })
            .collect();
        let n_candidates = candidates.len();
        candidates.sort_by_key(|it| it.cost());
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
                    super::pretty_term(it.cat()),
                    super::pretty_term(it.sem())
                );
            }
            if types_ok {
                match self.classify_felicitous(it, &hole_specs) {
                    Some(FelicitousOutcome::Closed(c)) => forest.push(c),
                    Some(FelicitousOutcome::Open(o)) => open.push(o),
                    None => {}
                }
            } else if let Some(c) = self.reduced_felicitous(it) {
                // Hole types unavailable (should not happen): closed path only.
                forest.push(c);
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
        forest.sort_by_key(|it| it.cost());
        if forest.len() > DEFAULT_FOREST_CAP {
            let dropped = forest.len() - DEFAULT_FOREST_CAP;
            eprintln!(
                "dcg::parse: ranked forest capped {} → {DEFAULT_FOREST_CAP} \
                 (dropped {dropped} higher-cost / rarer-sense parses)",
                forest.len(),
            );
            forest.truncate(DEFAULT_FOREST_CAP);
        }
        open.sort_by_key(|o| o.item.cost());
        if open.len() > DEFAULT_FOREST_CAP {
            open.truncate(DEFAULT_FOREST_CAP);
        }
        (forest, open)
    }

    /// Normalize `it.sem()` (NbE β-reduction → a normal form) and keep the item —
    /// carrying the reduced sem — only if the kernel confirms it **inhabits `⟦cat⟧`**:
    /// `Prop` for a declarative `S`, `T → Prop` for a wh-question `Q(T)`. Uses
    /// check-mode (not `check_infer`) so a wh-question's answer-property *lambda* —
    /// which `check_infer` cannot synthesize — is checked against its expected Π/→.
    fn reduced_felicitous(&self, it: &Item) -> Option<Item> {
        let expected = denote_cat(it.cat()).ok()?;
        let expected_val = eval(&expected, &Rho::Nil).ok()?;
        let nf = felicity_readback(&eval(it.sem(), &Rho::Nil).ok()?)?;
        let mut ctx = CheckCtx::with_layer(Rho::Nil, Vec::new(), Arc::clone(&self.layer));
        check(&mut ctx, &nf, &expected_val).ok()?;
        Some(Item::from_parts(it.cat().clone(), nf, it.prov(), it.cost()))
    }

    /// Classify a full-span candidate as a CLOSED felicitous parse or an OPEN one carrying
    /// unresolved holes (D64), or reject it. Generalizes [`Self::reduced_felicitous`] to
    /// hole-bearing sems: each hole is a free variable, so it is bound in `rho` to a generic neutral
    /// (else Pure `eval` errors `UnboundVariable`) and in `gamma` to **its own type** so `check`
    /// types it. `hole_specs` carries every candidate hole `(base name, type, kind)`; a candidate
    /// mentions only the subset it actually carries — `EntityRef` holes (`Entity`, in argument
    /// position) and/or `Quantification` holes (`Π(A:Set).(A→Prop)→Prop`, a bare plural's deferred
    /// determiner in head position). `Neut::Gen(0, base)` reads back as `Var("{base}0")`, so the
    /// gamma key and reported hole name use that readback form. With no holes present this is exactly
    /// `reduced_felicitous` (empty `rho`/`gamma`) — the closed path is unchanged.
    fn classify_felicitous(
        &self,
        it: &Item,
        hole_specs: &[(String, Exp, HoleKind)],
    ) -> Option<FelicitousOutcome> {
        // Holes carried by this parse (tested on the raw, pre-reduction sem).
        let present: Vec<&(String, Exp, HoleKind)> = hole_specs
            .iter()
            .filter(|(base, _, _)| exp_mentions_var(it.sem(), base))
            .collect();
        let expected = denote_cat(it.cat()).ok()?;
        let expected_val = eval(&expected, &Rho::Nil).ok()?;
        // Evaluate the assembled sem with each freshened hole base bound to a generic neutral
        // (else Pure eval errors on the free var). `Neut::Gen(0, base)` reads back as
        // `Var("{base}0")`, so the holes in the normal form carry that suffixed name.
        let mut eval_rho = Rho::Nil;
        for (base, _, _) in &present {
            eval_rho =
                eval_rho.extend(Patt::Var(base.clone()), Val::Nt(Neut::Gen(0, base.clone())));
        }
        // STEP-TIMING instrumentation (set `EIGENIUS_PARSE_DEBUG=1`): each step is flushed BEFORE
        // it runs, so the last line printed before an OOM/SIGKILL names the exploding step
        // (eval / readback / check) — the felicity gate is the witnessed full-lexicon blow-up site.
        let dbg = std::env::var("EIGENIUS_PARSE_DEBUG").is_ok();
        if dbg {
            eprintln!("    [felicity] eval start");
        }
        let evaled = eval(it.sem(), &eval_rho).ok()?;
        if dbg {
            eprintln!("    [felicity] readback start");
        }
        let nf = felicity_readback(&evaled)?;
        // Check the normal form under a context binding each (readback-named) hole in BOTH
        // `rho` (a neutral value — `check` evaluates subterms, which would otherwise error on the
        // free var) and `gamma` (its **own** type — `Entity` for a referent, the GQ type for a
        // quantification hole). The carried `HoleInfo` reports each hole's type + kind.
        let mut chk_rho = Rho::Nil;
        let mut gamma: Gamma = Vec::new();
        let mut infos: Vec<HoleInfo> = Vec::new();
        for (base, ty_exp, kind) in &present {
            let name = format!("{base}0");
            chk_rho = chk_rho.extend(Patt::Var(name.clone()), Val::Nt(Neut::Gen(0, name.clone())));
            gamma.push((name.clone(), eval(ty_exp, &Rho::Nil).ok()?));
            infos.push(HoleInfo {
                var: name,
                ty: (*ty_exp).clone(),
                kind: (*kind).clone(),
            });
        }
        let mut ctx = CheckCtx::with_layer(chk_rho, gamma, Arc::clone(&self.layer));
        if dbg {
            eprintln!("    [felicity] check start");
        }
        check(&mut ctx, &nf, &expected_val).ok()?;
        let item = Item::from_parts(it.cat().clone(), nf, it.prov(), it.cost());
        if infos.is_empty() {
            Some(FelicitousOutcome::Closed(item))
        } else {
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
        let nf = readback_val(0, &eval(open.item.sem(), &rho).ok()?);
        let expected = denote_cat(open.item.cat()).ok()?;
        let expected_val = eval(&expected, &Rho::Nil).ok()?;
        // Closed re-gate: empty Γ, so any leftover hole is an unbound variable ⇒ fail closed.
        let mut ctx = CheckCtx::with_layer(Rho::Nil, Vec::new(), Arc::clone(&self.layer));
        check(&mut ctx, &nf, &expected_val).ok()?;
        Some(Item::from_parts(
            open.item.cat().clone(),
            nf,
            open.item.prov(),
            open.item.cost(),
        ))
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

/// The head constructor of a determiner cat's `cat_forall(num, λT. body)` body — `"fwd"` for a
/// type-raised **subject** determiner (`S/(S\NP)`), `"bwd"` for an in-situ **object** determiner.
/// Selects the subject vs object deferred-quantifier sem in the bare-plural shift. `None` if `cat`
/// is not a `cat_forall(_, λ. <fwd|bwd>…)`.
fn cat_forall_body_head(cat: &Exp) -> Option<&'static str> {
    if let Some([_num, Exp::Lam(_, inner)]) = is_ctor(cat, "cat_forall") {
        return match inner.as_ref() {
            Exp::InductiveCtor(_, name, _) if name == "fwd" => Some("fwd"),
            Exp::InductiveCtor(_, name, _) if name == "bwd" => Some("bwd"),
            _ => None,
        };
    }
    None
}

/// Base name of the **quantification-hole** free variable a bare-plural NP spanning tokens `[i, j]`
/// carries (`docs/notes/d62-bare-plural-quantification.md`). Distinct prefix from [`hole_base`] so
/// the felicity gate types it as a higher-order [`HoleKind::Quantification`] hole, not an `Entity`
/// referent. Position-keyed: the two bare plurals in `genes affect cells` are distinct holes.
fn quant_hole_base(i: usize, j: usize) -> String {
    format!("$quant${i}_{j}")
}

/// The type a [`HoleKind::Quantification`] hole inhabits: `Π(A:Set). (A→Prop) → Prop` — a
/// generalized quantifier over the restrictor class `A` (identical to `exists_sem`/`a`'s `sem_type`),
/// here a free higher-order hole rather than a committed quantifier. The deferred sems always apply
/// it to an **η-expanded** scope `λx. V(x)` (not a rigid predicate), so the `x:A`-against-`Entity`
/// argument subsumption happens at the λ body — exactly as the concrete `∃` does (`∃x:A. V(x)`); a
/// rigid VP passed whole would need contravariant arrow subtyping the kernel doesn't do. Built in
/// code (no chain axiom ⇒ no bootstrap change). The probe `probe_kernel_gates_…` guards it.
fn quant_hole_type() -> Exp {
    Exp::Pi(
        Patt::Var("A".into()),
        Box::new(Exp::Sort(1)), // Set
        Box::new(Exp::Arrow(
            Box::new(Exp::Arrow(
                Box::new(Exp::Var("A".into())),
                Box::new(Exp::Sort(0)), // Prop
            )),
            Box::new(Exp::Sort(0)), // Prop
        )),
    )
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
        // Inductive nodes (e.g. `logic:And(P, Q)` as an `InductiveType`) carry subterms too — a
        // fronted-participial conjunct nests the anaphor inside an `And`, so the freshener must
        // descend into them (else the hole stays an unfreshened closed constant).
        Exp::InductiveType(d, args) => Exp::InductiveType(d.clone(), args.iter().map(go).collect()),
        Exp::InductiveCtor(d, n, args) => {
            Exp::InductiveCtor(d.clone(), n.clone(), args.iter().map(go).collect())
        }
        other => other.clone(),
    }
}

/// The placeholder variable a bare-plural NP carries for its deferred quantifier before per-span
/// freshening (D62 — `docs/notes/d62-bare-plural-quantification.md`). Never a binder name, so no
/// capture concern. Replaced per occurrence by [`freshen_quant`] with a [`quant_hole_base`] name.
const QUANT_SENTINEL: &str = "$quanthole$";

/// Replace every [`QUANT_SENTINEL`] occurrence in `exp` with the free variable `fresh` — the
/// quantification-hole analogue of [`freshen_anaphor`], so distinct bare-plural occurrences are
/// distinct holes. A plain structural rename (the sentinel is never bound, so no capture).
fn freshen_quant(exp: &Exp, fresh: &str) -> Exp {
    let go = |e: &Exp| freshen_quant(e, fresh);
    match exp {
        Exp::Var(v) if v == QUANT_SENTINEL => Exp::Var(fresh.to_string()),
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
        Exp::InductiveType(d, args) => Exp::InductiveType(d.clone(), args.iter().map(go).collect()),
        Exp::InductiveCtor(d, n, args) => {
            Exp::InductiveCtor(d.clone(), n.clone(), args.iter().map(go).collect())
        }
        other => other.clone(),
    }
}

/// `Q(A, λx. body)` — the deferred-quantifier hole applied to a restrictor class and an **η-expanded**
/// scope. The η-expansion is essential: the scope is a λ binding `x:A`, so the `x:A`-against-`Entity`
/// subsumption happens at `body` (argument position), matching the concrete `∃`. Passing a rigid VP
/// whole would demand contravariant arrow subtyping the kernel lacks (witnessed: rejected).
fn quant_apply(a: Exp, x: &str, body: Exp) -> Exp {
    let scope = Exp::Lam(Patt::Var(x.into()), Box::new(body));
    Exp::App(
        Box::new(Exp::App(
            Box::new(Exp::Var(QUANT_SENTINEL.into())),
            Box::new(a),
        )),
        Box::new(scope),
    )
}

/// The **subject** deferred-quantifier determiner sem `λA. λV. Q(A, λx. V(x))` — `exists_sem` with the
/// `∃x:A.V(x)` body replaced by the deferred hole `Q` applied to the η-expanded VP. Applied to a bare
/// plural's noun class `C` it yields the subject NP sem `λV. Q(C, λx. V(x))` (core-en `det=nil`,
/// deferred). D62 `docs/notes/d62-bare-plural-quantification.md`.
fn deferred_quant_subj_sem() -> Exp {
    // λA. λV. Q(A, λx. V(x))
    let body = quant_apply(
        Exp::Var("A".into()),
        "x",
        Exp::App(
            Box::new(Exp::Var("V".into())),
            Box::new(Exp::Var("x".into())),
        ),
    );
    Exp::Lam(
        Patt::Var("A".into()),
        Box::new(Exp::Lam(Patt::Var("V".into()), Box::new(body))),
    )
}

/// The **object** deferred-quantifier determiner sem `λT. λTV. λsubj. Q(T, λx. TV(x, subj))` —
/// `obj_exists_sem` with the `∃x:T.TV(x,subj)` body replaced by the deferred hole `Q` applied to the
/// η-expanded scope. Mirrors the object determiner's shape (object-first TV `T→Entity→Prop`).
fn deferred_quant_obj_sem() -> Exp {
    // λT. λTV. λsubj. Q(T, λx. TV(x, subj))
    let tv_app = Exp::App(
        Box::new(Exp::App(
            Box::new(Exp::Var("TV".into())),
            Box::new(Exp::Var("x".into())),
        )),
        Box::new(Exp::Var("subj".into())),
    );
    let body = quant_apply(Exp::Var("T".into()), "x", tv_app);
    Exp::Lam(
        Patt::Var("T".into()),
        Box::new(Exp::Lam(
            Patt::Var("TV".into()),
            Box::new(Exp::Lam(Patt::Var("subj".into()), Box::new(body))),
        )),
    )
}

/// What a hole dispatches to once resolved (the carrier's resolver tag — D64). `EntityRef`
/// (pronoun/possessive referents → the D64 anaphora resolver) is an *internal-resolution* hole;
/// `Quantification` (a bare plural's deferred determiner → a grounding/citation obligation) is an
/// *output* obligation (D62 output contract §3). `ProofObligation` (factive presupposition) is the
/// planned third arm. The carrier now types each hole per its kind, not uniformly `Entity`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HoleKind {
    /// An unresolved entity referent (a pronoun / possessor), resolved by substituting a chain
    /// antecedent and re-gating. First-order, `Entity`-typed, in argument position.
    EntityRef,
    /// A bare plural's **deferred quantifier** (`docs/notes/d62-bare-plural-quantification.md`): the
    /// `det=nil` of core-en's `bnp` rule, rendered as a higher-order hole of type
    /// `Π(A:Set).(A→Prop)→Prop` in head position. Discharged downstream by binding a quantifier
    /// **and** citing the literature `Reference` that warrants the generalization (raising the
    /// claim's grade from Declared) — an *output* obligation, not an internal resolution.
    Quantification,
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
/// type-checked `item.sem()` with each hole bound to its type; it is NOT a closed final parse.
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
    // Discourse / TRANSITIONAL connective adverbs (core-en `adv.xsl` `Transitional-Adverb`): inert
    // for a scientific claim (transparent sem) but attaching at the clause level (`S/S`/`S\S`), so a
    // sentence-initial `Thus, …` / `Hence, …` wraps the matrix. The comma after a sentence-initial one
    // is absorbed in the CKY (fronted-modifier comma absorption).
    const LEXICALIZED_ADVERBS: &[&str] = &[
        "also",
        "however",
        "yet",
        "thus",
        "therefore",
        "hence",
        "consequently",
        "moreover",
        "furthermore",
        "additionally",
        "subsequently",
        "similarly",
        "conversely",
        "notably",
        "importantly",
        "thereby",
        "nonetheless",
        "nevertheless",
    ];
    LEXICALIZED_ADVERBS.contains(&surface)
}

/// Whether `cat` is a sentence PRE-modifier `S/S` (`fwd(cat_s, cat_s)`) — the category a fronted
/// transitional adverb / participial adjunct carries. Used by the fronted-modifier comma absorption.
fn is_sentence_premod(cat: &Exp) -> bool {
    matches!(is_ctor(cat, "fwd"),
        Some([a, b]) if is_ctor(a, "cat_s").is_some() && is_ctor(b, "cat_s").is_some())
}

/// Whether `cat` is a VP-adjunct preposition `((S\NP)\(S\NP))/NP` (`fwd(bwd(VP,VP), NP)`) — as
/// opposed to the `cat_pp / NP` noun-modifier reading. Used by pied-piping (#2B) to pick the prep
/// whose sem (`λx.λV.λs. And(V(s), prep(s,x))`) threads the fronted antecedent into the VP.
fn is_vp_adjunct_prep(cat: &Exp) -> bool {
    matches!(is_ctor(cat, "fwd"),
        Some([res, np]) if is_ctor(res, "bwd").is_some() && is_ctor(np, "cat_np").is_some())
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
    cell.sort_by_key(|it| it.cost());
    cell.truncate(beam);
    dropped
}

/// Diagnostic (PARSE_DEBUG): a compact category-SHAPE histogram of a chart cell — total
/// items, count of distinct shapes ([`super::cat_shape`], type-indices erased), and the top
/// shapes by frequency. Many items under ONE shape ⇒ lexical/sense variation (a type-narrowing
/// candidate, GH#93); many distinct shapes ⇒ structural ambiguity (type-narrowing won't help).
fn cell_histogram(cell: &[Item]) -> String {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for it in cell {
        *counts.entry(super::cat_shape(it.cat())).or_default() += 1;
    }
    let distinct = counts.len();
    let mut pairs: Vec<(String, usize)> = counts.into_iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let top: Vec<String> = pairs
        .iter()
        .take(4)
        .map(|(s, c)| format!("{s}×{c}"))
        .collect();
    format!("shapes={distinct} top: {}", top.join(", "))
}

/// Readback for the **felicity oracle** — total where [`readback_val`] is partial. The gate evaluates
/// UNTRUSTED candidate sems off the chart, and a spurious derivation can produce a stuck application
/// (e.g. a resource applied as a function — witnessed for a named-individual subject under
/// do-support/modal + a PP), on which `readback_val` panics (`apply failed`). Such a candidate is
/// simply **not felicitous** — reject it (`None`) rather than crash the parser. `catch_unwind`
/// converts the panic into a rejection; `eval` is already fallible (`.ok()?`), this restores the same
/// totality to the readback half. (A fully fallible `readback_val` is the cleaner follow-up; until
/// then the caught panic may still print to stderr.)
fn felicity_readback(val: &Val) -> Option<Exp> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| readback_val(0, val))).ok()
}

fn sense_cap_key(e: &SeedEntry, ranks: Option<&BTreeMap<String, u32>>) -> (bool, u32, u32) {
    let ctx = e
        .sense
        .as_ref()
        .and_then(|s| ranks.and_then(|m| m.get(s).copied()));
    (ctx.is_none(), ctx.unwrap_or(0), e.item.cost().sense_rank)
}

/// Forward bounded type-raise (D63 §8.9 Slice 6-T) every name `NP` in a cell's items
/// to `S/(S\NP)`, tagged `Combinator::TypeRaised` so ENF lets it only *compose*.
/// Non-`NP` items (functors, groups, kinds, determined NPs) yield nothing.
fn raise_nps(items: &[Item], layer: &Arc<Layer>) -> Vec<Item> {
    items
        .iter()
        .filter_map(|it| {
            type_raise(it.cat(), it.sem(), layer)
                .map(|(cat, sem)| Item::from_parts(cat, sem, Combinator::TypeRaised, it.cost()))
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
        reserved::AND | reserved::COMMA => Some("urn:eigenius:logic:And"),
        reserved::OR => Some("urn:eigenius:logic:Or"),
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
    if let Exp::InductiveCtor(decl, name, args) = it.cat() {
        if name == "cat_n" && args.len() == 2 {
            if let Exp::InductiveCtor(num_decl, n, _) = &args[1] {
                if n == "num_any" {
                    let num =
                        Exp::InductiveCtor(num_decl.clone(), num_name.to_string(), Vec::new());
                    return Item::from_parts(
                        Exp::InductiveCtor(decl.clone(), name.clone(), vec![args[0].clone(), num]),
                        it.sem().clone(),
                        it.prov(),
                        it.cost(),
                    );
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
