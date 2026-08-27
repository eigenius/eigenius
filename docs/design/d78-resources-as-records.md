# D78 — Resources as records

**Status: IMPLEMENTED**, merged `2026-08-26` in [#229](https://github.com/eigenius/eigenius/pull/229).
Written `2026-08-24`. Every phase's own status line below reads *complete*; §9's two deferrals were
reviewed and closed out before the merge.

Implements **Seam B** of `docs/design/d75-fusing-eigentt-and-the-knowledge-graph.md`. D75 established
*what* and *why*; this document is *how*. It inherits D75's decisions and does not reopen them:
clause 8 as the membership rule (§6.4), **7b** (`Construct` returns a refinement), **9c** (dispatch on
a declared, commit-checked `is_a`), **10d** (`subclass_of` declared with entailment checked), Rule 0
kept as curation via `core:Resource` (§6.6), and classes δ-**opaque** (§8 Q2).

## 0. The decision

A class is a **constraint**, implemented three times and never reconciled — intensionally in the
validator, extensionally in the query engine, and as a `Val::Sigma` in the kernel that no membership
decision consults (D75 §6.0). This document makes the kernel's the same object as the other two, by
replacing the Σ-chain with a **record type** whose membership rule is A11.2 clause 8.

What it settles: the record's representation (§1), what `recommends` and conditional requirements
contribute to it (§1.1, §1.3), how a record's level is computed (§2), the refinement type 7b needs
(§3), the entailment judgment 10d needs (§4), whether `Any` is the lattice top (§4.1), and what
changes in each of the three implementations (§5).

## 1. `Val::Record` — a canonically-ordered dependent telescope

### The tension

A11.2 builds record types by **set union**, which is order-free. A11.6 dependency families let a
later field's type mention an earlier field, which requires *some* order. Both are required; neither
can be dropped.

### What exists already

**The current class Σ-chain is not dependent at all.** `build_sigma_chain`
(`program/ground.rs:293-310`) stores the rest of the chain in the closure's *environment* under a
fixed `__sigma_rest` binder and returns `Exp::Var(rest_var)` as the body, with its own comment saying
*"the rest type doesn't depend on the current property's value"*. So the dependency `Sig` can express
is unused for classes: the encoding is a flat product wearing Σ clothing.

That is why §1.3's conditional requirements live in the validator. A value-dependent field is not
merely unexercised in the class type — it is **inexpressible** in it today, so the check had nowhere
to go but the rule set.


`Val::Sig(Box<Val>, Clos)` (`nbe/val.rs:45`) is **already a named dependent telescope**: `Clos.patt`
carries the field name — which is how `find_sigma_field` matches on `Patt::Var(field_name)` — and
`Clos.body` + `Clos.env` let a later field's type mention earlier ones. The Σ-chain is not missing
names or dependency. It is missing:

1. **order-insensitivity** — `eq_nf` compares by `readback_val` and syntactic equality
   (`check/conv.rs:68-69`), so two orderings of the same fields are not equal;
2. **membership by inclusion** — clause 8 tests `⟨ℓ,a⟩ ∈ r`, while a Σ-chain demands the exact
   nesting;
3. **extra fields** — a Σ-chain has exactly its own.

### The representation

```rust
Exp::Record(Vec<(Iri, Patt, Exp)>)
Val::Record(Vec<(Iri, Patt, Exp)>, Rho)     // flat field list + one shared environment
```

**A per-field `Clos` does not work, and an earlier draft of this section specified one.** Field *i*'s
type may mention **any** earlier binder, not just the immediately preceding one, so each closure
would need an environment that does not exist until the earlier fields are known. Sharing one `Rho`
and extending it as the telescope is walked gives full dependency, exactly as `Sig`'s nesting does —
and it is the shape `Val::Data(Vec<(Name, Exp)>, Rho)` already uses for the same reason.

The fields are held in a **canonical order**: the topological order induced by the dependency
relation, ties broken by IRI, established by `Exp::record` (`nbe/term.rs`). That is:

- **deterministic** — the same field set always produces the same telescope;
- **dependency-respecting** — a field never precedes one its type mentions;
- **order-insensitive where it matters** — because canonical order makes union-equality decidable by
  the syntactic comparison `eq_nf` already performs, with no new conversion arm.

**This makes D75 §3.7's accident into a stated invariant.** Today the Σ-chain is well-defined only
because properties happen to live in a `BTreeMap` keyed by IRI. Canonical order is the same
determinism, established deliberately, with an invariant that can be asserted and a reason it holds.

**Why not plain IRI order.** Field `b` may depend on field `a` while sorting before it. Sorting by
IRI alone would then produce an ill-formed telescope, and rejecting such records would be a wedge —
the dependency is expressible and legitimate. Topological-with-IRI-tiebreak accepts it and is still
canonical.

**Cycle detection.** A dependency cycle has no topological order and is ill-formed. It is caught
where the record is built, not at use.

### 1.1 What `recommends` contributes to the record type: nothing

`recommends` names properties that are **not required for membership but commonly present**. That is
an annotation, and it has no type-level content.

Today `resolve_class_type` Option-wraps them into the Σ-chain (`program/ground.rs:73-81`), giving a
field of type `Option T`. Under clause 8 that is the wrong shape: `Option T` says the record **has** a
field holding `some x` or `none`, whereas a resource that omits the property has **no field at all**.
Absence and `none` are different states, and only one of them is what `recommends` describes.

**So the class's record type is its `requires` fields, and nothing else.** Four consequences:

- **The Option-wrapping is deleted.** It exists only because a *class* type had to represent
  "maybe there". Once a resource's type is the union of its own fields (§5), the maybe-ness lives
  where it belongs — the resource either has the field or does not.
- **Nothing is lost in checking.** `recommends` never gated admission; Rule 3 checks any present
  property against its declaration regardless of class, which is exactly *if `⟨ℓ,a⟩ ∈ r` then
  `a : T`*.
- **Entailment (§4) ignores it.** A subclass may drop a parent's recommendation, because it was never
  a requirement.
- **Projection improves.** A recommended property present on a resource is in that resource's own
  record at type `T`, not `Option T`. Projecting one off a *class* type fails — correctly, since the
  class does not guarantee it.

**The `Option` survives where it is right.** The mirror generators — Julia (`mirror_gen.rs`, 30
sites) and Lean (`mirror_gen/`, 27) — emit closed-world target-language structs, which genuinely need
a nullable slot for a maybe-present property. A closed struct needs `Option`; an open-world record
does not. The construct is not being deleted, only removed from the type system.

### 1.2 What this measures, and what it settles

Across the shipped ontologies: **749 of 894 declared classes have no `requires` at all**, including
**all 734 schema.org classes**. Under this reading those classes have **empty record types** — at the
type level they are `Any`, and their entire content is nominal (the name, the `subclass_of` lattice)
plus annotation (`recommends`, `description`).

That is accurate rather than alarming: schema.org is a vocabulary, not a schema, and genuinely
requires nothing. The type level should say so.

**It is also the strongest concrete argument for nominal identity in either document — and the
collapse is not hypothetical.** `resolve_class_type` short-circuits an empty property set to
`Val::One` (`program/ground.rs:83-85`). So **all 749 of those classes already resolve to `Val::One`
today**, the unit type with one inhabitant, and are definitionally equal to each other in the type
language right now.

D75 §8 Q2 argued classes must be δ-opaque because two same-field classes would otherwise become
definitionally equal, demonstrated on a two-class synthetic fixture. The chain is far past that
fixture: 749 classes — `schema_org:Person`, `schema_org:Organization`, all of them — currently
resolve to the *same* `Val`. The only thing keeping them apart is that `Val::EigonClass(iri)` is
opaque and `eq_nf` never unfolds it (D75 §3.3). **δ-opacity is not a precaution here; it is the sole
mechanism distinguishing most of the shipped ontology.**

Under records those classes resolve to an *empty record* rather than `Val::One`, which is equally
collapsed structurally. Nominal identity stays essential for the same reason.

### 1.3 Conditional requirements are dependent fields

`core:ConditionalRequirement` (`validation/rules/conditional.rs:117-165`) has the shape

```
{ when_property: ℓ_k,  has_value: [v₁ … vₙ],  then_requires: [ℓ₁ …],  then_recommends: [ …] }
```

— *if the resource's `ℓ_k` holds a value among `v₁ … vₙ`, require `ℓ₁ …`*. The match is **IRI-valued
only** (`as_iri` / `as_iri_array`), so the condition is a finite disjunction over discrete tags, not a
test on arbitrary data.

**That is a dependent field**: the presence of `ℓᵢ` in the record type is a function of the *value* of
`ℓ_k`. And it is exactly the shape A6 supplies — `has_value: [v₁ … vₙ]` is a **join of singleton
types** `T_{v₁} ∨ … ∨ T_{vₙ}`, which D75 §6.7 already identified as something Eigenius encodes ad hoc.
Making `has_value` a real singleton join is the same change as making conditional requirements
dependent fields.

**It splits by whether a resource is in hand, and only one half is on the critical path.**

| | case | what is needed |
|---|---|---|
| **(a)** | checking `r : C` | the conditions are evaluated **against `r`'s values**, yielding a concrete required field set, and clause 8 applies to that with no dependent machinery. This is what `evaluate_conditional_requires` already computes, and §5's validator step therefore needs **nothing new** |
| **(b)** | the class type standing alone — `Construct`, quantification, D18 ontology-as-types | the field set genuinely depends on a value, so the telescope carries `ℓᵢ`'s presence as a function of `ℓ_k`. M1 (D75 §8a) measured this case as **unexercised by any shipped ontology** |

So conditional requirements are settled for the work §5 and §7 actually schedule, and the dependent
form is specified but not blocking. §5 becoming a function of a *resource* is what makes (a) work:
with the resource in hand the dependency is discharged before clause 8 is applied.

**They are also a source of dependency edges for §1's ordering.** The telescope's topological sort is
not driven only by a `class_types` that references an earlier field — `when_property` induces an edge
too, since `ℓᵢ` must follow `ℓ_k`. Both feed the same relation.

## 2. Levels are computed

A record's level is the **max over its field types' levels**, floored at `Set`:

```
level(Record{ℓᵢ : Tᵢ})  =  max(1, maxᵢ level(Tᵢ))
```

The `max` is the standard MLTT rule for Σ. The **floor at 1 is a real decision**, not a convention:

- max over an empty field set is `Zero`, which would put the empty record — `core:Resource`, the
  `Any` class (D75 §6.6) — in `Prop`;
- `Prop` carries proof irrelevance (D46), so every two inhabitants would be definitionally equal;
- `core:Resource` has 9.4M distinguishable inhabitants.

So the floor is forced by proof irrelevance, and it agrees with the behaviour being replaced: today
every class inhabits `Set` and above (`check/mod.rs:564-568`).

This is what D75 §6.1 means by a class being level-generic *without* level parameters: the level is
read off the fields at each site, never declared and never instantiated.

**The `max` is over an instantiated telescope, not over static types.** A field's type is a `Clos`, so
for a dependent field `Tᵢ` is a function of earlier fields. The level is computed by walking the
telescope and **instantiating each closure at a fresh neutral variable** — the same discipline
`readback` uses — and taking the level of the resulting type:

```
level(Record[(ℓ₁,C₁) … (ℓₙ,Cₙ)])  at de Bruijn level k:
    l := 1
    for i in 1..n:  l := max(l, sort_level(infer(Cᵢ.apply(fresh_var(k + i - 1)))))
    return l
```

This is well-defined because **a dependent type's universe does not vary with the value**: `B(x)`
inhabits one `Sort(l)` for every `x`, so any instantiation gives the same answer and a fresh variable
is the canonical choice. It is `max`, not `imax` — `imax` belongs to `Π`, where a `Prop` codomain
collapses the whole type to `Prop`; a record is a Σ-shape and takes the plain maximum.

## 3. `Val::Refine` — the type 7b returns

```rust
Refine(Box<Val>, BTreeSet<Iri>)   // the record type, and the classes it satisfies
```

**A set of constraints, not a nest.** `is_a` is a list, so a record may satisfy several classes.
`Refine(R, {C, D})` is the direct image of that; `Refine(Refine(R, C), D)` is not. Three reasons the
flat form wins:

1. **Canonicity.** A `BTreeSet` has one representation. Nesting gives `Refine(Refine(R,C),D)` and
   `Refine(Refine(R,D),C)` as two spellings of one type, which `eq_nf` — readback plus syntactic
   comparison — would treat as distinct. That is the exact problem §1's canonical field order solves,
   and the same answer applies.
2. **The zero case degenerates cleanly.** `Refine(R, ∅) = R`, which is the synthesis's "0 or more
   constraints" with no special case. Nesting has no spelling of zero except bare `R`, so it needs a
   normalization rule to avoid two representations again.
3. **It matches the surface.** `is_a` is a set of declared names; so is the refinement.

**Restricted to a named class constraint, not an arbitrary predicate.** That is what keeps

- **nominal identity (D75 §8 Q2)** — `Refine(R, Alpha)` and `Refine(R, Beta)` differ even when `R` is
  shared, which is exactly the collapse Q2 forbids;
- **conversion decidable** — entailment between named constraints is §4's algorithm, whereas
  arbitrary predicates would need a prover.

| judgment | rule |
|---|---|
| equality | `Refine(R, S) ≡ Refine(R′, S′)` iff `R ≡ R′` and `S = S′` (**set equality on IRIs** — nominal) |
| subtyping | `Refine(R, S) <: Refine(R′, S′)` iff `R <: R′` and `⋀S ⊨ D` for every `D ∈ S′` |
| readback | `Exp::Refine(Box<Exp>, BTreeSet<Iri>)` |
| D47 codec | a `Refine` ctor beside the existing `Sig`/`Pi` arms (`eigentt_type_mirror.rs:111-127`) |

### 3.1 The complete subtyping rule is blocked on D76 — a cross-seam dependency

**Found while implementing Phase A.** `⋀S ⊨ D` resolves class IRIs against the layer chain, and
conversion has **no layer**: `subtype_of(level, sub, super_)` and `eq_nf(level, v1, v2)`
(`nbe/check/conv.rs:290`, `:30`) take no context at all, and `conv.rs` contains no reference to
`Layer` or `resolve`. Supplying one is D76's subject (D75 §8 Q1).

**This contradicts D75 §9 and §7 of this document**, which both state Seam B is gated on nothing in
Seam A. It is gated on one thing: the complete `Refine` subtyping rule.

**What ships instead, and why it is not a compromise of soundness.** Conversion uses set inclusion,
`S ⊇ S′`, which is sound — a constraint present in `S` is trivially entailed by `⋀S` — and
**incomplete**: it rejects the case where `S` entails `D` without containing it. An incomplete
subtyping relation refuses some legal programs; it never admits an illegal one. Strengthening it to
the full rule is a one-arm change once conversion carries `Γ_env`.

**The alternative was rejected on principle.** Precomputing each constraint's field set into the
`Refine` value would let conversion decide entailment with no layer — and is exactly the
inline-the-environment antipattern D75 §3.1 identifies as the root defect of the whole system. Taking
it here would reproduce, in new code, the thing this programme exists to remove.

Pinned by `entailment_beyond_set_inclusion_is_not_yet_decided` (`nbe/readback.rs`), which asserts the
rejection *and* records that it is for want of entailment rather than because the judgment is wrong.
The obligation is registered as an inbound dependency in
`docs/design/d76-the-typing-environment.md` §2.1, with that test named as the signal it has landed.

**Cumulativity and forgetting.** `Refine(R, S) : Sort(level(R))` — the constraint set is names, not
types, so it contributes no level — and by cumulativity it inhabits every sort above. Additionally
`Refine(R, S) <: R`: **forgetting constraints is always safe**, which is how a refined record flows
into a context expecting a plain record. The converse is not, and `Refine(R, S) <: Refine(R, S′)` for
`S′ ⊆ S` follows from the subtyping rule.

**Conjunction entailment `⋀S ⊨ D` is §4's rule unchanged.** A constraint is a field set, so the
conjunction of `S` has `fields(⋀S) = ⋃_{C∈S} fields(C)`, and §4 applies to that union exactly as it
applies to a single class. *An earlier draft claimed nesting "composes more obviously" with the
subtyping rule; it does not — §4 was already stated over field sets, so it generalizes to a
conjunction without a new case.*

**Deciding `sat` at `Construct`.** The fields are known statically, so `r sat C` is the §4 algorithm
run against a concrete record rather than another constraint — field inclusion plus a per-field
check. `NativeDecide(Constraint, Box<Exp>)` (`nbe/term.rs:86`) is the existing
check-produces-evidence path and reduces a decided constraint to `Refl`; this is the same shape and
should reuse it rather than introduce a second one.

## 4. Entailment `C ⊨ D`

```
C ⊨ D   iff   fields(D) ⊆ fields(C)
        and   for every ℓ ∈ fields(D):  type_C(ℓ) <: type_D(ℓ)
```

**Covariant in the field type**, via `subtype_of_inner` (`check/conv.rs:308`), which already carries
cumulativity. Justification: if every `C`-record's `ℓ` inhabits a subtype of what `D` demands, it
satisfies `D`'s requirement by subsumption.

**Enforced by a new validation rule** on `subclass_of` declarations: declaring `Pup : Dog` requires
`Pup ⊨ Dog`.

### 4.1 What entailment is actually for — corrected

An earlier draft gave three justifications. **Two of them are vacuous**, and finding out why changes
the plan.

**A field's type is global to the property, not per-class.** `resolve_property_type(prop_iri, layer)`
(`program/ground.rs:154`) takes only the property IRI and reads the type off the property's own
`data_type`; `collect_properties_inner` (`:108`) collects a `BTreeSet<Iri>` — property IRIs, no
types. **A class cannot redeclare or narrow a field's type**, because there is no per-`(class,
property)` type to redeclare.

Two consequences:

- **The per-field variance check is vacuous.** `type_C(ℓ)` and `type_D(ℓ)` resolve the same property
  to the same type by construction, so the `<:` clause can never fail. Entailment reduces to
  `fields(D) ⊆ fields(C)`.
- **The "subclass redeclares at an incompatible type" hole does not exist.** That justification is
  withdrawn.

And **`fields(C)` is the transitive collection** — `collect_properties` walks `subclass_of` — so
`fields(D) ⊆ fields(C)` holds **automatically** whenever `C` declares `subclass_of D`. A validation
rule over `subclass_of` declarations would therefore always pass, and the query engine's subclass
closure is sound by construction rather than by coincidence. Both of those justifications go too.

**What survives is the one that is not about `subclass_of` at all.** §3's subtyping rule needs
`⋀S ⊨ D` where `S` and `S′` are the constraint sets of two `Refine` types — arbitrary sets of classes
drawn from `is_a`, **not necessarily related by `subclass_of`**. Whether the union of one set's fields
covers another's is a real question with no structural guarantee behind it. That is the use, and it is
the only one.

So: **the algorithm is needed; the validation rule on `subclass_of` is not.**

### 4.1 Is `Any` the top of the lattice?

Measured over the shipped ontologies (`2026-08-24`): 894 declared classes, **0** declaring
`subclass_of core:Resource`, **138** with no `subclass_of` at all, and 3 resources declaring
`is_a core:Resource`. So the `Any` class exists in shape and is absent from the lattice. The question
splits three ways and gets three different answers.

**Entailment — already top, automatically, no edge required.** `C ⊨ Any` iff
`fields(Any) ⊆ fields(C)`, and `fields(Any) = ∅`, so it holds vacuously for every `C`. §4's algorithm
makes `Any` the top of the entailment order with nothing declared. **Adding a declared edge would
contribute nothing** to typing, subtyping, or `Refine`'s side condition.

**`subclass_of` — no implicit edge.** Three reasons:

1. **It would be a derived edge in a declared relation.** 10d's whole shape is that `subclass_of` is
   *declared* and entailment is *checked*. Synthesising edges is inference in a nominal relation —
   the thing 10b was rejected for.
2. **It would cost what the project has twice paid.** `class_with_subclass_closure(core:Resource)`
   would become every class in the chain, each triggering a `scan_chain` — the full-chain-scan
   antipattern behind two prior OOMs. Today it returns `{core:Resource}` and scans once.
3. It would rewrite 138 declarations for no typing benefit, since (1) above already holds.

**But the question points at a real hole.** `MATCH ?x : core:Resource` today returns the 3 resources
declaring it directly, not everything. "Query for all resources" does not work. The fix is not an
implicit edge — it is that **the universal class is answered by enumeration, not by closure**. Every
resource is a record, so the query engine can answer `: core:Resource` from the resource index
without walking subclasses at all. That is a query-planner special case for one well-known IRI, and
strictly cheaper than the closure it replaces.

Filed as its own concern rather than folded in: it is a query-engine gap that exists today and is
independent of whether records land.

## 5. What changes in each of the three implementations

D75 §6.4's four changes, as work:

| | implementation | change |
|---|---|---|
| kernel | `resolve_class_type` | returns `Val::Record` (§1) instead of a right-nested `Val::Sigma`, and becomes a function of a **resource** — the class constraint is the declared minimum, the resource's own record is the union of its actual fields |
| validator | Rules 1+2, and 3/8/9 | become an **evaluation of clause 8** against that record: same checks, same verdicts, one definition instead of an independent transitive walk of `requires`. **Rules 4–7 and 10 do not move** — see §5.1 |
| query | `class_with_subclass_closure` + `scan_chain` | **unchanged**. The index is keyed on the *declaration*, and clause 8 changes what "satisfies C" means, not how membership is enumerated |

Plus, in the kernel: `PropAccess` and `Construct` over records, which is what closes D75 §3.8 — an
undeclared property becomes projectable because a resource's type is the union of its own fields.

**`is_a` is unaffected as a surface.** It stays a declared name whose meaning is clause 8 (D75 §6.4).
Nothing becomes inferred; membership is not computed from structure.

### 5.1 What clause 8 does *not* cover — corrected

An earlier draft said Rules 1+2 **and 3–10** become an evaluation of clause 8. That is wrong, and the
line between them matters for scoping Phase D.

Clause 8 has exactly two clauses: `⟨ℓ, a⟩ ∈ r` (presence) and `a : T` (the field's type). A record's
field type comes from `resolve_property_type` (`program/ground.rs:224-258`), which reads
**`data_type`, `allows_only` and `class_types`** — and nothing else.

| rule | clause 8? | why |
|---|---|---|
| 1+2 presence, incl. inherited and conditional | **yes** | `⟨ℓ, a⟩ ∈ r` |
| 3 data_type, 8 class_types, 9 allows_only | **yes** | `a : T` — these *are* the field type |
| 4 format, 5 pattern, 6 range, 7 length | **no** | refinements on the property declaration; not in `Val` |
| 10 domain | **no** | a constraint on the *subject*, checked per-resource — and the rule §6.5 contrasts with `rdfs:domain`, which infers where this rejects |

So Phase D unifies **membership**, which is what §6.0's three implementations disagreed about. Rules
4–7 and 10 are per-property refinements that were never part of the disagreement and are left alone.

Folding them in would mean refinement-on-primitives — a `{x : string | matches(p)}` former, distinct
from `Val::Refine`, which refines a *record* by *class* constraints. Out of scope here, and noted in
§9.

### 5.3 What is deleted

At the end of this work **no Σ-chain is constructed for a class**. Class Σ-chains are built at exactly
one site — `build_sigma_chain(&props)` (`program/ground.rs:309`), reachable only from
`resolve_class_type` — and step 4 replaces it. Deleted with it:

- the `Val::One` short-circuit for an empty property set (`ground.rs:83-85`) — an empty class becomes
  an **empty record**, which is a distinct type per class rather than the shared unit;
- `make_option_type`'s use for `recommends` (§1.1);
- `find_sigma_field`'s class path (§9), replaced by IRI-keyed record lookup.

**`Val::Sig` is not deleted.** It survives for *anonymous pairs*: `Exp::Sig` and `Exp::Times`
evaluation (`nbe/eval/mod.rs:251,361`), which the DCG uses. The distinction is that a record is a
**named** field set and a `Sig` is a positional pair; records subsume the class use of `Sig`, not the
pair use.

## 6. Risk, and why it is smaller than it looks

**M1 (D75 §8a) measured the blast radius.** Across every shipped ontology:

- 11 distinct `ConstRef` targets in encoded terms, **0** of which resolve to a declared class, against
  894 declared classes;
- every binder domain is `Prop`, `Set`, `Type 1`, `core:string`, or a bound type variable — **none is
  a class**;
- the encoded-term constructor census is `ConstRef` 83, `OpRef` 47, `Pi` 27, `Zero` 2, `Sort` 1,
  `Succ` 1, `Var` 1 — **no `Construct`, no `PropAccess` at all**.

So the Σ-chain being replaced **has no users outside the kernel and its tests**. The 9.4M chain
resources are validated by the constraint path (Rules 1+2, 3–10), which §5 changes in implementation
and not in verdict.

**The honest caveat.** This does not prove the `Construct`/`PropAccess` path is unwanted. The demo's
own comment records quantifying over `Set` *because it needs a kind* — the shape a class-as-type
would otherwise serve. The absence may reflect that the current encoding is unusable for it rather
than that the need is absent, and if so this work removes the obstacle rather than a dead feature.

## 7. Phases

Six steps in five phases. The boundaries are drawn where the **risk class changes** — additive, then
measured, then enforced, then switched — so that each phase has one kind of failure and one kind of
gate.

**D78 is not a chain-format change, and forces no reseed.** `resolve_class_type` produces a `Val`
consumed at check time; a stored proposition encodes a class as a bare `ConstRef(iri)`
(`eigentt_type_mirror.rs:139`), never as its expanded type. So no persisted term contains a Σ-chain
and none will contain a record. This is the sharpest difference from D76, which #188 states is a
chain-format change by construction.

---

### Phase A — additive. Nothing uses it yet.

**Lands:** `Val::Record` with canonical ordering, cycle detection, readback, D47 codec arm (step 1).
`Val::Refine` with nominal equality, readback, codec arm, and **partial** subtyping (step 2).

**Behaviour change:** none. The constructs exist; no code path produces them.

**Subtyping is partial and that is expected**, not a shortfall of the phase: `Refine(R,S) <: R`
(forgetting) and `S ⊇ S′` (set inclusion) ship; the complete rule needs entailment against the layer
chain, which conversion cannot do — §3.1, registered as D76 §2.1.

**Gate:** full workspace tests unchanged; `every_shipped_ontology_document_round_trips` holds with the
new codec arms; the alongside-assertion of §7 step 1 — **field-set agreement** between a class's
record and its Σ-chain, not `eq_nf` equality.

**Reversible:** trivially. Pure addition.

**Status: complete** (`07ea5b3`, `429a0d9`). 26 tests; 1712 kernel tests green.

---

### Phase B — entailment as a kernel judgment. No measurement, no rule.

**Lands:** the `C ⊨ D` algorithm (§4) as a kernel function — field-set inclusion over the transitive
collection.

**Behaviour change:** none. It has one consumer, §3's `Refine` subtyping, which does not exist until
Phase A.

**No measurement phase, and no validation rule.** §4.1: over `subclass_of` declarations the judgment
is automatic, because `collect_properties` walks the relation transitively and a field's type is a
function of the property rather than of the class. Instrumenting it over the shipped ontologies would
return zero by construction and establish nothing. The judgment earns its place through `Refine`
subtyping between arbitrary `is_a` constraint sets, where nothing structural guarantees the inclusion.

*The previous plan made this a measure-then-enforce pair on the #194/#92 protocol, and recommended
running it first as the phase where a surprise would surface. The surprise surfaced in the design
instead.*

**It lands with no consumer, and that is the reason to do it now.** §3.1's entailment consumer — the
complete `Refine` subtyping rule — is blocked on D76. So Phase B ships a tested function nothing
calls. That is deliberate: D76 §2.1 promises the strengthening is a **one-arm change** in
`subtype_of_inner`, and that promise is only true if the algorithm already exists. Building it here
is what keeps the parked obligation cheap.

**Gate:** unit tests over constructed constraint sets, including the non-`subclass_of` cases that are
the actual use.

**Status: complete** (`program/ground.rs` — `constraint_fields`, `entails`, `conjunction_entails`).
7 tests. The load-bearing one is `a_conjunction_entails_what_no_member_does_alone`: neither
`JustName` nor `JustBreed` covers `Both`, and together they do — the case with no structural
guarantee behind it. `a_declared_subclass_entails_its_parent_automatically` pins the converse, why
there is no validation rule.

---

### Phase C — the kernel switches to records.

**Lands:** `resolve_class_type` returns `Val::Record` and takes a **resource** (step 4). Deletes
`build_sigma_chain`, the `Val::One` empty-class short-circuit, and `make_option_type`'s `recommends`
use (§5.3).

**Behaviour change:** in the type language only. The validator is still on the old path, so chain
verdicts are untouched.

**Gate:** kernel tests. The `Construct`/`PropAccess` path is the blast radius, and M1 (D75 §8a)
measured it as **unexercised by any shipped ontology** — it lives in the kernel and its tests.

**Status: complete.** Five tests flipped, all of which pinned the old representation, and each was
updated on its merits rather than as a chore:

| test | why it flipped |
|---|---|
| `a_class_with_no_requires_is_val_one_today_and_an_empty_record_after` | **written to flip.** Renamed to `..._is_now_an_empty_record_not_val_one` |
| `a_class_record_carries_the_same_fields_as_its_sigma_chain` | the Phase A gate is spent — there is no Σ-chain left to compare against. `#[ignore]`d with the reason, succeeded by `a_class_resolves_to_a_record_over_its_requires` |
| `resolve_dog_class` | asserted `Val::Sig`; now asserts the record's IRI-keyed field list |
| `readback_class_with_recommends_roundtrips` | guarded a crash caused by `Option`-typed `recommends` fields, which no longer exist; the round-trip is still pinned, and the test now also asserts no recommended-only property is a field |
| `a_class_and_its_own_unfolding_are_not_definitionally_equal` | setup asserted a Σ-chain. **The finding survived unchanged** — what a class unfolds *to* is not what makes `check` and `eq_nf` disagree |

