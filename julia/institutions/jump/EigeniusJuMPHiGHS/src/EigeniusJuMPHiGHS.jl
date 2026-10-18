"""
    EigeniusJuMPHiGHS

Handler package for the JuMP-HiGHS Eigenius institution (Phase 19f /
D27 §4.2). Exports `validate_optimum(c)` (the AutoOnLoad gate's worker
entry point for `OptimisesTo` resources) and `solve_problem(p)` (the
OnDemand `qc_jump_solve` worker entry point for `OptimisationProblem`
resources).

# Dispatch flow — `validate_optimum`

1. The kernel commits an `OptimisesTo(problem, termination_status,
   objective_value, variable_values, abstol, reltol)` resource on a
   chain that has the JuMP-HiGHS institution installed.
2. `commit_with_validation` fires the AutoOnLoad QueryClass, which
   sends a `DispatchExternal` RPC to the orchestrator carrying the
   `OptimisesTo` mirror struct (with the embedded `OptimisationProblem`
   and its FormulaTerm-typed `objective` + each `constraints[i].lhs`)
   serialised to Eigon-CBOR.
3. The orchestrator routes the call through the substrate's Julia
   runtime; the worker decodes the input via the mirror's
   `decode_OptimisesTo` codec — which transitively decodes the nested
   `problem::OptimisationProblem`, each `constraints::Vector{Constraint}`,
   each `objective::FormulaTerm`, each `lhs::FormulaTerm`, and each
   `relation::ConstraintRelation` (via the per-ctor mirror structs).
4. This handler walks each FormulaTerm under a JuMP-expression
   interpreter driven by a `Dict{String, JuMP.VariableRef}` env, builds
   a `JuMP.Model(HiGHS.Optimizer)`, attaches `@objective` + each
   `@constraint`, calls `optimize!`, and tolerance-checks the resolved
   objective against the claim.
5. Returns a `Verdict` Dict — `Holds` on a successful re-solve that
   matches; `Fails` on a status mismatch, an objective-value mismatch
   beyond tolerance, or a solver error when success was claimed;
   `Undecidable` on indeterminate solver states.

# Why FormulaTerm-typed objective and constraints

The chain's flat formula language is the right shape for JuMP's
expression DSL. Per the FormulaTerm-translation probe at
`julia/research/jump-formula-term-probe.md`, JuMP's `+` / `*` / `^`
operator overloading on `JuMP.VariableRef` produces `AffExpr` /
`QuadExpr` / `NonlinearExpr` automatically — the walker just calls the
right operator on the right operands and the type promotes
correctly. The `@objective(model, sense, expr)` and
`@constraint(model, expr ⋄ rhs)` macros accept the walker's output
directly; no `Meta.parse` / `@eval` source-string detour.

The sole subtlety the probe surfaced is that `pow(x, LitFloat(2.0))`
translated naively (`x ^ 2.0`) produces `NonlinearExpr` rather than
`QuadExpr`, because JuMP's overload of `^` on `(VariableRef, Float64)`
emits the nonlinear node — and HiGHS rejects nonlinear-typed
objectives even when the underlying math is quadratic. The
`smart_pow=true` walker variant (the default for this institution)
recognises integer-valued `LitFloat` exponents and unrolls `pow(x, 2.0)`
to `x * x` so the result is `QuadExpr`. Only `n=2` actually buys
QuadExpr — for `n=3+`, repeated multiplication promotes to NonlinearExpr
again because JuMP's MOI core types stop at quadratic.

# Verdict policy

- `Holds`: re-solve produces matching `termination_status` AND, when
  status is `OPTIMAL` / `LOCALLY_SOLVED`, the resolved objective is
  within `max(abstol, reltol * |actual|)` of the claim. When status
  is a non-optimal terminal that matches the claim (`INFEASIBLE` /
  `INFEASIBLE_OR_UNBOUNDED` / `DUAL_INFEASIBLE`), `Holds` records
  agreement that the problem has no feasible optimum — there's no
  numeric optimum to compare.
- `Fails`:
   - structural mismatch (`variable_values` length doesn't match
     `problem.variable_names`);
   - model-build failure (unknown operator IRI, malformed
     ConstraintRelation, VariableBound naming an unknown variable);
   - solver throws an exception;
   - `termination_status` mismatch;
   - objective-value mismatch beyond tolerance.
- `Undecidable`: solver returns indeterminate states like
  `ITERATION_LIMIT`, `TIME_LIMIT`, `NUMERICAL_ERROR`, `SLOW_PROGRESS`
  — the institution can't reach a binary answer with the configured
  budget.

The asymmetry mirrors EigeniusDiffEq's: `Fails` means "the institution
disagrees with the claim"; `Undecidable` means "the institution can't
decide either way."
"""
module EigeniusJuMPHiGHS

