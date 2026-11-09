---
title: "Typed Knowledge over Constructive Institutions"
description: "Eigenius, Volume I: Theory. The kernel type theory, justification logic, institutions, and knowledge-graph theory of Eigenius, with a Lean 4 formalization. Preprint on Zenodo."
---

**Typed Knowledge over Constructive Institutions**
*Eigenius, Volume I: Theory*

Hans-Martin Will

Preprint, 249 pages · [doi:10.5281/zenodo.23884679](https://doi.org/10.5281/zenodo.23884679)
· Lean 4 formalization: [github.com/eigenius/theory](https://github.com/eigenius/theory)
· CC BY-NC 4.0

The DOI resolves to the current version of the book on Zenodo.

## Abstract

Type theories designed as foundations for mathematics or for program
verification need only that a proposition holds; a theory with proof
irrelevance in conversion erases, at the kernel level, how it was
established. A persistent knowledge store, by contrast, must retain
that record: who established a claim, by which derivation or external
tool, and under which assumptions. This book, the first volume of a
series on Eigenius, presents the theory of Eigenius—a store of typed
knowledge over constructive institutions—and develops its metatheory
from that requirement.

The kernel theory is a dependent type theory with a cumulative universe
hierarchy, an impredicative universe of propositions, inductive
families with recursors, and an intensional identity type. It departs
from the theories of current proof assistants in two ways. First,
proof irrelevance is a provable fact with an explicit witness rather
than a rule of conversion, so proofs of a single proposition are
provably equal yet remain distinct terms. Second, definitional equality
is joinability under an orthogonal rewriting system—making it
confluent, injective on type formers, and decidable by a type-free
test—while a separate subsumption judgment covers universe
cumulativity, function types covariant in their codomains, and
declared class inclusions.

The metatheory is developed in five parts. First, a set-theoretic model
that interprets terms directly proves consistency, relative to a single
obligation on the global environment, for two bidirectional fragments
that the kernel implements and, by translation, for the declarative
theory. Second, a justification logic over the kernel's propositions
records evidence together with its support. Third, the theory of an
environment is organized as an institution, so that extending the store
is a signature morphism and a verdict obtained in an external system
enters along a comorphism. Fourth, a typed knowledge-graph theory with
record types and width subtyping gives the store its classes,
resources, and refinements. Finally, the procedures that decide the
bidirectional fragments—conversion by reduction to weak-head normal
form and normalization by evaluation—are stated with their correctness
relative to the termination of reduction. A Lean 4 formalization
accompanies the volume, and an appendix maps it to the definitions and
results in the text. Later volumes of the series treat the
implementation, the language pipeline, and the system.

## The Lean formalization

[EigeniusTheory](https://github.com/eigenius/theory) has one module
per section of the book:

| Book | Lean namespace | Content |
|---|---|---|
| Ch. 2 | `EigeniusTheory.TypeTheory` | The kernel type theory, reduction and confluence, the declarative and bidirectional systems, inductive declarations with their recursors, consistency by a stratified set-theoretic model |
| Ch. 3 | `EigeniusTheory.Justification` | The justification term algebra with its evidence function and support, provenance semirings, conservativity, consistency, and non-factivity of the justification extension |
| Ch. 4 | `EigeniusTheory.Institutions` | Institutions, entailment systems, π-institutions, proof calculi, the kernel calculus and the translation lemma, the kernel theory as a logic, Meseguer's `E†` |
| Ch. 5 | `EigeniusTheory.KnowledgeGraphs` | Knowledge graphs as typed data: inheritance, membership as a greatest fixed point, subsumption, refinement, feature structures |
| Ch. 6 | `EigeniusTheory.Checking` | Weak-head reduction and the conversion test, soundness and completeness of the algorithmic systems, the type checker, normalization by evaluation |

Every declaration is proved without `sorry` and depends only on Lean's
standard axioms: `propext`, `Classical.choice`, and `Quot.sound`.
Appendix A of the book maps each definition and result to the
declaration that formalizes it, and the repository's README lists the
results that are not formalized. The Lean sources are licensed under
Apache 2.0.

## Where to go next on this site

- **Justification logic** —
  [the concept page](/concepts/justification-logic/) relates
  Chapter 3 to the four warrant categories.
- **Institutions** —
  [ESL §9 — Institutions](/docs/esl/09-institutions/) and the
  [composition guide](/docs/composition/) document the implemented
  counterpart of Chapter 4.
- **The type theory** —
  [ESL §7 — Type-theory primer](/docs/esl/07-type-theory-primer/) and
  [ESL §6 — Resources, types, and the layer](/docs/esl/06-resources-types-and-the-layer/).

## How to cite

```bibtex
@book{Will2026Eigenius1,
  author    = {Will, Hans-Martin},
  title     = {Typed Knowledge over Constructive Institutions},
  series    = {Eigenius},
  volume    = {I},
  publisher = {Zenodo},
  year      = {2026},
  doi       = {10.5281/zenodo.23884679},
  url       = {https://doi.org/10.5281/zenodo.23884679},
}
```
