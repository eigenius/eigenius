# D11: Codata, Streams, and Resumable Execution

*Design document for the Eigenius project — April 2026*

**Status:** Implemented (Phase 9b)
**Required before:** Phase 8.9 implementation
**Depends on:** D9 (NbE unification), D10 (Grothendieck institutions)

---

## 1. Motivation

Three capabilities converge in this phase:

1. **Persistent traces for crash recovery** — the RocksTraceStore exists but isn't wired into the server. Once wired, programs can resume after kernel restart.

2. **Concurrent task tracking** — the kernel currently executes programs synchronously within a gRPC call. Long-running programs (multi-step LLM pipelines, iterative simulations) need to run as tracked tasks that can be monitored, paused, and resumed.

3. **Incremental data processing** — some computations naturally progress as new data arrives. A monitoring pipeline, an evolving analysis, or a multi-round LLM conversation are not batch programs with a fixed input — they are *processes* that observe a stream of events and incrementally produce results.

Codata (coinductive types) provides the type-theoretic foundation that unifies all three. A resumable computation is a codata value: each observation produces a partial result and a continuation. Traces record the observation history. Streams are the canonical codata type.

---

## 2. Codata: Data's Dual

### 2.1 Data vs Codata

| | Data (Inductive) | Codata (Coinductive) |
|---|---|---|
| Defined by | Constructors (how to build) | Observations (how to use) |
| Consumed by | Pattern matching | Copattern matching |
| Totality | Must be finite (well-founded recursion) | May be infinite (productive corecursion) |
| Key property | All values are reachable | All observations terminate |
| Fixed point | Least (μ) | Greatest (ν) |
| Examples | `Nat`, `List A`, `Tree A` | `Stream A`, `Process I O`, `Task` |

Data is defined by how you create it. Codata is defined by how you observe it. A stream doesn't have constructors — it has two observations: `head` (returns the current element) and `tail` (returns the rest of the stream).

### 2.2 Copatterns

Copatterns (Abel, Pientka, Thibodeau, Setzer 2013) define codata by specifying what each observation returns:

```
codata Stream (A : Type) where
  head : Stream A → A
  tail : Stream A → Stream A

-- Define by copattern matching on which observation is made:
nats : Nat → Stream Nat
head (nats n) = n
tail (nats n) = nats (n + 1)
```

This dualizes pattern matching: instead of matching on *how the input was built*, you match on *which observation is being performed*.

### 2.3 Productivity

A corecursive definition is **productive** if every observation eventually returns a value. This is the coinductive analogue of termination for recursive definitions:

- **Termination** (data): every recursive call operates on a structurally smaller argument
- **Productivity** (codata): every corecursive call appears under an observation (guarded)

The simplest check: **syntactic guardedness** — corecursive calls must appear directly under a copattern observation, not nested inside other computations. This is what Agda uses.

---

## 3. Codata in EigenTT

### 3.1 Extensions Required

EigenTT currently has: Π, Σ, Sum (finite), Unit, Set, and the Eigenius extensions (Id, DecEq, NativeDecide, Type(n), EigonClass, EigonPrimitive, PropAccess). To add codata:

**New term formers:**

```
Exp::Codata(Vec<Observation>)       -- codata type declaration
Exp::CoRecord(Vec<CoField>)        -- codata value (copattern definitions)  
Exp::Observe(Box<Exp>, Name)        -- observation: e.head, e.tail
```

**New value constructors:**

```
Val::Codata(Vec<(Name, Exp)>, Rho)  -- codata type (dual of Val::Data)
Val::CoRecord(Vec<(Name, Exp)>, Rho) -- codata value (lazy observations)
```

**Evaluation rules:**

```
eval(Observe(e, obs)) =
  let v = eval(e)
  match v:
    CoRecord(fields, rho) →
      find obs in fields, eval the body in rho
    Nt(n) →
      Nt(NtObserve(n, obs))    -- blocked observation
```

**Readback:**

```
readback(CoRecord(fields, rho)) =
  CoRecord(fields.map(|(name, body)| (name, readback(eval(body, rho)))))
```

### 3.2 Typing Rules

