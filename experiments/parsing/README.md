# Parse-rate experiment — reproducible protocol

How to measure whether the DCG parser **covers** a page of prose (every sentence parses) and how
**faithfully** it resolves it (how many sentences reach a single reading), over the *full* WordNet +
UMLS lexicon.

Every step is scripted. **Do not hand-roll a `cargo test` invocation** — the three ways to get a
wrong-but-plausible number are all in that command, and the scripts exist to close them (§4).

---

## 1. Provision the source data (once)

Both corpora are licensed and gitignored; neither is vendored.

```bash
scripts/provision-wordnet.sh          # WordNet 3.0 → references/WordNet-3.0/dict
scripts/provision-countability.sh     # Wiktionary uncountable nouns → references/wiktionary/
# UMLS requires your own UTS licence; download the release, then:
scripts/provision-umls.sh extract     # → references/umls/<release>/META/{MRCONSO,MRSTY,MRSAB,MRRANK,MRDEF}.RRF
```

## 2. Seed the store

Builds the importers + kernel image, cleans the docker volume, imports WordNet + UMLS into a
persisted chain, and copies the volume out to a dated read-only snapshot.

```bash
scripts/reseed-lexicon-db.sh --umls-all      # ~20 min → ../db-snapshot/wordnet-umls-<date>
```

The reseed also applies the **junk-atom drop set** (`experiments/lexicon-align/drops.json`, via
`--drop-atoms`): UMLS atoms whose only contribution is a case-mangled collision with a common word
(`gENE`→`gene`), which the D63 adjudicator judged a *different* concept. Their per-form entries are
skipped at import (the concept class stays; the common word is still covered by WordNet — every
dropped surface is a WordNet lemma). Regenerate the list with `lexicon-align drops` (see
[experiments/lexicon-align](../lexicon-align/README.md)).

### 2a. Layer the WordNet↔UMLS alignment on top

The measured store is the base **plus** the cross-lexicon concept alignment
([experiments/lexicon-align](../lexicon-align/README.md) — merges that make a WordNet synset and a
UMLS concept denote ONE class). ONE command emits the layer from `merges.json` **and** loads it —
never hand-carry a pre-emitted `.esl` (a stale one silently measures the wrong store):

```bash
scripts/build-alignment-snapshot.sh \
  --base ../db-snapshot/wordnet-umls-<date> \
  --out  ../db-snapshot/wordnet-umls-aligned-<date> \
  --merges experiments/lexicon-align/merges.json
```

The base is treated as **immutable**: it is staged into a scratch volume, the layer is loaded
**through the kernel** (so it goes through the validator — Rule 22, type-checking, the commit gate),
and the result is written to a NEW snapshot. Loading through the kernel is not a formality: on the
first attempt the validator rejected the layer with **721 type errors** (WordNet *instance* synsets
used where a `class` was required), which a hand-rolled loader would have written straight into the
store as silent lexicon corruption.

**A reseed is required after any edit to a bootstrap ontology** (`ontologies/logic`,
`ontologies/lexicon/closed-class`, …): the persisted chain is rooted at the bootstrap it was seeded
with *by content hash*, so an edited bootstrap makes the old store unresumable (`ManifestDrift`,
fail-closed). Pre-production posture is drop-and-reseed.

**Including comments.** `current_manifest()` hashes the raw source bytes —
`Sha256::digest(spec.source.as_bytes())` (`kernel/src/bootstrap/mod.rs`) — not the compiled
resources. So a typo fix or a clarifying comment in a bootstrapped `.esl`/`.json` invalidates every
snapshot exactly as a semantic change would. Near-missed on 2026-08-03: a two-line comment
correction to `ontologies/reasoning/reasoning.esl` would have cost the hour-long reseed that was
running at the time. Documentation about a bootstrap ontology is cheaper to put in `docs/notes/`.

## 3. Measure, and evaluate

```bash
scripts/measure-parse-rate.sh                       # newest snapshot, CNL-v3 page, live reranker
scripts/measure-parse-rate.sh --page cnl-v2         # a different page
scripts/measure-parse-rate.sh --no-llm              # cap-only, for an A/B
scripts/measure-parse-rate.sh --snapshot /path/to/store
```

It builds **release**, runs the sweep, and writes **one directory per run** under
`experiments/parsing/results/`:

