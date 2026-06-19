# WRN encoding — recompute findings log

Discrepancies surfaced by recomputing the paper's claims against the pinned public-data
snapshot ([data/MANIFEST.md](../data/MANIFEST.md)). Each is the discipline working: a gap
between the published number and what the data re-yields. Per [encoding-plan §5.1](01-encoding-plan.md),
a divergence is a recorded finding, not a silent pass.

| # | Claim (paper) | Recomputed | Class | Verdict |
|---|---|---|---|---|
| F1 | Spearman WRN-dep ~ #MS-deletions, all MSI: rho = −0.74, **n = 54** | rho = **−0.74** (matches), **n = 51** | B (effect) ✓ / **A (count) ✗** | **Discrepancy — benign** |
| F2 | Biomarker (common-MSI lineages): PPV = 0.73 (27/37), sensitivity = 1.00 (27/27) | PPV = **0.73 (27/37)**, sensitivity = **1.00 (27/27)** | A ✓ | **Confirms paper — provenance gotcha noted** |
| F3 | ED Fig 10c MMR-restoration (C-MMR): two-way ANOVA P = 5.7e-20 / 3.3e-12 / 1.6e-16 | **5.74e-20 / 3.26e-12 / 1.56e-16** (exact, from public MOESM12) | A ✓ | **Reproduces paper exactly** |
| F4 | C-VAL / C-MMR competition assays: two-way ANOVA `value ~ is_WRN + guide` (e.g. KM12 P = 2.7e-19) | **2.74e-19 reproduced** — but tests the *technical* residual; biological-unit P ≈ **2e-3 – 2e-6** | Methodology (design) | **Reproduces paper exactly; flags pseudoreplication; conclusion robust** |
| F5 | **D-DIFF (Achilles): WRN is the top MSI-vs-MSS differential dependency, Q = 4.8e-24** | **WRN rank 1, Q = 4.81e-24** (limma moderated-t over the 187 MB CERES matrix; 32 MSI / 413 MSS) | A ✓ | **Reproduces paper exactly — run live through the substrate (D53 + D56)** |
| F6 | **C-MECH (Fig 3a GSEA): WRN-KO depletes proliferation signatures, activates p53** | **G2M −3.53 / E2F −3.44 (padj 2.5e-49) down; P53 +2.89 (padj 9.9e-21) / apoptosis +1.78 up** (limma-voom → fgsea vs Hallmark) | A ✓ | **Reproduces Fig 3a — run live through the substrate (D53 Collection + D56)** |
| F7 | **C-MECH (ED Fig 5 IF): WRN-KO raises p-p53(S15)/p21 in MSI cells** | **p-p53 +0.155 (p=7e-69) / p21 +0.310 (p≈0)** in MSI+p53-proficient; **p21 NOT induced (−0.074)** in the p53-null MSI line KM12 (emmeans lsmeans over 175,974 cells) | A ✓ / **refinement** | **Reproduces + sharpens the claim: p21 induction is p53-status-gated, recovering the paper's p53-independence point from the data** |
| F8 | **C-MECH (ED Fig 6f/6h): WRN-KO induces 53BP1 DSB foci MSI-selectively** | **MSI lines ×2.08, MSS lines ×1.04** foci on WRN-KO; condition×MSI interaction **+1.82, p≈2.6e-142** (wrapped-R lm over 39,249 cells) | A ✓ | **Reproduces MSI-selective DSB induction — lifts the 53BP1 arm of CausesDSBs to reproduced-external** |
| F9 | **ED Fig 9a: WRN's MSI-dependence is not explained by paralogue co-loss** | MSI β **−0.667 (p=4.4e-60)** baseline; stays **−0.67..−0.70 (worst p≈1e-58)** controlling for each RECQ paralogue's loss (wrapped-R lm over the 1.6 GB DepMap rds) | A ✓ | **Reproduces the rejected-alternative test — run live over the large multi-schema D53 rds container** |
| F10 | **ED Fig 2b: common MSI lineages carry a higher mutator load than uncommon** | Wilcoxon **P = 1.7e-9** (uncommon n=45 vs common n=54, ms_deletions) | A ✓ | **Reproduces paper exactly — D52-native (kernel-recomputed inline SampleSet)** |
| F11 | **ED Fig 8d: WRN is delocalized from the nucleolus in MSI** | WRN–fibrillarin coloc MSI 0.36 < MSS 0.69, t **p = 3.2e-8** (n=15 vs 10) | A ✓ | **Reproduces paper — D52-native t-test** |
| F12 | **ED Fig 10a: restoring MMR restores mismatch repair** | host-cell reactivation parental 1.9 → Ch3+5 35.8, t **p = 2.3e-3** | A ✓ | **Reproduces paper — D52-native t-test (MMR-defect functional confirmation)** |
| F13 | **ED Fig 4d: WRN-KD raises apoptosis in MSI (shRNA, orthogonal to CRISPR)** | KM12 (MSI) control 12.3 → shWRN 35.1; MSS SW837 spared (12 → 8.6) | A ✓ | **Reproduces paper — D52-native; on-target confirmation (not a Cas9 artifact)** |
| F14 | **ED Fig 6c: WRN-KO raises γH2AX (canonical DSB marker) MSI-selectively** | intensity log10 FC **0.055 ES2 / 0.144 OVK18**, MSI-vs-MSS contrast **P<2×10⁻¹⁶** (wrapped-R emmeans interaction) | A ✓ | **Reproduces the paper's published γH2AX statistic — the canonical-lesion leg of CausesDSBs** |
| F14b | **ED Fig 6a/6d: WRN-KO raises γH2AX foci MSI-selectively (discrete-foci leg)** | interaction **+7.3**, foci **×3.4 MSI vs ×1.0 MSS** (wrapped-R lm; pan-nuclear cells counted at saturation ceiling) | A ✓ | **Reproduces the foci view; counting pan-nuclear vs dropping is load-bearing — dropping inverts the sign** |
| F15 | **ED Fig 7b/7d: WRN-KO activates pATM(S1981) DDR signaling MSI-selectively** | foci **×1.74 MSI vs ×1.11 MSS**, interaction **p≈0** (wrapped-R lm → `ActivatesDSBResponse`) | A ✓ | **Reproduces the DSB→p53 ATM-signaling bridge — a new proposition, previously absent from the chain** |

