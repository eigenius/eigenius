# Julia library API survey — Phase 19 ontology design

Generated 2026-05-03T19:45:02.855 by `introspect-libraries.jl`. Verifies API surface against live installed packages so the ontology vocabulary does not drift from the libraries it wraps.


## Resolved versions

```
Status `~/src/eigenius/julia/research/Project.toml`
  [479239e8] Catalyst v16.1.1
  [87dc4568] HiGHS v1.23.0
  [d1acc4aa] IntervalArithmetic v1.0.8
  [d2bf35a9] IntervalRootFinding v0.6.3
  [4076af6c] JuMP v1.30.1
  [23fbe1c1] Latexify v0.16.10
  [b8f27783] MathOptInterface v1.51.0
  [961ee093] ModelingToolkit v11.24.1
⌃ [1dea7af3] OrdinaryDiffEq v6.111.0
⌅ [0bca4576] SciMLBase v2.155.1
  [d1185830] SymbolicUtils v4.25.2
  [0c5d862f] Symbolics v7.21.0
Info Packages marked with ⌃ and ⌅ have new versions available. Those with ⌃ may be upgradable, but those with ⌅ are restricted by compatibility constraints from upgrading. To see why use `status --outdated`

```

## IntervalArithmetic.jl


### Module exports

```
@I_str
@exact
@interval
BareInterval
ComplexI
Constant
Domain
ExactReal
Interval
IntervalArithmetic
Overlap
Piecewise
RealIntervalType
RealOrComplexI
bareinterval
bisect
bounds
cancelminus
cancelplus
com
dac
decoration
def
diam
discontinuities
dist
domains
emptyinterval
entireinterval
exact
extended_div
fastpow
fastpown
has_exact_display
hull
ill
in_interval
inf
interiordiff
intersect_interval
interval
isatomic
isbounded
iscommon
isdisjoint_interval
isempty_interval
isentire_interval
isequal_interval
isguaranteed
isinterior
isnai
issetequal_interval
isstrictless
isstrictsubset
issubset_interval
isthin
isthininteger
isthinone
isthinzero
isunbounded
isweakless
mag
mid
midradius
mig
mince
mince!
nai
numtype
overlap
pieces
pow
pown
precedes
radius
rootn
sample
setdisplay
strictprecedes
sup
trv
union_interval
```

### Construction forms

- `interval(1.0, 2.0)` = `[1.0, 2.0]_com` :: `Interval{Float64}`
- `Interval(1.0, 2.0)` → ERROR `MethodError`: MethodError: no method matching Interval(::Float64, ::Float64)
The type `Interval` exists, but no method is defined for this combination of argument types when trying to construct it.

Closest candidates are:
  Interval(::Real)
   @ IntervalArithmetic ~/.julia/packages/IntervalArithmetic/0Eg2V/src/intervals/construction.jl:545
  Interval(!Matched::ExactReal)
   @ IntervalArithmetic ~/.julia/packages/IntervalArithmetic/0Eg2V/src/intervals/exact_literals.jl:136
  (::Type{T})(!Matched::Static.StaticInteger) where T<:Real
   @ Static ~/.julia/packages/Static/TjBVO/src/Static.jl:487
  ...

- `0.0 .. 1.0` → ERROR `UndefVarError`: UndefVarError: `..` not defined in `Main`
Suggestion: check for spelling errors or missing imports.
Hint: a global variable of this name also exists in IntervalSets.
    - Also exported by DomainSets (loaded but not imported in Main).

### Endpoint accessors (which exist?)

- `IntervalArithmetic.inf(I)` = `1.0` :: `Float64`
- `IntervalArithmetic.sup(I)` = `2.0` :: `Float64`
- `IntervalArithmetic.infimum` — NOT DEFINED
- `IntervalArithmetic.supremum` — NOT DEFINED
- `IntervalArithmetic.lower` — NOT DEFINED
- `IntervalArithmetic.upper` — NOT DEFINED
- `IntervalArithmetic.lo` — NOT DEFINED
- `IntervalArithmetic.hi` — NOT DEFINED
- `IntervalArithmetic.midpoint` — NOT DEFINED
- `IntervalArithmetic.diam(I)` = `1.0` :: `Float64`
- `IntervalArithmetic.radius(I)` = `0.5` :: `Float64`

### Empty / NaI / decoration

