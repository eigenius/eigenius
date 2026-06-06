# D3: Program Model and Component Interface

*Design document for the Eigenius project — April 2026*

**Status:** Implemented (Phase 2)
**Required before:** Phase 2 implementation
**Resolves:** Program representation, expression language, component interface, execution model

---

## 1. Overview

Programs in Eigenius are typed expressions in a functional language grounded in the Eigon ontology and EigenTT dependent type theory. A program that passes type checking carries formal guarantees: it terminates, is type-safe, and produces output of the declared type.

Programs are represented as Eigon-JSON resources — the same format as everything else in the system. They are queryable, validatable, storable, and content-addressable. An optional surface syntax (ESL) provides a human-friendly authoring experience that compiles to Eigon-JSON.

### 1.1 Design principles

**Programs are expressions, not workflow graphs.** A program is a composition of let-bindings, function applications, case expressions, and collection operations. There is no separate "step" or "binding" indirection — the expression structure directly mirrors the EigenTT term it represents.

**Components are functions.** A component is a typed function registered in the ontology. Calling a component is function application. No special "step" or "dispatch" mechanism — it's just `Apply(component, arguments)`.

**Parallelism from structure.** Independent let-bindings and map elements are automatically parallelizable. The scheduler infers concurrency from data dependencies — no explicit parallelism annotations needed.

---

## 2. Expression Language

### 2.1 Expression forms

| Form | Class IRI | Description |
|------|-----------|-------------|
| Program | `urn:eigenius:program:Program` | Top-level: declares input/output types and body expression |
| Let | `urn:eigenius:program:Let` | Sequential binding: `let x : A = e₁; e₂` |
| Apply | `urn:eigenius:program:Apply` | Function application: `f(arg₁, arg₂, ...)` |
| Var | `urn:eigenius:program:Var` | Variable reference: `x` |
| Lambda | `urn:eigenius:program:Lambda` | Anonymous function: `λx : A. e` |
| Case | `urn:eigenius:program:Case` | Pattern match on Sum type: `case e of c₁ → e₁ \| c₂ → e₂` |
| Pair | `urn:eigenius:program:Pair` | Pair construction: `(a, b)` |
| Construct | `urn:eigenius:program:Construct` | Build a typed resource from computed values |
| Project | `urn:eigenius:program:Project` | Property access on a resource: `e.property` |
| Map | `urn:eigenius:program:Map` | Parallel map: apply function to each element of a collection |
| Reduce | `urn:eigenius:program:Reduce` | Fold: accumulate over a collection |
| Literal | `urn:eigenius:program:Literal` | Concrete value: string, integer, float, boolean |

### 2.2 Mapping to EigenTT

Each expression form maps directly to a EigenTT term:

| Expression | EigenTT term |
|------------|-------------|
| `Program` | `Exp::Lam(input_param, body)` with Pi type |
| `Let` | `Exp::Dec(Decl::Def(name, type, value), body)` |
| `Apply` | `Exp::App(function, argument)` |
| `Var` | `Exp::Var(name)` |
| `Lambda` | `Exp::Lam(param, body)` |
| `Case` | `Exp::Case(branches)` |
| `Pair` | `Exp::Pair(first, second)` |
| `Construct` | Nested `Exp::Pair` matching the class's Sigma type |
| `Project` | `Exp::Fst` / `Exp::Snd` (computed from property position) |
| `Map` | Primitive: `map : (A → B) → List(A) → List(B)` |
| `Reduce` | Primitive: `reduce : (Acc → A → Acc) → Acc → List(A) → Acc` |
| `Literal` | `Exp::EigonResource(value)` |

No translation layer needed — parsing an expression resource directly produces a EigenTT term.

---

## 3. Eigon-JSON Representation

### 3.1 Program

```json
{
  "@id": "urn:eigenius:example:my-program",
  "urn:eigenius:core:is_a": ["urn:eigenius:program:Program"],
  "urn:eigenius:core:description": "Extract and summarize a document",
  "urn:eigenius:program:input_type": "urn:eigenius:example:Document",
  "urn:eigenius:program:output_type": "urn:eigenius:example:Summary",
  "urn:eigenius:program:body": { ... expression ... }
}
```

### 3.2 Let

```json
{
  "urn:eigenius:core:is_a": ["urn:eigenius:program:Let"],
  "urn:eigenius:program:name": "extracted",
  "urn:eigenius:program:type": "urn:eigenius:example:Entities",
  "urn:eigenius:program:value": { ... expression ... },
  "urn:eigenius:program:body": { ... expression ... }
}
```

### 3.3 Apply

