# D63 — Collapsing the domain-compound pile (plan of attack for the 3 residual gaps)

**Status:** plan / pre-implementation. Targets the 3 residual reranked gaps (#3 passive, #4 V-as-Y +
compared-to, #7 comparative + PP) that survive after lexicalize + build-then-subsume + reranker +
count-veto. Grounded in the re-assessment (`diagnose_residual_gaps`, `db_backed_encoding.rs`) that
**refuted PP-attachment as the lever** ([d63-pp-attachment-control-scoping.md](d63-pp-attachment-control-scoping.md),
shelved) and located the driver in **domain-term ambiguity — sense-product × N-N compound bracketing**.

## 1. What we know (grounded)

- **Grammar is complete for these three.** Every construction fragment parses in isolation with generic
  fillers — the passive + coordinated subject (`some lines and some lines were represented by data sets`
  CLOSED×6), V-as-Y + both PPs (`…as a dependency in cells compared to lines` ×121), comparative + PPs
  (`cells from lineages showed greater dependence on genes than counterparts` ×162). Adding a PP *raises*
  the reading count, never gaps. The gap appears **only** when generic fillers are replaced by the domain
  terms (`MSI cell lines`, `MSS counterparts`, `screening data sets`, `WRN`, `these four lineages`).
- **The pile is ~6 structural shapes × sense-product per shape**, not one or the other (v3 S5
  `analyze_chart_cells`: the saturating cells are `kept=432 shapes=6`, `kept=184 shapes=6`). So there are a
  handful of distinct `cat_shape`s (structural), each holding a large sense-product (same-shape).
- **The two collapse mechanisms already built hit different halves.** Packing
  ([packed-forest blueprint](d63-packed-forest-parsing-blueprint.md), default on) collapses the
  *same-shape* sense-product to O(nodes) (~8× measured) but **not** the distinct shapes. Build-then-subsume
  (D3) drops definitionally-equal readings post-felicity. Neither collapses distinct *structural* shapes.
- **The explosion is the CKY cross-product across the compound spans** (items²-per-split), amplified by the
  domain terms' sense-product and multi-noun brackets — 10.8M chart items on #7, OOM at cell_beam=1024.

## 2. Step 0 RESULT — the corpus NEVER packs (Derived, `2026-07-09`, `diagnose_compound_pile`)

`parse_needs_unpacked` routes the **whole** sentence off the packed path if any seeded span has a
**concrete non-Entity selectional slot** (`cat_np(SpecificClass)`, `slot_is_concrete_nonentity`) or is
pied-piping. **Measured (`routes_packed` over the count-veto snapshot): EVERY frame routes UNPACKED —
including trivial `genes affect cells`, `genes are large`, `genes are attractive targets`, and all three
residual sentences and their generic bases.** So it is **not** the comparative/passive/V-as-Y constructs —
**on the full lexicon the packed path is never taken at all.**

**This is the headline finding, and it reshapes the plan.** The measured ~8× packing win
([blueprint §10b](d63-packed-forest-parsing-blueprint.md)) was validated on *small-lexicon demo*
sentences; on the real corpus the dense lexicon means some sense of some common word always carries a
concrete selectional slot, and the router's **whole-sentence** rule unpacks everything on that single hit.
So the sense-product piles are **never** collapsed by packing — which is why even the generic bases run at
×121–162 and the domain terms tip them over. `self.packing` is on; the router is the sole cause
(`routes_packed = packing && !combinatory_core && !parse_needs_unpacked`).

**Consequence: Lever 1 (fix the whole-sentence router → per-cell packing) is THE lever — and it is
corpus-wide, not just the 3 gaps.** It would recover the packing win for *every* corpus sentence, collapsing
the sense-product piles that drive both the residual gaps and the high AMBIG generally. Lever 2 (collapse
structural shapes) drops in priority — packing is not even running, so "the residual is 6 structural shapes"
was a false premise; the real residual is the *uncollapsed sense-product on the unpacked path*.

## 3. Levers (Step 0 confirmed UNPACKED → Lever 1 is THE lever)

### Lever 1 — Per-cell packing (CONFIRMED corpus-wide) ★
Today `parse_needs_unpacked` is **whole-sentence**: one selectional slot anywhere unpacks everything,
including the index-independent noun-compound sub-cells that are the actual pile — and Step 0 showed that
on the full lexicon this fires on *every* sentence, so packing never runs. **Fix: pack the safe sub-cells,
unpack only the slot-bearing spans** — the noun compounds (`MSI cell lines`, `screening data sets`) carry
no selectional slot and are soundly packable even inside a sentence whose verb selects. Recovers the 8× on
exactly the spans that explode, on every corpus sentence.
- **Step 1 RESULT (Derived, `2026-07-09`, `EIGENIUS_ROUTE_DEBUG` on `diagnose_compound_pile`).** The
  offending category is the **same on every noun** — the object-position type-raised existential-GQ seeded
  on the bare plural (via the existential det-form, `lookup.rs:855`):
  `(S\NP)\((S\NP)/cat_np(<the noun's own synset>))` — e.g. `genes`→`…/cat_np(n05436752)`,
  `cells`→`…/cat_np(n00006484)`, `lines`→`…/cat_np(n00582388)`. Its argument slot is the noun's **own
  concrete class** (not `Entity`), so `cat_has_selectional_slot` fires and the whole sentence unpacks.
  Every noun carries it (object-position quantified NPs — `affect genes`, `represent lines` — need it), so
  **every** sentence trips the whole-sentence router even when the object-GQ reading is never used (`genes`
  is the *subject* in `genes affect cells`).
  - **Verdict: legitimate slot, not a spurious sense — so Lever 1 is per-cell packing, NOT a
    source/router-precision tightening.** The concrete slot is *semantically load-bearing*: it records
    which class fills the object (so `affect genes` denotes gene-object semantics), and combines with the
    generic verb `(S\NP)/cat_np(Entity)` only by contravariant subsumption (`gene ⊑ Entity`). Widening it
    to `Entity` would erase the object's type; it can't be tightened away. Packing by `cat_shape` (which
    erases `cat_np(gene)`→`cat_np(_)`) is therefore genuinely unsound *for the object-GQ item* — two nouns'
    object-GQs share a shape but combine/denote differently. The router is right to distrust it; it is
    **wrong to unpack the whole sentence** over it.
  - **So the object-GQ is a small, per-cell *unpacked residue*, not the pile.** Within each cell, the
    index-**in**dependent items — the plain NP, the compositional compound readings, the whole sense-product
    that is the actual explosion — are soundly packable; only the handful of concrete-slot object-GQ (and
    pied-piping) items must stay unpacked. Per-cell packing packs the pile and unpacks the residue. The fix
    is exactly Lever 1 below.
- Touches: `parse_needs_unpacked` (per-cell, not per-sentence); the packed-forest construction to mix
  packed sub-cells with unpacked slot-spans; the differential oracle (extend to mixed sentences).
- Risk/cost: the packed/unpacked boundary bookkeeping is the real work; the soundness precondition
  (index-independence of the packed sub-cells) is already the packing invariant, so no new unsoundness.

- **Step 2 — IMPLEMENTED (Derived, `2026-07-09`).** The fix is cleaner than "mix packed and unpacked
  sub-cells": per-cell packing falls out of a **packing-signature refinement**, so there is one packed
  forest, not a packed/unpacked split.
  - `node_sig` (`packed.rs`) now keys an item by `cat_shape` (indices erased — the coarse key that
    collapses the sense-product) **unless** its category has a concrete selectional slot
    (`cat_has_selectional_slot`), in which case it keys by the full category (`cat_key`, new in
    `pretty.rs`, prefixed `sel:`). So two object-GQs of different classes never share a node; the
    index-independent majority (the actual pile) still packs by `cat_shape`. The object-GQ is the small
    per-cell unpacked residue, exactly as Step 1 predicted.
  - The router's whole-sentence selectional carve-out (`parse_needs_unpacked` clause 2) is **removed** —
    concrete slots are sound per-cell now. Only the pied-piping **completeness** carve-out remains (the
    packed forest builds no edge for that ternary construct).
  - Removing the carve-out exposed one construct the selectional carve-out had incidentally been
    protecting: **close nominal apposition** (`the genes BRCA1 and MSH2`), which the packed forest did
    not build. Built it in as an `ApposeGroup` binary edge over adjacent splits (mirrors the unpacked
    CKY) — the structural completion, not a re-carve-out.
  - **Soundness witnessed** by the differential oracle `packed_forest_equals_unpacked_on_core_grammar`,
    extended with selectional (`depends on`, object-GQ) and close-apposition sentences: packed ≡ unpacked
    (closed forests + open counts) on all of them. Full kernel suite green (1605 + 135), `fmt`/`clippy`
    clean.
  - **Corpus witness — routing:** every corpus frame now routes `[PACKED]` (was `[UNPACK]` on every
    frame at Step 0), including all three residual-gap sentences.
  - **Corpus witness — deterministic cap-only sweep** (count-veto snapshot, no reranker): grammar-gap
    **7 → 2**, **zero new gaps** (strict subset — no regression), **5 closed** (incl. the #3 data-sets
    sentence, the #7 `greater dependence on WRN` sentence, `These observations suggest…`, `WRN dependency
    may require…`, `We hypothesized… synthetic-lethal`). The #7 sentence had previously OOM'd at
    `cell_beam=1024`; packing collapses the sense-product so it parses. Remaining 2 gaps: the
    `Project Achilles and project DRIVE identified WRN as…` sentence and `The MSI relationship compared
    favourably…`.
  - **Corpus witness — reranked sweep** (`--features use-llm`, non-deterministic): grammar-gap **3 → 1**
    (closed `MSI cell lines from these four lineages showed greater dependence on WRN than their MSS
    counterparts` and `Some MSI lines and some MSS lines were represented by these screening data sets`).
    The `encoded 1 → 0` / `open 0 → 1` shifts are within the reranker's per-unit non-determinism (the
    deterministic cap-only sweep above is the no-regression proof), not lost readings.
  - **Remaining gap (both sweeps agree):** `Project Achilles and project DRIVE identified WRN as the top
    preferential dependency in MSI cell lines compared to MSS cell lines` — the hardest unit, stacking
    `MSI cell lines` + `MSS cell lines` + `top preferential dependency` + `compared to` in one 20-token
    clause. Per §6's realistic fallback, one search-limited gap on a triple-compound clause is a
    legitimate stopping point; Levers 2/3 (shape collapse / beam headroom) remain if we choose to chase it.

### Lever 2 — Collapse the residual structural shapes (if PACKED, or as the second pass)
The ~6 shapes are the structural variants packing can't merge. Expected sources (confirm in Step 0):
- **(a) unit-vs-compositional.** A domain compound that is *also* a lexicon unit (`cell line`, `data set`
  are UMLS/WordNet units) parses BOTH as the unit AND as `[cell][line]` compositional → distinct shapes.
  **Fix: prefer the lexicon-unit reading** — the same lexicalization principle as hyphenate/inject
  (d63-nominal-modification §4). A multiword-unit span, when present, suppresses the compositional
  re-bracketing. Highest-leverage if Step 0 confirms it dominates.
- **(b) compound nesting.** The left-branching NF (`is_compound_refined`, `parser.rs:392`) forces
  left-branching for the *head*, but the v3 S5 profile still showed single-vs-nested `compound_kind`
  variants. **Fix: tighten the NF** so a 3+-noun compound collapses to exactly one tree (extend the
  existing guard; this is the *N-N* residual, distinct from the §3.3 adjective interleaving that was a
  no-op).
- Touches: `parser.rs` compound rule + `is_compound_refined`; the MWE-vs-compositional seeding in
  `lookup.rs` (§8.4) for (a).

### Lever 3 — Beam headroom (the marginal closer)
The generic fragments already run at ×121–162 — right against `DEFAULT_FOREST_CAP` (256) — so the domain
mass tips them over. After Levers 1/2 thin the pile, a small headroom bump likely closes the residual.
- Options: a targeted `cell_beam` raise on compound-heavy spans (adaptive, not global); or extend the
  widen-on-failure escalation ladder; or a modest `DEFAULT_FOREST_CAP` bump. **Do this last** — raising
  the beam before thinning the pile just moves the OOM.
- Touches: `with_cell_beam` / the widen ladder / `DEFAULT_FOREST_CAP` (`lookup.rs`).

## 4. Sequencing

1. **Step 0** — confirm routing + shape profile per sentence (cheap, bounded; no code).
2. **The dominant lever** — Lever 1 if unpacked, Lever 2(a) if packed-and-unit-driven. One at a time,
   re-measure after each.
3. **Lever 2(b)** — tighten the N-N nesting NF if shapes remain.
4. **Lever 3** — beam headroom to close the marginal tail.

## 5. Verification (per lever)

- **Deterministic:** `diagnose_residual_gaps` — the 3 full sentences move GAP → CLOSED/open at the default
  beam (the fragments already parse, so any change is the domain-term pile shrinking, not grammar).
- **Full page:** cap-only sweep (no-regression diff vs the current snapshot — *zero* new gaps) + reranked
  tally (GAP 3 → target 2/1/0).
- **Battery:** closed-class/determiner + the differential packing oracle (`packed ≡ unpacked`) stay green —
  the soundness gate for Lever 1.
- **Soundness:** no reading lost that a slower/unbounded parse would find — Lever 1 preserves it by
  construction (packing is exact on index-independent cells); Lever 2(a)/(b) must be witnessed as
  meaning-preserving (a lexicon unit ≡ its compositional reading; a left-branching tree ≡ the alternatives
  for these compounds).

## 6. Non-goals / risk log

- **Not PP-attachment control** — refuted (shelved note); the PPs parse.
- **Not the §3.3 adjective NF** — no-op for this corpus (gradable adjectives).
- **Realistic fallback:** these are 16-token sentences stacking 2–3 domain compounds; grammar is complete
  and 59/62 parse. If Levers 1–3 prove disproportionate, **accepting the 3-gap search-limited tail is a
  legitimate stopping point** — record it, don't grind. The value of this plan is bounded by whether Step 0
  shows a clean dominant lever.
