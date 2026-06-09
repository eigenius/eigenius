# D51 — Benchmark Implementation Gaps

*Status: implementation-planning memo · June 2026*

*Companion to [D50 benchmark evaluation approach](d50-benchmark-evaluation-approach.md). This memo enumerates the implementation work that must close before D50's pilot can be scheduled. Each gap is named, sized roughly, and located in the codebase. Items are ordered along the critical path: items earlier in the list block items later in the list.*

*Companion design documents the gaps consume: [D39 v2 justification logic](d39-justification-logic.md), [D49 ChainWitness machinery](d49-chainwitness-machinery.md), [D46 Prop universe + axiom framework](d46-prop-universe-and-proof-irrelevance.md), [D47 chain-mirrored EigenTT type fragment](d47-chain-mirrored-eigentt-type-fragment.md), [D48 indexed inductive families](d48-indexed-inductive-families.md), [D14 institution realisation](d14-institution-realisation.md).*

---

## 1. The critical path at a glance

Eight gaps, ordered top-to-bottom by dependency. Items 1–4 are kernel / institutional work; items 5–8 are experimental-infrastructure work. The kernel work must land before the infrastructure work can be exercised end-to-end, but the experimental infrastructure can be drafted (file layouts, scoring scripts, base-ontology authoring) in parallel with kernel work.

| # | Gap | Type | Rough effort | Blocked by |
|---|---|---|---|---|
| 1 | D49 `ChainWitness` machinery — witness table, synthesis, trace dispatch (excl. Lean) | Kernel | ~2 weeks | nothing |
| 2 | Lean → Reasoning comorphism + `VerifiedPropositionView` class (D49 §7) | Ontology + Lean worker | ~1 week | (1) |
| 3 | D39 v2 institutional artifacts — ontologies (`JustifiedBy`, `ReasoningSentence`, `Asserts(iri)`, `canonical_proposition`) + `crates/eigenius-reasoning/` (new crate parallel to `eigenius-lean`). The benchmark-scoped `TaskOutput` class lives with the harness (gap 7), not in the reasoning ontology. | Ontology + new crate | ~2 weeks | (1) |
| 4 | MCP surface extensions — `eigenius_load` ESL parameter, `eigenius_institution_dispatch` generic tool | Orchestrator | ~0.5 weeks | (3) |
| 5 | Six per-family base ontologies (`bench:chem` / `gis` / `bio` / `psych` / `mfg` / `opt`) | ESL authoring | ~3 days | (3) |
| 6 | Agent skill update for the model-then-reason discipline | Documentation + worked examples | ~1 week | (3), (4), (5) |
| 7 | Three-condition benchmark harness | Experimental infrastructure | ~2 weeks | (4), (5), (6) |
| 8 | Per-pilot-task wiring — task fetching, eval-script integration, LLM-judge pinning | Experimental infrastructure | ~1 week | (7) |

**Total**: ~10 working weeks if serialised; ~7 weeks with the parallelisable work running alongside the kernel critical path. The bottleneck is the kernel work (gaps 1–3); gaps 5–6 can start as soon as the D39 ontology surface is stable enough to author against.

The rest of this memo covers each gap in turn: what needs to be built, where it lives in the tree, what it depends on, and the design references that already specify the shape.

## 2. Gap 1 — D49 `ChainWitness` machinery (excl. Lean)

**Specified in**: D49 §3-§6 (table location, witness key, synthesis algorithm, trace-emission dispatch for the three non-Lean witness families).

**Build sites**:

- `kernel/src/layer/witness_index.rs` (new) — `WitnessKey` struct (`category` × `iri` × `prop_hash`), `BTreeMap<WitnessKey, ()>` materialised per `Layer`, `build_witness_index(&Layer)` builder, `OnceLock` for lazy construction.
- `kernel/src/layer/mod.rs` — wire `OnceLock<BTreeMap<WitnessKey, ()>>` into `Layer`; expose `lookup_chain_witness(&Layer, &WitnessKey) -> bool` walking the parent chain.
- `kernel/src/nbe/val.rs` — add `Val::ChainWitness { key: WitnessKey }` variant per D49 §8.
- `kernel/src/nbe/check.rs` — when type-checking a `JustifiedBy.declared` / `.observed` / `.derived` constructor, synthesise the witness via `lookup_chain_witness`; on miss, emit `TypeError::NoAdmittedChainWitness { … }` with the diagnostic shape D49 §5 specifies.
- `kernel/src/ontology/well_known.rs` — add the `reflection:canonical_proposition` IRI constant.
- `kernel/src/validation/rules/` — extend the validator with the per-resource `canonical_proposition` type-check at `Prop`.

**Test surface**: hand-built `Layer` carrying mock `DeclarationTrace` / `ObservationTrace` / `ProgramTrace` resources; smoke-test the synthesis algorithm catches the witness, the negative diagnostic on misses, the parent-chain walk admits an ancestor's witness for a descendant Layer.

