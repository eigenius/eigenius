---
title: "A Proof-Relevant Type Theory for Verified Science"
description: "Typed Knowledge over Constructive Institutions: the type-theoretic and institutional foundations of Eigenius at conference length. In preparation for submission to LICS."
---

**Typed Knowledge over Constructive Institutions: A Proof-Relevant Type
Theory for Verified Science**

Hans-Martin Will, Allen L. Brown Jr., Matthew Fuchs

> **Coming soon.** This paper is in preparation for submission to LICS.
> We are not circulating the manuscript yet. This page will be updated
> with the abstract and preprint when the paper is ready to share.

## What the paper covers

The paper presents the theory of the
[monograph](/research/papers/typed-knowledge-over-constructive-institutions/)
at conference length:

- **The kernel.** A dependent type theory with cumulative universes, an
  impredicative universe of propositions, inductive families, and
  intensional identity types. Proof irrelevance is a propositional
  axiom with an explicit witness, not a rule of conversion, so
  independent derivations of one claim remain distinct terms that the
  store can inspect, compare, and retract. Definitional equality is
  joinability under an orthogonal rewriting system.
- **Subsumption.** Universe cumulativity, codomain-covariant function
  subtyping, and ontology class inclusions in one coercion-free
  judgment, whose bidirectional fragments are sound and complete for
  declarative typing.
- **Institutions, justification, and knowledge graphs.** Kernel
  theories and external verifiers organized as an institutional
  hierarchy with explicit axiom policies; a justification logic with
  provenance support; Cooper's Type Theory with Records for open-world
  knowledge graphs.
- **Consistency and mechanization.** A stratified set-theoretic model
  in Grothendieck universes, and the metatheory mechanized in Lean 4.

## In the meantime

Every definition and theorem of the paper is developed in the
[monograph](/research/papers/typed-knowledge-over-constructive-institutions/)
and formalized in [EigeniusTheory](https://github.com/eigenius/theory).