```
results/<stamp>-<commit>[-dirty]-<page>-<kind>[-arms]/
    run.log      the harness output, led by a provenance header
                 (commit, page, snapshot, reranker, profile, config, exact command)
    ranks.json   every ranking the LLM reranker produced
```

then scores it:

```bash
scripts/eval-parse-rate.sh <run.log>                  # score one run
scripts/eval-parse-rate.sh <run.log> --baseline       # …and compare against the committed baseline
scripts/eval-parse-rate.sh <run.log> <other-run.log>  # …or against another run
```

`eval-parse-rate.sh` exits **0** = valid and meets baseline, **1** = the run is not trustworthy
(refuses to score it), **2** = regression.

### Replay — the reproducible arm

The reranker is an LLM: the one component that can answer differently for the same code against the
same store. Every run therefore **records** its ranking decisions to `ranks.json`, and

```bash
scripts/measure-parse-rate.sh --replay results/<run>/ranks.json
```

re-runs them with **no LLM at all** — deterministic, no network, no cost. A replay whose lexicon or
page has changed **MISSES**, and misses are *counted, not hidden*: a replay with `misses > 0` is a
different experiment, not a reproduction.

This is what lets a parser change be A/B'd against **fixed** rankings, isolating the code from the
model.

### What is committed, and what is not

`experiments/*/results/` is **gitignored** — run logs and rank recordings are large and
regenerable. The committed artifact is **`baseline.json`**: the reference run distilled to its
provenance + expected metrics, so the gate survives a clean checkout. Update it deliberately.

---

## 4. The three traps — each has produced a false result

The scripts close all three. They are documented because *reading the raw log by eye reopens them.*

1. **`--release` is load-bearing, not an optimization.** A debug build does not merely run slower —
   it **changes the result**. Debug stack frames are larger, so NbE readback recursion **overflows
   the stack**, the parse dies, and the harness reports it as a `GRAMMAR-GAP` *indistinguishable
   from a real one*.
   → On 2026-07-11 a debug run reported **12 grammar gaps and a stack overflow** against a snapshot
   that measures **grammar-gap 0** in release. Hours were spent bisecting a bug that did not exist.
   **Timing is the tell: the release sweep takes ~7 minutes.** Tens of minutes ⇒ you are in debug.
2. **The reranker must be on.** The canonical measure is `--features use-llm` + `ANTHROPIC_API_KEY`.
   A cap-only run inflates gaps *by construction* and is not comparable to a reranked one. The
   harness prints `contextual reranker: …`; `eval-parse-rate.sh` refuses to compare across kinds.
3. **`ranks.json` is a PRE-DEDUP instrument.** It records the reranker's *candidate list*, built
   before `lookup_span` runs the dedup and the cap — so it shows what the model was **asked**, not
   what **seeded**. It cannot tell you how many cap slots a merge freed. (I drew a conclusion from it
   that it structurally cannot support, three times.) Its `sems` field records what each sense
   *denotes*, so two senses with different labels and the same `sem` are visibly ONE concept — but
   the pre-dedup caveat still stands.
4. **`grammar-gap`, `total-readings`, and `total-skeletons` come from the summary line, and nowhere
   else.** The per-unit listing enumerates only AMBIG units and **silently omits grammar gaps** —
   counting from it reports 0 gaps on a run that had many. `eval-parse-rate.sh` reads the metrics from
   the `=== WRN first page over FULL lexicon: … ===` line only. And **a run with no summary line did not complete**; its
   partial counts are not a result.

---

## 5. What the outcomes mean

Each sentence unit is classified:

| outcome | meaning |
|---|---|
| `ENCODED` | exactly one reading survives — **the goal** |
| `AMBIG` | parses, but >1 reading survives (the faithfulness problem) |
| `OPEN` | parses, but a proposition is left open |
| `GRAMMAR-GAP` | no parse: every word is known, but nothing composes (**a coverage failure**) |
| `MISSING-LEXEME` | no parse: a word is out of vocabulary (**a lexicon failure**) |
| `SCALE-BOUND` | skipped: beyond the length bound (>60 tok) |

