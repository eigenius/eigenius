# Focused probe — does JuMP's expression DSL compose cleanly from a
# chain-shaped `FormulaTerm` walker? Three cases: LP (linear obj +
# linear constraints), QP (quadratic obj + linear equality), NL
# (nonlinear obj). For each case we hand-build a `FormulaTerm`-shaped
# AST mirroring exactly what the chain emits (same operator IRIs as
# `formulas/formulas-ontology.json`), translate via a recursive walker
# that calls `+` / `*` / `^` / `sin` / etc. over `JuMP.VariableRef`, and
# solve, comparing against analytical optima.
#
# This answers the load-bearing unknown for Phase 19f (JuMP institution):
# whether the FormulaTerm → JuMP path is *compositional* (elegant: the
# walker returns `AffExpr` / `QuadExpr` / `NonlinearExpr` values that the
# `@objective` / `@constraint` macros consume directly) or whether it
# forces source-string generation through `Meta.parse` / `@eval` (ugly:
# means generating Julia code at runtime, recompiling, and losing the
# benefit of the chain-typed AST).
#
# Run with: julia --project=. jump-formula-term-probe.jl

import Pkg
Pkg.activate(@__DIR__)
Pkg.resolve()
Pkg.instantiate()

using JuMP
using HiGHS
using Ipopt

const OUT = open(joinpath(@__DIR__, "jump-formula-term-probe.md"), "w")

section(t) = println(OUT, "\n## ", t, "\n")
log(s) = println(OUT, s)
block(s) = (println(OUT, "```"); println(OUT, s); println(OUT, "```"))

println(OUT, "# JuMP ← FormulaTerm translation probe\n")
println(OUT, "JuMP $(string(pkgversion(JuMP))), HiGHS $(string(pkgversion(HiGHS))), Ipopt $(string(pkgversion(Ipopt))). ")
println(OUT, "Goal: determine whether a `FormulaTerm` walker translates compositionally into JuMP")
println(OUT, "expression types (`AffExpr`, `QuadExpr`, the nonlinear `GenericNonlinearExpr`) such that")
println(OUT, "`@objective(model, Min, walker_output)` and `@constraint(model, walker_output ≤ rhs)`")
println(OUT, "accept the values directly — the elegant path. The alternative is source-string")
println(OUT, "generation through `Meta.parse` + `@eval`, which would force a structural conversation")
println(OUT, "about whether `FormulaTerm` is the right shape for solver-DSL targets at all.")

# ─── FormulaTerm AST (mirrors the chain shape) ─────────────────────────

abstract type FormulaTerm end

struct FT_Var <: FormulaTerm
    name::String
end

struct FT_LitFloat <: FormulaTerm
    value::Float64
end

struct FT_OpRef <: FormulaTerm
    iri::String
end

struct FT_App <: FormulaTerm
    head::FormulaTerm
    arg::FormulaTerm
end

# Builder helpers — same operator IRIs as formulas/formulas-ontology.json,
# same App-spine shape as EigeniusSymbolics / EigeniusCatalyst /
# EigeniusDiffEq emit.
const OP_ADD = "urn:eigenius:formulas:ops:add"
const OP_SUB = "urn:eigenius:formulas:ops:sub"
const OP_MUL = "urn:eigenius:formulas:ops:mul"
const OP_DIV = "urn:eigenius:formulas:ops:div"
const OP_POW = "urn:eigenius:formulas:ops:pow"
const OP_NEG = "urn:eigenius:formulas:ops:neg"
const OP_SIN = "urn:eigenius:formulas:ops:sin"
const OP_COS = "urn:eigenius:formulas:ops:cos"
const OP_EXP = "urn:eigenius:formulas:ops:exp"
const OP_LOG = "urn:eigenius:formulas:ops:log"

# Binary App: App(App(OpRef(op), a), b)
binop(op, a, b) = FT_App(FT_App(FT_OpRef(op), a), b)
unop(op, a) = FT_App(FT_OpRef(op), a)
add(a, b) = binop(OP_ADD, a, b)
sub(a, b) = binop(OP_SUB, a, b)
mul(a, b) = binop(OP_MUL, a, b)
divide(a, b) = binop(OP_DIV, a, b)
pow(a, b) = binop(OP_POW, a, b)
neg(a) = unop(OP_NEG, a)
sin_(a) = unop(OP_SIN, a)
lit(x) = FT_LitFloat(Float64(x))
var(name) = FT_Var(name)

