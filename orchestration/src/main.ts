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
import { ProgramExecutor } from "./program/executor.ts";
import { startServer } from "./server/mod.ts";

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

  // Register CompleteText component
  if (USE_MOCK_LLM) {
    console.log("Using mock LLM handler (EIGENIUS_MOCK_LLM=true)");
    components.register(COMPLETE_TEXT_IRI, createMockCompleteTextHandler());
  } else {
    components.register(COMPLETE_TEXT_IRI, createCompleteTextHandler());
  }

  const _executor = new ProgramExecutor(client, components);

  console.log(
    `Registered components: ${components.listComponents().join(", ")}`,
  );

  // Start the orchestrator server (gRPC + health)
  startServer(components, ORCHESTRATOR_PORT);
}

main();
