# 4. Declarations

ESL has seven top-level declaration forms: `namespace`, `class`, `property`, `resource`, `data`, `codata`, `program`. Each compiles to one or more Eigon-JSON resources; the section for each form below shows the syntax, the emitted resource shape, and the kernel mapping.

The AST type for the file root is [`ast::File`](../../../kernel/src/esl/ast.rs):

```rust
pub struct File {
    pub namespaces: Vec<NamespaceDecl>,
    pub declarations: Vec<Declaration>,
}
```

Namespaces are pulled out into their own list because they're scoping declarations, not entities. Everything else lives in `declarations` in source order. The order is preserved through compilation but doesn't affect semantics — every reference goes through IRIs, which resolve through the layer at use time.

## 4.1. `namespace`

```esl
namespace core = "urn:eigenius:core";
namespace ex   = "urn:eigenius:example";
```

Binds an alias to a base URI. Aliases are file-scoped: only declarations in the same `.esl` file see them. A qualified name `ex:Dog` expands to `<base>:<local>` — for `ex` aliased to `urn:eigenius:example`, that's `urn:eigenius:example:Dog`.

A qualified name with no declared alias is a compile-time error (`unknown namespace 'foo'`). The compiler does not pull aliases from elsewhere — every file declares the namespaces it uses.

Source: [`parse_namespace`](../../../kernel/src/esl/parser.rs), [`compile_file`](../../../kernel/src/esl/compile.rs).

## 4.2. `class`

```esl
class ex:Document {
    description = "A text document";
    requires ex:text;
    recommends ex:author, ex:created_at;
}

class ex:Dog : ex:Animal {
    description = "A dog";
    requires ex:breed;
}
```

Compiles to a `Class` resource. Eigon-JSON uses full property IRIs as keys — `@id` is the only reserved short key ([D1](../../design/d1-eigon-serialization-format.md)):

```json
{
    "@id": "urn:eigenius:example:Document",
    "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
    "urn:eigenius:core:short_name": "Document",
    "urn:eigenius:core:description": "A text document",
    "urn:eigenius:core:requires": ["urn:eigenius:example:text"],
    "urn:eigenius:core:recommends": ["urn:eigenius:example:author",
                                     "urn:eigenius:example:created_at"]
}
```

A class declared with `: Parent` adds `urn:eigenius:core:subclass_of: [Parent]`. Multiple parents are not currently supported in the surface syntax.

**Items inside the body** ([`ast::ClassItem`](../../../kernel/src/esl/ast.rs)):

| Item | Effect |
|---|---|
| `description = "..."` | Sets `core:description` on the class resource |
| `requires p1, p2, ...` | Sets `core:requires` to the IRI list. Each property is mandatory on instances |
| `recommends p1, p2, ...` | Sets `core:recommends`. Properties are optional but expected |

**Kernel mapping.** When the kernel encounters `ex:Document` as a type (e.g., in `let d : ex:Document = ...`), it looks up the resource via the layer and constructs a Σ-type whose fields correspond to the class's `requires` properties (`recommends` properties become Option-wrapped). The bridge is in [`ground.rs collect_properties`](../../../kernel/src/program/ground.rs); see [chapter 6](06-resources-types-and-the-layer.md) for details.

Source: [`compile_class`](../../../kernel/src/esl/compile.rs).

## 4.3. `property`

```esl
property ex:text : core:string {
    description = "The text content";
}

property ex:count : core:integer {
    description = "Number of items";
    min_value = 0;
    max_value = 100;
}

property ex:email : core:string {
    pattern = "^[^@]+@[^@]+$";
    format = core:email;
}

property ex:status : core:string {
    allows_only = "active", "pending", "closed";
}
```

