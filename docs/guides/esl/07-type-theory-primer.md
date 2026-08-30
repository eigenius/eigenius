# 7. Type theory primer

This chapter is a brief, pragmatic reference to the type theory underlying ESL — enough to read kernel error messages, understand what the type-checker is doing, and follow the design rationale in [D19](../../design/d19-inductive-types.md) . It is not a textbook. If you want depth, follow the references at the end.

The kernel descends from **Mini-TT** — Thierry Coquand, Yoshiki Kinoshita, Bengt Nordström, and Makoto Takeyama's compact normalisation-by-evaluation type-checker for a dependent type theory ([Cambridge chapter, 2009](https://www.cambridge.org/core/books/abs/from-semantics-to-computer-science/simple-typetheoretic-language-minitt/21451A12E2E24A1F51C82421B066824A)). The Mini-TT paper is the lineage we follow for the core architecture (term/value split, NbE conversion, bidirectional checking).

The Eigenius kernel **expands substantially beyond the teaching presentation**. Mini-TT as published is a small calculus designed to fit in one paper; EigenTT as implemented by the kernel adds first-class universes with cumulativity, inductive types, identity types with constraint discharge, an institution-dispatched constraint mechanism, and the ontology-as-types bridge that lets the type-checker resolve types lazily from the resource graph. Each of these is a substantial body of work in its own right and is documented in its own design note ([D9](../../design/d9-nbe-unification-and-type-extensions.md), [D18](../../design/d18-ontology-as-types-resolution.md), [D19](../../design/d19-inductive-types.md)). What you'll recognise from Mini-TT in the implementation is the shape of the conversion engine and the bidirectional check rules; the rest is layered on top.

The implementation is in [`kernel/src/nbe/`](../../../kernel/src/nbe/). Two key files: [`term.rs`](../../../kernel/src/nbe/term.rs) defines `Exp` (syntactic terms), [`val.rs`](../../../kernel/src/nbe/val.rs) defines `Val` (semantic values).

## 7.1. Universes — the unified `Sort(n)` ladder with `Prop` at the bottom

The kernel encodes every universe as a single `Sort(n)` constructor with a `usize` level. The level semantics:

```
Sort(0)   = Prop
Sort(1)   = Set         (= Type(0) under the surface naming)
Sort(n+1) = Type(n)     for n >= 1
```

Typing rule: `Sort(n) : Sort(n+1)`. Cumulativity is positional: `Sort(m) ⊆ Sort(n)` iff `m ≤ n`. The full cumulative chain is

```
Prop  ⊆  Set  ⊆  Type(1)  ⊆  Type(2)  ⊆  ...
```

— **`Prop` cumulates into `Set`**, matching Lean 4's variant. (Coq's predicative-Set tradition keeps Prop as a non-cumulative side branch; the kernel takes the simpler unified view.) A proposition can be coerced into a type when its propositional content is forgotten; the reverse is not admissible.

ESL surface syntax keeps `Set` and `Type(n)` as sugar for `Sort(1)` and `Sort(n+1)` respectively, and adds `Prop` as sugar for `Sort(0)`. You see all three in `data` parameter declarations and result-sort clauses:

```esl
data ex:List(A : core:Set) { ... }

data screen:HasLowIC50 : core:string -> Prop, stats:PopulationLevel {
}

data screen:Eq(A : core:Set) : A -> A -> Prop {
    refl : forall (a : A) => screen:Eq(A, a, a),
}
```

