"""
    EigeniusDiffEq

Handler package for the DiffEq Eigenius institution (Phase 19g /
D27 §4.5 / D32 §6). Exports `validate_solution(s)` — the AutoOnLoad
gate's worker entry point for `OdeSolution` resources.

# Dispatch flow

1. The kernel commits an `OdeSolution(problem, algorithm, abstol,
   reltol, final_state)` resource on a chain that has the DiffEq
   institution installed.
2. `commit_with_validation` fires the AutoOnLoad QueryClass, which
   sends a `DispatchExternal` RPC to the orchestrator carrying the
   `OdeSolution` mirror struct (with the embedded `OdeProblem` and
   its FormulaTerm-typed `rhs` components) serialised to Eigon-CBOR.
3. The orchestrator routes the call through the substrate's Julia
   runtime; the worker decodes the input via the mirror's
   `decode_OdeSolution` codec — which transitively decodes the
   nested `problem::OdeProblem`, each `rhs::Vector{RhsComponent}`,
   and each `term::FormulaTerm` (via the per-ctor mirror structs).
4. This handler walks each FormulaTerm under a numerical
   interpreter driven by the `(state_names, parameter_names, t)`
   environment, builds a Julia `rhs!(du, u, p, t)` closure,
   constructs `ODEProblem(rhs!, u0, tspan, p)`, calls `solve` with
   the claimed algorithm + tolerances, and per-component-compares
   the integrator's final state against the claim within tolerance.
5. Returns a `Verdict` Dict — `Holds` on a successful re-integration
   that matches; `Fails` on a refuted claim or solver error;
   `Undecidable` on indeterminate solver states.

# Why FormulaTerm-typed RHS

The previous v1 carried the RHS as a Julia source string and
`Core.eval`'d it. That worked but lost three things D32 wants:

1. **Validator type-checking at commit.** A FormulaTerm value gets
   structurally validated against the chain's ctor schema; a Julia
   source string sails through unchecked.
2. **Cross-institution comorphism.** A `Symbolics.SymbolicExpression`
   carries a FormulaTerm; with DiffEq also speaking FormulaTerm, a
   `Symbolics → DiffEq` comorphism is the identity on FormulaTerm
   (per D32 §6.2). Source strings broke that pairing.
3. **Interval-extension reuse.** `EigeniusIntervals.formula_to_interval`
   walks FormulaTerm under interval semantics. The future
   `DiffEq → IntervalArithmetic` comorphism that bounds an
   integrated trajectory reuses that machinery — only because
   DiffEq's RHS is *the same FormulaTerm shape* the interval
   institution already understands.

# Verdict policy

- `Holds`: `successful_retcode(sol)` and per-component
  `|claim_i - actual_i| ≤ max(abstol, reltol * |actual_i|)`.
- `Fails`:
   - structural mismatch (ICs, parameters, final_state lengths
     don't match `state_names` / `parameter_names`);
   - unknown algorithm name;
   - unknown operator IRI in any FormulaTerm;
   - solver returns `Failure` / `Unstable` / `InitialFailure`;
   - per-component tolerance check fails on any axis.
- `Undecidable`: solver returns indeterminate states like `MaxIters`,
  `ConvergenceFailure`, `DtNaN`, `Stalled` — the institution can't
  reach a binary answer with the configured budget.

The asymmetry is deliberate: `Fails` means "the institution
disagrees with the claim"; `Undecidable` means "the institution
can't decide either way."
"""
module EigeniusDiffEq

using OrdinaryDiffEq
using SciMLBase: successful_retcode, ReturnCode
using EigeniusMirror

export validate_solution, reify_problem

const VERDICT_CLASS_IRI = "urn:eigenius:institution:Verdict"
const IS_A_PROP = "urn:eigenius:core:is_a"
const CTOR_NAME_PROP = "urn:eigenius:core:ctor_name"

_verdict(ctor::AbstractString) = Dict{String, Any}(
    IS_A_PROP => [VERDICT_CLASS_IRI],
    CTOR_NAME_PROP => ctor,
)

# ─── Algorithm registry ─────────────────────────────────────────────────

const _ALG_REGISTRY = Dict{String, Any}(
    "Tsit5" => Tsit5(),
    "Vern9" => Vern9(),
    "Rosenbrock23" => Rosenbrock23(),
    "Rodas5" => Rodas5(),
    "Rodas5P" => Rodas5P(),
    "QNDF" => QNDF(),
    "FBDF" => FBDF(),
)

