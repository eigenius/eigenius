# D33 — Partial-Order Chains and Commutativity-Aware Semantics

**Status:** Draft (2026-05-10)
**Phase:** 20
**Supersedes:** the implicit "chain is a sequence" assumption that runs through D23 §5.2, D25 §6, D21 §3.6, and the validation rules in D1 §6
**Companion docs:** D23 (out-of-core layer architecture; the per-layer storage and resolve walk this phase reframes), D25 (chain consolidation; the v1 implementation of the two-hash identity model this phase generalizes), D20 (layer reconciliation; the merge mechanism partial-conflict semantics extend), D21 (task traces and checkpointing; the pinning model that gains a supporting-layer context), D13 (durable kernel state; the seed manifest constraints this phase respects)

## 1. Summary

The chain is structurally a **partial order of layers**, not a sequence. The linearization the system stores today is *one valid choice* among potentially many. Most operations that currently treat the chain as a sequence — resolve, branch refs, trace pins, merge, consolidation, garbage collection — produce identical externally-observable behaviour under any valid linearization. Making the partial-order structure explicit unlocks: notebook anchored-commit cache reuse, storage dedup keyed on content rather than position, forward compatibility with distributed execution across multiple kernels, and an honest audit story for replay.

The shift rests on three structural commitments:

1. **Two-hash identity** (delivered by D25 §X). Every layer carries a `content_hash` (over its definitions) and a `position_hash` (over content + parents). Identity decouples from position.
2. **Supporting layer**: for every layer L, the topmost ancestor whose ancestor-closure provides every IRI L references. The supporting layer is the *minimal anchor* of L in the chain — L can be placed anywhere between supporting(L) and the head without breaking references.
3. **Partial-conflict taxonomy**: a precise classification of layer commits that are not pure additions (constraint tightening, schema narrowing, type strengthening, recommends→requires promotion). Each class can retroactively invalidate lower-layer content; the platform must take an explicit position on whether and when to enforce.

Together these commitments make commutativity a *decidable* structural property rather than a heuristic, and they give every restructuring operation (consolidation, merge, GC) a clean precondition.

This document specifies the structural model, the supporting-layer index, the partial-conflict taxonomy, what changes for each existing surface, and a v1/v2 scoping that lets the model land incrementally on top of D25's two-hash identity work.

## 2. Motivation

Four payoffs justify the structural shift. Each is real today; each is achievable only piecemeal without the partial-order framing.

### 2.1 Notebook cell-output reuse

A user iterating in the notebook re-runs the same cell many times. Currently each re-run produces a fresh layer with a fresh `LayerId` (because parent identity is part of the hash), even when the cell's content and the cell's references are byte-identical to a previous run. The chain accumulates near-duplicate layers; the user pays re-execution cost for content that's already on the chain; expensive cells (LLM dispatches, simulation runs, comorphism dispatches) get redone gratuitously.

With content-hashed identity and supporting-layer awareness, the cache key for a cell's output is `(content_hash, supporting_layer_hash)`. Two re-runs of the same cell against the same supporting context dedup transparently; against a shifted but supporting-equivalent context they still dedup; against a context that genuinely changes a referenced definition they don't (correctly).

### 2.2 Distributed execution

The single-`serve`-per-DB constraint (Phase 14) is structural — chain commits serialize through branch-ref CAS. With partial-order chains, two kernels with non-overlapping IRI surfaces can commit independently, and a third process merges their commits later as a chain rewrite (no merge resolution, no pushout, no conflict). This is the largest architectural unlock and worth surfacing even though the multi-kernel work is outside this phase. Naming it commits the design to forward compatibility.

### 2.3 Audit honesty

The chain today records the order in which a single user committed layers, which is meaningful when commits are causally dependent and arbitrary when they are not. Auditors and reviewers reading the chain cannot tell the two cases apart. Under partial-order semantics, the chain records the *partial order* (what depends on what); the linearization the user committed in is metadata, not the canonical artifact. This is a structurally cleaner story for compliance, regulatory review, and replay against alternative ordering.

### 2.4 Integrity under restructuring

Consolidation (D25), merge (D20), and GC all rewrite the chain. Each currently relies on implicit invariants about layer identity and ordering that are not formally stated. D25's resolve-equivalence invariant *is* stated, but it depends on the linear chain assumption. Under partial-order semantics:

