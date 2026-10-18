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
 * Layer-inspector destination.
 *
 * Renders every resource defined in the layer pointed at by the
 * session read-pin (set by History's "Inspect resources…"). Each
 * resource appears as a collapsible card with pretty-printed Eigon
 * JSON. The list is filtered to resources whose `layer_id` attribute
 * (set by `kernel/src/server/topology.rs` on first sighting) matches
 * the pinned layer — i.e., what this layer *introduced*, not its
 * ancestors' inherited definitions.
 *
 * Data flow:
 *   1. `eigen.layerTopology({ rootLayer: pin, maxDepth: 1,
 *       includeResources: true })` enumerates the layer's resources.
 *   2. For each non-Layer node whose `layer_id` matches the pin,
 *      `eigen.inspect(iri, { atLayer: pin })` fetches the CBOR body.
 *   3. cbor-x decodes; `JSON.stringify(_, null, 2)` pretty-prints.
 *
 * `maxDepth: 1` keeps the walker bounded to the pinned layer plus
 * its immediate parent (so the layer's own children are still
 * captured); the client-side filter then keeps only this layer's
 * contributions.
 */

import { useEffect, useMemo, useState } from "react";
import {
  Accordion,
  AccordionHeader,
  AccordionItem,
  AccordionPanel,
  Body1,
  Button,
  Caption1,
  makeStyles,
  MessageBar,
  MessageBarBody,
  Spinner,
  Subtitle1,
  Tag,
  tokens,
} from "@fluentui/react-components";
import { CubeTree20Regular, Pin16Regular } from "@fluentui/react-icons";
import { decode as cborDecode } from "cbor-x";
import { NodeKind, type TopologyNode } from "@eigenius/client";
import { useEigen } from "../../runtime/EigenProvider";
import { useNotebookStore } from "../../runtime/notebookStore";

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
  headerSpacer: { flex: 1 },
  body: {
    flex: 1,
    minHeight: 0,
    overflowY: "auto",
    padding: tokens.spacingVerticalM,
  },
  bodyInner: {
    maxWidth: "960px",
    margin: "0 auto",
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalM,
  },
  pinNotice: {
    display: "inline-flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalXS,
    padding: `${tokens.spacingVerticalXS} ${tokens.spacingHorizontalS}`,
    background: tokens.colorNeutralBackground2,
    borderRadius: tokens.borderRadiusMedium,
    fontSize: tokens.fontSizeBase200,
  },
  pinHash: {
    fontFamily: tokens.fontFamilyMonospace,
  },
  countsRow: {
    display: "flex",
    gap: tokens.spacingHorizontalS,
    flexWrap: "wrap",
  },
  countTag: {
    fontFamily: tokens.fontFamilyMonospace,
  },
  itemHeaderInner: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalS,
    width: "100%",
    overflow: "hidden",
  },
  itemIri: {
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase300,
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
    flex: 1,
  },
  itemKind: {
    color: tokens.colorNeutralForeground3,
    fontSize: tokens.fontSizeBase200,
  },
  jsonPanel: {
    margin: 0,
    fontFamily: tokens.fontFamilyMonospace,
    fontSize: tokens.fontSizeBase200,
    whiteSpace: "pre",
    background: tokens.colorNeutralBackground2,
    padding: tokens.spacingVerticalS,
    borderRadius: tokens.borderRadiusSmall,
    overflowX: "auto",
  },
  emptyState: {
    padding: tokens.spacingVerticalXXL,
    textAlign: "center",
    color: tokens.colorNeutralForeground3,
  },
  loadingState: {
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    gap: tokens.spacingHorizontalS,
    padding: tokens.spacingVerticalXXL,
    color: tokens.colorNeutralForeground3,
  },
});

interface LayerResource {
  iri: string;
  kind: NodeKind;
  label: string;
  /** `ok` = decoded resource ready to render. `not-found` = the
   * inspect RPC said the IRI didn't resolve at this layer. `decode-
   * failed` = inspect returned bytes but cbor-x couldn't shape them
   * into a record. */
  body:
    | { status: "ok"; resource: Record<string, unknown> }
    | { status: "not-found" }
    | { status: "decode-failed"; error?: string };
}

