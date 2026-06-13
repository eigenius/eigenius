# SAB 16 — Compound Filter · Task Briefing

A briefing for ScienceAgentBench instance **16**, the pilot's Phase-0 tracer task
(D50 §3.1, §7). It covers the underlying benchmark task and what an agent must do
to solve it — first as the benchmark asks (condition A), then what the Eigenius
condition-C agent additionally produces.

| | |
|---|---|
| **instance_id** | 16 |
| **Domain** | Computational Chemistry |
| **Subtask** | Computational Analysis |
| **Source repo** | `felixjwong/antibioticsai` (`notebooks/Filtering hits and computing rationale scaffolds.ipynb`) |
| **Gold program** | `compound_filter.py` |
| **Output file** | `pred_results/compound_filter_results.txt` |
| **Eval script** | `antibioticsai_filter_eval.py` |
| **Module** | `mol` (on `bench-core`) |
| **Pattern / richness** | A (justified decision) · ●● moderate |

---

## 1. What the task is about

This is **early-stage hit triage** in antibiotic drug discovery. A screening
campaign has produced a list of candidate "hit" compounds; before anyone invests
in them, the obvious junk and the structurally redundant ones are filtered out.
The task automates that filter with two independent gates:

1. **Structural-alert clean** — no PAINS or Brenk substructure (likely-artifact /
   problematic chemistry is removed).
2. **Structurally novel** — not too similar to any compound already known to be
   active (keep new chemotypes, not near-analogs of the training actives).

A compound is kept only if it passes **both** gates.

## 2. Instruction (verbatim)

> Filter the compounds in "hits.csv" and save the SMILES representations of the
> left ones. Compounds to be kept should have no PAINS or Brenk filter
> substructures and have a maximum tanimoto similarity of less than 0.5 to any of
> the active compounds in "train.csv". Save the SMILES of left compounds to
> "pred_results/compound_filter_results.txt", with each one in a line.

## 3. Domain knowledge (provided with the task)

- **PAINS filter** — Pan-Assay INterference compoundS. Substructures that often
  produce false positives in high-throughput screening by mechanisms unrelated to
  the intended target. Implemented with RDKit `FilterCatalog` + `AddCatalog`.
- **Brenk filter** — structural rules flagging potential toxicity, instability, or
  reactivity; used in early drug discovery to improve hit drug-likeness. Also via
  RDKit `FilterCatalog` + `AddCatalog`.
- **Tanimoto similarity** — similarity between two molecules represented as Morgan
  fingerprints (binary substructure vectors). Ranges 0 (no shared features) to 1
  (identical). The `< 0.5` cutoff ensures kept compounds are structurally distinct
  from the known actives in the training set.

## 4. Datasets

```
compound_filter/
├── hits.csv     # candidate compounds to filter
└── train.csv    # known active compounds (the similarity reference set)
```

**`hits.csv`** — columns `SMILES, LIBRARY, ANTIBIOTIC_ACTIVITY`:

```
SMILES,LIBRARY,ANTIBIOTIC_ACTIVITY
CN([C@H]1CCOC1)C(=O)c1cc(on1)COc1ccc(cc1Cl)F,BROAD,0.21633882
Cc1c(oc2c1C(=O)NCCC2)[N+](=O)[O-],BROAD,0.247323559
CSc1nsc(c1-c1ccc(cc1)Cl)SC,BROAD,0.219585328
...
```

- `SMILES` — the molecule's structure (the only column the filter logic needs).
- `LIBRARY` — source library (e.g. BROAD).
- `ANTIBIOTIC_ACTIVITY` — a continuous activity score. **Not used by the filter**
  (the task filters on structure + novelty, not on this value).

**`train.csv`** — columns `SMILES, ACTIVITY`:

```
SMILES,ACTIVITY
Nc1nnc(o1)-c1ccc(o1)[N+](=O)[O-],1
O[C@H]1COC[C@@H]2O[C@H](CC[C@H]2N(C1)C(=O)Nc1ccc(cc1)C(F)(F)F)CC(=O)NCCc1ccncc1,1
...
```

- `SMILES` — structure of a **known active** compound.
- `ACTIVITY` — activity label (`1` = active). These are the reference set every
  hit's max-Tanimoto is computed against.

## 5. What the agent has to do (condition A — the benchmark task)

