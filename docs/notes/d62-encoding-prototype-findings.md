# D62 encoding pipeline — prototype findings (empirical)

*Working note. Results from the core-algorithm prototype
(`crates/eigenius-wordnet/tests/encoding_prototype.rs`) run against the real DCG parser,
real WordNet, and the emitted UMLS / NCBI-gene lexica. These are measured outcomes
(Observed/Derived), captured to drive the build order. See D62 for the pipeline spec.*

## The prototype

A test-only driver (no RPC, no ontology, no institution, no LLM) exercising the control-flow
heart: `segment(text) → parse_scoped → classify → report`, with the LLM-proposer stages
stubbed. Classification is the four-way outcome taxonomy (D62 §4):

- **Encoded** — 1 felicitous parse (gates to `Prop`).
- **Ambiguous** — >1 parse (stub: keep rank-0; LLM context-select is S4).
- **MissingLexeme** — empty parse + ≥1 token absent from the lexicon (route S5a).
- **GrammarGap** — empty parse + all tokens known (route S5b).

The missing↔grammar diagnosis uses `LexicalIndex::has_token` (added: surface +
lemmatizer-candidate entry presence). The control flow holds on real data — this taxonomy
is the right spine.

## Finding 1 — ambiguity is the dominant problem, even for trivial prose

"A dog sees a bird" over a seeded real-WordNet slice produced **256 readings** (hit
`DEFAULT_FOREST_CAP`) — pure WordNet sense ambiguity; the rank-0 (most-frequent-sense)
reading gates to a real `Prop`. Implication: **S4 must be *narrow-then-select*** (scope /
domain-lexicon narrowing first, then select among survivors), not select-from-256. Lexicon
**scope is the primary disambiguation lever**, not just an efficiency knob.

## Finding 2 — feeding real WRN-paper prose: the gap is basic English, not domain vocab

