# Build plan — kernel run records

*Branch: `kernel-run-records`. Covers eigenius#206, #148, #135, #147, #150; decides on #145 and
#149.*

*Governing document:
[Judgements, Warrants, and Logics](../design/judgements-and-warrants.tex) §"Taxonomy of Epistemic
Grounds". Follows `judgements-warrants-build-plan.md`, whose P0–P7 all landed.*

---

## 0. The framing this batch inherits, and why it changed

#206 was written `2026-08-22` to make two of *Derived's* three kernel routes attested, on this
ordering: *"this issue makes routes 1 and 3 mint → then `ProgramTrace` can be restricted → then
`Derived` is genuinely attested rather than assertable."*

**`Derived` no longer exists.** P4's three-grounds change removed it. Measured `2026-09-04`:

| | |
|---|---|
| `WitnessCategory` variants | `Declared`, `Observed`, `Verified` — `witness/mod.rs:51` |
| `trace_category(wk::PROGRAM_TRACE)` | `None`, deliberately — `layer/witness_index.rs` |
| `IsDerivedAs` | removed; *"It could only ever be consumed by `justification:Certificate.derived`, which is gone with the `DerivedEvidence` term constructor, so no lookup can ask for it — removing the constant is forced by the algebra"* (`well_known.rs:588`) |

So #206's destination is gone, and with it the #205 "enforcement half" it was gating: restricting
`ProgramTrace` to kernel-minted would distinguish two things that both ground nothing.

The paper supplies the replacement purpose in one sentence:

> A computed conclusion remains valid regardless of whether a run occurred. If the analysis plan
> denotes a function, the output is mathematically determined by the input. **The execution record
> constitutes provenance, not warrant.**

**This batch is provenance completeness.** A kernel-initiated run should leave a record that exists,
is complete, and can be read back. No item in it is evidential, and no item changes what any
proposition rests on.

## 1. What is true today

#206's factual claim is also stale in one half. Measured `2026-09-04`:

| route | mints a record? | where |
|---|---|---|
| `RunProgram` / `RunProgramByIri` | **yes** | `server/programs.rs` — *"Auto-commit program-run layer … ProgramTrace + all IO ComponentTraces. Per D41 §10"* |
| institution dispatch | yes | `reflection:InstitutionEmittedDerivation`, and since #160 a `prov:VerificationTrace` |
| `FIBER … INTO` | **no** | no reference to `PROGRAM_TRACE` anywhere under `kernel/src/query/` |
| resumed task | **no** | `server/lifecycle.rs:279` — `Ok(_) => { record.status = TaskStatus::Completed; }`, the result discarded |
| `RunRuntimeScript` | partial | `ProgramTrace` yes, `RuntimeInvocation` dropped (#145) |

And the records that *are* written are incomplete:

- **`prov:input` is never populated.** `domain: ProgramTrace`, recommended on the class. The only
  thing set is `reflection:input_hash` (`program/trace.rs:313`), a different property. A trace does
  not name what it ran on.
- **`prov:trace_tree` has no reader.** Written at `server/programs.rs:265`; no inverse of
  `trace_to_resource` exists, and nothing in the kernel, crates, CLI or orchestrator resolves a
  committed value.

## 2. The decision that shapes the work

The paper draws an asymmetry that this batch's single code path runs straight through:

> Conversely, for a stochastic process, the output is not strictly determined by the input. In this
> case, **the execution record is the evidence**, yielding an observation rather than an
> application. This structural asymmetry prevents the two from sharing a unified ground.

So a run record is provenance when the plan is `Declared(f : I -> O)`, and evidence — an `Observed`
leaf — when the process is stochastic. The minting code cannot tell which it is producing without
being told.

**Measured `2026-09-04`, and it settles the question: the plan declaration is not readable.**

`judgements-warrants-build-plan.md` §"Prerequisite found `2026-08-29`" recorded that 0 of 21
`stats:StatisticalAnalysisPlan` resources carried a `DeclarationTrace`, and set P4's shape
accordingly — *"author 21 `DeclarationTrace`s (with agents and rationales)"*. That did not land. On
the WRN chain today:

| | |
|---|---|
| `stats:StatisticalAnalysisPlan` resources | 21 |
| traced by `prov:ProgramTrace` | **21** |
| traced by `prov:DeclarationTrace` | **0** |

The chain carries 75 `DeclarationTrace` resources; none of them targets a plan. Since
`trace_category(wk::PROGRAM_TRACE)` is `None`, no plan emits any witness, so `Declared(plan)` — the
left half of `App(Declared(plan), Observed(inputs))` — cannot resolve for any of the 21.

**Nothing is broken by this yet**: no plan is cited as `DeclaredEvidence` anywhere on the chain
(`grep DeclaredEvidence …/chain/*.esl` matching `plan`: 0 hits), so no composite currently asks for
the missing half. The gap is latent.

**So this batch mints provenance only.** The stochastic case — where the paper says the execution
record *is* the evidence — stays out, stated as a limitation rather than left implicit, because the
minting site has nothing to read that would tell it which case it is in. Closing the plan-declaration
prerequisite is what unblocks it, and that is not this batch.

## 3. Items

### 3.1 `FIBER … INTO` commits a record — #206's surviving half

The write-back path produces chain resources and records nothing about what produced them. Record
the query and its inputs. `fiber.rs:406-424` is the site.

### 3.2 The resume path stops discarding — #148

`lifecycle.rs:279` sets `Completed` and commits nothing, while the non-resumed path through
`execute_program` commits the output resource and the trace. A task interrupted and resumed reports
success and leaves no output. This is the same hole as 3.1 on a different path; fixing `RunProgram`
without it leaves resumed runs unrecorded under a rule that says every run is recorded.

### 3.3 A failed run keeps its own record — #135

`programs.rs:128` constructs a fresh `TaskRecord::new_running` on the eval-error path, overwriting
what the record held. The completion path forty lines down does it correctly — re-reads with
`get_task` and mutates. #206 names this as a constraint on itself: *"the failure path must not
mint."* Both halves are in the same error arm.

### 3.4 The record carries its input, and can be read — #147

Populate `prov:input`. Then either give `prov:trace_tree` a reader or stop writing it — a
serialised tree with no inverse is not provenance anyone can traverse, and D6b §5 describes that
traversal as *the* provenance mechanism. **This is a decision, not only work**: the cheaper honest
outcome may be to drop `trace_tree` and keep the trace flat.

### 3.5 The `INTO` multi-binding rejection gets a test — #150

`fiber.rs:424` rejects a multi-row `FIBER` with `INTO`, since one IRI cannot name two resources.
No test drives it. 3.1 modifies this evaluator, so pin the path before changing it.

## 4. Decided out of the batch

- **#145 — `RunRuntimeScript` drops its `RuntimeInvocation`.** Same defect shape (record assembled,
  then discarded) and it is the blocker named in `docs/spec/w3c-prov-mapping.md` §5.2 for a
  chain-wide PROV export. **Out**, because it needs a `ComponentResponse` field — a protocol change
  with generated TS on two paths — and that sets a different exit gate for the whole batch. It is
  the natural follow-on.
