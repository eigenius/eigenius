# 11. Merge resolution

When two commits race against the same branch and modify the same resources, the kernel can't merge them automatically. **Merge resolution** is how the user picks a per-conflict strategy, sees the downstream consequences, acknowledges them, and commits a merged layer.

The full specification lives in [D20 — Layer Reconciliation](../../design/d20-layer-reconciliation.md) (kernel surface) and [D36 — Merge Resolution UX](../../design/d36-merge-resolution-ux.md) (notebook + CLI surface). This chapter is the user's view.

## 11.1. When does merge resolution kick in?

Two situations:

1. **A cell-commit race.** You ran a cell, but another commit reached the branch between when your cell read the head and when it tried to write. The kernel attempted a trivial merge; the contributions overlap, so it returned `NEEDS_WITNESSED_MERGE` and left the branch unchanged. The cell shows an error badge.

2. **An explicit branch merge.** You're folding `wip-types` into `main` from the Merge rail panel, and the predicted outcome is `NEEDS_WITNESSED_MERGE`.

Both situations route into the same resolution flow.

## 11.2. The six strategies

| Strategy | When to use it |
|---|---|
| **Witness** | You have a typed merge term (a committed `MergeComorphism`) that says how to combine the two bodies. Best for instance-level conflicts where the right body is computable from both sides — e.g., "take the more recent measurement," "take the unweighted average." |
| **Rename** | The two branches independently chose the same IRI for genuinely different concepts. Renaming one side disambiguates them. The kernel rewrites every reference to the old IRI within the renamed branch's slice. |
| **Keep both** | Accept the freely-combined pushout — both contributions coexist. Only legal for conflict kinds that admit both sides structurally (none of v1's classified kinds qualify, so this is currently never applicable; the radio is greyed out with a kind-specific rationale). |
| **Keep one** | Pick a winning side. The loser's contribution at the conflict point is dropped from the merge. The cascade preview flags everything downstream that referenced the dropped contribution. |
| **Keep neither** | Drop both contributions. If the ancestor had a body at this IRI, the ancestor's body becomes the merged value; otherwise the IRI is tombstoned (post-merge lookup returns `None`). |
| **Restructure** | Introduce a new common parent class and re-parent the conflicting classes under it. Sidesteps the disagreement by raising the abstraction. Classic example: `Dog subclass_of Mammal` vs `Dog subclass_of Reptile` → introduce `Animal`, put `Mammal` and `Reptile` both under `Animal`, point `Dog` at `Animal` only. |

## 11.3. Resolving in the notebook

When a cell hits `NEEDS_WITNESSED_MERGE`, it renders an inline error message with a **Resolve in Merge rail** button.

1. Click the button. The Merge rail opens in resolution mode.
2. Each conflict appears as a card with a radio list of applicable strategies. Strategies that don't apply (e.g., Keep both on a property-type conflict) are greyed out with a one-line rationale.
3. Pick a strategy per conflict and fill in the editor's fields. The Preview button enables once every conflict has a complete resolution.
4. Click **Preview cascade**. The notebook fetches the downstream consequences from the kernel and lists them — orphaned references, orphaned typings, etc.
5. Tick the "I understand" checkbox next to each item. The Commit button enables once everything is acknowledged.
6. Click **Commit merge**. The kernel applies your resolutions, commits the merged layer, and CAS-advances the branch. The cell's error badge clears automatically.

### What if the branch moves underneath?

If another commit reaches the branch between your Preview and your Commit, the kernel returns `BRANCH_CAS_RACE`. The notebook reloads the conflict list and shows a banner reporting what changed (`+1 new conflict`, `-2 previously-resolved conflicts gone`). Your prior strategy picks for surviving conflicts are preserved — you only re-edit the changed ones.

### Cancelling

The **Cancel** button drops the session. The orphaned layer (your cell's would-be commit) stays on disk until garbage collection reclaims it — re-running the cell is the recovery path.

## 11.4. Resolving from the CLI

The notebook's resolution flow is mirrored by `eigenius db merge`. Useful for scripting, dry-runs against fixture data, and the Restructure case if you prefer editing JSON over the notebook form.

### Preview cascade

```bash
eigenius --endpoint http://localhost:50051 db merge preview \
    --branch main \
    --candidate <hex-layer-id> \
    --resolutions resolutions.json
```

Prints each cascade item's stable id + a one-line body. Pipe those ids into `resolve --acknowledge`.

### Resolve

```bash
eigenius --endpoint http://localhost:50051 db merge resolve \
    --branch main \
    --candidate <hex-layer-id> \
    --resolutions resolutions.json \
    --acknowledge <item-id-1> \
    --acknowledge <item-id-2>
```

On success, prints the merge layer's id and the branch's new tip. On `INCOMPLETE_ACKNOWLEDGMENTS`, prints the missing ids with copy-pasteable `--acknowledge` lines.

### Resolution-file format

`resolutions.json` is an array of objects, one per resolution:

```json
[
  {
    "conflict_id": "iri_collision:urn:project:Patient",
    "kind": "witness",
    "comorphism_iri": "urn:project:patient_merge_witness"
  },
  {
    "conflict_id": "iri_collision:urn:project:Visit",
    "kind": "rename",
    "side": "a",
    "old_iri": "urn:project:Visit",
    "new_iri": "urn:project:billing:Visit"
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

- `kind`: `"witness"`, `"rename"`, `"schema_quotient"`, or `"restructure"`.
- `side` / `winner`: `"a"` or `"b"` (branch picker).
- `quotient`: `"keep_both"`, `"keep_one"`, or `"keep_neither"` (`winner` field only consulted for `keep_one`).
- For `restructure`: omit `new_parent_def` when `new_parent` already exists in the chain; supply it inline when introducing a fresh Class.

## 11.5. Errors you might see

The kernel maps typed error variants to a single `error_kind` enum the CLI and notebook both dispatch on:

| Error kind | Meaning | What to do |
|---|---|---|
| `INCOMPLETE_ACKNOWLEDGMENTS` | The cascade preview surfaced items you didn't acknowledge. | Tick the missing items (notebook) or add `--acknowledge <id>` flags (CLI). |
| `BRANCH_CAS_RACE` | The branch moved between preview and commit. | The notebook reloads automatically. From the CLI, re-run `preview` to get the fresh list. |
| `CONFLICT_NOT_FOUND` | A resolution targets a conflict id the classifier didn't produce. Usually a stale read. | Same as `BRANCH_CAS_RACE` — refresh. |
| `NO_COMMON_ANCESTOR` | The two layers share no ancestor in the topology. | Shouldn't happen from normal flow. Likely a stale `candidate_head`. |
| `MALFORMED_RESOLUTION` | The resolution's shape is invalid (unknown side, malformed IRI, bad Restructure JSON, etc.). | The error message points at the offending field. |
| `APPLICATION_PENDING` | The resolution validated but the commit path isn't wired. | Reserved; doesn't fire in current builds. |
| `INTERNAL` | Storage / handler failure. | Check the kernel logs. |

## 11.6. Worked example: the Patient IRI collision

Two teams in the same namespace independently introduced `urn:project:Patient` — one for medical records, one for billing.

**From the notebook:**

1. Team A commits their changes; CAS succeeds.
2. Team B runs a cell that adds *their* `Patient`. The cell hits `NEEDS_WITNESSED_MERGE`.
3. Team B opens the Merge rail via the cell's "Resolve in Merge rail" button.
4. The rail shows one conflict: `IriCollision` on `urn:project:Patient`.
5. Team B picks **Rename** with side = B, new IRI = `urn:project:billing:Patient`. The editor confirms the new IRI isn't taken elsewhere.
6. Preview cascade. The kernel reports: "Profile.profile_for → urn:project:Patient — reference will dangle post-merge." (Team A's pre-existing Profile referenced the old IRI; after the rename, that reference still resolves to *A's* Patient — semantically reasonable, but worth flagging.)
7. Team B ticks "I understand."
8. Commit. The merge layer is created; the branch advances; the cell's badge clears to ◆ merged.

**From the CLI:**

```bash
$ eigenius --endpoint http://localhost:50051 db merge preview \
    --branch main \
    --candidate $(cat orphan_layer_id) \
    --resolutions rename-patient.json

1 cascade item(s):
  orphaned_ref:urn:project:profile:urn:project:Patient:urn:project:profile_for
    OrphanedReference: urn:project:profile → urn:project:Patient (at urn:project:profile_for)

Acknowledge with: eigenius db merge resolve --acknowledge <ITEM_ID> [...]

$ eigenius --endpoint http://localhost:50051 db merge resolve \
    --branch main \
    --candidate $(cat orphan_layer_id) \
    --resolutions rename-patient.json \
    --acknowledge orphaned_ref:urn:project:profile:urn:project:Patient:urn:project:profile_for

Merge committed on main: layer 8b21…0f4
```

## 11.7. Further reading

- [D20 — Layer Reconciliation](../../design/d20-layer-reconciliation.md): kernel-level semantics, conflict taxonomy, the six-strategy resolution algebra.
- [D36 — Merge Resolution UX](../../design/d36-merge-resolution-ux.md): notebook + CLI surface, state machine, error-mode handling.
- `eigenius db merge --help`: live CLI reference.
