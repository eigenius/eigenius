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
 * Branches rail destination — D34 §4.2.
 *
 * Table of every branch with the workspace's actions:
 *
 *  Branch | Tip | Actions
 *
 * Phase 3 wires **Switch** and **Delete**. **View history**,
 * **Compact**, and **Merge into…** render disabled with "coming
 * soon" tooltips — those destinations land in Phases 4 / 6 / 5.
 *
 * "Resources" and "Last commit" columns from the spec table are
 * intentionally omitted — `GetBranch` returns only `name + head_layer`
 * today, so showing them would require per-branch `LayerTopology`
 * fan-out for a value that's noise on a chain with many branches.
 * Add them when `GetBranch` grows `resource_count` / `head_committed_at`
 * (separate kernel change, queued alongside the picker timestamps).
 */

import { useEffect, useState } from "react";
import {
  Body1,
  Button,
  Caption1,
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
  Add20Regular,
  Branch20Regular,
  Delete16Regular,
  History16Regular,
  Merge16Regular,
  Stack16Regular,
} from "@fluentui/react-icons";
import type { BranchInfo } from "@eigenius/client";
import { useEigen } from "../../runtime/EigenProvider";
import { useNotebookStore } from "../../runtime/notebookStore";
import { formatAbsoluteIso, formatRelative } from "../../runtime/relativeTime";
import { CreateBranchDialog } from "../dialogs/CreateBranchDialog";
import { DeleteBranchDialog } from "../dialogs/DeleteBranchDialog";

const AUTO_BRANCH_RE = /^auto-\d{4}-\d{2}-\d{2}/;

