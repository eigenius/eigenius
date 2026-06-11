# D52 — Measurement Statistics Institution

*Status: design memo · June 2026*

*Companion documents: [D14 institutions framework](d14-institutions.md), [D39 justification logic (v2 draft)](d39-justification-logic.md), [D46 Prop universe + axiom framework](d46-prop-universe-and-proof-irrelevance.md), [D47 chain-mirrored EigenTT type fragment](d47-chain-mirrored-eigentt-type-fragment.md), [D49 ChainWitness machinery](d49-chainwitness-machinery.md).*

*This memo specifies a chain-resident statistics institution that recomputes asserted statistical claims from raw replicate data. The premise is structural: traditional databases store post-summarised statistics and ask consumers to trust the summary; Eigenius commits raw replicates and recomputes every claim at commit time, shifting the epistemic burden from human trust to kernel verification. Getting the SampleSet shape right matters more than getting it done quickly — its constructors will be authored against for as long as the platform handles biomedical data, and a wrong shape will be paid for at every downstream consumer.*

---

## 1. Scope

In scope:

- The universal `StatisticalAnalysisPlan` schema — the parameters every statistical claim must declare, derived from the intersection of CONSORT / ARRIVE / MIQE / MIAME / MIAPE / STROBE / SAMPL / CLSI-EP requirements.
- The `SampleSet` sum type for Tier 1 (IID, Paired, Factorial) and Tier 2 (RCBD, Split-Plot, Repeated-Measures) experimental designs.
- The institution's decidable-recomputation contract: what the kernel runs at commit time, what verdict it returns, what artifacts it emits.
- Opinionated stances on three field-wide conflicts: hypothesis-test directionality, outlier handling, regression for method comparison.
- Interaction with D39 reasoning: how the institution's `DerivedResource` output feeds the `JustifiedBy.app` / `JustifiedBy.spec_str` composition pipeline.

Out of scope:

- Tier 3 designs (response-surface, crossover, sequential / group-sequential). Explicit deferral list in §10.
- Multiple-testing correction (Bonferroni, Benjamini-Hochberg FDR). These operate over *sets* of claims and need a separate higher-level institution that consumes the per-claim institution's output (§11).
- The numerics-library implementations of the underlying tests. The contract is what the verifier returns; *how* it computes a t-statistic is kernel-internal.
- Assay-quality validation upstream of the statistics layer (PCR efficiency, microscopy image quality, mass-spec drift). Owned by domain-specific observation institutions that emit the `SampleSet` itself (§11).
- Power / sample-size justification at *design* time. That is a different dispatch shape (consulted before a SampleSet exists) and belongs in an adjacent institution (§11).

## 2. Where this institution sits

A D14 `Decidable QueryClass` institution. Its dispatch shape:

- **Consumes**: a `StatisticalAnalysisPlan` resource (committed by the chain author) whose payload references a `SampleSet` ObservedResource holding raw replicate values, plus the asserted claim parameters (null/alternative hypothesis, alpha, effect-size threshold, directionality, etc.).
- **Recomputes**: the statistic from the SampleSet using the test prescribed by the SampleSet's design topology and the claim's variance assumption.
- **Returns a Verdict**: `Holds` or `Fails(diagnostic)`. On `Holds`, the kernel emits a `DerivedResource` whose `canonical_proposition` is the threshold predicate the claim establishes (e.g. `screen:HasLowIC50("urn:...:EIG_0291")`), together with a `ProgramTrace` admitting the `IsDerivedAs` witness (D49 §6).
- **Composability**: the emitted DerivedResource is consumed downstream by D39 reasoning via `DerivedEvidence` in a `JustifiedBy.app` / `JustifiedBy.spec_str` composition. The statistics institution does not know — and does not need to know — what reasoning conclusion the chain author will derive from its output.

This is the **decidability boundary**: every statistical claim must be recomputable from raw data alone in finite time using deterministic numerical procedures. Subjective judgement (whether IC50 < 100 nM is biologically meaningful) lives in declared literature rules at the D39 layer. The statistics institution settles only the arithmetic question.

## 3. The universal Claim schema (intersection of standards)

Every `StatisticalAnalysisPlan` resource — regardless of which SampleSet topology it consumes — must declare the following fields. These are the *intersection* of what CONSORT, ARRIVE, MIQE, MIAME, MIAPE, STROBE, SAMPL, and the CLSI EP-series all require; without them, the claim is not recomputable.

```esl
namespace stats      = "urn:eigenius:measurements";
namespace reflection = "urn:eigenius:reflection";

resource <iri> : stats:StatisticalAnalysisPlan {
    stats:sample_set            = <ResourceRef -> stats:SampleSetResource>;
    stats:null_hypothesis       = type_expr(...);   // Prop
    stats:alternative_hypothesis = type_expr(...);  // Prop
    reflection:canonical_proposition = type_expr(...);  // the predicate the claim establishes (= what downstream D39 cites via DerivedEvidence)
    stats:alpha                 = <core:float>;     // nominal Type I error, unadjusted
    stats:effect_size           = <stats:EffectSize>;
    stats:directionality        = <stats:Directionality>;
    stats:variance_assumption   = <stats:VarianceAssumption>;
    stats:outlier_exclusion     = <stats:OutlierExclusion>;  // defaults to Identity
    stats:autocorrelation_structure = <stats:AutocorrelationStructure>;  // required when sample_set.repeated_measures = Longitudinal(_); else ignored
}
```

**Note on the proposition slot.** Earlier D52 drafts named this field `stats:derived_proposition`; Phase 1 implementation review settled on the inherited `reflection:canonical_proposition` slot instead, because that's the slot D49 §6's witness emitter reads from when admitting `IsDerivedAs(claim_iri, proposition)`. Keeping a parallel `stats:derived_proposition` would either duplicate the data (two names, one source of truth) or require institution-specific witness-index logic (breaks the cross-class uniformity D49 §6 was designed for). The slot's class-neutral name ("canonical_proposition" — "this is *the* proposition this resource carries, regardless of which epistemic-category subclass") is exactly what makes it correctly shared across `DeclaredResource` / `ObservedResource` / `DerivedResource` / `VerifiedResource` — renaming to a derivation-flavored name would mislead three of the four.

Autocorrelation-structure sum type (required for longitudinal claims; author-asserted so the verifier is fully deterministic):

```esl
data stats:AutocorrelationStructure {
    AR1,                // first-order autoregressive
    CompoundSymmetry,   // exchangeable correlation across timepoints within unit
    Unstructured,       // freely estimated full covariance matrix
}
```

