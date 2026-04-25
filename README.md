# Eigenius

An open-source platform for **AI-driven science and engineering**.

Contemporary LLMs produce text that reads like knowledge but carries no epistemic warranty — there is no structural way to distinguish a correct derivation from a convincing hallucination. Eigenius addresses this by anchoring knowledge in a typed, queryable knowledge graph where every fact has tracked provenance, every derivation is replayable, and formal proofs provide machine-checked certainty.

The platform maintains four epistemic categories: **declared** knowledge (human assertions), **observed** knowledge (facts with provenance), **derived** knowledge (conclusions from typed pipelines with full audit trails), and **verified** knowledge (derivations with machine-checked formal proofs). For frontier research in quantum physics, life sciences, materials science, and beyond, this distinction makes it possible to know what has been truly verified versus what is plausible-sounding text without proper grounding.

## User guides

Three task-first guides, grounded in the implementation:

- **[Platform user guide](docs/guides/platform/README.md)** — thirteen chapters on operating the platform: installation, build, CLI reference, running locally, database management, the orchestrator, end-to-end demos, building WASM components and institutions, deployment.
- **[ESL — Eigenius Surface Language](docs/guides/esl/README.md)** — eleven chapters on the declarative surface (`namespace`, `class`, `property`, `resource`, `data`, `codata`, `program`) and the ML-style expression sublanguage. Most important chapter: [chapter 6 — Resources, types, and the layer](docs/guides/esl/06-resources-types-and-the-layer.md), the bridge between the resource graph and the kernel's type theory.
- **[EigenQL — query language](docs/guides/eigenql/README.md)** — twelve chapters on pattern matching, derived relations, expressions, `FIBER` institution dispatch, stratification, and the result-document format.

Landing page: **[docs/guides/](docs/guides/README.md)**.

## Current Status: Phases 0–11e Complete

The platform is operational end-to-end: kernel, orchestrator, LLM integration, and CLI connected via gRPC. The system can:

- Parse and serialize Eigon-JSON and CBOR documents
- Load the self-describing core, program, reflection, and institution ontologies (4 bootstrap layers)
- Build immutable layers with content-addressed identifiers (SHA-256 of CBOR)
- Validate resources against the full ontology constraint system (12 validation rules)
- Resolve resources through parent-pointer layer chains
- Query the knowledge graph with EigenQL (typed stratified Datalog with aggregation)
- Type-check programs using Mini-TT dependent type theory (NbE evaluator)
- Execute programs with local and remote IO components (LLM calls via orchestrator)
- Dispatch IO components to the Deno orchestrator via gRPC (ComponentExecutor service)
- Call LLMs via Vercel AI SDK (Anthropic) with prompt templating and metrics
- Generate structured LLM output via CompleteJson (JSON Schema from ontology classes)
- Expose kernel operations as MCP tools for LLM agents
- Track four epistemic categories: declared, observed, derived, verified
- Record tree-structured reasoning traces with memoization and incremental execution
- Validate epistemic base class requirements (DeclaredResource, DerivedResource, etc.)
- Persist layers, traces, and WASM capabilities in RocksDB — survives kernel restart
- Serve the kernel as a gRPC service (tonic) with streaming query results
- Compile ESL (Eigenius Surface Language) to Eigon-JSON — all CLI commands accept `.esl` files
- Register Grothendieck institutions with fiber reasoners and morphism validation
- Run untrusted WASM capabilities sandboxed via Wasmtime (components and institutions)
- Model coinductive types (codata/streams) and resumable tasks with checkpointing
- Resolve ontology classes as kernel types on demand via the layer chain (Phase 10, D18)
- Type-check inductive types with bounded binders for sized termination, plus self-referential parameterised codata for productivity by typing (Phase 11b, D19)
- Use `Map` and `Reduce` as type-level primitives with structural-recursion termination (Phase 11a)
- Fire institution-registered decide procedures at type-check time via property constraints (Phase 11c)
- Declare cross-institution `Comorphism` translations as first-class ontology resources, invocable from program bodies and from EigenQL (Phase 11d, 11e.1, 11e.2)
- Dispatch qualified-name function calls (`cap:predicate(...)`, `cap:translate(...)`) through a single institution-classification table shared by ESL and EigenQL (Phase 11e)
- Run locally via three terminals or Docker Compose

