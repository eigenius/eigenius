# 1. Introduction

This guide is about what happens when **multiple institutions cooperate**. The
per-host chapters elsewhere in the platform guide ([WASM in §10](../platform/10-wasm-institutions.md),
[substrate in §11](../platform/11-runtime-substrate.md)) cover what *one*
institution looks like; this guide covers what the *system* looks like when
several institutions, each with their own dispatch roles and their own host
runtime, share data through the chain and bridge across each other through
declared comorphisms.

The chapter that follows establishes the framing: what composition means here,
the three layers it operates at, and the kinase-institutions notebook as the
running example.

## 1.1. What this guide is for

If you can write a single ESL program, run a single EigenQL query, and follow
[platform §10](../platform/10-wasm-institutions.md) or
[platform §11](../platform/11-runtime-substrate.md) to author one institution
end-to-end, you have all the prerequisites this guide assumes. What you don't
have yet — and what this guide gives you — is the cross-cutting story: when a
chain commit triggers a cascade across two institutions, when a comorphism's
reify output becomes the trigger for a downstream gate, when an OnDemand FIBER
call reads a Verdict an AutoOnLoad gate produced earlier in the same query.

That story doesn't fit into any single per-host or single-language chapter
because it's about *interactions*, not about any one component. It is the
working surface of Eigenius for any non-trivial multi-domain workflow.

## 1.2. The three layers of composition

Composition operates at three layers. Each subsequent chapter sits at one of
them; the chapter numbering reflects the layer ordering from cheapest (just
agreeing on a data shape) to most structural (closing the loop with chain
reinsertion).

| Layer | What it is | Chapter |
|---|---|---|
| **Shared payload language** | A typed value shape multiple institutions consume directly. `formulas:FormulaTerm` is the v1 instance — a chain-mirrored fragment of EigenTT every numerical institution speaks. | [Chapter 2](02-shared-payload-languages.md) |
| **Declared comorphisms** | Chain-resident bridge declarations that translate from one institution's vocabulary into another's, with the kernel statically type-checking the alignment. | [Chapter 3](03-comorphisms.md) |
| **Coordinated dispatch roles** | AutoOnLoad gates, OnDemand FIBER calls, and Decidable predicates working together to produce a single user-facing response from a coordinated set of institution dispatches. | [Chapter 4](04-dispatch-roles-in-concert.md) |

Above these three layers sits the [chain-reinsertion mechanism (Chapter 5)](05-chain-reinsertion.md):
the way comorphism outputs become first-class chain residents, which is what
turns single-step translations into building blocks for multi-step pipelines.

The order matters in practice. If two institutions don't share a payload
language, every comorphism between them has to do real translation work — and
identity transformations (the cheapest case) become impossible. If comorphisms
aren't declared on the chain, dispatch coordination has nowhere to plug in. If
dispatch roles don't compose, chain reinsertion has nothing to feed.

## 1.3. The kinase-institutions notebook at a glance

The canonical worked example throughout this guide is
[`notebooks/examples/kinase-institutions.json`](../../../notebooks/examples/kinase-institutions.json) —
a 36-cell notebook organised in three parts:

| Part | Cells | Exercises |
|---|---|---|
| **A** — flat data + visualisation | 1–13 | 24 IC₅₀ measurements as typed `AssayResult` resources; six chart kinds; topology graph. No institutions involved. The baseline of "what flat queries can answer." |
| **B** — typed institutions | 14–28 | Catalyst → DiffEq forward simulation of EIG_0291 + CDK2 binding; Symbolics → JuMP fit of Kᵢ via Cheng–Prusoff; both AutoOnLoad gates fire; loop-closure query joins the fitted Kᵢ back to the screened 85 nM measurement. |
| **C** — chain reinsertion | 29–35 | Re-runs the `symbolics_to_jump` comorphism live through both the ESL program-invoke surface (`Exp::InstitutionInvoke`) and the EigenQL `FIBER ... INTO` surface. |

Five Julia institutions registered: Symbolics, IntervalArithmetic, Catalyst,
DiffEq, JuMP-HiGHS. Three cross-institution comorphisms registered:
`symbolics_to_intervals`, `catalyst_to_diffeq`, `symbolics_to_jump`. The
notebook itself is its own running narrative — its markdown cells (14, 19, 23,
26, 29, 36) describe what's happening as it runs through.

To run the notebook end-to-end, you need the institutions installed first —
see [`notebooks/examples/kinase-institutions-setup.sh`](../../../notebooks/examples/kinase-institutions-setup.sh).
Cold install is heavy (~30–60 minutes for the five Julia env image builds);
subsequent runs reuse buildah's layer cache and are fast.

For a per-cell narrative reading of the notebook, see
[chapter 6](06-kinase-walkthrough.md).

## 1.3a. A second composition shape — D52 statistics + D39 reasoning

The kinase notebook covers cross-numerical-institution composition: five Julia institutions that all consume `formulas:FormulaTerm` and bridge to each other via declared comorphisms. With D52 (the measurement-statistics institution) and D39 (the justification-logic reasoning institution) on the platform, a second composition shape lives alongside it:

