# WRN encoding — data provenance manifest (Phase 0)

Provenance for the vendored Phase-1 data slices. The slices themselves live under
`data/slices/` and are **gitignored** (large); this manifest is the committed,
content-addressed record. Decision §9.1 of the [encoding plan](../encoding-plan.md):
*fetch the minimal Phase-1 slices, checksum them, link the rest.*

Fetched 2026-06-12. Figshare `supplied_md5` verified on download (all OK); `sha256`
is our content address of the local copy.

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

**Total vendored: ~235 MB.** These four slices support the entire Phase-1 computational-discovery spine (`H1 → D-DIFF → C-WRN → D-RECQ/D-BIOM/D-REFINE`).

### Reference code (cloned, in `data/WRN_manuscript/`, gitignored)

| Source | Used for |
|---|---|
| `github.com/cancerdatasci/WRN_manuscript` (shallow) | Defines the exact Derived pipelines (the authoritative recompute reference). Phase-1-relevant: `WRN_stats_calcs.Rmd`, `make_cell_line_info.R`, `process_CCLE_MSI_data.R`, `WRN_helpers.R`, `generate_figs.Rmd`. Note: original scripts pull omics from Broad's internal *taiga* server; the public substitute is the 1.6 GB figshare rds (linked below). |

## Linked, not fetched (accession recorded; pull on demand)

| Resource | Accession | Why deferred |
|---|---|---|
| O-WRNFIG — "DepMap Datasets for WRN manuscript" | figshare **7712756**, `DepMap_18Q4_data.rds` (1.6 GB, md5 `f9e62e63bbc58ada5fc1f2d0534d08c5`) | The authors' curated `dat` bundle (expression/CN/mutation/RPPA/dependency). Not needed for Phase 1 (the dependency matrices are vendored above; MSI/MMR/TP53 calls are pre-computed in Supp Table 1). Pull if recomputing the *classifications* from raw omics. |
| O-OMICS — DepMap 18Q4 omics | depmap.org | Subsumed by O-WRNFIG for fidelity; classifications already in Supp Table 1. |
| O-MSEQ — WRN-KO mRNA-seq | GEO **GSE126464** | Phase-4 (differential expression / GSEA). |
| O-HALLMARK — MSigDB Hallmark | MSigDB [43] | Phase-4 (GSEA). |
| CCLE Phase-2 MSI source | Ghandi/CCLE 2019 Suppl. Table 7 | Upstream of the MSI calls already baked into Supp Table 1; only needed to recompute MSI classification from scratch. |

## Notes for Phase 1

- **ID reconciliation is the first join.** Achilles is keyed by `DepMap_ID`, DRIVE and Supp Table 1 by `CCLE_name`. Use `sample_info.csv` to map. Encode this mapping as an Observed reconciliation resource.
- **Orientation differs** (Achilles = lines×genes; DRIVE = genes×lines) — transpose one before joining.
- **MSI grouping** for D-DIFF comes from `CCLE_MSI` (NGS) with `GDSC_MSI` (PCR) as the concordance check; exclude `indeterminate`.
- **Recompute fidelity** per [encoding plan §5.1]: against this pinned snapshot (these sha256s + figshare md5s), Class-A/B exact-or-tight; the moderated-*t* Q-values await Phase 2.5.
