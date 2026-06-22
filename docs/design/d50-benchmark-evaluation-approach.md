# D50 — Benchmark Evaluation Approach

*Status: experimental-design memo · June 2026 · **scope narrowed 2026-06-11***

*Companion documents: [D14 institution realisation](d14-institution-realisation.md), [D28 Lean 4 as institution](d28-lean-4-as-institution.md), [D39 justification logic (v2 draft)](d39-justification-logic.md), [D46 Prop universe + axiom framework](d46-prop-universe-and-proof-irrelevance.md), [D47 chain-mirrored EigenTT type fragment](d47-chain-mirrored-eigentt-type-fragment.md), [D48 indexed inductive families](d48-indexed-inductive-families.md), [D49 ChainWitness machinery](d49-chainwitness-machinery.md), [D51 benchmark implementation gaps](d51-benchmark-implementation-gaps.md).*

*This memo specifies the experimental design for the benchmark evaluation the platform manifesto is building toward: testing whether forcing an agent to capture its reasoning as typed justified propositions improves agent decisions on standard scientific-reasoning and engineering-modeling benchmarks. The complementary memo D51 enumerates the implementation gaps that need to close before this experiment can run.*

> **Pilot scope (2026-06-11): chem + bio.** The pilot is narrowed to the **computational-chemistry and bioinformatics** subset of ScienceAgentBench — 8 tasks across two base ontologies. GIS, psychology, and the entire EngiBench Level 3 set (engineering / math-modeling problems, no chem/bio content) are **deferred to the scale-up tail** (§7). The three-condition design, the metrics, the `TaskOutput` deliverable handle, and the harness architecture are unchanged; only the problem subset (§3), the base ontologies (§4), and the pilot phasing (§7) narrow. Deferred-family design content is retained below for the scale-up, marked where it applies. The kernel/ontology critical path this pilot rides on (D49 ChainWitness, the D39 v2 Reasoning institution) is **implemented** as of this revision — see D51 §0.

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
| **C — Eigenius justified** | Agent authors typed `ReasoningSentence`s with `JustifiedBy` certificates committed to the chain, plus the deliverable. The MCP surface plus the model-then-reason discipline (D39 §4.5) are the surface. | The reasoning chain (committed ESL vocabulary + `ReasoningSentence` sequence + `benchmark:TaskOutput` referencing the chain — see §5b) plus the deliverable (extracted from the `TaskOutput.payload`). |

The three-condition design lets us separate two effects:
- **Externalisation effect** (B vs A): does requiring the agent to externalise reasoning at all help?
- **Discipline effect** (C vs B): does requiring the externalisation to be typed, justified, and structurally validated add anything beyond freeform externalisation?

Both deltas are scientifically interesting; together they tell us where the value of the structured-reasoning surface lives.

## 3. Selected problem subset

**Pilot scope (2026-06-11): 8 ScienceAgentBench tasks — computational chemistry + bioinformatics.** The subset mixes complexity levels within the two domains and excludes tasks that would dominate the pilot wall-clock (heavy DL training). GIS, psychology, and EngiBench are deferred to the scale-up tail (retained in §3.2 below for that purpose).

### 3.1 ScienceAgentBench — chem + bio (8 tasks, pilot)

| # | `instance_id` | Domain | Subtask | Why selected |
|---|---|---|---|---|
| 1 | 16 | Computational Chemistry | Computational Analysis | Compound filter (PAINS/Brenk). Short, RDKit-only — cheap baseline for iteration. |
| 2 | 17 | Computational Chemistry | Feature Eng + Stat + Viz | Chemical-space visualization for A2A-receptor compounds. Medium; multi-decision pipeline. |
| 3 | 28 | Computational Chemistry | Comp + Viz | Charge-density difference via pymatgen + VASP. Physical reasoning with multi-step computation. |
| 4 | 94 | Computational Chemistry | Molecule Visualization | RDKit + networkx molecule rendering. Short; tests discipline overhead on simple tasks. |
| 5 | 8 | Bioinformatics | Feature Select + Viz | DKPES backward feature selection via logistic regression. Decision-rich; sklearn-only. |
| 6 | 18 | Bioinformatics | Feature Eng + ML | DILI prediction via ECFP + Random Forest. Clean ML pipeline. |
| 7 | 69 | Bioinformatics | Feature Eng + Stat + Viz | scanpy heart-cell atlas: gene filtering + PCA + UMAP. Single-library closed-surface task. |
| 8 | 98 | Bioinformatics | Comp + Viz | scirpy single-cell TCR/RNA-seq chain QC. Multi-step filtering. |

