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
 * Compaction rail destination — D34 §7.
 *
 * Drives a 3-step compaction flow on the active branch:
 *
 *   1. **Range** — pick `from` (oldest) and `to` (newest) layers from
 *      the branch's first-parent walk. Toggle "preserve history" to
 *      keep the pre-consolidation chain reachable for time-travel
 *      (D25 §12.8.1(b)).
 *   2. **Estimate** — call `eigen.estimateConsolidation` and surface
 *      the predicted consolidated layer, the collapsed layer count,
 *      and the dedup savings (`predicted - actual` walk entries).
 *   3. **Run** — call `eigen.consolidateChain`; on success, refresh
 *      branches so the header tip indicator catches up.
 *
 * The branch is pinned to whatever's active in the workspace header;
 * the panel doesn't expose an explicit branch switcher (use the rail
 * header to switch first, then come back). Layer pickers are
 * populated from the same `LayerTopology` walk the History panel
 * uses — every layer in the active branch's first-parent ancestry
 * with its short hash + name.
 */

import { useEffect, useMemo, useState } from "react";
import {
  Body1,
  Button,
  Caption1,
  Checkbox,
  Combobox,
  Field,
  makeStyles,
  MessageBar,
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
import {
  Play20Regular,
  Search20Regular,
  Stack20Regular,
} from "@fluentui/react-icons";
import type {
  ConsolidateChainResponse,
  EstimateConsolidationResponse,
  LayerTopologyResponse,
} from "@eigenius/client";
import { ConsolidateErrorKind, EdgeKind, NodeKind } from "@eigenius/client";
import { useEigen } from "../../runtime/EigenProvider";
import { useNotebookStore } from "../../runtime/notebookStore";

const TOASTER_ID = "compaction-panel-toaster";

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
  block: {
    padding: tokens.spacingVerticalM,
    background: tokens.colorNeutralBackground2,
    borderRadius: tokens.borderRadiusMedium,
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalS,
  },
  metricsGrid: {
    display: "grid",
    gridTemplateColumns: "max-content 1fr",
    columnGap: tokens.spacingHorizontalM,
    rowGap: tokens.spacingVerticalXS,
  },
  metricLabel: {
    color: tokens.colorNeutralForeground3,
  },
  monospace: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
  },
  actions: {
    display: "flex",
    gap: tokens.spacingHorizontalS,
    justifyContent: "flex-end",
  },
  noBranchHint: {
    color: tokens.colorNeutralForeground3,
  },
});

interface LayerRow {
  layerId: string;
  name: string;
  /** Rendered as the option label — `short(name) · short(hash)` for clarity. */
  label: string;
}

type EstimateState =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "ready"; resp: EstimateConsolidationResponse }
  | { kind: "error"; message: string };

type RunState =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "done"; resp: ConsolidateChainResponse }
  | { kind: "error"; message: string };

