# WRN Helicase encoding — review memo

> A retrospective account of encoding Chan et al., *WRN helicase is a synthetic
> lethal target in microsatellite unstable cancers*, **Nature** 568:551–556
> (2019), doi:10.1038/s41586-019-1102-x, into Eigenius's typed,
> kernel-checkable representation. This is the narrative companion to the
> [encoding-plan.md](01-encoding-plan.md) — the design plan, since realized in
> full and annotated in place with dated as-built increment logs — and the
> discrepancy log [recompute-findings.md](03-recompute-findings.md). It describes **what we did,
> what we found, and — explicitly — what we left out.**

## 1. The study

Chan et al. report that **WRN** (a RecQ-family DNA helicase) is a **synthetic
lethal dependency specific to microsatellite-unstable (MSI) cancers**. The
argument runs end to end from computation to mechanism:

- **Computational discovery.** Across the DepMap/Achilles (CRISPR/CERES) and
  DRIVE (RNAi/DEMETER2) genome-wide dependency screens, WRN stands out as
  **selectively essential in MSI cell lines** and spared in microsatellite-stable
  (MSS) lines. The dependency tracks the **mutator load** (microsatellite
  deletion burden) and is the **only** RecQ-family member showing this
  MSI-selectivity. MSI status is itself a **strong biomarker** for WRN
  dependence.
- **Wet-lab validation.** Competition / viability assays confirm that depleting
  WRN impairs growth selectively in MSI lines; cDNA rescue (and a
  catalytic-dead, helicase-defective mutant that fails to rescue) shows the
  effect is **on-target and requires WRN's helicase activity**; a seed-matched
  C911 control rules out an off-target reagent artifact.
- **In vivo.** KM12 xenografts show WRN depletion suppresses tumour growth,
  corroborated in MSI patient-derived models.
- **Mechanism.** WRN loss induces **MSI-selective DNA double-strand breaks**,
  activating the DNA-damage response (DDR) → cell-cycle arrest + apoptosis →
  lethality. mRNA-seq + GSEA corroborate (cell-cycle/E2F signatures down,
  apoptosis/p53 up). The breaks are **diffuse chromosomal, not telomeric** (a
  tested-and-rejected sub-hypothesis), and the lethality is partly p53-modulated
  but operative even in p53-impaired cells.

## 2. Assets we retrieved

Every datum is content-addressed (`sha256`) in
[data/MANIFEST.md](../data/MANIFEST.md); the large slices live under
`data/slices/` (gitignored, pinned by hash).

| Asset | What it is | Role |
|---|---|---|
| **Supplementary Table 1** (this paper) | 1,415 cell lines × 37 cols: WRN dependency per screen, MSI calls, mutator load, MMR/TP53 status | The Phase-1 pivot table — backbone of the computational-discovery recomputes |
| **Achilles 18Q4 `gene_effect.csv`** | CRISPR/CERES gene-effect matrix (~187 MB) | D-DIFF differential dependency, RecQ comparison, aggregate dep |
| **DRIVE `D2_DRIVE_gene_dep_scores.csv`** | RNAi/DEMETER2 dependency matrix (~59 MB) | second screen for the same |
| **CCLE Phase-2 Supp Table 7** (Ghandi 2019) | raw indel counts + MSI-calling thresholds | upstream of the MSI classification |
| **10 per-figure Nature Source Data `.xlsx`** | per-replicate wet-lab values (Fig 2–4, ED Fig 3–10) | the recomputed + linked-external wet-lab readouts |
| **GSE126464 RNA-seq** + **MSigDB Hallmark v6.2** | WRN-KO expression counts + gene sets | GSEA mechanism corroboration |
| **DepMap 18Q4 omics bundle** (1.6 GB `.rds`) | curated matrices (expression, CN, mutation, RPPA) | omics analyses (paralog co-loss, etc.) |
| **Authors' code** (`github.com/cancerdatasci/WRN_manuscript`) | `WRN_stats_calcs.Rmd`, `in_vivo_KM12_analysis.R`, … | the authoritative recompute reference — *exactly which model produced which number* |

The authors' own R is load-bearing: it told us, for example, that the
competition-assay figures are `lm(value ~ is_WRN + guide) %>% anova()` and the
xenograft is `lmer(Volume ~ Day + (0+Day|Mouse))` — knowledge we needed to
recompute or wrap faithfully rather than guess.

## 3. How we represented it

The encoding is a **layered chain** (each layer immutable, parent-pointed):
ontology deps (`bench-core`, `onco`, and the bootstrap `reference` ontology) →
bibliographic **literature** warrants (`chain/02-literature.esl`) → narrative →
recompute **plans** (emitters) → recompute **conclusions** (consumers) →
wrapped-R warrants → reasoning phases (2/3/5) → a biological-SAP layer. Two
institutions compose through the shared chain: the **statistics institution**
writes `IsDerivedAs` witnesses; the **reasoning institution** reads them via the
D49 ChainWitness index to discharge `JustifiedBy` certificates. Declared rules
bridge statistical facts to domain conclusions, and the reasoning institution
also discharges **imported published claims** (the literature warrants below) as
Declared premises inside those certificates.

Every claim carries one of **four warrant grades**, ordered by strength:

1. **Recomputed (kernel-checkable).** The statistics institution re-runs the
   headline statistic from the pinned data, deterministically and
   bit-reproducibly, and emits a verdict + an `IsDerivedAs` witness:
   - Wilcoxon rank-sum — **C-WRN** (WRN selectively essential, 37 vs 91),
     **D-RECQ** (only RecQ MSI-selective, 32 vs 413), the **p53** dissection
     (23 vs 13).
   - Spearman — **D-REFINE** (dependency vs mutator load, 51 pairs).
   - Nested two-way ANOVA `value ~ is_WRN + guide` — **C-VAL** competition assay
     (KM12 P=2.74e-19, OVK18 P=1.2e-7), cell-cycle (ED 4b), apoptosis (ED 4c/d).
   - Crossed two-way ANOVA — **MMR restoration** (ED Fig 10c).
   - Two-sample / one-sample *t* — the **cDNA rescue** (WT rescues, E84A fails).
   - Classifier metrics (PPV / sensitivity) — **D-BIOM** (MSI as biomarker).
2. **Wrapped-R (operationally reproducible).** Mixed-effects models that depend
   on a runtime (REML/optimizer) rather than being mathematically re-checkable
   in-kernel run through the **R language runtime** (D55/D56) — the worker spawns
   a pinned Bioconductor container, runs the model, and commits a derived result
   with an `IsDerivedAs` witness keyed to the image digest:
   - **C-VIVO** — the authors' xenograft `lmer` random-slope LRT (p≈0.048 →
     `InVivoDependence`).
   - **C-VAL biological-unit** — our pseudoreplication-corrected
     `lmer(value ~ is_WRN + (1|guide))` LRT (p≈2.15e-6 →
     `ViabilityDependenceAtBiologicalUnit`); see Finding F4.
3. **Declared (reasoned, not measured).** Experimental-design logic the paper
   asserts: the C911 seed-control rule
   (`SeedControlInert → OnTarget`), the DSB→DDR mechanism rule, the
   selective-viability composition bridge, and the F4 dual-SAP annotation. These
   are `DeclaredResource`s with explicit rationales — first-class, but
   author-attested rather than recomputed.
4. **Linked-external (Observed-grade provenance, not recomputed).** Readouts we
   cite by their reported value with pinned source provenance but do **not**
   re-run — see §5.

**Literature references as typed, composable warrants.** The paper's cited prior
work is on the chain as first-class typed objects, not free text. A bootstrap
`reference` ontology defines `reference:Reference` (a global bibliographic
work — `doi`, `pmid`, `title`, `creator`, `container_title`, `issued_year`,
`url`) separately from `reference:Citation` (a `reflection:DeclaredResource`
recording one *use* of a work). Each citation is typed by its **CiTO** function
(`reference:citation_type`, drawn from a five-member SPAR/CiTO vocabulary —
`obtainsBackgroundFrom`, `usesMethodIn`, `citesAsEvidence`,
`citesAsSourceDocument`, `citesAsAuthority` — constrained by `allows_only`, so an
unknown function is rejected at commit). `chain/02-literature.esl` carries **18
`Reference`s** (real, validated DOIs + PMIDs) and **18 CiTO-typed `Citation`s**.

Eleven of those citations are **warrants**: a `Citation` carrying a
`reflection:canonical_proposition` (the imported claim, e.g.
`litclaim:WRNActivitiesSeparable("WRN")`) plus a `DeclarationTrace` that admits
it as an `IsDeclaredAs` witness. These are wired as **genuine logical premises**,
not provenance sidecars — the reasoning certificates discharge them with
`declared(LIT, …)`:

- `litclaim:WRNActivitiesSeparable` (Newman/Sturzenegger structure–function) →
  an antecedent of the helicase/exonuclease rules → **`concl_helicase_required`**
  and **`concl_exo_dispensable`**.
- `litclaim:C911ControlIsValid` (the C911 seed-control design) → the seed-control
  rule → **`concl_vivo_ontarget`**.
- `litclaim:pS15MarksP53Activation` (Loughery 2014, p-p53(S15) as a p53-activation
  marker) → the p53 rule → **`concl_p53_activation`**.

The remaining citations are provenance-only (CiTO-typed links to the datasets and
methods — CERES, DEMETER2, limma/voom, Hallmark — that the warrants rest on). So
the paper's bibliography is queryable, each citation says *how* the work is used,
and the load-bearing prior claims participate in the proof rather than sitting
beside it.

**Provenance is uniform.** Every recomputed datum (and now every wrapped-R
program input) traces to a pinned slice via a re-runnable recipe in
[extract/extract_samplesets.py](../extract/extract_samplesets.py); `--check`
re-derives all 17 SampleSets + both program-input tables and fails loudly on
drift. The R-program inputs were the last unpinned data in the encoding; they now
carry the same `bench:extracted_from_*` pins as the SampleSets.

**Live result.** On a clean database the full chain loads, all twelve wrapped-R
programs run in spawned containers, and **55/55 verdicts Hold** — including the
DSB-mechanism conclusions now closed (`concl_dsb_gh2ax`, `concl_dsb_gh2ax_foci`,
`concl_ddr_signaling`), the literature-composed conclusions
(`concl_helicase_required`, `concl_exo_dispensable`, `concl_vivo_ontarget`,
`concl_p53_activation`) and both halves of the F4 dual SAP (`viab_KM12_plan` at
2.74e-19 and `concl_viab_KM12_biological` at 2.15e-6) side by side. The demo is
[demo/wrn-helicase/run.sh](../../../../demo/wrn-helicase/run.sh).

## 4. What we found

Recomputing rather than restating surfaced four discrepancies between the paper's
prose and its data — the point of the exercise. Full detail in
[recompute-findings.md](03-recompute-findings.md); in brief:

- **F1 — Spearman n.** The correlation reports n=54; the pinned data yields **51**
  real pairs (3 dropped as `NA`/`NaN`). The kernel recomputes P from 51.
