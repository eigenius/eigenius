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
 * CodeMirror 6 language support for EigenQL — lexical-only highlighting.
 * Recognises keywords, built-in functions, qualified names (`ns:local`),
 * `?variable`, IRI strings, numbers, booleans, line/block comments.
 *
 * Aligned with the keyword table in the EigenQL guide §3.2 and the
 * authoritative spec in D2 §2.2.
 */

import { StreamLanguage } from "@codemirror/language";

const STRUCTURAL_KEYWORDS = new Set([
  "USING",
  "INSTITUTION",
  "AS",
  "DEFINE",
  "FROM",
  "MATCH",
  "FIBER",
  "WHERE",
  "RETURN",
  "GROUP",
  "BY",
  "ORDER",
  "ASC",
  "DESC",
  "DISTINCT",
  "LIMIT",
  "OFFSET",
]);

const OPERATOR_KEYWORDS = new Set([
  "AND",
  "OR",
  "NOT",
  "IN",
  "LIKE",
  "EXISTS",
]);

const BUILTIN_FUNCTIONS = new Set([
  "DATE",
  "TIMESTAMP",
  "REGEX",
  "LENGTH",
  "CONTAINS",
  "CONCAT",
  "COUNT",
  "SUM",
  "AVG",
  "MIN",
  "MAX",
]);

const LITERAL_KEYWORDS = new Set(["true", "false"]);

export const eigenqlLanguage = StreamLanguage.define<{ inComment: boolean }>({
  name: "eigenql",
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

    // String / IRI literal
    if (stream.match('"')) {
      while (!stream.eol()) {
        const ch = stream.next();
        if (ch === "\\") {
          stream.next(); // skip escaped char
        } else if (ch === '"') {
          return "string";
        }
      }
      return "string";
    }

    // Variable: ?name
    if (stream.match(/^\?[a-zA-Z_][a-zA-Z0-9_]*/)) {
      return "variableName";
    }

    // Number
    if (stream.match(/^-?\d+(\.\d+)?([eE][+-]?\d+)?/)) {
      return "number";
    }

    // Operators
    if (stream.match(/^(\*\*|<=|>=|<>|\|\||->|[=<>+\-*/%])/)) {
      return "operator";
    }

    // Punctuation
    if (stream.match(/^[(){}\[\],.;:]/)) {
      return "punctuation";
    }

    // Identifier (possibly qualified)
    const id = stream.match(/^[a-zA-Z_][a-zA-Z0-9_]*/);
    if (id) {
      const word = (id as RegExpMatchArray)[0];
      if (STRUCTURAL_KEYWORDS.has(word)) return "keyword";
      if (OPERATOR_KEYWORDS.has(word)) return "operatorKeyword";
      if (BUILTIN_FUNCTIONS.has(word)) return "function";
      if (LITERAL_KEYWORDS.has(word)) return "atom";
      return "name";
    }

    stream.next();
    return null;
  },
});