Produce a program whose output matches the gold filtered list. The deterministic
recipe:

1. **Load** `hits.csv` (hit SMILES) and `train.csv` (active SMILES).
2. **Build the alert catalog** — RDKit `FilterCatalog` with PAINS + Brenk
   (`FilterCatalogParams.AddCatalog(PAINS)` and `(BRENK)`).
3. **Fingerprint the actives** — Morgan fingerprints for every `train.csv` SMILES.
4. **For each hit**, parse SMILES → `Mol`, then keep it iff:
   - it has **no** catalog match (`not catalog.HasMatch(mol)` — alert-free), **and**
   - its **max** Tanimoto over all train actives is **< 0.5** (structurally novel).
5. **Write** the kept SMILES, one per line, to
   `pred_results/compound_filter_results.txt`.

**Evaluation:** `antibioticsai_filter_eval.py` compares the produced SMILES list
against the gold filtered list (exact-match on the output file). The two gate
criteria are independent — a compound failing either is dropped.

## 6. What the condition-C (Eigenius) agent additionally does

The benchmark's Python program *is* reasoning — but **mechanized and flattened into
object code**: it keeps the operations and drops everything that warranted them (why
Morgan r=2, that `hits.csv` is Observed vs. a chosen parameter, the typed claim each
step establishes, the spec interpretation). The condition-C agent authors the
**source** the program is a lossy compilation of: a typed, kernel-checked chain that
justifies *how the program is built*. (Model-then-reason discipline, D39 §4.5.)

The reasoning is about the **methodology**, not per-compound results (those are the
program's *output*). A worked, validated reference chain lives in
[`tracer-chain.esl`](tracer-chain.esl) (test:
`crates/eigenius-reasoning/tests/sab16_tracer.rs`, validates to `Holds`):

| Phase | Artifact | Warrant |
|---|---|---|
| Decisions | five `Conforms…(solution)` — featurization (Morgan r=2/2048), alert catalogs (PAINS∪Brenk), reduction (**max** over all actives), threshold (strict `<0.5`), composition (alert-free ∧ below-threshold) | Declared (each rationale cites spec vs. convention) |
| Acceptance rule | `∀p. ConformsFeaturization(p) → … → ConformsComposition(p) → ImplementsRequiredFilter(p)` | Declared |
| Conclusion | `ReasoningSentence ImplementsRequiredFilter(solution)` whose 5-premise `App`-chain (with a `SpecStr` core) the kernel type-checks | — |
| Deliverable | `bench:TaskOutput(deliverable_kind = "python_source")` — the program as `payload`, `reasoning_chain` → the conclusion | Derived |

Each methodological choice — exactly where SAB 16's correctness is won or lost — is
now an **explicit warranted decision** rather than a buried constant. A reviewer
sees `radius 2, justified by the ECFP4 convention`; a wrong choice would be a
visible, unwarranted decision. This is "compiler for AI thought" made literal:
**chain = source, Python = object code.**

**Honest limit:** each `Conforms…` is agent-attested (`Declared`) — the kernel checks
that the reasoning *composes*, not that the emitted Python *implements* the declared
decisions (the `payload` is opaque text). The forcing function + auditability are
real now; full verification is the **program-generated-from-chain** endpoint (then
consistency is by construction). The kernel guarantees the logic, not the chemistry.

**Metric caveat:** a well-warranted-but-different choice (say radius 3) validates to
`Holds` yet may *fail* the benchmark's exact-match SR (the gold's undocumented radius
2). Track `Holds` and SR separately — the gap isolates "reasoned soundly but didn't
reproduce an undocumented convention," which is itself a result.

**Decision ↔ code-section overlay.** The program isn't an opaque blob: `bench:CodeBlock`
resources link each methodological decision to the named region of the program that
realises it (`# region: alert_catalogs | featurization | filter` markers in the
`payload`). This enables a **coverage check** (tested): every region is warranted by
a decision, and every decision is realised by a region — so a *missing* methodological
step (e.g. declared "use Brenk" but no block adds it) is catchable structurally, even
though errors *within* a region (wrong radius inside `featurization`) remain agent-
attested. It's "chain = source, Python = object code" with a typed cross-reference;
the endpoint is generating the regions from the decisions (program-from-chain).
