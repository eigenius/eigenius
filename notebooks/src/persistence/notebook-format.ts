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

export type CellType =
  | "markdown"
  | "esl"
  | "eigenql"
  | "typescript"
  | "program-run"
  | "chart";

/** Source-bearing cell types (markdown, esl, eigenql, typescript). */
export interface SourceCellJson {
  /** Stable UUID assigned at cell creation. */
  id: string;
  type: "markdown" | "esl" | "eigenql" | "typescript";
  /** The cell's editable text. */
  source: string;
}

/**
 * Program-run cell (Phase 4d) — invokes a program already loaded into
 * the active layer chain against one or more inputs (also identified
 * by IRI). Single input renders a ResourceInspector + TraceTree;
 * multiple inputs render a results table with one row per input.
 */
export interface ProgramRunCellJson {
  id: string;
  type: "program-run";
  /** IRI of the program resource to invoke. */
  program_iri: string;
  /** IRIs of the input resources (one row per input on run). */
  input_iris: string[];
}

/**
 * Supported chart kinds for `ChartCellJson` (Phase 5d). Each maps to
 * a Fluent `@fluentui/react-charts` component; the executor pivots
 * the EigenQL ResultSet into the shape that component expects.
 */
export type ChartKind =
  | "grouped-bar" // GroupedVerticalBarChart
  | "vertical-bar" // VerticalBarChart
  | "horizontal-bar" // HorizontalBarChart
  | "donut" // DonutChart
  | "line" // LineChart
  | "area"; // AreaChart

/**
 * Chart cell (Phase 5d) — runs an EigenQL query and renders the
 * result as a Fluent chart. Form-based; no TypeScript required for
 * the common "chart this query" case.
 *
 * `x_column`, `y_column`, and `series_column` reference the short-
 * names from the query's `RETURN` clause. Single-axis charts
 * (donut) use `x_column` for the legend label and `y_column` for
 * the slice value; line/bar charts use `x_column` for the x-axis
 * and `y_column` for the y-axis. `series_column` is optional and
 * pivots the rows into one series per distinct value in that column
 * — required for `grouped-bar`, ignored by `donut`.
 */
export interface ChartCellJson {
  id: string;
  type: "chart";
  query: string;
  chart_kind: ChartKind;
  x_column: string;
  y_column: string;
  series_column?: string;
  title?: string;
}

export type CellJson = SourceCellJson | ProgramRunCellJson | ChartCellJson;

export interface NotebookMetaJson {
  /**
   * Required, non-empty notebook title. Surfaced as the queryable
   * `urn:eigenius:notebook:title` property on publish — Phase 6
   * promoted this from optional to required so the published-notebook
   * search dialog can rely on it without OPTIONAL-pattern semantics
   * in EigenQL (see issue #33).
   */
  title: string;
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
 * Phase 4d adds the program-run cell shape (program_iri + input_iris[]).
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
  const metaRaw = (obj.meta ?? {}) as Record<string, unknown>;
  if (typeof metaRaw.title !== "string" || metaRaw.title.trim().length === 0) {
    throw new Error(
      "notebook: meta.title is required and must be a non-empty string",
    );
  }
  const meta = metaRaw as unknown as NotebookMetaJson;
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
  if (typeof id !== "string" || id.length === 0) {
    throw new Error(`notebook: cells[${index}].id must be a non-empty string`);
  }
  if (type === "program-run") {
    const program_iri = obj.program_iri;
    const input_iris = obj.input_iris;
    if (typeof program_iri !== "string") {
      throw new Error(
        `notebook: cells[${index}].program_iri must be a string`,
      );
    }
    if (
      !Array.isArray(input_iris) ||
      !input_iris.every((s) => typeof s === "string")
    ) {
      throw new Error(
        `notebook: cells[${index}].input_iris must be an array of strings`,
      );
    }
    return { id, type, program_iri, input_iris };
  }
  if (type === "chart") {
    const query = obj.query;
    const chart_kind = obj.chart_kind;
    const x_column = obj.x_column;
    const y_column = obj.y_column;
    const series_column = obj.series_column;
    const title = obj.title;
    if (typeof query !== "string") {
      throw new Error(`notebook: cells[${index}].query must be a string`);
    }
    if (
      chart_kind !== "grouped-bar" &&
      chart_kind !== "vertical-bar" &&
      chart_kind !== "horizontal-bar" &&
      chart_kind !== "donut" &&
      chart_kind !== "line" &&
      chart_kind !== "area"
    ) {
      throw new Error(
        `notebook: cells[${index}].chart_kind must be one of grouped-bar|vertical-bar|horizontal-bar|donut|line|area`,
      );
    }
    if (typeof x_column !== "string") {
      throw new Error(
        `notebook: cells[${index}].x_column must be a string`,
      );
    }
    if (typeof y_column !== "string") {
      throw new Error(
        `notebook: cells[${index}].y_column must be a string`,
      );
    }
    if (series_column !== undefined && typeof series_column !== "string") {
      throw new Error(
        `notebook: cells[${index}].series_column must be a string when present`,
      );
    }
    if (title !== undefined && typeof title !== "string") {
      throw new Error(
        `notebook: cells[${index}].title must be a string when present`,
      );
    }
    return {
      id,
      type,
      query,
      chart_kind,
      x_column,
      y_column,
      ...(series_column !== undefined ? { series_column } : {}),
      ...(title !== undefined ? { title } : {}),
    };
  }
  if (
    type !== "markdown" &&
    type !== "esl" &&
    type !== "eigenql" &&
    type !== "typescript"
  ) {
    throw new Error(
      `notebook: cells[${index}].type must be one of markdown|esl|eigenql|typescript|program-run|chart`,
    );
  }
  const source = obj.source;
  if (typeof source !== "string") {
    throw new Error(`notebook: cells[${index}].source must be a string`);
  }
  return { id, type, source };
}
