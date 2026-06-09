# 6. Walkthrough: reading the kinase notebook end-to-end

The
[`kinase-institutions.json`](../../../notebooks/examples/kinase-institutions.json)
notebook is the canonical worked example for cross-institution composition
on the platform. Where chapters 2–5 introduced concepts in isolation, this
chapter shows them in concert against a notebook a reader can actually run
— mapping each cell to the mechanic from earlier chapters that it
exercises.

The notebook runs in three parts:

| Part | Cells | Exercises |
|---|---|---|
| **A** — flat dataset & visualisation | 1–13 | Chain commits with no institution involvement; the baseline of "what flat queries can answer" |
| **B** — typed institutions | 14–28 | Both AutoOnLoad cascades (DiffEq + JuMP-HiGHS), plus the loop-closure query that joins the fitted Kᵢ back to the screening data |
| **C** — chain reinsertion | 29–35 | Both surfaces of the [D14 §9.3](../../design/d14-institution-realisation.md) chain-reinsertion contract |

A 36th cell wraps with a "what just happened" summary.

This chapter doesn't re-explain mechanics — chapters 2–5 own those — but
points at *which mechanic each cell exercises*. Read the notebook
alongside; this chapter is the reading guide, not a substitute.

## 6.1. Setup

The notebook needs five Julia institutions and three comorphisms registered
on the chain before it runs. The setup script
[`notebooks/examples/kinase-institutions-setup.sh`](../../../notebooks/examples/kinase-institutions-setup.sh)
brings the docker stack up, generates the five mirror packages, builds the
five env images, commits the institution declarations, and registers the
three comorphisms.

```bash
EIGENIUS_MOCK_LLM=true docker compose up -d
./notebooks/examples/kinase-institutions-setup.sh
# Then in a browser: http://localhost:8080/notebooks/
# Import notebooks/examples/kinase-institutions.json and Run All.
```

Cold first run is heavy (~30–60 minutes — five Julia env image builds with
`Pkg.precompile`); subsequent runs reuse buildah's layer cache and finish
in seconds. The setup script is idempotent for the chain commits but not
for the env image builds; if the chain has partial state from an earlier
attempt, `docker compose down -v` resets cleanly.

## 6.2. Part A — flat dataset (no institutions) as the baseline

Cells 1–13 are deliberately institution-free. They demonstrate what
EigenQL + the chart cell type can do over flat typed data, without any
cross-institution machinery, so that the contrast with Part B is concrete.

| Cell | Type | What it does |
|---|---|---|
| 1 | markdown | Title + three-part roadmap |
| 2 | esl | Assay-screening ontology: `Compound`, `Target`, `AssayProtocol`, `AssayResult` classes + their properties |
| 3 | esl | Foundation resources: 6 compounds × 4 targets × 3 protocols, plus the kinetic constants (`michaelis_constant_um` on Target, `substrate_concentration_um` on AssayProtocol) that Part B's loop closure consumes |
| 4 | esl | 24 `AssayResult` measurements covering every (compound, target) cross-product with mixed protocols |
| 5 | eigenql | Selectivity matrix: every QC-passing AssayResult joined to compound + target |
| 6 | eigenql | QC pass-rate by protocol |
| 7–12 | chart × 6 | Six chart kinds: grouped-bar, donut, vertical-bar, horizontal-bar, line, area |
| 13 | typescript | Topology graph of the assay namespace via `eigen.layerTopology()` |

What's *not* there: any `Institution`, `QueryClass`, `Comorphism`, or
substrate dispatch. The data is just typed Eigon resources with property
references; queries pull rows; charts render aggregates. This is the
pre-institution baseline.

What flat queries *can't* answer with this data alone is the bridge into
Part B (cell 14).

## 6.3. The bridge cell (cell 14)

Cell 14 is a markdown bridge that does three things:

1. **States the gap.** Lists three questions the flat queries can't
   answer: binding kinetics, Kᵢ derivation via Cheng–Prusoff, rigorous
   bounds under measurement uncertainty.
2. **Maps each gap to an institution.** Catalyst → DiffEq for the
   kinetics; Symbolics → JuMP for the Kᵢ fit; Symbolics →
   IntervalArithmetic for the bounds (sketched, not exercised in v1).
3. **Anchors on EIG_0291 + CDK2.** The pan-CDK pyrazolopyrimidine from
   the screening data, with measured IC₅₀ = 85 nM at the Kinase-Glo
   protocol. CDK2's chain-recorded Kₘ = 20 μM and Kinase-Glo's
   chain-recorded [S] = 80 μM combine via Cheng–Prusoff to give a
   prediction that exactly matches the measurement: `Kᵢ · (1 + 80/20) =
   Kᵢ · 5 = 85`, so `Kᵢ = 17 nM`.

