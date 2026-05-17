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
 * History rail destination — D34 §5.
 *
 * Linear first-parent walk of the active branch from tip to root, with
 * a side detail panel for the currently-selected row.
 *
 * Data path: one `eigen.layerTopology({ rootLayer: activeHead })` call.
 * We don't ask for `includeResources` — only layer nodes and their
 * `parent_layer` edges are needed. The walk is then computed
 * client-side over the returned `nodes` + `edges`.
 *
 * Per-row data (all from the topology's LAYER `attrs`):
 * - `name` — the layer's display label.
 * - `created_at_ms` — commit timestamp.
 * - `resource_count` — instance resources defined directly *in this layer*
 *   (the kernel buckets Class/Property/Institution separately; this is
 *   the leftover "plain instance" count). Rendered verbatim — there's
 *   no meaningful delta to compute from this attribute alone, since the
 *   parent's `resource_count` is *its own* per-layer count, not a
 *   running total.
 *
 * Glyph (D34 §5.2):
 * - `◆` multi-parent merge (`parent_layer` edges >= 2 from this row).
 * - `⬚` root (no parent_layer edges out of this row).
 * - `●` ordinary layer.
 * - `◐` redirect-source tombstone (TODO — `LayerTopology` doesn't
 *   expose this today; deferred until the walker tags tombstones).
 */

import { useEffect, useMemo, useState } from "react";
import {
  Button,
  Caption1,
  Divider,
  makeStyles,
  MessageBar,
  MessageBarBody,
  Spinner,
  Subtitle1,
  Toast,
  ToastBody,
  Toaster,
  ToastTitle,
  tokens,
  Tooltip,
  useId,
  useToastController,
} from "@fluentui/react-components";
import {
  Copy16Regular,
  History20Regular,
  Pin16Regular,
} from "@fluentui/react-icons";
import type { LayerTopologyResponse, TopologyNode } from "@eigenius/client";
import { EdgeKind, NodeKind } from "@eigenius/client";
import { useEigen } from "../../runtime/EigenProvider";
import { useNotebookStore } from "../../runtime/notebookStore";
import { formatAbsoluteIso, formatRelative } from "../../runtime/relativeTime";
import { CreateBranchDialog } from "../dialogs/CreateBranchDialog";
import { CreateTagDialog } from "../dialogs/CreateTagDialog";

const TOASTER_ID = "history-panel-toaster";

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
    display: "flex",
  },
  // Linear list — left two-thirds of the body. Long histories scroll
  // here; the detail pane stays fixed.
  list: {
    flex: "1 1 60%",
    minWidth: 0,
    minHeight: 0,
    overflowY: "auto",
    padding: tokens.spacingVerticalM,
  },
  detail: {
    flex: "0 0 360px",
    minHeight: 0,
    overflowY: "auto",
    borderLeft: `1px solid ${tokens.colorNeutralStroke2}`,
    background: tokens.colorNeutralBackground2,
    padding: tokens.spacingVerticalM,
  },
  row: {
    display: "flex",
    alignItems: "baseline",
    gap: tokens.spacingHorizontalM,
    padding: `${tokens.spacingVerticalS} ${tokens.spacingHorizontalM}`,
    borderRadius: tokens.borderRadiusMedium,
    cursor: "pointer",
    "&:hover": {
      background: tokens.colorNeutralBackground2Hover,
    },
  },
  rowSelected: {
    background: tokens.colorNeutralBackground2Selected,
    "&:hover": {
      background: tokens.colorNeutralBackground2Selected,
    },
  },
  rowGlyph: {
    width: "20px",
    textAlign: "center",
    flexShrink: 0,
    fontFamily: tokens.fontFamilyMonospace,
  },
  rowHash: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground3,
    flexShrink: 0,
    width: "90px",
  },
  rowName: {
    flex: 1,
    minWidth: 0,
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
  },
  rowTime: {
    flexShrink: 0,
    color: tokens.colorNeutralForeground3,
    fontSize: tokens.fontSizeBase200,
  },
  rowDelta: {
    flexShrink: 0,
    color: tokens.colorPaletteGreenForeground1,
    fontSize: tokens.fontSizeBase200,
    fontFamily: tokens.fontFamilyMonospace,
    width: "100px",
    textAlign: "right",
  },
  detailRow: {
    display: "grid",
    gridTemplateColumns: "100px 1fr",
    gap: tokens.spacingHorizontalS,
    rowGap: tokens.spacingVerticalXS,
    margin: `${tokens.spacingVerticalS} 0`,
  },
  detailLabel: {
    color: tokens.colorNeutralForeground3,
  },
  detailValue: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    wordBreak: "break-all",
  },
  pinnedBadge: {
    color: tokens.colorPaletteYellowForeground1,
    fontWeight: tokens.fontWeightSemibold,
  },
  loadingState: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalS,
    padding: tokens.spacingVerticalXXL,
    color: tokens.colorNeutralForeground3,
  },
});

