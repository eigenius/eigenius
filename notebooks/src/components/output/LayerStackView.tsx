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

import { useMemo } from "react";
import {
  Body1Strong,
  Caption1,
  makeStyles,
  tokens,
} from "@fluentui/react-components";
import {
  EdgeKind,
  type LayerTopologyResponse,
  NodeKind,
  type TopologyEdge,
  type TopologyNode,
} from "@eigenius/client";

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
});

export interface LayerStackViewProps {
  topology: LayerTopologyResponse;
}

export function LayerStackView({ topology }: LayerStackViewProps) {
  const styles = useStyles();
  const stack = useMemo(() => orderLayersHeadFirst(topology), [topology]);

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
          styles={styles}
        />
      ))}
    </div>
  );
}

interface LayerBoxProps {
  layer: TopologyNode;
  isRoot: boolean;
  isLast: boolean;
  styles: ReturnType<typeof useStyles>;
}

function LayerBox({ layer, isRoot, isLast, styles }: LayerBoxProps) {
  const counts = readCounts(layer);
  return (
    <>
      <div
        className={`${styles.layerCard} ${
          isRoot ? styles.rootLayerCard : ""
        }`.trim()}
      >
        <div className={styles.label}>
          <Body1Strong>{layer.label || "(unnamed layer)"}</Body1Strong>
          <span className={styles.layerId}>{layer.id.slice(0, 12)}…</span>
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
