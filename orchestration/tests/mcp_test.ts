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
 * Unit tests for the MCP server + HTTP transport.
 *
 * These tests run entirely in-process: the orchestrator's `KernelClient`
 * is replaced with a stub that returns canned proto responses, and the
 * HTTP handler is driven via synthetic `Request` objects (no socket).
 * No kernel subprocess, no network — runs in milliseconds.
 *
 * What this catches:
 *  - A tool getting accidentally dropped during a refactor
 *    (`tools/list` count regression).
 *  - The implicit-any handler signature bug that broke compile in the
 *    initial rewrite.
 *  - The "stateless transport cannot be reused" regression
 *    (sequential-requests test).
 *  - Kernel-side errors silently succeeding (the isError contract).
 *
 * What this deliberately does NOT cover:
 *  - End-to-end against a real kernel (Layer 2 — `topology_e2e_test.ts`
 *    pattern; spawn kernel + orchestrator subprocesses, POST over the
 *    network).
 *  - Per-tool argument validation against the kernel's actual proto
 *    contract (those are caught by `deno check` + the e2e layer).
 */

import { assert, assertEquals, assertExists } from "@std/assert";
import { create } from "@bufbuild/protobuf";
import { HealthResponseSchema } from "../src/gen/eigenius_pb.ts";
import { createMcpServer } from "../src/mcp/server.ts";
import { createMcpHttpHandler } from "../src/mcp/http.ts";
import type { KernelClient } from "../src/client/kernel_client.ts";

// ---------------------------------------------------------------------------
// Stub KernelClient
// ---------------------------------------------------------------------------

/**
 * Build a KernelClient-shaped stub. Only the methods the tests below
 * actually invoke need to be supplied — `tools/list` doesn't call any
 * RPCs, so an empty `raw` works for that. `tools/call <tool>` requires
 * the corresponding `raw.<rpc>` to be present.
 */
// deno-lint-ignore no-explicit-any
type StubRaw = Record<string, (req?: unknown) => Promise<any>>;

function makeStubClient(raw: StubRaw = {}): KernelClient {
  return { raw } as unknown as KernelClient;
}

// ---------------------------------------------------------------------------
// HTTP request helper
// ---------------------------------------------------------------------------

async function rpcCall(
  handler: (req: Request) => Promise<Response>,
  body: unknown,
): Promise<{
  status: number;
  // deno-lint-ignore no-explicit-any
  body: any;
}> {
  const req = new Request("http://test/mcp", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "Accept": "application/json, text/event-stream",
    },
    body: JSON.stringify(body),
  });
  const res = await handler(req);
  return { status: res.status, body: await res.json() };
}

// ---------------------------------------------------------------------------
// The canonical tool inventory. If you add or remove a tool in
// `mcp/server.ts`, update this list — the `tools/list` test will fail
// otherwise. That failure is on purpose; a silent tool-count change is
// exactly what these tests exist to surface.
// ---------------------------------------------------------------------------

const EXPECTED_TOOLS = [
  "eigenius_query",
  "eigenius_inspect",
  "eigenius_list_branches",
  "eigenius_list_tags",
  "eigenius_list_institutions",
  "eigenius_get_schema",
  "eigenius_layer_topology",
  "eigenius_load",
  "eigenius_validate_program",
  "eigenius_run_program",
  "eigenius_run_program_by_iri",
  "eigenius_health",
  "eigenius_list_tasks",
  "eigenius_get_task_status",
].sort();

// ===========================================================================
// Server construction
// ===========================================================================

Deno.test("createMcpServer instantiates without I/O", () => {
  const server = createMcpServer(makeStubClient());
  assertExists(server);
});

// ===========================================================================
// tools/list
// ===========================================================================

Deno.test("HTTP handler: tools/list returns exactly the expected 14 tools", async () => {
  const handler = createMcpHttpHandler(() => createMcpServer(makeStubClient()));
  const { status, body } = await rpcCall(handler, {
    jsonrpc: "2.0",
    id: 1,
    method: "tools/list",
    params: {},
  });
  assertEquals(status, 200);
  assertExists(body.result, `no result in response: ${JSON.stringify(body)}`);
  const names = body.result.tools
    .map((t: { name: string }) => t.name)
    .sort();
  assertEquals(names, EXPECTED_TOOLS);
});

Deno.test("HTTP handler: every tool advertises an object-typed inputSchema", async () => {
  const handler = createMcpHttpHandler(() => createMcpServer(makeStubClient()));
  const { body } = await rpcCall(handler, {
    jsonrpc: "2.0",
    id: 1,
    method: "tools/list",
    params: {},
  });
  for (const t of body.result.tools) {
    assertExists(t.inputSchema, `tool ${t.name} missing inputSchema`);
    assertEquals(
      t.inputSchema.type,
      "object",
      `tool ${t.name} inputSchema.type !== "object"`,
    );
  }
});

