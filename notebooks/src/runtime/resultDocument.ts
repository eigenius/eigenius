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
 * Decode an Eigon-CBOR query result document (D2 Appendix A) into a
 * structure the renderer can iterate without re-walking IRI maps.
 *
 * Input: the raw bytes returned in `QueryResponse.document`.
 * Output: a typed `DecodedResultSet` with column metadata pulled from
 *         the synthesized Property resources (D22 §6.7 — column types
 *         are the synthesized Property `data_type`, never sniffed from
 *         the values).
 */

import { decode as cborDecode } from "cbor-x";

const IS_A = "urn:eigenius:core:is_a";
const SHORT_NAME = "urn:eigenius:core:short_name";
const DATA_TYPE = "urn:eigenius:core:data_type";
const PROPERTIES = "urn:eigenius:core:properties";
const ID = "@id";

const RESULT_SET_CLASS = "urn:eigenius:query:ResultSet";
const RESULT_CLASS_PROP = "urn:eigenius:query:result_class";
const ROWS_PROP = "urn:eigenius:query:rows";
const ROW_COUNT_PROP = "urn:eigenius:query:row_count";
const MATCHED_PROP = "urn:eigenius:query:matched";

export interface ColumnMeta {
  /** Synthesized Property IRI used as the row's value key. */
  iri: string;
  /** Bare name from the RETURN clause; used as the column header. */
  shortName: string;
  /** Datatype IRI from the synthesized Property; drives formatting. */
  dataType: string;
}

export interface DecodedRow {
  /** Map keyed by the synthesized Property IRI. */
  values: ReadonlyMap<string, unknown>;
}

export interface DecodedResultSet {
  columns: readonly ColumnMeta[];
  rows: readonly DecodedRow[];
  rowCount: number;
  matched: boolean;
}

interface CborResource {
  [key: string]: unknown;
}

function isResource(value: unknown): value is CborResource {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function asArray(value: unknown): unknown[] {
  if (Array.isArray(value)) return value;
  return value === undefined ? [] : [value];
}

function asString(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function isA(resource: CborResource, classIri: string): boolean {
  const tags = resource[IS_A];
  if (!Array.isArray(tags)) return false;
  return tags.some((t) => t === classIri);
}

export function decodeResultDocument(bytes: Uint8Array): DecodedResultSet {
  const decoded = cborDecode(bytes) as unknown;
  const resources: CborResource[] =
    (Array.isArray(decoded) ? decoded : [decoded])
      .filter(isResource);

  const resultSet = resources.find((r) => isA(r, RESULT_SET_CLASS));
  if (!resultSet) {
    throw new Error("result document has no ResultSet resource");
  }

  const matched = resultSet[MATCHED_PROP] !== false; // default true
  const rowCountValue = resultSet[ROW_COUNT_PROP];
  const rowCount = typeof rowCountValue === "number"
    ? rowCountValue
    : typeof rowCountValue === "bigint"
    ? Number(rowCountValue)
    : 0;

  // Locate the row class via the result_class IRI pointer.
  const rowClassIri = asString(resultSet[RESULT_CLASS_PROP]);
  let columns: ColumnMeta[] = [];
  if (rowClassIri) {
    const rowClass = resources.find((r) => r[ID] === rowClassIri);
    if (rowClass) {
      const propertyIris = asArray(rowClass[PROPERTIES]).map(asString)
        .filter((s): s is string => Boolean(s));
      columns = propertyIris.map((propIri): ColumnMeta => {
        const prop = resources.find((r) => r[ID] === propIri);
        return {
          iri: propIri,
          shortName: asString(prop?.[SHORT_NAME]) ?? propIri,
          dataType: asString(prop?.[DATA_TYPE]) ?? "",
        };
      });
    }
  }

  // Rows are embedded inline in the ResultSet (per query/document.rs:187).
  const rowValues = asArray(resultSet[ROWS_PROP])
    .filter(isResource)
    .map((r): DecodedRow => ({
      values: new Map(
        Object.entries(r).filter(([k]) => k !== ID && k !== IS_A),
      ),
    }));

  return { columns, rows: rowValues, rowCount, matched };
}
