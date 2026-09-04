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

`Const(iri, levels)`, `EigonClass(iri)` and `EigonAxiom(iri)` are references to chain resources.
Each becomes `Const(Name, levels)` — and **which `Name`** is the one genuinely open decision in
this document. (`Const` replaced `InductiveType(decl, args)` in D76 Phase B1; see §4.)

The requirement is a *total, injective, and stable* map from IRI to Lean `Name`, agreed by both the
externalizer and whatever produced the export.

**Decided `2026-08-22`: the mirror is the authority, and the mangling lives inside it.**

D74 cannot invent names. The export was compiled against D30's generated package, so a name the
generator did not emit cannot appear in the export, and inventing one would turn a naming
disagreement into a `def_eq` mismatch — a worse diagnostic at a later stage. The map therefore
reproduces the generator's, and a class the mirror does not cover is **refused at externalization**
with a diagnostic naming the class and the mirror.

**Amended `2026-09-02`: the mangling is the authority; the mirror is not consulted.** §6.3.1
deletes `mirror_iri`, so the institution has no handle on a mirror to look a name up in. It does
not need one. Once #208 makes the name a namespace-qualified function of the IRI (§3.3.1), the
generator's map and the externalizer's are the *same total function*, and they agree by
construction rather than by lookup — the reason for consulting the mirror was that the map was
a table only the generator held.

What is lost is the early coverage refusal: a class the mirror does not cover now surfaces as a
`def_eq` failure on a constant the export does not declare, rather than a diagnostic naming the
class and the mirror. That is the same trade §6.3.1 takes for check 2a, for the same reason.

**Implementation constraint this carries.** The function must exist once and be called by both
sides — the D30 generator when it emits, and the externalizer when it reads — the way
`ctor_classes::class_iri` is the single authority for D85's derived class names. Two
implementations of "the same" mangling is precisely the naming disagreement this section exists
to prevent, and it would surface as a `def_eq` mismatch with no indication that naming was the
cause.

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

**In the emitted Lean: one `namespace EigeniusFFI` block, dotted declaration names.** The
generator writes `structure eigenius.reflection.Person where` inside the single block it already
opens, rather than nesting a block per namespace path. Dotted declaration names preserve the
topological order a structure's field references require — grouping by namespace would fight it —
and keep the `export EigeniusLeanCommon (…)` helper resolution the codec emitter relies on,
since every declaration stays lexically inside `EigeniusFFI`. Codec functions qualify the same
way (`eigenius.reflection.decodePerson`), because `decodePerson` flat in `EigeniusFFI` collides
exactly as `structure Person` does.

---

## 4. The fragment

**v1 translates propositions, not programs.** The domain is the terms that legitimately appear
in `reflection:canonical_proposition` — which Rule 21 already constrains to inhabit `Sort(0)`.

**Restated `2026-09-03` against the current AST.** The original table was written before D76
Phase B1 and eigenius#218 and had drifted: it gave a row to `InductiveType(decl, args)`, which
no longer exists, refused four constructs that had been deleted (`Codata`, `CoRecord`,
`Observe`, `SizeSort`), and said nothing at all about nine that do exist. A table with holes
reads as permission, which is the opposite of what this section is for.

`kernel/src/nbe/term.rs` declares **43** `Exp` variants. All 43 are classified below, and
`crates/eigenius-lean/src/externalize.rs` matches on them exhaustively — so a variant added to
the kernel breaks that file rather than falling through to a translation nobody chose. **The
code is the authority for totality; this table is the authority for the reasons.**

### 4.0 The denominator that matters: what decoding can produce

§4.1–§4.6 classify every `Exp` variant, which is what makes the implementation's exhaustive
`match` meaningful. But a proposition reaches externalization only by being **decoded from
`reflection:canonical_proposition`**, so the set that can actually arrive is what `decode_type`
emits — not the whole AST.

The D47 codec's `eigentt:Term` declares **21** constructors:

```
Sort  Var  ConstRef  App  Ann  Pi  Sig  Lam  One  Id  UnitVal
LitInt  LitString  LitFloat  LitBool  CtorApp  Pair  Fst  Snd  Record  Refine
```

