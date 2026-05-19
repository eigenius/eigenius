# 16. Merge resolution

When two branches diverge with overlapping contributions, the kernel can't fold them together automatically — the merge would have to pick which body to keep at the conflicting IRIs, and that's a semantic decision. **Merge resolution** is the workflow that lets you make that decision per conflict: pick a strategy, preview the downstream consequences, acknowledge them, and commit a merge layer.

This chapter is the user's view of the resolution workflow. The full specification lives in:

- [D20 — Layer Reconciliation](../../design/d20-layer-reconciliation.md) — kernel-level semantics, conflict taxonomy, the resolution algebra.
- [D36 — Merge Resolution UX](../../design/d36-merge-resolution-ux.md) — notebook + CLI surface, state machine, error handling.
- [D37 — Lambda surface and typed merge comorphisms](../../design/d37-lambda-surface-and-typed-merge-comorphisms.md) — the typed witness surface.
- [D38 — Merge provenance and witness discovery](../../design/d38-merge-provenance-and-witness-discovery.md) — the audit-record + cross-branch witness scope.

## 16.1. When does merge resolution kick in?

Two ways into the same flow:

1. **A cell-commit race.** You ran a cell, but another commit reached the branch between when your cell read the head and when it tried to write. The kernel attempted a trivial merge; the contributions overlap; the kernel returned `NEEDS_WITNESSED_MERGE` and left the branch unchanged. The cell shows an error badge with a **Resolve in Merge rail** button.

2. **An explicit branch merge.** You opened the Merge panel (workspace rail's **Merge** destination, or via a Branches-panel row's **Merge into…** action), picked source + target, clicked **Refresh preview**, and the predicted outcome was **conflict**. Clicking **Merge** then surfaces a **Resolve conflicts** button on the result block.

Both routes land in the same `MergeResolutionFlow` component — the only difference is whether the "candidate head" being merged is your cell's orphaned commit or the source-branch tip.

## 16.2. The resolution flow at a glance

Six explicit states drive the panel; each transition is a deliberate user action (no silent auto-advancement):

```
loading ─→ picking ─→ previewing ─→ acknowledging ─→ committing ─→ done
   │          │            │              │              │
   └──────────┴────────────┴──────────────┴──────────────┴── error (recoverable, retryable)
   └──────────────────────── cancel ─────────────────────── closed
```

- **loading** — kernel returns the classified conflict list. Brief spinner.
- **picking** — one card per conflict with a radio list of applicable strategies. Strategies that don't apply are greyed out with a one-line rationale. The **Preview cascade** button enables once every conflict has a complete resolution.
- **previewing** — kernel computes downstream consequences. Spinner.
- **acknowledging** — the cascade preview lists every orphaned reference / typing / etc. produced by the picked resolutions. Each item needs a tick before **Commit merge** enables.
- **committing** — kernel applies the resolutions and commits the merge layer. Spinner.
- **done** — green success card with the merge layer id and a **Close** button. The Merge panel resets to the source/target form when closed.
- **error** — recoverable variants offer **Try again** (returns to the appropriate state); unrecoverable variants only offer **Close**.

State is persisted to `localStorage` keyed on `(branch, candidateHead)` so a page reload mid-resolution keeps your picks.

## 16.3. The four strategies

Eigenius v1 ships four resolution strategy variants — but **SchemaQuotient** has three sub-flavours, so the picker shows six radio buttons. The naming below mirrors what the UI labels.

### 16.3.1. Witness — typed merge term

Apply a committed `MergeComorphism` resource whose transformation lambda has signature `(A, A, Option<A>) → A` for the conflict's class `A`. The kernel runs the lambda against the two branch bodies plus the optional ancestor body and produces the merged value.

**Best for:** instance-level conflicts where the right body is computable from both sides — "take the average," "prefer the most recent measurement," "take side B's body unchanged," "field-wise merge."

**Authoring a witness:** ESL grew `merge_comorphism` + `lambda` + `pi` keywords in [D37](../../design/d37-lambda-surface-and-typed-merge-comorphisms.md). The simplest take-side-B witness is:

