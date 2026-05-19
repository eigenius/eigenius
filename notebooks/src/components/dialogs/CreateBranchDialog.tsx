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

import { useEffect, useMemo, useRef, useState } from "react";
import {
  Button,
  Caption1,
  Checkbox,
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
  Radio,
  RadioGroup,
  tokens,
} from "@fluentui/react-components";
import type { TagInfo } from "@eigenius/client";
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
  /** Pre-fill the dialog with `Specific layer id = defaultLayerId`.
   *  Set when the dialog is opened from the History panel's per-layer
   *  "Create branch…" affordance so the user only has to type a name.
   *  Ignored when `defaultTagName` is also set (tags win since they're
   *  the more human-readable handle on the same layer). */
  defaultLayerId?: string;
  /** Pre-select `Tag = defaultTagName` in the start-from picker. Set
   *  when the dialog is opened from a Tags-panel row so the user sees
   *  the tag name they clicked rather than its (semantically
   *  identical, but opaque) underlying layer id. */
  defaultTagName?: string;
}

/**
 * Discriminated union over the "Start from" radio options. The
 * `existing` and `tag` shapes carry the chosen name so we can look
 * up its target layer at create time (rather than at open time,
 * which could be stale by the time the user clicks Create).
 */
type StartFrom =
  | { kind: "existing"; branch: string }
  | { kind: "tag"; tag: string }
  | { kind: "explicit"; layerId: string };

/** Default `startFrom` derived from the dialog's pre-fill props.
 *  Priority: tag pre-select wins over layer-id pre-fill, since a tag
 *  is the more human-readable handle on the same layer; both fall
 *  back to the active branch's head. */
function initialStartFrom(
  defaultTagName: string | undefined,
  defaultLayerId: string | undefined,
  activeBranch: string,
): StartFrom {
  if (defaultTagName) return { kind: "tag", tag: defaultTagName };
  if (defaultLayerId) return { kind: "explicit", layerId: defaultLayerId };
  return { kind: "existing", branch: activeBranch };
}

