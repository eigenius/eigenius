# D6b: Reasoning Trace Schema

*Design document for the Eigenius project — April 2026*

**Status:** Implemented (Phase 4)
**Required before:** Phase 4 implementation
**Resolves:** Ontology classes and properties for reasoning traces, provenance link structure, universe level assignment, epistemic status computation

---

## 1. Overview

Reasoning traces are typed Eigon resources that mirror the structure of the program expression tree, with values filled in and metrics attached. Each evaluation step returns both its result and its trace. The trace tree builds up naturally as evaluation recurses — no post-hoc path computation needed.

Traces serve three purposes:

1. **Durability** — traces are the memoization cache for program execution (see D6). A ComponentTrace with matching inputs means the result is known.
2. **Provenance** — the trace tree links outputs to inputs through the expression structure. "Where did this result come from?" is a traversal of the trace tree.
3. **Epistemic status** — observed, derived, and verified knowledge are distinguished by the type of traces in the provenance chain.

---

## 2. Trace Types Mirror Expression Types

The trace type system mirrors the expression language (D3). Each expression form has a corresponding trace form. Pure leaf expressions (Var, Literal) produce no trace — they involve no computation.

| Expression | Trace type | What it records |
|-----------|-----------|-----------------|
| `Let` | `LetTrace` | Name, value trace, body trace |
| `Apply` (io) | `ComponentTrace` | Component, input hash, output, LLM metrics |
| `Apply` (pure) | `PureTrace` | Component, output (no metrics) |
| `Map` | `MapTrace` | Per-element traces |
| `Reduce` | `ReduceTrace` | Per-step accumulator traces |
| `Case` | `CaseTrace` | Scrutinee trace, which branch taken, branch body trace |
| `Construct` | `ConstructTrace` | Per-field traces |
| `Project` | `ProjectTrace` | Source trace, property accessed |
| (structural, ≥2 effectful children) | `SeqTrace` | Child traces in evaluation order |
| `Lambda` | (no trace) | Creates closure, no computation |
| `Var` | (no trace) | Variable lookup, no computation |
| `Literal` | (no trace) | Constant, no computation |

`SeqTrace` is the generic structural join: any expression form without a
dedicated trace type (`Pair`, `Id`, constructor arguments, the two curried
applications of one `Reduce` step, …) contributes its children's traces
directly — one child passes through unwrapped, two or more are grouped in a
`SeqTrace`. Purely structural subtrees still produce no trace. This closes
the pre-consolidation gap where only 8 expression forms were traced and
effects nested anywhere else were dropped from the tree (finding F-5,
`docs/notes/nbe-reorganization-analysis.md` §3.2); the evaluator is now a
single function generic over the tracing strategy, so the traced and
untraced paths cannot drift apart.

All node classes are `subclass_of` the abstract **`reflection:Trace`** base
class, and every trace-child property (`value_trace`, `body_trace`,
`source_trace`, `scrutinee_trace`, `branch_trace`, `element_traces`,
`step_traces`, `child_traces`, `FieldTrace.trace`, and
`ProgramTrace.trace_tree`) is `class_types`-constrained to it — Rule 8
matches transitively via `subclass_of`. A positional slot whose evaluation
was pure (a Map element, Reduce step, or Construct field) serializes as
**`reflection:EmptyTrace`**, a typed placeholder replacing the untyped
empty embedded resource used before.

`ConstructTrace.field_traces` is a `resource_array` of
**`reflection:FieldTrace`** entries, each a typed resource carrying the
constructed `property` IRI and the field's `trace` node. (It was formerly
an untyped embedded resource abused as an IRI-keyed map — a shape recursive
validation rightly rejects, since the keys are other classes' property
IRIs.)

**Enforcement is recursive** (validation Rule 23): `validate_resource`
descends into every embedded resource that declares an `is_a`, applying the
full rule set at every depth — so a malformed node deep inside a
`trace_tree` is caught, not just the root. Embedded resources *without* an
`is_a` are skipped: the resource type doubles as a structural carrier for
opaque internal encodings (program-expression and comorphism-argument
mirrors hold sub-expressions under raw property IRIs), which are not domain
data. `is_a` presence is the discriminator; every trace node sets it, so
the trace tree is fully covered.

### 2.1 How evaluation produces traces

