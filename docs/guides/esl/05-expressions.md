# 5. Expressions

The expression sublanguage appears inside `program` bodies. It's an ML-style typed expression language that compiles to kernel `Exp` terms via a two-step pipeline: ESL `Expr` → embedded resource (encoded program AST) → kernel `Exp`.

The two-step encoding exists because programs are first-class resources. The compiled body is a Resource subgraph that can be queried, traced, and persisted alongside everything else. The kernel reconstructs the `Exp` from this resource subgraph at type-check or eval time via [`parse_program`](../../../kernel/src/program/expr.rs).

This chapter walks every expression form from [`ast::Expr`](../../../kernel/src/esl/ast.rs). For each: syntax, the resource shape it compiles to, the kernel `Exp` it parses to, the type-check rule, the evaluation rule, and any capability-mode notes.

## 5.1. `let`

```esl
let name : Type = value_expr;
body_expr
```

Compiles to an embedded resource of class `program:Let` carrying `name`, `type`, `value`, `body`. The kernel form is `Exp::Let(name, type, value, body)`.

**Type-check.** `value_expr` is checked against `Type`; `body_expr` is checked in an environment extended with `name : Type`. The whole expression has the type of `body_expr`.

**Evaluation.** Evaluate `value_expr`, bind it to `name`, evaluate `body_expr` in the extended environment.

The semicolon between the let and body is mandatory — it's how the parser knows where the value expression ends and the body begins.

## 5.2. Function application — `f(args)`

Function application is the most overloaded form in ESL. Depending on what `function` resolves to, it dispatches to one of four paths:

1. **Inductive constructor** — `function` is the bare name of a constructor declared by `data`.
2. **Component** — `function` is a component name resolved against the registered component set.
3. **Decidable QueryClass** — `function` is a qualified name `cap:predicate` that classifies through the [`InstitutionIndex`](../../../kernel/src/institution/registry.rs) as a `Decidable` `QueryClass` (D14).
4. **Direct lambda application** — `function` is a lambda or other higher-order expression.

The compiler picks the path. For the first three, it emits a specialised resource shape; for the fourth, it falls through to the generic `program:Apply`. Comorphisms are *not* expression-position calls under D14 — they are declarations consumed by EigenQL FIBER param coercion (see §5.2.4).

### 5.2.1. Constructor application

```esl
succ(zero)
cons(1, cons(2, nil))
```

Bare-name function references that match a declared constructor compile to `program:CtorApply` with the constructor IRI in `function` and a positional `arguments` array. The kernel form is `Exp::InductiveCtor(decl, ctor_name, args)`.

**Type-check.** The constructor's declared signature gives the expected types for each arg. Sized constructors (Phase 11g) verify that any size argument supplied is strictly below the declared upper bound — otherwise: `"InductiveCtor 'X.succ': size argument ... is not strictly below upper bound ..."`.

**Evaluation.** Constructs `Val::InductiveVal { decl, ctor_name, args }`.

A constructor cannot take a trailing config block — `succ(zero) { ... }` is a compile error.

### 5.2.2. Component application

```esl
CompleteText(input)                    // single positional arg
CompleteText(input) { model = "x" }    // single positional + config block
CompleteText(input, config_resource)   // legacy: second positional → config
```

Bare-name function references that don't match a constructor are looked up in the registered component set. The compiler resolves `CompleteText` to `urn:eigenius:program:components:CompleteText` (or another registered IRI) and emits a `program:Apply` with the resolved IRI as the function and the input expression as the argument.

The optional trailing `{ ... }` config block becomes a `component_argument` resource — a structured config resource passed alongside the runtime input.

**Type-check.** The component's declared input/output type informs the check.

**Evaluation.** In `IO` capability mode, the component dispatcher invokes the registered handler. In `Pure`/`Read` mode, the application stays neutral — the program type-checks but doesn't run.

See [chapter 8](08-capability-modes.md) for the per-mode behaviour.

### 5.2.3. Decidable QueryClass application

```esl
cap:within_tolerance(input, 0.1)
```

When the qualified name `cap:within_tolerance` classifies through the [`InstitutionIndex`](../../../kernel/src/institution/registry.rs) as a `Decidable` `QueryClass`, the compiler emits `program:DecideApply` carrying the IRI and an `arguments` array of any positional args. The kernel form is `Exp::NativeDecide(Constraint::Institution { iri, args }, Unit)`.

