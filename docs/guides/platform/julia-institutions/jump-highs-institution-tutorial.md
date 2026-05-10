# Tutorial: the JuMP-HiGHS institution

This walkthrough wires the JuMP-HiGHS institution end-to-end — the Eigenius institution backed by [JuMP.jl](https://jump.dev) with [HiGHS](https://highs.dev) as the solver back end. Scope: linear programs and convex quadratic programs (LP / QP). The chain-typed objective and constraints are authored as `formulas:FormulaTerm` values — the same expression-tree shape Symbolics, DiffEq, IntervalArithmetic, and Catalyst already speak (D32 §6).

By the end you will have:

- Defined chain shapes for `OptimisationProblem` (problem definition: variables + bounds + objective + constraints) and `OptimisesTo` (a claim that the institution can solve the problem to a specified objective value with a specified termination status).
- Authored an LP and a QP as typed expression trees on the chain — `min x + 2y s.t. x + y ≤ 10` and `min (x-1)² + (y-2)² s.t. x + y == 2`, both validated at commit time against the FormulaTerm ctor schema.
- Generated a Julia mirror that decodes both classes (and the FormulaTerm payload, plus the `ConstraintRelation` inductive) into typed Julia structs.
- Built the env image with the `EigeniusJuMPHiGHS` handler package + JuMP + HiGHS.
- Watched two `OptimisesTo` claims fire the AutoOnLoad gate — Holds on the right-objective cases, Fails on a wildly wrong claimed objective.

The script form lives at [`demo/jump-highs/run.sh`](../../../../demo/jump-highs/run.sh) — `just demo-jump-highs` runs it end-to-end. Read this if you want to understand why the JuMP institution forks per-solver, what the smart-pow walker rule buys, and where this institution sits in the broader life-science modelling story (D27 §4.2).

If you haven't seen [the intervals tutorial](intervals-institution-tutorial.md) yet, read that first — it covers the substrate plumbing at a slower pace. The [Symbolics tutorial](symbolics-institution-tutorial.md) introduces the chain-side typed-formula machinery from D32; the [DiffEq tutorial](diffeq-institution-tutorial.md) walks through FormulaTerm-typed payloads in a comparable institution (the JuMP walker is structurally identical to DiffEq's `formula_to_value`, only the operator semantics differ).

## What's different about JuMP

Where DiffEq gates *integrated trajectories*, JuMP gates **optima**: the typed claim "this problem has this objective value at this solution under this termination status." Three things make the design choices distinctive:

### Per-solver institution siblings

JuMP itself is a solver-agnostic modelling DSL — the same model can be handed to HiGHS, Ipopt, GLPK, Gurobi, etc. Each of those solvers accepts a *different class of constraints / objectives*: HiGHS handles LP/QP and rejects `MOI.ScalarNonlinearFunction`; Ipopt handles general nonlinear; GLPK is LP-only with an integer extension. The chain-side `OptimisationProblem` is solver-agnostic — it carries variables + objective + constraints + bounds — and the institution dispatch picks which solver to use.

v1 ships **one** institution, `urn:eigenius:institutions:jump_highs`, wrapping HiGHS for LP/QP. Sibling institutions for Ipopt, GLPK, etc. plug in by reusing the same ontology + handler walker, swapping only the `Model(SolverFoo.Optimizer)` line and the `Project.toml` deps. The chain author chooses the institution by IRI; the problem stays the same.

### The smart-pow rule

