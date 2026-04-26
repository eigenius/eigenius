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
 * Notebook runtime state — Zustand store (D22 §6.4).
 *
 * As of Phase 4a the store owns the notebook's editable contents (cells
 * and meta) in addition to per-cell run state, outputs, and the active
 * layer. The on-disk JSON is the transport — it gets parsed once on load
 * and serialised on save; everything in between lives here.
 *
 * Outputs are not persisted across reloads; running a cell re-derives them.
 */

import { create } from "zustand";
import * as React from "react";
import {
  AreaChart,
  DonutChart,
  GroupedVerticalBarChart,
  HorizontalBarChart,
  LineChart,
  VerticalBarChart,
} from "@fluentui/react-charts";
import type { Eigen } from "@eigenius/client";
import type {
  CellJson,
  CellType,
  ChartCellJson,
  ChartKind,
  NotebookJson,
  NotebookMetaJson,
} from "../persistence/notebook-format";
import { CURRENT_FORMAT_VERSION } from "../persistence/notebook-format";
import { decodeResultDocument } from "./resultDocument";

/**
 * Curated chart namespace exposed to TS-cell sandboxes (Phase 5a).
 * `eigen.runProgramByIri(...)` returns data; the cell shapes it; the
 * cell returns `React.createElement(charts.GroupedVerticalBarChart, …)`
 * (or via the `h` shortcut) and the auto-renderer mounts it directly.
 *
 * Keep this list short — the chart catalogue is large, but exposing a
 * few common shapes is enough for the typical "chart this query" flow.
 */
const sandboxCharts = {
  AreaChart,
  DonutChart,
  GroupedVerticalBarChart,
  HorizontalBarChart,
  LineChart,
  VerticalBarChart,
} as const;

type SandboxCharts = typeof sandboxCharts;

/**
 * Helpers exposed to TS-cell sandboxes for shaping query results into
 * chart-friendly forms. The `rows` decoder turns a `QueryResponse.document`
 * (Eigon-CBOR ResultSet) into an array of plain objects keyed by the
 * `RETURN` short-names — eliminates the need to walk the CBOR document
 * by hand inside the cell.
 */
const sandboxHelpers = {
  /**
   * Decode an Eigon-CBOR ResultSet document into plain row objects
   * keyed by the column's short-name (the synthesized Property's
   * `core:short_name`). Convenient for piping query results straight
   * into chart props.
   */
  rows(document: Uint8Array): Record<string, unknown>[] {
    const decoded = decodeResultDocument(document);
    return decoded.rows.map((row) => {
      const out: Record<string, unknown> = {};
      for (const col of decoded.columns) {
        out[col.shortName] = row.values.get(col.iri);
      }
      return out;
    });
  },
} as const;

type SandboxHelpers = typeof sandboxHelpers;

export type CellRunState = "idle" | "running" | "done" | "error";

/** Discriminated union over the renderable shapes the runtime produces. */
export type CellOutput =
  | {
    kind: "load";
    layerId: string;
    resourceCount: number;
    /** Validation errors are non-fatal warnings here — load succeeded. */
    warnings: readonly string[];
  }
  | {
    kind: "validate";
    valid: boolean;
    programType: string;
    errors: readonly string[];
  }
  | {
    kind: "resultset";
    /** Eigon-CBOR document containing the ResultSet + row resources. */
    document: Uint8Array;
  }
  | {
    kind: "resource";
    /** CBOR-encoded Eigon resource (program output, single resource). */
    resource: Uint8Array;
    /** Optional trace IRI when the kernel has a trace store configured. */
    traceIri?: string;
  }
  | {
    /**
     * Phase 4b — TypeScript-cell return value. The auto-renderer
     * (`TypeScriptValueView`) dispatches on the runtime type:
     * Resource / ResultSet / RunResult / DOM node / object / primitive.
     * Console output captured during execution is included for surface.
     */
    kind: "value";
    value: unknown;
    log: readonly string[];
  }
  | {
    /**
     * Phase 4d — program-run cell output. One result per input IRI;
     * single result renders as ResourceInspector + TraceTreePanel,
     * multiple as a results table.
     */
    kind: "program-run";
    programIri: string;
    results: readonly ProgramRunResult[];
  }
  | {
    kind: "error";
    message: string;
  };