Fed a cleaned first page of the WRN *Nature* paper
(`references/publications/WRN-Helicase-Nature-OCR/first-page-cleaned.txt`) through the
prototype (WordNet seeded from the page's own words, so a miss is genuine OOV):
**0 of 47 units encoded — all 47 MissingLexeme**, 141 distinct OOV tokens, in three
categories:

1. **Closed-class function words (the dominant blocker).** The closed-class lexicon covers
   only ~33 forms (`a, all, every, no, some, each, is/are/was/were, has/have/had,
   do/does/did, can/could/may/might/must, of/in/on/by/with/for/from, that/which/what, not,
   than`). **Missing — and flagging every sentence:** `the`, `and`/`or` (no coordination),
   `to`, `we`, `this`, `their`, `its`, `would` (modal), `also`, `however`, `although`,
   `because`. These are not in WordNet/UMLS/NCBI (content lexica) — they need the
   closed-class lexicon + grammar (D63).
2. **Tokenization (the S0 gap).** Em-dashes `—`/`–` not split (`not—can`, `regions—which`),
   parens/slashes (`poly(adp-ribose`, `and/or`), hyphenated compounds as single tokens
   (`double-stranded`, `large-scale`, `crispr–cas9-mediated`), stats + figure-refs as
   lexemes (`10−13`, `1a`, `2b`, `n`, `p`, `q`). And the naive `.`/`!`/`?` segmenter
   **over-split 4 paragraphs into 47 units** (`0.56`, `Fig. 1a`, abbreviations).
3. **Genuine domain OOV** (`wrn, msi, mss, mmr, helicase, microsatellite, recq,
   crispr–cas9, parp-1, kras, braf, msh2/6, pms2, mlh1`) — real, but masked by (1) and (2).

### Re-measurement (after S0 segmentation + the closed-class/carrier build-out)

Same page, after: the S0 segmenter (47 → 26 units), the closed-class additions (determiners
`the/this/that/an`, modals incl. `would`, `if`, `but`, coordination signal `and`/`or`,
pronouns `it/they/its/their/we`), and **lemmatizing the WordNet seed** (the seed matched raw
plural/inflected *surfaces* against the lemma-keyed dictionary, spuriously inflating OOV —
the same surface-vs-lemma artifact as the plural-GQ / Identity-lemmatizer cases):

- **26 units → 9 parseable (≤22 tokens), 17 over-length skipped (parsing-scale bound).**
- **Of the 9: 0 encoded, 9 missing-lexeme.**
- **OOV: 99 → 24** distinct tokens (lemmatized seeding removed ~75 spuriously-OOV content
  words like `models/genes/biomarkers/datasets/lineages`).

The **24 genuine OOV** fall entirely into already-planned buckets: domain proper nouns /
acronyms (~16: `wrn, mlh1, msh2, msh6, pms2, recq, parp-1, dna, mmr, msi, helicase(s),
germline, hypermethylation, double-stranded, pcr-based, adp-ribose, poly`) → the domain-
lexicon injection (UMLS/NCBI); `-ly` adverbs (`commonly, preferentially, selectively,
typically`) → derivational `-ly` morphology (P3); the plural demonstrative `these` → trivial
closed-class add (sg `this`/`that` done); and `after`.

**Revised blocker ranking (this paper):** (1) **parsing scale** — long, dense sentences
(17/26 > 22 tokens) OOM the chart with a full polysemous slice — the dominant practical gate;
(2) **grammar coverage** of long constructions; (3) **residual vocab** (24 tokens, all
planned). Vocabulary is *not* the primary blocker — confirming Finding 3 — and the prior
"missing-lexeme" counts were measurement-inflated. Measured by `prototype_over_wrn_first_page`
(`crates/eigenius-wordnet/tests/encoding_prototype.rs`).

### Full-page measurement (after the sense-cap unblock — GH #97)

With `with_sense_cap(2)` (adaptive supertagging), the parsing-scale OOM lifts and the parser runs
over the **whole page**: **25 of 26 units parse (≤60 tokens), 1 over-length skipped**. Outcome:
**0 encoded, 25 missing-lexeme, 0 grammar-gap.** OOV = 71 distinct tokens.

- **OOV-per-unit:** min 1, max 21, **mean 6.6**; only **1** unit is one-OOV-away. So units are
  *far* from parsing on vocab alone — vocabulary saturates the gate.
- **OOV by fix-bucket:** **domain-lexicon 40 (56%)** · **connectives/function-words 17 (24%)** ·
  **-ly adverbs 10 (14%)** · **stat-symbol leaks 4 (6%, single letters `e/n/p/q` past S0)**.

**What this tells us:**
1. **Vocabulary — specifically the domain lexicon — is the encode-gate**, not parsing scale (now
   unblocked) and not the closed class. Domain-lexicon injection (UMLS/NCBI) clears the majority
   (56%) of OOV; closed-class connectives/quantifiers (`because/although/however/such/these/those/
   to/most/several/both/…`) clear 24%; `-ly` derivation 14%; an S0 single-letter route 6%.
2. **Grammar reach is not yet measurable** — `grammar-gap = 0` only because vocabulary fails first
   everywhere (mean 6.6 OOV/unit). Grammar gaps will surface only after the vocab buckets are filled.
3. The parsing-scale work (sense cap; the LLM reranker; widen-on-failure) is what made this
   measurable, but it does **not** produce encodes — that waits on the **domain-lexicon injection**,
   now quantified as the critical path to actual WRN encodes.

### DB-backed full-lexicon measurement — the *true* (d) (WordNet + UMLS, served store)

The prior full-page run seeded a WordNet *slice* from the page's own words. This run parses over the
**actual served store** — the full WordNet + UMLS chain (7.6M resources, 51-layer chain) in a
snapshot of the docker-volume RocksDB DB — opened in-process via the persistent backend and the
**lazy** `LexicalIndex` (on-demand `lexicon:form` value-index probes; the eager scan OOMs). Driver:
`crates/eigenius-wordnet/tests/db_backed_encoding.rs` (`wrn_first_page_over_full_lexicon`,
`#[ignore]`d; `EIGENIUS_DB_SNAPSHOT` points at the store). Sense cap = 2.

**Bootstrap caveat (load-bearing for reading the numbers).** The snapshot's chain is rooted at the
bootstrap it was seeded with (commit `ff7f6cc`), so resuming it requires the code's
`logic`/`closed-class` to match (else `ManifestDrift`, fail-closed). That seeded `closed-class`
**predates** the determiner (`the/this/that/an`), pronoun (`we/their/those`), and modal (`would`)
additions — so those words are reported OOV here as an **artifact of the resumed bootstrap**, not a
real gap in current code (verified by direct probe: `has_token` = false for `the/we/their/would`).

**Result — 26 units → 0 encoded, 24 missing-lexeme, 1 grammar-gap, 1 scale-bound.** OOV = **34
distinct** tokens (was 71 on the slice).

- **OOV-per-unit:** min 1, max 7, **mean 2.6** (was 6.6) — the full lexicon **roughly halved**
  per-unit OOV, and **7 units are now one OOV away** from parsing (was 1).
- **OOV by fix-bucket (raw):** domain-lexicon 17 · connectives/function-words 7 · -ly adverbs 10 ·
  stat leaks 0. **Adjusted for the bootstrap caveat:** ~4 of the "domain 17" are closed-class words
  the current bootstrap already has (`the/we/their/would`) → genuine domain residual ≈ **13**.
- **Two newly-isolated real gaps** (both confirmed OOV by direct probe over the full store):
  1. **-ly adverbs (10)** — `commonly/typically/selectively/preferentially/…` are OOV *even over
     full WordNet* (`has_token("commonly") = false`). So this is **not** "just inject more lexicon"
     — WordNet adverbs aren't reachable as loaded; this needs the `-ly` derivational route (P3) or
     an adverb-import fix.
  2. **Hyphenated compounds + true domain terms** — `cas9-mediated, double-stranded, genome-scale,
     next-generation, pcr-based, msi-predominant, hypermutable, recq, wilcoxon, cas9` — a mix of S0
     compound-tokenization gaps and genuine domain OOV (`recq`, `wilcoxon`).

**What this revises vs the slice finding:**
1. **Full domain coverage does materially help** — per-unit OOV halves and most units come within
   1–2 tokens of parsing. The slice run's "domain-lexicon 56%" overstated the *residual* domain gap
   (the slice lacked the page's own multiword/related terms that the full UMLS supplies).
2. **The residual encode-gate is now a long tail, not one bucket:** ~13 domain/compound terms +
   10 -ly adverbs + (in current code) the already-fixed closed-class words. Still **0 encoded** only
   because every dense sentence carries ≥1 of these (a hyphenated compound or an -ly adverb).
3. **Parsing scale was a hard wall at full-lexicon density — now cleared by Lever B.** A
   *fully-known* 17-token sentence (`These findings show that WRN is …`) **OOM'd the chart even at
   cap=2** (clausal embedding × coordination × full-lexicon polysemy; the per-lemma sense cap is not
   enough). **Lever B (per-cell beam, GH #97)** — `LexicalIndex::with_cell_beam(n)`, capping every
   non-top CKY cell to its `n` lowest-`Cost` items (orthogonal to Lever A: A caps senses *per lemma
   at the leaf*, B caps derivations *per composed cell*) — fixes it. Unit-tested
   (`cell_beam_bounds_a_cell_and_is_a_noop_when_generous`); harness wires `CELL_BEAM=64`.

### Fresh-DB full-lexicon measurement — Lever B validated (2026-06-28)

Reran over a **freshly reseeded** store (current HEAD bootstrap, WordNet `--all` + UMLS WRN-relevant
TUI subset; `scripts/reseed-lexicon-db.sh`), so no bootstrap drift — the prior run's `the/we/their/
would` OOV artifacts are gone (`has_token` = true). **26 units → 0 encoded, 24 missing-lexeme, 2
grammar-gap, 0 scale-bound.** OOV = 51 distinct, mean 3.2/unit.

- **Lever B holds at scale:** the 17-token fully-known unit that previously SIGKILL'd now parses in
  **0.2 s**, the beam dropping up to ~830 items per cell; **no OOM, nothing scale-bound** (`beam=64`
  was sufficient, no tuning needed). This was the binding constraint for fully-known sentences.
- **Grammar reach is now measurable:** **2 grammar-gaps** (units 6, 13 — fully known, parsed, no
  felicitous reading) surfaced *because* the beam let those known-vocab units parse to completion.
  The "grammar gaps appear once parsing scale is unblocked" prediction is confirmed.
- **OOV buckets:** domain-lexicon 24 · connectives/function-words 15 · -ly adverbs 12 · stat 0.
  The **-ly adverbs remain OOV even over full WordNet** (`has_token("commonly") = false`) — a real
  coverage gap (derivational `-ly`, P3), not "inject more lexicon". The **domain-lexicon 24** is
  higher than the prior run's because this store used the UMLS **subset** (8 TUIs); terms like
  `microsatellite/biomarker/germline/crispr` fall under TUIs outside the subset — rerun with
  `reseed-lexicon-db.sh --umls-all` for full domain coverage.
- A real importer bug was found + worked around en route: the UMLS **subset** path was loaded with a
  **stale `umls-import` binary** that emitted `subclass_of` to semantic-type classes the base layer
  didn't declare (base 30 vs concepts referencing 125) → fail-closed chunk rejection. A fresh
  release build is consistent (base == referenced); the reseed script force-builds release binaries
  and adds a pre-load dangling-STY guard.

## Finding 3 — the three lexica cover the content vocabulary (vocabulary is not the blocker)

Measured the page's vocabulary against the **real emitted lexica forms** on disk (WordNet
325k, UMLS 4.75M, NCBI-gene 268k forms) — form-coverage, since loading the full lexica into
the in-process parse index is what OOM'd the kernel.

Of **322 distinct page tokens**, single-word-entry coverage is **283 (87%)** —
WordNet 226 + UMLS 57 (+ NCBI 0, because the genes are already UMLS concepts: redundant-but-
confirming). The **39 "uncovered" are not content vocabulary**:

- **closed-class function words (~15):** `the, we, that, which, their, its, those, such,
  than, would, also, however, although, because, yet` — need closed-class entries;
- **inflected surfaces whose lemma IS covered (~18):** `regions, deficiencies, lineages,
  showed, were, commonly, selectively, …` — resolve at parse time via the Morphy lemmatizer
  (the exact-surface coverage script didn't apply it), so the true residual is smaller;
- **tokenization artifacts / domain names (~3):** `mlh` (from `MLH1` split), `recq`,
  `wilcoxon`.

**Genuinely-novel content words: ~3** (`correspondingly`, `counterparts`, `favourably` —
ordinary English absent as lexica forms). So the three lexica are **ready**; the residual is
exactly closed-class + morphology + tokenization.

## Reprioritized build order (evidence-based)

1. **S0 tokenization + segmentation** — em-dash/paren/slash handling, hyphenated-compound
   policy, abbreviation/decimal-aware sentence split, route stats/figure-refs out. (The §7.1
   non-prose routing we deferred is needed earlier than planned, at least for stats/refs.)
2. **Closed-class lexicon + grammar coverage of ordinary English** — `the`, coordination
   (`and`/`or`), `to`+infinitive, more pronouns/demonstratives, modals, common subordinators.
   This is D63 extension, and **the WRN feed is the gap-harvest that drives it** (D62 §9).
3. **Then** bring WordNet+UMLS+NCBI into parse scope (value-index-backed, no full
   materialization) — resolves the content words **and** collapses the 256-way ambiguity via
   scope (Finding 1).
4. **S4 disambiguation** as narrow-then-select; **S5a lexical recovery**; reference/D64.

Disambiguation and domain-lexicon loading both move *down* the list: they don't pay off
until sentences parse structurally.

## Infrastructure follow-ons surfaced

- **Object-value EigenQL query OOMs at scale.** `MATCH ?e { lexicon:form: "x" }` full-scans
  the 7.6M chain (no value-index/object pushdown) → kernel SIGKILL. The open audit item
  (*property-predicate pushdown for untyped patterns*), now witnessed as an OOM. The parse
  path already uses the D65 value index for form lookup; EigenGL object-value matching must
  too (or expose `has_token` over RPC).
- **NCBI-gene can't load whole.** `ncbi-gene.esl` is 165 MB single-file (> 128 MiB gRPC
  limit); `ncbi-gene-import` lacks the `--out-dir` partitioned emit that wordnet/umls got.
  Mirror it.
- **`ParseSentence` must report the open forest + missed tokens** (D62 §11.5) — the missed-
  token signal the prototype gets in-process via `has_token`.
