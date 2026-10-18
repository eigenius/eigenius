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
 * Health rail destination — D34 §3.1's "last position" rail item.
 *
 * Surfaces `eigen.health()` for kernel liveness + the D21 §4 resume
 * observability:
 *
 *  - **Healthy** badge driven by the boolean on the response.
 *  - **Version** — `CARGO_PKG_VERSION` from the kernel build.
 *  - **Layer count** + **Resource count** — quick sanity check against
 *    the current head's chain size.
 *  - **Resume sweep** — green idle, blue with a spinner + remaining-
 *    task count while the startup sweep is still draining.
 *
 * Auto-refreshes every 3 s while a resume is in progress so the
 * operator can watch the count drain; idle when there's nothing to
 * watch (a manual refresh button stays available).
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Badge,
  Body1,
  Body1Strong,
  Button,
  Caption1,
  makeStyles,
  MessageBar,
  MessageBarBody,
  MessageBarTitle,
  Spinner,
  Subtitle1,
  tokens,
  Tooltip,
  useId,
} from "@fluentui/react-components";
import {
  ArrowSync20Regular,
  CheckmarkCircle20Regular,
  ErrorCircle20Regular,
} from "@fluentui/react-icons";
import type { HealthResponse } from "@eigenius/client";
import { useEigen } from "../../runtime/EigenProvider";

/** Polling cadence while a resume sweep is draining. */
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
  statusBlock: {
    padding: tokens.spacingVerticalL,
    background: tokens.colorNeutralBackground2,
    borderRadius: tokens.borderRadiusMedium,
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalS,
  },
  statusHeader: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalS,
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
  resumeBlock: {
    padding: tokens.spacingVerticalM,
    background: tokens.colorNeutralBackground2,
    borderRadius: tokens.borderRadiusMedium,
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalS,
  },
  loadingState: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalS,
    padding: tokens.spacingVerticalXXL,
    color: tokens.colorNeutralForeground3,
  },
});

export function HealthPanel() {
  const styles = useStyles();
  const eigen = useEigen();
  const loadingId = useId("health-loading");

  const [health, setHealth] = useState<HealthResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [lastFetchMs, setLastFetchMs] = useState<number | null>(null);

  const fetchHealth = useCallback(async () => {
    try {
      const resp = await eigen.health();
      setHealth(resp);
      setError(null);
      setLastFetchMs(Date.now());
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [eigen]);

  useEffect(() => {
    void fetchHealth();
  }, [fetchHealth]);

  // Poll while a resume sweep is draining so the count goes down on
  // its own; idle the rest of the time to avoid background chatter.
  const resumeInProgress = useMemo(
    () => health?.resumeInProgress === true,
    [health],
  );

  useEffect(() => {
    if (!resumeInProgress) return;
    const handle = setInterval(() => {
      void fetchHealth();
    }, REFRESH_INTERVAL_MS);
    return () => clearInterval(handle);
  }, [resumeInProgress, fetchHealth]);

  const lastFetchLabel = useMemo(() => {
    if (lastFetchMs === null) return null;
    const now = Date.now();
    const secs = Math.max(0, Math.round((now - lastFetchMs) / 1000));
    if (secs < 2) return "just now";
    if (secs < 60) return `${secs}s ago`;
    return `${Math.round(secs / 60)} min ago`;
  }, [lastFetchMs]);

  return (
    <div className={styles.root}>
      <div className={styles.header}>
        <CheckmarkCircle20Regular />
        <Subtitle1 as="h2">Health</Subtitle1>
        <span className={styles.headerSpacer} />
        {lastFetchLabel && <Caption1>refreshed {lastFetchLabel}</Caption1>}
        <Tooltip content="Refresh now" relationship="label">
          <Button
            size="small"
            appearance="subtle"
            icon={<ArrowSync20Regular />}
            onClick={() => void fetchHealth()}
            aria-label="Refresh"
          />
        </Tooltip>
      </div>
      <div className={styles.body}>
        <div className={styles.bodyInner}>
          {error && (
            <MessageBar intent="error">
              <MessageBarBody>
                <MessageBarTitle>Health check failed</MessageBarTitle>
                {error}
              </MessageBarBody>
            </MessageBar>
          )}
          {health === null && !error && (
            <div className={styles.loadingState} aria-labelledby={loadingId}>
              <Spinner size="tiny" />
              <Caption1 id={loadingId}>pinging kernel…</Caption1>
            </div>
          )}
          {health && <HealthStatus health={health} styles={styles} />}
        </div>
      </div>
    </div>
  );
}

interface HealthStatusProps {
  health: HealthResponse;
  styles: ReturnType<typeof useStyles>;
}

function HealthStatus({ health, styles }: HealthStatusProps) {
  return (
    <>
      <div className={styles.statusBlock}>
        <div className={styles.statusHeader}>
          {health.healthy
            ? (
              <>
                <CheckmarkCircle20Regular
                  style={{ color: "var(--colorPaletteGreenForeground1)" }}
                />
                <Body1Strong>Kernel healthy</Body1Strong>
                <Badge appearance="tint" color="success" size="small">
                  ok
                </Badge>
              </>
            )
            : (
              <>
                <ErrorCircle20Regular
                  style={{ color: "var(--colorPaletteRedForeground1)" }}
                />
                <Body1Strong>Kernel unhealthy</Body1Strong>
                <Badge appearance="tint" color="danger" size="small">
                  error
                </Badge>
              </>
            )}
        </div>
        <div className={styles.metricsGrid}>
          <Caption1 className={styles.metricLabel}>Version</Caption1>
          <span style={{ fontFamily: "var(--fontFamilyMonospace)" }}>
            {health.version || "(unknown)"}
          </span>
          <Caption1 className={styles.metricLabel}>Layer count</Caption1>
          <span>{String(health.layerCount)}</span>
          <Caption1 className={styles.metricLabel}>Resource count</Caption1>
          <span>
            {String(health.resourceCount)}{" "}
            <Caption1 as="span">(at current head)</Caption1>
          </span>
        </div>
      </div>

      <ResumeStatus health={health} styles={styles} />
    </>
  );
}

interface ResumeStatusProps {
  health: HealthResponse;
  styles: ReturnType<typeof useStyles>;
}

function ResumeStatus({ health, styles }: ResumeStatusProps) {
  if (!health.resumeInProgress) {
    return (
      <div className={styles.resumeBlock}>
        <CheckmarkCircle20Regular
          style={{ color: "var(--colorPaletteGreenForeground1)" }}
        />
        <Body1>Resume sweep idle.</Body1>
        <Caption1>
          No Running or Suspended tasks awaiting startup re-evaluation.
        </Caption1>
      </div>
    );
  }
  return (
    <div className={styles.resumeBlock}>
      <Spinner size="tiny" />
      <Body1>
        Resume sweep in progress — {health.tasksResuming}{" "}
        task{health.tasksResuming === 1 ? "" : "s"} remaining.
      </Body1>
    </div>
  );
}
