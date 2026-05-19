# 15. Tags, branches, and history

Eigenius layers are immutable. Every commit produces a new layer with a content-addressed id; named refs (branches, tags) are just pointers into the layer graph. The notebook's workspace rail surfaces three panels — **Branches**, **Tags**, **History** — that let you create, switch, navigate, and time-travel without touching the CLI.

This chapter is the operational reference for those panels. The CLI surface is in [chapter 4 §4.6](04-cli-reference.md#46-branch-commands-require---endpoint); the programmatic surface is in [chapter 17 §17.4](17-typescript-sdk.md#174-branches). The kernel-level semantics are in [D23 — Out-of-core layer architecture](../../design/d23-out-of-core-layer-architecture.md) §5.4–§5.5 and [D34 — Notebook chain workspace](../../design/d34-notebook-chain-workspace.md).

## 15.1. What's in the workspace rail

The vertical rail on the right of the notebook (below the cells) hosts seven destinations. Three of them deal with the chain:

| Destination | What it shows | Source |
|---|---|---|
| **Branches** | Every named branch — current head, switch / merge / view-history actions, plus a `+ Create branch…` button. | [`notebooks/src/components/workspace/BranchesPanel.tsx`](../../../notebooks/src/components/workspace/BranchesPanel.tsx) |
| **Tags** | Every committed tag — target layer, created-at timestamp, plus per-row actions (view in history, fork branch, delete). | [`notebooks/src/components/workspace/TagsPanel.tsx`](../../../notebooks/src/components/workspace/TagsPanel.tsx) |
| **History** | The active branch's chain walked from tip to root, with a detail pane on the right for the selected layer. | [`notebooks/src/components/workspace/HistoryPanel.tsx`](../../../notebooks/src/components/workspace/HistoryPanel.tsx) |

The fourth chain-aware control is the **branch selector** at the top of every notebook view — it shows the active branch + an optional `· reading at <layer>` indicator when a read-pin is set (see §15.4).

Source: [`notebooks/src/components/`](../../../notebooks/src/components/).

## 15.2. Branches

A branch is a mutable named pointer to a layer id. Writes (cell commits, `eigen.load`, merges) advance the branch's head atomically with compare-and-set semantics — concurrent writes that race against the same tip get rejected with `STRICT_FAST_FORWARD_VIOLATION`, surfaced in the notebook as the cell's "branch moved, re-run to retry" error.

### 15.2.1. The Branches panel

Open it from the workspace rail's **Branches** button. The table lists each branch by name with three columns: **Branch**, **Tip** (12-char prefix of the head layer id), **Actions**.

Per-row actions:

| Action | What it does |
|---|---|
| **Switch** | Set this branch as the active branch for the session. The cells' reads now go through this branch's chain; writes go to its tip. The active branch shows the **Active** badge in place of Switch. |
| **View history** | Switch (if needed) and open the History panel against this branch. |
| **Merge into…** | Pre-fills this branch as the source in the Merge panel and navigates there. The user picks the target. See [chapter 16](16-merge-resolution.md). |
| **Delete** | Open a confirmation dialog. `main` is protected — the button is disabled with a tooltip explaining the kernel rejects `DeleteBranch` for the default branch. |

The header has a single primary action: **Create branch…** (with a `+` icon).

### 15.2.2. Creating a branch

The **Create branch…** dialog (also reachable from the BranchBar's dropdown menu, from a Tags-panel row's `Create branch…` action, and from the History panel's detail-pane `Create branch…` button) takes:

- **Name** — required. The kernel enforces shape (`[A-Za-z0-9._/-]+`).
- **Start from** — a four-way picker:
  - **Existing branch** (default — the active branch is pre-selected). Forks at the picked branch's current tip.
  - **Tag** — lists every committed tag. The selected tag's target layer is the fork point. Pre-selected when the dialog was opened from a Tags-panel row.
  - **Specific layer id** — paste a 64-char hex layer id. Pre-filled when the dialog was opened from a History-panel detail pane's `Create branch…` button.
- **Switch to it after creating** — toggle. When on, the new branch becomes the active branch immediately; reads + writes route through it.

The dialog calls `eigen.createBranch(name, startFrom)` and refreshes the panel on success.

### 15.2.3. Branch shape on the wire

The Branches panel lists what `eigen.listBranches()` returned — name + head layer id. The panel intentionally doesn't show resource counts or commit timestamps; those are out of `GetBranch`'s current shape. When `GetBranch` grows them, the table columns will too.

The CLI equivalent is `eigenius db branch list` / `create` / `delete` — see [chapter 4 §4.6](04-cli-reference.md#46-branch-commands-require---endpoint).

## 15.3. Tags

A tag is an *immutable* named pointer to a specific layer. Tags pin their target layer against garbage collection, which makes them the natural way to mark stable references — release snapshots, demo baselines, regression-test pinning, audit waypoints.

### 15.3.1. The Tags panel

Open it from the workspace rail's **Tags** button. The table lists each tag by name with four columns: **Tag**, **Layer** (12-char prefix), **Tagged at**, **Actions**.

Per-row actions:

| Action | What it does |
|---|---|
| **View in history** | Switch the History panel to the tag's branch context and scroll to the tag's target layer with the detail pane open. |
| **Create branch…** | Open the Create-branch dialog with the start-from picker pre-selected to **Tag** = this tag. |
| **Delete** | Confirmation dialog. Deleting a tag releases the GC protection for the target layer (it may still be pinned by being on a branch's chain). The kernel's delete is idempotent — already-gone is not an error. |

Header: **Create tag…** primary button.

### 15.3.2. Creating a tag

The **Create tag…** dialog opens from:

- The Tags panel's header.
- The History panel's detail pane (with the selected layer pre-filled as the target).

Fields:

- **Name** — required. Shape constraints match branch names.
- **Target layer** — two-way picker:
  - **Specific layer id** — paste a hex layer id. Pre-filled when opened from the History panel.
  - **Tip of branch** — Combobox of every branch. The dialog reads the current tip via `getBranch(...)` at submit time so a race between dialog open and submit produces the latest tip, not a stale one.

Tags are immutable once committed. Re-tagging requires deleting and recreating.

The CLI surface is `eigenius db tag create / list / delete` — same semantics.

## 15.4. History

The History panel is the chain navigator. It walks the active branch's tip backward through `parent_layer` edges, deterministic at merges (follows the first parent — see [D23 §5.1](../../design/d23-out-of-core-layer-architecture.md#51-parent-discipline)). Layers older than the LCA of all live branches are still rendered; the panel doesn't truncate.

### 15.4.1. Opening it

Three entry points:

1. The workspace rail's **History** button — opens against the active branch.
2. A Branches-panel row's **View history** — switches to that branch first, then opens History.
3. A Tags-panel row's **View in history** — switches to the tag's containing branch and scrolls to the tag's target.

### 15.4.2. Reading the timeline

Each row carries:

- **Layer id** — 12-char prefix. Click the row to select it and reveal the detail pane.
- **Name** — the layer's commit-time label (`merge:<head_a>+<head_b>` for merges, the ESL cell hash for cell-driven commits, `tag:<name>` rarely, etc.).
- **Created** — absolute ISO timestamp.
- **Resources** — how many resources this layer contributes (not the inherited total).
- **Parent count** — `0` for the root, `1` for normal commits, `2+` for merge layers.

A `📌 pinned` badge appears on the row that's currently the session read-pin (see §15.4.4).

### 15.4.3. The detail pane

Selecting a row reveals a right-hand detail pane with the layer's full hex id (with a copy button), commit-time metadata, and four buttons:

| Button | What it does |
|---|---|
| **Time-travel here** | Set this layer as the session **read-pin**. All subsequent kernel reads (cell `eigen.inspect`/`query`, panel refreshes) route through `atLayer = <this layer>`. Writes still go to the branch tip. When the pin is already on this row, the button shows **Currently pinned here** and is disabled. |
| **Create tag…** | Open the Create-tag dialog with **Target layer** pre-filled to this layer's hex id. |
| **Create branch…** | Open the Create-branch dialog with **Start from = Specific layer id** pre-filled. |
| **Inspect resources…** | Set the read-pin to this layer and navigate to the **Layer** destination — every resource defined in *this* commit (not the inherited chain), with full Eigon-JSON bodies. |

### 15.4.4. Read-pinning (time-travel)

The read-pin is a session-local override that routes reads through a specific layer instead of the branch tip. When set, the BranchBar at the top of the notebook shows `· reading at <12-char>` with a **Return to tip** button.

Writes still go to the branch tip, *not* the pinned layer. That asymmetry is intentional: time-travel lets you look at history without rewriting it. If you want to write somewhere else, fork a branch off the pinned layer first (the detail pane's `Create branch…` button is the one-click path).

Clearing the pin (Return to tip, or selecting a Branches-panel row and Switch to it) re-routes reads to the branch's current head.

The pin lives in the notebook store only — it's not a kernel concept; the kernel just sees `atLayer = <hash>` on each request. The CLI surface is `--at-layer <hash>` on per-command basis.

### 15.4.5. Walking through a merge

When the timeline reaches a merge layer (parent_count ≥ 2), the walk picks the first parent and continues from there. The other parent isn't lost — it's reachable by switching to whichever branch contributed it, then opening History. The first-parent rule matches the kernel's resolve walk (see [D23 §5.1](../../design/d23-out-of-core-layer-architecture.md#51-parent-discipline)) so the History panel's view of "what's reachable from here" matches what `eigen.inspect(iri)` would actually return.

For a merge's *provenance* — which strategy resolved each conflict, which witness was applied, which branch contributed each conflicting body — see [chapter 16 §16.5](16-merge-resolution.md#165-merge-provenance-records).

## 15.5. The branch selector at the top

The BranchBar lives at the top of every notebook view. Three controls:

- **Branch name** with a chevron — opens a dropdown menu listing every branch. Each entry has a Switch button (or shows **Active**); `+ Create branch…` sits at the bottom.
- **Forward / Back** arrows (added in Phase 5) — navigate between previously-visited destinations within the workspace shell.
- **Read-pin indicator** — appears as `· reading at <12-char>` with a **Return to tip** button whenever a History-panel time-travel is active.

The branch selector and the Branches panel share the same Combobox-of-branches mechanism; they're alternative entry points for the same Switch action.

## 15.6. Mental model — what's mutable, what isn't

| Thing | Mutable? | Identity |
|---|---|---|
| **Layer** | Immutable | Content-addressed `LayerId` (SHA-256 of canonical bytes) |
| **Branch** | Mutable | Name → head `LayerId`. Advances on each commit. |
| **Tag** | Immutable | Name → `LayerId`. Created once, deleted explicitly. |
| **Read-pin** | Mutable | Session-local; lives in the notebook store, not the kernel |

The implications:

- **Two branches pointing at the same layer are indistinguishable for reads.** Cells run identically regardless of which branch is active, as long as the head layers match.
- **Tags survive branch deletes.** If you tag `release-v1` at layer `abc…` and later delete the branch that originally committed it, the tag still pins the layer (and protects it from GC).
- **History is per-branch only at presentation time.** The kernel's layer graph is a single DAG; the History panel renders the linearisation from a chosen tip. Switching branches re-renders the same DAG from a different starting node.
- **Merging is the only way to advance one branch with another's work.** Cherry-picking, rebasing, and force-pushing aren't part of the v1 surface — they're explicitly out of scope per [D20 §11](../../design/d20-layer-reconciliation.md).

## 15.7. Common workflows

### "I want to make a stable named ref to this layer."

History panel → select the layer → **Create tag…** → name it `release-v1` or similar. The tag pins the layer against GC.

### "I want to base new work on a past commit without losing the current branch's tip."

History panel → select the layer → **Create branch…** → name it. The new branch's tip is the picked layer; the current branch is unaffected. Switch to the new branch to start committing there.

### "I want to read the chain as it looked yesterday without breaking my live work."

History panel → find the layer → **Time-travel here**. Reads route through the pinned layer; writes still go to the current branch tip. **Return to tip** clears the pin.

### "I want to fold experimental work back into main."

Branches panel → on the experimental branch's row, click **Merge into…** — pre-fills source. Pick `main` as target in the Merge panel that opens. Hit Refresh preview, then Merge. If there's a conflict, the merge surfaces the resolution flow — see [chapter 16](16-merge-resolution.md).

### "I want to ensure a demo's layer can't be garbage-collected later."

Tags panel → **Create tag…** → name `demo-day-2026-04-15` and target the demo's commit. The tag's GC-protection survives long-term storage hygiene.

## 15.8. Design references

- [**D23** — Out-of-core layer architecture](../../design/d23-out-of-core-layer-architecture.md) §5.4–§5.5 — branch ref semantics, CAS write model.
- [**D34** — Notebook chain workspace](../../design/d34-notebook-chain-workspace.md) — the rail's destinations, panel responsibilities.
- [**chapter 4 §4.6**](04-cli-reference.md#46-branch-commands-require---endpoint) — the equivalent CLI surface.
- [**chapter 16** — Merge resolution](16-merge-resolution.md) — the next chapter; how to fold one branch into another when their contributions conflict.
- [**chapter 17 §17.4**](17-typescript-sdk.md#174-branches) — the programmatic SDK surface.

---

Next: **[16. Merge resolution →](16-merge-resolution.md)**
