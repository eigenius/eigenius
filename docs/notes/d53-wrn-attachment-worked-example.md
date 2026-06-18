# D53 applied to the WRN study — a worked attachment example

> How the **full WRN file inventory** ([data/MANIFEST.md](../../experiments/publications/wrn-helicase/data/MANIFEST.md)) would be attached to the graph under
> [D53](../design/d53-large-data-tracking.md) — every file as a `PinnedExternalFile`,
> with a `DatasetSchema` (§4) where it's tabular. This validates the D53 abstractions
> against a real, varied corpus and serves as the implementation reference.
>
> *Caveat:* today the WRN PoC **inlines** small `SampleSet`s and extracts them with the
> Tier-1 `extract_samplesets.py` recipe; D53 is not yet built. This memo is the *Tier-2
> shape* — what attaching these files would look like once D53 exists. Syntax below is
> illustrative pseudo-ESL; the exact vocabulary routes through [D57](../design/d57-schema-org-vocabulary-mapping.md).

## Backend choice (§3.1)

- **`file://<volume>/<path>`** for everything vendored today — the slices already live in `data/slices/`, so a disk-volume reference is a zero-move attach. Covers the small/moderate files and is sufficient for a first implementation.
- **`oxen://repo@commit/path`** is the *upgrade* for the genuinely large / versioned / shared ones — the DepMap CRISPR matrix (187 MB), DRIVE (59 MB), and the omics `.rds` (1.6 GB). Either backend works; `content_hash` is the trust root regardless.

## The inventory

| File | ≈size | backend | media_type | schema profile |
|---|---|---|---|---|
| `achilles_18Q4_gene_effect.csv` | 187 MB | oxen:// | `text/csv` | **wide matrix** (A) — limma's input |
| `drive_D2_DRIVE_gene_dep_scores.csv` | 59 MB | oxen:// | `text/csv` | **wide matrix, transposed** (B) |
| `achilles_18Q4_sample_info.csv` | 63 KB | file:// | `text/csv` | **code-list / entity table** (D) |
| `wrn_supplementary_table_1.csv` | — | file:// | `text/csv` | **long, multi-measure** (C) |
| `wrn_supplementary_table_1.xlsx` | 246 KB | file:// | xlsx | source of the `.csv`; opaque (extract → `.csv`) |
| `GSE126464_STAR_Gene_Counts.csv.gz` | 461 KB | file:// | `text/csv`+gzip | **count matrix** (gene × sample) |
| `GSE126464_Cuff_Gene_Counts.csv.gz` | 888 KB | file:// | `text/csv`+gzip | count matrix (alt quantification) |
| `h.all.v6.2.symbols.gmt` | 48 KB | file:// | `text/tab-separated-values` (`.gmt`) | **gene-set collection** (F) |
| `DepMap_18Q4_data.rds` | 1.6 GB | oxen:// | `application/x-r-rds` | **multi-dataset container** (E) |
| `wrn_sourcedata_Fig2_MOESM3.xlsx` … `EDFig10_MOESM12.xlsx` (10 files) | ~0.1–1 MB each | file:// | xlsx | **irregular spreadsheet** (G) — opaque + extraction |
| `ccle_phase2_suppl_table_7_msi.xlsx` | 107 KB | file:// | xlsx | multi-sheet container; opaque + extraction |
| `WRN_manuscript/` (R scripts) | — | — | — | **not data** — RuntimeScript / reference, *not* D53 |

`content_hash` for each is the `sha256` already pinned in `MANIFEST.md` / the `SHA256` dict in `extract_samplesets.py` (e.g. `achilles…2eb68b`, `EDFig3…41fd6`); the `.rds` has only a vendor md5 today, so its Eigenius `content_hash` is computed at attach.

## Schema sketches (the representative profiles)

### A. Wide matrix — `achilles_18Q4_gene_effect.csv` (the canonical §4.1, limma's input)

