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
 * MCP stdio entry point.
 *
 * Runs the Eigenius MCP server over stdin / stdout — the standard wiring
 * for Claude Desktop, IDE-integrated agents, and `mcp-cli`. Connects to
 * an already-running kernel at `EIGENIUS_KERNEL_ENDPOINT` (no kernel is
 * spawned by this process).
 *
 * Example Claude Desktop config:
 *
 *     {
 *       "mcpServers": {
 *         "eigenius": {
 *           "command": "deno",
 *           "args": ["task", "mcp"],
 *           "cwd": "/path/to/eigenius/orchestration",
 *           "env": { "EIGENIUS_KERNEL_ENDPOINT": "http://localhost:50051" }
 *         }
 *       }
 *     }
 *
 * The orchestrator's main HTTP service (`src/main.ts`) is independent —
 * the MCP entry point connects directly to the kernel and does not need
 * the orchestrator's component / substrate plumbing.
 */

import { KernelClient } from "./client/kernel_client.ts";
import { createMcpServer, startStdioServer } from "./mcp/server.ts";
import * as log from "./observability/mod.ts";

const KERNEL_ENDPOINT = Deno.env.get("EIGENIUS_KERNEL_ENDPOINT") ??
  "http://localhost:50051";

async function main() {
  log.init();

  const client = new KernelClient(KERNEL_ENDPOINT);
  const server = createMcpServer(client);
  await startStdioServer(server);
}

await main();
