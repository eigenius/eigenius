# WRN encoding — data provenance manifest (Phase 0)

Provenance for the vendored Phase-1 data slices. The slices themselves live under
`data/slices/` and are **gitignored** (large); this manifest is the committed,
content-addressed record. Decision §9.1 of the [encoding plan](../docs/01-encoding-plan.md):
*fetch the minimal Phase-1 slices, checksum them, link the rest.*

Fetched 2026-06-12. Figshare `supplied_md5` verified on download (all OK); `sha256`
is our content address of the local copy.

This file is the human-readable narrative. The **machine-readable** counterpart —
one row per artifact with origin URL + checksums — is [`sources.tsv`](sources.tsv),
the single source of truth for [`fetch.sh`](fetch.sh), which downloads every
input from its public origin, verifies it, and runs the `extract/` derivers:

```bash
bash fetch.sh            # fetch + derive everything, then verify
bash fetch.sh --check    # verify the present copies; never download
```

## Vendored slices (`data/slices/`)

### Dependency matrices (the differential-dependency inputs)

| File | Source | figshare md5 (verified) | sha256 | Size | Used for |
|---|---|---|---|---|---|
| `achilles_18Q4_gene_effect.csv` | DepMap Achilles 18Q4, figshare art. **7270880**, file `gene_effect.csv` → `ndownloader.figshare.com/files/13396070` | `30f243486c3370d3e5cc6f8ef57b90b3` | `2186669d…2eb68b` | 187 MB | **O-ACHILLES** — CRISPR/CERES gene-effect. *cell lines (DepMap_ID, rows) × 17,634 genes ("SYMBOL (ENTREZ)", cols)*; `WRN (7486)` present. D-DIFF, RecQ, biomarker, aggregate dep. |
| `drive_D2_DRIVE_gene_dep_scores.csv` | DEMETER2, figshare art. **6025238**, file `D2_DRIVE_gene_dep_scores.csv` → `ndownloader.figshare.com/files/11489693` | `69b13ed329a027cad2d28166e1af20b0` | `3f863c29…38254b` | 59 MB | **O-DRIVE** — RNAi/DEMETER2 gene-dependency. *genes (rows) × 398 cell lines (CCLE_name, cols)*; `WRN (7486)` present. D-DIFF, RecQ, aggregate dep. |
| `achilles_18Q4_sample_info.csv` | figshare art. **7270880**, file `sample_info.csv` → `ndownloader.figshare.com/files/13396100` | `96167950d09e6aa1c9184eb61af5c4b2` | `c5778e66…fbdb498` | 63 KB | Cell-line ID bridge: maps `DepMap_ID` ↔ `CCLE_name` (the two screens use different conventions; cols incl `DepMap_ID`, `CCLE_name`, `primary_tissue`, `aliases`). |

### Cell-line annotation backbone

| File | Source | sha256 | Size | Used for |
|---|---|---|---|---|
| `wrn_supplementary_table_1.xlsx` | This paper's **Supplementary Table 1** (NIHMS1522798 supplement; in `references/publications/WRN-Helicase-Supplements/`) | `1a05d612…4c4c7b2` | 246 KB | **O-MSI + the Phase-1 pivot table.** |
| `wrn_supplementary_table_1.csv` | Derived from the `.xlsx` via a stdlib `zipfile`+`xml.etree` parser (first sheet) | `eebd4602…7243f2` | — | Machine-readable form. **1,415 cell lines × 37 cols.** Key cols: `CCLE_ID`, `Lineage`, `GDSC_MSI` (PCR), `CCLE_MSI` (NGS), `DRIVE_WRN_D2`, `CRISPR_WRN_CERES`, `avg_WRN_dep`, `is_WRN_dep`, `TP53_status`, `common_MSI_lineage`, `ms_deletions_normed`, `frac_deletions_in_ms_regions`, `MMR_loss`(+per-gene MLH1/MSH2/MSH6/PMS2 mut/deletion/loss/unexpressed). MSI labels (the D-DIFF grouping), WRN dep per screen + avg, mutator load, MMR/TP53 status. |

**Total dependency/annotation slices: ~235 MB.** These four support the entire Phase-1 computational-discovery spine (`H1 → D-DIFF → C-WRN → D-RECQ/D-BIOM/D-REFINE`).

### RNA-seq (Phase 4 mechanism — GSEA, fetched 2026-06-13)