Compiles to a `Property` resource carrying `data_type` (the IRI of the property's type) plus optional scalar constraints.

**Items inside the body** ([`ast::PropertyItem`](../../../kernel/src/esl/ast.rs)):

| Item | Property set | Use |
|---|---|---|
| `description = "..."` | `core:description` | Human-readable description |
| `min_value = N` | `core:min_value` | Numeric lower bound (inclusive) |
| `max_value = N` | `core:max_value` | Numeric upper bound (inclusive) |
| `min_length = N` | `core:min_length` | String/array minimum length |
| `max_length = N` | `core:max_length` | String/array maximum length |
| `pattern = "regex"` | `core:pattern` | String regex constraint |
| `format = ns:format` | `core:format` | Named format constraint (e.g. `email`) |
| `allows_only = a, b, c` | `core:allows_only` | Enum-like enumeration of permitted values |
| `domain = C1, C2, ...` | `core:domain` | Restrict the property to instances of these classes |
| `class_types = T1, ...` | `core:class_types` | For properties whose values are class IRIs, restrict the allowed class kinds |
| `element_type = T` | `core:element_type` | For array-typed properties, the element type |

**Kernel mapping.** `data_type` is the IRI of the property's type — typically one of `core:string`, `core:integer`, `core:float`, `core:boolean`, `core:resource`, or `core:resource_array`/`core:value_array` for collections. When the kernel resolves a property's type during type-check (via [`resolve_property_type`](../../../kernel/src/program/ground.rs)), it reads `data_type` and constructs the corresponding kernel `Val`.

Constraints are turned into kernel `Constraint` values that fire during type-check in `Check` capability mode — see [chapter 8](08-capability-modes.md). Institution-registered decide procedures attach to the same `data_type` machinery (Phase 11c — see [chapter 9](09-institutions.md)).

Source: [`compile_property`](../../../kernel/src/esl/compile.rs), [`resolve_property_type`](../../../kernel/src/program/ground.rs).

## 4.4. `resource`

```esl
resource ex:rex : ex:Dog {
    ex:name = "Rex";
    ex:breed = "German Shepherd";
}
```

Compiles to a resource with `is_a: [class_iri]` and one property assignment per field. Property names are full qualified names — there's no shortcut for "use the bare name from the class's declared properties", because the same property IRI can be reused across classes and resolution is deliberately explicit.

Field values can be any of the literal forms: strings, integers, floats, booleans, refs to other resources (qualified names), arrays of values, or nested embedded resources via `{ ... }` blocks.

**Constraint evaluation.** Constraints declared on the property fire at load time when the layer is built — out-of-range values, pattern mismatches, etc., are rejected before the resource enters the layer.

Source: [`compile_resource`](../../../kernel/src/esl/compile.rs).

## 4.5. `data` — inductive types

Inductive type declarations introduce a new type with a finite list of constructors. Recursive references to the type itself are allowed (and are exactly what makes inductives interesting). Sized inductives carry a size parameter and use bounded binders to track strictly-decreasing recursion.

### Non-parametric, non-sized

```esl
data ex:Nat {
    zero,
    succ(ex:Nat),
}
```

Two constructors: `zero` is nullary; `succ` takes one argument of type `Nat`. No type parameters.

### Parametric

```esl
data ex:List(A : core:Set) {
    nil,
    cons(A, ex:List(A)),
}
```

`List` is parameterised by the element type `A`. Constructor argument types may reference parameters by bare name (`A`) or by full IRI (`ex:List(A)`).

### Sized — bounded binders

```esl
data ex:SizedNat(i : core:Size) {
    zero,
    succ({j < i}, ex:SizedNat(j)),
}
```

The `{j < i}` form is a **bounded binder**: it introduces a fresh size variable `j` strictly less than `i`. The constructor's full kernel telescope becomes:

```
Π i : Size. SizedPi { j < i }. Π _ : SizedNat(j). SizedNat(i)
```

The `SizedPi { j < i }` binder is the form that powers the kernel's termination check ([D19 §3](../../design/d19-inductive-types.md)). Without it, recursion on `SizedNat` would not be guaranteed to terminate.

Three bounded-binder shapes are accepted ([`ast::CtorArg::Named`](../../../kernel/src/esl/ast.rs)):

```esl
{j < i}                  // shorthand for {j : core:Size < i}
{j : core:Size}          // unbounded — no upper limit
{j : core:Size < i}      // explicit kind + bound
```

**Constructor IRIs.** Each constructor gets its own resource at `<parent_iri>:<ctor_name>` — e.g., `urn:eigenius:example:SizedNat:succ`. This makes constructors first-class graph entities that EigenQL can query.

**Positivity.** Recursive references must appear in strictly positive positions ([D19 §6](../../design/d19-inductive-types.md), [`positivity.rs`](../../../kernel/src/nbe/positivity.rs)). The compiler doesn't enforce positivity itself; the type checker rejects non-positive declarations when they're loaded.

Source: [`compile_data`](../../../kernel/src/esl/compile.rs), [`compile_ctor_arg_type`](../../../kernel/src/esl/compile.rs), [`compile_ctor_binder`](../../../kernel/src/esl/compile.rs), [`decode_arg_type` and `decode_ctor_arg`](../../../kernel/src/program/ground.rs).

## 4.6. `codata` — coinductive types

Coinductive types are dual to inductives: instead of being built from constructors, they're consumed via observations. A `codata` declaration lists the observations and their result types.

### Non-parametric

```esl
codata ex:IntStream {
    head : core:integer;
    tail : ex:IntStream;
}
```

Two observations: `head` returns an integer, `tail` returns another `IntStream`.

### Parametric

```esl
codata ex:Stream(A : core:Set) {
    head : A;
    tail : ex:Stream(A);
}
```

Parameterised by element type `A`.

### Sized — productivity by typing

```esl
codata ex:Stream(A : core:Set, i : core:Size) {
    head : A;
    tail : {j < i} -> ex:Stream(A, j);
}
```

The `tail` observation has a function-typed shape: `{j < i} -> ex:Stream(A, j)`. To consume `tail` you supply a strictly smaller size `j` and observe the continuation at that smaller size. The kernel uses this to verify productivity of corecursive definitions — every observation chain eventually terminates because sizes strictly decrease ([D19 §8](../../design/d19-inductive-types.md)).

Observation types are written in the **`TypeExpr` sublanguage** ([`ast::TypeExpr`](../../../kernel/src/esl/ast.rs)):

| Form | Compiles to |
|---|---|
| `Name` or `Name(arg, ...)` | `Exp::Pi`/parameterised type ref |
| `A -> B` | Non-dependent `Exp::Pi(_, A, B)` |
| `{j : K} -> body` | `Exp::Pi(j, K, body)` |
| `{j < i} -> body` | `Exp::SizedPi { j, upper: i, body }` |
| `{j : core:Size < i} -> body` | `Exp::SizedPi { j, upper: i, body }` (explicit kind) |

**Self-references.** A codata observation type may mention the enclosing codata by IRI applied to fresh args (`ex:Stream(A, j)`). The compiler emits an `Exp::CodataType` that carries the self-reference, completing what [D19 §8.2](../../design/d19-inductive-types.md) calls the *parameterised self-referential codata pattern*.

Source: [`compile_codata`](../../../kernel/src/esl/compile.rs), [`compile_type_expr`](../../../kernel/src/esl/compile.rs), [`resolve_codata_type` and `decode_codata_observation_type`](../../../kernel/src/program/ground.rs).

## 4.7. `program`

```esl
program ex:identity : ex:Document -> ex:Document {
    input
}

program ex:summarize : ex:Document -> ex:Summary {
    let entities : ex:Entities = CompleteJson(input.ex:text);
    let summary : core:string = CompleteText(input.ex:text);
    Construct ex:Summary {
        ex:entities = entities,
        ex:summary = summary,
        ex:source = input
    }
}
```

A `program` declares a typed function from one resource type to another. The body is an expression in the ML-style sublanguage covered in [chapter 5](05-expressions.md).

**Implicit `input`.** The parameter is always named `input` — there's no other ceremony. Inside the body, `input` refers to the input value at the declared type.

**Output shape.** The body's type must match the declared output type. The type-checker enforces this via a normal Π-check.

**Compiled form.** A `program` resource carries:

- `is_a: ["urn:eigenius:program:Program"]`
- `program:input_type: <input_iri>`
- `program:output_type: <output_iri>`
- `program:body: <embedded body resource>` — the compiled expression

The body resource is an embedded resource whose `is_a` reflects the top-level expression form (`Let`, `Apply`, `CoRecord`, `Construct`, `NativeDecide`, etc.). Each sub-expression is itself an embedded resource, recursively. This is the program AST encoded as Eigon resources — see [`program/expr.rs`](../../../kernel/src/program/expr.rs) for the parser that recovers the kernel `Exp` from this resource shape.

**Kernel mapping.** The body is parsed by [`parse_program`](../../../kernel/src/program/expr.rs) into a kernel `Exp`. The wrapping `Lam(input, body)` plus an outer `Pi(input_type → output_type)` produces a closed term that the type-checker can verify via [`check_infer`](../../../kernel/src/nbe/check.rs).

Attributes (currently only `description = "..."`) appear before the body and are stored as resource properties for documentation purposes.

Source: [`compile_program`](../../../kernel/src/esl/compile.rs), [`parse_program`](../../../kernel/src/program/expr.rs).

## 4.8. Compilation order and stamping

The compiler walks `File.declarations` in source order and emits resources in roughly the same order (with constructors inlined as embedded resources within their parent inductive). Each resource gets a `core:declared_in` stamp identifying it as ESL-declared (not synthesized); this is mainly diagnostic.

The output is a `Vec<Resource>` ready to be loaded into a layer. Once loaded, all ontology-as-types resolution becomes available — declarations made in one file are usable as types in any subsequent compilation that has access to the same layer.

---

Next: **[5. Expressions →](05-expressions.md)**
