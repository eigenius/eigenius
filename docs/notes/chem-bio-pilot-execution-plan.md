# Chem + Bio Pilot — Execution Plan (ad-hoc tracker)

> **⚠️ Temporary working document. Delete once the chem+bio pilot infrastructure is implemented.**
> This is an execution checklist for the infra-first approach to D50/D51's chem+bio
> pilot. It is not a design memo — the design lives in
> [D50](../design/d50-benchmark-evaluation-approach.md) and
> [D51](../design/d51-benchmark-implementation-gaps.md). When the steps below are
> done, fold any durable learnings back into D50/D51 and remove this file.

*Created 2026-06-11.*

## Strategy

Build the experimental infrastructure (D51 gaps 4–8) for the chem+bio pilot
first; address kernel gaps as we run into them. This is safe because the kernel
critical path is already closed:

- **Gap 1 (ChainWitness machinery)** — done (`#76`).
- **Gap 3 (D39 v2 Reasoning institution)** — done (`#76`). `ValidateJustification`
  is load-bearing.
- **Gap 2 (Lean → Reasoning comorphism)** — partial, but **off the chem+bio
  critical path**: no chem/bio task needs a `JustifiedBy.verified` warrant. Defer
  it; pick it up only for the four-gate concrete demo.

So "kernel gaps as we run into them" will, in practice, be three small surfaces,
not big builds:

1. **Diagnostic quality** (D51 §11 risk #1) — can an LLM agent act on the
   kernel's rejection messages? The biggest unknown; the tracer step below
   front-loads its discovery.
2. **`EntailmentQuery` / `ConsistencyCheck` stubs** (D51 §4 caveat) — v1 is
   lookup-only / stub. Build out the bounded-depth search in `eigenius-reasoning`
   only if a task actually leans on it.
3. **ESL-surface papercuts** — real chem/bio vocab may expose small gaps in the
   `data` / `axiom` / `requires` surface. Fix as they appear.

## Ordered steps

Each step lists its done-criterion. Steps 1–2 are dependency-free against the
already-built kernel; step 3 is the cheap shakedown that de-risks the rest.

- [ ] **1. Base ontologies (D51 gap 5) — `bench-core` + `mol` first, then `materials` + `singlecell`.**
      Author the shared spine + data-shape modules under
      `experiments/benchmark/base-ontologies/` (see "Module shape" and the per-task
      sketches below). Zero kernel dependency — it's ESL, like `reasoning.esl`
      already is. Start here because it immediately tests whether the D39 base-class
      patterns hold for real domain vocab, and resolves the "base-ontology drift"
      risk (D51 §11) before anything depends on it.
      *Done when:* `bench-core` + `mol` round-trip through the commit pipeline
      cleanly (compile against the bootstrap chain, no validator failures), and the
      SAB 16 per-task vocabulary subclasses them cleanly (step 3).

- [ ] **2. MCP surface (D51 gap 4).** Add `format: "json" | "esl"` to
      `eigenius_load` and the generic `eigenius_institution_dispatch` tool in
      `orchestration/src/mcp/server.ts`; propagate `format` through the client.
      Needed before any agent automation.
      *Done when:* a Deno test loads an `.esl` file via `eigenius_load(format:"esl")`
      and confirms the resulting layer matches the JSON-loaded equivalent; a second
      test dispatches `EntailmentQuery` on a small Reasoning chain and gets a Verdict.

- [ ] **3. Hand-driven tracer — SAB 16, condition C, no harness.** Acting as the
      condition-C agent, take ONE chem task (SAB 16, the shortest) through the
      model-then-reason discipline over the MCP surface by hand. No Python harness
      yet. This is the real shakedown — it surfaces kernel-diagnostic friction
      (watch-list item 1) cheaply, before ~1.5 weeks of harness work, and it
      produces the worked example for the skill (step 4).
      *Done when:* SAB 16's deliverable is produced via a committed chain of
      `ReasoningSentence`s + a `benchmark:TaskOutput`, and we've logged every kernel
      diagnostic we hit and whether it was actionable.

- [ ] **4. Agent skill (D51 gap 6).** Write the reasoning-discipline sections of
      `.claude/skills/eigenius.md` from what the tracer taught (so the worked
      example is the real SAB 16 chain, not invented). Reconcile the skill's tool
      list with the post-gap-4 MCP surface.
      *Done when:* the skill covers the two-phase discipline, authoring
      `ReasoningSentence`s, querying past reasoning, `eigenius_institution_dispatch`
      usage, and commit-failure recovery — with the SAB 16 walkthrough as the
      reference example.

- [ ] **5. Harness (D51 gap 7) — ScienceAgentBench only.** Declare
      `benchmark:TaskOutput` in `harness-ontology.esl`, then build the three
      condition runners + `sab_score.py` + `derived_metrics.py` under
      `experiments/benchmark-harness/`. No EngiBench path (scale-up only).
      *Done when:* the three conditions run SAB 16 end-to-end unattended and
      `sab_score.py` emits VER/SR/CBS without manual intervention.

- [ ] **6. Per-task wiring (D51 gap 8) — 8 SAB tasks.** Wire the remaining 7 tasks
      (chem 17/28/94, bio 8/18/69/98): confirm each eval script runs on the gold
      program, package the dataset, author the `hints.esl`.
      *Done when:* all 8 tasks run through all three conditions and score cleanly.

- [ ] **7. Phase 0 + Phase 1.** Phase 0 shakedown on 3 tasks (SAB 16/17/18), then
      the full chem+bio pilot (8 × 3 × 3 = 72 runs). Per D50 §7.

## Module shape (base ontologies)

Modules cut by **data shape**, not SAB domain label (bio tasks 8/18 are molecule-centric, so a chem/bio cut would duplicate `Compound`). All extend a shared `bench-core` spine.

| Module (extends) | Nouns | Tasks |
|---|---|---|
| `bench-core` (→ reflection) | `ToolArtifact` (typed tool boundary), `Measurement` (value+unit), `Dataset`, `concerns` (linking predicate) | all |
| `mol` (→ bench-core) | `Compound`/`smiles`, `Fingerprint`, `ActivityMeasurement`, `Target` | 16, 17, 94, 8, 18 |
| `materials` (→ bench-core) | `CrystalStructure`, density artifacts (as `ToolArtifact`s) | 28 |
| `singlecell` (→ bench-core) | `Cell`, `Gene`, `ExpressionMatrix`, `CellType`, `ChainPairing` | 69, 98 |
| `ml` facet (→ bench-core) | `FeatureSet`, `Classifier`, `CVScore`, `Prediction` (may fold into `mol`) | 8, 18 |

## Per-task approach sketches

Richness: ●●● high · ●● moderate · ● thin. Two warrant patterns: **A** = justified decision (predicate + declared rule + derived tool-evidence + reasoning sentence, like `stats-and-reasoning.json` with `ToolArtifact`/`DerivedEvidence` standing in for the statistics institution); **B** = provenance pipeline (chain of derived `ToolArtifact`s → plot; reasoning sentences are lineage + declared parameter choices).

**Headline:** richness splits ~half/half. Pattern A (decision-rich, where the C-vs-B signal lives): 16, 8, 18, 98. Pattern B (thin/overhead probes): 17, 28, 69, 94. SAB 18 is the richest showcase; SAB 94 the thinnest.

### Pattern A — justified decision

- **SAB 16 — compound filter** · mol · ●● · *deliverable:* SMILES list (txt).
  Predicates `AlertFree(c)`, `Dissimilar(c)`, `PassesFilter(c)`. Rule `∀c. AlertFree(c) → Dissimilar(c) → PassesFilter(c)`. Tools→witness: RDKit FilterCatalog → `IsDerivedAs(·, AlertFree(c))`; Tanimoto-vs-actives → `IsDerivedAs(·, Dissimilar(c))`. Chain: per kept compound `PassesFilter(c)` = `App(App(SpecStr(rule,c), derived-alert), derived-sim)` → `TaskOutput` payload = passing SMILES. **This is the tracer (step 3).**
- **SAB 8 — backward feature selection** · mol+ml · ●● · *deliverable:* accuracy-vs-k plot.
  The plot *shows* the decision; the chain *states* it. Predicates `AccuracyAt(k,v)`, `OptimalSubsetSize(k)`, `Selected(feature)`. Rule (declared stopping criterion): `∀k. peak/plateau of AccuracyAt → OptimalSubsetSize(k)`. Tools→witness: sklearn SFS loop → `IsDerivedAs(·, AccuracyAt(k,v))`, `OptimalSubsetSize(k*)`.
- **SAB 18 — DILI Random Forest** · mol+ml · ●●● · *deliverable:* 3 prediction CSVs · **best showcase.**
  Three decisions the baseline buries in code: label-mapping (vMost/vLess→DILI; vNo/sider→NoDILI), config slicing (MCNC/MCLCNC/all), hyperparameters via 5-fold CV. Predicates `Predicted(c,label)`, `CVScore(config,θ,v)`, `BestHyperparams(config,θ)`. Rules: declared label-mapping; `∀θ. max CVScore(config,θ) → BestHyperparams(config,θ)`. Tools→witness: ECFP featurize → derived; RF CV/grid → `CVScore`, `BestHyperparams`; RF predict → `Predicted(c,label)`. Chain → `TaskOutput` = 3 CSVs.
- **SAB 98 — TCR chain QC** · singlecell · ●● · *deliverable:* stacked bar chart.
  Predicates `ChainPairing(cell, category)` for category ∈ {orphan, pair, extra, multichain}. Rules: the QC category definitions as declared resources. Tools→witness: scirpy `chain_qc` → `IsDerivedAs(·, ChainPairing(cell,cat))` → aggregate → bar chart.

### Pattern B — provenance pipeline

- **SAB 17 — chemical-space viz** · mol · ● — fingerprint→embed(t-SNE)→scatter; declared: projection method, colour scale.
- **SAB 28 — charge-density difference** · materials · ● — pymatgen reads 3 CHGCARs→difference field→planar average→plot; declared: `ρ_AB−ρ_A−ρ_B`, z-axis.
- **SAB 69 — scanpy UMAP** · singlecell · ● — filter_genes→PCA(30)→UMAP→plot; declared: gene-filter threshold (the one real decision).
- **SAB 94 — molecule graph** · mol · ● — RDKit Mol→networkx→draw; colours/seed/node-size all given by the task → essentially one tool call, thinnest chain.

For Pattern B the chain is mostly a `bench:ToolArtifact` provenance spine (`produced_from` edges); condition C's value is auditable lineage, not justified inference. Recomputation depth: tool outputs are `ToolArtifact`-backed `DerivedResource`s (a real `IsDerivedAs` warrant via ProgramTrace) — the statistics institution is **not** wired in for the pilot (the SAB tasks are viz/ML, not hypothesis tests it covers). Revisit only if a specific task's decision is a test an existing institution recomputes.

## Off critical path (do whenever convenient)

- [ ] **Gap 2 — Lean → Reasoning comorphism.** `lean_to_reasoning` comorphism
      resource + transform in `crates/eigenius-lean-worker/` + the `VerificationTrace`
      emit branch in `witness_index.rs` + the 2+2=4 round-trip test. Needed only for
      the four-gate concrete demo, never for the chem+bio pilot.

## Tracer findings — SAB 16 (2026-06-12)

Hand-driven condition-C chain at `experiments/benchmark/tasks/sab/16-compound-filter/tracer-chain.esl`; validates to **Holds** (`crates/eigenius-reasoning/tests/sab16_tracer.rs`).

**No kernel gaps.** The machinery handled everything first try: bench-core + mol compiled, the per-task vocab layered on top, the witness index admitted 1 `IsDeclaredAs` (rule) + 2 `IsDerivedAs` (tool artifacts) from hand-authored `ProgramTrace`s, and `spec_str` applied the universal rule per-compound. The typed-tool-boundary maps cleanly onto `ProgramTrace` + `canonical_proposition` — no new kernel surface.

**Modelling findings (per-task / ergonomics, not kernel):**

1. **The Derived warrant is agent-attested, not kernel-recomputed.** The agent hand-writes `canonical_proposition = AlertFree(C)` on the RDKit `ToolArtifact`; the kernel admits `IsDerivedAs` from the trace but never checks the RDKit output actually says that. For SAB tasks "Derived" = *the agent ran a tool and asserts it established P* (the D50 §9 boundary), weaker than the statistics institution's recompute. This is the central honest caveat of condition C for SAB and must be stated in the writeup.
2. **The reasoning rides on `bench-core` + the per-task predicates, not the `mol` nouns.** `mol:Compound` is committed as the observed anchor but the certificate references the compound only by IRI string; `mol:Fingerprint` went unused (the fingerprint is internal to the RDKit `ToolArtifact`). The base's value is the spine; the family nouns are provenance scaffolding. Keep `mol` thin.
3. **`bench:ToolArtifact.tool` / `produced_from` are human-audit fields, inert to the kernel** (the witness emitter only reads `canonical_proposition` via the trace). `bench:tool` duplicates `ProgramTrace.source`. ToolArtifact earns its place as a *convention*, but it is close to a documented `DerivedResource`. Revisit whether to require the trace and derive `tool` from `source`.
4. **Per-compound chain explosion.** One kept compound = 5 resources (2 ToolArtifacts + 2 traces + 1 sentence) + the shared rule. hits.csv has hundreds. Per-task design choice: fine-grained `PassesFilter(c)` per compound (audit-rich, linear cost) vs. one coarse sentence about the filtered set. Decide per task in the harness; default fine-grained for the audit story.
5. **Exclusions want positive failure predicates.** The chain justifies *kept* compounds (`PassesFilter`). Justifying *discards* needs `HasAlert(d)` / `NearActive(d)` rather than `¬AlertFree` (Prop negation is `→ False`, awkward). Deliverable only needs kept SMILES, so fine here; note for tasks where the decision is the exclusion.
6. **`bench:concerns` unused** — SAB 16's predicates name the compound IRI directly, so no identity bridge was needed. Confirms `concerns` is forward-looking (the `sample_for` case), correctly optional.
7. **Certificate verbosity is the agent-ergonomics risk (D51 §11).** ~30 lines of nested `app`/`spec_str` for a 2-premise modus ponens, even with `alias`. The gap-6 skill needs copy-pasteable per-pattern templates; the SAB-16 chain is now the reference template for Pattern A.

**8. The benchmark program is object code; the chain is the source. (Supersedes an earlier, wrong version of this finding.)** The benchmark's Python *is* reasoning — mechanized and flattened. It keeps the operations and drops everything that warranted them: why Morgan r=2/2048, that hits.csv is Observed vs. a chosen parameter vs. a Derived value, the typed propositions each step establishes, the spec interpretation (max over **all** actives, strict `<`), and any re-checkable/refutable structure. The program fuses *decision* and *execution*; we separate them across the four warrants (Observed inputs, Declared methodological choices, Derived results). "Most of the pieces the Python leaves out" = exactly that warrant structure. This is the "compiler for AI thought" framing made literal: **chain = source, Python = target.**

   Correcting the earlier mis-framing: SAB 16's real difficulty *is* reasoning — but **methodological/spec-adherence reasoning** (featurization, catalog set, similarity reduction, threshold semantics, composition), not the trivial per-compound rule I first modeled. Those decisions determine correctness and are where agents fail. The discipline engages them via the forcing function (D50 §1, *"independent of what the kernel catches"*): authoring each choice as a Declared, warranted decision makes the agent commit to and defend it against the spec, and makes a wrong choice a *visible unwarranted decision* rather than a buried constant. So SAB 16 **does** test discipline-benefit — on the spec-adherence axis (faithful spec→method translation). (SAB 18 turned out to be the *same* axis, just richer — see finding #10; my earlier "different axis / contingent decisions" claim was wrong.)

   Corrected model: the chain justifies the **program's construction** — one `ReasoningSentence` per methodological decision, each `Declared`-warranted by citing the instruction/domain-knowledge; the program is the `TaskOutput` (`python_source`) whose `reasoning_chain` points at them; per-compound results are the program's *output*, not individually reasoned. This is O(1) in compounds (dissolves the earlier "per-compound explosion" + "trivial chain" findings). **The existing `tracer-chain.esl` modeled the wrong layer (per-compound conclusions) and must be re-authored at this granularity.**

   Honest boundary (moves, doesn't vanish): the pilot gets the *forcing* + *auditability* benefit; it does **not** verify the emitted Python actually implements the declared decisions (payload is opaque text). Closing that needs **program-generated-from-chain** (consistency by construction) or program-vs-decisions checking — the north star, beyond the pilot. (A recomputing RDKit institution, like the statistics institution, is the other escalation — D50 §9/§12, also out of pilot scope.)

   Metric caveat: exact-match SR can diverge from a sound chain — a well-warranted radius=3 choice validates to `Holds` yet fails the gold's undocumented radius=2. Track `Holds` and SR separately; the gap isolates "reasoned soundly but didn't reproduce an undocumented convention" and is itself a result.

**9. Decision ↔ code-section overlay + coverage check (SAB 16 v3).** Added `bench:CodeBlock` (harness-ontology) — an overlay linking each methodological decision to the named program region that realises it (`of_output`, `block_label`, `realizes[]`). The program payload carries matching `# region: <label>` markers, so the mapping is navigable in the artifact and queryable on chain. This operationalises the "catch omitted/miscomposed steps" property (finding #8) at the *program* level via a **coverage check** (now tested in `sab16_tracer.rs`): every block's label has a region marker (no dangling overlay) and `realizes` across blocks = exactly the five decisions (every decision implemented; none unwarranted). One region may realise several decisions (the `filter` region carries reduction + threshold + composition). Honest residue persists *within* a block (the kernel doesn't check the region's code does what the decision says — only that the structure corresponds); shrinking that is the program-from-chain endpoint (blocks generated from decisions). This is "chain = source, Python = object code" with a typed cross-reference between them.

**10. SAB 18 (DILI RF) — the pattern generalises, and a meta-finding about the pilot.** Built in the same v3 fashion (briefing + tracer + 5 Declared conformances + acceptance rule + `ImplementsDILIPredictor` conclusion + `TaskOutput` + 5-block overlay + coverage); validates to `Holds` + coverage first try (`sab18_tracer.rs`). Confirms the model scales (5-premise certificate, more failure-prone decisions: the 4→2 label mapping, the row-range splits, the 3 configs). Two findings:
   - **The SAB chem+bio pilot is a *spec-adherence* benchmark, not data-driven inference.** SAB 18's methodological decisions are *all* spec-warranted (`Declared`) — same kind as SAB 16, just more and trickier. The "contingent" hyperparameters CV picks are the program's **runtime output** (analogue of SAB 16's per-compound keep/drop), not chain reasoning. So neither tracer exercises the *Derived*-evidence / data→decision composition path; that is the drug-screening worked example's job (a Derived statistical result composed with a declared rule). The discipline's demonstrated value on this pilot is **forcing faithful, auditable spec→program translation + structural coverage**, which is narrower than D50 §1's "improves multi-step scientific reasoning." **Reflect this in the writeup:** the pilot is the spec-adherence evidence; the worked example carries the inferential-discipline claim. (Candidate D50/D51 framing edit.)
   - **Family nouns stay unused, again.** SAB 18's chain references no `mol`/`ml` nouns — it's per-task `Conforms…` predicates on `bench-core` + harness. Confirms finding #2 and that the `ml` facet is not needed for the methodology model (it'd only matter if we modeled `Classifier`/`CVScore` as resources, which the spec-adherence model doesn't).

**Not yet exercised:** Fails-path diagnostic quality (every chain passed first try). Worth a deliberate break (wrong proposition / missing witness / drop a CodeBlock to trip coverage) to read the diagnostics — informs gap 6 + the §11 "is the discipline teachable" risk.

## Kernel-gaps log (fill in as we hit them)

Record here anything we run into so the eventual fold-back into D51 is accurate:

- _(none yet — SAB 16 hit no kernel gaps; see tracer findings above)_
