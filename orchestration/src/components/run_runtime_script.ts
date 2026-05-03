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
 * RunRuntimeScript Component Handler
 *
 * Implements the urn:eigenius:program:components:RunRuntimeScript
 * component by bridging into the runtime substrate via the napi addon
 * at orchestration/runtime-substrate-native/. Phase 18a / D26 §4.1.
 *
 * Codec: Eigon-CBOR end-to-end. The JS objects this handler receives
 * (already decoded from the kernel's Eigon-CBOR ComponentRequest by
 * component_executor.ts) are re-encoded to CBOR via wasm/cbor.ts and
 * passed to the addon as Buffers. The addon decodes back to a Resource
 * and dispatches through SubstrateDispatcher → LanguageRuntime →
 * worker. The output is symmetric: addon returns Eigon-CBOR bytes; we
 * decode to a JS object before returning to the orchestrator.
 */

import { decodeResource, encodeResource } from "../wasm/cbor.ts";
import type { RuntimeSubstrateAddon } from "../runtime/loadAddon.ts";
import * as log from "../observability/mod.ts";
import { operation } from "../observability/mod.ts";
import type {
  ComponentHandler,
  ComponentInput,
  ComponentOutput,
} from "./registry.ts";

/** The component IRI for RunRuntimeScript. */
export const RUN_RUNTIME_SCRIPT_IRI =
  "urn:eigenius:program:components:RunRuntimeScript";

/**
 * Build the handler bound to a specific addon instance. The addon must
 * have at least one `LanguageRuntime` registered (for dev/CI that's
 * the bash-c `TestLanguageRuntime`); dispatches against unregistered
 * languages return a typed `UnknownLanguage` error from the substrate.
 */
export function createRunRuntimeScriptHandler(
  addon: RuntimeSubstrateAddon,
): ComponentHandler {
  return async (req: ComponentInput): Promise<ComponentOutput> => {
    const startTime = Date.now();
    const inputCbor = encodeResource(req.input);
    const argumentCbor = encodeResource(req.argument);

    log.debug(operation.COMPONENT_DISPATCH, "RunRuntimeScript dispatching", {
      input_bytes: inputCbor.byteLength,
      argument_bytes: argumentCbor.byteLength,
    });

    let outputBytes: Uint8Array;
    try {
      outputBytes = await addon.dispatchRunRuntimeScript(
        inputCbor,
        argumentCbor,
      );
    } catch (e) {
      log.warn(
        operation.COMPONENT_DISPATCH,
        "RunRuntimeScript substrate dispatch failed",
        {
          error_kind: "substrate_dispatch_failed",
          error_message: e instanceof Error ? e.message : String(e),
          latency_ms: Date.now() - startTime,
        },
      );
      throw e;
    }

    const latencyMs = Date.now() - startTime;
    log.info(operation.COMPONENT_DISPATCH, "RunRuntimeScript completed", {
      output_bytes: outputBytes.byteLength,
      latency_ms: latencyMs,
    });

    return { output: decodeResource(outputBytes) };
  };
}
