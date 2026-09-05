# Build plan — kernel run records

*Branch: `kernel-run-records`. Covers eigenius#206, #148, #135, #147, #150, #144 (in full — §3.5),
#149 (§4.4); decides on #145 and #146 (§4).*

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

**The declaration exists, is populated, and nothing reads it.**

`program:component:deterministic` — *"Whether the component produces the same output for the same
input"* — is a `core:boolean` with `domain: program:Component`, `recommends` on the class. That is
the paper's `f : I -> O` assertion, sited on the component rather than on the analysis plan. Measured
`2026-09-04`:

| | |
|---|---|
| `program:Component` resources | 12 |
| declaring `deterministic` | 9 — **4 true, 5 false** |
| not declaring it | 3 |
| Rust code reading the property | **0** |

The split is meaningful, not incidental:

| | components |
|---|---|
| `true` | `Combine`, `Extract`, `Identity`, `Transform` — pure data transforms |
| `false` | `CompleteJson`, `CompleteText`, `HttpRequest`, `RunRuntimeScript`, `Checkpoint` |

The `false` set is exactly the paper's stochastic case. An LLM completion's output is not determined
by its input, so the execution record *is* the evidence — there is nothing to re-derive it from.

**So the minting site reads `deterministic` and the asymmetry is mechanical.** No new authoring
burden and no new vocabulary: a `true` component's run yields provenance, a `false` component's run
yields an `Observed` leaf.

**One decision this forces.** The property is `recommends`, so 3 of 12 components do not carry it.
Absent must default to **non-deterministic** — the record is evidence. Defaulting the other way
asserts a determinism nobody declared, which is the direction that silently manufactures a
`Declared(f : I -> O)` premise. Whether to promote the property to `requires` is a separate
question; it is a bootstrap edit and belongs with #235's batched reseed, not here.

*Related but not blocking:* #43 asks whether a component **declared** deterministic actually is, for
Julia runs (BLAS pinning, FMA discipline). This batch takes the declaration at its word, which is
what the paper's `Declared(f : I -> O)` means — an assertion by an accountable agent, not a
measurement.

### 2.1 A separate gap found while measuring this

The analysis plans are traced, but not declared. On the WRN chain:

| | |
|---|---|
| `stats:StatisticalAnalysisPlan` resources | 21 |
| traced by `prov:ProgramTrace` | **21** |
| traced by `prov:DeclarationTrace` | **0** |

The chain carries 75 `DeclarationTrace` resources; none targets a plan. Since
`trace_category(wk::PROGRAM_TRACE)` is `None`, no plan emits a witness, so `Declared(plan)` cannot
resolve for any of the 21. `judgements-warrants-build-plan.md` §"Prerequisite found `2026-08-29`"
recorded this and set P4's shape to *"author 21 `DeclarationTrace`s (with agents and rationales)"*;
that did not land.

Nothing is broken by it today — no plan is cited as `DeclaredEvidence` anywhere on the chain — so
the gap is latent, waiting for the first consumer that cites one. **It is not this batch**, and it is
not what gates §2: component determinism and plan declaration are different assertions at different
sites. It needs its own issue.

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

### 3.5 An unregistered component fails, and the three phantom builtins go — #144

**Two halves, both in scope.**

**(a) The dispatch is an error.** `eval_hooks.rs:320` returns the input unchanged for a component
the registry does not hold — no error, no diagnostic, no `ComponentTrace`. Replace it with
`EvalError::ComponentDispatchFailed { component_iri, message: "component not registered" }`, the
variant the failure arm two branches down already uses.

Why it gates the batch: three of the four `deterministic: true` components are unregistered.

| `deterministic: true` | registered? |
|---|---|
| `Identity` | yes (builtin) |
| `Combine` | **no** |
| `Extract` | **no** |
| `Transform` | **no** |

The builtin registry holds `Identity` and `Checkpoint`; `REMOTE_COMPONENTS` holds `CompleteJson`,
`CompleteText`, `HttpRequest`. §2 mints provenance for a deterministic run, so without (a) the first
records this batch emits attest three components that returned their input untouched — and #144
records the consequence: a downstream certificate discharging `derived(output_iri, P)` against that
trace succeeds, so the grade is earned by a no-op.

**(b) The three declarations are deleted.** #144 offers a fork — *"either implement the three
declared builtins or mark them in the ontology as reserved"*. A third reading is the right one,
because **they are not implementable as declared**:

| | declared | needed to implement |
|---|---|---|
| `Combine` | *"Merge properties from multiple inputs into one resource"* | multiple inputs; `input_type` is a single `Class` |
| `Extract` | *"Extract specific properties from a resource"* | *which* properties; `argument_type: None` |
| `Transform` | *"Apply property mappings and renames"* | *which* mappings; `argument_type: None` |