| File | Source | sha256 | Size | Used for |
|---|---|---|---|---|
| `GSE126464_STAR_Gene_Counts.csv.gz` | GEO **GSE126464**, `ftp.ncbi.nlm.nih.gov/geo/series/GSE126nnn/GSE126464/suppl/` | `e66c70f3…76daa5` | 461 KB | **O-MSEQ** — WRN-KO RNA-seq STAR gene counts. genes × 12 samples (OVK18, SW48 × {sgCh2-2, sgWRN2, sgWRN3} × {A, B} reps). C-MECH GSEA (Fig 3a). |
| `GSE126464_Cuff_Gene_Counts.csv.gz` | same | `c136488f…904eaa` | 888 KB | Cufflinks gene counts (alternative quantification). |
| `h.all.v6.2.symbols.gmt` | MSigDB [43], `data.broadinstitute.org/gsea-msigdb/msigdb/release/6.2/` | `0ee07a4a…22146b` | 48 KB | **O-HALLMARK** — Hallmark collection (50 sets, gene symbols), v6.2 (July 2018, era-matching the 18Q4 analysis). The gene-set definitions GSEA tests the RNA-seq DE ranking against (C-MECH, Fig 3a). Confirmed contains P53_PATHWAY/E2F_TARGETS/G2M_CHECKPOINT/APOPTOSIS. |

### DepMap omics + dependency bundle (vendored 2026-06-13)

| File | Source | md5 (verified) | Size | Used for |
|---|---|---|---|---|
| `large/DepMap_18Q4_data.rds` | figshare **7712756**.v1 → `ndownloader.figshare.com/files/14357999` | `f9e62e63bbc58ada5fc1f2d0534d08c5` (matches figshare `supplied_md5`/`computed_md5`; size 1,600,292,535 B exact) | 1.6 GB | **O-WRNFIG** — the authors' curated `dat` list (cell-lines × genes, except `MUT`). Needed for the **omics** analyses not covered by the vendored gene-effect slices: Ext Data 9a (paralog co-loss linear models) + 9b (POLE), and any feature-matrix work. |

The rds is an **R serialization** (a named list of matrices) — not readable by the Python extraction pipeline directly; consuming it needs R (`readRDS`) or `pyreadr`, i.e. an R conversion step (or the future D53 external-execution path). Contents: `DRIVE` (DEMETER2 dep), `CRISPR` (CERES dep), `GE` (log2 TPM expression), `CN` (log2 relative copy number), `MUT_HOT`/`MUT_DAM`/`MUT_OTHER` (binary mutation matrices), `MUT` (full mutation calls dataframe), `RPPA` (protein abundance).

### Wet-lab Source Data (Nature per-figure XLSX, fetched 2026-06-13)

The paper's per-figure Source Data, from `static-content.springer.com/esm/art%3A10.1038%2Fs41586-019-1102-x/MediaObjects/41586_2019_1102_MOESM{n}_ESM.xlsx`. These hold the per-replicate wet-lab values behind the Phase-2–5 readouts (the springer static-content URLs are directly fetchable; PMC does not host them).

| File (in `slices/`) | MOESM | sha256 | Backs (encoding node) |
|---|---|---|---|
| `wrn_sourcedata_Fig2_MOESM3.xlsx` | 3 | `e9c006d1…60c89e` | Fig 2: competition (2a), sgWRN-EIJ rescue (2c → `va_rescue_*`), xenograft (2d → `vivo_xenograft`/`vivo_seed_control`), organoid (2f,g) |
| `wrn_sourcedata_Fig3_MOESM4.xlsx` | 4 | `a0a6629b…3612d8` | Fig 3: GSEA (3a), p53-S15 IF contrast (3c) |
| `wrn_sourcedata_Fig4_MOESM5.xlsx` | 5 | `90a7bc2d…a7b63b6de` | Fig 4: γH2AX (4a,c), FISH (4d,e → `fish_readout`), MMR restoration (4f → `mmr_restoration`) |
| `wrn_sourcedata_EDFig3_MOESM6.xlsx` | 6 | `506d7ac0…41fd6` | **ED Fig 3b competition assay** (→ `va_competition`; two-way ANOVA) + clonogenic (3d) |
| `wrn_sourcedata_EDFig4_MOESM7.xlsx` | 7 | `bba867f2…5178549` | ED Fig 4b cell-cycle, 4c/d apoptosis (two-way ANOVA) |
| `wrn_sourcedata_EDFig5_MOESM8.xlsx` | 8 | `2b9272d8…28cc1f` | ED Fig 5 p53-S15 / p21 IF (lsmeans contrasts) |
| `wrn_sourcedata_EDFig6_MOESM9.xlsx` | 9 | `6093a494…88cb96` | ED Fig 6 γH2AX/53BP1 foci (→ `mech_dsb`) |
| `wrn_sourcedata_EDFig7_MOESM10.xlsx` | 10 | `0a374326…5e37f6` | ED Fig 7 pATM(S1981)/Chk2(T68) (→ `mech_dsb`) |
| `wrn_sourcedata_EDFig8_MOESM11.xlsx` | 11 | `a8b533a5…c1c47a` | ED Fig 8 FISH / WRN localization |
| `wrn_sourcedata_EDFig10_MOESM12.xlsx` | 12 | `3fc08eba…cdb33c` | ED Fig 10 MMR-restoration viability/clonogenic (→ `mmr_restoration`) |

