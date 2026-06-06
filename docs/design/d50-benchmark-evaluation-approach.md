# D50 — Benchmark Evaluation Approach

*Status: experimental-design memo · June 2026*

*Companion documents: [D14 institution realisation](d14-institution-realisation.md), [D28 Lean 4 as institution](d28-lean-4-as-institution.md), [D39 justification logic (v2 draft)](d39-justification-logic.md), [D46 Prop universe + axiom framework](d46-prop-universe-and-proof-irrelevance.md), [D47 chain-mirrored EigenTT type fragment](d47-chain-mirrored-eigentt-type-fragment.md), [D48 indexed inductive families](d48-indexed-inductive-families.md), [D49 ChainWitness machinery](d49-chainwitness-machinery.md), [D51 benchmark implementation gaps](d51-benchmark-implementation-gaps.md).*

*This memo specifies the experimental design for the benchmark evaluation the platform manifesto is building toward: testing whether forcing an agent to capture its reasoning as typed justified propositions improves agent decisions on standard scientific-reasoning and engineering-modeling benchmarks. The complementary memo D51 enumerates the implementation gaps that need to close before this experiment can run.*

---

## 1. Hypothesis

**The discipline of authoring reasoning as typed, justified, chain-resident propositions improves agent performance on multi-step scientific and engineering reasoning tasks.**

This is a sharper claim than "Eigenius gates catch errors that opaque artifacts hide" (the earlier framing in the draft publication this memo supports). The mechanism under test is the *forcing function* — the agent is required to articulate each reasoning step as a `ReasoningSentence` with a kernel-checked `JustifiedBy` certificate. The thesis is that this requirement, *independent of what the kernel catches*, structures the agent's decision-making well enough to measurably improve the final deliverable.

The reframing has three consequences for evaluation:

- **The headline metric is benchmark performance**, not a soundness tally. The benchmarks' native scoring (ScienceAgentBench's VER/SR/CBS, EngiBench's per-capability rubric) is the primary axis.
- **The gates that *do* fire are evidence the discipline is non-trivial** — they show the kernel catches structural unsoundness that the agent would otherwise commit. But they are a secondary finding, not the headline.
- **Comparison is against chain-of-thought**, not just opaque baseline. The existing literature shows large performance gains from externalised reasoning of any kind; the interesting question is whether *typed and justified* externalisation adds anything beyond freeform scratchpad.

## 2. Three experimental conditions

| Condition | Agent surface | What gets committed |
|---|---|---|
| **A — Baseline** | The benchmark's native agent protocol. SAB: emits a single Python file. EngiBench: emits a single prose response. | The deliverable, nothing else. |
| **B — Chain-of-thought** | Same agent, but instructed to emit a freeform reasoning trace before the deliverable. The trace is unconstrained prose. | The trace (in a separate field) + the deliverable. |
| **C — Eigenius justified** | Agent authors typed `ReasoningSentence`s with `JustifiedBy` certificates committed to the chain, plus the deliverable. The MCP surface plus the model-then-reason discipline (D39 §4.5) are the surface. | The reasoning chain (committed ESL vocabulary + `ReasoningSentence` sequence + `TaskOutput` referencing the chain) plus the deliverable (extracted from the `TaskOutput.payload`). |

The three-condition design lets us separate two effects:
- **Externalisation effect** (B vs A): does requiring the agent to externalise reasoning at all help?
- **Discipline effect** (C vs B): does requiring the externalisation to be typed, justified, and structurally validated add anything beyond freeform externalisation?

Both deltas are scientifically interesting; together they tell us where the value of the structured-reasoning surface lives.

## 3. Selected problem subset

The pilot uses 26 problems total: 15 from ScienceAgentBench and 11 from EngiBench Level 3. The subset is balanced across domains, mixes complexity levels, and excludes tasks that would dominate the pilot wall-clock (heavy DL training) or require subjective evaluation infrastructure we don't yet have (prose-only EngiBench rubrics).

### ScienceAgentBench (15 tasks)

