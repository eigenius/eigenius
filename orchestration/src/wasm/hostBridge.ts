/**
 * Host-bridge callbacks for the IO WASM world.
 *
 * The napi-rs addon invokes these three callbacks whenever a guest calls
 * the corresponding host import:
 *
 *   dispatch(iri, inputCbor, argumentCbor)  ← io-access.dispatch-component
 *   resolve(iri)                            ← read-access.resolve
 *   query(eigenql)                          ← query-access.query
 *
 * Dispatch routes by IRI:
 *   • Another WASM component → executeComponent again (CBOR pass-through).
 *   • Native TS handler      → CBOR decode → call handler → CBOR encode.
 *
 * Resolve/query route through the KernelClient when supplied; otherwise
 * they return safe defaults (null / []). Full kernel wiring lands in M3b.
 */

import type {
  ComponentHandler,
  ComponentRegistry,
} from "../components/registry.ts";
import type { KernelClient } from "../client/kernel_client.ts";
import type { WasmAddon } from "./loadAddon.ts";
import type { WasmComponentRegistry } from "./registry.ts";
import { decodeResource, encodeResource } from "./cbor.ts";

export interface HostBridgeDeps {
  addon: WasmAddon;
  registry: ComponentRegistry;
  wasmRegistry: WasmComponentRegistry;
  /** Optional kernel client for `resolve` / `query` (M3b wires this). */
  kernel?: KernelClient;
}

export interface HostBridge {
  dispatch: (
    iri: string,
    input: Uint8Array,
    argument: Uint8Array,
  ) => Promise<Uint8Array>;
  resolve: (iri: string) => Promise<Uint8Array | null>;
  query: (eigenql: string) => Promise<Uint8Array[]>;
}

export function createHostBridge(deps: HostBridgeDeps): HostBridge {
  const { addon, registry, wasmRegistry, kernel } = deps;

  // Self-reference so dispatch can recursively invoke WASM components.
  // eslint-disable-next-line prefer-const
  const bridge: HostBridge = {
    async dispatch(iri, input, argument) {
      // Prefer a WASM handler if one is registered — pass CBOR through
      // unchanged.
      if (wasmRegistry.has(iri)) {
        const handle = wasmRegistry.getHandle(iri)!;
        return await addon.executeComponent(
          handle,
          input,
          argument,
          bridge.dispatch,
          bridge.resolve,
          bridge.query,
        );
      }

      // Fall back to a native TS handler. Convert CBOR ↔ plain JS at the
      // boundary — TS handlers work with Eigon-JSON-shaped objects.
      if (registry.has(iri)) {
        const inputObj = decodeResource(input);
        const argObj = decodeResource(argument);
        const result = await registry.execute(iri, {
          input: inputObj,
          argument: argObj,
        });
        return encodeResource(result.output);
      }

      throw new Error(`dispatch: no handler registered for component: ${iri}`);
    },

    async resolve(iri) {
      if (!kernel) return null;
      const response = await kernel.inspect(iri);
      if (!response.found) return null;
      return response.resource;
    },

    async query(eigenql) {
      if (!kernel) return [];
      return await kernel.query(eigenql);
    },
  };

  return bridge;
}

/**
 * Build a ComponentHandler that forwards execute calls to a registered WASM
 * component. Intended to be plugged into the regular `ComponentRegistry` so
 * the kernel's gRPC Execute path reaches the WASM backend unchanged.
 *
 * The handler is responsible for CBOR ↔ JS conversion at the boundary —
 * the kernel speaks Eigon-JSON on this path (see kernel/src/program/remote.rs),
 * the WASM side speaks CBOR.
 */
export function createWasmComponentHandler(
  componentIri: string,
  deps: {
    addon: WasmAddon;
    wasmRegistry: WasmComponentRegistry;
    bridge: HostBridge;
  },
): ComponentHandler {
  const { addon, wasmRegistry, bridge } = deps;

  return async ({ input, argument }) => {
    const handle = wasmRegistry.getHandle(componentIri);
    if (handle === undefined) {
      throw new Error(
        `WASM component handle missing for ${componentIri} — registry out of sync`,
      );
    }

    const inputBytes = encodeResource(input);
    const argumentBytes = encodeResource(argument);

    const outputBytes = await addon.executeComponent(
      handle,
      inputBytes,
      argumentBytes,
      bridge.dispatch,
      bridge.resolve,
      bridge.query,
    );

    return { output: decodeResource(outputBytes) };
  };
}