```esl
namespace project = "urn:project";

merge_comorphism project:patient_take_b for project:Patient {
    (a, b, opt) => b
}
```

This compiles to two chain resources:

1. A synthesised standalone `Lambda` at `urn:eigenius:auto:lambda:<sha256>` carrying the term + its declared Pi-type. The compiler computes the IRI from a content-hash so structurally-identical bodies dedupe naturally.
2. A `MergeComorphism` at `urn:project:patient_take_b` pointing at the synthesised lambda, with `merge_target_class = urn:project:Patient`.

The kernel's commit-time validators check both: the comorphism's required slots (Rule 18) and the lambda's body type-checks against its declared Pi (Rule 19, NbE-backed). Mis-shapes are rejected at commit time, not at apply time.

**Picker behavior — the WitnessEditor Combobox:**

Selecting the **Witness** radio reveals a Combobox listing every `MergeComorphism` on the chain whose `merge_target_class` matches the conflict's class. Source: [`notebooks/src/components/merge/WitnessEditor.tsx`](../../../notebooks/src/components/merge/WitnessEditor.tsx).

- **Instance-level conflicts** (the canonical Witness fit): both branches commit `resource project:patient_42 : project:Patient { … }` with different field values. The conflict's `is_a[0]` is `urn:project:Patient` — the picker queries against that.
- **Class-level conflicts** (e.g. both branches re-declare `class project:Patient`): the conflict's `is_a[0]` is `urn:eigenius:core:Class`, so witnesses authored `for project:Patient` don't surface. Use Rename or Restructure for these.

If no applicable witness exists on the active branch, the Combobox is replaced by a free-form IRI input + a hint pointing to the `merge_comorphism` ESL form. You can still author a witness on the fly (in a separate cell) and re-fire the merge.

**Cross-branch witness discovery (D38 §4):** below the Combobox, the **▸ Search additional branches** disclosure lets you extend the picker's scope. Type a branch name and press Enter (or click Add); the entry shows up as a chip and the Combobox re-queries against that branch's chain too. Witnesses surfaced via this path are copied into the merge layer's contributions at commit time so the resulting merge stays self-contained even if the source branch is later deleted — see §16.5.

### 16.3.2. Rename — disambiguate by renaming one side

The two branches independently chose the same IRI for genuinely different concepts. Renaming one side disambiguates them. The kernel:

1. Checks the new IRI doesn't collide with anything else in the chain (the other branch's contributions, the ancestor's parent chain, the renamed branch's *other* contributions).
2. Rewrites every reference to the old IRI within the renamed branch's slice so the rename is internally consistent.
3. The merged layer carries the renamed-side's body at the new IRI plus the other side's body at the old IRI (or tombstones the old IRI if only the renamed side had it).

**Best for:** namespace collisions — two teams chose `urn:project:Patient` for genuinely different concepts (medical-records vs. billing).

