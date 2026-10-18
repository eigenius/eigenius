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
 * D36 §§4–8 — Merge resolution flow driver.
 *
 * Six-state state-machine renderer (loading / picking / previewing
 * / acknowledging / committing / done|error). State + transitions
 * live in the Zustand store; this component is the view layer.
 */
import { useMemo } from "react";
import {
  Body1,
  Button,
  Caption1,
  Link,
  makeStyles,
  MessageBar,
  MessageBarActions,
  MessageBarBody,
  MessageBarTitle,
  Spinner,
  Subtitle1,
  tokens,
} from "@fluentui/react-components";
import {
  CheckmarkCircle20Regular,
  QuestionCircle20Regular,
} from "@fluentui/react-icons";
import {
  PrepareMergeErrorKind,
  PreviewCascadeErrorKind,
  SubmitResolutionErrorKind,
} from "@eigenius/client";
import { useEigen } from "../../runtime/EigenProvider";
import { useNotebookStore } from "../../runtime/notebookStore";
import { ackCount, isResolvable } from "../../runtime/mergeResolution";
import { StrategyPicker } from "./StrategyPicker";
import { CascadePreviewPane } from "./CascadePreviewPane";

/**
 * D36 §15 (Decisions log §15.5) — in-app help link. Points at the
 * mkdocs-rendered platform guide chapter; users running `mkdocs
 * serve` alongside the notebook see the live doc, users without it
 * see the source markdown via the GitHub repo path. Single
 * constant so future hosting changes are one-line edits.
 */
const MERGE_RESOLUTION_GUIDE_URL =
  "https://eigenius.io/";

const useStyles = makeStyles({
  root: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalL,
  },
  header: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalXS,
  },
  helpLink: {
    display: "inline-flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalXXS,
    fontSize: tokens.fontSizeBase200,
  },
  conflictList: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalM,
  },
  actions: {
    display: "flex",
    gap: tokens.spacingHorizontalS,
    justifyContent: "flex-end",
    alignItems: "center",
  },
  raceBanner: {
    marginBottom: tokens.spacingVerticalS,
  },
  doneCard: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalM,
    padding: tokens.spacingVerticalL,
    border: `1px solid ${tokens.colorPaletteGreenBorder1}`,
    borderRadius: tokens.borderRadiusMedium,
    background: tokens.colorPaletteGreenBackground1,
  },
});