export interface ProgramRunResult {
  inputIri: string;
  success: boolean;
  output?: Uint8Array;
  traceIri?: string;
  errorMessage?: string;
}

export interface NotebookState {
  // ---- Document ----
  meta: NotebookMetaJson;
  cells: readonly CellJson[];

  // ---- Runtime ----
  cellStates: ReadonlyMap<string, CellRunState>;
  cellOutputs: ReadonlyMap<string, CellOutput>;
  /**
   * Hex `LayerId` of the most-recently-committed layer in this session,
   * or `null` if no ESL cell has been loaded yet. Display-only — queries
   * rely on the orchestrator's session active top, not on this value
   * (D21 §3.6).
   */
  activeLayer: string | null;

  // ---- Run actions ----
  runCell: (eigen: Eigen, cell: CellJson) => Promise<void>;
  runAll: (eigen: Eigen) => Promise<void>;
  resetOutputs: () => void;

  // ---- Document actions (Phase 4a) ----
  loadNotebook: (json: NotebookJson) => void;
  exportNotebook: () => NotebookJson;
  updateMeta: (partial: Partial<NotebookMetaJson>) => void;
  updateCellSource: (cellId: string, source: string) => void;
  insertCell: (afterCellId: string | null, type: CellType) => string;
  deleteCell: (cellId: string) => void;
  moveCell: (cellId: string, direction: "up" | "down") => void;
  /** Update fields on a program-run cell (no-op for other cell types). */
  updateProgramRunCell: (
    cellId: string,
    partial: { program_iri?: string; input_iris?: string[] },
  ) => void;
  /** Update fields on a chart cell (no-op for other cell types). */
  updateChartCell: (
    cellId: string,
    partial: Partial<Omit<ChartCellJson, "id" | "type">>,
  ) => void;
}

function copyMap<K, V>(map: ReadonlyMap<K, V>): Map<K, V> {
  return new Map(map);
}

function newCellId(): string {
  // crypto.randomUUID is available in all modern browsers + Deno.
  return crypto.randomUUID();
}

function defaultSourceFor(
  type: Exclude<CellType, "program-run" | "chart">,
): string {
  switch (type) {
    case "markdown":
      return "# New cell\n\nWrite Markdown here.";
    case "esl":
      return "// ESL declarations or program. Click Run to compile + commit.\n";
    case "eigenql":
      return "// EigenQL query. Run against the active layer chain.\n\n";
    case "typescript":
      return "// TypeScript orchestration cell (Phase 4b sandbox).\n";
  }
}

const EMPTY_NOTEBOOK: { meta: NotebookMetaJson; cells: readonly CellJson[] } = {
  meta: { title: "Untitled notebook" },
  cells: [],
};

