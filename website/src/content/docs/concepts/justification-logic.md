---
title: Justification logic
description: The modal calculus that underlies the four-warrant taxonomy. For the academically curious — the chain instantiation of Artemov's logic of justification.
---

> **This page is being written.** It will be expanded with the
> relationship to the four-warrant taxonomy. Chapter 3 of the
> [monograph](/research/papers/typed-knowledge-over-constructive-institutions/)
> gives the formal treatment, formalized in Lean 4; the
> [Eigenius systems paper](/research/papers/typed-knowledge-graph-dbms/)
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
- Will, H.-M., Brown Jr., A. L., Fuchs, M. *Eigenius: A Typed
  Knowledge-Graph DBMS with Epistemic Stratification and
  Institution-Mediated Reasoning.* arXiv:2608.04457, 2026.
- Will, H.-M. *Typed Knowledge over Constructive Institutions.*
  Eigenius, Volume I: Theory. Zenodo, 2026.
  doi:10.5281/zenodo.23884679.

## See also

- **[Concepts overview](/concepts/)** — the four-warrant taxonomy
  at user-facing depth
- **[The monograph](/research/papers/typed-knowledge-over-constructive-institutions/)** —
  Chapter 3, the justification logic over the kernel's propositions
- **[Eigenius systems paper](/research/papers/typed-knowledge-graph-dbms/)** —
  the bridge from Artemov to the chain