using JuMP
using HiGHS
using MathOptInterface
const MOI = MathOptInterface
using EigeniusMirror

export validate_optimum, solve_problem, reify_problem

const VERDICT_CLASS_IRI = "urn:eigenius:institution:Verdict"
const IS_A_PROP = "urn:eigenius:core:is_a"
const CTOR_NAME_PROP = "urn:eigenius:core:ctor_name"

_verdict(ctor::AbstractString) = Dict{String, Any}(
    IS_A_PROP => [VERDICT_CLASS_IRI],
    CTOR_NAME_PROP => ctor,
)

# ─── Operator catalog ───────────────────────────────────────────────────
#
# Mirrors the `_OP_NUMERIC` map in EigeniusDiffEq exactly, but the
# values are Julia's *general-arithmetic* operators rather than
# Float64-only operators. JuMP's operator overloading on
# `JuMP.VariableRef` promotes the result to the right expression type
# (`AffExpr` / `QuadExpr` / `NonlinearExpr`); the walker doesn't need
# to know the algebraic structure of its inputs.

const OP_ADD = "urn:eigenius:formulas:ops:add"
const OP_SUB = "urn:eigenius:formulas:ops:sub"
const OP_MUL = "urn:eigenius:formulas:ops:mul"
const OP_DIV = "urn:eigenius:formulas:ops:div"
const OP_POW = "urn:eigenius:formulas:ops:pow"
const OP_NEG = "urn:eigenius:formulas:ops:neg"

const _OP_TABLE = Dict{String, Function}(
    OP_ADD => +,
    OP_SUB => -,
    OP_MUL => *,
    OP_DIV => /,
    OP_POW => ^,
    OP_NEG => -,
    "urn:eigenius:formulas:ops:exp" => exp,
    "urn:eigenius:formulas:ops:log" => log,
    "urn:eigenius:formulas:ops:sin" => sin,
    "urn:eigenius:formulas:ops:cos" => cos,
    "urn:eigenius:formulas:ops:tan" => tan,
    "urn:eigenius:formulas:ops:sqrt" => sqrt,
    "urn:eigenius:formulas:ops:abs" => abs,
)

# ─── FormulaTerm → JuMP expression walker ───────────────────────────────

"""
    formula_to_jump(t::FormulaTerm, env; smart_pow=true)

Recursively translate a chain-typed FormulaTerm into a JuMP expression.
`env::Dict{String, Any}` maps variable names (the `Var(name)`
references from the FormulaTerm) to their `JuMP.VariableRef` instances.
The result is whatever JuMP's operator overloading produces — `Float64`
for pure-numeric subtrees, `AffExpr` for linear, `QuadExpr` for
quadratic, `NonlinearExpr` for transcendental.

The `smart_pow` flag (default `true` for this institution) controls
the integer-exponent unrolling rule per `julia/research/jump-formula-term-probe.md`:
a `pow(base, LitFloat(n))` with non-negative integer-valued `n ≤ 2`
unrolls to repeated `*`, so the result is `AffExpr` / `QuadExpr` rather
than `NonlinearExpr`. Only `n=2` actually buys QuadExpr — JuMP's MOI
core types stop at quadratic, so `n ≥ 3` would still promote to
NonlinearExpr. Setting `smart_pow=false` is useful for sibling
institutions targeting solvers that accept NonlinearExpr (Ipopt).
"""
function formula_to_jump(t::EigeniusMirror.FormulaTerm_Var, env; smart_pow::Bool = true)
    haskey(env, t.name) ||
        error("EigeniusJuMPHiGHS: unbound Var `$(t.name)` (not in problem.variable_names)")
    return env[t.name]
