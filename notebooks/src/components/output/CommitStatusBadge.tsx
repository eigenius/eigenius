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
 * Renders the cell footer indicator for D34 §6 commit outcomes:
 *
 * - **Fast-forward**: clean append. No badge — the layer summary the
 *   surrounding `CellOutputView` already shows is enough.
 * - **Cached**: anchored-commit cache hit at a different position; the
 *   branch ref did not advance. Renders as `◐ cached` with a hover
 *   tooltip explaining the situation.
 * - **Trivial merge**: concurrent disjoint commit was auto-merged; the
 *   branch ref advanced to the merge layer (not the cell's freshly-
 *   built layer). Renders as `◆ merged` with the merge layer's id.
 * - **Needs witnessed merge**: concurrent conflicting commit; the
 *   branch ref did not advance and the cell's layer is on disk but
 *   unreachable from any branch. Renders inline as an error
 *   `MessageBar` listing the conflicting IRIs. Recovery dialog (save
 *   as sibling / rebase / discard) lands in Phase 5.
 */
import {
  Badge,
  Button,
  Caption1,
  makeStyles,
  MessageBar,
  MessageBarActions,
  MessageBarBody,
  MessageBarTitle,
  tokens,
  Tooltip,
} from "@fluentui/react-components";
import { useEigen } from "../../runtime/EigenProvider";
import { classifyCommit, type CommitMeta } from "../../runtime/commitMeta";
import { useNotebookStore } from "../../runtime/notebookStore";

const useStyles = makeStyles({
  badge: {
    marginTop: tokens.spacingVerticalXXS,
    display: "inline-flex",
    gap: tokens.spacingHorizontalXS,
    alignItems: "center",
  },
  conflictList: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    margin: `${tokens.spacingVerticalXS} 0 0 0`,
    padding: `0 0 0 ${tokens.spacingHorizontalM}`,
  },
});

export interface CommitStatusBadgeProps {
  commit: CommitMeta;
  /** Cell that produced this commit. Threaded into the resolution
   * flow so a successful resolve can auto-clear this cell's error
   * badge (D36 §8.1). Optional for backwards compat with callers
   * that don't have a cell context (e.g., program-run cells where
   * the trace-layer commit has no associated source cell). */
  cellId?: string;
}

export function CommitStatusBadge({ commit, cellId }: CommitStatusBadgeProps) {
  const styles = useStyles();
  const eigen = useEigen();
  const activeBranch = useNotebookStore((s) => s.activeBranch);
  const openResolution = useNotebookStore((s) => s.openResolution);
  const status = classifyCommit(commit);

  switch (status.kind) {
    case "fast-forward":
      // Default case — the cell's "layer = X" summary already conveys
      // the successful commit. Skip the badge to keep cells uncluttered.
      return null;

    case "cached":
      return (
        <Tooltip
          relationship="description"
          content={status.cachedLayerId
            ? `Content already canonical at layer ${status.cachedLayerId}; branch ref did not move.`
            : "This content is already canonical at the layer shown above; the branch ref did not move."}
          withArrow
        >
          <Badge
            appearance="tint"
            color="informative"
            size="small"
            className={styles.badge}
          >
            ◐ cached
          </Badge>
        </Tooltip>
      );

    case "trivial-merge":
      return (
        <Tooltip
          relationship="description"
          content={status.mergeLayerId
            ? `Concurrent disjoint work merged automatically. Merge layer: ${status.mergeLayerId}`
            : "Concurrent disjoint work merged automatically."}
          withArrow
        >
          <Badge
            appearance="tint"
            color="success"
            size="small"
            className={styles.badge}
          >
            ◆ merged
          </Badge>
        </Tooltip>
      );

    case "needs-witnessed-merge":
      return (
        <>
          <MessageBar intent="error" className={styles.badge}>
            <MessageBarBody>
              <MessageBarTitle>
                Conflict — branch did not advance
              </MessageBarTitle>
              <Caption1>
                Another commit reached this branch between when you read its
                head and when you saved. The two commits modify the same
                resources and cannot be merged automatically (Phase 15
                witnessed-merge resolution is not yet available).
              </Caption1>
              {status.conflictingIris.length > 0 && (
                <ul className={styles.conflictList}>
                  {status.conflictingIris.map((iri) => (
                    <li key={iri}>{iri}</li>
                  ))}
                </ul>
              )}
              {status.currentHead && (
                <Caption1>
                  Branch's current head: <code>{status.currentHead}</code>
                </Caption1>
              )}
            </MessageBarBody>
            {/* D36 §8.1 — route the user into the rail Merge
              panel's resolution flow. The orphan layer becomes the
              candidate head; the resolution flow's `done` state
              auto-clears this cell's error badge via the cellId
              threaded into the resolution session. */}
            {status.orphanLayerId && (
              <MessageBarActions>
                <Button
                  size="small"
                  appearance="primary"
                  onClick={() => {
                    void openResolution(eigen, {
                      branch: activeBranch,
                      candidateHead: status.orphanLayerId!,
                      cellId,
                    });
                  }}
                >
                  Resolve in Merge rail
                </Button>
              </MessageBarActions>
            )}
          </MessageBar>
        </>
      );
  }
}