| # | `instance_id` | Domain | Subtask | Why selected |
|---|---|---|---|---|
| 1 | 16 | Computational Chemistry | Computational Analysis | Compound filter (PAINS/Brenk). Short, RDKit-only — cheap baseline for iteration. |
| 2 | 17 | Computational Chemistry | Feature Eng + Stat + Viz | Chemical-space visualization for A2A-receptor compounds. Medium; multi-decision pipeline. |
| 3 | 28 | Computational Chemistry | Comp + Viz | Charge-density difference via pymatgen + VASP. Physical reasoning with multi-step computation. |
| 4 | 94 | Computational Chemistry | Molecule Visualization | RDKit + networkx molecule rendering. Short; tests discipline overhead on simple tasks. |
| 5 | 21 | GIS | Geospatial Analysis | Deforestation % in 5.5 km road buffer (Rondônia). Clean geopandas/rasterio pipeline. |
| 6 | 48 | GIS | Comp + Map Viz | Leading EOF of SST over N. Pacific (eofs library). Scientific reasoning + standard library. |
| 7 | 64 | GIS | Geospatial + Viz | OGGM glacier flowline comparison 2005 vs 2010. Domain-specific reasoning. |
| 8 | 87 | GIS | Statistical Analysis | Quadratic polynomial fit on NetCDF N. American temperatures. Short numerical with statistical decisions. |
| 9 | 8 | Bioinformatics | Feature Select + Viz | DKPES backward feature selection via logistic regression. Decision-rich; sklearn-only. |
| 10 | 18 | Bioinformatics | Feature Eng + ML | DILI prediction via ECFP + Random Forest. Clean ML pipeline. |
| 11 | 69 | Bioinformatics | Feature Eng + Stat + Viz | scanpy heart-cell atlas: gene filtering + PCA + UMAP. Single-library closed-surface task. |
| 12 | 98 | Bioinformatics | Comp + Viz | scirpy single-cell TCR/RNA-seq chain QC. Multi-step filtering. |
| 13 | 24 | Psychology | Comp + Viz | ECG R-peak detection + outlier correction (BioPsyKit/NeuroKit). Signal-processing reasoning. |
| 14 | 34 | Psychology | Computational Analysis | HRV indices in time/freq/non-linear domains (NeuroKit). Multi-axis domain reasoning. |
| 15 | 45 | Psychology | Computational Analysis | PSS questionnaire score. Very short; baseline for discipline-overhead measurement. |

**Domain spread**: 4 chemistry, 4 GIS, 4 bioinformatics, 3 psychology.
**Complexity spread**: 4 short (~20-30 LOC), 9 medium (~40-80 LOC), 2 longer (28, 98).
**Library spread**: RDKit, pymatgen, geopandas, rasterio, eofs, OGGM, scanpy, scirpy, BioPsyKit, NeuroKit, scikit-learn.
**No DL training in the pilot** — deferred to a follow-up if the result wants the more expensive tail.

### EngiBench Level 3 (11 problems)

| # | Row | Parent (year/problem) | Axis emphasis | Why selected |
|---|---|---|---|---|
| 1 | 1 | 2024 CUMCM B (Industrial / sampling) | All four equal | Series header; sampling-inspection policy under confidence. |
| 2 | 2 | 2024 B (same parent) | DSR-heavy | Decision-network modeling — the discipline should engage hardest here. |
| 3 | 3 | 2024 B (same parent) | All four equal | Multi-stage defect propagation. Tests discipline scaling within a series. |
| 4 | 4 | 2024 CUMCM D (Ocean / depth-charge) | IE+UN+DSR heavy, MOD=0 | First sub-problem; single-objective kill-probability integration. |
| 5 | 5 | 2024 D (same parent) | All four equal | Adds optimization layer. |
| 6 | 6 | 2024 D (same parent) | All four equal | Extended scenarios — hardest in the series. |
| 7 | 38 | 2012 CUMCM D (Control / robot) | All four equal | Robot path-planning in 800×800 obstacle scene. Clean optimization. |
| 8 | 22 | 2016 CUMCM A (Ocean / mooring) | UN-dominant | Tests discipline specifically on uncertainty reasoning. |
| 9 | 33 | 2015 CUMCM C (Aerospace / astronomy) | IE+MOD+DSR, UN=0 | "Moon over willow" — astronomical model derivation; no uncertainty. |
| 10 | 35 | 2014 CUMCM C (Industrial / pig farm) | IE+DSR only, MOD=UN=0 | Min average farrowing under revenue-balance. Deterministic single-objective. |
| 11 | 41 | 2010 CUMCM C (Industrial / pipeline) | IE+UN+DSR heavy, MOD=0 | Oil-pipeline cost-minimization general model with cost-ratio k. |