export const useNotebookStore = create<NotebookState>((set, get) => ({
  meta: EMPTY_NOTEBOOK.meta,
  cells: EMPTY_NOTEBOOK.cells,

  cellStates: new Map(),
  cellOutputs: new Map(),
  activeLayer: null,

  // ---- Run actions ----

  async runCell(eigen, cell) {
    const setState = (state: CellRunState) => {
      set((prev) => ({
        cellStates: copyMap(prev.cellStates).set(cell.id, state),
      }));
    };
    const setOutput = (output: CellOutput) => {
      set((prev) => ({
        cellOutputs: copyMap(prev.cellOutputs).set(cell.id, output),
      }));
    };

    setState("running");
    try {
      // Snapshot the predecessor outputs at run time so TS cells can
      // refer to them via `previousOutputs[cellId]` (D22 §6.8).
      const { cells, cellOutputs } = get();
      const cellIndex = cells.findIndex((c) => c.id === cell.id);
      const previousOutputs: Record<string, CellOutput> = {};
      if (cellIndex > 0) {
        for (const prev of cells.slice(0, cellIndex)) {
          const out = cellOutputs.get(prev.id);
          if (out) previousOutputs[prev.id] = out;
        }
      }

      const output = await executeCell(eigen, cell, previousOutputs);
      setOutput(output);
      // Loads update the active layer for display purposes only — the
      // notebook does NOT pin downstream queries to this layer ID
      // (in-memory backend can't resolve explicit layer IDs).
      if (output.kind === "load" && output.layerId) {
        set({ activeLayer: output.layerId });
      }
      setState(output.kind === "error" ? "error" : "done");
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setOutput({ kind: "error", message });
      setState("error");
    }
  },

  async runAll(eigen) {
    for (const cell of get().cells) {
      // Markdown cells have nothing to run — render-only.
      if (cell.type === "markdown") continue;
      await get().runCell(eigen, cell);
      const finalState = get().cellStates.get(cell.id);
      if (finalState === "error") {
        // §6.3: halt on the first failing cell. The user sees the error
        // inline on the failed cell and can fix + resume manually.
        break;
      }
    }
  },

  resetOutputs() {
    set({
      cellStates: new Map(),
      cellOutputs: new Map(),
      activeLayer: null,
    });
  },

  // ---- Document actions ----

  loadNotebook(json) {
    set({
      meta: json.meta,
      cells: json.cells,
      // Reset all runtime state when a new notebook loads.
      cellStates: new Map(),
      cellOutputs: new Map(),
      activeLayer: null,
    });
  },

  exportNotebook() {
    const { meta, cells } = get();
    return {
      format_version: CURRENT_FORMAT_VERSION,
      meta,
      cells: cells.map(serializeCell),
    };
  },

  updateMeta(partial) {
    set((prev) => ({ meta: { ...prev.meta, ...partial } }));
  },

  updateCellSource(cellId, source) {
    set((prev) => ({
      cells: prev.cells.map((c) => {
        // program-run / chart cells have no `source` field — silently ignore.
        if (c.id !== cellId || c.type === "program-run" || c.type === "chart") {
          return c;
        }
        return { ...c, source };
      }),
    }));
  },

  insertCell(afterCellId, type) {
    const id = newCellId();
    let newCell: CellJson;
    if (type === "program-run") {
      newCell = { id, type, program_iri: "", input_iris: [] };
    } else if (type === "chart") {
      newCell = {
        id,
        type,
        query: "// EigenQL query — RETURN the columns you want to chart.\n\n",
        chart_kind: "vertical-bar",
        x_column: "",
        y_column: "",
      };
    } else {
      newCell = { id, type, source: defaultSourceFor(type) };
    }
    set((prev) => {
      if (afterCellId === null) {
        return { cells: [newCell, ...prev.cells] };
      }
      const idx = prev.cells.findIndex((c) => c.id === afterCellId);
      if (idx < 0) {
        // Unknown anchor — append at the end rather than silently failing.
        return { cells: [...prev.cells, newCell] };
      }
      const next = prev.cells.slice();
      next.splice(idx + 1, 0, newCell);
      return { cells: next };
    });
    return id;
  },

  updateProgramRunCell(cellId, partial) {
    set((prev) => ({
      cells: prev.cells.map((c) => {
        if (c.id !== cellId || c.type !== "program-run") return c;
        return { ...c, ...partial };
      }),
    }));
  },

  updateChartCell(cellId, partial) {
    set((prev) => ({
      cells: prev.cells.map((c) => {
        if (c.id !== cellId || c.type !== "chart") return c;
        return { ...c, ...partial };
      }),
    }));
  },

  deleteCell(cellId) {
    set((prev) => ({
      cells: prev.cells.filter((c) => c.id !== cellId),
      cellStates: dropKey(prev.cellStates, cellId),
      cellOutputs: dropKey(prev.cellOutputs, cellId),
    }));
  },

  moveCell(cellId, direction) {
    set((prev) => {
      const idx = prev.cells.findIndex((c) => c.id === cellId);
      if (idx < 0) return {};
      const target = direction === "up" ? idx - 1 : idx + 1;
      if (target < 0 || target >= prev.cells.length) return {};
      const next = prev.cells.slice();
      [next[idx], next[target]] = [next[target], next[idx]];
      return { cells: next };
    });
  },
}));