See [docs/design/implementation-plan.md](docs/design/implementation-plan.md) for the full phased build plan.

## Architecture

Everything in Eigenius is a **Resource** — classes, properties, data types, formats, and instance data are all represented uniformly with IRI identity and typed property values. The core ontology is self-describing: `Class` is an instance of `Class`.

- **Rust Kernel** — ontology validation, layer management, resource resolution, program execution, gRPC server. Uses `BTreeMap` for deterministic ordering and cache-friendly access.
- **Deno Orchestrator** — IO component dispatch, LLM integration (Vercel AI SDK), MCP server. Communicates with the kernel via Connect RPC/gRPC.
- **Layer System** — immutable layers with parent pointers (`Arc<Layer>`), forming a chain. Four bootstrap layers: core → program → reflection → institution. Resolution walks the chain top-down.
- **Eigon-JSON / CBOR** — the canonical serialization formats. `@id` is the only reserved key; all property keys are full IRIs. Three-layer type system: primitive data types, format constraints, and content types. CBOR for storage and gRPC wire format.
- **Validation** — 12 rules: required properties, inheritance, type checking, format/pattern validation, range/length constraints, class type checking, allowed values, domain checking, conditional requirements, open-world extra properties. Epistemic base classes enforce provenance requirements.
- **EigenQL** — typed stratified Datalog with aggregation. Supports USING, MATCH (typed/untyped/negated patterns), WHERE, GROUP BY, RETURN (with COUNT/SUM/AVG/MIN/MAX), ORDER BY, LIMIT/OFFSET, DISTINCT, DEFINE (recursive rules with seminaive fixpoint), dot-path navigation, NOT EXISTS. Full pipeline: lex → parse → stratify → type_check → evaluate.
- **Program Model** — programs are typed expressions (Let, Apply, Lambda, Case, Map, Reduce, etc.) that map 1:1 to Mini-TT terms. Type-checked via NbE (Normalization by Evaluation) with Eigon ontology types as ground types. IO components dispatched to the orchestrator via gRPC with trace recording and memoization.
- **Epistemic Model** — four categories (declared, observed, derived, verified) enforced via base classes in the reflection ontology. Reasoning traces mirror the expression tree and serve as memoization cache.
- **Grothendieck Institutions** — domain-specific reasoning systems contribute structured fibers to the knowledge graph. Each institution provides its own sentences, models, satisfaction relation, and internal morphisms via the `FiberReasoner` trait. Cross-institution queries and comorphism translations.
- **WASM Extensibility** — untrusted capabilities run sandboxed via Wasmtime. Components and institution fiber reasoners can be delivered as WASM modules with fuel/memory limits. Capability SDK for authors.
- **Durable State** — `eigenius serve --db <path>` persists layers, traces, and WASM capabilities in RocksDB. Restart rebuilds running state; embedded ontologies seeded with SHA-256 manifest and drift-refusal.
- **Codata and Tasks** — coinductive types (codata/corecord/observation) for streams. Programs run as tracked tasks with checkpointing, positional trace keys, and startup resume sweep for crash recovery.

Next phase: Phase 12 (Worked Institution Examples — life-science worked examples drawing on the Phase 11 inductive/codata/institution-dispatch surface).

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
  src/esl/         ESL compiler: lexer, parser, compiler to Eigon-JSON
  src/capability/  WASM capability hosting, ComponentRegistry, FiberReasoner dispatch
  src/context/     ExecutionContext (snapshot isolation, read/write control)
  src/bootstrap/   Ontology loader and system initialization (4 bootstrap layers)
  src/storage/     Storage interface traits (LayerStore, ResourceStore)
  src/task/        Task model: TaskRecord, Checkpoint, resume sweep
storage/         Storage backend implementations
  memory/          In-memory backend (BTreeMap + Arc<RwLock>)
  rocksdb/         RocksDB backend (durable layers, traces, capabilities)
  tikv/            TiKV backend (placeholder)
  indexing/        SPO/POS/OPS triple index construction
crates/
  wasm-runtime/    Wasmtime integration for WASM capability sandboxing
sdk/
  wasm-sdk/        Rust SDK for authoring WASM capabilities
