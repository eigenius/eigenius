# D53 — Data Ingestion Institution

*Status: design memo · June 2026*

*Companion documents: [D26 runtime substrate](d26-runtime-substrate.md), [D27 Julia institutions](d27-julia-institutions.md), [D28 Lean-4 as institution](d28-lean-4-as-institution.md), [D31 external institution lifecycle](d31-external-institution-lifecycle.md), [D49 ChainWitness machinery](d49-chainwitness-machinery.md), [D52 measurement statistics institution](d52-measurement-statistics-institution.md).*

*This memo specifies how external raw-data files (a vendored DepMap matrix, a supplementary-table CSV, a `.rds`/HDF5/parquet blob) are brought onto the chain as typed resources — `SampleSet`s and the like — by a content-hash-pinned external file fed through an on-chain conversion script run on the runtime substrate, with the result returned as Eigon-CBOR and hashed into the chain. It closes the one link the recompute story currently leaves uncommitted: the projection from raw bytes to the structured resource a recompute institution then verifies. The premise mirrors D52's: do not ask consumers to trust a hand-extracted array; make the extraction a reproducible, content-addressed execution the chain can re-run and check.*

---

## 1. Motivation — the uncommitted link

D52 closes the audit chain from a `SampleSet` down to a recomputed statistical claim: the kernel re-derives the statistic from raw replicates and attests it. But the `SampleSet` itself is, today, **hand-extracted** from a larger raw file — select a column, filter rows, drop missing, sort, group — and the resulting array is inlined into ESL by an author. The raw file is checksummed (a `sha256` in a manifest); the inlined array is committed (it is the chain); but the *projection between them* rests on "trust this extraction script I ran."

```
raw checksummed file  ──[ column + filter + sort + group ]──>  inlined SampleSet  ──[ D52 ]──>  recomputed claim
      ✓ pinned                    ✗ uncommitted recipe              ✓ committed              ✓ kernel-verified
```

The WRN-helicase encoding made this concrete (see `experiments/publications/wrn-helicase/`): three `SampleSet`s inlined from DepMap slices, with a committed Python extractor + an `#[ignore]`d verification test as a *Tier-1 pin* — a recipe re-run by hand, not by the kernel. This memo specifies **Tier 2**: lift the extraction into a kernel-orchestrated, reproducible execution so the `SampleSet` becomes a `DerivedResource` whose derivation is a re-runnable `RuntimeInvocation` witnessed by the input file's content hash.

### Governing principle

> **Recompute the science natively; pin-and-reproduce the plumbing.**

- **Analysis** (SampleSet → statistical claim): the transform *is* the scientific claim, so the kernel re-derives it in its own code (D52). Trust root = the kernel.
- **Ingestion** (raw file → SampleSet): a column-filter-sort over arbitrary external formats is plumbing, not a claim worth independent native re-derivation — and reimplementing every parser (CSV, xlsx, `.rds`, HDF5, parquet, domain binaries) would drag all of that into the kernel's trusted computing base. So ingestion is a **reproducible external execution**: trust root = a content-addressed program, in a pinned environment, on a content-addressed input, producing a content-addressed output that anyone can reproduce.

These are two warrant *grades*, not two implementations of one thing. Ingestion's grade is "faithful, reproducible transform of pinned data" — the right guarantee for plumbing, weaker than D52's "proven by re-derivation."

## 2. Where this sits — reuse, don't reinvent

This is **not** a new execution stack. It is a new *consumer* of the D26 runtime substrate, alongside Julia (D27) and Lean (D28/D31). The substrate already provides every load-bearing mechanism:

| Need | Existing mechanism (file) |
|---|---|
| On-chain conversion script, content-addressed | `RuntimeScript` (IRI hashed from `language + source + entry_point + entry_point_signature`), `ontologies/runtime/runtime-substrate-ontology.json` |
| Push to substrate for execution | `SubstrateDispatcher::dispatch_run_runtime_script` (`runtime-substrate/src/facade.rs`) → `LanguageRuntime::run_script` (`crates/eigenius-julia/src/runtime.rs`) → worker over UDS, CBOR-framed |
| Result back as CBOR | worker `DispatchOk{output: CBOR}` → `DispatchOutcome.output_cbor` (Eigon-CBOR) |
| Hashed into the chain | committed Resource folded into the layer hash via `eigon_cbor::canonicalize` (`kernel/src/ontology/eigon_cbor.rs`) |
| Hermetic, pinned environment | `ImageDigest` (`sha256:…` OCI), `Project.toml`/`Manifest.toml` lockfile, **host↔container manifest-hash cross-check** (`runtime-substrate/src/cross_check.rs`) |
| Re-runnable execution record | `RuntimeInvocation` linking `script + environment + inputs + output + image_digest + timestamps + numerical_metadata` |
| Output marked derived | facade stamps `reflection:DerivedResource` + emits `InstitutionEmittedDerivation` (`facade.rs`) → D49 admits `IsDerivedAs` |

