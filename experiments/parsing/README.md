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

**A reseed is required after any edit to a bootstrap ontology** (`ontologies/logic`,
`ontologies/lexicon/closed-class`, …): the persisted chain is rooted at the bootstrap it was seeded
with *by content hash*, so an edited bootstrap makes the old store unresumable (`ManifestDrift`,
fail-closed). Pre-production posture is drop-and-reseed.

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
3. **`grammar-gap` comes from the summary line, and nowhere else.** The per-unit listing enumerates
   only AMBIG units and **silently omits grammar gaps** — counting from it reports 0 gaps on a run
   that had many. And **a run with no summary line did not complete**; its partial counts are not a
   result.

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

**Coverage gate:** `grammar-gap 0` and `missing-lexeme 0` — every sentence parses.
**Faithfulness goal:** raise `encoded`.

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
