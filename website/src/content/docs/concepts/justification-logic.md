---
title: Justification logic
description: The modal calculus that underlies the four-warrant taxonomy. For the academically curious — the chain instantiation of Artemov's logic of justification.
---

> **This page is being written.** It will be expanded with a
> formal treatment and the relationship to the four-warrant
> taxonomy. In the interim, the
> [CACM paper §"Justification logic"](/research/papers/cacm-typed-reasoning/)
> is the bridge from Artemov's calculus to the chain-resident form.

## Summary

The four-warrant taxonomy of
[Concepts](/concepts/) is a chain-instantiated form of
Artemov's *justification logic* — a modal calculus that makes
explicit *which evidence justifies each modality*. Each
chain-resident proposition is paired with a typed
*justification term*: an explicit, structural accounting of how
the proposition was reached. The kernel's type-checker verifies
the composition of justification terms; what gets committed is
the agent's reasoning made structural, not the agent's prose.

The reasoning institution's `JustifiedBy` inductive type is the
chain-resident formalisation: it pairs a justification term
(`DeclaredEvidence(iri)`, `ObservedEvidence(iri)`,
`DerivedEvidence(iri)`, `VerifiedEvidence(iri)`, and their `App`
and `SpecStr` compositions) with the proposition the term
justifies. The kernel verifies that the pairing is well-formed.

## References

- Artemov, S. *The logic of justification.* The Review of
  Symbolic Logic 1(4):477–513, 2008.
- The CACM paper, §"The Compiler for AI Thought" → "Justification
  logic"

## See also

- **[Concepts overview](/concepts/)** — the four-warrant taxonomy
  at user-facing depth
- **[CACM paper](/research/papers/cacm-typed-reasoning/)** — the
  bridge from Artemov to the chain
