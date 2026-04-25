# 9. Institutions in ESL

An **institution** is a domain-specific reasoning module registered with the kernel. Institutions implement the [`FiberReasoner`](../../../kernel/src/institution/mod.rs) trait and declare three kinds of capability that ESL programs can invoke:

1. **Decide predicates** — boolean-valued domain facts evaluated by the institution at check-time or runtime.
2. **Comorphisms** — typed translations from one institution's resources to another's.
3. **Fiber queries** — structured query/response interactions, dispatched via [EigenQL `FIBER` clauses](../eigenql/07-fiber-clauses.md). ESL doesn't invoke these directly; they're a query-language concern.

This chapter focuses on what's visible from an ESL program author's perspective: how to invoke decide predicates and comorphisms, what classification means at compile time, and the life-science motivating example that drove the design.

For the full institution-side interface (implementing a new institution), see [`kernel/src/institution/mod.rs`](../../../kernel/src/institution/mod.rs) and [D10](../../design/d10-grothendieck-institution-protocol.md).

## 9.1. What an institution declares

When an institution registers, it provides a [`FiberDeclaration`](../../../kernel/src/institution/mod.rs):

```rust
pub struct FiberDeclaration {
    pub institution_iri: Iri,
    pub name: String,
    pub morphism_types: Vec<Resource>,
    pub query_types: Vec<Resource>,
    pub structural_properties: Vec<Resource>,
    pub comorphism_types: Vec<Resource>,      // Phase 11d
    pub decide_procedures: Vec<Iri>,          // Phase 11e
}
```

The two fields ESL cares about are:

- **`comorphism_types`** — IRIs of `Comorphism`-class resources this institution declares. Each is invokable as a translation from a source resource to a target.
- **`decide_procedures`** — IRIs of decide predicates this institution answers. Each is invokable as a boolean-valued function on positional arguments.

When the institution registers via [`InstitutionRegistry::register`](../../../kernel/src/institution/mod.rs), the registry builds two dispatch tables:

- `comorphism_dispatch: BTreeMap<Iri, Iri>` — comorphism IRI → declaring institution IRI
- `decide_dispatch: BTreeMap<Iri, Iri>` — procedure IRI → declaring institution IRI

These tables drive the classification step described next.

## 9.2. Classification at compile time

When the ESL compiler encounters a function-call expression with a qualified name, it asks the institution registry to classify the IRI:

```rust
pub fn classify(&self, iri: &Iri) -> Option<InstitutionCapability>;

pub enum InstitutionCapability {
    DecidePredicate,
    Comorphism,
}
```

Three outcomes:

- **`Some(DecidePredicate)`** — emit `Exp::NativeDecide(Constraint::Institution { iri, args }, Unit)`. The compile-time form is `program:DecideApply` carrying the IRI and the positional arg list.
- **`Some(Comorphism)`** — emit `Exp::InstitutionInvoke { iri, source }`. The compile-time form is `program:ComorphismInvokeApply` carrying the IRI and a single source resource. **Comorphisms take exactly one positional arg**; mismatches surface as compile errors.
- **`None`** — fall through to component dispatch. If no component is registered either, the call fails at evaluate time with `unknown function`.

The classifier is consulted only by `compile_with_institutions`. The plain `compile` entry point doesn't classify — it always falls through to component dispatch. For programs that use institution-dispatched calls, you must use:

```rust
let resources = esl::compile_with_institutions(source, registry)?;
```

This is the same single-source-of-truth classification used by EigenQL ([EigenQL chapter 8](../eigenql/08-institutions.md)) — both surface languages share `InstitutionRegistry::classify` to ensure they never disagree about which IRIs are decide predicates vs. comorphisms.

## 9.3. Invoking a decide predicate

```esl
namespace cap = "urn:eigenius:test";

program ex:check_program : ex:Thing -> ex:Thing {
    cap:within_tolerance(input.ex:delta, 0.1)
}
```

The qualified name `cap:within_tolerance` resolves to `urn:eigenius:test:within_tolerance`. If the institution registry classifies this as a `DecidePredicate`, the compiled body becomes:

```
Lam(input,
    NativeDecide(
        Constraint::Institution {
            iri: "urn:eigenius:test:within_tolerance",
            args: [input.ex:delta, 0.1]
        },
        Unit
    ))
```

