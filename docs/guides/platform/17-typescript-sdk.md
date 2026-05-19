# 17. TypeScript SDK — `@eigenius/client`

The TypeScript SDK at [`clients/eigenius-ts/`](../../../clients/eigenius-ts/) wraps the orchestrator's two Connect-RPC services in a single typed `Eigen` class. It is the same SDK the notebook uses ([chapter 14](14-notebook.md)); you can use it from any TypeScript runtime that speaks Connect-RPC — Deno scripts, browser apps, Node.js servers — to drive the kernel programmatically.

Per [D22 §5](../../design/d22-notebook-and-typescript-sdk.md).

## 17.1. What it covers

The `Eigen` class exposes everything the notebook MVP needs. All methods are async; all responses are protobuf message objects (camelCased fields).

| Method | Purpose | Underlying RPC |
|---|---|---|
| `eigen.health()` | Liveness check | `EigeniusKernel.Health` |
| `eigen.inspect(iri, options?)` | Resolve a resource by IRI from the active layer | `EigeniusKernel.Inspect` |
| `eigen.query(eigenql, options?)` | Run an EigenQL query, get an Eigon-CBOR ResultSet document | `EigeniusKernel.Query` |
| `eigen.load(source, options?)` | Compile + commit a layer (ESL source or Eigon-JSON document) | `EigeniusKernel.Load` |
| `eigen.validateProgram(program, options?)` | Type-check a program against the active layer chain | `EigeniusKernel.ValidateProgram` |
| `eigen.runProgram(program, input, options?)` | Run a program inline against an inline input | `EigeniusKernel.RunProgram` |
| `eigen.runProgramByIri(programIri, inputIri, options?)` | Run a program already loaded into the chain by IRI against an input also by IRI — the natural "one program × N inputs" shape | `EigeniusKernel.RunProgramByIri` |
| `eigen.listInstitutions()` | List registered institutions and their declared fiber structure | `EigeniusKernel.ListInstitutions` |
| `eigen.layerTopology(options?)` | Walk the layer chain; nodes for layers + classes + properties + institutions, edges for parent / is_a / subclass / requires / property-ref | `NotebookService.LayerTopology` |
| `eigen.publishNotebook(notebook)` | Translate a `NotebookJson` into Notebook + Cell resources (content-addressed IRIs), then load them | calls `Eigen.load` internally |

The SDK also exports the underlying request/response types (`LoadResponse`, `QueryResponse`, `RunProgramResponse`, `LayerTopologyResponse`, `TopologyNode`, `TopologyEdge`, `NodeKind`, `EdgeKind`, etc.) and notebook-specific types (`NotebookJson`, `CellJson`, `notebookJsonToResources`, `resourcesToNotebookJson`).

Source: [`clients/eigenius-ts/src/client.ts`](../../../clients/eigenius-ts/src/client.ts) (the `Eigen` class), [`clients/eigenius-ts/src/notebook.ts`](../../../clients/eigenius-ts/src/notebook.ts) (notebook-publish translator), [`clients/eigenius-ts/mod.ts`](../../../clients/eigenius-ts/mod.ts) (public exports).

## 17.2. Construction

```typescript
import { Eigen } from "@eigenius/client";

// Browser / single-origin: hits the same origin, expects the
// orchestrator's static-file route to be live.
const eigen = new Eigen({ endpoint: window.location.origin });

// Deno / Node / explicit endpoint:
const eigen = new Eigen({ endpoint: "http://localhost:8080" });
```

`EigenOptions`:

| Field | Required | Description |
|---|---|---|
| `endpoint` | yes | Orchestrator base URL — e.g. `http://localhost:8080` |
| `fetch` | no | Override the global `fetch` (Deno tests, custom interceptors) |
| `bearerToken` | no | Currently unused; the hook for token auth without an API change later (D22 §8.3) |
| `defaultBranch` | no | Branch used when a call doesn't pass an explicit `branch`. Empty / omitted = the server's default (`main`). See §17.4 for branch semantics. |

## 17.3. Five-line examples

### Liveness + inspect

```typescript
const h = await eigen.health();
console.log(`kernel v${h.version}, ${h.layerCount} layer(s), ${h.resourceCount} resource(s)`);

const cls = await eigen.inspect("urn:eigenius:core:Class");
console.log(cls.found ? `Class CBOR is ${cls.resource.length} bytes` : "not found");
```

