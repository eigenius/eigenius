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
 * Tags rail destination — D34 §8.2.
 *
 * Mirrors `BranchesPanel` but is simpler: tags are immutable named
 * refs, so there's no "switch to" action. Columns: Tag, Layer,
 * Tagged at, Actions (View in history, Delete).
 *
 * Creation surfaces:
 * - Header `+ Create tag…` button → `CreateTagDialog` without a
 *   pre-filled layer (the dialog falls back to a branch-tip picker).
 * - The History panel's per-layer detail panel adds a sibling
 *   "Create tag" button that opens the same dialog with the
 *   selected layer pre-filled.
 *
 * "View in history" navigates to the History destination and sets
 * the session read-pin to the tag's target. The user can clear the
 * pin via the workspace header's "Return to tip" affordance.
 */

import { useEffect, useState } from "react";
import {
  Body1,
  Button,
  Caption1,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  makeStyles,
  MessageBar,
  MessageBarBody,
  Spinner,
  Subtitle1,
  Toast,
  ToastBody,
  Toaster,
  ToastTitle,
  tokens,
  Tooltip,
  useId,
  useToastController,
} from "@fluentui/react-components";
import {
  Add16Regular,
  BranchFork16Regular,
  Delete16Regular,
  History16Regular,
  Tag20Regular,
} from "@fluentui/react-icons";
import type { TagInfo } from "@eigenius/client";
import { useEigen } from "../../runtime/EigenProvider";
import { useNotebookStore } from "../../runtime/notebookStore";
import { formatAbsoluteIso, formatRelative } from "../../runtime/relativeTime";
import { CreateBranchDialog } from "../dialogs/CreateBranchDialog";
import { CreateTagDialog } from "../dialogs/CreateTagDialog";

const TOASTER_ID = "tags-panel-toaster";

const useStyles = makeStyles({
  root: {
    display: "flex",
    flexDirection: "column",
    height: "100%",
    minHeight: 0,
  },
  header: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalM,
    padding: `${tokens.spacingVerticalM} ${tokens.spacingHorizontalXXL}`,
    borderBottom: `1px solid ${tokens.colorNeutralStroke2}`,
  },
  headerSpacer: { flex: 1 },
  body: {
    flex: 1,
    minHeight: 0,
    overflowY: "auto",
    padding: tokens.spacingVerticalM,
  },
  bodyInner: {
    maxWidth: "960px",
    margin: "0 auto",
  },
  table: {
    width: "100%",
    borderCollapse: "collapse",
    fontSize: tokens.fontSizeBase300,
  },
  th: {
    textAlign: "left",
    color: tokens.colorNeutralForeground3,
    fontWeight: tokens.fontWeightSemibold,
    fontSize: tokens.fontSizeBase200,
    textTransform: "uppercase",
    letterSpacing: "0.04em",
    padding: `${tokens.spacingVerticalS} ${tokens.spacingHorizontalM}`,
    borderBottom: `1px solid ${tokens.colorNeutralStroke2}`,
  },
  td: {
    padding: `${tokens.spacingVerticalS} ${tokens.spacingHorizontalM}`,
    borderBottom: `1px solid ${tokens.colorNeutralStroke3}`,
    verticalAlign: "middle",
  },
  tagName: {
    fontWeight: tokens.fontWeightSemibold,
  },
  hash: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground3,
  },
  actions: {
    display: "flex",
    gap: tokens.spacingHorizontalXS,
    flexWrap: "wrap",
  },
  emptyState: {
    padding: tokens.spacingVerticalXXL,
    textAlign: "center",
    color: tokens.colorNeutralForeground3,
  },
  loadingState: {
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    gap: tokens.spacingHorizontalS,
    padding: tokens.spacingVerticalXXL,
    color: tokens.colorNeutralForeground3,
  },
});

