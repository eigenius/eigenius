# 8. Capability modes

The kernel evaluator runs in one of two **capability modes**, the two variants of the [`EvalCtx`](../../../kernel/src/nbe/eval/mod.rs) enum:

```rust
pub enum EvalCtx {
    /// Standard NbE: normalize terms, check types. No side effects.
    Pure,
    /// Effectful evaluation: institution dispatch / IO component
    /// invocation delegated to `hooks`.
    Effectful {
        layer: Option<Arc<Layer>>,
        hooks: Arc<dyn EffectHooks>,
    },
}
```

`Pure` is standard normalisation-by-evaluation with no external access. `Effectful` carries an [`EffectHooks`](../../../kernel/src/nbe/eval/hooks.rs) implementation that exactly three expression forms delegate to; everything else in the evaluator behaves identically in both variants.

**The finer capability tiers are a property of the hooks implementation, not of the enum.** The kernel ships one implementation, [`InstitutionEngine`](../../../kernel/src/institution/eval_hooks.rs), with two constructors:

| Constructor | Layer | Institution index + runtime | Component registry | Trace store | Task context |
|---|---|---|---|---|---|
| `InstitutionEngine::for_check` | optional | yes | **no** | no | no |
| `InstitutionEngine::for_io` | yes | yes | yes | optional | optional |

A "check-tier" engine can fire institution-decided constraints but cannot dispatch components; an "IO-tier" engine can do both. That is the whole of the distinction the older `Pure` / `Read` / `Check` / `IO` four-mode vocabulary used to carry, and those four variants no longer exist in the code.

## 8.1. The three effectful forms

Only these three delegate to the hooks:

| Form | Hook | Pure | Check-tier `Effectful` | IO-tier `Effectful` |
|---|---|---|---|---|
| `App` whose callee is a registered component IRI | `is_component` + `dispatch_component` | neutral | neutral (`is_component` is false — no registry) | **dispatch**, producing a `ComponentTrace` |
| `NativeDecide(Constraint::Institution { … }, val)` | `decide_institution` | neutral | **dispatch** via `Institution::query`, reduce by Verdict | dispatch |
| `InstitutionInvoke { comorphism_iri, source }` | `institution_invoke` | neutral | **error** — the four-step pipeline needs the transformation Component, so a check-tier engine returns `InstitutionInvoke requires IO mode` | **four-step pipeline** (extract_typed → transformation Component → reify) |

"Neutral" means the form stays in the value as a stuck term. The evaluator returns successfully — it just doesn't reduce that form, and a Σ-tuple containing a neutral is still a Σ-tuple.

Every other form — β-redexes, ι-redexes (`match` on a constructor), projections of built pairs, `map` / `reduce` over known lists, `EigonPrimitive` arithmetic, `NativeDecide` on a *structural* constraint (`MinValue`, `Pattern`, …), corecords and observations — reduces the same way under both variants. In particular:

- `Exp::EigonClass(iri)` evaluates to `Val::EigonClass(iri)` in **both** modes. The evaluator never resolves a class IRI against the layer. Class resolution happens elsewhere: at resource-to-`Exp` parse time in [`program/expr.rs`](../../../kernel/src/program/expr.rs) and at check time through [`program/check_hooks.rs`](../../../kernel/src/program/check_hooks.rs), both calling [`resolve_class_type`](../../../kernel/src/program/ground.rs). See [chapter 6](06-resources-types-and-the-layer.md).
- The `layer` an `Effectful` context carries is read by the hooks (and by `EvalCtx::layer()` consumers), not by the evaluator's structural arms.

## 8.2. Which API gives you which mode

You rarely construct an `EvalCtx` directly:

- **`esl::compile`** runs no kernel at all — it emits resources.
- **Type-checking** goes through `CheckCtx::eval_ctx()`, which returns a check-tier `Effectful` context when an institution index *and* runtime are attached, and **`EvalCtx::Pure` otherwise**. There is no intermediate layer-only mode: a type-check with a layer but no institution registry evaluates Pure.
- **Running a program** goes through [`program::eval_io`](../../../kernel/src/program/eval_io.rs), which builds `InstitutionEngine::for_io` and calls `EvalCtx::effectful(Some(layer), engine)`.
- **Tests** use `EvalCtx::Pure` (or `EvalCtx::pure()`) for pure-term normalisation.

The constructors are `EvalCtx::Pure` / `EvalCtx::pure()` and `EvalCtx::effectful(layer, hooks)`.

## 8.3. Why the split exists

Two reasons:

1. **Type-checking should not have side effects.** A check-tier engine can call decide procedures, but it cannot dispatch components or write traces. Type-checking is reproducible; running the program is not.
2. **Pure normalisation is a useful subset.** Equality checks, definitional unfolding, and β/ι reduction need nothing else, and Pure is what the kernel falls back to when no registry is available.

The consequence to rely on: nothing you did not authorise happens silently. A check that needs to fire an institution decide procedure but is given a `Pure` context leaves the predicate stuck (neutral) and fails with a "couldn't decide" message rather than passing quietly.

Cross-references:

- [Chapter 6](06-resources-types-and-the-layer.md) explains the class-resolution mechanism.
- [Chapter 9](09-institutions.md) covers the institution-dispatched operations.
- [`kernel/src/nbe/eval/mod.rs`](../../../kernel/src/nbe/eval/mod.rs) `eval_ctx` / `eval_impl` is the evaluator; [`kernel/src/nbe/eval/hooks.rs`](../../../kernel/src/nbe/eval/hooks.rs) is the `EffectHooks` trait; [`kernel/src/institution/eval_hooks.rs`](../../../kernel/src/institution/eval_hooks.rs) is its only implementation.

---

Next: **[9. Institutions in ESL →](09-institutions.md)**
