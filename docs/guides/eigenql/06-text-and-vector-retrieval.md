# 6. Text and vector retrieval

EigenQL queries on natural-language content — descriptions, document bodies, comments, requirement statements — need *fuzzy* retrieval, not just exact pattern matching. D43 ships that as a **single binary operator `~`** between a property-bound variable and a string. The platform dispatches text retrieval (BM25 over an inverted index), vector retrieval (HNSW or flat over an embedding), or both with internal Reciprocal Rank Fusion, depending on which indexes the schema declared. The user query does not name the strategy, the embedder, or the fusion function — only the intent.

Implementation source:

- AST: [`Expression::Similarity`](../../../kernel/src/query/ast.rs) + [`HintSet`](../../../kernel/src/query/ast.rs) + [`Via`](../../../kernel/src/query/ast.rs)
- Parser: [`parse_similarity_continuation`](../../../kernel/src/query/parser.rs) + [`parse_hint_set`](../../../kernel/src/query/parser.rs)
- Typecheck: [`check_similarity`](../../../kernel/src/query/type_check.rs) + [`check_similarity_node`](../../../kernel/src/query/type_check.rs)
- Evaluator pre-pass: [`SimilarityContext`](../../../kernel/src/query/evaluate/similarity.rs)
- Per-row eval: [`eval_similarity`](../../../kernel/src/query/evaluate/expression.rs)
- Schema discovery: [`resolve_active_text_indexes`](../../../kernel/src/layer/index_discovery.rs), [`resolve_active_vector_indexes`](../../../kernel/src/layer/index_discovery.rs)

Design reference: [D43 — Text and Vector Retrieval](../../design/d43-text-and-vector-retrieval.md).

## 6.1 Why a similarity operator

The §7 quick-tour patterns are mostly structural: walk known IRIs, join classes, filter on equality. A different class of question begins with a fuzzy concept:

- "Find code artifacts related to WAL truncation."
- "Find GO terms whose definition is about chromosome housing."
- "Find documents discussing the layer-merge invariant."

You can't write these as `WHERE ?desc = "..."` — the user query is not a substring of the stored value. BM25 (term-weighted bag-of-words) and dense-vector cosine similarity are well-understood solutions, and the kernel maintains both indexes per declared Property. The operator `~` is the user-facing surface that consults them.

A single operator was chosen over the SQL-shaped alternative (`TEXT_MATCH(...)` / `VECTOR_NEAR(...)` / `EMBED(...)` / `RRF(...)` / `TOP K BY <score>`) because:

- The strategy is a schema decision, not a query decision. The schema owner declares which indexes are active; the query writer expresses intent.
- The embedder, fusion algorithm, and per-source scores are implementation details the user shouldn't have to name to ask "find related things."
- Multiple `~` operators compose under the same fusion machinery without the user having to write `RRF(t1, t2, v1, v2)` by hand.