examples/        WASM capability examples (excluded from workspace, built with cargo-component)
  wasm-cbor-echo/            CBOR echo component
  wasm-doc-validator/        Document validation component
  wasm-http-shout/           IO component with HTTP dispatch
  wasm-ordering-institution/ Ordering institution fiber reasoner
  wasm-read-query-probe/     Read-capability query probe
cli/             Command-line interface (load, validate, query, run, serve, tasks, capability, ...)
ontologies/      Ontology definitions
  core/            Core ontology (core-ontology.json) — self-describing bootstrap
  program/         Program ontology (program-ontology.json) — expression classes, components
  examples/        Example ontologies and programs
docs/design/     Design documents (D1–D21)
deploy/          Azure ContainerApps deployment (Dockerfiles, Bicep IaC)
proto/           gRPC protobuf definitions
orchestration/   Deno/TypeScript orchestration layer
demo/            End-to-end demo scripts
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

**WASM examples (required for tests)**

Needed to build the WASM fixtures under `examples/wasm-*` that kernel tests depend on via `include_bytes!`:

```bash
rustup target add wasm32-unknown-unknown
cargo install cargo-component
```

Once installed, `just build` (or `just build-wasm`) builds all WASM examples and copies the fixtures into `kernel/tests/fixtures/`.

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
just build        # build workspace + WASM examples + copy test fixtures
just test         # cargo test --workspace + deno test
just check        # fmt + clippy + deno lint
```

`just build` runs `cargo component build` for each WASM example and copies the resulting `.wasm` files into `kernel/tests/fixtures/` before building the workspace. To rebuild only the WASM fixtures:

```bash
just build-wasm
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
| [D4: Storage Key Encoding](docs/design/d4-storage-key-encoding.md) | Key encoding for RocksDB/TiKV, column families, index layout |
| [D5: gRPC API Specification](docs/design/d5-grpc-api-specification.md) | RPC definitions, streaming query, error codes, CLI/orchestration integration |
| [D6: Execution Architecture](docs/design/d6-execution-architecture.md) | Kernel-orchestrator boundary, activity dispatch, MCP server placement |
| [D6b: Reasoning Trace Schema](docs/design/d6b-reasoning-trace-schema.md) | Trace classes, provenance chain, epistemic status, universe stratification |
| [D7: ESL Surface Syntax](docs/design/d7-esl-surface-syntax.md) | Two-layer design: HCL-style structural + ML-style expressions |
| [D8: CompleteJson Component](docs/design/d8-complete-json-component.md) | Structured LLM output via JSON Schema from ontology classes |
| [D9: NbE Unification](docs/design/d9-nbe-unification-and-type-extensions.md) | Capability modes, type theory extensions, ground type resolution, trace storage |
| [D10: Grothendieck Institutions](docs/design/d10-grothendieck-institution-protocol.md) | FiberReasoner trait, institution registry, comorphisms, fiber query dispatch |
| [D11: Codata and Streams](docs/design/d11-codata-streams.md) | Coinductive types, stream semantics, tasks as codata, guardedness checking |
| [D12: WASM Extensibility](docs/design/d12-wasm-extensibility.md) | WASM module lifecycle, host imports, capability levels, fuel/memory limits |
| [D13: Durable Kernel State](docs/design/d13-durable-kernel-state.md) | `serve --db` flag, seeded bootstrap, drift-refusal, restart re-registration |
| [D18: Ontology-as-Types Resolution](docs/design/d18-ontology-as-types-resolution.md) | `find_sigma_field` layer-chain resolution, `CheckCtx`, inference-mode rules |
| [D19: Inductive and Sized Types](docs/design/d19-inductive-types.md) | Inductive types, sized termination via bounded binders, self-referential parameterised codata, productivity by typing |
| [D21: Task Traces and Checkpointing](docs/design/d21-task-traces-and-checkpointing.md) | Per-task trace keys, checkpoint primitive, resume sweep, task RPCs |
| [Implementation Plan](docs/design/implementation-plan.md) | Phased build plan (Phases 0–15) |
| [Architecture v0.3](docs/design/architecture-v0.3.md) | Full architecture specification |
| [Lean 4 as Institution](docs/design/lean-4-as-institution.md) | Integration plan for the Lean 4 proof checker as an Eigenius institution (uses [nanoda_lib](https://github.com/ammkrn/nanoda_lib)) |

## License

Apache-2.0