The last one is the informative one: the δ disagreement (D75 §3.3) is a property of the two surfaces,
not of the encoding, so changing the encoding did not touch it.

**Also landed here rather than in Phase E:** projection is now keyed by the **full IRI**
(`find_record_field`), closing the local-name collision §9 records. `PropAccess` has the IRI in hand,
so carrying local names alongside would have been redundant work to defer a fix by two phases.
`advance_sigma` is deleted — a flat record has nothing to walk past.

---

### Phase D — the validator switches. The risky one.

**Lands:** Rules 1+2 and 3/8/9 become an evaluation of clause 8 against the record (step 5) — the
step that unifies §6.0's three implementations on **membership**. Rules 4–7 and 10 stay as they are
(§5.1): they are per-property refinements, not membership, and were never part of the disagreement.

**Gate: verdict parity over the full chain.** Output must be identical resource-for-resource, before
and after. 9.4M resources.

**Mechanism:** the old and new paths must run side by side to be compared, so Phase E needs a shadow
mode — compute both, report on divergence, switch only when the divergence set is empty. Building
that shadow is part of the phase, not a prerequisite to it.

**A cost the plan did not anticipate.** Routing membership through `resolve_class_type` *is* the
unification — one definition, shared with the kernel — but that call reads back and re-evaluates a
telescope, where the walk it replaces is a `BTreeSet` union. Per resource over 9.4M resources that is
a serious regression, and it is the shape of the O(chain) problems this project has hit before. The
fix is to cache per **class**, not per resource: ~894 distinct classes against millions of instances.
`Validator` gains a `class_fields` memo, sound for the same reason `RESOLVE_MEMO` is — the chain is
immutable for the duration of a pass. **Phase D's gate therefore includes cost, not only verdict
parity.**

