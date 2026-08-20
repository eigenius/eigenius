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
 * Formalization cell editor (D71).
 *
 * Unlike every other cell, the source here is PROSE, not code — so it gets a
 * plain textarea rather than CodeMirror: there is no syntax to highlight, and a
 * code editor would suggest there is.
 *
 * Two fields sit beside it. `doc_id` names the run's `doc-<id>` working branch,
 * which holds the document glossary and the run's recorded proposer draws —
 * keeping it stable across runs is what makes a re-run replay those draws
 * instead of re-asking the model. `lexicon_profile` names the ordered parse
 * scope; empty means the whole chain.
 *
 * Running is asynchronous and can take minutes; the store polls the task.
 */

import {
  Field,
  Input,
  makeStyles,
  Switch,
  Textarea,
  tokens,
} from "@fluentui/react-components";
import type { FormalizeCellJson } from "../../persistence/notebook-format";
import { useNotebookStore } from "../../runtime/notebookStore";

const useStyles = makeStyles({
  root: {
    display: "flex",
    flexDirection: "column",
    gap: tokens.spacingVerticalS,
  },
  formRow: {
    display: "grid",
    gridTemplateColumns: "1fr 2fr",
    gap: tokens.spacingHorizontalM,
  },
  prose: {
    width: "100%",
  },
  hint: {
    color: tokens.colorNeutralForeground3,
    fontSize: tokens.fontSizeBase200,
  },
});

export function FormalizeCellEditor(
  { cellId, cell }: { cellId: string; cell: FormalizeCellJson },
) {
  const styles = useStyles();
  const update = useNotebookStore((s) => s.updateFormalizeCell);

  return (
    <div className={styles.root} data-testid="formalize-cell">
      <Textarea
        className={styles.prose}
        resize="vertical"
        rows={6}
        value={cell.source}
        placeholder="Prose to formalize — one claim per sentence parses best."
        aria-label="Prose to formalize"
        data-testid="formalize-prose"
        onChange={(_e, d) => update(cellId, { source: d.value })}
      />
      <div className={styles.formRow}>
        <Field
          label="Document id"
          hint="Names the doc-<id> working branch. Keep it stable to replay this run's recorded draws instead of re-asking the model."
        >
          <Input
            value={cell.doc_id ?? ""}
            placeholder={cellId}
            data-testid="formalize-doc-id"
            onChange={(_e, d) => update(cellId, { doc_id: d.value })}
          />
        </Field>
        <Field
          label="Lexicon profile (optional)"
          hint="A lexicon:LexiconProfile IRI naming the ordered parse scope. Empty = the whole chain."
        >
          <Input
            value={cell.lexicon_profile ?? ""}
            placeholder="urn:eigenius:lexicon:profile:…"
            data-testid="formalize-profile"
            onChange={(_e, d) => update(cellId, { lexicon_profile: d.value })}
          />
        </Field>
      </div>
      <Field
        hint="Off: the run produces an artifact to read, and landing it is a separate step. On: `Run all` reproduces the chain state too. Loading is idempotent — unchanged prose does not advance the branch."
      >
        <Switch
          checked={cell.land ?? false}
          label="Land the artifact on run"
          data-testid="formalize-land"
          onChange={(_e, d) => update(cellId, { land: d.checked })}
        />
      </Field>
      {cell.structure_iri && (
        <div className={styles.hint} data-testid="formalize-structure-iri">
          Last run produced {cell.structure_iri} — the artifact is not
          committed; load it explicitly to land the claims.
        </div>
      )}
    </div>
  );
}
