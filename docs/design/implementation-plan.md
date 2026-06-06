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
│   │   ├── nbe/               # EigenTT type theory, NbE evaluator (§4.2–4.6)
│   │   ├── bootstrap/         # Foundation Layer loader, capability primer (§2.5)
│   │   └── api/               # gRPC service definitions (Load/Query/Validate/Reflect)
│   ├── tests/
│   └── Cargo.toml
├── storage/                   # Storage backend implementations
│   ├── tikv/                  # TiKV backend crate (§10.7)
│   ├── rocksdb/               # RocksDB embedded storage backend (incl. RocksTripleIndex per D23 §5.9)
│   └── memory/                # In-memory backend (testing)
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
| 2 | Programs | ✓ | Programs type-check and execute via EigenTT/NbE |
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
| D34 | Notebook Chain Workspace | ✓ | Rail-based workspace shell: header branch picker · Branches · History · Tags · Merge · Compaction · Tasks · Institutions · Topology · GC · Health. Tags as GC roots; anchored-commit cache invalidation on consolidation; per-layer byte accounting for GC reclaim estimates; cooperative read-pin from History → Topology drill-down |
| 12 | D14 Institutions | M1–M8 ✓; WASM-pkg / EigenQL-surface / docs / proto-cleanup pending | D14 institution realisation replaces D10; dock→assay worked example; comorphisms via four-step pipeline; Verdict-shaped Decidable + AutoOnLoad dispatch |
| 13 | Azure + Ops | | Production deployment, CI/CD, observability, TiKV option |
| 14 | Out-of-Core Layer | 14a–14h ✓; 14i (notebook surface + GC triggers) pending | Topology/content split, per-layer bloom + bloom cache, two-pool ARC, DAG branching, multi-session writes, reachability GC, indexed query path |
| 15 | Layer Reconciliation | | Six typed resolution strategies (Witness / Rename / KeepBoth / KeepOne / KeepNeither / Restructure); pushout-based merge; three-stage conflict taxonomy (schema / equation / instance); cascade impact analysis with user-ack gates |
| 16 | Out-of-Core Query Execution | | Buffer-pool over storage, hash-join with spill, external sort, spillable group-by, per-query memory budget |
| 17 | Chain Consolidation | ✓ | Squash a contiguous ancestral range into a resolve-equivalent layer; at-head + below-head modes; resolve redirects per D25 §12.8; preserve-history vs reclaim modes; GC reachability through redirects; bloom-cache eviction; cost-estimation dry-run. `db consolidate-summary` (diagnostic enumeration) tracked as a follow-up |
| 18 | Runtime Substrate | ✓ | `LanguageRuntime` trait + parent ontology; spawn-per-invocation Job lifecycle (`JobSpawner` / `DockerJobSpawner`); image-build pipeline via `buildah`; CBOR + RFC 8746 wire format; Julia hello-world capstone (18d); CBOR consolidation across kernel ↔ orchestrator (18e). Service lifecycle shipped under 19a alongside Julia |
| 19 | Julia Institutions | 19a ✓, 19d.0 ✓, 19d ✓, 19e ✓, 19f ✓ (HiGHS), 19f.1 ✓, 19g ✓, 19h ✓, 19h.1 ✓ | First concrete substrate instance; `eigon-julia-gen`; reference institutions: Symbolics/MTK, JuMP, IntervalArithmetic, Catalyst, DiffEq (ODEs); Catalyst → DiffEq + Symbolics → JuMP comorphisms live |
| 20 | Lean 4 Verification Institution | | Substrate-hosted authoring (`lean4export`, `LeanMirrorGenerator`, `LeanEnvironment`) + in-process verification (nanoda_lib); first `InProcess`-runtime-kind institution; first *verified*-tier institution. Two landings per D28 §11: 20a complete first institution, 20b Mathlib-scale operational |
| 21 | Life-Science Worked Examples | | I_Dock / I_ADMET / I_Assay / I_PK end-to-end via Julia institutions + comorphisms; EIG-0042 cross-fiber discrepancy notebook |

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
- Triple index construction on layer commit: per-layer POS index (D23 §5.9 / Phase 14h) populated by `LayerBuilder::build` and consulted by the query evaluator's `scan_chain` for indexed pattern matching
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

**Goal:** programs type-check against the EigenTT dependent type system and execute through the orchestration layer. Partial evaluation works.

**Duration estimate:** 6–8 weeks. **Completed:** April 12, 2026 (1 day, with Claude Code). Programs represented as typed expressions (not workflow graphs), EigenTT core ported from Haskell reference, program ontology with 54 resources, end-to-end pipeline: parse → type-check → execute.

### 5.1 Deliverables

