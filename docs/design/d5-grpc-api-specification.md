# D5: gRPC API Specification

*Design document for the Eigenius project — April 2026*

**Status:** Implemented (Phase 3)
**Required before:** Phase 3 implementation
**Resolves:** RPC definitions, message types, streaming vs unary, error codes, authentication, context management

---

## 1. Overview

The Eigenius kernel exposes its functionality as a gRPC service. The orchestration layer (Deno/TypeScript) and CLI connect as gRPC clients. The API surface covers: loading resources, querying, program validation and execution, resource inspection, and reasoning trace recording.

### 1.1 Design principles

**CBOR as the wire format.** Resources cross the gRPC boundary as CBOR-encoded bytes (RFC 8949). CBOR is compact, fast to parse, and matches the storage format (see D4). Clients that need human-readable output (CLI) convert to Eigon-JSON on the client side. Clients may also send Eigon-JSON — the server accepts both formats and distinguishes by a `content_type` field.

**Stateful contexts.** The server maintains execution contexts with layer chains. Clients reference contexts by ID. This avoids re-transmitting the entire layer state on every request.

**Streaming for large results.** Query results are server-streamed. All other RPCs are unary.

---

## 2. Service Definition

```protobuf
service EigeniusKernel {
  // --- Resource management ---
  rpc Load(LoadRequest) returns (LoadResponse);
  rpc Inspect(InspectRequest) returns (InspectResponse);

  // --- Query ---
  rpc Query(QueryRequest) returns (QueryResponse);

  // --- Program ---
  rpc ValidateProgram(ValidateProgramRequest) returns (ValidateProgramResponse);
  rpc RunProgram(RunProgramRequest) returns (RunProgramResponse);

  // --- Reasoning traces ---
  rpc Reflect(ReflectRequest) returns (ReflectResponse);

  // --- Context management ---
  rpc CreateContext(CreateContextRequest) returns (CreateContextResponse);
  rpc CommitContext(CommitContextRequest) returns (CommitContextResponse);

  // --- Health ---
  rpc Health(HealthRequest) returns (HealthResponse);
}
```

---

## 3. RPCs

### 3.1 Load

Load resources into a context's working layer, validate, and optionally commit.

```protobuf
message LoadRequest {
  string context_id = 1;
  bytes resources = 2;          // Resources encoded as CBOR (default) or Eigon-JSON
  string content_type = 3;      // "application/cbor" (default) or "application/eigon+json"
  bool auto_commit = 4;         // If true, commit after successful validation
  CommitPolicy policy = 5;      // Retroactive-validation policy (D41 §3.3)
  repeated string explicit_tombstones = 6;  // IRIs to tombstone in this commit (D41 §10.1)
}

message LoadResponse {
  bool success = 1;
  repeated ValidationError errors = 2;
  string layer_id = 3;          // Set if auto_commit was true and succeeded
  uint32 resource_count = 4;
  // ... branch, branch_advanced, merge ...
  uint32 total_violations = 8;          // True violation count when errors are truncated
  repeated CommittedLayer committed_layers = 9;  // Per-layer outcomes (D41 §6)
}
```

`policy` carries the commit's retroactive-validation policy (per D41 §3.3, §8). Unset on the wire → server applies `Reject{ max_violations: 100 }`. `Reject{0}` is interpreted as "use the default cap". `CascadeTombstone` opts into iterative lower-layer tombstoning to fixpoint. `explicit_tombstones` ride into the initial user-layer builder before retroactive validation runs (D41 §10.1); under `CascadeTombstone` they're combined with any cascade-inferred tombstones.

`committed_layers` exposes the per-layer outcomes from the multi-layer commit pipeline (D41 §6). For an `auto_commit` Load this is up to three entries — the user layer (`role = LAYER_ROLE_USER`), the optional `verdict_provenance` audit Sibling (`role = LAYER_ROLE_AUDIT_PROVENANCE`), and the optional `institution_classes` Child (`role = LAYER_ROLE_INSTITUTION_CLASSES`) — each carrying its own `layer_id`, `branch_advanced`, `merge`, `cascade_tombstones`, and `cascade_iterations`. The top-level `layer_id` / `branch_advanced` / `merge` fields continue to point at the user layer specifically. `total_violations` reports the kernel-internal full violation count when `Reject` truncates `errors` to its `max_violations` cap, so clients can render "showing X of Y". See the proto file for the canonical wire shape; see D41 for the commit-pipeline semantics.

