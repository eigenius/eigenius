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
│   ├── rocksdb/               # RocksDB embedded storage backend
│   ├── memory/                # In-memory backend (testing)
│   └── indexing/              # SPO/POS/OPS triple index construction (§10.8)
├── orchestration/             # Deno/TypeScript orchestration layer (§2.2)
│   ├── src/
│   │   ├── program/            # Program execution engine (§12)
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

The build is organized into phases. Each phase produces a working system that can be tested end-to-end. Later phases extend earlier ones; nothing is thrown away.

| Phase | Name | Status | What it proves |
|-------|------|--------|----------------|
| 0 | Foundation | ✓ | Eigon types exist, layers work, storage round-trips |
| 1 | Query | ✓ | EigenQL v1 evaluates against a layer stack |
| 2 | Programs | ✓ | Programs type-check and execute via Mini-TT/NbE |
| 3 | Service | ✓ | Kernel runs as gRPC service with RocksDB |
| 4 | Intelligence | ✓ | LLM integration, orchestrator, MCP, reasoning traces |
| 4.5 | ESL | ✓ | Human-friendly surface syntax compiles to Eigon-JSON |
| 5 | Traces + NbE | ✓ | Trace persistence, type theory extensions, NbE with capability modes |
| 6 | Institutions | ✓ | Grothendieck institution protocol, fiber reasoners, morphism validation |
| 7 | CompleteJson | ✓ | Structured LLM output via JSON Schema from ontology classes |
| 8 | WASM | ✓ | Untrusted capabilities run sandboxed via Wasmtime |
| 9a | Durable State | ✓ | Layers, traces, WASM capabilities survive kernel restart |
| 9b | Codata + Streams | ✓ | Resumable execution, coinductive streams, concurrent tasks |
| 10 | Kernel Completeness | ✓ | Ontology-as-types resolution, universe soundness, typed errors |
| 11 | Type Theory Extensions | 11a ✓, 11b ✓, 11c ✓, 11d ✓, 11e.1 ✓, 11e.2 ✓ | Map/Reduce, inductive types, decision procedures, Comorphism class, ESL + EigenQL institution-capability surface |
| D22 | Notebook & TypeScript SDK | ✓ | React notebook SPA + `@eigenius/client` SDK, served by the orchestrator at `/notebooks/`; six cell types incl. form-based charts; content-addressed publish-to-layer |
| 12 | D14 Institutions | M1–M8 ✓; WASM-pkg / EigenQL-surface / docs / proto-cleanup pending | D14 institution realisation replaces D10; dock→assay worked example; comorphisms via four-step pipeline; Verdict-shaped Decidable + AutoOnLoad dispatch |
| 13 | Azure + Ops | | Production deployment, CI/CD, observability, TiKV option |
| 14 | Reconciliation | | Multi-session, layer merging via comorphism witnesses |
| 15 | Specialty Institutions | | Lean 4, SMT solvers, domain-specific proof institutions |

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

## 5. Phase 2 — Pipelines ✓

**Goal:** programs type-check against the Mini-TT dependent type system and execute through the orchestration layer. Partial evaluation works.

**Duration estimate:** 6–8 weeks. **Completed:** April 12, 2026 (1 day, with Claude Code). Programs represented as typed expressions (not workflow graphs), Mini-TT core ported from Haskell reference, program ontology with 54 resources, end-to-end pipeline: parse → type-check → execute.

### 5.1 Deliverables

