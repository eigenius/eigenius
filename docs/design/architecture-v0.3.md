# Eigenius Architecture: Execution Context and Foundations

**Status:** Draft — Working Document  
**Version:** 0.3  
**Date:** April 2026

---

## 1. Project Overview

Eigenius is an open-source platform for **AI-driven science and engineering**, built as a self-describing, schema-first AI orchestration substrate. Its purpose is to anchor and persist knowledge and insights in a way that avoids the epistemic risks of contemporary LLMs — hallucination, ungrounded confidence, opaque reasoning, and the inability to distinguish verified conclusions from plausible-sounding text.

The defining architectural commitment is that processing pipelines, domain models, and LLM reasoning traces are all expressed in — and introspectable through — the same typed knowledge graph. The system can formally reason about its own reasoning by design, not as an afterthought.

### 1.1 Motivation: Epistemic Integrity for Frontier Research

Large language models produce text that reads like knowledge but carries no epistemic warranty. An LLM can write a convincing paragraph about protein folding dynamics or quantum decoherence that is subtly wrong, and there is no structural way to distinguish that output from a correct derivation. The confidence sounds the same. The citations might be real. But the reasoning chain is opaque and probabilistic — the consumer is trusting a statistical pattern, not a verified argument.

For frontier science and engineering — quantum physics, genomics, drug discovery, materials science, climate modeling — this ambiguity is not an acceptable trade-off. When a researcher asks "does this molecule bind to this receptor?" or "does this quantum error correction code achieve the threshold theorem?", the answer might originate from an LLM-driven literature analysis, a computational simulation, or a formal derivation from first principles. These are epistemically different answers. Eigenius makes the difference visible, queryable, and enforceable rather than flattening everything into text that sounds equally confident.

The architecture maintains a separation between four epistemic categories:

**Declared knowledge** — axioms, definitions, and design decisions asserted by humans. The core ontology, domain ontologies, program specifications, and prompt templates are all declarations. A declaration is an assertion anchored in human thought and intent — "I define this to be so." It carries no claim about external reality and no computational derivation. Every system must start from declarations; they are the epistemic foundation.

**Observed knowledge** — facts recorded in the knowledge graph from external reality with tracked provenance. A measured binding affinity, a published experimental result, an uploaded document, a sensor reading. The system vouches for the provenance ("this is what was recorded, and here is where it came from"), not for the truth of the observation.

**Derived knowledge** — conclusions that follow from declared and observed knowledge through typed processing programs. The type system guarantees the program is well-formed. Reasoning traces record exactly which inputs produced which outputs through which steps. The derivation is replayable, queryable, and auditable. You can ask "what assumptions does this conclusion depend on?" and receive a typed, complete answer.

**Verified knowledge** — derivations that carry formal proofs, checked by constructive type theories registered as capabilities (§9.7). A proof term attached to a resource is not a confidence score or a citation — it is a machine-checked certificate that the conclusion follows from the premises by the rules of the type theory. If the proof checks, the derivation is correct in the mathematical sense, not the probabilistic sense.

The progression from declared → observed → derived → verified maps to increasing epistemic strength and the universe stratification in the reflection layer (§11.3). Resources declare their epistemic status via base classes (`DeclaredResource`, `ObservedResource`, `DerivedResource`, `VerifiedResource`) — see design doc D6b for the full schema.

### 1.2 Named Components

| Name | Role |
|---|---|
| **Eigenius** | The platform |
| **Eigon** | The canonical typed data format |
| **ESL** (Eigenius Schema Language) | Human-friendly surface syntax over Eigon |
| **EigenQL** | Typed semantic query language (stratified Datalog with aggregation). See design doc D2 for full specification. |
| **Rust + Verus** | Kernel implementation language with embedded formal proofs |
| **Lean 4** | Formal metatheory specification track |

### 1.3 Core Architectural Principles

**Self-description.** The Core Ontology that defines all other ontology concepts is itself expressed using those same concepts. The system bootstraps from a small fixed set of primitives, and everything else — including processing pipelines, capabilities, and reasoning traces — is expressed within the same knowledge graph.

**Type-first.** Programs are typed expressions. Validation is type-checking. A processing pipeline that passes validation carries a formal guarantee that its bindings are type-compatible and its required inputs are provably available. This is not a runtime check — it is static verification before execution begins.

**Self-reflection as load-bearing architecture.** The same graph that models a domain also captures the LLM's reasoning about that domain. Reasoning traces are first-class typed resources, not opaque logs. The system can query across domain and reasoning data uniformly using EigenQL.

**Formal foundations.** The Rust kernel is designed from the outset to have a provable correspondence with a Lean 4 formal specification. Critical kernel algorithms carry Verus proof annotations. The architecture is designed so that formal verification can deepen over time without requiring redesign.

---

## 2. Technology Stack

### 2.1 Rust Kernel

The kernel is implemented in Rust with Verus proof annotations on correctness-critical components. It compiles to a **native Rust binary** for the primary service deployment and optionally to **WebAssembly** for browser and edge targets. The kernel is responsible for structural semantic concerns: Eigon type-checking, layer management, capability dispatch, and the reflection layer. It is deliberately *not* responsible for the semantics of any particular hosted language — including EigenQL and the program expression language — which are registered capabilities.

**WASM as a capability sandbox.** The kernel's primary use of WebAssembly is not as its own compilation target but as an **isolation mechanism for untrusted capability code**. The capability protocol (§9) allows domain ontologies to register custom evaluators, validators, and parsers. When these are provided by third parties or untrusted domain authors, the kernel instantiates them as WASM modules via Wasmtime, providing memory isolation, bounded execution time, and a controlled interface surface. The kernel runs native; untrusted capabilities run sandboxed. This is the same pattern used by Envoy (proxy filters), Fastly (edge compute), and Figma (plugin isolation) — a trusted native host executing untrusted portable code within well-defined resource bounds.

**Kernel responsibilities:**

- Core Ontology bootstrap — hand-initialized loading of the self-describing Core Ontology and Foundation Layer (see §2.5)
- Eigon structural type system — runtime representation of classes, properties, datatypes, and their relationships; validation of resource structure against class constraints
- Resolver system — IRI scheme dispatch, namespace validation, working context management
- Layer management — layer creation, commit, immutability enforcement, stack resolution
- Capability dispatch — registration, class-anchored lookup, lifecycle management, sub-context instantiation
- Distributed storage interface — the kernel's abstraction over the storage layer
- Reflection layer — reasoning trace representation, provenance tracking, universe stratification
- Execution context management — snapshot binding, constraint enforcement, transaction coordination

**Verus annotations are applied to:**

- The Eigon structural type checker — soundness of class/property/datatype validation is the core formal guarantee
- Layer system — immutability enforcement, resolution order correctness, commit atomicity
- Capability dispatch — correct class-anchored lookup, snapshot-consistent resolution, sub-context isolation
- Core Ontology — immutability enforcement, namespace protection
- Execution context — snapshot isolation invariants, constraint enforcement

### 2.2 Host Layer

The kernel exposes a **service API** consumed by an **orchestration layer**. In the primary server deployment, the kernel runs as a native Rust service with RocksDB for persistent storage (TiKV for distributed deployments), and exposes its operations over gRPC. The orchestration layer — implemented in TypeScript targeting **Deno** as the preferred runtime — sits above the API boundary and handles program execution coordination, LLM adapter management, the MCP server surface, and developer tooling. The orchestration layer is an API client of the kernel service, not a WASM host.

Deno is chosen for native TypeScript execution (no build step), an explicit permission model that mirrors the capability protocol's design philosophy, and a direct path from server deployment to edge deployment (Deno Deploy) from the same codebase. Node.js is supported as a fallback runtime — the orchestration layer's TypeScript code is runtime-agnostic.

**The kernel service API (four operations):**

- **Load** — add ontology resources to the working context, receive structural validation results
- **Query** — dispatch an EigenQL query to the registered query capability within the current context, receive typed results
- **Validate** — dispatch a program specification to the registered program validation capability, receive a typed validation report (including partial evaluation results)
- **Reflect** — record a reasoning trace as a typed resource, query across reasoning and domain uniformly

**Orchestration layer responsibilities:**

- Program execution and orchestration — async runtime, component lifecycle, execution coordination
- Component implementations — LLM adapters, document processors, external services (see §2.3)
- MCP server — exposes the kernel's four operations as tools for LLM agents (see §2.3)
- Developer tooling — editors, visualizers, ontology browsers, language servers

**Kernel-internal services (native Rust, no API boundary):**

- **Storage** — the kernel calls the storage backend (RocksDB for single-node, TiKV for distributed) directly via native Rust. Resources are stored as CBOR (RFC 8949) for compact, fast serialization. No callback indirection.
- **Materialization** — lazy resource handles (§8.2) resolve content from storage on demand, within the kernel process.
- **Clock** — HLC timestamp management, including synchronization with other kernel nodes in distributed deployments.
- **Capability sandbox** — the kernel instantiates untrusted capability code as WASM modules via Wasmtime (see §2.1), managing their lifecycle, memory limits, and interface surface.

This architecture means the kernel has direct access to storage, native threading, and memory-mapped I/O in the primary deployment. The WASM compilation target remains available for browser and edge deployments (see §2.6), where the kernel runs as a WASM module with the storage and clock callbacks provided by the browser or edge runtime.

### 2.3 AI Integration Model

The system interacts with LLMs and reasoning models in two directions, both mediated by the capability protocol and the host layer.

**Core → LLM (program-driven invocation).** When a program expression applies a Component whose implementation is an LLM adapter, the execution engine invokes the adapter with the expression's typed inputs. The adapter translates the Eigon-typed request into a provider-specific API call (Anthropic Messages API, OpenAI Chat Completions, etc.), receives the response, and wraps it as a typed Eigon resource conforming to the Component's declared output type. The kernel sees only typed resources flowing through the program; the host handles the network call, authentication, retries, and response parsing.

LLM adapter Components are registered in the ontology like any other Component. Their type signatures declare input classes (e.g., a prompt resource with system message, user message, and configuration properties) and output classes (e.g., a completion resource with content, token usage, and model metadata). The `NonDeterministic` marker (§4.1) and `Result` type (§4.5) are part of the signature — every LLM adapter is non-deterministic by nature and fallible due to network/API conditions.

**LLM → Core (tool-use / agentic access).** An LLM operating as an agent can use Eigenius as a tool — querying the knowledge graph, validating pipeline specifications, recording reasoning traces. This is the inverse direction: the LLM is the caller, and Eigenius exposes its kernel contract as tool definitions.

The natural integration point is the **Model Context Protocol (MCP)** or equivalent tool-use protocols. An Eigenius MCP server exposes the kernel's four operations (Load, Query, Validate, Reflect) as tools that an LLM can invoke through its tool-use mechanism. The typed nature of Eigon resources maps naturally to tool parameter schemas — an EigenQL query is a structured input, and the query result is a structured output, both with schemas derivable from the ontology.

This bidirectional integration means an LLM can both *be orchestrated by* a program (as a Component invocation) and *orchestrate* programs (as an agent using tools). The reflection layer (§11) captures both interaction patterns as typed reasoning traces, creating a unified provenance record regardless of whether the LLM was the caller or the callee.

**Provider abstraction.** LLM adapter Components should be parameterized by a provider configuration resource rather than hardcoded to a specific API. The Vercel AI SDK provides a practical model for this: a unified interface across providers (Anthropic, OpenAI, Google, open-source models) with provider-specific configuration. The architecture does not mandate a specific SDK, but the Component registration model naturally supports swappable adapters — replacing an Anthropic adapter with an OpenAI adapter is a capability registration change, not a pipeline change, provided both conform to the same input/output class contract.

### 2.4 Lean 4 Formal Track

Lean 4 is developed in parallel with the Rust kernel as a formal specification and proof environment. It is not a build dependency — Eigenius compiles and runs without it. Its role is to provide machine-checked proofs of metatheoretic properties: both for the kernel's structural guarantees and for the hosted languages (EigenQL, the program expression language) that run as registered capabilities.

**Lean 4 covers:**

- Eigon structural type system soundness — well-typed resources satisfy their class constraints
- EigenQL query semantics — pattern matching is sound with respect to the Eigon type system; variable bindings are type-consistent; evaluation terminates (guaranteed for non-recursive queries trivially, and for recursive rules via seminaive fixpoint over finite fact sets; stratified negation prevents paradoxes)
- Program type safety — well-typed programs preserve types through execution; termination guaranteed by strong normalization of the EigenTT type theory
- Stratification consistency — the universe level system prevents self-reference paradoxes

**Connection to the Rust kernel:**

The Lean 4 development serves two distinct roles depending on whether the subject is kernel-resident or capability-hosted.

For kernel-resident algorithms (Eigon structural type-checking, layer resolution, capability dispatch), the Lean 4 specification directly informs the Rust implementation. Type system rules in Lean become match arms in the Rust type checker. Verus annotations on the Rust kernel enforce that the implementation satisfies the contracts established by the Lean proofs.

For capability-hosted languages (EigenQL, the program expression language), the Lean 4 proofs establish that the language semantics are sound — that well-typed queries terminate, that well-typed programs produce values of the declared type. The program type system is founded on a dependent type theory (EigenTT) that is a direct fragment of CIC — Lean 4's own core theory — making the formal specification a scaled-up version of the same computational model rather than a translation into a different formalism. The Lean 4 specification serves as the authoritative reference against which capability implementations are verified, but Verus is not the enforcement mechanism — the capability implementations carry their own correctness discipline informed by the Lean proofs.

As the ecosystem matures, proved decision procedures in Lean 4 — type unification, and (once EigenQL is extended with recursive rules) stratification checking — may be reimplemented in Rust with the Lean proof as a machine-checked specification, whether those implementations live in the kernel or in capability code.

