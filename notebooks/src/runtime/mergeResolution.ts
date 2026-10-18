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
 * D36 §10 — merge-resolution state machine.
 *
 * The flow is six explicit states (`loading → picking → previewing
 * → acknowledging → committing → done|error`); each transition is
 * driven by a deliberate user action (no silent auto-advancement).
 * State lives in the Zustand store; persistence to localStorage
 * keyed on (branch, candidate_head) so a page reload doesn't lose
 * partial picks.
 */
import type {
  CascadeItemWire,
  Eigen,
  MergeResolutionWire,
  PrepareMergeErrorKind,
  PreviewCascadeErrorKind,
  SubmitResolutionErrorKind,
  TypedConflictWire,
} from "@eigenius/client";

/**
 * Discriminated union of state-machine nodes. `closed` is the
 * default; every other node carries the (branch, candidateHead)
 * pair the resolution session is anchored to.
 */
export type MergeResolutionState =
  | { kind: "closed" }
  | {
    kind: "loading";
    branch: string;
    candidateHead: string;
    /** Cell id that triggered this resolution, if entered from a
     * cell-commit failure (§8.1). Used to auto-clear the cell's
     * error badge on successful resolution. */
    cellId?: string;
  }
  | {
    kind: "picking";
    branch: string;
    candidateHead: string;
    cellId?: string;
    branchTip: string;
    conflicts: TypedConflictWire[];
    /** Per-conflict resolution chosen so far. `undefined` means
     * the user hasn't picked or the editor isn't complete yet —
     * the Preview button stays disabled until every conflict has
     * a non-undefined value. */
    resolutions: Record<string, MergeResolutionWire | undefined>;
    /** D38 §4 — extra branches the resolver should search for
     * comorphism references on `Witness` resolutions. Empty array
     * = span-only (the pre-D38 default). Names accumulate as the
     * user expands the WitnessEditor's "Search additional branches"
     * disclosure and adds entries. Threaded into the eventual
     * `previewCascade` + `submitResolution` calls. */
    witnessSearchBranches: string[];
    /** Race-recovery diff banner content (D36 §11). Populated
     * after `BRANCH_CAS_RACE` recovery; cleared on the next user
     * action. */
    raceDiff?: RaceDiff;
  }
  | {
    kind: "previewing";
    branch: string;
    candidateHead: string;
    cellId?: string;
    branchTip: string;
    conflicts: TypedConflictWire[];
    resolutions: Record<string, MergeResolutionWire>;
    witnessSearchBranches: string[];
  }
  | {
    kind: "acknowledging";
    branch: string;
    candidateHead: string;
    cellId?: string;
    branchTip: string;
    conflicts: TypedConflictWire[];
    resolutions: Record<string, MergeResolutionWire>;
    witnessSearchBranches: string[];
    preview: CascadeItemWire[];
    /** Ack state keyed by item_id (deterministic across re-previews
     * per D20 §8). */
    acknowledged: Record<string, boolean>;
  }
  | {
    kind: "committing";
    branch: string;
    candidateHead: string;
    cellId?: string;
    branchTip: string;
    conflicts: TypedConflictWire[];
    resolutions: Record<string, MergeResolutionWire>;
    witnessSearchBranches: string[];
    preview: CascadeItemWire[];
    acknowledged: Record<string, boolean>;
  }
  | {
    kind: "done";
    branch: string;
    cellId?: string;
    mergeLayerId: string;
    branchTip: string;
  }
  | {
    kind: "error";
    branch: string;
    candidateHead: string;
    cellId?: string;
    /** Where to drop back to on retry. `null` means the error is
     * unrecoverable (the panel offers "close" only). */
    retryFrom: "loading" | "picking" | "acknowledging" | null;
    message: string;
    /** Numeric `error_kind` from the underlying RPC, normalised to
     * the smallest enum that covers all three surfaces. The flow
     * driver decides handling based on this. */
    rpc: "prepare" | "preview" | "submit";
    errorKind: number;
    /** Missing cascade item ids when this is an
     * `INCOMPLETE_ACKNOWLEDGMENTS` error. */
    missingAcks?: string[];
  };

/**
 * Diff between two snapshots of the conflict list — surfaced as a
 * banner after `BRANCH_CAS_RACE` recovery so the user understands
 * why some strategy picks may have been dropped (D36 §11).
 */
export interface RaceDiff {
  added: string[];
  removed: string[];
}

/**
 * Compute the diff between the previous and freshly-fetched
 * conflict id lists. Pure function — feeds the picking state's
 * `raceDiff` field after the recovery path.
 */
export function diffConflictIds(
  before: readonly string[],
  after: readonly string[],
): RaceDiff {
  const beforeSet = new Set(before);
  const afterSet = new Set(after);
  return {
    added: after.filter((id) => !beforeSet.has(id)),
    removed: before.filter((id) => !afterSet.has(id)),
  };
}

