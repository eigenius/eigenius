# 7. Type theory primer

This chapter is a brief, pragmatic reference to the type theory underlying ESL — enough to read kernel error messages, understand what the type-checker is doing, and follow the design rationale in [D19](../../design/d19-inductive-types.md) and [D11](../../design/d11-codata-streams.md). It is not a textbook. If you want depth, follow the references at the end.

The kernel descends from **Mini-TT** — Thierry Coquand, Yoshiki Kinoshita, Bengt Nordström, and Makoto Takeyama's compact normalisation-by-evaluation type-checker for a dependent type theory ([Cambridge chapter, 2009](https://www.cambridge.org/core/books/abs/from-semantics-to-computer-science/simple-typetheoretic-language-minitt/21451A12E2E24A1F51C82421B066824A)). The Mini-TT paper is the lineage we follow for the core architecture (term/value split, NbE conversion, bidirectional checking).

The Eigenius kernel **expands substantially beyond the teaching presentation**. Mini-TT as published is a small calculus designed to fit in one paper; EigenTT as implemented by the kernel adds first-class universes with cumulativity, inductive types with sized termination, coinductive types with productivity-by-typing, identity types with constraint discharge, an institution-dispatched constraint mechanism, and the ontology-as-types bridge that lets the type-checker resolve types lazily from the resource graph. Each of these is a substantial body of work in its own right and is documented in its own design note ([D9](../../design/d9-nbe-unification-and-type-extensions.md), [D11](../../design/d11-codata-streams.md), [D18](../../design/d18-ontology-as-types-resolution.md), [D19](../../design/d19-inductive-types.md)). What you'll recognise from Mini-TT in the implementation is the shape of the conversion engine and the bidirectional check rules; the rest is layered on top.

The implementation is in [`kernel/src/nbe/`](../../../kernel/src/nbe/). Two key files: [`term.rs`](../../../kernel/src/nbe/term.rs) defines `Exp` (syntactic terms), [`val.rs`](../../../kernel/src/nbe/val.rs) defines `Val` (semantic values).

## 7.1. Universes

`Set` is the universe of small types. `Type(n)` is the universe at level `n` — `Type(0)` contains `Set`, `Type(1)` contains `Type(0)`, and so on. The hierarchy is cumulative (Russell-style) — `Set : Type(0) : Type(1) : ...` — which lets you write `T : Set` without thinking about levels in most cases.

In ESL surface syntax, the universes appear as `core:Set` and (rarely) `core:Type`. You see them most often in `data` and `codata` parameter declarations:

```esl
data ex:List(A : core:Set) { ... }
```

Here `A : core:Set` says `A` is a small type. Cross-link to chapter 6: when the kernel resolves `core:Set`, it produces `Val::Set` directly.

## 7.2. Π-types — dependent functions

A Π-type `Π x : A. B(x)` is the type of functions whose return type may depend on the input. When `B(x)` doesn't actually mention `x`, you get the familiar non-dependent function type, which we usually write `A → B`.

In the kernel:

```rust
Exp::Pi(Patt, Box<Exp>, Box<Exp>)  // Pi(x, A, B)
Exp::Lam(Patt, Box<Exp>)           // λ x. body — inhabitant of Pi
```

ESL doesn't have a surface form for general Π-types — you don't write `Π x : T. U` directly. Π-types appear:

- As the type of `program` declarations: `program p : I -> O` produces `Pi(input : I, O)`.
- As function-typed `codata` observations: `tail : A -> Stream(A)` is `Pi(_ : A, Stream(A))`.
- As bounded binders' inner shape: `{j < i} -> body` desugars to `SizedPi { j, upper: i, body }`.

## 7.3. Σ-types — dependent pairs

A Σ-type `Σ x : A. B(x)` is the type of pairs whose second component's type may depend on the first. When `B(x)` doesn't mention `x`, you get a plain pair type `A × B`.

In the kernel:

```rust
Exp::Sig(Patt, Box<Exp>, Box<Exp>)  // Sig(x, A, B)
Exp::Pair(Box<Exp>, Box<Exp>)       // (a, b) — inhabitant of Sig
```

Σ-types are how `class` declarations are encoded ([chapter 6](06-resources-types-and-the-layer.md)). A class with required properties `p1 : T1, p2 : T2` becomes `Sig(p1 : T1, Sig(p2 : T2, One))` — a right-nested chain of Σ-types terminated by the unit type `One`. Each `Construct ex:C { ... }` builds the corresponding pair value.

