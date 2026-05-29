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
 * MCP Server — LLM tool-use surface for the Eigenius kernel.
 *
 * Exposes kernel operations as MCP tools that an LLM agent can invoke:
 * - eigenius_query: Execute an EigenQL query
 * - eigenius_inspect: Resolve a resource by IRI
 * - eigenius_load: Load resources into the kernel
 * - eigenius_validate: Validate a program
 *
 * Transport: SSE/HTTP for remote agents. Stdio also available.
 *
 * Architecture reference: §2.3 (AI Integration Model), Phase 4 plan §5
 */

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import type { KernelClient } from "../client/kernel_client.ts";
import * as log from "../observability/mod.ts";
import { operation } from "../observability/mod.ts";

/**
 * Create and configure the Eigenius MCP server.
 */
export function createMcpServer(client: KernelClient): McpServer {
  const server = new McpServer({
    name: "eigenius",
    version: "0.1.0",
  });

  // --- eigenius_query ---
  server.tool(
    "eigenius_query",
    "Execute an EigenQL query against the Eigenius knowledge graph. Returns matching resources as JSON.",
    { eigenql: z.string().describe("The EigenQL query string") },
    async (args: { eigenql: string }) => {
      try {
        const results = await client.query(args.eigenql);
        const decoder = new TextDecoder();
        const decoded = results.map((r: Uint8Array) => decoder.decode(r));
        return {
          content: [{
            type: "text" as const,
            text: JSON.stringify(decoded, null, 2),
          }],
        };
      } catch (e) {
        return {
          content: [{
            type: "text" as const,
            text: `Query failed: ${(e as Error).message}`,
          }],
          isError: true,
        };
      }
    },
  );

  // --- eigenius_inspect ---
  server.tool(
    "eigenius_inspect",
    "Resolve a resource by its IRI from the Eigenius knowledge graph. Returns the resource as JSON.",
    { iri: z.string().describe("The IRI of the resource to inspect") },
    async (args: { iri: string }) => {
      try {
        const response = await client.inspect(args.iri);
        if (!response.found) {
          return {
            content: [{
              type: "text" as const,
              text: `Resource not found: ${args.iri}`,
            }],
          };
        }
        const decoder = new TextDecoder();
        return {
          content: [{
            type: "text" as const,
            text: decoder.decode(response.resource),
          }],
        };
      } catch (e) {
        return {
          content: [{
            type: "text" as const,
            text: `Inspect failed: ${(e as Error).message}`,
          }],
          isError: true,
        };
      }
    },
  );

  // --- eigenius_load ---
  server.tool(
    "eigenius_load",
    "Load resources into the Eigenius knowledge graph. Resources are provided as Eigon-JSON.",
    {
      json: z.string().describe("Eigon-JSON array of resources to load"),
      auto_commit: z.boolean().optional().describe(
        "Whether to commit after loading (default: true)",
      ),
    },
    async (args: { json: string; auto_commit?: boolean }) => {
      try {
        const response = await client.load(
          args.json,
          { autoCommit: args.auto_commit ?? true },
        );
        return {
          content: [{
            type: "text" as const,
            text: JSON.stringify(
              {
                success: response.success,
                resourceCount: response.resourceCount,
                layerId: response.layerId,
                errors: response.errors,
              },
              null,
              2,
            ),
          }],
        };
      } catch (e) {
        return {
          content: [{
            type: "text" as const,
            text: `Load failed: ${(e as Error).message}`,
          }],
          isError: true,
        };
      }
    },
  );

  // --- eigenius_validate ---
  server.tool(
    "eigenius_validate",
    "Type-check and validate an Eigenius program. Returns validation results.",
    {
      program: z.string().describe("Program resource as Eigon-JSON"),
    },
    async (args: { program: string }) => {
      try {
        const response = await client.validateProgram(args.program);
        return {
          content: [{
            type: "text" as const,
            text: JSON.stringify(
              {
                valid: response.valid,
                programType: response.programType,
                errors: response.errors,
              },
              null,
              2,
            ),
          }],
        };
      } catch (e) {
        return {
          content: [{
            type: "text" as const,
            text: `Validate failed: ${(e as Error).message}`,
          }],
          isError: true,
        };
      }
    },
  );

  return server;
}

/**
 * Start the MCP server with stdio transport (for local integration).
 */
export async function startStdioServer(server: McpServer): Promise<void> {
  const transport = new StdioServerTransport();
  await server.connect(transport);
  log.info(operation.MCP_SERVER_START, "MCP server connected", {
    transport: "stdio",
  });
}