**Not in scope for this gap**: the Lean institution's `IsVerifiedAs` path (gap 2); the `JustifiedBy` inductive's authoring as a chain artifact (gap 3, since `JustifiedBy` is itself a `data` declaration that consumes D49's machinery).

## 3. Gap 2 — Lean → Reasoning comorphism + `VerifiedPropositionView`

**Specified in**: D49 §7 (comorphism-reify pattern; no new D14 trait surface).

This gap intentionally adds *no kernel trait surface* — the cross-institution translation rides on D14's existing comorphism machinery and chain-reinsertion path (D14 §9.3 step 4). An earlier draft of D49 introduced a new `Institution::export_proposition` trait method; that shape was over-engineered and was dropped in favour of the comorphism pattern. The build sites below reflect the current design.

**Build sites**:

- `ontologies/reasoning/reasoning-ontology.json` (in the same authoring pass as gap 3) — declare the `reasoning:VerifiedPropositionView` class. `is_a [reflection:DerivedResource]`; requires `reasoning:source_verified_resource` (IRI of the user-authored `VerifiedResource`) and `reflection:canonical_proposition` (D47-encoded EigenTT `Prop` term). The view's `derivation` invariant is satisfied by the comorphism's reify trace.
- `ontologies/lean/lean-ontology.json` (or wherever existing Lean comorphisms are declared) — declare the `lean_to_reasoning` comorphism per D14 §3-§5. Source class: `lean:LeanProofTerm`. Target class: `reasoning:VerifiedPropositionView`. Transformation: a reference to the inverse-D30 transformation Component (below). Dispatch role: `AutoOnLoad` on `lean:LeanProofTerm` commits. `exact: false` — not faithful for the full Lean fragment.
- `crates/eigenius-lean-worker/src/lean_to_reasoning.rs` (new) — the comorphism's transformation implementation. Reads the chain-mirrored `lean:LeanExpr` proposition from the source `VerifiedResource`, runs the inverse of D30's forward translation on the trivially-mappable `Prop` fragment, returns the EigenTT `Exp` as the comorphism's typed payload to be reified. Propositions outside the v1 fragment (universe polymorphism, Lean-specific definitional unfolding rules not mirrored in EigenTT) cause the transformation to fail with a `Verdict::Fails` whose diagnostic names the inexpressible feature — the reify step does not commit a view, and no `IsVerifiedAs` witness becomes admissible.
- `kernel/src/layer/witness_index.rs` — the `VerificationTrace` branch of the witness emitter (gap 1) reads `canonical_proposition` from the *reified* `VerifiedPropositionView` (looked up by `source_verified_resource = trace.resource`) rather than from the user-authored VerifiedResource. **No special dispatch path** — the same code that reads the property for `IsDeclaredAs` / `IsObservedAs` / `IsDerivedAs` reads it for `IsVerifiedAs`, just from a different chain resource. This branch should land as part of gap 1's witness-emitter implementation; gap 2 makes it work end-to-end by providing the comorphism that produces the view.

**Test surface**: a hand-authored `VerifiedResource` with a small Lean proof (e.g., `2 + 2 = 4` in Nat). Confirm: (a) on commit, the Lean → Reasoning comorphism's AutoOnLoad fires and reifies a `VerifiedPropositionView` with the EigenTT-form proposition; (b) the witness `IsVerifiedAs iri (Eq Nat (2+2) 4)` is admissible at the next type-check; (c) a separate `VerifiedResource` whose proposition uses universe polymorphism fails the comorphism reify with a diagnostic, no view is committed, and the witness is correctly absent. The diagnostic surfaces both at comorphism-dispatch time (a Verdict resource) and at downstream `JustifiedBy.verified` type-check time (the witness lookup misses with a hint pointing back at the Verdict).

**Independent of gap 3** in principle (the comorphism produces the view as soon as gap 1's witness emitter is in place; the absence of `JustifiedBy.verified` consumers just means no one looks up the witness yet). Easier to land *after* gap 3 because the `JustifiedBy.verified` consumer needed for end-to-end testing exists only once gap 3 has authored the `JustifiedBy` inductive.

**Why no kernel trait extension**: the inverse-D30 transformation is a pure function over `lean:LeanExpr` returning an `Exp`. Wrapping it in the comorphism transformation pattern (where comorphisms are *declared* as ontology resources and the transformation is the source-export step) reuses D14's commit-time AutoOnLoad dispatch, its content-addressed reify, its diagnostic shape, and its query-class registration without writing any new trait or dispatch code. The Reasoning institution does not call into the Lean institution directly — it consumes a chain resource the comorphism committed.

## 4. Gap 3 — D39 v2 institutional artifacts

**Specified in**: D39 v2 §3–§5. (The `TaskOutput` class previously specified in D39 §4.4 was relocated to D50 §5b on review — it is benchmark-scoped, not Reasoning-scoped. Its build moved to gap 7, the benchmark harness.)

**Build sites**:

- `ontologies/reasoning/reasoning-ontology.json` (new) — declares:
  - `JustificationTerm` indexed inductive (6 ctors per D39 §3: 4 groundings + `App` + `Sum`). Authored using the eigenius#72 Layer 2 ESL surface (`data` with indices, typed ctors).
  - `JustifiedBy` indexed inductive over `(JustificationTerm × Prop)` with 6 ctors per D39 §5 (`declared` / `observed` / `derived` / `verified` consuming `ChainWitness` witnesses + `app` / `sum_l` / `sum_r` composition). Same surface.
  - `ReasoningSentence` Resource class. `is_a: [reflection:DerivedResource, reasoning:ReasoningSentence]` per the D39 §4.2 update. Property declarations: `proposition`, `justification`, `certificate`, `subject_iri` (with index hint), `refutes` (optional). The `derivation` invariant from `DerivedResource` is satisfied by pointing at the `certificate`.
  - (`TaskOutput` was previously listed here per D39 §4.4. It has been relocated to D50 §5b — it is benchmark-scoped, not Reasoning-scoped — and now lives with the harness in gap 7.)
  - The Reasoning institution declaration (`institution:Institution` resource) with `extract_typed` / `reify` shapes and three query class declarations (`ValidateJustification` AutoOnLoad, `EntailmentQuery` OnDemand, `ConsistencyCheck` Decidable).
- `ontologies/core/core-ontology.json` — add `Asserts(iri) : Prop` declaration (uniform-parameter no-ctor inductive in `Sort(0)`) per D39 §4.1. Also add `reflection:canonical_proposition` as an optional property on `DeclaredResource` / `ObservedResource` / `DerivedResource` (the latter two carry it as a forward-compat property even when not yet authored on most resources).
- `kernel/src/bootstrap/mod.rs` — add the reasoning ontology as a new bootstrap layer parent (after `core`, `program`, `reflection`, `institution`, and the `eigentt-type-fragment` layer). Update `embedded_ontologies` count.
- `crates/eigenius-reasoning/` (new crate, parallel to `crates/eigenius-lean/`) — the Reasoning institution's `Institution` trait implementation. Single crate (no worker / runtime sub-crates needed) because the validator IS the kernel's NbE checker and there's no external runtime. Cargo deps: `eigenius-kernel` (for the `Institution` trait + `Resource` / `Layer` / `Val` / `Exp` / NbE checker types) plus the usual workspace utilities. File layout mirrors `crates/eigenius-lean/src/`:
  - `lib.rs` — top-level exports.
  - `institution.rs` — `ReasoningInstitution` struct + `impl Institution` wiring.
  - `extract.rs` — `extract_typed`: decode `ReasoningSentence` resource → `JustifiedBy J P` typed payload via the D47 codec.
  - `reify.rs` — the inverse.
  - `validate.rs` — `query(ValidateJustification, …)` handler: thin wrapper that type-checks the certificate against `JustifiedBy justification proposition` via the kernel's NbE checker; returns Verdict. Wired through D14's existing AutoOnLoad dispatch.
  - `entailment.rs` — `query(EntailmentQuery, …)` handler: given Γ and A, bounded-depth search for a `JustificationTerm` whose certificate type-checks; returns Verdict.
  - `consistency.rs` — `query(ConsistencyCheck, …)` handler: propositional-fragment consistency over the committed-sentence set.
  - `startup.rs` — chain-scan registration hook (parallel to `eigenius-lean/src/startup.rs`).

  No `chain_mirror.rs` (parallel to `eigenius-lean/src/chain_mirror.rs`) is needed because `JustificationTerm` and `JustifiedBy` are authored via the eigenius#72 Layer 2 ESL surface and decoded by existing kernel inductive machinery. No `checker.rs` is needed because there's no external term checker to delegate to — the validation runs through `eigenius-kernel`'s NbE machinery directly.

- `kernel/src/capability/registration.rs` — register the Reasoning institution at chain-scan time using the same auto-registration shape the Lean institution already uses (D14 §3, plus the existing in-kernel registration path that handles `eigenius-lean`).

**Test surface**: hand-authored `ReasoningSentence` resources with each `JustificationTerm` shape; confirm commit-time validation fires per D39 §4.3; confirm gate firings are recorded as `Verdict` resources alongside the sentences. End-to-end: a small chain (axiom + observed measurement + derived value + reasoning sentence citing them) round-trips through commit / lookup / EntailmentQuery.

## 5. Gap 4 — MCP surface extensions

**Specified in**: the conversation thread on `orchestration/src/mcp` review.

**Build sites**:

- `orchestration/src/mcp/server.ts` — extend `eigenius_load` (around line 281) with an optional `format: "json" | "esl"` parameter; thread through to `client.load(args.json, { … format })` which passes through to the kernel's existing `content_type` handling. ~30 lines.
- `orchestration/src/mcp/server.ts` — add a new `eigenius_institution_dispatch(institution_iri, query_class_iri, payload, branch?, atLayer?)` tool under the Explore group. Calls the kernel's institution-dispatch RPC (which already backs `eigenius_query`'s `FIBER` clause; the new tool exposes the standalone-dispatch path). ~40 lines.
- `orchestration/src/client.ts` (or wherever the `client.load` signature lives) — propagate the `format` parameter through the typed RPC surface.
- `proto/eigenius.proto` if a new RPC method is needed for standalone institution dispatch (likely it isn't — the existing surface that backs FIBER should suffice; verify before adding a new proto method).

**Test surface**: a Deno orchestration test that loads an ESL file via `eigenius_load(format: "esl")` and confirms the resulting chain layer matches the equivalent JSON-loaded layer; a second test that dispatches `EntailmentQuery` on a small Reasoning-institution chain and confirms the Verdict.

**Out of scope**: per-Reasoning-institution-query convenience MCP tools (e.g., `eigenius_check_entailment`). The generic `eigenius_institution_dispatch` covers them; convenience wrappers are added later if agent ergonomics in Phase 0 show the agent struggling with the institution-IRI / query-class-IRI parameters.

## 6. Gap 5 — Six per-family base ontologies

**Specified in**: D50 §4 (the table of six bases with anchor classes).

**Build sites**:

- `experiments/benchmark/base-ontologies/chem.esl` (or equivalent location in the experiments tree — separate from production ontologies):
  - `class bench:Compound { requires bench:smiles; }`
  - `class bench:Reaction { requires bench:reactants, bench:products; }`
  - `class bench:Measurement { requires bench:value, bench:unit; }`
  - `class bench:Predicted` / `bench:Observed` distinction (subclasses of relevant reflection categories).
  - …
- `experiments/benchmark/base-ontologies/gis.esl` — `SpatialFeature`, `RasterLayer`, `CRS`, `Buffer`, `Polygon`, `TemperatureSeries`, `Glacier`.
- `experiments/benchmark/base-ontologies/bio.esl` — `Cell`, `Gene`, `Expression`, `Protein`, `MLClassifier`, `FeatureSet`, `Sample`.
- `experiments/benchmark/base-ontologies/psych.esl` — `Signal`, `ECGRecord`, `HRVIndex`, `Subject`, `QuestionnaireResponse`, `ValidatedScore`.
- `experiments/benchmark/base-ontologies/mfg.esl` — `Component`, `Process`, `Decision`, `Cost`, `ConfidenceLevel`, `HypothesisTest`, `InspectionPolicy`, `DefectRate`.
- `experiments/benchmark/base-ontologies/opt.esl` — `Variable`, `Constraint`, `Objective`, `FeasibleRegion`, `Solution`, `ProbabilityModel`, `OptimizationProblem`.

Each base is 5-10 ESL declarations. Total authoring effort ~3 days; can be reduced by mining existing demos (the patent demo's ontology is a useful reference shape) and the existing per-domain institution catalogues (D27 Julia institutions for the formula side of `mfg` / `opt`).

**Quality check**: each base ontology must round-trip through the kernel's commit pipeline cleanly (no validator failures); each base must be loadable as a layer parent in the benchmark harness without conflicting with the bootstrap layers; each base's classes must support being subclassed by per-task agent vocabulary (test: hand-author a per-task vocabulary for one pilot task per family and confirm it commits cleanly on top of the base).

**Independent of kernel gaps 1-3** in principle (ESL authoring is supported today), but the agent's per-task vocabulary will need the eigenius#72 surface to author `axiom` declarations naturally — and the Reasoning institution's `ReasoningSentence` shape must exist before the agent can cite axioms in reasoning sentences. So gap 5 is *authoring-ready* now but *useful* only after gap 3.

## 7. Gap 6 — Agent skill update for the model-then-reason discipline

**Specified in**: D39 §4.5 (two-phase agent surface), §6.4 (trade-off pattern), the agent-skill summary in the conversation thread on MCP review.

**Build sites**:

- `.claude/skills/eigenius.md` — extend the existing skill with:
  - **Section: "Reasoning loop overview"** — the two-phase discipline (vocabulary, then reasoning), why it matters, when it engages.
  - **Section: "Authoring vocabulary"** — patterns for `class` / `property` / `axiom` / indexed `data` declarations in ESL; common shapes per domain (chemistry, GIS, manufacturing) with worked examples; how to recover from validator failures on vocabulary commits (most common: malformed `requires` lists, mis-typed axiom statements).
  - **Section: "Authoring `ReasoningSentence`s"** — the canonical Eigon-JSON / ESL shape; how to construct `JustificationTerm`s for each of the four grounding patterns from D39 §6; the trade-off pattern from D39 §6.4 with a worked example.
  - **Section: "Querying past reasoning"** — three canonical EigenQL templates: "my conclusions about subject X", "what does sentence Y depend on", "what axioms / inference rules are in scope for predicate P". Each template is copy-pasteable.
  - **Section: "When to use `eigenius_institution_dispatch`"** — operational guidance for `EntailmentQuery` (before committing a derivative conclusion, check whether the chain already entails it) and `ConsistencyCheck` (before committing a contradicting sentence). Concrete examples.
  - **Section: "Recovery from commit failures"** — the kernel's diagnostic shape for the common failure modes (missing prior, ungrounded justification, ill-typed proposition, vocabulary error) and the canonical revise-and-retry pattern for each.
  - **Section: "Common anti-patterns"** — vacuous justifications (`DeclaredEvidence` citing the agent's own assertion as the only ground), predicate-name proliferation (10 ad-hoc predicates where 3 would do), trying to reason in untyped prose first and lift to ESL later.

- `docs/guides/platform/14-notebook.md` or a new chapter — a worked end-to-end example: one pilot task taken through the model-then-reason discipline, with the chain artifacts at each step shown. This doubles as the publication's introductory worked example and as the skill's reference example.

**Test surface**: the Phase 0 shakedown (D50 §7) is the test for this gap. If the three Phase 0 tasks run smoothly with the updated skill, the discipline is teachable; if the agent fights the surface or produces vacuous chains, the skill needs more work.

## 8. Gap 7 — Three-condition benchmark harness

**Specified in**: D50 §5 (harness architecture sketch).

**Build sites**:

- `experiments/benchmark-harness/` (new tree, separate from production code):
  - `harness-ontology.esl` — declares the benchmark-scoped `benchmark:TaskOutput` class + its properties (`task`, `deliverable_kind`, `payload`, `reasoning_chain`, inherited `derivation`) per D50 §5b. Loaded as a sibling layer to the per-family base ontologies; the Reasoning institution stays unaware of it.
  - `conditions/baseline_runner.py` — wraps SAB's native agent (`agent.py` in `references/ScienceAgentBench/ScienceAgentBench_github/`) for SAB tasks; wraps a direct prompt-the-LLM path for EngiBench. Produces the deliverable in the format the benchmark's eval script expects.
  - `conditions/cot_runner.py` — same agent / direct path, but with a chain-of-thought instruction added to the system prompt. Records the agent's reasoning trace in a separate field alongside the deliverable.
  - `conditions/eigenius_runner.py` — drives the Eigenius MCP surface for the structured-reasoning condition. Loads the per-family base ontology as a layer parent, loads per-task vocabulary hints into the agent's context, runs the agent loop with access to the MCP tools (`eigenius_load`, `eigenius_query`, `eigenius_inspect`, `eigenius_institution_dispatch`), extracts the final `benchmark:TaskOutput.payload` as the deliverable.
  - `tasks/sab/<task-id>/{task.json, hints.esl}` — per-task config: task instruction, dataset path, eval script reference, vocabulary hints.
  - `tasks/engibench/<task-id>/{task.json, hints.esl}` — same shape for EngiBench.
  - `scoring/sab_score.py` — wraps the SAB per-task eval scripts (which live under `references/ScienceAgentBench/ScienceAgentBench_github/evaluation/`); produces VER / SR / CBS for each (condition × task × replicate) triple.
  - `scoring/engibench_score.py` — LLM-judge rubric scoring with a pinned judge model and version; produces per-axis scores plus an aggregate per (condition × task × replicate).
  - `scoring/derived_metrics.py` — gate-firing tally, vocabulary size, reasoning chain depth, citation density, trade-off pattern usage (per D50 §6.3).
  - `runs/<run-id>/<condition>/<task>/<replicate>/` — per-cell run artifacts: the agent's transcript, the deliverable, the scoring output, the timing data, the Eigenius chain artifacts (for condition C).
  - `analyze/headline.py` — produces the per-benchmark per-condition table; runs significance tests; emits the publication-ready figures.

**Effort estimate**: 2 weeks if Python-side, including the SAB / EngiBench native-runner integration work. Could be shorter if existing benchmark agent runners can be wrapped without modification.

**Operational considerations**:

- Run conditions one at a time per task (not parallelised across conditions for one task), so the conditions don't compete for LLM API rate-limit budget.
- Run different tasks in parallel only if the LLM API rate limit allows; otherwise serialise.
- Hard per-task per-condition timeout: 30 minutes. Tasks that time out are reported separately, not treated as failures.
- All runs use the same LLM model / version, pinned in the harness config. Mid-pilot model upgrades are not allowed.

## 9. Gap 8 — Per-pilot-task wiring

**Build sites**:

- For each of the 15 SAB tasks: confirm the eval script in `references/ScienceAgentBench/ScienceAgentBench_github/evaluation/` runs cleanly on the gold program; package the dataset for the harness; author the per-task vocabulary hints file (~5 suggested predicate names).
- For each of the 11 EngiBench tasks: package the problem statement; author the per-task vocabulary hints file; confirm the LLM judge produces stable scores on the gold-style human-written reference response (which CUMCM problems often have published).
- Per-pilot LLM-judge calibration for EngiBench: 2 of the 11 problems scored with a second LLM judge (different model / family) to establish inter-judge agreement before treating the primary judge's scores as authoritative.
- One pilot dry-run: the Phase 0 shakedown (D50 §7) covers the operational test of this wiring.

**Effort estimate**: ~1 week for the 26 tasks, including the LLM-judge calibration.

## 10. Sequencing recommendation

If kernel work and infrastructure work can run in parallel (different people, or one person multi-tasking), the suggested sequence is:

**Week 1-2** (kernel): Gap 1 (D49 machinery).
**Week 1-2** (parallel): Gap 5 (base-ontology authoring — uses only existing ESL surface). Gap 6 (agent skill — drafts the discipline patterns without depending on the kernel being live; final examples blocked by gap 3).

**Week 3** (ontology + Lean worker): Gap 2 (Lean → Reasoning comorphism + `VerifiedPropositionView`). No kernel changes — rides on D14's existing comorphism dispatch.
**Week 3** (parallel): Gap 7 (harness architecture — scaffolding without the Eigenius condition working).

**Week 4-5** (ontology + new crate): Gap 3 (D39 v2 artifacts) — `ontologies/reasoning/reasoning-ontology.json` declares the inductives + Reasoning institution + comorphism; `crates/eigenius-reasoning/` (new, parallel to `crates/eigenius-lean/`) implements the `Institution` trait. Largest gap of the eight; the Reasoning institution registration + the `JustifiedBy` inductive authoring are the load-bearing pieces.

**Week 5** (orchestrator): Gap 4 (MCP extensions).

**Week 6** (infrastructure): finalise Gap 6 (skill examples, now possible against a live D39 surface), finalise Gap 7 (Eigenius condition runner), Gap 8 (per-task wiring).

**Week 7**: Phase 0 shakedown per D50 §7 (3 tasks).

**Week 8+**: Phase 1 full pilot per D50 §7 (26 tasks × 3 conditions × 3 replicates = 234 runs). Wall-clock ~40 hours of agent time; calendar time depends on LLM rate limits.

Total calendar time to a publishable pilot result: roughly two months from kernel-work start, assuming parallel infrastructure work.

## 11. Risks specific to the implementation work

These are the implementation-side risks. Architectural-soundness risks are in D49 §9 / D39 §10. Experimental-design risks are in D50 §8.

**D39's first-wave UX may need iteration before the agent loop works.** The agent's experience of kernel diagnostics determines whether the discipline is teachable or feels like fighting the system. The Phase 0 shakedown is supposed to surface this, but if the first-wave diagnostics are too cryptic for an LLM agent to act on, expect to spend a week on diagnostic-quality iteration before Phase 1. **Mitigation**: budget an explicit "diagnostic-quality iteration" buffer week between gap 3 landing and Phase 0 starting.

**The Lean → Reasoning comorphism transformation may be narrower than expected.** Gap 2 commits to the trivially-mappable `Prop` fragment of Lean. If the demo theorems (or future use cases) need universe polymorphism or Lean-specific definitional unfolding, gap 2's v1 transformation is insufficient and a v2 with broader coverage is needed. The v2 path is purely additive to the comorphism's transformation implementation — no architectural reshape required, since the comorphism is the right shape; just more cases handled in the transformation. **Mitigation**: pick the demo Lean theorems early (parallel to gap 2 implementation) and sanity-check they fall in the v1-mappable fragment before committing the time.

**The agent may need more than the canonical EigenQL templates for self-recall.** Phase 0 will reveal whether the three templates in the skill (gap 6) are enough or whether more sophisticated queries are needed. **Mitigation**: track which queries the agent reaches for during Phase 0 and add them to the canonical-template list before Phase 1.

**Base ontology drift between authoring and pilot use.** Gap 5 authors the six base ontologies up front; gap 3's eventual D39 surface may impose constraints (e.g., the `canonical_proposition` property shape, the exact `Asserts(iri)` declaration shape) that require revising the base ontologies. **Mitigation**: re-validate the base ontologies against the D39 surface as the final step of gap 3; treat any base-ontology revisions as part of gap 3's effort, not gap 5's re-work.

**MCP surface ergonomics with the generic dispatch tool.** Gap 4 chooses generic `eigenius_institution_dispatch` over per-query-class convenience tools. If the agent struggles to remember institution IRIs and query class IRIs, convenience wrappers may be needed. **Mitigation**: include "agent successfully invokes EntailmentQuery via the generic tool in ≥80% of Phase 0 attempts" as a Phase 0 success criterion; add convenience wrappers if the criterion fails.

**Wall-clock and token-cost overhead in condition C.** The discipline adds friction. If condition C wall-clock blows past 30 min per task on most pilot tasks, the agent is fighting the surface rather than using it productively. **Mitigation**: report per-task wall-clock as part of Phase 0 and treat outliers as failure modes to debug before Phase 1.

## 12. What's *not* in scope for this gap inventory

- **Soundness-tally measurement infrastructure.** D50 §1 reframes this as a secondary finding rather than the headline. Gap 7's `derived_metrics.py` covers what's actually needed (gate-firing tally is a per-run statistic, not a comparison-harness output).
- **Wider domain institutional coverage.** Geopandas / DeepChem / BioPsyKit institutions don't exist in Eigenius today and are not built for this pilot. The pilot works around this by treating each tool invocation as a typed Component the agent declares (input type, output type) and the kernel checks the boundary, not the internal computation. The Python-bridge typed-Component shape may need a small extension to D14 / D26 to be authored cleanly; this is in scope for gap 5 (the base ontologies define what typed-Component shapes are available for each domain family) but the production-quality Python bridge is *not* on the critical path.
- **The four-gate concrete demo** (drug-candidate, dock_to_assay, Lean verdict). Worked example for the publication's introduction, not benchmark infrastructure. Authored in parallel; not blocking the pilot.
- **EigenQL surface for `subject_iri` indexing.** D39 §4.2 declares `subject_iri` as a first-class query index; D23's per-class triple index should auto-cover it, but if Phase 0 shows the query is slow, kernel-side index hints may be needed. Treated as a follow-up rather than a critical-path item.

---

*This is an implementation-planning memo. The eight gaps, their dependencies, and the sequencing recommendation are the load-bearing decisions; the per-gap effort estimates are first-draft proposals expected to slip as the work progresses. The risks in §11 should be re-reviewed before each gap is started.*
