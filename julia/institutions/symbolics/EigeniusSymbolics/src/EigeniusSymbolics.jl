"""
    EigeniusSymbolics

Handler package for the Symbolics Eigenius institution (Phase 19d /
D27 §4.1). Exports `validate_simplifies_to(s)` — the AutoOnLoad gate's
worker entry point for `SimplifiesTo` resources.

# Dispatch flow

1. The kernel commits a `SimplifiesTo(expr, simplified)` resource on a
   chain that has the Symbolics institution installed.
2. `commit_with_validation` fires the AutoOnLoad QueryClass, which
   sends a `DispatchExternal` RPC to the orchestrator carrying the
   `SimplifiesTo` mirror struct serialised to Eigon-CBOR.
3. The orchestrator routes the call through the substrate's Julia
   runtime; the worker decodes the input via the mirror's
   `decode_SimplifiesTo` codec — which transitively decodes the
   nested `expr.term` and `simplified.term` `FormulaTerm` values via
   `decode_FormulaTerm` (D32 §3.6).
4. This handler translates each FormulaTerm into a Symbolics `Num`,
   runs `Symbolics.simplify` on the source, and compares against the
   claimed simplified form via `isequal`. `Holds` on match;
   `Undecidable` otherwise — Symbolics' `simplify` is heuristic
   (D27 §4.1.1), so disagreement of representations does not imply
   algebraic non-equivalence.

# Verdict shape

The handler returns `Dict{String,Any}` carrying the same shape every
external-runtime institution produces:

```
"urn:eigenius:core:is_a"      => ["urn:eigenius:institution:Verdict"]
"urn:eigenius:core:ctor_name" => "Holds" | "Fails" | "Undecidable"
```

# Operator catalog

Maps the chain-committed operator IRIs (`urn:eigenius:formulas:ops:add`,
…) onto the Julia functions Symbolics dispatches against. Every entry
mirrors an `Operator` resource in the `formulas:` ontology layer; new
operators land on the chain *and* in this map together.
"""
module EigeniusSymbolics

using Symbolics
using SymbolicUtils
using EigeniusMirror

export validate_simplifies_to

# ─── Operator catalog ───────────────────────────────────────────────────

