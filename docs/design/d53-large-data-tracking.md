# D53 — Large-Data Tracking and External-File Inputs (Oxen)

*Status: design memo · June 2026 (rev. — rescoped to large-data tracking; folds in the Oxen decision from [D55 §9](d55-r-language-runtime.md))*

*Companion documents: [D26 runtime substrate](d26-runtime-substrate.md), [D49 ChainWitness machinery](d49-chainwitness-machinery.md), [D52 measurement statistics institution](d52-measurement-statistics-institution.md), [D55 R language runtime](d55-r-language-runtime.md), [D56 component execution](d56-component-execution-and-derivation-materialization.md), [D50 §9.1 data vendoring](d50-benchmark-evaluation-approach.md).*

*This memo specifies how data files **too large to inline on the chain** — genome-scale matrices, `.rds`/HDF5/parquet blobs — are (1) **tracked** by content hash, (2) **represented** as typed resource nodes in the knowledge graph, and (3) **provided as inputs to script executions** on the runtime substrate, without putting their bytes on the chain. The store is **Oxen**. Small inputs need none of this — they stay chain-resident Eigon-CBOR (the D55/D56 default); this memo is only about the large-data path.*

*Despite the filename, **D53 is not an institution.** An institution (D14: statistics, reasoning, Lean) is an epistemic actor that emits typed judgments by re-deriving or verifying a claim. D53 is two non-epistemic things — an **ontology extension** (the `PinnedExternalFile` + `DatasetSchema` typed nodes) and a **substrate capability** (fetch/verify/materialize external files as inputs). The actor that makes a claim over large data is the D56 component / wrapped-R analysis (e.g. limma) that D53 feeds. See §7 for the architectural placement and the attach → provision → capture lifecycle. (The earlier "Data Ingestion Institution" title is vestigial from a pre-rescope design that had an AutoOnLoad ingestion gate; that gate was removed.)*

---

## 1. The problem — data too large to inline

Most Eigenius inputs ride on the chain as Eigon-CBOR, and small tabular data (the WRN `SampleSet`s — tens to hundreds of values) is inlined directly, with a committed extraction recipe (`experiments/publications/wrn-helicase/extract/extract_samplesets.py` + its `--check` pin) tracing each array back to a checksummed slice. That is the right shape for small data and is **out of scope here** (§6).

Some inputs cannot be inlined. The motivating case is WRN **D-DIFF** (the genome-wide differential-dependency call behind `TopDifferentialDependency`): it needs the full DepMap CRISPR and DRIVE dependency matrices (~9M and ~3M values; the curated DepMap omics bundle is 1.6 GB). Putting those bytes on the chain would bloat it; vendoring them in-tree is rejected by the [D50 §9.1] stance (large slices stay out-of-band, the content address travels). Yet a recompute must still be able to **name the exact bytes**, **verify them**, and **feed them to a script**. Large data is therefore a third category: not inlined, not vendored, but **content-pinned and externally tracked**.

```
small data  → inlined Eigon-CBOR / committed SampleSet            (on-chain; the default)
large data  → PinnedExternalFile node → bytes tracked in Oxen     (off-chain bytes, on-chain hash; this memo)
```

