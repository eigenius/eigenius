# Parsing scale & pruning — controlling the DCG chart explosion (D63 / D62)

**Status:** Design note (grounded). Tracked by [#97](https://github.com/eigenius/eigenius/issues/97).
Motivated by the WRN-encoding measurement
(`docs/notes/d62-encoding-prototype-findings.md`): 17/26 sentences OOM the parser with a full
WordNet slice. This note diagnoses the scale wall and recommends two complementary, literature-
grounded pruning levers — **adaptive supertagging** (lexical) and **exact mid-chart felicity
pruning** (combinatorial). References are verified (ACL Anthology / DOI) and added to
`docs/references/eigenius_related_work.bib`.

## 1. The wall, precisely

The parser is CKY over the seeded chart (`kernel/src/dcg/lookup.rs`). Diagnosed:

- **O(n²) cells, each cell built from *all* shorter sub-spans.** Building span length `L`
  combines a length-`a` left with a length-`(L-a)` right for every `a = 1…L-1` — so it reads
  every shorter layer, not just the last two. No sliding-window reduction; the chart is
  irreducibly O(n²) cells.
- **The blow-up is the resident *item* population, not the container.** Cells are nearly free;
  the memory is the `Item`s, and their count explodes with **WordNet sense polysemy** (one seed
  Item per sense × POS, multiplied through CKY's items²-per-split combination), **un-pruned until
  the final full-span cell** (only there do `reduced_felicitous` + the `DEFAULT_FOREST_CAP` apply).
- **Items own their children; no subtree sharing.** `Exp` carries children by `Box`/`Vec`
  (sole ownership; only the `Arc<InductiveDecl>` *schema* is shared), and `apply` builds a parent
  via `App(left.sem.clone(), right.sem.clone())` — a deep clone. A sub-derivation used by *k*
  parents is duplicated *k* times, compounding up the chart.
- **Done:** the per-split `chart[i][k].clone()` was replaced with borrows (zero-cost) — a real
  time/churn win, but it does **not** lift the OOM (confirmed empirically: still SIGKILLs with no
  length cap), because the OOM is *resident* items, not the *transient* per-split copies.

So the levers are: **fewer items per cell**, **cheaper items**, and (limited) **release after
last use** — not chart-shape windowing.

## 2. Pruning approaches in the literature (grounded)

- **Beam / histogram / threshold pruning** — keep top-`k` per cell, or within a score factor of
  the best. Ubiquitous, *inexact* (risks search errors). [charniak-2000-maxent-parser]
- **Best-first with a figure-of-merit** — agenda ordered by inside×outside estimate, stop early.
  Inexact. [caraballo-charniak-1998-fom]
- **A\* parsing** — best-first but **exact**: an *admissible* outside heuristic guarantees the
  Viterbi parse with no search errors, touching <3% of edges. [klein-manning-2003-astar]
- **Coarse-to-fine** — parse with a coarse grammar to prune the chart for the finer pass.
  [charniak-etal-2006-coarse-to-fine]
- **Supertagging / adaptive lexical pruning** — for lexicalized/categorial grammars (CCG), the
  blow-up is *lexical-category ambiguity*; a tagger picks a small per-word category set *before*
  parsing ("almost parsing"), with an **adaptive β**: start tight, widen on parse failure.
  [bangalore-joshi-1999-supertagging; clark-curran-2004-supertagging; clark-curran-2007-ccg;
  xu-auli-clark-2015-rnn-supertag]
- **Unification / type-failure filtering + local ambiguity packing** — drop type-incompatible
  combinations early; pack equivalent sub-analyses. *Exact* (hard constraint).
  [oepen-carroll-2000-ambiguity-packing]
- **Packed / shared parse forests (SPPF)** — share sub-derivations so an exponential ambiguity
  set is stored compactly (memory, not search). [tomita-1987-glr; billot-lang-1989-shared-forests]

## 3. Why our setting is favorable — two facts

1. **Our blow-up is lexical** (WordNet sense polysemy) — structurally identical to CCG's
   lexical-category ambiguity. So **supertagging-style adaptive lexical pruning is the textbook
   fix**, and we are already set up for it: D65's `sense_rank` *is* a supertag prior, and neural
   supertaggers ([xu-auli-clark-2015-rnn-supertag]) show a learned per-word prior tightens the
   beam safely.
2. **We have an EXACT oracle** (the type checker). Most NLP pruning is inexact (probabilistic
   beams risk the right parse); type-incompatible combinations can be dropped with **zero
   search-error risk** — the soundness A\* gets from an admissible heuristic
   ([klein-manning-2003-astar]) and unification parsers get from type-failure filtering
   ([oepen-carroll-2000-ambiguity-packing]), we get from the kernel felicity check. We currently
   apply it *only* at the full span.

## 4. Recommendation — two complementary levers

**Lever A — adaptive supertagging (cut the seed count).** Seed only the top-`N` senses per token
by `sense_rank`/scope (D65); **widen on parse failure** (the Clark–Curran adaptive-β policy
[clark-curran-2004-supertagging]). Inexact, but the widen-on-failure loop recovers completeness,
and it attacks the explosion *at the seed*, before any combination.

  *Contextual reranking (the strong form of Lever A).* The supertag prior need not be the static
  `sense_rank` (global WordNet frequency) — it can be an **LLM contextual sense reranker**: given a
  content word *in its sentence*, the LLM reranks that word's candidate synsets, so the top-`N`
  beam keeps the contextually-right senses (a better-ordered beam ⇒ a tighter cap for the same
  recall ⇒ fewer seeds). This is exactly neural contextual supertagging
  ([xu-auli-clark-2015-rnn-supertag]) in zero-shot form, and it **reuses the resolver's
  proposer-behind-oracle pattern** (D64 §4): a sense reranker is the same shape as the anaphora
  `Proposer` — `(word, sentence, candidate synsets) → ranked synsets` vs.
  `(hole, candidates) → ranked antecedent IRIs` — an *untrusted LLM ranking over a typed candidate
  set, with the kernel as the validity oracle*. So it shares the same trait family (mock for CI /
  `allms` live / orchestrator prod). Division of labour: the **LLM ranks plausibility**, **felicity
  (Lever B) enforces type/grammar** (the LLM never votes on validity), and **widen-on-failure**
  recovers any contextually-right sense the LLM wrongly down-ranked — a bad rank costs a re-parse,
  never a missed parse. Caveats: fine-grained synset WSD is hard, but pruning only needs implausible
  senses pushed down (coarse ranking suffices + felicity/fallback cover the rest); **batch one call
  per sentence** (not per word). Pre-parse and lexical-level — distinct from S4 structural
  disambiguation (which selects among *full felicitous parses* post-parse); they compose.

**Lever B — exact mid-chart felicity pruning (cut the combination count).** Type-check (or a cheap
type-compat pre-check of) *interior* constituents during CKY and drop the ill-typed ones
immediately, rather than only at the full span. **Exact** — no search errors — which is the rare
luxury the typed kernel affords. This is the principled headline fix: it keeps cells small at the
source, so there is little to clone, retain, share, or cap.

Sequencing: **B is the ceiling-lifter** (sense-polysemy makes the *count* explode super-linearly,
so cutting it beats making an exploding number of items cheaper); **A** shrinks what enters the
chart; the existing forest cap stays as the final beam; **packed-forest / `Rc` subtree sharing**
([tomita-1987-glr; billot-lang-1989-shared-forests]) is a follow-on for per-item cost *if* size
is still the wall after the count is controlled — note `Rc<Exp>` is a foundational kernel refactor
(`Box<Exp>` is pervasive), so a parser-only packed forest is the lighter route there.

Out of scope unless count-control proves insufficient: A\* / coarse-to-fine multi-pass machinery —
our exact filter (B) is the simpler sound pruner for a typed grammar.

## 5. Verification plan

- A length-capped baseline exists (`prototype_over_wrn_first_page`, `MAX_UNIT_TOKENS = 22`). After
  Lever B: the cap should be raisable (long WRN sentences parse without OOM); measure max
  parseable length + peak memory before/after.
- Lever A: parse with `sense_rank`-top-`N` seeds; verify no coverage regression on the
  closed-class/determiner battery (the widen-on-failure path must recover any dropped parse).
- Both must leave the closed-term grammar tests green (no felicitous parse lost).

## 6. References (verified)

`bangalore-joshi-1999-supertagging`, `clark-curran-2004-supertagging`, `clark-curran-2007-ccg`,
`xu-auli-clark-2015-rnn-supertag`, `klein-manning-2003-astar`, `caraballo-charniak-1998-fom`,
`charniak-etal-2006-coarse-to-fine`, `charniak-2000-maxent-parser`, `tomita-1987-glr`,
`billot-lang-1989-shared-forests`, `oepen-carroll-2000-ambiguity-packing` — all in
`docs/references/eigenius_related_work.bib`, identifiers verified against the ACL Anthology / DOIs.
