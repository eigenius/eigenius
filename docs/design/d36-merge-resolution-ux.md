# D36: Merge Resolution UX

*Design document for the Eigenius project — May 2026*

**Status:** Implemented (six-state flow, four strategies, cascade gate, chain-resident provenance live in the notebook)
**Required before:** the first notebook release that ships Phase 15 — until D36 lands the notebook user has no way to resolve a conflict in-place.
**Builds on:** [D20 — Layer Reconciliation](d20-layer-reconciliation.md), [D22 — Notebook UX and TypeScript SDK](d22-notebook-and-typescript-sdk.md), [D34 — Notebook Chain Workspace](d34-notebook-chain-workspace.md).
**Supersedes:** D34 §6.2's `WitnessedMergeRecoveryDialog`. The "save as sibling / pin-rebase / discard" escape hatches are removed in favor of in-place resolution. We have not shipped any notebook release, so no muscle memory needs to be preserved.

---

## 1. Overview

### 1.1 Scope: two-divergent-head reconciliation

D20 / Phase 15 is the kernel's **branch-merge** surface: it reconciles two heads `head_a` and `head_b` that descend from a common ancestor and may have diverging contributions. Two situations produce this two-head shape:

- **Explicit branch merge.** The user is on `wip-types` and wants to fold it into `main`. Two distinct branch refs, one explicit operation.
- **CAS-race / implicit divergence.** A cell read the branch tip `T1`, built a new layer atop it, and the branch advanced to `T2` before the cell's CAS landed. The cell's would-be layer and `T2` are two divergent heads from a common ancestor (`T1`'s parent path) even though only one branch ref exists.

D36 covers both — the kernel doesn't distinguish them (`merge_with_resolutions` takes a `MergeSpan` regardless of source), and the UX in §§4–8 routes them through the same resolution flow.

Phase 15 / D20 is structurally complete in the kernel: every conflict the branch-merge classifier surfaces can be resolved through one of six strategies (`Witness`, `Rename`, `KeepBoth`, `KeepOne`, `KeepNeither`, `Restructure`), each strategy's commit path produces a real multi-parent merge layer, and the gRPC handlers `SubmitResolution` and `PreviewCascade` are wired through the kernel server. The CLI driver `eigenius db merge resolve / preview` exists.

What's missing is the **notebook UX** — the surface most users actually see. Today a cell that races against another commit and hits `NEEDS_WITNESSED_MERGE` opens a dialog whose only options are "save as sibling," "pin and rebase," or "discard." None of these resolves the conflict; they all sidestep it. D36 replaces that dialog with an in-place resolution flow that lets the user pick a strategy per conflict, preview the downstream consequences, acknowledge them, and commit.

D36 also adds one kernel-side RPC (`PrepareMerge`) that the existing surfaces don't cover — the notebook needs the typed-conflict list before it can show strategy buttons, and neither `MergeBranches` nor `PreviewMerge` exposes that shape today.

### 1.2 Out of scope: within-chain commit-time conflict detection

A separate concern uses the *same* classifier algorithm but a different entry point: a new layer being committed to a chain where there's no divergence — just monotonic-forward progress — but whose content conflicts with what the chain already says. Example: layer `N` defines `urn:project:weight` as `Property data_type=Integer`; the user attempts to commit layer `N+1` redefining it with `data_type=String`. No divergence, no branch refs in play; the conflict shows up at commit time as a chain-validity violation.

The detection is structurally similar (same `ConflictKind` shapes — `PropertyDataType`, `KindMismatch`, etc.) but the user-facing flow is different: the layer is rejected before it lands, not after, and remediation is "edit the layer and re-commit," not "pick a resolution strategy." There's no `Witness` for redefining `weight`; the user just can't do it.

D36 is **strictly scoped to two-divergent-head reconciliation**. Within-chain commit-time validation is independent work, to be addressed in a separate design doc when the kernel grows that detection surface.

### 1.3 Audience

- **Notebook authors and ontology engineers** — the primary consumer. They've hit a conflict and want it resolved before they lose context.
- **Implementers** — frontend (notebook), orchestrator (passthrough proxies), kernel (one new RPC + handler).
- **Design reviewers** — the strategy-editor inventory in §6 and the test plan in §12 are the schedule-bearing artifacts.

### 1.4 Non-goals (beyond §1.2)

- **Conflict prevention.** Resolution is the *cure*; D34 §6 already covers the trivial-merge fast path that handles most prevention. D36 only kicks in when prevention isn't enough.
- **Cross-conflict batch operations.** A merge that surfaces 47 conflicts of the same kind would benefit from "apply this strategy to all of them" — out of scope. Each conflict is resolved individually. §13 notes this as a known gap.
- **Conflict re-classification at submission time.** If the branch moves between `PrepareMerge` and `SubmitResolution`, the user re-runs the flow against the fresh state. We don't try to migrate resolutions across snapshots.
- **Resolution history audit log.** Who clicked which strategy and when is a future feature; for now, the resolution submission itself becomes a chain commit with normal commit-attribution.
- **Multi-user collaboration on a resolution session.** Same single-user MVP scope as D22/D34.

### 1.5 Relationship to other documents

- **D20 §6** defines the six resolution strategies and the cascade ack discipline. D36 specifies the surface that exposes them.
- **D20 §7.1–7.3** specifies the gRPC API. D36 reuses `SubmitResolution` and `PreviewCascade` verbatim and adds `PrepareMerge` as the front-door RPC the notebook needs.
- **D34 §6.1** ("implicit trivial merge surfaced as a commit outcome") stays — it covers the FAST_FORWARD / TRIVIAL_MERGE cases. D36 replaces the NEEDS_WITNESSED_MERGE branch of that flow.
- **D34 §6.2** (`WitnessedMergeRecoveryDialog`) is removed. Its three escape hatches don't survive: "save as sibling" duplicates work, "pin and rebase" loses context, "discard" loses work outright. In-place resolution is the only path.
- **D34 §6.3** (explicit `MergeBranches` panel) stays as the entry point for "fold branch X into Y." D36 extends the same panel with the resolution flow when the predicted outcome is needs-resolution.

---

## 2. Status today

What's done end-to-end, what's missing.

| Layer | Status | Notes |
|-------|--------|-------|
| Kernel `crate::layer::merge` | ✅ done | Six strategies, cascade preview, tombstones, multi-parent merge layer build. |
| gRPC `SubmitResolution` | ✅ done | [`kernel/src/server/mod.rs`](../../kernel/src/server/mod.rs). |
| gRPC `PreviewCascade` | ✅ done | Same. |
| gRPC `PrepareMerge` | ❌ **D36 §3** | New RPC; wraps `build_merge_span` + `classify_conflicts`. |
| CLI `db merge resolve` / `db merge preview` | ✅ done | [`cli/src/main.rs`](../../cli/src/main.rs). |
| Orchestrator passthrough | ❌ **D36 §9.1** | Three lines in [`orchestration/src/notebook/eigenius_kernel_passthrough.ts`](../../orchestration/src/notebook/eigenius_kernel_passthrough.ts). |
| TS bindings regen | ❌ **D36 §9.1** | `buf generate` to refresh [`orchestration/src/gen/eigenius_pb.ts`](../../orchestration/src/gen/eigenius_pb.ts). |
| Notebook resolution flow | ❌ **D36 §§4–8** | The bulk of this doc. |

---

## 3. Wire surface additions

One new RPC. The other two (`SubmitResolution`, `PreviewCascade`) already ship.

### 3.1 `PrepareMerge` RPC

```proto
service EigeniusKernel {
  // ...
  // Pre-compute the typed-conflict list for a (branch, candidate_head)
  // pair. Non-mutating. The browser calls this to populate the
  // resolution flow's strategy picker; the response carries enough
  // information that the picker knows which strategies apply per
  // conflict.
  rpc PrepareMerge(PrepareMergeRequest) returns (PrepareMergeResponse);
}

message PrepareMergeRequest {
  string branch = 1;
  // Hex-encoded candidate head: the layer the caller built that
  // diverged from the branch's current tip. Same shape as
  // SubmitResolutionRequest.candidate_head.
  string candidate_head = 2;
}

message PrepareMergeResponse {
  bool success = 1;
  string error = 2;
  PrepareMergeErrorKind error_kind = 3;
  // Empty when the branch and candidate are equal or the candidate
  // is a fast-forward from the branch (no resolution needed).
  repeated TypedConflictWire conflicts = 4;
  // Hex-encoded current tip — what the resolution will CAS-advance
  // from. The browser stashes this and compares to the post-resolve
  // branch tip; on mismatch, the user knows the branch moved.
  string branch_tip = 5;
}

enum PrepareMergeErrorKind {
  PREPARE_MERGE_ERROR_KIND_UNSPECIFIED = 0;
  PREPARE_MERGE_ERROR_KIND_NO_COMMON_ANCESTOR = 1;
  PREPARE_MERGE_ERROR_KIND_INTERNAL = 2;
}

// A single typed conflict from the classifier. The `kind` oneof
// mirrors `crate::layer::merge::ConflictKind`.
message TypedConflictWire {
  string id = 1;
  oneof kind {
    PropertyDataTypeConflict property_data_type = 2;
    KindMismatchConflict kind_mismatch = 3;
    IriCollisionConflict iri_collision = 4;
    InheritanceCycleConflict inheritance_cycle = 5;
    // Reserved for future stage-2 / stage-3 kinds (DeletionConflict,
    // DisjointnessViolation, PathEquationContradiction).
  }
  // Strategies whose applicability check passes for this kind. The
  // browser uses this to grey out inapplicable buttons.
  repeated MergeStrategyKind applicable_strategies = 6;
}

message PropertyDataTypeConflict {
  string property = 1;
  string branch_a_type = 2;
  string branch_b_type = 3;
  // Empty if the property was branch-introduced.
  string ancestor_type = 4;
}

message KindMismatchConflict {
  string iri = 1;
  string branch_a_kind = 2;  // "Class" | "Property" | "Other"
  string branch_b_kind = 3;
}

message IriCollisionConflict {
  string iri = 1;
  // Eigon-JSON-encoded bodies (so the UI can render a diff). Lighter
  // than CBOR for a body-disagreement display and matches how the
  // kernel's other diagnostic surfaces ship resource snapshots.
  string branch_a_body_json = 2;
  string branch_b_body_json = 3;
  string ancestor_body_json = 4;  // empty if absent in ancestor
}

message InheritanceCycleConflict {
  repeated string cycle = 1;
}

enum MergeStrategyKind {
  MERGE_STRATEGY_KIND_UNSPECIFIED = 0;
  MERGE_STRATEGY_KIND_WITNESS = 1;
  MERGE_STRATEGY_KIND_RENAME = 2;
  MERGE_STRATEGY_KIND_KEEP_BOTH = 3;
  MERGE_STRATEGY_KIND_KEEP_ONE = 4;
  MERGE_STRATEGY_KIND_KEEP_NEITHER = 5;
  MERGE_STRATEGY_KIND_RESTRUCTURE = 6;
}
```

Server-side: thin handler that calls `build_merge_span` + `classify_conflicts`, then `encode_typed_conflict` per conflict, then a `applicable_strategies_for(kind)` helper (mirroring the existing applicability rules in `apply_quotient_resolution`).

The `Witness`, `Rename`, and `Restructure` strategies apply to every kind (Witness via type-checked term, Rename always, Restructure for class-shaped conflicts) — so `applicable_strategies` is mostly redundant for those three. For SchemaQuotient the applicability table from D20 §6.3 drives it: `KeepBoth` only fires when both sides admit coexistence (no v1 kind qualifies), `KeepOne` and `KeepNeither` apply universally to single-IRI conflict kinds.

### 3.2 `MergeInfo` flat conflict list — kept

`MergeBranchesResponse.merge.conflicting_iris` stays as a summary (count + first few IRIs) for the toast / panel header. The resolution flow uses `PrepareMerge` for the structured list. Keeping both isolates the toast-time read (cheap, no extra round trip) from the full-resolution read (one extra round trip when the user opens the flow).

### 3.3 No protocol-level deprecations

Since we haven't shipped, there's no field to mark deprecated. The four wire types above are added cleanly; the existing surface stays. `SubmitResolutionRequest.acknowledgments` and `PreviewCascadeRequest.resolutions` already encode the rest of the flow.

---

## 4. Information architecture

The resolution flow lives in the **Merge rail panel** ([`notebooks/src/components/workspace/MergePanel.tsx`](../../notebooks/src/components/workspace/MergePanel.tsx)). One panel hosts both the explicit-merge flow (D34 §6.3) and the resolution flow (D36).

### 4.1 Why the rail panel, not a modal

A merge with N conflicts is genuinely workspace-sized. The user needs to:
- Scroll a list of conflicts to read each one.
- Compare branch-A vs branch-B bodies side-by-side for `IriCollision`.
- Inspect a `MergeComorphism` resource the chain already has, before picking it as a Witness.
- Refer back to the cell that triggered the conflict.

A modal can't host this. The rail panel can, and the user can switch to other rail destinations (History, Branches, Inspect) without losing the resolution session — the state lives in the panel's store.

### 4.2 Panel modes

The Merge panel switches between two modes based on inputs:

```
                    ┌──────────────────────────────────────────┐
                    │  Merge                                   │
                    │                                          │
                    │  Source  [ wip-types         ▾ ]         │
                    │  Target  [ main              ▾ ]         │
                    │                                          │
                    │  ┌── Preview ──────────────────────────┐ │
                    │  │ 12 layers ahead, 4 behind           │ │
                    │  │ Predicted: trivial merge — clean    │ │
                    │  └─────────────────────────────────────┘ │
                    │                                          │
                    │             [ Refresh ]  [ Merge ]       │
                    └──────────────────────────────────────────┘
                                  ▲
                       explicit-merge mode (D34 §6.3)
                                  │
                                  │  user clicks Merge,
                                  │  outcome = NeedsResolution
                                  ▼
                    ┌──────────────────────────────────────────┐
                    │  Merge — resolving conflicts             │
                    │                                          │
                    │  Source  wip-types        @  8b21…0f4    │
                    │  Target  main             @  c9a4…11b    │
                    │                                          │
                    │  3 conflicts                             │
                    │  ┌──────────────────────────────────────┐│
                    │  │ ▼ Patient — IRI collision            ││
                    │  │   strategy: ( ) Witness              ││
                    │  │             ( ) Rename               ││
                    │  │             (•) KeepOne — winner: A  ││
                    │  │             ( ) KeepNeither          ││
                    │  ├──────────────────────────────────────┤│
                    │  │ ▼ weight — property data-type        ││
                    │  │   ( ) Witness   (•) KeepOne — A      ││
                    │  ├──────────────────────────────────────┤│
                    │  │ ▷ Dog — IRI collision                ││
                    │  └──────────────────────────────────────┘│
                    │                                          │
                    │   [ Preview cascade ]   [ Cancel ]       │
                    └──────────────────────────────────────────┘
                                  ▲
                       resolution mode (this doc)
```

Modes are mutually exclusive within a session. Switching back to explicit-merge mode (e.g., picking a different source branch) discards the partially-built resolution; the panel warns first.

### 4.3 Cell-commit failure routes to the panel

When a cell commit returns `MergeOutcome::NEEDS_WITNESSED_MERGE`, the cell shows an error badge with a single action: **Resolve in Merge rail**. Clicking it:

1. Opens the Merge rail.
2. Sets the panel's source = `<implicit_branch>` (the orphan layer's id treated as a synthetic source).
3. Sets target = the branch the user was committing to.
4. Switches the panel into resolution mode with `prepareMerge` pre-loaded.

