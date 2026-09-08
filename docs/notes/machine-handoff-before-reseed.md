# Machine handoff — the state just before B4's reseed

`2026-09-07`, branch `numeric-core-and-verification-judgement`.

Everything through B6, the Lean version gate and the `justification.esl` rewrite is committed and
pushed. **B4's reseed has not run.** It is the next step, and it runs on the machine that has the
source corpora.

## What does not travel with the branch

`/references` is gitignored (`.gitignore:18`). Three inputs live only on the machine that
downloaded them:

| path | size | how to get it again |
|---|---|---|
| `references/WordNet-3.0/dict` | 38 MB | `scripts/provision-wordnet.sh` |
| `references/umls/2026AA/META` | 1.2 GB | `scripts/provision-umls.sh`, own UMLS licence |
| `references/umls-2026AA-metathesaurus-level0.zip` | 1.9 GB | the same download; the META above is what was extracted from it |

Also machine-local: the docker volume `eigenius_eigenius_db` and every snapshot under
`$SNAPSHOT_ROOT` (default `../db-snapshot`, i.e. **outside the repo**). A snapshot is rooted at the
bootstrap it was seeded with, and this branch has moved the manifest five times, so no existing
snapshot is resumable anyway — copying one across would not save the reseed.

The Lean toolchain is pinned in-tree (`lean/*/lean-toolchain` → `leanprover/lean4:v4.29.1`) and
`elan` fetches it, so that needs no transfer. `crates/eigenius-lean/src/checker.rs` refuses an
export from any other minor series, so a machine with a different default toolchain fails loudly
rather than producing a confusing defeq failure.

## What the receiving machine needs

1. `elan`, `cargo`, `docker` (the reseed brings up a kernel container).
2. The two corpora above, provisioned before starting.
3. Nothing else — the fixtures, ontologies and pinned manifest are all in git.

## Then run B4

The protocol is in `docs/notes/next-steps-after-d88.md` §B4 and is not repeated here. Two things
from it that are easy to get wrong:

- `--umls-all` is not the default. A bare invocation builds an 8-TUI store that looks fine and is
  not comparable to any tracked baseline; that cost one full reseed on `2026-08-14`.
- `measure-parse-rate.sh` builds **release**, and that is load-bearing rather than a speed choice:
  a debug build overflows the stack in NbE readback and the harness reports the dead parse as a
  grammar gap indistinguishable from a real one.

Gates: `grammar_gap == 0`, `missing_lexeme == 0`, `expected-hits 62/62` with the miss-set
unchanged, `reading_correct >= 30`, `reading_unadjudicated == 0`, `invalid_selected == 0`.
`eval-parse-rate.sh` enforces them; do not read the numbers by eye.

## Known failure, pre-existing

`eigon_r_marshalling_round_trip` (`crates/eigenius-r/tests/marshalling_round_trip.rs:156`) expects
`Value::Json` for `reflection:canonical_proposition` and gets `Value::Embedded`. D85 §6.1 made an
inductive value an embedded resource; this test was not updated. It fails on a clean checkout of
`main` too — proved by stashing — and is unrelated to this branch's work.

## Where the work stands

Built and passing: Scenario A (three grounds compose in one certificate), Scenario B (proving one
step removes one declaration from what a conclusion rests on), G5 (in-process `prov:Activity`), B6
(`core:mentions` reads the declaration), D86's float-literal normalisation, the Lean version gate.

Not built, and blocked on decisions rather than effort: the full Scenario B, where `stats:lt`
becomes the Verified step. It needs G1 (the proposition shape) and D86 §4's `Rat` pivot. The first
procedural proof to target after the pivot is threshold weakening,
`lt(x,T) -> T <= T' -> lt(x,T')` — see `docs/notes/end-to-end-scenarios-and-integration-gaps.md`.
