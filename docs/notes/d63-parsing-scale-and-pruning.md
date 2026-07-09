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

**Lever A — adaptive supertagging (cut the seed count). ✅ Deterministic form IMPLEMENTED.**
Seed only the top-`N` senses per token by `sense_rank`/scope (D65); **widen on parse failure** (the
Clark–Curran adaptive-β policy [clark-curran-2004-supertagging]). Inexact, but the widen-on-failure
loop recovers completeness, and it attacks the explosion *at the seed*, before any combination.

  *Implemented:* `LexicalIndex::with_sense_cap(n)` (`kernel/src/dcg/lookup.rs`) — an **opt-in**
  per-lemma cap (default off, so no behaviour change to the closed-class grammar) that keeps the
  lowest-`sense_rank` `n` entries per lemma at seed time. **Unblock measured** on the WRN page
  (`prototype_over_wrn_first_page`): with `sense_cap = 2`, **25 of 26 units parse (≤60 tokens)
  without OOM**, up from 9/26 at ≤22 tokens uncapped — i.e. the parser now runs over essentially the
  whole page. (Still 0 *encoded*, but now for measurable reasons — domain OOV + grammar — not an OOM
  that blocked measurement.) *Now implemented (2026-06-30 — supersedes the earlier "not yet"):* the
  **widen-on-failure loop** is in `parse_scoped_open` (`lookup.rs`): try the cap; if the parse comes
  back empty *and* every prose token is lexically known (so it is not an OOV miss), double the cap up to
  `SENSE_CAP_WIDEN_MAX = 16` and retry — so the cap never loses a parse a known-vocabulary sentence
  would otherwise get, while OOV-blocked sentences don't waste retries. The **contextual LLM sense
  reranker** (strong-form Lever A) is also implemented (`AnthropicSenseRanker`, `allms` feature, wired
  via `contextual_sense_ranks` as a one-call-per-sentence pre-pass that reorders each over-cap word's
  senses before the cap truncates). **Lever A is therefore complete** (deterministic cap + widen +
  contextual rerank).

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
luxury the typed kernel affords.

  *Caveat (sequencing):* with the **current** lexicon, verbs are generic `Entity → … → Prop`
  (selectional restrictions are still open — GH #93), so every Entity-subclass sense type-checks and
  felicity pruning is **largely a no-op for sense polysemy**. Lever B becomes the headline
  ceiling-lifter once #93 gives predicates narrow argument types (then ill-typed sense-combinations
  are dropped exactly). Until then, **Lever A (the sense cap) is the effective unblock** — which is
  why it was implemented first. Lever B still prunes spurious *structural* combinations (bad
  type-raise/composition) regardless, so it is worth landing, but its big payoff is gated on #93.

Sequencing: **B is the ceiling-lifter** (sense-polysemy makes the *count* explode super-linearly,
so cutting it beats making an exploding number of items cheaper); **A** shrinks what enters the
chart; the existing forest cap stays as the final beam; **packed-forest / `Rc` subtree sharing**
([tomita-1987-glr; billot-lang-1989-shared-forests]) is a follow-on for per-item cost *if* size
is still the wall after the count is controlled — note `Rc<Exp>` is a foundational kernel refactor
(`Box<Exp>` is pervasive), so a parser-only packed forest is the lighter route there.

Out of scope unless count-control proves insufficient: A\* / coarse-to-fine multi-pass machinery —
our exact filter (B) is the simpler sound pruner for a typed grammar.

