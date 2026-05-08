"""
    EigeniusCatalyst

Handler package for the Catalyst Eigenius institution (Phase 19h /
D27 §4.4). Exports `validate_conservation_law(c)` — the AutoOnLoad
gate's worker entry point for `ConservationLaw` resources.

# Dispatch flow

1. The kernel commits a `ConservationLaw(network, coefficients)`
   resource on a chain that has the Catalyst institution installed.
2. `commit_with_validation` fires the AutoOnLoad QueryClass, which
   sends a `DispatchExternal` RPC to the orchestrator carrying the
   `ConservationLaw` mirror struct (with the embedded
   `ReactionNetwork`) serialised to Eigon-CBOR.
3. The orchestrator routes the call through the substrate's Julia
   runtime; the worker decodes the input via the mirror's
   `decode_ConservationLaw` codec — which transitively decodes the
   nested `network::ReactionNetwork` via `decode_ReactionNetwork`.
4. This handler evaluates `network.network_source` to rebuild the
   Catalyst `ReactionSystem`, calls `Catalyst.conservationlaws(rn)`
   to get the conservation matrix, and row-span-checks the claimed
   coefficient vector against it.
5. Returns a `Verdict` Dict — `Holds` when the claim is verified,
   `Fails` when the rank check refutes it.

# Verdict policy

Conservation-law validity is a *structural* property — a vector
either lies in the row span of the conservation matrix or it
doesn't. `Fails` is meaningful here, unlike Symbolics' heuristic
simplifier where `Fails` is reserved. v1 doesn't produce
`Undecidable`; future extensions (e.g. a structurally-simplified
network where some species are eliminated) might.

# Verified API note

Catalyst 16.1.1: `@reaction_network` macro returns a
`ReactionSystem`; `Catalyst.conservationlaws(rn)` returns
`Matrix{Int64}` whose rows span the network's left-nullspace
(stoichiometric conservation laws). Empty matrix when the network
admits no conservation laws — a degenerate case but a valid one
(e.g. a network with only spontaneous creation reactions).
"""
module EigeniusCatalyst

using Catalyst
using LinearAlgebra
using EigeniusMirror

export validate_conservation_law

const VERDICT_CLASS_IRI = "urn:eigenius:institution:Verdict"
const IS_A_PROP = "urn:eigenius:core:is_a"
const CTOR_NAME_PROP = "urn:eigenius:core:ctor_name"

_verdict(ctor::AbstractString) = Dict{String, Any}(
    IS_A_PROP => [VERDICT_CLASS_IRI],
    CTOR_NAME_PROP => ctor,
)

# ─── Network parsing ────────────────────────────────────────────────────

"""
    parse_network(source::AbstractString) -> ReactionSystem

Evaluate the `network_source` string in this module's scope to
reconstruct the Catalyst `ReactionSystem`. The source is expected
to contain a complete `@reaction_network begin … end` macro
invocation; `Core.eval` expands the macro using `Catalyst`-imported
bindings from this module.

A defensive parse-only check (`Meta.parse`) runs first; sources
that don't parse to a single `@reaction_network` macro call are
rejected before any Catalyst machinery touches them. This is
narrower than running arbitrary Julia code through `eval` — the
chain shape's `network_source` property is meant to carry a
DSL invocation, not a free-form program.
"""
function parse_network(source::AbstractString)
    expr = Meta.parse(source)
    # Accept the bare macro form `@reaction_network begin … end` and
    # also the `@reaction_network ... end` (no explicit begin) shape.
    # Reject anything else — e.g. a top-level `include`, a bare
    # function call, multiple statements — to keep the eval surface
    # narrow.
    if !(expr isa Expr && expr.head === :macrocall && expr.args[1] === Symbol("@reaction_network"))
        error("EigeniusCatalyst: network_source must be a single `@reaction_network` macro invocation; got $(typeof(expr))")
    end
    return Core.eval(@__MODULE__, expr)
end

# ─── Row-span check ─────────────────────────────────────────────────────

"""
    in_row_span(v, M) -> Bool

True iff the integer vector `v` lies in the row span of the integer
matrix `M`. Implemented as a rank check: append `v` as a new row
and compare ranks.

The rank is computed in Float64 — for the small integer matrices
Catalyst's conservation-law machinery produces (typical reaction
networks have <20 species and <10 conservation laws), the SVD-
based rank is reliable. Pathological cases with extremely large
integer entries or near-singular conditions could in principle
mis-classify; v1 accepts that risk for the simplicity. Exact-
arithmetic rank (over Rational or BigInt with row-reduction)
lands when a real network triggers a misclassification.
"""
function in_row_span(v::AbstractVector{<:Integer}, M::AbstractMatrix{<:Integer})::Bool
    if size(M, 1) == 0
        # No conservation laws — only the zero vector is in the
        # (empty) row span.
        return all(==(0), v)
    end
    Mf = Float64.(M)
    vf = Float64.(v)
    Mext = vcat(Mf, vf')
    return rank(Mf) == rank(Mext)
end

# ─── The handler ────────────────────────────────────────────────────────

"""
    validate_conservation_law(c::ConservationLaw) -> Verdict

Verify the `ConservationLaw` claim by re-deriving the network's
conservation matrix and row-span-checking the claimed coefficient
vector.

Returns:

- `Holds` when the claim is a valid conservation law of the network.
- `Fails` when the structural rank check refutes the claim, or when
  the coefficient count doesn't match the network's species count
  (a malformed claim).

A coefficient-count mismatch is reported as `Fails` rather than
raising — the chain shape's `coefficients` array length is the
author's responsibility to match `species_declared`, and a wrong
count is exactly what the institution should refuse via Verdict.
"""
function validate_conservation_law(c::EigeniusMirror.ConservationLaw)
    rn = parse_network(c.network.network_source)

    M = Catalyst.conservationlaws(rn)
    species_count = size(M, 2)

    if length(c.coefficients) != species_count
        return _verdict("Fails")
    end

    coeffs = Int.(c.coefficients)
    if in_row_span(coeffs, M)
        return _verdict("Holds")
    else
        return _verdict("Fails")
    end
end

end # module