**Series**: 2024 B (rows 1-3), 2024 D (rows 4-6) — lets the discipline's behaviour be studied across sub-problems sharing one parent statement.
**Axis-emphasis spread**: covers all four profiles (UN-dominant, no-UN, no-MOD, IE+DSR-only, balanced).
**Decade spread**: 7 distinct CUMCM competition years — avoids the pilot becoming a study of one style.
**Open-ended outliers excluded**: the 2015 CUMCM B taxi rows (27-29) have prose-only rubrics; deferred to a follow-on with human-grader infrastructure.

## 4. Per-family base ontologies

The vocabulary-engineering decision settled in the D39 §4.5 update: thin per-family base ontologies authored once, agent extends per task. Six base ontologies for the pilot, each ~5-10 ESL declarations:

| Family namespace | Covers pilot tasks | Anchor classes (illustrative; final shape settled during authoring) |
|---|---|---|
| `bench:chem` | SAB 16, 17, 28, 94 | `Compound` (carries SMILES), `Reaction`, `Molecule`, `Property` (toxicity, solubility, …), `Measurement` (with units), `Predicted` vs `Observed` distinction, `ToxicityClass`. |
| `bench:gis` | SAB 21, 48, 64, 87 | `SpatialFeature`, `RasterLayer`, `CoordinateReferenceSystem`, `Buffer`, `Polygon`, `TemperatureSeries`, `Glacier`. |
| `bench:bio` | SAB 8, 18, 69, 98 | `Cell`, `Gene`, `Expression`, `Protein`, `MLClassifier`, `FeatureSet`, `Sample`. |
| `bench:psych` | SAB 24, 34, 45 | `Signal`, `ECGRecord`, `HRVIndex`, `Subject`, `QuestionnaireResponse`, `ValidatedScore`. |
| `bench:mfg` | EngiBench 1-3, 30 | `Component`, `Process`, `Decision`, `Cost`, `ConfidenceLevel`, `HypothesisTest`, `InspectionPolicy`, `DefectRate`. |
| `bench:opt` | EngiBench 4-6, 22, 33, 35, 38, 41 | `Variable`, `Constraint`, `Objective`, `FeasibleRegion`, `Solution`, `ProbabilityModel`, `OptimizationProblem`. |

Each base is committed as a layer parent before any pilot run. The agent's vocabulary phase (D39 §4.5) extends these with the task-specific specifics (predicates over compound IDs, per-task decision rules, per-task domain-specific predicates).

**Per-task vocabulary hints**: a small per-task hint file ships in the harness alongside each pilot problem, listing 3-5 suggested predicate names for the task-specific vocabulary. This is borderline confound vs. clean experiment; the rationale for including it is that without naming-convention hints, cross-run drift on predicate names becomes the dominant noise source. We document the hints as part of the experimental protocol and report whether the agent followed them (a derived metric).

## 5. Harness architecture

