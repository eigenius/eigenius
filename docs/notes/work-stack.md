# Work stack — unfinished work (top = active)

The single "where are we" pointer. A **LIFO stack** of the active working notes: work the **top** entry;
when its exit-gate is met, **pop** it and the entry below becomes active. When a sub-task splits off from
an entry, **push** its note on top. Keep this file current — it is the map back to the base plan after
any detour.

---

## Stack (top → bottom)

### 1. ▲ ACTIVE — [d63-parse-gap-closure.md](d63-parse-gap-closure.md) — **Phase 3 of 4: ambiguity**
Four-phase spine (user directive `2026-07-06`, worked in order — stop detouring):
**OOV ✓ → parsing gaps ✓ → ambiguity (HERE) → performance.**

#### STATUS — measured `2026-07-15`, `main`@`29930e4` (post `dcg-cleanup` merge), snapshot `wordnet-umls-aligned-v3-2026-07-15`
| config | units | GAP | MISSING | AMBIG | OPEN | ENCODED |
|---|---|---|---|---|---|---|
| **reranked** (`--features use-llm`) — *canonical* | 62 | **0** | **0** | 58 | 1 | **3** |
| deterministic (cap-only) — *the no-regression gate* | 62 | — | — | — | — | — |

> **Every sentence PARSES — `grammar-gap 0` and `missing-lexeme 0` (reranked). 3 of 62 resolve to a single
> reading.** The gap/OOV problem is **solved**; the ambiguity problem is **not**. `ENCODED 3/62` is the open front.
> *Deterministic (cap-only) row not re-measured since the alignment-v3 + sense-elimination work — re-run
> `scripts/measure-parse-rate.sh --no-llm` to refresh (last cap-only figure, `07-10` pre-alignment, was ENCODED 0).*

`ENCODED` climbed 1 → 3 (`2026-07-10` → `07-12`, now on `main`). The mover was **sense elimination** — the reranker
may now OMIT an impossible sense (the cap no longer backfills from rejects) and 132 closed-class entries carry a real
`core:description` instead of blank prompt lines; that alone took ENCODED 1 → 3–4 (baseline floor set at 3).
Cross-lexicon alignment (12,450 → 38,389 WordNet↔UMLS merges, v1→v3) cut reading *multiplicity* a few % but did
**not** by itself raise ENCODED — standing verdict, confirmed three times: alignment never reaches a single reading,
**the residual is structural** (readings ≈ skeletons × senses; both axes live, skeletons median 6). Treat ±1 ENCODED
as temp-0 reranker drift, not signal; gap/missing are the load-bearing columns. Full record:
`experiments/parsing/baseline.json`.

#### DONE
- **Phase 1 — OOV: CLOSED.** `missing-lexeme 0`, distinct OOV 0 (Stage-A augmentation grounds the page).
- **Phase 2 — parsing gaps: CLOSED.** `grammar-gap 0` (`20d608e`). History of the 12→…→0 descent and the
  per-gap root causes: **§0 + §3 of [d63-parse-gap-closure.md](d63-parse-gap-closure.md)** — not repeated here.
