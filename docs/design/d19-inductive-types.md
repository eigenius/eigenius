# D19: Inductive Types (Phase 11b)

**Date:** 2026-04-23
**Status:** Implemented (Phase 11b)
**Prerequisites:** Phase 11a (Map/Reduce), D18 (Ontology-as-Types)
**Dependencies:** nanoda_lib positivity + recursor algorithms

## 1. Motivation

### 1.1 Life-science fiber morphisms

Life-science requirements §10 defines fiber morphisms — typed relationships
between ontology resources (e.g. `ConformationalProximity` between protein
poses, `ReplicateRelationship` between assay replicates). These morphisms
are currently representable as flat resource classes, but compositional
queries like "is this proximity the composition of two shorter-range
proximities?" require structural recursion. Without inductive types, such
queries must traverse raw resource/trace trees — slow and non-composable.

### 1.2 Bounded universal quantification

Life-science requirements §4 describes claims of the form "for all poses in
ensemble E, score(p) < −7." The quantifier is bounded to ensemble
membership:

```
Π(p : Pose) (p ∈ E) → (score(p) < −7)
```

This works cleanly when `List` is a proper inductive type with a derived
eliminator. Phase 11a's `Val::List` and `Exp::list()` encode lists as
simplified sums with a `__list_tail` sentinel — adequate for Map/Reduce but
not for recursive proofs.

### 1.3 Replacing the list hack

`Exp::list()` (term.rs) produces `Data[nil : 1, cons : A × __list_tail]`
where `__list_tail` is an unbound sentinel. Phase 11b replaces this with a
proper inductive declaration:

```
inductive List (A : Set) {
  nil  : List A
  cons : A → List A → List A
}
```

yielding a derived recursor that subsumes Phase 11a's Map/Reduce at the
type level.

## 2. Scope

**In scope (Phase 11b):**
- Single (non-mutual), non-nested, strictly-positive inductive types
- Automatic positivity checking
- Automatic recursor (eliminator) derivation
- Iota reduction (pattern matching on constructors)
- Integration with existing NbE readback-based conversion
- List type replacement
- ESL surface syntax