**A shadow that compares a path against itself proves nothing.** The first cut of
`effective_record_fields` re-walked `collect_effective_properties`, which made the parity assertion a
tautology. The unified path has to actually go through `resolve_class_type`; only then are
`shadow_required_fields` (the walk) and `record_required_fields` (the record) two different code
paths whose agreement is evidence.

**The evidence the unification landed is a compiler error.** With membership routed through the
record, `collect_effective_properties` and `collect_from_class` became unreachable from the
production path and clippy failed the build on dead code. They are now `#[cfg(test)]`, surviving only
as the parity oracle — the second of §6.0's three implementations is gone, and the build says so
rather than the prose.

**Status: complete, gate met.** Four unit gates — field-set parity over every resource in the core
ontology and in the animals example, verdict parity over core, and a conditional requirement still
firing through the record path (§1.3 case (a)).

**Full-chain parity discharged `2026-08-24`** by a `--umls-all` reseed from `4ba900a`:
**9,439,633 resources, 35 loads, 0 errors** — the baseline count exactly, against a zero-error
baseline, which makes the check exhaustive rather than sampled. See
`docs/notes/reseed-timings-2026-08-24.md`.

Cost is **inconclusive and recorded as such**: 36 m 13 s against a baseline that was itself measured
twice on one commit at 34 m 40 s and 36 m 29 s. This run sits inside that spread. The memo's bound is
established by test rather than by the clock.