interface HistoryRow {
  layerId: string;
  name: string;
  createdAtMs: number;
  /** Instance-resource count for this layer alone (per-layer addition). */
  resourceCount: number;
  parentCount: number;
  /** Glyph chosen from D34 §5.2. */
  glyph: "●" | "◆" | "⬚";
}

export function HistoryPanel() {
  const styles = useStyles();
  const eigen = useEigen();
  const toasterId = useId("toaster", TOASTER_ID);
  const { dispatchToast } = useToastController(toasterId);

  const activeBranch = useNotebookStore((s) => s.activeBranch);
  const branches = useNotebookStore((s) => s.branches);
  const refreshBranches = useNotebookStore((s) => s.refreshBranches);
  const readPinLayerId = useNotebookStore((s) => s.readPinLayerId);
  const setReadPin = useNotebookStore((s) => s.setReadPin);
  const setDestination = useNotebookStore((s) => s.setDestination);

  const [topology, setTopology] = useState<LayerTopologyResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [createTagFor, setCreateTagFor] = useState<string | null>(null);
  const [createBranchFor, setCreateBranchFor] = useState<string | null>(null);

  // Ensure the branches cache is populated so we can resolve the
  // active branch's head. Refresh on mount + when the user switches
  // branches.
  useEffect(() => {
    void refreshBranches(eigen);
  }, [eigen, refreshBranches, activeBranch]);

  const activeHead = useMemo(
    () => branches?.find((b) => b.name === activeBranch)?.headLayer ?? null,
    [branches, activeBranch],
  );

  useEffect(() => {
    let cancelled = false;
    if (!activeHead) {
      setTopology(null);
      return;
    }
    setError(null);
    setTopology(null);
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
          setError(err instanceof Error ? err.message : String(err));
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [eigen, activeHead]);

  const rows = useMemo<HistoryRow[]>(
    () => (topology ? buildHistoryRows(topology, activeHead) : []),
    [topology, activeHead],
  );

  // Default-select the tip after the topology loads.
  useEffect(() => {
    if (selected === null && rows.length > 0) {
      setSelected(rows[0].layerId);
    }
  }, [rows, selected]);

  const selectedRow = useMemo(
    () => rows.find((r) => r.layerId === selected) ?? null,
    [rows, selected],
  );

  const onTimeTravel = (layerId: string) => {
    setReadPin(layerId);
    dispatchToast(
      <Toast>
        <ToastTitle>Reading at {shortHash(layerId)}</ToastTitle>
        <ToastBody>
          Writes still go to the branch tip. Click "Return to tip" in the header
          to clear the pin.
        </ToastBody>
      </Toast>,
      { intent: "info", timeout: 6000 },
    );
  };

  return (
    <div className={styles.root}>
      <div className={styles.header}>
        <History20Regular />
        <Subtitle1 as="h2">History</Subtitle1>
        <Caption1 style={{ color: "var(--colorNeutralForeground3)" }}>
          branch: <strong>{activeBranch}</strong>
        </Caption1>
      </div>
      <div className={styles.body}>
        <div className={styles.list}>
          {error && (
            <MessageBar intent="error">
              <MessageBarBody>{error}</MessageBarBody>
            </MessageBar>
          )}
          {!error && topology === null && (
            <div className={styles.loadingState}>
              <Spinner size="tiny" /> <Caption1>walking chain…</Caption1>
            </div>
          )}
          {topology !== null && rows.length === 0 && (
            <Caption1>(no layers found)</Caption1>
          )}
          {rows.map((row) => (
            <HistoryRowView
              key={row.layerId}
              row={row}
              selected={row.layerId === selected}
              isReadPin={row.layerId === readPinLayerId}
              styles={styles}
              onSelect={() => setSelected(row.layerId)}
            />
          ))}
        </div>
        {selectedRow && (
          <aside className={styles.detail}>
            <DetailPanel
              row={selectedRow}
              styles={styles}
              isReadPin={selectedRow.layerId === readPinLayerId}
              onTimeTravel={() => onTimeTravel(selectedRow.layerId)}
              onCopy={() => {
                void navigator.clipboard?.writeText(selectedRow.layerId);
                dispatchToast(
                  <Toast>
                    <ToastTitle>Layer id copied</ToastTitle>
                  </Toast>,
                  { intent: "info", timeout: 2500 },
                );
              }}
              onCreateTag={() => setCreateTagFor(selectedRow.layerId)}
              onCreateBranch={() =>
                setCreateBranchFor(selectedRow.layerId)}
              onInspectResources={() => {
                setReadPin(selectedRow.layerId);
                setDestination("layer");
              }}
            />
          </aside>
        )}
      </div>
      <CreateTagDialog
        open={createTagFor !== null}
        onClose={() => setCreateTagFor(null)}
        defaultLayerId={createTagFor ?? undefined}
        onCreated={(name) =>
          dispatchToast(
            <Toast>
              <ToastTitle>Created tag {name}</ToastTitle>
            </Toast>,
            { intent: "success", timeout: 4000 },
          )}
      />
      <CreateBranchDialog
        open={createBranchFor !== null}
        onClose={() => setCreateBranchFor(null)}
        defaultLayerId={createBranchFor ?? undefined}
        onCreated={(name) =>
          dispatchToast(
            <Toast>
              <ToastTitle>Created branch {name}</ToastTitle>
            </Toast>,
            { intent: "success", timeout: 4000 },
          )}
      />
      <Toaster toasterId={toasterId} position="top-end" />
    </div>
  );
}

interface HistoryRowViewProps {
  row: HistoryRow;
  selected: boolean;
  isReadPin: boolean;
  styles: ReturnType<typeof useStyles>;
  onSelect: () => void;
}

function HistoryRowView({
  row,
  selected,
  isReadPin,
  styles,
  onSelect,
}: HistoryRowViewProps) {
  const classes = [styles.row, selected && styles.rowSelected]
    .filter(Boolean)
    .join(" ");
  return (
    <div
      className={classes}
      onClick={onSelect}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
    >
      <span className={styles.rowGlyph}>{row.glyph}</span>
      <span className={styles.rowHash}>{shortHash(row.layerId)}</span>
      <span className={styles.rowName}>
        {row.name}
        {isReadPin && (
          <>
            {" "}
            <span className={styles.pinnedBadge}>· reading here</span>
          </>
        )}
      </span>
      <span className={styles.rowTime}>{formatRelative(row.createdAtMs)}</span>
      <span className={styles.rowDelta}>
        {row.resourceCount} {row.resourceCount === 1 ? "resource" : "resources"}
      </span>
    </div>
  );
}

interface DetailPanelProps {
  row: HistoryRow;
  isReadPin: boolean;
  styles: ReturnType<typeof useStyles>;
  onTimeTravel: () => void;
  onCopy: () => void;
  onCreateTag: () => void;
  onCreateBranch: () => void;
  onInspectResources: () => void;
}

function DetailPanel({
  row,
  isReadPin,
  styles,
  onTimeTravel,
  onCopy,
  onCreateTag,
  onCreateBranch,
  onInspectResources,
}: DetailPanelProps) {
  return (
    <>
      <Subtitle1 as="h3">Layer {shortHash(row.layerId)}</Subtitle1>
      <div className={styles.detailRow}>
        <Caption1 className={styles.detailLabel}>Id</Caption1>
        <span className={styles.detailValue}>
          {row.layerId}{" "}
          <Tooltip relationship="label" content="Copy id">
            <Button
              appearance="subtle"
              size="small"
              icon={<Copy16Regular />}
              onClick={onCopy}
              aria-label="Copy layer id"
            />
          </Tooltip>
        </span>

        <Caption1 className={styles.detailLabel}>Name</Caption1>
        <span>{row.name}</span>

        <Caption1 className={styles.detailLabel}>Created</Caption1>
        <span>{formatAbsoluteIso(row.createdAtMs) || "—"}</span>

        <Caption1 className={styles.detailLabel}>Resources</Caption1>
        <span>
          {row.resourceCount} this layer
        </span>

        <Caption1 className={styles.detailLabel}>Parents</Caption1>
        <span>
          {row.parentCount === 0
            ? "(root)"
            : `${row.parentCount} parent${row.parentCount === 1 ? "" : "s"}`}
        </span>
      </div>

      <Divider />

      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: 8,
          marginTop: 12,
        }}
      >
        {
          /* "Time-travel here" — sets the session read-pin. The actual
            kernel reads pick it up via `atLayer` (D34 §5.2). When the
            currently-selected row IS the pin, label the button so
            the user can clear it from here too. */
        }
        <Tooltip
          relationship="description"
          content="Pin reads to this layer. Writes still go to the branch tip."
        >
          <Button
            appearance={isReadPin ? "secondary" : "primary"}
            icon={<Pin16Regular />}
            onClick={onTimeTravel}
            disabled={isReadPin}
          >
            {isReadPin ? "Currently pinned here" : "Time-travel here"}
          </Button>
        </Tooltip>

        {
          /* "Create tag" — pre-fills the dialog with this row's layer id
            so the user only has to type a name. */
        }
        <Tooltip
          relationship="description"
          content="Create an immutable named ref at this layer (also protects it from GC)."
        >
          <Button onClick={onCreateTag}>Create tag…</Button>
        </Tooltip>

        {
          /* "Create branch" — opens the Create-branch dialog with
            `Specific layer id` pre-filled to this row's layer id. */
        }
        <Tooltip
          relationship="description"
          content="Fork a new branch starting at this layer."
        >
          <Button onClick={onCreateBranch}>Create branch…</Button>
        </Tooltip>

        {
          /* "Inspect resources" sets the read-pin to this layer and
            navigates to the Layer-inspector destination, which lists
            every resource defined in *this* layer (not the inherited
            chain) with pretty-printed Eigon JSON. */
        }
        <Tooltip
          relationship="description"
          content="Open the layer inspector — every resource defined in this commit, with full Eigon JSON."
        >
          <Button onClick={onInspectResources}>Inspect resources…</Button>
        </Tooltip>
      </div>
    </>
  );
}

