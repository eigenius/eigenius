# D28: Lean 4 as a Verification Institution in Eigenius

**Status:** Draft — calibrated against the live substrate (D26 shipped) and the Julia institution suite (D27/D29 shipped end-to-end).
**Scope:** What it takes to make Lean 4 a registered institution within Eigenius, contributing the *verified* epistemic level to the knowledge graph by realising the [D14](d14-institution-realisation.md) institution protocol.

**Related:**
- [D14 — Institution Realisation](d14-institution-realisation.md) — the institution protocol this doc instantiates. D14 specifies the trait surface, the five typed resource shapes, and the dispatch model; this doc fills in the Lean-specific instantiation.
- [D26 — Runtime Substrate](d26-runtime-substrate.md) — the substrate the Lean *authoring*-side toolchain runs on. The verification side stays in-process; see §2.3 for the factoring.
- [D27 — Julia Institutions](d27-julia-institutions.md) — the precedent for a multi-institution language integration. D28 reuses the substrate-mirror-institution pattern D27 settled.
- [D29 — Eigon → Julia Faithful Translation](d29-eigon-julia-mirror-spec.md) — the precedent for a per-language mirror specification. Lean's equivalent is a sibling document (§10.4).
- [D32 — Chain-Mirrored Mini-TT Inductives](d32-chain-mirrored-mini-tt-inductives.md) — the precedent for putting a typed term language on the chain. Lean's chain-mirrored expression form (§3.1) follows the same pattern.

## 1. Purpose and scope

Eigenius defines four epistemic categories: *declared*, *observed*, *derived*, and *verified*. The first three arise from ordinary operation of the system — human authorship, external ingestion, and typed pipeline execution with reasoning traces. The fourth requires a machine-checked proof, and thus requires an institution capable of checking proofs.

This document specifies what it takes to register **Lean 4** as that institution under the [D14 institution realisation protocol](d14-institution-realisation.md). The protocol-level shape (trait surface, resource declarations, dispatch model, Verdict, comorphism triadic shape) is fixed by D14; this doc fills in Lean-specific content for that shape.

### 1.1 D14 in one paragraph (so the rest of this doc is readable in isolation)

