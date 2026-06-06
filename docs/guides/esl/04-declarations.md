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

## 4.4a. `axiom` — postulated propositions (eigenius#72 Layer 1, D46 §10)

```esl
axiom ex:propext :
    forall (P : Prop, Q : Prop) => (P <-> Q) -> Id(Prop, P, Q)
```

The `axiom` keyword takes a name, a colon, and a type expression in `Prop`. The statement is **postulated** — the kernel admits an inhabitant of the type without requiring a proof term, parallel to how `propext` and `Quot.sound` are admitted as kernel built-ins. The chain validator type-checks the statement at commit and rejects malformed propositions.

The type-expression sub-grammar accepts everything Layer 2 needs for proposition authoring:

- `forall (x : T, y : U) => body` — value-typed Π binders (alias for `pi`).
- `A -> B` — non-dependent arrow.
- `Prop` / `Set` / `Type N` — sort literals.
- `Id(A, x, y)` — equality at type `A`.
- Constructor references, applied or nullary: `ex:Eq(A, x, y)`, `ex:zero`.

Optional `note: "…"` clause records the human-readable justification:

```esl
axiom ex:proof_irrelevance :
    forall (P : Prop, p : P, q : P) => Id(P, p, q)
note: "Folklore; built into the kernel's Prop universe per D46 §5."
```

**Wire shape.** Commits a Resource of class `eigentt:Axiom` with:
- `eigentt:axiom_statement` — the type expression, D47-encoded via the [type-fragment codec](../../../kernel/src/program/eigentt_type_mirror.rs);
- `eigentt:axiom_justification` (optional) — the `note:` string.

The axiom registers into the layer's axiom environment via `build_axiom_env` at the next environment build and is then citable from `DeclaredEvidence` justifications per D39 §10.

Source: [`parse_axiom`](../../../kernel/src/esl/parser.rs), [`compile_axiom` and `lower_type_expr_to_exp`](../../../kernel/src/esl/compile.rs), [`build_axiom_env`](../../../kernel/src/program/axiom_env.rs).

## 4.4b. `text_index` and `vector_index`

Sugar over `core:TextIndex` / `core:VectorIndex` Resource declarations from D43. Each declaration commits one Index Resource targeting a property; the kernel's text-search and vector-search dispatchers pick it up via the active-index lookup at the head layer. See **[EigenQL guide chapter 6](../eigenql/06-text-and-vector-retrieval.md)** for the query-side surface (`~` operator) these declarations enable.

### `text_index`

```esl
text_index ex:description_en {
    core:target_property = ex:description;
    core:text_analyzer = "en-stem-v1";
}
```

Compiles to a `core:TextIndex` Resource with the body fields preserved as properties. Required slots:

- `core:target_property` — the Property whose values get indexed.

Recommended:

- `core:text_analyzer` — analyzer identifier (default `"en-stem-v1"`); see [`analyzer/registry`](../../../kernel/src/query/text/analyzer.rs) for the shipped set (`en-stem-v1`, `en-no-stem`).

At commit time, [`populate_text_indexes`](../../../kernel/src/query/text/indexing.rs) auto-walks the layer's Resources, tokenises the target property's string values, and writes BM25 posting lists per `(index, layer)` pair.

### `vector_index`

```esl
namespace cd = "urn:eigenius:core:distances";
namespace cs = "urn:eigenius:core:strategies";

vector_index ex:description_oai_v3 {
    core:target_property = ex:description;
    core:vec_model       = ex:openai_text_embedding_3_large_v3;
    core:vec_dim         = 1536;
    core:vec_distance    = cd:cosine;
    core:vec_strategy    = cs:auto;
}
```

Compiles to a `core:VectorIndex` Resource. Required slots:

- `core:target_property` — the Property whose values get embedded and indexed.
- `core:vec_model` — IRI of an `Embedder` Component that produces the vectors.
- `core:vec_dim` — embedder output dimensionality (must match the Embedder's declared `dim()`; verified at parse time per the dimensionality recommendation in D43 §3.1).

Recommended:

- `core:vec_distance` — one of `cd:cosine`, `cd:l2`, `cd:dot`. Default `cd:cosine`.
- `core:vec_strategy` — `cs:flat`, `cs:hnsw`, or `cs:auto` (auto-promotes to HNSW above a segment-size threshold). Default `cs:auto`.
- `core:vec_hnsw_m`, `core:vec_hnsw_ef_construction` — HNSW build parameters (defaults 16 / 200).
- `core:vec_embedding_policy` — `eager_on_load` (default) / `lazy_on_query` / `manual`. v1 ships `eager_on_load` only.

**Nested-IRI scopes:** the `urn:eigenius:core:distances:cosine` style of nested URN can't be written as `core:distances:cosine` because ESL's `QualifiedName` is single-colon. Declare an additional namespace alias (`namespace cd = "urn:eigenius:core:distances"`) and use `cd:cosine` instead. Same pattern for strategies (`namespace cs = "urn:eigenius:core:strategies"`).

**Vector-index population** runs through the post-Load sweep ([`sweep_layer_vectors`](../../../kernel/src/query/vector/indexing.rs)) — it needs an Embedder Component the kernel can dispatch. Without one registered, the VectorIndex Resource still commits but no segments exist; queries against it return empty until the sweep completes.

**v1 multiplicity.** At most one TextIndex and at most one VectorIndex per target Property per head — both can coexist on the same Property (the hybrid retrieval case). The constraint is verified by [`verify_text_index_multiplicity`](../../../kernel/src/layer/index_discovery.rs) and `verify_vector_index_multiplicity`.

Parser: [`parse_text_index`](../../../kernel/src/esl/parser.rs) / [`parse_vector_index`](../../../kernel/src/esl/parser.rs). AST: [`TextIndexDecl`](../../../kernel/src/esl/ast.rs) / [`VectorIndexDecl`](../../../kernel/src/esl/ast.rs).

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

### Indexed — D48 indexed families (eigenius#72 Layer 2)

Indexed inductives carry an **index telescope** between params and the result sort. Each constructor's conclusion specifies values for the indices, and pattern matching against an indexed scrutinee can refine the expected type per arm.

```esl
data ex:Vec(A : core:Set) : core:Nat -> Set {
    nil  : ex:Vec(A, ex:zero),
    cons : forall (n : core:Nat) => A -> ex:Vec(A, n) -> ex:Vec(A, ex:succ(n)),
}
```

The clause `: core:Nat -> Set` after the params declares the index telescope (one anonymous index of type `core:Nat`) and the result sort (`Set`). Constructors switch to the **typed form** `name : <type-expr>` — required for indexed inductives because the positional form can't express conclusion indices. The full Π-telescope including the conclusion is supplied directly.

A propositional equality, declared in `Prop` rather than `Set`:

```esl
data ex:Eq(A : core:Set) : A -> A -> Prop {
    refl : forall (a : A) => ex:Eq(A, a, a),
}
```

The index kind can be a parameter reference (`A` here) — the compiler keeps it as a bare name and the kernel decodes it as `Exp::Var(A)` bound by the parameter telescope.

**Wire shape.** Indices land on `core:indices` (array of `InductiveParam` resources, parallel to `type_params`), result sort on `core:result_sort` (string: `Prop` / `Set` / `Type:N`), and each typed ctor on `core:ctor_type` (the full Π-telescope D47-encoded via the [type-fragment codec](../../../kernel/src/program/eigentt_type_mirror.rs)). Non-indexed declarations omit all three fields, preserving the pre-Layer-2 wire shape.

Source: [`parse_data_index_telescope`](../../../kernel/src/esl/parser.rs), [`compile_data`](../../../kernel/src/esl/compile.rs), [`decode_indices` and `decode_result_sort`](../../../kernel/src/program/ground.rs).

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
