# Eigenius

An open-source platform for **AI-driven science and engineering**.

Contemporary LLMs produce text that reads like knowledge but carries no epistemic warranty — there is no structural way to distinguish a correct derivation from a convincing hallucination. Eigenius addresses this by anchoring knowledge in a typed, queryable knowledge graph where every fact has tracked provenance, every derivation is replayable, and formal proofs provide machine-checked certainty.

The platform maintains four epistemic categories: **declared** knowledge (human assertions), **observed** knowledge (facts with provenance), **derived** knowledge (conclusions from typed pipelines with full audit trails), and **verified** knowledge (derivations with machine-checked formal proofs). For frontier research in quantum physics, life sciences, materials science, and beyond, this distinction makes it possible to know what has been truly verified versus what is plausible-sounding text without proper grounding.

## Current Status: Phases 0-4 Complete

The platform is operational end-to-end: kernel, orchestrator, LLM integration, and CLI connected via gRPC. The system can:

- Parse and serialize Eigon-JSON and CBOR documents
- Load the self-describing core, program, and reflection ontologies (130+ resources across 3 layers)
- Build immutable layers with content-addressed identifiers (SHA-256 of CBOR)
- Validate resources against the full ontology constraint system (12 validation rules)
- Resolve resources through parent-pointer layer chains
- Query the knowledge graph with EigenQL (typed stratified Datalog with aggregation)
- Type-check programs using Mini-TT dependent type theory (NbE evaluator)
- Execute programs with local and remote IO components (LLM calls via orchestrator)
- Dispatch IO components to the Deno orchestrator via gRPC (ComponentExecutor service)
- Call LLMs via Vercel AI SDK (Anthropic) with prompt templating and metrics
- Expose kernel operations as MCP tools for LLM agents
- Track four epistemic categories: declared, observed, derived, verified
- Record tree-structured reasoning traces with memoization
- Validate epistemic base class requirements (DeclaredResource, DerivedResource, etc.)
- Persist layers in RocksDB with CBOR serialization
- Serve the kernel as a gRPC service (tonic) with streaming query results
- Run locally via three terminals or Docker Compose

See [docs/design/implementation-plan.md](docs/design/implementation-plan.md) for the full phased build plan.

## Architecture

Everything in Eigenius is a **Resource** — classes, properties, data types, formats, and instance data are all represented uniformly with IRI identity and typed property values. The core ontology is self-describing: `Class` is an instance of `Class`.

- **Rust Kernel** — ontology validation, layer management, resource resolution, program execution, gRPC server. Uses `BTreeMap` for deterministic ordering and cache-friendly access.
- **Deno Orchestrator** — IO component dispatch, LLM integration (Vercel AI SDK), MCP server. Communicates with the kernel via Connect RPC/gRPC.
- **Layer System** — immutable layers with parent pointers (`Arc<Layer>`), forming a chain. Three bootstrap layers: core → program → reflection. Resolution walks the chain top-down.
- **Eigon-JSON / CBOR** — the canonical serialization formats. `@id` is the only reserved key; all property keys are full IRIs. Three-layer type system: primitive data types, format constraints, and content types. CBOR for storage and gRPC wire format.
- **Validation** — 12 rules: required properties, inheritance, type checking, format/pattern validation, range/length constraints, class type checking, allowed values, domain checking, conditional requirements, open-world extra properties. Epistemic base classes enforce provenance requirements.
- **EigenQL** — typed stratified Datalog with aggregation. Supports USING, MATCH (typed/untyped/negated patterns), WHERE, GROUP BY, RETURN (with COUNT/SUM/AVG/MIN/MAX), ORDER BY, LIMIT/OFFSET, DISTINCT, DEFINE (recursive rules with seminaive fixpoint), dot-path navigation, NOT EXISTS. Full pipeline: lex → parse → stratify → type_check → evaluate.
- **Program Model** — programs are typed expressions (Let, Apply, Lambda, Case, Map, Reduce, etc.) that map 1:1 to Mini-TT terms. Type-checked via NbE (Normalization by Evaluation) with Eigon ontology types as ground types. IO components dispatched to the orchestrator via gRPC with trace recording and memoization.
- **Epistemic Model** — four categories (declared, observed, derived, verified) enforced via base classes in the reflection ontology. Reasoning traces mirror the expression tree and serve as memoization cache.

Future phases add: ESL surface syntax (Phase 4.5) and WASM capability sandboxing (Phase 5).

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

The platform builds and runs on Linux (native or Windows with WSL 2)
and macOS. The demo rig is a Rust kernel, a Deno orchestrator, and a
CLI, all tied together by gRPC. Optional pieces (WASM examples, GitHub
issue workflow, Docker-based deployment) add their own tools.

**Core toolchain (required)**

