# Boundary Contracts in Eigenius

**Status:** Draft — starting point for design specification
**Scope:** Design approach for capturing and formalizing the contracts that govern boundaries between the Eigenius kernel, registered institutions, components, storage backends, and external clients.

## 1. Purpose

Eigenius is a platform with many extension points: institutions contributing structured fibers, components dispatching to external services, storage backends persisting traces and layers, clients submitting programs over gRPC. Each extension point is a boundary, and each boundary is a contract — a set of obligations the kernel places on the extension, and guarantees the kernel provides in return.

Today, most of these contracts exist implicitly: in Rust trait signatures, in docstrings, in comments, and in tribal knowledge held by the people who wrote the code. This document proposes a systematic approach for making these contracts explicit, formal where formalization pays off, and evolvable over time without silent incompatibility.

The central claim: **contracts in Eigenius should be first-class typed resources in the knowledge graph they govern**, not external documentation. Because the platform is a self-describing typed knowledge graph, its own quality-control machinery can be expressed in the same primitives that express domain data. This is not merely elegant — it is what makes long-term maintainability tractable as the number of extensions grows.

## 2. Goals and non-goals

### Goals

- Define what a boundary contract **is** in Eigenius — its ontological shape, its required properties, and its relationship to the things it governs.
- Establish the **spectrum of formalization** and provide guidance on which level is appropriate for which kinds of contractual obligations.
- Specify how contracts **version and evolve** under the immutable layer system without breaking existing traces.
- Identify the **initial set of boundaries** that require formal contracts and the order in which they should be addressed.
- Provide a **meta-specification** precise enough that individual contract specifications can be written uniformly against it.

### Non-goals

- This document does not specify any individual contract in full. Those are separate documents.
- This document does not prescribe a complete verification strategy for contract compliance. That is a longer-term piece of work.
- This document does not address contracts between Eigenius and its human operators (deployment, SRE, security review). Those are important but out of scope here.

## 3. What a boundary contract captures

A complete boundary contract specifies eight distinct kinds of obligation. Real contracts will emphasize different subsets, but a contract specification template needs to address each.

### 3.1 Syntactic interface

The method signatures of the boundary — input types, output types, error types. This is what a Rust trait captures today. It is necessary but insufficient: it describes what the boundary accepts without describing what it promises.

### 3.2 Semantic preconditions

Propositions that must hold of inputs before a method is called. Examples: "the input resource must be an instance of the component's declared `argument_type` class"; "the proof term's environment hash must match a registered environment in the current layer chain"; "the query's `FiberQuery` subclass must be one the institution declared at registration."

Some preconditions are expressible at the type level (class membership, structural well-formedness); others are only checkable at runtime (environment hash matching, resource-state invariants).

### 3.3 Semantic postconditions

Propositions the method guarantees about its output given valid inputs. Examples: "the returned resource is an instance of the declared output class"; "if `validate_morphism` returns success, the morphism satisfies all `structural_properties` declared at registration"; "the output's content hash appears in the trace produced by this call."

### 3.4 Determinism and idempotence

Whether the operation is deterministic given the same layer state. Whether it is idempotent. Whether it is safe to cache, retry, or parallelize. This is load-bearing for trace integrity in Eigenius: memoization correctness depends on determinism, and determinism is itself a contractual property that must be declared and honored.

Determinism has shades. An operation may be deterministic modulo a specific layer state, deterministic modulo the layer plus a named random seed, or genuinely non-deterministic (an LLM call without temperature=0). The contract must distinguish these.

### 3.5 Error taxonomy

A closed enumeration of distinguishable failure modes, each with documented semantics. Without this, every failure collapses to "something went wrong," which is operationally useless. Baseline categories:

- **`DomainRejection`** — inputs were well-typed but the institution's logic refused them. Surface to the caller as a normal user-facing error.
- **`StructuralViolation`** — inputs claimed to satisfy the contract's preconditions but did not. This is a caller bug.
- **`InstitutionInternalError`** — something went wrong inside the extension. Log, mark degraded, surface as an opaque failure.
- **`ResourceExhaustion`** — limits exceeded (memory, time, tokens, credit). May be retryable with different bounds.
- **`Unavailable`** — transient external failure. Retry with backoff is appropriate.
- **`VersionMismatch`** — contract version incompatibility detected.