`HasLowIC50` is a **zero-constructor opaque predicate** in `Prop` — its inhabitants are admitted by chain mechanism (a [D49 chain witness](../../design/d49-chainwitness-machinery.md) for an `IsObservedAs` / `IsDeclaredAs` / `IsVerifiedAs` triple, or a D39 reasoning sentence's grounding constructor) rather than constructed by users. `Eq` is propositional equality with its single `refl` constructor (the indexing forces both indices to match — see [§7.3a](#7-3a-indexed-inductive-families) for the type-theory of indices).

### Three kernel-level disciplines specific to `Prop`

D46 introduces three rules that distinguish `Prop` from `Set` / `Type(n)` operationally:

1. **Proof irrelevance.** Two inhabitants `p, q : P` of the same `Prop` are judgmentally equal — the kernel's `def_eq` returns true for `(p, q)` whenever it can decide `P : Prop`. Computationally: the kernel never needs to compare the *structure* of two proofs; it just checks that the type they inhabit is in `Prop`. This is what makes propositions about chain artifacts — "this measurement Holds at α = 0.05", "this rule grounds this assertion" — equal regardless of how they were derived. The audit invariant becomes: the chain attests *that* a proposition holds, not *which* proof was used.
2. **Singleton elimination.** Pattern-matching a `Prop`-typed scrutinee into a `Set` / `Type(n)` motive (so-called *large elimination*) is restricted to two shapes: (a) **zero-constructor** inductives (vacuous match); (b) **single-constructor** inductives where each constructor argument either lives in `Prop` itself or appears in the conclusion's indices (so the eliminator can reconstruct it from the type alone). `Eq(A, x, y)` fits Case (b) via the indices clause — `refl`'s argument `a` is bound to both indices, so any motive can recover `a` from the scrutinee's type. `True` fits Case (b) trivially (no arguments). Rejections surface as `cannot eliminate Prop-typed scrutinee into Set` errors. See [§7.3a](#7-3a-indexed-inductive-families) for the full Case A / Case B rules and [D46 §7](../../design/d46-prop-universe-and-proof-irrelevance.md) for the soundness story (the rule blocks the Hurkens construction without losing the useful singletons).
3. **Impredicative Π formation.** `Π (x : A), B` lives in `Prop` whenever the codomain `B` lives in `Prop`, regardless of where the domain `A` lives. This is what lets you write `forall (P Q : Prop), (P <-> Q) -> Id(Prop, P, Q)` (propext) as a member of `Prop` even though it quantifies over the entire propositional universe — the codomain is propositional, so the whole Π is. The `Type(n)` ladder is *predicative*: `Π (x : Sort(m)), B : Sort(n)` lives in `Sort(max(m, n))` when `n ≥ 1`, so non-Prop universe levels accumulate. Only `Prop` formation collapses them. Sigma, sum, and the other type formers follow the standard `max`-rule symmetrically — a Σ of two Prop fields is in Prop, a Σ mixing a Prop and a Set field is in Set.

### Why this matters

Without `Prop`, every proposition would be a `Set`-typed inductive, distinct proofs of the same proposition would be distinct values, and the audit invariant would have to be "this chain artifact is the canonical proof of *P*" — which is not a thing the chain can adjudicate. With `Prop` + proof irrelevance, the invariant is "this chain artifact has type *P*", and it doesn't matter how the inhabitant was produced. Two reasoning sentences with completely different `justification:Certificate` proof structures both attest the same final proposition, and the chain treats them as equal. This is what makes the D39 grounding constructors (`Declared` / `Observed` / `Verified`) compose freely — distinct evidence chains for the same conclusion produce judgmentally-equal certificates.

The unified-ladder, impredicative-Prop design follows the Lean 4 kernel and Calculus of Inductive Constructions tradition. See [D46](../../design/d46-prop-universe-and-proof-irrelevance.md) for the full universe-formation rules, the proof-irrelevance soundness argument, the strong-reduction skip optimization that takes advantage of irrelevance to avoid evaluating `Prop`-typed subterms, and the [validation rule 13 stratification](../../design/d46-prop-universe-and-proof-irrelevance.md) that prevents Prop-level resources from forward-referencing higher-level ones.

## 7.2. Π-types — dependent functions

A Π-type `Π x : A. B(x)` is the type of functions whose return type may depend on the input. When `B(x)` doesn't actually mention `x`, you get the familiar non-dependent function type, which we usually write `A → B`.

In the kernel:

```rust
Exp::Pi(Patt, Box<Exp>, Box<Exp>)  // Pi(x, A, B)
Exp::Lam(Patt, Box<Exp>)           // λ x. body — inhabitant of Pi
```

ESL doesn't have a surface form for general Π-types — you don't write `Π x : T. U` directly. Π-types appear:

- As the type of `program` declarations: `program p : I -> O` produces `Pi(input : I, O)`.

## 7.3. Σ-types — dependent pairs

A Σ-type `Σ x : A. B(x)` is the type of pairs whose second component's type may depend on the first. When `B(x)` doesn't mention `x`, you get a plain pair type `A × B`.

In the kernel:

```rust
Exp::Sig(Patt, Box<Exp>, Box<Exp>)  // Sig(x, A, B)
Exp::Pair(Box<Exp>, Box<Exp>)       // (a, b) — inhabitant of Sig
```

Σ-types are how `class` declarations are encoded ([chapter 6](06-resources-types-and-the-layer.md)). A class with required properties `p1 : T1, p2 : T2` becomes `Sig(p1 : T1, Sig(p2 : T2, One))` — a right-nested chain of Σ-types terminated by the unit type `One`. Each `Construct ex:C { ... }` builds the corresponding pair value.

The "field name" lives in the `Patt` (which is the property IRI for class-derived Σ-types). The kernel uses it for `find_sigma_field` lookups during projection type-checking.

## 7.3a. Indexed inductive families (D48)

Inductive types ([§7.4](#7-4-inductive-types)) split their telescope between **parameters** and **indices**:

- **Parameters** are fixed across all constructors. `data List(A : Set) { nil; cons(A, List(A)) }` has one parameter `A` — every constructor uses the same `A`.
- **Indices** may vary per constructor. `data Vec(A : Set) : Nat -> Set` has one parameter `A` and one index of type `Nat`; the `nil` constructor concludes at `Vec(A, zero)` and the `cons` constructor concludes at `Vec(A, succ(n))`. The conclusion shape's *value* depends on which constructor you used.

The distinction matters operationally because of **dependent pattern matching**. When you scrutinize a value of type `Vec(A, n)`, the type-checker can refine `n` based on which constructor the arm matched: in the `nil` arm `n` is known to be `zero`, in the `cons` arm `n` is `succ(m)` for some fresh `m`. The motive (the return type of the match) can mention `n` and the checker will substitute the refined value per arm.

This is what makes indexed families like `screen:Eq(A, x, y)` load-bearing for reasoning. With one constructor `refl : forall (a : A) => Eq(A, a, a)`, pattern-matching on a value of type `Eq(A, x, y)` forces `x = y = a` for some fresh `a` — there's no `mismatch` arm to write because no constructor produces an `Eq` between distinct things. The type system rejects, at elaboration time, the cases that would observe distinct equal values.

### Singleton elimination — Prop-indexed families

When an indexed family lives in `Prop` ([§7.1](#7-1-universes-the-unified-sortn-ladder-with-prop-at-the-bottom)), large elimination — pattern matching on its values into a `Set` / `Type(n)` motive — is restricted by the **singleton-elim rule** (D46 §7.2, lifted from nanoda_lib). The kernel admits the elimination iff the family fits one of two cases:

- **Case A — zero constructors.** A `Prop`-typed inductive with no constructors is vacuously inhabited only by absurd elimination; pattern matching is admissible to any motive because there are no arms to define. This is the shape of [D49's chain-witness predicates](../../design/d49-chainwitness-machinery.md) (`IsObservedAs`, `IsDeclaredAs`, `IsVerifiedAs`) and [D39's `core:Asserts`](../../design/d39-justification-logic.md) — the proposition exists structurally, its inhabitant is admitted by chain mechanism, and there's nothing to destructure.
- **Case B — exactly one constructor, with restrictions.** The single constructor takes some arguments; for each argument, the kernel requires either:
  - the argument is itself in `Prop` (no computational content the larger universe could observe), **or**
  - the argument appears in the *conclusion's indices* (after the parameters), so the eliminator can reconstruct it from the type alone.

  `Eq(A, x, y)` passes: `refl`'s only argument is `(a : A)` with `A : Set`, but `a` appears in both indices of the conclusion (`Eq(A, a, a)`), so it's reconstructible from the scrutinee's type. `True` passes trivially (no arguments). Existential singletons whose witness lives in `Type` and is *not* visible in the result type fail Case B and are rejected.

Pattern-matching shapes that fit neither case are rejected with `cannot eliminate Prop-typed scrutinee into Set` (or analogous). This is what blocks the Hurkens construction — a multi-constructor `Prop` being case-analyzed into `Type` would let proofs decide propositions about types, breaking consistency — while preserving the four useful singletons (equality, `True`, `And` of two Props, parameter-only Props like `Asserts`). See [D46 §7.4](../../design/d46-prop-universe-and-proof-irrelevance.md) for the soundness argument.

### Wire shape for indexed declarations

The compiler emits the constructor's full Π-telescope (parameters + indices + arguments + conclusion) as a chain-resident `eigentt:TypeExpr` value on the constructor resource's `core:ctor_type` property (declared `core:inductive` since eigenius#229, so the validator type-checks it rather than treating it as opaque JSON); the kernel reads it back at type-check time via the [D47 decoder](../../design/d47-chain-mirrored-eigentt-type-fragment.md). This is why indexed inductives can express dependencies the surface grammar doesn't directly cover — the dependency is encoded in the type expression, which is data the kernel decodes from the layer rather than implicit elaboration. See [§4.5 "Indexed"](04-declarations.md#indexed-d48-indexed-families) for the ESL surface and the wire-shape pointer.

### Pattern unification

Constructor conclusions like `cons : forall (n : Nat) => A -> Vec(A, n) -> Vec(A, succ(n))` declare the index value as a structural template (`succ(n)`). When the elaborator matches the scrutinee's index against a constructor's conclusion template, it unifies the two — `succ(n)` against the scrutinee's actual index, binding `n` to the result. The kernel's pattern unification is restricted to the **Miller fragment** (each free variable in the template applied to distinct bound variables, no projections); patterns outside that fragment surface as `cannot solve unification problem` errors. See [D48 §3.1](../../design/d48-indexed-inductive-families.md) for the algorithm.

## 7.4. Inductive types

Inductive types are introduced by `data` declarations ([§4.5](04-declarations.md)). Internally they're a kernel construct:

```rust
Exp::Const(Iri, Vec<Level>)               // the type-former, by NAME
Exp::InductiveCtor(Iri, Name, Vec<Exp>)   // constructor application
```

**These carry an IRI, not a declaration** (eigenius#229, D76 Phase B). There used to be an
`Exp::Inductive(InductiveDecl)` holding the declaration inline, and an `Exp::InductiveType`
applying it to parameters; both are gone. A type-former is now a `Const` naming the inductive,
applied to its parameters through the ordinary `App` spine — which is what the D47 wire has always
carried — and the layer chain resolves the name to a declaration (`Env::lookup`). Inlining the
declaration meant the codec had to *decode* a target inductive, and every constructor type of it,
merely to build a reference to it.

A constructor's identity is the pair `(inductive IRI, constructor name)`. Constructors have **no
IRI of their own**: they are closed — a type's constructors are exhaustively given by its
declaration — which is exactly what distinguishes them from resources, where anyone may add an
instance in a later layer. See [D79 §2.2.1](../../design/d79-the-representation-of-inductive-types.md).

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

The kernel's evaluator is in [`nbe/eval/mod.rs`](../../../kernel/src/nbe/eval/mod.rs); the type-checker in [`nbe/check/mod.rs`](../../../kernel/src/nbe/check/mod.rs); readback in [`nbe/readback.rs`](../../../kernel/src/nbe/readback.rs).

## 7.9. The `EigonClass` and `EigonPrimitive` bridges

Recapping from [chapter 6](06-resources-types-and-the-layer.md), with the type-theory framing:

`Val::EigonClass(iri)` is the kernel's "type whose definition lives in the layer". It behaves like a fully-reduced type for purposes of comparison (`EigonClass(iri1) == EigonClass(iri2)` iff the IRIs are equal), but to check whether a value inhabits it, the kernel calls `resolve_class_type` and checks against the resulting Σ-type.

`Val::EigonPrimitive(PrimitiveType)` is the type of one of `core:string`, `core:integer`, `core:float`, `core:boolean`, `core:json`. These are the leaf scalar types — they don't unfold further.

These two are the only kernel `Val` constructs that depend on layer state. Everything else is purely syntactic.

## 7.10. Cross-references to the rest of the guide

- **`Set` / `Type(n)`** — appear in `data` parameter types ([§4.5](04-declarations.md)). The kernel treats them as universe levels for cumulativity checks.
- **Π-types** — surface as `program ... : I -> O` ([§4.7](04-declarations.md)). Inhabited by lambdas ([§5.3](05-expressions.md)).
- **Σ-types** — surface as `class` declarations ([§4.2](04-declarations.md)). Inhabited by `Construct` and `Pair` ([§5.6](05-expressions.md), [§5.12](05-expressions.md)).
- **Inductive types** — surface as `data` declarations ([§4.5](04-declarations.md)). Inhabited by constructor application ([§5.2.1](05-expressions.md)). Consumed by `match` ([§5.5](05-expressions.md)).
- **Indexed families** — `data` declarations with an index telescope ([§4.5](04-declarations.md)). The constructor's full Π-telescope is carried on-chain as `core:ctor_type` (§7.6), which is what lets an index depend on an earlier argument.

Sized types are **not** a surface form: they were retired with codata in eigenius#218, and the size-constraint solver they needed was removed unwired in eigenius#139.

## 7.11. Further reading

- Coquand, T., Kinoshita, Y., Nordström, B., & Takeyama, M. (2009). [**A simple type-theoretic language: EigenTT**](https://www.cambridge.org/core/books/abs/from-semantics-to-computer-science/simple-typetheoretic-language-minitt/21451A12E2E24A1F51C82421B066824A). In Y. Bertot, G. Huet, J.-J. Lévy, & G. Plotkin (Eds.), *From Semantics to Computer Science: Essays in Honour of Gilles Kahn* (pp. 139–164). Cambridge University Press. <https://doi.org/10.1017/CBO9780511770524.007>. The EigenTT chapter — the lineage we follow for the term/value split, NbE conversion, and bidirectional checking. Eigenius extends EigenTT substantially with the additions documented in D9, D11, D18, D19; the small calculus in the chapter remains the cleanest entry point to the conversion engine's shape.
- [D9 — NbE unification and type extensions](../../design/d9-nbe-unification-and-type-extensions.md): capability modes, ground type resolution, and the EigenTT extensions the kernel ships.
- [D18 — Ontology-as-types resolution](../../design/d18-ontology-as-types-resolution.md): the bridge mechanism described in chapter 6 with type-theoretic detail.
- [D19 — Inductive and sized types](../../design/d19-inductive-types.md): the formal positivity check and recursor derivation. Its sized-recursion half is retired (eigenius#218).
- [`kernel/src/nbe/term.rs`](../../../kernel/src/nbe/term.rs) and [`kernel/src/nbe/val.rs`](../../../kernel/src/nbe/val.rs): the AST shapes for the kernel's terms and values.
- [nanoda_lib](https://github.com/ammkrn/nanoda_lib) — Chris Bailey's Lean 4 kernel implementation in Rust, which influenced the inductive-type design. The same library Eigenius integrates as the Lean institution checker (see [D28 §8.1](../../design/d28-lean-4-as-institution.md)).
- [*Type Checking in Lean 4*](https://ammkrn.github.io/type_checking_in_lean4/) — the design notes for Lean's kernel, accompanying nanoda_lib. Useful background on universe checking, positivity, and recursor derivation.
- Abel, A. (2010). [**MiniAgda: Integrating Sized and Dependent Types**](https://arxiv.org/abs/1012.4896). In *Workshop on Partiality and Recursion in Interactive Theorem Provers (PAR 2010)*, EPTCS 43, pp. 14–28. <https://doi.org/10.4204/EPTCS.43.2>. Was the foundational paper for the kernel's sized-types treatment. Sized types and codata were removed in eigenius#218; retained here as background for the technique.
- [**MiniAgda**](https://github.com/andreasabel/MiniAgda) (Andreas Abel) — the prototype implementation accompanying the paper. The kernel once carried a port of MiniAgda's `TreeShapedOrder.hs` and of its Warshall meta-variable solver; both are gone (eigenius#139, eigenius#218), the solver having never acquired a caller. Retained here as the reference for the technique. Available on [Hackage](https://hackage.haskell.org/package/MiniAgda).
- [**Agda — Sized Types**](https://agda.readthedocs.io/en/latest/language/sized-types.html) — the production-language documentation for the same machinery in Agda. Background for the technique in a production language. Eigenius does not implement sized types (eigenius#218).

---

Next: **[8. Capability modes →](08-capability-modes.md)**