The "field name" lives in the `Patt` (which is the property IRI for class-derived Σ-types). The kernel uses it for `find_sigma_field` lookups during projection type-checking.

## 7.4. Inductive types

Inductive types are introduced by `data` declarations ([§4.5](04-declarations.md)). Internally they're a kernel construct:

```rust
Exp::Inductive(InductiveDecl)             // the type-former declaration
Exp::InductiveType(Arc<InductiveDecl>, Vec<Exp>)  // applied to params
Exp::InductiveCtor(Arc<InductiveDecl>, String, Vec<Exp>)  // constructor application
```

An inductive type is defined by its constructors. `data Nat { zero, succ(Nat) }` introduces `Nat` along with two constructors (`zero` of type `Nat`, `succ` of type `Nat → Nat`). Recursive references are allowed — `succ` mentions `Nat` in its argument list.

**Pattern matching** consumes an inductive value:

```esl
match n returning ex:Nat {
    zero    -> n;
    succ(m) -> m;
}
```

The kernel form is `Exp::Match` (when the result type is synthesised from context) or `Exp::InductiveRec` (the elaborated recursor, with motive supplied). Iota reduction selects the arm whose constructor matches the scrutinee and substitutes the bindings.

**Positivity.** Recursive references must be in *strictly positive positions* — roughly, never on the left of an arrow. `data Bad { wrap(Bad -> A) }` would let the type be inhabited inconsistently and is rejected ([D19 §6](../../design/d19-inductive-types.md)).

## 7.5. Coinductive types

Coinductive types are introduced by `codata` declarations ([§4.6](04-declarations.md)). Where inductive types are *built up* from constructors, codata types are *consumed* through observations.

```rust
Exp::Codata(CodataDecl)
Exp::CodataType(Arc<CodataDecl>, Vec<Exp>)
Exp::CoRecord(Vec<CoField>)
Exp::Observe(Box<Exp>, String)
```

A `codata` type lists its observations and their result types. `codata Stream(A) { head : A; tail : Stream(A); }` means: a `Stream(A)` value can be asked for its `head` (yielding an `A`) or its `tail` (yielding another `Stream(A)`).

You build a `Stream(A)` value with `corecord { head = ...; tail = ... }`. You consume it by observing — which is a project-style operation in the surface syntax (`s.head`, though the projection is dispatched as an observation rather than a Σ-field lookup at the kernel level).

Codata is what makes infinite streams, transducers, and resumable processes representable. See [D11](../../design/d11-codata-streams.md).

## 7.6. Sized types — termination and productivity by typing

