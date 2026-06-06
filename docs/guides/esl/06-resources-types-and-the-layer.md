# 6. Resources, types, and the layer

This is the bridge chapter. Read it carefully — several patterns in chapters 4, 5, 8, and 9 stop making sense without it.

The thing that makes Eigenius unusual is this: **a single IRI is simultaneously a resource you can query and a type you can ascribe to**. `urn:eigenius:example:Document` is a `Class` resource — you can `MATCH` it via [EigenQL](../eigenql/README.md), inspect its `requires` list, attach instances to it. It is also a *type* — `let d : ex:Document = ...` is well-formed, and the kernel type-checker will verify that the value has the right shape. There is no separate "type system" living parallel to the resource graph; the type-checker reaches into the layer at check time and reads what it needs.

This chapter explains how that bridge works mechanically and why it shapes everything else.

## 6.1. Two worlds, one IRI

The two worlds are:

**The resource graph.** Eigon resources stored in immutable layers. Every `Class`, `Property`, `InductiveType`, `CodataType` declared in ESL becomes a resource here. Resources have IRIs, properties, and `is_a` lists. They're queryable through EigenQL, persistable through the storage layer, traceable through trace stores.

**The type theory.** A EigenTT kernel ([D19 §3](../../design/d19-inductive-types.md)) with Π-types, Σ-types, inductive types, coinductive types, sized types, identity types, universes, and normalisation-by-evaluation. The kernel doesn't know about resources directly — it works with `Exp` (terms) and `Val` (values).

The two worlds talk through one specific kernel construct: **`Exp::EigonClass(iri)`**. When the type-checker encounters an `EigonClass(iri)`, it doesn't try to interpret it abstractly — it calls into the layer, finds the resource at that IRI, and constructs a `Val` from what it finds. That call lives in [`resolve_class_type`](../../../kernel/src/program/ground.rs).

## 6.2. The bridge: ontology-as-types resolution

The mechanism is specified in [D18 (Ontology-as-Types Resolution)](../../design/d18-ontology-as-types-resolution.md). The implementation is in [`kernel/src/program/ground.rs`](../../../kernel/src/program/ground.rs). Three functions do the work:

```rust
pub fn resolve_class_type(class_iri: &Iri, layer: &Layer) -> Result<Val, String>;

pub fn resolve_property_type(prop_iri: &Iri, layer: &Layer) -> Result<Val, String>;

fn collect_properties(class_iri: &Iri, layer: &Layer)
    -> Result<(BTreeSet<Iri>, BTreeSet<Iri>), String>;
```

`resolve_class_type` is the entry point. Given a class IRI:

1. **Primitive shortcut.** If the IRI is one of `core:string`, `core:integer`, `core:float`, `core:boolean`, `core:json`, return the corresponding `Val::EigonPrimitive`.
2. **Codata shortcut.** If the resource at the IRI has `is_a: [CodataType]`, route to `resolve_codata_type` and build a `Val::CodataType`.
3. **Inductive shortcut.** If the resource has `is_a: [InductiveType]`, route to `resolve_inductive_type` and build a `Val::InductiveType`.
4. **Class — collect properties.** Otherwise, walk `requires` and `recommends` (transitively through `subclass_of`) via `collect_properties`. For each required property, resolve its type via `resolve_property_type`. Wrap recommended-property types in `Option`. Build a `Val::Sigma` chain from the resulting `(prop_iri, type)` pairs.

`resolve_property_type` reads a property's `data_type` and turns it into a `Val`. Primitives become `EigonPrimitive`. Resource references become `EigonClass(class_iri)` (the type of "any resource of class `class_iri`") if `class_types` is set, or `Val::Set` (the universe of types) if untyped. Arrays wrap the element type in a `List` type.

These functions get called *during type-check and during evaluation*, not ahead of time. The type-checker doesn't pre-load the ontology; it reads from the layer on demand.

## 6.3. The mappings

Concrete declaration → resource shape → kernel value:

