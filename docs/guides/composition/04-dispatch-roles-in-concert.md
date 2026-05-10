# 4. The three dispatch roles in concert

> **STATUS:** outline only. To be filled in.

## What this chapter covers

The three QueryClass dispatch roles (`AutoOnLoad`, `OnDemand`,
`Decidable`) are introduced individually in [ESL §9](../esl/09-institutions.md)
and [EigenQL §8](../eigenql/08-institutions.md). This chapter covers
how they *interact* — what a coordinated dispatch flow looks like when
multiple gates fire across multiple institutions in response to a
single chain commit.

Three patterns to internalise:

1. **AutoOnLoad cascades.** A commit can trigger multiple AutoOnLoad
   gates if it touches multiple chain resources whose classes are
   gated. The kinase notebook's Storyline 1 commits an `OdeSolution`
   that fires DiffEq's `validate_solution` gate; Storyline 2 commits
   an `OptimisesTo` that fires JuMP-HiGHS's `validate_optimum` gate.
   What a *cross-institution* cascade would look like (one gate's
   verdict spawning a downstream gate via a derived resource).

2. **OnDemand FIBER reading what AutoOnLoad produced.** AutoOnLoad
   gates produce Verdict + RuntimeInvocation chain residents. An
   OnDemand FIBER call later in a query can match against those
   verdicts, branch on them, or feed them as inputs to another
   institution's QueryClass.

3. **Decidable predicates as the connective tissue.** The Decidable
   role is what lets a *constraint* (e.g. "is this Kᵢ within the
   covenant range?") fire during type-check reduction or in `WHERE`
   filters. Composes naturally with the other two: AutoOnLoad
   produces a typed value, Decidable filters it, OnDemand FIBER
   re-dispatches if needed.

The chapter closes with a section on *write-side coordination* —
chain reinsertion (chapter 5) means a comorphism's reify output
becomes the trigger for the next gate downstream. That's how
multi-step pipelines (Catalyst → DiffEq → IntervalArithmetic) would
chain end-to-end as a single user-facing commit.

## Section outline

- **§4.1.** Recap: the three dispatch roles
- **§4.2.** AutoOnLoad cascades — single commit, multiple gates
- **§4.3.** OnDemand FIBER reading prior Verdicts
- **§4.4.** Decidable predicates as compositional filters
- **§4.5.** Write-side coordination via chain reinsertion (forward
  link to chapter 5)
- **§4.6.** A worked dispatch flow: the kinase Storyline 2 step by step

## Cross-references

- [ESL §9.3](../esl/09-institutions.md) — Decidable mechanics
- [EigenQL §7](../eigenql/07-fiber-clauses.md) — OnDemand FIBER
- [Platform §11](../platform/11-runtime-substrate.md) — AutoOnLoad
  in the substrate context

---

Next: **[5. Chain reinsertion of comorphism outputs →](05-chain-reinsertion.md)**
