# Reseed — `2026-08-24`, the D78 Phase D parity gate

Run to discharge D78 Phase D's outstanding gate: the validator's membership check now derives the
required field set from `resolve_class_type`'s record rather than from its own transitive walk, and
§7 gates that on **verdict parity over 9.4M resources**. The four unit gates cover the core and
animals ontologies only.

Command: `scripts/reseed-lexicon-db.sh --umls-all`, from `4ba900a`.
Snapshot `../db-snapshot/wordnet-umls-2026-08-24`.

## 1. Result: parity holds

| | 2026-08-23 (`d03d3ee`) | 2026-08-24 (`4ba900a`) | delta |
|---|---|---|---|
| resources | 9,439,633 | **9,439,633** | **0** |
| loads | 35 (4 WN + 31 UMLS) | **35** | 0 |
| **errors** | 0 | **0** | **0** |
| store | 2,868,699,807 B | 2,912,381,027 B | +43,681,220 (+1.5 %) |
| density | 304 B/resource | 308.5 B/resource | +4.5 |
| wall clock | 2,080 s (34 m 40 s) | 2,173 s (36 m 13 s) | +93 s (+4.5 %) |

**The parity check is exhaustive here, not a sample.** The baseline was zero errors over this exact
input. Zero errors on the new code therefore means no verdict moved pass → fail, and none could move
fail → pass because there were none to move. Phase D's gate is met.

## 2. Cost: inconclusive, and that is the honest reading

+4.5 % looks like a regression, and it is not distinguishable from noise. The previous session ran
this reseed **twice on the same code**, at 34 m 40 s and **36 m 29 s** — a spread of 1 m 49 s. This
run's 36 m 13 s sits inside that band, slightly below the slower of the two.

So the measurement cannot separate "the class-keyed memo costs ~90 s" from "this is the same run
twice". Establishing which would take repeated runs and hours; it is not worth it at this stage:

- the memo's **bound** is proven by test, not by this timing —
  `the_memo_is_bounded_by_distinct_classes_not_by_resources` shows 500 resources over one class
  leaving ≤4 entries, so it grows with the ontology's class count and not with the chain;
- the per-resource work it replaced (`resolve_class_type` uncached) would not have been a 4 % effect.
  It would have been catastrophic, and it is not present.

**Do not record this as "Phase D costs 4.5 %".** It is "within the observed spread of the same
measurement".

## 3. Store size: +1.5 % at identical resource count, unexplained

Density moved 304 → 308.5 B/resource over byte-identical input. Plausibly RocksDB compaction
nondeterminism; nobody has checked. Recorded rather than theorised — the `2026-08-23` note carries an
unexplained 15.8 % density *drop* in the other direction that was also never chased, so a ±few-percent
band here has no established meaning yet.

If a third data point is ever wanted, that is the measurement to make: same commit, two reseeds,
compare store size. It would settle both this and §2.

## 4. What this run does *not* cover

- **Rules 4–7 and 10** (format, pattern, range, length, domain) were untouched by Phase D (D78 §5.1),
  so this says nothing about them beyond that they still pass.
- **Phase E** is not in this build. `Construct` still returns `Val::EigonClass`, and a resource's own
  record does not exist yet.
- Per-commit `duration_ms` was not captured again — the container is removed at teardown and takes
  its logs with it. The `2026-08-23` note left the same reminder; capture before
  `docker compose down` if per-chunk timings are ever wanted.