`Arrow` and `Times` have no constructor of their own — they encode as `Pi` / `Sig` with a `Unit`
binder — and `ConstRef` decodes to `Const`, `EigonClass` or `EigonAxiom` depending on what it
resolves to. Read off `decode_type`, the **23 variants decoding can produce** are:

| | |
|---|---|
| **translated** | `Sort` `Var` `Const` `EigonClass` `EigonAxiom` `EigonPrimitive` `App` `Pi` `Sig` `Fst` `Snd` `One` `Unit` `Id` `LitInt` `LitString` `LitBool` `LitFloat` — 18 |
| **refused** | `Ann` `Lam` `InductiveCtor` `Pair` `Refine` — 5 |

Everything else the fragment classifies — `Map`, `Reduce`, `Construct`, `Template`, `Match`,
`InstitutionInvoke`, `InductiveRec`, `Case`, `Data`, `Dec`, `IdJ`, `NativeDecide`, `DecEq`,
`EigonResource`, `PropAccess`, `Record`, `Refl`, `Con`, `Arrow`, `Times` — cannot be produced by
decoding. Their refusals are correctness insurance against a future codec, not present limits.
(`Record` is a wire constructor but `decode_type` never yields one; §4.7.)

**What is actually committed is narrower still.** Measured `2026-09-03` across every `.esl` and
`.json` in the tree, committed `eigentt:Term` values use **three** constructors: `ConstRef` (282
occurrences), `Sort` (12), `Var` (6). All three translate, and no committed proposition contains
a `Lam`, a `Pair`, a `Record` or a `Refine`.

Three constructors is a thin corpus, so "everything committed externalizes" says less than it
sounds. The parser's own output is what will exercise the rest.

### 4.1 Translated (22)

| EigenTT | Lean | note |
|---|---|---|
| `Sort(l)` | `Sort l` | §3.2, no shift. `l` maps structurally: `Zero`/`Succ`/`Max`/`IMax`/`Param` |
| `Pi(p, a, b)` | `Pi` | binder name carried for readability only |
| `Arrow(a, b)` | `Pi` with an unused binder | `Arrow` is non-dependent `Pi` |
| `App(f, x)` | `App` | |
| `Var(x)` | `Var{idx}` | §3.1 — named to de Bruijn, against the binder stack |
| `Const(iri, levels)` | `Const(name, levels)` | §3.3. **Replaced `InductiveType(decl, args)`** in D76 Phase B1, and is now how every chain-resident reference translates |
| `EigonClass(iri)` | `Const(name, [])` | §3.3 |
| `EigonAxiom(iri)` | `Const(name, [])` | §3.3 |
| `EigonPrimitive(String/Integer/Boolean)` | `Const(String/Int/Bool)` | Lean's own; `Float` and `Json` are refused — see §4.2 |
| `LitString(s)` | `StringLit` | |
| `LitInt(n)`, `n ≥ 0` | `NatLit` | **negative refused**: `NatLit` holds a `BigUint`, and synthesising `Int.negSucc` would be a different term than the one authored |
| `LitBool(b)` | `Const(Bool.true / Bool.false)` | |
| `Id(a, x, y)` | `Eq` | |
| `Refl(x)` | `rfl` | |
| `Sig(p, A, B)` | `Subtype A (fun p : A => B)` | §4.5 — **not** `Sigma`; `Exists` cannot work |
| `Times(A, B)` | `Subtype A (fun _ : A => B)` | a non-dependent `Sig` |
| `Fst(e)` | `Subtype.val α p e` | §4.6 — implicits inferred, at any depth |
| `Snd(e)` | `Subtype.property α p e` | §4.6 — same |
| `LitFloat(v)` | `OfScientific` spine, `Neg.neg` if negative | §4.8 — exact via shortest round-trip; NaN / ±∞ refused |
| `EigonPrimitive(Float)` | `Const(Float)` | §4.8 — IEEE 754/854 binary64 |
| `One` | `PUnit` | |
| `Unit` | `PUnit.unit` | |

### 4.2 Refused (21)

