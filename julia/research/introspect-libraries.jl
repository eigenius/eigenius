# Phase-19 ontology research — introspect the actual API surface of each
# Julia library we're wrapping as an Eigenius institution. Writes a
# markdown report to `api-survey.md` next to this script.
#
# Run with: julia --project=. introspect-libraries.jl
#
# First run takes 10-30 minutes (Pkg downloads + precompilation across
# the SciML stack). Subsequent runs are fast — the env caches.

import Pkg
Pkg.activate(@__DIR__)

# --- Dependency list ---------------------------------------------------------
# Kept minimal: one core package per institution + a default solver / driver
# where the institution needs one. Companion packages added only when they
# carry vocabulary the ontology is going to mint IRIs against
# (IntervalRootFinding for `ContainsRoot`, SciMLBase for `ReturnCode`,
# Latexify for `to_latex`, etc.).

const DEPS = [
    "IntervalArithmetic",
    "IntervalRootFinding",
    "Symbolics",
    "ModelingToolkit",
    "SymbolicUtils",
    "Latexify",
    "JuMP",
    "MathOptInterface",
    "HiGHS",                   # default JuMP solver for probes
    "Catalyst",
    "OrdinaryDiffEq",
    "SciMLBase",
]

println("Installing dependencies (this can take 10-30 minutes on first run)...")
Pkg.add(DEPS)
Pkg.precompile()

# Versions snapshot
status_io = IOBuffer()
Pkg.status(; io = status_io)
versions = String(take!(status_io))

using Dates
using IntervalArithmetic
using IntervalRootFinding
using Symbolics
using ModelingToolkit
using SymbolicUtils
using Latexify
using JuMP
using MathOptInterface
const MOI = MathOptInterface
using HiGHS
using Catalyst
using OrdinaryDiffEq
using SciMLBase

# --- Helpers -----------------------------------------------------------------

const OUT = open(joinpath(@__DIR__, "api-survey.md"), "w")

section(title) = println(OUT, "\n## ", title, "\n")
subsection(title) = println(OUT, "\n### ", title, "\n")
log_line(s) = println(OUT, s)
log_block(s) = (println(OUT, "```"); println(OUT, s); println(OUT, "```"))

"""
List the names a module exports. Useful for "what's the public surface?"
"""
function exports(m)
    sort!([string(n) for n in names(m; all=false, imported=false)])
end

"""
Try calling `f(args...)`; report value and type, or the error class.
"""
function probe(label, f, args...; kwargs...)
    try
        r = f(args...; kwargs...)
        log_line("- `$label` = `$(repr(r))` :: `$(typeof(r))`")
    catch e
        log_line("- `$label` → ERROR `$(typeof(e))`: $(sprint(showerror, e))")
    end
end

"""
Test whether a name exists in a module (exported or not).
"""
function has_name(m, sym)
    isdefined(m, sym)
end

# --- Header ------------------------------------------------------------------

println(OUT, "# Julia library API survey — Phase 19 ontology design\n")
println(OUT, "Generated $(now()) by `introspect-libraries.jl`. ",
        "Verifies API surface against live installed packages so the ontology ",
        "vocabulary does not drift from the libraries it wraps.\n")

section("Resolved versions")
log_block(versions)

# --- IntervalArithmetic ------------------------------------------------------

section("IntervalArithmetic.jl")

subsection("Module exports")
log_block(join(exports(IntervalArithmetic), "\n"))

subsection("Construction forms")
probe("interval(1.0, 2.0)", interval, 1.0, 2.0)
probe("Interval(1.0, 2.0)",
      (lo, hi) -> @eval(IntervalArithmetic, Interval($lo, $hi)),
      1.0, 2.0)
probe("0.0 .. 1.0", () -> 0.0 .. 1.0)

