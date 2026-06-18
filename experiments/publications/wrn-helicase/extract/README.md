# SampleSet extraction — the Tier-1 provenance pin

The numeric arrays inlined as `stats:sample_set_value` in
[`../chain/03-phase1-recompute-plans.esl`](../chain/03-phase1-recompute-plans.esl) are **projections
of the pinned public-data slices** (a column + filter + sort of a checksummed
CSV). This directory closes the one previously-uncommitted link in the audit
chain:

```
raw checksummed slice  ──[column + filter + sort + group]──>  inlined SampleSet
        ✓ pinned (sha256)            ✗ → ✓ this recipe          ✓ committed (the ESL)
```

## What's here

- [`extract_samplesets.py`](extract_samplesets.py) — the **single source of
  truth** for the recipes. For each SampleSet it states the source slice,
  enforces its `sha256`, and re-derives the array from the named column +
  filter. Two modes:
  - `--check` (default) — re-derive every array and diff against the values
    inlined in the ESL; exit non-zero on any drift or sha256 mismatch.
  - `--emit` — print ESL-ready arrays, for regenerating the inlined blocks.

The same recipe is also recorded **in-chain** on each SampleSet via the
`bench:extracted_from_slice` / `extracted_from_sha256` / `extraction_columns`
/ `extraction_filter` / `extraction_recipe` fields (declared in
`experiments/benchmark/base-ontologies/bench-core.esl`).

This directory also holds the **slice derivers** that reshape raw paper supplements
into the pinned tidy slices (the file-backed SampleSet inputs). Each reproduces its
pinned `sha256` byte-for-byte; all three are wired into [`../data/fetch.sh`](../data/fetch.sh):

- [`supp_table_1_to_csv.py`](supp_table_1_to_csv.py) — the paper's Supplementary
  Table 1 workbook → `wrn_supplementary_table_1.csv` (stdlib `zipfile` + `xml.etree`;
  `--check` verifies byte-identity).
- [`if-ed5-extract.R`](if-ed5-extract.R) — ED Fig 5 IF workbook → `if_ed5_long.csv`.
- [`foci-ed6-extract.R`](foci-ed6-extract.R) — ED Fig 6 foci workbook → `foci_53bp1_long.csv`.

## Running the pin

The slices are gitignored (~235 MB; provenance in
[`../data/MANIFEST.md`](../data/MANIFEST.md)). With them present:

```bash
python3 extract_samplesets.py --check
```

It is also wired into the test suite as an `#[ignore]`d test
(`crates/eigenius-statistics/tests/wrn_sampleset_pin.rs`):

```bash
cargo test -p eigenius-statistics -- --ignored inlined_samplesets_reproduce
```

The test skips gracefully when the slices (or `python3`) are absent, and
fails only on an actual drift between the inlined arrays and the raw data.

## The three SampleSets

| Resource | Slice → column | Filter / cohort |
|---|---|---|
| `wrn:wrn_dep_sampleset` | `wrn_supplementary_table_1.csv` → `avg_WRN_dep` | common-MSI-lineage, `CCLE_MSI∈{MSI,MSS}` → 37 / 91 |
| `wrn:wrn_corr_sampleset` | same → `ms_deletions_normed`, `avg_WRN_dep` | all-lineage `CCLE_MSI=MSI`, both real → 51 pairs |
| `wrn:wrn_recq_sampleset` | `achilles_18Q4_gene_effect.csv` → `WRN (7486)` | all-lineage by `CCLE_MSI` (joined via `sample_info.csv`) → 32 / 413 |

All filters are NaN-aware (reject both `NA` and `NaN`); see
[`../recompute-findings.md`](../docs/03-recompute-findings.md) F2 for why that matters.

## Follow-up (Tier 2)

This is a committed-recipe-plus-verification pin, not yet a kernel-checked
derivation. Tier 2 lifts these recipes onto the runtime substrate: the slice
becomes a content-hash-pinned external file, this script becomes an on-chain
`RuntimeScript`, and a `DataExtractionPlan` commit runs it on the substrate
and **emits** the SampleSet as a `DerivedResource` witnessed by a
`RuntimeInvocation` (input hash + script hash + image digest + output hash) —
reclassifying it from Observed-with-recipe-sidecar to Derived-from-raw-Observed
and closing the audit chain to raw bytes. Full design:
[docs/design/d53-large-data-tracking.md](../../../../docs/design/d53-large-data-tracking.md).
