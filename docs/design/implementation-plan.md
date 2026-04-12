# Eigenius Implementation Plan

*Derived from eigenius-architecture-v0.3.md — April 2026*

**Target:** Solo / 1–2 developer build. Sequential milestones with each phase producing a deployable, testable artifact. GitHub-based development, Azure ContainerApps deployment.

---

## 1. Repository Structure

A single monorepo (`eigenius`) with workspace-level tooling:

```
eigenius/
├── kernel/                    # Rust kernel crate (native binary + WASM target)
│   ├── src/
│   │   ├── ontology/          # Core Ontology, Eigon structural types (§3)
│   │   ├── layer/             # Layer system, resolution, commit (§7)
│   │   ├── context/           # Execution context, snapshot binding (§8)
│   │   ├── capability/        # Capability protocol, dispatch, sandbox (§9)
│   │   ├── storage/           # Storage interface traits (§10.6)
│   │   ├── reflection/        # Reasoning traces, universe stratification (§11)
│   │   ├── nbe/               # Mini-TT type theory, NbE evaluator (§4.2–4.6)
│   │   ├── bootstrap/         # Foundation Layer loader, capability primer (§2.5)
│   │   └── api/               # gRPC service definitions (Load/Query/Validate/Reflect)
│   ├── tests/
│   └── Cargo.toml
├── storage/                   # Storage backend implementations
│   ├── tikv/                  # TiKV backend crate (§10.7)
│   ├── sqlite/                # SQLite/LibSQL backend crate
│   ├── memory/                # In-memory backend (testing)
│   └── indexing/              # SPO/POS/OPS triple index construction (§10.8)
├── orchestration/             # Deno/TypeScript orchestration layer (§2.2)
│   ├── src/
│   │   ├── dag/               # DAG execution engine (§12)
│   │   ├── llm/               # LLM adapters, provider abstraction (§2.3)
│   │   ├── mcp/               # MCP server — LLM→Core tool surface (§2.3)
│   │   └── client/            # gRPC client for kernel service
│   ├── deno.json
│   └── tests/
├── cli/                       # Command-line interface
│   ├── src/                   # Rust CLI binary (clap-based)
│   └── Cargo.toml
├── ontologies/                # Ontology definitions
│   ├── core/                  # Core Ontology (urn:eigenius:core:) in Eigon/ESL
│   └── foundation/            # Foundation Layer (urn:eigenius:foundation:)
├── proto/                     # Protobuf definitions for kernel gRPC API
├── deploy/                    # Azure ContainerApps deployment
│   ├── Dockerfile.kernel      # Kernel service container
│   ├── Dockerfile.orchestration  # Deno orchestration container
│   ├── bicep/                 # Azure Bicep IaC templates
│   └── .github/               # Symlinked or referenced CI workflows
├── docs/                      # Design documents (see §8 of this plan)
│   ├── design/
│   └── test-plans/
├── .github/
│   └── workflows/
│       ├── ci.yml             # PR checks: cargo test, cargo clippy, deno test
│       ├── release.yml        # Build containers, push to ACR, deploy to ContainerApps
│       └── formal.yml         # Optional: Lean 4 proof checks (when track matures)
├── lean/                      # Lean 4 formal specification track (§2.4)
│   └── Eigenius/
├── Cargo.toml                 # Rust workspace root
└── README.md
```

---

## 2. Phase Overview

The build is organized into six phases. Each phase produces a working system that can be tested end-to-end. Later phases extend earlier ones; nothing is thrown away.

| Phase | Name | What it proves | Deliverables |
|-------|------|----------------|-------------- |
| 0 | Foundation | Eigon types exist, layers work, storage round-trips | Kernel with in-memory storage, unit tests |
| 1 | Query | EigenQL v1 evaluates against a layer stack | EigenQL parser + evaluator, CLI `query` command |
| 2 | Pipelines | DAGs type-check and execute | NbE type checker, DAG execution in orchestration layer |
| 3 | Service | Kernel runs as a gRPC service with TiKV | Containerized kernel, Azure deployment, CLI talks to service |
| 4 | Intelligence | LLM integration works bidirectionally | LLM adapter Components, MCP server, reflection traces |
| 5 | Extensibility | Untrusted capabilities run sandboxed | WASM capability sandbox via Wasmtime, domain ontology loading |

---

## 3. Phase 0 — Foundation ✓

**Goal:** The core data model exists, layers stack, resources resolve, and everything round-trips through storage. This is the skeleton that every subsequent phase builds on.

**Duration estimate:** 4–6 weeks. **Completed:** April 11, 2026 (1 day, with Claude Code).

### 3.1 Deliverables