```rust
fn evaluate(expr, env) -> (Value, Option<Trace>) {
    match expr {
        Let(name, typ, value, body) => {
            let (val, val_trace) = evaluate(value, env);
            let env2 = env.extend(name, val);
            let (result, body_trace) = evaluate(body, env2);
            (result, Some(LetTrace { name, value_trace: val_trace, body_trace }))
        }
        Apply(component, arg) if component.is_io() => {
            // Check trace cache first
            let cache_key = hash(component, canonicalize(arg));
            if let Some(cached) = trace_store.get(cache_key) {
                return (cached.output, Some(ComponentTrace { cached: true, ...cached }));
            }
            // Dispatch to orchestrator
            let (result, metrics) = orchestrator.execute(component, arg);
            let trace = ComponentTrace { component, input_hash, output: result, metrics, cached: false };
            trace_store.put(cache_key, trace);
            (result, Some(trace))
        }
        Apply(component, arg) if component.is_pure() => {
            let result = component.execute_locally(arg);
            (result, Some(PureTrace { component, output: result }))
        }
        Map(func, collection) => {
            let (results, traces): (Vec<_>, Vec<_>) = collection
                .iter()
                .map(|elem| evaluate(Apply(func, elem), env))
                .unzip();
            (results, Some(MapTrace { element_traces: traces }))
        }
        Case(scrutinee, branches) => {
            let (scrut_val, scrut_trace) = evaluate(scrutinee, env);
            let (branch_name, branch_body) = match_branch(scrut_val, branches);
            let (result, branch_trace) = evaluate(branch_body, env);
            (result, Some(CaseTrace { scrutinee_trace: scrut_trace, branch_taken: branch_name, branch_trace }))
        }
        Construct(class, fields) => {
            let (values, traces): (Vec<_>, Vec<_>) = fields
                .iter()
                .map(|(name, expr)| {
                    let (val, trace) = evaluate(expr, env);
                    ((name, val), (name, trace))
                })
                .unzip();
            (Resource::from(values), Some(ConstructTrace { field_traces: traces }))
        }
        Var(name) => (env.lookup(name), None)
        Literal(val) => (val, None)
    }
}
```

The trace tree builds up as evaluation recurses. No separate tracing pass needed.

---

## 3. Trace Classes (Eigon-JSON)

### 3.1 LetTrace

```json
{
  "urn:eigenius:core:is_a": ["urn:eigenius:reflection:LetTrace"],
  "urn:eigenius:reflection:name": "parties",
  "urn:eigenius:reflection:value_trace": { ... nested trace ... },
  "urn:eigenius:reflection:body_trace": { ... nested trace ... }
}
```

### 3.2 ComponentTrace

The atomic unit of traced IO computation. This is the cache identity for memoization.

```json
{
  "@id": "urn:eigenius:trace:<content-hash>",
  "urn:eigenius:core:is_a": ["urn:eigenius:reflection:ComponentTrace"],
  "urn:eigenius:reflection:component": "urn:eigenius:program:components:CompleteText",
  "urn:eigenius:reflection:input_hash": "<cbor-deterministic-hash>",
  "urn:eigenius:reflection:argument_hash": "<cbor-deterministic-hash>",
  "urn:eigenius:reflection:output": { ... embedded output resource ... },
  "urn:eigenius:reflection:provider": "anthropic",
  "urn:eigenius:reflection:model": "claude-sonnet-4-6",
  "urn:eigenius:reflection:prompt_tokens": 450,
  "urn:eigenius:reflection:completion_tokens": 120,
  "urn:eigenius:reflection:latency_ms": 1200,
  "urn:eigenius:reflection:timestamp": "2026-04-12T14:30:01Z",
  "urn:eigenius:reflection:deterministic": false,
  "urn:eigenius:reflection:cached": false
}
```

ComponentTraces get `@id` (content-addressed) because they are the memoization cache entries. Other trace types are embedded (no `@id`) — they exist only as part of the trace tree.

### 3.3 PureTrace

```json
{
  "urn:eigenius:core:is_a": ["urn:eigenius:reflection:PureTrace"],
  "urn:eigenius:reflection:component": "urn:eigenius:program:components:Extract",
  "urn:eigenius:reflection:output": { ... embedded output resource ... }
}
```

### 3.4 MapTrace

```json
{
  "urn:eigenius:core:is_a": ["urn:eigenius:reflection:MapTrace"],
  "urn:eigenius:reflection:element_traces": [
    { ... trace for element 0 ... },
    { ... trace for element 1 ... },
    { ... trace for element 2 ... }
  ]
}
```

### 3.5 ReduceTrace

