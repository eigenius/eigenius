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
using Symbolics
using SymbolicUtils
using EigeniusMirror

export validate_conservation_law, compile_to_ode

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

# ─── Catalyst → DiffEq comorphism (D27 §4.4.4 / D32 §6) ─────────────────
#
# `compile_to_ode(input::CatalystToOdeInput) -> OdeProblem` is the
# operational backing of the chain-side `Catalyst → DiffEq` comorphism.
# Takes a network + ICs + parameters + tspan, computes the symbolic
# RHS via Catalyst's stoichiometry + rate-law machinery, and
# translates each component to a chain-typed FormulaTerm. The result
# is an `OdeProblem` mirror struct DiffEq can integrate as-is.
#
# The handler is gated by `@static if isdefined(...)` because
# `CatalystToOdeInput` and `OdeProblem` only enter the closure when
# the operator generates a mirror that includes them. The
# conservation-law-only flow (the v1 Catalyst demo) uses a narrower
# closure and skips compiling this handler.

@static if isdefined(EigeniusMirror, :CatalystToOdeInput) &&
          isdefined(EigeniusMirror, :OdeProblem)

# ─── num_to_formula — translator from Symbolics.Num to FormulaTerm
#
# TODO(common-package): this translator is duplicated from
# `EigeniusSymbolics`; both should share via a future
# `EigeniusFormulas` / extended `EigeniusJuliaCommon` package once
# more institutions need it. v1.6 keeps the duplication for cycle-
# avoidance — Catalyst depending on Symbolics is non-obvious, and
# adding Symbolics deps to EigeniusJuliaCommon currently changes
# the picture for every mirror consumer.

const _FN_TO_IRI = IdDict{Any, String}(
    (+) => "urn:eigenius:formulas:ops:add",
    (-) => "urn:eigenius:formulas:ops:sub",
    (*) => "urn:eigenius:formulas:ops:mul",
    (/) => "urn:eigenius:formulas:ops:div",
    (^) => "urn:eigenius:formulas:ops:pow",
    exp => "urn:eigenius:formulas:ops:exp",
    log => "urn:eigenius:formulas:ops:log",
    sin => "urn:eigenius:formulas:ops:sin",
    cos => "urn:eigenius:formulas:ops:cos",
    tan => "urn:eigenius:formulas:ops:tan",
    sqrt => "urn:eigenius:formulas:ops:sqrt",
    abs => "urn:eigenius:formulas:ops:abs",
)

"""
    num_to_formula(n) -> FormulaTerm

Translate a `Symbolics.Num` (or its underlying `BasicSymbolic` /
plain numeric value) into a chain-shaped FormulaTerm value emitted
by the mirror. Mirrors `EigeniusSymbolics.num_to_formula` exactly
— see that module's docstring for design rationale.
"""
num_to_formula(n::Symbolics.Num) = num_to_formula(Symbolics.value(n))

function num_to_formula(v)
    if v isa Real
        return EigeniusMirror.FormulaTerm_LitFloat(Float64(v))
    end
    # SymbolicUtils 4.x wraps numeric constants surviving simplification
    # (and stoichiometry coefficients) as `BSImpl.Const` BasicSymbolics
    # — neither `issym` nor `iscall` matches them. Detect and unwrap.
    if SymbolicUtils.isconst(v)
        return EigeniusMirror.FormulaTerm_LitFloat(Float64(SymbolicUtils.unwrap_const(v)))
    end
    if SymbolicUtils.issym(v)
        return EigeniusMirror.FormulaTerm_Var(string(SymbolicUtils.nameof(v)))
    end
    if SymbolicUtils.iscall(v)
        op = SymbolicUtils.operation(v)
        args = SymbolicUtils.arguments(v)
        # Catalyst represents species as time-dependent callable Syms
        # (`A(t)`): the operation is the underlying Sym and the args
        # are the independent variable(s). Chain-side `FormulaTerm`
        # is intentionally flat — no time-dependence in the variable
        # layer — so reify any "Sym applied to args" as a bare `Var`
        # carrying the Sym's name. This keeps `compile_to_ode`'s
        # output (FormulaTerm RHS over `[A, B, ...]`) aligned with
        # the chain-typed `OdeProblem.state_names`.
        if SymbolicUtils.issym(op)
            return EigeniusMirror.FormulaTerm_Var(string(SymbolicUtils.nameof(op)))
        end
        op_iri = get(_FN_TO_IRI, op, nothing)
        if op_iri === nothing
            error("EigeniusCatalyst: Symbolics produced operation `$op` with no FormulaTerm encoding; add it to _FN_TO_IRI")
        end
        # SymbolicUtils represents associative ops (`*`, `+`, `min`,
        # `max`, …) as n-ary internally, so `args` may carry more
        # operands than the chain's binary operator catalog declares.
        # Left-fold: `op(a, b, c, …)` → `op(op(op(a, b), c), …)`,
        # introducing a fresh `OpRef` at each step so each App-spine
        # collected by the chain validator carries exactly the
        # operator's declared arity (binary for `mul`/`add`, unary
        # for `neg`/`sin`/`cos`/etc.).
        n_args = length(args)
        n_args >= 1 ||
            error("EigeniusCatalyst: operator `$op` produced zero arguments — Symbolics shouldn't emit nullary calls")
        if n_args == 1
            return EigeniusMirror.FormulaTerm_App(
                EigeniusMirror.FormulaTerm_OpRef(op_iri),
                num_to_formula(args[1]),
            )
        end
        # n_args ≥ 2 — start from a well-formed binary App on the
        # first two operands, then left-fold the remainder by
        # nesting `op(prev_result, next)` at each step.
        result = EigeniusMirror.FormulaTerm_App(
            EigeniusMirror.FormulaTerm_App(
                EigeniusMirror.FormulaTerm_OpRef(op_iri),
                num_to_formula(args[1]),
            ),
            num_to_formula(args[2]),
        )
        for a in args[3:end]
            result = EigeniusMirror.FormulaTerm_App(
                EigeniusMirror.FormulaTerm_App(
                    EigeniusMirror.FormulaTerm_OpRef(op_iri),
                    result,
                ),
                num_to_formula(a),
            )
        end
        return result
    end
    error("EigeniusCatalyst: cannot encode Symbolics term of type $(typeof(v)) as FormulaTerm")