- All data represented uniformly as `Resource` instances (no separate `Class`/`Property`/`DataType` Rust structs) — classes, properties, data types, formats, and instances are all `Resource` values distinguished by their `is_a` property (design doc D1)
- Core Ontology loaded from `ontologies/core/core-ontology.json` as the root layer — the self-describing bootstrap in Eigon-JSON format (§3.1, §3.6, design doc D1)
- Layer system: immutable `Layer` with parent pointers (`Arc<Layer>`), forming a chain. `LayerBuilder` accumulates resources and `build()` produces an immutable layer with a content-addressed `LayerId` (SHA-256). Resolution walks the parent chain (§7.1–7.5)
- Execution context: `ExecutionContext` with snapshot binding, layer stack reference, read/read-write modes (§8.1–8.2)
- Storage interface traits: `LayerStore`, `CapabilityStore`, `BlobStore` as Rust traits (§10.6)
- In-memory storage backend implementing all three traits (§10.7)
- Namespace validation: IRI parsing, namespace ownership checks, `urn:eigenius:core:` protection (§6.1, §6.3). Identifiers are IRIs (RFC 3987) using the `urn:` scheme. Max IRI length 512 characters.
- Bootstrap sequence: hardcoded Core Ontology load + minimal Foundation Layer (§2.5) — no capabilities yet, just the structural scaffolding
- CLI skeleton: `eigenius` binary with `load` subcommand (loads an Eigon resource file, validates against Core Ontology, prints validation result)

### 3.2 Key decisions (resolved)

The following decisions have been made and documented in **design doc D1** (`docs/design/d1-eigon-serialization-format.md`):

- **Eigon serialization format:** Eigon-JSON — a custom JSON format inspired by Atomic Data. `@id` is the only reserved key; property keys are full IRIs. No `@context`, no JSON-LD. Class membership via `urn:eigenius:core:is_a` property (always an array). Three-layer type system: primitive data types, format constraints, and content types.
- **Identifier scheme:** IRIs (RFC 3987) using the registered `urn:` scheme (`urn:eigenius:<namespace>:<local-name>`). Not required to be fetchable — type resolution comes from the loaded ontology, not HTTP dereferencing.
- **Validation model:** Open-world (extra properties allowed). Classes declare `requires`/`recommends`. Subclasses inherit from ancestors. Conditional requirements via `conditional_requires`. Domain constraints restrict which classes a property may be used on. `class_types` and `allows_only` constrain resource-typed property values. `format`, `pattern`, `min_value`/`max_value`, `min_length`/`max_length` constrain primitive values.
- **Canonical form:** RFC 8785 (JSON Canonicalization Scheme) for content-addressed hashing.
- **Resource handle design:** Eager for Phase 0, refactor to lazy in Phase 3 when storage is remote.
- **Layer identifier scheme:** Content-addressed hash (SHA-256 of canonical form).

### 3.3 Test plan

- **Ontology self-description:** Load the Core Ontology from `ontologies/core/core-ontology.json`, verify that `Class` is an instance of `Class`, `Property` is an instance of `Class`, `is_a` is an instance of `Property`, all core properties validate against their declared data types, formats, domains, and conditional requirements.
- **Layer resolution:** Create a 3-layer chain (root → layer 2 → layer 3), place resources at different layers, verify resolution walks the parent chain and returns the correct (topmost) resource for each IRI.
- **Shadowing:** Verify that a resource in layer 3 shadows the same IRI in the root layer.
- **Immutability:** Commit a layer, attempt to modify it, verify rejection.
- **Namespace protection:** Attempt to create a resource under `urn:eigenius:core:` in a non-core layer, verify rejection.
- **Round-trip:** Create resources, commit a layer, read back from in-memory storage, verify byte-exact equality.
- **Conflict detection:** Two write contexts based on the same snapshot both modify the same resource; first commit succeeds, second detects conflict.

---

## 4. Phase 1 — Query ✓

**Goal:** EigenQL parses, type-checks, and evaluates against a populated layer chain. The system can answer questions about its own ontology, including recursive queries and aggregation.

**Duration estimate:** 4–6 weeks. **Completed:** April 11, 2026 (1 day, with Claude Code).

### 4.1 Deliverables

- EigenQL lexer and parser: full grammar per design doc D2 (`docs/design/d2-eigenql-specification.md`), producing a typed AST. Built with a Rust parser combinator library (nom, winnow, or chumsky). Supports:
  - USING (shortname imports), MATCH (typed/untyped/negated patterns), WHERE (expression filtering with NOT EXISTS), GROUP BY, RETURN (result shaping with aggregates), ORDER BY, LIMIT/OFFSET, DISTINCT
  - DEFINE (named derived relations with union semantics and recursive self-reference)
  - Dot-path navigation for embedded resources (`?person.address.city`)
  - Full IRI references without USING (`"urn:eigenius:example:Dog"(?d)`)