Under D14, an institution is registered by committing five kinds of typed Resources to the layer chain: an `Institution` (identity and runtime kind), `ExportFormat`s (typed extractions of class instances into Mini-TT payloads), `ImportFormat`s (typed constructors of class instances from Mini-TT payloads), `QueryClass`es (typed functions in the institution's fibre with a `dispatch_role` of `OnDemand` / `AutoOnLoad` / `Decidable` and a result class — `Verdict` for the gate-on-commit and decide-procedure roles), and `Comorphism`s (triples `(s, m, t)` where `s` is an ExportFormat, `m` is a Mini-TT Component, and `t` is an ImportFormat). The institution implements an `Institution` Rust trait with three methods: `extract_typed` (boundary out), `reify` (boundary in), and an optional `query` (for QueryClasses whose implementation is institution-runtime rather than a Mini-TT Component). The kernel maintains a derived registry from chain scans, runs `AutoOnLoad` QueryClasses on commit, dispatches `Decidable` QueryClasses from `Exp::NativeDecide`, dispatches `OnDemand` QueryClasses from EigenQL FIBER, and runs Comorphisms via `Exp::InstitutionInvoke`.

### 1.2 Non-goals

- This is not a plan to embed Lean 4's elaborator, tactic framework, or Mathlib into Eigenius. Lean remains Lean; Eigenius integrates with it at the proof-term boundary.
- This is not a privileged integration. Lean 4 is one verification institution among potentially many (Rocq, Isabelle/HOL, SMT checkers, domain-specific certifiers). The protocol accommodates them all on equal footing.
- This is not a replacement for the Mini-TT type system in the kernel. Mini-TT continues to check program composition; Lean 4 checks mathematical theorems.

## 2. Architectural position

Lean 4 enters Eigenius as a registered institution under D14 — committing an `Institution` resource, the supporting ExportFormat / QueryClass declarations (no ImportFormats or Comorphisms in v1; see §3.4), and an `Institution` trait implementation. The kernel does not know Lean is special. It knows only that an institution at a given IRI has declared itself, including a `ProofCheck` QueryClass with `dispatch_role: AutoOnLoad` returning `Verdict`. When a `LeanProofTerm` resource enters the chain, that QueryClass fires automatically; if the verdict is `Holds`, the resource is admitted (and tagged *verified*); if `Fails`, the Load is rejected.

### 2.1 The Option B decision

Three integration strategies were considered:

- **Option A — Accept Lean's verdict.** Lean says "checks," Eigenius trusts. Simple, but places all of Lean's kernel plus the attestation signer in the trusted computing base, and undermines the "show why it is verified" story the architecture commits to.
- **Option B — Accept Lean's proof term and re-check.** Lean exports the elaborated proof term via its existing export format. Eigenius has a Rust-native Lean term checker that re-verifies. The proof term becomes a first-class Eigon resource.
- **Option C — Translate to Mini-TT.** Lean proofs round-trip through Mini-TT. Faithful translation is a research project and fails for most of modern Lean.

**Option B is the chosen path.** It preserves the "tools that reason, kernel that validates" discipline, keeps Lean stable as an external system, and bounds the verification capability's trusted base to the Lean term checker rather than all of Lean.

In D14 vocabulary, Option B is the choice that the `ProofCheck` QueryClass's `implementation` is **institution-runtime** (a procedure dispatched to `Institution::query`) rather than a Mini-TT Component — Mini-TT's CIC fragment is too small to re-check a Lean proof. The `query` handler is where nanoda_lib lives.

### 2.2 Trusted computing base

- The Eigenius kernel: unchanged. Does not grow by a single line.
- The Lean verification institution's TCB: the Rust-native Lean term checker (`nanoda_lib`, §8.1), the EigonFFI correspondence machinery (§5), the chain-mirror translator (§6.3 + the sibling spec), and the serialization layer.
- The Lean authoring-side workflows (export, EigonFFI generation, environment instantiation) run on top of the runtime substrate's TCB. That TCB is broader than the verification side's, but the artifacts it produces (proof-term bytes, EigonFFI libraries, environment images) are themselves re-verified or content-anchored before any *verified* claim depends on them.
- Blast radius of a bug in the Lean checker or the chain-mirror translator: confined to the Lean institution's fiber. Cannot invalidate *derived* conclusions or corrupt the ontology.

### 2.3 Substrate factoring: hosted authoring vs in-process verification

The Lean integration touches the platform in two distinct places with different trust postures:

- **Authoring side** — running `lean4export` against a Lean project, generating EigonFFI libraries, instantiating `LeanEnvironment` images with the pinned Lean toolchain + Mathlib + dependencies. These are **language-toolchain workflows**: pin a runtime, run a tool, capture the output as a graph resource. Exactly what the [runtime substrate](d26-runtime-substrate.md) is for — they run as substrate-hosted dispatches via the `LanguageRuntime` trait ([crates/runtime-substrate/src/language_runtime.rs:65](../../crates/runtime-substrate/src/language_runtime.rs#L65)).
- **Verification side** — re-checking exported proof terms via nanoda_lib (the `qc_proof_check` AutoOnLoad QueryClass implementation), performing the three-part correspondence check, promoting the `LeanProofTerm` resource's epistemic status to *verified* on `Verdict::Holds`. This is **kernel-level term checking**: a small, auditable Rust crate with a tightly bounded TCB. It runs in-process inside the Eigenius orchestrator.

The factoring:

| Workflow | Substrate touchpoint | Runtime kind |
|---|---|---|
| `lean4export <project>` → `LeanProofTerm` resource | Substrate-hosted; runs through `LanguageRuntime::run_script` against a `LeanEnvironment` image | `External` |
| EigonFFI generation → `LeanPackageMirror` resource | Substrate-side `MirrorGenerator` impl wired into the image-build pipeline ([crates/runtime-substrate/src/mirror_generator.rs:78](../../crates/runtime-substrate/src/mirror_generator.rs#L78)) | `External` (authoring image) |
| `LeanEnvironment` image build | Substrate's `LanguageRuntime::build_environment_image` + `dockerfile_fragments` | `External` |
| Term checking via nanoda_lib | In-process Rust call from `eigenius-lean` | `InProcess` |
| Three-part correspondence check (§5.5) | In-process Rust call (alongside term checking) | `InProcess` |
| Chain-mirror translator (Lean Expr → `lean:LeanExpr`) | In-process Rust call from `eigenius-lean` | `InProcess` |

What this gives the integration:

- **Operational simplicity.** The authoring-side workflows are just substrate `LanguageRuntime` consumers. They get image pinning, worker pools, sandboxing, RPC framing, and provenance assembly for free. No bespoke Lean process management lives in `eigenius-lean`.
- **Authoring-side reproducibility for free.** A `LeanProofTerm` produced by the substrate-hosted `lean_export` entry point carries the same `RuntimeInvocation` provenance every substrate dispatch carries — image digest, environment IRI, input IRIs, dispatched-to method.
- **Verification-side trust posture preserved.** The in-process checker is unchanged. It receives a `LeanProofTerm` resource (whose bytes the substrate already vouched for via its own provenance chain) and re-checks. The trust surface for the *verdict* is nanoda_lib + the correspondence logic + the chain-mirror translator.
- **Closed audit chain.** An auditor verifying a *verified* `LeanProofTerm` walks: claim reference → `LeanProofTerm` resource → substrate `RuntimeInvocation` (proves the proof bytes came from `lean4export` on a pinned image against a pinned Lean project) → nanoda_lib re-check (proves the term type-checks) → correspondence check (proves the proposition matches the claim). Every step is a graph-internal computation; nothing external to Eigenius is consulted at audit time.

## 3. Declared surface — Lean-specific resource classes and QueryClasses

Under D14, an institution exposes itself by committing typed Resources to the layer chain. The Lean institution commits the following.

### 3.1 Resource classes

These are ordinary Eigon classes living in the Lean institution's ontology layer. They are *sentences* in the institution's logic — typed claims, not models.

**Anchor classes:**

- **`LeanProofTerm`** — the central class. Carries the verbatim Lean export bytes (the proof term + its transitive declaration closure), the target Theorem's name, the chain-mirrored proposition (a `lean:LeanExpr` value — see below), the environment reference, the EigonFFI library reference, the Eigenius-side claim it vouches for, and the claim layer hash. A `LeanProofTerm` resource entering the chain triggers `ProofCheck` (§3.3) automatically. On `Holds`, the resource's epistemic status promotes from *derived* to *verified*.
- **`LeanEnvironment`** — pinned environment as a `RuntimeEnvironment` subclass (§7).
- **`LeanProject`** — a `RuntimePackage` subclass: the Lean source tree the user authored against, used by the substrate-hosted authoring side (§6.2).
- **`LeanPackageMirror`** — a `RuntimePackageMirror` subclass: the EigonFFI library (tracked artifact mirroring Eigon classes as Lean types, §5).

**Chain-mirrored Lean expression language** (sibling spec — D40 forward reference):

- **`lean:LeanExpr`** — a chain-resident InductiveType mirroring nanoda's `Expr` shape (`Var`, `Sort`, `Const`, `App`, `Pi`, `Lambda`, `Let`, `Proj`, `StringLit`, `NatLit` — 10 ctors; we omit `Local` because committed proofs are closed terms). This is the queryable form for propositions. The `proposition` field on `LeanProofTerm` is a value of this inductive.
- **`lean:LeanLevel`** — a chain-resident InductiveType mirroring `Lean.Level` (`Zero`, `Succ`, `Max`, `IMax`, `Param`).
- **`lean:LeanName`** — a chain-resident shape for Lean's dotted-name hierarchy (`Anon`, `Str`, `Num`).

These follow the D32 pattern (chain-mirrored Mini-TT inductives) applied to Lean. The full faithful-translation spec — how nanoda's `Expr` decodes into chain-CBOR values of this inductive and back — lives in a sibling document (§10.4).

**Intra-fibre relation classes:**

- **`DependencyOn`** — relates a `LeanProofTerm` to an axiom or previously-verified lemma it depends on. Discovered by an OnDemand QueryClass (§3.4); not auto-fired.
- **`ReducesTo`** — relates two Lean terms by definitional equality. Discovered by an OnDemand QueryClass; intra-fibre structure that programs may query.

**Typed-payload + typed-result classes** (used by the QueryClasses below):

- `LeanProofPayload` — the `(verbatim_bytes, target_name, proposition, env_iri, mirror_iri)` tuple shape `ef_lean_proof_payload` extracts.
- `LeanEnvironmentSummary`, `LeanAxiomList`, `LeanProofMetrics`, `LeanEnvironmentDiff`, `LeanEnvironmentDiffInput`, `LeanProposition`, `LeanDependencySet`, `LeanReductionSet` — typed result/input classes for the OnDemand QueryClasses.
- `Verdict` — kernel-defined (D14 core); not a Lean-only class.

### 3.2 `ExportFormat` declarations

| ExportFormat | `from_class` | `payload_type` | `procedure` | Used by |
|---|---|---|---|---|
| `ef_lean_proof_payload` | `LeanProofTerm` | `LeanProofPayload` | `urn:eigenius:lean:extract_proof_payload` | `ProofCheck` AutoOnLoad QueryClass (§3.3); the `WhichAxioms` / `ProofSize` OnDemand queries |
| `ef_lean_environment_summary` | `LeanEnvironment` | `LeanEnvironmentSummary` | `urn:eigenius:lean:extract_env_summary` | `EnvironmentDiff` OnDemand query |

The `LeanProofPayload` payload type bundles:

- `proof_term_bytes: bytes` — verbatim Lean export-file content (CBOR-wrapped from nanoda's expected JSON shape; see §6).
- `target_declaration: string` — the Lean name of the Theorem to verify (the lookup key into the bytes' declaration list).
- `proposition: lean:LeanExpr` — the **chain-mirrored** proposition (the Theorem's type, decoded from the bytes into chain-typed form by the institution's translator).
- `environment_iri: Iri` — IRI of the `LeanEnvironment` the term was elaborated against.
- `mirror_iri: Iri` — IRI of the `LeanPackageMirror` (EigonFFI library) the proof was authored against.

Splitting `proof_term_bytes` (verbatim, opaque) from `proposition` (structured, queryable) is load-bearing: nanoda needs the bytes for re-checking (the export format is the standardized protocol; translating the proof side would widen the TCB), while the chain wants the proposition as a structured value so cross-institution Comorphisms and EigenQL queries can refer to proposition structure.

### 3.3 `QueryClass` declarations

| QueryClass | `query_class` | `result_class` | `dispatch_role` | `implementation` | Notes |
|---|---|---|---|---|---|
| `qc_proof_check` | `LeanProofTerm` | `Verdict` | `AutoOnLoad`, `OnDemand` | institution-runtime — `urn:eigenius:lean:proof_check` | The load-bearing one. Implementation calls nanoda_lib in-process; performs the three-part correspondence check (§5.5). AutoOnLoad fires on commit; OnDemand permits "what if I submitted this?" probes via FIBER. |
| `qc_which_axioms` | `LeanProofTerm` | `LeanAxiomList` | `OnDemand` | institution-runtime — `urn:eigenius:lean:which_axioms` | Walks the proof term, extracts non-reducible axiom references. Diagnostic query for compliance / audit. |
| `qc_proof_size` | `LeanProofTerm` | `LeanProofMetrics` | `OnDemand` | institution-runtime — `urn:eigenius:lean:proof_size` | Term-size metrics. |
| `qc_environment_diff` | `LeanEnvironmentDiffInput` | `LeanEnvironmentDiff` | `OnDemand` | institution-runtime — `urn:eigenius:lean:env_diff` | Coarse equal/not-equal verdict in v1; structured diff later. |
| `qc_discover_dependencies` | `LeanProofTerm` | `LeanDependencySet` | `OnDemand` | institution-runtime — `urn:eigenius:lean:discover_dependencies` | Returns `DependencyOn` candidate resources. Default depth = 1 (direct deps); configurable per-query. |
| `qc_discover_reductions` | `LeanProofTerm` | `LeanReductionSet` | `OnDemand` | institution-runtime — `urn:eigenius:lean:discover_reductions` | Returns `ReducesTo` candidate resources. |

`qc_proof_check` carries both `AutoOnLoad` and `OnDemand` roles in the same declaration — D14 permits this. The kernel dispatches the same procedure either way; the role just selects the trigger (Load commit vs. explicit FIBER).

D10's `discover_morphisms` method is folded into the `qc_discover_*` OnDemand QueryClasses per D14 §6 row 4 — discovery returns candidates; the calling program commits those it wants.

### 3.4 `ImportFormat` and `Comorphism` declarations

The Lean institution declares **no ImportFormats and no Comorphisms in v1**. Lean is a *verification* institution: it consumes proof terms and produces verdicts. It does not act as the *target* of any cross-institution comorphism in v1, because turning some other institution's typed payload into a Lean proof requires synthesising a proof — beyond v1 scope.

The natural future bridge — verification of `IntervalArithmetic` bounds via Lean — is sketched in [D27 §6.2](d27-julia-institutions.md#62-verification-of-intervalarithmetic-outputs--concrete-d14-comorphism). Realising it requires:

1. A Lean ImportFormat constructing a `LeanProofTerm` from a `(FormulaTerm, IntervalRepr, IntervalRepr)` triple. The constructor packages the interval data into a Lean Prop asserting the bound (a `lean:LeanExpr` value); the proof itself is still supplied externally.
2. A Comorphism `(ef_intv_proves_bound_on, m_pack_bound_obligation, if_lean_bound_proof)` whose middle Component packages the source-side `FormulaTerm` into the Lean Prop using EigonFFI's mirror of `formulas:FormulaTerm` as a Lean inductive.

Out of v1 scope but structurally clean once EigonFFI mirrors `formulas:FormulaTerm`.

### 3.5 Structural properties (advisory, not kernel-enforced)

- `LeanProofTerm` is functional modulo definitional equality: a given term proves exactly one proposition up to the environment's conversion rules. Enforced by `qc_proof_check`, not by the kernel.
- `DependencyOn` is transitive; the OnDemand `qc_discover_dependencies` reports up to a configurable depth.
- `ReducesTo` is confluent within Lean's reduction system; query results are unique up to canonicalisation.

These are advisory metadata about institution-internal relations; they have no dedicated declaration mechanism.

## 4. Institution trait realisation

Per [D14 §8](d14-institution-realisation.md), the Lean institution implements the three-method `Institution` trait: `extract_typed`, `reify`, and `query`.

### 4.1 `extract_typed`

Dispatched by the kernel when an ExportFormat (§3.2) is invoked.

For `ef_lean_proof_payload`: read the `LeanProofTerm` resource's properties; the `proposition` field is already a chain-mirrored `lean:LeanExpr` value (computed at commit time, §6.3); pack the tuple as a `LeanProofPayload`. CBOR-encode for `typed-value` transport.

For `ef_lean_environment_summary`: read the `LeanEnvironment` resource; pack the summary record (Lean version + library list + image digest + axiom allowlist).

### 4.2 `reify`

In v1 the Lean institution declares no ImportFormats (§3.4), so `reify` is unreachable. The trait method must still exist; the implementation returns `InstitutionError::NotImplemented` for any procedure IRI it receives. When the future Lean-target Comorphism (§3.4) lands, this method gains content.

### 4.3 `query`

The work centre. Dispatched by the kernel for every QueryClass whose `implementation` is institution-runtime — i.e. all of §3.3 — regardless of whether the trigger was AutoOnLoad, Decidable, or OnDemand.

Procedure dispatch:

- **`urn:eigenius:lean:proof_check`** — the load-bearing handler. The input resource is a `LeanProofTerm`; the body performs the three-part correspondence check (§5.5), runs nanoda_lib on the verbatim proof bytes, and returns a `Verdict` resource (`Holds` / `Fails`). `Undecidable` is unused — proof checking under nanoda_lib's regime is binary.
- **`urn:eigenius:lean:which_axioms`** — walks the proof term, returns a `LeanAxiomList` resource enumerating the axioms reached. Uses nanoda's parsed environment.
- **`urn:eigenius:lean:proof_size`** — returns a `LeanProofMetrics` resource with term-level counters.
- **`urn:eigenius:lean:env_diff`** — accepts a `LeanEnvironmentDiffInput`, returns a `LeanEnvironmentDiff` resource.
- **`urn:eigenius:lean:discover_dependencies`** — accepts a `LeanProofTerm`, returns a `LeanDependencySet` of candidate `DependencyOn` resources.
- **`urn:eigenius:lean:discover_reductions`** — accepts a `LeanProofTerm`, returns a `LeanReductionSet` of candidate `ReducesTo` resources.

The procedure dispatch is a simple match on `procedure_iri` inside the trait method.

## 5. The type correspondence problem

A proof term in Lean proves a proposition stated in Lean's type theory. A knowledge-graph claim in Eigenius is stated in Eigon. These are two different languages; something must establish the correspondence. This is the hardest piece of the design and the reason the *verified* epistemic category is genuinely stronger than the *derived* one.

The correspondence is established by a Lean library called **EigonFFI**, which mirrors the ontology's structure as Lean types. Users author proofs against the mirror; the verification institution checks both that the proof is valid Lean and that the mirror faithfully represents the Eigenius-side claim.

### 5.1 EigonFFI as a generated, tracked static mirror

EigonFFI is not a runtime API. It does not query the knowledge graph during proof checking. It is a *generated static library* that mirrors ontology structure as Lean code, produced at a specific ontology state and committed back to Eigenius as a tracked resource.

For each Eigon class a user might prove things about, EigonFFI provides a Lean structure with the same required fields, subclass relationships expressed as coercion instances, and ontology invariants encoded as theorems or axioms. A proof about the safety factor of a `StressResult` resource is, in Lean, a proof about an `EigonFFI.StressResult` value with the corresponding structural shape.

The generated library is an Eigon resource of class `LeanPackageMirror` (D26 §5's `RuntimePackageMirror` subclass), committed to the knowledge graph with its own content hash, its own declared provenance (source layer hash, generator identity, generator version + content hash), and optionally the full Lean source embedded as content.

A diagnostic test for future design decisions: *could this cause the same proof to check differently at different times?* If yes, the functionality does not belong in EigonFFI. If it is valuable, it belongs elsewhere — in EigenQL, in a component, in proof-authoring tooling. EigonFFI is the place where things are stable by construction; that stability is its whole value.

### 5.2 The anchor chain

A `LeanPackageMirror` is bound to a specific ontology state by three pieces of metadata that together form its *anchor*:

- **`source_layer`** — content-addressed SHA-256 of the CBOR-encoded layer stack at the moment the library was generated. Identifies the complete ontology snapshot the mirror was produced from.
- **`generator_identifier`** + **`generator_version`** + **`generator_content_hash`** — identity of the substrate-side Rust code that produced the mirror.
- **`library_content_hash`** — content hash of the generated library itself.

If the generator is deterministic — a design requirement — then `library_content_hash` is a function of `(source_layer, generator_identifier, generator_version, generator_content_hash, mirrored_classes)`. The content hash is kept in the anchor as belt-and-suspenders.

### 5.3 The generated library as a tracked resource

When the substrate's image-build pipeline runs the Lean `MirrorGenerator` implementation, its output is both a Lean source tree baked into the `LeanEnvironment` image and a corresponding `LeanPackageMirror` resource committed to Eigenius. The resource carries (inherited from `RuntimePackageMirror`):

- `source_layer`, `generator_identifier`, `generator_version`, `generator_content_hash`, `library_content_hash`, `library_content` (embedded or external reference), `generated_at`, `mirrored_classes`.

The library's IRI is content-addressed from its properties; two independent generations from the same layer with the same generator produce the same IRI. Re-committing an identical library is a no-op.

This is what makes **independent provenance verification** possible. An auditor with access to the substrate source + the layer chain can re-derive byte-identical mirror source. Auditability comes from the *deterministic output spec* (D30 — forward reference), not from the generator being a separately-runnable binary.

### 5.4 The generator and its requirements

The Lean mirror generator is **substrate Rust code** implementing the `MirrorGenerator` trait ([crates/runtime-substrate/src/mirror_generator.rs:78](../../crates/runtime-substrate/src/mirror_generator.rs#L78)) — same pattern as Julia's [crates/eigenius-julia/src/mirror_gen.rs](../../crates/eigenius-julia/src/mirror_gen.rs). Earlier drafts of this design framed `eigon-ffi-gen` as a separate CLI binary; that framing is misleading. The substrate's image-build pipeline runs the generator as a step of building each `LeanEnvironment` image. There is no v1 use case for invoking the generator outside the pipeline.

Two requirements remain load-bearing:

- **Determinism.** Given the same `source_layer` and the same `seed_classes`, the generator must produce byte-identical output. The closure walker — `MirrorGenerator::generate` walks structural references through `arg_types`, `class_types`, and `allows_only` — is the same shape Julia uses.
- **Faithful translation.** The generated Lean types must structurally correspond to the Eigon classes they mirror. Required properties become required fields; primitive types map by declared correspondence; subclass relationships become coercion instances; format and constraint declarations become refinement conditions where Lean-expressible.

The faithful-translation specification is D30 — a sibling to D29, structurally identical in role but Lean-flavoured. Authoring it is a design-work item on the path to landing this institution.

### 5.5 The correspondence check

The correspondence check is the body of `qc_proof_check`'s institution-runtime implementation (§4.3). When the kernel fires the AutoOnLoad QueryClass — or when an OnDemand caller invokes it via FIBER — the institution's `query` handler performs three checks, in order. The first is ordinary Lean verification; the second and third are what make the correspondence sound.

1. **Proof validity.** nanoda_lib parses the verbatim proof bytes, locates the Theorem named by `target_declaration`, and verifies that the proof term type-checks against its stated proposition under the declared environment.
2. **Mirror correspondence.** The institution resolves the `mirror_iri` to a committed `LeanPackageMirror`, reads its declared anchor, and checks that the library was anchored to a layer ancestral to or identical with the layer in which the claim's class is defined. The mirror type referenced in the proposition (the chain-mirrored `lean:LeanExpr` value already in the `LeanProofTerm`) must correspond structurally to that class.
3. **Anchor consistency.** The mirror's declared content hash is verified against its actual content; the layer hash it declares is confirmed to resolve in the current layer chain.

The QueryClass returns `Verdict::Holds` if all three pass, `Verdict::Fails` (with diagnostic detail) on any failure, and never returns `Undecidable`.

Check 2 is the load-bearing one for soundness. It is what prevents a proof about an `EigonFFI.StressResult` (anchored to an old ontology state) from being accepted as verification for a resource whose class has since acquired new required properties.

### 5.6 Compositionality under layer extension

An important property of the anchor design: a `LeanPackageMirror` does *not* need to be regenerated every time the ontology evolves. A library anchored to layer L₀ remains valid for claims in any descendant layer L₁ ⊒ L₀, provided the classes the library mirrors are unchanged in L₁.

This is the common case. A user generates EigonFFI from a layer containing Core plus FEA. They later commit their bracket analysis in a descendant layer; that layer adds domain data but does not modify the FEA class definitions. Their proof, authored against the L₀ mirror of `StressResult`, remains valid for verifying claims about stress results in the later layer.

The failure mode is symmetric. If a later layer L₂ modifies `StressResult` — adds a new required property, say — proofs anchored to L₀ fail the correspondence check when submitted against L₂ claims. The rejection surfaces as `qc_proof_check` returning `Verdict::Fails` with diagnostic field `FFIVersionMismatch`. The user regenerates EigonFFI from L₂ (or a descendant), re-states their proof against the updated mirror, and re-commits.

### 5.7 The closed chain from ontology to archived verdict

Putting the pieces together:

1. Ontology content is committed to Eigenius, producing layer hash L.
2. The substrate's image-build pipeline runs the Lean `MirrorGenerator` against L. Its output is both a Lean source tree baked into the `LeanEnvironment` image and a committed `LeanPackageMirror` resource carrying the library's content hash, the source-layer reference, and the generator's content hash.
3. The Lean source is imported into a `LeanProject`; a proof is authored against its mirror types.
4. The proof elaborates against a specific `LeanEnvironment` E (image-digest-pinned, §7).
5. The substrate-hosted `lean_export` entry point produces a `LeanProofTerm` resource carrying the exported term bytes, the target declaration name, the chain-mirrored proposition (decoded from the bytes by the institution's translator at commit time, §6.3), and references to E + the mirror + the Eigenius claim.
6. The kernel commits the `LeanProofTerm`. Structural validation passes.
7. The kernel's D14 dispatch fires `qc_proof_check` AutoOnLoad, calling `Institution::query` with `urn:eigenius:lean:proof_check`. The handler runs the three-part correspondence check (§5.5).
8. On `Verdict::Holds`, the kernel admits the resource and tags its epistemic status *verified*. On `Verdict::Fails`, the Load is aborted with the verdict's diagnostic.

Every artifact is content-addressed and tracked as a resource within Eigenius. The verification verdict is a pure function of archived inputs.

### 5.8 TCB implications

The Lean verification institution's TCB:

- **nanoda_lib** — Rust-native Lean term checker. Invoked from inside `qc_proof_check`'s handler.
- **The mirror generator** — substrate Rust code, deterministic, with a separately-specified faithful-translation contract (D30). Image-digest-pinned via the substrate.
- **The correspondence logic** in the `qc_proof_check` handler — code that matches mirror types to Eigon classes and verifies anchor consistency.
- **The chain-mirror translator** — code that decodes nanoda's parsed `Expr` into chain-CBOR `lean:LeanExpr` values and back (D40).

The kernel's TCB does not grow. The blast radius of a bug in any of the above is confined to the verification institution's fiber.

## 6. Proof term and proposition transport

### 6.1 Lean export format

Lean exports elaborated terms via [`lean4export`](https://github.com/leanprover/lean4export) — the official tool maintained by the leanprover organization. Output is JSON-based (semver 3.1.x at time of writing — nanoda's `check_semver` accepts `>=3.1.0, <3.2.0`). The format is consumed directly by nanoda_lib.

Structure of an export file:

- A name table.
- A universe-level table.
- An expression DAG with back-references (`In(idx)` / `Il(idx)` / `Ie(idx)` for name / level / expression references).
- A declaration list — each entry is `Axiom { info }` | `Theorem { info, val }` | `Definition { info, val, hint }` | `Inductive { ... }`.

For a proof of theorem `T`, the export file contains `T` as a `Theorem { info, val }` plus the transitive closure of every declaration `T` depends on. `lean4export` does the closure walk; nanoda parses the result.

### 6.2 `lean4export` as a substrate authoring entry point

The `lean4export` invocation is **not** a bespoke pipeline inside `eigenius-lean`. It runs as a substrate-hosted dispatch via `LanguageRuntime::run_script` (or `call_method` against a declared `lean_export` `RuntimeMethodSignature`) targeting a `LeanEnvironment` image. The flow:

1. The caller resolves a `LeanProject` resource (a `RuntimePackage` subclass carrying `lakefile.lean`, source tree, environment dependency).
2. Substrate dispatch into a worker container backed by the `LeanEnvironment`'s image digest.
3. The worker runs `lean4export` against the project.
4. The export bytes return as the `proof_term_bytes` field of a `LeanProofTerm` resource (after the institution's commit-time translator runs to populate `proposition`).
5. Substrate assembles a `RuntimeInvocation` provenance record (image digest, project IRI, environment IRI, runtime metadata).

The same machinery that hosts Julia computations hosts Lean's authoring side. The Lean mirror generator follows the same pattern: it runs as part of the image-build pipeline, baked into the `lean-tools` `LeanEnvironment` image, producing a `LeanPackageMirror` resource as a side effect of the build.

### 6.3 The `LeanProofTerm` resource — verbatim bytes + chain-mirrored proposition

The resource carries:

- **`proof_term_bytes: bytes`** — verbatim Lean export-file content (CBOR-wrapped). The source of truth for re-checking. nanoda parses these directly. Embedded for small proofs; externally referenced (content-addressed) for large ones — same pattern as `LeanPackageMirror.library_content`.
- **`target_declaration: string`** — the Lean name of the target Theorem (the lookup key into the bytes' declaration list).
- **`target_hash: H256`** — SHA-256 of the Theorem's serialized `(info, val)` pair within the bytes; lets the institution verify post-decode that `target_declaration` resolves to the same content the proposition was extracted from.
- **`proposition: lean:LeanExpr`** — the chain-mirrored Theorem type (queryable form). Populated at commit time by the institution's `query` handler running the chain-mirror translator on the bytes; idempotent given the bytes + `target_declaration`.
- **`environment_iri: Iri`** — IRI of the `LeanEnvironment` the term was elaborated against (§7).
- **`mirror_iri: Iri`** — IRI of the `LeanPackageMirror` (EigonFFI library) used.
- **`eigenius_claim_iri: Iri`** — IRI of the Eigon-side resource this proof vouches for.
- **`claim_layer_hash: LayerId`** — layer hash at which the claim exists; determines which ontology state the correspondence check resolves against.

The split between `proof_term_bytes` (opaque, verbatim) and `proposition` (structured, queryable) is load-bearing:

- **Proofs stay as verbatim bytes — always.** lean4export's format is the standardized interface between Lean and external checkers; translating the proof side widens the TCB to include a translator, breaks against lean4export semver bumps, and pollutes the chain (Mathlib-scale proofs have thousands of shared subterms via the export format's back-reference DAG — Eigon resources are trees, so chain-mirroring whole proofs would require either intra-resource sharing (which doesn't exist) or thousands of top-level resources per proof).
- **Propositions are chain-mirrored.** The proposition is a single Expr tree, small enough that the sharing problem doesn't bite; it's the natural target of cross-institution Comorphisms (§3.4) and audit queries; making it structured lets EigenQL queries refer to proposition shape without parsing bytes.

All references resolve within the knowledge graph itself. A committed `LeanProofTerm` is a closed object: given its properties, every artifact the verdict depends on is retrievable from Eigenius alone.

### 6.4 Content addressing and cache

The `LeanProofTerm`'s content hash (a function of all the fields above) is the cache key for verification results. A re-commit of an identical resource is a no-op; the cached verdict is served. A `LeanProofTerm` whose `proof_term_bytes` is unchanged but whose `claim_layer_hash` differs is a different resource — the AutoOnLoad QueryClass re-runs because the correspondence check resolves against a different ontology state.

nanoda's `Expr` internally carries structural hashes ([`references/nanoda_lib/src/expr.rs:8-18`](../../references/nanoda_lib/src/expr.rs#L8-L18)) — two byte-identical export files type-check to identical Expr DAGs. The chain's content-addressed `@id` composes cleanly with this: identical proofs always land at the same IRI.

## 7. Environment management

Mathlib is large. A naive "load the environment per request" design is non-viable. The substrate solves the language-toolchain pinning problem; Lean reuses its solution.

### 7.1 `LeanEnvironment` extends `RuntimeEnvironment`

A `LeanEnvironment` is a typed Eigon resource that subclasses the substrate's [`RuntimeEnvironment`](d26-runtime-substrate.md). It is immutable, content-addressed, and image-digest-anchored on the same terms as Julia's `JuliaEnvironment`.

| Property | Inherited / new | Purpose |
|---|---|---|
| `language` | inherited | Always `"lean"`. |
| `runtime_version` | inherited | Exact Lean version (`"4.10.0"`). |
| `manifest` | inherited | The Lake manifest (`lake-manifest.json`) — verbatim bytes. |
| `pinned_packages` | inherited | List of `LeanPackagePin` IRIs (subclasses of `RuntimePackagePin`). |
| `included_packages` | inherited | List of `LeanPackage` IRIs — user-authored Lean libraries baked into the image (e.g. EigonFFI mirror used as a Lake dependency). |
| `mirror_dependency` | inherited | Optional IRI of a `LeanPackageMirror` baked into the image. |
| `image_digest` | inherited | OCI image digest pinning Lean toolchain + Mathlib + dependencies + EigonFFI mirror. |
| `image_reference` | inherited | Optional registry tag (`registry.eigenius.io/runtime/lean-mathlib:4.10`). |
| `lake_lockfile_hash` | new | Hash of the Lake lockfile, separate from `manifest`. |
| `lean_permitted_axioms` | new | `value_array<string>` — axioms nanoda is configured to accept. Default: `["propext", "Classical.choice", "Quot.sound", "Lean.trustCompiler"]`. |
| `lean_unpermitted_axiom_hard_error` | new | `boolean` — controls nanoda's hard-error-vs-skip behaviour for unlisted axioms. Default `true`. |

The axiom allowlist lives on the environment rather than per-`LeanProofTerm` because nanoda's configuration is environment-scoped at parse time; tying the allowlist to the environment also means re-pinning the allowlist requires a new environment commit (new image digest, new content-addressed IRI), which is the right operational discipline.

Consequences:

- "Load the environment" reduces to "pull the image"; the substrate's worker pool already caches pulled images.
- "Multiple registrations share an environment" reduces to "multiple registrations reference the same image digest"; substrate-level deduplication is automatic.
- "Change to the environment produces a new resource" reduces to "new image digest produces a new content-addressed `LeanEnvironment` IRI."

### 7.2 Pinning and sharing

Institution registrations pin the `LeanEnvironment` IRI they expect. Prior `LeanProofTerm` resources tagged *verified* remain valid against their original environment IRIs even after newer environments are registered; the substrate doesn't implicitly upgrade and neither does the verification institution.

### 7.3 Caching

The substrate's worker pool handles environment caching for the *authoring* side: warm Lean toolchain workers per environment digest, LRU eviction, image-pull cache. The verification side holds nanoda's parsed environment representation in an in-process LRU cache indexed by `LeanEnvironment` IRI; first checking request loads from the resource bytes (or from a baked artifact in the image, fast path), subsequent requests are served from cache.

### 7.4 Environment diffing

Two `LeanEnvironment` resources can differ in library versions, axiom sets, or definitional transparencies. The coarse version — "these environments are different, proofs against one are not transferable to the other" — is the image-digest comparison; the structured version (`qc_environment_diff`) is non-trivial and lands as a targeted enhancement when needed.

## 8. Implementation approach

### 8.1 In-process Rust checker via nanoda_lib

The Lean term **checker** runs as a Rust library linked into the `eigenius-lean` crate. Proof terms produced by `lean4export` (which runs *separately*, on the substrate-hosted authoring side per §2.3 and §6.2) are parsed and checked without leaving the orchestrator process. The specific library is **nanoda_lib** ([github.com/ammkrn/nanoda_lib](https://github.com/ammkrn/nanoda_lib)), vendored in-tree at [`references/nanoda_lib/`](../../references/nanoda_lib/) and accompanied by the specification book [Type Checking in Lean 4](https://ammkrn.github.io/type_checking_in_lean4/).

This is preferred because:

- **Latency and integration on the verification path.** No cross-process IPC, no serialization of proof terms over a service boundary, no Lean toolchain on the verification path. The verification institution is a function call away from the evaluator.
- **Trust surface is auditable Rust.** Verification-side code is available, readable, and sized for independent review. No Lean source-level machinery (elaborator, tactic framework, metaprogramming) is in the verification TCB.
- **Pre-existing axiom governance.** nanoda implements axiom allow-listing via configuration. The allowlist lives on `LeanEnvironment` (§7.1).
- **Library-shaped.** nanoda is designed to be embedded as a Cargo dependency, matching the requirement that the checker be dispatchable from inside the `Institution::query` handler without spawning a process.

**Cross-checking as a soundness multiplier** — a future enhancement. Once a working integration exists, introducing [Lean4Lean](https://github.com/digama0/lean4lean) as a *secondary* verifier (the Venn-diagram argument from Type Checking in Lean 4) provides "the part where independent checker circles intersect is stronger." Out of scope for the first landing; recommended for operational hardening.

### 8.2 WASM-sandboxed checker (future option)

The Rust checker compiled to WASM and run under the platform's existing untrusted-capability sandbox. Performance overhead on Mathlib-scale proofs is likely meaningful; the engineering to make nanoda run cleanly under WASM is non-trivial. Future option once benchmarks justify it.

## 9. Verdict shape, error taxonomy, and runtime properties

### 9.1 Verdict diagnostics

The `qc_proof_check` QueryClass returns a `Verdict` resource. On `Fails`, the verdict's diagnostic field carries one of:

- **`ProofDoesNotCheck`** — nanoda rejected the proof. Diagnostic payload includes nanoda's error message.
- **`TargetDeclarationNotFound`** — `target_declaration` doesn't resolve to a Theorem in `proof_term_bytes`.
- **`TargetHashMismatch`** — the Theorem's serialized `(info, val)` hashes don't match `target_hash`. Indicates corruption or tampering between extraction and commit.
- **`PropositionDecodingFailed`** — the chain-mirrored `proposition` doesn't agree with the Theorem's type in `proof_term_bytes`. Indicates a bug in the chain-mirror translator.
- **`PropositionMismatch`** — proof checks but its proposition does not correspond to the Eigenius-side claim. Diagnostic includes the expected and actual `lean:LeanExpr` shapes.
- **`EnvironmentUnavailable`** — the referenced `LeanEnvironment` resource is not loadable.
- **`EnvironmentMismatch`** — the proof was elaborated against a different environment than declared.
- **`FFIVersionMismatch`** — the `LeanPackageMirror` was anchored to a layer that doesn't cover the claim's class definition.
- **`UnpermittedAxiom`** — the proof depends on an axiom not in the `LeanEnvironment`'s allowlist.

These are diagnostic field values, not separate result classes. The kernel's response to `Verdict::Fails` is uniform regardless of which diagnostic fired: the Load is rejected, the diagnostic surfaces to the caller, the resource never enters the chain.

### 9.2 Runtime properties (advisory)

- **Determinism.** Given the same `LeanProofTerm` resource (same bytes + target + proposition + env IRI + mirror IRI + claim layer hash), the verdict is constant.
- **Idempotence.** Re-firing the AutoOnLoad QueryClass on the same resource yields the same verdict.
- **Effects.** The handler is read-only against the chain; it does not commit resources.
- **Resource bounds.** Per-proof `max_wall_time_ms` and `max_memory_bytes`. Mathlib-scale proofs can be large; bound violations surface as `Verdict::Fails` with a `ResourceLimitExceeded` diagnostic.

### 9.3 Lifecycle and versioning

The institution registers by committing its `Institution`, ExportFormat, and QueryClass resources to a layer. Upgrades produce new resources at new content-addressed IRIs in later layers; prior registrations remain valid for their trace history.

## 10. Integration touchpoints in the existing code

### 10.1 Kernel-side changes

- **`InProcess` runtime kind dispatch** — the variant is defined ([kernel/src/institution/registry.rs:46](../../kernel/src/institution/registry.rs#L46)) and parsed from chain resources, but no registration path populates the `InstitutionRuntime` from in-process Rust crates. The first landing wires:
  - A statically-registered in-process institution registry (sibling to `InstitutionRuntime`, populated at orchestrator startup by `eigenius-lean::register(...)`).
  - A third branch in `build_institution_runtime` (alongside Wasm and External) for each `Institution` resource with `runtime: in_process`: look up the IRI in the in-process registry, register it into `InstitutionRuntime`, surface a clean error if missing.
- The kernel's D14 institution registry picks up the Lean institution's declarations from chain scan automatically. No Lean-specific kernel code beyond the dispatch wiring above.

### 10.2 Orchestrator-side additions

Three new crates:

- **`eigenius-lean`** (verification side). Houses:
  - The nanoda_lib dependency (vendored at [`references/nanoda_lib/`](../../references/nanoda_lib/)).
  - The chain-mirror translator (Lean `Expr` ↔ chain-mirrored `lean:LeanExpr`).
  - The EigonFFI correspondence logic.
  - The in-process environment cache (parsed nanoda `Env` LRU).
  - The D14 `Institution` trait implementation (`LeanInstitution`).
  - The `register(&mut InProcessRegistry)` startup hook called from the orchestrator's `main.rs`.
- **`eigenius-lean-runtime`** (authoring side). Houses:
  - The `LanguageRuntime` trait implementation (`LeanLanguageRuntime`).
  - Dockerfile fragments installing `elan` + the pinned Lean toolchain + Lake + Mathlib (matching D26 §9.2).
  - The worker bootstrap (`LeanWorker.jl`-equivalent — a thin Lake-driven binary handling CBOR-framed RPC for `lean_export` and method-call dispatch).
  - The `MirrorGenerator` trait implementation (`LeanMirrorGenerator`) — substrate Rust code that emits Lean source for the seed-class closure.
  - The `LeanProject` / `LeanPackage` / `LeanPackagePin` resource subclass declarations.
- **`eigenius-lean-chain-mirror`** (optional split — could live inside `eigenius-lean`). Houses the `lean:LeanExpr` / `lean:LeanLevel` / `lean:LeanName` ontology + the deterministic encoder/decoder. The institution's `query` handler invokes it at commit time to populate the `proposition` field.

Registration with the kernel's `InstitutionRuntime` (verification side) and with the substrate's `LanguageRuntime` registry (authoring side) lives in the orchestrator's startup code.

### 10.3 Ontology additions

The Lean institution's declarations land in `ontologies/lean/lean-institution.json` (D14 resource shapes):

**`Institution` resource**: one — `urn:eigenius:institutions:lean`, runtime `InProcess`.

**Eigon classes**:
- Anchor: `LeanProofTerm`, `LeanEnvironment` (subclass of `RuntimeEnvironment`), `LeanProject` (subclass of `RuntimePackage`), `LeanPackage` (subclass of `RuntimePackage`), `LeanPackagePin` (subclass of `RuntimePackagePin`), `LeanPackageMirror` (subclass of `RuntimePackageMirror`).
- Chain-mirrored Lean term language: `lean:LeanExpr` (InductiveType, 10 ctors), `lean:LeanLevel` (InductiveType, 5 ctors), `lean:LeanName` (InductiveType, 3 ctors).
- Intra-fibre relations: `DependencyOn`, `ReducesTo`.
- Typed payload / result classes: `LeanProofPayload`, `LeanEnvironmentSummary`, `LeanAxiomList`, `LeanProofMetrics`, `LeanEnvironmentDiff`, `LeanEnvironmentDiffInput`, `LeanProposition`, `LeanDependencySet`, `LeanReductionSet`.

**ExportFormat resources** (§3.2): `ef_lean_proof_payload`, `ef_lean_environment_summary`.

**ImportFormat resources**: none in v1 (§3.4).

**QueryClass resources** (§3.3): `qc_proof_check`, `qc_which_axioms`, `qc_proof_size`, `qc_environment_diff`, `qc_discover_dependencies`, `qc_discover_reductions`.

**Comorphism resources**: none in v1 (§3.4).

### 10.4 Sibling specification documents

Two design documents land alongside this one as part of the first landing's scope:

- **D30 — Eigon → Lean Faithful Translation.** The mirror specification for the Lean EigonFFI generator. Sibling to [D29](d29-eigon-julia-mirror-spec.md); structurally identical role (deterministic translation spec, conformance levels, integrity chain) but Lean-flavoured (refinement predicates, coercion instances, axiom encodings).
- **D40 — Chain-Mirrored Lean Expressions.** The `lean:LeanExpr` / `lean:LeanLevel` / `lean:LeanName` chain inductive specification. Sibling to [D32](d32-chain-mirrored-mini-tt-inductives.md); pins the encoder/decoder semantics, the determinism contract, the migration path, and the version-discipline against `lean4export` semver bumps.

D30 and D40 are scoped tightly enough to author in parallel with the first landing's implementation work; neither is research-sized.

## 11. Landing plan

D28 was originally drafted as a five-phase plan during a period when the runtime substrate was unfinished and the architectural shape uncertain. With D26 shipped, D27/D29 landed end-to-end, and nanoda_lib vendored, most of the phasing is no longer load-bearing — its purpose was risk management against uncertainties that have been resolved.

The integration is now two landings: a complete first Lean institution, followed by a Mathlib-scale operational landing.

### 11.1 First complete Lean institution

A single integrated landing that ships:

- **Kernel.** `InProcess` runtime kind dispatch (§10.1).
- **Verification-side crate** (`eigenius-lean`). nanoda_lib integration, chain-mirror translator, correspondence-check handler, environment cache, `Institution` trait implementation.
- **Authoring-side crate** (`eigenius-lean-runtime`). `LanguageRuntime` impl (`build_environment_image`, `dockerfile_fragments`, `run_script`, `call_method`), worker bootstrap, `MirrorGenerator` impl, Lean subclass declarations.
- **Ontology.** The `urn:eigenius:institutions:lean` `Institution` resource + the §10.3 class and declaration set + the `lean:LeanExpr` chain-mirror ontology.
- **First `LeanEnvironment` image.** Lean 4 + Lake + the EigonFFI mirror generated against Core; default axiom allowlist.
- **First end-to-end test.** A small but real proposition committed against a chain-side class with a required property — a `LeanProofTerm` enters the chain, AutoOnLoad fires, nanoda re-checks, the correspondence check passes, the resource lands as *verified*.
- **Sibling specs.** D30 (faithful translation Eigon → Lean) and D40 (chain-mirrored Lean expressions) authored alongside the implementation.

This replaces what previous drafts called Phases A + B + the substrate-irrelevant parts of C. The single landing carries the full architectural commitment from §1's "verified epistemic level" all the way to a committed resource so tagged. No PoC-without-correspondence-check intermediate.

### 11.2 Mathlib-scale operational landing

Once §11.1 is solid and a consumer asks for Mathlib-dependent proofs. Distinct landing because Mathlib's concerns are operational, not design:

- Image-build pipeline work — Mathlib's footprint, dependency lockfile management, deterministic image-build determinism at Mathlib scale.
- nanoda parsed-environment cache tuning.
- Resource-bound discipline (`max_wall_time_ms`, `max_memory_bytes`) calibrated against realistic proofs.
- EigonFFI mirror generation against the larger ontology surfaces Mathlib proofs naturally reference.

### 11.3 Post-landing enhancements (not phases)

- **Lean4Lean as secondary cross-checker** (§8.1). Soundness multiplier per the Venn-diagram argument.
- **WASM-sandboxed checker** (§8.2). Only if benchmarks justify it.
- **Structured `EnvironmentDiff`** (§7.4). When the coarse "different image digests" view isn't enough.
- **Bidirectional Lean ↔ Julia bridge** ([D27 §6.2](d27-julia-institutions.md#62-verification-of-intervalarithmetic-outputs--concrete-d14-comorphism)). First concrete cross-institution Comorphism involving Lean.

Each is a self-contained piece of work that layers onto the integrated foundation without changing its shape.

## 12. Open questions

The original D28 carried nine. Five have been answered structurally (correspondence granularity follows D29's seed-classes pattern; the generator is substrate Rust code; the faithful-translation spec is its own document D30; `DependencyOn` extraction depth defaults to direct deps with a per-query override; the kernel TCB doesn't grow). Four remain:

1. **Axiom allowlist policy.** The mechanism is in place (nanoda's allowlist, declared on `LeanEnvironment`, §7.1). What axioms ship as the default Eigenius-side allowlist? `propext`, `Classical.choice`, `Quot.sound`, and `Lean.trustCompiler` are the community baseline; whether to include `Classical.choice` by default (some deployments reject `Classical`-dependent proofs entirely) is the live policy question.

2. **Lean-version upgrade policy.** When a new Lean version or Mathlib version is released, what is the process for promoting it? Automatic with regression testing, or manual with explicit review? The substrate's image-digest model means an upgrade is structurally just a new content-addressed `LeanEnvironment` resource; the question is who/when/how, not what.

3. **Parallel verification institutions.** If Rocq lands and a proof could be checked by either, which is dispatched? User preference, contract preference, or explicit IRI only? Defer until Rocq is real.

4. **Kernel extension for verification status in types.** Should `EigonClass(iri)` be extended to carry a verification witness ("this `StressResult` was verified under environment E"), lifting epistemic status from runtime provenance to compile-time validation? D28 v1's question #9. The cost is non-trivial Mini-TT extension; the benefit needs a concrete consumer asking for it. Defer until the first pipeline that would benefit from typed verification status exists.

---

## Appendix A: Survey of external Lean 4 checkers

The implementation decision in §8.1 picked nanoda_lib. This appendix documents the landscape, captured to support that decision and to record why it was made.

External checkers operate on the **Lean 4 export format** rather than on Lean source code. They consume a plain-text representation of fully elaborated kernel terms, produced by a separate exporter, and verify that those terms type-check according to Lean's kernel rules. They are the mechanism by which Lean proofs can be independently re-checked outside the Lean toolchain itself — the basis of the de Bruijn criterion applied to Lean's ecosystem.

### A.1 lean4export — the exporter

**Repository:** [github.com/leanprover/lean4export](https://github.com/leanprover/lean4export)

Maintained under the `leanprover` organization (official). Produces the JSON-based declaration export format that external checkers consume. This is the tool the Lean side of the integration invokes to produce the proof-term bytes that cross the institution boundary.

The export format is documented in the repository README; Chris Bailey's book [Type Checking in Lean 4](https://ammkrn.github.io/type_checking_in_lean4/) is the comprehensive specification, recommended reading for anyone implementing against the format.

### A.2 Lean4Lean — the verified checker

**Repository:** [github.com/digama0/lean4lean](https://github.com/digama0/lean4lean)
**Paper:** *Lean4Lean: Verifying a Typechecker for Lean, in Lean*, Mario Carneiro, [arXiv:2403.14064](https://arxiv.org/abs/2403.14064)

The most actively developed external checker for Lean 4. Written in Lean itself, which allows the checker's own correctness to be partially formally verified against an abstract Lean metatheory specification.

Performance runs 20–50% slower than the C++ kernel. Checks all of Mathlib successfully. Implements complete Lean 4 kernel semantics including nested inductive reduction, η for structures, and the native Nat/String extensions.

**Caveat on independence.** Lean4Lean is derived directly from the C++ kernel implementation, so it isn't a truly independent implementation — any bug shared between the C++ kernel and Lean4Lean would go undetected. The author is explicit about this; the value proposition is a *consistent* external checker with proofs about its own behavior, not an *independent* one.

**Role in Eigenius.** Recommended as a secondary cross-checker for the post-landing enhancement queue (§11.3). If nanoda_lib and Lean4Lean both accept a proof, the verification is stronger than either alone; disagreements warrant investigation.

### A.3 nanoda_lib — the Rust checker (chosen)

**Repository:** [github.com/ammkrn/nanoda_lib](https://github.com/ammkrn/nanoda_lib) — vendored at [`references/nanoda_lib/`](../../references/nanoda_lib/).
**Documentation:** [Type Checking in Lean 4](https://ammkrn.github.io/type_checking_in_lean4/) (Chris Bailey).

A Rust library implementing type inference and checking for Lean 4 kernel terms. Consumable as a Cargo crate dependency, with an optional binary frontend. Clean-room Rust implementation rather than a port of the C++ kernel, consuming lean4export JSON as input.

Implements the kernel semantics necessary for practical proof checking:

- Mutual and nested inductive types (with the caveat that nested inductive *reduction* historically lagged — worth verifying current status during integration).
- Eta for structures and primitive projections.
- Optional Nat and String kernel extensions, configurable.
- Axiom allow-listing with per-axiom permission flags. Conventional allowlist: `propext`, `Classical.choice`, `Quot.sound`, plus `Lean.trustCompiler`.

**Role in Eigenius.** The primary dependency for the `eigenius-lean` crate. Wrapped inside the `urn:eigenius:lean:proof_check` procedure handler — inside the D14 `Institution::query` method — translating between the institution protocol and nanoda's API.

**Maintenance posture.** nanoda is primarily maintained by a single contributor. Eigenius takes on a relationship with the library including tracking upstream Lean kernel changes propagated through nanoda and contributing back where integration surfaces bugs or gaps.

### A.4 What this survey settles

nanoda_lib as a library-shaped Rust dependency with an accompanying specification book, combined with Lean4Lean as a future secondary cross-checker option, removes the "build a checker" work from the Eigenius roadmap. The implementation approach is a dependency relationship, not a research problem. The remaining work is integration, axiom policy configuration, environment management, and maintenance coordination with the upstream library.

Revisit this appendix annually, or when any of the surveyed projects undergoes a major release, to verify the underlying landscape assumptions still hold.
