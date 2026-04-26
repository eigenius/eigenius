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
}

function copyMap<K, V>(map: ReadonlyMap<K, V>): Map<K, V> {
  return new Map(map);
}

function newCellId(): string {
  // crypto.randomUUID is available in all modern browsers + Deno.
  return crypto.randomUUID();
}

function defaultSourceFor(
  type: Exclude<CellType, "program-run">,
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
  meta: {},
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
      meta: json.meta ?? {},
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
        // program-run cells have no `source` field — silently ignore.
        if (c.id !== cellId || c.type === "program-run") return c;
        return { ...c, source };
      }),
    }));
  },

  insertCell(afterCellId, type) {
    const id = newCellId();
    const newCell: CellJson = type === "program-run"
      ? { id, type, program_iri: "", input_iris: [] }
      : { id, type, source: defaultSourceFor(type) };
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