/** Compute the linearised history for the active branch.
 *
 *  Walks `parent_layer` edges from `head` down to the root, following
 *  the first parent at every merge (D23 §5.1, deterministic from the
 *  topology). Halts if a parent id can't be resolved in the topology
 *  (defensive — shouldn't happen on the kernel's output, but the
 *  walker tolerates missing nodes so we don't crash the panel). */
function buildHistoryRows(
  topology: LayerTopologyResponse,
  head: string | null,
): HistoryRow[] {
  if (!head) return [];
  // Index layer nodes by id.
  const layerNodes = new Map<string, TopologyNode>();
  for (const n of topology.nodes) {
    if (n.kind === NodeKind.LAYER) layerNodes.set(n.id, n);
  }
  // Index parent_layer edges by source. A source with N edges means
  // N parents — usually 1 (linear) or 2+ (merge).
  const parentsBySource = new Map<string, string[]>();
  for (const e of topology.edges) {
    if (e.kind !== EdgeKind.PARENT_LAYER) continue;
    const list = parentsBySource.get(e.source) ?? [];
    list.push(e.target);
    parentsBySource.set(e.source, list);
  }

  const rows: HistoryRow[] = [];
  const visited = new Set<string>();
  let cursor: string | null = head;
  while (cursor && !visited.has(cursor)) {
    visited.add(cursor);
    const node = layerNodes.get(cursor);
    if (!node) break;
    const parents: string[] = parentsBySource.get(cursor) ?? [];
    const resourceCount = parseInt(node.attrs.resource_count ?? "0", 10);
    const createdAtMs = parseInt(node.attrs.created_at_ms ?? "0", 10);
    const glyph: HistoryRow["glyph"] = parents.length === 0
      ? "⬚"
      : parents.length >= 2
      ? "◆"
      : "●";
    rows.push({
      layerId: cursor,
      name: node.attrs.name ?? node.label ?? "(unnamed)",
      createdAtMs,
      resourceCount,
      parentCount: parents.length,
      glyph,
    });
    cursor = parents[0] ?? null;
  }
  return rows;
}

function shortHash(hex: string): string {
  if (hex.length <= 10) return hex;
  return `${hex.slice(0, 4)}…${hex.slice(-4)}`;
}
