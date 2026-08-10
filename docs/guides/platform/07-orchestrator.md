# 7. The orchestrator

The orchestrator is a Deno/TypeScript service that sits above the kernel and handles three responsibilities the kernel cannot embed directly:

1. **IO component dispatch** — invoke external systems (LLM APIs, HTTP services) on the kernel's behalf when a program executes.
2. **LLM adapter** — talk to the actual model provider (Anthropic via Vercel AI SDK).
3. **MCP server** — expose Eigenius operations as Model Context Protocol tools so external LLM agents can query the knowledge graph and run programs.

The full architectural rationale is in [D6 — Execution architecture](../../design/d6-execution-architecture.md). This chapter is the operational view.

## 7.1. When you need it

| Operation | Orchestrator required? |
|---|---|
| `eigenius load`, `validate`, `compile`, `inspect` | No |
| `eigenius query` against in-process or remote kernel | No (queries are read-only) |
| `eigenius program-validate` | No (type-check is in-process) |
| `eigenius run` against a program with no IO components | No |
| `eigenius run` against a program that calls `CompleteText`, `CompleteJson`, or any IO component | **Yes** |
| `capability install <wasm-file>` for IO-capability components | Yes (the orchestrator hosts IO-level WASM components) |
| MCP-driven workflows (LLM agent calling Eigenius operations) | Yes |

If your workload is purely structural — load, query, type-check — you can skip the orchestrator entirely. As soon as a program dispatches an IO component, the kernel needs the orchestrator endpoint.

## 7.2. Starting the orchestrator

The orchestrator is a Deno program in [`orchestration/`](../../../orchestration/). Three ways to start it:

**Direct (real LLM):**

```bash
cd orchestration
ANTHROPIC_API_KEY=sk-ant-... deno run --allow-net --allow-env --allow-sys=hostname src/main.ts
```

**Direct (mock LLM):**

```bash
EIGENIUS_MOCK_LLM=true deno run --allow-net --allow-env --allow-sys=hostname src/main.ts
```

**Via `just`:**

```bash
just orchestrator         # real LLM
just orchestrator-mock    # mock LLM
```

**Via Docker Compose:** `docker compose up` brings both services up; see [chapter 5](05-running-locally.md).

## 7.3. Configuration via environment variables

Read at startup; no config file:

| Variable | Default | Effect |
|---|---|---|
| `EIGENIUS_KERNEL_ENDPOINT` | `http://localhost:50051` | gRPC endpoint for the kernel (the orchestrator calls back into the kernel for `read-access` and `query-access` host imports) |
| `EIGENIUS_ORCHESTRATOR_PORT` | `8080` | HTTP port the orchestrator binds to |
| `EIGENIUS_MOCK_LLM` | `false` | When `true`, bypass the LLM adapter and return canned responses |
| `ANTHROPIC_API_KEY` | none | Required when `EIGENIUS_MOCK_LLM` is unset |

When the kernel starts, it expects the orchestrator's endpoint via `--orchestrator <url>` or `EIGENIUS_ORCHESTRATOR_ENDPOINT`. The two endpoints (kernel and orchestrator) are independent and known to each other through these flags.

## 7.4. Built-in components

The orchestrator ships with two LLM-backed components, both registered at startup:

### `CompleteText`

**IRI:** `urn:eigenius:program:components:CompleteText`

Plain text completion. Takes a `TextInput` resource (with a `prompt` property) plus configuration (`model`, `temperature`, `max_tokens`); returns the completion as a string-typed resource.

Source: [`orchestration/src/components/complete_text.ts`](../../../orchestration/src/components/complete_text.ts).

In ESL:

```esl
program ex:summarize : ex:Document -> ex:Document {
    let summary : core:string = CompleteText(input);
    Construct ex:Document { ex:text = summary }
}
```

The bare name `CompleteText` resolves to the registered component IRI at compile time.

### `CompleteJson`

**IRI:** `urn:eigenius:program:components:CompleteJson`

Structured-output completion. Takes a `JsonInput` resource (prompt + target class) plus configuration; the orchestrator generates JSON Schema from the target class via `eigenius get-schema` and constrains the LLM to produce a structured result conforming to it.

