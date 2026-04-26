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
 * Notebook file format (D22 §6.5).
 *
 * Custom JSON, version-tagged so the loader can refuse files newer
 * than it understands or migrate older ones forward. Cell outputs are
 * intentionally NOT persisted in the MVP — they're re-derived from
 * cell sources at run time.
 */

export const CURRENT_FORMAT_VERSION = 1;

export type CellType = "markdown" | "esl" | "eigenql" | "typescript";

export interface CellJson {
  /** Stable UUID assigned at cell creation. */
  id: string;
  type: CellType;
  /** The cell's editable text. */
  source: string;
}

export interface NotebookMetaJson {
  title?: string;
  /**
   * Short description of what the notebook does. Surfaced as a queryable
   * `urn:eigenius:notebook:description` property when the notebook is
   * published to a layer (Phase 3.5).
   */
  description?: string;
  /** ISO 8601 — set by the editor on save. */
  created?: string;
  /** ISO 8601 — set by the editor on every save. */
  modified?: string;
  /** Eigenius platform version when last saved. */
  eigenius_version?: string;
}

export interface NotebookJson {
  format_version: number;
  meta: NotebookMetaJson;
  cells: CellJson[];
}

/**
 * Coerce an unknown value into a NotebookJson, throwing on shape errors.
 *
 * Phase 2 only validates structural shape; richer per-cell content
 * validation arrives in Phase 4 once cells become editable and round-trip
 * via save/load.
 */
export function parseNotebook(value: unknown): NotebookJson {
  if (typeof value !== "object" || value === null) {
    throw new Error("notebook: top-level value must be an object");
  }
  const obj = value as Record<string, unknown>;
  const formatVersion = obj.format_version;
  if (typeof formatVersion !== "number") {
    throw new Error("notebook: missing or non-numeric `format_version`");
  }
  if (formatVersion > CURRENT_FORMAT_VERSION) {
    throw new Error(
      `notebook: format_version ${formatVersion} is newer than this client supports (${CURRENT_FORMAT_VERSION})`,
    );
  }
  const meta = (obj.meta ?? {}) as NotebookMetaJson;
  const cellsRaw = obj.cells;
  if (!Array.isArray(cellsRaw)) {
    throw new Error("notebook: `cells` must be an array");
  }
  const cells: CellJson[] = cellsRaw.map((c, i) => parseCell(c, i));
  return { format_version: formatVersion, meta, cells };
}

function parseCell(value: unknown, index: number): CellJson {
  if (typeof value !== "object" || value === null) {
    throw new Error(`notebook: cells[${index}] must be an object`);
  }
  const obj = value as Record<string, unknown>;
  const id = obj.id;
  const type = obj.type;
  const source = obj.source;
  if (typeof id !== "string" || id.length === 0) {
    throw new Error(`notebook: cells[${index}].id must be a non-empty string`);
  }
  if (
    type !== "markdown" &&
    type !== "esl" &&
    type !== "eigenql" &&
    type !== "typescript"
  ) {
    throw new Error(
      `notebook: cells[${index}].type must be one of markdown|esl|eigenql|typescript`,
    );
  }
  if (typeof source !== "string") {
    throw new Error(`notebook: cells[${index}].source must be a string`);
  }
  return { id, type, source };
}
