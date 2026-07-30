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

//! **The parser** (D62 §8.8.1): a surface string → the forest of typed parses.
//!
//! Four stages, and the last one is the only one that decides anything:
//!
//! 1. **tokenize** the input ([`tokenize`]);
//! 2. **seed** the chart ([`seed`]) — for every token span (bounded by the longest multiword form),
//!    reduce the surface to candidate lemmas via the [`Lemmatizer`] and look them up in the lexicon.
//!    A multiword entry (`cell line`, `act on`) seeds its whole span *alongside* the single-token items
//!    for its parts: the MWE-vs-compositional ambiguity is carried as competing chart edges, not
//!    resolved here. Ranking, capping and beaming happen at this stage — all of them bounded and
//!    reversible (widen-on-failure), so a bad rank costs a re-parse, never a missed parse;
//! 3. **compose** — CKY over the categorial rules ([`super::rules`]), driven by [`super::chart`];
//! 4. **gate** ([`felicity`]) — the KERNEL type-checks the assembled sem against `⟦cat⟧`. A parse is
//!    admitted iff the kernel types it. Nothing else here is trusted to judge that.
//!
//! Then [`resolve`] (D64) binds any referent holes against the discourse — again propose-then-gate: the
//! untrusted proposer suggests an antecedent, the kernel re-checks the substitution.
//!
//! The parser returns the WHOLE forest (no selection, no commit). Selecting one parse and committing it
//! as a `lexicon:Sentence` is the encoding institution's job (§8.8.2–8.8.3); an empty forest is a
//! first-class outcome (no admissible parse), **not an error**.
//!
//! It reaches the lexicon only through [`LexicalLookup`](super::lexicon::LexicalLookup) — a two-method
//! trait held as `Arc<dyn …>`, so parsing cannot re-accrete onto the lexicon. It reaches the grammar
//! only through [`Grammar`](super::grammar::Grammar) — a value, not a service, so no rule can reach for
//! a word. (This module was called `lookup`, and was a `form → entries` map, before it grew all of the
//! above.)

// The stages, as child modules. Each holds the `impl Parser` block for the stage it names.
mod felicity;
mod paths;
mod resolve;
mod seed;

// Re-exported so `dcg::parse::X` keeps resolving exactly as before the split (`dcg/mod.rs` and
// `pipeline.rs` import these paths).
pub use felicity::{HoleInfo, HoleKind, OpenParse};
pub use resolve::{Candidate, ProposeCtx, Proposer, SentenceOutcome};

use super::category::{is_adjective_cat, is_vp_adjunct_prep};
use super::holes::{freshen_anaphor, hole_base};
use felicity::{is_finite_clause, FelicitousOutcome};
use seed::is_lexicalized_adverb;

use super::grammar::{DetTemplates, Grammar};
use super::lexicon::{read_description, FormEntries, LexEntry, LexicalIndex, LexicalLookup};
use super::segment::tokenize;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::layer::Layer;
use crate::nbe::check::{check, exp_mentions_var, CheckCtx};
use crate::nbe::env::Rho;
use crate::nbe::eval::eval;
use crate::nbe::readback::{readback_val, try_readback_val};
use crate::nbe::term::{Exp, Patt};
use crate::nbe::val::{Neut, Val};
use crate::ontology::Iri;

use super::category::{
    adverb_modifier_cats, denote_cat, is_ctor, predicative_adjective_cat, sentence_modifier_cats,
};
use super::item::Item;
use super::lemmatizer::{Lemmatizer, Pos};
use super::rules::constructions::front_participial;

use super::reserved::{ReservedKind, ReservedTable};
use super::sense_ranker::{SenseCandidate, SenseRanker, WordSenses};

/// Default forest cap (D63 §8.7 Stage B): `parse` returns at most this many parses,
/// the lowest-cost (most-frequent-sense) first; the rest are dropped with a log line.
/// Chosen from the scale-up baselines — short sentences over full-WordNet polysemy
/// reach ~2k well-typed parses, so this bounds the forest while keeping every
/// plausible reading; it sits far above any closed-class / demo forest, so those are
/// unaffected (no truncation, order preserved by the stable cost-0 sort).
///
/// **256 → 2048 (2026-07-25).** At 256 this was not a safety bound, it was deciding the grammar: on
/// "PARP-1 inhibitors are successful in cancers with deficiencies in homologous recombination." the
/// reported structure count tracked the constant itself — k=256 → 2 skeletons, 1024 → 6, 2048 → 13,
/// with the fully-nested (correct) reading last in every prefix. A result that scales smoothly with
/// an arbitrary constant is a measurement of the constant. Raised with the [`CLASSIFY_BUDGET`] log
/// as the standing signal for whether it still binds; see that constant for why a fixed number is a
/// step rather than the design.
pub const DEFAULT_FOREST_CAP: usize = 2048;

