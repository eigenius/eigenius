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
 * HTTP transport for the Eigenius MCP server.
 *
 * Mounts on the orchestrator's existing HTTP port (`/mcp`) so MCP clients
 * configured with an HTTP URL — Claude Desktop, Claude Code CLI, IDE
 * agents — can drive the kernel through the dockerized stack without a
 * separate stdio subprocess.
 *
 * Built on the MCP SDK's `WebStandardStreamableHTTPServerTransport`
 * (added in SDK 1.25.0). The transport's API is pure Web-standard
 * (`Request` in, `Response` out), so it slots into `Deno.serve`
 * directly — no Node-stream shimming.
 *
 * Mode: **stateless + JSON-response**. `sessionIdGenerator: undefined`
 * disables session tracking; `enableJsonResponse: true` returns plain
 * JSON-RPC responses instead of opening an SSE stream. Our tool
 * surface is synchronous request-response (every tool invocation
 * resolves to one MCP `CallToolResult`), so SSE buys nothing.
 *
 * Per-request lifecycle: a fresh `McpServer` + `Transport` pair is
 * constructed for every request, then closed when the request resolves.
 * The SDK enforces this — stateless transports cannot be reused, since
 * they keep per-request state to route in-flight responses. See the
 * SDK's `examples/server/simpleStatelessStreamableHttp.js` for the
 * canonical Express version of the same pattern.
 */

import { WebStandardStreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/webStandardStreamableHttp.js";
import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import * as log from "../observability/mod.ts";
import { operation } from "../observability/mod.ts";

/**
 * Build an HTTP handler that serves the MCP protocol against MCP server
 * instances produced by `buildServer`. Caller mounts the returned handler
 * on a route (typically `/mcp`).
 *
 * `buildServer` is invoked per request — the SDK requires a fresh
 * server + transport for every stateless invocation. The tool registry
 * itself is reconstructed each call but the work is cheap (no I/O, just
 * Zod schema + handler closure registration), and the underlying
 * `KernelClient` / Connect transport is long-lived.
 */
export function createMcpHttpHandler(
  buildServer: () => McpServer,
): (req: Request) => Promise<Response> {
  log.info(operation.MCP_SERVER_START, "MCP HTTP handler ready", {
    transport: "http",
    mode: "stateless+json",
  });

  return async (req: Request): Promise<Response> => {
    const transport = new WebStandardStreamableHTTPServerTransport({
      sessionIdGenerator: undefined,
      enableJsonResponse: true,
    });
    const server = buildServer();
    try {
      await server.connect(transport);
      return await transport.handleRequest(req);
    } catch (error) {
      log.error(operation.MCP_SERVER_START, "MCP request failed", {
        error_kind: "mcp_request_panic",
        error_message: error instanceof Error ? error.message : String(error),
      });
      return new Response(
        JSON.stringify({
          jsonrpc: "2.0",
          error: { code: -32603, message: "Internal server error" },
          id: null,
        }),
        {
          status: 500,
          headers: { "Content-Type": "application/json" },
        },
      );
    } finally {
      // The SDK example listens for `res.on('close')` and closes
      // there; Deno's Web `Response` flow doesn't expose an equivalent
      // event, but since `handleRequest` resolves once the response is
      // fully composed, closing eagerly after the await is safe.
      await transport.close().catch(() => {});
      await server.close().catch(() => {});
    }
  };
}