Source: [`orchestration/src/components/complete_json.ts`](../../../orchestration/src/components/complete_json.ts). Spec: [D8 — CompleteJson Component](../../design/d8-complete-json-component.md).

In ESL:

```esl
program ex:extract : ex:Document -> ex:Entities {
    CompleteJson(input)
}
```

## 7.5. Mock mode

Setting `EIGENIUS_MOCK_LLM=true` swaps the real handlers for mock implementations:

- `CompleteText` returns a canned `[MOCK COMPLETION] ...` string.
- `CompleteJson` returns a minimal JSON object matching the target schema's required fields with placeholder values.

Mock mode is what CI uses (no API key) and what the demo scripts default to (`docker compose up` defaults to mock unless an `ANTHROPIC_API_KEY` is exported).

The mock paths are in the same files as the real paths — `createMockCompleteTextHandler` in [`complete_text.ts`](../../../orchestration/src/components/complete_text.ts), `createMockCompleteJsonHandler` in [`complete_json.ts`](../../../orchestration/src/components/complete_json.ts).

## 7.6. The component registry

[`orchestration/src/components/registry.ts`](../../../orchestration/src/components/registry.ts) maintains a map from component IRI to handler function. On startup, `main.ts` registers the two built-in handlers; WASM components installed at runtime via `capability install` join the registry through the WASM addon path.

When the kernel dispatches a component, the orchestrator's RPC server (`/dispatch`) looks up the IRI in the registry and invokes the handler. The handler returns a CBOR-encoded response, which the orchestrator forwards back to the kernel.

## 7.7. The MCP server

