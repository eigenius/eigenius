# Reseed — `2026-08-25`, the D76 Phase B parity gate

Run to validate **Phase B** — the whole de-inlining of `Exp` — against the real chain. Four kernel
commits had landed with no reseed: `Exp::Inductive` / `Exp::InductiveType` deleted, `InductiveCtor` /
`InductiveRec` and `Val::InductiveVal` switched to the IRI, the self-reference stub removed, and the
typing environment threaded through eval, closure application, and 18 previously env-less call sites.

Command: `scripts/reseed-lexicon-db.sh --umls-all`, from `406f0b2`.
Snapshot `../db-snapshot/wordnet-umls-2026-08-24` (see §4 — the path is a day behind).

## 1. Result: exact parity

| | 2026-08-24 (`4ba900a`) | 2026-08-25 (`406f0b2`) | delta |
|---|---|---|---|
| resources | 9,439,633 | **9,439,633** | **0** |
| loads | 35 (4 WN + 31 UMLS) | **35** | 0 |
| **errors** | 0 | **0** | **0** |
| store | 2,912,381,027 B | 2,884,062,153 B | −28,318,874 (−1.0 %) |
| density | 308.5 B/resource | 305.5 B/resource | −3.0 |

**Exhaustive, not a sample.** The baseline was zero errors over this exact input, so zero on the new
code means no verdict moved pass → fail, and none could move fail → pass because there were none to
move. Phase B's gate is met over 9.4M resources.

**The identical resource count is the evidence for "de-inlining moves no bytes"** (D76 §8 Phase E2's
correction). Every inductive reference in this chain now decodes to a `Const` / `InductiveCtor`
carrying an IRI instead of a declaration, and the chain came out the same size in resources. Store
size moved −1.0 %, inside the unexplained ±few-percent band the `2026-08-23` and `2026-08-24` notes
both recorded in opposite directions; nothing is claimed from it.

## 2. Wall clock: 60 m 35 s, and **not** attributable to Phase B

Total was 3,635 s against the baseline's 2,173 s — +67 %, far outside the 2,080–2,189 s spread that
note recorded for the *same code run twice*. That is large enough that it had to be chased rather than
waved at noise.

**It is not the kernel.** Measured directly, same machine, back to back, on the same two converted
`.esl` files:

| | `wordnet-000-base` | `wordnet-001` | total |
|---|---|---|---|
| pre-Phase-B (`a9ae729`) | 19 s | 53 s | **72 s** |
| post-Phase-B (`406f0b2`) | 21 s | 52 s | **73 s** |

1.4 % apart, in both directions per layer. **Per-layer load time is unchanged**, so the extra ~24
minutes is in the host-side conversion steps or in machine conditions, not in commit-time validation.

This also refutes the hypothesis worth writing down because it was wrong: that de-inlining moved
declaration decoding from decode-time to lookup-time and the `Global` memo was not installed on the
commit path. The memo-scope gap is **real** — `validation::retroactive::revalidate_pending` calls
`validate_resource` in a loop with no scope installed, unlike `Validator::validate` — but it is not
what this timing shows, and the retroactive pass is scoped to redefinitions. Left as an observation,
not a fix chased on bad evidence.

## 3. What this run did *not* measure

**§4.2's `Global`-memo boundedness, despite `entry_count()` having been built for it.** The memo is a
thread-local inside the kernel process; the reseed runs the kernel in a container and reads nothing
back but load counts. There is no reporting path, so the count was never observed. The claim that the
key set is bounded by IRIs *appearing in terms* rather than by chain size therefore remains an
argument supported by a unit test, exactly as it was before this run.

Measuring it needs either an in-process harness over a large chain or a kernel-side log line at pass
end. Neither exists; whichever is cheaper belongs with Phase D, which is where §4.3 decides whether
the memo earns its place at all.

## 4. The snapshot path is a day behind

The script dates the snapshot by local day and this run crossed 00:00 UTC while the local clock still
read the 24th, so it wrote to `wordnet-umls-2026-08-24` — **overwriting the baseline store**. No loss
for this comparison: the baseline's numbers are recorded in its own note and this run reproduces them
exactly. But there is no longer a pre-Phase-B store on disk to diff against, and a future run wanting
one must snapshot before reseeding.
