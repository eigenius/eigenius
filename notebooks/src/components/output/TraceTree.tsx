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
 * Collapsible trace tree (D22 §6.7 / §6.9).
 *
 * Renders a `TraceNode` hierarchy as an indented list with a toggle
 * twistie for nodes that have children. Each node shows the trace
 * kind, a short label, and key/value summary lines (token counts,
 * latency, etc. for `ComponentTrace`).
 *
 * d3-hierarchy is loaded for future tree-layout work; for the MVP an
 * indented list is the right shape (it scales linearly with depth and
 * doesn't horizontally clip in the cell card).
 */

import { useMemo, useState } from "react";
import {
  Body1Strong,
  Caption1,
  makeStyles,
  Tag,
  tokens,
} from "@fluentui/react-components";
import {
  ChevronDown16Regular,
  ChevronRight16Regular,
} from "@fluentui/react-icons";
import {
  decodeTraceResource,
  type TraceNode,
} from "../../runtime/traceResource";

const useStyles = makeStyles({
  root: {
    fontFamily: tokens.fontFamilyBase,
    fontSize: tokens.fontSizeBase200,
  },
  node: {
    paddingLeft: tokens.spacingHorizontalS,
    paddingTop: tokens.spacingVerticalXXS,
  },
  row: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalXS,
  },
  twistie: {
    display: "inline-flex",
    width: "16px",
    height: "16px",
    flexShrink: 0,
    color: tokens.colorNeutralForeground3,
    cursor: "pointer",
  },
  twistiePlaceholder: {
    display: "inline-block",
    width: "16px",
    height: "16px",
    flexShrink: 0,
  },
  label: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
  },
  kindTag: {
    fontSize: tokens.fontSizeBase100,
  },
  summary: {
    paddingLeft: "20px",
    color: tokens.colorNeutralForeground2,
    display: "flex",
    flexDirection: "column",
    gap: "2px",
  },
  summaryRow: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase100,
  },
  children: {
    marginLeft: tokens.spacingHorizontalS,
    borderLeft: `1px dashed ${tokens.colorNeutralStroke3}`,
    paddingLeft: tokens.spacingHorizontalXS,
  },
});

export interface TraceTreeProps {
  /** CBOR-encoded program-trace resource. */
  trace: Uint8Array;
}

export function TraceTree({ trace }: TraceTreeProps) {
  const root = useMemo(() => decodeTraceResource(trace), [trace]);
  const styles = useStyles();
  return (
    <div className={styles.root}>
      <TreeNode node={root} initiallyExpanded styles={styles} />
    </div>
  );
}

interface TreeNodeProps {
  node: TraceNode;
  initiallyExpanded?: boolean;
  styles: ReturnType<typeof useStyles>;
}

function TreeNode({ node, initiallyExpanded = false, styles }: TreeNodeProps) {
  const [expanded, setExpanded] = useState(initiallyExpanded);
  const hasChildren = node.children.length > 0;

  return (
    <div className={styles.node}>
      <div className={styles.row}>
        {hasChildren
          ? (
            <span
              className={styles.twistie}
              onClick={() => setExpanded((v) => !v)}
              role="button"
              aria-label={expanded ? "Collapse" : "Expand"}
            >
              {expanded ? <ChevronDown16Regular /> : <ChevronRight16Regular />}
            </span>
          )
          : <span className={styles.twistiePlaceholder} />}
        <Tag size="extra-small" className={styles.kindTag}>
          {node.kind.replace(/Trace$/, "")}
        </Tag>
        <Body1Strong className={styles.label}>{node.label}</Body1Strong>
      </div>
      {node.summary.length > 0 && (
        <div className={styles.summary}>
          {node.summary.map((s) => (
            <Caption1 key={s.key} className={styles.summaryRow}>
              {s.key}: {s.value}
            </Caption1>
          ))}
        </div>
      )}
      {hasChildren && expanded && (
        <div className={styles.children}>
          {node.children.map((child, idx) => (
            <TreeNode
              key={idx}
              node={child}
              initiallyExpanded={node.children.length === 1}
              styles={styles}
            />
          ))}
        </div>
      )}
    </div>
  );
}