No standalone dialog. The cell stays in its error state until the user resolves (and the orphan is committed as part of the merge layer) or explicitly discards.

---

## 5. Resolution flow state machine

The panel's resolution mode is a six-state machine.

```
       opens with (source_tip, target_tip)
                  │
                  ▼
         ┌─────────────────┐
         │  loading        │  prepareMerge call in-flight
         └────────┬────────┘
                  │ conflicts received (or empty → switch to "trivial" branch)
                  ▼
         ┌─────────────────┐
         │  picking        │  user assigns a strategy to each conflict
         └────────┬────────┘
                  │ all conflicts have a strategy; user clicks "Preview cascade"
                  ▼
         ┌─────────────────┐
         │  previewing     │  previewCascade in-flight
         └────────┬────────┘
                  │ cascade items received
                  ▼
         ┌─────────────────┐
         │  acknowledging  │  user ticks "I understand" for each item
         └────────┬────────┘
                  │ all items acked; user clicks "Commit merge"
                  ▼
         ┌─────────────────┐
         │  committing     │  submitResolution in-flight
         └────────┬────────┘
                  │
            ┌─────┴─────┐
            ▼           ▼
         done         error
         ──────       ─────────────────
         • merge      • branch CAS race → back to "loading"
           layer id   • incomplete acks → back to "acknowledging"
         • banner +   • malformed res. → back to "picking" with field highlight
           "Open in   • internal → red banner, "Try again" / "Open issue"
           History"
```