end

formula_to_jump(t::EigeniusMirror.FormulaTerm_LitFloat, env; smart_pow::Bool = true) = t.value

formula_to_jump(t::EigeniusMirror.FormulaTerm_OpRef, env; smart_pow::Bool = true) =
    error("EigeniusJuMPHiGHS: bare OpRef `$(t.iri)` outside an App spine")

formula_to_jump(t::EigeniusMirror.FormulaTerm_Lam, env; smart_pow::Bool = true) =
    error("EigeniusJuMPHiGHS: Lam not supported (FormulaTerm should be flat for solver-DSL targets)")

formula_to_jump(t::EigeniusMirror.FormulaTerm_Pi, env; smart_pow::Bool = true) =
    error("EigeniusJuMPHiGHS: Pi not supported (FormulaTerm should be flat for solver-DSL targets)")

function formula_to_jump(t::EigeniusMirror.FormulaTerm_App, env; smart_pow::Bool = true)
    # Walk left-spine to collect (OpRef, args...) — same shape as
    # EigeniusDiffEq's formula_to_value.
    cursor = t
    spine = Any[]
    while cursor isa EigeniusMirror.FormulaTerm_App
        push!(spine, cursor.arg)
        cursor = cursor.head
    end
    cursor isa EigeniusMirror.FormulaTerm_OpRef ||
        error("EigeniusJuMPHiGHS: spine root is not OpRef, got $(typeof(cursor))")
    haskey(_OP_TABLE, cursor.iri) ||
        error("EigeniusJuMPHiGHS: operator `$(cursor.iri)` not in `_OP_TABLE`; extend the catalog")

    # Smart-pow: integer-valued LitFloat exponents on `pow` unroll to
    # repeated multiplication so the result lands in QuadExpr territory
    # (HiGHS-acceptable) rather than NonlinearExpr (HiGHS-rejecting).
    # Only n ≤ 2 actually buys QuadExpr; higher integer exponents would
    # promote to NonlinearExpr again, so the unroll is bounded.
    if smart_pow && cursor.iri == OP_POW && length(spine) == 2
        # `spine` is reversed — index 1 is the rightmost arg = exponent.
        base_ft = spine[2]
        exp_ft = spine[1]
        if exp_ft isa EigeniusMirror.FormulaTerm_LitFloat
            v = exp_ft.value
            if isinteger(v) && v >= 0 && v <= 2
                base_v = formula_to_jump(base_ft, env; smart_pow = smart_pow)
                n = Int(v)
                n == 0 && return 1.0
                n == 1 && return base_v
                # n == 2
                return base_v * base_v
            end
        end
    end

    args = reverse([formula_to_jump(a, env; smart_pow = smart_pow) for a in spine])
    return _OP_TABLE[cursor.iri](args...)
end

# ─── Sense + relation interpreters ──────────────────────────────────────

function _jump_sense(s::AbstractString)
    if s == "Min"
        return MOI.MIN_SENSE
    elseif s == "Max"
        return MOI.MAX_SENSE
    else
        error("EigeniusJuMPHiGHS: unknown optimisation sense `$s` (expected `Min` or `Max`)")
    end
end