```
PinnedExternalFile achilles_ceres {
  reference   = "oxen://depmap/18Q4@<commit>/Achilles_gene_effect.csv"
  content_hash = "sha256:2186669d…2eb68b"
  media_type  = "text/csv"
  schema      = ceres_matrix_schema
}
DatasetSchema ceres_matrix_schema {
  dimension cell_line { class = onco:CellLine;  code_list = depmap_id_codelist }   # row key
  dimension gene      { class = onco:Gene;      code_list = entrez_codelist }      # entity-per-column
  measure   ceres_gene_effect { property = onco:dependency_score; data_type = float }
  layout {
    row_key   = column("DepMap_ID") -> cell_line
    columns   = header "<SYMBOL> (<ENTREZ>)" -> gene  via entrez   # explicit parse, not name-suffix
    cell      = ceres_gene_effect
  }
}
```

This is the cube `dependency_score(cell_line, gene)`; limma's D-DIFF reads typed gene-columns over MSI-vs-MSS `cell_line` rows.

### B. Transposed wide matrix — `drive_D2_DRIVE_gene_dep_scores.csv`

DRIVE is the *same cube*, physically transposed: rows = genes, columns = `CCLE_name` cell lines. The **semantic schema is unchanged** (dims `{cell_line, gene}`, measure `demeter2_dep`); only the layout binding flips:

```
layout { row_key = column(gene-symbol) -> gene;  columns = header(CCLE_name) -> cell_line via depmap_bridge;  cell = demeter2_dep }
```

→ confirms the semantic/layout separation does real work: orientation is a *layout* fact, not a schema fact.

### C. Long, multi-measure — `wrn_supplementary_table_1.csv`

One `cell_line` dimension; the 37 columns split into **measures** (numeric) and **attributes** (categorical the analyses stratify on):

```
DatasetSchema supp_table_1 {
  dimension cell_line { class = onco:CellLine; code_list = ccle_id_codelist }   # row key CCLE_ID
  measure avg_WRN_dep        { property = onco:dependency_score; data_type = float }
  measure ms_deletions_normed { property = onco:mutator_load;    data_type = float }
  attribute CCLE_MSI   { property = onco:msi_status;  data_type = string }   # categorical → stratify
  attribute TP53_status { property = onco:tp53_status; data_type = string }
  attribute MMR_loss   { property = onco:mmr_loss;    data_type = boolean }
  layout { row_key = column("CCLE_ID") -> cell_line; columns = named -> measures+attributes }
}
```

### D. Code-list / entity table — `achilles_18Q4_sample_info.csv`

This file *defines* the `cell_line` code-list and the `DepMap_ID ↔ CCLE_name` bridge — it's the **foreign-key target** the matrices reference. So a dimension's `code_list` can itself be a `PinnedExternalFile`:

```
DatasetSchema depmap_id_codelist {
  dimension cell_line { class = onco:CellLine }   # this table enumerates the members
  attribute CCLE_name { ... }  attribute primary_tissue { ... }
  layout { row_key = column("DepMap_ID") -> cell_line }
}
```

DRIVE's `CCLE_name` columns resolve to `cell_line` *through* this bridge (`references` → `depmap_id_codelist`).

### E. Multi-dataset container — `DepMap_18Q4_data.rds`

The `.rds` is **one file holding several matrices** (`GE` expression, `CN` copy-number, `CRISPR`, `DRIVE`, `MUT_*`, `RPPA`). One `PinnedExternalFile`, but it needs **multiple `DatasetSchema`s** — one per contained matrix, each a cube sharing the `cell_line` / `gene` code-lists:

```
PinnedExternalFile depmap_omics_rds {
  reference = "oxen://depmap/18Q4@<commit>/DepMap_18Q4_data.rds"
  content_hash = "sha256:<computed-at-attach>"
  media_type = "application/x-r-rds"
  schemas = [ ge_matrix { rds-member="GE"; dims {cell_line,gene}; measure log2_tpm },
              cn_matrix { rds-member="CN"; dims {cell_line,gene}; measure log2_cn }, … ]
}
```

→ **refinement (below):** §4 currently implies one schema per file; the `.rds` (and the multi-sheet `ccle…_msi.xlsx`) need *N schemas per file*, à la Croissant's RecordSet-per-FileObject. The R worker reads the named member; the schema gives each member its cube.

### F. Non-tabular gene-set collection — `h.all.v6.2.symbols.gmt`

