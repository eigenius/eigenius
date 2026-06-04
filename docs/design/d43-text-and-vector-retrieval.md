# D43 — Text and Vector Retrieval in EigenQL

**Status:** Draft. §2 (storage architecture and layer integration) is the load-bearing first cut and is intended for review. §§3–7 are scope stubs; §8 enumerates the open questions.

**Phase:** TBD. Sequencing is gated by [D35 §7.4](d35-software-engineering-knowledge-graph.md) ("Text and vector retrieval as EigenQL primitives"), which declares this work as a hard dependency for the SE knowledge-graph rollout.

**Builds on:** [D2 — EigenQL v1 Specification](d2-eigenql-specification.md) (the surface this revision extends), [D23 — Out-of-Core Layer Architecture](d23-out-of-core-layer-architecture.md) (the per-layer-index pattern, the topology / content split, the atomic-commit envelope, the shadow-check primitive), [D25 — Chain Consolidation](d25-chain-consolidation.md) (the only mechanism that lets per-layer segments stay small enough to remain efficient long-term).

**Companion:** [D42 — Out-of-Core Query Execution](d42-out-of-core-query-execution.md) (independent EigenQL evolution; both touch the operator pipeline but with disjoint structural commitments — D42 makes existing operators spillable, D43 introduces new ranked-retrieval operators).

**Picks up from:** [D35 §7.4](d35-software-engineering-knowledge-graph.md) — that section sketches the surface (`TEXT_MATCH`, `VECTOR_NEAR`, `EMBED`, `TOP K BY`) and flags the layer-aware HNSW question as the hard part. D43 is the proper home for the spec; once D43 lands, D35 §7.4 collapses to a dependency declaration.

---

## 1. Motivation and scope

EigenQL today is a typed Datalog over the chain — pattern-matching against typed Resources with structural predicates. It does not retrieve. An agent (or human) starting work on an unfamiliar area of the graph cannot ask "find Resources whose `description` matches *WAL truncation under concurrent commit*" or "find Resources semantically near *rolling back a partially-written commit*" through the query language. The structural patterns of [D35 §7.1](d35-software-engineering-knowledge-graph.md) — Localisation, Coverage, Impact, History, Justification — all assume the caller already knows the IRI of the thing they want to start from. Discovery is unsolved.

The proposal: text and vector retrieval enter EigenQL as **built-in primitives**, not as institution queries dispatched through `FIBER`. The architectural argument (D35 §7.4) is summarised here; the rest of the document is the spec.

**Why built-in, not institution.** An institution's signature is a set of sentences with `Holds | Fails | Undecidable` verdicts. Retrieval is a *real-valued ranking over Resources*, not a verdict on a sentence. It does not typecheck as an institution query. The operational arguments — planner-level pushdown, top-k as an index property the planner must know about, hybrid retrieval requiring per-source rank fusion — reinforce the conceptual one. Treating retrieval as built-in lets the planner push selective predicates into index probes and reorder joins; treating it as an institution would force every hybrid structural-plus-retrieval query across `FIBER` boundaries and defeat pushdown entirely.

**One operator, hidden mechanism.** D43's user surface is deliberately small: a single similarity operator `~` between a property-bound variable and a text query (§3.3), with an optional hint block (§3.4) for the cases where defaults need overriding. Implementation details — which index (text, vector, both fused) produced each result, what embedding model was used, what the raw scores were — are all platform-internal. Schema owners declare TextIndex / VectorIndex Resources on properties to control retrieval strategy at schema time; query authors write intent and trust the platform. §3.2 develops the rationale.

**Scope of this document.** D43 specifies (a) the storage architecture for text and vector indexes, (b) the single-operator EigenQL surface, (c) the type-system additions needed to validate the operator and its hints, (d) the embedding lifecycle, (e) the planner integration (probe pushdown, internal RRF fusion, parallel hybrid scheduling), and (f) the layer-awareness rules for scores and shadowing. It does not specify the choice of embedding model, the production HNSW algorithm, or the cross-language tokenisation defaults — those are downstream tunables.

**Out of scope.** Full-text search over Resource *bodies* in the sense of "search the entire chain as a document corpus" is not what this provides. The unit of indexing is the **property value** on a specific Resource — indexability is declared by a separate `core:TextIndex` or `core:VectorIndex` Resource targeting a specific Property (§3.1), not as a chain-wide setting. Properties without an active Index Resource are not retrievable; queries against them fall back to the existing structural surface.

---

## 2. Storage architecture and layer integration

This is the load-bearing technical question. The constraints from D23 are non-negotiable; the design space is how to satisfy them with text and vector backends whose IO patterns and data structures differ sharply from the existing IRI-keyed triple index.

### 2.1 Design constraints

Four invariants from D23 govern how any new index integrates:

1. **Per-layer, not per-head.** "Whether layer L defines X is a property of L alone" (D23 §5.2). Per-head bookkeeping doesn't survive branches, makes multi-parent merge expensive, and replicates the resolved set on every commit. The triple index landed in Phase 14h chose per-layer for this reason; text and vector indexes follow suit.
2. **Atomic commit with the layer write.** D23 §6.3 requires that when `store_layer`'s `WriteBatch` commits, the layer's indexes are visible; when it fails, no partial state is visible. The triple index participates in the same `WriteBatch` as the layer commit. Text and vector index entries — including the segment blobs and posting-list keys themselves — participate in the same batch. D43 keeps *everything* in RocksDB rather than splitting bulk data onto the filesystem with a pointer fence; the consistency, backup, and atomic-commit stories collapse to "one `WriteBatch`, period." §2.3 develops the Phase 14h-aligned key schema (custom layer-aware inverted index, no Tantivy dependency); §2.4 mirrors the same key shape for vectors.
3. **Shadow semantics.** If layer L1 defines `R.description = "old"` and a descendant layer L2 defines `R.description = "new"`, a query at a head that includes both sees only the L2 definition. This is the bloom-walk shadow check from D23 §5.2 and Phase 14h: a hit at `defining_layer` is filtered out if some intermediate ancestor of the head (between the defining layer and the head) also defines the subject. The mechanism extends to text and vector hits unchanged: hits are tuples `(subject, score, defining_layer)`, and the same topology walk decides whether to drop.
4. **GC-friendly.** `delete_layer(L)` must drop all index entries belonging to L atomically with the layer-record delete (D23 §6.5). The reverse-index pattern from Phase 14h (the `idx_layer:` prefix that makes deletion a prefix scan) generalises to text and vector indexes that store their per-layer artifacts under a `layer_id`-prefixed key.

A fifth, soft constraint: the index design should remain compatible with D25 chain consolidation. When the chain consolidates the range `[from..to]` into a single layer, the text and vector segments belonging to the collapsed range need to be replaced by a single segment carrying the consolidated layer's id — without invalidating the head's queries during the swap.

### 2.2 Pluggable per-index pattern

The existing storage abstraction already gives us the shape:

```rust
// kernel/src/layer/index.rs (Phase 14h)
pub trait TripleIndex: Send + Sync { /* extend_layer, drop_layer, scan_predicate_object, stats */ }

// kernel/src/storage/mod.rs
impl PersistentBackend {
    fn triple_index_arc(&self) -> Arc<dyn TripleIndex>;
}
```

D43 adds two siblings, deliberately separate because their backends and IO shapes differ:

```rust
// kernel/src/layer/text_index.rs
pub trait TextIndex: Send + Sync {
    fn extend_layer(&self, batch: &mut dyn IndexBatch, layer: &LayerId, docs: &[TextDoc<'_>]) -> Result<(), StorageError>;
    fn drop_layer(&self, batch: &mut dyn IndexBatch, layer: &LayerId) -> Result<(), StorageError>;
    fn search(&self, property: &Iri, query: &TextQuery, opts: &SearchOpts) -> Box<dyn Iterator<Item = Result<TextHit, StorageError>> + '_>;
    fn stats(&self) -> IndexStats;
}

// kernel/src/layer/vector_index.rs
pub trait VectorIndex: Send + Sync {
    fn extend_layer(&self, batch: &mut dyn IndexBatch, layer: &LayerId, vecs: &[VectorDoc<'_>]) -> Result<(), StorageError>;
    fn drop_layer(&self, batch: &mut dyn IndexBatch, layer: &LayerId) -> Result<(), StorageError>;
    fn k_nearest(&self, property: &Iri, query: &[f32], k: usize) -> Box<dyn Iterator<Item = Result<VectorHit, StorageError>> + '_>;
    fn stats(&self) -> IndexStats;
}

pub struct TextDoc<'a>  { pub subject: &'a Iri, pub property: &'a Iri, pub text: &'a str }
pub struct VectorDoc<'a> { pub subject: &'a Iri, pub property: &'a Iri, pub vector: &'a [f32], pub model_iri: &'a Iri }

pub struct TextHit   { pub subject: Iri, pub defining_layer: LayerId, pub score: f32 }
pub struct VectorHit { pub subject: Iri, pub defining_layer: LayerId, pub distance: f32 }
```

`PersistentBackend` gains `text_index_arc` and `vector_index_arc`, mirroring `triple_index_arc`. `LayerStorage` gains `text_index: Arc<dyn TextIndex>` and `vector_index: Arc<dyn VectorIndex>` alongside the existing `triple_index`. The traits are intentionally narrow — they expose the per-layer lifecycle (extend / drop) and the per-property search primitive, nothing else. Query-time chain walking, shadow checking, and score merging happen in the kernel's query evaluator, above the trait, using the existing topology infrastructure.

### 2.3 Text index storage (custom inverted index, Phase 14h-aligned)

D43 ships a custom layer-aware inverted index rather than depending on Tantivy. The dependency-weight argument is in §8; the structural arguments are these:

**Phase 14h key alignment.** The existing triple index uses `idx_pos:<predicate>:<object>:<subject>:<layer>` — predicate / object lead, layer last — so reads are a single global prefix scan and chain-membership filtering happens above. Custom text and vector indexes adopt the same shape, with the Index Resource's IRI in the leading position (rather than the target Property's IRI — see §3.1 for the cross-chain rationale). Queries against term `T` under the active TextIndex `I` prefix-scan `text_term:<I>:<T>:` and get all layer-keyed hits in one stream; chain filtering, shadow checking, and score merging happen above the index in the same machinery the triple index already uses. Consistency with Phase 14h's pattern is itself a structural asset.

**Chain-aware BM25.** Tantivy's BM25 computes IDF per-segment, which Tantivy treats as a transient compromise — segments merge into larger ones, so the IDF skew has a bounded window. For our layer-aware design, segments correspond to immutable layers and don't merge across the chain, so per-segment IDF would be *permanent*, not transient. Rolling our own lets the scorer walk the chain at query time and compute global DF / N across visible layers; the layer semantics are baked into scoring rather than fought against it.

**Key schema (column family `cf_text`).**

```
text_term:<index_iri>:<term>:<layer>    →  varint(df) || roaring_bytes
text_docs:<index_iri>:<layer>           →  CBOR { subjects: [iri], doc_lengths: [u32] }
text_stats:<index_iri>:<layer>          →  CBOR { doc_count: uint, avg_doc_length: f32 }
text_terms_layer:<layer>:<index_iri>    →  CBOR [term, ...]      (reverse index for drop_layer)
```

- All four keys are keyed by the TextIndex Resource IRI, not by the target Property IRI. This is what makes divergent Index configurations across branches storage-safe (§3.1: per-Index segment keying).
- **`text_term`** carries the posting list for one `(index, term, layer)` triple. The value layout is `varint(df) || roaring_bytes` — the document-frequency count is prepended so IDF computation can read it without deserialising the bitmap. Full BM25 evaluation deserialises the Roaring bitmap to enumerate the document IDs.
- **`text_docs`** is the doc-id → subject IRI mapping for the layer-index pair, plus per-document length for BM25 length normalisation. The two arrays are parallel: `subjects[i]` is the IRI for doc-id `i`, `doc_lengths[i]` is its token count.
- **`text_stats`** caches per-layer aggregates (`doc_count`, `avg_doc_length`) so chain-aware BM25 doesn't reparse `text_docs` to compute them.
- **`text_terms_layer`** is the Phase 14h-style reverse index that turns `drop_layer(L)` into a single prefix scan plus batched deletes (§2.7). The value carries the per-layer-per-index vocabulary so the cleanup can enumerate the term keys to delete.

All four keys live in `cf_text`, separate from `cf_default` and `cf_vec`. Column-family separation isolates text-index compaction from layer / topology / triple-index churn; tuning is in §8.

**Indexing pipeline (per layer commit, per active TextIndex `I` targeting some property `P`).**

For each TextIndex Resource `I` that is active at the commit head and targets a property `P` receiving contributions in layer `L`:

