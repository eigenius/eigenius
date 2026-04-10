/**
 * Eigenius Orchestration Layer
 *
 * The Deno/TypeScript orchestration layer sits above the kernel service API
 * and handles DAG execution, LLM adapter management, and the MCP server surface.
 *
 * Architecture reference: §2.2 (Host Layer)
 */

import { KernelClient } from "./client/kernel_client.ts";
import { DagExecutor } from "./dag/executor.ts";

const KERNEL_ENDPOINT = Deno.env.get("EIGENIUS_KERNEL_ENDPOINT") ?? "http://localhost:50051";

async function main() {
  console.log(`Eigenius Orchestration Layer starting...`);
  console.log(`Kernel endpoint: ${KERNEL_ENDPOINT}`);

  const _client = new KernelClient(KERNEL_ENDPOINT);
  const _executor = new DagExecutor(_client);

  // TODO: Start HTTP API gateway
  // TODO: Start MCP server
  // TODO: Health check endpoint

  console.log("Orchestration layer ready.");
}

main().catch(console.error);