export function LayerPanel() {
  const styles = useStyles();
  const eigen = useEigen();
  const readPinLayerId = useNotebookStore((s) => s.readPinLayerId);
  const setReadPin = useNotebookStore((s) => s.setReadPin);
  const setDestination = useNotebookStore((s) => s.setDestination);

  const [resources, setResources] = useState<LayerResource[] | null>(null);
  const [layerLabel, setLayerLabel] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setResources(null);
    setLayerLabel(null);
    setError(null);
    if (!readPinLayerId) return;
    setLoading(true);
    (async () => {
      try {
        const topo = await eigen.layerTopology({
          rootLayer: readPinLayerId,
          maxDepth: 1,
          includeResources: true,
        });
        if (cancelled) return;
        const layerNode = topo.nodes.find(
          (n) => n.kind === NodeKind.LAYER && n.id === readPinLayerId,
        );
        setLayerLabel(layerNode?.label ?? null);

        const own = topo.nodes.filter(
          (n) =>
            n.kind !== NodeKind.LAYER &&
            n.attrs["layer_id"] === readPinLayerId,
        );
        own.sort(compareNodes);

        // Fan out N inspect calls in parallel. Typical layers carry
        // 1–30 resources; the cost is one round-trip per resource.
        const bodies = await Promise.all(
          own.map(async (n): Promise<LayerResource["body"]> => {
            try {
              const resp = await eigen.inspect(n.id, {
                atLayer: readPinLayerId,
              });
              if (!resp.found) {
                console.warn("[layer-inspector] not found", {
                  iri: n.id,
                  atLayer: readPinLayerId,
                });
                return { status: "not-found" };
              }
              const decoded = decodeResource(resp.resource);
              if (!decoded.ok) {
                console.warn("[layer-inspector] decode failed", {
                  iri: n.id,
                  bytes: resp.resource.length,
                  reason: decoded.reason,
                  raw: decoded.raw,
                });
                return {
                  status: "decode-failed",
                  error: decoded.reason,
                };
              }
              return { status: "ok", resource: decoded.value };
            } catch (e) {
              console.warn("[layer-inspector] inspect threw", {
                iri: n.id,
                error: e,
              });
              return {
                status: "decode-failed",
                error: e instanceof Error ? e.message : String(e),
              };
            }
          }),
        );
        if (cancelled) return;
        setResources(
          own.map((n, i) => ({
            iri: n.id,
            kind: n.kind,
            label: n.label,
            body: bodies[i],
          })),
        );
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : String(err));
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [eigen, readPinLayerId]);

  const counts = useMemo(() => bucketCounts(resources), [resources]);

  return (
    <div className={styles.root}>
      <div className={styles.header}>
        <CubeTree20Regular />
        <Subtitle1 as="h2">Layer inspector</Subtitle1>
        <span className={styles.headerSpacer} />
        {readPinLayerId && (
          <>
            <span className={styles.pinNotice}>
              <Pin16Regular />
              <span className={styles.pinHash}>
                {layerLabel ?? "layer"} · {shortHash(readPinLayerId)}
              </span>
            </span>
            <Button
              size="small"
              appearance="subtle"
              onClick={() => {
                setReadPin(null);
                setDestination("history");
              }}
            >
              Back to history
            </Button>
          </>
        )}
      </div>
      <div className={styles.body}>
        <div className={styles.bodyInner}>
          {!readPinLayerId && (
            <div className={styles.emptyState}>
              <Body1>
                No layer pinned. Open the History panel and click "Inspect
                resources…" on a commit.
              </Body1>
            </div>
          )}
          {readPinLayerId && error && (
            <MessageBar intent="error">
              <MessageBarBody>{error}</MessageBarBody>
            </MessageBar>
          )}
          {readPinLayerId && loading && (
            <div className={styles.loadingState}>
              <Spinner size="tiny" />
              <Caption1>loading resources…</Caption1>
            </div>
          )}
          {readPinLayerId && !loading && resources && resources.length === 0 && (
            <div className={styles.emptyState}>
              <Body1>This layer did not introduce any resources.</Body1>
            </div>
          )}
          {readPinLayerId && resources && resources.length > 0 && (
            <>
              <div className={styles.countsRow}>
                {counts.map((c) => (
                  <Tag
                    key={c.label}
                    size="small"
                    appearance="brand"
                    className={styles.countTag}
                  >
                    {c.label} · {c.count}
                  </Tag>
                ))}
              </div>
              <Accordion multiple collapsible>
                {resources.map((r) => (
                  <AccordionItem key={r.iri} value={r.iri}>
                    <AccordionHeader>
                      <div className={styles.itemHeaderInner}>
                        <Tag size="small" appearance="outline">
                          {kindLabel(r.kind)}
                        </Tag>
                        <span className={styles.itemIri} title={r.iri}>
                          {r.label}
                        </span>
                        <Caption1 className={styles.itemKind}>
                          {r.iri}
                        </Caption1>
                      </div>
                    </AccordionHeader>
                    <AccordionPanel>
                      {r.body.status === "ok" && (
                        <pre className={styles.jsonPanel}>
                          {prettyJson(r.body.resource)}
                        </pre>
                      )}
                      {r.body.status === "not-found" && (
                        <Caption1>
                          The inspect RPC reported no resource at this IRI at
                          this layer — the topology walker saw it, so this is
                          a kernel-side discrepancy worth flagging.
                        </Caption1>
                      )}
                      {r.body.status === "decode-failed" && (
                        <Caption1>
                          Inspect returned bytes, but cbor-x couldn't shape
                          them into a record
                          {r.body.error ? `: ${r.body.error}` : "."}
                          {" "}See the browser console for the raw payload.
                        </Caption1>
                      )}
                    </AccordionPanel>
                  </AccordionItem>
                ))}
              </Accordion>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

type DecodeResult =
  | { ok: true; value: Record<string, unknown> }
  | { ok: false; reason: string; raw?: unknown };

function decodeResource(bytes: Uint8Array): DecodeResult {
  if (bytes.length === 0) {
    return { ok: false, reason: "empty body" };
  }
  let decoded: unknown;
  try {
    decoded = cborDecode(bytes);
  } catch (e) {
    return {
      ok: false,
      reason: `cbor-x threw: ${e instanceof Error ? e.message : String(e)}`,
    };
  }
  // cbor-x decodes CBOR maps as `Map` instances by default for keys
  // that aren't strings; coerce to a plain object so JSON.stringify
  // sees the entries.
  if (decoded instanceof Map) {
    const obj: Record<string, unknown> = {};
    for (const [k, v] of decoded.entries()) {
      obj[String(k)] = v;
    }
    return { ok: true, value: obj };
  }
  if (Array.isArray(decoded)) {
    const first = decoded.find(
      (v): v is Record<string, unknown> =>
        typeof v === "object" && v !== null && !(v instanceof Map) &&
        !Array.isArray(v),
    );
    if (first) return { ok: true, value: first };
    // Maybe the array's elements are Maps.
    const firstMap = decoded.find((v): v is Map<unknown, unknown> =>
      v instanceof Map
    );
    if (firstMap) {
      const obj: Record<string, unknown> = {};
      for (const [k, v] of firstMap.entries()) obj[String(k)] = v;
      return { ok: true, value: obj };
    }
    return {
      ok: false,
      reason: `decoded an array of length ${decoded.length} but no entry was a resource record`,
      raw: decoded,
    };
  }
  if (typeof decoded === "object" && decoded !== null) {
    return { ok: true, value: decoded as Record<string, unknown> };
  }
  return {
    ok: false,
    reason: `decoded value has type ${typeof decoded} (expected object/map/array)`,
    raw: decoded,
  };
}

function prettyJson(body: Record<string, unknown>): string {
  // Stable key ordering: `@id` first, `urn:eigenius:core:is_a` second,
  // remaining keys sorted lexicographically. Matches Eigon-JSON's
  // canonical order so the rendered shape lines up with what the
  // chain stores.
  const ordered: Record<string, unknown> = {};
  if ("@id" in body) ordered["@id"] = body["@id"];
  if ("urn:eigenius:core:is_a" in body) {
    ordered["urn:eigenius:core:is_a"] = body["urn:eigenius:core:is_a"];
  }
  for (const key of Object.keys(body).sort()) {
    if (key === "@id" || key === "urn:eigenius:core:is_a") continue;
    ordered[key] = body[key];
  }
  return JSON.stringify(ordered, jsonReplacer, 2);
}

function jsonReplacer(_key: string, value: unknown): unknown {
  if (typeof value === "bigint") return value.toString();
  if (value instanceof Uint8Array) return `<${value.length} bytes>`;
  return value;
}

function bucketCounts(
  resources: LayerResource[] | null,
): Array<{ label: string; count: number }> {
  if (!resources) return [];
  const buckets = new Map<string, number>();
  for (const r of resources) {
    const key = kindLabel(r.kind);
    buckets.set(key, (buckets.get(key) ?? 0) + 1);
  }
  return Array.from(buckets, ([label, count]) => ({ label, count })).sort((
    a,
    b,
  ) => a.label.localeCompare(b.label));
}

function kindLabel(kind: NodeKind): string {
  switch (kind) {
    case NodeKind.CLASS:
      return "Class";
    case NodeKind.PROPERTY:
      return "Property";
    case NodeKind.INSTITUTION:
      return "Institution";
    case NodeKind.RESOURCE:
      return "Resource";
    default:
      return "Other";
  }
}

function compareNodes(a: TopologyNode, b: TopologyNode): number {
  // Stable order: by kind first (Class, Property, Institution,
  // Resource), then by IRI. The kind-first ordering keeps related
  // entries together when expanding the accordion.
  const kindOrder = (k: NodeKind): number => {
    switch (k) {
      case NodeKind.CLASS:
        return 0;
      case NodeKind.PROPERTY:
        return 1;
      case NodeKind.INSTITUTION:
        return 2;
      case NodeKind.RESOURCE:
        return 3;
      default:
        return 4;
    }
  };
  const dk = kindOrder(a.kind) - kindOrder(b.kind);
  if (dk !== 0) return dk;
  return a.id.localeCompare(b.id);
}

function shortHash(hex: string): string {
  return hex.length > 12 ? `${hex.slice(0, 12)}…` : hex;
}