**Domain spread**: 4 chemistry, 4 bioinformatics.
**Complexity spread**: 3 short (16, 94, and the lighter end of 8), ~4 medium (~40-80 LOC), 2 longer (28, 98).
**Library spread**: RDKit, pymatgen, scikit-learn, scanpy, scirpy.
**No DL training in the pilot** — deferred to a follow-up if the result wants the more expensive tail.

### 3.2 Deferred to scale-up (GIS, psychology, EngiBench)

These were in the original 26-problem subset and are retained for the scale-up phase (§7); they are **not** part of the chem+bio pilot.

**ScienceAgentBench — GIS (4 tasks)**: 21 (deforestation % in 5.5 km road buffer, Rondônia — geopandas/rasterio), 48 (leading EOF of SST over N. Pacific — eofs), 64 (OGGM glacier flowline comparison 2005 vs 2010), 87 (quadratic polynomial fit on NetCDF N. American temperatures).

**ScienceAgentBench — Psychology (3 tasks)**: 24 (ECG R-peak detection + outlier correction — BioPsyKit/NeuroKit), 34 (HRV indices in time/freq/non-linear domains — NeuroKit), 45 (PSS questionnaire score — very short).

**EngiBench Level 3 (11 problems)** — engineering / math-modeling; no chem/bio content, hence out of pilot scope. Also carries the heaviest harness dependency (the LLM-judge rubric scorer + inter-judge calibration, D51 gap 7/8). Retained set:

| # | Row | Parent (year/problem) | Axis emphasis |
|---|---|---|---|
| 1 | 1 | 2024 CUMCM B (Industrial / sampling) | All four equal |
| 2 | 2 | 2024 B (same parent) | DSR-heavy |
| 3 | 3 | 2024 B (same parent) | All four equal |
| 4 | 4 | 2024 CUMCM D (Ocean / depth-charge) | IE+UN+DSR heavy, MOD=0 |
| 5 | 5 | 2024 D (same parent) | All four equal |
| 6 | 6 | 2024 D (same parent) | All four equal |
| 7 | 38 | 2012 CUMCM D (Control / robot) | All four equal |
| 8 | 22 | 2016 CUMCM A (Ocean / mooring) | UN-dominant |
| 9 | 33 | 2015 CUMCM C (Aerospace / astronomy) | IE+MOD+DSR, UN=0 |
| 10 | 35 | 2014 CUMCM C (Industrial / pig farm) | IE+DSR only, MOD=UN=0 |
| 11 | 41 | 2010 CUMCM C (Industrial / pipeline) | IE+UN+DSR heavy, MOD=0 |

The 2024 B (rows 1-3) and 2024 D (rows 4-6) series, the four axis-emphasis profiles, and the 7-year decade spread are the rationale for this set when EngiBench is pulled back in. The 2015 CUMCM B taxi rows (27-29) have prose-only rubrics and stay excluded even at scale-up pending human-grader infrastructure.

## 4. Per-family base ontologies

The vocabulary-engineering decision settled in the D39 §4.5 update: thin base ontologies authored once, agent extends per task. **Revised 2026-06-12 — shared spine + data-shape modules.** Grounding the bases against the eight pilot tasks (see the per-task sketches in `docs/notes/chem-bio-pilot-execution-plan.md`) surfaced two facts that reshape the original per-family table:

1. **A domain-agnostic spine recurs in every task** — the typed tool boundary (the kernel checks "tool T produced typed output O from typed input I", not the computation; D50 §9 / D51 §12), the `Measurement(value, unit)` shape (reused from `statistics.esl`), and the input-`Dataset` anchor. Repeating this per family invites drift, so it is factored into a single **`bench-core`** spine.
2. **The clean module cut is by *data shape*, not by SAB domain label.** Bioinformatics tasks 8 and 18 are molecule-centric (`Compound` / SMILES / ECFP fingerprints) — identical to the chemistry tasks. Cutting `chem.esl` vs `bio.esl` would force `Compound` to be duplicated or cross-imported. Cutting by data shape keeps every noun in exactly one home.

The pilot therefore authors **`bench-core` + three thin modules** (`mol`, `materials`, `singlecell`), each extending `bench-core`:

| Module (extends) | Status | Nouns | Pilot tasks |
|---|---|---|---|
| `bench-core` (→ reflection) | **pilot** | `ToolArtifact` (typed tool boundary), `Measurement` (value+unit), `Dataset`, `concerns` (linking predicate) | all |
| `mol` (→ bench-core) | **pilot** | `Compound` (SMILES), `Fingerprint`, `ActivityMeasurement`, `Target` | 16, 17, 94, **8, 18** |
| `materials` (→ bench-core) | **pilot** | `CrystalStructure`, density artifacts (as `ToolArtifact`s) | 28 |
| `singlecell` (→ bench-core) | **pilot** | `Cell`, `Gene`, `ExpressionMatrix`, `CellType`, `ChainPairing` | 69, 98 |
| `ml` facet (→ bench-core) | **pilot** | `FeatureSet`, `Classifier`, `CVScore`, `Prediction` (may fold into `mol` if it stays small) | 8, 18 |

The SAB "chem"/"bio" labels remain at the *task/domain* level (problem subset §3, scoring); the *ontology modules* cut by data shape. Each module is ~5-10 ESL declarations. Authoring order: `bench-core` + `mol` first (they carry the SAB 16 tracer), then `materials` + `singlecell`.

