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
 * Tracks per-cell run state, per-cell output, and the active layer
 * the session has committed to. The store is *runtime-only*: cell
 * sources stay in the parsed `NotebookJson` (Phase 4 will lift them
 * into the store too, once cells are editable). Outputs are not
 * persisted across reloads; running a cell re-derives them.
 */

import { create } from "zustand";
import type { Eigen } from "@eigenius/client";
import type { CellJson } from "../persistence/notebook-format";

export type CellRunState = "idle" | "running" | "done" | "error";

/** Discriminated union over the renderable shapes Phase 3 produces. */
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
    kind: "error";
    message: string;
  };

export interface NotebookState {
  cellStates: ReadonlyMap<string, CellRunState>;
  cellOutputs: ReadonlyMap<string, CellOutput>;
  /**
   * Hex `LayerId` of the most-recently-committed layer in this session,
   * or `null` if no ESL cell has been loaded yet. The kernel returns
   * each new layer ID on Load; we track it here so subsequent Inspect /
   * Query calls can pin reads to the expected top.
   */
  activeLayer: string | null;

  runCell: (eigen: Eigen, cell: CellJson) => Promise<void>;
  runAll: (eigen: Eigen, cells: readonly CellJson[]) => Promise<void>;
  resetOutputs: () => void;
}

function copyMap<K, V>(map: ReadonlyMap<K, V>): Map<K, V> {
  return new Map(map);
}

export const useNotebookStore = create<NotebookState>((set, get) => ({
  cellStates: new Map(),
  cellOutputs: new Map(),
  activeLayer: null,

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
      const output = await executeCell(eigen, cell);
      setOutput(output);
      // Loads update the active layer for display purposes only — the
      // notebook does NOT pin downstream queries to this layer ID.
      // Reading at an explicit layer requires a persistent backend
      // (D21 §3.6); the docker stack runs without one. The orchestrator
      // holds a single gRPC connection to the kernel, so the kernel's
      // session active top advances naturally as cells commit, and
      // queries with empty `at_layer` see the latest state.
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

  async runAll(eigen, cells) {
    for (const cell of cells) {
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
}));

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
      // Empty atLayer = orchestrator's session active top (which the
      // gRPC connection keeps in sync as preceding cells commit). Do
      // not pass an explicit layer ID here — see runCell comment.
      const resp = await eigen.query(cell.source);
      if (!resp.success) {
        return {
          kind: "error",
          message: resp.error || "query failed (no error message)",
        };
      }
      return { kind: "resultset", document: resp.document };
    }
    case "typescript": {
      // Phase 4 wires up sandboxed TS execution (D22 §6.8). For Phase
      // 3 we surface a clear stub instead of a silent no-op.
      return {
        kind: "error",
        message: "TypeScript cell execution is a Phase 4 deliverable.",
      };
    }
    case "markdown":
      // Should never reach here — runAll skips markdown, and the
      // per-cell Run button is hidden on markdown cells.
      return { kind: "error", message: "markdown cells do not execute" };
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
