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

import { useState } from "react";
import { Button, makeStyles, tokens } from "@fluentui/react-components";
import { Edit16Regular, Eye16Regular } from "@fluentui/react-icons";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import rehypeKatex from "rehype-katex";
import "katex/dist/katex.min.css";
import { CodeMirrorEditor } from "../editors/CodeMirrorEditor";

const useStyles = makeStyles({
  root: {
    position: "relative",
  },
  toggle: {
    position: "absolute",
    top: tokens.spacingVerticalXS,
    right: tokens.spacingHorizontalXS,
    zIndex: 1,
  },
});

export interface MarkdownCellProps {
  source: string;
  onChange: (value: string) => void;
}

/**
 * Markdown cell — Jupyter-style edit/render toggle.
 *
 * Defaults to rendered view; the small Edit toggle in the corner swaps
 * to a CodeMirror source editor. Edits flow through `onChange` to the
 * store on every keystroke.
 */
export function MarkdownCell({ source, onChange }: MarkdownCellProps) {
  const styles = useStyles();
  const [editing, setEditing] = useState(false);

  return (
    <div className={styles.root}>
      <Button
        size="small"
        appearance="subtle"
        className={styles.toggle}
        icon={editing ? <Eye16Regular /> : <Edit16Regular />}
        title={editing ? "Render" : "Edit source"}
        onClick={() => setEditing((v) => !v)}
      />
      {editing
        ? (
          <CodeMirrorEditor
            source={source}
            cellType="markdown"
            readOnly={false}
            onChange={onChange}
          />
        )
        : (
          <div className="markdown-cell">
            <ReactMarkdown
              remarkPlugins={[remarkGfm, remarkMath]}
              rehypePlugins={[rehypeKatex]}
            >
              {source}
            </ReactMarkdown>
          </div>
        )}
    </div>
  );
}
