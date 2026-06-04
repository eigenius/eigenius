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

## What didn't get built and why

- **Real-embedder integration test.** The `DummyEmbedder` is hash-based and produces deterministic-but-meaningless vectors. Validating *recall quality* against real biomedical vocabulary needs a Sentence-BERT / E5-small model packaged as an Embedder Component, plus a hand-curated gold set of `(query, expected GO term)` pairs. Real work; deliberately deferred until there's an actual life-science consumer asking for it.

- **HNSW recall benchmark.** Synthetic-vector recall against brute-force is doable today (the algorithm is HNSW-vs-flat regardless of vector source), but D43's interesting recall claim is about *semantic* recall over real text. Synthetic gives us algorithmic correctness, not the publishable recall@K numbers. Both forms are valuable; the algorithmic one is in scope, the semantic one needs the real embedder.

- **Persistent reindex (TaskStore integration).** The `ReindexDriver` carries a `TaskRecord` but doesn't persist it through `TaskStore`. The in-process record is enough for synchronous CLI-driven reindex; the cross-restart persistence (so a kernel crash during reindex resumes cleanly) is the D21 follow-up that lands when the post-Load sweep gains the same persistence — they share the integration point.

- **Cross-Index score arithmetic enforcement.** A user query that does arithmetic across two `~` operators' scores via the deferred score-exposure surface would be nonsensical (scores from different sources aren't comparable). v1 doesn't expose scores at all, so the question doesn't arise. When a future revision exposes scores (the EXPLAIN-equivalent), the type checker will need to reject cross-source arithmetic; we have *not* designed that check yet.

- **D45 BIND clause.** Withdrawn during the surface reset (see top section). The withdrawal note in [d45-bind-clause.md](../design/d45-bind-clause.md) explains why; this is here as a pointer.

## Things I'd reconsider with hindsight

- **The `urn:obo:converter:eigenius-obograph` declared_by string is opaque.** It's a string in `core:string` data_type, so it could be anything. A future Resource describing the converter as an Agent (with version, run timestamp, command-line invocation) would make provenance richer. The opaque string is a v1 placeholder.

- **`META_DECLARED_IRIS` is hardcoded in the converter.** When obo-meta changes (we add a new shared OBO Property), the converter has to be rebuilt. A startup-time read of the meta ontology's declarations would be cleaner. The hardcoded list is fast and explicit; the dynamic check is more correct. Pick when a maintenance pain point actually shows up.

- **`~` returns Boolean only.** The score is computed but not bindable. Diagnostic visibility (which probe ranked which row, what the per-source score was) requires the EXPLAIN-equivalent. It's deferred for good reason (the surface stays clean) but the lack of debuggability already bit me once during integration test development when a similarity query unexpectedly returned the wrong row — I had to add `eprintln!` instrumentation to the pre-pass to see which probe was producing which candidates. A debug-only score-print mode wouldn't cost the surface much.

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