export function MergeResolutionFlow() {
  const styles = useStyles();
  const state = useNotebookStore((s) => s.mergeResolution);
  const eigen = useEigen();

  // Pulled here (vs. inside switch arms) so the hook call set is
  // stable across re-renders regardless of state.kind.
  const setMergeResolution = useNotebookStore((s) => s.setMergeResolution);
  const previewMergeCascade = useNotebookStore((s) => s.previewMergeCascade);
  const toggleCascadeAck = useNotebookStore((s) => s.toggleCascadeAck);
  const commitMergeResolution = useNotebookStore((s) =>
    s.commitMergeResolution
  );
  const cancelMergeResolution = useNotebookStore((s) =>
    s.cancelMergeResolution
  );
  const retryMergeResolution = useNotebookStore((s) =>
    s.retryMergeResolution
  );

  const resolvable = useMemo(() => isResolvable(state), [state]);
  const acks = useMemo(() => ackCount(state), [state]);

  switch (state.kind) {
    case "closed":
      return null;

    case "loading":
      return (
        <div className={styles.root}>
          <Spinner label="Loading conflicts…" />
        </div>
      );

    case "picking":
      return (
        <div className={styles.root}>
          <header className={styles.header}>
            <Subtitle1>Resolve {state.conflicts.length} conflict(s)</Subtitle1>
            <Caption1>
              Branch <code>{state.branch}</code> @{" "}
              <code>{state.branchTip.slice(0, 12)}…</code> ↔ candidate{" "}
              <code>{state.candidateHead.slice(0, 12)}…</code>
            </Caption1>
            <Link
              className={styles.helpLink}
              href={MERGE_RESOLUTION_GUIDE_URL}
              target="_blank"
              rel="noreferrer"
            >
              <QuestionCircle20Regular />
              Strategy reference
            </Link>
          </header>

          {state.raceDiff && (
            <MessageBar intent="warning" className={styles.raceBanner}>
              <MessageBarBody>
                <MessageBarTitle>The branch moved</MessageBarTitle>
                {state.raceDiff.added.length > 0 && (
                  <Caption1>
                    New conflicts:{" "}
                    {state.raceDiff.added.map((id, i) => (
                      <span key={id}>
                        {i > 0 && ", "}
                        <code>{id}</code>
                      </span>
                    ))}
                  </Caption1>
                )}
                {state.raceDiff.removed.length > 0 && (
                  <Caption1>
                    Previously-resolved conflicts now gone:{" "}
                    {state.raceDiff.removed.map((id, i) => (
                      <span key={id}>
                        {i > 0 && ", "}
                        <code>{id}</code>
                      </span>
                    ))}
                  </Caption1>
                )}
              </MessageBarBody>
            </MessageBar>
          )}

          <div className={styles.conflictList}>
            {state.conflicts.map((conflict) => (
              <StrategyPicker
                key={conflict.id}
                conflict={conflict}
                resolution={state.resolutions[conflict.id]}
                setResolution={setMergeResolution}
              />
            ))}
          </div>

          <div className={styles.actions}>
            <Button appearance="subtle" onClick={cancelMergeResolution}>
              Cancel
            </Button>
            <Button
              appearance="primary"
              disabled={!resolvable}
              onClick={() => previewMergeCascade(eigen)}
            >
              Preview cascade
            </Button>
          </div>
        </div>
      );

    case "previewing":
    case "committing":
      return (
        <div className={styles.root}>
          <Spinner
            label={state.kind === "previewing"
              ? "Computing cascade preview…"
              : "Committing merge…"}
          />
        </div>
      );

    case "acknowledging":
      return (
        <div className={styles.root}>
          <header className={styles.header}>
            <Subtitle1>Acknowledge consequences</Subtitle1>
            <Caption1>
              {state.preview.length === 0
                ? "No downstream consequences detected."
                : `Tick each item to confirm you've reviewed it (${acks.acked}/${acks.total} acked).`}
            </Caption1>
          </header>

          <CascadePreviewPane
            items={state.preview}
            acknowledged={state.acknowledged}
            onToggle={toggleCascadeAck}
          />

          <div className={styles.actions}>
            <Button
              appearance="subtle"
              onClick={cancelMergeResolution}
            >
              Cancel
            </Button>
            <Button
              appearance="primary"
              disabled={acks.acked < acks.total}
              onClick={() => commitMergeResolution(eigen)}
            >
              Commit merge
            </Button>
          </div>
        </div>
      );

    case "done":
      return (
        <div className={styles.root}>
          <div className={styles.doneCard}>
            <Subtitle1>
              <CheckmarkCircle20Regular /> Merge committed
            </Subtitle1>
            <Body1>
              Branch <code>{state.branch}</code> advanced to merge layer{" "}
              <code>{state.mergeLayerId.slice(0, 12)}…</code>.
            </Body1>
            {state.cellId && (
              <Caption1>
                The cell that triggered this resolution will clear its
                error badge automatically.
              </Caption1>
            )}
            <div className={styles.actions}>
              <Button onClick={cancelMergeResolution}>Close</Button>
            </div>
          </div>
        </div>
      );

    case "error":
      return (
        <div className={styles.root}>
          <MessageBar intent={errorIntent(state.rpc, state.errorKind)}>
            <MessageBarBody>
              <MessageBarTitle>{errorTitle(state.rpc, state.errorKind)}</MessageBarTitle>
              <Body1>{state.message}</Body1>
              {state.missingAcks && state.missingAcks.length > 0 && (
                <Caption1>
                  Missing acknowledgments:{" "}
                  {state.missingAcks.map((id, i) => (
                    <span key={id}>
                      {i > 0 && ", "}
                      <code>{id}</code>
                    </span>
                  ))}
                </Caption1>
              )}
            </MessageBarBody>
            <MessageBarActions>
              {state.retryFrom !== null && (
                <Button
                  size="small"
                  appearance="primary"
                  onClick={() => retryMergeResolution(eigen)}
                >
                  Try again
                </Button>
              )}
              <Button size="small" onClick={cancelMergeResolution}>
                Close
              </Button>
            </MessageBarActions>
          </MessageBar>
        </div>
      );
  }
}

function errorIntent(
  rpc: "prepare" | "preview" | "submit",
  errorKind: number,
): "error" | "warning" {
  if (
    rpc === "submit" &&
    errorKind === SubmitResolutionErrorKind.INCOMPLETE_ACKNOWLEDGMENTS
  ) {
    return "warning";
  }
  if (
    rpc === "submit" &&
    (errorKind === SubmitResolutionErrorKind.BRANCH_CAS_RACE ||
      errorKind === SubmitResolutionErrorKind.CONFLICT_NOT_FOUND)
  ) {
    return "warning";
  }
  return "error";
}

function errorTitle(
  rpc: "prepare" | "preview" | "submit",
  errorKind: number,
): string {
  if (rpc === "prepare") {
    if (errorKind === PrepareMergeErrorKind.NO_COMMON_ANCESTOR) {
      return "No common ancestor";
    }
    return "Preparing merge failed";
  }
  if (rpc === "preview") {
    if (errorKind === PreviewCascadeErrorKind.CONFLICT_NOT_FOUND) {
      return "Conflict id became stale";
    }
    if (errorKind === PreviewCascadeErrorKind.MALFORMED_RESOLUTION) {
      return "Resolution shape is invalid";
    }
    return "Computing cascade preview failed";
  }
  // submit
  switch (errorKind) {
    case SubmitResolutionErrorKind.INCOMPLETE_ACKNOWLEDGMENTS:
      return "Acknowledgments missing";
    case SubmitResolutionErrorKind.BRANCH_CAS_RACE:
      return "The branch moved";
    case SubmitResolutionErrorKind.CONFLICT_NOT_FOUND:
      return "Conflict id became stale";
    case SubmitResolutionErrorKind.NO_COMMON_ANCESTOR:
      return "No common ancestor";
    case SubmitResolutionErrorKind.MALFORMED_RESOLUTION:
      return "Resolution shape is invalid";
    case SubmitResolutionErrorKind.APPLICATION_PENDING:
      return "Resolution strategy not yet wired";
    default:
      return "Committing merge failed";
  }
}

