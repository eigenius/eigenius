# 8. Capability modes

The kernel evaluator runs in one of four **capability modes**, declared as the [`EvalCtx`](../../../kernel/src/nbe/eval.rs) enum:

```rust
pub enum EvalCtx {
    Pure,
    Read { layer: Arc<Layer> },
    Check {
        layer: Option<Arc<Layer>>,
        institution_index: Option<Arc<InstitutionIndex>>,
        institution_runtime: Option<Arc<InstitutionRuntime>>,
    },
    IO {
        layer, registry,
        institution_index, institution_runtime,
        trace_store, dispatched_traces, task_context,
    },
}
```

The mode controls **which kernel operations can produce values vs. stay neutral**. Operations that need resources beyond pure normalisation — layer lookups, component dispatch, Decidable QueryClass dispatch — require explicit capability. If the operation appears in code that runs in a weaker mode, the term stays neutral and the kernel reads it back without reducing.

This chapter explains what each mode permits and which kernel forms behave differently across modes.

## 8.1. The four modes at a glance

| Mode | Layer | Institutions | Components | Used by |
|---|---|---|---|---|
| **Pure** | — | — | — | Type-checker default; any closed-term normalisation |
| **Read** | yes | — | — | Type-checker when `EigonClass` is in scope |
| **Check** | optional | yes | — | Type-checker when institution-decide constraints fire |
| **IO** | yes | yes | yes | Runtime program execution |

Read top to bottom: each mode adds capabilities to the previous. Pure is the base — every kernel form that's purely syntactic (lambdas, lets, pairs, projections of built pairs, etc.) reduces in Pure. The other modes layer additional reduction power on top.

## 8.2. `Pure`

Pure mode performs standard normalisation-by-evaluation with no external access. The evaluator can:

- Reduce β-redexes (`(λx. body)(arg)` → `body[x ↦ arg]`).
- Reduce ι-redexes (pattern matching that hits a constructor: `match (succ n) { zero -> ...; succ(m) -> body } → body[m ↦ n]`).
- Project Σ-tuples that are already pairs (`(a, b).1 → a`).
- Apply `map` and `reduce` to fully-known finite lists.
- Evaluate built-in arithmetic, string operations, and the like on literal arguments.

The evaluator **cannot**:

- Resolve `Exp::EigonClass(iri)` — there's no layer to look up the IRI in. The `EigonClass` stays as a neutral value.
- Dispatch to components — there's no registry. `Exp::App` on a component IRI stays neutral.
- Fire institution decide procedures — there's no registry. `Exp::NativeDecide(Constraint::Institution{..}, _)` stays neutral.
- Read property values from layer resources — same reason.

**When Pure is enough.** Any expression that's a closed term over kernel primitives (literals, lambdas, pairs, constructors of fully-known inductive types) reduces in Pure to a fully-evaluated value. Most type-theory examples in [chapter 7](07-type-theory-primer.md) fall in this category.

**When Pure isn't.** As soon as your expression mentions an `EigonClass`, dispatches a component, or uses a constraint backed by an institution decide procedure, you need a stronger mode.

The constructor: `EvalCtx::pure()` or just `EvalCtx::Pure`.

## 8.3. `Read`

Read mode is Pure plus an `Arc<Layer>`. The evaluator can additionally:

- **Resolve `EigonClass` references.** When evaluation encounters `Exp::EigonClass(iri)`, the kernel calls [`resolve_class_type(iri, layer)`](../../../kernel/src/program/ground.rs) and substitutes the resulting `Val`.
- **Project resource values.** A projection `r.prop_iri` on a value bound to a layer resource fetches the property's value from the resource through the layer.
- **Resolve recursive type references** — codata self-references and inductive parameters look up the declaration through the layer.

What Read still **can't** do:

- Dispatch components — no component registry.
- Fire institution-registered decide procedures — no institution registry.

**When Read is enough.** Most type-checking. The type-checker runs in Read by default whenever the term mentions classes, properties, or any resource-derived type. Built-in property constraints (regex, `min_value`, etc.) fire here because they're built into the kernel.

**When Read isn't.** Type-checking that needs to fire an institution-registered constraint, or runtime program execution.

The constructor: `EvalCtx::Read { layer }`.

## 8.4. `Check`

Check mode is Read plus institution dispatch — an optional `Arc<InstitutionIndex>` (chain-derived classifier) and `Arc<InstitutionRuntime>` (`Box<dyn Institution>` keyed by IRI). The layer is also optional in Check (some checks can run without one). The evaluator can additionally:

- **Fire Decidable QueryClass dispatch.** When evaluation encounters `Exp::NativeDecide(Constraint::Institution { iri, args }, _)`, the kernel resolves the QueryClass in the index, marshals `args` into a synthetic input resource, and calls `Institution::query(query_handler, input, ctx)` on the registered runtime. The Verdict's `ctor_name` selects the reduction: `Holds` → `Refl(v)`, `Fails` → failing neutral, `Undecidable` → passthrough neutral (D14 §9.2).
- **Run the four-step comorphism pipeline.** When the kernel evaluator encounters `Exp::InstitutionInvoke { comorphism_iri, source }` (typically synthesised by EigenQL FIBER param coercion, not user-written), it dispatches `extract_typed → transformation Component → reify` through the relevant institutions and the component registry (D14 §9.3).