Each names a parameterised operation and declares no parameter slot. Implementing them is not
finishing an unfinished builtin — it is designing three argument types first, which is a separate
piece of design and not this batch.

Nothing uses them. `grep` across `*.rs`, `*.ts`, `*.esl`, `*.json`, `*.md` returns one hit outside
the ontology: `docs/design/d6b-reasoning-trace-schema.md:207`, where `components:Extract` is a value
inside an illustrative `PureTrace` JSON example. That example needs a different component IRI, and
`Identity` serves.

Marking them reserved keeps a declaration that reads as a capability, which is the condition #144
names as what makes the defect reachable: *"an author reading the program ontology finds three
deterministic builtins with descriptions, no marker that they are unimplemented, and a runtime that
accepts them without complaint."* A reserved marker fixes the runtime half only if someone reads it.
Deleting removes the affordance.

**This half is a bootstrap edit.** `ontologies/program/program-ontology.json` is compiled into
`BOOTSTRAP_CHAIN` (`bootstrap/mod.rs:273`), so removing three resources moves the manifest hash and
forces a reseed. It therefore **rides #235's batched reseed** rather than paying for its own — and
(a) does not wait for it, since (a) is Rust-only.

### 3.6 The `INTO` multi-binding rejection gets a test — #150

`fiber.rs:424` rejects a multi-row `FIBER` with `INTO`, since one IRI cannot name two resources.
No test drives it. 3.1 modifies this evaluator, so pin the path before changing it.

## 4. #144, #145, #146, #149 — reviewed `2026-09-04`

### 4.1 #144 — moved into the batch in full

