# 4. The three dispatch roles in concert

The three QueryClass dispatch roles — `AutoOnLoad`, `OnDemand`, and
`Decidable` — are introduced individually in
[ESL §9](../esl/09-institutions.md) and
[EigenQL §8](../eigenql/08-institutions.md). This chapter covers how they
*interact*: what a coordinated dispatch flow looks like when multiple gates
fire across multiple institutions in response to a single chain commit, and
how a downstream OnDemand or Decidable call can read what an earlier
AutoOnLoad gate produced.

The chapter is organised around three patterns plus a worked dispatch flow
from the kinase notebook.

## 4.1. Recap: the three dispatch roles

Each `QueryClass` declares a `dispatch_role` set drawn from
`{AutoOnLoad, OnDemand, Decidable}` (D14 §6.1–§6.2). A single QueryClass can
carry multiple roles; the most common shapes are summarised in the
classification tables in [ESL §9.8](../esl/09-institutions.md#98-the-classification-table-cross-language)
and [EigenQL §8.9](../eigenql/08-institutions.md#89-the-classification-table).
A one-line summary:

| Role | Trigger | Returns | Caller |
|---|---|---|---|
| **AutoOnLoad** | Every commit of a resource of the bound class | `Verdict` (Holds / Fails / Undecidable); Fails rejects the commit | Kernel commit pipeline |
| **OnDemand** | Explicit `FIBER ... AS ?var` clause in EigenQL | A typed resource of the QC's `result_class` | Query authors |
| **Decidable** | `inst:predicate(args)` in expression position; postfix `HOLDS` / `FAILS` / `UNDECIDABLE` projects to Boolean | `Verdict` (projected to Boolean by the postfix predicate) | ESL programs (during type-check) and EigenQL (in WHERE / RETURN) |

The roles aren't mutually exclusive at the language level — a QueryClass can
expose itself as Decidable in expression position *and* as OnDemand via FIBER,
each routing to the same handler with different input shapes. The kinase
notebook exercises each role at least once across its three storylines.

## 4.2. AutoOnLoad cascades — single commit, multiple gates

The simplest cross-institution flow is an **AutoOnLoad cascade**: a single
chain commit triggers multiple AutoOnLoad gates, each from a different
institution, in commit order. The kinase notebook's two storylines each fire
exactly one gate, but the structure naturally extends to more.

### How the cascade works

The kernel's commit pipeline scans the about-to-commit resources for any
class that has an `AutoOnLoad` QueryClass bound to it. For each match, it
dispatches the gate's handler (synchronously) and commits the Verdict +
RuntimeInvocation alongside the gated resource. If the Verdict is `Fails`,
the entire commit is rejected — gating the chain at the *aggregate* level,
not per-resource.

For a single-resource commit (the common case), there's at most one gate per
class. For a multi-resource commit (e.g. an ESL load with several `resource`
declarations), each resource's gate runs in source order. A `Fails` from any
gate aborts everything that hasn't yet committed.

### Cross-institution cascades

When the multi-resource commit spans classes from multiple institutions,
each gate runs in its own institution. From the kinase notebook's bridge
narrative: a hypothetical commit of three resources — a Catalyst
`ReactionNetwork`, a derived `OdeProblem` (from the Catalyst → DiffEq
comorphism's reify output), and an `OdeSolution` claim — could fire three
gates in sequence:

1. Catalyst's `validate_conservation_laws` AutoOnLoad on the `ReactionNetwork`.
2. (No AutoOnLoad on `OdeProblem` itself.)
3. DiffEq's `validate_solution` AutoOnLoad on the `OdeSolution`.

Each runs in its own institution's runtime (separate Julia worker
containers). Each produces a `Verdict + RuntimeInvocation` commit. The
chain's audit trail captures the full sequence as a partial order rooted at
the original commit.

The kinase notebook keeps storylines 1 and 2 separate (each storyline
commits its own gated resource and inspects its own Verdict) for clarity.
The structural shape — a single commit triggering a cross-institution
cascade — is what the platform supports natively.

### The D52 → D39 cascade

The motivating example for the cross-institution cascade in the Layer 2
reasoning stack is the **statistics → reasoning** pipeline:

1. **D52 fires first.** A commit of a `stats:StatisticalAnalysisPlan` resource
   triggers the statistics institution's `validate_analysis_plan`
   AutoOnLoad gate. The verifier recomputes the claim from the cited
   `SampleSet`'s raw replicates, returns `Verdict::Holds`, and (as a side
   effect of the trace + canonical_proposition pair already on the chain)
   the layer's witness index admits an `IsDerivedAs(claim_iri,
   canonical_proposition)` entry.
2. **D39 fires next.** If the same commit also includes a
   `reasoning:ReasoningSentence` whose certificate cites the just-committed
   claim via `DerivedEvidence(claim_iri)`, the reasoning institution's
   `validate_justification` AutoOnLoad gate fires. The kernel's NbE checker
   walks the certificate's `JustifiedBy.derived` constructor, consults the
   layer's witness index for the matching `IsDerivedAs` entry — which the
   D52 commit just admitted — and the certificate type-checks.

The cascade is **mechanical, not coordinated**. D52 doesn't know D39 is
about to fire; D39 doesn't know D52 ran. They share the chain artifact
shape — `DerivedResource` + `ProgramTrace` + `canonical_proposition` —
that the witness index reads from. The composition emerges from each
institution honouring the shared chain shape independently.

For the operational walkthrough — the full sequence of EigenQL inspection
calls that surface the cascade's audit trail — see
[chapter 7](07-stats-and-reasoning-walkthrough.md).

### What rejection looks like

If any gate in the cascade returns `Fails`, the kernel rejects the entire
commit. Resources that hadn't yet been written stay unwritten; resources
that had already committed (in a previous transactional layer) stay
committed. The commit response carries the rejecting gate's diagnostic.

For the chain's audit story: even on rejection, the Verdict + RuntimeInvocation
provenance is committed to a *side layer* (not the main chain), so the
"why was this commit rejected?" question has a recorded answer. See
[D14 §6.1](../../design/d14-institution-realisation.md) for the full
rejection-with-audit contract.

## 4.3. OnDemand FIBER reading what AutoOnLoad produced

AutoOnLoad gates produce Verdict + RuntimeInvocation chain residents. An
OnDemand FIBER call later in a query can match against those Verdicts —
either to filter a result set, to feed a downstream institution's QueryClass,
or to drive a Decidable predicate.

The kinase notebook's cell 22 is the simplest version of this pattern:

```eigenql
MATCH "urn:eigenius:institution:Verdict"(?v) {
    "urn:eigenius:core:ctor_name":                   ?ctor,
    "urn:eigenius:institution:verdict_subject":      ?subject,
    "urn:eigenius:institution:verdict_query_class":  ?qc
}
RETURN [] {
    verdict:     ?v,
    ctor:        ?ctor,
    subject:     ?subject,
    query_class: ?qc
}
ORDER BY ?ctor
```

This is read-only — pulls every Verdict on the chain plus its gated subject
and query class. After running the kinase notebook fresh, two rows: one
from the DiffEq AutoOnLoad gate, one from the JuMP-HiGHS AutoOnLoad gate.

The richer pattern uses an OnDemand FIBER call to feed the matched Verdicts
into a downstream institution's QueryClass. Schematic:

```eigenql
USING INSTITUTION "urn:my-org:institutions:risk" AS risk