const TOASTER_ID = "branches-panel-toaster";

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
  nameCell: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalS,
  },
  branchName: {
    fontWeight: tokens.fontWeightSemibold,
  },
  branchNameAuto: {
    fontWeight: tokens.fontWeightRegular,
    color: tokens.colorNeutralForeground3,
  },
  activeBadge: {
    color: tokens.colorPaletteGreenForeground1,
    fontSize: tokens.fontSizeBase200,
  },
  tip: {
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

export function BranchesPanel() {
  const styles = useStyles();
  const eigen = useEigen();
  const toasterId = useId("toaster", TOASTER_ID);
  const { dispatchToast } = useToastController(toasterId);

  const activeBranch = useNotebookStore((s) => s.activeBranch);
  const branches = useNotebookStore((s) => s.branches);
  const refreshBranches = useNotebookStore((s) => s.refreshBranches);
  const switchBranch = useNotebookStore((s) => s.switchBranch);
  const setDestination = useNotebookStore((s) => s.setDestination);
  const setPendingMergeSource = useNotebookStore(
    (s) => s.setPendingMergeSource,
  );

  const [loadError, setLoadError] = useState<string | null>(null);
  const [createOpen, setCreateOpen] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<BranchInfo | null>(null);

  // Refresh on mount and whenever the active branch changes — the
  // active row's badge updates after a switch.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      setLoadError(null);
      const result = await refreshBranches(eigen);
      if (cancelled) return;
      if (result === null) {
        setLoadError(
          "Branch operations require a persistent backend; this kernel is running in-memory mode.",
        );
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [eigen, refreshBranches, activeBranch]);

  const onSwitch = (b: BranchInfo) => {
    if (b.name === activeBranch) return;
    switchBranch(eigen, b.name);
    dispatchToast(
      <Toast>
        <ToastTitle>Switched to {b.name}</ToastTitle>
        <ToastBody>
          Cell outputs cleared. Click Run All in the Notebook to populate.
        </ToastBody>
      </Toast>,
      { intent: "info", timeout: 6000 },
    );
  };

  // "View history" jumps to the History rail destination. The
  // History panel is branch-scoped (D34 §5), so we switch the active
  // branch first if the row is for a different one — that keeps the
  // workspace's read/write context consistent with the destination.
  const onViewHistory = (b: BranchInfo) => {
    if (b.name !== activeBranch) {
      switchBranch(eigen, b.name);
    }
    setDestination("history");
  };

  // "Merge into…" jumps to the Merge destination with the row's
  // branch pre-filled as the source. The Merge panel consumes the
  // hint on mount and clears it — a later visit defaults to the
  // active branch instead of a stale source.
  const onMergeFrom = (b: BranchInfo) => {
    setPendingMergeSource(b.name);
    setDestination("merge");
  };

  return (
    <div className={styles.root}>
      <div className={styles.header}>
        <Branch20Regular />
        <Subtitle1 as="h2">Branches</Subtitle1>
        <div className={styles.headerSpacer} />
        <Button
          appearance="primary"
          icon={<Add20Regular />}
          onClick={() => setCreateOpen(true)}
        >
          New branch
        </Button>
      </div>

      <div className={styles.body}>
        <div className={styles.bodyInner}>
          {loadError && (
            <MessageBar intent="info">
              <MessageBarBody>{loadError}</MessageBarBody>
            </MessageBar>
          )}

          {branches === null && !loadError && (
            <div className={styles.loadingState}>
              <Spinner size="tiny" /> <Caption1>loading branches…</Caption1>
            </div>
          )}

          {branches !== null && branches.length === 0 && (
            <div className={styles.emptyState}>
              <Body1>No branches yet.</Body1>
            </div>
          )}

          {branches !== null && branches.length > 0 && (
            <BranchesTable
              branches={branches}
              activeBranch={activeBranch}
              styles={styles}
              onSwitch={onSwitch}
              onViewHistory={onViewHistory}
              onMergeFrom={onMergeFrom}
              onDelete={(b) => setDeleteTarget(b)}
            />
          )}
        </div>
      </div>

      <CreateBranchDialog
        open={createOpen}
        onClose={() => setCreateOpen(false)}
        onCreated={(name) => {
          dispatchToast(
            <Toast>
              <ToastTitle>Created branch {name}</ToastTitle>
            </Toast>,
            { intent: "success", timeout: 4000 },
          );
        }}
      />

      {deleteTarget && (
        <DeleteBranchDialog
          target={deleteTarget}
          open={deleteTarget !== null}
          onClose={() => setDeleteTarget(null)}
          onDeleted={(name) => {
            dispatchToast(
              <Toast>
                <ToastTitle>Deleted branch {name}</ToastTitle>
              </Toast>,
              { intent: "success", timeout: 4000 },
            );
          }}
        />
      )}

      <Toaster toasterId={toasterId} position="top-end" />
    </div>
  );
}

interface BranchesTableProps {
  branches: readonly BranchInfo[];
  activeBranch: string;
  styles: ReturnType<typeof useStyles>;
  onSwitch: (b: BranchInfo) => void;
  onViewHistory: (b: BranchInfo) => void;
  onMergeFrom: (b: BranchInfo) => void;
  onDelete: (b: BranchInfo) => void;
}

function BranchesTable({
  branches,
  activeBranch,
  styles,
  onSwitch,
  onViewHistory,
  onMergeFrom,
  onDelete,
}: BranchesTableProps) {
  return (
    <table className={styles.table}>
      <thead>
        <tr>
          <th className={styles.th}>Branch</th>
          <th className={styles.th}>Tip</th>
          <th className={styles.th}>Last commit</th>
          <th className={styles.th}>Actions</th>
        </tr>
      </thead>
      <tbody>
        {branches.map((b) => {
          const isActive = b.name === activeBranch;
          const isAuto = AUTO_BRANCH_RE.test(b.name);
          const isProtected = b.name === "main";
          return (
            <tr key={b.name}>
              <td className={styles.td}>
                <div className={styles.nameCell}>
                  <span
                    className={isAuto
                      ? styles.branchNameAuto
                      : styles.branchName}
                  >
                    {b.name}
                  </span>
                  {isActive && (
                    <span className={styles.activeBadge}>(active)</span>
                  )}
                </div>
              </td>
              <td className={styles.td}>
                <span className={styles.tip}>{shortHash(b.headLayer)}</span>
              </td>
              <td className={styles.td}>
                <LastCommitCell ms={Number(b.headCommittedAtMs)} />
              </td>
              <td className={styles.td}>
                <div className={styles.actions}>
                  <Button
                    size="small"
                    appearance="subtle"
                    disabled={isActive}
                    onClick={() => onSwitch(b)}
                  >
                    {isActive ? "Active" : "Switch"}
                  </Button>
                  <Tooltip
                    content={isActive
                      ? "Open the History panel for this branch."
                      : "Switch to this branch and open its History panel."}
                    relationship="description"
                  >
                    <Button
                      size="small"
                      appearance="subtle"
                      icon={<History16Regular />}
                      onClick={() => onViewHistory(b)}
                    >
                      View history
                    </Button>
                  </Tooltip>
                  <ComingSoonAction
                    label="Compact"
                    icon={<Stack16Regular />}
                    phase={6}
                  />
                  <Tooltip
                    content="Open the Merge panel with this branch pre-filled as the source."
                    relationship="description"
                  >
                    <Button
                      size="small"
                      appearance="subtle"
                      icon={<Merge16Regular />}
                      onClick={() => onMergeFrom(b)}
                    >
                      Merge into…
                    </Button>
                  </Tooltip>
                  <Tooltip
                    content={isProtected
                      ? "main is protected — the kernel rejects DeleteBranch for the default branch."
                      : "Permanently remove this branch ref. Layers reachable only through this branch become GC-eligible."}
                    relationship="description"
                  >
                    <Button
                      size="small"
                      appearance="subtle"
                      icon={<Delete16Regular />}
                      disabled={isProtected}
                      onClick={() => onDelete(b)}
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

interface ComingSoonActionProps {
  label: string;
  icon: React.ReactElement;
  phase: number;
}

function ComingSoonAction({ label, icon, phase }: ComingSoonActionProps) {
  return (
    <Tooltip
      content={`${label} — coming in Phase ${phase}.`}
      relationship="description"
    >
      <Button
        size="small"
        appearance="subtle"
        icon={icon}
        disabled
      >
        {label}
      </Button>
    </Tooltip>
  );
}

function shortHash(hex: string): string {
  if (hex.length <= 10) return hex;
  return `${hex.slice(0, 4)}…${hex.slice(-4)}`;
}

interface LastCommitCellProps {
  /** `0` means the kernel didn't surface a timestamp (no backend, or
   *  the head's handle was reclaimed). Renders an em-dash in that case. */
  ms: number;
}

/** Last-commit cell: relative time + tooltip with the absolute ISO
 *  stamp. Same display contract as the BranchBar's tip indicator
 *  (D34 §3.2). */
function LastCommitCell({ ms }: LastCommitCellProps) {
  if (ms <= 0) {
    return (
      <Caption1 style={{ color: "var(--colorNeutralForeground3)" }}>—</Caption1>
    );
  }
  return (
    <Tooltip
      relationship="description"
      content={formatAbsoluteIso(ms)}
      withArrow
    >
      <span>{formatRelative(ms)}</span>
    </Tooltip>
  );
}