**Coverage gate:** `grammar-gap 0` and `missing-lexeme 0` — every sentence parses. NON-NEGOTIABLE.
**Faithfulness goal:** raise `encoded` (units at exactly one reading).
**Multiplicity signal:** lower `total-readings` — the sum of closed readings over all units, the
more sensitive over-generation signal (a single sentence dropping from 40 readings to 20 moves it
where `encoded` does not). The harness prints it on the summary line and a **reading-count
histogram** with **pinned buckets** (`READING_BUCKETS` in `crates/eigenius-wordnet/tests/db_backed_encoding.rs`):
`0 (open/gap) · 1 (encoded) · 2-3 · 4-10 · 11-30 · 31-100 · >100`. The buckets are fixed in that one
constant so they do NOT drift between runs; `eval-parse-rate.sh` surfaces them verbatim and gates
`total-readings` against `total_readings_ceiling` in `baseline.json`.

**Structural multiplicity — the clean lever:** `total-skeletons` — distinct bracketings with senses
erased (runs of ≥4 digits → `§`), summed over units, printed on the summary line as
`total-skeletons N (sense× = total-readings / total-skeletons)`. Because it is sense-independent it is
**drift-free** (the reranker's sense choices collapse to `§`), so it isolates STRUCTURE from the sense
multiplicity `total-readings` conflates — which matters because `total-readings` has repeatedly *risen*
while structure *fell* (M3, RNR: the reranker sense-collapses the very units a structural fix improves).
`eval-parse-rate.sh` reports and gates it against `skeletons_ceiling` in `baseline.json`; a skeleton
RISE is the true over-generation signal. It is computed over the `SENSE_CAP` reading set (not fully
uncapped — `factor_ambiguity` is the manual uncapped deep-dive), but the erasure is what makes it clean.
**Prefer `total-skeletons` for a structural claim; `total-readings` is its sense-inflated companion.**

**Isolating a LEXICON change (drops, merges, imports) — use `--no-llm`.** The reranked metric is the
progress number, but the LLM reranker drifts ~5% between runs even at `temperature 0`, which swamps a
small lexicon change (16 dropped atoms moved `total-readings` by ±7, inside the ±60 drift band). A
lexicon change alters the reranker's candidate set, so a recorded `ranks.json` is not transferable
across it and `--replay` cannot A/B it cleanly either. **Cap-only (`--no-llm`) is deterministic** —
no LLM — so pre-vs-post cap-only isolates exactly what the lexicon change did, free of reranker noise.
Report both: cap-only for the deterministic delta, reranked for the tracked metric.

---

## 6. Reference run — the number everything is judged against

Committed as `baseline.json`; the full log is `results/2026-07-10-reference/run.log` (gitignored).

```
=== WRN first page over FULL lexicon: 62 units → encoded 1, ambiguous 60, open 1,
    missing-lexeme 0, grammar-gap 0, scale-bound (known, >60 tok) 0 ===
test result: ok. 1 passed ... finished in 393.85s
```

**Every sentence parses; only 1 of 62 resolves to a single reading.** The residual problem is
**ambiguity, not coverage.**

| | |
|---|---|
| commit | **`7933f05`** ("update default snapshot"), branch `parsing-fixes`, 2026-07-10 23:11 |
| profile | **release** (`target/release/deps/db_backed_encoding-510a93e5b355b773`) |
| features | **`use-llm`** — `AnthropicSenseRanker (live)`, model `claude-sonnet-4-6` |
| snapshot | `../db-snapshot/wordnet-umls-all-alone-2026-07-10` |
| page | `references/publications/WRN-Helicase-Nature-OCR/first-page-cnl-v3.txt` |
| augmentation | `1 OOV grounded + injected, 0 residual OOV` |
| knobs | `SENSE_CAP = 2`, `CELL_BEAM = 64` (widen-on-failure to 16 / 512) |
| runtime | **393.85 s** |

**Relation to `main`:** `7933f05` was squashed into `41af6db` ("Parsing fixes (#105)", now `main`).
Across `kernel/`, `ontologies/`, and `crates/*/src` the two are **byte-identical** — the only delta
is 6 lines in the test harness (`DEFAULT_SNAPSHOT` + doc comments). **The parsing code on `main` is
the code that produced this result.**

## 7. Ambiguity decomposition

`results/2026-07-10-ambiguity-factoring/run.log` — `factor_ambiguity_structural_x_sense` over the same page,
splitting each unit's readings into **structural skeletons × sense combinations**:

```
readings  median 32   ≈   skeletons  median 6   ×   sense×  median 5.5
```

Both axes are live and they *multiply*. Collapsing senses perfectly leaves ~18% of readings;
perfecting the structural normal form leaves ~15%. **Neither alone reaches ENCODED.**

### 7c. Faithfulness = "the unit contains its expected reading", not encoded-count

Encoded-count is a **weak** faithfulness signal: a unit can be encoded on the WRONG reading. On
2026-07-20 a gloss bug made "specific repair proteins" close on `compound_kind(x, C0205369 "Specific
(qualifier)")` — one reading, ENCODED, and wrong. Fixing it restored the adjective reading, so the unit
became AMBIG ×2 and encoded *fell*. Chasing encoded upward would have rewarded the bug.

So faithfulness is gated on **expected-reading hits** instead. `experiments/parsing/expected-readings.tsv`
pins, per curated unit, the sense-erased skeleton of the reading a human has verified is correct
(`sentence <TAB> skeleton <TAB> note`). The gate asserts each unit still **contains** that skeleton among
its readings — drift-free (senses erased) and robust to added ambiguity: a unit going ENCODED→AMBIG while
keeping the right reading is **not** a regression. `eval-parse-rate.sh` regresses on a hit drop or a
curated-set shrink; `encoded` is reported but no longer gated.

Author entries from a run: `EIGENIUS_DUMP_SKELETONS=1 scripts/measure-parse-rate.sh --replay <ranks>`
prints every unit's skeleton set; pick the correct one and pin it. **`encoded` ≠ `correct`** — verify
each entry; the 2026-07-21 seed (the 20 single-reading units) is provisional and pending sign-off.

### 7b. The skeleton eraser must erase the LEXICON PREFIX too

`total-skeletons` is the tracked structural lever, and it is only as good as `erase_senses`. The
erasure replaces **the whole token** carrying a run of ≥4 digits — not just the digit run.

That distinction is not cosmetic. The original erased only the digits, so the lexicon prefix survived:

```
n07342049 → n§        (WordNet)
C0205341  → C§        (UMLS)      ← different strings!
```

A word with one WordNet sense and one UMLS sense therefore produced **two "skeletons" for one
bracketing**. Measured on the reference page (2026-07-21): **86 of 326 skeletons — 26% — were this
artifact.** Corrected, `total-skeletons` is **240** and `sense×` rises 2.86 → 3.88, because those 86
belong to the sense axis. Nothing else moved (grammar-gap 0, encoded 10, total-readings 931): it is a
measurement correction, no reading was removed.

**Why it matters:** grammar changes are scored against `total-skeletons`. Under the old eraser a
quarter of that signal was sense noise, so a lexicon change that added a cross-lexicon sense pair would
show up as *structural over-generation*, and a genuine grammar fix would be diluted. If you ever see
skeleton counts move without a grammar change, check the eraser first.

How it surfaced: the attribution instrument reported the unit *"Nucleotide repeat regions are
microsatellites"* as 4 readings / 4 skeletons / `sense× 1.0` while simultaneously showing **two 2-way
surviving sense sites** — arithmetic that is only consistent if senses were being counted as structure.

### 7a. Attribution — which span, which rule, which sense

The split above says *structure vs sense*; it does not say **which word or which rule**. That question
used to be answered by hand (dump readings, erase senses, swap-ladder a trigger, look up each CUI) —
slow, and it produced several wrong diagnoses. `kernel/src/dcg/chart/attribute.rs` now reads it off the
packed forest directly: every OR-node that branches becomes a labelled *site* (competing `Leaf` edges ⇒
**sense**, competing rule/split edges ⇒ **structure**, named from `BinRule` / `UnaryKind`, and for the
lumped `Combinator::Compound` refined by the restrictor axiom into compound / adjective / pp / essive).

```bash
scripts/measure-parse-rate.sh --attribution --replay <run>/ranks.json   # page roll-up
EIGENIUS_TRACE_SENTENCE="…" EIGENIUS_TRACE_ATTRIBUTION=1 \
  cargo test -p eigenius-wordnet --test db_backed_encoding trace_one_sentence -- --ignored --nocapture
```

The roll-up ranks levers by `excess = Σ(factor−1)` — sense sites by surface form, structure sites by
named construction, with generic apply/compose lumped. A partial roll-up is emitted every 10 units, so
an interrupted run still leaves usable data (but see trap 4: a partial run is not a *result*).

**Read the two halves differently.**

*SENSE sites are felicity-intersected and DO rank.* Attribution runs after the top-span type-check and
dedup, so each sense is checked against the readings that actually survived (`n/m` = surviving/raw; a
non-survivor prints `[pruned]`). This matters enormously: on the reference page the surface `has` seeds
**7** noun/concept senses — `Hemagglutination test`, `Han Chinese`, `Ha Antibody`, hour-angle,
`rich_person`, plus UMLS `Possess`/`Have` — and **all 7 are pruned**. Ranking the raw forest put `has`
first; ranking survivors drops it out of the top 15 entirely. **Never size a lever from raw counts** —
an earlier analysis did exactly that and produced a wholly spurious root cause.

*STRUCTURE sites are RAW and rank NOTHING.* `kbest` records no per-reading derivation (items carry no
provenance and it truncates per node), so bracketings cannot be intersected. `compound` branching in
47/62 units is an upper bound, **not** evidence that residual multiplicity is structural — §6 measures
the extracted readings as sense-dominated, and the two count different populations. Making this half
rankable requires threading derivation ids through `kbest`/`cube`/`materialize_unary`.

Sense labels resolve through the layer chain — `C0018905 "Hemagglutination test" [T059]` — since the
importer emits each concept as `class umlscui:<CUI> : umlssty:<TUI>` with a `description`. No
`MRSTY`/`MRCONSO` side-lookup.

The instrument is **read-only**: on the reference replay, grammar-gap / encoded / total-readings /
total-skeletons are identical with and without `--attribution` (0 / 10 / 931 / 326).

---

## 8. What the recorded rankings already show

From `results/2026-07-11-1640-…/ranks.json` (407 words ranked; the LLM reordered **92%** of them):

**47% of ranked words spend BOTH `SENSE_CAP` slots on a cross-lexicon pair** — one UMLS sense and
one WordNet sense of the same word. And those pairs are frequently *the same concept*:

| word | UMLS gloss | WordNet gloss |
|---|---|---|
| `state` | "The way something is with respect to its main attributes." | "the way something is with respect to its main attributes" |
| `repair` | "The act of returning something to working order." | "the act of putting something in working order again" |
| `mismatch` | "A failure to correspond or match…" | "a bad or unsuitable match" |

The parser builds a reading for each. They are **not `Exp`-equal** (different IRIs), so
`subsume_duplicates` cannot collapse them — they survive as distinct readings and **multiply**. That
is the `sense×` axis (median 5.5/unit), measured rather than inferred, and it is the case for
[cross-lexicon sense alignment](../../docs/notes/d63-cross-lexicon-sense-alignment.md): make both
lexica's entries denote **one** concept.

---

## 9. Sense elimination (2026-07-12) — what a run now does

The reranker may **omit** a candidate index, and an omitted sense is **eliminated**: it stays out of
the `sense → rank` map, and the cap does not backfill its quota from the rejects
(`eff = cap.min(ranked)`). Before this, a permutation could reorder but never drop, so `SENSE_CAP=2`
— obliged to take two — seeded `BRIP1 wt Allele` as a reading of **"of"**, `Month of May` for
**"may"**, and `Department of Energy` for **"does"**.

**The cut applies at the BASE cap only.** On widen-on-failure it is ignored and eliminated senses
become seedable again, so a wrong elimination costs a **slower parse, never a grammar gap**. That is
why `grammar-gap 0` holds through it.

**Function words carry a `core:description`** (`ontologies/lexicon/closed-class.esl`, 132 entries).
They used to render as **blank lines** in the prompt — a function word's `sem` is an inline λ-term,
so it has no description — and the model was being asked to choose between `""` and a full NCI
definition. It eliminated the determiner `each` and kept *"Each (qualifier value)"*. Correctly: we
had told it nothing.

**`--no-llm` is now a genuinely different experiment**, not merely a noisier one: without a ranker
there is no elimination at all. It is not comparable to a reranked run, and `eval-parse-rate.sh`
refuses to diff across kinds.