**Deferred to scale-up** (further modules on `bench-core`, authored only if §7's criteria are met): GIS (`SpatialFeature`, `RasterLayer`, `CRS`, `Buffer`, `Polygon`, `TemperatureSeries`, `Glacier`), psychology (`Signal`, `ECGRecord`, `HRVIndex`, `Subject`, `QuestionnaireResponse`, `ValidatedScore`), and the EngiBench manufacturing/optimization vocabularies (`Component`/`Process`/`Decision`/…, `Variable`/`Constraint`/`Objective`/…).

Each module is committed as a layer parent before any pilot run. The agent's vocabulary phase (D39 §4.5) extends these with the task-specific specifics (domain predicates, per-task decision rules) — see the eight per-task templates in the execution-plan note.

**Per-task vocabulary hints**: a small per-task hint file ships in the harness alongside each pilot problem, listing 3-5 suggested predicate names for the task-specific vocabulary. This is borderline confound vs. clean experiment; the rationale for including it is that without naming-convention hints, cross-run drift on predicate names becomes the dominant noise source. We document the hints as part of the experimental protocol and report whether the agent followed them (a derived metric).

## 5. Harness architecture

The harness drives the three conditions against each pilot problem and records the artifacts each produces. Sketch below shows the **full** shape (concrete chem+bio-pilot scope settled in D51's gap inventory — the `engibench/` tasks, `engibench_score.py`, and the `gis`/`psych`/`mfg`/`opt` base ontologies are `‹scale-up›`, built only when those families are pulled back in):

```
benchmark-harness/
├── conditions/
│   ├── baseline_runner.py      # condition A — wraps the benchmark's native agent
│   ├── cot_runner.py           # condition B — same agent + CoT instruction
│   └── eigenius_runner.py      # condition C — drives the Eigenius MCP surface
├── tasks/
│   ├── sab/                    # 8 ScienceAgentBench chem+bio tasks (pilot)
│   │   ├── 16-compound-filter/
│   │   │   ├── task.json       # task instruction, dataset, eval script ref
│   │   │   └── hints.esl       # per-task vocabulary hints (~5 lines)
│   │   └── …
│   └── engibench/              # ‹scale-up› 11 EngiBench Level 3 problems
│       └── …
├── base-ontologies/
│   ├── bench-core.esl          # pilot — shared spine (→ reflection)
│   ├── mol.esl                 # pilot — molecules (16,17,94,8,18)
│   ├── materials.esl           # pilot — crystals/density (28)
│   ├── singlecell.esl          # pilot — cells/genes/TCR (69,98)
│   ├── gis.esl                 # ‹scale-up›
│   ├── psych.esl               # ‹scale-up›
│   ├── mfg.esl                 # ‹scale-up›
│   └── opt.esl                 # ‹scale-up›
├── scoring/
│   ├── sab_score.py            # wraps the benchmark's eval scripts (pilot)
│   ├── engibench_score.py      # ‹scale-up› LLM-judge rubric scoring (pinned judge)
│   └── derived_metrics.py      # gate-firing tally, vocabulary size, time-cost
└── runs/
    └── <run-id>/<condition>/<task>/  # per-cell run artifacts
```

The harness is per-pilot infrastructure, not a productised platform feature. It lives in a sibling repo (or under `experiments/` in this repo); production code does not depend on it.

## 5b. The `TaskOutput` Resource

`TaskOutput` is the chain-resident deliverable handle for condition C. It pairs the artifact the task asked for (Python source, prose, JSON, or a resource set) with an explicit pointer chain back to the `ReasoningSentence`s that justified its content. This is what the scoring harness consumes; it is also what makes the chain "complete in itself" — the agent's deliverable explicitly references which reasoning sentences justified its content, so an auditor can ask "for this Python file, which steps in the agent's reasoning produced which behaviour?" and walk the chain to find out.

`TaskOutput` was originally specified in D39 §4.4. On review during D39 Phase 4 implementation, it was relocated here: the class is justified entirely by benchmark evaluation (every property is benchmark-shaped) and putting it in the foundational Reasoning ontology would pollute that ontology with downstream-consumer concerns. The Reasoning institution does not need or reference `TaskOutput`.

| Property | Type | Required? | Reading |
|---|---|---|---|
| `is_a` | `[reflection:DerivedResource, benchmark:TaskOutput]` | yes | A subclass of `DerivedResource` like `ReasoningSentence`. |
| `task` | `core:iri` | yes | The task IRI this output answers. Provides task-scoped identity for the deliverable. |
| `deliverable_kind` | enumeration string | yes | What kind of artifact this is. Initial values: `"python_source"`, `"prose"`, `"json"`, `"resource_set"`. New kinds added as needed by future task families. |
| `payload` | `core:string` (or a class-specific shape for `resource_set`) | yes | The actual artifact content the task asked for. For `python_source` / `prose` / `json` this is a literal; for `resource_set` it's a list of chain IRIs. |
| `reasoning_chain` | array of `core:iri` referencing `ReasoningSentence`s | yes | The reasoning sentences this output rests on, in commit order. Auditors trace from the deliverable to the warrant. The kernel does not enforce that every line of the payload corresponds to a sentence in the chain — that's a methodological commitment, not a structural one — but commit-time validation checks that every IRI in this array resolves to a `ReasoningSentence` on the chain. |
| `derivation` (inherited) | reference to a `reflection:ProgramTrace` | yes (by `DerivedResource`'s `requires` list) | Trace of the program (or agent loop) that produced the deliverable from the reasoning chain. |

Implementation site: a benchmark-harness ontology (e.g. `experiments/benchmark/harness-ontology.esl`) declaring the `benchmark:TaskOutput` class and its properties, loaded as a sibling layer to the per-family base ontologies. The namespace is `benchmark`, not `reasoning` — the Reasoning institution stays unaware.

## 6. Scoring and metrics

### 6.1 Primary metrics (per-benchmark native)

- **ScienceAgentBench** (pilot): VER (Valid Execution Rate), SR (Success Rate), CBS (CodeBERTScore), cost. SR is the headline. This is the **only** scorer the chem+bio pilot needs — SAB's eval scripts are deterministic, so no LLM-judge infrastructure is on the pilot critical path.
- **EngiBench Level 3** (deferred to scale-up): per-capability rubric score (information_extraction, multi_objective_decision, uncertainty_handling, domain_specific_reasoning), aggregated per problem and per condition. Total rubric score is the headline; per-axis breakdown is a secondary view. Requires the pinned-LLM-judge rubric scorer and inter-judge calibration (D51 gap 7/8) — built only when EngiBench is pulled back in.

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

The headline result is a per-condition table. For the chem+bio pilot it is SAB-only (the EngiBench columns return at scale-up):

| Condition | SAB SR | SAB VER | SAB CBS | SAB cost |
|---|---|---|---|---|
| A (baseline) | … | … | … | … |
| B (CoT) | … | … | … | … |
| C (Eigenius) | … | … | … | … |

Plus separate plots of (i) wall-clock and token cost per condition; (ii) condition C's gate-firing tally and vocabulary statistics. At scale-up the table regains `EngiBench rubric` and `EngiBench cost` columns.

## 7. Pilot phasing

**Phase 0 — shakedown (3 tasks).** Stand up the three-condition runner against three chem+bio tasks before scaling: SAB 16 (shortest chem — compound filter), SAB 17 (medium-complexity chem — chemical-space viz), SAB 18 (bio — DILI ML pipeline). The goal of Phase 0 is operational, not statistical — find out whether the harness orchestrates the three conditions cleanly, whether the Eigenius condition's agent loop converges within token budgets, whether scoring runs without manual intervention.

**Phase 1 — chem+bio pilot (8 tasks × 3 conditions × 3 replicates = 72 runs).** Once Phase 0 is clean, run the full chem+bio pilot. At ~10 min agent time per run, this is ~12 hours of agent time — easily affordable on a single workstation, less with parallel orchestration.

**Scale-up criteria.** Phase 1 results inform whether and how far to expand:

- If condition C ≥ condition B on the headline (statistical noise aside), scale up in two steps: first the **deferred ScienceAgentBench domains** (GIS + psychology, §3.2) to test whether the result holds across domain families; then the broader tail — full SAB (102 tasks including the deferred DL-training tail) and **EngiBench Level 3** (which additionally requires the LLM-judge scoring + calibration infrastructure deferred in D51 gap 7/8).
- If condition C ≈ condition B but the discipline-specific metrics (vocabulary parsimony, gate-firing tally, structural soundness) tell an independent story, the publication direction is "Eigenius produces equivalently good answers with structurally auditable provenance" — still publishable, different framing.
- If condition C < condition B consistently, debug: is the discipline being followed earnestly, is the agent fighting the surface, is the kernel-side diagnostic quality the bottleneck. The Phase 0 shakedown is supposed to surface the first two; the third is more subtle and may require iteration on the agent skill / kernel error messages.

## 8. Risks (operational, not architectural)

These are the risks specific to the experimental design. Architectural-soundness risks are in D49 / D39 / the implementation-gaps memo D51.

**Vocabulary drift across runs.** Two condition-C runs on the same task may invent different predicate names. Mitigation: per-task vocabulary hints (§4); compare in the analysis with-and-without hint-following as a derived metric.

**Discipline-overhead skews comparison.** Condition C will take longer per task than A or B. This is honest cost; report it explicitly. Don't hide it in averaged metrics.

**LLM-judge variance on EngiBench.** *(Scale-up only — EngiBench is deferred out of the chem+bio pilot, so the pilot has no LLM-judge in the loop; SAB scoring is deterministic.)* When EngiBench returns: pin the judge model and version. Cross-check 2 of the 11 problems with a 2nd judge model; document agreement.

**Agent gaming via commit-then-retract.** The current `refutes` semantics is deliberately loose (D39 §9 defers it to chain-merge work). For the pilot, score only the non-retracted sentences as the agent's reasoning; weight retraction patterns in the analysis.

**Per-task wall-clock outliers.** Some tasks may take 30+ minutes in condition C due to the discipline overhead. Hard timeout at 30 min per task per condition; tasks that time out are reported separately (count + which condition) rather than treated as failures.

**Phase 0 fails to converge.** If after a week of Phase 0 iteration the three-condition runner is not producing comparable artifacts, the harness design is wrong; revisit before committing to Phase 1.

## 8a. Prior art — the typed-KG-generation benchmark landscape

The external landscape for generating typed knowledge graphs (and formal statements) from text frames condition C's metrics. Text2KGBench ([`mihindukulasooriya2023text2kgbench`]) scores both ontology conformance and subject/relation/object hallucination — the closest analogue to what the discipline thesis measures here. LLMs4OL ([`babaeigiglou2023llms4ol`]) decomposes ontology learning into term typing, taxonomy discovery, and non-taxonomic relation extraction (and finds foundational LLMs alone insufficient for high-reasoning ontology construction). SPIRES ([`caufield2024spires`]) is the schema-constrained, ontology-ID-grounded extraction precedent — structurally the same as the pilot's typed-tool-boundary workaround (§9, D51 §12). The autoformalization-faithfulness work (Herald / miniF2F-Lean Revisited / ReForm; see D28 §1.3, D30 §1.3) is the corresponding cautionary landscape on the proof side.

## 9. Out of scope for the pilot

- **Soundness tally as a headline metric** (the earlier framing). Re-evaluated as a secondary finding (§6.3); not the primary axis.
- **Four-gate concrete demo** (drug-candidate, dock_to_assay, Lean verdict). Useful as qualitative evidence the discipline has structural content, but a separate worked example, not part of the benchmark pilot. Authored in parallel for the publication's introduction.
- **Domain coverage beyond the pilot's two base ontologies.** RDKit / DeepChem / scanpy / scirpy institutions don't exist in Eigenius today; the pilot works around this by treating tool invocations as typed-boundary Components (the agent declares "I ran tool T with input I, the result is a TypedOutput") rather than building institutional wrappers for each library. See D51 §12 for the workaround's specifics.
- **Hierarchical reasoning patterns.** If the pilot shows agents struggling with deep `App`-tree composition, revisit; otherwise the flat-list-of-`ReasoningSentence`s pattern is what we test.
- **Auto-generation of EngiBench prose from the reasoning chain.** Open question; pilot evaluates both "agent writes prose separately" and "prose auto-generated from chain" and reports both.

## 10. Relationship to other documents

- **[D39 v2](d39-justification-logic.md)** — provides the `ReasoningSentence` + `JustifiedBy` substrate the condition-C agent surface uses. §4.5 (model-then-reason) is the methodological commitment the agent skill teaches. (The `TaskOutput` deliverable-handle was originally specified in D39 §4.4 but was relocated to D50 §5b on review — it is benchmark-scoped, not Reasoning-scoped.)
- **[D49](d49-chainwitness-machinery.md)** — provides the `ChainWitness` machinery that makes `JustifiedBy` certificates type-checkable at commit. Implementation status (2026-06-11): **built** for the non-Lean witness families (`#76`); the Lean `IsVerifiedAs` producer path is the one outstanding piece (D51 gap 2), not needed by the chem+bio pilot.
- **[D51 benchmark implementation gaps](d51-benchmark-implementation-gaps.md)** — companion memo enumerating the implementation work that must close before the pilot can run. Required reading before scheduling Phase 0.
- **D14 / D26 / D27 / D28** — the existing institutional substrate the discipline is layered on top of. The base ontologies (§4) cite these where relevant (e.g., chemistry tasks that engage Symbolics or Catalyst go through D27's existing institution dispatch).

---

*This is an experimental-design memo. The hypothesis, the three conditions, the problem subset, and the derived metrics are the load-bearing decisions and should be the focus of review. The harness file layout (§5) and the per-task vocabulary hints (§4) are first-draft proposals expected to be refined during Phase 0.*
