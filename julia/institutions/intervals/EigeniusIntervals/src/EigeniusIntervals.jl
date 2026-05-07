"""
    EigeniusIntervals

Handler package for the IntervalArithmetic Eigenius institution
(Phase 19a.6 / D31 §4.1). Exports `validate_bounded_by(b)` — the
AutoOnLoad gate's worker entry point for `BoundedBy` resources.

# Dispatch flow

1. The kernel commits a `BoundedBy(value, lower, upper)` resource on a
   chain that has the IntervalArithmetic institution installed.
2. `commit_with_validation` fires the AutoOnLoad QueryClass, which
   sends a `DispatchExternal` RPC to the orchestrator carrying the
   `BoundedBy` mirror struct serialised to Eigon-CBOR.
3. The orchestrator routes the call through the substrate's Julia
   runtime; the worker decodes the input via the mirror's
   `decode_BoundedBy` codec and dispatches `Main.validate_bounded_by`.
4. This handler computes the rigorous interval inclusion via
   `IntervalArithmetic.jl` and returns a Verdict — `Holds` / `Fails`
   / `Undecidable`.
5. The worker CBOR-encodes the Verdict and returns it; the kernel
   commits a Verdict + RuntimeInvocation alongside the gated
   resource per [D31 §6.3].

# Verdict shape

The handler returns `Dict{String,Any}` carrying

```
"urn:eigenius:core:is_a"      => ["urn:eigenius:institution:Verdict"]
"urn:eigenius:core:ctor_name" => "Holds" | "Fails" | "Undecidable"
```

The kernel's `parse_verdict` reads `core:ctor_name` to apply the
Holds/Fails/Undecidable rule. We return a Dict (rather than a typed
mirror struct) because the `Verdict` Eigenius class is an
`InductiveType`, not a `Class` — the mirror generator only emits
mirrors for the latter (see `crates/eigenius-julia/src/mirror_gen.rs`).
The Dict is forwarded as-is by the worker (no `_eigenius_encoders`
match), which is exactly what `parse_resource_lenient` expects on
the kernel side.

# Why three-valued

For real-valued bounds the inclusion `value ∈ [lower, upper]` is a
genuine three-state question once rounding is taken into account.
IntervalArithmetic.jl produces interval overlap relations whose
"unverified but not refuted" case maps cleanly onto Eigenius's
`Undecidable` verdict — preferable to silently rounding to one side.
"""
module EigeniusIntervals

using IntervalArithmetic
using EigeniusMirror

export validate_bounded_by
# `compute_bounds` is conditionally exported below — only when the
# baked mirror includes `FormulaTerm` (Phase 19d.0 / D32 §4). The
# IntervalArithmetic-only e2e seeds the mirror with just `BoundedBy`,
# so FormulaTerm is absent there and the cross-institution probe
# code doesn't compile in.

const VERDICT_CLASS_IRI = "urn:eigenius:institution:Verdict"
const IS_A_PROP = "urn:eigenius:core:is_a"
const CTOR_NAME_PROP = "urn:eigenius:core:ctor_name"

"""
    validate_bounded_by(b::BoundedBy) -> Dict

Verify `b.value ∈ [b.lower, b.upper]` rigorously via
`IntervalArithmetic.jl`. Returns the canonical Verdict shape
described in the module docstring.

The check uses degenerate (point) intervals on both sides so the
floating-point representation of the value gets the same
interval-arithmetic treatment as the bounds — `Holds` is a proof of
containment, not a heuristic.
"""
function validate_bounded_by(b::BoundedBy)
    target = interval(b.lower, b.upper)
    point = interval(b.value)
    if issubset_interval(point, target)
        return _verdict("Holds")
    elseif isdisjoint_interval(point, target)
        return _verdict("Fails")
    else
        # Overlap non-empty but not full-subset — the rigorous check
        # can't decide, typically because `value` lands exactly on a
        # bound that has multiple Float64 representations.
        return _verdict("Undecidable")
    end
end

_verdict(ctor::AbstractString) = Dict{String,Any}(
    IS_A_PROP => [VERDICT_CLASS_IRI],
    CTOR_NAME_PROP => ctor,
)