MATCH "urn:eigenius:institution:Verdict"(?v) {
    "urn:eigenius:core:ctor_name":               ?ctor,
    "urn:eigenius:institution:verdict_subject":  ?gated_resource
}
WHERE ?ctor = "Holds"
FIBER risk:assess_consequences {
    upstream_verdict: ?v,
    gated_resource:   ?gated_resource
} AS ?risk_assessment
RETURN [] { resource: ?gated_resource, risk: ?risk_assessment }
```

The FIBER call reads each `Holds` Verdict from the prior gate, hands it
plus its gated subject to a downstream `risk:assess_consequences`
QueryClass. The risk institution's response (a `risk_assessment` resource)
is bound to `?risk_assessment` and visible to the rest of the query. No
chain reinsertion — the Verdict was already committed by the upstream gate;
the FIBER response lives in the transient overlay (chapter 5 covers when to
reinsert via `INTO`).

The structural fact: OnDemand FIBER consumes whatever the chain currently
holds. AutoOnLoad gates put Verdicts on the chain. So FIBER can read what
previous AutoOnLoad gates produced — the dispatch surfaces compose without
either knowing about the other.

## 4.4. Decidable predicates as compositional filters

The Decidable role is what lets a *constraint* fire in expression position:

```eigenql
WHERE assay:within_dose_range(?compound, ?dose) HOLDS
```

The predicate `assay:within_dose_range` is a Decidable QueryClass. The
kernel resolves it to its institution's runtime, builds a synthetic input
resource from the positional args, dispatches the handler, and projects the
returned Verdict to a Boolean via the postfix `HOLDS` predicate.

For composition, three patterns matter:

### Decidable as a filter on prior FIBER results

```eigenql
USING INSTITUTION "urn:eigenius:institutions:symbolics" AS symb
USING INSTITUTION "urn:eigenius:institutions:intervals" AS intv

