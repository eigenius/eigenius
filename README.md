# Eigenius

An open-source platform for **AI-driven science and engineering**.

Contemporary LLMs produce text that reads like knowledge but carries no epistemic warranty — there is no structural way to distinguish a correct derivation from a convincing hallucination. Eigenius addresses this by anchoring knowledge in a typed, queryable knowledge graph where every fact has tracked provenance, every derivation is replayable, and formal proofs provide machine-checked certainty.

The platform maintains four epistemic categories: **declared** knowledge (human assertions), **observed** knowledge (facts with provenance), **derived** knowledge (conclusions from typed pipelines with full audit trails), and **verified** knowledge (derivations with machine-checked formal proofs). For frontier research in quantum physics, life sciences, materials science, and beyond, this distinction makes it possible to know what has been truly verified versus what is plausible-sounding text without proper grounding.

## Current Status: Phases 0, 1, and 2 Complete

The core data model, layer system, validation engine, query language, program model with dependent type checking, and CLI are implemented. The system can:

- Parse and serialize Eigon-JSON documents
- Load the self-describing core ontology and program ontology (86 resources across 2 layers)
- Build immutable layers with content-addressed identifiers (SHA-256)
- Validate resources against the full ontology constraint system (12 validation rules)
- Resolve resources through parent-pointer layer chains
- Query the knowledge graph with EigenQL (typed stratified Datalog with aggregation)
- Type-check programs using Mini-TT dependent type theory (NbE evaluator)
- Execute programs with built-in components

See [docs/design/implementation-plan.md](docs/design/implementation-plan.md) for the full phased build plan.

## Architecture

Everything in Eigenius is a **Resource** — classes, properties, data types, formats, and instance data are all represented uniformly with IRI identity and typed property values. The core ontology is self-describing: `Class` is an instance of `Class`.

- **Rust Kernel** — ontology validation, layer management, resource resolution. Uses `BTreeMap` for deterministic ordering and cache-friendly access.
- **Layer System** — immutable layers with parent pointers (`Arc<Layer>`), forming a chain. The root layer holds the core ontology. Resolution walks the chain top-down.
- **Eigon-JSON** — the canonical serialization format. `@id` is the only reserved key; all property keys are full IRIs. Three-layer type system: primitive data types, format constraints, and content types.
- **Validation** — 12 rules: required properties, inheritance, type checking, format/pattern validation, range/length constraints, class type checking, allowed values, domain checking, conditional requirements, open-world extra properties.
- **EigenQL** — typed stratified Datalog with aggregation. Supports USING, MATCH (typed/untyped/negated patterns), WHERE, GROUP BY, RETURN (with COUNT/SUM/AVG/MIN/MAX), ORDER BY, LIMIT/OFFSET, DISTINCT, DEFINE (recursive rules with seminaive fixpoint), dot-path navigation, NOT EXISTS. Full pipeline: lex → parse → stratify → type_check → evaluate.
- **Program Model** — programs are typed expressions (Let, Apply, Lambda, Case, Map, Reduce, etc.) that map 1:1 to Mini-TT terms. Type-checked via NbE (Normalization by Evaluation) with Eigon ontology types as ground types. Executed with a component registry (built-in + future WASM extensions).

Future phases add: gRPC service with TiKV storage, LLM integration with reasoning traces, and WASM capability sandboxing.

See [docs/design/architecture-v0.3.md](docs/design/architecture-v0.3.md) for the full architecture specification.

### Docker Compose

The demo can also run via Docker Compose without installing Rust or Deno locally.

```bash
# Build and start both services (mock LLM, no API key needed):
EIGENIUS_MOCK_LLM=true docker compose up --build -d

# Run the demo:
./demo/run.sh

# With a real LLM:
docker compose down
ANTHROPIC_API_KEY=sk-ant-... docker compose up -d
./demo/run.sh

# Stop:
docker compose down
```

## Repository Structure

```
kernel/          Rust kernel crate
  src/ontology/    IRI, Resource, Value, Eigon-JSON parser, well-known constants
  src/layer/       Layer, LayerBuilder, LayerId (content-addressed)
  src/validation/  Validator with 12 validation rules
  src/query/       EigenQL: lexer, parser, type checker, stratification, evaluator
  src/nbe/         Mini-TT type theory: terms, values, eval, readback, type checker
  src/program/     Program model: expression parser, ground type resolution, executor
  src/context/     ExecutionContext (snapshot isolation, read/write control)
  src/bootstrap/   Core + program ontology loader and system initialization
  src/storage/     Storage interface traits (LayerStore, ResourceStore)
storage/         Storage backend implementations
  memory/          In-memory backend (BTreeMap + Arc<RwLock>)
  sqlite/          SQLite backend (placeholder)
  tikv/            TiKV backend (placeholder)
cli/             Command-line interface (load, validate, query, program-validate, run, inspect)
ontologies/      Ontology definitions
  core/            Core ontology (core-ontology.json) — self-describing bootstrap
  program/         Program ontology (program-ontology.json) — expression classes, components
  examples/        Example ontologies and programs
docs/design/     Design documents
  d1-eigon-serialization-format.md   Eigon-JSON format specification
  implementation-plan.md             High-level 6-phase plan
  architecture-v0.3.md               Full architecture specification
deploy/          Azure ContainerApps deployment (Dockerfiles, Bicep IaC)
proto/           gRPC protobuf definitions
orchestration/   Deno/TypeScript orchestration layer (future)
```