| ESL declaration | Resource shape | Kernel `Val` |
|---|---|---|
| `class C { p : T, q : U }` | `Class` resource with `requires: [p_iri, q_iri]` | `Sigma((p_iri, T_val), Sigma((q_iri, U_val), One))` |
| `class C { recommends p : T }` | `Class` resource with `recommends: [p_iri]` | `Sigma((p_iri, Option(T_val)), One)` |
| `class C : Parent { ... }` | `Class` with `subclass_of: [Parent]` | Σ-type accumulated through `Parent`'s required/recommended properties |
| `property p : core:string` | `Property` with `data_type: core:string` | `Val::EigonPrimitive(String)` |
| `property p : ex:Other_Class` | `Property` with `data_type: core:resource`, `class_types: [Other_Class]` | `Val::EigonClass(Other_Class)` |
| `property p : core:value_array { element_type = core:integer }` | `Property` with `data_type: core:value_array` | `Val::List(Integer)` |
| `data D(params) { ctor(args) }` | `InductiveType` + `InductiveCtor` resources | `Val::InductiveType { decl, params: [] }` |
| `codata D(params) { obs : T }` | `CodataType` resource with `observations` array | `Val::CodataType { decl, params: [] }` |
| `resource r : C { p = v }` | Resource with `is_a: [C]` and properties | A value inhabiting `Val::EigonClass(C)` (the corresponding Σ-tuple) |
| `program P : I -> O = body` | Resource of class `Program` carrying body | A `Val::Lam(closure)` of type `Pi(_ : I_val, O_val)` |

The takeaway: **every ESL declaration produces a resource shape that the kernel can decode into a type**. The decoder is `ground.rs`. The types it produces are first-class kernel values that the type-checker can compare, normalise, unify, and use to drive elaboration.

## 6.4. When the kernel calls back into the layer

Layer access happens at three moments:

1. **Type-check** ([`kernel/src/nbe/check.rs`](../../../kernel/src/nbe/check.rs)). When `check_infer` encounters an `Exp::EigonClass(iri)`, it calls `resolve_class_type(iri, layer)`. The returned `Val` becomes the inferred type. Any check rule that compares an expected type against an `EigonClass`-mentioned type forces this resolution.

2. **Evaluation** ([`kernel/src/nbe/eval.rs`](../../../kernel/src/nbe/eval.rs)). The `EigonClass` arm of `eval` does the same lookup. Reduction can require the layer too — projections on resource values reach into the layer to fetch the underlying resource's properties.

3. **Constraint firing** ([`kernel/src/nbe/check.rs`](../../../kernel/src/nbe/check.rs) `NativeDecide` arm). When a property's declared constraint (e.g., `min_value`, regex pattern, or institution-decided predicate) needs to fire during type-check, the kernel reaches through the property IRI to the layer to find the constraint, then dispatches.

Two consequences:

- **Layer is the module system.** Changing the layer chain changes which classes and properties exist; same source can succeed in one layer and fail in another. There's no separate import or module mechanism — the layer is what's "in scope".
- **`Read` capability is required.** Any kernel operation that touches `EigonClass` needs at least `Read` capability (layer access). Pure-mode evaluation can normalise expressions that don't reference the layer, but the moment an `EigonClass` shows up, the kernel needs `Read`. See [chapter 8](08-capability-modes.md).

## 6.5. Why this matters in practice

Three concrete consequences of the bridge that you'll keep encountering:

### 6.5.1. No module imports — the layer is the module system

```esl
// file 1
namespace ex = "urn:eigenius:example";
class ex:Document { requires ex:text; }

// file 2 (compiled separately, loaded into a layer that already has file 1)
namespace ex = "urn:eigenius:example";
program ex:summarize : ex:Document -> ex:Document {
    Construct ex:Document { ex:text = "..." }
}
```

File 2 references `ex:Document` without importing file 1. The two files share a namespace alias declaration (each file declares its own aliases), and `ex:Document` resolves through whatever layer the two files are loaded into. If file 1 isn't in the layer, file 2's type-check fails with `class 'urn:eigenius:example:Document' not found in layer chain`. There's nothing else.

### 6.5.2. Schema additions become available immediately

If you add a new property to a class and load the updated ontology into the layer, every program that constructs instances of that class can now use the new property. There's no recompile step at the kernel level — the next time `resolve_class_type` is called, it sees the updated `requires`/`recommends` lists and produces the new Σ-type.

This is one of the things that makes the layered immutable-resource model attractive: schema evolution is layer evolution.