FIBER symb:qc_symb_simplify {
    expr: "urn:eigenius:notebook:kinase_demo:sse_expr"
} AS ?simplified
WHERE intv:bounded_by_in_range(?simplified, 0.0, 100.0) HOLDS
RETURN [] { simplified: ?simplified }
```

The FIBER call invokes Symbolics' OnDemand simplifier; the WHERE then runs
IntervalArithmetic's Decidable bound check on the simplified expression.
Two institutions, two roles, one query.

### Decidable in ESL programs as a constraint that gates type-check

ESL programs can use a Decidable QueryClass as a property constraint that
fires during type-check reduction (`NativeDecide`). Composition kicks in
when the property's value comes from a comorphism reify output:

```esl
program nb:assess_fit
    : symbolics:SymbolicsToJuMPInput -> ResourceWithCovenantCheck
{
    let problem : jump:OptimisationProblem =
        comorphisms:symbolics_to_jump(input);
    let optimum : jump:OptimisesTo =
        run_solver(problem);
    // Decidable predicate fires at type-check time.
    let _ : Verdict =
        treasury:dscr_above_threshold(optimum.objective_value, 1.25);
    Construct ResourceWithCovenantCheck {
        problem = problem,
        optimum = optimum
    }
}
```

The comorphism (chain reinsertion via `Exp::InstitutionInvoke`) produces
the `OptimisationProblem`; the OnDemand call (a hypothetical `run_solver`)
produces the `OptimisesTo`; the Decidable predicate (`dscr_above_threshold`)
fires at type-check and gates the program's compilation. Three institutions
in three roles, all on the same chain.

### Decidable producing the *value* a downstream comorphism reads

Decidable QueryClasses return Verdicts. A Verdict resource is itself
chain-typed; another comorphism can be written to consume Verdicts and
produce something else (e.g. a `RiskScore` for a downstream risk-assessment
institution). Whether to wire that up depends on whether the downstream
domain's ontology has a natural place for Verdict-as-input — most don't,
but some compliance / audit-tooling institutions naturally would.

## 4.5. Write-side coordination via chain reinsertion

The patterns above are all *read-side* — one role consumes what a previous
role wrote. The complementary pattern is *write-side*: a comorphism's reify
output (chapter 5) becomes the trigger for the next gate downstream.

The shape:

1. ESL program invokes a comorphism via `comorphisms:foo(input)`.
2. The comorphism's pipeline runs (extract → transform → reify).
3. The reify output commits to the chain at a deterministic content-hash
   IRI (`urn:eigenius:comorphism-output:foo:<hex>`).
4. The commit triggers any AutoOnLoad gate bound to the produced class.
5. The gate's Verdict commits to the chain alongside the produced resource.

Step 4 is what makes chain reinsertion the linchpin of multi-step
pipelines. Without it, a comorphism's output is transport-only — visible to
the immediate caller, gone after the dispatch returns. With it, the output
is a first-class commit that triggers all the downstream machinery the
chain has wired up.

The kinase notebook's Part C (cells 30–35) exercises this: cell 30's ESL
wrapper program invokes `comorphisms:symbolics_to_jump`; the produced
`OptimisationProblem` commits at `urn:eigenius:comorphism-output:symbolics_to_jump:<hex>`;
the chain has no AutoOnLoad gate bound to `OptimisationProblem` (only
`OptimisesTo` is gated), so the cascade stops there. But the structure is
in place — adding a gate to `OptimisationProblem` would extend the cascade
without changing any existing code.

Chapter 5 covers chain reinsertion in detail.

## 4.6. A worked dispatch flow: kinase Storyline 2 step by step

The richest dispatch flow in the kinase notebook is Storyline 2 (cells
19–22). The user-facing action is a single chain commit (cell 20's ESL
load). What actually happens in the kernel:

```
USER ACTION
    eigenius load (cell 20):
      - nb:sse_expr      : SymbolicExpression
      - nb:ki_bound      : VariableBound
      - nb:fit_input     : SymbolicsToJuMPInput
      - nb:opt_problem   : OptimisationProblem
      - nb:optimises_to  : OptimisesTo