State transitions are explicit. The panel never silently auto-advances; the user always sees a button to push forward, and a "Back" / "Cancel" to bail out. The state lives in `chainStore.mergeResolution`.

### 5.1 Why the explicit `previewing` step

Calling `previewCascade` lazily (e.g., on every keystroke in a strategy editor) creates round-trip noise the user doesn't need. A single `Preview cascade` button:
- Makes the round trip explicit.
- Lets the user batch multiple strategy changes before paying for the recompute.
- Makes "I'm done picking, now show me consequences" a deliberate transition.

The button is greyed until every conflict has a strategy assigned.

### 5.2 Why the explicit `acknowledging` step

D20 §8 mandates an acknowledgment gate. The UX makes this a real action: each cascade item gets its own checkbox; commit button is disabled until all are checked.

This is the only place in the notebook UX where a user is forced to interact with each item individually. The friction is intentional — D20 §8 calls out *"the user has to see N consequences before committing"* as the design constraint, and the only way to honor that is to make N clicks observably necessary.

---

## 6. Strategy editors

One component per `MergeStrategyKind`. Each takes:

```ts
type StrategyEditorProps = {
  conflict: TypedConflictWire;
  // Driven by the panel's state machine; the editor calls this on
  // each field change. The panel persists the resolution in its
  // store and re-renders the cascade preview as stale.
  onChange: (resolution: MergeResolutionWire | null) => void;
  // For browsing committed resources (Witness picker, Restructure
  // editor's classes_under_new multi-select). Returns IRIs reachable
  // from the merge span.
  chainBrowser: ChainBrowserClient;
};
```