- Mini-TT implementation in Rust: Pi types, Sigma types, labeled sums, closures, environments, the `Val`/`Neut` value representation (§4.2). Ported from the Haskell reference implementation.
- NbE evaluator: `eval`, `readback`, `check`/`checkI` (bidirectional type checking), `eqNf` (equality by normalization) (§4.6).
- program type system: Components, Pipe, Parallel, Select, Map, Retry as typed constructs (§4.3–4.4). Type signatures with `Result<A, E>` for fallibility (§4.5). `NonDeterministic` marker.
- program validator: takes a program specification and a typing context (derived from the layer stack's class definitions), runs the NbE type checker, reports type errors with source locations.
- program validator registered as a Foundation Layer capability alongside EigenQL.
- Partial evaluation: given a program and a subset of its inputs, produce a typed residual program (§4.9). Neutral terms (`Nt Neut`) represent unknown inputs.
- Ground type resolution interface: the bridge between Eigon ontology types and Mini-TT ground types (§4.10). Class references in program type signatures resolve against the layer stack.
- Orchestration layer (Deno/TypeScript): program execution engine that walks the validated program, executes Components (initially stub implementations), handles Pipe sequencing, Parallel fan-out, Select branching, Map iteration, Retry logic (§12.1–12.2).
- CLI `validate` command: takes a program specification file, validates it, prints type checking results.
- CLI `run` command: takes a validated program and input resources, executes it through the orchestration layer, prints output resources.

### 5.2 Key decisions required before coding

- program specification format: how are programs authored? ESL surface syntax? A YAML/JSON DSL? This needs a design document (see §8.2 of this plan).
- Component interface: what does a Component implementation look like from the orchestration layer's perspective? Input/output types, error handling, timeout contract.
- Orchestration ↔ kernel communication for Phase 2: before the gRPC service exists (Phase 3), the orchestration layer can embed the kernel as a Rust library via FFI or WASM. Recommendation: use the in-memory storage backend and call kernel functions directly via Deno FFI (`Deno.dlopen`) for Phase 2, then switch to gRPC in Phase 3.

### 5.3 Test plan

- **Mini-TT core:** Port the Mini-TT test suite from the Haskell implementation. Verify that well-typed terms check, ill-typed terms are rejected, and NbE produces expected normal forms.
- **program type checking:** A well-typed pipeline (Component → Pipe → Component) type-checks. A pipeline with a type mismatch (output type of step 1 ≠ input type of step 2) is rejected with a clear error.
- **Partial evaluation:** Provide 2 of 3 inputs to a program, verify the residual is a valid program with one remaining input. Execute the residual with the final input, verify the result matches full execution.
- **Select exhaustiveness:** A Select without a default branch is rejected. A Select with a default branch type-checks.
- **program execution:** A 3-step pipeline with stub Components executes end-to-end, producing typed output resources.
- **Error propagation:** A Component that returns `Err` triggers the Retry logic (if configured) or propagates the error.
- **Parallel execution:** A Parallel construct with 3 branches executes concurrently (verify via timing — parallel should be faster than sequential).

---

## 6. Phase 3 — Service ✓

**Goal:** The kernel runs as a standalone gRPC service with persistent storage. The CLI talks to the service over the network.

**Duration estimate:** 4–6 weeks. **Completed:** April 12, 2026 (1 day, with Claude Code). CBOR serialization, RocksDB storage, gRPC server (tonic), CLI dual mode (local/remote), DB admin commands, server integration tests.

### 6.1 Deliverables

- gRPC API definition (protobuf): `Load`, `Query`, `Validate`, `Reflect`, `Inspect`, `RunProgram` RPCs. Defined in `proto/eigenius.proto`.
- Kernel gRPC server: Rust binary using `tonic` that starts the kernel, runs the bootstrap sequence, opens RocksDB storage, and serves RPCs.
- RocksDB storage backend: implements `LayerStore` and `ResourceStore` traits. Ordered key-value model with prefix scans for efficient layer/resource retrieval. Same key encoding translates directly to TiKV when multi-node is needed.
- CLI refactored to dual mode: `eigenius --local` (embedded kernel + RocksDB) and `eigenius --endpoint <url>` (gRPC client to remote kernel).
- Orchestration layer gRPC client: interface defined (Connect RPC for Deno).
- Dockerfile for kernel service: multi-stage Rust build, minimal runtime image.
- Dockerfile for orchestration layer: Deno-based image.
- DB admin commands: `db stats`, `db compact`, `db export`.
- CBOR serialization (`eigon_cbor.rs`): compact binary format for storage and wire, deterministic encoding for content-addressed hashing.
- Server integration tests: gRPC round-trip for Load, Inspect, Query, Health.

Note: Azure deployment (Bicep templates, CI/CD) deferred to Phase 4, when the orchestration layer has real functionality to deploy.

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
- Authentication/authorization model for the service API: API keys, mTLS, Azure AD tokens? Deferred to Phase 4 alongside Azure deployment. Not the ontology-level security model (§13.2).

### 6.4 Test plan

- **Integration test:** Start kernel service with in-memory backend, send Load/Query/Validate RPCs via CLI, verify correct responses.
- **TiKV round-trip:** Start kernel with TiKV backend, load ontology, commit layers, restart kernel, verify data persists and queries return correct results.
- **Index correctness:** Load 1,000 resources, verify SPO/POS/OPS indexes in TiKV produce the same query results as the in-memory backend.
- **Container build:** Docker build succeeds, container starts, health check passes.
- **Azure deployment smoke test:** Deploy to ContainerApps staging environment, run the CLI integration test suite against the deployed endpoint.
- **Concurrent access:** Two CLI sessions loading resources and querying concurrently against the same kernel service — no corruption, snapshot isolation holds.

---

## 7. Phase 4 — Intelligence ✓

**Goal:** LLMs can be invoked from programs and can invoke Eigenius as a tool. Reasoning traces are recorded and queryable.

**Duration estimate:** 4–6 weeks.

**Status:** Core implementation complete. Reflection ontology, trace recording, orchestrator with Connect RPC, CompleteText LLM adapter (Vercel AI SDK / Anthropic), MCP server, epistemic base class validation, remote component dispatch, Docker Compose integration — all operational. End-to-end demo working (CLI → kernel gRPC → orchestrator → LLM → typed output). See `docs/design/phase4-implementation-plan.md` for step-by-step details.

**Deferred to follow-up:** Reflect RPC (trace persistence), universe stratification enforcement, Azure deployment, CompleteJson component.

### 7.1 Deliverables

- LLM adapter Components in the orchestration layer: Anthropic Claude, OpenAI, configurable via provider resource in the ontology (§2.3). Using Vercel AI SDK or direct provider SDKs.
- Orchestration layer gRPC client: Connect RPC connecting Deno to kernel service.
- MCP server in the orchestration layer: exposes Load, Query, Validate, Reflect as MCP tools (§2.3). An LLM agent can query the knowledge graph, validate pipelines, and record reasoning traces via tool-use.
- Reflection layer in the kernel: `ReasoningTrace` as a typed Eigon resource class (§11.2). Traces capture LLM invocations (prompt, completion, token usage, latency), program step results, and provenance links.
- Universe stratification enforcement in the reflection layer (§11.3): reasoning traces about resources at level N are recorded at level N+1.
- CLI `reflect` command: record a reasoning trace manually, query reasoning traces with EigenQL.
- Azure deployment: Bicep templates for ContainerApps, GitHub Actions CI/CD (build containers, push to ACR, deploy to staging).
- End-to-end demo: a program that takes a document resource, invokes an LLM to summarize it, records the reasoning trace, and stores the summary as a typed resource — all queryable after execution.

### 7.2 Key decisions required before coding

- MCP transport: stdio (for local LLM integration) vs. SSE/HTTP (for remote agents). Recommendation: implement both; MCP SDKs support multiple transports.
- Reasoning trace schema: the specific classes and properties for traces. Design document needed (see §8.5).
- Token/cost tracking: whether token usage and cost estimates are first-class properties on reasoning trace resources.

### 7.3 Test plan

- **LLM adapter:** Mock LLM provider, execute a program with an LLM step, verify typed output resource matches the mock response.
- **MCP round-trip:** Start MCP server, connect an MCP client, invoke Query tool, verify correct typed response.
- **Reasoning trace persistence:** Execute a program, verify reasoning traces are committed as part of the layer, queryable via EigenQL.
- **Universe stratification:** A reasoning trace about a level-0 resource is at level 1. Attempt to create a level-0 trace about a level-0 resource — verify rejection.
- **Provenance query:** "Which LLM calls contributed to resource X?" — answered by querying reasoning traces.

---

## 7.5. Phase 4.5 — ESL (Eigenius Schema Language) ✓

**Goal:** A human-friendly surface syntax for authoring programs, ontologies, and queries. ESL compiles to Eigon-JSON.

**Duration estimate:** 2–3 weeks.

**Status:** Complete. Two-layer design (HCL-style structural + ML-style expressions), hand-written recursive descent lexer/parser/compiler, all CLI commands accept `.esl` files, kernel gRPC accepts `application/esl` content type. Component argument blocks with `f(arg) { config }` syntax. See `docs/design/d7-esl-surface-syntax.md`.

---

## 8. Phase 5 — Traces and Incremental Execution ✓

**Goal:** Reasoning traces are persisted as resources in the knowledge graph and drive incremental execution. The executor unifies with NbE so that trace-driven evaluation is normalization — existing traces short-circuit, only untraced subexpressions dispatch.

**Duration estimate:** 4–6 weeks.

**Status:** Complete. NbE evaluator with capability modes (Pure/Read/IO) replaces the old executor. Type theory extended with Id types, decidable equality, native constraint checking, and universe stratification. Ground type resolution maps full ontology (requires, recommends, allows_only, class_types). ComponentTraces committed to trace layers as proof artifacts. Incremental execution works within a server session. Known gap: persistent trace store (RocksTraceStore) not wired into the server — deferred to Phase 9a. See `docs/design/d9-nbe-unification-and-type-extensions.md`.

### 8.1 Deliverables

- Reflect RPC implementation: accept serialized traces, store as resources in a layer, return trace IRI. ProgramTrace wraps the complete trace tree with metadata (tokens, latency, epistemic status).
- Trace persistence in RocksDB: ComponentTraces stored with content-addressed keys for memoization across executions.
- RunProgram returns trace IRI: after execution, the kernel automatically creates a ProgramTrace and returns its IRI alongside the output.
- Incremental execution: re-evaluating a program checks the trace store first. Traced IO components return instantly. Only untraced subexpressions dispatch to the orchestrator.
- NbE/executor unification: the program executor uses NbE's eval with a trace-aware environment. `Val::Resource` extends the value domain. Ground type resolution and execution share the same evaluator.
- Universe stratification enforcement: traces at level N can only reference resources at level N-1 or below.
- CLI `reflect` command: record a reasoning trace manually, query traces via EigenQL.
- Provenance queries: "Which LLM calls contributed to this output?" answered by walking the trace tree via EigenQL.

### 8.2 Key decisions required

- Whether NbE unification replaces the current executor entirely or wraps it
- Trace garbage collection policy (keep all? TTL? layer-scoped?)
- Whether RunProgram auto-commits the trace layer or returns it uncommitted

### 8.3 Test plan

- Execute a program, verify ProgramTrace is stored and queryable
- Re-execute the same program with same input — verify cached (zero LLM calls)
- Change the input — verify partial re-execution (only changed subexpressions dispatch)
- Crash recovery: kill mid-execution, restart, verify resume from last ComponentTrace
- Universe stratification: attempt to create a level-0 trace about a level-0 resource — verify rejection
- Provenance query end-to-end

### 8.4 References

- eigenius/eigenius#5 — Implement Reflect RPC
- `docs/design/d6b-reasoning-trace-schema.md` — trace classes and epistemic model

---

## 9. Phase 6 — Grothendieck Institutions ✓

**Goal:** Domain-specific reasoning systems (institutions) contribute structured fibers to the knowledge graph. Each institution provides its own sentences, models, satisfaction relation, and internal morphisms — not just flat data points.

**Duration estimate:** 6–8 weeks.

**Status:** Core protocol complete; **superseded by [D14](d14-institution-realisation.md) in Phase 12.** The original D10 surface (`FiberReasoner` trait, `InstitutionRegistry`, `ComorphismRegistry`, in-validator morphism dispatch, `FiberQuery` / `DiscoverMorphisms` / `ListInstitutions` RPCs, the ordering test institution) has been retired. Phase 12 replaced it with the D14 ontology-first realisation (`InstitutionIndex` over chain-declared institutions / formats / queries / comorphisms; `InstitutionRuntime` of `Institution` trait impls; Verdict-shaped Decidable + AutoOnLoad QueryClasses; four-step comorphism pipeline). The deliverables list below is preserved as historical record; the actual current shape lives in the Phase 12 section.

**Deferred:** Fully worked domain examples (mechanical engineering, biopharma) require WASM sandboxing for domain-specific institutions — deferred to Phase 12, then refocused under D14 onto a single dock→assay worked example with optional second domains as follow-on (see Phase 12).

### 9.1 Deliverables

- `FiberReasoner` trait: the kernel interface for institution-specific reasoning. Methods: `query`, `validate_morphism`, `discover_morphisms`, `fiber_declaration`.
- Fiber declaration protocol: institutions advertise their morphism types, query types, and structural properties as ordinary ontology resources at registration time.
- Morphism types as ontology classes: `FiberMorphism` base class with `source`, `target`, `institution_ref` properties. Domain institutions subclass this.
- Institution comorphisms: `Comorphism` trait with `translate_forward`/`translate_backward`, `ComorphismRegistry`.
- Cross-institution queries: EigenQL navigates fiber morphisms using existing query syntax (morphisms are resources).
- Fiber reasoner dispatch: the kernel dispatches fiber queries through NbE in IO mode and morphism validation through the validator.
- Institution ontology: `ontologies/institution/institution-ontology.json` loaded at bootstrap.
- gRPC: `FiberQuery`, `DiscoverMorphisms`, `ListInstitutions` RPCs.
- CLI: `list-institutions` command.

### 9.2 Resolved decisions

- Fiber reasoners are in-process Rust trait objects (Phase 6); gRPC for external services and WASM in Phase 8
- Structural properties are advisory, not enforced by the kernel
- Institution registration at server startup, ontology resources committed to bootstrap layer chain

### 9.3 References

- `docs/design/d10-grothendieck-institution-protocol.md` — full specification
- `docs/papers/eigenius-institutions.tex` — theoretical foundation

---

## 10. Phase 7 — CompleteJson (Structured LLM Output) ✓

**Goal:** The CompleteJson component calls an LLM with a JSON Schema derived from an ontology class, receives structured JSON, and converts it back to a typed Eigon resource with full type-level guarantees.

**Duration estimate:** 3–4 weeks.

**Status:** Complete. Schema generation from ontology classes with enums (`allows_only`), nested objects (`class_types`), and union types (multiple `class_types` with `_type` discriminator). Bijectivity check integrated into `ValidateProgram` and CLI `program-validate`. `GetSchema` RPC and CLI `get-schema` command. `complete_json.ts` orchestrator handler using Vercel AI SDK `generateObject()`. Template data type for prompt validation. Ontology-driven component dispatch via `argument_type`. Patent analysis demo (CompleteJson → CompleteText → Construct pipeline). 14 schema-specific tests covering enums, nested objects, unions, constraints, duplicate short name rejection, and round-trips. See `docs/design/d8-complete-json-component.md`.

### 10.1 Deliverables

- `schema_for_class` in the kernel: generate JSON Schema + `ShortNameTable` from a class definition. Walk class hierarchy, map data types, constraints, `allows_only` → enums, `class_types` → nested objects / `oneOf` unions.
- `convert_json_to_resource`: convert LLM JSON response back to typed Eigon resource using the `ShortNameTable`. Bijective mapping guaranteed by construction.
- Type-level bijectivity check: `validate_output_schemas` walks the program expression tree and verifies that each `output_schema` class admits a bijective short-name mapping. Integrated into `ValidateProgram` RPC and CLI `program-validate`.
- `GetSchema` RPC: expose schema generation for tooling and debugging. CLI `get-schema` command.
- `complete_json.ts` orchestrator handler: receives JSON Schema, calls `generateObject()`, returns raw JSON.
- End-to-end test with enums, nested objects, and union types (`ontologies/examples/schema-test.json`).

### 10.2 Resolved decisions

- Recursion depth limit for nested class schemas: 4 levels
- `_type` discriminator field name is fixed (not configurable)

### 10.3 References

- eigenius/eigenius#6 — Implement CompleteJson
- `docs/design/d8-complete-json-component.md` — full specification

---

## 11. Phase 8 — WASM Extensibility

**Goal:** Third-party capability code runs in WASM sandboxes. Domain ontologies can register custom validators and evaluators safely. Institution fiber reasoners (Phase 6) can be implemented as WASM modules.

**Duration estimate:** 4–6 weeks.

### 11.1 Deliverables

- Wasmtime integration in the kernel: instantiate a WASM module, provide the capability import interface, enforce memory and fuel limits (§9.6).
- Capability SDK: a Rust crate that capability authors compile to WASM. Provides typed bindings for reading resources from the execution context, emitting results, and declaring required external access.
- Capability registration via ontology: a domain layer can register a WASM module as a capability for a custom class.
- Domain ontology loading: load a third-party ontology layer that defines custom classes, properties, and WASM-sandboxed capabilities. Verify that it cannot shadow Foundation Layer capabilities (§9.5).
- WASM institutions: domain institution implementations as WASM modules, using the WASM Component Model. (Originally framed as `FiberReasoner` impls; re-targeted to D14's `Institution` trait + `eigenius-institution-d14` WIT world in Phase 12. The `wasm_institution_d14.rs` host bridge was added in Phase 12 M4.)
- CLI `capability` subcommand: list registered capabilities, inspect a capability's type signature, test-invoke a capability.
- Example domain ontology: a "Legal Document" ontology with a custom validator that checks document structure — delivered as a worked example and integration test.

### 11.2 Test plan

- **Sandbox isolation:** A WASM capability that attempts to access memory outside its linear memory — verify trap, no kernel corruption.
- **Fuel exhaustion:** A WASM capability with an infinite loop — verify termination within the fuel limit, error returned.
- **Interface control:** A WASM capability that attempts to make a network call — verify rejection (no network import provided).
- **Foundation protection:** Attempt to register a capability under `urn:eigenius:foundation:` from a domain layer — verify rejection.
- **End-to-end:** Load domain ontology with WASM capability, create a resource of the domain class, dispatch to the WASM capability, verify correct result.
- **WASM fiber reasoner:** Load a domain institution as WASM, query its morphisms, verify correct fiber reasoning through the sandbox.

---

## Phase 9 — Durable Kernel State, Persistent Traces, Resumable Execution, and Codata Streams

**Goal:** Make the running kernel durable end-to-end — layers, resources, traces, and WASM capabilities (including institutions) all survive restarts — and then build codata-based streaming and resumable execution on top of that foundation.

**Duration estimate:** 5–7 weeks total, split into two internal milestones.

**Prerequisites:** Phase 5 (traces + NbE), Phase 8 (WASM). Institution wiring originally framed as `validate_with_institutions` in the Load path was rebuilt under D14 in Phase 12 as AutoOnLoad QueryClass dispatch + the `commit_with_validation` head-promote-then-revert-on-failure semantics; institution registration into `start_server` lives as the per-commit `rebuild_institution_index` hook plus boot-time index/runtime construction. 9a's WASM-component re-registration handles the kernel-side plumbing that survives restart.

The phase decomposes into two milestones that are separately reviewable:

### Phase 9a — Durable kernel state (D13) — ~1 week

**Status:** Complete (April 2026). `eigenius serve --db <path>` persists every committed layer, resource, and trace; restart rebuilds running state from the DB; embedded ontologies are seeded once with a SHA-256 manifest and drift refuses to boot. See D13.

**Goal:** `eigenius serve --db <path>` persists every committed layer, resource, and trace. Restart rebuilds the running state from the DB; the embedded core ontology is seeded once and never re-overwritten silently.

- `--db <path>` flag (and `EIGENIUS_DB` env override) on `serve`. Open `RocksStore` at that path; in-memory fallback preserved for tests.
- First-run SEED: commit the four embedded ontology layers (core → program → reflection → institution) to the store. Record a seed manifest with SHA-256 of each embedded JSON.
- Subsequent RESUME: walk the persisted layer chain from the stored head; compare the manifest against embedded SHA-256s and refuse to boot on drift (no silent auto-upgrade).
- `ExecutionContext::commit` writes through to the store atomically before rotating the in-memory head; `Load` RPC becomes durable automatically.
- WASM + institution re-registration on RESUME: scan every persisted layer, re-compile + re-register components and institutions into their respective runtime registries. Closes [#15](https://github.com/eigenius/eigenius/issues/15) along the way — institution-declared classes now commit to the layer, not just the registry.
- Trace store backed by the persistent backend via `BackendTraceStore` adapter (shares the RocksDB handle; `meta:<key>` prefix for non-layer metadata).
- Integration coverage: in-process Rust test (`storage/rocksdb/tests/durability_test.rs`) plus CLI-surface smoke script (`examples/wasm-ordering-institution/run_durable.sh`), both installing an institution, restarting against the same DB, and verifying dispatch still works without re-install.
- See D13 for the full specification and the five-step implementation ordering.

### Phase 9b — Codata, streams, and resumable execution (D11, D21) — ~4–6 weeks

**Status:** Complete (April 2026). Shipped in four sub-milestones — 9b-i (Mini-TT codata + guardedness), 9b-ii (ESL codata/corecord/observation + ontology), 9b-iii (task model: storage primitives, evaluator re-keying, RPCs, CLI, resume sweep), plus issue #16 filed for sized-types follow-up.

**Goal:** Extend the execution model with codata (coinductive types) so that data and event streams become first-class. Programs run as tracked tasks that persist across kernel restarts, with per-task positional trace keys for correct streaming and a startup resume sweep for crash recovery.

- Mini-TT gains `Codata` / `CoRecord` / `Observe` terms, matching `Val` variants, eval + readback + type checker arms, and a syntactic guardedness check (Agda-style). See D11.
- ESL surface syntax: top-level `codata Name { obs : T; ... }` declarations, `corecord { obs = e; ... }` expressions, bare-name `.obs` observations unified with property access via `Exp::PropAccess` dispatch. Ontology gains `CodataType`, `Observation`, `CoRecord`, `CoField` classes with their supporting properties.
- Task model (D21): `TaskRecord`, `Checkpoint`, `TaskContext`, `TaskStore` + `BackendTaskStore` adapter. Per-task positional trace keys `(session_id, task_id, step_seq)` replace Phase 9a's content-address cache for IO components (determinism-gated — Pure/Read keep the memo for cross-task reuse). `components:Checkpoint` built-in persists program-declared state snapshots atomically via `write_batch`.
- gRPC surface: `RunProgram` returns a `task_id`; new `ListTasks` / `GetTaskStatus` / `CancelTask` RPCs; `Health` reports `resume_in_progress` / `tasks_resuming`; `Inspect` / `Query` / `GetSchema` gain an optional `at_layer` LayerId (D21 §3.6) for reaching forked task result layers.
- CLI: `eigenius tasks list|status|cancel`, plus `--at-layer` on `inspect` and `query`.
- Startup resume sweep (D21 §6): bounded-parallel background task that rehydrates pinned layer chains and re-executes `Running`/`Suspended` tasks, with `max_parallel_resumes=4` / `max_resume_attempts=1` defaults.

**Deferred to follow-ups:** sized types for productivity checking (#16), async `RunProgram` spawn (breaks existing synchronous clients; additive), cooperative cancellation check in the evaluator proper, `cancel_grace_seconds` force-abort + shadow-keyspace spill, retention pruning on checkpoint commit (D21 §5).

### Phase 9 — Key design questions (D11 scope)

- How do codata types interact with the NbE evaluator? Coinductive types require productive corecursion — the evaluator must guarantee progress (each observation step produces a value before the next observation)
- How do streams compose with the layer system? Each stream observation could produce resources committed to a new layer, extending the chain incrementally
- How does the task scheduler interact with the gRPC server? Is it a separate service or part of the kernel?
- What is the relationship between codata streams and institution fiber morphisms? (A stream of refinements could be a morphism chain in the FEA institution)

### Phase 9 — References

- D9: `docs/design/d9-nbe-unification-and-type-extensions.md` §5.10 (known gap: persistent trace store)
- **D13: `docs/design/d13-durable-kernel-state.md`** — durable-state specification, startup sequence, institution/WASM re-registration, migration policy
- D11: `docs/design/d11-codata-streams.md` — codata, streams, resumable execution
- **D21: `docs/design/d21-task-traces-and-checkpointing.md`** — per-task trace identity, checkpoint primitive, retention policy. Prerequisite for 9b-iii.

---

## Phase 10 — Kernel Completeness ✓

**Goal:** Close the fundamental kernel correctness gaps that block platform breadth — particularly ontology-as-types resolution, which is a prerequisite for the life-science representation work in Phase 11 and every other domain that typechecks against ontology classes. Framed in `docs/design/life-science-requirements.md` §19 step 1 as "nothing works cleanly until `find_sigma_field` resolves EigonClass to proper dependent records."

**Duration estimate:** 3–5 weeks total, three internal milestones.

**Status:** Complete (April 2026). Ontology-as-types resolution (#18, D18), defence-in-depth for IO-reachable panics, and typed `EvalError` refactor (#19) all shipped. The NbE evaluator returns `Result<Val, EvalError>` throughout — 20 panic sites converted to error returns, all tests use `?` propagation. `catch_unwind` retained as defence-in-depth but the primary error path is now the Result monad.

**Prerequisites:** Phase 5 (Mini-TT/NbE), Phase 8 (WASM).

**Drives:** eigenius/eigenius#12 (high-priority correctness hazards), #13 (medium), #14 (low).

### Phase 10a — Ontology-as-types resolution ✓ — ~2 weeks

**Status:** Complete (April 2026). `find_sigma_field` walks the layer chain to resolve `EigonClass(iri)` into dependent record types. `check_infer` extended with inference-mode rules for `Construct`, `EigonResource`, `Template`, `IdJ`, `Refl`, `NativeDecide`, `DecEq`. `CheckCtx` bundles `rho`/`gamma`/optional layer/per-check type cache. See D18 and #18.

**Goal:** `find_sigma_field` walks the layer chain to resolve `EigonClass(iri)` into a proper dependent record type (Sigma chain) instead of silently collapsing to `Val::Set`. Property access on ontology classes type-checks against the class's declared properties and their datatypes.

- Hook `find_sigma_field` into the Read-capability-mode layer access (D9) so it can resolve IRI → Class resource → iterated Sigma field chain at check time.
- Extend `check_infer` with cases for `Construct`, `EigonResource`, `Template`, `IdJ`, `Refl`, `NativeDecide`, `DecEq` — currently these fall through to an "unable to infer" error whenever they appear in inference position without an annotation.
- Integration tests: property access on ontology-typed resources; Construct expressions without annotations; round-trip through patent demo pipeline.
- Drives the "consolidated ontology-as-types layer-chain plumbing" prerequisite named in life-science §19.
- See D18.

### Phase 10b — Universe stratification and meta-level soundness — ~1–2 weeks

**Status:** Deferred — enforcement at the layer-ingestion level is not yet wired. The Mini-TT checker's universe rules are adequate for current use. Will revisit when Phase 12 worked examples exercise multi-level epistemic claims.

**Goal:** Enforce the three-level epistemic stratification (data → derivation → meta) at ingestion time so meta-level claims over traces are sound. Life-science §16.2 names this as unblocking §13 "Meta-level claims (sound)."

- Enforcement point: resource ingestion in the layer system (not Mini-TT term forms). A resource at epistemic level N that references a resource at level ≥ N is rejected with a clear error.
- Mini-TT checker: tighten universe-rule handling so attempts to construct self-referential meta-claims fail at check time (before runtime).
- Integration with D6b trace schema: a ProgramTrace at level 2 references only resources at level ≤ 1.
- nanoda_lib (see `lean-4-as-institution.md`) as reference for how universe checking integrates with type equality — Eigenius's needs are simpler (three fixed levels vs. Lean's universe polymorphism).

### Phase 10c — Robustness: typed errors, lossy conversions, allocator hygiene ✓ — ~1 week

**Status:** Complete (April 2026). Defence-in-depth for IO-reachable panics (Phase 10c-i: graceful fallbacks + `catch_unwind`) shipped first, then the full `EvalError` refactor (#19): `eval`/`eval_ctx`/`eval_traced` and all `Val`/`Clos` methods return `Result<Val, EvalError>`. 8-variant `EvalError` enum covers all 20 former panic sites. `check.rs` maps via `.map_err(|e| e.to_string())?`. `readback.rs` stays infallible with `.expect()` (type-checker-validated paths). All test functions use `Result` + `?` — no `.unwrap()` on eval results.

**Goal:** Close the residual low-priority hazards so kernel behaviour under misbehaving institutions is predictable.

- `eval`'s panics in Pure mode → typed `EvalError` returns. Misbehaving institutions can't crash the kernel.
- `val_to_resource`'s lossy non-ResourceVal collapse → debug-assertion + a clearer error path. Same invariant, louder failure.
- `Arrow` / `Times` → `Pi(Patt::Unit, …)` / `Sig(Patt::Unit, …)` cloning double-allocation → constructor-level optimisation. Measured improvement, not a correctness fix, but keeps the hot path clean.

### Phase 10 — Test plan

- Ontology-typed property access round-trips through all existing WASM examples without `Val::Set` leakage in the trace.
- A contrived program that tries to reference a level-2 resource from a level-1 resource is rejected at ingestion with a clear stratification error.
- The three correctness-hazard issues each get a regression test before closing.

### Phase 10 — References

- `docs/design/life-science-requirements.md` §19 (recommended sequencing), §16.2 (stratification), §16.3 (decision procedures — deferred to Phase 11)
- `docs/design/d9-nbe-unification-and-type-extensions.md` — Mini-TT surface this extends
- `docs/design/lean-4-as-institution.md` — nanoda_lib design references
- D18 (to be written) — Ontology-as-Types Resolution

---

## Phase 11 — Type Theory Extensions

**Goal:** The kernel type theory becomes expressive enough to represent the shapes life-science (and other domain) users need. Inductive types for fiber morphisms, `Map` and `Reduce` as language primitives, institution-registered decision procedures, and `Comorphism` as a first-class ontology class.

**Duration estimate:** 8–11 weeks total, four internal milestones.

**Prerequisites:** Phase 10 (Kernel Completeness — #12's ontology-as-types resolution is a hard prerequisite for inductive-type property access).

**Drives:** `docs/design/life-science-requirements.md` §18 Tier 1 + Tier 2 extensions.

### Phase 11a — `Map` and `Reduce` as type-level primitives — ✓

**Status:** Complete.

- `Exp::Map` and `Exp::Reduce` are first-class AST nodes with dedicated eval/check/readback arms.
- Dual list representation: `Val::List(Vec<Val>)` (primary, for resource arrays) + cons-pair chains (legacy, for algebraic construction). `cons_to_vec` normalises between them.
- Neutral forms `Neut::NtMap` / `Neut::NtReduce` for blocked computation.
- Array↔List bridge: `resource_value_to_val` converts `Value::Array` to `Val::List`; `val_to_resource_value` converts back; `resolve_property_type` returns proper list types for `resource_array` / `value_array` properties.
- Expression builder emits `Exp::Map` / `Exp::Reduce` directly (removed `__map` / `__reduce` sentinel variables).
- Traced evaluation produces `Trace::Map` / `Trace::Reduce`.
- Type checker infers `Map(f, coll)` and `Reduce(f, init, coll)` types with `extract_list_element_type` helper.

### Phase 11b — Inductive types — ✓

**Status:** Complete. See D19 (`docs/design/d19-inductive-types.md`) §13 for the 18-step implementation plan; all steps delivered.

- `Exp::Inductive`, `Exp::InductiveType`, `Exp::InductiveCtor`, `Exp::InductiveRec`, `Exp::Match` for declaration, application, construction, elimination via recursor, and motive-inferred pattern matching.
- `Val::InductiveType { decl, params }` / `Val::InductiveVal { decl, ctor_name, args }` / `Neut::NtRec` / `Neut::NtMatch`.
- Positivity checker module (`nbe/positivity.rs`) rejecting non-strictly-positive declarations.
- Automatic recursor derivation (`nbe/recursor.rs`), preserving `SizedPi` binders in minor signatures.
- Iota-reduction integrated with the conversion algorithm; readback round-trips.
- `Exp::list()` replaced with a proper inductive List backed by `Arc<InductiveDecl>`.
- ESL surface syntax: `data Name(p : Kind, …) { ctor, ctor({j < i}, Nat(j)), … }` with brace-delimited bounded-size binders.
- **Sized types (#16, D19 §8):** `Exp::SizeSort` / `SizeSucc` / `SizeInf` / `SizedPi`; ∞-absorption; Warshall meta-solver (`nbe/sized.rs`) + TSO rigid-hypothesis tracker (`nbe/sized_rigid.rs`) ported from MiniAgda; `size_le` / `size_lt` partial orders with hypothesis-consultation variants; size-aware `subtype_of` integrated into the checker fallthrough.
- **Termination-by-typing:** pattern-match arms on sized inductives introduce the bounded size as a rigid with TSO hypothesis, letting sub-term recursive calls type-check at strictly smaller sizes.
- **Productivity-by-typing:** `Lam` checked against `Val::SizedPi` opens the bound size with a hypothesis; sized codata observations using `SizedPi` give productivity as a typing consequence. Replaces D11's syntactic `check_guarded` for sized codata; guardedness remains as a legacy fallback for unsized codata per D19 §8.5.
- **Self-referential parameterised codata:** `Val::CodataType { decl, params }` parallels `Val::InductiveType`; `CodataDecl` carries observations with self-references encoded via a name-only stub Arc; `resolve_full_codata_decl` rehydrates stubs during check.
- **Scope boundary (D19 §2):** single, non-mutual, non-nested, strictly-positive — all honored. Mutual (#20), nested (#21), indexed families (#22) remain deferred.
- **Tests:** 585 kernel tests pass, including Nat/List/Tree, sized Nat with bounded binders, sized codata productivity, self-referential sized streams, and mixed inductive+codata end-to-end from ESL.

### Phase 11c — Institution-registered decision procedures — ✓

**Status:** Complete; **dispatch backbone re-targeted under D14 in Phase 12.** `Constraint::Institution { iri, args }` and `DecResult` survive; the procedural `FiberReasoner::decide` it dispatched to was retired and replaced with Decidable QueryClass dispatch via `Institution::query` + Verdict parsing (D14 §9.2). The check-time escalation (`EvalCtx::Check`, `CheckCtx::with_institutions_d14`) survives, now carrying `InstitutionIndex` + `InstitutionRuntime` instead of an `InstitutionRegistry`.

- `Constraint::Institution { iri, args }` variant on `Constraint` ([kernel/src/nbe/term.rs](../../kernel/src/nbe/term.rs)) — institution-dispatched predicates carry an IRI and a vector of argument expressions.
- `DecResult { Holds, Fails, Undecidable }` — three-valued so institutions can distinguish "predicate is false" from "can't determine at check time." Originally returned by `FiberReasoner::decide`; under D14 it's the kernel-internal tag produced by parsing a Verdict resource (`parse_verdict`) returned by `Institution::query`.
- `EvalCtx::Check` variant — a check-time evaluation mode carrying `InstitutionIndex` + `InstitutionRuntime` (originally `InstitutionRegistry`, retired). The type-checker escalates from `EvalCtx::Pure` to `EvalCtx::Check` when the index/runtime are attached.
- `CheckCtx` gains optional D14 `institution_index` + `institution_runtime` fields (`with_institutions_d14` builder) and a `ctx.eval(...)` method routing internal evals through `EvalCtx::Check`. This is the plumbing that makes institution-dispatched constraints fire at check time rather than at runtime.
- `val_to_resource_value` extended to marshal `Val::InductiveVal` to an embedded resource (ctor name as `is_a`, positional args under `ctor_arg_{i}`). Combined with the existing Phase 11a bridge for `Val::List` and cons-pair chains, this gives institutions concrete life-science argument shapes — scalars, ensembles, Pose-like inductive values — without ad-hoc marshalling.
- Test coverage: default-Undecidable (no index), Holds→Refl, Fails→failing neutral, Undecidable→passthrough neutral, scalar/list/InductiveVal arg roundtrip through the bridge, full check-time integration test. Originally 8 tests against the FiberReasoner-shaped `FakeInstitution`; migrated in Phase 12 B1 to a D14 `Institution`-shaped fixture reading `decide_args` and returning Verdict resources.
- **Explicit non-goals shipped as-is:** no counter-examples on `Fails`. ESL surface syntax now exists (Phase 11e.1 below), retargeted to the D14 classifier in Phase 12 B4a.

### Phase 11d — `Comorphism` as an ontology class — ✓

**Status:** Complete; **shape replaced under D14 in Phase 12.** The `Comorphism` ontology class and `Exp::InstitutionInvoke` AST node survive; the procedural `FiberReasoner::translate` it dispatched to was retired and replaced with the four-step pipeline (extract_typed → transformation Component → reify) keyed by the new triadic Comorphism shape `(export_format, transformation, import_format, exact)`. The original `(source_institution, target_institution, translation_procedure)` shape is gone.

- `urn:eigenius:institution:Comorphism` class in the institution ontology, now with required properties `export_format`, `transformation`, `import_format`, `exact` (D14 §4.5). The original `source_institution` / `target_institution` / `translation_procedure` properties were removed in Phase 12 M1.
- `FiberDeclaration.comorphism_types` (the procedural-registration field) is gone. Comorphisms now ride into the chain as ordinary ontology resources, indexed by `InstitutionIndex` (Phase 12 M2).
- `InstitutionIndex.comorphism(iri)` lookup replaced the old `InstitutionRegistry.institution_for_comorphism()` / `comorphism_institution_iri()` accessors.
- `Exp::InstitutionInvoke { comorphism_iri, source }` kernel AST node — eval dispatches via the four-step pipeline (`try_d14_institution_invoke`) under D14, with the post-translation validation invariant (D14 §9.3 step 5) firing AutoOnLoad QueryClasses on the reified target. Without an attached index/runtime (bare-Pure mode), reduces to a passthrough neutral so the conversion checker can compare two invocations structurally.
- Test coverage: comorphism index ingest, four-step pipeline end-to-end (PipelineLogger fixture in `nbe::eval` tests; dock→assay worked example in `kernel/tests/d14_dock_assay_demo.rs`), unknown-comorphism error, no-index passthrough, post-translation invariant rejecting invalid translations.
- **Explicit non-goals shipped as-is:** no comorphism composition (ρ₁ ∘ ρ₂) — D14 §5.2 deliberately excludes it; no backward translation. ESL surface (`f(x)` → `Exp::InstitutionInvoke`) ships under Phase 11e.1, retargeted to the D14 classifier in Phase 12 B4a. EigenQL surface for comorphism dispatch ships as FIBER param coercion under D2 v2.

### Phase 11e.1 — ESL surface for institution capabilities — ✓

**Status:** Complete; **classifier ported under D14 in Phase 12 B4a.** The ESL surface (`cap:comorphism(src)` → `Exp::InstitutionInvoke`; `cap:decide(a, b)` → `Exp::NativeDecide(Constraint::Institution, Unit)`) survives. The classifier consulted to make those routing decisions changed: `InstitutionRegistry::classify` was retired and replaced with `InstitutionIndex.comorphism(iri).is_some()` and `InstitutionIndex.query_class(iri)` checked for `DispatchRole::Decidable`.

- ESL surface: `compile_with_institutions(source, Arc<InstitutionIndex>)` entry point. `Compiler.institutions: Option<Arc<InstitutionIndex>>` drives compile-time classification in the `Apply` arm.
- Classification rules: function IRI is a Comorphism declaration → emits `ComorphismInvokeApply` resource; QueryClass declaration with `Decidable` dispatch role → emits `DecideApply`; otherwise falls through to ordinary component dispatch.
- `program::expr` decoders for `ComorphismInvokeApply` → `Exp::InstitutionInvoke`, and `DecideApply` → `Exp::NativeDecide(Constraint::Institution, Unit)` (unchanged).
- Test coverage: ESL `cap:comorphism(src)` compiles to `InstitutionInvoke`; ESL `cap:decide(a, b)` compiles to `NativeDecide(Institution)`; comorphism called with wrong arity errors at compile; no-index path compiles to plain `Apply` (backward-compatible).
- **Explicit non-goals:** `decide` result binding (still uses `Exp::Unit` as witness); typed argument signatures for predicates; ESL syntax for comorphism composition.

### Phase 11e.2 — EigenQL surface for institution capabilities — ✓

**Status:** Complete; **dispatch ported under D14 in Phase 12 B4b, semantics revised in D2 v2.** EigenQL accepts `ns:local(args)` qualified-name function calls in expression position. Under D14 these dispatch only as Decidable QueryClass invocations and return a typed `Verdict` (no longer Boolean). Comorphism dispatch in expression position was dropped — comorphisms surface as FIBER parameter coercions instead (D2 v2 §3.5). The postfix `HOLDS` / `FAILS` / `UNDECIDABLE` Verdict projection sugar lives in the open D2 v2 surface implementation milestone (see Phase 12).

- EigenQL parser accepts `ns:local(args)` qualified-name function calls (unchanged).
- `eval_expression` threads `FiberRuntime { index, runtime, ctx }` through the call chain; `FunctionCall` dispatch routes to `try_dispatch_decidable` against the `InstitutionIndex` before falling through to the builtin function table.
- `try_dispatch_decidable`: looks up the IRI as a Decidable QueryClass; marshals positional arguments onto a `decide_args` array on a synthetic input resource; calls `Institution::query`; returns the resulting Verdict resource as `Value::Embedded`.
- WHERE / Boolean-position semantics under D2 v2: a Verdict-typed expression is no longer auto-collapsed; the user applies a postfix predicate (`?v HOLDS`) to project. Bare Verdict in Boolean position is a type error (`bare_verdict_in_boolean_position`). Comorphism dispatch in expression position is gone (`f(x)` returning a translated resource is no longer a thing — use FIBER param coercion instead).
- Test coverage: parser accepts qualified calls; Decidable call returns Verdict resource (asserts `ctor_name`); unknown IRI falls through to builtin-dispatch error. (The original 4-test `Boolean(Holds)` semantics + comorphism-in-expression test were retired in B4b.)
- **Explicit non-goals:** no namespace-alias declaration in EigenQL syntax (users write full IRIs); no type-level classification at parse time. Postfix Verdict predicates and FIBER comorphism coercion are tracked under the D2 v2 surface implementation milestone in Phase 12.

### Phase 11 — Test plan

- Inductive type declaration + recursor + iota reduction: positive examples (Nat, List, Tree, a life-science morphism type) and negative examples (non-strictly-positive rejected).
- Map/Reduce desugaring + termination: compile ESL that uses Map/Reduce, verify no `Drec` leaks through.
- Institution-registered decide: an ordering-institution extension that decides `|delta| ≤ tolerance` at check time; property access through the decided constraint.
- Comorphism resource declaration + validator-integration test.

### Phase 11 — References

- `docs/design/life-science-requirements.md` §16 (required extensions), §18 (prioritization), §19 (sequencing)
- `docs/design/lean-4-as-institution.md` — nanoda reference, especially Appendix A
- D19 (`docs/design/d19-inductive-types.md`) — Inductive Types + Sized Types in Mini-TT

---

## Phase D22 — Notebook UX and TypeScript SDK ✓

**Goal:** Deliver a low-friction interactive surface on top of the kernel — a React single-page notebook served at `/notebooks/` and a typed `@eigenius/client` SDK consumable from any browser / Deno / Node runtime. Both are operational today and ship inside the orchestrator Docker image.

**Status:** Complete. Tracked outside the main 0–15 sequence because it's a parallel UX/SDK track rather than a kernel-capability phase; placed here in the document for chronological orientation. Internal sub-phases (per [D22](d22-notebook-and-typescript-sdk.md) §7):

- **Phase 1 — SDK foundation ✓** — `Eigen` class wraps Connect-RPC; `inspect`, `query`, `load`, `runProgram`, `runProgramByIri`, `layerTopology`, `publishNotebook`. Connect codegen wired; smoke test in `clients/eigenius-ts/examples/smoke-test.ts`.
- **Phase 2 — Static viewer ✓** — read-only notebook rendering with the four MVP cell types.
- **Phase 3 — Manual execution ✓** — Run / Run all / Reset, per-cell run states, output panels.
- **Phase 4 — Authoring (the MVP) ✓** — full editing UX, file Open / Save, the program-run cell, layer-stack and trace-tree visualisations, multi-stage `Dockerfile.orchestration` so the SPA serves alongside the RPC paths at `http://localhost:8080/notebooks/`.
- **Phase 5 — Visualisation ✓** — `@fluentui/react-charts` integration via TS-cell sandbox helpers (5a), `@xyflow/react` topology graph with per-layer drilldown (5b), and a dedicated form-based **chart cell** type covering grouped-bar / vertical-bar / horizontal-bar / donut / line / area kinds (5d). The `kinase-screening` notebook exercises every chart kind plus the topology graph.
- **Phase 6 — Reactivity + polish ✓** — sticky header with Pin toggle, file IO renamed Import / Export, dedicated **Open published notebook** dialog backed by EigenQL search, dismissable header MessageBars, mandatory `notebook:title` (ontology + SDK + UI guards), per-cell collapse/expand + global Expand/Collapse all, edit-metadata dialog with description editing, on-demand resource fetch for the per-layer topology graph. Reactivity: `Run` per cell becomes a `SplitButton` with `Run` / `Run from here…` / `Run to here…`; subdued cell-order **stale** marker tracks `lastRunCellId` honestly without pretending to model TS-to-TS dataflow (the explicit DAG approach was rejected because it can't see kernel-layer side effects — see [eigenius#33](https://github.com/eigenius/eigenius/issues/33) for the proper EigenQL `OPTIONAL` follow-on).
- **Cross-cutting fixes during the polish round:** topology walker edge dedup on first sighting (was emitting each schema edge once per layer, blowing up the displayed edge count); chart titles wrapped externally for the cartesian + horizontal-bar + donut kinds for visual consistency (Fluent's `CartesianChart` `chartTitle` prop is aria-only).

### Phase D22 — Deliverables

- `clients/eigenius-ts/` — `@eigenius/client` package + `Eigen` class + content-addressed publish translator.
- `notebooks/` — Vite + React + TypeScript SPA. Cell types: `markdown`, `esl`, `eigenql`, `typescript`, `program-run`, `chart`. Auto-renderers for ResultSet, Resource, ProgramTrace, LayerStack, Topology, raw values.
- `ontologies/notebook/notebook-ontology.json` — `Notebook`, `Cell`, `CellType` classes; baked into the kernel as the 5th bootstrap layer so publish succeeds without first registering anything.
- `deploy/Dockerfile.orchestration` — multi-stage build that compiles the SPA and serves it from `EIGENIUS_NOTEBOOK_STATIC=/app/notebooks` at `/notebooks/*`.
- Two Playwright e2e tests: `patent-demo.spec.ts` (the LLM-free critical path through the patent demo) and `kinase-screening.spec.ts` (chart-cell regression coverage across all six kinds).

### Phase D22 — References

- [D22 — Notebook UX and TypeScript SDK](d22-notebook-and-typescript-sdk.md) — full spec including the Eigon-CBOR ↔ TypeScript marshalling rules
- [Platform guide chapter 13 — Notebook](../guides/platform/13-notebook.md)
- [Platform guide chapter 14 — TypeScript SDK](../guides/platform/14-typescript-sdk.md)

---

## Phase 12 — D14 Institution Realisation

**Goal:** Replace the D10 institution surface with [D14](d14-institution-realisation.md)'s ontology-first realisation, retire the legacy types, and ship a worked example that exercises the full surface end-to-end — the four-step comorphism pipeline, Decidable QueryClass dispatch at type-check time, and AutoOnLoad QueryClass dispatch on Load.

**Duration estimate:** 6–8 weeks.

**Prerequisites:** Phase 6 (institution protocol — the surface being retired), Phase 8 (WASM extensibility), Phase 10 (ontology-as-types so EigenQL queries over institution classes type-check cleanly), Phase 11 (in full — d for comorphism translations, b for inductive types underpinning Verdict, c for the `Constraint::Institution` AST node).

D14 supersedes [D10](d10-grothendieck-institution-protocol.md). Phase 12 adopts D14 as its scope; the D10-era plan that previously occupied this slot ("two domain examples each as a WASM-sandboxed FiberReasoner") was abandoned mid-flight in favour of fixing the structural shape first. The D14 redesign replaced the procedural `FiberReasoner` trait + `InstitutionRegistry` with a derived `InstitutionIndex` over chain-declared institution / format / query / comorphism resources, plus an `InstitutionRuntime` of `Institution` trait implementations.

### Phase 12 — D14 milestones (D14 §13.4)

D14 sequences eight milestones M1–M8 covering the redesign. All eight have landed against the kernel; the WASM packaging and EigenQL surface enrichment of M8 remain open and are tracked below.

#### M1 — Ontology shape + well-known IRIs + Verdict — ✓

**Status:** Complete.

- Institution ontology revised: `Verdict` inductive type (constructors `Holds | Fails | Undecidable`); `RuntimeKind`, `DispatchRole`, `ExportFormat`, `ImportFormat`, `QueryClass` classes; `Comorphism` re-shaped as the triadic (export_format, transformation, import_format, exact) tuple from D14 §4.5. Comorphism's old (source_institution, target_institution, translation_procedure) shape removed.
- Well-known IRI constants in `kernel/src/ontology/well_known.rs`: `EXPORT_FORMAT_CLASS`, `IMPORT_FORMAT_CLASS`, `QUERY_CLASS_CLASS`, `VERDICT`, dispatch-role IRIs, ctor-name constants for the three Verdict constructors.

#### M2 — InstitutionIndex (chain-scan derived registry) — ✓

**Status:** Complete.

- `kernel/src/institution/registry.rs` — `InstitutionIndex` builds from `from_layer(&Layer)`, walking the chain and ingesting every Institution / ExportFormat / ImportFormat / QueryClass / Comorphism declaration. Per-resource parse errors collected and returned alongside the index so callers can surface them as validation problems.
- Dispatch sub-indexes: `auto_on_load_by_class`, `on_demand_by_class`, `decidable_by_class`, `procedures` (procedure IRI → declaring institution + ProcedureKind).
- Lookup API: `query_class(iri)`, `comorphism(iri)`, `auto_on_load_for(class_iri)`, `decidable_for(class_iri)`, `procedure(iri)`, `institutions()` iterator.

#### M3 — Institution trait + InstitutionRuntime — ✓

**Status:** Complete.

- `kernel/src/institution/runtime.rs` — `Institution` trait with three methods: `extract_typed(procedure, source, ctx) → Val`, `reify(procedure, val, ctx) → Resource`, `query(procedure, input, ctx) → Resource`. `InstitutionRuntime` keys `Box<dyn Institution>` by institution IRI.
- Comorphism well-formedness validation (validation Rule 15): every Comorphism resource's `export_format`, `transformation`, and `import_format` references must resolve in the chain. Surfaces as a structural validation error.
- Dead `kernel/src/institution/comorphism.rs` deleted; the Comorphism shape now lives entirely as ontology data + the dispatch path in `nbe/eval.rs`.

#### M4 — WIT world + SDK update + WASM host bridge — ✓

**Status:** Complete.

- `wit/eigenius-component.wit` — `eigenius-institution-d14` world with `extract-typed`, `reify`, `query` exports; the legacy `eigenius-institution` world removed alongside the legacy SDK builders.
- SDK (`sdk/wasm-sdk/src/institution.rs`): D14 declaration builders for `InstitutionDecl`, `ExportFormatDecl`, `ImportFormatDecl`, `QueryClassDecl`, `ComorphismDecl`. Legacy `FiberDeclaration` / `MorphismValidation` builders removed.
- Host bridge `kernel/src/capability/wasm_institution_d14.rs` — `WasmInstitution` implements the `Institution` trait via Wasmtime calls into the D14 WIT exports. M4 marshalling restriction: only `Val::ResourceVal` is exchanged across the WASM boundary. The `examples/wasm-d14-echo/` smoke fixture verifies the host bridge round-trips inputs with provenance.

#### M5 — `Exp::InstitutionInvoke` four-step pipeline — ✓

**Status:** Complete.

- `kernel/src/nbe/eval.rs::try_d14_institution_invoke` runs the four-step pipeline (D14 §9.3): resolve the Comorphism in the InstitutionIndex; call source institution's `extract_typed` with the ExportFormat procedure; apply the `transformation` Component; call target institution's `reify` with the ImportFormat procedure; run the post-translation validation invariant (D14 §9.3 step 5) by firing AutoOnLoad QueryClasses bound to the produced target class.
- IO mode required (the transformation Component dispatches through the kernel's `ComponentRegistry`). Bare-Pure mode reduces `Exp::InstitutionInvoke` to a passthrough neutral so the conversion checker can compare two invocations structurally.

#### M6 — `Exp::NativeDecide` Decidable QueryClass dispatch — ✓

**Status:** Complete.

- `kernel/src/nbe/eval.rs::try_d14_decide` resolves the constraint IRI as a Decidable QueryClass in the InstitutionIndex; marshals positional arguments onto a `decide_args` array on a synthetic input resource of the QueryClass's input class; dispatches via `Institution::query` against the institution registered at the QueryClass's `institution_ref`; parses the returned Verdict resource into `DecResult` (D14 §9.2).
- ESL surface: `f(x, y)` where `f` is a Decidable QueryClass IRI compiles to `Exp::NativeDecide(Constraint::Institution{..}, Unit)` via `kernel/src/esl/compile.rs`'s D14 classifier.
- EigenQL surface: qualified-name function calls dispatch through `kernel/src/query/evaluate.rs::try_dispatch_decidable` and return a `Verdict`-typed value (no longer Boolean — D2 v2 §3.8). The post-fix `HOLDS` projection sugar is part of the open D2 v2 surface implementation below.

#### M7 — AutoOnLoad dispatch + post-translation invariant — ✓

**Status:** Complete.

- `kernel/src/institution/dispatch.rs::dispatch_auto_on_load_for_resource` and `…_for_layer` — fire AutoOnLoad QueryClasses bound to a resource's class, parse the Verdict, surface `Fails` as a typed `ValidationError`. `Holds` and `Undecidable` accept silently.
- `kernel/src/context/mod.rs::commit_with_validation` — atomic head-promote-then-revert-on-failure semantics for AutoOnLoad commit gating. The Load RPC routes through this so a `Fails` Verdict aborts the commit before the layer becomes visible.
- Post-translation invariant: `try_d14_institution_invoke` (M5) calls `dispatch_auto_on_load_for_resource` on the reified target resource. A failing AutoOnLoad surfaces as a comorphism-implementation bug rather than a silent commit of an invalid translation.

#### M8 — Worked-example demo (kernel-level) — ✓

**Status:** Phase 1 (kernel surface) complete.

- `ontologies/examples/d14-dock-assay/dock-assay.json` — the dock→assay scenario from D14 §5.1: `Dock` and `Assay` Institutions, `DockingResult` and `AssayPrediction` classes, `WithinToleranceInput` class, `ef_dock_to_dg` ExportFormat, `if_assay_from_ic50` ImportFormat, `cm_arrhenius` transformation Component, `dock_to_assay` Comorphism, `within_tolerance` Decidable QueryClass, `assay_prediction_validity` AutoOnLoad QueryClass.
- `kernel/tests/d14_dock_assay_demo.rs` — integration test wiring two in-process Rust `Institution` impls (Dock, Assay) and a `BuiltinComponent` for the Arrhenius approximation `IC₅₀ ≈ exp(-ΔG/RT)·1e⁹`. Four `#[test]` cases — comorphism translation, Decidable holds-in-tolerance, Decidable fails-outside-tolerance, AutoOnLoad on Load — exercising every D14 dispatch path against a real-domain example.
- Phase-2 enrichment (WASM packaging, EigenQL queries) is open work tracked below.

### Phase 12 — D14 retirement (B1–B4) — ✓

The D10-era surface is fully retired:

- **B1** — Test fixtures migrated off the legacy `FiberReasoner` trait to the D14 `Institution` trait.
- **B2** — `EigeniusService` wired with `InstitutionIndex` + `InstitutionRuntime`; per-commit index rebuild; commit gating through `commit_with_validation`; M5 four-step pipeline dispatch attached to `Exp::InstitutionInvoke` evaluation.
- **B3** — Legacy `WasmFiberReasoner` host code, `wasm-ordering-institution` example crate + fixture, `eigenius-institution` legacy WIT world, `wasm_institution.rs`, the legacy fiber-query durability test, all removed.
- **B4a** — `kernel/src/esl/compile.rs::classify` ported from `InstitutionRegistry::classify` to `InstitutionIndex` (decide → Decidable QueryClass, comorphism → Comorphism declaration).
- **B4b** — `kernel/src/query/evaluate.rs` FIBER-clause evaluator + qualified-name expression dispatch ported to D14: `FiberRuntime` carries `InstitutionIndex` + `InstitutionRuntime` + `ExecutionContext`; `apply_fiber_clause` dispatches via `Institution::query` against an OnDemand QueryClass; qualified-name expressions return Verdict-typed values via `try_dispatch_decidable`.
- **B4c** — Legacy fallback branches in `Exp::InstitutionInvoke` and `decide_constraint` deleted. Only the D14 dispatch paths remain.
- **B4d/e** — `FiberReasoner` trait, `InstitutionRegistry` + impl + `Default`, `FiberDeclaration`, `InstitutionInfo`, `InstitutionCapability`, `MorphismValidation`, `validate_with_institutions`, `EvalCtx::institutions` field, `CheckCtx::institutions` field, `EigeniusService::institutions` field, `dispatch_fiber_query`, the `Exp::App` IO-branch institution-dispatch logic, SDK builders, well-known IRI constants for the legacy Comorphism shape, the `FiberQuery` and `DiscoverMorphisms` RPCs (stubbed as `Status::unimplemented`), `ListInstitutions` RPC ported to read from `InstitutionIndex` — all retired in one coordinated sweep.

### Phase 12 — Open work

#### M8 enrichment — WASM packaging of the dock-assay demo

**Status:** Pending.

- Two new WASM crates targeting the `eigenius-institution-d14` WIT world: `examples/wasm-d14-dock` (extract-typed reads `delta_g`) and `examples/wasm-d14-assay` (reify constructs an `AssayPrediction`; query handles both the `within_tolerance` Decidable and the `assay_prediction_validity` AutoOnLoad QueryClass).
- One pure WASM Component crate `examples/wasm-d14-arrhenius` implementing the `cm_arrhenius` transformation against the `eigenius-component` WIT world.
- Ontology revisions to switch the Dock and Assay `Institution` declarations from `runtime: in_process` to `runtime: wasm` with `wasm_binary` (or `wasm_binary_ref`) populated; switch the `cm_arrhenius` Component declaration to point at its WASM binary.
- Build and CI plumbing: workspace `Cargo.toml` exclude list, `justfile`, `.github/workflows/ci.yml` (cache key + build steps), ontology fixture-build script.
- A second integration test exercising the same four scenarios as `d14_dock_assay_demo.rs` end-to-end through WASM (verifying the host bridge plus `WasmInstitution` plus `WasmComponent` plumbing routes correctly under a real domain).

#### D2 v2 EigenQL surface implementation

**Status:** Pending. Spec authoritative ([D2](d2-eigenql-specification.md) v2 §3.3.1, §3.5, §3.7, §3.8, §5.7–5.9, §6.12, §6.13, §7, §9). Implementation:

- Lexer additions: `HOLDS`, `FAILS`, `UNDECIDABLE` keyword tokens.
- AST: `ParamBinding.value: ParamValue` sum type with `Expression` and `Comorphism { name, source }` variants; `Expression::VerdictPredicate { kind, operand }` variant.
- Parser: `comorphism_coercion` recognised inside FIBER param value position; `verdict_term ::= primary_expr (verdict_predicate)?` postfix shape inserted between unary and primary in the precedence chain.
- Type checker: every D14 rule from D2 §5.7–5.9 — `using_institution_unresolved`, `fiber_query_class_not_query_class`, `fiber_query_class_not_on_demand`, `fiber_institution_mismatch`, `fiber_param_short_name_unresolved`, `fiber_missing_required_param`, `comorphism_unresolved`, `comorphism_target_mismatch`, `comorphism_io_not_supported_in_v1`, `comorphism_target_class_mismatch`, `comorphism_source_not_resource`, `qualified_call_not_decidable`, `verdict_predicate_non_verdict_operand`, `bare_verdict_in_boolean_position`.
- Evaluator: comorphism coercion in FIBER params runs the four-step pipeline inline per D2 §6.12 (Pure/Read transformation only — IO is rejected at type-check); postfix predicate reads `ctor_name` off the operand and projects to Boolean.

#### Demo enrichment with EigenQL surface

**Status:** Pending. Blocked on the D2 v2 surface implementation above. Once that lands, the dock-assay demo gets a parallel EigenQL exercise:

- A FIBER-comorphism-coercion query (`FIBER assay:within_tolerance { predicted_ic50: dock:dock_to_assay(?d), … } AS ?v WHERE ?v HOLDS RETURN [] { d: ?d }`) showing the natural surface promised by D2 v2 §3.5.
- Postfix-predicate examples in WHERE and RETURN positions covering `HOLDS`, `FAILS`, `UNDECIDABLE`.
- Multi-FIBER chain showing one institution's Verdict feeding the next clause.
- Integration tests via the EigenQL evaluator alongside the existing kernel-level tests.

#### Docs/guides D14 rewrite

**Status:** Pending. Coordinated rewrite of every guide page that still teaches the legacy surface:

- `docs/guides/platform/10-wasm-institutions.md` — full rewrite around the D14 `Institution` trait + `eigenius-institution-d14` WIT world + the M8 worked example.
- `docs/guides/platform/03-building-and-testing.md`, `08-demos.md`, `09-wasm-components.md`, `15-appendix.md`, `README.md`, `01-introduction.md`, `12-troubleshooting.md` — D14-vocabulary updates and example-listing fixes.
- `docs/guides/esl/09-institutions.md`, `eigenql/08-institutions.md` — full rewrites on the D14 trait and D2 v2 surface.
- `docs/guides/esl/01-introduction.md`, `11-appendix.md`, `eigenql/02-quick-tour.md`, `04-program-structure.md`, `06-expressions.md`, `11-error-messages.md`, `12-appendix.md` — focused updates around dispatch, decide semantics, and Verdict projection.

#### Proto cleanup

**Status:** Pending. The kernel currently serves `FiberQuery` and `DiscoverMorphisms` as `Status::unimplemented` stubs (no D14 equivalent — superseded by Query+FIBER and by user-defined OnDemand QueryClasses respectively). Cleanup:

- Drop both RPCs from `proto/eigenius.proto`.
- Remove the stub implementations from `kernel/src/server/mod.rs`.
- Sweep the orchestrator + TypeScript client SDK + design docs for references.
- Re-evaluate the `morphism_types` field on `proto::InstitutionInfo` (currently empty under D14; either rename, drop, or expand the surface).

#### Optional second worked domain

**Status:** Deferred — not required for Phase 12 closure. The D10-era plan called for two domain examples (mechanical engineering and biopharma). Phase 12 ships one (dock-assay) end-to-end through the full D14 surface; a second domain (mechanical engineering FEA/CAD/GenAI per the D10-era description, or another life-science scenario) can land once the dock-assay path is fully WASM-packaged and EigenQL-enriched.

### Phase 12 — Test plan

- Kernel-level: `kernel/tests/d14_dock_assay_demo.rs` exercises every D14 dispatch path (four-step comorphism, Decidable, AutoOnLoad) against the worked example. ✓
- WASM-level: a parallel integration test loading the institutions + transformation through the kernel WASM-component install path will exercise the same scenarios end-to-end. (Open work, with the WASM packaging milestone above.)
- Surface-level: D2 v2 spec rules each get a focused parser/typecheck/evaluator test as part of the surface implementation; the D2 v2 §8.13 (Decidable + postfix) and §8.14 (FIBER comorphism coercion) examples become integration tests. (Open work.)
- Negative coverage: malformed declarations (missing required fields) parsed via `InstitutionIndex::from_layer` produce typed `IndexError`s; AutoOnLoad `Fails` produces a typed `ValidationError`; comorphism `Fails` post-translation aborts with a typed `EvalError`. ✓

### Phase 12 — References

- [D14 — Institution Realisation](d14-institution-realisation.md) — the canonical specification for the work in this phase.
- [D2 — EigenQL Specification](d2-eigenql-specification.md) — v2 revision aligned to D14 (institution surface, FIBER comorphism coercion, postfix Verdict predicate).
- [D10 — Grothendieck Institution Protocol](d10-grothendieck-institution-protocol.md) — superseded by D14 (the file is a redirect).
- `docs/papers/eigenius-institutions.tex` — categorical motivation; §5/§6 worked examples are still useful for picking a second domain (mechanical engineering / additional biopharma scenarios) when the Phase-12 closure work demands it.
- `docs/design/life-science-requirements.md` §10, §11 — representational shapes the dock-assay example (and any future second domain) covers.

---

## Phase 13 — Azure Deployment and Operations

**Goal:** Production-ready deployment to Azure Container Apps with CI/CD, observability, and operational tooling. Optional TiKV backend for horizontally scalable storage. Placed after Phase 12 so releases have compelling worked examples to demonstrate, not just infrastructure.

**Duration estimate:** 3–4 weeks.

**Prerequisites:** Phase 9a (durable state — deployment assumes `--db`). Other phases are optional but shipping a Phase 12 demo is the obvious deployment target.

### Phase 13 — Deliverables

- Azure deployment: update Bicep templates for Container Apps, wire `ANTHROPIC_API_KEY` through Key Vault, configure DAPR sidecars for mTLS and service discovery.
- CI/CD: GitHub Actions release workflow — build containers, push to ACR, deploy to staging, health check validation, production promotion.
- Structured logging: kernel and orchestrator emit structured JSON logs with trace IDs, compatible with Azure Monitor / Application Insights.
- Metrics: Prometheus-compatible metrics endpoint on the kernel — request counts, latency histograms, trace cache hit rate, LLM token usage.
- Health probes: HTTP health endpoints on both services, compatible with Container Apps readiness/liveness probes.
- TiKV storage backend: optional alternative to RocksDB for horizontally scalable deployments. Same key encoding (D4), same API (`LayerStore`/`ResourceStore` traits). Configurable via environment variable.
- Operational runbook: deployment procedures, scaling guidelines, backup/restore, incident response.

### Phase 13 — Test plan

- Docker build succeeds for both images
- Containers start and pass health checks
- CI/CD pipeline: push tag → build → ACR → staging → health check → promotion
- TiKV backend: all existing storage tests pass against TiKV
- Structured logs: verify log entries contain trace IDs and are parseable
- Metrics: verify Prometheus scrape returns expected metrics

### Phase 13 — References

- eigenius/eigenius#4 — Azure deployment ticket
- `deploy/bicep/` — existing Bicep templates
- `docs/design/d4-storage-key-encoding.md` — TiKV-compatible key scheme

---

## Phase 14 — Out-of-Core Layer Architecture

**Goal:** Decouple layer topology from resource content so the kernel's working set is bounded by cache size rather than graph size. Generalise the layer model from a single chain to a DAG with branches, multi-session writes, and lifecycle operations (GC, pruning). Read-side query path becomes index-driven through the storage backend; result-set processing remains in memory (operator-level spill is Phase 16).

**Duration estimate:** ~10 weeks.

**Prerequisites:** Phase 9a (durable state — backend already persists everything that needs to be on disk), Phase 12 (worked examples — gives realistic graph-size workloads to validate the cache and index design under).

**Motivation:** Today the layer chain is `Arc<Layer>` with full BTreeMap content held in memory; queries iterate against those BTreeMaps. The model breaks once graph size exceeds RAM, and degrades long before that for long-lived databases. Branching shares the same root cause: multiple active heads multiply the in-memory footprint linearly. Bundling the storage rework with the DAG primitive avoids building single-chain caching machinery only to retrofit it for branches later. Merging — the *semantic* operation that fuses two branches with a comorphism witness — is genuinely separable and lives in Phase 15.

### Phase 14 — Sub-milestones

- **14a — Topology / content split (~1 week).** In-memory DAG of `LayerId → parent[s]`, branch heads, named refs. Resource content moves to a cache keyed by `(LayerId, IRI)`. Naïve top-down lookup walks the topology and falls through to the storage backend on cache miss; correctness first, performance later.
- **14b — Per-head shadowing index (~1.5 weeks).** Persistent `IRI → highest_defining_layer` index per branch head, maintained incrementally on commit (same atomic write batch as the layer itself). Bloom-filter front end for negative answers. Reduces lookup from O(depth) probes to one. Time-travel queries reconstruct an "as-of-L" view via per-named-head checkpoints + on-demand rewind.
- **14c — Two-pool cache + eviction (~1 week).** Active-head pool (entries that are currently the top per the active head's shadowing index) vs. historical pool (entries shadowed in every active head, only reachable via time-travel or trace dereferences). ARC inside each pool; historical pool evicted first under memory pressure.
- **14d — `commit_layer` + `update_branch` with CAS (~1.5 weeks).** The two stateless write primitives that anchor the lattice. `commit_layer(parent, content)` appends an immutable layer to the DAG; `update_branch(branch, expected_old, new_head)` advances a branch ref via CAS with a `FastForward | NeedsWitnessedMerge` outcome (trivial merge ships in 14e). Pin is just a parameter; no kernel-side `Session`, `PromotionService`, or scratch-chain abstraction — clients (CLI / notebook / task runner / SDK) orchestrate. D21's `TaskRecord.layer_head` is already the per-task pin and needs no structural change.
- **14e — Trivial merge in `update_branch` + branch read surface (~2 weeks).** Extend `update_branch` with the `TrivialMerge` outcome: when the caller's chain and the branch's current head modify disjoint sets of IRIs since their lowest common ancestor, the kernel produces a multi-parent merge layer automatically and CAS-updates the branch to point at it. Witnessed merges (real conflicts) still return `NeedsWitnessedMerge` for Phase 15 to handle. Also lands `BranchManager` read/list/delete surface, the `auto-*` naming convention for client-saved divergent chains, and a small additive `outcome: BranchUpdateOutcome` field on D21's `TaskRecord` so users can see whether a task fast-forwarded, trivially merged, or needs witnessed-merge resolution. The trivial-merge case handles the majority of real-world divergence in dev workflows; without it Phase 14 ships with a sharp usability cliff for any concurrent activity.
- **14f — Reachability-based GC (~2 weeks).** Mark-and-sweep over the resource graph. Roots: pinned branch heads, active sessions, resources referenced by reflection-ontology traces, verified-knowledge claims. Background task with backpressure; configurable triggers (size threshold, idle interval).
- **14g — Branch pruning (~1 week).** `eigenius db prune <branch>` removes a branch from the topology; GC sweeps anything reachable only through it. Rejects pruning of branches with active sessions.
- **14h — Indexed resource access for queries (~1.5 weeks).** Wire the existing SPO/POS/OPS indexes from `storage/indexing/` through the EigenQL evaluator's pattern-matching path. `MATCH ?x : Class { prop = ?v }` becomes an indexed lookup against the storage backend instead of a full BTreeMap scan. Result-set processing (joins, sorts, group-by) stays in memory — operator spill is Phase 16. After this lands, queries continue to work; the read-side working set just shifts off-heap.

### Phase 14 — Key design questions

- **Working-set bound:** fixed LRU size, adaptive (hit-rate-aware), or eviction-policy-as-config?
- **Trace pinning:** does an active reflection-ontology trace pin its referenced resources from GC? Default instinct: yes — the epistemic guarantee depends on the chain being readable. Implies traces have explicit lifetime / expiration policy.
- **Migration vs. shadowing:** when an ontology migration rewrites resources via comorphism (Phase 15), does the new resource shadow the old, or supersede it? Different GC semantics either way.
- **Branch identity:** content hash, user label, both? Answers propagate to Phase 15's merge command.
- **Time-travel checkpoint cadence:** checkpoint at every named head only, or at fixed intervals? Storage cost vs. rewind cost trade-off.
- **Index regeneration on merge:** Phase 15's merge creates a new layer; its shadowing index has to be computed. Incremental from parents or full rebuild?

### Phase 14 — References

- D13 §8, §11 — drift-refusal and single-session boundary this phase generalises
- D21 §3.6 — `--at-layer` queries (the time-travel surface that constrains the checkpoint scheme)
- `storage/indexing/` — existing SPO/POS/OPS index implementation Phase 14h plugs into
- D23 (to be written) — Out-of-Core Layer Architecture

---

## Phase 15 — Witnessed Layer Reconciliation

**Goal:** Resolve the residual class of divergence that Phase 14's trivial merge cannot — *witnessed* merges, where two branches modify the same IRI in incompatible ways and reconciliation requires a `Comorphism` witness per Phase 11d to specify the resolution. Trivial merges (disjoint-IRI contributions) already auto-resolve in Phase 14e; Phase 15 closes the loop on the cases that genuinely need translation.

**Duration estimate:** 3–5 weeks (smaller than the original Phase 15 scope because trivial merge moved to Phase 14e).

**Prerequisites:** Phase 11d (`Comorphism` class), Phase 14e (`update_branch` returning `NeedsWitnessedMerge` is the entry point Phase 15 hooks into).

**Motivation:** Phase 14e's `update_branch` returns `NeedsWitnessedMerge { current_head, conflicting_iris }` whenever two branches modify the same IRI in incompatible ways. Pre-Phase-15 the only escape is to save the would-be-merged chain as a sibling branch (an `auto-*` ref) and live with the divergence. Phase 15 turns that residual case into "supply a comorphism witness; the kernel produces a witnessed merge layer." This is the operation that makes the lattice fully consolidatable for cross-cutting ontology evolution. The category-theoretic vocabulary from D10 (layers as a category, comorphisms as institutional view translations) gives the precise semantics — the witnessed merge is a colimit in the layer category up to comorphism equivalence.

### Phase 15 — Deliverables

- **Witnessed merge command:** `eigenius db merge <branch-a> <branch-b> --witness <comorphism>` — requires a `Comorphism` resource that resolves the conflicting IRIs from a `NeedsWitnessedMerge` outcome. Produces a multi-parent merge layer; CAS-updates the target branch to point at it.
- **Programmatic surface:** an `update_branch` variant or paired RPC that accepts a witness directly, so clients hitting `NeedsWitnessedMerge` can immediately retry with a witness rather than going through the CLI command.
- **Ontology migration as a degenerate witnessed merge:** replace D13's v1 drift-refusal with a `migrate` command that takes a `Comorphism` and rewrites persisted resources across a layer boundary. Implementation-wise, a migration is a witnessed merge with one branch being a single-resource diff.
- **Merge layer identity:** the witnessed merge layer is content-addressed like any other; common-ancestor invariants are preserved so subsequent merges have a well-defined LCA.
- **Real-world test surface:** since Phase 14 ships first, users may already have lots of `auto-*` branches by the time Phase 15 lands. The witnessed merge has to handle multi-ancestor cases on real, in-the-wild branch DAGs — not just freshly-created two-branch toy cases.
- **D20 — design doc.** The category-theoretic vocabulary actually pays off here because the witnessed merge operation needs precise semantics.

### Phase 15 — Key design questions

- **Witness sufficiency:** does a single `Comorphism` resource suffice to resolve a multi-IRI conflict, or do we need a per-IRI witness map?
- **Witness composition:** if branches A and B were each previously merged via different comorphisms, does merging A with B compose the witnesses?
- **Merge layer validation:** runs through D14 AutoOnLoad QueryClass dispatch + `commit_with_validation` (Phase 12) or treated as a special case?
- **Active WASM institutions during merge:** institutions re-register against the merged layer as in D13's RESUME path? Under D14 the `InstitutionIndex` is rebuilt per-commit, so a successful merge produces a fresh index over the merged chain naturally.

### Phase 15 — References

- D14 — Institution Realisation (canonical institution surface; supersedes D10)
- D10 — Grothendieck institutions, comorphisms, the category-theoretic vocabulary (superseded by D14; retained as historical motivation)
- D13 §8, §11 — drift-refusal and single-session boundary
- `docs/design/life-science-requirements.md` §11 (cross-institution claims) — consumer of Comorphism class
- D20 (to be written) — Layer Reconciliation via Comorphisms

---

## Phase 16 — Out-of-Core Query Execution

**Goal:** EigenQL operators (hash join, sort, group-by) become memory-bounded, spilling intermediate result sets to disk via a buffer-pool abstraction over the storage backend. After Phase 14, the read path is indexed and cached; after Phase 16, the operator pipeline tolerates result sets larger than memory.

**Duration estimate:** 4–6 weeks.

**Prerequisites:** Phase 14 (storage backend abstractions and indexed reads). Independent of Phase 15.

**Motivation:** Phase 14 reduces the working set on the read side; Phase 16 closes the remaining gap on the operator side. Without it, queries that build large hash tables (joins on multi-million-row sources) or sort large result sets (`ORDER BY` over 10M+ rows) will OOM at operator time even though the backing store is happy. The gap is real but not user-blocking until the first OOM — typical workloads continue working after Phase 14.

### Phase 16 — Deliverables

- **Buffer pool over the storage backend:** memory-bounded byte-buffer pool that operators allocate from; pool spills to disk via the same RocksDB instance the layer store uses.
- **Hash join with spill:** partitioned hashing, spill to disk when partition exceeds memory budget, recursive partitioning if a single partition is still too large.
- **External sort:** classic external merge sort for `ORDER BY` and sort-merge joins on result sets larger than memory.
- **Group-by accumulator:** spillable group-by hash table with per-group state spilled to disk.
- **Cost-model awareness:** the EigenQL planner considers spill cost when ordering joins and choosing operators. Doesn't have to be sophisticated — a simple cardinality estimator + spill-aware cost is sufficient.
- **Memory budget configuration:** per-query memory budget (default and override), with a process-wide cap that prevents a single query from starving others.

### Phase 16 — Key design questions

- **Spill granularity:** per-operator spill files (clean per-operator lifecycle) or shared spill region (better space utilisation)?
- **Concurrent queries:** memory budget per query or per session? Buffer pool global or per-session?
- **Spill encoding:** CBOR (matches Eigon-JSON, debuggable) or a tighter format (smaller, faster)?
- **Garbage on cancel:** how do we ensure spilled files are cleaned up if a query is cancelled or a session crashes mid-query? Resume sweep covers tasks; queries need the same.

### Phase 16 — References

- `kernel/src/query/` — current evaluator (full rewrite of the operator layer)
- D2 — EigenQL specification (operators are unchanged at the surface; semantics preserved)
- D24 (to be written) — Out-of-Core Query Execution

---

## Phase 17 — Specialty Institutions

**Goal:** Proof-assistant and solver institutions extend the platform's reach. Lean 4 is the canonical first example. Others (SMT solvers like Z3, possibly Coq, possibly TLA+) follow the same pattern with their own fiber reasoners.

**Duration estimate:** open-ended per institution; each is roughly 2–4 weeks after prerequisites are in place.

**Prerequisites:** Phase 8 (WASM or subprocess hosting), Phase 11b (inductive types — Lean's claim to fame; an institution can't produce useful inductive results if Mini-TT can't represent them).

**Drives:** `docs/design/lean-4-as-institution.md` (existing sketch).

### Phase 17 — Deliverables (per institution)

- **Fiber declaration:** morphism types the institution understands (for Lean: proofs, reductions, elaborations), query types it answers (e.g., "is this proposition provable?").
- **Reasoner implementation:** WASM-hosted for Lean-in-WASM if feasible; subprocess-hosted via nanoda_lib integration as a practical first cut.
- **Comorphism specifications:** per Phase 11d/12, how Lean's natural numbers relate to Mini-TT's Nat, how Lean's Prop relates to Mini-TT's Type(0), etc. Many small comorphisms.
- **Worked example:** a life-science or engineering claim proved in Lean, with the proof registered as a verified morphism on a class.

### Phase 17 — Open questions (carried from `lean-4-as-institution.md`)

- `verified_in` witness extension (§16.4 / `lean-4-as-institution.md` open question 9) — deferred until a consumer requests it.
- Hosting model: WASM (sandboxed, matches other institutions) vs. subprocess (matches Lean's normal operating model, avoids wasm-lean complexity). Decision pending empirical data on both options.
- Trust policy for Lean proofs: how much of the Lean environment is "ambient trust" vs. reproducible per-proof?

### Phase 17 — References

- `docs/design/lean-4-as-institution.md` — extended sketch, nanoda_lib as reference
- `docs/design/life-science-requirements.md` §16.4 — verified_in witness extension
- D10 — institution protocol (the contract Lean plays)

---

## 9. Design Documents

The following design documents must be written and reviewed before the phase that depends on them. Each resolves open questions from the architecture document (§14) and makes decisions that code will implement.

| # | Document | Resolves | Required before | Estimated length |
|---|----------|----------|-----------------|-----------------|
| D1 | **Eigon Serialization Format** | **COMPLETED** — `docs/design/d1-eigon-serialization-format.md`. Eigon-JSON format, IRI identity, three-layer type system (data types/formats/content types), validation rules, canonical form, core ontology in `ontologies/core/core-ontology.json` | Phase 0 | Done |
| D2 | **EigenQL v1 Specification** | **COMPLETED** — `docs/design/d2-eigenql-specification.md`. Full EBNF grammar, lexer spec, type checking rules, aggregation (COUNT/SUM/AVG/MIN/MAX), GROUP BY, ORDER BY, LIMIT/OFFSET, DISTINCT, NOT EXISTS, dot-path navigation, error format | Phase 1 | Done |
| D3 | **Program Model and Component Interface** | **COMPLETED** — `docs/design/d3-program-model.md`. Programs as typed expressions (not programs), 12 expression forms mapping 1:1 to Mini-TT, Map/Reduce as language primitives, automatic parallelism from data dependencies, two-tier component model (built-in + WASM), ESL surface syntax (future) | Phase 2 | Done |
| D4 | **Storage Key Encoding** | **COMPLETED** — `docs/design/d4-storage-key-encoding.md`. Key encoding for RocksDB/TiKV, column families, layer chain persistence, index layout, TiKV compatibility | Phase 3 | Done |
| D5 | **gRPC API Specification** | **COMPLETED** — `docs/design/d5-grpc-api-specification.md`. RPC definitions, streaming query, context management, error codes, authentication, CLI/orchestration integration | Phase 3 | Done |
| D6 | **Execution Architecture and Durability** | **COMPLETED** — `docs/design/d6-execution-architecture.md`. Kernel↔orchestrator boundary, DAPR integration, durable workflows, activity dispatch, reasoning trace ownership, MCP server placement | Phase 4 | Done |
| D6b | **Reasoning Trace Schema** | **COMPLETED** — `docs/design/d6b-reasoning-trace-schema.md`. ComponentTrace, ProgramTrace, ObservationTrace, VerificationTrace classes. Provenance chain, epistemic status (observed→derived→verified), universe stratification, trace-based memoization | Phase 4 | Done |
| D7 | **ESL Surface Syntax** | **COMPLETED** — `docs/design/d7-esl-surface-syntax.md`. Two-layer design (HCL-style structural + ML-style expressions), namespace aliases, program/class/property/resource syntax, EBNF grammar | Phase 4.5 | Done |
| D8 | **CompleteJson Component** | **IMPLEMENTED** — `docs/design/d8-complete-json-component.md`. Structured LLM output via JSON Schema generated from ontology classes. Bijective short-name mapping with bijectivity check in ValidateProgram. Enums (`allows_only`), nested objects (`class_types`), union types (multiple `class_types` with `_type` discriminator). Template data type for prompt validation. `GetSchema` RPC. Patent demo end-to-end | Phase 7 | Done |
| D9 | **NbE/Executor Unification** | **COMPLETED** — `docs/design/d9-nbe-unification-and-type-extensions.md`. Capability modes (Pure/Read/IO), type theory extensions (Id, DecEq, NativeDecide, universes), complete ground type resolution, trace storage architecture, crash recovery, trace pruning (proofs-as-programs) | Phase 5 | Done |
| D10 | **Grothendieck Institution Protocol** | **SUPERSEDED by D14** — `docs/design/d10-grothendieck-institution-protocol.md` is a redirect to D14. Original D10 surface (FiberReasoner, InstitutionRegistry, ComorphismRegistry, validator-side morphism dispatch, FiberQuery/DiscoverMorphisms RPCs) retired in Phase 12. Categorical motivation (Eigon as shared signature category, Mini-TT as kernel service, Lean as verification institution) survives in D14. | Phase 6 (orig) / Phase 12 (D14 redo) | Done |
| D11 | **Codata, Streams, and Resumable Execution** | **COMPLETED** — `docs/design/d11-codata-streams.md`. Coinductive types via copatterns (Abel et al. 2013), stream semantics, tasks as codata, trace-driven replay, concurrent task model, ESL codata syntax, guardedness checking | Phase 9b | Done |
| D12 | **WASM Extensibility** | WASM module lifecycle, host function interface (kernel → WASM), resource serialization across the boundary (Eigon-CBOR), capability levels → WASM import sets (pure/read/IO), integration with `ComponentRegistry` and (under D14) the `Institution` trait via the `eigenius-institution-d14` WIT world, registration via ontology resources, fuel/memory limits, SDK crate design. Merges the previously separate D12 (Capability SDK) and D13 (Wire Format) — the interface and wire format are inseparable. Resolves §14 open question on capability protocol | Phase 8 | 14–18 pages |
| D13 | **Durable Kernel State** | **COMPLETED** — `docs/design/d13-durable-kernel-state.md`. `serve --db` flag, seeded bootstrap with drift-refusal, commit-through to `RocksStore`, WASM + institution re-registration on restart, persistent trace store via `BackendTraceStore`. Prerequisite for D11/Phase 9b. (The previous D13 — Wire Format — was merged into D12.) | Phase 9a | Done |
| D14 | **Institution Realisation** | **COMPLETED** — `docs/design/d14-institution-realisation.md`. Ontology-first replacement for D10's procedural institution surface. Triadic Comorphism (export_format, transformation, import_format, exact); InstitutionIndex derived from chain scan; InstitutionRuntime of Institution trait impls; Verdict-shaped QueryClasses with OnDemand / AutoOnLoad / Decidable dispatch roles; four-step comorphism pipeline; post-translation validation invariant. | Phase 12 | Done |
| D14b | **Security Model** | Authentication, authorization, namespace delegation policy, namespace delegation depth, capability trust chain and authenticity (resolves §6.4, §13.2, and §14 open questions). Originally slated for D14; renumbered when D14 was taken by Institution Realisation. | Phase 13 | 10–15 pages |
| D15 | **Ontology Versioning & Evolution** | Semantic versioning policy for ontology layers, backward compatibility rules, ontology combination semantics, ESL extension mechanism (resolves §13.1 and §14 open questions) | Phase 6+ | 8–10 pages |
| D16 | **Observability & Operational Tooling** | Structured metrics, tracing spans, query plan explanation, program execution step-through, reasoning trace streaming for live monitoring (resolves §13.3) | Phase 13 | 6–8 pages |
| D17 | **Capability Versioning** | How capability implementations are versioned, version mismatch handling, backward compatibility obligations, upgrade path for Foundation capabilities across kernel releases (resolves §14 open question) | Phase 8 | 6–8 pages |
| D18 | **Ontology-as-Types Resolution** | **COMPLETED** — `docs/design/d18-ontology-as-types-resolution.md`. `find_sigma_field` walks the layer chain through `resolve_class_type` instead of silently collapsing `EigonClass` to `Val::Set`. Introduces `CheckCtx` (bundling `rho`/`gamma`/optional layer/per-check type cache) threaded through all checker entrypoints. Adds inference-mode rules for `Construct`, `EigonResource`, `Template`, `IdJ`, `Refl`, `NativeDecide`, `DecEq`. Rejects no-layer EigonClass resolution explicitly rather than returning a weakened type. Closes the #12 high-priority correctness hazards. Prerequisite for most of D19. | Phase 10a | Done |
| D19 | **Inductive Types in Mini-TT** | **DRAFT** — `docs/design/d19-inductive-types.md`. Single (non-mutual, non-nested) strictly-positive inductive types + sized types (#16). Declaration form, positivity checker, recursor/eliminator derivation, iota-reduction, sized termination for inductive/coinductive interaction. Deferred: mutual (#20), nested (#21), indexed families (#22). | Phase 11b | Draft |
| D20 | **Layer Reconciliation via Comorphisms** | Category-theoretic treatment of layer merging. Common-ancestor invariants, comorphism witnesses as merge proofs, conflict resolution via the witness, the `migrate` and `db merge` commands. Supersedes D13's v1 drift-refusal. Builds on the DAG primitive that lands in Phase 14. Draws on D10's institution protocol and the `Comorphism` ontology class introduced in Phase 11d. | Phase 15 | 12–16 pages |
| D21 | **Task Traces and Checkpointing** | **COMPLETED** — `docs/design/d21-task-traces-and-checkpointing.md`. Per-task positional `(session_id, task_id, step_seq)` trace keys replace the Phase-9a content-address cache for IO components (determinism-gated — Pure/Read keep the memo for cross-task reuse). `components:Checkpoint` built-in persists program-declared state atomically via `write_batch`. `ListTasks` / `GetTaskStatus` / `CancelTask` RPCs + `at_layer` on read RPCs. Startup resume sweep (`ResumeConfig`, `ResumeState`) rehydrates pinned layer chains and re-executes `Running`/`Suspended` tasks with bounded parallelism. Single hardwired session (`Uuid::nil()`) in 9b-iii with multi-session as a Phase-14 surface expansion. | Phase 9b-iii | Done |
| D23 | **Out-of-Core Layer Architecture** | Topology / content split, per-head shadowing index, two-pool ARC cache, time-travel checkpoint scheme, DAG branching primitive, multi-session writes, reachability-based GC with trace pinning, branch pruning, indexed resource access for queries. The structural rework that lifts the kernel's working-set bound from "graph size" to "cache size." | Phase 14 | 12–16 pages |
| D24 | **Out-of-Core Query Execution** | Buffer-pool abstraction over the storage backend, hash join with spill, external sort, spillable group-by accumulators, spill-aware cost model, per-query memory budget. The operator-side rewrite that lets EigenQL handle result sets larger than memory. Builds on D23's storage abstractions. | Phase 16 | 8–10 pages |

**Reference documents** (analysis rather than specification):

| File | Purpose |
|------|---------|
| `docs/design/life-science-requirements.md` | Systematic audit of life-science representation needs. Drives the Phase 10–12 sequencing and is the source of record for which kernel extensions each shape requires. |
| `docs/design/lean-4-as-institution.md` | Extended sketch of Lean 4 as an Eigenius institution. Primary reference for nanoda_lib integration patterns and the `verified_in` witness discussion. Drives Phase 17. |

---

## 10. Test Strategy

### 10.1 Test Layers

**Unit tests** (per crate, `cargo test`): cover individual data structures, algorithms, and pure functions. Every module in the kernel has unit tests. Target: >90% line coverage on kernel code.

**Integration tests** (cross-crate): cover the interactions between kernel subsystems — layer commit triggers index construction, capability dispatch invokes the correct evaluator, query evaluation respects layer resolution order. Run with the in-memory storage backend for speed.

**Service tests** (end-to-end): start the kernel gRPC service (with in-memory or RocksDB backend), run the CLI against it, verify correct behavior. These are the primary regression tests from Phase 3 onward.

**Contract tests** (storage backends): a single test suite that runs against every storage backend implementation (in-memory, RocksDB, TiKV), verifying that they all satisfy the `LayerStore`/`CapabilityStore`/`BlobStore` trait contracts identically.

**Property-based tests** (proptest/quickcheck): for the type system (random well-typed terms type-check, random ill-typed terms are rejected), layer resolution (random layer stacks produce deterministic resolution), and serialization (round-trip property for all Eigon types).

**Performance benchmarks** (criterion): query latency at various resource counts (100, 1K, 10K, 100K), program type-checking time vs. program size, index construction time per layer. Run on every release; regressions block the release.

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
  ├── Contract tests against RocksDB
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
  program-validate <file>  Type-check a program
  run <file> [inputs]      Execute a validated program
  reflect <trace-file>     Record a reasoning trace
  inspect <iri>            Print a resource by IRI
  layer list               List layers in the current stack
  layer commit             Commit the working layer
  capability list          List registered capabilities
  capability test <id>     Test-invoke a capability
  config                   Manage CLI configuration (endpoint, credentials)
  version                  Print version and build info
```

**Modes:** `--endpoint <url>` connects to a remote kernel service (gRPC). `--local` runs an embedded kernel with RocksDB storage. Default: `--local` if no endpoint is configured.

**Output formats:** human-readable (default), `--json` for machine consumption, `--table` for tabular query results.

---

## 14. Dependency Summary

### Rust (kernel + CLI + storage)

| Crate | Purpose |
|-------|---------|
| `tonic` + `prost` | gRPC server and protobuf codegen |
| `tikv-client` | TiKV Rust client |
| `rocksdb` | RocksDB embedded storage backend |
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
| TiKV on Azure operational complexity | Delays Phase 3 deployment | Start with RocksDB in ContainerApps for staging. Defer TiKV to production deployment. |
| WASM capability interface design | Blocks Phase 5 extensibility | Write design doc D7 early (during Phase 3 or 4) to derisk. |
| Solo developer burnout on 6-phase plan | Stalls project | Each phase is independently valuable. The project is useful after Phase 1 (queryable knowledge graph). |

---

## 16. Beyond Phase 5 — Future Horizons

The following capabilities are described in the architecture but are deliberately excluded from the initial six-phase plan. They become relevant once the platform is stable and has real domain ontology usage.

**EigenQL recursive Datalog extension (§5.6).** ✓ Implemented in Phase 1. DEFINE rules with union semantics, seminaive fixpoint evaluation, and stratified negation.

**Constructive type theories as capabilities (§9.7).** Registering Lean 4, Coq/Rocq, or Agda proof kernels as capabilities — enabling the system to dispatch proof obligations to external theorem provers. This requires the WASM sandbox (Phase 5) plus a well-defined proof term interchange format.

**Browser and edge deployment (§2.6).** Compiling the kernel to WASM for browser-based developer tooling (ontology browsers, program editors) and edge deployment (Deno Deploy, Cloudflare Workers). Requires adapting the storage interface to IndexedDB/Deno KV and replacing gRPC with a browser-compatible transport.

**Distributed TiKV multi-region deployment.** The initial Azure deployment uses a single-region TiKV cluster. Cross-region replication, geo-aware layer placement, and consistency under partition require significant operational engineering.

**Ontology marketplace / registry.** A mechanism for publishing, discovering, and installing domain ontologies — analogous to a package registry. Requires the ontology combination semantics (§14 open question) and a trust/authenticity chain (§6.4).

---

*This plan is a living document. Phase boundaries may shift as design documents are written and early phases reveal unexpected complexity. The key invariant is that each phase produces a working, testable system.*