**GH#97 status (2026-06-30).** Lever A is **complete** (deterministic cap + widen-on-failure +
contextual LLM rerank); Lever B's *inexact* per-cell cost beam (`with_cell_beam`) is implemented. The
**only remaining GH#97 piece is Lever B's exact mid-chart felicity pruning**, and per the §4 caveat its
sense-polysemy payoff is **gated on GH#93** (narrow verb argument types) — until predicates carry
narrow argument types, every WordNet sense subsumes every slot and an interior felicity check cuts no
sense combinations (only spurious structural ones), at the cost of a per-cell NbE check that is itself
expensive. **So the real ceiling-lifter is the chain GH#93 → Lever B, and GH#93 must land first.**
GH#93 is design-gated (4 deliberation points); its decision #3 is a live hazard for *this* corpus —
naively tightening a verb's subject to `Animal` would reject gene/protein subjects (`WRN affects …`).
Conservative resolution to carry into the #93 design: source selectional types from **WordNet sentence
frames** (the `Somebody ----s` / `Something ----s` animacy split that already ships with WordNet), which
prunes the `eat`/`think`-style nonsense senses *without* touching the `Something`-frame scientific verbs
(`affect`/`cause`/`encode`/`regulate`/`contribute`) that admit domain entities — so the pruning that
matters for `no cat eats a fish` leaves `WRN affects mismatch repair` untouched.

**Correction (see §4a, 2026-06-30):** the empirical chart-cell analysis of the CNL corpus shows the
"#93 → Lever B" chain is **not** this corpus's critical path. The CNL explosion is **nominal**
(compound-rule sense-product over content nouns), not verb-argument polysemy — so selectional
restrictions on verbs prune none of it. For the CNL the validated lever is the **contextual LLM sense
reranker**, with a **nominal-modification normal form** as the structural follow-on. #93 stays valid
for general WordNet (`eat`/`think`), just off the CNL path.

## 4a. Chart-cell analysis of the CNL corpus (2026-06-30) — what actually explodes

Instrumented the chart (PARSE_DEBUG per-cell **shape histograms** via `cat_shape`, type-indices
erased; + an `EIGENIUS_DUMP_CELL=i..j` full-category dump) and analysed the 5 CNL v2 sentences at a
wide beam. Findings (witnessed; `analyze_chart_cells_first_five` / `enumerate_function_word_noise` /
`verify_sense_lever_at_page_beam` in `crates/eigenius-wordnet/tests/db_backed_encoding.rs`):

- **The explosion is nominal, not verbal.** The cells saturating the beam are almost entirely
  `cat_n(Σ_, sg)` — refined (adjective-modified / N-N compound) nouns — often `shapes=1` (every kept
  item the same shape). Dumping one such cell (`cell[0..5]` of S1) showed the **same compound skeleton**
  (`compound_kind` nesting + Σ refinements) repeated with **different sense IRIs per slot**: each noun
  filled by a WordNet sense (`n…`) *or* a UMLS sense (`C…`), the compound enumerating the **Cartesian
  product** of per-noun senses. So the driver is **sense polysemy × the compound rule**, NOT verb
  argument polysemy.
