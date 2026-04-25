# `@eigenius/client`

TypeScript SDK for the Eigenius platform. Wraps the orchestrator's `EigeniusKernel` and `NotebookService` Connect surfaces in a single typed `Eigen` class. Targets browser and Deno consumers; npm publication via `dnt` will follow once the SDK stabilises.

Per [D22 §5](../../docs/design/d22-notebook-and-typescript-sdk.md).

## Layout

```
clients/eigenius-ts/
├── deno.jsonc               # Deno project config
├── mod.ts                   # public API exports
├── src/
│   ├── client.ts            # Eigen class — main entry point
│   └── topology.ts          # Topology / TopologyNode / TopologyEdge re-exports
├── generated/               # buf-generated Connect stubs (do not edit)
└── examples/
    └── smoke-test.ts        # Phase 1 acceptance — exercises every exposed RPC
```

## Quick use

```typescript
import { Eigen } from "jsr:@eigenius/client";  // future JSR target
// or, for now, while developing locally:
// import { Eigen } from "../../clients/eigenius-ts/mod.ts";

const eigen = new Eigen({ endpoint: "http://localhost:8080" });

const topo = await eigen.layerTopology();
console.log(`${topo.nodes.length} nodes, ${topo.edges.length} edges`);
```

## Regenerating stubs

The buf pipeline lives at the repository root (`buf.yaml` + `buf.gen.yaml`). The SDK's `generated/` directory is one of buf's output targets, so regeneration is the same as for the orchestrator:

```bash
just generate
```

## Status

Phase 1 (per D22): `layerTopology` only. The Eigen class will grow to wrap `inspect`, `query`, `load`, `compile`, `run`, `listInstitutions` (which all already exist on `EigeniusKernel`) plus future browser-specific methods on `NotebookService` as Phase 2–4 progresses.
