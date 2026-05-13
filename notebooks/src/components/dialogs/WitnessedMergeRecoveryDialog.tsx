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
 * D34 §6.2 — Witnessed-merge recovery dialog.
 *
 * Opens from a cell whose commit response carried
 * `MergeOutcome.NEEDS_WITNESSED_MERGE`. The kernel has already left
 * the orphan layer on disk (only reachable by its id) and the branch
 * pointing at someone else's commit. The user picks one of three
 * recovery paths:
 *
 * 1. **Save as a sibling branch** — calls `CreateBranch(name,
 *    orphan_layer_id)` so the work stays reachable. Default name is
 *    the D23 §5.4.4 auto-YYYY-MM-DD-HHMM convention; the user can
 *    override.
 * 2. **Pin and rebase** — sets the read-pin to the branch's current
 *    head and clears cell outputs. The user re-runs the cells from
 *    the top against the new head, producing a new commit attempt.
 * 3. **Discard** — no kernel call. The orphan layer becomes
 *    GC-eligible the moment nothing references it. Destructive;
 *    that's why this dialog exists.
 *
 * Phase 15 (witnessed-merge resolution) is *not* in scope here — the
 * dialog explains that and offers the recovery paths above.
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
  Radio,
  RadioGroup,
  tokens,
} from "@fluentui/react-components";
import { useEigen } from "../../runtime/EigenProvider";
import { useNotebookStore } from "../../runtime/notebookStore";

const useStyles = makeStyles({
  surface: {
    width: "min(640px, 95vw)",
    maxWidth: "none",
  },
  body: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalM,
  },
  conflictList: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    margin: `${tokens.spacingVerticalXS} 0 0 ${tokens.spacingHorizontalM}`,
    padding: 0,
    maxHeight: "180px",
    overflowY: "auto",
  },
  ids: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground3,
  },
  optionLabel: {
    fontWeight: tokens.fontWeightSemibold,
  },
  optionHint: {
    color: tokens.colorNeutralForeground3,
    marginTop: tokens.spacingVerticalXXS,
  },
  siblingNameField: {
    marginTop: tokens.spacingVerticalS,
    marginLeft: tokens.spacingHorizontalXL,
  },
});

type Recovery = "sibling" | "rebase" | "discard";

export interface WitnessedMergeRecoveryDialogProps {
  open: boolean;
  onClose: () => void;
  /** The orphan layer's hex id (from `merge.orphan_layer_id`). */
  orphanLayerId: string;
  /** The branch's current head (where the user lost the race to). */
  currentHead?: string;
  /** Conflicting IRIs surfaced by the kernel. */
  conflictingIris: readonly string[];
}

