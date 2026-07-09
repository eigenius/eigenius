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
 * Orchestrator HTTP/gRPC server.
 *
 * Serves two things on a single port:
 * 1. ComponentExecutor gRPC service (Connect protocol) — kernel dispatches here
 * 2. /health HTTP endpoint — for container readiness probes
 *
 * Uses Deno.serve with Connect's universal handler for gRPC,
 * falling back to plain HTTP for /health.
 */

import { createConnectRouter } from "@connectrpc/connect";
import type { ComponentRegistry } from "../components/registry.ts";
import type { KernelClient } from "../client/kernel_client.ts";
import * as log from "../observability/mod.ts";
import { operation } from "../observability/mod.ts";
import {
  type ComponentExecutorDeps,
  registerComponentExecutor,
} from "./component_executor.ts";
import { registerNotebookService } from "../notebook/notebook_service.ts";
import { registerEigeniusKernelPassthrough } from "../notebook/eigenius_kernel_passthrough.ts";
import { createNotebookStaticHandler } from "./notebook_static.ts";

/**
 * Start the orchestrator server.
 *
 * Listens on `port` and serves the Connect surfaces plus a health endpoint:
 * - gRPC: ComponentExecutor.Execute       (kernel → orchestrator IO dispatch)
 * - gRPC: NotebookService.LayerTopology   (browser → orchestrator → kernel)
 * - HTTP: GET /health
 * - HTTP: POST /mcp   (when `mcpHandler` is supplied — MCP Streamable HTTP)
 *
 * The browser also reaches the existing EigeniusKernel surface (Inspect,
 * Query, Load, RunProgram, etc.) via the orchestrator's kernel client;
 * NotebookService is only for browser-specific RPCs that don't fit the
 * kernel's machine-to-machine surface (D22 §3.2).
 */
export function startServer(
  registry: ComponentRegistry,
  kernel: KernelClient,
  port: number,
  substrate?: ComponentExecutorDeps["substrate"],
  mcpHandler?: (req: Request) => Promise<Response>,
): void {
  const router = createConnectRouter();
  registerComponentExecutor(router, { registry, substrate });
  registerNotebookService(router, { kernel });
  registerEigeniusKernelPassthrough(router, { kernel });

  // Optional notebook SPA static-file route. Active when
  // EIGENIUS_NOTEBOOK_STATIC points at a Vite-built dist/ directory
  // (D22 §6.10). In dev the notebook is served from `vite dev` on a
  // separate port and proxies RPC traffic here, so this is unset.
  const notebookStaticDir = (Deno.env.get("EIGENIUS_NOTEBOOK_STATIC") ?? "")
    .trim();
  const notebookStatic = notebookStaticDir.length > 0
    ? createNotebookStaticHandler(notebookStaticDir)
    : null;

  Deno.serve({ port }, async (req: Request) => {
    const url = new URL(req.url);

    // Health endpoint
    if (url.pathname === "/health" && req.method === "GET") {
      return new Response(
        JSON.stringify({
          healthy: true,
          service: "eigenius-orchestrator",
          components: registry.listComponents(),
        }),
        {
          status: 200,
          headers: { "Content-Type": "application/json" },
        },
      );
    }

    // MCP Streamable HTTP endpoint (when wired up). The transport
    // accepts POST (client → server messages), GET (SSE stream — unused
    // in our stateless+JSON mode but still routed), and DELETE (session
    // termination). Handing the full Web Request to the SDK keeps the
    // protocol shape entirely on the SDK side.
    if (mcpHandler && url.pathname === "/mcp") {
      return await mcpHandler(req);
    }

    // Notebook SPA static files (when configured).
    if (notebookStatic) {
      const staticResp = await notebookStatic.tryServe(req);
      if (staticResp) return staticResp;
    }

    // Try Connect/gRPC handler for everything else
    try {
      // connectNodeAdapter expects Node IncomingMessage/ServerResponse.
      // For Deno, we use the universal handler approach instead.
      // Fall through to a 404 if not handled.
      const response = await handleConnectRequest(router, req);
      if (response) return response;
    } catch (e) {
      log.error(operation.COMPONENT_DISPATCH, "Connect handler threw", {
        error_kind: "connect_handler_panic",
        error_message: e instanceof Error ? e.message : String(e),
      });
    }

    return new Response("Not Found", { status: 404 });
  });

  log.info(operation.SERVER_START, "orchestrator server listening", {
    port,
    services: [
      "ComponentExecutor",
      "NotebookService",
      "EigeniusKernel(passthrough)",
    ],
    health_endpoint: "/health",
    notebook_static: notebookStatic ? notebookStaticDir : null,
    mcp_endpoint: mcpHandler ? "/mcp" : null,
  });
}

/**
 * Handle a Connect/gRPC request using the universal handlers from the router.
 */
async function handleConnectRequest(
  router: ReturnType<typeof createConnectRouter>,
  req: Request,
): Promise<Response | null> {
  const url = new URL(req.url);

  // Find matching handler by path
  for (const handler of router.handlers) {
    if (url.pathname === handler.requestPath) {
      const uReq = {
        httpVersion: "2.0",
        method: req.method,
        // Must be a full URL, not just the path — the Connect protocol
        // handler factory does `new URL(uReq.url)` which throws on a
        // bare path. (gRPC took a different code path so this was
        // latent until the first Connect-protocol client hit it.)
        url: req.url,
        header: new Headers(req.headers),
        body: asyncIterableFromRequest(req),
        signal: req.signal,
      };

      const uRes = await handler(uReq);

      return new Response(concatUint8Arrays(uRes.body), {
        status: uRes.status,
        headers: uRes.header,
      });
    }
  }

  return null;
}

/**
 * Convert a Request body to an async iterable of Uint8Array.
 */
async function* asyncIterableFromRequest(
  req: Request,
): AsyncIterable<Uint8Array> {
  if (!req.body) return;
  const reader = req.body.getReader();
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      yield value;
    }
  } finally {
    reader.releaseLock();
  }
}

/**
 * Collect async iterable body into a single Uint8Array for Response.
 */
function concatUint8Arrays(
  body: AsyncIterable<Uint8Array> | undefined,
): ReadableStream<Uint8Array> | undefined {
  if (!body) return undefined;
  return new ReadableStream({
    async start(controller) {
      for await (const chunk of body) {
        controller.enqueue(chunk);
      }
      controller.close();
    },
  });
}