- `IntervalArithmetic.emptyinterval` defined: true
- `IntervalArithmetic.isempty` defined: true
- `IntervalArithmetic.isnai` defined: true
- `IntervalArithmetic.isthin` defined: true
- `IntervalArithmetic.isfinite` defined: true
- `IntervalArithmetic.isbounded` defined: true
- `emptyinterval()` = `∅_trv` :: `Interval{Float64}`

### Function evaluation over an interval

- `f(interval(0.0, 1000.0))  where  f(x) = 42/(42+x)` = `[0.0403071, 1.0]_com_NG` :: `Interval{Float64}`
- `sin(interval(0, π))` = `[-3.21625e-16, 1.0]_com` :: `Interval{Float64}`
- `exp(interval(-1.0, 1.0))` = `[0.367879, 2.71828]_com` :: `Interval{Float64}`

### Rounding-mode API surface

- `IntervalArithmetic.setrounding` defined: true
- `IntervalArithmetic.IntervalRounding` defined: true
- `IntervalArithmetic.rounding` defined: true
- `IntervalArithmetic.setformat` defined: false
- `IntervalArithmetic.configure` defined: true

### IntervalRootFinding — root-finding entry points

```
@I_str
@exact
@interval
BareInterval
Bisection
ComplexI
Constant
Domain
ExactReal
Interval
IntervalArithmetic
IntervalRootFinding
Krawczyk
Newton
Overlap
Piecewise
RealIntervalType
RealOrComplexI
Root
RootProblem
bareinterval
bisect
bisect_region
bounds
branch_and_prune
cancelminus
cancelplus
com
contract
dac
decoration
def
derivative
diam
discontinuities
dist
domains
emptyinterval
entireinterval
exact
extended_div
fastpow
fastpown
gauss_elimination_interval
gauss_seidel_contractor
gauss_seidel_interval
has_exact_display
hull
ill
in_interval
in_region
inf
interiordiff
intersect_interval
intersect_region
interval
isatomic
isbounded
isbounded_region
iscommon
isdisjoint_interval
isempty_interval
isempty_region
isentire_interval
isequal_interval
isequal_region
isguaranteed
isinterior
isnai
isnai_region
issetequal_interval
isstrictless
isstrictsubset
issubset_interval
isthin
isthininteger
isthinone
isthinzero
isunbounded
isunique
isweakless
jacobian
mag
mid
midradius
mig
mince
mince!
nai
numtype
overlap
pieces
pow
pown
precedes
radius
root_region
root_status
rootn
roots
sample
setdisplay
strictprecedes
sup
trv
union_interval
```
- `IntervalRootFinding.roots` defined: true
- `IntervalRootFinding.Krawczyk` defined: true
- `IntervalRootFinding.Newton` defined: true
- `IntervalRootFinding.Bisection` defined: true
- `IntervalRootFinding.Root` defined: true

## Symbolics.jl + ModelingToolkit + SymbolicUtils


### Symbolics module exports (head)

```
@acrule
@arrayop
@derivative_rule
@makearray
@register_array_symbolic
@register_derivative
@register_discontinuity
@register_inverse
@register_symbolic
@rule
@symbolic_wrap
@syms
@symstruct
@variables
@wrapped
Arr
BS
Differential
Equation
IRStructure
Inequality
Integral
LinearExpander
NAMESPACE_SEPARATOR
Num
Rewriters
RuleSet
SafeReal
SymReal
SymStruct
SymbolicLinearODE
SymbolicUtils
Symbolics
SymbolicsSparsityDetector
TreeReal
Unknown
Variable
VariableDefaultValue
VariableSource
_parse_vars
arguments
build_function
derivative
expand
expand_derivatives
factors
flatten_fractions
get_differential_vars
get_reachability
get_variables
get_variables!
getmetadata
gradient
groebner_basis
has_inverse
has_left_inverse
has_right_inverse
hasmetadata
hessian
infimum
inverse
is_derivative
is_groebner_basis
iscall
istree
jacobian
left_continuous_function
left_inverse
limit
linear_expansion
operation
option_to_metadata_type
parse_expr_to_symbolic
polynomial_coeffs
populate_ir!
print_ir
quick_cancel
right_continuous_function
right_inverse
rootfunction
```

(130 total exports.)

### Variable / equation idiom

- `Symbolics.@variables x y t` worked. Type of x: `Num`, name: `x`
- `Symbolics.@parameters` defined: false
- `ModelingToolkit.@parameters` defined: true
- `ModelingToolkit.@parameters p q` worked. Type of p: `Num`
- `x^2 + y ~ 0` →  `Equation` with fields `(:lhs, :rhs)`

