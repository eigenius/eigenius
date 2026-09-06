# Next steps after D88

*Written `2026-09-05`, at the tip of `numeric-core-and-verification-judgement`.*

---

## A. Close out the current batch

| | | cost |
|---|---|---|
| **A1** | Close eigenius#235 — fully discharged by this branch | — |
| **A2** | Open the PR | — |
| **A3** | **Do not reseed yet.** The snapshot is stale (`core-ontology.json` moved in `520137c`, after the image the snapshot was built from), so no snapshot opens at HEAD. The delta is three description strings and cannot touch the parser. Fold this reseed into **B2/B3**'s rather than paying for two | — |

`eval-parse-rate.sh` already refuses to score a run with no summary line, so a `ManifestDrift` SKIP
in the interim cannot be misread as a pass.

## B. Work D88 decides

**Order:** B2 and B3 first, then B1, then one reseed (B4). B1 was scheduled first on the belief that
it needed no reseed; it does — see below. Doing it after B2 also shrinks it, since the merge removes
`j1` and `j2` from `app` outright.

### B1 — declare the implicit binders (D88 §4) — **DONE `2026-09-05`**

Built as declared `core:implicit_args` (binder names) with an `implicit(A, B)` ESL clause, not as
the `eigentt:Implicit(Prop)` marker type recommended below. **The marker was the wrong shape.** It
annotates the binder's TYPE, so every reader of `core:ctor_type` that does not know about it sees a
type that does not exist — the mirror generators, `esl::print`, the Lean translation each would
have to remember to strip it. A separate property is read only by the code that cares; everything
else sees the true telescope. It also needs no universe-polymorphic marker constant whose own type
must be right for a thing that is stripped before checking.

Two kernel changes the plan did not have: a `MetaCtx` spanning the whole constructor check (Phase
F, as predicted), and componentwise comparison of two anonymous arrows (not predicted — `app`'s
binders sit inside `Certificate(A -> B)`, and every `Val::Pi` fell through to readback equality).
Anonymous is the whole soundness argument: nothing is bound, so no variable is introduced and
nothing can be captured. `solve_meta`'s scope check was strengthened alongside it — metas record
their creation level and refuse a solution proposed from inside a binder they do not scope over.
See D88 §4.

Content: 309 `app` calls lost two arguments each across 14 ESL files, plus 2 `sum_l`, 1 `sum_r`,
and the notebook's 4. `spec_poly` stayed fully explicit, `T` included.

The original plan, kept for the record:

### B1 — declare the implicit binders (D88 §4)

**Which binders.** Not "the solvable ones" — the ones that carry no authorial content. The grounding
constructors' `iri` is solvable and must stay written: it *is* the author's citation, and eliding it
would mean never naming what a claim rests on.

| ctor | binders | implicit |
|---|---|---|
| `declared` / `observed` / `verified` | `iri`, `P` | **none.** `iri` is the citation; `P` at the citation site is a check, not noise |
| `app` | `A`, `B`, `j1`, `j2` | **all four.** `j1`/`j2` restate the sub-certificates' terms, `B` restates the expected proposition, `A` is the intermediate |
| `sum_l` / `sum_r` | `P`, `j1`, `j2` | **all three** |
| `spec_poly` | `T`, `P`, `j`, `x` | `T`, `j`. `x` is the instance the author chose; `P` is higher-order and unsolvable anyway |

**The foundation already exists, and it is the type-keyed elision rule.** A `ChainWitness`-typed slot
is filled by the kernel and never written by the author, and *alignment is decided by the declared
type before any solving happens* — which is exactly what the reverted attempt lacked. Generalise
that rule rather than adding binder styles to `Exp::Pi`:

- Declare a marker `eigentt:Implicit`, and write the binder as `forall (A : eigentt:Implicit(Prop), …)`.
- `peel_ctor_telescope` **unwraps** it: the `CtorArg` records `implicit: true` and the binder's type
  as the bare `T`. The marker never reaches the type checker, so `Certificate(j1, A -> B)` still sees
  `A : Prop`. It is a declaration-site annotation that happens to be encoded as a type application.
