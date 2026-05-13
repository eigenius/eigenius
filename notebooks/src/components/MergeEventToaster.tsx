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
 * Subscribes to the notebook's cellOutputs and fires a Fluent UI toast
 * whenever a cell's commit outcome is `TRIVIAL_MERGE` (D34 §6.1).
 *
 * **What this surfaces.** A concurrent commit reached the same branch
 * between when the cell read the head and when it saved. The two
 * commits touched disjoint IRI sets, so the kernel auto-merged them
 * via D23 §5.4.3 — `branch:main` now points at a multi-parent merge
 * layer, not the cell's freshly-built layer. The cell footer's
 * `◆ merged` badge is easy to miss when the cell has scrolled
 * out of view; the toast pulls the event to the top of the UI.
 *
 * **Why a separate component, not the store.** Zustand stores live
 * outside the React tree, so dispatching toasts from store actions
 * would mean passing the toast controller through stable refs. This
 * component lives inside the FluentProvider, owns the `useToastController`
 * call, and uses cellOutputs as the truth — every new `TRIVIAL_MERGE`
 * output gets one toast, deduplicated by cell-id + merge-layer-id.
 */

import { useEffect, useRef } from "react";
import {
  Link,
  Toast,
  ToastBody,
  ToastTitle,
  useToastController,
} from "@fluentui/react-components";
import { MergeOutcome } from "@eigenius/client";
import { useNotebookStore } from "../runtime/notebookStore";

const TOASTER_ID = "merge-event-toaster";

export { TOASTER_ID };

/**
 * Mount once inside `<FluentProvider>` (peer-level to `<Notebook>`).
 * Owns no DOM of its own — the actual `<Toaster>` is mounted alongside
 * via `App.tsx` so the toast portal can render at the root.
 */
export function MergeEventToaster() {
  const { dispatchToast } = useToastController(TOASTER_ID);
  const cellOutputs = useNotebookStore((s) => s.cellOutputs);

  // Track which (cellId, mergeLayerId) pairs we've already toasted on
  // so re-renders don't re-fire the same notification. A re-execution
  // of the same cell that produces a different merge-layer id (e.g.
  // a fresh trivial merge against new concurrent work) does fire again.
  const seen = useRef<Set<string>>(new Set());

  useEffect(() => {
    for (const [cellId, output] of cellOutputs) {
      const commit = extractCommitMeta(output);
      if (commit === undefined) continue;
      if (commit.mergeOutcome !== MergeOutcome.TRIVIAL_MERGE) continue;

      const fingerprint = `${cellId}:${commit.mergeLayerId ?? ""}`;
      if (seen.current.has(fingerprint)) continue;
      seen.current.add(fingerprint);

      const mergeLayer = commit.mergeLayerId;
      dispatchToast(
        <Toast>
          <ToastTitle>Merged with concurrent work</ToastTitle>
          <ToastBody>
            Another commit reached this branch between when you read its head
            and when you saved. Your changes were merged automatically (no
            conflicts).
            {mergeLayer && (
              <div>
                Merge layer:{" "}
                <Link
                  onClick={() => {
                    // Phase 5 will jump to the merge layer in the
                    // History panel. For Phase 1c the toast is purely
                    // informational — copy the id so users can paste
                    // it into Inspect.
                    void navigator.clipboard?.writeText(mergeLayer);
                  }}
                >
                  <code>{mergeLayer.slice(0, 8)}…</code>
                </Link>
              </div>
            )}
          </ToastBody>
        </Toast>,
        { intent: "success", timeout: 8000 },
      );
    }
  }, [cellOutputs, dispatchToast]);

  return null;
}

/**
 * Pull the `CommitMeta` (if any) out of a CellOutput. Each commit-
 * producing kind stores it differently:
 *
 * - `load`, `resultset`, and `resource` each carry an optional
 *   `output.commit`. `load` is undefined for validate-only loads
 *   (no commit attempted); the other two are undefined for reads
 *   that didn't trigger a chain commit.
 * - `program-run` produces one per result; we surface the *first*
 *   trivial-merge in the result list (rare to have multiple in one
 *   batch; if it happens, only the first toasts — the rest still get
 *   their per-result `◆ merged` badge via `CommitStatusBadge`).
 *
 * Other kinds (`error`, `value`, `validate`, `markdown`) never commit.
 */
function extractCommitMeta(
  output:
    | import("../runtime/notebookStore").CellOutput
    | undefined,
): import("../runtime/commitMeta").CommitMeta | undefined {
  if (output === undefined) return undefined;
  switch (output.kind) {
    case "load":
      return output.commit;
    case "resultset":
    case "resource":
      return output.commit;
    case "program-run":
      for (const r of output.results) {
        if (r.commit?.mergeOutcome === MergeOutcome.TRIVIAL_MERGE) {
          return r.commit;
        }
      }
      return undefined;
    default:
      return undefined;
  }
}
