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

import { useCallback } from "react";
import {
  Button,
  Caption1,
  Card,
  makeStyles,
  Menu,
  MenuItem,
  MenuList,
  MenuPopover,
  MenuTrigger,
  Spinner,
  SplitButton,
  tokens,
  Tooltip,
} from "@fluentui/react-components";
import {
  ArrowDown16Regular,
  ArrowUp16Regular,
  ChevronDown16Regular,
  ChevronRight16Regular,
  Delete16Regular,
  Play16Regular,
} from "@fluentui/react-icons";
import type { CellType } from "../persistence/notebook-format";
import { useEigen } from "../runtime/EigenProvider";
import { useNotebookStore } from "../runtime/notebookStore";
import { CodeMirrorEditor } from "./editors/CodeMirrorEditor";
import { ChartCellEditor } from "./cells/ChartCell";
import { MarkdownCell } from "./cells/MarkdownCell";
import { ProgramRunCellEditor } from "./cells/ProgramRunCell";
import { CellOutputView } from "./output/CellOutputView";

const useStyles = makeStyles({
  card: {
    marginBottom: tokens.spacingVerticalM,
  },
  body: {
    padding: tokens.spacingVerticalS,
  },
  toolbar: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalS,
    padding: `${tokens.spacingVerticalXS} ${tokens.spacingHorizontalS}`,
    borderBottom: `1px solid ${tokens.colorNeutralStroke2}`,
  },
  typeBadge: {
    minWidth: "80px",
    color: tokens.colorNeutralForeground3,
    textTransform: "uppercase",
    letterSpacing: "0.04em",
  },
  indexCircle: {
    display: "inline-flex",
    alignItems: "center",
    justifyContent: "center",
    width: "22px",
    height: "22px",
    borderRadius: tokens.borderRadiusCircular,
    color: tokens.colorNeutralForegroundOnBrand,
    fontSize: tokens.fontSizeBase200,
    fontWeight: tokens.fontWeightSemibold,
    fontVariantNumeric: "tabular-nums",
    flexShrink: 0,
  },
  indexMarkdown: { background: tokens.colorPaletteBeigeForeground2 },
  indexEsl: { background: tokens.colorPaletteBlueForeground2 },
  indexEigenql: { background: tokens.colorPaletteGreenForeground2 },
  indexTypescript: { background: tokens.colorPaletteMarigoldForeground2 },
  indexProgramRun: { background: tokens.colorPalettePurpleForeground2 },
  indexChart: { background: tokens.colorPaletteRedForeground2 },
  spacer: {
    flex: 1,
  },
  staleHint: {
    color: tokens.colorPaletteDarkOrangeForeground1,
    fontStyle: "italic",
    cursor: "default",
  },
  rightCluster: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalXS,
    marginLeft: tokens.spacingHorizontalM,
  },
});

export interface CellProps {
  cellId: string;
}

const RUNNABLE: Record<CellType, boolean> = {
  markdown: false,
  esl: true,
  eigenql: true,
  typescript: true,
  "program-run": true,
  chart: true,
};

const TYPE_LABEL: Record<CellType, string> = {
  markdown: "Markdown",
  esl: "ESL",
  eigenql: "EigenQL",
  typescript: "TypeScript",
  "program-run": "Program run",
  chart: "Chart",
};

function indexCircleClass(
  styles: ReturnType<typeof useStyles>,
  type: CellType,
): string {
  const colorClass: Record<CellType, string> = {
    markdown: styles.indexMarkdown,
    esl: styles.indexEsl,
    eigenql: styles.indexEigenql,
    typescript: styles.indexTypescript,
    "program-run": styles.indexProgramRun,
    chart: styles.indexChart,
  };
  return `${styles.indexCircle} ${colorClass[type]}`;
}

/**
 * Generic cell wrapper — Fluent `Card` shell with a toolbar (type
 * dropdown, Run, more-actions menu) and a body that delegates to the
 * Markdown renderer or to a CodeMirror editor sized for the cell type.
 */
