# D48: Indexed Inductive Families

*Design document for the Eigenius project — June 2026*

**Status:** Implemented at the kernel + chain-mirror level (Phases A–I + eigenius#71). ESL surface (Phase J) deferred to [eigenius#72](https://github.com/eigenius/eigenius/issues/72) — life-science consumers route through JSON chain commits, not ESL source, so the deferral doesn't block them.
**Tracking issue:** [eigenius#22](https://github.com/eigenius/eigenius/issues/22)
**Depends on:** D19 (Inductive Types), D46 (Prop universe + singleton-elim), D47 (chain-mirrored type fragment)
**Unblocks:** Length-indexed lists (`Vec n`), bounded naturals (`Fin n`), McBride-style dependent pattern matching, full D46 §7 Case B singleton-elim ("ctor arg appears in conclusion"), life-science fiber morphisms with static shape preservation

---

## 1. Motivation

### 1.1 Three concrete shortfalls today

EigenTT's current inductive support (D19) covers *parameterised* inductives only — every constructor produces the same parametric type. Three things this doesn't reach:

**Length-indexed lists (`Vec n`).** Declaring `Vec : Set → Nat → Set` with `nil : Vec A 0` and `cons : A → Vec A n → Vec A (n+1)` requires the type to *vary* with the constructor: `nil` lands in `Vec A 0`, `cons` in `Vec A (n+1)`. Today every constructor must produce `Vec A` (no index argument), so static length tracking is impossible. Life-science requirements §4 lists fixed-shape ensembles where this would matter — protein-conformation ensembles whose pose count is statically known, fiber morphisms between resource arrays of declared cardinality.

**Bounded naturals (`Fin n`).** `Fin : Nat → Set` indexes by the upper bound; `fzero : Fin (n+1)`, `fsucc : Fin n → Fin (n+1)`. Indexing on `Nat` is what makes safe array indexing possible (`Vec A n → Fin n → A` is a total function). Without indices, `Fin` collapses to "some natural", and array indexing remains a partial operation guarded by runtime checks.

**Equality with refined indices.** `Eq : ∀ {A : Set}, A → A → Prop` is already indexed in spirit (D46 §9 moved `Id` to Prop), but EigenTT's current `Id(A, x, y)` treats `x` and `y` as *terms* in a homogeneous type, not as indices that vary per constructor. There's only one constructor (`refl`), and it elaborates `x ≡ y`. Adding proper indices unifies `Id` with the indexed-family treatment used by Lean's `Eq.{u}`, opens the door to McBride-style `J`-elimination patterns (case-splitting on `refl` *refines* `x` to `y` in the goal), and completes D46 §7's singleton-elim rule (Case B's "ctor arg appears in conclusion" clause becomes load-bearing for `Id`-style singletons).

### 1.2 What completes for D46 with indices

D46 §7 specifies the singleton-elim algorithm with two cases:

- **Case A** — zero ctors (always admits large elim). EigenTT has this today.
- **Case B** — exactly one ctor whose every argument *either* is propositional *or* appears in the conclusion (i.e., in the indices of the inductive's return type).

The current D46 implementation explicitly notes:

> EigenTT lacks indexed inductive families (issue #22), so the variant of case B that admits "arg appears in the conclusion" does not apply here — every non-Prop ctor argument fails the test.

This means that today, `Id A x y` with one constructor `refl(a) : Id A a a` is treated by EigenTT as "ctor `refl` has one non-prop argument `a` of type `A`, which is not in Prop and doesn't appear in indices (because there are no indices), so large elim is rejected". D46 papers over this by special-casing `Id` (it's still moved to Prop and large-elim works for it via the unfolding rules in `check_infer` for `Exp::Id`/`Exp::IdJ`), but the *algorithm* is incomplete. Indices close this gap structurally: `refl(a) : Id A a a` has `a` appearing in both index positions, so Case B's second clause fires and admits large elim cleanly without a special case.

### 1.3 Out of scope — D48 ships standalone

D48 is **explicitly not bundled** with the other deferred inductive extensions. The three open issues (#20 mutual, #21 nested, #22 indices) are largely orthogonal axes, and D48 covers only #22.

- **Mutual inductive families** (issue #20) — orthogonal to indices; the `InductiveDecl` extension D48 specifies works inside a single decl. #20 would later add a "block of decls" container around the same per-decl shape. Out of scope here.
- **Nested inductives** (issue #21) — orthogonal in spirit but per its tracking issue requires #20 as a prerequisite (Lean's specialize→check→unspecialize pass uses the mutual machinery). Out of scope here.
- **Inductive-recursive definitions** — substantially more complex than indices alone; not on the roadmap.
- **Higher inductive types** — different territory entirely.

**Reasoning for shipping D48 alone**: indices are independently valuable — Vec, Fin, dependent Eq, McBride-style pattern matching, and the D46 §7 Case B completion all land without needing mutual or nested. JustificationTerm in D39 is self-recursive (not mutual) and built from binary App (no nested-via-list), so D39 v2 specifically doesn't pull #20/#21 into scope. Bundling #20/#21 alongside D48 would roughly double the implementation cost and complexity for capability nothing currently demands.

If a future need arrives that wants all three (Lean Mathlib import is the most plausible candidate), the natural ordering remains: D48 lands first; #20 wraps a "block" container around the per-decl shape D48 specifies; #21 follows #20.

### 1.4 Why now (or: when)

The doc is a *plan*, not a commitment to ship soon. Indices are a substantial implementation — both new metatheory (unification, dependent pattern matching) and a sweep across `nbe/recursor.rs`, `nbe/check.rs`, `nbe/positivity.rs`, `program/ground.rs`, `program/expr.rs`, ESL parser, and D47's chain mirror. Realistic estimate: 4–6 weeks of focused kernel work, with non-trivial soundness risk.

Trigger conditions to start:
- D39 v2 lands and a real institution wants indexed propositions (e.g., a probabilistic-reasoning institution wanting `Dist : Set → Set` indexed by support cardinality).
- Life-science work hits the Vec / Fin wall — currently theoretical; the prototypes use untyped resource arrays.
- A Mathlib import or Lean institution corpus depends on indexed families that don't elaborate to the existing parametric form.

Until one of those triggers fires, the existing D19+D46 stack handles every concrete EigenTT use case. **This doc is the design we'll build against when the trigger arrives**, not a prompt to start now.

---

## 2. Today's state — what indices would change

### 2.1 `InductiveDecl` shape

[kernel/src/nbe/term.rs](../../kernel/src/nbe/term.rs):

```rust
pub struct InductiveDecl {
    pub name: Name,
    pub params: Vec<(Patt, Exp)>,   // shared parameter telescope
    pub sort: Exp,                  // universe (now Sort(n) post-D46)
    pub ctors: Vec<InductiveCtorDecl>,
}
```

`params` is the *only* binder telescope. Every ctor's `typ` ends in `D(p₁ … pₙ)` — no per-ctor index argument.

With indices the shape becomes:

```rust
pub struct InductiveDecl {
    pub name: Name,
    pub params: Vec<(Patt, Exp)>,     // shared parameter telescope
    pub indices: Vec<(Patt, Exp)>,    // index telescope — NEW
    pub sort: Exp,
    pub ctors: Vec<InductiveCtorDecl>,
}
```

Each ctor's `typ` ends in `D(p₁ … pₙ)(i₁ … iₘ)` where the `i_k` are expressions that can depend on the ctor's argument names. The index expressions vary per ctor — that's the whole point.

### 2.2 `Val::InductiveType` shape

```rust
// Current:
Val::InductiveType { decl: Arc<InductiveDecl>, params: Vec<Val> }

// With indices:
Val::InductiveType {
    decl: Arc<InductiveDecl>,
    params: Vec<Val>,
    indices: Vec<Val>,    // NEW
}
```

This is the dominant Val variant that touches everywhere in the kernel — every match arm that handles `Val::InductiveType` needs the new field, even if it ignores indices.

### 2.3 What pattern matching does today vs. with indices

Today (D19 + Phase 11b):

```
match v : List A {
  nil       => e_nil
  cons x xs => e_cons[x, xs]
}
```

The result type is `expected` (provided by the surrounding context); each arm body has type `expected`, no refinement. Internally synthesised as `Exp::InductiveRec` with a *constant motive* `λ_. expected`.

With indices:

```
match v : Vec A n {
  nil           => e_nil       -- in this branch, n ≡ 0
  cons k x xs   => e_cons[k, x, xs]   -- in this branch, n ≡ k+1
}
```

The branch body must type-check with the index variable `n` *refined* by the ctor's index pattern. `e_nil` is checked at type `motive 0` (motive applied to the matched ctor's index); `e_cons[k, x, xs]` at `motive (k+1)`. The motive itself is a function over both the inductive value AND its indices: `motive : (n : Nat) → Vec A n → expected_kind`.

Synthesising the motive when the user writes a bare `match` (no explicit `returning`) is the hard part — it requires inferring how the expected result type depends on the indices. Lean's elaborator does this by "abstracting" the expected type over the indices' values at the scrutinee. For motives the user can't infer, an explicit `returning` clause is required.

### 2.4 Iota reduction with indices

Currently `iota(rec(motive, minors, ctor(args))) = minors[ctor_idx](args, ih(args))`. Indices change this to:

```
iota(rec(motive, minors, ctor(idx_exprs)(args))) =
  minors[ctor_idx](args, ih(args))
```

The indices flow naturally through — they're computed from the ctor's args (the ctor's return type's index expressions). The motive applied to `idx_exprs` gives the result type. No new reduction rule, but `derive_minor_type` in [kernel/src/nbe/recursor.rs](../../kernel/src/nbe/recursor.rs) gets more involved (each minor's expected type now applies the motive to the index expressions extracted from the ctor's return type).

### 2.5 Positivity (essentially unchanged)

Strict positivity (D19 §5) checks that the inductive being declared appears only in strictly positive positions of its ctor argument types. Indices add nothing here — index *expressions* are checked against the index telescope types (typically `Nat`, `Fin`, etc.), but those are previously-declared types, so no recursive positivity issue. The existing checker carries over.

### 2.6 Chain mirror (D47)

D47's `eigentt:TypeExpr` represents types as the expression-level fragment of `Exp`. Indexed-inductive application `Vec A n` was already representable via App-currying:

```
Exp::InductiveType(vec_decl, [Exp::EigonClass(A_iri), Exp::Var("n")])
```

What changes: the `params` slot's interpretation. With indices, the *parameter prefix* + *index suffix* would need to be distinguished. Two options:

- **Keep currying** — App spine fold produces `InductiveType(decl, all_args)`; the kernel split into params/indices uses `decl.params.len()` and `decl.indices.len()`. Same chain shape, smarter decoder.
- **Split the value shape** — give `eigentt:TypeExpr.InductiveTypeApp` a `params: List MiniTTType, indices: List MiniTTType` shape. More semantically transparent on the chain, slightly more chain churn.

Option 1 wins on minimal D47 disruption. The decoder change is small (already walks App spines; just produces a different `Val::InductiveType` shape).

---

## 3. Design space — three architectural choices

### 3.1 Choice 1 — unification approach for dependent matching

Dependent pattern matching needs to solve equations like "the scrutinee is at index `n`, ctor `cons` produces index `k+1`, so `n = k+1` in the branch". The unifier solves these. Options:

**Option A — McBride / Goguen-style first-order unification.** Pattern-match equations between two types reducible to *first-order* shape (no `App`-of-lambdas, no eta-expansion needed) are decidable. Coq uses a variant of this. Sufficient for `Vec`, `Fin`, dependent `Eq`, and the bulk of indexed-family programs that arise from science/engineering modelling. ~300 lines, well-understood algorithm.

**Option B — Lean-style higher-order pattern unification.** A subset of higher-order unification that's decidable for "pattern" terms (each metavariable applied only to distinct bound variables). Catches more cases than first-order, including some motive-inference patterns common in abstract-math proofs. ~600–800 lines. Lean's elaborator depends heavily on this.

**Option C — Punt: require explicit `returning` annotations everywhere.** No unifier. Every `match` carries a user-supplied motive. Works (Agda has historically had stricter unifier modes) but makes indexed types tedious to use.

**Decision:** **A** (locked in). Same framing as §3.2: kernel-level features serve the science/engineering use cases (length-indexed resource arrays, bounded indexing, propositional equalities about chain artifacts), and first-order pattern unification covers those cleanly. The higher-order patterns Option B catches mostly arise in abstract-math proof contexts (motive inference over Mathlib-style lemmas), and **that's the Lean institution's job, not the kernel's** — nanoda inherits Lean's higher-order elaborator and runs it on any Lean term that rides into the chain. The kernel doesn't have to be the universal unifier; Lean is, in its institution. If a science/engineering use case ever needs Option B's expressivity, that's a separate D-doc reopening the question — and would need to explain why routing through the Lean institution doesn't suffice.

### 3.2 Choice 2 — equality treatment (axiom K vs proof-irrelevant Id)

The classical question: when you match `refl : Id A x x` and learn `x = x`, does this allow you to "unify x with x" and conclude `motive x ≡ motive x` definitionally? Answer depends on whether K is admitted:

**With K (Lean's choice).** Pattern matching on `refl` admits any motive. Heterogeneous equality is provable. Streicher's K-axiom holds: every proof of `x = x` is `refl`. Some intensional models reject K (HoTT incompatibility), but for ordinary CIC usage K-on-decidable-types is fine.

**Without K (Agda's `--without-K` mode, HoTT mode).** Pattern matching is restricted by *unification-with-cycle-rejection*: matching `refl : Id A x y` requires the unifier to solve `x = y` without using K. Some pattern matches that work with K (e.g., on equalities between equalities) are rejected. Future-proofs for HoTT-style univalent reasoning.

For EigenTT — D46 §10 already commits to `propext` and `Quot.sound` and rejects `Classical.choice`. K is a separate axiom. Given:
- We've moved Id to Prop (D46 §9), and proof irrelevance already collapses any two equality proofs to the same proof.
- Proof irrelevance + Id-in-Prop is *stronger* than K in some sense (definitionally identifies all equality proofs, not just up to provability).
- **Univalent / HoTT reasoning has a dedicated home.** The Lean 4 institution (D28, Phase 20) lets Lean proofs ride into the chain and be validated by nanoda using Lean's own type theory. If a future use case wants univalence, it lives there — Lean governs whatever it accepts, and the kernel's role is to confirm Lean accepts it, not to be a univalent foundation itself.
- Kernel Prop's job is propositions about science and engineering — physical claims, ontology constraints, justifications about chain artifacts, decidable predicates from numerical institutions. Proof irrelevance is the right shape for that.

**Decision:** **Admit K implicitly via the proof-irrelevant treatment of Id** (locked in). No separate K axiom resource needed — proof irrelevance subsumes it. Abstract-math use cases that need univalence route through the Lean institution rather than the kernel's Prop universe. Documented in §8 (interaction with D46) and §6.7 (risks).

### 3.3 Choice 3 — index erasure

Are indices computationally relevant (carried at runtime) or erased after type-checking?

**Carried.** `Vec A n` values carry `n` at runtime (perhaps redundantly with `xs.length`). Some derived eliminators need this. Lean carries indices.

**Erased.** Indices are type-level only, erased after checking. More efficient but limits some patterns (e.g., deciding the index from the value structurally requires the index to be carried).

For EigenTT — the kernel is a type-checker, not a runtime; "carried" really means "present in `Val::InductiveType`'s `indices: Vec<Val>` field after evaluation, present in the readback Exp, present in the chain mirror." This is the natural choice for an NbE-based system and matches Lean.

**Decision:** **Carried** (Lean-style, locked in). The `Val::InductiveType` extension in §2.2 reflects this. Matches NbE-style evaluation naturally — indices live alongside params in the `Val::InductiveType` payload. Erasure would be a perf optimization revisitable later if memory pressure shows up; not motivating reshaping the AST now.

---

## 4. The detailed design

### 4.1 New term forms

No new `Exp` variants — `InductiveDecl`, `InductiveType`, `InductiveCtor`, `InductiveRec`, `Match` all stay. The `indices` field on `InductiveDecl` is the structural addition.

The existing `Exp::InductiveType(decl, args)` and `Exp::InductiveCtor(decl, ctor_name, args)` interpret `args` as `params ++ indices` (parameters then indices). The decoder/elaborator splits using `decl.params.len()` and `decl.indices.len()`.

### 4.2 Declaration shape

```rust
pub struct InductiveDecl {
    pub name: Name,
    pub params: Vec<(Patt, Exp)>,
    pub indices: Vec<(Patt, Exp)>,    // NEW
    pub sort: Exp,
    pub ctors: Vec<InductiveCtorDecl>,
}
```

ESL surface:

```
data Vec (A : Set) : Nat → Set {
  nil  : Vec A 0
  cons : (n : Nat) → A → Vec A n → Vec A (n + 1)
}
```

The `: Nat → Set` after the parameter list declares the index telescope (here: one index of type `Nat`, result sort `Set`).

### 4.3 Constructor type elaboration

Each `ctor.typ` is a Π-telescope:
- Skip `params.len()` binders (parameter prefix; shared, named).
- Then the per-ctor binders (the ctor's value arguments).
- The terminal application is `D(params...)(idx_expr_1)...(idx_expr_m)` where each `idx_expr_k` is an expression in the parameter + arg variables.

The elaborator verifies each `idx_expr_k` type-checks against the declared index type `decl.indices[k].1` (after substituting params).

### 4.4 Constructor checking — args + indices

When the user writes `cons k x xs` and the expected type is `Vec A n`, the checker:
1. Looks up `cons : (k : Nat) → A → Vec A k → Vec A (k+1)`.
2. Checks `k : Nat`, `x : A`, `xs : Vec A k`.
3. The constructor's return-type indices are `[k+1]`.
4. Unifies the expected indices `[n]` with the constructor's `[k+1]` — i.e., solves `n = k+1`.
5. If `n` is a free unification variable, sets `n := k+1`. If `n` is a concrete expression, checks equality.
6. If unification fails, rejects with "index mismatch: expected `Vec A n` but ctor `cons` produces `Vec A (k+1)`".

### 4.5 `derive_minor_type` extension

For each ctor `c : Π(arg_telescope). D(params)(idx_exprs)`, the minor type becomes:

```
(arg_telescope) → (ih_telescope) → motive idx_exprs (c args)
```

vs. today's:

```
(arg_telescope) → (ih_telescope) → motive (c args)
```

The difference: motive now takes both the indices and the inductive value (`motive : (idx_telescope) → D(params)(indices) → kind`), so the minor's result type instantiates `motive` at the constructor-specific indices.

### 4.6 Motive shape

For elimination over `D(params)(indices)`:

```
motive : (idx_1 : I_1) → ... → (idx_m : I_m) → D(params)(idx_1, ..., idx_m) → kind
```

where `kind` is `Sort(n)` for some `n`. The user-supplied motive in `Exp::InductiveRec { motive, ... }` has this dependent-Pi shape.

### 4.7 Pattern matching elaboration

When the user writes `match v { ctor1 args1 => e1; ... }` without a `returning` annotation:

1. Infer scrutinee type `D(params)(indices)`.
2. The expected result type `T` is known from context.
3. **Motive inference**: try to abstract `T` over the indices' values at the scrutinee. If `T` mentions the indices, the abstraction is canonical (`λ idx_1 ... idx_m. T[idx_1 := old_idx_1_pos, ...]`). If `T` doesn't mention the indices, use a constant motive `λ _ ... _. T`.
4. For each arm:
   a. Compute the ctor's idx_exprs in the arm's local context.
   b. Build a refinement substitution from `indices[i] := idx_exprs[i]`.
   c. Apply the substitution to the arm's expected type.
   d. Check the arm body against the substituted expected type.
5. If motive inference fails (e.g., a complex dependency), require explicit `returning T` annotation.

Step 4b–4d is where the unifier (§3.1 Choice 1) runs. Each `indices[i] = idx_exprs[i]` becomes a unification equation; the solved substitution is applied to the arm's expected type.

### 4.8 Iota reduction

`iota(rec(motive, minors, ctor(idx_exprs)(args)))`:
1. Lookup `minor_i = minors[ctor_idx_of(ctor)]`.
2. Build the induction hypotheses for recursive args (unchanged from D19).
3. Apply: `minor_i(args, ih(args))`.

The indices flow through the motive application implicitly — the minor's type was already derived to apply `motive` to the constructor's idx_exprs, so the result has the right type.

### 4.9 Index telescope dependency

Index telescopes can be dependent: `Vec : (A : Set) → Nat → Set` is non-dependent, but `Slice : (xs : List A) → Fin (length xs) → Set` would be. The index telescope's later entries can reference earlier ones, like any Π-telescope.

EigenTT's existing `(Patt, Exp)` pair vector handles this naturally — each `Exp` is checked in the context extended with all prior bindings.

### 4.10 Index inference holes

When the user writes `cons x xs` (no explicit index), the elaborator infers the index from the expected type. With explicit syntax `Vec A 3`, the index `3` is concrete; with hole syntax `Vec A _`, the index is a metavariable that the unifier solves.

For EigenTT — we don't have metavariables yet (the type checker is bidirectional + readback-based, no unification metas). Adding them is part of the unifier work (§3.1). For v1, indices in user-facing positions must be either fully concrete or come from the expected type's shape.

---

## 5. Implementation plan

Estimated effort: **4–6 weeks** for a single experienced kernel engineer. Largest discrete pieces are §3.1's unifier and §4.7's motive inference.

### 5.1 Phase A — AST + Decl shape (~3 days)

- Add `indices: Vec<(Patt, Exp)>` to `InductiveDecl`.
- Add `indices: Vec<Val>` to `Val::InductiveType`.
- Update every match arm across `nbe/`, `program/`, `validation/`, `esl/`. Tests must pass with empty `indices` (compatibility — existing non-indexed inductives have `indices: vec![]`).
- Update `Val::InductiveType` `PartialEq` and any helpers.

**Exit criterion:** workspace builds; all existing tests pass with `indices: vec![]` everywhere.

**Status on landing:** complete. ~169 call sites swept; readback flattens `params ++ indices` into the single args slot; stub-Arc pattern preserved (eval skips arity check when `decl.indices.is_empty()`).

### 5.2 Phase B — Ctor type elaboration (~3 days)

- Extend the ctor-typ elaborator (currently in [program/ground.rs decode_ctors](../../kernel/src/program/ground.rs)) to recognise index expressions in the ctor's terminal application.
- Verify each index expression type-checks against the declared index telescope type.
- Reject ctor types whose conclusion's argument count doesn't equal `params.len() + indices.len()`.

**Exit criterion:** can declare `Vec A : Nat → Set` and the ctors `nil`/`cons` type-check at declaration time.

**Status on landing:** complete. `validate_indexed_ctor_conclusions` + `ctx_with_param_and_arg_binders` in [kernel/src/nbe/check.rs](../../kernel/src/nbe/check.rs) wired into `check_type`'s `Exp::Inductive` arm; eval splits `params ++ indices` for indexed decls (kept stub-Arc pattern intact for non-indexed). 6 tests covering well-formed `SimpleVec`, arg-count mismatch, index-type mismatch, non-indexed backward-compat.

### 5.3 Phase C — First-order unifier (~1.5 weeks)

- New module `kernel/src/nbe/unify.rs`. Pattern-style first-order unification on `Val` (with readback for variable-binding equality).
- API: `unify(level, lhs, rhs, mvars) -> Result<Substitution, UnifyError>`.
- Test coverage: trivial unification (`x = x`, `f x = f y` ⇒ `x = y`), occurs check (`x = f x` fails), constructor mismatch (`zero = succ y` fails), variable solve (`?n = succ k` ⇒ `?n := succ k`).

**Exit criterion:** unifier tests pass; can solve `Vec A (succ k) = Vec A ?n` ⇒ `?n := succ k`.

**Status on landing:** complete. `Neut::Meta(MetaId, Vec<Val>)` variant in [nbe/val.rs](../../kernel/src/nbe/val.rs); new module [nbe/unify.rs](../../kernel/src/nbe/unify.rs) with `MetaCtx`, `unify`, `zonk`, occurs check, pattern-spine restriction. v1 only solves bare metas (empty spine) — lambda construction for non-empty spines deferred until a real consumer needs it. 17 tests.

### 5.4 Phase D — Constructor checking with index unification (~1 week)

- Extend the constructor checking path so applying `cons k x xs` against `Vec A n` runs the unifier and checks the resulting substitution.
- Reject with structured errors when unification fails.
- Tests: positive (`nil : Vec A 0` accepted, `cons 0 x nil : Vec A 1` accepted), negative (`nil : Vec A 5` rejected with index mismatch).

**Exit criterion:** can construct typed Vec values; index mismatches caught at check time.

**Status on landing:** complete. `check_inductive_ctor_args` signature gained `expected_indices: &[Val]`; after `subtype_of_with_hyps` checks params, the new path runs `unify` on each (actual-index, expected-index) pair via a fresh per-call `MetaCtx`. 4 ctor-checking tests + Phase F's coherence tests exercise the same path indirectly.

### 5.5 Phase E — `derive_minor_type` extension (~3 days)

- Modify [nbe/recursor.rs:derive_minor_type](../../kernel/src/nbe/recursor.rs) to apply the motive at the ctor-specific index expressions, not just the unit motive application.
- Update tests in `recursor.rs::tests` to cover indexed cases.

**Exit criterion:** the derived minor types for `Vec`'s recursor are correct.

**Status on landing:** complete. `derive_minor_type` extracts conclusion-indices from `current` (residual after Π-peel) and applies the motive at them before the ctor app — `motive idx_1 ... idx_m (c args)` instead of pre-D48's `motive (c args)`. IH binders similarly: each recursive arg of type `D(params)(arg_idx_exprs)` yields IH type `motive arg_idx_1 ... arg_idx_m arg`. 3 tests including `SimpleVec` cons/nil + Nat backward-compat.

### 5.6 Phase F — Pattern matching with dependent motive (~1 week)

- Extend `check_match` in [nbe/check.rs](../../kernel/src/nbe/check.rs) to do motive inference (§4.7 step 3) and per-arm substitution (step 4b–4d).
- Detect when motive inference fails; emit a clear "needs explicit `returning T` annotation" error.
- Tests: bare `match` over `Vec` (no annotation), `match` with explicit annotation, `match` over `Eq A x y` doing dependent rewrite.

**Exit criterion:** dependent pattern matching works on `Vec` and `Eq`.

**Status on landing:** partial. `check_match` now captures the scrutinee's indices and runs a per-arm **index-coherence check** — if the ctor's conclusion indices fail to unify with the scrutinee's indices, the arm is rejected as unreachable with a structured error pointing the user at `Exp::InductiveRec` with `returning T`. Full **motive-refinement** (rewriting `expected` under index-equation substitutions inside the arm body) is deferred — the constant-motive path already works for the common case (`expected` doesn't depend on scrutinee indices), and the user can hand-write the explicit motive via `Exp::InductiveRec` when refinement is genuinely needed. 3 Phase F tests + the singleton-elim suite exercise the coherence path.

### 5.7 Phase G — Iota + Match elaboration update (~2 days)

- Verify iota reduction works through indexed recursors (mostly unchanged from D19; the indices are already encoded in the ctor's typ).
- Update `Match` elaboration tests.

**Exit criterion:** end-to-end `match` programs over indexed types compute correctly.

**Status on landing:** complete. Iota reduction works on indexed inductives without modification — indices were already encoded in the ctor's typ (handled by Phase B's eval split) and minor sequencing is index-agnostic. 2 end-to-end tests in [nbe/eval.rs](../../kernel/src/nbe/eval.rs) (`SimpleVec` nil + cons under `InductiveRec`).

### 5.8 Phase H — Singleton-elim Case B completion (~2 days)

- Extend `ctor_args_all_propositional` in [nbe/check.rs:large_elim_admitted](../../kernel/src/nbe/check.rs) to also admit args that appear in the conclusion's indices. This closes D46 §7 Case B's second clause that the current implementation explicitly skips.
- Reposition `Id`'s special handling — with indices, `Id A x y` becomes a standard indexed inductive whose `refl(a)` ctor's `a` arg appears in both indices. Large elim works via the standard rule.
- Add the previously-impossible large-elim test (Case B with non-Prop arg that appears in conclusion).

**Exit criterion:** D46 §7's algorithm matches the doc text without the "EigenTT lacks indices" caveat.

**Status on landing:** complete. `ctor_args_pass_singleton_b` (renamed from `ctor_args_all_propositional`) accepts `num_indices`, extracts conclusion indices, and admits non-Prop args that *are themselves conclusion indices* (the argument variable occurs as an index — membership, not mere mention; tightened per finding F-4 of the NbE port-fidelity analysis, matching nanoda's `large_elim_test_aux`). Shadowed binders are unrecoverable and don't qualify. The canonical `Eq A x y` admits large elim via the proper algorithm; tests verify Eq admitted, BadIxProp (non-Prop arg not in conclusion) rejected, mentions-only index rejected, shadowed reference rejected, non-indexed backward-compat.

### 5.9 Phase I — D47 chain mirror update (~3 days)

- Update D47's decoder to handle the new `Val::InductiveType { indices }` field.
- The encoder needs no change if we keep App-currying (§2.6 option 1).
- Update D47 doc.

**Exit criterion:** D47 codec round-trips an indexed inductive value.

**Status on landing:** complete (type-level), plus the term-level extension landed under [eigenius#71](https://github.com/eigenius/eigenius/issues/71). The codec round-trips:
- *Type-level indices* (e.g. `IxClassFamily SomeClass OtherClass`) via the existing App-curried `Exp::InductiveType` ↔ `ConstRef + App` flow.
- *Term-level indices* (e.g. `AssayShape (succ zero)`) via new `eigentt:TypeExpr` ctors: `UnitVal`, `Pair`, `CtorApp`, plus forward-declared `LitInt`/`LitString`/`LitFloat` (decoder errors until EigenTT's `Exp` adds literal variants).
- Commit-time validator extension `check_eigentt_ctor_app` verifies `CtorApp`'s decl IRI resolves to an `InductiveType` and the named ctor exists.

9 new tests, including the **`AssayShape (succ zero)` end-to-end round-trip** that unblocks life-science case 3.

### 5.10 Phase J — ESL surface (~3 days)

- Extend the `data` declaration parser to accept `: I → ... → Sort` index telescope syntax.
- ESL Pratt syntax for `Vec A 3`, dependent pattern matching with `returning T` annotation.

**Status on landing:** deferred to [eigenius#72](https://github.com/eigenius/eigenius/issues/72). The "~3 days" estimate was optimistic — clean indexed-type ESL needs index-telescope parsing, ctor result-type annotations, *and* a general expression parser for index values in ctor conclusions (~1 week total). Critically, **D48's primary near-term consumers don't need ESL** — life-science institutions commit indexed-shape resources as JSON via the codec (now complete), D39 v2 reads/writes via the codec, Lean institution imports go through Lean's own elaborator. Pick up when an ESL-source consumer arrives.

### 5.11 Phase K — Validator + documentation (~2 days)

- D32-style chain validator handles the extended `InductiveType` declarations naturally (the new `indices` property declarations follow the existing pattern).
- Update implementation-plan.md to reference D48 as a new phase.
- Mark D48 status: Implemented.

**Status on landing:** complete. Validator's `check_eigentt_ctor_app` (under #71) covers the term-level CtorApp resolution check; the existing chain-inductive walker handles indexed `InductiveType` shapes naturally via D32's machinery. This doc now reflects what landed; `implementation-plan.md` update happens in the same commit pass as the doc finalisation.

---

## 6. Risk areas

### 6.1 Unifier soundness

First-order pattern unification is decidable and sound *if the algorithm is implemented correctly*. The main pitfalls:
- **Occurs check** — missing it admits `?x = f ?x` ⇒ ⊥.
- **Variable scope** — a metavariable solved with a term containing a variable not in the meta's scope is unsound.
- **Eta-expansion handling** — solving `?f x = (λ y. body)` requires care.

Mitigation: port nanoda's or another mature implementation; extensive property-based tests for round-trip behaviour; reject anything that doesn't fit the well-understood first-order pattern fragment with a clear "out of scope" error.

### 6.2 Motive inference failure modes

Some dependent matches have multiple equally-valid motives, and not all of them work — the elaborator must pick one that actually type-checks the body. Lean handles this through unification + retry; EigenTT's bidirectional checker would do similar.

Mitigation: when motive inference is ambiguous, require explicit `returning T`. Clear error messages guide the user.

### 6.3 Performance of the unifier in hot paths

Conversion check (`def_eq_at_type`) is hot. Adding unification means every comparison of `Vec A n = Vec A m` runs the unifier. For closed terms (`n`, `m` are concrete), this is just structural equality and is fast; for open terms (with neutrals/metavariables), it can be expensive.

Mitigation: short-circuit closed-vs-closed comparisons with `eq_nf` first; only fall to unification when one side has a metavariable. Cache via existing `CheckCtx::type_cache`.

### 6.4 Interaction with sized types (D19 §8)

Sized inductives use `SizedPi` binders. Indexed sized inductives would need both: `Vec : (i : Size) → Set → Nat → Set`. The size param and the Nat index live in different telescopes (parameters for size, indices for `Nat`). The implementation should be straightforward — parameter telescope absorbs the size, index telescope absorbs the `Nat` — but the cross-product of features needs test coverage.

Mitigation: explicit tests for sized + indexed combinations.

### 6.5 Backward compatibility of chain-committed inductives

Existing chain-committed `core:InductiveType` resources have no `core:indices` property. Decoding them with the new D48 changes should treat missing `indices` as empty (the inductive is non-indexed). Forward-compatible.

Mitigation: validator and ground.rs decoder treat absent `indices` property as `[]`. Add fixture tests showing pre-D48 inductive declarations still parse.

### 6.6 D39 v2 interaction

If D39 v2 lands before D48, it'll use the existing parameter-only `Asserts(iri) : Prop`. After D48, `Asserts` could become indexed by the *context* it asserts in — but this is a D39 v2 decision, not a D48 design constraint. Mitigation: D48 doesn't depend on D39 v2; D39 v2 can opt in to indices later.

### 6.7 K-axiom decision lock-in (intentional)

§3.2 locks in K implicitly via proof irrelevance for the kernel's Prop universe. This makes kernel Prop permanently incompatible with HoTT-style univalent reasoning — a Prop where `Id A x y` has multiple distinguishable inhabitants doesn't typecheck under proof irrelevance.

This is a deliberate scope decision, not a constraint to mitigate. Kernel Prop's job is propositions about science and engineering — physical claims, ontology constraints, justifications about chain artifacts, decidable predicates from numerical institutions. Proof irrelevance is the right shape for those uses. Abstract-math use cases that genuinely want univalence have a dedicated home: the **Lean 4 institution (D28, Phase 20)** lets Lean terms ride into the chain and be validated under Lean's own type theory, which a future Lean stratum could configure with univalent axioms if needed. The kernel doesn't have to be the foundation for all of mathematics — Lean does that, in its institution.

No mitigation needed. If a science/engineering use case ever surfaces that genuinely wants univalence inside kernel Prop (rather than routing through the Lean institution), that's a separate D-doc reopening the §3.2 decision — but the doc would have to first explain why the Lean institution doesn't suffice for that case.

---

## 7. Remaining open questions

After the §1.3, §3.1, §3.2, and §3.3 lock-ins, three questions remain genuinely open. None affect the design *direction* — they're either implementation-time judgment calls or future-feature scoping.

### 7.1 Implementation-time judgment calls (decide at kick-off)

These don't affect the design and can be settled when implementation starts; the obvious defaults are noted.

- **Unifier module location**: new `nbe/unify.rs` (default — keeps the unifier independently testable) vs. extending `nbe/check.rs`. Mechanical organization.
- **Metavariable representation**: `Neut::Meta(id, scope)` extending the existing neutrals (default — matches Lean's approach and EigenTT's `Neut::Gen` is structurally close) vs. a fresh `Val::Meta(id)` variant. Final call falls out of the first unifier prototype's readback needs.

### 7.2 Future-feature scoping (decide when a consumer arrives)

- **Indexed codata**: parallel to D48 for codata (e.g., `Stream A : Nat → Type`). Out of scope here. Tracked as [eigenius#70](https://github.com/eigenius/eigenius/issues/70) — estimated ~2 weeks as a follow-on once D48 lands (most machinery reuses; only new piece is the observation-availability check, which is refinement-typing flavoured and considerably simpler than D48's unifier). Pick up when a concrete consumer appears (D11-style sized streams indexed by length, time-series with declared cadence).
- **No-confusion principle**: auto-generate `noConfusion`-style lemmas per inductive (e.g., proving `zero ≠ succ n`) à la Lean. Some dependent-matching patterns are nicer with them. Tracked as [eigenius#69](https://github.com/eigenius/eigenius/issues/69). Pick up when a science/engineering use case actually wants such a proof (e.g., proving two ensembles of distinct sizes can't be equated).

### 7.3 When to start D48 itself

Reactive per §1.4. Trigger conditions (D39 v2 wanting type-level `JustifiedBy`, life-science Vec wall, Lean institution corpus requiring indexed families) haven't fired. The doc exists so we can start cold when one does; nothing to do until then.

---

## 8. Interaction with prior design

### 8.1 D19 (Inductive Types — Phase 11b)

D48 *extends* D19's `InductiveDecl` with an `indices` field. All existing non-indexed inductives have `indices: vec![]` and behave identically. D19's positivity checker, recursor derivation, and iota reduction all carry forward with minor edits.

### 8.2 D46 (Prop universe)

§1.2 already covered this — singleton-elim Case B's "ctor arg appears in conclusion" clause becomes properly meaningful with indices. The current D46 §7 implementation has a TODO-style note about this; Phase 5.8 closes it. K-axiom is implicitly admitted via proof irrelevance (§3.2).

### 8.3 D47 (chain-mirrored type fragment)

§2.6 + Phase 5.9 cover the codec extension. App-currying continues to work; the decoder splits the App spine's args into params + indices using the resolved decl's telescope lengths.

### 8.4 D39 (justification logic)

If D39 v2 lands first, no impact. If D48 lands first, D39 v2 *could* use indexed `Asserts` (e.g., context-indexed propositions for modal-logic-flavoured institutions). D48 doesn't force this — the existing parameter-only path remains available.

### 8.5 Sized types (D19 §8)

§6.4 — sized + indexed combinations work in principle (size on the parameter telescope, indices on the index telescope) and need explicit test coverage in Phase D/E.

---

## 9. References

### 9.1 External

- nanoda_lib (`references/nanoda_lib/src/inductive.rs`) — Lean's indexed-family implementation. `local_indices: Vec<Vec<ExprPtr>>` (`line 173`) sits beside `local_params`. The whole `inductive.rs` walks an indexed-aware `InductiveCheckState`.
- McBride, C. & McKinna, J. (2004). "The view from the left." Original presentation of dependent pattern matching with index-driven case splitting.
- Goguen, H., McBride, C., & McKinna, J. (2006). "Eliminating dependent pattern matching." Compilation of dependent matching to recursor + eliminator applications.
- Carneiro, M. (2019). "The Type Theory of Lean." Reference for indexed inductive families in CIC + Lean's K treatment.
- Cockx, J. (2017). "Dependent Pattern Matching and Proof-Relevant Unification" (PhD thesis). Foundational treatment of pattern unification without K.

### 9.2 Internal

- [D19 — Inductive Types](d19-inductive-types.md) §2 ("Deferred — scope boundary") explicitly defers indexed families to this doc.
- [D46 — Prop universe](d46-prop-universe-and-proof-irrelevance.md) §7 references the missing Case B clause that D48 completes.
- [D47 — Chain-mirrored EigenTT type fragment](d47-chain-mirrored-eigentt-type-fragment.md) §2.1 lists `InductiveType(decl, args)` as a type-level form — the chain mirror naturally accommodates indices via App-currying.
- [implementation-plan.md](implementation-plan.md) Phase 11b ✓ — D48 lands as a follow-on extension when a trigger condition fires.
- [eigenius#22](https://github.com/eigenius/eigenius/issues/22) — tracking issue.