function _add_constraint!(model, lhs_expr, relation, rhs::Float64)
    if relation isa EigeniusMirror.ConstraintRelation_LE
        @constraint(model, lhs_expr <= rhs)
    elseif relation isa EigeniusMirror.ConstraintRelation_GE
        @constraint(model, lhs_expr >= rhs)
    elseif relation isa EigeniusMirror.ConstraintRelation_EQ
        @constraint(model, lhs_expr == rhs)
    else
        error("EigeniusJuMPHiGHS: unknown ConstraintRelation `$(typeof(relation))`")
    end
end

# ─── Model build ────────────────────────────────────────────────────────

"""
    build_model(problem::OptimisationProblem) -> (model, var_refs)

Materialise a `JuMP.Model(HiGHS.Optimizer)` from a chain-typed
OptimisationProblem. Returns the model plus the
`Dict{String, VariableRef}` map of variable names to their JuMP refs.
Throws on any structural error (unknown operator, malformed relation,
bound naming an unknown variable). Caller is responsible for `optimize!`
+ result extraction.
"""
function build_model(problem::EigeniusMirror.OptimisationProblem; smart_pow::Bool = true)
    model = Model(HiGHS.Optimizer)
    set_silent(model)

    var_refs = Dict{String, VariableRef}()
    for name in problem.variable_names
        v = @variable(model, base_name = name)
        var_refs[name] = v
    end

    # Apply per-variable bounds (recommended field — may be missing).
    if problem.variable_bounds !== nothing
        for vb in problem.variable_bounds
            haskey(var_refs, vb.variable_name) ||
                error("EigeniusJuMPHiGHS: VariableBound references unknown variable `$(vb.variable_name)`")
            x = var_refs[vb.variable_name]
            vb.lower !== nothing && set_lower_bound(x, vb.lower)
            vb.upper !== nothing && set_upper_bound(x, vb.upper)
        end
    end

    env = Dict{String, Any}(name => x for (name, x) in var_refs)

    obj_expr = formula_to_jump(problem.objective, env; smart_pow = smart_pow)
    sense = _jump_sense(problem.sense)
    @objective(model, sense, obj_expr)

    # `constraints` is recommended (an unconstrained problem with only
    # variable bounds is well-posed). `nothing` and the empty list both
    # mean "no algebraic constraints"; only iterate when there are some.
    if problem.constraints !== nothing
        for cstr in problem.constraints
            lhs_expr = formula_to_jump(cstr.lhs, env; smart_pow = smart_pow)
            _add_constraint!(model, lhs_expr, cstr.relation, Float64(cstr.rhs))
        end
    end

    return (model, var_refs)
end

# ─── solve_problem (OnDemand) ───────────────────────────────────────────

const _DEFAULT_ABSTOL = 1e-6
const _DEFAULT_RELTOL = 1e-6

"""
    solve_problem(problem::OptimisationProblem) -> OptimisesTo

Build the JuMP model from `problem`, optimise via HiGHS, and reify the
result as a chain-typed `OptimisesTo` resource carrying the resolved
objective value, the per-variable solution vector, the JuMP termination
status as a string, and default tolerances (`abstol = reltol = 1e-6`)
so a downstream AutoOnLoad re-validation gate fires cleanly without the
caller having to specify tolerances explicitly.
"""
function solve_problem(problem::EigeniusMirror.OptimisationProblem)
    (model, var_refs) = build_model(problem)
    optimize!(model)
    status = termination_status(model)

    # Read solution if the solver landed on a primal optimum (or a
    # near-equivalent — `LOCALLY_SOLVED` is what NLP solvers return for
    # local optima, and JuMP also emits `ALMOST_OPTIMAL` /
    # `ALMOST_LOCALLY_SOLVED` when tolerances were tight relative to
    # primal residuals; we treat all of these as "has a primal solution"
    # for read-back purposes, matching `primal_status(model) == FEASIBLE_POINT`).
    has_primal = (
        status == MOI.OPTIMAL ||
        status == MOI.LOCALLY_SOLVED ||
        status == MOI.ALMOST_OPTIMAL ||
        status == MOI.ALMOST_LOCALLY_SOLVED
    )
    obj_val = has_primal ? objective_value(model) : NaN
    var_vals = if has_primal
        [Float64(value(var_refs[name])) for name in problem.variable_names]
    else
        fill(NaN, length(problem.variable_names))
    end

    return EigeniusMirror.OptimisesTo(
        problem,
        string(status),
        obj_val,
        var_vals,
        _DEFAULT_ABSTOL,
        _DEFAULT_RELTOL,
    )
