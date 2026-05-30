# D22: Notebook UX and TypeScript SDK

*Design document for the Eigenius project — April 2026*

**Status:** Implemented (Phase D22; notebook + @eigenius/client shipped)
**Required before:** Phase 12 (Worked Institution Examples) gains a notebook surface
**Resolves:** Browser-side notebook architecture, TypeScript SDK shape, Connect-RPC API, orchestrator-side notebook routes, cell-execution model, notebook persistence format

---

## 1. Overview

This document specifies a **browser-based notebook UX** for Eigenius and the **TypeScript SDK** that powers it. The notebook lets researchers and ontology authors interact with the platform beyond what the CLI supports — tabular query results with sorting and filtering, charts and statistical plots, knowledge-graph topology visualisation, editable program cells with re-execution, and shareable notebook documents.

The SDK is the foundation: a TypeScript client for the Eigenius platform that any browser-side or Deno-side TypeScript code can consume. The notebook app is the first significant consumer.

### 1.1 Audience

- **Notebook consumers** — researchers, ontology authors, anyone using the notebook UI to explore loaded layers, run queries, execute programs, and visualise results.
- **SDK consumers** — TypeScript / JavaScript developers building any browser-side or Deno-side tool against the kernel: the notebook app today, dashboards or custom UIs in the future.
- **Implementers** — contributors building the SDK, the orchestrator's Connect endpoints, and the notebook app itself.

### 1.2 Relationship to other documents

- [**D5 — gRPC API specification**](d5-grpc-api-specification.md) defines the kernel's RPC surface. The notebook does **not** call the kernel directly; it goes through the orchestrator. The SDK consumes a Connect-RPC surface that mirrors most of D5 plus a small set of notebook-specific extensions.
- [**D6 — Execution architecture**](d6-execution-architecture.md) defines the kernel ↔ orchestrator boundary. This document extends the orchestrator with a new browser-facing surface (`/notebook.v1.NotebookService/*`) alongside the existing IO-component dispatch and MCP server surfaces.
- [**D2 — EigenQL specification**](d2-eigenql-specification.md) and [**D7 — ESL surface syntax**](d7-esl-surface-syntax.md) define the languages the notebook's cells contain.
- [**D1 — Eigon serialization format**](d1-eigon-serialization-format.md) defines the on-wire resource format the SDK translates to and from TypeScript values.

### 1.3 Non-goals

- **Reimplementing surface-language tooling.** The notebook app does not validate or compile ESL/EigenQL itself; it sends source text to the orchestrator and renders the result. Syntax highlighting in the editor is purely lexical.
- **Multi-user collaboration.** The MVP targets a single user editing one notebook in a single browser tab. Shared editing, presence, comments, etc. are deferred.
- **Server-side notebook persistence.** Notebooks are saved to and loaded from local files. Server-side storage is deferred until the use case appears.
- **Authentication and authorisation.** The MVP assumes a trusted single-user environment (typically `localhost`). Production-quality auth is deferred.
- **A complete reactive cell DAG across all cell types.** The MVP uses manual cell execution. Reactivity arrives in Phase 6, scoped initially to TypeScript cells.

---

## 2. Architecture

### 2.1 Three-tier topology

```
┌─────────────────────────────────────────────────┐
│  Notebook app — TypeScript + React (browser)    │
│  - CodeMirror 6 cell editors                    │
│  - Cell execution UI (manual MVP)               │
│  - Fluent UI v9 (DataGrid, Charts, layout)      │
│  - Custom JSON notebook format                  │
└────────────────────────┬────────────────────────┘
                         │  Connect-RPC over HTTP/1.1
                         │  via @connectrpc/connect-web
                         ▼
┌─────────────────────────────────────────────────┐
│  Orchestrator — Deno / TypeScript               │
│  - Existing Connect server                       │
│  - + new NotebookService routes                  │
│  - Existing component dispatch + MCP server      │
└────────────────────────┬────────────────────────┘
                         │  gRPC over HTTP/2
                         │  via @grpc/grpc-js (existing)
                         ▼
┌─────────────────────────────────────────────────┐
│  Kernel — Rust (tonic gRPC, unchanged surface)   │
│  + small new helper for layer topology walking   │
└─────────────────────────────────────────────────┘
```

The kernel's gRPC surface stays unchanged for the MVP, with one exception: a new `LayerTopology` helper is added (§4.2). Every other notebook RPC delegates to an existing kernel call.

### 2.2 Why orchestrator-mediated, not direct gRPC-Web

Browsers cannot speak vanilla gRPC: the HTTP/2 trailer-frame requirement isn't exposed by the Fetch / XHR APIs. Two viable options were considered:

1. **gRPC-Web** — adds a `tonic-web` layer to the kernel (or runs an Envoy proxy in front of it). Browser clients use `@improbable-eng/grpc-web` or `@connectrpc/connect-web` against the kernel directly.
2. **Connect-RPC via the orchestrator** — adds a new Connect surface to the orchestrator that proxies kernel calls. Browser clients use `@connectrpc/connect-web` against the orchestrator.

This document specifies **option 2**. Reasoning:

- The orchestrator already runs Connect-RPC for component dispatch ([D6](d6-execution-architecture.md)). Adding a notebook-facing service is a natural extension — same protocol, same server, different routes.
- The orchestrator is already the "browser-facing" tier — it hosts the MCP server, the LLM adapter, and the `/health` endpoint. Notebooks fit the same model.
- Keeps the kernel's gRPC surface focused on machine-to-machine traffic.
- Single-origin deployment becomes easy: serve the notebook static bundle from the same orchestrator that serves the API. No CORS configuration.
- Adds at most a few milliseconds of localhost-loopback latency per call, which is negligible against typical kernel query times.

The cost is one extra hop per call. Acceptable.

### 2.3 Single-origin deployment

For the MVP, the orchestrator serves the notebook's built static bundle at `/notebooks/*` and the Connect API at `/notebook.v1.NotebookService/*`. Both come from the same origin, eliminating CORS. The notebook app reads its own origin to construct API URLs:

```typescript
const transport = createConnectTransport({
  baseUrl: window.location.origin,
});
```

For deployments where the notebook is hosted separately (a CDN, GitHub Pages, etc.), the orchestrator must opt into CORS for the notebook origin. The MVP does not require this.

### 2.4 Network ports

Unchanged from existing deployment:

| Port | Service | Notes |
|---|---|---|
| 50051 | Kernel gRPC | Internal; orchestrator calls in |
| 8080 | Orchestrator HTTP | Now also serves notebook static bundle and `NotebookService` |

---

## 3. Connect API

### 3.1 Proto file location

All Eigenius protobufs live in a single flat file: **`proto/eigenius.proto`** (`package eigenius.v1`). The notebook additions extend this file in place rather than introducing a versioned-package layout. Reorganising into `eigenius.kernel.v1` / `eigenius.notebook.v1` namespaces is a worthwhile future cleanup but out of scope for the notebook MVP — it would touch every consumer (kernel `tonic_build`, orchestrator buf-generated TS, every existing import) and gains nothing the notebook needs.

### 3.2 Service definition

The kernel's existing `EigeniusKernel` service already provides almost every RPC the notebook needs (`Load`, `Inspect`, `Query`, `RunProgram`, `ValidateProgram`, `ListInstitutions`, `GetSchema`, etc.). The notebook reuses these directly via Connect-Web through the orchestrator.

The notebook adds **two things** to the proto:

1. A new `LayerTopology` RPC on the existing `EigeniusKernel` service — the only kernel-level operation genuinely needed by the notebook that doesn't already exist.
2. A new `NotebookService` containing browser-specific RPCs that don't fit the kernel's machine-to-machine surface. Initially it has only `LayerTopology` (proxying to the kernel) as a placeholder for the browser-specific surface area; further RPCs join it as concrete needs emerge during Phases 3–5.

```proto
// Added to the existing EigeniusKernel service:
rpc LayerTopology(LayerTopologyRequest) returns (LayerTopologyResponse);

// New service in the same proto file:
service NotebookService {
  rpc LayerTopology(LayerTopologyRequest) returns (LayerTopologyResponse);
  // Future browser-specific methods append here as needs surface.
}
```

The browser uses both services from a single Connect-Web transport pointed at the orchestrator. Routes are cleanly partitioned: existing kernel surface → `EigeniusKernel.*`, browser-specific additions → `NotebookService.*`.

The duplication of `LayerTopology` between the two services is intentional and inexpensive: the kernel needs it for itself (other tools, the CLI, future Rust consumers), and the orchestrator-side `NotebookService.LayerTopology` is a thin proxy that lets the browser-facing surface evolve independently of kernel-internal API decisions. Both methods share the same request/response message types.

All unary; no streaming in the MVP (deferred — see §8.2).

The remaining "notebook RPCs" originally proposed in earlier drafts of this document — `Inspect`, `Query`, `Load`, `Compile`, `Run`, `ListInstitutions` — **are not new**. The browser uses the existing `EigeniusKernel` versions of these directly. The TypeScript SDK presents a uniform `Eigen` class that wraps both services so consumers don't have to think about which service a method lives on.

### 3.3 Per-RPC specification

#### `Inspect` — fetch a resource by IRI

```proto
message InspectRequest {
  string iri = 1;
  optional string at_layer = 2;  // hex LayerId; defaults to the active top
}

message InspectResponse {
  Resource resource = 1;
}
```

Errors:

- `NOT_FOUND` — IRI doesn't resolve in the layer chain
- `INVALID_ARGUMENT` — malformed IRI
- `INVALID_ARGUMENT` — `at_layer` doesn't parse as a LayerId

#### `Query` — execute an EigenQL query

```proto
message QueryRequest {
  string eigenql = 1;
  optional string at_layer = 2;
}

message QueryResponse {
  ResultSet result = 1;
}

message ResultSet {
  // The synthesized row class for this query, with its declared properties.
  Resource result_class = 1;
  repeated Resource properties = 2;
  // Each row is an embedded Resource whose properties carry the
  // synthesized row-property IRIs as keys (per D2 Appendix A).
  repeated Resource rows = 3;
  uint64 row_count = 4;
  bool matched = 5;
}
```

Errors:

- `INVALID_ARGUMENT` — parse or type-check failure (the message body carries the kernel's `QueryError` rule and message)
- `FAILED_PRECONDITION` — stratification failure
- `INTERNAL` — evaluation runtime error

#### `Load` — load Eigon-JSON or ESL files into a new layer

```proto
message LoadRequest {
  message File {
    string name = 1;       // Original filename, used to detect ESL vs JSON via extension
    bytes content = 2;     // UTF-8 source for ESL; UTF-8 JSON or CBOR for Eigon
    string content_type = 3;  // "application/json", "application/eigon-cbor", "application/esl"
  }
  repeated File files = 1;
}

message LoadResponse {
  string layer_id = 1;       // hex LayerId of the newly created top
  uint64 resource_count = 2; // total resources committed
}
```

Errors:

- `INVALID_ARGUMENT` — parse failure for any file
- `FAILED_PRECONDITION` — validation failure (with rule + resource IRI in message)

#### `Compile` — compile ESL to Eigon-JSON without loading

```proto
message CompileRequest {
  string esl = 1;
}

message CompileResponse {
  repeated Resource resources = 1;
}
```

Errors:

- `INVALID_ARGUMENT` — ESL parse / compile failure

#### `Run` — execute a typed program against an input

```proto
message RunRequest {
  // Either supply a program by IRI (already loaded) or by source...
  oneof program {
    string program_iri = 1;
    string program_source = 2;  // ESL or Eigon-JSON
  }
  // ...and either an input resource by IRI or by inline bytes.
  oneof input {
    string input_iri = 3;
    InlineInput inline_input = 4;
  }

  message InlineInput {
    bytes content = 1;
    string content_type = 2;
  }

  optional string at_layer = 5;
}

message RunResponse {
  Resource output = 1;
  // Optional embedded trace for the notebook's trace-tree visualisation.
  // Empty when the kernel has no trace store configured.
  optional Resource trace = 2;
}
```

Errors:

- `INVALID_ARGUMENT` — type-check or compile failure
- `FAILED_PRECONDITION` — required IO components not registered
- `INTERNAL` — runtime evaluation error or component dispatch failure

#### `LayerTopology` — return the layer chain as a graph for visualisation

```proto
message LayerTopologyRequest {
  optional string root_layer = 1;  // defaults to the active top
  uint32 max_depth = 2;            // default 100; 0 = unlimited
  bool include_resources = 3;      // include per-resource nodes (default false; layers-only)
}

message LayerTopologyResponse {
  repeated TopologyNode nodes = 1;
  repeated TopologyEdge edges = 2;
}

message TopologyNode {
  string id = 1;            // IRI for resource nodes; LayerId for layer nodes
  NodeKind kind = 2;
  string label = 3;         // short_name or fallback
  map<string, string> attrs = 4;  // free-form metadata for renderer
}

enum NodeKind {
  NODE_KIND_UNSPECIFIED = 0;
  NODE_KIND_LAYER = 1;
  NODE_KIND_CLASS = 2;
  NODE_KIND_PROPERTY = 3;
  NODE_KIND_RESOURCE = 4;
  NODE_KIND_INSTITUTION = 5;
}

message TopologyEdge {
  string source = 1;
  string target = 2;
  EdgeKind kind = 3;
  map<string, string> attrs = 4;
}

enum EdgeKind {
  EDGE_KIND_UNSPECIFIED = 0;
  EDGE_KIND_PARENT_LAYER = 1;
  EDGE_KIND_IS_A = 2;
  EDGE_KIND_SUBCLASS_OF = 3;
  EDGE_KIND_REQUIRES = 4;
  EDGE_KIND_RECOMMENDS = 5;
  EDGE_KIND_PROPERTY_REF = 6;
  EDGE_KIND_INSTITUTION_DECLARES = 7;
}
```

This is the only RPC requiring new kernel-side code (§4.2). The shape — flat node and edge lists with kind enums and free-form attribute maps — is designed for direct consumption by any modern graph-rendering library (the MVP uses these for `LayerStackView`'s box-and-counts rendering; Phase 5's full-topology view consumes them via `@xyflow/react`).

#### `ListInstitutions` — list registered institutions

```proto
message ListInstitutionsRequest {}

message ListInstitutionsResponse {
  repeated InstitutionInfo institutions = 1;
}

message InstitutionInfo {
  string iri = 1;
  string name = 2;
  repeated string morphism_type_iris = 3;
  repeated string query_type_iris = 4;
  repeated string comorphism_iris = 5;
  repeated string decide_procedure_iris = 6;
}
```

Direct mirror of the kernel's `InstitutionInfo` struct.

### 3.4 Error model

Connect's error model maps gRPC status codes to HTTP status codes. The notebook SDK exposes errors as a typed `EigeniusError` class (§5.5) carrying the Connect code, the kernel-side `rule` identifier, and the human-readable message. Consumers can `instanceof`-discriminate or switch on `error.rule`.

Kernel error rules surface verbatim in `error.rule` so the notebook UI can react programmatically:

```typescript
try {
  await eigen.query("...");
} catch (err) {
  if (err instanceof EigeniusError && err.rule === "stratification") {
    showStratificationGuidance();
  }
}
```

### 3.5 Compatibility with existing proto

The single `proto/eigenius.proto` already defines the message types the notebook needs (`Resource`, `Value`, etc.). No changes to those types are required.

The proto gains:
- One new RPC on the existing `EigeniusKernel` service: `LayerTopology` — see §4.2.
- One new service definition: `NotebookService`, with `LayerTopology` as its initial (and only) method.
- Three new message types: `LayerTopologyRequest`, `LayerTopologyResponse`, plus the supporting `TopologyNode` / `TopologyEdge` and their enums (`NodeKind`, `EdgeKind`).

The existing TS stub generation (`buf generate` per the existing `buf.gen.yaml`) picks all of this up automatically. The new `clients/eigenius-ts/` SDK adds itself as a second `protoc-gen-es` output target in `buf.gen.yaml`, sharing the single `buf generate` invocation.

---

## 4. Orchestrator handlers

### 4.1 Module layout

```
orchestration/src/
├── notebook/
│   ├── service.ts         # NotebookService implementation, registered on the Connect server
│   ├── inspect.ts         # one file per RPC handler
│   ├── query.ts
│   ├── load.ts
│   ├── compile.ts
│   ├── run.ts
│   ├── topology.ts
│   ├── institutions.ts
│   └── errors.ts          # kernel error → Connect error mapping
└── server/
    └── mod.ts             # imports NotebookService and registers it
```

Each handler is a thin function that:

1. Validates the request shape (Connect handles proto-level validation; handlers add semantic checks).
2. Calls the existing kernel-gRPC client (`KernelClient` in `orchestration/src/client/kernel_client.ts`).
3. Maps the kernel's gRPC response back into the notebook proto's response shape.
4. Catches kernel errors and re-raises as Connect errors with appropriate codes and `rule` strings.

For example, `query.ts` skeletally:

```typescript
import { ConnectError, Code } from "@connectrpc/connect";
import type { QueryRequest, QueryResponse } from "../gen/notebook_pb.ts";
import { kernelClient } from "../client/kernel_client.ts";

export async function handleQuery(req: QueryRequest): Promise<QueryResponse> {
  if (!req.eigenql.trim()) {
    throw new ConnectError("eigenql is empty", Code.InvalidArgument);
  }
  try {
    const kernelResult = await kernelClient.executeQuery({
      query: req.eigenql,
      atLayer: req.atLayer,
    });
    return { result: mapResultSet(kernelResult) };
  } catch (e) {
    throw mapKernelError(e);  // see errors.ts
  }
}
```

### 4.2 The `LayerTopology` kernel helper

This is the one new kernel-side capability the notebook needs. It walks the layer chain and produces nodes/edges suitable for graph rendering.

Implementation lives in `kernel/src/server/topology.rs` (new file). The kernel exposes it as a gRPC method on the existing `EigeniusKernel` service:

```proto
// in proto/eigenius.proto (existing service, new method)
rpc LayerTopology(LayerTopologyRequest) returns (LayerTopologyResponse);
```

The kernel's response shape matches the `NotebookService.LayerTopology` response shape field-for-field (both share the `LayerTopologyResponse` message defined in the same proto file), so the orchestrator's `NotebookService.LayerTopology` handler is a thin pass-through to the kernel.

The walker logic:

1. Start at the requested layer (or the active top).
2. Emit a `NODE_KIND_LAYER` node; emit a `EDGE_KIND_PARENT_LAYER` edge to its parent if any.
3. For each resource in the layer:
   - If `include_resources` is false and the resource isn't a Class, Property, or Institution, skip.
   - Emit a node of the appropriate `NodeKind` (derived from `is_a`).
   - For each `is_a`, `subclass_of`, `requires`, `recommends` property whose value is an IRI (or array of IRIs), emit edges with the appropriate `EdgeKind`.
4. Recurse into the parent layer up to `max_depth`.
5. Deduplicate nodes by `id` (a resource appearing in multiple layers via overrides emits one node).

The walker is read-only and runs in the kernel's `Read` capability mode (no IO, no institution dispatch).

### 4.3 Authentication

Deferred for the MVP. The Connect server accepts requests from any origin; the assumption is that the orchestrator runs on `localhost` and is reachable only by the local user. Production deployments must add a reverse proxy or middleware that authenticates requests; the design of that auth layer is out of scope here.

The `NotebookService` is structured so that adding auth interceptors later is straightforward — Connect's middleware model is well-suited to this.

---

## 5. TypeScript SDK

### 5.1 Package layout

```
clients/eigenius-ts/
├── deno.jsonc                # Deno tasks (build, test, publish)
├── package.json              # for npm consumers via dnt
├── jsr.json                  # JSR publication config
├── README.md
├── mod.ts                    # public API exports
├── src/
│   ├── client.ts             # Eigen class — the main entry point
│   ├── resource.ts           # Resource, Value, ValueKind
│   ├── result.ts             # ResultSet, ResultRow
│   ├── layer.ts              # Layer
│   ├── topology.ts           # Topology, TopologyNode, TopologyEdge, NodeKind, EdgeKind
│   ├── institution.ts        # InstitutionInfo
│   ├── errors.ts             # EigeniusError
│   └── transport.ts          # Connect transport configuration helpers
├── generated/                # buf-generated Connect stubs
│   ├── notebook_pb.ts
│   ├── notebook_connect.ts
│   ├── kernel/
│   │   ├── resource_pb.ts
│   │   └── value_pb.ts
└── examples/
    └── smoke-test.ts         # Phase 1 acceptance criterion
```

### 5.2 Distribution

Two-channel publishing:

- **JSR (`@eigenius/client`)** — source of truth. Native TypeScript, native Deno; consumed via `import { Eigen } from "jsr:@eigenius/client"`.
- **npm (`@eigenius/client`)** — mirrored from the JSR source via [`dnt`](https://github.com/denoland/dnt) on each release. Same package name, same version. Consumed by the notebook app and any Node-side consumer via `import { Eigen } from "@eigenius/client"`.

Versioning follows semver. The SDK is `0.x` until the kernel API stabilises; breaking changes are expected and acceptable in this phase.

### 5.3 Public API — the `Eigen` class

```typescript
export interface EigenOptions {
  /** Orchestrator endpoint, e.g. "http://localhost:8080". Required. */
  endpoint: string;
  /** Optional fetch implementation override (defaults to global fetch). */
  fetch?: typeof fetch;
  /** Optional bearer token for future auth. Currently ignored. */
  bearerToken?: string;
}

export class Eigen {
  constructor(options: EigenOptions);

  /** Fetch a resource by IRI. */
  inspect(iri: string, opts?: { atLayer?: string }): Promise<Resource>;

  /** Execute an EigenQL query against the active layer (or `at_layer`). */
  query(eigenql: string, opts?: { atLayer?: string }): Promise<ResultSet>;

  /** Load Eigon-JSON or ESL files; returns the new layer's id. */
  load(files: { name: string; content: string | Uint8Array }[]): Promise<Layer>;

  /** Compile ESL to Eigon-JSON resources without loading. */
  compile(esl: string): Promise<Resource[]>;

  /** Run a typed program. Either programIri or programSource is required;
      either inputIri or inputResource is required. */
  run(opts: {
    programIri?: string;
    programSource?: string;
    inputIri?: string;
    inputResource?: Resource | Uint8Array;
    atLayer?: string;
  }): Promise<RunResult>;

  /** Walk the layer chain and return nodes/edges for visualisation. */
  layerTopology(opts?: {
    rootLayer?: string;
    maxDepth?: number;
    includeResources?: boolean;
  }): Promise<Topology>;

  /** List registered institutions. */
  listInstitutions(): Promise<InstitutionInfo[]>;
}
```

### 5.4 `Resource`, `ResultSet`, `Layer`, `Topology`, `RunResult`

```typescript
export class Resource {
  readonly id: string | undefined;  // may be undefined for embedded resources
  readonly properties: ReadonlyMap<string, Value>;

  /** Convenience accessor for `urn:eigenius:core:is_a`. */
  get isA(): readonly string[];

  /** Get a typed property value, or undefined if absent. */
  get(propertyIri: string): Value | undefined;
  getString(propertyIri: string): string | undefined;
  getInteger(propertyIri: string): number | undefined;
  getFloat(propertyIri: string): number | undefined;
  getBoolean(propertyIri: string): boolean | undefined;
  getArray(propertyIri: string): readonly Value[] | undefined;
  getEmbedded(propertyIri: string): Resource | undefined;

  /** Round-trip as Eigon-JSON. */
  toJSON(): EigonResourceJson;
  static fromJSON(json: EigonResourceJson): Resource;
}

export type Value =
  | { kind: "string"; value: string }
  | { kind: "integer"; value: number }     // 64-bit; truncates above MAX_SAFE_INTEGER
  | { kind: "integer-large"; value: bigint }
  | { kind: "float"; value: number }
  | { kind: "boolean"; value: boolean }
  | { kind: "iri"; value: string }
  | { kind: "array"; values: readonly Value[] }
  | { kind: "embedded"; resource: Resource };

export class ResultSet {
  readonly classMeta: Resource;            // the synthesized row class
  readonly properties: readonly Resource[]; // synthesized Property resources
  readonly rows: readonly Resource[];      // each row's properties keyed by synthesized IRIs
  readonly rowCount: number;
  readonly matched: boolean;

  /** Project rows to plain objects keyed by short_name (from properties metadata). */
  toObjects(): Record<string, unknown>[];

  /** CSV serialization with short_name headers. */
  toCSV(): string;
}

export class Layer {
  readonly id: string;
  readonly resourceCount: number;
}

export class Topology {
  readonly nodes: readonly TopologyNode[];
  readonly edges: readonly TopologyEdge[];

  /** Convenience: return only nodes of given kinds. */
  filterNodes(kinds: NodeKind[]): readonly TopologyNode[];

  /** Convenience: return react-flow nodes / edges directly (Phase 5). */
  toReactFlow(): { nodes: object[]; edges: object[] };
}

export interface TopologyNode {
  readonly id: string;
  readonly kind: NodeKind;
  readonly label: string;
  readonly attrs: Readonly<Record<string, string>>;
}

export interface TopologyEdge {
  readonly source: string;
  readonly target: string;
  readonly kind: EdgeKind;
  readonly attrs: Readonly<Record<string, string>>;
}

export const enum NodeKind {
  Layer = "layer",
  Class = "class",
  Property = "property",
  Resource = "resource",
  Institution = "institution",
}

export const enum EdgeKind {
  ParentLayer = "parent_layer",
  IsA = "is_a",
  SubclassOf = "subclass_of",
  Requires = "requires",
  Recommends = "recommends",
  PropertyRef = "property_ref",
  InstitutionDeclares = "institution_declares",
}

export interface RunResult {
  readonly output: Resource;
  readonly trace: Resource | undefined;  // present only when kernel has a trace store
}
```

### 5.5 Errors

```typescript
export class EigeniusError extends Error {
  readonly code: Code;          // Connect status code (re-exported)
  readonly rule: string;        // kernel-side error rule, e.g. "stratification"
  readonly position?: { line: number; column: number };

  constructor(message: string, code: Code, rule: string, position?: { line: number; column: number });
}
```

The SDK catches Connect-RPC errors and translates them to `EigeniusError` instances with the kernel's rule identifier extracted from the response detail. Consumers can:

```typescript
try {
  await eigen.query("...");
} catch (err) {
  if (err instanceof EigeniusError) {
    console.error(`${err.code}: ${err.rule}: ${err.message}`);
    if (err.position) {
      console.error(`  at ${err.position.line}:${err.position.column}`);
    }
  }
}
```

### 5.6 Eigon-JSON ↔ TypeScript marshalling

The notebook proto's `Resource` and `Value` types mirror Eigon-JSON's structure; the SDK marshals between proto messages and the user-facing `Resource` / `Value` TypeScript types.

Marshalling rules:

| Eigon-JSON value | Proto `Value.kind` | TypeScript `Value` |
|---|---|---|
| `"hello"` | `string_value` | `{ kind: "string", value: "hello" }` |
| `42` | `int_value` | `{ kind: "integer", value: 42 }` |
| `1234567890123456789` | `int_value` (bigint) | `{ kind: "integer-large", value: 1234567890123456789n }` |
| `3.14` | `float_value` | `{ kind: "float", value: 3.14 }` |
| `true` | `bool_value` | `{ kind: "boolean", value: true }` |
| `"urn:..."` (IRI-typed property) | `iri_value` | `{ kind: "iri", value: "urn:..." }` |
| `["a", "b"]` | `array_value` | `{ kind: "array", values: [...] }` |
| `{ "@id": ..., ... }` | `embedded_value` | `{ kind: "embedded", resource: Resource }` |

Integer values larger than `Number.MAX_SAFE_INTEGER` (2⁵³−1) marshal to `bigint` to avoid precision loss. The SDK chooses `integer-large` based on the proto's wire type (whether the kernel sent it as an `int64` requiring bigint representation), not based on the value's magnitude — this is symmetric and round-trippable.

### 5.7 The smoke test (Phase 1 acceptance criterion)

```typescript
// clients/eigenius-ts/examples/smoke-test.ts
import { Eigen } from "../mod.ts";

const eigen = new Eigen({ endpoint: "http://localhost:8080" });

// 1. Inspect a core resource
const cls = await eigen.inspect("urn:eigenius:core:Class");
console.assert(cls.isA.includes("urn:eigenius:core:Class"));

// 2. Run a query
const classes = await eigen.query(`
  USING "urn:eigenius:core:Class"
  MATCH Class(?c) { short_name: ?n }
  RETURN [] { name: ?n }
`);
console.assert(classes.rowCount > 0);

// 3. Compile ESL
const compiled = await eigen.compile(`
  namespace ex = "urn:example";
  class ex:Foo { description = "test"; requires ex:bar; }
  property ex:bar : core:string { description = "bar"; }
`);
console.assert(compiled.length === 2);

// 4. Load + topology
const layer = await eigen.load([
  { name: "patent.esl", content: "..." }
]);
const topo = await eigen.layerTopology({ rootLayer: layer.id });
console.assert(topo.nodes.length > 0);

// 5. List institutions
const insts = await eigen.listInstitutions();
console.log(`${insts.length} institutions registered`);

console.log("✓ smoke test passed");
```

When this script exits 0 against a running orchestrator+kernel, Phase 1 is complete.

---

## 6. Notebook app

### 6.1 Project layout

```
notebooks/
├── deno.jsonc                    # Deno tasks (dev, build, test)
├── package.json                  # Vite + React + dependencies
├── vite.config.ts
├── index.html
├── src/
│   ├── main.tsx                  # React root
│   ├── App.tsx                   # Notebook shell
│   ├── components/
│   │   ├── Notebook.tsx          # Cell list + toolbar
│   │   ├── Toolbar.tsx
│   │   ├── Cell.tsx              # Cell wrapper (header, output area, editor)
│   │   ├── cells/
│   │   │   ├── MarkdownCell.tsx
│   │   │   ├── ESLCell.tsx
│   │   │   ├── EigenQLCell.tsx
│   │   │   └── TypeScriptCell.tsx
│   │   ├── editors/
│   │   │   ├── CodeMirrorEditor.tsx
│   │   │   ├── eigenql-mode.ts   # CodeMirror language support
│   │   │   └── esl-mode.ts
│   │   └── output/
│   │       ├── ResultTable.tsx           # Fluent DataGrid (driven by Property metadata)
│   │       ├── ResourceInspector.tsx     # typed resource view (Fluent Card + DescriptionList)
│   │       ├── ResultPlot.tsx            # @fluentui/react-charts wrapper (Phase 5)
│   │       ├── LayerStackView.tsx        # custom JSX over Fluent primitives (MVP)
│   │       ├── LayerTopologyGraph.tsx    # @xyflow/react full-topology graph (Phase 5)
│   │       └── TraceTree.tsx             # D3 hierarchy wrapper (Phase 5)
│   ├── state/
│   │   ├── notebook-store.ts             # Zustand store
│   │   └── execution.ts                  # cell run state machine
│   ├── runtime/
│   │   ├── eigen-client.ts               # configures the SDK
│   │   ├── ts-cell-runner.ts             # sandbox for TypeScript cells
│   │   └── output-renderer.ts            # auto-render of TS cell return values
│   ├── persistence/
│   │   ├── notebook-format.ts            # types + version migration
│   │   └── file-io.ts                    # browser File API wrappers
│   └── styles/
│       └── app.css
└── README.md
```

### 6.2 Cell types (MVP)

Four cell types in the MVP:

| Type | Source | Output |
|---|---|---|
| **Markdown** | Markdown text | Rendered HTML (`react-markdown` + `remark-gfm`) |
| **ESL** | ESL source — declarations or a program | Either a `Layer` (for declarations / resources) or a `Resource` output (for programs); rendered as inspector |
| **EigenQL** | EigenQL query text | `ResultSet` rendered as `ResultTable` |
| **TypeScript** | TS code with `eigen` SDK and previous cell outputs in scope | Last expression's value, auto-rendered by `output-renderer.ts` |

ESL cells are mode-distinguished at run time:

- If the cell's compiled output contains zero `Program` resources → declarations / resources mode → `Load` is called, output is the resulting `Layer`.
- If exactly one `Program` resource → program mode; the cell needs a separate input source. The MVP requires the user to pre-load the input via a previous cell, then references it by IRI.
- More than one `Program` resource → compile-only mode; output is the resource list (rare; useful for inspecting compilation).

### 6.3 Execution model

**MVP: manual**, with two affordances:

- **Run cell** (per-cell button or `Shift+Enter`) — execute this cell only.
- **Run all** (toolbar) — execute all cells top-to-bottom; halt on the first failing cell.

Each cell has a state machine:

```
idle ──Run──▶ running ──success──▶ done
                       └─error───▶ error
done ──Run──▶ running ...
```

The state lives in the Zustand store (§6.4).

**Phase 6: hybrid reactivity.** TypeScript cells form a JS dataflow graph (analyzed via simple variable-name extraction at parse time). When a TS cell runs, downstream TS cells with affected variables transition to a `stale` state and visually indicate they need re-running. Auto-rerun is opt-in (a per-cell toggle, default off, until the model is proven).

Cross-language reactivity (e.g. "edit an ESL cell that loads an ontology, automatically re-run downstream EigenQL cells") is deferred indefinitely. The cost-benefit isn't clear, and Eigon's load semantics make automatic re-runs risky (each `load` creates a new layer; we'd need a "reset to before this cell" operation).

### 6.4 State management — Zustand

Single store, three top-level slices:

```typescript
interface NotebookState {
  // Document
  cells: Cell[];
  activeCellId: string | null;
  notebookMeta: NotebookMeta;

  // Runtime
  layer: string | null;                    // active layer id
  cellOutputs: Map<string, CellOutput>;
  cellStates: Map<string, CellRunState>;

  // Actions
  addCell: (afterId: string | null, type: CellType) => void;
  deleteCell: (id: string) => void;
  moveCell: (id: string, direction: "up" | "down") => void;
  updateCellSource: (id: string, source: string) => void;
  runCell: (id: string) => Promise<void>;
  runAll: () => Promise<void>;
  loadNotebook: (json: NotebookJson) => void;
  exportNotebook: () => NotebookJson;
}
```

Atom-based state (Jotai) is reserved for Phase 6 if the cell DAG benefits from per-cell reactive subscriptions; for the MVP, Zustand's selector-based subscriptions are sufficient.

### 6.5 Persistence — notebook file format

Custom JSON. Schema:

```typescript
interface NotebookJson {
  format_version: 1;             // bump on breaking changes; SDK provides migrators
  meta: {
    title?: string;
    created?: string;            // ISO 8601
    modified?: string;           // ISO 8601
    eigenius_version?: string;   // platform version when last saved
  };
  cells: CellJson[];
}

interface CellJson {
  id: string;                    // UUID
  type: "markdown" | "esl" | "eigenql" | "typescript";
  source: string;
  // Outputs are NOT persisted in MVP — re-run on load.
  // Future: optionally embed cached outputs.
}
```

File I/O via the browser's File System Access API where available (Chrome, Edge); fallback to `<input type="file">` + `Blob` download for other browsers.

The `format_version` field anchors future migrations. When the SDK sees a notebook with a higher version than it understands, it refuses to open and surfaces the version requirement to the user.

### 6.6 Editor — CodeMirror 6

`@uiw/react-codemirror` wraps CodeMirror 6 in a React component. Per-language modes live in `src/components/editors/`:

- `eigenql-mode.ts` — keyword highlighting (`MATCH`, `WHERE`, `RETURN`, `USING`, `INSTITUTION`, `FIBER`, `DEFINE`, `GROUP`, `BY`, `ORDER`, `ASC`, `DESC`, `LIMIT`, `OFFSET`, `DISTINCT`, `AND`, `OR`, `NOT`, `IN`, `LIKE`, `EXISTS`, `AS`, `FROM`); IRI string highlighting; `?variable` highlighting; built-in functions; line/block comments.
- `esl-mode.ts` — keyword highlighting (`namespace`, `class`, `property`, `resource`, `program`, `data`, `codata`, `let`, `case`, `match`, `returning`, `Construct`, `map`, `reduce`, `corecord`); namespace prefix highlighting (`ex:Foo`); strings; numbers; line/block comments; lambda symbol `λ` and `\`.

These are lexical-only — no semantic completion in the MVP. Adding completion (resolving namespace aliases, suggesting class/property names from the loaded layer) is a future enhancement that requires a query into the layer state on every keystroke.

### 6.7 Output rendering

Cell outputs are rendered by type-discriminated renderers in `src/components/output/`. The MVP needs:

- **`ResultTable`** — Fluent UI v9 `DataGrid` with sortable columns, selectable rows, and built-in virtualisation. Wraps an Eigenius `ResultSet`: column definitions are derived from the synthesized Property resources (D2 Appendix A), which carry both `short_name` (column header) and `data_type` (column type — drives sort comparator, default formatter, alignment). The synthesized Property metadata is the single source of truth; we do not type-sniff from values. Row IRIs come from the row resources themselves; cell values from the row's IRI-keyed properties, projected back to short-names via the Property table.
- **`ResourceInspector`** — table-like view of an Eigon resource: `@id`, `is_a`, then properties grouped by IRI. Embedded resources expand inline. IRI values are clickable (open inspector for that IRI).
- **`LayerStackView`** — plain JSX/CSS layer-chain visualisation. Renders the layer parent chain as a vertical stack of boxes, each labeled with its IRI prefix and per-class/per-property/per-resource/per-institution counts (extracted from a `Topology` value or computed via `eigen.layerTopology()` with `includeResources: false`). Click a box to drill into that layer's contents via the existing `ResourceInspector`. ~80 lines of React with no new dependencies; full intra-layer graph rendering is a Phase 5 concern (see §7).
- **`TraceTree`** — D3-hierarchy-based renderer for program execution traces. Reads the optional `trace` field of a `RunResult` (returned by `eigen.run()` whenever the kernel has a trace store configured) and renders the tree of expression evaluations and IO-component dispatches as an interactive collapsible hierarchy. Each node shows the expression form, the result type, and component-dispatch metadata where applicable.
- **TypeScript cell auto-renderer** — dispatches based on the value's runtime type:
  - `Resource` → `ResourceInspector`
  - `ResultSet` → `ResultTable`
  - `Topology` → `LayerStackView`
  - `RunResult` (or any value with a `trace` field) → output panel with both the typed result *and* a `TraceTree` view of the trace
  - DOM node → mounted directly
  - Plain object/array → JSON tree view
  - Primitive → text

Phase 5 adds:

- **`ResultPlot`** — `@fluentui/react-charts` wrapper for general-purpose charts (line, bar, vertical-bar, area, donut, sparkline, etc.) over arbitrary tabular data. Fluent's chart components are themed consistently with the rest of the notebook UI and inherit accessibility / colour-vision treatments.
- Additional D3-bespoke components only when Fluent's chart catalogue can't express what we need (rare for analytical visualisations; the `LayerTopologyGraph` Phase 5 component still uses `@xyflow/react` because Fluent has no graph component).

### 6.8 TypeScript cell sandbox

For the MVP, TS cells execute via:

```typescript
const fn = new Function("eigen", "previousOutputs", `return (async () => { ${source}\n })();`);
const result = await fn(eigen, previousOutputsObject);
```

`eigen` is the active SDK instance. `previousOutputs` is an object keyed by cell ID with each previous cell's output. The cell's last expression value is the result.

This sandbox is **trusted** — TS cells run with full access to the page's JavaScript context. Acceptable for single-user authoring. Multi-user notebooks (Phase 7+) require a real sandbox: iframe with restricted permissions or a Web Worker with structured message passing.

### 6.9 Visualisation

The MVP ships with four render targets:

- **Tables** — `ResultTable` (Fluent UI `DataGrid`) for any `ResultSet`.
- **Resource inspectors** — `ResourceInspector` for any typed `Resource`.
- **Layer stack** — `LayerStackView` (plain JSX/CSS) renders the layer parent chain as a navigable stack of boxes with counts (X classes, Y properties, Z resources, N institutions per layer) and click-to-inspect drilling. No graph library; the layer-chain model is best communicated as exactly what it is — a chain of immutable parent pointers — and the boxes-and-arrows shape conveys that immediately. The full intra-layer relationship graph (classes, properties, requires/recommends edges, etc.) is a Phase 5 deliverable.
  ```typescript
  // In a TypeScript cell:
  const topo = await eigen.layerTopology({ includeResources: false });
  return LayerStackView({ topology: topo });
  ```
- **Trace trees** — `TraceTree` (D3 hierarchy) for any `RunResult.trace`. Programs run via `eigen.run()` automatically include the trace when the kernel has a trace store configured; the auto-renderer (§6.7) dispatches a combined "result + trace" panel. Notebook authors don't have to opt in.
  ```typescript
  // In a TypeScript cell — automatic when kernel has trace store:
  const result = await eigen.run({ programIri: "urn:...", inputIri: "urn:..." });
  return result;  // auto-rendered: typed output + trace tree side-by-side
  ```

Phase 5 adds:

- **Charts** — `@fluentui/react-charts` (Fluent UI's chart catalogue):
  ```typescript
  import { LineChart } from "@fluentui/react-charts";
  return <LineChart data={chartData} />;
  ```
  Themed via the Fluent `FluentProvider` shared by the rest of the notebook. Available chart types: line, vertical-bar, horizontal-bar, area, donut, sparkline, gauge, heatmap. Built on D3 internally; renders SVG.
- Additional D3-bespoke components for visualisations that don't fit Fluent's chart catalogue (rare for analytical needs; reserved as the escape hatch).

D3 is the fallback for the one-off cases. The full intra-layer topology graph (Phase 5) sits outside Fluent's component set entirely and uses `@xyflow/react` per §6.7.

### 6.10 Build and serve

`vite build` produces a static SPA in `notebooks/dist/`. The orchestrator serves it from `/notebooks/*` via a static-file route in `orchestration/src/server/notebook-static.ts`, configured by the `EIGENIUS_NOTEBOOK_STATIC` environment variable (path to the `dist/` directory).

Development: `vite dev` runs a hot-reloading dev server on a separate port (typically 5173) that proxies API calls to the orchestrator. The notebook author edits cells via the dev server while the actual API hits the local orchestrator.

Production: a single `docker compose up` brings up kernel + orchestrator; the orchestrator serves both the API and the notebook bundle. Visit `http://localhost:8080/notebooks/`.

### 6.11 Docker stack integration

The notebook ships as part of the existing orchestrator container — no new Compose service. This keeps single-origin deployment simple (no CORS, one URL), matches `docker compose up` muscle memory, and avoids a separate release pipeline.

`deploy/Dockerfile.orchestration` becomes a multi-stage build:

```dockerfile
# Stage 1 — build the notebook bundle (Node-based)
FROM node:20-bookworm AS notebook-build
WORKDIR /build
COPY notebooks/package.json notebooks/package-lock.json ./
RUN npm ci
COPY notebooks/ ./
COPY clients/eigenius-ts/ /clients/eigenius-ts/
RUN npm run build         # produces /build/dist/

# Stage 2 — Deno runtime (existing orchestrator)
FROM denoland/deno:latest
WORKDIR /app
COPY orchestration/deno.json ./
COPY orchestration/src/ src/
RUN deno cache src/main.ts

# Notebook bundle into the runtime image
COPY --from=notebook-build /build/dist /app/notebook-static
ENV EIGENIUS_NOTEBOOK_STATIC=/app/notebook-static

ENV EIGENIUS_KERNEL_ENDPOINT=http://eigenius-kernel:50051
EXPOSE 8080
CMD ["deno", "run", "--allow-net", "--allow-env", "--allow-read", "--allow-sys=hostname", "src/main.ts"]
```

`docker-compose.yml` is **unchanged**. The orchestrator service stays one service; its image just grows by ~5–15 MB (the built notebook bundle, gzipped).

Trade-offs accepted for this single-origin design:

- Notebook is rebuilt every time the orchestrator image rebuilds, even when only orchestrator code changed. Mitigated by Docker layer caching when only one of the two source trees changes.
- Cannot independently CDN-cache the notebook bundle. Acceptable for the MVP; revisit if the notebook becomes a major frontend.
- The orchestrator container now has both a Node-stage and a Deno-stage, slightly increasing build complexity. Mitigated by the multi-stage pattern keeping the runtime image clean.

If the notebook later outgrows this model — multi-tenant deployment, separate domain, CDN-hosting — splitting it into its own Compose service is a one-PR change that doesn't require rearchitecting the API surface.

### 6.12 Test automation

Browser-based UIs need real browser-based testing. The notebook's MVP test suite uses [**Playwright**](https://playwright.dev) — Microsoft-developed, Apache 2.0 licensed, TypeScript-native. Same licensing posture as the rest of the project; same language stack as the notebook itself.

**Why Playwright:**

- TypeScript-first; tests live in the same language and toolchain as the notebook.
- Multi-browser real-engine testing (Chromium, Firefox, WebKit) — no Electron, no headless-only constraints.
- Auto-wait semantics drastically reduce flake compared to Selenium / Cypress.
- Trace viewer and screenshot/video capture make CI failures debuggable from the artifact.
- Component-testing mode (`@playwright/experimental-ct-react`) handles tight-feedback testing of individual React components.
- Network mocking via `route()` enables integration tests that exercise the full notebook UI against a stubbed orchestrator without standing up the kernel.
- Runs against `docker compose up` end-to-end without special configuration.
- Apache 2.0 license — clean for a project under the same license.

**Three coverage tiers:**

| Tier | Scope | Speed | Purpose |
|---|---|---|---|
| **Component tests** | Single React component in isolation, via Playwright Component Testing | ~100ms each | Tight feedback loop on rendering and interaction logic |
| **Integration tests** | Notebook app + SDK + mocked orchestrator (via Playwright `route()` interception) | ~1s each | Verify SDK ↔ UI wiring without kernel; covers happy/error paths fast |
| **End-to-end tests** | Full stack via `docker compose up` (kernel + orchestrator + notebook) | ~10–30s each | Catches integration issues nothing else can; the patent-demo flow is the golden test |

**MVP test scope** (lands in Phase 4 alongside the rest of the MVP):

- **One golden e2e test** — opens the patent-analysis notebook from `http://localhost:8080/notebooks/`, runs all cells, edits the EigenQL filter, re-runs from that cell down, asserts that the `PatentBrief` output reflects the edited filter, that a topology cell renders the layer chain, and that a trace tree renders for the program execution. Single test, exercises every MVP feature end-to-end.
- **Component tests for the four MVP renderers** — `ResultTable`, `ResourceInspector`, `LayerStackView`, `TraceTree`. Verify each renders correctly given representative inputs (and the empty/error states).
- **A handful of integration tests** for the cell-execution wiring — "click Run on an EigenQL cell, verify the SDK was called, verify a `ResultTable` appears", "Run on a cell that errors, verify the error UI appears". Network-mocked; no kernel needed.

That's roughly a dozen tests for the MVP — enough to catch the regressions that matter without becoming a maintenance burden.

**Project layout:**

```
notebooks/
├── tests/
│   ├── components/                    # Playwright Component Testing
│   │   ├── ResultTable.spec.tsx
│   │   ├── ResourceInspector.spec.tsx
│   │   ├── LayerStackView.spec.tsx
│   │   └── TraceTree.spec.tsx
│   ├── integration/                   # Network-mocked SDK tests
│   │   ├── eigenql-cell.spec.ts
│   │   ├── esl-cell.spec.ts
│   │   └── error-handling.spec.ts
│   └── e2e/                           # Full-stack against docker compose
│       └── patent-demo.spec.ts
├── playwright.config.ts               # e2e + integration config
└── playwright-ct.config.ts            # component-test config
```

**npm scripts** (`notebooks/package.json`):

```json
{
  "scripts": {
    "test:component":   "playwright test --config playwright-ct.config.ts",
    "test:integration": "playwright test tests/integration",
    "test:e2e":         "playwright test tests/e2e",
    "test":             "npm run test:component && npm run test:integration && npm run test:e2e"
  }
}
```

**CI**: a new GitHub Actions workflow `notebooks-tests.yml` that:

1. Builds the notebook + the rest of the workspace.
2. Brings up the Docker Compose stack (`docker compose up --wait` with mock LLM mode).
3. Runs all three test tiers.
4. Uploads `playwright-report/` and `test-results/` as artifacts on failure (Playwright's trace viewer makes these directly debuggable from the GitHub Actions UI).

Run time target: ~3 minutes total. Parallelisable across browsers (Chromium for the bulk; Firefox + WebKit for smoke tests on golden flows).

**Authoring discipline:**

- New cell types or output renderers ship with at least one component test demonstrating the happy path.
- New SDK methods ship with at least one integration test verifying the wire format.
- The patent-demo e2e is sacred — any change that breaks it must come with an explicit update to either the test or the demo notebook, never both silently.

**What's deliberately not in the MVP test suite:**

- **Visual regression testing** — Playwright supports it (snapshot diffing) but it's high-flake and high-maintenance. Add later if visual stability becomes a concern.
- **Cross-browser coverage on every test** — Chromium-only for the bulk; Firefox + WebKit only on the golden e2e. Cross-browser bugs are rare for our React + standard-DOM use case.
- **Accessibility testing** — `@axe-core/playwright` is the right tool when this becomes a priority; deferred until the notebook reaches an audience that needs it.

### 6.13 Design system — Fluent UI v9

The notebook's UI shell, layout primitives, form controls, tables, and charts come from **[Fluent UI v9](https://react.fluentui.dev/)** (`@fluentui/react-components` for the design system; `@fluentui/react-charts` for charts). Used wherever Fluent has a fit-for-purpose component. The few places it doesn't:

- **Cell editor** — CodeMirror 6 (Fluent has no code editor).
- **Full intra-layer topology graph (Phase 5)** — `@xyflow/react` (Fluent has no node-edge graph component).
- **`LayerStackView`** — custom JSX/CSS (boxes-and-arrows over the parent chain). Built using Fluent primitives (`Card`, `Body1`, `Caption1`, etc.) so it visually matches the rest of the UI.
- **`TraceTree`** — D3-hierarchy-based custom component. Fluent's `Tree` is generic and tree-shaped data fits, but the trace tree wants per-node metadata badges (epistemic category, IO dispatch markers) that the generic Tree doesn't accommodate cleanly. May revisit as Fluent evolves.

**Why Fluent specifically:**

- Coherent design system across UI shell, tables, charts, and form controls — no hand-stitching of disparate libraries.
- Accessibility built in (ARIA, keyboard navigation, screen-reader labels) by default.
- Dark-mode support free via `webDarkTheme` paired with the `FluentProvider` at the React root.
- Active Microsoft engineering investment; large component catalogue; production-tested.
- MIT licensed — clean alongside Eigenius's Apache 2.0.

**Catalogue mapping** for the components the notebook uses or will use:

| Notebook concept | Fluent component(s) |
|---|---|
| App shell / cell wrapper / panels | `Card`, `Body1`, `Title3`, `Subtitle2` |
| Cell toolbar / Run buttons | `Toolbar`, `Button`, `MenuButton` |
| Tables (`ResultTable`) | `DataGrid`, `DataGridHeader`, `DataGridBody`, `DataGridRow`, `DataGridCell` |
| Charts (`ResultPlot`, Phase 5) | `LineChart`, `VerticalBarChart`, `AreaChart`, `DonutChart`, `Sparkline`, etc. from `@fluentui/react-charts` |
| Inputs (Phase 4 cell-creation UI) | `Field`, `Input`, `Dropdown`, `Combobox` |
| Resource inspector | `Card` + `DescriptionList` (`Body1`/`Body2` rows) |
| Notifications / errors | `MessageBar`, `Toast` |
| Tabs / multi-notebook UI (Phase 6+) | `TabList`, `Tab` |
| Theme toggle | `useFluentTheme` hook + manual switch between `webLightTheme` and `webDarkTheme` |

**Bundle size note**: `@fluentui/react-components` is ~500KB minified (significantly larger than TanStack Table alone), but the unified design system replaces several smaller libraries (TanStack Table + Recharts/Plot wrapper + custom CSS). Net delta is moderate; tree-shaking via Fluent v9's per-component imports keeps unused components out of the bundle.

**Theming**: a single `<FluentProvider theme={webLightTheme}>` (or `webDarkTheme`) wraps the React tree. Custom theme tokens for Eigenius-specific accents (epistemic-category colours, layer-chain badges) layered on top via Fluent's design tokens API.

---

## 7. Phasing

Six phases, each with deliverables and acceptance criteria.

### Phase 1 — SDK foundation (~1–2 weeks)

**Deliverables:**

- `proto/eigenius.proto` (extended in place): new `LayerTopology` RPC on `EigeniusKernel`; new `NotebookService` containing `LayerTopology`; new `LayerTopologyRequest` / `LayerTopologyResponse` message types and `TopologyNode` / `TopologyEdge` / `NodeKind` / `EdgeKind` supporting types.
- `buf.gen.yaml`: second `protoc-gen-es` output target for `clients/eigenius-ts/generated/`.
- Generated Connect stubs in both the orchestrator and `clients/eigenius-ts/generated/`.
- Orchestrator handlers for all seven RPCs in `orchestration/src/notebook/`.
- Kernel-side `LayerTopology` walker in `kernel/src/server/topology.rs`.
- TypeScript SDK in `clients/eigenius-ts/` with all public API surfaces functional.
- Smoke test (`clients/eigenius-ts/examples/smoke-test.ts`) that exercises every RPC.

**Acceptance:** the smoke test passes against a freshly-started `docker compose up` stack.

### Phase 2 — Static viewer (~1 week)

**Deliverables:**

- `notebooks/` scaffold: Vite + React + TypeScript + dependencies.
- React app that loads a `.json` notebook from disk and renders all cells with CodeMirror syntax highlighting.
- Cells are read-only; no Run buttons.
- A hand-crafted `notebooks/examples/patent-analysis.json` demo file.

**Acceptance:** `vite dev` opens the patent demo and renders all four cell types correctly.

### Phase 3 — Manual execution (~2 weeks)

**Deliverables:**

- "Run cell" buttons on ESL/EigenQL cells.
- ESL execution path: compile + load (declarations) or compile + load + run (programs).
- EigenQL execution path: query against current layer.
- Fluent UI `DataGrid` rendering for `ResultSet` outputs (per §6.13).
- Resource inspector for `Resource` outputs.
- "Run all" toolbar action.

**Acceptance:** every cell in the patent-analysis notebook runs end-to-end; tables render correctly; the program output renders as an inspector.

### Phase 4 — Authoring (the MVP) (~3–4 weeks)

**Deliverables:**

- Editable cells via CodeMirror.
- Cell toolbar: add cell (above / below), delete cell, move up / down, change type.
- Save to file (browser download) and load from file (file picker).
- Notebook metadata (title, modified timestamp) persisted in the JSON.
- TypeScript cell support with sandboxed execution and `eigen` + previous outputs in scope (needed for the auto-renderer dispatch and for the `LayerStackView` / `TraceTree` panels to render).
- Output auto-renderer (dispatches based on runtime type).
- **`LayerStackView`** component (plain JSX/CSS) — the layer-stack visualisation. The patent-analysis notebook gains a cell that renders the loaded layer chain as a navigable stack with counts and click-to-inspect drilling.
- **`TraceTree`** component (D3 hierarchy) — automatic trace rendering whenever `eigen.run()` returns a trace. The auto-renderer dispatches `RunResult` to a combined "typed output + trace tree" panel.
- Multi-stage `deploy/Dockerfile.orchestration` per §6.11; `EIGENIUS_NOTEBOOK_STATIC` static-file route in `orchestration/src/server/notebook-static.ts`. `docker compose up --build` after Phase 4 brings up a stack where the notebook is reachable at `http://localhost:8080/notebooks/`.
- **MVP test suite** per §6.12: one golden e2e test (the patent-demo flow), four component tests for the MVP renderers, a handful of integration tests for cell execution wiring, and the `notebooks-tests.yml` GitHub Actions workflow.

**Acceptance criterion (the MVP success criterion):** open the patent-analysis notebook from `http://localhost:8080/notebooks/`, edit the EigenQL filter, re-run from that cell down, see updated `PatentBrief` with the new filter applied. The program's execution surfaces as a navigable trace tree alongside the typed output. A layer-stack cell renders the loaded layer chain as a navigable stack with counts and click-to-inspect. Save the modified notebook. Reload from file with all changes intact. The Playwright golden e2e test exercising this whole flow passes in CI.

### Phase 5 — Visualisation (~2–3 weeks)

Now lighter, since the high-impact custom visualisations (layer-stack, trace) ship in the MVP. Phase 5 focuses on general-purpose chart support, the full intra-layer topology graph, and broader visualisation flexibility.

**Deliverables:**

- **Full intra-layer topology graph** — a richer `LayerTopologyGraph` component that renders the *full* topology returned by `eigen.layerTopology({ includeResources: true })`: classes, properties, institutions, plus all the edge kinds (`is_a`, `subclass_of`, `requires`, `recommends`, `property_ref`, `institution_declares`). Recommended library: [**`@xyflow/react`**](https://reactflow.dev/) (formerly react-flow) — React-native, ~80KB, MIT-licensed, designed for DAG/flow-style graphs which match Eigenius's resource graph shape. Cytoscape.js was considered and demoted in favour of react-flow on the grounds of better React integration, smaller bundle, and right-sized capability for our typical graph scale (4–15 layers, 50–200 classes).
- `@fluentui/react-charts` integration for general chart cells (line, bar, area, donut, sparkline, etc.).
- Sample cells in the patent notebook demonstrating chart usage and the full-topology graph — e.g., a distribution of confidence scores across analysis fields, plus a graph view of `is_a` relationships across the patent ontology.
- Documentation patterns for "how to drop down to D3 when Plot or react-flow doesn't fit."
- Polishing of the MVP visualisations based on feedback: `LayerStackView` styling refinements, `TraceTree` interaction (collapse/expand, search, jump-to-source).

**Acceptance:** the patent notebook gains (a) a chart cell rendering distribution data via `@fluentui/react-charts`, and (b) a full-topology cell rendering the patent-ontology class graph via `@xyflow/react`. User-facing patterns for arbitrary D3 visualisations are documented in the notebook user guide.

### Phase 6 — Reactivity + polish (~4–6 weeks)

**Deliverables:**

- TypeScript cell DAG analysis (simple variable-name extraction).
- Reactive re-runs for downstream TS cells when an upstream value changes.
- Per-cell auto-rerun toggle (default off until the model is proven).
- Error UI polish: inline error rendering, "jump to error position" affordances.
- Multi-notebook UI (tabs or file-tree sidebar) — *if* the use case has emerged.

**Acceptance:** in a notebook with three TS cells where `b` depends on `a` and `c` depends on `b`, editing `a` and re-running it triggers `b` and `c` to re-render automatically (with the toggle on).

### End-to-end MVP timeline

Phases 1–4 ≈ **8–11 weeks** of focused work (revised from 6–8 to reflect the topology + trace + Docker integration pulled into Phase 4 from Phase 5). This delivers the patent-demo-as-notebook success criterion *with* layer-stack visualisation and program-trace visualisation in scope. Phases 5–6 are valuable extensions that can be sequenced based on user feedback rather than committed upfront.

---

## 8. Open questions

### 8.1 Higher-level convenience methods

The current SDK shape mirrors the kernel's gRPC API one-to-one (with `LayerTopology` as the one new method). Should the SDK also expose composed methods for common notebook flows? Examples:

- `Eigen.compileAndLoad(esl)` — currently requires `compile` + `load` chained.
- `Eigen.loadAndRun(esl, input)` — for programs supplied inline.

Adding these in the SDK is cheap (pure TS composition); the question is whether they reduce notebook code enough to be worth the API surface. Defer until notebook usage reveals the answer.

### 8.2 Streaming queries

The kernel's gRPC API supports server-streaming responses for large result sets. Connect supports streaming RPCs too. Should the notebook SDK expose streaming queries?

For the MVP, no — query results return as a single `ResultSet`. Most exploratory queries return small results. When this becomes a bottleneck (large knowledge graphs, queries that produce 10k+ rows), add a `Eigen.queryStream(...)` returning an `AsyncIterable<ResultRow>`.

### 8.3 Authentication

Deferred. The MVP assumes localhost trust. Three plausible models for future:

- **Bearer token** — simplest; the orchestrator validates tokens against a configured secret.
- **OIDC** — for organisational deployments.
- **mTLS** — for service-to-service deployments.

The SDK has a `bearerToken` option in `EigenOptions` that's currently ignored; this is the hook for token-based auth without an SDK API change.

### 8.4 Multi-user notebooks

Out of scope for the MVP. When the use case appears, a CRDT-based shared-state model (Yjs, Automerge) is the standard answer. The notebook's `Cell` and `NotebookMeta` types are designed to be CRDT-friendly (no implicit ordering dependencies between non-adjacent cells).

### 8.5 Trace browsing of historical executions

The kernel has a trace store ([D6b](d6b-reasoning-trace-schema.md)) that records every program execution. The MVP renders the trace returned with each `Run` call (`TraceTree` component, §6.7 / §6.9). The deferred question is broader: should the notebook expose the trace store *directly* — e.g., a "trace browser" cell type that lets users list historical executions and re-render any of them?

Out of scope for the MVP. The MVP's trace integration is per-execution (run a program, see its trace inline). Historical-trace browsing requires new RPCs (`ListTraces`, `GetTrace`) and a different UI affordance, neither of which is needed for the patent-demo success criterion. Add when there's a use case.

### 8.6 Markdown cells with embedded interactivity

Marimo and Observable both support inline reactive widgets in markdown (e.g. a slider that affects downstream cells). Should the notebook support this?

Out of scope for the MVP. The cleanest path is a TypeScript cell that returns a React component with `useState` — that gives full interactivity without a new cell type. If common patterns emerge, a "widget" cell type can wrap them.

### 8.7 iframe sandbox for TypeScript cells

Currently the TS cell runs via `new Function()` with full page-context access. Trusted-author-only. When (not if) multi-user notebooks arrive, this becomes unsafe.

Two approaches:


- **iframe sandbox** — render each TS cell's output in a sandboxed iframe; communicate via postMessage. Strong isolation; awkward for cells that need DOM access to the parent.
- **Web Worker** — run the TS code in a worker; message-pass results back. Easier for compute-only cells; harder when the cell wants to render DOM.

Decide when multi-user is actually planned.

### 8.8 Notebook indexing and search

The MVP supports one open notebook at a time. As notebook collections grow, users need to find specific notebooks. Filesystem listing? Tags? Search across notebook contents?

Out of scope for the MVP. Most early users will have a small handful of notebooks they remember by filename.

---

## 9. Decisions log

| Question | Decision | Rationale |
|----------|----------|-----------|
| Notebook architecture: existing tool (Jupyter, Marimo, Observable) vs. custom React app? | Custom React app | User preference for TypeScript-only tooling, deep integration with Eigenius-specific concepts (typed resource inspector, layer-chain topology, trace trees) |
| Build tool? | Vite | Standard for React+TS; works under Deno via npm interop; well-maintained |
| Cell editor? | CodeMirror 6 | Modular, lightweight (~200KB), good React binding, custom language modes well-supported |
| State management? | Zustand | Right complexity tier for the MVP; small, modern, hooks-native |
| UI design system? | Fluent UI v9 (`@fluentui/react-components`) | Single coherent system across shell / tables / charts / inputs; accessibility built in; dark mode free; MIT-licensed; production-tested. ~500KB but replaces several smaller libraries (TanStack Table + chart wrapper + custom CSS); tree-shakeable per-component imports |
| Plot library? | `@fluentui/react-charts` | Themed consistently with the rest of the UI; D3 internally; Fluent's accessibility / colour-vision treatments. Observable Plot considered but rejected to keep design-system unity |
| Table library? | Fluent UI `DataGrid` | Themed consistently; column definitions driven by ResultSet's synthesized Property metadata (`short_name` for headers, `data_type` for column types); built-in virtualisation; accessibility-first. TanStack Table considered but rejected to keep design-system unity |
| Layer-stack visualisation (MVP)? | Plain JSX/CSS — `LayerStackView` boxes-and-arrows view of the parent chain | Layer chain is linear and best communicated as exactly that; no graph library needed for the MVP scope; ~80 lines of React |
| Full intra-layer topology graph (Phase 5)? | `@xyflow/react` (react-flow) | React-native, ~80KB, MIT, designed for DAG/flow-shape graphs which fit our resource graph well; better React DX and smaller bundle than Cytoscape.js for our scale (4–15 layers, 50–200 classes); Cytoscape considered and demoted |
| Trace-tree library (MVP)? | D3 hierarchy | Trace visualisation is core to understanding program execution and surfacing the four epistemic categories; in scope for the MVP, auto-rendered whenever `eigen.run()` returns a trace |
| Notebook serving in Compose stack? | Bundled into the existing orchestrator container via multi-stage Dockerfile | Single origin, no CORS, no new service; trade-off is rebuild coupling, accepted for MVP |
| Test framework? | Playwright | TypeScript-native, Apache 2.0, multi-browser real-engine testing, auto-wait reduces flake, component-test mode handles tight feedback, runs against `docker compose up` for e2e; matches both our language stack and license posture |
| Test coverage scope (MVP)? | One golden e2e (patent demo) + four component tests for MVP renderers + a handful of integration tests | Catches regressions where they matter without becoming a maintenance burden; can grow over time |
| Browser ↔ kernel transport? | Connect-RPC via the orchestrator | Native browser support without a gRPC-Web proxy; reuses existing orchestrator infrastructure |
| New protobuf namespace vs. extending existing proto? | Extend the existing flat `proto/eigenius.proto` in place; add `LayerTopology` to `EigeniusKernel` and a new `NotebookService` in the same file | Single source of truth, no consumer-side path changes; reorganisation into versioned packages is a worthwhile future cleanup but irrelevant to MVP scope |
| Most "notebook RPCs" already exist on `EigeniusKernel`? | Yes — reuse `Inspect`/`Query`/`Load`/`RunProgram`/`ListInstitutions` directly from the browser. Only add genuinely new methods (`LayerTopology`) | Avoids duplicating the kernel surface; SDK presents a uniform `Eigen` class regardless of which service a method lives on |
| Connect API shape: 1:1 mirror vs. notebook-shaped? | 1:1 mirror with one addition (`LayerTopology`) | Lower design cost upfront; SDK can compose convenience methods later without API churn |
| `LayerTopology` location: kernel or orchestrator? | Kernel | The walk needs efficient layer-chain access that only the kernel has; orchestrator-side reimplementation would be redundant and slow |
| TypeScript cell sandbox? | `new Function()` for MVP, deferring iframe / Worker | Single-user trust model is fine for MVP; isolation work can be scoped to when multi-user actually arrives |
| Cell execution model: reactive vs manual? | Manual for MVP (Phases 3–4); hybrid reactive for TS cells in Phase 6 | Reactive cell DAG across mixed languages is hard; ship manual first, prove the value, add reactivity where it's well-understood (TS dataflow) |
| Cross-language reactivity? | Deferred indefinitely | Risky semantics (auto-loading layers); cost-benefit unclear |
| Notebook file format? | Custom JSON with `format_version: 1` | Full control; versioning baked in for migration; `.ipynb` semantics don't fit multi-language reactive cells well |
| Server-side notebook persistence? | Out of scope for MVP | File-based persistence works; server-side adds auth, multi-user, conflict-resolution complexity |
| SDK distribution: JSR vs npm? | Both — JSR source-of-truth, dnt-mirrored to npm | Native to the Deno orchestration tier; Vite consumers via npm; one source, two channels |
| SDK package name? | `@eigenius/client` | Precise, avoids overloaded "sdk" suffix |
| Serving the notebook? | From the orchestrator at `/notebooks/*` | Single origin, no CORS, single deployment artifact |
| Authentication for MVP? | None (assume trusted localhost) | Defers a separate design effort; production deployments will add a reverse proxy |
| Streaming queries? | Unary only for MVP | Most queries return small results; streaming added when needed |
| Same repo or new? | Same repo (`clients/eigenius-ts/` + `notebooks/` as new top-level dirs) | Co-evolution with kernel; single contributor story; reversible if scope grows |

---

## 10. Implementation pointers

Files that need to exist after Phase 1:

| Path | Purpose |
|---|---|
| `proto/eigenius.proto` (existing flat file) | + `LayerTopology` RPC on `EigeniusKernel`; + `NotebookService` (with `LayerTopology` method); + `LayerTopologyRequest` / `LayerTopologyResponse` / `TopologyNode` / `TopologyEdge` / `NodeKind` / `EdgeKind` |
| `buf.gen.yaml` (existing) | + second `protoc-gen-es` output target for `clients/eigenius-ts/generated/` |
| `kernel/src/server/topology.rs` | Layer-walking implementation |
| `kernel/src/server/mod.rs` | + `LayerTopology` handler registered |
| `orchestration/src/notebook/service.ts` | NotebookService Connect handler registration |
| `orchestration/src/notebook/{inspect,query,load,compile,run,topology,institutions,errors}.ts` | Per-RPC handlers |
| `orchestration/src/server/mod.ts` | + NotebookService mounted on the Connect server |
| `clients/eigenius-ts/deno.jsonc` | Deno tasks |
| `clients/eigenius-ts/package.json` | npm metadata |
| `clients/eigenius-ts/jsr.json` | JSR metadata |
| `clients/eigenius-ts/mod.ts` | Public API exports |
| `clients/eigenius-ts/src/{client,resource,result,layer,topology,institution,errors,transport}.ts` | SDK implementation |
| `clients/eigenius-ts/generated/` | buf-generated Connect stubs |
| `clients/eigenius-ts/examples/smoke-test.ts` | Phase 1 acceptance |
| `buf.gen.yaml`, `buf.work.yaml` | buf configuration for stub generation |

Files that need to exist after Phase 4 (the MVP):

| Path | Purpose |
|---|---|
| `notebooks/{deno.jsonc,package.json,vite.config.ts,index.html}` | Build configuration |
| `notebooks/src/{main.tsx,App.tsx}` | React root |
| `notebooks/src/components/Notebook.tsx` | Cell list + toolbar |
| `notebooks/src/components/Cell.tsx` | Cell wrapper |
| `notebooks/src/components/cells/{Markdown,ESL,EigenQL,TypeScript}Cell.tsx` | Per-type cells |
| `notebooks/src/components/editors/{CodeMirrorEditor.tsx,eigenql-mode.ts,esl-mode.ts}` | Editor + language modes |
| `notebooks/src/components/output/{ResultTable,ResourceInspector,LayerStackView,TraceTree}.tsx` | MVP renderers (table, resource, layer-stack chain, program trace) |
| `notebooks/src/runtime/{eigen-client.ts,ts-cell-runner.ts,output-renderer.ts}` | SDK wiring + TS-cell sandbox + auto-renderer |
| `notebooks/src/state/notebook-store.ts` | Zustand store |
| `notebooks/src/persistence/{notebook-format.ts,file-io.ts}` | Save / load |
| `notebooks/examples/patent-analysis.json` | The demo notebook |
| `notebooks/playwright.config.ts`, `notebooks/playwright-ct.config.ts` | Playwright e2e and component-test configs |
| `notebooks/tests/components/{ResultTable,ResourceInspector,LayerStackView,TraceTree}.spec.tsx` | Component tests for the four MVP renderers |
| `notebooks/tests/integration/{eigenql-cell,esl-cell,error-handling}.spec.ts` | Network-mocked SDK ↔ UI integration tests |
| `notebooks/tests/e2e/patent-demo.spec.ts` | Golden end-to-end test against `docker compose up` |
| `.github/workflows/notebooks-tests.yml` | CI: build + compose-up + Playwright run + artifact upload |
| `orchestration/src/server/notebook-static.ts` | Static-file route serving the notebook bundle at `/notebooks/*` |
| `deploy/Dockerfile.orchestration` | + multi-stage Node-build → Deno-runtime per §6.11 |

Additional files for Phase 5 (full intra-layer topology graph + chart cells):

| Path | Purpose |
|---|---|
| `notebooks/src/components/output/LayerTopologyGraph.tsx` | `@xyflow/react`-based full-topology graph |
| `notebooks/src/components/output/ResultPlot.tsx` | `@fluentui/react-charts` wrapper for chart cells |

---

## 11. References

- [D1 — Eigon serialization format](d1-eigon-serialization-format.md)
- [D2 — EigenQL specification](d2-eigenql-specification.md)
- [D5 — gRPC API specification](d5-grpc-api-specification.md)
- [D6 — Execution architecture](d6-execution-architecture.md)
- [D6b — Reasoning trace schema](d6b-reasoning-trace-schema.md)
- [D7 — ESL surface syntax](d7-esl-surface-syntax.md)
- [Connect-RPC](https://connectrpc.com/) — protocol spec, documentation
- [Fluent UI React v9](https://react.fluentui.dev/) — design system (UI shell, layout, `DataGrid`, inputs)
- [`@fluentui/react-charts`](https://github.com/microsoft/fluentui/tree/master/packages/charts/react-charts) — chart catalogue paired with Fluent v9
- [CodeMirror 6](https://codemirror.net/) — editor
- [`@xyflow/react`](https://reactflow.dev/) (formerly react-flow) — graph rendering for the Phase 5 full intra-layer topology
- [Playwright](https://playwright.dev) — browser test automation
- [Zustand](https://github.com/pmndrs/zustand) — state management
- [JSR](https://jsr.io/) — JavaScript Registry, source-of-truth publication target
- [`dnt`](https://github.com/denoland/dnt) — Deno-to-Node transformer for npm mirror publication
