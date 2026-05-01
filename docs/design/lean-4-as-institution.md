# Lean 4 as a Verification Institution in Eigenius

**Status:** Draft — outline for the design specification
**Scope:** What it takes to make Lean 4 a registered institution within Eigenius, contributing the *verified* epistemic level to the knowledge graph by realising the [D14](d14-institution-realisation.md) institution protocol.
**Related:** [`d14-institution-realisation.md`](d14-institution-realisation.md) (the institution protocol this doc instantiates — D14 specifies the trait surface, the five typed resource shapes, and the dispatch model; this doc fills in the Lean-specific surface), [`runtime-substrate.md`](runtime-substrate.md) (substrate the Lean *authoring*-side toolchain runs on; the verification side stays in-process — see §2.3), `boundary-contracts.md` (meta-spec context — note that under D14 the per-institution `BoundaryContract` collapses into the typed declarations of §4 plus the Verdict shape; see §9 for the migration)

## 1. Purpose and scope

Eigenius defines four epistemic categories: *declared*, *observed*, *derived*, and *verified*. The first three arise from ordinary operation of the system — human authorship, external ingestion, and typed pipeline execution with reasoning traces. The fourth requires a machine-checked proof, and thus requires an institution capable of checking proofs.

This document outlines what it takes to register **Lean 4** as that institution under the [D14 institution realisation protocol](d14-institution-realisation.md). It does not specify the institution in full; it scopes the work, names the design decisions that need to be made, and identifies where the hard engineering lives. The protocol-level shape (trait surface, resource declarations, dispatch model, Verdict, comorphism triadic shape) is fixed by D14; this doc fills in Lean-specific content for that shape.

### 1.0 D14 in one paragraph (so the rest of this doc is readable in isolation)

