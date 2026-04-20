# Lean 4 as a Verification Institution in Eigenius

**Status:** Draft — outline for the design specification
**Scope:** What it takes to make Lean 4 a registered institution within Eigenius, contributing the *verified* epistemic level to the knowledge graph via the platform's existing capability protocol.
**Related:** `boundary-contracts.md` (meta-specification this outline instantiates)

## 1. Purpose and scope

Eigenius defines four epistemic categories: *declared*, *observed*, *derived*, and *verified*. The first three arise from ordinary operation of the system — human authorship, external ingestion, and typed pipeline execution with reasoning traces. The fourth requires a machine-checked proof, and thus requires an institution capable of checking proofs.

This document outlines what it takes to register **Lean 4** as that institution. It does not specify the institution in full; it scopes the work, names the design decisions that need to be made, and identifies where the hard engineering lives.

### Non-goals

- This is not a plan to embed Lean 4's elaborator, tactic framework, or Mathlib into Eigenius. Lean remains Lean; Eigenius integrates with it at the proof-term boundary.
- This is not a privileged integration. Lean 4 is one verification institution among potentially many (Rocq, Isabelle/HOL, SMT checkers, domain-specific certifiers). The protocol must accommodate them all on equal footing.
- This is not a replacement for the Mini-TT type system in the kernel. Mini-TT continues to check program composition; Lean 4 checks mathematical theorems.

## 2. Architectural position

Lean 4 enters Eigenius as a registered capability implementing the `FiberReasoner` interface. The kernel does not know it is special. The kernel knows only that a capability at a given IRI has declared itself a verification institution and is prepared to validate proof-bearing morphisms in its fiber.

### 2.1 The Option B decision

Three integration strategies were considered in prior design discussion:

- **Option A — Accept Lean's verdict.** Lean says "checks," Eigenius trusts. Simple, but places all of Lean's kernel plus the attestation signer in the trusted computing base, and undermines the "show why it is verified" story the Eigenius architecture commits to.
- **Option B — Accept Lean's proof term and re-check.** Lean exports the elaborated proof term via its existing export format. Eigenius has a Rust-native Lean term checker that re-verifies. The proof term becomes a first-class Eigon resource.
- **Option C — Translate to Mini-TT.** Lean proofs round-trip through Mini-TT. Faithful translation is a research project and fails for most of modern Lean.

**Option B is the working choice.** It preserves the "tools that reason, kernel that validates" discipline, keeps Lean stable as an external system, and allows the verification capability's trusted base to be the Lean term checker rather than all of Lean.

### 2.2 Trusted computing base

- The Eigenius kernel: unchanged. Does not grow by a single line.
- The Lean verification institution's TCB: the Rust-native Lean term checker, the `EigonFFI` correspondence library (Section 5), and the serialization layer that moves proof terms between Lean and Eigenius.
- Blast radius of a bug in the Lean checker: confined to the Lean institution's fiber. Cannot invalidate *derived* conclusions or corrupt the ontology.

## 3. Fiber structure contributed

At registration, the Lean institution declares the morphism types, query types, and structural properties it brings to the knowledge graph.

### 3.1 Morphism types

- **`ProofOf(term, proposition)`** — the core relation. Asserts that a Lean proof term proves a stated proposition under a named environment. Inhabited only after `validate_morphism` succeeds.
- **`DependencyOn(proof, axiom_or_lemma)`** — records that a proof term depends on a specific axiom or previously-verified lemma. Walks Lean's definitional transparency and extracts the non-reducible dependencies.
- **`ReducesTo(term1, term2)`** — intra-fiber morphism capturing that two terms are related by Lean's definitional equality. This is the structured-fiber payoff: a proof term is not a flat blob; it has reduction behavior that other programs may want to query.

### 3.2 Query types (subclasses of `FiberQuery`)

- **`CheckProof`** — given a proof term and a claimed proposition, does it check against the declared environment?
- **`WhichAxioms`** — given a proof term, enumerate the axioms it depends on. Useful for regulatory audit ("this conclusion rests on `Classical.choice`") and for identifying candidates for further refinement.
- **`ProofSize`** — term-level complexity metrics. Surprisingly useful: proof size correlates with maintenance burden and with the likelihood that a refactoring breaks something.
- **`EnvironmentDiff`** — given two environment references, report what changed. Critical for understanding why a previously-verified proof might no longer check after an environment upgrade.

### 3.3 Structural properties (advisory)

- `ProofOf` is functional modulo definitional equality: a given term proves exactly one proposition up to the environment's conversion rules.
- `DependencyOn` is transitive.
- `ReducesTo` is confluent within Lean's reduction system.

These are declared but not enforced by the kernel. The institution enforces them in `validate_morphism` where enforcement matters.

## 4. The FiberReasoner operations, specified

### 4.1 `fiber_declaration`

Returns the above morphism types, query types, and structural properties as ontology resources. Called once at registration; results are committed to the layer in which the institution is registered.

Also returns the institution's declared `BoundaryContract` version (see Section 9), its implementation hash, and the Lean version and library set it corresponds to.

### 4.2 `validate_morphism`