### Load ESL → query

```typescript
const { success, layerId, resourceCount, errors } = await eigen.load(`
  namespace ex = "urn:eigenius:demo:ex";
  class ex:Widget {
    description = "A widget.";
    requires ex:label;
  }
  property ex:label : core:string { description = "Label."; }
`);
if (!success) throw new Error(errors.map((e) => e.message).join("; "));

const q = await eigen.query(`
  USING "urn:eigenius:core:Class"
  MATCH Class(?c) { short_name: ?n }
  RETURN [] { name: ?n }
  ORDER BY ?n
`);
```

`q.document` is an Eigon-CBOR document containing the ResultSet + row class + row Property metadata + row resources. Decode it with `cbor-x` (the same package the notebook uses) — see [`notebooks/src/runtime/resultDocument.ts`](../../../notebooks/src/runtime/resultDocument.ts) for a worked decoder.

### Run a program by IRI

```typescript
// Assumes a previous eigen.load(...) committed both the program
// (analyze_patent) and the input (US10452978B2) to the active chain.
const result = await eigen.runProgramByIri(
  "urn:eigenius:demo:patent:analyze_patent",
  "urn:eigenius:demo:patent:US10452978B2",
);
console.log(`success=${result.success}, trace=${result.traceIri}`);
// result.output is a CBOR-encoded PatentBrief resource;
// result.traceIri points at the ProgramTrace if a trace store is configured.
```

The `runProgram` (inline) variant takes both program and input as bytes with a single `content_type` — useful when neither is in the chain yet.

### Walk the layer chain

```typescript
import { NodeKind } from "@eigenius/client";

const topo = await eigen.layerTopology({ includeResources: false });
const layers = topo.nodes.filter((n) => n.kind === NodeKind.LAYER);
const classes = topo.nodes.filter((n) => n.kind === NodeKind.CLASS);
console.log(`${layers.length} layers, ${classes.length} classes`);
```

Set `includeResources: true` to also get one node per `Resource` instance (heavier; handy for debugging / full-graph rendering).

### Publish a notebook

```typescript
import type { NotebookJson } from "@eigenius/client";

const notebook: NotebookJson = JSON.parse(
  await Deno.readTextFile("./my-notebook.json"),
);
const { publish, load } = await eigen.publishNotebook(notebook);
console.log(`notebook IRI: ${publish.notebookIri}`);
console.log(`${publish.cellIris.length} cell IRI(s) in the published layer ${load.layerId}`);
```

