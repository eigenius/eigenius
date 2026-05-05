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
using EigeniusMirror: BoundedBy

export validate_bounded_by

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

end # module
