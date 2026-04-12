# D3: DAG Specification and Component Model

*Design document for the Eigenius project — April 2026*

**Status:** Draft
**Required before:** Phase 2 implementation
**Resolves:** DAG specification format, component interface, type checking foundation, built-in vs extension components, execution model

---

## 1. Overview

Processing DAGs in Eigenius are functional programs whose type system is grounded in dependent type theory (Mini-TT) with Eigon ontology types as ground types. A DAG that passes validation carries formal guarantees: it terminates, it is type-safe, and it produces output of the declared type. Validation is not a heuristic check — it is a proof of these properties, performed statically before the first step executes.

This document specifies:
- The type-theoretic foundation (Mini-TT with Eigon ground types)
- How DAGs are authored and represented (as Eigon-JSON resources)
- The component model (built-in components and WASM extensions)
- The DAG ontology (classes for Dag, Step, Binding, Component, etc.)
- The execution model

### 1.1 Design principles

**Validation before execution.** The DAG type checker validates the full pipeline before any step executes. Execution is mechanical — it cannot violate the established type guarantees. This is the core differentiator from conventional workflow engines.

**DAGs are resources.** DAG specifications are Eigon-JSON resources like everything else — they use the same format, the same IRIs, the same layer system. They are queryable via EigenQL, validatable by the kernel, and storable in any backend.

**Two-tier component model.** Built-in components are part of the platform (compiled into the kernel or orchestration layer). Extension components are WASM modules with WIT interfaces, sandboxed and independently installable.

---

## 2. Type-theoretic foundation

### 2.1 Mini-TT

The DAG type system is based on Mini-TT (Coquand et al.) — a minimal dependent type theory with:

- **Dependent function types (Pi):** `Π (x : A). B(x)` — a step from input A to output B where B may depend on the value of x
- **Dependent pair types (Sigma):** `Σ (x : A). B(x)` — a pair where the second component's type depends on the first
- **Labeled sum types:** `Sum(ok : A | err : E)` — tagged unions with exhaustiveness checking
- **Universe of types:** `Type` — the type of types, for polymorphism over classes

The evaluator uses **Normalization by Evaluation (NbE)**, which serves three purposes simultaneously:
1. **Type checking** — verifying that terms have their declared types
2. **Type equality** — comparing types by reducing to normal forms
3. **Partial evaluation** — evaluating a DAG with some inputs unknown produces a well-typed residual

### 2.2 Core value representation

```rust
/// Semantic values — the result of evaluation.
pub enum Val {
    /// A dependent function (closure)
    Lambda(String, Box<Closure>),
    /// A dependent function type: Π (x : A). B(x)
    Pi(Box<Val>, Box<Closure>),
    /// A dependent pair type: Σ (x : A). B(x)
    Sigma(Box<Val>, Box<Closure>),
    /// A dependent pair value
    Pair(Box<Val>, Box<Val>),
    /// A labeled sum type: Sum(label₁ : T₁ | label₂ : T₂ | ...)
    Sum(BTreeMap<String, Val>),
    /// A labeled sum constructor: label(value)
    Constructor(String, Box<Val>),
    /// The universe of types
    Universe,
    /// An Eigon ground type — resolved from the layer chain
    EigonClass(Iri),
    /// An Eigon primitive type
    EigonPrimitive(PrimitiveType),
    /// A neutral term — a computation blocked on an unknown
    Neutral(Neutral),
    /// A concrete Eigon resource value
    ResourceVal(Resource),
    /// Unit
    Unit,
}

/// Neutral terms — computations that cannot reduce further.
pub enum Neutral {
    /// A free variable (unknown input)
    Var(String),
    /// Application of a neutral function to an argument
    App(Box<Neutral>, Box<Val>),
    /// Projection from a neutral pair
    Fst(Box<Neutral>),
    Snd(Box<Neutral>),
    /// Case split on a neutral sum
    Case(Box<Neutral>, BTreeMap<String, Closure>),
    /// Property access on a neutral resource
    PropertyAccess(Box<Neutral>, Iri),
}

/// A closure: a term together with its environment.
pub struct Closure {
    pub param: String,
    pub body: Term,
    pub env: Environment,
}

/// Primitive Eigon types.
pub enum PrimitiveType {
    String,
    Integer,
    Float,
    Boolean,
    Json,
}
```

