# D34: Notebook Chain Workspace

*Design document for the Eigenius project — May 2026*

**Status:** Implemented (Phase D34; workspace rail destinations — Branches / Tags / History / Merge / Compaction / GC / Layer / Institutions / Health / Topology — live in the notebook)
**Required before:** Phase 21 (Life-Science Worked Examples) — the worked notebooks need branch-aware authoring, history, compaction, and merge visibility to be teachable.
**Builds on:** [D22 — Notebook UX and TypeScript SDK](d22-notebook-and-typescript-sdk.md), [D23 — Out-of-Core Layer Architecture §5.4–§5.5](d23-out-of-core-layer-architecture.md), [D25 — Chain Consolidation](d25-chain-consolidation.md), [D33 — Partial-Order Chains §6](d33-partial-order-chains.md).
**Resolves:** Notebook workspace IA, branch picker / branches / history / tags / merge UX, compaction wizard, GC trigger, anchored-commit cache visibility, task surfacing — and the kernel gaps each of these exposes.

---

## 1. Overview

D22 shipped the notebook MVP: cells, execution, layer-stack visualisation, topology view. Everything runs against the implicit `"main"` branch. None of the chain operations the kernel exposes — branches, consolidation, merges, the anchored-commit cache, tasks — is visible to the notebook user.

This document specifies the **notebook chain workspace**: the UX layer that exposes the chain as a first-class workspace metaphor. The notebook author picks a branch, watches their commits advance (or merge, or cache-hit), views history and tags, triggers compaction with a preview, fires GC, and recovers from merge conflicts — all without leaving the browser.

