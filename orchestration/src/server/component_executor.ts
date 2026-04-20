/**
 * ComponentExecutor gRPC service implementation.
 *
 * Receives component dispatch calls from the kernel and routes them
 * to the local ComponentRegistry. This is the reverse direction:
 * kernel → orchestrator.
 *
 * Architecture reference: D6 (execution architecture)
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

const TEXT_DECODER = new TextDecoder();

/**
 * Register the ComponentExecutor service implementation on a Connect router.
 */
export function registerComponentExecutor(
  router: ConnectRouter,
  registry: ComponentRegistry,
): void {
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

    // deno-lint-ignore require-await
    async registerWasmComponent(_req: RegisterWasmComponentRequest) {
      // Not yet implemented. The orchestrator will host IO WASM components
      // via a napi-rs + wasmtime addon (see Phase 8 plan). Until that's in
      // place, IO WASM installs are rejected with a clear error.
      return create(RegisterWasmComponentResponseSchema, {
        success: false,
        error:
          "orchestrator-side WASM hosting is not yet implemented (Phase 8, pending napi-rs integration)",
      });
    },
  });
}
