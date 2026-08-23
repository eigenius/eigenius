# Reseed timings — `2026-08-23`, the eigenius#188 / N4 reseed

The first `--umls-all` reseed since `2026-07-27`. Recorded because the P2 work changed the commit-time
verification path, and the question "did that cost anything" deserves a number rather than an
impression. Companion to [lexicon-load-benchmarks-2026-07-27.md](lexicon-load-benchmarks-2026-07-27.md),
whose measurements this compares against.

Command: `scripts/reseed-lexicon-db.sh --umls-all`, from `d03d3ee`. Snapshot
`../db-snapshot/wordnet-umls-2026-08-23`.

## 1. Headline

| | 2026-07-27 | 2026-08-23 | delta |
|---|---|---|---|
| wall clock | 1,989 s (33 m 09 s) | **2,080 s (34 m 40 s)** | +91 s (+4.6 %) |
| resources | 9,192,394 | **9,439,633** | +247,239 (+2.7 %) |
| loads | 34 (4 WN + 30 UMLS) | **35 (4 WN + 31 UMLS)** | +1 |
| store | 3,316,537,602 B (3.09 GiB) | **2,868,699,807 B (2.67 GiB)** | −447,837,795 (−13.5 %) |
| density | 361 B/resource | **304 B/resource** | −57 (−15.8 %) |
| whole-run average | 4,622 res/s | 4,538 res/s | −84 |
| validated commits, late chunks | 5,227 res/s | 5,777 res/s | +550 |
| errors | — | **0** | |

Wall clock is flat once the extra 2.7 % of data and the extra load are accounted for. Peak kernel RSS
15.8 GiB of 31.1 GiB, and it FLATTENED over the last ten commits (15.16 → 15.77 GiB) rather than
climbing — the shape the `2026-08-03` profile warned about did not recur.

**The density change is not explained here.** A 15.8 % drop in bytes per resource while carrying more
resources is a real change with no established cause. It is not the eigenius#188 retype: `type_name`
and `param_kind` appear only on inductive declarations, of which these chains contain none (§3).
The drop set and atom overrides have both moved since July, which would change content and chunk
count together, but that is a hypothesis and nobody has checked it.

## 2. Per-commit durations

32 of 36 captured — the container is removed at teardown, so the last four commits' `duration_ms`
were lost with its logs. Capture them next time before `docker compose down`.

```
WordNet   17.5  34.5  59.1   9.7        (4 chunks)
base       0.04                          (128 resources)
UMLS      37.7  38.5  37.8  42.5  42.0  47.4  59.0  54.9  50.6  41.0
          47.8  45.4  45.7  53.4  52.1  51.1  52.7  54.3  57    56
          61    65    64    61    54    59                      (26 of 31 captured)
```

Mean 41.8 s over the first 22; settling around 55–60 s by commit 32. The O(chain) term is present and
mild — roughly +55 % from first UMLS chunk to last captured, over a 30× growth in chain length. That
is the sublinear behaviour the partitioned-load work established, not the growth the earlier
`umls_load_scaling` finding described.

## 3. What this run does NOT measure

**It does not measure the new declaration gate**, and the number above should not be cited as if it
did. Rule 23 (`validation/rules/inductive_decl.rs`) routes every `core:InductiveType` through
`check_type`, which since eigenius#188 also walks both telescopes and applies the
constructor-argument universe constraint. Measured against the chains this reseed loads:

```
$ grep -c "^data " umls-chain/*.esl wordnet-chain/*.esl   → 0
$ grep -c "core:InductiveType" umls-chain/*.esl …          → 0
```

Zero. The chunks declare `resource` (241,113 in the first UMLS chunk) and `class` (22,911) only, so
Rule 23 returns at its first line for all 9.4 M resources. The gate runs on the bootstrap's 42
declarations and nothing else in this run.

Rule 21 *does* run, on 968,108 `type_expr` values in a single WordNet chunk alone — but the
eigenius#188 change there was to DECLINE two properties (`param_kind` / `type_name`) that lexical
entries do not carry, i.e. an early return, and Rule 21 reaches `check_infer` directly rather than
through `check_type`'s changed default arm.

**To actually measure the gate**, the population is inductive declarations and the bootstrap has 42 —
a ~2 s test, not a 35-minute reseed. The telescope walk is a full `check_type` per declaration where
it was `check_positivity` alone, so it is not free; it is invisible at this scale.

## 4. Where the gate WAS exercised: the WRN demo

`demo/wrn-helicase/run.sh`, run after this reseed on a clean volume: **56 `Holds`, 0 `Fails`,
0 errors**, all six steps including every wrapped-R warrant.

Unlike the lexicon chains it loads **46 inductive declarations** (`experiments/publications/wrn-helicase`,
the `onco:` predicates), and they are the shape that matters:

```
data onco:TopDifferentialDependency : core:string -> core:string -> Prop
```

An INDEX telescope of `core:string` — precisely where `decode_indices`' `_ => "urn:eigenius:core:Set"`
fallback lived (§1). Before the fix those indices decoded to `EigonClass(core:Set)`, a class type
nothing can inhabit. All 46 admitted through the new gate.

All 46 conclude in `Prop`, so the constructor-argument universe constraint takes its impredicative
exemption on every one. The constraint is therefore exercised as *not firing*, which is correct
behaviour rather than a coverage hole — but it means the only chain-resident declaration that has
ever tripped it is still `reasoning:JustifiedBy`.