The doc has a second purpose: **forcing function**. Several kernel surfaces are partially implemented (trivial merge runs but isn't surfaced), missing wire format (no tag RPC, no GC RPC), or have correctness gaps (silent `NeedsWitnessedMerge`). Specifying the UX makes the kernel gaps explicit and schedulable. Each gap is called out inline with a §G.x label.

### 1.1 Audience

- **Notebook authors and ontology engineers** — the primary consumer. The UX must teach the chain model through use without requiring the user to read D23 first.
- **Implementers** — frontend (notebook + SDK), backend (kernel gaps), and the orchestrator team wiring new Connect routes.
- **Design reviewers** — the kernel-gap inventory in §10 is the schedule-bearing artifact.

### 1.2 Non-goals

- **Witnessed merge UX.** Phase 15's comorphism-mediated conflict resolution is out of scope. This doc covers trivial merge (D23 §5.4.3) outcomes and how the UI handles `NeedsWitnessedMerge` as a recoverable error, not as an in-place resolution flow.
- **Multi-user collaboration.** Same single-user MVP scope as D22.
- **Server-side notebook persistence.** Notebooks remain local files.
- **History-rewriting operations** (rebase, squash beyond what `ConsolidateChain --preserve_history=false` already gives, force-push equivalents). The chain is append-only; the workspace exposes that honestly.
- **Cross-branch atomic operations** (e.g., "cherry-pick this layer onto that branch"). The Phase 14 design is per-branch CAS; cross-branch flows are user-mediated sequences of single-branch ops.

### 1.3 Relationship to other documents

- **D22 §2.1** defines the three-tier topology (browser ↔ orchestrator Connect ↔ kernel gRPC). This doc adds new orchestrator routes and consumes additional kernel RPCs; it does **not** change the architecture.
- **D23 §5.4** defines `update_branch`, `ConflictPolicy`, `UpdateOutcome`. The kernel implements trivial merge; this doc surfaces it.
- **D23 §5.5** defines branches as named refs over the layer DAG. This doc is the user-facing surface for that abstraction.
- **D25** defines `ConsolidateChain` semantics. This doc designs the wizard that drives it.
- **D33 §5** defines redirect entries (a consolidation by-product). This doc visualises them in history. §6 defines the anchored-commit cache; this doc shows when it fires.
- **D21** defines tasks. This doc surfaces task state in the workspace.

---

## 2. Server surface inventory

### 2.1 Shipped and ready to use

| RPC                          | Status | Notebook today | Gap |
|------------------------------|--------|----------------|-----|
| `Load`                       | ✓     | Used (every cell that mutates) | Consumes `branch_advanced`? — no (§G.5) |
| `RunProgram` / `RunProgramByIri` | ✓ | Used | Same |
| `Reflect`                    | ✓     | Unused          | Same |
| `Query`                      | ✓     | Used (read + INTO) | Same |
| `Inspect`                    | ✓     | Used            | — |
| `ValidateProgram`            | ✓     | Unused          | Consider for cell-level lint |
| `LayerTopology`              | ✓     | Used (`LayerStackView`, `TopologyGraphView`) | Branch-aware filter (§G.6) |
| `ListBranches`               | ✓     | Unused          | — |
| `GetBranch`                  | ✓     | Unused          | — |
| `CreateBranch`               | ✓     | Unused          | — |
| `DeleteBranch`               | ✓     | Unused          | — |
| `ConsolidateChain`           | ✓     | Unused          | — |
| `EstimateConsolidation`      | ✓     | Unused          | — |
| `ListTasks` / `GetTaskStatus` / `CancelTask` | ✓ | Unused | — |
| `ListInstitutions` / `GetSchema` | ✓ | Unused (counts only — `LayerStackView` shows "this layer added N institutions" but no inventory view) | Enrich `InstitutionInfo` (§G.8) |
| `Health`                     | ✓     | Used (banner)   | — |

### 2.2 Partially shipped — wire-up needed

| Feature | What's there | What's missing | Gap |
|---------|--------------|----------------|-----|
| Trivial merge (D23 §5.4.3) | `update_branch(AllowTrivial)` produces merge layer; every commit RPC opts in. | The outcome (`FastForward` / `TrivialMerge { merge_layer }` / `NeedsWitnessedMerge { conflicting_iris }`) is **swallowed** in [`advance_branch_for_layer`](../../kernel/src/server/mod.rs) — all three flatten to `Ok(())`. `NeedsWitnessedMerge` silently looks like success even though the branch ref didn't advance. | §G.1 |
| Anchored-commit cache | `branch_advanced` flag landed in `LoadResponse` / `RunProgramResponse` / `ReflectResponse` / `QueryResponse`. | SDK + notebook consume it. | §G.5 |
| History walk | `LayerTopology` returns the DAG; `LayerStackView` linearises it. | Branch-scoped history endpoint; per-layer metadata (name, timestamp, resource_count) is available on `LayerHandle` but not aggregated for "log" UX. Cursored fetch for long histories. | §G.6 |
| Redirect visualisation | `LayerHandle.is_redirect_source` flag plumbed; `LayerTopology` emits synthetic tombstones. | Notebook history needs to render "consolidated into L" gracefully. | §G.6 |

### 2.3 Not shipped — new kernel surface

| Feature | Needs | Gap |
|---------|-------|-----|
| Tags | Storage trait (`PersistentBackend::list_tags` / `put_tag` / `delete_tag`), `TagsRequest`/`Response` RPCs, on-disk column family. Semantics: immutable named ref to a `LayerId`; deletion allowed, mutation not. | §G.2 |
| Explicit `MergeBranches` RPC | Wraps `update_branch(target, target_tip, source_tip, AllowTrivial)`. Returns the same `UpdateOutcome` shape §G.1 introduces. | §G.3 |
| GC trigger RPC | `crate::gc::collect` exists; needs `RunGc` / `EstimateGc` RPCs with dry-run preview + admin-gated execution. | §G.4 |

---

## 3. Information architecture

### 3.1 Workspace metaphor

The chain workspace is **one persistent left rail + one main pane**, not modal dialogs over the notebook. The rail is the workspace; the main pane is the focused view.

```
┌──────────────────────────────────────────────────────────────────────┐
│ [branch: kinase-screen ▾]   tip: a4f2…3c1   ●  ⓘ unsaved             │ ← header
├──────────────┬───────────────────────────────────────────────────────┤
│  Notebook    │                                                       │
│              │                                                       │
│  ▾ Chain     │                                                       │
│    Branches  │                                                       │
│    History   │                  Main pane                            │
│    Tags      │                  (one of the rail items)              │
│    Merge     │                                                       │
│              │                                                       │
│  ▾ Workspace │                                                       │
│    Topology  │                                                       │
│    Institut. │                                                       │
│    Tasks     │                                                       │
│              │                                                       │
│  ▾ Admin     │                                                       │
│    Compact.  │                                                       │
│    GC        │                                                       │
│              │                                                       │
│  Health      │                                                       │
└──────────────┴───────────────────────────────────────────────────────┘
```

The Fluent UI [`Nav`](https://fluent2.microsoft.design/components/web/react/core/nav/usage) component is the right primitive: it gives us hierarchical grouping (Notebook stays at the top; chain operations group together; admin actions sit at the bottom), persistent selection state, expand/collapse for nested categories, and keyboard navigation for free.

**Alternatives considered:**

- *Tabs across the top.* Doesn't scale past ~5 tabs; chain operations alone need 6–8. Tabs also flatten hierarchy that has a natural shape (admin actions are different in kind from authoring).
- *Drawer / overlays.* Each operation as a modal overlay over the notebook. Bad for a workspace metaphor — modals are for transient input, but the user spends real time in History and Compaction and wants those persistent.
- *Split-pane with all panels visible.* Tried in prototypes; loses the notebook to a sliver and crowds the chrome.

Nav wins on hierarchy + persistence. Modals stay reserved for **transient input** — confirmation, "create branch", "create tag" — not for views the user inhabits.

### 3.2 Header — always visible

A single fixed-height header above the rail and main pane carries three pieces of state:

1. **Branch picker** — `[ branch: <name> ▾ ]` button → menu of all branches, ordered by recency, with a footer action "Create branch…". Switching branches reloads the notebook's session pin (a kernel-level concept from D21).
2. **Tip indicator** — `tip: <short-hash>` next to the picker, with a hover card showing the full id, the layer's `name`, `created_at`, and `resource_count` (data from `GetBranch`).
3. **Unsaved-notebook indicator** — `●` dot when the local `.eigon-notebook.json` has unsaved cell edits. Separate from chain state.

The header is the only place the user sees "where am I writing?" — keeping it always visible solves the recurring "wait, which branch did I just commit to?" problem.

### 3.3 Default rail selection

The notebook tab is selected by default. Users who never leave the notebook see no change from the D22 MVP except the new header. The chain operations are present-but-inert until needed.

---

## 4. Branch picker and branches panel

### 4.1 Picker (in header)

A Fluent UI `Menu` triggered from the header button. Items:

```
● main                 tip a4f2…3c1   2 min ago
○ kinase-screen        tip 9b81…7e2   12 min ago      ← currently active
○ wip-mtt-rewrite      tip 3d4f…9a0   yesterday
○ auto-2026-05-10      tip 5c12…0e8   3 days ago

[+ Create branch…]
```

- Current branch is marked.
- Each item shows the short tip hash and a relative-time stamp from `GetBranch`.
- Branches matching `^auto-\d{4}-\d{2}-\d{2}` (the D23 §5.4.4 sibling-branch naming convention) are visually de-emphasised — they're typically "save the old chain when it failed to merge" artefacts, not actively worked branches.
- The footer `+ Create branch…` opens a transient dialog (§4.3).

### 4.2 Branches panel (rail → Branches)

A table view of all branches, one row per branch:

| Branch              | Tip       | Resources | Last commit         | Actions                                                 |
|---------------------|-----------|-----------|---------------------|---------------------------------------------------------|
| **main** (active)   | a4f2…3c1  | 142       | 2 min ago           | View history · Compact · Merge into… · *(no delete)*    |
| kinase-screen       | 9b81…7e2  | 38        | 12 min ago          | Switch · View history · Compact · Merge into… · Delete  |
| wip-mtt-rewrite     | 3d4f…9a0  | 27        | yesterday           | Switch · View history · Compact · Merge into… · Delete  |
| auto-2026-05-10     | 5c12…0e8  | 12        | 3 days ago          | Switch · View history · Merge into… · Delete            |

- `main` cannot be deleted (kernel enforces this — `DeleteBranchResponse` has a `protected` rejection reason; surface it as a disabled action with tooltip).
- Inline "Merge into…" shortcut opens the merge dialog (§6) pre-filled with this branch as source.
- "Compact" jumps to the Compaction wizard (§7) scoped to this branch.

### 4.3 Create branch dialog

```
┌─────────────────────────────────────────────┐
│ Create branch                               │
├─────────────────────────────────────────────┤
│ Name        [ ____________________________] │
│ Start from  ● Current head of main          │
│             ○ Current head of kinase-screen │
│             ○ Specific layer id  [ ______ ] │
│                                             │
│ ☐ Switch to this branch after creating      │
├─────────────────────────────────────────────┤
│                          [ Cancel ] [ Create ] │
└─────────────────────────────────────────────┘
```

Calls `CreateBranch(name, layer_id)`. The notebook does not validate the name client-side beyond non-empty — the kernel rejects names matching the `^auto-` reserved prefix (§G.7).

---

## 5. History panel

### 5.1 What "history" means here

`LayerTopology` returns the whole DAG. The History panel is a **branch-scoped linearisation**: walk parent pointers from the selected branch's tip down to the root, rendering one row per layer. Merge layers (`parents.len() > 1`) get visual treatment but stay in the linear walk (following first parent, per D23 §5.1).

This is approximately what `git log` does, with the structural difference that Eigenius's parent walk *is* deterministic from the topology — there's no `--first-parent` flag because there's no other walk.

### 5.2 Layout

```
┌────────────────────────────────────────────────────────────────────┐
│ ● a4f2…3c1   load: kinase-data v3       2 min ago    +5 resources  │ ← tip
│ │
│ ◆ 8b21…0f4   merge: ↕ main ↕ wip-types  5 min ago    +0 resources  │ ← merge
│ │╲
│ │ ◐ <consolidated into 6cea…>           ← redirect source tombstone
│ │
│ ● 6cea…91d   consolidate: 17→1          1 hr ago    +0 resources   │ ← consolidation result
│ │
│ ● 9b81…7e2   program-run: simulate-rxn  3 hr ago    +12 resources  │
│ │
│ ● a112…cc9   load: kinase-data v2       yesterday   +8 resources   │
│ │
│ ⬚ <root>                                                            │
└────────────────────────────────────────────────────────────────────┘
```

Per-row data (all from `LayerHandle`):

- **Icon** — `●` ordinary layer, `◆` multi-parent merge, `◐` redirect source tombstone (D33 §5.1), `⬚` root.
- **Short hash** — first 4 hex chars of layer id with `…` continuation. Click → full id in a tooltip + "Copy".
- **Label** — `LayerHandle.name`. For merge layers, render as `merge: ↕ <branch1> ↕ <branch2>` if the merge layer's parents map to recognisable branch tips.
- **Timestamp** — relative; absolute on hover.
- **Resource delta** — `+N resources` from `LayerHandle.resource_count`.

Click a row → opens a side detail panel:

```
┌────────────────────────────────────────────────────────┐
│ Layer a4f2…3c1                                         │
├────────────────────────────────────────────────────────┤
│ Name       load: kinase-data v3                        │
│ Created    2026-05-10 14:32:17 UTC                     │
│ Resources  5 added                                     │
│ Parent     8b21…0f4                                    │
│ Branches   main                                        │
│ Tags       —                                           │
├────────────────────────────────────────────────────────┤
│ [ Inspect resources… ]                                 │
│ [ Time-travel here ]   (pins notebook reads to this    │
│                         layer; commits still go to tip)│
│ [ Create tag… ]                                        │
└────────────────────────────────────────────────────────┘
```

"Time-travel here" sets the notebook's local read-pin (using existing `at_layer` parameter on `Inspect`/`Query`/`RunProgramByIri`). Writes still go to branch tip — the read-pin is per-session, not a kernel concept.

### 5.3 Long histories

For chains with thousands of layers, the panel renders virtualised (Fluent UI `Table` with row virtualisation). The kernel returns the full topology in one call today (D22 §4.2); cursored fetch becomes necessary above ~10K layers (§G.6).

### 5.4 Consolidation / redirect rendering

When a redirect tombstone (D33 §5.1) appears mid-walk, render it as a muted row showing the target. The user can click into it; the side panel for a tombstone shows the redirect target and lets the user jump to that layer's row.

---

## 6. Merge UX

### 6.1 Implicit trivial merge — surfaced as a commit outcome

Every `Load` / `RunProgram` / `Reflect` / `Query INTO` already runs through `update_branch(AllowTrivial)`. When concurrent activity causes the CAS to race, the kernel attempts a trivial merge. **Today the outcome is invisible to the notebook** (§G.1).

After §G.1's wire-up, each commit response carries a structured outcome:

```proto
enum MergeOutcome {
  MERGE_OUTCOME_UNSPECIFIED = 0;
  MERGE_OUTCOME_FAST_FORWARD = 1;
  MERGE_OUTCOME_TRIVIAL_MERGE = 2;
  MERGE_OUTCOME_NEEDS_WITNESSED_MERGE = 3;
}

message MergeInfo {
  MergeOutcome outcome = 1;
  // Set when outcome = TRIVIAL_MERGE: id of the merge layer.
  string merge_layer_id = 2;
  // Set when outcome = NEEDS_WITNESSED_MERGE: IRIs that both contributing
  // chains defined since their LCA.
  repeated string conflicting_iris = 3;
  // Set when outcome = NEEDS_WITNESSED_MERGE: the branch's current head
  // that the caller raced against.
  string current_head = 4;
}
```

`LoadResponse`, `RunProgramResponse`, `ReflectResponse`, `QueryResponse` gain `MergeInfo merge = N;`.

**Cell-level UX per outcome:**

- `FAST_FORWARD` (the default case) — no UI change.
- `TRIVIAL_MERGE` — toast notification:

  ```
  ┌──────────────────────────────────────────────────────────┐
  │ ⓘ  Merged with concurrent work                           │
  │                                                          │
  │ Another commit reached this branch between when you read │
  │ the head and when you saved. Your changes were merged    │
  │ automatically (no conflicts).                            │
  │                                                          │
  │ Merge layer: 8b21…0f4                          [ View ]  │
  └──────────────────────────────────────────────────────────┘
  ```

  The toast persists for ~8 seconds. Clicking "View" jumps to the merge layer's row in the History panel. The cell's status indicator shows a small ◆ next to its "committed at <hash>" footer.

- `NEEDS_WITNESSED_MERGE` — the cell goes to an error state, the commit is rolled back (the orphaned layer is GC-eligible), and a recovery dialog opens (§6.2).

### 6.2 Witnessed-merge-needed recovery dialog

```
┌─────────────────────────────────────────────────────────────────┐
│ Conflict — branch advanced under your changes                   │
├─────────────────────────────────────────────────────────────────┤
│ Your commit raced against another commit to "main", and the     │
│ two contributions modify the same resources. Eigenius cannot    │
│ merge automatically without a witness (Phase 15).               │
│                                                                 │
│ Conflicting resources (3):                                      │
│   urn:eigenius:demo:Widget                                      │
│   urn:eigenius:demo:Gadget                                      │
│   urn:eigenius:demo:Sprocket                                    │
│                                                                 │
│ Branch's current head:  c9a4…11b                                │
│ Your unmerged work:     8b21…0f4  (will be discarded if you     │
│                                    don't save it)               │
│                                                                 │
│ Recovery options:                                               │
│                                                                 │
│   ○ Save my work as a new sibling branch                        │
│     Name: [ auto-2026-05-10-1432 _________________ ]            │
│                                                                 │
│   ○ Rebase: pin to the new head and re-run the cells that       │
│     produced this layer (re-execute manually).                  │
│                                                                 │
│   ○ Discard my work (the orphaned layer becomes GC-eligible).   │
├─────────────────────────────────────────────────────────────────┤
│                                       [ Cancel ] [ Continue ]   │
└─────────────────────────────────────────────────────────────────┘
```

This is the *only* destructive-action dialog in §6: discarding work is irreversible (modulo GC delay) and the user is being asked to make a real decision. The "Save as sibling" option calls `CreateBranch(name, layer_id)` against the orphaned layer's id.

### 6.3 Explicit `MergeBranches` RPC and dialog (§G.3)

Distinct from implicit trivial-merge: the user is on branch `wip-types`, finished their work, wants to fold it into `main`. This needs a new RPC:

```proto
message MergeBranchesRequest {
  string source = 1;       // branch to fold in
  string target = 2;       // branch to fold into
}

message MergeBranchesResponse {
  MergeInfo merge = 1;
  // If the merge succeeded (FastForward or TrivialMerge), the target
  // branch's new tip; otherwise the target's unchanged tip.
  string target_tip = 2;
}
```

Server-side: load `source`'s tip and `target`'s tip, call `update_branch(target, target_tip_old, source_tip, AllowTrivial)`. The outcome flows back through `MergeInfo`.

UX (rail → Merge):

```
┌─────────────────────────────────────────────────────────────┐
│ Merge branches                                              │
├─────────────────────────────────────────────────────────────┤
│ Source     [ wip-types        ▾ ]                           │
│ Target     [ main             ▾ ]                           │
│                                                             │
│ Preview                                                     │
│   12 layers ahead, 4 behind                                 │
│   Estimated outcome: trivial merge                          │
│   IRI overlap (will conflict): none detected                │
│                                                             │
│ [ Refresh preview ]                                         │
├─────────────────────────────────────────────────────────────┤
│                                  [ Cancel ] [ Merge ]       │
└─────────────────────────────────────────────────────────────┘
```

The preview runs LCA + IRI-disjointness server-side without actually CAS-ing. This needs **another** kernel addition: `PreviewMerge(source, target) → MergeInfo` (the same outcome enum, computed without side-effect). Folded into §G.3.

Cost of preview: same as a real trivial-merge attempt minus the writes — bounded by the LCA walk + IRI-set computation, which D23 §5.4.3 already estimates as cheap.

### 6.4 What about `auto-…` branches?

After a `NEEDS_WITNESSED_MERGE`, the user has chosen "save as sibling" with an auto-generated name. The Branches panel renders these de-emphasised. We do not auto-clean them — they're real work that someone might still want. Manual delete via the Branches panel is the recovery path.

---

## 7. Compaction wizard

`ConsolidateChain` (D25) plus `EstimateConsolidation` already exist. The UX is a 3-step wizard.

### 7.1 Step 1 — pick range

```
┌─────────────────────────────────────────────────────────────┐
│ Compaction · Step 1 of 3 — Select range                     │
├─────────────────────────────────────────────────────────────┤
│ Branch  main                                                │
│                                                             │
│ From layer    [ a112…cc9   load: kinase-data v2     ▾ ]     │
│ To layer      [ 6cea…91d   consolidate: 17→1        ▾ ]     │
│                                                             │
│ Selected range: 17 layers                                   │
│                                                             │
│ Mode                                                        │
│   ● Replace history    (faster; older layers become         │
│                         redirect sources to the result)     │
│   ○ Preserve history   (slower; old chain stays accessible) │
├─────────────────────────────────────────────────────────────┤
│                              [ Cancel ] [ Estimate → ]      │
└─────────────────────────────────────────────────────────────┘
```

Branch is pinned to whatever's active in the header. From/To dropdowns are populated from the branch history (§5).

### 7.2 Step 2 — estimate

Calls `EstimateConsolidation` and renders:

```
┌─────────────────────────────────────────────────────────────┐
│ Compaction · Step 2 of 3 — Preview                          │
├─────────────────────────────────────────────────────────────┤
│ Range        a112…cc9  →  6cea…91d  (17 layers)             │
│ Mode         Replace history                                │
│                                                             │
│ Estimated outcome                                           │
│   Layers collapsed:        17 → 1                           │
│   Resources kept:          184 (deduplicated from 312)      │
│   Bytes reclaimable:       ~12.4 MB                         │
│   Walk cost:               low (≤ 200 layer scans)          │
│                                                             │
│ Side effects                                                │
│   • 16 redirect tombstones will be created.                 │
│   • Tasks pinned to in-range layers: 0 (safe).              │
│   • Tags pointing into the range: 1                         │
│     ◦ "release-v2" → a112…cc9 will be re-targeted to        │
│       the consolidation result.                             │
├─────────────────────────────────────────────────────────────┤
│                       [ Back ] [ Cancel ] [ Run → ]         │
└─────────────────────────────────────────────────────────────┘
```

The "Tasks pinned" count is a critical safety field — D25 should refuse a consolidation that orphans active task pins; this UX surfaces it before the user runs.

### 7.3 Step 3 — execute

Long-running. Show a progress card; route the result back into the History panel which will now show the new consolidation row + redirect tombstones.

### 7.4 Granularity question

Should the wizard support "consolidate the older half of this branch" as a one-click action? — proposed: yes, as a button on the Branches panel that pre-fills From=root, To=midpoint and goes straight to Step 2. Defer until first feedback round.

---

## 8. Tags (§G.2)

### 8.1 Semantics (proposed for kernel design)

A tag is an **immutable named ref** to a `LayerId`. Once created, a tag cannot be retargeted (this is what differentiates it from a branch). It can be deleted. Tags do not pin against GC by themselves — GC roots are branches and active task pins; this is a separate question we should decide before implementing (§Open Q O.1).

Storage: a new column family `tags` in the persistent backend mapping `tag_name → LayerId`. Trait:

```rust
trait PersistentBackend {
    // ... existing methods ...
    fn put_tag(&self, name: &str, layer_id: &LayerId) -> Result<(), StorageError>;
    fn get_tag(&self, name: &str) -> Result<Option<LayerId>, StorageError>;
    fn list_tags(&self) -> Result<Vec<(String, LayerId)>, StorageError>;
    fn delete_tag(&self, name: &str) -> Result<(), StorageError>;
}
```

RPCs:

```proto
rpc CreateTag(CreateTagRequest) returns (CreateTagResponse);
rpc ListTags(ListTagsRequest) returns (ListTagsResponse);
rpc DeleteTag(DeleteTagRequest) returns (DeleteTagResponse);
```

`CreateTag` rejects re-using an existing name with `AlreadyExists`. There is intentionally no `UpdateTag`.

### 8.2 UX

The Tags panel mirrors Branches but is simpler — no "switch to" action (tags aren't writable):

| Tag         | Layer       | Reachable from | Actions                  |
|-------------|-------------|----------------|--------------------------|
| release-v2  | a112…cc9    | main           | View in history · Delete |
| baseline    | 9b81…7e2    | kinase-screen  | View in history · Delete |

Tag creation surfaces in two places: (a) the Tags panel's `+ Create tag…` button (with a layer-picker dropdown), and (b) the layer detail side panel from §5.2 (one-click "Create tag…" pre-filled with the layer's id).

### 8.3 Tag-pinning vs GC interaction

**Tags are GC roots.** A tag protects its target layer (and that layer's transitive ancestors) from garbage collection for as long as the tag exists. Deleting a tag releases the protection; whether the target then becomes GC-eligible depends on whether any other root (branch ref, active task pin, another tag) still reaches it.

This matches the intuitive contract — "tag this state so I can come back to it later" is undermined if GC can silently break the tag. Implementation cost is one extra root-set entry per tag during `gc::collect`'s root walk, which is small.

UI consequence: a broken-tag state is therefore unreachable by construction. §13 still covers the case for defensive rendering, but in practice it can only happen via direct backend manipulation outside the kernel.

---

## 9. Other rail destinations

### 9.1 Tasks

Surfaces `ListTasks` / `GetTaskStatus` / `CancelTask`. Tabular view:

| Task ID        | Program                  | Status     | Started    | Result layer | Actions          |
|----------------|--------------------------|------------|------------|--------------|------------------|
| 3f2a…b14       | simulate-kinase-rxn       | Completed  | 2h ago     | 9b81…7e2     | View trace       |
| 7c19…0e8       | optimize-yield            | Running    | 12s ago    | —            | View · Cancel    |
| 1d44…2a3       | check-equilibrium         | Failed     | 1h ago     | —            | View trace       |

"View trace" navigates to the trace layer's detail in History. Failed tasks surface their error in the row's detail expansion (no separate dialog).

### 9.2 Institutions inspector

#### 9.2.1 Why this lives in the chain workspace, not as a top-level panel

The set of installed institutions is **branch-scoped**: D14 §9.3 chain-reinsertion registers institutions through ordinary layer commits, and D31 §5 external-institution lifecycle ties registration to the chain. Two branches can have entirely different institution sets — a `wip-catalyst` branch may have `Catalyst.jl` installed that `main` doesn't have yet; an `auto-…` sibling branch may have a stale set after a failed merge.

The chain workspace is the right home: the active-branch header (§3.2) already determines what the user sees in History and Tags, and the inspector reads against the same active-branch tip (or whatever the user pinned via "Time-travel here" in §5.2). A top-level "Institutions" panel would have to either show the global superset (misleading — most of those aren't reachable from your current head) or duplicate the branch-picker logic.

Time-travel composes naturally: pinning the read to a historical layer shows the institution set as it was at that point in the chain's evolution. This is the same `at_layer` parameter `ListInstitutions` and `Inspect` already accept.

#### 9.2.2 List view

```
┌────────────────────────────────────────────────────────────────────┐
│ Institutions installed at main · tip a4f2…3c1                      │
├────────────────────────────────────────────────────────────────────┤
│ Name                       Runtime      Comorphisms  QueryClasses  │
│ ──────────────────────────────────────────────────────────────     │
│ Catalyst.jl                external      3            5            │
│ DifferentialEquations.jl   external      2            4            │
│ JuMP.jl                    external      1            3            │
│ IntervalArithmetic.jl      wasm          0            2            │
│ MTT-Lean4                  wasm          1            7            │
│ core/validation            in-process    0            12           │
└────────────────────────────────────────────────────────────────────┘
```

Click a row → detail panel (§9.2.3).

The list comes from `ListInstitutions(at_layer)`. With the §G.8 enrichment, the response carries enough metadata to populate the columns directly without a per-row `Inspect`.

#### 9.2.3 Detail panel

```
┌──────────────────────────────────────────────────────────────────────┐
│ Catalyst.jl                                                          │
│ urn:eigenius:institutions:catalyst                                   │
├──────────────────────────────────────────────────────────────────────┤
│ Runtime        external (julia-substrate-d29)                        │
│ Installed at   layer 9b81…7e2  (Load: catalyst-institution-v1.json)  │
│ Branch refs    main, wip-catalyst                                    │
│                                                                      │
│ Comorphisms (3)                                                      │
│   ReactionNetwork → ODESystem                                        │
│     program: urn:eigenius:catalyst:reify-to-ode                      │
│   ReactionNetwork → SBML                                             │
│     program: urn:eigenius:catalyst:export-sbml                       │
│   Species → InitialState                                             │
│     program: urn:eigenius:catalyst:default-initial                   │
│                                                                      │
│ Query classes (5)                                                    │
│   ◇ ReactionNetwork              kind: OnDemand                      │
│     bound to urn:eigenius:catalyst:ReactionNetwork                   │
│   ◇ ValidateReactionNetwork      kind: AutoOnLoad                    │
│     bound to urn:eigenius:catalyst:ReactionNetwork                   │
│     fires on every load of a ReactionNetwork resource                │
│   ◇ SteadyStateQuery             kind: Decidable                     │
│     bound to urn:eigenius:catalyst:ODESystem                         │
│   ...                                                                │
├──────────────────────────────────────────────────────────────────────┤
│ [ Inspect raw resource ]   [ View install layer in history ]         │
└──────────────────────────────────────────────────────────────────────┘
```

The "View install layer" link jumps to the History panel scrolled to that layer's row — cross-linking the chain views.

The "Inspect raw resource" action opens the full Eigon-JSON of the institution resource (existing `Inspect` RPC + JSON viewer) for users who want the unfiltered metadata.

#### 9.2.4 What the detail panel is built from

With §G.8's enrichment (richer `InstitutionInfo`), most of the detail panel renders from a single `ListInstitutions` call. The Comorphism source / target classes and QueryClass binding classes are themselves chain resources — clicking a class name in the detail panel calls `GetSchema(class_iri)` and shows its JSON Schema in a side overlay, or jumps to the class's row in a future Resource inspector. The Resource inspector itself is out of scope for d34 (it'd be its own rail destination later); for now the JSON Schema overlay is sufficient.

The "Installed at" line requires walking the chain for the layer that first defined the institution IRI — this is naturally produced by the §G.6 history endpoint (which already aggregates per-layer `defined_iris` for log rendering). Without §G.6 in place, the inspector falls back to displaying just "current head" without the lineage.

### 9.3 Topology view

The existing `TopologyGraphView` becomes a rail destination instead of being only embedded in cell outputs. Branch-aware additions: highlight the current branch's chain in colour; render other branches in grey; show merge layers with both incoming edges visually emphasised; allow filtering by branch.

The current view returns the *whole* topology; once we exceed ~5K layers, paint cost dominates. Cursored topology + viewport-based loading goes alongside §G.6.

### 9.4 GC (§G.4)

Rail item, last position (admin / destructive). Two RPCs needed:

```proto
rpc EstimateGc(EstimateGcRequest) returns (EstimateGcResponse);
rpc RunGc(RunGcRequest) returns (RunGcResponse);

message EstimateGcResponse {
  uint64 eligible_layers = 1;
  uint64 reclaimable_bytes = 2;
  // Active task pins / branch refs / tag refs that protect layers
  // from collection — listed so the operator sees what's keeping
  // the chain alive.
  uint64 task_pins = 3;
  uint64 branch_pins = 4;
  uint64 tag_pins = 5;
}
```

UX is two screens — Estimate → Confirm + Run — same shape as compaction but simpler (no range selector). Result goes back to a stats panel.

**Auth gating.** `RunGc` is not admin-gated in the MVP. The D22 trust model (single-user, typically localhost) makes a separate auth gate redundant; the confirmation dialog on Step 2 is the only friction. Production deployment may revisit this when D22's auth model is hardened, but that's out of scope here.

---

## 10. Kernel gap list (the forcing function)

| Tag    | Gap                                                                 | Estimate    | Blocks                  |
|--------|---------------------------------------------------------------------|-------------|-------------------------|
| §G.1   | Surface `UpdateOutcome` from `advance_branch_for_layer`. Fix the silent `NeedsWitnessedMerge` bug. Add `MergeInfo` to `LoadResponse` / `RunProgramResponse` / `ReflectResponse` / `QueryResponse`. | 1–2 days   | All merge UX (§6)       |
| §G.2   | Tags: storage trait, column family, three RPCs (`CreateTag`, `ListTags`, `DeleteTag`). Decide GC semantics (§O.1).                                          | 2–3 days   | Tags panel (§8)         |
| §G.3   | `MergeBranches` + `PreviewMerge` RPCs. Wrap existing `update_branch(AllowTrivial)` + LCA-only walk.                                                          | 1–2 days   | Explicit merge (§6.3)   |
| §G.4   | `EstimateGc` + `RunGc` RPCs over `crate::gc::collect`. Surface the protection accounting (task / branch / tag pins).                                         | 2–3 days   | GC panel (§9.3)         |
| §G.5   | SDK + notebook consume `branch_advanced`. Cell footer indicator + read-pin behaviour.                                                                        | 1 day      | Cell cache visibility   |
| §G.6   | Branch-scoped history endpoint (cursored). Merge-layer + redirect-tombstone metadata aggregated for log rendering.                                           | 2–3 days   | History panel (§5)      |
| §G.7   | Reject `^auto-` prefix in `CreateBranch` (it's reserved by D23 §5.4.4 for sibling-branch auto-naming). One-line server change.                                | < 1 hour   | Create-branch dialog    |
| §G.8   | Enrich `InstitutionInfo` with `runtime_kind` (in-process / wasm / external), declared comorphisms (source/target class IRIs + program ref), per-QueryClass kind (Decidable / OnDemand / AutoOnLoad) and bound class IRI. The kernel already has all of this — `InstitutionIndex` carries it; the proto message just exposes a subset. | 1–2 days   | Institutions inspector (§9.2) |

Cross-cutting orchestrator work (new Connect routes wrapping each new kernel RPC + new wrapper for `MergeInfo` everywhere): bundled in with each gap as it lands.

**Total kernel work**: roughly 2½ weeks for one engineer, parallelisable. None of it requires new design decisions beyond §O.1.

---

## 11. SDK additions

Extending `@eigenius/client` (D22 §5):

```ts
// New surface on the Eigen class
class Eigen {
  // ... existing ...

  // Branches
  listBranches(): Promise<Branch[]>;
  getBranch(name: string): Promise<Branch>;
  createBranch(name: string, fromLayer?: string): Promise<Branch>;
  deleteBranch(name: string): Promise<void>;

  // Tags (§G.2)
  listTags(): Promise<Tag[]>;
  createTag(name: string, layerId: string): Promise<Tag>;
  deleteTag(name: string): Promise<void>;

  // Merge (§G.3)
  mergeBranches(source: string, target: string): Promise<MergeResult>;
  previewMerge(source: string, target: string): Promise<MergeInfo>;

  // Compaction
  estimateConsolidation(args: ConsolidateArgs): Promise<ConsolidationEstimate>;
  consolidateChain(args: ConsolidateArgs): Promise<ConsolidationResult>;

  // GC (§G.4)
  estimateGc(): Promise<GcEstimate>;
  runGc(): Promise<GcResult>;

  // Tasks
  listTasks(): Promise<Task[]>;
  getTaskStatus(taskId: string): Promise<Task>;
  cancelTask(taskId: string): Promise<void>;

  // Institutions (§G.8 enriches the response shape; the call signature
  // stays the same — `atLayer` already exists on the kernel side).
  listInstitutions(atLayer?: string): Promise<Institution[]>;
  getSchema(classIri: string, atLayer?: string): Promise<JsonSchema>;
}

interface Institution {
  iri: string;
  name: string;
  runtimeKind: "in-process" | "wasm" | "external";
  comorphisms: Comorphism[];
  queryClasses: QueryClass[];
  installedAtLayerId?: string;  // §G.6-dependent; undefined if not yet computed
}

interface Comorphism {
  sourceClassIri: string;
  targetClassIri: string;
  programIri: string;
}

interface QueryClass {
  iri: string;
  kind: "decidable" | "on-demand" | "auto-on-load";
  boundClassIri: string;
  // For auto-on-load: human-readable trigger description
  // (e.g., "fires on every load of a ReactionNetwork resource").
  trigger?: string;
}

// Every commit-returning RPC now surfaces this:
interface MergeInfo {
  outcome: "fast-forward" | "trivial-merge" | "needs-witnessed-merge";
  mergeLayerId?: string;
  conflictingIris?: string[];
  currentHead?: string;
}
```

All RPCs are thin wrappers — each maps 1:1 to a Connect route on the orchestrator (D22 §3.2). The notebook consumes them through React hooks (`useBranches`, `useHistory`, `useMergeOutcome`, etc.) that subscribe to the Zustand `chainStore` introduced in §12.

---

## 12. Notebook state — the `chainStore`

A new Zustand store alongside the existing `notebookStore` (D22 §6.4):

```ts
interface ChainStore {
  activeBranch: string;          // header picker selection
  knownBranches: Branch[];       // cached from listBranches
  knownTags: Tag[];              // cached from listTags
  readPin?: string;              // local; not kernel-side
  recentMerges: MergeEvent[];    // last N for toast deduplication

  switchBranch(name: string): Promise<void>;
  refreshBranches(): Promise<void>;
  applyCommitOutcome(merge: MergeInfo): void;  // called on every Load/Run/etc response
}
```

The store is the single integration point between the kernel surface and the rail panels. Cells subscribe to `activeBranch` (so re-running a cell goes to the current branch) and to the most-recent commit's `MergeInfo` (for the cell footer indicator and toast firing).

---

## 13. Edge cases and error UX

- **Branch deleted while notebook is pinned to it.** The next commit returns `Status::NotFound`. The notebook surfaces an error toast and routes the user to the Branches panel to pick a new branch.
- **Tag pointing at a layer that GC reclaimed.** Shouldn't be possible if §O.1 resolves tags as GC roots. If we decide tags don't protect, the Tags panel renders broken tags with a clear "target not found — delete" affordance.
- **Layer-id collisions in history rendering.** Short hashes can collide at the 4-char level once you have a few thousand layers. Auto-extend the displayed prefix until unique within the current view.
- **Concurrent topology refresh during a long compaction.** The history panel polls `LayerTopology` periodically; during a compaction the topology genuinely changes underneath. Solution: server-Sent Events or a poll-on-write pattern; defer for v1.
- **Connection loss.** All chain operations are idempotent except `RunGc` and `ConsolidateChain` (which have committed side effects). Both have outcomes the client can re-query; reconnection-after-loss just re-fetches state rather than retrying.

---

## 14. Test plan

### 14.1 Kernel

- §G.1: unit tests in `lattice.rs` already cover `update_branch` outcomes — extend to assert each outcome propagates through `advance_branch_for_layer` and into the proto `MergeInfo`. End-to-end test: two concurrent `Load` calls against the same branch, identical content → both get `FAST_FORWARD` or one gets `TRIVIAL_MERGE`. Disjoint content + concurrent → one `FAST_FORWARD`, one `TRIVIAL_MERGE` with merge_layer surfaced. Conflicting content + concurrent → one `FAST_FORWARD`, one `NEEDS_WITNESSED_MERGE` with conflicting_iris populated, **and** the loser's branch_advanced reports `false`.
- §G.2: storage round-trips for tag CF; rejection of `CreateTag` for existing names; `DeleteBranch` does not affect tags that point into the deleted branch's reachable layers (they keep those layers alive if §O.1 decides tags are GC roots).
- §G.3: `PreviewMerge` returns the same `MergeInfo` as a subsequent `MergeBranches` would (proven by running both and comparing).
- §G.4: `EstimateGc` accuracy bounds (the actual `RunGc` reclaims ≥ the estimate ± a documented slack).

### 14.2 Notebook

- Playwright e2e: create branch → load resources → switch to main → merge dialog → verify outcome.
- Storybook stories for each rail panel with mock chain state, including the three merge outcomes and the witnessed-merge recovery dialog.
- Visual regression on the history-panel rendering for layer counts 1, 10, 100, 1000 (last via mock — real perf test waits for §G.6's cursored fetch).

---

## 15. Open questions

### O.1 Are tags GC roots? — *Resolved: yes*

See §8.3 for the resolved design. Tags participate in the GC root set; deleting a tag releases the protection.

### O.2 Does "Time-travel here" need its own pin storage?

Currently it's a session-local React state in the notebook (no kernel involvement). If the notebook saves and reloads, the pin is lost. Two options: keep it local (fine for MVP), or persist it in the `.eigon-notebook.json` (so reopening a notebook restores the pinned view). Lean toward persisting; trivial change.

### O.3 What's the right Nav granularity? — *Resolved: Chain / Workspace / Admin grouping*

Adopted:

- Notebook (top-level)
- **Chain** — Branches · History · Tags · Merge
- **Workspace** — Topology · Institutions · Tasks
- **Admin** — Compaction · GC
- Health (footer)

Institutions sit under Workspace because §9.2's discussion makes them branch-scoped — they belong to the same active-branch frame as Topology and Tasks. Reflected in §3.1's rail mockup. The grouping is a starting point — open to refinement after first user feedback (notably whether Tasks belongs under Workspace or Admin, since they're a hybrid of "what's running for me" and "what's pinning layers against GC").

### O.4 Should the merge toast (§6.1) replay on a notebook reload?

Concrete scenario: I `Load` data, the kernel trivially merges with concurrent work, the toast fires, I close the notebook before clicking "View". On reopen, is the merge fact surfaced somewhere? Probably yes, on the cell that triggered it (cell footer indicator `◆`), but not as a new toast.

### O.5 Cursored vs whole-DAG `LayerTopology`?

Threshold question, not architecture. Today we always return everything (D22 §4.2). At what layer count does the response cross 1MB / 100ms latency / 50MB JS heap? Worth measuring before §G.6 commits to a cursor design.

---

## 16. Roll-out order

The phases below are ordered by what unblocks the next; each phase is shippable on its own.

1. **§G.1 + §G.5** — `MergeInfo` wire-up + `branch_advanced` SDK consumer. Lights up the most-common path (every cell now shows merge / cache state) with the smallest kernel change. **~3 days.**
2. **Header + branch picker** — purely client-side. Unblocks every other rail panel since the active-branch concept exists. **~2 days.**
3. **Branches panel** + create / delete dialogs. Calls existing branch RPCs. **~3 days.**
4. **History panel** with linear walk over existing `LayerTopology`. Defer cursored fetch and §G.6 until needed. **~3 days.**
5. **§G.3 + Merge UX** — explicit and witnessed-merge-recovery dialogs. **~4 days.**
6. **Compaction wizard** — uses existing `ConsolidateChain` / `EstimateConsolidation`. **~3 days.**
7. **Tasks panel** — uses existing RPCs. **~2 days.**
8. **§G.2 + Tags panel.** **~3 days.**
9. **§G.4 + GC panel.** **~3 days.**
10. **§G.8 + Institutions inspector.** List + detail panels. The "Installed at" lineage degrades gracefully if §G.6 hasn't landed yet. **~3 days.**
11. **Topology view as rail destination** with branch filter. Polish. **~2 days.**

Total: ~31 days frontend + ~12 days kernel/server work, parallelisable. The first two phases together would already make the chain visible and the cache feature complete on the consumer side.

---

## 17. Outstanding design decisions before implementation starts

Resolved 2026-05-11:

1. **§O.1** — tags as GC roots. *Yes* — folded into §8.3.
2. **§O.3** — Nav grouping. *Chain / Workspace / Admin* — folded into §3.1 and §15.
3. **§G.4** — `RunGc` auth gating. *No gate in MVP* — folded into §9.4.

The remaining open questions (§O.2 read-pin persistence, §O.4 toast replay on reload, §O.5 cursored topology threshold) are non-blocking and can be resolved during implementation.
