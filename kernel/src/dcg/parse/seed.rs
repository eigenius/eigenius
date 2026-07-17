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
//! **Seeding** — surface tokens to the chart's leaf cells (bridge stage 2).
//!
//! For every token span (bounded by the longest multiword form) the surface is reduced to candidate
//! lemmas via the [`Lemmatizer`], looked up in the lexical index, filtered by the parse scope, capped by
//! the adaptive-supertagging sense cap (optionally reordered by the contextual reranker), and turned
//! into parse [`Item`]s. A multiword entry (`cell line`, `act on`) seeds its whole span *alongside* the
//! single-token items for its parts — the MWE-vs-compositional ambiguity is carried as competing chart
//! edges, not resolved here.
//!
//! Also here: the productive MORPHOLOGY that has no lexical entry of its own (`-ly` adverbs, denominal
//! and prefixed adjectives, degree-modified adverbs), and the leaf UNARY shifts (the bare-plural/mass
//! kind shift, type-raising) — the same shifts the CKY re-applies to composed cells, so a compound noun
//! shifts exactly like a leaf one.
//!
//! Shared by BOTH chart paths: [`Parser::seed_leaves`] is the single entry point the packed forest
//! and the flat beamed chart both build their leaf cells from.

use super::super::category::{is_adjective_cat, is_binary_relation_cat, kind_of};
use super::super::chart::{beam_cell, cell_histogram, Chart};
use super::super::lexicon::{FormEntries, LexEntry};
use super::*;

