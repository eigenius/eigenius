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
 * Auto-renderer for TypeScript-cell return values (D22 §6.7).
 *
 * Dispatches based on the value's runtime shape:
 *  - DOM `Node` → mounted directly
 *  - `Uint8Array` → ResourceInspector (assumed CBOR resource)
 *  - QueryResponse-shape (`{document: Uint8Array}`) → ResultTable
 *  - RunProgramResponse-shape (`{output: Uint8Array, traceIri?}`)
 *      → ResourceInspector with optional trace IRI
 *  - LoadResponse-shape (`{success, layerId, resourceCount}`)
 *      → status panel
 *  - InspectResponse-shape (`{found: bool, resource: Uint8Array}`)
 *      → ResourceInspector or "not found" panel
 *  - Plain object/array → JSON tree
 *  - Primitive → text
 *  - `null` / `undefined` → italic placeholder
 *
 * Phase 4c will add `Topology` → LayerStackView and any value with a
 * `trace` field → split panel with TraceTree.
 */

import { isValidElement, useEffect, useRef } from "react";
import {
  Body1,
  Caption1,
  makeStyles,
  tokens,
} from "@fluentui/react-components";
import { LayerStackView } from "./LayerStackView";
import { ResourceInspector } from "./ResourceInspector";
import { ResultTable } from "./ResultTable";
import { TopologyGraphView } from "./TopologyGraphView";
import { TraceTreePanel } from "./TraceTreePanel";
import { NodeKind } from "@eigenius/client";

const useStyles = makeStyles({
  log: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    background: tokens.colorNeutralBackground3,
    padding: `${tokens.spacingVerticalXS} ${tokens.spacingHorizontalS}`,
    borderRadius: tokens.borderRadiusSmall,
    margin: 0,
    marginBottom: tokens.spacingVerticalS,
    whiteSpace: "pre-wrap",
    wordBreak: "break-word",
    color: tokens.colorNeutralForeground2,
  },
  splitPanel: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalS,
  },
  panelLabel: {
    color: tokens.colorNeutralForeground3,
    textTransform: "uppercase",
    letterSpacing: "0.04em",
    fontSize: tokens.fontSizeBase100,
  },
  status: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalXXS,
    color: tokens.colorPaletteGreenForeground2,
  },
  meta: {
    color: tokens.colorNeutralForeground3,
    fontFamily: tokens.fontFamilyMonospace,
  },
  jsonPre: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    margin: 0,
    whiteSpace: "pre-wrap",
    wordBreak: "break-word",
  },
  placeholder: {
    color: tokens.colorNeutralForeground3,
    fontStyle: "italic",
  },
});

export interface TypeScriptValueViewProps {
  value: unknown;
  log: readonly string[];
}

export function TypeScriptValueView(
  { value, log }: TypeScriptValueViewProps,
) {
  const styles = useStyles();
  return (
    <div>
      {log.length > 0 && <pre className={styles.log}>{log.join("\n")}</pre>}
      {renderValue(value, styles)}
    </div>
  );
}