#### Extension by individual contracts

Individual contracts almost always need to refine this baseline. Two patterns recur:

- **Specialization** — refining a baseline category into multiple distinguishable variants. The Lean verification institution refines `DomainRejection` into `ProofDoesNotCheck` (the term doesn't type-check), `PropositionMismatch` (the proof's proposition doesn't correspond to the claim), `EnvironmentMismatch` (the proof was elaborated against a different environment than declared), and `FFIVersionMismatch` (the mirror library doesn't match the current ontology). Each variant is operationally distinguishable and warrants different caller responses; collapsing them into a single `DomainRejection` would discard actionable information.
- **Net-new categories** — failure modes that don't fit any baseline. The Lean institution adds `EnvironmentUnavailable` (the referenced Lean environment couldn't be loaded), which is closer to `Unavailable` than to `ResourceExhaustion` but neither baseline captures it cleanly.

When extending, contracts must:

1. Declare each new variant in the contract's `ErrorEnum` resource (Section 6.4).
2. Specify the diagnostic payload type for each variant.
3. Provide caller-response guidance — what should code do when it encounters this variant?
4. Indicate the variant's relationship to the baseline: is it a specialization of which baseline category, or a genuinely new category?

Specialization variants should remain classifiable back to their baseline category by callers that don't care about the distinction. A caller that only wants to know "did the verification fail for any reason the institution refused, regardless of why?" should be able to query "is this a `DomainRejection` (in any specialization)?" and get a useful answer. The contract specification makes this classification explicit so that callers can choose their level of granularity rather than being forced to handle every variant individually.

### 3.6 Resource consumption bounds

Declared upper bounds on resources the operation may consume: wall time, memory, network bandwidth, tokens, dollar cost, fuel for WASM capabilities. These are advisory at the type level but enforceable at runtime. They also enable planning — a workflow can be analyzed for expected cost before execution.

### 3.7 Lifecycle and versioning

When the boundary is valid to call, what layer states must be active, how the contract evolves, and what happens to in-flight calls when the layer changes. Institutions register against specific contract versions; traces record which contract version was in effect; queries about historical compliance resolve to the contract-as-it-was.

### 3.8 Effect declarations