/// **Felicity-eval budget** (fail-closed OOM guard): the number of full-span candidates the
/// felicity loop will NbE-eval, after cost-sorting. The top chart cell is unbeamed (Lever B beams
/// only `len < n`), so with sub-cells beamed to `cell_beam` it can hold up to ~`cell_beam²·n`
/// candidates; over the full lexicon, widen-on-failure escalation makes that thousands, and each
/// felicity check is a full eval/readback/check of an **impredicative-∃** GQ sem — evaluating all of
/// them OOMs (witnessed: ~400 doubly-∃ candidates SIGKILL the process). Cost-sorting first and
/// classifying only the lowest-cost `CLASSIFY_BUDGET` bounds the work.
///
/// The budget is spent on **distinct sems** ([`retain_distinct_sems`]), not on raw candidates. A
/// duplicate costs a full NbE eval and buys nothing: `subsume_duplicates` collapses it afterwards
/// anyway. Witnessed 2026-07-25 on "We also identified MSI cell lines from rare lineages." — after
/// core-en `bnp` gave a bare kind a plain `cat_np`, every kind-argument reading acquired a second
/// derivation and the unit reached **376 candidates carrying 44 distinct sems** (88% duplicate
/// mass). The duplicates filled the 256 window, pushing that unit's correct nested-PP readings —
/// which sit higher in the cost order than the flat VP-adjunct ones — out of it entirely: 4
/// structural skeletons → 2, with no diagnostic, and `grammar-gap` blind to it because the sentence
/// still parsed. Pre-`bnp` the same unit fitted (192 candidates) and kept all 4. Deduplicating
/// first is what makes "bounds the work without changing the result" true rather than aspirational.
///
/// …and it is spent across **distinct bracketings** ([`skeleton::spread_over_keys`]), not down a
/// cost prefix, because a cost prefix is biased *systematically* against the correct reading: a
/// deeper derivation costs more, so the first thing a prefix drops is the deeply-nested PP
/// attachment.
///
/// **THE NUMBER IS A STEP, NOT THE DESIGN.** There is no constant at which this stops binding —
/// candidate count grows with length and coordination density, and at 4096 the page's 24-token unit
/// ("Project Achilles and project DRIVE identified WRN as…") still offered 5920 distinct candidates.
/// The OOM justification above is also STALE: the witnessed ~400-candidate SIGKILL predates the
/// distinct-sem dedup, so those 400 raw candidates would be far fewer evals today, and nobody has
/// re-measured where the real limit sits. 2048 is adopted because it is strictly better than 256 on
/// every measurement taken, and because the truncation is no longer silent — it LOGS what it drops,
/// and once the expected-reading corpus covers all 62 units a truncated-away correct reading trips
/// the faithfulness gate instead of hiding in an improving metric. That pairing (loud log + full
/// pin coverage) is the feedback loop this constant never had; an outcome-keyed adaptive rule
/// ("stop when the last N evals yield no new post-felicity skeleton") is only worth building if
/// that loop shows the fixed number still costing us readings.
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

/// The **processing parameters** of a parse — every knob that is a decision about *how hard to look*,
/// not about what the language means. Kept in one struct, owned by [`Parser`], and deliberately NOT on
/// the lexicon: a lexicon has no opinion about beam widths.
#[derive(Default)]
pub struct ParseConfig {
    /// Optional **sense cap** (adaptive supertagging, D63 parsing-scale plan / GH #97): seed at
    /// most this many entries per lemma, the lowest-`sense_rank` (most-frequent / highest-prior)
    /// first. `None` = uncapped (default). Caps the WordNet sense-polysemy that drives the chart
    /// blow-up on long sentences; the closed class (1–few entries per form) is unaffected. Pair with
    /// widen-on-failure for completeness.
    sense_cap: Option<usize>,
    /// Optional **contextual sense reranker** (GH #97) — the *strong* form of the sense cap. When set
    /// (and a `sense_cap` is active), a per-sentence pre-pass asks the (untrusted) ranker to reorder
    /// each content word's candidate senses by contextual plausibility, so the senses the cap *keeps*
    /// are the ones most likely in this sentence — not merely the statically most-frequent. The ranker
    /// only reorders the seed beam; the kernel felicity gate still decides validity and widen-on-failure
    /// recovers a wrongly down-ranked sense, so a bad rank costs a re-parse, never a missed parse.
    sense_ranker: Option<Box<dyn SenseRanker + Send + Sync>>,
    /// Optional **per-cell beam** (Lever B — GH #97). Each CKY chart cell is capped to this many
    /// lowest-`Cost` items after it is built, bounding the chart's intermediate growth (the source of
    /// the full-lexicon OOM; the per-lemma `sense_cap` alone does not stop a fully-known,
    /// structurally-complex sentence's composed cells from blowing up over a dense lexicon). Applied to
    /// every non-top cell; leaf cells stay governed by `sense_cap` and the top cell by
    /// [`DEFAULT_FOREST_CAP`]. **Inexact** — like any beam it may drop a constituent the only full parse
    /// needed — so it is opt-in; `None` = the exact (unbounded) chart.
    cell_beam: Option<usize>,
    /// **Combinatory-core spike**: when set, the CKY also applies the additional CCG combinators —
    /// crossed composition (`>Bx`/`<Bx`), backward harmonic composition (`<B`), and generalized
    /// type-raising — alongside the hand-built rules, to measure how much of the composition long tail
    /// the general combinators subsume. Default `false` = the established rule-by-rule path.
    combinatory_core: bool,
    /// **Cross-POS prune** experiment (GH#97): when a surface token has a CLOSED-class (grammatical,
    /// `in_lexicon = None`) reading — it's a known function word — drop its open-class **nominal**
    /// (`cat_n`/`cat_np`) readings, the dense-lexicon noise that feeds the compound rule (`can`→
    /// container, `for`→noun, `is`→beryllium) into the sentence-spanning noun-pile. Open-class VERB/ADJ
    /// readings are KEPT (so `is`→the copula survives). Acts at seed time, so widen-on-failure cannot
    /// re-admit the dropped nouns. Opt-in; default off.
    pos_prune: bool,
    /// **Packed-forest parsing** (D63 blueprint, GH#97 Option A): route a parse to the node-level packed
    /// CKY + cube-pruning extractor instead of the flat beamed chart. Packing collapses the
    /// same-`cat_shape` sense-product into one node per signature, so combination is O(1) per node-pair.
    /// **Default ON**: the packed CKY mirrors every construct and is proven equivalent to the unpacked
    /// path (the differential oracle); `with_packing(false)` pins the unpacked baseline.
    packing: bool,
}

