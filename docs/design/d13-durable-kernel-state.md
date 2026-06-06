# D13: Durable Kernel State

*Design document for the Eigenius project — April 2026*

**Status:** Implemented (Phase 9a)
**Required before:** Phase 9a implementation
**Depends on:** D4 (storage key encoding), D6b (trace schema), D10 (institutions), D12 (WASM)
**Companion to:** D11 (codata + resumable execution)

---

## 1. Motivation

The kernel builds on a carefully layered, typed, content-addressable data
model. None of that matters at runtime today, because **the running
kernel keeps its entire state in memory**. A restart throws away every
capability install, every loaded layer, every trace, every institution
registration. The embedded core ontology reloads fresh on each startup,
not because we want it that way but because there is no alternative.

This document specifies how to make the kernel durable end-to-end:
layers persist, resources persist, traces persist, and WASM
capabilities — including institutions — survive restarts. D11 assumes
this foundation exists; this document says how to build it.

The scope is deliberately narrow: **the kernel's long-term
state**. It does not address:

- Deployment (Phase 13)
- Horizontal scaling (TiKV backend, Phase 13)
- Observability (Phase 13)
- Codata type theory or streams (D11)
- Concurrent task scheduling (D11)

---

## 2. Today's state (honest inventory)

A close reading of `start_server`, `bootstrap::bootstrap`, and
`EigeniusService::with_components`:

| Entity | Where it lives today | Restart loss |
|--------|----------------------|--------------|
| Core / program / reflection / institution ontologies | Embedded JSON via `include_str!`, loaded into in-memory layers | Rebuilt fresh from embedded bytes |
| User-loaded layers (via `Load` RPC / `eigenius load`) | In-memory `Arc<Layer>` chain rooted in the institution layer | **Total loss** |
| `ExecutionContext` working layer | `LayerBuilder` inside the service's `RwLock` | **Total loss** |
| Trace store | `InMemoryTraceStore` | **Total loss** |
| WASM component registry | `ComponentRegistry` in `Arc<RwLock<Arc<…>>>` | **Total loss**, but binaries are inline in layer resources → recoverable if layers persist |
| Institution registry | `InstitutionRegistry` in `Arc<RwLock<…>>` | **Total loss** (same recovery caveat) |
| Orchestrator client | `Arc<Mutex<…>>`, lazy gRPC client | Re-establishes automatically |

The storage crate (`storage/rocksdb`) implements the `LayerStore` +
`ResourceStore` traits with the key encoding from D4, and the CLI has a
standalone `db` subcommand that operates on a RocksDB file directly.
Neither the live kernel server nor the bootstrap sequence uses it.

---

## 3. Goals and non-goals

**Goals:**

- `eigenius serve --db <path>` persists every committed layer and the
  resources in it to a RocksDB store; subsequent restarts reconstruct
  the `ExecutionContext` from that store.
- On first run against an empty store, the kernel writes the four
  embedded ontology layers (core → program → reflection → institution)
  to the store exactly once, then treats the store as authoritative.
- On subsequent runs, the embedded ontology is **not** re-bootstrapped
  — the persisted versions win. We detect drift and refuse to boot
  rather than silently override.
