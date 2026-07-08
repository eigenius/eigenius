# D46: Prop Universe, Proof Irrelevance, and the Axiom-as-Resource Framework

*Design document for the Eigenius project — June 2026*

**Status:** Draft
**Required before:** D39 implementation (justification logic depends on Prop)
**Depends on:** D9 (NbE + type extensions), D18 (Ontology-as-Types Resolution), D19 (Inductive Types), D47 (Chain-mirrored EigenTT type fragment) — D47 is a small prerequisite for §10's axiom-statement encoding; Phases A–G of D46 do not depend on it
**Unblocks:** D39 (Justification Logic), future propositional institutions, indexed inductive families (issue #22) if pursued

---

## 1. Motivation

### 1.1 What's missing

Eigenius's EigenTT today has a cumulative `Type` ladder (`Exp::Set` = `Type(0)`, `Exp::Type(n)` for higher levels) and no propositional universe. Every term has *computational content*: two functions `f, g : A → B` are equal only if they reduce to the same value, two proofs of an equality are equal only if their reflexivity witnesses are convertible. This is fine for the data-and-computation half of the platform — Resources, Programs, Verdicts all benefit from extensional equality being structural.

It is not fine for the *reasoning* half. D39 (justification logic) needs to talk about propositions whose proofs are interchangeable: two distinct justifications for the same claim should *witness the same fact*, even though the justifications themselves are observably different chain artifacts. D39's current draft sidesteps this by encoding atomic propositions as `Asserts(iri) : Type` (a parameter-only inductive with no constructors), which works *only* because no one ever constructs a `Type`-level proof of `Asserts(iri)` — every warrant lives in the surrounding `JustificationTerm` resource and is validated procedurally. The kernel is not asked to reason about propositional equality at all.

That works for a v1 of D39, but it forces every D39 extension that wants to *prove* something about justifications (e.g., "every well-formed App term justifies an implication-consequent") to route through institutional dispatch rather than the kernel. The platform's "the kernel is the type theory" posture starts to leak.

### 1.2 Why now

D39 is the next institution we want to ship. Implementing it on top of `Asserts(iri) : Type` would lock in the leak. The cleaner sequence is:

1. **D46 (this doc)** — add a `Prop` universe with proof irrelevance and an axiom-as-Resource framework.
2. **D39 v2** — redraft `Asserts(iri) : Prop`, `JustificationTerm : Type`, `Validates : Justification → Prop → Prop`. Per-institution axioms ride into the chain as `core:Axiom` Resources.
3. **Subsequent institutions** — anything that talks about propositional truth (classical fragments, modal logics, probabilistic propositions) plugs into the same Prop layer with its own axiom set.

Doing D46 first costs ~3–6 weeks of kernel work; doing it after D39 v1 costs that plus the cost of rewriting every D39 internal that pattern-matched on `Asserts(iri) : Type`.

### 1.3 Scope

This document specifies the kernel-level Prop discipline and the chain-level axiom framework. It does *not* specify D39's redraft (that's D39 v2's job), or the indexed inductive families optionally enabled by Prop (issue #22, deferred).

In scope:
- A unified `Sort` ladder replacing `Exp::Set`/`Exp::Type(n)`, with `Sort(0) = Prop`.
- Impredicative Prop formation rules.
- Proof-irrelevant conversion.
- Singleton-elimination for large elim out of Prop.
- Strong-reduction skip and projection / eta restrictions for Prop-typed subterms.
- `Id` / `Refl` / `IdJ` repositioned to live in Prop.
- The `core:Axiom` ontology class and the `delta`-skip + provenance machinery.
- `propext` and `Quot.sound` as the default-admitted axioms; explicit rejection of `Classical.choice` at kernel level.

Out of scope:
- Migration of existing on-disk chains. Pre-production posture: drop and re-seed.
- D39 redraft.
- Indexed inductive families.
- Universe polymorphism.

---

## 2. Today's state

### 2.1 The universe ladder

`kernel/src/nbe/term.rs`:

```rust
pub enum Exp {
    Set,                    // Type(0)
    Type(usize),            // cumulative ladder; Type(0) = Set, Type(1) : Type(2), ...
    // ...
}

pub enum Val {
    Set,
    Type(usize),
    // ...
}
```

`check.rs` enforces stratification: `Exp::Type(n) : Val::Type(m)` iff `n + 1 == m`; `Exp::Set : Val::Type(1)`. Validation Rule 13 (`validation/mod.rs:746`) extends this to the resource layer: a level-N resource may only reference level-≤(N-1) resources.

### 2.2 Existing propositional-adjacent forms

Three EigenTT terms already behave morally like propositions but live in `Set`/`Type`:

- `Exp::Id(A, x, y)` — propositional equality. Today `Id(A, x, y) : Set`. Its only inhabitant constructor is `Refl(a)`.
- `Exp::NativeDecide(constraint, value)` — reduces to `Refl` when the constraint holds, otherwise neutral. Lives at the type level of whatever equality it witnesses.
- `Exp::DecEq(A, x, y)` — reduces to `Refl` if `x = y`, else neutral.

These are propositions in the informal sense (one-inhabitant types whose inhabitant is uninformative), but the kernel treats them with full computational content. Two distinct neutral `NativeDecide` terms over equivalent values are not definitionally equal.

### 2.3 What proof irrelevance would buy

If `Id(A, x, y) : Prop`, then any two proofs of `x = y` are definitionally equal — `Refl(a)` and a `NativeDecide` that reduces to `Refl` would be interchangeable in *any* context. This eliminates a class of conversion failures that currently force users to manually rewrite using `IdJ`.

More fundamentally, it lets D39 (and future propositional institutions) treat propositions as *opaque-up-to-proof* without each institution implementing its own proof-irrelevance discipline.

---

## 3. Universe structure

### 3.1 Unified Sort ladder

Replace `Exp::Set` / `Exp::Type(n)` with a single `Exp::Sort(usize)`. The level semantics:

```
Sort(0) = Prop
Sort(1) = Set       (the universe of small types; was Exp::Set)
Sort(2) = Type(1)   (was Exp::Type(1))
Sort(n+1)           (was Exp::Type(n) for n ≥ 1)
```

In other words, `Set` is no longer at the bottom of the ladder — `Prop` is. The naming convention shifts by one: what users currently write as `Type(0)` becomes `Type(0)` still (they're at level 1 in the new internal indexing), but the AST stores `Sort(1)`. ESL surface syntax keeps `Set` and `Type(n)` as sugar for `Sort(1)` and `Sort(n+1)` respectively, and adds `Prop` as sugar for `Sort(0)`.

```rust
pub enum Exp {
    Sort(usize),  // 0 = Prop, 1 = Set, 2+ = Type(n-1)
    // ...
}

pub enum Val {
    Sort(usize),
    // ...
}
```

The `Set`/`Type(n)` AST variants are deleted (no compatibility shims; pre-production posture).

### 3.2 Sort typing

A single rule, uniform across the ladder:

```
Sort(n) : Sort(n+1)
```

Cumulativity is decided positionally: `Sort(m) <: Sort(n)` iff `m ≤ n`. The cumulative chain is `Prop ⊆ Set ⊆ Type(1) ⊆ Type(2) ⊆ …` — note that **Prop is cumulative into Set** under the standard CIC reading; a proposition can be coerced into a type when its propositional content is forgotten. (This matches Lean 4's cumulative variant. Coq's predicative-Set tradition gives Prop a separate non-cumulative status; we take the simpler unified view.)

### 3.3 Pi formation (predicative case for Type, impredicative for Prop)

See §4.

### 3.4 Sigma and the other type formers

Sigma, sum, and the existing inductive formers all live at the max of their constituents' levels:

```
Σ (x : A : Sort(m)) (B : Sort(n)) : Sort(max(m, n))
```

For Sigma and sums, Prop participates symmetrically: a record of two Prop fields is in Prop, a record mixing a Prop field and a Type field is in Type. This differs from Pi (§4) because the impredicativity quirk is specific to Pi.

### 3.5 Validation Rule 13

Stratification reads almost identically after the rename: a level-N resource may only reference level-≤(N-1) resources. Prop-level (level 0) resources may reference no other resources at the type-system level (they are "ground" propositions like `Asserts(iri)` in D39). Existing fixtures using `Set` get rewritten to `Sort(1)` by the AST migration.

---

## 4. Predicativity: impredicative Prop

### 4.1 The formation rule

Pi formation has two cases:

```
case Pi codomain in Prop:
    Π (x : A : Sort(m)). (B : Sort(0)) : Sort(0)
                                          ^^^^^^^
                                          impredicative

case otherwise:
    Π (x : A : Sort(m)). (B : Sort(n)) : Sort(max(m, n))   when n > 0
                                          ^^^^^^^^^^^^^^^^
                                          predicative
```

So `∀ (P : Prop), P → P` lives in `Prop`, and so does `∀ (X : Type 17), X → X`. Quantification over arbitrarily large types collapses back into Prop when the codomain is propositional.

This is the source of impredicative Prop's expressiveness — and the reason §7 (large elimination) and §10 (axioms) have to be careful.

### 4.2 Why impredicative

Justified in detail during D46 design discussion (see commit history); the short version:

1. **D39 wants it.** Rule schemas of the form `∀ (t : Term), Holds(P, t) → Holds(Q, t)` should naturally be propositions. Predicative Prop forces such rules into Type, which then makes D39's universe accounting awkward.
2. **Standard practice.** Lean 4, Coq, and most CIC-family systems pick impredicative Prop. Knowledge / proof transcribed from those systems lands here cleanly.
3. **Bounded cost.** The kernel cost is one extra branch in Pi formation plus the singleton-elim rule (§7). Both are well-understood and have reference implementations in nanoda_lib.

The cost we accept: large elimination must be restricted (§7) to preserve consistency. Without that restriction, impredicative Prop + unrestricted elimination = Burali-Forti / Hurkens paradox.

### 4.3 Universe variables (rejected)

We do *not* introduce universe variables / universe polymorphism in this design. Sort levels are concrete `usize` values. Universe polymorphism is a substantial separate piece of work that interacts with this design but is not blocking — defer until a concrete D-XX demands it.

---

## 5. Conversion: proof irrelevance

### 5.1 The rule

In the conversion algorithm (`nbe/conversion.rs`'s `def_eq` analog), after weak-head reduction of both sides, check:

```
def_eq(t1, t2):
    let t1' = whnf(t1)
    let t2' = whnf(t2)
    if proof_irrel_eq(t1', t2'):
        return Ok(())
    // ... existing structural conversion ...

proof_irrel_eq(t1, t2):
    let T1 = infer_type(t1)
    if !is_proposition(T1):
        return false
    let T2 = infer_type(t2)
    if !is_proposition(T2):
        return false
    def_eq(T1, T2)  // the types must be convertible-as-propositions
```

`is_proposition(T)` holds iff `whnf(T)` reduces to `Sort(0)`, or to a type former whose target sort is `Sort(0)` (e.g., `Π x. (B : Prop)` is a proposition under §4.1).

This is a 9-line addition modeled directly on nanoda_lib's `proof_irrel_eq` (`tc.rs:1301-1309`).

### 5.2 What proof irrelevance does

For any `P : Prop` and `t1, t2 : P`, `def_eq(t1, t2)` succeeds. The contents of `t1` and `t2` are never compared structurally — only their types matter. This is the rule that lets distinct justifications for the same proposition be definitionally equal as propositional witnesses, while remaining observably distinct as Justification terms in `Type` (D39 v2 ergonomics).

### 5.3 Performance

The short-circuit fires *before* structural comparison, so it eliminates work rather than adding it. Cost is one type inference on each side. Type inference is already memoized in `CheckCtx::type_cache`.

### 5.4 Soundness

Proof irrelevance is a standard CIC feature; consistency proofs exist (Pédrot–Tabareau 2018 for the constructive case; Carneiro 2019 for Lean's flavor). The combination with our §7 singleton-elim restriction is exactly the Lean configuration.

---

## 6. Strong reduction: skip Prop-typed subterms

The reducer normally reduces subterms (for printing, conversion under binders, etc.) when invoked in "strong" mode. Under Prop, reducing the contents of a propositional subterm is *waste* — the result will be discarded by proof irrelevance anyway. nanoda_lib gates this with a `reduce_proofs` flag (`tc.rs:668-732`).

Eigenius adopts the same gate. The strong-reducer's recursion checks the type of each subterm; if the type is a proposition and `reduce_proofs` is false, the subterm is left as-is. WHNF reduction (the only mode that affects conversion) always reduces — the skip only affects pretty-printing and rare full-NF passes.

This is purely a performance optimization. Disabling it (always reducing) is correct, just slower.

---

## 7. Large elimination: the singleton-elim rule

### 7.1 The problem

Impredicative Prop without restrictions on elimination is unsound. The canonical counterexample: form `bad := ∀ (P : Prop), P : Prop`, then construct a function `Type → bad → Type` by case-splitting on a Prop inhabitant. Hurkens 1995 packages this into a closed-form paradox.

The standard fix (Lean, Coq, Agda) is to restrict *large elimination* out of Prop — i.e., to forbid Prop-typed terms from being used to produce Type-typed results, *except* in special "singleton" cases where no information is being smuggled across the Prop/Type boundary.

### 7.2 The singleton-elim rule

Lifted directly from nanoda_lib (`inductive.rs:845-903`). An inductive type `D : Prop` admits large elimination iff one of:

**Case A — zero constructors.** `D` has no constructors (e.g., `False`, `Asserts(iri)` per D39). There is no Prop inhabitant to eliminate, so any large eliminator is vacuously safe.

**Case B — exactly one constructor, with restrictions.** `D` has exactly one constructor `c : T → D`, and each argument of `c`:
  - is itself in `Prop`, **or**
  - **is** one of the *conclusion*'s indices (the argument variable itself occurs as an index of `D`, after the parameters), so the eliminator can reconstruct it from the type alone. An index that merely *mentions* the argument (e.g. `f(a)`) does not determine it and does not qualify — admitting it would let large elimination distinguish proofs that proof irrelevance makes definitionally equal.

Examples that pass: `Eq` (one ctor `refl`, the arguments `x` and `y` appear in the indices); `True` (one ctor `intro`, no arguments). Examples that fail: any `∃`-like singleton whose witness is in `Type` and not visible in the result type.

### 7.3 Where the check fires

When elaborating a recursor or `Match` whose major premise has type `D : Prop` and whose motive returns a non-propositional type, the kernel runs `large_elim_test(D)`. If it fails, the elaboration is rejected with a structured error pointing at the elimination site and the failing constructor.

For Match: same check, applied after motive synthesis.

### 7.4 Why this is the right amount of restriction

- It preserves the four key motivating singletons: equality (`Id`/`Eq`), `True`, `And` of two Props, `Or` of two Props (with caveats), and parameter-only Props like `Asserts`.
- It forbids the Hurkens construction (a multi-ctor Prop being case-analyzed into Type).
- It is decidable and local — no whole-program analysis required.

---

## 8. Projection and eta restrictions

### 8.1 Projection from Prop-typed structures

If `r : R` and `R : Prop`, then projections `r.fst`, `r.snd`, or named-field projections must yield Prop-typed components. Projecting a non-Prop field from a Prop-typed structure is rejected at infer time. This is automatic if §3.4 (Sigma in max universe) is enforced: a Prop-typed record's fields are all in Prop.

The harder case: if a Prop record's record-type-former is *declared* in Type (e.g., a Σ where the first component is in Type and the second in Prop, giving the whole thing `Sort(max(1, 0)) = Set`), projection follows the declared types straightforwardly. The Prop-projection restriction only fires when the *whole record* is in Prop.

### 8.2 Eta-expansion skip for Prop records

nanoda_lib's `iota_try_eta_struct` (`tc.rs:1019-1039`) skips structural eta when the type is Prop — proof irrelevance already handles equality. Eigenius adopts the same skip. For non-Prop records eta still applies.

---

## 9. Existing EigenTT term forms repositioned to Prop

The following forms currently live in `Set`/`Type`. After D46 they live in `Prop`:

| Form | Current home | New home | Reasoning |
|---|---|---|---|
| `Id(A, x, y)` | `Sort(level(A))` | `Sort(0) = Prop` | Standard CIC: equality is propositional. Two proofs of `x = y` are interchangeable. |
| `Refl(a)` | inhabitant of `Id` | inhabitant of `Id : Prop` | Unchanged at the term level; just lives in Prop. |
| `IdJ(...)` | eliminator over `Id` | eliminator over `Id : Prop` | Subject to singleton-elim. `Id` qualifies (Case B: one ctor, the indices `x` and `y` appear in the result type). Large elim continues to work. |
| `NativeDecide(c, v)` | reduces to `Refl : Id(..)` | unchanged behaviour, but the `Id` is now in Prop | Constraint decision is propositional. |
| `DecEq(A, x, y)` | reduces to `Refl : Id(..)` or neutral | same | Decidable equality witnesses equality, which is propositional. |

The `Asserts(iri)` declaration introduced in D39 will move to `Prop` in D39 v2 (D39's responsibility, not D46's). D46 only repositions the *existing* terms.

This means any existing test that relied on `Id`'s computational content (comparing two `Refl` witnesses via structural conversion) needs updating. Pre-production posture: the failing tests get rewritten.

---

## 10. The axiom-as-Resource framework

### 10.1 Default-admitted axioms

Two axioms are admitted at the kernel level:

**`propext` — propositional extensionality.**
```
propext : ∀ {P Q : Prop}, (P ↔ Q) → P = Q
```
Needed to give the chain a canonical-assertion identity: logically equivalent claims are propositionally equal, and (with proof irrelevance) share their inhabitant. Conservative over CIC.

**`Quot.sound` — quotient soundness.**
```
Quot.mk : Π {α : Type} (r : α → α → Prop), α → Quot r
Quot.lift : ...                              (* definitional *)
Quot.sound : ∀ {α : Type} {r : α → α → Prop} {a b : α}, r a b → Quot.mk r a = Quot.mk r b
```
Only `Quot.sound` is axiomatic; the rest is definitional. Needed for evidence normalization, chain consolidation deduplication, and standard mathematical quotient constructions. Conservative over CIC.

Both are exposed as built-in constants in the kernel's initial environment. The kernel's `delta` rule never unfolds them; conversion treats them as opaque.

### 10.2 Rejected at kernel level

`Classical.choice` is **not** admitted as a kernel constant.

```
Classical.choice : ∀ {α : Sort u}, Nonempty α → α   -- REJECTED
```

Reasoning: admitting choice (with `propext` and `Quot.sound` already in scope) gives excluded middle on all of Prop. Excluded middle lets the system derive `P` from `¬¬P` *without producing evidence*. For an institution like D39 whose entire purpose is to anchor every Prop-level belief to a chain-traceable justification, classical phantoms break the audit invariant.

Institutions that *need* classical reasoning (e.g., a future Mathlib-style institution) can admit `Classical.choice` as a per-institution axiom — see §10.4.

### 10.3 The `core:Axiom` Resource class

Axioms become first-class chain artifacts. Add to the core ontology:

```json
{
  "@id": "urn:eigenius:core:Axiom",
  "is_a": ["urn:eigenius:core:Class"],
  "label": "Axiom",
  "description": "A named axiom: a closed term whose type the kernel admits without checking the term itself. Treated opaquely by the delta rule and by conversion."
}

{
  "@id": "urn:eigenius:core:axiom_statement",
  "is_a": ["urn:eigenius:core:Property"],
  "domain": ["urn:eigenius:core:Axiom"],
  "data_type": "urn:eigenius:core:inductive",
  "class_types": ["urn:eigenius:core:EigenTTType"],
  "description": "The EigenTT type the axiom inhabits — typically a Prop. Encoded using the chain-mirrored EigenTT type fragment (D47)."
}

{
  "@id": "urn:eigenius:core:axiom_justification",
  "is_a": ["urn:eigenius:core:Property"],
  "domain": ["urn:eigenius:core:Axiom"],
  "data_type": "string",
  "description": "Free-form note: why this axiom is being admitted, what trust assumption it encodes."
}
```

Axioms are introduced by committing a `core:Axiom` resource to a layer. The `axiom_statement` value is a chain-mirrored EigenTT type per D47; on registration the kernel decodes it to a `kernel/src/nbe/term.rs::Exp`, type-checks the *statement* against the universe ladder, and then registers an opaque constant whose name is the resource IRI and whose type is the decoded value.

Conversion treats axiom-named constants the same way it treats `propext` and `Quot.sound`: opaque, no delta unfolding, equal only to themselves by symbol identity.

Voiding a layer that introduces an axiom removes the axiom from the kernel environment for any chain resolution that excludes that layer. Downstream proofs that depended on the axiom become unreachable.

### 10.4 Per-institution axioms

An institution that needs additional axioms ships them as `core:Axiom` resources in its institution layer. Examples:

- A "classical mathematics" institution layer admits `Classical.choice`, `funext`, and any further classical principles its corpus depends on. The axioms appear in the chain provenance of every derivation that imports the layer.
- D39 may admit a "rule extensionality" axiom (`∀ (R₁ R₂ : Rule), (∀ ctx, applies(R₁, ctx) ↔ applies(R₂, ctx)) → R₁ = R₂`) if its metatheory needs it. Such an axiom belongs in D39's layer, not the core.
- A future probabilistic-reasoning institution may admit Kolmogorov's axioms in propositional form.

The key property: **every Prop-level belief in a chain traces to either a constructive proof term or a citable, layer-scoped axiom**. There is no kernel-level escape hatch that lets axioms enter silently. This is what makes the audit invariant uniform.

### 10.5 Kernel mechanism

```rust
// In CheckCtx / EvalCtx:
struct AxiomEnv {
    axioms: BTreeMap<Iri, Val>,  // IRI → type of the axiom
}

// During environment construction:
fn build_axiom_env(layers: &[Arc<Layer>]) -> AxiomEnv {
    let mut env = AxiomEnv::default();
    for layer in layers {
        for resource in layer.resources_of_class("urn:eigenius:core:Axiom") {
            let statement = resource.get_expr("urn:eigenius:core:axiom_statement");
            let typ = check_type_in_universe(&statement)?;  // must be a well-formed type
            env.axioms.insert(resource.iri.clone(), eval(typ)?);
        }
    }
    env
}

// In def_eq:
// Two AxiomConst(iri1) and AxiomConst(iri2) are equal iff iri1 == iri2.
// AxiomConst never delta-reduces.

// In eval / WHNF:
// AxiomConst is normal; never unfolds.
```

The two default-admitted axioms (`propext`, `Quot.sound`) are registered identically — as if they were `core:Axiom` resources in the core ontology layer. There is no kernel-special-case for them.

---

## 11. Interaction with D39

D39 v2 (to be drafted separately) will:

1. Move `Asserts(iri)` from `Type` to `Prop`. It remains a parameter-only inductive with no constructors. Singleton-elim (§7) admits it under Case A (zero ctors).
2. Introduce `Validates : Justification → Prop → Prop` — the propositional relation "this justification supports that assertion." Proof irrelevance means two distinct proofs of `Validates(j, P)` are equal; the validation gate only needs to produce one.
3. Keep `Justification : Type` (the ADT of evidence terms). Justifications cannot live in Prop — proof irrelevance would collapse distinct evidence, defeating the point.
4. Register D39-specific axioms as `core:Axiom` resources in the D39 institution layer if needed. Most of the institution is constructive and needs no axioms beyond `propext` and `Quot.sound`.

The architectural relationship:

```
Justification : Type         -- evidence terms (chain artifacts)
       |
       | Validates(_, _)
       v
Asserts(iri) : Prop          -- propositional content of claims
       |
       | proof irrelevance
       v
proof of Asserts(iri)        -- collapsed to a single canonical inhabitant per Prop
```

The chain stores Justifications. The Prop layer is a derived view that says "this claim is warranted." Voiding evidence is a chain operation; it propagates to the Prop layer as "the constructive proof of this Asserts is no longer reachable from the current chain."

---

## 12. Implementation plan

Estimated effort: 3–6 weeks for a single experienced kernel engineer.

### 12.1 Phase A — AST and universe ladder (~1 week)

- Replace `Exp::Set` / `Exp::Type(n)` with `Exp::Sort(usize)` in `kernel/src/nbe/term.rs`.
- Replace `Val::Set` / `Val::Type(n)` with `Val::Sort(usize)` in `kernel/src/nbe/value.rs`.
- Update every match arm across `nbe/`, `program/`, `validation/`, `parser/`, `pretty/`. Use the compiler to drive the work; the rename is mechanical.
- Update Eigon-JSON serialization to use `"sort": n` instead of `"set"` / `"type": {"level": n}`.
- Update ESL surface to accept `Prop`, keep `Set` and `Type n` as sugar.
- Re-seed dev databases.

**Exit criterion:** workspace builds, all existing tests pass after the rename (no semantic change yet — Prop is just a syntactic name for Sort(0)).

### 12.2 Phase B — Pi formation and impredicativity (~3 days)

- In `check_type` for `Pi`: dispatch on the codomain's level. If codomain is in Prop, the whole Pi is in Prop; else max-rule.
- Add tests: `∀ (P : Prop), P → P : Prop`, `∀ (X : Type 17), X → X : Prop`, `∀ (n : Nat), Nat : Type 0` (still predicative when codomain isn't Prop).

### 12.3 Phase C — Proof irrelevance (~3 days)

- Add `is_proposition(T)`, `proof_irrel_eq(t1, t2)` helpers in `nbe/conversion.rs`.
- Wire `proof_irrel_eq` into `def_eq` after WHNF, before structural comparison.
- Add tests: two distinct `NativeDecide` reducing to `Refl` are convertible; two distinct `DecEq` outcomes are convertible; structurally different proofs of `Asserts(iri)` (once D39 v2 lands) are convertible.

### 12.4 Phase D — Singleton-elim (~1 week)

- Port `large_elim_test` from `nanoda_lib/src/inductive.rs:880-903`.
- Hook into recursor elaboration and `Match` elaboration: when major is in Prop and motive returns non-Prop, run the test.
- Structured error type `LargeElimRejected { type_iri, failing_ctor, reason }`.
- Add positive tests (Id, True, False, Asserts-style empties) and negative tests (multi-ctor Prop being eliminated to Type).

### 12.5 Phase E — Projection and eta restrictions (~2 days)

- In `infer` for projection: if the projected-from type is in Prop, the result must be in Prop.
- In `iota_try_eta_struct` analog: skip eta when the structure type is Prop.

**Status on landing**: this phase turned out to be a no-op for EigenTT. (a) §8.1's projection restriction is automatic under Phase B — a Sigma whose both components are in Prop lands in Prop (predicative Sigma rule), and a Sigma with any non-Prop component lands in Set or higher, so projecting "a non-Prop field from a Prop-typed structure" cannot be constructed in the first place. The existing `predicative_sigma_in_prop_requires_both_components_in_prop` test covers this at the construction site. (b) §8.2's eta skip is moot because EigenTT's `eq_nf` is purely readback-based and has no structural eta-expansion to skip.

### 12.6 Phase F — Strong-reduction skip (~1 day)

- Add `reduce_proofs: bool` flag to the strong-reducer's recursion. Default false.
- Skip reduction of Prop-typed subterms when false.
- Verify with a pretty-printer test on a term with a large Prop-typed argument.

**Status on landing**: deferred — tracked as [eigenius#67](https://github.com/eigenius/eigenius/issues/67). EigenTT's `readback_val` already reduces all subterms structurally; introducing a `reduce_proofs` flag requires threading type information through readback (which currently doesn't carry it). Per the original §6 framing this is "purely a performance optimization" — disabling it (always reducing) is correct, just slower. With no observed pretty-printing or full-NF performance issues today, we defer the optimization until a concrete need surfaces (likely when D39 v2 commits substantial JustificationTerm proof payloads that get pretty-printed).

### 12.7 Phase G — Reposition Id/Refl/IdJ to Prop (~2 days)

- Change `Id`'s inferred sort from `level(A)` to `Sort(0)`.
- Verify `IdJ` still elaborates (Id passes singleton-elim Case B).
- Update existing tests that compared `Refl` terms structurally — they now succeed by proof irrelevance.

### 12.8 Phase H — Axiom-as-Resource framework (~1 week, depends on D47)

**Prerequisite:** D47 (chain-mirrored EigenTT type fragment) landed, so `core:EigenTTType` exists and `Exp ↔ EigenTTType` codec is available.

- Add `core:Axiom`, `core:axiom_statement`, `core:axiom_justification` to `ontologies/core/core-ontology.json`. Triggers `seed_manifest_v1` bump.
- Implement `AxiomEnv` + `build_axiom_env` in `kernel/src/program/`. Decodes `axiom_statement` via the D47 codec, type-checks the resulting `Exp`, registers the IRI → type binding.
- Register `propext` and `Quot.sound` as core-ontology `core:Axiom` resources (no special-case in kernel code). Their `axiom_statement` values are encoded EigenTTType trees.
- Treat `AxiomConst(iri)` as opaque in eval, WHNF, and conversion.
- Add tests: an axiom-typed term type-checks against its statement; two distinct axiom names with the same statement are *not* convertible (axioms are nominal); voiding an axiom-introducing layer makes the axiom unreachable in the resolved environment.

### 12.9 Phase I — Documentation and migration completion (~2 days)

- Update [docs/design/d1-eigon-serialization-format.md](d1-eigon-serialization-format.md) for the `Sort` serialization shape.
- Update CLAUDE.md if any conventions shift.
- Mark D46 status: Implemented.

### 12.10 Sequencing note

Phases A–G can ship as a single PR or two (A together, then B–G). They depend only on D9/D18/D19 — already in place.

Phase H depends on D47 (chain-mirrored EigenTT type fragment, ~1 week of separate work). D47 can be drafted and implemented in parallel with D46 Phases A–G since the two tracks don't overlap.

D39 v2 implementation begins after D46 (all phases including H) and D47 land.

---

## 13. Risk areas

### 13.1 Singleton-elim correctness

The "non-recursive ctor args appear in the conclusion" check is the subtle part. Getting it wrong gives unsoundness (Hurkens-style derivation). Mitigation: port nanoda's implementation line-by-line, including its test fixtures (`MyTypeLarge` ✓, `MyTypeSmall` ✗). Add a CI test that asserts a known-bad multi-ctor Prop is *rejected* — not just that good ones are accepted.

### 13.2 The Sort(1) cumulativity boundary

Whether `Prop ⊆ Set` is "real" cumulativity (Lean 4 style) or only nominal matters for some edge cases (e.g., can a function expecting `A : Set` accept `A := Asserts(iri) : Prop`?). The pragmatic choice is yes (it's just universe coercion), and this is what §3.2 specifies. If a soundness issue appears here, the fallback is to make Prop non-cumulative and require explicit coercions — a small change.

### 13.3 Performance regression from proof_irrel_eq

The short-circuit infers types on both sides of every conversion check. For terms not in Prop, this is one cache hit each. For deeply-nested Prop-typed comparisons, the savings dominate. Profile during Phase C; if hot, add a fast-path that skips `proof_irrel_eq` when both sides are obviously non-Prop (e.g., both are constructor applications of a non-Prop inductive).

### 13.4 Existing test fallout from Phase G

Tests that constructed two `Refl` terms and asserted they were *not* convertible will now fail (correctly — they should be convertible). Tests that introspected the structure of `Id` proofs will need restructuring. Estimated ~10-20 affected tests across the test suite. Rewriting them is mechanical.

### 13.5 Axiom-as-Resource churn on the core ontology

Adding `core:Axiom` to the core ontology bumps `seed_manifest_v1`. Pre-production posture handles this (drop and re-seed), but it means every dev environment must re-seed once D46 lands. Note in the changelog.

### 13.6 Future incompatibility with universe polymorphism

Concrete `usize` sort levels work for now. If we later want universe-polymorphic definitions, we'll need to extend `Sort` to carry a level expression rather than a literal. This is a future-D-doc concern; the AST migration to support it is straightforward.

---

## 14. Decisions made during this design

- **§3 = Option A (unified Sort ladder).** Single rule for sort typing; matches CIC tradition; cheaper long-term than maintaining cross-sort case logic forever.
- **§4 = Option B (impredicative Prop).** D39's rule schemas want to be propositions; standard practice; bounded kernel cost.
- **§10 = Option D (propext + Quot.sound + axiom-as-Resource; reject kernel-level Classical.choice).** Preserves D39's audit invariant; lets institutions opt into classical reasoning per-institution; chain-layered axiom provenance.
- **§11 (Migration) = dropped.** Pre-production posture; drop and re-seed.

These four decisions were settled during the design discussion preceding this draft. Reopening them requires explicit pushback; the implementation plan in §12 assumes all four.

---

## 15. References

### 15.1 External

- nanoda_lib (`references/nanoda_lib/src/`):
  - `tc.rs` — `def_eq`, `proof_irrel_eq` (`1301-1309`), `is_proposition` (`1291`), `is_sort_zero` (`1284`), `iota_try_eta_struct` (`1019-1039`), `strong_reduce` with `reduce_proofs` gate (`668-732`), `infer_proj` Prop restriction (`436-483`).
  - `inductive.rs` — `large_elim_test` (`880-903`), `large_elim_test_aux` (`845-878`).
  - `expr.rs` — `Sort(Level)` variant (`50`), `prop()` = `mk_sort(zero)` (`720`).
  - License: Apache-2.0 (compatible with Eigenius).
- Pédrot, P.-M., Tabareau, N. (2018). "Failure is Not an Option: An Exceptional Type Theory." On proof irrelevance in CIC variants.
- Carneiro, M. (2019). "The Type Theory of Lean." Reference for Lean 4's impredicative-Prop discipline.
- Hurkens, A. J. C. (1995). "A Simplification of Girard's Paradox." The canonical inconsistency argument against unrestricted impredicative Prop.
- Artemov, S. (2008). "The Logic of Justifications." Relevant to §11 (D39 interaction); D39 is the doc that actually depends on this.

### 15.2 Internal

- [D9 — NbE and type extensions](d9-nbe-unification-and-type-extensions.md)
- [D18 — Ontology-as-Types Resolution](d18-ontology-as-types-resolution.md)
- [D19 — Inductive Types](d19-inductive-types.md) — singleton-elim depends on the inductive-decl machinery defined here.
- [D32 — Chain-mirrored EigenTT Inductives](d32-chain-mirrored-mini-tt-inductives.md) — D47 is the same pattern applied to the type-level EigenTT fragment.
- [D47 — Chain-mirrored EigenTT type fragment](d47-chain-mirrored-mini-tt-type-fragment.md) — prerequisite for §10's axiom-statement encoding.
- [D39 — Justification Logic](d39-justification-logic.md) — primary consumer; D39 v2 redraft follows D46.
- [implementation-plan.md](implementation-plan.md) — D46 lands as a new phase between Phase 11b (inductives, done) and the D39 implementation phase (TBD).
