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

## 2. Run records are provenance; sampled outcomes are Observed

The paper's criterion, from §"Epistemic Scope of Observations":

> a sampled outcome reduces to a single `Observed` leaf. Details such as the specific instrument,
> the configuration parameters, and **the execution trace belong in the provenance graph, not within
> the justification term**. Furthermore, the underlying substrate is irrelevant: both a model
> invocation and a laboratory assay function as recordings under a declared protocol. The criterion
> separating them from computed conclusions is **whether the plan formalizes a deterministic
> function, not the medium of execution**.

### 2.1 What that settles

**The run record never changes class.** A `ProgramTrace` is provenance, always, and grounds nothing.
An earlier draft of this section proposed flipping it to an `ObservationTrace` for a stochastic run;
the paper puts the execution trace in the provenance graph, so that would have moved the wrong
object. Two resources, two roles:

| | |
|---|---|
| `ProgramTrace` on the run | provenance — unchanged by this batch except for §3.4's completeness fixes |
| `ObservationTrace` on the **output** | the `Observed` leaf, when the outcome is sampled |

This makes §2 **additive**. Nothing existing changes meaning, and the pieces are in place:
`programs.rs` already commits an activity for `prov:was_generated_by`, which is what
`prov:ObservationTrace` requires alongside `prov:resource`.

**Sampled is the realistic default, not the fallback.** The paper's criterion is whether the plan
formalizes a deterministic function. For a real scientific pipeline that formalization is rarely
available — and where someone would assert it, #43 records why it is shaky for Julia runs (BLAS
pinning, FMA discipline). Measured from the other end: 21 `stats:StatisticalAnalysisPlan` resources
on the WRN chain, **0** carrying a `prov:DeclarationTrace`, so nothing asserts the function
declaration `App(Declared(plan), Observed(inputs))` needs.

So: **mint the `ObservationTrace` on the output unless the plan formalizes a deterministic
function.** Nothing asserts that today, so in practice it is minted always, and
`program:component:deterministic` becomes the future *opt-out* rather than the predicate.

### 2.2 Why the determinism flag is not the predicate

Three shapes, not two. The third is the one this platform mostly runs, and a per-component flag
cannot see it:

| shape | ground | example |
|---|---|---|
| declared function over observed inputs | `App(Declared(plan), Observed(inputs))` | requires an assertion nobody has made |
| sampled outcome | a bare `Observed` leaf | a scientific pipeline, an assay, a model invocation |
| stochastic proposer behind a deterministic acceptor | whatever the **acceptor** licenses | the parser (LLMs rank, the kernel type-checks → **Declared**, eigenius#201); nanoda behind a Lean proof |

In the third, the stochasticity is in the search and not in the result, so the search record does
not ground the output — the acceptance does. A component's `deterministic: false` therefore does not
by itself make a run's record evidence; it does so only when nothing deterministic stands between
the stochastic step and the committed output. Reading the flag and *recording* it is sound;
inferring a ground from it is not.

### 2.3 What this dissolves

`judgements-warrants-build-plan.md` §"Prerequisite found `2026-08-29`" reads as work owed: *"author
21 `DeclarationTrace`s (with agents and rationales)"*, so that `Declared(plan)` resolves. Under §2.1
that is **not owed** — if those results are Sampled, the composite is the wrong model for them and no
plan witness is required. The plan stays a declared document; it simply is not carrying the function
declaration the composite would need.

Nothing on the chain currently cites a plan as `DeclaredEvidence`, which is consistent with the
composite never having been the operative shape for them.

### 2.4 Blast radius

Small, and not where it was assumed. `kernel/src/dcg/` and `kernel/src/server/formalize.rs`
reference none of `dispatch_component`, `RunProgram` or `ComponentRegistry` — the parsing pipeline
does not use the program-execution machinery at all. The only consumer of `components:CompleteJson`
/ `CompleteText` outside the ontology and the registry wiring is `demo/summarize-program.json`.

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

### 3.4 The record names what it ran on — #147

**No fork. The trace-tree half of #147 is wrong** (corrected on the issue, `2026-09-05`).
`prov:trace_tree` *is* read: `notebooks/src/runtime/traceResource.ts:56` maps `PROP.traceTree` to
it and `:146` flattens the kernel's right-leaning `Trace::Let` chain into siblings for the
notebook's trace panel. The inverse of `trace_to_resource` exists — in TypeScript, in the notebook
runtime.

The issue reads otherwise because its scope enumerates *"the kernel, the crates, the CLI or the
orchestrator"*, all four true, and `notebooks/` is none of them. Working from it, this plan
previously recommended dropping the field. That would have deleted a working feature, and the
reader carries a comment about a previous bug where it read the wrong namespace and *"the panel
rendered an empty tree beside a perfectly good trace"*.

So 3.4 is two small things, no bootstrap edit and no #235 dependency:

1. **Populate `prov:input`.** Genuinely never set — the only thing written is
   `reflection:input_hash` (`program/trace.rs:313`), a different property — while `prov:input`'s
   domain is `ProgramTrace` and the class recommends it.
2. **Delete the `// Required: trace_tree` comment** in `build_run_records`. It is `recommends`;
   `prov:ProgramTrace` requires only `prov:resource`, `prov:was_generated_by`, `prov:timestamp`.

### 3.5 An unregistered component fails, and the three phantom builtins go — #144

**Two halves, both in scope.**

**(a) The dispatch is an error. Done `2026-09-04`.**

**#144's stated mechanism is wrong — the third in this batch.** It points at
`dispatch_component`'s identity fallback (`eval_hooks.rs:320`), which the evaluator never reaches:
the `App` arm gates on `hooks.is_component(name)` (`nbe/eval/mod.rs:393`), so an unregistered IRI
takes the ordinary-application branch instead. `Exp::Var` for a name unbound in `rho` then yields a
neutral rather than an error, because in effectful mode *"unbound variables may be component IRIs
that will be intercepted at the App level"* (`mod.rs:415`) — and the interception did not happen.

Measured before fixing: an unregistered component **evaluates successfully** to
`Nt(App(Gen(usize::MAX, iri), arg))`. A stuck neutral, which flows into downstream `Construct`
fields and commits with a `ProgramTrace` over a step that never ran — the consequence #144
describes, reached by a different route.

The error therefore lands in the `App` arm, gated on the name being unbound so an ordinary
lambda-bound head is untouched. `dispatch_component`'s fallback becomes an error too rather than
being deleted: it is a public trait method, and a caller skipping the predicate should get a
diagnostic instead of its input back.

**It caught a test asserting the defect.** `institution_invoke_runs_four_step_pipeline_end_to_end`
named a transformation that existed nowhere, with a comment saying so — *"No real Component —
dispatch_component falls back to identity for unknown component IRIs, which is what we want for this
structural test."* Fixed by registering a real identity component under that IRI
(`program::component::identity_component()`); the test's subject, the comorphism pipeline, never
needed the fallback.

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
forces a reseed. It therefore **leaves this branch for the D86/D87/#235 follow-on batch**, where
the reseed is paid once for all of them — and
(a) does not wait for it, since (a) is Rust-only.

### 3.6 The `INTO` multi-binding rejection gets a test — #150

`fiber.rs:424` rejects a multi-row `FIBER` with `INTO`, since one IRI cannot name two resources.
No test drives it. 3.1 modifies this evaluator, so pin the path before changing it.

## 4. #144, #145, #146, #149 — reviewed `2026-09-04`

### 4.1 #144 — moved into the batch in full

See §3.5. Both halves are in scope: the dispatch error (Rust-only, lands here) and deleting the
three phantom declarations — a bootstrap edit, so it goes with the D86/D87/#235 batch, not here.

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

1. **§2 first.** Mint a `prov:ObservationTrace` on a run's output alongside the existing
   `ProgramTrace`, which is unchanged. Additive, so every other item writes against a settled shape.
2. **§3.5(a) (#144) with §2**, and before any minting. An unregistered component must fail rather
   than return its input, or the first records the batch mints attest three components that never
   ran.
3. **3.6, then 3.1.** Pin the `INTO` error path before editing the evaluator.
4. **3.3 with 3.1**, since #206 constrains the failure path and both are the same error arm.
5. **3.2 after 3.1** — the resume path should mint what `RunProgram` mints, so it copies a shape
   rather than inventing one.
6. **3.4 last.** It changes the record's fields, so it lands once the set of minting sites is fixed.
7. **§4.4 (#149)** alongside 3.1 — same evaluator, and the deletion is Rust-only.
8. ~~**§3.5(b)**~~ — **not on this branch.** It is the only item here that touches a bootstrap
   ontology, and it gates nothing else, so it moves to the D86/D87/#235 follow-on batch where one
   reseed covers D86's `≤`/`==` declarations, D87 §5's two `prov:VerificationTrace` slots, the two
   false descriptions, and this.

**No ontology edit on this branch.** §3.5(b) removes three resources from the program ontology and
travels with the D86/D87/#235 batch instead. `prov:input` and `prov:trace_tree` are already
declared, so §3.4 needs none. If 3.4's
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
- **GATE CLOSED `2026-09-05`.** `kernel/src/server/programs.rs` has a `mod tests`.
  `execute_program` is `pub(super)`, so an in-module test reaches it, and
  `EigeniusService::with_persistent_backend(ComponentRegistry::default(), MemoryPersistentBackend)`
  supplies the task store `new()` leaves `None` — no server, port or proto marshalling. Two
  fixture notes worth keeping: the output class must resolve in the bootstrap chain or the run
  succeeds and the *commit* fails `UnresolvedClassReference` (`prov:Agent` requires nothing and
  recommends `core:short_name`, so a `Construct` over it validates with no seeded layer), and the
  tests need `#[tokio::test(flavor = "multi_thread")]` because the commit hook calls
  `block_in_place`.

  What follows was the state before that, kept because it is why three items landed verified
  only indirectly:

- ~~**UNMET GATE — the handler is undrivable.**~~ §6 asks for "a failed `RunProgram` leaves its
  original record intact and mints nothing" (3.3) as a failing test first. It cannot be written
  today. `execute_program` is `pub(super)`, so only a unit test inside `kernel/src/server/` reaches
  it — and `EigeniusService::new()` sets `task_store: None`, which is exactly what 3.3's path is
  gated on, so a bare service skips it. The one existing harness, `kernel/tests/server_integration.rs`,
  stands up a tonic server on an ephemeral port and drives it over the wire.

  Closing it needs either a service constructed with a backend + task store for in-module unit
  tests, or an over-the-wire `RunProgram` test. **It would close the gate for three items** — §2's
  emission, §3.2 and §3.3 — and until it lands those three are verified only indirectly. Three
  items depending on an untested handler is the shape eigenius#207 had.

- **A gap this batch inherits and now depends on**: nothing in the tree drives
  `server::programs::execute_program`. It is `pub(super)` behind the gRPC service, and no test
  under `kernel/tests/` or any crate exercises `RunProgram`. §2's emission is therefore pinned
  indirectly — `prov_layer_smoke::a_run_output_with_an_observation_trace_admits_observed` builds
  the trace exactly as that function builds it and asserts the witness follows, with a negative
  control (the same resource as a `ProgramTrace` must not admit). That pins the emit/read contract,
  not the emission. A harness driving the handler would close it and is worth its own item.
- After §3.5(b), and after 3.4 if it drops `trace_tree`: reseed, and check the resource count
  against `judgements-warrants-build-plan.md` §0's method.

## 7. What this batch does not claim

It does not change what a `ProgramTrace` grounds — nothing, before and after. It adds the
`Observed` leaf a sampled outcome is owed, on the run's **output**, which is a ground the chain
could not previously emit for a run (§2.1).

A computed claim still rests on `App(Declared(plan), Observed(inputs))`, and the run record is not
a third ground. What changes for the deterministic case is only that the record exists, names its
input, and can be read back.

It does not close §2.1's plan-declaration gap, and it does not verify that a component declared
deterministic is one (#43).

#206 should be retitled and rewritten against §0 and §1 before the first commit, so the issue does
not keep asserting a premise the tree removed.
