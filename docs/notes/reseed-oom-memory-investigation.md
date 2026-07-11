# Reseed OOM — memory investigation (RESOLVED: environmental, not a kernel bug)

**Status: RESOLVED (`2026-07-07`).** The kernel is **not** the problem. A native `serve` load of the
identical WordNet(`--all`, C3-precision) + all 27 UMLS chunks — same binary, same chains — **peaks at
~6 GiB RSS and completes**, producing a full 2.6 GB store at `/tmp/probe-db`. jemalloc heap profile at
the high-water mark: **live heap 3954 MB**, dominated by BTreeSet/BTreeMap cloning in the load path
(`::load::{{closure}}` → `clone_subtree` 2719 MB / `from_iter` 1735 MB) — i.e. the `defined_iris` churn
of §4, ~2.7 GB, **not 15 GB**. So the docker ~20 GiB OOM (exit 137) is **WSL2 VM memory pressure**:
page cache from the 2.6 GB of RocksDB writes + `docker compose build` residue + docker overhead, all
counted inside the WSL2 VM's memory cap — which is why it tracked *host* headroom ("21 GiB avail → OOM,
24–25 → success"), not any per-chunk kernel growth.

**Fixes.** (1) For a full-lexicon store now: **reseed natively** (`eigenius serve --db <path>` + load the
chains; ~6 GiB, no docker) — done, `/tmp/probe-db`. (2) For reliable *docker* reseeds: raise the WSL2
memory cap (`~/.wslconfig` `memory=28GB`), or reseed natively and import into the volume. (3) Real but
secondary optimisation (not the OOM): `Arc` the per-layer `defined_iris` sets instead of cloning them in
`build_chain`/`load_chain` (§4) — cuts the ~2.7 GB clone churn + the O(chain²) per-commit re-read.

**Method note:** the jemalloc harness is `cli` feature `jemalloc-prof` (feature-gated
`tikv-jemallocator`, `#[global_allocator]` in `cli/src/main.rs`); run `eigenius serve` under
`_RJEM_MALLOC_CONF=prof:true,prof_active:true,lg_prof_interval:31,prof_prefix:…`; analyse with
`perl <tikv-jemalloc-sys OUT_DIR>/bin/jeprof --text <binary> <dump>.heap`. Harness script:
scratchpad `profiled_load.sh`.

---

_Original investigation below (§3 candidates were all measured-out en route to the resolution above)._

**Do not re-tread §3** — those candidates are measured-out, not guesses.

**Symptom.** `scripts/reseed-lexicon-db.sh` (WordNet `--all` + UMLS `--all` into the dockerised
`eigenius serve` kernel) OOMs (exit 137, SIGKILL) **deep into the UMLS load** — established this
session at **~umls-020** on a run that started with ~21 GiB available. The same reseed **succeeds**
when it starts with ~24–25 GiB available (the `2026-07-07-c3` snapshot loaded clean). So the failure
is **pressure-dependent** and **cumulative**: it survives WordNet + ~19 UMLS layers and dies at the
~20th. User report: OOM began **after text-indexing of `description` was turned on**.

**The gap this note exists to close.** Every resident term I can name by reading the code sums to
**~5–7 GiB**, against a ~20 GiB OOM. The dominant allocator is **not** in any structure static
analysis surfaced. Only a live heap profile will name the owner.

---

## 1. Load structure (both lexica chunked identically)

- WordNet and UMLS are **both** emitted as `--out-dir` partitioned chains at the same
  `--split-bytes` ≈ **100 MiB per layer** boundary.
  - `wordnet-chain/`: `wordnet-000-base.esl` (16 MiB decls) + **3** entry layers × ~100 MiB.
  - `umls-chain/`: `umls-000-base.esl` (16 KiB) + **27** entry layers × ~100 MiB.
- Load order: WordNet chain first, then UMLS, via `$CLI load <file>` into the running service
  (`scripts/reseed-lexicon-db.sh` §5). The **service** process is what OOMs.
- WordNet (3 layers) loads fine; the failure is ~20 layers into UMLS. **Re-chunking WordNet, or
  finer chunking of either, does not help** — more layers = the same total resident state + more
  per-layer metadata; a cumulative baseline is chunk-count-invariant.

## 2. Measured facts (decisive)

**RocksDB pinned memory on the loaded c3 snapshot** — via `storage/rocksdb/tests/snapshot_memory_probe.rs`
(`#[ignore]`; `EIGENIUS_DB_SNAPSHOT=<store> cargo test -p eigenius-storage-rocksdb --test snapshot_memory_probe -- --ignored --nocapture`):

| CF | table-readers-mem (pinned RAM) | live-sst-size | num-keys |
|---|---|---|---|
| default (resources + triples + value index) | 293.8 MiB | 2223.4 MiB | 97,387,765 |
| **cf_text** (the description index) | **9.2 MiB** | 306.8 MiB | 6,398,174 |
| **TOTAL pinned** | **303.0 MiB** | 2530.2 MiB | — |