# ─── The walker — operator IRI → Julia function table ──────────────────
#
# Mirrors the `_OP_NUMERIC` map in EigeniusDiffEq exactly, but the
# values are Julia's general-arithmetic operators rather than Float64
# operators. The key insight under test: when the walker calls these on
# `JuMP.VariableRef` arguments, JuMP's operator overloading promotes to
# the right expression type (`AffExpr` for linear, `QuadExpr` for
# quadratic, `GenericNonlinearExpr` for nonlinear).

const _OP_TABLE = Dict{String, Function}(
    OP_ADD => +,
    OP_SUB => -,
    OP_MUL => *,
    OP_DIV => /,
    OP_POW => ^,
    OP_NEG => -,
    OP_SIN => sin,
    OP_COS => cos,
    OP_EXP => exp,
    OP_LOG => log,
)

function formula_to_jump(t::FT_App, env; smart_pow::Bool = false)
    # Walk left-spine to collect (OpRef, args...) — same as
    # EigeniusDiffEq's formula_to_value.
    cursor = t
    spine = Any[]
    while cursor isa FT_App
        push!(spine, cursor.arg)
        cursor = cursor.head
    end
    cursor isa FT_OpRef ||
        error("FormulaTerm walker: spine root is not OpRef, got $(typeof(cursor))")
    haskey(_OP_TABLE, cursor.iri) ||
        error("FormulaTerm walker: operator `$(cursor.iri)` not in table")

    # `smart_pow`: when the operator is `pow` with an integer-valued
    # LitFloat exponent (e.g. `pow(x, 2.0)`), unroll the exponentiation
    # to repeated multiplication so JuMP's operator overloading
    # produces a `QuadExpr` (or general polynomial AffExpr/QuadExpr
    # tower) rather than a `NonlinearExpr`. This is the difference
    # between HiGHS solving the problem and HiGHS rejecting it. The
    # original recursive walker (`smart_pow=false`) is the correct
    # default for solvers that handle nonlinear (Ipopt etc.); the
    # smart variant is what an institution targeting HiGHS-class
    # solvers would emit.
    if smart_pow && cursor.iri == OP_POW && length(spine) == 2
        # spine entries are *unevaluated* FormulaTerms here since we
        # want to inspect the exponent before recursing.
        base_ft, exp_ft = spine[2], spine[1]   # spine is reversed; index 1 is rightmost arg = exponent
        if exp_ft isa FT_LitFloat &&
           isinteger(exp_ft.value) &&
           exp_ft.value >= 0 &&
           exp_ft.value <= 8
            base_v = formula_to_jump(base_ft, env; smart_pow = smart_pow)
            n = Int(exp_ft.value)
            n == 0 && return 1.0
            n == 1 && return base_v
            acc = base_v
            for _ in 2:n
                acc = acc * base_v
            end
            return acc
        end
    end

    args = reverse([formula_to_jump(a, env; smart_pow = smart_pow) for a in spine])
    return _OP_TABLE[cursor.iri](args...)
end

formula_to_jump(t::FT_Var, env; smart_pow::Bool = false) =
    haskey(env, t.name) ? env[t.name] :
    error("FormulaTerm walker: unbound Var `$(t.name)`")
formula_to_jump(t::FT_LitFloat, env; smart_pow::Bool = false) = t.value

# ─── Test 1: LP ─────────────────────────────────────────────────────────
#
# minimise   x + 2y
# subject to x + y ≤ 10
#            0 ≤ x ≤ 10
#            0 ≤ y ≤ 10
#
# Analytic optimum: (x, y) = (0, 0), objective = 0.

section("Test 1 — LP: `min x + 2y s.t. x + y ≤ 10, 0 ≤ x,y ≤ 10`")

let
    model = Model(HiGHS.Optimizer)
    set_silent(model)
    @variable(model, 0 <= x <= 10)
    @variable(model, 0 <= y <= 10)
    env = Dict{String, Any}("x" => x, "y" => y)

    # FormulaTerm objective: x + 2*y
    obj_ft = add(var("x"), mul(lit(2.0), var("y")))
    log("- objective FormulaTerm shape: `$(obj_ft)`")
    obj_jmp = formula_to_jump(obj_ft, env)
    log("- walker output type: `$(typeof(obj_jmp))`")
    log("- walker output: `$(obj_jmp)`")

    # FormulaTerm constraint: x + y ≤ 10
    cstr_lhs_ft = add(var("x"), var("y"))
    cstr_lhs_jmp = formula_to_jump(cstr_lhs_ft, env)
    log("- constraint LHS walker type: `$(typeof(cstr_lhs_jmp))`")

    # Drive the macros with composed values.
    @objective(model, Min, obj_jmp)
    @constraint(model, cstr_lhs_jmp <= 10.0)

    optimize!(model)
    status = termination_status(model)
    log("- termination_status: `$(status)`")
    log("- objective_value: `$(objective_value(model))`")
    log("- value(x): `$(value(x))`")
    log("- value(y): `$(value(y))`")

    expected = 0.0
    actual = objective_value(model)
    log("- analytic optimum: `$(expected)`; |actual - expected| = `$(abs(actual - expected))`")
    log(abs(actual - expected) < 1e-6 ? "- ✓ MATCHES analytic optimum" : "- ✗ DOES NOT MATCH")
