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
  makeStyles,
  Subtitle1,
  tokens,
} from "@fluentui/react-components";
import {
  ArrowReset20Regular,
  PlayMultiple16Regular,
} from "@fluentui/react-icons";
import type { NotebookJson } from "../persistence/notebook-format";
import { Cell } from "./Cell";
import { useEigen } from "../runtime/EigenProvider";
import { useNotebookStore } from "../runtime/notebookStore";

const useStyles = makeStyles({
  root: {
    maxWidth: "880px",
    margin: "0 auto",
    padding: `${tokens.spacingVerticalL} ${tokens.spacingHorizontalL}`,
  },
  header: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalS,
    marginBottom: tokens.spacingVerticalL,
  },
  toolbar: {
    display: "flex",
    flexWrap: "wrap",
    alignItems: "center",
    gap: tokens.spacingHorizontalS,
  },
  meta: {
    color: tokens.colorNeutralForeground3,
    flex: 1,
  },
});

export interface NotebookProps {
  notebook: NotebookJson;
}

export function Notebook({ notebook }: NotebookProps) {
  const styles = useStyles();
  const eigen = useEigen();
  const runAll = useNotebookStore((s) => s.runAll);
  const resetOutputs = useNotebookStore((s) => s.resetOutputs);
  const anyRunning = useNotebookStore((s) =>
    Array.from(s.cellStates.values()).some((st) => st === "running")
  );

  const title = notebook.meta.title ?? "Untitled notebook";
  const modified = notebook.meta.modified
    ? `modified ${notebook.meta.modified}`
    : "";

  return (
    <div className={styles.root}>
      <div className={styles.header}>
        <Subtitle1 as="h1">{title}</Subtitle1>
        <div className={styles.toolbar}>
          <Caption1 className={styles.meta}>
            {notebook.cells.length} cell{notebook.cells.length === 1 ? "" : "s"}
            {modified ? ` · ${modified}` : ""}
          </Caption1>
          <Button
            size="small"
            appearance="subtle"
            icon={<ArrowReset20Regular />}
            disabled={anyRunning}
            onClick={() => resetOutputs()}
          >
            Reset
          </Button>
          <Button
            size="small"
            appearance="primary"
            icon={<PlayMultiple16Regular />}
            disabled={anyRunning}
            onClick={() => {
              void runAll(eigen, notebook.cells);
            }}
          >
            Run all
          </Button>
        </div>
      </div>
      {notebook.cells.map((cell) => <Cell key={cell.id} cell={cell} />)}
    </div>
  );
}
