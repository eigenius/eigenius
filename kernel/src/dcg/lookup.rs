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
    adverb_modifier_cats, appose_group, cats_coordinate, complete_coord, coordinate_but_not,
    coordinate_but_not_sem, coordinate_np, coordinate_prop, denote_cat, front_participial, is_ctor,
    pied_pipe, predicative_adjective_cat, reciprocate, relativize, relativize_appos,
    sentence_modifier_cats, subst_cat, type_raise, CatSubst,
};
use super::lemmatizer::{Lemmatizer, Pos};
use super::lexicon::entry_to_item;
use super::parser::{apply, apply_core, Combinator, Item};
use super::reserved::{ReservedKind, ReservedTable};
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
    /// (the guard, [`Self::parse_needs_unpacked`]). **Default ON** (§11 3g.2 / B9): the packed CKY now
    /// mirrors every construct and is proven equivalent to the unpacked path (the differential oracle),
    /// so it is the default; `with_packing(false)` pins the unpacked baseline (the oracle, A/B probes).
    packing: bool,

    /// The **reserved-construct table** (§11 3g.3 / B10): the reserved-word FORM set as *data*, loaded
    /// index-driven from the ontology (`lexicon:ReservedConstruct`) at build. The CKY's reserved-word
    /// rules (both paths) classify tokens against this, replacing the former hard-coded string consts.
    reserved: ReservedTable,

    /// **Document-augmentation overlay** (D63 lexicon-augmentation §6a): an in-memory `form → entries`
    /// map of a document's OOV groundings, consulted by [`Self::entries_for`] ALONGSIDE the persisted
    /// value-index probe. It lets a document's grounded aliases (`LexiconAugmentation`) be seeded WITHOUT
    /// committing them to the store — they are proposals, not committed lexicon
    /// ([`Self::with_document_augmentation`]). Each entry's cat/sem was resolved over the Arc chain
    /// (storage-independent), so the overlay works over a DB-backed head where the value-index probe
    /// cannot see uncommitted entries (§7-2). Empty by default (no behaviour change).
    overlay: BTreeMap<String, FormEntries>,
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
        // Reserved-construct table (B10): loaded once, index-driven, before the source branch so both
        // the lazy and eager indexes carry it.
        let reserved = ReservedTable::load(&layer);
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
                packing: true,
                reserved,
                overlay: BTreeMap::new(),
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
            packing: true,
            reserved,
            overlay: BTreeMap::new(),
        }
    }

    /// Overlay a document's [`LexiconAugmentation`](crate::dcg::LexiconAugmentation) (D63 §6a) — its
    /// grounded alias entries become seedable via an in-memory `form → entries` map consulted alongside
    /// the persisted index, so a DB-backed parse SEES the document's OOV groundings without those
    /// (proposal-grade) entries being committed to the store. Each alias's cat/sem is resolved over a
    /// throwaway doc chain (Arc parent = this index's head) — Arc-walk resolution is storage-independent,
    /// so a committed concept and a doc-local minted class (a grounding miss, carried in
    /// `LexiconAugmentation::supporting`) both resolve. Entries whose cat/sem don't resolve are skipped
    /// (fail-closed, as at import). Builder-style; default (unset) is the persisted index alone.
    pub fn with_document_augmentation(
        mut self,
        aug: &crate::dcg::augment::LexiconAugmentation,
    ) -> Self {
        use crate::layer::{LayerBuilder, LayerStorage};
        let form_prop = iri("urn:eigenius:lexicon:form");
        // Doc chain purely for RESOLUTION: the supporting resources (miss-minted classes) sit on this
        // index's head, so an alias's `sem` resolves whether the concept is committed (head) or doc-local.
        let mut b = LayerBuilder::new("doc-overlay", Some(Arc::clone(&self.layer)));
        for r in aug.supporting.iter().cloned() {
            let _ = b.add_resource(r);
        }
        let doc = Arc::new(b.build(LayerStorage::in_memory()));
        let mut overlay: BTreeMap<String, FormEntries> = BTreeMap::new();
        for binding in &aug.added {
            let entry = &binding.proposed;
            let Some(Value::String(form)) = entry.get(&form_prop) else {
                continue;
            };
            let key = form.trim().to_lowercase();
            if key.is_empty() {
                continue;
            }
            let Ok(item) = entry_to_item(&doc, entry) else {
                continue;
            };
            overlay.entry(key).or_default().push(SeedEntry {
                item,
                in_lexicon: read_in_lexicon(entry),
                sense: read_sense(entry),
            });
        }
        self.overlay = overlay;
        self
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

    /// Toggle **packed-forest parsing** ([`Self::packing`]) — node-level packing + cube-pruning
    /// extraction, gated at parse time on the grammar being index-independent. Builder-style; **default
    /// ON** (§11 3g.2 / B9). Pass `false` to pin the unpacked baseline (the differential oracle, A/B
    /// probes) — otherwise packed is used for every index-independent, construct-free sentence.
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
        let mut out = match &self.source {
            Source::Eager { by_form, .. } => by_form.get(form_lc).cloned().unwrap_or_default(),
            Source::Lazy {
                index_iri,
                normalizer,
                cache,
            } => {
                let key = normalize_value(normalizer, form_lc);
                // Bind the cache hit to a local so the `MutexGuard` temporary drops HERE — before
                // `probe_form` and the re-`lock()` below. (Holding it across the `else`, as an `if let
                // Some(hit) = cache.lock()…get()` would, deadlocks on the re-lock — the guard lives to the
                // end of the `if let`.)
                let cached = cache
                    .lock()
                    .expect("LexicalIndex cache poisoned")
                    .get(&key)
                    .cloned();
                if let Some(hit) = cached {
                    hit
                } else {
                    let items = self.probe_form(index_iri, normalizer, &key);
                    cache
                        .lock()
                        .expect("LexicalIndex cache poisoned")
                        .insert(key, items.clone());
                    items
                }
            }
        };
        // Merge the document-augmentation overlay (§6a): a doc's OOV groundings, seeded alongside the
        // persisted entries so a DB-backed parse sees them without their being committed.
        if let Some(extra) = self.overlay.get(form_lc) {
            out.extend(extra.iter().cloned());
        }
        out
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
        if self.reserved.coord_connective(&s_lc).is_some() {
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
        // A productive `-ly` adverb whose adjective base is known, a lexicalized discourse adverb, or
        // a morphologically-derived adjective whose base is known (D63 compound morphology §3), is
        // parseable — *known*, not a missing lexeme.
        self.is_derived_adverb(&s_lc)
            || is_lexicalized_adverb(&s_lc)
            || self.is_derived_adjective(&s_lc)
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

    /// Diagnostic (D1, `docs/notes/d63-nominal-modification-normal-form.md` §8): for each **adjective**
    /// entry resolved for `surface`, the [`super::category::ModifierClass`] its restrictor sem falls
    /// into — so the D1 classifier's verdict can be confirmed against the corpus's *real* lexicon
    /// (`attractive` → `Gradable`, a Boolean adjective → `Intersective`) rather than constructed sems.
    /// Returns `(cat, sense, class)` per distinct adjective entry.
    pub fn debug_modifier_classes(
        &self,
        surface: &str,
        lemmatizer: &dyn Lemmatizer,
    ) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
        for cand in self.candidate_lemmas(surface, lemmatizer) {
            for e in self.scoped(self.entries_for(&cand), None) {
                if !is_adjective_cat(e.item.cat()) {
                    continue;
                }
                let cat = super::pretty_term(e.item.cat());
                let sense = e.sense.clone().unwrap_or_default();
                if !seen.insert((cat.clone(), sense.clone())) {
                    continue;
                }
                let class = super::category::modifier_class(e.item.sem());
                out.push((cat, sense, format!("{class:?}")));
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
        // Bare-nominal shift (core-en `bnp` + the kind-subject reading, D63 §8.5 Slice 3c): a
        // determiner-less plural/mass common noun ALSO seeds its `cat_kind` copula-subject edge
        // ("Genes are cell lines" → subclass_of) and its raised bare-argument NPs (`kind_of(t)`, §7.4).
        // The SAME [`Self::bare_nominal_shifts`] rule runs here at leaf seeding and on composed cells in
        // the CKY (both chart paths), so a compound noun shifts identically to a leaf noun.
        let shifts: Vec<Item> = out
            .iter()
            .flat_map(|it| self.bare_nominal_shifts(it))
            .collect();
        out.extend(shifts);
        out
    }

    /// The **bare KIND NP shift** (D63 kind-predication reshape §7.4,
    /// `docs/notes/d63-kind-predication-reshape.md`) — one rule for a determiner-less **mass OR plural**
    /// common noun (core-en's `bnp`, `det=nil`, is likewise a single rule over `pl-or-mass`). The noun
    /// denotes its KIND, and as a bare argument it is that kind *realized as an individual*, `kind_of(t)
    /// : Entity` (Chierchia's ∩): a **closed** reading — "genes affect HeLa" → `affect(hela,
    /// kind_of(Gene))`, "instability affects HeLa" → `affects(kind_of(Instability), hela)`. Not the
    /// earlier deferred-quantifier (`Quantification`-hole, now retired) open parse — a generic is a *complete* proposition
    /// about the kind, and its warrant (citation / observation / derivation) belongs on the claim's
    /// **grade**, not a parser hole.
    ///
    /// `det_form` is the existential determiner whose subject- (`fwd`) and object- (`bwd`) type-raised
    /// CATEGORIES are reused: `a` for mass (singular agreement), `these` for plural. The raised category
    /// is built **directly** — substitute the noun's BASE class for the determiner's type variable — NOT
    /// via [`apply`]. That bypass is **load-bearing**: routing a REFINED (compound / relative) noun
    /// `cat_n(Σx:C. R, num)` through `apply` hits the GQ witness-projection (`DetRefine`, `parser.rs`),
    /// producing the ill-typed `Fst(kind_of(Σ))` — a kind nominalizes the WHOLE type, it does not project
    /// witnesses (this was the bare-plural-compound bug, "nucleotide repeat regions"). Indexing the raised
    /// category by the base `C` (`C ≤ Entity`) lets it fill a verb slot; the sem nominalizes
    /// `kind_of(Σx:C. R)`, keeping the compound's content. Type-raising (vs a plain `cat_np`) keeps it
    /// **argument-only**, so it cannot feed the named-entity compound rule — a noun's prenominal reading
    /// stays the `compound_kind` classifier, no spurious `compound(x, kind_of(C))` duplicate (§7.5).
    fn kind_raised_nps(&self, noun: &Item, det_form: &str, want_num: &str) -> Vec<Item> {
        let Some([t, num]) = is_ctor(noun.cat(), "cat_n") else {
            return Vec::new();
        };
        if !matches!(num, Exp::InductiveCtor(_, n, _) if n == want_num) {
            return Vec::new();
        }
        let base = base_class(t); // the raised category's NP index (a class in the subsumption lattice)
        let kind = kind_of(t.clone()); // the nominalized whole type — `kind_of(Σx:C.R)` for a compound
        self.entries_for(det_form)
            .iter()
            .filter_map(|det| {
                let head = cat_forall_body_head(det.item.cat())?;
                let Some([_dnum, body_lam]) = is_ctor(det.item.cat(), "cat_forall") else {
                    return None;
                };
                let Exp::Lam(Patt::Var(tvar), body) = body_lam else {
                    return None;
                };
                let mut subst = CatSubst::new();
                subst.insert(tvar.clone(), base.clone());
                let cat = subst_cat(body, &subst);
                let sem = match head {
                    // subject-raised `S/(S\NP)`: `λV. V(kind)`.
                    "fwd" => Exp::Lam(
                        Patt::Var("V".into()),
                        Box::new(Exp::App(
                            Box::new(Exp::Var("V".into())),
                            Box::new(kind.clone()),
                        )),
                    ),
                    // object-raised `(S\NP)\((S\NP)/NP)`: `λTV. λsubj. TV(kind, subj)`.
                    "bwd" => {
                        let tv_app = Exp::App(
                            Box::new(Exp::App(
                                Box::new(Exp::Var("TV".into())),
                                Box::new(kind.clone()),
                            )),
                            Box::new(Exp::Var("subj".into())),
                        );
                        Exp::Lam(
                            Patt::Var("TV".into()),
                            Box::new(Exp::Lam(Patt::Var("subj".into()), Box::new(tv_app))),
                        )
                    }
                    _ => return None,
                };
                Some(Item::with_cost(cat, sem, noun.cost()))
            })
            .collect()
    }

    /// Bare-MASS NP shift — the kind shift over a mass noun, singular agreement (reuse `a`).
    fn bare_mass_nps(&self, noun: &Item) -> Vec<Item> {
        self.kind_raised_nps(noun, "a", "mass")
    }

    /// Bare-PLURAL NP shift — the kind shift over a plural noun, plural agreement (reuse `these`). A bare
    /// plural denotes its kind (Carlson 1977), identically to a bare mass noun — only surface number
    /// differs — so it shares [`Self::kind_raised_nps`] (the §7.4 mass/plural unification). A bare
    /// *singular* count noun (`*gene is a vulnerability`) correctly does not shift.
    fn bare_plural_nps(&self, noun: &Item) -> Vec<Item> {
        self.kind_raised_nps(noun, "these", "pl")
    }

    /// The full **bare-nominal shift** (core-en's `bnp` unary rule + the copula kind-subject reading,
    /// D63 §8.5 Slice 3c): given a `cat_n`, produce (i) the `cat_kind` **copula-subject** edge
    /// ([`crate::dcg::kind_subject`]; a bare-plural kind, so `are_kind` yields `subclass_of`) and (ii)
    /// the raised **bare-argument NPs** ([`Self::bare_plural_nps`]/[`Self::bare_mass_nps`]). The single
    /// rule applied at BOTH leaf seeding AND to COMPOSED cells in both chart paths, so a compound
    /// `cat_n` (`repeat regions`, formed by the `KindCompound` rule) shifts exactly like a leaf noun —
    /// `bnp` is a rule over any `n`, not a leaf-only shortcut. Non-`cat_n`/non-plural/non-mass → empty.
    fn bare_nominal_shifts(&self, it: &Item) -> Vec<Item> {
        let mut v: Vec<Item> = crate::dcg::kind_subject(it.cat(), it.sem())
            .map(|(cat, sem)| Item::with_cost(cat, sem, it.cost()))
            .into_iter()
            .collect();
        v.extend(self.bare_plural_nps(it));
        v.extend(self.bare_mass_nps(it));
        v
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

    /// Whether `surface` is a morphologically-derived adjective whose base is **known to the
    /// lexicon** (D63 compound morphology, `docs/notes/d63-compound-morphology.md` §3, Slice 1): a
    /// closed-prefix concatenation (`hypermutable` → `mutable`) or a right-headed hyphen compound
    /// (`double-stranded` → `stranded`) whose base/head resolves to a predicative adjective. Shared
    /// by [`Self::derived_adjective_items`] (seeding) and [`Self::has_token`] (the missing-lexeme
    /// diagnostic), so a derived adjective counts as *known*. Mirrors [`Self::is_derived_adverb`].
    fn is_derived_adjective(&self, surface: &str) -> bool {
        let s = surface.trim().to_lowercase();
        // Slice 1: a closed-prefix / hyphen compound whose base is a known adjective.
        let slice1 = adjective_bases(&s).iter().any(|b| {
            self.entries_for(b)
                .iter()
                .any(|e| is_adjective_cat(e.item.cat()))
        });
        // Slice 2: `X-<suffix>` (denominal) where X is a known noun and the relation verb is available.
        slice1 || self.denominal_suffix_item(&s).is_some()
    }

    /// Derived-adjective items (D63 compound morphology §3). If `surface` is a recognized derived
    /// adjective ([`Self::is_derived_adjective`]), seed its `ADJ` `Item`(s) on the whole-token span,
    /// modifying nouns through the existing attributive-adjective refine rule (`RefineKind::Attrib`):
    ///   * **Slice 1** (`hypermutable`, `double-stranded`) — the base adjective's own items, the
    ///     prefix / hyphen modifier transparent (identity sem, like the `-ly` adverbs), so
    ///     `hypermutable ≡ mutable`;
    ///   * **Slice 2** (`X-based`) — a constructed `λx. base(x, kind_of(X))` predicate over the
    ///     `base` verb axiom ([`Self::denominal_based_item`]).
    ///
    /// Empty when no base resolves.
    fn derived_adjective_items(&self, surface: &str) -> Vec<Item> {
        let s = surface.trim().to_lowercase();
        let mut out = Vec::new();
        // Slice 1 (identity): reuse the base adjective's own items.
        for b in adjective_bases(&s) {
            for e in self.entries_for(&b) {
                if is_adjective_cat(e.item.cat()) {
                    out.push(e.item);
                }
            }
        }
        // Slice 2 (denominal `X-<suffix>`): the constructed `rel(…)` predicate over the element's verb.
        if let Some(it) = self.denominal_suffix_item(&s) {
            out.push(it);
        }
        out
    }

    /// The denominal-suffix adjective `X-E` (D63 compound morphology §3b, generalized from the shipped
    /// `-based` slice — see [`DENOMINAL_SUFFIXES`]): `X-<suffix>` (X a known common noun) seeds a
    /// predicative `ADJ` (`S[adj]\NP`) with sem `λθ. rel(…)`, reusing the element's WordNet verb axiom —
    /// *not* a freshly-minted relation. The verb entry carries the bare 2-place axiom as its sem (like
    /// the demo `affects`; imported WordNet verbs likewise). Argument order is set by the suffix's
    /// `theta_is_object` (passive-participle `rel(θ, X)` vs adjective/active `rel(X, θ)`). Treating the
    /// coarse 2-place axiom as the relation is the v1 representation; the faithful `rel(theme, ground)`
    /// roles and the phrasal `E link X` convergence are the passive-voice / alignment tracks
    /// (`docs/notes/d63-{passive-voice-handling,denominal-suffix-alignment}.md`).
    /// `None` unless `surface` is `X-<suffix>` for a known suffix, X resolves to a common noun, the
    /// element's relation verb is in the lexicon, and the `S[adj]\NP` inductives resolve.
    fn denominal_suffix_item(&self, surface: &str) -> Option<Item> {
        let s = surface.trim().to_lowercase();
        let (x_form, tail) = s.rsplit_once('-')?;
        if x_form.len() < 2 {
            return None;
        }
        let &(_, rel_lemma, theta_is_object) =
            DENOMINAL_SUFFIXES.iter().find(|(suf, _, _)| *suf == tail)?;
        // The element's relation — a binary-relation verb (transitive or argument-PP) carries the raw
        // 2-place `Entity → Entity → Prop` axiom as its sem.
        let rel_ax = self
            .entries_for(rel_lemma)
            .into_iter()
            .find(|e| is_binary_relation_cat(e.item.cat()))
            .map(|e| e.item.sem().clone())?;
        // X's entity: the noun's class realized as its kind (`kind_of(C)`), as a bare argument commits.
        let x_class = self.entries_for(x_form).into_iter().find_map(|e| {
            match is_ctor(e.item.cat(), "cat_n") {
                Some([t, _]) => Some(t.clone()),
                _ => None,
            }
        })?;
        let x_ent = kind_of(x_class);
        let adj_cat = predicative_adjective_cat(&self.layer)?;
        // sem `λθ. rel(first, second)` — argument order by voice (`theta_is_object`): θ in the object
        // slot for a passive participle (`θ is based on X` → `rel(θ, X)`), the noun in the object slot
        // for an adjective/active element (`θ resembles X` → `rel(X, θ)`).
        let tv = "__den_theta";
        let theta = Exp::Var(tv.to_string());
        let (first, second) = if theta_is_object {
            (theta, x_ent)
        } else {
            (x_ent, theta)
        };
        let sem = Exp::Lam(
            Patt::Var(tv.to_string()),
            Box::new(Exp::App(
                Box::new(Exp::App(Box::new(rel_ax), Box::new(first))),
                Box::new(second),
            )),
        );
        Some(Item::new(adj_cat, sem))
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
            if !self.reserved.is(&tokens[p], ReservedKind::WhRelativizer) {
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

    /// Packed-forest parse (D63 Option A, blueprint §11 3d) with **widen-on-failure** (§11 3g.2 / B9):
    /// try the packed extractor at the sense cap; if it yields nothing AND every token is known (not an
    /// OOV miss), double the cap up to [`SENSE_CAP_WIDEN_MAX`] and retry — the same "a dropped sense
    /// never loses a known-vocabulary parse" contract the unpacked path keeps ([`Self::parse_unpacked`],
    /// exercised by `sense_cap_widens_on_failure_for_known_vocabulary`). No cell-beam escalation:
    /// packing bounds the chart by cube pruning, not the per-cell beam, so only the cap can drop a
    /// needed sense. Reached only for index-independent, construct-free sentences (the router's guard),
    /// so it is equivalent to [`Self::parse_unpacked`] on those (the differential oracle, 3f).
    fn parse_packed(
        &self,
        text: &str,
        lemmatizer: &dyn Lemmatizer,
        scope: Option<&[Iri]>,
    ) -> (Vec<Item>, Vec<OpenParse>) {
        // Contextual sense ranking computed ONCE (as in the unpacked path), threaded into each attempt.
        let ranks = self.contextual_sense_ranks(text, lemmatizer, scope);
        // Pass 1 — the reranked order (static, if no ranker configured).
        let (closed, open) = self.widen_packed(text, lemmatizer, scope, ranks.as_ref());
        if !closed.is_empty() || !open.is_empty() {
            return (closed, open);
        }
        // Pass 2 — STATIC-RANK FALLBACK (GH #97; mirrors [`Self::parse_unpacked`]). The untrusted
        // reranker can bury a construction-triggered category variant that static rank + widen keeps;
        // escalating the cap within the reranked order never recovers it. Retry ONCE under static rank
        // when the reranked order gaps on an all-known-vocabulary sentence.
        if ranks.is_some() && self.all_prose_tokens_known(text, lemmatizer) {
            return self.widen_packed(text, lemmatizer, scope, None);
        }
        (closed, open)
    }

    /// One full packed widen-on-failure escalation under a FIXED sense order (`ranks`): parse at the
    /// base cap, and while an all-known-vocabulary sentence yields nothing, double the sense cap (up to
    /// [`SENSE_CAP_WIDEN_MAX`]) and retry. No cell-beam escalation — packing bounds the chart by cube
    /// pruning, not the per-cell beam, so only the cap can drop a needed sense. Called by
    /// [`Self::parse_packed`] once under the reranked order, once under static rank (the fallback).
    fn widen_packed(
        &self,
        text: &str,
        lemmatizer: &dyn Lemmatizer,
        scope: Option<&[Iri]>,
        ranks: Option<&BTreeMap<String, u32>>,
    ) -> (Vec<Item>, Vec<OpenParse>) {
        let mut cap = self.sense_cap;
        loop {
            let (closed, open) = self.parse_packed_at_cap(text, lemmatizer, scope, cap, ranks);
            if !closed.is_empty() || !open.is_empty() {
                return (closed, open);
            }
            // Widen only if a pruning artifact could be the cause (no OOV token).
            if !self.all_prose_tokens_known(text, lemmatizer) {
                return (closed, open);
            }
            match cap {
                Some(c) if c < SENSE_CAP_WIDEN_MAX => cap = Some((c * 2).min(SENSE_CAP_WIDEN_MAX)),
                _ => return (closed, open),
            }
        }
    }

    /// One packed-forest parse at a fixed sense `cap` (the widen-loop body of [`Self::parse_packed`]):
    /// build the shared forest ([`Self::build_forest`]), extract the top-span k-best via cube pruning
    /// ([`Self::kbest`]), and apply the felicity pop-filter ([`Self::classify_felicitous`]) — routing
    /// each survivor to the closed or open forest, exactly as [`Self::parse_at_cap`] does.
    fn parse_packed_at_cap(
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
        let forest = self.build_forest(&tokens, lemmatizer, scope, cap, ranks);
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

        // Hole context for classification — identical to the unpacked path. Only the referent
        // (`EntityRef`, pronoun/possessor → D64) hole remains; the bare-plural/mass quantification hole
        // was retired with the kind-predication reshape (Phase B).
        let entity_ty = Exp::EigonClass(iri(ENTITY_IRI));
        let types_ok = eval(&entity_ty, &Rho::Nil).is_ok();
        let mut hole_specs: Vec<(String, Exp, HoleKind)> = Vec::new();
        if types_ok {
            for i in 0..n {
                for j in i..n {
                    hole_specs.push((hole_base(i, j), entity_ty.clone(), HoleKind::EntityRef));
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
        Self::subsume_duplicates(&mut forest_out); // D3: collapse definitionally-equal readings
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
                // The list-with-operator model (D63 §8.4 Phase 3): a prop-ending conjunct builds/extends
                // a deferred `cat_coord` (folded later by the `CoordComplete` unary edge); an NP conjunct
                // builds a `cat_group`. Each enforces its own left-branching NF (right conjunct is a
                // single non-list constituent), so no `is_coordination` guard here.
                coordinate_prop(op, l.cat(), l.sem(), r.cat(), r.sem(), &self.layer)
                    .or_else(|| coordinate_np(op, l.cat(), l.sem(), r.cat(), r.sem(), &self.layer))
                    .map(|(cat, sem)| Item::with_cost(cat, sem, cost))
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
            UnaryKind::BareNp => out.extend(self.bare_nominal_shifts(it)),
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
            UnaryKind::CoordComplete => {
                if let Some((cat, sem)) = complete_coord(it.cat(), it.sem(), &self.layer) {
                    out.push(Item::with_cost(cat, sem, it.cost()));
                }
            }
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
                    if self.reserved.is_relativizer(tokens[c].as_str()) {
                        self.binary_edges(
                            &forest,
                            (i, c - 1),
                            (c + 1, j),
                            BinRule::Relativize,
                            &mut bin,
                        );
                    }
                    // Coordination: [X] and/or/`,` [Y].
                    if let Some(op) = self.reserved.coord_connective(tokens[c].as_str()) {
                        self.binary_edges(
                            &forest,
                            (i, c - 1),
                            (c + 1, j),
                            BinRule::Coordinate(op),
                            &mut bin,
                        );
                    }
                    // Contrastive: [O₁] but not [O₂].
                    if self.reserved.is(&tokens[c], ReservedKind::ContrastiveBut)
                        && tokens
                            .get(c + 1)
                            .is_some_and(|t| self.reserved.is(t, ReservedKind::Negator))
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
                    if self.reserved.is_relativizer(tokens[c].as_str())
                        && self.reserved.is_comma(&tokens[c - 1])
                    {
                        let body_end = if self.reserved.is_comma(&tokens[j]) {
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
                if j >= 3
                    && self
                        .reserved
                        .is(&tokens[j - 1], ReservedKind::ReciprocalEach)
                    && self.reserved.is(&tokens[j], ReservedKind::ReciprocalOther)
                {
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
                // Coordination list-completion (D63 §8.4 Phase 3): fold each prop-ending `cat_coord`
                // node in this cell into its base category. The `cat_coord` node stays (a longer list
                // extends it); the completed base-category node is what a copula / matrix consumes.
                for id in forest.cells[i][j].values().copied().collect::<Vec<_>>() {
                    let rep = forest.nodes[id].rep.clone();
                    if let Some((cat, sem)) = complete_coord(rep.cat(), rep.sem(), &self.layer) {
                        let item = Item::with_cost(cat, sem, rep.cost());
                        unary.push((node_sig(&item), item, id, UnaryKind::CoordComplete));
                    }
                }
                for (sig, item, child, kind) in unary.drain(..) {
                    let nid = forest.get_or_create(i, j, sig, &item);
                    forest.push_edge(nid, Edge::Unary { child, kind });
                }
                for id in forest.cells[i][j].values().copied().collect::<Vec<_>>() {
                    let rep = forest.nodes[id].rep.clone();
                    for np in self.bare_nominal_shifts(&rep) {
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
                if i == 0 && j >= 1 && self.reserved.is_comma(&tokens[j]) {
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
        // Pass 1 — the reranked order (static, if no ranker configured).
        let (closed, open) = self.widen_unpacked(text, lemmatizer, scope, ranks.as_ref());
        if !closed.is_empty() || !open.is_empty() {
            return (closed, open);
        }
        // Pass 2 — STATIC-RANK FALLBACK (GH #97). The reranker is UNTRUSTED: if its ordering yields no
        // parse even at the max cap/beam, and the failure could be a pruning artifact (every prose token
        // known — not an OOV miss), retry ONCE under the plain static `sense_rank` order. The reranker
        // can bury a *construction-triggered category variant* — e.g. the `cat_measure` reading of a
        // gradable nominalization (`greater dependence on X than Y`) — that static rank + widen would
        // keep; escalating the cap WITHIN the reranked order never recovers it. This restores the "a bad
        // rank costs a re-parse, never a missed parse" contract to the whole widen half, not just the cap.
        if ranks.is_some() && self.all_prose_tokens_known(text, lemmatizer) {
            return self.widen_unpacked(text, lemmatizer, scope, None);
        }
        (closed, open)
    }

    /// One full unpacked widen-on-failure escalation under a FIXED sense order (`ranks`): parse at the
    /// base cap/beam, and while an all-known-vocabulary sentence yields nothing, escalate beam-first
    /// then the sense cap (up to [`CELL_BEAM_WIDEN_MAX`] / [`SENSE_CAP_WIDEN_MAX`]) and retry. Returns
    /// the first non-empty forest, or the empty pair when the escalation is exhausted / an OOV blocks
    /// widening. Called by [`Self::parse_unpacked`] — once under the reranked order, once under static
    /// rank (the untrusted-reranker fallback).
    fn widen_unpacked(
        &self,
        text: &str,
        lemmatizer: &dyn Lemmatizer,
        scope: Option<&[Iri]>,
        ranks: Option<&BTreeMap<String, u32>>,
    ) -> (Vec<Item>, Vec<OpenParse>) {
        let mut cap = self.sense_cap;
        let mut beam = self.cell_beam;
        loop {
            let (closed, open) = self.parse_at_cap(text, lemmatizer, scope, cap, ranks, beam);
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
                    chart[i][j].push(it);
                }
                // Derived `-ly` adverbs (D62 Phase 3): transparent modifier items for a single `-ly`
                // token whose adjective base is known. Single-token spans; identity sem, no holes.
                if i == j {
                    for it in self.adverb_items(&surface) {
                        chart[i][j].push(it);
                    }
                    // Derived adjectives (D63 compound morphology §3, Slice 1): a closed-prefix /
                    // hyphen compound whose base is a known adjective seeds the base's transparent
                    // items on the whole-token span (`hypermutable ≡ mutable`).
                    for it in self.derived_adjective_items(&surface) {
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
            .map(|t| self.reserved.coord_connective(t.as_str()))
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
                // Coordination (D63 §8.4 Phase 3, the list-with-operator model ported from core-en):
                // `[X] and/or/`,` [Y]`. Prop-ending conjuncts build/extend a deferred `cat_coord` list
                // (`coordinate_prop`), folded later by the completion shift; NP conjuncts build a
                // member-retaining `cat_group` (`coordinate_np`), distributed/collected at the verb. BOTH
                // defer the operator, so a comma is neutral (`LIST_CONN`) and the trailing `and`/`or`
                // finalizes the list — `A, B, C or D` = all-`∨`. Each builder enforces its own
                // left-branching NF (the right conjunct is a single, non-list constituent).
                for c in (i + 1)..j {
                    let Some(op) = coord_op[c] else { continue };
                    let lefts = &chart[i][c - 1];
                    let rights = &chart[c + 1][j];
                    for l in lefts {
                        for r in rights {
                            if let Some((cat, sem)) =
                                coordinate_prop(op, l.cat(), l.sem(), r.cat(), r.sem(), &self.layer)
                            {
                                produced.push(Item::with_cost(
                                    cat,
                                    sem,
                                    l.cost().saturating_add(r.cost()),
                                ));
                            }
                            if let Some((cat, sem)) =
                                coordinate_np(op, l.cat(), l.sem(), r.cat(), r.sem(), &self.layer)
                            {
                                produced.push(Item::with_cost(
                                    cat,
                                    sem,
                                    l.cost().saturating_add(r.cost()),
                                ));
                            }
                        }
                    }
                }
                // Close nominal apposition (D63 §8.4 Phase 6, RC-6): a definite/bare common-noun HEAD
                // immediately followed by a coreferential NAME-GROUP — "the genes BRCA1 and MSH2",
                // "the MMR genes MSH2, MSH6, PMS2 or MLH1". The named group SPECIFIES the head's
                // referents, so `appose_group` passes the group through (gated on the members being of
                // the head noun's base kind); the result rides the existing distributive-subject /
                // -object machinery unchanged. Head and group are ADJACENT (no reserved token between),
                // so every split `m` is tried and the rule gates by shape.
                for m in i..j {
                    let heads = &chart[i][m];
                    let groups = &chart[m + 1][j];
                    for head in heads {
                        for grp in groups {
                            if let Some((cat, sem)) =
                                appose_group(head.cat(), grp.cat(), grp.sem(), &self.layer)
                            {
                                produced.push(Item::with_cost(
                                    cat,
                                    sem,
                                    head.cost().saturating_add(grp.cost()),
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
                    if !self.reserved.is(&tokens[c], ReservedKind::ContrastiveBut)
                        || !tokens
                            .get(c + 1)
                            .is_some_and(|t| self.reserved.is(t, ReservedKind::Negator))
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
                if j >= 3
                    && self
                        .reserved
                        .is(&tokens[j - 1], ReservedKind::ReciprocalEach)
                    && self.reserved.is(&tokens[j], ReservedKind::ReciprocalOther)
                {
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
                    if !self.reserved.is_relativizer(tokens[c].as_str()) {
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
                    if !self.reserved.is_relativizer(tokens[c].as_str()) {
                        continue;
                    }
                    if !self.reserved.is_comma(&tokens[c - 1]) {
                        continue;
                    }
                    let ante_end = c - 2; // antecedent NP is [i, c-2] (before the comma)
                    let body_end = if self.reserved.is_comma(&tokens[j]) {
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
                    if !tokens
                        .get(p + 1)
                        .is_some_and(|t| self.reserved.is(t, ReservedKind::WhRelativizer))
                    {
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
                // Coordination list-completion (D63 §8.4 Phase 3, core-en's `s-list`/`pred-adj-list`):
                // fold a prop-ending `cat_coord` list in this cell into its base category (`op(op(m₀,
                // m₁),…)`). Added ALONGSIDE the `cat_coord` (which stays, so a longer list can still
                // extend it); the completed base-category item is what a copula / matrix consumes. A
                // completed coordination can't re-enter `coordinate_prop` (its NF blocks an `And`/`Or`
                // sem), so this stays single-valued.
                let completed: Vec<Item> = chart[i][j]
                    .iter()
                    .filter_map(|it| {
                        complete_coord(it.cat(), it.sem(), &self.layer)
                            .map(|(cat, sem)| Item::with_cost(cat, sem, it.cost()))
                    })
                    .collect();
                chart[i][j].extend(completed);
                // Bare-nominal shift for COMPOSED common nouns (N-N compounds like "repeat regions" /
                // "MSI cancer models", adjective-refined nouns like "novel therapies" / "synthetic
                // lethality"): the leaf shift in `lookup_span` only covers lexical nouns, so a *composed*
                // `cat_n(_, pl/mass)` cell needs the SAME [`Self::bare_nominal_shifts`] here — both the
                // raised bare-argument NPs AND the `cat_kind` copula-subject edge, so a compound kind can
                // be an `are_kind` subject ("repeat regions are microsatellites"). The kind-subject arm
                // was missing here (only the argument NPs ran), so a compound-kind subject gapped.
                // Symmetric with the leaf path and the packed forest's `UnaryKind::BareNp`.
                let bare: Vec<Item> = chart[i][j]
                    .iter()
                    .flat_map(|it| self.bare_nominal_shifts(it))
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
                if i == 0 && len >= 2 && self.reserved.is_comma(&tokens[j]) {
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
        // The hole context for classification (D64 carrier): for every span `[i,j]`, a referent hole
        // (`Entity`/`EntityRef`, a pronoun/possessor). The bare-plural/mass quantification hole was
        // retired with the kind-predication reshape (Phase B — bare plural/mass now commit to
        // `kind_of(t)`, `LexicalIndex::kind_raised_nps`). A candidate mentions only the hole vars it
        // actually carries; `classify_felicitous` filters to those.
        let entity_ty = Exp::EigonClass(iri(ENTITY_IRI));
        // Degenerate guard (preserved): if the hole type can't even be evaluated, fall back to the
        // closed-only path. Normally it evals fine.
        let types_ok = eval(&entity_ty, &Rho::Nil).is_ok();
        let mut hole_specs: Vec<(String, Exp, HoleKind)> = Vec::new();
        if types_ok {
            for i in 0..n {
                for j in i..n {
                    hole_specs.push((hole_base(i, j), entity_ty.clone(), HoleKind::EntityRef));
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
        Self::subsume_duplicates(&mut forest); // D3: collapse definitionally-equal readings

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

    /// Build-then-subsume (D3, `docs/notes/d63-nominal-modification-normal-form.md` §8; Eisner 1996's
    /// exact restricted-grammar fallback): drop a closed reading whose sem is **definitionally equal**
    /// to one already kept. [`Self::reduced_felicitous`] / [`Self::classify_felicitous`] have already
    /// normalized every sem to its NbE normal form, so equal *meaning* is now equal *structure* — this
    /// collapses spurious ambiguity (different derivations, one reading) and, being an equality, **never
    /// drops a distinct reading** (the rare luxury the typed kernel affords). Uses structural `Exp`
    /// equality on the FULL IRIs — not the lossy [`super::pretty_term`], which shortens an IRI to its
    /// local segment and could false-merge two distinct senses. O(n²) over the pre-cap forest, which the
    /// felicity gate has already bounded to the classify-candidate count.
    fn subsume_duplicates(forest: &mut Vec<Item>) {
        let mut out: Vec<Item> = Vec::with_capacity(forest.len());
        for it in forest.drain(..) {
            if !out
                .iter()
                .any(|k| k.cat() == it.cat() && k.sem() == it.sem())
            {
                out.push(it);
            }
        }
        *forest = out;
    }

    /// Classify a full-span candidate as a CLOSED felicitous parse or an OPEN one carrying
    /// unresolved holes (D64), or reject it. Generalizes [`Self::reduced_felicitous`] to
    /// hole-bearing sems: each hole is a free variable, so it is bound in `rho` to a generic neutral
    /// (else Pure `eval` errors `UnboundVariable`) and in `gamma` to **its own type** so `check`
    /// types it. `hole_specs` carries every candidate hole `(base name, type, kind)`; a candidate
    /// mentions only the subset it actually carries — currently `EntityRef` holes (`Entity`, in
    /// argument position: a pronoun/possessor referent → D64). `Neut::Gen(0, base)` reads back as
    /// `Var("{base}0")`, so the gamma key and reported hole name use that readback form. With no holes
    /// present this is exactly `reduced_felicitous` (empty `rho`/`gamma`) — the closed path is unchanged.
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

    /// **Stage C — the discourse resolve loop** (D64 §4, `docs/design/d64-llm-anaphora-resolution.md`).
    /// Parse the document's `sentences` IN ORDER, threading a growing candidate set of antecedents. For
    /// each sentence: parse; if the best full parse is already CLOSED keep it; if it is OPEN (carries
    /// `EntityRef` referent holes — a pronoun / "these X"), resolve every hole against the in-scope
    /// `candidates` via [`Self::resolve_with`] (the untrusted `proposer` suggests, the kernel re-gates);
    /// a gap or unresolvable hole yields `None` (**fail-closed**). Then harvest the resolved sentence's
    /// referenced named entities into the candidate set — **most-recent-first** — for later sentences.
    /// Returns one resolved (closed) [`Item`] per input sentence.
    ///
    /// This is the piece D64 §4 leaves to the caller: the resolver primitives already exist, but nothing
    /// assembled candidates or threaded the discourse. The `proposer` is impl-agnostic — a deterministic
    /// mock in tests, the live `AnthropicProposer` (`use-llm`) end to end, or the orchestrator bridge
    /// (Phase 2). Recency is the only salience signal we model; the proposer does the ranking (§4). First
    /// cut: candidate surfaces are the entity IRI local names (a readable label is a later refinement),
    /// and only PRIOR-discourse entities are candidates (intra-sentential binding is a refinement).
    pub fn resolve_document(
        &self,
        sentences: &[&str],
        lemmatizer: &dyn Lemmatizer,
        proposer: &dyn Proposer,
    ) -> Vec<SentenceOutcome> {
        let mut candidates: Vec<Candidate> = Vec::new();
        let mut out = Vec::with_capacity(sentences.len());
        for s in sentences {
            let (mut closed, open) = self.parse_open(s, lemmatizer);
            let outcome = if closed.len() == 1 {
                SentenceOutcome::Encoded(closed.pop().expect("len==1"))
            } else if closed.len() > 1 {
                SentenceOutcome::Ambiguous(closed)
            } else if let Some(o) = open.first() {
                // OPEN: try to resolve its referent holes against the discourse; unresolvable ⇒ stays open.
                match self.resolve_with(o, s, &candidates, proposer) {
                    Some(item) => SentenceOutcome::Encoded(item),
                    None => SentenceOutcome::Open(o.clone()),
                }
            } else {
                SentenceOutcome::Gap
            };
            // Thread the discourse: harvest the chosen reading's named entities (most-recent-first) into
            // the candidate set for the following sentences' anaphora.
            let harvest = match &outcome {
                SentenceOutcome::Encoded(item) => Some(item.sem()),
                SentenceOutcome::Ambiguous(items) => items.first().map(Item::sem),
                _ => None,
            };
            if let Some(sem) = harvest {
                let mut fresh = entity_candidates(sem);
                fresh.append(&mut candidates);
                candidates = fresh;
            }
            out.push(outcome);
        }
        out
    }
}

/// The outcome of encoding one sentence — the classified result of [`LexicalIndex::resolve_document`]
/// (and the document pipeline). Fail-closed: a sentence that cannot be encoded is `Open` or `Gap`, never
/// a silently-dropped or wrong closed parse.
#[derive(Clone)]
pub enum SentenceOutcome {
    /// A single closed, resolved proposition — the encoded knowledge (`item.sem()` is the `Prop`).
    Encoded(Item),
    /// Multiple closed parses: the sentence parses but carries unresolved sense/structural ambiguity.
    Ambiguous(Vec<Item>),
    /// Parsed but carries an unresolved referent hole — the anaphora proposer found no antecedent.
    Open(OpenParse),
    /// No parse — an OOV token, or an all-known-tokens grammar gap.
    Gap,
}

/// The named-entity antecedent candidates a resolved sem references — every `EigonResource` IRI (a
/// committed named entity), as a [`Candidate`] whose surface is the IRI local name (the part after the
/// last `:`), in first-seen order. Used by [`LexicalIndex::resolve_document`] to build the discourse
/// candidate set. (Kinds / prior propositions as antecedents are a later refinement.)
fn entity_candidates(sem: &Exp) -> Vec<Candidate> {
    fn walk(e: &Exp, out: &mut Vec<Candidate>, seen: &mut BTreeSet<Iri>) {
        match e {
            Exp::EigonResource(res) => {
                if let Some(iri) = res.id() {
                    if seen.insert(iri.clone()) {
                        let surface = iri.as_str().rsplit(':').next().unwrap_or("").to_string();
                        out.push(Candidate {
                            iri: iri.clone(),
                            surface,
                        });
                    }
                }
            }
            Exp::App(a, b) | Exp::Arrow(a, b) | Exp::Times(a, b) | Exp::Pair(a, b) => {
                walk(a, out, seen);
                walk(b, out, seen);
            }
            Exp::Pi(_, a, b) | Exp::Sig(_, a, b) | Exp::Ann(a, b) => {
                walk(a, out, seen);
                walk(b, out, seen);
            }
            Exp::Lam(_, b) | Exp::Fst(b) | Exp::Snd(b) => walk(b, out, seen),
            Exp::InductiveType(_, args) | Exp::InductiveCtor(_, _, args) => {
                for a in args {
                    walk(a, out, seen);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    walk(sem, &mut out, &mut seen);
    out
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
/// deterministic mock (tests), a feature-gated live LLM client (`use-llm`), and the production
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

// The bare-plural/mass **deferred-quantifier** machinery — the determiner sems (`quant_apply`,
// `deferred_quant_subj_sem`, `deferred_quant_obj_sem`), the `$quanthole$` sentinel + `freshen_quant`,
// `quant_hole_type`/`quant_hole_base`, the per-span registration, and the `HoleKind::Quantification`
// variant — was RETIRED with the D63 kind-predication reshape (Phase B, 2026-07-04). Bare mass AND bare
// plural now commit to the closed kind-predication `kind_of(t)` (`LexicalIndex::kind_raised_nps`), so no
// quantification hole is ever produced; the full-UMLS re-measure confirmed OPEN=0 (§7.2), which
// justified removing it rather than leaving it inert. The `EntityRef` referent hole (pronouns/possessors
// → D64, `freshen_anaphor`) is unrelated and stays.

/// A `kind_of(A)` application — the class value `A` (a `Set`) realized as the `Entity` that is that
/// kind (Chierchia's ∩; the axiom `ontology:kind_of : Set -> Entity`, D63 kind-predication reshape).
fn kind_of(a: Exp) -> Exp {
    Exp::App(
        Box::new(Exp::EigonAxiom(iri("urn:eigenius:ontology:kind_of"))),
        Box::new(a),
    )
}

/// The base (non-refined) class of a common-noun type: peel `Σx:C. R` down to `C` (recursively, for
/// stacked refinements), else the type itself. A bare kind NP's raised category is indexed by this base
/// so it sits in the subsumption lattice (`C ≤ Entity`), while its sem nominalizes the WHOLE type
/// (`kind_of(Σx:C. R)`) — D63 kind-predication reshape §7.4 ([`LexicalIndex::kind_raised_nps`]).
fn base_class(t: &Exp) -> Exp {
    match t {
        Exp::Sig(_, base, _) => base_class(base),
        other => other.clone(),
    }
}

// `kind_subj_sem` / `kind_obj_sem` (the Phase-A committed determiner sems) were folded into
// [`LexicalIndex::kind_raised_nps`] (D63 reshape §7.4): the raised subject/object sems are now built
// there directly, with `kind_of(t)` pre-substituted, so the kind shift never routes through `apply`'s
// `DetRefine` witness-projection (which mis-fired `Fst(kind_of(Σ))` on refined/compound nouns).

/// What a hole dispatches to once resolved (the carrier's resolver tag — D64). Currently the single
/// `EntityRef` (pronoun/possessive referents → the D64 anaphora resolver), an *internal-resolution*
/// hole. (The `Quantification` variant — a bare plural's deferred determiner — was removed with the
/// kind-predication reshape Phase B, since bare plural/mass now commit to `kind_of(t)`; `ProofObligation`
/// for factive presuppositions is a planned future arm.) The carrier types each hole per its kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HoleKind {
    /// An unresolved entity referent (a pronoun / possessor), resolved by substituting a chain
    /// antecedent and re-gating. First-order, `Entity`-typed, in argument position.
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

/// Candidate adjective bases for a morphologically-derived adjective (D63 compound morphology,
/// `docs/notes/d63-compound-morphology.md` §3, Slice 1) — orthographic reverse-derivation, each
/// candidate probed against the lexicon in [`LexicalIndex::is_derived_adjective`] (data-driven, no
/// hardcoded adjective list; a non-adjective base simply fails the probe). Two productive shapes:
///   * a **closed prefix** `{hyper,hypo,poly,multi,mono}` concatenated onto a known adjective
///     (`hypermutable` → `mutable`);
///   * a **right-headed hyphen compound** whose head is a known adjective (`double-stranded` →
///     `stranded`).
///
/// The affix / left modifier is transparent in v1 (identity sem — the derived word reuses the base's
/// items), so only the base (prefix-stripped stem / compound head) is returned. Participial denominal
/// tails (`-based`) are Slice 2, handled separately, and are excluded here so they do not pick up a
/// wrong identity reading.
fn adjective_bases(surface: &str) -> Vec<String> {
    // Productive biomedical adjective prefixes (a declarative closed set, not a corpus-frequency
    // splitter — §2 "closed affix inventory, not frequency splitting").
    const ADJ_PREFIXES: &[&str] = &["hyper", "hypo", "poly", "multi", "mono"];
    let s = surface.trim().to_lowercase();
    let mut bases = Vec::new();
    if let Some((_, head)) = s.rsplit_once('-') {
        // Right-headed hyphen compound: the head (last segment) carries the category — UNLESS it is a
        // denominal suffix (`-based`/`-like`/…), which are handled by [`denominal_suffix_item`], not the
        // Slice-1 identity rule. Excluding them fixes the `-like` over-generation (§3b).
        let is_denominal = DENOMINAL_SUFFIXES.iter().any(|(suf, _, _)| *suf == head);
        if head.len() >= 3 && !is_denominal {
            bases.push(head.to_string());
        }
    } else {
        // Concatenated closed prefix.
        for p in ADJ_PREFIXES {
            if let Some(stem) = s.strip_prefix(p) {
                if stem.len() >= 3 {
                    bases.push(stem.to_string());
                }
            }
        }
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

/// Whether `cat` is a **binary relation** verb — `(S\NP)/NP` (transitive) or `(S\NP)/cat_pp_arg`
/// (argument-PP, e.g. `depend on`) — both carrying a raw 2-place `Entity → Entity → Prop` axiom as
/// their sem. Used by the denominal-suffix rule (D63 compound morphology §3b) to fetch each element's
/// relation from its verb lemma.
fn is_binary_relation_cat(cat: &Exp) -> bool {
    let Some([inner, obj]) = is_ctor(cat, "fwd") else {
        return false;
    };
    if is_ctor(obj, "cat_np").is_none() && is_ctor(obj, "cat_pp_arg").is_none() {
        return false;
    }
    let Some([s, subj]) = is_ctor(inner, "bwd") else {
        return false;
    };
    is_ctor(s, "cat_s").is_some() && is_ctor(subj, "cat_np").is_some()
}

/// The productive denominal-adjective suffixes (D63 compound morphology §3b, generalized from the
/// shipped `-based` slice). Each row is `(suffix_tail, relation_lemma, theta_is_object)`:
///   * `relation_lemma` — the verb lemma whose **2-place** axiom is the relation ([`is_binary_relation_cat`]).
///     Adjective-voice suffixes (`-like`, `-dependent`, `-related`) route to the corresponding *verb*
///     (`resemble`/`depend`/`relate`), since the 1-place adjective (`like`) is not a relation.
///   * `theta_is_object` — the modified noun's role, which fixes the argument order. **Passive-participle**
///     suffixes (`θ is based/mediated/… BY/ON X`) make θ the object → `rel(θ, X)`; **adjective/active**
///     suffixes (`θ resembles / depends on X`) make θ the subject → `rel(X, θ)`. Under the object-first
///     verb convention both render `rel(a, b)` = "b ⟨rel⟩ a".
///
/// Every tail here is also excluded from the Slice-1 hyphen-head identity rule ([`adjective_bases`]),
/// which fixes the `-like` over-generation (`like` is a WordNet adjective, so Slice-1 would otherwise
/// seed identity `like(x)` and drop `X`). A tail whose `relation_lemma` is absent from the lexicon just
/// fails the probe → the token stays OOV (fail-safe), never a wrong reading. `-specific` is omitted
/// (no verb relation; needs a minted `specific_to` — deferred).
const DENOMINAL_SUFFIXES: &[(&str, &str, bool)] = &[
    ("based", "base", true),
    ("mediated", "mediate", true),
    ("derived", "derive", true),
    ("induced", "induce", true),
    ("like", "resemble", false),
    ("dependent", "depend", false),
    ("related", "relate", false),
];

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
