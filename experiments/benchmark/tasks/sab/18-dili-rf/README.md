# SAB 18 — DILI Random-Forest Predictor · Task Briefing

A briefing for ScienceAgentBench instance **18**, the pilot's richest Pattern-A task
(D50 §3.1). It covers the benchmark task and what the Eigenius condition-C agent
additionally produces — modeled in the same fashion as
[SAB 16](../16-compound-filter/README.md): methodological decisions → a typed
deliverable → a decision↔code overlay with a coverage check.

| | |
|---|---|
| **instance_id** | 18 |
| **Domain** | Bioinformatics |
| **Subtask** | Feature Engineering, Machine Learning |
| **Source repo** | `anikaliu/CAMDA-DILI` |
| **Gold program** | `DILI_models_ECFP_RF.py` |
| **Output files** | `pred_results/{MCNC,MCLCNC,all}_RF.csv` |
| **Eval script** | `eval_DILI_RF.py` |
| **Module** | `mol` (on `bench-core`) |
| **Pattern / richness** | A (justified decisions) · ●●● high |

---

## 1. What the task is about

Predict **Drug-Induced Liver Injury (DILI)** risk from chemical structure. Given a
compound's SMILES, train a Random-Forest classifier (on ECFP fingerprints) to call it
a DILI concern or not. The twist that makes this the pilot's richest task: it is run
across **three dataset configurations** built from index-range splits, each with its
own **5-fold cross-validated hyperparameter search**, producing three prediction
files. The places an agent goes wrong are not the ML mechanics but the *spec
interpretation* — the 4→2 label mapping, the row-range splits, the three
config definitions.

## 2. Instruction (verbatim)

> Train a Random Forest classifier to predict whether a given chemical structure (in
> SMILES format) poses a Drug-Induced Liver Injury (DILI) concern. The dataset's
> vDILIConcern column classifies 'vMost-DILI-Concern' and 'vLess-DILI-Concern' as
> positive, and 'vNo-DILI-Concern' and 'sider_inactive' as negative. The training
> dataset is split into different sets: MC (1–173), LC (174–433), NC (434–660), and
> sider (661–923). Use these splits to create three task configurations: MCNC
> (classifying MC vs NC drugs), MCLCNC (classifying MC/LC vs NC drugs), and all
> (classifying MC/LC vs NC/sider-inactive drugs). For each task configuration, use the
> training examples to perform 5-fold cross-validation and hyperparameter search. Use
> the model trained on the best hyperparameter and save the prediction results to
> "pred_results/{data_conf}_RF.csv" where data_conf is the name of different data
> configurations (i.e., MCNC, MCLCNC, and all). Put the chemical smiles name and
> predicted label ('DILI' or 'NoDILI') in the 'standardised_smiles' and 'label'
> column respectively.

## 3. Domain knowledge (provided with the task)

- **Featurization** — fingerprints or molecular descriptors; e.g. the Morgan
  fingerprint (≡ ECFP) encodes presence/absence of chemical substructures.
- **Hyperparameter search** — a Random Forest fits many decision trees on data
  sub-samples and averages them. Tunable: number of trees, max depth, min samples to
  split a node, min samples at a leaf.

## 4. Datasets

```
dili/
├── train.csv   # labelled compounds (the 4 splits live here, by row range)
└── test.csv    # compounds to predict
```

**`train.csv`** — columns `,PubChem_CID,Compound Name,standardised_smiles,vDILIConcern,cluster`:

```
,PubChem_CID,Compound Name,standardised_smiles,vDILIConcern,cluster
0,34869,amineptine,O=C(O)CCCCCCNC1c2ccccc2CCc2ccccc21,vMost-DILI-Concern,0
1,2717,chlormezanone,CN1C(=O)CCS(=O)(=O)C1c1ccc(Cl)cc1,vMost-DILI-Concern,1
...
```

