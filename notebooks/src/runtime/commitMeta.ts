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
import { MergeOutcome, type MergeInfo } from "@eigenius/client";

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
    mergeLayerId: merge.mergeLayerId.length > 0 ? merge.mergeLayerId : undefined,
    currentHead: merge.currentHead.length > 0 ? merge.currentHead : undefined,
    conflictingIris: merge.conflictingIris,
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
  /** Anchored-commit cache hit at a different position; branch unchanged. */
  | { kind: "cached" }
  /** Concurrent disjoint commit auto-merged; branch advanced to the merge. */
  | { kind: "trivial-merge"; mergeLayerId?: string }
  /** Conflict — branch unchanged; user must recover (Phase 5 dialog). */
  | {
    kind: "needs-witnessed-merge";
    conflictingIris: readonly string[];
    currentHead?: string;
  };

export function classifyCommit(meta: CommitMeta): CommitStatus {
  switch (meta.mergeOutcome) {
    case MergeOutcome.NEEDS_WITNESSED_MERGE:
      return {
        kind: "needs-witnessed-merge",
        conflictingIris: meta.conflictingIris,
        currentHead: meta.currentHead,
      };
    case MergeOutcome.TRIVIAL_MERGE:
      return { kind: "trivial-merge", mergeLayerId: meta.mergeLayerId };
    case MergeOutcome.UNSPECIFIED:
      // No CAS ran. In practice this is either: no persistent backend
      // (nothing to report) or a different-position anchored-commit
      // cache hit (the interesting case — show "cached").
      // `branchAdvanced` distinguishes these: false for the cache hit,
      // also false for no-backend. Both render as "cached" today; if
      // we ever want to distinguish them we'd need an extra wire field.
      return meta.branchAdvanced ? { kind: "fast-forward" } : { kind: "cached" };
    case MergeOutcome.FAST_FORWARD:
    default:
      return { kind: "fast-forward" };
  }
}