## Getting Started

### Prerequisites

- Rust (stable, 1.82+)
- `pkg-config` and `libssl-dev` (for TiKV client dependency)

```bash
# Ubuntu/WSL
sudo apt-get install -y build-essential pkg-config libssl-dev protobuf-compiler
```

### Build and Test

```bash
cargo build --workspace
cargo test --workspace
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets
```

### CLI

```bash
# Validate an Eigon-JSON file against the core ontology
cargo run -p eigenius-cli -- validate ontologies/examples/animals.json

# Load an Eigon-JSON file (validates and commits as a new layer)
cargo run -p eigenius-cli -- load ontologies/examples/animals.json

# Query the knowledge graph with EigenQL
cargo run -p eigenius-cli -- query 'USING "urn:eigenius:core:Class" MATCH Class(?c) { short_name: ?name } RETURN [] { short_name: ?name }'

# Query with a loaded file
cargo run -p eigenius-cli -- query --file ontologies/examples/animals.json 'MATCH "urn:eigenius:example:Dog"(?d) { "urn:eigenius:example:name": ?name } RETURN [] { "urn:eigenius:example:name": ?name }'

# Type-check a program
cargo run -p eigenius-cli -- program-validate ontologies/examples/simple-program.json --ontology ontologies/examples/animals.json

# Execute a program with input data
cargo run -p eigenius-cli -- run ontologies/examples/simple-program.json ontologies/examples/animals.json --ontology ontologies/examples/animals.json

# Inspect a core ontology resource
cargo run -p eigenius-cli -- inspect "urn:eigenius:core:Class"

# Version
cargo run -p eigenius-cli -- version
```

## Running the End-to-End Demo

The demo loads a document, runs a program that dispatches to an LLM via the orchestrator, and returns a typed result. Requires three terminals.

### Prerequisites

```bash
# Rust kernel
cargo build -p eigenius-cli

# Deno orchestrator
cd orchestration && deno cache src/main.ts && cd ..

# API key (or use mock mode)
export ANTHROPIC_API_KEY=sk-ant-...
```

### Terminal 1: Start the orchestrator

```bash
cd orchestration
ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY deno run --allow-net --allow-env src/main.ts
```

For testing without an API key, use mock mode:

```bash
cd orchestration
EIGENIUS_MOCK_LLM=true deno run --allow-net --allow-env src/main.ts
```

### Terminal 2: Start the kernel

```bash
cargo run -p eigenius-cli -- serve --orchestrator http://localhost:8080
```

### Terminal 3: Run the demo

```bash
./demo/run.sh
```

This will:
1. Health-check the orchestrator
2. Load a document into the kernel
3. Inspect the core `Class` resource
4. Query all classes across core, program, and reflection ontologies
5. Run a summarization program that dispatches `CompleteText` to the orchestrator

You can also run individual commands against the kernel:

```bash
# Load resources
cargo run -p eigenius-cli -- --endpoint http://localhost:50051 load demo/document.json

# Run a program
cargo run -p eigenius-cli -- --endpoint http://localhost:50051 run demo/summarize-program.json demo/input.json

# Query
cargo run -p eigenius-cli -- --endpoint http://localhost:50051 query 'MATCH "urn:eigenius:core:Class"(?c) { short_name: ?name } RETURN [] { class: ?c, name: ?name }'

# Inspect
cargo run -p eigenius-cli -- --endpoint http://localhost:50051 inspect "urn:eigenius:core:Class"
```

## Design Documents

| Document | Description |
|----------|-------------|
| [D1: Eigon Serialization Format](docs/design/d1-eigon-serialization-format.md) | Eigon-JSON spec: IRI identity, three-layer type system, validation rules, canonical form |
| [D2: EigenQL Specification](docs/design/d2-eigenql-specification.md) | EigenQL spec: typed stratified Datalog, DEFINE, aggregation, full grammar |
| [D3: Program Model](docs/design/d3-program-model.md) | Program expression language, component model, scheduling, ESL surface syntax |
| [Implementation Plan](docs/design/implementation-plan.md) | High-level 6-phase plan from foundation to extensibility |
| [Architecture v0.3](docs/design/architecture-v0.3.md) | Full architecture specification |

## License

Apache-2.0
