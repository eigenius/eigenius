<p align="center">
  <img src="docs/guides/assets/eigenius_logo_400x400.png" alt="Eigenius" width="200">
</p>

# Eigenius

An open-source **AI platform for science and engineering** built on a typed, queryable knowledge graph.

Contemporary LLMs produce text that reads like knowledge but carries no epistemic warranty — there is no structural way to distinguish a correct derivation from a convincing hallucination. Eigenius addresses this by anchoring knowledge in a typed, queryable knowledge graph where every fact has tracked provenance, every derivation is replayable, and formal proofs provide machine-checked certainty.

The platform maintains four epistemic categories: **declared** knowledge (human assertions), **observed** knowledge (facts with provenance), **derived** knowledge (conclusions from typed pipelines with full audit trails), and **verified** knowledge (derivations with machine-checked formal proofs). For frontier research in quantum physics, life sciences, materials science, and beyond, this distinction makes it possible to know what has been truly verified versus what is plausible-sounding text without proper grounding.

> This is still a very early stage of this project. Anticipate
> features not working or missing functionality overall. Our goal
> is to close those quality gaps rather aggressively. Feel free
> to submit issues in the discussion forum or directly as issue.

Key features that we still need to wire up:

- Completion of storage graph management and versioning. [Branching and
  trivial merges have been implemented but are not yet exposed in the
  notebook interface](docs/design/d23-out-of-core-layer-architecture.md). [Layer reconciliation](docs/design/d20-layer-reconciliation.md) and [chain consolidation](docs/design/d25-chain-consolidation.md) still
  need to be implemented. [Garbage collection across graph layers has been
  implemented](docs/design/d23-out-of-core-layer-architecture.md), but it has yet to be integrated into the application 
  life-cycle.
