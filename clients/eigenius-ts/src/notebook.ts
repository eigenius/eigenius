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
 * Notebook ↔ resource translator (D22 Phase 3.5).
 *
 * The on-disk notebook format (`NotebookJson`, see notebooks/src/persistence)
 * is the portable transport: a JSON file you can email, paste in a gist,
 * or check into git. This module translates between that file shape and
 * the `notebook:Notebook` / `notebook:Cell` resources defined by
 * `ontologies/notebook/notebook-ontology.json`.
 *
 * IRI scheme — content-addressed:
 *
 *   Cell IRI     = urn:eigenius:notebook:cell:<sha256>
 *                  hash input: canonical JSON of {cell_type, source}.
 *                  Identical (cell_type, source) pairs across notebooks
 *                  yield the same IRI → cells de-duplicate naturally.
 *
 *   Notebook IRI = urn:eigenius:notebook:<sha256>
 *                  hash input: canonical JSON of
 *                    { format_version, title?, description?, cells:[<cellIri>...] }
 *                  Excluded from the hash (intentional): created, modified,
 *                  eigenius_version. Re-saving identical content at a different
 *                  time yields the same Notebook IRI.
 *
 * The hash input is JSON.stringify of an object literal with a fixed
 * key order — sufficient for stable identity across SDK runs. If a future
 * SDK change reorders keys, the IRIs would change; bump the SDK
 * majorversion at that point.
 *
 * Cell IRIs do NOT include the per-file UUID from `CellJson.id` — that's
 * an authoring artifact of a particular file, not a knowledge-graph
 * identity. Two notebooks containing the same exact ESL source share a
 * single Cell resource regardless of how the editor labels them.
 */

const NB_NS = "urn:eigenius:notebook";

const IS_A = "urn:eigenius:core:is_a";
const SHORT_NAME = "urn:eigenius:core:short_name";
const DESCRIPTION = "urn:eigenius:core:description";

const CELL_CLASS = `${NB_NS}:Cell`;
const NOTEBOOK_CLASS = `${NB_NS}:Notebook`;

const FORMAT_VERSION_PROP = `${NB_NS}:format_version`;
const TITLE_PROP = `${NB_NS}:title`;
const DESCRIPTION_PROP = `${NB_NS}:description`;
const CREATED_PROP = `${NB_NS}:created`;
const MODIFIED_PROP = `${NB_NS}:modified`;
const EIGENIUS_VERSION_PROP = `${NB_NS}:eigenius_version`;
const CELLS_PROP = `${NB_NS}:cells`;
const CELL_TYPE_PROP = `${NB_NS}:cell_type`;
const SOURCE_PROP = `${NB_NS}:source`;

const CELL_TYPE_IRI: Record<CellType, string> = {
  markdown: `${NB_NS}:markdown`,
  esl: `${NB_NS}:esl`,
  eigenql: `${NB_NS}:eigenql`,
  typescript: `${NB_NS}:typescript`,
};

const IRI_TO_CELL_TYPE: Record<string, CellType> = {
  [`${NB_NS}:markdown`]: "markdown",
  [`${NB_NS}:esl`]: "esl",
  [`${NB_NS}:eigenql`]: "eigenql",
  [`${NB_NS}:typescript`]: "typescript",
};

// Mirror of NotebookJson / CellJson in notebooks/src/persistence/notebook-format.ts.
// The SDK keeps its own copy so it doesn't depend on the notebook UI package.

export type CellType = "markdown" | "esl" | "eigenql" | "typescript";

export interface CellJson {
  /** UUID assigned by the editor — NOT used in the cell's content-addressed IRI. */
  id: string;
  type: CellType;
  source: string;
}

export interface NotebookMetaJson {
  title?: string;
  description?: string;
  created?: string;
  modified?: string;
  eigenius_version?: string;
}

export interface NotebookJson {
  format_version: number;
  meta: NotebookMetaJson;
  cells: CellJson[];
}

/** Eigon-JSON resource — IRI-keyed map plus an `@id` field. */
export type EigonResource = { "@id": string } & Record<string, unknown>;

export interface PublishOutput {
  /** Content-addressed Notebook IRI. */
  notebookIri: string;
  /** Content-addressed Cell IRIs (in source order). */
  cellIris: readonly string[];
  /** All resources to send via Load (Notebook + unique cells). */
  resources: readonly EigonResource[];
}

/**
 * Translate a NotebookJson into the resources you'd send to
 * `eigen.load(JSON.stringify({resources}), {contentType: "application/eigon+json"})`.
 *
 * Cells are de-duplicated by content: if two cells in the same notebook
 * (or two notebooks in the same load batch) have identical (type, source),
 * only one Cell resource is produced.
 */
