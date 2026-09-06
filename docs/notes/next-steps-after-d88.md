# Next steps after D88

*Written `2026-09-05` at the tip of `numeric-core-and-verification-judgement`; revised the same day
as B1 and B2 landed.*

---

## A. Close out the current batch

| | | state |
|---|---|---|
| **A1** | Close eigenius#235 — fully discharged by this branch | done |
| **A2** | Open the PR | done |
| **A3** | The bootstrap manifest has moved **three** times on this branch: `#235`'s description strings, B2's merge, B1's implicit binders. Every persisted store is unresumable until **B4** runs | pending B4 |

`eval-parse-rate.sh` already refuses to score a run with no summary line, so a `ManifestDrift` SKIP
in the interim cannot be misread as a pass.

A3 first said "do not reseed yet, fold this into B2/B3's rather than paying for two" and priced a
reseed at ~3h. **Both halves were wrong.** The reseed is ~30 minutes, so the batching argument never
rested on its cost; what makes batching worth it is the alignment snapshot and the two parse
measurements that follow it. And B1 moved the manifest too, which the original ordering assumed it
would not.

## B. Work D88 decides

**Order, as executed:** B2, then B1, then B3, then one reseed (B4). The note first said B2 and B3
before B1 on the belief that B1 needed no reseed; it did. B5 arrived from C2's examination and rides
B4 if its design question resolves cheaply — see below.

### B1 — declare the implicit binders (D88 §4) — **DONE `2026-09-05`** (`4fc8986`)

Built as declared `core:implicit_args` (binder names) with an `implicit(A, B)` ESL clause, not as
the `eigentt:Implicit(Prop)` marker type the original plan recommended. **The marker was the wrong
shape.** It annotates the binder's TYPE, so every reader of `core:ctor_type` that does not know
about it sees a type that does not exist — the mirror generators, `esl::print`, the Lean translation
each would have to remember to strip it. A separate property is read only by the code that cares;
everything else sees the true telescope. It also needs no universe-polymorphic marker constant whose
own type has to be right for a thing that is stripped before checking.

Two kernel changes the plan did not have: a `MetaCtx` spanning the whole constructor check (Phase F,
as predicted), and componentwise comparison of two anonymous arrows (not predicted — `app`'s binders
sit inside `Certificate(A -> B)`, and every `Val::Pi` fell through to readback equality). Anonymous
is the whole soundness argument: nothing is bound, so no variable is introduced and nothing can be
captured. `solve_meta`'s scope check was strengthened alongside it — metas record their creation
level and refuse a solution proposed from inside a binder they do not scope over. See D88 §4.

Content: 309 `app` calls lost two arguments each across 14 ESL files, plus 2 `sum_l`, 1 `sum_r` and
the notebook's 4. `spec_poly` stayed fully explicit, `T` included.

### B2 — collapse `justification:Term` into `justification:Certificate` (D88 §2) — **DONE `2026-09-05`** (`ba1abb6`)

663 argument deletions across 24 files plus 176 dead alias bindings — ~60× the estimate, which
counted the notebook and missed the WRN publication chain. See D88 §2. The 32 residual
`x = Declared(IRI)` aliases it left were removed by hand in `ce670ba`, along with the comments that
still described `justification:Term` as current.

### B3 — declare the leaf IRI-valued (D88 §3) — **NEXT**

**The sub-choice is resolved, and B2 moved the ground under it.** The note offered a `core:iri`
DataType *or* a format slot on `InductiveArgType`. The second reaches one of the seven slots. It was
written when the leaves lived on `justification:Term`'s **positional** constructors; B2 deleted
those, and the survivors are spread across three declaration forms:

