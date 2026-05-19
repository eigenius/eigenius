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
 * Merge rail destination — D34 §6.3.
 *
 * Pick a source branch and a target branch; preview the predicted
 * outcome (fast-forward / trivial merge / needs witnessed); commit
 * the merge.
 *
 * The kernel pieces this panel uses are Phase-5 additions:
 *
 * - `eigen.previewMerge(source, target)` — side-effect-free LCA +
 *   IRI-disjointness walk. Returns the predicted outcome without
 *   building a merge layer.
 * - `eigen.mergeBranches(source, target)` — wraps the kernel's
 *   `update_branch(target, target_tip, source_tip, AllowTrivial)`.
 *   On `NEEDS_WITNESSED_MERGE`, the response carries
 *   `merge.orphan_layer_id`, which we feed into `openResolution` to
 *   route the user into the D36 resolution flow.
 *
 * Both calls are scoped to branches the kernel already knows about —
 * the panel doesn't accept arbitrary layer ids. Layer-level merges
 * remain the kernel's internal `merge_independent_heads`; only the
 * branch-level wrapper is exposed.
 */

import { useEffect, useMemo, useRef, useState } from "react";
import {
  Body1,
  Button,
  Caption1,
  Combobox,
  Field,
  makeStyles,
  MessageBar,
  MessageBarActions,
  MessageBarBody,
  MessageBarTitle,
  Option,
  Spinner,
  Subtitle1,
  Toast,
  ToastBody,
  Toaster,
  ToastTitle,
  tokens,
  useId,
  useToastController,
} from "@fluentui/react-components";
import { Merge20Regular, Search20Regular } from "@fluentui/react-icons";
import {
  type MergeBranchesResponse,
  MergeOutcome,
  type PreviewMergeResponse,
} from "@eigenius/client";
import { useEigen } from "../../runtime/EigenProvider";
import { useNotebookStore } from "../../runtime/notebookStore";
import { MergeResolutionFlow } from "../merge/MergeResolutionFlow";

const TOASTER_ID = "merge-panel-toaster";

const useStyles = makeStyles({
  root: {
    display: "flex",
    flexDirection: "column",
    height: "100%",
    minHeight: 0,
  },
  header: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalM,
    padding: `${tokens.spacingVerticalM} ${tokens.spacingHorizontalXXL}`,
    borderBottom: `1px solid ${tokens.colorNeutralStroke2}`,
  },
  body: {
    flex: 1,
    minHeight: 0,
    overflowY: "auto",
    padding: tokens.spacingVerticalXXL,
  },
  bodyInner: {
    maxWidth: "640px",
    margin: "0 auto",
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalL,
  },
  fields: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalM,
  },
  previewBlock: {
    padding: tokens.spacingVerticalM,
    background: tokens.colorNeutralBackground2,
    borderRadius: tokens.borderRadiusMedium,
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalS,
  },
  actions: {
    display: "flex",
    gap: tokens.spacingHorizontalS,
    justifyContent: "flex-end",
  },
  conflictList: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    margin: `${tokens.spacingVerticalXS} 0 0 ${tokens.spacingHorizontalM}`,
    padding: 0,
  },
});

type PreviewState =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "ready"; resp: PreviewMergeResponse }
  | { kind: "error"; message: string };

type MergeState =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "done"; resp: MergeBranchesResponse }
  | { kind: "error"; message: string };

