/**
 * Eigenius Orchestration Layer
 *
 * The Deno/TypeScript orchestration layer sits above the kernel service API
 * and handles program execution, LLM adapter management, and the MCP server surface.
 *
 * Architecture reference: §2.2 (Host Layer)
 */

import { KernelClient } from "./client/kernel_client.ts";
import { ProgramExecutor } from "./program/executor.ts";

const KERNEL_ENDPOINT = Deno.env.get("EIGENIUS_KERNEL_ENDPOINT") ??
  "http://localhost:50051";

function main() {
  console.log(`Eigenius Orchestration Layer starting...`);
  console.log(`Kernel endpoint: ${KERNEL_ENDPOINT}`);

  const _client = new KernelClient(KERNEL_ENDPOINT);
  const _executor = new ProgramExecutor(_client);

  // TODO: Start HTTP API gateway
  // TODO: Start MCP server
  // TODO: Health check endpoint

  console.log("Orchestration layer ready.");
}

main();
