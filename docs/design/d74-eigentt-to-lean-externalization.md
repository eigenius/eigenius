# D74 — EigenTT → Lean externalization

**Status:** design note, precedes code.
**Issue:** [#159](https://github.com/eigenius/eigenius/issues/159) — *nothing binds a Lean proof to the claim it is supposed to prove*.
**Related:** [D28](d28-lean-4-as-institution.md) (Lean as verification institution) · [D30](d30-eigon-to-lean-faithful-translation.md) (Eigon classes → Lean **source**) · [D40](d40-chain-mirrored-lean-expressions.md) (Lean **bytes** → chain values) · [D46](d46-prop-universe-and-proof-irrelevance.md) / [D47](d47-chain-mirrored-eigentt-type-fragment.md) (EigenTT and its codec) · [D73](d73-justification-logic-witnesses-and-traces.md) §4 (Verified, and what it is relative to)

---

## 1. The gap this closes

`check_proof(bytes, target_name, permitted_axioms)` establishes two things: every declaration in the
export type-checks, and a declaration by that **name** exists. It never compares the named theorem's
*statement* to anything. So `Verdict::Holds` today means *"some theorem called `T` type-checks"*, and
a caller reading it as *"this proof proves this claim"* is reading something the check does not
establish.

Nothing supplies a statement to compare against. This document specifies the thing that does.

**Externalize, do not invert.** D49 §7 specified recovering the EigenTT proposition by inverting
D30's translation; #159 supersedes that with the forward direction: manufacture the Lean statement
*from the claim*, and check the returned proof term against it. The proof is bound to the claim
because the goal was built from the claim.

Forward translation is total on the domain that matters — every EigenTT proposition is expressible
by construction. The inverse is partial over Lean's much larger language, and D49 §7 concedes it.

### 1.1 What this is NOT

Two existing Lean-facing translations sit either side of this one and neither does its job:

| doc | direction | object | output |
|---|---|---|---|
| **D30** | Eigon → Lean | class *structure* | Lean **source** (a Lake package of `structure`s) |
| **D40** | Lean → Eigon | a theorem's type | chain `lean:LeanExpr` **values** |
| **D74** | Eigon → Lean | an EigenTT **`Prop` term** | an in-memory nanoda **`Expr`** |

#159 assumed this was "D30's direction, and the machinery exists." It is not: D30 translates
*classes to structures*, this translates *propositions to statements*. `grep 'Exp::'` across the
Lean crates returns nothing — no EigenTT term reaches them today in any form.

**No Lean source is emitted and no Lake build runs.** The export is already parsed in-process by
nanoda (`checker.rs`, `chain_mirror.rs`); this builds an `Expr` in the same arena and compares. That
is the whole reason the design is cheap: it is AST → AST, not AST → text → compiler.

---

## 2. Shape

```
claim.reflection:canonical_proposition        (D47-encoded eigentt:TypeExpr)
        │  decode_type                        (kernel, existing)
        ▼
  EigenTT Exp                                 the kernel's AST
        │  externalize          ◄── THIS DOCUMENT
        ▼
  nanoda ExprPtr  ────────────┐
                              │  TypeChecker::def_eq
  target's type from the       │
  parsed export  ─────────────┘
        ▲
        │  nanoda parse (existing)
  lean4export bytes
```

Both sides are `ExprPtr` in one `TcCtx` arena, so the comparison is nanoda's own **definitional
equality** (`tc.rs::def_eq`) rather than structural equality on a serialized form. That matters:
`α`-renaming, `δ`-unfolding of definitions, and `η` are handled by the checker we already trust,
not re-implemented here.

---

## 3. The two ASTs

**Target** (`nanoda_lib::expr::Expr`): `Var{dbj_idx}`, `Sort(Level)`, `Const(Name, Levels)`, `App`,
`Pi`, `Lambda`, `Let`, `Proj`, `StringLit`, `NatLit`, `Local`.
Construction is public and complete for our needs: `mk_var`, `mk_sort`, `mk_const`, `mk_app`,
`mk_pi`, `mk_lambda`, `mk_string_lit_quick`, `mk_nat_lit_quick`, plus `foldl_apps` and `abstr_pi`.

**Source** (`eigenius_kernel::nbe::term::Exp`): 40-odd variants. Most are irrelevant to propositions.

Three structural mismatches drive the design.

### 3.1 Names → de Bruijn

EigenTT `Var(Name)` is **named**; nanoda `Var{dbj_idx}` is **de Bruijn**. The translator threads a
binder stack and converts on the way down: a `Var(x)` resolves to the distance from its binder.

A name not in the stack is **not** a free variable to invent — it is a bug upstream, because
committed propositions are closed (the same invariant D40 §3.3 relies on, where it surfaces as
`UnexpectedLocal`). Refuse it.

`Local` is never produced. It exists in nanoda for type-checking under binders; we build closed
terms.

### 3.2 Universes line up exactly

D46 fixes `Sort(0) = Prop`, `Sort(1) = Set`, `Sort(n+1) = Type n`. Lean fixes `Sort 0 = Prop`,
`Sort 1 = Type 0`. **The indices agree**, so `Exp::Sort(n)` ↦ `mk_sort(Level::from(n))` with no
shift, and a proposition at `Sort(0)` lands in Lean's `Prop` — the impredicative universe, which is
what makes proof irrelevance available on the Lean side.

`Level::{Max, IMax, Param}` are not produced: EigenTT universes are concrete naturals, so every
level is a literal `Succ^n(Zero)`. Universe *polymorphism* is out of the fragment (§5), which is the
same restriction eigenius#136 left standing in `spec_poly`'s fixed `Type 1` binder.

### 3.3 Chain constants → Lean names

`EigonClass(iri)`, `EigonAxiom(iri)` and `InductiveType(decl, args)` are references to chain
resources. Each becomes `Const(Name, levels)` — and **which `Name`** is the one genuinely open
decision in this document.

The requirement is a *total, injective, and stable* map from IRI to Lean `Name`, agreed by both the
externalizer and whatever produced the export. Two candidate sources:

1. **D30's mirror namespace.** The EigeniusFFI package already names mirrored classes, and D28's
   correspondence check already walks propositions looking for names in that namespace. Reusing it
   keeps one naming authority.
2. **A direct IRI mangling** (`urn:eigenius:demo:onco:RequiresActivity` → `Eigenius.demo.onco.RequiresActivity`),
   independent of whether D30's generator has run.

**These are not interchangeable.** (1) ties externalization to the mirror generator having produced
a package covering every class the proposition mentions, and inherits D30's `UnrepresentableClass`
failures. (2) is total over IRIs but names things the Lean environment may not define — which
surfaces as a `def_eq` failure against the export rather than as a translation error, i.e. a worse
diagnostic at a later stage.

**Recommendation: (1), with (2) as the mangling *within* it.** Mirror-defined names when D30 covers
the class, and refuse — with a diagnostic naming the class and the mirror — when it does not. The
export was produced against the mirror; a name the mirror does not define cannot appear in it, so
refusing early is strictly more informative than proving a `def_eq` mismatch late.

This decision must be pinned before implementation, and it is the one thing here that another
component's behaviour depends on.

---

## 4. The fragment

**v1 translates propositions, not programs.** The domain is the terms that legitimately appear in
`reflection:canonical_proposition` — which Rule 21 already constrains to inhabit `Sort(0)`.

| EigenTT | Lean | note |
|---|---|---|
| `Sort(n)` | `Sort n` | §3.2, no shift |
| `Pi(p, a, b)` | `Pi` | binder name carried for readability only |
| `Arrow(a, b)` | `Pi` with an unused binder | `Arrow` is non-dependent `Pi` |
| `App(f, x)` | `App` | spines fold with `foldl_apps` |
| `Lam(p, e)` | `Lambda` | appears inside propositions as a motive |
| `Var(x)` | `Var{idx}` | §3.1 |
| `EigonClass(iri)` | `Const(name, [])` | §3.3 |
| `EigonAxiom(iri)` | `Const(name, [])` | §3.3 |
| `InductiveType(decl, args)` | `Const(name, [])` applied to args | §3.3 |
| `InductiveCtor(decl, c, args)` | `Const(name.c, [])` applied to args | |
| `EigonPrimitive(String/Integer/Boolean)` | `Const(String/Int/Bool)` | Lean's own |
| `LitString(s)` | `StringLit` | |
| `LitInt(n)`, `n ≥ 0` | `NatLit` | **negative refused**: `NatLit` is a `BigUint` |
| `LitBool(b)` | `Const(Bool.true / Bool.false)` | |
| `Id(a, x, y)` / `Refl(x)` | `Eq` / `rfl` | |
| `One` / `Unit` | `PUnit` / `PUnit.unit` | |

**Refused in v1**, each with a typed error naming the construct:

- `Sig` / `Pair` / `Fst` / `Snd` — Lean's `Sigma` is library, not primitive; admitting it means
  pinning which `Sigma` and is a decision of its own.
- `Codata` / `CoRecord` / `Observe` — coinduction has no v1 Lean image.
- `Map` / `Reduce` / `NativeDecide` / `DecEq` — computation, not proposition.
- `Template` / `PropAccess` / `Construct` / `EigonResource` — resource-level constructs; a
  proposition mentioning a *resource value* rather than its class is outside the fragment.
- `LitFloat` — Lean has no float literal; a proposition over reals needs a decision about `Float`
  vs `Real` that v1 does not make.
- `SizeSort` / sized binders — D46's sized fragment has no Lean counterpart.
- `Data` / `Case` / `Dec` / `Ann` — surface forms the codec does not emit into proposition slots.

Refusal is **typed and total**: an `ExternalizeError` naming the variant and the sub-term, never a
silent approximation. A proposition outside the fragment must fail loudly at externalization, since
the alternative — translating "close enough" — proves a different theorem soundly.

---

## 5. What this makes true, and what it does not

With externalization in place, D28's check becomes:

1. nanoda checks the export — *every declaration is well-typed* (unchanged)
2. the target's type is compared by `def_eq` against the externalized claim — **new**
3. `IsVerifiedAs(claim_iri, P)` is admitted with `P` the claim's **own** proposition

`reasoning:VerifiedPropositionView` and the comorphism reify step both disappear: the witness keys
on the claim's proposition hash, not on a reified view. It composes with eigenius#200 — the
`VerificationTrace` is the artifact (`proof_system: lean4`, `proof_term`: the export) and the
witness is its projection, so D39 §5's *"witness and trace are two projections of one validator
event"* holds uniformly across all four grounding families.

**The trust surface moves; it does not vanish.** D73 §4.2 says factivity for Verified is relative to
trusting the prover *and* the translation. This halves the second half — one forward translation
instead of a forward and an inverse — and relocates the rest **here**. If this document's mapping is
wrong, the system proves the wrong theorem soundly. That is a smaller and better-located surface
than an inverse over Lean's full language, and it is why the fragment is small, the refusals are
typed, and §3.3 is called out as a decision rather than an implementation detail.

D74 therefore joins nanoda_lib and D30 in the TCB.

### 5.1 Non-goals

- **Proof *search*.** Lean supplies the proof; this supplies the goal.
- **Translating proof terms.** Only statements cross. The proof stays bytes nanoda checks.
- **Round-tripping.** D40 is the other direction and stays as it is; nothing here needs the inverse
  to exist, which is the point.

---

## 6. Open questions

1. **§3.3 — the IRI → `Name` map.** Recommended above, not settled. Everything else here is
   mechanical once it is fixed.
2. **Where does `def_eq` run?** Inside the existing `check_proof` call (one arena, one parse) or as
   a second entry point beside it. The first is cheaper and keeps the export parsed once; the second
   keeps `check_proof`'s current contract untouched.
3. **What happens when the fields are absent?** #159's original text raises this and it survives the
   redesign in altered form: with the statement manufactured from the claim, `lean:proposition` and
   `mirror_iri` stop being load-bearing, but `claim_iri` becomes *required* — without it there is no
   claim to externalize. Promoting it is an ontology edit to `lean-institution`, hence a manifest
   move and a reseed.
4. **Is `def_eq` the right strictness?** It admits δ- and η-equal statements. That is almost
   certainly right — a prover may state the theorem in an unfolded form — but it means the check
   accepts statements that are not syntactically the claim, and that should be a deliberate choice
   rather than a default inherited from the API.

---

## 7. Prior state, for the record

The premise that made D49 §7 choose the inverse is gone. When the Lean institution was built,
`eigentt:TypeExpr` and the impredicative `Prop` universe did not exist, so Lean's own proposition
was the only representation available and recovering it was the only option. D46 and D47 removed
that constraint. The design was right for its time; this is the same shape of correction D73 records
for D39 §8.