Refusal is **typed and total**: an `ExternalizeError` naming the variant and the sub-term, never
a silent approximation. A proposition outside the fragment must fail loudly, since the
alternative — translating "close enough" — proves a different theorem soundly.

| group | variants | why |
|---|---|---|
| **Σ intro** | `Pair` | `Subtype.mk`'s predicate cannot be inferred from `p a` without higher-order unification — the elaborator's job, which a checker does not offer |
| **Records (D78)** | `Record`, `Refine` | an anonymous structural type, and one carrying the classes it satisfies. Refused for different reasons and with different prospects — see §4.7 |
| **Computation** | `Map`, `Reduce`, `NativeDecide`, `DecEq` | computation, not proposition |
| **Resource-level** | `Template`, `Construct`, `EigonResource`, `PropAccess` | a proposition mentioning a resource *value* rather than its class is outside the fragment. `PropAccess` projects a field off a value |
| **Elimination** | `Case`, `Match`, `IdJ`, `InductiveRec` | `Id` and `Refl` are in the fragment; eliminating them is not. `InductiveRec` is a recursor application |
| **Surface forms** | `Data`, `Dec`, `Ann`, `Con` | declaration, `let`/`letrec`, ascription, and a constructor application whose inductive is implicit — forms the codec does not emit into a proposition slot |
| **Effects** | `InstitutionInvoke` | dispatches a comorphism; its result is not determined by the proposition alone |
| **No Lean image** | `EigonPrimitive(Json)` | a chain-side carrier with no Lean counterpart |
| **Unannotated binder** | `Lam` | see §4.4 |
| **Open** | `InductiveCtor` | see §4.3 |

### 4.3 `InductiveCtor` — settled `2026-09-03`: refused, and the name is not the open part

The original table mapped `InductiveCtor(decl, c, args)` to `Const(name.c, [])`. It is refused,
and the reason is not that the name is undecided.

**D30 v1 emits no inductives.** The generator produces a `structure` per mirrored *class* and
nothing else — `mirror_gen/mod.rs:603` records the gap in its own words, *"`InductiveType`
bucket — those land with D30 v1.1"*, and D30 §11's roadmap puts Lean `inductive` support
(needed for `core:allows_only` enums too) at Phase 20b / v1.1. So an inductive is not in the
mirror, its constructors are not in the export, and there is no constant for a name to denote.
The refusal is a consequence of the mirror's coverage, not of an undecided mangling.

**When they do land, the name is `<inductive>.<ctor>`** — Lean's own convention, and the only
candidate. The alternative that made this look open was D85's derived constructor *class*: a
value's `is_a` names `ctor_classes::class_iri(inductive, ctor)`, which is `{inductive}-{ctor}`.
That name cannot be a Lean identifier at all — `validate_lean_identifier_chars` admits only
`[A-Za-z0-9_]`, and the separator is `-`. (D85 chose `-` deliberately: `.` is what ESL cannot
spell in a local name.) So the two-names-for-one-thing framing was wrong; there was one.

Nothing changes for the externalizer today: `InductiveCtor` refuses with a diagnostic naming
D30 v1's coverage. `Const(iri, …)` naming an inductive fails the same way, through
`UnknownConstant` — which reports the Lean name it looked for, so the diagnostic is legible
either way.

### 4.4 `Lam` is refused because Mini-TT lambdas carry no domain

The original table had `Lam(p, e)` in the fragment — *"appears inside propositions as a
motive"*. It cannot be translated as it stands.

`Exp::Lam(Patt, Box<Exp>)` is **Mini-TT's unannotated lambda**, inherited with the rest of this
AST from the Coquand et al. reference implementation `kernel/src/nbe/` ports. It carries no
domain by design, because Mini-TT is bidirectional: a lambda is only ever *checked* against a
known `Pi`, which supplies one. The kernel holds that line — `check_infer` has no `Lam` arm
(pinned as "not inferable"), and `(Exp::Lam(..), Val::Sort(n))` is an explicit type error, so a
λ cannot *be* a proposition. It can only appear as an argument inside one, where the applied
function's type determines its domain.

