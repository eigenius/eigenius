# D9: NbE Unification, Type Theory Extensions, and Trace-Driven Execution

*Design document for the Eigenius project — April 2026*

**Status:** Implemented (Phase 5; Phase 10 extensions)
**Required before:** Phase 5 implementation
**Resolves:** Integration of Eigon type system with EigenTT/CIC, trace persistence, incremental execution, validation/type-checking relationship

---

## 1. Problem Statement

The Eigenius kernel has two disconnected systems for checking correctness:

1. **Ontology validation** (`validation/mod.rs`): checks resources against class definitions. 12 rules covering required properties, type matching, formats, ranges, cardinality, and the open-world assumption. Sound and complete for structural resource correctness.

2. **EigenTT type checking** (`nbe/check.rs`): checks program expressions against dependent types. Correctly ported from the academic reference for core EigenTT (Pi, Sigma, Sum, Unit), but stubs for all Eigon-specific extensions.

Additionally, the **program executor** (`execute.rs`) ignores both systems entirely — it interprets programs dynamically with no type information at runtime.

This document specifies:
- How to extend EigenTT with Lean 4-inspired features to close the type-theoretic gaps
- How to make NbE trace-aware for incremental execution
- How validation and type checking relate (complementary, not redundant)
- The migration path from the current architecture

---

## 2. Current State Analysis

### 2.1 What works

- EigenTT core: Pi, Sigma, Sum, Unit types with NbE (eval, readback, check)
- Expression forms map 1:1 from Eigon-JSON to EigenTT terms
- Ontology validation is sound for the 12 structural rules
- Trace recording in the executor (Trace enum, TraceStore, memoization)
- ESL compiler produces valid Eigon-JSON resources

### 2.2 What's broken

| Component | Issue |
|-----------|-------|
| `ground.rs` | Drops ~60% of ontology metadata (recommends, allows_only, conditional_requires, formats, ranges, cardinality) |
| `check.rs` | PropAccess returns `Val::Set` unconditionally (stub) |
| `expr.rs` | Literals lose their values (`"hello"` → `EigonPrimitive(String)`, the type not the value) |
| `execute.rs` | Never calls the type checker. Fully dynamic. |
| `val.rs` | Values carry no type information. `Val::EigonClass(Iri)` has no schema. |

### 2.3 The two-system split

```
Resources → Validator → constraint errors     (works)
Programs  → NbE check → type errors           (stubs for Eigon)
Programs  → Executor  → results               (ignores types)
```

The validator and type checker don't compose. The executor respects neither.

---

## 3. Design Principles

1. **EigenTT core is correct and stays unchanged.** Extensions are additive — new term formers, new value constructors, new reduction rules. The existing NbE algorithm (eval/readback/check) keeps its structure.

2. **Borrow design patterns from Lean 4, not implementation.** Decidable propositions, native decide, structure inheritance — adapted to our scale.

3. **Validation and type checking are complementary.** Validation checks the open-world data surface (any resource, any properties). Type checking checks the closed-world program surface (declared properties, typed composition). They meet at the boundary: validation guarantees resources are well-formed; type checking guarantees programs handle well-formed resources correctly.

4. **Traces are computational bookkeeping, not part of types.** `eval` produces `(Val, Option<Trace>)`. Readback strips traces. Type equality ignores traces.

5. **Incremental execution is NbE with a trace-aware environment.** A traced IO component is already in normal form — no re-evaluation needed.

---

## 4. Type Theory Extensions (Lean 4-inspired)

### 4.1 Decidable Equality on Ground Types

Add a built-in equality check for ground types:

```
DecEq : (A : GroundType) → (x y : A) → Decidable (x = y)
```

For each ground type, the kernel has a decision procedure:
- `String`: byte-wise comparison
- `Integer`: numeric equality
- `Float`: IEEE 754 equality
- `Boolean`: trivial
- `IRI`: string equality on the IRI representation

**Implementation:** Add `Val::DecEq(GroundType)` to the value domain. During type checking, when the checker encounters an equality constraint on ground types, it calls the decision procedure and treats the result as a proof.

