# TTR as the model for classes — and what it says about universe polymorphism

Read from the vendored formal appendix, `references/publications/Cooper-2023-TTR-appendix-1.pdf`
(Cooper 2023, Appendices A1–A11), plus Cooper's CSTFRS 2021 workshop paper *"So what's all this
structure good for?"*. Written `2026-08-23`.

Two findings, one of which corrects a premise this repository acted on.

## 1. Universe polymorphism: the formal system does not have it, the working notation assumes it

> **CORRECTED `2026-08-23`, after reading Chapter 1 §1.4.3.3.** The first version of this section
> concluded flatly that "TTR does not use universe polymorphism" and that #188's remaining half had
> therefore lost its justification. That was right about the formal system and **wrong as a guide to
> what an implementation needs**. Both readings are kept below, because the difference between them
> is the whole point.

A10 ("The type `Type` and stratification") defines an intensional system of complex types as a
family of quadruples **indexed by the natural numbers**, with:

```
2. for each n,  Typeⁿ ⊆ Typeⁿ⁺¹                    — cumulativity
4. for each n>0, Typeⁿ ∈ Typeⁿ⁺¹                   — stratification
5. for each n>0, T :ⁿ Typeⁿ iff T ∈ Typeⁿ⁻¹
```

The `for each n` / `for any n` clauses are **metatheoretic schemas** defining the family. They are
not object-level quantification over levels: there is no level variable in a term, no declaration
carrying universe parameters, and no instantiation of a declaration at a level.

The two things TTR *calls* polymorphism are different mechanisms:

- **A4, partial function types.** `f : (T₁ ⇀ T₂)` iff there is some `T′` with `f : (T′→T₂)` and
  every `a : T′` satisfies `a : T₁`. That is polymorphism in the **domain**, driven by subtyping.
- **A3.1, polymorphic predicate signatures.** `Arity(P)` is a *set* of argument-type tuples —
  arity overloading.

Neither requires `uparams` or level arguments at reference sites.

**EigenTT already implements A10.** #188's first half gave `Exp::Sort(Level)` with `Zero`/`Succ`,
cumulativity through `Level::leq` in `subtype_of_inner`, and stratification as
`Sort(l) : Sort(Succ(l))` in `check_infer`. Measured against A10 clauses 2, 4 and 5, that is the
whole of what TTR's universe treatment needs.

**Consequence for #188.** The first half was worth doing on its own evidence — it removed the
`Level` `Ord` derive (a structural comparison masquerading as the universe order), `check_type`'s
`check(a, Sort(1))` fallback, and `result_sort`'s string grammar. None of that depended on Cooper.
But the **second half** — declaration-level `uparams` and level arguments at ~583 reference sites —
has now lost its stated justification. The TTR trigger is already satisfied by what shipped.

### What Chapter 1 adds, and why it reverses the conclusion

Two things the appendix alone does not convey.

**Cooper identifies `Typeⁿ` with Martin-Löf universes outright** (Ch. 1, fn. 2):

> "In Martin-Löf type theory, types of types are called **universes**. This is, however, potentially
> a confusing terminology for a theory relating to the kind of model theory which has been used in
> linguistics where 'universe' has a different meaning."

So the hierarchy is a universe hierarchy; only the word is avoided.

**The working notation is level-IMPLICIT, and he says so twice** (§1.4.3.3):

> "For everyday working purposes we will assume that this is the system we have and **ignore** the
> fact that this is bringing us into danger of introducing Russell's paradox."

> "We will in future assume that our type systems are stratified in this way **without mentioning it
> explicitly for the most part**."

That is the crux. In the text, `Type` is written unindexed and treated as though `Type : Type`; the
stratification is the background story that makes it safe. **Writing `Type` unindexed and letting
the order be inferred is exactly the ergonomic universe polymorphism buys**, achieved in prose by
deferring the indices.

**An implementation cannot defer them.** When `Type` appears in a source file, something must
resolve it to an order, and there are two options:

1. **A concrete order per site** — a literal reading of A10. Then a record-type definition cannot be
   reused at another order; it must be restated, once per order it is needed at.
2. **Level variables and instantiation** — object-level polymorphism, which is what #188's remaining
   half proposes.

Cooper's construction is (1) in the metatheory and reads like (2) on the page. An implementation
that wants his notation needs (2).

### So where does this leave #188