What Check still **can't** do:

- Dispatch IO components — no component registry, no trace store.
- Run programs that rely on IO components.

**When Check is the right mode.** Type-checking programs that use institution-dispatched constraints. The type-checker switches to Check mode when it needs to verify that a value satisfies a Decidable predicate, or to evaluate a comorphism call to know its result type.

The constructor: `EvalCtx::Check { layer, institution_index, institution_runtime }`.

## 8.5. `IO`

IO mode adds component dispatch, trace stores, and (optionally) a task context. This is the runtime mode — the one used when actually running a program against real inputs.

In addition to everything Check can do, IO can:

- **Dispatch components.** `Exp::App(component_iri, arg)` invokes the registered handler for that component IRI through the `ComponentRegistry`. Component dispatch is the only way for a program to do meaningful side effects.
- **Produce traces.** Each component dispatch contributes a `ComponentTrace` to `dispatched_traces`. After evaluation, traces are committed to the trace store (if one is configured).
- **Resume from traces.** When a `task_context` is supplied (D21 §3.2), the dispatcher consults per-task positional trace keys to recover prior dispatch results — this is how resumable execution works.

The constructor: `EvalCtx::IO { layer, registry, institution_index, institution_runtime, trace_store, dispatched_traces, task_context }`.

## 8.6. Per-form behaviour by mode

A summary of which kernel `Exp` forms behave differently across modes:

| Form | Pure | Read | Check | IO |
|---|---|---|---|---|
| `Lam`, `Pi`, `Sig` | reduce | reduce | reduce | reduce |
| `App` (β-redex on lambda) | reduce | reduce | reduce | reduce |
| `App` (component IRI) | neutral | neutral | neutral | **dispatch** |
| `Match`/`InductiveRec` over constructor | reduce | reduce | reduce | reduce |
| `Project` on pair | reduce | reduce | reduce | reduce |
| `Project` on resource ref | neutral | **fetch from layer** | fetch from layer | fetch from layer |
| `EigonClass(iri)` | neutral | **resolve via `resolve_class_type`** | resolve | resolve |
| `EigonPrimitive` | reduce | reduce | reduce | reduce |
| `NativeDecide(Constraint::Builtin{..}, val)` | reduce | reduce | reduce | reduce |
| `NativeDecide(Constraint::Institution{..}, val)` | neutral | neutral | **dispatch via `Institution::query` → reduce by Verdict** | dispatch |
| `InstitutionInvoke { comorphism_iri, source }` | neutral | neutral | **four-step pipeline (extract → transform → reify)** | pipeline |
| `Map`/`Reduce` over known list | reduce | reduce | reduce | reduce |
| `CoRecord` / `Observe` (productive) | reduce | reduce | reduce | reduce |

"Neutral" means the form remains in the value as a stuck term. The evaluator returns successfully — it just doesn't reduce that form. Subsequent operations that don't depend on the neutral can still proceed (e.g., a Σ-tuple containing a neutral is still a Σ-tuple; you can project the other field).

## 8.7. Choosing a mode in practice

You usually don't pick a mode directly — the surrounding API does it for you:

- **`esl::compile`** doesn't run any kernel — it just emits resources.
- **Type-checking** typically uses Read or Check, depending on whether the program references institution-decided constraints. The check pipeline picks the appropriate mode.
- **Running a program** uses IO. Construct `EvalCtx::IO { ... }` with your component registry, institution registry, and (optionally) a task context.
- **Tests / sanity checks** often use Pure for pure-term normalisation and Read for layer-dependent checks.

The capability hierarchy ensures that nothing you wouldn't expect happens silently — a check that needs to fire an institution decide procedure but is given only a Read context will leave the predicate stuck (neutral), and the check will fail with a clear "couldn't decide" message rather than silently passing or invoking something the caller didn't authorise.

## 8.8. Why capability modes exist

Two reasons:

1. **Type-checking should not have side effects.** The type-checker can call decide procedures (Check), but it can't dispatch components or write traces (IO). Type-checking is reproducible and incremental; running the program is not.
2. **Pure normalisation is a useful subset.** Many kernel operations — equality checks, definitional unfolding, beta/iota reduction — only need the pure subset. Pure is what the kernel falls back to when no other context is available, and it's enough for most internal kernel work.

Capability modes are thus a typing discipline on the *evaluator itself*. Each `Val` can be reasoned about as "what level of capability was needed to produce it"; the type-checker uses this to decide what to require of its callers.

Cross-references:

- [Chapter 6](06-resources-types-and-the-layer.md) explains the `EigonClass` resolution mechanism that Read enables.
- [Chapter 9](09-institutions.md) covers the institution-dispatched operations that Check and IO enable.
- [`kernel/src/nbe/eval.rs`](../../../kernel/src/nbe/eval.rs) `eval_ctx` is the implementation.

---

Next: **[9. Institutions in ESL →](09-institutions.md)**