**Where a divergence would come from**, in likelihood order: `recommends` no longer contributing
fields (§1.1); conditional requirements evaluated through the record rather than chained into
`all_required` (§1.3); and the empty-class case, which stops being `Val::One`.

---

### Phase E — the surface opens.

**Lands:** `PropAccess` and `Construct` over records, with `Construct` returning a `Refine` per 7b
(step 6), and `Exp::EigonResource` inferring the resource's **own** record.

**A third thing it fixes, measured `2026-08-24`.** `Exp::EigonResource` currently types a resource by
`classes.first()` (`nbe/check/mod.rs:803-809`) — one arbitrarily-chosen `is_a`, with the rest
discarded from the type. **2120 of 2903 shipped resources (73 %) declare more than one `is_a`**, so
this is the common case, not an edge.

`Refine(record, S)` with `S` the **whole** `is_a` set removes the choice. That is a concrete argument
for the constraint *set* on top of the three §3 gives: nesting or a single-class former would both
have to keep picking. It also means `⋀S ⊨ D` is routinely a **genuine conjunction** — Phase B's
`conjunction_entails` has a common case waiting on it, not a rare one.

**`resource_record` includes every property, `is_a` among them.** `is_a` is itself a declared
`core:Property` (`data_type: resource_array`, `class_types: [core:Class]`), so under "everything is a
Resource" it is a field like any other and its type is `resolve_property_type`'s answer for it. There
is no redundancy with the refinement: the *field* says the resource has an `is_a` holding an array of
classes; the *refinement* says which constraints it satisfies. Structure and claim, not the same
statement twice.

