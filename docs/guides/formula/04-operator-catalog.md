# 4. The operator catalog

Every `OpRef` in a `FormulaTerm` value points at a chain-committed
`formulas:Operator` resource. The catalog of operators is itself
chain-resident — declared in
[`ontologies/formulas/formulas-ontology.json`](../../../ontologies/formulas/formulas-ontology.json)
as a set of `Operator` instances, each carrying a typed
`operator_signature` and an integer `operator_arity`.

Read [§4.3](#4-3-the-app-spine-arity-check) before relying on the
signature: the validator's rank check reads `operator_arity` only. The
signature is declared, dogfooded and inspectable, but v1 does not check
anything against it.

This chapter covers the v1 catalog, the signature shape, the App-spine
arity check, and how to author a new operator.

## 4.1. The v1 catalog

The bootstrap layer ships with **twenty** `formulas:Operator` instances:
three nullary *type* operators under `urn:eigenius:formulas:types:` —
`Real`, `Int`, `Bool` — and seventeen *function* operators under
`urn:eigenius:formulas:ops:`. This chapter's "seventeen" always means the
function group; the type group exists to be named inside signatures.

| IRI tail | Arity | Assoc. | Comm. | Signature |
|---|---|---|---|---|
| `add` | 2 | `n_ary` | true | `Real → Real → Real` |
| `sub` | 2 | `left` | false | `Real → Real → Real` |
| `mul` | 2 | `n_ary` | true | `Real → Real → Real` |
| `div` | 2 | `left` | false | `Real → Real → Real` |
| `pow` | 2 | `right` | false | `Real → Real → Real` |
| `neg` | 1 | — | — | `Real → Real` |
| `abs` | 1 | — | — | `Real → Real` |
| `exp` | 1 | — | — | `Real → Real` |
| `log` | 1 | — | — | `Real → Real` |
| `sin` | 1 | — | — | `Real → Real` |
| `cos` | 1 | — | — | `Real → Real` |
| `tan` | 1 | — | — | `Real → Real` |
| `sqrt` | 1 | — | — | `Real → Real` |
| `eq` | 2 | — | true | `Real → Real → Bool` |
| `lt` | 2 | — | false | `Real → Real → Bool` |
| `le` | 2 | — | false | `Real → Real → Bool` |
| `derivative` | 2 | — | — | `(Real → Real) → Real → Real` |

**There is no `formulas:operator_symbol` property.** Operators carry
`core:short_name`, `formulas:operator_arity`,
`formulas:operator_signature`, and optionally
`formulas:operator_associativity` (`n_ary` / `left` / `right`) and
`formulas:operator_commutativity`. Surface spelling is not a chain
property: the five infix spellings `+ - * / ^` and prefix `-` are
hard-coded in the ESL Pratt parser
([§4.4](#4-4-authoring-a-new-operator)), and everything else is spelled as
a function call.

Note `derivative`'s signature: its first argument is itself a function
`Real → Real`, so the Pi spine is nested rather than flat. Its
`operator_arity` is still 2, and 2 is what the validator reads.

Full IRIs all live under `urn:eigenius:formulas:ops:` — e.g.
`urn:eigenius:formulas:ops:add`. Use those in `OpRef` constructors.

**Four of the seventeen are declared but uninterpreted.** `eq`, `lt`, `le`
and `derivative` appear in no Julia institution's operator map. A term using
one commits cleanly — the arity is declared and correct — and fails at
dispatch with a per-institution message naming the map that needs extending.
The catalog and the interpreters are separately maintained and nothing checks
that they agree.

## 4.2. The signature shape

An operator's `operator_signature` is a `FormulaTerm` itself — specifically,
a `Pi`-spine. For example, the binary `add: Real → Real → Real` is encoded
as:

```json
{
  "@id": "urn:eigenius:formulas:ops:add",
  "urn:eigenius:core:is_a": ["urn:eigenius:formulas:Operator"],
  "urn:eigenius:core:short_name": "add",
  "urn:eigenius:formulas:operator_arity": 2,
  "urn:eigenius:formulas:operator_associativity": "n_ary",
  "urn:eigenius:formulas:operator_commutativity": true,
  "urn:eigenius:formulas:operator_signature": {
    "core:is_a": ["formulas:FormulaTerm-Pi"],
    "formulas:FormulaTerm-Pi-name": "_",
    "formulas:FormulaTerm-Pi-ty": {
      "core:is_a": ["formulas:FormulaTerm-OpRef"],
      "formulas:FormulaTerm-OpRef-iri": "urn:eigenius:formulas:types:Real"
    },
    "formulas:FormulaTerm-Pi-body": {
      "core:is_a": ["formulas:FormulaTerm-Pi"],
      "formulas:FormulaTerm-Pi-name": "_",
      "formulas:FormulaTerm-Pi-ty": {
        "core:is_a": ["formulas:FormulaTerm-OpRef"],
        "formulas:FormulaTerm-OpRef-iri": "urn:eigenius:formulas:types:Real"
      },
      "formulas:FormulaTerm-Pi-body": {
        "core:is_a": ["formulas:FormulaTerm-OpRef"],
        "formulas:FormulaTerm-OpRef-iri": "urn:eigenius:formulas:types:Real"
      }
    }
  }
}
```

Two spellings to get right. The type reference is
`urn:eigenius:formulas:**types**:Real`, not `…:ops:Real` — the type
operators live in their own IRI segment. And `operator_arity` sits
alongside the signature as a redundant integer; it is redundant in
principle and load-bearing in practice, because it is the only thing
[§4.3](#4-3-the-app-spine-arity-check) reads.

Read the spine right-to-left for the function-type form: the innermost
`Real` is the return type; each enclosing `Pi(_, Real, …)` adds an
input. Two `Pi` binders means a 2-argument operator.

This is **dogfooding** the formula language — operator signatures live
in the same chain shape they describe. The `Pi` constructor is on the
chain because operator signatures need it; once it's there, the rest
of EigenTT-style typing comes along for free.

(The `Real` reference is itself an `OpRef` naming a nullary `Operator`
under `urn:eigenius:formulas:types:`. The chain-resident type system for
*types* of arithmetic values is intentionally minimal in v1.)

## 4.3. The App-spine arity check

Every `App` spine in a chain-committed `FormulaTerm` value gets
**rank-checked** at commit time by validation Rule 17
([`kernel/src/validation/rules/inductive.rs`](../../../kernel/src/validation/rules/inductive.rs),
`check_formula_term_arity`). What it actually does:

1. The rule fires only on a property whose declaration carries
   `core:data_type: core:inductive` *and* whose `core:class_types` is
   exactly the one-element list `[urn:eigenius:formulas:FormulaTerm]` — a
   literal string comparison against a pinned IRI constant.
2. It walks the value tree. Entering an `App` node it collects the whole
   left spine down to its head.
3. If the head is an `OpRef`, it resolves the named IRI in the layer and
   reads `formulas:operator_arity` off whatever it finds.
4. It compares that integer to the number of spine arguments with `!=`.
5. On a mismatch it emits `ValidationRule::OperatorArityMismatch` with the
   message `<path>: operator \`<iri>\` declares arity N; App spine supplies
   M arg(s)`, where `<path>` accumulates `.args[i]` segments from the
   property root.

So `App(App(OpRef(add), x), y)` is a 2-arg spine against `add`'s declared
arity 2: accepted. `App(App(App(OpRef(add), x), y), z)` is a 3-arg spine:
rejected.

**`operator_signature` is not read, and no `Pi` binder is ever counted.**
The code says so in as many words — "v1 ships arity-only". The `Operator`
class compounds this: it *requires* `operator_signature`, which the rule
ignores, and only *recommends* `operator_arity`, which is the sole property
it reads, so a spec-compliant operator declaration can be invisible to the
rule.

**Under-application is rejected too, not accepted as a partial
application.** The comparison is `!=`, in both directions, and a unit test
pins it: `App(OpRef(add), x)` against binary `add` is one error. What *is*
invisible is under-application nested inside a longer spine — intermediate
`App` nodes within a spine are deliberately not re-checked, because within
a spine they are partial applications rather than complete invocations.

**Every failed resolution is diagnosed.** `Validator::check_op_ref_head`
(`kernel/src/validation/rules/inductive.rs:302`) is a flat stage sequence,
and each stage that cannot proceed reports before returning:

| the operand | rule | message |
|---|---|---|
| is not a well-formed IRI | `UnknownOperator` | `` `OpRef` operand `…` is not a well-formed IRI `` |
| does not resolve in the layer chain | `UnknownOperator` | ``operator `…` does not resolve in the layer chain`` |
| resolves to something that is not a `formulas:Operator` | `UnknownOperator` | ``resolves to a resource that is not a `formulas:Operator` `` |
| declares a non-integer or negative `operator_arity` | `OperatorDeclarationMalformed` | ``declares an `operator_arity` that is not a non-negative integer`` |
| declares an arity ≠ the spine length | `OperatorArityMismatch` | ``declares arity N; App spine supplies M arg(s)`` |

The class check is subclass-aware — it goes through `is_instance_of_any`,
so a subclass of `formulas:Operator` passes.

Two stages still return without an error, and both are deliberate. A head
whose first argument is not a string is Rules 1 and 5's to reject, not this
one's. An operator resource with **no** `operator_arity` is
schema-conformant, since the ontology only *recommends* the property —
declaring it is what makes an operator arity-checkable.

Everything the rule does not recognise it traverses: the non-`App` arm walks
into whatever `args` array it finds, whatever the constructor is called.

## 4.4. Authoring a new operator

To add an operator (say, `min: Real → Real → Real`), commit a new
`Operator` resource on a layer. `operator_arity` is the property that makes
the operator visible to Rule 17; declare it even though the ontology only
*recommends* it. ESL form:

```esl
namespace formulas = "urn:eigenius:formulas";
namespace ops      = "urn:eigenius:formulas:ops";

resource ops:min : formulas:Operator {
    core:short_name             = "min";
    formulas:operator_arity     = 2;
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

**Validation rigour.** An `App` spine carrying the wrong number of
arguments is rejected before the runtime ever sees the value — the
difference between "handler error at dispatch" (slow, unclear) and
"validation error at commit" (fast, locality of blame). In v1 the *arity*
does that work and the signature is carried alongside it; per-argument type
checking against the `Pi` spine is the follow-on the signature is already
in place for.

**Cross-institution agreement.** If Symbolics' handler thinks `add` is
binary and DiffEq's thinks it's variadic, their views of the same chain
value diverge. With the signature on the chain — and both handlers
walking the same canonical encoding — they stay in agreement by
construction. The chain is the source of truth.

A third reason worth flagging for the medium term: **calculus and
beyond.** `derivative: (Real → Real) → Real → Real` is in the v1 catalog as
a placeholder; its signature is honest about its shape even though no v1
institution evaluates it. Adding richer dependent-typed operators —
indexed sums, parameterised quantifiers — slots into the same shape.

---

Next: **[5. The ESL `formula(...)` sublanguage →](05-esl-sublanguage.md)**
