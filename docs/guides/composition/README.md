# Composing institutions

This guide is about what happens when **multiple institutions cooperate** —
the cross-cutting story none of the per-host chapters tell on their own.
A single WASM-hosted institution chapter ([platform §10](../platform/10-wasm-institutions.md))
or a single substrate-hosted institution chapter
([platform §11](../platform/11-runtime-substrate.md)) covers what one
institution looks like; this guide covers what the *system* looks like
when several institutions, each with their own dispatch roles and their
own host runtime, share data through the chain and bridge across each
other through declared comorphisms.

The canonical worked example throughout is the
[`kinase-institutions.json`](../../../notebooks/examples/kinase-institutions.json)
notebook — five Julia institutions, three cross-institution comorphisms,
two storylines (Catalyst → DiffEq forward simulation; Symbolics → JuMP
parameter fit), the [D14 §9.3](../../design/d14-institution-realisation.md)
chain-reinsertion contract closed through both the ESL and EigenQL
surfaces. Each chapter pulls a slice of that example to ground the
concept it covers.

## How to read this guide

If you've read the [ESL](../esl/README.md), [EigenQL](../eigenql/README.md),
and [formula language](../formula/README.md) guides plus at least one
of the per-host institution chapters
([platform §10](../platform/10-wasm-institutions.md) or
[platform §11](../platform/11-runtime-substrate.md)) you have all the
prerequisites. If not, read at least [chapter 1](01-introduction.md) here
first — it summarises the surface vocabulary the rest assumes.

The chapters build on each other:

1. **[Introduction](01-introduction.md)** — what composition is, the
   three layers it operates at (shared payload language → declared
   comorphisms → coordinated dispatch roles), and the kinase notebook
   as the running example.
2. **[Shared payload languages](02-shared-payload-languages.md)** — why
   composition is cheap when both endpoints speak the same data shape,
   expensive when they don't. `formulas:FormulaTerm` as the v1 example
   and what makes it the right shape for cross-institution flow.
3. **[Comorphisms — bridges between domains](03-comorphisms.md)** —
   the triadic structure (`ExportFormat` + `transformation` Component +
   `ImportFormat`), the four-step dispatch pipeline (D14 §9.3), identity
   transformations vs. structural ones, the `exact: bool` Satisfaction-Condition
   and what it means for soundness.
4. **[The three dispatch roles in concert](04-dispatch-roles-in-concert.md)**
   — AutoOnLoad gates, OnDemand FIBER, Decidable predicates. How they
   compose: an AutoOnLoad commit can produce a Verdict that an OnDemand
   FIBER call later reads; a Decidable predicate downstream can branch
   on either. The kinase notebook fires two AutoOnLoad gates back-to-back
   — this chapter explains why that's the typical shape.
5. **[Chain reinsertion of comorphism outputs](05-chain-reinsertion.md)**
   — D14 §9.3 step 4 in practice. ESL `Exp::InstitutionInvoke`,
   EigenQL `FIBER ... INTO`, deterministic content-hash IRIs, the
   `Trace::Comorphism` audit variant. Why reinsertion matters for
   composition: downstream queries can find and reason about the
   produced resource as a first-class chain entity.
6. **[Walkthrough: reading the kinase notebook end-to-end](06-kinase-walkthrough.md)**
   — the canonical comorphism-mediated example, traced cell by cell.
   Three parts (flat data, typed institutions, chain reinsertion); how
   each cell exercises one or more of the mechanics in chapters 2–5.
7. **[Statistics + reasoning walkthrough](07-stats-and-reasoning-walkthrough.md)**
   — the second composition shape, traced end-to-end: raw IC50 readings
   → D52 `StatisticalAnalysisPlan` Holds → witness-index admission →
   D39 `ReasoningSentence` certificate consumes the witness via
   `DerivedEvidence` → `StrongInhibitor` conclusion. No comorphism
   between the institutions; the composition runs through the shared
   `core:EigenTTType` proposition slot.
8. **[Composition patterns](08-patterns.md)** — when to share a payload
   language, when to declare a comorphism, when an OnDemand FIBER
   suffices, when chain reinsertion matters. Identity comorphisms
   (Symbolics ↔ Intervals) vs. structural ones (Catalyst → DiffEq's
   reaction-network → ODE compilation). When `exact: false` is
   appropriate.
9. **[Failure modes across compositions](09-failure-modes.md)** — chain
   validation cascades through nested resources; what happens when a
   comorphism's source institution rejects an extract; race conditions
   between AutoOnLoad gates and OnDemand calls; what "stale Verdict"
   means in a composition.
10. **[Appendix](10-appendix.md)** — references, source index, related
    design docs (D14 §4–§6 + §9.3, D26, D27, D32, D39, D46, D47, D48,
    D49, D52), pointer to the
    [SHACL-comparison note](../../notes/note-for-a-shacl-user.md) for
    the conceptual pitch.

## Most important chapters

- **[1. Introduction](01-introduction.md)** for the framing.
- **[3. Comorphisms](03-comorphisms.md)** for the load-bearing concept.
- **[6. Walkthrough](06-kinase-walkthrough.md)** for the worked example
  in one place, end to end.

## What this guide is *not*

- **Not a per-host how-to.** WASM and substrate authoring live in
  [platform §10](../platform/10-wasm-institutions.md) and
  [platform §11](../platform/11-runtime-substrate.md) respectively.
  This guide assumes you can read those.
- **Not the formula language reference.** That's the
  [formula language guide](../formula/README.md). This guide treats
  `FormulaTerm` as a *coordination mechanism* between institutions; the
  formula guide treats it as a EigenTT fragment in its own right.
- **Not a single-institution tutorial.** Per-institution slow-walks
  live under [`platform/julia-institutions/`](../platform/julia-institutions/).
  This guide assumes you've internalised at least the intervals
  tutorial.
- **Not the conceptual pitch.** The
  [SHACL-comparison note](../../notes/note-for-a-shacl-user.md) frames
  the broad story for someone coming from the W3C semantic web stack.
  This guide is the structured reference once that framing has landed.

## Cross-references

- [**Platform §10** — Building WASM institutions](../platform/10-wasm-institutions.md)
- [**Platform §11** — Runtime substrate](../platform/11-runtime-substrate.md)
- [**Platform `julia-institutions/`**](../platform/julia-institutions/) — per-institution Julia tutorials
- [**ESL §9** — Institutions in ESL](../esl/09-institutions.md)
- [**EigenQL §7** — FIBER clauses](../eigenql/07-fiber-clauses.md), [**§8** — Institutions in EigenQL](../eigenql/08-institutions.md)
- [**Formula language guide**](../formula/README.md)
- [**D14** — Institution Realisation](../../design/d14-institution-realisation.md) — the canonical spec
- [**D26** — Runtime substrate](../../design/d26-runtime-substrate.md), [**D27** — Julia institutions](../../design/d27-julia-institutions.md), [**D32** — Chain-mirrored EigenTT inductives](../../design/d32-chain-mirrored-mini-tt-inductives.md)

---

Ready to start? → **[1. Introduction](01-introduction.md)**
