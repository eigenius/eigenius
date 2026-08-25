# Reseed — `2026-08-25`, the D76 Phase E2 chain-format gate

Run to validate **Phase E2 / #188 slice 5** — universe polymorphism reaching the language. Unlike the
`2026-08-25` Phase-B reseed earlier the same day, this one carries a **chain-format change**: the core
ontology declares `core:universe_params`, and `ConstRef` gained an optional trailing level-argument
list.

Command: `scripts/reseed-lexicon-db.sh --umls-all --snapshot-dir wordnet-umls-2026-08-25-e2`, from
`b0127d4`.

## 1. Result: exact parity, per layer

| | Phase B (`406f0b2`) | Phase E2 (`b0127d4`) | delta |
|---|---|---|---|
| resources | 9,439,633 | **9,439,633** | **0** |
| loads | 35 | **35** | 0 |
| **errors** | 0 | **0** | **0** |
| store | 2,884,062,153 B | 2,741,105,501 B | −142,956,652 (−5.0 %) |
| wall clock | 3,635 s | **1,740 s** (29 m 0 s) | −1,895 s |

**All 35 per-layer counts matched individually**, not just the total — the check that a redistribution
between layers cannot pass.

**This is the evidence for the optional-trailing-argument design.** The new `ConstRef` decode path
accepts one *or* two arguments; every persisted term in a 9.4M-resource chain took the one-argument
branch. Monomorphic references are byte-identical to what shipped before, which is what kept this a
*comparison* rather than a wholesale rewrite in which a regression could not have been noticed.

Layer hashes all differ, as they must: the bootstrap moved, so every layer rooted on it re-hashes.
Count is the parity signal; hash is not.

## 2. The `2026-08-24` timing anomaly is settled

That run took 3,635 s against a 2,173 s baseline (+67 %) and the note recorded it as *"host-side
conversion or machine conditions, not commit-time validation"*, on the strength of a back-to-back
per-layer A/B (72 s pre-Phase-B vs 73 s post).

**This run is 1,740 s — faster than the 2,080–2,189 s band that note cites for the same code run
twice**, on strictly more kernel work. That closes it: the +67 % was machine conditions, and no part
of Phases B–E2 costs load time.

## 3. Store size: −5.0 %, and still unexplained

Density 305.5 → 290.4 B/resource over identical input. The `2026-08-23` note recorded an unexplained
15.8 % drop, `2026-08-24` a 1.5 % rise, this one a 5.0 % drop — three data points, no direction, no
mechanism. Plausibly RocksDB compaction nondeterminism; still nobody has checked, and nothing is
claimed from it.

## 4. Still not measured: §4.2's memo boundedness

Unchanged from the earlier note. `GlobalMemoScope::entry_count()` was built for this and remains
unreadable from a containerized kernel — no reporting path exists. The claim that the memo's key set
is bounded by IRIs *appearing in terms* rather than by chain size is an argument plus a unit test, as
it was before either reseed.

## 5. Downstream gates — all pass

**Alignment** (`wordnet-umls-aligned-2026-08-25-e2`): 40,357 entries redefined, 38,389 merge rows,
plus the 5-resource `claim-kind-alignment` layer. Every count identical to the recorded baseline; the
merge set is unaffected by the format change.

**WRN demo** (`demo/prose-to-formulas-v2/run.sh`), both acceptance checkpoints:

- intact — `✓ COMMITTED`, `RequiresActivity(MSI, WRN)` justified twice (asserted, and derived from
  measurement + published rule);
- edited — `✓ REJECTED`; the script's `✗ UNEXPECTED … exit 1` path was not taken.

The edited case is the one E2 could plausibly have broken. The witness key hashes the **proposition**,
so had the encoding change perturbed how terms hash, either the asserted route would have failed or
the derived one would have wrongly survived. Neither did.

**Parse gate**, `--replay experiments/parsing/ranks/2026-08-22-productiontrace.json` against the new
aligned snapshot. Deterministic by construction, which matters here: `baseline.json` records that *a
post-reseed A/B against another branch is impossible — the bootstrap manifest welds code to snapshot,
so the control binary cannot open the snapshot*. Replay is the only instrument that removes reranker
drift, and it costs no model calls.

```
COVERAGE: PASS — every unit parses (grammar-gap 0, missing-lexeme 0)
SELECTION-VALIDITY: PASS — no invalid-adjudicated skeleton was selected
expected-hits 62/62 → 62/62, miss-set unchanged
```

**0 malformed replies, 0 replay misses** — the #212 check the baseline's method note requires, and
evidence that the recorded ranks still key correctly against the rehashed chain.

**Two baselines are in play, so be precise about the deltas.** `total-readings 674` reproduces the
replayed draw's own recorded value **exactly**; the `613` the evaluator prints is a *different* draw
(the d73 batch). `encoded` has no recorded value for the replayed draw at all, so `2 → 1` is measured
against the wrong control and the evaluator marks it ungated. **The only clean delta is
skeletons 170 → 171.**

**+1 skeleton is the direction Phase D predicts, stated as expectation and not as proof.** Completing
the `Refine` rule means entailment now accepts subtyping that set inclusion rejected, so a reading
that previously failed to type-check can survive — more distinct skeletons, not fewer. The
`encoded 2 → 1` / `ambiguous 40 → 41` movement is the same shape, and the history records it before:
*"encoded 11 -> 2 and ambiguous 31 -> 40, because more senses survive the cap so fewer units collapse
to one reading unaided"*. Which unit moved, and whether entailment is why, is untraced — the metric is
ungated and coverage holds, so buying that proof would cost a live-draw campaign for no decision.

## 6. Snapshot naming

Named explicitly (`--snapshot-dir wordnet-umls-2026-08-25-e2`) rather than letting the script date it.
The `2026-08-24` run crossed midnight UTC while the local clock read the 24th and **overwrote its own
baseline**; naming it here means the Phase-B store survives for comparison.
