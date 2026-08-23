# 2. Quick tour

Six worked examples. Each is short, drawn from the test suite, and exercised end-to-end. Read them top to bottom — together they sketch the whole language.

## 2.1. Class, property, resource

The minimal structural ontology: declare a class, declare the property it requires, instantiate one resource.

```esl
namespace core = "urn:eigenius:core";
namespace ex   = "urn:eigenius:example";

class ex:Document {
    description = "A text document";
    requires ex:text;
}

property ex:text : core:string {
    description = "The text content";
}

resource ex:doc1 : ex:Document {
    ex:text = "Hello world";
}
```

Compiles to three Eigon-JSON resources: a `Class` resource at `urn:eigenius:example:Document`, a `Property` resource at `urn:eigenius:example:text` (with `data_type = core:string`), and a resource at `urn:eigenius:example:doc1` whose `is_a` includes `ex:Document` and which carries `ex:text = "Hello world"`. Once loaded into a layer, the kernel can resolve `ex:Document` *as a Σ-type* with one required field — see [chapter 6](06-resources-types-and-the-layer.md).

Source: [`compile_class`](../../../kernel/src/esl/compile.rs), [`compile_property`](../../../kernel/src/esl/compile.rs), [`compile_resource`](../../../kernel/src/esl/compile.rs).

## 2.2. A `program` with `let` and `Construct`

Programs are typed expressions. Inside the body you write ML-style code: `let`, function application, lambda, projection, constructor calls.

```esl
namespace core = "urn:eigenius:core";
namespace ex   = "urn:eigenius:example";

program ex:summarize : ex:Document -> ex:Document {
    let summary : core:string = CompleteText(input);
    Construct ex:Document { ex:text = summary }
}
```

What happens:

- `input` is the implicit parameter name for the program's input.
- `CompleteText(input)` is a **component call** — `CompleteText` resolves to the registered component IRI `urn:eigenius:program:components:CompleteText`. The compiler emits an `Apply` whose function is the IRI.
- `let summary : core:string = ...` binds the component's output to a name.
- `Construct ex:Document { ... }` builds a fresh resource of class `ex:Document` with the given field assignments.

The whole body compiles to a kernel `Exp::Lam(input, Exp::Let(summary, Exp::App(component, input), Exp::Construct(...)))` wrapped by an outer `Pi` for the program's type.

Source: [`compile_simple_program`](../../../kernel/src/esl/compile.rs), [`compile_program_with_let_and_construct`](../../../kernel/src/esl/compile.rs), [`compile_component_shorthand`](../../../kernel/src/esl/compile.rs).

## 2.5. Component call with trailing config block

When a component takes structured configuration in addition to the runtime input, you can pass it as a trailing brace block.

```esl
namespace core = "urn:eigenius:core";
namespace ex   = "urn:eigenius:example";

program ex:translate : ex:Document -> ex:Document {
    CompleteText(input) {
        model = "claude-sonnet";
        temperature = 0.2;
    }
}
```

The trailing `{ ... }` desugars to an embedded resource passed to the component as a configuration value. Components see configuration fields as keyword-style options the framework supplies; the input position carries the runtime value.

Source: [`compile_component_shorthand`](../../../kernel/src/esl/compile.rs).

## 2.6. Institution-dispatched Decidable QueryClass

Programs can invoke an institution's `Decidable` `QueryClass` by qualified name. The compiler classifies the IRI through the [`InstitutionIndex`](../../../kernel/src/institution/registry.rs) (derived from the layer chain under D14) and emits the right kernel form.

```esl
namespace core = "urn:eigenius:core";
namespace cap  = "urn:eigenius:test";
namespace ex   = "urn:eigenius:example";

class ex:Thing {
    requires ex:name;
}
property ex:name : core:string {
    description = "test";
}

program ex:decide_program : ex:Thing -> ex:Thing {
    let v : urn:eigenius:institution:Verdict = cap:cap_decide(input, input);
    input
}
```

When the compiler is given an `InstitutionIndex` where `urn:eigenius:test:cap_decide` is a Decidable `QueryClass`, the call compiles to `Exp::NativeDecide(Constraint::Institution { iri, args }, Unit)` — a kernel form that delegates to the institution's `Institution::query(query_handler, …, ctx)` and reduces the surrounding term according to the returned `Verdict` (`Holds` → `Refl(v)`, `Fails` → failing neutral, `Undecidable` → passthrough).

Compile call:

```rust
esl::compile_with_institutions(source, institution_index)
```

Without the index, the same call falls through to component dispatch and fails at runtime with `unknown function`. See [chapter 9](09-institutions.md) for the full institution surface.

---

These examples cover the full surface area: ontology declarations (2.1), expression-language programs (2.2), component dispatch, institution dispatch. The chapters that follow drill into each.

Next: **[3. Lexical structure →](03-lexical-structure.md)**
