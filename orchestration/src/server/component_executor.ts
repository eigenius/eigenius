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
 * ComponentExecutor gRPC service implementation.
 *
 * Receives component dispatch calls from the kernel and routes them
 * to the local ComponentRegistry. This is the reverse direction:
 * kernel → orchestrator.
 *
 * Also handles `RegisterWasmComponent` for IO WASM components: compiles
 * the Component Model binary via the napi-rs addon and plugs the
 * resulting handle into `ComponentRegistry` as a regular handler.
 *
 * Architecture reference: D6 (execution architecture), D12 (WASM ext),
 * D12b (orchestrator-side WASM plan).
 */

import type { ConnectRouter } from "@connectrpc/connect";
import { create } from "@bufbuild/protobuf";
import {
  ComponentExecutor,
  ComponentMetricsSchema,
  ComponentResponseSchema,
  RegisterWasmComponentResponseSchema,
} from "../gen/eigenius_pb.ts";
import type {
  ComponentRequest,
  ComponentResponse,
  RegisterWasmComponentRequest,
} from "../gen/eigenius_pb.ts";
import { ComponentRegistry } from "../components/registry.ts";
import type { WasmComponentRegistry } from "../wasm/registry.ts";
import {
  createWasmComponentHandler,
  type HostBridge,
} from "../wasm/hostBridge.ts";
import type { WasmAddon } from "../wasm/loadAddon.ts";
import { decodeResource, encodeResource } from "../wasm/cbor.ts";
import * as log from "../observability/mod.ts";
import {
  type FailMark,
  operation,
  withRpcGuard,
} from "../observability/mod.ts";

const TEXT_DECODER = new TextDecoder();
const TEXT_ENCODER = new TextEncoder();

const CONTENT_TYPE_CBOR = "application/eigon+cbor";
// JSON-fallback branch is keyed on `!== CONTENT_TYPE_CBOR` (anything
// not CBOR — including the literal `application/eigon+json` and
// pre-18e clients that send empty content_type — falls through to
// JSON). No constant needed for the JSON tag.

export interface ComponentExecutorDeps {
  registry: ComponentRegistry;
  /** Optional WASM support bundle. Absent when the native addon failed to load. */
  wasm?: {
    addon: WasmAddon;
    wasmRegistry: WasmComponentRegistry;
    bridge: HostBridge;
  };
}

/**
 * Per-request dispatcher for the ComponentExecutor service. Extracted
 * from `registerComponentExecutor` so the codec-branching logic
 * (Phase 18e: CBOR by default, JSON for backward compat) is unit-
 * testable without a real Connect server.
 *
 * The CBOR / JSON branch is symmetric: the response is encoded in the
 * same codec the request used, so a pre-18e kernel that sends JSON
 * gets JSON back during a rolling upgrade.
 */
export async function executeComponentRequest(
  req: ComponentRequest,
  registry: ComponentRegistry,
  mark: FailMark,
): Promise<ComponentResponse> {
  const componentIri = req.componentIri;

  if (!registry.has(componentIri)) {
    mark.fail("unknown_component");
    return create(ComponentResponseSchema, {
      success: false,
      error: `No handler registered for component: ${componentIri}`,
    });
  }

  try {
    // Branch on content_type per the proto field. Phase 18e:
    // kernels send Eigon-CBOR by default; the JSON path stays
    // for backward compat (mismatched kernel/orchestrator
    // versions during a rolling deploy). Empty content_type is
    // treated as JSON since pre-18e clients didn't set it.
    const useCbor = req.contentType === CONTENT_TYPE_CBOR;

    let input: Record<string, unknown>;
    let argument: Record<string, unknown>;
    if (useCbor) {
      input = decodeResource(req.input) as Record<string, unknown>;
      argument = decodeResource(req.argument) as Record<string, unknown>;
    } else {
      const inputJson = TEXT_DECODER.decode(req.input);
      const argumentJson = TEXT_DECODER.decode(req.argument);
      input = inputJson ? JSON.parse(inputJson) : {};
      argument = argumentJson ? JSON.parse(argumentJson) : {};
    }

    const result = await registry.execute(componentIri, {
      input,
      argument,
    });

    // Encode output in the same codec the request used so a
    // pre-18e kernel still gets JSON back during a rolling
    // upgrade.
    const outputBytes = useCbor
      ? encodeResource(result.output)
      : TEXT_ENCODER.encode(JSON.stringify(result.output));

    const response = create(ComponentResponseSchema, {
      success: true,
      output: outputBytes,
    });

    if (result.metrics) {
      response.metrics = create(ComponentMetricsSchema, {
        provider: result.metrics.provider,
        model: result.metrics.model,
        promptTokens: BigInt(result.metrics.promptTokens),
        completionTokens: BigInt(result.metrics.completionTokens),
        latencyMs: BigInt(result.metrics.latencyMs),
      });
    }

    return response;
  } catch (e) {
    mark.fail("dispatch_failed");
    return create(ComponentResponseSchema, {
      success: false,
      error: `Component execution failed: ${(e as Error).message}`,
    });
  }
}

/**
 * Register the ComponentExecutor service implementation on a Connect router.
 */
export function registerComponentExecutor(
  router: ConnectRouter,
  deps: ComponentExecutorDeps,
): void {
  const { registry, wasm } = deps;

  router.service(ComponentExecutor, {
    execute(req: ComponentRequest) {
      return withRpcGuard(operation.COMPONENT_DISPATCH, (mark) =>
        executeComponentRequest(req, registry, mark));
    },

    registerWasmComponent(req: RegisterWasmComponentRequest) {
      return withRpcGuard(operation.WASM_COMPONENT_REGISTER, async (mark) => {
        if (!wasm) {
          mark.fail("wasm_disabled");
          return create(RegisterWasmComponentResponseSchema, {
            success: false,
            error:
              "orchestrator WASM support is disabled (native addon not loaded — " +
              "build orchestration/native)",
          });
        }

        const { addon, wasmRegistry, bridge } = wasm;
        const componentIri = req.componentIri;

        if (!componentIri) {
          mark.fail("missing_component_iri");
          return create(RegisterWasmComponentResponseSchema, {
            success: false,
            error: "component_iri is required",
          });
        }
        if (!req.wasmBinary || req.wasmBinary.length === 0) {
          mark.fail("missing_wasm_binary");
          return create(RegisterWasmComponentResponseSchema, {
            success: false,
            error: "wasm_binary is required",
          });
        }

        try {
          await wasmRegistry.register(componentIri, req.wasmBinary, {
            fuelLimit: Number(req.fuelLimit ?? 0n),
            memoryLimitPages: Number(req.memoryLimitPages ?? 0n),
          });

          registry.register(
            componentIri,
            createWasmComponentHandler(componentIri, {
              addon,
              wasmRegistry,
              bridge,
            }),
          );

          log.info(
            operation.WASM_COMPONENT_REGISTER,
            "registered WASM IO component",
            {
              component_iri: componentIri,
              size_bytes: req.wasmBinary.length,
            },
          );
          return create(RegisterWasmComponentResponseSchema, {
            success: true,
          });
        } catch (e) {
          mark.fail("registration_failed");
          return create(RegisterWasmComponentResponseSchema, {
            success: false,
            error: `WASM registration failed: ${(e as Error).message}`,
          });
        }
      });
    },
  });
}
