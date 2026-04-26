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

import { useState } from "react";
import {
  Button,
  Caption1,
  makeStyles,
  MessageBar,
  MessageBarBody,
  MessageBarTitle,
  Spinner,
  Subtitle1,
  tokens,
} from "@fluentui/react-components";
import {
  ArrowReset20Regular,
  GlobeArrowUp20Regular,
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
  iri: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    wordBreak: "break-all",
  },
});

export interface NotebookProps {
  notebook: NotebookJson;
}

type PublishState =
  | { kind: "idle" }
  | { kind: "publishing" }
  | { kind: "success"; notebookIri: string; cellCount: number }
  | { kind: "error"; message: string };

export function Notebook({ notebook }: NotebookProps) {
  const styles = useStyles();
  const eigen = useEigen();
  const runAll = useNotebookStore((s) => s.runAll);
  const resetOutputs = useNotebookStore((s) => s.resetOutputs);
  const anyRunning = useNotebookStore((s) =>
    Array.from(s.cellStates.values()).some((st) => st === "running")
  );

  const [publish, setPublish] = useState<PublishState>({ kind: "idle" });
  const isPublishing = publish.kind === "publishing";

  const onPublish = async () => {
    setPublish({ kind: "publishing" });
    try {
      const { publish: result, load } = await eigen.publishNotebook(notebook);
      if (!load.success) {
        setPublish({
          kind: "error",
          message: load.errors.map((e) => e.message).join("; ") ||
            "publish failed (no error message)",
        });
        return;
      }
      setPublish({
        kind: "success",
        notebookIri: result.notebookIri,
        cellCount: result.cellIris.length,
      });
    } catch (err) {
      setPublish({
        kind: "error",
        message: err instanceof Error ? err.message : String(err),
      });
    }
  };

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
            disabled={anyRunning || isPublishing}
            onClick={() => resetOutputs()}
          >
            Reset
          </Button>
          <Button
            size="small"
            appearance="subtle"
            icon={isPublishing ? <Spinner size="tiny" /> : <GlobeArrowUp20Regular />}
            disabled={anyRunning || isPublishing}
            onClick={() => {
              void onPublish();
            }}
          >
            Publish
          </Button>
          <Button
            size="small"
            appearance="primary"
            icon={<PlayMultiple16Regular />}
            disabled={anyRunning || isPublishing}
            onClick={() => {
              void runAll(eigen, notebook.cells);
            }}
          >
            Run all
          </Button>
        </div>
        {publish.kind === "success" && (
          <MessageBar intent="success">
            <MessageBarBody>
              <MessageBarTitle>Notebook published</MessageBarTitle>
              <div>
                {publish.cellCount} cell{publish.cellCount === 1 ? "" : "s"} ·
                {" "}
                <span className={styles.iri}>{publish.notebookIri}</span>
              </div>
            </MessageBarBody>
          </MessageBar>
        )}
        {publish.kind === "error" && (
          <MessageBar intent="error">
            <MessageBarBody>
              <MessageBarTitle>Publish failed</MessageBarTitle>
              <div className={styles.iri}>{publish.message}</div>
            </MessageBarBody>
          </MessageBar>
        )}
      </div>
      {notebook.cells.map((cell) => <Cell key={cell.id} cell={cell} />)}
    </div>
  );
}