export function CreateBranchDialog({
  open,
  onClose,
  onCreated,
  defaultLayerId,
  defaultTagName,
}: CreateBranchDialogProps) {
  const styles = useStyles();
  const eigen = useEigen();
  const branches = useNotebookStore((s) => s.branches);
  const activeBranch = useNotebookStore((s) => s.activeBranch);
  const createBranch = useNotebookStore((s) => s.createBranch);

  const [name, setName] = useState("");
  const [startFrom, setStartFrom] = useState<StartFrom>(
    initialStartFrom(defaultTagName, defaultLayerId, activeBranch),
  );
  const [switchAfter, setSwitchAfter] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Tags are pulled fresh on every dialog open. Cheap (one RPC) and
  // avoids the "stale tag list a session later" problem. `null` =
  // fetch in flight; `[]` = fetched, none defined.
  const [tags, setTags] = useState<readonly TagInfo[] | null>(null);

  // Held in a ref rather than the effect's deps because
  // `createBranch` flips `activeBranch` in the store mid-`await` (the
  // `switchAfter` path calls `switchBranch`). With `activeBranch` in
  // the deps the reset effect re-fires while the user is still inside
  // a create flow, clobbering the typed name + `busy` flag. The ref
  // always reflects the latest value so the captured-on-open default
  // for `startFrom` stays correct.
  const activeBranchRef = useRef(activeBranch);
  activeBranchRef.current = activeBranch;

  // Hold the latest `default*` props in refs for the same reason as
  // `activeBranch` — they can change while the dialog is open
  // (parent re-renders) without us wanting to clobber a typed name.
  const defaultLayerIdRef = useRef(defaultLayerId);
  defaultLayerIdRef.current = defaultLayerId;
  const defaultTagNameRef = useRef(defaultTagName);
  defaultTagNameRef.current = defaultTagName;

  // Reset transient state on the `open` false → true transition.
  useEffect(() => {
    if (open) {
      setName("");
      setStartFrom(
        initialStartFrom(
          defaultTagNameRef.current,
          defaultLayerIdRef.current,
          activeBranchRef.current,
        ),
      );
      setSwitchAfter(true);
      setBusy(false);
      setError(null);
      setTags(null);
      let cancelled = false;
      void (async () => {
        try {
          const list = await eigen.listTags();
          if (!cancelled) setTags(list);
        } catch {
          // Best-effort: a tag-fetch failure shouldn't block branch
          // creation off existing branches or explicit layer ids.
          if (!cancelled) setTags([]);
        }
      })();
      return () => {
        cancelled = true;
      };
    }
  }, [open, eigen]);

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

  const canSubmit = name.trim().length > 0 && (
    startFrom.kind === "existing"
      ? startFrom.branch.length > 0
      : startFrom.kind === "tag"
      ? startFrom.tag.length > 0
      : startFrom.layerId.trim().length > 0
  );

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
      } else if (startFrom.kind === "tag") {
        // Re-fetch on submit so the resolved layer is current even
        // if the dialog has been open for a while. Tags are immutable
        // by contract, but a fresh fetch surfaces deletions cleanly.
        const fresh = await eigen.listTags();
        const found = fresh.find((t) => t.name === startFrom.tag);
        if (!found) {
          setError(`tag ${startFrom.tag} not found`);
          setBusy(false);
          return;
        }
        fromLayer = found.layerId;
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
      // Reset the in-flight flag before closing so a follow-up open
      // can't catch a stale `busy=true` (which would render the form
      // disabled until the on-open reset effect fires).
      setBusy(false);
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
                value={startFrom.kind}
                onChange={(_e, data) => {
                  if (data.value === "explicit") {
                    setStartFrom({ kind: "explicit", layerId: "" });
                  } else if (data.value === "tag") {
                    setStartFrom({
                      kind: "tag",
                      tag: tags?.[0]?.name ?? "",
                    });
                  } else {
                    setStartFrom({
                      kind: "existing",
                      branch: activeBranchRef.current,
                    });
                  }
                }}
              >
                <Radio
                  value="existing"
                  disabled={busy}
                  label="Current head of a branch"
                />
                {startFrom.kind === "existing" && (
                  <Combobox
                    value={startFrom.branch}
                    selectedOptions={startFrom.branch ? [startFrom.branch] : []}
                    onOptionSelect={(_e, data) => {
                      if (data.optionValue) {
                        setStartFrom({
                          kind: "existing",
                          branch: data.optionValue,
                        });
                      }
                    }}
                    placeholder="Select a branch"
                    disabled={busy}
                  >
                    {startBranchOptions.map((b) => {
                      const head = branches?.find((x) => x.name === b)
                        ?.headLayer;
                      return (
                        <Option key={b} value={b} text={b}>
                          <span>
                            <strong>{b}</strong>
                            {head && (
                              <span className={styles.branchHead}>
                                {shortHash(head)}
                              </span>
                            )}
                          </span>
                        </Option>
                      );
                    })}
                  </Combobox>
                )}
                <Radio
                  value="tag"
                  disabled={busy || (tags !== null && tags.length === 0)}
                  label={tags !== null && tags.length === 0
                    ? "Tag (no tags defined)"
                    : "Tag"}
                />
                {startFrom.kind === "tag" && (
                  <>
                    {tags === null && <Caption1>Loading tags…</Caption1>}
                    {tags !== null && tags.length > 0 && (
                      <Combobox
                        value={startFrom.tag}
                        selectedOptions={startFrom.tag ? [startFrom.tag] : []}
                        onOptionSelect={(_e, data) => {
                          if (data.optionValue) {
                            setStartFrom({
                              kind: "tag",
                              tag: data.optionValue,
                            });
                          }
                        }}
                        placeholder="Select a tag"
                        disabled={busy}
                      >
                        {tags.map((t) => (
                          <Option key={t.name} value={t.name} text={t.name}>
                            <span>
                              <strong>{t.name}</strong>
                              <span className={styles.branchHead}>
                                {shortHash(t.layerId)}
                              </span>
                            </span>
                          </Option>
                        ))}
                      </Combobox>
                    )}
                  </>
                )}
                <Radio
                  value="explicit"
                  disabled={busy}
                  label="Specific layer id"
                />
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
              </RadioGroup>
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