- **F2 — `NA` vs `NaN`.** The curated table mixes R's `NA` and computed `NaN`;
  treating both as missing is what makes F1's count reproducible.
- **F3 — MMR-restoration model.** The ED Fig 10c model was *identified from the
  authors' code* (a crossed `value ~ CL + guide`) and reproduced exactly from
  public data.
- **F4 — competition-assay pseudoreplication.** The published competition ANOVA
  tests `is_WRN` against the **technical** within-guide residual (KM12: 25 df,
  P=2.74e-19). The biological unit is the **guide** (~2–3 df); tested correctly
  (mixed model, guide as random effect) the honest P is **≈2.15e-6**. The
  conclusion is **robust** under both, but the published value overstates the
  evidence by ~13 orders of magnitude. We encode **both** SAPs — the faithful
  reproduction *and* the corrected warrant — and link them with a declared
  dual-SAP annotation, so "the published number vs. the defensible number" is a
  machine-checkable, queryable fact on the model rather than a prose footnote.
  (The paper is itself internally inconsistent here: its in-vivo arm already uses
  the mixed-effects approach.)

## 5. What we left out — and why

Honest scope boundary: not every analysis in the paper is kernel-recomputed.
**Differential dependency via `limma` (D-DIFF) — now reproduced-external (this
session).** The paper's headline genome-wide call — *WRN is the top preferential
dependency in MSI vs MSS* — is an empirical-Bayes moderated *t* (`limma`) over the
**full Achilles dependency matrix** (~187 MB, cell-lines × 17,634 genes). It is
now **run live through the substrate** and reproduces the paper exactly: **WRN
rank 1, Q = 4.81e-24** (paper: 4.8e-24). This closes what was the encoding's last
real capability gap, and it exercises the whole new stack end to end:
**[D53](../../../../docs/design/d53-large-data-tracking.md)** tracks the matrix (and
the `sample_info` bridge + Supp Table 1 MSI labels) as content-addressed
`PinnedExternalFile`s; a **multi-input `RunRuntimeScript`** ships all three to a
DooD-spawned R worker (fetched + content-verified, not inlined); the worker runs
`limma::lmFit/eBayes`; and a **D56 wrapped-R warrant** commits the small result
(`wrn:dd_achilles:result`, carrying `TopDifferentialDependency("WRN","Achilles_MSI")`)
under a ProgramTrace → IsDerivedAs witness. Why wrapped-R and not native: a crude
Welch *t* over the same join ranks WRN **8th** — limma's variance shrinkage is
precisely what lifts WRN's large, consistent effect to rank 1 with the headline Q,
so faithfully reimplementing it natively (D52) is not warranted; D53 §6 wraps the
pinned tool instead and makes the warrant re-checkable by `content_hash` + image
digest. See [recompute-findings F5](03-recompute-findings.md) and
`programs/differential-dependency/dd-achilles-limma-program.json`. The **D-DIFF family** then replicates
the call across screens and MSI callers, all run live: **DRIVE** (RNAi/DEMETER2,
`programs/differential-dependency/dd-drive-limma-program.json`) puts **WRN rank 1, Q = 1.46e-45** (paper
1.5e-45) — #1 in *both* the CRISPR and RNAi screens; and a **GDSC PCR-MSI
robustness** re-run (`programs/differential-dependency/dd-gdsc-limma-program.json`, MSI-H vs MSS/MSI-L over
the same Achilles matrix) keeps **WRN rank 1, Q = 4.66e-20** — the headline does
not depend on the MSI calling method.

A second wrapped-R mechanism warrant has now closed, leaving one
**linked-external** block (microscopy) cited with pinned provenance:

**GSEA via `fgsea` (mechanism corroboration, Fig 3a) — now reproduced-external.**
The WRN-KO mRNA-seq gene-set enrichment is run as a D56 wrapped-R warrant
(`programs/mechanism/gsea-mech-program.json`): limma-voom DE over the GSE126464 STAR counts
→ fgsea against the Hallmark `.gmt`. It reproduces Fig 3a — **G2M_CHECKPOINT
NES −3.53 / E2F_TARGETS −3.44 (padj 2.5e-49) down, P53_PATHWAY +2.89 (padj
9.9e-21) / APOPTOSIS +1.78 up** — and commits `gsea_mech:result` carrying
`CausesCellCycleArrest("WRN","MSI")` under a ProgramTrace, a *transcriptional*
witness alongside the FACS-ANOVA evidence `concl_mech` already cites. It's the
first consumer of the §4/§10 **Collection profile** (the `.gmt` as a typed
`PinnedExternalFile` whose sets are over `onco:Gene`) and the multi-input path.

**p53-activation IF via `emmeans` (mechanism corroboration, ED Fig 5) — now
reproduced-external.** The per-cell phospho-p53(S15)/p21 immunofluorescence
(175,974 cells, pinned as a D53 file-backed `PinnedExternalFile` with a
`LongTable` schema, derived from the authors' source workbook by a vendored
extractor) is dispatched through a D56 wrapped-R `emmeans` least-squares-means
contrast (`programs/mechanism/if-ed5-lsmeans-program.json`): WRN-KO vs control on
log-intensity over the MSI + TP53-proficient stratum. It reproduces ED Fig 5 —
**p-p53 logFC +0.155 (p = 7e-69), p21 +0.310 (p ≈ 0)** — and commits
`if_ed5:result` carrying `ActivatesP53Response("WRN","MSI")`, discharged by
`concl_p53_activation` (chain/08-phase3-invivo-mechanism.esl). The recompute also *sharpens* the claim
(finding F7): p21 is a p53 transcriptional target, so the p53-null MSI line KM12
fails to induce it (`p21_null_logfc` ≈ −0.074), recovering the paper's own
p53-independence point — the upstream lesion is p53-independent, the p21 arm is
p53-dependent — directly from the per-cell data. This is the first consumer of
the `emmeans` package (one-line `RImagePlan` add) and the `LongTable` schema.

