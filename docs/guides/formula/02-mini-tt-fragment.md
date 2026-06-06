# 2. The EigenTT fragment

`FormulaTerm` is a fragment of the kernel's small dependent-type theory
(EigenTT) lifted onto the chain. Its six constructors mirror EigenTT's
core `Exp` shapes one-for-one (with primitive leaves at `LitFloat` and
`OpRef`). This chapter introduces them and explains why the binders
(`Lam`, `Pi`) ship alongside the expression-shaped ones.

## 2.1. The constructor table

Declared at [`ontologies/formulas/formulas-ontology.json`](../../../ontologies/formulas/formulas-ontology.json)
as a `core:InductiveType` with six constructors:

| Constructor | Arg slots (typed) | EigenTT correspondence |
|---|---|---|
| `Var(name)` | `name: String` | `Exp::Var` — a free variable, looked up in an environment |
| `LitFloat(value)` | `value: Float` | numeric literal (a primitive in the formula fragment) |
| `OpRef(iri)` | `iri: String` (resolves to a chain `Operator` resource) | named operator reference (extension over plain EigenTT) |
| `App(head, arg)` | `head: FormulaTerm`, `arg: FormulaTerm` | `Exp::App` — function application |
| `Lam(name, ty, body)` | `name: String`, `ty: FormulaTerm`, `body: FormulaTerm` | `Exp::Lam` — lambda abstraction |
| `Pi(name, ty, body)` | `name: String`, `ty: FormulaTerm`, `body: FormulaTerm` | `Exp::Pi` — function-type former |

The first four are *expression-shaped* — what you'd write inside a
`formula(...)` block in ESL, or the body of a typical algebraic
expression. `LitFloat` and `OpRef` are leaves; `Var` references a binder
in scope; `App` composes other expressions.

The last two — `Lam` and `Pi` — are *binders*. Most concrete formulas
don't carry them inline (`x + 0` has none); they show up in two specific
places:

1. **Operator signatures** ([chapter 4](04-operator-catalog.md)). Every
   `formulas:Operator` declares its arity and argument types as a
   `Pi`-spine. `add: Real → Real → Real` is encoded as
   `Pi("_", Real, Pi("_", Real, Real))`.
2. **Formulas that quantify or abstract.** When you need to express
   "for all `x` of this type, `body` holds" or "the function that takes
   `x` and returns `body`", you write a `Pi` or a `Lam` inline.

## 2.2. Why `Pi` and `Lam` are chain-resident

A reasonable question — most numerical work doesn't quantify or
abstract; why expose binders on the chain at all?

The answer is the **Curry–Howard correspondence** (proofs are programs,
propositions are types). Under this lens:

- A proposition like *"for all `x: Real`, the formula evaluates to a
  positive number"* corresponds exactly to a function type `Pi(x: Real,
  is_positive(x))`. A *proof* of the proposition is a function — a `Lam`
  — of that type.
- An operator's signature, e.g. `add: Real → Real → Real`, is itself a
  `Pi`-spine — and the validator type-checks `App` invocations against
  it (chapter 4).
- A *constraint* attached to a property — "this value must satisfy
  predicate `P`" — is a typed value of an inductive type, and binders
  let it carry the structure of universal/existential quantification.

That second use is the load-bearing one for v1: the validator's
arity-check on `App` spines against operator signatures (D32 §5.4)
*requires* `Pi` to be on the chain because the signature itself is a
chain-resident `FormulaTerm`. Without `Pi`, the operator catalog
couldn't declare typed arities in a way the validator could read
directly. And once `Pi` is there, `Lam` comes along for free — they're
the dual binders for the same arrow.

The third use (constraints, propositions, predicates carried as typed
values) is enabled by the same machinery and is the medium-term path
described in the [SHACL-comparison note](../../notes/note-for-a-shacl-user.md)
for clauses that travel between institutions. v1's institutions don't yet
exploit this fully, but the chain shapes are in place.

You don't need to write `Pi` or `Lam` by hand to use the formula
language for everyday arithmetic. They're a structural piece of the
foundation, not a daily-driver part of the surface.

## 2.3. The kernel's connection to its own type theory

The kernel's full EigenTT lives in [`kernel/src/nbe/`](../../../kernel/src/nbe/);
it has more than the six chain-mirrored constructors (variants for
`Sigma`, `Inductive`, `MapReduce`, `NativeDecide`, etc.). The chain
fragment is a *subset* — the fragment that's most useful as a typed
expression payload for institutions, plus the binders needed to make
the typing rules work.

Practically, that means:

- The kernel's NbE evaluator can interpret a `FormulaTerm` value
  directly, without translation. When a comorphism's transformation is
  `Lam(t. Var(t))` (the identity), the evaluator runs it as a EigenTT
  reduction step.
- The validator's inductive-value type-checking rule (D32 §3.5) is the
  same rule it uses for any `core:InductiveType` — `ReductionStrategy`,
  `ConstraintRelation`, `Verdict`, etc. all type-check the same way.
  FormulaTerm is just the most heavily-used instance.
- A future evolution that adds another constructor (say, `BoundedQuant`
  for constrained quantification) is a chain ontology change — adding
  a ctor to `formulas:FormulaTerm` and corresponding mirror-side
  decoder — not a kernel-source change.

## 2.4. Reading a `FormulaTerm` value

Practical advice for reading a chain-committed FormulaTerm value:

- **Start at the outermost App.** Most formulas are spines of `App`s.
  The leftmost descent of an `App` chain ends at an `OpRef` — that's
  the operator. The rest of the spine is the operator's arguments,
  reading right-to-left up the spine.
- **`App(App(OpRef(f), a), b)` is `f(a, b)`.** Two-argument operators
  curry naturally. Three-argument operators are
  `App(App(App(OpRef(g), a), b), c)`.
- **`LitFloat` and `Var` are atoms.** No further descent; they're the
  leaves of the tree.
- **`Lam` and `Pi` are usually only at the root.** Inside a formula
  body they're rare; you'll see them mostly in operator signatures and
  in expressions that explicitly quantify.

The full encoding rules are in [chapter 3](03-eigon-json-embedding.md).

## 2.5. Comparison with related shapes

Quick reference for users coming from related places:

| If you're familiar with… | …the closest analogue is |
|---|---|
| Lisp s-expressions | `App` chains; `OpRef` plays the role of the operator at the head |
| Haskell algebraic data types | `FormulaTerm` is one such type; the six ctors are its constructors |
| ATP / TPTP / SMT-LIB terms | Similar function-symbol-plus-argument tree, but typed and chain-resident |
| Sympy / Symbolics expressions | One faithful representation of the same trees, with explicit binders |
| EigenTT / Calculus-of-Constructions terms | Direct subset; `Pi`, `Lam`, `App`, `Var` mean what they mean there |

If none of these are familiar, [chapter 3](03-eigon-json-embedding.md)
presents the encoding from scratch with no assumed background.

---

Next: **[3. Eigon-JSON embedding →](03-eigon-json-embedding.md)**
