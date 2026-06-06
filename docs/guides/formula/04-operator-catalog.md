# 4. The operator catalog

Every `OpRef` in a `FormulaTerm` value points at a chain-committed
`formulas:Operator` resource. The catalog of operators is itself
chain-resident — declared in
[`ontologies/formulas/formulas-ontology.json`](../../../ontologies/formulas/formulas-ontology.json)
as a set of `Operator` instances, each carrying its symbol and a typed
`operator_signature` that the validator rank-checks against.

This chapter covers the v1 catalog, the signature shape, the App-spine
arity check, and how to author a new operator.

## 4.1. The v1 catalog

The bootstrap layer ships with seventeen operators covering most
arithmetic and a few comparison/calculus operations:

| IRI tail | Symbol | Arity | Signature |
|---|---|---|---|
| `add` | `+` | 2 | `Real → Real → Real` |
| `sub` | `-` (binary) | 2 | `Real → Real → Real` |
| `mul` | `*` | 2 | `Real → Real → Real` |
| `div` | `/` | 2 | `Real → Real → Real` |
| `pow` | `^` | 2 | `Real → Real → Real` |
| `neg` | `-` (unary) | 1 | `Real → Real` |
| `abs` | `abs` | 1 | `Real → Real` |
| `exp` | `exp` | 1 | `Real → Real` |
| `log` | `log` | 1 | `Real → Real` |
| `sin` | `sin` | 1 | `Real → Real` |
| `cos` | `cos` | 1 | `Real → Real` |
| `tan` | `tan` | 1 | `Real → Real` |
| `sqrt` | `sqrt` | 1 | `Real → Real` |
| `eq` | `=` (boolean) | 2 | `Real → Real → Bool` |
| `lt` | `<` | 2 | `Real → Real → Bool` |
| `le` | `≤` | 2 | `Real → Real → Bool` |
| `derivative` | `d/dx` | 2 | `Real → Real → Real` |

Full IRIs all live under `urn:eigenius:formulas:ops:` — e.g.
`urn:eigenius:formulas:ops:add`. Use those in `OpRef` constructors.

## 4.2. The signature shape

An operator's `operator_signature` is a `FormulaTerm` itself — specifically,
a `Pi`-spine. For example, the binary `add: Real → Real → Real` is encoded
as:

```json
{
  "@id": "urn:eigenius:formulas:ops:add",
  "core:is_a": ["urn:eigenius:formulas:Operator"],
  "core:short_name": "add",
  "formulas:operator_symbol": "+",
  "formulas:operator_signature": {
    "ctor": "Pi",
    "args": [
      "_",
      {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:Real"]},
      {"ctor": "Pi",
       "args": [
         "_",
         {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:Real"]},
         {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:Real"]}
       ]}
    ]
  }
}
```

Read the spine right-to-left for the function-type form: the innermost
`Real` is the return type; each enclosing `Pi(_, Real, …)` adds an
input. Two `Pi` binders means a 2-argument operator.

This is **dogfooding** the formula language — operator signatures live
in the same chain shape they describe. The `Pi` constructor is on the
chain because operator signatures need it; once it's there, the rest
of EigenTT-style typing comes along for free.

(The `Real` reference is itself an `OpRef` for now — a built-in
typed-as-an-operator-reference. The chain-resident type system for
*types* of arithmetic values is intentionally minimal in v1.)

## 4.3. The App-spine arity check

Every `App` spine in a chain-committed `FormulaTerm` value gets
**rank-checked** against the operator's `Pi`-spine signature at commit
time (D32 §5.4):

1. The validator walks the leftmost `App` chain down to the bottom.
2. The bottom must be an `OpRef` resolving to a chain-committed
   `formulas:Operator`. If it's not, reject with `unknown operator IRI`.
3. Read the operator's `operator_signature`. Count the `Pi` binders
   along the leftmost spine.
4. Count the arguments along the original `App` spine.
5. The two counts must match. If you have *more* args than `Pi`
   binders, reject with `OperatorArityMismatch: <op>: expected N args,
   got M` (where `N` is the binder count). If *fewer*, the value is
   partially applied — valid as a value but won't type-check in
   contexts expecting a result of the operator's return type.

So `App(App(OpRef(add), x), y)` is a 2-arg App-spine, matches `add`'s
2-arg signature: accepted. `App(App(App(OpRef(add), x), y), z)` has a
3-arg spine against `add`'s 2-arg signature: rejected.

## 4.4. Authoring a new operator

To add an operator (say, `min: Real → Real → Real`), commit a new
`Operator` resource on a layer. ESL form:

```esl
namespace formulas = "urn:eigenius:formulas";
namespace ops      = "urn:eigenius:formulas:ops";

resource ops:min : formulas:Operator {
    core:short_name           = "min";
    formulas:operator_symbol  = "min";
    formulas:operator_signature = formula(
        Real -> Real -> Real
    );
}
```

Two follow-on changes are needed before the operator is *useful*:

1. **Update the institution handlers that walk FormulaTerm.** The
   substrate-side decoder in `EigeniusMirror.jl` is auto-generated, so
   the ctor structs gain no new variant; but the per-handler walker
   functions (`formula_to_num` in Symbolics, `formula_to_interval` in
   IntervalArithmetic, `formula_to_jump` in JuMP-HiGHS, `formula_to_value`
   in DiffEq) carry an operator-IRI-to-Julia-function map that needs a
   new entry per operator. Without that, the worker will reject with
   `unknown operator urn:eigenius:formulas:ops:min`.
2. **Update the `formula(...)` ESL parser if you want surface syntax.**
   The Pratt parser in [`kernel/src/esl/parser.rs`](../../../kernel/src/esl/parser.rs)
   accepts `+ - * / ^` with hard-coded precedence and any other operator
   as a function-call (`min(a, b)`). New operators reachable as
   function-calls don't need parser changes; new operators with infix
   syntax do.

Adding an operator without these handler updates is fine for chain-side
correctness — the values type-check — but they'll fail at dispatch
when an institution tries to interpret them.

## 4.5. Why operators carry typed signatures on the chain

Two reasons.

**Validation rigour.** Without a typed signature the validator could
only check that an `OpRef` resolves to *something*; it couldn't catch
arity mismatches at commit time. With it, an `App` spine carrying the
wrong number of arguments is rejected before the runtime ever sees the
value. That's the difference between "handler error at dispatch" (slow,
unclear) and "validation error at commit" (fast, locality of blame).

**Cross-institution agreement.** If Symbolics' handler thinks `add` is
binary and DiffEq's thinks it's variadic, their views of the same chain
value diverge. With the signature on the chain — and both handlers
walking the same canonical encoding — they stay in agreement by
construction. The chain is the source of truth.

A third reason worth flagging for the medium term: **calculus and
beyond.** `derivative: Real → Real → Real` is in the v1 catalog as a
placeholder; its signature is honest about its arity even if no v1
institution evaluates it. Adding richer dependent-typed operators —
indexed sums, parameterised quantifiers — slots into the same shape.

---

Next: **[5. The ESL `formula(...)` sublanguage →](05-esl-sublanguage.md)**
