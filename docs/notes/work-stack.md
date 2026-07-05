# Work stack — unfinished work (top = active)

The single "where are we" pointer. A **LIFO stack** of the active working notes: work the **top** entry;
when its exit-gate is met, **pop** it and the entry below becomes active. When a sub-task splits off from
an entry, **push** its note on top. Keep this file current — it is the map back to the base plan after
any detour.

---

## Stack (top → bottom)

### 1. ▲ ACTIVE — [d63-parse-gap-closure.md](d63-parse-gap-closure.md)
Make the test document (WRN first page, cnl-v2) **parse completely** over the full WordNet+UMLS lexicon.
Baseline measured `2026-07-04`: 62 units → 39 AMBIG, 17 grammar-gap, 6 missing-lexeme, 0 encoded.
Dominant fix = missing **verb + PP-complement frames** (~10 of 17), then 4 OOV + a few constructions.
**Exit-gate:** re-measure → grammar-gap + missing-lexeme = 0 (every unit parses). Then **pop**.

### 2. [d63-next-steps.md](d63-next-steps.md) — the D63 pipeline spine (the base)
The overall sequence that (1) is a detour from. Remaining once (1) pops, in order:
**address ambiguity** (0 encoded → clean single parses) + long-sentence perf → **grading-phase gaps**
(Citation grade-climb; graded-props run over the full lexicon, persistent doc layer) → **Phase 2**
(orchestrator / served path). The Phase-1 machinery (reshape, pipeline, grader, ingestion, D47 codec) is
done.

---

## On deck (pushed onto the stack when its step becomes active)

- [d63-parsing-scale-and-pruning.md](d63-parsing-scale-and-pruning.md) — the CKY chart-explosion /
  sense-crowding sub-project (adaptive supertagging + mid-chart felicity pruning; issue #97). This is
  what the **ambiguity + perf** step of (2) concretely becomes — it becomes the top entry when we reach
  it. Directly relevant to the "0 encoded" and "3–5 min per long sentence" findings from (1)'s baseline.

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

## Completed (record, not work)
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