**Maintaining correspondence without a build dependency.** The Lean 4 specification is authoritative but is not mechanically enforced on capability-hosted implementations. For kernel-resident algorithms, Verus annotations provide mechanical enforcement. For capability-hosted languages, the correspondence is maintained through: (1) property-based test suites derived from the Lean 4 specifications, generated as part of the Lean 4 development process; (2) extraction of test oracles from proved lemmas; (3) periodic formal audits comparing implementation behavior against the specification. This discipline is weaker than mechanical verification but stronger than informal documentation. The architecture accepts this trade-off to avoid making Lean 4 a build dependency.

### 2.5 Bootstrap Sequence

If EigenQL and the program validator are registered capabilities, and capability registration is expressed in the ontology, the system cannot use the standard capability dispatch protocol to load the capabilities it needs to perform capability dispatch. This circularity is resolved by a two-phase bootstrap sequence.

**Phase 1: Core Ontology.** The kernel loads the Core Ontology from its embedded static representation using a hardcoded structural validator. This does not go through the capability protocol — it is the one place where the kernel processes ontology resources without dispatching to external capabilities. After Phase 1, the kernel can structurally validate Eigon resources but cannot evaluate EigenQL queries or validate programs.

**Phase 2: Foundation Layer.** The kernel loads the Foundation Layer (`urn:eigenius:foundation:`) using a minimal hardcoded capability loader. The Foundation Layer defines the ontology classes for EigenQL queries, program expressions, ESL syntax, and their associated capability registrations. The hardcoded loader understands just enough of the capability registration schema to bootstrap these registrations into the capability dispatch table. It does not implement EigenQL or program validation — it only reads capability registration resources and wires them into the dispatch mechanism.

After Phase 2 completes, the standard capability dispatch protocol is fully operational. All subsequent ontology loading, querying, and validation proceeds through registered capabilities. The Foundation Layer is the only layer (other than the Core Ontology) that receives special treatment from the kernel.

**Bootstrap versioning.** The hardcoded capability loader in Phase 2 creates a version-coupling point between the kernel and the Foundation Layer. If the Foundation Layer's capability registration schema evolves (e.g., adding new required fields), the kernel's hardcoded loader must be updated in tandem. This is an intentional, accepted coupling: the Foundation Layer's capability registration schema is part of the kernel's ABI. Changes to it constitute a major version increment requiring coordinated kernel and Foundation Layer releases. The schema is designed to be minimal and stable — it defines how to wire a capability into the dispatch table, nothing more. Domain-specific capability semantics are expressed within the capabilities themselves, not in the registration schema, minimizing the surface area of this coupling.

**Kernel-resident lookup primitive.** Capability dispatch itself requires finding the capability registration for a given class. This lookup cannot go through EigenQL (which is itself a capability). The kernel therefore contains a minimal, non-extensible lookup primitive: given a class identifier, scan the capability registrations in the current context's layer stack and return the matching registration. This is a fixed algorithm in the kernel — it is not an EigenQL query, not a capability, and not extensible. It exists solely to break the circularity between capability dispatch and query evaluation.

### 2.6 Deployment Model

The Rust kernel compiles to native code for the primary server deployment and to WASM for browser and edge targets. The orchestration layer provides program execution, LLM integration, and developer tooling. This architecture enables several deployment models from the same kernel codebase.

**Server (native Rust kernel + Deno orchestration).** The primary deployment model. The kernel runs as a native Rust service with RocksDB for persistent storage (or TiKV for distributed deployments). The Deno orchestration layer connects to the kernel's gRPC API and handles program execution coordination, LLM adapter calls, and the MCP server surface. The full capability set is available: program execution, concurrent Map evaluation, distributed storage, real-time reasoning trace streaming, WASM-sandboxed capability execution. Untrusted capability code runs in WASM sandboxes managed by the kernel via Wasmtime. Node.js is supported as a fallback orchestration runtime.

**Edge / Serverless (WASM kernel).** For edge deployment, the kernel compiles to WASM and runs in edge runtimes (Deno Deploy, Cloudflare Workers, Vercel Edge Functions). Deno Deploy is the natural edge target given Deno as the orchestration runtime — the same TypeScript orchestration code runs in both environments. Storage is backed by platform-specific services (Deno KV, Cloudflare D1/KV/Durable Objects, or remote RocksDB/TiKV connections). Suitable for read-heavy workloads: ontology queries, program validation, serving pre-computed results. Write-heavy workloads (program execution with many reasoning traces) may be constrained by edge platform limits. In this deployment, the kernel runs as a WASM module instantiated by the edge runtime, with storage and clock provided via callbacks — the same interface as the browser deployment.

**Browser (WASM kernel).** The kernel compiles to WASM and runs in a web browser. Storage is backed by IndexedDB (for local-first scenarios) or the kernel service's API (for thin-client scenarios). LLM adapters route through a backend proxy or use client-side API keys. Suitable for developer tooling (ontology browsers, program editors, query explorers) and local-first applications where the knowledge graph is small enough to fit in browser storage.

**The kernel is the constant; the deployment target determines the compilation and integration model.** In the primary server deployment, the kernel runs native with direct storage access. In edge and browser deployments, the kernel compiles to WASM with storage mediated by callbacks. The kernel's four operations (Load, Query, Validate, Reflect) and the capability sandbox are available in all deployment models. The Deno orchestration layer's TypeScript code is shared across server and edge targets.

---

## 3. Core Ontology

### 3.1 The Bootstrap Principle

The Core Ontology is the fixed, immutable foundation from which all other ontology definitions are derived. It provides the minimal set of primitives needed to describe ontology structure — and critically, it describes itself using those same primitives. The Core Ontology is not defined in a separate meta-language; it is expressed in Eigon.

This self-describing property is not incidental. It is the architectural mechanism that makes the entire system coherent. Every concept in the system — including processing pipelines, capability registrations, reasoning traces, and proof terms — is ultimately grounded in the same small set of primitives defined here. The Core Ontology is the fixed point of the system's self-description.

The Core Ontology lives permanently at the root of every layer stack, under the namespace `urn:eigenius:core:`. It is baked into the Rust kernel as a static, immutable artifact. No execution context can modify it. No layer can shadow its definitions. It is the one thing in the system that is truly fixed.

### 3.2 Primitive Classes

Three classes form the foundation. Each is an instance of itself.

**Class**
A Class describes an abstract concept — the type and meaning of a category of resources. Classes are the mechanism by which resources are typed. A resource may have one or more classes, and the union of their required and recommended properties defines the resource's expected shape. It is convention to use uppercase in a Class IRI shortname.

```
urn:eigenius:core:Class
```

**Property**
A Property is a single typed field that a resource may carry. It defines the relationship between a subject resource and a value. A Property specifies its datatype, a human-readable description of its meaning, and a shortname for use in queries and surface syntax. Properties are the edges of the knowledge graph.

```
urn:eigenius:core:Property
```

**Datatype**
A Datatype describes the shape of a value — what kinds of data a property can hold. Datatypes are the leaves of the type system. They are not resources in the domain sense; they are type descriptors that the kernel understands natively.

```
urn:eigenius:core:Datatype
```

### 3.3 Core Properties

The following properties are defined by the Core Ontology. Each is itself an instance of `urn:eigenius:core:Property`, described using the same properties it defines. Properties are grouped by concern.

**Identity and classification:**

| Short name | Data type | Description |
|---|---|---|
| `is_a` | `resource_array` | The classes of which this resource is an instance. Determines which properties are required and recommended. Class types: `Class`. |
| `description` | `markdown` | A human-readable description of the resource. By convention, the first sentence should be self-contained. |
| `short_name` | `identifier` | A short, unambiguous local name. Used in EigenQL queries and ESL surface syntax. Case-sensitive, lowercase only, letters, digits, and hyphens. |

**Class structure:**

| Short name | Data type | Description |
|---|---|---|
| `requires` | `resource_array` | The properties that must be present on any resource of this class. Inherited transitively from superclasses. Class types: `Property`. |
| `recommends` | `resource_array` | The properties that are optional but expected on resources of this class. Inherited transitively from superclasses. Class types: `Property`. |
| `subclass_of` | `resource_array` | The classes of which this class is a direct subclass. Property inheritance and constraint inheritance are transitive across the full subclass chain. Class types: `Class`. |
| `disjoint_with` | `resource_array` | Classes that cannot share instances with this class. A resource that is an instance of this class may not simultaneously be an instance of any listed class. Enforced at validation time. Class types: `Class`. |
| `equivalent_to` | `resource` | Declares this class or property semantically equivalent to another. Used primarily in bridge layers for cross-namespace alignment. No automatic inference is performed — the declaration is asserted and queryable, not reasoned from. |

**Property typing:**

| Short name | Data type | Description |
|---|---|---|
| `data_type` | `resource` | The datatype of a property's value. Class types: `Datatype`. |
| `class_types` | `resource_array` | For properties with a `resource` or `resource_array` datatype, specifies the classes that values must instantiate. A value satisfies this constraint if it is an instance of any listed class or any subclass thereof. Class types: `Class`. |
| `allows_only` | `resource_array` | Restricts a property to a fixed enumeration of permissible values. Values must have unique shortnames. |
| `domain` | `resource_array` | Restricts this property to resources that are instances of one of the specified classes. Class types: `Class`. |
| `element_type` | `resource` | For properties with a `value_array` datatype, specifies the primitive element type. One of: `boolean`, `float`, `integer`, `string`. |

**Property relationships:**

| Short name | Data type | Description |
|---|---|---|
| `subproperty_of` | `resource` | Declares this property a specialization of another. Any value valid for this property is also valid for the superproperty. Enables cross-namespace property alignment in bridge layers. Class types: `Property`. |
| `inverse_of` | `resource` | Declares that this property is the inverse of another. If resource X has this property pointing to Y, then Y has the referenced property pointing to X. Declaration only — no automatic inference is performed. Class types: `Property`. |

**Property characteristics:**

| Short name | Data type | Description |
|---|---|---|
| `functional` | `boolean` | If true, a subject resource may have at most one value for this property. Enforced at validation time. Equivalent to `owl:FunctionalProperty`. |
| `inverse_functional` | `boolean` | If true, at most one subject resource may have any given value for this property. Declares a natural key. Equivalent to `owl:InverseFunctionalProperty`. |
| `symmetric` | `boolean` | If true, this property is its own inverse. If X has this property pointing to Y, then Y has this property pointing to X. A shorthand for `inverse_of` referencing the property itself. |

**Cardinality:**

| Short name | Data type | Description |
|---|---|---|
| `min_count` | `integer` | The minimum number of values a resource must have for this property within this class context. A value of 1 is equivalent to inclusion in `requires`. Must be ≥ 0. |
| `max_count` | `integer` | The maximum number of values a resource may have for this property within this class context. Must be ≥ 1. |

**Navigation:**

| Short name | Data type | Description |
|---|---|---|
| `property_path` | `identifier_path` | A relative path specifying how to locate a nested resource by traversing properties from a root resource. |

### 3.4 Inheritance and Type Semantics

This section states precisely how the Core Ontology's structural properties compose. These rules are enforced by the kernel at validation time and are the subject of the corresponding Lean 4 formal proofs.

**Class inheritance is transitive.**
If B declares `subclass_of: [A]`, and C declares `subclass_of: [B]`, then C is also a subclass of A. The kernel resolves the full transitive subclass chain before validating any resource.

**Property inheritance accumulates by set-union.**
A resource of class C inherits the `requires` and `recommends` of all classes in C's transitive superclass chain. The effective required properties of C are the set-union of `requires` across all superclasses and C itself. The effective recommended properties are the set-union of `recommends` across all superclasses and C itself, minus any properties already in the required set.

**Multiple classes accumulate independently.**
A resource may have multiple classes via `is_a`. The effective required and recommended properties are the set-union across all listed classes and their full transitive superclass chains. A property required by any class in the set is required for the resource.

**`class_types` constraints are subclass-aware.**
A property value satisfies a `class_types` constraint if the value resource is an instance of any listed class or any subclass of any listed class. Subclass resolution uses the transitive chain.

**Cardinality refines presence constraints.**
`min_count: 1` is semantically equivalent to inclusion in `requires`. `min_count: 0` makes a property fully optional regardless of its presence in `recommends`. When both `requires` and `min_count` are present for the same property in the same class, the stricter constraint applies: `requires` implies `min_count ≥ 1`.

**`functional` is a cardinality shorthand.**
Declaring a property `functional: true` is equivalent to declaring `max_count: 1`. Both are enforced equivalently at validation time.

**`symmetric` is an inverse shorthand.**
Declaring a property `symmetric: true` is equivalent to declaring `inverse_of` referencing the property itself. Both are recorded in the ontology as explicit declarations. No automatic inference is performed — symmetry is checked only when explicitly validated.

**`inverse_of` and `subproperty_of` are declarations, not inference rules.**
These properties record relationships between properties for query and documentation purposes. The kernel does not automatically infer new triples from them. A EigenQL query may explicitly traverse an inverse relationship using the declared `inverse_of`, but the kernel does not materialise the inverse direction as stored data.

**Disjointness is checked pairwise at validation.**
When a resource is validated, the kernel checks that no two classes in its `is_a` set are declared `disjoint_with` each other, accounting for the transitive superclass chain. A class is considered implicitly disjoint with any class declared disjoint with one of its superclasses.

**`equivalent_to` is assertional, not inferential.**
Declaring two classes or properties equivalent does not cause the kernel to treat them as interchangeable during validation or query evaluation. The declaration is recorded as a resource property and is queryable via EigenQL. Reasoning from equivalence declarations is the responsibility of registered capabilities, not the kernel.

### 3.5 Core Datatypes

The type system uses three layers (see design doc D1 for full specification):

- **Primitive data types** determine the JSON-level representation
- **Formats** are validation constraints on string values (e.g., date, IRI, UUID)
- **Content types** declare the MIME type of content embedded in strings (e.g., text/markdown, text/html)

Data types are natively understood by the kernel. The following data types are defined by the Core Ontology.