**DSB induction via the 53BP1 foci (mechanism, ED Fig 6f/6h) — now
reproduced-external.** The per-cell Apple-53BP1-trunc DSB-foci counts (39,249
cells across MSS SW620/ES2 + MSI KM12/OVK18, a D53 file-backed SampleSet) run
through a D56 wrapped-R interaction lm (`programs/mechanism/foci-ed6-program.json`,
`foci ~ cell_line + condition*MSI`): the **condition×MSI interaction +1.82
(p ≈ 2.6e-142)** is the MSI-selective extra DSB induction — WRN-KO multiplies
foci ×2.08 in MSI vs ×1.04 in MSS. It commits `foci_dsb:result` carrying
`CausesDSBs("WRN","MSI")`, discharged by `concl_dsb_foci` (finding F8). This is
the concrete reproduced-external corroboration of `concl_dsb` (which still cites
the full-panel linked-external `mech_dsb`, keeping the mechanism chain
in-process-verifiable).

The rest of the DSB-marker panel is **now recomputed too** (the backlog is
closed). Each marker is recomputed by its *biologically valid* readout: **γH2AX**
by **intensity** (ED 6c, `emmeans` interaction — reproduces the paper's published
log10 fold-change 0.055 ES2 / 0.144 OVK18 and contrast P<2×10⁻¹⁶,
`concl_dsb_gh2ax`) *and* by **foci** (ED 6a/6d, interaction lm with saturated
pan-nuclear cells counted at a ceiling, `concl_dsb_gh2ax_foci`); the
DDR-signaling **pATM(S1981)** by **foci** (ED 7b/7d, interaction lm →
`onco:ActivatesDSBResponse`, `concl_ddr_signaling` — the ATM-activation bridge to
p53). A subtlety the data forced: γH2AX foci-counting is only valid if pan-nuclear
(saturated, uncountable) cells are *counted*, not dropped — they are the
most-damaged, MSI-enriched cells (pan-nuclear fraction KM12 13%→50% on WRN loss),
and dropping them inverts the result. That is why the authors quantify γH2AX
primarily by intensity, and why pATM/53BP1 (discrete foci, rarely pan-nuclear)
are quantified by foci.

The **only** mechanism readouts that stay linked are the two with **no numeric
source data**: the **Chk2(T68) western blot** (ED 7e — band levels, no per-cell
sheet) and the **telomere-FISH metaphase scoring** (ED 8a — no source sheet, and
the "no telomeric defect" finding is a qualitative negative anyway). Both are
recorded as data constraints, not scope choices (`MOESM10` carries only 7b/7d;
`MOESM11` only the 8d coloc).

The split is deliberate and is itself the prioritized roadmap: anything the
statistics institution can re-run from fetched data is recomputed (attestable);
anything that is a runtime-dependent or large-scale pipeline is linked until a
wrapped-R warrant (D56) plus, for the large-input cases, D53's Oxen-backed
`PinnedExternalFile` path closes the gap. Two frontier items have now closed: the
mixed-models case (lme4 via the R runtime), the large-data case (**limma D-DIFF
over the 187 MB matrix, run live via D53 + the multi-input D56 path**), and the
gene-set case (**fgsea GSEA over the pinned RNA-seq counts + Hallmark `.gmt`
Collection profile**), the per-cell-IF case (**`emmeans` lsmeans over the
175k-cell p53/p21 file-backed SampleSet**), the per-cell-count case (**53BP1 DSB
foci interaction lm**), and the large-multi-schema-container case (**paralogue
co-loss lm over the 1.6 GB DepMap omics rds, read in-worker via `readRDS`**), the
per-cell-intensity case (**γH2AX `emmeans` interaction, ED 6c**), and the
DDR-signaling case (**pATM(S1981) foci lm, ED 7b/7d**).
Every analysis class in the paper now runs live through the platform, and **every
per-cell quantitative DSB-mechanism readout is recomputed** (γH2AX intensity +
foci, 53BP1 foci, pATM foci); the only linked-external mechanism items left are
the two with **no numeric source data** — the Chk2(T68) western and the telomere
FISH — which is a data constraint, not a capability gap.

## 6. What the exercise demonstrates

The WRN encoding is the platform's flagship end-to-end demonstration that a
published study can be represented so that its claims are **re-derived and
kernel-checked from pinned source data**, not trusted as transcribed numbers —
and that doing so **surfaces what prose hides** (F1's sample size, F4's
pseudoreplication). The four warrant grades make the *epistemic status* of every
claim explicit and queryable: recomputed, runtime-reproduced, reasoned, or
merely cited. And the boundaries are honest: where the current implementation
can't yet recompute an analysis (limma at scale, D53), the chain says so rather
than overstating its coverage.

---

# Appendix A — Inventory of warrants, computations, and verdicts

Every row below `Holds` on the live chain (55 verdicts total; clean-DB run via
`run.sh`). Statistics are the kernel-recomputed values; the SampleSet/program
inputs are content-hash-pinned (`extract --check`).

## A.1 Recomputed statistical warrants (statistics institution)

Each plan resolves its SampleSet, dispatches on the design coordinate, recomputes
the statistic, and emits a verdict + an `IsDerivedAs(result, P)` witness whose
proposition `P` the reasoning layer discharges.

