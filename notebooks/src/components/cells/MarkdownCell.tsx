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

import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

export interface MarkdownCellProps {
  source: string;
}

/**
 * Markdown cell — Phase 2 read-only rendering.
 *
 * In Phase 4 this gains an "edit" toggle (CodeMirror-backed editing of
 * the raw markdown source) and a "preview" view. For now we just
 * render the prose.
 */
export function MarkdownCell({ source }: MarkdownCellProps) {
  return (
    <div className="markdown-cell">
      <ReactMarkdown remarkPlugins={[remarkGfm]}>{source}</ReactMarkdown>
    </div>
  );
}