| slot | declared as | carrier |
|---|---|---|
| `Certificate.declared` / `.observed` / `.verified` — `iri` | typed telescope | `core:ctor_type` (a D47 `Exp::Pi` binder) |
| `witness:IsDeclaredAs` / `IsObservedAs` / `IsVerifiedAs` — index #0 | index telescope | `core:indices` → `InductiveParam.param_kind` |
| `eigentt:Term.Checked` — `payload_iri` | positional | `core:arg_types` → `InductiveArgType.type_name` |

**So: `core:iri`, a sixth `PrimitiveType`.** All three forms name a *type*; change the type named and
every form follows. Annotating each form separately is one declaration written three ways.

Two facts worth having before starting:

- `core:formats:iri` and a working shape predicate **already exist** (`wk::FMT_IRI`, Rule 4 in
  `validation/rules/format.rs`). They are property-scoped — `check_format` reads the slot off a
  `prop_def` — so they do not apply to a constructor binder, but the predicate is reusable rather
  than something to write.
- `PrimitiveType` has five variants today and 67 `PrimitiveType::` match sites across the workspace.

Bootstrap edit — rides B4.

**The exact `core:mentions` rule is NOT part of this; it is B6.** D88 §3 wrote the declaration and
the consumer as one step and they are not: `json_mentions_of_value` walks an ENCODED term, where a
string's role is invisible without the constructor schema, so declaring the leaf does not by itself
retire `s.starts_with("urn:")`.

### B6 — make `core:mentions` read the declaration (D88 §3, second half)

The mechanism, found while doing B3 so it is not rediscovered: each argument of an encoded value is
carried under a property named `<ctor-class>-<arg-name>`, and **those are real chain resources
carrying `core:data_type`** — which is how B3's retyping first surfaced, as an `allows_only`
violation on `eigentt:Term-Checked-payload_iri`. So the walk can resolve each property definition
and treat `core:iri` as a reference, reusing the `match data_type` dispatch `layer/index.rs:324`
already performs at the top level.

Two reasons it is its own change rather than a rider on B3:

- `json_mentions_of_value` needs a `&Layer` threaded in. Both callers have one
  (`validation/mod.rs:1381`, `layer/index.rs:334`), as does `storage/memory.rs:242` for the
  sibling `is_witness_candidate`.
- It **narrows** the index's mention set, which the current walker over-approximates on purpose —
  its own doc says *"counting it is the correct answer, not an over-approximation to apologise
  for."* Changing what the index contains needs its own tests, not a rider's.

The same heuristic also sits at `program/expr.rs:903` and `nbe/eval/marshal.rs:35`; whether those
are the same question is B6's to answer.

### B4 — one reseed, then both baselines

B3 is bootstrap, and B5 may be; A3's three accumulated deltas ride along. **The reseed is ~30 minutes**; the
alignment snapshot and the two parse measurements are the rest of the wall clock.

**Use the scripted protocols; do not drive the harness by hand.** `measure-parse-rate.sh` is the
MEASUREMENT half and `eval-parse-rate.sh` is the SCORING half — the second exists because reading
these numbers by eye is how they get read wrong, and its header names three traps that have each
produced a false result (`grammar-gap` counted from the per-unit listing, which omits gaps; a run
with no summary line scored as if it completed; a cap-only run compared against a reranked one).
`measure-parse-rate.sh` chains into it automatically.

```sh
CARGO_FEATURES=use-llm scripts/reseed-lexicon-db.sh --umls-all
scripts/build-alignment-snapshot.sh --base <base> --out <aligned>
scripts/measure-parse-rate.sh --snapshot <aligned> \
  --replay experiments/parsing/ranks/2026-08-22-productiontrace.json \
  --selections experiments/parsing/selections/2026-08-22-productiontrace-live.json
scripts/eval-parse-rate.sh --baseline <run.log>   # if scoring a run separately
```

`measure-parse-rate.sh` builds **release** and that is load-bearing, not a speed choice: a debug
build overflows the stack in NbE readback, the parse dies, and the harness reports it as a
GRAMMAR-GAP indistinguishable from a real one. It also autodetects the newest snapshot, so
`--snapshot` is only needed to override that.

