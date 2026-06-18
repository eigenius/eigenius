# WRN-Helicase case study

An end-to-end Eigenius reproduction of Chan et al., *Nature* 2019 —
*"WRN helicase is a synthetic lethal target in microsatellite unstable cancers"*
(DOI [10.1038/s41586-019-1102-x](https://doi.org/10.1038/s41586-019-1102-x)).

The paper's argument — from the computational discovery (WRN is the top
differential dependency in MSI cell lines) through wet-lab validation, in-vivo
xenografts, mechanistic dissection, and the synthetic-lethal thesis — is encoded
as a typed, kernel-checkable layer chain. Statistical claims are *recomputed*
from raw data (the statistics + reasoning institutions), and literature warrants
enter as CiTO-typed citations, so every headline conclusion lands as a `Holds`
verdict the kernel verifies.

## Layout

```
chain/        the ESL layers, in load order (ontology → recompute → narrative phases)
programs/     analysis programs (JSON spec + R driver), grouped by conceptual arm:
                differential-dependency/   limma moderated-t (Achilles / DRIVE / GDSC)
                mechanism/                 GSEA, p53 IF lsmeans, 53BP1 foci
                specificity/               paralogue co-loss control (1.6 GB omics)
                invivo/                    xenograft + KM12 competition lme4
extract/      data-derivation scripts (raw source → pinned tidy slices + SampleSets)
data/         the external-data root — see "Data" below
docs/         narrative docs (encoding plan, dependency graph, recompute findings, review)
```

The chain layers load in numeric order:

| # | layer | role |
|---|---|---|
| 01 | `chain/01-onco.esl` | domain ontology (`onco:Gene`, `onco:InVivoDependence`, …) |
| 02 | `chain/02-literature.esl` | bibliographic `reference:Reference` + CiTO `reference:Citation` warrants |
| 03 | `chain/03-phase1-recompute-plans.esl` | SampleSets + StatisticalAnalysisPlans (AutoOnLoad emits `IsDerivedAs` witnesses) |
| 04 | `chain/04-phase1-recompute-conclusions.esl` | conclusions citing those witnesses |
| 05 | `chain/05-phase1-discovery.esl` | computational-discovery narrative + `TaskOutput` |
| 06 | `chain/06-phase1-biological-sap.esl` | biological-unit scope-of-inference (finding F4) |
| 07 | `chain/07-phase2-validation.esl` | wet-lab validation (on-target, structure-function) |
| 08 | `chain/08-phase3-invivo-mechanism.esl` | in-vivo + DSB/p53 mechanism |
| 09 | `chain/09-phase5-synthesis.esl` | final synthesis → `SyntheticLethal(WRN, MSI)` |

## Data

All external inputs plug into a single root, `data/`:

- **`data/MANIFEST.md`** — human-readable provenance (what each dataset is, where
  it came from, what it backs).
- **`data/sources.tsv`** — the machine-readable manifest: one row per artifact
  with its origin URL and the authoritative sha256 (the same hash used as the
  `PinnedExternalFile` content address on chain). This is the single source of
  truth for checksums.
- **`data/fetch.sh`** — downloads every input from its public origin, verifies
  md5/sha256, and runs the `extract/` derivers. Idempotent and fail-closed.
- **`data/slices/`**, **`data/large/`**, **`data/reference/`** — gitignored;
  the vendored bytes (`fetch.sh` repopulates them).

```bash
bash data/fetch.sh            # fetch + derive everything, then verify
bash data/fetch.sh --check    # verify the present copies; never download
```

One artifact — the paper's Supplementary Table 1 (an NIHMS author-manuscript
supplement) — is not cleanly auto-fetchable; `fetch.sh` copies it from the
in-repo fallback if present, else prints instructions.

### Provenance chain

```
public origin ──fetch.sh──> data/slices/ (raw, sha256-pinned)
                                  │
                                  ├─ extract/supp_table_1_to_csv.py ─> wrn_supplementary_table_1.csv
                                  ├─ extract/if-ed5-extract.R        ─> if_ed5_long.csv
                                  └─ extract/foci-ed6-extract.R      ─> foci_53bp1_long.csv
                                                                          │
              extract/extract_samplesets.py ──(column + filter + sort)──> stats:sample_set_value
                                                                          arrays inlined in chain/
```

Every step is content-addressed: the derivers reproduce their pinned sha256
byte-for-byte (`fetch.sh --check` proves it), and `extract/extract_samplesets.py
--check` re-derives the inlined SampleSet arrays from the raw slices.

## Running it

End-to-end against a local stack (loads the chain, runs the wrapped-R warrants
over the staged data, queries every verdict — all should be `Holds`):

```bash
EIGENIUS_MOCK_LLM=true docker compose up -d
./demo/wrn-helicase/run.sh
```

See [`docs/04-review.md`](docs/04-review.md) for what the demo exercises and
[`docs/02-dependency-graph.md`](docs/02-dependency-graph.md) for the full
epistemic dependency graph.

## Tests

```bash
cargo test -p eigenius-statistics wrn      # recompute warrants (statistics institution)
cargo test -p eigenius-reasoning wrn       # conclusion type-checking (reasoning institution)
```

The reasoning/statistics tests load the `chain/` ESL via `include_str!`. The
R-backed legs (xenograft, IF, foci, paralogue) are `#[ignore]`d in-process and
covered live by `demo/wrn-helicase/run.sh`.