/// **The parsing backbone**: surface string → the forest of typed parses.
///
/// It owns every *stage* — seeding, sense ranking, both CKY drivers, the widen ladder, the felicity
/// gate, and D64 resolution — and reaches the lexicon only through [`LexicalLookup`]. That separation is
/// the point: the lexicon answers questions about words, the parser decides what to do with the answers,
/// and neither can quietly become the other.
pub struct Parser {
    /// The lexicon, behind the fence. `Arc<dyn …>` (not a concrete `LexicalIndex`) so the parser is
    /// *structurally* confined to lookup — and so an alternative lexicon can be substituted.
    lex: Arc<dyn LexicalLookup>,
    /// The rules' world: the chain, the reserved-word triggers, and the resolved category templates.
    /// The rules are `impl Grammar` — they cannot see the lexicon at all.
    grammar: Grammar,
    /// The processing parameters ([`ParseConfig`]).
    config: ParseConfig,
    /// The document this parser is reading, as sentences, for the reranker's CONTEXT WINDOW.
    /// `None` ⇒ rank each sentence in isolation (the prior behaviour). Set with
    /// [`Parser::with_document`]; the sweep supplies the page.
    document: Option<Arc<Vec<String>>>,
    /// Sentences of context on EACH side of the ranked sentence. **Default 0 — off**, so a plain
    /// parser reproduces the isolated-sentence behaviour the committed baseline was measured under.
    /// The context window CHANGES the reranker's answer (and is unproven), so it is opt-in: set it
    /// (with a document) via [`Parser::with_document`], driven by the `--context-window` measurement arm.
    context_sentences: usize,
}

/// The default context-window size the `--context-window` arm turns on. A passage, not a corpus:
/// enough to fix the domain (genomics vs geography) without burying the target sentence.
pub const CONTEXT_SENTENCES: usize = 2;

impl Parser {
    /// Build a parser over `layer` — the one-call path: constructs the [`LexicalIndex`] and wraps it.
    /// Use [`Parser::over`] instead to share ONE index across several parsers (e.g. to A/B a config
    /// without paying for the index twice).
    pub fn build(layer: Arc<Layer>) -> Self {
        let lex = Arc::new(LexicalIndex::build(Arc::clone(&layer)));
        Parser::over(lex, layer)
    }

    /// Build a parser over an existing lexicon. `lex` is taken as `Arc<dyn LexicalLookup>` — the parser
    /// only ever asks it for entries, so any lookup will do (a shared index, a test double, a
    /// document-augmented overlay).
    pub fn over(lex: Arc<dyn LexicalLookup>, layer: Arc<Layer>) -> Self {
        // The grammar is resolved ONCE, here: the reserved-word triggers from the ontology, and the
        // determiner category templates from the lexicon. This is the only moment the grammar reads the
        // lexicon; from here on the rules hold values, not a lookup.
        let grammar = Grammar {
            reserved: ReservedTable::load(&layer),
            dets: DetTemplates::resolve(lex.as_ref()),
            layer,
        };
        Parser {
            lex,
            grammar,
            config: ParseConfig {
                packing: true, // default ON (§11 3g.2 / B9)
                ..ParseConfig::default()
            },
            document: None,
            context_sentences: 0,
        }
    }

    /// Set the per-lemma **sense cap** (adaptive supertagging — GH #97): keep at most `n` entries
    /// per lemma, lowest `sense_rank` first. Cuts WordNet sense-polysemy at the seed to keep the
    /// chart tractable on long sentences. Builder-style; default (unset) is uncapped.
    pub fn with_sense_cap(mut self, n: usize) -> Self {
        self.config.sense_cap = Some(n);
        self
    }

    /// Set the **per-cell beam** (Lever B — GH #97): cap every non-top CKY cell to `n`
    /// lowest-`Cost` items, bounding the chart's intermediate growth so a fully-known
    /// structurally-complex sentence doesn't OOM over a dense lexicon (where the per-lemma
    /// `sense_cap` alone is insufficient). Inexact (may drop a constituent the only full parse
    /// needed); builder-style, default (unset) is the exact unbounded chart.
    pub fn with_cell_beam(mut self, n: usize) -> Self {
        self.config.cell_beam = Some(n);
        self
    }

    /// Enable the **combinatory-core spike**: apply the additional CCG combinators (crossed +
    /// backward-harmonic composition, generalized type-raising) alongside the hand-built rules.
    /// Builder-style; default off (the established rule-by-rule path). For the A/B port measurement.
    pub fn with_combinatory_core(mut self, on: bool) -> Self {
        self.config.combinatory_core = on;
        self
    }

    /// Enable the **cross-POS prune** experiment (GH#97): drop a function word's open-class nominal
    /// readings (see the `pos_prune` field doc). Builder-style; default off.
    pub fn with_pos_prune(mut self, on: bool) -> Self {
        self.config.pos_prune = on;
        self
    }

    /// Toggle **packed-forest parsing** ([`Self::packing`]) — node-level packing + cube-pruning
    /// extraction, gated at parse time on the grammar being index-independent. Builder-style; **default
    /// ON** (§11 3g.2 / B9). Pass `false` to pin the unpacked baseline (the differential oracle, A/B
    /// probes) — otherwise packed is used for every index-independent, construct-free sentence.
    pub fn with_packing(mut self, on: bool) -> Self {
        self.config.packing = on;
        self
    }

    /// Set the **contextual sense reranker** (GH #97) — the strong form of the sense cap. With a
    /// cap active, a per-sentence pre-pass asks `ranker` to reorder each content word's candidate
    /// senses by contextual plausibility, so the cap keeps the senses most likely *in this
    /// sentence*, not merely the statically most-frequent. No-op without a [`Self::with_sense_cap`]
    /// (the ranker only influences which senses the cap drops). Builder-style; default is the plain
    /// static `sense_rank` cap.
    pub fn with_sense_ranker(mut self, ranker: Box<dyn SenseRanker + Send + Sync>) -> Self {
        self.config.sense_ranker = Some(ranker);
        self
    }