The anchoring matters compositionally: every subsequent storyline uses
EIG_0291 + CDK2 specifically, so the Part B fit's recovered Kᵢ has a
checkable answer (17 nM) and the loop closure (cells 26–27) compares
against a real screening row (R0013).

## 6.4. Part B Storyline 1 — Catalyst → DiffEq AutoOnLoad cascade

Cells 15–18 exercise the first AutoOnLoad gate.

**Cell 15 (markdown).** Frames Storyline 1: model EIG_0291 + CDK2 binding
as the one-step pseudo-first-order reaction `A → B` with `k = 1`,
`A(0) = 1`, `B(0) = 0`. The closed form at `t = 1` is `[e⁻¹, 1 − e⁻¹]` —
what the institution must independently re-derive.

**Cell 16 (ESL).** Commits a Catalyst `ReactionNetwork`:

```esl
resource nb:ab_network : catalyst:ReactionNetwork {
    core:short_name              = "eig0291_cdk2_binding";
    catalyst:network_source      = "@reaction_network begin\n    k, A --> B\nend";
    catalyst:species_declared    = ["A", "B"];
    catalyst:parameters_declared = ["k"];
}
```

This is a typed chain commit. No AutoOnLoad gate fires — Catalyst's
gates are bound to `ConservationLaw` claims, not to `ReactionNetwork`
itself. The network is just sitting on the chain, ready to be referenced.

**Cell 17 (ESL).** Hand-authors the `OdeProblem` the
`catalyst_to_diffeq` comorphism *would* produce, plus an `OdeSolution`
claim:

```esl
resource nb:rhs_A : diffeq:RhsComponent {
    diffeq:term = formula(-1 * A * k);  // dA/dt = -k·A
}

resource nb:rhs_B : diffeq:RhsComponent {
    diffeq:term = formula(A * k);       // dB/dt = k·A
}

resource nb:ode_problem : diffeq:OdeProblem { ... rhs = [nb:rhs_A, nb:rhs_B], ... }

resource nb:ode_solution : diffeq:OdeSolution {
    diffeq:problem     = nb:ode_problem;
    diffeq:final_state = [0.36787944117144233, 0.6321205588285577];
    ...
}
```

