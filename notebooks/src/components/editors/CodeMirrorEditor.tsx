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

import CodeMirror, { type Extension } from "@uiw/react-codemirror";
import { EditorView } from "@codemirror/view";
import { javascript } from "@codemirror/lang-javascript";
import { markdown } from "@codemirror/lang-markdown";
import { eigenqlLanguage } from "./eigenql-mode";
import { eslLanguage } from "./esl-mode";
import type { CellType } from "../../persistence/notebook-format";

export interface CodeMirrorEditorProps {
  source: string;
  cellType: CellType;
  /**
   * When true, the editor renders read-only (Phase 2). Phase 4 sets
   * this false and supplies an `onChange` for editable authoring.
   */
  readOnly?: boolean;
  onChange?: (value: string) => void;
}

function languageExtension(cellType: CellType): Extension[] {
  switch (cellType) {
    case "markdown":
      return [markdown()];
    case "esl":
      return [eslLanguage];
    case "eigenql":
      return [eigenqlLanguage];
    case "typescript":
      return [javascript({ jsx: false, typescript: true })];
    case "program-run":
    case "chart":
      // The CodeMirror editor isn't used for program-run / chart
      // cells (they render via their own form-based editors) — but
      // to keep this switch exhaustive, return an empty extension list.
      return [];
  }
}

const baseTheme = EditorView.theme({
  "&": {
    fontSize: "13px",
    fontFamily:
      'ui-monospace, "SFMono-Regular", "Menlo", "Monaco", "Cascadia Mono", "Roboto Mono", monospace',
  },
  ".cm-content": { padding: "8px 12px" },
  ".cm-gutters": { background: "transparent", border: "none" },
  ".cm-focused": { outline: "none" },
});

export function CodeMirrorEditor(props: CodeMirrorEditorProps) {
  const { source, cellType, readOnly = true, onChange } = props;

  const extensions: Extension[] = [
    ...languageExtension(cellType),
    baseTheme,
    EditorView.lineWrapping,
  ];

  // Line numbers help in code cells (ESL / EigenQL / TypeScript) where
  // the kernel reports errors with line/column positions; suppressed
  // for prose markdown.
  const showLineNumbers = cellType !== "markdown";

  return (
    <CodeMirror
      value={source}
      readOnly={readOnly}
      editable={!readOnly}
      basicSetup={{
        lineNumbers: showLineNumbers,
        foldGutter: false,
        highlightActiveLine: !readOnly,
        highlightActiveLineGutter: false,
      }}
      extensions={extensions}
      onChange={onChange}
    />
  );
}
