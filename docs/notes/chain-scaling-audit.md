# Chain-scaling audit — full-chain scans (`iter_all_resources`) and O(chain) hot spots

**Status:** living audit. **Context:** D23 (out-of-core layers). Large knowledge-graph
sources (UMLS Level-0 ≈ 281k resources, more to come) live on the *interactive* chain
as real KG content — that is the design target, not a stress hack. So any operation
that walks the whole chain is fine on a small chain and seconds-slow on a large one;
these are first-class scaling bugs to fix structurally, never to route around by moving
data off-chain.

## The anti-pattern

`Layer::iter_all_resources()` (and `iter_resources()`) materialise resource bodies —
under D23 each one pages cache→backend. Using them to *find a typed subset* (e.g. "all
`Institution` resources", "all `InductiveType`s") is O(chain) when it should be
O(matches). Likewise, re-`resolve`-ing the same definitions per resource is O(chain)
per call.

## Two fix techniques (and when each is safe)

1. **Index-driven discovery** — `crate::layer::resolve_typed_resources(layer,
   &[metaclass_iri, …])` finds resources of a type without materialising the chain,
   then resolves each to its top view (which also filters DAG-wide index hits to this
   chain). O(matches). It walks the chain consulting **each layer's own storage** in
   either state: the **triple index** for stored layers (populated at `store_layer`)
   and the **`pending` stage** for in-flight (built-but-not-stored) layers — so it is
   correct against freshly-built layers too (bootstrap, build-then-compile), not just
   committed chains. Triple indexes are deduped by `Arc` identity, so a shared-storage
   chain (production) scans once; per-layer-storage chains (some tests) are also
   covered. `is_a` must be indexable (`data_type = resource_array`) for the stored
   path — true on any core-rooted chain.
2. **Per-pass resolve memo** — `crate::layer::ResolveMemoScope` memoises
   `Layer::resolve` for the duration of an immutable-chain pass (used by validation).
   Collapses millions of redundant deep resolves into one per `(layer, iri)`.

## Site inventory (`kernel/src` `iter_all_resources` callers)

Legend: ✅ fixed · ⚠️ confirmed bottleneck, open · 🔵 legitimate (inherently needs all
resources) · 🔎 needs review/measurement.

| Site | What it finds | Disposition |
|---|---|---|
| `institution/registry.rs` `from_layer` | Institution/QueryClass/Comorphism/Export·ImportFormat | ✅ commit hot path now uses `from_layer_indexed` (index-driven); `from_layer` kept for tests + non-hot callers. Equivalence-tested. |
| `capability/registration.rs` `build_wasm_institution_runtime` | WASM `Institution`s | ✅ commit hot path uses `build_wasm_institution_runtime_indexed`; full-scan kept for other callers. |
| `capability/registration.rs` `validate_external_institution_chain` | external `Institution`s | ✅ switched to `resolve_typed_resources` (single hot-path caller). |
| `server/topology.rs` (was 2×: counts + nodes) | per-layer kind counts / nodes | ✅ counts via triple index; resource nodes only when `include_resources=true`; stack view is layers-only. |
| `esl/compile.rs` `collect_ctors_from_layer` | `InductiveType` (+ embedded ctors) | ✅ **fixed** (was the ESL-compile ~7s). Index-driven via `resolve_typed_resources`. The in-flight gap (compile runs against not-yet-stored layers during bootstrap → `lexicon:Cat` not in the index) is handled by `resolve_typed_resources` itself: it consults each chain layer's **own storage** — triple index for stored layers + `pending` for in-flight ones — so no build-time persisted artifact is needed. |
| `esl/compile.rs` `collect_macros_from_layer` | `core:Macro` | ✅ **fixed** — same mechanism (`resolve_typed_resources` with `core:Macro`). |
| `query/evaluate/pattern.rs` `collect_candidates_via_scan` (untyped `MATCH ?r {}`) | all resources | ✅ **fixed** for subject-bound WHERE (`LIKE`/`=`/`IN`) via subject-predicate pushdown — was the `query` RPC's 66s. Remaining untyped+non-subject-WHERE still scans (🔎, secondary). |
| `query/evaluate/pattern.rs` `resolve_name` (ShortName) | class by `short_name` | 🔎 short-name resolution task — see section below. Demo queries use full IRIs (no hit). |
| `query/type_check.rs` `resolve_short_name_to_query_class`, `resolve_property_name` | class/property by `short_name` (compile) | 🔎 short-name resolution task — see section below. |
| `query/evaluate/{fiber,similarity}.rs`, type_check other sites | EigenQL eval | 🔵/🔎 genuine all-resource MATCH (🔵) vs find-by-type (🔎); review per operator under real workloads. |
| `server/inspect.rs` `resource_count = head.iter_all_resources().count()` | total count for inspect/health | 🔎 O(chain) just to count. Prefer summing `LayerHandle.resource_count` over the chain (metadata, no bodies). |
| `program/axiom_env.rs` | axiom resources for a program run | 🔎 per-program-run; review if it's find-by-type. |
| `nbe/check.rs` | resources during NbE checking | 🔎 review. |
| `validation/retroactive.rs` | new layer's resources for retroactive pass | 🔎 measured fast so far (~0.6s); revisit if it grows. |
| `dcg/lookup.rs` | lexical entries for DCG | 🔎 parse path; the lazy `LexicalIndex` (value index) is meant to cover this — confirm this site isn't a redundant full scan. |
| `layer/consolidate.rs` | range consolidation | 🔵 bulk maintenance op, inherently whole-range. |
| `layer/mod.rs` (test helpers) | — | 🔵 tests. |

## Confirmed measurements (UMLS chain, kernel `duration_ms` + orchestrator `latency_ms`)

- Commit pipeline: **2–27 ms** (was fine).
- Post-commit institution rebuild: **9.3 s → ~0.4 ms** after index-driven fix.
- `layer_topology` (stack view): **57 ms** after lazy fix.
- `load` (ESL compile): **~7 s** ← `collect_ctors`/`collect_macros` full scans. ✅ fixed.
- `query` (EigenQL): untyped `MATCH ?r {}` + `WHERE LIKE` **66 s → 0.01 s**; `WHERE =`
  3.5 s → 0.00 s; typed/indexed query 3.6 s → 0.00 s. Two fixes: subject-predicate
  pushdown (below) + `type_check` using the index-driven institution rebuild (below).
  All ✅ — queries are effectively instant on the UMLS chain now.
- `run_program_by_iri`: ~19 s, dominated by an ~11 s LLM call (not a chain scan).

## EigenQL query — what was fixed vs still open

**Fixed — per-query institution-rebuild floor** (`type_check.rs`): every query's
`type_check` called `InstitutionIndex::from_layer(layer)` — the full-chain-scan version —
to build the index for FIBER / qualified-call checks. That was a ~3.5s per-query floor on
the UMLS chain, hidden *inside* a helper call (not a direct `iter_all_resources`, so it
didn't show in the grep — a reminder that O(chain) hides behind helpers like `from_layer`).
Switched to `from_layer_indexed` (index-driven; the query head is stored, so the triple
index covers it; equivalence-tested). Floor gone (3.5s → ~0).

**Fixed — subject-predicate pushdown** (`evaluate/pattern.rs`): an untyped `MATCH ?r {}`
no longer scans + materialises the whole chain when a top-level WHERE conjunct constrains
the subject. `extract_subject_constraint` recognises `?r LIKE "p%"` (pure trailing-`%`
prefix), `?r = "iri"`, and `?r IN [...]`; `collect_candidates_via_subject` then gathers
candidates by IRI — a prefix-filtered walk of `defined_iris` (metadata, no body paging)
for `LIKE`, or direct `resolve` for `=`/`IN`. The WHERE re-applies every condition, so
pushdown only ever pre-filters (never drops a row); proven by
`subject_pushdown_equals_scan` (pushdown result == full-scan+filter). Not applied to
negated patterns (would change `NOT` semantics).

**Still open (lower priority — not on the patent demo's hot path):**
- **Untyped `MATCH ?r {}` with a non-subject or non-pushable WHERE** still falls back to
  the full scan (`collect_candidates_via_scan`). Could push down property predicates via
  the value/triple index when the brace carries a concrete predicate.
- **Large subclass trees** — a typed `MATCH ?x : C` where `C` has a huge subclass tree
  (e.g. a UMLS semantic type ≈ 50k concept-class subclasses) does
  `class_with_subclass_closure` + one `scan_chain(is_a, sub)` per subclass → O(subclasses).
  Wants a transitive `subclass_of` index, or accept the inherently large result.
- **Per-candidate property clone** in the indexed path (`r.properties().clone()` per row)
  — project only the requested properties.

## Short-name resolution (its own task — NOT the patent-demo bottleneck)

`resolve_name` (`pattern.rs`), `resolve_short_name_to_query_class` + `resolve_property_name`
(`type_check.rs`) resolve a **bare short name** by `iter_all_resources` full-scan, at both
compile and execute, once per short-name reference. The demo queries use full IRIs so they
don't hit this — but any short-name query would pay O(chain) × refs.

The right scope (per design discussion): **short names need only resolve against the
classes/properties of imported namespaces** (the ontology vocabulary), NOT the whole KG —
so a global `short_name` value index is the wrong tool (it would index 281k UMLS
`short_name`s like `C0000005` and risk false matches). Open design question: today the
query `Name` AST is only `ShortName | FullIri` (no prefix), and `USING` imports *class
IRIs*, not namespaces, and the resolvers ignore `USING` entirely. So scoping short-name
resolution to imports needs an import/namespace-declaration mechanism to scope against.
**Task:** decide that mechanism, then resolve short names within imported-namespace schema
only.

## Next actions (priority order)

1. ~~ESL compile~~ ✅ done.
2. ~~EigenQL untyped-match-all subject pushdown~~ ✅ done.
3. **Short-name resolution scoping** (design + impl) — see section above.
4. **`inspect`/health count**: sum `LayerHandle.resource_count` instead of materialising.
5. EigenQL secondary items (property-predicate pushdown for untyped patterns; transitive
   subclass index for large class trees; per-candidate projection).
6. Review the 🔎 sites under real demo workloads.
