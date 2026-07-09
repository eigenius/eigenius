# Work stack — unfinished work (top = active)

The single "where are we" pointer. A **LIFO stack** of the active working notes: work the **top** entry;
when its exit-gate is met, **pop** it and the entry below becomes active. When a sub-task splits off from
an entry, **push** its note on top. Keep this file current — it is the map back to the base plan after
any detour.

---

## Stack (top → bottom)

### 1. ▲ ACTIVE — [d63-parse-gap-closure.md](d63-parse-gap-closure.md) — **Phase 3 of 4: ambiguity / search-scaling**
Four-phase spine (user directive `2026-07-06`, worked in order — stop detouring):
**OOV ✓ → parsing gaps ✓ → ambiguity / search (here) → performance.**
- **Phase 1 (OOV): CLOSED** — `missing-lexeme 0`, distinct OOV 0 (Stage-A augmentation grounds the page).
- **Phase 2 (parsing gaps): CLOSED — grammar-complete (`2026-07-08`).** The canonical config is
  **reranked** (`--features use-llm` + `AnthropicSenseRanker`, snapshot `wordnet-umls-all-2026-07-08`); the
  earlier `--no-llm` counts were cap-only and inflated. Under the reranker the grammar-gap count fell
  **12 → 9 → 8 → 7 → 5**: reranker on (−3), static-rank widen fallback `7d9cda4` (−1), gap #1 bnp
  compound-kind subject `970e9ae` (−1), #5 linking-verb `1cbeeda` + #2 UMLS process-mass `ab6a909` (−2).
  **The 5 survivors (#3/#4/#7/#8/#9) are search-starvation, not grammar** — every construction parses in
  isolation; the full sentence overruns the beam/cell budget. Grammar is complete for this page (§0 of the
  note). Earlier RC-2 comparatives + s20 modal+coordinated-object CLOSED `2026-07-07`.
- **Phase 3 (ambiguity) + Phase 4 (performance): ACTIVE — one lever, scale the search.** The 5 residual
  parse-gaps, every closing unit AMBIG (0 encoded), and the perf outliers all trace to one root cause: the
  **mass-shim over-generation / beam budget** (RC-1 head-inheritance is loose). The search-scaling lever
  (§6 / §7 of the note; **D63 §8.7 / GH#97**) is the whole remaining backlog: an intermediate-cell beam +
  tightening the mass shim should collapse the residual gaps, the ambiguity, and the parse time together.
**Exit-gate (phase 3):** the 5 residual full sentences close under a scaled search AND closing units drop
to a single clean reading (encoded > 0). Then phase 4 (perf) — same root cause — is the residual.

### 2. [d63-next-steps.md](d63-next-steps.md) — the D63 pipeline spine (the base)
The overall sequence that (1) is a detour from. Remaining once (1) pops, in order:
**address ambiguity** (0 encoded → clean single parses) + long-sentence perf → **grading-phase gaps**
(Citation grade-climb; graded-props run over the full lexicon, persistent doc layer) → **Phase 2**
(orchestrator / served path). The Phase-1 machinery (reshape, pipeline, grader, ingestion, D47 codec) is
done.

---

## On deck (pushed onto the stack when its step becomes active)

- **Reseed OOM — memory profiling follow-up** ([reseed-oom-memory-investigation.md](reseed-oom-memory-investigation.md)).
  Full WordNet+UMLS reseed OOMs (~20 GiB) deep into the UMLS load; blocks the at-scale re-verification of
  C3-precision (and any fresh full reseed). Static analysis is exhausted (named resident terms sum to ~5–7 GiB
  vs the 20 GiB OOM; the note's §3 lists what is measured-out — text index, RocksDB config, in-memory backend,
  bounded cache — do not re-tread). **Next action: the jemalloc heap profile in §6** (feature-gated
  `tikv-jemallocator` on `eigenius-cli`, bounded native `serve` + ~10 UMLS chunks + `jeprof` flame graph) to
  name the ~15 GiB owner. Diagnostic already in tree: `storage/rocksdb/tests/snapshot_memory_probe.rs`.

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