Each `CommittedLayer` carries a closed `LayerRole` taxonomy that clients should match on to identify the user / audit / institution-classes entries rather than string-comparing the free-form `name` field. The `role` discriminator is what makes the rejected-but-audited Sibling-rescue path (D41 §6.1) unambiguous: when `autoonload_dispatch` returns `Err` on a `Fails` verdict, the failing user-layer pipeline pushes nothing to `committed_layers` and the rescued audit Sibling lands at index 0, so position-in-vec mapping is unsafe. The `name` field remains useful for display labels (notebook layer-stack visualisation, trace logs) but is not the routing key.

### 3.2 Inspect

Resolve a resource by IRI from the context's layer chain.

```protobuf
message InspectRequest {
  string context_id = 1;
  string iri = 2;
  string accept = 3;            // "application/cbor" (default) or "application/eigon+json"
}

message InspectResponse {
  bool found = 1;
  bytes resource = 2;           // Resource in requested format
  string content_type = 3;      // Format of the response
}
```

### 3.3 Query

Execute an EigenQL program against the context's layer chain. Results are streamed.

```protobuf
message QueryRequest {
  string context_id = 1;
  string eigenql = 2;           // EigenQL program string
  string accept = 3;            // "application/cbor" (default) or "application/eigon+json"
}

message QueryResponse {
  bool success = 1;
  bytes document = 2;           // Eigon-CBOR document; same wire shape as LoadRequest.resources
  string content_type = 3;      // "application/cbor"
  string error = 4;             // Error message if !success
}
```

The `document` carries an array of resources: synthesized row Properties, a row Class, and a ResultSet with the row resources embedded. Consumers walk the ResultSet's `result_class` → property list to discover short-name ↔ IRI mappings. See D2 Appendix A for the full shape.

### 3.4 ValidateProgram

Type-check a program against the context's layer chain using EigenTT.

```protobuf
message ValidateProgramRequest {
  string context_id = 1;
  bytes program = 2;            // Program resource as CBOR or Eigon-JSON
  string content_type = 3;      // "application/cbor" (default) or "application/eigon+json"
}

message ValidateProgramResponse {
  bool valid = 1;
  repeated ValidationError errors = 2;
  string program_type = 3;      // Human-readable type description (e.g., "Dog → Dog")
}
```

### 3.5 RunProgram

Execute a validated program with input data.

```protobuf
message RunProgramRequest {
  string context_id = 1;
  bytes program = 2;            // Program resource as CBOR or Eigon-JSON
  bytes input = 3;              // Input resource as CBOR or Eigon-JSON
  string content_type = 4;      // Format of program and input
  string accept = 5;            // Desired format for output
}

message RunProgramResponse {
  bool success = 1;
  bytes output = 2;             // Output resource in requested format
  string content_type = 3;      // Format of the output
  repeated ValidationError errors = 4;
}
```

### 3.6 Reflect

Record a reasoning trace as a typed resource in the context.

```protobuf
message ReflectRequest {
  string context_id = 1;
  bytes trace = 2;              // Reasoning trace resource as CBOR or Eigon-JSON
  string content_type = 3;      // Format of the trace
}

message ReflectResponse {
  bool success = 1;
  string trace_iri = 2;         // IRI of the committed trace resource
}
```

### 3.7 CreateContext

Create a new execution context. The context starts with the bootstrapped layer chain (core + program ontologies).

```protobuf
message CreateContextRequest {
  string name = 1;              // Human-readable context name
  bool read_only = 2;           // If true, context rejects writes
}

message CreateContextResponse {
  string context_id = 1;
}
```

### 3.8 CommitContext

Commit the working layer in a context, producing a new immutable layer.

```protobuf
message CommitContextRequest {
  string context_id = 1;
  string layer_name = 2;        // Name for the committed layer
}

message CommitContextResponse {
  bool success = 1;
  string layer_id = 2;          // Content-addressed ID of the committed layer
  repeated ValidationError errors = 3;
}
```

### 3.9 Health

Health check for load balancers and container orchestration.

```protobuf
message HealthRequest {}

message HealthResponse {
  bool healthy = 1;
  string version = 2;
  uint64 layer_count = 3;
  uint64 resource_count = 4;
}
```

---

## 4. Common Types

```protobuf
message ValidationError {
  string resource_iri = 1;      // IRI of the resource with the error (if applicable)
  string property_iri = 2;      // Property with the error (if applicable)
  string rule = 3;              // Validation rule that was violated
  string message = 4;           // Human-readable error message
  string severity = 5;          // "error" or "warning"
}
```

---

## 5. Error Handling

gRPC status codes map to Eigenius error categories:

| gRPC Status | When used |
|-------------|-----------|
| `OK` | Request succeeded |
| `INVALID_ARGUMENT` | Malformed JSON, invalid IRI, parse errors |
| `FAILED_PRECONDITION` | Validation failures, type errors, stale context |
| `NOT_FOUND` | Resource not found, unknown context ID |
| `ALREADY_EXISTS` | Context name conflict, namespace violation |
| `INTERNAL` | Storage errors, unexpected kernel errors |
| `UNAVAILABLE` | Service starting up, storage not ready |
| `RESOURCE_EXHAUSTED` | Query result too large, program execution timeout |

Detail messages use the `ValidationError` type where applicable, embedded as gRPC error details.

---

## 6. Authentication

### 6.1 Phase 3 (initial)

API key in gRPC metadata:

```
authorization: Bearer <api-key>
```

The server validates the key against a configured set. No per-resource authorization — all authenticated clients have full access.

### 6.2 Future

- **mTLS** — mutual TLS for service-to-service authentication (kernel ↔ orchestration)
- **Azure AD tokens** — for Azure-hosted deployments with managed identity
- **Per-namespace authorization** — ontology-level access control (design doc D9)

---

## 7. Context Lifecycle

```
Client                          Server
  │                               │
  │ CreateContext("my-session")   │
  │──────────────────────────────>│
  │    context_id: "ctx-abc123"   │
  │<──────────────────────────────│
  │                               │
  │ Load(ctx-abc123, animals.json)│
  │──────────────────────────────>│
  │    success: true, count: 5    │
  │<──────────────────────────────│
  │                               │
  │ Query(ctx-abc123, "MATCH ...") │
  │──────────────────────────────>│
  │    stream: result 1           │
  │<──────────────────────────────│
  │    stream: result 2           │
  │<──────────────────────────────│
  │    stream: complete           │
  │<──────────────────────────────│
  │                               │
  │ CommitContext(ctx-abc123)     │
  │──────────────────────────────>│
  │    layer_id: "ee0b8a..."      │
  │<──────────────────────────────│
```

Contexts are server-side state. They are garbage-collected after inactivity (configurable timeout, default 30 minutes). The `context_id` is a UUID generated by the server.

**Default context:** The server creates a default read-write context on startup. Clients that don't call `CreateContext` use the default context. This preserves the simple CLI experience:

```bash
eigenius --endpoint http://host:50051 load animals.json
eigenius --endpoint http://host:50051 query 'MATCH ...'
```

---

## 8. CLI Integration

The CLI operates in two modes:

| Flag | Mode | Backend |
|------|------|---------|
| `--local` (default) | Embedded kernel | In-memory or RocksDB |
| `--endpoint <url>` | gRPC client | Remote kernel service |

Both modes expose the same commands. The CLI abstracts the backend behind a `KernelClient` trait:

```rust
trait KernelClient {
    fn load(&mut self, json: &str) -> Result<LoadResponse, Error>;
    fn query(&self, eigenql: &str) -> Result<Vec<Resource>, Error>;
    fn validate_program(&self, json: &str) -> Result<ValidateResponse, Error>;
    fn run_program(&self, program: &str, input: &str) -> Result<Resource, Error>;
    fn inspect(&self, iri: &str) -> Result<Option<Resource>, Error>;
}
```

`LocalClient` wraps the embedded kernel. `RemoteClient` wraps a tonic gRPC client.

---

## 9. Orchestration Layer Integration

The Deno orchestration layer connects as a gRPC client:

```typescript
class KernelClient {
    constructor(endpoint: string);

    async createContext(name: string): Promise<string>;
    async load(contextId: string, json: string): Promise<LoadResponse>;
    async query(contextId: string, eigenql: string): AsyncIterable<Resource>;
    async validateProgram(contextId: string, program: string): Promise<ValidateResponse>;
    async runProgram(contextId: string, program: string, input: string): Promise<Resource>;
    async commitContext(contextId: string): Promise<string>;
}
```

The orchestration layer uses `query` streaming to process large result sets without buffering everything in memory.

---

## 10. Decisions Log

| Question | Decision | Rationale |
|----------|----------|-----------|
| Wire format for resources | CBOR (default), Eigon-JSON (opt-in via content_type) | CBOR is compact and matches storage format; JSON available for debugging |
| Query results | Server-streaming | Supports large result sets without client OOM |
| All other RPCs | Unary | Request-response is sufficient; simpler implementation |
| Context management | Server-side state with UUID IDs | Avoids re-transmitting layer state on every request |
| Default context | Auto-created on startup | Preserves simple CLI experience |
| Authentication Phase 3 | API key in metadata | Simplest; sufficient for initial deployment |
| Context GC | 30-minute inactivity timeout | Prevents server memory leak from abandoned contexts |
| Health check | Dedicated RPC + HTTP endpoint | gRPC health for clients; HTTP for container orchestration probes |