### 2.3 NbE operations

The evaluator implements four core operations:

| Operation | Signature | Purpose |
|-----------|-----------|---------|
| `eval` | `Term × Env → Val` | Evaluate a term to a semantic value |
| `readback` | `Val → Term` | Convert a value back to a normal-form term |
| `check` | `Term × Val → ()` | Check that a term has a given type (checking mode) |
| `infer` | `Term → Val` | Synthesize a type for a term (inference mode) |
| `eq_nf` | `Val × Val → bool` | Check type equality by normalizing both sides |

**Type checking is bidirectional:** DAG boundaries, component signatures, and step bindings carry explicit type annotations (checked mode). Internal types are inferred (inference mode). This is the same discipline used by Lean 4, Agda, and Idris.

### 2.4 Ground type resolution

Eigon ontology types are **ground types** in the type theory. The evaluator resolves them from the layer chain:

1. A class IRI (e.g., `urn:eigenius:example:Dog`) resolves to a dependent record type — a nested Sigma over the class's required properties
2. Properties in the record are ordered canonically by IRI (BTreeMap iteration)
3. Subclass relationships are modeled as coercions — the type checker inserts implicit projections when a subclass value appears where a superclass is expected
4. Required properties are plain fields; recommended properties are wrapped in `Maybe` (a labeled sum: `Sum(some : T | none : Unit)`)

Example: the class `Dog` with required properties `name: String` and `breed: String`, inheriting from `Animal`:

```
Dog ≡ Σ (breed : String). Σ (name : String). Unit
Animal ≡ Σ (name : String). Unit
Coercion(Dog → Animal) = λ d. (d.name, ())
```

### 2.5 What validation proves

A DAG that passes type checking carries these formal guarantees:

1. **Type safety** — every step receives inputs of the types it declared
2. **Termination** — the DAG's control flow terminates on every well-typed input (wall-clock time bounded separately by execution constraints)
3. **Exhaustive error handling** — all `Result` cases are handled; no unmatched branches
4. **Output type correctness** — the final output is of the declared type
5. **Partial evaluability** — the DAG can be partially evaluated with respect to any subset of its inputs, producing a well-typed residual

---

## 3. DAG specification format

DAGs are authored in Eigon-JSON. There is no separate surface syntax. The DAG ontology defines the classes and properties used to describe pipelines.

### 3.1 A complete example

```json
[
  {
    "@id": "urn:eigenius:example:summarize-pipeline",
    "urn:eigenius:core:is_a": ["urn:eigenius:dag:Dag"],
    "urn:eigenius:core:description": "Summarize a document using LLM extraction and combination",
    "urn:eigenius:dag:input": "urn:eigenius:example:DocumentInput",
    "urn:eigenius:dag:output": "urn:eigenius:example:Summary",
    "urn:eigenius:dag:steps": [
      {
        "urn:eigenius:core:is_a": ["urn:eigenius:dag:Step"],
        "urn:eigenius:dag:step_label": "extract",
        "urn:eigenius:dag:component": "urn:eigenius:dag:components:CompleteJson",
        "urn:eigenius:dag:argument": "urn:eigenius:example:extract-prompt",
        "urn:eigenius:dag:bindings": [
          {
            "urn:eigenius:core:is_a": ["urn:eigenius:dag:Binding"],
            "urn:eigenius:dag:step_label": "input",
            "urn:eigenius:dag:component_property": "urn:eigenius:common:string",
            "urn:eigenius:dag:context_property": "urn:eigenius:example:document_text"
          }
        ]
      },
      {
        "urn:eigenius:core:is_a": ["urn:eigenius:dag:Step"],
        "urn:eigenius:dag:step_label": "summarize",
        "urn:eigenius:dag:component": "urn:eigenius:dag:components:CompleteText",
        "urn:eigenius:dag:argument": "urn:eigenius:example:summarize-prompt",
        "urn:eigenius:dag:bindings": [
          {
            "urn:eigenius:core:is_a": ["urn:eigenius:dag:Binding"],
            "urn:eigenius:dag:step_label": "extract"
          }
        ]
      }
    ]
  }
]
```

### 3.2 Rationale

