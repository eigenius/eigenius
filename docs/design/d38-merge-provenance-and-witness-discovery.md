# D38: Merge provenance and witness discovery

*Design document for the Eigenius project — May 2026*

**Status:** Implemented (chain-resident `MergeResolutionRecord` per resolved conflict + off-span witness discovery)
**Builds on:** [D20 — Layer Reconciliation](d20-layer-reconciliation.md), [D36 — Merge Resolution UX](d36-merge-resolution-ux.md), [D37 — Lambda surface and typed merge comorphisms](d37-lambda-surface-and-typed-merge-comorphisms.md).
**Closes:** D36 §15.6 (Resolution attribution: deferred).
**Forward-references:** D39 — Resolution strategy UX, second pass (the broader UX redesign that D37 §10.6 sketched; previously referred to as D38 but renumbered here).

---

## 1. Overview

After D37 lands, the merge resolution flow is end-to-end functional: typed witnesses authored in ESL, validated at commit time, surfaced by the notebook picker, applied by the resolver. Two gaps surfaced during D37's end-to-end testing that aren't structurally about *authoring* witnesses but about *living with them once committed*:

1. **No provenance record of the resolution.** Today the merge layer carries its parents, its resolved bodies, and a free-form `name` string the caller passes (e.g., `"merge:rename_collision"`). There is no chain-resident record of *which strategy resolved each conflict*, *which witness was applied*, or *which branch contributed each conflicting body*. The merge layer's result is auditable; its reasoning is not.

2. **Witness discovery is scoped to the merge span only.** `resolve_merge_comorphism` searches `span.sources_a`, `span.sources_b`, and the ancestor's chain. Witnesses committed on an unrelated branch (a witness-library branch, a CI-managed centralised catalogue, an in-progress author-then-merge workflow) are unreachable, even if the user explicitly names the IRI. The resolver returns `MergeComorphismNotFound`.

Both are workflow-shaped rather than algorithm-shaped problems. The witness *machinery* works fine; the *scope* of when it's findable and the *trace* of when it ran are missing. D38 closes both.

### 1.1 Scope

- A `MergeResolutionRecord` ontology class committed alongside the resolved bodies in every merge layer, one record per resolved conflict.
- **Witness availability in the merge layer's reachable chain.** Whenever a Witness resolution applies a `MergeComorphism`, the merge layer is guaranteed to resolve both the comorphism and its transformation Lambda — making the merge layer a self-contained audit record. The implementation uses a *narrow copy*: when the witness lives outside the merge span (PR 2's `witness_search_branches` case), the comorphism + transformation are committed into the merge layer's contributions at their original IRIs. When the witness lives inside the span (`sources_a` / `sources_b` / ancestor chain), no copy is emitted — the merge layer's own parent chain already pins those layers transitively, so duplicating the bodies into contributions would waste storage with no GC benefit.
- An extension to `resolve_merge_comorphism` that accepts an optional list of additional branches/tags to search for the comorphism beyond the merge span itself, with **apply-time class enforcement unchanged** so cross-class misuse remains rejected.
- A `SubmitResolutionRequest` wire-format addition exposing the search-scope extension to the SDK and notebook.
- The notebook's `WitnessEditor` Combobox respecting the same expanded scope so the discovery surface matches what the apply path accepts.
- A History-panel surface for inspecting a merge layer's resolution records (basic; rich UX is D39's territory).

### 1.2 Out of scope

- Resolution-strategy UX redesign — D39's domain (the visual witness builder, applicability surfacing, kind-specific picker affordances).
- Multi-way merges (more than two branches in a single resolution). D20 §11 already flagged this as v2 work; not affected by D38.
- Cascade-record provenance (per-IRI orphaned-reference / orphaned-typing / invalidated-signature audit trail). The cascade ack list already on the wire covers the immediate need; richer cascade attribution is independent.

---

## 2. Status today

