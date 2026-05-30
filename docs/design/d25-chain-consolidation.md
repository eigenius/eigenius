# D25 — Chain Consolidation

**Status:** Implemented (Phase 17; `eigenius db consolidate` + notebook Compaction destination)
**Phase:** 17
**Supersedes:** the deferred "deep chain" performance concern raised in D23 §5.2.7
**Companion docs:** D23 (out-of-core layer architecture; the per-layer bloom resolve walk this phase reduces), D20 (layer reconciliation; the resolution decisions a v2 multi-parent consolidate must preserve), D21 (task traces and checkpointing; the pinning semantics this phase must respect), D13 (durable kernel state; consolidation does not modify the seed manifest)

## 1. Summary

Long-lived databases accumulate layer depth. A notebook session produces 200 small commits while iterating on a derivation, and the resulting chain stays 200 layers deep forever. Reads pay a 200-layer bloom-walk per resolve; storage holds 200 small `topo:` / `bloom:` / `layer:` entries that GC won't reclaim because every layer in the range is still reachable via branch pointer or trace pin. This is the structural problem D23 §5.2.7 flagged as deferred.

Phase 17 introduces **chain consolidation**: an explicit operator action that collapses a contiguous ancestral range of layers `[from..to]` into one *resolve-equivalent* layer with `parent = from.parent` and content equal to the per-IRI top-of-stack values across the range. The 199 collapsed layers do not disappear at consolidation time — they remain in storage until GC sweeps them — but they become unreferenced from the head of the affected branch, and the consolidated layer takes their place in the chain.

The structural commitment is **resolve-equivalence under head substitution**: if `L_c` is the consolidated layer for `[from..to]`, and `L_c` is substituted at the head of the affected branch in place of `to`, then for every IRI `i` and every layer `L_h ≥ L_c` in the rewritten chain, `L_h.resolve(i)` produces the same value it produced in the original chain. The substitution is deliberately not transparent: resolves rooted at a layer *between* `from` and `to` (time-travel reads via D21 §3.6 against an intermediate layer) lose access to the consolidated-out layers. Phase 17's contract covers head-rooted resolves; intermediate-layer reads are addressed by the trace re-pinning policy (§7).

This is the "git squash" analog at the typed-knowledge-graph level. It is distinct from merge (which combines parallel branches; doesn't reduce depth) and from GC (which removes unreachable layers; doesn't restructure reachable ones).

This document specifies the consolidation API, the top-of-stack computation, the trace re-pinning policy that makes consolidation safe, the interaction with Phase 15 merge layers (v1 forbids; v2 sketches), the bloom-cache eviction discipline, and the sub-milestone sequencing for an implementation that lands incrementally on top of Phase 14 and Phase 15.

## 2. Motivation

A typical Eigenius chain accumulates layer depth in three regimes:

- **Notebook iteration.** A user evaluates 50 cells over a session, each producing a commit; the next session adds another 50; over a project's lifetime the chain reaches the 10³–10⁴ range.
- **Long-running tasks.** Phase 9b's task model produces a layer per `components:Checkpoint`, plus one per externally observable IO step. A multi-day batch run produces layers proportional to the work done.
- **Multi-author refactoring.** Each merge resolution from Phase 15 produces a multi-parent layer; a project with regular merges accumulates depth even when individual contributions are small.

D23 §5.2's per-layer-bloom resolve design pays an in-memory hash per layer in the chain. At 10² layers the cost is negligible (~µs). At 10⁴ layers it is in the millisecond range *per resolve*, and a single EigenQL query may issue thousands of resolves. The performance degradation is gradual but real.

D23 §5.2.7 sketched a roll-up index as an in-place mitigation — periodically materialise an index of "which layer in the recent past defines IRI X" — and deferred. The roll-up speeds resolve but doesn't reduce storage cost (small `bloom:` entries still litter the column family) and doesn't shorten the chain itself. It is the wrong tool for "I have 200 commits from yesterday's session and I want them gone."

Consolidation addresses the storage-cost-and-chain-depth problem directly. Reads against the post-consolidation chain pay a 1-layer bloom check where they previously paid 200; storage reclaims the small entries at the next GC pass. The roll-up index remains a valid future optimisation orthogonal to this phase.

The decision to ship consolidation as an *explicit* operator action rather than a background policy reflects Eigenius's epistemic posture: layer history matters. A user who explicitly requests consolidation has weighed the trade-off; a background process making the same decision silently has not. The auto-consolidation policy question (§12) is intentionally deferred.

## 3. Goals and non-goals

**Goals:**

- A typed `consolidate_chain(from, to, opts) → LayerId` API. Linear ancestral range only in v1. Atomic commit producing a content-addressed layer.
- Resolve-equivalence under head substitution: every IRI in the consolidated range has the same resolved value before and after, when read from the head of the affected branch.
- Trace-pin safety: refuse to consolidate ranges that would invalidate trace pins. v1 ships the conservative refusal policy; re-pointing and invalidation are v2.
- Coexistence with Phase 14's branch DAG and Phase 15's merge layers. v1 refuses to consolidate ranges that contain a merge node; v2 sketches multi-parent consolidation.
- Bloom-cache eviction for consolidated-out layers; the consolidated layer gets a fresh bloom at commit time.
- CLI and gRPC surfaces for explicit invocation, with cost estimation before commit.

**Non-goals:**