    /// Supply the DOCUMENT (its sentences) and the context-window size for the contextual reranker.
    /// `window` sentences on EACH side of the ranked sentence enter the prompt; **`window == 0` is
    /// off** — each sentence is ranked alone, the behaviour the committed baseline was measured under.
    /// Builder-style; the `--context-window` measurement arm passes [`CONTEXT_SENTENCES`].
    ///
    /// A non-zero window CHANGES the ranker's question, hence its cache key: a `ranks.json` recorded
    /// under a different window MISSES rather than replaying a stale answer.
    pub fn with_document(mut self, sentences: Vec<String>, window: usize) -> Self {
        self.document = Some(Arc::new(sentences));
        self.context_sentences = window;
        self
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
        if self.grammar.reserved.coord_connective(&s_lc).is_some() {
            return true;
        }
        if !self.lex.entries_for(&s_lc).is_empty() {
            return true;
        }
        for pos in [Pos::Noun, Pos::Verb, Pos::Adj, Pos::Adv] {
            for lemma in lemmatizer.lemmas(surface, pos) {
                if !self
                    .lex
                    .entries_for(&lemma.trim().to_lowercase())
                    .is_empty()
                {
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
            for e in self.scoped(self.lex.entries_for(&cand), None) {
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
            for e in self.scoped(self.lex.entries_for(&cand), None) {
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
    fn scoped(&self, entries: FormEntries, scope: Option<&[Iri]>) -> Vec<LexEntry> {
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
        // Domain-plural fallback (D63 §5.1, [`Lemmatizer::regular_plural_stem`]): a DOMAIN-lexicon plural
        // the (validated) lemmatizer can't reduce (`biomarkers` — `biomarker` ∉ WordNet) gets its crude
        // singular stem here, so a real entry for that singular is offered a PLURAL reading (stem ≠
        // surface ⇒ pl in [`Self::lookup_span`]) and takes the bare-plural kind shift. `None` under a
        // no-morphology lemmatizer, so `Identity`-based demo parses are unaffected.
        if let Some(stem) = lemmatizer.regular_plural_stem(surface) {
            candidates.insert(stem.trim().to_lowercase());
        }
        candidates
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
        // when packing is enabled, the combinatory-core spike is off, and this sentence is not
        // pied-piping (the one construct the packed forest builds no edge for — `parse_needs_unpacked`).
        // Concrete selectional slots are packed per-cell now (they key finer via `node_sig`), so the
        // dense-lexicon corpus takes this path. Otherwise the unpacked beamed path (the oracle baseline,
        // and the fallback for the combinatory-core spike / pied-piping).
        if self.config.packing
            && !self.config.combinatory_core
            && !self.parse_needs_unpacked(&tokenize(text), lemmatizer, scope)
        {
            return self.parse_packed(text, lemmatizer, scope);
        }
        self.parse_unpacked(text, lemmatizer, scope)
    }

    /// Per-parse packability guard (D63 Option A, blueprint §11 3b.2). Returns `true` if this sentence
    /// must use the UNPACKED path. As of §11 3g.3 the packed CKY mirrors every token-keyed sem-reading
    /// construct (coordination, the reciprocal, `but not`, the restrictive relative, the appositive,
    /// the fronted-modifier comma) plus the wh-determiner `which` (an ordinary leaf), and as of the
    /// per-cell packing refinement ([`super::chart::forest::node_sig`]) a concrete selectional argument slot
    /// no longer forces the unpacked path — such items just key finer ([`super::pretty::cat_key`]) so
    /// they never wrongly share a node, while the index-independent majority still packs by
    /// `cat_shape`. So one **completeness** carve-out remains:
    /// - **pied-piping** (`[prep] which [subj] [VP]`) — a ternary rule the packed forest builds no edge
    ///   for (rare, non-piling), detected structurally (a `which` right after a VP-adjunct preposition)
    ///   and routed to the unpacked path. It is not a soundness case; the packed forest simply cannot
    ///   express it yet.
    fn parse_needs_unpacked(
        &self,
        tokens: &[String],
        lemmatizer: &dyn Lemmatizer,
        scope: Option<&[Iri]>,
    ) -> bool {
        let n = tokens.len();
        // Pied-piping `[prep] which`: the packed forest builds no edge for this ternary construct, so
        // route it unpacked. A `which` right after a VP-adjunct preposition is pied-piping; a `which`
        // after a noun is the packed which-relative, and a sentence-initial / post-determiner `which`
        // is the packed wh-determiner.
        for p in 1..n {
            if !self
                .grammar
                .reserved
                .is(&tokens[p], ReservedKind::WhRelativizer)
            {
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
        false
    }

    /// Whether an (unscoped) parse of `text` would take the **packed** path (D63 Option A): packing
    /// enabled, no combinatory-core spike, and the sentence index-independent + construct-free (the
    /// [`Self::parse_needs_unpacked`] guard). The routing decision is otherwise unobservable —
    /// packed ≡ unpacked by construction (the differential oracle) — so this exposes it for tests to
    /// assert *which* path a sentence takes (blueprint §11 3f.2).
    pub fn routes_packed(&self, text: &str, lemmatizer: &dyn Lemmatizer) -> bool {
        self.config.packing
            && !self.config.combinatory_core
            && !self.parse_needs_unpacked(&tokenize(text), lemmatizer, None)
    }

    /// The **parse-attempt policy** shared by both chart paths (reorganization plan Phase 1) — the
    /// reranker two-pass and the widen-on-failure escalation, written once and parameterized by the
    /// `attempt` that actually parses at a given `(cap, beam, ranks)`.
    ///
    /// Pass 1 runs under the contextual sense ranking (computed ONCE per parse — one ranker call, not
    /// one per widen iteration); pass 2 is the **static-rank fallback** (GH #97). The reranker is
    /// UNTRUSTED: if its ordering yields no parse even at the max cap/beam, and the failure could be a
    /// pruning artifact (every prose token known — not an OOV miss), retry ONCE under the plain static
    /// `sense_rank` order. The reranker can bury a *construction-triggered category variant* — e.g. the
    /// `cat_measure` reading of a gradable nominalization (`greater dependence on X than Y`) — that
    /// static rank + widen would keep, and escalating the cap WITHIN the reranked order never recovers
    /// it. So a bad rank costs a re-parse, never a missed parse.
    ///
    /// `initial_beam` is the escalation ladder's beam rung: `self.config.cell_beam` for the unpacked path, and
    /// **`None` for the packed path** — packing bounds the chart by cube pruning, not a per-cell beam,
    /// so the beam never participates there and only the sense cap escalates. That single argument is
    /// the entire difference between the two paths' policies (it was two near-identical functions,
    /// `widen_packed` / `widen_unpacked`, before Phase 1).
    fn parse_widening<F>(
        &self,
        text: &str,
        lemmatizer: &dyn Lemmatizer,
        scope: Option<&[Iri]>,
        initial_beam: Option<usize>,
        attempt: F,
    ) -> (Vec<Item>, Vec<OpenParse>)
    where
        F: Fn(
            Option<usize>,
            Option<usize>,
            Option<&BTreeMap<String, u32>>,
        ) -> (Vec<Item>, Vec<OpenParse>),
    {
        let ranks = self.contextual_sense_ranks(text, lemmatizer, scope);
        // Pass 1 — the reranked order (static, if no ranker configured).
        let (closed, open) = self.widen(text, lemmatizer, initial_beam, ranks.as_ref(), &attempt);
        if !closed.is_empty() || !open.is_empty() {
            return (closed, open);
        }
        // Pass 2 — the static-rank fallback (only when a ranker actually reordered, and the gap could
        // be a pruning artifact rather than an OOV miss).
        if ranks.is_some() && self.all_prose_tokens_known(text, lemmatizer) {
            return self.widen(text, lemmatizer, initial_beam, None, &attempt);
        }
        (closed, open)
    }

    /// One full **widen-on-failure escalation** under a FIXED sense order (`ranks`): parse at the base
    /// cap/beam, and while an all-known-vocabulary sentence yields nothing, escalate and retry. Returns
    /// the first non-empty forest, or the empty pair when the escalation is exhausted / an OOV blocks
    /// widening (an OOV miss is not a pruning miss, so it must not buy retries).
    ///
    /// Escalates **beam-first**: grow the cell beam (keeping the sense cap LOW) until it maxes
    /// ([`CELL_BEAM_WIDEN_MAX`]), then grow the sense cap ([`SENSE_CAP_WIDEN_MAX`]). Raising the cap
    /// admits more senses per lemma, which re-crowds the chart and can beam out the very constituent a
    /// wider beam was meant to keep — so a beam-limited sentence is best recovered at a low cap + wide
    /// beam, not both wide at once. A `None` beam (the packed path) simply never grows, leaving the cap
    /// as the only rung.
    fn widen<F>(
        &self,
        text: &str,
        lemmatizer: &dyn Lemmatizer,
        initial_beam: Option<usize>,
        ranks: Option<&BTreeMap<String, u32>>,
        attempt: &F,
    ) -> (Vec<Item>, Vec<OpenParse>)
    where
        F: Fn(
            Option<usize>,
            Option<usize>,
            Option<&BTreeMap<String, u32>>,
        ) -> (Vec<Item>, Vec<OpenParse>),
    {
        let mut cap = self.config.sense_cap;
        let mut beam = initial_beam;
        loop {
            let (closed, open) = attempt(cap, beam, ranks);
            if !closed.is_empty() || !open.is_empty() {
                return (closed, open);
            }
            // Widen only if a pruning artifact could be the cause (no OOV token).
            if !self.all_prose_tokens_known(text, lemmatizer) {
                return (closed, open);
            }
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
        let ranker = self.config.sense_ranker.as_deref()?;
        let cap = self.config.sense_cap?; // ranking only matters when the cap can drop senses
        let tokens = tokenize(text);
        let n = tokens.len();
        if n == 0 {
            return None;
        }
        let span_limit = self.lex.span_limit(n);

        // Gather, per over-cap span, its pooled candidate senses (deduped by sense key).
        let mut surfaces: Vec<String> = Vec::new();
        let mut cands: Vec<Vec<SenseCandidate>> = Vec::new();
        for i in 0..n {
            let last = (i + span_limit).min(n);
            for j in i..last {
                let surface = tokens[i..=j].join(" ");
                let mut senses: Vec<SenseCandidate> = Vec::new();
                let mut seen: BTreeSet<String> = BTreeSet::new();
                // CASE-SENSITIVE ACRONYM MATCH — the SAME filter `lookup_span` applies, and it has to
                // be here too or the ranker is asked about senses that can never seed.
                //
                // This pass reaches `entries_for` DIRECTLY rather than through `lookup_span`, so
                // until now it saw the unfiltered candidate list: for a lowercase `cell` it offered
                // the ranker the CELP pseudogene alongside the ordinary noun. Two costs, and the
                // second is the one that matters. The wasted prompt line is cosmetic. But the ranker
                // returns an ELIMINATION signal — at the base cap the seeder takes no more senses
                // than the ranker kept — so an impossible sense competing for a top-`cap` slot can
                // displace a real one, and the displaced sense is then unavailable to the parse.
                //
                // Deliberately NOT measurable with the cap-only instrument: this whole function is
                // gated on `sense_ranker`, so cap-only never executes it. Its reach was measured
                // instead as rank-key misses against the tracked recording — see the commit.
                let surface_all_caps = all_caps_symbol(surface.trim());
                for c in self.candidate_lemmas(&surface, lemmatizer) {
                    for e in self.scoped(self.lex.entries_for(&c), scope) {
                        if !surface_all_caps && all_caps_symbol(&e.form) {
                            continue;
                        }
                        let Some(sense) = e.sense else { continue };
                        if !seen.insert(sense.clone()) {
                            continue;
                        }
                        // The gloss the reranker reads. Priority:
                        //   1. the ENTRY's own `core:description` — the only place a function word
                        //      can say what it means (its `sem` is an inline λ-term, so it carries
                        //      no description of its own);
                        //   2. the `sem` entity's description (WordNet synsets, UMLS concepts);
                        //   3. the category, as a last resort.
                        //
                        // Before (1), a function word rendered as a BLANK LINE and the prompt asked
                        // the model to choose between `""` and a full NCI definition. It eliminated
                        // the determiner `each` and the focus particle `alone` — correctly, given
                        // that we told it nothing. A prompt we built badly, not a model failure.
                        let gloss = e
                            .gloss
                            .clone()
                            .or_else(|| self.sem_gloss(e.item.sem()))
                            .unwrap_or_else(|| {
                                format!(
                                    "grammatical (function-word) reading; category {}",
                                    super::pretty_term(e.item.cat())
                                )
                            });
                        senses.push(SenseCandidate {
                            sense,
                            gloss,
                            sem: super::pretty_term(e.item.sem()),
                        });
                    }
                }
                // Rank EVERY polysemous word — not only the ones the cap would truncate.
                //
                // The old trigger was `senses.len() > cap`, on the reasoning that ranking only
                // matters when the cap can drop something. That was true when the ranker could only
                // REORDER. Now it can ELIMINATE, and a word with exactly `cap` senses — one real,
                // one junk — was never shown to the model at all, so both seeded unfiltered. The
                // cost is a slightly longer prompt; the gain is that the junk filter reaches every
                // ambiguous word.
                let _ = cap;
                if senses.len() > 1 {
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
        let rankings = ranker.rank(text, &self.document_context(text), &words);
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

    /// The surrounding sentences for `text` — `context_sentences` on each side, joined. Empty when the
    /// window is off (the default), no document was supplied, or `text` is not one of its sentences.
    fn document_context(&self, text: &str) -> String {
        if self.context_sentences == 0 {
            return String::new(); // window off (default) — rank in isolation
        }
        let Some(doc) = self.document.as_ref() else {
            return String::new();
        };
        let t = text.trim();
        let Some(i) = doc.iter().position(|s| s.trim() == t) else {
            return String::new();
        };
        let lo = i.saturating_sub(self.context_sentences);
        let hi = (i + self.context_sentences + 1).min(doc.len());
        doc[lo..hi]
            .iter()
            .map(|s| s.trim())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The `core:description` gloss the reranker reasons over for a leaf item's sense: the description
    /// of the FIRST chain entity found in a pre-order walk of `sem` that carries one.
    ///
    /// A bare noun / UMLS sem is an `EigonClass` at the ROOT — resolved directly, as before. A gradable
    /// adjective's sem is `λx. gt(deg_X(x), std_X)`, whose sense gloss is carried on the NESTED `deg_X`
    /// axiom — "the sense's semantic anchor" (`crates/eigenius-wordnet/src/convert.rs`, D63 §6a index
    /// c), likewise a verb's synset gloss on its verb axiom — so the walk must descend the `Lam`/`App`
    /// to reach it. Before this, only the root was checked, so every gradable adjective (and any sense
    /// whose concept is nested) rendered to the reranker as the bare-category fallback
    /// ("grammatical (function-word) reading; category …"), which read as a function word to omit and
    /// lost to any described UMLS concept competing for the surface. Grammar operators (`gt`, `And`,
    /// `kind_of`, the determiner λ-terms) carry no `core:description`, so the first hit is always the
    /// sense anchor. `None` (→ the category fallback) when nothing in the term is described — a genuine
    /// closed-class λ-term.
    fn sem_gloss(&self, sem: &Exp) -> Option<String> {
        match sem {
            Exp::EigonResource(r) => {
                if let Some(d) = read_description(r) {
                    return Some(d);
                }
            }
            Exp::EigonClass(i) | Exp::EigonAxiom(i) => {
                if let Some(r) = self.grammar.layer.resolve(i) {
                    if let Some(d) = read_description(r.as_ref()) {
                        return Some(d);
                    }
                }
            }
            _ => {}
        }
        Self::sem_subterms(sem)
            .into_iter()
            .find_map(|child| self.sem_gloss(child))
    }

    /// The immediate sub-expressions a gloss walk ([`Self::sem_gloss`]) descends into — the
    /// lambda-calculus core plus inductive-ctor args, which is every shape a lexical sem takes
    /// (`Lam`/`App` for adjectives and verbs, pairs/projections, annotations, `compound_kind`-style
    /// ctors). Variants that never wrap a described concept in a lexical sem (literals, sorts,
    /// data/case, codata) yield none; a missed variant only stops the walk there (→ the category
    /// fallback), never a wrong gloss.
    pub(super) fn sem_subterms(e: &Exp) -> Vec<&Exp> {
        match e {
            Exp::Lam(_, b) | Exp::Con(_, b) | Exp::Fst(b) | Exp::Snd(b) => vec![b.as_ref()],
            Exp::App(a, b) | Exp::Arrow(a, b) | Exp::Times(a, b) | Exp::Pair(a, b) => {
                vec![a.as_ref(), b.as_ref()]
            }
            Exp::Pi(_, a, b) | Exp::Sig(_, a, b) => vec![a.as_ref(), b.as_ref()],
            Exp::Ann(a, b) => vec![a.as_ref(), b.as_ref()],
            Exp::InductiveType(_, args) | Exp::InductiveCtor(_, _, args) => args.iter().collect(),
            _ => Vec::new(),
        }
    }
}

/// IRI of the referent-hole placeholder constant (`axiom lexicon:anaphor : lexicon:Entity`):
/// a pronoun entry stores this, and the lookup bridge freshens it into a per-occurrence free
/// variable at parse time (D64 open-parse carrier).
/// IRI of the universal entity class — the type of a (Slice-1) referent hole.
const ENTITY_IRI: &str = "urn:eigenius:lexicon:Entity";

// The bare-plural/mass **deferred-quantifier** machinery — the determiner sems (`quant_apply`,
// `deferred_quant_subj_sem`, `deferred_quant_obj_sem`), the `$quanthole$` sentinel + `freshen_quant`,
// `quant_hole_type`/`quant_hole_base`, the per-span registration, and the `HoleKind::Quantification`
// variant — was RETIRED with the D63 kind-predication reshape (Phase B, 2026-07-04). Bare mass AND bare
// plural now commit to the closed kind-predication `kind_of(t)` (`LexicalIndex::kind_raised_nps`), so no
// quantification hole is ever produced; the full-UMLS re-measure confirmed OPEN=0 (§7.2), which
// justified removing it rather than leaving it inert. The `EntityRef` referent hole (pronouns/possessors
// → D64, `freshen_anaphor`) is unrelated and stays.

// `kind_subj_sem` / `kind_obj_sem` (the Phase-A committed determiner sems) were folded into
// [`LexicalIndex::kind_raised_nps`] (D63 reshape §7.4): the raised subject/object sems are now built
// there directly, with `kind_of(t)` pre-substituted, so the kind shift never routes through `apply`'s
// `DetRefine` witness-projection (which mis-fired `Fst(kind_of(Σ))` on refined/compound nouns).

/// Whether `s` is an **ALL-CAPS symbol** — at least one cased character, and none of them lowercase
/// (`CELL`, `DNA`, `MSH2`, `BILE SALT-DEPENDENT LIPASE`; not `cell`, `RecQ`, `cAMP`, `Microsatellite
/// Instability`, `2026`).
///
/// This is the discriminator for CASE-SENSITIVE ACRONYM MATCHING (the refinement
/// [`super::lexicon::LexicalIndex`] documents as deferred from v1). The lexical index is keyed on
/// LOWERCASED forms and must stay that way — sentence-initial `Cell lines…` has to reach the lemma
/// `cell` — but that fold also makes an all-caps nomenclature symbol reachable from the lowercase
/// common noun it happens to spell. So the rule is applied at the point of use and is DELIBERATELY
/// ASYMMETRIC ([`Parser::lookup_span`]):
///
/// - an all-caps ENTRY (`CELL`) is reachable only from an all-caps OBSERVED token;
/// - a non-all-caps entry (`cell`, `RecQ`, `Microsatellite Instability`) is reachable from any
///   casing, exactly as before.
///
/// The asymmetry is the whole design. Making the match symmetric (exact case both ways) breaks
/// sentence-initial capitalisation, and keying on "contains an uppercase" instead of "is all-caps"
/// would stop Title-case terminology — `Microsatellite Instability`, `Werner Syndrome` — from
/// matching the lowercase prose that actually mentions it. All-caps is the narrowest predicate that
/// separates a nomenclature SYMBOL from a name.
///
/// Measured over MRCONSO 2026AA: 178,664 distinct all-caps English atoms, of which **4,319**
/// lowercase onto a WordNet common-noun lemma. 24 of those surfaces occur on the WRN reference page,
/// and every one except `DNA`/`RNA` occurs there ONLY in lowercase — so the rule removes the
/// spurious symbol sense without costing a single real symbol mention on that page.
///
/// What this gives up: a document writing a human gene symbol in lowercase (`wrn`) no longer reaches
/// it. That is the same tradeoff already accepted for `as`=arsenic — the document glossary is the
/// recovery path — and lowercase is not the convention for human gene symbols.
pub fn all_caps_symbol(s: &str) -> bool {
    let mut saw_upper = false;
    for c in s.chars() {
        if c.is_lowercase() {
            return false;
        }
        saw_upper |= c.is_uppercase();
    }
    saw_upper
}

#[cfg(test)]
mod all_caps_symbol_tests {
    use super::all_caps_symbol;

    /// The predicate separates a nomenclature SYMBOL from a name and from ordinary prose.
    #[test]
    fn flags_symbols_and_spares_names() {
        // Symbols — reachable only from an all-caps token once the filter is on.
        for s in [
            "CELL",
            "DNA",
            "RNA",
            "MSI",
            "WRN",
            "MSH2",
            "A",
            "BILE SALT-DEPENDENT LIPASE",
        ] {
            assert!(all_caps_symbol(s), "{s} is an all-caps symbol");
        }
        // NOT symbols. `RecQ`/`cAMP` are mixed-case gene forms and `Microsatellite Instability` is
        // Title-case terminology: keying on "contains an uppercase" instead of "is all-caps" would
        // stop all three from matching the lowercase prose that mentions them.
        for s in [
            "cell",
            "RecQ",
            "cAMP",
            "Microsatellite Instability",
            "Werner Syndrome",
            "microsatellite-stable",
        ] {
            assert!(!all_caps_symbol(s), "{s} must NOT be treated as a symbol");
        }
        // No cased character at all ⇒ not a symbol (never filters anything).
        for s in ["2026", "-", "", "4-1"] {
            assert!(!all_caps_symbol(s), "{s} has no cased character");
        }
    }

    /// **The asymmetry is the design.** A symmetric (exact-case) rule would break sentence-initial
    /// capitalisation, which is why v1 folded case in the first place. Encoded as the two directions
    /// the filter in [`Parser::lookup_span`] actually takes.
    #[test]
    fn asymmetry_keeps_sentence_initial_capitalisation_working() {
        // Observed token side: `Cell` (sentence-initial) is NOT all-caps, so the filter engages and
        // drops all-caps entries — while the lowercase lemma entry `cell`, which is what
        // `Cell lines were distinct.` must reach, is not all-caps and so survives.
        assert!(!all_caps_symbol("Cell"));
        assert!(!all_caps_symbol("cell"));
        // …and the symbol is still reachable when the document actually writes it as a symbol.
        assert!(all_caps_symbol("CELL"));
    }
}

fn iri(s: &str) -> Iri {
    Iri::parse(s).expect("valid lexicon iri")
}

#[cfg(test)]
mod tests {
    use super::seed::{dedup_same_concept, sense_cap_key};
    use super::{FormEntries, LexEntry};
    use crate::dcg::item::{Combinator, Cost, Item};
    use crate::nbe::term::Exp;
    use crate::ontology::Iri;
    use std::collections::BTreeMap;

    fn iri(s: &str) -> Exp {
        Exp::EigonClass(Iri::parse(s).unwrap())
    }

    /// One entry: its category, its denotation, and the sense label it came in under.
    fn entry(cat: Exp, sem: Exp, sense: &str) -> LexEntry {
        LexEntry {
            item: Item::from_parts(cat, sem, Combinator::Other, Cost::default()),
            in_lexicon: None,
            sense: Some(sense.to_string()),
            gloss: None,
            form: String::new(),
        }
    }

    /// The reranker's ranking OMITS a sense ⇒ it is eliminated ⇒ the cap must NOT backfill from it.
    ///
    /// The real failure this pins (WRN page, 2026-07-11): the word `of` had 6 candidate senses. The
    /// model correctly ranked the closed-class preposition #0 and omitted the rest — but the ranking
    /// was "completed" by re-appending them, and `SENSE_CAP = 2`, obliged to take TWO, seeded
    /// `umls:C1879775` = **BRIP1 wt Allele**. A gene, as a reading of "of".
    #[test]
    fn an_omitted_sense_is_eliminated_and_the_cap_does_not_backfill() {
        let cat = iri("urn:eigenius:lexicon:cat_np");
        let mut e: FormEntries = vec![
            entry(cat.clone(), iri("urn:eigenius:lexicon:of"), "of"), // the closed-class preposition
            entry(
                cat.clone(),
                iri("urn:eigenius:umlscui:C1879775"),
                "umls:C1879775",
            ), // BRIP1!
            entry(
                cat.clone(),
                iri("urn:eigenius:umlscui:C0919490"),
                "umls:C0919490",
            ), // SPI1 gene
        ];
        // The ranker kept ONE sense and omitted the other two — they are absent from the map.
        let ranks: BTreeMap<String, u32> = [("of".to_string(), 0u32)].into_iter().collect();

        // Sorting alone puts the eliminated senses last (sense_cap_key keys on `ctx.is_none()`)…
        e.sort_by_key(|x| sense_cap_key(x, Some(&ranks)));
        assert_eq!(
            e[0].sense.as_deref(),
            Some("of"),
            "the ranked sense sorts first"
        );

        // …and the cut is what stops a cap of 2 from taking the gene anyway.
        let ranked = e
            .iter()
            .filter(|x| x.sense.as_deref().is_some_and(|s| ranks.contains_key(s)))
            .count();
        assert_eq!(ranked, 1, "the ranker kept exactly one sense");
        let cap = 2usize;
        assert_eq!(
            cap.min(ranked.max(1)),
            1,
            "effective cap is 1, not 2 — no backfill"
        );
    }

    #[test]
    fn dedup_collapses_entries_that_denote_the_same_concept() {
        // The shape the WordNet/UMLS alignment produces: two entries, one per lexicon, made to
        // denote ONE class — identical (cat, sem), differing only in the sense label they arrived
        // with. They must collapse to a single seed, so the pair consumes ONE cap slot, not two.
        let cat = iri("urn:eigenius:lexicon:cat_np");
        let concept = iri("urn:eigenius:wn:n00024720"); // `state`, the canonical class
        let mut e: FormEntries = vec![
            entry(cat.clone(), concept.clone(), "wn:state.n.00024720"),
            entry(cat.clone(), concept.clone(), "umls:C1442792"), // redefined to the same class
        ];
        dedup_same_concept(&mut e);
        assert_eq!(e.len(), 1, "one concept ⇒ one seed ⇒ one cap slot");
        // Seed order is preserved: the FIRST occurrence survives, so the cap's ranking is unaffected.
        assert_eq!(e[0].sense.as_deref(), Some("wn:state.n.00024720"));
    }

    #[test]
    fn dedup_never_drops_a_distinct_reading() {
        // The half that must not break. Being an EQUALITY on (cat, sem), the dedup can only remove
        // an exact duplicate — never a real alternative.
        let np = iri("urn:eigenius:lexicon:cat_np");
        let n = iri("urn:eigenius:lexicon:cat_n");
        let a = iri("urn:eigenius:wn:n00024720");
        let b = iri("urn:eigenius:wn:n05696199");
        let mut e: FormEntries = vec![
            entry(np.clone(), a.clone(), "s1"),
            entry(n.clone(), a.clone(), "s2"), // SAME concept, DIFFERENT category (e.g. mass vs count)
            entry(np.clone(), b.clone(), "s3"), // SAME category, DIFFERENT concept
        ];
        dedup_same_concept(&mut e);
        assert_eq!(
            e.len(),
            3,
            "different cat, or different sem, is a distinct reading — keep it"
        );
    }

    #[test]
    fn dedup_is_inert_on_todays_lexicon() {
        // Before alignment, WordNet and UMLS mint DIFFERENT classes for the same meaning — so
        // (cat, sem) differs and nothing collapses. This is why the dedup can land on its own and be
        // verified as a no-op against the reference measurement, ahead of the alignment layer.
        let cat = iri("urn:eigenius:lexicon:cat_np");
        let mut e: FormEntries = vec![
            entry(
                cat.clone(),
                iri("urn:eigenius:wn:n00024720"),
                "wn:state.n.00024720",
            ),
            entry(
                cat.clone(),
                iri("urn:eigenius:umlscui:C1442792"),
                "umls:C1442792",
            ),
        ];
        dedup_same_concept(&mut e);
        assert_eq!(
            e.len(),
            2,
            "un-aligned lexica: two classes, two seeds — nothing collapses"
        );
    }
}
