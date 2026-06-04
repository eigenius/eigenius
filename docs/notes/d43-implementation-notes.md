# D43 implementation notes

Non-obvious decisions captured during the D43 v1 implementation (June 2026). Each section names the decision, the alternatives we considered, and the reason the shipped choice won. Read this before changing any of the structures it discusses — the trade-offs are not always self-evident from the code.

Design: [d43-text-and-vector-retrieval.md](../design/d43-text-and-vector-retrieval.md). Plan: [d43-implementation-plan.md](../design/d43-implementation-plan.md). User guide: [eigenql/06-text-and-vector-retrieval.md](../guides/eigenql/06-text-and-vector-retrieval.md).

## The surface reset

The original M1 plan modeled retrieval after the D35 §7.4 worked example: seven primitives (`TEXT_MATCH` / `TEXT_SCORE` / `VECTOR_NEAR` / `VECTOR_SIM` / `EMBED` / `RRF` / `TOP K BY <expr>`) plus a `BIND(expr AS ?var)` clause to name per-row scores. Six surface concepts the user has to learn before asking "find related things." Half-way through M7 implementation the surface was abandoned and collapsed to a single `~` operator with a `{ via, model, k, limit }` hint block and a bare `TOP N`.

Why the reset won:

1. **The user wasn't going to think in those concepts.** Nobody asks an agent "give me text-rank fused with cosine-rank where text-rank = BM25 of WAL truncation." They ask "find code about WAL truncation." The SQL-shaped surface forced the user to be the planner.

2. **Strategy is a schema decision, not a query decision.** Whether retrieval uses BM25, cosine, or a fused score depends on which indexes the schema owner declared. Forcing the query writer to name `TEXT_MATCH` vs `VECTOR_NEAR` puts the schema choice in the wrong place and locks queries to one source.

3. **The embedder, fusion algorithm, and per-source scores are implementation details.** Exposing them creates compatibility surface that's hard to evolve. The pre-reset surface would have made it a breaking change to switch fusion algorithms or to add a new probe source.

4. **BIND was already a wedge.** It existed only because the SQL surface forced score-naming for the BY clause. Once `TOP N` ranks by the platform-internal fused score, the user never needs to name a score, BIND has no users, D45 is withdrawn.

Cost of the reset: ~3 days of in-progress code deleted (lexer tokens, parser arms, evaluator dispatch, BIND/TOP-K-BY tests). The replacement surface shipped in M7 in roughly the same calendar time as the abandoned surface would have. The structural lesson: **surface decisions cost more later than they cost now**; reset them while the work is in progress, not after.