- **#149 — `Exp::InstitutionInvoke.target_iri` is never populated.** Its doc comment names `INTO` as
  the surface that would set it; `INTO` goes through the fiber evaluator instead, which stamps its
  own IRI. **Out as work, in as a decision**: while 3.1 is open, decide whether to populate it or
  delete the field. Do not leave it undecided a third time.
- **#144, #146** — component-registry and memo-key defects. Adjacent files, unrelated failures.
- **The witness review** — `judgements-warrants-build-plan.md` §"Open after P7" asks whether the
  witness index is now a cache over relations rather than a soundness boundary. It says *"Do not act
  on this during P0–P7"*; P7 is done, so it is live. It is a question to answer once, not a build,
  and it does not belong on this branch.

## 5. Sequencing

1. **§2's decision first.** It determines whether one record type or two come out of the minting
   sites, and every other item writes against that answer.
2. **3.5, then 3.1.** Pin the `INTO` error path before editing the evaluator.
3. **3.3 with 3.1**, since #206 constrains the failure path and both are the same error arm.
4. **3.2 after 3.1** — the resume path should mint what `RunProgram` mints, so it copies a shape
   rather than inventing one.
5. **3.4 last.** It changes the record's fields, so it lands once the set of minting sites is fixed.

**No ontology edit is expected.** `prov:input` and `prov:trace_tree` are already declared. If 3.4's
decision is to drop `trace_tree`, that *is* a bootstrap edit and joins #235's batched reseed rather
than paying for its own.

## 6. Verification

Per `judgements-warrants-build-plan.md` §"Verification, every phase":

- `cargo test --workspace`, `cargo fmt --all -- --check`, `RUSTFLAGS="-D warnings" cargo clippy
  --workspace --all-targets`.
- **Written as failing tests first**: a resumed task commits an output resource and a trace (3.2); a
  failed `RunProgram` leaves its original record intact and mints nothing (3.3); a multi-row `FIBER
  … INTO` is rejected (3.5).
- **The count that matters**: after 3.1 and 3.2, every kernel-initiated run on a test chain has
  exactly one record. Measure it, do not assert it.
- If 3.4 drops `trace_tree`: reseed, and check the resource count against the plan's §0 method.

## 7. What this batch does not claim

It does not make anything more attested than it was, and it does not close the plan-declaration
prerequisite in §2. `ProgramTrace` grounds nothing before and
after — a computed claim rests on `App(Declared(plan), Observed(inputs))`, and the run record is not
a third ground. What changes is that the record of a kernel-initiated run exists, names its input,
and can be read back.

#206 should be retitled and rewritten against §0 and §1 before the first commit, so the issue does
not keep asserting a premise the tree removed.
