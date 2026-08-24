# The parse gate flags REGRESSION on identical code

Recorded `2026-08-24` while gating D78 Phases A–D. Not a D78 finding — a property of the gate that
will keep producing false positives for anyone who runs it.

## What happened

Two live runs of `scripts/measure-parse-rate.sh`, **same build (`ca3b956`), same snapshot
(`wordnet-umls-aligned-2026-08-24`), minutes apart**:

| | baseline.json | run 1 (14:31) | run 2 (14:42) |
|---|---|---|---|
| grammar-gap | 0 | 0 | 0 |
| total-readings | 613 | 613 | 613 |
| **expected-hits** | 62 | **61** | **62** |
| **total-skeletons** | 170 | **175** | **181** |
| **reading-correct** | 30/40 | **27/40** | **26/40** |

**Both runs raised a `REGRESSION` flag, on different metrics.** Run 1 on `expected-hits` (*"a curated
unit lost its expected reading"*, naming «Synthetic lethality is an interaction between two genetic
events.»); run 2 on `reading-correct`. Neither is a code change — there was none between them.

## Why

The reranker prunes senses, so it changes which readings exist, not merely their order. `exp_hits` is
computed over the surviving skeleton set —

```rust
if unit_skel_set(&u.outcome).iter().any(|s| s == &e.skeleton) { exp_hits += 1; }
```

— so a live LLM moves `expected-hits` and `total-skeletons`, both of which read as structural metrics.

This is documented. `experiments/parsing/README.md`: the reranker *"drifts ~5 % between runs even at
`temperature 0`"*, with a **±60 band on `total-readings`**. What the README does not say, and what
these two runs show, is that the drift also reaches `expected-hits` and `total-skeletons` — and that
`eval-parse-rate.sh` compares them against **pinned single values**, so drift in either direction is
reported as a REGRESSION with a confident diagnostic attached.

## The consequence, and the protocol that already handles it

A live run cannot attribute a change. The README already says so and names the instrument:

> `--replay <ranks.json>` re-runs recorded rankings with **no LLM at all** — deterministic. *"This is
> what lets a parser change be A/B'd against fixed rankings, isolating the code from the model."*

`--no-llm` (cap-only) is the arm for a **lexicon** change; `--replay` is the arm for a **code**
change. Running the live gate and reading a ±1 movement as a regression is a procedural error, and it
was made here before the README was consulted.

## What would fix the gate rather than work around it

The gate pins a single value for a quantity that is stochastic under its own default mode. Options,
none taken here:

1. **Gate only the deterministic arm.** Make `--replay`/`--no-llm` the gating run and demote the live
   number to a reported-not-gated progress metric.
2. **Pin a band, not a point**, for the drift-affected metrics — the README already quantifies one for
   `total-readings` (±60) but `baseline.json` stores a scalar.
3. **Label the drift-affected metrics in the diff output**, so `REGRESSION` on `expected-hits` prints
   with the caveat rather than the bare claim that a unit lost its reading.

(1) is the smallest change that removes the false positive class entirely. Filed here rather than as
an issue because it is a decision about the measurement protocol, not a defect with an obvious fix.