- The [generic runtime substrate](docs/design/d26-runtime-substrate.md) exists, but language-specific worker implementations still need to be implemented. In addition, we need to provide institution-level
  integration into appropriate languages ([Julia Institutions](docs/design/d27-julia-institutions.md) [`Symbolics`](https://juliasymbolics.org/) / [`ModelingToolkit`](https://github.com/SciML/ModelingToolkit.jl), [`Catalyst`](https://docs.sciml.ai/Catalyst/stable/) and [`JuMP`](https://jump.dev/); [Lean-4 as theorem prover](docs/design/d28-lean-4-as-institution.md)).

## The notebook — start here

For most users, the notebook is the most accessible way to use the platform. It is a React SPA the orchestrator serves at `http://localhost:8080/notebooks/`; cells run ESL, EigenQL, TypeScript, and program invocations against the live kernel; outputs auto-render as typed inspectors, result tables, layer-stack diagrams, and program-trace trees.

<p align="center">
  <img src="docs/guides/assets/eigenius_notebook_ux.png" alt="The Eigenius notebook — top of the patent-analysis demo" width="900">
</p>

If you have the docker stack up (`docker compose up -d`), the notebook is already there — it's bundled into the orchestrator image at build time and serves alongside the RPC paths on the same origin. Open the URL above and the patent-analysis demo loads on first mount; click **Run all** and watch ESL compile + commit a layer, EigenQL produce a result table, and the program-run cell drive the kernel through a two-step LLM pipeline (`CompleteJson` → structured patent analysis, `CompleteText` → plain-language summary) with the resulting brief and an interactive trace tree rendered side-by-side.

See **[chapter 13 — Notebook](docs/guides/platform/13-notebook.md)** for the full reference. The graphing capabilities of the notework interface can
be explored by importing the [Kinase Assay](notebooks/examples/kinase-screening.json) example.

The same SDK that powers the notebook ([`@eigenius/client`](clients/eigenius-ts/)) is usable programmatically from any TypeScript runtime — see **[chapter 14 — TypeScript SDK](docs/guides/platform/14-typescript-sdk.md)**.

## User guides

Three task-first guides plus a consolidated bibliography, all grounded in the implementation:

- **[Platform user guide](docs/guides/platform/README.md)** — fifteen chapters on operating the platform: installation, build, CLI reference, running locally, database management, the orchestrator, end-to-end demos, building WASM components and institutions, deployment, **the notebook UX**, **the TypeScript SDK**.
- **[ESL — Eigenius Surface Language](docs/guides/esl/README.md)** — eleven chapters on the declarative surface (`namespace`, `class`, `property`, `resource`, `data`, `codata`, `program`) and the ML-style expression sublanguage. Most important chapter: [chapter 6 — Resources, types, and the layer](docs/guides/esl/06-resources-types-and-the-layer.md), the bridge between the resource graph and the kernel's type theory.
- **[EigenQL — query language](docs/guides/eigenql/README.md)** — twelve chapters on pattern matching, derived relations, expressions, `FIBER` institution dispatch, stratification, and the result-document format.
- **[References](docs/guides/references/README.md)** — consolidated bibliography for the platform: works actually cited in design docs / papers / guides, foundational works the system relies on, philosophical and methodological precursors, and contemporary related work. Generated from the BibTeX files in [`docs/references/`](docs/references/) by `scripts/bib-to-md.py`; verified against Crossref / arXiv / live URLs by `scripts/verify-citations.py`.

Guides landing page: **[docs/guides/](docs/guides/README.md)**. Full documentation index (guides + design documents + papers): **[docs/](docs/README.md)**.

## Current Status: Phases 0–11e + D22 Notebook & SDK + D14 Institution Realisation Complete

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
- Declare institutions, export/import boundary formats, query classes, and triadic comorphisms as ontology resources committed to the layer chain (D14 §3–§5)
- Fire `Decidable` `QueryClass`es at type-check time, returning a `Verdict` projected to the kernel's reduction (`Holds` → `Refl(v)`, `Fails` → failing neutral, `Undecidable` → passthrough) (D14 §9.2)
- Auto-register WASM institutions from layer scan: any `Institution` resource with `runtime: wasm` + inline `wasm_binary` is hosted by the kernel without an explicit install step (D14 §3, registration code in `kernel/src/capability/registration.rs`)
- Dispatch qualified-name function calls through a single `InstitutionIndex` shared by ESL and EigenQL (D14 §9.5); ESL emits `Exp::NativeDecide` returning `Verdict`; EigenQL adds postfix `HOLDS` / `FAILS` / `UNDECIDABLE` to project to Boolean
- Run cross-institution comorphism coercion inline inside FIBER param values (`param: comorphism_iri(source)`) — a four-step extract → transform → reify pipeline (D14 §9.3)
- Run locally via three terminals or Docker Compose
- Drive the platform from a React notebook (six cell types: markdown, ESL, EigenQL, TypeScript, program-run, and form-based chart cells covering grouped-bar / vertical-bar / horizontal-bar / donut / line / area; auto-rendered outputs; layer-stack and per-layer topology graph visualisations; cell-order Run / Run-from-here / Run-to-here with stale markers; collapse/expand; content-addressed publish-to-layer with a queryable Open dialog; bundled into the orchestrator image and served at `/notebooks/`)
- Use the same kernel from any TypeScript runtime via `@eigenius/client` — a typed SDK over the Connect-RPC surface (browser, Deno, Node)

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
- **Grothendieck Institutions (D14)** — domain-specific reasoning systems contribute structured fibres to the knowledge graph. Each institution is *declared* as ontology resources (`Institution`, `ExportFormat`, `ImportFormat`, `QueryClass`, `Comorphism`) committed to the layer chain, and *implemented* via the three-method `Institution` trait (`extract_typed` / `reify` / `query`). Comorphisms are triadic — source-side export + cross-institution Mini-TT transformation + target-side import — with optional `exact: bool` Satisfaction-Condition annotation. The category-theoretic Grothendieck construction emerges from declared comorphisms; the kernel provides the dispatch and well-typedness machinery.
- **WASM Extensibility** — untrusted capabilities run sandboxed via Wasmtime. Components and D14 institutions can be delivered as WASM modules with fuel/memory limits. WASM institutions targeting the `eigenius-institution-d14` WIT world auto-register from chain scan when their declaration carries `runtime: wasm` + `wasm_binary`. SDK builders for the five declaration shapes.
- **Durable State** — `eigenius serve --db <path>` persists layers, traces, and WASM capabilities in RocksDB. Restart rebuilds running state; embedded ontologies seeded with SHA-256 manifest and drift-refusal.
- **Codata and Tasks** — coinductive types (codata/corecord/observation) for streams. Programs run as tracked tasks with checkpointing, positional trace keys, and startup resume sweep for crash recovery.

Phase 12 in progress: D14 Institution Realisation landed (M1–M8 milestones complete — ontology shapes, derived registry, trait surface, WIT world + SDK rewrite, four-step `Exp::InstitutionInvoke` pipeline, `NativeDecide` dispatch, AutoOnLoad on Load, the M8 dock-assay worked example end-to-end through both in-process and WASM-hosted variants). Next: docs sweep + further worked examples drawing on the Phase 11 inductive/codata/institution-dispatch surface.

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

#### Inspecting the kernel's persistent state

The kernel writes its RocksDB store to a named docker volume (`eigenius_db`)
mounted at `/var/lib/eigenius/db` inside the kernel container. The volume
survives `docker compose down`; use `docker compose down -v` to wipe it (the
next `up` re-seeds at schema v1).

```bash
# Peek at the on-disk RocksDB files via the running kernel container
docker compose exec kernel ls -la /var/lib/eigenius/db
docker compose exec kernel du -sh /var/lib/eigenius/db

# Or attach a throwaway alpine container with just the volume mounted
# (useful when the kernel container won't start):
docker run --rm -it -v eigenius_eigenius_db:/data alpine sh
# inside: ls -la /data ; du -sh /data ; exit

# Show what docker thinks it knows about the volume
docker volume ls | grep eigenius_db
docker volume inspect eigenius_eigenius_db

# Inspect platform state through the kernel API (preferred — RocksDB's SST
# files are not directly readable; go through the kernel surface):
eigenius --endpoint http://localhost:50051 branch list
eigenius --endpoint http://localhost:50051 branch show main

# Reset the dev DB and start fresh:
docker compose down -v          # wipes the volume
docker compose up -d            # next `up` re-seeds at schema v1
```

The volume name on the host is `<project>_eigenius_db` — typically
`eigenius_eigenius_db` if you run from the repo root. See
[D24 — Schema Versioning](docs/design/d24-schema-versioning.md) for what
gets stamped at seed time and why a `down -v` reset is sometimes needed
after a kernel upgrade.

#### Reading the kernel logs

The kernel uses [`tracing`](https://docs.rs/tracing/) and emits structured
JSON when running in docker (no TTY). Standard `docker compose logs`
controls visibility; `RUST_LOG` and `EIGENIUS_LOG_FORMAT` control verbosity
and shape.

```bash
# All kernel logs since startup
docker compose logs kernel

# Follow (Ctrl-C to stop)
docker compose logs -f kernel

# Last N lines, then exit
docker compose logs --tail=100 kernel

# Bounded by time
docker compose logs --since 5m kernel

# Both services side-by-side
docker compose logs -f kernel orchestrator

# Pipe structured JSON through jq
docker compose logs --no-log-prefix kernel | jq 'select(.fields.operation)'

# Just RPC failures
docker compose logs --no-log-prefix kernel | jq 'select(.fields.error_kind)'
```

For more verbose output during debugging, add to the `kernel.environment`
block in `docker-compose.yml`:

```yaml
- RUST_LOG=eigenius_kernel=debug,info
- EIGENIUS_LOG_FORMAT=pretty   # human-readable instead of JSON
```

`eigenius_kernel=debug` turns on per-RPC and per-chain-walk events; `info`
keeps the rest of the workspace at the default level. `trace` is rarely
useful — that's where high-volume per-resource events live.

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
  src/capability/  WASM capability hosting, ComponentRegistry, WasmInstitution (D14), chain-scan auto-registration
  src/institution/ D14 Institution trait, InstitutionIndex (chain-derived), InstitutionRuntime, AutoOnLoad dispatch
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
  wasm-read-query-probe/     Read-capability query probe
  wasm-d14-echo/             Minimum-viable D14 institution (smoke test of WIT bindings)
  wasm-d14-dock/             Dock institution for the M8 worked example
  wasm-d14-assay/            Assay institution for the M8 worked example
  wasm-d14-arrhenius/        Arrhenius transformation Component (the m of dock_to_assay)
cli/             Command-line interface (load, validate, query, run, serve, tasks, capability, ...)
ontologies/      Ontology definitions
  core/            Core ontology (core-ontology.json) — self-describing bootstrap
  program/         Program ontology (program-ontology.json) — expression classes, components
  reflection/      Reflection ontology (reasoning traces, derivation, epistemic status)
  institution/     Institution ontology (D14: Institution / ExportFormat / ImportFormat / QueryClass / Comorphism / Verdict)
  notebook/        Notebook ontology (Notebook + Cell + CellType — backs `Publish` from the UI)
  examples/        Example ontologies and programs
notebooks/       React notebook SPA (D22 — six cell types incl. charts, layer/topology graphs, publish-to-layer) — bundled into the orchestrator image
clients/
  eigenius-ts/     `@eigenius/client` — TypeScript SDK that wraps the orchestrator's RPC surface
docs/design/     Design documents (D1–D22)
deploy/          Azure ContainerApps deployment (Dockerfiles, Bicep IaC)
proto/           gRPC protobuf definitions
orchestration/   Deno/TypeScript orchestration layer (LLM dispatch, MCP server, notebook static-file route)
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
| [D11: Codata and Streams](docs/design/d11-codata-streams.md) | Coinductive types, stream semantics, tasks as codata, guardedness checking |
| [D12: WASM Extensibility](docs/design/d12-wasm-extensibility.md) | WASM module lifecycle, host imports, capability levels, fuel/memory limits |
| [D13: Durable Kernel State](docs/design/d13-durable-kernel-state.md) | `serve --db` flag, seeded bootstrap, drift-refusal, restart re-registration |
| [D14: Institution Realisation](docs/design/d14-institution-realisation.md) | Institution trait (extract_typed/reify/query), ontology-first declarations, triadic comorphisms, Verdict shape, dispatch model. Supersedes D10. |
| [D18: Ontology-as-Types Resolution](docs/design/d18-ontology-as-types-resolution.md) | `find_sigma_field` layer-chain resolution, `CheckCtx`, inference-mode rules |
| [D19: Inductive and Sized Types](docs/design/d19-inductive-types.md) | Inductive types, sized termination via bounded binders, self-referential parameterised codata, productivity by typing |
| [D21: Task Traces and Checkpointing](docs/design/d21-task-traces-and-checkpointing.md) | Per-task trace keys, checkpoint primitive, resume sweep, task RPCs |
| [D22: Notebook UX and TypeScript SDK](docs/design/d22-notebook-and-typescript-sdk.md) | The React notebook, the `Eigen` SDK, the notebook ontology, content-addressed publish |
| [D23: Out-of-Core Layer Architecture](docs/design/d23-out-of-core-layer-architecture.md) | Phase 14: topology/content split, per-layer blooms, branches + CAS, multi-parent merges, GC, per-layer triple index |
| [D24: Schema Versioning Policy](docs/design/d24-schema-versioning.md) | On-disk schema versioning: kernel `SCHEMA_VERSION`, migration framework, boot-time check, contributor checklist. Companion: [Schema Changelog](docs/design/schema-changelog.md). |
| [Implementation Plan](docs/design/implementation-plan.md) | Phased build plan (Phases 0–15) |
| [Architecture v0.3](docs/design/architecture-v0.3.md) | Full architecture specification |
| [D26: Runtime Substrate](docs/design/d26-runtime-substrate.md) | Language-agnostic substrate for embedding scientific-computation runtimes (Julia, Python, R, …) into Eigenius. Resource classes, image-vs-graph boundary, container-digest-anchored deployment, mirror generators. |
| [D27: Julia Institutions](docs/design/d27-julia-institutions.md) | Julia as the first runtime-substrate instance, plus three reference institutions wrapping Julia libraries with their own fibers: `Symbolics` / `ModelingToolkit`, `JuMP`, `IntervalArithmetic`. The future Lean / Julia bridge. |
| [D28: Lean 4 as Verification Institution](docs/design/d28-lean-4-as-institution.md) | Integration plan for the Lean 4 proof checker as an Eigenius institution (uses [nanoda_lib](https://github.com/ammkrn/nanoda_lib)) |

## License

Apache-2.0