# ─── Operator catalog ───────────────────────────────────────────────────
#
# IRI → Julia function. Mirrors the `_OP_FN` map in
# EigeniusSymbolics + the `_OP_INTERVAL` map in EigeniusIntervals;
# duplicated rather than shared because each institution interprets
# operators in its own arithmetic (Symbolics' `Num`, IntervalArithmetic's
# `Interval`, DiffEq's `Float64` here). The chain-side operator
# catalog is the same in all three; the per-institution mappings
# parallel each other.

const _OP_NUMERIC = Dict{String, Function}(
    "urn:eigenius:formulas:ops:add" => +,
    "urn:eigenius:formulas:ops:sub" => -,
    "urn:eigenius:formulas:ops:mul" => *,
    "urn:eigenius:formulas:ops:div" => /,
    "urn:eigenius:formulas:ops:pow" => ^,
    "urn:eigenius:formulas:ops:neg" => -,
    "urn:eigenius:formulas:ops:exp" => exp,
    "urn:eigenius:formulas:ops:log" => log,
    "urn:eigenius:formulas:ops:sin" => sin,
    "urn:eigenius:formulas:ops:cos" => cos,
    "urn:eigenius:formulas:ops:tan" => tan,
    "urn:eigenius:formulas:ops:sqrt" => sqrt,
    "urn:eigenius:formulas:ops:abs" => abs,
)

# ─── FormulaTerm numerical interpreter ──────────────────────────────────

"""
    formula_to_value(t::FormulaTerm, env::Dict{String, Float64}) -> Float64

Walk a FormulaTerm under numerical-Float64 semantics with `env`
binding state/parameter/time names to current values. Recursive in
the same shape `EigeniusIntervals.formula_to_interval` and
`EigeniusSymbolics.formula_to_num` use — the institution-specific
arithmetic is the only thing that varies between them.

Free `Var(name)` not present in `env` raises an error rather than
silently zeroing — a malformed FormulaTerm should refute the claim
loudly, not produce a wrong numerical answer.
"""
formula_to_value(t::EigeniusMirror.FormulaTerm_Var, env) =
    haskey(env, t.name) ? env[t.name] :
    error("EigeniusDiffEq: unbound variable `$(t.name)` in RHS — must appear in state_names, parameter_names, or be the special name `t`")

formula_to_value(t::EigeniusMirror.FormulaTerm_LitFloat, env) = t.value

function formula_to_value(t::EigeniusMirror.FormulaTerm_App, env)
    spine = Any[]
    cursor = t
    while cursor isa EigeniusMirror.FormulaTerm_App
        push!(spine, cursor.arg)
        cursor = cursor.head
    end
    cursor isa EigeniusMirror.FormulaTerm_OpRef ||
        error("EigeniusDiffEq: unsupported App head — expected OpRef, got $(typeof(cursor))")
    haskey(_OP_NUMERIC, cursor.iri) ||
        error("EigeniusDiffEq: operator `$(cursor.iri)` not in numeric catalog — extend `_OP_NUMERIC`")
    args = reverse([formula_to_value(a, env) for a in spine])
    return _OP_NUMERIC[cursor.iri](args...)
end

formula_to_value(t::EigeniusMirror.FormulaTerm_OpRef, env) =
    error("EigeniusDiffEq: bare OpRef `$(t.iri)` outside an App spine is unsupported")

formula_to_value(t::EigeniusMirror.FormulaTerm_Lam, env) =
    error("EigeniusDiffEq: Lam binder is unsupported in numerical RHS interpretation — typed binders belong on operator signatures, not value-side terms")

formula_to_value(t::EigeniusMirror.FormulaTerm_Pi, env) =
    error("EigeniusDiffEq: Pi binder is unsupported in numerical RHS interpretation")

# ─── Closure builder ────────────────────────────────────────────────────

"""
    build_rhs_closure(rhs_components, state_names, parameter_names) -> rhs!

Build an in-place `rhs!(du, u, p, t)` closure that walks each
FormulaTerm under `formula_to_value` against the env populated from
the current `(u, p, t)`. The closure captures the FormulaTerm trees
and name-vectors once at construction time so per-step calls don't
rebuild the env structure — just refresh the values.
"""
function build_rhs_closure(
    rhs_components::AbstractVector{<:EigeniusMirror.AbstractRhsComponent},
    state_names::Vector{String},
    parameter_names::Vector{String},
)
    n = length(rhs_components)
    env = Dict{String, Float64}()
    function rhs!(du, u, p, t)
        @inbounds for (i, name) in enumerate(state_names)
            env[name] = u[i]
        end
        @inbounds for (i, name) in enumerate(parameter_names)
            env[name] = p[i]
        end
        env["t"] = t
        @inbounds for i in 1:n
            du[i] = formula_to_value(rhs_components[i].term, env)
        end
        return nothing
    end
    return rhs!
