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

import { useRef, useState } from "react";
import {
  Button,
  Caption1,
  Input,
  makeStyles,
  MessageBar,
  MessageBarActions,
  MessageBarBody,
  MessageBarTitle,
  Spinner,
  Subtitle1,
  tokens,
  Tooltip,
} from "@fluentui/react-components";
import {
  ArrowExport16Regular,
  ArrowImport16Regular,
  ArrowReset20Regular,
  ChevronDoubleDown16Regular,
  ChevronDoubleRight16Regular,
  Dismiss20Regular,
  DocumentAdd16Regular,
  Edit16Regular,
  FolderOpen16Regular,
  GlobeArrowUp20Regular,
  PlayMultiple16Regular,
} from "@fluentui/react-icons";
import { parseNotebook } from "../persistence/notebook-format";
import { Cell } from "./Cell";
import { CellInsertGap } from "./CellInsertGap";
import { EditMetadataDialog } from "./dialogs/EditMetadataDialog";
import { OpenPublishedDialog } from "./dialogs/OpenPublishedDialog";
import { useEigen } from "../runtime/EigenProvider";
import { useNotebookStore } from "../runtime/notebookStore";

const useStyles = makeStyles({
  // Outer fills its parent flex column. The notebook header is
  // fixed-height at top; the cell list is the only scroll surface.
  // WorkspaceShell sets the viewport bound; the notebook fills its
  // rail destination (`height: 100%`).
  root: {
    display: "flex",
    flexDirection: "column",
    height: "100%",
    overflow: "hidden",
  },
  header: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalS,
    flexShrink: 0,
    background: tokens.colorNeutralBackground1,
    borderBottom: `1px solid ${tokens.colorNeutralStroke2}`,
    padding: `${tokens.spacingVerticalM} ${tokens.spacingHorizontalL}`,
  },
  headerInner: {
    maxWidth: "880px",
    margin: "0 auto",
    width: "100%",
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalS,
  },
  cellsScroll: {
    flex: 1,
    minHeight: 0,
    overflowY: "auto",
  },
  cellsInner: {
    maxWidth: "880px",
    margin: "0 auto",
    padding: `${tokens.spacingVerticalL} ${tokens.spacingHorizontalL}`,
  },
  titleRow: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalS,
  },
  titleInput: {
    flex: 1,
    maxWidth: "560px",
  },
  description: {
    color: tokens.colorNeutralForeground3,
    whiteSpace: "pre-wrap",
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

type PublishState =
  | { kind: "idle" }
  | { kind: "publishing" }
  | { kind: "success"; notebookIri: string; cellCount: number }
  | { kind: "error"; message: string };

type LoadError = { kind: "error"; message: string } | { kind: "idle" };

export function Notebook() {
  const styles = useStyles();
  const eigen = useEigen();

  const meta = useNotebookStore((s) => s.meta);
  const cells = useNotebookStore((s) => s.cells);
  const updateMeta = useNotebookStore((s) => s.updateMeta);
  const exportNotebook = useNotebookStore((s) => s.exportNotebook);
  const loadNotebook = useNotebookStore((s) => s.loadNotebook);
  const newNotebook = useNotebookStore((s) => s.newNotebook);
  const markSaved = useNotebookStore((s) => s.markSaved);
  const runAll = useNotebookStore((s) => s.runAll);
  const resetOutputs = useNotebookStore((s) => s.resetOutputs);
  const setAllCellsCollapsed = useNotebookStore(
    (s) => s.setAllCellsCollapsed,
  );
  const anyRunning = useNotebookStore((s) =>
    Array.from(s.cellStates.values()).some((st) => st === "running")
  );
  // True when at least one cell is currently expanded (the default for
  // unset entries). Drives the smart toggle on the toolbar button.
  const anyExpanded = useNotebookStore((s) =>
    s.cells.some((c) => !(s.cellCollapsed.get(c.id) ?? false))
  );

  const [publish, setPublish] = useState<PublishState>({ kind: "idle" });
  const [loadError, setLoadError] = useState<LoadError>({ kind: "idle" });
  const [openDialogOpen, setOpenDialogOpen] = useState(false);
  const [editMetaOpen, setEditMetaOpen] = useState(false);
  const isPublishing = publish.kind === "publishing";

  const fileInputRef = useRef<HTMLInputElement | null>(null);

  const onSave = () => {
    const json = exportNotebook();
    json.meta = { ...json.meta, modified: new Date().toISOString() };
    // Update the in-memory state so the displayed timestamp matches the file.
    updateMeta({ modified: json.meta.modified });

    const slug = (json.meta.title || "notebook")
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/(^-|-$)/g, "") || "notebook";
    const filename = `${slug}.json`;

    const blob = new Blob([JSON.stringify(json, null, 2)], {
      type: "application/json",
    });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = filename;
    document.body.appendChild(link);
    link.click();
    document.body.removeChild(link);
    URL.revokeObjectURL(url);
    // The on-disk copy now matches the in-memory state — clear the
    // `●` unsaved-changes indicator until the next mutating edit.
    markSaved();
  };

  const onOpenClick = () => {
    fileInputRef.current?.click();
  };

  const onFilePicked = async (
    e: React.ChangeEvent<HTMLInputElement>,
  ) => {
    const file = e.target.files?.[0];
    e.target.value = ""; // allow picking the same file twice in a row
    if (!file) return;
    try {
      const text = await file.text();
      const raw = JSON.parse(text);
      const parsed = parseNotebook(raw);
      loadNotebook(parsed);
      setLoadError({ kind: "idle" });
      setPublish({ kind: "idle" });
    } catch (err) {
      setLoadError({
        kind: "error",
        message: err instanceof Error ? err.message : String(err),
      });
    }
  };

  const onPublish = async () => {
    setPublish({ kind: "publishing" });
    try {
      const { publish: result, load } = await eigen.publishNotebook(
        exportNotebook(),
      );
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

  const modified = meta.modified ? `modified ${meta.modified}` : "";
  const titleEmpty = meta.title.trim().length === 0;

  return (
    <div className={styles.root}>
      <input
        ref={fileInputRef}
        type="file"
        accept="application/json,.json"
        style={{ display: "none" }}
        onChange={onFilePicked}
      />
      <div className={styles.header}>
        <div className={styles.headerInner}>
          <div className={styles.titleRow}>
            <Subtitle1 as="h1" style={{ minWidth: "fit-content" }}>
              Notebook:
            </Subtitle1>
            <Input
              className={styles.titleInput}
              size="medium"
              placeholder="Untitled notebook"
              required
              value={meta.title}
              onChange={(_e, data) => updateMeta({ title: data.value })}
            />
            <Tooltip content="Edit notebook metadata" relationship="label">
              <Button
                size="small"
                appearance="subtle"
                icon={<Edit16Regular />}
                aria-label="Edit notebook metadata"
                onClick={() => setEditMetaOpen(true)}
              />
            </Tooltip>
          </div>
          {meta.description && (
            <Caption1 className={styles.description}>
              {meta.description}
            </Caption1>
          )}
          <div className={styles.toolbar}>
            <Caption1 className={styles.meta}>
              {cells.length} cell{cells.length === 1 ? "" : "s"}
              {modified ? ` · ${modified}` : ""}
            </Caption1>
            <Button
              size="small"
              appearance="subtle"
              icon={<DocumentAdd16Regular />}
              disabled={anyRunning || isPublishing}
              onClick={newNotebook}
            >
              New
            </Button>
            <Button
              size="small"
              appearance="subtle"
              icon={<FolderOpen16Regular />}
              disabled={anyRunning || isPublishing}
              onClick={() => setOpenDialogOpen(true)}
            >
              Open…
            </Button>
            <Button
              size="small"
              appearance="subtle"
              icon={<ArrowImport16Regular />}
              disabled={anyRunning || isPublishing}
              onClick={onOpenClick}
            >
              Import…
            </Button>
            <Tooltip
              content={titleEmpty
                ? "Set a title before exporting"
                : "Download the notebook as a JSON file"}
              relationship="label"
            >
              <Button
                size="small"
                appearance="subtle"
                icon={<ArrowExport16Regular />}
                disabled={anyRunning || isPublishing || titleEmpty}
                onClick={onSave}
              >
                Export…
              </Button>
            </Tooltip>
            <Button
              size="small"
              appearance="subtle"
              icon={anyExpanded
                ? <ChevronDoubleRight16Regular />
                : <ChevronDoubleDown16Regular />}
              disabled={cells.length === 0}
              onClick={() => setAllCellsCollapsed(anyExpanded)}
            >
              {anyExpanded ? "Collapse all" : "Expand all"}
            </Button>
            <Button
              size="small"
              appearance="subtle"
              icon={<ArrowReset20Regular />}
              disabled={anyRunning || isPublishing}
              onClick={() => resetOutputs()}
            >
              Reset
            </Button>
            <Tooltip
              content={titleEmpty
                ? "Set a title before publishing"
                : "Publish the notebook into the active layer chain"}
              relationship="label"
            >
              <Button
                size="small"
                appearance="subtle"
                icon={isPublishing
                  ? <Spinner size="tiny" />
                  : <GlobeArrowUp20Regular />}
                disabled={anyRunning || isPublishing || titleEmpty}
                onClick={() => {
                  void onPublish();
                }}
              >
                Publish
              </Button>
            </Tooltip>
            <Button
              size="small"
              appearance="primary"
              icon={<PlayMultiple16Regular />}
              disabled={anyRunning || isPublishing}
              onClick={() => {
                void runAll(eigen);
              }}
            >
              Run all
            </Button>
          </div>
          {loadError.kind === "error" && (
            <MessageBar intent="error">
              <MessageBarBody>
                <MessageBarTitle>Could not load notebook</MessageBarTitle>
                <div>{loadError.message}</div>
              </MessageBarBody>
              <MessageBarActions
                containerAction={
                  <Button
                    appearance="transparent"
                    icon={<Dismiss20Regular />}
                    aria-label="Dismiss"
                    onClick={() => setLoadError({ kind: "idle" })}
                  />
                }
              />
            </MessageBar>
          )}
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
              <MessageBarActions
                containerAction={
                  <Button
                    appearance="transparent"
                    icon={<Dismiss20Regular />}
                    aria-label="Dismiss"
                    onClick={() => setPublish({ kind: "idle" })}
                  />
                }
              />
            </MessageBar>
          )}
          {publish.kind === "error" && (
            <MessageBar intent="error">
              <MessageBarBody>
                <MessageBarTitle>Publish failed</MessageBarTitle>
                <div className={styles.iri}>{publish.message}</div>
              </MessageBarBody>
              <MessageBarActions
                containerAction={
                  <Button
                    appearance="transparent"
                    icon={<Dismiss20Regular />}
                    aria-label="Dismiss"
                    onClick={() => setPublish({ kind: "idle" })}
                  />
                }
              />
            </MessageBar>
          )}
        </div>
      </div>
      <div className={styles.cellsScroll}>
        <div className={styles.cellsInner}>
          <CellInsertGap afterCellId={null} />
          {cells.map((cell) => (
            <div key={cell.id}>
              <Cell cellId={cell.id} />
              <CellInsertGap afterCellId={cell.id} />
            </div>
          ))}
          {cells.length === 0 && (
            <Caption1>
              Empty notebook. Hover the line above to insert your first cell.
            </Caption1>
          )}
        </div>
      </div>
      <OpenPublishedDialog
        open={openDialogOpen}
        onOpenChange={setOpenDialogOpen}
        onPicked={(json) => {
          loadNotebook(json);
          setLoadError({ kind: "idle" });
          setPublish({ kind: "idle" });
        }}
      />
      <EditMetadataDialog
        open={editMetaOpen}
        onOpenChange={setEditMetaOpen}
        meta={meta}
        onSave={(next) => updateMeta(next)}
      />
    </div>
  );
}
