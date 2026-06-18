# D6: Execution Architecture and Durability

*Design document for the Eigenius project — April 2026*

**Status:** Implemented (Phase 4)
**Required before:** Phase 4 implementation
**Resolves:** Kernel↔orchestrator boundary, component dispatch, durability through traces, DAPR as service glue

---

## 1. Overview

The kernel executes programs by walking the expression tree. Pure expressions evaluate locally. IO component applications dispatch to the orchestrator. Durability arises from reasoning traces — if a step's trace already exists with the same inputs, the traced result is used without re-execution.

### 1.1 Design principles

**The kernel executes programs.** The kernel walks the expression tree, evaluates pure expressions, and dispatches IO components to the orchestrator. There is no separate "execution plan" — the program *is* the plan.

**Traces are checkpoints.** Each completed component call produces a reasoning trace stored in the kernel. On restart or re-evaluation, existing traces provide results without re-execution. This is memoization driven by the NbE evaluation model.

**The orchestrator executes components.** The orchestrator receives individual component invocation requests and returns results. It uses whatever capabilities it needs internally — Vercel AI SDK, agents, HTTP, DAPR workflows for complex multi-step components — but from the kernel's perspective, it's a function call.

**DAPR is service glue.** DAPR provides service invocation, mTLS, and observability between the kernel and orchestrator. It is not a workflow engine for program execution — that role belongs to the kernel's expression evaluator with trace-based durability.

---

## 2. Process Architecture

```
┌──────────────────────┐           ┌─────────┐           ┌──────────────────────────┐
│  Kernel (Rust)        │◄─────────│  DAPR    │──────────►│  Orchestrator (Deno)      │
│                       │──────────►│ sidecars │◄──────────│                           │
│  • Program evaluation  │           │          │           │  • LLM adapters           │
│  • EigenTT type check  │  DAPR     │ • mTLS   │  DAPR     │    (Vercel AI SDK)        │
│  • Trace-based caching │  service  │ • Trace  │  service  │  • Agent framework        │
│  • Reasoning traces    │  invoke   │ • PubSub │  invoke   │  • MCP server             │
│  • EigenQL queries     │           │          │           │  • HTTP integrations      │
│  • RocksDB storage     │           │          │           │  • Custom components      │
│  • gRPC API (external) │           │          │           │                           │
└──────────────────────┘           └─────────┘           └──────────────────────────┘
```

DAPR provides:
- **Service invocation** — kernel↔orchestrator calls with discovery, load balancing, retries
- **mTLS** — automatic encrypted communication (solves the auth question)
- **Observability** — distributed traces exported to OpenTelemetry
- **Pub/sub** — kernel publishes events (layer committed, program completed)

The orchestrator is free to use any additional infrastructure internally — DAPR workflows for multi-step agents, external databases, third-party APIs — but the kernel sees only the component invocation boundary.

---

## 3. Program Execution

### 3.1 The kernel walks the expression tree

The kernel evaluates the program expression by expression. At each node:

| Expression | Action |
|-----------|--------|
| `Let` | Evaluate value, bind result, continue to body |
| `Apply` (pure component) | Execute locally in kernel |
| `Apply` (io component) | Dispatch to orchestrator, await result |
| `Var` | Look up in current bindings |
| `Case` | Evaluate scrutinee, match branch |
| `Construct` | Build resource from computed values |
| `Project` | Access property on computed resource |
| `Map` | Dispatch elements (parallel for io, local for pure) |
| `Reduce` | Sequential fold |
| `Lambda` | Create closure |
| `Literal` | Return value |

**Before dispatching any io component**, the kernel checks: does a valid trace already exist for this exact call?

### 3.2 Trace-based durability