### Simplification, expansion, substitution

- `Symbolics.simplify` defined: true
- `Symbolics.expand` defined: true
- `Symbolics.substitute` defined: true
- `Symbolics.get_variables` defined: true
- `Symbolics.derivative` defined: true
- `Symbolics.jacobian` defined: true
- `Symbolics.hessian` defined: true
- `Symbolics.gradient` defined: true
- `Symbolics.polynomial_coeffs` defined: true

### Differential operator

- `Symbolics.Differential` defined: true
- `Symbolics.expand_derivatives` defined: true

### ModelingToolkit naming

- `ModelingToolkit.ODESystem` defined: true
- `ModelingToolkit.NonlinearSystem` defined: true
- `ModelingToolkit.OptimizationSystem` defined: true
- `ModelingToolkit.unknowns` defined: true
- `ModelingToolkit.states` defined: false
- `ModelingToolkit.equations` defined: true
- `ModelingToolkit.parameters` defined: true
- `ModelingToolkit.observed` defined: true
- `ModelingToolkit.structural_simplify` defined: true
- `ModelingToolkit.complete` defined: true

### SymbolicUtils internals (BasicSymbolic, RuleSet, @rule)

- `SymbolicUtils.BasicSymbolic` defined: true
- `SymbolicUtils.Term` defined: true
- `SymbolicUtils.Sym` defined: true
- `SymbolicUtils.Add` defined: true
- `SymbolicUtils.Mul` defined: true
- `SymbolicUtils.Pow` defined: false
- `SymbolicUtils.RuleSet` defined: true
- `SymbolicUtils.@rule` defined: true

### Latexify

- `Latexify.latexify` defined: true

## JuMP


### Status / solution API surface

- `JuMP.Model` defined: true
- `JuMP.optimize!` defined: true
- `JuMP.termination_status` defined: true
- `JuMP.primal_status` defined: true
- `JuMP.dual_status` defined: true
- `JuMP.is_solved_and_feasible` defined: true
- `JuMP.has_values` defined: true
- `JuMP.has_duals` defined: true
- `JuMP.objective_value` defined: true
- `JuMP.objective_bound` defined: true
- `JuMP.value` defined: true
- `JuMP.dual` defined: true
- `JuMP.reduced_cost` defined: true
- `JuMP.shadow_price` defined: true
- `JuMP.lp_sensitivity_report` defined: true
- `JuMP.compute_conflict!` defined: true
- `JuMP.copy_conflict` defined: true
- `JuMP.result_count` defined: true
- `JuMP.set_time_limit_sec` defined: true

### MOI termination/result-status enum values

Termination status enumerators (subset):
  - `MOI.OPTIMAL` defined: true
  - `MOI.INFEASIBLE` defined: true
  - `MOI.DUAL_INFEASIBLE` defined: true
  - `MOI.LOCALLY_SOLVED` defined: true
  - `MOI.LOCALLY_INFEASIBLE` defined: true
  - `MOI.INFEASIBLE_OR_UNBOUNDED` defined: true
  - `MOI.ALMOST_OPTIMAL` defined: true
  - `MOI.TIME_LIMIT` defined: true
  - `MOI.ITERATION_LIMIT` defined: true
  - `MOI.NUMERICAL_ERROR` defined: true
  - `MOI.INVALID_MODEL` defined: true
  - `MOI.OTHER_ERROR` defined: true
  - `MOI.OPTIMIZE_NOT_CALLED` defined: true
Result status enumerators:
  - `MOI.NO_SOLUTION` defined: true
  - `MOI.FEASIBLE_POINT` defined: true
  - `MOI.NEARLY_FEASIBLE_POINT` defined: true
  - `MOI.INFEASIBILITY_CERTIFICATE` defined: true
  - `MOI.NEARLY_INFEASIBILITY_CERTIFICATE` defined: true
  - `MOI.REDUCTION_CERTIFICATE` defined: true
  - `MOI.UNKNOWN_RESULT_STATUS` defined: true

### End-to-end LP probe (HiGHS)

- `termination_status(m)` = `OPTIMAL`
- `primal_status(m)` = `FEASIBLE_POINT`
- `dual_status(m)` = `FEASIBLE_POINT`
- `is_solved_and_feasible(m)` = `true`
- `objective_value(m)` = `16.0`
- `value(x)` = `2.0`, `value(y)` = `2.0`
- `dual(c1)` = `-1.0`, `dual(c2)` = `-2.0`
- `lp_sensitivity_report(m)` returned `SensitivityReport{Float64}`