**Primitive value types:**

| Short name | Description |
|---|---|
| `boolean` | True or false. Represented as a native JSON boolean in Eigon. |
| `integer` | Signed integer in the 53-bit safe range (-(2^53-1) to 2^53-1). Represented as a JSON number with no decimal point. |
| `float` | 64-bit IEEE 754 floating-point. Serialized as a JSON number. |
| `string` | UTF-8 string. May carry `format`, `content_type`, and/or `content_encoding` constraints via the property definition. |

**Resource types:**

| Short name | Description |
|---|---|
| `resource` | A reference to a resource. In Eigon JSON, either an IRI string (link to a top-level resource) or a nested object (an inline resource without its own IRI). |
| `resource_array` | An ordered array of resource references. Each element is either an IRI string or a nested object. |

**Array type:**

| Short name | Description |
|---|---|
| `value_array` | An array of primitive values. The element type is specified by the `element_type` property on the enclosing property definition. Elements must be homogeneous: all boolean, all float, all integer, or all string. |

**Opaque type:**

| Short name | Description |
|---|---|
| `json` | An opaque JSON value. Not validated by the ontology. |

**Formats** (validation constraints on `string` values):

| Short name | Description |
|---|---|
| `date` | ISO 8601 date without time component. Format: `YYYY-MM-DD`. |
| `datetime` | ISO 8601 date-time with timezone. |
| `time` | ISO 8601 time. |
| `iri` | A valid IRI (Internationalized Resource Identifier, RFC 3987). |
| `uuid` | A UUID (RFC 4122). |
| `regex` | A valid regular expression (ECMA 262 syntax). |

Formats are extensible — domain ontologies may define additional formats in their own namespaces.

**Content types** use standard MIME types (e.g., `text/markdown`, `text/html`, `application/xml`, `image/png`). They are declared on property definitions via the `content_type` property. Binary content embedded in strings uses `content_encoding` (e.g., `base64`).

### 3.6 The Self-Describing Property

The Core Ontology is expressed in Eigon using the same primitives it defines. The following ESL fragment illustrates this — the definition of `Property` itself, written using properties that `Property` defines:

```
// The Property class describes itself using the properties it introduces

class "urn:eigenius:core:Property" {
    description: "A Property defines a single typed field that resources may carry.
                  It specifies the relationship between a subject resource and a value,
                  including the value's datatype and the meaning of the relationship.",
    requires: [
        "urn:eigenius:core:data_type",
        "urn:eigenius:core:description",
        "urn:eigenius:core:short_name"
    ],
    short_name: "Property"
}

property "urn:eigenius:core:data_type" "urn:eigenius:core:resource" {
    description: "The data type of a property's value.",
    class_types: ["urn:eigenius:core:DataType"],
    short_name: "data_type"
}
```

The kernel bootstraps the Core Ontology by processing this self-description through a hand-initialized runtime that precedes the normal processing pipeline. Once the Core Ontology is loaded, all subsequent ontology processing — including the loading of any other layer — uses the standard Eigon pipeline.

### 3.7 Immutability and the Layer Root

The Core Ontology is the only layer in the system that is:

- Embedded directly in the Rust kernel rather than loaded from the distributed store
- Guaranteed to be present in every execution context's layer stack
- Incapable of being shadowed, overridden, or modified by any other layer

The Verus annotations on the kernel's layer management code formally enforce these properties. Any attempt to define a resource whose IRI falls within `urn:eigenius:core:` in any layer other than the Core Ontology is rejected at ingestion time as a hard error.

### 3.8 Extension through Domain Ontologies

The Core Ontology intentionally provides only the primitives needed for self-description. It does not define concepts specific to any domain, processing model, or application. Everything else — including the ontology classes for programs, Components, capability registrations, and reasoning traces — is defined in domain ontologies that extend the Core Ontology through the layer system.

The Core Ontology makes no assumptions about what will be built on top of it. Its role is to provide a stable, formally grounded foundation that can carry any domain that can be expressed in terms of classes, properties, and typed values.

### 3.9 Decidability Boundary

The Core Ontology is deliberately bounded to constructs for which type-checking and query evaluation remain decidable given a finite ontology. Every property and constraint in the Core Ontology is checkable at validation time without forward or backward chaining inference. This is a hard design constraint, not a temporary limitation.

The following constructs from OWL and related formalisms are **excluded from the Core Ontology** for this reason:

**Transitive properties** (`owl:TransitiveProperty`): Transitivity requires the kernel to derive new facts by chaining existing relationships, introducing potentially unbounded inference. If a domain requires transitive closure — such as hierarchical containment — this must be computed explicitly via EigenQL queries (or, once recursive rules are available, via EigenQL rule definitions), not declared as an ontology property.

**Union and intersection class constructors** (`owl:unionOf`, `owl:intersectionOf`): These push the expressive complexity toward OWL DL, where reasoning becomes EXPTIME-complete. They may be offered as a registered capability for advanced use cases but are not core primitives.

**Negation and complement** (`owl:complementOf`): Negation combined with existential and universal quantifiers pushes into undecidability. Excluded unconditionally.

**Property chains** (`owl:propertyChainAxiom`): Composing properties into chains introduces transitivity-like complexity. Excluded for the same reason as transitive properties.

**Anonymous class expressions**: Complex class descriptions formed from constructors without explicit IRIs make validation and query planning substantially harder. All classes in the Core Ontology and domain ontologies must have explicit IRIs.

**The guiding principle:** the Core Ontology validates. Registered capabilities reason. Any construct that requires the kernel to derive new facts from existing ones — rather than check stated facts against stated constraints — belongs in the capability layer, where its computational cost and termination properties are the responsibility of the capability implementation.

---

## 4. Processing Programs

### 4.1 Programs as Total Functional Expressions

Processing programs in Eigenius are typed expressions in a total functional programming language whose type system is grounded in the Eigon Core Ontology. Programs are represented as Eigon-JSON resources — the same format as everything else in the system — using expression forms (Let, Apply, Lambda, Case, Map, Reduce, etc.) that map directly to EigenTT terms. See design doc D3 (`docs/design/d3-program-model.md`) for the full specification.

The distinction from conventional workflow engines matters precisely. In a conventional engine, a pipeline is a description that an interpreter executes, with type errors and failures discovered at runtime. In Eigenius, a program that passes validation carries a formal guarantee: it terminates, it is type-safe, and it produces output of the declared type. Validation is not a heuristic check — it is a proof of these properties, performed statically before the first expression evaluates.

This makes the program validator a type inference and type checking algorithm, and the execution engine a runtime for a known-correct program. The two concerns are cleanly separated: correctness is established at validation time; execution is a mechanical process that cannot violate the established type guarantees.

**Formal vs. operational properties.** The formal guarantees established by validation — termination, type safety, exhaustive error handling — are properties of the program *language*. They ensure that the pipeline's structure is correct. They do not guarantee properties of program *execution* that depend on external systems. Specifically:

- **Non-determinism.** Steps that invoke LLMs or external services are non-deterministic: two executions of the same well-typed program with the same inputs may produce structurally different outputs. The type system guarantees that the output is of the declared type, but not that it is the same value. The reflection layer tracks this distinction — reasoning traces record which steps are deterministic and which are not, enabling downstream analysis of result reproducibility.

- **Bounded time.** The formal termination guarantee means the program program's *control flow* terminates — there are no infinite loops. It does not guarantee bounded wall-clock execution time, since external calls (LLM APIs, network services) may block indefinitely. The `ExecutionConstraints` on the execution context (§8.2) enforce wall-clock bounds as a runtime safety net, separate from the formal termination property.

Components declare their determinism and fallibility characteristics in their type signatures. A Component whose implementation is both deterministic and total declares a plain output type. A Component that is non-deterministic but infallible (e.g., an LLM call that always returns *something*) declares its output type with a `NonDeterministic` marker. A Component that may fail declares `Result<A, E>`. These markers propagate through the program type system, and the reflection layer records them in reasoning traces.

### 4.2 Theoretical Foundation: Dependent Type Theory with Normalization by Evaluation

The program language is founded on **total functional programming** — a discipline in which every well-typed program is guaranteed to terminate and produce a value of its declared type. Partial functions — functions that may diverge or fail on some inputs — cannot be expressed in the core language. Failure is represented in types, not as exceptions.

The specific theoretical foundation is a **dependent type theory** based on EigenTT — a minimal dependent type system with dependent functions (Pi types), dependent pairs (Sigma types), labeled sum types, and a universe of types — extended with Eigon ontology types as ground types. The evaluator uses **normalization by evaluation (NbE)**, which serves double duty as the core of both type checking and partial evaluation.

This foundation is chosen over System Fω (the polymorphic lambda calculus with type-level functions) for three reasons that are specific to Eigenius's requirements:

**Dependent types are native, not encoded.** In Eigenius, a program step that extracts a property from a resource produces an output whose type depends on the resource's class — a runtime value. In System Fω, this dependency must be encoded through type-level functions, which is indirect and limited. In dependent type theory, it is expressed directly as a Pi type: `Π (c : Class). PropType c → ResultType c`. The type of the output depends on the value of the class, and the type checker verifies this dependency through evaluation. This is the natural type-theoretic expression of ontology-driven typing.

**Partial evaluation is a consequence, not a feature.** The NbE approach evaluates terms as far as possible, producing *normal forms* — fully reduced terms. When a term contains free variables (unknown values), evaluation does not fail; it produces a normal form containing *neutral terms* that represent computations waiting on the unknown values. This is precisely partial evaluation: a program with some inputs bound and some abstract evaluates to a residual pipeline containing only the dynamic parts. This has direct practical value (see §4.9) and requires no additional machinery beyond the type checker's own evaluation strategy.

**The bridge to formal verification is structural, not an encoding.** EigenTT is a fragment of the Calculus of Inductive Constructions (CIC), which is Lean 4's core type theory. Both use the same NbE approach for type checking, the same notion of definitional equality via normalization, and the same bidirectional type-checking discipline. The Lean 4 formal specification of the program type system is therefore a direct embedding of the same computational model at a larger scale — not a translation from one type theory into another. Proof terms produced by Lean 4 can be interpreted directly by the program type system's evaluator (with appropriate embedding), creating a seamless path from pipeline validation to formal verification.

**The core type theory provides:**

**Strong normalization.** Every well-typed program reduces to a normal form in a finite number of steps. The proof follows from the standard strong normalization argument for EigenTT's type theory: all types are strictly positive, and the only recursion available is structural recursion over finite data (via `letrec` restricted to structurally decreasing arguments). There is no possibility of infinite loops in a validated pipeline.

**Dependent function types (Pi).** A step from input type A to an output type that depends on the input value has type `Π (x : A). B(x)`. This subsumes both simple function types (`A → B`, when B does not depend on x) and polymorphic types (`Π (α : Class). α → α`, when the type variable ranges over ontology classes). There is no separate polymorphism mechanism — dependent functions provide it natively.

**Dependent pair types (Sigma).** An execution context carrying a resource along with evidence about its type is expressed as `Σ (x : A). B(x)`. This subsumes both simple product types (when B does not depend on x) and existential types ("there exists a resource of some class satisfying a constraint"). The program execution context at any point is a nested Sigma type accumulating all step outputs computed so far.

**Labeled sum types.** The `Result<A, E>` type and domain-specific variant types are expressed as labeled sums: `Sum(ok : A | err : E)`. Pattern matching over sums is checked for exhaustiveness at the type level — every constructor must be handled.

**Decidable type checking with bidirectional discipline.** Type *checking* (verifying that a term has a given type) is decidable for the restricted theory. Full type *inference* (inferring a type without any annotations) is not — this is an inherent property of dependent type theories. The practical impact is managed through bidirectional type checking: program boundaries (declared input and output classes), Component signatures, and step bindings carry explicit type annotations; the type checker infers types internally and checks them against the annotations. This is the same discipline used by Lean 4, Agda, and Idris. In practice, the annotations required are exactly those the architecture already mandates for readability and documentation — Component input/output types and program boundary types.

### 4.3 The Program Type System

The program type system extends the Eigon Core Ontology's type system with the constructs of the dependent type theory. Eigon types — classes, properties, and datatypes — are the **ground types**: they are values in the type theory that the evaluator resolves against the ontology layer stack. The dependent type theory's Pi, Sigma, and Sum types compose these ground types into the types of program expressions, bindings, and pipelines.

**Eigon ground types.** Ontology classes, properties, and datatypes are first-class values in the type theory. The evaluator resolves a class reference like `urn:ford:vehicles:Vehicle` against the current execution context's layer stack, producing a ground type value that carries the class's full property structure (required properties, recommended properties, subclass relationships). Type checking a property access on a resource of class C proceeds by evaluating the property's declared type from C's schema — this is dependent typing in action, since the result type depends on the class value.

**Dependent function types (Pi).** A step from resource class A to resource class B has type `Π (_ : A). B` (written `A → B` when B does not depend on the input). When the output type depends on the input value — such as a step that extracts a named property and returns a value whose type is the property's declared datatype — the full dependent function type `Π (x : A). B(x)` is used. Composition of steps is function composition, and the type checker verifies compatibility by evaluating output and input types to normal forms and checking definitional equality.

**Dependent pair types (Sigma) as execution context.** A program execution context at any point is a Sigma type accumulating all step outputs computed so far: `Σ (s1 : T1). Σ (s2 : T2(s1)). ... Σ (sn : Tn(s1,...,sn-1)). Unit`. Each step has access to any earlier step's output via its bindings, and the type of each step may depend on the values of earlier steps. The context type grows monotonically as steps execute.

**Bounded polymorphism via dependent functions.** Steps may be polymorphic over ontology classes. A step that processes any resource satisfying a class constraint has type `Π (α : Class). (α <: C) → α → α`, where the subclass witness `α <: C` is a value that the type checker resolves from the ontology's subclass hierarchy. This replaces the System Fω-style bounded quantification `∀ α <: C. α → α` with a dependent formulation where the bound is a value-level witness rather than a syntactic constraint.

