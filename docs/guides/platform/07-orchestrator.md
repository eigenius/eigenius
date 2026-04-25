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

The orchestrator exposes Eigenius operations as MCP tools so external LLM agents (Claude Desktop, Cursor, etc.) can query the graph and run programs as part of their reasoning.

Source: [`orchestration/src/mcp/server.ts`](../../../orchestration/src/mcp/server.ts).

Tools currently exposed:

- **`eigenius_query`** — execute an EigenQL query against the kernel
- **`eigenius_inspect`** — fetch a resource by IRI
- **`eigenius_load`** — load an Eigon-JSON or ESL file
- **`eigenius_run`** — execute a typed program
- **`eigenius_list_institutions`** — list registered institutions

Connect a tool-using LLM by pointing its MCP client at `http://localhost:8080/mcp`. The exact configuration depends on the client; see the MCP specification at [modelcontextprotocol.io](https://modelcontextprotocol.io).

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