See the M7 surface-reset note in [d43-implementation-plan.md](../../design/d43-implementation-plan.md#m7--similarity-operator--hybrid-retrieval) for the full rationale.

## 6.2 Declaring an index on a Property

Retrieval is enabled by **declaring an Index Resource** that targets a specific Property. Two Index classes ship in the core ontology:

- [`urn:eigenius:core:TextIndex`](../../../ontologies/core/core-ontology.json) — BM25 inverted index with a configurable analyzer.
- [`urn:eigenius:core:VectorIndex`](../../../ontologies/core/core-ontology.json) — vector segments (flat or HNSW per the declared strategy) over an embedder's output.

A TextIndex Resource:

```json
{
  "@id": "urn:eigenius:example:ti_description",
  "urn:eigenius:core:is_a": ["urn:eigenius:core:TextIndex"],
  "urn:eigenius:core:target_property": "urn:eigenius:example:description",
  "urn:eigenius:core:text_analyzer": "en-stem-v1"
}
```

A VectorIndex Resource:

```json
{
  "@id": "urn:eigenius:example:vi_description",
  "urn:eigenius:core:is_a": ["urn:eigenius:core:VectorIndex"],
  "urn:eigenius:core:target_property": "urn:eigenius:example:description",
  "urn:eigenius:core:vec_model": "urn:eigenius:embed:my-model:v1",
  "urn:eigenius:core:vec_dim": 384,
  "urn:eigenius:core:vec_distance": "urn:eigenius:core:distances:cosine",
  "urn:eigenius:core:vec_strategy": "urn:eigenius:core:strategies:auto"
}
```

Required slots:

| Slot | Both | TextIndex | VectorIndex |
|---|---|---|---|
| `target_property` | ✓ | | |
| `text_analyzer` | | ✓ | |
| `vec_model` | | | ✓ |
| `vec_dim` | | | ✓ |

Recommended slots on VectorIndex: `vec_distance` (default `cosine`), `vec_strategy` (default `auto`), `vec_hnsw_m`, `vec_hnsw_ef_construction`, `vec_embedding_policy`. See [`ActiveVectorIndex`](../../../kernel/src/layer/index_discovery.rs) for the full field set.

**v1 multiplicity:** at most one TextIndex *and* at most one VectorIndex per target Property per head. Both can coexist on the same Property — that's the hybrid case.

**Text-index population** happens automatically at `LayerBuilder::build`: [`populate_text_indexes`](../../../kernel/src/query/text/indexing.rs) walks the layer's defined Resources, extracts the target property's string value, tokenises through the analyzer, and writes per-`(index, layer)` posting lists into the storage backend.

**Vector-index population** runs through the post-Load sweep ([`sweep_layer_vectors`](../../../kernel/src/query/vector/indexing.rs)) — it needs an Embedder Component (§6.8) which `LayerBuilder::build` doesn't have. Callers either invoke the sweep directly after build, or rely on the [`SweepCoordinator`](../../../kernel/src/task/sweep_registry.rs) wired into the commit hook.

## 6.3 The `~` operator

Syntax:

```
?property_var ~ "string literal"
```

The left-hand side **must** be a property-bound variable — a variable bound by a property pattern in `MATCH`. The right-hand side is a string literal (in v1; future revisions may accept string-typed expressions).

Example:

```eigenql
USING "urn:eigenius:example:CodeArtifact"
MATCH CodeArtifact(?a) { description: ?desc }
WHERE ?desc ~ "concurrent commit recovery"
```

`?desc` was bound by the `description: ?desc` property pattern; the operator consults the active index(es) on `description` to decide which rows survive. The same operator works whether the schema declared only a TextIndex, only a VectorIndex, or both (§6.6).

`~` sits at relational precedence in the [operator table](12-appendix.md) — between additive and comparison operators. `?a ~ "x" AND ?b ~ "y"` parses as `(?a ~ "x") AND (?b ~ "y")`, no parentheses needed. The operator is right-bounded by its hint block (next section) or by the next clause keyword.

**The operator is Boolean.** It returns true for rows the platform decided are similar enough; the platform-internal score it computed feeds `TOP N`'s implicit ranking (§6.5) but is not exposed as a value the query can bind. If you need score inspection for debugging, the EXPLAIN-equivalent surface is deferred to a future revision; see [D43 §3.7](../../design/d43-text-and-vector-retrieval.md).

## 6.4 The hint block

An optional trailing braces block overrides individual platform defaults:

```
?property_var ~ "query" { via: text|vector|hybrid, model: <iri>, k: <int>, limit: <int> }
```

Hint keys (all optional; any subset; order doesn't matter):

| Key | Type | Effect |
|---|---|---|
| `via` | `text` / `vector` / `hybrid` | Force the strategy. `text` uses only the TextIndex; `vector` uses only the VectorIndex; `hybrid` fuses both (default when both are active). |
| `model` | IRI string | Override the embedder. Implicitly forces `via: vector`. |
| `k` | positive integer | Override the RRF fusion constant. Default 60 (Cormack et al.). Smaller `k` → rank-1-dominant; larger `k` → flatter distribution. |
| `limit` | positive integer | Per-source candidate-set cap *before* fusion. Default 200. Tightens the over-fetch policy locally. |

Examples:

```eigenql
?desc ~ "WAL truncation"                                 // defaults
?desc ~ "WAL truncation" { via: text }                   // text-only
?desc ~ "WAL truncation" { via: vector, model: "..." }   // vector path, custom embedder
?desc ~ "WAL truncation" { k: 30 }                       // tighter RRF
?desc ~ "WAL truncation" { limit: 50 }                   // smaller probe pool
```

Validation at parse and typecheck:

- Unknown hint keys reject at parse: `unknown similarity hint 'X' (allowed: via, model, k, limit)`.
- `via: text` requires an active TextIndex (typecheck rule `similarity_hint_via_text_no_text_index`).
- `via: vector` requires an active VectorIndex (`similarity_hint_via_vector_no_vector_index`).
- `via: hybrid` requires both (`similarity_hint_via_hybrid_missing_index`).
- `model: M` requires an active VectorIndex whose `vec_model` matches `M` (`similarity_hint_model_mismatch` if different, `similarity_hint_model_no_vector_index` if no VectorIndex active).
- `model:` combined with `via: text` is rejected (`similarity_hint_model_with_via_text`).
- `k` and `limit` must be positive (≥ 1).

The parser surface is [`parse_hint_set`](../../../kernel/src/query/parser.rs); the typecheck rules are in [`check_similarity_node`](../../../kernel/src/query/type_check.rs).

## 6.5 `TOP N` — ranked truncation

When a query contains `~` operators, the rows that survive `WHERE` carry a fused similarity score. `TOP N` orders by that score descending and keeps the highest N:

```eigenql
MATCH ?d { description: ?desc }
WHERE ?desc ~ "kernel layer chain"
RETURN [] { d: ?d }
TOP 20
```

Structural rules (enforced at typecheck):

- `TOP N` requires N > 0 (rule `top_must_be_positive`).
- `TOP N` requires at least one `~` operator in `WHERE` (rule `top_without_similarity`). Without ~, ranking has no source — use `LIMIT` instead.
- `TOP N` is mutually exclusive with `LIMIT` (`top_with_limit`). `LIMIT` is un-ranked truncation; `TOP` is similarity-ranked truncation.
- `TOP N` is mutually exclusive with `ORDER BY` (`top_with_order_by`). The ranking key comes from the similarity score, not a user expression.

`TOP` is parsed alongside the other trailing-clause keywords (`LIMIT`, `OFFSET`, `DISTINCT`); see [chapter 4](04-program-structure.md) for the full clause order.

Implementation: the evaluator's [`SimilarityContext::aggregate_score`](../../../kernel/src/query/evaluate/similarity.rs) sums probe contributions across all `~` operators a row participated in. Bindings sort descending by aggregate score before RETURN shaping, then truncate to N. Sorting *before* shaping is load-bearing — shaped resources project away the subject-IRI binding the score lookup needs.

## 6.6 Hybrid retrieval and RRF fusion

When a Property has **both** a TextIndex and a VectorIndex active, the platform runs both probes in parallel and fuses the rankings with Reciprocal Rank Fusion (RRF):

```
score(row) = sum over sources i of  1 / (k + rank_i(row))
```

where `rank_i(row)` is the row's 1-indexed position in source `i`'s ranking, or ∞ if the row didn't appear in source `i`'s candidate set, and `k=60` by default (overridable per operator via the `k:` hint).

The fusion implementation is [`fuse_rrf`](../../../kernel/src/query/evaluate/similarity.rs):

- Text probe goes through [`run_text_search`](../../../kernel/src/query/text/search.rs) — chain-aware BM25 with the declared analyzer.
- Vector probe goes through [`top_k_subjects`](../../../kernel/src/query/vector/search.rs) — the query string is embedded once (caching applies across operators sharing the same string) and the resulting vector probes the declared VectorIndex's segments.
- A row in *both* candidate sets accumulates contributions from both, so it outranks rows that only appeared in one. This is why a hybrid query is more robust than either source alone.

Strategy selection ([`build_probe`](../../../kernel/src/query/evaluate/similarity.rs)):

1. Explicit `via:` hint wins.
2. Otherwise: `model:` hint forces vector.
3. Otherwise: both probes run when both indexes are active; only the one available runs when one is.
4. Otherwise: typecheck already failed.

## 6.7 Multiple operators — composition

`~` composes with itself and with normal Boolean operators:

**Conjunction.** Both must admit the row; both contribute to the aggregate score:

```eigenql
WHERE ?a ~ "x" AND ?b ~ "y"
```

**Disjunction.** Either admits the row; rows satisfying both rank higher than rows satisfying one:

```eigenql
WHERE ?desc ~ "WAL truncation"
   OR ?desc ~ "rolling back a partially-written commit"
TOP 20
```

The aggregate score is the sum of every probe whose candidate set contains the row's subject IRI. With OR semantics, rows in the intersection accumulate two contributions; rows in only one source accumulate one. The platform's internal ordering reflects that without the user writing anything explicit.

**Hybrid + composition.** Each `~` operator probes its property's active indexes independently. Two operators on a hybrid-indexed property produce up to four probe contributions per row; one operator on a hybrid-indexed property produces two.

This is the structurally interesting affordance D43 ships: **all retrieval composition is RRF inside the platform**, regardless of whether composition comes from multiple operators or from multiple indexes on one property. The user query and the schema's index declarations are the only inputs.

## 6.8 The Embedder Component

The vector path needs an embedder to translate the query string into a vector at runtime. Embedders are registered as a kernel-wide IO Component implementing the [`Embedder`](../../../kernel/src/program/embedder.rs) trait:

```rust
pub trait Embedder: Send + Sync {
    fn model_iri(&self) -> &Iri;
    fn dim(&self) -> u32;
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedderError>;
}
```

Registration:

```rust
use eigenius_kernel::program::embedder::EmbedderRegistry;

let mut registry = EmbedderRegistry::new();
registry.register(Arc::new(MyEmbedder::new("urn:eigenius:embed:my-model:v1", 384)));

let runtime = FiberRuntime {
    embedders: Some(&registry),
    ..FiberRuntime::default()
};
```

The query path looks up the embedder by IRI (the VectorIndex's `vec_model` slot, or the `model:` hint when present). Missing-embedder is a runtime error: `no Embedder registered for model '...'`.

For tests, [`DummyEmbedder`](../../../kernel/src/program/embedder.rs) produces deterministic blake3-derived vectors. For production, the Embedder Component is typically a thin wrapper around a Sentence-BERT / E5-small / instruction-tuned model, dispatched through the D6 IO envelope.

The post-Load sweep uses the same registry to populate vector segments — same embedder for indexing and querying, so the per-segment `model_iri` matches at query time.

## 6.9 Worked examples

### Pure text retrieval

Property has only a TextIndex declared:

```eigenql
USING "urn:eigenius:example:Doc"
MATCH Doc(?d) { description: ?desc }
WHERE ?desc ~ "kernel layer chain consolidation"
RETURN [] { d: ?d }
TOP 20
```

The platform runs BM25, returns the top 20 by score.

### Pure vector retrieval

Same query against a property with only a VectorIndex declared. The platform embeds the query string once via the declared embedder, probes the VectorIndex, returns top 20 by cosine similarity.

### Hybrid retrieval

Same query against a property with both indexes. Internal RRF fusion runs across both candidate sets; top 20 by fused score.

### Structural composition

The structural pattern filters; the similarity operator ranks the survivors:

```eigenql
USING "urn:eigenius:example:CodeArtifact",
      "urn:eigenius:contracts:BoundaryContract"
MATCH CodeArtifact(?a) {
    description: ?desc,
    contracted_by: ?bc
}
WHERE ?desc ~ "walk the chain and apply shadow filter"
RETURN [] { artifact: ?a, contract: ?bc }
TOP 50
```

`contracted_by: ?bc` requires the artifact to carry a contract. Among the survivors, BM25/vector ranks by similarity to the query.

### Disjunctive sources

Two queries OR'd; rows matching both rank highest:

```eigenql
MATCH Doc(?d) { title: ?t, body: ?b }
WHERE ?t ~ "WAL truncation"
   OR ?b ~ "rolling back a partial commit"
RETURN [] { d: ?d }
TOP 20
```

### Hint-driven override

Force text-only on a property that's exact-match heavy (function names, identifiers):

```eigenql
USING "urn:eigenius:example:Symbol"
MATCH Symbol(?s) { name: ?n }
WHERE ?n ~ "RocksStore::store_layer" { via: text }
RETURN [] { s: ?s }
TOP 10
```

### Recall-vs-precision tuning

Smaller `limit` → tighter candidate pool → fewer hits but higher precision:

```eigenql
MATCH Doc(?d) { description: ?desc }
WHERE ?desc ~ "concurrent commit recovery" { limit: 50 }
RETURN [] { d: ?d }
TOP 20
```

## 6.10 Failure modes

### Parse-time

- **Non-variable LHS.** `(?a || ?b) ~ "x"` fails with `similarity LHS must be a property-bound variable`. The LHS must be a bare variable token.
- **Unknown hint key.** `~ "x" { weights: 1 }` fails with `unknown similarity hint 'weights' (allowed: via, model, k, limit)`.
- **Bad `via` value.** `~ "x" { via: graph }` fails with `hint 'via' must be 'text', 'vector', or 'hybrid' (got 'graph')`.
- **TOP without similarity.** `MATCH ?x {} TOP 10` would parse but fail typecheck (next section). The parser admits the shape; typecheck enforces the requirement.

### Typecheck

Every rule below is in [`check_similarity_node`](../../../kernel/src/query/type_check.rs) or the TOP block in [`type_check`](../../../kernel/src/query/type_check.rs). The `QueryError::rule` field carries the short identifier in parentheses so callers can dispatch on it programmatically.

- (`similarity_lhs_not_property_bound`) The LHS variable wasn't bound by a property pattern in MATCH. Common cause: typo, or binding the variable as a subject (`?x` in `MATCH ?x {...}`).
- (`similarity_property_not_string`) The bound property isn't `data_type: core:string`. v1 only supports string content for similarity.
- (`similarity_no_active_index`) The property has neither a TextIndex nor a VectorIndex declared at this head. Add a schema declaration or query a different property.
- (`similarity_rhs_not_string_literal`) The right-hand side isn't a string literal. v1 accepts only literals.
- (`similarity_hint_via_text_no_text_index`, etc.) Hint inconsistency with the active index set; see §6.4.
- (`top_must_be_positive`) `TOP 0` is rejected.
- (`top_with_limit`) Use one or the other; never both.
- (`top_with_order_by`) ORDER BY supplies its own key; TOP supplies the similarity key.
- (`top_without_similarity`) TOP needs `~` somewhere in WHERE to have something to rank.

### Runtime

These survive past typecheck and only fail at evaluation:

- **Embedder unavailable.** The vector path is required (default-hybrid, `via: vector`, or implicit via `model:`), but no `EmbedderRegistry` was passed in the `FiberRuntime`. Error: `no Embedder registry available for the '~' operator's vector path`.
- **Embedder model not registered.** A registry is present but doesn't have the specific `model_iri` the VectorIndex declares. Error: `no Embedder registered for model 'X' (required by VectorIndex 'Y')`.
- **Analyzer not registered.** The TextIndex declares an analyzer ID the kernel doesn't ship. Error: `analyzer 'X' for TextIndex 'Y' not registered`. v1 ships `en-stem-v1` and `en-no-stem`.
- **Vector segment not yet swept.** The VectorIndex is declared but the post-Load sweep hasn't completed. The probe sees an empty candidate set; results are well-typed but partial. No error — the sweep's `TaskStatus` is the observability surface.
- **Embedder dispatch failure.** The Embedder Component itself errored (network, timeout, malformed input). Surfaces as `embedder dispatch failed: <err>` from the pre-pass.

## 6.11 Source pointers

| Concern | Module |
|---|---|
| AST | [kernel/src/query/ast.rs](../../../kernel/src/query/ast.rs) |
| Lexer (`Tilde` token) | [kernel/src/query/lexer.rs](../../../kernel/src/query/lexer.rs) |
| Parser | [kernel/src/query/parser.rs](../../../kernel/src/query/parser.rs) |
| Typecheck | [kernel/src/query/type_check.rs](../../../kernel/src/query/type_check.rs) |
| Schema discovery | [kernel/src/layer/index_discovery.rs](../../../kernel/src/layer/index_discovery.rs) |
| Evaluator pre-pass | [kernel/src/query/evaluate/similarity.rs](../../../kernel/src/query/evaluate/similarity.rs) |
| Per-row evaluator arm | [kernel/src/query/evaluate/expression.rs](../../../kernel/src/query/evaluate/expression.rs) |
| Aggregate-score + TOP sort | [kernel/src/query/evaluate/mod.rs](../../../kernel/src/query/evaluate/mod.rs) |
| Text BM25 dispatch | [kernel/src/query/text/search.rs](../../../kernel/src/query/text/search.rs) |
| Text indexing at build | [kernel/src/query/text/indexing.rs](../../../kernel/src/query/text/indexing.rs) |
| Vector probe + HNSW | [kernel/src/query/vector/search.rs](../../../kernel/src/query/vector/search.rs) |
| Vector sweep | [kernel/src/query/vector/indexing.rs](../../../kernel/src/query/vector/indexing.rs) |
| Embedder trait + registry | [kernel/src/program/embedder.rs](../../../kernel/src/program/embedder.rs) |
| Sweep task driver | [kernel/src/task/sweep.rs](../../../kernel/src/task/sweep.rs) |
| Reindex task driver | [kernel/src/task/reindex.rs](../../../kernel/src/task/reindex.rs) |
| Sweep + reindex coordinator | [kernel/src/task/sweep_registry.rs](../../../kernel/src/task/sweep_registry.rs) |

End-to-end integration tests showing the full pipeline:

- [`kernel/src/query/evaluate/similarity.rs`](../../../kernel/src/query/evaluate/similarity.rs) — text-only, hybrid, TOP K, via hints, error paths
- [`kernel/tests/d35_se_retrieval_worked_example.rs`](../../../kernel/tests/d35_se_retrieval_worked_example.rs) — D35 §7.4 worked example
- [`crates/eigenius-obograph/tests/d43_go_subset_integration.rs`](../../../crates/eigenius-obograph/tests/d43_go_subset_integration.rs) — real GO data, RocksDB backend

Next: **[7. Expressions →](07-expressions.md)**
