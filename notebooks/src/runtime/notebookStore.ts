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
import type { Eigen } from "@eigenius/client";
import type {
  CellJson,
  CellType,
  NotebookJson,
  NotebookMetaJson,
} from "../persistence/notebook-format";
import { CURRENT_FORMAT_VERSION } from "../persistence/notebook-format";

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
    kind: "error";
    message: string;
  };

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
}

function copyMap<K, V>(map: ReadonlyMap<K, V>): Map<K, V> {
  return new Map(map);
}

function newCellId(): string {
  // crypto.randomUUID is available in all modern browsers + Deno.
  return crypto.randomUUID();
}

function defaultSourceFor(type: CellType): string {
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
      cells: cells.map((c) => ({ id: c.id, type: c.type, source: c.source })),
    };
  },

  updateMeta(partial) {
    set((prev) => ({ meta: { ...prev.meta, ...partial } }));
  },

  updateCellSource(cellId, source) {
    set((prev) => ({
      cells: prev.cells.map((c) => c.id === cellId ? { ...c, source } : c),
    }));
  },

  insertCell(afterCellId, type) {
    const id = newCellId();
    const newCell: CellJson = {
      id,
      type,
      source: defaultSourceFor(type),
    };
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
    case "markdown":
      // Should never reach here — runAll skips markdown, and the
      // per-cell Run button is hidden on markdown cells.
      return { kind: "error", message: "markdown cells do not execute" };
  }
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
  const wrapped = `return (async () => {\n${source}\n})();`;
  let fn: (
    eigen: Eigen,
    previousOutputs: Record<string, CellOutput>,
    console: typeof capturedConsole,
  ) => Promise<unknown>;
  try {
    fn = new Function("eigen", "previousOutputs", "console", wrapped) as (
      eigen: Eigen,
      previousOutputs: Record<string, CellOutput>,
      console: typeof capturedConsole,
    ) => Promise<unknown>;
  } catch (err) {
    return {
      kind: "error",
      message: `TS cell parse error: ${
        err instanceof Error ? err.message : String(err)
      }`,
    };
  }

  try {
    const value = await fn(eigen, previousOutputs, capturedConsole);
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