**NbE impact:** `eval` reduces `DecEq(String, "hello", "hello")` to `Val::Refl`. `eval` reduces `DecEq(String, "hello", "world")` to `Val::Absurd`. New reduction rules, existing algorithm.

### 4.2 Propositional Equality (Id Type)

Add the identity type:

```
Id : (A : Type) → A → A → Type
refl : (a : A) → Id A a a
J : (A : Type) → (C : (x y : A) → Id A x y → Type) →
    ((x : A) → C x x (refl x)) → (x y : A) → (p : Id A x y) → C x y p
```

This is standard Martin-Löf identity. The `J` eliminator allows transporting proofs along equalities.

**Implementation:** Add `Exp::Id(Box<Exp>, Box<Exp>, Box<Exp>)`, `Exp::Refl(Box<Exp>)`, `Exp::J(...)` to the term language. Add corresponding `Val` constructors. Evaluation reduces `J` when the proof is `refl`.

**NbE impact:** `eval(J(A, C, d, x, x, refl(x))) = eval(d(x))`. Standard reduction rule. Readback produces `Id` terms for neutral equalities.

### 4.3 Native Decide for Constraints

Lean's `native_decide` pattern: during type checking, evaluate decidable propositions by calling native code.

For Eigenius constraints:

| Constraint | Proposition | Decision procedure |
|-----------|-------------|-------------------|
| `min_value = 0` | `x ≥ 0` | Integer comparison |
| `max_length = 100` | `length(s) ≤ 100` | String length check |
| `pattern = "^[A-Z]"` | `matches(s, regex)` | Regex engine |
| `format = date` | `is_valid_date(s)` | Date parser |
| `allows_only = [a, b, c]` | `x = a ∨ x = b ∨ x = c` | Finite disjunction via DecEq |

**Implementation:** Add `Exp::NativeDecide(Constraint, Box<Exp>)` as a term. During evaluation, `eval` calls the corresponding Rust function and produces `Val::Refl` (satisfied) or `Val::Absurd` (violated).

**NbE impact:** New reduction rules for each constraint kind. The existing `validation/mod.rs` functions (is_valid_date, is_valid_uuid, etc.) are reused as decision procedures.

### 4.4 Universe Stratification

Replace the single `Set` with a fixed hierarchy:

```
Type 0 : Type 1 : Type 2
```

- **Type 0**: domain resources, program values
- **Type 1**: ontology definitions, traces about domain resources
- **Type 2**: meta-ontology (core ontology definitions, traces about traces)

The core ontology's self-description (`Class is_a Class`) lives at Type 2.

**Implementation:** Replace `Exp::Set` with `Exp::Type(usize)`. `Val::Set` becomes `Val::Type(usize)`. Universe checking: `Type n : Type (n+1)`. A term at level n can reference types at level n-1 or below.

**NbE impact:** `eval(Type(n))` produces `Val::Type(n)`. Readback preserves levels. The existing `eq_nf` check compares universe levels.

### 4.5 Closed Records with Optional and Dynamic Access

Classes map to Sigma types with three zones:

```
Dog = Σ (name : String)          -- required (from requires)
     . Σ (breed : String)         -- required (from requires)
     . Σ (color : Option String)  -- optional (from recommends)
     . Unit
```

**Required properties:** `resource.name : String` — total access, guaranteed by type.

**Recommended properties:** `resource.color : Option String` — declared but optional. Programs must handle `None`.

**Undeclared properties:** `resource.get(iri) : Option Value` — dynamic access via a built-in function. Returns `Option Value` where `Value` is an untyped dynamic value. The program must pattern-match on the result.

```
get : Resource → IRI → Option Value
```

**Implementation:**
- `ground.rs` maps `requires` properties to Sigma components with their declared types
- `ground.rs` maps `recommends` properties to `Option T` Sigma components
- Add `Exp::Get(Box<Exp>, Iri)` for dynamic property access, typed as `Option Value`
- `Val::Option(Option<Box<Val>>)` for optional values with `Some`/`None`

