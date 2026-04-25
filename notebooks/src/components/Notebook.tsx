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
  Caption1,
  makeStyles,
  Subtitle1,
  tokens,
} from "@fluentui/react-components";
import type { NotebookJson } from "../persistence/notebook-format";
import { Cell } from "./Cell";

const useStyles = makeStyles({
  root: {
    maxWidth: "880px",
    margin: "0 auto",
    padding: `${tokens.spacingVerticalL} ${tokens.spacingHorizontalL}`,
  },
  header: {
    marginBottom: tokens.spacingVerticalL,
  },
  meta: {
    color: tokens.colorNeutralForeground3,
  },
});

export interface NotebookProps {
  notebook: NotebookJson;
}

export function Notebook({ notebook }: NotebookProps) {
  const styles = useStyles();
  const title = notebook.meta.title ?? "Untitled notebook";
  const modified = notebook.meta.modified
    ? `modified ${notebook.meta.modified}`
    : "";

  return (
    <div className={styles.root}>
      <div className={styles.header}>
        <Subtitle1 as="h1">{title}</Subtitle1>
        <Caption1 className={styles.meta}>
          {notebook.cells.length} cell{notebook.cells.length === 1 ? "" : "s"}
          {modified ? ` · ${modified}` : ""}
        </Caption1>
      </div>
      {notebook.cells.map((cell) => <Cell key={cell.id} cell={cell} />)}
    </div>
  );
}