The hermetic-execution burden — the part this author flagged as the hard problem when the approach was first weighed — **is already paid** by the substrate: images are pinned by digest and the worker refuses to start if its environment manifest hash disagrees with the substrate's. "Re-run and get the same bytes" already has teeth.

## 3. The one genuine gap — a content-hash-pinned external file input

Substrate inputs today are **chain-resident resources** (Eigon-CBOR). There is no first-class notion of an **external input file pinned by content hash**. Two stubbed hooks already point at the shape:

- `LibraryContent::External { reference, content_hash }` (D26 §7.2, deferred) — used only for mirror archives today.
- `RuntimePackage.source_tree` (`data_type: json`) — documented as "an embedded archive **or** a content-addressed reference to external storage."

**Proposal:** a new resource class

- `ingest:PinnedExternalFile` — `requires`: `reference` (a locator: blob-store IRI / URL / content-addressable store key), `content_hash` (`sha256:…`), `media_type`/`format` (e.g. `text/csv`, `application/x-hdf5`). The committed, content-addressed stand-in for a raw slice. Its IRI is derived from its `content_hash` (byte-identical files converge, as mirror IRIs do).

The substrate **fetches by `reference`, verifies the bytes hash to `content_hash`, fails closed on mismatch**, and materializes the file into the worker's sandbox at a known path the script reads. This reuses the `LibraryContent::External` fetch-and-verify discipline and the `boundary.rs` pre-dispatch check pattern; it does not invent a new trust mechanism.

The bytes themselves are **not** committed to the chain — only the `content_hash` + `reference` are. This preserves the D50 §9.1 data-vendoring stance (large slices stay out-of-band; the content address travels). The chain commits the *recipe and the result*, not the raw input.

## 4. The ingestion plan — an AutoOnLoad gate

Mirroring D52's `StatisticalAnalysisPlan` (whose commit fires the statistics institution) and the external-institution `QueryClass` AutoOnLoad pattern (D31), a single committed resource is the trigger:

- `ingest:DataExtractionPlan` — `requires`: `input_file` (a `PinnedExternalFile` IRI), `script` (a `RuntimeScript` IRI, the conversion program), `output_class` (the expected result class, e.g. `stats:SampleSetResource`); `recommends`: `expected_invariants` (see §6).
- A `QueryClass` with `dispatch_role = auto_on_load` and `query_class = DataExtractionPlan` routes the plan to the substrate.

On commit the kernel: fetches + verifies the input file (§3); dispatches the script via `dispatch_run_runtime_script` with the materialized file path; receives the CBOR output; stamps it `DerivedResource`; writes a `RuntimeInvocation` whose `inputs` includes the `PinnedExternalFile` IRI (so the input content hash is *in* the execution record) and whose `output` is the emitted resource; and commits the output `SampleSet` at a deterministic IRI (e.g. `{plan_iri}:result`).

The emitted `SampleSet` is now a **`DerivedResource` derived from the raw Observed file**, carrying an `IsDerivedAs` witness (D49) — exactly the input shape D52 already consumes. The two institutions compose through the chain with no direct call, the established pattern.

This reclassifies the `SampleSet` from *Observed-with-a-recipe-sidecar* (Tier 1) to *Derived-from-raw-Observed* — the honest typing: a raw file is the genuine `ObservedResource`; the projection is `Derived`.

## 5. Warrant grade and the witness chain

The `RuntimeInvocation` is the warrant. It pins, all content-addressed:

- **input** — `PinnedExternalFile.content_hash`;
- **program** — `RuntimeScript` IRI (hashed from source + entry point);
- **environment** — `RuntimeEnvironment` + `ImageDigest` (interpreter version, lockfile, OS image);
- **output** — the committed `SampleSet`, hashed into the layer via `eigon_cbor::canonicalize`.

The claim it underwrites is: *"this exact program, in this exact environment, on this exact input, produces this exact output — reproducibly."* Anyone with the four hashes can re-fetch, re-run, and confirm. This is the same grade that, applied to limma in Phase 2.5, would upgrade `dd_achilles`/`dd_drive` from **linked-external (asserted)** to **reproduced-external (re-run and hash-checked)** — the natural endpoint for that frontier too. Ingestion and limma share one machinery.

## 6. Cheap native invariant gate (the Lean correspondence-check analog)

Lean's institution does not only trust the prover: after `check_proof`, it recomputes `library_content_hash` over the embedded mirror and checks anchor consistency (D28 §5.5). The ingestion analog: after the substrate returns the `SampleSet`, the kernel runs a **cheap native gate** without reimplementing the parse —

- recompute the output's content hash (free; it is committed anyway);
- optionally assert declared invariants from `expected_invariants`: the group sizes / `n` match, values lie in a declared range, the declared filter predicate holds on the *emitted* values (a linear scan, not a re-parse).

