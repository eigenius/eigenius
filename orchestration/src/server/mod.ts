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
import { ComponentRegistry } from "../components/registry.ts";
import {
  type ComponentExecutorDeps,
  registerComponentExecutor,
} from "./component_executor.ts";

/**
 * Start the orchestrator server.
 *
 * Listens on `port` and serves:
 * - gRPC: ComponentExecutor.Execute (Connect protocol)
 * - HTTP: GET /health
 */
export function startServer(
  registry: ComponentRegistry,
  port: number,
  wasm?: ComponentExecutorDeps["wasm"],
): void {
  const router = createConnectRouter();
  registerComponentExecutor(router, { registry, wasm });

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

    // Try Connect/gRPC handler for everything else
    try {
      // connectNodeAdapter expects Node IncomingMessage/ServerResponse.
      // For Deno, we use the universal handler approach instead.
      // Fall through to a 404 if not handled.
      const response = await handleConnectRequest(router, req);
      if (response) return response;
    } catch {
      // Not a Connect request, fall through
    }

    return new Response("Not Found", { status: 404 });
  });

  console.log(`Orchestrator server listening on port ${port}`);
  console.log(`  gRPC: ComponentExecutor service (Connect protocol)`);
  console.log(`  HTTP: GET /health`);
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
        url: url.pathname + url.search,
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