Gates: `grammar_gap == 0`, `missing_lexeme == 0`, `expected-hits 62/62` with the miss-set unchanged,
`reading_correct >= 30`, `reading_unadjudicated == 0`, `invalid_selected == 0`. `eval-parse-rate.sh`
enforces these against the two committed baselines — `baseline.json` gates the grammar and lexicon,
`selection-baseline.json` gates the ranker; they re-baseline on different triggers. A single **live**
draw is a draw, not a measurement — replay is the comparison.

## C. Decisions

### C1 — widen the unification fragment past first-order patterns · **not needed**

**Nothing is broken without it.** `spec_poly` works today and every certificate using it
type-checks, because the author writes `T`, `P` and `x` out. No test is skipped and nothing is
unsound. Earlier phrasing here — *"what `spec_poly`'s `P` needs"* — read as though `P` were
defective; what needs C1 is making `P` **implicit**, which is ergonomics.

What it would buy, measured `2026-09-05` across every authored certificate in the tree:

| | |
|---|---|
| `spec_poly` call sites | 7 |
| characters written for `T` | 321 |
| characters written for `P` | 1,622 |
| total an author could stop writing | **1,943** |

`x` stays explicit either way: it is the instance the author chose, the same authorial-content
argument that keeps the grounding constructors' `iri` written.

`T` is gated on C1 as well, which is not obvious — it looks first-order. Solving it from the premise
requires the whole of `Certificate(forall (y : T) => P(y))` to unify, and that type's codomain is
`P(y)`, a meta applied to a bound variable. Unification is all-or-nothing per argument, so `T`
cannot be recovered while `P` cannot.

So the trade is a change to EigenTT's unification fragment (D48 §3.1) against 1,943 characters at 7
sites. Poor ratio today. It changes if universal rules get used more heavily than 7 sites suggest.

### C2 — examined `2026-09-05`, and it is two questions, not one

**C2a — replace the three `witness:Is*As` families with one `ChainWitness(category, iri, P)`:
declined.**

The three constants do not disappear, they move: from three decl IRIs
(`wk::chain_witness_category_for_iri`, read at `program/check_hooks.rs:48`) to three constructors of
a new `witness:Category` inductive plus a value→enum readback. One new inductive, one more index to
validate, the same enumeration.

It also costs what D88 §1 found the witness types are *for* — the type declares the trigger and
carries the lookup's parameters as its indices. Under C2a the category stops being declared by the
type and becomes an argument. And the independence of the families, which
`witness_admission.rs:1496` tests and which a since-deleted `IsVerifiedAs → IsDerivedAs` match arm once
violated, degrades from a type distinction to a value comparison.

**C2b — the trace-kind → grade mapping lives in Rust, not the ontology: stands, and C2a does not
address it.**

This is what C2 was actually about, and merging the families is not a route to it. `trace_category`
(`layer/witness_admission.rs:224`) maps five trace classes onto three categories and a deliberate
`None`:

```
DeclarationTrace       → Declared
ObservationTrace       → Observed
ProgramTrace           → None        (deliberately)
VerificationTrace      → Verified
ExternalExecutionTrace → Declared    (eigenius#205)
```

D81 §1.1 and §1.3 already state the defect, and more sharply than "three constants": the ontology
carries no class, property or relation expressing *"this trace kind grounds that grade"*. The set
that matters to the witness machinery — trace kinds that ground a witness — has no class, no
`subclass_of` edge joining its members, and no property marking membership; it exists only as
`is_witness_candidate` (`:197`) and `trace_category` (`:224`), both Rust. Its members span two unrelated families plus
a parentless group: `DeclarationTrace`, `ObservationTrace` and `VerificationTrace` are siblings of
nothing, while `ExternalExecutionTrace` and `ProgramTrace` sit under `ProductionTrace`.