**The trigger stands, restated.** Not "Cooper uses universe polymorphism" — he does not, formally —
but "TTR's working notation is level-implicit, and an implementation of it needs either polymorphism
or per-site concrete orders". That is a real motivation and it is the one to record, because it is
the one that survives reading the source.

EigenTT today is stricter than Cooper's *working* system and matches his *formal* one: it has
`Sort(l) : Sort(Succ(l))`, never `Type : Type`. Nothing in the current chain writes a level-generic
definition, because nothing can. The question the TTR work will actually pose is whether a
chain-resident record type needs to be usable at more than one order — and if it does, option (1)
means restating it per order in the ontology, which is the cost polymorphism removes.

## 2. A11.2 is the formal statement of a defect in `resolve_class_type`

D18 §494 and D62 §3 already say a class *is* a record type — D62 says "**exactly** the
Class-as-record-signature Eigenius uses". The appendix makes precise what the current encoding
loses.

**Record types are built by set union, and witnessed by field membership** (A11.2, clauses 7–8):

```
7. if R ∈ RType, ℓ does not occur in R, T ∈ Type,  then  R ∪ {⟨ℓ, T⟩} ∈ RType
8. r : R ∪ {⟨ℓ, T⟩}   iff   r : R,  ⟨ℓ, a⟩ ∈ r,  and  a : T
```

Clause 8 gives subtyping directly: a record carrying *extra* fields still witnesses, because
witnessing tests membership rather than shape. And union is order-free, so a record type has no
field order to get wrong.

`resolve_class_type` instead walks `requires` + `recommends` and builds a right-nested
**`Val::Sigma` chain**. Cooper names this exact substitution in the 2021 paper §2.3:

> "as TTR record types are **sets of fields** there are **several Σ-types which intuitively
> correspond to a single record type** … this equivalence is **not directly derivable** from the
> characterization of Σ-types. **These Σ-types do not have a witness in common.** In TTR we use
> record types in place of Σ-types."

Two consequences for Eigenius, neither previously written down:

- **The encoding is well-defined only by accident.** Properties live in a `BTreeMap` keyed by IRI,
  so the Σ-chain comes out in a canonical order. A data-structure choice is standing in for a
  theory. Nothing states the invariant, and nothing would catch its loss.
- **Nominal `subclass_of` is forced, not chosen.** `collect_properties` walks `subclass_of`
  transitively, so a subclass's sorted chain has a different *shape* from its parent's, and by
  Cooper's observation the two share no witness. Structural subtyping is unavailable to a Σ-chain;
  it is free for a record type.

## 3. Two further mappings the appendix supplies

- **A6 singleton types are manifest fields.** `T_a`, with `b : T_a iff b : T and a = b` — a field
  pinned to a value. Eigenius has this in disguise: `core:has_value` on a
  `core:ConditionalRequirement`, and `allows_only` enumerations. Both are singleton or join types
  encoded ad hoc.
- **A11.6 supplies the dependency discipline Eigenius lacks.** Generalizing a record type to a path
  must carry its **dependency family** `pathsπ(T)` — closed under both "depends on" directions.
  The 2021 paper puts it plainly: "If we remove a field on which another field depends we have to
  remove the dependent field as well." Eigenius has dependent properties (a `class_types` that
  references an earlier field) and **no check that a `subclass_of` respects them**.

## 4. What this changes

**Sequencing.** #188's remaining half proposes level arguments on five `Exp` variants, two of which
(`EigonClass`, `EigonAxiom`) this analysis says should become one record/`Const` former. Building
polymorphism onto shapes we intend to collapse is still the wrong order — but that is now an
argument about ORDER, not about whether to build it at all. §1 no longer says the justification
fails; it says the justification is different from the one recorded in N3 §5a.

**The consolidation question splits cleanly.** The class half has a reference (TTR) and a
demonstrable defect. The inductive half (`InductiveType` / `InductiveCtor` / `InductiveRec`, fused
former-and-arguments) is a nanoda-shaped question, constrained by `EvalCtx::Pure` having no layer to
resolve a `Const` against — Cooper says nothing about it.

**What is NOT settled here.** Adopting record types is not a representation swap: `subclass_of` is
nominal and load-bearing for Rule 22, `class_types` and institution dispatch. Moving to
field-inclusion subtyping changes what "is a" means chain-wide. That is the question a follow-on
note has to open with.