```json
{
  "urn:eigenius:core:is_a": ["urn:eigenius:program:Apply"],
  "urn:eigenius:program:function": "urn:eigenius:components:CompleteJson",
  "urn:eigenius:program:argument": { ... expression ... }
}
```

For multi-argument functions, arguments are curried or passed as a pair:

```json
{
  "urn:eigenius:core:is_a": ["urn:eigenius:program:Apply"],
  "urn:eigenius:program:function": "urn:eigenius:components:CompleteJson",
  "urn:eigenius:program:argument": {
    "urn:eigenius:core:is_a": ["urn:eigenius:program:Pair"],
    "urn:eigenius:program:first": { ... prompt ... },
    "urn:eigenius:program:second": { ... parameters ... }
  }
}
```

### 3.4 Var

```json
{
  "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
  "urn:eigenius:program:name": "extracted"
}
```

### 3.5 Lambda

```json
{
  "urn:eigenius:core:is_a": ["urn:eigenius:program:Lambda"],
  "urn:eigenius:program:parameter": "item",
  "urn:eigenius:program:parameter_type": "urn:eigenius:example:Document",
  "urn:eigenius:program:body": { ... expression ... }
}
```

### 3.6 Case

```json
{
  "urn:eigenius:core:is_a": ["urn:eigenius:program:Case"],
  "urn:eigenius:program:scrutinee": { ... expression ... },
  "urn:eigenius:program:branches": [
    {
      "urn:eigenius:program:constructor": "ok",
      "urn:eigenius:program:body": { ... expression ... }
    },
    {
      "urn:eigenius:program:constructor": "err",
      "urn:eigenius:program:body": { ... expression ... }
    }
  ]
}
```

### 3.7 Construct

Build a typed resource from computed values:

```json
{
  "urn:eigenius:core:is_a": ["urn:eigenius:program:Construct"],
  "urn:eigenius:program:class": "urn:eigenius:example:Summary",
  "urn:eigenius:program:fields": {
    "urn:eigenius:example:title": {
      "urn:eigenius:core:is_a": ["urn:eigenius:program:Project"],
      "urn:eigenius:program:expression": {
        "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
        "urn:eigenius:program:name": "input"
      },
      "urn:eigenius:program:property": "urn:eigenius:example:title"
    },
    "urn:eigenius:example:summary_text": {
      "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
      "urn:eigenius:program:name": "summary"
    }
  }
}
```

### 3.8 Project

Property access on a resource expression:

```json
{
  "urn:eigenius:core:is_a": ["urn:eigenius:program:Project"],
  "urn:eigenius:program:expression": {
    "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
    "urn:eigenius:program:name": "input"
  },
  "urn:eigenius:program:property": "urn:eigenius:example:letter"
}
```

### 3.9 Map

Apply a function to each element of a collection:

```json
{
  "urn:eigenius:core:is_a": ["urn:eigenius:program:Map"],
  "urn:eigenius:program:function": {
    "urn:eigenius:core:is_a": ["urn:eigenius:program:Lambda"],
    "urn:eigenius:program:parameter": "page",
    "urn:eigenius:program:parameter_type": "urn:eigenius:example:Page",
    "urn:eigenius:program:body": {
      "urn:eigenius:core:is_a": ["urn:eigenius:program:Apply"],
      "urn:eigenius:program:function": "urn:eigenius:components:CompleteText",
      "urn:eigenius:program:argument": {
        "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
        "urn:eigenius:program:name": "page"
      }
    }
  },
  "urn:eigenius:program:collection": {
    "urn:eigenius:core:is_a": ["urn:eigenius:program:Project"],
    "urn:eigenius:program:expression": {
      "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
      "urn:eigenius:program:name": "input"
    },
    "urn:eigenius:program:property": "urn:eigenius:example:pages"
  }
}
```

Type: `Map : (A → B) → List(A) → List(B)`

Elements are independent and can be executed in parallel by the scheduler.

### 3.10 Reduce

Fold over a collection with an accumulator:

```json
{
  "urn:eigenius:core:is_a": ["urn:eigenius:program:Reduce"],
  "urn:eigenius:program:function": {
    "urn:eigenius:core:is_a": ["urn:eigenius:program:Lambda"],
    "urn:eigenius:program:parameter": "acc_and_item",
    "urn:eigenius:program:body": { ... }
  },
  "urn:eigenius:program:initial": { ... initial accumulator value ... },
  "urn:eigenius:program:collection": { ... expression producing a list ... }
}
```

Type: `Reduce : (Acc → A → Acc) → Acc → List(A) → Acc`

Sequential by nature — each step depends on the previous accumulator.

### 3.11 Literal

```json
{
  "urn:eigenius:core:is_a": ["urn:eigenius:program:Literal"],
  "urn:eigenius:program:value": "hello world"
}
```