subsection("Endpoint accessors (which exist?)")
let I = interval(1.0, 2.0)
    for sym in [:inf, :sup, :infimum, :supremum, :lower, :upper, :lo, :hi, :midpoint, :diam, :radius]
        if isdefined(IntervalArithmetic, sym)
            try
                v = getfield(IntervalArithmetic, sym)(I)
                log_line("- `IntervalArithmetic.$sym(I)` = `$(repr(v))` :: `$(typeof(v))`")
            catch e
                log_line("- `IntervalArithmetic.$sym(I)` exists but errored: `$(typeof(e))`")
            end
        else
            log_line("- `IntervalArithmetic.$sym` — NOT DEFINED")
        end
    end
end

subsection("Empty / NaI / decoration")
for sym in [:emptyinterval, :isempty, :isnai, :isthin, :isfinite, :isbounded]
    log_line("- `IntervalArithmetic.$sym` defined: $(isdefined(IntervalArithmetic, sym))")
end
probe("emptyinterval()", () -> emptyinterval())

subsection("Function evaluation over an interval")
let f = x -> 42.0 / (42.0 + x)
    probe("f(interval(0.0, 1000.0))  where  f(x) = 42/(42+x)",
          () -> f(interval(0.0, 1000.0)))
end
probe("sin(interval(0, π))", () -> sin(interval(0.0, π)))
probe("exp(interval(-1.0, 1.0))", () -> exp(interval(-1.0, 1.0)))

subsection("Rounding-mode API surface")
for sym in [:setrounding, :IntervalRounding, :rounding, :setformat, :configure]
    log_line("- `IntervalArithmetic.$sym` defined: $(isdefined(IntervalArithmetic, sym))")
end

subsection("IntervalRootFinding — root-finding entry points")
log_block(join(exports(IntervalRootFinding), "\n"))
for sym in [:roots, :Krawczyk, :Newton, :Bisection, :Root]
    log_line("- `IntervalRootFinding.$sym` defined: $(isdefined(IntervalRootFinding, sym))")
end

# --- Symbolics + ModelingToolkit + SymbolicUtils -----------------------------

section("Symbolics.jl + ModelingToolkit + SymbolicUtils")

subsection("Symbolics module exports (head)")
let xs = exports(Symbolics)
    log_block(join(xs[1:min(end, 80)], "\n"))
    log_line("\n($(length(xs)) total exports.)")
end

subsection("Variable / equation idiom")
# Qualify the macros — `@variables` is exported by Symbolics, JuMP, MTK,
# Catalyst, and ModelingToolkitBase. `@parameters` does NOT live in
# Symbolics — it's MTK / ModelingToolkitBase. (Useful ontology signal:
# parameter declaration is MTK vocabulary.)
Symbolics.@variables x y t
log_line("- `Symbolics.@variables x y t` worked. Type of x: `$(typeof(x))`, name: `$x`")
let sym_p = Symbol("@parameters")
    log_line("- `Symbolics.@parameters` defined: $(isdefined(Symbolics, sym_p))")
    log_line("- `ModelingToolkit.@parameters` defined: $(isdefined(ModelingToolkit, sym_p))")
end
ModelingToolkit.@parameters p q
log_line("- `ModelingToolkit.@parameters p q` worked. Type of p: `$(typeof(p))`")
let eq = (x^2 + y ~ 0)
    log_line("- `x^2 + y ~ 0` →  `$(typeof(eq))` with fields `$(fieldnames(typeof(eq)))`")
end

subsection("Simplification, expansion, substitution")
for sym in [:simplify, :expand, :substitute, :get_variables, :derivative,
            :jacobian, :hessian, :gradient, :polynomial_coeffs]
    log_line("- `Symbolics.$sym` defined: $(isdefined(Symbolics, sym))")
end

subsection("Differential operator")
for sym in [:Differential, :expand_derivatives]
    log_line("- `Symbolics.$sym` defined: $(isdefined(Symbolics, sym))")
end

subsection("ModelingToolkit naming")
for sym in [:ODESystem, :NonlinearSystem, :OptimizationSystem,
            :unknowns, :states,                 # `states` was renamed
            :equations, :parameters, :observed,
            :structural_simplify, :complete]
    log_line("- `ModelingToolkit.$sym` defined: $(isdefined(ModelingToolkit, sym))")