function dropKey<K, V>(map: ReadonlyMap<K, V>, key: K): Map<K, V> {
  const next = copyMap(map);
  next.delete(key);
  return next;
}

function serializeCell(c: CellJson): CellJson {
  if (c.type === "program-run") {
    return {
      id: c.id,
      type: c.type,
      program_iri: c.program_iri,
      input_iris: [...c.input_iris],
    };
  }
  if (c.type === "chart") {
    return {
      id: c.id,
      type: c.type,
      query: c.query,
      chart_kind: c.chart_kind,
      x_column: c.x_column,
      y_column: c.y_column,
      ...(c.series_column !== undefined ? { series_column: c.series_column } : {}),
      ...(c.title !== undefined ? { title: c.title } : {}),
    };
  }
  return { id: c.id, type: c.type, source: c.source };
}

/**
 * Dispatch a cell's source against the active session, returning a
 * structured CellOutput (success or error). Throws only on truly
 * unexpected failures (network blow-up, malformed responses); RPC-level
 * "this didn't work" results are returned as `kind: "error"` so the
 * UI renders a structured error panel rather than a stack trace.
 */
async function executeCell(
  eigen: Eigen,
  cell: CellJson,
  previousOutputs: Record<string, CellOutput>,
): Promise<CellOutput> {
  switch (cell.type) {
    case "esl": {
      const resp = await eigen.load(cell.source, {
        contentType: "application/x-esl",
        autoCommit: true,
      });
      if (!resp.success) {
        const messages = resp.errors.map((e) => formatValidationError(e));
        return {
          kind: "error",
          message: messages.length === 0
            ? "load failed (no errors reported)"
            : messages.join("\n"),
        };
      }
      return {
        kind: "load",
        layerId: resp.layerId,
        resourceCount: resp.resourceCount,
        warnings: [],
      };
    }
    case "eigenql": {
      const resp = await eigen.query(cell.source);
      if (!resp.success) {
        return {
          kind: "error",
          message: resp.error || "query failed (no error message)",
        };
      }
      return { kind: "resultset", document: resp.document };
    }
    case "typescript":
      return executeTypeScriptCell(eigen, cell.source, previousOutputs);
    case "program-run":
      return executeProgramRunCell(eigen, cell.program_iri, cell.input_iris);
    case "chart":
      return executeChartCell(eigen, cell);
    case "markdown":
      // Should never reach here — runAll skips markdown, and the
      // per-cell Run button is hidden on markdown cells.
      return { kind: "error", message: "markdown cells do not execute" };
  }
}

/**
 * Phase 4d — program-run dispatch. Calls `runProgramByIri` once per
 * input IRI. Per-input failures are captured into the result row's
 * `errorMessage` rather than failing the whole cell, so a batch with
 * one bad input still renders the others.
 */
async function executeProgramRunCell(
  eigen: Eigen,
  programIri: string,
  inputIris: readonly string[],
): Promise<CellOutput> {
  const trimmedProgram = programIri.trim();
  if (trimmedProgram.length === 0) {
    return { kind: "error", message: "program IRI is empty" };
  }
  const validInputs = inputIris.map((s) => s.trim()).filter((s) => s.length > 0);
  if (validInputs.length === 0) {
    return { kind: "error", message: "no input IRIs provided" };
  }

  const results: ProgramRunResult[] = [];
  for (const inputIri of validInputs) {
    try {
      const resp = await eigen.runProgramByIri(trimmedProgram, inputIri);
      if (!resp.success) {
        results.push({
          inputIri,
          success: false,
          errorMessage: resp.errors.map((e) => e.message).join("; ") ||
            "program failed (no error message)",
        });
      } else {
        results.push({
          inputIri,
          success: true,
          output: resp.output,
          traceIri: resp.traceIri || undefined,
        });
      }
    } catch (err) {
      results.push({
        inputIri,
        success: false,
        errorMessage: err instanceof Error ? err.message : String(err),
      });
    }
  }
  return { kind: "program-run", programIri: trimmedProgram, results };
}

