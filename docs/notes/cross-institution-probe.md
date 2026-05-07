# Cross-institution probe: one chain payload, two institutions, no glue

## What the probe demonstrates

A `sin(x) + 0.5` expression, authored once as a chain-side resource, is consumed by two independently-written numerical institutions — Symbolics (which would simplify it) and IntervalArithmetic (which interval-extends it over `[0, π/2]`) — without any per-institution transformation. The cross-institution dispatch carries the **identity function** as its translation step. That's the load-bearing claim of [D32](../design/d32-chain-mirrored-mini-tt-inductives.md): the chain has *one* shared typed term language that every numerical institution speaks natively.

Test source: [`crates/eigenius-julia/tests/cross_institution_probe.rs`](../../crates/eigenius-julia/tests/cross_institution_probe.rs).

## Concepts (just enough to read the rest)

**Institution.** A typed reasoning system — a Rust trait implementation plus chain-committed resources (an `Institution`, `QueryClass`es, etc.) that contributes structured fibres to the knowledge graph. IntervalArithmetic and Symbolics are two such institutions, each with its own handler package running inside a Docker-spawned Julia worker.

**Resource class.** A typed shape committed on the chain (e.g. `BoundedBy`, `SymbolicExpression`). Instances are typed values that the chain validator type-checks at commit time.

**Mirror generator.** Walks the chain's class declarations and emits a Julia package (`EigeniusMirror`) with a struct per class plus encode/decode functions. Every institution's handler imports `EigeniusMirror` and dispatches on the typed Julia structs — no JSON parsing in handler code, no per-institution boilerplate.

**Mini-TT inductive types on the chain (D32).** The chain's `core:InductiveType` declarations define algebraic-data-type-shaped values (recursive trees with named constructors and typed argument slots). Constructors and their arg types are committed as ordinary chain resources; the validator type-checks values at commit time; the mirror generator emits Julia abstract+per-ctor structs with `decode_<T>` / `encode_<T>` functions.

**`FormulaTerm` (D32 §4).** A `core:InductiveType` committed under `urn:eigenius:formulas:` — the symbol-algebra-relevant fragment of Mini-TT `Exp`, lifted to the chain. Six constructors:

```
FormulaTerm ::=
  | Var(name: String)              -- free or binder-bound variable
  | LitFloat(value: Float)         -- numeric literal
  | OpRef(iri: String)             -- reference to a chain-committed Operator
  | App(head: FormulaTerm, arg: FormulaTerm)        -- application
  | Lam(name: String, ty: FormulaTerm, body: FormulaTerm)  -- typed binder
  | Pi(name: String,  ty: FormulaTerm, body: FormulaTerm)  -- dependent fn type
```

Crucially, `FormulaTerm` is **not** declared under any specific institution. It lives at `urn:eigenius:formulas:`, a layer above the institution-specific layers. Every numerical institution (Symbolics, IntervalArithmetic, JuMP, DiffEq, Catalyst, …) is expected to consume *the same* `FormulaTerm` shape.

**Operator catalog (D32 §5).** A v1 set of `Operator` resources at `urn:eigenius:formulas:ops:*` (`add`, `sub`, `mul`, `div`, `pow`, `sin`, `cos`, `exp`, `log`, `sqrt`, `abs`, `eq`, `lt`, `le`, `derivative`). Each operator carries a typed Mini-TT signature (also expressed as a `FormulaTerm` — the type language dogfoods itself), allowing the chain validator to rank-check `App` invocations at commit time.

**Comorphism (D14).** A typed boundary translation between two institutions, formalised as a triple `(s, m, t)`: a source `ExportFormat`, a Mini-TT Component `m` carrying the typed transformation, and a target `ImportFormat`. The component `m` is what bridges the institutions' typed payloads.

## What the probe does, mechanically

Setup:

1. **One chain accessor** pools resources from three ontology files:
   - `intervals-ontology.eigon.json` (declares `BoundedBy`).
   - `symbolics-ontology.eigon.json` (declares `SymbolicExpression`, whose `term` field has `data_type: core:inductive` and `class_types: [formulas:FormulaTerm]`).
   - `formulas-ontology.json` (declares `FormulaTerm` itself plus the operator catalog).