Even with a single `ChainWitness(category, …)`, something still has to decide that a trace of a
given kind yields `Declared`.

**But that decision belongs in the kernel, and declaring it on the trace classes is a deprecated
pattern.** See B5 above, refuted `2026-09-06` against the paper. C2 is closed in both halves: the
merge is declined, and the hardcoded mapping is the specified design rather than a defect.

### B5 — declare the trace-kind → grade mapping · **REFUTED `2026-09-06`, not built**

The paper rules against it directly. `judgements-and-warrants.tex` §"Deprecated Architectural
Patterns" lists, as a pattern to be replaced:

> **Grades assigned by class membership, by a trace declaring its own grade, or by the importer
> that wrote the resource.** Replaced by computation from stored evidence. No path exists by which
> asserting a class confers evidential standing.

Both shapes I built or proposed are named there. A property on the trace class is "a trace declaring
its own grade". Deciding the grade by `subclass_of` is "grades assigned by class membership".

The extensibility premise is refuted in the same list:

> **A protocol for institutions to supply their own witness kinds.** Unnecessary: an institution
> supplies a judgement in a logic the system checks, or its output is *Computed*.

D81 §3.2 reports that adding a trace kind takes a kernel edit and calls it a limitation — *"the
extensibility the institution mechanism provides stops at this boundary."* The paper says that
boundary is intended. An institution with something to contribute either supplies a checkable
judgement, reaching *Verified*, or its output is `App(Declared(plan), Observed(input))`.

**And `trace_category` is a TCB element, which I had recorded as the opposite.** The paper: *"the
`Verified` state is provable, whereas `Declared` and `Observed` states are postulated ... the kernel
asserts the remaining two as proof constants under a defined constant specification"*, and the TCB
*"consists of the kernel's native type checker, each hosted external proof checker, each formal
comorphism, and the constant specification governing attributions."* The rule *"a `DeclarationTrace`
targeting R postulates `IsDeclaredAs(R, P)`"* is that constant specification. Moving it into chain
data would move a TCB element somewhere a layer can extend. The earlier note here said the opposite
— that a class grounds nothing and `Verified` is separately gated — which is true of `Verified` and
irrelevant to the two grades that are postulated, since having no gate is what postulation means.

D81 §1.3 observes the concept lives in two Rust functions. That observation is correct and the
condition is intended.

**Kept from the attempt:** `crates/eigenius-statistics/tests/d39_composition.rs` builds a chain that
commits `prov:DeclarationTrace` resources without loading `prov`; the layer is now loaded. And the
ESL grammar accepts a property on a class, which was a real surface gap — the grammar admitted only
`description`, `requires` and `recommends`, so a class-level annotation had to be authored in JSON.
Nothing in the tree uses it; two tests pin it.

**Method note.** I built this from `next-steps-after-d88.md`, then read D81, then read the paper,
in that order. The paper is the governing document and rules against the design in one sentence.
Read it first.

## D. Deferred, reasons already recorded

| | |
|---|---|
| **D1** | eigenius#236 — D30 emitting chain definitions as Lean `def`s. Drift produces an unmapped constant and `unknown_pp_declar_hard_error` makes nanoda refuse, so drift is refused rather than silent |
| **D2** | A PROV exporter — needs the in-process Activity gap closed first (`w3c-prov-mapping.md` §5.2) |

## E. Removed

**The ScienceAgentBench tracer tasks, `2026-09-05`** (`6a105cc`). Both task chains, `mol.esl` and
their two tests carried no unique regression coverage: with B1's first, over-restrictive unification
rule, twelve tests reject it — `wrn_phase3` (8), `wrn_phase2`, `wrn_phase5`, `d39_composition` and
the two tracers (1 each). `bench-core.esl` and `harness-ontology.esl` stay; the WRN chain loads both.
D50 and D51 are marked dormant. One shape is no longer exercised anywhere: `spec_poly` over a
five-premise rule discharged one premise at a time.