**At type-check (Check mode):** the kernel calls `reasoner.decide(iri, args, ctx)`. The three-valued return:

| `DecResult` | Kernel behaviour |
|---|---|
| `Holds` | Reduces the surrounding `NativeDecide` to `Refl(value)` — predicate accepted, no further work |
| `Fails` | Emits a failing neutral — the type-check rejects with the institution's failure reason |
| `Undecidable` | Stays neutral — the predicate is left for runtime |

**At runtime (IO mode):** same dispatch path. An `Undecidable` from check time becomes a runtime call with the same outcome semantics.

**No compile-time arity check.** The kernel does not enforce a specific number of arguments to decide predicates — institutions validate args themselves. This is intentional: a decide predicate may legitimately take 1, 2, or N arguments, and the institution is the authority on what's well-typed.

**Default behaviour.** Institutions that don't override `decide` return `DecResult::Undecidable` for every call. This makes "I declared a decide procedure but didn't implement it" cleanly recoverable — programs that use the predicate just stay neutral instead of crashing.

## 9.4. Invoking a comorphism

```esl
namespace dock  = "urn:eigenius:institutions:docking";

program ex:translate : ex:DockingResult -> ex:AssayPrediction {
    dock:dock_to_assay(input)
}
```

The qualified name `dock:dock_to_assay` resolves to `urn:eigenius:institutions:docking:dock_to_assay`. If classified as `Comorphism`, the compiled body becomes:

```
Lam(input,
    InstitutionInvoke {
        iri: "urn:eigenius:institutions:docking:dock_to_assay",
        source: input
    })
```

**Arity rule.** Exactly one positional argument — the source resource. The compiler enforces this:

```
comorphism `urn:...:dock_to_assay` expects exactly 1 source argument, got 2 positional arg(s)
```

A trailing `{ ... }` config block on a comorphism call is also rejected.

**At evaluate time (Check or IO):** the kernel resolves the institution via `registry.institution_for_comorphism(iri)` and calls `reasoner.translate(iri, source, ctx)`. The returned `Resource` is wrapped in `Val::Embedded(Box::new(resource))`. Subsequent kernel operations can pattern-match or project on this embedded resource.

**Default behaviour.** Institutions that declare `comorphism_types` but don't override `translate` get a runtime error: `"institution does not implement 'translate' for comorphism 'X'"`. Unlike `decide` (which silently returns `Undecidable`), an unimplemented `translate` is a programmer error — comorphisms are invoked because their result is needed.

## 9.5. Constraints attached to properties

Institutions can register decide procedures that are wired to property constraints. The wiring lives at the property declaration:

```esl
property ex:rmsd : core:float {
    description = "Root-mean-square deviation in angstroms";
    min_value = 0.0;
    // Built-in constraints fire in Read mode; institution-registered
    // constraints would be declared via the property's metadata in
    // the underlying resource graph (Phase 11c surface for this is
    // still informal — see compile.rs PropertyItem variants).
}
```

When a value flows through the position of a property carrying institution-decided constraints, the kernel iterates the constraints during type-check (Check mode) and dispatches each through the institution registry. A `Holds` accepts the value; a `Fails` rejects the type-check with the institution's diagnostic; an `Undecidable` defers the constraint to runtime.

This is the mechanism that makes domain reasoning *automatic* — the program author writes a normal program, and the kernel verifies that values flowing through property positions satisfy whatever the institution registered for that property. There's no `assert` in the program text.

## 9.6. Comparison with components

ESL has two distinct ways to invoke external work: components and institutions. They look syntactically similar but differ along several axes:

| Dimension | Component | Decide predicate | Comorphism |
|---|---|---|---|
| Surface syntax | `Component(input)` | `cap:predicate(args)` | `cap:translate(source)` |
| Identification | bare or namespaced name → component IRI | qualified name → decide IRI | qualified name → comorphism IRI |
| Registration | `ComponentRegistry` | `InstitutionRegistry::register`'s `decide_procedures` | `InstitutionRegistry::register`'s `comorphism_types` |
| Arity | 1 positional + optional config block | any | exactly 1 |
| Capability mode required | IO | Check (or IO) | Check (or IO) |
| Result shape | arbitrary (component-defined output type) | boolean | embedded resource |
| Side-effecting | yes (typically) | no (pure decision) | no (pure translation) |