Under D14, an institution is registered by committing five kinds of typed Resources to the layer chain: an `Institution` (identity and runtime kind), `ExportFormat`s (typed extractions of class instances into Mini-TT payloads), `ImportFormat`s (typed constructors of class instances from Mini-TT payloads), `QueryClass`es (typed functions in the institution's fibre with a `dispatch_role` of `OnDemand` / `AutoOnLoad` / `Decidable` and a result class — `Verdict` for the gate-on-commit and decide-procedure roles), and `Comorphism`s (triples `(s, m, t)` where `s` is an ExportFormat, `m` is a Mini-TT Component, and `t` is an ImportFormat). The institution implements an `Institution` Rust trait with three methods: `extract_typed` (boundary out), `reify` (boundary in), and an optional `query` (for QueryClasses whose implementation is institution-runtime rather than a Mini-TT Component). The kernel maintains a derived registry from chain scans, runs `AutoOnLoad` QueryClasses on commit, dispatches `Decidable` QueryClasses from `Exp::NativeDecide`, dispatches `OnDemand` QueryClasses from EigenQL FIBER, and runs Comorphisms via `Exp::InstitutionInvoke`. There is no `FiberReasoner` trait, no procedural `fiber_declaration()`, and no `BoundaryContract` per institution — those collapsed into typed declarations + the Verdict shape. The remainder of this document fills in the Lean-specific instantiation.

### Non-goals

- This is not a plan to embed Lean 4's elaborator, tactic framework, or Mathlib into Eigenius. Lean remains Lean; Eigenius integrates with it at the proof-term boundary.
- This is not a privileged integration. Lean 4 is one verification institution among potentially many (Rocq, Isabelle/HOL, SMT checkers, domain-specific certifiers). The protocol must accommodate them all on equal footing.
- This is not a replacement for the Mini-TT type system in the kernel. Mini-TT continues to check program composition; Lean 4 checks mathematical theorems.

## 2. Architectural position

Lean 4 enters Eigenius as a registered institution under D14 — committing an `Institution` resource, the supporting ExportFormat / ImportFormat / QueryClass declarations, and an `Institution` trait implementation. The kernel does not know Lean is special. It knows only that an institution at a given IRI has declared itself, including a `ProofCheck` QueryClass with `dispatch_role: AutoOnLoad` returning `Verdict`. When a `LeanProofTerm` resource enters the chain, that QueryClass fires automatically; if the verdict is `Holds`, the resource is admitted (and tagged *verified*); if `Fails`, the Load is rejected.

### 2.1 The Option B decision

Three integration strategies were considered in prior design discussion:

- **Option A — Accept Lean's verdict.** Lean says "checks," Eigenius trusts. Simple, but places all of Lean's kernel plus the attestation signer in the trusted computing base, and undermines the "show why it is verified" story the Eigenius architecture commits to.
- **Option B — Accept Lean's proof term and re-check.** Lean exports the elaborated proof term via its existing export format. Eigenius has a Rust-native Lean term checker that re-verifies. The proof term becomes a first-class Eigon resource.
- **Option C — Translate to Mini-TT.** Lean proofs round-trip through Mini-TT. Faithful translation is a research project and fails for most of modern Lean.

**Option B is the working choice.** It preserves the "tools that reason, kernel that validates" discipline, keeps Lean stable as an external system, and allows the verification capability's trusted base to be the Lean term checker rather than all of Lean.

In D14 vocabulary, Option B is the choice that the `ProofCheck` QueryClass's `implementation` is **institution-runtime** (a procedure dispatched to `Institution::query`) rather than a Mini-TT Component. A Mini-TT Component cannot re-check a Lean proof term — Mini-TT's CIC fragment is too small. The `query` handler is where nanoda_lib lives. See [D14 §6.2](d14-institution-realisation.md) for the Component-vs-institution-runtime distinction.

### 2.2 Trusted computing base

- The Eigenius kernel: unchanged. Does not grow by a single line.
- The Lean verification institution's TCB: the Rust-native Lean term checker, the `EigonFFI` correspondence library (Section 5), and the serialization layer that moves proof terms between Lean and Eigenius.
- The Lean authoring-side workflows (export, generation, environment instantiation) run on top of the runtime substrate's TCB ([`runtime-substrate.md`](runtime-substrate.md) §2.3). That TCB is broader than the verification side's, but the artifacts it produces (proof terms, `EigonFFI` libraries, environment images) are themselves re-checked or content-anchored before any *verified* claim depends on them. The factoring is detailed in §2.3.
- Blast radius of a bug in the Lean checker: confined to the Lean institution's fiber. Cannot invalidate *derived* conclusions or corrupt the ontology.

### 2.3 Substrate factoring: hosted authoring vs in-process verification

The Lean integration touches the platform in two distinct places, and they have different trust postures:

- **Authoring side** — running `lean4export` against a Lean project, generating `EigonFFI` libraries with `eigon-ffi-gen`, instantiating `LeanEnvironment` images with the pinned Lean toolchain + Mathlib + dependencies. These are **language-toolchain workflows**: pin a runtime, run a tool, capture the output as a graph resource. They are exactly what the [runtime substrate](runtime-substrate.md) is for.
- **Verification side** — re-checking exported proof terms via nanoda_lib (the `qc_proof_check` AutoOnLoad QueryClass implementation, §3.3), performing the three-part correspondence check (§5.5), promoting the `LeanProofTerm` resource's epistemic status to *verified* on `Verdict::Holds`. This is **kernel-level term checking**: a small, auditable Rust crate with a tightly bounded TCB. It runs in-process inside the Eigenius orchestrator, exactly where the verification trust posture demands.

The factoring is therefore:

| Workflow | Where it runs | Why |
|---|---|---|
| `lean4export <project>` → `LeanProofTerm` resource | Substrate-hosted (`RunLeanExport` component) | Operationally reproducible; needs pinned Lean toolchain; benefits from image-digest anchoring |
| `eigon-ffi-gen --layer L` → `GeneratedLibrary` resource | Substrate-hosted (`RunEigonFFIGen` component, or a substrate-hosted `RunRuntimeScript` against a `lean-tools` env) | Same reasons; deterministic generator pinned by image digest |
| `LeanEnvironment` image build | Substrate's image-build pipeline ([`runtime-substrate.md`](runtime-substrate.md) §9.2) | Same as any other `RuntimeEnvironment` image build |
| Term checking via nanoda_lib | In-process Rust call from `eigenius-lean` | Trust surface must be small; no IPC, no Lean toolchain at runtime; result is the *verified* warrant the rest of Eigenius depends on |
| Three-part correspondence check (§5.5) | In-process Rust call | Load-bearing for soundness; must run alongside term checking |

This split is asymmetric on purpose. The authoring side produces operationally-reproducible artifacts whose value is "we ran this exact pinned tool against this exact pinned input and got these bytes" — the substrate's posture exactly. The verification side produces a mathematically-reproducible verdict whose value is "we re-checked this proof term against its proposition and environment and the kernel rules accepted it" — a posture the substrate cannot offer because it cannot in-process re-check. See [`runtime-substrate.md`](runtime-substrate.md) §2.2 for the substrate-side framing.

What this gives the integration:

- **Operational simplicity.** The authoring-side workflows are just substrate components. They get image pinning, worker pools, sandboxing, RPC framing, and provenance assembly for free. No bespoke Lean process management lives in `eigenius-lean`.
- **Authoring-side reproducibility for free.** A `LeanProofTerm` resource produced by `RunLeanExport` carries the same `RuntimeInvocation` provenance that any substrate dispatch carries — image digest, environment IRI, input IRIs, dispatched-to method, numerical metadata. Re-running export against the same Lean project on the same image digest yields byte-identical proof-term bytes.
- **Verification-side trust posture preserved.** The in-process checker is unchanged. It receives a `LeanProofTerm` resource (whose bytes the substrate already vouched for via its own provenance chain) and re-checks. The trust surface for the *verdict* remains nanoda_lib + the correspondence logic + the generator's faithful-translation specification.
- **Closed audit chain.** An auditor verifying a *verified* `LeanProofTerm` walks: claim reference → `LeanProofTerm` resource → substrate `RuntimeInvocation` (proves the proof bytes came from `lean4export` on a pinned image against a pinned Lean project) → nanoda_lib re-check (proves the term type-checks) → correspondence check (proves the proposition matches the claim). Every step is a graph-internal computation; nothing external to Eigenius is consulted at audit time.

## 3. Declared surface — Lean-specific resource classes and QueryClasses

Under D14, an institution exposes itself by committing typed Resources to the layer chain ([D14 §4](d14-institution-realisation.md)). The Lean institution commits the following.

### 3.1 Resource classes (Lean-specific Eigon classes)

These are ordinary Eigon classes living in the Lean institution's ontology layer. They are *sentences* in the institution's logic ([D14 §2.2](d14-institution-realisation.md)) — typed claims, not models.

- **`LeanProofTerm`** — the central class. Carries the elaborated proof term bytes (CBOR-encoded `lean4export` output), the claimed proposition (stated in terms of `EigonFFI` mirror types — see §5), the environment reference, the `EigonFFI` library reference, the Eigenius-side claim it vouches for, and the claim layer hash. A `LeanProofTerm` resource entering the chain triggers `ProofCheck` (§3.3) automatically. On `Holds`, the resource's epistemic status promotes from *derived* to *verified*.
- **`LeanEnvironment`** — pinned environment as a `RuntimeEnvironment` subclass (§7).
- **`LeanProject`** — a `RuntimePackage` subclass: the Lean source tree the user authored against, used by the substrate-hosted authoring side (§6.2).
- **`LeanPackageMirror`** (alias `GeneratedLibrary` in §5 wording) — a `RuntimePackageMirror` subclass: the `EigonFFI` library, also tracked by the substrate.
- **`DependencyOn`** — relates a `LeanProofTerm` to an axiom or previously-verified lemma it depends on. Discovered by an OnDemand QueryClass (§3.4); not auto-fired.
- **`ReducesTo`** — relates two Lean terms by definitional equality. Discovered by an OnDemand QueryClass; intra-fibre structure that programs may query.

D10's `ProofOf(term, proposition)` morphism class is no longer a separate notion under D14: a `LeanProofTerm` *is* the proof-of relation, since the resource itself carries both term and proposition. Promoting from *derived* to *verified* is recorded by the kernel's epistemic-status provenance ([D14 §7.1](d14-institution-realisation.md)) when the AutoOnLoad QueryClass returns `Holds`; the doc no longer needs a separate `ProofOf` resource class.

### 3.2 `ExportFormat` declarations

Per [D14 §4.2](d14-institution-realisation.md), each typed extraction the institution exposes is an ExportFormat resource.

| ExportFormat | `from_class` | `payload_type` | `procedure` | Used by |
|---|---|---|---|---|
| `ef_lean_proof_payload` | `LeanProofTerm` | `(ProofTermBytes, PropositionRepr, EnvironmentRef, MirrorRef)` (a Mini-TT record / inductive) | `urn:eigenius:lean:extract_proof_payload` | The `ProofCheck` AutoOnLoad QueryClass (§3.3); the `WhichAxioms` and `ProofSize` OnDemand queries |
| `ef_lean_environment_summary` | `LeanEnvironment` | `EnvironmentSummary` (Lean version + library list + image digest) | `urn:eigenius:lean:extract_env_summary` | `EnvironmentDiff` OnDemand query |

The `payload_type` for `ef_lean_proof_payload` is a typed Mini-TT inductive (or record) defined in the Lean institution's ontology layer. It is the *only* shape the in-process checker receives; it is also the shape any future cross-institution Comorphism would consume on the source side.

### 3.3 `QueryClass` declarations

Per [D14 §4.4 and §6](d14-institution-realisation.md), the Lean institution's reasoning surface is a set of QueryClasses with appropriate dispatch roles.

| QueryClass | `query_class` | `result_class` | `dispatch_role` | `implementation` | Notes |
|---|---|---|---|---|---|
| `qc_proof_check` | `LeanProofTerm` | `Verdict` | `AutoOnLoad`, `OnDemand` | institution-runtime — `urn:eigenius:lean:proof_check` | The load-bearing one. Implementation calls nanoda_lib in-process. AutoOnLoad fires on commit; OnDemand permits "what if I submitted this?" probes via FIBER. |
| `qc_which_axioms` | `LeanProofTerm` | `LeanAxiomList` | `OnDemand` | institution-runtime — `urn:eigenius:lean:which_axioms` | Walks the proof term, extracts the non-reducible axiom references. Diagnostic query for compliance / audit. |
| `qc_proof_size` | `LeanProofTerm` | `LeanProofMetrics` | `OnDemand` | institution-runtime — `urn:eigenius:lean:proof_size` | Term-size metrics. |
| `qc_environment_diff` | `LeanEnvironmentDiffInput` (a wrapper class with two `LeanEnvironment` references) | `LeanEnvironmentDiff` | `OnDemand` | institution-runtime — `urn:eigenius:lean:env_diff` | Coarse equal/not-equal verdict in v1; structured diff later. |
| `qc_proof_search` | `LeanProposition` (a wrapper class carrying a goal proposition + environment) | `LeanProofTerm` (or null) | `OnDemand` | institution-runtime — `urn:eigenius:lean:proof_search` | Optional. If Lean's hammer / built-in tactics produce a proof, return it as a `LeanProofTerm` resource for the caller to commit. |

`qc_proof_check` carries both `AutoOnLoad` and `OnDemand` roles in the same declaration — D14 permits this. The kernel dispatches the same procedure either way; the role just selects the trigger (Load commit vs. explicit FIBER).

### 3.4 `ImportFormat` and `Comorphism` declarations

The Lean institution declares **no ImportFormats and no Comorphisms in v1**. Lean is a *verification* institution: it consumes proof terms and produces verdicts. It does not act as the *target* of any cross-institution comorphism in v1, because turning some other institution's typed payload into a Lean proof would require synthesising a proof — beyond v1 scope.

Future work (see also [`julia-institutions.md`](julia-institutions.md) §6.2): a `Comorphism` from a Julia `IntervalArithmetic.BoundedBy` to a `LeanProofTerm` whose proposition asserts the same bound; the transformation Component would package the interval data + the Lean proof obligation; an `ImportFormat` on the Lean side would construct the resulting `LeanProofTerm`. The proof itself would still need to be supplied externally — the comorphism is plumbing for the structurally-aligned bridge, not a proof generator.

### 3.5 Structural properties (advisory, not kernel-enforced)

- `LeanProofTerm` is functional modulo definitional equality: a given term proves exactly one proposition up to the environment's conversion rules. Not a kernel invariant; the institution's `qc_proof_check` enforces it where it matters.
- `DependencyOn` is transitive; the OnDemand `qc_which_axioms` reports the transitive closure up to a configurable depth.
- `ReducesTo` is confluent within Lean's reduction system; query results are unique up to canonicalisation.

These were "structural properties" in D10's vocabulary. Under D14 they remain advisory metadata about institution-internal relations; they are not formal kernel invariants and they no longer have a dedicated declaration mechanism.

## 4. Institution trait realisation

Per [D14 §8](d14-institution-realisation.md), the Lean institution implements the three-method `Institution` trait: `extract_typed`, `reify`, and (since `qc_proof_check` and friends are institution-runtime) `query`.

### 4.1 `extract_typed`

Called by the kernel when an ExportFormat (§3.2) is dispatched — for the AutoOnLoad QueryClass on Load, for OnDemand FIBER queries, and for any future Comorphism whose source side is Lean.

For `ef_lean_proof_payload`: read the `LeanProofTerm` resource's properties; pack the proof term bytes, proposition representation, environment IRI, and mirror IRI into a Mini-TT value matching the declared `payload_type`. CBOR-encode for `typed-value` transport ([D14 §12](d14-institution-realisation.md)).

For `ef_lean_environment_summary`: read the `LeanEnvironment` resource; pack the summary record.

### 4.2 `reify`

In v1 the Lean institution declares no ImportFormats (§3.4), so `reify` is unreachable. The trait method must still exist; the implementation returns `InstitutionError::NotImplemented` for any procedure IRI it receives. When a future Lean-target Comorphism lands, this gains content.

### 4.3 `query`

The work centre. Dispatched by the kernel for every QueryClass whose `implementation` is institution-runtime — i.e. all of §3.3 — regardless of whether the trigger was AutoOnLoad, Decidable, or OnDemand.

Procedure dispatch:

- **`urn:eigenius:lean:proof_check`** — the load-bearing handler. The input resource is a `LeanProofTerm`; the body performs the three-part correspondence check (§5.5), runs nanoda_lib on the term, and returns a `Verdict` resource:
  - `Holds` if the proof checks and the correspondence is sound.
  - `Fails` if the proof rejects, with diagnostic detail in the verdict's auxiliary fields.
  - `Undecidable` is unused by Lean — proof checking is decidable in nanoda_lib's regime; it's `Holds` or `Fails`.
- **`urn:eigenius:lean:which_axioms`** — walks the proof term, returns a `LeanAxiomList` resource enumerating the axioms reached.
- **`urn:eigenius:lean:proof_size`** — returns a `LeanProofMetrics` resource with term-level counters.
- **`urn:eigenius:lean:env_diff`** — accepts a `LeanEnvironmentDiffInput`, returns a `LeanEnvironmentDiff` resource.
- **`urn:eigenius:lean:proof_search`** (if implemented) — accepts a `LeanProposition`, attempts proof, returns a `LeanProofTerm` or a null/empty result.

The procedure dispatch is a simple match on `procedure_iri` inside the trait method; the institution does not re-derive intent from the input class, since the QueryClass declaration already pinned the input class.

### 4.4 Discovery (replacing D10 `discover_morphisms`)

Under D10 the institution had a separate `discover_morphisms` method that returned candidate morphisms (e.g. `DependencyOn`, `ReducesTo`) for a given input. Under D14 ([§6 table](d14-institution-realisation.md)) discovery is just an OnDemand QueryClass whose `result_class` is a list of resource IRIs. Lean exposes:

- **`qc_discover_dependencies`** (`LeanProofTerm` → `LeanDependencySet`, `OnDemand`) — returns a list of `DependencyOn` candidate resources.
- **`qc_discover_reductions`** (`LeanProofTerm` → `LeanReductionSet`, `OnDemand`) — returns a list of `ReducesTo` candidate resources up to a configured depth.

The institution still has no write access to the graph: discovery returns candidates; the calling program commits those it wants ([D14 §6 row 4](d14-institution-realisation.md)).

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

The correspondence check is the body of `qc_proof_check`'s institution-runtime implementation (§4.3). When the kernel fires the AutoOnLoad QueryClass — or when an OnDemand caller invokes it via FIBER — the institution's `query` handler performs three checks, in order. The first is ordinary Lean verification; the second and third are what make the correspondence sound.

1. **Proof validity.** The Lean term checker verifies that the proof type-checks against its stated proposition under its declared environment.
2. **Mirror correspondence.** The verification institution resolves the `LeanPackageMirror` IRI in the `LeanProofTerm` resource to a committed mirror, reads its declared anchor, and checks that the library was anchored to a layer ancestral to or identical with the layer in which the claim's class is defined. The specific mirror type used in the proposition must correspond structurally to that class.
3. **Anchor consistency.** The mirror's declared content hash is verified against its actual content, and the layer hash it declares is confirmed to resolve in the current layer chain. Optionally, the generator identified in the resource's provenance can be re-invoked to confirm that it reproduces the committed library — this is the independent provenance verification described in Section 5.3, and can be performed at any time after submission, not only at check time.

The QueryClass returns `Verdict::Holds` if all three pass, `Verdict::Fails` if any fails (with the diagnostic detail attached to the verdict's auxiliary fields), and never returns `Undecidable` for proof checking — proof checking under nanoda_lib's regime is a binary verdict.

Check 2 is the load-bearing one for soundness. It is what prevents a proof about an `EigonFFI.StressResult` (anchored to an old ontology state) from being accepted as verification for a resource whose class has since acquired new required properties.

### 5.6 Compositionality under layer extension

An important property of the anchor design is that an `EigonFFI` library does *not* need to be regenerated every time the ontology evolves. A library anchored to layer L₀ remains valid for claims in any descendant layer L₁ ⊒ L₀, provided the classes the library mirrors are unchanged in L₁.

This is the common case. A user generates `EigonFFI` from a layer containing Core plus FEA. They later commit their bracket analysis in a descendant layer; that layer adds domain data (specific stress results, meshes, materials) but does not modify the FEA class definitions. Their proof, authored against the L₀ mirror of `StressResult`, remains valid for verifying claims about stress results in the later layer. The correspondence check confirms that `StressResult`'s class definition is byte-identical in the resolved layer chain and accepts the proof.

The failure mode is symmetric. If a later layer L₂ modifies `StressResult` — adds a new required property, for instance — then proofs anchored to L₀ fail the correspondence check when submitted against L₂ claims. The rejection surfaces as `qc_proof_check` returning `Verdict::Fails` with diagnostic field `FFIVersionMismatch`, naming which mirror type no longer matches which class. The user regenerates `EigonFFI` from L₂ (or a descendant), re-states their proof against the updated mirror, and re-commits the `LeanProofTerm` resource.

This is the desirable behavior. It is the anchoring doing its job: catching exactly the cases where proof soundness would otherwise silently degrade. Users regenerate `EigonFFI` when the ontology changes in ways that matter to their proofs, and not otherwise.

### 5.7 The closed chain from ontology to archived verdict

Putting the pieces together, the full chain of anchored and tracked artifacts in a verification is:

1. Ontology content is committed to Eigenius, producing layer hash L.
2. `eigon-ffi-gen` is run against L (substrate-hosted as `RunEigonFFIGen`, §6.2). Its output is both a Lean source file (delivered as part of a `LeanPackage` artifact) and a committed `LeanPackageMirror` resource in Eigenius, carrying the library's content hash, the source layer reference, and the generator's own content hash.
3. The Lean source is imported into a `LeanProject` resource; a proof is authored against its mirror types.
4. The proof elaborates against a specific `LeanEnvironment` resource E (image-digest-pinned, §7).
5. `RunLeanExport` (substrate component, §6.2) produces a `LeanProofTerm` resource carrying the exported term bytes, the proposition (in `EigonFFI` mirror terms), the environment reference, the mirror reference, the claim reference, and the claim layer hash.
6. The kernel commits the `LeanProofTerm` to the chain. Structural validation passes.
7. The kernel's D14 dispatch fires the `qc_proof_check` AutoOnLoad QueryClass, calling `Institution::query` with `urn:eigenius:lean:proof_check`. The handler runs the three-part correspondence check (§5.5).
8. On `Verdict::Holds`, the kernel admits the resource and tags its epistemic status *verified*. On `Verdict::Fails`, the Load is aborted with the verdict's diagnostic.

At every step, every artifact is content-addressed and tracked as a resource within Eigenius. The verification verdict is a pure function of archived inputs, and every input lives in the knowledge graph itself rather than in external storage. Given the graph, the verdict reproduces at any future point and yields the same result. Independent auditors can verify each step locally: re-check the proof against the environment, re-run the generator against the source layer to reproduce the library hash, re-evaluate the correspondence check against the committed classes.

This is the reproducibility property that distinguishes verification from the other epistemic categories: *derived* resources depend on traces that can be re-executed but produce new traces each time; *verified* resources depend on anchored, tracked artifacts that reproduce identically and forever. The move from "tracked by external hash" to "tracked as committed resource" is what makes the archive self-contained.

### 5.8 TCB implications

The Lean verification institution's trusted computing base has three components:

- The **Lean term checker** — Rust-native, shared across deployments. Invoked from inside the `qc_proof_check` `Institution::query` handler.
- The **`EigonFFI` generator** — deterministic, with a separately specified faithful-translation contract. Runs as a substrate-hosted component (§6.2); image-digest-pinned.
- The **correspondence logic** in the `qc_proof_check` handler (formerly `validate_morphism` in D10 vocabulary) — the code that matches mirror types to Eigon classes and verifies anchor consistency.

The kernel's TCB does not grow. The blast radius of a bug in any of the three is confined to the verification institution's fiber: bad proofs may be accepted, but *derived* conclusions and the ontology itself remain unaffected.

This TCB is larger than the original single-component version (just the Lean checker) but much smaller than the full-translation alternative (Option C), and each component is independently auditable. The generator's faithful-translation specification in particular is a finite piece of design work that can be reviewed once and then relied upon across all subsequent proofs.

## 6. Proof term transport

### 6.1 Lean 4 export format

Lean 4 can export elaborated terms in a well-specified format produced by the `lean4export` tool (see Appendix A.1). The format is designed to be consumed by external checkers; nanoda_lib (the chosen dependency, Section 8.1) parses it directly. This is the format Eigenius consumes at the institution boundary.

### 6.2 `lean4export` as a substrate component

Per the substrate factoring (§2.3), the `lean4export` invocation is **not** a bespoke pipeline inside `eigenius-lean`. It is a substrate-hosted component called `RunLeanExport`, dispatched against a `LeanEnvironment` image (which extends `RuntimeEnvironment` — see §7). The component:

1. Resolves the input `LeanProject` resource (a `RuntimePackage` subclass that carries the `lakefile.lean`, source tree, and environment dependency).
2. Dispatches into a substrate worker container backed by the project's `LeanEnvironment` image digest.
3. Runs `lean4export` inside the container against the project.
4. Returns the export bytes as a `LeanProofTerm` resource (CBOR-encoded for transport, content-addressed by hash).
5. Substrate assembles a `RuntimeInvocation` provenance record carrying the image digest, project IRI, environment IRI, and runtime metadata.

The `LeanProofTerm` resource the verification side receives therefore carries an audit trail back to a specific Lean project, a specific Lean toolchain version, and a specific image digest — all without `eigenius-lean` having to write its own process-management or RPC code. The same machinery that hosts Julia computations hosts Lean's authoring side.

`eigon-ffi-gen` follows the same pattern: it runs as a substrate component (`RunEigonFFIGen`, or equivalently a `RunRuntimeScript` against a `lean-tools` environment) producing a `GeneratedLibrary` resource. The deterministic-generator requirement from §5.4 is enforced exactly the way the substrate enforces image-digest determinism for any of its hosted runtimes.

### 6.3 Resource encoding — the `LeanProofTerm` resource

D10 introduced a separate `LeanProofSubmission` wrapper that carried (proof_term, environment, EigonFFI, claim) references and was distinct from the proof term itself. Under D14 there is no wrapper: the `LeanProofTerm` resource carries those references directly, and the AutoOnLoad QueryClass fires on the resource as soon as it enters the chain. A "submission" *is* a `LeanProofTerm` commit.

A `LeanProofTerm` resource carries the full anchor chain described in Section 5.7:

- `proof_term_bytes` — the exported term, CBOR-encoded. Embedded for small proofs; externally referenced (content-addressed) for large ones. Produced by `RunLeanExport` (§6.2); the resource's provenance points back to the exporting `RuntimeInvocation`.
- `claimed_proposition` — the Lean proposition the term claims to prove, stated in terms of `EigonFFI` mirror types.
- `environment_reference` — content-addressed IRI of the `LeanEnvironment` resource the term was elaborated against (§7).
- `mirror_reference` — IRI of the `LeanPackageMirror` (`EigonFFI` library) used, which resolves to the committed mirror with its full substrate provenance.
- `eigenius_claim_reference` — IRI of the Eigon-side resource this proof vouches for.
- `claim_layer_hash` — layer hash at which the claim exists; determines which ontology state the correspondence check resolves against.

All references resolve within the knowledge graph itself. A committed `LeanProofTerm` is a closed object: given its properties, every artifact the verdict depends on is retrievable from Eigenius alone, without external dependencies. The `qc_proof_check` AutoOnLoad QueryClass is bound to this class, so commit time *is* check time.

### 6.4 Content addressing and cache

The `LeanProofTerm`'s content hash (a function of all the fields above) is the cache key for verification results. A re-commit of an identical resource is a no-op: the resource IRI is already present, and the cached verdict is served. A `LeanProofTerm` whose proof bytes are unchanged but whose `claim_layer_hash` differs is a different resource — the AutoOnLoad QueryClass re-runs because the correspondence check resolves against a different ontology state.

## 7. Environment management

Mathlib is large. A naive "load the environment per request" design is non-viable. The substrate already solves the language-toolchain pinning problem; Lean reuses its solution.

### 7.1 `LeanEnvironment` extends `RuntimeEnvironment`

A `LeanEnvironment` is a typed Eigon resource that subclasses the substrate's [`RuntimeEnvironment`](runtime-substrate.md) ([`runtime-substrate.md`](runtime-substrate.md) §5.3). It is immutable, content-addressed, and image-digest-anchored on the same terms as Julia's `JuliaEnvironment`.

| Property | Inherited / new | Purpose |
|---|---|---|
| `language` | inherited | Always `"lean"`. |
| `runtime_version` | inherited | Exact Lean version (`"4.10.0"`). |
| `manifest` | inherited | The Lake manifest (`lake-manifest.json`) — verbatim bytes, the round-trip anchor for re-instantiation. |
| `pinned_packages` | inherited | List of `LeanPackagePin` IRIs (subclasses of `RuntimePackagePin`) — parsed Eigon view of the manifest. Optional, lands when graph queries need it. |
| `included_packages` | inherited | List of `LeanPackage` IRIs (user-authored Lean libraries baked into the image, e.g. an `EigonFFI` mirror used as a Lake dependency). |
| `mirror_dependency` | inherited | Optional IRI of a `LeanPackageMirror` (subclass of `RuntimePackageMirror`) baked into the image — the `EigonFFI` mirror used by the authoring side. |
| `image_digest` | inherited | OCI image digest pinning Lean toolchain + Mathlib + dependencies + `EigonFFI` mirror. |
| `image_reference` | inherited | Optional registry tag like `registry.eigenius.io/runtime/lean-mathlib:4.10`. |
| `lake_lockfile_hash` | new | Hash of the Lake lockfile, separate from `manifest` to make Lake-specific tooling queries easy. |

Consequences:

- The "load the environment" problem reduces to "pull the image"; the substrate's worker pool already caches pulled images.
- "Multiple registrations share an environment" reduces to "multiple registrations reference the same image digest"; substrate-level deduplication is automatic.
- "Change to the environment produces a new resource" reduces to "new image digest produces a new content-addressed `LeanEnvironment` IRI"; nothing Lean-specific is needed.

### 7.2 Pinning and sharing

Institution registrations pin the `LeanEnvironment` IRI (and therefore the image digest) they expect. Prior `LeanProofTerm` resources tagged *verified* remain valid against their original environment IRIs even after newer environments are registered; the substrate doesn't implicitly upgrade and neither does the verification institution.

### 7.3 Caching

The substrate's worker pool ([`runtime-substrate.md`](runtime-substrate.md) §8) handles environment caching for the *authoring* side: warm Lean toolchain workers per environment digest, LRU eviction, image pull cache. The verification side holds nanoda_lib's parsed environment representation in a separate in-process LRU cache indexed by `LeanEnvironment` IRI; first checking request loads from the resource bytes (or from a baked artifact in the corresponding image, fast path), subsequent requests are served from cache. Eviction is policy-driven; default is "keep the working set Mathlib's worth fits in memory."

### 7.4 Environment diffing

Two `LeanEnvironment` resources can differ in library versions, axiom sets, or definitional transparencies. Structured diffing (the `EnvironmentDiff` query of Section 4.3) is non-trivial to implement well, but even a coarse version — "these environments are different, proofs checked against one are not transferable to the other" — is valuable as a safety rail. With the substrate factoring, the coarse version is trivially "the image digests differ"; the structured version is a targeted analysis run as a substrate-hosted component when needed.

## 8. Implementation approach

The original version of this outline listed three deployment shapes as options under evaluation. With the survey of existing external checkers summarized in Appendix A, the decision has effectively been made: Eigenius will use an **in-process Rust checker** (the shape described in Section 8.1), with **nanoda_lib** as the concrete dependency. The other shapes are retained in Sections 8.2 and 8.3 as context for why this one was chosen and where it might evolve.

### 8.1 In-process Rust checker (chosen approach)

The Lean term **checker** runs as a Rust library linked into the `eigenius-lean` crate. Proof terms produced by `lean4export` (which runs *separately*, on the substrate-hosted authoring side per §2.3 and §6.2) are parsed and checked without leaving the Eigenius orchestrator process. The specific library is **nanoda_lib** ([github.com/ammkrn/nanoda_lib](https://github.com/ammkrn/nanoda_lib)), maintained by Chris Bailey, accompanied by the specification book [Type Checking in Lean 4](https://ammkrn.github.io/type_checking_in_lean4/).

This approach is preferred because:

- **Latency and integration on the verification path.** No cross-process IPC, no serialization of proof terms over a service boundary, no Lean toolchain on the verification path. (The Lean toolchain *does* run elsewhere — on the authoring side, hosted by the substrate — but the *verifier* never touches it.) The verification institution is a function call away from the evaluator.
- **Trust surface is auditable Rust.** The verification-side code is available, readable, and sized for independent review. No Lean source-level machinery (elaborator, tactic framework, metaprogramming) is in the verification TCB — only the kernel-level checking logic. The authoring-side TCB is broader (it includes the substrate plus the hosted Lean toolchain), but everything the authoring side produces is content-anchored and re-verified before any *verified* claim depends on it.
- **Pre-existing axiom governance.** nanoda_lib already implements axiom allow-listing via its configuration. This provides the mechanism for the policy decisions raised in Section 12, question 2 — Eigenius still needs to choose *which* axioms to permit, but no implementation work is required to enforce the choice. The permitted-axioms list is declarative and inspectable.
- **Library-shaped.** nanoda_lib is designed to be embedded as a crate dependency, not only invoked as a binary. This matches the Eigenius requirement that the checker be dispatchable from inside the `Institution::query` handler ([D14 §8](d14-institution-realisation.md)) without spawning a process.

The cost is an ongoing maintenance relationship: nanoda_lib must track upstream Lean kernel changes, and Eigenius must track nanoda_lib. This is not a one-time integration. See Section 11 Phase C and Appendix A.3 for the maintenance considerations.

**Cross-checking as a soundness multiplier.** An additional use of the Lean4Lean checker (Appendix A.2) as a *secondary* verifier is recommended: if nanoda_lib and Lean4Lean disagree on a verdict, the submission is flagged for review rather than accepted. This is the Venn-diagram argument from [Type Checking in Lean 4](https://ammkrn.github.io/type_checking_in_lean4/trust/trust.html): the part where independent checker circles intersect is the stronger soundness claim. This is not part of Phase A, but is a natural enhancement during Phase C hardening.

### 8.2 Out-of-process Lean service (rejected for the primary path)

A Lean 4 process running as a service, dispatched to via gRPC. Lean checks the term in its own kernel; Eigenius receives the verdict.

This is Option A from Section 2.1 in disguise. The trust surface becomes all of Lean plus the service boundary, which defeats the Option B rationale. It is retained here only to note that it remains acceptable as a *bootstrap* during early Phase A experimentation, before the nanoda_lib integration is live — specifically for validating that the protocol end-to-end works before committing to the Rust dependency. It is not the long-term design.

### 8.3 WASM-sandboxed checker (future option)

The Rust checker compiled to WASM and run under the platform's existing untrusted-capability sandbox. Uniform with how other untrusted capabilities run; fuel-bounded; memory-isolated; dispatches through the same path as other WASM institutions.

The performance overhead on large Mathlib-scale proofs is likely meaningful, and the engineering to make nanoda_lib run cleanly under WASM is non-trivial. This remains a future option worth evaluating once the WASM capability infrastructure is mature and once realistic benchmarks exist, but is out of scope for Phases A through D.

## 9. Verdict shape, error taxonomy, and runtime properties

D10 framed institutional contracts as a `BoundaryContract` instance per institution. Under D14, those contracts collapse into the typed declarations of §3 (resource classes, ExportFormats, ImportFormats, QueryClasses, Comorphisms) plus the kernel-defined `Verdict` shape ([D14 §6.1](d14-institution-realisation.md)). This section names the Lean-specific concretisations of those contract concerns.

### 9.1 Verdict diagnostics — the failure detail surfaced through `Verdict::Fails`

The `qc_proof_check` QueryClass returns a `Verdict` resource. On `Fails`, the verdict's diagnostic field carries one of:

- `ProofDoesNotCheck` — Lean term checker rejected the proof. Diagnostic payload includes the Lean error message from nanoda_lib.
- `PropositionMismatch` — proof checks but its proposition does not correspond to the Eigenius-side claim. Diagnostic includes the expected and actual propositions.
- `EnvironmentUnavailable` — the referenced `LeanEnvironment` resource is not loadable (its image isn't pullable, or its bytes are gone).
- `EnvironmentMismatch` — the proof was elaborated against a different environment than declared.
- `FFIVersionMismatch` — the `LeanPackageMirror` was anchored to a layer that doesn't cover the claim's class definition.

These are diagnostic field values, not separate result classes. The kernel's response to `Verdict::Fails` is uniform regardless of which diagnostic fired: the Load is rejected, the diagnostic is surfaced to the caller, the resource never enters the chain. The OnDemand variant of `qc_proof_check` returns the verdict as data without rejecting anything.

### 9.2 Runtime properties (advisory)

These are not declared in any single resource; they are properties of the institution's behaviour that operators rely on for capacity planning and audit reasoning:

- **Determinism.** Given the same `LeanProofTerm` resource (i.e. same proof bytes + proposition + environment IRI + mirror IRI + claim layer hash), the verdict is constant.
- **Idempotence.** Re-firing the AutoOnLoad QueryClass on the same resource (e.g. on rehydration) yields the same verdict.
- **Effects.** The handler is read-only against the chain; it does not commit resources. (It may produce candidate `DependencyOn` / `ReducesTo` resources via the OnDemand discovery QueryClasses, but those are only candidates the caller chooses to commit.)
- **Resource bounds.** Per-proof `max_wall_time_ms` and `max_memory_bytes`. Mathlib-scale proofs can be large; defaults need to be generous but bounded. Bound violations surface as `Verdict::Fails` with a `ResourceLimitExceeded` diagnostic.

### 9.3 Lifecycle and versioning

The institution registers by committing its `Institution`, ExportFormat, ImportFormat (none in v1), QueryClass, and Comorphism (none in v1) resources to a layer. Upgrades produce new resources at new content-addressed IRIs in later layers; prior registrations remain valid for their trace history. A `LeanProofTerm` checked against version 1 of `qc_proof_check` carries that QueryClass IRI in its provenance, and the verdict reproduces deterministically against the original logic even after a new QueryClass is registered.

## 10. Integration touchpoints in the existing code

### 10.1 Kernel-side changes

- None to the Mini-TT type theory, its evaluator, or its type checker.
- The kernel's D14 institution registry ([D14 §9](d14-institution-realisation.md)) — already specified — picks up the Lean institution's declarations from chain scan. No Lean-specific kernel code.
- The kernel's commit-time type-checker for `Comorphism` resources ([D14 §4.5](d14-institution-realisation.md)) is irrelevant in v1 because Lean declares no comorphisms; gains relevance when the future Lean-target Comorphism work (§3.4) lands.

### 10.2 Orchestrator-side additions

- A new Rust crate (working name `eigenius-lean`) housing the Lean term checker (nanoda_lib wrapper), the `EigonFFI` correspondence logic, the in-process environment cache, and the D14 `Institution` trait implementation ([D14 §8](d14-institution-realisation.md)). This is the **verification side**; it runs in-process inside the orchestrator.
- The same crate (or a sibling such as `eigenius-lean-runtime`) implements the substrate's `LanguageRuntime` trait for the **authoring side**: Dockerfile fragments installing `elan` + the pinned Lean toolchain + Lake + Mathlib, the worker bootstrap that exposes `lean4export` / `eigon-ffi-gen` as RPC entry points, and the `LeanProject` / `LeanPackage` resource subclass declarations. Depends on `eigenius-runtime-substrate`.
- Registration with the kernel's D14 `InstitutionRegistry` (verification side) and with the substrate's `LanguageRuntime` registry (authoring side).
- gRPC surface for OnDemand QueryClass dispatch (reuses the kernel's existing institution dispatch plumbing). The substrate-component dispatch (`RunLeanExport`, `RunEigonFFIGen`) reuses the substrate's existing `ComponentExecutor` plumbing.

### 10.3 Ontology additions

The Lean institution's declarations land in `ontologies/lean/lean-institution.json` (or equivalent), structured by D14 resource shapes:

**`Institution` resource**: one — `urn:eigenius:institutions:lean`, runtime `InProcess` (verification side).

**Eigon classes** (Lean-specific resource classes, declared via ordinary class resources, not D14 declaration shapes):

- `LeanProofTerm`, `LeanEnvironment` (subclass of `RuntimeEnvironment`), `LeanProject` (subclass of `RuntimePackage`), `LeanPackage` (subclass of `RuntimePackage`), `LeanPackagePin` (subclass of `RuntimePackagePin`), `LeanPackageMirror` (subclass of `RuntimePackageMirror`; the `EigonFFI` library; alias `GeneratedLibrary` retained in §5 wording).
- `DependencyOn`, `ReducesTo` — intra-fibre relation resource classes.
- `LeanAxiomList`, `LeanProofMetrics`, `LeanEnvironmentDiff`, `LeanEnvironmentDiffInput`, `LeanProposition`, `LeanDependencySet`, `LeanReductionSet` — typed result classes for the OnDemand QueryClasses.
- `Verdict` — kernel-defined (D14 core) or institution-shared; not a Lean-only class.

D10's `LeanProofSubmission` and `ProofOf` resource classes are dropped — `LeanProofTerm` carries the references and the verification institution's QueryClass produces the verdict; the *verified* status is tagged by the kernel rather than recorded as a separate morphism resource.

**`ExportFormat` resources** (§3.2): `ef_lean_proof_payload`, `ef_lean_environment_summary`.

**`ImportFormat` resources**: none in v1 (§3.4).

**`QueryClass` resources** (§3.3): `qc_proof_check`, `qc_which_axioms`, `qc_proof_size`, `qc_environment_diff`, `qc_proof_search` (optional), `qc_discover_dependencies`, `qc_discover_reductions`.

**`Comorphism` resources**: none in v1 (§3.4).

**Substrate-component classes** (substrate-side, not D14 institution declarations): `RunLeanExport`, `RunEigonFFIGen` (or, if the team prefers reusing the generic verb, instances of `RunRuntimeScript` against a `lean-tools` environment with a declared entry point).

### 10.4 `EigonFFI` and its generator

- A new Rust crate (working name `eigon-ffi-gen`) implementing the generator. Deterministic by construction. Takes a layer hash as input, produces both a Lean source file for local consumption and a committed `LeanPackageMirror` resource in Eigenius. Part of the verification institution's trusted computing base.
- The generator runs **inside the substrate**, baked into the `lean-tools` `LeanEnvironment` image and dispatched via `RunEigonFFIGen`. This means the generator binary's content hash is captured at image-build time as part of the image-digest derivation; the per-`LeanPackageMirror` `generator_content_hash` field is then a redundant integrity check rather than the only anchor.
- A faithful-translation specification document, authored alongside the generator, that pins down how Eigon constructs map to Lean constructs.
- Initial scope: mirror types for the Core Ontology and for the first domain ontology against which proofs will be written.

## 11. Phased implementation plan

Scope is given as t-shirt sizes (Small / Medium / Large / Open-ended) rather than as time estimates, since duration depends on team size, parallelization, and deployment priorities. The phases are ordered by dependency — each builds on what came before — and the named scope reflects the relative size of the engineering commitment, not a calendar prediction.

### Phase A — Proof of concept

Integration of nanoda_lib into the `eigenius-lean` crate. Minimal `Institution` trait implementation ([D14 §8](d14-institution-realisation.md)) with `extract_typed` for `ef_lean_proof_payload` and `query` dispatching `urn:eigenius:lean:proof_check` to nanoda_lib. The `Institution`, ExportFormat, and `qc_proof_check` QueryClass declarations land as ordinary chain resources committed at registration time; the kernel's D14 dispatch fires the AutoOnLoad QueryClass on `LeanProofTerm` Loads. Toy propositions only — propositions stated directly about primitive types, no `EigonFFI` yet. Demonstrates end-to-end: a `LeanProofTerm` resource enters the chain, AutoOnLoad fires, nanoda_lib re-checks, `Verdict::Holds` admits the resource, the kernel tags it *verified*.

Phase A depends on D14 milestones M1–M5 ([D14 §13.4](d14-institution-realisation.md)) and substrate Phase A (the substrate skeleton — see [`runtime-substrate.md`](runtime-substrate.md) §13). The substrate side, the verification side, and the D14 dispatch land together; none of the three alone can prove out the integration.

Phase A's authoring side is the simplest substrate consumer: a `LeanEnvironment` image with the pinned Lean toolchain, a `RunLeanExport` substrate component that invokes `lean4export` against a `LeanProject`, and a hand-written test project producing toy proof terms.

Optional fallback: if either nanoda_lib integration or substrate dispatch hits unexpected obstacles, a brief out-of-process Lean service (Section 8.2) can serve as a bootstrap to validate the protocol end-to-end before returning to the primary integration. This is a contingency rather than a planned step.

**Scope:** Small-to-Medium. Validates D14 dispatch on a real institution; establishes the nanoda_lib dependency relationship; establishes the substrate-as-Lean-host dependency relationship.

### Phase B — `EigonFFI`, the generator, and real propositions

First version of the `eigon-ffi-gen` generator: deterministic implementation, faithful-translation specification authored in parallel, `LeanPackageMirror` resource committed back to Eigenius as part of each generation. First generated `EigonFFI` library mirroring Core Ontology types. The `mirror_reference` field on `LeanProofTerm` resources is exercised; the three-part correspondence check (§5.5) becomes the body of `urn:eigenius:lean:proof_check`'s handler, including the layer-ancestry logic that gives compositionality under layer extension.

The generator runs inside the substrate from Phase B onward (substrate-hosted `RunEigonFFIGen` against the `lean-tools` `LeanEnvironment`). Its determinism is verified by re-running and comparing content hashes, exactly the way the substrate's image-build pipeline verifies determinism for any of its hosted runtimes ([`runtime-substrate.md`](runtime-substrate.md) §9.2). This depends on substrate Phase B (mirror anchoring + boundary check).

The `qc_which_axioms`, `qc_proof_size`, `qc_environment_diff` OnDemand QueryClasses can land in Phase B opportunistically — they share the trait method dispatch that Phase A established, so adding them is a matter of authoring the QueryClass declarations and the procedure handlers.

**Scope:** Medium. This is where the trust-surface work happens, the faithful-translation specification gets authored, and the load-bearing design decisions about mirror structure and library tracking get crystallized.

### Phase C — Integration hardening and checker operational maturity

Performance work: profiling against realistic proof sizes, identifying hotspots in the dispatch path, tuning trace-cache policy. In-process environment caching infrastructure (the verification side's nanoda_lib environment cache). Substrate-side environment management is handled by substrate Phase C ([`runtime-substrate.md`](runtime-substrate.md) §13) and arrives "for free" — `LeanEnvironment` images get worker pools, LRU eviction, and image-pull caching from the substrate without per-language work. Upstream tracking protocol with nanoda_lib: establishing how Eigenius follows Lean kernel changes propagated through nanoda_lib, and what the version-pinning discipline looks like. Optional follow-on (recommended in Section 8.1): introduction of Lean4Lean as a secondary cross-checker per the Venn-diagram soundness argument.

**Scope:** Large. Smaller than the original framing because substrate's worker pool + image pipeline removes the bespoke environment-management work; the remaining engineering is verification-side optimisation and upstream-tracking discipline.

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

*This outline is a starting point. With D14 in place, the protocol-level shape no longer needs its own per-institution contract document; the next step is resolving the open questions below and authoring the faithful-translation specification for `eigon-ffi-gen`, at which point Phase A can begin.*

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

**Role in Eigenius.** The primary dependency for the `eigenius-lean` crate. Wrapped inside the `urn:eigenius:lean:proof_check` procedure handler — i.e. inside the D14 `Institution::query` method — that translates between the institution protocol and nanoda_lib's API. The axiom allow-listing mechanism addresses Section 12 open question on axiom policy directly.

**Maintenance posture.** nanoda_lib is primarily maintained by a single contributor. Eigenius takes on a relationship with the library that includes tracking upstream Lean kernel changes as they are propagated through nanoda_lib, and contributing back where integration surfaces bugs or gaps. This is a consideration for Phase C rather than an immediate concern.

### A.4 trepplein and successors

**Original (Lean 3):** [github.com/gebner/trepplein](https://github.com/gebner/trepplein) — Gabriel Ebner's Scala checker for Lean 3. Still exists but is a Lean 3 tool.

**Lean 4 successor attempt:** [siddhartha-gadgil.github.io/trepplein4](https://siddhartha-gadgil.github.io/trepplein4/) — a Lean 4 Scala successor by Siddhartha Gadgil, based on Gabriel's original. Less mature and less actively maintained than either Lean4Lean or nanoda_lib. Retained here for historical completeness: trepplein was the name cited in early Eigenius design discussions as a candidate reference implementation, and the lineage of external-checker thought traces through it.

**Role in Eigenius.** None directly. A Scala checker in the stack would impose a JVM dependency on the orchestrator that the Rust path avoids.

### A.5 What this survey settles

The availability of nanoda_lib as a library-shaped Rust dependency with an accompanying specification book, combined with Lean4Lean as a secondary cross-checker option, removes the "build a checker" work from the Eigenius roadmap. The decision in Section 8 reflects this: the implementation approach is not an open question but a dependency relationship. The remaining work is integration, axiom policy configuration, environment management, performance tuning, and maintenance coordination with the upstream library — all of which is tractable engineering, none of which is research.

This appendix should be revisited annually, or when any of the surveyed projects undergoes a major release, to verify that the underlying landscape assumptions still hold.