**Behaviour change: user-visible.** An undeclared property becomes projectable — this is the phase
that closes D75 §3.8, and the first one an author would notice.

**Gate — corrected.** An earlier draft said the D75 §3.8 witness
(`an_undeclared_property_is_admitted_by_validation_but_cannot_be_projected`) **must flip**. Reading
it shows that is wrong: it asserts `find_sigma_field(Dog, "nickname").is_none()` — projection off the
**class type** — and that assertion is *permanently correct*. `Dog` does not declare `nickname`, so
projecting it off `Dog` must fail before and after. A class type is the declared minimum, not the
whole of what an instance carries.

What Phase E changes is projection off a **resource**. The gate is therefore a new assertion, not a
flip:

- projecting an undeclared property off a **resource that carries it** — succeeds, at the property's
  own type `T`, not `Option T`;
- projecting it off the **class** — still fails.

The existing witness keeps its class-side assertion and gains the resource-side one, so the pair
states the distinction rather than one replacing the other.

The local-name projection collision is already closed — it landed in Phase C, not here.

**Status: complete.** Three changes:

- **`resource_record`** (`program/ground.rs`) — a resource's own record, the union of the fields it
  actually carries, each at the property's declared type. Every property including `is_a`. A property
  whose type will not resolve is skipped rather than fatal: Rule 22 §c already rejects an undeclared
  key at commit, so refusing to type the rest would report one defect twice.
