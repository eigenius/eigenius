# 3. Eigon-JSON embedding

A `FormulaTerm` value is a **resource**, and so is every subterm of it.
This chapter is the encoding reference: every shape your value can take,
the validator's rule for each, and the worked examples for the most
common patterns.

## 3.1. The encoding rule

A value's `is_a` names the **constructor's class**, and each argument is
a **named property** on that class (D85 §6.1):

```json
{
  "urn:eigenius:core:is_a": ["urn:eigenius:formulas:FormulaTerm-<CtorName>"],
  "urn:eigenius:formulas:FormulaTerm-<CtorName>-<argName>": <arg>,
  ...
}
```

`<CtorName>` is one of the six constructor names declared in
[`formulas-ontology.json`](../../../ontologies/formulas/formulas-ontology.json):
`Var`, `LitFloat`, `OpRef`, `App`, `Lam`, `Pi`. `<argName>` is the
`arg_name` that constructor's declaration gives the argument. Both
classes and properties are **derived from the inductive's declaration**
when the layer is built — you never write them by hand in an ontology,
and you cannot name one the declaration does not have.

| Ctor | Properties |
|---|---|
| `Var(name)` | `FormulaTerm-Var-name`: a string |
| `LitFloat(value)` | `FormulaTerm-LitFloat-value`: a float |
| `OpRef(iri)` | `FormulaTerm-OpRef-iri`: an IRI string |
| `App(head, arg)` | `FormulaTerm-App-head`, `FormulaTerm-App-arg`: two nested values |
| `Lam(name, ty, body)` | `FormulaTerm-Lam-name`: a string; `-ty`, `-body`: nested values |
| `Pi(name, ty, body)` | `FormulaTerm-Pi-name`: a string; `-ty`, `-body`: nested values |

That's the whole rule. There is no `ctor` field and no `args` list:
the class says which constructor it is, and the properties say what its
arguments are. Argument ORDER lives in the declaration, so reading a
value back positionally means reading the declaration too — which is why
the kernel's one read of a value, `ctor_and_args`, takes the layer.

## 3.2. Multi-argument operators curry

The chain doesn't have a variadic `App`. Multi-arg operators are
**curried via left-spined `App`s**, mirroring EigenTT's binary application
discipline directly (D32 §4.1).

| Surface | FormulaTerm |
|---|---|
| `f(a)` | `App(OpRef(f), a)` |
| `f(a, b)` | `App(App(OpRef(f), a), b)` |
| `f(a, b, c)` | `App(App(App(OpRef(f), a), b), c)` |
| `f(g(a, b), c)` | `App(App(OpRef(f), App(App(OpRef(g), a), b)), c)` |

The leftmost descent always reaches the `OpRef`. The arguments are read
right-to-left up the spine.

## 3.3. Worked example: `(x + 0) * 1`

Reading outermost-in: top-level is multiplication of two things. Left
arg is `x + 0`; right arg is `1`.

Property IRIs are abbreviated below with the `formulas:` prefix for
`urn:eigenius:formulas:` and `core:` for `urn:eigenius:core:`.

```json
{ "core:is_a": ["formulas:FormulaTerm-App"],
  "formulas:FormulaTerm-App-head": {
    "core:is_a": ["formulas:FormulaTerm-App"],
    "formulas:FormulaTerm-App-head": {
      "core:is_a": ["formulas:FormulaTerm-OpRef"],
      "formulas:FormulaTerm-OpRef-iri": "urn:eigenius:formulas:ops:mul"
    },
    "formulas:FormulaTerm-App-arg": {
      "core:is_a": ["formulas:FormulaTerm-App"],
      "formulas:FormulaTerm-App-head": {
        "core:is_a": ["formulas:FormulaTerm-App"],
        "formulas:FormulaTerm-App-head": {
          "core:is_a": ["formulas:FormulaTerm-OpRef"],
          "formulas:FormulaTerm-OpRef-iri": "urn:eigenius:formulas:ops:add"
        },
        "formulas:FormulaTerm-App-arg": {
          "core:is_a": ["formulas:FormulaTerm-Var"],
          "formulas:FormulaTerm-Var-name": "x"
        }
      },
      "formulas:FormulaTerm-App-arg": {
        "core:is_a": ["formulas:FormulaTerm-LitFloat"],
        "formulas:FormulaTerm-LitFloat-value": 0.0
      }
    }
  },
  "formulas:FormulaTerm-App-arg": {
    "core:is_a": ["formulas:FormulaTerm-LitFloat"],
    "formulas:FormulaTerm-LitFloat-value": 1.0
  }
}
```