end

# ─── validate_optimum (AutoOnLoad) ──────────────────────────────────────

"""
    validate_optimum(claim::OptimisesTo) -> Verdict

Re-solve the claimed problem and compare the resolved
(termination_status, objective_value) pair against the claim. See the
module docstring for the full Verdict policy.
"""
function validate_optimum(claim::EigeniusMirror.OptimisesTo)
    problem = claim.problem
    n_vars = length(problem.variable_names)

    # Structural cross-check the chain validator's per-property rules
    # don't span: claim.variable_values length must match the problem's
    # variable count.
    if length(claim.variable_values) != n_vars
        return _verdict("Fails")
    end

    (model, _var_refs) = try
        build_model(problem)
    catch _e
        return _verdict("Fails")
    end

    try
        optimize!(model)
    catch _e
        return _verdict("Fails")
    end

    actual_status = termination_status(model)

    # Indeterminate solver states → Undecidable, regardless of what
    # the claim said the status was. The institution genuinely
    # couldn't decide.
    if actual_status == MOI.ITERATION_LIMIT ||
       actual_status == MOI.TIME_LIMIT ||
       actual_status == MOI.NUMERICAL_ERROR ||
       actual_status == MOI.SLOW_PROGRESS ||
       actual_status == MOI.MEMORY_LIMIT ||
       actual_status == MOI.NODE_LIMIT
        return _verdict("Undecidable")
    end

    if string(actual_status) != claim.termination_status
        return _verdict("Fails")
    end

    has_primal = (
        actual_status == MOI.OPTIMAL ||
        actual_status == MOI.LOCALLY_SOLVED ||
        actual_status == MOI.ALMOST_OPTIMAL ||
        actual_status == MOI.ALMOST_LOCALLY_SOLVED
    )

    if has_primal
        actual_obj = objective_value(model)
        diff = abs(claim.objective_value - actual_obj)
        tol = max(claim.abstol, claim.reltol * abs(actual_obj))
        if diff > tol
            return _verdict("Fails")
        end
        return _verdict("Holds")
    end

    # Status matched a non-optimal terminal state (INFEASIBLE,
    # DUAL_INFEASIBLE, INFEASIBLE_OR_UNBOUNDED, etc.). The institution
    # agrees with the claim that the problem has no feasible optimum;
    # there's no numeric optimum to compare.
    return _verdict("Holds")
end

# ─── reify_problem (Comorphism target-side reify) ───────────────────────

"""
    reify_problem(problem::OptimisationProblem) -> OptimisationProblem

Target-side reify for the `Symbolics -> JuMP` Comorphism (D14 §9.3
step 4). The comorphism's typed middle is the identity Lambda on
`OptimisationProblem` (D32 §6.2 generalised) — both ends of the
comorphism speak the same chain shape, with FormulaTerm carried
verbatim. The reify is therefore identity at the institution boundary;
the Julia function exists so the kernel's `Exp::InstitutionInvoke`
dispatch can route comorphism reify calls through the substrate
uniformly with `query` and `extract_typed`, rather than special-casing
no-op procedures kernel-side.

Sibling institutions targeting JuMP (e.g. an Ipopt-backed institution
for nonlinear programming) reuse the same `OptimisationProblem` shape
and the same identity reify; only the dispatch package and the
accepted constraint/objective complexity differ.
"""
function reify_problem(problem::EigeniusMirror.OptimisationProblem)
    return problem
end

end # module