block-cache-usage 0, cur-size-all-mem-tables 0 (idle snapshot). The whole store is **2.5 GiB on
disk**; the process OOMs at ~20 GiB → an **~8× blow-up that is entirely kernel-side / in-RAM**, not
storage.

Reading of the store: `cf_text` (descriptions) is only **12 %** of the DB and pins **9 MiB** — the
text index did **not** balloon storage. The bulk is the **default CF** (2223 MiB / 97 M keys), which
includes the `description` strings embedded in every resource's serialization.

## 3. Ruled out (with evidence — do not re-investigate)

1. **Per-chunk text-index batch transient** (`populate_text_indexes`, `kernel/src/query/text/indexing.rs:72`).
   Real: it accumulates the whole layer's tokenised descriptions in `batches` before one
   `extend_layer` flush (~1 GiB for a 300 k-resource chunk). **But it is freed each chunk** — a
   per-commit spike, not cumulative. Not the driver. (A streaming-flush fix — "C1" — would trim this
   spike but not the OOM.)
2. **`resolve_active_text_indexes` O(chain) scan** — *retracted.* `scan_chain`
   (`kernel/src/layer/index.rs:812`) is **POS-index-accelerated** (`scan_predicate_object` prefix
   scan, RocksDB bloom), returning only the `(is_a, core:TextIndex)` entries. Cost is
   `collect_ancestors` (O(#layers) pointer walk) + a tiny index scan. Not a resource walk.
3. **RocksDB default config pinning index/filter blocks unboundedly** — *measured wrong.* Pinned
   table-readers = **303 MiB** for a 2.5 GiB DB (§2). RocksDB config is all-defaults
   (`storage/rocksdb/src/lib.rs` `open`: Lz4 only, no block cache, `cache_index_and_filter_blocks`
   false, `max_open_files` -1) but at this DB size that is not the driver.
4. **Kernel Rocks indexes accumulating in RAM** — no. Every `Rocks{Text,Triple,Value,Vector}Index`
   struct is `{ db: Arc<DB>, <atomic counters> }` (`storage/rocksdb/src/*.rs`). Pure RocksDB writers;
   `extend_layer` writes a `WriteBatch` and returns. Nothing per-layer retained in process RAM.
5. **In-memory backend instead of RocksDB** (the "we used an in-memory structure" lead) — not active
   in the server. Every production commit path builds on **`LayerStorage::with_persistent`**
   (`kernel/src/server/{load,lifecycle,mod,consolidate,branches,gc}.rs`), which wires
   `text_index = pb.text_index_arc()` (RocksTextIndex), `backend = pb` (RocksDB), and a **bounded**
   cache (`storage.rs:213`). The `in_memory()` constructor is **test-only** (topology.rs:430/450).
   cf_text having 6.4 M postings on disk confirms the text index reaches RocksDB.
6. **Unbounded resource cache** — bounded. `with_persistent` uses `BoundedResourceCache::new(cache_budget())`;
   `DEFAULT_CACHE_BUDGET_ENTRIES = 250_000` (`kernel/src/layer/storage.rs:64`); the reseed's `serve`
   does **not** pass `--cache-budget`. Caveat: the bound is an **entry count, not bytes** — fattening
   each resource with a `description` raises the cache's byte footprint at a fixed entry budget — but
   250 k entries × even a fat 2–5 KiB resource is only ~0.5–1.25 GiB. Not the driver.
7. **Commit working set materialising the chain** — no. `CommitWorkingSet`
   (`kernel/src/validation/working_set.rs:18,168,267`) accumulates **IRI dedup sets** (~2 copies per
   IRI), not resource bodies.

## 4. Resident, confirmed — the ~5–7 GiB that *is* accounted for

- **`defined_iris` per layer, for every layer in the chain.** `RocksStore::load_chain`
  (`storage/rocksdb/src/lib.rs:289–307`) calls `list_layer_iris` per layer into
  `ChainInfo.defined_iris_per_layer: BTreeMap<LayerId, BTreeSet<Iri>>` (`kernel/src/storage/mod.rs:93`);
  `build_chain` (`kernel/src/layer/mod.rs:98–119`) clones each into `Layer.defined_iris`
  (`mod.rs:295`); the `Arc<Layer>` chain (`parents: Vec<Arc<Layer>>`, `mod.rs:288`) keeps them all
  alive. `Iri` is `pub struct Iri(String)` (`kernel/src/ontology/iri.rs:34`). ~6–7 M IRIs × ~100 B
  ≈ **~1 GiB**, growing with chain depth. Note also: `load_chain` rebuilds the **entire** map each
  call → O(chain) re-read per commit → **O(chain²)** across a load (matches the documented
  "per-chunk time grows with chain length"). Time + transient churn, plus a transient 2nd copy per
  commit.
- **RocksDB pinned:** ~0.3 GiB (§2).
- **Per-chunk transient** (freed each commit): the `LayerBuilder`'s `BTreeMap<Iri, Resource>` for the
  ~100 MiB chunk + the parsed ESL AST + the text-index `batches` + the validation working set —
  ~2–4 GiB peak per chunk. The built `Layer` has **no `resources` field**, so bodies are not retained
  (lazy from the backend).

**Sum: ~5–7 GiB.** Missing: **~13–15 GiB.**

## 5. Hypotheses still standing (unmeasured)

- A resident structure **not surfaced by reading** the storage/commit path — the reason to profile.
- Something in the **validation / ref-resolution** path (Rule 22 closed-world reference check;
  `build_axiom_env`) that holds O(chain) **resource bodies** transiently but at a high-water mark
  overlapping the per-chunk peak. The memory notes flag a *"full-chain-resident OOM still open"* on
  the reference-integrity path — candidate.
- Fragmentation / allocator retained pages (the system allocator not returning freed per-chunk
  transients to the OS) — a jemalloc profile distinguishes live-heap from RSS.

## 6. Next step — jemalloc heap profile (the plan)

Tooling on this machine: **no** heaptrack, **no** system jemalloc, **no** jeprof installed; 31 GiB
total RAM. Because RocksDB's C++ side is measured small (§2), the missing ~15 GiB must be on the
**Rust heap**, which `tikv-jemallocator` as the global allocator captures fully (its C++ blindspot
does not cost us here).

1. Add `tikv-jemallocator = { version = "0.6", features = ["profiling"], optional = true }` to
   `cli/Cargo.toml` behind a `jemalloc-prof` feature (the `serve` binary is `eigenius` from
   `eigenius-cli`, `cli/src/main.rs`).
2. In `cli/src/main.rs`: `#[cfg(feature = "jemalloc-prof")] #[global_allocator] static ALLOC:
   tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;`. Feature-gated → zero impact on
   normal builds/tests/CI.
3. `cargo build --release -p eigenius-cli --features jemalloc-prof`.
4. `_RJEM_MALLOC_CONF=prof:true,prof_active:true,lg_prof_interval:30,prof_prefix:/tmp/probe/jeprof \
   target/release/eigenius serve --db /tmp/probe-db --port 50051` — dumps a heap profile every
   ~1 GiB allocated (`lg_prof_interval:30`). **`prof_final` will not fire** — OOM is SIGKILL — so the
   interval dumps are the capture mechanism.
5. Load `wordnet-chain/*.esl` then **~10** `umls-chain/*.esl` via `eigenius --endpoint … load <f>`.
   No need to reach OOM — the dominant owner is the widest bar well before then. `wordnet-chain/` and
   `umls-chain/` already exist on disk from the last reseed.
6. `jeprof --svg target/release/eigenius /tmp/probe/jeprof.<largest>.heap` → flame graph; the top of
   the widest stack names the owner (`Iri::parse`/`BTreeSet::insert` under `load_chain` ⇒ the IRI
   sets; a cache/backend insert ⇒ resources held; the ESL parser ⇒ per-commit working set).
   `jeprof` ships in the jemalloc build (`tikv-jemalloc-sys` OUT_DIR) if not otherwise available.

Keep the load **bounded** (~10 UMLS chunks) so it never approaches the OOM ceiling.

## 7. Key files

- Reseed / load: `scripts/reseed-lexicon-db.sh`; `docker-compose.yml` (kernel `serve`); the serve
  binary `cli/src/main.rs` (`eigenius-cli` → `eigenius`); `deploy/Dockerfile.kernel`.
- Text-index population: `kernel/src/query/text/indexing.rs:72` (writes `layer.storage().text_index`,
  line 150); `kernel/src/layer/mod.rs:1170` (`populate_text_indexes` in the persist path).
- Storage wiring: `kernel/src/layer/storage.rs` (`LayerStorage`, `with_persistent{,_bounded}`,
  `DEFAULT_CACHE_BUDGET_ENTRIES=250_000`); `storage/rocksdb/src/lib.rs` (`RocksStore`, `store_layer`,
  `load_chain`); `storage/rocksdb/src/text_index.rs` (`RocksTextIndex`, `extend_into_batch`).
- Resident IRIs: `kernel/src/layer/mod.rs` (`Layer.defined_iris`, `build_chain`);
  `kernel/src/storage/mod.rs:93` (`ChainInfo.defined_iris_per_layer`); `kernel/src/ontology/iri.rs:34`.
- Diagnostic: `storage/rocksdb/tests/snapshot_memory_probe.rs`.
