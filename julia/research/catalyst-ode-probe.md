# Catalyst → ODE compilation probe

Catalyst 16.1.1, ModelingToolkit 11.24.1, OrdinaryDiffEq 6.111.0, SciMLBase 2.155.1.


## Network basics

- typeof(rn) = `ReactionSystem{Catalyst.NetworkProperties{Int64, SymbolicUtils.BasicSymbolicImpl.var"typeof(BasicSymbolicImpl)"{SymReal}}}`
- species(rn) = `SymbolicUtils.BasicSymbolicImpl.var"typeof(BasicSymbolicImpl)"{SymReal}[C(t), K(t), CK(t)]`
- parameters(rn) = `SymbolicUtils.BasicSymbolicImpl.var"typeof(BasicSymbolicImpl)"{SymReal}[k_on, k_off]`
- length(reactions(rn)) = 2
- length(unknowns(rn)) = 3
- length(equations(rn)) = 2

## Direct ODEProblem(rn, u0, tspan, p) — D27's proposed v1 path

- ERROR: `BoundsError`: BoundsError: attempt to access Float64 at index [2]

## ODEProblem with map-form u0 / p (the symbolic-keyed alternative)

- WORKS: typeof(prob) = `ODEProblem{Vector{Float64}, Tuple{Float64, Float64}, true, MTKParameters{Vector{Float64}, Vector{Float64}, Tuple{}, Tuple{}, Tuple{}, Tuple{}}, ODEFunction{true, SciMLBase.AutoSpecialize, ModelingToolkitBase.GeneratedFunctionWrapper{(2, 3, true), RuntimeGeneratedFunctions.RuntimeGeneratedFunction{(:__mtk_arg_1, :___mtkparameters___, :t), ModelingToolkitBase.var"#_RGF_ModTag", ModelingToolkitBase.var"#_RGF_ModTag", (0x05d30504, 0xf137b772, 0x2a71ed98, 0x585a50eb, 0x43743d5f), Nothing}, RuntimeGeneratedFunctions.RuntimeGeneratedFunction{(:ˍ₋out, :__mtk_arg_1, :___mtkparameters___, :t), ModelingToolkitBase.var"#_RGF_ModTag", ModelingToolkitBase.var"#_RGF_ModTag", (0x9cd3ea8e, 0x19790a06, 0x65351ae7, 0x7a49dea4, 0x2d474ca2), Nothing}}, LinearAlgebra.UniformScaling{Bool}, Nothing, Nothing, Nothing, Nothing, Nothing, Nothing, Nothing, Nothing, Nothing, Nothing, Nothing, Nothing, ModelingToolkitBase.ObservedFunctionCache{System, Nothing}, Nothing, System, Union{Nothing, SciMLBase.OverrideInitData}, Union{Nothing, SciMLBase.ODENLStepData}}, Base.Pairs{Symbol, Union{}, Nothing, @NamedTuple{}}, SciMLBase.StandardODEProblem}`
- final state = `[47.1667980469613, 47.1667980469613, 52.8332019530387]`

## convert(ODESystem, rn) — D27 §4.4.4 sketch (known to error)

- ERROR (expected): `MethodError`: MethodError: Cannot `convert` an object of type ReactionSystem{Catalyst.NetworkProperties{Int64, SymbolicUtils.BasicSymbolicImpl.var"typeof(BasicSymbolicImpl)"{SymReal}}} to an object of type ModelingToolkitBase.IntermediateDeprecationSystem
The function `convert` exists, but no method is defined for this combination of argument types.

Closest candidates are:
  ModelingToolkitBase.IntermediateDeprecationSystem(::Any...; kwargs...)
   @ ModelingToolkitBase ~/.julia/packages/ModelingToolkitBase/7AL5w/src/deprecations.jl:20
  convert(::Type{T}, !Matched::T) where T
   @ Base Base_compiler.jl:133


## Alternatives — try every Catalyst-or-MTK conversion that might work in 16/11

- `complete` exists (in Catalyst: true, MTK: true)
- `flatten` exists (in Catalyst: true, MTK: true)
- `structural_simplify` exists (in Catalyst: true, MTK: true)

## complete(rn) — MTK finalisation

- WORKS: typeof(complete(rn)) = `ReactionSystem{Catalyst.NetworkProperties{Int64, SymbolicUtils.BasicSymbolicImpl.var"typeof(BasicSymbolicImpl)"{SymReal}}}`
- ODEProblem on completed network: ERROR `BoundsError`: BoundsError: attempt to access Float64 at index [2]

## Look for Catalyst conversion entry points by name

```
DiscreteSpaceReactionSystem
DiscreteSystem
ImplicitDiscreteSystem
JumpSystem
MiscSystemData
ModelingToolkitBase
NonlinearSystem
ODEFunction
ODEProblem
ODESystem
OptimizationSystem
PDESystem
ReactionSystem
SDESystem
SymbolicLinearODE
System
balance_system
convert_system_indepvar
debug_system
fractional_to_ordinary
generate_initializesystem
hybrid_model
jump_model
linear_fractional_to_ordinary
make_si_ode
modelingtoolkitize
noise_to_brownians
ode_model
oderatelaw
parse_expr_to_symbolic
save_reactionsystem
sde_model
solve_linear_ode_system
ss_ode_model
symbolic_solve_ode
symbolics_to_sympy
symbolics_to_sympy_pythoncall
sympy_ode_solve
sympy_pythoncall_ode_solve
sympy_pythoncall_to_symbolics
sympy_to_symbolics
```

## ODEProblem method signatures (filtered)

Total methods: 24
- Tuple{Type{ODEProblem}, ReactionSystem, Any, Any}
- Tuple{Type{ODEProblem}, ReactionSystem, Any, Any, Any, Vararg{Any}}
- Tuple{Type{ODEProblem}, DiscreteSpaceReactionSystem, Any, Any}
- Tuple{Type{ODEProblem}, DiscreteSpaceReactionSystem, Any, Any, Any, Vararg{Any}}

## equations(rn) shape

- typeof(equations(rn)) = `Vector{Union{Equation, Reaction}}`
- length = 2
  [1] k_on, C + K --> CK
  [2] k_off, CK --> C + K