export function TagsPanel() {
  const styles = useStyles();
  const eigen = useEigen();
  const toasterId = useId("toaster", TOASTER_ID);
  const { dispatchToast } = useToastController(toasterId);

  const setDestination = useNotebookStore((s) => s.setDestination);
  const setReadPin = useNotebookStore((s) => s.setReadPin);

  const [tags, setTags] = useState<readonly TagInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [createOpen, setCreateOpen] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<TagInfo | null>(null);
  // Tag we're forking a branch off of. `null` = dialog closed. We
  // carry the tag *name* (not its layer id) so the dialog pre-selects
  // the tag in the "Tag" picker — semantically the same layer but a
  // more human-readable handle than the hex.
  const [createBranchFor, setCreateBranchFor] = useState<string | null>(null);

  const refresh = async () => {
    try {
      const list = await eigen.listTags();
      setTags(list);
      setError(null);
    } catch (err) {
      // The kernel rejects ListTags on in-memory mode with
      // `failed_precondition`; surface that as a friendly inline
      // hint rather than an error toast.
      const message = err instanceof Error ? err.message : String(err);
      setTags([]);
      setError(message);
    }
  };

  useEffect(() => {
    void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [eigen]);

  const onViewInHistory = (t: TagInfo) => {
    setReadPin(t.layerId);
    setDestination("history");
  };

  const onDelete = async (t: TagInfo) => {
    try {
      const resp = await eigen.deleteTag(t.name);
      if (!resp.success) {
        dispatchToast(
          <Toast>
            <ToastTitle>Couldn't delete tag</ToastTitle>
            <ToastBody>{resp.error || "kernel refused delete"}</ToastBody>
          </Toast>,
          { intent: "error", timeout: 6000 },
        );
        return;
      }
      dispatchToast(
        <Toast>
          <ToastTitle>Deleted tag {t.name}</ToastTitle>
          <ToastBody>
            {resp.deleted
              ? "The target layer becomes GC-eligible if no other root reaches it."
              : "Tag was already gone (idempotent delete)."}
          </ToastBody>
        </Toast>,
        { intent: "info", timeout: 4000 },
      );
      await refresh();
    } catch (err) {
      dispatchToast(
        <Toast>
          <ToastTitle>Delete failed</ToastTitle>
          <ToastBody>
            {err instanceof Error ? err.message : String(err)}
          </ToastBody>
        </Toast>,
        { intent: "error", timeout: 6000 },
      );
    } finally {
      setDeleteTarget(null);
    }
  };

  return (
    <div className={styles.root}>
      <div className={styles.header}>
        <Tag20Regular />
        <Subtitle1 as="h2">Tags</Subtitle1>
        <span className={styles.headerSpacer} />
        <Button
          appearance="primary"
          icon={<Add16Regular />}
          onClick={() => setCreateOpen(true)}
        >
          Create tag…
        </Button>
      </div>
      <div className={styles.body}>
        <div className={styles.bodyInner}>
          {error && (
            <MessageBar intent="warning">
              <MessageBarBody>{error}</MessageBarBody>
            </MessageBar>
          )}
          {tags === null && !error && (
            <div className={styles.loadingState}>
              <Spinner size="tiny" />
              <Caption1>fetching tags…</Caption1>
            </div>
          )}
          {tags !== null && tags.length === 0 && !error && (
            <div className={styles.emptyState}>
              No tags yet. Use <strong>Create tag…</strong>{" "}
              to pin a layer and protect it from garbage collection.
            </div>
          )}
          {tags !== null && tags.length > 0 && (
            <TagsTable
              tags={tags}
              styles={styles}
              onViewInHistory={onViewInHistory}
              onCreateBranch={(t) => setCreateBranchFor(t.name)}
              onDelete={(t) => setDeleteTarget(t)}
            />
          )}
        </div>
      </div>

      <CreateTagDialog
        open={createOpen}
        onClose={() => setCreateOpen(false)}
        onCreated={(name) => {
          dispatchToast(
            <Toast>
              <ToastTitle>Created tag {name}</ToastTitle>
            </Toast>,
            { intent: "success", timeout: 4000 },
          );
          void refresh();
        }}
      />

      <DeleteTagConfirmDialog
        target={deleteTarget}
        onCancel={() => setDeleteTarget(null)}
        onConfirm={(t) => void onDelete(t)}
      />

      <CreateBranchDialog
        open={createBranchFor !== null}
        onClose={() => setCreateBranchFor(null)}
        defaultTagName={createBranchFor ?? undefined}
        onCreated={(name) =>
          dispatchToast(
            <Toast>
              <ToastTitle>Created branch {name}</ToastTitle>
            </Toast>,
            { intent: "success", timeout: 4000 },
          )}
      />

      <Toaster toasterId={toasterId} position="top-end" />
    </div>
  );
}

interface TagsTableProps {
  tags: readonly TagInfo[];
  styles: ReturnType<typeof useStyles>;
  onViewInHistory: (t: TagInfo) => void;
  onCreateBranch: (t: TagInfo) => void;
  onDelete: (t: TagInfo) => void;
}

function TagsTable({
  tags,
  styles,
  onViewInHistory,
  onCreateBranch,
  onDelete,
}: TagsTableProps) {
  return (
    <table className={styles.table}>
      <thead>
        <tr>
          <th className={styles.th}>Tag</th>
          <th className={styles.th}>Layer</th>
          <th className={styles.th}>Tagged at</th>
          <th className={styles.th}>Actions</th>
        </tr>
      </thead>
      <tbody>
        {tags.map((t) => {
          const ms = Number(t.taggedAtMs);
          return (
            <tr key={t.name}>
              <td className={styles.td}>
                <span className={styles.tagName}>{t.name}</span>
              </td>
              <td className={styles.td}>
                <Tooltip content={t.layerId} relationship="description">
                  <span className={styles.hash}>{shortHash(t.layerId)}</span>
                </Tooltip>
              </td>
              <td className={styles.td}>
                {ms > 0
                  ? (
                    <Tooltip
                      content={formatAbsoluteIso(ms) || ""}
                      relationship="description"
                    >
                      <span>{formatRelative(ms) || "—"}</span>
                    </Tooltip>
                  )
                  : <span className={styles.hash}>—</span>}
              </td>
              <td className={styles.td}>
                <div className={styles.actions}>
                  <Tooltip
                    content="Open the History panel pinned to this layer."
                    relationship="description"
                  >
                    <Button
                      size="small"
                      appearance="subtle"
                      icon={<History16Regular />}
                      onClick={() => onViewInHistory(t)}
                    >
                      View in history
                    </Button>
                  </Tooltip>
                  <Tooltip
                    content="Fork a new branch starting at this tag's layer."
                    relationship="description"
                  >
                    <Button
                      size="small"
                      appearance="subtle"
                      icon={<BranchFork16Regular />}
                      onClick={() => onCreateBranch(t)}
                    >
                      Create branch…
                    </Button>
                  </Tooltip>
                  <Tooltip
                    content="Remove this tag. The target layer becomes GC-eligible if nothing else reaches it."
                    relationship="description"
                  >
                    <Button
                      size="small"
                      appearance="subtle"
                      icon={<Delete16Regular />}
                      onClick={() => onDelete(t)}
                    >
                      Delete
                    </Button>
                  </Tooltip>
                </div>
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

interface DeleteTagConfirmDialogProps {
  target: TagInfo | null;
  onCancel: () => void;
  onConfirm: (t: TagInfo) => void;
}

function DeleteTagConfirmDialog({
  target,
  onCancel,
  onConfirm,
}: DeleteTagConfirmDialogProps) {
  return (
    <Dialog
      open={target !== null}
      onOpenChange={(_e, data) => {
        if (!data.open) onCancel();
      }}
    >
      <DialogSurface>
        <DialogBody>
          <DialogTitle>Delete tag {target?.name}?</DialogTitle>
          <DialogContent>
            <Body1>
              The tag will be removed. The layer it points at becomes eligible
              for garbage collection if no other root (branch, active task,
              another tag) still reaches it.
            </Body1>
          </DialogContent>
          <DialogActions>
            <Button appearance="secondary" onClick={onCancel}>
              Cancel
            </Button>
            <Button
              appearance="primary"
              onClick={() => target && onConfirm(target)}
            >
              Delete
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}

function shortHash(hex: string): string {
  if (hex.length <= 10) return hex;
  return `${hex.slice(0, 4)}…${hex.slice(-4)}`;
}
