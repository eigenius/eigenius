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
 * D34 §8.2 — Create tag dialog. Opens from two surfaces:
 *
 * - The Tags rail panel's `+ Create tag…` button. `defaultLayerId`
 *   is omitted; the user picks a branch tip from the dropdown (or
 *   pastes a hex LayerId).
 * - The History panel's per-layer detail panel's `+ Create tag…`
 *   button. `defaultLayerId` is the selected layer; the dropdown
 *   defaults to that id and the user only types a name.
 *
 * Tags are immutable (D34 §G.2): there is no retarget surface; the
 * kernel rejects re-using a name with `already_exists`, which we
 * surface as a friendly inline hint so the user can retry with a
 * different name without losing context.
 */

import { useEffect, useMemo, useState } from "react";
import {
  Button,
  Caption1,
  Combobox,
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
  Option,
  tokens,
} from "@fluentui/react-components";
import { useEigen } from "../../runtime/EigenProvider";
import { useNotebookStore } from "../../runtime/notebookStore";

const useStyles = makeStyles({
  surface: {
    width: "min(560px, 95vw)",
    maxWidth: "none",
  },
  body: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalM,
  },
  layerInput: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
  },
});

export interface CreateTagDialogProps {
  open: boolean;
  onClose: () => void;
  onCreated?: (name: string) => void;
  /** Pre-fill the layer-id field; renders the layer source as a
   *  read-only context line. Omit to let the user pick. */
  defaultLayerId?: string;
}

/**
 * Two ways to specify the target: pick a branch tip from the cached
 * `branches` list, or paste an explicit hex LayerId. The "explicit"
 * case is what the History panel pre-fills.
 */
type Target =
  | { kind: "branch"; branch: string }
  | { kind: "explicit"; layerId: string };

export function CreateTagDialog({
  open,
  onClose,
  onCreated,
  defaultLayerId,
}: CreateTagDialogProps) {
  const styles = useStyles();
  const eigen = useEigen();
  const branches = useNotebookStore((s) => s.branches);
  const activeBranch = useNotebookStore((s) => s.activeBranch);

  const initialTarget = useMemo<Target>(() => {
    if (defaultLayerId) return { kind: "explicit", layerId: defaultLayerId };
    return { kind: "branch", branch: activeBranch };
  }, [defaultLayerId, activeBranch]);

  const [name, setName] = useState("");
  const [target, setTarget] = useState<Target>(initialTarget);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Reset on each open so a previous dismissal doesn't leak state.
  useEffect(() => {
    if (open) {
      setName("");
      setTarget(initialTarget);
      setBusy(false);
      setError(null);
    }
  }, [open, initialTarget]);

  const branchOptions = useMemo(() => branches ?? [], [branches]);

  const canSubmit = name.trim().length > 0 &&
    (target.kind === "branch"
      ? target.branch.length > 0
      : target.layerId.trim().length === 64);

  const onCreate = async () => {
    setBusy(true);
    setError(null);
    try {
      // Resolve to a concrete hex LayerId at submit time.
      let layerId: string;
      if (target.kind === "branch") {
        const cached = branches?.find((b) => b.name === target.branch);
        if (cached) {
          layerId = cached.headLayer;
        } else {
          const resp = await eigen.getBranch(target.branch);
          if (!resp.found) {
            setError(`branch ${target.branch} not found`);
            setBusy(false);
            return;
          }
          layerId = resp.headLayer;
        }
      } else {
        layerId = target.layerId.trim();
      }
      const resp = await eigen.createTag(name.trim(), layerId);
      if (!resp.success) {
        // The kernel populates `already_exists` only on the name
        // collision path; everything else surfaces as a free-form
        // error string.
        setError(
          resp.alreadyExists
            ? `A tag named "${name.trim()}" already exists. Tags are immutable — choose a different name, or delete the existing tag first.`
            : resp.error || "tag creation failed",
        );
        setBusy(false);
        return;
      }
      onCreated?.(name.trim());
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
          <DialogTitle>Create tag</DialogTitle>
          <DialogContent className={styles.body}>
            <Field label="Name" required>
              <Input
                value={name}
                onChange={(_e, data) => setName(data.value)}
                placeholder="release-v1"
                disabled={busy}
                autoFocus
              />
              <Caption1>
                Letters, digits, <code>-</code>,{" "}
                <code>_</code>. Tags are immutable — pick a name you won't want
                to retarget.
              </Caption1>
            </Field>

            <Field label="Target layer" required>
              {defaultLayerId
                ? (
                  <>
                    <Input
                      className={styles.layerInput}
                      value={target.kind === "explicit" ? target.layerId : ""}
                      readOnly
                      disabled={busy}
                    />
                    <Caption1>
                      Pre-filled from the History panel selection.
                    </Caption1>
                  </>
                )
                : (
                  <>
                    <Combobox
                      value={target.kind === "branch" ? target.branch : ""}
                      selectedOptions={target.kind === "branch"
                        ? [target.branch]
                        : []}
                      onOptionSelect={(_e, data) => {
                        if (data.optionValue) {
                          setTarget({
                            kind: "branch",
                            branch: data.optionValue,
                          });
                        }
                      }}
                      placeholder={branchOptions.length === 0
                        ? "(no branches available)"
                        : "Select a branch"}
                      disabled={busy || branchOptions.length === 0}
                    >
                      {branchOptions.map((b) => (
                        <Option key={b.name} value={b.name}>
                          {b.name}
                        </Option>
                      ))}
                    </Combobox>
                    <Caption1>
                      Tagging the current head of the chosen branch. Use "Create
                      tag" from the History panel to tag a specific older layer.
                    </Caption1>
                  </>
                )}
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
              onClick={() => void onCreate()}
              disabled={!canSubmit || busy}
            >
              {busy ? "Creating…" : "Create"}
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