end

# ─── Test 2: QP ─────────────────────────────────────────────────────────
#
# minimise   (x - 1)² + (y - 2)²
# subject to x + y == 2
#
# Analytic optimum (Lagrangian): on x + y = 2, gradient of obj parallels
# constraint; minimiser at (x*, y*) = (0.5, 1.5), obj* = 0.5.

# The QP is `min (x-1)² + (y-2)² s.t. x + y == 2`. We test the same
# problem twice: first with the *naive* walker (`smart_pow=false`) which
# emits `pow(x-1, 2.0)` as `(x-1)^2.0` — JuMP overloads this into
# `NonlinearExpr` because the exponent is a Float64, not a small Int.
# HiGHS rejects nonlinear-typed objectives even when the underlying
# math is quadratic. Then we test with the smart walker that unrolls
# integer-valued LitFloat exponents to repeated multiplication, which
# JuMP recognises as a true `QuadExpr` and HiGHS accepts.

section("Test 2 — QP via HiGHS, naive walker (`pow` → `^` → NonlinearExpr): EXPECTED TO FAIL")

let
    model = Model(HiGHS.Optimizer)
    set_silent(model)
    @variable(model, x)
    @variable(model, y)
    env = Dict{String, Any}("x" => x, "y" => y)

    obj_ft = add(
        pow(sub(var("x"), lit(1.0)), lit(2.0)),
        pow(sub(var("y"), lit(2.0)), lit(2.0)),
    )
    obj_jmp = formula_to_jump(obj_ft, env)   # smart_pow defaults to false
    log("- walker output type: `$(typeof(obj_jmp))`")
    log("- (this is `NonlinearExpr` — HiGHS does not accept it for the objective)")

    cstr_jmp = formula_to_jump(add(var("x"), var("y")), env)
    try
        @objective(model, Min, obj_jmp)
        @constraint(model, cstr_jmp == 2.0)
        optimize!(model)
        log("- termination_status: `$(termination_status(model))`")
    catch e
        msg = sprint(showerror, e)
        first_line = first(split(msg, '\n'))
        log("- HiGHS rejected the model: `$(typeof(e)): $first_line`")
    end
end

section("Test 2b — same QP via HiGHS, smart walker (integer LitFloat exponent → repeated `*` → QuadExpr)")

let
    model = Model(HiGHS.Optimizer)
    set_silent(model)
    @variable(model, x)
    @variable(model, y)
    env = Dict{String, Any}("x" => x, "y" => y)

    obj_ft = add(
        pow(sub(var("x"), lit(1.0)), lit(2.0)),
        pow(sub(var("y"), lit(2.0)), lit(2.0)),
    )
    obj_jmp = formula_to_jump(obj_ft, env; smart_pow = true)
    log("- walker output type: `$(typeof(obj_jmp))`")
    log("- (this is `QuadExpr` — HiGHS accepts it)")

    cstr_jmp = formula_to_jump(add(var("x"), var("y")), env; smart_pow = true)
    @objective(model, Min, obj_jmp)
    @constraint(model, cstr_jmp == 2.0)
    optimize!(model)

    status = termination_status(model)
    log("- termination_status: `$(status)`")
    log("- objective_value: `$(objective_value(model))`")
    log("- value(x): `$(value(x))`")
    log("- value(y): `$(value(y))`")
    expected = 0.5
    actual = objective_value(model)
    log("- analytic optimum: `$(expected)`; |actual - expected| = `$(abs(actual - expected))`")
    log(abs(actual - expected) < 1e-6 ? "- ✓ MATCHES analytic optimum" : "- ✗ DOES NOT MATCH")
end

# Same QP through Ipopt — works regardless of walker mode because Ipopt
# accepts both QuadExpr and NonlinearExpr objectives.

section("Test 2c — same QP via Ipopt (naive walker, NonlinearExpr): Ipopt accepts NL")

