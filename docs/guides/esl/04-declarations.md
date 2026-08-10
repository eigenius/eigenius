# 4. Declarations

ESL's top-level declaration forms are `namespace`, `class`, `property`, `resource`, `axiom`, `def`, `text_index`, `vector_index`, `data`, `codata`, `program`, and `macro`. Each compiles to one or more Eigon-JSON resources (except `macro`, which is compile-time only); the section for each form below shows the syntax, the emitted resource shape, and the kernel mapping.

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

## 4.4a. `axiom` — postulated propositions (D46 §10)

```esl
axiom ex:propext :
    forall (P : Prop, Q : Prop) => (P <-> Q) -> Id(Prop, P, Q)
```

The `axiom` keyword takes a name, a colon, and a type expression. The statement is **postulated** — the kernel admits an inhabitant of the type without requiring a proof term, treating the axiom's name as an opaque constant equal only to itself by symbol identity. Conversion never `delta`-reduces it. The chain validator type-checks the *statement* against the universe ladder at commit and rejects malformed propositions; the inhabitant is granted by fiat.

Axioms are the chain-author surface for the kernel's "admit without proof" mechanism. Anything that lives here becomes a citable chain artifact downstream reasoning can name via `DeclaredEvidence`; anything that doesn't can't enter the trust base silently. (For a named term the kernel *unfolds* instead of postulating — definitional equality rather than fiat — see `def`, [§4.4c](#44c-def--transparent-definitions-d66).)

### Type-expression sub-grammar

The body to the right of the `:` is a type expression with the following forms (everything Layer 2 needs for proposition authoring):

- `forall (x : T, y : U) => body` — value-typed Π binders (alias for `pi`).
- `A -> B` — non-dependent arrow.
- `Prop` / `Set` / `Type N` — sort literals.
- `Id(A, x, y)` — equality at type `A`.
- Constructor references, applied or nullary: `ex:Eq(A, x, y)`, `ex:zero`.

The body need not be in `Prop` — `Set`-level axioms (e.g., postulating the existence of a particular structure) are admitted by the same mechanism — but the audit invariant (§ "What's rejected" below) means almost every chain-author axiom is a Prop.

### The optional `note:` clause

Records the human-readable justification — why this axiom is being admitted, what trust assumption it encodes. Surfaces in the chain artifact as `eigentt:axiom_justification`:

```esl
axiom ex:proof_irrelevance :
    forall (P : Prop, p : P, q : P) => Id(P, p, q)
note: "Folklore; built into the kernel's Prop universe per D46 §5."
```

### Wire shape

The declaration commits a Resource of class `eigentt:Axiom` with:

- `eigentt:axiom_statement` — the type expression, D47-encoded as a chain-resident `core:EigenTTType` value via the [type-fragment codec](../../../kernel/src/program/eigentt_type_mirror.rs) (see [§5.14a](05-expressions.md#5-14a-type_expr-eigentt-type-expressions) for the surface-syntax sibling that produces the same wire shape);
- `eigentt:axiom_justification` (optional) — the `note:` string.

At env-build time the kernel's `build_axiom_env` walks every layer in the active chain, decodes each axiom's `axiom_statement` against the universe ladder, and registers the axiom's IRI as an opaque constant in the type-checking environment. From that point the axiom is citable from any program in the chain.

### Default-admitted axioms

Two axioms are admitted by the kernel itself in the initial environment — they appear as `core:Axiom` resources in the core ontology layer, registered by the same `build_axiom_env` mechanism that handles user axioms (no kernel-special-case):

- **`core:propext`** — propositional extensionality: `forall (P Q : Prop), (P <-> Q) -> Id(Prop, P, Q)`. Gives the chain a canonical-assertion identity (logically equivalent propositions are propositionally equal). Conservative over CIC.
- **`core:Quot_sound`** — quotient soundness: `forall {α : Type} {r : α -> α -> Prop} {a b : α}, r a b -> Id(Quot r, Quot.mk r a, Quot.mk r b)`. Needed for evidence normalization, chain consolidation deduplication, and standard mathematical quotient constructions. The rest of `Quot` (`Quot.mk`, `Quot.lift`) is definitional; only `Quot.sound` is axiomatic.

You can cite either from a `DeclaredEvidence(core:propext)` justification just as you would a user-declared axiom — the witness shape is the same.

### What's rejected at kernel level

`Classical.choice` is **not** admitted as a kernel constant. With `propext` and `Quot.sound` already in scope, admitting choice would give excluded middle on all of `Prop`; excluded middle lets the system derive `P` from `¬¬P` *without producing evidence*. For an institution like [D39](../../design/d39-justification-logic.md) whose entire purpose is to anchor every Prop-level belief to a chain-traceable justification, classical phantoms break the audit invariant.

Institutions that need classical reasoning (e.g., a future Mathlib-style verification institution) admit `Classical.choice` as a **per-institution axiom** — a `core:Axiom` resource in the institution's own layer. Every derivation that depends on that institution's layer carries the axiom's IRI in its chain provenance; consumers can refuse to admit derivations whose chain transitively cites it.

The structural property the framework upholds: **every Prop-level belief in a chain traces to either a constructive proof term or a citable, layer-scoped axiom**. There is no kernel-level escape hatch.

### Composing with D39 reasoning

To cite an axiom from a [D39 reasoning sentence](09-institutions.md), pair the `axiom` declaration with a [D49](../../design/d49-chainwitness-machinery.md) `DeclarationTrace` pointing at the axiom resource — that admits the `IsDeclaredAs(axiom_iri, statement)` witness the certificate's `JustifiedBy.declared` constructor consumes:

```esl
axiom ex:strong_inhibitor_implication :
    forall (c : core:string) =>
        screen:HasLowIC50(c) -> screen:StrongInhibitor(c)
note: "Standard medicinal-chemistry threshold; CLSI EP09 alignment."

resource ex:strong_inhibitor_implication_trace : reflection:DeclarationTrace {
    reflection:resource    = ex:strong_inhibitor_implication;
    reflection:declared_by = "literature:smith_et_al_2024";
    reflection:timestamp   = "2026-04-10T09:00:00Z";
}
```

A D39 reasoning sentence then references the axiom via `DeclaredEvidence("urn:eigenius:example:strong_inhibitor_implication")` in its `justification` and `declared(...)` in its certificate. See [§9.10](09-institutions.md) for the full reasoning surface.

### Voiding semantics

Voiding a layer that introduced an axiom removes the axiom from the kernel environment for any chain resolution that excludes that layer. Downstream proofs that depended on it become unreachable through that resolution; cited derivations in *other* resolutions remain intact (their layer still has the axiom). This is the same provenance discipline every other chain artifact follows — axioms aren't special.

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

## 4.4c. `def` — transparent definitions (D66)

```esl
def onco2:HasActivity(m : Set, g : Set, a : Set) : Prop =
    wn:v02203362_t(
        eigentt:fst(ontology:the(
            (exists x0 : wn:n13440063 =>
                logic:And(
                    ontology:compound_kind(x0, a),
                    ontology:prep_of(x0, ontology:kind_of(g))
                )))),
        ontology:kind_of(m));
```

The `def` keyword takes a name, an optional parenthesised parameter list, a result type, and — after `=` — a body. Both the result type and the body are type expressions (the same sub-grammar `axiom` uses); the parameters use the `forall` production's `TypedParam`, so higher-order parameters like `(P : T -> Prop)` are accepted. An optional `desc : "…"` clause before the terminating `;` records a `core:description`.

Where an `axiom` is **postulated** — an opaque constant conversion never unfolds — a `def` is **transparent**: the D47 decoder unfolds a reference to it by substituting the body at the use site (peel-and-substitute, [`eigentt_type_mirror.rs`](../../../kernel/src/program/eigentt_type_mirror.rs)), so `HasActivity(m, g, a)` and its unfolded body are the *same term* up to definitional equality. Nothing has to assert the connection between an abbreviation and what it abbreviates; the kernel computes it (D66 §2). This is how `demo/prose-to-formulas` lifts parsed sentences into domain vocabulary without a single Declared bridge.

### Wire shape

The declaration commits a Resource of class `eigentt:Definition` with two D47-encoded halves ([`compile_def`](../../../kernel/src/esl/compile.rs)):

- `eigentt:definition_type` — one `Pi` per parameter ending in the result type: `Pi (m : Set). Pi (g : Set). Pi (a : Set). Prop`;
- `eigentt:definition_body` — the corresponding lambda chain `Lam(m, Lam(g, Lam(a, ⟨body⟩)))`.

Arity and parameter types live only in the type, so a stored arity can never contradict it. The body is stored **as written**, not normalized by the compiler — D9 requires the stored body to be its own normal form, and Rule 24 refuses one that is not, rather than silently rewriting the author's term.

### Rule 24 — what commit checks

Validation ([`validation/mod.rs`](../../../kernel/src/validation/mod.rs)) rejects a definition whose:

1. body references its own IRI (recursion — checked first, **on the encoded form**, because decode would unfold the cycle);
2. body fails to decode against the chain;
3. body is not β-normal;
4. body does not inhabit `definition_type` (checked *against* the declared type — a lambda chain has no inferable type, which is why `eigentt:definition_body` is exempt from Rule 21's `check_infer`; the type itself is not exempt, and Rule 10's restrictive domain keeps a `definition_body` from riding on any class but `eigentt:Definition`).

### Opacity

`eigentt:definition_opaque` exists in the ontology (issue #95): an opaque definition decodes to a rigid `EigonAxiom` — never unfolded, its identity the folded name — but with a body that was type-checked and then sealed (Coq's `Qed`), unlike an axiom's pure assertion. The ESL surface does **not** accept an `opaque` modifier yet; `axiom` remains the authoring surface for opaque statements.

`eigenius decompile` prints a definition as a generic `resource … : eigentt:Definition { … }` block, which reparses; round-trip is pinned by `a_definition_round_trips_through_the_printer`.

Parser: [`parse_def`](../../../kernel/src/esl/parser.rs). AST: [`DefDecl`](../../../kernel/src/esl/ast.rs). Substitution: [`nbe/subst.rs`](../../../kernel/src/nbe/subst.rs).

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

### Indexed — D48 indexed families

Indexed inductives carry an **index telescope** between the parameters and the result sort. Each constructor's conclusion specifies *values* for the indices (not just the types), and pattern matching against an indexed scrutinee can refine the expected type per arm. This is the surface that lets us express length-indexed vectors, equality on a type, the [D39 `JustifiedBy(justification, proposition)`](../../design/d39-justification-logic.md) certificate, and any other family where the conclusion shape depends on the scrutinee.

```esl
data ex:Vec(A : core:Set) : core:Nat -> Set {
    nil  : ex:Vec(A, ex:zero),
    cons : forall (n : core:Nat) => A -> ex:Vec(A, n) -> ex:Vec(A, ex:succ(n)),
}
```

The clause `: core:Nat -> Set` after the params declares the index telescope (one anonymous index of type `core:Nat`) and the result sort (`Set`). Constructors switch to the **typed form** `name : <type-expr>` — required for indexed inductives because the positional form can't express conclusion indices. The full Π-telescope including the conclusion is supplied directly.

**Parameters vs. indices.** Parameters are fixed across all constructors (`A` is the same on `nil` and `cons`); indices may vary (`nil` concludes at `Vec(A, zero)`, `cons` at `Vec(A, succ(n))`). The distinction matters for elaboration: parameters can be inferred from the scrutinee's type without unification, indices generally cannot. See [§7.3a](07-type-theory-primer.md) for the kernel-side pattern unification.

A propositional equality, declared in `Prop` rather than `Set`:

```esl
data ex:Eq(A : core:Set) : A -> A -> Prop {
    refl : forall (a : A) => ex:Eq(A, a, a),
}
```

The two indices have type `A` (a parameter reference — the compiler keeps it as a bare name and the kernel decodes it as `Exp::Var(A)` bound by the parameter telescope). `refl` is the only constructor, and its conclusion forces both indices to be the *same* `a`: there is no syntactic way to produce an inhabitant of `Eq(A, a, b)` for distinct `a`, `b`. This is exactly the type-level discipline that makes equality reasoning load-bearing — the type system rejects the case branches that would observe distinct equal things.

Indexed inductives in `Prop` also enable the [singleton-elimination rule](07-type-theory-primer.md) the kernel uses to admit pattern matching on propositional witnesses without breaking proof irrelevance — see [D48 §4.7](../../design/d48-indexed-inductive-families.md) for the elaboration rule and [§7.3a](07-type-theory-primer.md) for the user-facing summary.

### Wire shape

Indices land on `core:indices` (array of `InductiveParam` resources, parallel to `type_params`), result sort on `core:result_sort` (string: `Prop` / `Set` / `Type:N`), and each typed ctor on `core:ctor_type` (the full Π-telescope D47-encoded via the [type-fragment codec](../../../kernel/src/program/eigentt_type_mirror.rs); see [§5.14a](05-expressions.md#5-14a-type_expr-eigentt-type-expressions) for the surface-syntax sibling that produces the same wire shape). Non-indexed declarations omit all three fields, preserving the pre-Layer-2 wire shape.

You don't author the `core:EigenTTType` value directly — the compiler produces it from the `forall (n : core:Nat) => ...` source you write — but understanding that it exists as a first-class chain value explains why indexed inductives can express dependencies in the first place: the constructor's type telescope is *data* the kernel reads back out of the layer at type-check time, not implicit elaboration.

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

## 4.5a. Multi-class `data` declarations — marker classes (D52 §12 #8)

A comma-separated list of qualified-name classes after the result-sort clause (or after the name, when no result-sort clause is present) decorates the emitted inductive-type resource with additional `is_a` membership:

```esl
data screen:HasLowIC50 : core:string -> Prop, stats:PopulationLevel {
}

data assay:HasLowIC50_OnThisBatch : core:string -> Prop, stats:MeasurementLevel {
}
```

The inductive-type resource's `core:is_a` array carries both the implicit `core:InductiveType` membership and the comma-separated extras (`stats:PopulationLevel` here). Parallels the existing multi-parent `class X : P1, P2` syntax.

### What it's for

Institutions that need to attach **scope** or **policy** metadata to predicates use this surface to encode that metadata at the predicate's declaration site. The reading institution then walks `predicate_resource.is_a()` and decides admissibility from the markers it finds — no companion resource, no separate property, no out-of-band convention. The two motivating examples ride together:

- **D52 §7.4 epistemic-scope check.** Predicates ride one of two scope markers, and the statistics institution's epistemic-scope gate consults them at StatisticalAnalysisPlan admissibility time:
  - `stats:PopulationLevel` — generalizes to the underlying biological population. Admissible from `BiologicalReplication` and `NestedReplication`; rejected from `TechnicalWithinRun` (the institution refuses to attest population claims from technical-only replicates).
  - `stats:MeasurementLevel` — local to a single measurement event (e.g., `HasLowIC50_OnThisBatch`). Admissible from any replication kind including `TechnicalWithinRun`.
  - Predicates with **no** explicit scope marker default to `PopulationLevel` (the more restrictive admissibility — fail-safe). See [D52 §7.4](../../design/d52-measurement-statistics-institution.md) for the full admissibility table.

- **D52 §7.1 impossibility witnesses.** A resource (not necessarily an inductive — `class` instances can use the same `is_a`-marker pattern via multi-parent `class` syntax) carrying `is_a stats:ImpossibilityWitness` stands as proof that the inverse direction of some hypothesis is physically impossible within the system under study. The statistics institution accepts `Directionality.OneSidedWitnessed(witness_iri)` on a StatisticalAnalysisPlan only when the witness IRI resolves to a chain resource carrying this marker; otherwise it rejects the claim with a structured diagnostic.

### Why this syntax

The alternative — a companion resource (`PredicateMetadata { predicate_iri = …; scope = PopulationLevel }`) — works structurally but introduces:

- A second IRI per predicate, doubling the chain footprint and requiring authors to remember to commit both;
- A lookup step in the reading institution (resolve the companion resource by some convention), which adds a place where the companion can be missing without anything noticing;
- A divergence between "what the chain says about this predicate" and "where the predicate's identity lives."

The multi-class header puts the scope decision *on* the declaration, where the predicate's identity is. Removing the marker removes it from the chain artifact in the same commit that removes the declaration — the metadata can't outlive what it describes.

### Wire shape

The emitted `core:InductiveType` resource's `core:is_a` array contains `core:InductiveType` (implicit) plus each comma-separated qualified name in declaration order. Reading institutions consult `resource.is_a()` directly — there is no separate "markers" property.

### Compatibility with other `data` axes

The extras list is orthogonal to the parametric / indexed / sized axes of [§4.5](#4-5-data-inductive-types) and composes with all of them:

```esl
data ex:WeightedTree(A : core:Set) : core:Nat -> Set, ex:Persistable, ex:Auditable {
    leaf : A -> ex:WeightedTree(A, ex:zero),
    node : forall (n m : core:Nat) =>
        ex:WeightedTree(A, n) -> ex:WeightedTree(A, m) ->
        ex:WeightedTree(A, ex:succ(n, m)),
}
```

The result-sort clause ends in `Set` (or `Prop` / `Type N`); commas after the sort literal start the extras list. The parser's structural terminator for the type expression is `{` (data body) or `,` (next extra class) — both unambiguous even when the indices' Π-telescope contains commas in its binder list, because those are bracketed inside `(... : ...)` groups.

When no result-sort clause is present (non-parametric, non-indexed shape), the extras start directly after the name:

```esl
data ex:Color, ex:Enumerable {
    red,
    green,
    blue,
}
```

Source: [`parse_data` (parser entry point)](../../../kernel/src/esl/parser.rs), [`compile_data` (extras → is_a)](../../../kernel/src/esl/compile.rs).

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

## 4.9. `macro` — compile-time smart constructors (D52 §12)

```esl
macro stats:SingleSampleEstimate(
    measurements : core:value_array,
    replication  : stats:Replication,
) : stats:SampleSet =>
    Bundle(
        CompleteRandom(),
        Unblocked(),
        NoFactor(),
        replication,
        CrossSectional(),
        Units([]),
        Columns([]),
        Entries([]),
        measurements,
    );
```

A `macro` declares a **compile-time AST-substitution smart constructor**. At every call site, the compiler substitutes positional argument values into the body and recursively compiles the result. The macro itself is not a runtime function — it produces no `program` resource, has no callable body at runtime, and never appears in a program trace. Its job is to give the author a compact authoring surface for a sum-type ctor that would otherwise require pages of positional bookkeeping.

The surface form:

```
macro <qualified-name>(p1 : T1, p2 : T2, …) : RetType => body;
```

Trailing semicolon optional.

### Where it fits

The motivating use is [D52's universal `SampleSet` Bundle ctor](../../design/d52-measurement-statistics-institution.md): every statistical design lands at the same 9-arg `Bundle(randomization, blocking, factor, replication, repeated_measures, units, columns, sample_map, observations)` product position. Writing that out at every commit site would bury the author's intent under axis bookkeeping; with `macro` they can write:

```esl
stats:IID([72.0, 85.0, 100.0], [88.0, 95.0], BiologicalReplication())
```

and the compiler expands it to the full `Bundle(...)` ctor with `CompleteRandom()`, `Unblocked()`, `SingleFactor()`, `CrossSectional()`, and the empty topology slots filled in. The verifier's dispatch then reads the bundle's product position directly — the macro is just authoring-time sugar over the wire shape the verifier actually consumes.

### Restrictions (v1)

- **Body must be a `Value`** (resource-property value AST), not a `TypeExpr` or a program `Expr`. The use case is producing ctor values; programs have their own decl form ([§4.7](#4-7-program)).
- **No recursion.** Macro expansion is one-shot per call site; recursive calls have no termination guarantee and the smart-constructor use case doesn't need them. Cycles are caught by the compiler with a structured diagnostic.
- **Positional only, no defaults.** Each call site supplies arguments in declared order; ESL doesn't have keyword arguments or default values. Authors who want to abbreviate further can declare a thinner macro that delegates to the wider one.

### Type-checking discipline

The parameter types and return type are **stored but not enforced at the macro-decl site** — they're metadata for diagnostics. Type errors surface at the **expanded call site** against the body's substituted shape. This matches the AST-substitution model: the macro is sugar, the actual type-checking happens against the elaborated form, and the diagnostic should point at the call site where the author has the source context to fix the error.

If you author a macro whose body's type doesn't match the declared `RetType`, the compiler doesn't reject the declaration — every call site will reject instead, with the call site's diagnostic carrying the type mismatch. (A future hardening could check the macro-decl-site shape; v1 lives with the call-site-only check.)

### Wire shape

Unlike every other top-level declaration, a `macro` declaration commits a `core:Macro` resource that carries the **serialized `MacroDecl` AST** as its `core:macro_decl` property:

```json
{
  "@id": "urn:eigenius:measurements:SingleSampleEstimate",
  "is_a": ["urn:eigenius:core:Macro"],
  "core:macro_decl": "{...serialized MacroDecl JSON...}"
}
```

This is the mechanism that makes macros **visible across files** — a child file compiled against a layer that includes the macro's declaration can invoke it by qualified name without re-declaring it locally. See [§6.5](06-resources-types-and-the-layer.md) for the cross-file visibility surface (`esl::compile_against_layer`).

### When to reach for a macro vs. a `program`

Use a `macro` when:
- The expansion produces a fixed-shape ctor or value the author wants to abbreviate.
- The "function" has no runtime semantics — there's nothing to evaluate at use time other than producing the ctor.
- Cross-file authoring ergonomics matter (the macro lives in an ontology layer, fixtures use it).

Use a `program` ([§4.7](#4-7-program)) when:
- The function takes a real input value and produces a transformed output.
- The body has computational content the kernel should evaluate.
- The function may be cited by reasoning sentences via `DerivedEvidence` — programs leave provenance through traces, macros don't.

Source: [`parse_macro`](../../../kernel/src/esl/parser.rs), [`MacroDecl` AST type](../../../kernel/src/esl/ast.rs), [`compile_macro_resource`](../../../kernel/src/esl/compile.rs), [`expand_macro_call`](../../../kernel/src/esl/compile.rs), [`collect_macros_from_layer`](../../../kernel/src/esl/compile.rs) (cross-file visibility).

---

Next: **[5. Expressions →](05-expressions.md)**