Or for resource references:

```json
{
  "urn:eigenius:core:is_a": ["urn:eigenius:program:Literal"],
  "urn:eigenius:program:value": "urn:eigenius:example:my-prompt"
}
```

---

## 4. Type System

### 4.1 Program types

A program has type `Π (input : InputType). OutputType` — a dependent function from input to output. Type checking validates that the body expression produces a value of `OutputType` when given an input of `InputType`.

### 4.2 Component types

Components are typed functions registered in the ontology:

```
CompleteJson : Π (input : Arguments). Result(OutputClass, Error)
CompleteText : Π (input : Arguments). Result(String, Error)
Identity     : Π (A : Class). A → A
```

Fallible components return `Sum(ok : A | err : E)`. The type checker ensures that Case expressions handle both branches (exhaustiveness).

### 4.3 Type checking

Type checking is bidirectional EigenTT (as implemented in `nbe/check.rs`):

1. **Program boundary** — declared input/output types provide the checking context
2. **Let bindings** — value is checked against declared type; body is checked with extended context
3. **Apply** — function type is inferred; argument is checked against domain; result type is the codomain
4. **Case** — scrutinee type is inferred; branches checked against the Sum's constructors; all constructors must be handled
5. **Map/Reduce** — function argument checked against element type; result type computed from function's codomain
6. **Construct** — each field expression checked against the class's property types
7. **Project** — expression type must be a class with the named property; result type is the property's data type

### 4.4 What validation proves

A program that passes type checking carries these guarantees:

1. **Type safety** — every subexpression has a well-formed type
2. **Termination** — the program terminates on all inputs (no infinite recursion; Map/Reduce operate on finite collections)
3. **Exhaustive error handling** — all Result/Sum cases are handled
4. **Output correctness** — the output matches the declared type
5. **Partial evaluability** — the program can be partially evaluated with any subset of inputs bound

---

## 5. Scheduling and Parallelism

### 5.1 Automatic parallelism

The scheduler analyzes data dependencies in the expression tree:

**Independent let-bindings** — two let-bindings that don't reference each other's variables can run concurrently:

```json
{
  "urn:eigenius:program:name": "parties",
  "urn:eigenius:program:value": { "Apply CompleteJson ..." },
  "urn:eigenius:program:body": {
    "urn:eigenius:program:name": "facts",
    "urn:eigenius:program:value": { "Apply CompleteText ..." },
    "urn:eigenius:program:body": { "... uses both parties and facts ..." }
  }
}
```

If `facts` doesn't reference `parties`, both can be computed in parallel.

**Map elements** — each element of a Map is independent by definition. The scheduler can fan out across available workers.

**Reduce** — inherently sequential. Each step depends on the previous accumulator value.

### 5.2 Why Map and Reduce are language primitives

Making Map and Reduce language primitives (rather than library functions) gives the scheduler explicit knowledge of parallelism opportunities:

- **Map** → fan-out to N parallel workers, collect results
- **Reduce** → sequential pipeline, no parallelism
- **Independent lets** → analyze dependency graph, parallelize independent subexpressions

If these were encoded as recursive functions, the scheduler couldn't distinguish parallelizable from sequential patterns without program analysis.

---

## 6. Component Model

### 6.1 Two-tier architecture

Unchanged from the previous design:

**Built-in components** — native Rust or TypeScript, compiled into the platform. Full kernel access.

**Extension components** — WASM modules via WASI Component Model. Sandboxed, independently installable, language-agnostic.

### 6.2 Component registration

Components are resources in a layer with declared types:

```json
{
  "@id": "urn:eigenius:components:CompleteText",
  "urn:eigenius:core:is_a": ["urn:eigenius:program:Component"],
  "urn:eigenius:core:description": "LLM text completion",
  "urn:eigenius:core:short_name": "CompleteText",
  "urn:eigenius:program:component:input_type": "urn:eigenius:components:completion:Arguments",
  "urn:eigenius:program:component:output_type": "urn:eigenius:core:string",
  "urn:eigenius:program:component:implementation": "builtin",
  "urn:eigenius:program:component:capability_level": "urn:eigenius:program:capability_levels:io",
  "urn:eigenius:program:component:deterministic": false,
  "urn:eigenius:program:component:fallible": true,
  "urn:eigenius:program:component:error_type": "urn:eigenius:components:completion:Error"
}
```

### 6.3 Capability levels

| Level | IRI | What it can do |
|-------|-----|---------------|
| Pure | `urn:eigenius:program:capability_levels:pure` | Data transformation only. No imports. |
| Read | `urn:eigenius:program:capability_levels:read` | Read from the layer chain. |
| IO | `urn:eigenius:program:capability_levels:io` | Read + network requests (LLM APIs, HTTP). |

