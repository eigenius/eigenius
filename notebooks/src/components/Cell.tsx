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

import {
  Button,
  Caption1,
  Card,
  CardHeader,
  makeStyles,
  Spinner,
  tokens,
} from "@fluentui/react-components";
import { Play16Regular } from "@fluentui/react-icons";
import type { CellJson } from "../persistence/notebook-format";
import { useEigen } from "../runtime/EigenProvider";
import { useNotebookStore } from "../runtime/notebookStore";
import { MarkdownCell } from "./cells/MarkdownCell";
import { ESLCell } from "./cells/ESLCell";
import { EigenQLCell } from "./cells/EigenQLCell";
import { TypeScriptCell } from "./cells/TypeScriptCell";
import { CellOutputView } from "./output/CellOutputView";

const useStyles = makeStyles({
  card: {
    marginBottom: tokens.spacingVerticalM,
  },
  body: {
    padding: tokens.spacingVerticalS,
  },
  header: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalS,
  },
  typeBadge: {
    color: tokens.colorNeutralForeground3,
    textTransform: "uppercase",
    letterSpacing: "0.04em",
  },
  spacer: {
    flex: 1,
  },
});

export interface CellProps {
  cell: CellJson;
}

const TYPE_LABEL: Record<CellJson["type"], string> = {
  markdown: "Markdown",
  esl: "ESL",
  eigenql: "EigenQL",
  typescript: "TypeScript",
};

const RUNNABLE: Record<CellJson["type"], boolean> = {
  markdown: false,
  esl: true,
  eigenql: true,
  // TypeScript cell execution is a Phase 4 deliverable; the button
  // stays hidden until the sandbox lands.
  typescript: false,
};

/**
 * Generic cell wrapper — Fluent `Card` shell with a type-label header,
 * a Run button for executable cell types, and a body that delegates
 * to one of the four per-type source renderers (followed by the cell's
 * output, when present).
 */
export function Cell({ cell }: CellProps) {
  const styles = useStyles();
  const eigen = useEigen();
  const runState = useNotebookStore((s) => s.cellStates.get(cell.id) ?? "idle");
  const output = useNotebookStore((s) => s.cellOutputs.get(cell.id));
  const runCell = useNotebookStore((s) => s.runCell);

  const runnable = RUNNABLE[cell.type];
  const isRunning = runState === "running";

  return (
    <Card className={styles.card} appearance="filled-alternative">
      <CardHeader
        header={
          <div className={styles.header}>
            <Caption1 className={styles.typeBadge}>
              {TYPE_LABEL[cell.type]}
            </Caption1>
            <div className={styles.spacer} />
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
          </div>
        }
      />
      <div className={styles.body}>
        {cell.type === "markdown" && <MarkdownCell source={cell.source} />}
        {cell.type === "esl" && <ESLCell source={cell.source} />}
        {cell.type === "eigenql" && <EigenQLCell source={cell.source} />}
        {cell.type === "typescript" && <TypeScriptCell source={cell.source} />}
        {output && <CellOutputView output={output} />}
      </div>
    </Card>
  );
}