A `.gmt` is ragged, not a cube: each line is `set_name⇥description⇥gene1⇥gene2⇥…` (variable length). Bind at the **collection** level — a `GeneSetCollection` whose sets `reference` the `gene` code-list — and let **fgsea read the `.gmt` format natively** (the §4.3 "worker is the bridge"; the format schema is the tool's, the graph binding is "these sets are over `onco:Gene`").

### G. Irregular spreadsheets — the 10 wet-lab `.xlsx`

The §4.1 **boundary case**: multi-block geometry (per-mouse rows at fixed offsets, interleaved day-header / Firefly-Renilla blocks) can't be bound declaratively. Attach each `.xlsx` as an **opaque** `PinnedExternalFile` (no `schema`); the **extraction script** (`extract_samplesets.py`) produces clean tabular `DerivedResource`s (the competition/cell-cycle/apoptosis/xenograft tables) that carry the §4 schema. The schema attaches to the *clean output*, not the raw `.xlsx`.

## Refinements this surfaces (feed back into D53 §4)

1. **Multiple schemas per file.** The `.rds` container and the multi-sheet `ccle…_msi.xlsx` need *N* `DatasetSchema`s per `PinnedExternalFile` (one per contained matrix/sheet) — exactly Croissant's RecordSet-per-FileObject. D53 §4 should make `schema` a *set* keyed by an intra-file selector (rds member / sheet name), not a single binding.
2. **A gene-set / collection profile.** A third tabular-ish profile beyond cube and opaque: a ragged collection (`set → variable list of entity refs`). Either a first-class `Collection` schema or "opaque + the tool reads the format" (the `.gmt` route). Worth naming.
3. **Code-list source tables are first-class.** A dimension's `code_list` (e.g. `sample_info.csv` defining `DepMap_ID ↔ onco:CellLine`) is itself a `PinnedExternalFile` + schema — the FK *target*. The dimension/`references` machinery must resolve a code-list that lives in another attached file.
4. **Layout captures orientation.** DRIVE vs Achilles (transposed) confirms the layout binding must record *which axis is which dimension* — a layout fact, separate from the (shared) semantic schema.
5. **Code is not data.** `WRN_manuscript/` (the authors' R scripts) is *not* a `PinnedExternalFile` — it's reference / `RuntimeScript` (D56) territory. D53 attaches *data*, not programs.

## Illustrative ESL

The proposed D53 ESL surface (the `ingest:` ontology is *not built* — this is the target shape). A minimal ontology sketch, then the representative WRN attachments. Schemas use embedded resource blocks for the named components; layout is a single embedded `ingest:Layout`.

```esl
namespace core       = "urn:eigenius:core";
namespace reflection = "urn:eigenius:reflection";
namespace ingest     = "urn:eigenius:ingest";
namespace onco       = "urn:eigenius:benchmark:onco";
namespace wrn        = "urn:eigenius:pub:wrn";

// ── ingest ontology (D53), minimal sketch ────────────────────────────
class ingest:PinnedExternalFile : reflection:ObservedResource {
    // requires: reference, content_hash, media_type (+ inherited reflection:source)
    // recommends: schema | schemas
    description = "An external file tracked by content hash; bytes off-chain.";
}
class ingest:DatasetSchema { description = "Dimension/measure/attribute cube + a physical layout binding."; }
class ingest:Dimension     { description = "An identifying axis bound to a class (+ optional code_list)."; }
class ingest:Measure       { description = "A value bound to a property."; }
class ingest:Attribute     { description = "A per-component qualifier bound to a property."; }
class ingest:Layout        { description = "How the semantic cube maps to the physical file (wide/long/transposed)."; }

// ── D. code-list / bridge: sample_info.csv (the FK target) ───────────
resource wrn:depmap_cellline_codelist : ingest:DatasetSchema {
    description = "DepMap_ID ↔ CCLE_name cell-line code-list.";
    ingest:dimension = [ ingest:Dimension { ingest:name = "cell_line"; ingest:class = onco:CellLine; } ];
    ingest:attribute = [
        ingest:Attribute { ingest:name = "CCLE_name";      ingest:property = onco:ccle_name; ingest:data_type = core:string; ingest:source = "CCLE_name"; },
        ingest:Attribute { ingest:name = "primary_tissue"; ingest:property = onco:tissue;    ingest:data_type = core:string; ingest:source = "primary_tissue"; }
    ];
    ingest:layout = ingest:Layout { ingest:kind = "LongTable"; ingest:row_key = "DepMap_ID"; ingest:row_key_binds = "cell_line"; };
}
resource wrn:sample_info_file : ingest:PinnedExternalFile {
    reflection:source   = "DepMap 18Q4 sample_info.csv (cell-line ID bridge)";
    ingest:reference    = "file://depmap-slices/achilles_18Q4_sample_info.csv";
    ingest:content_hash = "sha256:c5778e66...fbdb498";
    ingest:media_type   = "text/csv";
    ingest:schema       = wrn:depmap_cellline_codelist;
}

// ── A. wide matrix: Achilles CERES (limma's input) ──────────────────
resource wrn:ceres_matrix_schema : ingest:DatasetSchema {
    ingest:dimension = [
        ingest:Dimension { ingest:name = "cell_line"; ingest:class = onco:CellLine; ingest:code_list = wrn:depmap_cellline_codelist; },
        ingest:Dimension { ingest:name = "gene";      ingest:class = onco:Gene; }   // entity-per-column; headers define the members
    ];
    ingest:measure = [ ingest:Measure { ingest:name = "ceres"; ingest:property = onco:dependency_score; ingest:data_type = core:float; } ];
    ingest:layout = ingest:Layout {
        ingest:kind            = "WideMatrix";
        ingest:row_key         = "DepMap_ID";        ingest:row_key_binds = "cell_line";
        ingest:column_dimension = "gene";
        ingest:header_parse    = "<symbol> (<entrez>)";   // explicit → entrez, the gene code
        ingest:cell_measure    = "ceres";
    };
}
resource wrn:achilles_ceres : ingest:PinnedExternalFile {
    reflection:source   = "DepMap Achilles 18Q4 gene_effect (CRISPR/CERES)";
    ingest:reference    = "oxen://depmap/18Q4@<commit>/Achilles_gene_effect.csv";
    ingest:content_hash = "sha256:2186669d...2eb68b";
    ingest:media_type   = "text/csv";
    ingest:schema       = wrn:ceres_matrix_schema;
}

// ── C. long, multi-measure: Supplementary Table 1 ───────────────────
resource wrn:supp_table_1_schema : ingest:DatasetSchema {
    ingest:dimension = [ ingest:Dimension { ingest:name = "cell_line"; ingest:class = onco:CellLine; ingest:code_list = wrn:depmap_cellline_codelist; } ];
    ingest:measure = [
        ingest:Measure { ingest:name = "avg_WRN_dep";        ingest:property = onco:dependency_score; ingest:data_type = core:float; ingest:source = "avg_WRN_dep"; },
        ingest:Measure { ingest:name = "ms_deletions_normed"; ingest:property = onco:mutator_load;    ingest:data_type = core:float; ingest:source = "ms_deletions_normed"; }
    ];
    ingest:attribute = [
        ingest:Attribute { ingest:name = "CCLE_MSI";    ingest:property = onco:msi_status;  ingest:data_type = core:string;  ingest:source = "CCLE_MSI"; },
        ingest:Attribute { ingest:name = "TP53_status"; ingest:property = onco:tp53_status; ingest:data_type = core:string;  ingest:source = "TP53_status"; },
        ingest:Attribute { ingest:name = "MMR_loss";    ingest:property = onco:mmr_loss;    ingest:data_type = core:boolean; ingest:source = "MMR_loss"; }
    ];
    ingest:layout = ingest:Layout { ingest:kind = "LongTable"; ingest:row_key = "CCLE_ID"; ingest:row_key_binds = "cell_line"; };
}
resource wrn:supp_table_1 : ingest:PinnedExternalFile {
    reflection:source   = "This paper, Supplementary Table 1";
    ingest:reference    = "file://wrn-slices/wrn_supplementary_table_1.csv";
    ingest:content_hash = "sha256:eebd4602...7243f2";
    ingest:media_type   = "text/csv";
    ingest:schema       = wrn:supp_table_1_schema;
}

// ── E. multi-dataset container: DepMap_18Q4_data.rds (N schemas/file) ─
resource wrn:depmap_omics_rds : ingest:PinnedExternalFile {
    reflection:source   = "DepMap 18Q4 omics bundle (figshare 7712756)";
    ingest:reference    = "oxen://depmap/18Q4@<commit>/DepMap_18Q4_data.rds";
    ingest:content_hash = "sha256:<computed-at-attach>";
    ingest:media_type   = "application/x-r-rds";
    ingest:schemas = [          // one per rds member; selector = ingest:member
        ingest:DatasetSchema { ingest:member = "GE"; ingest:dimension = [ /* cell_line, gene */ ]; ingest:measure = [ ingest:Measure { ingest:name = "log2_tpm"; ingest:property = onco:expression; ingest:data_type = core:float; } ]; ingest:layout = ingest:Layout { ingest:kind = "WideMatrix"; }; },
        ingest:DatasetSchema { ingest:member = "CN"; ingest:measure = [ ingest:Measure { ingest:name = "log2_cn"; ingest:property = onco:copy_number; ingest:data_type = core:float; } ]; ingest:layout = ingest:Layout { ingest:kind = "WideMatrix"; }; }
        // … CRISPR, DRIVE, MUT_*, RPPA …
    ];
}

// ── G. opaque irregular spreadsheet (no schema; extraction → clean table) ─
resource wrn:edfig3_sourcedata : ingest:PinnedExternalFile {
    reflection:source   = "This paper, ED Fig 3 Source Data (competition assay; multi-block xlsx)";
    ingest:reference    = "file://wrn-slices/wrn_sourcedata_EDFig3_MOESM6.xlsx";
    ingest:content_hash = "sha256:506d7ac0...41fd6";
    ingest:media_type   = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
    // no ingest:schema — irregular layout; extract_samplesets.py produces the clean
    // tabular DerivedResource that carries the §4 schema.
}

// ── F. gene-set collection: Hallmark .gmt (collection profile) ──────
resource wrn:hallmark_gmt : ingest:PinnedExternalFile {
    reflection:source   = "MSigDB Hallmark v6.2 (gene symbols)";
    ingest:reference    = "file://wrn-slices/h.all.v6.2.symbols.gmt";
    ingest:media_type   = "text/tab-separated-values";
    ingest:content_hash = "sha256:0ee07a4a...22146b";
    ingest:schema = ingest:DatasetSchema {        // collection: each set references the gene code-list
        ingest:layout = ingest:Layout { ingest:kind = "Collection"; ingest:element_class = onco:Gene; };
    };
}

// ── B. transposed wide matrix: DRIVE / DEMETER2 (same cube as Achilles, flipped) ─
resource wrn:drive_matrix_schema : ingest:DatasetSchema {
    ingest:dimension = [
        ingest:Dimension { ingest:name = "gene";      ingest:class = onco:Gene; },          // row key (symbol)
        ingest:Dimension { ingest:name = "cell_line"; ingest:class = onco:CellLine; ingest:code_list = wrn:depmap_cellline_codelist; }  // columns (CCLE_name)
    ];
    ingest:measure = [ ingest:Measure { ingest:name = "demeter2_dep"; ingest:property = onco:dependency_score; ingest:data_type = core:float; } ];
    ingest:layout = ingest:Layout {
        ingest:kind            = "Transposed";       // rows = gene, columns = cell_line — Achilles, flipped
        ingest:row_key         = "<gene-symbol>";    ingest:row_key_binds = "gene";
        ingest:column_dimension = "cell_line";
        ingest:header_key      = "CCLE_name";        // columns resolve via the code-list's CCLE_name, not the primary DepMap_ID
        ingest:cell_measure    = "demeter2_dep";
    };
}
resource wrn:drive_demeter2 : ingest:PinnedExternalFile {
    reflection:source   = "DEMETER2 DRIVE gene dependency scores";
    ingest:reference    = "oxen://depmap/drive@<commit>/D2_DRIVE_gene_dep_scores.csv";
    ingest:content_hash = "sha256:3f863c29...38254b";
    ingest:media_type   = "text/csv";
    ingest:schema       = wrn:drive_matrix_schema;
}

// ── count matrix: GSE126464 — and its sample code-list (the factorial design) ─
resource wrn:gse126464_sample_codelist : ingest:DatasetSchema {
    description = "The 12 RNA-seq samples: cell_line × guide × replicate. The design lives here, as Sample attributes — the matrix is just gene × sample.";
    ingest:dimension = [ ingest:Dimension { ingest:name = "sample"; ingest:class = onco:Sample; } ];
    ingest:attribute = [
        ingest:Attribute { ingest:name = "cell_line"; ingest:property = onco:from_cell_line; ingest:data_type = core:string; },  // OVK18 | SW48
        ingest:Attribute { ingest:name = "guide";     ingest:property = onco:guide;          ingest:data_type = core:string; },  // sgCh2-2 | sgWRN2 | sgWRN3
        ingest:Attribute { ingest:name = "replicate"; ingest:property = onco:replicate;      ingest:data_type = core:string; }   // A | B
    ];
    ingest:layout = ingest:Layout { ingest:kind = "LongTable"; ingest:row_key = "sample_id"; ingest:row_key_binds = "sample"; };
}
resource wrn:gse126464_counts_schema : ingest:DatasetSchema {
    ingest:dimension = [
        ingest:Dimension { ingest:name = "gene";   ingest:class = onco:Gene; },
        ingest:Dimension { ingest:name = "sample"; ingest:class = onco:Sample; ingest:code_list = wrn:gse126464_sample_codelist; }
    ];
    ingest:measure = [ ingest:Measure { ingest:name = "read_count"; ingest:property = onco:star_count; ingest:data_type = core:integer; } ];
    ingest:layout = ingest:Layout {
        ingest:kind            = "WideMatrix";
        ingest:row_key         = "<gene-id>";   ingest:row_key_binds = "gene";
        ingest:column_dimension = "sample";     ingest:header_key = "sample_id";
        ingest:cell_measure    = "read_count";
    };
}
resource wrn:gse126464_star_counts : ingest:PinnedExternalFile {
    reflection:source     = "GEO GSE126464 STAR gene counts (WRN-KO RNA-seq)";
    ingest:reference      = "file://wrn-slices/GSE126464_STAR_Gene_Counts.csv.gz";
    ingest:content_hash   = "sha256:e66c70f3...76daa5";
    ingest:media_type     = "text/csv";
    ingest:content_encoding = "gzip";
    ingest:schema         = wrn:gse126464_counts_schema;
}
```

Notes on the surface: `content_hash` would normally be *computed* at attach (§7) — shown here as the pinned `MANIFEST` values for illustration. `oxen://…@<commit>` placeholders stand for a real Oxen commit. The `ingest:schemas` (plural, keyed by `ingest:member`) is the multi-schema-per-file refinement (§10); `ingest:Layout.kind = "Collection"` is the gene-set profile refinement.

## What this validates

The cases that matter fit cleanly: the **DepMap matrices** (limma's actual input) are the canonical wide-matrix cube; **Supp Table 1** is the long multi-measure case; the **bridge table** is the FK target. The awkward cases are all handled by documented escape hatches — multi-schema-per-file for the container/`.rds`, the collection profile for `.gmt`, and opaque-plus-extraction for the irregular `.xlsx`. Net: D53's abstractions cover the entire WRN corpus, and the four refinements above are the concrete TODOs they expose for §4.

**Storage ⊥ warrant grade (D53 §6).** Attaching these as `PinnedExternalFile`s is a *storage* decision; it does **not** make them limma/wrapped-only. Any of these matrices can equally back a **`SampleSet` whose values reference the `PinnedExternalFile`**, fed to a **native (D52) recompute** over the materialized array — native-grade at genome scale. So the WRN `SampleSet`s are inlined today purely because they're tiny; the *same* SampleSet shape would reference a file at scale, with the method (native vs wrapped), not the file size, choosing the grade. D-DIFF is wrapped because limma's eBayes is wrapped, not because its input is a file.
