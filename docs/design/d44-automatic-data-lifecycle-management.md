# D44 — Automatic Data Lifecycle Management

**Status:** Stub / reserved. Placeholder for the policy layer that decides *when* lifecycle operations fire — consolidation (D25), garbage collection (D23), retrieval-index cache eviction (D43), trace pruning (D9). The mechanisms exist; D44 unifies the trigger policy and ties it to observable system state.

**Phase:** TBD. Production-observation-driven. The policy decisions in this document should be informed by measured behavior on real workloads, not speculated up-front. Writing D44 substantively before we have meaningful operational data would lock in shapes for problems we don't yet understand.

**Companion docs:**

- [D23 — Out-of-Core Layer Architecture](d23-out-of-core-layer-architecture.md) — reachability-based GC primitive (§5.7), per-layer Bloom infrastructure
- [D25 — Chain Consolidation](d25-chain-consolidation.md) — the consolidation primitive (trigger today is manual operator action)
- [D43 — Text and Vector Retrieval in EigenQL](d43-text-and-vector-retrieval.md) — retrieval-index consolidation is correctly handled in §2.8, but cadence policy lives here; SegmentCache / TermCache / DocsCache / embedding-cache eviction policies live here
- [D9 — NbE/Executor Unification](d9-nbe-unification-and-type-extensions.md) — trace pruning (proofs-as-programs derivability)
- [D21 — Task Traces and Checkpointing](d21-task-traces-and-checkpointing.md) — trace pinning constraints that GC and pruning policies must respect

---

## 1. Motivation

Eigenius has well-designed primitives for shaping its data on disk: garbage collection drops unreachable layers (D23 §5.7); chain consolidation collapses dense layer ranges into resolve-equivalent single layers (D25); segment caches age entries out (LRU); trace pruning removes derivable-from-program traces (D9). Today these primitives are triggered manually (operator action) or by per-component defaults.

At production scale, three patterns will emerge that those defaults won't handle gracefully:

1. **Consolidation lag.** High-cadence workloads accumulate per-layer index entries faster than queries can amortise. Eventually query latency degrades; the operator manually consolidates; the cycle repeats. An automatic policy maintains query health without operator burden.
2. **GC backlog.** Unreachable layers accumulate as branches are abandoned and tags retire. Until GC runs, they consume storage and pollute compaction. An automatic policy reclaims space at appropriate cadence.
3. **Inter-policy interaction.** Consolidation, GC, and cache eviction are not independent — consolidating a layer changes what GC can reclaim; GC affects which layers caches should be evicting first; trace pruning depends on reachability that GC modifies. A coherent policy layer reasons about them together.

D44 is *not* about adding new lifecycle mechanisms. All the mechanisms exist. It is about the *trigger policy* — when each mechanism fires, how triggers are configured, and how the policies interact.

## 2. Scope (sketch)

- **Automatic chain consolidation.** Trigger conditions (chain density, layer count along a head, query-latency degradation observed on retrieval workloads); range-selection policy; operational visibility (preview, dry-run, cancellation).
- **Automatic garbage collection.** Trigger cadence (time-based, storage-based, reachability-change-based); interaction with trace pinning (D21) and tag-rooted reachability (D34 tag GC roots); operator-defined retention windows.
- **Cache eviction coordination.** Shared eviction budgets across cache families — D23 ARC cache (resource content), D43 SegmentCache (vector segments), D43 TermCache / DocsCache (text postings), D43 embedding cache (§5.3). Conditions under which caches should age faster (post-consolidation, post-GC, memory-pressure events).
- **Trace pruning.** When derivable-from-program traces should be removed (D9 §pruning); how that interacts with task resumption (D21); IO-trace retention rules (D21 positional-keying interaction).
- **Index-side maintenance.** Vector-index strategy upgrades (when a `strategy: auto` segment crosses the HNSW threshold during sweep / consolidation); per-property reindex policies for model upgrades (D43 §5.7); deferred vector-embedding sweep scheduling.

## 3. Open questions (sketch)

- **Observable signals.** What are the observable signals that should trigger each policy? Layer count? Query-latency degradation? Storage size? Operator-defined SLOs? Per-tenant vs. global? Should signals be sampled or stream-aggregated?
- **Implementation locus.** Should the policy layer be a kernel-resident component, an institution registered through D14, or an orchestrator-side scheduler? The orchestrator already owns durable task execution (D6) — a natural fit for periodic policy work — but the kernel owns the lifecycle primitives.
- **Configuration surface.** How do users configure the policies — per-context, per-deployment, global defaults with overrides? Does ESL grow lifecycle-policy declarations?
- **Manual override and lockouts.** How do automatic policies interact with manual operator action? Maintenance-window lockouts? Pause/resume semantics?
- **Auditability and reversibility.** Policy decisions are themselves derivations; making them observable and reasoning about them belongs to the same machinery the platform uses for every other derivation. But a policy that consolidates aggressively might invalidate a recovery scenario; reversibility deserves explicit treatment.
- **Resource budgets.** Should policies be bounded by overall resource budgets (CPU, IO, memory) so they don't starve the foreground workload? Coordination with D42 buffer-pool budgets?

---

*This document is reserved as a stub. Substantive content will be added when production workload data informs the policy decisions. Pre-emptive specification here would lock in shapes we don't yet understand. Until then, the underlying primitives (D23 GC, D25 consolidation, D43 retrieval caches, D9 trace pruning) operate under their existing manual-trigger or per-component-default behavior.*
