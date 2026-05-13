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
 * GC rail destination — D34 §9.4.
 *
 * Two-screen flow:
 *
 *   1. **Estimate** — call `eigen.estimateGc()` and surface
 *      `eligible_layers`, `protected_by_age`, and the protection
 *      accounting (branch / tag / task pins). Read-only on the
 *      kernel; safe to invoke at any time.
 *   2. **Confirm + Run** — destructive. The big primary button
 *      opens a confirmation dialog spelling out what will be
 *      reclaimed; on confirm, `eigen.runGc()` executes the sweep
 *      and the result replaces the estimate panel.
 *
 * Per D34 §9.4 the MVP is not admin-gated — the dialog is the only
 * friction between the user and the sweep. Production deployments
 * that want a stricter gate revisit it when the D22 auth model is
 * hardened.
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
  MessageBarTitle,
  Spinner,
  Subtitle1,
  Toast,
  ToastBody,
  Toaster,
  ToastTitle,
  tokens,
  useId,
  useToastController,
} from "@fluentui/react-components";
import {
  Archive20Regular,
  ArrowSync20Regular,
  Delete20Regular,
} from "@fluentui/react-icons";
import type { EstimateGcResponse, RunGcResponse } from "@eigenius/client";
import { useEigen } from "../../runtime/EigenProvider";

const TOASTER_ID = "gc-panel-toaster";

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
    padding: tokens.spacingVerticalXXL,
  },
  bodyInner: {
    maxWidth: "640px",
    margin: "0 auto",
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalL,
  },
  block: {
    padding: tokens.spacingVerticalM,
    background: tokens.colorNeutralBackground2,
    borderRadius: tokens.borderRadiusMedium,
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalS,
  },
  metricsGrid: {
    display: "grid",
    gridTemplateColumns: "max-content 1fr",
    columnGap: tokens.spacingHorizontalM,
    rowGap: tokens.spacingVerticalXS,
  },
  metricLabel: {
    color: tokens.colorNeutralForeground3,
  },
  actions: {
    display: "flex",
    gap: tokens.spacingHorizontalS,
    justifyContent: "flex-end",
  },
  bigNumber: {
    fontSize: tokens.fontSizeHero700,
    lineHeight: tokens.lineHeightHero700,
    fontWeight: tokens.fontWeightSemibold,
  },
});

type EstimateState =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "ready"; resp: EstimateGcResponse }
  | { kind: "error"; message: string };

type RunState =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "done"; resp: RunGcResponse }
  | { kind: "error"; message: string };