- Sized types for coinductive/inductive termination (issue #16)

**Out of scope (deferred):**
- Mutual inductive types (issue #20)
- Nested inductive types (issue #21)
- Indices / inductive families (issue #22)
- Large elimination restrictions (all inductives can eliminate to any sort)

## 3. Core theory

### 3.1 Inductive declarations

An inductive declaration `I` consists of:
- **Name**: `I`
- **Parameters**: `(x₁ : A���) ... (xₙ : Aₙ)` — shared across all constructors
- **Sort**: the universe level of `I(x₁, ..., xₙ)`
- **Constructors**: `c₁ : T₁`, ..., `cₖ : Tₖ` where each `Tᵢ` is a
  telescope ending in a valid application of `I`

### 3.2 Strict positivity

A constructor is strictly positive with respect to `I` if `I` never
appears in a negative position (left of an arrow) in any constructor
argument. Formally, walking the constructor's Π-telescope: for each
binder type `Aⱼ`, the inductive `I` must not occur in `Aⱼ`.

Rejected example:
```
inductive Bad {
  mk : (Bad → Nat) → Bad   -- Bad appears negatively
}
```

This ensures decidability of type checking and prevents Curry-style
paradoxes.

### 3.3 Recursor

The kernel derives a recursor `I.rec` for each inductive:

```
I.rec : Π(params)(C : I(params) → Sort u)(minors)(major : I(params)) → C(major)
```

where:
- `C` is the **motive** — what is being proved/computed
- **minors** — one per constructor, specifying how to handle each case.
  For a constructor `c��` with recursive arguments, the minor includes
  induction hypotheses
- **major** — the value being eliminated

### 3.4 Iota reduction

When the recursor is applied to a constructor:

```
I.rec params C m₁...mₖ (cⱼ args) ↝ mⱼ(args, ih₁, ..., ihₘ)
```

where `ih₁, ..., ihₘ` are recursive calls to `I.rec` on the recursive
sub-arguments of `cⱼ`.

## 4. EigenTT implementation

### 4.1 New expression forms

```rust
/// Inductive type declaration.
Exp::Inductive(InductiveDecl)

/// Inductive type applied to parameters (the "type former").
Exp::InductiveType(Name, Vec<Exp>)

/// Constructor application.
Exp::InductiveCtor(Name, Name, Vec<Exp>)  // (type_name, ctor_name, args)

/// Recursor application.
Exp::InductiveRec {
    type_name: Name,
    motive: Box<Exp>,
    minors: Vec<Exp>,
    major: Box<Exp>,
}
```

### 4.2 Inductive declaration data

```rust
pub struct InductiveDecl {
    pub name: Name,
    pub params: Vec<(Patt, Exp)>,  // parameter telescope
    pub sort: Exp,                  // universe level
    pub ctors: Vec<InductiveCtorDecl>,
}

pub struct InductiveCtorDecl {
    pub name: Name,
    pub typ: Exp,  // full type including parameters
}
```

### 4.3 New value forms

```rust
/// Inductive type value (evaluated type former).
Val::InductiveType {
    decl: Arc<InductiveDecl>,
    params: Vec<Val>,
}

/// Inductive value (evaluated constructor application).
Val::InductiveVal {
    type_name: Name,
    ctor_name: Name,
    args: Vec<Val>,
}

/// Neutral recursor: blocked on a neutral major premise.
Neut::NtRec {
    type_name: Name,
    motive: Box<Val>,
    minors: Vec<Val>,
    major: Box<Neut>,
}
```

### 4.4 Evaluation rules

**InductiveRec** when major is a constructor:
```
eval(InductiveRec { motive, minors, major }) =
  let major_val = eval(major)
  match major_val:
    InductiveVal(_, ctor_name, args) =>
      let minor = minors[ctor_index(ctor_name)]
      // Build induction hypotheses for recursive args
      let ihs = recursive_args(args).map(|arg|
        eval(InductiveRec { motive, minors, major: arg })
      )
      apply(minor, args ++ ihs)
    Nt(n) =>
      Nt(NtRec { motive, minors, major: n })
```

**InductiveRec** when major is neutral: produce `Neut::NtRec`.

### 4.5 Readback

- `Val::InductiveType` → `Exp::InductiveType(name, readback(params))`
- `Val::InductiveVal` → `Exp::InductiveCtor(type_name, ctor_name, readback(args))`
- `Neut::NtRec` → `Exp::InductiveRec { ... readback all components }`

### 4.6 Type checking

**Inductive declaration** (in `check_decl` for a new `Decl::Inductive`):

1. Check each parameter type is a well-formed type
2. Check the sort is a valid universe
3. For each constructor:
   a. Check the constructor type is well-formed
   b. Run positivity check (§5)
   c. Verify constructor ends with valid application of the inductive

**Recursor application** (in `check_infer`):

1. Infer motive type: must be `Π(x : I(params)) → Sort u`
2. For each minor: check it matches the expected type derived from the
   constructor and motive
3. Check major has type `I(params)`
4. Return `motive(major)` (apply motive to major)

## 5. Positivity checking

Algorithm from nanoda_lib (`check_positivity1`, inductive.rs:666-787):

```
check_positivity(I, ctor_type):
  walk the Π-telescope of ctor_type:
    for each binder (x : A):
      if has_inductive_occurrence(A, I):
        ERROR: non-positive occurrence
      continue with body[x := fresh]
    at end of telescope:
      verify result is a valid application of I to parameters
```

Helper `has_inductive_occurrence(expr, I)` recursively searches for any
reference to `I` in `expr`. This is a conservative check — it rejects
`(I → Nat) → I` even though `I` appears positively in the outer arrow,
because it appears negatively in the inner one.

## 6. Recursor derivation

Following nanoda_lib's three-sub-phase approach
(inductive.rs:922-1311):

1. **Elimination level** (`mk_elim_level`): Determine the universe
   level of the recursor's return type. If the inductive allows large
   elimination (can target any Sort), add a universe parameter.

2. **Motive, majors, minors** (`mk_motives`, `mk_majors`, `mk_minors`):
   - **Motive**: `C : I(params) → Sort u` — what is being computed
   - **Major**: the value being eliminated, of type `I(params)`
   - **Minors**: one per constructor. For constructor
     `cⱼ(a₁, ..., aₘ, r₁, ..., rₖ)` where `rᵢ` are recursive args:
     ```
     mⱼ : Π(a₁ ... aₘ)(r₁ ... rₖ)(ih₁ : C(r₁)) ... (ihₖ : C(rₖ)) → C(cⱼ args)
     ```

3. **Computation rules** (`mk_rec_rules`): For each constructor `cⱼ`:
   ```
   I.rec params C m₁..mₖ (cⱼ args) ↝ mⱼ(non_rec_args, rec_args, ih_args)
   ```
   where `ih_args` are recursive recursor calls on each recursive argument.

## 7. Conversion algorithm

The existing NbE readback-based equality check extends naturally:

- Two `InductiveType` values are equal iff their declarations are the
  same and their parameter values are definitionally equal
- Two `InductiveVal` values are equal iff they have the same constructor
  and their arguments are definitionally equal
- `NtRec` neutrals are equal iff all components are equal

No changes to the core `eq_nf` algorithm — readback handles the new forms.

## 8. Sized types (issue #16)

### 8.1 Problem

Phase 9b-i (D11) introduced codata with a syntactic guardedness check.
This check catches direct unguarded observations (`bad.head` inside
`bad`'s own definition) but cannot track productivity through function
calls. Inductive types introduce the dual problem: structural recursion
must decrease on each call, but the current system has no way to verify
this for general recursion via `Drec`.

The combination is the critical gap: fiber morphism queries will recurse
over inductive structures (e.g. walking a composition chain) while
producing coinductive streams of results. Without sized types, each side
relies on incomplete syntactic checks.

### 8.2 Size sort and annotations

Add a `Size` sort with ordinal structure:

```
Size : Sort               -- the type of sizes
ŝ    : Size → Size        -- successor
∞    : Size               -- limit (no bound)
```

Inductive and coinductive types gain an optional size parameter:

```
inductive List (A : Set) (i : Size) {
  nil  : List A i
  cons : A → List A j → List A (ŝ j)   -- j < i
}

codata Stream (A : Set) (i : Size) {
  head : A
  tail : Stream A (i - 1)              -- strictly smaller
}
```

When the size parameter is `∞`, the type behaves as unsized (backward
compatible with existing code).

### 8.3 Checking rules

**Inductive (structural decrease):** For each recursive call in a
recursor body, the size argument must be strictly smaller than the
major premise's size. The recursor's type ensures this by construction
— recursive arguments have a smaller size index.

**Coinductive (guarded increase):** For each corecursive field body,
the result size must be the successor of the recursive reference's
size. This replaces the syntactic guardedness check with a semantic
one.

**Subtyping:** `T(i) <: T(ŝ i) <: ... <: T(∞)`. A sized type at any
finite size is a subtype of the same type at a larger size. This
ensures that sized code interoperates with unsized code via `∞`.

### 8.4 Implementation sketch

```rust
// New expression forms
Exp::SizeSort,                     // Size : Sort
Exp::SizeSucc(Box<Exp>),           // ŝ(i)
Exp::SizeInf,                      // ∞

// New value forms
Val::SizeSort,
Val::SizeSucc(Box<Val>),
Val::SizeInf,
Val::SizeVar(usize, Name),        // size variable (neutral)
```

The size-checking pass runs after type checking and before evaluation:
1. Annotate all inductive/coinductive types with size variables
2. For each recursive/corecursive call, emit a size constraint
3. Solve constraints: check that all constraints are satisfiable
   (every recursive call decreases, every corecursive call is guarded)

This replaces `check_guarded` in check.rs with a more precise analysis
that handles function-call boundaries.

### 8.5 Migration path

1. Existing codata definitions without size annotations continue to work
   (size defaults to `∞`, syntactic guardedness check remains as fallback)
2. New inductive types can optionally include size annotations
3. When both sides are sized, the checker uses size constraints instead
   of syntactic guardedness
4. Future: make sizing mandatory for `Drec` bodies (closes #13 item 3)

## 9. List replacement

Once inductive types land, `Exp::list()` is replaced:

```
inductive List (A : Set) {
  nil  : List A
  cons : A → List A → List A
}
```

The derived recursor `List.rec` subsumes the current `Exp::Map` and
`Exp::Reduce` — they become sugar for specific recursor applications:

```
map f xs = List.rec A (λ_. List B) (nil) (λx _ acc. cons (f x) acc) xs
reduce f init xs = List.rec A (λ_. B) init (λx _ acc. f acc x) xs
```

Phase 11a's `Exp::Map`/`Exp::Reduce` remain as optimised built-ins
(they avoid the overhead of recursor instantiation for the common case).
`Val::List(Vec<Val>)` remains as the runtime representation for
efficiency.

## 10. ESL surface syntax

```
data List(A : Set) {
  nil : List(A),
  cons : A -> List(A) -> List(A),
}
```

Desugars to `Exp::Inductive(InductiveDecl { ... })`.

Pattern matching:
```
match xs {
  nil => ...,
  cons(x, rest) => ...,
}
```

Desugars to `Exp::InductiveRec { ... }` with the motive inferred from
the match arms.

## 11. Interaction with EigonClass

`EigonClass` types are opaque to the positivity checker — they are not
inductive types and cannot contain inductive occurrences. A constructor
like `mk : EigonClass("urn:...") → MyInd` is always strictly positive
because EigonClass never unfolds to expose a negative occurrence.

## 12. Risk areas

| Risk | Mitigation |
|------|-----------|
| Iota reduction correctness — off-by-one in recursive arg identification | Extensive test suite with known-good examples from nanoda_lib |
| Stack depth on deeply nested inductive values | Iterative evaluation (like Phase 11a's Map/Reduce) for tail-recursive recursors |
| Large values (e.g. lists with 10K elements) | `Val::List(Vec<Val>)` short-circuits; recursor on List produces `Val::List` directly |
| Interaction with codata — inductive containing codata fields | Defer; inductive fields must be inductive or ground types initially |
| Readback of recursive values may diverge | Readback is structural over finite values; infinite values require codata, not inductive |
| Size inference complexity — inferring sizes when user omits annotations | Default to `∞` (unsized); explicit annotations required for sized checking. Syntactic guardedness remains as fallback |
| Size subtyping interaction with NbE conversion | Size annotations are erased during evaluation; subtyping checked before eval, not during readback |

## 13. Implementation plan

| Step | Description | Files |
|------|-------------|-------|
| 1 | Add `InductiveDecl`, `InductiveCtorDecl` to term.rs | term.rs |
| 2 | Add `Exp::Inductive`, `Exp::InductiveType`, `Exp::InductiveCtor`, `Exp::InductiveRec` | term.rs |
| 3 | Add `Val::InductiveType`, `Val::InductiveVal`, `Neut::NtRec` | val.rs |
| 4 | Positivity checker module | check.rs or new nbe/positivity.rs |
| 5 | Recursor derivation module | new nbe/recursor.rs |
| 6 | Evaluation rules (iota reduction) | eval.rs |
| 7 | Readback rules | readback.rs |
| 8 | Type-checking rules | check.rs |
| 9 | Replace `Exp::list()` with proper inductive List | term.rs, ground.rs |
| 10 | ESL parser: `data` declarations | esl/parser.rs |
| 11 | ESL compiler: inductive → resource encoding | esl/compile.rs |
| 12 | Expression builder: resource → `Exp::Inductive` | program/expr.rs |
| 13 | Tests: positivity, recursor, iota reduction, List replacement | tests throughout |
| 14 | Add `Size` sort, `SizeSucc`, `SizeInf` to term.rs and val.rs | term.rs, val.rs |
| 15 | Size constraint generation in type checker | check.rs or new nbe/sized.rs |
| 16 | Size constraint solver | nbe/sized.rs |
| 17 | Replace `check_guarded` with size-based checking for sized definitions | check.rs |
| 18 | Tests: sized inductive, sized codata, mixed inductive+codata | tests throughout |

## 15. References

- [nanoda_lib `src/inductive.rs`](https://github.com/ammkrn/nanoda_lib/blob/main/src/inductive.rs) — positivity checking
  (lines 666-787), recursor derivation (lines 922-1311), iota reduction
  (lines 1137-1170)
- [*Type Checking in Lean 4*, Inductive Types](https://ammkrn.github.io/type_checking_in_lean4/declarations/inductive.html) —
  Lean 4 inductive type specification
- `docs/design/d11-codata-streams.md` — precedent for adding type forms
  to the EigenTT kernel
- `docs/design/life-science-requirements.md` §4 (universal claims),
  §10 (fiber morphisms), §16.1 (inductive types)
- Abel, A. (2010). "MiniAgda: Integrating Sized and Dependent Types" —
  sized types theory

## 16. Decisions

| Question | Decision | Rationale |
|----------|----------|-----------|
| Mutual inductives? | Deferred (#20) | Single-inductive covers List, Nat, Tree; mutual adds complexity without immediate need |
| Nested inductives? | Deferred (#21) | Requires specialize/unspecialize pass (Lean 4 §28-65); not needed for initial use cases |
| Indices (families)? | Deferred (#22) | Parameters suffice for List, Tree; indexed families (Vec n) add dependent pattern matching complexity |
| Large elimination? | Allow all | Life-science recursors need to produce terms at all levels; restriction adds no safety for our use case |
| Sized types? | In scope (#16) | Required for complete termination story when combining inductive recursion with codata corecursion |
| Separate modules for positivity/recursor/sized? | Yes | Keeps check.rs manageable; follows nanoda_lib's separation |
| Keep Map/Reduce as built-ins? | Yes | Performance optimisation for the common case; recursor subsumes but is heavier |
| `Val::List` representation? | Keep | Vec<Val> is the fast path for resource arrays; recursor on List produces Val::List directly |
