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

## 5. Snapshot naming

Named explicitly (`--snapshot-dir wordnet-umls-2026-08-25-e2`) rather than letting the script date it.
The `2026-08-24` run crossed midnight UTC while the local clock read the 24th and **overwrote its own
baseline**; naming it here means the Phase-B store survives for comparison.