**Sum types and Result.** Fallible steps return a labeled sum type: `Sum(ok : A | err : E)` where A is the success resource class and E is the error resource class. Both are Eigon resource classes. Pattern matching over the sum is checked for exhaustiveness at the type level — both constructors must be handled. A program that ignores the error case does not type-check. This replaces the previous `Result<A, E>` notation with the type theory's native labeled sums, making Result a defined type rather than a primitive.

**Collection types.** The `resource_array` datatype from Eigon is lifted to a typed collection `List(A)` where A is an Eigon class. The Map combinator is typed as `Π (A B : Class). (A → B) → List(A) → List(B)`. The element type is tracked through the type system via dependent function application.

### 4.4 Primitive Constructs

The following constructs are primitive in the program language — they are not defined in terms of other constructs but are understood natively by the type checker and execution engine. All are total by construction.

**Step.** The atomic unit of computation. A Step references a Component — a named, typed function registered in the ontology — and provides bindings that route data from the current execution context into the Component's inputs. The type of a Step is determined by the Component's declared input and output types.

```
step "extract-entities" ExtractEntities {
    input: ctx.document
}
```

**Sequence.** Ordered composition of steps sharing an execution context. The context type after a Sequence is the product of the context type before it and the output types of all steps within it. A Sequence is a reusable, named sub-pipeline that can be embedded within a program or within another Sequence.

**Binding.** The mechanism by which data is routed between steps. A Binding selects a value from the current execution context — identified by step label and optionally by property path — and routes it to a named input of the receiving step. The type checker verifies that the selected value's type is compatible with the receiving input's declared type, accounting for subclass relationships.

**Map.** A typed higher-order combinator that applies a Sequence to every element of a collection independently, producing a new collection of results. Map does not require general recursion — it is a primitive fold over a finite collection, total by construction.

```
Map(sequence: ProcessPage, input: ctx.pages) → List<PageResult>
```

The type of Map's output collection is inferred from the declared output type of the Sequence applied to each element's type. If the Sequence is fallible — returning `Result<B, E>` — the Map output is `List<Result<B, E>>`, preserving error information per element.

**Select.** A typed case expression over the execution context. A Select specifies a set of guarded branches — each branch has a guard condition and a Sequence to execute if the guard is satisfied. The first branch whose guard is satisfied is executed. The type checker verifies that all branches produce compatible output types.

Guard conditions are expressed as EigenQL queries used as boolean predicates (see §5, EigenQL). A guard query evaluates to true if there exists at least one assignment of values to its query variables that satisfies all constraints. This makes guard conditions first-class ontology-queryable resources rather than opaque boolean expressions.

**Exhaustiveness.** A Select must include a default (catch-all) branch as its final case. The type checker verifies that the default branch is present and that all branches produce compatible output types. Full semantic exhaustiveness checking — proving that a set of EigenQL guard queries collectively covers all possible input states without needing a default — is a desirable future extension but is not part of v1. The general problem of proving that a disjunction of existentially quantified conjunctive queries covers an open-ended input type is undecidable in the general case and would require restricting the guard language to a decidable fragment. For v1, the mandatory default branch provides a safe, simple alternative.

**Reduce.** A typed fold over a collection that accumulates a result. Takes a Sequence defining the accumulation step, an initial accumulator value of a declared type, and a collection. Returns a single value of the accumulator type. Total by construction — the fold terminates because the collection is finite.

```
Reduce(sequence: Summarize, initial: EmptySummary, input: ctx.chunks) → Summary
```

### 4.5 Result Types and Fallibility

Total functional programming requires that failure be represented in types rather than as exceptions. In the program language, any step that may fail due to external conditions — LLM calls, document processing, external service invocations — has an output type of `Result<A, E>` where A is the success class and E is the error class, both Eigon resources.

This has several important consequences:

**Error handling is mandatory.** A program that consumes a fallible step's output without handling the `Result` does not type-check. Error handling is not optional or easily overlooked — it is a type-level requirement.

**Error types are domain resources.** The error class E is a full Eigon resource with typed properties. Error information is structured and queryable, not an opaque string message. This means reasoning traces can include structured error context, and EigenQL can query across success and failure outcomes uniformly.

**Fallibility propagates explicitly.** If a step producing `Result<A, E>` feeds into a subsequent step expecting A, the pipeline must include an explicit unwrapping step — either a Select that handles the error branch, or a combinator that propagates the error outward. The type system makes propagation visible and deliberate.

**Components declare fallibility in their type.** A Component whose implementation may fail declares this in its output type. The program type checker sees the `Result` type at the Component boundary and propagates it through the pipeline accordingly. Components that are total — guaranteed to succeed — declare plain output types without Result.

### 4.6 Program Validation as Type Checking

program validation is **bidirectional type checking** in the dependent type theory, using NbE for type equality. The validator operates in two modes — **checking** (verifying that a term has a given type) and **inference** (synthesizing a type from a term) — following the standard bidirectional discipline of EigenTT.

The validation process proceeds in the following phases:

**Name resolution.** Component references and class references are resolved from shortnames to fully qualified IRIs using the current execution context's layer stack and capability registry. Resolved class references become ground type values in the evaluator.

**Bidirectional type propagation.** Starting from the program's declared input type, the type checker propagates types forward through the step sequence. Steps, Bindings, and combinators (Map, Reduce, Select) are in *checking* mode — they are checked against the type expected by the context. Component references and variable lookups are in *inference* mode — their types are synthesized from the ontology schema and type environment. The context type is extended with each step's output type as it is processed.

**Type equality by normalization.** When the type checker needs to verify that two types are compatible — e.g., that a step's output type matches the next step's expected input type — it evaluates both types to normal forms using the NbE evaluator and compares the results structurally. Two types are definitionally equal if and only if they have the same normal form. This is the `eqNf` operation in the type-checking algorithm. For Eigon ground types, normalization resolves class references against the layer stack, computes subclass relationships, and expands property inheritance, so that type equality accounts for the full ontology structure.

**Subclass compatibility.** A value of class B is compatible with an input expecting class A if B is a subclass of A in the transitive subclass chain. In the dependent type system, this is expressed as a coercion: the type checker inserts an implicit subclass witness when B's normal form includes A in its transitive superclass set. This is resolved at validation time from the ontology schema, not at runtime.

**Default branch checking.** For each Select, the type checker verifies that a default (catch-all) branch is present and that all branches — including the default — produce compatible output types (by normalizing and comparing).

**Output type verification.** The type inferred for the final step's output is normalized and compared against the program's declared output type. If they are definitionally equal (accounting for subclass coercions), the program is well-typed.

**Totality verification.** The type checker verifies that all recursive structures — Map, Reduce — are applied to finite collections and that all Sequences terminate. General recursion (`letrec`) is restricted to structural recursion over finite data — the same discipline enforced by Lean 4's termination checker. Since all primitives are total and recursion is bounded, strong normalization is guaranteed.

A program that passes all validation phases carries the following formal guarantees:
- Its control flow terminates on every well-typed input (this is a structural guarantee about the program; wall-clock execution time depends on external systems and is bounded by `ExecutionConstraints`, not by the type system)
- Every step receives inputs of the types it declared, verified by normalization-based type equality
- The final output is of the declared output type
- All failure cases are explicitly handled (sum type exhaustiveness)
- Every Select has a default branch, so no input state is unhandled
- The program can be partially evaluated with respect to any subset of its inputs, producing a well-typed residual (see §4.9)

### 4.7 Relationship to the Eigon Type System

The program type system is not separate from the Eigon Core Ontology — it is built on top of it. Program specifications are themselves Eigon resources. The expression classes (`Program`, `Let`, `Apply`, `Lambda`, `Case`, `Map`, `Reduce`, etc.) are defined in the program ontology (under `urn:eigenius:program:`) using the Core Ontology primitives. See design doc D3 for the full specification.

This means:
- program specifications are stored, versioned, and queried like any other ontology resource
- EigenQL can query across program structure and domain data uniformly — finding all programs that reference a specific Component, or all steps that consume a specific resource class
- The reflection layer captures reasoning traces that reference the program steps that produced them, creating a typed provenance graph linking outputs back to the pipeline that computed them
- The program type checker is a registered capability, not a kernel primitive — the kernel knows how to dispatch to it, but the type checking algorithm lives outside the kernel and can be evolved independently

### 4.8 Connection to the Lean 4 Formal Track

The program type system has a precise formal account in Lean 4. The dependent type theory underlying the program language is a fragment of the Calculus of Inductive Constructions (CIC) — Lean 4's core type theory. Both use the same computational model: terms, values, neutral terms, normalization by evaluation, and definitional equality via readback to normal forms. The program type system embeds into CIC directly and without encoding — it is a subsystem, not a translation target.

This structural correspondence is significantly tighter than the previous System Fω-based design, where embedding into CIC required encoding System Fω's type-level functions as CIC's dependent functions (a lossy step that obscured the relationship between the two systems). With the EigenTT foundation, the program type checker's `eval`, `readback`, and `eqNf` operations correspond directly to the same operations in Lean 4's kernel, making the formal specification a scaled-up version of the implementation rather than a different formalism.

The Lean 4 formal development for programs covers:

**Type system soundness.** A well-typed program does not go wrong — every step receives values of the types it declared, and the final output is of the declared type. Formally: if `Γ ⊢ program : Π (x : A). B(x)` and the input has type A, then execution produces a value of type B(input). The dependent formulation means soundness covers value-dependent output types, not just fixed output types.

**Termination.** Every well-typed program normalizes to a value (or a normal form containing neutral terms, for partial evaluation). The proof proceeds by showing that the type theory's recursion is restricted to structural recursion over finite data, and that all primitive program constructs (Map, Reduce, Select) are definable in the total fragment.

**NbE correctness.** The normalization-by-evaluation algorithm is correct: two terms that are definitionally equal produce the same normal form, and two terms with different normal forms are not equal. This is the central proof obligation, since type equality in the dependent type system is decided by normalization. The proof follows the standard NbE correctness argument for EigenTT, extended with the Eigon ground type resolution layer.

**Partial evaluation soundness.** A partially evaluated program (one where some inputs are bound and others are abstract) is a well-typed normal form: it type-checks under an extended context where the abstract inputs are free variables. Executing the residual with the remaining inputs produces the same result as executing the original program with all inputs. This is a consequence of NbE correctness — partial evaluation is just normalization under an open context — but it warrants a separate formal statement because of its practical significance.

**Correspondence with the capability implementation.** The implementation of program validation is a direct computational realization of the Lean 4 specification. The Lean 4 `eval`, `readback`, and type-checking functions correspond one-to-one with the capability implementation's functions. Because the program validator is a registered capability rather than a kernel primitive, Verus is not the enforcement mechanism — correctness is maintained through the structural correspondence, property-based test suites extracted from the Lean 4 development, and periodic formal audits.

### 4.9 Partial Evaluation

The NbE approach used for type checking provides partial evaluation as a direct consequence, requiring no additional machinery.

**Mechanism.** The NbE evaluator processes a program term by evaluating it in an environment where some bindings are concrete values and others are *generators* — abstract placeholders representing unknown inputs. Evaluation proceeds as far as possible: concrete computations are reduced, and computations that depend on abstract inputs produce *neutral terms* — a structured representation of the stuck computation. The readback function converts the result to a normal form: a residual program term containing only the computations that depend on the remaining unknowns.

In EigenTT's implementation, this is the exact mechanism used for type checking under binders: when checking `Π (x : A). B(x)`, the type checker evaluates `B` with `x` bound to a generator (`Gen i`), producing a normal form of the body where `x` appears as a neutral term wherever the result depends on it. The same mechanism, applied to program terms, produces partially evaluated pipelines.

**Practical applications:**

**Pipeline specialization.** A generic pipeline parameterized by an ontology class can be partially evaluated with a specific class bound, producing a specialized pipeline where all type-dependent computations are resolved and only the data-dependent steps remain. This is analogous to template instantiation, but it is semantically grounded in the type theory rather than being a separate mechanism.

**Static configuration resolution.** program expressions that depend only on ontology schema information (class structure, property types, capability registrations) can be fully evaluated at validation time, since this information is available in the execution context's layer stack. The residual pipeline contains only steps that require runtime data (document contents, LLM responses, external API results).

**Incremental revalidation.** When a layer is added to the stack (e.g., a new domain ontology), programs that were partially evaluated against the previous stack can be re-evaluated with the new layer. The NbE evaluator reduces the terms that are affected by the new layer and leaves the rest unchanged. This is more efficient than full revalidation when the change is small relative to the pipeline.

**Formal status.** Partial evaluation soundness — the guarantee that executing a partially evaluated residual with the remaining inputs produces the same result as executing the original program with all inputs — is a formal consequence of NbE correctness and is a proof target in the Lean 4 development (§4.8). It is not an optimization heuristic; it is a theorem about the type system.

### 4.10 Connecting the Eigon Ontology to the Type Theory

The dependent type theory (§4.2) uses Eigon classes, properties, and datatypes as ground types. This section specifies how the ontology primitives connect to the CIC-based core, identifies the design decisions required, and defines the boundary between what the type theory handles and what remains in the kernel's structural validator.

#### 4.10.1 Self-Description and Universe Separation

The Core Ontology is self-describing: Class is an instance of Class, Property is an instance of Property. In CIC terms, `Class : Class` — a type inhabiting itself — is precisely what causes Girard's paradox and what CIC's universe hierarchy (`Type 0 : Type 1 : Type 2 : ...`) exists to prevent. EigenTT's single universe `U` similarly excludes `U : U`.