export function Cell({ cellId }: CellProps) {
  const styles = useStyles();
  const eigen = useEigen();
  const cell = useNotebookStore((s) => s.cells.find((c) => c.id === cellId));
  const cellIndex = useNotebookStore(
    (s) => s.cells.findIndex((c) => c.id === cellId),
  );
  const runState = useNotebookStore(
    (s) => s.cellStates.get(cellId) ?? "idle",
  );
  const output = useNotebookStore((s) => s.cellOutputs.get(cellId));
  const lastRunIndex = useNotebookStore((s) => {
    if (!s.lastRunCellId) return -1;
    return s.cells.findIndex((c) => c.id === s.lastRunCellId);
  });
  const anyRunning = useNotebookStore((s) =>
    Array.from(s.cellStates.values()).some((st) => st === "running")
  );
  const collapsed = useNotebookStore((s) =>
    s.cellCollapsed.get(cellId) ?? false
  );

  const runCell = useNotebookStore((s) => s.runCell);
  const runFromCell = useNotebookStore((s) => s.runFromCell);
  const runToCell = useNotebookStore((s) => s.runToCell);
  const updateCellSource = useNotebookStore((s) => s.updateCellSource);
  const deleteCell = useNotebookStore((s) => s.deleteCell);
  const moveCell = useNotebookStore((s) => s.moveCell);
  const toggleCellCollapsed = useNotebookStore((s) => s.toggleCellCollapsed);

  const onSourceChange = useCallback(
    (value: string) => updateCellSource(cellId, value),
    [cellId, updateCellSource],
  );

  if (!cell) return null;

  const runnable = RUNNABLE[cell.type];
  const isRunning = runState === "running";
  // "Stale": an *upstream* cell ran more recently than this one.
  // Concretely: this cell is positioned AFTER the most-recently-run
  // cell, AND has its own output to be stale about (state === "done"
  // or "error"). Idle cells and currently-running cells aren't stale,
  // and non-runnable types (markdown) never participate.
  const isStale = runnable &&
    lastRunIndex >= 0 &&
    cellIndex > lastRunIndex &&
    (runState === "done" || runState === "error");

  return (
    <Card className={styles.card} appearance="filled-alternative">
      <div className={styles.toolbar}>
        <Button
          size="small"
          appearance="subtle"
          icon={collapsed
            ? <ChevronRight16Regular />
            : <ChevronDown16Regular />}
          aria-label={collapsed ? "Expand cell" : "Collapse cell"}
          aria-expanded={!collapsed}
          title={collapsed ? "Expand cell" : "Collapse cell"}
          onClick={() => toggleCellCollapsed(cellId)}
        />
        <span
          className={indexCircleClass(styles, cell.type)}
          aria-label={`Cell ${cellIndex + 1}`}
          title={`Cell ${cellIndex + 1}`}
        >
          {cellIndex + 1}
        </span>
        <Caption1 className={styles.typeBadge}>
          {TYPE_LABEL[cell.type]}
        </Caption1>
        {runnable && (
          <Menu positioning="below-start">
            <MenuTrigger disableButtonEnhancement>
              {(triggerProps) => (
                <SplitButton
                  size="small"
                  appearance="subtle"
                  icon={isRunning ? <Spinner size="tiny" /> : <Play16Regular />}
                  disabled={isRunning || anyRunning}
                  menuButton={triggerProps}
                  primaryActionButton={{
                    onClick: () => {
                      void runCell(eigen, cell);
                    },
                  }}
                >
                  Run
                </SplitButton>
              )}
            </MenuTrigger>
            <MenuPopover>
              <MenuList>
                <MenuItem
                  icon={<Play16Regular />}
                  onClick={() => {
                    void runCell(eigen, cell);
                  }}
                >
                  Run
                </MenuItem>
                <MenuItem
                  onClick={() => {
                    void runFromCell(eigen, cellId);
                  }}
                >
                  Run from here…
                </MenuItem>
                <MenuItem
                  onClick={() => {
                    void runToCell(eigen, cellId);
                  }}
                >
                  Run to here…
                </MenuItem>
              </MenuList>
            </MenuPopover>
          </Menu>
        )}
        {isStale && (
          <Tooltip
            content={`Cell ${
              lastRunIndex + 1
            } ran more recently. This cell may need to be re-run.`}
            relationship="description"
          >
            <Caption1 className={styles.staleHint}>stale</Caption1>
          </Tooltip>
        )}
        <div className={styles.spacer} />
        <div className={styles.rightCluster}>
          <Button
            size="small"
            appearance="subtle"
            icon={<ArrowUp16Regular />}
            aria-label="Move cell up"
            title="Move cell up"
            onClick={() => moveCell(cellId, "up")}
          />
          <Button
            size="small"
            appearance="subtle"
            icon={<ArrowDown16Regular />}
            aria-label="Move cell down"
            title="Move cell down"
            onClick={() => moveCell(cellId, "down")}
          />
          <Button
            size="small"
            appearance="subtle"
            icon={<Delete16Regular />}
            aria-label="Delete cell"
            title="Delete cell"
            onClick={() => deleteCell(cellId)}
          />
        </div>
      </div>
      {!collapsed && (
        <div className={styles.body}>
          {cell.type === "markdown"
            ? <MarkdownCell source={cell.source} onChange={onSourceChange} />
            : cell.type === "program-run"
            ? <ProgramRunCellEditor cellId={cell.id} cell={cell} />
            : cell.type === "chart"
            ? <ChartCellEditor cellId={cell.id} cell={cell} />
            : (
              <CodeMirrorEditor
                source={cell.source}
                cellType={cell.type}
                readOnly={false}
                onChange={onSourceChange}
              />
            )}
          {output && <CellOutputView output={output} cellId={cellId} />}
        </div>
      )}
    </Card>
  );
}