- Auto-consolidation policy. The kernel never picks a range to consolidate on the user's behalf; v1 ships explicit-only.
- Multi-parent consolidation across merge nodes. v1 refuses; v2 sketched in §8.
- Distributed consolidation coordination across multiple kernel instances on the same DB; Phase 14's single-`serve`-per-DB constraint applies.
- Trace-pin re-pointing or invalidation; v1 refuses to consolidate trace-pinned ranges. The re-pointing/invalidation policies are sketched in §7 as v2 work.
- Storage-level reclamation beyond what the existing GC sweeper provides. Consolidation marks layers unreferenced; GC does the actual byte reclamation as a separate operation.
- Roll-up indexes for resolve performance (D23 §5.2.7's deferred mitigation). Orthogonal to consolidation; can land independently.

## 4. Theoretical foundation

The framing is intentionally lighter than D20's. Consolidation is a strictly weaker structural operation than merge: it operates on a *linear* range with no span to combine, no pushout to compute, no universal property to preserve beyond the per-IRI top-of-stack identity.

**Resolve as a function.** For a chain `L_0 ◁ L_1 ◁ … ◁ L_n` (where `◁` is the parent relation, `L_n` the head), `resolve_chain(L_n, i)` returns the value of IRI `i` at the topmost layer in the chain that defines `i`, or `⊥` if no layer defines `i`. The kernel's existing `Layer::resolve` walks the chain head→root via the per-layer-bloom skip pattern (D23 §5.2.2).

**Top-of-stack over a range.** For a range `[from..to]` with `from ◁* to` (`from` is an ancestor of `to`), define `tos(range, i)` as `(L*).resolve(i)` where `L*` is the topmost layer in the range that defines `i`, or `⊥` if no layer in the range defines `i`. This is the value the consolidated layer claims for `i`.

**The consolidation invariant.** Let `L_c` be the consolidated layer for `[from..to]` with `parent(L_c) = parent(from)` and `defined_iris(L_c) = ⋃ {defined_iris(L) : L ∈ [from..to]}`. For each `i ∈ defined_iris(L_c)`, `L_c` stores `tos([from..to], i)`. After substituting `L_c` at the head of the original chain (replacing the chain segment `[from..to]` with `L_c` only), the following holds:

> **For every layer `L_h` at or above the substitution point (`L_h = L_c` or `L_c ◁* L_h`) and every IRI `i`, `resolve_chain(L_h, i)` produces the same value it produced in the original chain.**

This is the structural specification of "what consolidation must preserve." A consolidated layer that violates the invariant has lost information that head-rooted resolves depend on. The consolidation algorithm in §6 is straightforward to verify against this invariant by induction on the range walk.

**What the invariant does not preserve.** Resolves rooted at a layer *between* `from` and `to` in the original chain — time-travel reads against an intermediate layer — are not covered. Such reads, after consolidation, must either (a) re-point at `L_c` if the desired IRI's value is preserved there, (b) be rejected with a typed `LayerConsolidatedOut` error, or (c) be prevented by refusing to consolidate the relevant range. v1 ships (c) as the trace-pin refusal policy (§7); v2 may relax to (a) for unpinned time-travel reads with a `LayerConsolidatedOut` fallback.

**Content addressing.** The consolidated layer's `LayerId` is computed by the existing layer-hashing rules (D1 + D23) over its content + parent pointer + commit metadata. Two independent consolidations of the same range against the same parent produce the same `LayerId`. Determinism is preserved.

**Atomicity.** The consolidation writes `topo:<L_c>`, `bloom:<L_c>`, `layer:<L_c>:res:*`, and the chain pointer of the affected branch in one `WriteBatch` per D23 §6.3. The original layers stay in place until GC; the consolidated layer is purely additive at commit time.

## 5. Consolidation API

### 5.1 Kernel API

```rust
pub fn consolidate_chain(
    branch: &str,
    from: LayerId,
    to: LayerId,
    opts: ConsolidateOpts,
) -> Result<ConsolidationOutcome, ConsolidateError>;

pub struct ConsolidateOpts {
    /// Cost-estimation cap: if the predicted top-of-stack walk would
    /// exceed this many resource entries, return CostExceedsCap before
    /// computing. Default: a few million; deployment-tunable.
    pub max_walk_entries: u64,
    /// Whether to enforce the trace-pin refusal policy (v1 default true).
    /// Reserved for future v2 re-pointing modes; kept off the wire for v1.
    pub trace_pin_policy: TracePinPolicy,
}

pub enum TracePinPolicy {
    /// v1 default: refuse if the range contains trace-pinned resources.
    Refuse,
    /// v2 (not implemented): re-point pins at the consolidated layer.
    RepointOnConsolidate,
    /// v2 (not implemented): mark pins stale; trace becomes uninspectable
    /// past the consolidation point.
    Invalidate,
}

pub struct ConsolidationOutcome {
    pub consolidated_layer: LayerId,
    pub collapsed_layer_count: u64,
    pub reclaimable_bytes_estimate: u64,
    pub head_advanced: bool,  // true if the branch's head moved
}

pub enum ConsolidateError {
    /// `from` is not an ancestor of `to` in the affected branch.
    RangeNotAncestral { from: LayerId, to: LayerId },
    /// The range contains a merge node (multi-parent layer). v1 refuses.
    RangeContainsMergeNode { merge_layer: LayerId },
    /// The range contains a layer with active trace pins. v1 refuses.
    RangeContainsTracePin {
        pinned_layer: LayerId,
        trace_count: u64,
    },
    /// Predicted walk exceeds `opts.max_walk_entries`.
    CostExceedsCap { predicted_entries: u64 },
    /// Branch ref CAS failed (concurrent advance).
    BranchAdvancedConcurrently { observed_head: LayerId, expected_head: LayerId },
    /// Underlying storage write failure.
    WriteFailed(StorageError),
}
```

### 5.2 gRPC

`ConsolidateChain` RPC parallels the kernel API. Carries the range, the affected branch name, and the `ConsolidateOpts`. Returns `ConsolidationOutcome` or a typed `ConsolidateError`.

A non-mutating `EstimateConsolidation` RPC computes the predicted walk size, collapsed-layer count, and reclaimable-bytes estimate without committing — symmetric with D20's `preview_cascade`. Lets the CLI surface the cost before invoking.

### 5.3 CLI

```
eigenius db consolidate <from-layer-id>..<to-layer-id> --branch <name>
    [--max-walk-entries <n>]
    [--dry-run]
```

`--dry-run` calls `EstimateConsolidation` instead of `ConsolidateChain`; prints the cost estimate and the layer-id of the layer that *would* be produced (deterministic from the range).

```
eigenius db consolidate-summary
```

A diagnostic command listing recent consolidations with their `LayerId`, range, collapsed-count, and reclaimable-bytes estimate. Useful for the operator who's auditing chain health.

## 6. Top-of-stack computation algorithm

```
def consolidate_chain(branch, from, to, opts):
    # Range validation (cheap)
    assert from is_ancestor_of to in branch.chain
    for L in walk(from, to):
        if L is multi_parent: raise RangeContainsMergeNode(L)
        if trace_pin_count(L) > 0 and opts.trace_pin_policy == Refuse:
            raise RangeContainsTracePin(L, trace_pin_count(L))

    # Cost estimation (cheap)
    predicted_entries = sum(len(defined_iris(L)) for L in walk(from, to))
    if predicted_entries > opts.max_walk_entries:
        raise CostExceedsCap(predicted_entries)

    # Top-of-stack walk (head→root in the range)
    seen_iris = set()
    consolidated_resources = {}
    for L in reverse(walk(from, to)):  # to → from, head→root
        for iri in defined_iris(L):
            if iri not in seen_iris:
                consolidated_resources[iri] = L.resolve(iri)
                seen_iris.add(iri)

    # Build the consolidated layer
    L_c = build_layer(
        parent=parent(from),
        defined_iris=seen_iris,
        resources=consolidated_resources,
        commit_metadata=fresh_commit_metadata(),
    )

    # Atomic commit (single WriteBatch per D23 §6.3)
    write_batch.put_topo(L_c)
    write_batch.put_bloom(L_c)
    for iri, value in consolidated_resources.items():
        write_batch.put_resource(L_c, iri, value)
    write_batch.cas_branch(branch, expected_old=branch.head, new_head=L_c)
    write_batch.commit()  # atomic; rolls back on CAS failure

    # Bloom cache eviction for collapsed layers (§9)
    for L in walk(from, to):
        bloom_cache.evict_layer(L)

    return ConsolidationOutcome(...)
```

The walk is **head→root** so that the topmost-defining-layer wins for each IRI by virtue of being seen first. This matches the existing `Layer::resolve` walk direction and reuses the chain-walk infrastructure D23 already provides.

The walk is **single-pass linear** in the total number of `(layer, defined_iri)` pairs in the range. For typical ranges (10²–10³ layers, low thousands of `defined_iris` per layer) the walk completes in low milliseconds. The cost-estimation gate prevents pathological invocations.

The walk does **not** materialise the resources' bodies — it stores references. The atomic commit then writes the resource bodies into the consolidated layer's resource keyspace, which is what makes the reclaimable-bytes estimate meaningful: the consolidated layer's bytes are a fresh, deduplicated copy of the per-IRI top-of-stack values, and the original layers' bytes become reclaimable at the next GC pass.

The consolidated layer's `commit_metadata` records the consolidation operation explicitly: a `consolidation_record: { from: LayerId, to: LayerId, collapsed_count: u64, consolidated_at: Timestamp }` property. This is for audit; the kernel does not consult it for resolve correctness. The presence of this property is also how the diagnostic `db consolidate-summary` command identifies consolidated layers.

## 7. Trace re-pinning policy

This is the load-bearing policy decision. Three options, of which v1 ships the most conservative.

### 7.1 The problem

Phase 9b/D21 trace pins are `(LayerId, Iri)` pairs that anchor a trace step to a specific resolved value. A trace inspector walking back through a `ProgramTrace` follows pins to reconstruct the chain state at each step.

When a pinned `LayerId` falls inside a consolidation range `[from..to]`, that layer is no longer at the head of any branch after consolidation. The pin's `LayerId` still resolves (the layer is still in storage until GC), but the trace's epistemic claim — "this step ran against this layer" — becomes harder to interpret as the chain evolves: the consolidated chain has the *value* at the new head, but the *layer* the pin names is divergent from the canonical chain.

### 7.2 Three policies

**(a) Refuse (v1 default).** Reject `consolidate_chain` calls whose range contains a trace-pinned layer. Returns `RangeContainsTracePin { pinned_layer, trace_count }`. The user must wait for the relevant traces to be pruned (by D21's pruning policy) or explicitly accept losing the traces (by separately deleting them) before consolidating.

Pros: requires no trace-store changes; the conservative semantics; pin meaning is preserved exactly. Cons: long-lived traces block consolidation indefinitely; the user has to manage the timing manually.

**(b) Repoint on consolidate (v2).** When consolidation succeeds, the trace store updates every pin from `(L, iri)` to `(L_c, iri)` for `L ∈ [from..to]` — but only if `L_c.resolve(iri)` equals `L.resolve(iri)`. If a pin's IRI has been redefined later in the range and the topmost-defining-layer's value differs from the pinned value, the pin cannot be re-pointed without changing the trace's epistemic claim.

Pros: traces survive consolidation cleanly when the pinned value didn't change. Cons: requires the trace store to subscribe to consolidation events; pins whose values *did* change still fail and need policy (b) or (c) for those cases; introduces a new failure mode ("partial re-point").

**(c) Invalidate.** The trace store marks all pins whose `LayerId` falls in the range as `stale`. The trace inspector surfaces stale pins as "this trace step ran against a layer that has been consolidated; the value at the time was X, but the layer is no longer canonical."

Pros: traces remain inspectable, with honest provenance about consolidation. Cons: trace store needs a `stale` flag on pins; consumer code (trace inspector, audit tooling) needs to render the stale state correctly.

### 7.3 v1 ships (a)

Refusal is correct, requires no schema changes, and is consistent with the rest of Eigenius's epistemic posture. The cost is operational: long-lived traces will block consolidation of the surrounding range. This cost is known and acceptable for v1.

The path to (b) or (c) in v2 requires:

- A trace-store schema change: pins gain a `consolidated_to: Option<LayerId>` field for (b) and a `stale: bool` for (c).
- A subscription mechanism: consolidation publishes an event the trace store consumes.
- A policy-selection knob on the consolidation call: `TracePinPolicy::RepointOnConsolidate` or `TracePinPolicy::Invalidate`. The `ConsolidateOpts` already carries the field; v1 just refuses anything other than `Refuse`.

D25 v2 will pick the policy based on usage data: do real workflows have long-lived traces that block consolidation often enough to warrant the trace-store coupling? The decision is correctly deferred.

## 8. Interaction with merge layers and resolution decisions

D20 (Phase 15) introduces multi-parent merge layers as the carrier for resolution decisions. A merge layer's content depends on the resolution strategy applied (Witness / Rename / KeepBoth / KeepOne / KeepNeither / Restructure) and, for `Witness`, on a `MergeComorphism` resource.

### 8.1 v1 refuses to consolidate across merge nodes

If a consolidation range `[from..to]` contains a multi-parent layer, `consolidate_chain` returns `RangeContainsMergeNode { merge_layer }`. The user must consolidate the linear segments before and after the merge separately, or wait for v2.

The reason is structural. A consolidated layer subsuming a merge node would need to:

- Carry forward the resolution strategy (so the consolidated chain remains auditable).
- Carry forward the `MergeComorphism` reference if `Witness` was used.
- Carry forward the `CascadeAck` records (D20 §8) that gated the merge commit.
- Decide what its parent pointer is — the merge node's parents (which resurrects the multi-parent shape one layer up), or one specific ancestor (which loses information).

The first three items are tractable but not free. The fourth is the genuine design decision and warrants its own deliberation. v2 sketched in §8.2.

### 8.2 v2 sketch — multi-parent consolidated layers

The natural shape: a consolidated layer that absorbs a merge node has multiple parents (the merge node's parents) and carries a `consolidated_resolutions: List<ResolutionRecord>` property where each record carries:

```rust
pub struct ResolutionRecord {
    pub original_merge_layer: LayerId,
    pub strategy: StrategyKind,
    pub witness: Option<Iri>,  // MergeComorphism IRI if Witness was used
    pub cascade_acks: Vec<CascadeAckSummary>,
}
```

The consolidated layer's `LayerId` is content-addressed over the records (alongside everything else), so v2 consolidation remains deterministic.

The trade-off: a multi-parent consolidated layer doesn't reduce parent-pointer count. It reduces *intermediate* layer count but not topology complexity at the consolidation boundary. For chains where merges are frequent, this is the limit of how much consolidation can compress; for chains where merges are rare (the common case), v1's linear-only restriction is harmless.

### 8.3 Resolution decisions and the pushout invariant

A subtle question: does consolidating the linear segments *before* and *after* a merge preserve the merge's resolution decision?

Yes — by the resolve-equivalence invariant. The pre-merge segment's consolidated layer presents the same per-IRI values to the merge node that the original segment did; the post-merge segment's consolidated layer presents the same per-IRI values to head-rooted resolves that the original segment did. The merge node's resolution strategy operates on values, not on layer identity, so the consolidation is transparent to it.

The invariant only breaks if v2 multi-parent consolidation is allowed and consolidation absorbs a merge node — which is exactly why v1 refuses, and why v2's `consolidated_resolutions` property is load-bearing for auditability.

## 9. Bloom cache eviction

The per-layer bloom cache (D23 §5.2.3) holds bounded `BloomFilter` entries keyed by `LayerId`. After consolidation, the entries for the collapsed range are no longer reached by head-rooted resolves; holding them in memory is wasteful.

Consolidation calls `bloom_cache.evict_layer(L)` for every `L ∈ [from..to]` before returning. This is the same hook GC uses when removing layers; consolidation is just an early eviction trigger.

The consolidated layer `L_c` gets a fresh bloom computed over its `defined_iris` at commit time and inserted into the cache via the standard path. Subsequent resolves benefit from the shallower chain immediately.

For very large consolidations (10⁴+ layers), the eviction loop is part of the synchronous response. This is fast — a few microseconds per eviction — and bounded by the layer count, which is already gated by `max_walk_entries`.

## 10. Worked examples

### 10.1 Notebook session squash

A user runs a notebook session producing 200 commits. After the session, the chain depth is 1200 (was 1000 before the session). The user runs:

```
eigenius db consolidate <session-start-layer>..<session-end-layer> --branch main --dry-run
```

CLI output:

```
Range: 200 layers (cost: 145,000 walk entries, well under cap)
Predicted consolidated layer: urn:eigenius:layer:sha256:abc123…
Reclaimable bytes (after next GC): ~38 MB
```

The user re-runs without `--dry-run`. Consolidation completes in ~200ms. The chain depth is now 1001 (the consolidated layer replaces the 200 session layers). The next GC pass reclaims the 38 MB. Subsequent resolves against the head pay 1001 layers of bloom-walk instead of 1200.

### 10.2 Long-lived production chain

A monthly operator job runs:

```
eigenius db consolidate $(eigenius db chain-tip --offset=-30d)..$(eigenius db chain-tip --offset=-7d) --branch main
```

This consolidates the 23-day window between 30 days ago and 7 days ago, leaving the most recent week unconsolidated (recent layers are likely still being read against; consolidation provides less benefit). The trace-pin refusal policy may reject this call if active traces still pin layers in the consolidated window; the operator job catches the typed error and either waits for trace pruning or invokes the prune-and-consolidate sequence explicitly.

This is the auto-consolidation policy question (§12) made concrete. v1 ships explicit-only because the right cadence is workload-dependent and operators should choose deliberately.

### 10.3 Trace-pin refusal (negative path)

A user attempts to consolidate a range, and the kernel returns:

```
ConsolidateError::RangeContainsTracePin {
    pinned_layer: urn:eigenius:layer:sha256:def456…,
    trace_count: 3,
}
```

The CLI prints:

```
Cannot consolidate: layer urn:eigenius:layer:sha256:def456… is pinned by 3 active traces.
Use `eigenius traces list --pinned-layer <layer>` to inspect them.
After the relevant traces are pruned (or the layer is no longer pinned), retry consolidation.
```

The user inspects the pinned traces, decides they're stale, prunes them via the existing D21 trace-pruning surface, and retries. The consolidation succeeds.

## 11. Test plan and sub-milestone sequencing

### 11.0 Shared foundation with D33

Phase 17's implementation rests on a foundation shared with D33 (partial-order chains). Bundling both phases on the same foundation avoids paying migration costs twice and keeps the two design docs in sync on their shared structural commitments. The shared foundation lands in a single prerequisite PR that PR 17a depends on:

**PR 0 — Shared foundation** (~4–5 days; preconditions both this phase and D33's milestones 20b–20e):

- **Two-hash identity split** in `kernel/src/layer/mod.rs`. Replace the existing single `compute_layer_id` with `compute_content_hash` (hashes resources only) + `compute_position_hash` (hashes content_hash + sorted parent ids). `LayerId` becomes an alias for `PositionHash`; `ContentHash` is added as a distinct type. `Layer` and `LayerHandle` carry both hashes.
- **Supporting-layer computation** in `kernel/src/layer/supporting.rs` (new). Hooks into `LayerBuilder::build` alongside the existing `canonicalise_resource_refs` pass. Reference-extraction shares structure with the triple-index pass in `kernel/src/layer/index.rs`. Result is cached on `Layer.supporting_layer` and persisted on `LayerHandle.supporting_layer`; no separate index is needed for the forward query.
- **Content-hash dedup index** as a dedicated column family. Enables `Storage::lookup_by_content_hash(ContentHash) -> Vec<PositionHash>` which 17a uses to dedup consolidated-layer content across branches that share a sub-history. This *is* a separate index because it serves reverse lookup (`ContentHash → Vec<PositionHash>`), which the topology entry can't answer on its own.

Migration discipline: PR 0 changes the position-hash byte layout and adds two `LayerHandle` fields. Existing persistent DBs are unreadable after PR 0; recovery is `rm -rf <db>` + reload from source files. This matches the wire-format-break pattern already accepted for Phase 14e (see [`kernel/src/layer/mod.rs`](../../kernel/src/layer/mod.rs)'s `compute_position_hash` docstring). No back-fill path is needed.

Three pre-flight decisions for PR 0:

1. **`LayerId` type discipline.** Introduce `PositionHash` and `ContentHash` as distinct types with `LayerId = PositionHash` alias. Static type safety pays back when comorphism-output IRIs (content-keyed) interact with branch-ref code (position-keyed).
2. **Supporting-layer storage location.** As a field on `LayerHandle`, computed at commit time and serialized through the existing topology entry — forward lookup is the only PR 0 / D25 v1 / D33 v1 access pattern, and the field already covers it. A reverse-lookup index (`supporting_layer → Vec<LayerId>`) is a v2 addition iff a use case lands.
3. **Cost-cap default.** `5_000_000` walk entries for `ConsolidateOpts.max_walk_entries`; deployment-tunable via `EIGENIUS_CONSOLIDATE_MAX_WALK_ENTRIES`.

This phase's 17a precondition tightens once PR 0 lands: every layer in `[from..to]` must have `supporting_layer ◁* parent(from)`. The supporting-layer field on `LayerHandle` makes the check O(|range|) lookups against already-loaded topology entries; the resolve-equivalence invariant (§4) becomes structurally guaranteed rather than hand-rolled.

### 11.1 Phase 17 milestones

```
PR 0 — shared foundation (two-hash + supporting layer + indexes)
                              │
                              ▼
17a top-of-stack algorithm + atomic commit ──┐
                                              │
                                              ├─→ 17b range validation (ancestral / merge-free / pin-free)
                                              │
                                              ├─→ 17c bloom-cache eviction
                                              │
                                              ├─→ 17d cost estimation + dry-run surface
                                              │
                                              └─→ 17e CLI + gRPC surface ──→ shipping
```

| Milestone | Test surface | Pass criterion |
|-----------|---|---|
| 17a | Top-of-stack algorithm; atomic commit via single `WriteBatch`. | Hand-constructed ranges of 10–100 layers consolidate; resolve-equivalence regression passes (head-rooted resolves match before/after); content-addressed `LayerId` is deterministic across re-runs. |
| 17b | Range validation: non-ancestral ranges rejected; merge-containing ranges rejected; pin-containing ranges rejected. | Each negative case returns the typed error with the offending `LayerId`; positive cases fall through to 17a's commit path. |
| 17c | Bloom-cache eviction for collapsed layers; fresh bloom for consolidated layer. | Cache reflects the post-consolidation state immediately; subsequent resolves use the shallow path. |
| 17d | Cost estimation; `--dry-run` flag; `EstimateConsolidation` RPC. | Predicted walk-entry count matches actual within tolerance; `CostExceedsCap` returned when the cap is exceeded; `--dry-run` does not commit. |
| 17e | CLI `eigenius db consolidate` and `eigenius db consolidate-summary`; gRPC `ConsolidateChain` and `EstimateConsolidation`. | End-to-end: notebook produces a chain, operator invokes consolidation via CLI, chain depth shrinks, subsequent EigenQL queries return identical results, `consolidate-summary` shows the operation. |

Cross-cutting tests:

- **Resolve-equivalence regression.** The load-bearing test. Construct a chain with diverse IRI patterns (rewrites, deletions, adds), consolidate a range, run a battery of EigenQL queries against both the original and consolidated chains, assert byte-equal results.
- **Atomicity under crash injection.** Kill the kernel mid-`WriteBatch`; restart; verify the chain is either at the pre-consolidation head (commit didn't happen) or the post-consolidation head (commit did happen) but never in a partial state.
- **Phase 14 + Phase 15 compatibility.** Trivial-merge fast path unchanged; consolidating a range adjacent to a merge node works (per §8.3); consolidating across a merge node returns `RangeContainsMergeNode`.
- **Determinism.** Two independent consolidations of the same range against the same parent produce identical `LayerId`s.

## 12. Open questions

### 12.1 Auto-consolidation policy

When (if ever) does the kernel consolidate without explicit user request? The structural difficulty isn't *when* to consolidate but *what range* to pick — any auto-policy that has the kernel guess what's significant fights with the epistemic posture (§2) that says layer history matters.

**Anchors as the resolution.** Rather than have the kernel guess range boundaries, define the consolidation surface in terms of *anchors* — structurally-derivable or operator-marked points the kernel never consolidates across. Three categories cluster naturally:

1. **User-authored tags.** Explicit operator-marked milestones on the chain — a release, a regulatory checkpoint, a published intermediate result, a notebook save point. Tags are first-class chain resources (sketch below).
2. **Branch events.** Layers that any branch ref currently points at, plus layers where branches were forked (structurally derivable from the topology; Phase 14g already maintains the index).
3. **Merge events.** Multi-parent layers from Phase 14e (trivial merge) and Phase 15 (D20). §8.1's "consolidation refuses to span merge nodes" generalises to "consolidation refuses to span anchors," with merge nodes being the canonical built-in anchor.

Under the anchor framing, the auto-policy becomes structurally simple:

> A range `[from..to]` is *consolidation-candidate* iff no anchor sits strictly between `from` and `to`. The auto-policy picks the oldest unconsolidated candidate range.

The operator places anchors where they care about preserving chain history; the kernel consolidates within the spaces between. No heuristic about "when is a range significant" — significance is operator-declared.

**Tag primitive (v2 sketch).** A new chain-resident class:

```esl
class chain:Tag {
    requires chain:tag_name, chain:tag_target_content;
    recommends chain:tag_message, chain:tag_target_position,
               chain:branch, chain:created_at, chain:created_by;
}
```

Two structural choices worth flagging:

- The target is `chain:tag_target_content` (a `ContentHash`, not a `PositionHash`). Tags survive canonical-linearization rewrites (D33 v2) because the tagged *content* is durable across reordering. `chain:tag_target_position` is recommended as a convenience cache.
- Tags are themselves chain commits — creating, listing, and deleting tags goes through the standard `eigenius load` / commit pipeline. The history of tag operations is as auditable as any other content; deletion is a tombstone resource, not an out-of-band mutation.

CLI surface (v2):

```
eigenius tag create <name> [<layer-id> default head] [--message "..."] [--branch <name>]
eigenius tag list   [--branch <name>]
eigenius tag show   <name>
eigenius tag delete <name>
```

**Range validation extends.** §11.0's range-validation check gains a new clause: `range_contains_anchor(from, to)` returns true if any `chain:Tag` targets a layer in `(from, to)`, if any branch ref currently points at a layer in `(from, to)`, or — the existing rule — if any layer in `(from, to)` has `parents.len() > 1`. Explicit consolidation in v1 already enforces the merge-node case; the anchor framing absorbs it cleanly when v2 adds the tag and branch-event checks.

**Other uses of tags beyond consolidation.** Tags are a chain-management primitive whose primary motivator here is auto-consolidation but whose other uses are independently valuable:

- `eigenius inspect <iri> --at-tag release-v1.0` — time-travel reads against a memorable name rather than a layer-id hex.
- Regulatory audit checkpoints — quarterly review boundaries tagged for later "as of" queries.
- Branch fork points — `branch create feature-x` could implicitly tag the fork layer.
- Notebook breakpoints — cells that publish meaningful intermediate state tag the resulting chain state.
- GC reachability — tagged layers and their ancestor closure don't get reclaimed; tags give operators a precise mechanism for "preserve this point in chain history."

**Scoping.** v1 ships explicit-only consolidation; tags are a v2 addition. The tag primitive is small enough to land alongside v2's auto-policy as a single phase (probably Phase 17.5 or whenever auto-consolidation is wanted), but tags' other uses (above) may justify shipping them earlier as their own small phase. The decision is operator-demand-driven; the design surface is small either way.

Three sub-questions that fall out of the anchor framing:

- **Anchor scope:** are tags global (chain-wide) or branch-local? Recommended: branch-local namespace, with the option to declare a tag global by qualifying its name.
- **Anchor deletion semantics:** if an operator deletes a tag and the auto-policy then consolidates across what was previously protected, is that surprising? Recommended: tag deletion writes a tombstone that prevents auto-consolidation for a configurable cool-down (default: 24 hours) so operators have time to notice.
- **Anchor count limits:** very large tag sets (10⁴+) might slow consolidation validation. Recommended: an indexed lookup so per-range anchor-containment is O(log n) rather than O(n); no v1 concern.

### 12.2 v2 multi-parent consolidation

The §8.2 sketch is the working model but several decisions are open:

- Do `consolidated_resolutions` records get exposed in EigenQL queries (queryable as a regular property), or only via a dedicated diagnostic surface?
- Is the multi-parent consolidated layer auditable transitively — if I want to audit a claim that depends on a merged-and-consolidated layer, do I have to reconstruct the original merge layer to check the cascade-acks?
- How does v2 multi-parent consolidation compose with itself (consolidate a range that contains a previously-consolidated multi-parent layer)?

### 12.3 Trace pin re-pointing vs. invalidation

§7.2's options (b) and (c) both have merit; v1 defers. The decision criterion is usage data: how often do real workflows have long-lived traces that block consolidation? If "rarely" — v1's refusal policy is fine indefinitely. If "often" — re-pointing is the obvious next step. The trace-store schema change is small (one nullable field per pin) but the consumer-side render of stale state is a UX decision.

### 12.4 Roll-up index orthogonality

D23 §5.2.7's roll-up index speeds resolves without reducing storage cost. Consolidation does both. The two are orthogonal and both can ship; the decision question is which lands first when chain-depth pathology becomes a real workload concern. Probably consolidation (this phase) lands first because it addresses both axes; the roll-up becomes a v2 layer over the post-consolidation chain if resolve cost is *still* dominant.

### 12.5 Cost estimation accuracy

`predicted_entries = sum(len(defined_iris(L)) for L in range)` is an upper bound on the walk; the actual walk skips IRIs already seen. For ranges with heavy rewrites (the same IRI redefined many times), the actual walk is much smaller than the prediction. v1 uses the upper bound for the cap; v2 may invest in a tighter estimate if real workloads hit the cap on benign ranges.

### 12.6 Consolidation across long-running tasks

A long-running Phase 9b task may pin layers across the consolidation window. The trace-pin refusal policy catches this. But what about a task that's *in progress* — its pins exist but its trace isn't yet "complete"? The refusal policy treats both cases identically; it's worth verifying this is the right choice (versus, say, allowing consolidation that respects only completed traces).

### 12.7 Interaction with branch pruning (Phase 14g)

Phase 14g's `eigenius db prune <branch>` removes a branch from the topology; reachable layers become unreachable. Consolidation operates on the chain reachable from a specific branch's head. If a consolidation completes and a subsequent prune removes the branch, the consolidated layer becomes unreachable along with the original layers. This is correct — both fall to GC — but worth confirming with a regression test.

### 12.8 Forward pointers — consolidating below the branch head

v1 requires `to = current_branch_head` (enforced as `BranchAdvancedConcurrently` when it isn't). The motivation is structural: layer ids are content-addressed and fold their parent ids into the hash, so changing the parent of any layer above `to` would cascade re-ids through every descendant up to head. Rather than rewrite that tail, v1 simply forbids the case.

A cleaner v2 resolution is a *resolve redirect* (forward pointer) installed on `to`:

- Topology stays unchanged. Layers above `to` keep their existing parent pointers and their existing `LayerId`s.
- A new metadata entry — call it `redirect:<to> → <L_c>` — sits next to the topology entry. It is *not* part of any layer's identity hash; only the resolve walk consults it.
- When `Layer::resolve` walks head→root and reaches `to`, it follows the redirect to `L_c` and continues the walk through `L_c`'s ancestor closure (i.e., `parent(from)` and below). The collapsed content stays accessible via `L_c`; the original layers in `(parent(from), to]` are GC-eligible once the redirect is in place.

Two structural properties make this work:

1. **Resolve-equivalence is preserved.** The redirect short-circuits the walk at `to` to go through `L_c`. Since `L_c` contains the top-of-stack value for every IRI in `[from..to]`, and `L_c.parent = parent(from)`, the walk returns the same values it returned before consolidation — for every IRI, from any head-rooted starting point above `to`.
2. **No id cascade.** Because the redirect lives outside the hash domain, no layer needs to be re-ided. Branch refs, trace pins, task-record `layer_head` values, and external system keys that name layers by id all stay valid.

Trade-offs and open questions for v2:

- **Storage shape.** A dedicated `redirect:<position>` column family (or a field on `LayerHandle` populated when a redirect is installed). The redirect is one entry per consolidation operation; storage cost is negligible. Atomicity bundles into the existing single-`WriteBatch` per D23 §6.3.
- **Resolve cost.** One extra hop at the redirect site. The bloom-skip pattern (D23 §5.2.2) still applies on both sides of the redirect. For deep chains the savings from collapsing dominate the one-hop cost.
- **Redirect chaining.** Consolidating a range whose `to` is already a redirect target needs a policy. Two natural choices: (a) refuse — operator must `from = parent(existing_redirect_target)` instead; (b) collapse the chain of redirects into one. (b) is structurally cleaner; (a) is simpler.
- **GC interaction.** When the redirect is installed, the layers in `(parent(from), to]` become unreachable from head-rooted resolves. They stay reachable for time-travel reads against intermediate layer ids until GC reclaims them — same lifecycle as the `to = head` case today.
- **Time-travel reads against `to`.** A `at_layer = to` read still resolves the redirect (because the resolve walk starts at `to` and the redirect points to `L_c`). For an audit-style "what did the chain look like before consolidation?" view, the redirect needs a bypass mode, or the operator consults `db consolidate-summary` (D25 §10.1) to find the pre-consolidation history. v2 to decide.

The redirect mechanism is largely orthogonal to the rest of D25's machinery and can ship as its own milestone (call it 17f) once a workload demands it. v1's `to = head` restriction is sufficient for the notebook-session-squash and rolling-window-consolidation patterns that motivated Phase 17; the redirect is the natural next step when operators want to consolidate older history while preserving newer commits.

#### 12.8.1 v1 design decisions

Four decisions captured from Phase 17 wrap-up so that when 17f lands, the design call is recorded rather than re-derived. Each ships an explicit reversal path (none of these locks us in).

**(a) Redirect chaining policy — refuse for v1.**
First consolidation installs `redirect: B → L_c1`. A second consolidation whose range touches `B`, `L_c1`, or anything in between is rejected with a typed error (working name `RangeCrossesExistingRedirect`). The operator works around by consolidating above the existing redirect, or by structuring one larger range upfront. Reverse-out is a future `RedirectChainPolicy::Replace` opt-in that absorbs the previous redirect's target into a new `L_c2` (one-hop walks, old `L_c1` becomes GC-eligible). The *chain* alternative — keep multiple redirects and walk N hops — is rejected on the grounds of unbounded resolve-hot-path cost; only one-hop *replace* is the future direction. Tracked separately as [issue #49](https://github.com/eigenius/eigenius/issues/49).

**(b) Time-travel reads — reclaim by default, opt-in preserve.**
GC's reachability mark, by default, does not exempt the consolidated range from reclaim. Storage shrinks; time-travel reads against intermediate layers fail with the standard missing-layer error once GC has run. This matches the effective behavior of the 17a–17e `to = head` consolidation, so operators have one consistent mental model: "consolidation is destructive."

`ConsolidateOpts.preserve_history: bool` (default `false`) flips the contract: GC's mark phase follows redirect *sources*, not just targets, keeping the consolidated range alive. The redirect becomes a pure resolve-optimization rather than a storage-savings mechanism. The preserve mode is for compliance / regulatory workloads where the pre-consolidation history must remain queryable; everyday rolling consolidation uses the default. Both modes coexist in a single chain on a per-call basis.

Note that 12.8.1(a)'s future *replace* compose policy is feasible only against `preserve_history = true` redirects — composing across a reclaimed range has no original content to re-run top-of-stack over. The typed error for compose-against-reclaim is part of (a)'s eventual surface.

**(c) Redirect storage — dedicated CF on disk, inline cache on `Layer`.**
The hot path matters: `Layer::resolve` probes for a redirect at every visited layer. A pure HashMap lookup costs ~20–50 ns per step (hash + bucket access + comparison); an inline `Option` check costs ~1 ns (branch-predictable, mostly `None`). For a 1000-step resolve walk on a chain with no redirects, that's the difference between ~1 µs and ~30 µs of pure-probe overhead — non-trivial on hot EigenQL paths that resolve repeatedly.

The v1 shape gets inline-speed reads with dedicated-CF storage:

- **On disk.** A new `redirect:<layer_hex>` column family on RocksDB; an equivalent `BTreeMap<LayerId, LayerId>` on the memory backend. Sparse (one entry per consolidation, not per layer), atomically installed via the existing single-`WriteBatch` per D23 §6.3, prefix-scannable for enumeration. `LayerHandle` CBOR is **unchanged** — no per-handle bloat, no rewrite amplification on install.
- **In memory.** `LayerStorage` gains an `Arc<dyn RedirectMap>` alongside `bloom_cache` and `triple_index`, loaded once at startup from the CF into a `HashMap<LayerId, LayerId>`.
- **`Layer` enrichment.** `Layer` gains an inline `redirect_target: Option<Arc<Layer>>` field. `build_chain` populates it: for every constructed layer, consult the in-memory redirect map; if the layer is a redirect source, also build the target's chain and store its head Arc in `redirect_target`.
- **`Layer::resolve` (and `resolve_all`, `iter_all_resources`) gain one line.** Before consulting the layer's bloom and content, check `if let Some(t) = layer.redirect_target.as_ref() { … }` — a single branch with no map probe. The follow itself is a pointer indirection through the pre-resolved Arc.

The result: storage shape stays sparse and easy to enumerate; hot-path read stays inline and cheap. Considered and rejected: embedding `redirect_to: Option<LayerId>` on `LayerHandle` itself. That grows the on-disk handle by ~33 bytes per layer (mostly `None`), rewrites the handle CBOR on every install, and forces a topology-wide scan for enumeration — none of which the hybrid suffers.

**(d) `to`'s topology entry — persistent tombstone on disk (shape 1).**
A topology-walk audit (gc mark/sweep, `lattice::find_lca`, the trivial-merge IRI-source resolver, `merge_independent_heads` head validation, `LayerTopology::walk_chain`) identified six in-kernel sites that follow `LayerHandle.parents` purely structurally. Each one terminates its walk at the first `topology.get_layer(id) == None`. Naive full-reclaim of `to` would terminate GC's mark phase at `to.id`, leaving `L_c`'s ancestor subtree unmarked and exposing it to sweep — catastrophic.

Three shapes were considered:

1. **Persistent tombstone on disk.** Keep `to`'s `LayerHandle` stored after consolidation. ~150 bytes per consolidation; no walker changes; the source layer's interior parents (below `to`) and content can still be reclaimed by GC.
2. **Full reclaim + redirect-aware walkers.** Each of the six call sites learns to consult the redirect map on a topology miss. Spreads the redirect concern across `gc`, three `lattice` functions, `walk_chain`, and the storage backends' `load_chain_from` impls.
3. **Synthetic tombstone — full reclaim on disk, manufacture on load.** The redirect CF is the source of truth. `PersistentBackend::load_topology` joins the redirect CF with the topology CF and manufactures synthetic tombstones for reclaimed redirect sources. Works for `load_topology` callers but **not** for `load_chain_from`, which reads `topo:<id>` directly per layer and would miss reclaimed entries.

**Shape 1 is the v1 choice.** The implementation lives in `gc::mark_reachable`: when the BFS visits a redirect source, the source is marked reachable (so its on-disk topology entry survives GC); the source's `parents` are skipped in reclaim mode so the *interior* of the consolidated range is reclaimable, and the redirect's target is enqueued so `L_c`'s ancestor closure stays alive. The source's content (resources, bloom, content-hash index) and topology entry persist forever after consolidation — a few hundred bytes per operation — but no walker in the kernel needs to know redirects exist for topology purposes.

The `is_redirect_source: bool` flag on `LayerHandle` and the `augment_topology_with_redirects` mechanism remain useful: they let diagnostic surfaces render reclaimed-interior ranges as "consolidated into <target>" and provide a path to shape (3) in the future. In v1, the synthetic-tombstone path is exercised only when an operator manually invokes `delete_layer(source)` on a redirect source — not via the standard reclaim flow.

Reverse-out to shape (3) is a single change: stop marking redirect sources as reachable in `gc::mark_reachable`. The same change requires `load_chain_from` to consult the redirect CF when a `topo:<id>` lookup misses, so it can construct a synthetic chain entry instead of returning `NotFound`. Defer until a deployment justifies the additional ~few hundred bytes of cost per consolidation.

## 13. Related work

**Git rebase / squash.** The closest user-facing analog. `git rebase -i` lets a user squash a contiguous range of commits into one. The operations are not fully analogous: Git's squash modifies history by producing a new commit chain that doesn't share ancestry with the original; Eigenius's consolidation is content-addressed and the consolidated layer's `LayerId` is determined by content, so two independent squashes by different operators against the same range produce the same layer. Git's squash is also not atomic (it's a sequence of operations the user can interrupt mid-rebase); Eigenius's consolidation is a single `WriteBatch` per D23 §6.3.

**Postgres VACUUM.** The auto-consolidation analog. Postgres reclaims space from MVCC-versioned rows in the background; the operator has limited control over when. Eigenius's v1 ships explicit-only and defers the auto-policy; the §12.1 question is exactly the VACUUM trade-off.

**LSM-tree compaction (RocksDB, Kafka, ScyllaDB).** The storage-internal analog. Eigenius runs RocksDB underneath, so consolidation is "compaction at the layer level on top of compaction at the storage level." The interaction is benign: the layer-level consolidation produces fewer column-family entries; RocksDB's compaction merges those entries into fewer SSTs. Both are doing the same job at different granularities. Worth noting that LSM-tree compaction is *automatic* (the engine triggers it based on size and level); the layer-level analog is intentionally not (Eigenius's epistemic posture).

**Datomic excision.** Datomic's `excision` operation removes specific values from history under retention rules. Closer to GC than to consolidation in spirit — it removes information, where consolidation preserves all head-reachable information and just compresses the representation. The two are complementary; Eigenius could grow a similar excision surface for compliance use cases (e.g., GDPR right-to-erasure) without disturbing consolidation semantics.

**Mercurial collapse extension.** Mercurial has a `collapse` extension that merges multiple changesets into one — same shape as Git squash. Same caveats apply.

**TerminusDB squash.** TerminusDB's squash operation is the closest existing analog in a typed-graph database. Their semantics preserve the head-resolve behaviour but don't have an explicit resolve-equivalence invariant in their docs. Worth studying their UX precedent for the consolidation CLI surface.

## 14. References

- D23 §5.2.7 — the deferred deep-chain performance concern that motivates this phase
- D23 §6.3 — the atomic-commit `WriteBatch` discipline this phase respects
- D20 — Phase 15 layer reconciliation; the resolution decisions v2 multi-parent consolidation must preserve (§8.2)
- D21 — task traces and checkpointing; the pinning semantics §7's refusal policy enforces
- D13 — durable kernel state; the seed manifest consolidation does not modify
- Phase 17 in `docs/design/implementation-plan.md`

Source code touchpoints (entering Phase 17):

- `kernel/src/layer/consolidate.rs` (new) — top-of-stack algorithm, range validation, atomic commit
- `kernel/src/layer/cache.rs` — bloom-cache eviction hook (already exists for GC; consolidation reuses)
- `kernel/src/storage/branch.rs` — CAS-update of branch ref to consolidated layer
- `kernel/src/trace/store.rs` — trace-pin enumeration for the refusal policy (read-only in v1)
- `proto/eigenius_kernel.proto` — `ConsolidateChain` and `EstimateConsolidation` RPCs
- `cli/src/db/consolidate.rs` (new) — `db consolidate`, `db consolidate-summary`, `--dry-run` surface