## Catalyst.jl


### Module exports (head)

```
@acrule
@arrayop
@brownian
@brownians
@component
@compound
@compounds
@connector
@constants
@derivative_rule
@derivatives
@discretes
@independent_variables
@makearray
@mtkbuild
@mtkcompile
@mtkcomplete
@named
@namespace
@network_component
@nonamespace
@pack!
@parameters
@poissonians
@reaction
@reaction_network
@register_array_symbolic
@register_derivative
@register_discontinuity
@register_inverse
@register_symbolic
@rule
@species
@symbolic_wrap
@syms
@symstruct
@transport_reaction
@unpack
@variables
@wrapped
AbstractCollocation
AbstractDynamicOptProblem
AnalysisPoint
AssignmentAffect
BS
BipartiteGraph
CartesianGrid
CartesianGridReJ
CasADiCollocation
CasADiDynamicOptProblem
Catalyst
Clock
Connection
Differential
DiscreteFunction
DiscreteProblem
DiscreteSpaceReactionSystem
DiscreteSystem
DynamicOptSolution
Equation
EvalAt
Flow
Girsanov_transform
GlobalScope
Hold
HomotopyContinuationProblem
HybridProblem
IRStructure
ImplicitDiscreteFunction
ImplicitDiscreteProblem
ImplicitDiscreteSystem
Inequality
InfiniteOptCollocation
InfiniteOptDynamicOptProblem
Initial
InitializationProblem
Integral
IntervalNonlinearFunction
IntervalNonlinearProblem
JuMPCollocation
```

(426 total exports.)

### Network construction + accessors

- `@reaction_network` returns: `ReactionSystem{Catalyst.NetworkProperties{Int64, SymbolicUtils.BasicSymbolicImpl.var"typeof(BasicSymbolicImpl)"{SymReal}}}`
- `Catalyst.species` exported & defined
- `Catalyst.parameters` exported & defined
- `Catalyst.reactions` exported & defined
- `Catalyst.equations` exported & defined
- `Catalyst.unknowns` exported & defined
- `Catalyst.netstoichmat` exported & defined
- `Catalyst.substoichmat` exported & defined
- `Catalyst.prodstoichmat` exported & defined
- `Catalyst.conservationlaws` exported & defined
- `Catalyst.conservedequations` exported & defined
- `Catalyst.conservationlaw_constants` exported & defined
- `Catalyst.complexes` NOT defined in Catalyst
- `Catalyst.complexstoichmat` exported & defined
- `Catalyst.reactioncomplexes` exported & defined

### Conservation laws probe

- `conservationlaws(rn)` = `[-1 1 0; 1 0 1]` :: `Matrix{Int64}`

### Compilation paths to dynamics

- `Catalyst.ODEProblem` reachable via Catalyst: true
- `Catalyst.SDEProblem` reachable via Catalyst: true
- `Catalyst.JumpProblem` reachable via Catalyst: true
- `Catalyst.DiscreteProblem` reachable via Catalyst: true
- `Catalyst.SteadyStateProblem` reachable via Catalyst: true
- `convert(ODESystem, rn)` errored: MethodError: MethodError: Cannot `convert` an object of type ReactionSystem{Catalyst.NetworkProperties{Int64, SymbolicUtils.BasicSymbolicImpl.var"typeof(BasicSymbolicImpl)"{SymReal}}} to an object of type ModelingToolkitBase.IntermediateDeprecationSystem
The function `convert` exists, but no method is defined for this combination of argument types.

Closest candidates are:
  ModelingToolkitBase.IntermediateDeprecationSystem(::Any...; kwargs...)
   @ ModelingToolkitBase ~/.julia/packages/ModelingToolkitBase/7AL5w/src/deprecations.jl:20
  convert(::Type{T}, !Matched::T) where T
   @ Base Base_compiler.jl:133


### Deficiency-theorem support

- `Catalyst.deficiency` defined: true
- `Catalyst.deficiencyzerotheorem` defined: false
- `Catalyst.deficiencyonetheorem` defined: false
- `Catalyst.isweaklyreversible` defined: true
- `Catalyst.iscomplexbalanced` defined: true

## OrdinaryDiffEq + SciMLBase


### Problem / solution / algorithm types

