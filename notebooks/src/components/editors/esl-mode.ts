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
 * CodeMirror 6 language support for ESL — lexical-only highlighting.
 *
 * Recognises: top-level / expression keywords, qualified names (`ns:local`),
 * lambda forms (`\` and `λ`), strings, numbers, booleans, line/block
 * comments, size-bound binders (`{j < i}`).
 *
 * Aligned with the keyword table in the ESL guide §3.2 and the
 * authoritative spec in D7 §2.
 */

import { StreamLanguage } from "@codemirror/language";

const TOP_LEVEL_KEYWORDS = new Set([
  "namespace",
  "class",
  "property",
  "resource",
  "program",
  "data",
  "codata",
]);

const EXPRESSION_KEYWORDS = new Set([
  "let",
  "case",
  "match",
  "returning",
  "Construct",
  "map",
  "reduce",
  "corecord",
]);

const CLASS_BODY_KEYWORDS = new Set([
  "description",
  "requires",
  "recommends",
  "min_value",
  "max_value",
  "min_length",
  "max_length",
  "pattern",
  "format",
  "allows_only",
  "domain",
  "class_types",
  "element_type",
]);

const LITERAL_KEYWORDS = new Set(["true", "false"]);

export const eslLanguage = StreamLanguage.define<{ inComment: boolean }>({
  name: "esl",
  startState: () => ({ inComment: false }),
  token(stream, state) {
    if (state.inComment) {
      while (!stream.eol()) {
        if (stream.match("*/")) {
          state.inComment = false;
          return "comment";
        }
        stream.next();
      }
      return "comment";
    }

    if (stream.eatSpace()) return null;

    // Block comment
    if (stream.match("/*")) {
      state.inComment = true;
      return "comment";
    }
    // Line comment
    if (stream.match("//")) {
      stream.skipToEnd();
      return "comment";
    }

    // String literal
    if (stream.match('"')) {
      while (!stream.eol()) {
        const ch = stream.next();
        if (ch === "\\") {
          stream.next();
        } else if (ch === '"') {
          return "string";
        }
      }
      return "string";
    }

    // Lambda forms — `\` and `λ` (U+03BB)
    if (stream.match(/^[\\λ]/)) {
      return "keyword";
    }

    // Number
    if (stream.match(/^-?\d+(\.\d+)?([eE][+-]?\d+)?/)) {
      return "number";
    }

    // Arrow / size-bound operators
    if (stream.match(/^(->|<)/)) {
      return "operator";
    }

    // Punctuation
    if (stream.match(/^[(){}\[\]:;,.=]/)) {
      return "punctuation";
    }

    // Identifier — also handles qualified names (we tokenise `ns:local`
    // as Ident Colon Ident, matching the ESL parser; the colon already
    // got "punctuation").
    const id = stream.match(/^[a-zA-Z_][a-zA-Z0-9_]*/);
    if (id) {
      const word = (id as RegExpMatchArray)[0];
      if (TOP_LEVEL_KEYWORDS.has(word)) return "keyword";
      if (EXPRESSION_KEYWORDS.has(word)) return "keyword";
      if (CLASS_BODY_KEYWORDS.has(word)) return "propertyName";
      if (LITERAL_KEYWORDS.has(word)) return "atom";
      return "name";
    }

    stream.next();
    return null;
  },
});