- EigenTT implementation in Rust: Pi types, Sigma types, labeled sums, closures, environments, the `Val`/`Neut` value representation (§4.2). Ported from the Haskell reference implementation.
- NbE evaluator: `eval`, `readback`, `check`/`checkI` (bidirectional type checking), `eqNf` (equality by normalization) (§4.6).
- program type system: Components, Pipe, Parallel, Select, Map, Retry as typed constructs (§4.3–4.4). Type signatures with `Result<A, E>` for fallibility (§4.5). `NonDeterministic` marker.
- program validator: takes a program specification and a typing context (derived from the layer stack's class definitions), runs the NbE type checker, reports type errors with source locations.
- program validator registered as a Foundation Layer capability alongside EigenQL.
- Partial evaluation: given a program and a subset of its inputs, produce a typed residual program (§4.9). Neutral terms (`Nt Neut`) represent unknown inputs.
- Ground type resolution interface: the bridge between Eigon ontology types and EigenTT ground types (§4.10). Class references in program type signatures resolve against the layer stack.
- Orchestration layer (Deno/TypeScript): program execution engine that walks the validated program, executes Components (initially stub implementations), handles Pipe sequencing, Parallel fan-out, Select branching, Map iteration, Retry logic (§12.1–12.2).
- CLI `validate` command: takes a program specification file, validates it, prints type checking results.
- CLI `run` command: takes a validated program and input resources, executes it through the orchestration layer, prints output resources.

### 5.2 Key decisions required before coding

- program specification format: how are programs authored? ESL surface syntax? A YAML/JSON DSL? This needs a design document (see §8.2 of this plan).
- Component interface: what does a Component implementation look like from the orchestration layer's perspective? Input/output types, error handling, timeout contract.
- Orchestration ↔ kernel communication for Phase 2: before the gRPC service exists (Phase 3), the orchestration layer can embed the kernel as a Rust library via FFI or WASM. Recommendation: use the in-memory storage backend and call kernel functions directly via Deno FFI (`Deno.dlopen`) for Phase 2, then switch to gRPC in Phase 3.

### 5.3 Test plan

- **EigenTT core:** Port the EigenTT test suite from the Haskell implementation. Verify that well-typed terms check, ill-typed terms are rejected, and NbE produces expected normal forms.
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
- **Index correctness:** Load 1,000 resources, verify the per-layer POS index in RocksDB produces the same query results as the in-memory backend.
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

**Status:** Complete (April 2026). Shipped in four sub-milestones — 9b-i (EigenTT codata + guardedness), 9b-ii (ESL codata/corecord/observation + ontology), 9b-iii (task model: storage primitives, evaluator re-keying, RPCs, CLI, resume sweep), plus issue #16 filed for sized-types follow-up.

**Goal:** Extend the execution model with codata (coinductive types) so that data and event streams become first-class. Programs run as tracked tasks that persist across kernel restarts, with per-task positional trace keys for correct streaming and a startup resume sweep for crash recovery.

- EigenTT gains `Codata` / `CoRecord` / `Observe` terms, matching `Val` variants, eval + readback + type checker arms, and a syntactic guardedness check (Agda-style). See D11.
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

**Goal:** Close the fundamental kernel correctness gaps that block platform breadth — particularly ontology-as-types resolution, which is a prerequisite for the life-science representation work in Phase 11 and every other domain that typechecks against ontology classes. Framed in `docs/design/life-science-requirements.md` §19 step 1 as "nothing works cleanly until `Indexed inductive families` resolves EigonClass to proper dependent records."

**Duration estimate:** 3–5 weeks total, three internal milestones.

**Status:** Complete (April 2026). Ontology-as-types resolution (#18, D18), defence-in-depth for IO-reachable panics, and typed `EvalError` refactor (#19) all shipped. The NbE evaluator returns `Result<Val, EvalError>` throughout — 20 panic sites converted to error returns, all tests use `?` propagation. `catch_unwind` retained as defence-in-depth but the primary error path is now the Result monad.

**Prerequisites:** Phase 5 (EigenTT/NbE), Phase 8 (WASM).

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

**Status:** Deferred — enforcement at the layer-ingestion level is not yet wired. The EigenTT checker's universe rules are adequate for current use. Will revisit when Phase 12 worked examples exercise multi-level epistemic claims.

**Goal:** Enforce the three-level epistemic stratification (data → derivation → meta) at ingestion time so meta-level claims over traces are sound. Life-science §16.2 names this as unblocking §13 "Meta-level claims (sound)."

- Enforcement point: resource ingestion in the layer system (not EigenTT term forms). A resource at epistemic level N that references a resource at level ≥ N is rejected with a clear error.
- EigenTT checker: tighten universe-rule handling so attempts to construct self-referential meta-claims fail at check time (before runtime).
- Integration with D6b trace schema: a ProgramTrace at level 2 references only resources at level ≤ 1.
- nanoda_lib (see `d28-lean-4-as-institution.md`) as reference for how universe checking integrates with type equality — Eigenius's needs are simpler (three fixed levels vs. Lean's universe polymorphism).

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
- `docs/design/d9-nbe-unification-and-type-extensions.md` — EigenTT surface this extends
- `docs/design/d28-lean-4-as-institution.md` — nanoda_lib design references
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
- `docs/design/d28-lean-4-as-institution.md` — nanoda reference, especially Appendix A
- D19 (`docs/design/d19-inductive-types.md`) — Inductive Types + Sized Types in EigenTT

---

## Phase D22 — Notebook UX and TypeScript SDK ✓

**Goal:** Deliver a low-friction interactive surface on top of the kernel — a React single-page notebook served at `/notebooks/` and a typed `@eigenius/client` SDK consumable from any browser / Deno / Node runtime. Both are operational today and ship inside the orchestrator Docker image.

**Status:** Complete. Tracked outside the main 0–15 sequence because it's a parallel UX/SDK track rather than a kernel-capability phase; placed here in the document for chronological orientation. Internal sub-phases (per [D22](d22-notebook-and-typescript-sdk.md) §7):

- **Phase 1 — SDK foundation ✓** — `Eigen` class wraps Connect-RPC; `inspect`, `query`, `load`, `runProgram`, `runProgramByIri`, `layerTopology`, `publishNotebook`. Connect codegen wired; smoke test in `clients/eigenius-ts/examples/smoke-test.ts`.
- **Phase 2 — Static viewer ✓** — read-only notebook rendering with the four MVP cell types.
- **Phase 3 — Manual execution ✓** — Run / Run all / Reset, per-cell run states, output panels.
- **Phase 4 — Authoring (the MVP) ✓** — full editing UX, file Open / Save, the program-run cell, layer-stack and trace-tree visualisations, multi-stage `Dockerfile.orchestration` so the SPA serves alongside the RPC paths at `http://localhost:8080/notebooks/`.
- **Phase 5 — Visualisation ✓** — `@fluentui/react-charts` integration via TS-cell sandbox helpers (5a), `@xyflow/react` topology graph with per-layer drilldown (5b), and a dedicated form-based **chart cell** type covering grouped-bar / vertical-bar / horizontal-bar / donut / line / area kinds (5d). The `kinase-institutions` notebook exercises every chart kind plus the topology graph.
- **Phase 6 — Reactivity + polish ✓** — sticky header with Pin toggle, file IO renamed Import / Export, dedicated **Open published notebook** dialog backed by EigenQL search, dismissable header MessageBars, mandatory `notebook:title` (ontology + SDK + UI guards), per-cell collapse/expand + global Expand/Collapse all, edit-metadata dialog with description editing, on-demand resource fetch for the per-layer topology graph. Reactivity: `Run` per cell becomes a `SplitButton` with `Run` / `Run from here…` / `Run to here…`; subdued cell-order **stale** marker tracks `lastRunCellId` honestly without pretending to model TS-to-TS dataflow (the explicit DAG approach was rejected because it can't see kernel-layer side effects — see [eigenius#33](https://github.com/eigenius/eigenius/issues/33) for the proper EigenQL `OPTIONAL` follow-on).
- **Cross-cutting fixes during the polish round:** topology walker edge dedup on first sighting (was emitting each schema edge once per layer, blowing up the displayed edge count); chart titles wrapped externally for the cartesian + horizontal-bar + donut kinds for visual consistency (Fluent's `CartesianChart` `chartTitle` prop is aria-only).

### Phase D22 — Deliverables

- `clients/eigenius-ts/` — `@eigenius/client` package + `Eigen` class + content-addressed publish translator.
- `notebooks/` — Vite + React + TypeScript SPA. Cell types: `markdown`, `esl`, `eigenql`, `typescript`, `program-run`, `chart`. Auto-renderers for ResultSet, Resource, ProgramTrace, LayerStack, Topology, raw values.
- `ontologies/notebook/notebook-ontology.json` — `Notebook`, `Cell`, `CellType` classes; baked into the kernel as the 5th bootstrap layer so publish succeeds without first registering anything.
- `deploy/Dockerfile.orchestration` — multi-stage build that compiles the SPA and serves it from `EIGENIUS_NOTEBOOK_STATIC=/app/notebooks` at `/notebooks/*`.
- Two Playwright e2e tests: `patent-demo.spec.ts` (the LLM-free critical path through the patent demo) and `kinase-charts.spec.ts` (chart-cell regression coverage across all six kinds).

### Phase D22 — References

- [D22 — Notebook UX and TypeScript SDK](d22-notebook-and-typescript-sdk.md) — full spec including the Eigon-CBOR ↔ TypeScript marshalling rules
- [Platform guide chapter 14 — Notebook](../guides/platform/14-notebook.md)
- [Platform guide chapter 17 — TypeScript SDK](../guides/platform/17-typescript-sdk.md)

---

## Phase D34 — Notebook Chain Workspace ✓

**Goal:** Turn the existing notebook (D22) into a chain-aware workspace. The single-pane notebook view becomes one rail destination inside a Fluent `NavDrawer` shell; the other destinations expose every kernel surface a working operator needs (branches, history, tags, explicit merge, compaction, tasks, institutions, topology, GC, kernel health). The workspace metaphor — *what's in your chain right now*, browsable through one rail — replaces the prior "everything lives inside cell outputs" pattern.

**Status:** Complete. Tracked parallel to the main 0–15 sequence because, like D22, it's a UX/SDK track on top of kernel features that already shipped under Phase 14 (branches, GC), Phase 17 (consolidation), D33 (anchored-commit cache), and the existing institution / task surfaces. Eleven phases per [D34 §16](d34-notebook-chain-workspace.md#16-roll-out-order):

- **Phase 1 — §G.1 + §G.5 ✓** — `MergeInfo` wire-up; `branch_advanced` honesty fix in `advance_branch_for_layer` (the silent `NeedsWitnessedMerge` bug); cell-footer indicator (`◆ trivial merge` / `◐ cached at different position`).
- **Phase 2 — Header + branch picker ✓** — `BranchBar` with branch picker, tip indicator, unsaved-dot, read-pin indicator with "Return to tip".
- **Phase 3 — Branches panel + dialogs ✓** — list + create / delete dialogs; pin-aware delete safety; collapsible rail via `PanelLeftContract` / `PanelLeftExpand`.
- **Phase 4 — History panel ✓** — linear first-parent walk over `LayerTopology`; per-row glyphs (`●` / `◆` / `⬚`); detail panel with "Time-travel here" (read-pin), copy id, create tag, inspect resources.
- **Phase 5 — §G.3 + Merge UX ✓** — kernel: `MergeBranches` + `PreviewMerge` RPCs over `update_branch(AllowTrivial)` + a shared LCA/IRI-disjointness walk; `MergeInfo.orphan_layer_id`. Notebook: `MergePanel` rail destination + `WitnessedMergeRecoveryDialog` (save-as-sibling / pin-and-rebase / discard).
- **Phase 6 — Compaction wizard ✓** — three-step UI over the existing `ConsolidateChain` / `EstimateConsolidation` RPCs (range → estimate → run). Drove two structural kernel fixes uncovered while testing: (a) anchored-commit cache invalidation in `consolidate_chain_locked` per D33 §3 (`list_anchored_commits` → `delete_anchored_commit` for entries pointing into the consolidated range), and (b) `branch_contexts` cache invalidation on the consolidate RPC handler so subsequent reads see the post-consolidation chain instead of the stale in-memory head.
- **Phase 7 — Tasks panel ✓** — table over `ListTasks` / `GetTaskStatus` / `CancelTask`. Auto-poll only while at least one task is non-terminal; status badge colours by lifecycle category. Cooperative-cancel gap filed as [#51](https://github.com/eigenius/eigenius/issues/51) — the panel's `Cancel` button flips persisted status but the live evaluator doesn't yet observe it (D21 §8 cooperative cancellation deferred).
- **Phase 8 — §G.2 + Tags panel ✓** — kernel: new `tags` column family in both backends, four-method storage trait (`create_tag` immutable / `get_tag` / `delete_tag` idempotent / `list_tags`), three RPCs (`CreateTag` / `ListTags` / `DeleteTag`), `GcRoots.tag_targets` joins `branch_heads` + `task_pins` in reachability per D34 §8.3. Notebook: `TagsPanel` (mirrors `BranchesPanel` minus "switch to"), `CreateTagDialog` invoked from both the Tags panel header *and* the History detail panel's per-layer "Create tag" affordance.
- **Phase 9 — §G.4 + GC panel ✓** — kernel: `EstimateGc` / `RunGc` RPCs over a new `gc::estimate` (mark-only, no sweep) helper; `SweepStats.bytes_reclaimable` accumulator. Per-layer byte accounting: `LayerHandle.byte_size` (sum of encoded resource bytes, `#[serde(default)]` for legacy on-disk handles); both `store_layer` impls pre-serialize resources once to stamp the handle and reuse the bytes for the write batch. Notebook: `GcPanel` two-screen flow (estimate → confirm + run) with reclaimable-bytes row using a 1024-based formatter.
- **Phase 10 — §G.8 + Institutions inspector ✓** — `InstitutionInfo` enriched with `runtime_kind`, `requires_environment`, full `QueryClassDecl` and `ComorphismDecl` lists (source/target classes resolved by joining through the referenced `ExportFormat` / `ImportFormat`). New proto enums `RuntimeKind` and `DispatchRole`. Notebook: `InstitutionsPanel` with filterable list + detail card, "Inspect raw resource" via the existing `ResourceInspector` over the `Inspect` RPC.
- **Phase 11 — Topology + Health ✓** — `TopologyPanel` wraps the existing `TopologyGraphView` as a rail destination with a branch-tip picker, "Include resources" toggle, and read-pin honoring (History's "Inspect resources" sets the pin + navigates here; the panel disables the branch combobox and labels the title with the pinned hash). `HealthPanel` surfaces `eigen.health()` (healthy badge, version, layer + resource counts, D21 resume-sweep state with 3 s auto-poll while draining). Drove fixing the hardcoded `layer_count: 2 // core + program ontology` stub left over from the pre-Phase-14 era — now reads `backend.load_topology()?.layer_count()`.

### Phase D34 — Cross-cutting kernel additions

- **Branch-context cache coherence.** `consolidate_chain` advances the backend's branch ref via `put_branch` but bypasses `ExecutionContext`; the `branch_contexts` in-memory cache held a stale `Layer` graph and every subsequent read RPC (LayerTopology, RunProgram, Load candidate-parent walks) saw the old chain. `BranchContextCache::invalidate` exposed and called from the consolidate handler — fixes "topology view still shows pre-compaction layers" and "cell re-run cached at orphan layer id".
- **Anchored-commit cache invalidation on consolidation.** `gc::collect`'s bloom eviction always ran; the anchored-commit cache pointing at layers in the consolidated range was untouched. After CAS, `consolidate_chain_locked` now scans `list_anchored_commits()` and deletes entries whose `layer_id` is in `range_layers`. Per D33 §3.
- **Tag-rooted reachability in GC.** `GcRoots` extended with `tag_targets`; `from_branches` constructor now pulls both branch and tag refs into the root set so a tagged layer (and its ancestors) is durably protected until the tag is deleted. Per D34 §8.3.
- **Per-layer byte accounting.** `LayerHandle.byte_size` populated by both `store_layer` impls; `gc::estimate` / `collect` sum it into `SweepStats.bytes_reclaimable`; `EstimateGcResponse.reclaimable_bytes` no longer hardcoded to 0.

### Phase D34 — Deliverables

- `notebooks/src/components/workspace/` — `WorkspaceShell` rail + ten destination panels (`BranchesPanel`, `HistoryPanel`, `TagsPanel`, `MergePanel`, `CompactionPanel`, `TasksPanel`, `InstitutionsPanel`, `TopologyPanel`, `GcPanel`, `HealthPanel`).
- `notebooks/src/components/dialogs/` — `CreateBranchDialog`, `DeleteBranchDialog`, `CreateTagDialog`, `WitnessedMergeRecoveryDialog`.
- `notebooks/src/components/output/CommitStatusBadge.tsx` — cell-footer indicator for cached / trivial-merge / needs-witnessed-merge outcomes.
- `notebooks/src/runtime/commitMeta.ts` — `CommitMeta` + `classifyCommit` translating wire `MergeOutcome` into the renderer's `CommitStatus` taxonomy.
- Kernel: tag storage primitives + RPCs; `EstimateGc` / `RunGc`; `MergeBranches` / `PreviewMerge`; tag-rooted GC; cache-invalidation fixes for `consolidate_chain` (anchored-commit + `branch_contexts`); per-layer `byte_size` on `LayerHandle`. `InstitutionInfo` enriched per §G.8.
- SDK: `mergeBranches`, `previewMerge`, `consolidateChain`, `estimateConsolidation`, `createTag`, `listTags`, `deleteTag`, `estimateGc`, `runGc`, `listTasks`, `getTaskStatus`, `cancelTask`. Value-level enum re-exports for `MergeOutcome`, `ConsolidateErrorKind`, `RuntimeKind`, `DispatchRole`.

### Phase D34 — Deferred enrichments

Acknowledged in the design doc; gated on §G.6 (cursored branch-scoped history endpoint):

- Cross-branch topology overlay (active chain in colour, others grey, merge edges emphasised).
- "Installed at <layer>" lineage + "View install layer in history" cross-link in the Institutions inspector.
- §7.4 one-click "consolidate older half" affordance on the Branches panel (deferred until first feedback round).

### Phase D34 — Open follow-ups

- [#51](https://github.com/eigenius/eigenius/issues/51) — `CancelTask` doesn't propagate to the live evaluator; the persisted status flips to `Cancelling` but a Running task continues to completion.
- [#50](https://github.com/eigenius/eigenius/issues/50) — Pre-existing `DockerSpawner` TOCTOU race surfaced during the History timeline work; `#[ignore]`'d test guards the flake.
- `EstimateGcResponse.reclaimable_bytes` excludes per-layer bloom / topo / chain pointer / content-index / triple-index overhead — bounded per-layer overhead, dwarfed by resource bytes; a future enrichment that wants exact storage footprint would aggregate the full WriteBatch byte count at store time. Legacy on-disk `LayerHandle`s report `byte_size = 0` (via `#[serde(default)]`) until they churn through GC + recommit.

### Phase D34 — References

- [D34 — Notebook Chain Workspace](d34-notebook-chain-workspace.md) — full IA, rail layout, per-destination shapes, kernel gap list (§G.1–§G.8), 11-phase rollout (§16).
- [D33 — Partial-Order Chains §6](d33-partial-order-chains.md) — anchored-commit cache the consolidation invalidation fix is grounded in.

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

### Phase 13 — Forward implications: substrate Service / Job lifecycle

Phase 13 sets up the kernel + orchestrator deployment shape, but Phase 18+'s [runtime substrate](d26-runtime-substrate.md) introduces a second axis of cloud topology that this phase's tooling has to extend to cover:

- **`RuntimeEnvironment` with `lifecycle: Service`** maps to a **Container App service** with autoscaling rules (`minReplicas` / `maxReplicas`, HTTP / KEDA scale rules; serverless-equivalent cost is `minReplicas: 0` plus aggressive scale-to-zero). On a future k8s deployment this is a Deployment + Service with HPA / KEDA. Each Service env is a separate cloud resource sharing the kernel-orchestrator deployment's Container Apps environment / cluster.
- **`RuntimeEnvironment` with `lifecycle: Job`** maps to a **Container App Job** (manual or event-triggered execution) on Azure CA, or a k8s Job on a cluster. These are spawned per `RunRuntimeScript` dispatch and have no idle cost.

Implications for Phase 13's Bicep / GHA tooling that should be planned in advance even though they are realised when Phase 18+ workloads land:

- Bicep templates need a parameterised module for "register a runtime env as a Container App resource of the appropriate kind." A Service env produces a `Microsoft.App/containerApps` resource; a Job env produces `Microsoft.App/jobs`. The substrate's image-build pipeline (Phase 18c) writes the resulting `image_digest` into the env resource; Bicep consumes the digest to pin the image reference.
- GHA release workflow needs a "publish runtime env" step distinct from "deploy kernel + orchestrator": each new substrate-built image gets pushed to ACR and the corresponding CA service / job updated. CI must not couple runtime-env releases to kernel releases — they cadence differently (per-language updates land much more often than kernel updates).
- DAPR-side service discovery: the orchestrator needs to resolve Service-env workers by env IRI / image digest. On Azure CA this is per-resource hostnames inside the Container Apps environment; on k8s it's per-Service DNS. The substrate's `ServiceSpawner` backends (D26 §8.2 — `DockerServiceSpawner` for local DooD, `K8sDeploymentSpawner` later) read this discovery target from a deployment-supplied config. Phase 13 owns the config shape.
- Observability must distinguish kernel/orchestrator metrics from substrate-worker metrics. A Service env is a long-lived resource with steady-state replicas, request rate, latency histograms; a Job env has run-count, run-duration, success rate. Different scrape targets, different dashboards. Phase 13 lays the metric-naming convention; Phase 18+ populates the dashboards as workloads land.
- The Phase 13 deployment story is "kernel + orchestrator + DB"; the post-substrate story is "kernel + orchestrator + DB + N substrate runtime envs." Operational runbook should already use the language of "register a runtime env" so the post-Phase-18 ops surface is a natural extension.

This is a *forward note*, not Phase 13 deliverables. The substrate's deployment integration ships with Phase 18c (Job-side) and Phase 19a (Service-side) and reuses Phase 13's primitives. Capturing the implications here so the Bicep / GHA tooling is shaped to extend cleanly rather than to need a refactor on Phase 18+ landing.

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
- **14b — Per-layer shadowing bloom + bloom cache (~1.5 weeks).** Each layer carries a bloom filter over its `defined_iris`, computed at commit and persisted as `bloom:<layer_id>` (CBOR) in the same atomic write batch as the rest of the layer. Blooms page through a bounded `BloomCache` (mirrors `ResourceCache`'s shape). `Layer::resolve` walks the chain head→root using the cached blooms to skip non-defining layers, only probing cache/backend at layers the bloom flags. Per-layer (not per-head) keeps the kernel a pure DAG: no per-head index to maintain on commit/branch/merge; multi-parent merges (14e and Phase 15) just inherit. Trade-off: O(chain_depth × bloom_check) instead of O(1) per-head lookup; bloom checks are in-memory hash ops measured in tens of ns. See D23 §5.2 for rationale.
- **14c — Two-pool cache + eviction (~1 week).** Active-head pool (entries that are top-of-stack for the active head per the §5.2.2 chain walk) vs. historical pool (entries shadowed by a higher layer in every active head, only reachable via time-travel or trace dereferences). ARC inside each pool; historical pool evicted first under memory pressure.
- **14d — `commit_layer` + `update_branch` with CAS (~1.5 weeks).** The two stateless write primitives that anchor the lattice. `commit_layer(parent, content)` appends an immutable layer to the DAG; `update_branch(branch, expected_old, new_head)` advances a branch ref via CAS with a `FastForward | NeedsWitnessedMerge` outcome (trivial merge ships in 14e). Pin is just a parameter; no kernel-side `Session`, `PromotionService`, or scratch-chain abstraction — clients (CLI / notebook / task runner / SDK) orchestrate. D21's `TaskRecord.layer_head` is already the per-task pin and needs no structural change.
- **14e — Trivial merge in `update_branch` + branch read surface (~2 weeks).** Extend `update_branch` with the `TrivialMerge` outcome: when the caller's chain and the branch's current head modify disjoint sets of IRIs since their lowest common ancestor, the kernel produces a multi-parent merge layer automatically and CAS-updates the branch to point at it. Witnessed merges (real conflicts) still return `NeedsWitnessedMerge` for Phase 15 to handle. Also lands `BranchManager` read/list/delete surface, the `auto-*` naming convention for client-saved divergent chains, and a small additive `outcome: BranchUpdateOutcome` field on D21's `TaskRecord` so users can see whether a task fast-forwarded, trivially merged, or needs witnessed-merge resolution. The trivial-merge case handles the majority of real-world divergence in dev workflows; without it Phase 14 ships with a sharp usability cliff for any concurrent activity.

  Documented client convention shipped alongside 14e (in the task runner / SDK, not the kernel): **per-task recovery branch** named `recovery-{task_id}`. On task launch, the runner creates the recovery branch pinned to the launch-point layer; each task commit advances it via `update_branch(... StrictFastForward)`. On successful completion, the runner attempts to advance the *target* branch from the recovery branch's head and prunes the recovery branch on success. On failure (or `NeedsWitnessedMerge`), the recovery branch stays for inspection. The kernel doesn't model this — `TaskRecord` derives the recovery branch name from `task_id`, no new persisted field — but 14e is the right milestone to document the convention since this is the first phase where trivial merge makes the publish-step viable end-to-end. Read-only tasks skip the recovery-branch dance entirely.
- **14f — Reachability-based GC (~2 weeks).** Mark-and-sweep over the resource graph. Roots: pinned branch heads, active sessions, resources referenced by reflection-ontology traces, verified-knowledge claims. Background task with backpressure; configurable triggers (size threshold, idle interval).
- **14g — Branch pruning (~1 week).** `eigenius db prune <branch>` removes a branch from the topology; GC sweeps anything reachable only through it. Rejects pruning of branches with active sessions.
- **14h — Indexed resource access for queries (~1.5 weeks).** Implement the per-layer triple index (D23 §5.9) and wire it through the EigenQL evaluator's pattern-matching path. `MATCH ?x : Class { prop = ?v }` becomes a POS prefix scan against the storage backend instead of a full chain scan, with chain-membership filtering and a bloom-walk shadow check (matching §5.2's per-layer model) to dedupe across the DAG. POS-only in v1 (the three hot sites need only `(p, o) → s`); SPO/OPS deferred. IRI-valued objects only (`Property.data_type ∈ {resource, resource_array}`); literal-typed properties post-filter the index-narrowed candidate set. Result-set processing (joins, sorts, group-by) stays in memory — operator spill is Phase 16. After this lands, queries continue to work; the read-side working set just shifts off-heap. The previously-stubbed `storage/indexing/` crate is superseded by the new in-kernel implementation in `kernel/src/layer/index.rs` and the RocksDB-backed impl in `storage/rocksdb/src/triple_index.rs`.
- **14i — Notebook surface for branches & trivial merge + GC trigger wiring (~2 weeks).** *Pending.* 14a–14h shipped the kernel mechanics; this milestone closes the visibility gap so users see what the kernel is doing. Three independent strands:
  - **Notebook branch surface.** The notebook SPA renders the branch DAG (current head, divergent `auto-*` branches, recovery branches per task), exposes branch switching in the UI, and threads the chosen branch through `Run from here…` / `Run to here…`. Trivial-merge outcomes from Phase 14e's `update_branch` surface as labelled merge nodes on the topology graph and as MessageBar summaries when a task run trivially merges into the user's branch. `NeedsWitnessedMerge` outcomes display the conflicting IRIs and prepare the link into the (still-pending) Phase 15 witnessed-merge command.
  - **TypeScript SDK additions.** `@eigenius/client` gains `branches.list()`, `branches.head(name)`, `branches.subscribe(name)`, `tasks.outcome(taskId)` so the notebook can fetch the data above without ad-hoc EigenQL.
  - **GC trigger wiring.** Phase 14f's reachability GC implementation already exists; this milestone exposes the triggers as kernel config — size threshold (default a few GiB of unreachable bytes), idle interval (default 1h), manual `eigenius db gc` CLI command. Trigger evaluation runs as a low-priority background task with backpressure; `Health` reports `gc_in_progress` and last-sweep statistics. `eigenius db stats` extends with branch count, layer count, reachable / unreachable byte estimates, last GC sweep timestamp + reclaimed bytes.

### Phase 14 — Key design questions

- **Working-set bound:** fixed LRU size, adaptive (hit-rate-aware), or eviction-policy-as-config?
- **Trace pinning:** does an active reflection-ontology trace pin its referenced resources from GC? Default instinct: yes — the epistemic guarantee depends on the chain being readable. Implies traces have explicit lifetime / expiration policy.
- **Migration vs. shadowing:** when an ontology migration rewrites resources via comorphism (Phase 15), does the new resource shadow the old, or supersede it? Different GC semantics either way.
- **Branch identity:** content hash, user label, both? Answers propagate to Phase 15's merge command.
- **Bloom FPR vs. storage trade:** D23 §5.2 sets a 1% FPR default at commit time. Heavy-resolve workloads may want lower FPR (more storage, fewer spurious probes); negligible-resolve workloads may want higher FPR. Decide whether to expose this as a per-DB config knob or keep it baked at the kernel default.
- **Pathological chain depth:** the per-layer-bloom design's worst case is deep chains (10⁴+ layers) where bloom-walk dominates resolve cost. D23 §5.2.7 sketches an optional roll-up index as the mitigation; defer until a workload demonstrates need.

### Phase 14 — References

- D13 §8, §11 — drift-refusal and single-session boundary this phase generalises
- D21 §3.6 — `--at-layer` queries (time-travel surface; D23 §5.6 shows it reuses the standard `Layer::resolve` rooted at L)
- `kernel/src/layer/index.rs` — `TripleIndex` trait + `MemoryTripleIndex` + chain-walk helpers (Phase 14h)
- `storage/rocksdb/src/triple_index.rs` — RocksDB-backed `TripleIndex` (Phase 14h)
- `storage/indexing/` — pre-14h stub crate, superseded; can be removed
- D23 — Out-of-Core Layer Architecture

---

## Phase 15 — Layer Reconciliation

**Goal:** Close the conflict-resolution gap left by Phase 14e's trivial merge by giving the user a typed, kernel-checked menu of resolution strategies for divergent branches with overlapping contributions. Six strategies (Witness / Rename / KeepBoth / KeepOne / KeepNeither / Restructure) cover the conflict shapes seen in real workflows. Conflicts are surfaced through a three-stage taxonomy (schema / equation / instance); each resolution is well-typed at the kernel level and gated on explicit cascade-impact acknowledgment from the user.

**Duration estimate:** 8–12 weeks total, seven internal milestones aligned with [D20](d20-layer-reconciliation.md) §12.

**Prerequisites:** Phase 11d (Comorphism class — the typing discipline Phase 15's merge witnesses reuse), Phase 12 (D14 — `AutoOnLoad` QueryClass dispatch runs against merge layers per [D20](d20-layer-reconciliation.md) §7.2 step 5), Phase 14e (`update_branch` returning `NeedsWitnessedMerge` is the entry point Phase 15 replaces with the richer `NeedsResolution` outcome).

**Motivation:** Phase 14e ships trivial merge, which handles the ~80% of dev-workflow divergence with disjoint-IRI contributions. The remaining 20% — branches with overlapping contributions — is also where most regulated R&D workflows live (multiple humans evolving the same ontology in parallel). Pre-Phase-15 the only escape is to save the would-be-merged chain as an `auto-*` sibling branch; Phase 15 turns that residual case into "supply a typed resolution; the kernel produces a merge layer." The structural commitment is that **a layer merge is the pushout of a span of layer extensions**, computed first in the category of ontology presentations and then pointwise in Set-valued instances; every resolution is just a choice of transformation applied to the input span before pushing out. This keeps the kernel surface small (one pushout machine) while the policy surface scales to whatever real workflows need. Full theoretical framing in [D20](d20-layer-reconciliation.md) §4.

**Drives:** [D20 — Layer Reconciliation](d20-layer-reconciliation.md). Unblocks Phase 17 (Chain Consolidation; consolidating across merge nodes preserves resolution decisions encoded in the merge layer) and life-science worked examples in Phase 21 that involve multi-author ontology evolution.

### Phase 15 — Sub-milestones (per [D20 §12](d20-layer-reconciliation.md))

- **15a — Theoretical scaffolding (~2 weeks).** Pushout computation on finite category presentations (the schema-level pushout machine); Σ migration along inclusion functors (the data-level left pushforward); shared infrastructure for span-and-pushout that all subsequent strategies reuse. Hard prerequisite for 15b–15e.
- **15b — `Witness` resolution (~2 weeks).** The instance-level resolution: user supplies a `MergeComorphism` resource (typing discipline shared with the cross-institution Comorphism from Phase 11d / Phase 12; D20 owns the naming-distinction internally) whose transformation Component has signature `(branch_a, branch_b, Option<ancestor>) → merged`. Kernel applies the witness, runs `AutoOnLoad` validation against the merged resource, commits.
- **15c — `Rename` resolution (~2 weeks).** Apply a disambiguating isomorphism functor to one side before computing the pushout. Closed reference walk over the renamed branch's diff updates every reference to the old IRI; kernel rejects renames that collide with anything else in the chain or that break path equations. The cascade walker shipped here is reused by 15f.
- **15d — `KeepBoth` / `KeepOne` / `KeepNeither` (~2 weeks).** The three schema-level quotients. `KeepBoth` accepts the freely-combined pushout; `KeepOne` quotients out the loser; `KeepNeither` collapses both sides back to the ancestor. Kernel rejects strategies that don't apply to the conflict type (e.g., `KeepBoth` on a `PropertyDataType` conflict, since a property can't have two primitive types).
- **15e — `Restructure` resolution (~2 weeks).** The heaviest strategy: augment the ancestor with new objects/arrows before computing the span. User must supply explicit IRIs for new structure (kernel rejects synthesised parents like `urn:eigenius:auto:CommonParent_xyz`); kernel checks the augmented category type-checks and that subsumed `subclass_of` arrows are derivable through the new structure.
- **15f — Cascade impact analysis (~1.5 weeks).** Closed reference walk rooted at each conflict point, bounded by the chains being merged, producing a `CascadePreview` of `OrphanedReference` / `InvalidatedSignature` / `OrphanedTyping` / `InvalidatedTrace` items. Kernel-enforced acknowledgment gate: `submit_resolution` rejects with `IncompleteAcknowledgments` if any cascade item is unacknowledged. Sub-second synchronous on typical chains; D20 §11.4 leaves the door open for an async fallback if real workloads exceed the latency budget.
- **15g — Resolution UI surface (~1.5 weeks).** gRPC: `NeedsResolution` outcome on `update_branch` (replacing 14e's `NeedsWitnessedMerge` stub), `submit_resolution` with `MergeResolution` + `CascadeAck` lists, `preview_cascade` non-mutating endpoint. CLI: `eigenius db merge resolve` and `eigenius db merge preview` per D20 §7.4. The notebook surface that Phase 14i prepared (the `NeedsWitnessedMerge` MessageBar that links to the resolution UI) wires through to this milestone's RPCs.

### Phase 15 — Open questions (carried from [D20 §11](d20-layer-reconciliation.md))

- **Pre-declared `BranchMergePolicy` resources** for CI-style auto-resolution of known-safe conflicts; deferred to v2.
- **Auto-resolution declined for v1.** The kernel never picks a strategy on the user's behalf, even when "obvious"; in regulated workflows the user should always face the choice.
- **`conditional_requires` interaction.** Predicate-conditional requirements live outside the category proper; merges combining branches each satisfying their own conditional requirements may produce a merged dataset where the conjunction doesn't. Caught by `AutoOnLoad` re-validation; not surfaced as a typed conflict in advance.
- **Cascade-preview latency.** Synchronous v1 with hard timeout; async fallback (`cascade_preview_async` + polling) is the natural shape if real workloads exceed budget.
- **Reconciliation of `auto-*` branches accumulated pre-Phase-15.** Phase 15 unblocks resolving them through the new surface; a `eigenius db divergence resolve <branch>` action is on the CLI roadmap.
- **Multi-way merges** (more than two branches in a single resolution); pairwise composition probably suffices for v1.
- **Witness composition.** Whether two sequential merges with different witnesses are associative; likely yes for the comorphism shape, not formally established.
- **Equation-closure performance** on real-world ontology sizes when transitive subclass chains interact; may need an indexed equation-closure if naive walking is too slow.

### Phase 15 — Test plan

Per [D20 §10](d20-layer-reconciliation.md) — each milestone 15a–15g carries its own test surface (pushout-of-trivial-span; witnesses with mismatched signatures rejected; rename collisions and reference closure correctness; per-strategy applicability checks; user-supplied vs. synthesised parents in Restructure; cascade-ack enforcement; end-to-end resolution UI). Cross-cutting: composite conflicts resolved with mixed strategies; failure-path coverage for malformed strategies, missing acks, and `AutoOnLoad` validation failures; Phase 14 trivial-merge fast-path unchanged.

### Phase 15 — References

- [D20 — Layer Reconciliation](d20-layer-reconciliation.md) — full specification (the canonical reference for this phase's scope and shape)
- D14 — Institution Realisation (canonical institution surface; the cross-institution Comorphism shape Phase 15's merge witness reuses)
- D10 — Grothendieck institutions, comorphisms, the category-theoretic vocabulary (superseded by D14; retained as historical motivation)
- D13 §8, §11 — drift-refusal and single-session boundary; ontology migration is a degenerate single-resource case of a witnessed merge in Phase 15's framing
- D23 §5.4 — Phase 14 trivial merge surface this phase extends
- `docs/design/life-science-requirements.md` §11 (cross-institution claims) — downstream consumer of cross-fibre comorphisms

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
- D42 (stub) — Out-of-Core Query Execution

---

## Phase 17 — Chain Consolidation

**Goal:** Reshape sequential runs of layers into shorter equivalent ones, without losing the per-IRI top-of-stack semantics consumers depend on. Long-lived databases accumulate depth (many small commits per session, fine-grained edits during exploration), and Phase 14b's per-layer bloom design degrades on chains beyond ~10⁴ layers because resolve walks them all. Consolidation is the structural fix; the alternative — a periodically-materialised roll-up index alongside the per-layer blooms (D23 §5.2.7) — speeds up resolve but doesn't reduce storage cost or shorten chains.

This is distinct from merge (which combines parallel branches; doesn't reduce depth) and from GC (which removes unreachable layers; doesn't restructure reachable ones). Consolidation is the "git squash" analog at the typed-knowledge-graph level.

**Duration estimate:** 2–4 weeks (closer to 4 if trace re-pinning and consolidate-across-merge land in scope).

**Prerequisites:** Phase 14 (the DAG model and bloom-cache resolve are what consolidation operates on), Phase 15 (Layer Reconciliation — needed to consolidate across a merge node, since the consolidated layer must preserve any conflict-resolution decisions the merge encoded).

**Motivation:** Consider a notebook session that produces 200 small commits while iterating on a derivation. The session ends; the result is one effective state. With Phase 14, the chain stays 200 layers deep — every read pays a 200-layer bloom walk plus storage holds 200 small `topo:`/`bloom:`/`layer:` entries. Consolidation collapses the run into one layer that's resolve-equivalent to the topmost: same per-IRI top-of-stack values, same parent pointer (the one before the consolidated range began). The 199 collapsed layers become collectable by GC at the next pass.

### Phase 17 — Deliverables

- **`consolidate_chain(from: LayerId, to: LayerId) -> Result<LayerId, ConsolidateError>` API.** Takes a contiguous ancestral range `from..=to` (`from` must be an ancestor of `to`, no merge nodes in between for v1) and produces a single new layer with `parent = from.parent`, content = the range's top-of-stack values per IRI. Returns the new layer id; the caller updates whatever branch ref previously pointed at `to`.
- **Top-of-stack computation.** Walk `from..=to` head→root, materialising the merged view per the existing `iter_all_resources` semantics. The consolidated layer's `defined_iris` is the union of the range's `defined_iris`. Per-IRI value comes from the topmost layer in the range that defines it.
- **Trace re-pinning policy.** Traces (D21) pin specific `(LayerId, Iri)` pairs. When a pinned layer is consolidated away, the pin needs to either (a) re-point at the consolidated layer if the IRI's value is preserved, (b) be invalidated, or (c) keep the pinned layer alive (effectively blocking consolidation for traced layers). Default for v1: option (c) — refuse to consolidate ranges that contain trace-pinned resources. Less aggressive but safer; (a)/(b) are post-v1 refinements.
- **Atomic commit.** The consolidation writes the new layer (`topo:`, `bloom:`, `layer:<id>:res:*`, chain pointer) in one `WriteBatch` (per D23 §6.3 — the same atomicity contract every commit honors). The old layers stay in place until GC sweeps them; consolidation does not delete.
- **`eigenius db consolidate <from>..<to>` CLI.** Operator surface for triggering consolidation of an ancestral range.
- **Bloom cache eviction for consolidated-out layers.** Whatever entries the bloom cache held for the now-collected range are dropped via `evict_layer`. Same hook as GC.

### Phase 17 — Key design questions

- **Consolidate across merge nodes?** v1 says no (linear ancestral range only). Multi-parent consolidation needs to decide what the consolidated layer's parents are — the merge node's parents? The lowest common ancestor? This is its own design discussion.
- **Consolidation interaction with merge resolution.** A consolidated layer that absorbs a merge layer needs to carry forward whatever resolution decision (Phase 15 strategy + any `MergeComorphism` witness) the merge used. The consolidated representation is "as if the conflict had been resolved this way from the start" — the resolution metadata is still required to justify the merged content. Per the prerequisite on Phase 15.
- **Trace pin policy.** Refuse-to-consolidate (v1) is conservative. Re-pin-on-consolidate is more aggressive but requires the trace store to know about consolidation events.
- **Should consolidation be automatic?** Some systems consolidate background (Postgres VACUUM analog), others require explicit user action (git rebase). v1 ships explicit-only; auto-consolidation policies are post-v1.

### Phase 17 — References

- D23 §5.2.7 — the deferred "deep chain" performance concern that motivates this phase
- D21 — Task Traces and Checkpointing (trace pin re-pointing semantics)
- Phase 15 — Layer Reconciliation (resolution decisions preserved across consolidation)
- [D25 — Chain Consolidation](d25-chain-consolidation.md) — full specification (canonical reference for this phase's scope)

---

## Phase 18 — Runtime Substrate

**Goal:** Stand up the language-agnostic substrate that hosts external language toolchains inside Eigenius with full provenance. Pinned `RuntimeEnvironment` images, content-addressed `RuntimeScript` and `RuntimePackage` resources, the `LanguageRuntime` trait that per-language crates implement, the `RunRuntimeScript` / `CallRuntimeMethod` substrate components. No language implementations yet — this phase delivers the trait + the plumbing. Julia (Phase 19) and Lean's authoring side (Phase 20) layer on top.

**Duration estimate:** 8.5–10.5 weeks total, five internal milestones — 18a-c aligned with [D26](d26-runtime-substrate.md) §13, 18d is the closing capstone (a substrate-built image extending an upstream Julia base, ~1.5 weeks), 18e is a cross-cutting codec consolidation that the substrate work makes timely.

**Prerequisites:** Phase 8 (WASM as the contrast point — the substrate is a sibling, not a replacement, for fine-grained untrusted capability hosting), Phase 9a (durable kernel state — substrate resources persist across restart), Phase 12 (D14 — substrate components dispatch through the existing `ComponentExecutor`; per-language institutions surface as D14 institutions in Phase 19+).

**Drives:** [D26 — Runtime Substrate](d26-runtime-substrate.md). Enables Phase 19 (Julia) and Phase 20 (Lean's authoring-side workflows). Phase 18 ships the Job lifecycle end-to-end (18a–18d); the Service lifecycle is forced by Julia's startup cost and lands with Phase 19a.

### Phase 18a — Substrate skeleton (~3 weeks)

- New crate `eigenius-runtime-substrate` (workspace member at `crates/runtime-substrate`) with the `LanguageRuntime` trait and parent ontology resource classes (`RuntimeScript`, `RuntimePackage`, `RuntimeEnvironment`, `RuntimePackageMirror`, `RuntimeInvocation`, `RuntimeMethodSignature`, `RuntimePackagePin`).
- Parent ontology JSON at `ontologies/runtime/runtime-substrate-ontology.json` declaring the seven parent classes plus the `DispatchedTo` morphism class. Loaded at bootstrap alongside the institution ontology.
- Worker RPC framing using CBOR with RFC 8746 typed-array tags (matching the rest of Eigenius's serialization), over a Unix domain socket. JSON-on-the-wire kept behind a `json-bootstrap` feature for first-language Phase A debugging only — not the production codec.
- `RunRuntimeScript` and `CallRuntimeMethod` substrate components registered through the existing `ComponentExecutor` plumbing; `IO`-tagged.
- `DispatchedTo` morphism class as a structural-metadata morphism on `RuntimeInvocation`.
- `WorkerSpawner` trait (`spawn`, `wait`, `kill`, `attach_uds`) as the substrate's container/process-lifecycle seam — see Phase 18c for backend impls. Ships in 18a so the seam is in place from day one. `LocalSpawner` (host subprocess, no container) lands here as the default for dev / CI / smoke tests; `DockerSpawner` is a stub that errors out at construction with `not yet implemented (Phase 18c)`.
- Orchestrator-side wiring: a new sibling napi addon (modeled on `orchestration/native`) hosts the substrate crate and exposes worker-management entry points to Deno; the new TS handlers register against the existing `ComponentRegistry`. No new gRPC RPCs in 18a.

### Phase 18b — Mirror anchoring + boundary check (~2 weeks)

- Boundary check (D26 §7.5): mirror resolution, input shape check, method-signature check on every `RunRuntimeScript` / `CallRuntimeMethod` dispatch.
- Mirror-anchor compositionality logic: a `RuntimePackageMirror` anchored to layer L₀ is valid for invocations against descendant layers iff the mirrored classes are byte-identical; otherwise the substrate rejects with `MirrorVersionMismatch`.
- Per-language mirror-generator integration points (the substrate hosts the generator; per-language crates supply the generator binary).
- Boundary check runs kernel-side (it reads chain ancestry and class definitions); the orchestrator-side substrate handler invokes it via a new `RuntimeSubstrate` gRPC service or by extending `ComponentExecutor` — decision to settle in-flight, not a structural lock-in.

### Phase 18c — Image-build pipeline + spawn-per-invocation + sandbox (~2 weeks)

- Deterministic image-build pipeline (D26 §9.2): compose Dockerfile from per-language fragments + shared base layers, materialise `included_packages` and the mirror archive into the build context, invoke `buildah` deterministically (build path is `buildah`-driven, never via the run-side container client), push to registry, capture digest.
  - The `ImageBuilder` trait abstracts the build tool. `BuildahImageBuilder` is the only impl in 18c; a `KanikoImageBuilder` is a reasonable second backend when CI environments where the substrate's build pipeline runs inside a container anyway (k8s-based CI, GitHub Actions runners) want kaniko's "build OCI from inside a container, push to registry" ergonomics. Both satisfy D26 §9.2's "daemonless, never via the run-side container client." Add when there's concrete demand; the trait makes it a contained change.
- In-image build provenance baked into `/etc/eigenius-runtime-env/`; worker bootstrap performs the start-time cross-check (D26 §9.3).
- `DockerSpawner` implementation of `WorkerSpawner` using Bollard against the host Docker daemon (DooD: `/var/run/docker.sock` mounted into the orchestrator container, sibling-container model). Production default on Linux deployments. `PodmanSpawner` and a k8s-aware backend deferred — the trait makes either a contained add-on later.
- DooD bind-mount discipline: the substrate writes per-invocation tempdirs and the runtime depot under a single, stable host path (e.g. `/var/lib/eigenius-runtime/`) bind-mounted into the orchestrator at the same location, so paths the substrate hands to workers are valid in both filesystems without translation. Enforced as a runtime assertion at `DockerSpawner` construction.
- Security-boundary acknowledgement: granting the orchestrator process access to `/var/run/docker.sock` is root-equivalent on the host; deployment docs and a startup log line treat this as a spec-level constraint, not a footnote. The orchestrator host is the substrate's security boundary — no multi-tenant co-tenancy, no untrusted RPC surfaces forwarded to it.
- **Spawn-per-invocation execution model — for `lifecycle: Job` envs only.** Each `RunRuntimeScript` against a Job env spawns a fresh container, runs the job, waits for completion or failure, surfaces the result, then the container exits (`auto_remove: true`). No pool accounting, no idle/health-check machinery for the Job side. The Docker daemon's image-layer cache provides image-pull caching for free — subsequent invocations against the same digest skip the network hop without us writing pool code. **`CallRuntimeMethod` and `lifecycle: Service` envs are out of scope for 18c**: the service-backed dispatcher (D26 §8.1, §8.2) is wrong-architecture under spawn-per-invocation and lands in 19a alongside the first language that needs it (Julia). Phase 18c ships only the Job-side machinery — `JobSpawner` trait, `LocalJobSpawner`, `DockerJobSpawner` stub.
- OS-level sandbox depth depends on the active spawner. **`DockerSpawner`** (Linux production): namespaces (mnt, pid, net, user) and cgroups v2 from Docker; `no-new-privileges:true` as free defense-in-depth; per-spawn seccomp profile via `WorkerSpec.seccomp_profile` when a language crate ships one (Docker's default applies otherwise); per-invocation tempdir + read-only depot bind via the DooD discipline. **`LocalSpawner`** (dev / CI): per-invocation tempdir only, no namespacing; orchestrator logs a warning once per process at first dispatch under LocalSpawner so it is never silently used in production.
  - **What's deliberately NOT in the substrate's default container config.** Per D26 §1.2 the substrate is provenance + dispatch for *trusted* language toolchains, not adversarial containment. So defaults stop where they start to impose ops cost without a trust-model justification: no `cap_drop: ALL` (would force every deployment to solve container-UID ↔ host-bind-mount-owner alignment for zero benefit on trusted code), no hand-curated seccomp allow-list, no AppArmor profile loading. Every one of these is reasonable as an opt-in `WorkerSpec` field if a future deployment scenario actually motivates adversarial containment — add the field then with a deliberate decision about UID alignment / image USER conventions, rather than wedging the workaround in now. Single-tenant orchestrator host per D26 §9.5 makes this the right shape until proven otherwise.
- `numerical_metadata` recording on `RuntimeInvocation` (BLAS lib, FMA flag, GPU determinism flags, host kernel).

### Phase 18d — End-to-end Julia hello-world capstone (~1.5 weeks)

The closing acceptance milestone for Phase 18 — the witness that ties every preceding sub-milestone together against a real interpreter. Stays out of Phase 19's full Julia-integration scope (no `eigon-julia-gen`, no mirror generator, no institutions, no `JuliaScript` subclass, no Service-lifecycle pool).

**Setup.** A test fixture provides a tiny `LanguageRuntime` impl with `language_id() = "julia"`. It produces a *substrate-built* image that **extends** an [official Julia image](https://hub.docker.com/_/julia) — the upstream digest is the base, the substrate's build pipeline composes a Dockerfile on top:

```dockerfile
FROM julia:1.10-bookworm@sha256:<pinned-upstream-digest>
COPY JuliaWorker.jl /opt/eigenius/
RUN julia -e 'using Pkg; Pkg.add("CBOR"); Pkg.precompile()'
COPY etc/eigenius-runtime-env/ /etc/eigenius-runtime-env/
CMD ["julia", "/opt/eigenius/JuliaWorker.jl"]
```

`JuliaWorker.jl` is a minimal Julia worker (~100 lines) that reads `EIGENIUS_TEST_WORKER_UDS`, binds a Unix socket, speaks the substrate's CBOR RPC (length-prefixed frames, the five verbs `health` / `instantiate` / `register_mirror` / `dispatch_method` / `evict`) using `CBOR.jl`, and on `dispatch_method` evaluates the supplied Julia source. It's the Job-side counterpart of what `eigenius-julia`'s production worker will be — Phase 19a inherits it as a starting point.

The build runs through 18c's pipeline: per-language fragments composed, `JuliaWorker.jl` materialised into the build context, in-image provenance baked into `/etc/eigenius-runtime-env/`, `buildah` produces a deterministic OCI image, the captured digest goes onto a `JuliaEnvironment` resource — *that* digest, not the upstream Julia digest, is what the runtime spawns against.

**Round-trip.** Commit the `JuliaEnvironment` resource (with the substrate-built digest) and a `RuntimeScript` whose source is a Julia one-liner (e.g. `uppercase(read(stdin, String))`). Dispatch `RunRuntimeScript` with a single input resource. The substrate:

1. Resolves script + environment from the chain.
2. Asks `DockerSpawner` to spawn the substrate-built image as a sibling container, with the per-invocation tempdir bind-mounted from the host depot path (§A9 / D26 §9.5).
3. Worker bootstrap inside the container reads `EIGENIUS_RUNTIME_ENV_DIGEST`, cross-checks against `/etc/eigenius-runtime-env/manifest-hash`, then accepts the UDS connection.
4. `JuliaWorker.jl` receives `dispatch_method` with the script source as the CBOR target, evaluates it, returns the output via CBOR.
5. Substrate captures the output, assembles a `RuntimeInvocation` with the substrate-built image digest echoed verbatim, commits everything to the chain.

**Acceptance criteria (as shipped — see Status block below for which were achieved vs deferred).**
- `RuntimeInvocation.image_digest` matches the **substrate-built** digest (not the upstream Julia digest). ✓
- In-image provenance files (`/etc/eigenius-runtime-env/{manifest-hash, mirror-iri, included-pkgs, built-at}`) are present and consistent with what the substrate stamped at build time. ✓
- Worker bootstrap cross-check fires: tampering with `EIGENIUS_RUNTIME_ENV_MANIFEST_HASH` at spawn time produces a worker exit with `EXIT_CODE_CROSS_CHECK_FAILURE` (78). ✓
- The output resource carries the expected transformed payload (`uppercase("phase 18d capstone") → "PHASE 18D CAPSTONE"`). ✓
- The container is removed after exit (`auto_remove`). ✓ (already covered by 18c.3 tests)
- A second invocation against the same environment skips the registry pull (Docker layer cache, observable via timing — first build ~150s, cached rebuild ~13s). ✓
- The substrate-built digest is deterministic — building the same `JuliaEnvironment` resource twice produces byte-identical OCI images. **DEFERRED to 19c.** Two clean Julia builds produce different digests because Julia's `.ji` precompile cache files embed UUIDs/paths that buildah's `--timestamp 0` (which normalises filesystem mtimes) cannot reach. 19c is already scoped for "deterministic two-stage Dockerfile, `Pkg.instantiate` + `Pkg.precompile` baked into the image build" — that's the right place to nail Julia-specific image determinism.
- ~~The script cannot write outside its tempdir (sandbox check; `SandboxViolation`).~~ **DROPPED.** Per D26 §1.2 the substrate is provenance + dispatch for trusted toolchains, not adversarial containment. 18c.4 explicitly stripped sandbox aspirations from the substrate's defaults; there is no `SandboxViolation` enforcement to test against. The criterion was inconsistent with the spec's stated non-goals.
- ~~Re-running the same `(script, environment, inputs)` against the same host yields a byte-identical output resource (deterministic-by-environment per D26 §8.4).~~ **NOT TESTED in 18d.** Trivially true for the capstone's `uppercase(...)` script (no RNG, no time, no hostname); a meaningful test of D26 §8.4 needs a realistic numerics workload and lands with 19a's first concrete Julia institution work, not 18d's e2e plumbing capstone.

**Status (closed).** Capstone shipped as documented above. Implementation lives in [`julia/runtime-worker/`](../../julia/runtime-worker/) (Project.toml + Manifest.toml + src/JuliaWorker.jl) and [`crates/runtime-substrate/src/test_runtime_julia.rs`](../../crates/runtime-substrate/src/test_runtime_julia.rs); integration tests at [`crates/runtime-substrate/tests/julia_capstone_integration.rs`](../../crates/runtime-substrate/tests/julia_capstone_integration.rs). Two acceptance criteria diverged from the original spec: image-build determinism deferred to 19c (Julia-specific precompile-determinism is its own substantive workstream), `SandboxViolation` criterion dropped (inconsistent with D26 §1.2 / 18c.4's non-sandbox posture). When 19a lands, `julia/runtime-worker/` becomes the seed of `eigenius-julia`'s production worker as planned.

**Why this shape.** Extending an upstream image rather than using one verbatim — or building from scratch — gives the capstone genuine end-to-end coverage: the substrate's build pipeline (18c.1), the worker bootstrap cross-check (18c.2), the DooD bind-mount discipline (18c.3), the spawn / RPC path all fire under one test. Using Julia rather than the `bash` smoke runtime ensures the substrate works against a real interpreter with non-trivial startup cost and an actual standard library — i.e. against the kind of runtime Phase 19 will host. The upstream digest as base keeps the capstone focused on substrate machinery rather than language-toolchain installation. When Phase 19a lands, `JuliaWorker.jl` is the seed of `eigenius-julia`'s production worker, and this capstone test continues to pass against the production code — making it the regression anchor between the two phases.

### Phase 18e — CBOR consolidation for kernel ↔ orchestrator (~1 week)

Cross-cutting codec change: replace Eigon-JSON with Eigon-CBOR on the `ComponentExecutor` gRPC path so kernel ↔ orchestrator traffic is uniform with the rest of Eigenius (worker RPC per D26 §8.1, persistence per D13 / D24, every CBOR-encoded resource on disk).

**Motivation.** Today the kernel-side dispatcher ([`kernel/src/program/remote.rs`](../../kernel/src/program/remote.rs)) serialises input/argument with `eigon_json::serialize_resource(..).to_string().into_bytes()` and tags `content_type = "application/eigon+json"`; the orchestrator side ([`orchestration/src/server/component_executor.ts`](../../orchestration/src/server/component_executor.ts)) `TextDecoder + JSON.parse`s it. Substrate-routed traffic then crosses *back* into CBOR for the worker RPC and *back again* into JSON for the response. End-to-end CBOR eliminates the JSON↔CBOR transitions, halves the codec surface area, and matches D26's "CBOR everywhere" framing.

**Starting point — the proto already anticipates CBOR.** [`proto/eigenius.proto`](../../proto/eigenius.proto) already carries `content_type` fields on the relevant messages — `LoadRequest`, `QueryResponse`, `ValidateProgramRequest`, `RunProgramRequest`, `ReflectRequest`, and `ComponentRequest`. `ComponentRequest.content_type` is even commented as `"application/cbor" or "application/eigon+json"`. The schema is ready; this milestone just flips the default codec on each path and lights up the corresponding decoder at the receiver. Begin work by inventorying every site that reads / writes one of these `content_type`-carrying fields — both in the kernel and in the orchestrator — and audit which ones already branch on the value vs. silently assume JSON.

**Scope.**
- **Kernel side.** `program/remote.rs` switches to `eigon_cbor::serialize_resource(..)` and `eigon_cbor::parse_resource(..)`. `ComponentRequest.content_type` becomes `application/eigon+cbor`. The kernel reads the orchestrator's response as CBOR.
- **Orchestrator side.** `component_executor.ts` uses a TS-side CBOR codec to decode `req.input` / `req.argument` into the JS-shaped objects handlers already receive, and re-encodes the handler's response as CBOR. The handler interface (`ComponentHandler`, `ComponentInput`, `ComponentOutput`) is unchanged — the codec change is internal to the executor. A TS-side CBOR library lands as a dep (candidates: `cbor-x`, `cborg`); selection is part of this milestone.
- **WASM IO components.** Internal bridge already speaks CBOR; the orchestrator-side host bridge can pass bytes through directly instead of JSON-stringify-then-CBOR-encode. Net simplification.
- **Backward compatibility.** Honour `content_type` per request: if the kernel writes `application/eigon+json`, the orchestrator falls back to the JSON path. Lets the change land in a single PR without a synchronised kernel/orchestrator deploy.
- **Tests.** Existing component-executor unit tests update to expect CBOR; add a regression test covering the JSON-fallback path so the compatibility shim is exercised. `RemoteComponent` end-to-end tests round-trip a Resource through the new codec.
- **Removal of the JSON path.** The fallback is staged for removal in a follow-up phase once all consumers (kernel, every per-language crate) have moved over. Deletion is *not* part of 18e — keeping the shim until consumers catch up is deliberate.

**Why now / why in Phase 18.** The substrate's worker RPC introduced CBOR to the orchestrator process, so the TS-side CBOR machinery is already a forced dependency once 18a-18d land. Consolidating the kernel ↔ orchestrator codec at the same time keeps the codec story coherent rather than mixing JSON and CBOR across closely-related code paths. It is *not* gated on substrate work — could land independently — but the timing is right.

**Out of scope.** Changes to the Eigon-JSON parser/serialiser themselves; changes to the proto schema beyond the `content_type` value; client-side breaking changes (kernel's TS bindings, if any, get the CBOR option but JSON stays available).

**Status (closed).** Shipped across commits `d962d87` (kernel `program/remote.rs` + orchestrator `component_executor.ts` switch + JSON fallback shim), `cea5ac8` (substrate facade boundary), and follow-up "close the 18e gaps" work. Final touches in the follow-up: `kernel_client.ts` flips `load` / `validateProgram` / `runProgram` / `reflect` to send `application/eigon+cbor`; `executeComponentRequest` extracted into a unit-testable function with codec-branching tests (`tests/component_executor_codec_test.ts`) covering CBOR happy path, JSON fallback, empty-content_type pre-18e behaviour, and the unknown-component error path; kernel `program/remote.rs` grows codec-contract tests pinning the serialiser/parser choices and the `application/eigon+cbor` content_type. CLI clients (`cli/src/main.rs`) deliberately left on JSON — they are operator-facing and outside the kernel↔orchestrator scope of 18e; can flip later if uniformity-everywhere becomes warranted. **Caveat for future:** the orchestrator's `kernel_client.ts` JSON-string→CBOR bridge uses cbor-x's default encoding, which does NOT wrap `Value::Json` payloads with `EIGENIUS_JSON_TAG` — invisible today since none of the four flipped RPCs carry json-typed properties; if a future caller needs to round-trip a json-typed property through these RPCs, extend the bridge to mirror the kernel's `eigon_cbor::value_to_cbor` Json branch.

### Phase 18 — Test plan

- Smoke language test: a `bash -c` test runtime feature-gated on `crates/runtime-substrate` (`test-runtime` feature) wraps a long-lived bash worker speaking the substrate's CBOR RPC, so the skeleton can be exercised end-to-end without dragging in a real interpreter. Round-trips a `RunRuntimeScript` invocation, produces a `RuntimeInvocation` with full provenance, demonstrates worker bootstrap cross-check fires on misconfiguration.
- Boundary-check coverage: missing mirror class, mirror anchored to non-ancestral layer, method-signature mismatch — all produce typed errors before reaching the worker.
- Spawner-variant matrix: each boundary-check / spawn-per-invocation / numerical-metadata test runs once against `LocalSpawner` (always) and once against `DockerSpawner` (gated on `--features docker-spawner` and a reachable Docker daemon; skipped in environments without one).
- Image determinism: building the same `RuntimeEnvironment` resource twice produces byte-identical OCI images (modulo build timestamp, which is normalised).
- DockerSpawner-specific: container `auto_remove` actually removes; bind-mount paths visible inside the container match what the substrate expected (DooD path-translation regression); custom seccomp profile blocks a known-disallowed syscall; capability drop is in effect; `EIGENIUS_RUNTIME_ENV_DIGEST` cross-check refuses a tampered image.
- Sandbox isolation: a script attempting to access disallowed paths or syscalls is killed; resource-limit violations surface as structured errors.

### Phase 18 — References

- [D26 — Runtime Substrate](d26-runtime-substrate.md) — full specification
- D12 — WASM extensibility (the contrast: WASM for fine-grained untrusted, substrate for trusted-but-tracked language toolchains)
- D14 — institution realisation (per-language crates layer institution declarations on top of substrate components)

---

## Phase 19 — Julia Substrate Instance + Reference Institutions

**Goal:** Bring up Julia as the first concrete substrate instance, ship `eigon-julia-gen` as a deterministic mirror generator, and register five reference institutions under D14: `Symbolics`/`ModelingToolkit` (symbolic algebra), `JuMP` (optimisation), `IntervalArithmetic` (rigorous bounds), `Catalyst` (chemical reaction networks), `DiffEq` (ODE solving). Provides the computational footing for life-science worked examples in Phase 21.

**Duration estimate:** 18–22 weeks total, eight internal milestones aligned with [D27](d27-julia-institutions.md) §8.

**Prerequisites:** Phase 11b (inductive types — `Verdict`-shaped payloads and inductive-shaped term representations need them), Phase 12 (D14 — each Julia institution is a D14 institution), Phase 18 (runtime substrate).

**Drives:** [D27 — Julia Institutions](d27-julia-institutions.md). Enables [`life-science-requirements.md`](life-science-requirements.md) worked examples in Phase 21 (PK ODEs, ML ensemble bounds, certified intervals, reaction-network dynamics).

### Phase 19a — Julia substrate POC + mirror generator (~6 weeks) ✓

The first production-shape milestone for Julia. Combines what earlier drafts split as 19a (substrate POC) and 19b (mirror generator) into a single phase, because the worker's dispatch contract is shaped by whether mirrors exist — separating them forces the worker-side dispatch logic to be written twice (once for raw-dict payloads, once for typed mirror struct payloads) and the interdependency is tight enough that one combined milestone is cleaner.

Stands up the `eigenius-julia` crate, lights up the Service-backed dispatcher (deferred from Phase 18c), implements the mirror generator as substrate Rust code (D27 §3), wires `CallRuntimeMethod` end-to-end with typed mirror struct dispatch, populates `dispatched_to` on `RuntimeInvocation` (also 18c-deferred), and ships a minimal config primitive that future tunables extend. Deployment shape (c) for the entire phase — Julia bundled in the orchestrator image; the renumbered 19b (formerly 19c) flips to shape (a) once the per-env image-build path is exercised.

Sub-milestones below. Total estimate ≈ 30 working days. The kinase ontology classes (`Compound`, `Target`, `AssayProtocol`, `AssayResult` from `ontologies/examples/kinase/`) are the test data that grounds every sub-milestone's worked example.

#### Phase 19a.1 — `eigenius-julia` crate skeleton + LanguageRuntime impl (~2 days) ✓

**Goal.** Promote the test-only `crates/runtime-substrate/src/test_runtime_julia.rs` into a real production crate. Capstone-equivalent functionality continues to pass.

**No mirror artifacts in 19a.1.** The mirror generator lands in 19a.3 of this same phase. 19a.1 ships only the crate skeleton + `RunRuntimeScript` regression coverage; `call_method` returns `Err(NotImplemented)` until 19a.4 wires it up against generator-produced mirrors.

**Files created.**
- `crates/eigenius-julia/Cargo.toml` — new workspace member.
- `crates/eigenius-julia/src/lib.rs` — public surface (re-exports `JuliaLanguageRuntime`).
- `crates/eigenius-julia/src/runtime.rs` — `JuliaLanguageRuntime` struct implementing `LanguageRuntime`.
- `crates/eigenius-julia/src/dockerfile.rs` — Dockerfile-fragment provider (Julia install, worker copy, env-var stamping).
- `crates/eigenius-julia/src/conventions.rs` — shared constants (manifest-hash file path, env-var names) so Rust + Julia sides don't drift.
- `crates/eigenius-julia/tests/round_trip_test.rs` — capstone-equivalent round trip.

**Files modified.**
- Workspace `Cargo.toml` — add `crates/eigenius-julia` member.
- `crates/runtime-substrate/src/test_runtime_julia.rs` — collapse to a thin re-export of the production type, or delete entirely if `eigenius-julia` is reachable from substrate tests via dev-dep.
- `crates/runtime-substrate/tests/julia_capstone_integration.rs` — point at the production crate.

**Tasks.**
1. Crate scaffold (`Cargo.toml`, `lib.rs`, license header).
2. `JuliaLanguageRuntime`: `language_id() = "julia"`, `dockerfile_fragments` returns Julia-specific fragments, `build_environment_image` delegates to the 18c.1 builder with those fragments, `spawn_worker` initially uses 18c's `LocalSpawner` / `DockerSpawner` (per-invocation; the Service path lands in 19a.2), `run_script` mirrors the capstone, `call_method` returns `Err(NotImplemented)` (lights up in 19a.3), `query_health` already shipped on `JuliaWorker.jl`.
3. Update the 18d capstone integration test to point at the production crate; confirm green.
4. Document in the crate's `lib.rs` docstring that 19a operates without a mirror package — payloads are raw IRI-keyed dicts; 19b's mirror generator changes that.

**Acceptance.**
- `cargo build --workspace` clean.
- `cargo test -p eigenius-julia` passes (round-trip test against `RunRuntimeScript`).
- 18d capstone integration test passes against the production crate (regression anchor).

---

#### Phase 19a.2 — `ServiceSpawner` trait + Local/Docker backends (~4 days) ✓

**Goal.** Per [D26 §8.2](d26-runtime-substrate.md), introduce long-lived per-environment workers. Two dev-side backends — host subprocess and DooD-launched persistent container. `eigenius-julia` switches to using the Service path; per-invocation spawn stays available for the bash test runtime and 18d's existing tests.

**Pooling deferred.** Production-target backends (Azure Container Apps, Kubernetes) handle scaling, max-replica enforcement, idle eviction, and liveness/readiness probing at the platform level (HPA / KEDA / ACA scale rules). A substrate-side pool would duplicate and potentially conflict with the platform's scaling decisions. Local subprocess and Docker backends are dev-only; their concurrent dispatch story is "one long-lived worker per env, dispatches share it" — sufficient for dev usage without a pool layer. The `ServiceSpawner` trait shape (`ensure_service` / `attach_uds` / `drain` / `backend`) generalises cleanly to future K8s and ACA spawners — those don't lease/release, they ensure-and-route.

**Files created.**
- `crates/runtime-substrate/src/spawner/service/mod.rs` — `ServiceSpawner` trait + `ServiceHandle` type.
- `crates/runtime-substrate/src/spawner/service/local.rs` — `LocalServiceSpawner` (long-lived host subprocess, UDS RPC).
- `crates/runtime-substrate/src/spawner/service/docker.rs` — `DockerServiceSpawner` (Bollard, persistent container per `(env_iri, image_digest)`, `auto_remove: false`).
- `crates/runtime-substrate/tests/service_spawner_test.rs` — backend matrix (Local always, Docker gated on `--features docker-spawner` + reachable daemon).

**Files modified.**
- `crates/runtime-substrate/src/spawner/mod.rs` — expose `service` submodule.
- `crates/runtime-substrate/src/lib.rs` — re-exports.
- `crates/runtime-substrate/src/facade.rs` — Service-backed dispatch path; on dispatch, `ensure_service` then `attach_uds`, send the request.
- `crates/eigenius-julia/src/runtime.rs` — `spawn_worker` uses `ServiceSpawner`; per-invocation `DockerSpawner` behaviour preserved as a fallback.

**Tasks.**
1. `ServiceSpawner` trait: `ensure_service(spec) -> Result<ServiceHandle, SpawnError>` (idempotent — same env returns same handle), `attach_uds(service) -> Result<UnixStream, SpawnError>`, `drain(service) -> Result<(), SpawnError>`, `backend() -> &'static str`. No leasing, no health-check, no max-size — those are the platform's concern (or the worker's, for concurrent dispatch on one process).
2. `LocalServiceSpawner`: `std::process::Command` to spawn the worker; UDS bound to a per-service tempdir; `Request::Evict` on drain followed by SIGKILL on timeout. Cross-check against `EIGENIUS_RUNTIME_ENV_MANIFEST_HASH` happens in the worker as today.
3. `DockerServiceSpawner`: Bollard, `auto_remove: false` (vs the per-invocation `DockerSpawner`'s `auto_remove: true`). Container persists across invocations; one container per `(env_iri, image_digest)`. UDS bind-mounted per 18c.3 DooD discipline. `drain` calls `docker stop` + `docker rm` (or Bollard equivalents).
4. `eigenius-julia` rewires `spawn_worker` to call `ServiceSpawner::ensure_service` and return a connection-attaching handle. `run_script` / `call_method` become "ensure_service, attach_uds, dispatch" instead of "spawn, dispatch, terminate".
5. The Job-style `WorkerSpawner` trait stays in place for the bash test runtime + 18d's existing tests — they're explicitly per-invocation, no migration needed.

**Acceptance.**
- Both backends pass a smoke test: `ensure_service` → `attach_uds` → dispatch a Health RPC → `drain`.
- Cold-start vs warm-reuse timing measurable (Julia first call ≥ several seconds; subsequent calls against the same service handle sub-100ms — no pool needed because the worker stays alive).
- Docker-backed service keeps the container alive across invocations until `drain` (verify via `docker ps`).
- 18d capstone integration test still passes (regression — the bash + Julia per-invocation paths stay green via `WorkerSpawner`).

---

#### Phase 19a.3 — Mirror generator (~10 days) ✓

**Goal.** Implement the mirror generator as substrate Rust code per [D27 §3](d27-julia-institutions.md). Walks the chain's ontology layer at image-build time, emits Julia source matching the [D29 faithful-translation specification](d29-eigon-julia-mirror-spec.md), commits the result as a `RuntimePackageMirror` resource, bakes the precompiled artifact into the env image. D29 v1 lands in 19a.3.c alongside the chain-commit work; the implementation is reconciled to it in 19a.3.d (subclass hierarchy + remaining gap items called out in [D29 §11.2](d29-eigon-julia-mirror-spec.md#112-planned-extensions)).

**Files created.**
- `crates/eigenius-julia/src/mirror_gen.rs` — generator entry point: `generate_mirror(layer: &Layer, classes: &[Iri]) -> Result<MirrorOutput, GenError>` returning `(julia_source, content_hash, mirrored_class_iris)`.
- `crates/eigenius-julia/src/mirror_gen/struct_emitter.rs` — emits Julia struct definitions from class declarations.
- `crates/eigenius-julia/src/mirror_gen/codec_emitter.rs` — emits `decode_*` / `encode_*` per class for IRI-keyed CBOR round-trip.
- `crates/eigenius-julia/src/mirror_gen/validator_emitter.rs` — emits constructor-level format-constraint validation (`min_value`, `max_value`, `pattern`, etc.) per the faithful-translation spec.
- `crates/eigenius-julia/tests/mirror_gen_test.rs` — golden-file tests against the kinase ontology.
- `julia/common/EigeniusJuliaCommon/Project.toml` — shared package providing helpers the generated code calls (`validate_min`, `validate_pattern`, `decode_iri_keyed_map`, etc.).
- `julia/common/EigeniusJuliaCommon/src/EigeniusJuliaCommon.jl`.
- `julia/env/Project.toml` — the shared env's Pkg env; declares path-deps on `EigeniusJuliaCommon` and (at build time) on the generated mirror packages.
- [`docs/design/d29-eigon-julia-mirror-spec.md`](d29-eigon-julia-mirror-spec.md) — faithful-translation specification (D29). v1 draft committed in 19a.3.c.

**Files modified.**
- `crates/eigenius-julia/src/runtime.rs` — `build_environment_image` invokes `generate_mirror` for the env's mirror class list, copies the generated source into the build context, commits the `JuliaPackageMirror` resource alongside the image digest.
- `crates/eigenius-julia/src/dockerfile.rs` — Dockerfile fragments grow a `COPY mirror/ /julia-src/mirrors/<name>/` step plus precompile invocation.
- `julia/runtime-worker/src/JuliaWorker.jl` — worker bootstrap loads each mirror package via `using <PackageName>` based on the env's `mirror_dependency` list (passed via env var or config).
- The ontology / class-walking helpers in `crates/runtime-substrate/src/boundary.rs` may need refactoring for reuse by the generator (the boundary check already walks classes; the generator does similar walking).

**Tasks.**
1. Class-walking pass: from a layer + a list of class IRIs, transitively collect all reachable classes (via `requires` / `recommends` resource-typed property class_types) and topologically sort so structs can be emitted in dependency order.
2. Per-class emit pipeline: `class → (struct decl, decode fn, encode fn, constructor with validation)`. Each piece in its own emitter module so the spec stays readable.
3. Faithful-translation spec D29 — capture the full mapping table from D27 §3.3 plus edge cases (empty `recommends`, polymorphic `class_types`, format constraints by data type, value_arrays with element_type, nested embedded resources). Author this concurrently with the generator so the spec and the implementation co-evolve.
4. Generator self-tests:
   - Determinism: same `(layer_hash, class_iris)` input → byte-identical Julia source.
   - Idempotence: regenerating against an unchanged ontology produces the same `content_hash`.
   - Spec conformance: golden-file tests for the kinase ontology (the four classes + their properties → exact expected Julia source).
5. `JuliaPackageMirror` resource commit: at build time, the substrate creates a `JuliaPackageMirror` resource with `library_content` = generated source, `library_content_hash`, `mirrored_classes` = the class IRIs, `source_layer` = the head layer, `generator_identifier` = the substrate version, `generator_content_hash` = the substrate binary hash.
6. Image-build wiring: the Dockerfile composer pulls the generated mirror into the build context; `Pkg.precompile()` runs over `julia/env/` so the mirror is precompiled at build time, not at first dispatch.
7. `EigeniusJuliaCommon` shared helpers — kept minimal (only what the generated code needs).

**Acceptance.**
- Generator produces a valid `EigeniusKinaseMirror` Julia package from `ontologies/examples/kinase/kinase-ontology.json`.
- Generated package compiles + precompiles cleanly in the env image.
- Round-trip test: a `Compound` resource → CBOR → `decode_compound` → `encode_compound` → CBOR → equals original.
- Determinism: regenerating the kinase mirror twice produces byte-identical source.
- A `JuliaPackageMirror` resource is committed with the right content hash, source layer, and mirrored class IRIs on each `build_environment_image` call.

**Sub-milestones.** 19a.3 ships in four chunks: 19a.3.a (class-walking + struct emitter), 19a.3.b (codec emitters + validating constructors + `EigeniusJuliaCommon`), 19a.3.c (`RuntimePackageMirror` chain commit + image-build wiring + [D29 v1 draft](d29-eigon-julia-mirror-spec.md)), 19a.3.d (reconcile generator output to D29 — see below).

---

#### Phase 19a.3.d — D29 conformance pass (~3 days) ✓

**Goal.** Reconcile the v1 generator's actual output to [D29 v1](d29-eigon-julia-mirror-spec.md). 19a.3.a–c shipped the generator and the spec in parallel; 19a.3.d closes the gap items the spec calls out as bugs against the v1 implementation, plus implements the spec's required-but-not-yet-emitted features.

**In scope.**
- **Pattern anchoring** ([D29 §9.4](d29-eigon-julia-mirror-spec.md#94-validator-semantics-delegated-to-eigeniusjuliacommon)): `validate_pattern` wraps the user's pattern in `^(?:…)$` so Julia-side validation matches the kernel-side semantics in `kernel/src/validation/mod.rs:check_pattern`.
- **Polymorphic `class_types` as `Union`** ([D29 §4](d29-eigon-julia-mirror-spec.md#4-faithful-type-translation)): a property with multiple `class_types` produces a `Union{T₁, …, Tₙ}` field type (sorted by IRI), with helper-driven encode dispatch on `typeof` and decode dispatch on the input dict's `is_a` list ([D29 §8.3](d29-eigon-julia-mirror-spec.md#83-polymorphic-union-field-codecs)). Extends the kinase fixture with a polymorphic case.
- **Format IRI passthrough** ([D29 §9.3](d29-eigon-julia-mirror-spec.md#93-format-symbol-rendering)): non-`urn:eigenius:core:formats:` format IRIs are passed to `validate_format` as-is rather than silently dropped; `validate_format` raises on unknown formats.
- **Cycle detection** ([D29 §3.3](d29-eigon-julia-mirror-spec.md#33-topological-order)): closure containing class cycles raises `MirrorGeneratorError::UnrepresentableClass`. Currently the topological sort silently produces invalid Julia.
- **Subclass hierarchy emission** ([D29 §3.2](d29-eigon-julia-mirror-spec.md#32-subclass-closure-planned-112) / §11.2): `subclass_of` walked into the closure; abstract types emitted (`abstract type SuperType end`) before concrete structs; `struct Sub <: SuperType`. Bumps the spec to v1.1.
- **Generator self-tests** for each of the above + a snapshot test that the kinase-fixture output matches the post-fix shape.

**Out of scope** (pinned for later milestones in [D29 §11.2](d29-eigon-julia-mirror-spec.md#112-planned-extensions)).
- Multi-mirror per `RuntimeEnvironment` (lands in 19a.4 alongside `CallRuntimeMethod`).
- Per-class file split.
- `core:allows_only` enum support, embedded resources.
- Real generator binary content hash (replaces the `sha256("eigon-julia-gen:<version>")` v1 placeholder).

**Acceptance.**
- D29 v1.1 published; generator output conforms by every spec rule it cites.
- Existing kinase snapshot test updated; new snapshot covers a polymorphic field and a subclass relationship.
- Pattern-anchoring regression: `pattern: "abc"` rejects `"xxxabcxxx"` on both kernel and Julia validators.

---

#### Phase 19a.4 — `CallRuntimeMethod` + `JuliaWorker.jl` method dispatch + `dispatched_to` wiring (~5 days) ✓

**Status: substrate side landed (2026-05-04).** `target_kind` wire discriminator, `MethodInvocation` payload, JuliaWorker method dispatch with `Base.invokelatest`, `JuliaLanguageRuntime::call_method`, `DispatchTrace.dispatched_to` plumbing, substrate-side epistemic-category stamping (`urn:eigenius:reflection:DerivedResource`), and the codec registries (`_eigenius_decoders` / `_eigenius_encoders`) on each generated mirror are all in. Spec [D29 v1.2](d29-eigon-julia-mirror-spec.md#85-codec-registries) pins the registry shape and the world-age rule.

**Orchestrator-side glue and chain commit pipeline** are subsumed by Phase 19a.5 (D31 infrastructure) — they were originally framed as a "carry-over" sub-milestone but D31 settled the broader institution lifecycle, of which the orchestrator commit pipeline is one component. See [Phase 19a.5](#phase-19a5--d31-infrastructure-external-institution-lifecycle-3-weeks) below.

**Goal.** Light up `CallRuntimeMethod` end-to-end against the generator-produced mirror from 19a.3. Worker dispatches by `RuntimeMethodSignature` IRI, performs Julia multiple dispatch on typed mirror struct inputs, captures `which()` for `dispatched_to`. Substrate propagates `dispatched_to` through `DispatchTrace` to the orchestrator, which stamps it on the committed `RuntimeInvocation` (closing the 18c.5-deferred property).

**Files created.**
- `crates/runtime-substrate/src/components/call_method.rs` — kernel-registered `CallRuntimeMethod` Component (substrate-level; resolves a `RuntimeMethodSignature`, leases worker, dispatches).
- `julia/institutions/kinase-demo/EigeniusKinaseDemo/Project.toml` — small demo handler package depending on the generator-produced `EigeniusKinaseMirror`.
- `julia/institutions/kinase-demo/EigeniusKinaseDemo/src/EigeniusKinaseDemo.jl` — sample method handlers operating on the typed mirror structs (e.g. `compute_selectivity_index(c::Compound, t1::Target, t2::Target)::Float64` reading `c.compound_id` etc.). Used as the test fixture for 19a.4 and the 19a.8 e2e demo; not a real institution. (The first real institution lands in 19a.6 — IntervalArithmetic.)
- `crates/eigenius-julia/tests/call_method_test.rs` — kinase-grounded e2e.

**Files modified.**
- `julia/runtime-worker/src/JuliaWorker.jl` — `dispatch_method` evolves substantially:
  - Maintains a *method registry*: at boot, walks loaded mirror + handler modules' exports and registers `(method_name, parameter types)` entries against `RuntimeMethodSignature` IRIs.
  - On `dispatch_method`: decodes `target` as the `RuntimeMethodSignature` IRI; decodes `inputs` as CBOR-encoded mirror struct values via the per-class `decode_*` functions the generator emitted; looks up the handler; invokes it via Julia's multiple dispatch; captures `which(handler, typeof.(args))` for `dispatched_to`; encodes return value as CBOR via the per-class `encode_*` (or a primitive encoder).
  - Returns `Response::DispatchOk { invocation_id, output, dispatched_to: <which-string> }` (the previously-`nothing` field is now real).
  - Old eval-Julia-source behaviour preserved under a `target_kind = "script"` discriminator on the wire so the 18d capstone path stays green.
- `crates/runtime-substrate/src/language_runtime.rs` — `call_method` signature stable; the impl now does real work.
- `crates/runtime-substrate/src/invocation.rs` — `DispatchTrace.dispatched_to` field (likely already exists from 18c.5 stub).
- `crates/runtime-substrate/src/facade.rs` — propagate `dispatched_to` from the language-runtime call up to the trace.
- Orchestrator-side `JuliaInvocation` handler — accept `dispatched_to` from substrate, stamp on `RuntimeInvocation` resource.
- `crates/eigenius-julia/src/runtime.rs` — `call_method` implemented (was `Err(NotImplemented)` in 19a.1).
- `julia/env/Project.toml` — adds the `EigeniusKinaseDemo` handler package as a path-dep so it's part of the env image.

**Tasks.**
1. Settle the method-IRI scheme. Proposal: `urn:eigenius:julia:method:<package_iri>:<method_name>(<param_class_iri_1>,<param_class_iri_2>,…)`. The class IRIs are meaningful now that mirrors are typed; multi-method dispatch by class IRI works from day one.
2. `JuliaWorker.jl` registry: a `Dict{String, Function}` keyed on the method IRI; a `register_methods(mod)` function called at boot for each loaded handler module.
3. `dispatch_method` rewrite:
   - Old behaviour (eval Julia source) preserved when `target_kind = "script"` — keeps the 18d capstone test green.
   - New behaviour for `target_kind = "method"`: registry lookup + multiple-dispatch invocation on typed mirror struct args.
   - The `Request::DispatchMethod` shape on the wire grows a `target_kind` discriminator.
4. `CallRuntimeMethod` Component: substrate-level, kernel-registered. Input: `(method_signature_iri, inputs)`. Output: a `RuntimeInvocation` resource. Internally: resolves the signature → resolves the env → `ServiceSpawner::ensure_service` (idempotent) → `attach_uds` → dispatch → assembles the invocation.
5. `dispatched_to` propagation: language-runtime → facade → trace → orchestrator → `RuntimeInvocation` property.
6. Decision for the 18c.5-deferred `spawner_backend` trace property: I'd land it (one-line addition; useful for audit queries; trivial test).

**Acceptance.**
- `CallRuntimeMethod` invokes a kinase handler (`compute_selectivity_index(c::Compound, t1::Target, t2::Target)`) and returns the expected value.
- The committed `RuntimeInvocation` has `dispatched_to` populated with the `Module.method(::Compound, ::Target, ::Target)` string Julia's `which()` returned — typed class-IRI-bearing dispatch info, useful for audit.
- Mirror struct values round-trip across the boundary correctly (kinase resources committed in tests are usable as `CallRuntimeMethod` inputs via the generator's `decode_*` / `encode_*` helpers).
- Test exercises both `RunRuntimeScript` and `CallRuntimeMethod` against the same warm `ServiceHandle` to confirm coexistence.

---

#### Phase 19a.5 — D31 infrastructure (external institution lifecycle, ~3 weeks) ✓

**Goal.** Land the framework that [D31](d31-external-institution-lifecycle.md) specifies: mirror generation CLI, env image build, institution registration, kernel-emits-request / orchestrator-services-IO dispatch, Verdict commit pipeline. Generator-agnostic; the surfaces light up against Julia first because that's the only generator shipped, but they accept any future language without protocol churn.

The work is split into five sub-milestones (a–e). Sub-milestones a–d build the framework; sub-milestone e is the **acceptance test for the framework** — the first real external institution (IntervalArithmetic) ships in 19a.6 and exercises every piece of 19a.5 end-to-end.

##### 19a.5.a — Mirror CLI (~3 days)

**Files created.**
- `cli/src/commands/mirror.rs` — `mirror create`, `mirror get`, `mirror list`, `mirror inspect` subcommand handlers.

**Files modified.**
- `cli/src/main.rs` — register the new `mirror` subcommand group.

**Tasks.**
1. `mirror create`: resolve `--filter` / `--filter-file` query against `--layer`, materialize seed class IRIs, dispatch to language-specific generator (`JuliaMirrorGenerator` from `eigenius-julia`), commit `RuntimePackageMirror` to chain via `LoadRequest`, write source files to `--output`. Idempotence: same inputs produce same `library_content_hash` → kernel deduplicates the commit.
2. `mirror get`: read `RuntimePackageMirror.library_content` from the chain by IRI, base64-decode the embedded files, write to `--output`. Read-only; no commit.
3. `mirror list`: EigenQL query for all `RuntimePackageMirror` resources, optional `--language` / `--layer` filters.
4. `mirror inspect <iri>`: fetch the resource, print its metadata (generator id/version, source layer, mirrored class IRIs, library_content_hash).
5. Tests: a fake-chain integration test exercising `create` + `get` round-trip, and a CLI smoke test against the kinase mirror.

**Acceptance.**
- `eigenius mirror create --layer <l> --filter-file ./classes.eigenql --language julia --output ./out` produces files matching what the existing `JuliaMirrorGenerator` integration test expects, plus a chain commit.
- `eigenius mirror get --iri <mirror-iri> --output ./out` produces byte-identical files to what `create` wrote.
- Re-running `mirror create` with the same inputs produces no new commit (idempotence).

---

##### 19a.5.b — `env create` for Julia (~3 days)

**Files created.**
- `cli/src/commands/env.rs` — `env create`, `env list`, `env inspect` subcommand handlers.
- `crates/eigenius-julia/src/env_builder.rs` — high-level builder that takes `--handler-package` + `--mirror` and produces an env image + RuntimeEnvironment commit. Wraps the existing `JuliaLanguageRuntime::build_environment_image` machinery.

**Files modified.**
- `crates/eigenius-julia/src/runtime.rs` — extend `build_environment_image` to accept an optional handler-package path that gets baked in alongside `EigeniusJuliaCommon` and the mirror.
- `crates/eigenius-julia/src/dockerfile.rs` — extend `JuliaImagePlan` with handler-package fields; install_packages fragment runs `Pkg.develop` for the handler package.

**Tasks.**
1. CLI surface per [D31 §4.2](d31-external-institution-lifecycle.md#42-phase-3--build-the-env-image-with-eigenius-env-create): `--lang`, `--handler-package`, `--mirror`, `--include-package` (repeatable), `--as-iri`, `--base-image`, `--push-to`.
2. Validation: handler-package's `Project.toml` must declare deps on `EigeniusMirror` + `EigeniusJuliaCommon`; mirror IRI must resolve to a chain-committed `RuntimePackageMirror`.
3. Image build: extend the existing `eigenius-julia` build pipeline to bake the handler-package source into the image alongside the mirror, set the worker's `Project.toml` deps to include it, run `Pkg.develop` + `Pkg.precompile` in install_packages so the handler module is precompiled at image-build time.
4. Commit `RuntimeEnvironment` resource with `image_digest`, `language`, references to mirror + handler-package metadata.
5. Tests: end-to-end test that builds an env image with a fake handler package, verifies the worker can `using` the handler module after spawn.

**Acceptance.**
- `eigenius env create --lang julia --handler-package ./TestHandler --mirror <iri> --as-iri <env-iri>` produces an image_digest + committed RuntimeEnvironment.
- The built image's worker boots, finds the handler module in `Base.loaded_modules`, finds its handler functions in `Main` after `using`.
- Re-running `env create` with the same inputs produces the same digest (deterministic image build).

---

##### 19a.5.c — `DispatchExternal` RPC + orchestrator handler (~5 days)

**Files created.**
- `proto/external_dispatch.proto` (or extend existing kernel proto) — `DispatchExternalRequest`, `DispatchExternalResponse`, `DispatchExternalError` messages per [D31 §6.2](d31-external-institution-lifecycle.md#62-wire-shape).
- `kernel/src/server/external_dispatch.rs` — kernel-side RPC stub: emits a `DispatchExternalRequest` over a back-channel during commit pipeline processing, blocks on the orchestrator's response.
- `orchestration/runtime-substrate-native/src/dispatch_external.rs` — napi addon method `addon.dispatchExternal(institution_iri, env_iri, image_digest, method_name, signature_iri, input_cbors)` → `{ output_cbor, runtime_invocation_partial_cbor }`.
- `orchestration/src/components/dispatch_external.ts` — TS handler that receives the kernel's request, routes to the addon method, returns the response.

**Files modified.**
- Kernel commit pipeline (`kernel/src/server/mod.rs` — `Load` RPC handler) — when the new layer's institutions include `runtime: external` and the commit triggers AutoOnLoad, route the dispatch through `external_dispatch.rs` rather than via the in-process `Institution::query`.

**Tasks.**
1. Wire shape exactly per D31 §6.2 — list-of-CBOR for inputs (Sigma-lowering happens in the kernel before the request).
2. Channel mechanics: regular request/response gRPC. Kernel already holds an `orchestrator_client` ([kernel/src/server/mod.rs:1051](../../kernel/src/server/mod.rs#L1051)) for IO-WASM-component registration and dispatch — same pattern reused for `DispatchExternal`. The orchestrator's gRPC server gains a new RPC method; the kernel's institution dispatch path calls it via the existing client. No streaming, no polling.
3. Failure path mapping per [D31 §6.4](d31-external-institution-lifecycle.md#64-failure-paths) — orchestrator unreachable → `ExternalDispatchUnavailable`; substrate failed → Verdict.Fails with diagnostic; etc.
4. Tests: integration test with a stub external institution that always returns Verdict.Holds; verify the kernel-orchestrator round-trip works end-to-end.

**Acceptance.**
- A test institution with a stub Julia handler + AutoOnLoad QueryClass fires correctly on commit; Verdict round-trips via DispatchExternal.
- All four failure modes from §6.4 produce the expected error shape.

---

##### 19a.5.d — Verdict commit pipeline (~3 days)

**Files modified.**
- `kernel/src/institution/dispatch.rs` — extend `dispatch_auto_on_load_for_resource` to handle `runtime: external` institutions: emit `DispatchExternalRequest` (via 19a.5.c's mechanism), parse the returned Verdict resource, apply the gate.
- `kernel/src/server/mod.rs` — commit pipeline integration: AutoOnLoad runs as part of the commit transaction; Verdict + RuntimeInvocation commit transactionally with the gated resource per [D31 §6.3](d31-external-institution-lifecycle.md#63-verdict-commit-semantics).

**Tasks.**
1. Extend `dispatch_auto_on_load_for_resource` to branch on `Institution.runtime`. WASM/InProcess use the existing `InstitutionRuntime::query`. External emits via 19a.5.c.
2. Verdict commit: when an external dispatch returns successfully, build the Verdict resource (IRI, ctor_name, verdict_subject, verdict_query_class, runtime_invocation, dispatched_to, optional diagnostic) and add to the same kernel commit transaction as the gated resource.
3. RuntimeInvocation commit: build from the orchestrator's `runtime_invocation_partial_cbor` (carrying dispatched_to, image_digest, started_at/completed_at, numerical_metadata) plus kernel-known fields (script ← signature IRI, environment ← env IRI, inputs ← gated resource IRI, output ← Verdict IRI). Commit transactionally.
4. Failure-mode handling: Verdict.Fails commits Verdict but rejects gated resource; orchestrator-unreachable aborts the entire transaction; etc.
5. Tests: a stub external institution fixture that returns Holds/Fails/Undecidable on demand; verify the gate behavior + Verdict commit semantics for all three.

**Acceptance.**
- Verdict.Holds → both gated resource and Verdict committed.
- Verdict.Fails → Verdict committed; gated resource rejected; the `Load` request errors out with a typed error pointing at the Verdict IRI.
- Verdict.Undecidable → both committed (audit-only).
- All three commits land in one kernel transaction; no partial-state windows.

---

##### 19a.5.e — `institution install` external mode (~2 days)

**Files created.**
- `cli/src/commands/institution.rs` — `institution install`, `institution list`, `institution inspect` subcommand handlers.

**Files modified.**
- `cli/src/main.rs` — register the `institution` subcommand group; `capability install` becomes a deprecation alias dispatching to `component install` / `institution install` (per [issue #42](https://github.com/eigenius/eigenius/issues/42)).

**Tasks.**
1. `institution install --definition <file>` per [D31 §5.1](d31-external-institution-lifecycle.md#51-cli-surface). Sends LoadRequest with auto_commit; kernel cross-checks env_iri + mirror IRI references at commit time.
2. Cross-checks: `Institution.runtime_environment` must resolve to a `RuntimeEnvironment`; `Institution.mirror` must resolve to a `RuntimePackageMirror`; QueryClass.query_handler procedure IRIs must be unique within the institution.
3. Definition file shape per [D31 §5.2](d31-external-institution-lifecycle.md#52-definition-file-structure) — Eigon-JSON with Institution + QueryClass + ExportFormat + ImportFormat + Comorphism declarations, no inline binary, references to chain-pinned env/mirror.
4. `institution list` / `inspect` for visibility into installed institutions.
5. Tests: install a stub external institution (no real worker yet); verify the chain commits correctly and that subsequent commits trigger AutoOnLoad dispatch via 19a.5.d.

**Acceptance.**
- A definition file with all the institution-shape resources commits cleanly.
- Cross-checks reject definitions referencing non-existent env or mirror IRIs.
- `eigenius institution list` shows installed institutions with their runtime kind.

---

#### Phase 19a.6 — IntervalArithmetic institution (~1 week, integration test for D31) ✓

**Goal.** Bring up the first real external institution against 19a.5's framework. **Acts as the integration test for D31 end-to-end** — 19a.5 isn't done until IntervalArithmetic's `BoundedBy` AutoOnLoad gate fires correctly on commit, with a Verdict landing on the chain. Surfaces any 19a.5 gaps that need fixing before more institutions land.

**Why IntervalArithmetic first.** Smallest non-trivial institution in the Phase 19 plan:
- One resource class (`BoundedBy`).
- One QueryClass (`AutoOnLoad` validator).
- No outgoing Comorphisms (defer §6.6 until a second institution exists).
- Verified API surface ([D27 §4.0.2](d27-julia-institutions.md): `inf`/`sup` accessors, `interval` constructor, `issubset_interval`).
- No parameter macros, no MTK, no JuMP — cleanest of the five Julia institutions.

**Files created.**
- `crates/eigenius-julia-intervals/Cargo.toml` — new workspace member, the institution's Rust crate.
- `crates/eigenius-julia-intervals/src/lib.rs` — institution declarations (Institution + BoundedBy class + QueryClass + ExportFormat + ImportFormat resources as Eigon-JSON / ESL).
- `julia/institutions/intervals/EigeniusIntervals/Project.toml` — handler package per [D31 §4.1](d31-external-institution-lifecycle.md#41-phase-2--author-the-handler-package).
- `julia/institutions/intervals/EigeniusIntervals/src/EigeniusIntervals.jl` — `validate_bounded_by(b::BoundedBy)` handler returning Verdict.
- `crates/eigenius-julia-intervals/tests/end_to_end.rs` — full e2e: commit BoundedBy ontology → mirror create → env create → institution install → commit a BoundedBy instance → verify AutoOnLoad fires + Verdict committed.

**Tasks.**
1. Author the institution's resource ontology (BoundedBy class, BoundedBy properties: value/lower/upper, ExportFormat/ImportFormat for the typed payload).
2. Generate the mirror against the BoundedBy class via `eigenius mirror create`.
3. Author the handler package per D31 §4.1; `validate_bounded_by` calls `IntervalArithmetic.issubset_interval`.
4. Build env image via `eigenius env create`.
5. Author institution declaration; `eigenius institution install`.
6. **Trait-shape change carried over from 19a.5.d** — thread `runtime_invocation_partial_cbor` from `DispatchExternalResponse` back through `Institution::query` so the kernel can fold the substrate-captured provenance (image digest echo, started/completed timestamps, numerical_metadata, dispatched_to) into a full `RuntimeInvocation` and commit it transactionally with the gated resource and the Verdict per [D31 §6.3](d31-external-institution-lifecycle.md#63-verdict-commit-semantics). This requires:
   - Growing the `Institution::query` return type from `Result<Resource, InstitutionError>` to a richer struct that carries optional invocation provenance (or adding a sibling `query_with_provenance` default method that other impls fall back to).
   - Updating every existing impl: `WasmInstitution`, in-process institutions, `ExternalInstitution`, and the test stubs in `kernel/src/institution/dispatch.rs::tests`.
   - Wiring `dispatch_auto_on_load_for_resource` to receive the provenance and surface it to the commit caller.
   - Building the full `RuntimeInvocation` resource in the commit pipeline (kernel fills `script` ← signature_iri, `environment` ← env_iri, `inputs` ← gated resource IRI, `output` ← Verdict IRI) and adding it to the same transaction as the gated resource + Verdict.
   The IntervalArithmetic e2e is the first verified consumer — design the trait shape against its actual needs rather than guessing now. **No "we'll do this later" deferrals**: 19a.6 is incomplete until the provenance commit lands.
7. Test the full lifecycle: commit a BoundedBy(value=2, lower=1, upper=3) → expect Verdict.Holds + commit succeeds with both Verdict and RuntimeInvocation resources committed alongside. Commit a BoundedBy(value=5, lower=1, upper=3) → expect Verdict.Fails + commit rejected with the Verdict IRI in the error.
8. Document the bring-up as the canonical "how to add an institution" tutorial.

**Acceptance.**
- The full e2e test passes against `DockerServiceSpawner` (with buildah).
- Verdict resources land on the chain with the expected shape ([D31 §6.3](d31-external-institution-lifecycle.md#63-verdict-commit-semantics)).
- `RuntimeInvocation` resources land alongside, carrying the substrate-captured provenance — provenance commit, **not deferred any further**.
- Failed commits return errors that point at the rejecting Verdict IRI.
- Docs cover the "fresh institution from scratch" workflow.
- **D31 acceptance criterion fulfilled**: the framework supports a real external institution end-to-end without structural gaps.

If 19a.6 surfaces structural issues with 19a.5, fix them in 19a.5 (don't paper over in IntervalArithmetic). The framework's correctness is what 19a.6 validates.

---

#### Phase 19a.7 — Minimal config primitive (~3 days) ✓

**Goal.** A small layered config loader (defaults → file → env → construction overrides) covering the substrate concerns 19a forces. New crate so the kernel and orchestrator can adopt it later without circular deps. Replaces ad-hoc env-var reads in the substrate. Per the [config-system memory](../../.claude/projects/-home-hm-src-eigenius/memory/project_config_system.md), the comprehensive settings story (audit, hot-reload, validation, per-namespace overrides) is a follow-on phase; this sub-milestone ships the *primitive*, not the full system.

**Files created.**
- `crates/eigenius-config/Cargo.toml` — new workspace member.
- `crates/eigenius-config/src/lib.rs` — `Config` struct, layered `Loader`, search-path conventions.
- `crates/eigenius-config/src/substrate.rs` — `SubstrateConfig` schema (image registry, backend selection, per-backend tunables).
- `crates/eigenius-config/tests/loader_test.rs` — defaults / file / env / override precedence tests.
- `crates/eigenius-config/examples/eigenius.toml` — annotated sample config.

**Files modified.**
- Workspace `Cargo.toml` — add `crates/eigenius-config`.
- `crates/runtime-substrate/Cargo.toml` — depend on `eigenius-config`.
- `crates/runtime-substrate/src/facade.rs` — read substrate-level config (image registry, default spawner backend) from `SubstrateConfig` instead of constants.
- `crates/runtime-substrate/src/spawner/service/local.rs` + `docker.rs` — read backend-specific config (registry URL, daemon socket override).
- `kernel/src/main.rs` (or wherever the substrate is constructed) — load config at startup, pass to substrate.

**Tasks.**
1. Schema:
   ```toml
   [image]
   registry_url = "localhost:5000"
   registry_credentials_env = ""  # name of env var holding the auth token

   [docker]
   daemon_socket = "unix:///var/run/docker.sock"

   [local]
   julia_binary = "julia"        # PATH lookup if relative
   ```
   Pool / scaling tunables are deliberately absent — production scaling is the platform's concern (HPA / KEDA / ACA scale rules), not the substrate's. Add per-backend knobs only as concrete needs arise.
2. Loader: load from $EIGENIUS_CONFIG, then ./eigenius.toml, then ~/.config/eigenius/config.toml; layer env vars (`EIGENIUS_IMAGE_REGISTRY_URL`, `EIGENIUS_DOCKER_DAEMON_SOCKET`, …) on top; allow construction-time overrides for tests.
3. Validation: registry URL parseable, daemon socket reachable when its backend is selected.
4. Replace direct env reads in substrate. NB: per-spawn env vars (`EIGENIUS_RUNTIME_ENV_DIGEST`, `EIGENIUS_RUNTIME_ENV_MANIFEST_HASH`) are *not* config — they're per-invocation parameters and stay as direct env reads inside the worker bootstrap.
5. Document the precedence rules in the crate docstring.

**Acceptance.**
- `cargo test -p eigenius-config` passes (defaults / file / env / override precedence).
- Substrate boots with explicit `SubstrateConfig`; ad-hoc env-var reads removed for image / backend-selection concerns.
- Sample `eigenius.toml` loads cleanly and yields the documented defaults.
- Validation rejects malformed config with clear errors.

---

#### Phase 19a.8 — End-to-end demo + integration tests (~4 days) ✓

**Goal.** Kinase-grounded e2e exercise of all 19a pieces (`ServiceSpawner` lifecycle, mirror generator, `CallRuntimeMethod` with typed-mirror dispatch, `dispatched_to`, config-loaded backend tunables). Regression coverage anchors against Phase 18.

**Files created.**
- `crates/eigenius-julia/tests/e2e_kinase.rs` — full e2e exercising `CallRuntimeMethod` against the generator-produced kinase mirror.
- `crates/eigenius-julia/tests/service_lifecycle_test.rs` — `ensure_service` idempotence; warm-reuse timing across multiple dispatches against the same service handle; `drain` tears down cleanly.
- `crates/eigenius-julia/tests/regression_18d_capstone.rs` — explicit 18d capstone path against the production crate.
- `crates/eigenius-julia/tests/mirror_regeneration_test.rs` — verifies that regenerating the kinase mirror against an unchanged ontology layer produces byte-identical output (determinism anchor).

**Tasks.**
1. e2e scenario:
   - Commit the kinase ontology layer (`ontologies/examples/kinase/kinase-ontology.json`).
   - Commit a few `Compound` and `Target` instances from the notebook fixture.
   - Define a `JuliaMethodSignature` resource for `compute_selectivity_index(::Compound, ::Target, ::Target) -> Float64`.
   - Invoke `CallRuntimeMethod` against the signature; substrate `decode_*`s the resources via the generator-emitted helpers, worker dispatches on typed mirror structs.
   - Assertions: result correct (verified against hand-computed selectivity); `RuntimeInvocation.dispatched_to` shows `Module.method(::Compound, ::Target, ::Target)`; second `CallRuntimeMethod` against the same `ServiceHandle` is warm (sub-100ms after the first cold start) — the worker stays alive across dispatches without any pool layer.
2. Service lifecycle test:
   - `ensure_service(spec)` → `attach_uds` → dispatch a Health RPC (sub-100ms; service is warm) → `attach_uds` again → dispatch (also warm) → `drain` → `attach_uds` returns `Err` (service is gone).
   - Idempotence: second `ensure_service(spec)` with the same spec returns the same `ServiceHandle`.
3. Mirror regeneration test:
   - Build the env image; capture the kinase mirror's `library_content_hash`.
   - Build again from the same ontology layer; assert the new hash matches.
   - Modify a property in the kinase ontology (in a test-local layer); assert the new hash differs.
4. Regression coverage:
   - All Phase 18 substrate tests pass (`cargo test --workspace`).
   - 18d capstone test passes against the production crate.
   - The bash test runtime (Phase 18c.6) still works alongside the Julia production crate (no shared-state interference).
5. Document the test scenarios in `crates/eigenius-julia/tests/README.md` so future contributors can extend rather than re-derive.

**Acceptance.**
- e2e kinase test passes against both `LocalServiceSpawner` and `DockerServiceSpawner`.
- Warm-reuse timing assertion holds: first dispatch into a fresh service ≥ several seconds (Julia cold-start); subsequent dispatches against the same service handle sub-100ms.
- Mirror determinism test passes (same ontology → same hash; modified ontology → different hash).
- All Phase 18 tests pass.
- `dispatched_to` shows the expected `Module.method(::Compound, ::Target, ::Target)` string in the test's assertion.

### Phase 19b — Folded into 19a

The original Phase 19b (`eigon-julia-gen` mirror generator) is folded into [Phase 19a.3](#phase-19a3--mirror-generator-10-days). The worker dispatch contract is shaped by whether mirrors exist; separating the substrate POC from the mirror generator would force the worker-side dispatch logic to be written twice (raw-dict payloads in 19a, typed-mirror-struct payloads in 19b) and the interdependency is tight enough to address them together. RFC 8746 typed-array tags for numerical arrays also land in 19a.3 alongside the rest of the CBOR codec work.

The 19b letter is preserved (rather than renumbering 19c–19h up) to keep cross-references in the codebase + downstream design docs stable.

### Phase 19c — Per-environment images — *skipped*

Mostly fictional once 19a.5/19a.6 landed. The original framing assumed the substrate POC would ship Julia bundled inside the orchestrator (deployment shape (c)) and that 19c would later flip to shape (a) (per-env image); in practice 19a built directly at shape (a) and the bundled-Julia intermediate step never existed. Concretely:

- The deterministic two-stage Dockerfile, `Pkg.instantiate` + `Pkg.precompile`, build-time provenance under `/etc/eigenius-runtime-env/`, and `RuntimeEnvironment.image_digest` commit all shipped in 19a — the IntervalArithmetic demo exercises every piece.
- Registry push (vs. the current `buildah push docker-archive: → docker load` against the local daemon) is a multi-host deployment concern, not a shape flip. The `image.registry_url` field reserved in 19a.7's config schema is the consumer-side hook; lift the actual push when the first deployment that needs cross-host image distribution lands.
- `JuliaPackagePin` projection (parsed Manifest.toml as one Resource per pinned dep) is genuinely not done, but it has no current consumer — the lockfile bytes already commit on `RuntimeEnvironment.lockfile`, and no EigenQL query depends on the parsed shape yet. Add when the first such query is written.

The 19c letter is preserved (rather than renumbering 19d-19h up) to keep cross-references in downstream docs stable.

- *Production scaling concerns (HPA / KEDA / ACA scale rules) sit in their respective deployment-platform configs, not in the substrate. The platform-managed `ServiceSpawner` backends (`K8sDeploymentSpawner`, `AzureContainerAppsSpawner`) land as a separate phase when production deployment ships.*

### Phase 19d.0 — Chain-mirrored EigenTT inductives + `FormulaTerm` (~1.5–2 weeks) ✓

**Goal.** Bring EigenTT inductives onto the chain so cross-institution formulas have a typed shared representation. Spec: [D32](d32-chain-mirrored-mini-tt-inductives.md). Prerequisite for 19d (Symbolics needs `FormulaTerm` to type its `SymbolicExpression.term`) and for the comorphism story across 19e–19h (every numerical institution consumes the same `FormulaTerm` payload).

Four independently-shippable landings, dependency-ordered:

#### Phase 19d.0.a — `core:arg_name` ontology addition (~half day) ✓

**Goal.** Add the one missing piece on top of the chain ontology's existing inductive-ctor-arg surface (the `InductiveArgType` family is already in place from Phase 11b). Per [D32 §3.2](d32-chain-mirrored-mini-tt-inductives.md): `arg_name` lets JSON-authored ontologies and mirror generators give ctor argument slots readable names instead of falling back to positional defaults.

**Files modified.**
- `ontologies/core/core-ontology.json` — add `core:arg_name` Property (recommended-not-required on `InductiveArgType`). Triggers a `seed_manifest_v1` bump; existing dev DBs hit the `seed manifest drift` boot check and need `docker compose down -v` to re-seed.

**Acceptance.**
- `cargo test --workspace` clean (existing ESL-emitted inductives validate unchanged; new property surfaces).
- `eigenius inspect "urn:eigenius:core:arg_name"` resolves and shows the recommended-on-InductiveArgType shape.

#### Phase 19d.0.b — Validator + `core:inductive` primitive type (~2 days) ✓

**Goal.** Type-check inductive values at commit time per [D32 §3.2 + §3.4](d32-chain-mirrored-mini-tt-inductives.md). New primitive type `core:inductive`; new validator rule recursing through ctor_args; structured field-path errors.

**Files modified.**
- `kernel/src/validation/mod.rs` — new rule `check_inductive_value`; extend `check_class_types` to accept `InductiveType` references.
- `kernel/src/ontology/well_known.rs` — `INDUCTIVE` primitive type IRI.
- `kernel/src/ontology/value.rs` — wire-shape recognition for inductive values.

**Acceptance.**
- Hand-rolled `Nat = Zero | Succ(Nat)` declared in a test ontology; `Succ(Succ(Zero))`-shaped value validates; `Succ(LitFloat(1.0))` rejects with structured error.
- `Verdict` continues to validate without modification (no `ctor_args` → no per-arg checks).

#### Phase 19d.0.c — Mirror generator extends to `InductiveType` (~2–3 days) ✓

**Goal.** Emit Julia for chain-committed `InductiveType` resources per [D32 §3.3](d32-chain-mirrored-mini-tt-inductives.md): abstract type + per-ctor structs + decode/encode + `_eigenius_decoders` / `_eigenius_encoders` registry hooks.

**Files modified.**
- `crates/eigenius-julia/src/mirror_gen.rs` — extend the closure walker to visit `InductiveType` references via `arg_type`; add `inductive_emitter.rs` (or a new module) for the per-inductive emission.
- `crates/eigenius-julia/tests/mirror_gen_test.rs` — golden test against a `Nat` fixture.
- `crates/eigenius-julia/tests/mirror_inductive_round_trip.rs` — substrate-side e2e: build an env image with `Nat`, dispatch a script that constructs `Succ(Succ(Zero))` and round-trips the encoded value.

**Acceptance.**
- Generated `Nat.jl` produces `abstract type Nat end; struct Zero <: Nat end; struct Succ <: Nat; pred::Nat end` plus the decode/encode pair.
- Round-trip test passes against `DockerServiceSpawner`.

#### Phase 19d.0.d — `formulas:` ontology layer + initial operator catalog (~1 day) ✓

**Goal.** Commit `urn:eigenius:formulas:FormulaTerm` and the v1 operator catalog per [D32 §4 + §5](d32-chain-mirrored-mini-tt-inductives.md).

**Files created.**
- `ontologies/formulas/formulas-ontology.json` — `FormulaTerm` InductiveType (six ctors: Var, LitFloat, OpRef, App, Lam, Pi), `Operator` Class with `operator_signature` / `operator_arity` / `operator_associativity` / `operator_commutativity` properties, type-as-operator entries (`formulas:types:Real`, `Int`, `Bool`), and the initial operator set (arithmetic, unary numeric, comparisons, `derivative`).

**Files modified.**
- `kernel/src/bootstrap/mod.rs` — embed the new `formulas` ontology layer alongside core / program / reflection / institution / runtime / notebook.
- `kernel/src/validation/mod.rs` — `App`-spine rank check against `Operator.operator_signature`.

**Acceptance.**
- `eigenius inspect "urn:eigenius:formulas:FormulaTerm"` resolves; `formulas:ops:add` resolves with its typed signature.
- An `App(App(OpRef("formulas:ops:add"), Var("x")), LitFloat(2.0))` value validates as a `FormulaTerm`.
- An arity-mismatched `App(OpRef("formulas:ops:add"), Var("x"))` (one arg short) rejects with `OperatorArityMismatch`.
- Existing tests stay green; the new ontology layer adds to `seed manifest drift` detection on stale DBs.

### Phase 19d — `Symbolics` / `ModelingToolkit` institution (~2.5 weeks, was 3) ✓

**Prerequisite:** Phase 19d.0 (`FormulaTerm` available on the chain).

- `eigenius-julia-symbolics` crate implementing the D14 `Institution` trait with declarations from D27 §4.1: `SymbolicExpression`, `SymbolicallyReducesTo`, `Substitutes`, `SimplifiesTo`, `SatisfiesEquation` resource classes — all carrying `term: FormulaTerm` (from 19d.0). ExportFormats, ImportFormats, QueryClasses (AutoOnLoad on commit-time validation; OnDemand for FIBER-side; Decidable for `qc_symb_check_equivalence`).
- End-to-end demo: a notebook that loads physical-system equations, gets them simplified via `qc_symb_simplify`, runs a numerical solve via a substrate component.

### Phase 19e — `IntervalArithmetic` institution (~2 weeks) ✓

*Landed under [Phase 19a.6](#phase-19a6--intervalarithmetic-institution-1-week-integration-test-for-d31) as the D31 framework's integration test rather than as a separate phase. The substantive deliverables (resource classes, Decidable role on `qc_intv_validate_bounded_by`, end-to-end e2e test) all shipped there; this section is preserved for the cross-reference.*

- `eigenius-julia-intervals` crate. Declarations from D27 §4.3: `BoundedBy(value, interval)`, `ProvesBoundOn(function, domain, interval)`, `ContainsRoot` resource classes; Decidable role on `qc_intv_validate_bounded_by` so user programs can write `Exp::NativeDecide` predicates that reduce operationally.
- *Reordered ahead of JuMP* — IntervalArithmetic has no solver-dependency surface, the kinase CI columns map directly onto `BoundedBy`, and the Decidable role is the most novel piece of D14 runtime mechanics; we want it exercised early.
- *Numerical hardening (BLAS pinning, FMA discipline, host-conformance gating, cross-host reproducibility tooling) split out and deferred to [issue #43](https://github.com/eigenius/eigenius/issues/43)*. Interval arithmetic is conservative under any IEEE-754-conforming host, so the institution itself doesn't depend on it; determinism becomes load-bearing once institutions chain (DiffEq → IntervalArithmetic, etc.). Land it before claiming reproducibility guarantees that compose across institutions.

### Phase 19f — `JuMP` institution (~2 weeks; ~1 day landed) ✓

**Landed scope (HiGHS only):** `OptimisationProblem` (variable_names, variable_bounds, objective, sense, constraints — objective and each constraint LHS as `FormulaTerm`), `VariableBound`, `Constraint`, `ConstraintRelation` inductive (`LE | GE | EQ`), `OptimisesTo` (problem, termination_status, objective_value, variable_values, abstol, reltol). AutoOnLoad `validate_optimum` re-solves the model and tolerance-checks the resolved objective against the claim; OnDemand `qc_jump_solve` materialises an `OptimisesTo` from a fresh solve.

**Walker design:** structurally identical to EigeniusDiffEq's `formula_to_value` (left-spine collection, OpRef dispatch, recursive descent), with the operator catalog mapped to Julia's general-arithmetic operators rather than Float64-only operators — JuMP's overloading on `JuMP.VariableRef` promotes the result to `AffExpr` / `QuadExpr` / `NonlinearExpr` automatically. The walker has a **smart-pow flag** (default `true`) that recognises integer-valued `LitFloat` exponents on `pow` with `n ≤ 2` and unrolls to repeated multiplication, so HiGHS sees a `QuadExpr` rather than a `NonlinearExpr` it would reject. See [`julia/research/jump-formula-term-probe.md`](../../julia/research/jump-formula-term-probe.md) for the empirical demonstration (LP + QP + NL through the walker, all four cases landing on their analytical optima).

**Why one solver and not all four (HiGHS / GLPK / Ipopt / Gurobi).** Per-solver siblings reuse the same ontology + handler walker; only the `Model(SolverFoo.Optimizer)` line and the `Project.toml` deps change. `urn:eigenius:institutions:jump_ipopt` is the natural next drop (general nonlinear; smart-pow flag flips to `false` since Ipopt accepts `NonlinearExpr` directly), and lands when a downstream worked example needs it.

**Sub-milestone Phase 19f.1 — Symbolics → JuMP comorphism ✓:** Composite-input pattern parallel to Catalyst → DiffEq. New `symbolics:SymbolicsToJuMPInput` composite class (objective: SymbolicExpression + variable_names + framing variable_bounds + sense + framing constraints), Symbolics-side `frame_as_optimisation_problem` handler, OnDemand `qc_symb_to_jump` QueryClass, ExportFormat `ef_symb_to_jump_input`, JuMP-side ImportFormat `if_jump_optimisation_problem`, identity Lambda on OptimisationProblem, and the Comorphism triple at [`julia/comorphisms/symbolics-to-jump.eigon.json`](../../julia/comorphisms/symbolics-to-jump.eigon.json). The handler reads `objective.term` (identity on FormulaTerm — D32 §6) and forwards the framing fields unchanged to `OptimisationProblem`. End-to-end test ([`crates/eigenius-julia/tests/symbolics_to_jump_e2e.rs`](../../crates/eigenius-julia/tests/symbolics_to_jump_e2e.rs)): kinase Ki-fit example — fit the inhibition constant `Ki` from observed IC50 measurements at known substrate concentrations via the closed-form competitive-inhibition relation `IC50 = Ki·(1 + [S]/Km)`, expressed as an SSE objective `Σ(IC50_obs_i − Ki·c_i)²` authored as a SymbolicExpression. The test exercises the full pipeline: SymbolicsToJuMPInput → `frame_as_optimisation_problem` → OptimisationProblem → `qc_jump_solve` → OptimisesTo (`Ki* = 2.0`, SSE = 0) → `validate_optimum` → Holds. One structural fix discovered along the way: chain rejects empty arrays at commit, so `OptimisationProblem.constraints` and `SymbolicsToJuMPInput.framing_constraints` were promoted from required to recommended (an unconstrained problem with only variable bounds is well-posed and shouldn't need a redundant constraint to satisfy the schema).

### Phase 19g — `DifferentialEquations.jl` institution — ODEs only (~3 weeks) ✓

- `eigenius-julia-diffeq` crate. **v1 scope: ODEs only.** SDEs, DAEs, DDEs, jump processes, and hybrid systems are deferred to follow-on milestones (19i and beyond) to be triggered by domain demand.
- Resource classes (per D27 §4.5 verified note): `OdeProblem` (renamed from `OdeSystem` to avoid MTK collision), `OdeSolution`, `OdeSteadyState`, plus `ReproducibleIntegration` framing for AutoOnLoad re-validation. `IntegrationCertificate` / `BoundedError` IRIs reserved for a future TaylorModels-backed institution that produces rigorous interval enclosures. `ParameterFit` moved to the JuMP / Optimization institution scope.
- ExportFormats / ImportFormats: extract an `OdeProblem` and an `OdeSolution`; reify solution and steady-state results back as typed resources.
- QueryClasses: `qc_diffeq_validate_solution` (`AutoOnLoad` — re-solves and checks the trajectory hash), `qc_diffeq_solve(input)` (`OnDemand`), `qc_diffeq_steady_state` (`OnDemand`).
- Comorphism out to Phase 19e (DiffEq → IntervalArithmetic) — given an `OdeSolution` plus an interval-extension of the vector field, produce a `ProvesBoundOn` resource. This is one of the bridges Phase 21 needs for *operationally verified* PK predictions.
- *Reordered ahead of Catalyst* — Catalyst's `qc_to_ode` Comorphism has nowhere to land if DiffEq isn't ready first; with this reordering, 19g ships using hand-written compartmental ODEs (PK two-compartment is well-defined without Catalyst), and the DiffEq institution is in place when 19h adds the Catalyst → DiffEq Comorphism.

### Phase 19h — `Catalyst.jl` institution (~3 weeks) ✓

**Landed scope:** `ReactionNetwork`, `ConservationLaw`, plus the `CatalystToOdeInput` composite class supporting the cross-institution comorphism. AutoOnLoad `qc_cat_validate_conservation_law`, OnDemand `qc_cat_to_ode` (the operational backing of the Catalyst → DiffEq comorphism). The `SteadyState` / `DeficiencyZero` / `DeficiencyOne` / `WeaklyReversible` / `ComplexBalanced` resource classes and their AutoOnLoad query classes are deferred until a domain-driven test case appears — adding them speculatively without a verifying example would compound rather than reduce risk in the chain ontology.

**Sub-milestone Phase 19h.1 — Catalyst → DiffEq comorphism ✓:** Identity-on-`OdeProblem` Lambda, Comorphism triple linking `ef_cat_to_ode_input` + `m_id_ode_problem` + `if_diffeq_problem`, and the operational `qc_cat_to_ode` OnDemand QueryClass that compiles a `ReactionNetwork` to an `OdeProblem` with `FormulaTerm`-typed RHS. End-to-end test ([`crates/eigenius-julia/tests/catalyst_to_diffeq_e2e.rs`](../../crates/eigenius-julia/tests/catalyst_to_diffeq_e2e.rs)): `A → B; k*A` reaction → Catalyst compile → DiffEq integrate → `Holds` verdict against the closed-form trajectory. Three structural fixes uncovered along the way (CBOR.Tag-wrapping for inductive payloads in the mirror encoder; abstract-typed `Vector{Abstract<C>}` decode-side comprehension annotation; SymbolicUtils 4 / Symbolics 7 compat alignment) are documented inline at the call sites.

- `eigenius-julia-catalyst` crate. Declarations promoted from D27 §4.4 (Catalyst is now a first-class reference institution given the life-science focus on PK / signaling pathways / metabolic networks).
- Resource classes: `ReactionNetwork`, `ConservationLaw`, `SteadyState`, `DeficiencyZero` / `DeficiencyOne` relations, `WeaklyReversible` / `ComplexBalanced` markers. `MassActionKinetics` / `JumpProcessSemantics` reframed as compilation-path discriminators per D27 §4.4.1 verified note.
- ExportFormats / ImportFormats: extract a `ReactionNetwork` and a `ConservationLaw`; reify steady-state and conservation-law results.
- QueryClasses: `qc_cat_validate_conservation_law` (`AutoOnLoad`), `qc_cat_validate_steady_state` (`AutoOnLoad`), `qc_cat_validate_deficiency_zero` / `_one` (`AutoOnLoad`), `qc_cat_compute_steady_states` (`OnDemand`), `qc_cat_extract_invariants` (`OnDemand`), `qc_cat_check_deficiency` (`OnDemand`, `Decidable`).
- **Comorphism into DiffEq**: declared as a typed D14 Comorphism. Per the Catalyst-ODE probe (`julia/research/catalyst-ode-probe.md`): in Catalyst 16.1.1 the canonical entry point is the symbolic-keyed map form `ODEProblem(rn, [species_sym => value, ...], tspan, [param_sym => value, ...])`; positional-vector form errors with `BoundsError`, `convert(ODESystem, rn)` is broken. The Comorphism's transformation Component compiles direct to `OdeProblem` skipping the `ODESystem` intermediate.
- Comorphism into Symbolics/MTK: deferred until the Symbolics institution gets a typed `ODESystem`-equivalent class (and the Catalyst→ODESystem path itself is settled — the probe shows the conversion to `ODESystem` is broken in 16.1.1).
- Why an institution and not just a substrate component: the fibre has structural invariants (linear conservation laws, deficiency classes, mass-action equivalence) that EigenQL FIBER queries can traverse; substrate components alone would just return numbers without the typed-relation status.

### Phase 19i — D14 §9.3 chain reinsertion ✓ (Phase 1)

**Goal:** Close the spec gap that `Exp::InstitutionInvoke` (and program output more generally) was producing typed Resources but never inserting them into the chain. D14 §9.3 step 4 specifies that the comorphism's reified target-class resource enters the graph through the target institution; step 5's post-translation invariant relies on this entry to operationalise the Satisfaction Condition (§5). Pre-Phase 19i the produced resource only existed in transit as a `Val::ResourceVal` and a serialised RPC payload — addressable from neither EigenQL nor `Inspect`.

**Landed scope (Phase 1).** The same elevation applies uniformly to comorphism reify outputs and to a program's final-step Resource value — they're the same kind of thing, "domain resources the run emitted":

- `Exp::InstitutionInvoke { comorphism_iri, source, target_iri }` — AST node gains an optional `target_iri` for surface-language overrides (default `None` from ESL; Phase 2 EigenQL `INTO X` sets `Some(X)`).
- `EvalCtx::IO { ..., produced_resources: Arc<Mutex<Vec<Resource>>> }` — collector that the reify boundary pushes to.
- `try_d14_institution_invoke` — after reify, mints a deterministic content-hash IRI of the form `urn:eigenius:comorphism-output:<comorphism-tail>:<hex>` from SHA-256 over the canonical Eigon-CBOR of the resource (with `@id` cleared), sets the `@id`, runs the in-eval AutoOnLoad gate (rejects bad outputs before they touch the chain), and pushes into `produced_resources`. Identical reify outputs collide on the same IRI — that is the cross-fibre dedup property the Grothendieck construction wants.
- Run-boundary (`execute_program_nbe_with_institutions_d14`) — when the program's final value is a non-empty Resource with no `@id`, the same scheme assigns `urn:eigenius:program-output:<program-tail>:<hex>` and pushes to the collector. Top-level passthrough (output already has an `@id`) is left alone; primitive values are returned without commit.
- `run_program_inner` (server) — adds `produced_resources` to the program-run layer alongside `ProgramTrace` + `ComponentTrace` observability, switches `ctx.commit("trace")` → `ctx.commit_with_validation("program-run", ...)` (so AutoOnLoad fires on chain entry per D14 §9.3 step 5 as part of normal chain mechanics), and persists both `user_layer` and `provenance_layer` from the `CommitOutcome`.
- `RunProgramResponse.output_resource_iris: Vec<String>` — proto field exposing the assigned IRIs to clients so the notebook / `eigen` SDK can resolve them.

**Tests.** [`kernel/src/program/eval_io.rs::tests::run_boundary_elevates_output_to_chain_resource`](../../kernel/src/program/eval_io.rs) and `…does_not_elevate_when_output_already_has_iri`; the comorphism path extension in [`kernel/tests/d14_dock_assay_demo.rs::comorphism_translates_dock_to_assay`](../../kernel/tests/d14_dock_assay_demo.rs) asserting the reified `AssayPrediction` carries a deterministic `comorphism-output:dock_to_assay:<hex>` IRI; full server-boundary coverage in [`storage/rocksdb/tests/program_output_chain_commit_test.rs`](../../storage/rocksdb/tests/program_output_chain_commit_test.rs) (load ontology, run Construct program over RPC, assert output IRI resolves through `Inspect`, identical input dedupes, distinct input doesn't).

**Notebook demo.** Cells 12–15 of `notebooks/examples/kinase-institutions.json` invoke `comorphisms:symbolics_to_jump` through a one-line wrapper program; the produced `OptimisationProblem` lands at `urn:eigenius:comorphism-output:symbolics_to_jump:<hex>` and is queryable via EigenQL pattern-match on the IRI prefix.

**Phase 2 — EigenQL `INTO` clause ✓.** Same chain-reinsertion semantics, exposed at the EigenQL surface so callers can dispatch via interactive query rather than a wrapper program:

- `FIBER <institution>:<QueryClass> { ... } AS ?var INTO "<iri>"` — optional suffix; without it, current overlay-only behaviour preserved.
- Lexer: `INTO` keyword. AST: `FiberClause.into: Option<Iri>`. Parser: literal-string IRI argument.
- Evaluator (`apply_fiber_clause`): when `into` is set, the response stamps the user-named IRI rather than the synthesized `<doc-iri>:fiber:<clause-idx>:<binding-idx>` transient, and the resource is pushed onto a per-query collector that bubbles up via the new [`query::QueryOutcome`](../../kernel/src/query/mod.rs).
- Server-side `Query` RPC handler: drains `outcome.into_resources` and commits them to the regular chain via `commit_with_validation` (so AutoOnLoad fires uniformly), persisting both `user_layer` and `provenance_layer` on backed sessions. Failure of the INTO commit fails the query — surfacing the validation error to the caller rather than silently dropping the resource.
- Proto: `QueryResponse.output_resource_iris: repeated string` returns the chain-resident IRIs to clients.

A multi-row FIBER with `INTO` (whose IRI is fixed across rows) is rejected at evaluation time with a clear error — the alternative would silently overwrite or fail a chain-validation step, both worse than refusing up front.

**Tests.** Parser: [`fiber_with_into_clause_parses`](../../kernel/src/query/parser.rs), `fiber_without_into_clause_has_none`, `fiber_into_rejects_non_string_argument`. Evaluator: [`eigenql_fiber_into_collects_response_for_chain_commit`](../../kernel/tests/d14_dock_assay_demo.rs) — the dock-assay `validate_prediction` FIBER + INTO produces a Verdict resource in `into_resources` carrying the user-named IRI.

**Notebook demo.** Cells 16–18 of `notebooks/examples/kinase-institutions.json` invoke `qc_symb_to_jump` (the operational backing of the comorphism) via FIBER + INTO, pinning the produced `OptimisationProblem` at a caller-named IRI and verifying it resolves through a follow-up MATCH.

### Phase 19 — Test plan

- Per-institution: AutoOnLoad-on-commit acceptance and rejection, OnDemand FIBER queries, Decidable `Exp::NativeDecide` reduction (where applicable).
- Cross-institution within Julia: a notebook that calls Symbolics to simplify a problem, JuMP to solve a parameter-fit, Catalyst to express the dynamics, DiffEq to integrate, IntervalArithmetic to bound the solution residual.
- Catalyst → DiffEq comorphism: a `ReactionNetwork` resource translates through `qc_cat_to_ode` into an `OdeSystem`; the resulting `OdeSolution` AutoOnLoad-validates against the original network.
- Mirror-anchor regression: a layer that modifies a class used by an existing `JuliaPackageMirror` triggers `MirrorVersionMismatch` on subsequent dispatch.
- Numerical reproducibility: same image + inputs + seed on the same host yields bit-identical output; on different hosts yields semantically equivalent output with `numerical_metadata` divergence flagged.

### Phase 19 — References

- [D27 — Julia Institutions](d27-julia-institutions.md) — full specification
- [D26 — Runtime Substrate](d26-runtime-substrate.md) — substrate this layers on
- D14 — institution protocol (each Julia institution is a D14 institution)
- [D29](d29-eigon-julia-mirror-spec.md) — Faithful translation specification for `eigon-julia-gen`

---

## Phase 20 — Lean 4 Verification Institution

**Goal:** Register Lean 4 as Eigenius's first verification institution under D14, contributing the *verified* epistemic level. Authoring-side workflows (`lean4export`, `LeanMirrorGenerator`, `LeanEnvironment` instantiation) run on the runtime substrate; the verification side (proof-term re-check via `nanoda_lib`) stays in-process for trust-surface reasons. First institution registered with `runtime: in_process` — the `InProcess` `RuntimeKind` is defined ([kernel/src/institution/registry.rs:46](../../kernel/src/institution/registry.rs#L46)) but has no registration path yet, so the kernel side gets one new piece of plumbing. Future verification institutions (Rocq, Isabelle/HOL, SMT checkers) follow the same factoring.

**Structure:** two landings per [D28](d28-lean-4-as-institution.md) §11 — `20a` (the complete first Lean institution, integrated; no PoC-without-correspondence intermediate per D28 §11.1) and `20b` (Mathlib-scale operational landing per D28 §11.2). Phase 20a's sub-milestones (20a.0 design lane through 20a.8 capstone) follow the Phase 19a shape but compress because the substrate, mirror-generator pattern, and runtime-kind taxonomy are all settled.

**Duration estimate:** ~5–7 weeks for 20a across three lanes with significant parallelism; 20b sized by the consumer that asks for it.

**Prerequisites:**
- Phase 11b (inductive types) — both the chain-mirrored `lean:LeanExpr` ontology and the `Verdict`-shaped result need them.
- Phase 12 (D14) — Lean is registered as a D14 institution.
- Phase 18 (runtime substrate) — authoring side runs on it.
- Phase 19a (substrate validated against Julia first; Lean is the second forcing function on the substrate's abstractions and the first consumer of the `InProcess` runtime kind).

**Drives:** [D28 — Lean 4 as Verification Institution](d28-lean-4-as-institution.md).

**Sibling specs landing in 20a:** [D30 — Eigon → Lean Faithful Translation](d30-eigon-to-lean-faithful-translation.md) (sibling to [D29](d29-eigon-julia-mirror-spec.md)) and [D40 — Chain-Mirrored Lean Expressions](d40-chain-mirrored-lean-expressions.md) (sibling to [D32](d32-chain-mirrored-mini-tt-inductives.md)).

### Phase 20a — First complete Lean institution ✓ (landed)

A single integrated landing per D28 §11.1 — the full architectural commitment from "Eigenius's verified epistemic level" through to a committed resource so tagged. No phased intermediate that ships misleading semantics; the correspondence check (D28 §5.5) is the load-bearing piece, so it lands in the same phase as the verification dispatch.

Three parallel lanes, eight sub-milestones, capstone test at the end. Lane structure:

- **Design lane** (20a.0, 20a.0b) — D40 + D30 specs. Authoring; no code. ~1.5 weeks, runs first.
- **Kernel + chain-mirror lane** (20a.1 → 20a.2 → 20a.3 → 20a.4) — `InProcess` dispatch, Lean ontology, nanoda integration, Institution trait impl. ~2.5 weeks, runs after design.
- **Authoring + correspondence lane** (20a.5 → 20a.6 → 20a.7) — `LanguageRuntime` impl, mirror generator, correspondence check. ~2.5 weeks, overlaps with the kernel lane once 20a.0b is settled.
- **Capstone** (20a.8) — end-to-end test tying every preceding sub-milestone into one passing test. ~3 days.

#### Phase 20a.0 — D40 spec (chain-mirrored Lean expressions, ~3 days)

**Goal.** Author [D40](d40-chain-mirrored-lean-expressions.md) — the chain-resident inductive specification for Lean's expression form. Sibling to D32 (FormulaTerm) but Lean-flavoured. No code; design-doc work.

**Scope.**
- `lean:LeanExpr` InductiveType, 10 ctors mirroring nanoda's `Expr` minus `Local` (committed proofs are closed terms; the `Local` variant only exists during nanoda's traversal). Ctors: `Var`, `Sort`, `Const`, `App`, `Pi`, `Lambda`, `Let`, `Proj`, `StringLit`, `NatLit`. Each ctor's argument shape pinned to mirror nanoda's `Expr` arm structurally.
- `lean:LeanLevel` InductiveType: `Zero`, `Succ`, `Max`, `IMax`, `Param`.
- `lean:LeanName` InductiveType: `Anon`, `Str`, `Num`.
- Encoder/decoder semantics: deterministic translation between nanoda's parsed `Expr` and the chain-CBOR shape.
- Version discipline: how `lean4export` semver bumps interact with the spec; what's load-bearing for forward compatibility.

**Acceptance.**
- D40 v1 committed to `docs/design/`.
- Review verifies every nanoda `Expr` variant maps to a `lean:LeanExpr` ctor (modulo the `Local` omission), every universe form maps to a `lean:LeanLevel` ctor, every name shape to a `lean:LeanName` ctor.

---

#### Phase 20a.0b — D30 spec (faithful translation Eigon → Lean, ~4 days)

**Goal.** Author [D30](d30-eigon-to-lean-faithful-translation.md) — the EigonFFI mirror specification. Sibling to D29 (Julia mirror spec); same structural role, Lean-flavoured.

Runs in parallel with 20a.0.

**Scope.**
- Conformance levels (v1 supported subset, planned extensions).
- Mirror package layout (Lake project structure, module file shape, EigonFFI naming convention).
- Closure walk (same algorithm as D29: structural references through `arg_types`, `class_types`, `allows_only`).
- Faithful type translation:
  - Eigon class with required properties → Lean `structure` with required fields.
  - Subclass relationship → Lean coercion instance.
  - `data_type: resource` → field type is the referenced class's mirror structure.
  - Primitive types → Lean equivalents (`Int`, `Float` → `Float`, `String`, `Bool`).
  - Format constraints (regex, date, IRI pattern) → refinement predicates or structure-validity theorems where Lean-expressible.
- Determinism contract.
- Integrity chain (generator content hash, source layer reference).

**Acceptance.**
- D30 v1 committed to `docs/design/`.

---

#### Phase 20a.1 — Kernel `InProcess` runtime kind registration path (~2 days)

**Goal.** Wire the third branch of the kernel's institution-registration logic alongside `Wasm` and `External`. Not Lean-specific — this unlocks any future in-process institution.

**Files created.**
- `kernel/src/institution/in_process_registry.rs` — `InProcessInstitutionRegistry`: a process-global registry of `Arc<dyn Institution>` instances keyed by IRI, populated at kernel-binary startup (see Phase 20a.4 for the call site; the orchestrator process is Deno and links the substrate via napi-rs, *not* the kernel's verification path). Sibling to `InstitutionRuntime` (which is the dispatch target — the in-process registry feeds into it during chain scan).
- `kernel/src/institution/in_process_registry/test_echo.rs` — a trivial `EchoInstitution` stub returning `Verdict::Holds` for any input. Lets 20a.1 ship a registration-path test without needing Lean yet.

**Files modified.**
- `kernel/src/institution/runtime.rs` — exposes a `register_in_process(...)` startup-hook entry that wraps `InstitutionRuntime::register`.
- `kernel/src/capability/registration.rs` (or a new sibling file) — `build_in_process_institution_runtime` walks the chain for `Institution` resources with `runtime: in_process`, looks each up in the `InProcessInstitutionRegistry`, registers into `InstitutionRuntime`. Surfaces a clean error if a chain-declared `in_process` institution has no registered impl (rather than silently dropping it, which is the External path's behaviour when `--orchestrator` is missing).
- `kernel/src/server/mod.rs` — startup wires the in-process registry into the institution-registration pass; emits a `tracing::warn!` when chain-declared `in_process` institutions lack a registered impl, mirroring the existing External warning at [server/mod.rs:1210-1216](../../kernel/src/server/mod.rs#L1210-L1216).

**Tasks.**
1. `InProcessInstitutionRegistry` struct + thread-safe registration API.
2. Third branch in the registration logic — for each `Institution` resource with `runtime: in_process`, look up the impl, register or error.
3. Test harness: a `EchoInstitution` registers via the startup-hook, chain commits a declaration with `runtime: in_process`, end-to-end test confirms an AutoOnLoad QueryClass fires + returns the echo response.

**Acceptance.**
- `cargo test -p eigenius-kernel --lib in_process` passes.
- Chain-declared `in_process` institution dispatches correctly through the standard `InstitutionRuntime` path.
- A chain-declared `in_process` institution with no registered impl surfaces a clean warning + the institution is excluded from the dispatchable set (parallel to the External-without-orchestrator case).

---

#### Phase 20a.2 — Lean ontology: chain-mirrored expressions (~2 days)

**Goal.** Land the `lean:LeanExpr` / `LeanLevel` / `LeanName` chain inductives per the 20a.0 spec. Smoke-tested by hand-encoding a tiny Lean expression as a chain value and round-tripping it through commit + inspect.

**Prerequisite:** 20a.0 (D40 spec).

**Files created.**
- `ontologies/lean/lean-expressions.eigon.json` — the three chain InductiveTypes per D40.

**Files modified.**
- `kernel/src/bootstrap/` — register `lean-expressions.eigon.json` as a bootstrap layer (or add to the per-environment ontology load path; both are reasonable).

**Tasks.**
1. Author the InductiveType declarations per D40 — `lean:LeanExpr` (10 ctors), `lean:LeanLevel` (5 ctors), `lean:LeanName` (3 ctors).
2. Smoke-test: hand-encode the term `λ x : Nat, x` as a chain-CBOR `lean:LeanExpr` value, commit, inspect, confirm round-trip.

**Acceptance.**
- Chain commit + inspect of the smoke-test value passes.
- Type-checker accepts the hand-encoded values (no `AllowedValueViolation` / `ClassTypeMismatch`).

---

#### Phase 20a.3 — `eigenius-lean` crate skeleton + nanoda integration (~3 days)

**Goal.** Stand up the verification-side crate with nanoda_lib as a Cargo dep. Exposes a `check_proof(bytes, target_name, env_iri) -> Verdict` function with no chain integration yet — lib-test only.

**Independent of 20a.1 and 20a.2** — can start in parallel.

**Files created.**
- `crates/eigenius-lean/Cargo.toml` — new workspace member. Deps: `nanoda_lib` (path = "../../references/nanoda_lib"), `eigenius-kernel`, `serde_json`, `cbor`, `thiserror`.
- `crates/eigenius-lean/src/lib.rs` — public surface.
- `crates/eigenius-lean/src/checker.rs` — `check_proof` function wrapping nanoda's parse + check.
- `crates/eigenius-lean/tests/toy_check_test.rs` — lib-test against a hand-rolled toy proof file (e.g. `theorem foo : True := trivial`).
- `crates/eigenius-lean/test_resources/toy_proof.json` — a `lean4export`-format JSON file produced by running `lean4export` on a hand-written Lean project. Vendored for offline reproducibility.

**Files modified.**
- Workspace `Cargo.toml` — add `crates/eigenius-lean` member.

**Tasks.**
1. Crate scaffold (`Cargo.toml`, `lib.rs`, license header).
2. `check_proof` API: accepts the verbatim export bytes, the target Theorem name, and an axiom allowlist (from the eventual `LeanEnvironment`). Parses via nanoda's `Parser::new`, runs the check, returns a structured `Verdict` enum (`Holds` | `Fails { diagnostic }`).
3. Vendor a hand-built toy proof JSON; lib-test confirms `check_proof` admits it; a second test confirms a deliberately-broken proof returns `Fails` with `ProofDoesNotCheck`.

**Acceptance.**
- `cargo test -p eigenius-lean` passes.
- Toy proof admitted; broken proof rejected with structured diagnostic.

---

#### Phase 20a.4 — `LeanInstitution: Institution` impl + chain-mirror translator (~1 week)

**Goal.** Wire the verification side into the kernel: `LeanInstitution` implements `Institution`, registers via 20a.1's `InProcessInstitutionRegistry`, dispatches `proof_check` through 20a.3's `check_proof`. Also ships the bytes → `lean:LeanExpr` translator per D40 — a standalone authoring-side utility, *not* called from `query` (the institution always verifies via the bytes; chain-mirroring is a separate visibility concern for whoever constructs the `LeanProofTerm` resource).

**Correspondence check stub.** This sub-milestone ships with a *stub* correspondence check that always passes (returns `Verdict::Holds` as long as nanoda accepts the proof). Real correspondence lands in 20a.7 once the mirror generator (20a.6) is wired. The stub lets the end-to-end smoke test fire here without blocking on the authoring + mirror path.

**Prerequisites:** 20a.1, 20a.2, 20a.3.

**Files created.**
- `crates/eigenius-lean/src/institution.rs` — `LeanInstitution: Institution` impl. `query` dispatches `urn:eigenius:lean:proof_check` to 20a.3's `check_proof` (correspondence check stubbed); `extract_typed` handles `ef_lean_proof_payload`.
- `crates/eigenius-lean/src/chain_mirror.rs` — bytes → `lean:LeanExpr` translator (D40-conformant). Standalone `pub fn`; the smoke test calls it directly to derive a `proposition` value before committing the `LeanProofTerm`. Production callers (20a.5 authoring runtime, future CLI subcommand) invoke it during proof bundling. Not on the verification path — `LeanInstitution::query` never invokes it because the institution always operates on the raw bytes.
- `crates/eigenius-lean/src/startup.rs` — `pub fn register(service: &EigeniusService)` hook called from [`cli/src/main.rs`](../../cli/src/main.rs)'s `serve` subcommand path. Wraps `LeanInstitution::new(...)` in an `Arc` and calls `service.register_in_process_institution(...)` (Phase 20a.1's API). Verification-side code links into the **kernel binary** ([`cli/`](../../cli/)) — not the Deno orchestrator — so the verdict is a direct function call inside the kernel process, not a gRPC round-trip widening the TCB (per D28 §2.3 / §10.2).
- `crates/eigenius-lean/tests/end_to_end_smoke.rs` — commits a `LeanProofTerm` with hand-crafted bytes + a stubbed `mirror_iri` + `claim_iri`; asserts the resource lands tagged *verified*.
- `ontologies/lean/lean-institution.eigon.json` — the `Institution` resource, ExportFormats (`ef_lean_proof_payload`, `ef_lean_environment_summary`), QueryClasses (`qc_proof_check` AutoOnLoad+OnDemand + the OnDemand cluster), and the `LeanProofTerm` / `LeanProofPayload` / `LeanAxiomList` / etc. resource classes.

**Files modified.**
- [`cli/Cargo.toml`](../../cli/Cargo.toml) — add `eigenius-lean = { path = "../crates/eigenius-lean" }`.
- [`cli/src/main.rs`](../../cli/src/main.rs) — in the `serve` subcommand path, after constructing the `EigeniusService` and before calling `service.into_server().serve(...)`, call `eigenius_lean::startup::register(&service)`.

**Tasks.**
1. `LeanInstitution::query` procedure-dispatch table: `proof_check` → 20a.3's `check_proof` + stubbed correspondence; `which_axioms`, `proof_size`, `env_diff`, `discover_dependencies`, `discover_reductions` initially return `NotImplemented` (light up opportunistically — they share the dispatch table).
2. Chain-mirror translator: standalone `pub fn` that walks nanoda's parsed `Expr` tree and emits chain-CBOR `lean:LeanExpr` values per D40. Independent of the verification path. Round-trip test: bytes → `lean:LeanExpr` value → `serialize_resource` is deterministic.
3. End-to-end test: AutoOnLoad fires, institution dispatches, returns `Holds`, kernel tags *verified*. Uses a hand-crafted `LeanPackageMirror` resource (just a placeholder; correspondence check is stubbed so its content doesn't matter for this test).

**Acceptance.**
- `cargo test -p eigenius-lean --test end_to_end_smoke` passes — the toy `LeanProofTerm` lands as *verified*.
- Chain-mirror translator round-trips: bytes → `lean:LeanExpr` value → `serialize_resource` produces a deterministic CBOR encoding.

---

#### Phase 20a.5 — `eigenius-lean-runtime` authoring side (~1 week)

**Goal.** Stand up the authoring-side crate: `LanguageRuntime` impl for Lean, Dockerfile fragments, worker bootstrap, the first `LeanEnvironment` image with Lean toolchain + Lake (no Mathlib yet — that's 20b). Mirrors the shape of `eigenius-julia` ([crates/eigenius-julia/](../../crates/eigenius-julia/)).

**Independent of 20a.2/20a.3/20a.4** — can run in parallel once 20a.0b is settled (Dockerfile + worker bootstrap don't depend on the verification side).

**Files created.**
- `crates/eigenius-lean-runtime/Cargo.toml`.
- `crates/eigenius-lean-runtime/src/lib.rs`.
- `crates/eigenius-lean-runtime/src/runtime.rs` — `LeanLanguageRuntime: LanguageRuntime`. `language_id() = "lean"`, `build_environment_image` delegates to the substrate's builder with Lean-specific fragments, `run_script` dispatches to a Lake-driven worker, `call_method` lights up against declared `RuntimeMethodSignature`s (`lean_export` initially).
- `crates/eigenius-lean-runtime/src/dockerfile.rs` — Dockerfile fragments: `elan` install, pinned Lean toolchain (`elan toolchain install <version>`), Lake, worker binary copy, env-var stamping.
- `crates/eigenius-lean-runtime/src/conventions.rs` — shared constants (manifest-hash file path, env-var names) so Rust + Lake sides don't drift.
- `lean/runtime-worker/Lake/` (or `lakefile.lean`) — the Lake project for the worker binary.
- `lean/runtime-worker/Worker/Main.lean` — the worker binary. Thin Lake-driven Lean program handling CBOR-framed RPC over UDS (same protocol Julia's worker uses). Three verbs: `health`, `lean_export` (wraps `lean4export` against a `LeanProject` resource and returns the bytes), `dispatch_method` (call a Lean function declared as a `RuntimeMethodSignature`).
- `ontologies/lean/lean-runtime-classes.eigon.json` — `LeanProject` (subclass of `RuntimePackage`), `LeanPackage` (subclass of `RuntimePackage`), `LeanPackagePin` (subclass of `RuntimePackagePin`), `LeanEnvironment` (subclass of `RuntimeEnvironment`; adds `lean_permitted_axioms`, `lean_unpermitted_axiom_hard_error`, `lake_lockfile_hash`).
- `crates/eigenius-lean-runtime/tests/lean_export_test.rs` — end-to-end: build a `LeanEnvironment` image, run `lean_export` against a hand-rolled `LeanProject` containing a toy proof, confirm the returned bytes round-trip through 20a.3's `check_proof`.

**Files modified.**
- Workspace `Cargo.toml` — add `crates/eigenius-lean-runtime` member.
- [`orchestration/runtime-substrate-native/Cargo.toml`](../../orchestration/runtime-substrate-native/Cargo.toml) — add `eigenius-lean-runtime = { path = "../../crates/eigenius-lean-runtime" }`. The authoring-side runtime links into the **Deno orchestrator binary** via the napi-rs addon, same way [`eigenius-julia`](../../crates/eigenius-julia/) does today.
- [`orchestration/runtime-substrate-native/src/lib.rs`](../../orchestration/runtime-substrate-native/src/lib.rs) — add `register_lean_language_runtime(...)` napi export mirroring the existing `register_julia_language_runtime` shape.
- [`orchestration/src/runtime/loadAddon.ts`](../../orchestration/src/runtime/loadAddon.ts) — declare the `registerLeanLanguageRuntime(...)` method on the `RuntimeSubstrateAddon` interface.
- [`orchestration/src/main.ts`](../../orchestration/src/main.ts) — at startup, call `substrateAddon.registerLeanLanguageRuntime(workerProjectDir, baseImageRef, depotPath)`, mirroring the existing `registerJuliaLanguageRuntime` block.

**Tasks.**
1. Crate scaffold + Dockerfile fragments. Use `elan` to install a specific Lean toolchain version, copy the worker binary into the image, stamp env-var metadata.
2. Lake-driven worker binary: CBOR-framed RPC over UDS per the substrate's protocol; `lean_export` verb wraps `lean4export` invocation; `dispatch_method` verb supports method-signature dispatch for later QueryClasses.
3. `LeanProject`-subclass resource declarations.
4. `LeanEnvironment` subclass declaration with the new `lean_permitted_axioms` properties (default `["propext", "Classical.choice", "Quot.sound", "Lean.trustCompiler"]`).
5. End-to-end test: substrate-driven `lean_export` returns bytes; bytes round-trip through 20a.3.

**Acceptance.**
- Image build succeeds (or is skipped if Docker isn't available; gate behind `--features docker-spawner` like Julia does).
- `lean_export` round-trip test passes against the test environment.

---

#### Phase 20a.6 — `LeanMirrorGenerator` + first EigonFFI library (~1 week)

**Goal.** Implement the Lean mirror generator per D30 as substrate Rust code (matching D27/D29 pattern). Substrate's image-build pipeline invokes it; first generated EigonFFI library mirrors Core ontology classes; the library commits as a `LeanPackageMirror` resource baked into the `LeanEnvironment` image from 20a.5.

**Prerequisites:** 20a.0b (D30 spec), 20a.5 (env image to bake into).

**Files created.**
- `crates/eigenius-lean-runtime/src/mirror_gen.rs` — `LeanMirrorGenerator: MirrorGenerator` impl. Walks the chain's ontology layer, emits Lean source matching D30, commits the result as a `LeanPackageMirror` resource, returns the precompiled artifact for the image-build pipeline to bake in.
- `crates/eigenius-lean-runtime/src/mirror_gen/structure_emitter.rs` — emits Lean `structure` declarations from class declarations per D30.
- `crates/eigenius-lean-runtime/src/mirror_gen/codec_emitter.rs` — emits decode/encode helpers per class.
- `crates/eigenius-lean-runtime/src/mirror_gen/validator_emitter.rs` — emits constructor-level format-constraint refinement predicates (where Lean-expressible).
- `lean/common/EigeniusLeanCommon/lakefile.lean` — shared Lake package providing helpers the generated code calls (analogous to Julia's `EigeniusJuliaCommon`).
- `lean/common/EigeniusLeanCommon/src/EigeniusLeanCommon.lean`.
- `crates/eigenius-lean-runtime/tests/mirror_gen_test.rs` — golden-file tests against the Core ontology; confirms the generated Lean source compiles via Lake.

**Files modified.**
- `crates/eigenius-lean-runtime/src/runtime.rs` — `build_environment_image` invokes `LeanMirrorGenerator::generate` for the env's mirror class list, copies the generated source into the build context, commits the `LeanPackageMirror` resource alongside the image digest.

**Tasks.**
1. Closure walker (mirror of Julia's): visit `arg_types`, `class_types`, `allows_only` from seed classes, collect the structural closure.
2. Source emitters per D30's faithful-translation specification.
3. Image-build integration: the build pipeline writes the generated source into the build context; Lake precompiles; the resulting `.olean` files are baked into the env image.
4. Golden-file test: regenerate the Core mirror, diff against the checked-in golden, assert byte-equality.

**Acceptance.**
- Golden-file test passes (deterministic generation).
- The generated EigonFFI library compiles cleanly via Lake.
- The `LeanPackageMirror` resource lands in the chain with the right content hashes.

---

#### Phase 20a.7 — Three-part correspondence check (~4 days)

**Goal.** Replace 20a.4's stubbed correspondence check with the real D28 §5.5 implementation. Reads the `LeanPackageMirror` resource referenced by the `LeanProofTerm`, walks the mirror anchor, confirms structural correspondence between the mirror type used in the proposition and the claim's Eigon class.

**Prerequisites:** 20a.4, 20a.6.

**Files modified.**
- `crates/eigenius-lean/src/institution.rs` — replaces the correspondence stub with the three-part check:
  1. **Proof validity** — already passing from 20a.3's `check_proof`.
  2. **Mirror correspondence** — resolve the `mirror_iri`, read the `LeanPackageMirror`'s anchor (`source_layer`, `mirrored_classes`), verify the layer is ancestral-to-or-identical-with the claim's `claim_layer_hash`, confirm the proposition's mirror type corresponds structurally to the claim's class.
  3. **Anchor consistency** — verify the mirror's declared `library_content_hash` matches its actual content; confirm `source_layer` resolves.
- `crates/eigenius-lean/tests/correspondence_test.rs` — new tests:
  - Happy path: proof against a current-mirror class passes.
  - Compositionality regression: proof against an L₀ mirror remains valid for a claim in L₁ ⊒ L₀ when L₁ doesn't modify the mirrored classes; rejected with `FFIVersionMismatch` when L₁ does modify them.
  - Anchor-consistency failure: mirror with a tampered `library_content_hash` rejects.

**Acceptance.**
- All `correspondence_test` cases pass.
- `LeanProofTerm`s landing with malformed `mirror_iri` / `claim_layer_hash` produce structured `Verdict::Fails` diagnostics (`PropositionMismatch`, `FFIVersionMismatch`, etc., per D28 §9.1).

---

#### Phase 20a.8 — End-to-end test (capstone, ~3 days)

**Goal.** A small but real Eigon claim verified through the integrated stack. Tests every preceding sub-milestone against one passing scenario.

**Prerequisites:** 20a.4, 20a.7.

**Scenario.** A chain-side class with one required numeric property — pick a Core-mirrored class or commit a test class (`urn:test:Patient` with `urn:test:weight: float`). Commit an instance with a specific value. Run a Lean project that:
1. Imports the generated EigonFFI library (containing the mirrored `EigonFFI.Patient` structure).
2. States a proposition: e.g. `∀ p : EigonFFI.Patient, p.weight ≥ 0 → p.weight + 10 ≥ 10` (or any small inequality that compiles).
3. Proves it (one-line `omega` or `linarith` suffices).
4. Exports via `lean4export`.

Commit the resulting `LeanProofTerm` resource pointing at the chain-side `Patient` instance. AutoOnLoad fires, nanoda re-checks, correspondence check passes, the resource lands tagged *verified*.

**Files created.**
- `lean/research/capstone-proof/lakefile.lean` — the Lean project.
- `lean/research/capstone-proof/Capstone.lean` — the proof.
- `crates/eigenius-lean/tests/capstone_test.rs` — drives the full flow: substrate builds the env image, runs `lean_export` against the capstone project, commits the resulting `LeanProofTerm`, asserts the AutoOnLoad sequence completes with the resource tagged *verified*.

**Acceptance.**
- `cargo test -p eigenius-lean --test capstone_test` passes against the docker-substrate-backed test fixture.
- The closed audit chain is walkable end-to-end: from the *verified* claim through `LeanProofTerm` → `RuntimeInvocation` provenance (`lean_export`) → `LeanPackageMirror` → `LeanEnvironment` → image digest. Documented as a walk-through in the test's docstring.

---

### Phase 20b — Mathlib-scale operational landing

**Goal.** Extend the integrated foundation from 20a to Mathlib-dependent proofs. Distinct landing because Mathlib's concerns are operational, not design — the architecture from 20a is unchanged.

**Triggered by a consumer ask, not architecturally required.** Scope sized to that consumer when they appear; bullet list below is the canonical scope per D28 §11.2.

**Scope.**
- Image-build pipeline work for Mathlib's footprint (multi-gigabyte; needs incremental layer caching, `.olean` deduplication).
- Mathlib `LeanEnvironment` image with the full library installed.
- nanoda parsed-environment cache tuning — Mathlib's environment is large enough that the default LRU policy needs calibration.
- Resource-bound discipline (`max_wall_time_ms`, `max_memory_bytes`) calibrated against realistic Mathlib proofs.
- EigonFFI mirror generation against the broader ontology surfaces Mathlib proofs naturally reference (real-analysis classes, linear-algebra classes, etc.).
- `qc_environment_diff` structured implementation if the coarse "different image digests" view isn't enough for the consumer's audit needs.

**Acceptance.** A Mathlib-dependent proof for a representative claim re-checks within configured wall-clock + memory bounds; the verdict reproduces deterministically across runs.

---

### Phase 20 — Post-landing enhancements (not phases)

Per D28 §11.3 — self-contained pieces of work that layer onto the integrated foundation without changing its shape. None of these are phase-shaped; each is triggered by a specific need.

- **Lean4Lean as secondary cross-checker.** Soundness multiplier per the Venn-diagram argument (D28 §8.1). Wired as an alternate `check_proof` backend; `qc_proof_check` runs both and flags disagreements.
- **WASM-sandboxed checker.** Only if benchmarks justify it (D28 §8.2). Requires nanoda to compile cleanly to WASM and the WASM capability sandbox to handle Mathlib-scale memory.
- **Bidirectional Lean ↔ Julia bridge.** First concrete cross-institution Comorphism involving Lean ([D27 §6.2](d27-julia-institutions.md#62-verification-of-intervalarithmetic-outputs--concrete-d14-comorphism)). Requires EigonFFI to mirror `formulas:FormulaTerm` so a Lean proposition can quantify over FormulaTerm values; once that exists, the Comorphism is a clean identity-on-`FormulaTerm` plumbing job.
- **Verified-status type extension** (D28 §12 remaining question / `life-science-requirements.md` §16.4) — kernel extension to express verification status in the type system. Non-trivial EigenTT extension; defer until a concrete pipeline consumer asks for it.

### Phase 20 — Open questions (per D28 v2 §12)

The original D28 carried nine open questions; D28 v2 resolved five structurally and Phase 20a closed two more (axiom allowlist default and Lean-version upgrade policy — see [d28-lean-4-as-institution.md §12](d28-lean-4-as-institution.md#12-open-questions) "Resolved at end of Phase 20a"). Two remain:

1. **Parallel verification institutions.** If Rocq lands, dispatch by user preference / contract / explicit IRI? Defer until Rocq is real.
2. **Kernel extension for verification status in types** (D28 v1 question #9). Defer until the first pipeline that would benefit asks for it.

### Phase 20 — Test plan

- **20a.1.** Echo-stub in-process institution registers via the startup-hook, AutoOnLoad fires through `InstitutionRuntime`, returns the echo response.
- **20a.2.** Chain commit + inspect round-trip of a hand-encoded `lean:LeanExpr` value.
- **20a.3.** Toy proof JSON admitted by nanoda; deliberately broken proof rejected with structured diagnostic.
- **20a.4.** Toy `LeanProofTerm` resource lands tagged *verified*. Chain-mirror translator produces deterministic CBOR.
- **20a.5.** Substrate-driven `lean_export` against a hand-rolled `LeanProject` returns bytes that round-trip through 20a.3's checker.
- **20a.6.** Golden-file test on the Core EigonFFI mirror passes; Lake compiles the generated source.
- **20a.7.** Happy-path correspondence + compositionality regression (anchor-ancestral and anchor-divergent cases) + anchor-consistency failure each produce the right verdict.
- **20a.8.** Capstone — full audit chain walkable end-to-end against a real proposition.
- **20b.** A representative Mathlib-dependent proof re-checks within configured bounds.

### Phase 20 — References

- [D28 — Lean 4 as Verification Institution](d28-lean-4-as-institution.md) — full specification (v2: two-landing structure, in-process runtime kind, verbatim bytes + chain-mirrored propositions).
- [D26 — Runtime Substrate](d26-runtime-substrate.md) — substrate the authoring side runs on.
- [D27 — Julia Institutions](d27-julia-institutions.md) — the multi-institution precedent.
- [D29 — Eigon → Julia Faithful Translation](d29-eigon-julia-mirror-spec.md) — D30's sibling-pattern reference.
- [D32 — Chain-Mirrored EigenTT Inductives](d32-chain-mirrored-mini-tt-inductives.md) — D40's sibling-pattern reference.
- [D14 — Institution Realisation](d14-institution-realisation.md) — institution protocol.
- D30 (to be written in 20a.0b) — Eigon → Lean faithful translation specification.
- D40 (to be written in 20a.0) — Chain-mirrored Lean expressions specification.
- `references/nanoda_lib/` — vendored Lean term checker.
- [Type Checking in Lean 4](https://ammkrn.github.io/type_checking_in_lean4/) — nanoda's accompanying specification book.

---

## Phase 21 — Life-Science Worked Examples

**Goal:** Bring up the four life-science institutions outlined in [`life-science-requirements.md`](life-science-requirements.md) (`I_Dock`, `I_ADMET`, `I_Assay`, `I_PK`) end-to-end through real Julia institutions, real Lean proofs (where the *verified* warrant is wanted), and the EigenQL surface. Validates the platform against the original life-science motivation; produces the worked examples cited throughout the design corpus (the EIG-0042 cross-fiber discrepancy, the dock→assay comorphism translation, the PK ODE bound proof).

**Duration estimate:** open-ended; each life-science institution is roughly 3–5 weeks given the prerequisites are in place.

**Prerequisites:** Phase 11 (inductive types + Map/Reduce + decide procedures + Comorphism class — all of [`life-science-requirements.md`](life-science-requirements.md) §16's Tier 1 + Tier 2), Phase 19 (Julia institutions: 19g Catalyst for reaction-network dynamics, 19h DiffEq for ODE integration, 19f IntervalArithmetic for certified bounds, 19d Symbolics for derivation, 19e JuMP for parameter fitting). Phase 20 (Lean) is required only for the *verified* path on life-science computations; the *derived*-only path for the four institutions doesn't strictly need it.

**Drives:** [`life-science-requirements.md`](life-science-requirements.md). Capstone for the platform vision laid out in the original requirements doc.

### Phase 21 — Deliverables (per institution)

- **`I_Dock` institution:** typed declarations for poses, scoring functions, conformational families; AutoOnLoad QueryClasses validating pose-scoring consistency and ensemble representativeness; OnDemand queries for "best pose by score" / "structural neighbours within RMSD ε".
- **`I_ADMET` institution:** structure-to-property mappings, ML ensemble predictions with confidence bounds; AutoOnLoad QueryClasses gating prediction admission on ensemble disagreement thresholds.
- **`I_Assay` institution:** typed assay protocols, dose-response curves, replicate relationships; AutoOnLoad QueryClasses validating curve-fit goodness; OnDemand queries for "compounds active in assay X" / "EC₅₀ for compound Y".
- **`I_PK` institution:** compartmental models declared as `ReactionNetwork` (Phase 19g) compiled to `OdeSystem` (Phase 19h); AutoOnLoad QueryClasses validating ODE solutions against measured concentrations within tolerance; OnDemand `qc_pk_predict_cmax` / `qc_pk_predict_auc` etc.
- **Cross-institution comorphisms:** `Dock → Assay` (binding affinity ΔG → predicted IC₅₀), `ADMET → PK` (predicted clearance → compartmental parameters), `PK → IntervalArithmetic` (PK trajectory → certified concentration bounds via interval extension); declared as D14 `Comorphism` resources.
- **Worked notebook:** the EIG-0042 cross-fiber discrepancy scenario from [`life-science-requirements.md`](life-science-requirements.md) §1, end-to-end through the four institutions and the comorphism translations.

### Phase 21 — Open scope items

- Bayesian inference (`Turing.jl`) for posterior over PK parameters; deferred until a domain demand is concrete.
- SDE / jump-process kinetics (Phase 19i+) for stochastic low-copy-number signaling pathways; deferred.
- Algorithm-correctness Lean proofs for life-science computations (the [D27](d27-julia-institutions.md) §6.3 bridge); deferred.
- Domain ontology curation (compound libraries, target families, assay panels); a Phase 21 follow-on rather than core.

### Phase 21 — References

- [`life-science-requirements.md`](life-science-requirements.md) — driving requirements
- [D27 — Julia Institutions](d27-julia-institutions.md) — Julia institutions this builds on
- [D28 — Lean 4 as Verification Institution](d28-lean-4-as-institution.md) — verification track for the life-science claims that warrant it
- [D26 — Runtime Substrate](d26-runtime-substrate.md) — what the Julia + Lean tracks both ride on

---

## 9. Design Documents

The following design documents must be written and reviewed before the phase that depends on them. Each resolves open questions from the architecture document (§14) and makes decisions that code will implement.

| # | Document | Resolves | Required before | Estimated length |
|---|----------|----------|-----------------|-----------------|
| D1 | **Eigon Serialization Format** | **COMPLETED** — `docs/design/d1-eigon-serialization-format.md`. Eigon-JSON format, IRI identity, three-layer type system (data types/formats/content types), validation rules, canonical form, core ontology in `ontologies/core/core-ontology.json` | Phase 0 | Done |
| D2 | **EigenQL v1 Specification** | **COMPLETED** — `docs/design/d2-eigenql-specification.md`. Full EBNF grammar, lexer spec, type checking rules, aggregation (COUNT/SUM/AVG/MIN/MAX), GROUP BY, ORDER BY, LIMIT/OFFSET, DISTINCT, NOT EXISTS, dot-path navigation, error format | Phase 1 | Done |
| D3 | **Program Model and Component Interface** | **COMPLETED** — `docs/design/d3-program-model.md`. Programs as typed expressions (not programs), 12 expression forms mapping 1:1 to EigenTT, Map/Reduce as language primitives, automatic parallelism from data dependencies, two-tier component model (built-in + WASM), ESL surface syntax (future) | Phase 2 | Done |
| D4 | **Storage Key Encoding** | **COMPLETED** — `docs/design/d4-storage-key-encoding.md`. Key encoding for RocksDB/TiKV, column families, layer chain persistence, index layout, TiKV compatibility | Phase 3 | Done |
| D5 | **gRPC API Specification** | **COMPLETED** — `docs/design/d5-grpc-api-specification.md`. RPC definitions, streaming query, context management, error codes, authentication, CLI/orchestration integration | Phase 3 | Done |
| D6 | **Execution Architecture and Durability** | **COMPLETED** — `docs/design/d6-execution-architecture.md`. Kernel↔orchestrator boundary, DAPR integration, durable workflows, activity dispatch, reasoning trace ownership, MCP server placement | Phase 4 | Done |
| D6b | **Reasoning Trace Schema** | **COMPLETED** — `docs/design/d6b-reasoning-trace-schema.md`. ComponentTrace, ProgramTrace, ObservationTrace, VerificationTrace classes. Provenance chain, epistemic status (observed→derived→verified), universe stratification, trace-based memoization | Phase 4 | Done |
| D7 | **ESL Surface Syntax** | **COMPLETED** — `docs/design/d7-esl-surface-syntax.md`. Two-layer design (HCL-style structural + ML-style expressions), namespace aliases, program/class/property/resource syntax, EBNF grammar | Phase 4.5 | Done |
| D8 | **CompleteJson Component** | **IMPLEMENTED** — `docs/design/d8-complete-json-component.md`. Structured LLM output via JSON Schema generated from ontology classes. Bijective short-name mapping with bijectivity check in ValidateProgram. Enums (`allows_only`), nested objects (`class_types`), union types (multiple `class_types` with `_type` discriminator). Template data type for prompt validation. `GetSchema` RPC. Patent demo end-to-end | Phase 7 | Done |
| D9 | **NbE/Executor Unification** | **COMPLETED** — `docs/design/d9-nbe-unification-and-type-extensions.md`. Capability modes (Pure/Read/IO), type theory extensions (Id, DecEq, NativeDecide, universes), complete ground type resolution, trace storage architecture, crash recovery, trace pruning (proofs-as-programs) | Phase 5 | Done |
| D10 | **Grothendieck Institution Protocol** | **SUPERSEDED by D14** — `docs/design/d10-grothendieck-institution-protocol.md` is a redirect to D14. Original D10 surface (FiberReasoner, InstitutionRegistry, ComorphismRegistry, validator-side morphism dispatch, FiberQuery/DiscoverMorphisms RPCs) retired in Phase 12. Categorical motivation (Eigon as shared signature category, EigenTT as kernel service, Lean as verification institution) survives in D14. | Phase 6 (orig) / Phase 12 (D14 redo) | Done |
| D11 | **Codata, Streams, and Resumable Execution** | **COMPLETED** — `docs/design/d11-codata-streams.md`. Coinductive types via copatterns (Abel et al. 2013), stream semantics, tasks as codata, trace-driven replay, concurrent task model, ESL codata syntax, guardedness checking | Phase 9b | Done |
| D12 | **WASM Extensibility** | WASM module lifecycle, host function interface (kernel → WASM), resource serialization across the boundary (Eigon-CBOR), capability levels → WASM import sets (pure/read/IO), integration with `ComponentRegistry` and (under D14) the `Institution` trait via the `eigenius-institution-d14` WIT world, registration via ontology resources, fuel/memory limits, SDK crate design. Merges the previously separate D12 (Capability SDK) and D13 (Wire Format) — the interface and wire format are inseparable. Resolves §14 open question on capability protocol | Phase 8 | 14–18 pages |
| D13 | **Durable Kernel State** | **COMPLETED** — `docs/design/d13-durable-kernel-state.md`. `serve --db` flag, seeded bootstrap with drift-refusal, commit-through to `RocksStore`, WASM + institution re-registration on restart, persistent trace store via `BackendTraceStore`. Prerequisite for D11/Phase 9b. (The previous D13 — Wire Format — was merged into D12.) | Phase 9a | Done |
| D14 | **Institution Realisation** | **COMPLETED** — `docs/design/d14-institution-realisation.md`. Ontology-first replacement for D10's procedural institution surface. Triadic Comorphism (export_format, transformation, import_format, exact); InstitutionIndex derived from chain scan; InstitutionRuntime of Institution trait impls; Verdict-shaped QueryClasses with OnDemand / AutoOnLoad / Decidable dispatch roles; four-step comorphism pipeline; post-translation validation invariant. | Phase 12 | Done |
| D14b | **Security Model** | Authentication, authorization, namespace delegation policy, namespace delegation depth, capability trust chain and authenticity (resolves §6.4, §13.2, and §14 open questions). Originally slated for D14; renumbered when D14 was taken by Institution Realisation. | Phase 13 | 10–15 pages |
| D15 | **Ontology Versioning & Evolution** | Semantic versioning policy for ontology layers, backward compatibility rules, ontology combination semantics, ESL extension mechanism (resolves §13.1 and §14 open questions) | Phase 6+ | 8–10 pages |
| D16 | **Observability & Operational Tooling** | Structured metrics, tracing spans, query plan explanation, program execution step-through, reasoning trace streaming for live monitoring (resolves §13.3) | Phase 13 | 6–8 pages |
| D17 | **Capability Versioning** | How capability implementations are versioned, version mismatch handling, backward compatibility obligations, upgrade path for Foundation capabilities across kernel releases (resolves §14 open question) | Phase 8 | 6–8 pages |
| D18 | **Ontology-as-Types Resolution** | **COMPLETED** — `docs/design/d18-ontology-as-types-resolution.md`. `find_sigma_field` walks the layer chain through `resolve_class_type` instead of silently collapsing `EigonClass` to `Val::Set`. Introduces `CheckCtx` (bundling `rho`/`gamma`/optional layer/per-check type cache) threaded through all checker entrypoints. Adds inference-mode rules for `Construct`, `EigonResource`, `Template`, `IdJ`, `Refl`, `NativeDecide`, `DecEq`. Rejects no-layer EigonClass resolution explicitly rather than returning a weakened type. Closes the #12 high-priority correctness hazards. Prerequisite for most of D19. | Phase 10a | Done |
| D19 | **Inductive Types in EigenTT** | **DRAFT** — `docs/design/d19-inductive-types.md`. Single (non-mutual, non-nested) strictly-positive inductive types + sized types (#16). Declaration form, positivity checker, recursor/eliminator derivation, iota-reduction, sized termination for inductive/coinductive interaction. Deferred: mutual (#20), nested (#21), indexed families (#22). | Phase 11b | Draft |
| D20 | **Layer Reconciliation** | **DRAFT** — `docs/design/d20-layer-reconciliation.md`. Pushout-of-a-span framing for layer merge (schema-level pushout in the category of ontology presentations + Σ migration of instance functors). Six typed resolution strategies — `Witness`, `Rename`, `KeepBoth`, `KeepOne`, `KeepNeither`, `Restructure` — each a transformation applied to the input span before pushing out. Three-stage conflict taxonomy (`SchemaConflict`, `EquationConflict`, `InstanceConflict`). Cascade impact analysis with kernel-enforced acknowledgment gates. `MergeComorphism` resource for the `Witness` strategy (sibling to the cross-institution Comorphism; D20 owns the naming-distinction internally). gRPC `submit_resolution` / `preview_cascade`. Supersedes D13's v1 drift-refusal (ontology migration is a degenerate single-resource witnessed merge). Sub-milestones 15a–15g sequenced for incremental delivery. | Phase 15 | Draft |
| D21 | **Task Traces and Checkpointing** | **COMPLETED** — `docs/design/d21-task-traces-and-checkpointing.md`. Per-task positional `(session_id, task_id, step_seq)` trace keys replace the Phase-9a content-address cache for IO components (determinism-gated — Pure/Read keep the memo for cross-task reuse). `components:Checkpoint` built-in persists program-declared state atomically via `write_batch`. `ListTasks` / `GetTaskStatus` / `CancelTask` RPCs + `at_layer` on read RPCs. Startup resume sweep (`ResumeConfig`, `ResumeState`) rehydrates pinned layer chains and re-executes `Running`/`Suspended` tasks with bounded parallelism. Single hardwired session (`Uuid::nil()`) in 9b-iii with multi-session as a Phase-14 surface expansion. | Phase 9b-iii | Done |
| D23 | **Out-of-Core Layer Architecture** | Topology / content split, per-layer shadowing bloom + bounded `BloomCache`, two-pool ARC cache, time-travel as standard resolve rooted at the target layer, DAG branching primitive, multi-session writes, reachability-based GC with trace pinning, branch pruning, indexed resource access for queries. The structural rework that lifts the kernel's working-set bound from "graph size" to "cache size." | Phase 14 | 12–16 pages |
| D24 | **Schema Versioning Policy** | **ACTIVE** — `docs/design/d24-schema-versioning.md`. Single integer schema version stamped per RocksDB instance, refused-on-mismatch boot policy, contributor-side migration discipline. Orthogonal to the D13 seed-manifest check (which catches ontology-content drift). Introduced alongside the Phase 14 merge; schema version starts at 1 with the cumulative Phase 14 on-disk shape. | Phase 14 | Active |
| D25 | **Chain Consolidation** | **DRAFT** — `docs/design/d25-chain-consolidation.md`. Linear-range chain consolidation: collapse a contiguous ancestral range `[from..to]` into one resolve-equivalent layer with `parent = from.parent`. Top-of-stack algorithm (head→root walk, single-pass linear in defined-IRI count). Atomic commit per D23 §6.3. Resolve-equivalence under head substitution as the load-bearing invariant. Trace-pin refusal policy in v1 (re-pointing and invalidation deferred to v2). Refuses to consolidate across merge nodes in v1; v2 sketches multi-parent consolidation that preserves Phase 15 resolution decisions via `consolidated_resolutions` records. Bloom-cache eviction for collapsed layers. Sub-milestones 17a–17e. Distinct from merge (combines branches) and GC (drops unreachable layers). | Phase 17 | Draft |
| D26 | **Runtime Substrate** | **DRAFT** — `docs/design/d26-runtime-substrate.md`. Language-agnostic substrate for hosting external language toolchains inside Eigenius with full provenance. `LanguageRuntime` trait, parent ontology classes (`RuntimeScript`, `RuntimePackage`, `RuntimeEnvironment`, `RuntimePackageMirror`, `RuntimeInvocation`, `RuntimeMethodSignature`, `RuntimePackagePin`), image-vs-graph boundary, deterministic image-build pipeline with digest capture, worker pool + sandbox, mirror-anchor compositionality, `RunRuntimeScript` / `CallRuntimeMethod` substrate components. Cross-language wire format = CBOR + RFC 8746 typed-array tags. | Phase 18 | Draft |
| D27 | **Julia Institutions** | **DRAFT** — `docs/design/d27-julia-institutions.md`. First concrete substrate instance plus reference institutions wrapping Julia libraries under D14. `JuliaScript` / `JuliaPackage` / `JuliaEnvironment` / `JuliaPackageMirror` / `JuliaInvocation` / `JuliaMethodSignature` / `JuliaPackagePin` subclasses. `eigon-julia-gen` mirror generator. Five reference institutions: `Symbolics`/`ModelingToolkit`, `JuMP`, `IntervalArithmetic`, `Catalyst`, `DiffEq` (ODEs). Future Lean / Julia bridge sketch (interval-bound proof obligations). | Phase 19 | Draft |
| D28 | **Lean 4 as Verification Institution** | **DRAFT v2** — `docs/design/d28-lean-4-as-institution.md`. Lean 4 as Eigenius's first verification institution under D14. `LeanProofTerm` resource (verbatim Lean export bytes + chain-mirrored proposition) / `LeanEnvironment` (carries axiom allowlist + image digest) / `LeanProject` / `LeanPackage` / `LeanPackageMirror` resource classes. Chain-mirrored `lean:LeanExpr` / `lean:LeanLevel` / `lean:LeanName` inductives (D40). EigonFFI static-mirror generator implemented as substrate Rust code (D30). Three-part correspondence check (proof validity + mirror correspondence + anchor consistency). Substrate-hosted authoring side (`lean4export`, mirror generator, environment images) + in-process verification side (nanoda_lib re-check) via the new `InProcess` runtime kind. Two landings per §11: 20a complete first institution, 20b Mathlib-scale operational. | Phase 20 | Draft v2 |
| D29 | **Faithful Translation Specification — `eigon-julia-gen`** | The mapping from Eigon class structure to Julia struct / abstract-type-hierarchy / constructor-validation form. Pinned per generator version; the load-bearing TCB artifact alongside the generator binary. | Phase 19a.4 (v1.2 draft) | Draft v1.2 |
| D30 | **Faithful Translation Specification — Eigon → Lean** | Sibling to D29. The mapping from Eigon class structure to Lean `structure` / coercion-instance / refinement-predicate form. Pinned per generator version; the load-bearing TCB artifact alongside nanoda_lib. The mirror generator itself is substrate Rust code (matching the D27/D29 pattern), not a separately-runnable binary — auditability comes from the deterministic output spec. | Phase 20a.0b | 10–14 pages |
| D31 | **External Institution Authoring & Dispatch Lifecycle** | The end-to-end lifecycle for institutions whose `runtime` is `external`: mirror generation CLI, institution registration, kernel-emits-request / orchestrator-services-IO dispatch routing. Generator-agnostic; per-language faithful-translation specs (D29 / D30 / D32) plug in independently. Implementation milestone: Phase 19a.5 (framework) + Phase 19a.6 (IntervalArithmetic, the framework's integration test). | Phase 19a.5 (v1.1 draft) | Draft v1.1 |
| D32 | **Faithful Translation Specification — `eigon-rust-gen`** | The mapping from Eigon class structure to Rust struct / trait / impl form for WASM institution authoring. Tracked in [#41](https://github.com/eigenius/eigenius/issues/41). | TBD | Planned |
| D33 | **Partial-Order Chains and Commutativity-Aware Semantics** | **DRAFT** — `docs/design/d33-partial-order-chains.md`. Reframes the chain as a partial order of layers rather than a sequence; two-hash identity (position vs content) generalises D25 §11.0; supporting-layer property indexes "what this layer depends on"; **anchored-commit cache** (the v1 deliverable — content-addressed commit reuse keyed on `(content_hash, supporting_content_hash)`) generalises the notebook cell-output reuse pattern to any deterministic content generator. Partial-order operations (linearization-independent reads, distributed-execution forward compatibility) sketched for v2. Anchored-commit cache shipped in support of D34 §6 cell-cache visibility; invalidation on consolidation wired in D34 Phase 6. Full v1 storage path under Phase 20. | Phase 20 | Draft |
| D34 | **Notebook Chain Workspace** | **COMPLETED** — `docs/design/d34-notebook-chain-workspace.md`. Rail-based workspace shell on top of D22 exposing every kernel chain surface a working operator needs. Eleven phases (§16) — branch picker, branches / history / tags / merge / compaction / tasks / institutions / topology / GC / health rail destinations. Forced kernel gaps §G.1–§G.8: `MergeInfo` honesty fix, tag storage + RPCs (with tag-rooted GC), `MergeBranches` / `PreviewMerge`, `EstimateGc` / `RunGc`, `branch_advanced` consumer surface, branch-scoped cursored history (§G.6, deferred), `^auto-` prefix reservation (§G.7, deferred), enriched `InstitutionInfo` (§G.8). Cross-cutting kernel fixes uncovered during the rollout: `branch_contexts` cache invalidation on consolidation, anchored-commit cache invalidation on consolidation per D33 §3, per-layer `byte_size` on `LayerHandle` for GC reclaim estimates. | Concurrent with Phase 12+ (track parallel to D22) | Done |
| D40 | **Chain-Mirrored Lean Expressions** | Sibling to D32 (chain-mirrored EigenTT inductives + FormulaTerm). The `lean:LeanExpr` (10 ctors mirroring nanoda's `Expr` minus `Local`) / `lean:LeanLevel` (5 ctors) / `lean:LeanName` (3 ctors) chain InductiveTypes. Deterministic encoder/decoder semantics between nanoda's parsed `Expr` and the chain-CBOR shape. Version discipline against `lean4export` semver. The queryable-proposition counterpart to the verbatim proof-term bytes carried by `LeanProofTerm`. | Phase 20a.0 | 8–12 pages |
| D42 | **Out-of-Core Query Execution** | **STUB** — `docs/design/d42-out-of-core-query-execution.md`. Buffer-pool abstraction over the storage backend, hash join with spill, external sort, spillable group-by accumulators, spill-aware cost model, per-query memory budget. The operator-side rewrite that lets EigenQL handle result sets larger than memory. Builds on D23's storage abstractions. Picks up the Phase 16 spec previously sketched as D24 before that number was claimed by Schema Versioning. | Phase 16 | 8–10 pages |
| D43 | **Text and Vector Retrieval in EigenQL** | **DESIGN COMPLETE** (§§2–7 substantive; §8 resolved) — `docs/design/d43-text-and-vector-retrieval.md`. Implementation plan at `docs/design/d43-implementation-plan.md` (M1–M9, ~12–18 weeks scope). Built-in text and vector retrieval primitives in EigenQL: ESL `text_index` / `vector_index` first-class Resource declarations (`core:TextIndex` / `core:VectorIndex`), `TEXT_MATCH` / `VECTOR_NEAR` / `EMBED` / `RRF` / `TOP K BY` surface, custom layer-aware inverted index in RocksDB (Phase 14h-aligned per-Index keying; chain-aware BM25; ~10 transitive deps via `roaring` + `unicode-segmentation` + `rust-stemmers`), zero-copy CBOR vector segments with per-VectorIndex `strategy: flat | hnsw | auto` and additive HNSW CBOR fields (v1 envelope ~10M vectors/segment, covers life-science ontologies and bounded PubMed extractions; v2 quantisation / IVF / out-of-core HNSW for 100M+), Embedder as a D3 Component with content-addressed cache + post-Load sweep, atomic-reindex policy for embedding-model upgrades. Implementation bundles a RocksDB column-family rollout (`cf_text`, `cf_vec`, `cf_embed_cache`) and a D24 schema-version bump under the no-legacy-DBs policy. Promotes the sketch from D35 §7.4 to a real spec; hard dependency for the SE knowledge-graph rollout. | TBD (D35-dependent) | 30+ pages |
| D44 | **Automatic Data Lifecycle Management** | **STUB / RESERVED** — `docs/design/d44-automatic-data-lifecycle-management.md`. Policy layer that decides *when* consolidation (D25), garbage collection (D23 §5.7), retrieval-index cache eviction (D43), and trace pruning (D9) fire, plus how the policies interact. Mechanisms already exist across the dependency docs; D44 unifies trigger policies and ties them to observable system state. Deliberately reserved as a stub pending production-observation-driven scoping — pre-emptive specification would lock in shapes for problems we don't yet understand. | TBD (production-observation-driven) | TBD |

**Reference documents** (analysis rather than specification):

| File | Purpose |
|------|---------|
| `docs/design/life-science-requirements.md` | Systematic audit of life-science representation needs. Drives the Phase 10–12 kernel-extension sequencing, the Phase 19 Julia-institution selection (Catalyst + DiffEq for PK / signaling, IntervalArithmetic for certified bounds), and the Phase 21 worked-example shape. Source of record for which kernel and institution extensions each life-science shape requires. |

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
| EigenTT → Rust port complexity | Delays Phase 2 type system | Port incrementally, test against Haskell reference. Start with a minimal subset (Pi, Sigma, sums). |
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