This paradox dissolves at the right architectural boundary. The Core Ontology's self-description is *assertional* — it is a metadata statement recorded in the knowledge graph (`Class` has `is_a: [Class]`). The type theory does not need to internalize this as `Class : Class`. Instead, Eigon ground types sit at a fixed level in the type theory's universe: they are opaque base types with known structure that the evaluator resolves from the layer stack. The type theory *uses* ontology types to type program terms; it does not *validate the ontology's self-description*. The kernel's hardcoded bootstrap (§2.5) validates the Core Ontology outside the normal type-checking pipeline — this boundary between ontology self-description and type-theoretic typing already exists architecturally.

Consequently, the universe stratification in §11.4 and the type-theoretic universe hierarchy are separate mechanisms with different concerns. The ontology stratification prevents self-referential paradox in the knowledge graph (authorship levels). The type-theoretic universe prevents paradox in the type system (typing levels). They align — ontology level 0 maps to a fixed ground type level in the theory — but they are enforced independently.

#### 4.10.2 Classes as Dependent Record Types

An ontology class is represented in the type theory as a dependent record type — a nested Sigma type over the class's required properties:

```
Vehicle ≡ Σ (make : String). Σ (model : String). Σ (year : Integer). Unit
Truck   ≡ Σ (make : String). Σ (model : String). Σ (year : Integer). Σ (payload : Float). Unit
```

The NbE evaluator constructs these record types by resolving a class reference against the current execution context's layer stack: it retrieves the class's required and recommended properties (including those inherited through the transitive subclass chain), retrieves each property's declared datatype, and builds the corresponding Sigma type.

**Canonical property ordering.** The order of fields in a Sigma chain affects definitional equality — two Sigma types with the same fields in different order are distinct. To ensure that the same class always produces the same type regardless of the order in which properties were defined in the ontology, properties are ordered canonically by their fully qualified IRI (lexicographic). This is a fixed convention, not a user-facing concern — the evaluator produces canonically ordered types from any ontology input.

**Required vs. recommended properties.** Required properties are plain fields in the Sigma type. Recommended (optional) properties are wrapped: `Σ (nickname : Maybe String). ...`, where `Maybe A ≡ Sum(some : A | none : Unit)`. A resource that omits a recommended property is well-typed (the field is `none`); a resource that omits a required property is not. The type checker enforces this distinction at validation time.

**Subclass coercion.** If B is a subclass of A, then the type `B` is a Sigma type that extends `A` with additional fields. Subclass compatibility is modeled as a coercion function `B → A` that projects away the extra fields. The type checker inserts this coercion implicitly when a value of class B appears where class A is expected. The coercion is computed at validation time from the ontology's subclass hierarchy — it is not a runtime operation. Formally, the type checker maintains a coercion table derived from the transitive subclass chain, and inserts coercions during bidirectional type checking whenever the inferred type and the expected type differ by a subclass relationship.

#### 4.10.3 Datatypes as Base Types

The primitive Eigon datatypes map directly to base types in the theory:

| Eigon Data Type | Type Theory Representation |
|---|---|
| `boolean` | Base type, two values |
| `integer` | Base type, 53-bit signed |
| `float` | Base type, double-precision |
| `string` | Base type, UTF-8 (formats and content types are constraints, not separate base types) |
| `json` | Base type, opaque |

These are opaque to the type theory — it does not reason about their internal structure. Arithmetic, string operations, and date functions are external operations available in EigenQL expressions and Component implementations, not in the type theory itself.

**The `resource` datatype requires dependent typing.** A property with datatype `resource` can hold either a IRI reference to a top-level resource or an inline nested object. An inline resource has structure that depends on its class. The type of a `resource`-typed property with `classtypes: [C]` is:

```
ResourceOf(C) ≡ Sum(ref : IRI | inline : Σ (c : SubclassOf C). RecordType(c))
```

Where `SubclassOf C` is the type of classes in the transitive subclass set of C (resolved from the layer stack at validation time), and `RecordType(c)` is the dependent record type for class `c`. This is a genuine use of dependent typing — the inline variant's structure depends on a value (the class). The type checker verifies that inline resources match the `class_types` constraint by checking that the inline resource's class is in the subclass set.

**The `resource_array` datatype is a typed heterogeneous list.** With `classtypes: [C]`, it is `List(ResourceOf(C))` — each element independently satisfies the class constraint but may be an instance of a different subclass of C.

**The `value_array` datatype is a typed homogeneous list.** With `elementtype: T`, it is `List(T)` where T is one of the primitive base types. This is straightforward.

#### 4.10.4 Contextual Ground Types

The type theory's ground types are resolved from the execution context's layer stack. Type checking is parameterized by the ontology snapshot — the same class reference may resolve to different record types under different layer stacks (e.g., if a higher layer adds a new required property to a class).

This has specific implications:

**Program validity is snapshot-relative.** A program validated under layer stack S is guaranteed well-typed only under S. If the layer stack changes (a new layer adds or modifies class definitions), programs that reference affected classes must be revalidated. The partial evaluation mechanism (§4.9) mitigates this cost — a partially evaluated program can be incrementally re-evaluated against the new layer, and only the terms affected by the change need reprocessing.

**Type equality includes layer resolution.** When the type checker compares two types for definitional equality, both sides are first evaluated (which includes resolving class references from the layer stack). Two class references that resolve to the same record type (same properties, same types, same canonical order) are definitionally equal, even if they have different IRIs. Two references to the same IRI that resolve differently under different layer stacks produce different types. This is handled naturally by the NbE evaluator — ground type resolution is part of evaluation, and equality checking compares normal forms.

#### 4.10.5 Boundary Between Type Theory and Kernel Validation

Not all ontology constraints belong in the type theory. The division is:

**The type theory handles** (at program validation time):

- Types of program terms — every step, binding, and combinator is type-checked
- Property-dependent function types — output types that depend on input class structure
- Pipeline composition — binding compatibility verified by normalization-based type equality
- Subclass coercions — implicit record projections inserted by the type checker
- Sum type exhaustiveness — every labeled sum (including Result) is pattern-matched completely
- Partial evaluation — NbE with abstract inputs produces well-typed residuals

**The kernel structural validator handles** (at resource ingestion time):

- Ontology self-description — Core Ontology bootstrap, self-referential class structure
- Cardinality constraints — `functional` (at most one value), `inverse_functional` (at most one subject), `min_count`, `max_count`
- Population constraints — `disjoint_with` (no resource may instantiate two disjoint classes)
- Value enumeration — `allows_only` (property values restricted to a fixed set)
- Namespace integrity — resources written only within declared namespaces
- Layer immutability — committed layers cannot be modified

The two systems meet at the **ground type resolution interface**: the type theory's evaluator calls into the kernel to resolve a class reference into its record type (properties, datatypes, subclass relationships). The kernel ensures the ontology is structurally consistent *before* the type theory sees it. If the kernel accepts a class definition, the type theory can trust that the resulting record type is well-formed. If the kernel rejects a class definition (e.g., a circular subclass chain, a property with an undefined datatype), the type theory never encounters it.

This boundary is clean because the concerns are genuinely different: the type theory reasons about individual program terms and their compositions; the kernel validates the consistency of the knowledge graph as a whole. Neither subsumes the other, and neither needs to duplicate the other's work.

---

## 5. EigenQL

### 5.1 Overview

EigenQL is a typed semantic query language for pattern matching and retrieval over the Eigon knowledge graph. It provides a declarative way to query resources based on their classes, properties, and relationships, while maintaining type safety with respect to the ontology schema.

EigenQL is a **typed stratified Datalog** with aggregation — it supports conjunctive queries, recursive rule definitions (DEFINE), stratified negation, GROUP BY, aggregation (COUNT/SUM/AVG/MIN/MAX), ORDER BY, LIMIT/OFFSET, and DISTINCT. Non-recursive queries evaluate in a single pass; recursive rules use bottom-up seminaive fixpoint evaluation. Negation in MATCH patterns is subject to stratification checking. See design doc D2 (`docs/design/d2-eigenql-specification.md`) for the full specification.

### 5.2 Query Structure

A query consists of four clauses:

```
Query ::= [USING clause] MATCH clause [WHERE clause] RETURN clause
```

**USING.** Imports ontology classes for shortname reference within the query. Each IRI must resolve to a valid Class resource in the current execution context's layer stack. Class shortnames must be unique within the query scope.

```
USING "urn:eigenius:program:Program",
      "urn:eigenius:program:Component"
```

**MATCH.** Specifies typed patterns to match against resources in the knowledge graph. Patterns bind variables to resources and their property values. Multiple patterns are joined by shared variables, forming a conjunction (implicit AND).

```
MATCH Program(?prog) {
    description: ?desc,
    input_type: ?inputType
},
Component(?comp) {
    short_name: ?compName,
    input_class: ?inputClass
}
```

A pattern may be *typed* (with a class name, constraining matches to instances of that class and its subclasses) or *untyped* (matching any resource). Properties referenced in a typed pattern must be valid for the specified class, accounting for property inheritance through the subclass chain.

**WHERE.** Filters matched bindings using boolean expressions. Supports comparison operators (`=`, `<>`, `<`, `<=`, `>`, `>=`), logical operators (`AND`, `OR`, `NOT`), arithmetic operators (`+`, `-`, `*`, `/`, `%`, `**`), string concatenation (`||`), pattern matching (`LIKE`, `NOT LIKE`), collection membership (`IN`, `NOT IN`), and built-in functions (`DATE()`, `TIMESTAMP()`, `REGEX()`).

```
WHERE ?desc LIKE "document%" AND ?inputClass = "urn:example:Document"
```

Type constraints on expressions: comparison operands must have the same datatype; arithmetic operands must be numeric; `LIKE` operands must be strings; `IN` requires a resource on the left and a resource array on the right. No implicit type coercion is performed.

**RETURN.** Shapes query results into typed resources. The result class determines which properties are valid in the output. Expression types must match the declared property datatypes.

```
RETURN Step {
    component: ?comp,
    programDescription: ?desc
}
```

### 5.3 Variables and Type Inference

Variables are prefixed with `?` and are strongly typed based on their usage. A variable bound to a property in MATCH inherits the property's declared datatype. A variable used across multiple patterns must have a consistent type across all uses. Variables must be defined in MATCH before use in WHERE or RETURN.

The query evaluator performs type checking at query submission time: property references are validated against the ontology schema, variable types are inferred and checked for consistency, and expression types are verified against operator requirements. A query that fails type checking is rejected before evaluation begins.

### 5.4 Layer-Aware Resolution

EigenQL queries execute within an execution context and see exactly the resources visible within that context's layer stack. Resource resolution follows the same linear scan as all other layer operations (§7.2): the query evaluator scans from the top of the stack to the Core Ontology, and the first definition of a resource wins.

Class and property references in USING and MATCH clauses are resolved through the same mechanism — a shortname is resolved against the imported classes and their properties as defined in the visible layer stack.

Queries are monotonic with respect to the layer stack in v1: adding resources to the stack can only increase the result set, never decrease it. This property is significant for caching and incremental evaluation. It will require revisiting if EigenQL is extended with negation (§5.6).

### 5.5 Guard Expression Semantics

EigenQL queries are used as guard conditions in program Select constructs. When used as a guard, a query evaluates to true if there exists at least one assignment of values to query variables that satisfies all constraints — the MATCH patterns bind successfully and the WHERE conditions hold.

Guard queries omit the RETURN clause (since the purpose is boolean evaluation, not result shaping). A predefined class with shortname `Input` is available for pattern matching, bound to the current execution context's available properties at the point where the Select is evaluated.

```
MATCH Input(?input) {
    status: ?status,
    priority: ?priority
}
WHERE ?status = "failed" AND ?priority > 3
```

Additional properties may be made accessible via their shortnames through the Select step's configuration, allowing guards to reference domain-specific properties without full IRI qualification.

### 5.6 Extension Path to Recursive Datalog

EigenQL supports both standalone queries and recursive rule definitions. Non-recursive queries terminate trivially. Recursive rules terminate via seminaive fixpoint evaluation (bounded by the finite set of derivable facts). Stratified negation prevents paradoxes. The Lean 4 formal track proves type soundness, binding correctness, and stratification safety.

The following constructs are implemented:

**Rule definitions.** A `DEFINE` construct that names a query result as a derived relation, referenceable in MATCH patterns of other rules (including itself, for recursion).

```
DEFINE Ancestor(?x, ?z) FROM
    MATCH Employee(?x) { reports_to: ?z }

DEFINE Ancestor(?x, ?z) FROM
    MATCH Employee(?x) { reports_to: ?y },
    Ancestor(?y, ?z)
```

Multiple rules for the same relation provide union semantics. Self-reference enables recursion. Every v1 query remains valid and semantically identical — it is simply a single non-recursive rule.

**Fixpoint evaluation.** Recursive rules are evaluated using bottom-up seminaive evaluation: starting from base facts, applying all rules, adding newly derived facts, and repeating until no new facts are derived. Non-recursive queries bypass the fixpoint loop and evaluate in a single pass as before, with identical performance characteristics.

**Stratified negation.** Negation in MATCH patterns (not just WHERE filters), restricted by a stratification checker that orders rule evaluation to ensure negated relations are fully computed before being negated. This prevents paradoxes like "X is true if X is not true." The stratification checker is itself a well-understood algorithm — compute the dependency graph, check for negation cycles — and becomes a genuine proof target for the Lean 4 formal track.

**Monotonicity implications.** v1 queries are monotonic — adding facts to the layer stack can only increase results. Recursive rules without negation preserve this property. Stratified negation breaks monotonicity: adding a fact can invalidate a previously true negated condition. The mitigation is to track which queries use negation (the stratification checker computes this) and flag non-monotonic queries as requiring full re-evaluation on layer changes rather than incremental updates. The layer resolution model and caching strategy should document their monotonicity assumptions explicitly so that the impact of adding negation is localized and predictable.

**Design investments for v1 that keep this path open:** (1) The query evaluator is a registered capability behind the standard capability dispatch boundary, so the evaluation engine can be swapped from single-pass to fixpoint without touching callers. (2) Monotonicity assumptions in the layer/caching system are documented as explicit invariants, not invisible implementation details, so the exact points requiring revision when negation arrives are known in advance.

