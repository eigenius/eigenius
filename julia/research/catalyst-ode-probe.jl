# Focused probe — what's the current Catalyst → ODE compilation entry
# point against Catalyst 16.1.1 + ModelingToolkit 11.24.1? D27 §4.4
# assumed `convert(ODESystem, rn)` but the introspection survey showed
# that errors. Determine the right path so the Catalyst→DiffEq
# Comorphism design is grounded.
#
# Run with: julia --project=. catalyst-ode-probe.jl
# (Pkg env is shared with introspect-libraries.jl; deps already installed.)

import Pkg
Pkg.activate(@__DIR__)

using Catalyst
using ModelingToolkit
using OrdinaryDiffEq
using SciMLBase

const OUT = open(joinpath(@__DIR__, "catalyst-ode-probe.md"), "w")

section(t) = println(OUT, "\n## ", t, "\n")
log(s) = println(OUT, s)
block(s) = (println(OUT, "```"); println(OUT, s); println(OUT, "```"))

println(OUT, "# Catalyst → ODE compilation probe\n")
println(OUT, "Catalyst 16.1.1, ModelingToolkit 11.24.1, OrdinaryDiffEq 6.111.0, SciMLBase 2.155.1.\n")

# Set up the kinase binding network from the Catalyst examples.json
rn = @reaction_network begin
    k_on,  C + K --> CK
    k_off, CK --> C + K
end

section("Network basics")
log("- typeof(rn) = `$(typeof(rn))`")
log("- species(rn) = `$(species(rn))`")
log("- parameters(rn) = `$(parameters(rn))`")
log("- length(reactions(rn)) = $(length(reactions(rn)))")
log("- length(unknowns(rn)) = $(length(unknowns(rn)))")
log("- length(equations(rn)) = $(length(equations(rn)))")

# u0 and p in the orderings species() / parameters() return
u0_vec = [100.0, 100.0, 0.0]   # C, K, CK
p_vec  = [0.01, 0.42]          # k_on, k_off
tspan  = (0.0, 5.0)

section("Direct ODEProblem(rn, u0, tspan, p) — D27's proposed v1 path")
try
    prob = ODEProblem(rn, u0_vec, tspan, p_vec)
    log("- WORKS: typeof(prob) = `$(typeof(prob))`")
    sol = OrdinaryDiffEq.solve(prob, Tsit5(); abstol=1e-8, reltol=1e-8)
    log("- solve(prob, Tsit5()): retcode = `$(sol.retcode)`, length(sol.t) = $(length(sol.t))")
    log("- final state sol.u[end] = `$(sol.u[end])`")
catch e
    log("- ERROR: `$(typeof(e))`: $(sprint(showerror, e))")
end

section("ODEProblem with map-form u0 / p (the symbolic-keyed alternative)")
try
    @unpack C, K, CK, k_on, k_off = rn
    u0_map = [C => 100.0, K => 100.0, CK => 0.0]
    p_map  = [k_on => 0.01, k_off => 0.42]
    prob = ODEProblem(rn, u0_map, tspan, p_map)
    log("- WORKS: typeof(prob) = `$(typeof(prob))`")
    sol = OrdinaryDiffEq.solve(prob, Tsit5(); abstol=1e-8, reltol=1e-8)
    log("- final state = `$(sol.u[end])`")
catch e
    log("- ERROR: `$(typeof(e))`: $(sprint(showerror, e))")
end

section("convert(ODESystem, rn) — D27 §4.4.4 sketch (known to error)")
try
    osys = convert(ODESystem, rn)
    log("- WORKS unexpectedly: typeof(osys) = `$(typeof(osys))`")
catch e
    log("- ERROR (expected): `$(typeof(e))`: $(sprint(showerror, e))")
end

section("Alternatives — try every Catalyst-or-MTK conversion that might work in 16/11")
for fn in [:complete, :flatten, :structural_simplify]
    if isdefined(Catalyst, fn) || isdefined(ModelingToolkit, fn)
        log("- `$fn` exists (in Catalyst: $(isdefined(Catalyst, fn)), MTK: $(isdefined(ModelingToolkit, fn)))")
    else
        log("- `$fn` NOT defined")
    end
end

# Try `complete(rn)` — MTK's preferred-since-v9 finalisation step
section("complete(rn) — MTK finalisation")
try
    rn_completed = complete(rn)
    log("- WORKS: typeof(complete(rn)) = `$(typeof(rn_completed))`")
    try
        prob = ODEProblem(rn_completed, u0_vec, tspan, p_vec)
        log("- ODEProblem(complete(rn), u0, tspan, p): typeof = `$(typeof(prob))`")
    catch e
        log("- ODEProblem on completed network: ERROR `$(typeof(e))`: $(sprint(showerror, e))")
    end
catch e
    log("- complete(rn) ERROR: `$(typeof(e))`: $(sprint(showerror, e))")
end

section("Look for Catalyst conversion entry points by name")
catalyst_export_names = [string(n) for n in names(Catalyst; all=false)]
ode_related = filter(s -> occursin(r"ode|System|convert|to_"i, s), catalyst_export_names)
block(join(sort(ode_related), "\n"))

# Try methods of ODEProblem that take a ReactionSystem
section("ODEProblem method signatures (filtered)")
ms = methods(ODEProblem)
log("Total methods: $(length(ms))")
for m in ms
    sig = string(m.sig)
    if occursin("ReactionSystem", sig) || occursin("Catalyst", sig)
        log("- $sig")
    end
end

section("equations(rn) shape")
eqs = equations(rn)
log("- typeof(equations(rn)) = `$(typeof(eqs))`")
log("- length = $(length(eqs))")
for (i, e) in enumerate(eqs)
    log("  [$i] $e")
end

close(OUT)
println("Wrote ", joinpath(@__DIR__, "catalyst-ode-probe.md"))
