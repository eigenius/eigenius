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

use super::super::category::{
    governs_named_preposition, is_adjective_cat, is_binary_relation_cat, is_ctor,
    is_vp_adjunct_prep, kind_of, slash_parts,
};
use super::super::chart::{beam_cell, cell_histogram, Chart};
use super::super::lexicon::{FormEntries, LexEntry};
use super::super::rules::constructions::coordinate_np;
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
        // Cross-POS NOMINAL prune for a GLOSS-GOVERNED RELATIONAL word (ALWAYS ON — a correctness fix).
        // A surface that governs a named preposition (`concordant WITH`, `dependent ON`, `essential
        // FOR`, `associated WITH` — [`governs_named_preposition`]) must not ALSO be read as a common
        // noun. The third sibling of the two prunes above, and the same argument: the nominal reading is
        // a LEXICON ARTIFACT, here a UMLS qualifier concept whose term string happens to be an adjective
        // (`concordant` -> C4553529 "Concordance").
        //
        // What it costs when the artifact survives: the nominal feeds the predicative kind-raise
        // (`bare_nominal_shifts`), the copula predicates it, and the result asserts membership between
        // two things that cannot be identical under ANY sense assignment — `is_a(classification,
        // Concordance)` on «These classifications were highly concordant with PCR-based MSI phenotyping
        // and with predicted MMR deficiency.», the page's last two `invalid` rows. The governed
        // preposition's object is the ADJECTIVE'S COMPLEMENT, so nominalising the adjective strands it
        // and it reattaches as an adjunct on the subject.
        //
        // PRUNED AT SEEDING, not at the kind-raise. Two reasons, both structural. (1) The bad reading
        // arrives at two different sites — the leaf raise (`is_a(x, C)`) and the raise of a COMPOSED
        // `cat_n(Σx:C. prep_with(x, …))` (`is_a(x, Σ…)`) — and killing the leaf entry removes the base
        // both are built from, where a guard at the raise would have to catch each site. (2)
        // `bare_nominal_shifts` takes a bare `Item` and `Grammar` deliberately carries NO lexicon, so
        // "this surface also has a governed-preposition entry" is not knowable there; `lookup_span`
        // holds the surface and is where the sibling surface-keyed prunes already live.
        //
        // A REFUSAL AT THE KIND-RAISE WAS ALREADY TRIED AND REFUTED (see `kind_raised_nps`): gated on
        // `is_adjective_refined`, it was too broad — it also removed the legitimate "the group is an
        // INDETERMINATE line" and re-gapped its unit. That guard asked "is this nominal adjective-
        // refined?", true of the good case too. The discriminating fact is that here the nominal's HEAD
        // IS the adjective, and that adjective governs a preposition.
        let surface_is_governed_relational = self
            .lex
            .entries_for(&s_lc)
            .iter()
            .any(|e| governs_named_preposition(e.item.cat()));
        // CASE-SENSITIVE ACRONYM MATCH ([`super::all_caps_symbol`]). The index is keyed on lowercased
        // forms, so an all-caps nomenclature SYMBOL is otherwise reachable from the lowercase common
        // noun it happens to spell. `CELL` — HGNC `NS` for the CELP pseudogene (C1413337) and OMIM
        // `ACR` for CEL (C1413336) — is why `MSI cell lines from these four lineages…` carried 16
        // skeletons that read `cell` as a GENE, via `named(…)`, which only a `cat_np` named individual
        // can supply.
        //
        // Applied HERE and not in the index, for two reasons. The index deliberately "does lookup and
        // nothing else" (its `LexicalLookup` note), and `entries_for` receives a LOWERCASED key — the
        // observed token's casing is already gone by then. `lookup_span` still holds the raw surface,
        // and it is where the sibling surface-keyed prunes (`surface_is_function`,
        // `surface_is_determiner`) already live.
        //
        // Not fixable in the drop set, and it should not be: `drops.rs` keeps clean all-caps
        // collisions DELIBERATELY (`CAT` the catalase gene, `SET` the oncogene are real symbols) and
        // its `DROP_TTYS` excludes `ACR`/`NS`. The symbol is legitimate; reaching it from lowercase
        // prose is not.
        let surface_all_caps = super::all_caps_symbol(surface.trim());
        let mut out = Vec::new();
        for c in &candidates {
            let mut entries = self.scoped(self.lex.entries_for(c), scope);
            if !surface_all_caps {
                entries.retain(|e| !super::all_caps_symbol(&e.form));
            }
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
            if surface_is_governed_relational {
                entries.retain(|e| {
                    e.in_lexicon.is_none()
                        || !(is_ctor(e.item.cat(), "cat_n").is_some()
                            || is_ctor(e.item.cat(), "cat_np").is_some())
                });
            }
            // **A `-s` surface cannot take a PLURAL subject** (verb-side number refinement). Morphology
            // reduces `affects` to `affect`, and the lemma's entry is the BASE form, whose finite
            // category encodes agreement with a plural/non-3sg subject — so seeding it at a 3sg span
            // let `The genes affects cells.` parse. Traced 2026-07-26: the noun was correctly `pl`
            // throughout and the verb supplied
            // `fwd(bwd(cat_s(dcl, fin), cat_np(Entity, pl)), …)` alongside the correct `sg` one.
            //
            // The nominal counterpart of this is [`with_noun_num`], which REWRITES an underspecified
            // `num_any`; a verb's subject number is already concrete in the lexicon, so the base-form
            // entry is simply the wrong entry for this surface and is dropped rather than restamped.
            //
            // Deliberately keyed on the `-s` detachment alone
            // ([`crate::dcg::regular_plural_stem`], shared with Morphy and the UMLS importer), NOT on
            // "the candidate was reduced". The importer emits base + gerund/passive participles only,
            // so a PAST-tense surface (`affected`, `ran`) is reachable ONLY through its lemma's finite
            // entry; dropping on any reduction would gap every past-tense verb.
            if crate::dcg::regular_plural_stem(&s_lc).as_deref() == Some(c.as_str()) {
                entries.retain(|e| !has_plural_finite_subject(e.item.cat()));
            }
            if entries.is_empty() {
                continue;
            }
            // Cross-lexicon concept dedup (D63, `docs/notes/d63-wordnet-umls-concept-unification.md`)
            // runs AFTER the sense cap — see the `dedup_same_concept` call below the cap block, and the
            // comment there for why the order (cap-then-dedup) is load-bearing (the `event` case).
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
                // The cut applies at EVERY cap rung (see below), Pass 2 being the safety net — NOT
                // base-cap-only as it once was. Base-cap-only let widen un-eliminate, which re-seeded
                // the reranker's rejects onto every word the moment ONE word forced a wider cap (the
                // `gene`/`C5849123` bug).
                //
                // KNOWN GAP (2026-07-12): the ranker can eliminate a CLOSED-CLASS reading — it
                // dropped the determiner `each` in favour of UMLS's "Each (qualifier value)".
                // `sense_cap_key` sorts unranked entries last, so the truncate takes exactly the
                // entry that should be exempt. Pass 2 (the ranks-`None` widen) recovers it (the sweep
                // held at `grammar-gap 0`), but the grammatical core should not be eliminable at all.
                //
                // THE UNDERLYING SHAPE (characterised 2026-07-24): the cap is keyed per **SENSE**
                // (`sense_cap_key` reads only sense fields) but truncates per **ENTRY**. One sense
                // legitimately has SEVERAL entries — the same meaning in different grammatical
                // categories: `with` (sense "with") is a noun-modifier `cat_pp/NP`, an argument marker
                // `cat_pp_arg(prep_with)/NP` AND a VP-adjunct `((S\NP)\(S\NP))/NP`; a governed gradable
                // adjective (one WordNet sense) is a predicative `S[adj]\NP`, a `cat_measure`, a
                // `cat_measure/cat_pp_arg(prep)` and a positive-relational `(S[adj]\NP)/cat_pp_arg(prep)`.
                // Those share one key, so the truncate cuts CATEGORIES of a sense the ranker KEPT,
                // arbitrarily by emission order (a stable sort preserves it). Measured consequences: the
                // WRN page's `concordant with …` unit GAPS at base cap and floods on widen (8 of its 16
                // skeletons), `a_subj` seeds only at cap>=4, and Fix A (c)'s positive-relational entry —
                // emitted 4th — never seeds at base cap, so it is inert on the page.
                //
                // TWO FIXES MEASURED AND REJECTED (2026-07-24), both parser-only:
                //  (1) cap by SENSE (keep every category of each kept sense). Same-ranks A/B: readings
                //      724 -> 757, total-skeletons 229 -> 248, **encoded 10 -> 0** (every unit lost
                //      single-reading status) and the target unit stayed at 16. Cap-only readings fell
                //      33% (1741 -> 1169, the avoided widen-floods) but the tracked STRUCTURAL lever
                //      rose — extra categories per sense are genuine added ambiguity in the reranked
                //      regime. Net loss; not shipped.
                //  (1b) cap by SENSE, RE-TESTED 2026-07-25 on the tree that now carries the Eisner
                //      guards (`LeftNotRaised` covering `KindRaised`, `RightNotKindRaised`, the
                //      sortal-name guard), since the first rejection was blamed on duplicate
                //      derivations those guards remove. It is STILL a loss: same-ranks replay gave
                //      encoded 11 -> 4 (the guards recovered part of the earlier 10 -> 0, not enough)
                //      and readings 347 -> 352. Crucially it does NOT fix what motivated the retry:
                //      "does not lead to cell death" needs `lead`'s frame-04 PP-oblique sense
                //      (02555908 "be conducive to" / 02635956 "tend to or result in"), and those are
                //      SEPARATE SENSES among lead's 14 verb senses — not categorial variants of a KEPT
                //      sense. Capping by sense cannot reach a sense the draw never kept. That unit is a
                //      SENSE-RANKING problem (the documented ~5% temperature-0 draw variance over a
                //      2-slot cap), not a cap-mechanics one.
                //  (2) exempt the closed class (`in_lexicon.is_none()`) from the cap — the shape the
                //      note above proposes. It FAILS `sense_reranker_overrides_static_cap_order`, which
                //      is why the earlier attempt failed too: `in_lexicon.is_none()` means UNTAGGED, not
                //      closed-class. Test fixtures and demo entries are untagged as well, and the cap
                //      tests rely on their being capped, so the predicate exempts far more than the
                //      grammatical core. A real fix needs a POSITIVE closed-class marker (or a
                //      category-shape test), not the absence of a lexicon tag.
                let mut eff = cap;
                // Apply the reranker's ELIMINATION at EVERY cap rung, not just the base — INCLUDING
                // during widen. An eliminated sense (absent from `ranks`) stays out even when the cap
                // grows to admit some word's deeper RANKED sense, so a sentence that widens for ONE word
                // (e.g. `cause`/`mutations` needing its #3–4 sense) does not FLOOD every OTHER word with
                // its rejects — the `gene` junk (`C5849123` = "Gross Extranodal Extension" re-seeding for
                // `gene` via a "gENE" synonym) that only ever appeared because widen used to un-eliminate.
                // The wrong-elimination SAFETY that once justified the base-cap-only cut is provided by
                // [`Self::parse_widening`]'s PASS 2 — a ranks-`None` widen (pure frequency, no cut),
                // reached only when Pass 1 yields nothing, i.e. exactly when a NEEDED sense was wrongly
                // eliminated. So the cut can hold at all caps without ever costing a grammar gap.
                if let Some(r) = ranks {
                    let ranked = entries
                        .iter()
                        .filter(|e| e.sense.as_deref().is_some_and(|s| r.contains_key(s)))
                        .count();
                    if ranked > 0 {
                        eff = eff.min(ranked);
                    }
                }
                if entries.len() > eff {
                    entries.sort_by_key(|e| sense_cap_key(e, ranks));
                    entries.truncate(eff);
                }
            }
            // **Collapse entries that denote the SAME concept — AFTER the cap** (D63 cross-lexicon
            // unification, `docs/notes/d63-wordnet-umls-concept-unification.md`). Structural `Exp`
            // equality on `(cat, sem)` (full IRIs, never `pretty_term` which shortens an IRI to its
            // local segment and could false-merge two senses); the same predicate as the forest-side
            // [`Self::subsume_duplicates`]. Being an equality it **cannot drop a distinct reading**.
            //
            // The ORDER — cap FIRST, then dedup — is load-bearing. The reranker ranks RAW senses
            // (pre-merge), so a cross-lexicon pair is TWO votes for ONE concept. When the reranker puts
            // a concept's WordNet sense AND its UMLS twin in its top-`cap` positions, capping first
            // keeps both and this dedup collapses them to ONE reading (the word's single best sense).
            // Dedup-BEFORE-the-cap instead dropped the twin and let the cap BACKFILL the freed slot
            // with a LOWER-ranked, contextually-wrong sense — the `event` case (2026-07-16): the ranker
            // ranked event-*happening* #1 and its UMLS twin #2, the wrong event-*circumstance* sense
            // #3; pre-cap dedup promoted circumstance into the cap, so `Each event alone …` never
            // encoded. Cap-then-dedup keeps ≤ `cap` DISTINCT concepts and never seeds past the ranker's
            // top-`cap` raw positions. (The 2026-07-11 measurement that first added the pre-cap dedup
            // found it left encoded UNCHANGED — the freed slot was "immediately refilled"; that refill
            // is this bug.) INERT until a pair is actually merged: distinct-lexicon senses carry
            // different class IRIs, so `(cat, sem)` differs and nothing collapses. O(n²) over one
            // lemma's (already cap-bounded) senses.
            // **The cap is a fragile equilibrium, and removing INVALID entries exposes that** (measured
            // 2026-07-26). The UMLS importer began pruning inflected duplicate forms
            // (`convert::is_inflection_of_sibling`) — 52 505 entries a lemma-keyed lexicon should never
            // have held. Cap-only, that moved exactly three units on the reference page: the germline
            // unit 73 -> 41 (the intended effect — a plural surface stopped carrying a singular
            // reading), and TWO units UP, 18 -> 52 and 40 -> 60, for a net +22.
            //
            // The rise is displacement, not new grammar. `entries_for("models")` and
            // `entries_for("model")` draw on different concepts, and items compete for this cap per
            // candidate and for `CELL_BEAM` per cell; removing entries frees room that refills with a
            // different category — the added skeletons pair a 2-place predicate with a ONE-place one,
            // i.e. the singular's VERB reading (`model`, `arrest`) surfacing where the plural form's
            // noun-only entries used to sit. Those two units were encoding correctly only because the
            // invalid entries happened to occupy the slots.
            //
            // So it is not an argument against pruning invalid forms — it is a second witness that the
            // cap truncates by a criterion (emission order) that carries no linguistic content. See the
            // long note above for two fixes already measured and rejected; this is the same shape.
            dedup_same_concept(&mut entries);
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
    /// Drop a preposition's VP-adjunct reading at position `p` when the ADJECTIVE head immediately to
    /// its left (`[p-1]`) subcategorizes for exactly that preposition (`X / cat_pp_arg(prep_P)`, X a
    /// `cat_measure` or `S[adj]\NP`) and the surface at `p` offers the matching `cat_pp_arg(prep_P)`
    /// argument marker. The governed PP is then the head's argument, never a free adjunct — so the
    /// `And`-introducing entry (`is_vp_adjunct_prep`) is removed. Adjective-scoped by design (see the
    /// call site); verbs are excluded because they can genuinely both govern and adjunct one preposition.
    /// Single-token head/preposition adjacency (the page's shape); an intervening adverb slips it.
    fn suppress_governed_adjunct(&self, chart: &mut Chart, n: usize) {
        for p in 1..n {
            // The preposition governed by an adjective head immediately to the left.
            let governed_prep: Option<Exp> = chart[p - 1][p - 1].iter().find_map(|it| {
                let (_m, res, arg) = slash_parts(it.cat(), "fwd")?;
                let [prep] = is_ctor(arg, "cat_pp_arg")? else {
                    return None;
                };
                (is_ctor(res, "cat_measure").is_some() || is_adjective_cat(res))
                    .then(|| prep.clone())
            });
            let Some(prep) = governed_prep else {
                continue;
            };
            // Disambiguate only when this surface offers BOTH the matching argument marker and a
            // competing VP-adjunct reading — otherwise there is nothing spurious to drop.
            let has_matching_arg = chart[p][p].iter().any(|it| {
                matches!(slash_parts(it.cat(), "fwd"),
                    Some((_m, a, _)) if matches!(is_ctor(a, "cat_pp_arg"), Some([pp]) if *pp == prep))
            });
            let has_adjunct = chart[p][p].iter().any(|it| is_vp_adjunct_prep(it.cat()));
            if has_matching_arg && has_adjunct {
                chart[p][p].retain(|it| !is_vp_adjunct_prep(it.cat()));
            }
        }
    }

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

        // RNR head distribution (D63, `docs/notes/d63-rnr-head-distribution.md`): a coordinated
        // pre-nominal modifier list sharing a head noun ("colon, gastric and ovarian cancers") seeds the
        // group-of-lexicalized-kinds reading its shared-head coordination cannot reach (the head sits
        // once at the end — no adjacency to the multiword). NOT bounded by `span_limit` (a coordination
        // is longer than any lexeme); `distribute_head` bails cheaply on a non-coordination. The seeded
        // `cat_group` protects its span (`multiword_spans`), so the composition it supersedes is pruned
        // with the multiword widen fallback — coverage-safe (grammar-gap 0).
        let mut dists: Vec<(usize, usize, Vec<Item>)> = Vec::new();
        for a in 0..n {
            for b in (a + 1)..n {
                let d = self.distribute_head(&tokens[a..=b], lemmatizer, scope, cap, ranks);
                if !d.is_empty() {
                    dists.push((a, b, d));
                }
            }
        }
        for (a, b, d) in dists {
            chart[a][b].extend(d);
        }

        // Argument/adjunct suppression (D63 §8.13): a governed-ADJECTIVE head immediately followed by
        // its subcategorized preposition takes that PP as its ARGUMENT, so the preposition's competing
        // VP-adjunct entry (the `And`-introducing `((S\NP)\(S\NP))/NP`, closed-class.esl `prep_*_sem`)
        // is spurious there and is dropped, leaving only the transparent `cat_pp_arg` marker. Scoped to
        // ADJECTIVE heads (`cat_measure` / `S[adj]\NP` over `cat_pp_arg(prep)`): a governed adjective has
        // no competing adjunct use of its preposition ("dependent on X" is always the complement),
        // unlike a verb ("operate on Monday" — the temporal adjunct is real). Coverage-safe: lifted at
        // the FINAL widen rung (mirroring multiword-preference, line below), so the adjunct entry is
        // re-admitted if suppressing it would gap the sentence — `grammar-gap 0` holds by construction.
        if !matches!(cap, Some(c) if c >= SENSE_CAP_WIDEN_MAX) {
            self.suppress_governed_adjunct(&mut chart, n);
        }

        // Forward bounded type-raising `T` (D63 §8.9 Slice 6-T) at the LEAF cells: a name `NP` lifts
        // to `S/(S\NP)` so it can forward-compose into a relative clause's object-extraction body.
        // ENF (`TypeRaised` provenance) keeps these inert outside extraction. Composed cells are
        // raised in the CKY loop.
        let mut beam_drops = 0usize;
        for (i, row) in chart.iter_mut().enumerate() {
            let raised = self.grammar.raise_nps(&row[i]);
            row[i].extend(raised);
            // Pre-nominal modifier lift at the LEAF cells (D63 coordinated-modifier category): every
            // modifier-eligible leaf item (lexical adjective, derived/`-based` adjective) → `cat_mod`,
            // so it can coordinate before meeting the head noun. This is the universal leaf point —
            // after ALL seeding paths — mirroring the leaf `raise_nps` above; composed cells lift via
            // the `ModLift` unary shift.
            let mut mods: Vec<Item> = row[i]
                .iter()
                .flat_map(super::super::rules::combinators::mod_lifts)
                .collect();
            // Attributive past-participle lift, GATED: only when this surface has NO lexical adjective
            // (else the WordNet adjective already covers the attributive use, and the rule's
            // reduced-passive reading would just double-seed — "reduced"/"increased"). Where there is
            // no adjective ("predicted"), the rule is the only source of the attributive reading.
            if !row[i].iter().any(|it| is_adjective_cat(it.cat())) {
                let parts: Vec<Item> = row[i]
                    .iter()
                    .flat_map(super::super::rules::combinators::participial_lifts)
                    .collect();
                mods.extend(parts);
            }
            // POST-nominal oblique participial lift — UNGATED, unlike the pre-nominal one above. Its
            // gate exists because a lexical adjective already covers the ATTRIBUTIVE use; there is no
            // adjective entry that covers "compared to MSS cell lines", so there is nothing to
            // double-seed against. Must run at the LEAF: the lift is on the still-unsaturated functor
            // (that is where oblique and transitive are still distinguishable), and `compared` is one
            // token — the `UnaryShift` table only runs over spans of length >= 2.
            mods.extend(
                row[i]
                    .iter()
                    .flat_map(super::super::rules::combinators::oblique_participial_lifts)
                    .collect::<Vec<_>>(),
            );
            row[i].extend(mods);
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
            // Targeted LEAF dump — the twin of the composed-cell dump in `chart::unpacked`, which
            // only fires inside `for len in 2..=n` and so can never show a single-token cell. Leaf
            // categories are what decide whether a seed-time lift has anything to fire on, so they
            // need their own view. Set `EIGENIUS_DUMP_CELL=i..i`.
            if let Ok(want) = std::env::var("EIGENIUS_DUMP_CELL") {
                if want == format!("{i}..{i}") {
                    eprintln!(
                        "  ===== DUMP leaf[{i}..{i}] tok={:?} ({} items, sample 20) =====",
                        tokens[i],
                        row[i].len()
                    );
                    for it in row[i].iter().take(20) {
                        eprintln!(
                            "    [{:?} cost={:?}] {}",
                            it.prov(),
                            it.cost(),
                            crate::dcg::pretty::pretty_term(it.cat())
                        );
                    }
                }
            }
        }
        (chart, beam_drops)
    }

    /// **RNR head distribution** (`docs/notes/d63-rnr-head-distribution.md`) — a SEED-time rule (it
    /// needs the lexicon, which the grammar and chart deliberately cannot reach). For a span `[i, j]`
    /// whose prefix `tokens[i..j]` is a pre-nominal modifier COORDINATION and `tokens[j]` is the shared
    /// head, re-look-up each "conjunct + head" compound IN ISOLATION — the isolated surface resolves the
    /// lexicalized concept cleanly, where the shared-head coordination cannot (a multiword lexeme needs
    /// adjacency, and the head sits once at the end) — bare-NP-shift each to its kind, and fold them into
    /// a `cat_group` the distributive rules predicate over. So "colon, gastric and ovarian cancers"
    /// reaches `[kind_of(colon-cancer), …]` rather than a generic cancer with a disjunctive modifier.
    ///
    /// First cut (D63 §8 M-RNR-2): fires only when EVERY conjunct lexicalizes; a mixed list (a conjunct
    /// that only composes, e.g. "ovarian cancer") yields nothing here, and the `cat_mod`/`Or` reading
    /// stands — coverage is never reduced.
    fn distribute_head(
        &self,
        span: &[String],
        lemmatizer: &dyn Lemmatizer,
        scope: Option<&[Iri]>,
        cap: Option<usize>,
        ranks: Option<&BTreeMap<String, u32>>,
    ) -> Vec<Item> {
        // `span` is `tokens[i..=j]`: the modifier coordination followed by its shared head (last token).
        let Some((head, mods)) = span.split_last() else {
            return Vec::new();
        };
        let head = head.as_str();
        let Some(conjuncts) = split_coord_conjuncts(mods, |t| {
            self.grammar.reserved.coord_connective(t).is_some()
        }) else {
            return Vec::new();
        };
        // The shared head's class `H` types the group — each distributed compound `Kᵢ ⊆ H`, so
        // `group_member_fits` sees the type a predicate accepts. NOT `common_super(Kᵢ)`, which for
        // UMLS-CUI compounds is the `Entity` top and fits no concrete slot (Wall 1a).
        let Some(h) = self
            .lookup_span(head, lemmatizer, scope, cap, ranks)
            .iter()
            .find_map(|it| match it.cat() {
                Exp::InductiveCtor(_, name, args) if name == "cat_n" && args.len() == 2 => {
                    Some(args[0].clone())
                }
                _ => None,
            })
        else {
            return Vec::new();
        };
        // Per conjunct: the kind NPs of "conjunct + head", looked up IN ISOLATION and bare-NP-shifted.
        // Bail unless EVERY conjunct yields ≥1 lexicalized concept (first-cut all-lexicalized gate).
        let mut per_conjunct: Vec<Vec<Item>> = Vec::with_capacity(conjuncts.len());
        for c in &conjuncts {
            let surface = format!("{} {head}", c.join(" "));
            let raw = self.lookup_span(&surface, lemmatizer, scope, cap, ranks);
            // The bare-kind NP of each looked-up common noun: `cat_np(C, num)` with sem `kind_of(C)`,
            // built directly — `bare_nominal_shifts` yields the RAISED subject/object forms, not the
            // plain `cat_np` that `coordinate_np` folds into a group.
            let kinds: Vec<Item> = raw
                .iter()
                .filter_map(|it| match it.cat() {
                    Exp::InductiveCtor(decl, name, args) if name == "cat_n" && args.len() == 2 => {
                        Some(Item::with_cost(
                            // Typed by the head class `H`, sem is the SPECIFIC kind `kind_of(Kᵢ)`.
                            Exp::InductiveCtor(
                                decl.clone(),
                                "cat_np".into(),
                                vec![h.clone(), args[1].clone()],
                            ),
                            kind_of(args[0].clone()),
                            it.cost(),
                        ))
                    }
                    _ => None,
                })
                // Top-ranked sense only (raw is already sense-ranked by cap/reranker): the union is ONE
                // structural reading, so cross-producing N conjuncts × their senses is an Nᵗʰ-power seed
                // explosion (4⁴ on the 4-way cancer list → 192 groups → forest blow-up, Wall 2). Sense
                // choice is the cap/reranker's job downstream, not distribute_head's.
                .take(1)
                .collect();
            if kinds.is_empty() {
                return Vec::new();
            }
            per_conjunct.push(kinds);
        }
        // Connective: `or` anywhere → union; else `and` (a comma list finalizes to conjunction).
        let op = if mods
            .iter()
            .any(|t| self.grammar.reserved.coord_connective(t) == Some("urn:eigenius:logic:Or"))
        {
            "urn:eigenius:logic:Or"
        } else {
            "urn:eigenius:logic:And"
        };
        // One representative cost per conjunct, summed — the distributed compound's seed cost.
        let mut cost = per_conjunct[0][0].cost();
        for c in &per_conjunct[1..] {
            cost = cost.saturating_add(c[0].cost());
        }
        // Fold `coordinate_np` left over the one kind per conjunct into the `cat_group` union.
        let mut groups: Vec<(Exp, Exp)> = per_conjunct[0]
            .iter()
            .map(|it| (it.cat().clone(), it.sem().clone()))
            .collect();
        for conj in &per_conjunct[1..] {
            let mut next = Vec::new();
            for (gc, gs) in &groups {
                for it in conj {
                    if let Some(pair) =
                        coordinate_np(op, gc, gs, it.cat(), it.sem(), &self.grammar.layer)
                    {
                        next.push(pair);
                    }
                }
            }
            groups = next;
        }
        groups
            .into_iter()
            .map(|(cat, sem)| Item::with_cost(cat, sem, cost))
            .collect()
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
/// Whether a category takes a **plural subject on a FINITE clause** — it contains the subject slot
/// `bwd(cat_s(_, fin), cat_np(_, pl))` at any depth, so it matches an intransitive VP as well as a
/// transitive verb's `fwd(VP, obj)` and any further-curried frame. A BASE-form VP is
/// `bwd(cat_s(_, bse), cat_np(_, num_any))` and never matches, which is what keeps the check off
/// infinitives and participles.
fn has_plural_finite_subject(cat: &Exp) -> bool {
    if let Some((_m, res, arg)) = slash_parts(cat, "bwd") {
        let finite = matches!(is_ctor(res, "cat_s"),
            Some([_, Exp::InductiveCtor(_, f, _)]) if f == "fin");
        let plural_subj = matches!(is_ctor(arg, "cat_np"),
            Some([_, Exp::InductiveCtor(_, n, _)]) if n == "pl");
        if finite && plural_subj {
            return true;
        }
    }
    match cat {
        Exp::InductiveCtor(_, _, args) => args.iter().any(has_plural_finite_subject),
        _ => false,
    }
}

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

/// **RNR head distribution** (`docs/notes/d63-rnr-head-distribution.md` §4) — split a candidate
/// pre-nominal modifier-coordination slice on its connectives into the conjunct token-runs, so the
/// caller can re-look-up each "conjunct + head" compound in isolation. `is_conn(t)` holds for a
/// coordination connective (comma / `and` / `or`). Returns the conjunct surfaces ("colon",
/// "microsatellite-stable"); `None` unless there are ≥2 non-empty conjuncts separated ONLY by
/// connectives — a leading / trailing / doubled connective, or a single conjunct, is not a coordination.
fn split_coord_conjuncts(
    tokens: &[String],
    is_conn: impl Fn(&str) -> bool,
) -> Option<Vec<Vec<String>>> {
    let mut conjuncts: Vec<Vec<String>> = vec![Vec::new()];
    for t in tokens {
        if is_conn(t) {
            if conjuncts.last().unwrap().is_empty() {
                return None; // a leading or doubled connective — malformed
            }
            conjuncts.push(Vec::new());
        } else {
            conjuncts.last_mut().unwrap().push(t.clone());
        }
    }
    if conjuncts.len() < 2 || conjuncts.last().unwrap().is_empty() {
        return None; // a single conjunct, or a trailing connective
    }
    Some(conjuncts)
}

#[cfg(test)]
mod rnr_tests {
    use super::split_coord_conjuncts;

    fn toks(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }
    // The parser tokenises the comma as its own token; connectives are `,` / `and` / `or`.
    fn is_conn(t: &str) -> bool {
        matches!(t, "," | "and" | "or")
    }

    #[test]
    fn splits_a_four_way_comma_list() {
        assert_eq!(
            split_coord_conjuncts(&toks("colon , gastric , endometrial and ovarian"), is_conn),
            Some(vec![
                toks("colon"),
                toks("gastric"),
                toks("endometrial"),
                toks("ovarian")
            ])
        );
    }

    #[test]
    fn or_list_keeps_multitoken_conjuncts() {
        assert_eq!(
            split_coord_conjuncts(&toks("insertion or deletion"), is_conn),
            Some(vec![toks("insertion"), toks("deletion")])
        );
        assert_eq!(
            split_coord_conjuncts(&toks("double stranded or single stranded"), is_conn),
            Some(vec![toks("double stranded"), toks("single stranded")])
        );
    }

    #[test]
    fn rejects_non_coordinations() {
        assert_eq!(split_coord_conjuncts(&toks("colon"), is_conn), None);
        assert_eq!(split_coord_conjuncts(&toks("colon and"), is_conn), None);
        assert_eq!(split_coord_conjuncts(&toks("and colon"), is_conn), None);
        assert_eq!(
            split_coord_conjuncts(&toks("colon and and gastric"), is_conn),
            None
        );
    }
}