Effect-size sum type (the literature has not converged; the schema carries the author's choice). Standardized effect sizes carry their *inputs* rather than just the derived value, so the verifier can check both the input recovery against the SampleSet and the derivation arithmetic — stronger audit trail than recording only `d`:

```esl
data stats:EffectSize {
    Absolute(magnitude : core:float, units : core:string),
    Relative(fold_change : core:float),                            // ratio
    StandardizedCohensD(mean_diff : core:float, pooled_sd : core:float),
    StandardizedHedgesG(mean_diff : core:float, pooled_sd : core:float, n_total : core:integer),
}
```

Variance-assumption sum type (drives which test the verifier dispatches):

```esl
data stats:VarianceAssumption {
    Pooled,            // classical equal-variance t-test
    WelchUnequal,      // Welch–Satterthwaite for unequal variances
    NonParametric,     // distribution-free (Mann–Whitney / Wilcoxon)
    RankBased,         // rank-transformed parametric
}
```

What is deliberately *not* in the universal schema, and why:

- **Power (1 − β)**. Meaningful only at design time, not at verification time. The kernel cannot recompute power from completed data without re-asserting the design's prior assumptions, and re-asserting prior assumptions defeats the institution's purpose. Carried as a field on the `SampleSet` resource (where it pairs with the sample-size justification), not on the claim.
- **Drop-out / attrition rates**. Properties of the cohort, not the claim. Live on `SampleSet`.
- **Variance values themselves** (SD, SE, CV). Derived from raw replicates by recomputation; the author asserts the *hypothesis* and the *threshold*, never the variance.
- **Confidence-interval endpoints**. Computed, not asserted; the verifier reports them as part of the `Holds` outcome.
- **Replication kind** (biological vs technical-within-run vs nested). Carried on `SampleSet` via the `replication` axis (§4.2), not on the claim. The distinction is structural to the data (the same three numbers mean different things depending on whether they're independent biological samples or repeated reads of one prep) and the verifier consumes it to select the right variance-component stratification per CLSI EP05-A3. The §7.4 stance further constrains which claim shapes are admissible given the SampleSet's `replication` value.

This is the rigour transposition the institution buys: the author asserts the hypothesis, the threshold, and the direction; the kernel computes everything that depends on the raw data.

## 4. SampleSet shape — the open design choice

Two structurally distinct shapes for the `SampleSet` type are viable. Both are documented here so future scope discussions can refer to a fixed comparison rather than re-deriving the trade-off. The recommendation in §4.3 settles on Option B (product over orthogonal axes, with smart-constructor wrappers for the named designs); §4.4 records the conditions under which Option A becomes preferable instead.

### 4.1 Option A — Flat sum type, one constructor per named design

A top-level sum type whose constructors are named after the literature's named designs (`IID`, `Paired`, `Factorial`, `RCBD`, `SplitPlot`, `RepeatedMeasures`, plus `SingleSampleEstimate` for one-sample threshold cases). Each constructor's docstring records its position in the five-axis experimental-design space (randomization × blocking × factor × replication × repeated-measures) as a *documented* invariant.

```esl
data stats:SampleSet {
    SingleSampleEstimate(
        measurements     : core:Array<stats:Replicate>,
        replication_kind : stats:ReplicationKind,
    ),
    IID(
        replicates       : core:Array<stats:Replicate>,
        replication_kind : stats:ReplicationKind,
    ),
    Paired(
        pairs            : core:Array<stats:PairedObservation>,
        replication_kind : stats:ReplicationKind,
    ),
    Factorial(
        factors          : core:Array<stats:Factor>,
        observations     : core:Array<stats:FactorialObservation>,
        replication_kind : stats:ReplicationKind,
    ),
    RCBD(...),
    SplitPlot(...),
    RepeatedMeasures(
        unit_axis        : stats:Factor,
        time_axis        : stats:TimeAxis,
        factors          : core:Array<stats:Factor>,    // 0..k within-subjects factors
        observations     : core:Array<stats:LongitudinalObservation>,
        replication_kind : stats:ReplicationKind,
    ),
}
```

Trade-offs:

- **(+)** Verifier dispatch is direct: `match sample_set with IID(...) => one_way(...) | Paired(...) => paired_test(...) | ...`.
- **(+)** Surface cardinality matches authoring vocabulary. A biologist reaches for "paired"; the constructor of that name is exactly where they expect it.
- **(+)** The observation-element type is type-distinguished per constructor (`Array<PairedObservation>` vs `Array<Replicate>`), so mismatched shapes are caught at type-check rather than at verification.
- **(−)** The axis decomposition lives in docstrings, not in the type. A constructor that mis-labels its coordinates is a doc bug, not a type error.
- **(−)** Common axis variations leak into individual constructor bodies as varying-length parameters: `factors` on `RepeatedMeasures` (to accommodate factorial-repeated-measures); `replication_kind` on every constructor; potentially `time_axis` on `RCBD`/`SplitPlot` (per §12) for longitudinal variants. Three of the five axes end up parameterized inside constructors, half-recreating Option B with more typing.
- **(−)** Two constructors whose axis coordinates partially overlap (e.g. `Paired` is structurally `RCBD` with block size two; `SingleSampleEstimate` is `IID` with zero-factor design and one group) cannot share verifier dispatch without manual delegation.

### 4.2 Option B — Product over the five orthogonal axes, with smart-constructor wrappers

A single primary constructor `Bundle` whose record-shaped fields name the 5-axis coordinate plus the cross-cutting properties (biological units, assay columns, sampleMap, observations). The named designs from the literature are recovered as **smart-constructor functions** (ESL `macro` declarations, per D52 §12 #1) that desugar to `Bundle` at the right product position.

**Note on the ctor name.** Earlier drafts named the product constructor `Set`, but ESL reserves `Set` as a sort-keyword lexer token, so the data declaration `data stats:SampleSet { Set(...) }` fails to parse. Phase 1 implementation settled on `Bundle` ("bundle of axes + topology + observations") which captures the product nature without colliding with the sort keyword. The constructor name appears only in the data-declaration body and inside smart-constructor macro bodies; the surface authors interact with — `stats:SingleSampleEstimate(...)`, `stats:Paired(...)`, etc. — is unaffected.

The axis enums:

```esl
data stats:Randomization { CompleteRandom, Restricted }

data stats:Blocking {
    Unblocked,
    PairedBlocking,                                    // block_size = 2 (paired/matched designs)
    RCB(block_size : core:integer),                    // block_size ≥ 3
    Incomplete(block_size : core:integer),
}

data stats:FactorDesign {
    NoFactor,                                          // single-sample estimation
    SingleFactor,
    FullFactorial(k : core:integer),
    FractionalFactorial(k : core:integer, p : core:integer, generators : core:Array<core:string>),
}

data stats:Replication {
    BiologicalReplication,
    TechnicalWithinRun,
    NestedReplication(biological_n : core:integer, technical_per_biological : core:integer),
}

data stats:RepeatedMeasuresAxis {
    CrossSectional,
    Longitudinal(n_timepoints : core:integer),
}
```

The product:

```esl
data stats:SampleSet {
    Bundle(
        stats:Randomization,        // randomization
        stats:Blocking,             // blocking
        stats:FactorDesign,         // factor
        stats:Replication,          // replication
        stats:RepeatedMeasuresAxis, // repeated_measures
        core:string,                // units (serialized; refined in Phase 3 when sampleMap topology lands)
        core:string,                // columns (serialized)
        core:string,                // sample_map (serialized)
        core:value_array,           // observations (inline floats v1; promoted to Replicate-IRI array Phase 1.5+)
    ),
}
```

Smart constructors (ESL `macro` declarations producing `Bundle` values at canonical product positions — what the chain author actually writes; per D52 §12 #1 implementation these are compile-time AST substitution, not runtime closures):

```esl
macro stats:SingleSampleEstimate(
    measurements : core:value_array,
    replication  : stats:Replication,
) : stats:SampleSet =>
    Bundle(
        CompleteRandom(),
        Unblocked(),
        NoFactor(),
        replication,
        CrossSectional(),
        "_implicit_single_unit",
        "_implicit_single_column",
        "_implicit",
        measurements,
    );

macro stats:Paired(
    pairs       : core:value_array,
    replication : stats:Replication,
) : stats:SampleSet =>
    Bundle(
        CompleteRandom(),
        PairedBlocking(),
        SingleFactor(),
        replication,
        CrossSectional(),
        /* ... derived from pairs ... */
    );

// ...IID, Factorial, RCBD, SplitPlot, RepeatedMeasures similarly...
```

Trade-offs:

- **(+)** Axis decomposition is type-level. Misclassification is a type error, not a doc bug.
- **(+)** Adding the `replication_kind`-on-every-constructor distinction, the factorial-repeated-measures hybrid, and the single-sample case is *free* — they're already product positions.
- **(+)** Tier 3 designs and future hybrids (RCBD-longitudinal, SplitPlot-longitudinal) slot in by extending an axis enum + adding a smart constructor + adding a verifier arm. No new top-level data type.
- **(+)** Smart constructors recover the literature's vocabulary at the authoring surface — the chain author writes `stats:Paired(pairs, BiologicalReplication())`, not the full nine-field `Bundle(...)`.
- **(+)** Cross-cutting properties (sampleMap, biological units, assay columns) live in one place rather than repeating across constructors.
- **(−, mitigated by indexed-inductive upgrade — deferred from Phase 1 to follow-on)** The naive product encoding gives observations a uniform `core:value_array` (or, post-promotion, `core:resource_array` of `Replicate` IRIs), turning shape mismatches into runtime `WrongTestForDesign` rejections rather than type errors. The principled fix — make `Bundle` an *indexed* inductive whose observation element-type is computed from the product position via a `stats:ObservationFor(rand, blk, fac, rep, rep_meas)` function — was the §12 #2 decision target for Phase 1. **Implementation review deferred it** to a post-Phase-1 follow-on: the basic vertical landed faster with runtime rejection, and the indexed-inductive upgrade benefits from a real chain pulling on the shape before being committed to. D48 machinery covers the kernel side; the ~30–50 line ESL cost (`ObservationFor` + indexed `Bundle` decl) is still the budget when the upgrade lands.
- **(−)** The product type admits nonsense combinations (e.g. `(CompleteRandom, PairedBlocking, FullFactorial, …, Longitudinal)` — what test recomputes that?). The verifier rejects these explicitly with a `WrongTestForDesign` diagnostic; the cost is one wildcard arm rather than one explicit arm per supported position.
- **(−)** Smart constructors require ESL function support that produces inductive-ctor values. If that pattern isn't ergonomically supported today, this lands as a small parallel lift (§9 Phase 1).

### 4.3 Recommendation — Option B for v1

Reasons:

1. **The Option-A leakage already happened, pre-implementation.** Four axis parameterizations were identified during D52 review: the `factors` field on `RepeatedMeasures` (factorial-repeated-measures), the `replication_kind` field on every constructor (technical vs biological replicates per §3), the `SingleSampleEstimate` case (zero-factor designs that don't fit `IID`), and prospective `time_axis` fields on `RCBD`/`SplitPlot` (per §12). Between them, three of the five axes (Factor, Replication, RepeatedMeasures) end up exposed as parameterized fields *inside* Option A's constructors. At that point the type-level discipline of Option B is paying for itself in coherence, not just future-proofing.
2. **Authoring vocabulary is preserved via smart constructors.** The "named designs match literature" argument that originally favoured Option A is recoverable in Option B at the authoring surface: `stats:Paired(pairs, BiologicalReplication)` reads the same as Option A's `Paired(pairs)`, but desugars to a typed product position the verifier can dispatch on uniformly.
3. **Verifier dispatch is the same complexity.** Six or seven supported product positions, same six numerics routines, expressed as a single nested `match` over the product. Nonsense combinations get a wildcard arm returning `WrongTestForDesign` — one arm to write, not one-per-rejected-combination.
4. **Future hybrids are free additions, not new constructors.** `RCBD`-with-longitudinal-measurements becomes a new smart constructor over an existing product position (`Restricted × RCB(k) × SingleFactor × * × Longitudinal(n)`), not a new top-level `data` ctor that requires updating every helper / verifier table.
5. **The platform is pre-production.** No deployed chains, no migration cost. The right shape now is the shape that costs less when the institution starts being used in anger; that's Option B with smart constructors.

The remainder of this document specifies Option B.

### 4.4 When Option A would be the better choice

The conditions that would flip the recommendation back to Option A, recorded so future scope discussions can refer to a fixed list:

- **ESL doesn't support smart-constructor functions cleanly.** If a function defined in an ESL file cannot produce an inductive-ctor value with the right ergonomics — e.g. the function call site looks materially worse than an Option A direct constructor call — then the authoring-surface argument for Option B collapses and Option A wins on vocabulary alone. This is the load-bearing implementation question to settle in Phase 1.
- **The set of named designs stabilises at a small number that doesn't grow.** If after Phases 1–4 we find that the seven Tier 1+2 designs really are the entire authoring surface for the foreseeable future and no hybrids are showing up, the additional structure of Option B becomes pure overhead and Option A's directness wins.
- **Observation-type discipline turns out to be load-bearing in v1.** If the indexed-inductive `ObservationFor` upgrade (§12) proves infeasible *and* runtime rejection of shape-mismatched observations turns out to be a frequent friction point for chain authors, the per-constructor observation-element typing Option A provides natively becomes more valuable than the axis-decomposition discipline.

Until at least one of these conditions is met, Option B with smart constructors is the right shape.

## 5. SampleSet smart constructors (Tier 1 + Tier 2)

The chain author writes against the smart-constructor functions, not against `stats:SampleSet.Set` directly. Each smart constructor lands at a specific product position and the verifier dispatches off that position; the named designs preserve the literature's vocabulary while the structural discipline lives in the product type.

Each subsection records the product position the smart constructor produces, the shape of its observation payload, the verifier procedure it triggers, and any common-use notes.

### 5.1 Tier 1

#### `SingleSampleEstimate(measurements, replication)`

- **Product position**: `(CompleteRandom, Unblocked, NoFactor, replication, CrossSectional)`.
- **Observation shape**: each entry is a `stats:Replicate` carrying its value plus its biological-unit id.
- **Verifier procedure**: one-sample t-test against the claim's threshold (`Pooled`/`WelchUnequal`), Wilcoxon signed-rank against the threshold (`NonParametric`/`RankBased`), or threshold-CI-overlap test for `EffectSize.Absolute` claims. The claim's `derived_proposition` is verified to follow from the one-sample inference.
- **Epistemic-scope guardrail**: when `replication = TechnicalWithinRun`, the verifier rejects any `derived_proposition` whose interpretation is population-level (see §7.4); the claim is admissible only as a per-batch/per-plate statement.
- **Common use**: the IC50-threshold case from `drug_screening.esl` — three replicate IC50 readings against a "< 100 nM" threshold; CDx-style cut-off determinations; single-sample assay validation.

#### `IID(replicates, replication)`

- **Product position**: `(CompleteRandom, Unblocked, SingleFactor, replication, CrossSectional)`.
- **Observation shape**: each entry is a `stats:Replicate` carrying its value, biological-unit id, and the treatment level it received.
- **Verifier procedure**: dispatched on `variance_assumption` — Student's two-sample t-test (`Pooled`), Welch's t-test (`WelchUnequal`), Mann–Whitney U (`NonParametric`), rank-transformed t-test (`RankBased`). For more than two groups: one-way ANOVA / Kruskal–Wallis.
- **Common use**: the simplest comparative case — single condition vs control, completely randomized assignment.

#### `Paired(pairs, replication)`

- **Product position**: `(CompleteRandom, PairedBlocking, SingleFactor, replication, CrossSectional)`.
- **Observation shape**: each entry is a `stats:PairedObservation` carrying `{ unit_id, before, after }` (or analogous matched-case/control structure).
- **Verifier procedure**: paired t-test (`Pooled`/`WelchUnequal`) or Wilcoxon signed-rank (`NonParametric`/`RankBased`).
- **Why a distinct smart constructor despite collapsing to `(*, PairedBlocking, SingleFactor, *, CrossSectional)`**: treating paired data as IID is the most common false-positive-inducing error in the biomedical literature. The smart constructor takes a `core:Array<PairedObservation>` parameter (not `core:Array<Replicate>`), so authoring the wrong shape fails at the call site rather than at verification. The `PairedBlocking` ctor on the `Blocking` axis is what makes this distinct from `RCBD` (whose `RCB(k)` requires `k ≥ 3`) at the type level.

#### `Factorial(factors, observations, replication)`

- **Product position**: `(CompleteRandom, Unblocked, FullFactorial(k), replication, CrossSectional)` where `k = factors.len()`.
- **Observation shape**: each entry is a `stats:FactorialObservation` tagged with the `k`-tuple of factor levels.
- **Verifier procedure**: k-way ANOVA with all main effects and interaction terms.
- **Common use**: synergy / antagonism testing in 2×2 or 3×3 layouts — drug × concentration, genotype × treatment, etc.
- **Scope**: cross-sectional only. The factorial-with-longitudinal-measurements case (same biological units measured at multiple timepoints under all factor-level combinations) is handled by `RepeatedMeasures` with `factors.len() ≥ 2`; see §5.2.3.

### 5.2 Tier 2

#### `RCBD(block_factor, treatment, observations, replication)`

- **Product position**: `(Restricted, RCB(block_factor.size), SingleFactor, replication, CrossSectional)` where `block_factor.size ≥ 3`.
- **Observation shape**: each entry is a `stats:BlockedObservation` tagged with its block id and treatment level. The smart constructor enforces that every block contains every treatment level before producing the `Bundle`.
- **Verifier procedure**: linear mixed-effects model with `block_factor` as a random effect and `treatment` as a fixed effect. Reduces to two-way ANOVA when blocks are treated as fixed.
- **Common use**: controlling for known nuisance variation (plate position, day-of-experiment, animal cage) without confounding it with treatment.

#### `SplitPlot(whole_plot_factor, subplot_factor, observations, replication)`

- **Product position**: `(Restricted, RCB(whole-plot-nested), FullFactorial(2), replication, CrossSectional)`. The `RCB(whole-plot-nested)` form expresses the constraint that subplots are nested within whole plots; this is a Tier-2-specific shape that the smart constructor produces (the `Blocking` axis enum admits it implicitly via the `block_size` field's contextual interpretation).
- **Observation shape**: each entry is a `stats:SplitPlotObservation` carrying the whole-plot factor level, the subplot factor level, and the whole-plot id.
- **Verifier procedure**: mixed-effects model with whole-plot error stratum and subplot error stratum, fitted *separately* and not pooled.
- **Why high-priority despite Tier 2**: split-plot designs are *frequently misidentified* as factorial. When that mistake gets verified with a flat factorial ANOVA, the whole-plot error term is drastically under-estimated and the institution silently admits false-positive whole-plot claims. Catching exactly this class of bug is one of the institution's primary justifications.

#### `RepeatedMeasures(unit_axis, time_axis, factors, observations, replication)`

- **Product position**: `(CompleteRandom or Restricted depending on the within-unit randomization, blocking-as-implied-by-the-design, FactorDesign-determined-by-factors.len(), replication, Longitudinal(time_axis.size))`.
- **Observation shape**: each entry is a `stats:LongitudinalObservation` carrying `{ unit_id, timepoint, factor_levels, value, covariates }`.
- **Verifier procedure**: longitudinal mixed-effects model with unit as a random effect, the entries of `factors` as fixed effects (with interaction terms when `factors.len() ≥ 2`), and an autocorrelation structure on time within unit (AR(1) / compound symmetry / unstructured — see §12 open question).
- **Three common shapes by `factors.len()`**:
  - `factors = []` — pure time-series on a panel of units (no treatment; observational longitudinal cohort).
  - `factors = [f]` — single-treatment longitudinal study; e.g. drug-vs-placebo dose response over time.
  - `factors = [f1, …, fk]` — **factorial repeated-measures**; e.g. drug × concentration tested on the same biological units across multiple timepoints. One of the most common designs in pharmacology and longitudinal clinical work.
- **Why one smart constructor across `factors.len() = 0 / 1 / k`**: these cases share the same verifier infrastructure (longitudinal mixed-effects + autocorrelation), the same product-position template, and the same authoring vocabulary in the literature ("repeated measures"); collapsing them into one smart constructor with a varying-length `factors` parameter is more honest about the structural similarity than splitting into named hybrids would be.

### 5.3 The sampleMap (cross-cutting)

Every product position whose observations involve a non-rectangular biological-unit × assay-column topology populates the `Bundle`'s `sample_map` field via the smart constructor. The map shape — modelled on Bioconductor's MultiAssayExperiment `sampleMap` slot:

```esl
data stats:SampleMap {
    Entries(rows : core:Array<stats:SampleMapEntry>),
}

data stats:SampleMapEntry {
    Entry(
        assay     : core:string,    // the assay-id within the SampleSet
        primary   : core:string,    // the biological-unit IRI
        col_name  : core:string,    // the specific column-id within the assay
    ),
}
```

This is the load-bearing structural choice from Bioconductor's two-decade evolution: the kernel must not require rectangularity across the assay layers of a multi-modal SampleSet. The bipartite map structurally encodes pseudo-replicates (many columns per unit), cross-modal missingness (a unit with no column in some assay), and many-to-one bench replicates. The recomputation step consumes the map to identify nested variance components correctly and to select the right error stratification.

For `SingleSampleEstimate`, `IID`, and `Paired`, the smart constructor synthesizes an implicit sampleMap (one column per unit). For `Factorial`, `RCBD`, `SplitPlot`, and `RepeatedMeasures`, the chain author passes an explicit sampleMap argument to the smart constructor — without it, the verifier cannot tell pseudo-replicates from true replicates and variance components come out wrong.

### 5.4 Verifier dispatch table

The kernel-side verifier consumes a `Bundle(...)` and dispatches on the product position. The supported cells:

| Product position (Randomization, Blocking, Factor, RepeatedMeasures) | Procedure |
|---|---|
| `(CompleteRandom, Unblocked, NoFactor, CrossSectional)` | one-sample test against threshold |
| `(CompleteRandom, Unblocked, SingleFactor, CrossSectional)` | two-sample / one-way ANOVA |
| `(CompleteRandom, PairedBlocking, SingleFactor, CrossSectional)` | paired test |
| `(CompleteRandom, Unblocked, FullFactorial(k), CrossSectional)` | k-way ANOVA |
| `(Restricted, RCB(_), SingleFactor, CrossSectional)` | RCBD mixed-effects |
| `(Restricted, RCB(_), FullFactorial(_), CrossSectional)` | split-plot mixed-effects |
| `(*, *, *, Longitudinal(_))` | longitudinal mixed-effects (`factors.len()` selects fixed-effect structure) |
| anything else | `Verdict::Fails(WrongTestForDesign { ... })` |

The `Replication` axis is read inside every arm to select the variance-component stratification (CLSI EP05's repeatability vs intermediate precision) and to drive the §7.4 epistemic-scope check; it doesn't appear in the dispatch table because every supported cell consults it the same way.

## 6. The decidable-recomputation contract

The institution declares itself a D14 `Decidable QueryClass`. At commit time, the kernel invokes its dispatch handler with:

- The `StatisticalAnalysisPlan` resource (with the universal-schema fields from §3).
- The resolved `SampleSet` resource (raw replicates + design topology + sampleMap + biological-unit metadata).
- The current `ExecutionContext` (read-only access to the chain for resolving cross-references like the `derived_proposition`'s constituent predicates).

The handler returns a `Verdict` ∈ `{ Holds, Fails(diagnostic) }`. On `Holds`, the kernel emits:

1. A `DerivedResource` whose IRI is content-addressed from `(claim_iri, sample_set_iri, derived_proposition_hash)` and whose `canonical_proposition` is the claim's `derived_proposition`.
2. A `ProgramTrace` resource (per D49 §6) pointing at the `DerivedResource`, so the witness index admits an `IsDerivedAs` entry.

The recomputation must satisfy three properties:

- **Deterministic**: identical inputs produce bit-identical Verdicts and bit-identical numerics. The numerics library used must commit to IEEE-754 semantics; no fast-math, no non-deterministic parallel reductions, no time- or RNG-dependent algorithms (use the kernel's seeded RNG if a procedure needs randomness, e.g. for bootstrap CIs).
- **Bounded**: every Tier 1+2 procedure terminates in time polynomial in N. No iterative procedure can spin without a hard step bound.
- **Reproducible from chain**: the handler reads only from the SampleSet and the claim resource. No external data sources, no network access, no system time.

The `Verdict::Fails` diagnostic is structured (not a free-form string). It names which clause of the claim failed and reports the computed numerics:

- `AlphaNotCrossed { computed_p : core:float, threshold : core:float }`
- `EffectSizeBelowThreshold { computed : stats:EffectSize, asserted : stats:EffectSize }`
- `WrongTestForDesign { sample_set_topology : Iri, claim_assumes_topology : Iri }` (catches the split-plot-as-factorial misidentification)
- `InsufficientReplication { n : core:integer, minimum_required : core:integer }`

This lets the chain author correct the claim and re-commit without guessing at the cause.

## 7. Opinionated stances

The prior-art survey identified three field-wide conflicts where competing standards disagree. The institution adopts an opinionated default rather than mirroring the disagreement, because mirroring would let the wrong choice ride into the chain unchallenged.

### 7.1 Two-sided tests by default; one-sided requires an impossibility witness

`directionality` defaults to `Two_Sided`. To assert `One_Sided_Witnessed`, the claim must carry an `impossibility_witness` field — a chain-resident proof (an EigenTT type expression, validated against the chain's existing reasoning) that the inverse direction is physically or mathematically impossible within the system under study.

In practice: a clinical author cannot one-sidedly claim "drug X reduces blood pressure" — they must either accept two-sided p-values (the bar most of the literature now publishes) or carry a witness (e.g. for radioactive decay, where negative half-lives are physically meaningless). This is the ARRIVE-aligned stance; legacy software's silent one-sided defaults are rejected.

Rationale: one-sided tests are widely misused to inflate apparent significance. Allowing them by default would let the institution be weaponized for the exact false-positive class it exists to prevent.

### 7.2 Immutable raw data; outlier exclusion as a functor that produces dual verdicts

The `SampleSet` carries every replicate the bench produced. Outlier exclusion is *not* a property of the SampleSet — it is a property of the *claim*:

```esl
data stats:OutlierExclusion {
    Identity,
    PassingBablokResidual(threshold_sigma : core:float),
    ESD(max_outliers : core:integer, alpha_esd : core:float),
    Manual(excluded : core:Array<stats:ManualExclusionEntry>),
}

data stats:ManualExclusionEntry {
    Entry(
        unit_id                    : core:string,
        quality_check_resource_iri : core:string,    // must resolve to a committed assay-quality observation
    ),
}
```

When a claim carries a non-`Identity` exclusion functor, the kernel computes the claim **twice** — once with the functor applied, once with `Identity` — and commits *both* verdicts to the chain (as two `DerivedResource`s related via a `stats:dual_verdict_pair` link). Downstream consumers see both; transparency rides in the data model, not in social trust of the author.

For `Manual` exclusion specifically, the kernel additionally verifies — *before* admitting the exclusion — that each referenced `quality_check_resource_iri` resolves to a committed assay-quality observation (from one of the §11 assay-quality observation institutions) whose scope covers the excluded unit. No free-form justification: `Manual` exclusion is only available once a typed quality-check resource exists for each excluded unit. This makes `Manual` structurally inhabitable but only useful once the assay-quality institutions land; until then, dual-verdict commits via `PassingBablokResidual` or `ESD` are the sensitivity-analysis path.

Rationale: STROBE-aligned sensitivity-analysis stance, structurally hardened. The choice to exclude an outlier is a methodological decision that future readers must be able to second-guess; storing only the post-exclusion result is the same epistemic loss as storing only the summary statistic. Free-form justification strings, the obvious alternative, would let unverifiable claims back in through the back door.

### 7.3 Passing-Bablok for method comparison; OLS rejected

When the claim's hypothesis compares two measurement methods or two assay readouts — i.e. the claim resource's class is `stats:MethodComparisonAnalysisPlan` (a subclass of `stats:StatisticalAnalysisPlan`) — the kernel rejects ordinary least-squares regression. The verifier defaults to Passing-Bablok (non-parametric, robust to outliers, errors-in-both-variables). Deming regression is acceptable when the author asserts a known variance ratio between the methods.

Rationale: CLSI EP09-aligned. OLS assumes the X-axis has zero measurement error, which for biological measurements compared against each other is structurally false. Authors who insist on OLS for method comparison are asserting something the institution cannot let stand.

### 7.4 Technical-only replicates cannot support population-level propositions

The SampleSet's `replication` axis is consulted at every verifier dispatch (§5.4) for variance-component stratification. It is *also* consulted at claim-admissibility time:

- **`replication = BiologicalReplication`**: any `canonical_proposition` shape is admissible (subject to the other verifier checks).
- **`replication = TechnicalWithinRun`**: only `canonical_proposition` shapes whose interpretation is local to the *measurement event* are admissible. The institution rejects population-level propositions outright with diagnostic `EpistemicScopeViolation { sample_replication: TechnicalWithinRun, proposition_scope: PopulationLevel }`. To assert a population-level proposition from technical-only replicates, the chain author must either gather biological replicates and recommit the `SampleSet`, or commit the claim against a `…_OnThisBatch(...)` / `…_OnThisPlate(...)` measurement-scope predicate that does not generalize beyond the run.
- **`replication = NestedReplication(biological_n, technical_per_biological)`**: population-level propositions admissible; the verifier uses CLSI EP05-A3 nested ANOVA to stratify within-run vs intermediate-precision variance, and the claim's `canonical_proposition` must explicitly cite which precision tier it asserts against.

The *scope* of a proposition (population-level vs measurement-event-level) is determined from its constituent predicate's class membership: the chain ontology should mark each `HasLowIC50`-style predicate with either `stats:PopulationLevel` or `stats:MeasurementLevel` so the admissibility check is mechanical. Predicates with no scope marker default to population-level (the more restrictive admissibility — fail-safe).

**Authoring (Phase 1.5).** Scope markers ride directly on the predicate's `data` declaration via the multi-class header syntax landed in Phase 1.5:

```esl
data screen:HasLowIC50 : core:string -> Prop, stats:PopulationLevel {
}

data assay:HasLowIC50_OnThisBatch : core:string -> Prop, stats:MeasurementLevel {
}
```

The inductive-type resource's `is_a` array carries both the implicit `InductiveType` membership and the author-declared scope class(es). The §7.4 admissibility check reads the predicate's `is_a` directly — no companion-resource workaround. Predicates omitting the marker still default to `PopulationLevel` (the more restrictive admissibility — fail-safe).

Rationale: the institution exists to prevent the trust-the-summary problem. Silently admitting a population-level claim from three reads of one plate would re-introduce exactly that problem — the chain would attest "EIG_0291 has IC50 < 100 nM" when what was actually established is "this one plate's reading of EIG_0291's IC50 was < 100 nM on this one day." The two propositions have different evidential weight; conflating them is the same epistemic loss as conflating the summary statistic with the raw data. The §7.2 dual-verdict commit shape doesn't help here because there is no transformation between the two; biological replication is structural information the SampleSet either carries or doesn't.

## 8. Interaction with D39 reasoning

The statistics institution's output is the *input* to D39 reasoning. The flow is:

1. The chain author commits a `StatisticalAnalysisPlan` (with universal-schema fields + a Tier 1/2 SampleSet reference).
2. The statistics institution recomputes the claim from raw replicates. On `Holds`, it emits a `DerivedResource` (say, IRI `urn:org:lab:claim_eig0291_lowic50`) whose `canonical_proposition` is `screen:HasLowIC50("urn:...:EIG_0291")`, plus the matching `ProgramTrace`.
3. The D49 witness index admits an `IsDerivedAs` entry for that IRI under that proposition.
4. A `ReasoningSentence` then composes the derived claim with a universal literature rule via the D39 grammar:

   ```esl
   reasoning:justification = App(
       SpecStr(
           DeclaredEvidence("urn:org:lab:rule_strong_inhibitor"),
           "urn:...:EIG_0291"
       ),
       DerivedEvidence("urn:org:lab:claim_eig0291_lowic50")
   );
   ```

   The certificate's `derived(...)` JustifiedBy constructor consumes the `IsDerivedAs` witness from step 3; the `spec_str(...)` constructor specializes the rule at `"urn:...:EIG_0291"`; the `app(...)` constructor composes the specialised implication with the observation to derive `StrongInhibitor("urn:...:EIG_0291")`.

The statistics institution does not know — and explicitly does not need to know — what reasoning conclusion the chain author will derive from its output. It certifies only that the `derived_proposition` holds against the raw replicates. The reasoning layer composes from there.

**D39 requires no changes to accommodate this institution.** `DerivedEvidence` already exists; the `IsDerivedAs` witness machinery is already in place. The statistics institution is a downstream producer that fits the existing API.

## 9. Implementation phasing

**Phase 1 — Universal Claim schema + `SingleSampleEstimate`, end-to-end vertical proven. ✅ LANDED.** What actually landed in the first slice:

- **ESL `macro` extension** (per §12 #1): `Declaration::Macro(MacroDecl)`, `Value::MacroCall { name, args, pos }`, `TokenKind::Macro`, parser + compile-time AST-substitution machinery. Pure compile-time expansion; no runtime closure / NbE evaluation. Tests cover positive expansion, undeclared-macro errors, and arity mismatches. Surface keyword `macro` (distinct from `fun` which stays for type-level lambdas inside `type_expr(...)`).
- **Statistics ontology** (`ontologies/statistics/statistics.esl`): all five axis enums, the `SampleSet` product type with the `Bundle` ctor (not `Set` — see §4.2 note), `StatisticalAnalysisPlan` + `SampleSetResource` + `MeasurementVerdict` classes, all universal-Claim sum types (`EffectSize`, `Directionality`, `VarianceAssumption`, `AutocorrelationStructure`, `OutlierExclusion` with typed `ManualExclusionEntry`), the two scope-marker classes (`PopulationLevel` / `MeasurementLevel`), and the institution + QueryClass resource declarations.
- **`eigenius-statistics` Rust crate**: `StatisticsInstitution` with full `Institution` trait impl, `ndarray` + `statrs` numerics for the one-sample t-test (deterministic, R-reference-validated, bit-identical-across-runs), validate handler reading the claim → resolving the SampleSet → decoding the `Bundle` product position → dispatching → running the §7.4 epistemic-scope check → emitting the verdict resource with computed numerics attached.
- **D49 witness admission**: claims declare `reflection:canonical_proposition` (per §3 settled-on naming); companion `ProgramTrace` resources admit the `IsDerivedAs` witness. Test verifies the witness lands in the index via `lookup_chain_witness`.
- **D52 §8 D39 composition end-to-end**: confirmatory IC50 SampleSet → `StatisticalAnalysisPlan` Holds → `IsDerivedAs` admitted → `ReasoningSentence` with `App(SpecStr(DeclaredEvidence(rule), EIG_0291), DerivedEvidence(claim))` type-checks against `JustifiedBy(_, StrongInhibitor(EIG_0291))`. Full chain of evidence works.

What deferred from the original Phase 1 scope to follow-on commits:

- **`IID` two-sample dispatch** — landed in Phase 1.5 (see entry below).
- **Indexed-inductive observation-type discipline** (§12 #2) — current `observations : core:value_array` admits shape mismatches at runtime via `WrongTestForDesign`; the principled `ObservationFor` upgrade deferred until a real chain pulls on it.
- **Cross-file macro visibility** — Phase 1 fixtures re-declare the smart constructors they call because chain-storing macro decls needs serde derives across the AST. Tracked as the next macro-extension follow-on.
- **Multi-class `data` declarations** — needed to give predicates explicit `PopulationLevel` / `MeasurementLevel` markers without the dual-decl `is_a` collision (§7.4 caveat). Until that lands, Phase 1 uses the default-PopulationLevel admissibility.

**Phase 1.5 — `IID` two-sample dispatch + cross-file macros + multi-class data decls. ✅ LANDED.** Closed the Phase 1 deferrals that the smallest vertical didn't need:

- **`IID` two-sample**: new `stats:IID(group_a, group_b, replication)` smart constructor lands at product position `(CompleteRandom, Unblocked, SingleFactor, _, CrossSectional)`; verifier arm dispatches Pooled vs Welch two-sample t-test based on `variance_assumption`; integration test confirms Holds with |t|≫5, p≪0.001 on cleanly-separated groups.
- **Cross-file macros**: serde derives across the macro-reachable AST; `core:Macro` chain resource carrying the serialized `MacroDecl` per declaration; `collect_macros_from_layer` re-hydrates them at `compile_against_layer` time; fixtures no longer re-declare macros they import.
- **Multi-class data decls**: ESL `data X : T, Marker1, Marker2 { ... }` syntax lands; `DataDecl` gained `extra_classes: Vec<QualifiedName>`; `compile_data` writes them into the resource's `is_a` array. Authors can mark predicates with scope classes (`PopulationLevel` / `MeasurementLevel`) directly on the data declaration.

**Phase 2 — `Paired`. ✅ LANDED.** New `stats:Paired(pairs, replication)` smart constructor at product position `(CompleteRandom, PairedBlocking, SingleFactor, _, CrossSectional)`; observations stored as interleaved `[b_0, a_0, b_1, a_1, …]`; `paired_t_test` numerics (reduces to a one-sample t-test on per-pair differences); integration test confirms Holds with t ≈ 7.4, df = 4, p ≈ 0.002 on a 5-patient pre/post-treatment BP fixture, plus a dispatch-position assertion that paired data routes only through `PairedBlocking`.

**Phase 2.5 — `Factorial` (omnibus k-way ANOVA). ✅ LANDED.** New `stats:Factorial(k, factor_levels, observations, replication)` smart constructor at product position `(CompleteRandom, Unblocked, FullFactorial(k), _, CrossSectional)`; observations slot is a wrapper `[factor_levels, flat_observations]` where each observation row is `k + 1` floats (`k` level indices + value); `factorial_omnibus_anova` numerics via standard sum-of-squares decomposition + `statrs::FisherSnedecor` for the F-distribution p-value. The verifier reports a single F-statistic + one-sided p-value in the same verdict-resource shape the t-based dispatches use (`computed_statistic` is the domain-neutral name). Integration test: a 2×2 design with cell means (10, 20, 30, 40) and within-cell SD ≈ 1 gives F = 500, df = (3, 8), p ≈ 1.9e-9 — clear Holds. **Per-effect decomposition** (main effects + interactions tested separately) is a Phase 5 hardening that lands when claim shapes distinguish those tests; the omnibus F-test is the right scope for v1 because it answers the simple "does the design's factor structure explain any of the variance?" question without committing to a richer multi-effect verdict shape.

**Phase 3 — sampleMap + multi-assay topology. ✅ LANDED (structural prep).** Refactored the SampleSet's `units` / `columns` / `sample_map` slots from `core:string` placeholders to four new structured types:

- `stats:BiologicalUnits.Units(unit_iris)` — flat list of primary-unit IRIs
- `stats:AssayColumns.Columns(pairs)` — interleaved `[assay_0, col_0, assay_1, col_1, …]`
- `stats:SampleMap.Entries(entries)` — array of `SampleMapEntry` ctors
- `stats:SampleMapEntry.Entry(assay_id, primary_iri, col_name)` — the bipartite-graph triple

Tier 1 smart constructors (SingleSampleEstimate, IID, Paired, Factorial) synthesize empty values for the new slots — those dispatches don't need explicit unit identification (it's implicit in the observation row order or the per-arm payload shape). The validate handler's `decode_bundle` parses the new structured types into typed Rust values (`Vec<String>` for units/columns, `Vec<SampleMapEntry>` for the map) but doesn't consume them yet — Phase 4's Tier 2 dispatch arms are where verifier reads start. **No behaviour change in Phase 3**; all existing tests pass unmodified. The chain artifact for every Tier 1 SampleSet now carries the structural shape that Phase 4 builds on without the smart-constructor surfaces changing for the author.

**Phase 4 — Tier 2: `RCBD`, `SplitPlot`, `RepeatedMeasures`.** The mixed-effects verifier infrastructure lands here, split across three sub-phases per the natural complexity ordering:

- **Phase 4.0 — `RCBD`. ✅ LANDED.** New `stats:RCBD(n_blocks, n_treatments, observations, replication)` smart constructor at product position `(Restricted, RCB(n_blocks), SingleFactor, _, CrossSectional)`. Observations encoded as `[block_idx, treatment_idx, value]` rows (3 floats per observation, total `3 · n_blocks · n_treatments`). The verifier extracts `n_blocks` from the `RCB(k)` ctor's integer arg in the blocking slot, infers `n_treatments` from total observation count, validates the complete design (every (block, treatment) cell exactly once), and runs two-way ANOVA reporting the treatment F-test (`F_treatment` ~ F(df_treatment, df_error) under H0). Block effect is computed and stored in `SS_block` for audit but not p-tested in v1 — that's a Phase 5 hardening when claim shapes distinguish "treatment matters" from "block matters." **Numerics caveat**: SS_error computed from per-cell residuals under the additive no-interaction model rather than `SS_total − SS_block − SS_treatment` subtraction, to avoid catastrophic-cancellation loss-of-precision when block variance dominates by orders of magnitude (a real RCBD failure mode I caught and fixed in the numerics tests). Integration test: 3-cohort × 3-dose dose-response design with treatment means (10, 20, 30) → F_treatment > 50, p ≪ 0.001, clear Holds.

- **Phase 4.5 — `SplitPlot`. ✅ LANDED.** New `stats:SplitPlot(a, b, r, observations, replication)` smart constructor at product position `(Restricted, SplitPlotBlocking(a, r), FullFactorial(2), _, CrossSectional)`. The `Blocking` enum gained a `SplitPlotBlocking(a, r)` ctor distinct from `RCB(k)` so the nested-error-stratum dispatch is unambiguous — incorrectly routing split-plot data through the flat `Factorial` arm would use the smaller subplot error for the whole-plot F-test and produce the inflated significance the false-positive shield exists to catch. Observations encoded as `[whole_plot_id, w_level, s_level, value]` rows; the verifier validates that each whole plot has a consistent W level and contains every S level exactly once, and that each W level has exactly r whole-plot replicates. Numerics produces three F-tests: F_W = MS_W / **MS_WP_within_W** (whole-plot factor, against whole-plot error), F_S = MS_S / MS_error (subplot factor, against subplot error), F_WS = MS_WS / MS_error (interaction, against subplot error). SS_error computed directly from per-observation residuals under the additive model to avoid catastrophic-cancellation precision loss (same approach RCBD takes). The verdict reports the **smallest p-value across the three F-tests** in the existing `(computed_statistic, computed_p_value)` slots — a diagnostic note in the verdict-resource enumerates all three (F, p) pairs and names which effect produced the reported one. The dispatch return shape evolved from `(statistic, p)` to `(statistic, p, Option<String>)` so single-test arms continue to pass `None` while SplitPlot threads its omnibus note through. Integration test: 2×2×3 design with cell means (50, 45, 70, 65, …) → Holds with both whole-plot temperature and subplot drug effects detectable, diagnostic naming the dominant effect.

- **Phase 4.9 — `RepeatedMeasures` (constructor + dispatch skeleton). ✅ LANDED.** New `stats:RepeatedMeasures(n_subjects, n_timepoints, k_between_factors, factor_levels, observations, replication)` smart constructor at product position `(CompleteRandom, Unblocked, FullFactorial(k_between_factors), _, Longitudinal(n_timepoints))`. The constructor shape covers all three §5.2.3 cases — time-only (`k_between_factors = 0`), single-treatment longitudinal (`k = 1`), and factorial-RM (`k ≥ 2`) — uniformly; ESL macros can't branch on integers, so `FullFactorial(k)` is used as the single Factor-slot ctor including the degenerate `FullFactorial(0)` for the time-only case. The `Longitudinal(T)` axis on the fifth slot distinguishes RM from cross-sectional `Factorial` (both share `(CompleteRandom, Unblocked, FullFactorial(*))` on the first three axes). Observations slot mirrors Factorial's wrapper: `[factor_levels, flat_observations]`; the verifier cross-checks `factor_levels.len() == k_between_factors`. The verifier reads `autocorrelation_structure` from the claim per §12 #5 and routes across the (autocorrelation × k_between_factors) **completeness matrix** below; each unwired cell rejects with a diagnostic naming the unimplemented combination and the GitHub issue tracking the work. Replacing the previous Phase 4.9.5/4.9.6/… cascade with this matrix is a deliberate structural choice — phase numbers stopped being a useful unit of progress once "done" recedes every time a new design dimension surfaces; the matrix makes incompleteness visible at the same layer the dispatch lives.

  **RepeatedMeasures dispatch completeness:**

  | autocorrelation_structure ↓ × k_between_factors → | `k = 0` (time-only) | `k = 1` (one between-subjects factor) | `k ≥ 2` (factorial-RM) |
  |---|---|---|---|
  | **CompoundSymmetry** (or absent → default) | ✅ wired — `repeated_measures_cs_anova` | ❌ [#79](https://github.com/eigenius/eigenius/issues/79) — factorial-RM (CompoundSymmetry) | ❌ [#79](https://github.com/eigenius/eigenius/issues/79) — factorial-RM (CompoundSymmetry) |
  | **AR(1)** | ❌ [#77](https://github.com/eigenius/eigenius/issues/77) — RM with AR(1) covariance | ❌ [#77](https://github.com/eigenius/eigenius/issues/77) — RM with AR(1) covariance | ❌ [#77](https://github.com/eigenius/eigenius/issues/77) — RM with AR(1) covariance |
  | **Unstructured** | ❌ [#78](https://github.com/eigenius/eigenius/issues/78) — RM with Unstructured covariance | ❌ [#78](https://github.com/eigenius/eigenius/issues/78) — RM with Unstructured covariance | ❌ [#78](https://github.com/eigenius/eigenius/issues/78) — RM with Unstructured covariance |

  The wired `(CompoundSymmetry, k = 0)` cell runs univariate RM-ANOVA — subject as random block, time as fixed factor — algebraically equivalent to RCBD with `subject = block`, `time = treatment`. Observations decode to `[subject, time, value]` triples. SS_error is computed via the per-cell residual formula under the additive subject + time model, matching the catastrophic-cancellation-avoidance approach RCBD and SplitPlot take. The treatment (time) F-test is reported in the existing `(computed_statistic, computed_p_value)` slots; the verdict diagnostic names both the autocorrelation structure and `k_between_factors` used. Integration test: 5-subject × 4-timepoint drug clearance design with clean monotone decline (~-20 units/timepoint) → F_time > 100, p ≪ 1e-6, clear Holds. The unwired cells share natural work-grouping boundaries: AR(1) needs the ρ parameter + GLS once across all k values; Unstructured needs MANOVA-style multivariate tests once across all k values; factorial-RM extends the existing CompoundSymmetry path with a multi-factor fixed-effect decomposition. Each grouping is one GitHub issue, not three phases.

Order can flex on real chain-author pull; Phase 4.0 is the prerequisite for both 4.5 and 4.9 because they share the two-way-ANOVA scaffold. Phase 4 closes at "the Tier 2 dispatch skeleton is in place for all three designs"; unwired RM cells (and any future Tier 2 hardenings) are tracked as GitHub issues against the completeness matrix above, not as Phase 4.x sub-numbers.

**Phase 5 — Opinionated-stance hardening. ✅ LANDED.** All three §7 stances flipped from skeletal to load-bearing in one session because they are orthogonal (each touches a different surface):

- **§7.1 OneSidedWitnessed validation.** New `stats:ImpossibilityWitness` marker class; verifier now accepts `Directionality.OneSidedWitnessed(witness_iri)` only when the witness IRI resolves to a chain-resident resource carrying `is_a stats:ImpossibilityWitness`. The one-sided p-value path halves the two-sided p-value for the alpha comparison; the witness's structural existence on chain (not the test statistic's sign) authorizes the halving. F-based dispatches (Factorial / RCBD / SplitPlot / RepeatedMeasures) reject `OneSidedWitnessed` with a diagnostic explaining that F-statistics are intrinsically non-negative and the directionality refinement doesn't apply. New `DispatchPos::supports_one_sided_directionality()` method captures the t-based / F-based split. Replaces the previous "Phase 1 only supports TwoSided" hard-coded rejection.

- **§7.2 Dual-verdict outlier exclusion.** New `esd_filter(samples, max_outliers, alpha)` numerics implementing Rosner's generalized ESD test (1983) — iteratively flags up to `max_outliers` observations using Studentized deviates against critical values from the one-sided t distribution. The `(SingleSampleEstimate, ESD)` cell of the (dispatch × exclusion) matrix is wired: the verifier computes the test twice (with the exclusion functor applied and on the raw samples), reports the with-exclusion numerics as the primary verdict, and emits a `DualVerdict` diagnostic enumerating both branches plus the excluded original-array indices. All other matrix cells (`PassingBablokResidual` on any dispatch, `Manual` on any dispatch, `ESD` on dispatches other than SingleSampleEstimate) reject with structured diagnostics referencing the tracked follow-on GitHub issue. v1 carries the dual-verdict in the diagnostic string rather than committing two `DerivedResource`s linked via `stats:dual_verdict_pair` — that fuller commit shape is the natural Phase 5.1 follow-on once the institution API supports multi-resource output cleanly.

- **§7.3 MethodComparisonAnalysisPlan + Passing-Bablok.** New `stats:MethodComparisonAnalysisPlan : stats:StatisticalAnalysisPlan` subclass + a second `QueryClass` registration bound to the new subclass (the kernel's AutoOnLoad dispatch matches resource is_a entries directly against registered query_class IRIs — no transitive subclass walk — so subclasses need their own registration even when sharing a handler). New `passing_bablok_regression(method_a, method_b)` numerics: all N·(N−1)/2 pairwise slopes, K-offset median estimator, rank-based 95% CIs via the normal approximation (Passing & Bablok 1983). The validator branches on `claim.is_a` early: when MethodComparisonAnalysisPlan is present, it skips the SampleSet-shape dispatch and runs PB regression on the bundle's paired observations (the same `stats:Paired(...)` authoring surface). Verdict: Holds iff `1.0 ∈ slope_CI ∧ 0.0 ∈ intercept_CI` (the CLSI EP09 method-agreement criterion); Fails with `MethodComparisonDisagreement` diagnostic naming both CIs otherwise. `computed_statistic` carries the median slope; `computed_p_value` carries a binary disagreement indicator (0.0 on agreement, 1.0 on disagreement) so the verdict shape stays uniform across dispatches while the structural decision is the CI check. OneSidedWitnessed directionality is rejected for this dispatch (PB is a CI-based agreement test, not a sign-of-effect test), and non-Identity outlier exclusion is rejected pending the §7.2 follow-on (filed as a GitHub issue) that wires `PassingBablokResidual` on method-comparison data.

Five integration tests in [`phase5_opinionated_stances.rs`](../../crates/eigenius-statistics/tests/phase5_opinionated_stances.rs) cover the load-bearing rejection paths: OneSidedWitnessed Holds with valid witness, OneSidedWitnessed Fails with missing witness, ESD dual-verdict diagnostic with two clear outliers, PB Holds on concordant methods, PB Fails on a 1.5× proportional bias.

The remaining outlier-exclusion matrix cells are tracked as GitHub issues against the (dispatch × exclusion) matrix rather than as Phase 5.x sub-numbers — same structural posture the RM completeness matrix took for the (autocorrelation × k_between) dispatch cells:

- [#80](https://github.com/eigenius/eigenius/issues/80) — ESD on multi-dispatch positions (IID / Paired / Factorial / RCBD / SplitPlot / RM)
- [#81](https://github.com/eigenius/eigenius/issues/81) — PassingBablokResidual exclusion on MethodComparisonAnalysisPlan
- [#82](https://github.com/eigenius/eigenius/issues/82) — materialized two-resource dual-verdict commit shape (current v1 carries dual-verdict in the diagnostic string)
- `Manual` exclusion remains gated on the §11 assay-quality observation institutions (decided in §12 — typed witness validation, no free-form justification).

## 10. Explicitly deferred (Tier 3)

Recorded here so future scope discussions can refer to a fixed list rather than re-deriving the deferral case.

- **Response-surface designs** (Central Composite, Box-Behnken). Industrial process-optimization shape; verification requires quadratic-surface fitting and stationary-point calculus. The institution can be extended with a new constructor + new verifier procedure if a real use case arrives.
- **Crossover designs** (A→B / B→A washout). Carryover effects need context-specific assumptions that the kernel cannot generically verify; deferred until a domain-specific institution can encode the carryover model explicitly.
- **Sequential / group-sequential designs** with alpha-spending (O'Brien-Fleming boundaries etc.). The verifier would need to be aware of historical interim-analysis state, which violates the institution's "verifiable from the SampleSet alone" property. The deferral here is structural, not effort-based — these designs are a poor fit for chain-resident verification full stop and might belong in a separate institution with a different decidability boundary.

## 11. Adjacent institutions (scoped, not specified)

Three institutions sit at the boundary of this one and warrant scoping mention so the boundaries stay clean.

- **Multiple-testing correction institution** (above). Operates over a *set* of `StatisticalAnalysisPlan` resources, applies Bonferroni / Benjamini-Hochberg FDR / Holm / Šidák, and emits an `AdjustedClaim` whose `canonical_proposition` is `HoldsAfterMultipleTestingCorrection(...)`. Consumes the unadjusted `alpha` and unadjusted p-value the per-claim institution stored. Keeping per-claim and aggregate separate is the architectural decoupling that makes both implementable independently.
- **Assay-quality observation institutions** (below). Verify that the SampleSet's raw replicates are themselves trustworthy *before* the statistics institution operates on them. Examples: MIQE PCR-efficiency verification, microscopy image-quality checks, mass-spec calibration-drift detection. Their output is the `SampleSet` itself, structurally validated. They are observation institutions (Decidable on the bench data shape), not derivation institutions (Decidable on a claim about the data).
- **Power and design-justification institution** (alongside, design-time). Verifies at SampleSet-authoring time that the planned `N` is sufficient for the target effect size at the asserted `alpha`. Different dispatch shape — consulted *before* a SampleSet has replicate data, against a design spec — so it does not share an institution with the present one.

## 12. Open questions — decisions, status after Phase 1, and remaining items

The eight open questions identified during D52 review have been walked one-by-one. Decisions are recorded inline; Phase 1 implementation findings are appended where they shifted the answer.

- **ESL smart-constructor ergonomics — SETTLED, cross-file visibility LANDED.** Phase 1 added the `macro` declaration + `Value::MacroCall` extension to ESL (`Declaration::Macro(MacroDecl)` + `TokenKind::Macro`; compile-time AST substitution, no runtime closure). Call sites read cleanly: `stats:SingleSampleEstimate([72.0, 85.0, 100.0], BiologicalReplication())`. The macro is named "Macro" rather than "Function" to honestly reflect that it's compile-time substitution, not a runtime callable — leaves the `Function` AST name available for a future real-function addition. **Phase 1.5 follow-on landed** (cross-file macro visibility): serde derives added across the macro-reachable AST types (`Position`, `QualifiedName`, `Value`, `ResourceField`, `TypeExpr`, `MacroDecl`, `MacroParam`, `TypedParam`, `SortKind`); each `macro` declaration emits a `core:Macro` chain resource carrying the serialized `MacroDecl` AST under `core:macro_decl_json`; new `collect_macros_from_layer` mirrors `collect_ctors_from_layer`; `compile_against_layer` seeds both tables; fixtures no longer re-declare macros they import from a parent layer.

- **Indexed-inductive observation-type discipline — DEFERRED from Phase 1 to a follow-on.** Originally decided "indexed inductives from Phase 1" with ~30–50 line budget. Phase 1 implementation review reversed this: the basic vertical landed faster with runtime rejection (`WrongTestForDesign` from the validator's product-position dispatch), and the indexed-inductive upgrade benefits from a real chain pulling on the shape before being committed to. The kernel infrastructure (D48) is still ready; ESL cost still ~30–50 lines. Lands once a chain author hits a real shape-mismatch friction point or as a planned post-Phase-1.5 hardening.

- **Axis-enum granularity for type-level discrimination — DECIDED, applied minimally.** Hybrid (split where needed, parameterize otherwise). Phase 1 split `PairedBlocking` from `RCB(k)` so paired-vs-RCBD is a ctor distinction; no other splits required yet (no `ObservationFor` to drive further granularity until the indexed-inductive follow-on lands).

- **EffectSize encoding fidelity — DECIDED: inductive with inputs.** `EffectSize.StandardizedCohensD(mean_diff : core:float, pooled_sd : core:float)` shape implemented in the Phase 1 ontology. Verifier currently dispatches only on `Absolute`; the standardized arms' verifier-side input-recovery + derivation check land when the IID two-sample procedure (Phase 1.5) starts using them.

- **RepeatedMeasures autocorrelation structure — DECIDED: claim asserts the structure.** Field added to the universal-claim schema in Phase 1 (`stats:autocorrelation_structure : stats:AutocorrelationStructure`), recommended-not-required because cross-sectional claims don't need it. Verifier-side enforcement lands with the Tier 2 longitudinal verifier in Phase 4.

- **Numerics library choice — DECIDED: `ndarray` + `statrs`.** Both crates added to the `eigenius-statistics` Cargo.toml; one-sample t-test implemented with `statrs::distribution::StudentsT::cdf` against R-reference values, with a bit-identical-across-runs determinism test (D52 §6 reproducibility property). Sets the precedent for any further statistics or mixed-effects crates the institution acquires.

- **`OutlierExclusion.Manual` validation — DECIDED: typed witness only.** `Manual(excluded : core:string)` placeholder shape committed in Phase 1; the proper `Manual(excluded : core:Array<ManualExclusionEntry>)` shape with each entry carrying `{unit_id, quality_check_resource_iri}` will be flipped on when the assay-quality observation institutions (§11) provide real quality-check resources to reference. Until then, Phase 1 admits only `Identity` / `PassingBablokResidual` / `ESD`.

- **Population-vs-measurement-scope predicate marking — DECIDED, ESL-extension follow-on LANDED.** Settled on class-membership + default-to-PopulationLevel: `screen:HasLowIC50 is_a stats:PopulationLevel`, predicates without a marker default to PopulationLevel. **Phase 1 implementation finding** (now resolved): ESL's pre-1.5 `data X` syntax produced a chain resource with `is_a [InductiveType]` only; adding a marker via a companion `resource X : Marker {}` at the same IRI collided via LayerBuilder's last-wins merge. **Phase 1.5 follow-on landed**: ESL `data` syntax extended to accept a comma-separated list of extra classes after the result sort — `data X : Prop, stats:PopulationLevel, OtherMarker { ... }`. `DataDecl` gained an `extra_classes: Vec<QualifiedName>` field; `parse_data` consumes the comma-separated list; `compile_data` appends those IRIs to the resource's `is_a` array alongside the implicit `InductiveType`. Authors can now mark predicates with both scope classes (`PopulationLevel` / `MeasurementLevel`) directly on the data declaration. Default-PopulationLevel behavior continues to apply to predicates that omit the marker.

---

*The architectural commitments this document settles, after Phase 1 implementation: (a) product-typed `SampleSet` with `Bundle` constructor + smart-constructor `macro` wrappers — proven viable, working end-to-end; (b) the universal Claim schema is the standards-intersection above, with `reflection:canonical_proposition` carrying the predicate the claim establishes (one slot shared across DerivedResource subclasses); (c) the institution composes into D39 reasoning via `DerivedEvidence(claim_iri)` consuming the IsDerivedAs witness admitted at claim-commit time — no D39 changes needed, proven end-to-end with the IC50 → StrongInhibitor composition test; (d) the four opinionated stances are non-negotiable defaults; (e) the indexed-inductive observation-type upgrade is the named follow-on for hardening the runtime-rejection fallback. Phasing: Phase 1 LANDED; Phase 1.5 (IID two-sample + cross-file macros + multi-class data decls) next; Phase 2+ proceeds per §9. The order Phase 1 → 1.5 → 2 → 3 → 4 → 5 is the lowest-risk path; specific orderings flex on real chain-author pull.*