export function WitnessedMergeRecoveryDialog({
  open,
  onClose,
  orphanLayerId,
  currentHead,
  conflictingIris,
}: WitnessedMergeRecoveryDialogProps) {
  const styles = useStyles();
  const eigen = useEigen();
  const refreshBranches = useNotebookStore((s) => s.refreshBranches);
  const setReadPin = useNotebookStore((s) => s.setReadPin);
  const resetOutputs = useNotebookStore((s) => s.resetOutputs);

  const [choice, setChoice] = useState<Recovery>("sibling");
  const [siblingName, setSiblingName] = useState<string>(defaultSiblingName);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Reset on each open so leftover state from a previous dismissal
  // doesn't ambush the user.
  useEffect(() => {
    if (open) {
      setChoice("sibling");
      setSiblingName(defaultSiblingName());
      setBusy(false);
      setError(null);
    }
  }, [open]);

  const onConfirm = async () => {
    setBusy(true);
    setError(null);
    try {
      if (choice === "sibling") {
        const name = siblingName.trim();
        if (name.length === 0) {
          setError("Sibling branch name is required.");
          setBusy(false);
          return;
        }
        const resp = await eigen.createBranch(name, {
          fromLayer: orphanLayerId,
        });
        if (!resp.success) {
          setError(resp.error || "branch creation failed");
          setBusy(false);
          return;
        }
        await refreshBranches(eigen);
      } else if (choice === "rebase") {
        if (currentHead) {
          // Pin reads to the new head so the user can inspect what
          // beat them; they'll re-run the cells against branch tip
          // to produce a fresh commit attempt.
          setReadPin(currentHead);
        }
        resetOutputs();
      }
      // "discard" — no kernel call; the orphan becomes GC-eligible
      // when nothing references it. Just clears local outputs so
      // the user moves on.
      if (choice === "discard") {
        resetOutputs();
      }
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
          <DialogTitle>
            Conflict — branch advanced under your changes
          </DialogTitle>
          <DialogContent className={styles.body}>
            <div>
              Your commit raced against another commit to this branch, and the
              two contributions modify the same resources. Eigenius cannot merge
              automatically without witnessed-merge resolution (Phase 15, not
              yet available).
            </div>

            {conflictingIris.length > 0 && (
              <div>
                <Caption1>
                  Conflicting resources ({conflictingIris.length}):
                </Caption1>
                <ul className={styles.conflictList}>
                  {conflictingIris.map((iri) => <li key={iri}>{iri}</li>)}
                </ul>
              </div>
            )}

            <div className={styles.ids}>
              {currentHead && <div>Branch's current head: {currentHead}</div>}
              <div>
                Your unmerged work: {orphanLayerId}{" "}
                <Caption1 as="span">
                  (will be discarded if you don't save it)
                </Caption1>
              </div>
            </div>

            <Field label="Recovery options">
              <RadioGroup
                value={choice}
                onChange={(_e, data) => setChoice(data.value as Recovery)}
              >
                <Radio
                  value="sibling"
                  disabled={busy}
                  label={
                    <div>
                      <div className={styles.optionLabel}>
                        Save my work as a new sibling branch
                      </div>
                      <Caption1 className={styles.optionHint}>
                        Keeps the orphan layer reachable through a fresh branch
                        ref; you can come back to it later.
                      </Caption1>
                    </div>
                  }
                />
                {choice === "sibling" && (
                  <Field
                    className={styles.siblingNameField}
                    label="Sibling name"
                  >
                    <Input
                      value={siblingName}
                      onChange={(_e, data) => setSiblingName(data.value)}
                      disabled={busy}
                      placeholder={defaultSiblingName()}
                    />
                    <Caption1>
                      The <code>auto-</code>{" "}
                      prefix is the D23 §5.4.4 convention for "saved sibling on
                      conflict". You can override.
                    </Caption1>
                  </Field>
                )}
                <Radio
                  value="rebase"
                  disabled={busy || !currentHead}
                  label={
                    <div>
                      <div className={styles.optionLabel}>
                        Pin reads to the new head and re-run from the top
                      </div>
                      <Caption1 className={styles.optionHint}>
                        Sets the read-pin to the branch's current head so you
                        can see what beat you; cell outputs clear so you can
                        re-run.
                      </Caption1>
                    </div>
                  }
                />
                <Radio
                  value="discard"
                  disabled={busy}
                  label={
                    <div>
                      <div className={styles.optionLabel}>
                        Discard my work
                      </div>
                      <Caption1 className={styles.optionHint}>
                        Orphan layer stays on disk until the next GC pass, then
                        becomes irrecoverable.
                      </Caption1>
                    </div>
                  }
                />
              </RadioGroup>
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
              disabled={busy}
            >
              {busy ? "Working…" : "Continue"}
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}

/** D23 §5.4.4 "save as sibling" naming convention: `auto-YYYY-MM-DD-HHMM`. */
function defaultSiblingName(): string {
  const now = new Date();
  const pad = (n: number) => n.toString().padStart(2, "0");
  return [
    "auto",
    `${now.getUTCFullYear()}-${pad(now.getUTCMonth() + 1)}-${
      pad(now.getUTCDate())
    }`,
    `${pad(now.getUTCHours())}${pad(now.getUTCMinutes())}`,
  ].join("-");
}