- **`Exp::EigonResource`** infers `Refine(resource_record(r), r.is_a())` — the resource's own record,
  refined by **every** class it declares. Falls back to the old `EigonClass(first)` shape when the
  context has no layer, since pure-mode `check` is a legitimate caller.
- **`Construct`** returns `Refine(record-of-the-given-fields, {class})` per 7b, rather than the bare
  class. Returning the class re-imposed the class's type on the instance (§3.8); returning a bare
  record would drop the nominal claim (D75 §8 Q2).

**Gate met.** `an_undeclared_property_is_projectable_off_the_resource_but_not_off_the_class` asserts
both halves — `nickname` projects off a `Dog` that carries it, at type `string` rather than
`Option string`, and still does **not** project off `Dog` itself. Its companion, which pins the class
side, is unchanged and stays true permanently. `a_resource_carries_every_class_it_declares_not_just_the_first`
pins the 73 % case: both declared classes survive into the refinement.

**A bug this phase exposed in §3's subtyping.** `Refine(R, S) <: R` — forgetting the constraint set —
is safe only against a **structural** supertype. Against a **nominal** one, `EigonClass(C)`, the
constraint set is exactly what makes the record an inhabitant of `C`, and forgetting it discards the
claim under test. The rule as first written forgot unconditionally, so
`Construct lexicon:Gene {}` (now `Refine(Record([]), {Gene})`) was compared carrier-first against
`dep`'s `EigonClass(Gene)` parameter and rejected —
`type mismatch: Record([]) ≠ EigonClass(Gene)`.

