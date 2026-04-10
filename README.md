# Eigenius

An open-source platform for **AI-driven science and engineering**.

Contemporary LLMs produce text that reads like knowledge but carries no epistemic warranty — there is no structural way to distinguish a correct derivation from a convincing hallucination. Eigenius addresses this by anchoring knowledge in a typed, queryable knowledge graph where every fact has tracked provenance, every derivation is replayable, and formal proofs provide machine-checked certainty.

The platform maintains three epistemic categories: **observed** knowledge (facts with provenance), **derived** knowledge (conclusions from typed pipelines with full audit trails), and **verified** knowledge (derivations with machine-checked formal proofs). For frontier research in quantum physics, life sciences, materials science, and beyond, this distinction makes it possible to know what has been truly verified versus what is plausible-sounding text without proper grounding.

## Architecture

- **Rust Kernel** — native binary with Verus proof annotations. Handles ontology validation, type checking (Mini-TT / NbE), layer management, capability dispatch, and the reflection layer.
- **Deno Orchestration Layer** — TypeScript service for DAG execution, LLM adapter management, and MCP server surface.
- **TiKV Storage** — distributed ordered key-value store for production deployments. SPO/POS/OPS triple indexes built and maintained by the Eigenius host layer.
- **EigenQL** — typed semantic query language (conjunctive queries v1, with extension path to recursive Datalog).

See [docs/design/architecture-v0.3.md](docs/design/architecture-v0.3.md) for the full architecture specification.

## Repository Structure

```
kernel/          Rust kernel crate (native binary + WASM capability sandbox)
storage/         Storage backend implementations (TiKV, SQLite, in-memory)
orchestration/   Deno/TypeScript orchestration layer
cli/             Command-line interface
ontologies/      Core Ontology and Foundation Layer definitions
proto/           gRPC protobuf definitions
deploy/          Azure ContainerApps deployment (Dockerfiles, Bicep IaC)
docs/            Architecture, design documents, and test plans
lean/            Lean 4 formal specification track
```

## Getting Started

### Prerequisites

- Rust (stable, 1.82+)
- Deno (2.x)
- Protobuf compiler (`protoc`)

### Build

```bash
# Rust kernel and CLI
cargo build --workspace

# Run tests
cargo test --workspace

# Deno orchestration
cd orchestration && deno test
```

### CLI

```bash
# Local mode (embedded kernel + SQLite)
eigenius --local load ontologies/core/core.eigon
eigenius --local query "MATCH ?c a :Class RETURN ?c"

# Remote mode (connect to kernel service)
eigenius --endpoint http://localhost:50051 query "MATCH ?c a :Class RETURN ?c"
```

## Development

See [docs/design/implementation-plan.md](docs/design/implementation-plan.md) for the phased build plan.

## License

Apache-2.0