- Consolidation generalizes from "collapse a contiguous linear range" to "collapse any antichain whose union has a single supporting layer." The supporting-layer concept gives consolidation a precondition that today it implicitly requires but doesn't check.
- Merge becomes "the non-commutative case" — divergent branches that commute trivially merge by union; non-trivial merge is what D20 already handles.
- GC reachability is unchanged in structure but gains a precise definition under multiple valid linearizations.

The partial-order framing makes every restructuring operation's contract explicit. Without it, each operation hand-rolls invariants that don't compose.

## 3. Goals and non-goals

**Goals:**

- A precise definition of the chain as a partial order, with the linear chain as one valid linearization.
- A precise definition of the **supporting layer** for any layer L, computable in one pass over the reference graph.
- A precise **partial-conflict taxonomy** classifying layer commits that are not pure additions.
- A precise definition of **commutativity** between two layers, decidable from the supporting layers and the partial-conflict taxonomy.
- An explicit position on monotonicity: which retroactive validations the platform performs by default, and which require opt-in.
- Cell-output reuse keyed on `(content_hash, supporting_layer_hash)` as the user-facing v1 deliverable.
- Coexistence with D25 (two-hash identity is shared), D20 (merge picks up partial-conflict semantics), D21 (trace pins gain supporting-layer context), and D13 (no change to seed manifest).
- Sub-milestone sequencing for incremental landing on top of D25.

**Non-goals:**

- Full DAG operations exposed at the user surface (multiple-frontier branches, partial-order resolve walks chosen at query time, etc.). v1 keeps the chain linear at the user surface; partial-order structure is metadata. v2 may expose DAG operations.
- Retroactive constraint validation by default. v1 commits to forward-only semantics for AutoOnLoad gates and constraint additions; v2 may add opt-in retroactive validation.
- Distributed coordination protocols. v1 retains the single-`serve`-per-DB constraint; v2 may add multi-kernel concurrent commits.
- Permissions / access control over partial-order linearizations. Genuinely far-future; mentioned in §10.
- A new merge resolution shape. D20's six strategies suffice; partial-conflict detection extends D20's conflict surface, doesn't replace it.
- A protocol for proof-carrying commutativity claims (a hypothetical future where Lean-4 institutions emit verified commutativity proofs the kernel checks). §10 only.

## 4. Theoretical foundation