**NbE impact:** `eval(PropAccess(resource, prop))` reduces by looking up the property in the resource value. If declared and present → the value. If declared and absent (recommended) → `None`. The `Get` form always returns `Option Value`.

This resolves the PropAccess stub: instead of returning `Val::Set`, the type checker looks up the property IRI in the class's Sigma type and returns the declared type (or `Option T` for recommended, or rejects if undeclared without `get`).

---

## 5. Effect-Aware NbE and Trace Production

### 5.1 The Evaluation Signature

Current: `eval(exp, rho) -> Val`

New: `eval(exp, rho, ctx) -> (Val, Option<Trace>)`

Where `ctx` is an evaluation context providing:
- `layer: &Layer` — for ground type resolution
- `trace_store: &dyn TraceStore` — for memoization lookup/storage
- `component_dispatch: &dyn ComponentDispatch` — for IO component execution

### 5.2 Capability Modes

Programs declare a capability level: Pure, Read, or IO. The kernel invokes `eval` with the matching mode. See §7 for the full capability mode design.

| Level | What eval can do | Trace production |
|-------|-----------------|------------------|
| Pure | Reduce terms, no side effects | No traces (pure computation) |
| Read | Pure + read from layer chain | No traces (deterministic reads) |
| IO | Read + dispatch to orchestrator | ComponentTraces for each dispatch |

Ordering: Pure ≤ Read ≤ IO. A pure function can be called from IO context (its reduction rules are a subset). An IO component in pure mode produces a neutral term — it's blocked, not an error.

### 5.3 Incremental Execution via Traces

When `eval` in IO mode encounters an Apply to an IO component:

1. Compute the trace cache key: `SHA-256(component_iri || canonicalize(input))`
2. Check `trace_store.get(key)`:
   - **Hit:** return `(cached_output, ComponentTrace { cached: true, ... })`
   - **Miss:** dispatch to orchestrator, store result, return `(output, ComponentTrace { cached: false, ... })`

### 5.4 Crash Recovery

If execution crashes mid-program:
1. Re-evaluate from the top (invoke `eval` in IO mode)
2. All completed IO components hit the trace cache (instant)
3. Evaluation resumes from the first untraced IO component
4. The trace tree is rebuilt incrementally

No explicit checkpointing needed — the trace store *is* the checkpoint.

### 5.5 ProgramTrace Construction

After full evaluation, `eval` returns `(output, trace_tree)`. The kernel wraps this in a `ProgramTrace` resource:

```json
{
  "@id": "urn:eigenius:trace:exec-<uuid>",
  "urn:eigenius:core:is_a": ["urn:eigenius:reflection:ProgramTrace"],
  "urn:eigenius:reflection:program": "<program IRI>",
  "urn:eigenius:reflection:input": "<input IRI>",
  "urn:eigenius:reflection:output": "<output>",
  "urn:eigenius:reflection:trace_tree": { ... },
  "urn:eigenius:reflection:total_tokens": 4020,
  "urn:eigenius:reflection:started_at": "...",
  "urn:eigenius:reflection:completed_at": "..."
}
```

Stored via the Reflect RPC and queryable via EigenQL.

### 5.6 Trace Pruning (Proofs as Programs)

A deterministic computation is its own proof — you don't need to store the proof separately. You store the axioms (IO results) and the theorem (program), and the proof can be reconstructed by re-evaluation.

After a program completes, the trace tree is pruned. Only non-deterministic IO ComponentTraces are retained:

| Trace type | Deterministic? | Pruning action |
|-----------|---------------|----------------|
| ComponentTrace (IO) | **No** | **Keep** — non-deterministic, memoization cache entry |
| PureTrace | Yes | Discard — re-computable from inputs |
| LetTrace | Yes | Discard — structural, recoverable from program |
| ConstructTrace | Yes | Discard — re-computable |
| ProjectTrace | Yes | Discard — re-computable |
| CaseTrace | Yes | Discard — branch determined by scrutinee |
| MapTrace | Yes (structure) | Discard structure, extract nested IO traces |
| ReduceTrace | Yes (structure) | Discard structure, extract nested IO traces |

**The pruning algorithm:**