See §3.5. Both halves are in scope: the dispatch error (Rust-only, lands here) and deleting the
three phantom declarations (a bootstrap edit, rides #235's reseed).

### 4.2 #145 — reconsidered, and the case is stronger than §2 first allowed

`ComponentResponse` is `{success, output, error}` (`proto/eigenius.proto:767`). No field can carry a
`RuntimeInvocation`, so this is a protocol change, as originally judged.

Two facts change the weighting:

- **`RunRuntimeScript` declares `deterministic: false`.** Under §2 its run record is *evidence*, not
  provenance — an `Observed` leaf. The `RuntimeInvocation` is what makes that evidence mean
  anything: image digest, `random_seed`, `numerical_metadata`. An `Observed` leaf for an external
  run that does not say which image produced it is an observation of nothing in particular.
- **The precedent exists and works.** `DispatchExternalResponse.runtime_invocation_partial_cbor`
  (field 2) is populated by the orchestrator at `component_executor.ts:254` and parsed by the kernel
  at `external_institution.rs:256`. The component path needs the same field on a different message,
  not a new mechanism.

**Decision needed**: include it and accept a proto change plus regenerated TS on two paths, or ship
§2's stochastic branch with a known-thin `Observed` leaf for `RunRuntimeScript` and follow up. The
second is defensible only if it is written down at the emit site.

### 4.3 #146 — live, but latent, and less severe than the issue states

`compute_trace_key(component, input)` omits the argument and both `ComponentTrace` sites set
`argument_hash: None`. Confirmed.

The issue's failure — *"a second call with the same input and a different argument gets the first
call's output back and never runs"* — is **not reachable today**. The memo is determinism-gated
(`eval_hooks.rs:431`: *"Deterministic component — content-address memo is sound"*), and no component
is both `deterministic: true` and argument-taking:

| takes an argument | `CompleteText`, `CompleteJson` (`argument_type: Arguments`) — both `deterministic: false` |
|---|---|
| **deterministic** | `Identity`, `Combine`, `Extract`, `Transform` — all `argument_type: None` |

So the collision fires the first time someone declares a deterministic component that takes an
argument, and nothing warns them. **Out of this batch**, since it is not a record defect. Worth
either adding the argument to the key as insurance while §3.4 is in `trace.rs`, or recording the
determinism gate in the `TraceStore` doc comment that currently calls the divergence a defect
without noting what makes it unreachable.

### 4.4 #149 — decided `2026-09-04`: delete the field

`Exp::InstitutionInvoke.target_iri` is removed. Nothing sets it, and the surface its doc comment
names as the one that would cannot reach it.

**Why `INTO` cannot be that surface.** `FIBER … INTO` is evaluated by the fiber evaluator, a
query-plan path; `Exp::InstitutionInvoke` is a program term reduced by NbE. `INTO` never lowers to
an `InstitutionInvoke`. The fiber evaluator picks the response IRI itself
(`query/evaluate/fiber.rs:420`):

```rust
let (response_iri, persist_to_chain) = match &fc.into {
    Some(target) => { /* reject a second arrival at the same IRI */ (target.clone(), true) }
    None         => (fp.fiber_response_iri(clause_idx, binding_idx), false),
};
let mut stamped = Resource::new(response_iri.clone());
for (k, v) in response.properties() { stamped.set(k.clone(), v.clone()); }
```

It takes the IRI straight off the parsed clause, builds a fresh `Resource` at it, copies the
response's properties across, and pushes it to the `into_collector` for chain commit. Without
`INTO` it synthesises a query-scoped transient. The chain-reinsertion behaviour D14 §9.3 describes
is fully implemented — just not through this field.

**What the field's absence changes: nothing.** `institution/eval_hooks.rs:253` is
`match target_iri { Some(iri) => iri.clone(), None => deterministic_run_output_iri(…) }`, and the
`Some` arm is unreachable, so the content-hash IRI is already what every comorphism output gets.

**Surface — Rust-only, no reseed.** `urn:eigenius:program:target_iri` is **not declared** in
`program-ontology.json` (0 hits), and the ESL lowering that builds `ComorphismInvokeApply` sets
`function` and `source` and never a target. So no bootstrap ontology moves.

| file | what goes |
|---|---|
| `nbe/term.rs:272, 289, 735, 739` | the field on the variant, its doc line, and its clone in the structural traversal |
| `program/expr.rs:218-243` | the decoder reading `urn:eigenius:program:target_iri` |
| `nbe/eval/hooks.rs:77` | the `institution_invoke` trait parameter |
| `nbe/eval/mod.rs:538, 556`, `institution/eval_hooks.rs:122, 251-258, 588-590, 803` | call sites and the dead override branch |

Rule 12 keeps an authored `urn:eigenius:program:target_iri` from erroring after the decoder goes;
it becomes an unknown property and is ignored. Nothing in the tree authors one.

**In the batch**, since §3.1 opens the same evaluator and this is the third time the field has been
looked at without being resolved.

### 4.5 Unchanged

`#144`'s sibling defects in component registration, and any work implementing `Combine` / `Extract`
/ `Transform`, stay out.

## 5. Sequencing

1. **§2 first**, and it is now a small piece of work rather than only a decision: read
   `program:component:deterministic` at the minting site, defaulting absent to non-deterministic.
   Every other item writes against which record type it produces.
2. **§3.5(a) (#144) with §2**, and before any minting. An unregistered component must fail rather
   than return its input, or the first records the batch mints attest three components that never
   ran.
3. **3.6, then 3.1.** Pin the `INTO` error path before editing the evaluator.
4. **3.3 with 3.1**, since #206 constrains the failure path and both are the same error arm.
5. **3.2 after 3.1** — the resume path should mint what `RunProgram` mints, so it copies a shape
   rather than inventing one.
6. **3.4 last.** It changes the record's fields, so it lands once the set of minting sites is fixed.
7. **§4.4 (#149)** alongside 3.1 — same evaluator, and the deletion is Rust-only.
8. **§3.5(b)** whenever #235's reseed runs. It is the only item on this branch that touches a
   bootstrap ontology, and it does not gate anything else here.

**One ontology edit, deferred.** §3.5(b) removes three resources from the program ontology and
rides #235's reseed. Otherwise `prov:input` and `prov:trace_tree` are already declared. If 3.4's
decision is to drop `trace_tree`, that *is* a bootstrap edit and joins #235's batched reseed rather
than paying for its own.

## 6. Verification

Per `judgements-warrants-build-plan.md` §"Verification, every phase":

- `cargo test --workspace`, `cargo fmt --all -- --check`, `RUSTFLAGS="-D warnings" cargo clippy
  --workspace --all-targets`.
- **Written as failing tests first**: a resumed task commits an output resource and a trace (3.2); a
  failed `RunProgram` leaves its original record intact and mints nothing (3.3); a multi-row `FIBER
  … INTO` is rejected (3.6); applying an unregistered component IRI is a `ComponentDispatchFailed`,
  not an identity step (3.5a).
- **The count that matters**: after 3.1 and 3.2, every kernel-initiated run on a test chain has
  exactly one record. Measure it, do not assert it.
- After §3.5(b), and after 3.4 if it drops `trace_tree`: reseed, and check the resource count
  against `judgements-warrants-build-plan.md` §0's method.

## 7. What this batch does not claim

It does not make a computed conclusion more attested than it was — `ProgramTrace` grounds nothing
before and after. It does give the stochastic case its `Observed` leaf, which is a ground the chain
could not previously emit for a run.

A computed claim still rests on `App(Declared(plan), Observed(inputs))`, and the run record is not
a third ground. What changes for the deterministic case is only that the record exists, names its
input, and can be read back.

It does not close §2.1's plan-declaration gap, and it does not verify that a component declared
deterministic is one (#43).

#206 should be retitled and rewritten against §0 and §1 before the first commit, so the issue does
not keep asserting a premise the tree removed.