- **Faithfulness — exclusive-focus `alone` (`22e550a`).** Sentence 3 ("Each event alone does not lead to cell
  death") had **0 universals, a lost negation, and a "Department of Energy" subject**. The reranker was
  *already right* (it ranked `DOE` #19/drop, causative `lead` #0/#1) — the faithful reading existed at **no**
  cap, because post-nominal `alone` had no rule, so widen-on-failure kept lowering the cap until the noun-pile
  was the only complete parse. Fix: `alone` as a bare post-nominal `cat_pp` carrying the opaque
  exclusive-focus operator `ontology:sole` ("this event alone" ≡ "only this event"); reuses the existing
  `RefineKind::PpMod` rule with **zero new parser code**, closed-class ⇒ cap-exempt. Now:
  `∀x:(Σy:event. sole(y)). ¬(x causatively-leads-to cell_death)` — 50/50 readings, **0** noun-pile.

#### NEXT — the exit gate: `AMBIG → ENCODED`
Two concrete levers, cheapest first:
1. **Re-test `pos_prune`** (categorical drop of function-word-as-noun readings; `EIGENIUS_POS_PRUNE`, currently
   default-off). It is *the* lever against the `does→DOE`/`doe`/`DO` noun-pile junk that inflates ambiguity.
   It previously made sentence 3 **unparseable — but only because post-nominal `alone` had no rule. That
   blocker is now gone**, so it is newly viable and untested. Gate on the deterministic sweep (`GAP` must stay 0).
2. **Mass-shim precision fixes** (§6 of the parse-gap note): strictly-uncountable-head test +
   acronym↔domain-word collision filter — kill the spurious `mass` readings that inflate *both* reading count
   and parse time.

Levers already applied (hyphenation, build-then-subsume D3, sense cap/reranker) and the ones ruled out for this
corpus (NF §3.3 adjective rule): **§6/§6a of the parse-gap note** and
[d63-parsing-scale-and-pruning.md §4c](d63-parsing-scale-and-pruning.md).

#### DO NOT RE-TRY
- **Per-span pooled sense cap — tried, measured, REVERTED (`b91e100`).** Pooling the cap across a span's
  candidate lemmas *does* make the reranker's drop-verdict bite (a rank-dropped sense hiding in a sub-cap lemma
  bucket — `DOE` in the 2-entry `doe` bucket — otherwise slips the per-lemma cap). But it **regressed
  `grammar-gap 0 → 1`** (unit 52, *"The MSI relationship compared favourably…"*) by over-pruning a multi-lemma
  span, and it is **unnecessary now that `alone` exists** (the faithful reading is reachable at the tight cap,
  so widen-on-failure never fires and the junk is never admitted). Isolated by reverting *only* the seeding
  code — now the `dcg/parse/` module (the `dcg-cleanup` refactor split the old `lookup.rs`; the pooled
  sense-cap logic is in `parse/seed.rs`) — against the same store. Do not re-land without repeating that A/B.
- Kept from the same session: the **UMLS grammatical-surface filter** (17 surfaces incl. `does not`/`alone`/
  `lead`) in `crates/eigenius-umls/src/convert.rs` — that one is a keeper.

#### GOTCHAS (both cost real time — read before measuring)
- **Counting.** `summarize()`'s per-unit listing enumerates **only AMBIG units**; grammar-gaps print in a
  different format, so grepping `[AMBIG` **silently misses every gap**. Count from the
  `=== WRN first page over FULL lexicon: … grammar-gap N …===` summary line (or the `[unit N] … TAG` lines).
- **Snapshot drift.** A bootstrap-ontology edit changes its content hash, so older snapshots **ManifestDrift** —
  and the harness **SKIPs fail-closed while reporting `ok`**: every `db_backed_encoding` test goes green doing
  nothing. Latest drift: the `dcg-cleanup` merge declared `conn_list` on `lexicon:Conn` (`2026-07-15`), retiring
  the 07-12 snapshots. Two `2026-07-16` bootstrap edits invalidated the chain in turn: the
  definite-referential fix (axiom `ontology:the`) and then the quantifier-determiner fix
  (`several`/`many`/`few`/`most`/`both`). **Current resumable snapshot:
  `wordnet-umls-aligned-v3-2026-07-16-quant`** (reseed `--umls-all` + v3-align, 2.7 GB). Always drive the
  measurement through `scripts/measure-parse-rate.sh` (it sets `EIGENIUS_DB_SNAPSHOT` to the newest
  snapshot); the harness fallback `DEFAULT_SNAPSHOT`
  (`crates/eigenius-wordnet/tests/db_backed_encoding.rs:64`) now points at it.

#### Follow-up spun out of the faithfulness work (not started)
**Pre-nominal `only` / `just`.** Same `ontology:sole` operator (already in the ontology), but they attach
**outside the determiner** ("only [this event]") — NP-level focus, a different rule from `alone`'s N-level
refine (an NP-level rule must reach into the generalized quantifier's restrictor). Deliberately deferred rather
than shipping a mis-shaped N-level `only` that would only cover "the only X". Small, self-contained.

### 2. [d63-next-steps.md](d63-next-steps.md) — the D63 pipeline spine (the base)
The overall sequence that (1) is a detour from. Remaining once (1) pops, in order:
**address ambiguity** (0 encoded → clean single parses) + long-sentence perf → **grading-phase gaps**
(Citation grade-climb; graded-props run over the full lexicon, persistent doc layer) → **Phase 2**
(orchestrator / served path). The Phase-1 machinery (reshape, pipeline, grader, ingestion, D47 codec) is
done.

---

## On deck (pushed onto the stack when its step becomes active)

- **Reseed OOM — memory profiling follow-up** ([reseed-oom-memory-investigation.md](reseed-oom-memory-investigation.md)).
  **⚠ POSSIBLY STALE — verify before picking up.** A full `scripts/reseed-lexicon-db.sh --umls-all` ran to
  **completion on `2026-07-10`** (exit 0, 2.9 GB snapshot `wordnet-umls-all-alone-2026-07-10`), i.e. the claim
  below that it "blocks any fresh full reseed" no longer reproduces — likely superseded by the `--out-dir`
  chained-load path. Re-confirm the OOM still happens before investing in the profile.
  *Original:* Full WordNet+UMLS reseed OOMs (~20 GiB) deep into the UMLS load; blocks the at-scale
  re-verification of C3-precision (and any fresh full reseed). Static analysis is exhausted (named resident
  terms sum to ~5–7 GiB vs the 20 GiB OOM; the note's §3 lists what is measured-out — text index, RocksDB
  config, in-memory backend, bounded cache — do not re-tread). **Next action: the jemalloc heap profile in §6**
  (feature-gated `tikv-jemallocator` on `eigenius-cli`, bounded native `serve` + ~10 UMLS chunks + `jeprof`
  flame graph) to name the ~15 GiB owner. Diagnostic already in tree:
  `storage/rocksdb/tests/snapshot_memory_probe.rs`.

- **Phases 3 (ambiguity) + 4 (performance)** — one root cause, worked together once phase 2 pops.
  Concrete first lever: the **mass-shim precision fixes** (d63-parse-gap-closure.md §6 — strictly-
  uncountable-head test + acronym↔domain-word collision filter) to kill the spurious `mass` readings that
  inflate BOTH the reading count (median 105/unit, capped at 256) AND parse time (up to 930 s/unit).
  Backstop = [d63-parsing-scale-and-pruning.md](d63-parsing-scale-and-pruning.md) — the CKY
  chart-explosion sub-project (adaptive supertagging + **intermediate-cell** felicity pruning; GH#97) —
  becomes the top entry when phase 4 is active. The reranker (`--features use-llm`) is the phase-3
  AMBIG→ENCODED metric.

## Parked tracks (real, but off this stack)
Separate threads, not blocking the parse→encode pipeline; pull onto the stack only if picked up:
- **GH#104 — NbE readback panic** (`readback.rs:38`): surface `cell` resolves to UMLS **gene** concepts
  `C1413336`/`C1413337` (TUI **T028**), which are then **applied as functions** → `NotAFunction(ResourceVal(…))`.
  **Pre-existing** (48 panics on the pre-`alone` baseline, 32 on current HEAD — recent work reduced, did not
  cause it) and caught per-candidate, so **no unit is lost** and the sweep still completes. But an ill-formed
  term is reaching readback, so the defect is at the **construction site**, not readback; the `.expect()` is
  also the wrong failure mode. Off the critical path.
- **GH#103 — `CompleteJson` intermittently fails** ("No object generated: could not parse the response",
  patent-analysis notebook). Ruled out: the `main` merge (website-only), reseed/schema explosion (schema is
  class-derived, not chain-derived), `max_tokens` truncation (standalone repro used 304 of 2000 tokens).
  Two real findings: (a) the catch block discards `NoObjectGeneratedError`'s `finishReason`/`usage`/raw `text`,
  making every recurrence undiagnosable; (b) `orchestration/deno.json` pins `ai`/`@ai-sdk/anthropic` to
  **`@latest`** and the Dockerfile never copies `deno.lock` — the container has drifted **two majors**
  (`ai` 6.0.158→7.0.19, `@ai-sdk/anthropic` 3.0.69→4.0.11) and re-resolves on every restart, so local ≠ prod.
- [d61-llm-based-encoding-methodology.md](d61-llm-based-encoding-methodology.md) — grounding-discovery +
  typed decision-making layer (the D61 plan).
- Benchmark pilot (D50/D51) — chem+bio; kernel gaps done, infra gaps remain.
- [d63-passive-voice-handling.md](d63-passive-voice-handling.md) — general passive-voice infrastructure:
  object→subject promotion + agent suppression + `rel(theme, ground)` roles (importer `cat_pss` / a grammar
  passive rule). Serves the denominal phrasal half **and** ordinary passive clauses (`were represented by`,
  `is associated with`, … — in the current grammar-gap list). **Trigger:** closing passive clauses on the
  page, or the denominal phrasal half.
- [d63-denominal-suffix-alignment.md](d63-denominal-suffix-alignment.md) — the **spec**: the
  `DenominalElement` table + the `⟦X-E⟧ = ⟦E link X⟧` alignment invariant for the denominal-adjective suffix
  class (`-based`/`-like`/`-mediated`/…). The **compound half is DONE** (compound-morphology §3b, shipped
  `2026-07-05`); the **phrasal** half → d63-passive-voice-handling.md. **Trigger:** after the phrasal half
  lands, to gate the `X-E ≡ E link X` equivalence.
- [d63-lexicon-augmentation.md](d63-lexicon-augmentation.md) — the `DocumentPipeline` generalization for
  **lexical gaps**: `AbbrDef → LexicalBinding{surface, long_form?, grounding}`, the pipeline as a
  lexicon-augmentation transducer (`AugmentOptions`/`LexiconProfile`/seed-in-added-out + the feedback cache),
  two-moment grounding with the concept-convergence invariant (`RecQ DNA helicase → C0084304`). **Trigger:**
  generalizing Stage A / closing `recq` via retrieval-grounding; needs the gene-family source
  ([[gene_family_lexicon_gap]]) + a lexicon/ontology index.

## Completed (record, not work)
- **Phase-2 constructions, Step 5/5b/5c — COMPLETED `2026-07-06`** (uncommitted on `13c5bbe` + the
  refactor on top). RC-6 apposition (`appose_group`, bidirectional concept↔semantic-type felicity),
  comma-list connective inheritance (neutral comma finalized by the trailing `and`/`or`), and the
  **coordination refactor** to core-en's list-with-operator shape (`cat_coord` + `coordinate_prop` +
  `complete_coord`, retiring the eager `coordinate_sem` + the Step-5b n-ary workaround). Together −8
  grammar-gaps (20→12). Kernel lib 1611 + `closed_class` 126 green. Detail in d63-parse-gap-closure.md
  §4 Steps 5/5b/5c.
- [d63-compound-morphology.md](d63-compound-morphology.md) — **COMPLETED `2026-07-05`.** Derived-adjective
  OOV closed (Slices 1–2 + §3b denominal-suffix table + `-like` fix); missing-lexeme 6 → 2 over the
  snapshot. Deferred pieces extracted to the parked tracks above (alignment / passive-voice) and the
  gene-family track ([[gene_family_lexicon_gap]] — `recq`).

## Reference / design notes (consulted, not "work")
Not stack items — background for the above: `d63-{document-preprocessing-scope, kind-predication-reshape,
coren-coupled-port-design, pp-attachment-control-scoping, packed-forest-parsing-blueprint,
cnl-*}`, `d62-*`. Pull in when a step needs them.

---

### Maintenance
- Finishing the top's exit-gate → delete/collapse its entry and promote the next. Note the pop here.
- A new sub-task splitting off the active entry → write its note, push it as the new §1, demote the rest.
- This file is the index; the per-note detail lives in the linked notes, not here.
