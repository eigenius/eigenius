# Eigenius

An open-source platform for **AI-driven science and engineering**.

Contemporary LLMs produce text that reads like knowledge but carries no epistemic warranty — there is no structural way to distinguish a correct derivation from a convincing hallucination. Eigenius addresses this by anchoring knowledge in a typed, queryable knowledge graph where every fact has tracked provenance, every derivation is replayable, and formal proofs provide machine-checked certainty.

The platform maintains three epistemic categories: **observed** knowledge (facts with provenance), **derived** knowledge (conclusions from typed pipelines with full audit trails), and **verified** knowledge (derivations with machine-checked formal proofs). For frontier research in quantum physics, life sciences, materials science, and beyond, this distinction makes it possible to know what has been truly verified versus what is plausible-sounding text without proper grounding.

## Current Status: Phase 0 + Phase 1 Complete

The core data model, layer system, validation engine, query language, and CLI are implemented. The system can:

- Parse and serialize Eigon-JSON documents
- Load the self-describing core ontology (classes, properties, data types, formats)
- Build immutable layers with content-addressed identifiers (SHA-256)
- Validate resources against the full ontology constraint system (12 validation rules)
- Resolve resources through parent-pointer layer chains
- Query the knowledge graph with EigenQL (typed stratified Datalog with aggregation)

See [docs/design/implementation-plan.md](docs/design/implementation-plan.md) for the full phased build plan.

## Architecture

Everything in Eigenius is a **Resource** — classes, properties, data types, formats, and instance data are all represented uniformly with IRI identity and typed property values. The core ontology is self-describing: `Class` is an instance of `Class`.

- **Rust Kernel** — ontology validation, layer management, resource resolution. Uses `BTreeMap` for deterministic ordering and cache-friendly access.
- **Layer System** — immutable layers with parent pointers (`Arc<Layer>`), forming a chain. The root layer holds the core ontology. Resolution walks the chain top-down.
- **Eigon-JSON** — the canonical serialization format. `@id` is the only reserved key; all property keys are full IRIs. Three-layer type system: primitive data types, format constraints, and content types.
- **Validation** — 12 rules: required properties, inheritance, type checking, format/pattern validation, range/length constraints, class type checking, allowed values, domain checking, conditional requirements, open-world extra properties.
- **EigenQL** — typed stratified Datalog with aggregation. Supports USING, MATCH (typed/untyped/negated patterns), WHERE, GROUP BY, RETURN (with COUNT/SUM/AVG/MIN/MAX), ORDER BY, LIMIT/OFFSET, DISTINCT, DEFINE (recursive rules with seminaive fixpoint), dot-path navigation, NOT EXISTS. Full pipeline: lex → parse → stratify → type_check → evaluate.

Future phases add: DAG pipelines with dependent type checking (Mini-TT/NbE), gRPC service with TiKV storage, LLM integration with reasoning traces, and WASM capability sandboxing.

See [docs/design/architecture-v0.3.md](docs/design/architecture-v0.3.md) for the full architecture specification.

## Repository Structure

```
kernel/          Rust kernel crate
  src/ontology/    IRI, Resource, Value, Eigon-JSON parser, well-known constants
  src/layer/       Layer, LayerBuilder, LayerId (content-addressed)
  src/validation/  Validator with 12 validation rules
  src/query/       EigenQL: lexer, parser, type checker, stratification, evaluator
  src/context/     ExecutionContext (snapshot isolation, read/write control)
  src/bootstrap/   Core ontology loader and system initialization
  src/storage/     Storage interface traits (LayerStore, ResourceStore)
storage/         Storage backend implementations
  memory/          In-memory backend (BTreeMap + Arc<RwLock>)
  sqlite/          SQLite backend (placeholder)
  tikv/            TiKV backend (placeholder)
cli/             Command-line interface (load, validate, query, inspect)
ontologies/      Ontology definitions
  core/            Core ontology (core-ontology.json) — self-describing bootstrap
  examples/        Example ontologies (animals.json)
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

# Inspect a core ontology resource
cargo run -p eigenius-cli -- inspect "urn:eigenius:core:Class"

# Version
cargo run -p eigenius-cli -- version
```

## Design Documents

| Document | Description |
|----------|-------------|
| [D1: Eigon Serialization Format](docs/design/d1-eigon-serialization-format.md) | Eigon-JSON spec: IRI identity, three-layer type system, validation rules, canonical form |
| [D2: EigenQL Specification](docs/design/d2-eigenql-specification.md) | EigenQL spec: typed stratified Datalog, DEFINE, aggregation, full grammar |
| [Implementation Plan](docs/design/implementation-plan.md) | High-level 6-phase plan from foundation to extensibility |
| [Architecture v0.3](docs/design/architecture-v0.3.md) | Full architecture specification |

## License

Apache-2.0