**Type-check.** Result type is `Verdict` (the inductive type with constructors `Holds | Fails | Undecidable`, D14 §6.1). Args are not statically checked — the institution validates them at call time.

**Evaluation.** In `Check` or `IO` mode (any mode with an `InstitutionIndex` + `InstitutionRuntime`), invokes `Institution::query(query_handler, synthetic_input, ctx)` and reduces the surrounding `NativeDecide` based on the returned Verdict's `ctor_name`: `Holds` → `Refl(v)`, `Fails` → failing neutral, `Undecidable` → passthrough neutral.

A trailing config block on a decide call is a compile error.

### 5.2.4. Comorphisms — declaration-only in ESL

Under D14 a `Comorphism` is a typed ontology resource (not a callable function). ESL has no expression-position syntax for comorphism dispatch — `cap:dock_to_assay(input)` is *not* a valid call form. Three ways to use a comorphism from ESL:

1. **Apply the transformation Component directly** — the comorphism's `transformation` is an ordinary EigenTT Component. If your program already has the typed payload, call it: `let ic50 : core:float = cm_arrhenius(delta_g);`.
2. **Translate inside an EigenQL `FIBER`** — comorphism coercion in FIBER param values runs the four-step extract → transform → reify pipeline (see [EigenQL §7.5](../eigenql/07-fiber-clauses.md) and [§8.6](../eigenql/08-institutions.md)).
3. **Wrap as a Component-implemented OnDemand QueryClass and dispatch via FIBER.** A QueryClass whose `implementation` is a Component IRI runs `extract → component → reify` automatically; EigenQL FIBER is the surface that reaches it.

See [chapter 9](09-institutions.md) for the rationale.

### 5.2.5. Generic apply

```esl
(\x -> x)(input)
my_lambda(value)
```

Anything else compiles to `program:Apply` with the function expression and the argument expression as embedded sub-resources. The kernel form is `Exp::App(f, arg)`.

**Type-check.** Standard Π-application: `f : A → B`, `arg : A`, result is `B`.

**Evaluation.** Standard β-reduction.

## 5.3. Lambda — `\x -> e` or `λx -> e`

```esl
\x -> body
λx -> body
```

Compiles to `program:Lambda` with `param` and `body`. The kernel form is `Exp::Lam(x, body)`.

**Type-check.** Lambdas are checked against an expected `Pi(x : A, B)` — `body` is checked against `B` in an environment extended with `x : A`. Lambdas are not type-inferable in isolation; they need a checking-mode context.

**Evaluation.** Yields a closure `Val::Lam(closure)`.

The body extends as far right as possible — `\x -> let y = ... ; ...` puts the entire let-expression in the lambda body. Use parentheses if you need to limit scope.

## 5.4. Variable — `x`

```esl
input         // the implicit program parameter
my_value      // any locally-bound name
```

Bare identifiers compile to `program:Var` with the name. The kernel form is `Exp::Var(name)`.

**Type-check.** Looks up `name` in the local environment. If not found, the type-checker may attempt to resolve the name as a class IRI through `EigonClass` — see [chapter 6](06-resources-types-and-the-layer.md).

**Evaluation.** Looks up the name in the runtime environment.

## 5.5. Pattern match — `match e returning T { arm => body; ... }`

```esl
match my_nat returning ex:Nat {
    zero -> zero;
    succ(n) -> n;
}
```

Pattern-matches a value of an inductive type. Each arm names a constructor by its bare name and (optionally) binds variables for its arguments positionally. Use `_` for arguments not referenced in the body.

Compiles to either:

- `program:InductiveRec` (when `returning T` is present) — a fully-elaborated recursor with motive `λ_. T`, ready for the kernel.
- `program:Match` (when `returning T` is omitted) — a checking-mode match where the kernel synthesises the motive from the surrounding context.

The kernel form is correspondingly `Exp::InductiveRec` or `Exp::Match`.

**`returning` clause.** Always include it when the match's result type isn't obvious from the surrounding context. Without it, the kernel synthesises the motive from the checking-mode expectation; in inference mode (e.g. as the right-hand side of a `let` without an annotation), an unannotated match fails with `"cannot infer type of match — add a returning clause or annotate context"`.

