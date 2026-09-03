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
claim.reflection:canonical_proposition        (D47-encoded eigentt:Term)
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
externalizer and whatever produced the export.

**Decided `2026-08-22`: the mirror is the authority, and the mangling lives inside it.**

D74 cannot invent names. The export was compiled against D30's generated package, so a name the
generator did not emit cannot appear in the export, and inventing one would turn a naming
disagreement into a `def_eq` mismatch — a worse diagnostic at a later stage. The map therefore
reproduces the generator's, and a class the mirror does not cover is **refused at externalization**
with a diagnostic naming the class and the mirror.

### 3.3.1 D30's current naming is not injective, and that has to be fixed first

D30 §7.1 emits `structure <short_name>` flat inside `namespace EigeniusFFI`, so the Lean name of a
class is its `core:short_name`. §11.1 checks that class short_names are valid Lean identifiers and
that *property* short_names are unique within a class's transitive field set — **it does not check
that CLASS short_names are unique across the closure.**

Measured over every ontology in the repo (`2026-08-22`): **948 class short_names, 8 colliding.**

| short_name | IRIs |
|---|---|
| `Person` | `reflection:Person`, `schema_org:Person` |
| `Organization` | `reflection:Organization`, `schema_org:Organization` |
| `Axiom` | `eigentt:Axiom`, `objective:Axiom` |
| `Observation` | `core:Observation`, `enc:Observation` |
| `DecisionPoint` | `enc:DecisionPoint`, `objective:DecisionPoint` |
| `CutItem` | `enc:CutItem`, `objective:CutItem` |
| `Hypothesis` | `enc:Hypothesis`, `objective:Hypothesis` |
| `Map` | `program:Map`, `schema_org:Map` |

`reflection:Person` is not incidental — D72 made it a `reflection:Agent` subclass, so it is what
`declared_by` resolves to on any attributed claim.

A closure containing both members of a pair emits two `structure Person` declarations in one
namespace. That is a D30 defect independent of this document; it becomes load-bearing here because
a non-injective map means externalization can name the wrong class, which is the failure mode §5
calls proving the wrong theorem soundly.

**The mangling: qualify by the IRI's namespace segment.** `urn:eigenius:reflection:Person` becomes
`EigeniusFFI.reflection.Person`; `urn:schema_org:Person` becomes `EigeniusFFI.schema_org.Person`.
Lean namespaces are dot-separated, so this is idiomatic rather than an escape encoding, it stays
readable in a proof, and it is injective wherever IRIs are — which is everywhere, by construction.

This is a change to **D30's** emission, not only to D74's reading of it: the generator must place
each structure in its namespace-qualified path, and §11.1 should gain the class-level uniqueness
check that makes the property-level one meaningful. Tracked as eigenius#208, and a **prerequisite**
for implementing this document.

**Decided `2026-09-02`: the full `urn:` path minus the local name.**
`urn:eigenius:reflection:Person` becomes `EigeniusFFI.eigenius.reflection.Person`;
`urn:schema_org:Person` becomes `EigeniusFFI.schema_org.Person`. The table above is also satisfied
by the last component alone, and that is the whole of its case — it is injective by measurement,
and re-collides the first time two namespaces share a final segment. See §6.1.

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

All four are settled. §6.1 was decided with the document; §6.2-§6.4 on `2026-09-02`, when a
collaboration with Nada Amin's group made the Lean institution load-bearing and the
"no live consumer" ground for deferring #159 expired.

1. ~~**§3.3 — the IRI → `Name` map.**~~ **Decided `2026-08-22`** — the mirror, with
   namespace-qualified mangling inside it (§3.3). It carries a prerequisite: D30's flat
   `EigeniusFFI.<short_name>` is not injective (8 live collisions, §3.3.1) and must be qualified
   before externalization can rely on it.

   ~~The remaining latitude — full `urn:` path minus the local name, or its last component
   only.~~ **Decided `2026-09-02` — the full path minus the local name.**
   `urn:eigenius:reflection:Person` becomes `EigeniusFFI.eigenius.reflection.Person`.
   The last component satisfies today's eight collisions and reads better, and that is the whole
   of its case; it is injective by measurement, not by construction, and re-collides the first
   time two namespaces share a final segment. The property being bought here is injectivity, and
   buying it from the IRI — which is injective by construction, everywhere — costs only name
   length in proof text.

2. ~~**Where does `def_eq` run?**~~ **Decided `2026-09-02` — inside the existing `check_proof`
   call.** One arena, one parse: the export is parsed there already, so externalizing the
   expected statement into the same `TcCtx` is the cheap direction, and both sides must share an
   arena for `def_eq` to be callable at all. The cost is `check_proof`'s signature, which gains
   the expected statement as an option. A second entry point would keep that contract and pay a
   second parse of the same bytes for it, which is the wrong thing to protect.

3. ~~**What happens when the fields are absent?**~~ **Decided `2026-09-02` — `claim_iri` becomes
   `requires`, and the reseed is taken now.** With the statement manufactured from the claim,
   `lean:proposition` and `mirror_iri` stop being load-bearing, but without `claim_iri` there is
   no claim to externalize and the check has nothing to be total over. The alternative — leaving
   it recommended and having the institution reject rather than skip — costs no migration and
   changes the same behaviour, at the price of a schema that no longer describes what the
   institution accepts; §9.11's whole point is that the declaration is the contract.

   Taken now rather than later because the edit moves the `lean-institution` layer hash and every
   layer id below it, and it bundles with #208's ontology work, which moves the manifest anyway.
   One reseed covers both. Deferring means paying it once collaborators hold chains committed
   against the current ids.

4. ~~**Is `def_eq` the right strictness?**~~ **Decided `2026-09-02` — yes, and deliberately.**
   It admits δ- and η-equal statements, so the check accepts statements that are not
   syntactically the claim. That is the correct latitude: a prover states a theorem in whatever
   form the proof made convenient, and unfolding a definition does not change what was proved.
   Syntactic equality would reject correct proofs for cosmetic reasons and push authors toward
   restating claims to match their proofs, which inverts the dependency this document exists to
   establish. The residue in §5 is unchanged and is the real bound on what `def_eq` buys: it
   compares the externalized statement, so the externalization's faithfulness stays load-bearing.

---

## 7. Prior state, for the record

The premise that made D49 §7 choose the inverse is gone. When the Lean institution was built,
`eigentt:Term` and the impredicative `Prop` universe did not exist, so Lean's own proposition
was the only representation available and recovering it was the only option. D46 and D47 removed
that constraint. The design was right for its time; this is the same shape of correction D73 records
for D39 §8.