Eigon-JSON as the DAG format means:
- No new parser — reuses the existing Eigon-JSON parser and validator
- DAGs are queryable: `MATCH Dag(?d) { step_label: ?s }` finds all DAGs with a specific step
- DAGs live in layers alongside the ontologies that define their input/output types
- The entire pipeline is a typed, validated, content-addressed resource

---

## 4. DAG ontology

The DAG ontology is defined as a layer loaded on top of the core ontology (under `urn:eigenius:dag:`).

### 4.1 Classes

| Class | IRI | Description |
|-------|-----|-------------|
| Dag | `urn:eigenius:dag:Dag` | A complete processing pipeline |
| Step | `urn:eigenius:dag:Step` | An atomic computation step referencing a Component |
| Binding | `urn:eigenius:dag:Binding` | Routes data between steps |
| Component | `urn:eigenius:dag:Component` | A registered computation unit |
| Sequence | `urn:eigenius:dag:Sequence` | Ordered composition of steps |
| Select | `urn:eigenius:dag:Select` | Conditional branching with guard queries |
| MapStep | `urn:eigenius:dag:MapStep` | Apply a sequence to each element of a collection |
| ReduceStep | `urn:eigenius:dag:ReduceStep` | Fold over a collection to produce a single result |
| RetryPolicy | `urn:eigenius:dag:RetryPolicy` | Retry configuration for fallible steps |
| RequestParameters | `urn:eigenius:dag:RequestParameters` | Configuration for LLM/external service calls |

### 4.2 Dag properties

| Property | IRI | Data type | Description |
|----------|-----|-----------|-------------|
| input | `urn:eigenius:dag:input` | resource | Input class for the pipeline |
| output | `urn:eigenius:dag:output` | resource | Output class for the pipeline |
| steps | `urn:eigenius:dag:steps` | resource_array | Ordered list of steps (embedded resources) |

### 4.3 Step properties

| Property | IRI | Data type | Description |
|----------|-----|-----------|-------------|
| step_label | `urn:eigenius:dag:step_label` | string | Unique label within the DAG, used for binding references |
| component | `urn:eigenius:dag:component` | resource | The Component to execute |
| argument | `urn:eigenius:dag:argument` | resource | Static configuration/prompt for the component |
| bindings | `urn:eigenius:dag:bindings` | resource_array | Data routing from context into this step |
| retry | `urn:eigenius:dag:retry` | resource | Optional retry policy |

### 4.4 Binding properties

| Property | IRI | Data type | Description |
|----------|-----|-----------|-------------|
| step_label | `urn:eigenius:dag:step_label` | string | Source step to bind from (or "input" for DAG input) |
| component_property | `urn:eigenius:dag:component_property` | resource | Which property of the source step's output to bind |
| context_property | `urn:eigenius:dag:context_property` | resource | Which property of the DAG's input context to bind |

### 4.5 Component properties

| Property | IRI | Data type | Description |
|----------|-----|-----------|-------------|
| input_class | `urn:eigenius:dag:component:input_class` | resource | Expected input class |
| output_class | `urn:eigenius:dag:component:output_class` | resource | Produced output class |
| implementation | `urn:eigenius:dag:component:implementation` | string | "builtin" or "wasm" |
| wasm_binary | `urn:eigenius:dag:component:wasm_binary` | string (format: iri) | IRI of the WASM binary (for wasm components) |
| capability_level | `urn:eigenius:dag:component:capability_level` | resource | What the component can access |
| deterministic | `urn:eigenius:dag:component:deterministic` | boolean | Whether output is reproducible |
| fallible | `urn:eigenius:dag:component:fallible` | boolean | Whether the component may fail (output is Result type) |
| error_class | `urn:eigenius:dag:component:error_class` | resource | Error class if fallible |

---

## 5. Component model

### 5.1 Two-tier architecture

Components come in two flavors, unified by the same ontology type signature and the same Mini-TT types:

**Built-in components** — native Rust or TypeScript, compiled into the platform. These are the foundational building blocks: LLM completion, resource manipulation, core I/O. They run in-process with full kernel access. Registered in the Foundation Layer.

**Extension components** — WASM modules compiled to the WASI Component Model. These are domain-specific, third-party, or untrusted. They run sandboxed via wasmtime with declared capabilities, installable independently.

