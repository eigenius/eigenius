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
 * Structured logging primitives for the orchestrator.
 *
 * Mirrors the kernel's `tracing`-based shape so a single log query
 * works against both services. Call sites take three positional
 * arguments — `operation`, `message`, `fields` — exactly matching
 * the kernel convention.
 *
 *   import * as log from "./observability/log.ts";
 *   import * as op from "./observability/operation.ts";
 *
 *   log.info(op.COMPONENT_REGISTER, "registered component", {
 *     component_iri: iri,
 *     host: "orchestrator",
 *   });
 *
 * JSON output matches the kernel record shape:
 *
 *   {"timestamp":"…","level":"INFO","fields":{"message":"…","operation":"…",…},"target":"orchestration"}
 */

import { OPERATION } from "./field.ts";

export type LogLevel = "debug" | "info" | "warn" | "error";

const LEVEL_ORDER: Record<LogLevel, number> = {
  debug: 0,
  info: 1,
  warn: 2,
  error: 3,
};

const LEVEL_LABEL: Record<LogLevel, string> = {
  debug: "DEBUG",
  info: "INFO",
  warn: "WARN",
  error: "ERROR",
};

let currentLevel: LogLevel = "info";
let currentFormat: "json" | "pretty" = "pretty";

/** Set the minimum level to emit. Calls below this level are dropped. */
export function setLevel(level: LogLevel): void {
  currentLevel = level;
}

/** Set the output format. JSON for log aggregators; pretty for local dev. */
export function setFormat(format: "json" | "pretty"): void {
  currentFormat = format;
}

/**
 * Emit at the given level. Prefer the typed wrappers (`debug`,
 * `info`, `warn`, `error`) at call sites — this is the underlying
 * machinery they share.
 */
export function emit(
  level: LogLevel,
  operation: string,
  message: string,
  fields: Record<string, unknown> = {},
): void {
  if (LEVEL_ORDER[level] < LEVEL_ORDER[currentLevel]) return;

  const timestamp = new Date().toISOString();
  if (currentFormat === "json") {
    // Match the kernel's `tracing-subscriber` JSON shape: timestamp,
    // level, target, and a `fields` object containing the message
    // alongside the call-site fields. Cleaner than the alternative
    // of nesting fields under a separate key.
    const record = {
      timestamp,
      level: LEVEL_LABEL[level],
      target: "orchestration",
      fields: {
        message,
        [OPERATION]: operation,
        ...fields,
      },
    };
    console.log(JSON.stringify(record));
    return;
  }

  // Pretty mode: one line per record, fields appended as key=value.
  const fieldStr = Object.entries(fields)
    .map(([k, v]) => `${k}=${formatValue(v)}`)
    .join(" ");
  const head = `${timestamp} ${LEVEL_LABEL[level].padEnd(5)} ${operation} `;
  const tail = fieldStr.length > 0 ? `${message} ${fieldStr}` : message;
  console.log(head + tail);
}

function formatValue(v: unknown): string {
  if (typeof v === "string") {
    // Quote if the value contains spaces or special chars; otherwise
    // leave bare for easier eyeballing.
    return /[\s"=]/.test(v) ? JSON.stringify(v) : v;
  }
  if (v === null || v === undefined) return String(v);
  if (typeof v === "object") return JSON.stringify(v);
  return String(v);
}

export function debug(
  operation: string,
  message: string,
  fields: Record<string, unknown> = {},
): void {
  emit("debug", operation, message, fields);
}

export function info(
  operation: string,
  message: string,
  fields: Record<string, unknown> = {},
): void {
  emit("info", operation, message, fields);
}

export function warn(
  operation: string,
  message: string,
  fields: Record<string, unknown> = {},
): void {
  emit("warn", operation, message, fields);
}

export function error(
  operation: string,
  message: string,
  fields: Record<string, unknown> = {},
): void {
  emit("error", operation, message, fields);
}