/**
 * Phase 5d — chart cell dispatch. Runs the cell's EigenQL query,
 * decodes the ResultSet, pivots into the shape Fluent's chart
 * components expect, and returns a React element via the `kind: "value"`
 * output path. The auto-renderer mounts it directly.
 */
async function executeChartCell(
  eigen: Eigen,
  cell: ChartCellJson,
): Promise<CellOutput> {
  const trimmedQuery = cell.query.trim();
  if (trimmedQuery.length === 0) {
    return { kind: "error", message: "chart query is empty" };
  }
  if (cell.x_column.trim().length === 0 || cell.y_column.trim().length === 0) {
    return {
      kind: "error",
      message: "chart x_column and y_column are required",
    };
  }
  const resp = await eigen.query(trimmedQuery);
  if (!resp.success) {
    return {
      kind: "error",
      message: resp.error || "chart query failed (no error message)",
    };
  }
  const rows = sandboxHelpers.rows(resp.document);
  if (rows.length === 0) {
    return { kind: "error", message: "query returned no rows to chart" };
  }
  const firstRow = rows[0] as Record<string, unknown>;
  if (!(cell.x_column in firstRow)) {
    return {
      kind: "error",
      message: `x_column "${cell.x_column}" not found in query results (available: ${
        Object.keys(firstRow).join(", ")
      })`,
    };
  }
  if (!(cell.y_column in firstRow)) {
    return {
      kind: "error",
      message: `y_column "${cell.y_column}" not found in query results (available: ${
        Object.keys(firstRow).join(", ")
      })`,
    };
  }
  if (
    cell.series_column !== undefined &&
    cell.series_column.length > 0 &&
    !(cell.series_column in firstRow)
  ) {
    return {
      kind: "error",
      message:
        `series_column "${cell.series_column}" not found in query results`,
    };
  }
  const element = renderChart(cell.chart_kind, rows, {
    x: cell.x_column,
    y: cell.y_column,
    series: cell.series_column,
    title: cell.title,
  });
  return { kind: "value", value: element, log: [] };
}

interface ChartShape {
  x: string;
  y: string;
  series?: string;
  title?: string;
}