export function CompactionPanel() {
  const styles = useStyles();
  const eigen = useEigen();
  const toasterId = useId("toaster", TOASTER_ID);
  const { dispatchToast } = useToastController(toasterId);

  const activeBranch = useNotebookStore((s) => s.activeBranch);
  const branches = useNotebookStore((s) => s.branches);
  const refreshBranches = useNotebookStore((s) => s.refreshBranches);

  const activeHead = useMemo(
    () => branches?.find((b) => b.name === activeBranch)?.headLayer ?? null,
    [branches, activeBranch],
  );

  const [topology, setTopology] = useState<LayerTopologyResponse | null>(null);
  const [topoError, setTopoError] = useState<string | null>(null);
  const [fromLayer, setFromLayer] = useState<string>("");
  const [toLayer, setToLayer] = useState<string>("");
  const [preserveHistory, setPreserveHistory] = useState(false);
  const [estimate, setEstimate] = useState<EstimateState>({ kind: "idle" });
  const [run, setRun] = useState<RunState>({ kind: "idle" });

  // Refresh branches on mount so `activeHead` resolves even on a cold
  // notebook load (the header may not have populated yet).
  useEffect(() => {
    void refreshBranches(eigen);
  }, [eigen, refreshBranches]);

  // Fetch the active branch's topology so the layer pickers have
  // something to show. Same call HistoryPanel makes; we walk it
  // identically and discard everything but the layer nodes.
  useEffect(() => {
    let cancelled = false;
    setTopology(null);
    setTopoError(null);
    if (!activeHead) return;
    (async () => {
      try {
        const resp = await eigen.layerTopology({
          rootLayer: activeHead,
          maxDepth: 0,
          includeResources: false,
        });
        if (!cancelled) setTopology(resp);
      } catch (err) {
        if (!cancelled) {
          setTopoError(err instanceof Error ? err.message : String(err));
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [eigen, activeHead]);

  // Build the first-parent chain (tip → root) for the From/To pickers.
  // Same walk as the History panel — the panels stay in sync because
  // they read the same `LayerTopology` shape.
  const layers = useMemo<LayerRow[]>(
    () => (topology && activeHead ? buildLayerChain(topology, activeHead) : []),
    [topology, activeHead],
  );

  // Sensible defaults once the chain loads: From = oldest, To = tip.
  // Re-applied whenever the chain identity changes (different head /
  // branch switch) so stale selections from a previous chain don't
  // survive.
  useEffect(() => {
    if (layers.length === 0) {
      setFromLayer("");
      setToLayer("");
      return;
    }
    setFromLayer(layers[layers.length - 1].layerId); // oldest
    setToLayer(layers[0].layerId); // newest
    setEstimate({ kind: "idle" });
    setRun({ kind: "idle" });
  }, [layers]);

  // Any range/mode change invalidates the prior estimate and any
  // result message — leaving stale values around would mislead.
  useEffect(() => {
    setEstimate({ kind: "idle" });
    setRun({ kind: "idle" });
  }, [fromLayer, toLayer, preserveHistory]);

  const rangeValid = useMemo(() => validateRange(layers, fromLayer, toLayer), [
    layers,
    fromLayer,
    toLayer,
  ]);

  const onEstimate = async () => {
    if (!rangeValid.ok) return;
    setEstimate({ kind: "loading" });
    try {
      const resp = await eigen.estimateConsolidation({
        branch: activeBranch,
        fromLayer,
        toLayer,
        preserveHistory,
      });
      setEstimate({ kind: "ready", resp });
    } catch (err) {
      setEstimate({
        kind: "error",
        message: err instanceof Error ? err.message : String(err),
      });
    }
  };

  const onRun = async () => {
    if (!rangeValid.ok) return;
    setRun({ kind: "running" });
    try {
      const resp = await eigen.consolidateChain({
        branch: activeBranch,
        fromLayer,
        toLayer,
        preserveHistory,
      });
      setRun({ kind: "done", resp });
      if (resp.success) {
        await refreshBranches(eigen);
        dispatchToast(
          <Toast>
            <ToastTitle>Compaction complete</ToastTitle>
            <ToastBody>
              {String(resp.collapsedLayerCount)} layers collapsed on{" "}
              {activeBranch}.
            </ToastBody>
          </Toast>,
          { intent: "success", timeout: 6000 },
        );
      }
    } catch (err) {
      setRun({
        kind: "error",
        message: err instanceof Error ? err.message : String(err),
      });
    }
  };

  const canEstimate = rangeValid.ok && estimate.kind !== "loading";
  // Only enable Run after a successful estimate — keeps the user from
  // pressing Run before they've read what it does. The estimate is
  // also what surfaces COST_EXCEEDS_CAP / TRACE_PIN errors safely.
  const canRun = rangeValid.ok &&
    estimate.kind === "ready" &&
    estimate.resp.success &&
    run.kind !== "running";

  return (
    <div className={styles.root}>
      <div className={styles.header}>
        <Stack20Regular />
        <Subtitle1 as="h2">Compaction</Subtitle1>
      </div>
      <div className={styles.body}>
        <div className={styles.bodyInner}>
          {!activeHead && (
            <Caption1 className={styles.noBranchHint}>
              Loading active branch…
            </Caption1>
          )}
          {topoError && (
            <MessageBar intent="error">
              <MessageBarBody>
                <MessageBarTitle>Couldn't load the layer chain</MessageBarTitle>
                {topoError}
              </MessageBarBody>
            </MessageBar>
          )}

          <div className={styles.fields}>
            <Field
              label="Branch"
              hint="Pinned to the workspace's active branch. Switch via the header."
            >
              <Body1 className={styles.monospace}>{activeBranch}</Body1>
            </Field>
            <LayerPicker
              label="From (oldest)"
              value={fromLayer}
              onChange={setFromLayer}
              layers={layers}
              disabled={layers.length === 0}
            />
            <LayerPicker
              label="To (newest)"
              value={toLayer}
              onChange={setToLayer}
              layers={layers}
              disabled={layers.length === 0}
            />
            {!rangeValid.ok && rangeValid.reason && (
              <Caption1>{rangeValid.reason}</Caption1>
            )}
            <Checkbox
              checked={preserveHistory}
              onChange={(_e, data) => setPreserveHistory(data.checked === true)}
              label={
                <span>
                  <strong>Preserve history</strong>{" "}
                  — keep the pre-consolidation chain reachable for time-travel
                  reads. Default off (the source range becomes GC-eligible).
                </span>
              }
            />
          </div>

          <EstimateBlock state={estimate} styles={styles} />
          <RunBlock state={run} styles={styles} />

          <div className={styles.actions}>
            <Button
              appearance="secondary"
              icon={<Search20Regular />}
              disabled={!canEstimate}
              onClick={() => void onEstimate()}
            >
              {estimate.kind === "loading" ? "Estimating…" : "Estimate"}
            </Button>
            <Button
              appearance="primary"
              icon={<Play20Regular />}
              disabled={!canRun}
              onClick={() => void onRun()}
            >
              {run.kind === "running" ? "Running…" : "Run compaction"}
            </Button>
          </div>
        </div>
      </div>
      <Toaster toasterId={toasterId} position="top-end" />
    </div>
  );
}

interface LayerPickerProps {
  label: string;
  value: string;
  onChange: (v: string) => void;
  layers: readonly LayerRow[];
  disabled: boolean;
}

function LayerPicker({
  label,
  value,
  onChange,
  layers,
  disabled,
}: LayerPickerProps) {
  // Render the chosen layer's *label* in the Combobox input, not its
  // hex id — Combobox's `value` prop drives the text shown, so passing
  // the layerId would show a long hash. We map id → label for display
  // and let `selectedOptions` carry the actual identity.
  const selectedRow = layers.find((l) => l.layerId === value);
  const displayValue = selectedRow?.label ?? "";
  return (
    <Field label={label}>
      <Combobox
        value={displayValue}
        selectedOptions={value ? [value] : []}
        onOptionSelect={(_e, data) => onChange(data.optionValue ?? "")}
        placeholder={layers.length === 0
          ? "(no layers available)"
          : "Select a layer"}
        disabled={disabled}
      >
        {layers.map((l) => (
          <Option key={l.layerId} value={l.layerId} text={l.label}>
            {l.label}
          </Option>
        ))}
      </Combobox>
    </Field>
  );
}

interface EstimateBlockProps {
  state: EstimateState;
  styles: ReturnType<typeof useStyles>;
}

function EstimateBlock({ state, styles }: EstimateBlockProps) {
  if (state.kind === "idle") {
    return (
      <div className={styles.block}>
        <Caption1>
          Click <strong>Estimate</strong>{" "}
          to preview the outcome without touching the chain.
        </Caption1>
      </div>
    );
  }
  if (state.kind === "loading") {
    return (
      <div className={styles.block}>
        <Spinner size="tiny" label="computing estimate" />
      </div>
    );
  }
  if (state.kind === "error") {
    return (
      <MessageBar intent="error">
        <MessageBarBody>
          <MessageBarTitle>Estimate failed</MessageBarTitle>
          {state.message}
        </MessageBarBody>
      </MessageBar>
    );
  }
  const resp = state.resp;
  if (!resp.success) {
    return (
      <MessageBar intent="error">
        <MessageBarBody>
          <MessageBarTitle>
            {friendlyErrorKind(resp.errorKind) ?? "Estimate rejected"}
          </MessageBarTitle>
          <div>{resp.error || "estimate failed"}</div>
          {resp.errorLayer && (
            <Caption1 className={styles.monospace}>
              Offending layer: {resp.errorLayer}
            </Caption1>
          )}
          {resp.errorCount > 0n && (
            <Caption1>
              {kindIsCostCap(resp.errorKind) ? "Predicted entries" : "Count"}:
              {" "}
              {String(resp.errorCount)}
            </Caption1>
          )}
        </MessageBarBody>
      </MessageBar>
    );
  }
  const savings = resp.predictedWalkEntries - resp.actualWalkEntries;
  return (
    <div className={styles.block}>
      <Body1>
        <strong>Estimated outcome</strong>
      </Body1>
      <div className={styles.metricsGrid}>
        <Caption1 className={styles.metricLabel}>Layers collapsed</Caption1>
        <Body1>{String(resp.collapsedLayerCount)} → 1</Body1>
        <Caption1 className={styles.metricLabel}>Resources kept</Caption1>
        <Body1>
          {String(resp.actualWalkEntries)} (deduplicated from{" "}
          {String(resp.predictedWalkEntries)}; saved {String(savings)})
        </Body1>
        <Caption1 className={styles.metricLabel}>Result layer</Caption1>
        <Body1 className={styles.monospace}>
          {resp.predictedConsolidatedLayer}
        </Body1>
      </div>
    </div>
  );
}

interface RunBlockProps {
  state: RunState;
  styles: ReturnType<typeof useStyles>;
}

function RunBlock({ state, styles }: RunBlockProps) {
  if (state.kind === "idle") return null;
  if (state.kind === "running") {
    return (
      <div className={styles.block}>
        <Spinner size="tiny" label="running compaction" />
      </div>
    );
  }
  if (state.kind === "error") {
    return (
      <MessageBar intent="error">
        <MessageBarBody>
          <MessageBarTitle>Compaction failed</MessageBarTitle>
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
        <MessageBarBody>
          <MessageBarTitle>
            {friendlyErrorKind(resp.errorKind) ?? "Compaction rejected"}
          </MessageBarTitle>
          <div>{resp.error || "compaction failed"}</div>
          {resp.errorLayer && (
            <Caption1 className={styles.monospace}>
              Offending layer: {resp.errorLayer}
            </Caption1>
          )}
        </MessageBarBody>
      </MessageBar>
    );
  }
  return (
    <MessageBar intent="success">
      <MessageBarBody>
        <MessageBarTitle>
          Compacted {String(resp.collapsedLayerCount)} layers → 1
        </MessageBarTitle>
        <div>
          {resp.headAdvanced
            ? "Branch tip advanced to the consolidated layer."
            : "Branch tip unchanged; a resolve redirect was installed at the range's end."}
        </div>
        <Caption1 className={styles.monospace}>
          Consolidated layer: {resp.consolidatedLayer}
        </Caption1>
      </MessageBarBody>
    </MessageBar>
  );
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Walk the topology from `head` along first-parent edges, producing
 * a tip→root list of `LayerRow`s. Identical shape to HistoryPanel's
 * walk, kept inline so the two panels don't accidentally couple on a
 * shared helper that changes shape under one of them.
 */
function buildLayerChain(
  topology: LayerTopologyResponse,
  head: string,
): LayerRow[] {
  const nodesById = new Map<string, (typeof topology.nodes)[number]>();
  for (const n of topology.nodes) {
    if (n.kind === NodeKind.LAYER) nodesById.set(n.id, n);
  }
  const parentBySource = new Map<string, string>();
  for (const e of topology.edges) {
    if (e.kind !== EdgeKind.PARENT_LAYER) continue;
    // First-parent only — the History panel's convention, and the
    // shape ConsolidateChain understands (the range walks parent[0]).
    if (!parentBySource.has(e.source)) {
      parentBySource.set(e.source, e.target);
    }
  }
  const rows: LayerRow[] = [];
  const visited = new Set<string>();
  let cursor: string | undefined = head;
  while (cursor && !visited.has(cursor)) {
    visited.add(cursor);
    const node = nodesById.get(cursor);
    if (!node) break;
    const name = node.attrs.name ?? node.label ?? "(unnamed)";
    rows.push({
      layerId: cursor,
      name,
      label: `${name} · ${shortHash(cursor)}`,
    });
    cursor = parentBySource.get(cursor);
  }
  return rows;
}

function shortHash(hex: string): string {
  if (hex.length <= 10) return hex;
  return `${hex.slice(0, 4)}…${hex.slice(-4)}`;
}

type RangeValidation =
  | { ok: true }
  | { ok: false; reason?: string };

/**
 * Validate the From/To selection against the chain. Both layers must
 * exist; `from` (oldest) must be deeper in the chain than `to`
 * (newest), i.e. its index in the tip→root array must be *higher*.
 */
function validateRange(
  layers: readonly LayerRow[],
  from: string,
  to: string,
): RangeValidation {
  if (!from || !to) return { ok: false };
  if (from === to) {
    return { ok: false, reason: "From and To must differ." };
  }
  const fromIdx = layers.findIndex((l) => l.layerId === from);
  const toIdx = layers.findIndex((l) => l.layerId === to);
  if (fromIdx === -1 || toIdx === -1) {
    return {
      ok: false,
      reason: "Selected layers aren't on the active branch.",
    };
  }
  if (fromIdx < toIdx) {
    return {
      ok: false,
      reason:
        "“From” is newer than “To”. From must be the older end of the range.",
    };
  }
  return { ok: true };
}

function kindIsCostCap(kind: ConsolidateErrorKind): boolean {
  return kind === ConsolidateErrorKind.COST_EXCEEDS_CAP;
}

/**
 * Translate the kernel's typed error kind into a one-line UI title.
 * Returns `null` when the kind is `NONE` (success) or the kernel sent
 * an unknown variant — the caller falls back to a generic title.
 */
function friendlyErrorKind(kind: ConsolidateErrorKind): string | null {
  switch (kind) {
    case ConsolidateErrorKind.UNSPECIFIED:
      return null;
    case ConsolidateErrorKind.RANGE_NOT_ANCESTRAL:
      return "“From” is not an ancestor of “To”";
    case ConsolidateErrorKind.BRANCH_ADVANCED:
      return "Branch advanced under the request";
    case ConsolidateErrorKind.RANGE_CONTAINS_MERGE_NODE:
      return "Range contains a merge node";
    case ConsolidateErrorKind.RANGE_CONTAINS_TRACE_PIN:
      return "Range contains a layer pinned by an active task";
    case ConsolidateErrorKind.COST_EXCEEDS_CAP:
      return "Predicted cost exceeds cap";
    case ConsolidateErrorKind.INTERNAL:
      return "Internal kernel error";
    case ConsolidateErrorKind.TO_NOT_REACHABLE_FROM_HEAD:
      return "“To” is not reachable from the branch head";
    case ConsolidateErrorKind.RANGE_CROSSES_EXISTING_REDIRECT:
      return "Range crosses an existing redirect";
    default:
      return null;
  }
}
