# Statistics institution tutorial

Slow-walk worked example of D52, the platform's measurement-statistics institution. Walks the full chain from raw replicate readings on the chain to a typed Holds/Fails verdict mechanically derived from those readings.

Read this if you want to know what a `StatisticalAnalysisPlan` commit does, how the seven dispatch positions split across the experimental-design space, what each opinionated stance (one-sided witnessing, dual-verdict outlier exclusion, Passing-Bablok for method comparison, epistemic-scope guard) actually enforces, or how the institution's emitted `DerivedResource` becomes a citable evidence node for the [D39 reasoning institution](../reasoning-institution/README.md).

Design spec: [**D52 Measurement Statistics Institution**](../../../design/d52-measurement-statistics-institution.md). Implementation: [`crates/eigenius-statistics/`](../../../../crates/eigenius-statistics/). Ontology: [`ontologies/statistics/statistics.esl`](../../../../ontologies/statistics/statistics.esl).

## Why D52 is different from the numerical and verification institutions

Three institution families are now in the platform:

- **Numerical institutions** ([Symbolics, Catalyst, DiffEq, JuMP-HiGHS, IntervalArithmetic](../julia-institutions/)) — execute typed expression trees (`formulas:FormulaTerm`) against a hosted runtime (Julia, in v1). The institution returns a computed result; the user trusts the runtime to evaluate.
- **Verification institutions** ([Lean](../lean-institution/README.md)) — re-check formal proofs the user authored externally. The institution returns Holds iff the proof type-checks against the named theorem; the user wrote the proof.
- **Statistics institutions** (D52) — re-compute a statistical claim from raw replicate data. The institution returns Holds iff the asserted claim parameters are consistent with the recomputation; the user wrote neither the proof nor the runtime, only the *claim* (alpha, effect-size, threshold) and the raw data.

The difference matters for the audit story. A numerical institution's verdict ("the simulation produced these outputs") is a recorded fact about a computation. A verification institution's verdict ("the proof checks") is a fact about a mathematical artifact. A statistics institution's verdict ("the claim is supported by the data at α = 0.05") is a fact about the *logical relationship between the claim and the evidence* — recomputed deterministically from raw data, so the chain can attest the relationship holds without trusting the author's interpretation of their own measurements.

The institution lives in-process: the verifier runs synchronously inside the kernel via the `eigenius-statistics` crate using deterministic IEEE-754 numerics (`ndarray` + `statrs`). No external runtime, no orchestrator round-trip. The verdict is a direct function call inside the kernel process; the TCB is bounded by the kernel and the numerics crates.

## The universal claim schema

Every `stats:StatisticalAnalysisPlan` resource carries the same seven slots, plus an optional autocorrelation structure for longitudinal designs:

| Property | Type | What it asserts |
|---|---|---|
| `sample_set` | IRI of a `SampleSetResource` | The raw replicates the verifier recomputes against. |
| `null_hypothesis` | EigenTT proposition | The null the test is rejecting against — propagated to the verdict for audit. |
| `alternative_hypothesis` | EigenTT proposition | The alternative the test is asserting — used for diagnostic shape. |
| `canonical_proposition` (`reflection:` inherited) | EigenTT proposition | The predicate the claim establishes when the test holds. This is what downstream `DerivedEvidence` consumers read; the witness index hashes it. |
| `alpha` | Float | Type-I error threshold, unadjusted. Multiple-testing correction is a higher-level institution. |
| `effect_size` | `Absolute(magnitude, units)` / `Relative(ratio)` / `StandardizedCohensD` / `StandardizedHedgesG` | The asserted effect — for `SingleSampleEstimate`, the threshold the mean must cross. |
| `directionality` | `TwoSided()` / `OneSidedWitnessed(witness_iri)` | Whether the test is two-sided (the safe default) or one-sided with a chain-resident impossibility witness ([§7.1](#7-1-opinionated-stance-onesidedwitnessed-requires-an-impossibility-witness)). |
| `variance_assumption` | `Pooled` / `WelchUnequal` / `NonParametric` / `RankBased` | Which test family dispatches; author-asserted so the verifier output is fully deterministic. |
| `outlier_exclusion` | `Identity` / `ESD(k, α)` / `PassingBablokResidual(σ)` / `Manual(witnesses)` | Whether and how to drop outliers; non-`Identity` triggers the dual-verdict shape ([§7.2](#7-2-opinionated-stance-dualverdict-outlier-exclusion)). |
| `autocorrelation_structure` (optional) | `AR1` / `CompoundSymmetry` / `Unstructured` | Required for `RepeatedMeasures` dispatches; author-asserted so the verifier doesn't have to fit it iteratively. |

The schema is the *intersection of standards* — every cell satisfies a specific requirement from ARRIVE, STROBE, CLSI EP09, or CLSI EP05. The author asserts; the verifier checks. No field defaults silently — missing required fields are commit errors with structured diagnostics.

## The seven SampleSet dispatch positions

Each `stats:SampleSet` value is a `Bundle(...)` ctor at a specific position in the 5-axis experimental-design product space: `(Randomization, Blocking, Factor, Replication, RepeatedMeasures)`. The verifier reads the position, picks the matching dispatch arm, decodes the observations payload, and runs the matching numerics routine. Smart-constructor macros ([ESL §4.9](../../esl/04-declarations.md#4-9-macro-compile-time-smart-constructors-d52-12)) give the author a compact authoring surface; the position they land at is documented per macro.

| Smart constructor | Product position | Test family |
|---|---|---|
| `stats:SingleSampleEstimate(observations, replication)` | `(CompleteRandom, Unblocked, NoFactor, _, CrossSectional)` | One-sample t-test against `effect_size = Absolute(threshold, units)`. The IC50 case. |
| `stats:IID(group_a, group_b, replication)` | `(CompleteRandom, Unblocked, SingleFactor, _, CrossSectional)` | Two-sample t-test under `variance_assumption` (Pooled / Welch / Mann-Whitney / rank). |
| `stats:Paired(pairs, replication)` | `(CompleteRandom, PairedBlocking, SingleFactor, _, CrossSectional)` | Paired t-test (or Wilcoxon signed-rank). Distinct constructor surface so treating paired data as IID — the most common false-positive-inducing error in the literature — fails at the call site. |
| `stats:Factorial(k, factor_levels, observations, replication)` | `(CompleteRandom, Unblocked, FullFactorial(k), _, CrossSectional)` | k-way omnibus ANOVA. Per-effect decomposition is a follow-on hardening; v1 reports the single F-statistic + p-value. |
| `stats:RCBD(n_blocks, n_treatments, observations, replication)` | `(Restricted, RCB(n_blocks), SingleFactor, _, CrossSectional)` | Randomized Complete Block Design — two-way ANOVA controlling for block effect, reports the treatment F-test. Catches "treated paired/blocked data as IID" with the `RCB(n_blocks ≥ 3)` discipline. |
| `stats:SplitPlot(a, b, r, observations, replication)` | `(Restricted, SplitPlotBlocking(a, r), FullFactorial(2), _, CrossSectional)` | Split-plot mixed-effects with **nested error strata** — whole-plot factor tested against whole-plot error, subplot factor and interaction tested against subplot error. The distinct `SplitPlotBlocking(a, r)` ctor makes the nested dispatch unambiguous; otherwise routing split-plot data through flat `Factorial` would use the smaller subplot error for the whole-plot F-test and silently produce inflated significance. This false-positive shield is one of the institution's primary justifications. |
| `stats:RepeatedMeasures(n_subjects, n_timepoints, k_between_factors, factor_levels, observations, replication)` | `(CompleteRandom, Unblocked, FullFactorial(k_between_factors), _, Longitudinal(n_timepoints))` | Longitudinal mixed-effects with subject as random effect, time as within-subjects fixed factor, optional between-subjects factorial overlay. Phase 4.9 wires the `(CompoundSymmetry, k=0)` cell (univariate RM-ANOVA); other cells of the (autocorrelation × k_between_factors) matrix reject with diagnostics referencing tracked GitHub issues. |

A `MethodComparisonAnalysisPlan` subclass dispatches differently: it bypasses the SampleSet-shape table and routes to Passing-Bablok regression on the cited `Paired` SampleSet's observations ([§7.3](#7-3-opinionated-stance-passingbablok-mandatory-for-methodcomparisonclaim)).

The full dispatch table is at [D52 §5.4](../../../design/d52-measurement-statistics-institution.md). The verifier's per-dispatch arm implementations live in [`crates/eigenius-statistics/src/validate.rs`](../../../../crates/eigenius-statistics/src/validate.rs); the numerics routines live in [`crates/eigenius-statistics/src/numerics.rs`](../../../../crates/eigenius-statistics/src/numerics.rs).

## The four-step `validate_analysis_plan` check

AutoOnLoad fires `validate_analysis_plan` on every `StatisticalAnalysisPlan` commit. The kernel rejects the commit if any step fails.

1. **Resolve + decode the SampleSet.** Read the claim's `sample_set` IRI, resolve to a `SampleSetResource` on the chain, read its `sample_set_value` (a chain-mirrored `Bundle(...)` inductive), decode the 9 args into a typed `DecodedBundle` Rust struct (randomization, blocking, factor, replication, repeated_measures, units, columns, sample_map, observations). Malformed bundles produce structured diagnostics naming the offending slot.

2. **Read claim parameters.** Read `alpha`, `directionality`, `effect_size`, `variance_assumption`, `outlier_exclusion`, optional `autocorrelation_structure`. Validate directionality (TwoSided allowed always; OneSidedWitnessed requires the chain-witness check from [§7.1](#7-1-opinionated-stance-onesidedwitnessed-requires-an-impossibility-witness) for t-based dispatches only). Validate outlier-exclusion routing (per the dispatch matrix in [§7.2](#7-2-opinionated-stance-dualverdict-outlier-exclusion)).

3. **Dispatch on the product position.** Match on the bundle's `(randomization, blocking, factor, repeated_measures)` ctor names, pick one of the seven dispatch arms (or fall through to `MethodComparisonAnalysisPlan` if the claim's `is_a` carries that marker). Each arm decodes the observations payload per its expected shape, runs the matching numerics routine, and returns a `(statistic, p_value, diagnostic_note)` tuple.

4. **Check the §7.4 epistemic-scope and emit the verdict.** Walk the claim's `canonical_proposition`'s head predicate, look up its `is_a` markers (`PopulationLevel` / `MeasurementLevel`), and confirm the SampleSet's replication kind admits propositions of that scope ([§7.4](#7-4-opinionated-stance-technicalonly-replicates-cannot-support-populationlevel-propositions)). Compare the test's p-value against `alpha` (halved if OneSidedWitnessed). Emit a `Verdict::Holds` resource if `p < alpha`, else `Verdict::Fails` with a structured `AlphaNotCrossed` diagnostic.

All four must pass for `Verdict::Holds`. Any failure produces `Verdict::Fails` with a typed diagnostic; the commit is rejected. The Holds verdict's resource carries the computed statistic + p-value in the standard `(stats:computed_statistic, stats:computed_p_value)` slots, plus any per-dispatch diagnostic note (e.g., the SplitPlot omnibus diagnostic naming which of three F-tests produced the reported p-value, or the dual-verdict note from §7.2 enumerating both with-exclusion and without-exclusion numerics).

## The four opinionated stances (§7 hardenings)

The prior-art survey identified four field-wide conflicts where competing standards disagree. The institution adopts an opinionated default rather than mirroring the disagreement, because mirroring would let the wrong choice ride into the chain unchallenged.

### 7.1. Opinionated stance: OneSidedWitnessed requires an impossibility witness

`directionality` defaults to `TwoSided()`. To assert `OneSidedWitnessed(witness_iri)`, the claim must reference a chain resource carrying `is_a stats:ImpossibilityWitness` — a marker class declaring "the inverse direction of this hypothesis is physically or mathematically impossible within the system under study" (e.g., a half-life cannot be negative; a probability cannot exceed 1).

The verifier admits the one-sided p-value path (halve the two-sided p for the alpha comparison) only when the witness IRI resolves to such a resource. The witness's structural existence on chain — *not* the test statistic's sign — is what authorizes the halving. If the witness IRI doesn't resolve, or resolves to a resource without the `ImpossibilityWitness` marker, the claim is rejected with a structured `MissingImpossibilityWitness` diagnostic.

F-based dispatches (Factorial, RCBD, SplitPlot, RepeatedMeasures) reject `OneSidedWitnessed` outright: F-statistics are intrinsically non-negative, so the one-sided / two-sided distinction doesn't refine them.

Implementation: `check_impossibility_witness` in [`crates/eigenius-statistics/src/validate.rs`](../../../../crates/eigenius-statistics/src/validate.rs); `DispatchPos::supports_one_sided_directionality()` captures the t-based / F-based split. ARRIVE-aligned stance; legacy software's silent one-sided defaults are rejected.

### 7.2. Opinionated stance: dual-verdict outlier exclusion

The `SampleSet` carries every replicate the bench produced — outlier exclusion is *not* a property of the SampleSet, it's a property of the *claim*. When a claim carries a non-`Identity` exclusion functor, the verifier computes the test twice — once with the functor applied, once on the raw samples — and reports both outcomes. v1 packs both into a single diagnostic string under a `DualVerdict` label; the v2 [tracked follow-on](https://github.com/eigenius/eigenius/issues/82) materializes two `MeasurementVerdict` resources linked via `stats:dual_verdict_pair` so downstream consumers can resolve each branch independently.

Three exclusion functors are exposed:

- **`Identity()`** — no exclusion. Standard single-verdict path.
- **`ESD(max_outliers, alpha_esd)`** — Rosner's generalized Extreme Studentized Deviate test (1983). Iteratively flags up to `max_outliers` observations using Studentized deviates against critical values from the one-sided t distribution.
- **`PassingBablokResidual(threshold_sigma)`** — residuals from a Passing-Bablok regression, used in CLSI EP09 method-comparison. Only meaningful for `MethodComparisonAnalysisPlan` dispatches.
- **`Manual(witnesses)`** — typed exclusion witnesses referencing committed assay-quality observations. Deferred to the §11 assay-quality institutions; v1 rejects.

Phase 5 v1 wires the `(SingleSampleEstimate, ESD)` cell completely; other (dispatch × non-Identity exclusion) combinations reject up front with structured diagnostics referencing the [tracked GitHub issues](https://github.com/eigenius/eigenius/issues/80) per the (dispatch × exclusion) matrix in [D52 §9 Phase 5](../../../design/d52-measurement-statistics-institution.md). STROBE-aligned sensitivity-analysis stance; storing only the post-exclusion result is the same epistemic loss as storing only the summary statistic, structurally prevented.

### 7.3. Opinionated stance: Passing-Bablok mandatory for `MethodComparisonAnalysisPlan`

`stats:MethodComparisonAnalysisPlan : stats:StatisticalAnalysisPlan` is a subclass that triggers a class-based early dispatch: when the claim's `is_a` contains the marker, the verifier bypasses the SampleSet-shape table and routes to **Passing-Bablok regression** (non-parametric, robust to outliers, errors-in-both-variables). Ordinary least-squares regression is rejected outright — OLS assumes zero measurement error on the X-axis, which for two biological measurements compared against each other is structurally false. Deming regression with an asserted variance ratio is acceptable but a follow-on.

The SampleSet shape mirrors `stats:Paired(pairs, replication)`: each pair is `(method_a_reading, method_b_reading)` for one specimen. The verdict criterion is **CI-based, not p-value-based**: Holds iff `1.0 ∈ slope_CI ∧ 0.0 ∈ intercept_CI` (CLSI EP09 method-agreement criterion). The verdict's `computed_statistic` carries the median slope; `computed_p_value` carries a binary disagreement indicator (0.0 on agreement, 1.0 on disagreement); the diagnostic enumerates both CIs.

A second `QueryClass` resource binds `stats:MethodComparisonAnalysisPlan` to the same `validate_analysis_plan` handler so AutoOnLoad fires on subclass instances — the kernel's dispatch matches `resource.is_a()` entries directly against registered query_class IRIs without transitive subclass walks, so the subclass needs its own registration. CLSI EP09-aligned.

### 7.4. Opinionated stance: technical-only replicates cannot support population-level propositions

The SampleSet's `replication` axis is consulted at every dispatch for variance-component stratification (CLSI EP05's repeatability vs intermediate precision). It is *also* consulted at claim-admissibility time:

- **`BiologicalReplication`** — any `canonical_proposition` shape is admissible (subject to the other verifier checks).
- **`TechnicalWithinRun`** — only `canonical_proposition` shapes whose predicate carries `is_a stats:MeasurementLevel` are admissible. Population-level propositions are rejected with `EpistemicScopeViolation { sample_replication: TechnicalWithinRun, proposition_scope: PopulationLevel }`.
- **`NestedReplication(biological_n, technical_per_biological)`** — population-level propositions admissible; the verifier uses CLSI EP05-A3 nested ANOVA to stratify within-run vs intermediate-precision variance.

The scope of a proposition is determined from its head predicate's `is_a` class memberships. Domain ontologies mark predicates via the [multi-class `data` header](../../esl/04-declarations.md#4-5a-multi-class-data-declarations-marker-classes-d52-12) form:

```esl
data screen:HasLowIC50 : core:string -> Prop, stats:PopulationLevel { }
data assay:HasLowIC50_OnThisBatch : core:string -> Prop, stats:MeasurementLevel { }
```

Predicates with no scope marker default to `PopulationLevel` (the more restrictive admissibility — fail-safe). The institution exists to prevent the trust-the-summary problem; silently admitting "EIG_0291 has IC50 < 100 nM" from three reads of one plate would re-introduce exactly that problem.

## Walking the audit chain — IC50 worked example

The fixture at [`crates/eigenius-statistics/tests/fixtures/ic50_measurement.esl`](../../../../crates/eigenius-statistics/tests/fixtures/ic50_measurement.esl) walks the cycle from raw replicate readings to a verdict. Read forward:

```text
HasLowIC50 predicate                       [data : Prop, stats:PopulationLevel]
  ↑ canonical_proposition
m_eig0291_sampleset                        [SampleSetResource]
  │ ↑ sample_set_value
  │  stats:SingleSampleEstimate(
  │    [72.0, 85.0, 100.0],
  │    BiologicalReplication()
  │  )                                     [Bundle ctor at (CompleteRandom, Unblocked,
  │                                                          NoFactor, BiologicalReplication,
  │                                                          CrossSectional)]
  │ ↑ resource
  │  m_eig0291_sampleset_trace             [ObservationTrace — admits IsObservedAs]
  │
  ↑ sample_set
claim_eig0291_lowic50                      [StatisticalAnalysisPlan]
  │ alpha = 0.05
  │ effect_size = Absolute(100.0, "nM")
  │ directionality = TwoSided()
  │ variance_assumption = WelchUnequal()
  │ outlier_exclusion = Identity()
  │ canonical_proposition = HasLowIC50("urn:...:EIG_0291")
  │
  ↑ validate_analysis_plan AutoOnLoad
  │   1. Resolve SampleSet → decode Bundle
  │   2. Read claim params; no impossibility witness needed (TwoSided)
  │   3. Dispatch on (CompleteRandom, Unblocked, NoFactor, CrossSectional)
  │      → SingleSampleEstimate → one_sample_t_test([72.0, 85.0, 100.0], 100.0)
  │      → t = -1.776, p_two_sided ≈ 0.218
  │   4. §7.4 epistemic scope: BiologicalReplication admits PopulationLevel ✓
  │      Compare p < alpha: 0.218 < 0.05 → False → Verdict::Fails
  │
Verdict("Fails", AlphaNotCrossed: computed p = 0.218..., threshold alpha = 0.05)
```

The IC50 from three replicate readings doesn't cross the threshold at α = 0.05 — the standard deviation across (72, 85, 100) is too large for the n = 3 sample to reject the null. The same fixture commits a *confirmatory* SampleSet with n = 6 tightly clustered around 85 nM and a corresponding claim; that one produces Holds with p ≪ 0.05. The cycle closes through the `canonical_proposition` slot: the verdict's resource carries the predicate `HasLowIC50("urn:...:EIG_0291")`; the [D49 witness index](../reasoning-institution/README.md#the-d49-witness-index-how-the-kernel-admits-grounding-witnesses) reads it to admit `IsDerivedAs(claim_iri, HasLowIC50(...))`; downstream [D39 reasoning sentences](../reasoning-institution/README.md) cite the claim via `DerivedEvidence` and consume the witness via `JustifiedBy.derived`.

Every byte that went into the verification — the three raw IC50 readings, the asserted parameters, the recomputation procedure, the resulting verdict — sits on the chain as a typed, queryable, content-addressed resource. The verdict is reproducible: you can re-run `validate_analysis_plan` against the same chain state and get bit-identical numerics, because the institution uses deterministic IEEE-754 arithmetic.

## Authoring your own claim

The high-level shape, modeled on the IC50 fixture:

1. **Mark the predicate's scope.** Use the [multi-class `data` header](../../esl/04-declarations.md#4-5a-multi-class-data-declarations-marker-classes-d52-12) form to declare whether the predicate is population-level or measurement-level:

   ```esl
   data screen:HasLowIC50 : core:string -> Prop, stats:PopulationLevel { }
   ```

2. **Commit the SampleSetResource carrying raw replicates.** Use the smart constructor that matches your experimental design — `SingleSampleEstimate` for threshold-against-one-mean cases, `IID` for two-group comparisons, `Paired` for matched-pairs, etc.:

   ```esl
   resource screen:m_eig0291_sampleset : stats:SampleSetResource {
       reflection:source      = "instrument-log:kinase-glo-plate-2026-03-04-A1";
       reflection:observed_at = "2026-03-04T14:22:11Z";

       stats:sample_set_value = stats:SingleSampleEstimate(
           [72.0, 85.0, 100.0],
           BiologicalReplication(),
       );
   }

   resource screen:m_eig0291_sampleset_trace : reflection:ObservationTrace {
       reflection:resource  = screen:m_eig0291_sampleset;
       reflection:source    = "instrument-log:kinase-glo-plate-2026-03-04-A1";
       reflection:timestamp = "2026-03-04T14:22:11Z";
   }
   ```

3. **Author the StatisticalAnalysisPlan.** Fill in the universal-claim schema. Use [`type_expr(...)`](../../esl/05-expressions.md#5-14a-type_expr-eigentt-type-expressions) for the `Prop`-typed proposition slots; literal ctors (`Absolute`, `TwoSided`, etc.) for the sum-typed parameter slots:

   ```esl
   resource screen:claim_eig0291_lowic50 : stats:StatisticalAnalysisPlan {
       stats:sample_set = screen:m_eig0291_sampleset;

       stats:null_hypothesis = type_expr(
           screen:HasLowIC50("urn:eigenius:demo:screen:EIG_0291")
       );
       stats:alternative_hypothesis = type_expr(
           screen:HasLowIC50("urn:eigenius:demo:screen:EIG_0291")
       );
       reflection:canonical_proposition = type_expr(
           screen:HasLowIC50("urn:eigenius:demo:screen:EIG_0291")
       );

       stats:alpha = 0.05;
       stats:effect_size = Absolute(100.0, "nM");
       stats:directionality = TwoSided();
       stats:variance_assumption = WelchUnequal();
       stats:outlier_exclusion = Identity();
   }

   resource screen:claim_eig0291_lowic50_trace : reflection:ProgramTrace {
       reflection:resource  = screen:claim_eig0291_lowic50;
       reflection:source    = "statistics-institution:validate_analysis_plan";
       reflection:timestamp = "2026-03-04T14:22:11Z";
   }
   ```

4. **Commit.** Load the fixture (`eigenius load <doc>`). The statistics institution's `validate_analysis_plan` AutoOnLoad gate fires automatically on every `StatisticalAnalysisPlan` commit; the verdict is admitted as a new `Verdict` resource on chain. Failed claims are rejected at commit with a structured diagnostic.

## Phase-completeness matrix

D52 lands the verifier across the seven dispatch positions in phases. The Phase 5 hardenings (§7.1 OneSidedWitnessed, §7.2 dual-verdict ESD, §7.3 MethodComparisonAnalysisPlan+PB) are landed. Remaining dispatch coverage is tracked as a completeness matrix rather than as cascading sub-phase numbers — see [D52 §9 Phase 4.9 RepeatedMeasures matrix](../../../design/d52-measurement-statistics-institution.md) for the (autocorrelation × k_between_factors) table and the GitHub issues tracking the unwired cells.

| Dispatch position | Status | Phase |
|---|---|---|
| SingleSampleEstimate | ✅ Wired | 1 |
| IID two-sample (Pooled / Welch) | ✅ Wired | 1.5 |
| Paired | ✅ Wired | 2 |
| Factorial (omnibus k-way ANOVA) | ✅ Wired | 2.5 |
| RCBD | ✅ Wired | 4.0 |
| SplitPlot | ✅ Wired | 4.5 |
| RepeatedMeasures (CompoundSymmetry, k=0) | ✅ Wired | 4.9 |
| RepeatedMeasures (AR1, all k) | ❌ Tracked | [#77](https://github.com/eigenius/eigenius/issues/77) |
| RepeatedMeasures (Unstructured, all k) | ❌ Tracked | [#78](https://github.com/eigenius/eigenius/issues/78) |
| RepeatedMeasures (CompoundSymmetry, k≥1, factorial-RM) | ❌ Tracked | [#79](https://github.com/eigenius/eigenius/issues/79) |
| OneSidedWitnessed + ImpossibilityWitness | ✅ Wired | 5 (§7.1) |
| Dual-verdict ESD on SingleSampleEstimate | ✅ Wired | 5 (§7.2) |
| Dual-verdict ESD on grouped dispatches | ❌ Tracked | [#80](https://github.com/eigenius/eigenius/issues/80) |
| MethodComparisonAnalysisPlan + Passing-Bablok | ✅ Wired | 5 (§7.3) |
| PassingBablokResidual exclusion on MethodComparison | ❌ Tracked | [#81](https://github.com/eigenius/eigenius/issues/81) |
| Materialized dual-verdict commit shape (two DerivedResources via `stats:dual_verdict_pair`) | ❌ Tracked | [#82](https://github.com/eigenius/eigenius/issues/82) |

Wired cells run on the [`crates/eigenius-statistics/`](../../../../crates/eigenius-statistics/) implementation; unwired cells reject up front with a structured diagnostic naming the unimplemented combination and the GitHub issue tracking it.

## Composition with the reasoning institution

The statistics institution's emitted verdict — specifically the claim resource itself, since `StatisticalAnalysisPlan IS the chain-resident DerivedResource` — becomes a citable evidence node for D39 reasoning sentences. The composition pattern:

```text
raw IC50 readings (ObservedResource + ObservationTrace)
  → D52 validate_analysis_plan AutoOnLoad fires
  → Verdict::Holds; claim_eig0291_lowic50 is committed as DerivedResource
  → ProgramTrace pairs → witness index admits IsDerivedAs(claim_iri, HasLowIC50(...))
  → D39 ReasoningSentence cites claim_iri via DerivedEvidence
  → D39 validate_justification AutoOnLoad fires
  → certificate's JustifiedBy.derived consumes the IsDerivedAs witness
  → Verdict::Holds for the reasoning conclusion (e.g., StrongInhibitor(EIG_0291))
```

The two institutions don't call each other — they share the chain artifact shape (`DerivedResource` + `ProgramTrace` + `canonical_proposition`) that the witness index reads from. D52 emits the artifact; D39 reads the witness; the composition works because both honour the shared chain shape independently.

Full walkthrough: [composition guide §7 stats+reasoning](../../composition/07-stats-and-reasoning-walkthrough.md).

## Troubleshooting

- **`Verdict::Fails` with `AlphaNotCrossed`** — the computed p-value didn't cross `alpha`. The diagnostic names the actual p; check (a) whether the SampleSet has enough replicates to power the test, (b) whether the variance assumption matches the data shape (try `WelchUnequal` for heteroscedastic-looking samples), (c) whether the effect size you asserted is realistic.
- **`Verdict::Fails` with `EpistemicScopeViolation`** — your SampleSet's replication is `TechnicalWithinRun` but the claim's `canonical_proposition`'s head predicate isn't marked `is_a stats:MeasurementLevel`. Either gather biological replicates and recommit the SampleSet, or assert against a measurement-scope predicate (`HasLowIC50_OnThisBatch` rather than `HasLowIC50`).
- **`Verdict::Fails` with `MissingImpossibilityWitness`** — you used `OneSidedWitnessed(witness_iri)` but the IRI doesn't resolve to a chain resource, or it resolves to a resource without `is_a stats:ImpossibilityWitness`. Either commit the witness resource with the marker, or use `TwoSided()`.
- **`Verdict::Fails` with `WrongTestForDesign`** — the bundle's product position has no dispatch arm. Either the SampleSet smart constructor produces a position the verifier doesn't yet support (check the [phase-completeness matrix](#phase-completeness-matrix)), or the macro is being misused (e.g., a `Bundle(...)` literal with the wrong axis ctors). The diagnostic prints the actual position tuple.
- **`Verdict::Fails` with `MalformedSampleSet`** — the SampleSet's `sample_set_value` couldn't be decoded as a `Bundle(...)`. Usually means a smart constructor was used incorrectly (wrong number of args, wrong axis ctor names). Compare against the smart-constructor signatures in [`ontologies/statistics/statistics.esl`](../../../../ontologies/statistics/statistics.esl).
- **`Verdict::Fails` with `OutlierExclusion not yet wired for {dispatch}`** — you asserted a non-`Identity` exclusion functor on a dispatch position that doesn't yet support it. Either use `Identity()` for now, or follow the GitHub issue link in the diagnostic to track the extension.
- **Claim accepted but downstream D39 sentence fails with `NoAdmittedChainWitness`** — the `StatisticalAnalysisPlan` commit succeeded but the witness index doesn't have the expected `IsDerivedAs` entry. Check that the claim's `ProgramTrace` companion was committed in the same layer (D49 requires both the resource and the trace for witness admission).

## Cross-references

- [**D52 design spec**](../../../design/d52-measurement-statistics-institution.md) — full design rationale, the universal-claim schema's intersection-of-standards table, the five-axis design space, the opinionated-stances appendix, and the §9 phase plan with the per-phase completeness matrix.
- [**Reasoning institution tutorial**](../reasoning-institution/README.md) — the D39 institution that consumes D52 verdicts as `DerivedEvidence` groundings.
- [**ESL §4.5a Multi-class data declarations**](../../esl/04-declarations.md#4-5a-multi-class-data-declarations-marker-classes-d52-12) — the `data : Prop, stats:PopulationLevel` syntax used for §7.4 scope markers.
- [**ESL §4.9 macro declarations**](../../esl/04-declarations.md#4-9-macro-compile-time-smart-constructors-d52-12) — the compile-time AST substitution mechanism the seven smart constructors use.
- [**ESL §5.14a type_expr(...)**](../../esl/05-expressions.md#5-14a-type_expr-eigentt-type-expressions) — the chain-mirrored EigenTT type fragment used for the proposition slots.
- [**ESL §6.4a Witness predicates**](../../esl/06-resources-types-and-the-layer.md#6-4a-witness-predicates-admitting-propositions-from-layer-state) — the D49 witness machinery that propagates statistics verdicts into reasoning groundings.
- [**Composition guide §1.3a**](../../composition/01-introduction.md) — where the statistics + reasoning composition shape sits relative to the numerical-institution comorphism shape.
- [**Composition guide §7**](../../composition/07-stats-and-reasoning-walkthrough.md) — full stats+reasoning walkthrough.
- [`crates/eigenius-statistics/`](../../../../crates/eigenius-statistics/) — implementation crate.
- [`ontologies/statistics/statistics.esl`](../../../../ontologies/statistics/statistics.esl) — ontology source: universal-claim schema, sample-set sum types, seven smart constructors, opinionated-stance marker classes, verdict resource shape.
- [`crates/eigenius-statistics/tests/fixtures/`](../../../../crates/eigenius-statistics/tests/fixtures/) — the per-dispatch fixtures the integration tests run against; useful as worked examples for each smart constructor.
