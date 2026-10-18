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

import { useMemo } from "react";
import {
  Caption1,
  createTableColumn,
  DataGrid,
  DataGridBody,
  DataGridCell,
  DataGridHeader,
  DataGridHeaderCell,
  DataGridRow,
  makeStyles,
  type TableColumnDefinition,
  tokens,
} from "@fluentui/react-components";
import {
  type ColumnMeta,
  type DecodedRow,
  decodeResultDocument,
} from "../../runtime/resultDocument";

const useStyles = makeStyles({
  root: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalS,
  },
  meta: {
    color: tokens.colorNeutralForeground3,
  },
  emptyState: {
    color: tokens.colorNeutralForeground3,
    fontStyle: "italic",
  },
  numericCell: {
    textAlign: "right",
    fontVariantNumeric: "tabular-nums",
  },
});

export interface ResultTableProps {
  /** Raw Eigon-CBOR `QueryResponse.document` bytes. */
  document: Uint8Array;
}

export function ResultTable({ document }: ResultTableProps) {
  const styles = useStyles();
  const decoded = useMemo(() => decodeResultDocument(document), [document]);
  const columns = useMemo(
    () => buildColumns(decoded.columns, styles.numericCell),
    [decoded.columns, styles.numericCell],
  );

  if (decoded.columns.length === 0) {
    return (
      <Caption1 className={styles.emptyState}>
        match-only query — {decoded.matched ? "matched" : "no matches"}
      </Caption1>
    );
  }
  if (decoded.rows.length === 0) {
    return (
      <Caption1 className={styles.emptyState}>
        no rows ({decoded.columns.length} column
        {decoded.columns.length === 1 ? "" : "s"})
      </Caption1>
    );
  }

  return (
    <div className={styles.root}>
      <DataGrid
        items={decoded.rows as DecodedRow[]}
        columns={columns}
        sortable
        size="small"
      >
        <DataGridHeader>
          <DataGridRow>
            {({ renderHeaderCell }) => (
              <DataGridHeaderCell>{renderHeaderCell()}</DataGridHeaderCell>
            )}
          </DataGridRow>
        </DataGridHeader>
        <DataGridBody<DecodedRow>>
          {({ item, rowId }) => (
            <DataGridRow<DecodedRow> key={rowId}>
              {({ renderCell }) => (
                <DataGridCell>{renderCell(item)}</DataGridCell>
              )}
            </DataGridRow>
          )}
        </DataGridBody>
      </DataGrid>
      <Caption1 className={styles.meta}>
        {decoded.rows.length} row{decoded.rows.length === 1 ? "" : "s"}
      </Caption1>
    </div>
  );
}

function buildColumns(
  metas: readonly ColumnMeta[],
  numericClass: string,
): TableColumnDefinition<DecodedRow>[] {
  return metas.map((col) =>
    createTableColumn<DecodedRow>({
      columnId: col.iri,
      compare: makeComparator(col),
      renderHeaderCell: () => col.shortName,
      renderCell: (row) => {
        const raw = row.values.get(col.iri);
        const formatted = formatValue(raw, col.dataType);
        if (isNumericType(col.dataType)) {
          return <span className={numericClass}>{formatted}</span>;
        }
        return formatted;
      },
    })
  );
}

function makeComparator(col: ColumnMeta) {
  if (isNumericType(col.dataType)) {
    return (a: DecodedRow, b: DecodedRow) => {
      const av = toNumber(a.values.get(col.iri));
      const bv = toNumber(b.values.get(col.iri));
      return av - bv;
    };
  }
  return (a: DecodedRow, b: DecodedRow) => {
    const av = formatValue(a.values.get(col.iri), col.dataType);
    const bv = formatValue(b.values.get(col.iri), col.dataType);
    return av.localeCompare(bv);
  };
}

const NUMERIC_TYPES = new Set([
  "urn:eigenius:core:Integer",
  "urn:eigenius:core:Float",
]);

function isNumericType(dataType: string): boolean {
  return NUMERIC_TYPES.has(dataType);
}

function toNumber(value: unknown): number {
  if (typeof value === "number") return value;
  if (typeof value === "bigint") return Number(value);
  return Number.NaN;
}

function formatValue(value: unknown, _dataType: string): string {
  if (value === null || value === undefined) return "";
  if (typeof value === "string") return value;
  if (typeof value === "number") return String(value);
  if (typeof value === "bigint") return value.toString();
  if (typeof value === "boolean") return value ? "true" : "false";
  if (Array.isArray(value)) {
    return `[${value.map((v) => formatValue(v, "")).join(", ")}]`;
  }
  // Embedded resources / unknown shapes — best-effort JSON.
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}
