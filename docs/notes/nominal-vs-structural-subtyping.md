# Nominal `subclass_of` vs structural field-inclusion

Split out of the TTR investigation (`fa03128..276fa5c`) because it is **independent** of the thesis
in D75. D75 argues the typing environment is missing from the judgment; this question is about what
"is a" means, blocks nothing in D75's build order, and is a genuine trade rather than a defect.

Written `2026-08-23`, split out `2026-08-24`.

## What Cooper defines

Chapter 1 §1.4.3.5 gives the relation and a syntactic criterion:

> "T₁ is a subtype of T₂ (in symbols T₁ ⊑ T₂) just in case for any a, a : T₁ implies a : T₂, **no
> matter what is assigned to the basic types and ptypes**."
>
> "We can tell that (53b) is a subtype of (53a) simply by the fact that **the set of fields of (53a)
> is a subset of the set of fields of (53b)**."

More fields ⇒ subtype. The relation is **modal** — it must hold in every possibility of a modal
system (§1.4.3.5, A9), so it is necessary rather than contingent on one model.

## What Eigenius has

Field inclusion holds **by construction**: `collect_properties` walks `subclass_of` transitively, so
a declared subclass inherits its parent's `requires` and its resolved type contains the parent's
fields. `class ex:Pup : ex:Dog { }` with `Dog` requiring two properties and `Pup` declaring none
resolves to the *same* Σ-type as `Dog`. No rule compares property sets because none needs to.

## The two directions

| | Eigenius | TTR |
|---|---|---|
| direction | **nominal-generative** — declare `subclass_of`, inheritance follows, inclusion becomes true | **structural-derived** — have the fields, the relation follows |
| two classes with identical fields, no declaration | **unrelated** | mutual subtypes |
| "any record with a `name` field" | inexpressible — a class must be named | a record type |
| same structure in two ontologies | unrelated until a comorphism bridges them | already related |

Rows three and four are the ones that cost. Cross-vocabulary alignment is a standing problem here —
the institution/comorphism apparatus carries part of that load — and under a structural reading two
ontologies describing the same shape are related without anyone declaring it.

## Why it is a trade, not a defect

Nominal subtyping is *intentional*, and `subclass_of` being a **declaration** is what makes
institution dispatch and `allows_only` decidable by looking at one resource. `subclass_of` is
load-bearing for Rule 22, `class_types`, and dispatch. Moving to field-inclusion subtyping is not a
representation swap — it changes what "is a" means chain-wide.

## Interaction with D75 §3.7

D75 §3.7 records that `resolve_class_type` builds a right-nested `Val::Sigma` chain where A11.2
builds a record type by set union, and that Cooper's observation — *"These Σ-types do not have a
witness in common"* — means **structural subtyping is unavailable to a Σ-chain and free for a record
type**. So the encoding question and this question are coupled in one direction: adopting record
types makes structural subtyping *possible*; it does not make it *chosen*.

## A hypothesis that did not survive

The tempting claim is that structural class identity would fix D75 §3.4 (witness credit surviving a
rebinding), because a proposition mentioning `C` would then hash over `C`'s structure and stale
credit would be invalidated by construction.

It over-invalidates. Any field added to `C` moves the hash, including fields the proposition never
mentions — discarding credit on edits that cannot affect the proposition's truth, and destroying the
stability of a proposition's identity under irrelevant vocabulary edits. The real target is finer
than either extreme: invalidate when a rebinding changes something the proposition **depends on**.
That is a dependency question (cf. A11.6 dependency families, D75 §6), not an identity question.

## Open

- Would structural subtyping be *additional* to `subclass_of` rather than replacing it — a derived
  relation queryable alongside the declared one?
- What does Rule 22 mean if `is a` has two sources?
- Does cross-ontology alignment actually want this, or does it want the comorphism to stay explicit?