# ─── Cross-institution probe (D32 §6 / Phase 19d follow-on) ─────────────
#
# The same `formulas:FormulaTerm` value that `EigeniusSymbolics`
# consumes for `simplify` is also a legitimate input to interval-
# arithmetic operations. The two handlers don't share any code, but
# they share the chain-side typed payload — exactly the
# "comorphism `m` is the identity on FormulaTerm" story D32 §6.2
# describes.
#
# `compute_bounds` takes a `SymbolicExpression` (which is `FormulaTerm`-
# in-a-typed-wrapper) plus a domain interval `[lo, hi]` for the free
# variable `x`, walks the FormulaTerm by interval-arithmetic
# semantics, and returns a `BoundedBy` resource containing the
# resulting range. Free variables other than `x` aren't supported in
# this v1 probe — a richer multi-variable surface lands when the
# Symbolics institution gains a typed `var_context` extension to
# `SymbolicExpression`.
#
# The FormulaTerm-dependent code path is guarded by `@static if`
# because the same handler package is exercised by the
# IntervalArithmetic-only e2e (mirror seeds = `[BoundedBy]`), where
# FormulaTerm doesn't make it into the closure. With the guard, the
# module precompiles cleanly in both shapes; with FormulaTerm
# present, `compute_bounds` is exported and ready.

@static if isdefined(EigeniusMirror, :FormulaTerm_Var)

export compute_bounds

const _OP_INTERVAL = Dict{String, Function}(
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

"""
    formula_to_interval(t::FormulaTerm, env::Dict{String, <:Interval}) -> Interval

Recursive interval-arithmetic interpreter over the chain-shared
`FormulaTerm` language. `env` binds variable names to intervals; an
unbound `Var` raises a clear error so a malformed input doesn't
silently produce a wrong bound.
"""
formula_to_interval(t::EigeniusMirror.FormulaTerm_Var, env) =
    haskey(env, t.name) ? env[t.name] :
    error("EigeniusIntervals: free variable `$(t.name)` not bound in env (only `x` is supported in v1)")

formula_to_interval(t::EigeniusMirror.FormulaTerm_LitFloat, env) = interval(t.value, t.value)

function formula_to_interval(t::EigeniusMirror.FormulaTerm_App, env)
    spine = Any[]
    cursor = t
    while cursor isa EigeniusMirror.FormulaTerm_App
        push!(spine, cursor.arg)
        cursor = cursor.head
    end
    if !(cursor isa EigeniusMirror.FormulaTerm_OpRef)
        error("EigeniusIntervals: unsupported App head — expected OpRef, got $(typeof(cursor))")
    end
    if !haskey(_OP_INTERVAL, cursor.iri)
        error("EigeniusIntervals: operator `$(cursor.iri)` not in interval-arithmetic catalog")
    end
    args = reverse([formula_to_interval(a, env) for a in spine])
    return _OP_INTERVAL[cursor.iri](args...)
end

formula_to_interval(t::EigeniusMirror.FormulaTerm_OpRef, env) =
    error("EigeniusIntervals: bare OpRef `$(t.iri)` outside an App spine is unsupported")

formula_to_interval(t::EigeniusMirror.FormulaTerm_Lam, env) =
    error("EigeniusIntervals: Lam binder is unsupported in interval-arithmetic dispatch")

formula_to_interval(t::EigeniusMirror.FormulaTerm_Pi, env) =
    error("EigeniusIntervals: Pi binder is unsupported in interval-arithmetic dispatch")

"""
    compute_bounds(expr::SymbolicExpression, domain::BoundedBy) -> BoundedBy

Interval-extend `expr` over the domain `[domain.lower, domain.upper]`
for the free variable `x`. Returns a `BoundedBy` whose `lower` / `upper`
fields enclose the function's range over the domain — a rigorous
bound by interval-arithmetic discipline, not a heuristic. The
`value` field carries the interval's midpoint as a representative
point; what the consumer cares about is the bounds.

Two-input typed dispatch: the substrate's worker decodes both
arguments through the mirror's `_eigenius_decoders` registry, so
both must be chain-typed Resource shapes. `BoundedBy` doubles as the
domain carrier (its `value` field is unused here, the `lower`/`upper`
pair is what matters); a future richer probe with multiple free
variables would introduce a typed `Domain` resource class.

The probe demonstrates D32's central claim: the same `FormulaTerm`
value the Symbolics handler simplifies can be handed to
IntervalArithmetic without transformation. The two institutions
share the typed payload language; the comorphism between them is
the identity on `FormulaTerm`.
"""
function compute_bounds(
    expr::EigeniusMirror.SymbolicExpression,
    domain::EigeniusMirror.BoundedBy,
)
    env = Dict{String, Any}("x" => interval(domain.lower, domain.upper))
    result = formula_to_interval(expr.term, env)
    lo = inf(result)
    hi = sup(result)
    return EigeniusMirror.BoundedBy((lo + hi) / 2, lo, hi)
end

end # @static if isdefined(EigeniusMirror, :FormulaTerm_Var)

end # module