Called when a `ProofOf` resource enters the graph. The kernel has already done structural validation (required properties present, types well-formed). The institution's job:

1. Extract the proof term, the claimed proposition, and the environment hash from the morphism resource.
2. Resolve the environment hash to a concrete `LeanEnvironment` resource.
3. Check that the proposition, when stated in Lean's type theory, corresponds to the Eigenius-side claim via the `EigonFFI` correspondence (Section 5).
4. Run the Lean term checker on the proof term against the proposition and environment.
5. If valid, accept the morphism, returning success with any extracted dependency morphisms as candidate discoveries.
6. If invalid, return a typed error with diagnostic information preserved.

### 4.3 `query`

Dispatched when the evaluator recognizes a `FiberQuery` subclass the Lean institution declared. The major queries are:

- **`CheckProof`** reduces to the same path as `validate_morphism` but without committing. Used for "what would happen if this proof were submitted?" queries.
- **`WhichAxioms`** walks the term, extracts the reference set, filters to non-reducible axioms and declared constants.
- **`ProofSize`** computes term-level metrics.
- **`EnvironmentDiff`** is non-trivial — it requires comparing two environment snapshots and producing a structured diff. First implementations can return a coarse "different" / "same" answer; structural diff comes later.

Query results are typed resources in the Lean institution's fiber. They pass through the ordinary trace-cache machinery — a `CheckProof` on the same term and environment hits the cache.

### 4.4 `discover_morphisms`

Given a set of resources in the institution's fiber, infer morphisms not yet committed. For Lean: given a proof term, extract its `DependencyOn` relationships and its `ReducesTo` relationships up to a configured depth. Returns candidate resources; the calling program decides whether to commit them.

The institution has no write access to the graph. It returns; the program decides.

## 5. The type correspondence problem

A proof term in Lean proves a proposition stated in Lean's type theory. A knowledge-graph claim in Eigenius is stated in Eigon. These are two different languages, and something has to establish the correspondence. This is the hardest piece of the design, and the mechanism by which it is made sound is the reason the *verified* epistemic category is genuinely stronger than the *derived* one, rather than a label that merely sounds stronger.

The correspondence is established by a Lean library called `EigonFFI`, which mirrors the ontology's structure as Lean types. Users author proofs against the mirror; the verification institution checks both that the proof is valid Lean and that the mirror faithfully represents the Eigenius-side claim.

### 5.1 `EigonFFI` as a generated, tracked static mirror

`EigonFFI` is not a runtime API. It does not query the knowledge graph during proof checking. It is a *generated static library* that mirrors ontology structure as Lean code, produced at a specific ontology state and — critically — committed back to Eigenius as a tracked resource.

For each Eigon class a user might prove things about, a generated `EigonFFI` library provides a Lean structure with the same required fields, subclass relationships expressed as coercion instances, and ontology invariants encoded as theorems or axioms. A proof about the safety factor of a `StressResult` resource is, in Lean, a proof about an `EigonFFI.StressResult` value with the corresponding structural shape.

The generated library itself becomes an Eigon resource of class `GeneratedLibrary` (name placeholder — see Section 5.3), committed to the knowledge graph with its own content hash, its own declared provenance (source layer hash, generator version, generator content hash), and optionally the full Lean source text embedded as content. This is the move that makes verification auditable end-to-end: the library is not merely an external artifact referenced by a hash in proof submissions, but a tracked resource whose existence and provenance the knowledge graph itself records.

The runtime-API alternative was considered and rejected. A library that queried the knowledge graph during proof checking would make proof validity depend on live state, destroying reproducibility. It would also expand the trust surface to every resource the proof touched during checking, rather than containing it in a small, versioned, tracked artifact. The static-mirror design keeps the trust surface small and makes proofs into closed mathematical objects that can be archived.

A diagnostic test for future design decisions: *could this cause the same proof to check differently at different times?* If yes, the functionality does not belong in `EigonFFI`. If it is valuable, it belongs elsewhere — in EigenQL, in a component, in proof-authoring tooling. `EigonFFI` is the place where things are stable by construction, and that stability is its whole value.

### 5.2 The anchor chain

An `EigonFFI` library is bound to a specific ontology state by three pieces of metadata that together form its *anchor*:

- **Layer hash** — the content-addressed SHA-256 of the CBOR-encoded layer stack at the moment the library was generated. This identifies the complete ontology snapshot the mirror was produced from. Because layers are immutable and content-addressed, this hash is a stable and total identifier for the ontology state.
- **Generator version** — the identity of the tool (working name `eigon-ffi-gen`) that produced the library. Different generator versions may produce different mirrors from the same ontology state.
- **`EigonFFI` content hash** — a hash of the generated library itself.

If the generator is deterministic — a design requirement, not an assumption — then the `EigonFFI` content hash is a function of `(layer_hash, generator_version)`. The content hash is kept in the anchor as belt-and-suspenders: it guards against non-determinism in the generator or in subsequent Lean compilation, and makes verification verdicts a pure function of hashes without requiring the generator to be re-invoked during checking.

Every `EigonFFI` library carries its anchor declaratively, for example as a header comment:

```
-- Generated from layer urn:eigenius:layer:sha256:abc123...
-- Using eigon-ffi-gen version 1.4.2 (content hash sha256:def456...)
-- Do not edit; regenerate with: eigon-ffi-gen --layer <hash>
```

Every proof submission pins the `EigonFFI` it was authored against, by content hash. That pinning is what makes the correspondence check possible and reproducible.

### 5.3 The generated library as a tracked resource

When `eigon-ffi-gen` runs against a layer, its output is both a Lean source file on disk (for the user to import into their Lean project) and a corresponding `GeneratedLibrary` resource committed to Eigenius. The resource carries at minimum:

- **`source_layer`** — IRI of the layer the library was generated from.
- **`generator_identifier`** — identifier of the generator tool.
- **`generator_version`** — version string.
- **`generator_content_hash`** — content hash of the generator binary, pinning the exact tool that produced the library.
- **`library_content_hash`** — content hash of the generated Lean source.
- **`library_content`** — optional embedded source (for small libraries) or a content-addressed reference to external storage (for large ones).
- **`generated_at`** — timestamp, advisory.
- **`mirrored_classes`** — the set of Eigon class IRIs the library chose to mirror, supporting scoped generations where not every class in the ontology is included.

The library's IRI in the knowledge graph is content-addressed from its properties, so two independent generations from the same layer with the same generator produce the same IRI. Re-committing an identical library is a no-op.

This is what makes **independent provenance verification** possible. An auditor with access to the generator binary and the layer chain can re-run `eigon-ffi-gen --layer L` in their own environment, compute the content hash of the result, and compare it to the committed library's declared hash. If the hashes match, the library is authentic and its declared anchor is truthful. If they diverge, something is wrong — non-determinism in the generator, tampering with the committed artifact, or an environment difference that affects layer resolution. In all cases, the audit is a local computation whose result the auditor can verify without trusting anyone.

Committing `EigonFFI` libraries as resources also enables **canonical publication**. A team can designate a specific `GeneratedLibrary` resource as the canonical mirror for a given domain ontology, and downstream users can discover and reuse it rather than regenerating independently. Proofs authored by different teams against the same canonical library interoperate without coordination about generator versions or content-hash alignment.

The Lean source content itself may be embedded directly in the resource or stored externally with the resource holding a content-addressed reference. For small libraries embedded content is simplest and keeps archives self-contained. For large libraries — ontologies producing multi-megabyte mirrors — external storage with a reference is the right pattern. Both modes are supported; the choice is a deployment policy question, not an architectural one.

### 5.4 The generator and its requirements

Because proof soundness depends on the correspondence between `EigonFFI` mirror types and Eigon classes, and because `EigonFFI` is mechanically generated, the generator is part of the verification institution's trusted computing base alongside the Lean term checker.

This imposes two requirements on the generator:

- **Determinism.** Given the same layer hash, the generator must produce byte-identical output. Two users regenerating `EigonFFI` from the same ontology state must obtain identical libraries; otherwise their proofs will fail to interoperate despite being about the same underlying data. Determinism is also what makes independent provenance verification (Section 5.3) work — without it, an auditor cannot reproduce the library hash.
- **Faithful translation.** The generated Lean types must structurally correspond to the Eigon classes they mirror. Required properties become required fields; primitive types map by a declared correspondence; subclass relationships become coercion instances; format and constraint declarations become refinement conditions where Lean-expressible.

The faithful-translation specification is itself a piece of design work warranting its own document. For the purposes of this outline: the generator is not an afterthought tool; it is trusted infrastructure, and its specification deserves the same rigor as the verification institution it feeds.

The generator binary should itself be content-hashed, and that hash committed as part of each `GeneratedLibrary` resource's provenance. This closes the chain: the library's authenticity can be verified against the generator, and the generator's identity is part of the trusted record.

### 5.5 The correspondence check

When a proof submission enters Eigenius, `validate_morphism` performs three checks, in order. The first is ordinary Lean verification; the second and third are what make the correspondence sound.

1. **Proof validity.** The Lean term checker verifies that the proof type-checks against its stated proposition under its declared environment.
2. **Mirror correspondence.** The verification institution resolves the `EigonFFI` library IRI in the submission to a committed `GeneratedLibrary` resource, reads its declared anchor, and checks that the library was anchored to a layer ancestral to or identical with the layer in which the claim's class is defined. The specific mirror type used in the proposition must correspond structurally to that class.
3. **Anchor consistency.** The `GeneratedLibrary`'s declared content hash is verified against its actual content, and the layer hash it declares is confirmed to resolve in the current layer chain. Optionally, the generator identified in the resource's provenance can be re-invoked to confirm that it reproduces the committed library — this is the independent provenance verification described in Section 5.3, and can be performed at any time after submission, not only at check time.

Check 2 is the load-bearing one for soundness. It is what prevents a proof about an `EigonFFI.StressResult` (anchored to an old ontology state) from being accepted as verification for a resource whose class has since acquired new required properties.

### 5.6 Compositionality under layer extension

An important property of the anchor design is that an `EigonFFI` library does *not* need to be regenerated every time the ontology evolves. A library anchored to layer L₀ remains valid for claims in any descendant layer L₁ ⊒ L₀, provided the classes the library mirrors are unchanged in L₁.