1. Enumerate the subjects in `L` carrying property `P`. Assign per-`(L, I)` local doc-ids `0..n`.
2. Tokenize each subject's value using `I`'s configured analyzer. Record `doc_lengths[i]`.
3. For each unique term `T`: build a Roaring bitmap over `{i ∈ 0..n | doc i contains T}`. The bitmap's set-bit count is the per-layer DF; the bitmap and the DF are encoded as `varint(df) || roaring_bytes`.
4. Encode the four keys for this `(L, I)`:
   - One `text_term:<I>:<T>:<L>` per unique term `T` in `(L, I)`.
   - One `text_docs:<I>:<L>` carrying the subjects and doc-lengths arrays.
   - One `text_stats:<I>:<L>` carrying `doc_count = n` and `avg_doc_length`.
   - One `text_terms_layer:<L>:<I>` carrying the unique-term list.
5. All keys for the layer commit (across all active TextIndex Resources discovered for the committing layer's contributions) batch into the same `WriteBatch` as the layer record and the triple-index entries (§2.5).

**Query path.**

Given a head `H` and `TEXT_MATCH(?prop, "q")` against property `P`:

1. Resolve the active TextIndex Resource `I` for property `P` at head `H` (chain-walk discovery via the triple index: find `TextIndex` where `target_property = P`; take the most-recent non-shadowed match). Read `I`'s analyzer. If no active TextIndex exists for `P` at `H`, fail at parse (§4).
2. Parse and tokenize `"q"` using `I`'s analyzer → term sequence `[t1, ..., tm]` and boolean structure. v1: implicit AND over all terms; OR / NOT / phrase deferred (§8).
3. Collect `chain = collect_ancestors(H)` (existing helper).
4. For each `ti`: prefix-scan `text_term:<I>:<ti>:` → stream of `(layer, df, postings_bytes)`. RocksDB iteration yields only existing entries, so layers with no contribution to `ti` cost nothing.
5. Filter the stream to `layer ∈ chain` (chain-membership filter, identical to Phase 14h's pattern).
6. Compute chain-aware IDF for each `ti`:
   - Global `N` = sum of `text_stats:<I>:<L>.doc_count` for all `L ∈ chain` with any contribution under `I` (one prefix scan of `text_stats:<I>:`, filtered to chain; cached per query).
   - Global `df(ti)` = sum of the `varint(df)` prefixes from the surviving `text_term:<I>:<ti>:<L>` entries.
   - `idf(ti) = ln((N - df + 0.5) / (df + 0.5) + 1)`.
7. For each layer `L` with hits for *all* `ti` (the AND constraint):
   - Deserialise the Roaring bitmaps for `(I, t1, L), ..., (I, tm, L)`; intersect to get the doc-id set in `L` matching the implicit AND.
   - Fetch `text_docs:<I>:<L>` (cached); resolve each doc-id to `(subject_iri, doc_length)`.
   - Score each surviving doc with BM25 using the global IDF from step 6 and the local doc length.
   - Emit `(subject_iri, score, defining_layer = L)` per surviving doc.
8. Apply the bloom-walk shadow check (§2.6, D23 §5.2): drop hits at `L` shadowed by any intermediate ancestor `M` strictly between `L` and `H` that also defines the subject.
9. Merge per-layer scored hits into a single top-k via a bounded heap; return to the planner.

**Caching.**

A bounded LRU `TermCache<(index_iri, term, layer), Arc<Roaring>>` caches deserialised bitmaps; a parallel `DocsCache<(index_iri, layer), Arc<TextDocs>>` caches the `text_docs` arrays. Sized analogously to D23's ARC cache for resource content. Both are content-addressed and never go stale because RocksDB values for committed layers are immutable; eviction is just memory pressure.

**Dependency budget.**

- `roaring` (Rust Roaring bitmaps).
- `unicode-segmentation` (tokenization).
- `rust-stemmers` (Porter stemmer, English v1).
- ~600–800 LOC of our own for the indexer, scorer, query parser, and chain-walk integration.

Transitive dep count in the low double digits, not three digits.

**v1 deferrals.**

Phrase queries (positional postings — would add a `text_pos:<I>:<T>:<L>` key per term with per-doc position arrays), fuzzy queries (Levenshtein), wildcard / regex queries, multi-language analyzers, and a richer query DSL (boost, grouping, fielded text queries beyond single-property `TEXT_MATCH`). All are extensible from this key schema without breaking changes; the deferral is about implementation scope, not architecture.

### 2.4 Vector index storage (zero-copy CBOR blobs; strategy-switched flat or HNSW per segment)

Vector indexing fits the RocksDB-blob-plus-zero-copy-view pattern even more naturally than text. The segment is again a CBOR blob, laid out so the vector array is one contiguous, alignment-padded byte string that can be cast to `&[f32]` for SIMD distance computation without copying. v1 ships *two* search strategies — flat brute-force and HNSW — selected per-property at indexing time, with the HNSW graph stored as an optional additive field in the same CBOR blob.

**Strategy selection.** Each VectorIndex Resource declares a `strategy: flat | hnsw | auto` configuration (§3.1) with `auto` as the default. The encoder chooses per-segment:

- `flat`: vectors only; queries use SIMD brute-force; exact results.
- `hnsw`: vectors plus an HNSW graph; queries use approximate-nearest-neighbour traversal.
- `auto`: build HNSW iff `count` exceeds a configurable threshold (default ~50K); below threshold, brute-force is faster than HNSW's setup cost amortised over the segment's lifetime.

Both strategies coexist on the same key schema. A query against property P transparently handles either: the reader inspects the segment for the `hnsw_graph` field and dispatches.

**Why both, not one.** Brute-force is optimal for small segments — HNSW's per-query graph traversal has higher constants than a tight SIMD loop until vector counts grow. HNSW is essential for large segments (≥100K vectors) where brute-force degrades to tens of milliseconds per segment and the chain walk fanout compounds into seconds. Life-science workloads (UMLS ~3.5M, UniProt SwissProt ~570K, SNOMED ~350K) and PubMed-scale extraction (~36M abstracts) are in HNSW territory; SE knowledge-graph workloads (D35) typically aren't. Strategy switching gives each operating point the right algorithm without forcing users to choose at deployment time. The additive CBOR layout means future v2 extensions (IVF, quantisation, mmap-HNSW for 100M+ scale) can fit the same structure without breaking the v1 schema.

**Segment layout.** A vector segment for `(layer L, property P)` is a CBOR top-level map. Two layout invariants matter for SIMD: vectors stored as a *single concatenated byte string*, and an alignment-padding field positioning the vector payload at a 32-byte boundary.

```cbor
{
  "schema_version": 1,
  "model_iri":      "urn:eigenius:embed:text-embedding-3-large",
  "dim":            <uint>,         ; e.g. 256
  "count":          <uint>,         ; n vectors
  "distance":       "cosine",       ; or "l2" / "dot"
  "subjects":       [tstr, tstr, ...],          ; n subject IRIs, parallel to vectors
  "_pad":           h'00 00 ... 00', ; pad so the next field lands on a 32-byte boundary
  "vectors":        h'<n × dim × 4 bytes of fp32, little-endian>',

  ; --- optional, present iff strategy is `hnsw` (or `auto` with count above threshold) ---
  "hnsw_graph":     h'<bytes>',     ; serialised graph: per-node level + neighbour lists
  "hnsw_params": {
    "M":               <uint>,      ; max connections per node per level
    "ef_construction": <uint>,      ; build-time exploration depth
    "max_level":       <uint>,      ; highest level present in this segment
  },
}
```

Two layout details earn their keep on the vector payload:

1. **Single concatenated `bstr` for vectors, not an array of per-vector `bstr`s.** With `count = 100k` and `dim = 256`, the array-of-bstr form would require parsing 100,000 CBOR tag/length headers to find vector *i*. The single-`bstr` form gives `&vectors[i*dim*4..(i+1)*dim*4]` in O(1). Brute-force k-NN scans the whole thing sequentially anyway, so a contiguous buffer is also cache-friendliest. HNSW traversal also benefits — random access into the vector array via node IDs is O(1).
2. **Alignment padding via the `_pad` field.** CBOR byte strings are 1-byte aligned. f32 SIMD prefers 32-byte alignment (AVX-2 `_mm256_load_ps`); 16-byte for NEON / SSE. The encoder pads `_pad` so the `vectors` payload lands at a 32-byte boundary within the blob. Cost: ≤32 bytes per segment. Result: `bytemuck::cast_slice::<u8, f32>(&vectors_payload)` produces an aligned `&[f32]` that both SIMD-distance kernels and HNSW node-fetches consume directly.

**HNSW graph encoding (when present).** Per-node fields, packed sequentially: level (varint), level-0 neighbour list (length-prefixed Vec<u32>), upper-level neighbour lists (sparse — only nodes at level ≥ L appear at each upper-level section). The on-wire format is stable across HNSW library choices so the decoder dispatches correctly regardless of which library produced the graph at build time. Loaded into RAM at SegmentCache admission alongside the vectors.

**Implementation choice.** HNSW build / search can come from `instant-distance` (minimal, ~1k LOC), `usearch` (production-grade with Rust bindings), `hnsw_rs`, or a roll-our-own from the Malkov-Yashunin paper. The schema commits to a stable on-wire format, so the library can be swapped without index migration. Recommendation deferred to implementation time; the surface in §3 doesn't depend on the choice.

**RocksDB keys** (Phase 14h-aligned with §2.3 — index IRI leads, layer last).

- `vec_seg:<index_iri>:<layer_id>` → segment blob (the CBOR map above).
- `vec_layer:<layer_id>:<index_iri>` → empty (reverse index for `drop_layer`).

Like §2.3's text keys, vector keys are keyed by the VectorIndex Resource IRI, not by the target Property IRI. The same divergent-Index-configuration story applies: per-Index keying means branches with different VectorIndex configurations have separately-addressable segments (§3.1: per-Index segment keying). Vector queries first resolve the active VectorIndex `I` for the queried Property at the query head (chain-walk discovery), then prefix-scan `vec_seg:<I>:` to enumerate every layer that contributed a segment under `I`. The reverse index `vec_layer:<L>:<I>` enumerates the Indexes layer `L` contributed to, so `drop_layer(L)` is a single prefix scan plus batched deletes.

Both live in column family `cf_vec`, separate from `cf_text` and `cf_default`. Vector blobs have different size and update profiles than text postings; isolating their compaction keeps both well-behaved.

**Zero-copy access path.**

1. `SegmentCache` miss → `db.get(vec_seg:<I>:<L>)` → `Arc<[u8]>`.
2. Header parse → `VectorSegmentLayout { model_iri, dim, count, distance_kind, subjects: Vec<Iri>, vectors_range: Range<usize> }`. The subjects list is small (typically <5% of segment size) and is parsed eagerly as `Vec<Iri>` for fast index → IRI lookup at hit time; the vectors range is just byte offsets, no decode.
3. Cache stores `Arc<CachedVectorSegment>` holding `Arc<[u8]>` + the layout.
4. At query time: `let vectors: &[f32] = bytemuck::cast_slice(&arc_bytes[layout.vectors_range])` — zero copy, aligned, SIMD-ready.

The SIMD distance kernel iterates `0..count`, computes the configured distance against `&vectors[i*dim..(i+1)*dim]` and `&query`, and maintains a bounded top-k heap. `subjects[i]` resolves the hit's IRI when a candidate enters the top-k. No heap allocation per comparison; one heap allocation total for the bounded result set.

**Query path.** Given a head H, a query vector `q`, a property P, a `k`, and an optional `ef` (HNSW search depth):

1. Resolve the active VectorIndex Resource `I` for property `P` at head `H` (chain-walk discovery via the triple index: find `VectorIndex` where `target_property = P`; take the most-recent non-shadowed match). If no active VectorIndex exists for `P` at `H`, fail at parse (§4).
2. Prefix-iterate `vec_seg:<I>:` (keys only) — stream of `<L>` for every layer that has contributed a segment under `I`. RocksDB returns only existing keys; absent layers cost nothing.
3. Collect `chain = collect_ancestors(H)` and filter the stream to `layer ∈ chain` (chain-membership filter, identical to §2.3 / Phase 14h).
4. For each surviving `L`: fetch `Arc<CachedVectorSegment>` via the SegmentCache (cache-miss path triggers the `db.get(vec_seg:<I>:<L>)` admission sequence above).
5. Verify the segment's `model_iri` matches `I`'s declared model — typechecked at parse time (§4), re-verified at runtime against the segment's recorded model.
6. **Dispatch by strategy** (per-segment, from segment metadata):
   - If `hnsw_graph` is present: HNSW traversal with parameter `ef` (default `max(k * 4, 64)`). Returns per-segment approximate top-k as `(subject, distance)` with a per-segment recall estimate.
   - Else: SIMD brute-force k-NN over the cached `&[f32]`. Returns per-segment exact top-k.
7. Apply shadow check: drop hits at `L` shadowed by any intermediate ancestor (D23 §5.2 bloom-walk).
8. Merge per-segment top-k into a global top-k via a bounded heap.

The HNSW path returns approximate results; per-segment recall depends on `ef` (typical: ~95% recall@k at `ef=k*2`, ~99% at `ef=k*8`). The final result set carries the minimum per-segment recall touched by any returned hit, so callers can reason about exactness. Queries that need exact-k-NN — duplicate detection, exact-match verification — request `strategy: flat` in the VectorIndex declaration, or override via a query-time hint (§3).

**Capacity math.**

- **Flat** (brute-force, AVX-2 SIMD):
  - 100K vectors at 256 dim: ~2 ms per segment (cache-warm).
  - 1M vectors: ~20 ms per segment.
  - 10M vectors: ~200 ms per segment.
- **HNSW** (typical parameters M=16, ef_search ≈ k*4):
  - 100K vectors: ~0.5 ms per segment.
  - 1M vectors: ~1 ms per segment.
  - 10M vectors: ~2 ms per segment.
- **Build cost.** HNSW construction is 10–100× slower than flat. For a 1M-vector segment, ~30–60 s of build time per `(layer, property)`, run inside the post-Load sweep or consolidation. Background; not user-facing latency.
- **Per-segment memory** (cache-resident):
  - Flat: 4 × dim × count bytes for vectors + per-subject IRI strings.
  - HNSW: flat overhead + ~M × 8 × count bytes for the graph. At M=16, ~128 bytes/vector overhead. A 10M-vector 256-dim segment occupies ~10 GB flat + ~1.3 GB HNSW.

**v1 operating envelope.** Comfortable up to ~10M vectors per segment given the in-memory HNSW footprint. Coverage:

- Comfortably handles: UMLS (3.5M concepts), UniProt SwissProt (570K), SNOMED CT (350K), ChEBI / DrugBank / Reactome, GO, bounded-scope PubMed extractions (millions of abstracts), the SE knowledge graph for any plausibly-sized codebase.
- Stretches: PubMed-scale full ingestion (100M+ vectors), TrEMBL-scale protein corpora (250M+), enterprise-knowledge-base scale beyond ~10M per segment.

**v2 work is triggered** when a real workload demonstrates segments above ~10M vectors with measured memory pressure or query latency above operational thresholds. v2 extensions stay forward-compatible with the §2.4 layout — vector quantisation (int8 for 4× compression) and IVF clusters land as new optional fields; out-of-core HNSW with mmap is an alternative loader path keyed on the same `hnsw_graph` bytes. No schema migration.

### 2.5 Atomic commit

The combined invariant: a layer is queryable through any of its indexes (triple, text, vector) iff `topo:<layer_id>` is committed. With all index data in RocksDB, the implementation is uniform:

- `LayerBuilder::build` produces in-memory descriptions of each index's per-layer contribution: triple-index triples; text-index postings (`text_term:`, `text_docs:`, `text_stats:`, `text_terms_layer:` per active TextIndex Resource); vector-index segments (`vec_seg:` plus `vec_layer:` reverse per active VectorIndex Resource).
- All of them go into the same `WriteBatch` that carries the layer record (`topo:`, `chain:`, `bloom:`, `branch:` where applicable) plus the triple-index entries (`idx_pos:`, `idx_layer:`).
- `RocksStore::store_layer` writes the batch atomically. Layer visibility and all three index types become visible at the same moment.

No filesystem materialisation, no fence pointer, no orphan-recovery sweep. The atomic-commit story is the same envelope Phase 14h established; D43 adds the text and vector key families (in dedicated column families) to the batch.

One structural exception, developed in §5.6: the `vec_seg:<I>:<L>` segment is the only index entry that may be backfilled by a post-Load sweep rather than written atomically with the originating layer. The relaxation is narrowly scoped — it exists because vector derivation requires an IO call to an embedder, and forcing atomicity there would gate Load on the embedder's availability. Every other index entry stays atomic-with-Load.

### 2.6 Shadow check

For all three index types — triple, text, vector — shadow semantics are identical.

A hit produced by layer `L` against head `H` is dropped if any layer `M` strictly between `L` and `H` in the head's ancestor topology also defines the subject of the hit. The mechanism is the bloom-walk from D23 §5.2: BFS from `H` down to `L` (excluding `L` itself), bloom-probing each visited layer for `subject`. First confirmed hit → shadowed.

The bloom used here is the existing per-layer shadowing bloom from D23 (the `bloom:<layer_id>` filter over all subjects defined in `L`); it answers the subject-shadow question after a hit. D43 does not introduce any second bloom — the Phase 14h key ordering already makes "does this layer contribute to property P / term T at all" a cheap prefix-existence check (key absent = no contribution), so we have no need for a per-`(layer, property)` bloom in the hot path.

### 2.7 GC and layer deletion

`delete_layer(L)` drops every key contributed by L across `cf_text` and `cf_vec` via the layer-keyed reverse indexes:

- **Text.** Prefix-scan `text_terms_layer:<L>:` → enumerate `(index_iri, term-list)` pairs that L contributed under each active TextIndex. For each pair `(I, [T1, ..., Tk])`, batch-delete `text_term:<I>:<Ti>:<L>` for each `Ti`, plus `text_docs:<I>:<L>`, `text_stats:<I>:<L>`, and the `text_terms_layer:<L>:<I>` key itself.
- **Vector.** Prefix-scan `vec_layer:<L>:` → enumerate the VectorIndexes L contributed to. For each `I`, batch-delete `vec_seg:<I>:<L>` and the `vec_layer:<L>:<I>` key itself.

All in the same `WriteBatch` as the rest of the layer drop. This is the Phase 14h `idx_pos:` / `idx_layer:` pattern, applied uniformly across the three index types — the text variant carries the term list inside its reverse-index value so the cleanup can enumerate the term-keyed entries; the vector variant just enumerates properties because the segment is one key per `(property, layer)`. The text-index delete cost scales as O(unique terms in `(L, P)`) per property — typically thousands per layer, not millions. No filesystem step, no orphan recovery — once the batch commits, the layer's index footprint is gone from every column family in one atomic operation.

### 2.8 Consolidation (D25 interaction)

D25 collapses a contiguous ancestral range `[from..to]` into a single layer `C`. For triple-index entries, consolidation re-labels: surviving definitions get `defining_layer = C`. The same shape applies to text and vector indexes, with the actual mechanics differing because text is derived (tokenisation, postings) while vector is materialised (embeddings already computed and stored):

- **Text.** Re-extract from the resolved Resource set as of the collapsed range. For each active TextIndex `I` whose `target_property` matches a property in the resolved set, the §2.3 indexing pipeline runs over the surviving subjects with fresh local doc-ids assigned within `C`, producing new `text_term:<I>:<T>:<C>` (one per term), `text_docs:<I>:<C>`, `text_stats:<I>:<C>`, and `text_terms_layer:<C>:<I>` entries. Re-extraction is necessary because per-layer Roaring bitmaps use layer-local doc-ids; merging them across layers would require remapping doc-ids and re-deduplicating subjects that appear in multiple layers — more complex than just rebuilding. The collapsed range's N key-sets per Index are replaced by one new key-set per Index.
- **Vector.** For each active VectorIndex `I` in the resolved set, concatenate the surviving vectors from the collapsed range under `I` — re-labelled with `defining_layer = C`, encoded per the §2.4 layout, written to `cf_vec` as `vec_seg:<I>:<C>`. Re-embedding is not required: the embedding model IRI travels with each vector; the consolidation preserves it. The §2.4 strategy applies to the consolidated segment: if `I`'s strategy is `hnsw` (or `auto` and the consolidated `count` exceeds the threshold), the consolidation rebuilds the HNSW graph over the consolidated vector set. The `vec_layer:<C>:<I>` reverse-index entry is written in the same batch.

The atomicity story matches D25's existing invariant: consolidation is one `WriteBatch` that writes the new consolidated layer's records and *all* its index entries (triple, text, vector), then deletes the collapsed-range layers' records and index entries via the layer-keyed reverse indexes (`text_terms_layer:`, `vec_layer:`). Resolve-equivalence under head substitution (D25's load-bearing invariant) extends to "search-equivalence" for retrieval: the same query at the same head produces the same set of subjects before and after consolidation. Scoring is also stable under consolidation for text (BM25 with chain-aware IDF is computed at query time from the surviving stats, so consolidating the chain doesn't shift the IDF distribution as long as the resolved subject set is the same) and for vector (distances are computed against the unchanged query vector; the surviving vectors haven't moved).

**v1 stance.** Refuse to consolidate ranges spanning the same kinds of merge-node boundaries D25 v1 refuses. Mixed-model consolidation is structurally impossible under §5.7's reindex-required model-upgrade policy — there is never a chain range that crosses a model boundary, because model upgrades atomically replace all `vec_seg` entries for the affected property. The earlier "refuse across model upgrades" guard is therefore unnecessary and is dropped.

---

## 3. EigenQL surface additions

D43 introduces two kinds of surface additions: *ESL index declarations* (top-level Resource declarations of class `core:TextIndex` or `core:VectorIndex`, separate from the Property definitions they target — §3.1), and a single *EigenQL similarity operator* `~` that queries those indexes (§3.3) with an optional hint block (§3.4). The grammar deltas land in D2's next revision; D43 specifies the semantics and types.

The design stance here matters: D43 deliberately keeps the user surface small. Retrieval implementations (BM25, vector cosine, RRF fusion), embedding models, and per-source scores are all platform-internal — not user-visible primitives. §3.2 explains why and what falls out.

### 3.1 ESL index declarations

Indexes are first-class Resources of class `core:TextIndex` or `core:VectorIndex`, declared as top-level ESL blocks separate from the Property definitions they target. This separation has three structural payoffs:

1. **Schema and operational concerns separated.** Property definitions capture the semantic data model — what a property *is*. Index declarations capture operational configuration — *how* the property's values should be retrievable. Changing the analyzer, the embedding model, or HNSW parameters is operational tuning; it does not modify the Property's identity, lifecycle, or downstream consumers.
2. **Per-deployment indexing of shared schemas.** A Property defined in a shared ontology layer can have different Indexes declared in different deployment layers. One deployment indexes `description` with one analyzer; another with a different model. The base schema stays portable; indexing strategy is a local choice.
3. **Index lifecycle independent of Property lifecycle.** Indexes can be added, removed, or revised without re-issuing the Property definition. The §5.7 atomic-reindex policy operates on Index Resources, not on Property definitions — schema evolution and index tuning stay as separate concerns.

The ESL block forms `text_index "name" { ... }` and `vector_index "name" { ... }` are syntactic sugar for `resource "name" of <Index class> { ... }`; semantically, they produce normal Resources in the chain that are discoverable through standard chain walking.

**`text_index` declaration.**

```esl
text_index "description_en" {
    target_property = "urn:eigenius:se:description"
    analyzer        = "en-stem-v1"     // optional; default "en-stem-v1"
}
```

- Block name (`"description_en"`) is the index Resource's short_name; the full IRI is built from the host namespace per the standard naming convention.
- `target_property` (required, IRI). The Property to index. Must resolve to a Property in the visible chain at Load time; the Property's value type must be string-shaped.
- `analyzer` (optional, string, default `"en-stem-v1"`). Names the analyzer that performs tokenisation, lowercasing, and stemming. v1 ships `"en-stem-v1"` (Unicode segmentation + lowercase + Porter English stem) and `"en-no-stem"` (Unicode segmentation + lowercase, no stem). Additional analyzers are additive; the analyzer ID is recorded in the per-segment `text_stats` metadata so the query-side analyzer matches the index-side analyzer.

**`vector_index` declaration.**

```esl
vector_index "description_oai_v3" {
    target_property  = "urn:eigenius:se:description"
    model            = "urn:eigenius:embed:openai:text-embedding-3-large:v3"
    dimensionality   = 1536
    distance         = "cosine"        // optional; default "cosine"
    strategy         = "auto"          // optional; default "auto"
    hnsw_params      = { M = 16, ef_construction = 200 }  // optional; sensible defaults if omitted
    embedding_policy = "eager_on_load" // optional; v1 ships eager_on_load only
}
```

- `target_property` (required, IRI). The Property to embed and index. Must resolve to a Property in the visible chain at Load time; the Property's value type must be string-shaped (v1 only embeds text content).
- `model` (required, IRI). The embedder Component IRI (§5.2). Must reference a registered Embedder Component; the planner verifies registration at parse time.
- `dimensionality` (required, integer). Declared statically so EigenQL can type vector expressions at parse time without runtime probes. Must match the Embedder Component's declared output dimensionality (verified at parse).
- `distance` (optional, enum: `cosine | l2 | dot`, default `cosine`). The distance metric the platform uses when computing similarity against this VectorIndex's segments at query time, and by the segment encoder at index time.
- `strategy` (optional, enum: `flat | hnsw | auto`, default `auto`). Drives §2.4's per-segment selection.
- `hnsw_params` (optional, struct). HNSW build-time parameters; sensible defaults (M=16, ef_construction=200) apply when omitted. Required to be consistent across all segments of a given VectorIndex — the §5.7 atomic-reindex policy is the mechanism for changing them.
- `embedding_policy` (optional, enum: `eager_on_load | lazy_on_query | manual`, default `eager_on_load`). v1 ships `eager_on_load` only; the other values reserve grammar slots for future revisions (§5.9).

**Class definitions** for `core:TextIndex` and `core:VectorIndex` are added to the core ontology (loaded at kernel boot) as part of D43 implementation. Their property structure matches the field set above.

**v1 multiplicity constraint.** At most one *active* (non-shadowed) TextIndex and at most one *active* VectorIndex may target a given Property at any chain head. Validated at Load: if a layer's commit would produce a chain state at any head where two TextIndexes (or two VectorIndexes) target the same Property, the Load fails. The constraint is a v1 simplification that avoids query-time index disambiguation; future revisions may relax it to support multi-index-per-property with explicit selection in queries. A single Property may simultaneously have an active TextIndex and an active VectorIndex — the two index kinds are independent.

**Per-Index segment keying** (the cross-chain story). All retrieval-index keys in `cf_text` and `cf_vec` are keyed by the Index Resource's IRI, not by the Property IRI. See §2.3 and §2.4 for the schemas. This is the load-bearing mechanism for handling divergent Index configurations across branches:

- **Branches sharing the same active Index Resource share segments.** Two branches that both reach an Index Resource `I` through their chain walks both read and write the same `text_term:<I>:*` (or `vec_seg:<I>:*`) keys. No duplication.
- **Branches with different active Index Resources have separately-addressable segments.** When a branch writes a layer that declares a new `TextIndex` Resource `I_v2` with different parameters (e.g., a different analyzer), `I_v2` has a fresh IRI. Its segments live under `text_term:<I_v2>:*`; the prior `I_v1`'s segments stay under `text_term:<I_v1>:*`. Storage is conflict-free; queries from each branch reach the right segments through the chain-walk discovery of the active Index.
- **Query-time resolution.** The planner walks the chain at the query head, finds the active Index Resource for the queried Property (via the existing triple index — "find TextIndex where `target_property = P`"), and probes that Index's segments. Each branch sees its own active Index; no leakage across branches.
- **Old Indexes don't disappear when they're shadowed.** If branch A keeps using `I_v1` and branch B has moved to `I_v2`, both Indexes' segments remain in storage and remain queryable from any branch that references them. Once no chain head references an Index Resource, its segments become GC-reclaimable through standard reachability-based GC (D23 §5.7).

**Discovery and indexing trigger.** At Load time, the kernel walks the chain to find Index Resources whose `target_property` matches any Property carried by Resources in the committing layer. The §2.3 indexing pipeline (text) or §5.5 sweep (vector) triggers based on the discovered Index Resources. An Index Resource is the *only* signal that triggers indexing — a Property without an Index Resource targeting it is not indexed.

**Removing or revising an Index.** Standard layer/shadow semantics. Add a new layer that supersedes the Index Resource — redefining it with new parameters, or marking it removed. Changes that affect segment contents (model, dimensionality, distance, analyzer for text; any HNSW parameter for vector) trigger the §5.7 atomic-reindex policy so segments rebuild in lockstep with the Index Resource update. Changes that don't affect segment contents (e.g., `embedding_policy` swaps once additional policies ship) are non-reindex-triggering.

### 3.2 Design stance: one operator, hidden mechanism

D43 surfaces retrieval as a *single user-visible operator* — the similarity test `~` between a property-bound variable and a text query — and hides everything else (which index produced the result, what model embedded the query, how multiple sources fuse, what the raw scores are). Users express intent ("find resources whose property value is most related to this query"); the platform commits to the implementation. The ranking that flows out of the operator drives the standard `TOP K` / `LIMIT` clauses; users never see scores, ranks, fusion algorithms, or embedding vectors.

This stance differs from a SQL-shaped surface (separate primitives for text and vector retrieval; explicit score projection; user-visible fusion functions like RRF; embedding vectors as first-class properties). The rationale:

- **Implementation is not the user's concern.** Whether a query uses BM25, vector cosine, or both fused isn't something the agent or human author wants to think about. Surfacing the choice forces every query author to learn two retrieval paradigms and a fusion algorithm, and creates compositional friction (separate primitives don't compose as naturally as one operator does).
- **The schema is the policy.** Which indexes a property has determines retrieval behavior. Schema owners pick text-only, vector-only, or hybrid by declaring exactly those indexes — at schema time, where the choice is reviewable, rather than at every call site.
- **Defaults serve the 95% case; hints serve the 5%.** A trailing braces block on the operator (§3.4) lets power users override fusion, force a specific index, or pin an embedder model — but the unhinted operator is the path most queries take.
- **The platform owns scoring details.** RRF, k=60, the fusion algorithm, per-source weighting, query embedding — all platform-internal. Tuning surface emerges only when use cases demand it; the default is opinionated and out of the way.

What that costs: queries that want to inspect raw scores (debugging; per-source attribution) need a diagnostic surface (`EXPLAIN`-equivalent, §3.7) rather than putting scores back in the language. That's a deliberate trade — the common path stays clean.

### 3.3 The similarity operator

A single binary operator `~` between a property-bound variable and a text query:

```
?property_var ~ "natural-language query"
```

**Semantics.** In any position where it appears (`WHERE`, `RETURN`, `TOP K BY`), the operator denotes "rows where this property value is related to this query, ordered by relatedness." The platform consults every active similarity index on the property's source `Property`, dispatches probe(s), and produces a per-row relevance contribution that participates in the implicit ranking.

The right-hand side is a string (literal in v1; an expression in future revisions). For vector-indexed properties, the platform embeds the RHS string using the property's declared embedder (§3.1's `vector_index { model }`); for text-indexed properties, it parses the RHS through the property's declared analyzer. The user never names the embedder, never sees the vector.

**Where it can appear:**

- `WHERE` — filter + ranking contribution. The operator admits rows where the platform decides the property value is "related enough" (by a platform-chosen threshold), and the row's position in the result ordering is derived from its relatedness score.
- `TOP K` — implicitly drives ranking. When the `WHERE` contains similarity operators, `TOP K` truncates by the relatedness-derived ordering. No `BY` clause needed in the pure-similarity case.

Multiple `~` operators in the same query compose:

- `?a ~ "x", ?b ~ "y"` (conjunction) — both contribute to the ranking; rows that satisfy both rank higher.
- `?a ~ "x" OR ?b ~ "y"` (disjunction) — either contributes; rows satisfying both rank higher than rows satisfying one. (This is the hybrid retrieval shape from §6.5.)

Fusion happens internally when multiple `~` operators are in scope, or when a single `~` operator's property has multiple active similarity indexes. The fusion algorithm and parameters are platform-determined (§3.5).

**Reading the operator.** `~` is the user-facing surface for "*find related things*." Schema-side `core:TextIndex` and `core:VectorIndex` declarations (§3.1) are the schema-side surface for "*this property is similarity-retrievable.*" The two surfaces compose: declare what's retrievable, then query it with `~`.

### 3.4 Hint surface

An optional trailing braces block on the operator overrides individual platform defaults:

```
?property_var ~ "query" { via: text|vector|hybrid, model: <iri>, k: <int>, limit: <int> }
```

**Hint keys** (all optional; any subset):

| Key | Type | Effect |
|---|---|---|
| `via` | `text` / `vector` / `hybrid` | Force the strategy. `text` uses only the TextIndex; `vector` uses only the VectorIndex; `hybrid` fuses (default when both indexes are active). Mutually exclusive with hints that imply a different strategy. |
| `model` | IRI | Override the embedder. Implicitly forces `via: vector`. Useful when a property has multiple VectorIndexes declared (currently constrained to one per kind in v1 — see §3.1 — but the hint is forward-compatible). |
| `k` | positive integer | Override the RRF fusion constant. Default 60 (Cormack et al.). Affects the relative weighting of rank 1 vs rank N in the fused output. |
| `limit` | positive integer | Probe-side cap — bound the per-source candidate set the platform fetches before fusion / ranking. Tightens the over-fetch policy locally. |

**Examples:**

```eigenql
?desc ~ "WAL truncation"                                  // default everything
?desc ~ "WAL truncation" { via: text }                    // text-only override
?desc ~ "WAL truncation" { via: vector, model: "..." }   // vector path with custom embedder
?desc ~ "WAL truncation" { k: 30 }                        // tighter RRF (rank-1-dominant)
?desc ~ "WAL truncation" { limit: 50 }                    // bound the probe candidate set
```

**Validation** (at typecheck, per §4):

- Hint keys are checked against the allowed set; unrecognised keys fail with a clear error.
- `via: text` requires the property to have an active TextIndex; `via: vector` requires an active VectorIndex; `via: hybrid` requires at least one of each (or, more permissively, at least one similarity index of any kind).
- `model: M` requires an active VectorIndex on the property declaring model `M`. (In v1, this is just a defence against typos since at most one VectorIndex per property is allowed.)
- `model:` and `via: text` are mutually exclusive — the model only applies to the vector path.
- `k:` is a positive integer literal.
- `limit:` is a positive integer literal.

**Defaults stay implicit.** A query without hints picks up the platform defaults (active-index set determines strategy; RRF k=60 for fusion; platform-chosen probe size for over-fetch). Hints exist for the cases where defaults are wrong; they're not part of the common-path surface.

**Forward-compatible extension.** New hint keys land additively without breaking existing queries. Reserved names for future revisions: `weights:` (per-source weighting), `threshold:` (minimum relevance floor), `analyzer:` (override text analyzer). None ship in v1.

### 3.5 Default fusion behavior and active-index discovery

The operator's behavior is fully determined by the schema view at the query head, with no query-time heuristics:

1. **No active similarity index on the property.** `?p ~ q` fails at typecheck with "Property P has no active similarity index at head H." Same diagnostic shape as the pre-v1 "property has no active TextIndex" check, generalised across both kinds.
2. **Exactly one active index** (TextIndex *or* VectorIndex). The platform probes that index; the result drives ranking. No fusion needed.
3. **Both active** (TextIndex *and* VectorIndex). The platform runs both probes in parallel (§6.5 hybrid scheduling), produces per-source ranked candidate sets, fuses via Reciprocal Rank Fusion with `k=60` default (§6.4).
4. **Multiple `~` operators in the same query.** Each operator contributes a ranked source. The platform fuses all sources via the same RRF mechanism. AND-joined operators contribute to all rows that pass both filters; OR-joined operators contribute to the union of their candidate sets.

**RRF formula** (platform-internal; users don't see it but documented here for traceability):

```
fused_score(row) = sum over sources i of  1 / (k + rank_i(row))
```

Where `rank_i(row)` is the row's 1-indexed position in source `i`'s ordering, or `∞` if the row didn't appear in source `i`'s candidate set. `k=60` is the default smoothing constant; the `k:` hint overrides it per query.

**Same chain, same query → same plan, always.** No query-shape heuristics. The schema owner controls strategy by declaring the indexes they want; the query author writes intent.

### 3.6 Worked examples

**Pure text retrieval** (property has only a TextIndex):

```eigenql
USING "urn:eigenius:se:Doc"
MATCH Doc(?d) { description: ?desc }
WHERE ?desc ~ "kernel layer chain consolidation"
TOP 20
```

**Pure vector retrieval** (property has only a VectorIndex). The query string is the same shape; the platform embeds it using the declared model:

```eigenql
USING "urn:eigenius:se:Doc"
MATCH Doc(?d) { description: ?desc }
WHERE ?desc ~ "collapsing a contiguous range of layers"
TOP 20
```

**Hybrid retrieval** (property has both indexes). The query is unchanged; the platform fuses internally:

```eigenql
USING "urn:eigenius:se:Doc"
MATCH Doc(?d) { description: ?desc }
WHERE ?desc ~ "kernel layer chain consolidation"
TOP 20
```

**Composition with structural filters.** The structural pattern filters; the similarity operator ranks the survivors:

```eigenql
USING "urn:eigenius:se:RustFunction", "urn:eigenius:se:Module"
MATCH RustFunction(?f) { declared_in: ?m, description: ?desc },
      Module(?m) { short_name: "evaluate" }
WHERE ?desc ~ "walk the chain and apply shadow filter"
TOP 50
```

**Disjunctive sources** (multi-property hybrid). The platform fuses contributions from both operators:

```eigenql
USING "urn:eigenius:se:Doc"
MATCH Doc(?d) { title: ?t, body: ?b }
WHERE ?t ~ "WAL truncation"
   OR ?b ~ "rolling back a partial commit"
TOP 20
```

**Hint-driven override** (force text-only for an exact-match-heavy property):

```eigenql
USING "urn:eigenius:se:Symbol"
MATCH Symbol(?s) { name: ?n }
WHERE ?n ~ "RocksStore::store_layer" { via: text }
TOP 10
```

**Probe-side limit** (recall-vs-precision tuning):

```eigenql
MATCH Doc(?d) { description: ?desc }
WHERE ?desc ~ "concurrent commit recovery" { limit: 50 }
TOP 20
```

Compare to the pre-collapse spec form (kept here as a migration reference, *not* as supported syntax). The D35 §7.4 worked example was:

```eigenql
WHERE TEXT_MATCH(?desc, "WAL truncation concurrent commit")
   OR VECTOR_NEAR(?vec, EMBED("rolling back a partial commit"), k: 50)
RETURN ?d,
       TEXT_SCORE(?desc, "WAL truncation concurrent commit") AS ts,
       VECTOR_SIM(?vec, EMBED("rolling back a partial commit")) AS vs,
       RRF(ts, vs) AS fused
TOP 20 BY fused DESC
```

Under D43 v1, this reduces to:

```eigenql
WHERE ?desc ~ "WAL truncation concurrent commit"
   OR ?desc ~ "rolling back a partial commit"
TOP 20
```

(Assuming a single `description` property indexed by both TextIndex and VectorIndex, which is what real corpora look like — there's no separate `description_embedding` property; the vector lives in the index, not in the schema.)

### 3.7 Surface affordances deferred

- **Phrase queries.** Quoted exact phrases within the RHS string (`"exact phrase"`) are reserved syntax; v1's text analyzer treats the quotes as token characters. Phrase queries require positional postings (§2.3) and an extended query grammar; deferred.
- **Boolean operators in the query string.** v1 implicit-AND across whitespace-separated terms. `OR`, `NOT`, parenthesisation inside the string are deferred; users compose multiple `~` operators at the EigenQL level instead.
- **Per-source weighting.** `?p ~ "q" { weights: [0.7, 0.3] }` (or schema-side weighting on the index declaration) for non-uniform fusion. Deferred until a workload demonstrates uniform RRF is wrong.
- **Relevance threshold.** `?p ~ "q" { threshold: 0.5 }` to set a minimum-relatedness floor. Today the platform picks an internal threshold; the hint surfaces tuning. Deferred.
- **Diagnostic / `EXPLAIN` surface.** A query-level mode that reports the per-source candidate sets, ranks, and fused scores for debugging. Not in v1; it's the natural compensation for hiding the scores from the language, so it lands as soon as users need to debug rankings.
- **Multi-field hybrid.** `?Doc ~ "q"` (no property variable; "find Docs by total relevance across all their similarity-indexed properties") is plausible v1.1 sugar over `?title ~ "q" OR ?body ~ "q"`. Deferred.
- **Multiple indexes of the same kind per property** (e.g., two TextIndexes with different analyzers). §3.1's v1 multiplicity constraint stays; relaxing it is forward-compatible with the operator (the platform would fuse all active indexes of any kind).

## 4. Type system extensions

D43's user-visible surface is small enough that the type system extensions are correspondingly small. The single `~` operator and its optional hint block are the only new shapes. The Vector type, EMBED type inference, and score-expression composition rules from earlier D43 drafts are *gone* — those constructs were collapsed in §3, so they no longer need typecheck rules.

What remains: validating that the operator's left-hand side resolves to a similarity-indexed property, validating the right-hand side is a string-typed expression, and validating the hint block against the property's active index set.

### 4.1 Schema view at typecheck

The typechecker operates against a *fixed schema view* derived from the query head H. The view aggregates all `Class`, `Property`, `core:TextIndex`, and `core:VectorIndex` Resources visible at H through the standard chain-walk.

For retrieval typing, two derived lookups matter:

- **`active_text_index(P, H)`** — the set of non-shadowed `TextIndex` Resources whose `target_property = P` at H. In v1 the §3.1 multiplicity constraint allows at most one, so the lookup returns `Option<TextIndex>`.
- **`active_vector_index(P, H)`** — same shape, for `VectorIndex`.

Combined into the operator's lookup:

- **`active_similarity_indexes(P, H)`** — the union of the two: returns the set of similarity-providing Index Resources active for property `P` at head `H`. Empty if neither TextIndex nor VectorIndex targets `P`; size 1 in the text-only or vector-only case; size 2 in the hybrid case. This is the lookup the `~` operator's typecheck consults.

Lookups are memoised for the duration of a typecheck pass; the chain-walk cost is paid once per distinct property reference in the query.

The schema view is *frozen* for the typecheck — even if H is the tip of a write-active branch, typecheck sees the snapshot at submit time. Same query against different heads can typecheck differently (an Index Resource active in one branch but shadowed in another); this is by design.

### 4.2 Property reference typing

Unchanged from D2: a property reference `?p` bound through a `MATCH` pattern carries the value type declared by its source `Property`'s `class_types`. The similarity operator does not change this type — it uses the active Index Resources as a *lens* through which retrieval semantics apply.

For example, in `MATCH Doc(?d) { description: ?desc }`:

- `?desc` has value type `String`.
- For `?desc ~ "q"` to typecheck, the typechecker additionally requires `active_similarity_indexes(description, H)` to be non-empty.
- The operator uses whichever indexes are active for retrieval; `?desc`'s static type stays `String`.

This separation — value type stays declared, retrieval lens comes from the active Index Resources — is what lets the same property carry both a `TextIndex` and a `VectorIndex` simultaneously without colliding type identities.

### 4.3 Similarity operator typecheck

```
?prop ~ <query> [ { <hints> } ] : Boolean
```

When evaluated as a boolean (e.g., as a WHERE filter), the operator returns true iff the property value is similarity-related to the query under the platform-chosen interpretation. When used as a ranking input (in WHERE alongside `TOP K`), it contributes to the implicit ordering. The operator is *not* a value-returning expression in v1 — there is no exposed Float score the user can bind to a variable. (Diagnostic surfaces that expose per-source scores live in §3.7's deferred `EXPLAIN`-equivalent surface, not in the operator itself.)

**Typing constraints:**

1. **LHS** must be a property-bound variable. A `MATCH` pattern binds `?prop` to the value of some Property; `~` is only legal when `?prop` came from a property pattern. Errors with "left-hand side of `~` must be a property-bound variable" when the LHS is a literal, an unbound variable, or a variable bound by a different mechanism (e.g., a FIBER binding).
2. **LHS property** must have at least one active similarity index — i.e., `active_similarity_indexes(P, H)` is non-empty where `P` is the property the LHS variable was bound by. Errors with "property `P` has no active similarity index at head `H`" otherwise. The error mentions both kinds so the schema owner knows what to declare.
3. **LHS property** must have a string-shaped value type. Errors with "property `P` has value type `T`; similarity requires String-shaped" otherwise. (v1 only embeds and tokenises text content.)
4. **RHS** must be a string-typed expression. v1 accepts only string literals; future revisions may accept expressions resolving to strings (e.g., a `?bound_var` carrying a runtime-determined query). Errors with "right-hand side of `~` must be a string expression" otherwise.

### 4.4 Hint validation

The trailing braces block on the operator is validated against the active index set:

**Per-hint type checks:**

| Hint | Check |
|---|---|
| `via` | Value is one of `text`, `vector`, `hybrid`. Other values error with "unknown via strategy: <value>". |
| `model` | Value is a string parseable as an IRI. Errors otherwise. |
| `k` | Value is a positive integer literal. Errors on zero, negative, or non-literal expressions. |
| `limit` | Value is a positive integer literal. Same shape as `k`. |
| Other keys | Reserved or unknown — error with "unrecognised hint key: <key>". |

**Consistency checks against the property's active index set:**

| Condition | Error |
|---|---|
| `via: text` on a property without an active TextIndex | "via: text requires an active TextIndex on property P (none declared at H)" |
| `via: vector` on a property without an active VectorIndex | "via: vector requires an active VectorIndex on property P (none declared at H)" |
| `via: hybrid` on a property with only one active index kind | "via: hybrid requires both a TextIndex and a VectorIndex on property P (only X declared)" |
| `model: M` and the active VectorIndex doesn't declare model M | "model: M doesn't match the active VectorIndex on property P (which declares M')" |
| `model:` combined with `via: text` | "model: is incompatible with via: text — the model only applies to the vector path" |

A query that fails any of these errors at parse, never at evaluation. The user sees their mistake before the kernel does any retrieval work.

### 4.5 Failure modes at typecheck — summary

| Condition | Error |
|---|---|
| `~` on a property with no active similarity index at H | "property P has no active similarity index at head H" |
| `~` LHS not a property-bound variable | "left-hand side of `~` must be a property-bound variable" |
| `~` on a non-string property | "property P has value type T; similarity requires String-shaped" |
| `~` RHS not string-typed | "right-hand side of `~` must be a string expression" |
| Hint with unknown key | "unrecognised hint key: <key>" |
| Hint with wrong-typed value | "<hint>: expected <type>, got <actual>" |
| Hint inconsistent with active index set | (per-hint error message from §4.4 table) |

These all happen at parse / typecheck. Runtime sees only well-typed expressions plus the IO-level checks of §4.6.

### 4.6 Runtime checks (recap)

The typechecker catches schema-level and structural issues. A small number of conditions remain runtime-only:

- **Embedder unreachable / Component failure** during similarity evaluation against a vector-indexed property — §5.8 IO-failure handling. The query embeds the RHS string at evaluation time; if the embedder is unavailable, the query fails with the standard D6 IO-Component-failure error.
- **Per-segment `model_iri` verification** at evaluation — §2.4 query path step 5. The segment's recorded `model_iri` is re-verified against the active VectorIndex's `model_iri` as defence-in-depth; a mismatch should never happen if the §5.7 reindex policy is honoured, but if one occurs the query fails with `SegmentModelMismatch` rather than silently returning garbage.
- **Vector segment not yet materialised** — the sweep hasn't completed for some `(layer, VectorIndex)` pair (§5.5). That layer contributes nothing to the query; results are well-typed but partial. The result includes sweep-status visibility for callers that need to know.

Runtime never sees a typing error — by the time evaluation begins, every expression's type is concrete and every Index Resource referenced is bound to a specific IRI in the schema view.

## 5. Embedding lifecycle

The vector index of §2.4 needs vectors. Vectors come from running source content (typically text properties) through an embedding model. Embedding is **IO-dependent and non-deterministic across model versions** — fundamentally different from the deterministic derivations Load handles atomically. §5 specifies how embedding fits into the layer model without compromising Load reliability or query correctness.

The core decisions:

- Text indexing stays synchronous-and-gating at Load (pure, cheap, deterministic).
- Vector embedding is asynchronous-and-non-gating: Load commits without waiting on the embedder; a sweep produces the vector segments later (§5.5).
- Embeddings are *values* computed by an `Embedder` **Component** (per D3 / D6 / D12 / D26), not chain Resources. Provenance lives in the Component's reasoning trace (D6b).
- Indexing-side and query-side embedding paths dispatch the *same* Embedder Component against the *same* content-addressed cache (§5.3), so repeated content — whether the same string indexed in many layers or the same query string issued repeatedly — embeds once.

### 5.1 Two embedding paths, one Embedder Component

Two distinct call sites need embeddings:

1. **Indexing-side.** When a Resource enters the chain whose property is targeted by an active VectorIndex `I`, the property's value is embedded for storage in `vec_seg:<I>:<L>`. Many calls per layer, batched, results stored persistently in segments.
2. **Query-side.** When a query contains `EMBED("literal text", model: ?m)`, the literal string is embedded so the resulting vector can flow into `VECTOR_NEAR`. One or a few calls per query, results ephemeral.

Both dispatch the same `Embedder` Component for a given `model_iri`, and both hit the same content-addressed cache. A query whose `EMBED("...")` matches the content of a previously-indexed Resource returns immediately from cache; an indexing pass populates the cache that future queries hit. Cross-path reuse is automatic and is the operational win that makes hosted-API embedders affordable at scale.

### 5.2 The Embedder Component

An `Embedder` is declared as a Component per D3 §3 with:

- `capability_level: IO` — dispatched through the orchestrator per D6 §3.
- `determinism: NonDeterministic` — per D3 §6.2; output can shift across model versions, hosted-API silent upgrades, or non-deterministic decoding strategies.
- `input_type:  { source_content: bytes, model_iri: Iri }`
- `output_type: { vector: bytes, dimensionality: uint, model_iri: Iri }`

One Embedder Component is registered per supported model. The model IRI is part of the Component's identity — `urn:eigenius:embed:openai:text-embedding-3-large:v3` and `urn:eigenius:embed:openai:text-embedding-4-large:v1` are distinct Components with distinct dispatch routes. Implementations may be WASM (for an in-process ONNX runtime, say) or substrate-dispatched (per D26 / D27 / D31 for Python / hosted-API embedders); the platform does not constrain the choice, the embedding cache and the dispatch envelope are the same.

Embedder Components are not institutions and do not register QueryClasses. They expose no FIBER surface, no `Verdict` shape, no comorphism story. They are computational endpoints reached through the existing Component dispatch envelope — the same path any IO Component uses. Treating embedders as a `QueryClass(OnDemand)` (as the §5 stub originally proposed) was a shape mismatch: a QueryClass returns a verdict on a sentence in some institution's logic; an embedder returns a real-valued vector with no institutional semantics.

### 5.3 Content-addressed embedding cache

Embeddings are cached by `(content_hash, model_iri)`. The cache is a dedicated content-addressed store with its own lifecycle, in the same pattern as D33's anchored-commit cache — content-addressing for deterministic reuse of expensive computations.

- **Key**: `(blake3(source_content), model_iri)` — 32-byte hash + IRI bytes.
- **Value**: vector bytes + dimensionality + the trace IRI of the embedding call that produced it.
- **Storage**: RocksDB in a dedicated column family `cf_embed_cache`, separate from `cf_text`, `cf_vec`, and `cf_default`.
- **Lifecycle**: independent of layers and traces. The cache survives kernel restarts and layer GC; entries are evicted by an LRU policy with a configurable size budget (§5.9).

The cache is intentionally orthogonal to the layer chain. Same content + same model = same vector, regardless of which layer the content appears in. Cache hits are observable through the Embedder Component's trace (a cache-hit invocation records the served `(content_hash, model_iri)` and the source trace IRI of the original embedding); this is the audit path for "did this vector actually go through the model, or did we reuse?".

### 5.4 Query-side: inline Component dispatch within EigenQL

`EMBED("literal", model: ?m)` is an inline Component invocation within an EigenQL expression. The kernel's evaluator treats it the same way it treats any Component call in expression position — typechecked at parse, dispatched through the orchestrator at evaluation.

**Type-checking.**

- `EMBED("text", model: ?m)` has type `Vector(model: ?m, dim: ?d)` where `?d` is determined by the model.
- If `model:` is omitted, the typechecker infers it from the corresponding `VECTOR_NEAR`'s vector property (whose active VectorIndex Resource's `model_iri` is the only valid match). `MATCH X(?x) WHERE VECTOR_NEAR(?x.embedding, EMBED("text"))` infers the model from the active VectorIndex targeting `?x.embedding`.
- `VECTOR_NEAR(?v, EMBED("text", model: ?m1))` with `?v.model_iri != ?m1` fails typecheck. Mismatched-model queries cannot run.

**Evaluation.**

- The kernel evaluator encounters `EMBED(...)` during query plan execution.
- It dispatches the corresponding Embedder Component via the D6 IO-callback envelope — the same envelope every IO Component uses. No new RPC, no pre-pass.
- Content-addressed memoisation makes repeated `EMBED("text", model: M)` within the same query, the same session, or across sessions a cache hit.
- Multiple distinct `EMBED` calls in one query dispatch serially by default. A planner optimisation (§6) hoists all `EMBED` calls into a parallel batch before the structural query runs, eliminating round-trip stalls when many embeds are needed.

**Failure.** A query that requires `EMBED` and the embedder is unreachable returns the standard Component-failure error from D6. The query fails at evaluation; no partial results from already-resolved structural matches are returned. Treat embedder availability as a query precondition.

This is the resolution to the open question from §8 of the first draft ("Where does `EMBED("query string")` evaluate?"). The answer is: through the same D6 component-dispatch envelope every IO Component uses. The kernel doesn't host model runtimes, doesn't pre-resolve embeddings into a separate phase, doesn't grow a new RPC. The orchestrator services the IO; the kernel sees a vector value at the point `EMBED` is needed.

### 5.5 Indexing-side: post-Load sweep

When a layer L commits with new Resources whose properties are vector-indexed (i.e., targeted by an active VectorIndex Resource discovered at the commit head), the deterministic index entries (triple, text postings, per-layer shadowing bloom) commit atomically per §2.5. The vector segments `vec_seg:<I>:<L>` and their reverse-index entries `vec_layer:<L>:<I>` (one pair per active VectorIndex `I`) do not — they are produced asynchronously by a sweep.

Mechanism:

1. **Triggering.** Layer commit emits a sweep task (per D21 task traces) targeting `(L, I)` for each active VectorIndex `I` whose target Property received new content in L. Per-VectorIndex `embedding_policy: eager_on_load | lazy_on_query | manual` configuration chooses the trigger; v1 ships `eager_on_load` as the only supported value, the other two as scope markers (§5.9).
2. **Execution.** The sweep enumerates `(subject, content)` pairs for property P in layer L, dispatches the Embedder Component for each (batched up to an orchestrator-wide in-flight limit), collects results. Cache hits short-circuit the model call.
3. **Materialisation.** Once all vectors for `(L, I)` are computed, the sweep writes a single atomic `WriteBatch`: the `vec_seg:<I>:<L>` CBOR blob (per §2.4 layout) and the corresponding `vec_layer:<L>:<I>` reverse-index entry.
4. **Trace.** The sweep records a trace Resource (per D6b) summarising the materialisation: subject count, model IRI, cache-hit ratio, wall time, failure counts.

While the sweep is in flight, vector queries at any head visible through L see no contribution from L for property P. Structural and text queries are unaffected. This is operational state, not a correctness problem — `MATCH VectorIndexCoverage(?c) { layer: ?L, property: ?P }` returns sweep-status records so users (and agents) can see which `(layer, property)` pairs are fully indexed and which are pending.

Sweep concurrency, retry policy, and chunking are tunables. v1: per-orchestrator in-flight Embedder-call limit (default ~64), exponential-backoff retry on transient failures, one materialisation unit per `(layer, VectorIndex)`. Splitting one `(L, I)` into multiple `vec_seg:<I>:<L>:<chunk>` blobs is deferred until segment sizes exceed memory comfort.

Coarse layer cadence (D35 §9, D43 §8) compounds well: per-PR cadence means sweeps run at PR-merge time rather than continuously; the catch-up window is bounded.

Layer-delete interaction: `delete_layer(L)` cancels any in-flight sweep targeting L via the D21 task-cancel surface. A cancelled sweep leaves no partial state because materialisation is a single atomic write — either `vec_seg:<I>:<L>` exists in full or it doesn't exist at all.

### 5.6 Atomic-commit refinement (revising §2.5)

§2.5 stated that all index entries for a layer become visible at the same moment as `topo:<layer_id>`. With §5.5's sweep model, the precise invariant is:

> Every *deterministic* index entry for layer L commits atomically with L's topology record. The *IO-dependent* index entries — vector segments (`vec_seg:<I>:<L>`) and their reverse-index entries (`vec_layer:<L>:<I>`) — commit atomically per `(layer, VectorIndex)` pair in a separate `WriteBatch` produced by the post-Load sweep. The layer is queryable structurally and via text from the moment its topology record commits; vector queries return progressively as sweep `WriteBatch`es land.

The relaxation is narrowly scoped: vector segments are the *only* index data with this asynchronous-commit pattern, and they have it because their derivation requires an IO call that cannot be made reliable at Load time. Forcing atomicity here would gate Load on embedder availability — a structurally worse outcome than partial vector visibility during a sweep window.

The shadow check (§2.6) and the chain-walk's key-existence skip (§2.3 / §2.4 — the Phase 14h pattern where key absent = no contribution) are unaffected: both operate on whatever `vec_seg` entries are currently committed. They are correct under partial materialisation because a layer with no `vec_seg:<I>:<L>` simply doesn't appear in the prefix scan and contributes nothing to a vector query — which is the right answer.

### 5.7 Model identity and provenance

Model identity is the `model_iri` field, present on every `vec_seg` segment header (§2.4) and every embedding-cache entry (§5.3). Two model IRIs are distinct identities; vector queries cannot cross them, full stop.

**Versioned IRIs are required for any model whose outputs aren't stable.** Hosted APIs that silently change their model under the same nominal identity (a real failure mode for `openai:text-embedding-3-large` over a multi-year horizon) must be pinned by including a version suffix (`urn:eigenius:embed:openai:text-embedding-3-large:v3`). The user is responsible for pinning discipline; the platform cannot detect a silent vendor upgrade.

**Model upgrade requires full reindex.** Updating a VectorIndex Resource's `model` (or any other parameter that affects segment contents — `dimensionality`, `distance`, `hnsw_params`) is an atomic, full-reindex operation. Mechanically, the update is expressed as a new VectorIndex Resource declared in a new layer; the new Resource has a fresh IRI (call it `I_v2`) and shadows the old `I_v1` for the target Property. The reindex sweep then:

1. Enumerates all visible subjects with the target property across the chain.
2. Re-embeds each subject's value under `I_v2`'s model via the corresponding Embedder Component. Hits the same content-addressed cache (§5.3), keyed by the new model IRI — so cache hits only occur if some other VectorIndex had already used the new model against the same content.
3. Builds new `vec_seg:<I_v2>:<L>` entries for every layer `L` that contributed under `I_v1`.
4. Commits the new VectorIndex Resource and all new `vec_seg:<I_v2>:<L>` / `vec_layer:<L>:<I_v2>` entries in atomic `WriteBatch`es. The old `vec_seg:<I_v1>:*` and `vec_layer:<*>:<I_v1>` entries are not immediately deleted: they remain in storage and remain queryable from any chain head that still references `I_v1`, and become GC-reclaimable once no head does.

While the reindex is in flight, queries against the queryer's head see the state that's currently active there (either `I_v1` for branches that haven't yet adopted the new Index Resource, or `I_v2` once the new Index Resource becomes visible). At any single head, only one VectorIndex per target Property is active, so "pin-to-model" query semantics, hybrid-model queries, and the typing complexity they would carry simply don't arise.

The reindex is structurally similar to a consolidation event in that it touches many layers' index entries; it differs from D25 chain consolidation in that the layer topology doesn't change and it operates per `(layer, VectorIndex)` rather than collapsing layer ranges. Implementable on top of D25's atomic-multi-layer-write machinery without changes to D25 itself.

A consequence: D43 §2.8's previous "refuse to consolidate across an embedding-model upgrade" guard is removed. Mixed-model chain history per Index cannot exist — each VectorIndex Resource is model-homogeneous by construction — so there's no state to guard against.

**Provenance.** Embeddings are values, not chain Resources. The §5-stub's `VectorEmbedding` Resource shape is demoted to an internal value type used by the cache and the segment encoder. The provenance trail for any single vector is recoverable from the embedder Component's reasoning trace (D6b) keyed by the trace IRI stored in the embedding-cache entry. Queries that ask "what model produced this vector at what time" walk the trace; queries that ask "is this vector valid under the current head" check the `model_iri` in the segment header. There is no chain Resource per embedding.

### 5.8 Failure modes

**Embedder unreachable, indexing-side.** The sweep retries with exponential backoff per D6's IO-failure policy. The layer remains committed and structurally queryable. The corresponding `vec_seg:<I>:<L>` entry never appears until the embedder is reachable. Observable through the sweep task's status (D21): `Failed | Retrying | Succeeded`. The layer is not "broken"; it has partial index coverage that surfaces through the same query the user uses to check sweep progress.

**Embedder unreachable, query-side.** The `EMBED(...)` Component call fails per D6 IO-failure handling. The query fails at evaluation. No partial results are returned. Agents whose workflow can tolerate degraded retrieval should issue an embed-then-query pattern that tolerates the failure, or pre-warm the cache.

**Embedder produces an unexpected vector shape** (wrong dimensionality, wrong dtype). Caught by the Component output type-check (D3); the call fails with a typed error rather than silently writing garbage into a segment. The sweep treats this as a retry-after-cooldown condition; after N retries, the sweep marks the property as `Failed` and stops. The Component is presumed misconfigured upstream and needs human intervention.

### 5.9 Open questions (§5-specific)

- **Embedding-cache eviction policy.** LRU with a size budget is the v1 default, but evicting a hot embedding is expensive (every subsequent query that needs it re-pays the model cost). Alternatives: never evict (size-unbounded, possibly an operational issue at scale); evict only on explicit policy (`evict_unused_for: Duration`); piggyback on trace pruning (D9 §pruning). Revisit if production workloads show high evict-then-refetch churn.
- **Sweep concurrency limit.** A burst of large layers can saturate the orchestrator's outbound bandwidth to the embedder. Per-Embedder-Component throttling is plausible but introduces new configuration surface. v1: orchestrator-global in-flight cap. Per-Component overrides deferred.
- **`embedding_policy` value set.** v1 ships `eager_on_load` only; `lazy_on_query` and `manual` are scope markers for a future revision when workload diversity warrants them. The configuration slot is reserved on `core:VectorIndex` now so the surface (§3) doesn't have to grow later.
- **Model reindex UX.** §5.7 specifies model upgrade as an atomic reindex. The UX questions (how the reindex is triggered — CLI / API / notebook action; how progress is surfaced via D21 task status; whether to expose a cost estimate before commit; what canary / dry-run options exist) are §3 / §4 surface-spec concerns. The atomicity property simplifies the UX: there is no "transition window" the user needs to manage; the reindex either succeeds atomically or doesn't change the chain at all.
- **`EMBED` planner-side batching.** §5.4 commits to the in-evaluator dispatch envelope as the v1 semantics. The optimisation that hoists all `EMBED` calls in a query into a parallel pre-pass is real but lives properly in §6 (planner). Confirming the surface is unchanged by the optimisation (the query author never sees a different result, just a faster one) before committing.

## 6. Planner integration

The EigenQL planner produces an execution plan from a typechecked query. The user-visible retrieval surface is a single similarity operator (§3.3); the planner expands each `~` operator into the platform-internal probe + fusion machinery that this section specifies. Index probes replace full chain scans, the `TOP K` clause and the optional `limit:` hint push K-bounds into per-segment probes, similarity-operator RHS strings hoist into a parallel embedding pre-pass for vector-indexed properties, and rank fusion across multiple sources runs internally without a user-visible function call.

This section specifies how the planner integrates with §§2–5. It does not redesign the existing EigenQL planner — D2's structural-query planning surface remains in place — but it specifies the additions and the cost-model adjustments retrieval requires.

### 6.1 Inputs from typecheck

The planner inherits from §4's typecheck the following bindings per query:

- The schema view at the query head, including `active_similarity_indexes(P, H)` for every property `P` referenced by a `~` operator.
- The concrete Index Resource IRIs in that lookup — one per active TextIndex / VectorIndex on `P` — bound at plan time and not re-resolved during execution.
- The validated hint block (§4.4) per operator, with strategy / model / k / limit fields ready to consume.
- Each `~` operator's source `Property` IRI (for probe dispatch) and its source `MATCH` pattern's subject variable (for chain-walk subject filtering).

The plan is bound to these IRIs at planning time; execution does not re-resolve them. A schema change after planning but before execution has no effect on an in-flight query.

### 6.2 Top-K pushdown

The `TOP K` clause and the optional `limit:` hint on a `~` operator both push K-bounds into per-segment probes. The planner combines them with the structural-selectivity heuristic (§6.6) to derive each probe's effective bound.

**`TOP K` clause pushdown.** When `TOP K` is present and a `~` operator drives the ranking, K (or `K * over_fetch_factor`) is propagated as the per-segment probe bound:

- **Text probe**: the per-layer text probe (§2.3 query path step 7) is bounded to emit at most `K * over_fetch_factor` hits per layer; the top-k merge across layers (step 9) yields the final K. Over-fetch covers structural filters that may reduce the survivor count below K.
- **Vector probe**: the per-segment vector probe (§2.4 query path step 6) is bounded to `K * over_fetch_factor`; HNSW dispatch uses `ef = max(K * 4, 64)` by default. Flat dispatch ignores `ef`.

**Per-operator `limit:` hint.** When a `~` operator carries `{ limit: N }`, that N replaces the planner-chosen probe bound for *that operator's* probes. Useful for recall-vs-precision tuning at the call site. Multiple `~` operators in the same query may carry independent `limit:` values; each applies to its own probes.

**Over-fetch factor.** Configurable per-query; default `4×`. Heuristic adjustment based on the estimated selectivity of structural filters in the same query (see §6.6): high structural selectivity → larger over-fetch; low selectivity → smaller.

**Interaction with multiple `~` operators.** When the query has both a `TOP K` clause and multiple similarity operators, K applies to the fused result; each contributing source is over-fetched independently (per §6.4) so fusion has enough candidates to produce a confident top-K.

### 6.3 Query-string embedding (pre-pass for vector-indexed `~`)

When a `~` operator targets a property with an active VectorIndex (text-only properties skip this step), the RHS string must be embedded under the property's declared model before the vector probe runs. The planner hoists every such embedding into a parallel pre-pass:

1. **Collection.** Walk the typed AST; for each `~` operator whose property has an active VectorIndex, collect the `(rhs_string, model_iri)` tuple. The model IRI comes from the active VectorIndex's declared `model`, or from the operator's `model:` hint if supplied.
2. **Cache probe.** For each distinct tuple, consult the §5.3 content-addressed embedding cache. Hits resolve immediately.
3. **Batched dispatch.** Tuples missing from the cache are sent in one batched Component-invocation request to the orchestrator. The orchestrator dispatches them concurrently subject to its per-Embedder concurrency limits (§5.5).
4. **Vector substitution.** Returned vectors are bound to their corresponding `~` operators internally — the user never sees them as bindings.
5. **Failure semantics.** If pre-pass dispatch fails (embedder unreachable or Component error), the query fails at evaluation per §5.8 — *before* the structural query begins. No partial work is performed.

A query whose every `~` operator targets text-only properties skips this step entirely. The pre-pass is the platform-internal counterpart to the user-visible `EMBED` call that earlier D43 drafts exposed; the mechanism is the same, but it never crosses the user surface.

### 6.4 Internal rank fusion (was: RRF rank materialisation)

When a query contains multiple `~` operators, or a single `~` operator whose property has multiple active similarity indexes (the hybrid case), the platform fuses per-source rankings via Reciprocal Rank Fusion. The fusion algorithm and parameters are platform-internal — no user-visible function call drives it — but the mechanics are the same as standard RRF.

The execution shape:

1. **Source identification.** Each `~` operator contributes one or more ranked sources. A text-only property contributes one (the TextIndex probe); a vector-only property contributes one (the VectorIndex probe); a hybrid property contributes two (text + vector). Multiple `~` operators on different properties contribute one per (operator, active-index) pair.
2. **Per-source over-fetch.** Each source over-fetches by `K * over_fetch_factor` candidates (default 4×, or the operator's `limit:` hint if set) — large enough that the final top-K from the fused ranking is unlikely to depend on candidates beyond the over-fetched window.
3. **Per-source rank materialisation.** Each source produces a ranked stream of `(row_iri, score)` pairs ordered by descending score. The platform assigns rank `j` to the `j`-th highest score per source (1-indexed; ties broken by row IRI for determinism).
4. **Row union and rank join.** Union the rows from all sources by row identity. For each row, record its per-source rank (or "missing" if the row didn't appear in that source's over-fetched window).
5. **Fused score computation.** For each row, compute `sum_i 1 / (k + rank_i)` treating missing-source rank as infinity (contributes 0). The default `k` is 60; the per-operator `k:` hint overrides it.
6. **Final top-K.** Sort by fused score; emit top K per the outer `TOP K`.

Rows that appear in *zero* sources are not in the result set. The result is therefore the union of the source result sets, ordered by per-source-rank fusion rather than per-source raw scores.

**Single-source short-circuit.** When a query has exactly one ranked source (one `~` operator on a single-index property, no `OR`-disjoined siblings), the platform skips fusion and ranks directly by the source's score. This is the common case.

### 6.5 Hybrid scheduling — text and vector probes in parallel

The canonical hybrid query under D43 v1: a single `~` operator against a property that carries both an active TextIndex and an active VectorIndex.

```eigenql
MATCH Doc(?d) { description: ?desc }
WHERE ?desc ~ "WAL truncation under concurrent commit"
TOP 20
```

One operator, one query string, two retrieval mechanisms running in parallel. The user expresses intent ("find Docs related to this"); the platform commits to running both probes and fusing them.

Assuming `description` has both an active TextIndex and an active VectorIndex, this plans as:

1. **Pre-pass.** Embed the RHS string `"WAL truncation under concurrent commit"` under the active VectorIndex's declared model (§6.3). One Component call; cached for subsequent identical queries. The text probe doesn't need a pre-pass — its analyzer-side tokenisation runs inline.
2. **Parallel index probes.** Schedule the two probes against `?desc` concurrently:
   - Text probe (BM25 over the tokenised RHS).
   - Vector probe (HNSW or flat against the embedded RHS).
   Each probe is bounded by `K * over_fetch_factor` (here `~80`) or the operator's `limit:` hint if supplied.
3. **Structural filter** (when present — here, the `?d` must be a `Doc`, which is checked above the index probes via the existing structural pattern).
4. **Per-source rank materialisation** (§6.4): each probe produces its own ranked candidate set.
5. **RRF fusion** per row, k=60 default. A row appearing in both probes' top-K gets contributions from each; a row appearing only in one gets a single contribution.
6. **Final TOP 20**, ties broken by row IRI.

Cardinality at each step:

| Step | Cardinality bound |
|---|---|
| Pre-pass | 1 embedding call (cached after first use) |
| Text probe | ≤ ~80 per visible layer |
| Vector probe | ≤ ~80 per visible segment |
| Per-source top-k merge | ≤ 80 (text) and ≤ 80 (vector) |
| Structural filter | ≤ ~160 |
| RRF | ≤ ~160 |
| Final TOP 20 | 20 |

The pipeline runs in tens of milliseconds typical, vs. seconds-to-minutes if the planner had materialised the full Doc set and computed scores row-by-row.

**Multi-operator hybrid.** When the user wants to retrieve against *multiple distinct queries* (the "find Docs related to either X or Y" shape), they compose with `OR`:

```eigenql
MATCH Doc(?d) { description: ?desc }
WHERE ?desc ~ "WAL truncation concurrent commit"
   OR ?desc ~ "rolling back a partial commit"
TOP 20
```

Two operators, two distinct RHS strings, two embedding calls. Each operator's probes run concurrently with every other operator's probes; fusion happens across all contributing sources. Cardinality scales linearly with the number of operators — two operators on a hybrid-indexed property produces four ranked sources (2 operators × 2 indexes each), all fused via RRF.

**The user-facing surface stays small in both cases.** Three lines for the single-operator case, four lines for the multi-operator OR. The planner's expansion handles probe scheduling, embedding pre-pass, fusion, and top-K truncation without forcing the user to write any of it by hand. That's the structural payoff of collapsing the surface to a single operator.

### 6.6 Structural-plus-retrieval scheduling

Queries mixing `~` operators with structural patterns (joins, additional `WHERE` clauses, class constraints) require the planner to decide the order. The general rule: push the most selective predicate down to the index level first.

Selectivity estimates:

- **Structural patterns** carry Phase 14h-style cardinality estimates (per the triple index `idx_pos:` stats).
- **`~` against a text index** has selectivity estimated from per-term document frequency: rare terms (low `df`) → high selectivity; common terms (high `df`) → low. The planner reads `df` values from the `text_term:<I>:<T>:<L>` value prefixes (§2.3) without deserialising the bitmaps.
- **`~` against a vector index** has cardinality exactly equal to the per-segment probe bound (the operator's `limit:` hint or the planner-derived `K * over_fetch_factor`) — bounded by construction.

Plan ordering heuristic: order predicates by ascending estimated cardinality, push the smallest to the index. Specifically:

- If a structural filter binds the subject to a narrow set (e.g., `?d.author = "alice"` resolves to ~10 docs via the triple index), the planner pushes it ahead of any `~` operator and applies the similarity probe only to the resulting subject set.
- If a `~` operator is highly selective (a rare-term query with `df = 100` against the text path), it goes first.
- Otherwise, `~` operators with bounded probe size (vector path always; text path under `TOP K`) provide bounded cardinality regardless of selectivity and run independently.

Over-fetch adjustment: when a structural filter is expected to reduce the retrieval result by a factor of `f`, the planner over-fetches by an additional factor of `f` to compensate. So a query with `?desc ~ "x"` AND `?d.author = "alice"` and `TOP 20`, where the author filter is estimated to drop 80% of similarity-passing rows, plans the similarity probes to over-fetch `~20 / 0.2 = 100` candidates per source.

### 6.7 D42 cost-model integration

D43's ranked retrieval produces bounded result sets. The D42 spill-aware cost model (when it ships) exploits this for downstream operators:

- After `TOP K`, the row count is at most `K`. Downstream `JOIN`, `ORDER BY`, `GROUP BY`, or aggregation operates on ≤ `K` rows — well within memory budgets for typical `K` (≤ 1000).
- A `~` operator with a `limit:` hint produces at most `limit × #sources` candidates per outer binding before fusion; the planner uses this as the row-count estimate for downstream operators.
- The cost model treats the output of similarity operators as having known bounded cardinality, so it does not insert spill-prep operations downstream of them by default.

The D42 buffer pool isn't typically engaged by D43-style queries unless the user constructs a non-ranked pipeline (e.g., gathering all `~`-passing rows without a `TOP K` clause and joining against millions of structural rows). In those cases, D42's existing operator-spill machinery applies; D43 imposes no new requirements on it.

### 6.8 Cost estimation stats

The planner reads the following stats at plan time:

| Stat | Source |
|---|---|
| Per-Index per-layer doc count | `text_stats:<I>:<L>.doc_count` (§2.3) |
| Per-Index per-layer avg doc length | `text_stats:<I>:<L>.avg_doc_length` (§2.3) |
| Per-term per-layer doc frequency | `text_term:<I>:<T>:<L>` value prefix (varint) (§2.3) |
| Per-VectorIndex per-layer vector count | `vec_seg:<I>:<L>` segment header `count` field (§2.4) |
| Per-VectorIndex strategy and HNSW parameters | Active VectorIndex Resource (§3.1) |
| Triple-index selectivity for structural patterns | Phase 14h `idx_pos:` stats |

Stats fetches are cheap — they're small RocksDB reads against keys the planner is already going to touch during execution. For cold-cache queries the planner may issue a prep-pass that pre-fetches stats in parallel with the EMBED pre-pass (§6.3) so neither blocks the other.

The planner does not maintain a separate global statistics catalog. All stats are derived live from the active Index Resources and the per-layer index entries; they reflect the chain state at the query head.

### 6.9 Cache-aware planning

The planner exploits five caches that D43 introduces or extends:

| Cache | Keyed by | Hit short-circuits |
|---|---|---|
| Embedding cache (§5.3) | `(content_hash, model_iri)` | EMBED Component dispatch |
| SegmentCache (§2.3, §2.4) | `(index_iri, layer)` for both vector and text | RocksDB blob fetch and decode |
| TermCache (§2.3) | `(index_iri, term, layer)` | Roaring bitmap deserialisation |
| DocsCache (§2.3) | `(index_iri, layer)` | `text_docs` CBOR parse |
| D23 ARC cache | resource IRI | structural-resource fetches in MATCH |

For batched workloads (notebook-driven exploration, agent-driven query bursts), the planner can issue a *cache prewarm* prep-pass for segments the upcoming query is likely to touch — admitting them via the SegmentCache before the query proper starts. This is opt-in via a planner hint; default behaviour is on-demand fetch.

### 6.10 Failure-mode boundaries

The planner's failure surface is small:

- **Typecheck failure** is already separated (§4.5) — the planner only receives well-typed plans.
- **Pre-pass embedding failure** (§5.8 query-side) — the platform-internal embedding of a `~` operator's RHS string for a vector-indexed property fails (embedder unreachable or Component error); the planner returns a query-failure error before the structural query begins. No partial work is performed.
- **Stats-fetch failure** (RocksDB read error during plan-time stats lookup) — the planner falls back to default selectivity estimates (treat all `~` operators as moderately selective). The query still executes; latency may be suboptimal but correctness is unaffected.
- **Plan-time Component registration changes** (an Embedder Component is deregistered between typecheck and execution) — D6's existing IO-Component-failure handling applies. Rare in practice.

Runtime-only failure conditions (per-segment model_iri verification, vector segment not materialised) are handled in execution per §§2.4 and 5.5, not planning. The planner produces correct plans; correctness of the plan's *result* under all observable cache and materialisation states is the execution-layer's responsibility.

## 7. Layer awareness of scores and shadowing

D43's retrieval primitives produce scored hits per `(subject, defining_layer)` pair. The chain walk surfaces hits across visible layers; the shadow check (§2.6) and the active-Index resolution (§3.1, §4.2) determine which hits survive into the result. This section consolidates the layer-awareness rules so they are stated explicitly in one place. Most of the substance is already established in §§2 and 5; §7 cross-references and articulates the layer-level semantics.

### 7.1 Score combination across layers — last-writer-wins, no aggregation

When the same subject appears in multiple segments — typically because the subject's value of an indexed property was redefined in a descendant layer — the shadow check (§2.6) drops all but the hit from the most-recent non-shadowed defining layer. The score from the surviving hit is the score returned; older hits' scores are discarded without aggregation, averaging, weighted combination, or recency adjustment.

Worked illustration. Suppose subject `S` is defined as follows:

- Layer `L1` defines `S.description = "WAL truncation in the kernel"`.
- Layer `L2`, a descendant of `L1`, redefines `S.description = "WAL behaviour during concurrent commit"`.

A query at head `L2`:

```eigenql
MATCH ?s { description: ?d }
WHERE TEXT_MATCH(?d, "WAL truncation")
RETURN ?s, TEXT_SCORE(?d, "WAL truncation") AS score
```

- L1's segment yields a high BM25 score for `S` (both query terms are an exact match in the old description).
- L2's segment yields a lower score (only "WAL" matches the new description).
- The shadow check drops the L1 hit because L2 redefines `S`.
- The returned score for `S` is L2's lower score — the score against the *current* state of `S` as of head `L2`.

The semantic this implements is "what is the current state of subject S as of head H?" — the surviving hit is the answer, its score is the answer's relevance. The older version's higher score belongs to a subject-version that no longer exists at the query head. This differs from search systems that index all historical document versions and aggregate across them; D43 indexes per-layer-defining contributions and resolves to the latest non-shadowed.

### 7.2 Per-head active Index — divergent indexing is queryable independently

Each chain head sees one active TextIndex and one active VectorIndex per indexed Property (§3.1 v1 multiplicity constraint). The per-Index segment keying (§2.3, §2.4) ensures that different branches with different active Index Resources have separately-addressable segments; queries from each head probe their own active Index without leaking across branches.

The §5.7 atomic-reindex policy is what makes this work. Switching a target Property to a new model (or any other parameter that affects segment contents) is expressed as a fresh VectorIndex Resource with a new IRI, shadowing the prior Resource. Each head observes exactly one active Resource for the (Property, kind) at any time; there is no "pin-to-model" query semantics, no hybrid-model queries, and no run-time model selection — the model in use at any head is unambiguous and provably model-homogeneous per active Resource.

The §4.2 schema view formalises this for the typechecker: `active_text_index(P, H)` and `active_vector_index(P, H)` are the per-head lookups. The same head dependency carries through to the query path (§2.3, §2.4) — segments probed at query time are exactly those keyed under the head's active Index Resource.

### 7.3 Time-travel — queries at historical heads see historical schema and historical indexes

EigenQL supports at-layer reads (per D6): a query can specify a historical layer as its head rather than the current branch tip. The chain walk visits only ancestors of the specified head; segments contributed by later layers are invisible. The active-Index lookup (§4.2) is rooted at the queried head, so an Index Resource declared *after* that head doesn't exist for the query, and its segments don't appear in the prefix scans.

This is standard D23 at-layer-read behaviour — D43 adds no new mechanism. The point worth stating: retrieval correctness composes naturally with time-travel. A query at head `H_past` sees the schema as of `H_past`, the active Index Resources as of `H_past`, the segments as of `H_past`, and the BM25 chain-aware IDF computed over the layers visible from `H_past`. There is no "show me the index as if today's analyzer were in use" mode; that would require re-running the indexing pipeline against `H_past`'s resources with today's Index parameters, which is structurally a §5.7 reindex against a specific head — not a query-time operation.

### 7.4 Chain-aware scoring within a single active Index

Within a single active Index Resource, scores from different layers are comparable.

**Text (BM25).** IDF is computed chain-aware across visible layers (§2.3 query path step 6). `N` (total document count) sums across all visible layers contributing under the active TextIndex; `df(t)` for each query term sums across the same set. Scores from all surviving hits — regardless of defining layer — share the same IDF baseline and the same average document length, so direct top-k merge across layers is sound.

**Vector.** Distance is computed per row against the same query vector. The distance metric is fixed by the active VectorIndex's `distance` configuration and is uniform across all its segments. No normalisation across layers is required.

In both cases, the top-k merge (§2.3 query path step 9; §2.4 query path step 8) treats each surviving hit independently. No score adjustment, normalisation, or recombination is applied across layers — chain-aware IDF (text) and uniform distance metric (vector) make the raw merge sound.

### 7.5 Cross-Index scoring is platform-internal

Scores from different Index Resources — a TextIndex and a VectorIndex on the same Property, two TextIndexes targeting different Properties, or any combination — are *not* commensurate. BM25 scores depend on the analyzer and the chain's document distribution; cosine similarity depends on the embedding model and is bounded differently from BM25. Direct score arithmetic across them is structurally meaningless.

D43 v1 solves this by *not surfacing raw scores at all*. The `~` operator's output is a rank-derived ordering; the platform fuses across sources via Reciprocal Rank Fusion (§6.4) internally, with no user-visible score column or fusion call. Each source's ranking is computed independently; ranks are combined via the rank-fusion formula; the user gets a single ordering as the operator's output.

This eliminates a class of footguns the earlier D43 draft's score-projection surface could create: users summing BM25 and cosine scores, or building custom weighted combinations across mismatched scales. With scores hidden, those mistakes aren't expressible. The diagnostic surface deferred per §3.7 is where per-source attribution becomes inspectable when debugging is needed.

## 8. Open questions

- ~~**Tantivy as the text backend, or a custom layer-aware inverted index?**~~ **Resolved in §2.3.** Custom layer-aware inverted index. Two structural arguments closed the question: (a) Phase 14h's key-ordering pattern (`<property>:<term>:<layer>` with layer-as-suffix) lets the chain walk reuse the kernel's existing chain-membership-filter machinery, where Tantivy's segment model would have required a parallel segment-per-layer machinery; (b) Tantivy's per-segment BM25 IDF is a transient compromise in Tantivy's design (segments merge) but would be permanent in ours (layer segments don't merge), so the chain-aware IDF in §2.3's query path is structurally better than what Tantivy provides. Dependency-weight argument seals it. v1 deferrals — phrase queries, fuzzy, multi-language — are extensible from the schema without breaking changes.
- ~~**Layer cadence and per-layer segment count.**~~ **Deferred to [D44 — Automatic Data Lifecycle Management](d44-automatic-data-lifecycle-management.md) (stub).** D43 places no requirement on commit cadence. When chain density becomes a query-performance problem, D25 chain consolidation is the mechanism for collapsing dense ranges; §2.8 commits to re-extracting text postings and concatenating vector segments correctly during consolidation. The *policy* for when consolidation should fire — and the parallel question of when garbage collection should fire — is automatic-data-lifecycle-management territory and is reserved as D44 (deliberately a stub pending production-observation-driven scoping). D43's commitment ends at "retrieval indexes consolidate correctly when D25 triggers"; the trigger policy itself is out of scope.
- ~~**Column-family budget tuning.**~~ **Resolved.** v1 ships equal-weight defaults for `cf_text`, `cf_vec`, `cf_embed_cache`, and `cf_default`, with per-CF tunables exposed in the kernel config (memtable size, block-cache share, write-buffer count). Revisit when production workload profiles are available; the per-CF tunables make tuning a config change, not a schema change.
- ~~**HNSW migration path from v1 flat-segment storage.**~~ **Resolved in §2.4.** HNSW ships in v1, not deferred. Per-VectorIndex `strategy: flat | hnsw | auto` configuration drives the choice; segments coexist transparently because the reader dispatches on the optional `hnsw_graph` CBOR field. Life-science ontologies (UMLS, SwissProt, SNOMED) and bounded PubMed extractions need HNSW for usable latency; the SE knowledge graph doesn't. Strategy-switching gives each workload the right algorithm. v2 work (int8 quantisation, IVF, out-of-core HNSW) is triggered when segments exceed ~10M vectors with measured pressure; the §2.4 CBOR layout is forward-compatible to those extensions as additional optional fields.
- ~~**Hot-path per-property bloom storage.**~~ **Resolved in §2.6.** No per-`(layer, property)` bloom in v1. The Phase 14h key ordering adopted by §2.3 (text) and §2.4 (vector) makes "does this layer contribute to property P / term T" a cheap key-existence check via prefix iteration; the bloom would be redundant. The per-layer shadowing bloom from D23 §5.2 remains the only bloom in play, and it serves the subject-shadow purpose unchanged.
- ~~**Cross-property scoring under multi-field text queries.**~~ **Resolved.** v1 ships single-field `TEXT_MATCH(?prop, "query")`. Multi-field text queries with per-field boosts are a §3 surface extension that the §2.3 schema already supports without change — the per-`(property, term, layer)` keying lets the planner issue independent per-field probes and combine scores above the index. Adding the surface later is additive; no schema migration required.
- ~~**Embedding-model upgrades during a chain's life.**~~ **Resolved in §5.7.** Model upgrades require a full atomic reindex expressed as a fresh VectorIndex Resource (shadowing the prior Resource for the target Property). The new Resource gets its own IRI, so its segments live under fresh `vec_seg:<I_v2>:*` keys; the prior Resource's segments stay queryable from any chain head that still references them and become GC-reclaimable once no head does. Each VectorIndex Resource is model-homogeneous by construction. This eliminates pin-to-model query semantics, hybrid-model queries, and §2.8's earlier consolidation refusal across upgrades.
- ~~**Where does `EMBED("query string")` evaluate?**~~ **Resolved in §5.4.** `EMBED` is an inline Component invocation in EigenQL expression position; the kernel typechecks it at parse and dispatches via the existing D6 IO-callback envelope at evaluation. No new RPC, no pre-pass, no kernel-resident model runtime; the same Embedder Component the indexing-side sweep uses, against the same content-addressed cache.

---

*D43 v1 design complete. All sections §§2–7 carry substantive design; §1 and §8 frame the scope and record the resolution trail. The §8 and §5.9 open questions have all been resolved (decisions recorded inline as strikethrough + resolution notes). The proposal as it stands is what should be implemented; subsequent revisions track from this baseline.*