**Merge layer construction** ([kernel/src/layer/merge.rs:2440](../../kernel/src/layer/merge.rs#L2440) — `commit_resolutions_as_merge_layer`):

- Builds a multi-parent layer with both branch tips as parents.
- Commits the resolved bodies (witness-merged, renamed, kept, restructured) as the layer's contributions.
- Tombstones any IRIs that the resolution drops.
- Names the layer with a caller-supplied string. **That string is the only chain-resident hint about what happened.**

**Witness lookup** ([kernel/src/layer/merge.rs:1118-1126](../../kernel/src/layer/merge.rs#L1118-L1126) — `resolve_merge_comorphism`):

> "Search both branches' contributions before falling back to the ancestor's chain. v1 doesn't require the comorphism to live strictly under the ancestor — D20 §6.1 leaves the chain location open as long as the resource is reachable from the merge span."

So today: span-only search. Any witness IRI outside the merge span surfaces as `MergeComorphismNotFound`.

**SubmitResolution wire** ([proto/eigenius.proto](../../proto/eigenius.proto) — `SubmitResolutionRequest`):

- Carries `branch`, `candidate_head`, the per-conflict `MergeResolutionWire` list, and the `CascadeAck` list. No place to express "also search these branches for the witness."

**Notebook WitnessEditor query** ([notebooks/src/components/merge/WitnessEditor.tsx](../../notebooks/src/components/merge/WitnessEditor.tsx) — `queryApplicableComorphisms`):

- Runs an EigenQL query for `MergeComorphism` resources with the matching `merge_target_class`, scoped to the SDK's default branch (which mirrors the workspace's active branch). The user is on the merge target during resolution, so the query reads against that branch's chain — same scope as the resolver. If the resolver can't find a witness on a third branch, the picker doesn't surface it either.

---

## 3. Merge provenance records

### 3.1 Resource shape

A new `urn:eigenius:core:MergeResolutionRecord` Class, committed by `commit_resolutions_as_merge_layer` once per resolved conflict, alongside the resolved bodies in the same merge layer.

```json
{
  "@id": "urn:eigenius:auto:merge-record:<sha256>",
  "urn:eigenius:core:is_a": ["urn:eigenius:core:MergeResolutionRecord"],
  "urn:eigenius:core:merge_record_conflict_id": "iri_collision:urn:project:patient_42",
  "urn:eigenius:core:merge_record_strategy": "Witness",
  "urn:eigenius:core:merge_record_witness": "urn:project:patient_take_b",
  "urn:eigenius:core:merge_record_branch_a_source_layer":
    "abc123…def",
  "urn:eigenius:core:merge_record_branch_b_source_layer":
    "fed321…cba",
  "urn:eigenius:core:merge_record_ancestor_source_layer":
    "0a1b2c…3d4e"
}
```

**Required slots** on every record:

| Property | Type | Purpose |
|---|---|---|
| `merge_record_conflict_id` | `core:string` | The classifier-emitted conflict id (e.g., `"iri_collision:<iri>"`). Lets readers correlate the record with a fresh `PreviewCascade` run. |
| `merge_record_strategy` | `core:string` | One of `"Witness"`, `"Rename"`, `"SchemaQuotient"`, `"Restructure"`. The strategy name; per-strategy detail lives in optional slots. |
| `merge_record_branch_a_source_layer` | `core:string` | Hex layer id of the layer that contributed branch A's body at the conflict IRI. |
| `merge_record_branch_b_source_layer` | `core:string` | Hex layer id of branch B's contributing layer. |

**Strategy-specific optional slots:**

| Property | Set for strategy | Value |
|---|---|---|
| `merge_record_witness` | `Witness` | IRI of the `MergeComorphism` applied. The comorphism resource is copied into this same merge layer (see §5.3), so the IRI always resolves on the merge layer's own chain. |
| `merge_record_witness_source_layer` | `Witness` | Hex layer id of the layer the witness was *originally* committed on (before the copy into the merge layer). Preserves provenance back to the authoring layer for audit purposes. |
| `merge_record_rename_side` | `Rename` | `"a"` or `"b"` — which side was renamed. |
| `merge_record_rename_from_iri` | `Rename` | The IRI being renamed away from. |
| `merge_record_rename_to_iri` | `Rename` | The new IRI. |
| `merge_record_quotient_kind` | `SchemaQuotient` | `"KeepBoth"`, `"KeepOne"`, or `"KeepNeither"`. |
| `merge_record_quotient_winner` | `SchemaQuotient` with `KeepOne` | `"a"` or `"b"`. |
| `merge_record_restructure_new_parent` | `Restructure` | IRI of the introduced common parent class. |
| `merge_record_restructure_affected_class` | `Restructure` | IRI of the class being re-parented. |
| `merge_record_ancestor_source_layer` | All | Hex layer id of the ancestor's body for this IRI, when one exists. Absent for IRIs that didn't exist at the LCA. |

**`@id` derivation:** content-hash over the record's canonical Eigon-CBOR (mirroring §4.3's synthesised-lambda IRI scheme from D37). Identical resolutions of the same conflict produce the same record IRI, so reading the merge layer's records gives a deterministic view across runs.

### 3.2 Compiler / builder integration

`commit_resolutions_as_merge_layer` ([kernel/src/layer/merge.rs:2440](../../kernel/src/layer/merge.rs#L2440)) gets two new builder steps run alongside each existing per-strategy arm:

1. After each resolution's body emission (the existing `builder.add_resource(merged)` / `builder.add_resource(body)` calls), construct the appropriate `MergeResolutionRecord` resource from the resolution's shape + the span's source-layer information.
2. Compute its content-hash IRI.
3. `builder.add_resource(record)`.
4. **For `Witness` resolutions only**: read the resolved `MergeComorphism` body (via the same lookup the resolver used) plus the transformation Lambda it points at. For each, check whether the IRI is reachable through the merge span (`find_in_span_chain` returns `Some`). If yes — the merge layer's parent chain already pins it transitively — skip the copy. If no — the witness lives off-span (PR 2's `witness_search_branches` case) — `builder.add_resource(body)` at the original IRI so the merge layer becomes the canonical residency for the witness from that point forward. Both writes (when emitted) are idempotent because the bodies are deterministic and `add_resource` upserts by IRI, so re-commits produce the same merge-layer hash.

This is additive — no new validators on the cascade path, no schema changes to the cascade gate, no wire-format break. Each merge layer carries N more chain-resident resources (one record per resolved conflict, plus — only for off-span Witness resolutions — one comorphism + one lambda per such resolution).

### 3.3 Consumption

**EigenQL discovery query** (what the History panel would run):

```eigenql
USING "urn:eigenius:core:MergeResolutionRecord"

MATCH MergeResolutionRecord(?r) {
    "urn:eigenius:core:merge_record_conflict_id": ?conflict,
    "urn:eigenius:core:merge_record_strategy": ?strategy
}
RETURN [] {
    record: ?r,
    conflict: ?conflict,
    strategy: ?strategy
}
ORDER BY ?conflict
```

`MergeResolutionRecord`s are committed in the merge layer alongside the resolved bodies, so the query reads the merge layer's contributions directly.

**Witness-instance audit query** ("show me every conflict resolved by `patient_take_b`"):

```eigenql
USING "urn:eigenius:core:MergeResolutionRecord"

MATCH MergeResolutionRecord(?r) {
    "urn:eigenius:core:merge_record_witness": ?w,
    "urn:eigenius:core:merge_record_conflict_id": ?conflict
}
WHERE ?w = "urn:project:patient_take_b"
RETURN [] {
    record: ?r,
    conflict: ?conflict
}
```

The presence of the record on the chain is what makes this query possible at all. Without it, the only way to find merge-layer-resolved conflicts is to re-run the classifier against historical chain states.

### 3.4 Notebook surface — minimal

The History panel's existing detail panel (read pin → layer's resource list) already surfaces every resource in a selected layer. With §3.1 records committed, those records show up automatically. A small dedicated affordance for "show the resolution trace for this merge layer" is a nice-to-have but D38 v1 ships with the records as plain chain-resident resources; rich rendering can land later or as part of D39.

---

## 4. Witness discovery scope

### 4.1 The structural change

`resolve_merge_comorphism`'s search path ([kernel/src/layer/merge.rs:1118-1126](../../kernel/src/layer/merge.rs#L1118-L1126)) currently falls through:

1. `span.sources_a` — IRIs branch A contributed since the LCA.
2. `span.sources_b` — IRIs branch B contributed since the LCA.
3. The ancestor chain — `find_iri_in_chain(span.ancestor, …)`.

D38 adds an optional fourth tier: **explicit search branches** supplied by the caller via a new `SubmitResolutionRequest` field.

```
search_branches: Vec<String>
```

Each entry is a branch name (or a tag name). For each entry, the resolver loads that branch's tip via `backend.get_branch(...)` and walks its chain via `find_iri_in_chain`. The first hit wins; class-equality enforcement (`merge_target_class` matching the conflict's class) happens after lookup as today, so cross-class misuse is still rejected — the wider search only relaxes *where* the witness can live, not *which* witnesses are valid.

The walk order is deliberate:

1. Branch sources (most likely place for a freshly-committed in-context witness).
2. Ancestor chain (the canonical inherited location).
3. Explicit search branches (catch-all for cross-branch references).

So an unscoped witness on a sibling branch doesn't shadow a "real" witness reachable from the merge span; users opt into broader scope by naming the branches.

### 4.2 Wire-format addition

```proto
message SubmitResolutionRequest {
  // ... existing fields ...
  // D38 §4.1 — additional branches the resolver should consult when
  // looking up `comorphism_iri` references. Each entry is a branch
  // name (or a tag name). Searched in declared order after the merge
  // span's sources and the ancestor chain. Class-equality enforcement
  // applies regardless of where the witness was found.
  repeated string witness_search_branches = ...;
}
```

The same field needs adding to `PreviewCascadeRequest` so the picker can preview-resolve too without the witness being on the merge span.

### 4.3 SDK + notebook integration

`Eigen.submitResolution(...)` and `Eigen.previewCascade(...)` gain a `witnessSearchBranches?: string[]` option. The notebook's resolution-flow store actions pass it through.

The `WitnessEditor`'s Combobox query reads against the SDK's default branch by default, which during resolution is the merge target. To surface witnesses on the user-specified search branches, the query needs to run **once per search branch** (or use EigenQL's chain-walking semantics — see §10.2). The simplest v1: the user expands "Search additional branches" in the picker, types a branch name, the editor adds it to a local list, the query refires reading that branch's chain. The selected list is what populates `witnessSearchBranches` on the eventual submit.

### 4.4 Worked example

User has a `witness-library` branch holding a catalogue of reusable comorphisms. They're resolving a conflict on `main`:

1. Picker opens with the default scope. Combobox queries `main`'s chain, finds no `patient_take_b`.
2. User opens "Search additional branches", types `witness-library`. The editor re-runs the query against `witness-library`'s chain, finds `patient_take_b`, surfaces it in the Combobox.
3. User selects it. Resolution state captures `witnessSearchBranches: ["witness-library"]`.
4. Preview cascade — `previewCascade` carries the search branches, resolver finds the witness via the new fourth-tier walk.
5. Commit — `submitResolution` carries the search branches, resolver finds the witness, apply path runs as today. The merge layer's `MergeResolutionRecord` (§3.1) captures `merge_record_witness = urn:project:patient_take_b` and `merge_record_witness_source_layer = <witness-library tip layer id>`. The §3.2 step-4 guard detects that the witness is off-span (`find_in_span_chain` returns `None`), so the comorphism body plus its transformation Lambda are committed into the merge layer's contributions. Even if `witness-library` is later deleted, the merge layer's resolution trace remains fully resolvable: the IRI lookup hits the merge layer's own contribution, with the `merge_record_witness_source_layer` slot preserving the historical attribution.

---

## 5. Kernel changes

### 5.1 Core ontology

[ontologies/core/core-ontology.json](../../ontologies/core/core-ontology.json) gains the `MergeResolutionRecord` Class declaration plus its required + optional properties (per §3.1).

### 5.2 Well-known IRI constants

[kernel/src/ontology/well_known.rs](../../kernel/src/ontology/well_known.rs) gets:

```rust
pub const MERGE_RESOLUTION_RECORD: &str =
    "urn:eigenius:core:MergeResolutionRecord";
pub const MERGE_RECORD_CONFLICT_ID: &str =
    "urn:eigenius:core:merge_record_conflict_id";
pub const MERGE_RECORD_STRATEGY: &str =
    "urn:eigenius:core:merge_record_strategy";
pub const MERGE_RECORD_WITNESS: &str =
    "urn:eigenius:core:merge_record_witness";
// … and the rest of the §3.1 slots.
```

### 5.3 `commit_resolutions_as_merge_layer`

One new helper per strategy variant that builds the appropriate `MergeResolutionRecord` resource from the resolution + the conflict's source-layer info:

```rust
fn build_merge_resolution_record(
    conflict: &TypedConflict,
    resolution: &MergeResolution,
    span: &MergeSpan,
    // For Witness resolutions, the layer id from which `resolve_merge_comorphism`
    // pulled the comorphism resource. Threaded into `merge_record_witness_source_layer`.
    witness_source_layer: Option<&LayerId>,
) -> Resource { … }
```

Called immediately after each per-strategy arm's body emission (so the record commits in the same layer as the resolved body).

**Witness copying — guarded.** For `Witness` resolutions, the existing apply path already loads the comorphism body via `resolve_merge_comorphism`. After the body emission and record emission steps, the builder evaluates a per-resource guard:

1. For the comorphism IRI: `find_in_span_chain(iri, span, …)`. If `Some(_)` is returned (the resource lives somewhere on `sources_a` / `sources_b` / the ancestor's chain), the merge layer's parents already reach the body transitively — skip the copy. If `None`, the resource lives off-span (PR 2's `witness_search_branches` surface) and the merge layer would otherwise not be self-contained — `builder.add_resource(comorphism_body.clone())` at the comorphism's IRI.
2. For the transformation Lambda: same guard, against the transformation's IRI. The two checks are independent — the comorphism and its transformation can in principle live on different branches.

Both writes (when emitted) are idempotent: re-committing the same merge with the same witness produces the same resource bytes at the same IRIs, so the merge-layer hash is stable. The guard makes the merge layer's reachability the load-bearing invariant: every witness referenced from a `MergeResolutionRecord` is resolvable through the merge layer — either via a contribution (off-span case) or via the merge layer's parent chain (in-span case).

### 5.4 `resolve_merge_comorphism` — fourth-tier search

`resolve_merge_comorphism` gains an `extra_branches: &[String]` parameter. After the existing three-tier walk falls through:

```rust
for branch_name in extra_branches {
    let Some(branch_tip) = backend.get_branch(branch_name)
        .map_err(MergeError::Storage)?
    else {
        continue; // unknown branch — skip
    };
    if let Some((layer_id, resource)) =
        find_iri_in_chain(&branch_tip, iri, topology, backend)
            .map_err(MergeError::Storage)? {
        return Ok((layer_id, resource));
    }
}
```

Class-equality check stays where it is. Caller-side validation of the branch list (no empty strings, max length) lives in the server handler.

### 5.5 Server handler updates

`submit_resolution` and `preview_cascade` in [kernel/src/server/mod.rs](../../kernel/src/server/mod.rs) read the new `witness_search_branches` field and pass it through to `merge_with_resolutions` / `preview_cascade` / `resolve_merge_comorphism`.

---

## 6. SDK changes

[clients/eigenius-ts/src/client.ts](../../clients/eigenius-ts/src/client.ts) gains `witnessSearchBranches?: string[]` on the `SubmitResolutionOptions` and `PreviewCascadeOptions` interfaces, threading the field into the request proto.

---

## 7. Notebook changes

### 7.1 WitnessEditor — search-branches affordance

Below the Combobox, add a collapsed "Search additional branches" disclosure that, when expanded, shows a small list of comma-separated branch / tag names with a Combobox-of-branches picker. The list updates as the user adds entries; the picker's EigenQL query re-fires per-branch.

The simplest v1 UI shape:

```
[Combobox showing applicable comorphisms from default scope]
[+] Search additional branches  ▼
    [Branch picker]  [Add]
    Current: witness-library, main:prod-witnesses
```

### 7.2 mergeResolution state

The resolution-flow state ([notebooks/src/runtime/mergeResolution.ts](../../notebooks/src/runtime/mergeResolution.ts)) gains a `witnessSearchBranches: string[]` field on the `picking` / `previewing` / `acknowledging` / `committing` variants. The store actions `previewMergeCascade` and `commitMergeResolution` pass it through the SDK.

### 7.3 History-panel surface (minimal)

No new UI; the per-layer resource list already surfaces `MergeResolutionRecord` resources via the existing read-pin path. A richer "resolution trace" view (per-conflict cards, links to applied witnesses, source-layer comparisons) is D39's responsibility.

---

## 8. Phasing / rollout

Three PRs. Each individually shippable; §3 (provenance records) is fully independent of §4 (witness discovery scope), so the order is flexible.

### PR 1: Merge provenance records + witness copying

- Core ontology: `MergeResolutionRecord` class + its properties (including `merge_record_witness_source_layer`).
- Well-known IRI constants.
- `commit_resolutions_as_merge_layer` emits one record per resolution.
- For `Witness` resolutions: copy comorphism resource + transformation Lambda into the merge layer at their original IRIs.
- Kernel unit tests: each strategy variant produces the expected record shape; Witness merges include the comorphism + lambda contributions; re-commits are idempotent (same layer hash).

Estimated effort: ~1.5–2 days.

### PR 2: Witness discovery scope

- Proto: `witness_search_branches` on `SubmitResolutionRequest` + `PreviewCascadeRequest`.
- Kernel: `resolve_merge_comorphism` extra-branches argument + server handler plumbing.
- SDK: `witnessSearchBranches` option on the typed wrappers.
- Notebook: WitnessEditor search-branches affordance + state-machine field.
- Tests: kernel unit tests for the fourth-tier search, notebook smoke for the UI.

Estimated effort: ~2 days. Mostly plumbing; the kernel-side change is small.

### PR 3: Minor polish + docs

- Update D36 §15.6's deferred attribution note to point at D38.
- Add an entry to the D36 manual test scenarios doc covering the new provenance records (verify they appear in the Layer Inspector after a merge).
- (Optional) Add a single dedicated query helper in the notebook for "show resolution trace of layer X" — primarily for testing, not a polished UI.

Estimated effort: ~0.5 day.

### Total

Roughly **4–4.5 days** of focused work. Independent of the D39 UX work; both can land in parallel.

---

## 9. Validation plan

### PR 1

- Unit test: Rename resolution produces a record with `strategy = "Rename"`, `rename_side`, `rename_from_iri`, `rename_to_iri` populated.
- Unit test: SchemaQuotient (each of the three kinds) produces the expected `quotient_kind` + `quotient_winner` (where applicable).
- Unit test: Witness produces `strategy = "Witness"`, `witness = <comorphism IRI>`, `witness_source_layer = <original layer id>`.
- Unit test: in-span witness — `find_in_span_chain` returns `Some`, the comorphism + transformation are *not* duplicated into the merge layer's `defined_iris`, but both still resolve through the merge layer's parent chain.
- Unit test: off-span witness (lands with PR 2, which exposes the search-branches surface) — the comorphism + transformation appear as merge-layer contributions at their original IRIs.
- Unit test: re-committing the same merge with the same Witness resolution produces an identical layer hash (idempotent witness copy / no-copy path).
- Unit test: Restructure produces `restructure_new_parent` + `restructure_affected_class`.
- End-to-end: commit a merge through the existing `merge_with_resolutions` test fixtures, verify the merge layer's resources include the records, run the EigenQL discovery query.

### PR 2

- Unit test: `resolve_merge_comorphism` with empty `extra_branches` matches today's behaviour.
- Unit test: witness on a third branch resolved via `extra_branches` succeeds.
- Unit test: witness with wrong class still rejected when found via `extra_branches`.
- Unit test: unknown branch in `extra_branches` is silently skipped (vs. erroring) — supports "best-effort search" semantics.
- End-to-end via the notebook: the WitnessEditor's expanded scope surfaces witnesses on a sibling branch.

### PR 3

- Manual: open a merge layer in the History panel's Layer Inspector, confirm `MergeResolutionRecord` resources are listed.

---

## 10. Open questions

### 10.1 EigenQL across multiple branches

§4.3 noted that the picker would need to query per-search-branch sequentially. An EigenQL extension that takes a list of branch tips (or a tag set) and runs the MATCH against the union of their chains would be cleaner, but it's outside D38's scope. Sequential per-branch queries are good-enough for v1 with typical N ≤ 5 search branches.

### 10.2 Multi-witness resolutions

Today every conflict in a single resolution uses one strategy. A future feature might let a Witness resolution chain *multiple* comorphisms (apply one, then another). The current record shape carries only one `merge_record_witness` IRI. If multi-witness lands, the record would extend to a `merge_record_witnesses: [iri, ...]` array. Out of scope here.

### 10.3 Cascade record provenance

Each cascade item (orphaned reference, orphaned typing, etc.) was acknowledged by the user before commit. Should the merge layer also carry an audit record of *which* cascade items were acked? Useful for compliance but verbose. Lean: skip for v1 — the cascade ack list is part of the wire request, which is logged by the server's RPC trace. The compliance answer can be reconstructed from logs without a chain-resident record.

### 10.4 D39 dependency

D39's UX redesign will want to surface the merge provenance records in a polished "resolution trace" view, and the witness-discovery search affordance will land in D39's redesigned WitnessEditor rather than as a separate disclosure. D38's notebook-side changes (§7) are minimal-viable; D39 can replace them without breaking the wire-format / kernel surface that D38 establishes.

---

## 11. References

- [D20 — Layer Reconciliation](d20-layer-reconciliation.md) §6.1 — the witness contract D38 traces.
- [D36 — Merge Resolution UX](d36-merge-resolution-ux.md) §15.6 — the deferred-attribution note this closes.
- [D37 — Lambda surface and typed merge comorphisms](d37-lambda-surface-and-typed-merge-comorphisms.md) — establishes the ESL-authored witness surface whose application is what D38 records.
- `kernel/src/layer/merge.rs` — `commit_resolutions_as_merge_layer`, `resolve_merge_comorphism`, the integration points.
- `proto/eigenius.proto` — `SubmitResolutionRequest` / `PreviewCascadeRequest` extensions.