Nobody writes that by hand. Author it in ESL (chapter 5) or build it
with the encoder; this is what the chain holds.

Reading bottom-up:

1. `Var("x")` and `LitFloat(0.0)` are the leaves of the inner tree.
2. `App(App(OpRef(add), Var("x")), LitFloat(0.0))` is `add(x, 0)`,
   i.e. `x + 0`.
3. `App(App(OpRef(mul), <that>), LitFloat(1.0))` is `mul(x+0, 1)`,
   i.e. `(x + 0) * 1`.

## 3.4. How the validator checks a value

There is no separate inductive-value rule. A value is a resource, so the
rules that check every resource check it (D85 §6.1):

1. **The slot.** `data_type: core:inductive` admits an embedded resource
   and nothing else, and `class_types: [formulas:FormulaTerm]` admits the
   constructors of that inductive — each derived class lists the
   inductive in its `parent_classes`. A constructor the declaration does
   not have has no class, so `is_a` names something that does not
   resolve.
2. **The arity.** A constructor class `requires` one property per
   declared argument. A missing argument is a missing required property.
3. **Each argument.** The derived property carries the argument's
   declared type: a primitive `data_type` for a string or float, an
   inductive one for a subterm, `class_types` for an IRI that has to
   resolve to a chain resource (an `OpRef`'s operator, say).
4. **Nested values.** Each argument that is a value is a resource in its
   own right, so the same three checks run on it.
5. For `App`, *additionally* the operator-arity rank check
   (described in [chapter 4](04-operator-catalog.md#43-the-app-spine-arity-check)).

If any step fails, the entire chain commit is rejected. The error
points at the resource and the property whose value carried the bad
node. Errors are surfaced in the `eigenius load`'s response.

## 3.5. Hand-authoring tips

- **Use ESL `formula(...)` if at all possible.** It's the hand-friendly
  surface and produces the same chain bytes ([chapter 5](05-esl-sublanguage.md)).
- **Curry left-to-right.** `f(a, b, c)` is
  `App(App(App(OpRef(f), a), b), c)` — left-spined, not right-spined.
  Reverse spines fail arity checks.
- **Float literals are floats.** `3.14`, `0.0`, `-1.5`. The validator
  distinguishes int and float; `LitFloat`'s argument is declared
  `core:float`. (For integer-valued literals you commonly want
  `LitFloat(2.0)`, not the integer `2`.)
- **`OpRef` requires a chain-resolved operator.** A typo in the IRI
  fails commit with "unknown operator IRI". The set of valid operators
  is whatever's declared as `formulas:Operator` on the chain (chapter 4).
- **Don't try to encode binders inside expression bodies.** `Lam` /
  `Pi` aren't the right tool for "the function `λx. x + 1`" *as data
  inside an arithmetic expression*; they're the binders used at the
  top of an operator signature or in a quantified clause. v1's authoring
  surfaces (`formula(...)` in ESL, the institution handlers) don't yet
  surface inline `Lam` / `Pi` cleanly; for v1 stick to `Var`, `LitFloat`,
  `OpRef`, `App` for arithmetic.

## 3.6. Reading the validator's error messages

A few of the most common rejection messages and what they mean:

| Message | Cause |
|---|---|
| `expected one of: Var, LitFloat, OpRef, App, Lam, Pi; got: Foo` | Misspelled or unknown ctor name |
| `args length mismatch: expected 2, got 3` | Wrong number of arguments for the constructor |
| `expected string at args[0]; got float` | Wrong primitive type in a leaf slot |
| `OperatorArityMismatch: formulas:ops:mul: expected 2 args, got 3` | An `App` spine has more args than the operator's signature has `Pi` binders (chapter 4) |
| `unknown operator urn:eigenius:formulas:ops:foo` | `OpRef`'s IRI doesn't resolve to a chain-committed `formulas:Operator` resource |

Most of these go away if you author through `formula(...)` — the ESL
parser's typed lowering is structurally correct by construction. The
exception is `OperatorArityMismatch` against an operator that's ambiguous
between binary and unary forms (e.g. unary minus vs binary subtraction);
the parser handles that, but a hand-authored value can still trip it.

---

Next: **[4. The operator catalog →](04-operator-catalog.md)**
