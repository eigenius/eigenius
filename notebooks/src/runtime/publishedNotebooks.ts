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
 * Helpers for the "Open published notebook" dialog (D22 Phase 6
 * polish). Two operations:
 *
 *   1. `searchPublishedNotebooks` — EigenQL query against the active
 *      layer chain. Filters by title (always bound — title is a
 *      required Notebook property as of the Phase 6 ontology change);
 *      description is bound and filtered only if the caller provides
 *      a description query, since it remains optional. Result rows
 *      carry just `{ iri, title }` from the query; we then enrich
 *      each row with description / modified by inspecting the
 *      individual Notebook resource. The N+1 round-trips are the
 *      MVP workaround for EigenQL lacking OPTIONAL — see
 *      eigenius#33 — and are acceptable for typical workspace sizes.
 *
 *   2. `loadPublishedNotebook` — given a Notebook IRI, fetch the
 *      Notebook + every Cell it references, decode them all, and
 *      reconstruct the `NotebookJson` via the SDK's
 *      `resourcesToNotebookJson`. Used when the user picks a row in
 *      the search dialog and clicks Open.
 */

import { decode as cborDecode } from "cbor-x";
import {
  Eigen,
  type EigonResource,
  type NotebookJson,
  resourcesToNotebookJson,
} from "@eigenius/client";
import { decodeResultDocument } from "./resultDocument";

const NB_NS = "urn:eigenius:notebook";
const TITLE_PROP = `${NB_NS}:title`;
const DESCRIPTION_PROP = `${NB_NS}:description`;
const MODIFIED_PROP = `${NB_NS}:modified`;
const CELLS_PROP = `${NB_NS}:cells`;

export interface SearchFilters {
  /** Substring to LIKE-match against `notebook:title`. */
  titleQuery: string;
  /** Substring to LIKE-match against `notebook:description`. */
  descriptionQuery: string;
}

export interface PublishedNotebookSummary {
  iri: string;
  title: string;
  description: string;
  modified: string;
}

/**
 * Escape a substring for inclusion in an EigenQL LIKE-pattern string
 * literal. `\` and `"` would close or break the literal; `%` and `_`
 * have wildcard meaning and are passed through unchanged so callers
 * can opt into wildcards if they want, but we wrap the input in `%…%`
 * by default at the call site for substring matching.
 */
function escapeForLikeLiteral(s: string): string {
  return s.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}

export async function searchPublishedNotebooks(
  eigen: Eigen,
  filters: SearchFilters,
): Promise<readonly PublishedNotebookSummary[]> {
  const titleEsc = escapeForLikeLiteral(filters.titleQuery);
  const descEsc = escapeForLikeLiteral(filters.descriptionQuery);

  // title is always bound (required by the Notebook class). description
  // is bound only when the caller filters on it — binding it
  // unconditionally would exclude notebooks that were published before
  // it was provided.
  const matchProps = [`"${TITLE_PROP}": ?title`];
  const whereClauses = [`?title LIKE "%${titleEsc}%"`];
  if (filters.descriptionQuery.length > 0) {
    matchProps.push(`"${DESCRIPTION_PROP}": ?desc`);
    whereClauses.push(`?desc LIKE "%${descEsc}%"`);
  }

  const eigenql = `USING "${NB_NS}:Notebook"
USING NAMESPACE "${NB_NS}:"
MATCH Notebook(?n) {
  ${matchProps.join(",\n  ")}
}
WHERE ${whereClauses.join(" AND ")}
RETURN [] {
  iri: ?n,
  title: ?title
}
ORDER BY ?title
`;

  const resp = await eigen.query(eigenql);
  const decoded = decodeResultDocument(resp.document);

  // The result rows carry { iri, title }. Inspect each Notebook to
  // pick up the optional description / modified columns the dialog
  // wants to show.
  const irisAndTitles: { iri: string; title: string }[] = [];
  for (const row of decoded.rows) {
    let iri: string | undefined;
    let title: string | undefined;
    for (const [key, value] of row.values) {
      if (typeof value !== "string") continue;
      if (key.endsWith(":iri")) iri = value;
      else if (key.endsWith(":title")) title = value;
    }
    if (iri && title) irisAndTitles.push({ iri, title });
  }

  return await Promise.all(
    irisAndTitles.map(async ({ iri, title }) => {
      const inspect = await eigen.inspect(iri);
      let description = "";
      let modified = "";
      if (inspect.found && inspect.resource.length > 0) {
        const decoded = cborDecode(inspect.resource) as Record<string, unknown>;
        const d = decoded[DESCRIPTION_PROP];
        if (typeof d === "string") description = d;
        const m = decoded[MODIFIED_PROP];
        if (typeof m === "string") modified = m;
      }
      return { iri, title, description, modified };
    }),
  );
}

/**
 * Materialise a `NotebookJson` for a previously-published notebook.
 * Inspects the Notebook resource and every Cell it references, then
 * delegates to the SDK's `resourcesToNotebookJson` for the actual
 * shape reconstruction.
 *
 * Throws if the Notebook IRI doesn't resolve, or if any of its
 * referenced Cell IRIs are missing from the current layer chain.
 */
export async function loadPublishedNotebook(
  eigen: Eigen,
  notebookIri: string,
): Promise<NotebookJson> {
  const notebookInspect = await eigen.inspect(notebookIri);
  if (!notebookInspect.found || notebookInspect.resource.length === 0) {
    throw new Error(`Notebook resource not found: ${notebookIri}`);
  }
  const notebookCbor = cborDecode(notebookInspect.resource) as Record<
    string,
    unknown
  >;
  notebookCbor["@id"] = notebookIri;
  const cellIrisRaw = notebookCbor[CELLS_PROP];
  const cellIris: string[] = Array.isArray(cellIrisRaw)
    ? cellIrisRaw.filter((v): v is string => typeof v === "string")
    : [];

  const cellResources = await Promise.all(
    cellIris.map(async (cellIri) => {
      const inspect = await eigen.inspect(cellIri);
      if (!inspect.found || inspect.resource.length === 0) {
        throw new Error(
          `Cell resource not found: ${cellIri} (referenced by ${notebookIri})`,
        );
      }
      const cell = cborDecode(inspect.resource) as Record<string, unknown>;
      cell["@id"] = cellIri;
      return cell as EigonResource;
    }),
  );

  const resources: EigonResource[] = [
    notebookCbor as EigonResource,
    ...cellResources,
  ];
  return resourcesToNotebookJson(notebookIri, resources);
}
