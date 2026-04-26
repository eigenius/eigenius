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
 * Plain JSX/CSS layer-chain visualisation (D22 §6.7 / §6.9).
 *
 * Renders the parent chain returned by `eigen.layerTopology()` as a
 * vertical stack of boxes — head at top, root at bottom — with each
 * box showing the layer's label and per-kind counts (classes,
 * properties, institutions, resources). No graph library; the model
 * IS a chain of immutable parent pointers, and the boxes-and-arrows
 * shape conveys that immediately.
 *
 * Click-to-inspect drilldown is a Phase 4c+ enhancement.
 */

import { useMemo, useState } from "react";
import {
  Body1Strong,
  Button,
  Caption1,
  Dialog,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  DialogTrigger,
  makeStyles,
  tokens,
} from "@fluentui/react-components";
import { Dismiss20Regular, Share20Regular } from "@fluentui/react-icons";
import {
  EdgeKind,
  type LayerTopologyResponse,
  NodeKind,
  type TopologyEdge,
  type TopologyNode,
} from "@eigenius/client";
import { TopologyGraphView } from "./TopologyGraphView";

const useStyles = makeStyles({
  root: {
    display: "flex",
    flexDirection: "column",
    alignItems: "center",
    gap: tokens.spacingVerticalXS,
  },
  layerCard: {
    width: "100%",
    maxWidth: "520px",
    padding: tokens.spacingVerticalS,
    border: `1px solid ${tokens.colorNeutralStroke2}`,
    borderRadius: tokens.borderRadiusMedium,
    background: tokens.colorNeutralBackground1,
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalXXS,
  },
  rootLayerCard: {
    // Faintly distinguish the root (core) layer.
    borderTopColor: tokens.colorNeutralStrokeAccessible,
    borderRightColor: tokens.colorNeutralStrokeAccessible,
    borderBottomColor: tokens.colorNeutralStrokeAccessible,
    borderLeftColor: tokens.colorNeutralStrokeAccessible,
    background: tokens.colorNeutralBackground2,
  },
  label: {
    display: "flex",
    alignItems: "baseline",
    gap: tokens.spacingHorizontalS,
  },
  layerId: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground3,
  },
  counts: {
    display: "flex",
    flexWrap: "wrap",
    gap: tokens.spacingHorizontalM,
    color: tokens.colorNeutralForeground2,
    fontSize: tokens.fontSizeBase200,
  },
  count: {
    display: "inline-flex",
    gap: tokens.spacingHorizontalXXS,
  },
  countNumber: {
    fontWeight: tokens.fontWeightSemibold,
    color: tokens.colorNeutralForeground1,
  },
  arrow: {
    color: tokens.colorNeutralForeground3,
    fontSize: tokens.fontSizeBase300,
    lineHeight: 1,
  },
  empty: {
    color: tokens.colorNeutralForeground3,
    fontStyle: "italic",
  },
  layerHeaderRow: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: tokens.spacingHorizontalS,
  },
  fullScreenSurface: {
    width: "95vw",
    maxWidth: "none",
    height: "92vh",
    display: "flex",
    flexDirection: "column",
    position: "relative",
  },
  fullScreenClose: {
    position: "absolute",
    top: tokens.spacingVerticalS,
    right: tokens.spacingHorizontalS,
    zIndex: 1,
  },
  fullScreenBody: {
    flex: 1,
    minHeight: 0,
    display: "flex",
    flexDirection: "column",
  },
  fullScreenContent: {
    flex: 1,
    minHeight: 0,
    width: "100%",
  },
  fullScreenGraph: {
    width: "100%",
    height: "100%",
    minHeight: "70vh",
  },
  dialogTitle: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalXXS,
  },
  dialogTitleSummary: {
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground3,
    fontWeight: tokens.fontWeightRegular,
  },
});

export interface LayerStackViewProps {
  topology: LayerTopologyResponse;
}