`onChange(null)` means "the user started editing but the form is incomplete; the resolution isn't submittable yet." The panel uses this to keep the `Preview cascade` button greyed.

### 6.1 `WitnessEditor`

```
┌────────────────────────────────────────────────────────────┐
│ Witness                                                    │
│                                                            │
│ Apply a typed merge term ((A, A, Option<A>) → A) committed │
│ earlier in the chain. The kernel type-checks the term      │
│ against the conflict's class at submission time.           │
│                                                            │
│ Comorphism IRI                                             │
│  [ urn:project:patient_merge_witness          ] [ Browse ] │
│                                                            │
│ ⚠ This IRI doesn't resolve. Did you mean:                  │
│   • urn:project:patient_merge                              │
│   • urn:project:patient_witness                            │
└────────────────────────────────────────────────────────────┘
```

- Free-text IRI input plus a `Browse` button opening a side panel with all `MergeComorphism` resources reachable from the span. Browse pulls via `Query` + `is_a urn:eigenius:core:MergeComorphism` (the SDK already supports filter-by-class).
- Inline validation: as the user types, debounced `inspect(iri)` against the branch shows three states: ✓ resolves to a `MergeComorphism`, ⚠ resolves but not a `MergeComorphism`, ✗ doesn't resolve.
- The "did you mean" suggestions come from a fuzzy match against the browse list (cheap, client-side).
- `onChange` fires with the resolution as soon as the IRI resolves to a valid `MergeComorphism`; greys out otherwise.

### 6.2 `RenameEditor`

```
┌────────────────────────────────────────────────────────────┐
│ Rename                                                     │
│                                                            │
│ Apply a disambiguating IRI rename to one side of the span. │
│ The rename rewrites references within the renamed side's   │
│ slice; references outside that slice stay pointing at the  │
│ old IRI (the cascade preview will surface those as         │
│ orphaned references).                                      │
│                                                            │
│ Which side to rename?    ( ) Branch A   (•) Branch B       │
│                                                            │
│ Old IRI                                                    │
│  urn:project:Patient            (set automatically)        │
│                                                            │
│ New IRI                                                    │
│  [ urn:project:billing:Patient                ]            │
│                                                            │
│ ✓ Doesn't collide with anything else in the span.          │
└────────────────────────────────────────────────────────────┘
```