```
Γ ⊢ A : Type    Γ, x : A ⊢ B(x) : Type
─────────────────────────────────────────
Γ ⊢ codata { obs₁ : T₁; obs₂ : T₂ } : Type

Γ ⊢ e₁ : T₁    Γ ⊢ e₂ : T₂
─────────────────────────────────────
Γ ⊢ corecord { obs₁ = e₁; obs₂ = e₂ } : codata { obs₁ : T₁; obs₂ : T₂ }

Γ ⊢ e : codata { ...; obsᵢ : Tᵢ; ... }
─────────────────────────────────────────
Γ ⊢ e.obsᵢ : Tᵢ
```

### 3.3 Guardedness Check

A definition is **guarded** if every corecursive occurrence appears directly as a copattern field:

```
-- Guarded (productive):
corecord nats(n) {
  head = n;
  tail = nats(n + 1);   -- corecursive call under observation
}

-- NOT guarded (may not be productive):
corecord bad {
  head = bad.head;       -- self-reference NOT under observation
}
```

The guardedness check is syntactic — scan the body of each observation field and verify that corecursive calls appear only in observation positions.

### 3.4 Termination, Productivity, and the Capability Mode Boundary

Total functional programming guarantees termination — every function call returns. Stream processors, by design, do not terminate. They run until an explicit end-of-stream message. This creates a fundamental tension: how does a system built on total programs accommodate computations that won't terminate?

The answer is that termination and productivity are **dual** totality guarantees, and the capability mode boundary already separates them:

**Pure mode — termination guaranteed.** EigenTT is strongly normalizing. Every pure program terminates. This is the domain of data types (inductive): finite, well-founded, every recursion reaches a base case.

**IO mode — termination depends on external services.** An IO component call may take arbitrarily long, fail, or never respond. Termination was already not guaranteed when we added IO dispatch to `eval_ctx`. The trace store mitigates this: completed IO steps are recorded and recoverable after crashes, but the system cannot guarantee that an IO step will ever complete.

**Stream mode — non-termination by design.** A stream processor runs until an explicit end-of-stream. This is the domain of codata types (coinductive): potentially infinite, productive (each observation returns in finite time), but the sequence of observations has no inherent bound.

The type system makes these distinctions explicit:

| Program kind | Termination | Guarantee | Type |
|-------------|-------------|-----------|------|
| Pure function | Always terminates | Type-checked (strong normalization) | `A → B` |
| IO program | Terminates if all IO steps complete | Trace-backed (recovery on crash) | `A → IO B` |
| Stream processor | Does not terminate (until end-of-stream) | Productive (each step terminates) | `A → Process Event Result` |

The key constraint: **a pure function cannot call a stream processor** without explicitly consuming a finite prefix. You must write `take(n, stream)` to extract data from a stream in a terminating context. A stream processor cannot appear where a terminating computation is expected. The capability modes enforce this boundary — Pure mode refuses IO and codata unfolding; IO mode permits both.

**The practical concern:** the kernel must not hang on a non-terminating stream within a synchronous gRPC call. This is solved by the task model (§5). Stream programs run as background tasks. The gRPC interface provides `step`/`cancel`/`status` operations — the client drives the observation loop, not the kernel. Each `step` call is synchronous and bounded; the sequence of steps is controlled by the client.

**Mixed inductive-coinductive types** handle the end-of-stream case:

```
codata Process Event Result where
  step : Event → Step Event Result

data Step Event Result where
  Continue : Resource → Process Event Result → Step Event Result
  Done     : Result → Step Event Result
```

Each `step` observation returns a finite `Step` value — either `Continue` (result + continuation) or `Done` (final result). The `Step` is data (must be finite). The `Process` is codata (may be infinite). Each step terminates. The process as a whole terminates only when it returns `Done`. Both outcomes are well-typed.

This is Abel's mixed inductive-coinductive types (2012): the guardedness checker verifies that each observation body terminates (the `Step` constructor is produced before the corecursive `Process` continuation). The sequence of observations may be infinite, but each observation is bounded.

---

## 4. Streams

### 4.1 Stream Type

```
codata Stream (A : Type) where
  head : A
  tail : Stream A
```

In ESL:

```esl
codata demo:IntStream {
  head : core:integer;
  tail : demo:IntStream;
}
```

### 4.2 Stream Operations

```esl
// Map over a stream
corecord map(f, s) : Stream B {
  head = f(s.head);
  tail = map(f, s.tail);
}

// Filter (partial — may not be productive if no elements pass)
// Requires sized types or fuel for safety

// Zip two streams
corecord zip(s1, s2) : Stream (A, B) {
  head = (s1.head, s2.head);
  tail = zip(s1.tail, s2.tail);
}

// Take n elements (codata → data conversion)
take(0, s) = [];
take(n, s) = s.head :: take(n - 1, s.tail);
```