- Rust (stable, **1.95+** — matches `deploy/Dockerfile.kernel`; earlier
  versions fail to build wasmtime 43 which the WASM runtime depends
  on). Install via [rustup](https://rustup.rs).
- [Deno](https://deno.land) — orchestration layer (`orchestration/`).
- System packages (Ubuntu / WSL 2):
  ```bash
  sudo apt-get install -y build-essential pkg-config libssl-dev protobuf-compiler libclang-dev
  ```
  - `build-essential` — C/C++ toolchain for RocksDB's native sources.
  - `pkg-config` + `libssl-dev` — TiKV client dependency.
  - `protobuf-compiler` — `protoc` for the gRPC build scripts.
  - `libclang-dev` — bindgen needs it to compile RocksDB headers.
- [`just`](https://github.com/casey/just) (task runner, optional but
  matches the commands in this README):
  ```bash
  cargo install just
  ```

**WASM examples (optional)**

Needed to build / modify the fixtures under `examples/wasm-*`:

```bash
rustup target add wasm32-unknown-unknown
cargo install cargo-component
```

**GitHub workflow (optional, recommended)**

The project tracks correctness hazards and phase work as GitHub issues.
The [`gh` CLI](https://cli.github.com) is the usual entrypoint for
reading / filing them:

```bash
# Ubuntu / WSL 2
sudo apt-get install -y gh
gh auth login
```

**Docker (optional)**

The end-to-end demo can run entirely in containers — skips Rust and
Deno on the host. Install Docker Engine and Compose v2 per your
distribution's instructions; then see the [Docker Compose](#docker-compose)
section below.

**Note for WSL 2 users:** all of the above installs into the WSL
distribution (Ubuntu or similar), not Windows itself. VS Code's WSL
remote extension is the smoothest way to edit the repo from Windows
while compiling inside WSL 2.

### Build and Test

```bash
just build        # or: cargo build --workspace
just test         # or: cargo test --workspace + deno test
just check        # fmt + clippy + deno lint
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

## ESL — Eigenius Surface Language

ESL is a human-friendly surface syntax that compiles to Eigon-JSON. It uses a two-layer design: HCL-style blocks for structural declarations (classes, properties, resources) and ML-style expressions for program bodies.

```esl
namespace core = "urn:eigenius:core";
namespace demo = "urn:eigenius:demo";

class demo:Document {
    description = "A text document for analysis.";
    requires demo:text;
}

property demo:text : core:string {
    description = "The text content of a document.";
}

resource demo:doc_001 : demo:Document {
    demo:text = "Eigenius is a typed knowledge graph platform.";
}

program demo:summarize : demo:Document -> demo:Document {
    let summary : core:string = CompleteText(input);
    Construct demo:Document { demo:text = summary }
}
```

This compiles to the equivalent of 80+ lines of Eigon-JSON. All CLI commands accept `.esl` files directly — the format is auto-detected by file extension.

```bash
# Compile ESL to Eigon-JSON (output to stdout)
cargo run -p eigenius-cli -- compile demo/document.esl

# Load and validate an ESL file
cargo run -p eigenius-cli -- load demo/document.esl

# Validate without loading
cargo run -p eigenius-cli -- validate demo/document.esl
```

The kernel's gRPC service also accepts ESL via `content_type: "application/esl"`.

See [docs/design/d7-esl-surface-syntax.md](docs/design/d7-esl-surface-syntax.md) for the full specification.

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
2. Load a document (Eigon-JSON) into the kernel
3. Inspect the core `Class` resource
4. Query all classes across core, program, and reflection ontologies
5. Run a summarization program (JSON) that dispatches `CompleteText` to the orchestrator
6. Load an ESL ontology directly into the kernel
7. Run an ESL program against the kernel

### Patent Analysis Demo

A two-step LLM pipeline that demonstrates CompleteJson (structured extraction) and CompleteText (narrative generation) working together:

1. Load a patent ontology (ESL) defining `PatentClaim`, `PatentAnalysis`, and `PatentBrief` classes
2. Load a patent document (the "Attention Is All You Need" transformer patent)
3. Run a pipeline that extracts structured analysis via CompleteJson, generates a plain-language summary via CompleteText, and combines them into a `PatentBrief`

```bash
./demo/patent/run.sh
```

The patent ontology (`demo/patent/patent-ontology.esl`) defines:
- **PatentClaim** — input: title, patent number, abstract text (+ optional assignee, filing date)
- **PatentAnalysis** — structured output: invention category, technical domain, key innovations, practical applications, prior art, limitations
- **PatentBrief** — final output: plain-language summary + structured analysis

The program (`demo/patent/analyze-patent.esl`) chains two LLM calls:
```
PatentClaim → CompleteJson → PatentAnalysis → CompleteText → string → Construct → PatentBrief
```

### Individual commands

You can also run individual commands against the kernel:

```bash
# Load resources (JSON or ESL)
cargo run -p eigenius-cli -- --endpoint http://localhost:50051 load demo/document.json
cargo run -p eigenius-cli -- --endpoint http://localhost:50051 load demo/document.esl

# Run a program (JSON or ESL)
cargo run -p eigenius-cli -- --endpoint http://localhost:50051 run demo/summarize-program.json demo/input.json
cargo run -p eigenius-cli -- --endpoint http://localhost:50051 run demo/summarize.esl demo/input.json

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
| [D7: ESL Surface Syntax](docs/design/d7-esl-surface-syntax.md) | Two-layer design: HCL-style structural + ML-style expressions |
| [D8: CompleteJson Component](docs/design/d8-complete-json-component.md) | Structured LLM output via JSON Schema from ontology classes |
| [Implementation Plan](docs/design/implementation-plan.md) | High-level 6-phase plan from foundation to extensibility |
| [Architecture v0.3](docs/design/architecture-v0.3.md) | Full architecture specification |

## License

Apache-2.0