export function MergePanel() {
  const styles = useStyles();
  const eigen = useEigen();
  const toasterId = useId("toaster", TOASTER_ID);
  const { dispatchToast } = useToastController(toasterId);

  const activeBranch = useNotebookStore((s) => s.activeBranch);
  const branches = useNotebookStore((s) => s.branches);
  const refreshBranches = useNotebookStore((s) => s.refreshBranches);
  const openResolution = useNotebookStore((s) => s.openResolution);
  const pendingMergeSource = useNotebookStore((s) => s.pendingMergeSource);
  const setPendingMergeSource = useNotebookStore(
    (s) => s.setPendingMergeSource,
  );

  // Sensible target default: pick `main` if it exists and isn't the
  // source, otherwise the first branch that isn't the source. The
  // user always re-selects intentionally, but a non-empty default
  // means the first preview click works without a fiddly setup.
  // Factored out so both the initial setup and the post-resolution
  // reset can compute it consistently.
  const computeDefaultTarget = (
    src: string,
    brs: readonly { name: string }[] | null,
  ): string => {
    if (!brs) return src === "main" ? "" : "main";
    if (src !== "main" && brs.some((b) => b.name === "main")) {
      return "main";
    }
    return brs.find((b) => b.name !== src)?.name ?? "";
  };

  // `pendingMergeSource` is a one-shot hint from BranchesPanel's
  // "Merge into…" action; consume it on mount so a later visit
  // doesn't keep pre-filling a stale source.
  const [source, setSource] = useState<string>(
    pendingMergeSource ?? activeBranch,
  );
  const initialTarget = useMemo(
    () => computeDefaultTarget(source, branches),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );
  const [target, setTarget] = useState<string>(initialTarget);
  const [preview, setPreview] = useState<PreviewState>({ kind: "idle" });
  const [mergeState, setMergeState] = useState<MergeState>({ kind: "idle" });

  // Always refresh branches on mount so the dropdowns are populated
  // with what's currently on disk.
  useEffect(() => {
    void refreshBranches(eigen);
  }, [eigen, refreshBranches]);

  // Consume the pending source hint once it's been applied.
  useEffect(() => {
    if (pendingMergeSource !== null) {
      setPendingMergeSource(null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Reset preview + result whenever source/target change — they
  // referenced the previous pair and would mislead.
  useEffect(() => {
    setPreview({ kind: "idle" });
    setMergeState({ kind: "idle" });
  }, [source, target]);

  // Reset the form when a resolution session ends (committed via the
  // `done` card → Close, or cancelled at any state). Without this
  // the panel returns the user to a stale source/target + preview/
  // result snapshot from before the resolution started; a fresh
  // form matches the "new workflow" mental model.
  const resolutionKind = useNotebookStore((s) => s.mergeResolution.kind);
  const prevResolutionOpen = useRef(resolutionKind !== "closed");
  useEffect(() => {
    const isOpen = resolutionKind !== "closed";
    if (prevResolutionOpen.current && !isOpen) {
      const freshSource = activeBranch;
      setSource(freshSource);
      setTarget(computeDefaultTarget(freshSource, branches));
      // preview + mergeState reset via the [source, target] effect
      // above; setting them explicitly here keeps the post-reset
      // render free of a flicker where the old preview/result is
      // still visible against the new source/target pair.
      setPreview({ kind: "idle" });
      setMergeState({ kind: "idle" });
    }
    prevResolutionOpen.current = isOpen;
  }, [resolutionKind, activeBranch, branches]);

  const onPreview = async () => {
    if (!source || !target || source === target) return;
    setPreview({ kind: "loading" });
    try {
      const resp = await eigen.previewMerge(source, target);
      setPreview({ kind: "ready", resp });
    } catch (err) {
      setPreview({
        kind: "error",
        message: err instanceof Error ? err.message : String(err),
      });
    }
  };

  const onMerge = async () => {
    if (!source || !target || source === target) return;
    setMergeState({ kind: "running" });
    try {
      const resp = await eigen.mergeBranches(source, target);
      setMergeState({ kind: "done", resp });
      if (resp.success) {
        await refreshBranches(eigen);
        const outcome = resp.merge?.outcome;
        const friendly = outcome === MergeOutcome.FAST_FORWARD
          ? "fast-forwarded"
          : outcome === MergeOutcome.TRIVIAL_MERGE
          ? "merged (trivial)"
          : outcome === MergeOutcome.NEEDS_WITNESSED_MERGE
          ? "conflicted — branch unchanged"
          : "completed";
        dispatchToast(
          <Toast>
            <ToastTitle>Merge {friendly}</ToastTitle>
            <ToastBody>
              {source} → {target}
            </ToastBody>
          </Toast>,
          {
            intent: outcome === MergeOutcome.NEEDS_WITNESSED_MERGE
              ? "warning"
              : "success",
            timeout: 6000,
          },
        );
      }
    } catch (err) {
      setMergeState({
        kind: "error",
        message: err instanceof Error ? err.message : String(err),
      });
    }
  };

  const canSubmit = source && target && source !== target;
  // D36 §4 — when a resolution session is open, the panel switches
  // into resolution mode and hosts `MergeResolutionFlow` instead of
  // the explicit source/target merge UI. The two modes are mutually
  // exclusive within a session.
  if (resolutionKind !== "closed") {
    return (
      <div className={styles.root}>
        <div className={styles.header}>
          <Merge20Regular />
          <Subtitle1 as="h2">
            Merge — {resolutionKind === "done"
              ? "committed"
              : "resolving conflicts"}
          </Subtitle1>
        </div>
        <div className={styles.body}>
          <div className={styles.bodyInner}>
            <MergeResolutionFlow />
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className={styles.root}>
      <div className={styles.header}>
        <Merge20Regular />
        <Subtitle1 as="h2">Merge branches</Subtitle1>
      </div>
      <div className={styles.body}>
        <div className={styles.bodyInner}>
          <div className={styles.fields}>
            <BranchPicker
              label="Source"
              hint="Branch to fold in"
              value={source}
              onChange={setSource}
              branches={branches}
            />
            <BranchPicker
              label="Target"
              hint="Branch to fold into"
              value={target}
              onChange={setTarget}
              branches={branches}
            />
            {source && target && source === target && (
              <Caption1>Source and target must differ.</Caption1>
            )}
          </div>

          <PreviewBlock
            state={preview}
            onRun={() => void onPreview()}
            canSubmit={!!canSubmit}
            styles={styles}
          />

          <ResultBlock
            state={mergeState}
            styles={styles}
            onResolve={(candidateHead) => {
              void openResolution(eigen, {
                branch: target,
                candidateHead,
              });
            }}
          />

          <div className={styles.actions}>
            <Button
              appearance="secondary"
              icon={<Search20Regular />}
              disabled={!canSubmit || preview.kind === "loading"}
              onClick={() => void onPreview()}
            >
              {preview.kind === "loading" ? "Previewing…" : "Refresh preview"}
            </Button>
            <Button
              appearance="primary"
              icon={<Merge20Regular />}
              disabled={!canSubmit || mergeState.kind === "running"}
              onClick={() => void onMerge()}
            >
              {mergeState.kind === "running" ? "Merging…" : "Merge"}
            </Button>
          </div>
        </div>
      </div>
      <Toaster toasterId={toasterId} position="top-end" />
    </div>
  );
}

interface BranchPickerProps {
  label: string;
  hint: string;
  value: string;
  onChange: (v: string) => void;
  branches: readonly { name: string }[] | null;
}

function BranchPicker({
  label,
  hint,
  value,
  onChange,
  branches,
}: BranchPickerProps) {
  return (
    <Field label={label} hint={hint}>
      <Combobox
        value={value}
        selectedOptions={value ? [value] : []}
        onOptionSelect={(_e, data) => {
          onChange(data.optionValue ?? "");
        }}
        placeholder={branches ? "Select a branch" : "(no branches available)"}
        disabled={!branches || branches.length === 0}
      >
        {(branches ?? []).map((b) => (
          <Option key={b.name} value={b.name}>
            {b.name}
          </Option>
        ))}
      </Combobox>
    </Field>
  );
}

interface PreviewBlockProps {
  state: PreviewState;
  onRun: () => void;
  canSubmit: boolean;
  styles: ReturnType<typeof useStyles>;
}

function PreviewBlock({ state, onRun, canSubmit, styles }: PreviewBlockProps) {
  if (state.kind === "idle") {
    return (
      <div className={styles.previewBlock}>
        <Caption1>
          Click <strong>Refresh preview</strong>{" "}
          to estimate the outcome without committing.
        </Caption1>
        {!canSubmit && (
          <Caption1>Pick a source and target above first.</Caption1>
        )}
      </div>
    );
  }
  if (state.kind === "loading") {
    return (
      <div className={styles.previewBlock}>
        <Spinner size="tiny" label="computing preview" />
      </div>
    );
  }
  if (state.kind === "error") {
    return (
      <MessageBar intent="error">
        <MessageBarBody>
          <MessageBarTitle>Preview failed</MessageBarTitle>
          {state.message}
        </MessageBarBody>
      </MessageBar>
    );
  }
  // Ready.
  const resp = state.resp;
  if (!resp.success || !resp.merge) {
    return (
      <MessageBar intent="error">
        <MessageBarBody>{resp.error || "preview failed"}</MessageBarBody>
      </MessageBar>
    );
  }
  const outcome = resp.merge.outcome;
  if (outcome === MergeOutcome.FAST_FORWARD) {
    return (
      <div className={styles.previewBlock}>
        <Body1>
          <strong>Predicted: fast-forward.</strong>{" "}
          Target already includes source's tip (or source's tip is a descendant
          of target's).
        </Body1>
        <Caption1>
          No new merge layer — target advances to source's tip.
        </Caption1>
      </div>
    );
  }
  if (outcome === MergeOutcome.TRIVIAL_MERGE) {
    return (
      <div className={styles.previewBlock}>
        <Body1>
          <strong>Predicted: trivial merge.</strong> {resp.predictedIriCount}
          {" "}
          resource
          {resp.predictedIriCount === 1 ? "" : "s"} in the merge layer.
        </Body1>
        <Caption1>
          The two branches modify disjoint IRIs since their LCA — a multi-
          parent merge layer will be built.
        </Caption1>
      </div>
    );
  }
  if (outcome === MergeOutcome.NEEDS_WITNESSED_MERGE) {
    return (
      <MessageBar intent="warning">
        <MessageBarBody>
          <MessageBarTitle>Predicted: conflict</MessageBarTitle>
          <div>
            The two branches modify the same {resp.merge.conflictingIris.length}
            {" "}
            resource
            {resp.merge.conflictingIris.length === 1 ? "" : "s"}{" "}
            since their LCA. Merging will require Phase 15 witnessed-merge
            resolution.
          </div>
          {resp.merge.conflictingIris.length > 0 && (
            <ul className={styles.conflictList}>
              {resp.merge.conflictingIris.slice(0, 10).map((iri) => (
                <li key={iri}>{iri}</li>
              ))}
              {resp.merge.conflictingIris.length > 10 && (
                <li>… and {resp.merge.conflictingIris.length - 10} more</li>
              )}
            </ul>
          )}
        </MessageBarBody>
      </MessageBar>
    );
  }
  // UNSPECIFIED / CACHED_DIFFERENT_POSITION — shouldn't happen on
  // preview but render something rather than blanking the panel.
  return (
    <div className={styles.previewBlock}>
      <Caption1>
        Unexpected preview outcome ({outcome}). Try refreshing or report this as
        a bug.
      </Caption1>
    </div>
  );
  void onRun;
}

interface ResultBlockProps {
  state: MergeState;
  styles: ReturnType<typeof useStyles>;
  onResolve: (candidateHead: string) => void;
}

function ResultBlock({ state, styles, onResolve }: ResultBlockProps) {
  if (state.kind === "idle") return null;
  if (state.kind === "running") {
    return (
      <div className={styles.previewBlock}>
        <Spinner size="tiny" label="merging" />
      </div>
    );
  }
  if (state.kind === "error") {
    return (
      <MessageBar intent="error">
        <MessageBarBody>
          <MessageBarTitle>Merge failed</MessageBarTitle>
          {state.message}
        </MessageBarBody>
      </MessageBar>
    );
  }
  // done
  const resp = state.resp;
  if (!resp.success) {
    return (
      <MessageBar intent="error">
        <MessageBarBody>{resp.error || "merge failed"}</MessageBarBody>
      </MessageBar>
    );
  }
  const outcome = resp.merge?.outcome;
  if (outcome === MergeOutcome.NEEDS_WITNESSED_MERGE) {
    const orphanLayerId = resp.merge?.orphanLayerId;
    return (
      <MessageBar intent="warning">
        <MessageBarBody>
          <MessageBarTitle>Conflict — target unchanged</MessageBarTitle>
          <div>
            The merge would conflict on{" "}
            {resp.merge?.conflictingIris.length ?? 0} resource(s). The target
            branch is unchanged; pick a per-conflict resolution strategy to
            commit a merge layer.
          </div>
        </MessageBarBody>
        {orphanLayerId && (
          <MessageBarActions>
            <Button
              size="small"
              appearance="primary"
              onClick={() => onResolve(orphanLayerId)}
            >
              Resolve conflicts
            </Button>
          </MessageBarActions>
        )}
      </MessageBar>
    );
  }
  return (
    <MessageBar intent="success">
      <MessageBarBody>
        <MessageBarTitle>
          {outcome === MergeOutcome.FAST_FORWARD ? "Fast-forwarded" : "Merged"}
        </MessageBarTitle>
        <div>
          Target's new tip:{" "}
          <code style={{ fontFamily: "monospace" }}>{resp.targetTip}</code>
        </div>
      </MessageBarBody>
    </MessageBar>
  );
}
