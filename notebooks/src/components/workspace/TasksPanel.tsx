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
 * Tasks rail destination — D34 §9.1.
 *
 * Tabular view over `Eigen.listTasks()` with per-row actions backed by
 * `getTaskStatus` / `cancelTask`. Each row shows:
 *
 *  - **Task ID** (short hash, hover for full id).
 *  - **Program IRI** — what `RunProgram` was invoked with.
 *  - **Status** badge, coloured by terminal state.
 *  - **Started** — relative timestamp of `created_at_ms`.
 *  - **Result layer** — short hash when the task is `Completed`;
 *    em-dash otherwise.
 *  - **Actions** — `Cancel` for live tasks, `Copy id` always.
 *
 * Auto-refreshes while any task is non-terminal (Running / Suspended /
 * Cancelling). Polls every 3s and stops as soon as all tasks reach a
 * terminal state; a manual `Refresh` button works regardless.
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Badge,
  Button,
  Caption1,
  makeStyles,
  MessageBar,
  MessageBarBody,
  MessageBarTitle,
  Spinner,
  Subtitle1,
  Switch,
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
  ArrowSync20Regular,
  ClipboardTaskListLtr20Regular,
  Copy16Regular,
  DismissCircle16Regular,
} from "@fluentui/react-icons";
import type { TaskInfo } from "@eigenius/client";
import { useEigen } from "../../runtime/EigenProvider";
import { formatAbsoluteIso, formatRelative } from "../../runtime/relativeTime";

const TOASTER_ID = "tasks-panel-toaster";

/** Status values the kernel returns on `TaskInfo.status` (D21 task lifecycle). */
const TERMINAL_STATUSES = new Set([
  "Completed",
  "Failed",
  "Cancelled",
]);

/** Polling cadence while at least one task is non-terminal. */
const REFRESH_INTERVAL_MS = 3_000;

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
  autoRefresh: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalXS,
  },
  body: {
    flex: 1,
    minHeight: 0,
    overflowY: "auto",
    padding: tokens.spacingVerticalM,
  },
  bodyInner: {
    maxWidth: "1100px",
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
  hash: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground3,
  },
  programIri: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    wordBreak: "break-all",
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