Phase C is what made it reachable: a no-`requires` class used to resolve to `Val::One` and now
carries an empty record, and **749 of 894 shipped classes have no `requires`**.

The added arm precedes forgetting: `Refine(_, S) <: EigonClass(C)` iff `C ∈ S`. Sound — a record
satisfying every constraint in `S` with `C ∈ S` satisfies `C`. Membership rather than entailment, for
the same reason the `Refine <: Refine` arm uses inclusion (§3.1). Caught by
`felicity_filter_accepts_well_typed_composition` in `kernel/tests/lexicon_validates.rs` — an
**integration** test, not a unit test.

---

### Ordering

```
A (additive) ──▶ B (entailment) ──▶ C (kernel) ──▶ D (validator) ──▶ E (surface)
```

**No phase forces a reseed.** The reseed risk previously attributed to Phase C came from a validation
rule that §4.1 removed; with no rule over `subclass_of`, no bootstrap ontology has to change. D78 is
additive to the chain throughout.

B follows A because its only consumer is `Refine`. D is the sole phase touching 9.4M resources; E is
the sole phase changing what an author sees.

## 8. Gates

Per phase (§7), plus these standing across all of them:

| gate | when |
|---|---|
| full workspace tests + clippy clean | every phase |
| `every_shipped_ontology_document_round_trips` | A (new codec arms), and unchanged after |
| entailment unit tests over non-`subclass_of` constraint sets | B |
| kernel tests; `Construct`/`PropAccess` blast radius | C |
| field-set + verdict parity over the shipped ontologies | D — done |
| **verdict parity over 9.4M resources** | D — **outstanding**, needs the reseeded store |
| membership cost does not regress (per-class cache) | D |
| undeclared property projectable off a **resource**, still not off the **class** | E |
| local-name projection collision closed | **C**, not E — `PropAccess` already has the IRI |
| parse gate and WRN demo unchanged | D and E — the two phases that could move them |

