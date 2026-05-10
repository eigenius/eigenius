# 5. The ESL `formula(...)` sublanguage

Inside a `formula(...)` block in ESL, the parser switches into a
Pratt-parsed math sublanguage with infix operators, function calls,
parens, and unary minus. The result is a `Value::CtorApp` literal
mirroring the FormulaTerm tree from [chapter 3](03-eigon-json-embedding.md).

This is the surface you use 99% of the time. Outside `formula(...)`, ESL
is still ESL; only inside the block does the math-style syntax apply.

## 5.1. The surface

```esl
resource ex:expr : symbolics:SymbolicExpression {
    symbolics:term = formula(x + 0);
}
```

Inside the parentheses you can write:

| Pattern | Lowers to |
|---|---|
| `42`, `3.14`, `-1.5` | `LitFloat` |
| `x`, `Ki`, `theta` | `Var` |
| `a + b` | `App(App(OpRef(add), a), b)` |
| `a - b` | `App(App(OpRef(sub), a), b)` |
| `a * b` | `App(App(OpRef(mul), a), b)` |
| `a / b` | `App(App(OpRef(div), a), b)` |
| `a ^ b` | `App(App(OpRef(pow), a), b)` |
| `-a` (unary) | `App(OpRef(neg), a)` |
| `(expr)` | the parenthesised expression |
| `f(a)` | `App(OpRef(formulas:ops:f), a)` |
| `f(a, b)` | `App(App(OpRef(formulas:ops:f), a), b)` |

Function-call syntax accepts any name; the parser resolves `f` to
`urn:eigenius:formulas:ops:f` (a chain-resident operator) at parse time.
Unknown operators are rejected at commit by the validator.

## 5.2. Operator precedence

Standard math precedence, with right-associative exponentiation:

| Precedence | Operators | Associativity |
|---|---|---|
| 1 (lowest) | `+`, `-` (binary) | left |
| 2 | `*`, `/` | left |
| 3 | `^` | **right** |
| 4 (highest) | unary `-`, function call, parens | — |

So `a + b * c` parses as `a + (b * c)`. `a ^ b ^ c` parses as
`a ^ (b ^ c)` (right-associative, like Haskell, math, and most calculator
conventions). `(a + b) * c` and `-a ^ 2` parse as you'd expect.

## 5.3. The unary-minus subtlety (lexer note)

A small detail with lexer-level consequences worth flagging: in v0,
`-1.5` was a single signed-literal token at the lexer level. That broke
`formula(x - 2)` because the lexer would read `-2` as a signed literal,
leaving `formula(x 2)` with no operator between them.

Phase 19f.3 fixed this by retiring lexer-level sign folding. The lexer
now always emits `Minus` as its own token; the parser decides whether
it's:

- Unary minus on a numeric literal (`ex:value = -1.5;` outside
  `formula(...)` — handled by `parse_value`)
- Unary minus on a `formula(...)` expression (handled by the Pratt
  parser's prefix-minus rule)
- Binary subtraction inside `formula(...)` (`formula(x - 2)`)
- An error in any other position

Practical effect: write `formula(x - 2)` and it parses as
`App(App(OpRef(sub), Var("x")), LitFloat(2.0))`. Write `formula(-x)` and
it parses as `App(OpRef(neg), Var("x"))`. Both work as expected.

The one quirk: a leading `-` followed immediately by a digit, *outside*
`formula(...)`, is still parsed as a signed literal (`-1.5` for the
backwards-compatible `ex:value = -1.5;` shape). That shape never
ambiguates against arithmetic, so it stays.

## 5.4. Worked examples

A few patterns you'll see in the kinase-institutions notebook and other
demos:

### A simple ODE right-hand side

```esl
resource nb:rhs_A : diffeq:RhsComponent {
    diffeq:term = formula(-1 * A * k);   // dA/dt = -k·A
}

resource nb:rhs_B : diffeq:RhsComponent {
    diffeq:term = formula(A * k);        // dB/dt = k·A
}
```

The first lowers to `App(App(OpRef(mul), App(App(OpRef(mul),
LitFloat(-1.0)), Var("A"))), Var("k"))` — left-spined, n-ary expressions
fold into nested binary apps.

### A polynomial objective

```esl
resource nb:sse_expr : symbolics:SymbolicExpression {
    symbolics:term = formula((4 - 2*Ki)^2 + (12 - 6*Ki)^2 + (22 - 11*Ki)^2);
}
```

Three squared residuals chained with `add`. Each residual is
`pow(sub(IC50_obs, mul(c, Ki)), LitFloat(2.0))`. The chain validates
the structure; the JuMP-HiGHS handler's "smart-pow" rule unrolls
`pow(expr, LitFloat(2.0))` to `expr * expr` to keep the result in
`QuadExpr` rather than `NonlinearExpr` territory (see the
[JuMP tutorial](../platform/julia-institutions/jump-highs-institution-tutorial.md)).

### A trigonometric expression

```esl
resource ex:expr : symbolics:SymbolicExpression {
    symbolics:term = formula(sin(x) + cos(x)^2);
}
```

`sin(x)` is `App(OpRef(sin), Var("x"))`. `cos(x)^2` is
`App(App(OpRef(pow), App(OpRef(cos), Var("x"))), LitFloat(2.0))`. The
top-level `+` glues them: `App(App(OpRef(add), <sin(x)>), <cos(x)^2>)`.

## 5.5. What's *not* in the sublanguage

A few absent-by-design features worth flagging:

- **No `let`-bindings inside `formula(...)`.** If the same
  sub-expression appears twice (the SSE example above duplicates the
  cycle of `(c - k*Ki)^2`), you write it twice. v1 doesn't have
  formula-level let-bindings; if you want sharing, factor at the ESL
  level by computing the sub-expression once outside the formula and
  referencing it as a `Var`.
- **No `Lam` / `Pi` surface inside arithmetic.** v1 doesn't have a
  way to write `formula(λx. x + 1)` inline. Operator signatures are
  authored as data; quantified clauses are an advanced shape that
  doesn't have ergonomic ESL syntax in v1.
- **No comparison operators in expression position.** v1's `formula(...)`
  parses *expressions*, not predicates. A comparison like `x < 2`
  inside `formula(...)` would parse as the `lt` operator returning a
  `Bool`-typed FormulaTerm — but the parser doesn't currently surface
  it as an infix operator. Use the function-call form: `formula(lt(x, 2))`.
- **No string literals or non-numeric constants.** `formula(...)` is for
  numerical expression trees. String values, IRIs, and other non-Float
  primitives don't belong in there.

## 5.6. Compiler output: `Value::CtorApp`

Internally, the parser emits a `Value::CtorApp` AST node — the kernel's
representation of "a structured constructor application" — that the ESL
compiler lowers into a chain `Value::Json` carrying the canonical
`{"ctor", "args"}` tagged-dict shape. From the validator's perspective
the result is identical to a hand-authored Eigon-JSON value.

This means the surface is "free" from a chain-correctness standpoint:
anything `formula(...)` can produce is a structurally well-formed
FormulaTerm value by construction. The only failure modes you can
reach via the surface are:

- Unknown operator names (parser succeeds, validator rejects at commit)
- Operator arity mismatches that the parser can't catch at parse time
  (rare — the parser knows the precedence-table operators are
  binary/unary; user-defined operators get arity-checked by the
  validator at commit)

For Eigon-JSON-authored values, the validator's full type-checking
applies — see [chapter 3 §3.4](03-eigon-json-embedding.md#34-the-validators-inductive-value-rule).

---

Next: **[6. Sharing across institutions →](06-sharing-across-institutions.md)**
