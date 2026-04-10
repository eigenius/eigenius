/**
 * MCP Server — LLM → Core tool-use surface
 *
 * Exposes the kernel's four operations (Load, Query, Validate, Reflect)
 * as MCP tools that an LLM agent can invoke via tool-use.
 *
 * Architecture reference: §2.3 (AI Integration Model)
 */

import { KernelClient } from "../client/kernel_client.ts";

export class EigeniusMcpServer {
  private client: KernelClient;

  constructor(client: KernelClient) {
    this.client = client;
  }

  async start(_port: number): Promise<void> {
    // TODO: Phase 4 — MCP server implementation
    // Register tools: eigenius_load, eigenius_query, eigenius_validate, eigenius_reflect
    // Each tool delegates to the kernel client
    throw new Error("Not implemented");
  }
}