Lean's `Lambda` requires a domain, and `def_eq` **compares** it: `def_eq_binder_aux` runs
`if self.def_eq(t1, t2) { … } else { return false }` over the binder types. So there is no
placeholder that `def_eq` sees through — a wrong domain is a wrong term.

Admitting `Lam` therefore means making externalization **bidirectional**, threading the expected
type down so a lambda under an application takes its domain from the function. That is a real
change to the shape of §2's pipeline, not a missing row.

**Measured before refusing:** of the 102 committed `canonical_proposition` values in the tree,
**zero** contain a `Lam`. The four occurrences of "Lam" in `ontologies/` are the *declaration* of
the constructor in `eigentt:Term`, `lean:LeanExpr` and the formulas term type, not uses. v1 gives
up nothing that exists.

### 4.8 Float is translated — `2026-09-04`

An earlier reading of this section — *"Lean has no float literal"* — was wrong in a way worth
correcting, because it made the refusal look permanent when it was neither permanent nor
expensive. `LitFloat` and `EigonPrimitive(Float)` are both in the fragment now.

**Lean 4 has a proper IEEE 754 binary64 `Float`.** Checked against the pinned toolchain
(`leanprover/lean4:v4.29.1`):

```lean
structure Float where toModel : Float.Model
structure Float.Model where
  toBits : UInt64
  valid  : Float.Model.Format.binary64.Valid toBits.toBitVec
```

A real structure over a bit pattern with a validity proof — not opaque, not axiomatised.
`Float.add` is an ordinary definition over the model depending only on `propext`,
`Classical.choice` and `Quot.sound`, and it **computes in the kernel**:
`example : (1.0 : Float) + 1.0 = 2.0 := by rfl` goes through.

So what actually blocks `LitFloat` is two other things.

**There is no float literal `Expr` node.** Lean's `Expr` carries `StringLit` and `NatLit` and
nothing else — which is exactly why nanoda exposes `mk_string_lit` and `mk_nat_lit` and no float
constructor. `(0.0 : Float)` elaborates to

```
@OfScientific.ofScientific.{0} Float instOfScientificFloat (nat_lit 0) Bool.true (nat_lit 1)
```

a typeclass application over *nat* literals. Translating a `LitFloat` means synthesising that
spine and pinning `instOfScientificFloat` — mechanical, but it is elaboration, and §4.6 already
records where the line between checking and elaborating falls.

**`Float` vs `Real` — decided `2026-09-04`: `Float`, i.e. IEEE 754/854.**

A chain proposition about a measured quantity is a claim about the value a computation actually
produced, not about an idealised real that the computation approximates. An IC50 on the chain came
out of a solver operating on doubles; its rounding IS part of what is being asserted, and
`Real` would state something the pipeline never established. `Real` also lives in Mathlib, absent
from a bare export, so it would drag a dependency in for a weaker claim.

This is newly reasonable rather than merely convenient. Lean's `Float` is no longer an opaque FFI
handle — the model above is binary64 *bits with a validity proof*, and arithmetic over it reduces
in the kernel — so propositions about IEEE arithmetic are checkable there, which is exactly what a
measurement claim needs.

With that settled the encoding was the only thing left, and it is thirty lines: format the `f64`
with `{:e}`, fold the decimal point into the exponent, and build the five-argument spine.
Negative values wrap in `@Neg.neg.{0} Float instNegFloat`, which is what Lean itself emits for
`-1.5`. NaN and ±∞ are refused — they have no decimal form, so there is nothing to round-trip.

**Three things only building it revealed.**

`Float` pulls in the standard axioms. Its model depends on `propext`, `Classical.choice` and
`Quot.sound`, so any export using floats needs them permitted — which the institution's
`DEFAULT_LEAN_AXIOMS` already carries, but a test supplying its own list does not.

The universe must be explicit. `@OfScientific.ofScientific.{0}` sits at level 0 because it builds
a `Float : Type 0` — fixed by the type it operates on, not by the enclosing declaration. The
general `const_levels` takes the TARGET's parameters, which is right for a constant standing at
the target's universe and wrong here; `lean_const_at` supplies a literal level instead.