```json
{
  "urn:eigenius:core:is_a": ["urn:eigenius:reflection:ReduceTrace"],
  "urn:eigenius:reflection:step_traces": [
    { ... trace for accumulator step 0 ... },
    { ... trace for accumulator step 1 ... }
  ]
}
```

### 3.6 CaseTrace

```json
{
  "urn:eigenius:core:is_a": ["urn:eigenius:reflection:CaseTrace"],
  "urn:eigenius:reflection:scrutinee_trace": { ... trace ... },
  "urn:eigenius:reflection:branch_taken": "ok",
  "urn:eigenius:reflection:branch_trace": { ... trace for the taken branch ... }
}
```

### 3.7 ConstructTrace

```json
{
  "urn:eigenius:core:is_a": ["urn:eigenius:reflection:ConstructTrace"],
  "urn:eigenius:reflection:field_traces": {
    "urn:eigenius:example:employee_name": { ... trace ... },
    "urn:eigenius:example:complaint_facts": { ... trace ... },
    "urn:eigenius:example:response_letter": { ... trace ... }
  }
}
```

### 3.8 ProjectTrace

```json
{
  "urn:eigenius:core:is_a": ["urn:eigenius:reflection:ProjectTrace"],
  "urn:eigenius:reflection:source_trace": { ... trace of the resource expression ... },
  "urn:eigenius:reflection:property": "urn:eigenius:example:letter"
}
```

---

## 4. Program-Level Traces

### 4.1 ProgramTrace

A ProgramTrace wraps the entire trace tree for a program execution:

```json
{
  "@id": "urn:eigenius:trace:exec-<uuid>",
  "urn:eigenius:core:is_a": ["urn:eigenius:reflection:ProgramTrace"],
  "urn:eigenius:reflection:program": "urn:eigenius:example:workers-comp",
  "urn:eigenius:reflection:input": "urn:eigenius:example:complaint-001",
  "urn:eigenius:reflection:output": "urn:eigenius:example:response-001",
  "urn:eigenius:reflection:trace_tree": {
    "urn:eigenius:core:is_a": ["urn:eigenius:reflection:LetTrace"],
    "urn:eigenius:reflection:name": "parties",
    "urn:eigenius:reflection:value_trace": {
      "urn:eigenius:core:is_a": ["urn:eigenius:reflection:ComponentTrace"],
      "urn:eigenius:reflection:component": "urn:eigenius:program:components:CompleteJson",
      "urn:eigenius:reflection:cached": false,
      "urn:eigenius:reflection:prompt_tokens": 450,
      "...": "..."
    },
    "urn:eigenius:reflection:body_trace": {
      "urn:eigenius:core:is_a": ["urn:eigenius:reflection:LetTrace"],
      "urn:eigenius:reflection:name": "facts",
      "urn:eigenius:reflection:value_trace": { "...": "..." },
      "urn:eigenius:reflection:body_trace": { "...": "..." }
    }
  },
  "urn:eigenius:reflection:total_tokens": 4020,
  "urn:eigenius:reflection:total_latency_ms": 7800,
  "urn:eigenius:reflection:cached_steps": 0,
  "urn:eigenius:reflection:executed_steps": 3,
  "urn:eigenius:reflection:started_at": "2026-04-12T14:30:00Z",
  "urn:eigenius:reflection:completed_at": "2026-04-12T14:30:08Z",
  "urn:eigenius:reflection:epistemic_status": "derived",
  "urn:eigenius:reflection:universe_level": 1
}
```

The `trace_tree` is the root of the tree-structured trace, mirroring the program's expression tree.

### 4.2 DeclarationTrace

Records that a resource was asserted by a human as an axiom, definition, or design decision. This is the epistemic grounding for ontology definitions, program specifications, prompt templates — anything created by human intent rather than observed from reality or computed by a program.

```json
{
  "@id": "urn:eigenius:trace:decl-<hash>",
  "urn:eigenius:core:is_a": ["urn:eigenius:reflection:DeclarationTrace"],
  "urn:eigenius:reflection:resource": "urn:eigenius:core:Class",
  "urn:eigenius:reflection:declared_by": "Eigenius core team",
  "urn:eigenius:reflection:rationale": "Foundational axiom of the self-describing type system",
  "urn:eigenius:reflection:timestamp": "2026-04-11T00:00:00Z",
  "urn:eigenius:reflection:epistemic_status": "declared",
  "urn:eigenius:reflection:universe_level": 0
}
```

The core ontology, domain ontologies, program definitions, and prompt templates are all `declared`. A declaration is an assertion — "I define this to be so." It carries no claim about external reality and no computational derivation.