- Side radio (A / B). Old IRI is read-only (it's `conflict.iri` for the relevant kinds).
- New IRI input with live collision check: `inspect(new_iri)` on the merge span. Three states: ✓ free, ⚠ exists in the rename side's other contributions (will collide), ✗ exists in the other branch or ancestor chain (will silently re-bind — corresponds to `RenameCollision` kernel error).
- The Rename editor only shows when the conflict has an `iri` field (IriCollision / KindMismatch / PropertyDataType). For InheritanceCycle, Rename is offered against any IRI in the cycle (a sub-picker appears).

### 6.3 `QuotientEditor`

```
┌────────────────────────────────────────────────────────────┐
│ Quotient                                                   │
│                                                            │
│  ( ) Keep both                                             │
│      Not applicable to this conflict (data_type is single- │
│      valued; both bodies can't coexist).                   │
│                                                            │
│  (•) Keep one                                              │
│      Winner: ( ) Branch A           (•) Branch B           │
│      Branch A's body will be dropped; Branch B's body      │
│      becomes the merged value.                             │
│                                                            │
│  ( ) Keep neither                                          │
│      Both bodies will be dropped. The ancestor's body      │
│      (urn:project:Patient @ ancestor:c9a4…) will be        │
│      committed in the merge layer.                         │
└────────────────────────────────────────────────────────────┘
```

- Three radio options. Each option is enabled only if `conflict.applicable_strategies` contains the matching `MergeStrategyKind`.
- `KeepOne` exposes a nested side radio.
- `KeepNeither` previews what the merge layer will contain: if the ancestor has a body, the resolved value is the ancestor's body (with the body rendered inline as a folded `<details>` block); if absent, the editor states "The IRI will be tombstoned — `resolve(iri)` returns None post-merge."

### 6.4 `RestructureEditor`

The heaviest editor. Walks the user through four sub-fields:

```
┌────────────────────────────────────────────────────────────┐
│ Restructure                                                │
│                                                            │
│ Introduce a new parent class so the conflicting class can  │
│ subclass it directly, sidestepping the disagreement.       │
│                                                            │
│ ─────── Affected class ──────────────────────────────       │
│                                                            │
│ Dog            (set automatically — the conflict's class)  │
│                                                            │
│ ─────── New parent ──────────────────────────────────       │
│                                                            │
│  New parent IRI                                            │
│   [ urn:project:Animal                          ]          │
│                                                            │
│  This IRI doesn't exist in the chain yet.                  │
│  ▼ Define the new class                                    │
│    Short name [ Animal                          ]          │
│    Description [ Common parent for Mammal/Reptile ___ ]    │
│    Parent classes [ + add row ]                            │
│                                                            │
│ ─────── Existing classes to subclass it ────────────       │
│                                                            │
│  ☑ urn:project:Mammal                                      │
│  ☑ urn:project:Reptile                                     │
│  ☐ urn:project:Bird                                        │
│  ☐ urn:project:Fish                                        │
│  [ + Add class IRI ]                                       │
│                                                            │
│ ─────── Affected class placement ────────────────────      │
│                                                            │
│  ☑ Dog subclasses Animal directly (replaces its current    │
│    parents Mammal and Reptile)                             │
└────────────────────────────────────────────────────────────┘
```

- Affected class is read-only.
- `new_parent` IRI input with the "exists or not" probe driving conditional rendering of the new-class definition section.
- New-class definition is a mini resource builder: `short_name` + `description` + optional `parent_classes` list. Reuses Fluent's `Field` + `Input` + an existing IRI-list editor (already in the notebook for `class_types` editing — single component, two consumers).
- `classes_under_new` is a checkbox list pulling all Classes reachable from the span (via Browse), pre-checked for the obvious candidates if heuristically derivable from the conflict. User can also free-type IRIs not in the list (e.g., classes they're about to introduce).
- `affected_class_under_new` toggle, defaulting to true (matches the D20 §6.4 motivating example).

The Restructure editor is the largest single piece of UI in D36. §14's PR 3 separates it from PR 2 so the rest of the flow can ship first.

### 6.5 KeepBoth — always greyed out, always documented

The `KeepBoth` radio in `QuotientEditor` is rendered but disabled for every v1 conflict kind, with the inline explanation `"Not applicable to this conflict (data_type is single-valued; both bodies can't coexist)."` The verbiage adapts to the conflict kind. This is deliberate: showing the option as greyed teaches the user that the strategy exists in the taxonomy, and surfaces the structural reason why it doesn't apply here — which prepares them for future taxonomies that admit it.

---

## 7. Cascade preview + acknowledgment

The `acknowledging` state renders the `CascadeItemWire[]` from `previewCascade`. One section per item kind:

```
┌────────────────────────────────────────────────────────────┐
│ Consequences                                               │
│                                                            │
│ ─── Orphaned references (2) ─────────────────────────       │
│                                                            │
│ ☑ urn:project:profile.profile_for → urn:project:Patient   │
│    Reference will dangle after Branch B's rename.          │
│                                                            │
│ ☐ urn:project:visit.subject → urn:project:Patient          │
│    Reference will dangle after Branch B's rename.          │
│                                                            │
│ ─── Orphaned typing (1) ────────────────────────────       │
│                                                            │
│ ☐ urn:project:Animal — 4 resources typed as Animal will    │
│   lose their typing.                                       │
│    ▼ Affected resources:                                   │
│       urn:project:pet_42                                   │
│       urn:project:pet_43                                   │
│       urn:project:zoo_1                                    │
│       urn:project:zoo_2                                    │
└────────────────────────────────────────────────────────────┘

         [ Back to picking ]    [ Commit merge ] (1/3 acked)
```

- Each item has an "I understand" checkbox keyed by `item_id`.
- Item bodies render kind-specifically:
  - `OrphanedReference`: source resource IRI · target IRI · property path · one-line "why this matters" string.
  - `OrphanedTyping`: class IRI · affected count · expandable list of affected resources (collapsed by default if > 5).
  - `InvalidatedSignature` / `InvalidatedTrace`: reserved by the kernel — rendered as "informational" if they ever fire.
- The `Commit merge` button counter (`1/3 acked`) is the primary affordance for the user to see how far they are.
- `Back to picking` returns to the strategy picker without losing the partially-acked state; if the user re-enters `acknowledging` with the same resolutions, their acks are preserved (item ids are deterministic per D20 §8). If resolutions change, acks for items no longer in the new preview are dropped silently.

### 7.1 The empty-cascade case

Some resolutions (every `Witness`, KeepOne when the loser doesn't have external refs) produce an empty cascade. The flow short-circuits: `previewing → committing` directly, with the `acknowledging` state skipped entirely. The panel shows a one-line "Resolutions are self-contained — no downstream consequences." banner so the user isn't surprised by the absence of the ack step.

---

## 8. Entry points

Two entry points lead to the resolution flow.

### 8.1 Cell-commit failure (CAS-race / implicit divergence)

The current `NEEDS_WITNESSED_MERGE` handling in [`notebooks/src/runtime/commitMeta.ts`](../../notebooks/src/runtime/commitMeta.ts) and the recovery dialog are replaced:

- The cell receives a `MergeInfo.outcome = NEEDS_WITNESSED_MERGE` from its commit response.
- The cell renders an error badge with a single action button: **Resolve in Merge rail**.
- Clicking it opens the Merge rail destination, sets the panel into resolution mode with `source = orphan_layer_id`, `target = branch_at_commit_time`, and triggers `prepareMerge`.
- The orphan layer is preserved on disk for the duration of the resolution session (the user might cancel and re-trigger). The flow's `committing → done` transition turns it into part of the merge layer's parent set; cancelling without committing leaves it GC-eligible.
- On successful resolution (`committing → done`), the cell's error badge auto-clears and is replaced by a green footer note `"Merged in resolution at layer 8b21…0f4 — View"` for ~30 seconds. The merge layer commit includes the cell's orphan layer as a parent, so the cell's work landed; the footer makes that visible. After dismissal the cell looks like a normally-committed cell, distinguishable in History via the `merge:…` layer name (resolution attribution beyond that is deferred — see §15).
- No "save as sibling," no "pin and rebase," no "discard." If the user genuinely doesn't want to resolve, they cancel and re-edit the cell. The orphan layer gets GC'd. This is a cleaner contract than the previous three-option dialog.

### 8.2 Explicit `MergeBranches` from the panel

The Merge panel already accepts source/target pickers (D34 §6.3). When the predicted outcome is `NEEDS_WITNESSED_MERGE`:
- The "Merge" button changes to "Resolve…"
- Clicking switches the panel into resolution mode (no separate dialog).

The source-tip + target-tip pair becomes the `(branch=target, candidate_head=source_tip)` input to `prepareMerge`. Same handler from there.

### 8.3 No standalone "open resolution flow" command

The panel is always reached via a real conflict. We don't offer a "let me play with the resolution UI on an arbitrary span" entry point — that's the CLI's job (`db merge preview`).

---

## 9. SDK additions

### 9.1 Orchestrator passthrough

Four additions to [`orchestration/src/notebook/eigenius_kernel_passthrough.ts`](../../orchestration/src/notebook/eigenius_kernel_passthrough.ts) (three RPCs + one wire shape touch).

```ts
prepareMerge:     (req) => proxy(operation.KERNEL_PASSTHROUGH_PREPARE_MERGE,     kernel.raw.prepareMerge,     req),
previewCascade:   (req) => proxy(operation.KERNEL_PASSTHROUGH_PREVIEW_CASCADE,   kernel.raw.previewCascade,   req),
submitResolution: (req) => proxy(operation.KERNEL_PASSTHROUGH_SUBMIT_RESOLUTION, kernel.raw.submitResolution, req),
```

Plus three `KERNEL_PASSTHROUGH_*` operation constants. The passthrough deliberately doesn't wrap or transform the responses — D34 §11 sets the precedent that orchestrator passthroughs are mechanical proxies. The notebook handles the typed-error dispatch.

### 9.2 TS bindings regen

`buf generate` against the updated [`proto/eigenius.proto`](../../proto/eigenius.proto). Regenerates [`orchestration/src/gen/eigenius_pb.ts`](../../orchestration/src/gen/eigenius_pb.ts) with the new RPC client methods and message types. Mechanical; should appear in the same PR as the proto change.

### 9.3 Chain browser helper

New helper in [`notebooks/src/runtime/chainBrowser.ts`](../../notebooks/src/runtime/chainBrowser.ts):

```ts
export type ChainBrowserClient = {
  // For the Witness editor's Browse button.
  listMergeComorphisms(branch: string): Promise<{ iri: string; label?: string }[]>;
  // For the Restructure editor's classes_under_new multi-select.
  listClasses(branch: string): Promise<{ iri: string; label?: string }[]>;
  // For inline IRI-validity checks (RenameEditor, RestructureEditor).
  resolves(branch: string, iri: string): Promise<{
    exists: boolean;
    kind?: "Class" | "Property" | "MergeComorphism" | "Other";
  }>;
};
```

Implemented on top of existing `eigen.query` and `eigen.inspect`. Cached per-session (the chain doesn't change underfoot during a resolution session except at the CAS attempt).

---

## 10. State model — the `chainStore.mergeResolution` slice

```ts
type MergeResolutionState =
  | { kind: "closed" }
  | { kind: "loading"; branch: string; candidateHead: string }
  | {
      kind: "picking";
      branch: string;
      candidateHead: string;
      conflicts: TypedConflictWire[];
      branchTip: string;
      // Per-conflict resolution; null while the user is still
      // editing (Preview button stays disabled).
      resolutions: Record</* conflict_id */ string, MergeResolutionWire | null>;
    }
  | {
      kind: "previewing";
      // Same fields as picking + an in-flight request marker.
      ...
    }
  | {
      kind: "acknowledging";
      ...
      preview: CascadeItemWire[];
      // Ack state keyed by item_id (deterministic across re-previews).
      acknowledged: Record</* item_id */ string, boolean>;
    }
  | {
      kind: "committing";
      ...
    }
  | {
      kind: "done";
      mergeLayerId: string;
      branchTip: string;
    }
  | {
      kind: "error";
      retryFrom: "loading" | "picking" | "acknowledging";
      errorKind: SubmitResolutionErrorKind;
      message: string;
      missing?: string[];  // missing acks when applicable
    };
```

State transitions are the only mutations. Each transition is an explicit reducer in [`notebooks/src/runtime/chainStore.ts`](../../notebooks/src/runtime/chainStore.ts).

The store persists `MergeResolutionState` to `localStorage` keyed on `(branch, candidate_head)` so a page reload doesn't lose the user's in-progress picks. Cleared on `done` or explicit cancel.

---

## 11. Error UX

One row per typed error variant. The error state carries `retryFrom` so the panel knows which sub-state to drop back into on the user's "Try again" click.

| `SubmitResolutionErrorKind` | UX | `retryFrom` |
|---|---|---|
| `INCOMPLETE_ACKNOWLEDGMENTS` | Banner: "N items still need acknowledgment." `missing` highlights the offending items. | `acknowledging` |
| `BRANCH_CAS_RACE` | Banner: "The branch moved while you were resolving. Reloading the conflict list." Auto-runs `prepareMerge` again. After reload, a secondary banner shows the diff: `"+1 new conflict: urn:test:Visit"`, `"-2 previously-resolved conflicts gone: ..."`, so the user understands what changed and why some strategy picks may have been dropped. | `loading` |
| `CONFLICT_NOT_FOUND` | Banner: "A conflict id became stale (most likely the branch moved). Reloading." Same handling as `BRANCH_CAS_RACE`, including the diff banner. | `loading` |
| `NO_COMMON_ANCESTOR` | Red banner: "Internal error: the two layers don't share an ancestor. This shouldn't happen from the Merge rail. Please report." Includes "Copy diagnostic" button. | (no retry) |
| `MALFORMED_RESOLUTION` | Banner: "Resolution shape is invalid: {error}." Highlights the offending strategy editor's field. Usually a UI bug. | `picking` |
| `APPLICATION_PENDING` | (kernel reserved — should never fire) Red banner with "Open issue" link. | (no retry) |
| `INTERNAL` | Red banner with kernel's `error` string and "Open issue" link. | (no retry) |

`BRANCH_CAS_RACE` is the most common non-trivial error. It happens when another commit reaches the branch between `prepareMerge` and `submitResolution`. The handling is benign: re-fetch and let the user re-pick. The previously-checked acks are preserved by `item_id`; the strategy picks are preserved by `conflict_id`. If a new conflict surfaces that wasn't there before, its strategy is empty and the user is forced to address it; if an old conflict is gone, its strategy is silently dropped.

---

## 12. Test plan

Mirrors D20 §10 row 15g and adds notebook-specific surfaces.

| Surface | Test |
|---|---|
| `PrepareMerge` handler | Returns typed conflicts identical (modulo id ordering) to the kernel-internal `classify_conflicts` result. |
| `PrepareMerge` empty case | Returns empty conflicts when branch and candidate are equal. |
| `PrepareMerge` no-LCA | Returns `NO_COMMON_ANCESTOR` for unrelated DAGs. |
| Resolution state machine | Unit tests in the notebook for each transition: loading → picking → previewing → acknowledging → committing → done. Each transition mocks the relevant RPC. |
| Per-strategy editor | Unit tests confirming `onChange` fires with the right `MergeResolutionWire` shape, and `null` when the form is incomplete. |
| Cascade ack gate | UI test: commit button disabled until every item is checked; counter updates correctly. |
| `BRANCH_CAS_RACE` recovery | Mock the kernel returning CAS_RACE; verify the state drops back to `loading` and re-fetches. |
| Empty cascade short-circuit | Witness-only resolutions skip the `acknowledging` state cleanly. |
| Playwright E2E | The D20 §10 / D34 §14 scenario: two cells commit divergent layers, one hits NEEDS_WITNESSED_MERGE, user opens Merge rail, picks KeepOne for the conflict, previews, acks, commits, verifies the cell's commit badge reflects the new merge layer id. |

The Playwright test is the schedule-bearing artifact. Until it passes, D36 isn't done.

---

## 13. Edge cases and known gaps

- **Restructure UI complexity.** §6.4 is the largest single editor. Separated into PR 3 (§14) so the rest of the resolution flow can ship first; until PR 3 lands, the picker shows Restructure with a "use the CLI for this kind of resolution" link.
- **Mass conflicts.** A merge with 47 conflicts of the same kind is unusable as N independent strategy pickers. Not solved here. The picker UI shows a count + scrolls; usability past ~20 conflicts is acknowledged as poor and tracked separately.
- **Live re-preview.** Today the cascade preview is computed on user click. A future improvement: debounced live re-preview as the user changes strategies, so consequences update in real time. Adds round-trip cost; deferred.
- **Witness signature type-check timing.** The kernel type-checks the witness term inside `submitResolution`. From the user's perspective, the failure surfaces late (post-ack-gate, during commit). A future improvement: type-check the witness at picker time so the user sees the failure before acking. Requires a new `validateWitness(comorphism_iri, conflict_class)` RPC. Deferred.
- **Inheritance cycle resolution.** Restructure works for class-shaped cycles; for property-cycle shapes the picker greys out Restructure and only offers Witness/Rename/Quotient. Manual edge-case handling lives in the conflict-kind-specific rendering in §3.1's `TypedConflictWire.kind` oneof.
- **Cell context preservation.** When the resolution flow opens from a cell-commit failure, the user can navigate freely while the flow is open. The cell's error state persists; cancelling resolution doesn't auto-clear it. The user explicitly dismisses via "Discard this commit" on the cell or commits successfully.

---

## 14. Roll-out order

Four PRs, each individually shippable.

### PR 1: Wire surface — `PrepareMerge`

- Proto: `PrepareMerge` RPC, `TypedConflictWire`, `MergeStrategyKind`, four `*Conflict` messages.
- Kernel handler: thin wrapper around `build_merge_span` + `classify_conflicts` + encoder.
- Server tests pin the encoding.
- buf generate; passthrough proxy adds.

### PR 2: Resolution flow shell + Witness + Rename + Quotient editors

- New `MergeResolutionFlow.tsx` component embedded in `MergePanel`.
- State machine + store slice.
- `WitnessEditor`, `RenameEditor`, `QuotientEditor` (the three editors with radio+input shapes; share the strategy-picker container component).
- Cascade preview + ack list (§7).
- Empty-cascade short-circuit (§7.1).
- Cell-commit failure entry point (§8.1) with auto-clear-on-success behavior, and explicit merge entry (§8.2).
- Race-recovery diff banner (§11).
- Per-conflict applicability filtering driven by `TypedConflictWire.applicable_strategies`.
- E2E tests: one per editor (Witness, Rename, KeepOne) end-to-end against fixture chains.

This is the "useful resolution UI" ship — it covers the three editor shapes that share structure (radio + IRI input variations). Witness exercises the comorphism browse; Rename exercises inline collision checking; Quotient exercises the per-kind applicability table.

### PR 3: Restructure editor

- `RestructureEditor` — the largest single piece, with structurally-different sub-UI (new-class definition builder, classes-under-new multi-select).
- Reusable IRI-list field component (extracted from existing `class_types` editor if available, else newly written).
- E2E test against the D20 §6.4 motivating example.

### PR 4: Polish + telemetry

- All seven `SubmitResolutionErrorKind` paths exercised (§11).
- Telemetry events per state transition.
- Performance pass: state-machine reducers, cascade-preview rendering for 100+ items.
- Documentation: in-app help (linked from each strategy picker) + a `docs/notebook/merge-resolution.md` user-facing walkthrough.

Total estimated effort: ~2 KLoC TS (mostly UI), ~400 LoC Rust (PrepareMerge handler + tests), ~600 LoC test code.

---

## 15. Design-decisions log

The following design points were resolved during the D36 review. Each is folded into the relevant section above; this log preserves the rationale so future readers don't have to reconstruct it from PR commits.

### 15.1 Scope: two-divergent-head reconciliation only

D36 covers the **branch-merge** case (D20's surface) — both the explicit `MergeBranches` flow and the CAS-race / implicit-divergence flow. **Within-chain commit-time conflict detection** — where a new layer being added monotonically to a chain conflicts with the chain's existing content — uses the *same* classifier algorithm but a different entry point and remediation. That work is independent and will be addressed in a separate design doc when the kernel grows the detection surface. See §1.1 / §1.2.

### 15.2 PR 2 scope: Witness + Rename + Quotient editors together

PR 2 ships three editors in one PR rather than serializing them. The three share a strategy-picker container, a cascade-preview pane, and an ack list; landing them together is cheaper than three separate PRs and produces a useful resolution UI (covering the IRI-collision / property-data-type / kind-mismatch cases — the only conflict kinds the kernel currently surfaces). Restructure is separated into PR 3 because its sub-UI is structurally different (resource builder, multi-select).

### 15.3 Race-recovery diff banner

When `BRANCH_CAS_RACE` or `CONFLICT_NOT_FOUND` drops the flow back to `loading`, the secondary banner surfaces what changed: `"+1 new conflict: urn:test:Visit"`, `"-2 previously-resolved conflicts gone"`. The store's keyed-by-`conflict_id` shape preserves prior strategy picks for conflicts that survive the race; the banner is the user-facing signal that some picks may have been dropped. Folded into §11.

### 15.4 Cell error auto-clear on successful resolution

When `committing → done` resolves a conflict that started as a cell-commit failure, the cell's error badge auto-clears (it has effectively committed — the orphan layer is now a parent of the merge layer) and is replaced for ~30 seconds by a green footer note pointing at the merge layer. After dismissal the cell looks like a normally-committed cell. Folded into §8.1.

### 15.5 KeepBoth visibility: greyed-out, never hidden

`KeepBoth` is rendered in `QuotientEditor` even when no current conflict kind admits it (none do in v1). The radio is disabled with an inline "not applicable to this conflict" explanation. Teaching the user that the strategy exists and clarifying the structural reason it doesn't apply is worth the UI surface; hiding it entirely would be mysterious and would create a discontinuity if a future taxonomy admits it. Already reflected in §6.5.

### 15.6 Resolution attribution: closed by D38

A merge layer committed via resolution has the same identity shape as a layer committed via cell execution. The wider attribution problem — *which strategy resolved each conflict, which witness was applied, which branch contributed each conflicting body* — is closed by [D38 — Merge provenance and witness discovery](d38-merge-provenance-and-witness-discovery.md): `commit_resolutions_as_merge_layer` now commits a `MergeResolutionRecord` resource per resolved conflict alongside the resolved bodies, queryable via ordinary EigenQL. The History panel's existing per-layer resource list surfaces the records automatically; a polished "resolution trace" view is D39's territory (UX, second pass).
