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
| `query/evaluate/*` (`fiber.rs`, `pattern.rs`, `similarity.rs`), `query/type_check.rs` | EigenQL evaluation | ⚠️ **the `query` RPC ~8.2s on the UMLS chain (measured).** Mix of inherently-all-resources MATCH (🔵) and find-by-type that could be index-driven (🔎). Needs per-operator review — start with the patterns the demo actually runs. |
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
- `load` (ESL compile): **~7 s** ← `collect_ctors`/`collect_macros` full scans. **Open.**
- `query` (EigenQL): **~8.2 s** ← evaluator chain scans. **Open.**
- `run_program_by_iri`: ~19 s, dominated by an ~11 s LLM call (not a chain scan).

## Next actions (priority order)

1. ~~ESL compile~~ ✅ done — `collect_ctors`/`collect_macros` now use
   `resolve_typed_resources` (index + `pending`, per-layer-storage aware). Re-measure
   the `load` RPC to confirm the ~7s is gone.
2. **EigenQL evaluator**: make find-by-type pattern matching index-driven (the value /
   triple index); keep genuine full scans full. Kills the ~8.2s query.
3. **`inspect`/health count**: sum `LayerHandle.resource_count` instead of materialising.
4. Review the 🔎 sites under real demo workloads.
