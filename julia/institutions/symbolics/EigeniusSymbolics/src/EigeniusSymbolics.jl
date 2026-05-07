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

end # module