KERNEL COMMIT PIPELINE
    1. Validate each resource against its class shape.
       (chain-side validation: 5 resources × per-class rules)
    2. Scan for AutoOnLoad gates.
       Match: nb:optimises_to.is_a includes jump:OptimisesTo,
              JuMP-HiGHS's `validate_optimum` is bound to it.
    3. Dispatch the gate.
       3a. Build extract input: nb:optimises_to with embedded
           nb:opt_problem (IRI deref through the property graph).
       3b. RPC to orchestrator: DispatchExternal(env_iri, image_digest,
           method_name=validate_optimum, signature, [optimises_to_cbor]).
       3c. Orchestrator → substrate addon → JuMP-HiGHS Julia worker.
       3d. Worker decodes input via mirror; runs validate_optimum:
           - Builds Model(HiGHS.Optimizer)
           - Walks objective FormulaTerm under formula_to_jump (smart-pow on)
           - Walks variable bounds, sets via set_lower_bound / set_upper_bound
           - Calls optimize!
           - Reads back termination_status, objective_value, value(Ki)
           - Compares against claim within max(abstol, reltol·|actual|)
           - Returns _verdict("Holds")
       3e. Worker → orchestrator → kernel: Verdict resource (CBOR).
    4. Commit the original 5 resources + the Verdict + the RuntimeInvocation.

CHAIN STATE AFTER COMMIT
    - 5 user resources, all reachable by the IRIs the cell named
    - 1 Verdict at urn:eigenius:invocation:<uuid>:verdict (Holds)
    - 1 RuntimeInvocation at urn:eigenius:invocation:<uuid> (image
      digest, runtime version, started_at/completed_at, numerical_metadata)

DOWNSTREAM (cells 21, 22, 24, 25)
    - cell 21: EigenQL pulls the Verdict by gated subject
    - cell 22: aggregate Verdicts query (this and the DiffEq gate's verdict)
    - cell 24: gate-endorsed values query (claim + Verdict ctor)
    - cell 25: RuntimeInvocation provenance query (image digest, timing)
```

Three institutions are touched (Symbolics for the expression's class
ontology, JuMP-HiGHS for the gate, and implicitly the formulas namespace
for FormulaTerm itself); three roles fire (the AutoOnLoad gate at commit
time; the OnDemand machinery from cells 21/22 querying the produced
Verdict; the RuntimeInvocation provenance closure). All within one
user-facing chain commit + a few read-only queries.

The pattern is general: a multi-institution composition is what *the chain*
sees, not what the user types. The user types one ESL load and gets a
cascade of gates, verdicts, and provenance entries — all of it
reproducible, all of it inspectable via EigenQL.

---

Next: **[5. Chain reinsertion of comorphism outputs →](05-chain-reinsertion.md)**