### 4.3 ObservationTrace

Records that a resource was ingested from external reality with provenance.

```json
{
  "@id": "urn:eigenius:trace:obs-<hash>",
  "urn:eigenius:core:is_a": ["urn:eigenius:reflection:ObservationTrace"],
  "urn:eigenius:reflection:resource": "urn:eigenius:example:complaint-001",
  "urn:eigenius:reflection:source": "manual upload",
  "urn:eigenius:core:source_irl": "https://example.com/documents/complaint-001.pdf",
  "urn:eigenius:reflection:observed_at": "2026-04-12T14:00:00Z",
  "urn:eigenius:reflection:timestamp": "2026-04-12T14:00:00Z",
  "urn:eigenius:reflection:epistemic_status": "observed",
  "urn:eigenius:reflection:universe_level": 0
}
```

An observation is a recorded fact from external reality. The system vouches for its provenance ("this is what was recorded, and here is where it came from"), not for its truth.

### 4.4 VerificationTrace

Records formal proof attachment — promotes a derived result to verified.

```json
{
  "@id": "urn:eigenius:trace:verify-<hash>",
  "urn:eigenius:core:is_a": ["urn:eigenius:reflection:VerificationTrace"],
  "urn:eigenius:reflection:resource": "urn:eigenius:example:theorem-42",
  "urn:eigenius:reflection:proof_system": "lean4",
  "urn:eigenius:reflection:proof_term": "urn:eigenius:blob:<proof-hash>",
  "urn:eigenius:reflection:derivation_trace": "urn:eigenius:trace:exec-<uuid>",
  "urn:eigenius:reflection:timestamp": "2026-04-12T16:00:00Z",
  "urn:eigenius:reflection:epistemic_status": "verified",
  "urn:eigenius:reflection:universe_level": 1
}
```

---

## 5. Provenance

### 5.1 Provenance from the trace tree

The trace tree *is* the provenance chain. To find what inputs produced an output, walk the tree from root to leaves:

```
ProgramTrace (workers-comp execution) [derived]
  └── LetTrace (parties)
        ├── value: ComponentTrace (CompleteJson → Anthropic API)
        │     ├── input from: ObservationTrace (complaint letter) [observed]
        │     └── prompt from: DeclarationTrace (extract_prompt) [declared]
        └── body: LetTrace (facts)
              ├── value: ComponentTrace (CompleteText → Anthropic API)
              │     ├── input from: ObservationTrace (complaint letter) [observed]
              │     └── prompt from: DeclarationTrace (facts_prompt) [declared]
              └── body: LetTrace (response)
                    ├── value: ComponentTrace (CompleteText → Anthropic API)
                    │     └── inputs from: parties [derived] + facts [derived]
                    └── body: ConstructTrace (output) [derived]
```

All four epistemic categories appear in a single program execution:
- The program itself and its prompts are **declared** (human-authored)
- The input document is **observed** (recorded from external reality)
- The computed results are **derived** (produced by the program)
- If a result is later formally proved, it becomes **verified**

### 5.2 Querying provenance

"Which ComponentTraces contributed to this program execution?"

```
USING "urn:eigenius:reflection:ProgramTrace"
MATCH ProgramTrace(?t) {
    program: ?prog,
    output: ?output
}
WHERE ?output = "urn:eigenius:example:response-001"
RETURN [] {
    program: ?prog,
    trace: ?t
}
```

For deeper traversal (finding all ComponentTraces within a trace tree), the trace tree can be walked programmatically or via recursive EigenQL on the embedded trace structure.

---

## 6. Epistemic Status

### 6.1 Four epistemic categories

| Status | Meaning | Trace type | Example |
|--------|---------|-----------|---------|
| **declared** | Asserted by a human as axiom or definition | DeclarationTrace | Core ontology, domain ontologies, program definitions, prompt templates |
| **observed** | Recorded from external reality with provenance | ObservationTrace | Uploaded documents, sensor readings, API responses, experimental data |
| **derived** | Computed by a typed program from other resources | ProgramTrace | LLM outputs, extracted entities, generated summaries |
| **verified** | Carries a machine-checked formal proof | VerificationTrace | Proved theorems, certified computations |

Each category represents a different epistemic act:
- **Declared** = "I assert this" — anchored in human thought and intent
- **Observed** = "This was recorded" — anchored in external reality
- **Derived** = "This follows from those inputs through this process" — anchored in computation with full audit trail
- **Verified** = "This is mathematically certain" — anchored in machine-checked proof