- EigenQL type checker: variable type inference from class constraints, expression type checking, aggregate/GROUP BY validation, stratification checking for negated DEFINE rules. Queries that fail type checking are rejected before evaluation.
- EigenQL evaluator:
  - Single-pass evaluation for non-recursive queries (conjunctive pattern matching against the layer chain, variable binding, WHERE filtering, aggregation, result shaping, result modifiers)
  - Bottom-up seminaive fixpoint evaluation for recursive DEFINE rules
  - Stratified evaluation ordering for negated patterns
- Built-in functions: DATE, TIMESTAMP, REGEX, LENGTH, CONTAINS, CONCAT
- Aggregate functions: COUNT, SUM, AVG, MIN, MAX
- Triple index construction on layer commit: SPO/POS/OPS indexes built as in-memory BTreeMap structures, used by the query evaluator for efficient pattern matching
- CLI `query` command: takes an EigenQL program string, evaluates it against the current layer chain, prints typed results
- Structured query error reporting with position, phase (lexer/parser/type_check/stratification/evaluation), rule, and message

### 4.2 Key decisions (resolved)

The following decisions have been made and documented in **design doc D2** (`docs/design/d2-eigenql-specification.md`):

- **Concrete syntax:** USING/MATCH/WHERE/GROUP BY/RETURN/ORDER BY/LIMIT/OFFSET/DISTINCT clause structure. DEFINE for derived relations. Keywords are case-sensitive uppercase.
- **Name resolution:** USING imports enable shortname references; full IRI as quoted string always available without USING. Property shortnames resolve against the matched class's property set.
- **Absence testing:** `NOT EXISTS(?var)` instead of `undefined` literal. Eigon-JSON has no null; absence is tested explicitly.
- **Dot-path navigation:** Shortname-only sugar over multi-pattern joins. Full IRI paths use decomposed multi-pattern queries.
- **Recursion:** DEFINE with self-reference, seminaive fixpoint evaluation. Multiple DEFINEs with the same name provide union semantics.
- **Negation:** Negated patterns (`NOT ClassName(...)`) in MATCH, with stratification checking to prevent negation cycles.
- **Aggregation:** COUNT, SUM, AVG, MIN, MAX with GROUP BY. Non-aggregated RETURN expressions must appear in GROUP BY.
- **Result modifiers:** DISTINCT, ORDER BY (ASC/DESC), LIMIT, OFFSET.
- **Monotonicity:** Queries without negation are monotonic. Queries with negation are flagged as non-monotonic for cache invalidation.
- **Query planner:** Simple left-to-right pattern matching strategy for v1 (no cost-based optimization).

### 4.3 Test plan

- **Parse round-trip:** Parse a query, serialize the AST, re-parse, verify equality.
- **Self-query:** Load Core Ontology, query "find all classes" — should return `Class`, `Property`, `DataType`, `Format`, `Encoding`, `ConditionalRequirement`.
- **Pattern matching:** Load the animals example ontology, query by class, by property value, by variable join across two patterns.
- **Full IRI references:** Query using only quoted IRI strings, no USING clause.
- **Dot-path navigation:** Query with `?person.address.city` and verify equivalent to decomposed multi-pattern form.
- **Guard expressions:** Query with WHERE clauses involving comparison, string matching, NOT EXISTS, IN.
- **NOT EXISTS:** Query for resources where an optional property is absent.
- **Aggregation:** COUNT dogs per breed with GROUP BY. SUM/AVG/MIN/MAX over numeric properties.
- **Result modifiers:** DISTINCT, ORDER BY ASC/DESC, LIMIT, OFFSET — verify correct ordering and pagination.
- **Recursive rules:** DEFINE a transitive Ancestor relation, query for all ancestors of a resource, verify fixpoint terminates and produces correct results.
- **Negated patterns:** DEFINE using NOT, verify stratification checking accepts valid programs and rejects negation cycles.
- **Layer-aware resolution:** Same query against two different layer chains returns different results based on which resources are visible.
- **Type checking:** A query referencing a non-existent class or property produces a structured error with position information.
- **Stratification errors:** A program with a negation cycle produces a stratification error before evaluation.
- **Performance baseline:** Query 10,000 resources with a 2-pattern join. Establish a baseline latency number for regression tracking.

---

## 5. Phase 2 — Pipelines

**Goal:** DAGs type-check against the Mini-TT dependent type system and execute through the orchestration layer. Partial evaluation works.

**Duration estimate:** 6–8 weeks.

### 5.1 Deliverables

