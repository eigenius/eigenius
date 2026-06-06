# Julia institution tutorials

Slow-walk worked examples of substrate-hosted Julia institutions. Each one
exercises the install flow described in
[platform §11 — Runtime substrate](../11-runtime-substrate.md) against a
real Julia library, wires up the institution's chain shapes, and runs at
least one verdict end-to-end.

Read in order if this is your first substrate institution. The intervals
tutorial covers the substrate plumbing (mirror generator, env image,
worker container, AutoOnLoad gate) at a slow pace; the others assume that
plumbing knowledge and focus on what's domain-specific to each institution.

| Tutorial | Wraps | What's distinctive |
|---|---|---|
| [Intervals](intervals-institution-tutorial.md) | [`IntervalArithmetic.jl`](https://juliaintervals.github.io/) | The simplest possible external-runtime institution — one class, one AutoOnLoad gate, one handler. The recommended first read. |
| [Symbolics](symbolics-institution-tutorial.md) | [`Symbolics.jl`](https://juliasymbolics.org/) | Three dispatch roles in one institution (AutoOnLoad / OnDemand / Decidable); the FormulaTerm-as-EigenTT-fragment story end-to-end. |
| [Catalyst](catalyst-institution-tutorial.md) | [`Catalyst.jl`](https://docs.sciml.ai/Catalyst/stable/) | Chemical reaction networks; companion to the DiffEq tutorial via the Catalyst → DiffEq comorphism. |
| [DiffEq](diffeq-institution-tutorial.md) | [`OrdinaryDiffEq.jl`](https://docs.sciml.ai/DiffEqDocs/stable/) | ODE integration; the gate re-integrates the FormulaTerm RHS to verify a claimed solution. |
| [JuMP-HiGHS](jump-highs-institution-tutorial.md) | [`JuMP`](https://jump.dev/) + [`HiGHS`](https://highs.dev) | LP/QP optimisation; the smart-pow walker rule that keeps quadratic objectives in `QuadExpr` rather than `NonlinearExpr`. |

The end-to-end "everything together" demo notebook that exercises all five
institutions plus three cross-institution comorphisms lives at
[`notebooks/examples/kinase-institutions.json`](../../../../notebooks/examples/kinase-institutions.json)
(setup script:
[`notebooks/examples/kinase-institutions-setup.sh`](../../../../notebooks/examples/kinase-institutions-setup.sh)).
See [platform §8.4](../08-demos.md#84-kinase-institutions--multi-institution-julia-stack)
for the demo overview.

## Cross-references

- [**Platform §11 — Runtime substrate**](../11-runtime-substrate.md) — the
  conceptual chapter the tutorials ground.
- [**The formula language guide**](../../formula/README.md) — the shared
  payload language for cross-institution numerical work; pervasively used
  by these institutions.
- [**ESL §9 — Institutions in ESL**](../../esl/09-institutions.md),
  [**EigenQL §8 — Institutions in EigenQL**](../../eigenql/08-institutions.md)
  — the user-facing dispatch surface.
- [**D14 — Institution Realisation**](../../../design/d14-institution-realisation.md),
  [**D26 — Runtime Substrate**](../../../design/d26-runtime-substrate.md),
  [**D27 — Julia institutions**](../../../design/d27-julia-institutions.md),
  [**D32 — Chain-Mirrored EigenTT Inductives**](../../../design/d32-chain-mirrored-mini-tt-inductives.md)
  — the design specs.