Anchored in [d43-implementation-plan.md M7 surface-reset note](../design/d43-implementation-plan.md#m7--similarity-operator--hybrid-retrieval).

## Pointer-keyed `SimilarityContext`

The evaluator runs a pre-pass that probes every active index referenced by a `~` operator once per query and caches the result. Per-row evaluation has to map an AST `Similarity` node back to its precomputed probe.

Options considered:

- **Parallel index walks**: at pre-pass time number each Similarity node in DFS order; at eval time do the same walk and use the index. Fragile against shared AST nodes (none in v1, but any future refactor that interns expressions breaks it silently).
- **Mutable node tag**: add an `Option<u64>` slot to `Expression::Similarity` that the pre-pass populates. Pollutes the AST with evaluator state; complicates equality / hashing.
- **Pointer identity** (`*const Expression as usize` used as map key): no AST changes, O(1) lookup, exact identity. The AST is owned by `Program` and borrowed through evaluation, so `&Expression` pointers stay stable for the evaluation's lifetime. Shipped.

The risk is real but bounded: a future refactor that boxes / re-allocates Expression nodes during evaluation breaks the lookup. Mitigation is the [`SimilarityContext`](../../kernel/src/query/evaluate/similarity.rs) module docstring explicitly stating the lifetime constraint and the per-row error message that fires if a probe isn't found ("similarity operator `~` not registered in the pre-pass context") — loud failure mode rather than silent miss.

## RRF with k=60

[`fuse_rrf`](../../kernel/src/query/evaluate/similarity.rs) implements `score(row) = sum_i 1 / (k + rank_i(row))` with k=60 default (Cormack-Clarke-Buettcher 2009). The constant is publicly documented in D43 §3.5 and exposed via the `k:` hint.

Why k=60 specifically: published literature shows 60 is the empirically robust default across heterogeneous source distributions; smaller k weights rank-1 too heavily (the fused score collapses toward the top-ranked source's choice), larger k flattens the distribution to where rank position barely matters. The hint exists so a query can tune for its own data without users having to fork a kernel build.

## OBO IRI rewriting

OBO ontologies use HTTP IRIs (`http://purl.obolibrary.org/obo/GO_0005634`) as opaque identifiers. The Eigenius convention is `urn:` (CLAUDE.md: "IRIs use the urn: scheme — `urn:eigenius:<namespace>:<local-name>`"). The obograph converter ([`crates/eigenius-obograph/src/convert.rs::rewrite_iri`](../../crates/eigenius-obograph/src/convert.rs)) maps HTTP IRIs to URNs uniformly.

Mapping rules:

- `http://purl.obolibrary.org/obo/<PREFIX>_<LOCAL>` → `urn:obo:<PREFIX>:<LOCAL>` — preserves the canonical OBO CURIE shape biologists already use (`GO:0005634` reads the same).
- `http://purl.obolibrary.org/obo/<PREFIX>#<frag>` → `urn:obo:<PREFIX>:<frag>` — covers intra-ontology subsets / synonym types.
- `http://www.geneontology.org/formats/oboInOwl#X` → `urn:obo:oboInOwl:X` — OBO's RDF schema annotations.
- `http://www.w3.org/2000/01/rdf-schema#X` → `urn:rdfs:X` and `http://www.w3.org/2002/07/owl#X` → `urn:owl:X` — these are *not* under `urn:obo:` because RDFS and OWL aren't OBO-specific vocabularies. Future kernel revisions may want first-class support for `urn:rdfs:label` (synonyms-with-language-tag) without involving the OBO namespace.

Provenance under `urn:eigenius:core:source_irl`: every rewritten Resource preserves its original HTTP IRI as a string slot so downstream consumers can join with external OBO data that still uses HTTP form. The slot was already declared in core ontology (recommended on every Resource) — we didn't have to invent it.

What we *didn't* do: a fully bidirectional rewrite. The converter only goes HTTP → URN. The reverse (querying URN-converted Eigon for external HTTP-keyed data) needs a separate join step that uses `source_irl`. That's a conscious limitation, not an omission — supporting bidirectional rewrite means tracking provenance on the *fields* (which got rewritten, which didn't), which doubles the converter complexity for a use case no consumer has asked for yet.

## The obo-meta layer

The four OBO synonym Properties (`urn:obo:has_exact_synonym`, `has_related_synonym`, `has_broad_synonym`, `has_narrow_synonym`) and the `urn:obo:inverseOf` axiom are a fixed OBO Foundry vocabulary — they don't change between GO and ChEBI. Initially the converter synthesised inline declarations for every used `urn:obo:*` slot per imported document. The result was 4 redundant Property declarations per imported ontology, all identical, all shadowing each other.

The fix: [`ontologies/obo/obo-meta-ontology.json`](../../ontologies/obo/obo-meta-ontology.json) declares them once, the bootstrap chain loads it as a parent layer ahead of any import, the converter's `META_DECLARED_IRIS` constant lists them as "skip per-document synthesis." Ad-hoc `urn:obo:*` IRIs the converter discovers in real data still get per-document declarations (proof: the test `synthetic_property_declarations_emitted_for_ad_hoc_urn_obo_slots` uses `hasAlternativeNamespace`, an IRI not in the meta layer).

The structural lesson: **shared third-party vocabularies belong in a parent layer, not in every imported document**. The same pattern would apply to other ontology families if we add support for them (Wikidata's property namespace, Schema.org's, etc.) — each gets its own meta layer between core and the import.

What got added to core ontology in the process: `urn:eigenius:core:Resource` (catch-all super-class for entities whose specific class isn't known — `INDIVIDUAL` nodes, anonymous edge targets) and `urn:eigenius:core:deprecated` (Boolean Property for the OBO `meta.deprecated` slot). Both were referenced by the converter before being declared anywhere; the kernel was tolerant enough not to fail but the validator couldn't chase the references. Adding the declarations costs nothing and closes the validator gap.

## DeclaredResource tagging on imported data

Every Resource the converter emits gets `is_a` extended with `urn:eigenius:reflection:DeclaredResource` and a `declared_by` slot pointing at the source graph IRI. Imports represent **declared knowledge** (asserted by an external curating authority — the GO Consortium, ChEBI curators) rather than derived knowledge, so the epistemic tagging matters for queries that filter on provenance.

Two attribution paths:

1. **Source-graph attribution** for nodes/edges that came from the input ontology: `declared_by: <graph.id>` (e.g. `"http://purl.obolibrary.org/obo/go.owl"`). Override via the CLI `--declared-by` flag — useful for ingesting curated subsets where the graph IRI doesn't unambiguously identify the authority.
2. **Converter attribution** for synthesised Property declarations (the ad-hoc `urn:obo:*` ones that aren't in obo-meta): `declared_by: "urn:obo:converter:eigenius-obograph"`. Distinct constant so downstream auditors can tell "this Property was inferred by the importer" apart from "this Property was declared by the source curators."

Splitting the attributions was an explicit design decision after considering "just attribute everything to the source graph" — that would over-claim authority and make the converter's inference opaque. The split is honest: the synonym values came from GO curators; the *Property declaration that says synonyms are a string-array slot* came from the converter.

## TOP-before-RETURN sort ordering

The evaluator's clause-order pipeline is roughly: pattern matching → GROUP BY → **TOP** → RETURN shaping → DISTINCT → ORDER BY → OFFSET → LIMIT. The TOP step runs *before* RETURN shaping, not at the conventional "ORDER BY / LIMIT" position.

Why: TOP sorts by per-binding similarity score. The score lookup needs the binding's subject IRI to probe each `~` operator's score map. After RETURN shaping the binding-to-resource projection has dropped the subject — the row Resource carries projected slots, not the original binding. Sorting after shaping would require either re-materialising bindings (expensive, error-prone) or threading the subject IRI through shaped resources as an extra slot (changes the result-format Appendix A shape).

Sorting before shaping is the cleanest fix: bindings still have everything the score lookup needs, the truncation happens before the per-row projection cost, and RETURN shaping only runs N times instead of |total candidates|. Implementation: [`evaluate/mod.rs`](../../kernel/src/query/evaluate/mod.rs) — the `bindings.sort_by` + `bindings.truncate(n)` block sits between GROUP BY and the shape loop.

The trade-off: TOP can't mix with user-supplied ORDER BY. That's exposed as a typecheck rule (`top_with_order_by`) rather than a runtime surprise. Users who want a secondary order on the TOP-truncated set need to either use LIMIT instead (un-ranked) or do the rank-then-sort in a downstream wrapper.

## v1 multiplicity: one TextIndex + one VectorIndex per Property

[`verify_text_index_multiplicity`](../../kernel/src/layer/index_discovery.rs) and `verify_vector_index_multiplicity` enforce: at most one active TextIndex and at most one active VectorIndex *per target Property* per head. Both can coexist on the same Property — that's the hybrid case.

Why not multiple of either: the planner becomes harder to predict, the fusion math needs to weight per-source-type contributions, and the user query has no way to address a specific TextIndex when multiple are active. The constraint is also recoverable — users can declare a *different* Property if they need a parallel TextIndex with a different analyzer (e.g. one for tokenised body, one for stemmed body) and a join in the query.

Forward-compatible: the multiplicity check is a verification function on the resolved active set, not a structural restriction on what can be committed. A future revision can relax it (add a primary-vs-secondary distinction, or per-Index addressing) without re-shaping the storage layer.

## Reindex registry split from sweep registry

[`SweepRegistry`](../../kernel/src/task/sweep_registry.rs) carries two parallel maps: `sweeps` (keyed by `LayerId`) and `reindexes` (keyed by VectorIndex IRI). Initially I considered a single map; the keys disagree on shape because the tasks disagree on scope.

A sweep covers every active VectorIndex at a layer in one driver call — the layer's id is the natural unit, multiple Indexes covered by one handle.

A reindex (D43 §5.7 model upgrade) walks the entire chain, not one layer. Several reindexes against different target Indexes can be in flight concurrently against the same head — the target-Index IRI is the natural unit.

Sharing one map would either (a) require synthetic composite keys, (b) lose the "multiple concurrent reindexes against one head" affordance, or (c) introduce per-key disambiguation (sweep vs reindex prefix). All worse than two purpose-built maps.

Cancellation propagates the same way through both: `cancel_by_layer(L)` flips the sweep flag for layer L; `cancel_reindex(I)` flips the reindex flag for Index I. The `delete_layer(L)` hook would call both before proceeding to GC.

## `~` at relational precedence, not unary

The `~` operator sits at relational precedence (alongside `<`, `>`, `IN`, `LIKE`) rather than at primary or unary. Three places this matters:

- `?a ~ "x" AND ?b ~ "y"` parses without parentheses (AND sits looser than relational).
- `?a ~ "x" OR ?b ~ "y"` likewise.
- `NOT ?a ~ "x"` parses as `NOT (?a ~ "x")` (NOT is unary, sits tighter).

Considered alternatives: unary `~ "string"` operating implicitly on the surrounding context (rejected — pulls context out of the AST, fragile under refactor), function-call `similar(?a, "x")` (rejected — falls back into the abandoned SQL-shaped surface), inline-method `?a.similar_to("x")` (rejected — no other EigenQL surface uses method syntax, would force a parser extension).

The relational-precedence binary slot was the natural fit and required only a `Tilde` token + one continuation branch in `parse_relational_expr`.

## `core:resource` data_type for OBO object properties

OBO OBJECT properties (e.g. `BFO_0000050` = part_of) carry an IRI value. The converter emits them with `data_type: core:resource` and stores the value as `Value::Array` of `ResourceRef` so multiple part-of relationships accumulate cleanly on one subject.

What we did *not* emit: `core:resource_array` data_type with `element_type: core:resource`. The cleaner shape, but it requires every OBJECT property declaration to carry an `element_type` slot the OBO source doesn't provide. The kernel is tolerant of `data_type: core:resource` carrying an Array value (the validator doesn't strictly enforce data-type/value shape match for resource references), so the looser declaration works in practice.

This is a recorded papering-over. The structurally correct fix would be either (a) emit `resource_array` with an element_type derived from OBO `domainRangeAxioms` (which the converter currently drops as a v1 deferral), or (b) tighten the kernel's data-type/value shape check and force the converter to choose. Neither blocks today's life-science integration test, but it's a real loose end if we add property-data-type-driven validation.

## Real-embedder semantic recall

[`crates/eigenius-embedder-candle`](../../crates/eigenius-embedder-candle/) wraps HuggingFace [Candle](https://github.com/huggingface/candle) as an Eigenius `Embedder` Component. Default model: [BGE-small-en-v1.5](https://huggingface.co/BAAI/bge-small-en-v1.5) (384-dim Sentence-BERT, 33M parameters, ~130 MB SafeTensors). Pure-Rust inference, no C++ runtime, no Python subprocess — models download from the HuggingFace Hub into `~/.cache/huggingface/` on first use via the [`hf-hub`](https://github.com/huggingface/hf-hub) crate.

The semantic recall test [`crates/eigenius-embedder-candle/tests/go_recall.rs`](../../crates/eigenius-embedder-candle/tests/go_recall.rs) exercises the full pipeline end-to-end against real GO data with **seven hand-curated paraphrased queries**:

| Paraphrased query | Expected GO term | Rank |
|---|---|---|
| fixing damage to genetic material | DNA repair | 2 |
| splitting one cell into two daughter cells | cell division | 4 |
| proteins binding to other proteins | protein binding | 1 |
| where chromosomes are stored in eukaryotes | nucleus | 3 |
| the organelle that produces cellular energy | mitochondrion | 5 |
| the fluid inside a cell that surrounds organelles | cytoplasm | 4 |
| moving molecules across cell membranes | transmembrane transport | 1 |

**Recall@10 = 7/7 = 1.00.** Every paraphrased query (no word-overlap with the canonical GO term name) found the expected term in the top 10, most in the top 5. The test corpus was 1 007 GO Classes (the 7 gold-set targets + 1 000 random distractors with biomedical descriptions) under a flat `core:VectorIndex` strategy — the test measures embedder semantic quality, not HNSW behaviour (the HNSW story has its own bench).

**Timing:**

| Phase | Wall-clock |
|---|---|
| Obograph convert (52 032 Resources) | 0.23s |
| Vector sweep, CPU per-text (baseline) | 162s |
| Vector sweep, CPU batched at batch_size=32 | 326s |
| Vector sweep, CUDA batched at batch_size=32 (RTX 4070) | **3.62s** |
| Per-query embed + flat search (CPU) | ~130ms |
| Per-query embed + flat search (CUDA) | ~30ms |

**The batched-sweep follow-up.** The `Embedder` trait grew an `embed_batch` method (default impl: per-text loop; `CandleEmbedder` overrides with a single batched BertModel forward pass). The sweep was restructured to dispatch in chunks of `SweepOptions::batch_size`.

On **CUDA** (`cargo test -p eigenius-embedder-candle --features cuda ...`) the result is what the trait promised: a single forward pass over `[32, max_seq]` saturates the GPU's compute units, so the dispatch saving dwarfs every other cost. 162s → 3.62s — a 45× speedup over the per-text CPU baseline.

On **CPU**, the batched path was actually ~2× *slower* (326s vs 162s) on this corpus. The cause is well-understood: `Tokenizer::encode_batch` uses `BatchLongest` padding, so each batch's forward cost is `[batch, max_seq] × hidden`. GO Class labels run from ~10 to a few hundred tokens, and a batch with one long member multiplies everyone's compute by an order of magnitude. CPU BLAS's batched-GEMM win (typically 2-5× on fixed-length batches) gets out-fought by that padding penalty. The single-text CPU path embeds each Resource at its own native length and avoids the waste entirely. The standard cure is length-bucketed batching (sort by `text.len()` then chunk so a batch's members are similar in length); that's tracked separately and would be the path to a fast CPU sweep on heterogeneous corpora without needing a GPU.

Functionally the batched path is correct on both devices — round-trip parity is pinned by [`sweep_results_are_independent_of_batch_size`](../../kernel/src/query/vector/indexing.rs) and the recall@10 stays at 7/7. The intra-sweep deduplication contract that the cache supplied in the per-subject loop is preserved in the batched path explicitly (group cache-miss entries by text, dispatch each unique text once, fan out to peers). Cancellation responsiveness was tightened to also cover the case where cancel fires *during* the final batch's embed — the segment write is now gated on a post-batch cancel check so the cooperative-cancel contract holds regardless of batch size.

GPU enablement is feature-flagged: `cargo build -p eigenius-embedder-candle --features cuda` (or `--features metal` on Apple Silicon). The `select_device()` helper attempts the accelerator and falls back to CPU with a `eprintln!` warning if the driver isn't present. The default build is CPU-only so a fresh checkout doesn't require CUDA toolchain to compile.

For production scenarios where on-CPU embedding cost matters, the same crate can build with `--features cuda` or `--features metal` (Candle's GPU backends). The Embedder trait stays unchanged; only the constructor path differs. Multi-process / paid-API embedders (Cohere, OpenAI) plug in via the D6 IO envelope as before — Candle isn't the only option, just the recommended pure-Rust default.

## What didn't get built and why

- **HNSW recall benchmark.** Synthetic-vector recall against brute-force is doable today (the algorithm is HNSW-vs-flat regardless of vector source), but D43's interesting recall claim is about *semantic* recall over real text. Synthetic gives us algorithmic correctness, not the publishable recall@K numbers. Both forms are valuable; the algorithmic one is in scope, the semantic one needs the real embedder.

- **Persistent reindex (TaskStore integration).** The `ReindexDriver` carries a `TaskRecord` but doesn't persist it through `TaskStore`. The in-process record is enough for synchronous CLI-driven reindex; the cross-restart persistence (so a kernel crash during reindex resumes cleanly) is the D21 follow-up that lands when the post-Load sweep gains the same persistence — they share the integration point.

- **Cross-Index score arithmetic enforcement.** A user query that does arithmetic across two `~` operators' scores via the deferred score-exposure surface would be nonsensical (scores from different sources aren't comparable). v1 doesn't expose scores at all, so the question doesn't arise. When a future revision exposes scores (the EXPLAIN-equivalent), the type checker will need to reject cross-source arithmetic; we have *not* designed that check yet.

- **D45 BIND clause.** Withdrawn during the surface reset (see top section). The withdrawal note in [d45-bind-clause.md](../design/d45-bind-clause.md) explains why; this is here as a pointer.

## Things I'd reconsider with hindsight

- **The `urn:obo:converter:eigenius-obograph` declared_by string is opaque.** It's a string in `core:string` data_type, so it could be anything. A future Resource describing the converter as an Agent (with version, run timestamp, command-line invocation) would make provenance richer. The opaque string is a v1 placeholder.

- **`META_DECLARED_IRIS` is hardcoded in the converter.** When obo-meta changes (we add a new shared OBO Property), the converter has to be rebuilt. A startup-time read of the meta ontology's declarations would be cleaner. The hardcoded list is fast and explicit; the dynamic check is more correct. Pick when a maintenance pain point actually shows up.

- **`~` returns Boolean only.** The score is computed but not bindable. Diagnostic visibility (which probe ranked which row, what the per-source score was) requires the EXPLAIN-equivalent. It's deferred for good reason (the surface stays clean) but the lack of debuggability already bit me once during integration test development when a similarity query unexpectedly returned the wrong row — I had to add `eprintln!` instrumentation to the pre-pass to see which probe was producing which candidates. A debug-only score-print mode wouldn't cost the surface much.

## v1 operating envelope

Measured June 2026 on a developer workstation (single-NUMA Linux/WSL2, RocksDB tempdir on tmpfs, criterion-free `Instant`-based timing). Numbers are wall-clock per phase; re-run via the M9.3 / M9.4 benches to refresh.

### GO corpus end-to-end (M9.4)

Source: [`crates/eigenius-obograph/tests/d43_perf_bench.rs`](../../crates/eigenius-obograph/tests/d43_perf_bench.rs). Real Gene Ontology dump (52 032 Resources after conversion: 51 967 CLASS + 65 PROPERTY).

| Phase | Wall-clock | Notes |
|---|---|---|
| Obograph convert (68 MB JSON → 52 032 Resources) | 0.24s | Pure in-memory; serde + IRI rewriting + DeclaredResource tagging + synonym ingestion |
| RocksDB seed bootstrap (11 ontology layers) | 0.09s | One-time per database; resumes are ~ms-level (verified manifest hash → already-committed) |
| `add_resource × 52 032` to LayerBuilder | 0.14s | In-memory accumulation; no I/O |
| `LayerBuilder::build` | 1.83s | Bloom + triple index + text index population through RocksDB CFs |
| **Total load**: convert + bootstrap + build | **~2.3s** | Cold start to queryable layer |
| BM25 `~` query, cold cache | 344-494ms | Across 5 nucleus-vocabulary queries; mean ~398ms |
| BM25 `~` query, warm cache | 336-420ms | Block cache primed by cold pass; mean ~359ms |

**RSS footprint:**

| Stage | Process RSS |
|---|---|
| Baseline (post-startup) | 147 MiB |
| After converter (52 k Resources in memory) | 269 MiB |
| After load + index | 563 MiB |
| End of bench (queries finished) | 614 MiB |
| Net delta (load + index + queries) | ~468 MiB |

Per-Resource memory cost is ~9 KiB end-to-end including the BM25 posting lists, triple index, and bloom filter. That's significantly higher than the on-disk Eigon-JSON (~1.3 KiB per Resource average) because the in-memory layout duplicates strings as `Arc<str>` per index lookup path and the Roaring bitmaps for postings carry their own overhead.

**Query-time dominant cost.** A 350-400ms BM25 query against 52 k indexed docs is much slower than the text-search dispatcher itself. Tracing shows the pattern-match scan (`MATCH ?c { description: ?desc }` against every Resource with a description slot) is the bottleneck, not the BM25 probe. Adding a class filter (`USING "..." MATCH SomeClass(?c) { ... }`) would prune the candidate set before the similarity scan, but real GO has no narrowing class above CLASS itself. The bench result is therefore "unfiltered MATCH + BM25 + TOP 10" against the full corpus — the realistic shape an agent asking "find GO terms about X" produces. Improving this number is a planner-pushdown concern (D43 §6.2 top-K pushdown beyond what v1 ships), not a BM25 concern.

### HNSW recall + latency (M9.3)

Source: [`kernel/tests/d43_hnsw_recall_bench.rs`](../../kernel/tests/d43_hnsw_recall_bench.rs). Synthetic clustered vectors (50 cluster centres, 64-dim, cosine similarity, shared between corpus + queries so brute-force top-K is a meaningful cluster neighbourhood). 100 queries, K=10.

#### Baseline sweep at the v1 schema default (m=16, ef_construction=200)

| Corpus N | Build | Flat brute-force | HNSW ef=16 | HNSW ef=64 | HNSW ef=256 |
|---|---|---|---|---|---|
| 1 000 | 0.14s | 0.041 ms/q | 0.005 ms/q, recall 1.000 | 0.016 ms/q, recall 1.000 | 0.060 ms/q, recall 1.000 |
| 10 000 | 5.94s | 0.435 ms/q | 0.015 ms/q, recall 0.722 | 0.029 ms/q, recall 0.722 | 0.020 ms/q, recall 0.722 |
| 50 000 | 258s | 2.833 ms/q | 0.089 ms/q, recall 0.551 | 0.143 ms/q, recall 0.577 | 0.329 ms/q, recall 0.579 |

**Latency story is healthy.** HNSW beats flat brute-force at every size — 8× at 1k, 15-30× at 10k, 9-32× at 50k. The query path scales sublinearly with corpus size, which is what HNSW promises.

**Recall doesn't scale with `ef`.** At N=10k the ef=16 / ef=64 / ef=256 results are within 0.01 of each other across the board. The published HNSW expectation is ~95% recall at ef=k·2 (here ef=20), ~99% at ef=k·8 (ef=80). The shipped builder converges to its asymptotic recall well before ef=20, suggesting **the graph's connectivity rather than the search effort is the constraint**.

#### m-sweep at N=10k (validating the connectivity hypothesis)

| m | Build | ef=16 | ef=64 | ef=256 |
|---|---|---|---|---|
| **16** | 5.94s | recall **0.722** | recall 0.722 | recall 0.722 |
| **32** | 7.00s | recall **0.927** | recall 0.927 | recall 0.927 |
| **48** | 8.29s | recall **0.890** | recall 0.890 | recall 0.890 |

**Hypothesis confirmed.** Raising m from 16 to 32 lifts recall@10 from 0.72 to 0.93 — a +0.21 absolute improvement that the `ef` knob cannot achieve at m=16 no matter how high it goes. Build time grows ~linearly in m (1.18× per step), which is the expected proportional cost.

**Two non-obvious findings from the m-sweep:**

1. **m=48 *under-performs* m=32 (0.89 vs 0.93).** Repeatable across the three ef values, so it's not Monte-Carlo noise on a single setting. Likely cause: the neighbour-selection heuristic the builder uses retains too many redundant edges at high m, biasing the graph toward overconnected hubs that the search hits and gets stuck on. A neighbour-pruning improvement (heuristic from [Malkov & Yashunin §4.3](https://arxiv.org/abs/1603.09320)) would likely make m=48 monotonically better than m=32, but the in-tree builder doesn't implement that. **Use m=32 as the practical sweet spot until the builder gains heuristic pruning.**

2. **`ef` still does nothing inside one m.** At m=32, ef=16 / 64 / 256 all return identical recall (0.927). The search is finding everything reachable from the entry point, just faster or slower. Combined with finding (1) — the search exhausts the graph quickly because the graph itself is the constraint — this points at the same place: **the build path (neighbour selection + entry-point seeding) is where the algorithm's recall is decided**, not the search path.

#### Other findings

**Recall degrades with N at fixed m.** 1k → 1.000 (clean separation), 10k → 0.77, 50k → 0.58 (all at m=16). The synthetic corpus has fixed `CLUSTERS = 50` centres regardless of N, so each cluster grows from 20 members at N=1k to 1 000 at N=50k. Finding the *exact* top-10 within a 1 000-member cluster is structurally harder than within a 20-member cluster — many corpus points are roughly equidistant from a random query. Some of the recall droop is structural to the synthetic data; the m-sweep confirms the rest is the algorithm.

**Build time scales superlinearly.** 1k → 0.14s, 10k → 5.94s (42×), 50k → 258s (43×) at m=16. For O(N log N) ideal scaling the 10× steps should be ~12-14×; the 42× factor suggests something in the build loop is closer to O(N²). The 4-minute 50k build is the dominant cost in the 50k benchmark; it's not characteristic of well-tuned HNSW implementations and is a clear optimisation target.

#### Net verdict on the in-tree HNSW (M6.4)

- **Query latency**: well-tuned at every size and m value.
- **Recall**: m=32 + ef=16 (`vec_hnsw_m = 32` on the VectorIndex Resource) is the recommended operating point for v1 users. The ef knob does nothing meaningful past ~16-20 at any m; raising it just spends more CPU for the same result.
- **Build cost**: needs profiling; current scaling is closer to O(N²) than O(N log N).
- **Future work**: implement Malkov & Yashunin §4.3 neighbour-selection heuristic to unlock m=48+ scaling, then revisit the recall table.

None of these block v1 shipping — the surface is correct and small corpora work fine — but the implementation has clear maintenance work ahead before users with >10k vectors per VectorIndex segment will get the recall they expect.

## Source pointers

Every claim above is checkable against the source:

| Topic | Source |
|---|---|
| Similarity operator + hint block | [kernel/src/query/parser.rs](../../kernel/src/query/parser.rs) |
| Pre-pass + RRF fusion | [kernel/src/query/evaluate/similarity.rs](../../kernel/src/query/evaluate/similarity.rs) |
| TOP-before-RETURN sort | [kernel/src/query/evaluate/mod.rs](../../kernel/src/query/evaluate/mod.rs) |
| Typecheck rules | [kernel/src/query/type_check.rs](../../kernel/src/query/type_check.rs) |
| OBO IRI rewriting | [crates/eigenius-obograph/src/convert.rs](../../crates/eigenius-obograph/src/convert.rs) |
| obo-meta layer | [ontologies/obo/obo-meta-ontology.json](../../ontologies/obo/obo-meta-ontology.json) |
| Bootstrap chain | [kernel/src/bootstrap/mod.rs](../../kernel/src/bootstrap/mod.rs) |
| Sweep + reindex coordinator | [kernel/src/task/sweep_registry.rs](../../kernel/src/task/sweep_registry.rs) |
| Multiplicity verification | [kernel/src/layer/index_discovery.rs](../../kernel/src/layer/index_discovery.rs) |
| HNSW recall bench (M9.3 algorithm-level) | [kernel/tests/d43_hnsw_recall_bench.rs](../../kernel/tests/d43_hnsw_recall_bench.rs) |
| Semantic recall test (M9.3 semantic-level) | [crates/eigenius-embedder-candle/tests/go_recall.rs](../../crates/eigenius-embedder-candle/tests/go_recall.rs) |
| Candle-backed embedder | [crates/eigenius-embedder-candle](../../crates/eigenius-embedder-candle/) |
| GO perf envelope bench (M9.4) | [crates/eigenius-obograph/tests/d43_perf_bench.rs](../../crates/eigenius-obograph/tests/d43_perf_bench.rs) |