---

## 6. Namespace and Governance Model

### 6.1 Namespaces as Governance

Distinct ontologies live within distinct namespaces. This is not an organizational convenience — it is the primary isolation and governance mechanism of the entire system. A namespace is an institutional assertion of ownership and authority.

Examples:

- `urn:ford:` — definitions owned and controlled by the Ford Motor Corporation
- `urn:schema_org:` — definitions controlled by the schema.org consortium
- `urn:eigenius:` — definitions controlled by the Eigenius project

The hierarchy within a namespace reflects the governance hierarchy of the owning institution. `urn:ford:vehicles:` may be delegated to Ford's vehicle team. `urn:ford:vehicles:engines:` may be delegated further to the powertrain team. This delegation is an internal governance decision made by Ford, expressed in the Eigenius layer system.

### 6.2 Consequences of Namespace Separation

Because distinct ontologies live in distinct namespaces, identifier collisions between ontologies are structurally impossible. `urn:ontology-a:person:first` and `urn:ontology-b:person:first` are different resources by definition. Ontology combination therefore requires no conflict resolution machinery — it is always a disjoint union at the identifier level.

When a relationship between concepts from two ontologies is needed, a **bridge layer** is authored in a third namespace:

```
urn:bridge-ford-schema_org:vehicle-mapping
```

This bridge layer asserts the relationship without modifying either source ontology. It is versioned independently, queryable via EigenQL, and is itself a first-class Eigon resource.

### 6.3 Kernel Enforcement

The kernel enforces namespace boundaries structurally, not as policy:

**Namespace declaration.** When a layer is created, it declares its claimed namespace prefix and optionally its sub-namespace delegations.

**Resource ingestion validation.** Every resource added to a layer must have a IRI within the layer's declared namespace. This is a hard constraint enforced at ingestion time.

**Namespace conflict detection.** When a new layer is added to a stack, the kernel verifies that its claimed namespace does not overlap with any other layer in the stack.

**Cross-namespace references.** Resources in one namespace may reference resources in another. This is always permitted. Writing resources into another namespace is structurally impossible.

### 6.4 Trust and Authenticity

The kernel enforces namespace boundaries but does not adjudicate namespace ownership — that is an external governance question. The ecosystem should support cryptographic layer signing as a trust mechanism: namespace owners hold private keys, layers are signed, consuming systems verify signatures. The kernel provides hooks for signature verification without mandating a specific scheme.

An optional community-maintained namespace registry — itself an Eigon layer under `urn:eigenius:registry:` — may record known namespace owners and their public keys as a practical trust anchor for the open source ecosystem. This is positioned as a community service, not as Eigenius claiming governance authority.

---

## 7. Layer System

### 7.1 Structure

The ontology within any single execution context is organized as a linear stack of layers. Each layer has exactly one parent. The Core Ontology is the fixed, immutable root of every stack.

```
┌─────────────────────┐  ← top layer (mutable during construction)
├─────────────────────┤  ← committed layer N
├─────────────────────┤  ← committed layer N-1
│         ...         │
├─────────────────────┤  ← Foundation Layer (always present)
├─────────────────────┤  ← Core Ontology (immutable, always present)
└─────────────────────┘
```

A linear stack is chosen over a program of layers deliberately. A program introduces resolution ambiguity when the same resource is defined in multiple parent layers, complicates snapshot semantics, and makes provenance reasoning difficult. The linear model eliminates these problems: within any single execution context, the visibility order is unambiguous, and there are no merge conflicts by construction.

**The global layer structure is a tree, not a single stack.** Different execution contexts may have different top layers. Two contexts that share a common ancestor diverge above that ancestor — each sees its own linear stack, but the stacks share a common prefix. The set of all stacks across all contexts forms a tree (a singly-rooted structure where each layer has exactly one parent but may have multiple children). This is analogous to Git: each checkout sees a linear history, but the repository's branch structure is a program. The linearity guarantee is per-context, not global.

**Ontology combination is orthogonal to layers.** Combining two independent ontologies is an explicit operation that produces a new layer — it is not a structural property of the layer graph. The result is a normal Eigon layer that can be versioned, queried, and stacked like any other.

### 7.2 Resolution

Resource resolution within an execution context is a linear scan from the top of the context's stack to the Core Ontology, stopping at the first definition found. First definition wins. This is computationally simple, cache-friendly, and formally unambiguous.

### 7.3 Immutability

Committed layers are immutable. Once a layer becomes the parent of another layer, it cannot be modified. This provides:

- Snapshot isolation — a snapshot is a pointer to a top layer; the entire history below it is immutable
- Cheap context creation — creating an execution context requires only recording which layer is the top
- Natural versioning — the layer stack is the version history of the ontology

### 7.4 Construction and Commit

A layer under construction is mutable and visible only to its write context. On commit, the layer becomes immutable and is assigned a snapshot identifier. It then becomes visible to subsequent read contexts.

This is a two-phase model:

- **Construction phase** — top layer is mutable, visible only to the write context. The write context sees its own uncommitted writes (read-your-own-writes semantics).
- **Commit phase** — top layer becomes immutable, assigned a snapshot identifier (HLC timestamp), and becomes the new permanent top of the stack.

### 7.5 Conflict Model

**Layer addition conflicts.** Two write contexts attempting to add a layer on top of the same parent represent the primary conflict scenario. Resolution: first commit wins. The second context fails at commit time because its parent is no longer the top of the stack. The second context must be retried on top of the newly committed layer. This is standard optimistic concurrency — detect conflict at commit, retry on failure.

**Resource conflicts within a layer.** Cannot occur. Each write context owns its own mutable top layer during construction. A layer is written by exactly one context.

---

## 8. Execution Context

### 8.1 Definition

An execution context is the complete description of the environment in which a single unit of computation occurs. Every operation in Eigenius — a query, a validation, a capability invocation, a layer commit — executes within a context. Both the storage layer and the capability protocol are defined in terms of execution contexts.

Formally, an execution context is a tuple:

```
Context = (
    id:           ContextIdentifier,
    parent:       Option<ContextIdentifier>,
    top_layer:    LayerIdentifier,
    mode:         ReadOnly | ReadWrite,
    snapshot:     SnapshotIdentifier,
    capabilities: LazyMap<ClassIdentifier, Capability>,
    resources:    ResourceCache,
    transaction:  Option<Transaction>,
    constraints:  ExecutionConstraints,
    provenance:   ProvenanceMetadata
)
```

### 8.2 Fields

**id.** A unique identifier for this execution context. Used for provenance tracking, audit, conflict detection at commit, and linking reasoning traces to the context that produced them.

**parent.** The context that spawned this one, if any. Capabilities may spawn sub-contexts for their internal operations. The parent chain forms the provenance structure of a computation — a reasoning trace can be traced back to the original initiating context through this chain. This is where the self-reflective architecture becomes concrete.

**top_layer.** The layer at the top of the stack for this context. The entire layer stack is implicit — following the parent chain of layers from top_layer down to the Core Ontology gives the complete ordered stack. The full stack is not materialized in the context object; only layers actually touched during execution are loaded.

**mode.** ReadOnly contexts bind to a committed layer and cannot write resources. ReadWrite contexts have a mutable top layer under construction and may commit new resources.

**snapshot.** The HLC (Hybrid Logical Clock) timestamp identifying the committed state of top_layer. For ReadOnly contexts, this is fixed at context creation and does not advance during execution — the context's view of the ontology is stable for its lifetime. For ReadWrite contexts, the snapshot advances on commit.

HLC is chosen over pure logical clocks because it maintains causality across distributed nodes while remaining close to wall-clock time, which matters for reasoning trace provenance and audit.

**capabilities.** A lazy map from ontology class identifiers to their registered capability implementations. Populated from the layer stack: capabilities registered in higher layers shadow those in lower layers. Resolution is lazy — the first dispatch to a class triggers resolution and caches the result for the context's lifetime. A missing capability registration is a hard failure, not a silent fallback.

**resources.** A per-context resource cache tracking materialization state for each resource encountered during execution. States: Unknown, Known-Pending, Loading, Materialized. Capabilities receive lazy handles by default; explicit materialization is requested on demand. This allows the query planner to control what is loaded and when, rather than materializing entire subgraphs speculatively.

**transaction.** Present only for ReadWrite contexts. Manages the write set, conflict detection at commit, and rollback on failure. Optimistic concurrency: writes are buffered, conflicts detected at commit time.

**constraints.** Hard limits enforced by the context: maximum resources materialized, maximum wall-clock duration, maximum resources written, maximum sub-context nesting depth. These are safety constraints. In v1, EigenQL is a conjunctive query language whose evaluation terminates trivially — but program execution involves external calls (LLM APIs, document processors) with unbounded latency. Constraints bound the operational cost of execution where the type system cannot. When EigenQL is extended with recursive rules (see §5.6), these constraints also serve as termination backstops for fixpoint evaluation. They are not quality-of-service parameters.

**provenance.** Metadata linking this context to its initiator: the requesting principal, the wall-clock creation time, the initiating request identifier. Combined with the parent chain, this provides a complete audit trail for any computation.

### 8.3 Context Lifecycle

```
Creation → Execution → [Commit | Rollback] → Termination
```

**Creation.** The context is instantiated with a specific top_layer and mode. For ReadOnly contexts, the snapshot is fixed at the committed state of top_layer. For ReadWrite contexts, a new mutable layer is opened on top of top_layer.

**Execution.** Queries, capability invocations, and resource accesses occur within the context. All operations see a consistent view of the ontology as of the context's snapshot. Sub-contexts may be spawned by capabilities; these inherit the parent context's snapshot but have independent resource caches and constraint budgets.

**Commit (ReadWrite only).** The mutable top layer is finalized, assigned a snapshot identifier, and atomically made visible to subsequent contexts. If a conflicting layer has been committed since this context's creation, the commit fails and the context must be retried.

**Rollback.** On failure or explicit cancellation, all writes are discarded. The stack is unchanged. The context's provenance record is retained for audit purposes even on rollback.

### 8.4 Interdependencies Between Capability Protocol and Storage Layer

The capability protocol and storage layer are not independently designed systems that interface at a boundary. They share fundamental concepts that must be defined once and honored by both:

**Resource handles.** The storage layer traffics in handles — references to resources that may or may not be loaded. The capability protocol defines whether capabilities receive handles or materialized resources, and how they request materialization. This is the primary interface between the two systems.

**Execution context as shared primitive.** Every storage operation and every capability invocation executes within a context. The context is the unit of consistency — a capability executing within a context has access to exactly the resources and capabilities visible within that context's snapshot, no more.

**Consistency boundaries.** The storage layer defines what consistency it guarantees. The capability protocol defines what capabilities may assume. These must be aligned — a capability that assumes strong consistency on top of an eventually consistent storage configuration is a correctness bug.

**Transactions.** Capabilities that write resources — recording reasoning traces, updating capability registrations — must do so transactionally. A capability that partially executes and fails must leave the store in a consistent state. The transaction model is shared between the storage layer and the capability protocol; it is not defined separately by each.

**Query planning.** In v1, EigenQL queries are conjunctive pattern matches with filters — they do not invoke capabilities during evaluation. The query planner optimizes pattern matching and resource materialization order based on the storage layer's indexing structure. When EigenQL is extended with recursive rules (§5.6), rule evaluation may trigger capability dispatch, at which point the query planner will need to incorporate capability invocation cost into its planning decisions.

---

## 9. Capability Protocol

### 9.1 Design Principle

The kernel provides a small, fixed set of foundational primitives. Everything else — programs, Components, EigenQL itself, ESL, and constructive type theories — is a registered capability expressed within the ontology and associated with specific classes. New computational capabilities and sublanguages are introduced by extending the ontology, not by modifying the kernel.

This means the kernel is not a system that knows about programs and Components. It is a system that knows how to host things that describe themselves as programs and Components.

**Bootstrap exception.** The kernel contains a minimal hardcoded loader for the Foundation Layer and a fixed capability lookup primitive, as described in §2.5. These are the only places where the kernel has hardcoded knowledge of specific capability semantics. They exist solely to resolve the bootstrap circularity — once the Foundation Layer is loaded, all subsequent capability registration and dispatch proceeds through the standard protocol.

### 9.2 Class-Anchored Extension

Each class in the ontology may be associated with:

- An **evaluator** — a component that knows how to execute instances of that class
- A **validator** — a component that knows how to type-check instances of that class beyond Eigon's structural typing
- A **parser** — a component that knows how to parse a surface syntax associated with that class into Eigon resources

Examples:

| Class | Evaluator | Validator | Parser |
|---|---|---|---|
| Program | program execution engine | program type-checker | ESL program/expression keywords |
| EigenQL query | Query engine | Query well-formedness checker | EigenQL grammar |
| Lean4Proof | Lean 4 kernel | Proof term checker | Lean surface syntax |
| Component | Component dispatcher | Component type-checker | ESL component keyword |

### 9.3 Registration

Capability registrations are ontology resources. They associate a class with its evaluator, validator, and parser implementations. As ontology resources, they:

- Live in the distributed store under the namespace of their registering layer
- Participate in the layer system — a capability registered in a higher layer shadows one in a lower layer
- Are queryable via EigenQL — the system can reason about its own capabilities
- Are versioned and audited like any other ontology resource

### 9.4 Dispatch

When the kernel encounters an instance of a class and needs to dispatch to a registered capability, it:

1. Queries the capability registry for the class (lazy, cached within the context)
2. Verifies the capability is accessible within the current execution context's snapshot
3. Instantiates a sub-context for the capability invocation, inheriting the parent's snapshot
4. Invokes the capability with the resource handle and sub-context
5. Records the invocation in the provenance chain

A missing capability registration is a hard failure. The system does not silently degrade.

### 9.5 Capability Trust and Foundation Capabilities

