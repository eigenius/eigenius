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
 * Full intra-layer topology graph (D22 §6.7 + §6.9, Phase 5b).
 *
 * Renders the nodes + edges from a `LayerTopologyResponse` as a
 * directed graph via `@xyflow/react`. Node positions are computed by
 * `dagre` (hierarchical layout). Each `NodeKind` gets its own colour
 * + label style; each `EdgeKind` gets its own stroke colour and
 * label.
 *
 * Used by the TS-cell auto-renderer for `LayerTopologyResponse`s that
 * contain non-LAYER nodes (Class / Property / Resource / Institution).
 * Pure layer chains (only LAYER nodes + PARENT_LAYER edges) keep
 * going to `LayerStackView` since the boxes-and-arrows shape conveys
 * a chain better than a graph layout does.
 */

import { useMemo, useState } from "react";
import {
  Button,
  Dialog,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  DialogTrigger,
  makeStyles,
  tokens,
} from "@fluentui/react-components";
import {
  Dismiss20Regular,
  FullScreenMaximize20Regular,
} from "@fluentui/react-icons";
import {
  Background,
  Controls,
  type Edge,
  type Node,
  ReactFlow,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import dagre from "@dagrejs/dagre";
import {
  EdgeKind,
  type LayerTopologyResponse,
  NodeKind,
  type TopologyEdge,
  type TopologyNode,
} from "@eigenius/client";

const useStyles = makeStyles({
  root: {
    position: "relative",
    width: "100%",
    height: "560px",
    border: `1px solid ${tokens.colorNeutralStroke2}`,
    borderRadius: tokens.borderRadiusMedium,
    background: tokens.colorNeutralBackground1,
    overflow: "hidden",
  },
  expandButton: {
    position: "absolute",
    top: tokens.spacingVerticalS,
    right: tokens.spacingHorizontalS,
    zIndex: 5,
    background: tokens.colorNeutralBackground1,
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

// Visual style per node kind. Background colour + a friendly badge.
const NODE_STYLE: Record<
  NodeKind,
  { bg: string; fg: string; border: string; label: string }
> = {
  [NodeKind.UNSPECIFIED]: {
    bg: "#eee",
    fg: "#333",
    border: "#bbb",
    label: "?",
  },
  [NodeKind.LAYER]: {
    bg: "#e8e8e8",
    fg: "#222",
    border: "#888",
    label: "Layer",
  },
  [NodeKind.CLASS]: {
    bg: "#dfe8ff",
    fg: "#0a4499",
    border: "#5b88c5",
    label: "Class",
  },
  [NodeKind.PROPERTY]: {
    bg: "#e2f4e6",
    fg: "#1a5c2a",
    border: "#37a172",
    label: "Property",
  },
  [NodeKind.RESOURCE]: {
    bg: "#fff0e0",
    fg: "#7a3f10",
    border: "#cf6f1e",
    label: "Resource",
  },
  [NodeKind.INSTITUTION]: {
    bg: "#f1e3f3",
    fg: "#5a2160",
    border: "#a45fa1",
    label: "Institution",
  },
};

// Visual style per edge kind. Stroke colour + a short label that
// xyflow renders on the edge midpoint.
const EDGE_STYLE: Record<
  EdgeKind,
  { stroke: string; label: string; strokeDasharray?: string }
> = {
  [EdgeKind.UNSPECIFIED]: { stroke: "#888", label: "" },
  [EdgeKind.PARENT_LAYER]: {
    stroke: "#999",
    label: "parent",
    strokeDasharray: "4 3",
  },
  [EdgeKind.IS_A]: { stroke: "#5b88c5", label: "is_a" },
  [EdgeKind.SUBCLASS_OF]: { stroke: "#0a4499", label: "subclass_of" },
  [EdgeKind.REQUIRES]: { stroke: "#37a172", label: "requires" },
  [EdgeKind.RECOMMENDS]: {
    stroke: "#37a172",
    label: "recommends",
    strokeDasharray: "4 3",
  },
  [EdgeKind.PROPERTY_REF]: { stroke: "#cf6f1e", label: "type" },
  [EdgeKind.INSTITUTION_DECLARES]: {
    stroke: "#a45fa1",
    label: "declares",
  },
};

export interface TopologyGraphViewProps {
  topology: LayerTopologyResponse;
  /**
   * Drop PARENT_LAYER edges from the rendered graph. The layer chain
   * is already conveyed by `LayerStackView`, and including the edges
   * here clutters the structural view. Default: true.
   */
  hideParentLayerEdges?: boolean;
  /**
   * Optional title shown in the full-screen dialog header. Defaults
   * to "Topology graph".
   */
  title?: string;
}

export function TopologyGraphView(
  { topology, hideParentLayerEdges = true, title = "Topology graph" }:
    TopologyGraphViewProps,
) {
  const styles = useStyles();
  const [fullScreen, setFullScreen] = useState(false);
  const { nodes, edges } = useMemo(
    () => buildGraph(topology, hideParentLayerEdges),
    [topology, hideParentLayerEdges],
  );

  return (
    <div className={styles.root}>
      <Button
        size="small"
        appearance="subtle"
        icon={<FullScreenMaximize20Regular />}
        title="Open in full screen"
        aria-label="Open in full screen"
        className={styles.expandButton}
        onClick={() => setFullScreen(true)}
      />
      <Graph nodes={nodes} edges={edges} />

      <Dialog
        open={fullScreen}
        onOpenChange={(_e, data) => setFullScreen(data.open)}
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
            {
              /* action={null} suppresses Fluent's default close-button
                in the title's action slot — we use the absolutely-
                positioned X above so it stays anchored when the
                title grows to two lines. */
            }
            <DialogTitle action={null}>
              <div className={styles.dialogTitle}>
                <span>{title}</span>
                <span className={styles.dialogTitleSummary}>
                  {nodes.length} nodes · {edges.length} edges
                </span>
              </div>
            </DialogTitle>
            <DialogContent className={styles.fullScreenContent}>
              <div className={styles.fullScreenGraph}>
                {
                  /* Re-mount on open so xyflow's fitView recalculates
                    against the new viewport size. */
                }
                {fullScreen && <Graph nodes={nodes} edges={edges} />}
              </div>
            </DialogContent>
          </DialogBody>
        </DialogSurface>
      </Dialog>
    </div>
  );
}

interface GraphProps {
  nodes: Node[];
  edges: Edge[];
}

function Graph({ nodes, edges }: GraphProps) {
  return (
    <ReactFlow
      nodes={nodes}
      edges={edges}
      fitView
      // Pad enough to keep edge labels inside the viewport, allow
      // zooming far enough out to see big graphs (the patent demo's
      // class hierarchy fans out wide), and let the user pinch-zoom
      // close in for inspection.
      fitViewOptions={{ padding: 0.2, includeHiddenNodes: false }}
      minZoom={0.05}
      maxZoom={4}
      nodesDraggable={true}
      elementsSelectable={true}
      proOptions={{ hideAttribution: true }}
    >
      <Background gap={16} size={1} />
      <Controls position="bottom-right" showInteractive={false} />
    </ReactFlow>
  );
}

interface BuiltGraph {
  nodes: Node[];
  edges: Edge[];
}

function buildGraph(
  topology: LayerTopologyResponse,
  hideParentLayerEdges: boolean,
): BuiltGraph {
  // Drop edges whose source or target isn't in the node set — xyflow
  // silently skips rendering them, but they'd otherwise inflate the
  // displayed "N edges" count and mislead the user. Common cause:
  // upstream filter (e.g., the kinase notebook's namespace prefix
  // scope) prunes target nodes but leaves edges that point at them.
  const nodeIds = new Set(topology.nodes.map((n) => n.id));
  const visibleEdges = topology.edges.filter((e) => {
    if (hideParentLayerEdges && e.kind === EdgeKind.PARENT_LAYER) return false;
    return nodeIds.has(e.source) && nodeIds.has(e.target);
  });

  const xnodes: Node[] = topology.nodes.map((n) => buildNode(n));
  const xedges: Edge[] = visibleEdges.map((e, i) => buildEdge(e, i));

  // Compute positions via dagre.
  const g = new dagre.graphlib.Graph();
  g.setDefaultEdgeLabel(() => ({}));
  g.setGraph({
    rankdir: "LR",
    nodesep: 40,
    ranksep: 80,
    marginx: 20,
    marginy: 20,
  });
  for (const n of xnodes) {
    g.setNode(n.id, { width: 180, height: 48 });
  }
  for (const e of xedges) {
    g.setEdge(e.source, e.target);
  }
  dagre.layout(g);

  for (const n of xnodes) {
    const pos = g.node(n.id);
    if (pos) {
      // dagre returns center; xyflow uses top-left.
      n.position = { x: pos.x - 90, y: pos.y - 24 };
    } else {
      n.position = { x: 0, y: 0 };
    }
  }

  return { nodes: xnodes, edges: xedges };
}

function buildNode(node: TopologyNode): Node {
  const style = NODE_STYLE[node.kind] ?? NODE_STYLE[NodeKind.UNSPECIFIED];
  return {
    id: node.id,
    position: { x: 0, y: 0 },
    data: {
      label: (
        <div
          style={{
            textAlign: "center",
            fontSize: "11px",
            lineHeight: 1.2,
          }}
        >
          <div
            style={{
              color: style.fg,
              fontSize: "9px",
              textTransform: "uppercase",
              letterSpacing: "0.05em",
              opacity: 0.7,
              marginBottom: "2px",
            }}
          >
            {style.label}
          </div>
          <div style={{ fontWeight: 600, color: style.fg }}>
            {node.label || shortenIri(node.id)}
          </div>
        </div>
      ),
    },
    style: {
      background: style.bg,
      border: `1px solid ${style.border}`,
      borderRadius: 6,
      padding: 6,
      width: 180,
    },
  };
}

function buildEdge(edge: TopologyEdge, index: number): Edge {
  const style = EDGE_STYLE[edge.kind] ?? EDGE_STYLE[EdgeKind.UNSPECIFIED];
  return {
    id: `${edge.source}--${edge.kind}-${index}-->${edge.target}`,
    source: edge.source,
    target: edge.target,
    label: style.label,
    labelStyle: {
      fontSize: 9,
      fill: style.stroke,
    },
    labelBgPadding: [3, 1],
    labelBgStyle: { fill: "#fff", opacity: 0.85 },
    style: {
      stroke: style.stroke,
      strokeWidth: 1.5,
      strokeDasharray: style.strokeDasharray,
    },
    type: "smoothstep",
  };
}

function shortenIri(iri: string): string {
  const colon = iri.lastIndexOf(":");
  return colon >= 0 ? iri.slice(colon + 1) : iri;
}