1. Walk the trace tree
2. At each node: if it's an IO ComponentTrace, extract it to the flat trace store keyed by `SHA-256(component_iri || canonicalize(input))`
3. Discard everything else

**What survives:**
- A flat set of `(cache_key, ComponentTrace)` pairs in the trace store — exactly the entries needed for memoization
- The ProgramTrace metadata (total_tokens, latency, timestamps) as a summary resource
- The program IRI and input IRI — sufficient to reconstruct the full trace tree by re-evaluation

**Reconstruction:** to recover the full trace tree, re-evaluate the program with the same input in IO mode. All IO components hit the trace cache and return instantly. The deterministic structure rebuilds itself. The result is identical to the original trace tree.

**When to prune:** immediately after ProgramTrace construction. The full trace tree is built during evaluation, the ProgramTrace metadata is computed from it (total_tokens, cached_steps, etc.), then the tree is flattened to IO traces only.

**Implication for EigenQL queries on traces:** queries that navigate the trace tree structure (e.g., "which LetTrace bound variable X?") are not available on pruned traces. If full tree queries are needed, the user can re-evaluate to reconstruct the tree. For most use cases — "which LLM calls contributed?", "what was the total token usage?", "was this result cached?" — the flat ComponentTraces and ProgramTrace summary are sufficient.

### 5.7 Trace Storage Architecture

Traces live in two places serving different purposes:

**ComponentTrace cache (RocksDB key-value store):**
- Written individually during execution for memoization and crash recovery
- Content-addressed by `SHA-256(component_iri || canonicalize(input))`
- Survives kernel crashes (RocksDB WAL guarantees)
- Fast key-value lookup for memoization during evaluation
- **Not** the authoritative record — it's a performance cache

**Trace layer (layer system — the authoritative record):**
- After execution completes, a **trace layer** is committed extending the layer chain
- Contains both the ProgramTrace summary *and* all IO ComponentTraces as resources with `@id`
- The ProgramTrace references the ComponentTrace IRIs — forming a proof chain
- Queryable via EigenQL — "which LLM calls contributed to this result?"
- Immutable and content-addressed — the trace layer is a proof artifact

The dual-write design:
1. **During execution:** ComponentTraces are written to the side cache as they complete (for memoization and crash recovery)
2. **After execution:** the surviving IO ComponentTraces are committed as resources in a new trace layer, alongside the ProgramTrace summary
3. The side cache enables fast re-execution; the trace layer enables provenance queries and proof verification

**Why both?** The side cache handles the write-during-execution requirement that atomic layer commits can't serve. The trace layer puts ComponentTraces in the graph where they can be queried, validated, and used as proof artifacts. They contain the same data — the side cache is the fast path, the layer is the truth.

### 5.8 Trace Layers as Proof Artifacts

A trace layer extends the chain:

```
core → program → reflection → user_data → trace_layer₁ → trace_layer₂ → ...
```

Each program execution appends a trace layer containing:
- `ProgramTrace` — summary with program IRI, input IRI, metrics
- `ComponentTrace₁, ComponentTrace₂, ...` — the IO axioms (LLM calls, HTTP requests)

**Proof verification:**
1. Look up ProgramTrace `T` in the trace layer
2. `T` references program `Π` and input `I`
3. `T` references ComponentTraces `C₁, C₂, ...`
4. Re-evaluate `Π(I)` in IO mode — each `Cᵢ` hits the cache
5. Output matches the recorded output ✓

The ComponentTraces are the **axioms** (non-deterministic IO results). The program is the **theorem**. Re-evaluation is the **proof** — it reconstructs the derivation from axioms. The trace layer is the **proof certificate** — it records the axioms in the graph so the proof can be checked without re-dispatching IO.

### 5.9 Crash Recovery

The dual-write design guarantees correct crash recovery:

**Crash during execution:**
1. ComponentTraces written to the side cache before the crash survive (RocksDB WAL)
2. No trace layer exists (execution didn't complete)
3. Client retries `RunProgram`
4. Kernel re-evaluates — completed IO steps hit the cache instantly
5. Evaluation resumes from the first untraced IO component
6. After completion, trace layer is committed with all ComponentTraces + ProgramTrace

**Crash after execution, before response:**
1. ComponentTraces survive in the side cache
2. Trace layer may or may not be committed
3. Client retries `RunProgram`
4. All IO steps hit the cache — re-execution is instant
5. Trace layer is committed (idempotent — same content produces same layer ID)

**Auto-commit policy:** `RunProgram` commits the trace layer after successful execution and returns the ProgramTrace IRI in `RunProgramResponse`.

### 5.10 Known Gap: Persistent Trace Store Wiring

**Current state (Phase 5):** The server uses `InMemoryTraceStore`. Within a server session, ComponentTrace memoization works — re-running the same program with the same input hits the cache. Incremental execution works within a session.

**What's missing:** The server does not use the `RocksTraceStore` implementation. After a crash and restart, the in-memory trace store is lost, and all IO components re-dispatch. The `RocksTraceStore` exists and is tested (including persistence across reopen), but wiring it into the server requires:
- A `--db` flag on `serve` (storage path)
- `start_server` passing a `RocksTraceStore` via `with_trace_store`
- Data directory conventions for production deployment

**Resolution:** Phase 9 (Azure deployment & operations). The `RocksTraceStore` is a drop-in replacement — once a DB path is configured, crash recovery and cross-session memoization work automatically. No architectural changes needed; this is a deployment configuration concern.

---

## 6. Validation and Type Checking: Division of Responsibilities

### 6.1 Complementary, Not Redundant

| Concern | Checked by | When |
|---------|-----------|------|
| Required properties present | Validation | Data ingestion (Load RPC) |
| Property value types correct | Validation + Type checking | Ingestion + program compilation |
| Format constraints (date, UUID, regex) | Validation | Data ingestion |
| Range constraints (min/max value) | Validation | Data ingestion |
| Pattern constraints | Validation | Data ingestion |
| allows_only (enum values) | Validation | Data ingestion |
| Open-world extra properties allowed | Validation | Data ingestion |
| Program types compose (functions, let bindings) | Type checking | Program compilation |
| Property access on declared types | Type checking | Program compilation |
| Component input/output types match | Type checking | Program compilation |
| Capability level respected (pure/read/io) | Type checking | Program compilation |

### 6.2 The Boundary

**Validation guarantees:** a resource in the knowledge graph is structurally well-formed against its class definition. All constraints satisfied. Extra properties allowed.

**Type checking guarantees:** a program that type-checks will not encounter runtime type errors when processing well-formed resources. Declared property accesses succeed. Components receive compatible inputs. Capability levels are respected.

**Neither guarantees:** that a program produces *correct* results (semantic correctness is the domain institution's job, Phase 6).

### 6.3 Validation as an Institution (Phase 6 Preview)

When the Grothendieck institution framework arrives, the validator becomes the **Eigon structural institution**:
- Signatures: ontology snapshots (layer configurations)
- Sentences: constraint predicates (the 12 validation rules)
- Models: layers (collections of resources)
- Satisfaction: the current validation logic

The EigenTT type checker becomes the **program institution**:
- Signatures: type contexts (Gamma environments)
- Sentences: typing judgments
- Models: well-typed terms
- Satisfaction: the check/infer algorithm

Program validation is the **comorphism** between them: the structural institution translates a program resource into a typing judgment that the program institution checks.

This formalization doesn't change the implementation — it gives the two-system architecture a formal foundation and makes it extensible to domain institutions.

---

## 7. Capability Modes: One Evaluator, Multiple Modes

### 7.1 Design

There is no separate executor. NbE `eval` is the evaluator for type checking *and* execution. The difference is the **capability mode** — an `EvalCtx` parameter that controls what effects are available:

```rust
pub enum CapabilityMode {
    /// Standard NbE: normalize terms, check types. No side effects.
    Pure,
    /// Pure + read access to the layer chain. Ground type resolution,
    /// property lookups, ontology queries.
    Read { layer: Arc<Layer> },
    /// Read + IO component dispatch + trace production.
    /// Full program execution.
    IO {
        layer: Arc<Layer>,
        trace_store: Arc<dyn TraceStore>,
        component_dispatch: Arc<dyn ComponentDispatch>,
    },
}
```

The same `eval` function handles all three:

```rust
pub fn eval(exp: &Exp, rho: &Rho, mode: &CapabilityMode) -> Result<(Val, Option<Trace>), EvalError>
```

- **Pure mode:** `eval` refuses IO dispatch. Component applications produce neutral terms (blocked). Used during type checking.
- **Read mode:** `eval` can resolve class IRIs from the layer chain. Property access returns typed values. Used during elaboration and ground type resolution.
- **IO mode:** `eval` dispatches IO components to the orchestrator, checks the trace store for memoization, produces traces. Used during program execution.

A program declares its capability level. The kernel invokes `eval` with the matching mode. If a pure program attempts IO, `eval` returns a neutral term (not an error — it's a term that can't reduce further).

### 7.2 How the Current Code Maps

| Current | After |
|---------|-------|
| `nbe/eval.rs` — pure NbE | `eval` in Pure mode (unchanged behavior) |
| `nbe/check.rs` — type checking | Calls `eval` in Pure or Read mode |
| `program/execute.rs` — dynamic executor | `eval` in IO mode |
| `program/ground.rs` — ground type resolution | Called by `eval` in Read/IO mode |
| `program/trace.rs` — trace types | Produced by `eval` in IO mode |

`execute.rs` is not removed — it's absorbed. Its logic (Apply dispatch, Let binding, Construct, Project) moves into `eval` as reduction rules for the Eigon extensions. The existing EigenTT reduction rules stay unchanged.

### 7.3 What Changes in eval

**New reduction rules** (additive, don't change existing rules):

| Term | Pure mode | Read mode | IO mode |
|------|-----------|-----------|---------|
| `PropAccess(e, prop)` | Neutral (blocked) | Reduce: look up property in resource value | Same as Read |
| `Apply(component, arg)` where component is IO | Neutral (blocked) | Neutral (blocked) | Check trace cache → dispatch or return cached |
| `Apply(component, arg)` where component is pure | Reduce (call component) | Reduce | Reduce + produce PureTrace |
| `EigonClass(iri)` | Return as value | Resolve from layer → Sigma type | Same as Read |
| `NativeDecide(constraint, val)` | Neutral (blocked) | Reduce: evaluate constraint | Same as Read |

**Existing reduction rules** (unchanged):

| Term | Behavior |
|------|----------|
| `App(Lam(p, body), arg)` | Substitute and reduce (standard beta) |
| `Fst(Pair(a, b))` | Return a |
| `Case(Con(c, v), branches)` | Select branch c, apply to v |
| All other EigenTT rules | Unchanged |

### 7.4 Trace Production

Only IO mode produces traces. The return type of `eval` is `(Val, Option<Trace>)`:

- Pure/Read mode: always returns `(val, None)`
- IO mode: returns `(val, Some(trace))` for expressions that involve computation

Readback ignores traces — `rbV(val, trace)` produces the same `Exp` as `rbV(val, None)`. Traces are bookkeeping, not logical content.

---

## 8. Migration Path

### 8.1 Step 1: Trace Persistence (Low Risk)

Add Reflect RPC and trace persistence to the existing executor. No NbE changes.

- Implement `Reflect` RPC handler
- Wire `execute_program_traced` into `RunProgram` — auto-create ProgramTrace
- Store ComponentTraces in RocksDB with content-addressed keys
- Return trace IRI in RunProgramResponse
- CLI `reflect` command

This is eigenius/eigenius#5. Ships independently. The existing executor gains persistence without architectural change.

### 8.2 Step 2: Type Theory Extensions (Medium Risk)

Extend EigenTT with Lean-inspired features. Independently testable additions.

1. Add `Id` type, `refl`, `J` eliminator
2. Add `DecEq` for ground types
3. Add `NativeDecide` for constraints
4. Add `Type(n)` universe stratification (3 levels)
5. Complete `ground.rs` — map all ontology features to types (requires, recommends as Option, constraints as propositions)
6. Fix PropAccess in `check.rs` — resolve property types from the ontology

Each extension can be landed and tested independently. The existing type checker keeps working — new features are additive.

### 8.3 Step 3: Capability Modes in eval (Medium Risk)

Add the `CapabilityMode` parameter to `eval`. This is the unification step.

1. Add `CapabilityMode` enum and `EvalCtx`
2. Add reduction rules for Eigon extensions (PropAccess, IO dispatch, NativeDecide)
3. Route type checking through `eval` in Pure/Read mode (verify same behavior)
4. Route execution through `eval` in IO mode (verify same behavior as current executor)
5. Add trace production in IO mode
6. Incremental execution via trace cache in IO mode
7. Deprecate `execute.rs` — all callers use `eval` with IO mode

The key risk mitigation: Steps 1-4 can be tested against the existing type checker and executor outputs. If `eval` in Pure mode produces the same results as the old `eval`, and `eval` in IO mode produces the same results as the old `execute.rs`, the migration is correct.

---

## 9. Implementation Estimates

| Step | Description | Lines | Risk |
|------|-------------|-------|------|
| 1 | Reflect RPC, trace persistence, ProgramTrace | ~300 | Low |
| 2.1 | Id type, refl, J eliminator | ~150 | Medium |
| 2.2 | DecEq for ground types | ~100 | Low |
| 2.3 | NativeDecide for constraints | ~200 | Medium |
| 2.4 | Universe stratification (3 levels) | ~100 | Low |
| 2.5 | Complete ground.rs (requires + recommends + constraints) | ~200 | Medium |
| 2.6 | Fix PropAccess type inference | ~100 | Low |
| 3 | Capability modes in eval, deprecate execute.rs | ~400 | Medium |
| | **Total** | **~1550** | |

---

## 10. Decisions

| Question | Decision | Rationale |
|----------|----------|-----------|
| Separate executor? | No — NbE eval with capability modes | One evaluator, capability mode selects effects |
| Refinement types? | No — decidable propositions instead | Lean pattern: propositions as types with native decision procedures |
| Row types / open world? | No — closed records + `Option` for recommended + `get` for undeclared | Lean pattern: structures are closed, dynamic access is explicit |
| Subtyping? | No | Avoids fundamental change to bidirectional type checking |
| Universe polymorphism? | No — fixed 3-level stratification | Matches epistemic levels, avoids universe inference complexity |
| PropAccess semantics? | Look up property type from class Sigma type; reject undeclared without `get` | Sound, forces programs to declare their dependencies |
| Trace representation in types? | Traces are not types; `eval` produces `(Val, Option<Trace>)`, readback strips traces | Traces are computational bookkeeping, not logical content |
| Validation vs type checking? | Complementary: validation checks data surface (open world), type checking checks program surface (closed world) | Each is sound for its domain; neither subsumes the other |
| Lean 4 borrowings? | Design patterns (Decidable, native_decide, structure inheritance), not implementation | Right scale for our system; avoids importing Lean's complexity |
| ComponentTrace storage? | Dual-write: side cache during execution, trace layer after completion | Side cache for crash recovery + memoization; trace layer for queryability + proof verification |
| ProgramTrace storage? | Trace layer, committed with ComponentTraces after execution | All trace artifacts in one immutable layer — the proof certificate |
| Trace layers? | Append-only extension of the layer chain | Each execution adds a trace layer; layers are immutable; proofs are content-addressed |
| RunProgram auto-commit? | Yes — auto-commit ProgramTrace, return trace IRI in response | Consistent with Load auto-commit; ensures traces always persisted for completed executions |
| Trace pruning? | Keep IO ComponentTraces only; discard deterministic trace tree | Proofs-as-programs: deterministic structure is reconstructible from program + input + IO cache |
| Crash recovery? | No explicit checkpointing — ComponentTrace cache in RocksDB is the checkpoint | Re-evaluation hits cache for completed IO steps; resumes from first untraced step |
| Trace GC? | Start with keep-all; add TTL or layer-scoped cleanup later | Simplest correct policy; GC is an optimization, not a correctness concern |