- Mini-TT implementation in Rust: Pi types, Sigma types, labeled sums, closures, environments, the `Val`/`Neut` value representation (§4.2). Ported from the Haskell reference implementation.
- NbE evaluator: `eval`, `readback`, `check`/`checkI` (bidirectional type checking), `eqNf` (equality by normalization) (§4.6).
- DAG type system: Components, Pipe, Parallel, Select, Map, Retry as typed constructs (§4.3–4.4). Type signatures with `Result<A, E>` for fallibility (§4.5). `NonDeterministic` marker.
- DAG validator: takes a DAG specification and a typing context (derived from the layer stack's class definitions), runs the NbE type checker, reports type errors with source locations.
- DAG validator registered as a Foundation Layer capability alongside EigenQL.
- Partial evaluation: given a DAG and a subset of its inputs, produce a typed residual DAG (§4.9). Neutral terms (`Nt Neut`) represent unknown inputs.
- Ground type resolution interface: the bridge between Eigon ontology types and Mini-TT ground types (§4.10). Class references in DAG type signatures resolve against the layer stack.
- Orchestration layer (Deno/TypeScript): DAG execution engine that walks the validated DAG, executes Components (initially stub implementations), handles Pipe sequencing, Parallel fan-out, Select branching, Map iteration, Retry logic (§12.1–12.2).
- CLI `validate` command: takes a DAG specification file, validates it, prints type checking results.
- CLI `run` command: takes a validated DAG and input resources, executes it through the orchestration layer, prints output resources.

### 5.2 Key decisions required before coding

- DAG specification format: how are DAGs authored? ESL surface syntax? A YAML/JSON DSL? This needs a design document (see §8.2 of this plan).
- Component interface: what does a Component implementation look like from the orchestration layer's perspective? Input/output types, error handling, timeout contract.
- Orchestration ↔ kernel communication for Phase 2: before the gRPC service exists (Phase 3), the orchestration layer can embed the kernel as a Rust library via FFI or WASM. Recommendation: use the in-memory storage backend and call kernel functions directly via Deno FFI (`Deno.dlopen`) for Phase 2, then switch to gRPC in Phase 3.

### 5.3 Test plan

- **Mini-TT core:** Port the Mini-TT test suite from the Haskell implementation. Verify that well-typed terms check, ill-typed terms are rejected, and NbE produces expected normal forms.
- **DAG type checking:** A well-typed pipeline (Component → Pipe → Component) type-checks. A pipeline with a type mismatch (output type of step 1 ≠ input type of step 2) is rejected with a clear error.
- **Partial evaluation:** Provide 2 of 3 inputs to a DAG, verify the residual is a valid DAG with one remaining input. Execute the residual with the final input, verify the result matches full execution.
- **Select exhaustiveness:** A Select without a default branch is rejected. A Select with a default branch type-checks.
- **DAG execution:** A 3-step pipeline with stub Components executes end-to-end, producing typed output resources.
- **Error propagation:** A Component that returns `Err` triggers the Retry logic (if configured) or propagates the error.
- **Parallel execution:** A Parallel construct with 3 branches executes concurrently (verify via timing — parallel should be faster than sequential).

---

## 6. Phase 3 — Service

**Goal:** The kernel runs as a standalone gRPC service. TiKV is the storage backend. The CLI talks to the service over the network. Deployed to Azure ContainerApps.

**Duration estimate:** 4–6 weeks.

### 6.1 Deliverables

- gRPC API definition (protobuf): `Load`, `Query`, `Validate`, `Reflect` RPCs with typed request/response messages derived from Eigon classes. Defined in `proto/`.
- Kernel gRPC server: Rust binary using `tonic` that starts the kernel, runs the bootstrap sequence, connects to TiKV, and serves the four RPCs.
- TiKV storage backend: implements `LayerStore`, `CapabilityStore`, `BlobStore` against TiKV's Rust client. Key encoding scheme for SPO/POS/OPS indexes. Layer-prefixed key ranges.
- SQLite storage backend: for local development without TiKV.
- CLI refactored to gRPC client mode: `eigenius --endpoint <url> load|query|validate|inspect`. Retains a `--local` mode using the in-memory or SQLite backend for offline use.
- Dockerfile for kernel service: multi-stage Rust build, minimal runtime image.
- Dockerfile for orchestration layer: Deno-based image.
- Azure Bicep templates: ContainerApps Environment, kernel service container app, orchestration container app, Azure Container Registry, managed identity, TiKV connectivity (either self-hosted in Azure VMs or via a managed compatible service).
- GitHub Actions CI/CD: PR checks (cargo test, cargo clippy, deno test, deno lint), release workflow (build containers, push to ACR, deploy to ContainerApps).
- Health check and readiness probes on the kernel service.

### 6.2 Azure ContainerApps Architecture

```
┌─────────────────────────────────────────────────────────┐
│  Azure ContainerApps Environment                        │
│                                                         │
│  ┌──────────────────┐    ┌───────────────────────────┐  │
│  │  Kernel Service   │    │  Orchestration Service    │  │
│  │  (Rust native)    │◄───│  (Deno)                   │  │
│  │                   │    │                           │  │
│  │  gRPC :50051      │    │  HTTP :8080 (API gateway) │  │
│  │  Health :8081     │    │  MCP  :3000               │  │
│  └────────┬──────────┘    └───────────────────────────┘  │
│           │                                              │
└───────────┼──────────────────────────────────────────────┘
            │
            ▼
   ┌─────────────────┐
   │  TiKV Cluster    │
   │  (Azure VMs or   │
   │   AKS sidecar)   │
   └─────────────────┘
```

### 6.3 Key decisions required before coding

- TiKV hosting on Azure: self-managed on VMs, AKS-hosted with the TiKV Operator, or use a compatible managed service (PingCAP TiDB Cloud has a serverless tier). Design document needed (see §8.4).
- gRPC vs. Connect Protocol: tonic supports standard gRPC; `connect-go`/`connect-web` offers HTTP/1.1 compatibility. Recommendation: standard gRPC via tonic for kernel↔orchestration; consider Connect for the public-facing API gateway if browser clients need direct access.
- Authentication/authorization model for the service API: API keys, mTLS, Azure AD tokens? Scoped to Phase 3 as infrastructure, not the ontology-level security model (§13.2).

### 6.4 Test plan

- **Integration test:** Start kernel service with in-memory backend, send Load/Query/Validate RPCs via CLI, verify correct responses.
- **TiKV round-trip:** Start kernel with TiKV backend, load ontology, commit layers, restart kernel, verify data persists and queries return correct results.
- **Index correctness:** Load 1,000 resources, verify SPO/POS/OPS indexes in TiKV produce the same query results as the in-memory backend.
- **Container build:** Docker build succeeds, container starts, health check passes.
- **Azure deployment smoke test:** Deploy to ContainerApps staging environment, run the CLI integration test suite against the deployed endpoint.
- **Concurrent access:** Two CLI sessions loading resources and querying concurrently against the same kernel service — no corruption, snapshot isolation holds.

---

## 7. Phase 4 — Intelligence

**Goal:** LLMs can be invoked from DAGs and can invoke Eigenius as a tool. Reasoning traces are recorded and queryable.

**Duration estimate:** 4–6 weeks.

### 7.1 Deliverables

- LLM adapter Components in the orchestration layer: Anthropic Claude, OpenAI, configurable via provider resource in the ontology (§2.3). Using Vercel AI SDK or direct provider SDKs.
- MCP server in the orchestration layer: exposes Load, Query, Validate, Reflect as MCP tools (§2.3). An LLM agent can query the knowledge graph, validate pipelines, and record reasoning traces via tool-use.
- Reflection layer in the kernel: `ReasoningTrace` as a typed Eigon resource class (§11.2). Traces capture LLM invocations (prompt, completion, token usage, latency), DAG step results, and provenance links.
- Universe stratification enforcement in the reflection layer (§11.3): reasoning traces about resources at level N are recorded at level N+1.
- CLI `reflect` command: record a reasoning trace manually, query reasoning traces with EigenQL.
- End-to-end demo: a DAG that takes a document resource, invokes an LLM to summarize it, records the reasoning trace, and stores the summary as a typed resource — all queryable after execution.

### 7.2 Key decisions required before coding

- MCP transport: stdio (for local LLM integration) vs. SSE/HTTP (for remote agents). Recommendation: implement both; MCP SDKs support multiple transports.
- Reasoning trace schema: the specific classes and properties for traces. Design document needed (see §8.5).
- Token/cost tracking: whether token usage and cost estimates are first-class properties on reasoning trace resources.

### 7.3 Test plan

- **LLM adapter:** Mock LLM provider, execute a DAG with an LLM step, verify typed output resource matches the mock response.
- **MCP round-trip:** Start MCP server, connect an MCP client, invoke Query tool, verify correct typed response.
- **Reasoning trace persistence:** Execute a DAG, verify reasoning traces are committed as part of the layer, queryable via EigenQL.
- **Universe stratification:** A reasoning trace about a level-0 resource is at level 1. Attempt to create a level-0 trace about a level-0 resource — verify rejection.
- **Provenance query:** "Which LLM calls contributed to resource X?" — answered by querying reasoning traces.

---

## 8. Phase 5 — Extensibility

**Goal:** Third-party capability code runs in WASM sandboxes. Domain ontologies can register custom validators and evaluators safely.

**Duration estimate:** 4–6 weeks.

### 8.1 Deliverables

- Wasmtime integration in the kernel: instantiate a WASM module, provide the capability import interface, enforce memory and fuel limits (§9.6).
- Capability SDK: a Rust crate that capability authors compile to WASM. Provides typed bindings for reading resources from the execution context, emitting results, and declaring required external access.
- Capability registration via ontology: a domain layer can register a WASM module as a capability for a custom class.
- Domain ontology loading: load a third-party ontology layer that defines custom classes, properties, and WASM-sandboxed capabilities. Verify that it cannot shadow Foundation Layer capabilities (§9.5).
- CLI `capability` subcommand: list registered capabilities, inspect a capability's type signature, test-invoke a capability.
- Example domain ontology: a "Legal Document" ontology with a custom validator that checks document structure — delivered as a worked example and integration test.

### 8.2 Test plan

- **Sandbox isolation:** A WASM capability that attempts to access memory outside its linear memory — verify trap, no kernel corruption.
- **Fuel exhaustion:** A WASM capability with an infinite loop — verify termination within the fuel limit, error returned.
- **Interface control:** A WASM capability that attempts to make a network call — verify rejection (no network import provided).
- **Foundation protection:** Attempt to register a capability under `urn:eigenius:foundation:` from a domain layer — verify rejection.
- **End-to-end:** Load domain ontology with WASM capability, create a resource of the domain class, dispatch to the WASM capability, verify correct result.

---

## 9. Design Documents

The following design documents must be written and reviewed before the phase that depends on them. Each resolves open questions from the architecture document (§14) and makes decisions that code will implement.

| # | Document | Resolves | Required before | Estimated length |
|---|----------|----------|-----------------|-----------------|
| D1 | **Eigon Serialization Format** | **COMPLETED** — `docs/design/d1-eigon-serialization-format.md`. Eigon-JSON format, IRI identity, three-layer type system (data types/formats/content types), validation rules, canonical form, core ontology in `ontologies/core/core-ontology.json` | Phase 0 | Done |
| D2 | **EigenQL v1 Specification** | **COMPLETED** — `docs/design/d2-eigenql-specification.md`. Full EBNF grammar, lexer spec, type checking rules, aggregation (COUNT/SUM/AVG/MIN/MAX), GROUP BY, ORDER BY, LIMIT/OFFSET, DISTINCT, NOT EXISTS, dot-path navigation, error format | Phase 1 | Done |
| D3 | **DAG Specification Format** | How DAGs are authored: ESL surface syntax or a JSON/YAML DSL. Component signature declaration. | Phase 2 | 8–12 pages |
| D4 | **TiKV Key Encoding & Deployment** | Key encoding scheme (SPO/POS/OPS layout), TiKV region placement strategy, Azure hosting model (VMs, AKS, or TiDB Cloud) | Phase 3 | 10–15 pages |
| D5 | **gRPC API Specification** | Protobuf message definitions, streaming vs. unary RPCs, error codes, pagination for query results | Phase 3 | 8–10 pages |
| D6 | **Reasoning Trace Schema** | Ontology classes and properties for traces, provenance link structure, universe level assignment rules | Phase 4 | 6–8 pages |
| D7 | **Capability SDK & WASM Interface** | Import/export functions for WASM capabilities, resource serialization across the WASM boundary, fuel budget policy | Phase 5 | 10–12 pages |
| D8 | **Capability Protocol Wire Format** | How native and WASM capabilities communicate with the kernel, serialization format for resource handles and results (resolves §14 open question) | Phase 5 | 6–8 pages |
| D9 | **Security Model** | Authentication, authorization, namespace delegation policy, namespace delegation depth, capability trust chain and authenticity (resolves §6.4, §13.2, and §14 open questions) | Phase 3+ | 10–15 pages |
| D10 | **Ontology Versioning & Evolution** | Semantic versioning policy for ontology layers, backward compatibility rules, ontology combination semantics, ESL extension mechanism (resolves §13.1 and §14 open questions) | Phase 3+ | 8–10 pages |
| D11 | **Execution Context Internals** | Snapshot advancement policy, HLC clock synchronization bounds and violation behavior, capability sub-context isolation boundaries, inline resource semantics in EigenQL (resolves §8.4 and §14 open questions) | Phase 2 | 8–10 pages |
| D12 | **Observability & Operational Tooling** | Structured metrics, tracing spans, query plan explanation, DAG execution step-through, reasoning trace streaming for live monitoring (resolves §13.3) | Phase 4 | 6–8 pages |
| D13 | **Capability Versioning** | How capability implementations are versioned, version mismatch handling, backward compatibility obligations, upgrade path for Foundation capabilities across kernel releases (resolves §14 open question) | Phase 5 | 6–8 pages |

---

## 10. Test Strategy

### 10.1 Test Layers

**Unit tests** (per crate, `cargo test`): cover individual data structures, algorithms, and pure functions. Every module in the kernel has unit tests. Target: >90% line coverage on kernel code.

**Integration tests** (cross-crate): cover the interactions between kernel subsystems — layer commit triggers index construction, capability dispatch invokes the correct evaluator, query evaluation respects layer resolution order. Run with the in-memory storage backend for speed.

**Service tests** (end-to-end): start the kernel gRPC service (with in-memory or SQLite backend), run the CLI against it, verify correct behavior. These are the primary regression tests from Phase 3 onward.

**Contract tests** (storage backends): a single test suite that runs against every storage backend implementation (in-memory, SQLite, TiKV), verifying that they all satisfy the `LayerStore`/`CapabilityStore`/`BlobStore` trait contracts identically.

**Property-based tests** (proptest/quickcheck): for the type system (random well-typed terms type-check, random ill-typed terms are rejected), layer resolution (random layer stacks produce deterministic resolution), and serialization (round-trip property for all Eigon types).

**Performance benchmarks** (criterion): query latency at various resource counts (100, 1K, 10K, 100K), DAG type-checking time vs. DAG size, index construction time per layer. Run on every release; regressions block the release.

### 10.2 CI/CD Pipeline

```
PR opened/updated
  ├── cargo fmt --check
  ├── cargo clippy -- -D warnings
  ├── cargo test --workspace
  ├── deno lint
  ├── deno test
  └── (all pass) → PR mergeable

Merge to main
  ├── All PR checks
  ├── cargo test --workspace --release  (optimized build correctness)
  ├── Contract tests against SQLite
  ├── Build container images
  ├── Push to Azure Container Registry
  └── Deploy to staging ContainerApps environment

Manual promotion
  └── Deploy staging → production ContainerApps environment
```

### 10.3 Formal Verification Track

The Lean 4 formal track (§2.4) runs independently of the implementation CI. It has its own GitHub Actions workflow that checks Lean 4 compilation and proof verification. Property-based tests in the Rust kernel are derived from Lean 4 specifications as they are written — the Lean 4 track produces test oracles that the Rust tests consume.

Verus annotations on kernel-resident algorithms are checked as part of `cargo test` (Verus integrates with the Rust build). These are enabled from Phase 0 for the Eigon structural type checker and layer system.

---

## 11. GitHub Development Workflow

**Branching model:** trunk-based development. `main` is always deployable. Short-lived feature branches (1–3 days), squash-merged via PR.

**Issue tracking:** GitHub Issues with labels per phase (`phase-0`, `phase-1`, ...), per subsystem (`kernel`, `storage`, `orchestration`, `cli`, `deploy`), and per type (`design-doc`, `feature`, `bug`, `test`).

**Milestones:** One GitHub Milestone per phase. Each milestone has a clear "done" definition — the deliverables listed in this plan, verified by the phase's test plan.

**Release cadence:** A GitHub Release at the end of each phase, tagged `v0.{phase}.0`. The release includes container images in ACR and a CLI binary (built via `cargo build --release` in CI).

---

## 12. Azure ContainerApps Deployment

### 12.1 Infrastructure Components

- **Azure Container Registry (ACR):** stores kernel and orchestration container images.
- **ContainerApps Environment:** shared networking, logging (Azure Monitor), and scaling configuration.
- **Kernel Container App:** the Rust gRPC service. Scaling: 1–N replicas based on CPU/memory. Each replica is stateless — all state is in TiKV. Min replicas: 1 (staging), 2 (production).
- **Orchestration Container App:** the Deno service. Scaling: 1–N replicas based on request concurrency. Connects to kernel service via internal DNS.
- **TiKV:** initially a single-node TiKV instance on an Azure VM (dev/staging). Production: 3-node TiKV cluster on Azure VMs or AKS, with PD (Placement Driver) for region management. Design document D4 specifies the exact topology.
- **Azure Key Vault:** LLM API keys, TiKV credentials, service-to-service authentication secrets.
- **Azure Monitor / Log Analytics:** container logs, gRPC request metrics, custom metrics from the kernel (query latency, layer commit rate, capability dispatch count).

### 12.2 Bicep Template Structure

```
deploy/bicep/
├── main.bicep              # Orchestrates all modules
├── modules/
│   ├── acr.bicep           # Container Registry
│   ├── environment.bicep   # ContainerApps Environment + Log Analytics
│   ├── kernel.bicep        # Kernel service Container App
│   ├── orchestration.bicep # Orchestration service Container App
│   ├── keyvault.bicep      # Key Vault + secrets
│   └── tikv.bicep          # TiKV VM(s) or AKS config
└── parameters/
    ├── staging.bicepparam
    └── production.bicepparam
```

### 12.3 Deployment Environments

**Staging:** deployed on every merge to `main`. Single-replica kernel, single-replica orchestration, single-node TiKV. Used for integration testing and demo.

**Production:** manually promoted from staging after verification. Multi-replica kernel and orchestration, 3-node TiKV. Used for real workloads.

---

## 13. CLI Design

The CLI (`eigenius`) is the primary developer interface for interacting with the platform.

```
eigenius [--endpoint <url>] [--local] <command>

Commands:
  load <file>              Load an Eigon resource file into the working context
  query <eigenql>          Execute an EigenQL query
  validate <dag-file>      Type-check a DAG specification
  run <dag-file> [inputs]  Execute a validated DAG
  reflect <trace-file>     Record a reasoning trace
  inspect <iri>            Print a resource by IRI
  layer list               List layers in the current stack
  layer commit             Commit the working layer
  capability list          List registered capabilities
  capability test <id>     Test-invoke a capability
  config                   Manage CLI configuration (endpoint, credentials)
  version                  Print version and build info
```

**Modes:** `--endpoint <url>` connects to a remote kernel service (gRPC). `--local` runs an embedded kernel with SQLite storage. Default: `--local` if no endpoint is configured.

**Output formats:** human-readable (default), `--json` for machine consumption, `--table` for tabular query results.

---

## 14. Dependency Summary

### Rust (kernel + CLI + storage)

| Crate | Purpose |
|-------|---------|
| `tonic` + `prost` | gRPC server and protobuf codegen |
| `tikv-client` | TiKV Rust client |
| `rusqlite` | SQLite backend |
| `wasmtime` | WASM capability sandbox |
| `clap` | CLI argument parsing |
| `serde` + `serde_json` | Eigon JSON serialization |
| `proptest` | Property-based testing |
| `criterion` | Benchmarking |
| `tracing` | Structured logging |
| `verus` | Proof annotations (where applicable) |

### TypeScript/Deno (orchestration)

| Package | Purpose |
|---------|---------|
| `@grpc/grpc-js` or Deno gRPC | Kernel service client |
| `ai` (Vercel AI SDK) | LLM provider abstraction |
| `@modelcontextprotocol/sdk` | MCP server |

---

## 15. Risk Register

| Risk | Impact | Mitigation |
|------|--------|------------|
| TiKV Rust client maturity gaps | Blocks Phase 3 storage integration | Spike TiKV client early (before Phase 3). Fallback: use TiKV's gRPC API directly. |
| Mini-TT → Rust port complexity | Delays Phase 2 type system | Port incrementally, test against Haskell reference. Start with a minimal subset (Pi, Sigma, sums). |
| Deno FFI to Rust kernel friction | Complicates Phase 2 orchestration↔kernel communication | Use gRPC even for local development (Phase 2 can start the kernel as a subprocess). |
| TiKV on Azure operational complexity | Delays Phase 3 deployment | Start with SQLite in ContainerApps for staging. Defer TiKV to production deployment. |
| WASM capability interface design | Blocks Phase 5 extensibility | Write design doc D7 early (during Phase 3 or 4) to derisk. |
| Solo developer burnout on 6-phase plan | Stalls project | Each phase is independently valuable. The project is useful after Phase 1 (queryable knowledge graph). |

---

## 16. Beyond Phase 5 — Future Horizons

The following capabilities are described in the architecture but are deliberately excluded from the initial six-phase plan. They become relevant once the platform is stable and has real domain ontology usage.

**EigenQL recursive Datalog extension (§5.6).** The v1 conjunctive query language is sufficient for the initial platform. Extending to recursive rules requires stratification checking, termination proofs, and query planner changes. This is a major effort with its own design document and Lean 4 proof obligations.

**Constructive type theories as capabilities (§9.7).** Registering Lean 4, Coq/Rocq, or Agda proof kernels as capabilities — enabling the system to dispatch proof obligations to external theorem provers. This requires the WASM sandbox (Phase 5) plus a well-defined proof term interchange format.

**Browser and edge deployment (§2.6).** Compiling the kernel to WASM for browser-based developer tooling (ontology browsers, DAG editors) and edge deployment (Deno Deploy, Cloudflare Workers). Requires adapting the storage interface to IndexedDB/Deno KV and replacing gRPC with a browser-compatible transport.

**Distributed TiKV multi-region deployment.** The initial Azure deployment uses a single-region TiKV cluster. Cross-region replication, geo-aware layer placement, and consistency under partition require significant operational engineering.

**Ontology marketplace / registry.** A mechanism for publishing, discovering, and installing domain ontologies — analogous to a package registry. Requires the ontology combination semantics (§14 open question) and a trust/authenticity chain (§6.4).

---

*This plan is a living document. Phase boundaries may shift as design documents are written and early phases reveal unexpected complexity. The key invariant is that each phase produces a working, testable system.*