| Warrant (plan) | SampleSet (n) | Design → test | Recomputed statistic | Canonical proposition `P` |
|---|---|---|---|---|
| `wrn_dep_plan` (C-WRN) | 37 MSI / 91 MSS | IID → Wilcoxon rank-sum, 1-sided | P = 1.1e-8 (median −0.49 vs −0.11) | `lt(mean_diff_of(s), 0)` |
| `wrn_corr_plan` (D-REFINE) | 51 pairs | Paired → Spearman | ρ < 0, n = 51 *(paper said 54 — F1)* | `lt(spearman_rho(s), 0)` |
| `wrn_recq_plan` (D-RECQ) | 32 MSI / 413 MSS | IID → Wilcoxon | WRN P = 1.1e-8; BLM 0.65, RECQL 0.58 (n.s.) | `lt(mean_diff_of(s), 0)` |
| `biomarker_plan` (D-BIOM) | 37 (27 WRN-dep) | Classification → PPV / sensitivity | PPV 27/37 = 0.73; sensitivity 27/27 = 1.00 | `ge(ppv(s),0.7)`, `ge(sensitivity(s),0.9)` |
| `p53_dep_plan` | 23 p53-intact / 13 impaired | IID → Wilcoxon | P = 0.02 | `lt(mean_diff_of(s), 0)` |
| `viab_KM12_plan` (C-VAL) | 18 sgWRN / 12 ctrl (5 guides) | Nested → 2-way ANOVA, **technical** stratum | F(1, 25), P = 2.74e-19 | `lt(mean_diff_of(s), 0)` |
| `viab_OVK18_plan` | 18 / 12 (5 guides) | Nested → 2-way ANOVA | P = 1.2e-7 | `lt(mean_diff_of(s), 0)` |
| `cc_KM12_plan` / `cc_SW48_plan` / `cc_OVK18_plan` | 6 sgWRN / 3 ctrl | Nested → 2-way ANOVA (%S-phase) | 6.1e-7 / 3.5e-4 / 2.6e-6 | `lt(mean_diff_of(s), 0)` |
| `apop_KM12_plan` / `apop_SW48_plan` / `apop_OVK18_plan` | 3 ctrl / 6 sgWRN | Nested → 2-way ANOVA (apoptosis, ctrl<wrn) | 3.4e-3 / 3.6e-4 / 3.6e-5 | `lt(mean_diff_of(s), 0)` |
| `mmr_rescue_plan` / `mmr_resens1_plan` / `mmr_resens2_plan` | 12 (shWRN1,2 × 6) | Crossed → 2-way ANOVA (`value ~ CL + guide`) | ∗ vs † P = 5.7e-20 (rescue/re-sensitize arms) | `lt(mean_diff_of(s), 0)` |
| `rescue_wt_plan` | 6 GFP / 6 WT-cDNA | IID → 2-sample t | P = 2.4e-7 (0.41 → 0.68) | `lt(mean_diff_of(s), 0)` |
| `rescue_e84a_plan` | 6 GFP / 6 E84A-cDNA | IID → 2-sample t | P = 3.4e-6 (→ 0.80) | `lt(mean_diff_of(s), 0)` |

*18 plans.* The proposition shape is uniform: a comparison reduces to `lt(f(s),
0)` for a statistic function `f` (the directionality witness licenses the
one-sided test); the biomarker reduces to two `ge(·, threshold)` facts.

## A.2 Wrapped-R warrants (R language runtime, D55/D56)

| Program | Input (n) | Model | LRT p | Proposition `P` | Conclusion |
|---|---|---|---|---|---|
| `program:xenograft_lme4` | `vivo_xenograft_table` (73 rows, 10 mice) | `lmer(Volume ~ Day + Day:Dox + (0+Day\|Mouse))`, LRT of the `Day:Dox` interaction | **0.04845** | `InVivoDependence(WRN, MSI)` | `concl_vivo` |
| `program:km12_competition_lme4` | `viab_KM12_competition_table` (30 rows, 5 guides) | `lmer(value ~ is_WRN + (1\|guide))`, LRT vs guide-only | **2.1475e-6** | `ViabilityDependenceAtBiologicalUnit(WRN, KM12)` | `concl_viab_KM12_biological` |

The second is the F4 biological-stratum counterpart of `viab_KM12_plan`'s
published technical-stratum 2.74e-19 — same data, honest unit of inference.

## A.3 Domain conclusions (reasoning institution)

The 33 `ReasoningSentence`s, each with the proposition it asserts and the grade of
its load-bearing warrant (R = recomputed, W = wrapped-R, D = declared,
L = linked-external).