# IRI → Julia function. Pinned to the v1 operator set declared in
# `ontologies/formulas/formulas-ontology.json`; new operators on the
# chain require an entry here before this institution accepts them.
const _OP_FN = Dict{String, Function}(
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

# Symbolic-variable cache keyed by name. Two `formula_to_num` calls
# referencing the same variable name need to produce the *same*
# Symbolics.Num — otherwise `simplify` can't match them up.
const _VAR_CACHE = Dict{String, Any}()

"""
    _get_or_make_var(name::String)

Get or create a Symbolics symbolic variable of type `Real` with the
given name. Cached so repeat lookups produce the same `Num` (load-
bearing for `Symbolics.simplify`'s structural identity).
"""
function _get_or_make_var(name::String)
    return get!(_VAR_CACHE, name) do
        Symbolics.variable(Symbol(name); T = Real)
    end
end

# ─── FormulaTerm → Symbolics.Num translation ────────────────────────────

"""
    formula_to_num(t::EigeniusMirror.FormulaTerm)

Translate a chain-shaped FormulaTerm value into a Symbolics `Num`.
Dispatches on the concrete ctor struct emitted by the mirror
generator (D32 §3.6 — `FormulaTerm_Var`, `FormulaTerm_LitFloat`, …).

Unsupported ctors (`Lam`, `Pi` — typed binders used in operator
signatures, not in the value-side expressions Symbolics simplifies)
raise an error; if you hit it, the SimplifiesTo claim was
authored with a typed binder where Symbolics expected a value-level
expression.
"""
formula_to_num(t::EigeniusMirror.FormulaTerm_Var) = _get_or_make_var(t.name)

formula_to_num(t::EigeniusMirror.FormulaTerm_LitFloat) = t.value

function formula_to_num(t::EigeniusMirror.FormulaTerm_App)
    # Walk the left spine: every `App(head, arg)` peels one arg off
    # until the head is no longer an `App`. Spine args come out
    # right-to-left from the outermost App; reverse for natural call
    # order.
    spine = Any[]
    cursor = t
    while cursor isa EigeniusMirror.FormulaTerm_App
        push!(spine, cursor.arg)
        cursor = cursor.head
    end
    if !(cursor isa EigeniusMirror.FormulaTerm_OpRef)
        error("EigeniusSymbolics: unsupported App head — expected OpRef, got $(typeof(cursor))")
    end
    op_iri = cursor.iri
    if !haskey(_OP_FN, op_iri)
        error("EigeniusSymbolics: operator `$op_iri` not in handler catalog; add it to _OP_FN to support it")
    end
    op_fn = _OP_FN[op_iri]
    args = reverse([formula_to_num(a) for a in spine])
    return op_fn(args...)
end

function formula_to_num(t::EigeniusMirror.FormulaTerm_OpRef)
    # A bare `OpRef` outside an App context could only be a constant
    # type / nullary value-operator. v1 doesn't lift any such use
    # case onto the chain; reject so a malformed term doesn't silently
    # produce something Symbolics won't understand.
    error("EigeniusSymbolics: bare OpRef `$(t.iri)` outside an App spine is unsupported in v1")
end

formula_to_num(t::EigeniusMirror.FormulaTerm_Lam) =
    error("EigeniusSymbolics: Lam binder is unsupported in v1 simplification — typed binders belong on operator signatures, not value-side terms")

formula_to_num(t::EigeniusMirror.FormulaTerm_Pi) =
    error("EigeniusSymbolics: Pi binder is unsupported in v1 simplification — typed binders belong on operator signatures, not value-side terms")

# ─── The handler ────────────────────────────────────────────────────────

"""
    validate_simplifies_to(s::SimplifiesTo) -> Dict

Verify the SimplifiesTo claim by re-running Symbolics' simplifier and
comparing structurally against the claim's `simplified` form.

Returns the canonical Verdict shape:
- `Holds` when `simplify(expr) == claimed`.
- `Undecidable` when they differ (Symbolics' `simplify` is heuristic
  per D27 §4.1.1; disagreement of representations does not imply
  algebraic non-equivalence).

`Fails` is reserved for a future strict-equivalence path (groebner-
basis / polynomial canonicalisation per D27 §4.1.1) that can decide
non-equivalence over restricted fragments.
"""
function validate_simplifies_to(s::EigeniusMirror.SimplifiesTo)
    expr_num = formula_to_num(s.expr.term)
    claimed_num = formula_to_num(s.simplified.term)
    actual = Symbolics.simplify(expr_num)
    if isequal(actual, claimed_num)
        return _verdict("Holds")
    else
        return _verdict("Undecidable")
    end
end

const VERDICT_CLASS_IRI = "urn:eigenius:institution:Verdict"
const IS_A_PROP = "urn:eigenius:core:is_a"
const CTOR_NAME_PROP = "urn:eigenius:core:ctor_name"

_verdict(ctor::AbstractString) = Dict{String, Any}(
    IS_A_PROP => [VERDICT_CLASS_IRI],
    CTOR_NAME_PROP => ctor,
)

# ─── Symbolics → FormulaTerm encoder (Phase 19d.3) ──────────────────────
#
# Inverse of `formula_to_num`. Walks a `Symbolics.Num` (post-simplify)
# and produces the chain-shaped FormulaTerm value the mirror can
# encode. Required for OnDemand handlers like `qc_symb_simplify` whose
# output type is a `SymbolicExpression` carrying a typed `term`.
#
# The traversal dispatches on the underlying SymbolicUtils term shape
# rather than on `Symbolics.Num` directly: a `Num` is a thin wrapper
# around a `BasicSymbolic`, and pattern-matching the Sym / Term /
# Number cases is what gives us the round-trip.
#
# This map is the inverse of `_OP_FN`. We use `IdDict` keyed on the
# Julia function values themselves — `SymbolicUtils.operation(t)`
# returns the actual function (`+`, `sin`, …), not a symbol or string.
# Adding a new operator requires entries in BOTH directions: in `_OP_FN`
# above (decode side) and in `_FN_TO_IRI` here (encode side).
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
by the mirror. The resulting tree round-trips through
`formula_to_num` to a structurally-equal `Num`.

`App` nodes are emitted left-spined: `App(App(OpRef(op), arg1), arg2)`
for binary `op(arg1, arg2)`, matching the spine the decoder walks.
"""
num_to_formula(n::Symbolics.Num) = num_to_formula(Symbolics.value(n))

function num_to_formula(v)
    # Plain numeric leaf — Float, Int, Rational, etc. all coerce
    # losslessly to the chain's `LitFloat` payload.
    if v isa Real
        return EigeniusMirror.FormulaTerm_LitFloat(Float64(v))
    end
    # SymbolicUtils 4.x wraps surviving numeric constants as
    # `BSImpl.Const` BasicSymbolics — neither `issym` nor `iscall`
    # matches them. Detect and unwrap to a flat `LitFloat`.
    if SymbolicUtils.isconst(v)
        return EigeniusMirror.FormulaTerm_LitFloat(Float64(SymbolicUtils.unwrap_const(v)))
    end
    # Symbolic variable.
    if SymbolicUtils.issym(v)
        return EigeniusMirror.FormulaTerm_Var(string(SymbolicUtils.nameof(v)))
    end
    # Function application — left-spine the args.
    if SymbolicUtils.iscall(v)
        op = SymbolicUtils.operation(v)
        args = SymbolicUtils.arguments(v)
        # A callable Sym applied to args (e.g. a time-dependent
        # variable `A(t)` from MTK/Catalyst inputs) reifies to a
        # flat `Var` carrying the Sym's name — the chain's
        # FormulaTerm has no time-dependence in its variable layer.
        # Symbolics-only usage typically never produces this shape,
        # but keeping the branch in lockstep with the duplicate in
        # `EigeniusCatalyst` preserves the invariant that both
        # translators agree on FormulaTerm encoding.
        if SymbolicUtils.issym(op)
            return EigeniusMirror.FormulaTerm_Var(string(SymbolicUtils.nameof(op)))
        end
        op_iri = get(_FN_TO_IRI, op, nothing)
        if op_iri === nothing
            error("EigeniusSymbolics: Symbolics produced operation `$op` with no FormulaTerm encoding; add it to _FN_TO_IRI")
        end
        # SymbolicUtils represents associative ops (`*`, `+`, `min`,
        # `max`, …) as n-ary internally; the chain's operator catalog
        # declares them as binary. Left-fold: `op(a, b, c, …)` →
        # `op(op(op(a, b), c), …)` so each App-spine the chain
        # validator collects carries exactly the operator's declared
        # arity (binary for `mul`/`add`, unary for `neg`/`sin`/etc.).
        # Same fold lives in `EigeniusCatalyst` — duplicated
        # translators per the existing TODO(common-package).
        n_args = length(args)
        n_args >= 1 ||
            error("EigeniusSymbolics: operator `$op` produced zero arguments — Symbolics shouldn't emit nullary calls")
        if n_args == 1
            return EigeniusMirror.FormulaTerm_App(
                EigeniusMirror.FormulaTerm_OpRef(op_iri),
                num_to_formula(args[1]),
            )
        end
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
    error("EigeniusSymbolics: cannot encode Symbolics term of type $(typeof(v)) as FormulaTerm")
end

# ─── Comorphism source-side: extract_term (Symbolics → Intervals) ────────

export extract_term

"""
    extract_term(expr::SymbolicExpression) -> FormulaTerm

Source-side `extract_typed` for the `Symbolics -> IntervalArithmetic`
Comorphism (D14 §9.3 step 2). Pulls the `term` field off a
`SymbolicExpression`, surfacing the chain-typed FormulaTerm as the
typed payload that flows through the comorphism's middle and into
the IntervalArithmetic-side reify.

Identity-on-FormulaTerm — both ends of this comorphism speak the same
flat formula language (D32 §6 generalised), so the extraction is just
the field accessor. The Julia function exists so the kernel's
`Exp::InstitutionInvoke` dispatch can route comorphism extract calls
through the substrate uniformly with `query` and `reify`, rather than
special-casing field-projection procedures kernel-side.
"""
function extract_term(expr::EigeniusMirror.SymbolicExpression)
    return expr.term
end

# ─── OnDemand: simplify_expression (qc_symb_simplify) ───────────────────

@static if isdefined(EigeniusMirror, :SimplifyRequest)

export simplify_expression

"""
    simplify_expression(req::SimplifyRequest) -> SymbolicExpression

Simplify the input expression and return a fresh `SymbolicExpression`
whose `term` is the simplified form re-encoded as a FormulaTerm.
Dispatched by the `qc_symb_simplify` OnDemand QueryClass via FIBER.

The `_id` keyword on the returned mirror struct is `nothing` because
this is a synthesised result, not a chain-committed resource — the
caller (or a downstream FIBER step) stamps an IRI on it before any
chain commit.
"""
function simplify_expression(req::EigeniusMirror.SimplifyRequest)
    simplified_num = Symbolics.simplify(formula_to_num(req.expr.term))
    simplified_term = num_to_formula(simplified_num)
    # `_id` defaults to `nothing` on the generated constructor — a
    # synthesised result, not a chain-committed resource. Caller (or
    # downstream FIBER step) stamps an IRI before any commit.
    return EigeniusMirror.SymbolicExpression(simplified_term)
end

end # @static if isdefined(EigeniusMirror, :SimplifyRequest)

# ─── Decidable: check_equivalence (qc_symb_check_equivalence) ───────────

@static if isdefined(EigeniusMirror, :EquivalenceCheck)

export check_equivalence

"""
    check_equivalence(check::EquivalenceCheck) -> Verdict

Simplify both `lhs` and `rhs` under the institution's pinned
rewriter and return a Verdict. `Holds` when the simplified forms
are structurally equal via `isequal`; `Undecidable` otherwise.
`Fails` is reserved for a future strict-decision path (groebner-
basis / polynomial canonicalisation) — Symbolics' `simplify` is
heuristic, so non-equality of representations does not imply
algebraic non-equivalence (D27 §4.1.1).

Decidable role: `Exp::NativeDecide` invokes this handler during
type-check reduction. On `Holds` the constraint reduces to `Refl`,
on `Undecidable` it stays a passthrough.
"""
function check_equivalence(check::EigeniusMirror.EquivalenceCheck)
    lhs_simplified = Symbolics.simplify(formula_to_num(check.lhs.term))
    rhs_simplified = Symbolics.simplify(formula_to_num(check.rhs.term))
    if isequal(lhs_simplified, rhs_simplified)
        return _verdict("Holds")
    else
        return _verdict("Undecidable")
    end
end

end # @static if isdefined(EigeniusMirror, :EquivalenceCheck)

# ─── AutoOnLoad: validate_satisfies_equation (SatisfiesEquation) ────────

@static if isdefined(EigeniusMirror, :SatisfiesEquation)

export validate_satisfies_equation

"""
    validate_satisfies_equation(s::SatisfiesEquation) -> Verdict

Verify a `SatisfiesEquation` claim. Builds a Symbolics substitution
dict from `s.bindings`, applies it to both `lhs` and `rhs` of
`s.equation`, simplifies the residue `lhs_subbed - rhs_subbed`, and
returns:

- `Holds` when the residue simplifies to zero (or `isequal` to a
  zero literal) — algebraic equivalence under substitution.
- `Undecidable` otherwise — `Symbolics.simplify` is heuristic
  (D27 §4.1.1), so a non-zero representation does not refute the
  claim. `Fails` is reserved for a future strict-decision path.

The substitution map is keyed on `Symbolics.Num` variables produced
by `_get_or_make_var`, the same cache `formula_to_num` populates —
so a binding for variable name `\"x\"` matches every
`FormulaTerm_Var(\"x\")` somewhere in the equation, regardless of
where it appears.
"""
function validate_satisfies_equation(s::EigeniusMirror.SatisfiesEquation)
    # Build the substitution map. Keys are `Symbolics.Num` variable
    # tokens; values are the bound expressions translated to `Num`.
    sub_map = Dict{Any, Any}()
    for binding in s.bindings
        var_num = _get_or_make_var(binding.variable)
        sub_map[var_num] = formula_to_num(binding.expression.term)
    end

    lhs_num = formula_to_num(s.equation.lhs.term)
    rhs_num = formula_to_num(s.equation.rhs.term)

    # `Symbolics.substitute` accepts an empty dict (returns the term
    # unchanged) — empty `bindings` reduces to a pure equivalence
    # check on the equation's two sides.
    lhs_subbed = isempty(sub_map) ? lhs_num : Symbolics.substitute(lhs_num, sub_map)
    rhs_subbed = isempty(sub_map) ? rhs_num : Symbolics.substitute(rhs_num, sub_map)

    residue = Symbolics.simplify(lhs_subbed - rhs_subbed)

    # Coerce residue to a value we can ask `iszero` of. After
    # simplification, a numerically-zero residue is either the literal
    # `0`, a zero-`Num`, or a `BasicSymbolic` whose `isequal` matches a
    # zero. `iszero` handles the numeric and Num cases; we add an
    # `isequal(residue, 0)` fallback for the symbolic-zero case the
    # rewriter produces when it cancels typed terms.
    if iszero(residue) || isequal(residue, 0) || isequal(residue, Symbolics.Num(0))
        return _verdict("Holds")
    else
        return _verdict("Undecidable")
    end
end

end # @static if isdefined(EigeniusMirror, :SatisfiesEquation)

# ─── AutoOnLoad: validate_substitutes (Substitutes) ─────────────────────

@static if isdefined(EigeniusMirror, :Substitutes)

export validate_substitutes

"""
    validate_substitutes(s::Substitutes) -> Verdict

Verify a `Substitutes` claim. Runs
`Symbolics.substitute(target, var => replacement)`, simplifies both
the actual and claimed result, and returns:

- `Holds` when the simplified actual and claimed are structurally
  equal (`isequal`).
- `Undecidable` otherwise — `Symbolics.simplify` is heuristic
  (D27 §4.1.1), so disagreement of representations does not refute
  the claim.

Symmetric simplification is intentional: `substitute` may leave
the result in an unsimplified form (e.g. `0 * x` rather than `0`),
and the claimed result may have been authored in a non-canonical
form. Simplifying both sides before `isequal` recovers Holds in
those cases without pretending to refute non-trivial mismatches.
"""
function validate_substitutes(s::EigeniusMirror.Substitutes)
    target_num = formula_to_num(s.target.term)
    replacement_num = formula_to_num(s.replacement.term)
    var_num = _get_or_make_var(s.variable)

    actual = Symbolics.simplify(Symbolics.substitute(target_num, var_num => replacement_num))
    claimed = Symbolics.simplify(formula_to_num(s.result.term))

    if isequal(actual, claimed)
        return _verdict("Holds")
    else
        return _verdict("Undecidable")
    end
end

end # @static if isdefined(EigeniusMirror, :Substitutes)

# ─── AutoOnLoad: validate_symbolically_reduces_to ───────────────────────

@static if isdefined(EigeniusMirror, :SymbolicallyReducesTo)

export validate_symbolically_reduces_to

"""
    apply_strategy(strategy, expr)

Per-strategy dispatch from a `ReductionStrategy_*` mirror struct to
the underlying Symbolics rewriter operation. New strategies land as
additional methods here paired with a new `InductiveCtor` on the
chain-side `ReductionStrategy` inductive — the closed-set discipline
of inductives means a chain-committed claim with a missing strategy
is a chain-validation failure (Rule 16 catches it), not a
`MethodError` here.
"""
apply_strategy(::EigeniusMirror.ReductionStrategy_Simplify, expr) =
    Symbolics.simplify(expr)
apply_strategy(::EigeniusMirror.ReductionStrategy_Expand, expr) =
    Symbolics.expand(expr)

"""
    validate_symbolically_reduces_to(s::SymbolicallyReducesTo) -> Verdict

Verify a `SymbolicallyReducesTo` claim. Dispatches on `s.strategy`
to the matching Symbolics operation, applies it to the source, and
compares structurally against the claimed result. Symmetric
`simplify` on both sides absorbs non-canonical authoring (the
`Expand` policy still folds under `simplify` for now; a strict-
Fails policy on polynomial mismatch lands when the polynomial-
canonicalisation surface is wired).
"""
function validate_symbolically_reduces_to(s::EigeniusMirror.SymbolicallyReducesTo)
    source_num = formula_to_num(s.expr.term)
    claimed_num = formula_to_num(s.result.term)

    actual = apply_strategy(s.strategy, source_num)

    if isequal(Symbolics.simplify(actual), Symbolics.simplify(claimed_num))
        return _verdict("Holds")
    else
        return _verdict("Undecidable")
    end
end

end # @static if isdefined(EigeniusMirror, :SymbolicallyReducesTo)

# ─── Symbolics → JuMP comorphism (D27 §4.2 / D32 §6 / Phase 19f.1) ─────
#
# `frame_as_optimisation_problem(input::SymbolicsToJuMPInput) ->
# OptimisationProblem` is the operational backing of the chain-side
# `Symbolics → JuMP` comorphism. The handler reads the FormulaTerm out
# of `input.objective.term` (identity on the chain's flat formula
# language) and forwards the input's framing fields (variable_names,
# variable_bounds, sense, constraints) to a fresh `OptimisationProblem`.
#
# The handler is gated by `@static if isdefined(...)` because
# `OptimisationProblem` and `SymbolicsToJuMPInput` only land in
# `EigeniusMirror` when the consuming chain has the JuMP ontology
# committed and the mirror seed includes them. Symbolics-only
# deployments compile cleanly without these types.
@static if isdefined(EigeniusMirror, :SymbolicsToJuMPInput) &&
          isdefined(EigeniusMirror, :OptimisationProblem)

export frame_as_optimisation_problem

"""
    frame_as_optimisation_problem(input::SymbolicsToJuMPInput) -> OptimisationProblem

Frame a Symbolics-side optimisation candidate as a chain-typed
`jump:OptimisationProblem`. The path:

1. Read `input.objective.term` — the FormulaTerm wrapped in the
   SymbolicExpression. No translation: the chain's FormulaTerm is the
   shared formula language across every numerical institution
   (D32 §6).
2. Forward `input.variable_names`, `input.framing_variable_bounds`,
   `input.sense`, and `input.framing_constraints` unchanged. The
   `framing_*` properties already use JuMP-side resource types
   (`jump:VariableBound`, `jump:Constraint`) so the cross-institution
   boundary speaks one bound / constraint vocabulary.
3. Pack into `EigeniusMirror.OptimisationProblem(...)`.

The mirror generator types `OptimisationProblem.variable_bounds` as
`Vector{AbstractVariableBound}` and `.constraints` as
`Vector{AbstractConstraint}`. Julia's parametric types are invariant,
so the comprehensions / explicit casts are necessary even though the
input arrays already carry concrete subtype values.
"""
function frame_as_optimisation_problem(input::EigeniusMirror.SymbolicsToJuMPInput)
    # Both `framing_variable_bounds` and `framing_constraints` are
    # recommended (the chain rejects empty arrays at commit time, so
    # an unconstrained / unbounded framing omits the field), and the
    # mirror generator emits them as `Union{Vector{Abstract*}, Nothing}`.
    # The OptimisationProblem constructor accepts `nothing` for both —
    # pass-through preserves the abstract-typed array element shape
    # without coercing into the concrete-typed vector that Julia's
    # invariant parametric types would reject.
    return EigeniusMirror.OptimisationProblem(
        collect(String, input.variable_names),
        input.objective.term,
        String(input.sense);
        constraints = input.framing_constraints,
        variable_bounds = input.framing_variable_bounds,
    )
end

end # @static if isdefined(EigeniusMirror, :SymbolicsToJuMPInput)

end # module