end

# ─── The handler ────────────────────────────────────────────────────────

"""
    validate_solution(s::OdeSolution) -> Verdict

Re-integrate the referenced `OdeProblem` with the claimed
algorithm + tolerances and verify the institution arrives at the
claimed `final_state`. See module docstring for the full Verdict
policy.
"""
function validate_solution(s::EigeniusMirror.OdeSolution)
    prob_def = s.problem

    state_names = collect(String, prob_def.state_names)
    param_names = collect(String, prob_def.parameter_names)
    n_states = length(state_names)
    n_params = length(param_names)

    # Structural cross-checks — caught here rather than at the
    # validator because they involve cross-property arithmetic
    # the chain-validator's per-property type-check rules don't span.
    if length(prob_def.initial_conditions) != n_states ||
       length(prob_def.parameters) != n_params ||
       length(prob_def.rhs) != n_states ||
       length(s.final_state) != n_states
        return _verdict("Fails")
    end

    if !haskey(_ALG_REGISTRY, s.algorithm)
        return _verdict("Fails")
    end
    alg = _ALG_REGISTRY[s.algorithm]

    rhs! = try
        build_rhs_closure(prob_def.rhs, state_names, param_names)
    catch _e
        # Malformed FormulaTerm (unknown operator IRI, etc.) — refute.
        return _verdict("Fails")
    end

    u0 = Float64.(prob_def.initial_conditions)
    p = Float64.(prob_def.parameters)
    tspan = (Float64(prob_def.time_span_start), Float64(prob_def.time_span_end))

    prob = ODEProblem(rhs!, u0, tspan, p)
    sol = try
        solve(prob, alg; abstol = s.abstol, reltol = s.reltol)
    catch _e
        return _verdict("Fails")
    end

    rc = sol.retcode
    if rc == ReturnCode.MaxIters ||
       rc == ReturnCode.ConvergenceFailure ||
       rc == ReturnCode.DtNaN ||
       rc == ReturnCode.DtLessThanMin ||
       rc == ReturnCode.Stalled ||
       rc == ReturnCode.MaxNumSub ||
       rc == ReturnCode.MaxTime
        return _verdict("Undecidable")
    end
    if !successful_retcode(sol)
        return _verdict("Fails")
    end

    actual_final = sol.u[end]
    claimed_final = Float64.(s.final_state)
    if !_within_tolerance(claimed_final, actual_final, s.abstol, s.reltol)
        return _verdict("Fails")
    end

    return _verdict("Holds")
end

"""
    _within_tolerance(claim, actual, abstol, reltol) -> Bool

Per-component tolerance check matching OrdinaryDiffEq's own error
control: `|claim_i - actual_i| ≤ max(abstol, reltol * |actual_i|)`.
"""
function _within_tolerance(
    claim::AbstractVector{Float64},
    actual::AbstractVector{Float64},
    abstol::Real,
    reltol::Real,
)
    length(claim) == length(actual) || return false
    for (c, a) in zip(claim, actual)
        bound = max(abstol, reltol * abs(a))
        abs(c - a) <= bound || return false
    end
    return true
end

# ─── reify_problem (Comorphism target-side reify) ───────────────────────

"""
    reify_problem(problem::OdeProblem) -> OdeProblem

Target-side reify for the `Catalyst -> DiffEq` Comorphism (D14 §9.3
step 4). The comorphism's typed middle is the identity Lambda on
`OdeProblem` (D32 §6.2 generalised) — Catalyst's `compile_to_ode`
produces a chain-typed `OdeProblem` with FormulaTerm RHS, and the
DiffEq side speaks the same shape, so the reify is identity at the
boundary. The Julia function exists so the kernel's
`Exp::InstitutionInvoke` dispatch can route comorphism reify calls
through the substrate uniformly with `query` and `extract_typed`,
rather than special-casing no-op procedures kernel-side.

Sibling institutions targeting DiffEq for non-ODE problems (DDE, SDE,
DAE) can introduce their own ImportFormats with non-identity
reifies; this one stays identity because `OdeProblem` is the
shared chain shape between Catalyst's compile output and DiffEq's
solver entry.
"""
function reify_problem(problem::EigeniusMirror.OdeProblem)
    return problem
end

end # module
