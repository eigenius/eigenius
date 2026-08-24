# D78 — Resources as records

Status: draft. Written `2026-08-24` on `p2-residue`.

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

Five things it must settle, and does: the record's representation (§1), how its level is computed
(§2), the refinement type 7b needs (§3), the entailment judgment 10d needs (§4), and what changes in
each of the three implementations (§5).

## 1. `Val::Record` — a canonically-ordered dependent telescope

### The tension

A11.2 builds record types by **set union**, which is order-free. A11.6 dependency families let a
later field's type mention an earlier field, which requires *some* order. Both are required; neither
can be dropped.

### What exists already

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
Record(Vec<(Iri, Clos)>)     // canonical order; Clos.patt names the binder
```

A telescope, like `Sig`, but with the fields held in a **canonical order**: the topological order
induced by the dependency relation, ties broken by IRI. That is:

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

## 3. `Val::Refine` — the type 7b returns

```rust
Refine(Box<Val>, Iri)        // the record type, and the class it satisfies
```

**Restricted to a named class constraint, not an arbitrary predicate.** That is what keeps

- **nominal identity (D75 §8 Q2)** — `Refine(R, Alpha)` and `Refine(R, Beta)` differ even when `R` is
  shared, which is exactly the collapse Q2 forbids;
- **conversion decidable** — entailment between named constraints is §4's algorithm, whereas
  arbitrary predicates would need a prover.

| judgment | rule |
|---|---|
| equality | `Refine(R, C) ≡ Refine(R′, D)` iff `R ≡ R′` and `C = D` (**by IRI** — nominal) |
| subtyping | `Refine(R, C) <: Refine(R′, D)` iff `R <: R′` and `C ⊨ D` (§4) |
| readback | `Exp::Refine(Box<Exp>, Iri)` |
| D47 codec | a `Refine` ctor beside the existing `Sig`/`Pi` arms (`eigentt_type_mirror.rs:111-127`) |

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

**Three independent reasons this is needed** (D75 §6.0, §8 Q10):

1. It closes a hole: nothing today compares a subclass's property declarations against its parent's,
   so a subclass may redeclare a property at an incompatible type unchecked.
2. It supplies §3's subtyping side condition.
3. **The query engine already assumes it.** `class_with_subclass_closure` returns instances declared
   at a subclass as answers for the parent, which is sound only if the subclass entails it. Today
   that holds by coincidence — `collect_properties` and Rules 1+2 walk `subclass_of` transitively, so
   an instance of `Pup` was in fact checked against `Dog`'s requirements. Under explicit field sets
   the coincidence disappears and the closure becomes unsound unless entailment is checked.

Point 3 is the one that makes this rule **load-bearing rather than hygienic**, and it is why the rule
lands in the same change as the record model, not after it.

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
| validator | Rules 1+2, 3–10 | become an **evaluation of clause 8** against that record: same checks, same verdicts, one definition instead of an independent transitive walk of `requires` |
| query | `class_with_subclass_closure` + `scan_chain` | **unchanged**. The index is keyed on the *declaration*, and clause 8 changes what "satisfies C" means, not how membership is enumerated |

Plus, in the kernel: `PropAccess` and `Construct` over records, which is what closes D75 §3.8 — an
undeclared property becomes projectable because a resource's type is the union of its own fields.

**`is_a` is unaffected as a surface.** It stays a declared name whose meaning is clause 8 (D75 §6.4).
Nothing becomes inferred; membership is not computed from structure.

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

## 7. Build order

1. **`Val::Record`** (§1) with canonical ordering, cycle detection, readback, and the D47 codec arm.
   No behaviour change yet — construct it alongside the Σ-chain and assert they agree.
2. **`Val::Refine`** (§3) with its equality, subtyping, readback and codec arms.
3. **Entailment + its validation rule** (§4). Runs against the existing `subclass_of` declarations, so
   it is measurable before it is enforced — instrument, log, count, then enforce, per the #194/#92
   protocol. A shipped ontology that fails entailment is a finding, not a blocker.
4. **`resolve_class_type` returns a record** and takes a resource (§5).
5. **The validator's rules become clause-8 evaluation** (§5). The step that unifies the three
   implementations, and the one to gate hardest: the verdicts must be identical over the full chain
   before and after.
6. **`PropAccess` / `Construct` over records**, closing D75 §3.8, with `Construct` returning a
   `Refine` per 7b.

Step 3 is where a surprise would surface, and it is cheap to run early — it needs only the entailment
algorithm, not the record former.

## 8. Gates

- Every existing kernel test, unchanged.
- **Verdict parity over the full chain**: the validator's output before and after step 5 must be
  identical, resource for resource. This is the step that touches 9.4M resources.
- `every_shipped_ontology_document_round_trips` — the codec arms of steps 1 and 2 must round-trip.
- The parse gate and the WRN demo, unchanged.
- The D75 §3.8 witness (`an_undeclared_property_is_admitted_by_validation_but_cannot_be_projected`)
  **must flip** at step 6. It pins current behaviour and names D75 §3.8; when the projection works,
  it fails, and that is the signal.

## 9. Open

- **The empty-record floor** (§2) is argued from proof irrelevance. If `Prop`-valued records are ever
  wanted, the floor has to become a per-record decision rather than a constant.
- **`Refine` nesting.** A record satisfying two classes — `is_a` is a list — is `Refine(Refine(R, C), D)`
  or `Refine(R, {C, D})`. The second is flatter and matches `is_a`'s shape; the first composes more
  obviously with §3's subtyping rule. Not settled.
- **Whether `Val::Sig` survives at all** once records exist, or whether the anonymous pair type is the
  only remaining use.

## 10. References

- D75 §3.7, §3.8, §3.9 (the symptoms), §6.0–§6.7 (the constraint reading), §8 Q7/Q9/Q10 (the
  inherited decisions), §8a (M1)
- `references/publications/Cooper-2023-TTR-appendix-1.pdf` — A11.2 clauses 7–8, A11.6 dependency
  families, A6 singleton types
- Cooper, *"So what's all this structure good for?"*, CSTFRS 2021 §2.3 — Σ-types vs record types
- #215 (tracker), #225–#228 (the filed defects)