export function GcPanel() {
  const styles = useStyles();
  const eigen = useEigen();
  const toasterId = useId("toaster", TOASTER_ID);
  const { dispatchToast } = useToastController(toasterId);

  const [estimate, setEstimate] = useState<EstimateState>({ kind: "idle" });
  const [run, setRun] = useState<RunState>({ kind: "idle" });
  const [confirmOpen, setConfirmOpen] = useState(false);

  const onEstimate = async () => {
    setEstimate({ kind: "loading" });
    setRun({ kind: "idle" });
    try {
      const resp = await eigen.estimateGc();
      setEstimate({ kind: "ready", resp });
    } catch (err) {
      setEstimate({
        kind: "error",
        message: err instanceof Error ? err.message : String(err),
      });
    }
  };

  // Auto-run an initial estimate on mount so the operator lands on
  // useful numbers without clicking. The button stays available for
  // re-fetches.
  useEffect(() => {
    void onEstimate();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [eigen]);

  const onRun = async () => {
    setConfirmOpen(false);
    setRun({ kind: "running" });
    try {
      const resp = await eigen.runGc();
      setRun({ kind: "done", resp });
      if (resp.success) {
        dispatchToast(
          <Toast>
            <ToastTitle>GC complete</ToastTitle>
            <ToastBody>
              Swept {String(resp.layersSwept)} layers; marked{" "}
              {String(resp.layersMarked)} reachable.
            </ToastBody>
          </Toast>,
          { intent: "success", timeout: 6000 },
        );
        // Refresh the estimate so the operator sees the post-sweep
        // state — eligible should now be 0 (or smaller).
        void onEstimate();
      }
    } catch (err) {
      setRun({
        kind: "error",
        message: err instanceof Error ? err.message : String(err),
      });
    }
  };

  const eligibleCount = estimate.kind === "ready" && estimate.resp.success
    ? Number(estimate.resp.eligibleLayers)
    : 0;
  const canRun = estimate.kind === "ready" &&
    estimate.resp.success &&
    eligibleCount > 0 &&
    run.kind !== "running";

  return (
    <div className={styles.root}>
      <div className={styles.header}>
        <Archive20Regular />
        <Subtitle1 as="h2">Garbage collection</Subtitle1>
        <span className={styles.headerSpacer} />
        <Button
          appearance="secondary"
          icon={<ArrowSync20Regular />}
          disabled={estimate.kind === "loading"}
          onClick={() => void onEstimate()}
        >
          {estimate.kind === "loading" ? "Estimating…" : "Refresh estimate"}
        </Button>
      </div>
      <div className={styles.body}>
        <div className={styles.bodyInner}>
          <EstimateBlock state={estimate} styles={styles} />
          <RunBlock state={run} styles={styles} />
          <div className={styles.actions}>
            <Button
              appearance="primary"
              icon={<Delete20Regular />}
              disabled={!canRun}
              onClick={() => setConfirmOpen(true)}
            >
              {run.kind === "running"
                ? "Running…"
                : eligibleCount > 0
                ? `Run GC — sweep ${eligibleCount} layer${
                  eligibleCount === 1 ? "" : "s"
                }`
                : "Nothing to sweep"}
            </Button>
          </div>
        </div>
      </div>

      <ConfirmDialog
        open={confirmOpen}
        eligibleCount={eligibleCount}
        onCancel={() => setConfirmOpen(false)}
        onConfirm={() => void onRun()}
      />

      <Toaster toasterId={toasterId} position="top-end" />
    </div>
  );
}

interface EstimateBlockProps {
  state: EstimateState;
  styles: ReturnType<typeof useStyles>;
}

function EstimateBlock({ state, styles }: EstimateBlockProps) {
  if (state.kind === "idle") {
    return (
      <div className={styles.block}>
        <Caption1>
          Click <strong>Refresh estimate</strong> to begin.
        </Caption1>
      </div>
    );
  }
  if (state.kind === "loading") {
    return (
      <div className={styles.block}>
        <Spinner size="tiny" label="computing estimate" />
      </div>
    );
  }
  if (state.kind === "error") {
    return (
      <MessageBar intent="error">
        <MessageBarBody>
          <MessageBarTitle>Estimate failed</MessageBarTitle>
          {state.message}
        </MessageBarBody>
      </MessageBar>
    );
  }
  const resp = state.resp;
  if (!resp.success) {
    return (
      <MessageBar intent="error">
        <MessageBarBody>{resp.error || "estimate failed"}</MessageBarBody>
      </MessageBar>
    );
  }
  const eligible = Number(resp.eligibleLayers);
  const protectedByAge = Number(resp.protectedByAge);
  return (
    <div className={styles.block}>
      <Body1>
        <strong>Estimated outcome</strong>
      </Body1>
      <div className={styles.bigNumber}>
        {eligible}{" "}
        <Caption1 as="span">
          layer{eligible === 1 ? "" : "s"} eligible for sweep
        </Caption1>
      </div>
      <div className={styles.metricsGrid}>
        <Caption1 className={styles.metricLabel}>Reclaimable</Caption1>
        <Body1>
          {formatBytes(resp.reclaimableBytes)}{" "}
          <Caption1 as="span">
            (encoded resource bytes; per-layer index/bloom overhead not
            included)
          </Caption1>
        </Body1>
        <Caption1 className={styles.metricLabel}>Protected by min-age</Caption1>
        <Body1>
          {protectedByAge} layer{protectedByAge === 1 ? "" : "s"}{" "}
          (unreachable but committed within the kernel's 60-second window — will
          sweep on a later pass)
        </Body1>
        <Caption1 className={styles.metricLabel}>Reachability roots</Caption1>
        <Body1>
          {String(resp.branchPins)} branch · {String(resp.tagPins)} tag ·{" "}
          {String(resp.taskPins)} active task
        </Body1>
      </div>
      <Caption1>
        Eligible = unreachable from any root AND older than the kernel's min-age
        window. Sweeping does not affect reachable layers.
      </Caption1>
    </div>
  );
}

/**
 * Render a byte count in the largest sensible unit. Uses 1024-based
 * units (KiB / MiB / GiB) because the operator's mental model is
 * "how much disk does this free", which matches the binary
 * convention RocksDB-side tooling uses. Negative / NaN / undefined
 * collapse to `"0 B"` defensively.
 */
function formatBytes(bytes: bigint | number): string {
  const n = typeof bytes === "bigint" ? Number(bytes) : bytes;
  if (!Number.isFinite(n) || n <= 0) return "0 B";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let value = n;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  // 1 decimal once we're past raw bytes; integer in the B range so
  // small handle-only chains don't render "12.0 B".
  return unitIndex === 0
    ? `${Math.round(value)} ${units[unitIndex]}`
    : `${value.toFixed(1)} ${units[unitIndex]}`;
}

interface RunBlockProps {
  state: RunState;
  styles: ReturnType<typeof useStyles>;
}

function RunBlock({ state, styles }: RunBlockProps) {
  if (state.kind === "idle") return null;
  if (state.kind === "running") {
    return (
      <div className={styles.block}>
        <Spinner size="tiny" label="running gc" />
      </div>
    );
  }
  if (state.kind === "error") {
    return (
      <MessageBar intent="error">
        <MessageBarBody>
          <MessageBarTitle>GC failed</MessageBarTitle>
          {state.message}
        </MessageBarBody>
      </MessageBar>
    );
  }
  const resp = state.resp;
  if (!resp.success) {
    return (
      <MessageBar intent="error">
        <MessageBarBody>{resp.error || "gc failed"}</MessageBarBody>
      </MessageBar>
    );
  }
  return (
    <MessageBar intent="success">
      <MessageBarBody>
        <MessageBarTitle>
          Swept {String(resp.layersSwept)} layer
          {Number(resp.layersSwept) === 1 ? "" : "s"}
        </MessageBarTitle>
        <div>
          {String(resp.layersMarked)} reachable ·{" "}
          {String(resp.layersUnreachable)} unreachable ·{" "}
          {String(resp.layersProtectedByAge)} skipped (younger than min-age)
        </div>
      </MessageBarBody>
    </MessageBar>
  );
}

interface ConfirmDialogProps {
  open: boolean;
  eligibleCount: number;
  onCancel: () => void;
  onConfirm: () => void;
}

function ConfirmDialog({
  open,
  eligibleCount,
  onCancel,
  onConfirm,
}: ConfirmDialogProps) {
  return (
    <Dialog
      open={open}
      onOpenChange={(_e, data) => {
        if (!data.open) onCancel();
      }}
    >
      <DialogSurface>
        <DialogBody>
          <DialogTitle>Run garbage collection?</DialogTitle>
          <DialogContent>
            <Body1>
              <strong>{eligibleCount}</strong>{" "}
              unreachable layer{eligibleCount === 1 ? "" : "s"}{" "}
              will be permanently removed from storage. Branch refs, tag refs,
              and active task pins protect the rest.
            </Body1>
          </DialogContent>
          <DialogActions>
            <Button appearance="secondary" onClick={onCancel}>
              Cancel
            </Button>
            <Button appearance="primary" onClick={onConfirm}>
              Run GC
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
