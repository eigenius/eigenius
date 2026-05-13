# `@eigenius/client`

TypeScript SDK for the Eigenius platform. Wraps the orchestrator's
`EigeniusKernel` and `NotebookService` Connect-RPC surfaces in a single typed
`Eigen` class. Targets browser, Deno, and Node consumers; the
[notebook UI](../../notebooks/) consumes it as a `file:` workspace dep, and the
same code runs from a Deno script or any Node app.

Per [D22 §5](../../docs/design/d22-notebook-and-typescript-sdk.md). The
user-facing reference is
**[platform guide chapter 14](../../docs/guides/platform/14-typescript-sdk.md)**
— read that first; this README is the package-shape companion.

## Layout

```
clients/eigenius-ts/
├── deno.jsonc            # Deno project config (npm: + jsr: imports)
├── package.json          # npm package shape (name, exports, deps)
├── mod.ts                # public API exports
├── src/
│   ├── client.ts         # Eigen class — main entry point
│   └── notebook.ts       # NotebookJson <-> Resource[] translator (content-addressed IRIs)
├── generated/            # buf-generated Connect stubs (do not edit)
│   └── eigenius_pb.ts
└── examples/
    ├── smoke-test.ts     # exercises every RPC against a live stack
    ├── publish-test.ts   # publishNotebook end-to-end
    └── error-test.ts     # demonstrates the orchestrator's gRPC→Connect error translation
```

## Quick use

```typescript
import { Eigen } from "@eigenius/client";

const eigen = new Eigen({ endpoint: "http://localhost:8080" });

// Liveness
const h = await eigen.health();

// Walk the layer chain
const topo = await eigen.layerTopology();
console.log(`${topo.nodes.length} nodes, ${topo.edges.length} edges`);

// Compile + commit ESL
const { layerId } = await eigen.load(
  `namespace ex = "urn:eigenius:demo:ex"; ...`,
);

// Run a program already in the chain by IRI
const result = await eigen.runProgramByIri(
  "urn:eigenius:demo:patent:analyze_patent",
  "urn:eigenius:demo:patent:US10452978B2",
);
```

The `Eigen` class wraps:

| Method             | RPC                                |
| ------------------ | ---------------------------------- |
| `health`           | `EigeniusKernel.Health`            |
| `inspect`          | `EigeniusKernel.Inspect`           |
| `query`            | `EigeniusKernel.Query`             |
| `load`             | `EigeniusKernel.Load`              |
| `validateProgram`  | `EigeniusKernel.ValidateProgram`   |
| `runProgram`       | `EigeniusKernel.RunProgram`        |
| `runProgramByIri`  | `EigeniusKernel.RunProgramByIri`   |
| `listInstitutions` | `EigeniusKernel.ListInstitutions`  |
| `layerTopology`    | `NotebookService.LayerTopology`    |
| `publishNotebook`  | translator → `EigeniusKernel.Load` |

Plus notebook-format types and translators (`NotebookJson`, `CellJson`,
`notebookJsonToResources`, `resourcesToNotebookJson`).

Worked examples for each method live in
[platform chapter 14 §14.3](../../docs/guides/platform/14-typescript-sdk.md#143-five-line-examples).

## Smoke test

```bash
cd clients/eigenius-ts
deno run --allow-net --allow-env examples/smoke-test.ts
```

Expects an orchestrator at `http://localhost:8080`; override with
`EIGENIUS_ORCHESTRATOR=...`. Output is a 7-step transcript exercising every
public method (`health` → `inspect` → `query` → `listInstitutions` →
`layerTopology` → `load` → `validateProgram`).

## Regenerating stubs

The buf pipeline lives at the repo root ([`buf.yaml`](../../buf.yaml) +
[`buf.gen.yaml`](../../buf.gen.yaml)). The SDK's `generated/` is one of two
output targets (the other being the orchestrator's `src/gen/`). Regenerate after
any change to [`proto/eigenius.proto`](../../proto/eigenius.proto):

```bash
npx --yes @bufbuild/buf generate
```

## Status

The SDK is feature-complete for the notebook MVP (D22 phases 1–4d):

- ✅ All EigeniusKernel + NotebookService methods needed by the notebook
- ✅ `RunProgramByIri` for the natural "one program × N inputs" pattern
- ✅ Notebook-publish translator with content-addressed IRIs (Cell + Notebook)
- ✅ Error translation: orchestrator re-wraps kernel gRPC errors as Connect
  errors so messages decode cleanly in browsers (otherwise they'd surface as
  `[internal] HTTP 400`)
- ⏳ npm publication via `dnt` (Deno-to-Node) — once the SDK stabilises
- ⏳ Typed `Resource` / `ResultSet` wrappers per D22 §5.4 — currently consumers
  decode CBOR ad-hoc; see
  [`notebooks/src/runtime/resultDocument.ts`](../../notebooks/src/runtime/resultDocument.ts)
  for a worked example
- ⏳ `RunProgramByIri` per-field content types (proto currently uses one
  `content_type` for both program and input — workaround: load both first, then
  call by IRI)

## Design references

- [**D22** — Notebook UX and TypeScript SDK](../../docs/design/d22-notebook-and-typescript-sdk.md)
  — full spec (SDK + notebook)
- [**D5** — gRPC API specification](../../docs/design/d5-grpc-api-specification.md)
  — the underlying RPC surface
- [**Platform guide chapter 13** — Notebook](../../docs/guides/platform/13-notebook.md)
  — the SDK's largest consumer
- [**Platform guide chapter 14** — TypeScript SDK](../../docs/guides/platform/14-typescript-sdk.md)
  — the user-facing reference