- **⇒ GH#93 (selectional restrictions) is NOT the lever for this corpus.** Our verbs
  (`affect`/`cause`/`encode`/`regulate`/`lead`) are all `Something`-frame; the `eat`/`think` animacy
  litmus that motivates #93 doesn't occur. The explosion is `compound_kind` (opaque `Entity → Set →
  Prop`) enumerating noun senses — selectional typing on verbs touches none of it. (#93 remains valid
  for general WordNet, just orthogonal to the CNL beam pressure.)
- **Function-word noun-sense noise is real but not dominant.** Function words pick up dense-lexicon
  open-class senses: `is`→`be`=**beryllium** (`wn:Be.n.14631295`) + 50 `be`-verbs + UMLS; `an`→`AN`
  noun + gene-symbol named-individuals; `a`→letter/ampere/**adenine**/vitamin-A. These let the compound
  rule chain *across* a copula/determiner into a bogus noun pile. **Why the static cap doesn't drop
  them:** `sense_rank` is per-(lemma, POS) (`read_sense_ranks`), so beryllium is rank 0 *among nouns of
  `be`* — indistinguishable from the rank-0 `be`-verb under the per-lemma cap. The cap is a within-POS
  frequency prior, with no cross-POS or plausibility judgment.
- **The contextual LLM reranker is the effective sense lever; the deterministic POS rule is not.**
  A/B at the page beam (64), 5 sentences:

  | | baseline | +closed-class-wins | +llm | +llm+ccw |
  |---|---|---|---|---|
  | S1 | GAP | GAP | **open×80** | GAP |
  | S2 | open×20 | GAP | open×8 | GAP |
  | S3/S4/S5 | GAP | GAP | GAP | GAP |

  The **LLM reranker recovers S1** (downranks beryllium/adenine in context so the junk falls below the
  cap). The **deterministic "closed-class-wins"** (drop all open-class senses of any function word) is
  **harmful** — it regresses S2 and, with the LLM, breaks even S1, because the grammatical reading of
  "X is Y" *relies on* an open-class `be`-verb sense, and a blanket POS rule can't tell `be`-verb
  (needed) from beryllium (junk). Only contextual *sense* plausibility makes that cut. **`ccw` was
  reverted** (kept only as this recorded negative result); the LLM reranker is retained as the sense
  lever.
- **Residual after the sense lever.** S1 recovers but S3/S4/S5 don't. S4 gaps even at a wide beam — the
  real **compound-as-preposition-object** grammar gap (`for cancer therapeutics`: single-noun prep
  objects and compound *direct* objects both parse; only the combination fails). S3/S5 are beam-limited
  by **compound bracketing/derivation multiplicity** (the Catalan blowup of a stacked modifier chain —
  `DNA repair processes`, `attractive synthetic lethal targets`), which sense-ranking does not reduce.
  **SUPERSEDED (§4b, `2026-07-08`): after the grammar fixes all 5 first-CNL sentences parse CLOSED; the
  S4 compound-as-prep-object gap is CLOSED. The S3/S5 structural-multiplicity diagnosis stands and is now
  quantified (§4b).**

**Updated lever order for the CNL** (supersedes "B is the ceiling-lifter via #93" for this corpus):
1. **Contextual LLM sense reranker** (Lever A strong form) — validated, recovers S1; the cheapest win.
2. **A normal form for nominal modification** (canonical bracketing of the adjective/compound stack,
   the analogue of the existing coordination + Eisner NFs) — the structural lever for S3/S5, which
   neither sense-ranking nor selectional typing addresses. **Now the primary remaining lever (§4b);
   design: [d63-nominal-modification-normal-form.md](d63-nominal-modification-normal-form.md).**
3. The compound-as-prep-object grammar gap (S4) — separate, small. **CLOSED `2026-07-08` (§4b).**
GH#93/Lever-B selectional pruning is **not** on this corpus's critical path.

## 4b. Re-measurement after the grammar fixes (Derived, `2026-07-08`)

Re-ran `analyze_chart_cells_first_five` over the current snapshot (`wordnet-umls-all-2026-07-08`,
cap-only static rank, `sense_cap=2`, `cell_beam=1024`; `EIGENIUS_PARSE_DEBUG=1`; 23 s). **All 5 first-CNL
sentences now parse CLOSED** — S1×240, S2×150, S3×32, S4×8, S5×48 — where the `2026-06-30` run (§4a) had
S1/S3/S4/S5 all GAP (cap-only). **The residual is ambiguity, not gaps**, and the S4 compound-as-prep-object
grammar gap is CLOSED.

**The ambiguity decomposes as `structural bracketing/category-choice × sense-product`** (witnessed via the
classify-candidate sems, sense IRIs erased to count structural skeletons):

| sentence | closed | classify candidates | distinct structural skeletons | sense × |
|---|---|---|---|---|
| S1 `Synthetic lethality is an interaction between two genetic events` | 240 | 256 | 22 | ~12× |
| S2 `The co-occurrence of these two events leads to cell death` | 150 | 186 | 36 | ~5× |
| S3 `Each event alone does not lead to cell death` | 32 | 32 | 2 | ~16× |
| S4 `Scientists can exploit synthetic lethality for cancer therapeutics` | 8 | 8 | 2 | ~4× |
| S5 `DNA repair processes are attractive synthetic lethal targets` | 48 | 144 | 12 (**3 within one subject-frame**) | ~12× |

**Cleanest case — S5, within one subject-sense frame: exactly 3 modifier-stack skeletons** ×16 WordNet-vs-
UMLS sense variants over the noun slots:
1. all three modifiers as **adjectives** — `Σ:Σ:N. And(And(gt,gt),gt). compound_kind(K,N)`;
2. **nested compound** — `Σ:Σ:N. gt(…). compound_kind(K, …compound_kind(K,N))`;
3. **mixed** — one adjective + one compound + one adjective-on-compound.

**The refined-noun `cat_n(Σ_)` shape is the dominant saturating mid-chart shape** (top shape in 32 of 173
non-leaf cells; the rest are copula / type-raise artifacts) — §4a's "the explosion is nominal" holds
post-fix.

**Three consequences for the levers:**
- **Both are load-bearing and multiply.** The NF collapses the structural skeletons (S5: 3→1); the
  reranker/cap collapses the ×N sense product. Neither alone reaches a single ENCODED reading — witnessed,
  not assumed.
- **The structural count is non-trivial** (2–36 skeletons; S1/S2 the high cases), so the NF is real work,
  not a constant-factor tidy-up.
- **Category-choice entangles the two levers.** A word's adjective-sense routes to `Attrib`, its
  noun-sense to `KindCompound` — so a *structural* skeleton is partly *sense*-chosen. The NF must be
  defined over the chosen category, and the sense pick feeds it. This is where the spurious-vs-genuine
  criterion lands: S5's all-adjective skeleton (`attractive ∧ synthetic ∧ lethal`) is *meaning-distinct*
  from the compound `[synthetic lethal] targets` (the domain collocation) — so the right bracketing needs
  the lexicon to carry `synthetic lethal` as a multiword entry.

## 4c. S5's blow-up was mostly LEXICALIZATION — the cheapest lever is a CNL fix (Derived, `2026-07-09`)

`synthetic lethal` is not `synthetic` ∧ `lethal`; it is a lexicalized domain term (the attributive form of
*synthetic lethality*, C4280020). Unhyphenated it masqueraded as a two-adjective stack — `synthetic` and
`lethal` each seeded with adjective *and* noun senses — which is what drove S5's fork. **Fix: hyphenate it
in the CNL** (`synthetic-lethal`, a style-guide rule — [d62-controlled-language-style-guide.md](d62-controlled-language-style-guide.md)),
so the D63 hyphen morphology reads it as one compound adjective. **Zero parser code.** Measured on the v3
page (`first-page-cnl-v3.txt`; re-confirmed on the merged kernel + Rust 1.97, #101 parse-neutral):

| S5 `… attractive synthetic[- ]lethal targets` | v2 (`synthetic lethal`) | v3 (`synthetic-lethal`) |
|---|---|---|
| leaf seeding | `synthetic` (2 shapes) + `lethal` (2 shapes) | **one token, 1 shape** (predicative adjective) |
| classify candidates | 144 | **48** (÷3) |
| structural skeletons | 12 | **4** |
| closed readings (cap-only) | 48 | **24** |

**But at the page level the win is ambiguity/cost, not the tally.** Full-page reranked measure on v3
(`--features use-llm`, snapshot `wordnet-umls-all-2026-07-08`): **62 units → ENCODED 0, AMBIG 57,
GRAMMAR-GAP 5, MISSING 0** — *identical* unit classification to v2 (the same 5 gaps #3/#4/#7/#8/#9;
`microsatellite-stable` parses, no OOV). Reducing S5's readings keeps it AMBIG, so nothing crosses to
ENCODED. **Even reranked, S5 stays ambiguous — the reranker collapses *sense*, not the residual 4
*structural* skeletons.** So the three levers are distinct and composing: **(1) lexicalize/hyphenate**
(cheapest, done for the corpus), **(2) the sense reranker/cap** (built), **(3) the nominal-modification NF**
for the genuine structural residual. The NF's target is now the 4 residual skeletons (the `[[DNA repair]
processes]` bracketing + copula predication + the `attractive`/`synthetic-lethal` gradable pair) — **~3×
smaller than the v2 numbers implied**, and no longer confounded by the un-lexicalized term.

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