### 4.3 Event Streams

An event stream in the knowledge graph:

```esl
codata demo:EventProcessor {
  react : demo:Event -> (core:resource_array, demo:EventProcessor);
}
```

Each call to `react` consumes an event and returns actions plus a continuation. The continuation is the next state of the processor.

---

## 5. Resumable Execution

### 5.1 Tasks as Codata

A program execution is a task — a codata value that can be observed (resumed):

```
codata Task (Result : Type) where
  step : Step Result

data Step (Result : Type) where
  Done    : Result → Step Result
  Yield   : Resource → Task Result → Step Result   -- partial result + continuation
  Blocked : ComponentIRI → Resource → (Resource → Task Result) → Step Result
```

- `Done`: the task completed, here's the result
- `Yield`: the task produced a partial result and can continue
- `Blocked`: the task is waiting for an IO component dispatch

### 5.2 Integration with NbE

The NbE evaluator in IO mode already handles blocking and resumption implicitly:

- An Apply to an IO component checks the trace cache → if cached, returns immediately (like a `Done`)
- If not cached, dispatches to the orchestrator (like a `Blocked`)
- Let-bindings chain computations (like sequential `step` observations)

Making this codata-explicit means:
1. `eval_ctx` in IO mode returns a `Task` codata value instead of a `Val`
2. The caller observes `step` to drive execution
3. Each step produces a trace entry
4. The task can be serialized (suspended) and deserialized (resumed) via its trace

### 5.3 Persistent Task State

With `RocksTraceStore`:
1. Program execution starts → task ID assigned
2. Each IO dispatch writes a ComponentTrace to RocksDB
3. If the kernel crashes → on restart, the task's trace history survives
4. Re-running the task → cached IO steps return instantly, execution resumes from the first untraced step
5. The task table records: task ID, program IRI, input IRI, status (running/suspended/completed), trace count

### 5.4 Concurrent Tasks

Multiple programs execute as concurrent tasks:
- Each task has its own trace collection (via the `dispatched_traces` in `EvalCtx`)
- Tasks share the ComponentTrace cache — if two tasks call the same component with the same input, the second hits the cache
- The kernel maintains a task table accessible via gRPC (list tasks, check status, cancel)
- Tasks don't share mutable state — they share the immutable layer chain and the trace cache

---

## 6. Codata and Institutions

### 6.1 Streams as Fiber Morphisms

A stream of refinements in the FEA institution is a morphism chain:

```
result₁ →[MeshRefinement]→ result₂ →[MeshRefinement]→ result₃ → ...
```

This is a codata value — you can always observe the next refinement. The stream is productive (each observation runs the FEA solver and produces a new result).

As a codata type:

```
codata RefinementStream {
  current  : fea:StressResult;
  refine   : fea:MeshRefinement;
  continue : RefinementStream;
}
```

### 6.2 Event-Driven Institutions

An institution that processes experimental data as it arrives:

```
codata AssayProcessor {
  ingest : assay:Measurement → (assay:DoseResponseUpdate, AssayProcessor);
}
```

Each measurement updates the dose-response curve and returns a new processor state. The institution's fiber grows incrementally as data arrives.

---

## 7. Traces and Codata

### 7.1 Observation Traces

Each observation on a codata value produces a trace entry:

```
CoObservationTrace {
  observation: String,        // "head", "tail", "react"
  result_trace: Option<Trace>, // trace of the observation's computation
  timestamp: DateTime,
}
```

The full history of a codata computation is a sequence of observation traces — exactly analogous to the ComponentTrace sequence for IO-driven programs.

### 7.2 Trace-Driven Codata Replay

To replay a codata computation:
1. Start with the codata definition
2. For each recorded observation, check the trace cache
3. If cached → return the stored result, advance to the continuation
4. If not cached → recompute the observation

This is the same replay semantics as IO-driven program resumption. Codata and IO traces share the same memoization infrastructure.

---

## 8. ESL Syntax

### 8.1 Codata Declarations

```esl
codata demo:IntStream {
  head : core:integer;
  tail : demo:IntStream;
}
```

### 8.2 Corecord Definitions

```esl
program demo:nats : core:integer -> demo:IntStream {
  corecord {
    head = input;
    tail = demo:nats(input + 1);
  }
}
```