Components do *work* — make API calls, run model inference, transform data — and they require IO. Decide predicates and comorphisms are *queries on the institution* — they don't have side effects, they require only the institution registry, and they fire at type-check time when possible.

## 9.7. Worked example: docking + assay

Two institutions:

- `docking` decides whether two molecular poses are within an RMSD tolerance, and defines a comorphism `dock_to_assay` translating docking results into assay-domain predictions.
- `assay` accepts the translated input.

ESL program:

```esl
namespace core = "urn:eigenius:core";
namespace ex   = "urn:eigenius:example";
namespace dock = "urn:eigenius:institutions:docking";

class ex:DockingResult {
    requires ex:compound, ex:pose, ex:delta_g;
}

property ex:compound : core:resource {
    class_types = ex:Compound;
}

property ex:pose : core:resource {
    class_types = ex:Pose;
}

property ex:delta_g : core:float {}

class ex:AssayPrediction {
    requires ex:compound, ex:predicted_ic50;
}

program ex:predict : ex:DockingResult -> ex:AssayPrediction {
    dock:dock_to_assay(input)
}

program ex:filter_acceptable : ex:DockingResult -> ex:DockingResult {
    let ok : core:boolean = dock:within_tolerance(input.ex:delta_g, 2.0);
    input
}
```

The first program invokes a comorphism — translates a docking result into an assay prediction at runtime. The second invokes a decide predicate — checks at check time whether the `delta_g` is within the docking institution's tolerance. The decide call's boolean result is bound to `ok` and then ignored; in a real program you'd branch on it.

To compile this:

```rust
let mut registry = InstitutionRegistry::new();
registry.register(Box::new(DockingInstitution::new()))?;
registry.register(Box::new(AssayInstitution::new()))?;

let registry_arc = Arc::new(registry);
let resources = esl::compile_with_institutions(source, registry_arc.clone())?;
```

The compiled program references the institution capabilities. To run:

```rust
let runtime_ctx = EvalCtx::IO {
    layer,
    registry: component_registry,
    institutions: registry_arc,
    trace_store,
    dispatched_traces: Default::default(),
    task_context: None,
};
```

See [`life-science requirements §16.3`](../../design/life-science-requirements.md) for the original motivating discussion.

## 9.8. The classification table (cross-language)

The same classification mechanism powers ESL and EigenQL. When the same IRI appears in both languages, it dispatches identically:

| IRI resolves to | ESL emits | EigenQL emits | Runtime call |
|---|---|---|---|
| Registered comorphism | `Exp::InstitutionInvoke { iri, source }` | `dispatch_institution_call(Comorphism, ..)` | `reasoner.translate` |
| Registered decide procedure | `Exp::NativeDecide(Constraint::Institution{..}, Unit)` | `dispatch_institution_call(DecidePredicate, ..)` | `reasoner.decide` |
| FIBER query class | n/a | `FIBER` clause → `apply_fiber_clause` | `reasoner.query` |
| Class / primitive / literal | `Exp::EigonClass(iri)` etc. | resolved via layer | various |

Cross-link: [EigenQL chapter 8](../eigenql/08-institutions.md) covers the same table from the query-language side.

## 9.9. Where the implementation lives

| File | Purpose |
|---|---|
| [`kernel/src/institution/mod.rs`](../../../kernel/src/institution/mod.rs) | `FiberReasoner` trait, `InstitutionRegistry`, `DecResult`, `InstitutionCapability` |
| [`kernel/src/institution/error.rs`](../../../kernel/src/institution/error.rs) | `InstitutionError`, `MorphismValidation` |
| [`kernel/src/esl/compile.rs`](../../../kernel/src/esl/compile.rs) | Phase 11e compile-time classification of `Apply` expressions |
| [`kernel/src/nbe/check.rs`](../../../kernel/src/nbe/check.rs) | `NativeDecide` check arm — fires decide procedures at type-check time |
| [`kernel/src/nbe/eval.rs`](../../../kernel/src/nbe/eval.rs) | `decide_constraint` and `InstitutionInvoke` evaluation |

---

Next: **[10. Error messages →](10-error-messages.md)**