- `standardised_smiles` — the structure to featurize (input).
- `vDILIConcern` — the four-way label that the **label mapping** collapses to binary.
- The four splits (**MC / LC / NC / sider**) are **row ranges** of this file, not a
  column: MC = rows 1–173, LC = 174–433, NC = 434–660, sider = 661–923.

**`test.csv`** — same columns; the compounds whose `label` the model predicts.

## 5. What the agent has to do (condition A — the benchmark task)

1. **Featurize** `standardised_smiles` → ECFP/Morgan fingerprints.
2. **Map labels** — `vMost-DILI-Concern`, `vLess-DILI-Concern` → positive;
   `vNo-DILI-Concern`, `sider_inactive` → negative.
3. **Build the three configs** from the row ranges — `MCNC` (MC+ vs NC−), `MCLCNC`
   (MC/LC+ vs NC−), `all` (MC/LC+ vs NC/sider−).
4. **Per config**: 5-fold CV + hyperparameter search over the RF; keep the best-
   hyperparameter model.
5. **Predict** the test compounds and **write** `pred_results/{config}_RF.csv` with
   columns `standardised_smiles`, `label` ∈ {`DILI`, `NoDILI`}.

**Evaluation:** `eval_DILI_RF.py` compares the three prediction CSVs against gold.

## 6. What the condition-C (Eigenius) agent additionally does

Same model as SAB 16: the program is the **object code**; the chain is the **source**
that justifies its construction. The deliverable's correctness is won in
*spec-adherence* decisions — and SAB 18 has more of them, trickier. The validated
reference chain is in [`tracer-chain.esl`](tracer-chain.esl)
(test: `crates/eigenius-reasoning/tests/sab18_tracer.rs`, `Holds` + coverage):

| Phase | Artifact | Warrant |
|---|---|---|
| Decisions | five `Conforms…(solution)` — featurization (ECFP), **label mapping** (4→2), **configs** (the row-range splits + 3 configs), model selection (RF + 5-fold CV), output format (3 CSVs, `{DILI,NoDILI}`) | Declared (each rationale cites the spec) |
| Acceptance rule | `∀p. ConformsFeaturization(p) → … → ConformsOutputFormat(p) → ImplementsDILIPredictor(p)` | Declared |
| Conclusion | `ReasoningSentence ImplementsDILIPredictor(solution)` — 5-premise `App`-chain, `SpecStr` core | — |
| Deliverable | `bench:TaskOutput(python_source)` — the program as `payload`, `reasoning_chain` → the conclusion | Derived |
| Overlay | five `bench:CodeBlock`s linking each `# region:` to its decision; coverage check tested | — |

**Why SAB 18 is the higher-value test.** Its failure modes are exactly the decisions
the discipline forces into the open: the **label mapping** (treat `sider_inactive` as
negative, not drop it; don't miss `vLess`), the **row-range splits** (is "MC (1–173)"
1-based inclusive? — a real interpretation the `ConformsConfigs` rationale must
pin down), and the **three config definitions**. In condition A these are silent
constants; here each is an explicit warranted decision, and a *missing* one (e.g. a
config the program never builds) is caught by the coverage check.

**Honest scope (corrects an earlier framing).** SAB 18's methodological decisions are
all *spec-warranted* (`Declared`) — like SAB 16, just more and trickier. The
*contingent* part everyone pictures (which hyperparameters CV picks) is the program's
**runtime output**, not chain reasoning — the analogue of SAB 16's per-compound
keep/drop. So SAB 18 stresses **spec-adherence** reasoning hardest; it does not, by
itself, exercise data-driven inference (that's the drug-screening worked example,
which composes a *Derived* statistical result with a declared rule). The chem+bio SAB
pilot is, in this light, a spec-adherence benchmark for the discipline.

**Honest limit & metric caveat** (as SAB 16): each `Conforms…` is agent-attested; the
kernel checks the reasoning composes and the program structure corresponds (coverage),
not that a region's code is internally correct. And a well-warranted-but-different
choice can `Hold` yet miss the gold's exact-match — track `Holds` and SR separately.
