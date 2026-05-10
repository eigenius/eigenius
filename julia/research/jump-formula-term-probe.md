# JuMP ← FormulaTerm translation probe

JuMP 1.30.1, HiGHS 1.23.0, Ipopt 1.14.3. 
Goal: determine whether a `FormulaTerm` walker translates compositionally into JuMP
expression types (`AffExpr`, `QuadExpr`, the nonlinear `GenericNonlinearExpr`) such that
`@objective(model, Min, walker_output)` and `@constraint(model, walker_output ≤ rhs)`
accept the values directly — the elegant path. The alternative is source-string
generation through `Meta.parse` + `@eval`, which would force a structural conversation
about whether `FormulaTerm` is the right shape for solver-DSL targets at all.

## Test 1 — LP: `min x + 2y s.t. x + y ≤ 10, 0 ≤ x,y ≤ 10`

- objective FormulaTerm shape: `FT_App(FT_App(FT_OpRef("urn:eigenius:formulas:ops:add"), FT_Var("x")), FT_App(FT_App(FT_OpRef("urn:eigenius:formulas:ops:mul"), FT_LitFloat(2.0)), FT_Var("y")))`
- walker output type: `AffExpr`
- walker output: `x + 2 y`
- constraint LHS walker type: `AffExpr`
- termination_status: `OPTIMAL`
- objective_value: `0.0`
- value(x): `0.0`
- value(y): `0.0`
- analytic optimum: `0.0`; |actual - expected| = `0.0`
- ✓ MATCHES analytic optimum

## Test 2 — QP via HiGHS, naive walker (`pow` → `^` → NonlinearExpr): EXPECTED TO FAIL

- walker output type: `NonlinearExpr`
- (this is `NonlinearExpr` — HiGHS does not accept it for the objective)
- HiGHS rejected the model: `ErrorException: The solver does not support an objective function of type MathOptInterface.ScalarNonlinearFunction.`

## Test 2b — same QP via HiGHS, smart walker (integer LitFloat exponent → repeated `*` → QuadExpr)

- walker output type: `QuadExpr`
- (this is `QuadExpr` — HiGHS accepts it)
- termination_status: `OPTIMAL`
- objective_value: `0.5000000000000013`
- value(x): `0.5000000249999987`
- value(y): `1.4999999750000013`
- analytic optimum: `0.5`; |actual - expected| = `1.3322676295501878e-15`
- ✓ MATCHES analytic optimum

## Test 2c — same QP via Ipopt (naive walker, NonlinearExpr): Ipopt accepts NL

- walker output type: `NonlinearExpr`
- termination_status: `LOCALLY_SOLVED`
- objective_value: `0.5`
- value(x): `0.5`
- value(y): `1.5`
- analytic optimum: `0.5`; |actual - expected| = `0.0`
- ✓ MATCHES analytic optimum

## Test 3 — NL: `min sin(x) + 0.1·x² s.t. -π ≤ x ≤ π` (Ipopt)

- walker output type: `NonlinearExpr`
- (note: this should be a `GenericNonlinearExpr` or similar in JuMP 1.x)
- termination_status: `LOCALLY_SOLVED`
- objective_value: `-0.7945823375615283`
- value(x): `-1.3064400076808929`
- analytic root: x* ≈ `-1.306440008369511`, obj* ≈ `-0.7945823375615283`
- |obj_actual - obj_expected| = `0.0`
- ✓ MATCHES analytic optimum

## Walker output type taxonomy

- Var lookup: `VariableRef`
- LitFloat: `Float64`
- Linear `x + 2y`: `AffExpr`
- Quadratic `x*y`: `QuadExpr`
- Quadratic `x^2`: `NonlinearExpr`
- Nonlinear `sin(x)`: `NonlinearExpr`
- Nonlinear `sin(x) + x^2`: `NonlinearExpr`

## Findings

**1. The compositional path works.** `@objective(model, Min, expr)` and
`@constraint(model, expr <= rhs)` accept the walker's *value* output directly — no
`Meta.parse` / `@eval` source-string detour is needed. The walker's structure
parallels EigeniusDiffEq's `formula_to_value` exactly (left-spine collection, OpRef
dispatch, recursive descent); only the `_OP_TABLE` values change (Julia's general
arithmetic operators rather than Float64-only operators), and JuMP's operator
overloading on `VariableRef` does the rest of the work.

**2. `pow(x, 2.0)` is a footgun.** When the walker translates a FormulaTerm
`pow(base, LitFloat(2.0))` via `_OP_TABLE[OP_POW] = ^`, JuMP's overloading produces a
`NonlinearExpr` rather than a `QuadExpr` because the exponent is a *Float64*, not a
small integer literal. HiGHS (and any solver that doesn't advertise
`MOI.ScalarNonlinearFunction` support) rejects the resulting objective even though
the underlying math is quadratic. Ipopt accepts it.

The fix is local to the JuMP walker: when the operator is `pow` and the exponent is
an integer-valued `LitFloat` in a small range (e.g. 0..8), unroll to repeated
multiplication so the result is `QuadExpr` (or higher polynomial). Test 2 vs.
test 2b above demonstrates this directly. The fix doesn't change the chain-side
`FormulaTerm` shape — `pow(x, 2.0)` remains the canonical encoding (consistent with
how Symbolics emits `x^2`), and the institution-side walker is what notices the
integer exponent and emits the polynomial form.

**3. Solvers fork by accepted function class.** The institution will declare two
(or more) `Institution` resources — `urn:eigenius:jump:highs` for LP/QP via HiGHS,
`urn:eigenius:jump:ipopt` for nonlinear via Ipopt — each referencing its own
`JuliaEnvironment` and each carrying its own dispatch package. The walker is
shared; the smart-pow flag is on (default) for HiGHS-targeted dispatch and either
on or off for Ipopt-targeted dispatch (Ipopt accepts both). The chain-side
`OptimisationProblem` resource is solver-agnostic; the institution dispatch chooses
which solver based on the requested institution IRI.

**4. `OptimisationProblem` shape.** Mirroring `OdeProblem`: `variable_names:
Vector<String>` (with optional `variable_bounds: Vector<VariableBound>` for boxed
variables), `objective: FormulaTerm`, `constraints: Vector<Constraint>` where each
`Constraint` carries `lhs: FormulaTerm`, `relation: ConstraintRelation` (an
inductive: `LE | GE | EQ`), `rhs: Float64`. The institution's `solve_problem`
handler walks each FormulaTerm under the smart-pow walker, hands the results to
JuMP's macros, calls `optimize!`, and reifies the `OptimisesTo` certificate
(carrying primal solution + dual values + termination status). AutoOnLoad re-solve
follows the DiffEq institution's `validate_solution` shape: re-instantiate the
model, re-solve, compare against the claimed optimum within a primal+dual tolerance.

**Revised Phase 19f estimate: 3–5 days** of focused work given the templates in place.
The smart-pow finding doesn't extend the timeline — it's a local walker tweak
discovered before the institution skeleton goes in.

