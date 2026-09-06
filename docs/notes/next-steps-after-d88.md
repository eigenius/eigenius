# Next steps after D88

*Written `2026-09-05`, at the tip of `numeric-core-and-verification-judgement`.*

---

## A. Close out the current batch

| | | cost |
|---|---|---|
| **A1** | Close eigenius#235 — fully discharged by this branch | — |
| **A2** | Open the PR | — |
| **A3** | **Do not reseed yet.** The snapshot is stale (`core-ontology.json` moved in `520137c`, after the image the snapshot was built from), so no snapshot opens at HEAD. The delta is three description strings and cannot touch the parser. Fold this reseed into **B2/B3**'s rather than paying ~3h twice | — |

`eval-parse-rate.sh` already refuses to score a run with no summary line, so a `ManifestDrift` SKIP
in the interim cannot be misread as a pass.

## B. Work D88 decides

**Order:** B2 and B3 first, then B1, then one reseed (B4). B1 was scheduled first on the belief that
it needed no reseed; it does — see below. Doing it after B2 also shrinks it, since the merge removes
`j1` and `j2` from `app` outright.

### B1 — infer `app`'s `forall`-bound arguments (D88 §4)

**Attempted `2026-09-05` and reverted. It is a bootstrap change, not a kernel-only one, so it rides
B4's reseed with B2 and B3.**

The attempt derived implicitness rather than declaring it: solve every binder by unifying the ctor's
result type against the expected type, then let the author omit the solved ones, with an
argument budget deciding how many. That needs no ESL syntax, no `Exp` change and no reseed — and it
is **unsound at the alignment step**, because "omitted a solvable binder" is indistinguishable from
"supplied a value for one".

Measured: `verified(CLAIM, P)` has two arguments against three specs, and both `iri` and `P` are
solvable from the expected type. Auto-filling `iri` shifts `CLAIM` onto the `P : Prop` slot —
`type mismatch: EigonPrimitive(String) ≠ Sort(Zero)`. Caught by
`a_certificate_citing_the_verified_claim_type_checks`, which is the test written for D88 §1's bridge
and had no other consumer.

So implicitness has to be **declared**, which means:

- a binder style on `Exp::Pi` (it has none — `Pi(Patt, Box<Exp>, Box<Exp>)`), through the D47 codec;
- ESL syntax for an implicit binder;
- marking `justification:Certificate`'s binders, which changes the constructors' encoded types —
  **a bootstrap edit**.

Then the mechanism is the easy part, and most of it exists: `nbe/unify.rs` (D48 Phase C), index
unification already running on every ctor check (Phase D), and a `MetaCtx` that needs to outlive one
check (the code names this Phase F). `j1`, `j2` and `B` unify against the expected type; **`A` needs
one argument elaborated in inference mode**, since it appears in no result index. `spec_poly` stays
explicit — `P` is higher-order, outside D48 §3.1's fragment.

### B2 — collapse `justification:Term` into `justification:Certificate` (D88 §2)

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

B2 and B3 are bootstrap; A3's stale delta rides along.

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
