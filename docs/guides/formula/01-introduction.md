# 1. Introduction

The formula language is a small, chain-resident, typed expression-tree
language that every numerical institution on the platform speaks. It sits
at `urn:eigenius:formulas:` — *not* under any one institution — and serves
three purposes:

1. **Cross-institution payload.** Symbolics, IntervalArithmetic, Catalyst,
   DiffEq, and JuMP-HiGHS each consume the same expression shape. A formula
   authored in one institution's vocabulary can flow into another via a
   comorphism whose typed middle is the *identity function on FormulaTerm*
   (D32 §6.2). The cost of bridging two domains drops from "write a custom
   translator" to "declare a comorphism resource."
2. **Validated chain commits.** A formula isn't a string. It's a typed
   tagged-dict tree where every node carries a constructor name and the
   validator type-checks each node against the ctor's declared argument
   types at commit time. Mismatched arity, unknown operators, free
   variables in unexpected positions — all rejected before the runtime
   ever sees the value.
3. **Authoring ergonomics.** Inside `formula(...)` blocks in ESL, you write
   the math the way you'd write it on paper: `1 + 2 * 3`, `sin(x)`,
   `(4 - 2*Ki)^2`. The compiler emits the typed tree.

## 1.1. The three-surface mental model

Three places the formula language shows up, and how they relate:

| Surface | Where you see it | What it is |
|---|---|---|
| **EigenTT fragment** | Inside the kernel's type theory | A subset of EigenTT `Exp` lifted onto the chain as an inductive type. Six constructors mirror EigenTT one-for-one. |
| **Eigon-JSON encoding** | Chain commits, the wire | A tagged-dict tree `{"ctor": "App", "args": [head, arg]}` recursively. The validator type-checks every node. |
| **ESL `formula(...)` sublanguage** | ESL program source | A Pratt-parsed math surface compiling to the encoded tree. `formula(x + 0)` produces `App(App(OpRef(+), Var(x)), LitFloat(0.0))`. |

The middle surface is the load-bearing one — it's the actual representation
on the chain. The first surface (EigenTT fragment) is the kernel's
internal handle on it; the third (ESL sublanguage) is the authoring
ergonomics. All three describe the same six-constructor expression-tree
language; the only difference is which form a given user is looking at.

## 1.2. Why FormulaTerm sits outside any one institution

Institutional cleanliness. If `FormulaTerm` lived under
`urn:eigenius:symbolics:` (where Symbolics-the-institution declares its
domain shapes), Catalyst couldn't speak it without reaching into Symbolics's
namespace; and a Symbolics → JuMP comorphism would have to *convert* the
expression tree from Symbolics's vocabulary into JuMP's. Since both
institutions actually speak the same expression-tree shape, the conversion
is unnecessary work.

Living at `urn:eigenius:formulas:` makes FormulaTerm a *shared* asset —
declared in the kernel bootstrap layer alongside `core:`, `program:`,
`reflection:`, `institution:`, and `notebook:`. Every chain has it from
startup. Every institution that consumes it does so as a peer; nobody
"owns" it.

Concretely, comorphisms between two FormulaTerm-speaking institutions
collapse to the identity function. The Symbolics → IntervalArithmetic
comorphism's `transformation` is `Lam(t: FormulaTerm. Var(t))` — D32 §6.2.
The chain bytes flow through unchanged. That's the architectural payoff.

## 1.3. What FormulaTerm is *not*

A few clarifications worth flagging up front, especially for readers
coming from EigenTT, ATP / SMT, or symbolic-algebra backgrounds:

- **It's not the full EigenTT.** The chain-resident form is a small
  fragment — six constructors, no induction principle, no universe
  hierarchy beyond what an inductive type already carries. The kernel's
  full EigenTT lives in [`kernel/src/nbe/`](../../../kernel/src/nbe/) and
  is not user-facing.
- **It's not a generic Lisp / s-expression.** Operators are *named on
  the chain* via `OpRef` constructor referencing a `formulas:Operator`
  resource that declares the operator's typed signature. Random IRIs in
  `OpRef` position get rejected at commit.
- **It's not the only inductive type on the chain.** The validator's
  inductive-value rule is generic — any `core:InductiveType` declared
  with constructors and argument types gets the same type-checking
  treatment. FormulaTerm just happens to be the most heavily-used one.
  Other inductives in the kinase notebook include `ReductionStrategy`
  (Symbolics), `ConstraintRelation` (JuMP), `Verdict` (institution).

## 1.4. The six constructors at a glance

A teaser; details in [chapter 2](02-mini-tt-fragment.md).

| Constructor | Args (EigenTT view) | Concrete example |
|---|---|---|
| `Var(name)` | one string | `x`, `Ki` |
| `LitFloat(value)` | one float | `0.0`, `2.5` |
| `OpRef(iri)` | one IRI | `formulas:ops:add`, `formulas:ops:sin` |
| `App(head, arg)` | two recursive nodes | `f(x)` is `App(f, x)` |
| `Lam(name, ty, body)` | binder + two nodes | `λx:Real. x + 1` |
| `Pi(name, ty, body)` | binder + two nodes | `Real → Real`, `Real → Real → Real` |

The first four are expression-shaped (what you'd write inside a
`formula(...)` block); the last two are binders that show up in operator
signatures and (less commonly) in formulas that need quantification.

## 1.5. Concrete first taste

A `SymbolicExpression` resource carrying the formula `(x + 0) * 1`,
written in Eigon-JSON:

```json
{
  "@id": "urn:eigenius:demo:expr:x_plus_0_times_1",
  "core:is_a": ["urn:eigenius:symbolics:SymbolicExpression"],
  "symbolics:term": {
    "ctor": "App",
    "args": [
      {"ctor": "App", "args": [
        {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:mul"]},
        {"ctor": "App", "args": [
          {"ctor": "App", "args": [
            {"ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:add"]},
            {"ctor": "Var", "args": ["x"]}
          ]},
          {"ctor": "LitFloat", "args": [0.0]}
        ]}
      ]},
      {"ctor": "LitFloat", "args": [1.0]}
    ]
  }
}
```

The same value authored inside an ESL program:

```esl
resource ex:expr : symbolics:SymbolicExpression {
    symbolics:term = formula((x + 0) * 1);
}
```

Both forms produce identical chain bytes. The ESL form is what you'll
write 99% of the time; the Eigon-JSON form is what the chain stores and
what handlers in worker containers see when they decode the resource.

---

Next: **[2. The EigenTT fragment →](02-mini-tt-fragment.md)**