function renderValue(value: unknown, styles: ReturnType<typeof useStyles>) {
  if (value === null || value === undefined) {
    return (
      <Caption1 className={styles.placeholder}>
        {value === null ? "null" : "no return value"}
      </Caption1>
    );
  }

  // React element — mount directly. This is the path Phase 5a chart
  // cells use: `return h(charts.GroupedVerticalBarChart, { … })`.
  if (isValidElement(value)) {
    return <>{value}</>;
  }

  // DOM node — mount directly via a small adapter component.
  if (value instanceof Node) {
    return <DomNodeMount node={value} />;
  }

  // Raw CBOR bytes — assume a resource.
  if (value instanceof Uint8Array) {
    return <ResourceInspector resource={value} />;
  }

  if (typeof value !== "object") {
    // Primitive — string, number, boolean, bigint, symbol, function.
    return <Body1>{formatPrimitive(value)}</Body1>;
  }

  // Object/array shape detection — duck-typed against the SDK responses.
  const obj = value as Record<string, unknown>;

  // LayerTopologyResponse: { nodes: TopologyNode[], edges: TopologyEdge[] }
  // Layer-only chains render as the linear stack; richer responses
  // (containing Class / Property / Resource / Institution nodes)
  // render as the full xyflow topology graph.
  if (
    Array.isArray(obj.nodes) &&
    Array.isArray(obj.edges) &&
    looksLikeTopologyNode(obj.nodes[0])
  ) {
    const hasNonLayerNodes = obj.nodes.some(
      (n) =>
        looksLikeTopologyNode(n) &&
        (n as { kind: number }).kind !== NodeKind.LAYER,
    );
    return hasNonLayerNodes
      ? (
        <TopologyGraphView
          // The duck-typed cast keeps the public component strictly typed.
          // deno-lint-ignore no-explicit-any
          topology={value as any}
        />
      )
      : (
        <LayerStackView
          // deno-lint-ignore no-explicit-any
          topology={value as any}
        />
      );
  }

  // QueryResponse: { success, document, ... }
  if (
    obj.document instanceof Uint8Array &&
    typeof obj.success === "boolean"
  ) {
    if (obj.success === false) {
      return (
        <Body1>
          query failed: {String(obj.error ?? "(no error message)")}
        </Body1>
      );
    }
    return <ResultTable document={obj.document} />;
  }

  // RunProgramResponse: { success, output, traceIri?, errors? }
  if (
    obj.output instanceof Uint8Array &&
    typeof obj.success === "boolean"
  ) {
    if (obj.success === false) {
      const errs = Array.isArray(obj.errors) ? obj.errors : [];
      const lines = errs
        .map((e) => {
          if (!e || typeof e !== "object") return String(e);
          const er = e as Record<string, unknown>;
          const rule = typeof er.rule === "string" && er.rule.length > 0
            ? `[${er.rule}] `
            : "";
          const msg = typeof er.message === "string" ? er.message : "";
          return `${rule}${msg}`;
        })
        .filter((s) => s.length > 0);
      return (
        <div>
          <Body1>Program failed</Body1>
          {lines.length > 0 && (
            <pre className={styles.jsonPre}>{lines.join("\n")}</pre>
          )}
        </div>
      );
    }
    const traceIri = typeof obj.traceIri === "string" && obj.traceIri.length > 0
      ? obj.traceIri
      : undefined;
    // When the kernel produced a trace, render typed output + trace
    // tree side-by-side (vertically stacked here for readability inside
    // the cell card; D22 §6.7).
    if (traceIri) {
      return (
        <div className={styles.splitPanel}>
          <div>
            <Caption1 className={styles.panelLabel}>output</Caption1>
            <ResourceInspector resource={obj.output} traceIri={traceIri} />
          </div>
          <div>
            <Caption1 className={styles.panelLabel}>trace</Caption1>
            <TraceTreePanel traceIri={traceIri} />
          </div>
        </div>
      );
    }
    return <ResourceInspector resource={obj.output} />;
  }

  // InspectResponse: { found: boolean, resource: Uint8Array }
  if (
    typeof obj.found === "boolean" &&
    obj.resource instanceof Uint8Array
  ) {
    if (!obj.found) {
      return <Body1 className={styles.placeholder}>not found</Body1>;
    }
    return <ResourceInspector resource={obj.resource} />;
  }

  // LoadResponse: { success, layerId, resourceCount, errors }
  if (
    typeof obj.success === "boolean" &&
    typeof obj.layerId === "string" &&
    typeof obj.resourceCount === "number"
  ) {
    if (!obj.success) {
      return <Body1>load failed</Body1>;
    }
    return (
      <div className={styles.status}>
        <Body1>
          Loaded {obj.resourceCount}{" "}
          resource{obj.resourceCount === 1 ? "" : "s"}
        </Body1>
        {obj.layerId && (
          <Caption1 className={styles.meta}>
            layer = {obj.layerId}
          </Caption1>
        )}
      </div>
    );
  }

  // Generic object / array — JSON tree.
  return <pre className={styles.jsonPre}>{safeJsonStringify(value)}</pre>;
}

function looksLikeTopologyNode(value: unknown): boolean {
  if (typeof value !== "object" || value === null) return false;
  const n = value as Record<string, unknown>;
  return typeof n.id === "string" &&
    typeof n.label === "string" &&
    typeof n.kind === "number";
}

function formatPrimitive(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "bigint") return `${value.toString()}n`;
  return String(value);
}

function safeJsonStringify(value: unknown): string {
  try {
    return JSON.stringify(value, jsonReplacer, 2);
  } catch (err) {
    return `<unserialisable: ${err instanceof Error ? err.message : "?"}>`;
  }
}

// Handle bigint and Uint8Array (which JSON.stringify can't represent natively).
function jsonReplacer(_key: string, value: unknown): unknown {
  if (typeof value === "bigint") return `${value.toString()}n`;
  if (value instanceof Uint8Array) {
    return `<Uint8Array length=${value.length}>`;
  }
  return value;
}

interface DomNodeMountProps {
  node: Node;
}

function DomNodeMount({ node }: DomNodeMountProps) {
  const ref = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.replaceChildren(node);
    return () => {
      // The same node can't appear in multiple places; if the cell
      // re-renders we let React tear down our wrapper which removes the
      // mounted node naturally.
    };
  }, [node]);
  return <div ref={ref} />;
}