function renderChart(
  kind: ChartKind,
  rows: Record<string, unknown>[],
  shape: ChartShape,
): React.ReactElement {
  const title = shape.title;
  const palette = [
    "#5b88c5", "#37a172", "#cf6f1e", "#a45fa1",
    "#c93434", "#3aa3a8", "#b3a02f", "#7e57c2",
  ];
  const colorOf = (seriesKey: string, idx: number): string =>
    palette[Math.abs(hashString(seriesKey)) % palette.length] ?? palette[idx % palette.length];

  switch (kind) {
    case "donut": {
      const chartData = rows.map((r, i) => ({
        legend: String(r[shape.x] ?? `slice ${i}`),
        data: Number(r[shape.y]) || 0,
        color: colorOf(String(r[shape.x] ?? i), i),
      }));
      const total = chartData.reduce((s, d) => s + d.data, 0);
      return React.createElement(DonutChart, {
        data: { chartTitle: title, chartData },
        innerRadius: 55,
        height: 240,
        width: 240,
        valueInsideDonut: total,
      });
    }
    case "grouped-bar": {
      // Group by x; series_column (if present) splits each group.
      const grouped = new Map<string, { key: string; legend: string; data: number; color: string }[]>();
      for (const r of rows) {
        const groupName = String(r[shape.x] ?? "");
        const seriesKey = shape.series && shape.series.length > 0
          ? String(r[shape.series] ?? "")
          : shape.y;
        if (!grouped.has(groupName)) grouped.set(groupName, []);
        grouped.get(groupName)!.push({
          key: seriesKey,
          legend: seriesKey,
          data: Number(r[shape.y]) || 0,
          color: colorOf(seriesKey, grouped.get(groupName)!.length),
        });
      }
      const data = Array.from(grouped.entries()).map(([name, series]) => ({
        name,
        series,
      }));
      return React.createElement(GroupedVerticalBarChart, {
        data,
        chartTitle: title,
        height: 320,
        width: 800,
      });
    }
    case "vertical-bar": {
      const chartData = rows.map((r, i) => ({
        x: String(r[shape.x] ?? `point ${i}`),
        y: Number(r[shape.y]) || 0,
        legend: String(r[shape.x] ?? ""),
        color: colorOf(String(r[shape.x] ?? i), i),
      }));
      return React.createElement(VerticalBarChart, {
        data: chartData,
        chartTitle: title,
        height: 320,
        width: 800,
      });
    }
    case "horizontal-bar": {
      const yMax = Math.max(
        ...rows.map((r) => Number(r[shape.y]) || 0),
        1,
      );
      const data = rows.map((r, i) => ({
        chartData: [{
          legend: String(r[shape.x] ?? `point ${i}`),
          horizontalBarChartdata: {
            x: Number(r[shape.y]) || 0,
            y: yMax,
          },
          color: colorOf(String(r[shape.x] ?? i), i),
        }],
      }));
      return React.createElement(HorizontalBarChart, { data });
    }
    case "line":
    case "area": {
      // Fluent's LineChart / AreaChart only support numeric or Date
      // x-axes — string values would yield a broken d3 scale (axis
      // ticks render, points don't). For categorical x, build an
      // index map (label → 0,1,2,...) preserving encounter order so
      // the EigenQL `ORDER BY` controls the layout.
      const xCategoryIndex = new Map<string, number>();
      const isCategoricalX = !rows.every((r) => {
        const v = r[shape.x];
        return typeof v === "number" || isDateLike(v);
      });

      const seriesMap = new Map<
        string,
        { x: number | Date; y: number; xAxisCalloutData?: string }[]
      >();
      for (const r of rows) {
        const seriesKey = shape.series && shape.series.length > 0
          ? String(r[shape.series] ?? "")
          : shape.y;
        if (!seriesMap.has(seriesKey)) seriesMap.set(seriesKey, []);
        const xv = r[shape.x];
        let x: number | Date;
        let xAxisCalloutData: string | undefined;
        if (isCategoricalX) {
          const label = String(xv ?? "");
          if (!xCategoryIndex.has(label)) {
            xCategoryIndex.set(label, xCategoryIndex.size);
          }
          x = xCategoryIndex.get(label)!;
          // Show the original label in the hover callout instead of
          // the numeric index we plot against.
          xAxisCalloutData = label;
        } else if (isDateLike(xv)) {
          x = new Date(String(xv));
        } else {
          x = Number(xv);
        }
        seriesMap.get(seriesKey)!.push({
          x,
          y: Number(r[shape.y]) || 0,
          ...(xAxisCalloutData !== undefined ? { xAxisCalloutData } : {}),
        });
      }
      const lineChartData = Array.from(seriesMap.entries()).map(
        ([legend, data], i) => ({
          legend,
          data,
          color: colorOf(legend, i),
        }),
      );
      const Component = kind === "area" ? AreaChart : LineChart;
      const extraProps: Record<string, unknown> = {};
      if (isCategoricalX) {
        // Fluent's LineChart / AreaChart auto-pick a numeric x-axis
        // when x is a number. We pin `tickValues` to the integer
        // indices we assigned and use `xAxis.tickText` to label each
        // index with the original category — matching Fluent's
        // documented "tick labels at tickValues positions" pattern.
        const labels = Array.from(xCategoryIndex.keys());
        extraProps.tickValues = labels.map((_, i) => i);
        extraProps.xAxis = { tickText: labels };
      }
      if (kind === "line") {
        // Per-line callout instead of the default stacked one — keeps
        // the popover compact and shows only the hovered series.
        // (AreaChart hard-codes the stacked callout internally; the
        // prop is a no-op there.)
        extraProps.isCalloutForStack = false;
      }
      return React.createElement(Component, {
        data: { chartTitle: title, lineChartData },
        height: 320,
        width: 800,
        ...extraProps,
      });
    }
  }
}