- `SciMLBase.ODEProblem` defined: true
- `SciMLBase.ODESolution` defined: true
- `SciMLBase.SteadyStateProblem` defined: true
- `SciMLBase.ReturnCode` defined: true
- `SciMLBase.successful_retcode` defined: true
- `SciMLBase.EnsembleProblem` defined: true
- `SciMLBase.remake` defined: true
- `OrdinaryDiffEq.Tsit5` defined: true
- `OrdinaryDiffEq.Vern9` defined: true
- `OrdinaryDiffEq.Rosenbrock23` defined: true
- `OrdinaryDiffEq.Rodas5` defined: true
- `OrdinaryDiffEq.Rodas5P` defined: true
- `OrdinaryDiffEq.QNDF` defined: true
- `OrdinaryDiffEq.FBDF` defined: true
- `OrdinaryDiffEq.AutoTsit5` defined: true
- `OrdinaryDiffEq.AutoVern9` defined: true

### ReturnCode enumerators

```
APosterioriSafetyFailure
ConvergenceFailure
Default
DtLessThanMin
DtNaN
ExactSolutionLeft
ExactSolutionRight
Failure
FloatingPointLimit
Infeasible
InitialFailure
InternalLineSearchFailed
InternalLinearSolveFailed
MaxIters
MaxNumSub
MaxTime
ReturnCode
ShrinkThresholdExceeded
Stalled
StalledSuccess
Success
T
Terminated
Unstable
```

### End-to-end ODE probe (Tsit5)

- `solve(prob, Tsit5())` returned `ODESolution{Float64, 2, Vector{Vector{Float64}}, Nothing, Nothing, Vector{Float64}, Vector{Vector{Vector{Float64}}}, Nothing, ODEProblem{Vector{Float64}, Tuple{Float64, Float64}, true, Vector{Float64}, ODEFunction{true, SciMLBase.AutoSpecialize, FunctionWrappersWrappers.FunctionWrappersWrapper{Tuple{FunctionWrappers.FunctionWrapper{Nothing, Tuple{Vector{Float64}, Vector{Float64}, Vector{Float64}, Float64}}}, FunctionWrappersWrappers.AllowNonIsBits, FunctionWrappersWrappers.SingleCacheStorage}, LinearAlgebra.UniformScaling{Bool}, Nothing, Nothing, Nothing, Nothing, Nothing, Nothing, Nothing, Nothing, Nothing, Nothing, Nothing, Nothing, typeof(SciMLBase.DEFAULT_OBSERVED), Nothing, Nothing, Nothing, Nothing}, Base.Pairs{Symbol, Union{}, Nothing, @NamedTuple{}}, SciMLBase.StandardODEProblem}, Tsit5{typeof(OrdinaryDiffEqCore.trivial_limiter!), typeof(OrdinaryDiffEqCore.trivial_limiter!), Static.False}, OrdinaryDiffEqCore.InterpolationData{ODEFunction{true, SciMLBase.AutoSpecialize, FunctionWrappersWrappers.FunctionWrappersWrapper{Tuple{FunctionWrappers.FunctionWrapper{Nothing, Tuple{Vector{Float64}, Vector{Float64}, Vector{Float64}, Float64}}}, FunctionWrappersWrappers.AllowNonIsBits, FunctionWrappersWrappers.SingleCacheStorage}, LinearAlgebra.UniformScaling{Bool}, Nothing, Nothing, Nothing, Nothing, Nothing, Nothing, Nothing, Nothing, Nothing, Nothing, Nothing, Nothing, typeof(SciMLBase.DEFAULT_OBSERVED), Nothing, Nothing, Nothing, Nothing}, Vector{Vector{Float64}}, Vector{Float64}, Vector{Vector{Vector{Float64}}}, Nothing, OrdinaryDiffEqTsit5.Tsit5Cache{Vector{Float64}, Vector{Float64}, Vector{Float64}, typeof(OrdinaryDiffEqCore.trivial_limiter!), typeof(OrdinaryDiffEqCore.trivial_limiter!), Static.False}, Nothing}, SciMLBase.DEStats, Nothing, Nothing, Nothing, Nothing}`
- `SciMLBase.successful_retcode(sol)` = `true`
- `sol.retcode` = `Success`
- `length(sol.t)` = `12`, `length(sol.u)` = `12`
- `sol(0.5)` (interpolation) = `[0.7788007830458987]`
- `sol[1]` = `[1.0]` (NB: SciMLBase v3 changed `sol[i]` to AbstractArray indexing)
- ODESolution fields: `(:u, :u_analytic, :errors, :t, :k, :discretes, :prob, :alg, :interp, :dense, :tslocation, :stats, :alg_choice, :retcode, :resid, :original, :saved_subsystem)`