**Picker fields:** Side (`a` or `b`), Old IRI (pre-filled to the conflict's IRI), New IRI (free-form, validated).

### 16.3.3. SchemaQuotient — KeepBoth / KeepOne / KeepNeither

A quotient over the conflict point, three sub-flavours:

- **KeepBoth** — accept the freely-combined pushout (both contributions coexist). Only legal for conflict kinds that admit both sides structurally. **None of v1's classified kinds qualify**, so the radio is rendered but disabled with a kind-specific rationale (per [D36 §15.5](../../design/d36-merge-resolution-ux.md#155-keepboth-visibility-greyed-out-never-hidden)). Carried for forward compatibility — future conflict taxonomies may admit it.

- **KeepOne { winner }** — pick a winning side. The loser's contribution at the conflict point is dropped from the merge. The cascade gate flags everything downstream that referenced the dropped contribution (orphaned references, orphaned typings, …) so the user explicitly acknowledges the consequence.

- **KeepNeither** — drop *both* contributions. If the ancestor had a body at this IRI, the ancestor's body becomes the merged value; otherwise the IRI is tombstoned (post-merge `resolve` returns `None`). The cascade gate again flags every downstream reference.

**Best for:** property-type disagreements, kind-mismatches, and instance-level conflicts where neither side's body is salvageable into a merged form.

**Picker fields:** Quotient kind (radio) and, for KeepOne, **Winner** = `a` / `b`.

### 16.3.4. Restructure — raise the abstraction

Introduce a new common parent class and re-parent the conflicting classes under it. The classic motivating shape:

- Branch A: `Dog subclass_of Mammal`
- Branch B: `Dog subclass_of Reptile`

Restructure introduces a new `Animal` class, points both `Mammal` and `Reptile` at `Animal`, and re-parents `Dog` at `Animal` only. The original disagreement is sidestepped by raising the abstraction; downstream code that referenced `Mammal` or `Reptile` still works.

**Best for:** inheritance-cycle conflicts and any subclass-of disagreement where a common parent makes structural sense.

**Picker fields:**

- **Affected class** — pre-filled to the conflict's class IRI.
- **New parent IRI** — free-form. The kernel rejects synthesised IRIs (no `urn:eigenius:auto:*`); you must name the new class explicitly so the merged schema stays readable.
- **Introduce new parent class** — toggle. When on, the dialog grows a sub-form for the new Class resource (short_name + description); the kernel commits it as part of the merge.
- **Classes under new parent** — multi-select of every class in the merge span. Tick the ones that should subclass the new parent (e.g. tick `Mammal` and `Reptile`).
- **Re-parent affected class under new parent** — toggle. When on (the canonical Dog/Animal case), the affected class's `parent_classes` is replaced with `[new_parent]`; when off, the affected class keeps its existing ancestry and only the picked siblings move.

### 16.3.5. When does each strategy apply?

The radio for each strategy is disabled with a one-line tooltip explaining why if it doesn't apply to the conflict's kind. Summary:

| Conflict kind | Witness | Rename | KeepBoth | KeepOne | KeepNeither | Restructure |
|---|---|---|---|---|---|---|
| `IriCollision` (instance-level) | ✅ | ✅ | ❌ | ✅ | ✅ | — |
| `IriCollision` (class-level re-declare) | — | ✅ | ❌ | ✅ | ✅ | ✅ |
| `KindMismatch` | — | ✅ | ❌ | ✅ | ✅ | — |
| `PropertyDataType` | — | — | ❌ | ✅ | ✅ | — |
| `InheritanceCycle` | — | — | ❌ | ✅ | ✅ | ✅ |

`KeepBoth` is universally disabled in v1; the column is preserved for future taxonomies.

## 16.4. The cascade gate

Some resolutions have downstream consequences that aren't structurally part of the resolution itself — e.g. KeepNeither drops both bodies at an IRI, but other resources elsewhere on the chain referenced that IRI; after the merge those references will dangle. The **cascade preview** surfaces every such consequence so the user explicitly acknowledges them before commit.

Cascade items the kernel surfaces:

| Kind | When it fires | What it tells you |
|---|---|---|
| **Orphaned reference** | A Rename or KeepNeither or KeepOne drops or moves an IRI that's referenced elsewhere. | `<resource> → <dropped target>` at `<property_path>`. |
| **Orphaned typing** | A resolution drops a class definition that some resource was `is_a` of. | `<class>`: `<affected resources>`. |
| **Invalidated signature** | (reserved — not firing in v1) | A program's signature stops being valid post-merge. |
| **Invalidated trace** | (reserved — not firing in v1) | An execution trace's preconditions no longer hold. |

Each item has a stable `item_id` that's deterministic per `(span, resolutions)`; identical retries produce identical ids, so prior acks survive race-recovery.

The `acknowledging` state's pane shows the items grouped by kind with a checkbox per item. Long lists fold after 20 items per group; the **Commit merge** button enables once every box is ticked.

Witness resolutions have empty cascades by construction — a well-typed witness produces a value of the right class, so no downstream invariant breaks. The flow short-circuits straight from `previewing` to `committing` when the cascade is empty (no ack step).

## 16.5. Merge provenance records

Every committed merge layer carries one `MergeResolutionRecord` resource per resolved conflict, alongside the resolved bodies. The record's `@id` is content-hashed (`urn:eigenius:auto:merge-record:<sha256>`), so deterministic resolutions of the same conflict produce the same record IRI across runs.

Required slots:

| Slot | Value |
|---|---|
| `merge_record_conflict_id` | The classifier's opaque conflict id, e.g. `iri_collision:urn:project:patient_42`. |
| `merge_record_strategy` | `Witness` / `Rename` / `SchemaQuotient` / `Restructure`. |
| `merge_record_branch_a_source_layer` | Hex layer id contributing branch A's body at the conflict IRI (absent for cycle-shaped conflicts). |
| `merge_record_branch_b_source_layer` | Hex layer id contributing branch B's body at the conflict IRI. |
| `merge_record_ancestor_source_layer` | Hex layer id of the ancestor's body, if one exists at the conflict IRI. |

Strategy-specific optional slots:

| Slot | Set for | Value |
|---|---|---|
| `merge_record_witness` | Witness | IRI of the applied `MergeComorphism`. |
| `merge_record_witness_source_layer` | Witness | Hex layer id of the layer the witness was originally committed on, preserved after the copy into the merge layer (see below). |
| `merge_record_rename_side` | Rename | `a` or `b`. |
| `merge_record_rename_from_iri` | Rename | The IRI being renamed away from. |
| `merge_record_rename_to_iri` | Rename | The new IRI. |
| `merge_record_quotient_kind` | SchemaQuotient | `KeepBoth` / `KeepOne` / `KeepNeither`. |
| `merge_record_quotient_winner` | SchemaQuotient with KeepOne | `a` or `b`. |
| `merge_record_restructure_new_parent` | Restructure | IRI of the introduced common parent class. |
| `merge_record_restructure_affected_class` | Restructure | IRI of the class being re-parented. |

### 16.5.1. Inspecting records in the History panel

Open the History panel, locate the merge layer (named `merge:<head_a>+<head_b>`), and click **Inspect resources…** in the detail pane. The Layer Inspector lists every resource the merge layer contributes — including one `urn:eigenius:auto:merge-record:<sha256>` per resolved conflict.

### 16.5.2. Querying records via EigenQL

The records are chain-resident, so an EigenQL cell can find every merge-resolution event across the history:

```eigenql
USING "urn:eigenius:core:MergeResolutionRecord"

MATCH MergeResolutionRecord(?r) {
    "urn:eigenius:core:merge_record_conflict_id": ?conflict,
    "urn:eigenius:core:merge_record_strategy":    ?strategy
}
RETURN [] { record: ?r, conflict: ?conflict, strategy: ?strategy }
ORDER BY ?conflict
```

Or, scoped to a specific witness IRI ("show me every conflict resolved by `patient_take_b`"):

```eigenql
USING "urn:eigenius:core:MergeResolutionRecord"

MATCH MergeResolutionRecord(?r) {
    "urn:eigenius:core:merge_record_witness": ?w,
    "urn:eigenius:core:merge_record_conflict_id": ?conflict
}
WHERE ?w = "urn:project:patient_take_b"
RETURN [] { record: ?r, conflict: ?conflict }
```

### 16.5.3. The witness-copy guarantee

For Witness resolutions where the witness lives on a branch **outside** the merge span (surfaced via the WitnessEditor's search-branches disclosure), the kernel copies the `MergeComorphism` resource plus its transformation Lambda into the merge layer's contributions at their original IRIs. This makes the merge layer self-contained: `merge_record_witness` is guaranteed to resolve on the merge layer's own chain regardless of what happens to the source branch later.

For Witness resolutions where the witness is already reachable through the merge span (typical case — committed on one of the merged branches or on a shared ancestor), the kernel skips the copy. The merge layer's parent chain already pins the witness transitively; duplicating it would waste storage. The `merge_record_witness` IRI still resolves, just through the parent walk instead of via a contribution.

The `merge_record_witness_source_layer` slot is set in both cases and points at the *original* committing layer — preserving authoring-time attribution.

## 16.6. Resolving in the notebook

1. The cell badge or Merge panel surfaces **Resolve conflicts**. Click it. The Merge panel switches into resolution mode and renders `MergeResolutionFlow`.
2. The picker opens with one card per conflict. Each card carries:
   - The conflict kind + the IRI(s) involved.
   - A radio list of strategies. Inapplicable ones are disabled with a tooltip.
   - The strategy-specific editor (WitnessEditor / RenameEditor / QuotientEditor / RestructureEditor) for the picked option.
3. Fill in each card. The **Preview cascade** button at the bottom enables once every conflict has a complete resolution.
4. Click **Preview cascade**. The kernel returns the cascade item list. If it's empty, the flow short-circuits to `committing` — click **Commit merge**.
5. Otherwise, the acknowledging pane lists every cascade item with a checkbox. Tick each one. **Commit merge** enables.
6. Click **Commit merge**. The kernel applies your resolutions, commits the merge layer (carrying the resolved bodies + merge-resolution records + any off-span witness copies), and CAS-advances the target branch. The cell's error badge clears automatically.
7. The success card shows the merge layer id. Click **Close**. The Merge panel resets to a fresh source/target form.

### 16.6.1. What if the branch moves underneath?

If another commit reaches the branch between your Preview and your Commit, the kernel returns `BRANCH_CAS_RACE`. The notebook re-runs `prepareMerge` and shows a banner:

```
The branch moved
+1 new conflict: <iri>
-2 previously-resolved conflicts gone: <iri>, <iri>
```

Your prior strategy picks for surviving conflicts are preserved — only the changed ones need re-editing. The witness-search-branches list also survives across race recovery.

### 16.6.2. Cancelling

The **Cancel** button drops the session. The orphaned layer (your cell's would-be commit, if entered from a cell-commit race) stays on disk until garbage collection reclaims it — re-running the cell is the recovery path. For explicit-merge entry, no orphaned layer is produced; cancelling just closes the flow.

### 16.6.3. Error recovery

| Error kind | Title in the UI | What to do |
|---|---|---|
| `INCOMPLETE_ACKNOWLEDGMENTS` | Acknowledgments missing | Tick the missing items. The error pane lists their ids. |
| `BRANCH_CAS_RACE` | The branch moved | The notebook reloads automatically. |
| `CONFLICT_NOT_FOUND` | Conflict id became stale | Race-recovery; the notebook re-prepares. |
| `NO_COMMON_ANCESTOR` | No common ancestor | The two layers share no ancestor — usually a stale `candidate_head`. Re-fetch + retry. |
| `MALFORMED_RESOLUTION` | Resolution shape is invalid | The error message points at the offending field. Adjust the editor and try again. Common cause: a Witness resolution naming a comorphism that isn't reachable from the merge span and isn't named in the search-branches list. |
| `APPLICATION_PENDING` | Resolution strategy not yet wired | Reserved; shouldn't fire in current builds. |
| `INTERNAL` | Committing merge failed | Check the kernel logs. |

## 16.7. Resolving from the CLI

The notebook flow mirrors `eigenius db merge`. Useful for scripting, dry-runs against fixture data, and editing complex Restructure specs in JSON.

### 16.7.1. Preview cascade

```bash
eigenius --endpoint http://localhost:50051 db merge preview \
    --branch main \
    --candidate <hex-layer-id> \
    --resolutions resolutions.json
```

Prints each cascade item's stable id + a one-line body. Pipe those ids into `resolve --acknowledge`.

### 16.7.2. Resolve

```bash
eigenius --endpoint http://localhost:50051 db merge resolve \
    --branch main \
    --candidate <hex-layer-id> \
    --resolutions resolutions.json \
    --acknowledge <item-id-1> \
    --acknowledge <item-id-2>
```

On success, prints the merge layer id and the branch's new tip. On `INCOMPLETE_ACKNOWLEDGMENTS`, prints the missing ids with copy-pasteable `--acknowledge` lines.

### 16.7.3. Resolution-file format

`resolutions.json` is an array of objects, one per resolution. The four shapes:

```json
[
  {
    "conflict_id": "iri_collision:urn:project:patient_42",
    "kind": "witness",
    "comorphism_iri": "urn:project:patient_take_b"
  },
  {
    "conflict_id": "iri_collision:urn:project:Patient",
    "kind": "rename",
    "side": "a",
    "old_iri": "urn:project:Patient",
    "new_iri": "urn:project:billing:Patient"
  },
  {
    "conflict_id": "property_data_type:urn:project:weight",
    "kind": "schema_quotient",
    "quotient": "keep_one",
    "winner": "a"
  },
  {
    "conflict_id": "subclass_conflict:urn:project:Dog",
    "kind": "restructure",
    "affected_class": "urn:project:Dog",
    "new_parent": "urn:project:Animal",
    "new_parent_def": {
      "@id": "urn:project:Animal",
      "urn:eigenius:core:is_a": ["urn:eigenius:core:Class"],
      "urn:eigenius:core:short_name": "Animal",
      "urn:eigenius:core:description": "Common parent for Mammal and Reptile."
    },
    "classes_under_new": ["urn:project:Mammal", "urn:project:Reptile"],
    "affected_class_under_new": true
  }
]
```

Fields:

- `kind`: `"witness"`, `"rename"`, `"schema_quotient"`, or `"restructure"`.
- `side` / `winner`: `"a"` or `"b"`.
- `quotient`: `"keep_both"`, `"keep_one"`, or `"keep_neither"` (`winner` only consulted for `keep_one`).
- For `restructure`: omit `new_parent_def` when `new_parent` already exists in the chain; supply it inline when introducing a fresh class.

Both `preview` and `resolve` accept `--witness-search-branch <name>` (repeatable) — the CLI equivalent of the notebook's search-branches disclosure for off-span witnesses.

## 16.8. Worked examples

### 16.8.1. Patient IRI collision → Rename

Two teams in the same namespace independently introduced `urn:project:Patient` — one for medical records, one for billing.

1. Team A commits their changes; CAS succeeds.
2. Team B runs a cell that adds *their* `Patient`. The cell hits `NEEDS_WITNESSED_MERGE`.
3. Team B opens the Merge rail via the cell's **Resolve in Merge rail** button.
4. The rail shows one conflict: `IriCollision` on `urn:project:Patient`.
5. Team B picks **Rename** with Side = B, New IRI = `urn:project:billing:Patient`. The editor confirms the new IRI isn't taken elsewhere.
6. Preview cascade. The kernel reports: *"Profile.profile_for → urn:project:Patient — reference will dangle post-merge."* (Team A's pre-existing Profile referenced the old IRI; after the rename, that reference still resolves to *A's* Patient — semantically reasonable, but worth flagging.)
7. Team B ticks "I understand."
8. Commit. The merge layer is created; the branch advances; the cell's badge clears.
9. Open History → select the merge layer → **Inspect resources**. The layer's `MergeResolutionRecord` shows `strategy = "Rename"`, `rename_side = "b"`, `rename_from_iri = "urn:project:Patient"`, `rename_to_iri = "urn:project:billing:Patient"`.

### 16.8.2. Field-merge witness

Both branches commit `resource project:patient_42 : project:Patient { project:weight = … }` with different weights. The right post-merge body is the unweighted average.

```esl
namespace project = "urn:project";

merge_comorphism project:patient_avg_weight for project:Patient {
    (a, b, opt) => Patient {
        weight = (a.weight + b.weight) / 2.0
    }
}
```

In the picker, Witness → Combobox lists `patient_avg_weight`. Select it. Preview cascade is empty (a typed witness produces a value of the right class by construction). Commit. The merge layer's `patient_42` carries the averaged weight; the `MergeResolutionRecord` records the witness's IRI + its original source layer.

### 16.8.3. Dog/Mammal vs Dog/Reptile → Restructure

Branch A: `class project:Dog : project:Mammal`. Branch B: `class project:Dog : project:Reptile`. Classifier surfaces an `IriCollision` on `urn:project:Dog` (different `subclass_of` arrays). Picker:

- Strategy: **Restructure**.
- Affected class: `urn:project:Dog` (pre-filled).
- New parent IRI: `urn:project:Animal`.
- Introduce new parent class: on. Short_name = `Animal`; description = `Common parent for Mammal and Reptile.`.
- Classes under new parent: tick `urn:project:Mammal` and `urn:project:Reptile`.
- Re-parent affected class under new parent: on.

Preview cascade reports any references to `Mammal` / `Reptile` that the new parent change touches. Acknowledge → Commit. Post-merge:

- `urn:project:Animal` is a new chain-resident `Class`.
- `urn:project:Mammal` and `urn:project:Reptile` both subclass `urn:project:Animal`.
- `urn:project:Dog` subclasses `urn:project:Animal` only (the original disagreement is sidestepped).

### 16.8.4. Off-span witness

You maintain a `witness-library` branch holding reusable merge-comorphisms. The current merge is on `main`; the relevant witness lives only on `witness-library`.

1. Open the picker. Witness Combobox is empty — the active branch (`main`) has no applicable comorphisms.
2. Expand **▸ Search additional branches**. Type `witness-library`, press Enter (or click Add). A chip appears.
3. The Combobox re-queries against `witness-library`'s chain and surfaces the witness.
4. Select → Preview → Commit. The kernel resolves the witness via the fourth-tier search, copies the comorphism + its transformation Lambda into the merge layer at their original IRIs (so the merge layer remains self-contained if `witness-library` is later deleted), and produces the merge layer.
5. Inspect the merge layer's `MergeResolutionRecord`: `merge_record_witness` is the comorphism IRI; `merge_record_witness_source_layer` is `witness-library`'s tip layer id at the time of the merge.

## 16.9. Common gotchas

- **The picker is empty for an instance-level conflict, but a witness should exist.** Confirm the witness was committed on a branch reachable from the merge target (the WitnessEditor queries against the active branch's chain). If it lives on a sibling branch, use the search-branches disclosure.

- **The picker is empty for a class-level conflict, and a witness `for ClassName` exists.** Class-level conflicts (both branches re-declaring a Class) have `is_a[0] = urn:eigenius:core:Class`, not the class itself — so the witness query filters it out. Use Rename or Restructure for class-level redeclarations.

- **`MALFORMED_RESOLUTION: merge_target_class must be a ResourceRef to a Class`.** This usually means the comorphism resource exists but its `merge_target_class` slot is missing or shaped wrong. Author the comorphism via the `merge_comorphism` ESL form rather than constructing it manually as Eigon JSON; the compiler emits the slot in the right shape.

- **The free-form IRI input accepted my paste but Commit failed with `Witness comorphism IRI not found in the merge span`.** The picker shows a fallback Input when the chain has no applicable comorphisms; pasting an IRI there doesn't bypass the resolver. If the witness lives off-span, also add the branch to the **Search additional branches** disclosure before submitting.

- **Restructure rejects my new parent IRI as `urn:eigenius:auto:*`.** Synthesised IRIs are reserved for the compiler. Pick an explicit IRI in your project namespace.

- **The merge succeeds but `merge_record_witness_source_layer` points at a branch I've since deleted.** That's expected — the slot preserves the *original* committing layer's id for audit purposes. The comorphism + lambda were copied into the merge layer at commit time so `merge_record_witness` is still resolvable; only the historical attribution stays at the deleted layer.

## 16.10. Design references

- [**D20** — Layer Reconciliation](../../design/d20-layer-reconciliation.md) — kernel-level semantics, conflict taxonomy.
- [**D36** — Merge Resolution UX](../../design/d36-merge-resolution-ux.md) — notebook + CLI surface, state machine.
- [**D37** — Lambda surface and typed merge comorphisms](../../design/d37-lambda-surface-and-typed-merge-comorphisms.md) — the typed witness surface, ESL syntax, commit-time validators.
- [**D38** — Merge provenance and witness discovery](../../design/d38-merge-provenance-and-witness-discovery.md) — `MergeResolutionRecord` shape, the fourth-tier witness search, the off-span copy guard.
- [**chapter 15** — Tags, branches, and history](15-tags-branches-history.md) — the workspace panels driving the chain operations that lead into resolution.
- `eigenius db merge --help` — live CLI reference.

---

Next: **[17. TypeScript SDK →](17-typescript-sdk.md)**