export function TasksPanel() {
  const styles = useStyles();
  const eigen = useEigen();
  const toasterId = useId("toaster", TOASTER_ID);
  const { dispatchToast } = useToastController(toasterId);

  const [tasks, setTasks] = useState<readonly TaskInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [autoRefresh, setAutoRefresh] = useState(true);
  const [busyTaskId, setBusyTaskId] = useState<string | null>(null);

  const fetchTasks = useCallback(async () => {
    try {
      const list = await eigen.listTasks();
      setTasks(list);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [eigen]);

  // Initial fetch on mount.
  useEffect(() => {
    void fetchTasks();
  }, [fetchTasks]);

  // While any task is non-terminal AND auto-refresh is on, poll. We
  // gate on tasks having loaded at least once so the initial mount
  // doesn't double-fire alongside the effect above.
  const hasLiveTask = useMemo(() => {
    if (!tasks) return false;
    return tasks.some((t) => !TERMINAL_STATUSES.has(t.status));
  }, [tasks]);

  useEffect(() => {
    if (!autoRefresh || !hasLiveTask) return;
    const handle = setInterval(() => {
      void fetchTasks();
    }, REFRESH_INTERVAL_MS);
    return () => clearInterval(handle);
  }, [autoRefresh, hasLiveTask, fetchTasks]);

  const onCancel = async (task: TaskInfo) => {
    setBusyTaskId(task.taskId);
    try {
      const resp = await eigen.cancelTask(task.taskId);
      if (!resp.success) {
        dispatchToast(
          <Toast>
            <ToastTitle>Cancel rejected</ToastTitle>
            <ToastBody>{resp.error || "kernel refused cancel"}</ToastBody>
          </Toast>,
          { intent: "error", timeout: 6000 },
        );
      } else {
        dispatchToast(
          <Toast>
            <ToastTitle>Task {resp.status.toLowerCase()}</ToastTitle>
            <ToastBody>
              {task.programIri || task.taskId} — new status: {resp.status}
            </ToastBody>
          </Toast>,
          { intent: "info", timeout: 4000 },
        );
      }
      // Refresh immediately so the row reflects the new status without
      // waiting for the poll tick.
      await fetchTasks();
    } catch (err) {
      dispatchToast(
        <Toast>
          <ToastTitle>Cancel failed</ToastTitle>
          <ToastBody>
            {err instanceof Error ? err.message : String(err)}
          </ToastBody>
        </Toast>,
        { intent: "error", timeout: 6000 },
      );
    } finally {
      setBusyTaskId(null);
    }
  };

  const onCopyId = (taskId: string) => {
    void navigator.clipboard.writeText(taskId);
    dispatchToast(
      <Toast>
        <ToastTitle>Copied task id</ToastTitle>
      </Toast>,
      { intent: "info", timeout: 1500 },
    );
  };

  // Newest first by `created_at_ms` so the user lands on the row that
  // most likely matters to them right now.
  const sorted = useMemo(() => {
    if (!tasks) return null;
    const copy = [...tasks];
    copy.sort((a, b) => Number(b.createdAtMs - a.createdAtMs));
    return copy;
  }, [tasks]);

  return (
    <div className={styles.root}>
      <div className={styles.header}>
        <ClipboardTaskListLtr20Regular />
        <Subtitle1 as="h2">Tasks</Subtitle1>
        <span className={styles.headerSpacer} />
        <div className={styles.autoRefresh}>
          <Switch
            checked={autoRefresh}
            onChange={(_e, data) => setAutoRefresh(data.checked === true)}
            label="Auto-refresh"
          />
        </div>
        <Tooltip content="Refresh now" relationship="label">
          <Button
            size="small"
            appearance="subtle"
            icon={<ArrowSync20Regular />}
            onClick={() => void fetchTasks()}
            aria-label="Refresh"
          />
        </Tooltip>
      </div>
      <div className={styles.body}>
        <div className={styles.bodyInner}>
          {error && (
            <MessageBar intent="error">
              <MessageBarBody>
                <MessageBarTitle>Couldn't load tasks</MessageBarTitle>
                {error}
              </MessageBarBody>
            </MessageBar>
          )}
          {sorted === null && !error && (
            <div className={styles.loadingState}>
              <Spinner size="tiny" />
              <Caption1>fetching task records…</Caption1>
            </div>
          )}
          {sorted !== null && sorted.length === 0 && (
            <div className={styles.emptyState}>
              No task records. Run a program via <code>RunProgram</code>{" "}
              to see it appear here.
            </div>
          )}
          {sorted !== null && sorted.length > 0 && (
            <TasksTable
              tasks={sorted}
              busyTaskId={busyTaskId}
              styles={styles}
              onCancel={(t) => void onCancel(t)}
              onCopyId={onCopyId}
            />
          )}
        </div>
      </div>
      <Toaster toasterId={toasterId} position="top-end" />
    </div>
  );
}

interface TasksTableProps {
  tasks: readonly TaskInfo[];
  busyTaskId: string | null;
  styles: ReturnType<typeof useStyles>;
  onCancel: (t: TaskInfo) => void;
  onCopyId: (id: string) => void;
}

function TasksTable({
  tasks,
  busyTaskId,
  styles,
  onCancel,
  onCopyId,
}: TasksTableProps) {
  return (
    <table className={styles.table}>
      <thead>
        <tr>
          <th className={styles.th}>Task ID</th>
          <th className={styles.th}>Program</th>
          <th className={styles.th}>Status</th>
          <th className={styles.th}>Started</th>
          <th className={styles.th}>Result layer</th>
          <th className={styles.th}>Actions</th>
        </tr>
      </thead>
      <tbody>
        {tasks.map((t) => {
          const isLive = !TERMINAL_STATUSES.has(t.status);
          const isBusy = busyTaskId === t.taskId;
          const createdMs = Number(t.createdAtMs);
          return (
            <tr key={t.taskId}>
              <td className={styles.td}>
                <Tooltip content={t.taskId} relationship="description">
                  <span className={styles.hash}>{shortHash(t.taskId)}</span>
                </Tooltip>
              </td>
              <td className={styles.td}>
                <span className={styles.programIri}>
                  {t.programIri || <Caption1>(unknown)</Caption1>}
                </span>
              </td>
              <td className={styles.td}>
                <StatusBadge status={t.status} />
              </td>
              <td className={styles.td}>
                <Tooltip
                  content={formatAbsoluteIso(createdMs) || "(no timestamp)"}
                  relationship="description"
                >
                  <span>{formatRelative(createdMs) || "—"}</span>
                </Tooltip>
              </td>
              <td className={styles.td}>
                {t.resultLayerHead
                  ? (
                    <Tooltip
                      content={t.resultLayerHead}
                      relationship="description"
                    >
                      <span className={styles.hash}>
                        {shortHash(t.resultLayerHead)}
                      </span>
                    </Tooltip>
                  )
                  : <span className={styles.hash}>—</span>}
              </td>
              <td className={styles.td}>
                <div className={styles.actions}>
                  <Tooltip
                    content="Copy the full task id to the clipboard."
                    relationship="description"
                  >
                    <Button
                      size="small"
                      appearance="subtle"
                      icon={<Copy16Regular />}
                      onClick={() => onCopyId(t.taskId)}
                    >
                      Copy id
                    </Button>
                  </Tooltip>
                  {isLive && (
                    <Tooltip
                      content="Request cancellation. Cancelling tasks reach Cancelled once the runtime acknowledges."
                      relationship="description"
                    >
                      <Button
                        size="small"
                        appearance="subtle"
                        icon={<DismissCircle16Regular />}
                        disabled={isBusy}
                        onClick={() => onCancel(t)}
                      >
                        {isBusy ? "Cancelling…" : "Cancel"}
                      </Button>
                    </Tooltip>
                  )}
                </div>
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

interface StatusBadgeProps {
  status: string;
}

/**
 * Colour the task status by lifecycle category:
 * - Running / Suspended → informative (blue tint).
 * - Cancelling → warning.
 * - Completed → success.
 * - Failed / Cancelled → danger.
 * Anything else falls back to neutral.
 */
function StatusBadge({ status }: StatusBadgeProps) {
  let color: "informative" | "warning" | "success" | "danger" | "subtle" =
    "subtle";
  switch (status) {
    case "Running":
    case "Suspended":
      color = "informative";
      break;
    case "Cancelling":
      color = "warning";
      break;
    case "Completed":
      color = "success";
      break;
    case "Failed":
    case "Cancelled":
      color = "danger";
      break;
  }
  return (
    <Badge appearance="tint" color={color} size="small">
      {status}
    </Badge>
  );
}

function shortHash(hex: string): string {
  if (hex.length <= 10) return hex;
  return `${hex.slice(0, 4)}…${hex.slice(-4)}`;
}
