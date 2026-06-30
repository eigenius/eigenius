# D62 — grammar-gap analysis: why the 12 fully-known WRN units don't parse

> **CORRECTION 2026-06-29 (Derived — read this first).** Two premises below are now revised by
> measurement after the felicity-gate OOM fix ([[axiom_env_fullscan_oom]], `build_axiom_env` made
> index-driven), which unblocked clean full-lexicon re-measurement:
>
> 1. **Predicate nominals (§2 #6) ALREADY WORK — not a gap.** On the small lexicon `HeLa is a
>    cell line` and `a cell is a gene` both parse (kernel test, fast). The WRN clause `WRN is a
>    vulnerability` fails for a *different* reason: `WRN` is imported as a **common noun**, so as a
>    bare *singular* subject it needs a determiner (`*gene is a vulnerability` is genuinely
>    ungrammatical). The real issue is **gene/entity symbols used as proper nouns** in science text
>    but modeled as CNs — a lexicon-modeling gap, not a copula-grammar gap.
> 2. **NEW highest-leverage gap — bare-plural NP arguments.** `genes affect cells` does **not**
>    parse (full lexicon → `—`; reproduced on the small lexicon with a plural-aware lemmatizer:
>    `genes affect HeLa`, `HeLa affects genes`, `genes are large` all 0 parses), while
>    `a gene affects a cell` and `these genes affect HeLa` (determiner) do. Cause: a bare plural
>    common noun `cat_n(T, pl)` gets number-refined and a `kind_subject` edge, but has **no
>    type-shift to an argument `cat_np(T, pl)`** — only determiners produce argument NPs. Bare
>    plurals (existential/generic) are pervasive in scientific prose → this blocks the most basic
>    clause shape. Fix: a unary shift `cat_n(T,pl) → cat_np(T,pl)` with the existential GQ sem,
>    mirroring `a`/`some` (`exists_sem` subject / `obj_exists_sem` object); existential is the
>    established first-cut convention (closed-class.esl §"definite/demonstrative ≈ existential").
>    Generic semantics is a later refinement.
>
> Also: the OOV/grammar split is now **per-fragment** in `diagnose_grammar_gap_fragments` (it reports
> missed tokens), and sub-clausal NP fragments (`MSI cancer models`, `the helicase activity`) are
> *expected* non-clauses, not gaps — the diagnostic seeks a full-span `S`.
>
> **NEW (not in the §2 inventory) — SIMPLE PAST TENSE, now fixed.** The inventory was built by
> inspecting *constructions* and assumed verbs parse; in fact only present (3sg/pl) + participles were
> emitted, so **no simple-past clause parsed** (`HeLa affected BRCA1` → 0) — the dominant blocker for a
> past-tense narrative like the WRN page. Fixed: the WordNet importer now emits a **finite simple-past**
> verb form (`e_v…_fpast`, `Fin=fin`, `num_any` subject — past tense has no number agreement; reuses
> the past-participle surface, with the `went`/`gone` irregular-past class a known follow-on), and the
> bootstrap adds the **past copula** `was`/`were` (over a predicate nominal/adjective). The §2 items
> below remain (apposition, relatives, lists, numerals, fronted adjuncts, by-agent passives, `but not
> X`, deep NPs); **predicate nominals (#6) + bare-plural NP args + gene-symbol proper nouns are done**.
>
> **Parser-side §2 batch (2026-06-29, no reseed) — two more done:**
> - **N-N compound stacking + bare-plural over a composed compound** (`MSI cancer models`): 3-noun
>   stacking already left-branched; the gap was the bare-plural NP shift ran only at lexical *leaves*,
>   so a composed `cat_n(_, pl)` (compound / adjective-refined) could never be a bare argument. The
>   shift now also runs on composed cells. → `MSI cancer models required HeLa` open×2,
>   `… required the helicase activity of WRN` open×189, `thus novel therapies are needed` open×12 (all
>   were grammar-gap).
> - **Stacked attributive adjectives** (`synthetic lethal vulnerability`): refining an already-refined
>   noun now **conjoins over the same base** (`Σx:Base. P(x) ∧ adj(x)`) instead of nesting
>   (`Σy:Σ. adj(y)`, which applied the adjective to the Σ pair — ill-typed). →
>   `WRN is a synthetic lethal vulnerability` **CLOSED×6** (was a readback panic, then grammar-gap).
>
> - **GQ-as-preposition-object** (`within a gene`, `for a gene`, `within cells`): before, only a bare
>   NAME could fill a preposition's `cat_np` object slot (`within HeLa` ✓ / `within a gene` ✗) —
>   because a quantified NP is *type-raised* (`S/(S\NP)`, `λV. Q(A,V)`), not a plain `cat_np`. Unlike
>   OpenCCG (where a determined NP stays a plain `np` and scope is separate), our grammar bakes scope
>   into the category, so the GQ must scope *over* the preposition. New parser rule (no new lexical
>   entry, **no reseed**): a `cat_pp/NP` preposition consuming a subject-form GQ on its right yields
>   `cat_pp` with `λx. GQ(λy. prep(x,y))` — the parser-side analogue of the verb-object raise
>   (`a_obj`), polymorphic in the functor. ONE rule covers **both** prep families (post-nominal
>   `cat_pp` noun-mod *and* VP-adjunct, since both surface the GQ as `S/(S\NP)`) and **all three**
>   object kinds (name / singular ∃-GQ closed / bare-plural deferred-Q open): →
>   `WRN is a vulnerability for a gene` **CLOSED×14**, `therapies are needed for a gene` **CLOSED×184**,
>   `HeLa affects a gene within cells` **open×4**, `WRN is a vulnerability for MSI cancers` **CLOSED×16**
>   (all were grammar-gap). Restricted to `cat_pp` functors so it never re-derives `a_obj`.
>
> **Parser-side no-reseed batch (2026-06-29, after the measurement pass):**
> - **#5a fronted participial adjunct** — a subject-gapped `ger` VP (`affecting BRCA1`,
>   schematically `hypothesizing that P`) fronted as a sentence pre-modifier `S/S` asserting the
>   participial proposition alongside the matrix: `λm. And(m, body(hole))`
>   (`category.rs::front_participial`), where the participle's subject is CONTROLLED — a referent hole
>   (D64) ⇒ an **OPEN** parse resolvable to the matrix subject. core-en `purp-i`/`tpc` fronted-`s`
>   type-changes. The produced `S/S` rides the fronted-comma absorption (#5b) to span the trailing
>   comma, then forward-applies. → `affecting BRCA1 , HeLa affects BRCA1` open×1 ::
>   `And(affects(brca1,hela), affects(brca1, ?))`. Fixed en route: `freshen_anaphor`/`freshen_quant`
>   now recurse into `InductiveType`/`InductiveCtor` (the controlled-subject hole nests inside a
>   `logic:And`, so without it the hole stayed an unfreshened closed constant). *Follow-on:*
>   intransitive single-token participles (leaf cells) not yet shifted (only composed `ger` VPs).
> - **#5b transitional adverbs + fronted-comma absorption** — `thus`/`therefore`/`hence`/… added to
>   the lexicalized (transparent, clause-level `S/S`) adverb set; a sentence-initial fronted `S/S`
>   modifier now ABSORBS a trailing comma (the comma is otherwise a reserved coordinator with no chart
>   item, leaving a gap), so `Thus, S` composes. Plus **degree-modified adverbs** (`more commonly`,
>   `most notably`) as transparent sentence adverbs (2-token seed). → `thus , HeLa affects BRCA1`,
>   `more largely , HeLa affects BRCA1` CLOSED, transparent (same claim).
> - **#2A non-restrictive (appositive) relative — subject position** — `BRCA1, which affects HeLa, is
>   primary` ⇒ `And(is_primary(brca1), affects(…, brca1))`. The antecedent NP is type-raised to a
>   **conjoining quantifier** `λP. And(P(r), body(r))` (`category.rs::relativize_appos`, reusing the
>   type-raise cat so it composes like any subject GQ) — a SEPARATE assertion, NOT a Σ-restriction
>   (core-en `RelPro-Appos`: `s\s`+`Trib`). Signalled by the comma BEFORE the relativizer, so it never
>   competes with the restrictive rule (whose noun must be relativizer-adjacent); a trailing comma is
>   absorbed into the appositive NP span. **Correction to the close-out read:** the comma variant was a
>   coverage GAP (the comma broke restrictive adjacency), *not* the silent restrictive-collapse the
>   code-comment at `lookup.rs` suggested. *Follow-on:* the OBJECT-position antecedent
>   (`HeLa affects BRCA1, which …`) needs the object-raised conjoining form (test
>   `object_position_non_restrictive_relative_is_a_known_gap`).
> - **#7 light-verb `give rise to`** — NOT done here: it is lexical *data* (a multiword `(S\NP)/NP`
>   entry + a causation axiom), so it rides the numerals reseed, not this parser-side batch.
>
> **Remaining genuine clause gaps:** the still-open §2 list (apposition, comma-lists, relatives,
> numerals, `because`/`although` subordinators, S0 hyphenated compounds). NB: `novel therapies are
> needed for tumours` is **NOT** a prep-object gap (Derived: `therapies are needed for a gene` is
> CLOSED but `novel therapies are needed for a gene` is grammar-gap at the page beam=64, yet **open×216
> at beam=1024**) — the residual is ambiguity explosion from the attributive adjective `novel` over a
> bare-plural subject *plus* a PP, a Lever-B per-cell-beam scale issue (GH #97), not a missing rule.
> The NP-only fragments (`MSI cancer models`, `the helicase activity`) are expected non-clauses, not
> gaps.
>
> **The fix is derivation-level, not sense-level (Derived, 2026-06-29, `--features allms` live).**
> Re-ran the same sentence at the page beam (64) with the live `AnthropicSenseRanker` (sentence-
> contextual sense reranking) ON: still **GRAMMAR-GAP** (one span still explodes to ~10.8k items,
> capped to 64). So contextual *sense* disambiguation does not rescue it — the bottleneck is the cell
> beam ranking *derivations* by a context-blind scalar `Cost`, and the LLM reranker only reorders a
> word's senses (cap already ≤2). The real GH #97 lever is a contextual **derivation/constituent**
> ranker (adaptive supertagging), not the sense reranker. Test: `llm_reranker_on_structural_residual`.


> **MEASUREMENT-FIRST CLOSE-OUT PASS (2026-06-29, Derived — supersedes the §2 status column).** Before
> writing any more grammar code, the §2 table was re-probed on the small lexicon (fast, deterministic;
> tests `already_covered_constructions_are_derived` + `non_restrictive_comma_relative_is_a_known_gap` in
> `kernel/tests/closed_class_determiners.rs`). Result: **roughly half of §2 is already covered or was
> mis-recorded.** Now-**DONE** (Derived):
> - **#3 comma lists** — subject *and* object position (`HeLa affects BRCA1, BRCA1 and BRCA1` CLOSED).
>   `tokenize` already preserves `,` and maps it to `logic:And`; §3a's "tokenize strips all punctuation"
>   is now false.
> - **#6 predicate / VP coordination — fully done**, all three shapes: coordinated NP-predicate
>   (`is a gene and a cell line`), same-feature VP, and **cross-feature** adj-pred + verbal VP
>   (`is primary and affects BRCA1`) — the last was predicted to fail on a `Fin`-feature mismatch; it
>   doesn't (generalized conjunction handles it).
> - **#7 passive — done**, both by-agent long passive (`is affected by HeLa`) and agentless short
>   passive (`is affected`). `are needed` was already open×12.
>
> **Genuine remaining gaps** (Derived): **#2A** non-restrictive comma-relative (does NOT parse — the
> comma breaks the restrictive rule's noun-adjacency; a coverage gap, *not* the silent restrictive-
> collapse a code-read suggested — flag to recheck `lookup.rs:1068` when implementing); **#1**
> apposition (parenthetical asides are *dropped*, not parsed); **#2B** pied-piping (`in which`); **#4**
> numerals (the one reseed-gated item); **#5a** fronted participials; **#5b** `thus`/`more commonly`;
> **#7** light-verb MWE (`give rise to`); **#8** `but not X`; plus S0 work — **S0-b** preserve `( ) —`
> for apposition, **S0-c** the `Fig. 1d, e.` over-merge in `is_abbrev`. Core-en blueprints + our change
> sites are mapped per item (close-out analysis, 2026-06-29). **Reseed-gated: only numerals.** Highest
> risk: #1 (ambiguity vs the working comma-list) and #8 (argument-cluster gapping, which core-en itself
> defers). Cross-cutting: long units also need the GH #97 **derivation**-level ranker (adaptive
> supertagging) to survive the cell beam — sense reranking proven insufficient (see above).

*Analysis note. After the full-UMLS + closed-class/adverb batch, the WRN first page is **grammar-limited,
not vocabulary-limited** (`d62-encoding-prototype-findings.md`): 12 of 26 units are fully known yet
yield no parse. This note diagnoses the blocking constructions and maps each to a remediation in the
reference grammar (core-en OpenCCG, `references/openccg/grammars/core-en/`). Grading: the
construction inventory is **Declared** (linguistic inspection of the 12 units, not yet chart-instrumented);
the open/grammar split and the core-en families cited are **Derived** (measured / read from the grammar).*

## 1. First: they are *true* grammar gaps, not hidden open parses (Derived)

The harness classified via the **closed** forest only, so a unit yielding only an **open** parse (a
referent hole from `we`/`its`/pronouns, D64) would be misfiled as a grammar gap. Fixed the harness to
split `Open` from `GrammarGap` (parse via `parse_open`) and re-ran: **open = 0.** All 12 produce **no
full-span `S` at all** — closed *or* open. So the pronoun holes never even arise: the clause fails to
assemble before reference resolution is reachable. This is purely missing **grammar coverage**.

(A caveat on precision: the blocking constructions below are inferred by linguistic inspection, not by
instrumenting the chart to see the exact stall point. Confirmation = chart-max-span instrumentation, or
the ratchet — fix a construction, re-measure.)

## 2. The recurring uncovered constructions (Declared)

Each long unit needs *all* its constructions to compose; one gap kills the full-span parse. So the
leverage is in the **recurring** constructions. Ranked by frequency across the 12, with D63 status and
the core-en remediation:

| # | Construction | Units | D63 status | core-en remediation |
|---|---|---|---|---|
| 1 | **Apposition** — parenthetical `(MSI)`/`(PARP-1)`, em-dash appositive, comma-appositive naming (`the MMR genes MSH2, MSH6, …`; `data sets, project Achilles and project DRIVE`) | 1,2,3,6,9,11 | **none** | appositive comma (`punct.xsl:128`) + `RelPro-Appos` family (`misc.xsl:48`, `rel.appos`) + appositive PP (`pp.xsl:50`) |
| 2 | **Non-restrictive & pied-piping relatives** — `…, which results from…`, `…, which is caused by…`, `in which`, `through which` | 1,3,6,11 | restrictive `that`/`which` only (Slice 6-rel); no comma / no pied-piping | `RelPro-Appos` + `which` Wh entry (`dict.xsl:256`) + pied-piping via the relativizer (`misc.xsl:31`) |
| 3 | **Multi-item comma lists** — `X, Y, Z and W` (`colorectal, endometrial, gastric and ovarian cancers`) | 6,9,10,11 | binary left-branching `and`/`or` only | list-completion **typechanging** rules: `np-list-c` / `s-list` / `pred-adj-list` (`conj.xsl:157–263`) |
| 4 | **Numerals / measure determiners** — `14 cell lines`, `two data sets`, `0.56-fold fewer` | 1,2,9,11,12 | **none** (no numeral determiners) | numerals as determiners (`det.xsl`); measure phrases |
| 5 | **Fronted participial / adverbial adjuncts** — `Hypothesizing that …, we …`; `demonstrating that …`; `More commonly,`; `Thus,` | 3,7,8,9,10 | sentence adverbs `S/S`/`S\S` (just added) but not participial adjuncts or `thus`/`more commonly` | `s.from-1.fronted` (`cats.xsl:881`) + adverb Initial family (`adv.xsl`) + reduced relatives (`unary-rules.xsl:22`) for `-ing` adjuncts |
| 6 | **Predicate nominals + coordinated predicates** — `WRN is a vulnerability and … drug target`; `were distinct and contained …` | 5,12 | copula + predicative **adjective** only | copula `be` takes the predicate as an argument (`v.xsl:484` `copula.pred`; X and P both args) + `pred-adj-list` |
| 7 | **Long passives + light verbs** — `is caused by [agent]`; `give rise to`; `are needed` | 1,3,6,8 | short passive (existential agent) only | passive lexical forms (`v.xsl`); light-verb `give rise to` as an MWE |
| 8 | **`but not X` contrastive ellipsis** — `…, but not its exonuclease activity` | 4 | `but`→`And`; no negated-NP ellipsis | the `but` family (`conj.xsl`) with the elided predicate |
| 9 | **Deep PP-stacked / complex subject NPs** — `The success of … inhibitors in cancers with deficiencies in …` | 2,8,11 | PP noun-modifier exists; depth/subject-NP composition is the strain | (covered shape; stress-test once 1–5 land) |

Out of band: **U10 is also an S0 segmentation defect** — the sentence splitter over-merged two
sentences at `Fig. 1d, e.` (abbreviation period), producing a giant unit. A tokenizer/segmenter fix,
not grammar.

## 3. Prioritization (Declared)

Two axes: **recurrence** (unblocks many) and **nearest-unit** (unblocks a specific short unit now).

**Highest leverage — necessary across most units (do first):**
- **Apposition (#1)**, especially the parenthetical `(MSI)`/`(PARP-1)` gloss — it appears in the
  majority of units; nothing parses around it. core-en's appositive-comma + appositive-relative is the
  blueprint.
- **Multi-item comma lists (#3)** — core-en's list-completion typechanging rules are a clean, bounded
  add over our existing binary coordination.
- **Numerals (#4)** — small, self-contained determiner additions; recur across 5 units.

**Nearest wins — unblock a specific short unit (high morale / validates the ratchet):**
- **Predicate nominals (#6)** → unit 5 (`These findings show that WRN is a … vulnerability …`) is
  close once `is a NP` + coordinated-NP predicate work.
- **`but not X` (#8)** → unit 4 (the shortest) is close once contrastive ellipsis + the `of`-PP land
  (it would then be an **open** parse via `its`, not a grammar gap).
- **Fronted discourse adverbs (#5, partial)** → `thus` / `more commonly` (extend the lexicalized
  discourse-adverb set + degree `more`); helps units 7, 8.

**Then:** non-restrictive/pied-piping relatives (#2), fronted participials (#5), long passives (#7),
and finally stress-test deep NP stacking (#9). And the S0 abbreviation-merge for unit 10.

## 3a. Structural correction — half of these are S0-gated, not grammar (Derived)

`tokenize` **strips all structural punctuation** (`tokenize("p53, (BRCA1)") → ["p53","brca1"]` — commas,
parens, em-dashes gone). So the markers that *signal* several constructions are invisible to the parser:

- **Apposition (#1)** — the parenthetical `(MSI)` / em-dash / comma-appositive *is* the only signal;
  stripped, `microsatellite instability (MSI)` is three adjacent NPs. **Cannot be a grammar rule until
  S0 preserves the markers.**
- **Multi-item lists (#3)** — without the commas, `X, Y, Z and W` → `X Y Z and W`, which the binary
  `and` rule can't join. **S0-gated.**

The rest are **word-driven, not punctuation-gated**: relatives (#2 — teach the relativizer `which` /
`in which`; the comma is only semantic), predicate nominals (#6), numerals (#4), passive (#7),
`but not X` (#8). So the campaign is two tracks:

- **Track A — S0 punctuation foundation, then marker-keyed constructions** (apposition, lists,
  comma-boundary adjuncts). Reference design: core-en treats **punctuation as typed lexical tokens** —
  a `Comma` family (`pos=","`, `punct.xsl:153`) with **List-Comma** (`comma.conj.np`, builds a list)
  and **Appositive-Comma** (`comma.vp.ng`) entries. Our analogue: preserve `,`/`(`/`)`/`—`/`;` in S0
  and make them **parser-reserved words** with list / apposition rules (mirroring how `and`/`or` are
  reserved). Parser-side → measurable via `cargo test`, no reseed.
- **Track B — word-driven constructions**: relatives (`which`/pied-piping), predicate nominals,
  numerals, passive, `but not X`. Parser-code parts re-measure with no reseed; the bootstrap parts
  (numerals, copula, `but`) batch into **one** reseed.

Per-construction validation = synthetic **unit tests** (as for the adverb slice); full-WRN re-measures
are the milestone view (a long unit flips only once its *whole* construction set lands).

## 4. Note on method / next step

This is the *map*, not the *fix*. The honest next step before building is to **chart-instrument** one
or two units (report the maximal-span constituents the chart did build) to confirm the inferred stall
points — then take the highest-leverage construction (apposition) first and re-measure, letting the
ratchet validate each construction empirically rather than trusting the inventory wholesale.
