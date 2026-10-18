---
title: "Eigenius: A Typed Knowledge-Graph DBMS"
description: "Epistemic stratification and institution-mediated reasoning. Preprint on arXiv (2608.04457), cs.DB, 5 August 2026."
---

**Eigenius: A Typed Knowledge-Graph DBMS with Epistemic Stratification
and Institution-Mediated Reasoning**

Hans-Martin Will, Allen L. Brown Jr., Matthew Fuchs

Preprint, 5 August 2026 · [arXiv:2608.04457](https://arxiv.org/abs/2608.04457)
· [PDF](https://arxiv.org/pdf/2608.04457)
· [doi:10.48550/arXiv.2608.04457](https://doi.org/10.48550/arXiv.2608.04457)
· cs.DB (primary), cs.AI, cs.LO · CC BY 4.0

## Abstract

As "AI Scientists" emerge to drive research via the Model Context
Protocol (MCP), systems relying on ephemeral scripts will fail. The
sheer scale of stateful, interconnected evidence requires a
machine-walkable warranty grounded in a purpose-built database
architecture. Eigenius is an open-source, typed knowledge-graph DBMS
built on a single premise: answering the audit question ("what do you
know, and what is your warranty?") requires a unified kernel. By
tightly coupling the type system, storage engine, and integration
protocol, Eigenius turns data provenance into a structural invariant
rather than a property reconstructed across subsystem boundaries. The
kernel rests on three pillars: a dependent type theory woven through
the core, institutions acting as strongly typed integration
boundaries, and a content-addressed immutable storage layer. On this
foundation, epistemic status (declared/observed/derived/verified) is
enforced as a strict commit-time invariant. Cross-system translations
(comorphisms) are checked at commit and materialized directly into the
graph as durable, first-class resources. To eliminate O(N²) polystore
bottlenecks, shared on-chain intermediate representations (IRs)
collapse multi-system translations to identity. Crucially, this
architecture unifies both domains of scientific epistemology: it
relies on justification logic for empirical science, while embedding a
fast, in-process term checker to safely evaluate formal mathematical
proofs (via Lean 4) without IPC overhead. In an end-to-end
recomputation of a published Nature study from fragile scripts to a
materialized evidence graph, all 52 derived conclusions hold from
pinned data, surfacing four machine-checked discrepancies in the
original study.

## Where to go next on this site

Each of the paper's pillars has a narrative counterpart here, and the
platform chapters document the implementation the paper describes:

- **Epistemic stratification** — the four warrant categories
  (declared / observed / derived / verified) enforced at commit:
  [Concepts](/concepts/), and
  [justification logic](/concepts/justification-logic/) for the
  Artemov calculus the certificates instantiate.
- **Institutions as typed integration boundaries** —
  [ESL §9 — Institutions](/docs/esl/09-institutions/) for the surface,
  and the [composition guide](/docs/composition/) for comorphisms and
  the shared-IR argument against the O(N²) polystore.
- **The dependent type theory** —
  [ESL §7 — Type-theory primer](/docs/esl/07-type-theory-primer/) and
  [ESL §6 — Resources, types, and the layer](/docs/esl/06-resources-types-and-the-layer/),
  which is the bridge between the resource graph and the kernel.
- **The Lean 4 term checker** —
  the [Lean institution tutorial](/docs/platform/lean-institution/),
  which runs in-process, no IPC.
- **The end-to-end recomputation** —
  the [drug-screening example](/examples/drug-screening/) is the same
  audit-chain shape at a size you can read in one sitting.

## How to cite

```bibtex
@misc{will2026eigenius,
  author        = {Will, Hans-Martin and Brown, Jr., Allen L. and Fuchs, Matthew},
  title         = {Eigenius: A Typed Knowledge-Graph {DBMS} with Epistemic
                   Stratification and Institution-Mediated Reasoning},
  year          = {2026},
  eprint        = {2608.04457},
  archivePrefix = {arXiv},
  primaryClass  = {cs.DB},
  doi           = {10.48550/arXiv.2608.04457},
  url           = {https://arxiv.org/abs/2608.04457},
}
```

## Open source

The code the paper describes is at
[github.com/eigenius/eigenius](https://github.com/eigenius/eigenius).
Because the platform is content-addressed, a citation can pin the
exact chain state a result was computed from rather than a repository
snapshot.
