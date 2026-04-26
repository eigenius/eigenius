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
  Card,
  Caption1,
  makeStyles,
  Spinner,
  tokens,
} from "@fluentui/react-components";
import {
  ArrowDown16Regular,
  ArrowUp16Regular,
  Delete16Regular,
  Play16Regular,
} from "@fluentui/react-icons";
import type { CellType } from "../persistence/notebook-format";
import { useEigen } from "../runtime/EigenProvider";
import { useNotebookStore } from "../runtime/notebookStore";
import { CodeMirrorEditor } from "./editors/CodeMirrorEditor";
import { MarkdownCell } from "./cells/MarkdownCell";
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
  spacer: {
    flex: 1,
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
};

const TYPE_LABEL: Record<CellType, string> = {
  markdown: "Markdown",
  esl: "ESL",
  eigenql: "EigenQL",
  typescript: "TypeScript",
};

/**
 * Generic cell wrapper — Fluent `Card` shell with a toolbar (type
 * dropdown, Run, more-actions menu) and a body that delegates to the
 * Markdown renderer or to a CodeMirror editor sized for the cell type.
 */
export function Cell({ cellId }: CellProps) {
  const styles = useStyles();
  const eigen = useEigen();
  const cell = useNotebookStore((s) => s.cells.find((c) => c.id === cellId));
  const runState = useNotebookStore(
    (s) => s.cellStates.get(cellId) ?? "idle",
  );
  const output = useNotebookStore((s) => s.cellOutputs.get(cellId));

  const runCell = useNotebookStore((s) => s.runCell);
  const updateCellSource = useNotebookStore((s) => s.updateCellSource);
  const deleteCell = useNotebookStore((s) => s.deleteCell);
  const moveCell = useNotebookStore((s) => s.moveCell);

  const onSourceChange = useCallback(
    (value: string) => updateCellSource(cellId, value),
    [cellId, updateCellSource],
  );

  if (!cell) return null;

  const runnable = RUNNABLE[cell.type];
  const isRunning = runState === "running";

  return (
    <Card className={styles.card} appearance="filled-alternative">
      <div className={styles.toolbar}>
        <Caption1 className={styles.typeBadge}>{TYPE_LABEL[cell.type]}</Caption1>
        {runnable && (
          <Button
            size="small"
            appearance="subtle"
            icon={isRunning ? <Spinner size="tiny" /> : <Play16Regular />}
            disabled={isRunning}
            onClick={() => {
              void runCell(eigen, cell);
            }}
          >
            Run
          </Button>
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
      <div className={styles.body}>
        {cell.type === "markdown"
          ? <MarkdownCell source={cell.source} onChange={onSourceChange} />
          : (
            <CodeMirrorEditor
              source={cell.source}
              cellType={cell.type}
              readOnly={false}
              onChange={onSourceChange}
            />
          )}
        {output && <CellOutputView output={output} />}
      </div>
    </Card>
  );
}

