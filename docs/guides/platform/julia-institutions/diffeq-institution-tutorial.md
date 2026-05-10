# Tutorial: the DiffEq institution

This walkthrough wires the DiffEq institution end-to-end — the Eigenius institution backed by [`OrdinaryDiffEq.jl`](https://github.com/SciML/OrdinaryDiffEq.jl) for ODE integration, with **`formulas:FormulaTerm` as the chain-typed payload for the RHS expression**. By the end you will have:

- Defined chain shapes for `OdeProblem` (problem definition) and `OdeSolution` (a claim that the institution can integrate the problem to a specified final state).
- Authored an ODE's right-hand side as a *typed expression tree* on the chain — `du/dt = -k·u` becomes `App(OpRef(neg), App(App(OpRef(mul), Var(k)), Var(u)))`, validated at commit time against the FormulaTerm ctor schema and the operator catalog.
- Generated a Julia mirror that decodes both classes (and the FormulaTerm payload) into typed Julia structs.
- Built the env image with the `EigeniusDiffEq` handler package + a precompiled `OrdinaryDiffEq.jl`.
- Watched two `OdeSolution` claims fire the AutoOnLoad gate — one with the right closed-form final state (Holds, accepted), one with a wildly wrong final state (Fails, rejected at commit).

The script form lives at [`demo/diffeq/run.sh`](../../../../demo/diffeq/run.sh) — `just demo-diffeq` runs it end-to-end. Read this if you want to understand why DiffEq carries its RHS as a FormulaTerm rather than a Julia source string, what that buys you in cross-institution composition, and where this institution sits in the broader life-science modelling story (D27 §4.5, D32 §6).

If you haven't seen [the intervals tutorial](intervals-institution-tutorial.md) yet, read that first — it covers the substrate plumbing at a slower pace. The [Symbolics tutorial](symbolics-institution-tutorial.md) introduces the chain-side typed-formula machinery from D32; **DiffEq is the second institution to consume FormulaTerms** (after IntervalArithmetic's `qc_compute_bounds`), and the comorphism story really starts paying off here. The [Catalyst tutorial](catalyst-institution-tutorial.md) introduces the kinase mechanism whose time-course this institution can integrate via the `Catalyst → DiffEq` comorphism — see [Where this is heading](#where-this-is-heading).

## What's different about DiffEq

Where intervals gates flat numerics, Symbolics gates expression trees, and Catalyst gates structural network invariants, **DiffEq gates *integrated trajectories*** — the typed claim "this is the solution to that ODE problem at those tolerances, ending at this final state."

Two things make DiffEq's design choices distinctive:

### Tolerance-bounded verdicts

Intervals gives you bit-deterministic verdicts (interval arithmetic with pinned rounding mode). Symbolics gives you heuristic verdicts (the simplifier might or might not converge). DiffEq gives you **tolerance-bounded verdicts**: the institution re-integrates with the same algorithm and tolerances and confirms the final state matches *within the tolerance the user committed*. A claim with `abstol = 1e-3` accepts looser agreement than a claim with `abstol = 1e-12`.

This matters because:

- **The same problem can have multiple valid solutions at different fidelities.** A coarse `abstol = 1e-3` solution and a tight `abstol = 1e-12` solution are *both* valid claims, validated independently. Downstream queries pick which fidelity they need.
- **`Undecidable` is meaningful here**, not just a heuristic-fallback escape hatch. When a stiff problem hits `MaxIters` or `ConvergenceFailure`, the institution genuinely can't reach a binary verdict — it's not refuting the claim, it's telling you the integrator timed out. The user re-authors the claim with a stiff solver (`QNDF`, `FBDF`, `Rodas5`) and tries again.

### FormulaTerm-typed RHS

The RHS function `f(u, p, t) = du/dt` is exactly the kind of expression D32's `formulas:FormulaTerm` was designed for. DiffEq carries it that way — a list of FormulaTerm-typed `RhsComponent` wrappers, one per state variable. This is the v1.5 fix that landed in Phase 19g.5.

Three things this buys, none of which work if the RHS is a Julia source string:

1. **Validator type-checking at commit.** A FormulaTerm value gets structurally validated against the chain's ctor schema and the operator catalog. A source string sails through unchecked; an unknown operator IRI shows up only at dispatch time when the worker explodes mid-integration.
2. **Cross-institution comorphism.** A `Symbolics.SymbolicExpression` carries a FormulaTerm; with DiffEq also speaking FormulaTerm, a `Symbolics → DiffEq` comorphism is the identity on FormulaTerm (per [D32 §6.2](../../../design/d32-chain-mirrored-mini-tt-inductives.md#62-concrete-example---symbolics--intervalarithmetic)). Source strings broke that pairing.
3. **Interval-extension reuse.** [`EigeniusIntervals.formula_to_interval`](../../../../julia/institutions/intervals/EigeniusIntervals/src/EigeniusIntervals.jl) walks FormulaTerm under interval semantics. The (still-ahead) `DiffEq → IntervalArithmetic` comorphism that bounds an integrated trajectory reuses that machinery — only because DiffEq's RHS is *the same FormulaTerm shape* the interval institution already understands.

## How a FormulaTerm RHS encodes

If you haven't read the [Symbolics tutorial's encoding section](symbolics-institution-tutorial.md#how-formulaterm-values-are-encoded-on-the-chain) yet, read it now — the encoding rule (tagged-dict trees with `ctor`/`args` shape, left-spined `App` currying for multi-arg operators) is identical here.

Concrete example: `du/dt = -k·u` (exponential decay). With `state_names = ["u"]`, `parameter_names = ["k"]`, the single RHS component is:

```json
{
  "ctor": "App",
  "args": [
    {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:neg"]},
    {"ctor": "App",
     "args": [
       {"ctor": "App",
        "args": [
          {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:mul"]},
          {"ctor": "Var", "args": ["k"]}
        ]},
       {"ctor": "Var", "args": ["u"]}
     ]}
  ]
}
```

Reading outermost-in: top-level `App(OpRef(neg), _)` is `-_`; the `_` is `App(App(OpRef(mul), Var(k)), Var(u))` which curries to `mul(k, u)` — i.e., `-(k·u)`.

The free variables `Var("k")` and `Var("u")` look up by name in the env the handler builds at each integration step:

| Var name | Source |
|---|---|
| names from `state_names` | `u[N]` from the integrator, in `state_names` order |
| names from `parameter_names` | `parameters[N]` from the OdeProblem |
| `t` (reserved) | the integrator's current time |

Free variables outside this set raise an error from the handler — there's no implicit zero-fill.

## Prerequisites

- The compose stack running: `EIGENIUS_MOCK_LLM=true docker compose up -d`.
- A reachable Docker daemon on the host. (See [the intervals tutorial](intervals-institution-tutorial.md#prerequisites) for the substrate-depot detail.)
- `jq`.
- The workspace built once.
- **Patience for the cold env build.** OrdinaryDiffEq.jl pulls the SciML dep tail (SciMLBase, RecursiveArrayTools, FunctionWrappers, NonlinearSolve, …); first-run `Pkg.precompile` takes ~5 minutes. Cached after that.

The institution sources used throughout live at [`julia/institutions/diffeq/`](../../../../julia/institutions/diffeq/):

```
julia/institutions/diffeq/
├── declarations/
│   ├── diffeq-ontology.eigon.json     # OdeProblem + OdeSolution + RhsComponent
│   │                                  # classes + their properties
│   └── diffeq-institution.eigon.json  # Institution + RuntimeMethodSignature
│                                      # + AutoOnLoad QueryClass
└── EigeniusDiffEq/
    ├── Project.toml                   # OrdinaryDiffEq + SciMLBase + EigeniusMirror
    └── src/EigeniusDiffEq.jl          # validate_solution + formula_to_value
                                       # + algorithm registry + numeric
                                       # operator catalog
```

## Step 1 — Load the DiffEq ontology

```bash
eigenius --endpoint http://localhost:50051 \
    load julia/institutions/diffeq/declarations/diffeq-ontology.eigon.json
```

### What just happened

The ontology declares three classes:

| Class | Required properties | Role |
|---|---|---|
| `OdeProblem` | `state_names`, `parameter_names`, `rhs`, `initial_conditions`, `parameters`, `time_span_start`, `time_span_end` | The problem definition. No AutoOnLoad gate — it's data. |
| `RhsComponent` | `term` | Wrapper around one FormulaTerm — the Nth `RhsComponent` in `OdeProblem.rhs` carries the expression for `du[N]/dt`. |
| `OdeSolution` | `problem`, `algorithm`, `abstol`, `reltol`, `final_state` | A claim that the institution can integrate the referenced problem to the named final state. AutoOnLoad-gated. |

A few worth pausing on:

- **`rhs: resource_array<RhsComponent>`** — a list of typed wrappers. `RhsComponent` is a one-property class wrapping a FormulaTerm. The wrapper exists because the chain doesn't have a parametric `core:List<T>` shape yet ([D32 §3.4](../../../design/d32-chain-mirrored-mini-tt-inductives.md) sketches it as future work); the wrapper class works with existing chain machinery and leaves room for per-component metadata to grow later (provenance refs, doc strings, etc.).
- **`state_names` / `parameter_names`** — value arrays of strings. These are the canonical positional ordering the FormulaTerms reference by name. Length must match `initial_conditions` / `parameters`; the handler enforces this at dispatch.
- **`abstol` / `reltol`** — both required on every `OdeSolution`. The institution enforces no defaults at the chain shape (you choose your fidelity at commit time and live with it forever).
- **`algorithm: string`** — the named OrdinaryDiffEq integrator. v1 supports `Tsit5` (5th-order RK, default for non-stiff problems), `Vern9` (9th-order, high accuracy), `Rosenbrock23` / `Rodas5` / `Rodas5P` (stiff), and `QNDF` / `FBDF` (very stiff). Adding an algorithm = one entry in the handler's `_ALG_REGISTRY` map. The validator doesn't restrict the string at commit; an unsupported name shows up as `Fails` at dispatch.

## Steps 2–6 — Mirror, env image, env Resource, install institution

Same shape as the [intervals](intervals-institution-tutorial.md#step-2--anchor-the-mirror-to-a-layer) and [Catalyst](catalyst-institution-tutorial.md#steps-26--mirror-env-image-env-resource-install-institution) tutorials. DiffEq-specific notes:

### The mirror filter seeds three classes

```bash
eigenius mirror create \
    --layer "$LAYER_IRI" \
    --filter 'MATCH "urn:eigenius:core:Class"(?iri) {
                "urn:eigenius:core:short_name": ?name
              }
              WHERE ?name IN ["OdeProblem", "OdeSolution", "RhsComponent"]
              RETURN [] { iri: ?iri }' \
    --language julia \
    --output /tmp/diffeq-mirror \
    --json
```

Closure walking pulls in `formulas:FormulaTerm` (the inductive type) plus the operator catalog automatically — same closure shape the Symbolics tutorial uses. The mirror generator emits `struct OdeProblem`, `struct RhsComponent { term::FormulaTerm }`, `struct OdeSolution`, plus the `abstract type FormulaTerm` hierarchy and per-ctor structs (`FormulaTerm_Var`, `FormulaTerm_App`, etc.).

### The env build is slow

Cold runs take ~5 minutes — OrdinaryDiffEq's dep tree is substantial. Faster than Catalyst, slower than intervals.

### The institution declares one QueryClass

`solution_validity`, AutoOnLoad on `OdeSolution`. Future v2: `qc_diffeq_solve` (OnDemand), `qc_diffeq_sensitivity` (OnDemand), trajectory-hashing `ReproducibleIntegration`.

## Step 4 — Read the handler

The handler at [`EigeniusDiffEq.jl`](../../../../julia/institutions/diffeq/EigeniusDiffEq/src/EigeniusDiffEq.jl) has four pieces worth reading.

### The algorithm registry

```julia
const _ALG_REGISTRY = Dict{String, Any}(
    "Tsit5" => Tsit5(),
    "Vern9" => Vern9(),
    "Rosenbrock23" => Rosenbrock23(),
    "Rodas5" => Rodas5(),
    "Rodas5P" => Rodas5P(),
    "QNDF" => QNDF(),
    "FBDF" => FBDF(),
)
```

A name → constructor allowlist. The chain's `algorithm` string is *looked up*, never `Core.eval`'d. Adding an algorithm means adding an entry; unrecognised names produce `Fails` with a clear diagnostic.

### The numeric operator catalog

```julia
const _OP_NUMERIC = Dict{String, Function}(
    "urn:eigenius:formulas:ops:add" => +,
    "urn:eigenius:formulas:ops:mul" => *,
    "urn:eigenius:formulas:ops:neg" => -,
    "urn:eigenius:formulas:ops:exp" => exp,
    # … the rest of the catalog …
)
```

Mirrors `_OP_INTERVAL` in `EigeniusIntervals` and `_OP_FN` in `EigeniusSymbolics` — the chain's operator catalog is the same; what varies is the per-institution interpretation (Float64 here, `Interval` over there, `Symbolics.Num` for Symbolics).

### The FormulaTerm interpreter

```julia
formula_to_value(t::FormulaTerm_Var, env) =
    haskey(env, t.name) ? env[t.name] : error("unbound variable `$(t.name)`")

formula_to_value(t::FormulaTerm_LitFloat, env) = t.value

function formula_to_value(t::FormulaTerm_App, env)
    spine = Any[]
    cursor = t
    while cursor isa FormulaTerm_App
        push!(spine, cursor.arg)
        cursor = cursor.head
    end
    cursor isa FormulaTerm_OpRef || error("expected OpRef at App head")
    args = reverse([formula_to_value(a, env) for a in spine])
    return _OP_NUMERIC[cursor.iri](args...)
end
```

Recursive walker. Julia's multiple dispatch on the per-ctor mirror structs (`FormulaTerm_Var`, `FormulaTerm_LitFloat`, `FormulaTerm_App`, …) is the dispatch — no manual tag-checking, no string parsing. The `App` case walks the left spine to find the `OpRef` head and applies the operator to the reversed arg list (matches the curried encoding).

### The closure builder + handler

```julia
function build_rhs_closure(rhs_components, state_names, parameter_names)
    env = Dict{String, Float64}()
    function rhs!(du, u, p, t)
        for (i, name) in enumerate(state_names);     env[name] = u[i] end
        for (i, name) in enumerate(parameter_names); env[name] = p[i] end
        env["t"] = t
        for i in eachindex(rhs_components)
            du[i] = formula_to_value(rhs_components[i].term, env)
        end
    end
    return rhs!
end
```

Built once at problem-construction time; the closure captures the FormulaTerm trees and name-vectors and just refreshes env values each step. Then `validate_solution` builds the closure, constructs `ODEProblem(rhs!, u0, tspan, p)`, calls `solve` with the claimed algorithm + tolerances, and per-component-compares the integrator's final state against `s.final_state`.

### Verdict policy

- **`Holds`**: solver succeeded AND final state matches per-component within `max(abstol, reltol * |actual_i|)`.
- **`Fails`**: structural error (length mismatches, unknown algorithm, malformed FormulaTerm), solver hard-fails, OR final state differs significantly. The institution refutes the claim.
- **`Undecidable`**: solver returns indeterminate states (`MaxIters`, `ConvergenceFailure`, `DtNaN`, `DtLessThanMin`, `Stalled`, `MaxNumSub`, `MaxTime`). Can't decide either way.

## Step 7 — Commit the OdeProblem (exponential decay)

```bash
eigenius load <<<'[
  {
    "@id": "urn:eigenius:demo:diffeq:problem:exp_decay",
    "core:is_a": ["urn:eigenius:diffeq:OdeProblem"],
    "core:short_name": "exp_decay",
    "diffeq:state_names": ["u"],
    "diffeq:parameter_names": ["k"],
    "diffeq:initial_conditions": [1.0],
    "diffeq:parameters": [1.0],
    "diffeq:time_span_start": 0.0,
    "diffeq:time_span_end": 1.0,
    "diffeq:rhs": [
      {
        "core:is_a": ["urn:eigenius:diffeq:RhsComponent"],
        "diffeq:term": { ...FormulaTerm for -k*u... }
      }
    ]
  }
]'
```

(See [the encoding section above](#how-a-formulaterm-rhs-encodes) for the full FormulaTerm expansion of `-k*u`.)

### What just happened

An `OdeProblem` resource was committed. **No AutoOnLoad gate fired** — the institution declares no `query_class: OdeProblem`. The problem is data; only `OdeSolution` claims trigger validation.

Authoring richer ODEs by hand (e.g. the kinase mechanism's six coupled ODEs) is verbose — that's where the **`Catalyst → DiffEq` comorphism** shines: it produces FormulaTerm-typed `OdeProblem`s directly from chain-committed `ReactionNetwork`s. See [Where this is heading](#where-this-is-heading).

## Step 8 — Commit the Holds-case OdeSolution

```bash
eigenius load <<<'[
  {
    "@id": "urn:eigenius:demo:diffeq:solution:exp_decay_at_1",
    "core:is_a": ["urn:eigenius:diffeq:OdeSolution"],
    "diffeq:problem": "urn:eigenius:demo:diffeq:problem:exp_decay",
    "diffeq:algorithm": "Tsit5",
    "diffeq:abstol": 1e-8,
    "diffeq:reltol": 1e-8,
    "diffeq:final_state": [0.36787944117144233]
  }
]'
```

### What just happened

The kernel commit pipeline:

1. Type-checks the resource — `final_state` is a `value_array<float>`, all good.
2. Looks up `solution_validity` in `auto_on_load_by_class[OdeSolution]`.
3. Dispatches `validate_solution` to the orchestrator with the `OdeSolution` mirror struct (with the embedded `OdeProblem` carrying its FormulaTerm RHS) serialised to Eigon-CBOR. The kernel's IRI-dereference pass embeds the chain-committed problem into the synthetic input.
4. The Julia handler decodes the input, builds the env-driven closure from the FormulaTerm RHS, constructs `ODEProblem`, solves with `Tsit5; abstol=1e-8, reltol=1e-8`. Each integration step calls `rhs!(du, u, p, t)` which calls `formula_to_value(term, env)` for each component — typed dispatch on the per-ctor mirror structs all the way down.
5. `sol.u[end] ≈ 0.36787944117144222`. Tolerance check: `|0.36787944117144233 - 0.36787944117144222| ≈ 1.1e-16`, bound is `max(1e-8, 1e-8 * 0.368) = 1e-8`, well within. `Holds`.
6. The kernel commits the `OdeSolution` plus a `Verdict + RuntimeInvocation` audit anchor.

## Step 9 — Try to commit a Fails-case OdeSolution

```bash
eigenius load <<<'[
  {
    "@id": "urn:eigenius:demo:diffeq:solution:exp_decay_wrong",
    "core:is_a": ["urn:eigenius:diffeq:OdeSolution"],
    "diffeq:problem": "urn:eigenius:demo:diffeq:problem:exp_decay",
    "diffeq:algorithm": "Tsit5",
    "diffeq:abstol": 1e-8,
    "diffeq:reltol": 1e-8,
    "diffeq:final_state": [0.5]
  }
]'
```

This load fails:

```
Load failed:
  InstitutionValidation: AutoOnLoad QueryClass `solution_validity` returned Fails
```

The tolerance check refutes: `|0.5 - 0.368| = 0.132`, bound is `1e-8`, way over. Per [D31 §6.3](../../../design/d31-external-institution-lifecycle.md), the gated commit is rejected; the chain stays consistent. The `Verdict + RuntimeInvocation` audit anchor *does* commit (on a side layer) so the rejection has a verifiable cause.

## Step 10 — Inspect the verdicts

```bash
eigenius query \
    'MATCH "urn:eigenius:institution:Verdict"(?v) {
        "urn:eigenius:core:ctor_name": ?ctor
     } RETURN [] { verdict: ?v, ctor: ?ctor }'
```

Two rows: one `Holds` (step 8), one `Fails` (step 9 audit anchor).

## What now lives on the chain

After running the demo:

| Resource | Source |
|---|---|
| `OdeProblem` + `OdeSolution` + `RhsComponent` classes + 13 properties | step 1 |
| `RuntimePackageMirror` | step 3 |
| `RuntimeEnvironment` (`urn:eigenius:diffeq:env:v1`) | step 5 |
| `Institution` + `RuntimeMethodSignature` + `QueryClass` | step 6 |
| Exponential-decay `OdeProblem` (with FormulaTerm RHS) | step 7 |
| Holds-case `OdeSolution` + `Verdict` + `RuntimeInvocation` | step 8 |
| Fails-case `Verdict` + `RuntimeInvocation` (audit anchor for the rejection) | step 9 |

The Fails-case `OdeSolution` is *not* on the chain — its load was rejected. But the audit trail explaining why is.

## Where this is heading

DiffEq v1.5 (this version, with FormulaTerm-typed RHS) is the foundation. The interesting downstream pieces stack on top.

**The `Catalyst → DiffEq` comorphism** (next, the natural pairing of Catalyst and DiffEq):
- A typed `Comorphism(ef_cat_to_ode_input, m_cat_to_diffeq, if_diffeq_problem)` triple.
- `m_cat_to_diffeq` is an institution-runtime Component owned by Catalyst that internally calls `ODEProblem(rn, u0_map, tspan, p_map)` to expand mass-action kinetics, then walks the resulting MTK `ODESystem` and produces FormulaTerm RHS components via the same `num_to_formula` path Symbolics' simplifier uses.
- With both institutions on the chain, FIBER calls like `cap:qc_cat_to_ode(network)` become single-step ways to generate FormulaTerm-typed solvable problems.
- The kinase mechanism becomes the canonical demo: ReactionNetwork → (comorphism) → OdeProblem with FormulaTerm RHS → (DiffEq AutoOnLoad) → OdeSolution.

**The `DiffEq → IntervalArithmetic` comorphism** (eventually):
- A `BoundedTrajectory` claim class on the IntervalArithmetic side: claim that `u(t)` stays within an interval over the time domain.
- The transformation walks DiffEq's FormulaTerm RHS under interval-arithmetic semantics + the OdeSolution's time grid to produce the bound. Reuses [`EigeniusIntervals.formula_to_interval`](../../../../julia/institutions/intervals/EigeniusIntervals/src/EigeniusIntervals.jl) — the *same FormulaTerm walker* the cross-institution probe ([`crates/eigenius-julia/tests/cross_institution_probe.rs`](../../../../crates/eigenius-julia/tests/cross_institution_probe.rs)) already demonstrates for `compute_bounds`.

**DiffEq v2** (independent of comorphism work):
- **`ReproducibleIntegration`** claims with trajectory-hash content addressing — bit-reproducibility audits per D27 §4.5.1.
- **`qc_diffeq_solve`** OnDemand QueryClass — `solve(prob; alg)` from FIBER directly.
- **`qc_diffeq_sensitivity`** OnDemand — sensitivity analysis along the trajectory.

**The kinase IC₅₀ pipeline** (real downstream value):
- ReactionNetwork (Catalyst) → OdeProblem (Catalyst→DiffEq comorphism) → OdeSolution (DiffEq AutoOnLoad).
- Parameter sweep over inhibitor concentration → numerical IC₅₀.
- IntervalArithmetic-bounded confidence intervals via the FormulaTerm-shared interval extension.
- The kinase-institutions notebook ([`notebooks/examples/kinase-institutions.json`](../../../../notebooks/examples/kinase-institutions.json)) upgrades from "flat dataset of measurements" to "mechanistic predictions cross-checked against assay measurements with rigorous confidence intervals."

Each of these stacks on top of the FormulaTerm-typed RHS this version pins. They wouldn't compose if DiffEq's RHS were a Julia source string.

## Common failure modes

| Symptom | Cause | Fix |
|---|---|---|
| `unbound variable \`X\` in RHS` | A `Var(name)` in a FormulaTerm references a name not in `state_names`, `parameter_names`, or the reserved `t`. | Add the name to the right list, or fix the FormulaTerm to reference an existing one. |
| `operator \`urn:eigenius:formulas:ops:foo\` not in numeric catalog` | A FormulaTerm uses an operator the handler doesn't know how to evaluate numerically. | Add an entry to `_OP_NUMERIC` in `EigeniusDiffEq.jl`. |
| `Fails` on a claim you believe should hold | Two common causes: (1) `final_state` length doesn't match `length(state_names)`; (2) the tolerances are tighter than the integrator can deliver. | Print the integrator's actual final state in a Julia REPL and compare. Loosen tolerances if the discrepancy is at the `1e-12` level. |
| `Undecidable` on a stiff system | Default `Tsit5` can't handle stiffness; bumping into `MaxIters` / `ConvergenceFailure`. | Switch `algorithm` to `Rodas5` (BDF-style) or `QNDF` (very stiff). |
| `unknown algorithm` from the handler | The `algorithm` string isn't in `_ALG_REGISTRY`. | Use a registered name (`Tsit5`, `Vern9`, `Rosenbrock23`, `Rodas5`, `Rodas5P`, `QNDF`, `FBDF`) or extend the registry. |
| Validator rejects the OdeProblem with a `class_types` mismatch on `rhs` | A `RhsComponent` element has the wrong `is_a` or its `term` isn't a valid FormulaTerm. | Re-author the FormulaTerm against the encoding rules in [the Symbolics tutorial](symbolics-institution-tutorial.md#how-formulaterm-values-are-encoded-on-the-chain). |

For substrate-level failure modes (`seed manifest drift`, depot bind-mount issues, manifest-hash mismatch), see [the intervals tutorial](intervals-institution-tutorial.md#common-failure-modes).
