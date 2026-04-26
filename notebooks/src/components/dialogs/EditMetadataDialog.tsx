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
 * Modal for editing notebook metadata (D22 Phase 6 polish). Title is
 * required and surfaced as the headline display field; description
 * is optional but encouraged because it powers the published-notebook
 * search dialog. Auto-managed timestamps (created / modified) and
 * `eigenius_version` are shown read-only for context.
 *
 * Edits stage in local component state and only commit to the store
 * on Save, so Cancel actually cancels.
 */

import { useEffect, useState } from "react";
import {
  Body1,
  Button,
  Caption1,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Field,
  Input,
  makeStyles,
  Textarea,
  tokens,
} from "@fluentui/react-components";
import type { NotebookMetaJson } from "../../persistence/notebook-format";

const useStyles = makeStyles({
  surface: {
    width: "min(680px, 95vw)",
    maxWidth: "none",
  },
  body: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalM,
  },
  readonlyBlock: {
    display: "grid",
    gridTemplateColumns: "auto 1fr",
    columnGap: tokens.spacingHorizontalM,
    rowGap: tokens.spacingVerticalXS,
    padding: tokens.spacingVerticalS,
    background: tokens.colorNeutralBackground2,
    borderRadius: tokens.borderRadiusMedium,
  },
  readonlyLabel: {
    color: tokens.colorNeutralForeground3,
  },
  readonlyValue: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground2,
    wordBreak: "break-all",
  },
});

export interface EditMetadataDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  meta: NotebookMetaJson;
  onSave: (next: NotebookMetaJson) => void;
}

export function EditMetadataDialog(
  { open, onOpenChange, meta, onSave }: EditMetadataDialogProps,
) {
  const styles = useStyles();
  const [title, setTitle] = useState(meta.title);
  const [description, setDescription] = useState(meta.description ?? "");

  // Re-seed the staged values whenever the dialog re-opens.
  useEffect(() => {
    if (open) {
      setTitle(meta.title);
      setDescription(meta.description ?? "");
    }
  }, [open, meta.title, meta.description]);

  const titleInvalid = title.trim().length === 0;

  const onCommit = () => {
    if (titleInvalid) return;
    onSave({
      ...meta,
      title: title.trim(),
      description: description.length > 0 ? description : undefined,
    });
    onOpenChange(false);
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(_e, data) => onOpenChange(data.open)}
      modalType="modal"
    >
      <DialogSurface className={styles.surface}>
        <DialogBody>
          <DialogTitle>Edit notebook metadata</DialogTitle>
          <DialogContent>
            <div className={styles.body}>
              <Field
                label="Title"
                required
                validationState={titleInvalid ? "error" : "none"}
                validationMessage={titleInvalid
                  ? "Title is required"
                  : undefined}
              >
                <Input
                  value={title}
                  placeholder="Untitled notebook"
                  onChange={(_e, data) => setTitle(data.value)}
                />
              </Field>
              <Field
                label="Description"
                hint="Plain text. Surfaced in the Open dialog when the notebook is published."
              >
                <Textarea
                  value={description}
                  rows={4}
                  placeholder="What this notebook does, who it's for, what it depends on…"
                  onChange={(_e, data) => setDescription(data.value)}
                />
              </Field>
              {(meta.created || meta.modified || meta.eigenius_version) && (
                <div className={styles.readonlyBlock}>
                  {meta.created && (
                    <>
                      <Caption1 className={styles.readonlyLabel}>
                        created
                      </Caption1>
                      <Body1 className={styles.readonlyValue}>
                        {meta.created}
                      </Body1>
                    </>
                  )}
                  {meta.modified && (
                    <>
                      <Caption1 className={styles.readonlyLabel}>
                        modified
                      </Caption1>
                      <Body1 className={styles.readonlyValue}>
                        {meta.modified}
                      </Body1>
                    </>
                  )}
                  {meta.eigenius_version && (
                    <>
                      <Caption1 className={styles.readonlyLabel}>
                        eigenius version
                      </Caption1>
                      <Body1 className={styles.readonlyValue}>
                        {meta.eigenius_version}
                      </Body1>
                    </>
                  )}
                </div>
              )}
            </div>
          </DialogContent>
          <DialogActions>
            <Button appearance="secondary" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button
              appearance="primary"
              disabled={titleInvalid}
              onClick={onCommit}
            >
              Save
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