## F3 — ED Fig 10c MMR-restoration: model identified from authors' code, reproduced exactly from public data

**Where:** `data/WRN_manuscript/src/WRN_stats_calcs.Rmd:228-323` (the authors' own analysis code,
vendored). The C-MMR MMR-restoration viability contrasts (ED Fig 10c — the Ch2-vs-Ch3+5 rescue
and the two sgMLH1-KO re-sensitization controls) are each a **crossed additive two-way ANOVA**
`lm(value ~ CL + guide)` over a *pair* of conditions, testing the `CL` (MMR-context) main effect
controlling for `guide`. This is the **same formula family** as C-VAL's `value ~ is_WRN + guide`
— *not* the pooled interaction-contrast model first assumed. The distinction from the nested
dispatch (increment 8): here `guide` is **crossed** (the same shRNAs appear in both `CL` levels),
so the residual is `N − #CL − #guide + 1` with any CL×guide interaction pooled into it — a
different SS decomposition than the nested `N − n_subgroups`.

**The exact recipe (reproduces the paper to 2 s.f.).** Source: `wrn_sourcedata_EDFig10_MOESM12.xlsx`,
sheet **"ED Fig 10c"** (relative viability, **n = 6 biological replicates**) — four HCT116
derivative blocks: Ch2 (∗), Ch3+5+sgCh2-2 (†), Ch3+5+sgMLH1-1 (‡), Ch3+5+sgMLH1-2 (§), each with
guides {shRFP, shPSMD2, shRPL6, shWRN1, shWRN2} × 6 reps. Use the **normalized** (relative-viability)
values, filter to the **shWRN guides** (shWRN1+shWRN2 — the bars the ∗†‡§ symbols mark), and run
`lm(value ~ CL + guide)` on each pair, testing the CL main effect:

| Contrast | Conditions | Paper P | Recomputed |
|---|---|---|---|
| ∗ vs † | Ch2 → Ch3+5 (restore MMR) | 5.7e-20 | **5.74e-20** |
| † vs ‡ | Ch3+5 → +sgMLH1-1 (re-sensitize) | 3.3e-12 | **3.26e-12** |
| † vs § | Ch3+5 → +sgMLH1-2 (re-sensitize) | 1.6e-16 | **1.56e-16** |

**Correction of an earlier mis-call.** A first pass reported this as "blocked — public data ≠
analysis data," for two wrong reasons: (1) it analyzed the wrong sheet — "ED Fig 10f" (n = 3, a
secondary clonogenic-adjacent panel with duplicate-fill replicates like Ch2/shWRN1 = `0.10, 9.19,
9.19`), not the n = 6 viability sheet "ED Fig 10c" that backs the reported p-values; (2) it tested
the CL main effect over **all** guides (≈7e-5), diluting the shWRN-specific rescue with the
shRFP/pan-essential bars. With the right sheet + shWRN-only contrast the public data reproduces
the paper exactly. The non-public `reformattedforstats.xlsx` is merely a relabeled concatenation
of these same published display numbers — not a different (cleaner) dataset.

**Status — LIFTED (increment 10).** C-MMR's `mmr_restoration` is now **kernel-recomputed**. A new
`stats:CrossedAnovaAnalysisPlan` dispatch (`numerics::crossed_two_way_anova`, group = 2-level
CL/MMR-context, crossed blocking factor = guide; distinct from the increment-8 nested dispatch)
recomputes the three contrasts; `chain/03-phase1-recompute-plans.esl` carries the three Tier-1-pinned
SampleSets + plans + `bridge_mmr_restoration` → `concl_mmr_restoration_recomputed`
(`RestorationPartiallyRescues(dMMR, WRN)`). The linked-external `wrn:mmr_restoration` ToolArtifact
is retired; `concl_mmr` (phase 5) discharges its antecedent by D54 lemma citation. The unit test
`crossed_two_way_anova_reproduces_wrn_ed10c_rescue` pins F(1,21)=1187.5 / P=5.74e-20.

### F3 — the data-mapping difficulty (why this took several wrong turns)

This was, by a wide margin, the **hardest mapping** in the whole encoding — not because the
statistics were exotic (it is an ordinary two-way ANOVA) but because *nothing about the published
artifacts told us how to wire it*, and several plausible-but-wrong wirings each produced a
confident, wrong number. Recorded here because future wet-lab recomputes will hit the same wall,
and because "the p-value reproduced" hides how much detective work the binding actually took.

The obstacle chain, in the order it bit:
1. **Display data ≠ analysis data, with no signpost.** The authors' code reads a non-public
   `NatureDataSpreadsheet_..._reformattedforstats.xlsx`; what Nature hosts is the per-figure
   *display* Source Data (MOESM12). Whether the two even contain the same numbers was unknowable
   up front — it took reproducing the result to confirm the reformatted file is just a relabeled
   concatenation, not a cleaned superset. The first conclusion ("blocked — data not public") was
   wrong, but *defensibly* wrong given the evidence then in hand.