/** Local-storage key for persisting the in-progress state. */
const STORAGE_KEY = "eigenius.mergeResolution.v1";

/**
 * Persist the resolution state to localStorage so a page reload
 * doesn't lose partial picks (D36 §10). Skips the `closed`,
 * `done`, and `error` nodes (those are terminal / transient).
 * No-ops in non-browser environments (SSR / tests).
 */
export function persistMergeResolution(state: MergeResolutionState): void {
  if (typeof window === "undefined") return;
  try {
    if (
      state.kind === "closed" ||
      state.kind === "done" ||
      state.kind === "error"
    ) {
      window.localStorage.removeItem(STORAGE_KEY);
      return;
    }
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {
    // localStorage may be disabled / full — silent fail is fine;
    // the worst case is a page reload loses in-progress picks.
  }
}

/**
 * Restore the resolution state from localStorage. Returns `closed`
 * if nothing is stored or the stored shape is unparseable.
 */
export function restoreMergeResolution(): MergeResolutionState {
  if (typeof window === "undefined") return { kind: "closed" };
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (raw === null) return { kind: "closed" };
    const parsed = JSON.parse(raw) as MergeResolutionState;
    // Defensive: validate the shape minimally so corrupt entries
    // don't crash the panel mount.
    if (
      typeof parsed === "object" && parsed !== null &&
      typeof (parsed as { kind?: string }).kind === "string"
    ) {
      return parsed;
    }
  } catch {
    // Fall through to closed.
  }
  return { kind: "closed" };
}

/**
 * Type-narrowed accessors used across the flow driver and editor
 * components. Cleaner than `state.kind === "picking" ? state : ...`
 * peppered through every render.
 */
export function isResolvable(state: MergeResolutionState): boolean {
  return state.kind === "picking" &&
    state.conflicts.every((c) => state.resolutions[c.id] !== undefined);
}

export function allAcked(state: MergeResolutionState): boolean {
  if (state.kind !== "acknowledging") return false;
  return state.preview.every((item) => state.acknowledged[item.itemId]);
}

export function ackCount(state: MergeResolutionState): {
  acked: number;
  total: number;
} {
  if (state.kind !== "acknowledging") return { acked: 0, total: 0 };
  const total = state.preview.length;
  const acked = state.preview.filter((item) =>
    state.acknowledged[item.itemId]
  ).length;
  return { acked, total };
}

/**
 * RPC actions. Imported by `notebookStore` so the store's resolution
 * actions (openResolution / previewCascade / commit) can call them
 * without re-exporting the SDK shape everywhere.
 */
export interface ResolutionRpcs {
  prepareMerge: typeof Eigen.prototype.prepareMerge;
  previewCascade: typeof Eigen.prototype.previewCascade;
  submitResolution: typeof Eigen.prototype.submitResolution;
}

/**
 * Emit a telemetry event per state-machine transition. Currently a
 * `console.debug` so the events are visible in browser devtools and
 * captured by any console-intercepting telemetry pipeline; future
 * surfaces (D34 telemetry, a Sentry-style sink) can substitute a
 * real emitter without changing callers.
 *
 * Props are kept small and side-effect-free — branch + short
 * candidate-head hash + state kind + strategy/conflict counts. No
 * PII, no full IRIs (which can be sensitive in private chains).
 */
export function emitResolutionTelemetry(
  event: string,
  state: MergeResolutionState,
  extra?: Record<string, unknown>,
): void {
  // Cheap props: pull only the high-level shape without copying
  // the full conflict list or resolutions map.
  const props: Record<string, unknown> = { state: state.kind };
  if (
    state.kind === "loading" ||
    state.kind === "picking" ||
    state.kind === "previewing" ||
    state.kind === "acknowledging" ||
    state.kind === "committing" ||
    state.kind === "error"
  ) {
    props.branch = state.branch;
    props.candidateHeadShort = state.candidateHead.slice(0, 12);
  }
  if (state.kind === "picking" || state.kind === "previewing") {
    props.conflictCount = state.conflicts.length;
  }
  if (state.kind === "acknowledging" || state.kind === "committing") {
    props.cascadeItemCount = state.preview.length;
  }
  if (state.kind === "done") {
    props.branch = state.branch;
    props.mergeLayerShort = state.mergeLayerId.slice(0, 12);
  }
  if (state.kind === "error") {
    props.errorKind = state.errorKind;
    props.rpc = state.rpc;
  }
  if (extra) Object.assign(props, extra);
  console.debug(`[merge-resolution] ${event}`, props);
}

/**
 * Map the kernel's error kinds to the flow driver's discriminator.
 * Each subset of the three error-kind enums (Prepare / Preview /
 * Submit) is normalised to a single error node carrying the
 * original numeric value + the rpc that produced it so the UI can
 * render kind-specific copy.
 */
export type MergeResolutionErrorKind =
  | PrepareMergeErrorKind
  | PreviewCascadeErrorKind
  | SubmitResolutionErrorKind;