| Conclusion | Proposition | Grade |
|---|---|---|
| `concl_wrn_selective` (narrative) | `SelectivelyEssential(WRN, MSI)` | L |
| `concl_wrn_selective_recomputed` | `SelectivelyEssential(WRN, MSI)` | R |
| `concl_refine_recomputed` | `DependencyCorrelatesWithMutatorLoad(WRN, MSI)` | R |
| `concl_recq_recomputed` | `OnlyMSISelectiveInFamily(WRN, RecQ_helicases)` | R |
| `concl_biomarker_recomputed` | `StrongBiomarker(MSI, WRN_dependency)` | R |
| `concl_lineage_mutator_recomputed` | `ElevatedMutatorLoadInCommonLineages(MSI_common, MSI_uncommon)` | R (Wilcoxon, ED Fig 2b) |
| `concl_coloc_recomputed` | `ReducedNucleolarColocalization(WRN, MSI)` | R (t-test, ED Fig 8d) |
| `concl_hcr_recomputed` | `MMRRestorationRestoresRepair(HCT116, Ch3plus5)` | R (t-test, ED Fig 10a) |
| `concl_apop_shrna_recomputed` | `CausesApoptosis(WRN, MSI)` | R (shRNA, ED Fig 4d) |
| `concl_p53_modulates` | `ModulatesDependence(TP53, WRN)` | R |
| `concl_val_recomputed` | `SelectiveViabilityDependence(WRN, MSI)` | R (+D bridge) |
| `concl_cellcycle_recomputed` | `CausesCellCycleArrest(WRN, MSI)` | R |
| `concl_apoptosis_recomputed` | `CausesApoptosis(WRN, MSI)` | R |
| `concl_mmr_restoration_recomputed` | `RestorationPartiallyRescues(dMMR, WRN)` | R |
| `concl_rescue_wt_recomputed` | `RescuesDepletion(WRN_cDNA_WT, sgWRN_EIJ)` | R |
| `concl_rescue_e84a_recomputed` | `RescuesDepletion(WRN_cDNA_E84A, sgWRN_EIJ)` | R |
| `concl_ontarget` | `OnTarget(WRN, MSI_viability)` | D |
| `concl_helicase_required` | `RequiresActivity(WRN, helicase)` | D |
| `concl_exo_dispensable` | `DispensableActivity(WRN, exonuclease)` | D |
| `concl_vivo` | `InVivoDependence(WRN, MSI)` | W |
| `concl_vivo_ontarget` | `OnTarget(WRN, xenograft_growth)` | D (over W) |
| `concl_viab_KM12_biological` | `ViabilityDependenceAtBiologicalUnit(WRN, KM12)` | W |
| `concl_dsb` | `CausesDSBs(WRN, MSI)` | L |
| `concl_dsb_foci` | `CausesDSBs(WRN, MSI)` | W (53BP1 foci interaction lm, ED Fig 6f/6h) |
| `concl_dsb_gh2ax` | `CausesDSBs(WRN, MSI)` | W (γH2AX intensity emmeans interaction, ED Fig 6c) |
| `concl_dsb_gh2ax_foci` | `CausesDSBs(WRN, MSI)` | W (γH2AX foci interaction lm, pan-nuclear at ceiling, ED Fig 6a/6d) |
| `concl_ddr_signaling` | `ActivatesDSBResponse(WRN, MSI)` | W (pATM(S1981) foci interaction lm, ED Fig 7b/7d) |
| `concl_p53_activation` | `ActivatesP53Response(WRN, MSI)` | W (emmeans lsmeans, ED Fig 5) |
| `concl_mech` | `DSBDrivenLethality(WRN, MSI)` | D (over R+L) |
| `concl_not_telomere` | `NotViaTelomereDefect(WRN, MSI)` | L |
| `concl_paralog` | `NotExplainedByParalogLoss(WRN, MSI)` | W (paralogue co-loss lm over the 1.6 GB DepMap rds, ED Fig 9a) |
| `concl_mmr` | `ContributesToDependence(dMMR, WRN)` | D |
| `concl_main` | `SyntheticLethal(WRN, MSI)` | D (composes all) |

## A.4 Declared bridges & the F4 annotation (selected)

| Resource | Logical content |
|---|---|
| `bridge_biomarker` | `ge(ppv(s),0.7) → ge(sensitivity(s),0.9) → StrongBiomarker(MSI, WRN_dependency)` |
| `bridge_viability` | `lt(mean_diff_of(s_KM12),0) → lt(mean_diff_of(s_OVK18),0) → SelectiveViabilityDependence(WRN, MSI)` |
| `seed_control_rule` | `SeedControlInert(WRN, xenograft_growth) → OnTarget(WRN, xenograft_growth)` |
| `viab_KM12_dual_sap` | declares the F4 relation: technical-stratum 2.74e-19 vs biological-stratum 2.15e-6, conclusion robust |

## A.5 Statistical vocabulary (the "applications")

Function symbols the statistics institution computes over a SampleSet IRI `s`,
and the relations that turn them into `Prop`s:

| Symbol | Type | Meaning |
|---|---|---|
| `mean_diff_of(s)` | `SampleSet → Float` | mean(group A) − mean(group B) |
| `spearman_rho(s)` | `SampleSet → Float` | Spearman rank correlation |
| `ppv(s)` | `SampleSet → Float` | positive predictive value of the classifier |
| `sensitivity(s)` | `SampleSet → Float` | classifier sensitivity (recall) |
| `lt(x, c)` | `Float → Float → Prop` | `x < c` |
| `ge(x, c)` | `Float → Float → Prop` | `x ≥ c` |

Certificate / witness term-formers (D49 ChainWitness + D54 justification):
`derived(r, P)` and `declared(r, P)` (a chain witness inhabiting `P`),
`DerivedEvidence(r)` / `DeclaredEvidence(r)` (citing a witness), and `app`
(application / →-elimination).

---

# Appendix B — A warrant in logical notation

To show what the raw ESL/Eigon-JSON *means*, here is the **D-BIOM** warrant
(`concl_biomarker_recomputed`, MSI is a strong biomarker for WRN dependency) in
proof-theoretic terms. It is the richest single warrant — two recomputed inputs
discharged through a two-premise declared bridge.

**The propositions.** Over the dependency SampleSet `s = wrn_dep_sampleset`:

$$
\mathsf{PPV} \;\equiv\; \mathit{ppv}(s) \ge 0.7
\qquad
\mathsf{SENS} \;\equiv\; \mathit{sensitivity}(s) \ge 0.9
\qquad
\mathsf{SB} \;\equiv\; \mathrm{StrongBiomarker}(\mathrm{MSI}, \mathrm{WRN\_dep})
$$