The harness drives the three conditions against each pilot problem and records the artifacts each produces. Sketch (concrete shape settled in D51's gap inventory):

```
benchmark-harness/
├── conditions/
│   ├── baseline_runner.py      # condition A — wraps the benchmark's native agent
│   ├── cot_runner.py           # condition B — same agent + CoT instruction
│   └── eigenius_runner.py      # condition C — drives the Eigenius MCP surface
├── tasks/
│   ├── sab/                    # 15 ScienceAgentBench tasks
│   │   ├── 16-compound-filter/
│   │   │   ├── task.json       # task instruction, dataset, eval script ref
│   │   │   └── hints.esl       # per-task vocabulary hints (~5 lines)
│   │   └── …
│   └── engibench/              # 11 EngiBench Level 3 problems
│       └── …
├── base-ontologies/
│   ├── chem.esl
│   ├── gis.esl
│   ├── bio.esl
│   ├── psych.esl
│   ├── mfg.esl
│   └── opt.esl
├── scoring/
│   ├── sab_score.py            # wraps the benchmark's eval scripts
│   ├── engibench_score.py      # LLM-judge rubric scoring (pinned judge)
│   └── derived_metrics.py      # gate-firing tally, vocabulary size, time-cost
└── runs/
    └── <run-id>/<condition>/<task>/  # per-cell run artifacts
```

The harness is per-pilot infrastructure, not a productised platform feature. It lives in a sibling repo (or under `experiments/` in this repo); production code does not depend on it.

## 6. Scoring and metrics

### 6.1 Primary metrics (per-benchmark native)

- **ScienceAgentBench**: VER (Valid Execution Rate), SR (Success Rate), CBS (CodeBERTScore), cost. SR is the headline.
- **EngiBench Level 3**: per-capability rubric score (information_extraction, multi_objective_decision, uncertainty_handling, domain_specific_reasoning), aggregated per problem and per condition. Total rubric score is the headline; per-axis breakdown is a secondary view.

### 6.2 Cross-cutting metrics (all conditions)

- **Wall-clock time per task** (per condition). The structured condition's overhead is real; report it explicitly rather than averaging it away.
- **Token cost per task** (per condition). Same rationale.

### 6.3 Eigenius-specific derived metrics (condition C only)

These are secondary findings supporting the discipline thesis with structural evidence:

- **Gate-firing tally per task**: how many `ValidateJustification` rejections did the agent encounter, classified by failure mode (missing prior, ungrounded justification, ill-typed proposition, vocabulary error, …). Reports "the discipline catches real things."
- **Vocabulary size**: number of agent-authored classes / properties / axioms per task. Tests whether the discipline produces parsimonious models or sprawling ones.
- **Reasoning chain depth**: number of `ReasoningSentence`s committed per task, plus the average and max `JustificationTerm` tree depth. Proxies for reasoning structure.
- **Citation density**: fraction of `JustificationTerm` constructors that are `DerivedEvidence`-citations to prior sentences (vs. fresh groundings in declared / observed resources). Tests whether the agent builds *on its own prior reasoning* or starts fresh per step.
- **Trade-off pattern usage**: count of decisions made using the §6.4 pattern (alternatives clustered by `subject_iri` + final pick-sentence). Tests whether the agent recognises decision shapes when they appear.

### 6.4 Headline comparison

The headline result is a per-benchmark, per-condition table:

| Condition | SAB SR | SAB VER | SAB cost | EngiBench rubric | EngiBench cost |
|---|---|---|---|---|---|
| A (baseline) | … | … | … | … | … |
| B (CoT) | … | … | … | … | … |
| C (Eigenius) | … | … | … | … | … |

Plus separate plots of (i) wall-clock and token cost per condition; (ii) condition C's gate-firing tally and vocabulary statistics.

## 7. Pilot phasing

**Phase 0 — shakedown (3 tasks).** Stand up the three-condition runner against three tasks before scaling: SAB 45 (shortest), SAB 17 (medium-complexity chem), EngiBench row 1 (structured industrial decision). The goal of Phase 0 is operational, not statistical — find out whether the harness orchestrates the three conditions cleanly, whether the Eigenius condition's agent loop converges within token budgets, whether scoring runs without manual intervention.

**Phase 1 — full pilot (26 tasks × 3 conditions × 3 replicates = 234 runs).** Once Phase 0 is clean, run the full pilot. At ~10 min agent time per run, this is ~40 hours of agent time — affordable on a single workstation, dramatically less with parallel orchestration.

**Scale-up criteria.** Phase 1 results inform whether to expand:

- If condition C ≥ condition B on the headline (statistical noise aside), scale up: full SAB (102 tasks including the deferred DL-training tail), full EngiBench Level 3 (43 problems).
- If condition C ≈ condition B but the discipline-specific metrics (vocabulary parsimony, gate-firing tally, structural soundness) tell an independent story, the publication direction is "Eigenius produces equivalently good answers with structurally auditable provenance" — still publishable, different framing.
- If condition C < condition B consistently, debug: is the discipline being followed earnestly, is the agent fighting the surface, is the kernel-side diagnostic quality the bottleneck. The Phase 0 shakedown is supposed to surface the first two; the third is more subtle and may require iteration on the agent skill / kernel error messages.

## 8. Risks (operational, not architectural)

These are the risks specific to the experimental design. Architectural-soundness risks are in D49 / D39 / the implementation-gaps memo D51.

**Vocabulary drift across runs.** Two condition-C runs on the same task may invent different predicate names. Mitigation: per-task vocabulary hints (§4); compare in the analysis with-and-without hint-following as a derived metric.

**Discipline-overhead skews comparison.** Condition C will take longer per task than A or B. This is honest cost; report it explicitly. Don't hide it in averaged metrics.

**LLM-judge variance on EngiBench.** Pin the judge model and version. Cross-check 2 of the 11 problems with a 2nd judge model; document agreement.

**Agent gaming via commit-then-retract.** The current `refutes` semantics is deliberately loose (D39 §9 defers it to chain-merge work). For the pilot, score only the non-retracted sentences as the agent's reasoning; weight retraction patterns in the analysis.

**Per-task wall-clock outliers.** Some tasks may take 30+ minutes in condition C due to the discipline overhead. Hard timeout at 30 min per task per condition; tasks that time out are reported separately (count + which condition) rather than treated as failures.

**Phase 0 fails to converge.** If after a week of Phase 0 iteration the three-condition runner is not producing comparable artifacts, the harness design is wrong; revisit before committing to Phase 1.

## 9. Out of scope for the pilot

- **Soundness tally as a headline metric** (the earlier framing). Re-evaluated as a secondary finding (§6.3); not the primary axis.
- **Four-gate concrete demo** (drug-candidate, dock_to_assay, Lean verdict). Useful as qualitative evidence the discipline has structural content, but a separate worked example, not part of the benchmark pilot. Authored in parallel for the publication's introduction.
- **Domain coverage beyond the six base ontologies.** Geopandas / DeepChem / BioPsyKit institutions don't exist in Eigenius today; the pilot works around this by treating tool invocations as typed-boundary Components (the agent declares "I ran tool T with input I, the result is a TypedOutput") rather than building institutional wrappers for each library. See D51 §3 for the workaround's specifics.
- **Hierarchical reasoning patterns.** If the pilot shows agents struggling with deep `App`-tree composition, revisit; otherwise the flat-list-of-`ReasoningSentence`s pattern is what we test.
- **Auto-generation of EngiBench prose from the reasoning chain.** Open question; pilot evaluates both "agent writes prose separately" and "prose auto-generated from chain" and reports both.

## 10. Relationship to other documents

- **[D39 v2](d39-justification-logic.md)** — provides the `ReasoningSentence` + `JustifiedBy` + `TaskOutput` substrate the condition-C agent surface uses. §4.5 (model-then-reason) is the methodological commitment the agent skill teaches. §4.4 (`TaskOutput`) is the benchmark-deliverable chain shape.
- **[D49](d49-chainwitness-machinery.md)** — provides the `ChainWitness` machinery that makes `JustifiedBy` certificates type-checkable at commit. Implementation status: design memo; not yet built.
- **[D51 benchmark implementation gaps](d51-benchmark-implementation-gaps.md)** — companion memo enumerating the implementation work that must close before the pilot can run. Required reading before scheduling Phase 0.
- **D14 / D26 / D27 / D28** — the existing institutional substrate the discipline is layered on top of. The base ontologies (§4) cite these where relevant (e.g., chemistry tasks that engage Symbolics or Catalyst go through D27's existing institution dispatch).

---

*This is an experimental-design memo. The hypothesis, the three conditions, the problem subset, and the derived metrics are the load-bearing decisions and should be the focus of review. The harness file layout (§5) and the per-task vocabulary hints (§4) are first-draft proposals expected to be refined during Phase 0.*