```
Kernel evaluates: Apply(CompleteJson, input, argument)

1. Compute trace key:
   hash(component_iri, canonicalize(input), canonicalize(argument))

2. Check trace store:
   trace = lookup_trace(trace_key)

3a. If trace exists and is valid:
    → Use trace.output (no orchestrator call)
    → This is NbE: a known value, not a neutral term

3b. If no trace:
    → Dispatch to orchestrator
    → Receive result + component metrics
    → Store trace:
        trace_key → {
          output: result,
          provider: "anthropic",
          model: "claude-sonnet-4-6",
          prompt_tokens: 450,
          completion_tokens: 120,
          latency_ms: 1200,
          timestamp: "2026-04-12T14:30:01Z"
        }
    → Use result, continue evaluation
```

### 3.3 What this gives you

**Durability.** If the kernel crashes after step 2 of 4, restart re-evaluates the program. Steps 1-2 find existing traces → instant. Steps 3-4 dispatch to orchestrator → execute. No DAPR workflow state needed for program-level checkpointing.

**Caching.** Re-run the same program with the same input → all traces exist → instant result, zero API calls.

**Incremental re-evaluation.** Change one component's prompt → only that step re-executes. All other steps use existing traces.

**Partial evaluation connection.** In EigenTT terms: a trace is a known value. A missing trace is a neutral term (blocked on an unknown). The NbE evaluator naturally handles both — it reduces known values and preserves neutral terms. Trace-based execution is partial evaluation where "known" means "previously computed and traced."

### 3.4 Trace invalidation

A trace is valid when:
- The component IRI matches
- The canonicalized input (CBOR deterministic encoding) matches
- The canonicalized argument matches
- The component's ontology definition hasn't changed (same version)

If any of these change, the trace is stale and the component re-executes.

**Non-deterministic components.** Components marked `deterministic: false` (like LLM calls) can still be cached — the cache answers "what did this exact call produce last time?" Whether to re-execute is a policy decision:
- **Default:** use cached result (fast, reproducible)
- **Force re-execute:** ignore cache (fresh LLM response)
- **TTL-based:** re-execute if trace is older than N hours

### 3.5 Parallelism

The kernel identifies independent expressions and dispatches them concurrently:

```
Let parties = CompleteJson(input.letter, prompt1) in   ← IO
Let facts = CompleteText(input.letter, prompt2) in     ← IO (independent of parties)
Let response = CompleteText({parties, facts}, prompt3) ← IO (depends on both)
Construct { parties, facts, response }                 ← pure
```

The kernel:
1. Sees `parties` and `facts` are independent (neither references the other)
2. Dispatches both to orchestrator concurrently (two DAPR calls in parallel)
3. Awaits both results
4. Dispatches `response` (depends on `parties` and `facts`)
5. Awaits result
6. Evaluates `Construct` locally

For each dispatch, the trace check happens first — if both `parties` and `facts` have valid traces, no orchestrator calls are made.

---

## 4. Component Dispatch

### 4.1 Where components execute

| Capability Level | Executor | Trace check | Dispatch |
|-----------------|----------|-------------|----------|
| `pure` | Kernel | No (deterministic, fast) | Local function call |
| `read` | Kernel | No (depends on layer state) | Local with layer access |
| `io` | Orchestrator | Yes (may be cached) | DAPR service invocation |

### 4.2 Kernel → orchestrator call

```protobuf
// The orchestrator exposes this service
service ComponentExecutor {
  rpc Execute(ComponentRequest) returns (ComponentResponse);
}

message ComponentRequest {
  string component_iri = 1;
  bytes input = 2;            // CBOR
  bytes argument = 3;         // CBOR
  string execution_id = 4;   // For correlation
}

message ComponentResponse {
  bool success = 1;
  bytes output = 2;           // CBOR
  string error = 3;
  ComponentMetrics metrics = 4;
}

message ComponentMetrics {
  string provider = 1;
  string model = 2;
  uint64 prompt_tokens = 3;
  uint64 completion_tokens = 4;
  uint64 latency_ms = 5;
}
```

The kernel makes this call via DAPR service invocation. From the kernel's perspective, it's a function call that takes input and returns output with metrics.

### 4.3 Orchestrator component handlers

The orchestrator registers handlers for each component:

```typescript
const handlers: Record<string, ComponentHandler> = {
    "urn:eigenius:program:components:CompleteText": async (input, argument) => {
        const params = extractRequestParams(argument);
        const result = await generateText({
            model: getProvider(params.model),
            prompt: formatPrompt(argument, input),
            temperature: params.temperature,
        });
        return {
            output: wrapAsResource(result.text),
            metrics: {
                provider: result.provider,
                model: params.model,
                promptTokens: result.usage.promptTokens,
                completionTokens: result.usage.completionTokens,
            },
        };
    },

    "urn:eigenius:program:components:CompleteJson": async (input, argument) => {
        // Similar, using generateObject for structured output
    },

    "urn:eigenius:program:components:HttpRequest": async (input, argument) => {
        const response = await fetch(argument.url, { ... });
        return { output: wrapAsResource(await response.json()), metrics: {} };
    },
};
```

The orchestrator is free to use any capabilities internally:
- **Vercel AI SDK** for LLM calls
- **DAPR workflows** for multi-step agent interactions
- **External databases** for domain-specific lookups
- **Third-party APIs** for integrations

From the kernel's perspective, each is just `Execute(component, input, argument) → output`.

### 4.4 Agent components

An agent is a component whose implementation involves multiple LLM calls with tool use. The orchestrator can implement this however it wants — including DAPR workflows for durability within the agent:

```typescript
handlers["urn:eigenius:program:components:ResearchAgent"] = async (input, argument) => {
    // This might use DAPR workflow internally for multi-turn durability
    const agent = createAgent({
        model: anthropic("claude-sonnet-4-6"),
        tools: {
            query: async (eigenql: string) => {
                // Call back to kernel via DAPR
                return await kernelClient.query(eigenql);
            },
        },
    });
    const result = await agent.run(formatPrompt(argument, input));
    return { output: wrapAsResource(result), metrics: collectMetrics(result) };
};
```

The kernel sees this as a single component call. The multi-turn agent logic, the internal DAPR workflow, the tool callbacks — all hidden behind the `Execute` boundary.

---

## 5. Reasoning Traces

### 5.1 Trace structure

Each completed component call produces a trace stored as an Eigon resource:

```json
{
  "@id": "urn:eigenius:trace:<hash>",
  "urn:eigenius:core:is_a": ["urn:eigenius:reflection:ComponentTrace"],
  "urn:eigenius:reflection:component": "urn:eigenius:program:components:CompleteText",
  "urn:eigenius:reflection:input_hash": "<cbor-hash-of-input>",
  "urn:eigenius:reflection:argument_hash": "<cbor-hash-of-argument>",
  "urn:eigenius:reflection:output": { ... embedded resource ... },
  "urn:eigenius:reflection:provider": "anthropic",
  "urn:eigenius:reflection:model": "claude-sonnet-4-6",
  "urn:eigenius:reflection:prompt_tokens": 450,
  "urn:eigenius:reflection:completion_tokens": 120,
  "urn:eigenius:reflection:latency_ms": 1200,
  "urn:eigenius:reflection:timestamp": "2026-04-12T14:30:01Z",
  "urn:eigenius:reflection:deterministic": false
}
```

### 5.2 Program-level trace

After all steps complete, the kernel assembles a program-level trace:

```json
{
  "@id": "urn:eigenius:trace:exec-abc123",
  "urn:eigenius:core:is_a": ["urn:eigenius:reflection:ProgramTrace"],
  "urn:eigenius:reflection:program": "urn:eigenius:example:workers-comp",
  "urn:eigenius:reflection:input_hash": "<hash>",
  "urn:eigenius:reflection:steps": [
    "urn:eigenius:trace:<hash1>",
    "urn:eigenius:trace:<hash2>",
    "urn:eigenius:trace:<hash3>"
  ],
  "urn:eigenius:reflection:total_tokens": 4020,
  "urn:eigenius:reflection:total_latency_ms": 7800,
  "urn:eigenius:reflection:cached_steps": 0,
  "urn:eigenius:reflection:executed_steps": 3,
  "urn:eigenius:reflection:started_at": "2026-04-12T14:30:00Z",
  "urn:eigenius:reflection:completed_at": "2026-04-12T14:30:08Z"
}
```