2. **Panel-letter drift between analysis and publication.** The authors' code analyzes sheets it
   labels ED10a / **ED10b** / **ED10e**; the published figure + MOESM12 sheets are labeled ED10a /
   **ED10c** / ED10f. The reported viability p-values live in published **10c** (= the code's
   "10b"), while the sheet literally named "ED Fig 10f" is a *different*, n=3 panel. Matching by
   panel letter sends you to the wrong sheet.
3. **A decoy sheet that looks right.** "ED Fig 10f" (n=3) has the same Ch2/Ch3+5 structure and
   superficially fits, but its replicates are corrupted (duplicate-fill artifacts like
   `0.10, 9.19, 9.19`) and it is *not* what the reported p-values come from. It produces garbage
   p-values (0.80 / 0.082 / 0.011) under the correct model — which reads as "the model is wrong"
   rather than "the sheet is wrong." The right sheet (10c, n=6) is three rows further down the
   same file.
4. **Symbol→condition decoding with unlabeled sub-blocks.** MOESM12 stacks four identically-titled
   "HCT116 Ch3+5" blocks with no sgCh2-2 / sgMLH1-1 / sgMLH1-2 annotation; the figure's ∗ † ‡ §
   symbols (which the legend compares pairwise) had to be matched to blocks by cross-referencing
   the R code's condition order and the viability pattern (rescued ≈0.71 vs re-sensitized ≈0.34–0.47).
5. **Which rows: raw vs normalized.** Each block carries *both* raw counts and "value N normalized"
   rows; the analysis is on the normalized relative viability. Using raw values gives the wrong p.
6. **Which bars: the shWRN-only contrast.** The decisive step. The legend says "two-way ANOVA" and
   the code says `lm(value ~ CL + guide)`, which *reads* like "test CL across all guides" — but
   that dilutes the shWRN rescue with shRFP/pan-essential bars and gives ≈7e-5. The ∗†‡§ symbols
   mark only the **shWRN** bars, so the ANOVA is run on the shWRN1+shWRN2 subset. Only with sheet
   10c + normalized + shWRN-only does `value ~ CL + guide` land on 5.74e-20.

**The lesson for the audit chain.** Each wrong turn produced a *plausible* number (0.80; 7e-5),
not an error — so without the published target p-value to check against, any of them could have
been silently encoded as "the recompute." The binding discipline (encoding-plan §5.1: a recompute
must preserve the published claim *and* land within the quantity-class tolerance) is exactly what
rejected the wrong wirings: a one-sided rescue at p≈7e-5 "supports the claim" but misses the
reported 5.7e-20 by 15 orders of magnitude, flagging that the mapping was not yet right. Faithful
recomputation of published wet-lab statistics is therefore **gated on having both the analysis-grade
data and the exact model+subset**, and neither is reliably recoverable from a paper's display
Source Data alone — the authors' analysis code was indispensable, and even with it the
display↔analysis data mapping required reproducing the target number to confirm.

## F4 — the competition-assay ANOVA pseudoreplicates: the published p is technical, not biological

**Where:** `data/WRN_manuscript/src/WRN_stats_calcs.Rmd:35,59,83,107,…` — the authors run
`lm(value ~ is_WRN + guide) %>% anova()`, filtered to `term == 'is_WRN'`, for *every*
competition-assay figure (Fig 2b/2d/3a/3b; the crossed `value ~ CL + guide` sibling backs ED Fig
10c, see F3). `is_WRN = factor(grepl('WRN', guide))`, so **`guide` is nested in `is_WRN`** — each
shRNA/sgRNA reagent is either WRN-targeting or control. The reported P tests the `is_WRN` main
effect against the model **residual**.

**The issue.** That residual is the **within-guide, technical-replicate** variation. For KM12
(3 sgWRN + 2 control guides × 6 reps, N = 30) `is_WRN` is tested against **25 residual df** — but
the independent **biological units** for a claim about *WRN* are the **5 guides** (≈ 2–3 df), not
the 30 wells. The 6 reps per guide are technical (repeated reads of one perturbation); counting
them as independent evidence that "depleting WRN impairs viability" is **pseudoreplication** —
borrowing precision from technical replication that does not bear on biological generality,
inflating the denominator df and the significance.

**Quantified (recomputed locally, lme4; same KM12 data as `viab_KM12_sampleset`):**

| Model | Unit of inference | P (`is_WRN`) |
|---|---|---|
| `lm(value ~ is_WRN + guide)` — authors' | technical residual (25 df) | **2.74e-19** (reproduces paper's 2.7e-19) |
| `lmer(value ~ is_WRN + (1\|guide))` LRT | guide as biological random effect | **2.15e-6** |
| t-test on the 5 guide means | guide (2 df) | **2.3e-3** |

The conclusion is **robust** — WRN depletion significantly impairs MSI viability under *all three*
(p < 0.01) — but the published **2.74e-19 is a pseudoreplication artifact**, overstating the
evidence by ~13–16 orders of magnitude relative to the biologically-honest analysis.

**Internal inconsistency in the paper.** The authors *do* use the correct mixed-effects approach
elsewhere: the in-vivo xenograft (`in_vivo_KM12_analysis.R`) is analyzed with
`lmer(Volume ~ Day + (0+Day|Mouse))` — mouse as the biological random effect — yet the in-vitro
competition assays use the fixed-effects two-way ANOVA against the technical residual. The
replication-stratification choice is inconsistent across the paper's own analyses.