export function LayerStackView({ topology }: LayerStackViewProps) {
  const styles = useStyles();
  const stack = useMemo(() => orderLayersHeadFirst(topology), [topology]);
  const [openLayer, setOpenLayer] = useState<TopologyNode | null>(null);

  // Build the per-layer subgraph when a layer is selected. The kernel
  // tags each non-LAYER node with `attrs.layer_id` (the layer that
  // first declared it); we filter the full topology client-side.
  const layerSubgraph = useMemo<LayerTopologyResponse | null>(() => {
    if (!openLayer) return null;
    const layerId = openLayer.id;
    const keepNode = (n: TopologyNode): boolean => {
      if (n.id === layerId) return true; // include the layer node itself
      if (n.kind === NodeKind.LAYER) return false; // drop other layers
      return n.attrs?.layer_id === layerId;
    };
    const nodes = topology.nodes.filter(keepNode);
    const ids = new Set(nodes.map((n) => n.id));
    const edges = topology.edges.filter(
      (e) =>
        // Drop parent_layer edges (only one layer in this view) and
        // any edge whose endpoints aren't in the filtered set.
        e.kind !== EdgeKind.PARENT_LAYER && ids.has(e.source) &&
        ids.has(e.target),
    );
    return {
      ...topology,
      nodes,
      edges,
    } as LayerTopologyResponse;
  }, [openLayer, topology]);

  if (stack.length === 0) {
    return (
      <Caption1 className={styles.empty}>
        topology has no layer nodes
      </Caption1>
    );
  }

  return (
    <div className={styles.root}>
      {stack.map((layer, idx) => (
        <LayerBox
          key={layer.id}
          layer={layer}
          isRoot={idx === stack.length - 1}
          isLast={idx === stack.length - 1}
          onOpenGraph={() => setOpenLayer(layer)}
          styles={styles}
        />
      ))}
      <Dialog
        open={openLayer !== null}
        onOpenChange={(_e, data) => {
          if (!data.open) setOpenLayer(null);
        }}
        modalType="non-modal"
      >
        <DialogSurface className={styles.fullScreenSurface}>
          <DialogTrigger disableButtonEnhancement>
            <Button
              size="small"
              appearance="subtle"
              icon={<Dismiss20Regular />}
              aria-label="Close"
              className={styles.fullScreenClose}
            />
          </DialogTrigger>
          <DialogBody className={styles.fullScreenBody}>
            {/* action={null} suppresses Fluent's default close-button
                in the title's action slot — we use the absolutely-
                positioned X above so it stays anchored when the
                title grows to two lines. */}
            <DialogTitle action={null}>
              <div className={styles.dialogTitle}>
                <span>
                  Layer · {openLayer?.label || "(unnamed layer)"}
                </span>
                <span className={styles.dialogTitleSummary}>
                  {layerSubgraph?.nodes.length ?? 0} nodes · {layerSubgraph
                    ?.edges.length ?? 0} edges
                </span>
              </div>
            </DialogTitle>
            <DialogContent className={styles.fullScreenContent}>
              <div className={styles.fullScreenGraph}>
                {layerSubgraph && layerSubgraph.nodes.length > 1
                  ? (
                    <TopologyGraphView
                      topology={layerSubgraph}
                      title={`Layer · ${openLayer?.label}`}
                    />
                  )
                  : (
                    <Caption1 className={styles.empty}>
                      This layer added no class / property / institution /
                      resource nodes — only its layer marker. Nothing to
                      graph.
                    </Caption1>
                  )}
              </div>
            </DialogContent>
          </DialogBody>
        </DialogSurface>
      </Dialog>
    </div>
  );
}

interface LayerBoxProps {
  layer: TopologyNode;
  isRoot: boolean;
  isLast: boolean;
  onOpenGraph: () => void;
  styles: ReturnType<typeof useStyles>;
}

