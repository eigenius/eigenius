# 6. Walkthrough: reading the kinase notebook end-to-end

> **STATUS:** outline only. To be filled in.

## What this chapter covers

The canonical worked example for cross-institution composition,
walked through cell by cell. Where chapters 2–5 introduced concepts
in isolation, this chapter shows them in concert against a real
notebook a reader can actually run.

The notebook
([`notebooks/examples/kinase-institutions.json`](../../../notebooks/examples/kinase-institutions.json))
runs in three parts:

1. **Part A (cells 1–13) — flat dataset & visualisation.** Authors
   24 IC₅₀ measurements as typed `AssayResult` resources, renders
   them across every Fluent chart kind plus the topology graph. No
   institutions involved; demonstrates the baseline (what flat
   queries can answer).

2. **Part B (cells 14–28) — typed institutions.** Anchors on
   EIG_0291 + CDK2. Storyline 1 (Catalyst → DiffEq) commits a
   reaction network and an `OdeSolution` claim that fires the DiffEq
   AutoOnLoad gate. Storyline 2 (Symbolics → JuMP) commits a Kᵢ-fit
   `OptimisesTo` claim that fires the JuMP-HiGHS gate. Then a loop-closure
   query joins the fit's Kᵢ back to the screened EIG_0291 / CDK2 /
   Kinase-Glo measurement via Cheng–Prusoff.

3. **Part C (cells 29–35) — chain reinsertion.** Re-runs the
   Symbolics → JuMP comorphism live (rather than hand-authoring its
   output) through both the ESL program-invoke surface and the
   EigenQL `FIBER ... INTO` surface.

For each part the chapter calls out *which mechanic from chapters 2–5
is being exercised*. The goal isn't to re-explain mechanics —
chapters 2–5 own those — but to ground them in a concrete sequence
the reader can run and inspect.

## Section outline

- **§6.1.** Setup: the notebook and the `setup-institutions.sh` script
- **§6.2.** Part A — flat dataset (no institutions) as the baseline
- **§6.3.** The bridge cell (cell 14) — what flat queries can't
  answer, and which institution closes each gap
- **§6.4.** Part B Storyline 1 — Catalyst → DiffEq AutoOnLoad cascade
- **§6.5.** Part B Storyline 2 — Symbolics → JuMP fit + loop closure
  to the screening data
- **§6.6.** What the institutions actually computed (cells 23–25;
  surfacing gate-endorsed values + `RuntimeInvocation` provenance)
- **§6.7.** Closing the loop: Cheng–Prusoff prediction matches the
  screened 85 nM (cells 26–27)
- **§6.8.** Part C — comorphism chain reinsertion through both
  surfaces (cells 30–35)
- **§6.9.** Where this notebook falls short (the
  Symbolics → IntervalArithmetic comorphism is registered but not
  exercised; OnDemand `qc_jump_solve` could be the next worked cell)

## Cross-references

- [Platform §8.4](../platform/08-demos.md#84-kinase-institutions--multi-institution-julia-stack)
  — the demo overview
- [Platform §11](../platform/11-runtime-substrate.md) — the substrate
  the notebook runs on
- [`notebooks/examples/kinase-institutions-setup.sh`](../../../notebooks/examples/kinase-institutions-setup.sh)
  — what installs the institutions before the notebook can run

---

Next: **[7. Composition patterns →](07-patterns.md)**
