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
 * Confirmation modal for the Branches panel's Delete action.
 *
 * Delete is irreversible from the user's perspective — the branch ref
 * is gone immediately and layers reachable only through that ref
 * become GC-eligible (D23 §5.5). A typed confirmation guards against
 * trigger-pull misclicks. The kernel's `protected` rejection (for
 * `main`) is surfaced verbatim if it ever fires (the panel already
 * disables the action; this is belt-and-suspenders).
 */

import { useEffect, useState } from "react";
import {
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
  MessageBar,
  MessageBarBody,
  tokens,
} from "@fluentui/react-components";
import type { BranchInfo } from "@eigenius/client";
import { useEigen } from "../../runtime/EigenProvider";
import { useNotebookStore } from "../../runtime/notebookStore";

const useStyles = makeStyles({
  surface: {
    width: "min(480px, 95vw)",
    maxWidth: "none",
  },
  body: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalM,
  },
  branchName: {
    fontFamily: tokens.fontFamilyMonospace,
    fontWeight: tokens.fontWeightSemibold,
  },
  tip: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground3,
  },
});

export interface DeleteBranchDialogProps {
  target: BranchInfo;
  open: boolean;
  onClose: () => void;
  onDeleted?: (name: string) => void;
}

export function DeleteBranchDialog({
  target,
  open,
  onClose,
  onDeleted,
}: DeleteBranchDialogProps) {
  const styles = useStyles();
  const eigen = useEigen();
  const refreshBranches = useNotebookStore((s) => s.refreshBranches);

  const [confirmText, setConfirmText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      setConfirmText("");
      setBusy(false);
      setError(null);
    }
  }, [open]);

  const canDelete = confirmText.trim() === target.name && !busy;

  const onConfirm = async () => {
    setBusy(true);
    setError(null);
    try {
      const resp = await eigen.deleteBranch(target.name, { force: false });
      if (!resp.success) {
        setError(resp.error || "delete failed");
        setBusy(false);
        return;
      }
      // `success: true, deleted: false` is the idempotent path —
      // branch didn't exist (or already gone). Either way the
      // post-condition the user wanted holds, so treat the same.
      await refreshBranches(eigen);
      onDeleted?.(target.name);
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setBusy(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(_e, data) => {
        if (!data.open && !busy) onClose();
      }}
    >
      <DialogSurface className={styles.surface}>
        <DialogBody>
          <DialogTitle>Delete branch?</DialogTitle>
          <DialogContent className={styles.body}>
            <div>
              You are about to delete{" "}
              <span className={styles.branchName}>{target.name}</span>{" "}
              <span className={styles.tip}>
                ({shortHash(target.headLayer)})
              </span>.
            </div>
            <Caption1>
              The branch ref is removed immediately. Layers reachable only
              through this branch become GC-eligible on the next pass — they
              can't be recovered without re-importing the chain.
            </Caption1>
            <Field
              label={
                <span>
                  Type <strong>{target.name}</strong> to confirm:
                </span>
              }
            >
              <Input
                value={confirmText}
                onChange={(_e, data) => setConfirmText(data.value)}
                placeholder={target.name}
                disabled={busy}
                autoFocus
              />
            </Field>
            {error && (
              <MessageBar intent="error">
                <MessageBarBody>{error}</MessageBarBody>
              </MessageBar>
            )}
          </DialogContent>
          <DialogActions>
            <Button appearance="secondary" onClick={onClose} disabled={busy}>
              Cancel
            </Button>
            <Button
              appearance="primary"
              onClick={() => void onConfirm()}
              disabled={!canDelete}
            >
              {busy ? "Deleting…" : "Delete"}
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}

function shortHash(hex: string): string {
  if (hex.length <= 10) return hex;
  return `${hex.slice(0, 4)}…${hex.slice(-4)}`;
}