This is the common case. A user generates `EigonFFI` from a layer containing Core plus FEA. They later commit their bracket analysis in a descendant layer; that layer adds domain data (specific stress results, meshes, materials) but does not modify the FEA class definitions. Their proof, authored against the L₀ mirror of `StressResult`, remains valid for verifying claims about stress results in the later layer. The correspondence check confirms that `StressResult`'s class definition is byte-identical in the resolved layer chain and accepts the proof.

The failure mode is symmetric. If a later layer L₂ modifies `StressResult` — adds a new required property, for instance — then proofs anchored to L₀ fail the correspondence check when submitted against L₂ claims. The rejection surfaces as `FFIVersionMismatch`, with diagnostic information naming which mirror type no longer matches which class. The user regenerates `EigonFFI` from L₂ (or a descendant), re-states their proof against the updated mirror, and resubmits.

This is the desirable behavior. It is the anchoring doing its job: catching exactly the cases where proof soundness would otherwise silently degrade. Users regenerate `EigonFFI` when the ontology changes in ways that matter to their proofs, and not otherwise.

### 5.7 The closed chain from ontology to archived verdict

Putting the pieces together, the full chain of anchored and tracked artifacts in a verification is:

1. Ontology content is committed to Eigenius, producing layer hash L.
2. `eigon-ffi-gen` is run against L. Its output is both a local Lean source file and a committed `GeneratedLibrary` resource in Eigenius, carrying the library's content hash, the source layer reference, and the generator's own content hash.
3. The Lean source is imported into a Lean project; a proof is authored against its mirror types.
4. The proof elaborates against a specific Lean environment E (a pinned, content-addressed resource).
5. The proof term is exported; its content hash is T.
6. A `LeanProofSubmission` resource is committed to Eigenius carrying (T, E, `GeneratedLibrary` IRI, claim_IRI, claim_layer_hash).
7. The verification institution runs the three-part correspondence check.
8. On success, a `ProofOf(T, claim)` morphism is committed with full provenance.

At every step, every artifact is content-addressed and tracked as a resource within Eigenius. The verification verdict is a pure function of archived inputs, and every input lives in the knowledge graph itself rather than in external storage. Given the graph, the verdict reproduces at any future point and yields the same result. Independent auditors can verify each step locally: re-check the proof against the environment, re-run the generator against the source layer to reproduce the library hash, re-evaluate the correspondence check against the committed classes.

This is the reproducibility property that distinguishes verification from the other epistemic categories: *derived* resources depend on traces that can be re-executed but produce new traces each time; *verified* resources depend on anchored, tracked artifacts that reproduce identically and forever. The move from "tracked by external hash" to "tracked as committed resource" is what makes the archive self-contained.

### 5.8 TCB implications

The Lean verification institution's trusted computing base has three components:

- The **Lean term checker** — Rust-native, shared across deployments.
- The **`EigonFFI` generator** — deterministic, with a separately specified faithful-translation contract.
- The **correspondence logic** in `validate_morphism` — the code that matches mirror types to Eigon classes and verifies anchor consistency.

The kernel's TCB does not grow. The blast radius of a bug in any of the three is confined to the verification institution's fiber: bad proofs may be accepted, but *derived* conclusions and the ontology itself remain unaffected.

This TCB is larger than the original single-component version (just the Lean checker) but much smaller than the full-translation alternative (Option C), and each component is independently auditable. The generator's faithful-translation specification in particular is a finite piece of design work that can be reviewed once and then relied upon across all subsequent proofs.

## 6. Proof term transport

### 6.1 Lean 4 export format

Lean 4 can export elaborated terms in a well-specified format produced by the `lean4export` tool (see Appendix A.1). The format is designed to be consumed by external checkers; nanoda_lib (the chosen dependency, Section 8.1) parses it directly. This is the format Eigenius consumes at the institution boundary.

### 6.2 Resource encoding

A proof submission to Eigenius is a resource of class `LeanProofSubmission` carrying the full anchor chain described in Section 5.7:

- `proof_term` — the exported term, CBOR-encoded for Eigenius-internal transport.
- `claimed_proposition` — the Lean proposition the term claims to prove, stated in terms of `EigonFFI` mirror types.
- `environment_reference` — content-addressed IRI of the `LeanEnvironment` resource the term was elaborated against.
- `eigonffi_library_reference` — IRI of the `GeneratedLibrary` resource used, which resolves to the committed library with its full provenance (source layer, generator version and hash, library content).
- `eigenius_claim_reference` — IRI of the Eigon-side resource this proof vouches for.
- `claim_layer_hash` — layer hash at which the claim exists; determines which ontology state the correspondence check resolves against.

All references resolve within the knowledge graph itself. A committed submission is a closed object: given its properties, every artifact the verdict depends on is retrievable from Eigenius alone, without external dependencies.

### 6.3 Content addressing

The proof term's content hash, combined with the environment hash and the `EigonFFI` hash, forms the cache key for verification results. A previously-checked proof does not re-check under the same environment and FFI; its result is served from the trace cache.