function hashString(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) {
    h = (h * 31 + s.charCodeAt(i)) | 0;
  }
  return h;
}

const ISO_DATE_RE = /^\d{4}-\d{2}-\d{2}(T[\d:.]+(Z|[+\-]\d{2}:\d{2})?)?$/;

function isDateLike(v: unknown): v is string {
  return typeof v === "string" && ISO_DATE_RE.test(v);
}

/**
 * TypeScript cell sandbox (D22 §6.8). Compiles `source` as the body of
 * an async IIFE with `eigen` and `previousOutputs` in scope, captures
 * `console.log/info/warn/error`, and returns the IIFE's resolved value
 * as a `kind: "value"` CellOutput for the auto-renderer.
 *
 * Trusted execution — the cell runs with full page-context access.
 * Multi-user notebooks (post-MVP) will need an iframe or Web Worker
 * sandbox; for single-user authoring, this is acceptable.
 */
async function executeTypeScriptCell(
  eigen: Eigen,
  source: string,
  previousOutputs: Record<string, CellOutput>,
): Promise<CellOutput> {
  const log: string[] = [];
  const capturedConsole = {
    log: (...args: unknown[]) => log.push(args.map(formatConsoleArg).join(" ")),
    info: (...args: unknown[]) => log.push(args.map(formatConsoleArg).join(" ")),
    warn: (...args: unknown[]) =>
      log.push(`[warn] ${args.map(formatConsoleArg).join(" ")}`),
    error: (...args: unknown[]) =>
      log.push(`[error] ${args.map(formatConsoleArg).join(" ")}`),
  };

  // The IIFE wrapping lets users either `return value` explicitly or
  // simply have the last statement be an expression — though only an
  // explicit return surfaces it (we can't run the source through a
  // compiler here without dragging one in). Document accordingly.
  //
  // Phase 5a: `React`, `h` (alias for React.createElement),
  // `charts` (a curated @fluentui/react-charts namespace), and
  // `nb` (notebook helpers — `nb.rows(document)` decodes a ResultSet
  // CBOR into plain objects) are also in scope. A cell can return a
  // React element (e.g. `h(charts.GroupedVerticalBarChart, { data })`)
  // and the auto-renderer mounts it directly.
  const wrapped = `return (async () => {\n${source}\n})();`;
  type SandboxFn = (
    eigen: Eigen,
    previousOutputs: Record<string, CellOutput>,
    console: typeof capturedConsole,
    React: typeof import("react"),
    h: typeof React.createElement,
    charts: SandboxCharts,
    nb: SandboxHelpers,
  ) => Promise<unknown>;
  let fn: SandboxFn;
  try {
    fn = new Function(
      "eigen",
      "previousOutputs",
      "console",
      "React",
      "h",
      "charts",
      "nb",
      wrapped,
    ) as SandboxFn;
  } catch (err) {
    return {
      kind: "error",
      message: `TS cell parse error: ${
        err instanceof Error ? err.message : String(err)
      }`,
    };
  }

  try {
    const value = await fn(
      eigen,
      previousOutputs,
      capturedConsole,
      React,
      React.createElement,
      sandboxCharts,
      sandboxHelpers,
    );
    return { kind: "value", value, log };
  } catch (err) {
    return {
      kind: "error",
      message: `${err instanceof Error ? err.message : String(err)}${
        log.length > 0 ? "\n\nconsole:\n" + log.join("\n") : ""
      }`,
    };
  }
}

function formatConsoleArg(arg: unknown): string {
  if (typeof arg === "string") return arg;
  if (arg instanceof Error) return arg.message;
  try {
    return JSON.stringify(arg);
  } catch {
    return String(arg);
  }
}

interface ValidationErrorLike {
  message: string;
  rule?: string;
  line?: number;
  column?: number;
}

function formatValidationError(err: ValidationErrorLike): string {
  const prefix = err.rule ? `[${err.rule}] ` : "";
  const position = err.line ? ` (${err.line}:${err.column ?? 0})` : "";
  return `${prefix}${err.message}${position}`;
}
