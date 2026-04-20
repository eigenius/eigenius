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
  RegisterWasmComponentRequest,
} from "../gen/eigenius_pb.ts";
import { ComponentRegistry } from "../components/registry.ts";
import type { WasmComponentRegistry } from "../wasm/registry.ts";
import {
  createWasmComponentHandler,
  type HostBridge,
} from "../wasm/hostBridge.ts";
import type { WasmAddon } from "../wasm/loadAddon.ts";

const TEXT_DECODER = new TextDecoder();

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
 * Register the ComponentExecutor service implementation on a Connect router.
 */
export function registerComponentExecutor(
  router: ConnectRouter,
  deps: ComponentExecutorDeps,
): void {
  const { registry, wasm } = deps;

  router.service(ComponentExecutor, {
    async execute(req: ComponentRequest) {
      const componentIri = req.componentIri;

      if (!registry.has(componentIri)) {
        return create(ComponentResponseSchema, {
          success: false,
          error: `No handler registered for component: ${componentIri}`,
        });
      }

      try {
        // Decode input and argument from bytes (Eigon-JSON)
        const inputJson = TEXT_DECODER.decode(req.input);
        const argumentJson = TEXT_DECODER.decode(req.argument);

        const input = inputJson ? JSON.parse(inputJson) : {};
        const argument = argumentJson ? JSON.parse(argumentJson) : {};

        const result = await registry.execute(componentIri, {
          input,
          argument,
        });

        // Encode output back to Eigon-JSON bytes
        const outputBytes = new TextEncoder().encode(
          JSON.stringify(result.output),
        );

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
        return create(ComponentResponseSchema, {
          success: false,
          error: `Component execution failed: ${(e as Error).message}`,
        });
      }
    },

    async registerWasmComponent(req: RegisterWasmComponentRequest) {
      if (!wasm) {
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
        return create(RegisterWasmComponentResponseSchema, {
          success: false,
          error: "component_iri is required",
        });
      }
      if (!req.wasmBinary || req.wasmBinary.length === 0) {
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

        console.log(`[wasm] registered ${componentIri}`);
        return create(RegisterWasmComponentResponseSchema, { success: true });
      } catch (e) {
        return create(RegisterWasmComponentResponseSchema, {
          success: false,
          error: `WASM registration failed: ${(e as Error).message}`,
        });
      }
    },
  });
}