The `formula(-1 * A * k)` is the [ESL formula(...) sublanguage](../formula/05-esl-sublanguage.md)
producing a chain-resident FormulaTerm that the [validator's
inductive-value rule](../formula/03-eigon-json-embedding.md#34-the-validators-inductive-value-rule)
type-checks at commit time — the chain shape is verified before the
institution ever runs.

When `nb:ode_solution` commits, **the AutoOnLoad cascade fires** (chapter
4 §4.2). DiffEq's `validate_solution` gate is bound to `OdeSolution`. The
kernel:

1. Builds the extract input — the embedded `nb:ode_problem`.
2. RPCs to the orchestrator, which routes to the DiffEq Julia worker.
3. The worker decodes the input via the generated mirror, walks each
   `RhsComponent.term` under `formula_to_value` to build a numerical
   closure, constructs an `ODEProblem(rhs!, u0, tspan, p)`, calls
   `solve(Tsit5)` at `abstol = reltol = 1e-8`, and compares the result
   against the claimed `final_state` within `max(abstol, reltol·|actual|)`.
4. Returns `Verdict::Holds` (matches the closed form `[e⁻¹, 1 − e⁻¹]`).
5. The kernel commits `nb:ode_solution` plus the Verdict + RuntimeInvocation.

**Cell 18 (EigenQL).** Pulls the Verdict by gated subject:

```eigenql
MATCH "urn:eigenius:institution:Verdict"(?v) {
    "urn:eigenius:core:ctor_name":               ?ctor,
    "urn:eigenius:institution:verdict_subject":  ?subject
}
WHERE ?subject = "urn:eigenius:notebook:kinase_demo:ode_solution"
RETURN [] { verdict: ?v, ctor: ?ctor, subject: ?subject }
```

One row, `ctor = "Holds"`. The DiffEq institution agrees with the claim
within tolerance.

## 6.5. Part B Storyline 2 — Symbolics → JuMP fit + loop closure

Cells 19–27 exercise the second AutoOnLoad gate, then close the loop with
the screening data.

**Cell 19 (markdown).** Frames Storyline 2. The dose-response panel at
`[S] ∈ {20, 60, 180}` μM (with CDK2's `Kₘ = 20`) gives Cheng–Prusoff
coefficients `c = (2, 4, 10)` and observed IC₅₀ values `(34, 68, 170)`,
chosen consistent with `Kᵢ = 17`. The SSE objective `(34 − 2·Kᵢ)² + (68
− 4·Kᵢ)² + (170 − 10·Kᵢ)²` minimises at `Kᵢ* = 17` with `SSE* = 0`.

**Cell 20 (ESL).** Commits the five resources:

```esl
resource nb:sse_expr : symbolics:SymbolicExpression {
    symbolics:term = formula((34 - 2*Ki)^2 + (68 - 4*Ki)^2 + (170 - 10*Ki)^2);
}

resource nb:ki_bound : jump:VariableBound { ... 0.0 .. 50.0 ... }

resource nb:fit_input : symbolics:SymbolicsToJuMPInput {
    symbolics:objective              = nb:sse_expr;
    symbolics:variable_names         = ["Ki"];
    symbolics:framing_variable_bounds = [nb:ki_bound];
    symbolics:sense                  = "Min";
}

resource nb:opt_problem : jump:OptimisationProblem {
    jump:objective       = formula((34 - 2*Ki)^2 + (68 - 4*Ki)^2 + (170 - 10*Ki)^2);
    ...
}

resource nb:optimises_to : jump:OptimisesTo {
    jump:problem                = nb:opt_problem;
    jump:objective_value        = 0.0;
    jump:variable_values        = [17.0];
    ...
}
```

Same SSE FormulaTerm appears twice — once in `nb:sse_expr.term` and once
in `nb:opt_problem.objective`. ESL v1 doesn't have let-bindings inside
`formula(...)`, so the expression is duplicated; both copies parse via
the same Pratt path and produce identical chain bytes.

When `nb:optimises_to` commits, **the JuMP-HiGHS AutoOnLoad gate fires**.
The full step-by-step is in [§4.6](04-dispatch-roles-in-concert.md#46-a-worked-dispatch-flow-kinase-storyline-2-step-by-step)
— briefly: rebuild the model under the FormulaTerm walker (smart-pow
on, so `pow(•, LitFloat(2.0))` unrolls to `(• * •)` keeping the result
in `QuadExpr` rather than `NonlinearExpr`), call `optimize!`, compare
against the claimed `objective_value = 0.0` and `variable_values = [17.0]`
within `max(abstol, reltol·|actual|) = 1e-6`. Returns `Verdict::Holds`.

**Cell 21 (EigenQL).** Verdict for the JuMP-HiGHS gate, same shape as
cell 18.

**Cell 22 (EigenQL).** Aggregate Verdicts: pulls every Verdict on the
chain. After running cells 1–21 fresh, two rows: one from DiffEq, one
from JuMP-HiGHS. This is the OnDemand-FIBER-reads-AutoOnLoad-output
pattern from [§4.3](04-dispatch-roles-in-concert.md#43-ondemand-fiber-reading-what-autoonload-produced)
— except the read here is plain EigenQL pattern-matching, not FIBER, but
the structural fact (downstream queries can read upstream Verdicts) is
the same.

**Cell 23 (markdown).** Bridges to "what did the institutions actually
compute?"

**Cell 24 (EigenQL).** Pulls the OptimisesTo's claimed values plus the
Verdict's ctor:

```eigenql
MATCH OptimisesTo(?o) {
    "urn:eigenius:jump:variable_values":     ?vals,
    "urn:eigenius:jump:objective_value":     ?obj,
    "urn:eigenius:jump:termination_status":  ?status,
    ...
}
MATCH "urn:eigenius:institution:Verdict"(?v) {
    "urn:eigenius:core:ctor_name":              ?ctor,
    "urn:eigenius:institution:verdict_subject": ?o
}
WHERE ?o = "urn:eigenius:notebook:kinase_demo:optimises_to"
RETURN [] {
    fitted_Ki_nM:     ?vals,        // [17.0]
    sse_min:          ?obj,         // 0.0
    termination:      ?status,      // "OPTIMAL"
    institution_says: ?ctor         // "Holds"
}
```

The gate's `Holds` Verdict is what turns the claim's `[17.0]` from "the
author asserted this" into "the JuMP-HiGHS institution endorsed this
within 1e-6 tolerance."

**Cell 25 (EigenQL).** Pulls the `RuntimeInvocation` provenance — image
digest, started_at / completed_at, numerical_metadata. This is the audit
closure D31 §6.3 prescribes.

**Cells 26–27 — closing the loop with the screening data.** Cell 26
(markdown) computes Cheng–Prusoff: `Kᵢ · (1 + 80/20) = 17 · 5 = 85`,
matching R0013's measured 85 nM (CI [72, 100]). Cell 27 (EigenQL) does
the join in code:

```eigenql
USING "urn:eigenius:demo:assay:AssayResult"
USING "urn:eigenius:jump:OptimisesTo"

MATCH AssayResult(?r) { ... },
?c { compound_id: ?compound_id },
?t { target_name: ?target_name, michaelis_constant_um: ?Km_uM },
?p { protocol_name: ?protocol_name, substrate_concentration_um: ?S_uM }
MATCH OptimisesTo(?o) { variable_values: ?fitted_Ki }
WHERE ?compound_id = "EIG_0291"
  AND ?target_name = "CDK2"
  AND ?protocol_name = "Kinase-Glo"
  AND ?qc = true
  AND ?o = "urn:eigenius:notebook:kinase_demo:optimises_to"
RETURN [] {
    fitted_Ki_nM:      ?fitted_Ki,    // [17.0]
    Km_uM:             ?Km_uM,        // 20.0
    S_uM:              ?S_uM,         // 80.0
    ic50_measured_nM:  ?ic50_measured, // 85.0
    ic50_ci_low_nM:    ?ci_low,        // 72.0
    ic50_ci_high_nM:   ?ci_high        // 100.0
}
```

A two-MATCH-clause-with-single-trailing-WHERE join (per
[EigenQL §3.1](../eigenql/03-lexical-structure.md)) that pulls Part A's
flat data and Part B's institution-derived value into one row. The
reader does the multiplication; the chain provides every input.

**Cell 28 (chart).** Verdict-distribution donut. After the notebook runs
fresh, two `Holds`. A failed gate would have shown up as a `Fails` slice.

## 6.6. Part C — chain reinsertion through both surfaces

Cells 29–35 exercise the [D14 §9.3](../../design/d14-institution-realisation.md)
chain-reinsertion contract. Already walked through in detail in
[§5.6](05-chain-reinsertion.md#56-worked-example-the-kinase-notebooks-part-c-end-to-end);
this is the brief recap mapped to which mechanic each cell exercises.

| Cell | Type | Mechanic |
|---|---|---|
| 29 | markdown | Bridge to chain reinsertion |
| 30 | esl | Wrapper program — `comorphisms:symbolics_to_jump(input)` lowers to `Exp::InstitutionInvoke` ([§5.2](05-chain-reinsertion.md#52-esl-surface-comorphismsfooinput--expinstitutioninvoke)) |
| 31 | program-run | Runs cell 30's program against `nb:fit_input` — the four-step pipeline ([§3.2](03-comorphisms.md#32-the-four-step-dispatch-pipeline-d14-93)) commits the produced `OptimisationProblem` at a content-hash IRI ([§5.4](05-chain-reinsertion.md#54-deterministic-content-hash-iris)) |
| 32 | eigenql | Confirms the produced resource exists at `urn:eigenius:comorphism-output:symbolics_to_jump:%` |
| 33 | markdown | Bridge to EigenQL `INTO` |
| 34 | eigenql | `FIBER ... AS ?problem INTO "<iri>"` — same translation, caller-named IRI ([§5.3](05-chain-reinsertion.md#53-eigenql-surface-fiber--as-var-into-iri)) |
| 35 | eigenql | Confirms the FIBER-INTO result resolves at the user-named IRI |

Both paths go through `commit_with_validation`. The produced
`OptimisationProblem`s are structurally identical — the *content* is the
same; only the IRI differs (kernel-minted content-hash vs. caller-named).

## 6.7. Where this notebook falls short

Three things the notebook *doesn't* exercise but the platform supports:

1. **Symbolics → IntervalArithmetic.** The third comorphism the setup
   script registers, but no cell exercises it. The natural fit is
   propagating the screening data's confidence intervals through the
   Cheng–Prusoff formula to bound Kᵢ rigorously — given
   `IC₅₀ ∈ [72, 100]` and `[S], Kₘ` as point values, the implied Kᵢ
   band is `[14.4, 20.0]` nM. A `BoundedBy` claim AutoOnLoad-validated
   by IntervalArithmetic would slot in alongside the JuMP fit and
   tighten cell 27's loop closure with rigorous bounds.

2. **OnDemand `qc_jump_solve`.** Cell 24 surfaces the gate-endorsed
   values (the AutoOnLoad gate's `Holds` Verdict turns the claim into
   the institution's answer to within tolerance). The OnDemand
   `qc_jump_solve` QueryClass exposed by JuMP-HiGHS would let a FIBER
   `INTO` call re-solve fresh and commit a new `OptimisesTo` with the
   solver's actual output values — the explicit "institution computes a
   result" surface as opposed to the implicit "institution endorses a
   claim" one. A useful follow-on cell.

3. **A failing gate.** Both AutoOnLoad gates produce `Holds`. A version
   of cell 20 with `objective_value: -1.0` (unreachable for the
   bounded SSE) would surface a `Fails` Verdict and cause the entire
   commit to be rejected, demonstrating the chain-state semantics of
   gate rejection. Useful as a follow-on demo.

These are notebook gaps, not platform gaps — the underlying machinery
supports them. The kinase notebook covers the *minimum viable* multi-institution
walkthrough; richer notebooks could exercise the rest.

---

Next: **[7. Statistics + reasoning walkthrough →](07-stats-and-reasoning-walkthrough.md)**
