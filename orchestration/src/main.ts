/**
 * Eigenius Orchestration Layer
 *
 * The Deno/TypeScript orchestration layer sits above the kernel service API
 * and handles IO component execution, LLM adapter management, and the MCP
 * server surface.
 *
 * Architecture reference: §2.2 (Host Layer)
 */

import { KernelClient } from "./client/kernel_client.ts";
import { ComponentRegistry } from "./components/registry.ts";
import {
  COMPLETE_TEXT_IRI,
  createCompleteTextHandler,
  createMockCompleteTextHandler,
} from "./components/complete_text.ts";
import {
  COMPLETE_JSON_IRI,
  createCompleteJsonHandler,
  createMockCompleteJsonHandler,
} from "./components/complete_json.ts";
import { ProgramExecutor } from "./program/executor.ts";
import { startServer } from "./server/mod.ts";
import { tryLoadWasmAddon } from "./wasm/loadAddon.ts";
import { WasmComponentRegistry } from "./wasm/registry.ts";
import { createHostBridge } from "./wasm/hostBridge.ts";

const KERNEL_ENDPOINT = Deno.env.get("EIGENIUS_KERNEL_ENDPOINT") ??
  "http://localhost:50051";
const ORCHESTRATOR_PORT = parseInt(
  Deno.env.get("EIGENIUS_ORCHESTRATOR_PORT") ?? "8080",
);
const USE_MOCK_LLM = Deno.env.get("EIGENIUS_MOCK_LLM") === "true";

function main() {
  console.log(`Eigenius Orchestration Layer starting...`);
  console.log(`Kernel endpoint: ${KERNEL_ENDPOINT}`);

  const client = new KernelClient(KERNEL_ENDPOINT);
  const components = new ComponentRegistry();

  // Register LLM components
  if (USE_MOCK_LLM) {
    console.log("Using mock LLM handlers (EIGENIUS_MOCK_LLM=true)");
    components.register(COMPLETE_TEXT_IRI, createMockCompleteTextHandler());
    components.register(COMPLETE_JSON_IRI, createMockCompleteJsonHandler());
  } else {
    components.register(COMPLETE_TEXT_IRI, createCompleteTextHandler());
    components.register(COMPLETE_JSON_IRI, createCompleteJsonHandler());
  }

  const _executor = new ProgramExecutor(client, components);

  // Native addon for IO WASM components (optional — skipped if not built).
  const addon = tryLoadWasmAddon();
  const wasm = addon
    ? (() => {
      const wasmRegistry = new WasmComponentRegistry(addon);
      const bridge = createHostBridge({
        addon,
        registry: components,
        wasmRegistry,
        kernel: client,
      });
      console.log("WASM IO components: enabled (native addon loaded)");
      return { addon, wasmRegistry, bridge };
    })()
    : undefined;
  if (!wasm) {
    console.log(
      "WASM IO components: disabled (addon not loaded — RegisterWasmComponent will fail)",
    );
  }

  console.log(
    `Registered components: ${components.listComponents().join(", ")}`,
  );

  // Start the orchestrator server (gRPC + health)
  startServer(components, ORCHESTRATOR_PORT, wasm);
}

main();