## 7. Environment management

Mathlib is large. A naive "load the environment per request" design is non-viable.

### 7.1 Environment as a resource

A `LeanEnvironment` is a typed Eigon resource with content-addressed identity. It is immutable. Its properties include the Lean version, the set of libraries loaded, their versions, and — critically — its content hash.

### 7.2 Pinning and sharing

Institution registrations pin the environment they expect. Multiple registrations can share an environment by referencing the same content-addressed IRI. A change to the environment produces a new resource at a new IRI; prior proofs remain valid against the prior environment.

### 7.3 Caching strategy

The verification institution maintains an environment cache indexed by content hash. First request for a given environment loads and caches it; subsequent requests are served from cache. Eviction is policy-driven.

### 7.4 Environment diffing

Two `LeanEnvironment` resources can differ in library versions, axiom sets, or definitional transparencies. Structured diffing (the `EnvironmentDiff` query of Section 4.3) is non-trivial to implement well, but even a coarse version — "these environments are different, proofs checked against one are not transferable to the other" — is valuable as a safety rail.

## 8. Implementation approach

The original version of this outline listed three deployment shapes as options under evaluation. With the survey of existing external checkers summarized in Appendix A, the decision has effectively been made: Eigenius will use an **in-process Rust checker** (the shape described in Section 8.1), with **nanoda_lib** as the concrete dependency. The other shapes are retained in Sections 8.2 and 8.3 as context for why this one was chosen and where it might evolve.

### 8.1 In-process Rust checker (chosen approach)

