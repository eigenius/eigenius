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
 * Subscriber init for the orchestrator. Reads two env vars:
 *
 * - `EIGENIUS_LOG_LEVEL` — one of `debug` / `info` / `warn` / `error`.
 *   Defaults to `info`.
 * - `EIGENIUS_LOG_FORMAT` — `json` writes one-line JSON suitable for
 *   log aggregators; `pretty` writes a human-readable single-line
 *   format. If unset, picks `pretty` when stdout is a TTY and `json`
 *   otherwise — same default as the kernel.
 *
 * The kernel uses `RUST_LOG` for filter (via `tracing-subscriber`'s
 * `EnvFilter`); the orchestrator uses a single global level since we
 * don't have per-module filtering needs here.
 */

import { type LogLevel, setFormat, setLevel } from "./log.ts";

export function init(): void {
  const levelRaw = (Deno.env.get("EIGENIUS_LOG_LEVEL") ?? "info")
    .toLowerCase();
  const level: LogLevel = isLogLevel(levelRaw) ? levelRaw : "info";
  setLevel(level);

  const formatEnv = Deno.env.get("EIGENIUS_LOG_FORMAT");
  if (formatEnv === "json" || formatEnv === "pretty") {
    setFormat(formatEnv);
  } else if (Deno.stdout.isTerminal()) {
    setFormat("pretty");
  } else {
    setFormat("json");
  }
}

function isLogLevel(s: string): s is LogLevel {
  return s === "debug" || s === "info" || s === "warn" || s === "error";
}