end

subsection("SymbolicUtils internals (BasicSymbolic, RuleSet, @rule)")
for sym in [:BasicSymbolic, :Term, :Sym, :Add, :Mul, :Pow, :RuleSet, Symbol("@rule")]
    log_line("- `SymbolicUtils.$sym` defined: $(isdefined(SymbolicUtils, sym))")
end

subsection("Latexify")
log_line("- `Latexify.latexify` defined: $(isdefined(Latexify, :latexify))")

# --- JuMP --------------------------------------------------------------------

section("JuMP")

subsection("Status / solution API surface")
for sym in [:Model, :optimize!, :termination_status, :primal_status, :dual_status,
            :is_solved_and_feasible, :has_values, :has_duals,
            :objective_value, :objective_bound, :value, :dual, :reduced_cost, :shadow_price,
            :lp_sensitivity_report, :compute_conflict!, :copy_conflict,
            :result_count, :set_time_limit_sec]
    log_line("- `JuMP.$sym` defined: $(isdefined(JuMP, sym))")
end

subsection("MOI termination/result-status enum values")
log_line("Termination status enumerators (subset):")
for sym in [:OPTIMAL, :INFEASIBLE, :DUAL_INFEASIBLE, :LOCALLY_SOLVED,
            :LOCALLY_INFEASIBLE, :INFEASIBLE_OR_UNBOUNDED,
            :ALMOST_OPTIMAL, :TIME_LIMIT, :ITERATION_LIMIT,
            :NUMERICAL_ERROR, :INVALID_MODEL, :OTHER_ERROR, :OPTIMIZE_NOT_CALLED]
    log_line("  - `MOI.$sym` defined: $(isdefined(MOI, sym))")
end
log_line("Result status enumerators:")
for sym in [:NO_SOLUTION, :FEASIBLE_POINT, :NEARLY_FEASIBLE_POINT,
            :INFEASIBILITY_CERTIFICATE, :NEARLY_INFEASIBILITY_CERTIFICATE,
            :REDUCTION_CERTIFICATE, :UNKNOWN_RESULT_STATUS]
    log_line("  - `MOI.$sym` defined: $(isdefined(MOI, sym))")
end

subsection("End-to-end LP probe (HiGHS)")
let m = JuMP.Model(HiGHS.Optimizer)
    JuMP.set_silent(m)
    JuMP.@variable(m, x >= 0)
    JuMP.@variable(m, y >= 0)
    JuMP.@constraint(m, c1, x + y <= 4)
    JuMP.@constraint(m, c2, x + 2y <= 6)
    JuMP.@objective(m, Max, 3x + 5y)
    JuMP.optimize!(m)
    log_line("- `termination_status(m)` = `$(termination_status(m))`")
    log_line("- `primal_status(m)` = `$(primal_status(m))`")
    log_line("- `dual_status(m)` = `$(dual_status(m))`")
    log_line("- `is_solved_and_feasible(m)` = `$(is_solved_and_feasible(m))`")
    log_line("- `objective_value(m)` = `$(objective_value(m))`")
    log_line("- `value(x)` = `$(value(x))`, `value(y)` = `$(value(y))`")
    log_line("- `dual(c1)` = `$(dual(c1))`, `dual(c2)` = `$(dual(c2))`")
    try
        rep = JuMP.lp_sensitivity_report(m)
        log_line("- `lp_sensitivity_report(m)` returned `$(typeof(rep))`")
    catch e
        log_line("- `lp_sensitivity_report(m)` errored: $(typeof(e))")
    end
end

# --- Catalyst ---------------------------------------------------------------

section("Catalyst.jl")

subsection("Module exports (head)")
let xs = exports(Catalyst)
    log_block(join(xs[1:min(end, 80)], "\n"))
    log_line("\n($(length(xs)) total exports.)")
end