Deno.test("HTTP handler: tools carrying required arguments declare them", async () => {
  const handler = createMcpHttpHandler(() => createMcpServer(makeStubClient()));
  const { body } = await rpcCall(handler, {
    jsonrpc: "2.0",
    id: 1,
    method: "tools/list",
    params: {},
  });
  // deno-lint-ignore no-explicit-any
  const byName: Record<string, any> = {};
  for (const t of body.result.tools) byName[t.name] = t;

  // Sanity-check a few tools whose required-field set we care about.
  // Catches schema regressions like an optional field becoming required
  // or vice versa.
  assertEquals(byName.eigenius_query.inputSchema.required, ["eigenql"]);
  assertEquals(byName.eigenius_inspect.inputSchema.required, ["iri"]);
  assertEquals(byName.eigenius_load.inputSchema.required, ["json"]);
  assertEquals(byName.eigenius_run_program.inputSchema.required, [
    "programJson",
    "inputJson",
  ]);
  assertEquals(byName.eigenius_get_task_status.inputSchema.required, [
    "taskId",
  ]);

  // No-args tools should not declare a required array (or it should be empty).
  for (
    const noArgs of [
      "eigenius_health",
      "eigenius_list_branches",
      "eigenius_list_tags",
      "eigenius_list_tasks",
    ]
  ) {
    const req = byName[noArgs].inputSchema.required;
    if (req !== undefined) assertEquals(req, []);
  }
});

// ===========================================================================
// tools/call — happy path
// ===========================================================================

Deno.test("HTTP handler: tools/call eigenius_health surfaces the kernel response", async () => {
  const client = makeStubClient({
    health: () =>
      Promise.resolve(create(HealthResponseSchema, {
        healthy: true,
        version: "test-1.0",
        layerCount: 7n,
        resourceCount: 42n,
      })),
  });
  const handler = createMcpHttpHandler(() => createMcpServer(client));

  const { status, body } = await rpcCall(handler, {
    jsonrpc: "2.0",
    id: 2,
    method: "tools/call",
    params: { name: "eigenius_health", arguments: {} },
  });
  assertEquals(status, 200);
  assertExists(body.result?.content?.[0]?.text);
  const parsed = JSON.parse(body.result.content[0].text);
  assertEquals(parsed.healthy, true);
  assertEquals(parsed.version, "test-1.0");
  // uint64 fields come through `toJson` as decimal strings.
  assertEquals(parsed.layerCount, "7");
  assertEquals(parsed.resourceCount, "42");
});

// ===========================================================================
// tools/call — kernel rejection propagates as isError
// ===========================================================================

Deno.test("HTTP handler: tools/call surfaces kernel rejections as isError", async () => {
  const client = makeStubClient({
    listBranches: () =>
      Promise.reject(
        new Error("branch operations require a persistent backend"),
      ),
  });
  const handler = createMcpHttpHandler(() => createMcpServer(client));

  const { status, body } = await rpcCall(handler, {
    jsonrpc: "2.0",
    id: 3,
    method: "tools/call",
    params: { name: "eigenius_list_branches", arguments: {} },
  });
  assertEquals(status, 200);
  // The SDK wraps a thrown handler error as `isError: true` content
  // (CallToolResult shape) rather than a JSON-RPC error envelope.
  // That's the contract we depend on — if a future SDK switches to
  // JSON-RPC errors for handler throws, the assertion fires.
  assertEquals(body.result?.isError, true);
  const text = body.result.content[0].text;
  assert(
    typeof text === "string" && text.length > 0,
    `expected non-empty error text, got: ${JSON.stringify(text)}`,
  );
});

// ===========================================================================
// Per-request transport lifecycle
// ===========================================================================

Deno.test(
  "HTTP handler: sequential calls succeed (per-request fresh transport)",
  async () => {
    // The SDK's stateless transport must be reconstructed per request —
    // reusing one across requests raises
    //   "Stateless transport cannot be reused across requests."
    // (See `mcp/http.ts` for the per-request `buildServer()` pattern.)
    // This test pins that contract: three back-to-back calls must all
    // succeed against a single handler instance.
    const handler = createMcpHttpHandler(() =>
      createMcpServer(makeStubClient({
        health: () =>
          Promise.resolve(create(HealthResponseSchema, { healthy: true })),
      }))
    );

    for (let i = 0; i < 3; i++) {
      const { status, body } = await rpcCall(handler, {
        jsonrpc: "2.0",
        id: 100 + i,
        method: "tools/call",
        params: { name: "eigenius_health", arguments: {} },
      });
      assertEquals(status, 200, `request ${i} status`);
      assertExists(body.result, `request ${i} had no result`);
      assertEquals(body.result.isError, undefined, `request ${i} was error`);
    }
  },
);

// ===========================================================================
// tools/call — unknown tool
// ===========================================================================

Deno.test("HTTP handler: tools/call with unknown tool returns an error", async () => {
  const handler = createMcpHttpHandler(() => createMcpServer(makeStubClient()));

  const { body } = await rpcCall(handler, {
    jsonrpc: "2.0",
    id: 4,
    method: "tools/call",
    params: { name: "eigenius_nonexistent", arguments: {} },
  });
  // The SDK may report this as a JSON-RPC error or as `isError: true`
  // content — accept either, but reject silent success.
  const isJsonRpcError = body.error !== undefined;
  const isContentError = body.result?.isError === true;
  assert(
    isJsonRpcError || isContentError,
    `expected error response, got: ${JSON.stringify(body)}`,
  );
});