---

## 7. ESL Surface Syntax (Future)

The Eigon-JSON representation is verbose for hand-authoring. A surface syntax (ESL) compiles to Eigon-JSON:

```esl
program extract_and_summarize
  : Document → Summary
  = λ input .
    let parties : Parties = CompleteJson(input.letter, extract_prompt) in
    let facts : String = CompleteText(input.letter, facts_prompt) in
    let response : String = CompleteText(
      Construct ResponseInput {
        employee_name = parties.employee_name,
        complaint_facts = facts
      },
      response_prompt
    ) in
    Construct Summary {
      employee_name = parties.employee_name,
      complaint_facts = facts,
      response_letter = response
    }
```

ESL is syntactic sugar over the expression forms. The compiler produces Eigon-JSON resources. ESL design is deferred to a separate specification.

---

## 8. Complete Example

A program that extracts entities from a document and produces a summary:

```json
{
  "@id": "urn:eigenius:example:extract-summarize",
  "urn:eigenius:core:is_a": ["urn:eigenius:program:Program"],
  "urn:eigenius:core:description": "Extract entities and summarize a document",
  "urn:eigenius:program:input_type": "urn:eigenius:example:Document",
  "urn:eigenius:program:output_type": "urn:eigenius:example:Summary",
  "urn:eigenius:program:body": {
    "urn:eigenius:core:is_a": ["urn:eigenius:program:Let"],
    "urn:eigenius:program:name": "entities",
    "urn:eigenius:program:type": "urn:eigenius:example:Entities",
    "urn:eigenius:program:value": {
      "urn:eigenius:core:is_a": ["urn:eigenius:program:Apply"],
      "urn:eigenius:program:function": "urn:eigenius:components:CompleteJson",
      "urn:eigenius:program:argument": {
        "urn:eigenius:core:is_a": ["urn:eigenius:program:Project"],
        "urn:eigenius:program:expression": {
          "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
          "urn:eigenius:program:name": "input"
        },
        "urn:eigenius:program:property": "urn:eigenius:example:text"
      }
    },
    "urn:eigenius:program:body": {
      "urn:eigenius:core:is_a": ["urn:eigenius:program:Let"],
      "urn:eigenius:program:name": "summary_text",
      "urn:eigenius:program:type": "urn:eigenius:core:string",
      "urn:eigenius:program:value": {
        "urn:eigenius:core:is_a": ["urn:eigenius:program:Apply"],
        "urn:eigenius:program:function": "urn:eigenius:components:CompleteText",
        "urn:eigenius:program:argument": {
          "urn:eigenius:core:is_a": ["urn:eigenius:program:Project"],
          "urn:eigenius:program:expression": {
            "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
            "urn:eigenius:program:name": "input"
          },
          "urn:eigenius:program:property": "urn:eigenius:example:text"
        }
      },
      "urn:eigenius:program:body": {
        "urn:eigenius:core:is_a": ["urn:eigenius:program:Construct"],
        "urn:eigenius:program:class": "urn:eigenius:example:Summary",
        "urn:eigenius:program:fields": {
          "urn:eigenius:example:entities": {
            "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
            "urn:eigenius:program:name": "entities"
          },
          "urn:eigenius:example:summary_text": {
            "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
            "urn:eigenius:program:name": "summary_text"
          },
          "urn:eigenius:example:title": {
            "urn:eigenius:core:is_a": ["urn:eigenius:program:Project"],
            "urn:eigenius:program:expression": {
              "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
              "urn:eigenius:program:name": "input"
            },
            "urn:eigenius:program:property": "urn:eigenius:example:title"
          }
        }
      }
    }
  }
}
```

In this example, `entities` and `summary_text` are independent let-bindings (neither references the other). The scheduler can compute both in parallel.

---

## 9. Decisions Log

| Question | Decision | Rationale |
|----------|----------|-----------|
| Program representation | Expressions in Eigon-JSON, not workflow graphs | Direct 1:1 mapping to EigenTT; no translation layer needed |
| Map and Reduce | Language primitives, not components | Enables scheduler to identify parallelism; type checker understands them natively |
| Parallelism | Automatic from data dependencies + explicit Map | Scheduler infers concurrency from independent let-bindings and Map |
| Surface syntax | ESL (future); Eigon-JSON is the canonical form | JSON is machine-processable; ESL is for humans |
| Component model | Unchanged: built-in + WASM, capability levels | Components are typed functions; calling them is just Apply |
| Namespace | `urn:eigenius:program:` for expression classes | Distinct from `urn:eigenius:core:` |