The capability protocol allows higher layers to shadow capabilities registered in lower layers (§9.3). This creates a trust concern: a domain ontology could replace the program type checker or EigenQL evaluator with an implementation that violates the formal guarantees established by the Lean 4 proofs.

To prevent this, the Foundation Layer capabilities — EigenQL evaluation, program validation, ESL parsing — are designated as **foundation capabilities**. The kernel enforces the following invariant: capability registrations under the `urn:eigenius:foundation:` namespace cannot be shadowed by registrations in non-foundation layers. This parallels the immutability protection on the Core Ontology (§3.7) but applies at the capability level rather than the resource level. A domain layer may register new capabilities for new classes, but it may not replace the foundation capabilities that provide the system's core formal guarantees.

Custom capability registrations for domain-specific classes are unrestricted. The trust boundary applies only to foundation capabilities whose correctness is load-bearing for the entire system.

### 9.6 WASM Capability Sandboxing

Non-foundation capabilities registered by domain ontologies — custom validators, specialized evaluators, data transformation functions, external system adapters — may originate from untrusted or semi-trusted sources. The kernel executes these capabilities as **WASM modules** instantiated via Wasmtime, providing three isolation guarantees:

**Memory isolation.** Each capability instance runs in its own WASM linear memory. A buggy or malicious capability cannot read or corrupt the kernel's memory, other capabilities' state, or the storage layer. The kernel passes data to the capability through the WASM module's import/export interface and receives results the same way — there is no shared mutable state.

**Bounded execution.** The kernel enforces wall-clock and instruction-count limits on capability invocations via Wasmtime's fuel mechanism. A capability that enters an infinite loop or consumes excessive resources is terminated, and the dispatch returns an error through the standard `Result` type (§4.5). This complements the program-level `ExecutionConstraints` (§8.2) with per-capability-invocation bounds.

**Interface control.** A WASM capability module imports only the functions the kernel explicitly provides — a narrow interface for reading resources from the current execution context, emitting typed results, and logging. It cannot make network calls, access the file system, or interact with storage directly. If a capability needs external access (e.g., an LLM adapter calling a provider API), it declares this in its capability registration, and the kernel provides a controlled callback that the orchestration layer fulfills.

Foundation capabilities (§9.5) are exempt from sandboxing — they run as native Rust code within the kernel process, since their correctness is formally verified and they are part of the trusted computing base. The WASM sandbox is specifically for the extensibility surface: the domain-specific capabilities that make the system useful but whose code the kernel cannot vouch for.

In browser and edge deployments where the kernel itself runs as WASM (§2.6), capability sandboxing uses the host runtime's WASM instantiation rather than Wasmtime. The isolation guarantees are the same — separate WASM module instances with independent linear memories — but the instantiation mechanism is the browser's or edge runtime's WASM engine.

### 9.7 Constructive Type Theories as Capabilities

The intended interface with Lean 4 and other constructive type theories is through the capability protocol. A Lean 4 proof kernel is registered as a capability associated with a class representing Lean 4 proof terms. When the system encounters a resource of that class — a proof obligation, a type-checking request — it dispatches to the Lean 4 capability.

This means formal verification is not a special privileged operation in the kernel. It is an instance of the general capability protocol, making the system open to any constructive type theory that can be wrapped as a capability — Lean 4, Coq/Rocq, Agda, or systems not yet conceived.

---

## 10. Distributed Storage Architecture

### 10.1 Design Orientation

The knowledge graph and ontology definitions are expected to exceed available RAM. The storage architecture is modeled after a distributed database system, not a heap-resident graph library. This is an architectural commitment, not a future scaling concern — shortcuts appropriate for in-memory systems would become lies at scale and are excluded from the design from the outset.

**Storage/compute separation.** The architecture separates distributed data storage from reasoning and query evaluation. Storage — replication, consensus, partitioning, durability — is delegated to a dedicated distributed storage engine. Reasoning — type checking, NbE evaluation, EigenQL query planning and execution, program validation — runs in stateless Eigenius computation nodes that talk to the storage cluster. This separation has several consequences: storage and computation scale independently; computation nodes are stateless and horizontally scalable; the storage engine carries no semantic opinions about Eigenius's data model, query language, or type system; and the operational complexity of distributed storage (Raft groups, rebalancing, compaction) is isolated from the complexity of semantic evaluation (type inference, partial evaluation, layer-aware resolution). The storage engine provides ordered keys, range scans, and transactions. Eigenius builds everything else.

### 10.2 Write Profile

The storage layer is optimized for a specific write profile:

- **Ontology layer additions** — infrequent, large, atomic, high-stakes. A small number of large transactions committing new ontology layers.
- **Reasoning trace recording** — frequent, small, append-oriented. Many small writes as LLM processing generates provenance records.
- **Capability registration changes** — infrequent, coordinated, requiring strong consistency.
- **Reads** — dominant by volume. The vast majority of execution contexts are read-only queries and validations.

The storage layer is optimized heavily for reads and append-style writes, with full read-write transactions reserved for ontology layer management.

### 10.3 Snapshot Isolation

The baseline consistency model is snapshot isolation. A read-only execution context binds to a fixed snapshot at creation time and sees a consistent view of the ontology for its entire lifetime. Write contexts commit atomically, with conflict detection at commit time.

The snapshot identifier is an HLC timestamp, providing causality tracking across distributed nodes while remaining close to wall-clock time.

### 10.4 Structural Properties

**Immutable committed layers.** Committed layers are never modified. The store is append-oriented at the layer level. This provides snapshot isolation structurally and makes the storage layer's consistency obligations substantially simpler.

**Shared layer prefix structure.** The global layer structure is a tree: multiple execution contexts with different top layers share all committed layers below their point of divergence. The storage layer exploits this sharing explicitly — common layer prefixes are stored once. Physically, the layer tree is a trie over the ontology's version history, where each path from root to leaf corresponds to a single execution context's linear stack (see §7.1).

**Layer-oriented indexing.** The primary indexing structure is organized around layers, not individual resources. This matches the access pattern: most queries resolve resources by scanning the layer stack from top to bottom, making layer-local indexes the highest-value optimization target.

### 10.5 Reasoning Trace Write Path

Committed layers are immutable (§7.3), but reasoning traces are "frequent, small, append-oriented" writes (§10.2). These two facts require an explicit reconciliation.

Reasoning traces accumulate in the mutable top layer of a ReadWrite execution context during program execution. They are not individually committed as separate layers — that would be prohibitively expensive for the expected write frequency. Instead, when a program execution completes and its context commits, all reasoning traces generated during execution are committed atomically as part of the new immutable layer.

This means reasoning traces generated during execution are invisible to concurrent read-only contexts until the executing context commits. For long-running program executions, this creates a visibility window: traces exist but are not queryable from outside the executing context. This is an acceptable trade-off — reasoning traces are provenance records, not coordination mechanisms. If real-time trace visibility is needed (e.g., for live monitoring of a running pipeline), a dedicated streaming interface outside the layer system should be provided. The layer system captures the authoritative, committed record.

### 10.6 Storage Interface

The kernel defines an abstract storage interface. The host layer (§2.2) provides the concrete implementation. This separation is the architectural mechanism that makes the same kernel deployable across server, edge, browser, and embedded environments — each host implements the storage interface against its platform's available storage primitives.

**The kernel's abstract storage interface consists of:**

**LayerStore** — the primary interface for the layer system:
- `resolve_layer(id) → LayerMetadata` — retrieve a layer's metadata (parent, namespace, snapshot identifier, commit status)
- `read_resource(layer_id, uri) → Option<Resource>` — read a single resource from a specific layer
- `scan_layer(layer_id, class_filter) → ResourceIterator` — iterate resources in a layer, optionally filtered by class
- `commit_layer(layer_id, resources, traces) → Result<SnapshotId, ConflictError>` — atomically commit a mutable layer with its resources and reasoning traces
- `resolve_stack(top_layer_id) → LayerIterator` — iterate the layer stack from top to Core Ontology (for resource resolution)

**CapabilityStore** — the interface for capability registration lookup (used by the kernel-resident lookup primitive, §2.5):
- `lookup_capability(class_id, layer_stack) → Option<CapabilityRegistration>` — find the capability registration for a class, scanning the layer stack top-down

**BlobStore** — the interface for large content (document bodies, LLM response payloads, proof terms):
- `store_blob(content) → BlobId` — content-addressed storage; returns a hash-based identifier
- `fetch_blob(blob_id) → Bytes` — retrieve blob content by identifier

All interfaces are asynchronous from the kernel's perspective — the kernel issues a request and the host provides a response via callback. The kernel never blocks on I/O.

### 10.7 Storage Backend Implementations

The abstract storage interface (§10.6) admits multiple backend implementations, each suited to different deployment models (§2.6):

**RocksDB backend (server, single-node).** RocksDB is an embedded, persistent, ordered key-value store (LSM tree). It provides the exact primitive the storage interface requires: ordered byte-string keys, prefix scans, and write batches. Resources are stored as CBOR (RFC 8949) for compact serialization. The key scheme (see design doc D4) uses layer-prefixed keys for efficient range scans. RocksDB is the primary production backend for single-node deployments — zero administration, automatic compaction, concurrent read/write access. Same key encoding as TiKV (which uses RocksDB internally), so migration to distributed storage is a data copy.

**TiKV backend (server, distributed, future).** TiKV is a distributed, transactional, ordered key-value store built on RocksDB — originally extracted from TiDB (PingCAP) and now a CNCF graduated project. It adds Raft-based replication, multi-region sharding, and distributed transactions (MVCC via the Percolator protocol) on top of the same ordered key-value model. The same key encoding used by the RocksDB backend works directly with TiKV. TiKV becomes relevant when horizontal scalability and replication are needed — for single-node deployments, RocksDB is simpler and has no operational overhead.

**IndexedDB backend (browser).** For browser deployments, layers and resources are stored in IndexedDB object stores. Transactions use IndexedDB's built-in transaction model. Suitable for small-to-medium knowledge graphs (tens of thousands of resources). The immutable layer model fits naturally — committed layers are written once and never modified.

**In-memory backend (testing, embedded).** A simple hash-map-based implementation for unit testing, development, and small embedded deployments where persistence is not required. Layers are in-memory data structures; commits are atomic swaps.

**Content-addressed layer storage.** Regardless of the backend, committed layers can be content-addressed — identified by the hash of their contents. This enables deduplication across the shared layer prefix structure (§10.4), efficient comparison (two layer stacks that share a prefix can be identified by comparing hashes rather than scanning contents), and distribution (layers can be fetched from any node that has the hash, enabling peer-to-peer ontology sharing). Content addressing is an optimization on top of the abstract interface, not a requirement — the storage interface works with opaque layer identifiers.

### 10.8 Indexing Strategy

The storage engine (RocksDB or TiKV) provides ordered keys and range scans. All semantic indexing is built and maintained by the Eigenius host layer, stored as key-value entries in the storage engine.

**Triple indexes.** Each resource property is stored as a triple: (subject IRI, property IRI, value). Three index orderings are maintained as separate key ranges — SPO (subject → property → object), POS (property → object → subject), and OPS (object → property → subject). SPO supports forward traversal ("given resource X, find all its properties"). POS supports class-based scans and type lookups ("find all resources with property P having value V"). OPS supports reverse traversal ("find all resources that reference resource X"). Each index entry is prefixed by layer identifier, making indexes layer-local.

**Layer-local indexes, query-time composition.** Each committed layer maintains its own triple indexes. The EigenQL evaluator composes results across layers at query time, scanning indexes in layer-stack order (top to bottom) and applying the first-definition-wins resolution rule. This avoids rebuilding global indexes on every layer commit. Since committed layers are immutable, their indexes are written once and never updated.

**Index construction on commit.** When a layer commits (via `commit_layer` in §10.6), the host constructs the triple indexes for that layer's resources and writes them to the storage engine as part of the commit transaction. Index construction is a deterministic function of the layer's contents — the same layer always produces the same indexes — which means content-addressed layers imply content-addressed indexes.

### 10.9 Third-Party Building Blocks

The storage and runtime architecture draws from several mature open-source systems:

| Component | Candidates | Role in Eigenius |
|---|---|---|
| Embedded KV store | RocksDB (Meta, C++) | Single-node persistent storage — ordered keys, prefix scans, LSM tree, zero administration |
| Distributed KV store | TiKV (CNCF, Rust) | Distributed storage (future) — built on RocksDB, adds Raft replication and distributed transactions |
| Binary serialization | CBOR (RFC 8949) | Storage and wire format for resources — compact, fast parsing, deterministic encoding |
| Embedded KV (Rust) | redb, RocksDB (via rust-rocksdb), sled | Native Rust embedded storage for non-WASM deployments |
| Raft consensus (Rust) | openraft | Custom distribution layer option — commit ordering for immutable layers |
| Orchestration runtime | Deno (primary), Node.js (fallback) | TypeScript orchestration layer — program execution, LLM adapters, MCP server |
| Capability sandbox | Wasmtime | WASM isolation for untrusted capability code in the native kernel (§9.6) |
| WASM compilation | wasm-bindgen, wasm-pack | Kernel compilation to WASM for browser and edge deployment targets |
| LLM provider abstraction | Vercel AI SDK | Unified interface across Anthropic, OpenAI, Google, open-source models |
| Tool-use protocol | Model Context Protocol (MCP) | LLM → Core agentic access (§2.3) |
| Formal verification | Lean 4, Lake (build system) | Metatheory specification and proof (§2.4) |
| Proof-carrying code | Verus | Rust kernel proof annotations (§2.1) |

The architecture does not mandate specific third-party dependencies. The storage interface (§10.6) and host interface (§2.2) are the abstraction boundaries — concrete implementations are swappable. The table above identifies the candidates that best match the design constraints as of this writing.