The parse gate and the demo are listed only against D and E because A–C change nothing a chain
consumer observes.

## 9. Open

### Settled while auditing

**Cycle detection lives in a validation rule, with the kernel constructor as defence in depth.** The
dependency edges come from `class_types` references and `when_property` (§1.3) — both *ontology*
data — so a cycle is a malformed **class declaration**, detectable at commit with no term in hand.
That puts the primary gate on the commit path where declarations are already checked. The
`Val::Record` constructor returns an error rather than panicking, so a hand-built record cannot
smuggle one past. Detection is free: the topological sort of §1 finds cycles as a by-product.

**`find_sigma_field` is replaced by IRI-keyed lookup — and that closes a latent defect.** Today
`Exp::PropAccess` projects by **local name**: `check/mod.rs:768` takes `prop.local_name()` and
`find_sigma_field` compares it against `Clos.patt` as `Patt::Var(name)` (`:1112`). So
`urn:eigenius:a:name` and `urn:eigenius:b:name` are the same field to a projection. `Vec<(Iri, Clos)>`
is keyed by the full IRI and the collision disappears. Cost is unchanged — a linear scan over a
field list, where the Σ-walk was also linear, and class field counts are small.

**`Val::Sig` survives.** `Exp::Times` is live in the DCG (`dcg/holes.rs:53,108`,
`dcg/pretty.rs:101`, `dcg/rules/constructions.rs:1223`) and encodes to an anonymous `Sig`. Records
are *named* field sets; anonymous pairs are a different construct that records do not subsume. `Sig`
stays for that use, and the question is answered rather than deferred.

**Level computation** — §2 now states the instantiation rule.
**`Refine` and cumulativity** — §3 now states it, plus `Refine(R, S) <: R`.

### Genuinely deferred

**The empty-record floor** (§2). Argued from proof irrelevance, and low-stakes on inspection: a
record of *proofs* — a conjunction — would want `Prop`, but conjunctions in Eigenius are inductives
(`Data`), not records. Records are for resources, and resources are not proofs. Revisit only if a
`Prop`-valued record is ever wanted, at which point the floor becomes a per-record decision rather
than a constant.

> **Reviewed `2026-08-25`: the condition has not fired.** No `Prop`-valued record exists anywhere in
> the tree. Still correctly deferred.

**Antichain normalization** (§3). `Refine(R, {Pup, Dog})` with `Pup ⊨ Dog` keeps the redundant member,
because dropping it would discard a declared fact that 9c/10d keep authoritative. The consequence —
that type and `Refine(R, {Pup})` have identical inhabitants and are not equal — is what nominal
identity means. Revisit only if that inequality causes friction in practice.

> **Reviewed `2026-08-25`: no friction, but D76 Phase D sharpened the consequence.** Entailment is
> decidable in conversion now, so the two types are **mutually subtypes** — `{Pup,Dog} ⊇ {Pup}` gives
> one direction by inclusion, `⋀{Pup} ⊨ Dog` the other by entailment. Before Phase D only the first
> held, so this paragraph was written against a weaker state than the code is now in.
>
> Mutual subtyping without equality is not friction: a subtyping system routinely has distinct types
> that admit each other, and the declared set being part of identity is exactly what §9 chose to keep.
> Pinned by `readback::refine_semantics::a_redundant_refinement_member_is_mutually_a_subtype_but_not_equal`
> so the state is a test rather than prose. Still correctly deferred.

## 10. References

- D75 §3.7, §3.8, §3.9 (the symptoms), §6.0–§6.7 (the constraint reading), §8 Q7/Q9/Q10 (the
  inherited decisions), §8a (M1)
- `references/publications/Cooper-2023-TTR-appendix-1.pdf` — A11.2 clauses 7–8, A11.6 dependency
  families, A6 singleton types
- Cooper, *"So what's all this structure good for?"*, CSTFRS 2021 §2.3 — Σ-types vs record types
- #215 (tracker), #225–#228 (the filed defects)