subsection("Network construction + accessors")
rn = Catalyst.@reaction_network begin
    k1, A + B --> C
    k2, C --> A + B
end
log_line("- `@reaction_network` returns: `$(typeof(rn))`")
for sym in [:species, :parameters, :reactions, :equations, :unknowns,
            :netstoichmat, :substoichmat, :prodstoichmat,
            :conservationlaws, :conservedequations, :conservationlaw_constants,
            :complexes, :complexstoichmat, :reactioncomplexes]
    if isdefined(Catalyst, sym)
        log_line("- `Catalyst.$sym` exported & defined")
    else
        log_line("- `Catalyst.$sym` NOT defined in Catalyst")
    end
end

subsection("Conservation laws probe")
try
    cl = Catalyst.conservationlaws(rn)
    log_line("- `conservationlaws(rn)` = `$(repr(cl))` :: `$(typeof(cl))`")
catch e
    log_line("- `conservationlaws(rn)` errored: $(typeof(e)): $(sprint(showerror,e))")
end

subsection("Compilation paths to dynamics")
for sym in [:ODEProblem, :SDEProblem, :JumpProblem, :DiscreteProblem, :SteadyStateProblem]
    log_line("- `Catalyst.$sym` reachable via Catalyst: $(isdefined(Catalyst, sym))")
end
try
    osys = Catalyst.convert(ODESystem, rn)
    log_line("- `convert(ODESystem, rn)` → `$(typeof(osys))`")
catch e
    log_line("- `convert(ODESystem, rn)` errored: $(typeof(e)): $(sprint(showerror,e))")
end

subsection("Deficiency-theorem support")
for sym in [:deficiency, :deficiencyzerotheorem, :deficiencyonetheorem,
            :isweaklyreversible, :iscomplexbalanced]
    log_line("- `Catalyst.$sym` defined: $(isdefined(Catalyst, sym))")
end

# --- DiffEq / OrdinaryDiffEq / SciMLBase ------------------------------------

section("OrdinaryDiffEq + SciMLBase")

subsection("Problem / solution / algorithm types")
for (m, syms) in [
        (SciMLBase, [:ODEProblem, :ODESolution, :SteadyStateProblem,
                     :ReturnCode, :successful_retcode, :EnsembleProblem,
                     :remake]),
        (OrdinaryDiffEq, [:Tsit5, :Vern9, :Rosenbrock23, :Rodas5, :Rodas5P,
                          :QNDF, :FBDF,
                          :AutoTsit5, :AutoVern9])]
    for sym in syms
        log_line("- `$(nameof(m)).$sym` defined: $(isdefined(m, sym))")
    end
end

subsection("ReturnCode enumerators")
let rc_names = propertynames(SciMLBase.ReturnCode)
    log_block(join(string.(rc_names), "\n"))
end

subsection("End-to-end ODE probe (Tsit5)")
let f = (du, u, p, t) -> (du[1] = -p[1] * u[1])
    prob = ODEProblem(f, [1.0], (0.0, 1.0), [0.5])
    sol = OrdinaryDiffEq.solve(prob, Tsit5(); abstol=1e-8, reltol=1e-8)
    log_line("- `solve(prob, Tsit5())` returned `$(typeof(sol))`")
    log_line("- `SciMLBase.successful_retcode(sol)` = `$(SciMLBase.successful_retcode(sol))`")
    log_line("- `sol.retcode` = `$(sol.retcode)`")
    log_line("- `length(sol.t)` = `$(length(sol.t))`, `length(sol.u)` = `$(length(sol.u))`")
    log_line("- `sol(0.5)` (interpolation) = `$(sol(0.5))`")
    log_line("- `sol[1]` = `$(sol[1])` (NB: SciMLBase v3 changed `sol[i]` to AbstractArray indexing)")
    log_line("- ODESolution fields: `$(fieldnames(typeof(sol)))`")
end

# --- Done --------------------------------------------------------------------

close(OUT)
println("\nWrote api-survey.md to $(joinpath(@__DIR__, "api-survey.md"))")