end

"""
    compile_to_ode(input::CatalystToOdeInput) -> OdeProblem

Compile a Catalyst reaction network to a chain-typed `OdeProblem`
with FormulaTerm-typed RHS components. The path:

1. Parse the network's `@reaction_network` source via the same
   defensive eval-shape check `validate_conservation_law` uses.
2. Extract the symbolic RHS for each species: walk
   `netstoichmat(rn) * oderatelaw.(reactions(rn))`. The Nth row of
   the stoichiometry matrix dotted with the rate-law vector gives
   `du[N]/dt` as a `Symbolics.Num`.
3. Translate each `Num` to a `FormulaTerm` via `num_to_formula`.
4. Pack into an `OdeProblem` mirror struct, aligning species/
   parameter names with the network's canonical ordering.

The output's `state_names` and `parameter_names` come straight from
the input network's `species_declared` / `parameters_declared` —
the comorphism preserves the user's authored ordering through to
the integrator's u/p vectors.
"""
"""
    _bare_name(s) -> String

Extract the bare textual name from a Catalyst species symbolic.
Catalyst represents species as time-dependent variables `A(t)` — a
`Term` whose operation is the underlying `Sym`. Plain (non-time-
dependent) parameters come back as bare `Sym`s. Both shapes need to
collapse to the user-visible name (`"A"`, `"k"`).
"""
function _bare_name(s)
    v = Symbolics.unwrap(s)
    if SymbolicUtils.iscall(v)
        return string(SymbolicUtils.nameof(SymbolicUtils.operation(v)))
    elseif SymbolicUtils.issym(v)
        return string(SymbolicUtils.nameof(v))
    else
        error("EigeniusCatalyst: cannot extract bare name from $(typeof(v))")
    end
end

function compile_to_ode(input::EigeniusMirror.CatalystToOdeInput)
    rn = parse_network(input.network.network_source)
    rn = Catalyst.complete(Catalyst.flatten(rn))

    species_syms = Catalyst.species(rn)
    rxs = Catalyst.reactions(rn)
    n_species = length(species_syms)
    n_reactions = length(rxs)

    # Declared ordering from the chain side.
    declared_species = collect(String, input.network.species_declared)
    declared_params = collect(String, input.network.parameters_declared)

    actual_species_names = [_bare_name(s) for s in species_syms]
    if Set(actual_species_names) != Set(declared_species)
        error("EigeniusCatalyst.compile_to_ode: declared species $(declared_species) do not match network's actual species $(actual_species_names)")
    end

    # Build the per-species symbolic RHS:  rhs[i] = Σⱼ N[i,j] · rate(j)
    # where N is the net stoichiometric matrix. Using the explicit
    # sum keeps the construction transparent (no MTK ODESystem
    # round-trip needed; D27 §4.4 lists `oderatelaw` as the per-
    # reaction rate-law accessor).
    N = Catalyst.netstoichmat(rn)
    rate_laws = [Catalyst.oderatelaw(r) for r in rxs]

    # Map from Catalyst's species ordering to the user's
    # declared ordering — `actual_species_names[i]` may not equal
    # `declared_species[i]`, so we look up by name.
    declared_idx_of_actual = Dict(name => i for (i, name) in enumerate(declared_species))

    # `num_to_formula` reifies time-dependent Catalyst species
    # (`A(t)` callable Syms) as flat `Var`s, matching the chain's
    # flat variable layer.
    rhs_terms = Vector{Any}(undef, n_species)
    for (catalyst_i, name) in enumerate(actual_species_names)
        rhs_sym = sum(N[catalyst_i, j] * rate_laws[j] for j in 1:n_reactions; init = Symbolics.Num(0))
        declared_i = declared_idx_of_actual[name]
        rhs_terms[declared_i] = num_to_formula(Symbolics.simplify(rhs_sym))
    end

    # Mirror generator types `OdeProblem.rhs` as
    # `Vector{AbstractRhsComponent}` (D29 §7 — fields take the
    # abstract base, not the concrete struct). Julia's parametric
    # types are invariant, so `Vector{RhsComponent}` would not match;
    # build the comprehension as `AbstractRhsComponent[...]` so the
    # element type lines up with the constructor's signature.
    rhs_components = EigeniusMirror.AbstractRhsComponent[
        EigeniusMirror.RhsComponent(t)
        for t in rhs_terms
    ]

    return EigeniusMirror.OdeProblem(
        declared_species,
        declared_params,
        rhs_components,
        Float64.(input.initial_conditions),
        Float64.(input.parameter_values),
        Float64(input.time_span_start),
        Float64(input.time_span_end),
    )
end

end # @static if isdefined(EigeniusMirror, :CatalystToOdeInput)

end # module