`pow(x, LitFloat(2.0))` translated naively (`x ^ 2.0`) produces `MOI.ScalarNonlinearFunction` because JuMP's overload of `^` on `(VariableRef, Float64)` emits a nonlinear node. HiGHS rejects nonlinear-typed objectives even when the underlying math is quadratic. The walker's smart-pow rule recognises integer-valued `LitFloat` exponents on `pow` and unrolls to repeated multiplication so the result lands as `QuadExpr` instead — see [the JuMP/FormulaTerm probe](../../../../julia/research/jump-formula-term-probe.md) for the empirical demonstration. Only `n=2` actually buys QuadExpr (JuMP's MOI core types stop at quadratic), so the unroll is bounded; for `n ≥ 3` you get NonlinearExpr regardless.

The chain-side `FormulaTerm` shape does *not* change — `pow(x, LitFloat(2.0))` stays the canonical encoding (it's what Symbolics emits for `x^2`, what Catalyst's mass-action ODE compiler emits for second-order rate terms, etc.). The fix lives entirely inside the JuMP walker.

### `ConstraintRelation` is an inductive

The relation discriminator `LE | GE | EQ` is a chain-typed inductive (mirroring `Verdict`'s shape: `Holds | Fails | Undecidable`). The validator type-checks the value against the inductive's ctor schema and rejects malformed payloads at commit. A string field would let typos like `"<="` (with the operator misspelled) through to dispatch.

Strict inequalities (`<`, `>`) deliberately aren't in v1: numerical solvers don't distinguish strict vs. non-strict — the strict version is approximated by `lhs ≤ rhs - ε` — and committing to the discrete enum keeps the chain shape honest about what the runtime can actually verify.

## How an OptimisationProblem encodes

The objective and each `Constraint.lhs` are `FormulaTerm` values; encoding follows the [Symbolics tutorial's rule](symbolics-institution-tutorial.md#how-formulaterm-values-are-encoded-on-the-chain) exactly (tagged-dict trees with `ctor`/`args` shape, left-spined `App` currying for multi-arg operators).

Concrete example for the LP `min x + 2y`:

```json
{
  "urn:eigenius:jump:objective": {
    "ctor": "App",
    "args": [
      {"ctor": "App",
       "args": [
         {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:add"]},
         {"ctor": "Var", "args": ["x"]}
       ]},
      {"ctor": "App",
       "args": [
         {"ctor": "App",
          "args": [
            {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:mul"]},
            {"ctor": "LitFloat", "args": [2.0]}
          ]},
         {"ctor": "Var", "args": ["y"]}
       ]}
    ]
  }
}
```

Reading outermost-in: top-level `App(App(OpRef(add), x), App(App(OpRef(mul), 2.0), y))` curries to `add(x, mul(2.0, y))` — i.e. `x + 2y`.

The free variables `Var("x")` and `Var("y")` look up by name in the env the handler builds — `Dict{String, JuMP.VariableRef}` keyed by `OptimisationProblem.variable_names`. Variables outside this set raise an error from the handler.

Constraints follow the same shape, plus a `ConstraintRelation` ctor and a Float64 RHS:

```json
{
  "urn:eigenius:core:is_a": ["urn:eigenius:jump:Constraint"],
  "urn:eigenius:jump:lhs": { /* x + y as FormulaTerm */ },
  "urn:eigenius:jump:relation": {"ctor": "LE"},
  "urn:eigenius:jump:rhs": 10.0
}
```

## Prerequisites

- The compose stack running: `EIGENIUS_MOCK_LLM=true docker compose up -d`.
- A reachable Docker daemon on the host.
- `jq`.
- The workspace built once.
- **Patience for the cold env build.** JuMP + HiGHS_jll precompile in ~2–3 minutes — much lighter than the SciML dep tail DiffEq pulls. Cached after that.

The institution sources used throughout live at [`julia/institutions/jump/`](../../../../julia/institutions/jump/):

```
julia/institutions/jump/
├── declarations/
│   ├── jump-ontology.eigon.json              # OptimisationProblem + Constraint
│   │                                         # + VariableBound + ConstraintRelation
│   │                                         # + OptimisesTo
│   └── jump-highs-institution.eigon.json     # Institution + 2 RuntimeMethodSignatures
│                                             # + 2 QueryClasses (AutoOnLoad +
│                                             # OnDemand qc_jump_solve)
└── EigeniusJuMPHiGHS/
    ├── Project.toml                          # JuMP + HiGHS + MathOptInterface
    │                                         # + EigeniusMirror
    └── src/EigeniusJuMPHiGHS.jl              # solve_problem + validate_optimum
                                              # + formula_to_jump (smart-pow on)
                                              # + ConstraintRelation interpreter
```

## Step 1 — Load the JuMP ontology

```bash
eigenius load julia/institutions/jump/declarations/jump-ontology.eigon.json
```

This commits the `OptimisationProblem`, `VariableBound`, `Constraint`, `OptimisesTo` classes, the `ConstraintRelation` inductive, and the per-class properties. `OptimisationProblem.objective` and `Constraint.lhs` are typed as `core:inductive` with `class_types: [formulas:FormulaTerm]` — the validator ensures any committed value is a well-formed FormulaTerm against the operator catalog.

## Step 2 — Mirror, env, install

The institution-installation pattern is identical to the [DiffEq tutorial](diffeq-institution-tutorial.md#step-2--mirror-env-install) — generate a mirror seeded on `OptimisationProblem` + `OptimisesTo` (the closure walker pulls in `Constraint`, `VariableBound`, `ConstraintRelation`, `FormulaTerm`, and the operator catalog automatically), build the env image with the handler package + JuMP + HiGHS, commit the `RuntimeEnvironment` resource at `urn:eigenius:jump_highs:env:v1`, install the institution declaration. The demo script automates all of this.

## Step 3 — Commit the LP problem

`min x + 2y` subject to `x + y ≤ 10` and `0 ≤ x, y ≤ 10`. Optimum at `(x, y) = (0, 0)` with objective value `0`. The full JSON form is in [`demo/jump-highs/run.sh`](../../../../demo/jump-highs/run.sh) (Step 7); the structure is:

- Two `VariableBound` resources (`x ∈ [0, 10]`, `y ∈ [0, 10]`).
- One `Constraint` resource (`x + y ≤ 10`, with the LHS as a FormulaTerm).
- One `OptimisationProblem` resource referencing the bounds + constraints + a FormulaTerm objective + `sense: "Min"`.

Loading the problem doesn't fire any AutoOnLoad gate — `OptimisationProblem` is the *problem definition*, not a claim. Only `OptimisesTo` (the claim) triggers validation.

## Step 4 — Commit a Holds-case OptimisesTo

```json
{
  "@id": "urn:eigenius:demo:jump:optimum:lp",
  "urn:eigenius:core:is_a": ["urn:eigenius:jump:OptimisesTo"],
  "urn:eigenius:jump:problem": "urn:eigenius:demo:jump:problem:lp",
  "urn:eigenius:jump:termination_status": "OPTIMAL",
  "urn:eigenius:jump:objective_value": 0.0,
  "urn:eigenius:jump:variable_values": [0.0, 0.0],
  "urn:eigenius:jump:abstol": 1e-6,
  "urn:eigenius:jump:reltol": 1e-6
}
```

Loading this commit fires the AutoOnLoad gate. The handler:

1. Decodes the embedded `OptimisationProblem` (mirror-typed structs all the way down).
2. Builds a `JuMP.Model(HiGHS.Optimizer)` and creates two `VariableRef`s for `x`, `y`.
3. Walks each `VariableBound`, calling `set_lower_bound` / `set_upper_bound`.
4. Walks the objective FormulaTerm under `formula_to_jump` with `smart_pow=true`. The result is an `AffExpr` (`x + 2*y`).
5. Walks the constraint LHS, dispatches `@constraint(model, lhs_expr <= 10.0)` based on the `LE` relation.
6. Calls `optimize!`. HiGHS returns `OPTIMAL`, `objective_value(model) = 0.0`.
7. Compares status (matches) + objective value (within tolerance). Returns `Holds`.

The kernel accepts the commit.

## Step 5 — Commit the QP problem

`min (x-1)² + (y-2)² s.t. x + y == 2`. Optimum at `(x, y) = (0.5, 1.5)` with objective `0.5`. The objective is the FormulaTerm:

```
add(pow(sub(x, 1.0), 2.0), pow(sub(y, 2.0), 2.0))
```

The walker's smart-pow rule unrolls each `pow(•, LitFloat(2.0))` to `(• * •)`, so the result is `QuadExpr` rather than `NonlinearExpr`. Without that unrolling, HiGHS would reject the model — the same chain-side encoding would fail to dispatch.

The QP `OptimisesTo` claim follows the same shape; the handler builds the `QuadExpr` objective, calls `optimize!`, gets `OPTIMAL` + `objective_value = 0.5`, the verdict is Holds.

## Step 6 — A Fails-case OptimisesTo

Claim a wildly-wrong objective for the LP — say, `objective_value: -1.0`. The LP is bounded below by 0 (the objective is `x + 2y` with `x, y ≥ 0`), so −1 is unreachable. The handler:

1. Re-solves: HiGHS returns `OPTIMAL` with `objective_value = 0.0`.
2. Compares status: matches.
3. Compares objective: `|claim - actual| = 1.0`, `tol = max(1e-6, 1e-6 * |0|) = 1e-6`. Diff ≫ tol → returns `Fails`.

The kernel rejects the commit. The `OptimisesTo` claim never lands.

## Verdict policy summary

The handler's `validate_optimum` returns:

| Outcome | Trigger |
|---|---|
| `Holds` | Re-solve produces matching termination status AND, for `OPTIMAL` / `LOCALLY_SOLVED`, objective is within `max(abstol, reltol * \|actual\|)` of the claim. For `INFEASIBLE` / `DUAL_INFEASIBLE` / `INFEASIBLE_OR_UNBOUNDED`, status match alone (no numeric optimum to compare). |
| `Fails` | Structural mismatch (variable_values length); model-build error (unknown operator IRI, malformed `ConstraintRelation`, bound naming an unknown variable); solver throws; status mismatch; objective-value mismatch beyond tolerance. |
| `Undecidable` | Solver returns indeterminate state: `ITERATION_LIMIT`, `TIME_LIMIT`, `NUMERICAL_ERROR`, `SLOW_PROGRESS`, `MEMORY_LIMIT`, `NODE_LIMIT`. |

The `Fails` / `Undecidable` asymmetry is the same as DiffEq's: `Fails` means "the institution disagrees with the claim"; `Undecidable` means "the institution can't decide either way."

Note that `OptimisesTo.variable_values` is **recorded but not validated**. LP degeneracy is real — multiple optimal vertices can share the same objective value, and the re-solver may legitimately land at a different one. The institution's load-bearing scalar is the objective value (unique under feasibility + boundedness); the variable vector is a hint for downstream consumers reading the optimum.

## Where this is heading

- **Sibling institutions per solver.** The next planned drop is `urn:eigenius:institutions:jump_ipopt` for general nonlinear programming. The handler reuses the same `formula_to_jump` walker (just `smart_pow=false` since Ipopt accepts `NonlinearExpr` directly), the same ontology, only the `Project.toml` swaps `HiGHS` for `Ipopt`. Beyond that: `jump_glpk` (LP/MILP), `jump_gurobi` (commercial, advanced features) — all reusing the same `OptimisationProblem` shape.

- **Parameter-fitting flows.** The kinase-IC50 modelling story (Phase 21) wants to fit kinetic parameters from dose-response measurements. That's an optimisation problem: `min ‖predicted(p) − measured‖²` where `predicted(p)` is the output of a Catalyst → DiffEq simulation at parameter vector `p`. The objective lives as a FormulaTerm whose free variables are the parameters; constraints encode prior bounds. The optimum is committed as an `OptimisesTo` whose AutoOnLoad gate re-solves and confirms — i.e., the chain records both *what the optimal parameters are* and *that the institution can independently reproduce them*. The kinase case keeps things in LP/QP territory (linear least-squares with bounds) and stays inside HiGHS's wheelhouse; nonlinear fits (logistic dose-response) move to `jump_ipopt`.

- **`Symbolics → JuMP` comorphism (sketch).** A `SymbolicExpression` carrying a polynomial in the chain's variable names can be reified directly as an `OptimisationProblem.objective` — both are FormulaTerm. The natural pairing is "user authors a model symbolically, dispatches optimisation through JuMP" without re-typing the expression. The Comorphism resource declaration is straightforward (identity Lambda on FormulaTerm, source/target the relevant institutions); landing it is an afternoon's work once the second user shows up.