The Lean term checker runs as a Rust library linked into the `eigenius-lean` crate. Proof terms produced by `lean4export` are parsed and checked without leaving the Eigenius orchestrator process. The specific library is **nanoda_lib** ([github.com/ammkrn/nanoda_lib](https://github.com/ammkrn/nanoda_lib)), maintained by Chris Bailey, accompanied by the specification book [Type Checking in Lean 4](https://ammkrn.github.io/type_checking_in_lean4/).

This approach is preferred because:

- **Latency and integration.** No cross-process IPC, no serialization of proof terms over a service boundary, no Lean toolchain at runtime. The verification institution is a function call away from the evaluator.
- **Trust surface is auditable Rust.** The checker's code is available, readable, and sized for independent review. No Lean source-level machinery (elaborator, tactic framework, metaprogramming) is in the verification TCB — only the kernel-level checking logic.
- **Pre-existing axiom governance.** nanoda_lib already implements axiom allow-listing via its configuration. This provides the mechanism for the policy decisions raised in Section 12, question 2 — Eigenius still needs to choose *which* axioms to permit, but no implementation work is required to enforce the choice. The permitted-axioms list is declarative and inspectable.
- **Library-shaped.** nanoda_lib is designed to be embedded as a crate dependency, not only invoked as a binary. This matches the Eigenius requirement that the checker be dispatchable through the existing `FiberReasoner` protocol.

The cost is an ongoing maintenance relationship: nanoda_lib must track upstream Lean kernel changes, and Eigenius must track nanoda_lib. This is not a one-time integration. See Section 11 Phase C and Appendix A.3 for the maintenance considerations.

**Cross-checking as a soundness multiplier.** An additional use of the Lean4Lean checker (Appendix A.2) as a *secondary* verifier is recommended: if nanoda_lib and Lean4Lean disagree on a verdict, the submission is flagged for review rather than accepted. This is the Venn-diagram argument from [Type Checking in Lean 4](https://ammkrn.github.io/type_checking_in_lean4/trust/trust.html): the part where independent checker circles intersect is the stronger soundness claim. This is not part of Phase A, but is a natural enhancement during Phase C hardening.

### 8.2 Out-of-process Lean service (rejected for the primary path)

A Lean 4 process running as a service, dispatched to via gRPC. Lean checks the term in its own kernel; Eigenius receives the verdict.

This is Option A from Section 2.1 in disguise. The trust surface becomes all of Lean plus the service boundary, which defeats the Option B rationale. It is retained here only to note that it remains acceptable as a *bootstrap* during early Phase A experimentation, before the nanoda_lib integration is live — specifically for validating that the protocol end-to-end works before committing to the Rust dependency. It is not the long-term design.

### 8.3 WASM-sandboxed checker (future option)

The Rust checker compiled to WASM and run under the platform's existing untrusted-capability sandbox. Uniform with how other untrusted capabilities run; fuel-bounded; memory-isolated; dispatches through the same path as other WASM institutions.

The performance overhead on large Mathlib-scale proofs is likely meaningful, and the engineering to make nanoda_lib run cleanly under WASM is non-trivial. This remains a future option worth evaluating once the WASM capability infrastructure is mature and once realistic benchmarks exist, but is out of scope for Phases A through D.

## 9. Instantiating the BoundaryContract

The Lean 4 verification institution instantiates `VerificationInstitutionContract`, which in turn specializes `InstitutionContract` from the boundary contracts meta-specification.

### 9.1 Error taxonomy additions

Beyond the baseline `ErrorEnum` (Section 3.5 of the contracts meta-spec), the Lean institution distinguishes:

- `ProofDoesNotCheck` — Lean term checker rejected the proof. Diagnostic payload includes the Lean error message.
- `PropositionMismatch` — proof checks but its proposition does not correspond to the Eigenius-side claim. Diagnostic includes the expected and actual propositions.
- `EnvironmentUnavailable` — the referenced environment is not loaded and could not be loaded.
- `EnvironmentMismatch` — the proof was elaborated against a different environment than declared.
- `FFIVersionMismatch` — the `EigonFFI` version does not match the current Eigenius ontology.

### 9.2 Declared properties

- **Determinism:** `DeterministicModuloLayer`. Given the same proof term, proposition, environment, and `EigonFFI` version, the checker's verdict does not vary.
- **Idempotence:** `Idempotent`. Re-checking a proof yields the same result.
- **Effects:** `Read` (consults environment resources); produces traces (verification results cached).
- **Resource bounds:** per-proof `max_wall_time_ms` and `max_memory_bytes`. Mathlib-scale proofs can be large; defaults need to be generous but bounded.

### 9.3 Lifecycle

The institution is registered against a specific `VerificationInstitutionContract` version. Upgrades to the Lean version, the `EigonFFI` library, or the contract itself produce new registrations in later layers; prior registrations remain valid for their trace history.

## 10. Integration touchpoints in the existing code

### 10.1 Kernel-side changes

- None to the Mini-TT type theory, its evaluator, or its type checker.
- Minor: the `InstitutionRegistry` must accept the verification institution's registration and expose it for dispatch. This already works in principle; concrete testing is needed.

### 10.2 Orchestrator-side additions

- A new Rust crate (working name `eigenius-lean`) housing the Lean term checker, the `EigonFFI` correspondence logic, and the environment cache.
- Integration with the existing `FiberReasoner` trait and `InstitutionRegistry`.
- gRPC surface for proof submission and query dispatch (reuses existing institution dispatch plumbing).

### 10.3 Ontology additions

- Classes: `LeanProofSubmission`, `LeanEnvironment`, `LeanProofTerm`, `GeneratedLibrary`, `ProofOf`, `DependencyOn`, `ReducesTo`.
- Query classes: `CheckProof`, `WhichAxioms`, `ProofSize`, `EnvironmentDiff`.
- Error classes corresponding to the error enum additions.

### 10.4 `EigonFFI` and its generator

- A new Rust crate (working name `eigon-ffi-gen`) implementing the generator. Deterministic by construction. Takes a layer hash as input, produces both a Lean source file for local consumption and a committed `GeneratedLibrary` resource in Eigenius. Part of the verification institution's trusted computing base.
- The generator binary itself is content-hashed, and that hash is recorded in every `GeneratedLibrary` it produces, enabling independent provenance verification.
- A faithful-translation specification document, authored alongside the generator, that pins down how Eigon constructs map to Lean constructs.
- Initial scope: mirror types for the Core Ontology and for the first domain ontology against which proofs will be written.

## 11. Phased implementation plan

Scope is given as t-shirt sizes (Small / Medium / Large / Open-ended) rather than as time estimates, since duration depends on team size, parallelization, and deployment priorities. The phases are ordered by dependency — each builds on what came before — and the named scope reflects the relative size of the engineering commitment, not a calendar prediction.

### Phase A — Proof of concept

Integration of nanoda_lib into the `eigenius-lean` crate. Minimal `FiberReasoner` implementation dispatching to nanoda_lib for proof term checking. Toy propositions only — propositions stated directly about primitive types, no `EigonFFI` yet. Demonstrates end-to-end: submission, dispatch, checker invocation, result committed to the knowledge graph as a `ProofOf` morphism.

Optional fallback: if nanoda_lib integration hits unexpected obstacles, a brief out-of-process Lean service (Section 8.2) can serve as a bootstrap to validate the protocol end-to-end before returning to the primary integration. This is a contingency rather than a planned step.

**Scope:** Small. Validates the protocol and establishes the nanoda_lib dependency relationship.

### Phase B — `EigonFFI`, the generator, and real propositions

First version of the `eigon-ffi-gen` generator: deterministic implementation, faithful-translation specification authored in parallel, `GeneratedLibrary` resource committed back to Eigenius as part of each generation. First generated `EigonFFI` library mirroring Core Ontology types. Anchor representation in `LeanProofSubmission` resources, referencing the committed `GeneratedLibrary` by IRI. The three-part correspondence check in `validate_morphism`, including the layer-ancestry logic that gives compositionality under layer extension.

**Scope:** Medium. This is where the trust-surface work happens, the faithful-translation specification gets authored, and the load-bearing design decisions about mirror structure and library tracking get crystallized.

### Phase C — Integration hardening and checker operational maturity

Performance work: profiling against realistic proof sizes, identifying hotspots in the dispatch path, tuning trace-cache policy. Environment caching infrastructure for `LeanEnvironment` resources. Upstream tracking protocol with nanoda_lib: establishing how Eigenius follows Lean kernel changes propagated through nanoda_lib, and what the version-pinning discipline looks like. Optional follow-on (recommended in Section 8.1): introduction of Lean4Lean as a secondary cross-checker per the Venn-diagram soundness argument.

**Scope:** Large. Where the original outline framed this as "port a Rust checker," the availability of nanoda_lib replaces the port with integration and operational work — different in character but comparable in scale.

### Phase D — Mathlib-dependent proofs

Extension of `EigonFFI` and environment management to support proofs that depend on Mathlib. Environment diff tooling. Resource-bound enforcement at production scale.

**Scope:** Large. Much of it integration and performance work rather than new design.

### Phase E — Production hardening

WASM sandboxing (Option 8.3) if warranted. Full error-diagnostic preservation. Audit tooling. Regulatory-facing query surfaces.

**Scope:** Open-ended, driven by deployment needs.

## 12. Open questions

1. **Correspondence granularity.** Does `EigonFFI` mirror every class that might appear in a proof, or only those used in the initial domain? The first is comprehensive but expensive; the second is incremental but requires a policy for when to extend.

2. **Axiom allow-list policy.** Eigenius will use nanoda_lib's allow-listing mechanism (Appendix A.3), but still needs to decide *which* axioms to permit (the standard set is `propext`, `Classical.choice`, `Quot.sound`, plus `Lean.trustCompiler` for compiled-primitive references) and *where* the allow-list is specified — in the contract, the registration, or per-query. Some deployments will want to reject `Classical.choice`-dependent proofs entirely.

3. **Environment upgrade policy.** When a new Lean version or Mathlib version is released, what is the process for promoting it? Automatic, subject to regression testing, or manual with explicit review?

4. **Proof reproducibility across Lean versions.** If a proof checks under Lean 4.N but not 4.N+1, what is the policy? Pin each proof to its Lean version (loses future verification), or re-check on upgrade (loses proofs that break)?

5. **Parallel verification institutions.** If Rocq and Lean both register and a proof is submitted that could be checked by either, which is dispatched? Is there a user-level preference, a contract-level preference, or is it by explicit IRI only?

6. **Granularity of `DependencyOn` extraction.** Should the institution extract all transitive dependencies automatically, only direct dependencies, or up to a configurable depth? Full extraction scales badly on large Mathlib proofs.

7. **Generator governance and the faithful-translation specification.** Who maintains `eigon-ffi-gen`, who owns the faithful-translation specification, how are generator versions promoted to canonical status? Since the generator is in the verification institution's TCB, its governance matters more than ordinary library governance. The faithful-translation specification is a long-lived artifact and deserves an explicit maintenance model.

8. **Scope of the faithful-translation specification.** Which Eigon constructs translate to which Lean constructs? Required properties become required fields, but how are recommended properties handled — as `Option` types, or omitted entirely? How are format constraints (regex patterns, date formats) rendered in Lean — as refinement predicates with decision procedures, as axioms, as runtime-only checks outside the mirror? How is the three-layer Eigon type system (primitive / format / content type) flattened into Lean's single-layer type theory? Each of these is a design decision the specification needs to make explicit.

9. **Kernel extension to express verification status in the type system.** Should `EigonClass(iri)` be extended to carry a *witness* — a parameter referencing the verification environment under which the resource's class invariants have been proved? Today, a `StressResult` *derived* by an FEA pipeline and a `StressResult` *verified* by a Lean proof are indistinguishable to the Mini-TT type checker: they share the same class IRI and property structure, and the epistemic distinction lives only in the provenance metadata. With the extension, a pipeline could declare its input as `EigonClass(StressResult, verified_in: E)` rather than just `EigonClass(StressResult)`, and the type checker would reject compositions that feed unverified inputs into positions requiring verification. This lifts the epistemic check from runtime introspection of the trace tree to compile-time validation. Verified types would coerce to their unverified counterparts — every verified `StressResult` is a `StressResult` — but not vice versa, mirroring the subclass-coercion pattern the architecture already uses elsewhere. The cost is a non-trivial extension to Mini-TT (a new term form, NbE equality rules, coercion machinery) and crossing the architectural boundary that currently keeps epistemic status out of types and in provenance. Probably not for Phase B (no consumers yet); worth deciding before Phase C, when the first pipelines that benefit from typed verification status start to exist. The decision is much easier to make when a concrete consumer is asking for it than as an abstract design question.

---

*This outline is a starting point. The next step is turning Sections 3 through 9 into a concrete design specification (the `VerificationInstitutionContract v1` document) with the open questions resolved, at which point Phase A can begin.*

---

## Appendix A: Survey of external Lean 4 checkers

This appendix documents the landscape of external Lean 4 type checkers as of the time of this writing, captured to support the implementation decision in Section 8.1 and to record why that decision was made rather than leaving the reader to reconstruct it.

External checkers operate on the **Lean 4 export format** rather than on Lean source code. They consume a plain-text representation of fully elaborated kernel terms, produced by a separate exporter, and verify that those terms type-check according to Lean's kernel rules. They are the mechanism by which Lean proofs can be independently re-checked outside the Lean toolchain itself — the basis of the de Bruijn criterion applied to Lean's ecosystem.

### A.1 lean4export — the exporter

**Repository:** [github.com/leanprover/lean4export](https://github.com/leanprover/lean4export)

Maintained under the `leanprover` organization (official). Produces the plain-text NDJSON-based declaration export format that all external checkers consume. This is the tool the Lean side of the Eigenius integration invokes to produce the proof term representation that crosses the institution boundary.

The export format is documented both in the repository README and, for historical Lean 3 context that still applies broadly, at [leanprover-community/lean/doc/export_format.md](https://github.com/leanprover-community/lean/blob/master/doc/export_format.md). Chris Bailey's book [Type Checking in Lean 4](https://ammkrn.github.io/type_checking_in_lean4/) is the more comprehensive specification, and is recommended reading for anyone implementing against the format.

### A.2 Lean4Lean — the verified checker

**Repository:** [github.com/digama0/lean4lean](https://github.com/digama0/lean4lean)
**Paper:** *Lean4Lean: Verifying a Typechecker for Lean, in Lean*, Mario Carneiro, [arXiv:2403.14064](https://arxiv.org/abs/2403.14064)

The most actively developed external checker for Lean 4. Written in Lean itself, which unusually allows the checker's own correctness to be partially formally verified against an abstract Lean metatheory specification. The paper documents this effort along with implementation architecture.

Performance runs 20–50% slower than the C++ kernel. Checks all of Mathlib successfully. Implements complete Lean 4 kernel semantics including nested inductive reduction, η for structures, and the native Nat/String extensions.

**Caveat on independence.** Because Lean4Lean is derived directly from the C++ kernel implementation, it isn't a truly independent implementation — any bug shared between the C++ kernel and Lean4Lean would go undetected. The author is explicit about this; the value proposition is that of a *consistent* external checker with proofs about its own behavior, rather than an *independent* one.

**Role in Eigenius.** Recommended as a secondary cross-checker during Phase C or later (see Section 8.1). If nanoda_lib and Lean4Lean both accept a proof, the verification is stronger than either alone; disagreements warrant investigation. Not needed for Phase A or Phase B.

### A.3 nanoda_lib — the Rust checker (chosen dependency)

**Repository:** [github.com/ammkrn/nanoda_lib](https://github.com/ammkrn/nanoda_lib)
**Documentation:** [Type Checking in Lean 4](https://ammkrn.github.io/type_checking_in_lean4/) (Chris Bailey)

A Rust library implementing type inference and checking for Lean 4 kernel terms. Consumable as a Cargo crate dependency, with an optional binary frontend. Clean-room-style Rust implementation rather than a port of the C++ kernel, consuming lean4export output as its input format.

Implements the kernel semantics necessary for practical proof checking, including:

- Mutual and nested inductive types (with the caveat that nested inductive *reduction* historically lagged — worth verifying current status during Phase A integration).
- Eta for structures and primitive projections.
- Optional Nat and String kernel extensions, configurable.
- Axiom allow-listing with per-axiom permission flags. The conventional allow-list is `propext`, `Classical.choice`, `Quot.sound`, plus `Lean.trustCompiler` for exports that reference compiled primitives.

The companion book *Type Checking in Lean 4* is the most comprehensive specification of what an external checker needs to do and why; it serves both as documentation for nanoda_lib and as a standalone reference for the trust model and kernel semantics.

**Role in Eigenius.** The primary dependency for the `eigenius-lean` crate. Wrapped by a thin `FiberReasoner` implementation that translates between the institution protocol and nanoda_lib's API. The axiom allow-listing mechanism addresses Section 12 open question on axiom policy directly.

**Maintenance posture.** nanoda_lib is primarily maintained by a single contributor. Eigenius takes on a relationship with the library that includes tracking upstream Lean kernel changes as they are propagated through nanoda_lib, and contributing back where integration surfaces bugs or gaps. This is a consideration for Phase C rather than an immediate concern.

### A.4 trepplein and successors

**Original (Lean 3):** [github.com/gebner/trepplein](https://github.com/gebner/trepplein) — Gabriel Ebner's Scala checker for Lean 3. Still exists but is a Lean 3 tool.

**Lean 4 successor attempt:** [siddhartha-gadgil.github.io/trepplein4](https://siddhartha-gadgil.github.io/trepplein4/) — a Lean 4 Scala successor by Siddhartha Gadgil, based on Gabriel's original. Less mature and less actively maintained than either Lean4Lean or nanoda_lib. Retained here for historical completeness: trepplein was the name cited in early Eigenius design discussions as a candidate reference implementation, and the lineage of external-checker thought traces through it.

**Role in Eigenius.** None directly. A Scala checker in the stack would impose a JVM dependency on the orchestrator that the Rust path avoids.

### A.5 What this survey settles

The availability of nanoda_lib as a library-shaped Rust dependency with an accompanying specification book, combined with Lean4Lean as a secondary cross-checker option, removes the "build a checker" work from the Eigenius roadmap. The decision in Section 8 reflects this: the implementation approach is not an open question but a dependency relationship. The remaining work is integration, axiom policy configuration, environment management, performance tuning, and maintenance coordination with the upstream library — all of which is tractable engineering, none of which is research.

This appendix should be revisited annually, or when any of the surveyed projects undergoes a major release, to verify that the underlying landscape assumptions still hold.