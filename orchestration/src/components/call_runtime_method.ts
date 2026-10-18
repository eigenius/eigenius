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
 * CallRuntimeMethod Component Handler
 *
 * Implements the urn:eigenius:program:components:CallRuntimeMethod
 * component by bridging into the runtime substrate via the napi addon
 * (D26 §4.1). Same codec pattern as RunRuntimeScript: Eigon-CBOR
 * Buffers in / out, JS objects on the handler boundary.
 *
 * **Phase 18a behaviour.** `CallRuntimeMethod` is a Service-lifecycle
 * surface — typed library calls dispatched against a long-lived
 * `RuntimeEnvironment` worker pool (D26 §5.3.1). Today's substrate
 * ships only the Job-side machinery; the service-backed dispatcher
 * lands in Phase 19a alongside Julia (D27). Until then this handler
 * surfaces the substrate's `MethodSignatureMismatch` error verbatim,
 * which carries the "service lifecycle not yet implemented" guidance
 * in its message.
 */

import { decodeResource, encodeResource } from "../codec/cbor.ts";
import type { RuntimeSubstrateAddon } from "../runtime/loadAddon.ts";
import * as log from "../observability/mod.ts";
import { operation } from "../observability/mod.ts";
import type {
  ComponentHandler,
  ComponentInput,
  ComponentOutput,
} from "./registry.ts";

/** The component IRI for CallRuntimeMethod. */
export const CALL_RUNTIME_METHOD_IRI =
  "urn:eigenius:program:components:CallRuntimeMethod";

export function createCallRuntimeMethodHandler(
  addon: RuntimeSubstrateAddon,
): ComponentHandler {
  return async (req: ComponentInput): Promise<ComponentOutput> => {
    const startTime = Date.now();
    const inputCbor = encodeResource(req.input);
    const argumentCbor = encodeResource(req.argument);

    log.debug(operation.COMPONENT_DISPATCH, "CallRuntimeMethod dispatching", {
      input_bytes: inputCbor.byteLength,
      argument_bytes: argumentCbor.byteLength,
    });

    let outcome: { output: Uint8Array; partialInvocation: Uint8Array };
    try {
      outcome = await addon.dispatchCallRuntimeMethod(inputCbor, argumentCbor);
    } catch (e) {
      log.warn(
        operation.COMPONENT_DISPATCH,
        "CallRuntimeMethod substrate dispatch failed",
        {
          error_kind: "substrate_dispatch_failed",
          error_message: e instanceof Error ? e.message : String(e),
          latency_ms: Date.now() - startTime,
        },
      );
      throw e;
    }

    const latencyMs = Date.now() - startTime;
    log.info(operation.COMPONENT_DISPATCH, "CallRuntimeMethod completed", {
      output_bytes: outcome.output.byteLength,
      partial_invocation_bytes: outcome.partialInvocation.byteLength,
      latency_ms: latencyMs,
    });

    return { output: decodeResource(outcome.output) };
  };
}
