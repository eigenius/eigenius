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