export async function notebookJsonToResources(
  notebook: NotebookJson,
): Promise<PublishOutput> {
  // First pass: compute every cell IRI (preserving order).
  const cellIris: string[] = [];
  const cellResources = new Map<string, EigonResource>();

  for (const cell of notebook.cells) {
    const iri = await cellIri(cell.type, cell.source);
    cellIris.push(iri);
    if (!cellResources.has(iri)) {
      cellResources.set(iri, makeCellResource(iri, cell.type, cell.source));
    }
  }

  // Notebook IRI hashes the structural form (excludes timestamps).
  const nbIri = await notebookIri(notebook, cellIris);

  const meta = notebook.meta ?? {};
  const notebookResource: EigonResource = {
    "@id": nbIri,
    [IS_A]: [NOTEBOOK_CLASS],
    [DESCRIPTION]: meta.description ??
      "Notebook published via @eigenius/client.",
    [SHORT_NAME]: nbIri.slice(`${NB_NS}:`.length, `${NB_NS}:`.length + 12),
    [FORMAT_VERSION_PROP]: notebook.format_version,
    [CELLS_PROP]: cellIris,
  };
  if (meta.title) notebookResource[TITLE_PROP] = meta.title;
  if (meta.description) notebookResource[DESCRIPTION_PROP] = meta.description;
  if (meta.created) notebookResource[CREATED_PROP] = meta.created;
  if (meta.modified) notebookResource[MODIFIED_PROP] = meta.modified;
  if (meta.eigenius_version) {
    notebookResource[EIGENIUS_VERSION_PROP] = meta.eigenius_version;
  }

  return {
    notebookIri: nbIri,
    cellIris,
    resources: [notebookResource, ...cellResources.values()],
  };
}

/**
 * Inverse of `notebookJsonToResources` — given a Notebook IRI and the
 * full set of resources reachable in the layer (typically the result of
 * a query that pulled the Notebook + its cells), reconstruct the
 * NotebookJson.
 *
 * Cells are matched by IRI from the Notebook's `cells` array; missing
 * cells yield a placeholder error cell so the user sees the gap rather
 * than silent data loss.
 */
export function resourcesToNotebookJson(
  notebookIri: string,
  resources: readonly EigonResource[],
): NotebookJson {
  const byIri = new Map(resources.map((r) => [r["@id"], r]));
  const notebook = byIri.get(notebookIri);
  if (!notebook) {
    throw new Error(`notebook resource not found: ${notebookIri}`);
  }

  const cellIris = asStringArray(notebook[CELLS_PROP]);
  const cells: CellJson[] = cellIris.map((iri, index) => {
    const cell = byIri.get(iri);
    if (!cell) {
      // The IRI in the Notebook's cells array doesn't resolve in the
      // provided resource set. Surface as a placeholder so the editor
      // shows the gap rather than silently dropping the cell.
      return {
        id: iri,
        type: "markdown",
        source:
          `> Cell ${iri} not found in the published layer. The notebook's cells array references this IRI but the resource was not in the load batch.`,
      };
    }
    const typeIri = String(cell[CELL_TYPE_PROP] ?? "");
    const cellType = IRI_TO_CELL_TYPE[typeIri] ?? "markdown";
    const source = String(cell[SOURCE_PROP] ?? "");
    // Reconstruct a UUID from the IRI tail so re-saving is round-trippable.
    return { id: `nb-${index}-${iri.slice(-12)}`, type: cellType, source };
  });

  const meta: NotebookMetaJson = {};
  const title = notebook[TITLE_PROP];
  if (typeof title === "string") meta.title = title;
  const description = notebook[DESCRIPTION_PROP];
  if (typeof description === "string") meta.description = description;
  const created = notebook[CREATED_PROP];
  if (typeof created === "string") meta.created = created;
  const modified = notebook[MODIFIED_PROP];
  if (typeof modified === "string") meta.modified = modified;
  const eigeniusVersion = notebook[EIGENIUS_VERSION_PROP];
  if (typeof eigeniusVersion === "string") {
    meta.eigenius_version = eigeniusVersion;
  }

  const formatVersion = notebook[FORMAT_VERSION_PROP];
  return {
    format_version: typeof formatVersion === "number" ? formatVersion : 1,
    meta,
    cells,
  };
}

// ---------------------------------------------------------------------------
// IRI computation
// ---------------------------------------------------------------------------

async function cellIri(type: CellType, source: string): Promise<string> {
  // Fixed key order: cell_type before source.
  const canonical = JSON.stringify({ cell_type: type, source });
  const hash = await sha256Hex(canonical);
  return `${NB_NS}:cell:${hash}`;
}

async function notebookIri(
  notebook: NotebookJson,
  cellIris: readonly string[],
): Promise<string> {
  // Fixed key order. Excludes meta.created / meta.modified /
  // meta.eigenius_version so identity is stable across re-saves.
  const meta = notebook.meta ?? {};
  const canonical = JSON.stringify({
    format_version: notebook.format_version,
    title: meta.title ?? null,
    description: meta.description ?? null,
    cells: cellIris,
  });
  const hash = await sha256Hex(canonical);
  return `${NB_NS}:${hash}`;
}

async function sha256Hex(input: string): Promise<string> {
  const data = new TextEncoder().encode(input);
  const digest = await crypto.subtle.digest("SHA-256", data);
  return Array.from(new Uint8Array(digest))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeCellResource(
  iri: string,
  type: CellType,
  source: string,
): EigonResource {
  return {
    "@id": iri,
    [IS_A]: [CELL_CLASS],
    [DESCRIPTION]: `${type} cell`,
    [SHORT_NAME]: iri.slice(`${NB_NS}:cell:`.length, `${NB_NS}:cell:`.length + 12),
    [CELL_TYPE_PROP]: CELL_TYPE_IRI[type],
    [SOURCE_PROP]: source,
  };
}

function asStringArray(value: unknown): string[] {
  if (Array.isArray(value)) {
    return value.filter((v): v is string => typeof v === "string");
  }
  return typeof value === "string" ? [value] : [];
}