Sized types in Eigenius are inspired by the design Andreas Abel pioneered in [**Agda**](https://agda.readthedocs.io/en/latest/language/sized-types.html) and prototyped in [**MiniAgda**](https://hackage.haskell.org/package/MiniAgda). The original treatment appears in Abel's paper *MiniAgda: Integrating Sized and Dependent Types* (2010); the implementation is documented at [github.com/andreasabel/MiniAgda](https://github.com/andreasabel/MiniAgda). The kernel's [`sized_rigid.rs`](../../../kernel/src/nbe/sized_rigid.rs) is a direct port of MiniAgda's `TreeShapedOrder.hs`, and the dual-solver pattern (Warshall for meta-variables + TSO for rigid hypotheses) follows MiniAgda's `TCM.hs`.

The kernel has first-class support for **size variables** — a separate kind from `Set`/`Type(n)`:

```rust
Exp::SizeSort                  // the sort of sizes
Exp::SizeInf                   // ∞ — the largest size
Exp::SizedPi { patt, upper, body }  // bounded binder { j < upper } → body
```

Sizes form a partial order. `SizedPi { j < i }. body(j)` says "for any size `j` strictly less than `i`, the body has type `body(j)`". When you supply `j`, the kernel verifies `j < i` — a hypothesis tracked via the **TSO** (Tree-Shaped Order) rigid-hypothesis solver, the data structure ported from MiniAgda ([D19 §3](../../design/d19-inductive-types.md)).

This is the foundation of:

- **Termination of inductive recursion.** A recursive call must be at a strictly smaller size; the bounded binder `{j < i}` in a constructor's recursive argument tracks this. Without sizes, `data Nat { zero, succ(Nat) }` would type-check but the kernel couldn't prove that recursive functions over `Nat` terminate. With sizes, `data SizedNat(i : core:Size) { zero, succ({j < i}, SizedNat(j)) }` makes termination structural.
- **Productivity of corecursive definitions.** Each observation of a `codata` value must reduce to a constructor or another observation in finite time. For sized codata, the productivity check is that each observation of size `i` produces a result observable at size `j < i`.

ESL surface syntax for bounded binders ([§4.5](04-declarations.md)):

| Form | Compiles to |
|---|---|
| `{j < i}` | `SizedPi { j, upper: i, body }` (in constructor arg context) |
| `{j : core:Size < i}` | Same, with explicit kind |
| `{j : core:Size}` | Plain `Pi(j, SizeSort, body)` (unbounded) |
| `{j < i} -> body` | `SizedPi { j, upper: i, body }` (in observation type context) |

## 7.7. Identity types

The identity type `Id(A, x, y)` is the type of proofs that `x` and `y` are equal at type `A`. Its only inhabitant is `Refl x` (reflexivity — `x` is equal to itself).

The kernel exposes it for use by built-in `Constraint::Equality` predicates and for (rare) explicit equality reasoning in programs. Most ESL programs don't write `Id` types directly; they appear in error messages when constraint-firing fails.

The `J` eliminator is the standard way to use an identity proof — given `Id(A, x, y)` and a motive, prove things about both `x` and `y` simultaneously. In practice, ESL programs trigger `J` indirectly through constraint discharge.

## 7.8. Normalisation-by-evaluation (NbE)

The kernel's central conversion engine is normalisation-by-evaluation. The mechanics:

- **Evaluate** an `Exp` (term) to a `Val` (value). Reduction happens to the extent possible — substitutions are applied, β-redexes fired, constructors revealed.
- **Read back** a `Val` to an `Exp` in normal form. Applied to a closed value, this produces the term's normal form. Applied to a value containing free variables (called "neutrals"), it produces a normal form with those neutrals embedded.
- **Compare** two `Val`s for definitional equality by reading them back and comparing the normal forms syntactically.

Two key concepts you'll see in error messages:

**Neutral terms** are `Val`s that can't be reduced further because they're stuck on a free variable or an unresolved IRI. For example, `Var("x")` applied to anything is neutral until `x` gets bound. `EigonClass(iri)` is neutral until the kernel has layer access to resolve it. Neutrals show up in error messages as `Nt(...)` or printed in their term form.

**Closures** (`Val::Lam(closure)`, `Val::Sig(...)`) are `Val`s that capture an environment. They're how lambdas and Σ-types stay lazy — the body isn't evaluated until applied/projected.

The kernel's evaluator is in [`nbe/eval.rs`](../../../kernel/src/nbe/eval.rs); the type-checker in [`nbe/check.rs`](../../../kernel/src/nbe/check.rs); readback in [`nbe/readback.rs`](../../../kernel/src/nbe/readback.rs).

## 7.9. The `EigonClass` and `EigonPrimitive` bridges

Recapping from [chapter 6](06-resources-types-and-the-layer.md), with the type-theory framing:

`Val::EigonClass(iri)` is the kernel's "type whose definition lives in the layer". It behaves like a fully-reduced type for purposes of comparison (`EigonClass(iri1) == EigonClass(iri2)` iff the IRIs are equal), but to check whether a value inhabits it, the kernel calls `resolve_class_type` and checks against the resulting Σ-type.

`Val::EigonPrimitive(PrimitiveType)` is the type of one of `core:string`, `core:integer`, `core:float`, `core:boolean`, `core:json`. These are the leaf scalar types — they don't unfold further.

These two are the only kernel `Val` constructs that depend on layer state. Everything else is purely syntactic.

## 7.10. Cross-references to the rest of the guide

- **`Set` / `Type(n)`** — appear in `data`/`codata` parameter types ([§4.5](04-declarations.md), [§4.6](04-declarations.md)). The kernel treats them as universe levels for cumulativity checks.
- **Π-types** — surface as `program ... : I -> O` ([§4.7](04-declarations.md)) and as `codata` observation types ([§4.6](04-declarations.md)). Inhabited by lambdas ([§5.3](05-expressions.md)).
- **Σ-types** — surface as `class` declarations ([§4.2](04-declarations.md)). Inhabited by `Construct` and `Pair` ([§5.6](05-expressions.md), [§5.12](05-expressions.md)).
- **Inductive types** — surface as `data` declarations ([§4.5](04-declarations.md)). Inhabited by constructor application ([§5.2.1](05-expressions.md)). Consumed by `match` ([§5.5](05-expressions.md)).
- **Coinductive types** — surface as `codata` ([§4.6](04-declarations.md)). Inhabited by `corecord` ([§5.10](05-expressions.md)). Consumed by projection ([§5.7](05-expressions.md)).
- **Sized types** — bounded binder syntax ([§4.5](04-declarations.md), [§4.6](04-declarations.md)). The TSO solver verifies size relationships during type-check.

## 7.11. Further reading

- Coquand, T., Kinoshita, Y., Nordström, B., & Takeyama, M. (2009). [**A simple type-theoretic language: EigenTT**](https://www.cambridge.org/core/books/abs/from-semantics-to-computer-science/simple-typetheoretic-language-minitt/21451A12E2E24A1F51C82421B066824A). In Y. Bertot, G. Huet, J.-J. Lévy, & G. Plotkin (Eds.), *From Semantics to Computer Science: Essays in Honour of Gilles Kahn* (pp. 139–164). Cambridge University Press. <https://doi.org/10.1017/CBO9780511770524.007>. The EigenTT chapter — the lineage we follow for the term/value split, NbE conversion, and bidirectional checking. Eigenius extends EigenTT substantially with the additions documented in D9, D11, D18, D19; the small calculus in the chapter remains the cleanest entry point to the conversion engine's shape.
- [D9 — NbE unification and type extensions](../../design/d9-nbe-unification-and-type-extensions.md): capability modes, ground type resolution, and the EigenTT extensions the kernel ships.
- [D11 — Codata, streams, and resumable execution](../../design/d11-codata-streams.md): the coinductive-type design.
- [D18 — Ontology-as-types resolution](../../design/d18-ontology-as-types-resolution.md): the bridge mechanism described in chapter 6 with type-theoretic detail.
- [D19 — Inductive and sized types](../../design/d19-inductive-types.md): the formal positivity check, sized recursion rules, and recursor derivation.
- [`kernel/src/nbe/term.rs`](../../../kernel/src/nbe/term.rs) and [`kernel/src/nbe/val.rs`](../../../kernel/src/nbe/val.rs): the AST shapes for the kernel's terms and values.
- [nanoda_lib](https://github.com/ammkrn/nanoda_lib) — Chris Bailey's Lean 4 kernel implementation in Rust, which influenced the inductive-type design. The same library Eigenius integrates as the Lean institution checker (see [D28 §8.1](../../design/d28-lean-4-as-institution.md)).
- [*Type Checking in Lean 4*](https://ammkrn.github.io/type_checking_in_lean4/) — the design notes for Lean's kernel, accompanying nanoda_lib. Useful background on universe checking, positivity, and recursor derivation.
- Abel, A. (2010). [**MiniAgda: Integrating Sized and Dependent Types**](https://arxiv.org/abs/1012.4896). In *Workshop on Partiality and Recursion in Interactive Theorem Provers (PAR 2010)*, EPTCS 43, pp. 14–28. <https://doi.org/10.4204/EPTCS.43.2>. The foundational paper for the kernel's sized-types treatment — bounded size binders, strict-inequality hypotheses, sized inductives and codata for termination and productivity by typing.
- [**MiniAgda**](https://github.com/andreasabel/MiniAgda) (Andreas Abel) — the prototype implementation accompanying the paper. The kernel's [`sized_rigid.rs`](../../../kernel/src/nbe/sized_rigid.rs) is a direct port of MiniAgda's `TreeShapedOrder.hs`, and the dual-solver pattern (Warshall for meta-variables + TSO for rigid hypotheses) follows MiniAgda's `TCM.hs`. Available on [Hackage](https://hackage.haskell.org/package/MiniAgda).
- [**Agda — Sized Types**](https://agda.readthedocs.io/en/latest/language/sized-types.html) — the production-language documentation for the same machinery in Agda. Useful background for the *user-facing* shape of sized inductives and codata, with pedagogical examples.

---

Next: **[8. Capability modes →](08-capability-modes.md)**