2. **One mirror generator pass** seeds on `[BoundedBy, SymbolicExpression]` and walks the closure. The walker pulls in `FormulaTerm` (because `SymbolicExpression.term`'s `class_types` references it) and the operator catalog. Output: one Julia package containing `EigeniusMirror.BoundedBy`, `EigeniusMirror.SymbolicExpression`, the abstract type `EigeniusMirror.FormulaTerm`, and concrete-per-ctor structs `FormulaTerm_Var` / `FormulaTerm_App` / `FormulaTerm_LitFloat` / `FormulaTerm_OpRef` / `FormulaTerm_Lam` / `FormulaTerm_Pi`.
3. **One env image** is built with the EigeniusIntervals handler package baked in. The handler is a single Julia module that exposes:
   - `validate_bounded_by(b::BoundedBy)` — the existing AutoOnLoad gate handler (unchanged).
   - `compute_bounds(expr::SymbolicExpression, domain::BoundedBy)` — the new cross-institution path. Walks `expr.term` recursively by `IntervalArithmetic.jl` semantics, with `x` bound to `interval(domain.lower, domain.upper)`, and returns the resulting interval as a `BoundedBy`.

Dispatch:

4. The probe constructs **two CBOR-encoded inputs**:
   - A `SymbolicExpression` resource whose `term` is the `FormulaTerm` tree for `sin(x) + 0.5`:
     ```
     App(App(OpRef("formulas:ops:add"),
             App(OpRef("formulas:ops:sin"), Var("x"))),
         LitFloat(0.5))
     ```
   - A `BoundedBy(_, 0.0, π/2)` carrying the domain.
5. It calls `dispatch_external_institution("julia", env, digest, "compute_bounds", sig, [expr_cbor, domain_cbor])` against the substrate.
6. The worker decodes both inputs through the mirror, dispatches `compute_bounds`, gets a `BoundedBy(lower, upper)` enclosing the function's range over the domain.

Verification:

7. The probe asserts the returned interval brackets the analytic range `[0.5, 1.5]` — the rigorous interval-arithmetic enclosure of `sin(x) + 0.5` over `[0, π/2]`. It also asserts the interval is reasonably tight (within `[0, 2]`) so we know the answer comes from interval arithmetic and not from an `Inf`/`-Inf` escape.

## What this proves structurally

`SymbolicExpression` was authored under `urn:eigenius:symbolics:`. Its `term` field was designed for the Symbolics handler's `simplify` to consume. The IntervalArithmetic handler — written independently, in a different Rust crate, by a different person in principle — accepts a `SymbolicExpression` value and **operates directly on its `term`**:

```julia
function compute_bounds(expr::SymbolicExpression, domain::BoundedBy)
    env = Dict("x" => interval(domain.lower, domain.upper))
    result = formula_to_interval(expr.term, env)   # <-- expr.term is FormulaTerm
    ...
end
```

There is no Symbolics→IntervalArithmetic format conversion. There is no per-institution payload adapter. The `expr.term :: EigeniusMirror.FormulaTerm` value is the same Julia object the Symbolics handler would dispatch `Symbolics.simplify` on — it just happens to be flowing into a different institution's handler this time.

That's [D32 §6.2](../design/d32-chain-mirrored-mini-tt-inductives.md#62-concrete-example---symbolics--intervalarithmetic) in code: a Comorphism `Symbolics → IntervalArithmetic` would carry the **identity function on `FormulaTerm`** as its Mini-TT Component `m`. The probe demonstrates that such a Comorphism is *expressible*: both ends of the boundary speak the same shape, so there's nothing for `m` to translate.

## What it intentionally does not do

- **No chain-committed `Comorphism` resource.** The probe wires `compute_bounds` as a directly-dispatchable method (via the substrate's multi-input typed dispatch), not as a formal `(s, m, t)` triple committed under `institution:Comorphism`. Doing that would require Symbolics to additionally declare an `ExportFormat ef_symb_expr` and IntervalArithmetic to declare an `ImportFormat if_intv_function`, plus the `Comorphism` triple linking them with `m = id`. That's bookkeeping on top of a working dispatch surface — none of it changes what the probe demonstrates.
- **No multi-variable expressions.** The probe binds the single free variable `x` to the domain interval. A richer multi-variable surface waits on a typed `var_context` extension to `SymbolicExpression` (track in a follow-up).
- **No actual Symbolics dispatch in the probe.** The probe exercises only IntervalArithmetic; it doesn't round-trip the same `FormulaTerm` through both institutions in one test. It doesn't need to — Symbolics' [`demo/symbolics/run.sh`](../../demo/symbolics/run.sh) already proves Symbolics consumes the shape, and the probe proves IntervalArithmetic consumes the *same* shape. The cross-institution claim is the union of those two facts; bundling both into one test would conflate two demonstrations.

## Why this matters for the platform

The "shared payload language" claim is what makes [D14](../design/d14-institution-realisation.md) Comorphisms *typed*: the kernel's component-typing machinery checks `m` against the `FormulaTerm → FormulaTerm` signature; the chain validator type-checks both endpoints against the same `InductiveType`; EigenQL FIBER queries that traverse comorphism chains compose `m`s without per-step impedance matching. None of that is possible without `FormulaTerm` actually being shared in practice — and "actually being shared" is a property no static analysis can prove. It either runs, or it doesn't.

The probe runs. That's the proof point.

## References

- [D32 — Chain-Mirrored Mini-TT Inductives + the FormulaTerm Language](../design/d32-chain-mirrored-mini-tt-inductives.md). Specifies `FormulaTerm`, the operator catalog, and the typed shared-payload claim this probe demonstrates.
- [D14 — Institution Realisation](../design/d14-institution-realisation.md). Defines Comorphisms, ExportFormat / ImportFormat, and the typed boundary discipline.
- [D27 §4 — Reference institutions](../design/d27-julia-institutions.md). The five Julia institutions (Symbolics, JuMP, IntervalArithmetic, Catalyst, DiffEq) that all speak FormulaTerm.
- [`crates/eigenius-julia/tests/cross_institution_probe.rs`](../../crates/eigenius-julia/tests/cross_institution_probe.rs). The probe itself.
- [`julia/institutions/intervals/EigeniusIntervals/src/EigeniusIntervals.jl`](../../julia/institutions/intervals/EigeniusIntervals/src/EigeniusIntervals.jl). The handler with both `validate_bounded_by` and `compute_bounds`.
- [`demo/symbolics/run.sh`](../../demo/symbolics/run.sh). The matching Symbolics demo — same `FormulaTerm` shape, simplified instead of interval-extended.
