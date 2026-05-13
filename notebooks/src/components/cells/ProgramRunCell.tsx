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
 * Program-run cell editor (Phase 4d). A small form: one program IRI
 * field, a list of input IRI fields (add/remove), no source editor.
 * `Run` on the cell toolbar dispatches `eigen.runProgramByIri` once
 * per non-empty input IRI; the auto-renderer shows a single-result
 * panel for N=1 or a results table for N>1.
 */

import {
  Button,
  Field,
  Input,
  makeStyles,
  tokens,
  Tooltip,
} from "@fluentui/react-components";
import { Add16Regular, Delete16Regular } from "@fluentui/react-icons";
import type { ProgramRunCellJson } from "../../persistence/notebook-format";
import { useNotebookStore } from "../../runtime/notebookStore";

const useStyles = makeStyles({
  root: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalS,
  },
  inputRow: {
    display: "flex",
    alignItems: "center",
    gap: tokens.spacingHorizontalXS,
  },
  inputField: {
    flex: 1,
    fontFamily: tokens.fontFamilyMonospace,
  },
  addRow: {
    display: "flex",
  },
});

export interface ProgramRunCellEditorProps {
  cellId: string;
  cell: ProgramRunCellJson;
}

export function ProgramRunCellEditor(
  { cellId, cell }: ProgramRunCellEditorProps,
) {
  const styles = useStyles();
  const updateProgramRunCell = useNotebookStore(
    (s) => s.updateProgramRunCell,
  );

  const setProgramIri = (value: string) =>
    updateProgramRunCell(cellId, { program_iri: value });

  const setInputAt = (index: number, value: string) => {
    const next = cell.input_iris.slice();
    next[index] = value;
    updateProgramRunCell(cellId, { input_iris: next });
  };

  const removeInputAt = (index: number) => {
    const next = cell.input_iris.slice();
    next.splice(index, 1);
    updateProgramRunCell(cellId, { input_iris: next });
  };

  const addInput = () =>
    updateProgramRunCell(cellId, {
      input_iris: [...cell.input_iris, ""],
    });

  // Always show at least one input row even if the array is empty.
  const inputs = cell.input_iris.length > 0 ? cell.input_iris : [""];
  if (cell.input_iris.length === 0) {
    // Lazy-initialize one empty row in the store on first render.
    queueMicrotask(() => updateProgramRunCell(cellId, { input_iris: [""] }));
  }

  return (
    <div className={styles.root}>
      <Field label="Program IRI">
        <Input
          className={styles.inputField}
          value={cell.program_iri}
          placeholder="urn:eigenius:demo:patent:analyze_patent"
          onChange={(_e, data) => setProgramIri(data.value)}
        />
      </Field>
      <Field label={`Input IRI${inputs.length === 1 ? "" : "s"}`}>
        <div className={styles.root}>
          {inputs.map((iri, idx) => (
            <div key={idx} className={styles.inputRow}>
              <Input
                className={styles.inputField}
                value={iri}
                placeholder="urn:eigenius:demo:patent:US10452978B2"
                onChange={(_e, data) => setInputAt(idx, data.value)}
              />
              <Tooltip content="Remove this input" relationship="label">
                <Button
                  size="small"
                  appearance="subtle"
                  icon={<Delete16Regular />}
                  disabled={inputs.length <= 1}
                  onClick={() => removeInputAt(idx)}
                />
              </Tooltip>
            </div>
          ))}
          <div className={styles.addRow}>
            <Button
              size="small"
              appearance="subtle"
              icon={<Add16Regular />}
              onClick={addInput}
            >
              Add input
            </Button>
          </div>
        </div>
      </Field>
    </div>
  );
}