Identical content yields the same `notebookIri` (content-addressed). See [chapter 14 §14.5](14-notebook.md#135-publish-to-layer).

## 17.4. Branches

Phase 14g made every `load` / `runProgram` / `runProgramByIri` / `reflect` call branch-aware. The TypeScript client mirrors the gRPC surface: per-call `branch` overrides on the relevant methods, plus a `defaultBranch` constructor option and a `useBranch(name)` mutator for setting the client-wide default after construction. Empty / omitted on a call falls back to the client default; empty / omitted on the constructor falls back to the server default (`main`).

```typescript
// Default to a feature branch for the lifetime of this client
const eigen = new Eigen({
  endpoint: "http://localhost:8080",
  defaultBranch: "feature-x",
});

// All commits land on feature-x
await eigen.load(eslSource, { contentType: "application/x-esl" });

// Override for one call — commit to main even though the default is feature-x
await eigen.load(eslSource, {
  contentType: "application/x-esl",
  branch: "main",
});

// Mutate the default after construction
eigen.useBranch("experiment-42");
console.log(eigen.getDefaultBranch()); // "experiment-42"
```

For reads, `branch` and `atLayer` are mutually exclusive — pass either to pin the read, leave both empty to use the (default) branch's current head:

```typescript
// Pin to a specific layer
const r1 = await eigen.inspect("urn:example:Dog", { atLayer: "abe85ea7..." });

// Pin to a branch's current head
const r2 = await eigen.inspect("urn:example:Dog", { branch: "feature-x" });
```

Branch CRUD methods (require a kernel with a persistent backend):

```typescript
// List
const branches = await eigen.listBranches();
for (const b of branches) console.log(`${b.name}: ${b.headLayer}`);

// Show
const main = await eigen.getBranch("main");
if (main.found) console.log(`main is at ${main.headLayer}`);

// Create — branch off main's head
const created = await eigen.createBranch("feature-x", { fromLayer: main.headLayer });
if (!created.success) console.error(created.error);

// Delete (defaults to safe — refuses if a task pin matches the head)
const del = await eigen.deleteBranch("feature-x");
console.log(del.deleted ? `deleted (was at ${del.previousHead})` : "did not exist");

// Force-delete (skip the task-pin safety check)
await eigen.deleteBranch("feature-x", { force: true });
```

See [chapter 4 §4.6](04-cli-reference.md#46-branch-commands-require---endpoint) for the equivalent CLI surface and [D23 §5.4–§5.5](../../design/d23-out-of-core-layer-architecture.md#54-write-model) for the design.

## 17.6. Error handling

Most methods return structured response objects with `success: bool` and an `errors: ValidationError[]` array — non-RPC failures (validation, parse, etc.) come back this way and the call doesn't throw.

Connect-RPC errors (network failures, kernel-side gRPC `Status` errors that the orchestrator translates) throw `ConnectError` (from `@connectrpc/connect`). The orchestrator's passthrough explicitly re-wraps kernel gRPC errors so the inbound Connect protocol carries the actual message — without this, browser callers see `[internal] HTTP 400` with the real message URL-encoded into a `grpc-message` header. See [`orchestration/src/notebook/eigenius_kernel_passthrough.ts`](../../../orchestration/src/notebook/eigenius_kernel_passthrough.ts).

## 17.7. Layout and dependencies

```
clients/eigenius-ts/
├── deno.jsonc            # Deno project config (npm: + jsr: imports)
├── package.json          # npm package shape (name, exports, deps)
├── mod.ts                # public API exports
├── src/
│   ├── client.ts         # Eigen class
│   └── notebook.ts       # NotebookJson <-> Resource[] translator + content-addressed IRIs
├── generated/            # buf-generated Connect stubs (do not edit)
│   └── eigenius_pb.ts    # generated by `npx buf generate`
└── examples/
    ├── smoke-test.ts     # exercises every RPC against a live stack
    ├── publish-test.ts   # publishNotebook end-to-end
    └── error-test.ts     # demonstrates the orchestrator's gRPC→Connect error translation
```

Runtime deps: `@bufbuild/protobuf@^2`, `@connectrpc/connect@^2`, `@connectrpc/connect-web@^2`. The notebook consumes the SDK as a `file:../clients/eigenius-ts` workspace dep; once the SDK stabilises, npm publication via `dnt` (Deno-to-Node) will follow.

## 17.8. Regenerating the proto stubs

The buf pipeline lives at the repo root ([`buf.yaml`](../../../buf.yaml) + [`buf.gen.yaml`](../../../buf.gen.yaml)). The SDK's `generated/` is one of two output targets (the other being the orchestrator's `src/gen/`). Regenerate after any change to [`proto/eigenius.proto`](../../../proto/eigenius.proto):

```bash
npx --yes @bufbuild/buf generate
```

Buf is pinned at v2; both targets emit `protoc-gen-es target=ts`, which produces idiomatic Connect-Web TypeScript stubs.

## 17.9. Smoke test

The smoke test at [`clients/eigenius-ts/examples/smoke-test.ts`](../../../clients/eigenius-ts/examples/smoke-test.ts) hits every RPC against a live stack:

```bash
cd clients/eigenius-ts
deno run --allow-net --allow-env examples/smoke-test.ts
# expects http://localhost:8080 by default; override with EIGENIUS_ORCHESTRATOR=...
```

Output is a 7-step transcript: `health → inspect → query → listInstitutions → layerTopology (taxonomy + full) → load → validateProgram`. This is the SDK's Phase 1 acceptance criterion (D22 §5.7).

## 17.10. Design references

- [**D22** — Notebook UX and TypeScript SDK](../../design/d22-notebook-and-typescript-sdk.md) — full SDK + notebook spec, including the layer-topology data shape and the planned post-MVP additions
- [**D5** — gRPC API specification](../../design/d5-grpc-api-specification.md) — the underlying RPC surface (the SDK is a thin wrapper)
- [**chapter 14** — Notebook](14-notebook.md) — the SDK's largest consumer

---

Next: **[18. Appendix →](18-appendix.md)**