The type checker treats both identically — a Component's type is `Π (input : InputClass). ResultType` regardless of implementation. The executor dispatches differently based on the `implementation` property.

### 5.2 Built-in components (Phase 2)

| Component | IRI | Type signature | Description |
|-----------|-----|---------------|-------------|
| CompleteText | `urn:eigenius:dag:components:CompleteText` | `Arguments → Result<String, Error>` | LLM text completion |
| CompleteJson | `urn:eigenius:dag:components:CompleteJson` | `Arguments → Result<OutputClass, Error>` | LLM structured output |
| Combine | `urn:eigenius:dag:components:Combine` | `Inputs → OutputClass` | Merge properties into one resource |
| Extract | `urn:eigenius:dag:components:Extract` | `Resource → Resource` | Extract specific properties |
| Transform | `urn:eigenius:dag:components:Transform` | `Resource → Resource` | Apply property mappings |
| HttpRequest | `urn:eigenius:dag:components:HttpRequest` | `Request → Result<Response, Error>` | HTTP request |

Implementation trait:

```rust
pub trait BuiltinComponent: Send + Sync {
    /// The Mini-TT type of this component: Π (input : A). B
    fn type_signature(&self, layer: &Layer) -> Val;

    /// Execute the component
    fn execute(
        &self,
        input: &Resource,
        argument: &Resource,
        context: &ExecutionContext,
    ) -> Result<Resource, ComponentError>;
}
```

### 5.3 Extension components (WASM)

Extension components are WASM modules compiled to the WASI Component Model.

**WIT interface:**

```wit
package eigenius:component@0.1.0;

interface component {
    variant value {
        string-val(string),
        integer-val(s64),
        float-val(float64),
        boolean-val(bool),
        array-val(list<value>),
    }

    record eigon-resource {
        properties: list<tuple<string, value>>,
    }

    execute: func(
        input: eigon-resource,
        argument: eigon-resource,
    ) -> result<eigon-resource, string>;
}

interface resource-reader {
    use component.{eigon-resource};
    resolve: func(iri: string) -> option<eigon-resource>;
    query-by-class: func(class-iri: string) -> list<eigon-resource>;
}
```

**Capability levels as worlds:**

```wit
world pure-component {
    export component;
}

world read-component {
    import resource-reader;
    export component;
}

world io-component {
    import resource-reader;
    import wasi:http/outgoing-handler@0.2.0;
    export component;
}
```

### 5.4 Component registration

Components are resources in a layer:

```json
{
  "@id": "urn:eigenius:dag:components:CompleteText",
  "urn:eigenius:core:is_a": ["urn:eigenius:dag:Component"],
  "urn:eigenius:core:description": "LLM text completion",
  "urn:eigenius:core:short_name": "CompleteText",
  "urn:eigenius:dag:component:input_class": "urn:eigenius:dag:components:completion:Arguments",
  "urn:eigenius:dag:component:output_class": "urn:eigenius:common:classes:String",
  "urn:eigenius:dag:component:implementation": "builtin",
  "urn:eigenius:dag:component:capability_level": "urn:eigenius:dag:capability_levels:io",
  "urn:eigenius:dag:component:deterministic": false,
  "urn:eigenius:dag:component:fallible": true,
  "urn:eigenius:dag:component:error_class": "urn:eigenius:dag:components:completion:Error"
}
```

---

## 6. DAG type checking

### 6.1 Type checking as NbE

DAG validation is bidirectional type checking in the Mini-TT type theory. The process:

1. **Resolve ground types** — class IRIs in the DAG spec are evaluated to record types (nested Sigma) by resolving against the layer chain
2. **Check DAG input type** — the declared input class becomes the initial typing context
3. **Check each step** — for each step in sequence:
   a. Resolve the component's type signature: `Π (input : A). B`
   b. Check that the bindings produce a value of type A (checking mode)
   c. Extend the context with the step's output type B, keyed by step_label
4. **Check DAG output type** — the final step's output type must equal the declared output type (by `eq_nf`)
5. **Check error handling** — if a step produces `Result<A, E>`, downstream must handle both branches
6. **Check Select exhaustiveness** — every Select has a default branch

### 6.2 Partial evaluation