impl Parser {
    /// The lexical items for one token span's surface: the raw surface plus every
    /// lemma the [`Lemmatizer`] yields across all parts of speech (so an inflected
    /// or collocated form resolves to its base entries). Candidate strings are
    /// de-duplicated before lookup. `scope` filters + ranks by lexicon (§4). `ranks`, when
    /// present, is the per-sentence contextual sense ranking (`sense → rank`) that overrides the
    /// static `sense_rank` ordering when the cap drops senses.
    pub(super) fn lookup_span(
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
            self.lex
                .entries_for(surf)
                .iter()
                .any(|e| e.in_lexicon.is_none())
        };
        let surface_is_function = self.config.pos_prune
            && (has_closed(&s_lc) || {
                let toks: Vec<&str> = s_lc.split_whitespace().collect();
                toks.len() > 1 && toks.iter().all(|t| has_closed(t))
            });
        // Cross-POS ADJECTIVE prune (ALWAYS ON — a correctness fix, not the `pos_prune` experiment):
        // a surface that carries a closed-class DETERMINER reading (a quantifier — its category is
        // `cat_forall(num, λT. …)`) must not ALSO be read as an open-class gradable ADJECTIVE. WordNet
        // ships `several`/`many`/`few`/`most`/`both` as descriptive adjectives; prenominally the
        // attributive rule (`refine_attrib`) turns that `S[adj]\NP` into a spurious `gt(deg_a, std_a)`
        // modifier ("more-several-than-standard cancers") that duplicates the determiner reading —
        // over-generation, `experiments/parsing/near-encoded-bucket-analysis.md`. Keyed on the
        // DETERMINER category, so a plain adjective (no closed-class determiner reading on the surface)
        // is untouched. A rare PREDICATIVE use ("the problems were several") is given up with it; these
        // words are quantifiers, not gradable properties, in this grammar.
        let surface_is_determiner = self
            .lex
            .entries_for(&s_lc)
            .iter()
            .any(|e| e.in_lexicon.is_none() && is_ctor(e.item.cat(), "cat_forall").is_some());
        let mut out = Vec::new();
        for c in &candidates {
            let mut entries = self.scoped(self.lex.entries_for(c), scope);
            if surface_is_function {
                entries.retain(|e| {
                    e.in_lexicon.is_none()
                        || !(is_ctor(e.item.cat(), "cat_n").is_some()
                            || is_ctor(e.item.cat(), "cat_np").is_some())
                });
            }
            if surface_is_determiner {
                entries.retain(|e| e.in_lexicon.is_none() || !is_adjective_cat(e.item.cat()));
            }
            if entries.is_empty() {
                continue;
            }
            // **Collapse entries that denote the SAME concept — BEFORE the cap** (D63 cross-lexicon
            // unification, `docs/notes/d63-wordnet-umls-concept-unification.md`).
            //
            // Same predicate [`Self::subsume_duplicates`] applies to the parsed forest — structural
            // `Exp` equality on `(cat, sem)`, full IRIs, never `pretty_term` (which shortens an IRI
            // to its local segment and could false-merge two distinct senses). Being an equality it
            // **cannot drop a distinct reading**.
            //
            // What it earns is the ORDER. `subsume_duplicates` runs *after* parsing: by then the cap
            // has already spent a slot on the duplicate and the chart has already built it. Run here,
            // a duplicate never consumes a cap slot at all — which is the point, because with
            // `SENSE_CAP = 2` a word gets only two. **Measured 2026-07-11 over the WRN page: 47% of
            // ranked words spent BOTH slots on a UMLS/WordNet pair of the same concept** (`state`'s
            // two survivors have verbatim-identical glosses).
            //
            // INERT until the lexica are unified: two entries from different lexica carry different
            // class IRIs, so `(cat, sem)` differs and nothing collapses. The mass/count variants of
            // one concept differ in `cat` (`cat_n(C, mass)` vs `cat_n(C, num_any)`) and are likewise
            // preserved. O(n²) over one lemma's senses, which is small.
            dedup_same_concept(&mut entries);
            // Adaptive-supertagging sense cap (GH #97): keep at most `cap` entries for this lemma —
            // by contextual plausibility first (the reranker's `ranks`, when present), falling back
            // to the static `sense_rank` (most-frequent first) — cutting WordNet polysemy at the
            // seed. The closed class (≤ cap entries) is untouched. A stable sort preserves seed
            // order within a rank. (`cap` is the per-attempt cap from the widen loop.)
            if let Some(cap) = cap {
                // **The ranker's ELIMINATION signal.** The reranker returns a ranking that may OMIT
                // senses it judges impossible in this sentence; an omitted sense is absent from
                // `ranks`, so `sense_cap_key` sorts it after every ranked one. Without the cut below
                // the cap would simply fill its quota from those rejects — which is exactly how `of`
                // seeded a reading of `BRIP1 wt Allele` and `may` one of `Month of May`: the model
                // ranked the correct sense #0 and the cap, obliged to take TWO, grabbed the next.
                //
                // So at the BASE cap, take no more than the ranker kept. **On widen (cap above the
                // base) the cut is ignored** and the eliminated senses become seedable again — a
                // wrong elimination therefore costs a slower parse, never a grammar gap.
                // **The CLOSED CLASS is never eliminated.** It is the grammatical core — the
                // determiner `each`, the preposition `of` — and the ranker is untrusted. Observed
                // 2026-07-12: the model eliminated the determiner reading of `each` and kept UMLS's
                // "Each (qualifier value)" instead. Counting the closed class is not enough:
                // `sense_cap_key` sorts UNRANKED entries last, so a truncate would drop exactly the
                // entry that must survive. So partition, and cap only the open class.
                // **The reranker's ELIMINATION signal.** Its ranking may OMIT a sense it judges
                // impossible here; an omitted sense is absent from `ranks`, so `sense_cap_key` sorts
                // it after every ranked one. Without the cut below, the cap would simply fill its
                // quota from those rejects — which is how `of` seeded a reading of `BRIP1 wt Allele`
                // and `may` one of `Month of May`: the model ranked the correct sense #0, and the
                // cap, obliged to take TWO, grabbed the next off the list.
                //
                // The cut applies at the BASE cap only. On **widen** (cap above the base) it is
                // skipped and the eliminated senses become seedable again, so a wrong elimination
                // costs a slower parse, never a grammar gap.
                //
                // KNOWN GAP (2026-07-12): the ranker can eliminate a CLOSED-CLASS reading — it
                // dropped the determiner `each` in favour of UMLS's "Each (qualifier value)".
                // `sense_cap_key` sorts unranked entries last, so the truncate takes exactly the
                // entry that should be exempt. Widen recovers it (the sweep held at `grammar-gap 0`),
                // but the grammatical core should not be eliminable at all. Partitioning the closed
                // class out is the fix; a first attempt broke `sense_reranker_overrides_static_cap_order`
                // and needs the seeding path understood before retrying.
                let mut eff = cap;
                if Some(cap) == self.config.sense_cap {
                    if let Some(r) = ranks {
                        let ranked = entries
                            .iter()
                            .filter(|e| e.sense.as_deref().is_some_and(|s| r.contains_key(s)))
                            .count();
                        if ranked > 0 {
                            eff = eff.min(ranked);
                        }
                    }
                }
                if entries.len() > eff {
                    entries.sort_by_key(|e| sense_cap_key(e, ranks));
                    entries.truncate(eff);
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
            .flat_map(|it| self.grammar.bare_nominal_shifts(it))
            .collect();
        out.extend(shifts);
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
    pub(super) fn is_derived_adverb(&self, surface: &str) -> bool {
        let s = surface.trim().to_lowercase();
        adverb_bases(&s).iter().any(|b| {
            self.lex
                .entries_for(b)
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
        let mut cats = adverb_modifier_cats(&self.grammar.layer).unwrap_or_default();
        if lexicalized {
            cats.extend(sentence_modifier_cats(&self.grammar.layer).unwrap_or_default());
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
    pub(super) fn is_derived_adjective(&self, surface: &str) -> bool {
        let s = surface.trim().to_lowercase();
        // Slice 1: a closed-prefix / hyphen compound whose base is a known adjective.
        let slice1 = adjective_bases(&s).iter().any(|b| {
            self.lex
                .entries_for(b)
                .iter()
                .any(|e| is_adjective_cat(e.item.cat()))
        });
        // Slice 2: `X-<suffix>` (denominal) where X is a known noun and the relation verb is available.
        slice1 || self.denominal_suffix_item(&s).is_some()
    }

    /// Derived-adjective items (D63 compound morphology §3). If `surface` is a recognized derived
    /// adjective ([`Self::is_derived_adjective`]), seed its `ADJ` `Item`(s) on the whole-token span,
    /// modifying nouns through the existing attributive-adjective refine rule (the `attrib` rule in
    /// `combine_nominal_mod`'s table):
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
            for e in self.lex.entries_for(&b) {
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
            .lex
            .entries_for(rel_lemma)
            .into_iter()
            .find(|e| is_binary_relation_cat(e.item.cat()))
            .map(|e| e.item.sem().clone())?;
        // X's entity: the noun's class realized as its kind (`kind_of(C)`), as a bare argument commits.
        let x_class = self.lex.entries_for(x_form).into_iter().find_map(|e| {
            match is_ctor(e.item.cat(), "cat_n") {
                Some([t, _]) => Some(t.clone()),
                _ => None,
            }
        })?;
        let x_ent = kind_of(x_class);
        let adj_cat = predicative_adjective_cat(&self.grammar.layer)?;
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
        let mut cats = adverb_modifier_cats(&self.grammar.layer).unwrap_or_default();
        cats.extend(sentence_modifier_cats(&self.grammar.layer).unwrap_or_default());
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

    /// Seed the LEAF cells of a CKY chart — the shared front-end of both the unpacked path
    /// ([`Self::parse_at_cap`]) and the packed forest (D63 blueprint §11 3c.3). Multi-span MWE
    /// [`Self::lookup_span`] + hole-freshening (`$anaphor$`/`$quant$`) + `-ly`/degree adverbs +
    /// fronted participials + leaf forward type-raising, optionally per-cell beamed. Returns the
    /// `n × n` chart (only leaf spans `[i,j]` populated) and the accumulated beam-drop count.
    /// Behaviour-identical to the inline seeding it replaces — the packed path calls it with
    /// `beam = None` (packing bounds via k-best, not a beam).
    pub(super) fn seed_leaves(
        &self,
        tokens: &[String],
        lemmatizer: &dyn Lemmatizer,
        scope: Option<&[Iri]>,
        cap: Option<usize>,
        ranks: Option<&BTreeMap<String, u32>>,
        beam: Option<usize>,
    ) -> (Chart, usize) {
        let debug = std::env::var("EIGENIUS_PARSE_DEBUG").is_ok();
        let n = tokens.len();
        // chart[i][j] = every item spanning tokens i..=j.
        let mut chart: Chart = vec![vec![Vec::new(); n]; n];

        // 1. Seed lexical spans (multi-span MWE seeding). A multiword form at [i,j] is seeded
        //    ALONGSIDE the items of its parts, so both readings survive into the chart.
        let span_limit = self.lex.span_limit(n);
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
                            front_participial(it.cat(), it.sem(), &self.grammar.layer).map(
                                |(c, s)| {
                                    Item::with_cost(
                                        c,
                                        freshen_anaphor(&s, &hole_base(i, j)),
                                        it.cost(),
                                    )
                                },
                            )
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
            let raised = self.grammar.raise_nps(&row[i]);
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
/// candidate probed against the lexicon in [`Parser::is_derived_adjective`] (data-driven, no
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
pub(super) fn is_lexicalized_adverb(surface: &str) -> bool {
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

/// **Collapse entries that denote the SAME concept** — structural `Exp` equality on `(cat, sem)`,
/// full IRIs (never `super::super::pretty_term`, which shortens an IRI to its local segment and could
/// false-merge two distinct senses). Being an equality it **cannot drop a distinct reading**: a
/// different category, or a different denotation, survives.
///
/// The same predicate `Parser::subsume_duplicates` applies to the parsed forest. What this
/// earns is the **ORDER**: `subsume_duplicates` runs *after* parsing, by which point the sense cap
/// has already spent a slot on the duplicate and the chart has already built it. Run at seed time —
/// **before the cap** — a duplicate never consumes a cap slot at all, which is the point, because
/// with `SENSE_CAP = 2` a word gets only two of them.
///
/// **Why it matters** (D63, `docs/notes/d63-wordnet-umls-concept-unification.md`): WordNet and UMLS
/// each mint their own class for the same concept — `state` is `wn:n00024720` *and*
/// `umlscui:C1442792`, with verbatim-identical glosses. Measured over the WRN page (2026-07-11),
/// **47% of ranked words spent BOTH cap slots on such a cross-lexicon pair**, so no genuine
/// alternative could seed at all.
///
/// **INERT until the lexica are unified**: two entries from different lexica carry different class
/// IRIs, so `(cat, sem)` differs and nothing collapses. It fires only once the alignment layer makes
/// both entries denote one class. O(n²) over one lemma's senses, which is small.
pub(super) fn dedup_same_concept(entries: &mut FormEntries) {
    if entries.len() < 2 {
        return;
    }
    let mut kept: Vec<usize> = Vec::with_capacity(entries.len());
    for i in 0..entries.len() {
        let dup = kept.iter().any(|&j| {
            entries[j].item.cat() == entries[i].item.cat()
                && entries[j].item.sem() == entries[i].item.sem()
        });
        if !dup {
            kept.push(i);
        }
    }
    if kept.len() == entries.len() {
        return;
    }
    let mut i = 0usize;
    entries.retain(|_| {
        let keep = kept.contains(&i);
        i += 1;
        keep
    });
}

pub(super) fn sense_cap_key(
    e: &LexEntry,
    ranks: Option<&BTreeMap<String, u32>>,
) -> (bool, u32, u32) {
    let ctx = e
        .sense
        .as_ref()
        .and_then(|s| ranks.and_then(|m| m.get(s).copied()));
    (ctx.is_none(), ctx.unwrap_or(0), e.item.cost().sense_rank)
}

/// Instantiate a common noun's underspecified `num_any` with the surface number.
/// Only a `cat_n(T, num_any)` item is refined (to `cat_n(T, <num>)`); verbs,
/// names, and multiword leaves pass through unchanged. The `lexicon:Num` decl is
/// reused from the existing `num_any` ctor, so no decl lookup is needed.
pub(super) fn with_noun_num(it: &Item, num_name: &str) -> Item {
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
