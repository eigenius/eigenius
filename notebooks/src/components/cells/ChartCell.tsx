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
 * Chart cell editor (Phase 5d). A two-section form:
 *   1. Chart spec — kind dropdown, title, x/y/series column names
 *   2. Query     — CodeMirror EigenQL editor; the cell's Run button
 *      executes it, decodes the ResultSet, pivots according to the
 *      spec, and returns a Fluent chart React element.
 *
 * The query's `RETURN` short-names are referenced by the column
 * fields. We don't auto-discover them — keeps the editor static and
 * predictable; the user already wrote the RETURN clause.
 */

import {
  Field,
  Input,
  makeStyles,
  Select,
  tokens,
} from "@fluentui/react-components";
import type {
  ChartCellJson,
  ChartKind,
} from "../../persistence/notebook-format";
import { useNotebookStore } from "../../runtime/notebookStore";
import { CodeMirrorEditor } from "../editors/CodeMirrorEditor";

const useStyles = makeStyles({
  root: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalS,
  },
  formRow: {
    display: "grid",
    gridTemplateColumns: "1fr 1fr 1fr",
    gap: tokens.spacingHorizontalM,
  },
  formRowFull: {
    display: "grid",
    gridTemplateColumns: "1fr 2fr",
    gap: tokens.spacingHorizontalM,
  },
  monoInput: {
    fontFamily: tokens.fontFamilyMonospace,
  },
  queryLabel: {
    fontSize: tokens.fontSizeBase200,
    color: tokens.colorNeutralForeground3,
  },
});

const CHART_KIND_LABELS: Record<ChartKind, string> = {
  "grouped-bar": "Grouped vertical bar",
  "vertical-bar": "Vertical bar",
  "horizontal-bar": "Horizontal bar",
  "donut": "Donut",
  "line": "Line",
  "area": "Area",
};

const CHART_KIND_ORDER: readonly ChartKind[] = [
  "grouped-bar",
  "vertical-bar",
  "horizontal-bar",
  "donut",
  "line",
  "area",
];

export interface ChartCellEditorProps {
  cellId: string;
  cell: ChartCellJson;
}

export function ChartCellEditor({ cellId, cell }: ChartCellEditorProps) {
  const styles = useStyles();
  const updateChartCell = useNotebookStore((s) => s.updateChartCell);

  const update = (partial: Partial<Omit<ChartCellJson, "id" | "type">>) =>
    updateChartCell(cellId, partial);

  const supportsSeries = cell.chart_kind === "grouped-bar" ||
    cell.chart_kind === "line" || cell.chart_kind === "area";
  const xLabel = cell.chart_kind === "donut"
    ? "Slice label column"
    : "X column";
  const yLabel = cell.chart_kind === "donut"
    ? "Slice value column"
    : "Y column";

  return (
    <div className={styles.root}>
      <div className={styles.formRowFull}>
        <Field label="Chart kind">
          <Select
            value={cell.chart_kind}
            onChange={(_e, data) =>
              update({ chart_kind: data.value as ChartKind })}
          >
            {CHART_KIND_ORDER.map((k) => (
              <option key={k} value={k}>{CHART_KIND_LABELS[k]}</option>
            ))}
          </Select>
        </Field>
        <Field label="Title (optional)">
          <Input
            value={cell.title ?? ""}
            placeholder="Chart title"
            onChange={(_e, data) => update({ title: data.value })}
          />
        </Field>
      </div>

      <div className={styles.formRow}>
        <Field label={xLabel}>
          <Input
            className={styles.monoInput}
            value={cell.x_column}
            placeholder="e.g. compound"
            onChange={(_e, data) => update({ x_column: data.value })}
          />
        </Field>
        <Field label={yLabel}>
          <Input
            className={styles.monoInput}
            value={cell.y_column}
            placeholder="e.g. ic50_nm"
            onChange={(_e, data) => update({ y_column: data.value })}
          />
        </Field>
        <Field
          label={supportsSeries
            ? "Series column (optional)"
            : "Series column (n/a)"}
        >
          <Input
            className={styles.monoInput}
            value={cell.series_column ?? ""}
            placeholder={supportsSeries ? "e.g. target" : "—"}
            disabled={!supportsSeries}
            onChange={(_e, data) =>
              update({
                series_column: data.value.length === 0 ? undefined : data.value,
              })}
          />
        </Field>
      </div>

      <Field label="EigenQL query">
        <CodeMirrorEditor
          source={cell.query}
          cellType="eigenql"
          readOnly={false}
          onChange={(value) => update({ query: value })}
        />
      </Field>
    </div>
  );
}