### 5.3 Traces are queryable

```
USING "urn:eigenius:reflection:ComponentTrace"
MATCH ComponentTrace(?t) {
    component: ?comp,
    model: ?model,
    prompt_tokens: ?tokens
}
WHERE ?tokens > 500
RETURN [] { component: ?comp, model: ?model, tokens: ?tokens }
ORDER BY ?tokens DESC
```

"Which LLM calls consumed the most tokens?" "How many calls used claude-sonnet vs gpt-4o?" "What's the cache hit rate for this program?" — all EigenQL queries over trace resources.

---

## 6. MCP Server

The MCP server runs in the orchestrator and exposes kernel operations as tools:

| Tool | Implementation | Description |
|------|---------------|-------------|
| `eigenius_query` | Kernel → DAPR → orchestrator callback | Query the knowledge graph |
| `eigenius_load` | Kernel → DAPR | Load resources |
| `eigenius_inspect` | Kernel → DAPR | Look up resource by IRI |
| `eigenius_validate` | Kernel → DAPR | Type-check a program |
| `eigenius_run` | Kernel evaluates, dispatches IO to orchestrator | Execute a program |

---

## 7. Deployment

### 7.1 With DAPR (production)

```
┌──────────────────────────────────────────────────────────────┐
│  Azure ContainerApps Environment (DAPR-enabled)               │
│                                                                │
│  ┌───────────────────┐           ┌───────────────────────┐    │
│  │  Kernel Service    │◄──DAPR───►  Orchestrator Service  │    │
│  │  (Rust + DAPR)     │           │  (Deno + DAPR)        │    │
│  │  gRPC :50051       │           │  MCP :3000            │    │
│  └────────┬───────────┘           └───────────────────────┘    │
│           │                                                    │
│  ┌────────▼───────────┐                                       │
│  │  RocksDB volume     │                                       │
│  │  (layers, resources,│                                       │
│  │   traces)           │                                       │
│  └────────────────────┘                                       │
└──────────────────────────────────────────────────────────────┘
```

### 7.2 Without DAPR (development)

```
┌──────────────────┐         ┌───────────────────────┐
│  Kernel Service    │◄─gRPC──►  Orchestrator Service  │
│  (Rust)            │         │  (Deno)               │
└────────┬───────────┘         └───────────────────────┘
         │
  ┌──────▼──────┐
  │  RocksDB     │
  └─────────────┘
```

---

## 8. Decisions Log

| Question | Decision | Rationale |
|----------|----------|-----------|
| Program execution | Kernel walks expression tree directly | No separate execution plan; the program is the plan |
| Durability | Reasoning traces as memoization | Trace exists → use result; no trace → execute. Natural extension of NbE partial evaluation |
| DAPR role | Service glue (mTLS, discovery, observability) | Not a workflow engine for programs; traces provide durability |
| DAPR workflows | Within-component only (e.g., multi-step agents) | Orchestrator's internal concern; kernel sees single Execute call |
| Activity dispatch | `pure`/`read` → kernel; `io` → orchestrator via DAPR | Pure stays local; IO dispatches with trace check |
| Trace caching | Deterministic by default; force-refresh option | Avoid redundant LLM calls; policy-configurable |
| Trace invalidation | Input hash + argument hash + component version | Any change → re-execute |
| Reasoning traces | Kernel-owned, stored in RocksDB as Eigon resources | Queryable via EigenQL; component metrics from orchestrator |
| Parallelism | Kernel identifies independent expressions, dispatches concurrently | Trace check before each dispatch; cached steps are instant |
| Orchestrator internals | Orchestrator's choice (AI SDK, agents, DAPR, databases) | Kernel sees only Execute boundary |