### 6.2 Epistemic base classes

Resources declare their epistemic status via `is_a` using base classes. This uses the existing type system — multiple class membership, `requires`/`recommends`, validation rules:

```json
{
  "@id": "urn:eigenius:core:Class",
  "urn:eigenius:core:is_a": [
    "urn:eigenius:core:Class",
    "urn:eigenius:reflection:DeclaredResource"
  ],
  "urn:eigenius:reflection:declared_by": "Eigenius core team",
  "urn:eigenius:reflection:rationale": "Foundational axiom of the self-describing type system",
  "...": "..."
}
```

```json
{
  "@id": "urn:eigenius:example:complaint-001",
  "urn:eigenius:core:is_a": [
    "urn:eigenius:example:Document",
    "urn:eigenius:reflection:ObservedResource"
  ],
  "urn:eigenius:reflection:source": "manual upload",
  "urn:eigenius:core:source_irl": "https://example.com/complaint-001.pdf",
  "...": "..."
}
```

```json
{
  "@id": "urn:eigenius:example:response-001",
  "urn:eigenius:core:is_a": [
    "urn:eigenius:example:Output",
    "urn:eigenius:reflection:DerivedResource"
  ],
  "urn:eigenius:reflection:derivation": "urn:eigenius:trace:exec-abc123",
  "...": "..."
}
```

The base classes enforce provenance through `requires`:

| Base class | Required properties |
|-----------|-------------------|
| `DeclaredResource` | `declared_by` |
| `ObservedResource` | `source` |
| `DerivedResource` | `derivation` (link to ProgramTrace) |
| `VerifiedResource` | `derivation`, `verification` (link to VerificationTrace) |

A resource without any epistemic base class is **untraced** — it has no provenance record. Whether untraced resources are allowed is a policy decision (open in development, strict in production).

### 6.3 Transitions

Transitions are monotonic upward: declared → observed → derived → verified. Each level adds epistemic strength. A resource can accumulate multiple statuses — a declared ontology class can later be verified by a formal proof.

The resource's *effective* epistemic status is the strongest in its `is_a`:
- Has `VerifiedResource` → verified
- Has `DerivedResource` → derived
- Has `ObservedResource` → observed
- Has `DeclaredResource` → declared
- Has none → untraced

---

## 7. Universe Stratification

- **Level 0** — traces about domain resources
- **Level 1** — traces about level-0 traces (meta-reasoning)
- **Level N** — traces about level N-1 traces

**Rule:** A trace at level N can only reference resources at level N-1 or below.

**Enforcement:** The kernel validates universe levels when storing traces.

---

## 8. Trace-Based Memoization (connection to D6)

Only **ComponentTraces** participate in memoization. They are the only traces with `@id` (content-addressed) because they represent actual computation with external effects.

| Scenario | Behavior |
|----------|----------|
| First execution | No ComponentTrace exists → dispatch → store trace |
| Re-execution (same inputs) | ComponentTrace found → return cached output |
| Changed prompt | ComponentTrace key misses → dispatch → store new trace |
| Crash recovery | Partial trace tree → resume from last ComponentTrace |

The trace tree structure handles crash recovery naturally: the kernel re-evaluates the program expression tree. Each `Apply` to an io component checks the trace cache. Completed steps return instantly. Incomplete steps dispatch to the orchestrator.

---

## 9. Decisions Log

| Question | Decision | Rationale |
|----------|----------|-----------|
| Trace structure | Tree mirroring expression tree | Composes naturally during recursive evaluation; no post-hoc path computation |
| Trace composition | Each `evaluate()` returns `(Value, Option<Trace>)` | Trace builds up as evaluation recurses |
| Which traces get `@id` | Only ComponentTrace (memoization cache) | Other traces are embedded in the tree; no independent identity needed |
| Leaf traces | Var and Literal produce no trace | No computation → nothing to record |
| Provenance | Walk the trace tree | The tree IS the provenance chain |
| Epistemic categories | Four levels: declared → observed → derived → verified | Declaration is a distinct epistemic act from observation; core ontology is declared, not observed |
| Epistemic enforcement | Base classes via `is_a` (DeclaredResource, ObservedResource, etc.) | Uses existing type system; `requires` enforces provenance properties |
| Universe stratification | Level N references level N-1 only | Prevents self-referential paradox |
| Memoization scope | ComponentTrace only | Pure computations are fast; only IO needs caching |