| Composition shape | Bridge mechanism | Worked example |
|---|---|---|
| **Numerical → numerical** | Declared comorphisms over `formulas:FormulaTerm` (extract → transformation → reify pipeline). | Kinase notebook — Symbolics → JuMP, Catalyst → DiffEq, Symbolics → IntervalArithmetic. |
| **Statistics → reasoning** | Per-layer chain-witness index over `core:EigenTTType` propositions. | D52 `StatisticalAnalysisPlan` verdict emits a `DerivedResource` with `canonical_proposition`; a D39 `ReasoningSentence` cites it via `DerivedEvidence` and the witness index admits the grounding. See [chapter 7](07-stats-and-reasoning-walkthrough.md). |

The two shapes are structurally different: comorphisms are *active* translations (one institution's runtime is invoked, output is reified back into the chain), while the witness-index path is *passive* — D39 doesn't call D52, it just reads the chain artifact D52 emitted. Both are first-class composition mechanisms; which one applies depends on whether the downstream institution needs the input *value translated* (comorphism) or just *cited as evidence* (witness index).

Chapter 2 covers both shared payload languages — `formulas:FormulaTerm` for the comorphism shape, `core:EigenTTType` for the witness-index shape. Chapter 4 covers how AutoOnLoad gates from the two institutions cascade through a single commit. Chapter 7 walks the full statistics + reasoning pipeline end-to-end.

## 1.4. Surface vocabulary you should already have

Composition assumes you can read each of these terms without further context.
If any are unfamiliar, the cross-link is the right entry point.

| Term | Where to learn it |
|---|---|
| `Institution`, `ExportFormat`, `ImportFormat`, `QueryClass`, `Comorphism` (the five chain shapes) | [ESL §9.1](../esl/09-institutions.md#91-what-an-institution-declares) |
| `AutoOnLoad` / `OnDemand` / `Decidable` (the three dispatch roles) | [ESL §9.2](../esl/09-institutions.md#92-classification-at-compile-time), [EigenQL §8.2](../eigenql/08-institutions.md#82-the-classification-at-parse-time) |
| `Verdict` (`Holds` / `Fails` / `Undecidable`) | [ESL §9.3](../esl/09-institutions.md#93-invoking-a-decidable-queryclass) |
| `formula(...)` ESL sublanguage and `formulas:FormulaTerm` | [Formula language guide](../formula/README.md) |
| `type_expr(...)` ESL sublanguage and `core:EigenTTType` (D47) | [ESL §5.14a](../esl/05-expressions.md#5-14a-type_expr-eigentt-type-expressions) |
| `Prop` universe and proof irrelevance (D46) | [ESL §7.1](../esl/07-type-theory-primer.md#7-1-universes-the-unified-sortn-ladder-with-prop-at-the-bottom) |
| `ChainWitness.Is*As` predicates and the witness index (D49) | [ESL §6.4a](../esl/06-resources-types-and-the-layer.md#6-4a-witness-predicates-admitting-propositions-from-layer-state) |
| `StatisticalAnalysisPlan` / `SampleSet` (D52) | [Statistics institution tutorial](../platform/statistics-institution/README.md) |
| `JustifiedBy` / `JustificationTerm` / `ReasoningSentence` (D39) | [ESL §9.10](../esl/09-institutions.md#9-10-the-reasoning-institution-d39-justification-logic), [reasoning institution tutorial](../platform/reasoning-institution/README.md) |
| `FIBER ... AS ?var INTO "<iri>"` | [EigenQL §7.6](../eigenql/07-fiber-clauses.md#76-into--pinning-the-response-iri) |
| Comorphism program-invoke (`comorphisms:foo(input)` in ESL) | [ESL §9.5](../esl/09-institutions.md#95-invoking-comorphisms-from-esl-programs) |
| Runtime substrate (`mirror create → env build → env create → institution install`) | [Platform §11](../platform/11-runtime-substrate.md) |

You don't need to have *authored* an institution from scratch — but you should
be able to read the kinase notebook's ESL cells (cells 16, 17, 20, 30) without
needing to look anything up.

## 1.5. What this guide is *not* for

Composition is cross-cutting; the per-domain reference material lives
elsewhere:

- **Per-host how-to.** WASM authoring lives in
  [platform §10](../platform/10-wasm-institutions.md); substrate authoring in
  [platform §11](../platform/11-runtime-substrate.md). This guide assumes you
  can read those.
- **The formula language reference.** [`urn:eigenius:formulas:FormulaTerm`](../formula/README.md)
  has its own guide. This guide treats it as a *coordination mechanism* between
  institutions; the formula guide treats it as a EigenTT fragment in its own
  right.
- **Single-institution slow-walks.** Per-institution Julia tutorials live under
  [`platform/julia-institutions/`](../platform/julia-institutions/). Read at
  least the [intervals tutorial](../platform/julia-institutions/intervals-institution-tutorial.md)
  before continuing.
- **The conceptual pitch.** The
  [SHACL-comparison note](../../notes/note-for-a-shacl-user.md) frames the
  broad story for someone coming from the W3C semantic-web stack. This guide
  is the structured reference once that framing has landed.

Readers looking for the **theoretical lineage** of institution theory (its
classical model-theoretic origins, Eigenius's constructive realisation, and
the open research direction of formalising the translation into type theory)
should skip ahead to [§3.9](03-comorphisms.md#39-note-theoretical-foundations-and-a-research-direction).

---

Next: **[2. Shared payload languages →](02-shared-payload-languages.md)**
