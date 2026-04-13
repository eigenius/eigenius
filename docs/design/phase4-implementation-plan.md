# Phase 4 — Intelligence: Detailed Implementation Plan

*April 2026*

**Goal:** LLMs can be invoked from programs and can invoke Eigenius as a tool. Reasoning traces are recorded and queryable with full provenance. The system deploys to Azure.

**Prerequisites:** Phases 0-3 complete. Design docs D6 (execution architecture) and D6b (reasoning trace schema) complete.

---

## Architecture

```
┌──────────────────────┐           ┌─────────┐           ┌──────────────────────────┐
│  Kernel (Rust)        │◄─────────│  DAPR    │──────────►│  Orchestrator (Deno)      │
│                       │──────────►│ sidecars │◄──────────│                           │
│  • Program evaluation  │           │          │           │  • LLM adapters           │
│  • Trace-based caching │  DAPR     │ • mTLS   │           │    (Vercel AI SDK)        │
│  • Trace storage       │  service  │          │           │  • MCP server             │
│  • EigenQL queries     │  invoke   │          │           │  • Component handlers     │
│  • gRPC API            │           │          │           │  • Connect RPC client     │
│  • RocksDB             │           │          │           │                           │
└──────────────────────┘           └─────────┘           └──────────────────────────┘
```

Phase 4 can start **without DAPR** — direct gRPC between kernel and orchestrator. DAPR adds mTLS and observability when deploying to Azure.

---

## Step 1: Reflection ontology (`ontologies/reflection/`)

**Goal:** Define the trace classes and epistemic base classes as an ontology layer loaded at bootstrap.

Creates `ontologies/reflection/reflection-ontology.json` containing:

**Trace classes (from D6b):**
- `ComponentTrace`, `PureTrace`, `LetTrace`, `MapTrace`, `ReduceTrace`, `CaseTrace`, `ConstructTrace`, `ProjectTrace`
- `ProgramTrace`, `DeclarationTrace`, `ObservationTrace`, `VerificationTrace`

**Epistemic base classes:**
- `DeclaredResource` (requires: `declared_by`)
- `ObservedResource` (requires: `source`)
- `DerivedResource` (requires: `derivation`)
- `VerifiedResource` (requires: `derivation`, `verification`)
- `EpistemicStatus` with instances: `declared`, `observed`, `derived`, `verified`

**All properties** from D6b §3 and §6.

Update `bootstrap.rs` to load the reflection ontology as a third layer (core → program → reflection).

Testing:
- Bootstrap loads all three ontology layers
- Reflection classes resolvable from any execution context
- Validation: a `DerivedResource` without `derivation` property fails

---

## Step 2: Trace recording in program executor (`kernel/src/program/`)

**Goal:** The program executor returns `(Value, Option<Trace>)` from each evaluation step, as specified in D6b §2.1.

Refactor `program/execute.rs`:
- Each expression evaluation returns its result and a trace
- IO component calls check the trace store before dispatching
- ComponentTraces are stored in RocksDB with content-addressed keys
- ProgramTrace wraps the complete trace tree

```rust
pub fn execute_program(
    program: &Resource,
    input: &Resource,
    layer: &Layer,
    registry: &ComponentRegistry,
    trace_store: &dyn TraceStore,
) -> Result<(Resource, ProgramTrace), ProgramError>;
```

New trait:
```rust
pub trait TraceStore: Send + Sync {
    fn get_component_trace(&self, key: &[u8; 32]) -> Option<ComponentTrace>;
    fn put_component_trace(&self, key: [u8; 32], trace: ComponentTrace);
}
```

Testing:
- Execute identity program → produces ProgramTrace with PureTrace inside
- Execute with mock IO component → produces ComponentTrace with metrics
- Re-execute same program → traces cached, no re-dispatch
- Crash recovery: store partial traces, resume, verify completion

---

## Step 3: Orchestrator gRPC client (Connect RPC)

**Goal:** The Deno orchestrator connects to the kernel via Connect RPC and exposes the ComponentExecutor service.

