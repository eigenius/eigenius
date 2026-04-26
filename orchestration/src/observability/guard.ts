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
 * Per-RPC observability guard for the orchestrator. Mirror of
 * `kernel/src/observability/guard.rs`'s `RpcGuard`.
 *
 * Wrap an async handler body with `withRpcGuard` so the entry / exit
 * events fire automatically for every return path including thrown
 * errors:
 *
 * ```ts
 * await withRpcGuard(operation.COMPONENT_DISPATCH, async (mark) => {
 *   try {
 *     return await dispatch(...);
 *   } catch (e) {
 *     mark.fail("dispatch_failed");
 *     throw e;
 *   }
 * });
 * ```
 *
 * The guard emits `request received` at debug on entry and a
 * completion event at debug on success (with `latency_ms`), or at
 * `warn` if the handler called `mark.fail("kind")`.
 */

import { debug, warn } from "./log.ts";

export interface FailMark {
  /** Mark the request as failed with the given stable error kind. */
  fail(kind: string): void;
}

export async function withRpcGuard<T>(
  operation: string,
  body: (mark: FailMark) => Promise<T>,
): Promise<T> {
  const started = performance.now();
  let errorKind: string | undefined;
  const mark: FailMark = {
    fail(kind: string) {
      errorKind = kind;
    },
  };

  debug(operation, "request received");

  try {
    const result = await body(mark);
    return result;
  } catch (e) {
    if (errorKind === undefined) {
      // Unhandled throw — record it as a generic failure so the
      // completion event still goes out at warn level.
      errorKind = "unhandled_exception";
    }
    throw e;
  } finally {
    const latencyMs = Math.round(performance.now() - started);
    if (errorKind !== undefined) {
      warn(operation, "request failed", {
        latency_ms: latencyMs,
        error_kind: errorKind,
      });
    } else {
      debug(operation, "request completed", { latency_ms: latencyMs });
    }
  }
}