**Long-term engine option.** The immutable-layer model has a write profile simpler than what a general-purpose distributed transaction engine provides. Layer construction is single-writer; only commit is a coordination point; committed data is read-only. RocksDB handles this profile well for single-node deployments. For distributed deployments, TiKV provides the standard path. A lighter-weight custom engine — RocksDB per node with Raft consensus for commit ordering (openraft) and content-addressed layer replication — could avoid TiKV's full MVCC overhead for this workload. RocksDB is the pragmatic choice; TiKV or a custom engine are future options once access patterns are empirically understood.

---

## 11. Reflection Layer

### 11.1 Purpose

The reflection layer is the architectural feature that distinguishes Eigenius from a conventional knowledge graph or AI orchestration system. It provides the substrate for the system to reason about its own reasoning formally, not merely to log it.

### 11.2 Reasoning Traces as Typed Resources

LLM reasoning steps are first-class typed resources in the same knowledge graph as the domain they reason about. A reasoning trace is not an opaque log entry — it is an Eigon resource with:

- A type (class) that describes what kind of reasoning step it represents
- Typed properties capturing the inputs, outputs, assumptions, and alternatives considered
- Provenance links to the execution context and parent reasoning steps that produced it
- Optional links to formal proof terms if the step is backed by a constructive type theory capability

Because reasoning traces are typed resources, they are queryable via EigenQL alongside domain resources. The system can ask: which conclusions depend on assumptions that have known limitations? Which reasoning steps are formally verified? Where does the system's self-model lack formal backing?

### 11.3 Epistemic Categories

Every resource in the knowledge graph falls into one of four epistemic categories — declared, observed, derived, or verified — determined by its provenance chain and the presence or absence of formal proof terms. Resources declare their status via epistemic base classes (see design doc D6b). These categories are queryable — an EigenQL query can filter by epistemic status, and the system can produce an epistemic audit of any conclusion.

**Observed** — a resource that represents a recorded fact with external provenance: a measurement, an experimental result, a published claim, a dataset entry. Its reasoning trace records the source (a paper DOI, a database identifier, a sensor reading) and the ingestion path (who loaded it, when, from where). The system does not vouch for its truth — it vouches for its provenance. An observed resource answers: "this is what was recorded, and here is where it came from."

**Derived** — a resource that was produced by a typed processing pipeline (program) from other resources. Its reasoning trace records the complete derivation: which inputs, which pipeline steps, which Component invocations, what intermediate results. The type system guarantees the pipeline was well-formed; the reasoning trace guarantees the derivation is replayable. A derived resource answers: "this follows from those inputs through this process." The derivation can be audited, replayed, and challenged — but it is not formally proved.

**Verified** — a derived resource that additionally carries a formal proof term, checked by a constructive type theory capability (§9.7). The proof term is a machine-checked certificate that the conclusion follows from the premises by the rules of the type theory. A verified resource answers: "this is mathematically certain given those axioms." The proof term itself is a typed Eigon resource (§11.5), queryable and auditable.

The epistemic category is not a static label — it is determined by the resource's base class membership and provenance. A resource is verified if it carries a `VerifiedResource` class with a checked proof term. It is derived if it was produced by a program (`DerivedResource`). It is observed if it was ingested from an external source (`ObservedResource`). It is declared if it was asserted by a human (`DeclaredResource`). A resource's category can be promoted (from derived to verified, by attaching a proof) but never demoted. The reflection layer enforces that epistemic category transitions are monotonic and auditable.

### 11.4 Universe Stratification

The ontology describes itself, programs are typed expressions, and programs operate on the ontology. This creates potential for self-referential paradox. The reflection layer enforces universe stratification to prevent inconsistency.

Ontology resources exist at levels. A resource at level N may describe and reference resources at levels 0 through N-1. It may not describe resources at its own level or higher.

The Core Ontology is at level 0. Domain ontologies derived from it are at level 1. Reasoning traces about domain resources are at level 2. Reasoning traces about reasoning traces are at level 3. The self-reflective graph does not collapse these levels — it maintains a principled tower where each level can describe the one below it.

**Level assignment.** Universe levels are determined by the namespace and class of a resource, not by explicit annotation. The Core Ontology (`urn:eigenius:core:`) is fixed at level 0. The Foundation Layer (`urn:eigenius:foundation:`) is fixed at level 0. Domain ontology resources are at level 1. Resources whose class is a reasoning trace class (defined in the Foundation Layer under `urn:eigenius:foundation:reflection:`) are at level 2 or higher, determined by the level of their `about` property's target: a trace about a level-N resource is at level N+1. The level assignment algorithm is kernel-resident and runs at resource ingestion time.

**Enforcement.** Level violations are detected at ingestion time as structural validation errors, the same mechanism that enforces namespace boundaries and class constraints. A resource at level N that declares `is_a` including a class at level N or higher, or that has a property whose value is a resource at level N or higher that the property semantically "describes" (as opposed to merely "references"), is rejected. The distinction between "describes" and "references" is made by a property-level annotation (`reflective: true`) on properties that create descriptive relationships. The `about`, `describes`, and `conclusion` properties on reasoning trace classes carry this annotation. Plain reference properties (like `input_class` or `component`) do not — a reasoning trace may reference the component it describes without violating stratification.

**Query across levels.** EigenQL queries may join resources across levels — a query can retrieve domain resources and their associated reasoning traces in a single query. The stratification constraint governs *authorship* (what level a resource is written at), not *visibility* (what a query can see). This is analogous to how a program can read its own source code but cannot modify the compiler that compiled it.

This stratification has practical consequences: it determines at what level new resources can be authored, governs how reasoning traces reference the ontology that typed them, and prevents classes of bugs where a self-modifying ontology invalidates its own schema.

### 11.5 Proof Terms as Resources

As the Lean 4 formal track matures, proof terms become first-class Eigon resources. A proved lemma has a statement (a proposition), a proof term (a computation), and metadata (dependencies, authorship, version). All of these are expressible as Eigon resources under the `urn:eigenius:proofs:` namespace. EigenQL can then query across domain, reasoning traces, and the proof graph in a unified way.

This is a Phase 3 capability — not v1 scope — but the architecture is designed from the outset to leave this path open. The stratification model already accommodates proof resources at appropriate universe levels, and the capability protocol supports a Lean 4 proof kernel as a registered capability.

---

## 12. Program Execution Model

### 12.1 Concurrency

The program execution model is sequential by default. Steps in a Sequence execute in order, each seeing the results of all prior steps in the execution context.

**Map parallelism.** The Map combinator applies a Sequence to each element of a collection "independently" — there are no data dependencies between iterations. The execution engine *may* execute Map iterations concurrently at its discretion. Each iteration runs in an independent sub-context that inherits the parent context's snapshot but has its own resource cache. Iterations do not share mutable state. The Map combinator collects results in input order regardless of execution order.

**No inter-step parallelism.** Steps within a Sequence are not executed concurrently. The type system depends on the execution context growing monotonically as steps execute — concurrent steps would create ambiguity about context state. If a use case requires parallel execution of independent steps, it should be expressed as a Map over a collection of tasks, making the parallelism boundary explicit and type-safe.

**Capability concurrency.** When the execution engine invokes a Component, the Component may internally use concurrency (e.g., parallel HTTP requests within an LLM adapter). This internal concurrency is invisible to the program type system — it is the Component's implementation concern, bounded by the sub-context's `ExecutionConstraints`.

### 12.2 Error Recovery and Retry

The program type system requires explicit error handling via `Result<A, E>` types (§4.5). This addresses error *representation* but not error *recovery*. The following mechanisms complement the type system:

**Retry policy as Component metadata.** Components that interact with external services (LLM APIs, network services) may declare a retry policy as part of their ontology registration. The retry policy specifies: maximum retry count, backoff strategy (fixed, exponential), retryable error classes (e.g., rate limiting, transient network failure), and a timeout per attempt. The execution engine applies the retry policy transparently — from the program's perspective, the Component either succeeds or returns an error `Result`. Retries are recorded in the reasoning trace for provenance.

**No program-level checkpointing in v1.** A failed program execution does not automatically resume from the last successful step. Full checkpointing requires serializing the execution context (including all intermediate step outputs) and restoring it, which is a significant engineering effort that can be deferred. For v1, a failed program is re-executed from the beginning. The reasoning trace from the failed execution is retained for diagnosis.

**Idempotency annotation.** Components that have external side effects (writing to a document store, sending an API call) may declare `idempotent: true` or `idempotent: false` in their registration. The execution engine uses this annotation to determine whether retries are safe. A non-idempotent Component that fails is not retried — its error is propagated immediately.

**Compensation is out of scope for v1.** Compensating transactions (undoing the effects of partially executed programs) require a saga pattern or equivalent, which is a substantial addition to the execution model. For v1, programs that require transactional atomicity across external side effects should encapsulate that logic within their Component implementations.

---

## 13. Operational Concerns

### 13.1 Ontology Versioning

The layer stack is the version history of the ontology (§7.3), but this is structural versioning, not semantic versioning. The following policies govern ontology evolution:

**Additive changes are non-breaking.** Adding new classes, properties, or optional (`recommends`) properties to existing classes is always safe. Existing resources remain valid because new optional properties have no presence requirement.

**New required properties are breaking.** Adding a property to a class's `requires` set invalidates all existing resources of that class that lack the property. This is a breaking change.

**Breaking changes require a new namespace version.** A breaking change to `urn:ford:vehicles:Vehicle` produces a new class at `urn:ford:vehicles:v2:Vehicle` (or an equivalent versioning scheme chosen by the namespace owner). The old class remains valid and queryable. A bridge layer may declare `equivalent_to` or migration relationships between versions.

**Migration is explicit, not automatic.** The system does not automatically upgrade resources from one schema version to another. Migration is performed by a program that reads resources of the old class and writes resources of the new class — using the same typed pipeline infrastructure as any other processing task. This keeps the kernel simple and makes migrations auditable through reasoning traces.

### 13.2 Security Model

The v1 security model relies on three mechanisms: namespace governance (§6), capability trust (§9.5), and execution context isolation (§8).

**Namespace governance provides write isolation.** A layer can only write resources within its declared namespace. Cross-namespace writes are structurally impossible. This prevents a malicious or buggy layer from corrupting another namespace's resources.

**Foundation capability protection provides execution integrity.** Foundation capabilities cannot be shadowed (§9.5). A domain layer cannot replace the EigenQL evaluator or program validator with a compromised implementation.

**Execution context isolation provides read boundaries.** A capability invoked within a sub-context can only access resources visible in that context's snapshot. It cannot reach outside its context boundary to access resources in other contexts or at other snapshots.

**Not addressed in v1:** fine-grained access control within a namespace (all resources in a namespace are equally accessible to any context that includes that namespace's layer); authentication of capability invocations (the kernel dispatches to whichever capability is registered, with no caller authentication); and sandboxing of capability code (a registered capability has access to the full execution context and could exfiltrate data through side channels). These are important for multi-tenant deployments and will require additional design work.

### 13.3 Observability

**Reasoning traces as observability primitives.** The reflection layer (§11) provides structured, queryable records of every significant computation. Since reasoning traces are typed Eigon resources, they can be queried with EigenQL to answer operational questions: which steps in a program execution were slowest? Which Components failed most frequently? What was the token usage of a particular LLM call?

**Execution constraints as monitoring hooks.** When an execution context approaches its constraint limits (e.g., 80% of maximum wall-clock duration), the kernel emits a structured warning as a reasoning trace resource. This provides a query-able record of near-limit executions.

**Developer tooling specification is deferred.** The TypeScript layer includes "developer tooling — editors, visualizers, ontology browsers, language servers" (§2.2). The specific design of debugging tools (step-through program execution, intermediate context inspection, capability failure diagnosis) is a separate design effort that builds on the reasoning trace infrastructure defined here.

---

## 14. Open Questions

The following design questions require resolution before implementation begins:

**Storage layer internal architecture.** The abstract storage interface (§10.6), backend implementations (RocksDB for single-node, TiKV for distributed), and key encoding (see design doc D4) are specified. Distribution strategy details (TiKV region placement policies, replication factor tuning, cross-region deployment), compaction approach (how layer stacks are periodically flattened for read performance), and the index cost model for the EigenQL query planner are not yet specified.

**Capability protocol wire format.** How capabilities communicate with the kernel — in-process, IPC, or network — and what serialization format is used for resource handles and results.

**Namespace delegation depth.** Whether sub-namespace delegation is bounded in depth and what the enforcement mechanism is for delegation chains.

**Snapshot advancement.** Whether long-running write contexts may advance their snapshot during construction, and what consistency guarantees apply if they do.

**Capability versioning.** How capability implementations are versioned, how the kernel handles version mismatches, and what backward compatibility obligations capability authors carry.

**ESL extension mechanism.** How new ESL keywords associated with new ontology classes are introduced — whether keyword registration is part of the capability protocol or a separate concern.

**HLC clock synchronization.** The HLC timestamp model (§8.2) assumes bounded clock skew between distributed nodes. The specific skew bounds, the behavior when bounds are violated, and the synchronization protocol are not yet specified.

**Inline resource semantics.** The `resource` datatype permits both IRI references (top-level resources) and nested objects (inline resources without IRIs). The precise semantics of inline resources in EigenQL queries — whether they can be independently matched, how they participate in variable binding, and how they interact with the resource cache — need specification.

**Capability sub-context isolation boundaries.** Sub-contexts inherit the parent's snapshot but have independent resource caches (§8.3). Whether a sub-context can spawn further sub-contexts (and the depth limit), whether concurrent sub-contexts share materialized resources, and the rollback semantics for nested sub-contexts need detailed specification.

**Ontology combination semantics.** Combining two independent ontologies is described as "an explicit operation that produces a new layer" (§7.1). The specific semantics of this operation — handling of overlapping capability registrations, validation of cross-namespace references, and the status of bridge layers in the combined result — need specification.

---

*This document represents the working architectural understanding as of April 2026. It is a living document, expected to evolve substantially as the storage layer architecture, capability protocol wire format, and operational tooling are developed.*