Set up:
- `@connectrpc/connect` and `@bufbuild/protobuf` in `orchestration/deno.json`
- Generate TypeScript types from `proto/eigenius.proto`
- Implement `KernelClient` using Connect transport
- Implement `ComponentExecutor` gRPC service (the reverse direction — kernel calls orchestrator)

```typescript
// Kernel calls this to execute IO components
const componentExecutor = {
    execute: async (request: ComponentRequest): Promise<ComponentResponse> => {
        const handler = handlers[request.componentIri];
        if (!handler) {
            return { success: false, error: `Unknown component: ${request.componentIri}` };
        }
        return await handler(request.input, request.argument);
    },
};
```

Testing:
- Orchestrator connects to kernel, calls Health RPC
- Kernel dispatches component to orchestrator, receives result
- Round-trip: kernel → orchestrator (execute component) → kernel (store trace)

---

## Step 4: CompleteText LLM adapter

**Goal:** Implement CompleteText as the first real LLM component using Vercel AI SDK. CompleteJson is deferred to a follow-up once the pipeline is proven.

**CompleteText argument structure:**

```json
{
  "@id": "urn:eigenius:example:my-prompt",
  "urn:eigenius:core:is_a": ["urn:eigenius:program:components:completion:Arguments"],
  "urn:eigenius:program:components:completion:user_prompt": "Summarize this document:\n\n{{string}}",
  "urn:eigenius:program:components:completion:system_prompt": "You are a helpful assistant.",
  "urn:eigenius:program:components:completion:request_parameters": {
    "urn:eigenius:core:is_a": ["urn:eigenius:program:RequestParameters"],
    "urn:eigenius:program:request:model": "claude-sonnet-4-20250514",
    "urn:eigenius:program:request:temperature": 0.3,
    "urn:eigenius:program:request:max_tokens": 4000
  }
}
```

**Implementation:**

```typescript
import { generateText } from "ai";
import { anthropic } from "@ai-sdk/anthropic";

handlers["urn:eigenius:program:components:CompleteText"] = async (input, argument) => {
    const params = decodeRequestParams(argument);
    const prompt = formatPrompt(argument, input);
    const startTime = Date.now();
    
    const result = await generateText({
        model: anthropic(params.model),
        system: params.systemPrompt,
        prompt,
        temperature: params.temperature,
        maxTokens: params.maxTokens,
    });

    return {
        success: true,
        output: encodeAsResource(result.text),
        metrics: {
            provider: "anthropic",
            model: params.model,
            promptTokens: result.usage.promptTokens,
            completionTokens: result.usage.completionTokens,
            latencyMs: Date.now() - startTime,
        },
    };
};
```

Add to `orchestration/deno.json`:
```json
{
  "imports": {
    "ai": "npm:ai@latest",
    "@ai-sdk/anthropic": "npm:@ai-sdk/anthropic@latest"
  }
}
```

**Mock for testing:** A mock handler that returns deterministic text without an API call, used for the full trace pipeline tests.

Testing:
- Mock provider: verify argument parsing, prompt formatting, output wrapping
- Real API call (integration test, requires `ANTHROPIC_API_KEY`): CompleteText returns text
- Error handling: API timeout → ComponentResponse with error
- Metrics: token counts and latency recorded in ComponentTrace

**Deferred:** CompleteJson (structured output with schema validation) — follow-up once the pipeline is proven with CompleteText.

---

## Step 5: MCP server

**Goal:** Expose kernel operations as MCP tools for LLM agents.

```typescript
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";

const server = new McpServer({ name: "eigenius", version: "0.1.0" });

server.tool("eigenius_query", { eigenql: z.string() }, async ({ eigenql }) => {
    const results = await kernelClient.query(eigenql);
    return { content: [{ type: "text", text: JSON.stringify(results) }] };
});

server.tool("eigenius_inspect", { iri: z.string() }, async ({ iri }) => {
    const resource = await kernelClient.inspect(iri);
    return { content: [{ type: "text", text: JSON.stringify(resource) }] };
});

server.tool("eigenius_load", { json: z.string() }, async ({ json }) => {
    const result = await kernelClient.load(json);
    return { content: [{ type: "text", text: JSON.stringify(result) }] };
});

server.tool("eigenius_validate", { program: z.string() }, async ({ program }) => {
    const result = await kernelClient.validateProgram(program);
    return { content: [{ type: "text", text: JSON.stringify(result) }] };
});
```