Partial evaluation is a natural consequence of NbE. Given a DAG and a subset of its inputs:

1. Bind the known inputs in the environment
2. Leave unknown inputs as free variables
3. Run the NbE evaluator — it reduces as far as possible
4. The result is a normal form containing neutral terms for the unknown parts
5. This residual is a well-typed DAG awaiting the remaining inputs

This requires no additional machinery — partial evaluation is just `eval` under an open environment.

---

## 7. DAG execution

### 7.1 Execution model

The DAG executor walks a validated DAG step by step:

1. Start with the DAG's input resource
2. For each step in order:
   a. Resolve bindings — select values from the execution context
   b. Construct the component's input resource from bindings + argument
   c. Dispatch to the component (built-in: call Rust function; WASM: instantiate via wasmtime)
   d. Add the step's output to the execution context, keyed by step_label
3. The final step's output is the DAG's output

### 7.2 Combinators

| Combinator | Type | Execution behavior |
|-----------|------|-------------------|
| Step | `A → B` | Execute a single component |
| Sequence | `A → B → C` | Execute steps in order, accumulating context |
| Select | `A → Sum(branches) → B` | Evaluate EigenQL guard queries, execute first matching branch |
| MapStep | `List(A) → List(B)` | Apply a sequence to each element, in parallel |
| ReduceStep | `Acc × List(A) → Acc` | Fold a sequence over a collection with an accumulator |
| RetryPolicy | `(A → Result<B, E>) → (A → Result<B, E>)` | Re-execute up to N times with backoff |

### 7.3 Phase 2 execution architecture

For Phase 2, DAG execution lives in the Rust kernel:

```
CLI                    Kernel
 │                      │
 │ eigenius run dag.json│
 │─────────────────────>│
 │                      ├── Parse DAG from Eigon-JSON
 │                      ├── Resolve ground types from layer chain
 │                      ├── Type-check via Mini-TT NbE
 │                      ├── Execute steps in order
 │                      │   ├── Built-in: call Rust function
 │                      │   └── WASM: instantiate via wasmtime (Phase 5)
 │                      ├── Collect output resource
 │<─────────────────────┤
 │ result               │
```

The Deno orchestration layer takes over DAG execution in Phase 3+ when async execution, LLM adapter management, and parallel fan-out are needed.

---

## 8. Phasing

| Phase | What is implemented |
|-------|-------------------|
| Phase 2 | Mini-TT core (Val, Neutral, NbE evaluator), ground type resolution, DAG ontology, DAG type checker, built-in components (stubs), DAG executor, CLI `validate` and `run` commands, partial evaluation |
| Phase 3 | gRPC service wrapping the DAG executor, Deno orchestration layer for async execution |
| Phase 4 | LLM adapter components (CompleteText, CompleteJson) with real implementations |
| Phase 5 | WASM extension components via wasmtime, WIT interface, capability levels, fuel metering |

---

## 9. Decisions log

| Question | Decision | Rationale |
|----------|----------|-----------|
| DAG format | Eigon-JSON (no ESL surface syntax) | Everything is a Resource; reuses existing parser; queryable via EigenQL |
| Type checking foundation | Mini-TT with NbE — not deferred | The formal guarantees (type safety, termination, partial evaluation) are the core differentiator; Mini-TT is ~10 pages of Haskell, implementable in days |
| Ground type resolution | Classes → nested Sigma types, resolved from layer chain | Natural mapping; properties ordered by IRI for canonical record types |
| Component model | Two-tier: built-in (native) + extension (WASM) | Built-ins for platform fundamentals; WASM for domain extensions |
| WIT interface | Adopt WASI Component Model for extensions | Language-agnostic, sandboxed, ecosystem momentum, wasmtime support |
| Capability levels | Pure / Read / IO worlds | Maps directly to WIT worlds; least-privilege by default |
| Phase 2 execution | Rust kernel, not Deno | Avoids FFI/IPC; kernel already has all data model capabilities |
| Partial evaluation | Consequence of NbE, not separate feature | No additional machinery needed; eval under open environment |
| Component registration | Resources in the layer chain | Uniform with everything else; queryable, validatable, versionable |
| Component type signature | Mini-TT Pi types: `Π (input : A). B` | Dependent types enable output types that depend on input class |