The framing borrows the categorical apparatus from D20 (Spivak's `Seven Sketches in Compositionality`, ch. 3) and extends it with two new structural concepts.

### 4.1 The chain as a partial order

Recall (D20 §4): an ontology layer `L` presents a category `C_L`. Layer extension is functorial; a child layer `L'` extending `L` gives a functor `F : C_L → C_L'` that is the identity on inherited content.

**Definition.** A *chain* is a poset `(Λ, ◁)` where `Λ` is a set of layers and `L_a ◁ L_b` ("L_a is an ancestor of L_b") iff there is a sequence of functors `C_{L_a} → C_{L_{a+1}} → … → C_{L_b}` each of which is a layer-extension functor.

The current implementation realises `(Λ, ◁)` as a strict total order via the parent pointer in `LayerId`. The partial-order claim is that the parent pointer encodes one valid linearization of `(Λ, ◁)`; other linearizations may be equally valid.

### 4.2 The reference graph

**Definition.** For a layer `L`, let `defined_iris(L)` be the set of IRIs L declares (classes, properties, data types, inductive types, constructor names, resources). Let `references(L)` be the set of IRIs L mentions in property `class_types`, `subclass_of`, `requires`, `recommends`, `domain`, `data_type`, `format_constraints`, ctor `arg_types`, comorphism `export_format` / `transformation` / `import_format`, signature `input_types` / `output_type`, and any other place a chain-resident IRI appears in L's content.

The reference graph for the chain is the directed graph `(N, E)` where `N = ⋃ defined_iris(L) for L ∈ Λ` and `E = {(r, d) : ∃ L. r ∈ references(L) ∧ d ∈ defined_iris(L_owner(r))}`. The reference graph is well-defined under the standard validity rules for chain commits (every reference resolves to a defined IRI somewhere in the chain).

### 4.3 Supporting layer

**Definition.** For a layer `L`, the *supporting layer* of L is

```
supporting(L) = the topmost layer L_s ∈ Λ such that
                references(L) ⊆ ⋃ {defined_iris(L') : L' ∈ ancestors(L_s) ∪ {L_s}}
```

If L has no references (a pure top-level definition layer), `supporting(L) = ⊥` (the chain root). If references(L) cannot be satisfied by the chain, L is invalid (the standard pre-commit validation already rejects this).

**Properties of `supporting`:**

1. **Position freedom.** L can be placed at any chain position L' such that `supporting(L) ◁ L'` without breaking references. This is the structural width of L's freedom in the partial order.
2. **Computable in one pass.** Given the reference graph, computing `supporting(L)` is a single walk from the head down to the lowest layer whose ancestor-closure covers `references(L)`. With a per-IRI "topmost-defining-layer" index (D23 §5.2.2 already maintains a related structure for resolve), computation is O(|references(L)|) lookups.
3. **Indexable.** Storing `supporting(L)` as a property on every committed layer enables O(1) lookups for downstream operations (consolidation precondition checks, partial-order linearization checks, anchored-commit cache invalidation).

### 4.4 Partial-conflict taxonomy

A pure-addition layer adds new resources without modifying existing schema. Pure-addition layers are *monotonic*: their addition cannot invalidate any previously-valid resource. Many real layer commits are not pure additions. The four classes:

#### (a) Constraint tightening

A new layer adds a Decidable-QC constraint, a `min_value`, a `max_value`, a regex `format_pattern`, an enumerated `allowed_values`, or any other property-level constraint. Existing resources whose values satisfied the looser constraint may no longer satisfy the tighter one.

#### (b) Schema narrowing

A new layer narrows an `allowed_values` list, narrows a class hierarchy by removing a `subclass_of` link, removes an `InductiveCtor` from an `InductiveType`, or otherwise reduces the set of valid resource shapes. Existing resources / inductive values that used the now-disallowed shapes become invalid against the narrowed schema.

#### (c) Type strengthening

A new layer narrows a property's `data_type` (e.g., `core:string` → `core:iri`), narrows its `class_types` (e.g., from `[A, B]` to `[A]`), or otherwise restricts the type of values the property can carry. Existing resources whose property values typed under the looser shape become invalid.

#### (d) Recommends → requires promotion

A new layer promotes a `recommends` property to `requires`. Existing resources missing the now-required property become invalid.

**The structural fact.** Each class is a *partial* layer commit — the new layer changes the schema but does not provide the resources affected by the change. The chain-as-of-the-new-layer is internally inconsistent unless either (i) the platform performs retroactive validation and rejects the commit if any existing resource violates, or (ii) the platform commits the change with forward-only semantics and accepts the inconsistency as historical.

### 4.5 Commutativity

**Definition.** Two layers `L_a` and `L_b` *commute* iff:

1. `defined_iris(L_a) ∩ defined_iris(L_b) = ∅` (no IRI fights).
2. `supporting(L_a) ◁ supporting(L_b)` or `supporting(L_b) ◁ supporting(L_a)` or `supporting(L_a) = supporting(L_b)` — that is, neither's supporting layer is in the *other's* descendant chain.
3. Neither L_a nor L_b is a partial-conflict commit whose constraint affects content defined in or below the other.

(1) and (2) are decidable in O(1) per pair given pre-computed `defined_iris` and `supporting` indexes. (3) is decidable but more expensive — it requires checking each constraint's targets against the other layer's resources. v1 sidesteps (3) by restricting commutativity decisions to pure-addition layer pairs (where (3) is trivially true).

**Lemma (resolve-equivalence under reordering).** If `L_a` and `L_b` commute, then any chain that contains both — in either order — produces identical head-rooted resolves for every IRI. Proof by induction on resolve walks: the ordering of L_a and L_b affects which is encountered first by the head→root walk, but since their `defined_iris` are disjoint, no walk depends on the order. Reference-resolution is unaffected by (2), and constraint application is unaffected by (3) under v1's pure-addition restriction.

This is the structural foundation that justifies treating commuting layer pairs as an antichain in the partial order rather than as a sequence.

## 5. The two structural commitments

### 5.1 Two-hash identity (cited from D25)

D25 §X commits the kernel to two hashes per layer:

```
content_hash  = SHA256(canonical_eigon_cbor(defined_iris ∪ resources ∪ commit_metadata))
position_hash = SHA256(content_hash || sorted(parent_position_hashes))
```

D33 builds on this. The supporting-layer index is keyed on `position_hash` (locating a specific instance in the chain); anchored-commit cache lookups are keyed on `content_hash` (matching content regardless of position); commutativity decisions use `position_hash` for the supporting-layer comparison and `content_hash` for the equality check.

This phase does not modify D25's identity model. It uses both hashes for distinct purposes that D25's surface already exposes.

### 5.2 Commutativity as a property of layer pairs

A new chain-resident property records the result of commutativity decisions:

```
{
  "@id": "urn:eigenius:chain:commutativity:<position_a>:<position_b>",
  "core:is_a": ["urn:eigenius:chain:Commutativity"],
  "chain:layer_a": "<position_hash_a>",
  "chain:layer_b": "<position_hash_b>",
  "chain:commute": true | false,
  "chain:basis": "DefinedIrisDisjoint" | "SupportingLayerOrder" | "ManualAssertion" | …,
  "chain:established_at": "<timestamp>"
}
```

These are *cached decisions* — the kernel computes commutativity at commit time for every newly-committed pair (or lazily on first query) and stores the result. Subsequent operations (anchored-commit cache, partial-order operations in v2) consume the cached decisions rather than re-deriving.

The `chain:basis` property records *why* the kernel decided commutativity. This matters for audit (reviewer can verify the basis) and for invalidation (if the chain ontology changes such that a `DefinedIrisDisjoint` decision becomes wrong, the cache entry can be selectively invalidated based on basis).

## 6. Anchored-commit cache (cell-output reuse and beyond)

The user-facing v1 deliverable. Any commit path that produces deterministic content anchored to a supporting layer can route through the kernel's *anchored-commit cache* and get content-addressed reuse for free. The mechanism is general; "cell-output reuse" is the canonical application but not the only one. Concrete use cases:

- **Notebook cell re-runs.** A cell that produces byte-identical resources against an unchanged supporting context returns the existing layer's id without re-executing.
- **Institution ontology reload.** Re-loading the same ontology against the same core layer hits the cache; no new chain commit.
- **Mirror regeneration.** A deterministic mirror generator (e.g., Julia, Lean) re-runs against the same institution layer and produces the same output content → cache hit.
- **Any deterministic content generator whose supporting layer is its dependency anchor.**

The mechanism is a single `commit(content, supporting_layer) → LayerId` memoization, keyed on the content hash and the supporting layer's *content hash* (not its position hash — that's the load-bearing detail). The kernel checks the cache before committing:

```
cache_key = (content_hash, supporting_content_hash)

if cache_key in anchored_commit_cache:
    return cached_layer_id  # no commit; reuse existing

else:
    L_new = build_and_commit_layer(content, supporting=supporting_layer)
    anchored_commit_cache[cache_key] = L_new.position_hash
    return L_new
```

Three cases (notebook-cell flavor):

- **Cache hit, identical context.** The cell's content is byte-equal to a previous run, and its supporting layer is also unchanged. Re-running returns the existing layer's `position_hash` immediately; no chain commit; no re-execution of the cell's preceding work.
- **Cache hit, supporting-equivalent context.** Same content, different parent chain, but the supporting layer is the same content (different parent linearizations of the same supporting context). Same outcome as the above — the cache key matches because supporting *content* is what's keyed on, not supporting *position*.
- **Cache miss.** Either the content has changed or the supporting layer's content has changed. Standard commit path; the new layer is added; the cache is updated.

**What the cache buys.** Four concrete things (the first three are the notebook flavor, the fourth is the generalization):

1. Re-executing a cell whose content is unchanged is free (no chain commit, no re-execution of expensive operations like LLM calls or comorphism dispatches that produced the cell's content).
2. Cells that don't depend on each other can be re-run in any order without producing different chain identities.
3. Two notebooks (or two users) running structurally-equivalent cells against equivalent supporting contexts dedup their commits — the chain has one canonical record of "this content was produced from this support."
4. **Operational no-ops.** Re-loading an institution ontology against an unchanged core is a no-op. Regenerating a deterministic mirror against an unchanged source institution is a no-op. Bootstrap-style "reload everything" workflows become idempotent at the commit level (not just at the `store_layer` level, which the position-hash content-addressing already covered for roots).

**What the cache does not do.** Two non-features worth flagging:

1. The cache doesn't replay side effects. If the cached cell originally dispatched an LLM call, the cache hit returns the chain layer — not the LLM's response. The chain layer *is* the response (it's what the cell committed); but if downstream code needs to re-trigger the LLM's effects, the cache won't do that.
2. The cache doesn't provide replay against an arbitrary historical chain state. If the user wants "re-run the cell as if today's chain didn't have last week's commits," that's a separate operation (the kernel would need to compute the cell's output against a hypothetical chain state). Anchored-commit reuse is for the common case where the user wants the cell's output against the actual current chain.

**Implementation footprint.** A new `anchored:<content_hex>:<supporting_content_hex>` column family in RocksDB → position-hash value. Insertions are atomic with the layer commit (single `WriteBatch` per D23 §6.3). Lookups are O(1) hash probes. Eviction is unnecessary in v1 — the cache is small (one entry per distinct content × supporting context); pruning can be added later if deployment data shows growth. The kernel-side surface lives behind four `PersistentBackend` trait methods (`lookup_anchored_commit`, `put_anchored_commit`, `delete_anchored_commit`, `list_anchored_commits`) and a high-level `commit_layer_with_cache` wrapper in `lattice.rs`.

**The structural framing: "anchored."** The supporting layer is the content's *dependency anchor* in the chain — the youngest ancestor that any of the content's references resolve through. The cache keys commits on `(content, anchor's content)` exactly because two commits with the same content + the same anchor are structurally indistinguishable. The framing is intentionally broader than "cell output" so that v2 use cases (mirror regeneration, ontology reload, etc.) inherit the property without separate plumbing.

## 7. What changes for existing surfaces

Each subsection below answers: *under partial-order semantics, what's the v1 contract, and what's the v2 sketch?*

### 7.1 Resolve

**v1.** The chain is canonically linearized at storage time; resolve walks the linearization head→root via the existing per-layer-bloom skip pattern (D23 §5.2.2). The canonical linearization groups commutative layers stably (deterministic order keyed on `content_hash`) but otherwise preserves the user's commit order. No semantic change to resolve.

**v2.** Resolve walks any topological order over the partial-order chain. Two valid linearizations produce identical resolves by §4.5's lemma; the kernel may choose the order that minimizes bloom-walk cost.

### 7.2 Branch refs

**v1.** A branch ref points to a single `position_hash` (the canonical linearization's head). Unchanged from D23.

**v2.** A branch ref points to a *frontier* — a set of `position_hash`s that are pairwise commutative and represent the maximal antichain of recent commits. `branch_show` displays both the canonical linearization and the frontier; downstream operations work against either.

### 7.3 Trace pins

**v1.** A trace pin is `(position_hash, IRI)` plus an optional `supporting_layer_hash` indicating the supporting context the trace ran against. The supporting-layer context lets D25's consolidation re-point pins to the consolidated layer when the supporting context is preserved (D25 §7.2 (b) becomes tractable: re-pointing is safe iff the consolidated layer's supporting layer matches the pin's recorded supporting layer).

**v2.** Pins gain a partial-order context — the antichain of layers the trace ran against, not just a single layer. Replay reconstructs the chain state at the pinned antichain.

### 7.4 Merge (D20)

**v1.** Merge picks up the partial-conflict taxonomy. D20's `Witness` resolution generalizes to "any conflict whose witness includes a partial-conflict-resolving constraint," with the constraint's class (constraint-tightening, schema-narrowing, etc.) recorded in the resolution metadata. D20's six strategies remain — partial-conflict semantics extend the conflict surface they classify, not the resolution surface.

A practical example: branch A adds resource `R` of class `C`; branch B promotes `C.foo` from `recommends` to `requires`. The branches don't trivially merge (B's promotion makes A's R invalid). The conflict is a partial-conflict-promotion (§4.4 (d)); D20's `Witness` resolution requires a comorphism that supplies the missing `foo` property for R or rejects R from the merged chain.

**v2.** Merge can resolve commutativity-aware automatically: if branches A and B contain only pure-addition layers and their layers' supporting layers all sit at-or-below the common ancestor, they trivially merge by union (this is D23's existing trivial-merge fast path generalized to partial-order semantics).

### 7.5 Consolidation (D25)

**v1.** Consolidation gains a precondition: every layer L in the consolidation range has `supporting(L) ◁ parent(from)`. If this fails — if some layer in `[from..to]` has its supporting layer *inside* the range — consolidation is rejected (the consolidated layer would be unable to satisfy its references against `parent(from)`'s ancestor closure). The current D25 implementation implicitly relies on this; D33 makes it a checkable precondition.

The consolidation also generalizes from "linear range" to "antichain": if a set of layers is pairwise commutative and they all share a supporting layer at-or-below `parent(from)`, they can be consolidated together. v1 keeps the linear-range surface; the antichain generalization is a v2 expansion that doesn't break existing callers.

**v2.** Consolidation operates over arbitrary antichains in the partial order; the consolidated layer is positioned at any chain location dominated by the antichain's supporting layer.

### 7.6 GC

**v1.** Reachability is unchanged: a layer is reachable iff it's transitively pointed at by a branch ref or a trace pin. Under partial-order semantics with frontier branch refs (v2), reachability sweeps the partial order from the frontier instead of from a single head. v1's linear-chain reachability subsumes this.

### 7.7 AutoOnLoad gates as the canonical retroactive case

The most concrete instance of partial-conflict semantics. An AutoOnLoad gate added in layer `L_b` does not retroactively fire against resources committed in earlier layers — the platform tolerates this quietly today. D33 makes the position explicit:

**v1 commits to forward-only semantics.** AutoOnLoad gates apply to commits made *at-or-after* the gate's containing layer. Lower-layer resources are grandfathered; their compliance with the gate is undefined. Any chain query asking "do all resources of class C satisfy gate G?" must explicitly distinguish "yes for resources committed at-or-after G's layer" from "unknown for earlier resources."

This is the simplest and least-surprising semantics for v1; it matches the platform's current behaviour. The cost is that operators auditing for compliance against retroactively-added gates have to do the work themselves (run the gate manually against historical resources).

**v2 may add opt-in retroactive validation.** A gate's declaration could carry a `retroactive_scan: bool` flag; setting it triggers a chain-wide scan at gate-commit time. If any historical resource fails, the gate's commit is rejected (or, alternatively, the gate's commit succeeds and a separate `Verdict` resource per failing historical resource is committed alongside, surfacing each violation for operator review). The trade-off is cost (chain-wide scan can be expensive) vs. honesty (the chain's compliance state is fully characterized).

### 7.8 Schema-narrowing and type-strengthening commits

These are *also* partial conflicts but currently untracked. v1 treats them like AutoOnLoad gates: forward-only semantics, no retroactive scan. The chain commit succeeds even if existing resources violate the new schema; queries asking "do all resources match the current schema?" must explicitly distinguish "yes for resources committed at-or-after the schema layer" from "unknown for earlier."

v2 could add the same `retroactive_scan` opt-in.

## 8. v1 / v2 scoping

### v1 — pure-addition partial-order chains

- Two-hash identity from D25 §X (prerequisite).
- Supporting-layer computation and indexed property on every committed layer.
- Commutativity decisions for pure-addition layer pairs (§4.5 conditions (1) and (2)); cached as `Commutativity` resources.
- Anchored-commit cache keyed on `(content_hash, supporting_layer_hash)`.
- Forward-only semantics for AutoOnLoad gates and constraint additions, made explicit in documentation and the chain commit messages.
- Consolidation gains the supporting-layer precondition (§7.5).
- Merge gains the partial-conflict taxonomy classification (§7.4); D20's resolution surface is unchanged.
- The chain stays linear at the user surface. Branch refs remain single-`position_hash`. Resolve walks the canonical linearization. The partial-order structure is *metadata* exposed for query but not used to relax linear-chain operations.

This is the smallest coherent unit. It delivers cell-output reuse (the immediate UX win), the supporting-layer concept (foundation for D25 v2 and D20 v2), and the partial-conflict taxonomy (informs all future schema-evolution work) without changing any existing user-facing operation's behaviour.

### v2 — full DAG operations

- Branch refs become frontiers; resolve walks topological orders chosen at query time; consolidation operates over antichains.
- Constraint-augmenting layers (§4.4 (a)–(d)) gain commutativity decisions, with the `Commutativity.basis` recording which partial-conflict class was inspected.
- Opt-in retroactive validation for AutoOnLoad gates and schema-narrowing commits.
- Distributed coordination: multiple kernels can commit independently to commuting parts of the chain; a separate process merges their commits as chain rewrites.
- Multi-frontier branch operations: `branch_advance_frontier(branch, new_layer)` adds to the frontier; existing operations like `eigenius load` / `eigenius run` accept either single-head or frontier-based branch references.

v2 is a substantial architectural change with non-trivial interactions across the codebase. v1 is the foundation; v2 lands when usage data justifies the scope.

### Out of v1 and v2 scope

- Permissions / privacy (§10.4).
- Proof-carrying commutativity claims (§10.5).
- Schema migration tooling (a layer that systematically transforms existing resources to satisfy a new constraint). Could be a separate phase after v2.

## 9. Migration story

D33 v1 is purely additive. No existing chain becomes invalid; no existing operation changes behaviour. Migration consists of:

### 9.1 Lazy supporting-layer back-fill

Existing layers don't have a `supporting_layer` property. The kernel adds the property lazily on first query — when an operation needs `supporting(L)` for a layer that doesn't have it, the kernel computes it (one walk over the reference graph) and stores the result. The computation is idempotent and side-effect-free; concurrent computations of the same layer's supporting deterministically produce the same result.

A background job can pre-compute supporting layers for all existing layers if operators want the index hot. The job is incremental; interruptible; resumable. The kernel's reachability sweep can integrate the back-fill (sweep already walks the chain; back-fill adds one property write per layer).

### 9.2 Canonical linearization

The current chain is one valid linearization. D33 v1 doesn't rewrite the chain to a canonical linearization — that would change `position_hash`s and invalidate trace pins. The chain stays in its current order; canonical linearization is computed *for new commits only*, ensuring fresh layers join in a deterministic position regardless of the order they were committed in.

For chains where operators want to canonicalize history (an audit-cleanness exercise), a separate `canonicalize_chain(branch)` operator can be added in v2 that rewrites the chain to canonical order, produces fresh `position_hash`s, and re-points trace pins via the supporting-layer-aware re-pointing policy from §7.3.

### 9.3 Partial-conflict scan as one-time chain audit

For chains that have accumulated constraint additions over time, a one-time audit can scan for historical resources that violate currently-applicable constraints. The audit is read-only (does not modify the chain); it produces a report listing each violating resource along with the constraint and the layer that introduced the violation.

This is operator-facing tooling, not part of v1's automated behaviour. Operators can run it at their own cadence; the report informs whether the chain is suitable for opt-in retroactive validation in v2.

## 10. Open questions

### 10.1 Distributed coordination

v2's multi-kernel concurrent commit story is sketched in §2.2 and §8, but the coordination protocol is undefined. Specifically: when two kernels each commit a layer that they each believe commutes with each other, how do they merge their commits without a conflict-resolution round-trip? Two candidate shapes:

- **Optimistic.** Each kernel commits independently; a third process (a coordinator, or one of the kernels acting as one) merges. Conflicts surface at merge time; resolution uses D20's existing strategies.
- **Pessimistic with leases.** Kernels lease IRI surfaces from a coordinator; commits within a lease are guaranteed independent. No merge step needed, but the coordinator becomes a scaling bottleneck.

The decision deserves its own design doc once usage data shows multi-kernel demand.

### 10.2 Retroactive AutoOnLoad firing

§7.7's v2 sketch (`retroactive_scan: bool` flag) is one shape; another is to defer the scan until queried (lazy retroactive validation). Both have merit; the choice depends on whether operators want commit-time knowledge or query-time knowledge of historical compliance.

### 10.3 Opt-in retroactive validation for schema changes

Same shape question as 10.2 but for schema-narrowing and type-strengthening commits. The trade-offs are similar.

### 10.4 Permissions and privacy

A future requirement that's worth holding space for: some layers may be private to one user / role / branch. Under partial-order semantics, "what's reachable from this branch" is well-defined; the question is whether the chain's *partial order itself* should encode visibility. Genuinely far-future; mentioned for completeness.

### 10.5 Proof-carrying commutativity claims

A speculative shape: a Lean-4 institution (D28) emits a verified proof that two layers commute according to some richer commutativity definition than §4.5's structural one (e.g., commutativity over a domain-specific equivalence relation). The kernel verifies the proof and admits the commutativity claim with `Commutativity.basis: ProofWitness`. Worth tracking as a hypothetical extension of the framework but not scoped.

### 10.6 Chain-rewriting tooling

§9.2's `canonicalize_chain(branch)` operator and similar chain-rewriting tools (e.g., `compress_partial_order`, `reorder_for_dedup`) sit between v1 and v2. They're individually small operators that benefit from D33's foundational concepts; their UX shape (CLI, gRPC, dry-run semantics) deserves its own decision pass.

### 10.7 Anchored-commit cache eviction

§6's cache is intentionally unbounded in v1 because each entry is small and cardinality is bounded by distinct cell content × supporting context. In long-lived production deployments with many notebooks, this may grow. Eviction strategies (LRU, TTL, content-hash-popularity-weighted) deserve a v2 decision based on observed cardinality.

## 11. Test plan and sub-milestone sequencing

### 11.0 Shared foundation with D25

Milestone 20a (two-hash identity split, supporting-layer computation, supporting-layer index, content-hash dedup index) is shared with D25 (chain consolidation) and lands as a single prerequisite PR per [D25 §11.0](d25-chain-consolidation.md). The remaining D33 milestones (20b–20e) and D25's milestones (17a–17e) build on that foundation independently and can land in any order against it. Recommended ordering reflects user-facing value: D25 v1 first (consolidation lands operator-facing capability), then D33's anchored-commit cache (20c — notebook UX win), then D33's commutativity and partial-conflict work (20b, 20d, 20e).

### 11.1 D33 milestones

```
PR 0 — shared foundation (delivered as 20a; see D25 §11.0)
                              │
                              ▼
                              ├─→ 20b commutativity decisions for pure-addition pairs
                              │
                              ├─→ 20c anchored-commit cache
                              │
                              ├─→ 20d partial-conflict taxonomy as commit metadata
                              │
                              └─→ 20e CLI + gRPC + docs ──→ shipping (v1)
```

| Milestone | Test surface | Pass criterion |
|---|---|---|
| 20a | Supporting-layer computation; index storage; lazy back-fill. | Computed `supporting` matches an oracle (independently-computed reference); back-fill is idempotent; concurrent computations yield identical results. |
| 20b | Commutativity decisions for pure-addition pairs. | Hand-constructed pairs with disjoint defined_iris and supporting-layer ordering are decided commutative; pairs with overlap or interleaved supporting are decided non-commutative; cached `Commutativity` resources match. |
| 20c | Anchored-commit cache: hit, miss, supporting-equivalent context. | Re-running an unchanged cell returns the cached `position_hash` without re-executing; supporting-context shifts that don't change the supporting layer also hit; genuine content or support changes miss. |
| 20d | Partial-conflict taxonomy: each class detected at commit time and surfaced as commit metadata. | Each of §4.4's four classes produces the expected commit-metadata classification; pure-addition layers are correctly classified as such. |
| 20e | CLI surfaces (`eigenius layer supporting <iri>`, `eigenius chain commutativity <a> <b>`, anchored-commit cache stats); gRPC parallels; documentation in platform guide. | End-to-end smoke: a notebook re-runs with cell-output reuse working; CLI commands return correct values; docs cover the v1 contract. |

Cross-cutting tests:

- **Resolve-equivalence regression.** D25's regression harness extends to verify that canonical linearization (§7.1 v1) produces identical resolves to the original commit order.
- **Determinism across linearizations.** For chains with commutative layer pairs, multiple linearizations of the same set of commits produce identical canonical linearizations and identical content_hashes (different position_hashes).
- **Forward compatibility check.** New commits with the supporting-layer index in place behave identically to commits made under the pre-D33 codebase (no behavioural regression; metadata is purely additive).

## 12. References

- D23 §5.2 — per-layer-bloom resolve walk this phase reframes
- D23 §5.2.2 — the per-IRI topmost-defining-layer index supporting-layer computation reuses
- D25 §X — two-hash identity model D33 builds on
- D25 §6 — top-of-stack consolidation algorithm that gains the supporting-layer precondition (§7.5)
- D25 §7.2 — trace re-pointing policy that becomes tractable under supporting-layer awareness (§7.3)
- D20 §4–§6 — categorical foundation D33 extends
- D20 §6 — six merge resolution strategies; partial-conflict taxonomy classifies their inputs (§7.4)
- D21 §3.6 — trace pinning that gains supporting-layer context
- D13 — durable kernel state; D33 does not modify the seed manifest
- D28 — Lean-4 as verification institution; potential consumer of proof-carrying commutativity (§10.5)
- Phase 20 in `docs/design/implementation-plan.md`

Source code touchpoints (entering Phase 20):

- `kernel/src/layer/supporting.rs` (new) — supporting-layer computation
- `kernel/src/layer/handle.rs` — `LayerHandle` extends with `supporting_layer` accessor
- `kernel/src/layer/cache.rs` — supporting-layer index storage; anchored-commit cache
- `kernel/src/storage/mod.rs` — `Storage` trait extension for supporting-layer index lookups
- `storage/memory/src/lib.rs`, `storage/rocksdb/src/lib.rs` — backend implementations
- `kernel/src/validation/mod.rs` — partial-conflict taxonomy classification at commit time
- `proto/eigenius_kernel.proto` — `LayerSupporting`, `ChainCommutativity`, `CellOutputCacheStats` RPCs
- `cli/src/layer/supporting.rs`, `cli/src/chain/commutativity.rs` (new) — CLI surfaces
- `docs/guides/platform/04-cli-reference.md`, `docs/guides/platform/14-notebook.md` — user docs