Transport: SSE/HTTP for remote agents (default port 3000). Stdio transport also available for local integration.

Testing:
- Start MCP server, connect MCP client, invoke `eigenius_query`
- Verify result matches direct kernel query
- Test with Claude Desktop or similar MCP client

---

## Step 6: Epistemic base classes in validation

**Goal:** The kernel validates epistemic base class requirements.

When a resource declares `is_a: ["urn:eigenius:reflection:DerivedResource"]`, validation ensures the `derivation` property is present and points to a valid ProgramTrace.

Update `validation/mod.rs`:
- Recognize epistemic base classes during validation
- Enforce their `requires` properties
- Compute effective epistemic status from `is_a`

New CLI command:
```
eigenius reflect <trace-file>     Record a reasoning trace manually
```

Testing:
- Create a `DerivedResource` without `derivation` → validation error
- Create a `DeclaredResource` with `declared_by` → passes
- Query resources by epistemic status via EigenQL

---

## Step 7: Azure deployment

**Goal:** Deploy kernel + orchestrator to Azure ContainerApps.

Steps:
- Update Bicep templates for two container apps (kernel, orchestrator)
- Configure DAPR sidecars for mTLS and service invocation
- Set up GitHub Actions workflow: build containers → push to ACR → deploy to staging
- API key management via Azure Key Vault
- Health check probes on both services

```
┌──────────────────────────────────────────────────────────────┐
│  Azure ContainerApps Environment (DAPR-enabled)               │
│                                                                │
│  ┌───────────────────┐           ┌───────────────────────┐    │
│  │  Kernel Service    │◄──DAPR───►  Orchestrator Service  │    │
│  │  (Rust)            │           │  (Deno)               │    │
│  │  gRPC :50051       │           │  MCP :3000            │    │
│  └────────┬───────────┘           └───────────────────────┘    │
│           │                                                    │
│  ┌────────▼───────────┐                                       │
│  │  RocksDB volume     │                                       │
│  └────────────────────┘                                       │
└──────────────────────────────────────────────────────────────┘
```

Testing:
- Docker build succeeds for both images
- Containers start and pass health checks
- CLI connects to deployed endpoint: `eigenius --endpoint https://staging.eigenius.app:50051 inspect "urn:eigenius:core:Class"`

---

## Step 8: End-to-end demo

**Goal:** A complete demo: load a document, run a program that invokes an LLM to analyze it, record reasoning traces, query the results and provenance.

Demo program — summarize a document using CompleteText (Eigon-JSON, per D3 §3):

```json
{
  "@id": "urn:eigenius:demo:summarize",
  "urn:eigenius:core:is_a": ["urn:eigenius:program:Program"],
  "urn:eigenius:program:input_type": "urn:eigenius:demo:Document",
  "urn:eigenius:program:output_type": "urn:eigenius:demo:Summary",
  "urn:eigenius:program:body": {
    "urn:eigenius:core:is_a": ["urn:eigenius:program:Let"],
    "urn:eigenius:program:name": "summary_text",
    "urn:eigenius:program:type": "urn:eigenius:core:string",
    "urn:eigenius:program:value": {
      "urn:eigenius:core:is_a": ["urn:eigenius:program:Apply"],
      "urn:eigenius:program:function": "urn:eigenius:program:components:CompleteText",
      "urn:eigenius:program:argument": {
        "urn:eigenius:core:is_a": ["urn:eigenius:program:Project"],
        "urn:eigenius:program:expression": {
          "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
          "urn:eigenius:program:name": "input"
        },
        "urn:eigenius:program:property": "urn:eigenius:demo:text"
      }
    },
    "urn:eigenius:program:body": {
      "urn:eigenius:core:is_a": ["urn:eigenius:program:Construct"],
      "urn:eigenius:program:class": "urn:eigenius:demo:Summary",
      "urn:eigenius:program:fields": {
        "urn:eigenius:demo:summary_text": {
          "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
          "urn:eigenius:program:name": "summary_text"
        },
        "urn:eigenius:demo:source": {
          "urn:eigenius:core:is_a": ["urn:eigenius:program:Var"],
          "urn:eigenius:program:name": "input"
        }
      }
    }
  }
}
```

