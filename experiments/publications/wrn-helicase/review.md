# WRN Helicase encoding — review memo

> A retrospective account of encoding Chan et al., *WRN helicase is a synthetic
> lethal target in microsatellite unstable cancers*, **Nature** 568:551–556
> (2019), doi:10.1038/s41586-019-1102-x, into Eigenius's typed,
> kernel-checkable representation. This is the narrative companion to the
> forward-looking [encoding-plan.md](encoding-plan.md) and the discrepancy log
> [recompute-findings.md](recompute-findings.md). It describes **what we did,
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
[data/MANIFEST.md](data/MANIFEST.md); the large slices live under
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
ontology deps (`bench-core`, `onco`) → narrative (`wrn-phase1`) → recompute
**plans** (emitters) → recompute **conclusions** (consumers) → wrapped-R warrants
→ reasoning phases (2/3/5) → a biological-SAP layer. Two institutions compose
through the shared chain: the **statistics institution** writes `IsDerivedAs`
witnesses; the **reasoning institution** reads them via the D49 ChainWitness
index to discharge `JustifiedBy` certificates. Declared rules bridge statistical
facts to domain conclusions.

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

**Provenance is uniform.** Every recomputed datum (and now every wrapped-R
program input) traces to a pinned slice via a re-runnable recipe in
[extract/extract_samplesets.py](extract/extract_samplesets.py); `--check`
re-derives all 17 SampleSets + both program-input tables and fails loudly on
drift. The R-program inputs were the last unpinned data in the encoding; they now
carry the same `bench:extracted_from_*` pins as the SampleSets.

**Live result.** On a clean database the full chain loads, both lme4 R programs
run in spawned containers, and **41/41 verdicts Hold** — including both halves of
the F4 dual SAP (`viab_KM12_plan` at 2.74e-19 and `concl_viab_KM12_biological` at
2.15e-6) side by side. The demo is [demo/wrn-helicase/run.sh](../../../demo/wrn-helicase/run.sh).

## 4. What we found

Recomputing rather than restating surfaced four discrepancies between the paper's
prose and its data — the point of the exercise. Full detail in
[recompute-findings.md](recompute-findings.md); in brief:

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
Three remain **linked-external** `bench:ToolArtifact`s — cited with pinned
provenance, but not re-executed. They differ in *why*, and the distinction
matters:

- **Differential dependency via `limma` (D-DIFF) — a real capability gap.** The
  paper's genome-wide differential-dependency call is an empirical-Bayes
  moderated *t* (`limma`) over the **full Achilles dependency matrix** (~187 MB,
  cell-lines × 17,634 genes). We do **not** run it. limma-the-analysis would be a
  **wrapped-R warrant** (the same D56 mechanism as the lme4 models we *did* ship);
  the blocker is its **large input** — that matrix is too big to inline on the
  chain. The fix is **[D53](../../../docs/design/d53-large-data-tracking.md)**:
  track the matrix as an Oxen-backed `PinnedExternalFile`, fetch + content-verify +
  materialize it into the worker, and commit only the small differential-dependency
  result. Neither D53's `PinnedExternalFile` path nor a D56 limma warrant is built
  yet, so D-DIFF stays Observed-grade for now — a known boundary, not something we
  papered over. The *downstream* consequence of D-DIFF — that WRN is MSI-selective —
  *is* independently recomputed by the Wilcoxon C-WRN warrant, so the conclusion is
  not left unsupported, only this particular statistic is un-rerun.

- **GSEA via `fgsea` (mechanism corroboration, Fig 3a) — not-yet-wired.** The
  mRNA-seq gene-set enrichment (WRN-KO vs Hallmark sets; cell-cycle/E2F down,
  apoptosis/p53 up) is a permutation test (1M permutations). Its inputs are
  vendored (`GSE126464`, `h.all.v6.2.symbols.gmt`) and *not* large; we simply
  have not wired an fgsea path. It corroborates C-MECH (cited in the `concl_mech`
  rationale) but is not load-bearing for any verdict. Now that the R runtime
  hosts `lme4`, fgsea is a natural next wrapped-R warrant — a backlog item, not a
  capability gap.

- **Immunofluorescence contrasts + foci counts (lsmeans / direct readouts) —
  not-yet-wired.** The p53-S15 / p21 IF least-squares-means contrasts (Fig 3c,
  ED 5), the γH2AX/53BP1/pATM/Chk2 DSB foci (ED 6/7), and FISH (ED 8) stay
  linked-external. Source data is pinned; these are the paper's own microscopy
  assays, deferred behind the same external-tool frontier.

The split is deliberate and is itself the prioritized roadmap: anything the
statistics institution can re-run from fetched data is recomputed (attestable);
anything that is a runtime-dependent or large-scale pipeline is linked until a
wrapped-R warrant (D56) plus, for the large-input cases, D53's Oxen-backed
`PinnedExternalFile` path closes the gap. The mixed-models frontier item *already*
closed this session (lme4 via the R runtime); limma is the next, gated on the D53
large-data input path.

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

Every row below `Holds` on the live chain (41 verdicts total; clean-DB run via
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

The 23 `ReasoningSentence`s, each with the proposition it asserts and the grade of
its load-bearing warrant (R = recomputed, W = wrapped-R, D = declared,
L = linked-external).

| Conclusion | Proposition | Grade |
|---|---|---|
| `concl_wrn_selective` (narrative) | `SelectivelyEssential(WRN, MSI)` | L |
| `concl_wrn_selective_recomputed` | `SelectivelyEssential(WRN, MSI)` | R |
| `concl_refine_recomputed` | `DependencyCorrelatesWithMutatorLoad(WRN, MSI)` | R |
| `concl_recq_recomputed` | `OnlyMSISelectiveInFamily(WRN, RecQ_helicases)` | R |
| `concl_biomarker_recomputed` | `StrongBiomarker(MSI, WRN_dependency)` | R |
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
| `concl_mech` | `DSBDrivenLethality(WRN, MSI)` | D (over R+L) |
| `concl_not_telomere` | `NotViaTelomereDefect(WRN, MSI)` | L |
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
Figure numbers cross-checked against [data/MANIFEST.md](data/MANIFEST.md) (which
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
> mapping (option (a)). Today the link is carried by free-text
> `reflection:declared_by` slugs (`wrn-paper:…`) that resolve to nothing, and the
> paper's own bibliography is not on the chain. Promoting the publication and its
> claims/citations to first-class resolvable resources — so the mapping is
> queryable and literature references are typed objects — is a separate
> structural step (options (b)/(c)), deferred to its own design note.