let
    model = Model(Ipopt.Optimizer)
    set_silent(model)
    @variable(model, x)
    @variable(model, y)
    env = Dict{String, Any}("x" => x, "y" => y)

    obj_ft = add(
        pow(sub(var("x"), lit(1.0)), lit(2.0)),
        pow(sub(var("y"), lit(2.0)), lit(2.0)),
    )
    obj_jmp = formula_to_jump(obj_ft, env)
    log("- walker output type: `$(typeof(obj_jmp))`")

    cstr_jmp = formula_to_jump(add(var("x"), var("y")), env)
    @objective(model, Min, obj_jmp)
    @constraint(model, cstr_jmp == 2.0)
    optimize!(model)

    status = termination_status(model)
    log("- termination_status: `$(status)`")
    log("- objective_value: `$(objective_value(model))`")
    log("- value(x): `$(value(x))`")
    log("- value(y): `$(value(y))`")
    expected = 0.5
    actual = objective_value(model)
    log("- analytic optimum: `$(expected)`; |actual - expected| = `$(abs(actual - expected))`")
    log(abs(actual - expected) < 1e-6 ? "- ✓ MATCHES analytic optimum" : "- ✗ DOES NOT MATCH")
end

# ─── Test 3: NL ─────────────────────────────────────────────────────────
#
# minimise   sin(x) + 0.1·x²
# subject to -π ≤ x ≤ π
#
# This is genuinely nonlinear (transcendental). Analytic minimum at
# x* such that cos(x*) + 0.2·x* = 0; numerically x* ≈ -1.3065,
# obj* ≈ -0.7949.

section("Test 3 — NL: `min sin(x) + 0.1·x² s.t. -π ≤ x ≤ π` (Ipopt)")

let
    model = Model(Ipopt.Optimizer)
    set_silent(model)
    @variable(model, -π <= x <= π, start = -1.0)
    env = Dict{String, Any}("x" => x)

    # sin(x) + 0.1 * x^2
    obj_ft = add(
        sin_(var("x")),
        mul(lit(0.1), pow(var("x"), lit(2.0))),
    )
    obj_jmp = formula_to_jump(obj_ft, env)
    log("- walker output type: `$(typeof(obj_jmp))`")
    log("- (note: this should be a `GenericNonlinearExpr` or similar in JuMP 1.x)")

    @objective(model, Min, obj_jmp)
    optimize!(model)
    status = termination_status(model)
    log("- termination_status: `$(status)`")
    log("- objective_value: `$(objective_value(model))`")
    log("- value(x): `$(value(x))`")

    # Newton-iterate the analytic root for comparison: cos(x) + 0.2*x = 0
    xn = -1.0
    for _ in 1:50
        f = cos(xn) + 0.2 * xn
        fp = -sin(xn) + 0.2
        xn -= f / fp
    end
    expected_x = xn
    expected_obj = sin(expected_x) + 0.1 * expected_x^2
    log("- analytic root: x* ≈ `$(expected_x)`, obj* ≈ `$(expected_obj)`")
    log("- |obj_actual - obj_expected| = `$(abs(objective_value(model) - expected_obj))`")
    log(abs(objective_value(model) - expected_obj) < 1e-6 ? "- ✓ MATCHES analytic optimum" : "- ✗ DOES NOT MATCH")
end

# ─── What does the walker output look like? ─────────────────────────────
#
# Quick demo so the report shows the actual JuMP expression types
# the walker produces. Confirms compositional path: `AffExpr` for
# linear inputs, `QuadExpr` for quadratic, nonlinear-expr for NL.

section("Walker output type taxonomy")

let
    model = Model()
    @variable(model, x)
    @variable(model, y)
    env = Dict{String, Any}("x" => x, "y" => y)

    log("- Var lookup: `$(typeof(formula_to_jump(var("x"), env)))`")
    log("- LitFloat: `$(typeof(formula_to_jump(lit(3.14), env)))`")
    log("- Linear `x + 2y`: `$(typeof(formula_to_jump(add(var("x"), mul(lit(2.0), var("y"))), env)))`")
    log("- Quadratic `x*y`: `$(typeof(formula_to_jump(mul(var("x"), var("y")), env)))`")
    log("- Quadratic `x^2`: `$(typeof(formula_to_jump(pow(var("x"), lit(2.0)), env)))`")
    log("- Nonlinear `sin(x)`: `$(typeof(formula_to_jump(sin_(var("x")), env)))`")
    log("- Nonlinear `sin(x) + x^2`: `$(typeof(formula_to_jump(add(sin_(var("x")), pow(var("x"), lit(2.0))), env)))`")
end

section("Findings")

log("""
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
""")

close(OUT)
println("wrote ", joinpath(@__DIR__, "jump-formula-term-probe.md"))