The clause accepts two shapes (eigenius#72 Layer 3):

- **Bare type reference** — `returning ex:Nat`. The kernel wraps it as the constant motive `λ_. ex:Nat`. Use this whenever the scrutinee is non-indexed and the result type doesn't depend on the scrutinee.
- **Lambda motive** — `returning fun (i : core:Nat) => Vec(A, i)`. The motive abstracts over the scrutinee's indices. **Required** when the scrutinee is an indexed inductive whose result type depends on those indices — the kernel uses the motive to refine each arm's expected type per the constructor's conclusion indices.

```esl
match v returning fun (i : core:Nat) => Vec(A, i) {
    nil -> nil;
    cons(k, x, xs) -> cons(k, x, xs);
}
```

Compiles to `program:result_motive` carrying a D47-encoded `λ`-chain rather than the flat `program:result_type` IRI. The kernel decoder type-checks the motive's body at `Sort(n)` against the scrutinee inductive's `Π indices. Sort` signature, then specialises each arm's expected type per the constructor's conclusion indices.

**Sized recursion.** When the scrutinee has a sized inductive type, each arm body sees a hypothesis recording that any recursive call within the arm must be on a strictly smaller size. This is what powers sized termination ([D19 §4](../../design/d19-inductive-types.md)).

**Evaluation.** Iota-reduces — selects the arm matching the constructor and substitutes the bindings.

## 5.6. `Construct` — record/resource construction

```esl
Construct ex:Document {
    ex:text = my_summary,
    ex:author = input.ex:user
}
```

Constructs a fresh resource of the given class with the listed field assignments. Compiles to `program:Construct` carrying the class IRI and a `fields` array of `(property_iri, expr)` pairs. The kernel form depends on what the class resolves to:

- If `ex:Document` is a `Class` resource (Σ-type), constructs a Σ-tuple.
- If `ex:Document` is something else (an inductive type, etc.), the form may differ — `Construct` is the user-facing front for "build a value of this declared type".

**Type-check.** Each field expression is checked against the property's declared type. Required (`requires`) properties of the class must be supplied; `recommends` properties are optional. Extra properties not declared by the class are an error.

**Evaluation.** Builds a value of the declared type with the given field values.

## 5.7. Projection — `e.prop`

```esl
input.ex:name
input.ex:user.ex:email
```

Reads a property from a resource. Compiles to `program:Project` carrying the underlying expression and the property IRI.

**Type-check.** The expression's type is unfolded as a Σ-type or `EigonClass`-resolved class; the property must appear in its field list. Result type is the property's declared `data_type`.

**Evaluation.** Looks up the property on the value (a resource ref or embedded resource).

Chained projection (`a.b.c`) is left-associative — equivalent to `(a.b).c`.

## 5.8. `map` — apply a function over a collection

```esl
map(\x -> process(x), my_items)
```

Compiles to `program:MapExpr` with `function` and `collection`. The kernel form is `Exp::Map(f, coll)`.

**Type-check.** Function must be `A → B`; collection must be `List A` (or array-shaped); result is `List B`.

**Evaluation.** Apply `f` to each element. Termination is structural over the finite list.

## 5.9. `reduce` — fold over a collection

```esl
reduce(\acc x -> combine(acc, x), initial, my_items)
```

Compiles to `program:ReduceExpr` with `function`, `initial`, `collection`. The kernel form is `Exp::Reduce(f, init, coll)`.

**Type-check.** Function must be `B → A → B`; `init : B`; collection is `List A`; result is `B`.

**Evaluation.** Left fold over the list with `init` as starting accumulator.

## 5.10. `corecord` — coinductive value construction

```esl
corecord {
    head = first_value;
    tail = make_next_stream();
}
```

Builds a value of a coinductive type. Each cofield names an observation and supplies its value (or a function for size-bounded observations).

Compiles to `program:CoRecord` carrying a `cofields` array of `(observation_name, body)` pairs.

**Field order matters.** Cofields must be listed in the same order as the codata's declared observations. The compiler does not reorder; the kernel validates against the declared sequence.

**Sized observations.** For an observation typed `{j < i} -> Body`, the body must be a lambda taking the size argument: `tail = λj -> ...`. The kernel verifies productivity — the body's recursive references must use strictly smaller sizes ([D19 §8](../../design/d19-inductive-types.md), Phase 11f productivity-by-typing).

**Type-check.** The expected codata type is required (checking mode); each cofield body is checked against its observation's declared type.

**Evaluation.** Yields `Val::CoRecord(closures)`. Observations are evaluated lazily — the body for each observation runs only when that observation is consumed.

## 5.11. `case` — closed-tag matching

```esl
case my_result {
    Ok -> success_value;
    Err -> fallback;
}
```

A simpler match form for closed-tag types (variants without binding). Compiles to `program:Case` with `scrutinee` and a `branches` array of `(tag, body)` pairs.

`case` is an older form; for new code, prefer `match` with explicit constructors.

## 5.12. `Pair` — `(a, b)`

```esl
(name, age)
```

Compiles to `program:Pair` with `first` and `second`. The kernel form is `Exp::Pair(a, b)`.

**Type-check.** Components are checked individually; result type is `Sigma(_ : A, B)`.

**Evaluation.** Yields `Val::Pair(a, b)`.

## 5.13. Literals

```esl
"hello"   // string literal
42        // integer literal
3.14      // float literal
true      // boolean literal
```

Compile to `program:Literal` carrying the value with its data type. Kernel form is `Exp::Literal(...)` for the appropriate kind.

**Type-check.** Inferred type is the matching `core:` primitive.

**Evaluation.** Identity — the literal is its own value.

## 5.14. `formula(...)` — typed expression-tree literals

A `formula(...)` block opens a Pratt-parsed math sublanguage *inside* an ESL expression. It compiles to a chain-resident `formulas:FormulaTerm` value — the typed expression-tree language every numerical institution speaks (Symbolics, IntervalArithmetic, Catalyst, DiffEq, JuMP-HiGHS).

```esl
namespace symbolics = "urn:eigenius:symbolics";
namespace nb        = "urn:eigenius:notebook:kinase_demo";

resource nb:rhs_A : diffeq:RhsComponent {
    diffeq:term = formula(-1 * A * k);  // dA/dt = -k·A
}

resource nb:sse_expr : symbolics:SymbolicExpression {
    symbolics:term = formula((34 - 2*Ki)^2 + (68 - 4*Ki)^2 + (170 - 10*Ki)^2);
}
```

Inside the parens you can write `+ - * / ^`, function calls (`sin(x)`, `pow(a, b)`), parens, unary minus, `Var`-shaped identifiers, and float literals. Standard math precedence; `^` is right-associative. Function names lower to the chain operator catalog at `urn:eigenius:formulas:ops:<name>`; unknown operators are accepted by the parser but rejected by the chain validator at commit time.

**Compile target.** A `Value::CtorApp` literal mirroring the FormulaTerm tree (`App`, `OpRef`, `Var`, `LitFloat`, …). The kernel emits the canonical Eigon-JSON tagged-dict shape `{"ctor": "App", "args": [head, arg]}` recursively, which the validator type-checks against `formulas:FormulaTerm`'s ctor schema and against each operator's declared `Pi`-spine arity (`OperatorArityMismatch` if the spine is wider than the signature).

**Lexer note (Phase 19f.3).** A bare `-` no longer folds into a numeric literal at lex time, so `formula(x - 2)` lexes correctly as `Minus IntLit(2)` (binary subtraction) rather than `Minus IntLit(-2)`. Outside `formula(...)`, the legacy `ex:value = -1.5;` shape (unary minus on a numeric literal) is handled by `parse_value` and still works.

**Where this is allowed.** Any expression position. The most common shapes are property values inside `resource` declarations (as above) and `let` bindings.

This is the entry point for the platform's typed numerical surface. The full reference — six constructors, operator catalog, validator rule, Eigon-JSON encoding, identity-comorphism collapse across institutions — lives in the [formula language guide](../formula/README.md), specifically [§5 ESL `formula(...)` sublanguage](../formula/05-esl-sublanguage.md).

<a id="5-14a-type_expr-eigentt-type-expressions"></a>
## 5.14a. `type_expr(...)` — EigenTT type expressions as chain values (D47)

The surface counterpart of `formula(...)` for the [D47 chain-mirrored EigenTT type fragment](../../design/d47-chain-mirrored-eigentt-type-fragment.md). A `type_expr(...)` block lets you embed an EigenTT type expression (a proposition in `Prop`, a function type, an inductive-ctor application, etc.) directly in any value position; the compiler parses it with the same `parse_type_expr` grammar `axiom` and `data` ctor types use, lowers it to a kernel `Exp`, and runs the D47 encoder to produce a chain-resident `core:EigenTTType` value:

```esl
namespace screen     = "urn:eigenius:demo:screen";
namespace reflection = "urn:eigenius:reflection";
namespace stats      = "urn:eigenius:measurements";

resource screen:claim_eig0291_lowic50 : stats:StatisticalAnalysisPlan {
    stats:sample_set = screen:m_eig0291_sampleset;

    stats:null_hypothesis = type_expr(
        screen:HasLowIC50("urn:eigenius:demo:screen:EIG_0291")
    );
    reflection:canonical_proposition = type_expr(
        screen:HasLowIC50("urn:eigenius:demo:screen:EIG_0291")
    );

    stats:alpha = 0.05;
    stats:effect_size = Absolute(100.0, "nM");
    stats:directionality = TwoSided();
    stats:variance_assumption = WelchUnequal();
    stats:outlier_exclusion = Identity();
}
```

The `type_expr(HasLowIC50(...))` expression compiles to a `Value::Json` carrying the tagged-dict tree:

```json
{
  "ctor": "App",
  "args": [
    {"ctor": "ConstRef", "args": ["urn:eigenius:demo:screen:HasLowIC50"]},
    {"ctor": "LitString", "args": ["urn:eigenius:demo:screen:EIG_0291"]}
  ]
}
```

…which the chain validator type-checks against `core:EigenTTType`'s ctor schema. Downstream consumers — the [D49 witness index](../../design/d49-chainwitness-machinery.md) computing the witness key for `IsDerivedAs`, the [D39 reasoning institution](../../design/d39-justification-logic.md) reading the predicate to decide the certificate's grounding shape, the [D52 statistics institution](../../design/d52-measurement-statistics-institution.md) checking the predicate's `is_a` scope marker against the SampleSet's replication kind — all decode this same value with the [D47 decoder](../../../kernel/src/program/eigentt_type_mirror.rs) and read out the same kernel `Exp`.

### What grammar the inner expression accepts

The same `parse_type_expr` grammar that powers `axiom` ([§4.4a](04-declarations.md#4-4a-axiom-postulated-propositions-d46-10)) and `data` ctor types ([§4.5](04-declarations.md#4-5-data-inductive-types)):

| Form | Lowers to |
|---|---|
| `forall (x : T, y : U) => body` | `Exp::Pi(x, T, Exp::Pi(y, U, body))` |
| `exists (x : T, y : U) => body` | `Exp::Sig(x, T, Exp::Sig(y, U, body))` |
| `A -> B` | Non-dependent `Exp::Pi(_, A, B)` |
| `Prop` / `Set` / `Type N` | `Exp::Sort(...)` |
| `Id(A, x, y)` | `Exp::EigonClass(core:Id)` applied to args (D46 §7) |
| `ex:Pred(arg1, arg2, ...)` | `Exp::App(Exp::ConstRef(IRI), arg1, arg2, ...)` |
| `ex:zero` (nullary ctor) | `Exp::ConstRef(IRI)` |
| `eigentt:fst(p)` / `eigentt:snd(p)` | `Exp::Fst(p)` / `Exp::Snd(p)` |
| `()` | `Exp::Unit` |
| `"literal-iri"` | `Exp::LitString("literal-iri")` |

The `LitString` form is what lets you embed concrete subject IRIs — `screen:HasLowIC50("urn:eigenius:demo:screen:EIG_0291")` — directly in a proposition without authoring a separate `core:axiom_statement` resource per compound. The kernel treats the literal as an opaque `string` value; downstream institutions consume it by string-equality matching.

### Σ-binders and their projections

`exists` is the dependent-pair binder, the dual of `forall`: N binders lower to N nested `Exp::Sig`, rightmost innermost, exactly as `forall` lowers to nested `Exp::Pi`. The binder list takes the same optionally-parenthesised form (`exists x : T => B` and `exists (x : T, y : U) => B` both parse), and `=>` closes it — not `.`, which stays free for a future postfix projection form.

A Σ in `Prop` position *is* the existential, and `eigentt:fst` recovers the witness:

```esl
axiom demo:t : eigentt:fst(exists x : core:string => core:string);
```

`eigentt:fst(p)` / `eigentt:snd(p)` are written as one-argument applications because `TypeExpr` has no postfix form. They are **not axioms**: the compiler intercepts those two IRIs and emits `Exp::Fst` / `Exp::Snd` term nodes, so `fst(pair)` reduces — an axiom would be opaque and never compute. A one-argument call to anything else stays an ordinary `App`. On the wire they are the `Fst` / `Snd` ctors of [`eigentt:TypeExpr`](../../../ontologies/eigentt/eigentt-type-fragment.json), added alongside the `Pair` ctor D48 shipped.

`()` is the unit *value* (`Exp::Unit`), the sole inhabitant of `One`. Hand-written certificates normally omit it — the kernel synthesises the witness slot in e.g. `declared(bridge, P)` — so it exists mainly so that [`eigenius decompile`](../platform/04-cli-reference.md#decompile-file---verify---pretty) can print back what the kernel encoded and have it reparse.

`exists` and `()` are meaningful only where `type_expr(...)` lowering applies (including `axiom` and `data` ctor types, which take the same path). In the resource-shaped type language the compiler rejects both:

```
`exists` (Sigma) is only available inside `type_expr(...)`, which lowers to the
D47 ctor encoding; the resource-shaped type language has no binder for it

the unit value `()` is a TERM, not a type — it is only meaningful inside `type_expr(...)`
```

What forces the pair: the DCG parser renders every definite description as `the(Σx : C. P(x)).1`, so a proposition parsed from prose is unwritable in ESL — and, before `Fst`/`Snd` were fragment ctors, uncommittable at all — without the binder and its projections.

### Why this surface vs. the JSON form

Authors *could* hand-write the tagged-dict JSON tree directly as a `Value::Json` literal, but in practice three-nested `App(ConstRef, LitString, …)` shapes get verbose quickly and the syntactic noise drowns out the proposition. `type_expr(...)` is to the D47 type fragment what `formula(...)` is to the D32 formula language: a Pratt-parsed inline sublanguage that compiles to the same wire shape the verifier consumes, with the syntactic shape an author actually reads.

The two sublanguages target different chain types — `formula(...)` produces `formulas:FormulaTerm`, `type_expr(...)` produces `core:EigenTTType` — and they don't overlap: numerical institutions speak FormulaTerm, propositional / reasoning institutions speak EigenTTType. An ESL expression position can host either, depending on what the property's `class_types` constraint demands.

### Compile target

`type_expr(...)` lowers in two steps:

1. The parser produces a `Value::TypeExpr { typ, pos }` AST node carrying the inner `ast::TypeExpr` tree;
2. The compiler's `encode_type_expr_to_json` walks that tree, calls `lower_type_expr_to_exp` to produce a kernel `Exp`, then `eigentt_type_mirror::encode_type` to produce the tagged-dict JSON shape, and rewraps as `Value::Json`.

The resulting value lands in the resource's property slot just like any other property literal — there's no special chain-storage treatment.

### Where this appears in practice

- `reflection:canonical_proposition` on every `DerivedResource` subclass (StatisticalAnalysisPlan, ReasoningSentence, custom institution-emitted derived resources) — the proposition the resource asserts.
- `eigentt:axiom_statement` on every `axiom` declaration ([§4.4a](04-declarations.md#4-4a-axiom-postulated-propositions-d46-10)) — surface-compiled via the same lowering path.
- `stats:null_hypothesis` / `stats:alternative_hypothesis` on `StatisticalAnalysisPlan` — the null and alternative the verifier reports in the verdict's audit trail.
- `reasoning:proposition` on `ReasoningSentence` — the proposition the certificate type-checks against.
- `core:ctor_type` on the typed-ctor form of indexed inductives — emitted by the compiler from the `data` declaration, not authored as a literal.

Source: [`parse_type_expr`](../../../kernel/src/esl/parser.rs), [`lower_type_expr_to_exp`](../../../kernel/src/esl/compile.rs), [`encode_type_expr_to_json`](../../../kernel/src/esl/compile.rs), [`eigentt_type_mirror::encode_type`](../../../kernel/src/program/eigentt_type_mirror.rs).

## 5.15. Capability modes — quick reference

[Chapter 8](08-capability-modes.md) covers this in detail. The short version:

- **Pure**: only normalisation. Lambdas, lets, pairs, projections of static records, etc., reduce. Component apps and decide calls stay neutral.
- **Read**: Pure + layer access. Adds the ability to resolve `EigonClass` references and read property values from layer resources.
- **Check**: Read + institution registry. Used by the type-checker when constraints with institution-decide procedures need to fire.
- **IO**: Read + institution registry + component dispatch + trace store. The runtime mode.

Most expressions reduce in Pure. Component application, decide predicates, and comorphism invocation only do real work in Check (decide) or IO (all three).

---

Next: **[6. Resources, types, and the layer →](06-resources-types-and-the-layer.md)**