- `check_inductive_ctor_args` skips implicit slots when consuming user arguments and solves them by
  unifying the result type against the expected indices — the same unification D48 Phase D already
  runs at the end of that function.
- A binder still unsolved after that (`app`'s `A`, which appears in no result index) is solved by
  elaborating one explicit argument in inference mode. Unsolved after *that* is an error naming the
  binder.

**Why not a binder style on `Exp::Pi`:** it would need a new field on `Exp`, a codec change and ESL
grammar, and would duplicate a mechanism the kernel already has. The marker-type route touches
`CtorArg`, the peeler and the arg loop, and reuses the elision path that is already sound.

Still a bootstrap edit — the constructors' encoded types change — so it rides B4's reseed.

### B2 — collapse `justification:Term` into `justification:Certificate` (D88 §2) — **DONE `2026-09-05`**

Landed as `ba1abb6`. Cost 663 argument deletions across 24 files plus 176 dead alias bindings —
~60× the estimate, which counted the notebook and missed the WRN publication chain. See D88 §2.
Residue: 32 `x = Declared(IRI)` aliases in 7 files, to be removed by hand.


`Justification : Prop -> Type 2`, seven constructors, each losing its term arguments.

- `certificate_indices` → one index. Five call sites; three only test `.is_some()`.
- `support`, `is_fully_verified`, `wellfounded` walk the certificate value instead of the term index.
- Remove `justification:Term` from the codec and `well_known`.
- Rewrite the demo notebook's certificate — it gets **smaller**.
- Versioned ADT change.

### B3 — declare the leaf IRI-valued (D88 §3)

On `Declared`, `Observed`, `Verified` and `Checked`.

- Choose: a `core:iri` DataType, or a format slot on `InductiveArgType`. **Open sub-choice.**
- Then `core:mentions` gets an exact rule instead of `s.starts_with("urn:")`, and the validator
  rejects a malformed leaf at commit instead of leaving it to whichever consumer parses first.

### B4 — one reseed, then both baselines

B2 and B3 are bootstrap; A3's stale delta rides along. **The reseed is ~30 minutes**, not the
several hours this note first assumed — the alignment snapshot and the two parse measurements are
what the rest of the wall clock goes on.

```sh
CARGO_FEATURES=use-llm scripts/reseed-lexicon-db.sh --umls-all
scripts/build-alignment-snapshot.sh --base <base> --out <aligned>
scripts/measure-parse-rate.sh --snapshot <aligned> \
  --replay experiments/parsing/ranks/2026-08-22-productiontrace.json \
  --selections experiments/parsing/selections/2026-08-22-productiontrace-live.json
```

Gates: `grammar_gap == 0`, `missing_lexeme == 0`, `expected-hits 62/62` with the miss-set unchanged,
`reading_correct >= 30`, `reading_unadjudicated == 0`, `invalid_selected == 0`. A single **live**
draw is a draw, not a measurement — replay is the comparison.

## C. Decisions, not work

Neither is answered by D88, and neither blocks B.

| | |
|---|---|
| **C1** | Widen the unification fragment past first-order patterns, which is what `spec_poly`'s `P` needs. A decision about EigenTT (D48 §3.1), not about the justification layer |
| **C2** | Replace the three `witness:Is*As` families with one `ChainWitness(category, iri, P)`, making `trace_category`'s mapping a value rather than three constants. Not examined |

## D. Deferred, reasons already recorded

| | |
|---|---|
| **D1** | eigenius#236 — D30 emitting chain definitions as Lean `def`s. Drift produces an unmapped constant and `unknown_pp_declar_hard_error` makes nanoda refuse, so drift is refused rather than silent |
| **D2** | A PROV exporter — needs the in-process Activity gap closed first (`w3c-prov-mapping.md` §5.2) |