**The recomputes are axiom leaves.** The statistics institution evaluates the
classifier and, because the computed values clear the thresholds
(`ppv = 0.73 ≥ 0.7`, `sensitivity = 1.00 ≥ 0.9`), commits two derived results
whose `IsDerivedAs` witnesses *inhabit* those propositions:

$$
r_{\mathrm{ppv}} : \mathsf{PPV}
\qquad\qquad
r_{\mathrm{sens}} : \mathsf{SENS}
$$

**The declared bridge is an implication.** The author asserts the criterion as a
curried implication, inhabited by the declared resource `B = bridge_biomarker`:

$$
B \;:\; \mathsf{PPV} \to \mathsf{SENS} \to \mathsf{SB}
$$

**The certificate is the proof term.** The reasoning sentence's certificate is
exactly the term `B\ r_{\mathrm{ppv}}\ r_{\mathrm{sens}}`, i.e. two applications
(→-elimination / modus ponens):

$$
\dfrac{
  \dfrac{\;B : \mathsf{PPV}\to\mathsf{SENS}\to\mathsf{SB}
        \qquad r_{\mathrm{ppv}} : \mathsf{PPV}\;}
        {\,B\;r_{\mathrm{ppv}} \;:\; \mathsf{SENS}\to\mathsf{SB}\,}\ {\to}E
  \qquad
  r_{\mathrm{sens}} : \mathsf{SENS}
}{
  B\;r_{\mathrm{ppv}}\;r_{\mathrm{sens}} \;:\; \mathsf{SB}
}\ {\to}E
$$

Type-checking that proof term against the sentence's stated proposition `SB`
**is** the verdict: `qc_validate_justification` elaborates the certificate, each
`derived(r, P)` leaf is discharged by looking `P` up in the per-layer
ChainWitness index (it must resolve to a real `IsDerivedAs` the statistics
institution actually committed), the declared leaf against an `IsDeclaredAs`, and
if the term inhabits `SB` the sentence `Holds`. There is no separate "trust the
numbers" step — the number's significance *is* the witness, and the conclusion *is*
the proof.

**The same shape in the committed ESL** (`concl_biomarker_recomputed`,
abbreviated) — what the notation above renders:

```
proposition  = StrongBiomarker("MSI", "WRN_dependency")                 -- the goal SB
justification = App(App(DeclaredEvidence(bridge_biomarker),             -- B
                        DerivedEvidence(biomarker_plan:result:ppv)),    -- r_ppv
                    DerivedEvidence(biomarker_plan:result:sensitivity)) -- r_sens
certificate  = app( SENS, SB,                                           -- final →E
                    App(DeclaredEvidence(B), DerivedEvidence(R_PPV)),   -- B r_ppv
                    DerivedEvidence(R_SENS),                            -- r_sens
                    cert1,                                              -- proof of B r_ppv
                    derived(R_SENS, SENS) )                             -- r_sens : SENS leaf
```

Read top to bottom, the proof tree and the certificate term are the same object:
the **statistical recomputes are the leaves, the declared bridge is the
implication, and the domain conclusion is what the application inhabits.** That
correspondence — verdict = inhabitation of the asserted proposition by a proof
term whose leaves are chain-resident witnesses — is the whole point of the
encoding.

---

# Appendix C — Traceability to the Nature paper

Mapping every chain proposition back to where Chan et al. argue it. Anchored to
the published Nature article (`references/publications/WRN-Helicase-Nature.pdf`,
converted to text via `pdftotext` →
`references/publications/WRN-Helicase-Nature-OCR/WRN-Helicase-Nature_pdftotext.txt`).
Figure numbers cross-checked against [data/MANIFEST.md](../data/MANIFEST.md) (which
ties each source-data file to its figure). Grade: R = recomputed, W = wrapped-R,
D = declared, L = linked-external.

The chain's proposition graph follows the paper's argument arc — hypothesis →
computational discovery → wet-lab validation → in vivo → mechanism → thesis. Each
conclusion's `reflection:declared_by` already names the paper criterion in the
left column; this table grounds it in the Nature figure + narrative claim.

### Hypothesis
| Proposition (conclusion) | Nature locus | Narrative claim | Grade |
|---|---|---|---|
| `SyntheticLethal(WRN, MSI)` (`concl_main`) | Abstract; Title; whole paper | "WRN is a synthetic lethal vulnerability and drug target for MSI cancers" — the thesis, composing all of the below | D (composes all) |

### Computational discovery (Fig. 1)
| Proposition | Nature locus | Narrative claim | Grade |
|---|---|---|---|
| `SelectivelyEssential(WRN, MSI)` (`concl_wrn_selective` / `…_recomputed`) | Fig. 1a; main text | "the RecQ helicase WRN was selectively essential in MSI models … dispensable in MSS" | L (narrative) / **R** (Wilcoxon recompute) |
| `DependencyCorrelatesWithMutatorLoad(WRN, MSI)` (`concl_refine_recomputed`) | Fig. 1b | WRN dependency scales with the microsatellite-deletion (mutator) load | R (Spearman) |
| `OnlyMSISelectiveInFamily(WRN, RecQ_helicases)` (`concl_recq_recomputed`) | Extended Data Fig. (RecQ family) | "none of the four other RecQ DNA helicases were preferentially essential in MSI cell lines" | R (Wilcoxon) |
| `StrongBiomarker(MSI, WRN_dependency)` (`concl_biomarker_recomputed`) | Extended Data Fig. (biomarker) | MSI–WRN compares favourably to KRAS/BRAF biomarker–dependency relationships | R (PPV/sensitivity) |