### 6.5.3. Constraints attached to properties fire at the use site

A property declared with `min_value = 0` or `pattern = "^[a-z]+$"` carries the constraint as a kernel-recognised structure. Wherever that property's value is computed in a program body, the type-checker (in `Check` capability mode) verifies the constraint by calling the appropriate decide procedure.

For institution-registered decide procedures (Phase 11c — see [chapter 9](09-institutions.md)), this means a program that produces a value satisfying a registered domain predicate is *automatically verified* against that predicate at type-check time, with no explicit `assert` or call needed in the program text.

## 6.6. Concrete example: `class` to `Val`

Take this ESL:

```esl
namespace core = "urn:eigenius:core";
namespace ex   = "urn:eigenius:example";

property ex:name  : core:string {}
property ex:age   : core:integer { min_value = 0; max_value = 150; }
property ex:email : core:string { pattern = "^[^@]+@[^@]+$"; }

class ex:Person {
    requires ex:name, ex:age;
    recommends ex:email;
}
```

When the kernel resolves `ex:Person` as a type:

1. `resolve_class_type("urn:eigenius:example:Person", layer)` finds the resource, sees no inductive/codata `is_a`, falls through to the class path.
2. `collect_properties` walks `requires` (`[ex:name, ex:age]`) and `recommends` (`[ex:email]`).
3. For each required property, `resolve_property_type` is called:
   - `ex:name` → `Val::EigonPrimitive(String)`
   - `ex:age` → `Val::EigonPrimitive(Integer)` (constraints are kept on the property resource for later firing)
4. For `ex:email` (recommended): `Val::Sum((some, EigonPrimitive(String)), (none, One))` — the option-wrapped form.
5. `build_sigma_chain` produces:

```
Sigma((ex:name,  String),
   Sigma((ex:age,   Integer),
      Sigma((ex:email, Option(String)),
         One)))
```

This is the type that `let p : ex:Person = ...` will be checked against. A `Construct ex:Person { ex:name = "Alice", ex:age = 30 }` produces the corresponding Σ-tuple value. A projection `p.ex:age` is type-checked by walking the Σ-chain to find the `ex:age` field; the kernel verifies that the value satisfies `min_value = 0` and `max_value = 150` before allowing it to flow further.

## 6.7. The `EigonClass` escape hatch

`Exp::EigonClass(iri)` is the only kernel form that carries a layer-relative reference. Every other kernel form is fully self-contained. This is intentional — it confines the "needs the layer" boundary to one syntactic point, which is what makes it possible to reason about which capability mode is required for any given expression.

When you see `EigonClass(iri)` in a debug print or error message, read it as: "the type the kernel will look up at IRI `iri` when it has access to the layer". Until that lookup happens, it's an opaque token.

The `EigonClass` form isn't something you write directly in ESL. The compiler emits it whenever a qualified name in a type position resolves to a class IRI (for example, `let p : ex:Person = ...` compiles to a `Let` whose type field is the IRI; the kernel reads that as `EigonClass(...)`).

## 6.8. Constraints, decide procedures, and the check pass

The general rule for constraint firing during type-check:

- A property carries zero or more `Constraint` values.
- When a value flows through a position typed by that property, the type-checker iterates the constraints and dispatches each one to its appropriate decide procedure.
- Built-in constraints (`min_value`, `max_length`, regex patterns) have built-in decide procedures.
- **Institution-registered Decidable QueryClasses** ([chapter 9](09-institutions.md)) are called the same way — the kernel sees a `Constraint::Institution { iri, args }`, resolves the QueryClass in the [`InstitutionIndex`](../../../kernel/src/institution/registry.rs), and dispatches `Institution::query(query_handler, synthetic_input, ctx)` on the registered runtime.

The capability mode required is **`Check`** — `Read` plus institution-index + institution-runtime access. In `Pure` mode, constraints can't fire (no layer); in `Read`, they can fire only for the built-in ones; in `Check`, the institution-registered ones fire too.

This chapter has been the *what* and *why* of the bridge. The capability-mode chapter ([chapter 8](08-capability-modes.md)) covers the *when* in operational terms.

---

Next: **[7. Type theory primer →](07-type-theory-primer.md)**