The mechanism is **size-agnostic** — content-hash + reference + fetch-verify-materialize does not care whether the file is 187 MB or 100 KB. Large data *requires* it (can't inline); smaller already-out-of-band sources may *opt into* it.

**Storage is independent of warrant grade (§6).** The inline-vs-`PinnedExternalFile` choice above is *only about where the bytes sit* — it does not determine how a dataset is computed over or how strongly its result is warranted. A `SampleSet` may be **inlined** *or* **reference a `PinnedExternalFile`** for its values; in either case the statistics institution's native numerics run over the materialized array, native-grade. Size picks storage; the *method* (native vs wrapped) picks the grade. (The one thing genuinely out of scope here is the *extraction step* — re-deriving a small `SampleSet` from a raw multi-block source — which stays the `extract_samplesets.py` + `--check` recipe, §9.)

## 2. Oxen — the large-data store

[Oxen](https://github.com/Oxen-AI/Oxen) (`liboxen`, Rust) is a git-like **data version-control system for large datasets** — it mirrors git's interface, is built for any data type, and scales to millions of files / terabytes. It gives us a **content-addressed, versioned dataset reference**: a stable `oxen://repo@commit/path` coordinate that resolves to exact bytes and can be re-fetched anywhere.

**Three forcing reasons, not just size.** Oxen is warranted by (1) **size** (can't inline genome-scale data), (2) **versioning** (a stable `@commit` coordinate), and — critically — (3) **distribution**: when orchestration workers run across multiple machines, a content-addressed pull store is what lets *every node* obtain the bytes independently, by hash, with no shared filesystem. This is the location-independent-identity property that content-addressed stores (Oxen / Git-LFS / DVC / IPFS) are built for, and it makes Oxen the natural backend for a distributed substrate (§3.1).

**Trust model — Oxen is in the availability TCB, not the correctness TCB.** Eigenius does **not** trust Oxen's internal addressing for correctness. When the substrate materializes a tracked file it computes Eigenius's **own `content_hash`** over the bytes and checks it against the value pinned on the chain (fail closed on mismatch). So:

- **Correctness root** = the chain-committed `content_hash` (+ script IRI, image digest, output hash). Verification = re-fetch *any* byte-identical copy, re-run, re-hash.
- **Availability TCB** = Oxen (plus the image registry). Losing Oxen costs reproducibility *convenience*, not verifiability — the dataset can be restored from any byte-identical source and re-checked against the same hash.

This is what makes a content-addressed, versioned dataset reference "as trustworthy as an inlined hash, without bloating the chain."

## 3. `PinnedExternalFile` — the typed resource node

The graph extension: a resource class that *is* the on-chain, content-addressed stand-in for an off-chain large file.

- `ingest:PinnedExternalFile` — `requires`: `reference` (the locator — **scheme-dispatched** over pluggable backends, §3.1), `content_hash` (`sha256:…`, **Eigenius-computed over the materialized bytes**), `media_type`/`format` (e.g. `application/vnd.apache.parquet`, `application/vnd.apache.arrow.file`, `text/csv`, `application/x-hdf5`, `application/octet-stream` — Parquet/Arrow being the expected tabular case, §4.1). Its IRI is derived from `content_hash`, so byte-identical files converge to one node (mirror-IRI discipline).

### 3.1 Backends — Oxen for the large/versioned case, a disk volume for the rest

`reference` is resolved by a small **resolver** that dispatches on scheme; `content_hash` is the **backend-independent trust root**, so adding or swapping a backend never touches correctness — only availability.

- **`oxen://repo@commit/path`** — the versioned, large-scale, possibly-remote case (§2). Fetched substrate-side via the Oxen client.
- **`file://<volume>/<path>`** (or a bare content-store key resolved against a configured store dir) — a **plain disk volume/folder**. This is the no-Oxen fallback and the *simplest* backend: the bytes already sit on a mounted volume; the substrate just verifies `content_hash` and exposes the path (no fetch). **It matches current practice** — the WRN slices live in `data/slices/`, pinned by `sha256` in `MANIFEST.md`; modeling them as `PinnedExternalFile`s with a `file://` reference is exactly that, made first-class. Oxen is the *upgrade* (versioning, terabyte scale, sharing) when a folder isn't enough.

Both are in the **availability TCB only**; the Eigenius `content_hash` check (substrate-side, §5) is what makes either trustworthy. Future backends (S3, an HTTP content store) slot into the same resolver without touching the node shape or the trust model.

**Machine-independence — the rule for distributed workers.** When orchestration workers are distributed across machines, a backend is valid only if **every worker node can resolve the `reference` to the same `content_hash`'d bytes**. This splits the backends:
- **`oxen://` (and any pull store: S3, HTTP-CAS) is distribution-native** — each node fetches by reference into its *own* local content-addressed cache; no shared filesystem required. Same hash → same bytes on every node, so per-node caches are coherent by construction. This is the distributed default.
- **`file://` is distribution-valid only on a *shared / network* volume** (NFS, a CSI volume, a cloud shared disk) mounted at the **same path on every node**. A **node-local** `file://` path is **single-machine only** — it silently breaks the moment a worker lands on another host. So `file://` is for single-machine / dev, or for a deliberately network-shared volume; node-local `file://` must be rejected (or flagged) in a distributed deployment.

Consequence for the depot (§5): with co-located orchestrator + DooD workers, the substrate fetches once into the shared depot. **Distributed → the fetch is per-host** (each host's agent pulls by reference into its local depot, content-addressed so idempotent), *or* the depot is itself a network-shared volume. Either way the `content_hash` check runs wherever the bytes land, so the correctness root is unchanged.

`PinnedExternalFile` is a genuine `reflection:ObservedResource` — the raw file is the honest Observed node; anything a script derives from it is `Derived`. **The bytes are not committed** — only `reference` + `content_hash` + `media_type` (+ the schema of §4) travel on the chain. This is the typed-graph representation of "a large external dataset, pinned."

## 4. Dataset schema — binding file axes to the graph

`content_hash` + `media_type` say what the file *is*; they say nothing about what its *contents mean*. Without more, a `PinnedExternalFile` is an opaque blob a script happens to parse. To make it a genuine **extension of the resource graph** — so a recompute can operate on, and a warrant can name, the file's internals by graph identity — the node carries an optional **dataset schema** that binds the file's internal structure to typed resources.

Two rules shape it, both validated by the prior-art survey ([notes](../notes/d53-dataset-schema-prior-art.md)): **bind at the granularity where entities live** (→ two profiles, below), and **separate the *semantic* cube from the *physical* layout** — *what* the axes mean (graph-bound, reusable) is declared apart from *where they sit in the file* (per-file, wide vs long). That second rule is the lesson of the W3C **RDF Data Cube (QB)** family — especially Allotrope's **ADF-DCO**, which maps an abstract scientific data cube to a concrete HDF5 layout — and of **MLCommons Croissant**, which superimposes a logical schema over unmodified files.

### 4.1 Tabular profile — a data cube bound to the graph

Adopt the QB **dimension / measure / attribute** decomposition, expressed as a reusable schema resource plus a per-file layout binding.

**The semantic schema — `ingest:DatasetSchema` (a reusable DSD analog).** Declared once, referenced by many `PinnedExternalFile`s with the same shape (QB's `qb:structure` reuse):
- **dimensions** — the axes that *identify* a value, each bound to an Eigenius **class** (or coded code-list): `cell_line → onco:CellLine`, `gene → onco:Gene`. A dimension bound to a class is a **foreign key** (Croissant `references` / QB `qb:codeList`): values *resolve to* existing resources, never copied.
- **measure(s)** — the *value* itself, bound to a **property**: `onco:dependency_score`.
- **attributes** — units / qualifiers, attachable **per component** (per dimension or measure) — fixing base-QB's gap where an attribute could only hang off a whole observation.

So a bound dataset is a **typed cube**: a dimension-tuple ↦ measure, i.e. `dependency_score(cell_line, gene) = value`. A recompute (limma) operates on *typed* axes ("the WRN gene-dimension over the MSI cell-line values") by graph identity, which its warrant can name.

**The physical layout binding (per file).** Separately, the file declares *where* each dimension/measure lives — Croissant-`source`-style (`column` / `row-key` / `header-regex`). This is what makes one semantic cube readable from either layout:
- *long/tidy table* — one column per dimension + a measure column.
- *wide matrix* (the genome-wide case) — one axis is the row key (`DepMap_ID → cell_line`), the other is **entity-per-column**: the column header maps to a dimension value by an **explicit** parse rule (`WRN (7486)` → entrez → the `onco:Gene` code-list) — declared, not Croissant's fragile implicit name-suffix match.

*Validated against the WRN PoC files:* the DepMap CRISPR/DRIVE matrices (`DepMap_ID` rows × `SYMBOL (ENTREZ)` columns) — limma D-DIFF's actual input — fit the wide-matrix/entity-per-column case exactly; Supp Table 1 fits the long/named-columns case (one `cell_line` dimension, numeric measures, categorical attributes the analyses stratify on); and the existing `SampleSet`s are this schema in miniature. **Boundary — declarative layout has limits.** Irregular **multi-block spreadsheets** (the raw Nature Source-Data `.xlsx`: per-mouse rows at fixed offsets, interleaved day-header / Firefly-Renilla blocks) cannot be bound by a declarative `source` — they require an *imperative* extraction step (the existing Tier-1 `extract_samplesets.py` is exactly that). For such sources the §4 schema attaches not to the raw file but to the **clean tabular `DerivedResource`** the extraction produces — the same "re-emerges on the derived output" pattern as §4.2. So: declarative `source` binding for clean tabular/matrix files; an extraction script → clean table → schema for irregular ones.

**Formats: Parquet and Arrow simplify the binding.** The expected on-disk formats are increasingly **Parquet** (columnar, on disk) and **Arrow** (columnar, in-memory / IPC), not CSV/xlsx. Both are *self-describing*: column names and types travel inside the file. So for these the §4 schema needs only the **semantic** layer — which named column is which graph dimension/measure — sitting on top of the format's free **structural** schema (no datatype inference, no CSV-quoting ambiguity, no fragile header parsing). Columnar storage also makes the wide-matrix case cheap: limma selecting a few gene-columns from a 17k-column Parquet is a projection push-down, not a full scan — important at genome scale. A file's embedded key-value metadata (Arrow field metadata / Parquet footer) is a convenient place to *carry* the binding, but the **authoritative `DatasetSchema` is the chain node, not the file's self-description** (same boundary stance: borrow the structural schema, keep the semantic typing on-chain). And the natural output of the irregular-spreadsheet extraction step above is a clean **Parquet/Arrow** table — which then takes the declarative schema directly. (Croissant already treats Parquet/Arrow as first-class `FileObject` formats, so this aligns with the borrowed vocabulary.)

**Scale: the schema is the linkage, not a materialization.** A 1,400 × 17,634 matrix is *not* exploded into cell nodes; the binding resolves lazily against the pinned bytes — by the script at run time, by an auditor re-checking. Only the schema + layout binding (a handful of axis→IRI bindings) is committed. **Checkable, not just asserted:** a cheap header/key scan validates the layout against the declared dimensions and code-lists before dispatch (the §6-analog invariant gate).

**Lineage.** This borrows QB's dimension/measure/attribute split, reusable-DSD, and IRI-coded dimensions; Croissant's logical-over-physical superimposition, `source`, and `references` foreign key; and CSVW's `csv2rdf` tabular→typed prior art — all bound to **kernel-typed Eigenius IRIs**, not free-text labels (the same boundary stance as RO-Crate/D57). ADF-DCO is the direct precedent (a scientific cube mapped to a physical file). It generalizes what `SampleSet` already does in miniature — `group_a`/`group_b` is a hand-rolled micro-schema; this lifts it to arbitrary external tables. Full survey + citations: [d53-dataset-schema-prior-art.md](../notes/d53-dataset-schema-prior-art.md).

### 4.2 Opaque / record-collection profile (FASTQ/BAM reads, images, …)

The column/row model does **not** apply — a FASTQ file is millions of reads, and reads are not graph entities; binding them individually is a category error. The binding moves to two other places:

- **File level — the dataset *is* the typed node.** The file is a typed `ReadSet` / `SequencingRun` resource linked to the entities that *do* exist: the `Sample` / `onco:CellLine` it derives from, the assay (RNA-seq), platform, reference genome, read length, paired-end. These are the graph bindings.
- **Records stay opaque.** Their internal structure is a *format* schema (FASTQ) understood by the worker's tools (the aligner), never bound to the graph. The reads are payload, not resources.
- **The binding re-emerges on the derived output.** STAR/featureCounts turns reads into a **gene × sample count matrix** — which is the tabular profile (§4.1) again: columns → `onco:Gene`, rows → `Sample`, cells → counts — committed as a `DerivedResource` whose `ObservedResource` ancestor is the `ReadSet`. Graph-linkage isn't lost; it re-attaches at the stage where entities reappear.

### 4.3 The worker is the bridge

The schema is a *contract*; the **script/worker honors it**, turning format-level access (read column `X`, parse a FASTQ record) into typed values per the bound axes. This is the same role the mirror generator plays for chain-resident resources (class → typed struct), extended to external files: the schema declares the typing, the worker materializes it, and the warrant names axes by IRI rather than by byte offset. A file may be pinned with *no* schema (pure bytes for a black-box tool); it just cannot then be referenced typedly downstream — the schema is what graduates a blob to a graph-linked dataset.

### 4.4 Worked example — the full WRN corpus

A concrete attachment of the **entire WRN file inventory** under these abstractions — wide matrices (DepMap CRISPR/DRIVE), the long multi-measure pivot (Supp Table 1), a code-list/bridge table, the multi-matrix `.rds` container, the `.gmt` gene-set collection, and the irregular wet-lab `.xlsx` — is worked out in [docs/notes/d53-wrn-attachment-worked-example.md](../notes/d53-wrn-attachment-worked-example.md). It validates that the abstractions cover the whole corpus and surfaces four refinements folded into §10: **multiple schemas per file** (the `.rds`/multi-sheet container case), a **collection profile** (ragged `.gmt`), **code-list source tables** as first-class FK targets, and **layout-captures-orientation** (transposed DRIVE).

## 5. Provision to script executions

Large files reach a computation as **inputs to a substrate script run** (D26 / D56 `RunRuntimeScript`), alongside (or instead of) the chain-resident Eigon-CBOR inputs that small data uses:

1. The kernel resolves a script's `PinnedExternalFile` input(s) and asks the substrate to **fetch by `reference`** (via the prebuilt Oxen client — §9), **verify the bytes hash to `content_hash`** (fail closed), and **materialize** the file into the shared depot at a content-addressed path.
2. The substrate **passes the worker a file reference (the path), not the bytes**: it injects the materialized path into the script's input binding. The script (e.g. an R/limma program) opens the file at that path and runs.
3. The **output is a small `DerivedResource`** committed to the chain (e.g. the WRN D-DIFF call — a handful of numbers + the ranking), carrying an `IsDerivedAs` witness (D49). **The large input never touches the chain.**

**Fetch is substrate-side, once per host — workers stay runtime-agnostic.** The fetch + content-hash verification run in the substrate (orchestrator side); the file lands in the **`depot`**, which is bind-mounted into the DooD-launched worker siblings (D26 §9.5) and used today for worker UDS sockets. With co-located orchestrator + workers this is one fetch into a shared local depot; **distributed across machines, it is one fetch *per host*** into that host's content-addressed depot (or a network-shared depot) — see §3.1. Either way the worker sees the file at a path inside its sandbox without ever running Oxen, and the only addition to the worker protocol is a **file-path input kind** alongside the existing chain-resident CBOR input. This is the load-bearing DRY decision: the Oxen client + fetch + auth + verify live in **one place** (the substrate), not reimplemented across the R / Julia / Python runtimes — each runtime only learns to open a path. Large bytes travel over the **filesystem**, never the UDS (which carries control + small CBOR). Read-only mount for the pinned input; the worker only ever sees content-verified bytes (the correctness root stays substrate-side).

Multi-file joins (WRN's RecQ analysis joins a matrix + sample-info + supp table) are an array of `PinnedExternalFile` inputs, each fetched and verified independently and materialized to its own depot path; the join logic lives in the script.

The fetch-and-verify reuses the existing `LibraryContent::External { reference, content_hash }` discipline (D26 §7.2) and the `boundary.rs` pre-dispatch check pattern — no new trust mechanism. This is the one genuine new seam: substrate inputs today are chain-resident resources passed as CBOR; `PinnedExternalFile` adds an **externally-tracked, content-verified file input passed by depot path**.

**A wrapped script is not the only consumer.** The same fetch + verify + materialize serves a **native (D52) recompute** equally: a `SampleSet` whose values are a `PinnedExternalFile` is materialized the same way, and the institution's deterministic Rust numerics read the materialized array instead of an inlined one — same path, native grade (§6). "Provision to script executions" is the D56 instance; provision-to-native-recompute is its symmetric twin over the identical mechanism.

## 6. Warrant grade — follows the method, not the storage

**Storage and warrant grade are orthogonal.** Where a dataset's bytes live — inline Eigon-CBOR vs a `PinnedExternalFile` — is a *storage* choice driven by size; it does **not** set the warrant grade. The grade is set by the **method's implementation**:

- **Native recompute (D52)** — the statistic is re-derived in deterministic Rust (Wilcoxon, ANOVA, classifier). Trust root = the kernel's code; re-checkable by re-running the Rust.
- **Wrapped runtime (D56)** — the method is a pinned external tool (limma eBayes, lme4 REML, fgsea) too costly to reimplement faithfully. Trust root = the pinned image; re-checkable by re-running the tool in that image.

**Either grade can consume a `PinnedExternalFile`.** The substrate fetches + content-verifies + materializes it (§5); the deterministic Rust *or* the wrapped tool then reads the materialized array. **Re-checkability comes from the `content_hash`** — re-fetch by hash → re-run → same result — so it holds regardless of where the bytes sat or where the code ran. limma is wrapped because *eBayes is not natively reimplemented*, **not** because its matrix is large; a *native* method (say a Wilcoxon over a genome-scale column) reading the same `PinnedExternalFile` would be **native-grade**. Size picks storage; the method picks the grade.

The witness pins the same content-addressed quartet either way: input `content_hash`, the method/program identity, the environment (`ImageDigest` for wrapped; the kernel version for native), and the output hash. The claim: *"this method, in this environment, on this exact pinned input, produces this exact output — reproducibly."* (The lme4 warrants — `concl_vivo` / `concl_viab_KM12_biological` — are the wrapped instance, proven live; D-DIFF/limma is the next wrapped one. A native recompute over a `PinnedExternalFile` is the symmetric case, not yet exercised in WRN only because its native methods happen to run over small inlined SampleSets.)

### 6.1 Placement of native-over-file recompute

A native method recomputes in **deterministic Rust in the kernel** — that is unchanged by where the *bytes* live (the recompute stays in-kernel, so native grade is unambiguous). What changes is how the kernel obtains the array, because **the kernel does not fetch** (fetch is a substrate capability, §5; the kernel has no network/remote I/O and does not share the orchestrator's depot for remote pulls). The placement therefore splits the two concerns:

- **The orchestrator owns the I/O.** Per the standing division of labour (the orchestrator performs I/O on the kernel's behalf), the orchestrator fetches + **content-verifies** (`content_hash`, fail closed) the `PinnedExternalFile` via the §5 resolver and makes the bytes available to the kernel — either by returning the materialized array as **Eigon-CBOR**, or by populating a **local content-addressed store** the kernel can read by hash.
- **The kernel reads it through a storage-abstraction capability.** A file-backed SampleSet's observations resolve through a kernel-side **content-array capability** keyed by `content_hash` + a column selector (a §4 `DatasetSchema` measure/dimension). The institution code stays storage-agnostic: it asks for "the SampleSet's values" and gets a `Vec<f64>` whether they were inline inductive data or read from a materialized file. The read is a *content-addressed local read* (re-verifiable against `content_hash`), not a remote fetch — so the kernel's I/O-free-of-the-network posture holds.
- **Grade is native** because the method is deterministic Rust re-runnable over the hash-verified bytes (§6); the witness pins the kernel version as the environment, exactly as for inlined SampleSets. The only TCB addition over the inline case is the orchestrator's fetch+extract, which re-checkability covers (re-fetch by hash → re-extract → re-run → same result).

This unifies the two framings: the orchestrator-returns-CBOR transport and the kernel-storage-capability are the *write* and *read* halves of one content-addressed seam. The kernel-facing surface is the storage capability; the orchestrator is one populator of it (a `file://` on a shared volume needs no fetch; an `oxen://` is fetched once per host into the content store).

**Implemented shape.** Read side: the kernel's `ContentArrayStore` (a *pure local reader* — it never fetches) resolves `file://` directly and any other reference against `<depot>/extfile-cache/<hash>/<name>`, re-verifying `content_hash`; the statistics institution is wired to it at startup when `EIGENIUS_EXTFILE_CACHE_DIR` is set. Write side: `eigenius data provision <iri>` (the §7 provision step) materializes a `PinnedExternalFile` into that cache via the §5 resolver. The two share the cache by filesystem — **no kernel→orchestrator RPC**; co-located (or per-host) deployment is assumed, matching §3.1's per-node pull model. The CBOR-transport variant is only needed for a kernel/orchestrator split with no shared filesystem — net-new plumbing (the kernel is a gRPC *server* and does not call out mid-validation), and deliberately *not* built.

## 7. Architectural placement & lifecycle

D53 is **not an institution** — it makes no claim and emits no verdict. It contributes an **ontology extension** (the `PinnedExternalFile` + `DatasetSchema` nodes, §3–§4) and a **substrate capability** (fetch/verify/materialize external files, §5). The epistemic work — turning data into a derived result with a witness — is the **D56 component / wrapped-R analysis** D53 feeds (D52/D56/D49). Three layers, cleanly separated: typed nodes (D53) · substrate (D26/D55) · the analysis that makes the claim (D52/D56).

The lifecycle has three steps:

1. **Attach** (data → graph) — *a plain content-addressed commit, not a kernel commit hook.* The `PinnedExternalFile` IRI is derived from `content_hash`, so **the hash *is* the node's identity** (self-certifying): the commit records "this node ≡ hash X, located at `reference` R," nothing more. Bytes stay off-chain (Oxen / disk volume, §2–§3.1); only the typed node travels; idempotent (same hash → same IRI). "Properly" means the hash is **computed, not author-asserted** — a thin `eigenius data attach <reference>` has the substrate fetch the bytes **once**, hash them, mint the IRI, and commit (hand-authoring + `eigenius load` is allowed, but then the hash is unchecked until use). The kernel **cannot** verify the hash at commit regardless: it has no fetch capability (the Oxen/disk client is a *substrate* capability, §5 / §10); its only commit-time role is ordinary ontology validation (well-formed node). There is **no separate "ingest → SampleSet" step** — that ingestion-as-recompute path (and its AutoOnLoad gate) was removed in the rescope; producing a result is the *consume* step below.

   *Verification happens at provision, not attach.* The `reference ↔ bytes` consistency — do R's bytes actually hash to X, and does the file match the §4 schema? — is enforced at **provision time, fail-closed** (step 3 / §5): the substrate re-hashes before any script runs, so a wrong or stale reference can never feed a computation. That is sufficient for correctness; attach-time checking is optional. A deployment *may* add an **eager attach-time check** (to catch an unreachable reference early) — but that is a **substrate-dispatched** validation (orchestrator fetches + hashes + scans the schema), surfaced via the `attach` CLI or an AutoOnLoad dispatch — *not* a kernel hook and *not* a claim.
2. **Provision** (graph → consumer). The substrate fetches + verifies + materializes the `PinnedExternalFile` to a depot path (§5), read per the `DatasetSchema` (§4). The consumer is *either* a **D56 `RunRuntimeScript`** (wrapped — names the file as an input) *or* a **native D52 recompute** (a `SampleSet` whose values reference the file; the institution's Rust numerics read the materialized array). Storage and grade are orthogonal (§6) — the same provision serves both; the consumer's method, not the file, sets the warrant grade.
3. **Capture** (worker → graph). The script emits an output resource — the **existing D56/D49 mechanism, unchanged** (proven for the lme4 warrants): the worker returns the output (Eigon-CBOR via `r_eigon_*`) carrying its `canonical_proposition`; the kernel commits it; a `reflection:ProgramTrace` links program→output; D49 mints `IsDerivedAs(output, canonical_proposition)` for downstream reasoning. The `RuntimeInvocation` records the `PinnedExternalFile` input `content_hash`, tying the derived result reproducibly to the exact pinned input. D53 adds nothing to capture — it only types and provisions the *input* side.

So: **attach is an ontology commit, provision is a substrate capability, capture is the unchanged D56/D49 derivation path** — none of it an institution.

### 7.1 CLI surface

Mirrors the `eigenius script` family (D26 §10) and `eigenius env`. Spec-level; implementation follows the resource model, as `eigenius script` followed D26 §10.

```bash
# Attach — the only byte-touching verb. The substrate fetches <reference> once,
# computes content_hash, mints the content-addressed IRI, and commits the
# PinnedExternalFile. --schema binds a DatasetSchema (§4); --media-type when not
# inferable; --verify also runs the eager check (reachability + hash + schema scan).
eigenius data attach <reference> [--schema <schema-iri>] [--media-type <mt>] [--verify]

eigenius data list [--media-type <mt>]        # graph query over PinnedExternalFiles
eigenius data inspect <iri>                    # reference, content_hash, media_type, schema, metadata

# Standalone provision-time verification (fetch + re-hash + schema scan) without
# running a script — an availability/integrity check.
eigenius data verify <iri>
```

- **`attach`** is the only verb that reaches the bytes (via the substrate, §5/§10); `list`/`inspect` are pure graph queries; `verify` invokes the substrate check on demand.
- A **`DatasetSchema`** (§4) is an ordinary resource — committed via `eigenius load` and referenced by `--schema` (a `data schema …` convenience is possible but not core).
- **Provision and capture are *not* CLI verbs** — they happen inside `eigenius run` / a script run (the consume step). The `data` family is only the attach side of the lifecycle.

## 8. Smaller inputs: optional de-duplication

The same `PinnedExternalFile` path applied to *small* already-pinned sources closes a real wart in the wrapped-R warrants. Today the WRN lme4 programs read **inlined** input tables (`programs/invivo/km12-competition-input.json`, `xenograft-input.json`) that re-transcribe data already pinned elsewhere: `km12-competition-input.json` carries the same ED Fig 3b bytes as `viab_KM12_sampleset`, just reshaped flat — **two transcriptions of one already-checksummed slice**.

Routing the program's input through a `PinnedExternalFile` that references that **one pinned source** (the xlsx slice, or a small pinned CSV slice of it) removes the duplicate: the script reads the genuine pinned bytes, and the only remaining on-chain copy is the `SampleSet`'s inlined values — which D52's pure-Rust path *must* read on the chain, and which is a legitimate Observed→Derived projection of the same source (verified by the `--check` recipe), not an independent hand-transcription.

This **supersedes** an earlier idea (have the program read the committed `SampleSet` by IRI): that needed the R worker to decode the nested `stats:Nested(...)` term, whereas reading a CSV/xlsx is what the extraction tooling already does — strictly less work, and it makes the program's input *provably the genuine source bytes* rather than a curated reshape.

**The trade-off, made deliberate.** Externalizing a small input moves its bytes off-chain, adding an availability dependency to re-run that program — versus inlining, which keeps the bytes on-chain and EigenQL-queryable. So this is the right call only when the bytes are *already* pinned out-of-band (the WRN case: the source slice is already an external pinned artifact, so reading it directly *removes* a copy rather than *adding* a dependency). For data that exists only inline, on-chain remains preferable. `PinnedExternalFile` is the uniform input-provenance mechanism; whether a given small input uses it is a per-input judgement, not a blanket "externalize everything."

## 9. Boundary / out of scope

Deliberately narrow. D53 is *only* the large-data tracking + provision path. Explicitly **not** D53:

- **Small-data extraction (raw → `SampleSet`).** Column-filter-sort over a checksummed slice that inlines a small array is handled by the committed `extract_samplesets.py` recipe + its `--check` pin. It is not large data and needs no external tracking; it stays as is.
- **Analysis numerics.** Native statistical recompute is D52 (kernel re-derivation); runtime-hosted analysis (limma, lme4, fgsea) is D56 wrapped-R. D53 supplies the *large input*; it does not own the computation.
- **Script execution itself.** The substrate, worker model, image pinning, and cross-check are D26 / D55. D53 reuses them.
- **Format-specific parsers.** Owned by the worker ecosystem, by design.
- **The Oxen deployment / blob-store implementation.** Owned by Oxen (and D44 data lifecycle for retention/GC of cached materializations).

## 10. Open questions

- **Oxen access in the worker (resolved — use a prebuilt client, not the crate).** Embedding `liboxen` as a Rust dependency is rejected: it pulls ~160 crates (actix, arrow, polars, …), wrong for the Eigenius build and TCB. The client functionality ships prebuilt, so the weight lives in the *worker image*, not our build: the **`oxen` CLI binary** (`brew install oxen` / release binary; shell out `oxen download <repo>@<commit> <path>` — language-agnostic, the default for the R worker) or the **`oxenai` Python wheel** (`pip install oxenai`; maturin/pyo3, `liboxen` compiled in; `RemoteRepo().download(...)`). The raw HTTP protocol is *not* a documented option — `oxen-server` exposes routes but there is no published spec/OpenAPI; it's the private `liboxen`↔server wire protocol, so a hand-rolled HTTP client would reverse-engineer a moving target. Eigenius recomputes its own `content_hash` over the materialized bytes regardless, so the Oxen client stays in the **availability TCB**, never the correctness TCB. The client lives in the **substrate (orchestrator) image, not the worker images** — see §5: the substrate fetches + verifies once and hands the worker a depot path, so no runtime reimplements Oxen. **Auth (mechanism settled).** Oxen uses a **per-host bearer token** (an OxenHub API key, or a self-hosted `oxen-server` access key) stored in `auth_config.toml` (`host → auth_token`) under the oxen config dir — default `~/.config/oxen/`, overridable by `$OXEN_CONFIG_DIR` — and sent as `Authorization: Bearer <token>`. Public repos read tokenless; private repos require the token; tokens are host-scoped (one per host/account). There is no token env var, so the substrate injects it by writing the secret into an `auth_config.toml` under a substrate-owned `$OXEN_CONFIG_DIR`. The token is a **deployment secret held substrate-side only** (platform secret store / mounted secret), never in worker images — the credential never leaves the orchestrator, reinforcing the substrate-side-fetch decision. *Open is only the ops policy:* which secret store, rotation, and per-host config for multi-host deployments.
- **Local content-addressed cache (settled).** The depot's `sha256`-keyed materialization dir (§5) *is* the cache. Dedup is by construction — **same `content_hash` → same file**, so a present entry is reused with no re-fetch and re-running is a no-op. Eviction is **LRU with in-use pinning**: an entry a running task depends on is never evicted; LRU reclaims the rest. The *only* hard limit is when the files needed by **concurrently in-flight tasks exceed available disk** — a **scheduler/resource-accounting** concern (don't admit more concurrent tasks than their inputs fit), not a cache-design one. Retention/GC of idle entries is the only true cache policy → D44.
- **Reference scheme (backends settled — §3.1).** Two backends: `oxen://repo@commit/path` (versioned/large/remote) and `file://<volume>/<path>` (a plain disk volume/folder — the no-Oxen fallback, matching today's `data/slices/` + `MANIFEST.md`). `content_hash` is the backend-independent trust root. *Remaining:* the exact canonical form of each locator (Oxen `repo@commit/path` grammar; `file://` volume-relative vs absolute; whether a bare `sha256:` key resolves against a configured store dir) — a small spec, not a design fork.
- **Execution idempotence (settled).** Distinct from the *file* cache above: identical `(input content_hash, script IRI, image_digest)` converges to one **output** IRI and skips re-execution — the existing anchored-commit / mirror-IRI dedupe discipline applies directly, no new mechanism.
- **Schema authoring + representation (§4 — model settled).** Resolved by the §4 rewrite (prior-art survey, [notes](../notes/d53-dataset-schema-prior-art.md)): a **reusable `ingest:DatasetSchema`** (QB DSD analog) declaring dimensions (class/code-list-bound), measure(s) (property-bound), and per-component attributes, plus a **per-file physical layout binding** (Croissant `source`-style) that separates semantics from wide-vs-long layout; the entity-per-column header rule is an **explicit** parse mapping (not name-suffix), and the layout is **checkable** by a header/key scan. *Remaining (detail, not shape):* the concrete property/IRI spelling of the schema vocabulary — which routes through [D57](d57-schema-org-vocabulary-mapping.md) and the QB/Croissant term mapping — and the exact code-list representation for entity-per-column dimensions. *Refinements surfaced by the [WRN worked example](../notes/d53-wrn-attachment-worked-example.md):* (a) **multiple schemas per file** — a `PinnedExternalFile` should carry a *set* of `DatasetSchema`s keyed by an intra-file selector (`.rds` member / `.xlsx` sheet), à la Croissant RecordSet-per-FileObject, not a single binding; (b) a **collection profile** for ragged non-cube data (a gene-set `.gmt`: `set → variable list of entity refs`); (c) **code-list source tables** (e.g. `sample_info.csv` defining `DepMap_ID ↔ onco:CellLine`) are themselves attached `PinnedExternalFile`s serving as `references` FK targets; (d) the layout binding must record **axis orientation** (DRIVE is Achilles transposed — same semantic cube, flipped layout).
- **File-level descriptive metadata (settled: require none beyond the functional minimum).** D53 *requires* only the load-bearing fields — `reference`, `content_hash`, `media_type` (§3) + inherited `reflection:source` — which make a file fetchable, verifiable, readable, and traceable. All other metadata (`content_size`, `license`, `creator`/`source_organization`, `source_identifier` DOI/URL, `original_checksum`, `date_published`, `is_part_of` — the provenance now stranded in `data/MANIFEST.md` prose) is **`recommends`, added per-need, not required**, and **not hand-named here**: its canonical form is D57's `urn:schema_org:` vocabulary (`schema_org:contentSize`, `schema_org:license`, …), so D53 keeps the descriptive slot open and extensible and lets D57 define the field set rather than minting `ingest:*` names D57 would have to replace.
  - *Note — see [D57](d57-schema-org-vocabulary-mapping.md).* The descriptive fields route through D57's `urn:schema_org:` vocabulary. The **minimum slice D53 needs is defined in [D57 §2.5](d57-schema-org-vocabulary-mapping.md)**: ~10 hand-authored string-typed properties (`name`, `description`, `contentSize`, `encodingFormat`, `license`, `creator`, `sourceOrganization`, `identifier`, `datePublished`, `isPartOf`) — no classes, no generator, no type-mapping machinery. D53 functionally needs *none* of it (its required fields are `ingest:`-native); the slice is purely for moving the `MANIFEST.md` provenance onto the node.
- **RO-Crate interop is tooling, not Eigenius proper.** An Eigenius derivation (`PinnedExternalFile` inputs + `RuntimeScript` + `RuntimeInvocation` + result) maps closely onto a **Workflow Run RO-Crate**, and a vendored dataset's own RO-Crate could populate the §4/metadata fields on import. But this is a **converter that sits outside the platform** — it reads/writes the boundary (chain artifacts ⇄ descriptive JSON-LD) without touching the kernel, the typed graph, or the correctness TCB. It needs no platform design and no D-series memo; build the export/import tool if and when FAIR deposit (WorkflowHub/Zenodo) is wanted. The only thing Eigenius proper owes it is that the on-node metadata (above) be expressible — which the schema.org-aligned field names already give.

## 11. Relationship to D52, D55/D56

D53 is the **input layer**; the computation layers sit above it and are chosen by *method*, not by where the bytes live (§6, the storage ⊥ grade principle).

- **D52 (native recompute)** — deterministic Rust over a dataset. Its input is a `SampleSet` whose values are **inline *or* a `PinnedExternalFile` reference**; in the latter case the substrate materializes the file (§5) and the same numerics read the materialized array. Native-grade at any size. So D53 feeds D52 just as it feeds D56 — a large `SampleSet` is a D53 + D52 composition, not a different tier.
- **D55/D56 (wrapped runtime)** — the R runtime and wrapped execution (proven live for the xenograft + competition lme4 warrants). Inputs are chain-resident Eigon-CBOR (small) *or* `PinnedExternalFile` (large), materialized identically.

D55 §9 deferred the large-data input question to "a D53 revision, to land before D-DIFF" — this memo is that revision.

D-DIFF (limma) is the first *large-data* consumer: a `RunRuntimeScript` reading the CRISPR/DRIVE matrices as `PinnedExternalFile` inputs and committing the small differential-dependency result — lifting `dd_achilles`/`dd_drive` from linked-external to reproduced-external. It is **D56-grade because limma's eBayes is a wrapped tool, not because the matrices are large** — a native method over the same matrices would be D52-grade. xenograft and GSEA do not need D53 today only because their inputs happen to be small/moderate and chain-resident.

---

*Implementation:* a detailed, codebase-mapped, phased build plan is in [docs/notes/d53-implementation-plan.md](../notes/d53-implementation-plan.md); the concrete WRN attachment worked example is in [docs/notes/d53-wrn-attachment-worked-example.md](../notes/d53-wrn-attachment-worked-example.md); the schema prior-art survey in [docs/notes/d53-dataset-schema-prior-art.md](../notes/d53-dataset-schema-prior-art.md).
