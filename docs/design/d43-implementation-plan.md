# D43 Implementation Plan

## Status

All milestones shipped (June 2026). Benchmarks landed alongside the implementation notes; the v1 operating envelope is published in [d43-implementation-notes.md](../notes/d43-implementation-notes.md).

| Milestone | Status |
|---|---|
| M1 — Foundation | ✅ Shipped |
| M2 — Storage substrate | ✅ Shipped |
| M3 — Text retrieval | ✅ Shipped |
| M4 — Embedder Component | ✅ Shipped |
| M5 — Vector retrieval (flat) | ✅ Shipped |
| M6 — HNSW addition | ✅ Shipped |
| M7 — Similarity operator + hybrid retrieval | ✅ Shipped (post-surface-reset) |
| M8 — Consolidation + atomic reindex | ✅ Shipped |
| M9 — End-to-end validation | ✅ Shipped |

M9 deliverable detail:

| M9 item | Status | Lands at |
|---|---|---|
| M9.1 D35 §7.4 worked example | ✅ Shipped | [d35_se_retrieval_worked_example.rs](../../kernel/tests/d35_se_retrieval_worked_example.rs) + D35 §7.4 rewrite |
| M9.2 Life-science integration test | ✅ Shipped | [crates/eigenius-obograph](../../crates/eigenius-obograph/) + [d43_go_subset_integration.rs](../../crates/eigenius-obograph/tests/d43_go_subset_integration.rs) (real GO data, RocksDB backend) |
| M9.3 HNSW benchmark | ✅ Shipped (algorithm + semantic) | Algorithm-level: [d43_hnsw_recall_bench.rs](../../kernel/tests/d43_hnsw_recall_bench.rs). m-sweep at N=10k confirms graph connectivity (not `ef`) is the recall constraint: m=16 → 0.72, m=32 → 0.93, m=48 → 0.89 (non-monotone — neighbour-pruning gap). Build time scales ~O(N²) above 10k. v1 recommendation: declare `vec_hnsw_m = 32`. Semantic-level: [go_recall.rs](../../crates/eigenius-embedder-candle/tests/go_recall.rs) with [eigenius-embedder-candle](../../crates/eigenius-embedder-candle/) (BGE-small via Candle) — **recall@10 = 7/7 = 1.00** on 7 paraphrased biomedical queries against 1 007 GO Classes. |
| M9.4 Performance benchmarks | ✅ Shipped | [d43_perf_bench.rs](../../crates/eigenius-obograph/tests/d43_perf_bench.rs). Cold-start to queryable layer in ~2.3s for 52k GO Resources on RocksDB; 350-400ms per BM25 query (dominated by unfiltered MATCH scan); ~468 MiB net RSS. Numbers captured in implementation notes. |
| M9.5 User documentation | ✅ Shipped | EigenQL guide [chapter 6](../guides/eigenql/06-text-and-vector-retrieval.md) + ESL guide [§4.4a](../guides/esl/04-declarations.md#44a-text_index-and-vector_index) |
| M9.6 Implementation notes appendix | ✅ Shipped | [d43-implementation-notes.md](../notes/d43-implementation-notes.md) |

M7's surface differs from the original M1 plan: the seven function-shaped primitives + BIND + `TOP K BY` were collapsed into a single `~` operator with a `{ via:, model:, k:, limit: }` hint block, ranked by `TOP N` against the platform-internal RRF fusion. See D43 §3.3 / §3.4 for the surface and the surface-reset note in M7 for the rationale.

M8 ships the full set: the consolidation extension point lives in [consolidate.rs](../../kernel/src/layer/consolidate.rs); text-side re-extraction runs inside `LayerBuilder::build` via `populate_text_indexes`; vector-side concatenation + relabel + HNSW rebuild runs in [`consolidate_layer_vectors`](../../kernel/src/query/vector/indexing.rs); the reindex driver lives in [task/reindex.rs](../../kernel/src/task/reindex.rs); the post-build trigger (`detect_reindex_targets` → `SweepCoordinator::trigger_reindex_blocking`) lives in [task/sweep_registry.rs](../../kernel/src/task/sweep_registry.rs); search-equivalence under consolidation has both text and vector integration tests; in-flight reindex cancellation composes through the same registry as the sweep cancellation. The remaining gap is the commit-hook wiring at the server layer that calls `trigger_reindex_blocking` after each `put_branch`; that's an M9 concern.

M9.2 added the obograph importer ([crates/eigenius-obograph](../../crates/eigenius-obograph/)) — converts OBO Graphs JSON dumps (GO, ChEBI, etc.) to Eigon-JSON with HTTP → URN IRI rewriting, `core:source_irl` provenance, `is_a: [..., DeclaredResource]` epistemic tagging, and a synthesised shared OBO meta-vocabulary at [`ontologies/obo/obo-meta-ontology.json`](../../ontologies/obo/obo-meta-ontology.json) loaded by the bootstrap chain. Real GO (52k Classes) converts cleanly, loads into a RocksDB-backed kernel layer in ~3s release, and answers BM25 `~` queries against actual biomedical content.

Pre-existing core-ontology additions to support imports: `urn:eigenius:core:Resource` (catch-all super-class for `INDIVIDUAL`-shaped nodes) and `urn:eigenius:core:deprecated` (Boolean Property for the OBO `meta.deprecated` slot).

## Scope

Implement D43 v1 end-to-end:

- ESL `text_index` / `vector_index` declarations as first-class Resources (`core:TextIndex`, `core:VectorIndex`)
- Custom layer-aware inverted index in RocksDB (Phase 14h-aligned key schema; chain-aware BM25; no Tantivy dependency)
- Zero-copy CBOR vector segments with per-property `strategy: flat | hnsw | auto` and additive HNSW CBOR fields
- Embedder as a D3 Component with content-addressed cache and post-Load sweep
- Atomic-reindex policy for embedding-model upgrades
- EigenQL surface additions: `TEXT_MATCH`, `TEXT_SCORE`, `VECTOR_NEAR`, `VECTOR_SIM`, `EMBED`, `RRF`, `TOP K BY`
- New column families (`cf_text`, `cf_vec`, `cf_embed_cache`) under D24 schema-version bump

## Estimate

~12–18 weeks of focused work; ~8–12 weeks of wall time with parallel execution where dependencies allow. The estimate excludes time spent on D35-side integration (the consumer of D43's primitives) and on production-validation workloads.

## Dependencies

- [D23 — Out-of-Core Layer Architecture](d23-out-of-core-layer-architecture.md). Provides the per-layer shadowing bloom (§5.2), chain-walk primitives (`collect_ancestors`, `is_shadowed`, `scan_chain`), and storage abstractions D43 builds on.
- [D24 — Schema Versioning Policy](d24-schema-versioning.md). D43 implementation includes a schema-version bump under D24's mechanism for the new column families and key prefixes; per the no-legacy-DBs policy, pre-D43 DBs are refused at boot with no migration code.
- [D6 — Execution Architecture](d6-execution-architecture.md) and [D6b — Reasoning Trace Schema](d6b-reasoning-trace-schema.md). The IO Component dispatch envelope and trace-recording machinery D43's Embedder rides.
- [D21 — Task Traces and Checkpointing](d21-task-traces-and-checkpointing.md). The task surface D43's §5.5 sweep uses with cancel-on-`delete_layer` semantics.
- [D25 — Chain Consolidation](d25-chain-consolidation.md). D43 §2.8 plugs into D25's `consolidate_chain` with a new re-extraction extension point.
- [Phase 14h — Indexed Reads](phase-14h-indexed-reads.md). The architectural template D43's text and vector indexes mirror.

## Sequencing strategy

Milestones group into three streams that can partially overlap:

```
   M1 (foundation)
       │
       ▼
   M2 (storage substrate)
       │
       ├──▶ M3 (text retrieval) ─────┐
       │                              │
       ├──▶ M4 (embedder + EMBED) ──▶ M5 (vector flat) ──▶ M6 (HNSW)
       │                                                       │
       │                                                       ▼
       └────────────────────────────▶ M7 (RRF + hybrid) ◀──────┘
                                          │
                                          ▼
                                      M8 (consolidation + reindex)
                                          │
                                          ▼
                                      M9 (validation + benchmarks)
```

M3 and M4 run in parallel after M2. M6 (HNSW) can start whenever M5 (vector flat) lands; M7 (RRF) gates on both M3 and M5. M8 lands last among feature work; M9 wraps everything for acceptance.

---

## M1 — Foundation

**Goal:** Land all the non-functional preliminaries — schema, grammar, lexer, parser — so subsequent milestones plug into a kernel that knows about D43's surface even if no behavior is wired yet.

**Prerequisites:** None.

**Duration:** ~3–5 days.

**Deliverables:**

| Item | Location | Notes |
|---|---|---|
| `core:TextIndex` and `core:VectorIndex` Class definitions | [ontologies/core/core-ontology.json](ontologies/core/core-ontology.json) | Property set per D43 §3.1; `target_property`, `analyzer` (text); `model`, `dimensionality`, `distance`, `strategy`, `hnsw_params`, `embedding_policy` (vector). |
| ESL grammar additions | [kernel/src/esl/ast.rs](kernel/src/esl/ast.rs), [kernel/src/esl/lexer.rs](kernel/src/esl/lexer.rs), [kernel/src/esl/parser.rs](kernel/src/esl/parser.rs) | New `Declaration::TextIndex` / `Declaration::VectorIndex` variants; lexer keywords `TextIndex`, `VectorIndex`; parser dispatch following the existing class/property pattern. |
| EigenQL lexer keywords | [kernel/src/query/lexer.rs](kernel/src/query/lexer.rs) | New `TokenKind` variants for `TEXT_MATCH`, `TEXT_SCORE`, `VECTOR_NEAR`, `VECTOR_SIM`, `EMBED`, `RRF`, `TOP`. |
| ~~EigenQL parser stubs~~ | — | **Deferred to M3 (text) / M5 (vector) / M7 (TOP K BY).** Extending the expression AST and typechecker for these primitives is naturally part of the implementation milestones where the behaviour lives. M1's lexer-level reservation is what's load-bearing — it locks the keywords so they cannot be shadowed by user identifiers between now and the implementation milestones. Queries using the new keywords today fail with a parse error that includes the token kind, which is a meaningful "not yet implemented" signal until M3/M5/M7 land the AST extensions. |
| D24 schema version bump | [docs/design/schema-changelog.md](docs/design/schema-changelog.md) | Record the new column families and key prefixes; bump the kernel's compiled-in schema version. |

**Implementation guidance:**

- The four new column families (`cf_text`, `cf_vec`, `cf_embed_cache`) are declared at this milestone but not yet *populated*. RocksDB CFs must be declared at DB-open time, so the migration is wired now even if no writes happen yet.
- ESL grammar follows the existing `property` / `class` / `resource` block convention. The `target_property` field is an IRI; the rest is a simple key-value map.
- EigenQL keyword additions are mechanical. Verify no existing user identifiers shadow the new keywords (a grep of the codebase confirms none do).
- The parser produces AST nodes for `TEXT_MATCH` etc.; the typechecker (M3-onwards) attaches Index resolution; the evaluator (M3-onwards) wires actual behavior. Wiring the parse path first lets later milestones land incrementally.

**Verification:**

- ESL: round-trip parse + serialize tests for `text_index` and `vector_index` blocks; validation that `target_property` IRI is required.
- EigenQL: parser tests for each new keyword and the `TOP K BY` clause; explicit "not yet implemented" evaluator errors for the new functions.
- Kernel boot: opens a fresh DB with the new column families; refuses to open a pre-D43 DB per the schema-version check.
- Core ontology: `cargo test` reloads with the new Class definitions present.

**Risk areas:**

- Column-family declaration must match exactly between writer and reader; mistyped CF name causes a silent miss. Lock the CF list in a single constant.
- The ESL parser is the largest file in the kernel; adding declarations is straightforward but follow the existing `class` declaration's parse path closely.

---

## M2 — Storage substrate

**Goal:** Build the `TextIndex` and `VectorIndex` traits, their in-memory and RocksDB implementations, and the per-`(layer, Index)` key schemas. This milestone makes the kernel capable of *writing* index entries; reading and query evaluation come in M3 / M5.

**Prerequisites:** M1.

**Duration:** ~2 weeks.

**Deliverables:**

| Item | Location | Notes |
|---|---|---|
| `TextIndex` trait + types | New: kernel/src/layer/text_index.rs | Following the [Phase 14h `TripleIndex`](kernel/src/layer/index.rs#L84) pattern. Methods: `extend_layer`, `drop_layer`, `scan_term`, `stats`. Types: `TextDoc<'a>`, `TextQueryResult`. |
| `VectorIndex` trait + types | New: kernel/src/layer/vector_index.rs | Same shape. Methods: `extend_layer`, `drop_layer`, `prefix_scan_index`, `stats`. Types: `VectorDoc<'a>`, `VectorHit`. |
| `MemoryTextIndex` + `MemoryVectorIndex` | Same files | In-memory implementations following [`MemoryTripleIndex`](kernel/src/layer/index.rs#L289). Required for the pre-populate-on-build path that mirrors the triple-index dual-path design. |
| `RocksTextIndex` + `RocksVectorIndex` | New: storage/rocksdb/src/text_index.rs, storage/rocksdb/src/vector_index.rs | Persistent implementations with `extend_into_batch` / `drop_into_batch` so they participate in the layer's atomic `WriteBatch`. Roaring bitmaps for postings (text); CBOR-blob segments with the §2.4 layout (vector — flat only at this milestone, HNSW arrives in M6). |
| `PersistentBackend` trait extensions | [kernel/src/storage/mod.rs](kernel/src/storage/mod.rs#L231) | `fn text_index_arc(&self) -> Arc<dyn TextIndex>` and `fn vector_index_arc(&self) -> Arc<dyn VectorIndex>` mirroring `triple_index_arc`. |
| `LayerBuilder::build` hooks | [kernel/src/layer/mod.rs](kernel/src/layer/mod.rs#L813) | Discover active TextIndex / VectorIndex Resources for the committing layer's contributions; populate both in-memory and persistent backends in the `WriteBatch`. |
| `delete_layer` hooks | Same | Reverse-index-driven prefix scans for `text_terms_layer:<L>:` and `vec_layer:<L>:`; batched deletes in the same `WriteBatch` as the layer record. |
| Dependency additions | [Cargo.toml](Cargo.toml) | `roaring` crate for posting bitmaps; `bytemuck` for SIMD-friendly vector casts; `unicode-segmentation` for tokenization (used in M3). |

**Implementation guidance:**

- Mirror the [Phase 14h dual-path pattern](docs/design/phase-14h-indexed-reads.md#L137) exactly: build pre-populates the in-memory backend; commit writes both into the `WriteBatch`; post-restart reads use the persistent backend; trace memoisation is per `(index_iri, ...)`.
- Active Index resolution happens via the existing triple index — `find TextIndex where target_property = P`, shadow-filter via `is_shadowed`, take the most-recent surviving match. Add a small helper `resolve_active_index(layer, property, kind)` in [kernel/src/layer/index.rs](kernel/src/layer/index.rs) that all five call-sites (build, delete, query, sweep, consolidation) share.
- The vector segment encoder produces the CBOR layout from D43 §2.4 with `_pad` for 32-byte alignment. `hnsw_graph` is omitted at this milestone; M6 adds it.
- The four text key prefixes (`text_term`, `text_docs`, `text_stats`, `text_terms_layer`) all live in `cf_text`. The two vector key prefixes (`vec_seg`, `vec_layer`) live in `cf_vec`. Verify with a CF-routing test.
- Validate the v1 multiplicity constraint at commit time: if a layer's commit would produce a chain state where two TextIndexes (or two VectorIndexes) target the same Property at any visible head, Load fails.

**Verification:**

- Unit tests for each trait method on both in-memory and RocksDB backends.
- Equivalence tests: identical operations produce identical reads across in-memory and RocksDB.
- Atomic commit test: a layer that commits both triple-index and text-index and vector-index entries either commits all-or-nothing.
- `delete_layer` test: enumeration via reverse indexes catches every key written by the layer.
- Multiplicity constraint test: two TextIndex Resources targeting the same property fail at Load.
- Branch-divergence test: two branches that declare different TextIndex Resources for the same Property have separately-addressable keys with no collision.

**Risk areas:**

- Active-Index resolution must be consistent across build, query, and delete. A single helper used by all paths prevents drift.
- The atomicity guarantee for the `WriteBatch` is load-bearing — make sure neither index can write outside it.

---

## M3 — Text retrieval

**Goal:** End-to-end text retrieval: tokenization, BM25 with chain-aware IDF, `TEXT_MATCH` and `TEXT_SCORE` in EigenQL, caches.

**Prerequisites:** M1, M2.

**Duration:** ~2–3 weeks.

**Deliverables:**

| Item | Location | Notes |
|---|---|---|
| Tokenizer pipeline | New: kernel/src/query/text/analyzer.rs | Analyzer abstraction; `"en-stem-v1"` (Unicode segmentation + lowercase + Porter stem) and `"en-no-stem"` (Unicode segmentation + lowercase). Pluggable per the §3.1 `analyzer` field. |
| BM25 scorer with chain-aware IDF | New: kernel/src/query/text/bm25.rs | Per-§2.3 query path: global N + global DF computed across visible layers; per-document length normalisation. ~150 LOC. |
| Posting-list intersector | Same | AND-intersection of Roaring bitmaps per layer; rank merge across layers. |
| `TextIndex::scan_term` implementation | M2 traits filled in | Returns `(layer, df, postings_bytes)` stream; planner uses df without decoding the bitmap. |
| `TermCache` + `DocsCache` | New: kernel/src/query/text/cache.rs | Bounded LRU caches keyed by `(index_iri, term, layer)` and `(index_iri, layer)` respectively. Sized analogously to the D23 ARC cache. |
| Typechecker schema-view extension (text) | [kernel/src/query/type_check.rs](kernel/src/query/type_check.rs) | `active_text_index(P, H)` lookup; reject `TEXT_MATCH` / `TEXT_SCORE` on properties without an active TextIndex; reject on non-string-typed properties. |
| Query evaluator wiring | [kernel/src/query/evaluate/mod.rs](kernel/src/query/evaluate/mod.rs#L60) | `TEXT_MATCH` evaluates to Boolean per the §2.3 query path; `TEXT_SCORE` evaluates to Float using the same probe (planner deduplicates). Shadow check via existing `is_shadowed`. |

**Implementation guidance:**

- Analyzer applies both index-side (in M2 indexing pipeline) and query-side (here). Ensure the analyzer is consistent by reading it from the active TextIndex Resource at both points.
- The chain-aware IDF (§2.3 step 6) requires a prefix scan over `text_stats:<I>:` plus per-term `df` reads. Cache the global `N` and `avg_doc_length` per-query.
- Posting-list intersection within a layer: deserialize the Roaring bitmaps for each term, intersect, iterate the survivors.
- Shadow check happens after per-layer scoring, before global top-k merge. Match the existing triple-index pattern: hits are tuples `(subject, score, defining_layer)`; the shadow walk uses the per-layer `bloom:<layer_id>` record.
- Cache sizing: budget for `TermCache` and `DocsCache` together at ~256 MiB default, configurable. Per-entry is small (Roaring bitmap typically <1 KiB; `text_docs` blob typically tens of KiB).

**Verification:**

- Unit tests for tokenizer (English-only stemming; edge cases for Unicode segmentation).
- Unit tests for BM25 scorer against published reference outputs.
- Integration test: index a small corpus, run TEXT_MATCH and TEXT_SCORE, verify scoring matches reference BM25.
- Chain-aware IDF test: re-define a property across multiple layers, verify IDF changes correctly as the chain grows.
- Shadow check test: redefine a subject in a descendant layer, verify only the latest non-shadowed hit's score is returned (per §7.1 "last-writer-wins").
- Branch-divergence test: two branches with different TextIndex Resources return non-overlapping result sets.

**Risk areas:**

- Analyzer drift between index-side and query-side produces silent recall problems. Lock the analyzer ID into the per-segment `text_stats` metadata and verify at query time.
- BM25 implementation details (the +1 adjustment for IDF, the b parameter, etc.) — test against a known reference.

---

## M4 — Embedder Component + EMBED

**Goal:** Wire the Embedder Component infrastructure, the content-addressed embedding cache, inline `EMBED` dispatch through the existing D6 IO envelope, and the planner-side pre-pass batching.

**Prerequisites:** M1, M2.

**Duration:** ~1–2 weeks.

**Deliverables:**

| Item | Location | Notes |
|---|---|---|
| Embedder Component trait extension | [kernel/src/program/component.rs](kernel/src/program/component.rs#L54) | Extend `BuiltinComponent` (or add a marker trait `IsEmbedder`) so the dimensionality is statically declared on the Component IRI. |
| Embedding cache | New: kernel/src/program/embedding_cache.rs | Content-addressed by `(blake3(content), model_iri)`; stored in `cf_embed_cache`; LRU eviction with 1 GiB default budget. |
| Inline `EMBED` dispatch | [kernel/src/query/evaluate/fiber.rs](kernel/src/query/evaluate/fiber.rs#L303) | `EMBED` is treated as an inline IO Component call; existing D6 envelope handles the orchestrator round-trip; cache hits short-circuit. |
| `EMBED` pre-pass batching (planner) | New planner stage | Walk the typed AST, collect distinct `(text, model_iri)` tuples, dispatch in one parallel batch before structural query execution. |
| Reference Embedder Component (for tests) | New: kernel/src/program/builtin/dummy_embed.rs | Deterministic stub that produces a 256-dim vector from a hash of the input; used by integration tests without requiring a real embedder. |
| Trace recording | [kernel/src/program/trace.rs](kernel/src/program/trace.rs#L31) | Embedder Component invocations record as `Trace::Component(ComponentTrace)` with `cached: bool`; cache entries carry the trace IRI. |

**Implementation guidance:**

- The Embedder Component's static dimensionality is required so the typechecker can verify model_iri / dim consistency at parse time (§4.4).
- Cache key is `(blake3(content), model_iri)` — model_iri is part of the identity so model upgrades produce fresh cache entries.
- `EMBED` failures bubble up per the existing D6 IO-Component-failure path; no new error handling needed.
- The pre-pass batching is an optimisation, not a correctness requirement. Implement inline-dispatch first (simple), add the pre-pass as a planner pass second.
- The reference dummy Embedder makes test isolation possible — no test should depend on a real embedding service.

**Verification:**

- Unit tests for the embedding cache (content-addressed lookup; LRU eviction).
- Integration test using the dummy Embedder: register, dispatch via `EMBED("text")`, verify the vector and the trace.
- Cache-hit test: identical `EMBED` calls within a query (and across queries) hit the cache without re-dispatching.
- Pre-pass batching test: a query with 5 distinct `EMBED` calls makes one orchestrator round-trip with 5 parallel embeds.
- Failure-mode test: dummy Embedder configured to fail; query fails at evaluation per §5.8.

**Risk areas:**

- Cache invalidation under model upgrade: cache entries are content-addressed by `(content, model_iri)` so upgrades don't pollute — but verify with an explicit test.
- Hosted-model embedders may have rate limits; the orchestrator-global in-flight cap (default 64) needs to be respected.

---

## M5 — Vector retrieval (flat, v1)

**Goal:** End-to-end vector retrieval with brute-force k-NN (no HNSW yet): segment CBOR encoding, SegmentCache, SIMD distance kernels, `VECTOR_NEAR` and `VECTOR_SIM`, post-Load sweep as a D21 task.

**Prerequisites:** M2, M4.

**Duration:** ~2–3 weeks.

**Deliverables:**

| Item | Location | Notes |
|---|---|---|
| Vector segment encoder | M2 `RocksVectorIndex` filled in | CBOR top-level map with `_pad` for 32-byte alignment; concatenated `vectors` bstr; subjects and doc_lengths in parallel arrays. |
| Zero-copy segment reader | New: kernel/src/query/vector/segment.rs | One `db.get` into `Arc<[u8]>` at SegmentCache admission; CBOR header parse → `VectorSegmentLayout` with byte ranges; `bytemuck::cast_slice::<u8, f32>(...)` for the SIMD-ready `&[f32]`. |
| SIMD distance kernels | New: kernel/src/query/vector/distance.rs | Cosine / L2 / dot with AVX-2 and NEON fallback; bounded top-k heap. |
| SegmentCache | Same | Bounded LRU keyed by `(index_iri, layer)`; 256 MiB default budget; shared between vector segments and (in M3) text segments. |
| Typechecker schema-view extension (vector) | [kernel/src/query/type_check.rs](kernel/src/query/type_check.rs) | `active_vector_index(P, H)` lookup; `Vector(model: M, dim: D)` typing; `EMBED` model inference from context (§4.4); mismatched-model rejection. |
| Query evaluator wiring | [kernel/src/query/evaluate/mod.rs](kernel/src/query/evaluate/mod.rs) | `VECTOR_NEAR` and `VECTOR_SIM` per the §2.4 query path. Chain-walk discovery of segments; per-segment brute-force k-NN; shadow check; top-k merge. |
| Post-Load sweep task | New: kernel/src/task/sweep.rs | D21 task that materialises `vec_seg:<I>:<L>` entries after Layer commit. Eager-by-default; per-orchestrator in-flight cap (64); exponential backoff retry; cancel on `delete_layer(L)`. |
| Sweep status surface | TaskStatus integration | The sweep's `TaskRecord` is observable through `GetTaskStatus`; a query like "what's the vector-index coverage for layer L" returns the sweep's progress. |

**Implementation guidance:**

- SIMD: use `std::simd` (nightly) or a stable crate like `wide`. Fallback to scalar for unsupported architectures. Verify with a bytemuck-cast roundtrip test.
- Brute-force k-NN: bounded heap of size `K`, iterate the cast `&[f32]` in `dim`-sized chunks. ~10 ms per 100K vectors at 256 dim on a modern x86-64 core.
- Sweep task triggers from the post-Layer-commit hook. It enumerates active VectorIndex Resources targeting any property in the committing layer's Resources, then issues per-`(L, I)` materialisation units.
- One materialisation unit writes one `vec_seg:<I>:<L>` blob plus one `vec_layer:<L>:<I>` reverse entry. Single atomic `WriteBatch` per unit.
- `delete_layer(L)` cancels in-flight sweeps targeting L via the D21 task-cancel surface. Partial state never appears because materialisation is atomic.
- The §5.4 inline `EMBED` dispatch path (M4) is reused at index time during the sweep; the sweep just batches a lot of embeds.

**Verification:**

- Unit tests for the segment encoder/decoder (round-trip; alignment; CBOR validity).
- SIMD distance correctness: cosine / L2 / dot against scalar reference; alignment edge cases.
- Brute-force k-NN: known nearest-neighbor sets.
- Integration test using the dummy Embedder from M4: index, sweep completes, vector queries return correct results.
- Sweep cancellation test: cancel an in-flight sweep, verify no partial state.
- Partial-materialisation test: query during an in-flight sweep returns partial-but-correct results.
- Branch-divergence test: two branches with different VectorIndex Resources have separately-keyed segments and produce different results.

**Risk areas:**

- Alignment in the CBOR layout: verify on both x86-64 and ARM that the cast slice is correctly aligned for SIMD.
- Sweep retry loop must not spin under sustained embedder failure; exponential backoff with a maximum-attempts ceiling.
- Memory accounting: SegmentCache holds full segment blobs; a 100K-vector × 256-dim segment is ~100 MiB. Watch budget consumption under realistic loads.

---

## M6 — HNSW addition

**Goal:** Add HNSW to v1 per the strategy-switched design (§2.4). Each VectorIndex's `strategy: flat | hnsw | auto` configuration drives per-segment selection; `auto` builds HNSW when segment `count` exceeds a threshold.

**Prerequisites:** M5.

**Duration:** ~1–2 weeks.

**Deliverables:**

| Item | Location | Notes |
|---|---|---|
| HNSW library choice | Crate selection | Candidates: `instant-distance`, `usearch`, `hnsw_rs`, or roll-our-own. Decide based on dependency weight, performance, and Rust ergonomics. |
| HNSW graph encoding | M5 segment encoder extended | Additive `hnsw_graph` field per the §2.4 layout; stable on-wire format independent of the chosen library so swaps don't require index migration. |
| Strategy dispatch | M5 query path | Reader checks for `hnsw_graph` presence; HNSW traversal with `ef = max(K*4, 64)` default. |
| Build-time strategy selection | Sweep task / consolidation | `strategy: flat` always flat; `hnsw` always HNSW; `auto` builds HNSW when `count > 50_000` (configurable). |
| Recall measurement infrastructure | New: kernel/src/query/vector/recall.rs | Per-segment recall@K estimate emitted with HNSW results; final result set carries the minimum recall touched. |
| Query-time `ef` parameter | Parser + evaluator | Already specified in §3.4; wire the optional `ef` argument through to the HNSW dispatch. |

**Implementation guidance:**

- The library choice can be deferred until the segment-CBOR layout is locked. Once locked, the chosen library is swappable: a different library produces different graph topology but the same on-wire bytes.
- Build cost is 10–100× the flat-storage cost. Run it inside the sweep task or at consolidation time; never on the synchronous query path.
- HNSW memory: at M=16, ~128 bytes per vector for the graph. A 10M-vector segment with HNSW occupies ~10 GiB flat + ~1.3 GiB HNSW — comfortable on commodity hardware.
- For recall measurement, sample a small fraction of queries with a parallel brute-force baseline; emit the per-segment recall estimate without re-running brute-force on every query.

**Verification:**

- Recall benchmark: against a known dataset (e.g., SIFT or GIST), measure recall@10 at varying `ef` values; verify ≥95% at `ef=k*2`, ≥99% at `ef=k*8`.
- Strategy-switch test: a property with `strategy: auto` and threshold 1000 produces flat segments for layers with <1000 vectors and HNSW segments for layers with ≥1000.
- Build-time benchmark: HNSW construction time for 100K-, 1M-, 10M-vector segments.
- End-to-end: index UMLS or SwissProt subset, verify query latency stays in tens of milliseconds at the chain-walk fanout typical for those datasets.

**Risk areas:**

- Library lock-in: stable on-wire format makes swaps possible but not painless. Choose carefully.
- Memory pressure at scale: a 100M-vector workload may push us into v2 territory (out-of-core HNSW, quantization). Document the v1 envelope clearly.

---

## M7 — Similarity operator + hybrid retrieval

**Goal:** D43's single user-visible retrieval surface — the `~` operator (D43 §3.3) with its optional hint block (D43 §3.4) — plus the platform-internal machinery it expands into: per-property active-index discovery, parallel probe scheduling for hybrid-indexed properties, internal RRF fusion across multiple sources, top-K pushdown.

**Prerequisites:** M3, M5.

**Duration:** ~1–2 weeks.

**Surface-reset note.** An earlier draft of M7 (and a companion D45 BIND-clause proposal) modeled retrieval after the D35 §7.4 worked example: separate `TEXT_MATCH` / `TEXT_SCORE` / `VECTOR_NEAR` / `VECTOR_SIM` / `EMBED` / `RRF` primitives, a `TOP K BY <score>` clause, and a `BIND(expr AS ?var)` clause for naming per-row scores. That direction was abandoned after the D43 surface review (June 2026) — the seven primitives carried SQL-shaped imagination into a language that didn't need it, exposed implementation details (embedding vectors, raw scores, fusion algorithm) the user shouldn't care about, and forced workarounds (BIND) for surface gaps that disappear under a smaller language. The reset collapses all seven primitives + BIND into the single `~` operator. See D43 §3 for the user-visible surface and the D45 withdrawal note for the BIND rollback.

**Deliverables:**

| Item | Location | Notes |
|---|---|---|
| `~` operator | Lexer, parser, AST | New `Tilde` token; binary operator with high precedence (below comparison, above logical). LHS must be a property-bound variable; RHS is a string expression. |
| Hint block | Parser | Optional trailing `{ via:, model:, k:, limit: }` braces; validated keys; parsed alongside the operator. |
| Typecheck | kernel/src/query/type_check.rs | Per D43 §4.3: LHS property-bound, RHS string, property has at least one active similarity index. Per §4.4: hint values are well-typed, consistent with active-index set. |
| Active-similarity-index discovery | kernel/src/layer/index_discovery.rs (extension) | `active_similarity_indexes(P, H)` — union of `active_text_index(P, H)` + `active_vector_index(P, H)`. Memoised per query. |
| Query-string embedding pre-pass | kernel/src/query/evaluate or new pre-pass module | Per D43 §6.3: collect every `~` operator whose property has an active VectorIndex; batch-dispatch embeddings to the orchestrator via the existing D6 envelope; substitute results before the structural query begins. |
| Per-source probe scheduling | Planner (kernel/src/query/evaluate) | Per D43 §6.5: parallel text + vector probes per operator; each bounded by the operator's `limit:` hint or planner-derived `K * over_fetch_factor`. |
| Internal RRF fusion | kernel/src/query/rank.rs + evaluator | Per D43 §6.4: rank materialisation across all contributing sources; fused score = `sum_i 1/(k + rank_i)` with default k=60 (overridable via the operator's `k:` hint); missing-source rank = ∞ (contributes 0). RRF is *not* user-visible. |
| Top-K integration | Evaluator | `TOP K` clause (the existing D2 surface) consumes the implicit similarity-derived ranking when `~` operators appear in WHERE. No `BY <expr>` needed in the pure-similarity case. |
| Over-fetch policy | Planner | Default 4× over-fetch per source; structural-selectivity-aware adjustment per D43 §6.6; operator's `limit:` hint overrides locally. |

**Implementation guidance:**

- The `~` operator's evaluator dispatches based on the active-index set: 1 active index → single probe → rank-by-score; 2 active indexes (hybrid) → parallel probes → internal RRF; multiple `~` operators in the same query → fuse all sources.
- Fusion is *always* internal — RRF, scores, per-source ranks never enter the user-facing AST or RETURN projections. The pre-existing `Expression::Rrf` AST variant from the abandoned draft is removed.
- Embedding-model selection comes from the active VectorIndex's declared `model`, with the operator's `model:` hint overriding. The user never names the embedder in the query body.
- The user-visible `TOP K` truncates the WHERE-implied ranking. No `BY <expr>` clause for the similarity case; the existing `BY` keyword remains for non-similarity ORDER BY which is unchanged.
- Diagnostic surface (`EXPLAIN`-equivalent for inspecting per-source scores and ranks) is deferred per D43 §3.7; without it, debuggability for unexpected rankings is reduced — flag this in user docs (M9.5).

**Verification:**

- Parser tests: `~` operator with and without hints; hint validation (unknown keys, wrong-typed values).
- Typecheck tests: property without active similarity index; LHS not property-bound; RHS not string; per-hint consistency errors (`via: text` on a property with no TextIndex, `model:` with `via: text`, etc.).
- Evaluator unit tests: single-index path (text-only, vector-only); hybrid path (RRF over text + vector on same property); multi-operator path (RRF over multiple `~` operators).
- Integration test: rewritten D35 §7.4 worked example using `~` (replaces the earlier M9.1 integration test which used the abandoned surface).
- Cache test: identical RHS strings across multiple `~` operators on different vector-indexed properties share the embedding cache.
- Over-fetch test: a query with selective structural filter; verify the planner over-fetches enough to satisfy K after filtering.

**Risk areas:**

- The `~` operator is high-precedence; conflicts with existing EigenQL operator-precedence rules need verification. Reserved-keyword test should cover this.
- Multi-operator AND vs OR fusion semantics: AND requires the same row to satisfy all operators (intersection-of-candidate-sets); OR requires any operator to admit the row (union-of-candidate-sets). The fusion math is the same either way (RRF across contributing sources) but the candidate-set construction differs — needs care.
- Hint validation must be strict — typos like `{ k: "60" }` (string, not int) should fail at parse with a clear error, not silently default.

---

## M8 — Consolidation + atomic reindex

**Goal:** Plug D43 into D25's `consolidate_chain` (re-extraction for text, concatenation for vector, with per-Index strategy applied to consolidated segments). Implement the §5.7 atomic reindex for VectorIndex Resource replacement (model upgrades).

**Prerequisites:** M2 (storage substrate), M3 (text), M5 (vector flat), M6 (HNSW).

**Duration:** ~1–2 weeks.

**Deliverables:**

| Item | Location | Notes |
|---|---|---|
| Consolidation extension point | [kernel/src/layer/consolidate.rs](kernel/src/layer/consolidate.rs#L291) | New callback / method on `TextIndex` and `VectorIndex` traits: `consolidate_range(from: LayerId, to: LayerId, consolidated: LayerId, resolved_set: ResolvedView)`. |
| Text consolidation | `RocksTextIndex::consolidate_range` | Re-extract: enumerate surviving subjects, re-tokenize, assign fresh local doc-ids in `C`, build new `text_term:<I>:<T>:<C>`, `text_docs:<I>:<C>`, `text_stats:<I>:<C>`, `text_terms_layer:<C>:<I>` entries. |
| Vector consolidation | `RocksVectorIndex::consolidate_range` | Concatenate surviving vectors from the collapsed range, re-label with `defining_layer = C`, encode per §2.4; if strategy is `hnsw` (or `auto` with count above threshold), rebuild HNSW. |
| Atomic reindex driver | New: kernel/src/task/reindex.rs | Triggered when a new VectorIndex Resource with the same `target_property` shadows the existing one. Re-embeds all visible subjects under the new Index's model; replaces all `vec_seg:<I_old>` entries with `vec_seg:<I_new>` entries. |
| Reindex task status | TaskStatus integration | Reindex runs as a D21 task; observable through `GetTaskStatus`; cancellable. |
| §2.8 consolidation atomicity test | Integration test | Consolidation of a range with active TextIndex and VectorIndex Resources; verify search-equivalence under head substitution. |

**Implementation guidance:**

- The consolidation extension point should fit `ConsolidateOpts` cleanly; a builder pattern for per-index consolidation callbacks works.
- Text consolidation requires re-tokenization because Roaring bitmaps use layer-local doc-ids and remapping across layers is more complex than re-extracting.
- Vector consolidation is just concatenation + relabeling (no re-embedding required, per §2.8) — until the strategy demands an HNSW rebuild.
- Atomic reindex is structurally similar to consolidation but operates per-(layer, VectorIndex) rather than collapsing layer ranges. Implementable on top of D25's atomic-multi-layer-write machinery.

**Verification:**

- Consolidation atomicity test: search-equivalence under head substitution per D25's load-bearing invariant.
- Cross-model reindex test: declare a new VectorIndex Resource with a different model; verify the reindex sweep completes; verify old segments are no longer queryable at the post-reindex head but remain queryable from any branch that still references the old Index Resource.
- HNSW rebuild test: consolidate a flat-strategy segment range that crosses the auto-threshold; verify the consolidated segment has an HNSW graph.

**Risk areas:**

- Cross-Index consolidation: ensure the consolidation enumerates all active Index Resources in the resolved set, not just the ones present at one layer.
- Reindex is a long-running task that touches many layers; ensure cancel-on-`delete_layer(L)` works for all the layers it touches.

---

## M9 — End-to-end validation

**Goal:** Acceptance testing against the workloads D43 was specified to serve.

**Prerequisites:** M1–M8.

**Duration:** ~1–2 weeks.

**Deliverables:**

| Item | Notes |
|---|---|
| D35 §7.4 example queries | Verify the SE knowledge-graph queries from D35 run end-to-end and produce expected results against a representative test ontology. |
| Life-science integration test | Index a representative subset of a life-science ontology (e.g., GO subset, ChEBI subset). Verify performance envelope. |
| HNSW benchmark | Recall and latency measurements at the 1M, 10M-vector segment sizes. |
| Performance benchmarks | Index-build time, per-query latency, memory footprint at v1-envelope scales. Document the operating envelope clearly so users know when to consider v2 work. |
| User documentation | Surface-level documentation for the new EigenQL keywords, the ESL declarations, and the embedder Component registration. Lives in guides/. |
| Implementation notes appendix | Capture any non-obvious decisions made during implementation that future maintainers should know. Lives as an addendum or in [phase-XX-d43-implementation.md](docs/design/) following the phase-14h pattern. |

**Verification:** All previous milestones' tests still pass; new acceptance tests pass; performance benchmarks meet the published v1 envelope.

---

## Critical path

The shortest dependency chain from start to acceptance:

```
M1 → M2 → M4 → M5 → M6 → M7 → M8 → M9
```

M3 (text retrieval) runs in parallel with M4–M6 (embedder + vector + HNSW). The M7 RRF milestone is the synchronisation point where both streams meet.

## Risk register

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Column-family migration breaks something subtle in the kernel | Low | High | Land M1 first; verify exhaustively before M2 lands keys into the new CFs. |
| Active-Index resolution inconsistency between build, query, sweep | Medium | High | Single shared helper used everywhere; integration tests cover all five call-sites. |
| Cross-source rank fusion (RRF) implementation surprises | Medium | Medium | Test against published reference implementations; RRF formula is well-known. |
| HNSW library lock-in | Low | Medium | Stable on-wire format; library is swappable but with rebuild cost. |
| Sweep retry storms under sustained embedder failure | Medium | Medium | Exponential backoff with maximum-attempts ceiling; visible task status. |
| BM25 chain-aware IDF performance | Low | Medium | The IDF computation is one prefix scan per query term; cache aggressively. |
| Memory pressure from caches | Medium | Low | Bounded LRU budgets, configurable; under-provisioning is correct (slower) not wrong. |
| User-error queries that mix scores across Indexes (per §7.5) | High | Low | Typechecker doesn't police; user documentation steers toward RRF. |

## Out of scope (deferred to future revisions)

D43 v1 explicitly does not implement these — they're either §8 v1 deferrals, §3.9 surface deferrals, or external concerns:

- Phrase queries (positional postings) — §2.3 deferral; additive to the key schema.
- Fuzzy queries (Levenshtein automata).
- Wildcard / regex queries.
- Multi-language analyzers — beyond `"en-stem-v1"` and `"en-no-stem"`.
- Multi-field text queries with per-field boosts — additive; schema supports it.
- Vector quantisation (int8) — v2 work when segments exceed ~10M vectors.
- IVF preceding HNSW — same.
- Out-of-core HNSW with mmap — same.
- `vector_embedding_policy` values `lazy_on_query` and `manual` — grammar slot reserved; v1 ships `eager_on_load` only.
- Automatic consolidation policy — [D44 — Automatic Data Lifecycle Management](d44-automatic-data-lifecycle-management.md) territory.
- Automatic GC policy — same.
- Cross-Index score arithmetic enforcement — typechecker accepts; user documentation steers.

## References

- [d43-text-and-vector-retrieval.md](d43-text-and-vector-retrieval.md) — the design D43 implements.
- [phase-14h-indexed-reads.md](phase-14h-indexed-reads.md) — the architectural template the text and vector indexes mirror.
- [D23 §5.2](d23-out-of-core-layer-architecture.md) — the per-layer shadowing bloom and chain-walk primitives.
- [D24](d24-schema-versioning.md) — the schema-version bump mechanism for the new column families.
- [D6](d6-execution-architecture.md) and [D6b](d6b-reasoning-trace-schema.md) — the IO Component dispatch envelope and trace recording.
- [D21](d21-task-traces-and-checkpointing.md) — the task surface for the sweep and reindex.
- [D25](d25-chain-consolidation.md) — the consolidation primitive D43 §2.8 extends.
- [D44](d44-automatic-data-lifecycle-management.md) — where automatic consolidation and GC policies live (stub).

---

*Living document. Update as milestones complete; capture non-obvious implementation decisions in an addendum as they're made.*