And a test that used `LitFloat` as its example of an out-of-fragment variant had to change,
which is the sort of thing that makes a fragment change visible.

**Verified against terms Lean elaborated**, not against the encoding's own idea of itself:
`0.1` and `-2.5` match their proofs, `0.2` does NOT match a proof about `0.1`, `∀ (x : Float),
Measured x → Measured x` matches, and `NaN` is refused by name
(`externalize_test`, five tests, fixture at `lean/research/sigma-fixture/`).

**What this changes.** The fragment now expresses a measurement, not merely a quantification
over one. A claim naming an IC50, a viability value or an effect size can be stated and checked.

What still stands between that and the WRN chain is not the number: it is the RELATION. A claim
like `0.0 ≤ x` is `@LE.le.{0} Float instLEFloat …`, and nothing maps a chain relation onto a Lean
typeclass operator at a chosen instance — D30 mirrors classes as `structure`s and stops there.
Naming one by hand is the naming disagreement §3.3 exists to prevent. A chain axiom
`demo:le : Float -> Float -> Prop` works today, but it is not Lean's `≤` and does not inherit its
lemmas.

### 4.5 Σ is `Subtype` — settled `2026-09-03` by measurement

The original table refused `Sig` on the ground that *"Lean's `Sigma` is library, not primitive;
admitting it means pinning WHICH `Sigma`, a decision of its own"*. The decision turns out to be
made by the data, and by a constraint the kernel already imposes.

**Lean 4 core ships four Σ-shaped types, and there is no default.** `∃` notation binds `Exists`
specifically, but that is notation, not a canonical choice:

| | signature |
|---|---|
| `Exists` | `{α : Sort u} → (α → Prop) → Prop` |
| `Sigma` | `{α : Type u} → (α → Type v) → Type (max u v)` |
| `PSigma` | `{α : Sort u} → (α → Sort v) → Sort (max 1 u v)` |
| `Subtype` | `{α : Sort u} → (α → Prop) → Sort (max 1 u)` |