Which capabilities (in the Pure/Read/IO sense of the kernel's capability modes) the operation requires and which it exercises. Whether it produces traces, reads the layer chain, dispatches network calls, commits resources. This is the institutional-scale extension of the existing component capability-level declarations.

## 4. The spectrum of formalization

Contracts can be captured at several levels of rigor. A single contract typically mixes levels, with different clauses formalized to different depths. The levels, from lightest to heaviest:

### 4.1 Documentation-level

Prose description in markdown, docstrings, or design documents. Binding on humans, not on code. Appropriate for contracts whose violations are rare, recoverable, and spotted in review: naming conventions, internal-only invariants, style guidelines.

**Cost:** low to write, low to maintain.
**Enforcement:** human review.

### 4.2 Schema-level

The contract is expressed as an ontology resource — a typed declaration in Eigon with required properties, enumerated values, and structural constraints. The system can query and validate contracts, but does not mechanically check compliance beyond schema well-formedness.

**Cost:** modest — requires an ontology class and instances.
**Enforcement:** kernel structural validation at registration time.

### 4.3 Runtime-checked-level

The contract includes executable predicates that fire at boundary crossings. Preconditions evaluated before dispatch, postconditions evaluated after. Violations produce typed errors.

**Cost:** moderate — requires predicate implementations and dispatch-time invocation.
**Enforcement:** runtime checks at call sites; violations caught after the fact but reliably.

### 4.4 Type-level

The contract is encoded in EigenTT types the kernel checks. Violations are impossible to express without being rejected. Uses dependent records for input/output types, `NativeDecide` for constraint predicates, `DecEq` for equality obligations, `Id` types for equations that must hold.

**Cost:** higher — requires the contract's obligations to be expressible in the type theory.
**Enforcement:** kernel type checking before dispatch; violations caught as compile-time-equivalent errors.

### 4.5 Proof-level

The contract's satisfaction is demonstrated by a checked proof, produced by a verification institution (Lean 4, Rocq, an SMT checker). Appropriate for the small number of invariants where type-level expression isn't rich enough or where the property must survive refactoring across implementations.

**Cost:** highest — requires writing and maintaining formal proofs.
**Enforcement:** verification institution checks the proof; proof term stored as a typed resource.

### 4.6 Choosing a level per clause

The right discipline is **specify once at the richest level the clause is naturally expressed in, and let the system downgrade to weaker mechanisms where the richer one does not apply**. Guidelines:

- **Syntactic interfaces** — always type-level. This is what EigenTT type checking is for.
- **Structural preconditions and postconditions** (class membership, simple constraints) — type-level where expressible via dependent records and `NativeDecide`, runtime-checked otherwise.
- **Quantified properties over finite collections** — type-level once `Map` and `Reduce` primitives are wired to support bounded quantification; runtime-checked in the interim.
- **Determinism and idempotence** — schema-level declarations, enforced structurally by requiring layer-parameterized signatures and runtime cache-key consistency checks.
- **Error taxonomy** — always type-level. Closed sum types over the error enum.
- **Resource bounds** — schema-level declarations, runtime-enforced via instrumentation. Not expressible in EigenTT.
- **Probabilistic and timing bounds** — runtime-checked only. Declared as advisory at schema level.
- **Lifecycle and versioning** — schema-level, enforced by the layer system's immutability guarantees.
- **Full correctness of algorithms** — proof-level via verification institution. The verification institution's own contract is itself a candidate for proof-level treatment.

## 5. Contracts as typed resources in the knowledge graph

The structural decision: contracts are Eigon resources, classed by contract classes in the ontology, committed to the immutable layer stack. This is not decoration. It has several consequences that matter operationally.

### 5.1 Queryability

Because contracts are resources, EigenQL can operate over them. Questions the system can answer directly:

- Which institutions claim determinism?
- Which contracts declare that their operations produce traces?
- Which contract versions are active in the current layer chain?
- For a given trace, what contract governed the institution that produced it?
- Which contracts require proof-level verification of their implementations?

These are not special-purpose queries requiring bespoke tooling. They are ordinary EigenQL over ordinary resources.

### 5.2 Immutability and auditability

A contract committed to layer N cannot be silently changed. Evolution happens by committing a new version in a later layer. Traces record which contract version was in effect at production time. "Was institution X compliant with the contract it claimed, at the time it produced this trace?" becomes a query against the historical layer state, not an archaeological investigation.

### 5.3 Self-description

The contract classes are themselves ontology resources, typed by the Core Ontology. The system's quality-control machinery is expressed in the same primitives as the system's domain data. The self-describing property the platform already commits to extends recursively to its own contracts.

### 5.4 Bounded trust scope

Each contract has its own trust scope — its own TCB — owned by whoever implements the contract, not by the kernel. The kernel's TCB consists only of the kernel's own code: the type checker, the layer system, the institution registry, the dispatch machinery. The kernel's TCB does not grow when an institution registers; it stays minimal regardless of how many institutions are active or how complex their implementations are.

An institution implementing a contract owns the contract's TCB. For the Lean verification institution, this includes the Lean term checker (a Rust library, currently nanoda_lib), the EigonFFI generator, and the correspondence logic in `validate_morphism`. For an FEA institution, it would include the underlying solver and any pre- or post-processing code. For an LLM-based component, it would include whatever validation logic stands between the LLM's output and the typed result the component returns.

This bounded-scope property has two operational consequences. First, the **blast radius of bugs** is contained: a bug in an institution's TCB can produce wrong results in that institution's fiber, but cannot corrupt the kernel, the ontology, or other institutions' fibers. A buggy proof checker can accept invalid proofs as verified; it cannot make the kernel mis-typecheck a pipeline or alter a layer's content. Second, **audit and review can be scoped to individual institutions** without auditing the entire platform. An organization that needs to review the trust assumptions for verification can review the verification institution's TCB without simultaneously reviewing every other institution's implementation.

The contract specification makes the institution's TCB scope explicit. The `InstitutionRegistration` resource (Section 6.6) and its associated `ImplementationManifest` (Section 6.7) record the components that constitute the institution's implementation precisely so that "what was in the TCB at the time of this trace?" is a queryable property of the system rather than an archaeological exercise.

### 5.5 Versioning model

Contracts evolve by layer extension. A contract resource is content-addressed; its IRI is stable; a new version is a new resource at a new IRI (or at the same IRI in a later layer, depending on the chosen evolution policy). Institutions register against a specific contract IRI, which resolves to a specific content-addressed resource via the layer chain at registration time. This resolution is pinned in the institution's own registration record.

Two evolution policies need to be specified:

- **Additive evolution** — new versions may add optional clauses but may not strengthen existing obligations. Backward-compatible by construction.
- **Breaking evolution** — new versions may change obligations in ways that invalidate prior compliance. Requires explicit migration for existing institutions.

The default should be additive; breaking evolution should require a new contract class rather than a new version of an existing one.

**Compositionality under version evolution.** Layer-ancestry compositionality applies to contract evolution as well as to ontology evolution. An artifact produced under contract version V₁ — a registered institution, a trace, a generated library — remains valid for queries made under any descendant contract version V₂ ⊒ V₁ where the clauses relevant to the artifact are unchanged. This is what makes additive evolution genuinely backward-compatible: existing artifacts don't need migration unless they want to claim compliance with newly-added clauses. The same compositionality property generalizes to artifacts produced *by* institutions under their contracts — the Lean verification institution exploits it when an EigonFFI library anchored at one ontology layer remains valid for verifying claims in descendant layers where the relevant classes are unchanged.

### 5.6 Trusted artifacts and anchoring

Some institutions don't only produce results at runtime — they produce *trusted artifacts* with persistent existence: generated libraries, schemas, certificates, derived ontology fragments. The Lean verification institution generates EigonFFI libraries; future institutions may generate code stubs, JSON schemas, regulatory certification bundles, or other persistent content that downstream consumers depend on.

The contract framework treats such artifacts as first-class typed resources committed to the knowledge graph, not as files maintained outside it. Each artifact carries declarative provenance:

- The **source layer** it was generated from (when the artifact's content depends on ontology state).
- The **generator identity** (which tool, which version, identified by content hash) that produced it.
- The **content hash** of the artifact itself.
- Optionally, the **content** embedded directly or referenced via content-addressed external storage.

This treatment unlocks two properties that are valuable beyond any single institution.

**Independent provenance verification.** When the generator is deterministic — a design requirement for any generator in an institution's TCB — an auditor with access to the generator binary and the source layer can re-run generation locally and compare hashes. If they match, the artifact is authentic and its declared anchor is truthful. If they diverge, something is wrong: non-determinism in the generator, tampering with the committed artifact, or environment differences that affect generation. The audit is a local computation requiring no trust in any party.

**Anchoring under layer extension.** A generated artifact anchored to source layer L₀ remains valid for use in any descendant layer L₁ ⊒ L₀ where the parts of the ontology the artifact depends on are unchanged. This is the same compositionality property the platform uses for its own layer chain, applied to generated content. Users do not need to regenerate every time anything in the ontology changes — only when changes affect the specific classes the artifact mirrors. The institution's contract specifies what counts as "affecting the artifact" and what does not.

The ontology class for tracked artifacts (`GeneratedArtifact`, Section 6.8) and the contract obligations on generators in TCB (determinism, faithful translation per institution-specific specifications) make this pattern reusable across any institution that needs to produce trusted persistent content.

## 6. Ontology sketch

This section sketches the ontology classes a full specification will define in detail. Names and property lists are illustrative; the final specification will pin them down.

### 6.1 `BoundaryContract`

The abstract base class for all boundary contracts. Required properties:

- `contract_name` — human-readable name
- `interface_version` — IRI identifying the syntactic surface
- `declared_operations` — array of `OperationContract` resources
- `error_taxonomy` — IRI pointing to the `ErrorEnum` for this contract
- `lifecycle_policy` — one of `Additive`, `Breaking`
- `contract_version` — semver-like identifier
- `parent_contract` — optional reference to the previous version

### 6.2 `OperationContract`

A single method or operation within a contract. Required properties:

- `operation_name` — identifier within the contract
- `input_type` — IRI to the input class
- `output_type` — IRI to the output class
- `effects` — array of effect descriptors (Pure, Read, IO, TraceProducing, NetworkAccess, etc.)
- `determinism_class` — one of `Deterministic`, `DeterministicModuloLayer`, `DeterministicModuloSeed`, `NonDeterministic`
- `idempotence_class` — one of `Idempotent`, `NonIdempotent`
- `preconditions` — array of `ContractPredicate` resources
- `postconditions` — array of `ContractPredicate` resources
- `resource_bounds` — optional `ResourceBoundDeclaration`

### 6.3 `ContractPredicate`

A predicate expressed at some level of formalization. Required properties:

- `formalization_level` — one of `Documentation`, `Schema`, `Runtime`, `TypeLevel`, `ProofLevel`
- `description` — prose description (always required)
- `minitt_expression` — EigenTT term encoding the predicate (required for TypeLevel and above)
- `runtime_checker` — IRI to a component implementing the check (required for Runtime and above)
- `proof_obligation` — reference to a verification institution and proof term (required for ProofLevel)

A single predicate may be captured at multiple levels simultaneously — the prose description is always required, and stronger levels are added as formalization deepens.

### 6.4 `ErrorEnum` and `ErrorVariant`

`ErrorEnum` declares the closed set of failure modes for a contract. Each `ErrorVariant` carries:

- `variant_name` — identifier
- `description` — prose
- `diagnostic_payload_type` — IRI to a class describing what diagnostic information the variant carries
- `caller_response_guidance` — advisory guidance on how callers should respond

### 6.5 `ResourceBoundDeclaration`

Advisory declarations of resource consumption. Properties include optional `max_wall_time_ms`, `max_memory_bytes`, `max_network_bytes`, `max_llm_tokens`, `max_dollar_cost`. Missing properties mean "no declared bound." Enforcement is at runtime via instrumentation.

### 6.6 `InstitutionRegistration` (and similar for other extensions)

When an institution registers, it does so against a specific `BoundaryContract`. The registration itself is a typed resource:

- `registered_iri` — the IRI under which the institution is reachable
- `contract_reference` — content-addressed IRI of the contract being implemented
- `implementation_manifest` — IRI of an `ImplementationManifest` resource (Section 6.7) listing all hashed components of the institution's implementation
- `registration_layer` — the layer in which this registration was committed
- `claimed_properties` — optional overrides or refinements of contract clauses

### 6.7 `ImplementationManifest`

For institutions whose implementation consists of multiple trusted components — a checker plus a generator plus a correspondence library, for instance — the manifest enumerates each component:

- `manifest_name` — human-readable identifier
- `components` — array of `ImplementationComponent` resources, each carrying:
  - `component_role` — what role this component plays (e.g., `Checker`, `Generator`, `Correspondence`, `CrossChecker`, `Adapter`)
  - `component_hash` — content hash pinning the exact artifact
  - `component_version` — human-readable version string
  - `component_source` — IRI describing where the component came from (an external library with its repository URL, an internally maintained crate, a generated artifact tracked elsewhere in the graph)
  - `tcb_membership` — whether the component is part of the trust base for this contract (`Trusted`, `Advisory`, or `NonTrusted`); some components like cross-checkers may be advisory rather than trusted

Single-component institutions can use a manifest with one entry. The manifest pattern accommodates the common case where an institution depends on external libraries (which evolve independently and need explicit pinning) alongside internally-maintained code, and it makes the institution's effective TCB a queryable property of the registration.

### 6.8 `GeneratedArtifact`

Base class for trusted artifacts produced by institution generators (Section 5.6). Required properties:

- `source_layer_hash` — layer hash the artifact was generated from. Optional for ontology-independent artifacts.
- `generator_reference` — IRI of an `ImplementationComponent` (Section 6.7) identifying the generator that produced this artifact, including its content hash.
- `artifact_content_hash` — content hash of the artifact itself.
- `artifact_content` — optional embedded content or content-addressed external reference for large artifacts.
- `artifact_role` — what role the artifact plays in its institution's operation (e.g., `MirrorLibrary`, `Schema`, `CertificateBundle`, `DerivedOntologyFragment`).
- `mirrored_classes` — for artifacts that mirror or depend on a subset of the ontology, the IRIs of the classes the artifact represents. Supports scoped generations where not every class is included.
- `generated_at` — timestamp, advisory.

Specific institutions specialize this class with additional properties as needed. The Lean verification institution's `GeneratedLibrary` is a specialization of `GeneratedArtifact` carrying Lean-specific fields (Lean version compatibility, FFI library naming conventions, etc.).

## 7. The initial set of boundaries

Not every extension point needs a full contract immediately. The following are the boundaries that warrant formal contracts in the near term, roughly in order of leverage:

### 7.1 `InstitutionContract`

The most load-bearing boundary. Currently expressed as the `FiberReasoner` trait with four operations (`fiber_declaration`, `validate_morphism`, `query`, `discover_morphisms`). A formal contract covers input/output types for each operation, the error taxonomy, determinism requirements, effect declarations, and the lifecycle around fiber declarations and structural property advisories.

The verification institution (Lean 4 checker and any future proof-checking peer) is a specialized refinement — `VerificationInstitutionContract extends InstitutionContract`, with additional obligations around proof term validity, environment management, and the correspondence between proof propositions and Eigon-side claims.

### 7.2 `ComponentContract`

Governs components in the program model. Capability levels (pure/read/io), determinism declarations, fallibility (the `Sum(ok | err)` discipline already in the type theory), input/output resource classes, schema-generation obligations for LLM components, retry semantics for transient failures.

### 7.3 `TraceStorageContract`

Governs the persistence layer for traces and the layer chain. Durability guarantees, atomicity of trace commits, content-addressing invariants, canonicalization requirements for keys, behavior under concurrent commits (the "first commit wins" rule). This contract is the one trace memoization correctness depends on.

### 7.4 `ClientProtocolContract`

Governs the gRPC surface and the embedded CLI. Session semantics, authentication (where applicable), layer-state visibility rules, concurrent-request semantics, trace import/export.

### 7.5 `OntologyExtensionContract`

Governs how new ontology layers are proposed, validated, and committed. Namespace governance, subclass discipline, required properties for domain classes, the relationship between ontology evolution and existing data.

The Lean checker as an institution fits under 7.1 as a specialization. Domain capabilities (FEA solvers, docking engines, molecular property predictors) fit under 7.1 as further specializations.

## 8. Gradual formalization strategy

The temptation with this kind of work is to try to formalize everything at the highest level up front. This is how contract specifications sit half-done for years. The working practice should be gradual and value-producing at each step.

### Phase A — Error taxonomy

Define the baseline `ErrorEnum` and require every institution method to return either its declared success type or a variant of the enum. Replace current panics in dispatch paths with typed error returns. This solves the most immediate operational issue and establishes the pattern.

**Expected cost:** one to two weeks.

### Phase B — Schema-level contract declarations

Define the ontology classes sketched in Section 6. Write a first-pass `InstitutionContract` resource for the existing `FiberReasoner` interface and a `ComponentContract` for the existing component protocol. Declare determinism, effects, and baseline resource bounds. No enforcement beyond structural validation yet.

**Expected cost:** two to three weeks.

### Phase C — Runtime-checked preconditions and postconditions

Implement `ContractPredicate` evaluation at dispatch time. Wire preconditions to fire before a boundary is crossed, postconditions after. Start with simple predicates (class membership, structural well-formedness) and expand as needed.

**Expected cost:** two to three weeks, with a long tail as specific predicates are authored.

### Phase D — Type-level lifting for naturally-typed clauses

Move preconditions and postconditions that are expressible in EigenTT up from runtime to type-level. Hinges on the ontology-as-types work being complete (the `find_sigma_field` layer-chain plumbing). Once complete, class membership preconditions become automatic and many runtime checks collapse.

**Expected cost:** concurrent with the ontology-as-types integration work; primarily additive effort.

### Phase E — Lifecycle and versioning enforcement

Pin institution registrations to specific contract versions. Record contract version in every trace. Implement the queries that answer "was this compliant with its contract at the time?" Requires the layer system to treat contract resources with the same immutability discipline as everything else (which it should automatically — they are ordinary resources).

**Expected cost:** one to two weeks, mostly in tooling and tests.

### Phase F — Proof-level contracts for safety-critical clauses

Identify the small number of invariants that genuinely warrant formal proof: the verification institution's own correctness, key properties of the trace commit logic, perhaps the universe stratification enforcement. Write Lean proofs. Wire the verification institution to check them.

**Expected cost:** open-ended, proceeds as warranted.

Each phase produces value independently. The system is never in a "formalization is in progress but nothing works" state; it is always in a state where some contracts are fully formal, some are schema-and-runtime, and the rest are documented-and-aspirational, with a clear record of which is which.

## 9. The contract meta-specification as a living document

This document is the meta-specification in draft form. A complete meta-specification fills in the details sketched here: pins down the ontology class names and property lists, specifies the error taxonomy baseline precisely, defines the `ContractPredicate` evaluation protocol, and provides enough concrete examples that individual contract specifications can be written by following the pattern.

Individual contract specifications — `InstitutionContract v1`, `ComponentContract v1`, `TraceStorageContract v1`, and so on — are separate documents, each a day or two of work once the meta-specification is firm.

The meta-specification itself evolves under the same discipline it prescribes for the contracts it governs: versioned, committed to the project's own design record, with changes tracked explicitly rather than merged silently. The first concrete step is turning this draft into a `v0.1` meta-specification with the ontology classes pinned and the baseline error taxonomy fixed. From there, the individual contract documents can be authored in parallel by anyone familiar with their respective boundaries.

## 10. Open questions for the v0.1 meta-specification

The following are questions this draft deliberately leaves open. They should be resolved before the meta-specification is promoted from draft to v0.1.

1. **Evolution policy default.** Should contracts default to additive-only evolution, with breaking evolution requiring a new contract class? Or should breaking evolution be allowed within a contract class under a major-version bump, with migration machinery?

2. **Predicate language for `ContractPredicate.minitt_expression`.** Is the predicate a closed EigenTT term of type `Prop`, or a function from input/output to `Prop`? The latter is more flexible but requires deciding how the function is applied at check time.

3. **Relationship between contract version and institution implementation hash.** Should the implementation hash be part of the trace record, the registration record, or both? Both is probably right, but the semantics need to be spelled out.

4. **Scope of the error taxonomy baseline.** The six categories in Section 3.5 are a first cut. Are there failure modes not covered? Should `AuthorizationFailure` be separated from `DomainRejection`? Should `Degraded` (partial functionality) be distinguished from `Unavailable` (no functionality)?

5. **Handling of resource bounds at runtime.** Are bound violations treated as `ResourceExhaustion` errors or as a separate category? Who enforces them — the kernel, the institution, both?

6. **Contract for the meta-specification itself.** The meta-specification defines contracts; does it itself need a contract? (The author's current view: no, but the question should be answered explicitly.)

7. **Multi-component implementation manifest evolution.** How should the `ImplementationManifest` (Section 6.7) handle institutions whose component set evolves frequently? Each change to a component produces a new manifest hash; if an institution's external dependencies update weekly, are weekly re-registrations the right model, or should the manifest support partial updates within a single registration? Related: how do `Advisory` or `NonTrusted` component changes interact with re-registration requirements?

8. **Cross-institution cooperation patterns.** Should the meta-spec model patterns where multiple institutions cooperate to strengthen a single epistemic claim — cross-checking by secondary verifiers (the Lean verification institution may use Lean4Lean as a peer to nanoda_lib), parallel verification by independent institutions, voting protocols among multiple ML-based predictors, redundant computation for safety-critical domains? Currently institutions are modeled in isolation. The cooperation pattern may be common enough to deserve first-class treatment in the contract framework, or it may be sufficiently institution-specific that the meta-spec leaves it to individual contract authors.

9. **Epistemic categorization of generated artifacts.** Section 5.6 implicitly takes the position that trusted generated artifacts (mirror libraries, schemas, certificates) are of a different epistemic character than the four documented categories — they are not *declared* (no human authorship), not *observed* (the generator is in the institution's TCB rather than external reality), not *derived* (production by typed pipelines within the kernel's evaluator is not what generation is), and not *verified* (no formal proof attached). Should there be a fifth category — perhaps *generated* — to capture this position cleanly? Or is generated content best modeled as a specialization of *observed* with the generator treated as the "external source"? The decision affects how queries about provenance distinguish "this came from an external observation" from "this came from a deterministic process in our trust base."

These questions are not blockers — work can proceed in parallel with them — but they should be resolved by the time the meta-specification stabilizes, because each of them affects how individual contract specifications are written.

---

*This document is a starting point. It is deliberately more concerned with getting the shape right than with getting every detail right. The next step is turning it into a v0.1 meta-specification with the open questions answered and the ontology classes pinned, at which point individual contract specifications can be written against it in parallel.*