The orchestrator exposes a curated subset of the kernel surface as [Model Context Protocol](https://modelcontextprotocol.io) tools so LLM agents (Claude Code, Claude Desktop, Cursor, IDE agents) can drive Eigenius as part of their reasoning.

Two transports are wired:

- **HTTP** — `http://localhost:8080/mcp` on the orchestrator's existing port, mounted alongside the notebook SPA and Connect RPC. This is the right path for `docker compose up` workflows — the agent client connects directly without a host-side subprocess.
- **stdio** — `cd orchestration && deno task mcp` runs a standalone process that talks MCP over stdin/stdout against an already-running kernel. Useful when you want a kernel on the host (e.g., `eigenius serve --port 50051`) without bringing up the orchestrator container.

Source: [`orchestration/src/mcp/server.ts`](../../../orchestration/src/mcp/server.ts) (tools), [`orchestration/src/mcp/http.ts`](../../../orchestration/src/mcp/http.ts) (HTTP transport), [`orchestration/src/mcp_main.ts`](../../../orchestration/src/mcp_main.ts) (stdio entry point).

### 7.7.1. The tool surface (14 tools)

Each tool wraps one or two kernel RPCs and surfaces the response as JSON. Categories follow the platform's natural division:

**Explore (read-only chain navigation):**

- `eigenius_query` — execute an EigenQL query; rows decoded from Eigon-CBOR to JSON
- `eigenius_inspect` — resolve a single resource by IRI
- `eigenius_list_branches` — branches with head LayerId + commit time
- `eigenius_list_tags` — immutable tag refs
- `eigenius_list_institutions` — D14 institutions with their QueryClasses + comorphisms + runtime kind
- `eigenius_get_schema` — JSON Schema for an ontology class, derived from the chain
- `eigenius_layer_topology` — graph (nodes + edges) of classes / properties / institutions per layer

**Mutate (commit-shaped operations):**

- `eigenius_load` — load Eigon-JSON; supports `policy: "reject" | "cascadeTombstone"`, `branch`, `explicitTombstones` (D41)
- `eigenius_validate_program` — type-check a program without executing
- `eigenius_run_program` — execute program + input (both inline as Eigon-JSON)
- `eigenius_run_program_by_iri` — execute a program already loaded in the chain

**Observe (state and tasks):**

- `eigenius_health` — kernel version, layer count, resource count, D21 resume-sweep state
- `eigenius_list_tasks` — D21 tasks (running, suspended, completed, failed)
- `eigenius_get_task_status` — detail for one task UUID

**Out of scope (deliberately).** Branch / tag mutation, merge submission, chain consolidation, GC, task cancellation. These are stateful multi-turn flows or destructive operations — they belong in the notebook UI or operator CLI, not in a single-turn agent invocation. Use the [CLI](04-cli-reference.md) or the [notebook workspace rail](14-notebook.md) for those.

### 7.7.2. Setup — Docker Compose path (recommended)

Bring the stack up:

```bash
EIGENIUS_MOCK_LLM=true docker compose up -d
# kernel on :50051, orchestrator on :8080 (notebook + /mcp + RPC)
```

Then register the MCP server with your client of choice. For [**Claude Code**](https://claude.com/claude-code) (CLI or VS Code extension):

```bash
claude mcp add --transport http eigenius http://localhost:8080/mcp
claude mcp list                            # verify it shows ✓ Connected
```

For **Claude Desktop**, edit `claude_desktop_config.json` (`%APPDATA%\Claude\` on Windows, `~/Library/Application Support/Claude/` on macOS, `~/.config/Claude/` on Linux). Claude Desktop's native HTTP transport support is uneven across builds — use the [`mcp-remote`](https://www.npmjs.com/package/mcp-remote) stdio bridge to be safe (requires Node.js on the host):

```json
{
  "mcpServers": {
    "eigenius": {
      "command": "npx",
      "args": ["-y", "mcp-remote", "http://localhost:8080/mcp"]
    }
  }
}
```

Some builds accept `{ "type": "http", "url": "http://localhost:8080/mcp" }` directly — try that first if you'd rather skip the bridge. If the app reports "MCP failed to connect," fall back to the `mcp-remote` form above.

Restart the app. The 14 `eigenius_*` tools appear in the tool list (look for the hammer icon in the input area).

### 7.7.3. Setup — stdio path (kernel-on-host development)

When you want a kernel running on your host (faster iteration, no rebuild for kernel changes) but don't want to bring up the orchestrator container, use stdio. Start the kernel directly:

```bash
eigenius serve --port 50051 --db /var/lib/eigenius/db
```

Then configure your client to launch the MCP stdio entry point on demand. Claude Desktop config:

```json
{
  "mcpServers": {
    "eigenius": {
      "command": "deno",
      "args": ["task", "mcp"],
      "cwd": "/absolute/path/to/eigenius/orchestration",
      "env": { "EIGENIUS_KERNEL_ENDPOINT": "http://localhost:50051" }
    }
  }
}
```

`deno task mcp` is defined in [`orchestration/deno.json`](../../../orchestration/deno.json) and runs [`src/mcp_main.ts`](../../../orchestration/src/mcp_main.ts). The orchestrator container does **not** need to be running for this path.

### 7.7.4. The agent guide

[`docs/method/eigenius.md`](../../method/eigenius.md) teaches a coding agent how to use the platform correctly: the mental model, the three surface languages, the MCP tool selection table, minimal Eigon-JSON / ESL / EigenQL shapes, common workflows, and the pitfalls that trip agents up (mandatory `is_a`, synthesized IRI row keys in query results, persistent-backend requirements, D41 multi-layer outcomes, …).

It is a **reference document, not an auto-loaded agent skill** — point the agent at the path, or paste the section you need. Two companions sit beside it: [`reasoning.md`](../../method/reasoning.md), the method for capturing reasoning as a typed chain of graded, witnessed propositions, and [`grounding.md`](../../method/grounding.md), the retrieval-and-citation method its anchoring step uses.

### 7.7.5. Smoke-test with the MCP Inspector

Before wiring into a client, confirm the server works against the [MCP Inspector](https://github.com/modelcontextprotocol/inspector):

```bash
npx @modelcontextprotocol/inspector --url http://localhost:8080/mcp
```

Opens a local web UI where you can list tools and invoke them interactively. If `eigenius_health` returns `{ healthy: true, version: "...", ... }`, the orchestrator → kernel path is good.

### 7.7.6. Debugging

If a client says "MCP server failed to connect," walk the chain bottom-up:

1. **Kernel reachable?** — `curl http://localhost:50051/` (gRPC will reject the bare GET but the connection should establish; "connection refused" means the kernel isn't running).
2. **Orchestrator reachable?** — `curl http://localhost:8080/health` should return `{"healthy":true,...}`.
3. **MCP endpoint live?** — `curl -X POST http://localhost:8080/mcp -H "Accept: application/json, text/event-stream" -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"curl","version":"0"}}}'` should return a JSON-RPC response with `serverInfo.name === "eigenius"`.
4. **Client wiring** — `claude mcp list` (Claude Code) or check `~/Library/Logs/Claude/mcp.log` / `%APPDATA%\Claude\Logs\mcp.log` (Claude Desktop) for handshake errors.

The orchestrator logs each MCP request to stderr; pipe it through `jq` when running against a real client to see what the agent is calling.

### 7.7.7. Architecture and scope notes

- **Mode:** stateless + JSON-response. `WebStandardStreamableHTTPServerTransport` from the SDK with `sessionIdGenerator: undefined` and `enableJsonResponse: true`. Each POST is one tool invocation, one JSON-RPC response. No SSE streaming, no session bookkeeping.
- **Per-request lifecycle:** the SDK requires a fresh `McpServer` + `Transport` pair per request in stateless mode; the orchestrator's HTTP handler builds them on demand and tears them down when the response resolves. The underlying `KernelClient` (Connect RPC over gRPC-Web) is long-lived.
- **CORS:** not configured. Native MCP clients don't need it; if you want a browser-based agent to call MCP from a different origin, add CORS headers to the `/mcp` branch in [`orchestration/src/server/mod.ts`](../../../orchestration/src/server/mod.ts).
- **Authentication:** none. The current model assumes the orchestrator port is reachable only to trusted callers (local docker stack, dev environment). Production deployment should front the orchestrator with a proxy that enforces auth before forwarding to `/mcp`.

## 7.8. WASM IO components

WASM components installed with `--capability io` are hosted by the orchestrator (not the kernel) because they need access to the orchestrator's `io-access` host imports (notably `dispatch-component`, which lets WASM IO components call other components — including `CompleteText`).

The WASM addon machinery lives in [`orchestration/src/wasm/`](../../../orchestration/src/wasm/). Worked example: [`wasm-http-shout`](../../../examples/wasm-http-shout/), which dispatches `CompleteText` from inside WASM. See [chapter 9](09-wasm-components.md) §9.6.

## 7.9. Observability

The orchestrator logs each component dispatch to stdout:

```
[Orchestrator] Dispatching urn:eigenius:program:components:CompleteText
  input.bytes = 142
  argument.bytes = 56
  duration_ms = 1240
```

Plus per-LLM-call metrics from the underlying Vercel AI SDK (token counts, model used). Pipe stdout to a file or your log aggregation tool to capture the full record.

The kernel records its own trace of each component dispatch for incremental re-evaluation; that trace lives in the kernel's trace store, not in the orchestrator. See [D6b — Reasoning trace schema](../../design/d6b-reasoning-trace-schema.md).

## 7.10. Adding a TypeScript-side component

For non-WASM components implemented in TypeScript:

1. Write a handler in `orchestration/src/components/<name>.ts` matching the `ComponentHandler` shape from [`registry.ts`](../../../orchestration/src/components/registry.ts).
2. Register it in `main.ts` alongside the built-ins.
3. Declare the component in your ontology (a `Component` resource with `input_type`, `output_type`, and the chosen IRI).
4. Reference it from your ESL programs by short name or by IRI.

This path is suitable for components that are best written in TypeScript (e.g., wrapping a TypeScript-only library). For components that should be sandboxed and portable, prefer the WASM path ([chapter 9](09-wasm-components.md)).

## 7.11. Stopping the orchestrator

Plain `Ctrl-C` for the foreground process, or `docker compose down` for the containerized setup. There's no persistent state on the orchestrator side — it's stateless between restarts. All persistent state lives in the kernel.

---

Next: **[8. Worked demos →](08-demos.md)**
