// Copyright 2026 The Eigenius Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

/**
 * Per-cell capture of the kernel's branch-CAS outcome (D23 §5.4 / D34 §6).
 *
 * Every commit-producing RPC (`Load`, `RunProgram`, `Reflect`, `Query`
 * with FIBER INTO) carries `branchAdvanced` and a `MergeInfo` on its
 * response. The notebook's `CellOutput` variants stash a normalised
 * version of those two fields here so the renderer doesn't have to
 * juggle multiple wire shapes — every commit looks the same to the UI.
 *
 * Normalisation:
 *
 * - The wire `merge` field is `MergeInfo | undefined`. Treating it as
 *   `undefined` is indistinguishable from `outcome = UNSPECIFIED` (the
 *   proto3 zero value), so we collapse both to `mergeOutcome =
 *   MergeOutcome.UNSPECIFIED`.
 * - The wire's empty-string defaults for `mergeLayerId` / `currentHead`
 *   become `undefined` so the renderer can use `?.` short-circuiting.
 * - `conflictingIris` defaults to an empty readonly array.
 */
import { type MergeInfo, MergeOutcome } from "@eigenius/client";

export interface CommitMeta {
  /** Did the branch ref actually move? */
  readonly branchAdvanced: boolean;
  /** Outcome of the CAS; `UNSPECIFIED` means no CAS ran. */
  readonly mergeOutcome: MergeOutcome;
  /** Set when `mergeOutcome === TRIVIAL_MERGE`. */
  readonly mergeLayerId?: string;
  /** Set when `mergeOutcome === NEEDS_WITNESSED_MERGE`. */
  readonly currentHead?: string;
  /** Non-empty when `mergeOutcome === NEEDS_WITNESSED_MERGE`. */
  readonly conflictingIris: readonly string[];
  /**
   * Set when `mergeOutcome === NEEDS_WITNESSED_MERGE`: hex-encoded
   * id of the orphan layer the caller built. Fed into the D36
   * resolution flow as the `candidateHead` so the user can pick
   * per-conflict resolution strategies and commit a merge layer.
   */
  readonly orphanLayerId?: string;
}

/** Build a `CommitMeta` from any commit-producing response. */
export function commitMetaFrom(
  response: { branchAdvanced: boolean; merge?: MergeInfo | undefined },
): CommitMeta {
  const merge = response.merge;
  if (merge === undefined) {
    return {
      branchAdvanced: response.branchAdvanced,
      mergeOutcome: MergeOutcome.UNSPECIFIED,
      conflictingIris: [],
    };
  }
  return {
    branchAdvanced: response.branchAdvanced,
    mergeOutcome: merge.outcome,
    mergeLayerId: merge.mergeLayerId.length > 0
      ? merge.mergeLayerId
      : undefined,
    currentHead: merge.currentHead.length > 0 ? merge.currentHead : undefined,
    conflictingIris: merge.conflictingIris,
    orphanLayerId: merge.orphanLayerId.length > 0
      ? merge.orphanLayerId
      : undefined,
  };
}

/**
 * UI-facing classification of a commit. Drives the cell-footer badge
 * (D34 §6.1) and the toast trigger in `MergeEventToaster`. Folds
 * `branchAdvanced` + `mergeOutcome` into one of five mutually-exclusive
 * states the renderer can `switch` over.
 */
export type CommitStatus =
  /** Default success — clean append, no surprise. No badge. */
  | { kind: "fast-forward" }
  /**
   * Anchored-commit cache hit at a different chain position; branch
   * unchanged. `cachedLayerId` carries the canonical layer's id so the
   * tooltip can point at it.
   */
  | { kind: "cached"; cachedLayerId?: string }
  /** Concurrent disjoint commit auto-merged; branch advanced to the merge. */
  | { kind: "trivial-merge"; mergeLayerId?: string }
  /** Conflict — branch unchanged; user must recover (D34 §6.2 dialog). */
  | {
    kind: "needs-witnessed-merge";
    conflictingIris: readonly string[];
    currentHead?: string;
    orphanLayerId?: string;
  };

export function classifyCommit(meta: CommitMeta): CommitStatus {
  switch (meta.mergeOutcome) {
    case MergeOutcome.NEEDS_WITNESSED_MERGE:
      return {
        kind: "needs-witnessed-merge",
        conflictingIris: meta.conflictingIris,
        currentHead: meta.currentHead,
        orphanLayerId: meta.orphanLayerId,
      };
    case MergeOutcome.TRIVIAL_MERGE:
      return { kind: "trivial-merge", mergeLayerId: meta.mergeLayerId };
    case MergeOutcome.CACHED_DIFFERENT_POSITION:
      // Anchored-commit cache hit at a different chain position
      // (D33 §6). The branch ref did not advance; the cached
      // canonical layer's id is in `mergeLayerId` for the tooltip.
      return { kind: "cached", cachedLayerId: meta.mergeLayerId };
    case MergeOutcome.UNSPECIFIED:
      // No CAS happened — no persistent backend, or the commit
      // didn't reach the persist step (eval errored, validation
      // rejected the resources). No badge: we have nothing
      // commit-shape-related to surface to the user.
      return { kind: "fast-forward" };
    case MergeOutcome.FAST_FORWARD:
    default:
      return { kind: "fast-forward" };
  }
}
