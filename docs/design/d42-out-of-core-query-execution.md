# D42 — Out-of-Core Query Execution

**Status:** Stub / outline. Placeholder for the Phase 16 deliverable previously sketched as "D24 — Out-of-Core Query Execution" in [`implementation-plan.md`](implementation-plan.md). The D24 number was claimed by [D24 — Schema Versioning Policy](d24-schema-versioning.md); this document picks up the operator-spill spec under its proper number.

**Phase:** 16

**Builds on:** [D23 — Out-of-Core Layer Architecture](d23-out-of-core-layer-architecture.md) (storage backend abstractions, the per-layer indexes that make the read side memory-bounded), [D2 — EigenQL v1 Specification](d2-eigenql-specification.md) (the operator surface this work makes spillable; semantics preserved).

**Companion:** [D43 — Text and Vector Retrieval in EigenQL](d43-text-and-vector-retrieval.md) (independent EigenQL evolution; no shared structural commitments beyond both touching the operator pipeline).

---

## 1. Motivation

Phase 14 lifted the *read-side* working-set bound from "graph size" to "cache size" via topology/content split, layer-aware indexes, and the bounded ARC cache. The *operator side* still holds full intermediate result sets in memory: hash joins build complete in-memory hash tables, `ORDER BY` collects everything before sorting, `GROUP BY` accumulators grow without bound. Phase 16 closes that gap so the operator pipeline tolerates result sets larger than memory.

Until this lands, queries that build large hash tables (joins on multi-million-row sources) or sort large result sets (`ORDER BY` over 10M+ rows) OOM at operator time even though the backing store is happy. The gap is real but not user-blocking until the first OOM — typical workloads continue working after Phase 14.

## 2. Buffer pool abstraction

TBD. Memory-bounded byte-buffer pool that operators allocate from; pool spills to disk via the same RocksDB instance the layer store uses (or a dedicated scratch backend — open question, see §9). Operator-facing API, lifecycle, eviction policy.

## 3. Hash join with spill

TBD. Partitioned hashing; spill a partition to disk when it exceeds the per-operator memory budget; recursive partitioning if a single partition is still too large after spill. Interaction with the indexed-read path from D23 §5.9 (POS-narrowed candidate sets may already be small enough to keep in memory; spill only kicks in for the large case).

## 4. External merge sort

TBD. Classic external merge sort for `ORDER BY` and sort-merge joins on result sets larger than memory. Run generation, k-way merge, tie-breaking on Resource IRI for determinism.

## 5. Spillable group-by

TBD. Spillable group-by hash table with per-group state spilled to disk. Interaction with the aggregation functions specified in D2 §3 (COUNT, SUM, AVG, MIN, MAX) — running aggregates can spill the per-group state cheaply; AVG and similar need both sum and count carried through the spill format.

## 6. Spill-aware cost model

TBD. The EigenQL planner considers spill cost when ordering joins and choosing operators. Doesn't have to be sophisticated — a simple cardinality estimator plus spill-aware cost is sufficient. Interaction with the per-layer triple index statistics (`IndexStats` from Phase 14h).

## 7. Per-query memory budget

TBD. Per-query memory budget (default and override), with a process-wide cap that prevents a single query from starving others. Open question: per-query vs. per-session budget; global vs. per-session buffer pool (§9).

## 8. Lifecycle of spill artifacts

TBD. Cleanup on query cancel or session crash — the resume-sweep analogue from D21. Spilled files must not survive a kernel restart, must not survive `CancelTask`, and must not be reachable as garbage after a normal end-of-query.

## 9. Open questions

- **Spill granularity:** per-operator spill files (clean per-operator lifecycle) or shared spill region (better space utilisation)?
- **Concurrent queries:** memory budget per query or per session? Buffer pool global or per-session?
- **Spill encoding:** CBOR (matches Eigon-JSON, debuggable) or a tighter format (smaller, faster)?
- **Spill backend:** reuse the layer-store RocksDB (one DB to manage, share cache budget) or a dedicated scratch backend (clean separation, no contention with layer reads)?
- **Garbage on cancel:** how do we ensure spilled files are cleaned up if a query is cancelled or a session crashes mid-query? Resume sweep covers tasks; queries need the same.

---

*This is a stub. Sections 2–8 will be filled in when Phase 16 enters active design. The outline matches the Phase 16 deliverable list in [`implementation-plan.md`](implementation-plan.md) §"Phase 16 — Out-of-Core Query Execution" so the planning text and spec stay aligned.*
