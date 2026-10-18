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
 * Renderer for `kind: "program-run"` cell outputs (Phase 4d).
 *
 * Single input: ResourceInspector + TraceTreePanel stacked, matching
 *   the TS-cell auto-renderer's RunProgramResponse split view.
 * Multiple inputs: a Fluent DataGrid with one row per input, columns
 *   for input IRI, status, output (collapsible inspector), and trace
 *   IRI (with an inline expand toggle that mounts the TraceTreePanel).
 */

import { useState } from "react";
import {
  Body1,
  Button,
  Caption1,
  makeStyles,
  MessageBar,
  MessageBarBody,
  MessageBarTitle,
  Tag,
  tokens,
} from "@fluentui/react-components";
import {
  ChevronDown16Regular,
  ChevronRight16Regular,
} from "@fluentui/react-icons";
import type { ProgramRunResult } from "../../runtime/notebookStore";
import { CommitStatusBadge } from "./CommitStatusBadge";
import { ResourceInspector } from "./ResourceInspector";
import { TraceTreePanel } from "./TraceTreePanel";

const useStyles = makeStyles({
  root: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalS,
  },
  header: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalS,
    color: tokens.colorNeutralForeground3,
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
  },
  panelLabel: {
    color: tokens.colorNeutralForeground3,
    textTransform: "uppercase",
    letterSpacing: "0.04em",
    fontSize: tokens.fontSizeBase100,
  },
  table: {
    width: "100%",
    borderCollapse: "collapse",
    fontSize: tokens.fontSizeBase200,
  },
  th: {
    textAlign: "left",
    fontWeight: tokens.fontWeightSemibold,
    color: tokens.colorNeutralForeground2,
    padding: `${tokens.spacingVerticalXS} ${tokens.spacingHorizontalS}`,
    borderBottom: `1px solid ${tokens.colorNeutralStroke2}`,
  },
  td: {
    padding: `${tokens.spacingVerticalXS} ${tokens.spacingHorizontalS}`,
    borderBottom: `1px solid ${tokens.colorNeutralStroke3}`,
    verticalAlign: "top",
  },
  iri: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    wordBreak: "break-all",
  },
  expanded: {
    background: tokens.colorNeutralBackground2,
  },
  expandedDetail: {
    padding: tokens.spacingVerticalS,
    background: tokens.colorNeutralBackground2,
    borderBottom: `1px solid ${tokens.colorNeutralStroke3}`,
  },
});

export interface ProgramRunOutputViewProps {
  programIri: string;
  results: readonly ProgramRunResult[];
}

export function ProgramRunOutputView(
  { programIri, results }: ProgramRunOutputViewProps,
) {
  const styles = useStyles();

  if (results.length === 0) {
    return <Caption1>(no inputs)</Caption1>;
  }

  // Single-input: render in the same shape the TS-cell auto-renderer
  // uses for a RunProgramResponse — output above, trace below.
  if (results.length === 1) {
    const r = results[0];
    return (
      <div className={styles.root}>
        <div className={styles.header}>
          <span>program: {programIri}</span>
          <span>·</span>
          <span>input: {r.inputIri}</span>
        </div>
        {!r.success
          ? (
            <MessageBar intent="error">
              <MessageBarBody>
                <MessageBarTitle>Program failed</MessageBarTitle>
                <div>{r.errorMessage ?? "(no error message)"}</div>
              </MessageBarBody>
            </MessageBar>
          )
          : (
            <>
              <div>
                <Caption1 className={styles.panelLabel}>output</Caption1>
                {r.output && (
                  <ResourceInspector
                    resource={r.output}
                    traceIri={r.traceIri}
                  />
                )}
              </div>
              {r.traceIri && (
                <div>
                  <Caption1 className={styles.panelLabel}>trace</Caption1>
                  <TraceTreePanel traceIri={r.traceIri} />
                </div>
              )}
              {r.commit && <CommitStatusBadge commit={r.commit} />}
            </>
          )}
      </div>
    );
  }

  // Multi-input: results table with expand-to-inspect rows.
  return (
    <div className={styles.root}>
      <div className={styles.header}>
        <span>program: {programIri}</span>
        <span>·</span>
        <span>{results.length} inputs</span>
      </div>
      <ResultTable results={results} styles={styles} />
    </div>
  );
}

interface ResultTableProps {
  results: readonly ProgramRunResult[];
  styles: ReturnType<typeof useStyles>;
}

function ResultTable({ results, styles }: ResultTableProps) {
  return (
    <table className={styles.table}>
      <thead>
        <tr>
          <th className={styles.th} style={{ width: "20px" }} />
          <th className={styles.th}>Input</th>
          <th className={styles.th}>Status</th>
          <th className={styles.th}>Trace</th>
        </tr>
      </thead>
      <tbody>
        {results.map((r, idx) => (
          <ResultRow key={idx} result={r} styles={styles} />
        ))}
      </tbody>
    </table>
  );
}

interface ResultRowProps {
  result: ProgramRunResult;
  styles: ReturnType<typeof useStyles>;
}

function ResultRow({ result, styles }: ResultRowProps) {
  const [expanded, setExpanded] = useState(false);
  const canExpand = result.success && (result.output || result.traceIri);

  return (
    <>
      <tr className={expanded ? styles.expanded : undefined}>
        <td className={styles.td}>
          {canExpand && (
            <Button
              size="small"
              appearance="subtle"
              icon={expanded
                ? <ChevronDown16Regular />
                : <ChevronRight16Regular />}
              aria-label={expanded ? "Collapse" : "Expand"}
              onClick={() => setExpanded((v) => !v)}
            />
          )}
        </td>
        <td className={`${styles.td} ${styles.iri}`}>{result.inputIri}</td>
        <td className={styles.td}>
          {result.success
            ? <Tag size="extra-small" appearance="brand">ok</Tag>
            : <Tag size="extra-small" appearance="filled">failed</Tag>}
        </td>
        <td className={`${styles.td} ${styles.iri}`}>
          {result.traceIri ?? <span>—</span>}
        </td>
      </tr>
      {expanded && (
        <tr>
          <td colSpan={4} className={styles.expandedDetail}>
            {result.success
              ? (
                <div className={styles.root}>
                  {result.output && (
                    <div>
                      <Caption1 className={styles.panelLabel}>output</Caption1>
                      <ResourceInspector
                        resource={result.output}
                        traceIri={result.traceIri}
                      />
                    </div>
                  )}
                  {result.traceIri && (
                    <div>
                      <Caption1 className={styles.panelLabel}>trace</Caption1>
                      <TraceTreePanel traceIri={result.traceIri} />
                    </div>
                  )}
                </div>
              )
              : (
                <Body1>
                  {result.errorMessage ?? "(no error message)"}
                </Body1>
              )}
            {result.commit && <CommitStatusBadge commit={result.commit} />}
          </td>
        </tr>
      )}
    </>
  );
}
