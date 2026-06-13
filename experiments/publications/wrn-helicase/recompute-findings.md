# WRN encoding — recompute findings log

Discrepancies surfaced by recomputing the paper's claims against the pinned public-data
snapshot ([data/MANIFEST.md](data/MANIFEST.md)). Each is the discipline working: a gap
between the published number and what the data re-yields. Per [encoding-plan §5.1](encoding-plan.md),
a divergence is a recorded finding, not a silent pass.

| # | Claim (paper) | Recomputed | Class | Verdict |
|---|---|---|---|---|
| F1 | Spearman WRN-dep ~ #MS-deletions, all MSI: rho = −0.74, **n = 54** | rho = **−0.74** (matches), **n = 51** | B (effect) ✓ / **A (count) ✗** | **Discrepancy — benign** |
| F2 | Biomarker (common-MSI lineages): PPV = 0.73 (27/37), sensitivity = 1.00 (27/27) | PPV = **0.73 (27/37)**, sensitivity = **1.00 (27/27)** | A ✓ | **Confirms paper — provenance gotcha noted** |

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