function LayerBox(
  { layer, isRoot, isLast, onOpenGraph, styles }: LayerBoxProps,
) {
  const counts = readCounts(layer);
  const hasContent = (counts.classes ?? 0) + (counts.properties ?? 0) +
      (counts.institutions ?? 0) + (counts.resources ?? 0) > 0;
  return (
    <>
      <div
        className={`${styles.layerCard} ${
          isRoot ? styles.rootLayerCard : ""
        }`.trim()}
      >
        <div className={styles.layerHeaderRow}>
          <div className={styles.label}>
            <Body1Strong>{layer.label || "(unnamed layer)"}</Body1Strong>
            <span className={styles.layerId}>{layer.id.slice(0, 12)}…</span>
          </div>
          {hasContent && (
            <Button
              size="small"
              appearance="subtle"
              icon={<Share20Regular />}
              title="Open this layer's topology graph"
              aria-label="Open this layer's topology graph"
              onClick={onOpenGraph}
            />
          )}
        </div>
        <div className={styles.counts}>
          <CountBadge label="classes" value={counts.classes} styles={styles} />
          <CountBadge
            label="properties"
            value={counts.properties}
            styles={styles}
          />
          <CountBadge
            label="institutions"
            value={counts.institutions}
            styles={styles}
          />
          <CountBadge
            label="resources"
            value={counts.resources}
            styles={styles}
          />
        </div>
      </div>
      {!isLast && <span className={styles.arrow}>↑</span>}
    </>
  );
}

function CountBadge(
  { label, value, styles }: {
    label: string;
    value: number | undefined;
    styles: ReturnType<typeof useStyles>;
  },
) {
  return (
    <span className={styles.count}>
      <span className={styles.countNumber}>{value ?? 0}</span>
      {" "}
      {label}
    </span>
  );
}

interface LayerCounts {
  classes?: number;
  properties?: number;
  institutions?: number;
  resources?: number;
}

function readCounts(node: TopologyNode): LayerCounts {
  // The kernel's LayerTopology walker stamps these on LAYER nodes when
  // include_resources=false (kernel/src/server/topology.rs); see D22 §4.2.
  const a = node.attrs ?? {};
  return {
    classes: parseIntOrUndef(a.class_count),
    properties: parseIntOrUndef(a.property_count),
    institutions: parseIntOrUndef(a.institution_count),
    resources: parseIntOrUndef(a.resource_count),
  };
}

function parseIntOrUndef(v: string | undefined): number | undefined {
  if (v === undefined) return undefined;
  const n = Number(v);
  return Number.isFinite(n) ? n : undefined;
}

/**
 * Order LAYER nodes head-first by following PARENT_LAYER edges. The
 * topology response is order-agnostic; we recover the chain by starting
 * at the head (the layer that no PARENT_LAYER edge points TO from a
 * child — equivalently, the layer that isn't a parent of any other
 * layer in the response) and walking parent pointers down to the root.
 */
function orderLayersHeadFirst(
  topology: LayerTopologyResponse,
): TopologyNode[] {
  const layers = topology.nodes.filter((n) => n.kind === NodeKind.LAYER);
  if (layers.length === 0) return [];

  const byId = new Map(layers.map((n) => [n.id, n]));

  // Build a child→parent map from PARENT_LAYER edges.
  const parentOf = new Map<string, string>();
  for (const edge of topology.edges) {
    if (edge.kind === EdgeKind.PARENT_LAYER) {
      parentOf.set(edge.source, edge.target);
    }
  }

  // The head is a layer that no other layer claims as a parent — i.e.
  // it never appears as the *target* of a PARENT_LAYER edge.
  const targetIds = new Set(
    topology.edges
      .filter((e: TopologyEdge) => e.kind === EdgeKind.PARENT_LAYER)
      .map((e) => e.target),
  );
  const heads = layers.filter((l) => !targetIds.has(l.id));
  // If the chain is well-formed there's exactly one head; if it's
  // malformed we still render whatever single layer we can identify.
  const start = heads[0] ?? layers[0];

  const ordered: TopologyNode[] = [];
  const visited = new Set<string>();
  let cursor: string | undefined = start.id;
  while (cursor && !visited.has(cursor)) {
    visited.add(cursor);
    const node = byId.get(cursor);
    if (!node) break;
    ordered.push(node);
    cursor = parentOf.get(cursor);
  }
  return ordered;
}