**What the formalizer actually builds is `Σ x : <class at Set>. <predicate at Prop>`.** Every Σ
goes through `dcg/rules/combinators.rs::refine_conjoin`, whose second component is an application
of a relation declared into `Prop` — `axiom ontology:compound_kind : lexicon:Entity -> Set ->
Prop`, and its siblings `compound`, `prep_of`, `prep_in` — or `logic:And` over two of those,
which `logic.esl` documents as *"sort-typed at `Prop`, mirroring Lean's `And (a b : Prop) :
Prop`"*. `ontology.esl:65` states the intended shape: *"mutator load" → `Σx:Load.
compound_kind(x, Mutator)`*.

That signature is `Subtype`'s exactly. The universes line up too: EigenTT's Σ-in-`Prop` is
predicative (`nbe/check/mod.rs`: *"Sigma in Prop is predicative — both components must be in
Prop"*), so a Σ over a class cannot inhabit `Prop` and its domain sits at `Set`; `Subtype` over a
`Sort 1` domain gives `Sort (max 1 1) = Sort 1`.

**Three things settle it beyond fit.** `Exists` is ruled out independently — it has no
projections, and `Exists.elim` eliminates into `Prop` only, so `Fst`/`Snd` could never have an
image. `Sigma` requires both components in `Type`, excluding the `Prop` predicate that is the
whole point. And **D30 already emits `Subtype`**: the golden `Mirror.lean` renders a
refinement-constrained field as `{ x : Float // 0.0 ≤ x ∧ x ≤ 100.0 }` and its encoder projects
with `.val`. Choosing it adds no mirror vocabulary.

The `Subtype` level is `1`, not one of the target's parameters: which universe it sits at is
fixed by its domain, and a parsed Σ's domain is a class, which D30 emits as a Lean `structure` —
a `Type`.

**Verified against a real declaration.** `crates/eigenius-lean/test_resources/sigma_subtype.json`
exports `def refined : { w : EigeniusFFI.eigenius.test.Widget // …Big w } → PUnit`, with the
structure and the relation named as §3.3's mangling spells them.
`externalize_test::a_sigma_over_a_class_matches_a_lean_subtype` externalizes
`Arrow(Sig(w, Widget, Big(w)), One)` and `def_eq`s it against that type. Regenerate it the way
§6.5 regenerates the capstone — `lake build && lake exe lean4export SigmaFixture -- refined`.

**What this does not unblock.** `Lam` (§4.4) still needs bidirectional externalization, `Pair`
needs unification, and `Fst`/`Snd` under a binder need the upstream change in §4.6. The parser
builds all of those. Admitting Σ removes the quantifier from the blocking list, not the whole of
parser output.

### 4.6 `Fst` / `Snd` translate — and what it took

The projections were first refused because `Subtype.val : {α} → {p} → Subtype p → α` takes
implicits the EigenTT form does not carry. That reasoning was wrong twice over, and the
corrections are the interesting part.

**The implicits are recoverable.** The externalized scrutinee is a well-formed term whose type is
`Subtype α p`; inferring it and destructuring the spine gives both. `TypeChecker::infer` is
`pub(crate)`, but `is_proof` is public and returns `(is_prop, infer(e))`.

**What blocked it was building with loose bound variables.** `infer` rejects them outright ("no
loose bvars allowed in infer"), so a term built by emitting `mk_var(depth - 1 - position)` while
descending — the obvious reading of §3.1 — cannot be inferred under a binder. nanoda's own answer
is **locally nameless**: descend by turning the binder into a FREE variable (`mk_dbj_level`),
work on the open term, then `abstr` it closed. Externalization now does the same, so a sub-term
is inferrable at any depth.

**Following it there needed one upstream change.** Inference and construction were mutually
exclusive from outside nanoda: `TypeChecker::new` asserts `dbj_level_counter == 0`, so no checker
may be created once a binder is open, and the `ctx` field was `pub(crate)`, so a live checker
could not be used to build. `eigenius/nanoda_lib` adds the accessor —

```rust
pub fn ctx(&mut self) -> &mut TcCtx<'t, 'p> { self.ctx }
```

— and externalization holds **one** checker for the whole pass, building through it. A PR is open
upstream; the fork exists so the work is not blocked on review. The fork is branched off upstream
`master`, which also picks up their "prohibit orphan recursors" tightening that the previous pin
`6ae1f0c` predates.

Two consequences worth recording. The comparison must run on that same checker — opening a nested
one after externalizing a binder trips the counter assertion, which is exactly what it did on the
first attempt. And the `EnvLimit` is now set once, by `with_tc_and_declar(info)`, rather than per
inference: `Fst`'s inference and the final `def_eq` share one environment by construction, which
is what §6.5 wanted anyway.

**`Pair` stays refused**, and for two independent reasons — see §4.7. `Subtype.mk : {α} → {p} →
(val : α) → p val → Subtype p` needs the predicate `p`, and recovering it from the type `p a` is
higher-order unification, which is the elaborator's work and not something a checker of
already-elaborated terms offers. And nothing produces one.

### 4.7 Posture on the refusals nothing produces: `Record`, `Pair`, `Refine`

Three refusals share a property that changes what to do about them: **nothing generates the
shape**. They are not queued features waiting on a decision, and treating them as such invites
building against a guess — which is what §4's stale table already cost once. Their prospects
still differ, so they are separated here.

**`Record` can never be translated, and refusing it is the correct answer rather than a deferred
decision.** A `Record` is an *anonymous* structural type: a canonically-ordered telescope of
named fields. D30 mirrors **named classes** as `structure`s, so an anonymous record has no Lean
name because no declaration was ever emitted for it. Unlike §4.5's `Sig`, this cannot be fixed by
choosing a library type: the proof was compiled against a fixed environment, and a declaration
cannot be added to an export after the fact. A record the proof never mentioned can never be
`def_eq` to anything in it.

**`Refine(R, {C})` over a single class plausibly can be.** It carries the record *together with
the classes it satisfies*, and nominal identity is the point — `Exp::Refine`'s own documentation
records that `Refine(R, {Alpha})` and `Refine(R, {Beta})` must differ even when `Alpha` and
`Beta` have the same fields (D75 §8 Q2). So the class name is the identity and the record is the
structural detail it stands for, which makes `Const(C)` — the class D30 already mirrors — the
faithful image.

Two things stop that being written today, and neither is a naming question:

1. **Multi-class `Refine(R, {C, D})` has no single Lean name.** D30 emits one `structure` per
   class; a record satisfying two of them is an intersection Lean has no primitive for.
2. **Nothing produces one.** No committed proposition carries a `Refine`, and the DCG formalizer
   never builds one (`Exp::Refine` is constructed only in `nbe/readback.rs` and the D47 codec).

**`Pair` has no producer either, and the reason is structural.** Measured `2026-09-03`: the DCG
formalizer never originates one. Every `Exp::Pair` under `kernel/src/dcg/` is a traversal —
rebuilding an existing node during substitution or hole-walking, or destructuring it to
pretty-print — and ESL's surface has no pair syntax at all. The two real construction sites are
on the PROGRAM path: `program/expr.rs:378` packs `Pair(arg, EigonResource(resolved))` as a
dispatcher calling convention, and `parse_pair` reads `program:first` / `program:second` from the
program AST. Neither lands in `reflection:canonical_proposition`.

That follows from what a proposition is. A pair is an *introduction* form — a value — and a
proposition is a type. `Sig` appears because a refinement is a type; `Fst`/`Snd` appear because a
projection can occur inside one (`Big s.val`). A bare `Pair` would be a term embedded in a type,
which the parser has no reason to build.

**Posture: refuse all three, and read an arrival as a signal rather than a feature request.**

- A `Record` reaching externalization means a proposition quantified over an anonymous shape —
  a modelling error upstream, and the refusal should say so.
- A `Pair` reaching it means something is generating a value where a type belongs. Same reading.
- A single-class `Refine` is the exception: ordinary, and it should be implemented as `Const(C)`
  at that point — MEASURED the way §4.5 measured `Sig`, not assumed.

## 5. What this makes true, and what it does not

**Numbers, but not yet Lean's arithmetic.** §4.8: floats are translated, so a measurement claim
can be stated and checked against IEEE 754/854 semantics. What is not available is Lean's own
relational vocabulary — `≤` is `@LE.le.{0} Float instLEFloat`, and nothing maps a chain relation
onto a typeclass operator at a chosen instance. A claim can say `Measured(0.1)`; it cannot yet
say `0.0 ≤ x` and mean Lean's `≤`.

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
   call, and a mismatch is `Verdict::Fails`.** One arena, one parse: the export is parsed there already, so externalizing the
   expected statement into the same `TcCtx` is the cheap direction, and both sides must share an
   arena for `def_eq` to be callable at all. The cost is `check_proof`'s signature, which gains
   the expected statement as an option. A second entry point would keep that contract and pay a
   second parse of the same bytes for it, which is the wrong thing to protect.

   A mismatch **vetoes the commit**. §9.11.1: *"An institution may return `Fails` and block a
   commit on its own authority."* A proof term whose statement is not the claim's is exactly
   that case, and the asymmetry §9.11.1 draws applies — an incorrect `Fails` loses data, which
   is recoverable; letting the resource land as `Undecidable` puts a proof term on the chain
   whose claim it does not prove, looking like an ordinary unverified one. The
   `verdict_provenance` Sibling still lands as the audit anchor (D41 §6.1).

3. ~~**What happens when the fields are absent?**~~ **Decided `2026-09-02` — `claim_iri` becomes
   `requires`, the other two slots are deleted, and the reseed is taken now.** Without
   `claim_iri` there is no claim to externalize and the check has nothing to be total over. The
   alternative — leaving it recommended and having the institution reject rather than skip —
   costs no migration and changes the same behaviour, at the price of a schema that no longer
   describes what the institution accepts; §9.11's whole point is that the declaration is the
   contract.

   Taken now rather than later because the edit moves the `lean-institution` layer hash and every
   layer id below it, and it bundles with #208's ontology work, which moves the manifest anyway.
   One reseed covers both. Deferring means paying it once collaborators hold chains committed
   against the current ids.

### 6.5 Mechanics settled while implementing (`2026-09-03`)

Three questions the document left implicit, each answered by reading nanoda rather than by
choosing.

**Which `EnvLimit` the comparison runs under: `ByName(target_name)`.** nanoda checks every
declaration with `EnvLimit::ByName(d.name)` (`util.rs::with_tc_and_declar`), which cuts the
environment off *at* that declaration so it sees only what precedes it. Comparing under the same
limit compares under the environment nanoda used when it checked the theorem. `PpUnlimited`
would let δ-unfolding reach declarations *after* the target, admitting equalities the proof's own
check would not. `with_tc_and_declar(target_info, …)` sets it.

**Universe parameters must align with the target's.** The same call passes `DeclarInfo`, which
carries `uparams`; `check_declar_info` asserts `no_dupes_all_params(info.uparams)`. A
`Level::Param(n)` in an externalized statement has to name a parameter the target declares —
otherwise `def_eq` compares one parameter against a different one and fails with nothing to say
that universes were the cause. Externalization takes the target's `uparams` and refuses an
unknown name the way it refuses an unknown constant.

**Regenerating the capstone proof is mechanical.** `lean4export` is vendored in-repo at
`lean/runtime-worker/vendor/lean4export/` and required by path from
`lean/research/capstone-proof/lakefile.lean`; both pin `leanprover/lean4:v4.29.1`. Nothing needs
installing:

```sh
cd lean/research/capstone-proof
lake build
lake exe lean4export Capstone -- patient_weight_nonneg \
  > ../../../crates/eigenius-lean/test_resources/capstone_proof.json
```

Verified `2026-09-03`: run against the current tree it reproduces the committed fixture
byte-for-byte (527,994 bytes). That matters for #208 — the capstone's hand-rolled
`EigeniusFFI.lean` declares `structure Patient`, the pre-qualification name, and must be
re-declared as `structure eigenius.test.capstone.Patient` with the theorem quantifying over it
before this check can resolve the constant. The lakefile records why hand-rolling is legitimate:
the verification side reads `library_content_hash`, `mirrored_classes` and `source_layer`, never
the source's shape.

### 6.3.1 `lean:LeanProofTerm` drops to three slots

`def_eq` becomes the **one** correspondence check, and it is total. Everything the two
`recommends`-and-skip checks were doing, it does better:

| slot | check | fate |
|---|---|---|
| `claim_iri` | — | **`requires`**. The statement is manufactured from it. |
| `proof_payload` | 1, nanoda | `requires`, unchanged |
| `target_name` | 1, nanoda | `requires`, unchanged |
| `lean:proposition` | 2c, short-name correspondence | **deleted** |
| `mirror_iri` | 2a, `source_layer` ancestry | **deleted** |

**2c is subsumed.** It asked whether the committed proposition *mentions* the claim's class —
a proxy for the question `def_eq` answers directly. Its implementation is also the place
#208's collision does its damage: `short_to_iri` is a `BTreeMap<String, Iri>` keyed on
`core:short_name`, so on a colliding pair the second insert overwrites the first and a mirror
holding both `reflection:Person` and `schema_org:Person` silently keeps one.

**2a is subsumed.** A moved mirror makes the externalized `Const` names disagree with the
export's, so `def_eq` fails. The cost of deleting it is diagnostic, not soundness: a version
skew arrives as a statement mismatch rather than as `FFIVersionMismatch` naming the moved
layer. Accepted `2026-09-02` on the ground that one total check beats two partial ones plus a
better error message.

**This retires D40's direction.** `chain_mirror.rs`, `bytes_to_lean_expr` and the four
`lean:Lean{Name,Level,LevelList,Expr}` inductives exist to recover a proposition *from* Lean.
Nothing reads the result once the goal is manufactured from the claim. D40 stays as the record
of why the inverse was built — the premise that made it necessary is stated in §7 — and its
implementation goes.

**Sequencing note.** The demo fixture's generator writes the proposition through
`bytes_to_lean_expr`; that step goes with it, and the fixture regenerates. Until this lands the
fixture must keep landing `Holds`, so `notebook_demo_fixture_lands_holds` (un-`#[ignore]`d in
#207) stays green across the change rather than being suspended for it.

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
