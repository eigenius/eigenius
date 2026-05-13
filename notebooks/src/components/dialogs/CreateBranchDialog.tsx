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
 * D34 §4.3 — Create branch dialog. Opens from the BranchBar's picker
 * footer.
 *
 * The user picks (a) a name, (b) a starting layer (either the current
 * head of an existing branch, or an explicit layer id), and (c)
 * whether to switch to the new branch after creation. The dialog
 * does no client-side validation beyond non-empty name + non-empty
 * starting layer — name shape is the kernel's contract
 * (`[A-Za-z0-9_-]+`, no `auto-` prefix, max 256 chars) and we
 * surface the kernel's `error` verbatim on rejection so the user
 * sees the actual reason.
 */

import { useEffect, useMemo, useState } from "react";
import {
  Button,
  Caption1,
  Checkbox,
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
  branchHead: {
    color: tokens.colorNeutralForeground3,
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    marginLeft: tokens.spacingHorizontalS,
  },
});

export interface CreateBranchDialogProps {
  open: boolean;
  onClose: () => void;
  onCreated?: (name: string) => void;
}

/**
 * Discriminated union over the "Start from" radio options. The
 * `existing` shape carries the chosen branch name so we can look up
 * its current head at create time (rather than at open time, which
 * could be stale by the time the user clicks Create).
 */
type StartFrom =
  | { kind: "existing"; branch: string }
  | { kind: "explicit"; layerId: string };

export function CreateBranchDialog({
  open,
  onClose,
  onCreated,
}: CreateBranchDialogProps) {
  const styles = useStyles();
  const eigen = useEigen();
  const branches = useNotebookStore((s) => s.branches);
  const activeBranch = useNotebookStore((s) => s.activeBranch);
  const createBranch = useNotebookStore((s) => s.createBranch);

  const [name, setName] = useState("");
  const [startFrom, setStartFrom] = useState<StartFrom>({
    kind: "existing",
    branch: activeBranch,
  });
  const [switchAfter, setSwitchAfter] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Reset transient state every time the dialog opens — leftover
  // values from a previous interaction would be confusing.
  useEffect(() => {
    if (open) {
      setName("");
      setStartFrom({ kind: "existing", branch: activeBranch });
      setSwitchAfter(true);
      setBusy(false);
      setError(null);
    }
  }, [open, activeBranch]);

  const startBranchOptions = useMemo(() => {
    // Always offer the active branch even when the SDK's branches
    // list isn't loaded (in-memory mode). Other branches are
    // additive on top.
    const seen = new Set<string>([activeBranch]);
    const out = [activeBranch];
    for (const b of branches ?? []) {
      if (!seen.has(b.name)) {
        seen.add(b.name);
        out.push(b.name);
      }
    }
    return out;
  }, [branches, activeBranch]);

  const canSubmit = name.trim().length > 0 &&
    (startFrom.kind === "existing"
      ? startFrom.branch.length > 0
      : startFrom.layerId.trim().length > 0);

  const onCreate = async () => {
    setBusy(true);
    setError(null);
    try {
      // Resolve the starting layer at submit time. For the "existing"
      // radio we look up the branch's head from the cached list, or
      // fall back to a live `getBranch` if the cache is empty.
      let fromLayer: string;
      if (startFrom.kind === "existing") {
        const cached = branches?.find((b) => b.name === startFrom.branch);
        if (cached) {
          fromLayer = cached.headLayer;
        } else {
          const resp = await eigen.getBranch(startFrom.branch);
          if (!resp.found) {
            setError(`branch ${startFrom.branch} not found`);
            setBusy(false);
            return;
          }
          fromLayer = resp.headLayer;
        }
      } else {
        fromLayer = startFrom.layerId.trim();
      }
      const result = await createBranch(
        eigen,
        name.trim(),
        fromLayer,
        switchAfter,
      );
      if (!result.success) {
        setError(result.error || "branch creation failed");
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
          <DialogTitle>Create branch</DialogTitle>
          <DialogContent className={styles.body}>
            <Field label="Name" required>
              <Input
                value={name}
                onChange={(_e, data) => setName(data.value)}
                placeholder="kinase-screen"
                disabled={busy}
                autoFocus
              />
              <Caption1>
                Letters, digits, <code>-</code>, <code>_</code>. The
                <code>auto-</code> prefix is reserved.
              </Caption1>
            </Field>

            <Field label="Start from" required>
              <RadioGroup
                value={startFrom.kind === "existing"
                  ? `existing:${startFrom.branch}`
                  : "explicit"}
                onChange={(_e, data) => {
                  if (data.value === "explicit") {
                    setStartFrom({ kind: "explicit", layerId: "" });
                  } else {
                    const branch = data.value.slice("existing:".length);
                    setStartFrom({ kind: "existing", branch });
                  }
                }}
              >
                {startBranchOptions.map((b) => {
                  const head = branches?.find((x) => x.name === b)?.headLayer;
                  return (
                    <Radio
                      key={b}
                      value={`existing:${b}`}
                      disabled={busy}
                      label={
                        <span>
                          Current head of <strong>{b}</strong>
                          {head && (
                            <span className={styles.branchHead}>
                              {shortHash(head)}
                            </span>
                          )}
                        </span>
                      }
                    />
                  );
                })}
                <Radio
                  value="explicit"
                  disabled={busy}
                  label="Specific layer id"
                />
              </RadioGroup>
              {startFrom.kind === "explicit" && (
                <Input
                  className={styles.layerInput}
                  value={startFrom.layerId}
                  onChange={(_e, data) =>
                    setStartFrom({ kind: "explicit", layerId: data.value })}
                  placeholder="64-char hex LayerId"
                  disabled={busy}
                />
              )}
            </Field>

            <Checkbox
              checked={switchAfter}
              onChange={(_e, data) => setSwitchAfter(data.checked === true)}
              disabled={busy}
              label="Switch to this branch after creating"
            />

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

/** Render a `LayerId` hex string as `aaaa…bbbb` (4 + 4). */
function shortHash(hex: string): string {
  if (hex.length <= 10) return hex;
  return `${hex.slice(0, 4)}…${hex.slice(-4)}`;
}
