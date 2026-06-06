# 3. Eigon-JSON embedding

A `FormulaTerm` value is encoded on the chain as a recursive
**tagged-dict tree**. This chapter is the encoding reference: every shape
your value can take, the validator's rule for each, and the worked
examples for the most common patterns.

## 3.1. The encoding rule

Every `FormulaTerm` value is a JSON object of exactly two fields:

```json
{
  "ctor": "<CtorName>",
  "args": [ <arg₀>, <arg₁>, ... ]
}
```

`<CtorName>` is one of the six constructor names declared in
[`formulas-ontology.json`](../../../ontologies/formulas/formulas-ontology.json):
`Var`, `LitFloat`, `OpRef`, `App`, `Lam`, `Pi`. `args` is an ordered
list whose contents and types match the ctor's declared `arg_types`.

| Ctor | `args` shape |
|---|---|
| `Var(name)` | `["x"]` — one string |
| `LitFloat(value)` | `[2.5]` — one float |
| `OpRef(iri)` | `["urn:eigenius:formulas:ops:add"]` — one IRI string |
| `App(head, arg)` | `[<head FormulaTerm>, <arg FormulaTerm>]` — two recursive nodes |
| `Lam(name, ty, body)` | `["x", <FormulaTerm>, <FormulaTerm>]` — string + two recursive nodes |
| `Pi(name, ty, body)` | `["x", <FormulaTerm>, <FormulaTerm>]` — string + two recursive nodes |

That's the whole rule. Recursive descent down `App` heads and arguments
gives you the full tree.

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

```json
{ "ctor": "App",
  "args": [
    { "ctor": "App",
      "args": [
        { "ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:mul"] },
        { "ctor": "App",
          "args": [
            { "ctor": "App",
              "args": [
                { "ctor": "OpRef", "args": ["urn:eigenius:formulas:ops:add"] },
                { "ctor": "Var", "args": ["x"] }
              ] },
            { "ctor": "LitFloat", "args": [0.0] }
          ] }
      ] },
    { "ctor": "LitFloat", "args": [1.0] }
  ]
}
```

Reading bottom-up:

1. `Var("x")` and `LitFloat(0.0)` are the leaves of the inner tree.
2. `App(App(OpRef(add), Var("x")), LitFloat(0.0))` is `add(x, 0)`,
   i.e. `x + 0`.
3. `App(App(OpRef(mul), <that>), LitFloat(1.0))` is `mul(x+0, 1)`,
   i.e. `(x + 0) * 1`.

## 3.4. The validator's inductive-value rule

When a chain commit lands carrying a `core:inductive`-typed property
referencing `formulas:FormulaTerm` (or any inductive type), the
validator does the following per node (D32 §3.5):

1. Read the `ctor` field. Look it up in the inductive's declared
   constructor list. If the name isn't a declared ctor, reject with
   `expected one of: Var, LitFloat, OpRef, App, Lam, Pi; got: <bad>`.
2. Read the `args` field. Match its length against the ctor's declared
   `arg_types`. If the count doesn't match, reject with arity mismatch.
3. For each `args[i]`, type-check it against `arg_types[i]`:
   - If the slot is a primitive (string, float, IRI), check the JSON
     type matches.
   - If the slot references another inductive, recurse into the value.
   - If the slot references a class (e.g. an `OpRef`'s IRI must resolve
     to a `formulas:Operator` resource), look the IRI up on the chain
     and confirm it exists with the expected `is_a`.
4. For `App`, *additionally* run the operator-arity rank check
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
- **Float literals are JSON numbers.** `3.14`, `0.0`, `-1.5`. The
  validator distinguishes int and float; `LitFloat` requires a floating
  representation. (For integer-valued literals you commonly want
  `LitFloat(2.0)`, not the JSON integer `2`.)
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