**Derived tidy slices (reshaped from the source `.xlsx`, pinned + run as D53 file-backed SampleSets):**
- `if_ed5_long.csv` (sha256 `8d26fbb8…c86c519`, 175,974 rows) — ED Fig 5b/d/f per-cell p-p53(S15)/p21 IF intensities reshaped from `wrn_sourcedata_EDFig5_MOESM8.xlsx` by `extract/if-ed5-extract.R` into `(cell_line, readout, guide, condition, value)` long form. Consumed by the `emmeans` lsmeans warrant (`if_ed5:result` → `ActivatesP53Response`, finding **F7**).
- `foci_53bp1_long.csv` (sha256 `1ba6dc6f…4e9b83`, 39,249 rows) — ED Fig 6f/6h per-cell Apple-53BP1-trunc DSB foci counts reshaped from `wrn_sourcedata_EDFig6_MOESM9.xlsx` by `extract/foci-ed6-extract.R` (same long shape). Consumed by the wrapped-R interaction-lm warrant (`foci_dsb:result` → `CausesDSBs`, MSI-selective, finding **F8**).
- `gh2ax_intensity_long.csv` (sha256 `d8da9e95…625f95`, 32,882 rows) — ED Fig 6c per-cell nuclear **γH2AX** staining intensity (ES2 MSS + OVK18 MSI) reshaped from `wrn_sourcedata_EDFig6_MOESM9.xlsx` by `extract/gh2ax-ed6c-extract.R`. Consumed by the `emmeans` interaction warrant (`gh2ax:result` → `CausesDSBs`, the canonical-marker leg; reproduces the paper's log10 fold-change 0.055 ES2 / 0.144 OVK18 and contrast P<2×10⁻¹⁶, finding **F14**).
- `gh2ax_foci_long.csv` (sha256 `70abbad2…9e4b198c`, 94,791 rows) — ED Fig 6a/6d per-cell nuclear **γH2AX** foci counts (colon SW620/KM12/SW48 + ovarian ES2/OVK18) reshaped from `wrn_sourcedata_EDFig6_MOESM9.xlsx` by `extract/gh2ax-ed6ad-extract.R`. Pan-nuclear (saturated, uncountable) cells — the most-damaged, MSI-enriched — are counted at a saturation ceiling, not dropped. Consumed by the wrapped-R interaction-lm warrant (`gh2ax_foci:result` → `CausesDSBs`, the discrete-foci leg; interaction +7.3, fold-change MSI ×3.4 vs MSS ×1.0, finding **F14b**).
- `patm_foci_long.csv` (sha256 `9a718df8…2cc325de`, 191,241 rows) — ED Fig 7b/7d per-cell nuclear **phospho-ATM(S1981)** foci counts (colon SW620/KM12/SW48 + ovarian ES2/OVK18) reshaped from `wrn_sourcedata_EDFig7_MOESM10.xlsx` by `extract/patm-ed7-extract.R`. Consumed by the wrapped-R interaction-lm warrant (`patm:result` → `ActivatesDSBResponse`, the DDR-signaling leg, finding **F15**).

**Recompute-upgradeable subset (existing two-way-ANOVA dispatch):** ED Fig 3b (`va_competition`), ED Fig 4b/c/d (cell-cycle/apoptosis). ED Fig 3b layout: per cell line × {day} × {Firefly, Renilla luminescence} × {sgCh2-2, sgCh2-4, sgPolR2D, sgMYC, sgWRN1/2/3} × Value 1–6 (n=6). Relative viability = Firefly/Renilla, normalized; the two-way ANOVA is `value ~ is_WRN + guide` per cell line (per `WRN_stats_calcs.Rmd`). The IF contrasts (Fig 3c, 4c, ED 5) are lsmeans, the rescue (Fig 2c) a t-test, the xenograft (Fig 2d) lme4 — external-tool frontier, not the current institution.

### Reference code (cloned, in `data/reference/WRN_manuscript/`, gitignored)

| Source | Used for |
|---|---|
| `github.com/cancerdatasci/WRN_manuscript` (shallow) | Defines the exact Derived pipelines (the authoritative recompute reference). Phase-1-relevant: `WRN_stats_calcs.Rmd`, `make_cell_line_info.R`, `process_CCLE_MSI_data.R`, `WRN_helpers.R`, `generate_figs.Rmd`. Note: original scripts pull omics from Broad's internal *taiga* server; the public substitute is the 1.6 GB figshare rds (linked below). |

### CCLE Phase-2 MSI source (vendored 2026-06-13)

| File (in `slices/`) | Source | sha256 | Used for |
|---|---|---|---|
| `ccle_phase2_suppl_table_7_msi.xlsx` | **Ghandi et al. 2019**, *Nature* 569:503 (DOI 10.1038/s41586-019-1186-3; PMC6697103), **Supplementary Table 7** — fetched via `static-content.springer.com/…/41586_2019_1186_MOESM10_ESM.xlsx` (the PMC `bin/` URL is JS-gated) | `ad26cb44…c03eb8` | Upstream raw indel counts for the MSI classification. 3 sheets: `Descriptions`, `MSI calls` (1331 cell lines; `CCLE.hc/wes/wgs.*` + `GDSC.*` `msi_del`/`total_del` + MSI calls), `Thresholds used for MSI annot.` (the calling cutoffs, e.g. CCLE-WES `P_MS_del_1/2 = 70/80`, `N_MS_del = 750`). `process_CCLE_MSI_data.R` normalizes these → the `CCLE_MSI`/`ms_deletions_normed` already in Supp Table 1. **Caveat:** this is the *final published* table; the WRN code used a pre-publication "early version" — correct table, possibly not byte-identical. Needed only to recompute the MSI *classification* from scratch (the calls are already vendored downstream in Supp Table 1).

## Data-acquisition status (2026-06-13) — complete

Every dataset the encoding needs is now **secured** (vendored + checksummed, or trivially fetchable):

- ✅ Phase-1 dependency/annotation slices (Achilles, DRIVE, Supp Table 1) — vendored.
- ✅ DepMap omics+dep bundle (`DepMap_18Q4_data.rds`, 1.6 GB) — vendored, md5-verified.
- ✅ WRN-KO RNA-seq counts (GSE126464) — vendored, sha256.
- ✅ **Wet-lab Source Data (all 10 per-figure XLSX)** — vendored, sha256 (above). The earlier "nature.com-only / not auto-fetchable" note is **resolved**: the `static-content.springer.com` URLs are directly fetchable.
- ✅ **MSigDB Hallmark v6.2** (`h.all.v6.2.symbols.gmt`) — vendored, sha256.
- ✅ **CCLE Phase-2 Suppl Table 7** (Ghandi 2019, MSI source) — vendored, sha256.

**Every dataset the encoding could need is now vendored + checksummed.** Nothing left to fetch. The remaining gates are purely infrastructure (institution capabilities), not data.

**Infrastructure gates now closed (D53 + D56 wrapped-R, this session):** the genome-wide differential (`dd_achilles` rank-1 Q = 4.81e-24; `dd_drive` rank-1 Q = 1.46e-45; `dd_gdsc` PCR-MSI robustness rank-1 Q = 4.66e-20) all run live through the D53 ingestion path (187 MB / 59 MB matrices content-addressed, not inlined) + a multi-input `RunRuntimeScript` to a DooD-spawned R worker running limma. GSEA (`gsea_mech`, Fig 3a) likewise runs via fgsea over pinned RNA-seq counts + the Hallmark `.gmt` (D53 Collection profile). lme4 mixed models (`vivo`, `viab_KM12_bio`) run via the same wrapped-R path. The wet-lab two-way-ANOVA recomputes (`va_competition`, cell-cycle, apoptosis) run D52-native. Remaining: the IF/foci microscopy readouts (close-out plan #3–4).

## Notes for Phase 1

- **ID reconciliation is the first join.** Achilles is keyed by `DepMap_ID`, DRIVE and Supp Table 1 by `CCLE_name`. Use `sample_info.csv` to map. Encode this mapping as an Observed reconciliation resource.
- **Orientation differs** (Achilles = lines×genes; DRIVE = genes×lines) — transpose one before joining.
- **MSI grouping** for D-DIFF comes from `CCLE_MSI` (NGS) with `GDSC_MSI` (PCR) as the concordance check; exclude `indeterminate`.
- **Recompute fidelity** per [encoding plan §5.1]: against this pinned snapshot (these sha256s + figshare md5s), Class-A/B exact-or-tight; the moderated-*t* Q-values await Phase 2.5.