Reproducible external execution **+** a thin native invariant gate is the strongest grade short of full native recompute, at negligible cost. Invariants are optional and declared; absent, the warrant is reproduction alone.

## 7. Runtime is swappable

The conversion script's runtime is a `LanguageRuntime` choice, not part of the contract. A `PythonLanguageRuntime` (the WRN Tier-1 extractor is already Python — a natural first worker), an `R` runtime (the WRN paper's own tooling), or the existing Julia runtime (shared with Phase 2.5 limma) are sibling implementations + a worker-bootstrap analog of `julia/runtime-worker/src/JuliaWorker.jl`. The architecture is runtime-agnostic; the choice is which worker image is pinned. Format-diversity (the reason ingestion is external rather than native) is handled in the worker's ecosystem, not the kernel.

## 8. Relationship to the WRN Tier-1 pin

Tier 1 (already in tree) is the on-ramp, and maps one-to-one:

| Tier 1 (today) | Tier 2 (this memo) |
|---|---|
| `extract/extract_samplesets.py` (committed sidecar) | the `RuntimeScript` `source` |
| `bench:extracted_from_sha256` field | `PinnedExternalFile.content_hash` |
| `bench:extraction_columns` / `extraction_filter` | the script body + `expected_invariants` |
| inlined `stats:sample_set_value` arrays | substrate-emitted output, `RuntimeInvocation`-witnessed |
| `#[ignore]`d `--check` test (hand re-run) | kernel AutoOnLoad re-execution |

Migration is incremental: the Tier-1 provenance fields and verification test stay valid throughout and can be retired per-SampleSet as each moves to a `DataExtractionPlan`.

## 9. Open questions / deferrals

- **First runtime.** Python (fastest path; matches Tier-1) vs R (paper-native) vs Julia (limma-shared). Likely Python first, since ingestion is generic and the limma path is its own (Julia) work.
- **External blob store.** What `reference` resolves against — a local content-addressed cache keyed by `sha256`, a blob-store IRI, a URL with on-fetch verification. Reuse whatever D26 §7.2 settles for `LibraryContent::External`; do not invent a parallel.
- **Environment pinning for interpreted scripts.** A full OCI image per ingestion script is heavy for "run a 60-line CSV reader." Whether a lighter pinned-interpreter lockfile suffices, or the existing image pipeline is simply reused, is an implementation call — but the *witness* (image digest or lockfile hash) must remain content-addressed.
- **Plan vs orchestrator-driven.** §4 proposes an AutoOnLoad `DataExtractionPlan` for chain-resident reproducibility. A CLI-driven `eigenius ingest` that commits the same `RuntimeInvocation` is an acceptable bootstrap; the resource shapes are identical.
- **Multi-file joins.** The WRN RecQ SampleSet joins three slices (matrix + sample-info + supp table). `DataExtractionPlan.input_file` should be an array of `PinnedExternalFile`, each verified; the join logic lives in the script.
- **Caching / idempotence.** Identical `(input hashes, script, image)` should converge to one output IRI and skip re-execution — the mirror-IRI dedupe discipline applies directly.
- **Program inputs reading directly from committed SampleSets (de-duplication).** The wrapped-R RuntimeScript inputs (the WRN lme4 xenograft + KM12-competition tables, D56) currently carry their own Tier-1 pins *and re-inline data that already exists, pinned, in a committed SampleSet* — e.g. `wrn:viab_KM12_competition_table` is the same ED Fig 3b bytes as `wrn:viab_KM12_sampleset`, just reshaped flat. That is two pinned copies of one datum (verified mutually consistent by `extract_samplesets.py --check`, but still a duplication). The single-source-of-truth fix is for the program to read its input **directly from the committed, content-addressed SampleSet by IRI** — via the D26 §6.2 / B.5 `eigenius script run --inputs <iri>` path (the kernel resolves the graph-resident input and hands it to the worker), eliminating the separate input table entirely. The blocker is worker-side marshalling: the SampleSet's payload is the structured `stats:Nested(...)` term, not a flat property array, so the R worker needs a mirror-struct decoder for it (today's `r_eigon_f64_array` only reads flat columns). Once the substrate can decode a chain-resident typed SampleSet, the flat input tables retire and the SampleSet becomes the one pinned source feeding both the statistics-institution warrant and the wrapped-R warrant. Tracked here rather than built now because it depends on the broader mirror-generator work (D26 §7) reaching the R runtime.

## 10. Out of scope

- Native re-derivation of extractions in the kernel (rejected by §1's principle — that is D52's domain, not ingestion's).
- The numerics of any downstream recompute (owned by D52 and successors).
- Format-specific parsers (owned by the worker ecosystem, by design).
- The blob-store implementation itself (owned by D26 §7.2 / D44 data lifecycle).