### 8.3 Observations

```esl
// Take the first 3 elements
let s : demo:IntStream = demo:nats(0);
let first : core:integer = s.head;
let rest : demo:IntStream = s.tail;
let second : core:integer = rest.head;
```

---

## 9. Implementation Plan

### 9.1 Steps

1. Wire `RocksTraceStore` into `start_server` with `--db` flag
2. Add task table to the kernel (task ID, status, trace count)
3. Add `Exp::Codata`, `Exp::CoRecord`, `Exp::Observe` to EigenTT terms
4. Add `Val::Codata`, `Val::CoRecord` to values
5. Add eval rules for codata (observation reduction)
6. Add readback rules for codata
7. Add type checking rules (codata type formation, corecord checking, observation typing)
8. Add guardedness checker for productivity
9. Add ESL syntax for `codata`, `corecord`, observations
10. Add task management gRPC RPCs (ListTasks, CancelTask, GetTaskStatus)
11. Integration test: event-driven stream processing with traces

### 9.2 Complexity Assessment

| Component | Difficulty | Lines (est.) |
|-----------|-----------|-------------|
| Persistent trace store wiring | Low | ~50 |
| Task table | Low | ~150 |
| Codata term/value/eval/readback | Medium | ~200 |
| Codata type checking | Medium | ~100 |
| Guardedness checker | Medium | ~150 |
| ESL codata syntax | Low | ~100 |
| Task management RPCs | Low | ~100 |
| **Total** | | **~850** |

The core type theory extension (codata types, copatterns, guardedness) is well-understood from the literature. The main implementation risk is ensuring productivity checking is neither too strict (rejecting valid programs) nor too lenient (accepting divergent ones). Syntactic guardedness is the simplest and most predictable approach.

---

## 10. References

### Type Theory

- Abel, Pientka, Thibodeau, Setzer (2013) — *"Copatterns: Programming Infinite Structures by Observations"* (POPL 2013). Foundational paper on copatterns.
- Abel and Pientka (2013) — *"Wellfounded Recursion with Copatterns: A Unified Approach to Termination and Productivity"* (ICFP 2013). Sized types for mixed recursion/corecursion.
- Atkey and McBride (2013) — *"Productive Coprogramming with Guarded Recursion"* (ICFP 2013). The ▸ ("later") modality for productivity.
- Basold and Geuvers (2016) — *"Type Theory based on Dependent Induction and Coinduction"*. Extending CIC with coinduction.
- Hancock and Setzer (2000) — *"Interactive Programs in Dependent Type Theory"*. I/O as coinductive interaction.

### Interaction Trees and Effects

- Xia, Zakowski, et al. (2020) — *"Interaction Trees: Representing Recursive and Impure Programs in Coq"*. Coinductive representation of effectful programs.
- Plotkin and Pretnar (2013) — *"Handling Algebraic Effects"*. Algebraic effects as the dual of codata handlers.

### Streams and Process Calculi

- Jacobs (2017) — *"Introduction to Coalgebra"* (Cambridge). Reference text for coalgebraic methods.
- Hagino (1987) — *"A Categorical Programming Language"*. Original categorical treatment of data/codata duality.

### Guarded Recursion

- Nakano (2000) — *"A Modality for Recursion"*. The ▸ modality.
- Bahr, Grathwohl, Mogelberg (2017) — *"Clocked Type Theory"*. Clock variables for controlled unfolding.
- Birkedal et al. (2012) — *"First Steps in Synthetic Guarded Domain Theory"*. Semantic foundations.

---

## 11. Decisions

| Question | Decision | Rationale |
|----------|----------|-----------|
| Codata representation | Copatterns (Abel et al. 2013) | Integrates with existing pattern matching; well-understood typing rules |
| Productivity checking | Syntactic guardedness | Simplest, most predictable; can upgrade to sized types later |
| Stream model | `codata Stream A { head : A; tail : Stream A }` | Standard, matches the literature |
| Task model | Codata with `step` observation returning `Done \| Yield \| Blocked` | Natural fit for resumable computation; observations are resumption points |
| Trace integration | Observation traces share memoization infrastructure with IO traces | Same cache, same replay semantics, same crash recovery |
| Concurrent tasks | Shared trace cache, independent trace collections | Tasks share immutable state (layers, cache); no mutable sharing |
| ESL syntax | `codata`, `corecord`, `.observation` | Consistent with existing block + expression two-layer design |
