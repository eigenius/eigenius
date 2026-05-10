# 7. Common failure modes

The most common ways FormulaTerm authoring goes wrong, with the
diagnostic each produces and the fix. The validator catches most of
these at commit time; a few only surface when an institution handler
tries to interpret the value.

## 7.1. Validator-level failures (commit time)

These reject the chain commit before the worker runs.

### Unknown constructor name

```
expected one of: Var, LitFloat, OpRef, App, Lam, Pi; got: Foo
```

A FormulaTerm value's `ctor` field carries something that isn't one of
the six declared constructors. Usually a typo (`Apply` instead of
`App`, `OpRefC` instead of `OpRef`).

**Fix.** Match the ctor name to one declared on `formulas:FormulaTerm`.

### Wrong argument count

```
args length mismatch: expected 2, got 3
```

The `args` array of a FormulaTerm node has more or fewer entries than
the constructor's `arg_types` declares. Usually a hand-authoring
mistake — `App` takes two args, not three; `Lam` and `Pi` take three
(name + type + body), not two.

**Fix.** Match the array length to the constructor's declared arity.

### Operator arity mismatch (App-spine vs `Pi`-spine)

```
OperatorArityMismatch: formulas:ops:mul: expected 2 args, got 3
```

An `App` spine has more arguments than the operator's signature has
`Pi` binders. `App(App(App(OpRef(mul), x), y), z)` is `mul(x, y, z)`,
but `mul` is binary — `Pi(_, Real, Pi(_, Real, Real))` — so the third
arg has nowhere to go.

**Fix.** Either refactor the call (multi-arg ops are curried, not
n-ary — `mul(mul(x, y), z)` for 3-way multiplication) or check the
operator's signature is what you expected.

### Unknown operator IRI

```
unknown operator urn:eigenius:formulas:ops:foo
```

An `OpRef`'s IRI doesn't resolve to a chain-committed
`formulas:Operator` resource. Usually a typo, or an operator that's
declared in a layer not currently active.

**Fix.** Confirm the operator exists with `eigenius inspect
urn:eigenius:formulas:ops:<name>`. If it's missing, either commit the
operator declaration ([chapter 4 §4.4](04-operator-catalog.md#44-authoring-a-new-operator))
or use one that exists.

### Wrong primitive type in a leaf slot

```
expected string at args[0]; got float
```

A constructor's argument slot expected a string (e.g. `Var(name)` or
`OpRef(iri)`) but got a different JSON type. Usually the result of
hand-authoring with the wrong shape — e.g. `{"ctor": "Var", "args":
[42]}` instead of `{"ctor": "Var", "args": ["x"]}`.

**Fix.** Match the leaf slot's expected primitive type. The constructor
table in [chapter 2 §2.1](02-mini-tt-fragment.md#21-the-constructor-table)
lists each.

### Integer where a float is expected

A subtler form of the previous failure: `{"ctor": "LitFloat", "args":
[2]}` (integer `2`) gets rejected because `LitFloat` requires a float.
JSON's `2` and `2.0` parse to different Eigon-Value types.

**Fix.** Use `2.0` instead of `2` in `LitFloat` args.

## 7.2. Parser-level failures (compile time)

These only show up when authoring through `formula(...)` in ESL.

### Unexpected token inside `formula(...)`

```
formula sublanguage: unexpected token at position N: expected expression
```

A typo or unsupported syntax inside `formula(...)`. Common causes:
mistyping a paren, using comparison operators in expression position
(v1 doesn't have infix `<` / `>` inside `formula(...)`), or an
extension construct that the v1 Pratt parser doesn't accept.

**Fix.** Check the syntax against [chapter 5 §5.1](05-esl-sublanguage.md#51-the-surface).
For comparisons, use the function-call form: `lt(x, 2)` rather than
`x < 2`.

### Unary `-` outside `formula(...)`

```
unary `-` outside `formula(...)` only applies to numeric literals; got Var(...)
```

Outside `formula(...)`, the only valid use of unary `-` is on a
numeric literal — the backwards-compatible `ex:value = -1.5;` shape.
Inside `formula(...)`, unary `-` works on any expression. Trying
`ex:expr = -x;` outside `formula(...)` fails; inside, `formula(-x)`
is fine.

**Fix.** Wrap the expression in `formula(...)`, or rewrite to avoid
unary minus outside the sublanguage.

## 7.3. Handler-level failures (dispatch time)

These surface only when an institution tries to interpret the value.
They typically mean the handler doesn't know about an operator, or
the value contains a free variable the handler can't resolve.

### Unknown operator (handler-side)

```
EigeniusSymbolics: unknown operator urn:eigenius:formulas:ops:newop
```

The handler's per-operator dispatch map (e.g. `_OP_FN` in Symbolics,
`_OP_INTERVAL` in IntervalArithmetic, `_OP_JUMP` in JuMP-HiGHS) doesn't
have an entry for the operator. The validator accepted the FormulaTerm
because the operator *is* declared on the chain — but the handler
hasn't been updated to know about it.

**Fix.** Add an entry in the relevant Julia handler's per-operator
map, rebuild the env image, commit a new `RuntimeEnvironment` resource
([chapter 4 §4.4](04-operator-catalog.md#44-authoring-a-new-operator)).

### Free variable not in environment

```
EigeniusJuMPHiGHS: free variable `Kj` not declared in OptimisationProblem.variable_names
```

A `Var(name)` in the formula references a name that's not in the
handler's environment. For Symbolics, the handler walks the FormulaTerm
into a `Symbolics.Num` and any free variables become symbolic; for
IntervalArithmetic / JuMP / DiffEq, the handler expects the free
variables to be declared in a list (`variable_names`, `state_names`,
etc.) and resolves them by index.

**Fix.** Add the variable to the institution's declared name list, or
correct the typo in the formula.

### Per-institution evaluator restrictions

Each institution's walker has its own restrictions on what it can
interpret:

- **IntervalArithmetic** doesn't handle `Lam` / `Pi` inline — the
  walker is purely arithmetic. Don't put binders inside an
  `IntervalFunction.term`.
- **JuMP-HiGHS** doesn't accept transcendental functions (`sin`,
  `cos`, `exp`, `log`) — they produce `NonlinearExpr` which HiGHS
  rejects. The walker errors at dispatch, not at validation.
- **DiffEq's** walker assumes binary operators are arithmetic; the
  comparison operators (`eq`, `lt`, `le`) trigger an error at
  dispatch.

When in doubt, check the walker source — `formula_to_num` (Symbolics),
`formula_to_interval` (IntervalArithmetic), `formula_to_jump`
(JuMP-HiGHS), `formula_to_value` (DiffEq) — at
[`julia/institutions/<institution>/Eigenius<Institution>/src/`](../../../julia/institutions/).

## 7.4. Why these distinctions matter

The split between validator-level, parser-level, and handler-level
failures is intentional:

- **Validator failures** are fast (no runtime spawn), cheap (no
  buildah), and locality-of-blame. The failure points at the bad node
  in the bad resource. No partial chain state results.
- **Parser failures** are even faster (no chain commit, no kernel
  round-trip). The ESL compiler refuses the source.
- **Handler failures** are slowest because they require a worker
  spin-up, but they catch the things validation can't — institution-specific
  evaluator restrictions, unknown operator entries.

The platform tries to push as much rejection as possible to the
validator level by encoding institution-specific constraints into the
chain ontology where it can. The result is fewer surprises at dispatch
time and a more honest "what's possible to commit" surface.

---

Next: **[8. Appendix →](08-appendix.md)**