### Wet-lab validation & structure–function (Fig. 2, ED Fig. 3)
| Proposition | Nature locus | Narrative claim | Grade |
|---|---|---|---|
| `SelectiveViabilityDependence(WRN, MSI)` (`concl_val_recomputed`) | Fig. 2a / ED Fig. 3b (competition assay) | WRN depletion impairs MSI viability, spares MSS | R (nested ANOVA) + D bridge |
| `ViabilityDependenceAtBiologicalUnit(WRN, KM12)` (`concl_viab_KM12_biological`) | ED Fig. 3b *(our F4 re-analysis)* | the biologically-honest (guide-level) restatement of the above; **our addition, not a paper claim** | W (lme4) |
| `OnTarget(WRN, MSI_viability)` (`concl_ontarget`) | Fig. 2b,c (WRN-EIJ sgRNA rescue) | the phenotype is attributable to WRN inactivation, not an off-target reagent effect | D |
| `RescuesDepletion(WRN_cDNA_WT, sgWRN_EIJ)` (`concl_rescue_wt_recomputed`) | Fig. 2c | wild-type WRN cDNA rescues EIJ depletion | R (2-sample t) |
| `RescuesDepletion(WRN_cDNA_E84A, sgWRN_EIJ)` (`concl_rescue_e84a_recomputed`) | Fig. 2c | exonuclease-dead E84A cDNA still rescues | R (2-sample t) |
| `RequiresActivity(WRN, helicase)` (`concl_helicase_required`) | Fig. 2c (helicase-dead fails to rescue) | "MSI cancer models required the helicase activity of WRN" | D |
| `DispensableActivity(WRN, exonuclease)` (`concl_exo_dispensable`) | Fig. 2c | "…but not its exonuclease activity" | D |

### In vivo (Fig. 2d, organoids 2f,g)
| Proposition | Nature locus | Narrative claim | Grade |
|---|---|---|---|
| `InVivoDependence(WRN, MSI)` (`concl_vivo`) | Fig. 2d (xenograft) + 2f,g (organoid) | "Induction of WRN shRNA 1 … significantly impaired tumour growth" | W (lme4 random-slope LRT) |
| `OnTarget(WRN, xenograft_growth)` (`concl_vivo_ontarget`) | Fig. 2d (WRN^C911 seed control) | WRN^C911 shRNA is inert in vivo ⇒ the in-vivo effect is on-target | D (seed-control rule, over W) |

### Mechanism (Fig. 3–4, ED Fig. 4–8, 10)
| Proposition | Nature locus | Narrative claim | Grade |
|---|---|---|---|
| `CausesDSBs(WRN, MSI)` (`concl_dsb`) | Fig. 4a; ED Fig. 6,7 | "WRN silencing in MSI, but not MSS, cells substantially increased γH2AX and 53BP1 foci (DSBs)" | L |
| `CausesCellCycleArrest(WRN, MSI)` (`concl_cellcycle_recomputed`) | ED Fig. 4b | "WRN silencing reduced the proportion of MSI cells in S phase … cell cycle arrest" | R (nested ANOVA) |
| `CausesApoptosis(WRN, MSI)` (`concl_apoptosis_recomputed`) | ED Fig. 4c | WRN loss raises apoptosis selectively in MSI | R (nested ANOVA) |
| `ModulatesDependence(TP53, WRN)` (`concl_p53_modulates`) | Fig. 3 (p53 activation) | "p53 activation in WRN-depleted MSI cells" partly modulates the dependence | R (Wilcoxon) |
| `DSBDrivenLethality(WRN, MSI)` (`concl_mech`) | Fig. 3–4 (GSEA + DSB + arrest + apoptosis) | DSB → DDR is the MSI-selective lethal mechanism | D (over R + L) |
| `NotViaTelomereDefect(WRN, MSI)` (`concl_not_telomere`) | Fig. 4d,e; ED Fig. 8 (FISH) | the DSBs are diffuse chromosomal, **not** telomeric — a tested-and-rejected sub-hypothesis | L |
| `ContributesToDependence(dMMR, WRN)` (`concl_mmr`) | Fig. 4f; ED Fig. 10e,f (MMR re-knockout) | MMR loss is causal for the WRN dependence | D |
| `RestorationPartiallyRescues(dMMR, WRN)` (`concl_mmr_restoration_recomputed`) | ED Fig. 10c | restoring MMR (chr 3+5) raises WRN-depletion viability | R (crossed ANOVA) |

**What the table shows.** Every chain proposition resolves to a specific Nature
figure and narrative beat; the warrant grade records *how strongly* we stand
behind it (recomputed > wrapped-R > declared > linked-external). The two places
the chain departs from the paper are explicit: the `…_recomputed` propositions
re-derive what the paper asserts (and, per Findings F1/F4, sometimes disagree on
the number), and `ViabilityDependenceAtBiologicalUnit` is *our* methodological
addition, flagged as such.

> **Note on representation.** This table is the *documentation* form of the
> figure-to-proposition mapping. The complementary structural step — promoting
> the paper's **bibliography** to first-class resolvable resources — is **done**
> (it was deferred to its own design note when this memo was first written): the
> bootstrap `reference` ontology + [`chain/02-literature.esl`](../chain/02-literature.esl)
> put 18 typed `reference:Reference` works and 18 CiTO-typed `reference:Citation`s
> on the chain, with 11 of them composed into the proof as imported-claim warrants
> (see §3, "Literature references as typed, composable warrants"). The
> figure-to-chain-proposition mapping in this appendix is still carried as
> documentation rather than as per-proposition `cites` edges; tightening *that*
> last link is the remaining structural step.