**Standard methodology (textbook).** Biological vs technical replicates must be stratified so
inference lands at the biological unit; pooling technical reps as independent is the textbook
definition of pseudoreplication:
- Lazic, S. E. *Experimental Design for Laboratory Biologists: Maximising Information and Improving
  Reproducibility.* Cambridge Univ. Press, 2016 — chapters on replication & nested designs.
- Blainey, P., Krzywinski, M. & Altman, N. "Points of Significance: Replication." *Nat. Methods*
  11, 879–880 (2014).
- Hurlbert, S. H. "Pseudoreplication and the design of ecological field experiments." *Ecol.
  Monogr.* 54, 187–211 (1984) — origin of the term.
- (CLSI EP05-A3 — already D52's reference for variance-component stratification.)

**How the chain models it (decision).** Represent **both**, not one:
1. **Faithful reproduction** — the authors' *declared* SAP: a `StatisticalAnalysisPlan` over the
   competition-assay `SampleSet` reproducing `lm(value ~ is_WRN + guide)` (`nested_group_anova`,
   P = 2.74e-19). This is "what the paper claimed," recomputed exactly.
2. **Alternative (preferred) SAP** — the biological-level analysis: `guide` as the biological
   replicate unit, the mixed model `lmer(value ~ is_WRN + (1|guide))` LRT (P ≈ 2.15e-6). This is
   **lifted through the R language runtime** (D55/D56), *not* reimplemented as statistics-institution
   numerics: a mixed-effects LRT (REML/optimizer-dependent) is exactly the runtime-dependent
   computation D26 §2.2 says belongs in the substrate (operationally reproducible, not
   mathematically re-checkable in-kernel) — and it is the *same* model and *same* mechanism the
   paper's own in-vivo arm uses (`concl_vivo`, the xenograft `lmer` LRT). Using it here also resolves
   the paper's internal inconsistency (mixed model in vivo, fixed-effects ANOVA in vitro): when *we*
   do the in-vitro analysis correctly, we use the same tool.

**Implemented (this is now on the chain, not a follow-up).** The R program
[`programs/invivo/km12-competition-lme4-program.json`](../programs/invivo/km12-competition-lme4-program.json) runs the
LRT on [`programs/invivo/km12-competition-input.json`](../programs/invivo/km12-competition-input.json) (the ED Fig 3b
KM12 data) and commits `wrn:viab_KM12_bio_lme4:result` carrying an `IsDerivedAs` witness over
`onco:ViabilityDependenceAtBiologicalUnit("WRN","KM12")`. [`chain/06-phase1-biological-sap.esl`](../chain/06-phase1-biological-sap.esl)
holds `concl_viab_KM12_biological` (the D54 reasoning sentence that discharges that witness) and
`wrn:viab_KM12_dual_sap` (a declared resource recording F4 itself: the technical-stratum warrant
`wrn:viab_KM12_plan` at 2.74e-19 vs the biological-stratum warrant at ≈2.15e-6, conclusion robust).
Like `concl_vivo`, the witness exists only after the R program runs, so the warrant is exercised by
the live demo ([`demo/wrn-helicase/run.sh`](../../../../demo/wrn-helicase/run.sh) Step 3b), not the
in-process recompute tests.

The two SAPs are linked on the chain: the alternative **refines / annotates** the published claim, so
both the reproduced number *and* the methodological caveat are first-class, queryable facts — the
"audit chain surfaces what prose hides" demonstration, here on the *model* rather than the data
(cf. F1, which did it on a sample size).

> *Design note.* An earlier attempt lifted this as a deterministic between-guide nested ANOVA
> (`nested_group_anova`'s F(1, k−2) sibling) inside the statistics institution. That was reverted:
> it computes a *coarser proxy* (the pooled t-test on guide means, ≈2.3e-3) rather than the mixed
> model we actually ran, and it grows the institution with a test whose principled form (REML) is
> not deterministically re-checkable anyway. The R-runtime path uses the real model and reuses
> existing infrastructure. (A deterministic biological-stratum primitive in the institution may still
> be worth having later as a *second*, kernel-recomputed cross-check — but it is not the warrant.)

## F1 — Spearman sample size: paper says n=54, data gives n=51

**Where:** main text (Extended Data Fig. 2c); `generate_figs.Rmd:329`
`with(comb_data %>% filter(MSI), print_spearman_corr(ms_deletions_normed, avg_WRN_dep))`.

**Forensics (against the pinned snapshot):**
- The `MSI` flag is `CCLE_MSI == 'MSI'` (`generate_figs.Rmd:54`) — 99 MSI lines.
- `avg_WRN_dep` is non-NA for exactly **51** of them (NA for the 48 lacking *both* screens; no coercion casualties; all 51 also have `ms_deletions_normed`). → Spearman n = **51**.
- Independent cross-check against the **raw** published matrices: WRN values exist for **32** MSI lines in Achilles (CERES) and **34** in DRIVE (DEMETER2); **union = 51**, matching the curated Supplementary Table 1 exactly (0 lines dropped in curation).
- **No published artifact yields 54** — neither the curated table nor the raw screen matrices.

**Conclusion:** `n = 54` is a paper-internal inconsistency — most plausibly a stale count from
an earlier analysis snapshot (pre-release DepMap version / pre-QC) where 3 additional MSI
lines still carried a WRN dependency score. **rho = −0.74 is robust to the difference**
(reproduced exactly), so the correlation's conclusion is unaffected and the error escaped review.

**Significance:** benign (conclusion intact) but real and citable — and it appeared on the
*first* recompute. Exactly the "audit chain surfaces what prose hides" demonstration: the
qualitative claim + effect size hold within tolerance, while the reported sample size diverges
and is flagged. When encoded, node `D-REFINE` should carry the recomputed `n = 51` with a
`refutes`/annotation pointer recording the paper's `n = 54` and this provenance trail.

## F2 — the dual `NA`/`NaN` sentinel, and the "measured cohort" definition

**Not a paper discrepancy** — the paper's biomarker numbers reproduce exactly (PPV = 27/37 =
0.73, sensitivity = 27/27 = 1.00). This entry records a **data-hygiene gotcha** that surfaced
while recomputing D-BIOM and momentarily produced a wrong intermediate count (54 instead of
37). It is logged so anyone re-deriving from these slices avoids the same trap.

**The gotcha.** `wrn_supplementary_table_1.csv` uses **two** missing-data spellings:
- `"NA"` — R's default, throughout the table;
- `"NaN"` — specifically in the *computed float* columns (`avg_WRN_dep`, `ms_deletions_normed`),
  written by R when a derived value had no inputs.

A null filter that strips only `""`/`"NA"` (the obvious one) silently treats `"NaN"` cells as
*present*, inflating any cohort defined by "has a value."

**Where it bit.** Counting MSI lines in common-MSI lineages "with a WRN dependency value":
- naive (`NA` only) → **54** (includes 17 lines whose `avg_WRN_dep = "NaN"`);
- correct (`NA` + `NaN`) → **37**.

The 17 inflation cases (OC316, TGBC11TKB, SNU520, SNUC5, RL952, HEC108, COLO684, SNU1040,
SNU175, OC314, COLO704, IGROV1, JHUEM2, DOV13, SNUC2B, GP5D, HEC1) are MSI cell lines that were
**never in either screen** (`avg_WRN_dep="NaN"`, `CRISPR_WRN_CERES=NA`, `DRIVE_WRN_D2=NA`). They
have no dependency value and must drop out of any dependency analysis.

**The cohort definition (use this).** The analyzable MSI cohort = lines with **≥1 screen
measurement**, equivalently any of (all yield 37 / 91 MSS):
- `is_WRN_dep != NA`;
- `CRISPR_WRN_CERES` present `OR` `DRIVE_WRN_D2` present;
- `avg_WRN_dep` parses as a real number (rejects both `NA` and `NaN`).

This is the cohort the validated `wrn_dep_sampleset` (37 MSI / 91 MSS) already uses, so C-WRN
and D-BIOM are both correct as encoded.

**Relation to F1.** F1's `54` is unrelated — it is the paper's all-lineage Spearman n, which
matches *neither* sentinel interpretation (all-lineage MSI with `avg_WRN_dep`: 51 correct / 99
naive) nor the common-lineage restriction (37). The two `54`s are a coincidence, not a shared
cause. F1 remains a genuine paper-internal inconsistency; F2 is purely a re-derivation hazard.

**Robustness note.** The `wrn_recq_sampleset` (32 MSI / 413 MSS) is immune by construction — it
reads the Achilles gene-effect *matrix* directly and parses each cell with `float()`, which
rejects `NA` and `NaN` identically.

## F5 — D-DIFF (Achilles): WRN is the top differential dependency, reproduced live through the substrate

**Where:** `programs/differential-dependency/dd-achilles-limma-program.json` (the wrapped-R D56 warrant) over the pinned
`achilles_18Q4_gene_effect.csv` (D53 `PinnedExternalFile`, sha256 `2186669d…2eb68b`, matches
`MANIFEST.md`), joined to MSI labels across two more pinned files via the multi-input path
(matrix `DepMap_ID` → `sample_info` `CCLE_name` → Supp Table 1 `CCLE_MSI`).

**The recipe (reproduces the paper exactly).** limma `lmFit`/`eBayes` moderated-*t* of CERES
gene-effect, MSI vs MSS (445 lines: 32 MSI / 413 MSS), over 17,634 genes; rank the
MSI-preferential genes (logFC < 0, more essential in MSI) by P. **WRN comes out rank 1 of 10,507,
adj.P (Q) = 4.81e-24** — the paper's reported **Q = 4.8e-24**.

**Why this is a D56 wrapped-R warrant, not native (the methodological point).** A crude Welch
*t*-test on the same join ranks WRN **8th** (q ≈ 0.004) — WRN has the *largest* effect size
(meanDiff −0.368) but a noisier per-gene variance. limma's empirical-Bayes variance shrinkage,
which borrows strength across genes, is exactly what rewards WRN's large, consistent effect and
lifts it to rank 1 with the headline Q. Re-implementing eBayes natively (D52) would be
re-deriving a non-trivial statistical method; instead we **wrap the pinned tool** (D53 §6) and
make the warrant re-checkable by the `content_hash` of the inputs + the image digest.

**End-to-end through Eigenius.** `eig run` dispatches the program; the kernel resolves the two
auxiliary inputs from `runtime:additional_inputs`; the substrate materializes + content-verifies
all three; a DooD-spawned R worker reads them via `r_eigon_materialized_path` and runs limma; the
result commits as `wrn:dd_achilles:result` (Q = 4.81e-24, rank 1,
`canonical_proposition = TopDifferentialDependency("WRN","Achilles_MSI")`) under a ProgramTrace →
IsDerivedAs witness. This lifts D-DIFF from **linked-external** to **reproduced-external**.

**D-DIFF family (the same warrant across screens + MSI callers), all run live through the substrate:**
- **DRIVE (RNAi/DEMETER2, ED Fig 1b)** — `programs/differential-dependency/dd-drive-limma-program.json` over the 59 MB
  `drive_D2_DRIVE_gene_dep_scores.csv` (genes × cell-lines; columns ARE CCLE_IDs, so the MSI join is
  direct to Supp Table 1 — no sample_info bridge). **WRN rank 1 of 4,591, Q = 1.46e-45** — the
  paper's **1.5e-45**. WRN is #1 in *both* the CRISPR (Achilles) and RNAi (DRIVE) screens; commits
  `wrn:dd_drive:result` → `TopDifferentialDependency("WRN","DRIVE_MSI")`.
- **GDSC PCR-MSI robustness (ED Fig 1b)** — `programs/differential-dependency/dd-gdsc-limma-program.json`, the Achilles
  D-DIFF re-run grouped by the orthogonal GDSC PCR panel (MSI-H vs MSS/MSI-L; only 19 MSI-H lines)
  instead of the NGS CCLE_MSI calls. Same pinned matrix + join, only the label column differs.
  **WRN STILL rank 1 of 9,714, Q = 4.66e-20** — the headline does not depend on the MSI calling
  method; commits `wrn:dd_gdsc:result` → `TopDifferentialDependency("WRN","Achilles_GDSC_MSI")`.

## F6 — C-MECH (Fig 3a GSEA): WRN-KO transcriptional arrest signature, reproduced live through the substrate

**Where:** `programs/mechanism/gsea-mech-program.json` (the wrapped-R D56 warrant) over two pinned D53
`PinnedExternalFile`s — the WRN-KO RNA-seq counts `GSE126464_STAR_Gene_Counts.csv.gz`
(genes × 12 samples, gzipped; sha256 `e66c70f3…876daa5`, primary input) and the MSigDB Hallmark
v6.2 gene-set file `h.all.v6.2.symbols.gmt` (sha256 `0ee07a4a…22146b`, carried as
`runtime:additional_inputs`). The `.gmt` is pinned under the D53 **Collection** layout profile
(`hallmark_gmt_schema`): ragged rows, each a named gene set over `onco:Gene`.

**The recipe (reproduces Fig 3a).** limma-voom moderated-*t* of the STAR counts, WRN-KO vs control
across both lines (`~ cell + cond`, last coef = the KO effect; filter `rowSums(M >= 10) >= 6`);
rank genes by *t*; `fgsea` against the 50 Hallmark sets (minSize 15, maxSize 500, seed 1).
**G2M_CHECKPOINT NES −3.53, E2F_TARGETS NES −3.44 (padj 2.5e-49)** depleted; **P53_PATHWAY
NES +2.89 (padj 9.9e-21)** and **APOPTOSIS NES +1.78** activated — the transcriptional signature of
cell-cycle arrest the paper reports in Fig 3a.

**Why wrapped-R, not native.** voom precision weights + empirical-Bayes moderation + the fgsea
enrichment statistic are a multi-stage pipeline; re-deriving them natively (D52) would be
re-implementing two established methods. We **wrap the pinned tools** (D53 §6) and make the warrant
re-checkable by the `content_hash` of both inputs + the image digest.

**End-to-end through Eigenius.** `eig run` dispatches the program; the kernel resolves the `.gmt`
from `runtime:additional_inputs`; the substrate materializes + content-verifies both files; a
DooD-spawned R worker reads them via `r_eigon_materialized_path` and runs limma-voom → fgsea; the
result commits as `wrn:gsea_mech:result` (the NES/padj measures,
`canonical_proposition = CausesCellCycleArrest("WRN","MSI")`) under a ProgramTrace → IsDerivedAs
witness. It is a *transcriptional* corroboration of `concl_mech` alongside the FACS-ANOVA evidence,
and the first consumer of the D53 Collection profile + the multi-input path for a ragged gene-set
file. This lifts C-MECH/GSEA from **linked-external** to **reproduced-external**.

## F7 — C-MECH (ED Fig 5 IF): WRN-KO activates the p53 response, p53-status-gated, reproduced live through the substrate

**Where:** `programs/mechanism/if-ed5-lsmeans-program.json` (the wrapped-R D56 `emmeans` warrant) over the tidy
per-cell immunofluorescence slice `if_ed5_long.csv` (175,974 cells; a D53 file-backed `PinnedExternalFile`
with a `LongTable` schema, sha256 `8d26fbb8…c86c519`), derived from the authors' ED Fig 5 source workbook
`wrn_sourcedata_EDFig5_MOESM8.xlsx` (panels 5b/5d/5f) by the vendored `extract/if-ed5-extract.R`.
Genotype (CCLE_MSI + TP53_status) is joined from Supp Table 1 (the additional input).

**The recipe.** Per readout, the WRN-KO vs control least-squares-means contrast on log-intensity,
adjusting for `cell_line`, computed over the **MSI + TP53-proficient** stratum. **phospho-p53(S15) rises
logFC = +0.155 (t = 17.7, p = 7.1e-69); p21 rises logFC = +0.310 (p ≈ 0)** — WRN loss activates the p53
DNA-damage response, MSI-selectively, exactly as the paper reports.

**The refinement (why this is richer than a bare reproduction).** Naively pooling all lines gives
*inconsistent* signs — and the resolution is biological, not a wiring bug. p21 is a p53 transcriptional
target, so it can only be induced where p53 is intact. The p53-null MSI line **KM12** fails to induce p21
(`p21_null_logfc` ≈ **−0.074**, emitted as a measurement) even though it is MSI. Stratifying by
`TP53_status` recovers a clean, coherent pattern: p-p53/p21 rise on WRN-KO in MSI + p53-proficient lines
(OVK18, SW48); the p53-null lines do not mount the p21 arm. This is the paper's own point — the upstream
lesion (DSBs) and the lethality are **p53-independent**, while the transcriptional p21 readout is
p53-dependent — recovered directly from the per-cell source data rather than asserted.

**End-to-end through Eigenius.** `eig run` dispatches the program; the kernel resolves Supp Table 1 from
`runtime:additional_inputs`; the substrate materializes + content-verifies both files; a DooD-spawned R
worker (image now bakes `emmeans`) runs the lsmeans contrasts; the result commits as `wrn:if_ed5:result`
(the five logFC/p-value measures, `canonical_proposition = ActivatesP53Response("WRN","MSI")`, set when
BOTH p-p53 and p21 rise significantly) under a ProgramTrace → IsDerivedAs witness. The
`concl_p53_activation` ReasoningSentence (chain/08-phase3-invivo-mechanism.esl) discharges that witness — a reproduced-external
corroboration of `concl_mech`. This lifts the p53-activation arm of C-MECH from **linked-external** to
**reproduced-external**.

**Remaining:** the DSB-marker readouts (γH2AX intensity ED 6c; γH2AX/53BP1/pATM/Chk2 foci *counts*
ED 6/7) are the next microscopy lift (close-out #4) — count data over the same per-cell file-backed
SampleSet shape, via D52-native ANOVA / a count-model warrant.

## F8 — C-MECH (ED Fig 6f/6h): WRN-KO induces 53BP1 DSB foci MSI-selectively, reproduced live through the substrate

**Where:** `programs/mechanism/foci-ed6-program.json` (the wrapped-R D56 warrant) over the tidy per-cell foci slice
`foci_53bp1_long.csv` (39,249 cells; a D53 file-backed `PinnedExternalFile` with a `LongTable` schema,
sha256 `1ba6dc6f…4e9b83`), derived from the authors' ED Fig 6 source workbook
`wrn_sourcedata_EDFig6_MOESM9.xlsx` (panels 6f/6h — Apple-53BP1-trunc foci) by the vendored
`extract/foci-ed6-extract.R`. MSI genotype is joined from Supp Table 1 (the additional input).

**The recipe.** Panels 6f (SW620 MSS, KM12 MSI) and 6h (ES2 MSS, OVK18 MSI) together span both MSI
strata, so MSI-selectivity is a single interaction model: `foci ~ cell_line + condition*MSI`. The
**condition×MSI interaction** is the MSI-selective extra DSB induction on WRN loss — **+1.82 foci,
t = 25.5, p ≈ 2.6e-142**. Descriptively, WRN-KO multiplies mean foci by **2.08× in MSI lines** vs only
**1.04× in MSS lines** — WRN loss induces DSBs selectively in the MSI background, exactly the paper's claim.

**Why wrapped-R, not the native institution.** The claim is the *MSI-selectivity*, which is an
interaction (condition×MSI) over per-cell counts — not the additive `value ~ is_WRN + guide` two-way
ANOVA the statistics institution dispatches (used for the cell-cycle/apoptosis recomputes). The
interaction model is the faithful shape, so this rides the same proven D53 file-backed SampleSet +
wrapped-R path as the p53 IF warrant (F7).

**End-to-end through Eigenius.** `eig run` dispatches the program; the substrate materializes +
content-verifies the foci slice + Supp Table 1; a DooD-spawned R worker fits the interaction lm; the
result commits as `wrn:foci_dsb:result` (the interaction estimate/p + per-stratum fold-changes,
`canonical_proposition = CausesDSBs("WRN","MSI")`, set when the interaction is positive and significant)
under a ProgramTrace → IsDerivedAs witness. The `concl_dsb_foci` ReasoningSentence (chain/08-phase3-invivo-mechanism.esl)
discharges it — a reproduced-external corroboration of `concl_dsb`. This lifts the **53BP1 arm** of
CausesDSBs from linked-external to reproduced-external; the broader marker panel (γH2AX intensity ED 6c,
pATM(S1981)/Chk2(T68) ED 6/7) remains linked corroboration in `mech_dsb`, the same-shape backlog.

## F9 — ED Fig 9a: WRN's MSI-dependence is intrinsic, not a paralogue-co-loss confound, reproduced live over the 1.6 GB DepMap rds

**Where:** `programs/specificity/paralog-ed9a-program.json` (the wrapped-R D56 warrant) over the authors' 1.6 GB
DepMap 18Q4 omics bundle `DepMap_18Q4_data.rds` (D53 `PinnedExternalFile`, sha256 `14e82c39…9c85ed`,
read in-worker via `readRDS` — the **large multi-schema container** path: a named list of cell-line×gene
matrices DRIVE/CRISPR/GE/CN/MUT_*/RPPA). `avg_WRN_dep` + `CCLE_MSI` come from Supp Table 1 (the
additional input).

**The recipe (the authors' ED Fig 9a alternative-hypothesis test).** A gene is "lost" in a line if it
carries a damaging mutation (`MUT_DAM`), or `CN < −1` (log2 relative copy number), or `GE < 1` (log2 TPM)
— the authors' thresholds. For each RECQ paralogue, fit `lm(avg_WRN_dep ~ MSI + gene_loss)` and read the
MSI coefficient. **Baseline `lm(avg_WRN_dep ~ MSI)`: MSI β = −0.667, p = 4.4e-60.** Controlling for each
paralogue's co-loss the MSI coefficient barely moves and stays overwhelmingly significant:

| Control for loss of | MSI β | MSI p |
|---|---|---|
| RECQL | −0.670 | 1.0e-58 |
| BLM | −0.678 | 2.0e-61 |
| RECQL4 | −0.694 | 6.3e-64 |
| RECQL5 | −0.703 | 3.3e-64 |

WRN's MSI-selective dependence is **intrinsic to MSI**, not a confound of paralogue co-deletion — the
explicitly tested-and-rejected alternative.

**Why this is the large-container D53 path.** The rds is 1.6 GB and cannot inline; it is content-addressed
+ materialized + verified like the CSV matrices, but consumed *member-wise* (`readRDS` then index the
DRIVE/CRISPR/GE/CN/MUT matrices) rather than as one tabular layout — the `depmap_rds_schema` documents the
container's members with an `Other` layout. The worker `readRDS` of the full bundle completes in ~5 s.

**End-to-end through Eigenius.** `eig run` dispatches; the substrate materializes + content-verifies the
1.6 GB rds + Supp Table 1; a DooD-spawned R worker reads the rds and fits the models; the result commits as
`wrn:paralog_ctrl:result` (baseline + controlled MSI β/p, `canonical_proposition =
NotExplainedByParalogLoss("WRN","MSI")`, set when the MSI coefficient stays significant + same-signed
across all paralogue controls) under a ProgramTrace → IsDerivedAs witness, discharged by `concl_paralog`
(chain/08-phase3-invivo-mechanism.esl). This closes the omics-analysis frontier: every analysis class in the paper — native
recompute, mixed models (lme4), large-matrix limma, GSEA (fgsea), per-cell IF/foci (emmeans/interaction
lm), and now the large multi-schema rds container — runs live through the platform.

## F14 — ED Fig 6c: WRN-KO raises γH2AX (the canonical DSB marker) MSI-selectively, reproduced live

γH2AX is named *before* 53BP1 in the mechanism text ("substantially increased γH2AX and 53BP1 foci,
markers of DSB"); ED 6c is its **published quantification** — nuclear γH2AX staining **intensity** per cell.
The tidy slice `gh2ax_intensity_long.csv` (32,882 cells, ES2 MSS + OVK18 MSI; ED 6c via
`extract/gh2ax-ed6c-extract.R`) runs through a D56 wrapped-R `emmeans` interaction on **log10** intensity
(`programs/mechanism/gh2ax-intensity-program.json`). Reproduces the paper's reported statistic essentially
exactly: mean log10 fold-change **0.055 (ES2)** / **0.144 (OVK18)** vs the paper's 0.055 / 0.147, and the
OVK18-vs-ES2 contrast-of-LSM **P ≈ 2.7e-39 < 2×10⁻¹⁶** (paper: P<2×10⁻¹⁶). Commits `wrn:gh2ax:result`
(`canonical_proposition = CausesDSBs("WRN","MSI")`, set on a positive, significant MSI-vs-MSS interaction),
discharged by `concl_dsb_gh2ax`. The log base mattered: the paper uses log10, not natural log.

## F14b — ED Fig 6a/6d: WRN-KO raises γH2AX foci MSI-selectively — but only if pan-nuclear cells are counted

ED 6a (colon) / 6d (ovarian) give the discrete per-cell γH2AX **foci** counts. The crucial subtlety: γH2AX
is a *diffuse* marker that **saturates (goes pan-nuclear)** at high damage, and pan-nuclear cells have
uncountable, blank foci. Those cells are the most-damaged ones and are MSI-enriched — on WRN loss the
pan-nuclear fraction jumps (KM12 **13%→50%**, SW48 **1%→21%**) while MSS lines stay flat. **Dropping** them
(the naive parse) discards the signal and yields a *spurious decrease* in MSI (interaction −0.43). Counting
them at a saturation ceiling (`extract/gh2ax-ed6ad-extract.R`) recovers the true MSI-selective induction:
interaction **+7.3 (p≈0)**, foci **×3.4 MSI vs ×1.0 MSS** (`programs/mechanism/gh2ax-foci-program.json` →
`wrn:gh2ax_foci:result` → `CausesDSBs`, `concl_dsb_gh2ax_foci`). This is *why* the authors quantify γH2AX
primarily by intensity (F14), and why this panel exists alongside it. A documented modeling decision, not a
silent one.

## F15 — ED Fig 7b/7d: WRN-KO activates pATM(S1981) DDR signaling MSI-selectively (the bridge to p53)

pATM(S1981) autophosphorylation reports activation of the apical ATM DSB-response kinase — the signaling
step the paper uses to connect DSBs to p53 ("DSB responses known to activate p53"). Unlike γH2AX, pATM
forms **discrete** foci even at high damage (94–100% countable, pan-nuclear rare), so foci is the valid,
unbiased readout. The slice `patm_foci_long.csv` (191,241 cells, colon SW620/KM12/SW48 + ovarian
ES2/OVK18; ED 7b/7d via `extract/patm-ed7-extract.R`) runs through a wrapped-R interaction lm
(`programs/mechanism/patm-foci-program.json`): foci **×1.74 MSI vs ×1.11 MSS**, interaction **p≈0**.
Commits `wrn:patm:result` → the **new** `onco:ActivatesDSBResponse("WRN","MSI")` (`concl_ddr_signaling`) —
a mechanism proposition previously absent from the chain. The companion Chk2(T68) readout (ED 7e) is a
western blot with no per-cell numeric source data, so it stays linked in `mech_dsb`.