- Traces persist. A re-run of the same program hits cached traces
  without re-invoking components. (This is the enabling primitive for
  D11's resumable execution.)
- WASM capabilities and institutions survive restarts: on boot, the
  kernel scans every persisted layer and re-registers any
  `implementation: wasm` resources with the component / institution
  registries.
- In-memory mode (no `--db`) still works for tests and ephemeral
  runs. Same code paths, different storage.

**Non-goals:**

- Migration tooling beyond "refuse to boot on drift." Real schema
  migrations are deferred.
- Any storage backend other than RocksDB. TiKV is Phase 13.
- Orchestrator-side durability. The orchestrator's binary cache is
  separately managed by D12b's compiled-component cache.
- Streaming reads, incremental indexing, or any query-performance
  work. This document is about correctness under restart, not speed.

---

## 4. Startup sequence

### 4.1 `eigenius serve` with `--db <path>`

```
1. Open RocksStore at <path>. Create if missing.
2. Check store.get_head():
     Some(head_id) → RESUME
     None          → SEED
```

### 4.2 SEED path (fresh DB)

1. Run `bootstrap::bootstrap()` as today: produce the four embedded
   ontology layers and their `ExecutionContext`.
2. For each layer (core → program → reflection → institution) in
   order: `store.put_layer(layer)`, which writes the layer's metadata
   (id, name, parent_id) and every `(iri → resource)` pair under the
   layer-local keyspace (D4 §4).
3. `store.set_head(institution.id)` atomically.
4. Record a **seed manifest**: a small key
   (`meta:seed_manifest`) containing the SHA-256 of each embedded
   ontology's JSON bytes. This is what we check on RESUME to detect
   drift.

### 4.3 RESUME path (existing DB)

1. Load `store.get_head()` → layer id.
2. Walk parent links via `store.get_layer(id)` until the root. Build
   `Arc<Layer>` for each, chained by parent. This is the existing
   `LayerStore::load_chain` pattern.
3. Compare the stored seed manifest against the current embedded
   ontologies' SHA-256. Three cases:
   - **Match:** normal boot, continue.
   - **Mismatch:** refuse to boot. Print a clear error naming which
     ontology drifted and point at a migration procedure (v1: reseed
     on a fresh path). See §8.
   - **Missing (legacy DB without a manifest):** same as mismatch.
4. Construct `ExecutionContext::new(institution_layer, "persistent",
   ReadWrite)`.
5. Re-register WASM capabilities. See §6.
6. Re-register institutions. See §7.

### 4.4 No `--db` flag (current behaviour)

Identical to today: in-memory layers, `InMemoryTraceStore`, no
persistence. Existing tests and CI runs are unaffected.

---

## 5. Commit flow (writing through)

`ExecutionContext::commit` today builds a new `Arc<Layer>` from the
working `LayerBuilder` and rotates `head`. To make commits durable:

1. Build the new layer as today (in-memory first — we want to
   validate, canonicalise, and compute the layer id before persisting).
2. **If a `LayerStore` is attached**: `store.put_layer(&new_layer)`
   then `store.set_head(new_layer.id)`. Both calls are synchronous
   within the commit critical section — a commit either persists or
   returns `ContextError::PersistenceFailed`.
3. Rotate the in-memory head.

This makes `Load` RPC inherently durable: `load → add_resource* →
commit → put_layer → set_head`. The kernel never exposes a moment
where an acknowledged commit could be lost on restart.

`LayerStore::put_layer` is idempotent by layer id (content-addressed).
Re-submitting identical resources is a no-op.

---

## 6. WASM component re-registration

Today, the scan-and-register machinery runs inside the `Load` RPC
([kernel/src/server/mod.rs `register_wasm_from_layer`](../../kernel/src/server/mod.rs)).
It reads capability resources from a newly-built layer, extracts the
base64-encoded `wasm_binary`, compiles it via wasmtime, and inserts
the result into the component or institution registry.

To make this survive restarts:

1. During RESUME, walk the persisted layer chain from root to head.
2. For each layer, run the same `scan_and_register` logic the `Load`
   handler uses — but against an already-committed layer, not a
   builder.
3. For IO-capability components, the orchestrator-side registration
   path (D12b §4) runs the same way it would on a fresh install:
   forward the binary via `RegisterWasmComponent` gRPC.
4. Tie-in with [issue #11](https://github.com/eigenius/eigenius/issues/11)
   (orchestrator auto re-registration): the same scan-and-forward
   logic that runs during RESUME can be invoked on demand when the
   orchestrator reconnects. Issue #11 proposes an `OrchestratorHello`
   gRPC (not yet implemented) for the orchestrator to announce itself;
   on receipt the kernel would re-run step 3 for every persisted IO
   component. Same code path, different trigger.

The WASM binary is always recoverable because it was committed inline
as a base64-encoded property on the capability resource. The
compiled-component cache from D12b §7 (sha256-keyed `.cwasm` files) is
an independent speed optimisation: deserialise if the binary hash matches
a cached entry, otherwise compile fresh.

---

## 7. Institution re-registration

Institutions layer a twist on top of §6. `InstitutionRegistry::register`
does **two** things ([kernel/src/institution/mod.rs:108](../../kernel/src/institution/mod.rs#L108)):

1. Inserts the reasoner into the dispatch tables.
2. **Returns a `Vec<Resource>`** of the institution's declared morphism
   types, query types, and structural properties. Today the server
   silently discards this — tracked as [#15](https://github.com/eigenius/eigenius/issues/15).

For durable state we must fix #15 as part of this work:

- On the initial `capability install`, the returned `Vec<Resource>`
  must be committed to the currently-building layer (via
  `ExecutionContext::add_resource` before the commit).
- On RESUME, those declared classes are already in the persisted
  layer — we do not publish them again. We do re-instantiate the
  reasoner (compile + register the WASM binary) and rebuild the
  dispatch tables from the class resources we find in the layer.

Concretely: the RESUME scan (§6 step 2) distinguishes "this resource
represents a WASM component" from "this resource represents a WASM
institution" by `is_a` tag, and calls the appropriate registration
path. Component → `ComponentRegistry`. Institution → `InstitutionRegistry`
+ recover the morphism/query dispatch maps by reading the class
resources that are already in the layer.

---

## 8. Drift and migration

**The core ontologies are embedded via `include_str!`.** If they
change between releases — a new property on `Class`, a renamed
constant — the kernel MUST refuse to boot against a DB that was
seeded with the old versions. Silent auto-update would corrupt
downstream resources typed against the old schema.

**v1 policy** (this doc):
- SHA-256 of each embedded ontology's JSON is compared to the
  persisted seed manifest.
- Any mismatch is a hard error with an actionable message:
  ```
  eigenius serve: seed manifest drift
    core-ontology: persisted sha = <a>, embedded sha = <b>
  Refusing to boot against a DB seeded with a different core ontology.
  Options:
    1. Use a fresh DB path (you'll re-install your capabilities).
    2. Run `eigenius db migrate --from <a> --to <b>` (not yet implemented).
  ```
- No `eigenius db migrate` in v1. When we need it, that's its own
  document.

**v2 (future work):** proper migration as a sequence of
content-addressable ontology layers, each deriving from the previous,
with the kernel walking the chain and producing a new DB state.
Out of scope here.

**Deferred direction — reconciliation through the institution lens.**
Ontology migration in Eigenius is naturally a *layer reconciliation*
problem: two layers define overlapping concepts, and we want to know
when a resource typed against one layer can be safely retyped against
the other. D10 already gives us the vocabulary — layers form a
category, morphisms between layers are structure-preserving mappings,
and comorphisms (see D10 §7) translate between institutions. A mature
migration story therefore looks less like database schema migration
and more like "prove that a co-morphism exists between the old layer
and the new one, and use it to rewrite persisted resources." That's
the direction a future migration doc should take. v1's drift-refusal
is the cautious stopgap until we do the category-theoretic work
properly.

---

## 9. Trace store

`InMemoryTraceStore` → `RocksTraceStore`. D6b already specifies the
trace schema; D11 specifies how resumable execution uses it. This
document only says:

- The server's `TraceStore` instance comes from the same `RocksStore`
  that backs layers (same DB, separate column family per D4).
- Trace writes are transactional with layer commits when a commit's
  effects include new trace resources (D6b §5).
- No trace-store-specific API surface changes; the existing
  `TraceStore` trait is already shaped for both backends.

---

## 10. Proposed implementation ordering

Five pieces, each shippable independently (approximate sizes):

1. **DB wiring on `serve`** (~2 days). `--db` flag, open `RocksStore`,
   thread it into `EigeniusService`. In-memory fallback preserved.
2. **Seed + manifest** (~1 day). First-run detection, commit embedded
   ontologies, write seed manifest. Drift-refusal on mismatch.
3. **Commit-through** (~1 day). `ExecutionContext::commit` writes to
   store before rotating the in-memory head. `Load` RPC becomes
   durable automatically.
4. **WASM + institution re-registration on RESUME** (~2 days). Fix
   [#15](https://github.com/eigenius/eigenius/issues/15) along the
   way (institution's published classes commit to the layer, not
   just to the registry). Integration test: install → restart →
   re-dispatch.
5. **Persistent trace store** (~1 day). Swap `InMemoryTraceStore`
   for `RocksTraceStore`. D11-driven; included here for
   completeness.

Total: ~1 week of focused work. Each piece lands with its own test
and can be reviewed independently.

---

## 11. Open questions

- **Trace store lifecycle.** Traces from programs that were running
  when the kernel crashed — do we keep them (for resumable execution
  per D11) or garbage-collect them? v1: keep them; D11 will decide
  the GC policy.
- **Sessions, not writers.** A kernel instance is effectively a single
  session against a DB. The `--db` flag should document that two
  kernel processes pointed at the same path is not a supported mode —
  not because RocksDB would silently corrupt (it wouldn't; it refuses
  concurrent opens) but because the data model doesn't yet know how
  to reconcile divergent layer chains from parallel sessions. D11's
  concurrent task model is multiple tasks *within* one session.

  Multi-session — where N independent kernel instances work against
  shared storage and then rejoin — is a natural follow-on, and it's
  the place where layer reconciliation stops being a migration-only
  concern and becomes a first-class feature. Two sessions produce two
  layer-chain branches; making them merge requires exactly the
  category-theoretic vocabulary §8 pointed at: morphisms between
  layers, comorphisms between institutional views, and a policy for
  when two concurrent extensions of the same parent layer can be
  fused rather than chosen between. Deferred until we need it.
- **Orchestrator client persistence.** The orchestrator endpoint is
  passed as a CLI flag. Should it be persisted in the DB so a
  restart picks up the same orchestrator without the user re-specifying
  it? Leaning no — the kernel↔orchestrator binding is deployment
  configuration, not kernel state.
- **Layer pruning.** If a user `load`s a huge ontology and then
  decides they don't want it, there's no way to remove a layer —
  layers are immutable in the chain, and we'd need a "supersede"
  concept. This is a design gap regardless of persistence;
  persistence just makes it observable. Deferred.

---

## 12. Integration into the phase plan

The implementation plan splits durable kernel state and
stream/resumable-execution work into two paired milestones of one phase:

- **Phase 9a (this document):** durable layers, seeded bootstrap,
  persistent traces, institution + WASM re-registration on RESUME.
  Prerequisite for 9b. ~1 week.
- **Phase 9b (D11):** codata EigenTT extension, stream observations,
  concurrent task model, resumable execution on top of the now-durable
  trace store. ~4–6 weeks.

This split preserves D11's conceptual coherence while acknowledging
that the plumbing in 9a is prerequisite and separately reviewable.
Deployment and operations (Phase 13) treat durability as a given; the
correctness-hardening phases (10) and type-theory extensions (11) can
run in parallel with 9b once 9a has landed.