(Phase 4.5 introduces ESL surface syntax that compiles to this form.)
```

Demo script:
```bash
# Load document
eigenius load demo/document.json

# Run analysis program
eigenius run demo/summarize-program.json demo/document.json

# Query results
eigenius query 'MATCH "urn:eigenius:demo:Analysis"(?a) { summary: ?s } RETURN [] { summary: ?s }'

# Query provenance
eigenius query 'USING "urn:eigenius:reflection:ProgramTrace" MATCH ProgramTrace(?t) { total_tokens: ?tokens } RETURN [] { tokens: ?tokens }'

# Inspect a trace
eigenius inspect "urn:eigenius:trace:exec-..."
```

---

## Implementation Order

```
Step 1: Reflection ontology
  │
  └──→ Step 2: Trace recording in program executor
         │
         ├──→ Step 6: Epistemic base class validation
         │
         └──→ Step 3: Orchestrator gRPC client (Connect RPC)
                │
                ├──→ Step 4: LLM adapter components (Vercel AI SDK)
                │      │
                │      └──→ Step 8: End-to-end demo
                │
                ├──→ Step 5: MCP server
                │
                └──→ Step 7: Azure deployment
```

Steps 1-2 are sequential (kernel-side). Step 3 enables Steps 4, 5, 7 (orchestrator-side). Step 8 requires everything.

---

## Estimated Effort

| Step | Description | Effort |
|------|-------------|--------|
| 1 | Reflection ontology | 1 day |
| 2 | Trace recording in executor | 2-3 days |
| 3 | Orchestrator Connect RPC client | 2 days |
| 4 | CompleteText LLM adapter (+ mock for tests) | 1-2 days |
| 5 | MCP server | 1-2 days |
| 6 | Epistemic base class validation | 1 day |
| 7 | Azure deployment | 2-3 days |
| 8 | End-to-end demo | 1 day |
| | **Total** | **~2-3 weeks** |

---

## Key decisions (resolved)

| Question | Decision | Reference |
|----------|----------|-----------|
| Execution architecture | Kernel walks expression tree, dispatches IO to orchestrator | D6 |
| Durability | Traces as memoization, not DAPR workflows | D6 |
| Trace schema | Tree-structured, mirrors expression types | D6b |
| Epistemic model | Four levels: declared → observed → derived → verified | D6b |
| Epistemic enforcement | Base classes via `is_a` | D6b |
| Orchestrator ↔ kernel | Connect RPC (gRPC-compatible, Deno-native) | D5, D6 |
| LLM SDK | Vercel AI SDK | Phase 4 plan |
| MCP transport | SSE/HTTP (remote), stdio (local) | Phase 4 plan |
| DAPR | Service glue for Azure deployment; not required for dev | D6 |

---

## New files

```
ontologies/reflection/
  reflection-ontology.json         Trace classes + epistemic base classes

kernel/src/program/
  trace.rs                         TraceStore trait + trace recording logic

orchestration/src/
  components/
    complete_text.ts               CompleteText handler (Vercel AI SDK)
    complete_json.ts               CompleteJson handler (Vercel AI SDK)
    registry.ts                    Component handler registry
  mcp/
    server.ts                      MCP server (rewritten from stub)
  client/
    kernel_client.ts               Connect RPC client (rewritten from stub)
```

## What changes

- `kernel/src/bootstrap/mod.rs` — load reflection ontology as third layer
- `kernel/src/program/execute.rs` — return traces from evaluation
- `kernel/src/validation/mod.rs` — enforce epistemic base class requirements
- `cli/src/main.rs` — add `reflect` command
- `orchestration/deno.json` — add AI SDK and Connect RPC dependencies
- `deploy/` — update Bicep templates, add CI/CD workflow
