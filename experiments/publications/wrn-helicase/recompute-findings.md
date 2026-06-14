# WRN encoding — recompute findings log

Discrepancies surfaced by recomputing the paper's claims against the pinned public-data
snapshot ([data/MANIFEST.md](data/MANIFEST.md)). Each is the discipline working: a gap
between the published number and what the data re-yields. Per [encoding-plan §5.1](encoding-plan.md),
a divergence is a recorded finding, not a silent pass.

| # | Claim (paper) | Recomputed | Class | Verdict |
|---|---|---|---|---|
| F1 | Spearman WRN-dep ~ #MS-deletions, all MSI: rho = −0.74, **n = 54** | rho = **−0.74** (matches), **n = 51** | B (effect) ✓ / **A (count) ✗** | **Discrepancy — benign** |
| F2 | Biomarker (common-MSI lineages): PPV = 0.73 (27/37), sensitivity = 1.00 (27/27) | PPV = **0.73 (27/37)**, sensitivity = **1.00 (27/27)** | A ✓ | **Confirms paper — provenance gotcha noted** |
| F3 | ED Fig 10c MMR-restoration (C-MMR): two-way ANOVA P = 5.7e-20 / 3.3e-12 / 1.6e-16 | **5.74e-20 / 3.26e-12 / 1.56e-16** (exact, from public MOESM12) | A ✓ | **Reproduces paper exactly** |
| F4 | C-VAL / C-MMR competition assays: two-way ANOVA `value ~ is_WRN + guide` (e.g. KM12 P = 2.7e-19) | **2.74e-19 reproduced** — but tests the *technical* residual; biological-unit P ≈ **2e-3 – 2e-6** | Methodology (design) | **Reproduces paper exactly; flags pseudoreplication; conclusion robust** |

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
recomputes the three contrasts; `wrn-phase1-recompute-plans.esl` carries the three Tier-1-pinned
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
[`programs/km12-competition-lme4-program.json`](programs/km12-competition-lme4-program.json) runs the
LRT on [`programs/km12-competition-input.json`](programs/km12-competition-input.json) (the ED Fig 3b
KM12 data) and commits `wrn:viab_KM12_bio_lme4:result` carrying an `IsDerivedAs` witness over
`onco:ViabilityDependenceAtBiologicalUnit("WRN","KM12")`. [`wrn-phase1-biological-sap.esl`](wrn-phase1-biological-sap.esl)
holds `concl_viab_KM12_biological` (the D54 reasoning sentence that discharges that witness) and
`wrn:viab_KM12_dual_sap` (a declared resource recording F4 itself: the technical-stratum warrant
`wrn:viab_KM12_plan` at 2.74e-19 vs the biological-stratum warrant at ≈2.15e-6, conclusion robust).
Like `concl_vivo`, the witness exists only after the R program runs, so the warrant is exercised by
the live demo ([`demo/wrn-helicase/run.sh`](../../../demo/wrn-helicase/run.sh) Step 3b), not the
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
