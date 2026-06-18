# Runtime Substrate

**Status:** Implemented (Phase 18; Julia substrate live, Lean authoring side wired)
**Scope:** A language-agnostic substrate for hosting external language toolchains inside Eigenius with full provenance. Defines the trait, the resource classes, the image-vs-graph boundary, the deployment model, and the CLI surface. Julia is the first concrete instance for the *derived*-knowledge path; the Lean integration's authoring-side workflow is the second consumer. Per-language design docs (e.g. [`d27-julia-institutions.md`](d27-julia-institutions.md), [`d28-lean-4-as-institution.md`](d28-lean-4-as-institution.md)) layer on top.
**Related:** [`d28-lean-4-as-institution.md`](d28-lean-4-as-institution.md) (verification institution; substrate-hosted authoring + in-process verification), `boundary-contracts.md` (the meta-specification this outline instantiates), [`d27-julia-institutions.md`](d27-julia-institutions.md) (Julia-specific institutions wrapping this substrate)

## 1. Purpose and scope

Eigenius distinguishes *declared* knowledge (human assertion), *observed* knowledge (recorded with provenance), *derived* knowledge (produced by typed pipelines with traces), and *verified* knowledge (machine-checked proofs). Producing knowledge at any of these levels typically involves running an external language toolchain — a numerical-computing runtime, a proof-authoring environment, a symbolic-algebra system, a data-analysis stack. Different toolchains, same problem: how do you host them with the reproducibility guarantees the rest of Eigenius relies on?

This substrate is the answer. It is the route to running *any* external language toolchain inside Eigenius such that every input, every dependency, every artifact produced is content-addressed, layer-anchored, and reproducible. Its primary consumer is *derived*-knowledge production — simulations, numerical analysis, optimisation, symbolic manipulation, parameter fitting, control-loop tuning, all the load-bearing computation that engineering and science actually do, regardless of which language a particular team writes their code in. Its secondary consumer is the *authoring* side of verification: Lean's `lean4export` toolchain, environment instantiation, and `EigonFFI` generation are themselves containerised language-toolchain workflows that benefit from the same provenance machinery (see [`d28-lean-4-as-institution.md`](d28-lean-4-as-institution.md) §2.3 for the substrate factoring).

The substrate provides:

- **Typed pipeline steps** (`RunRuntimeScript`, `CallRuntimeMethod`) that turn input resources into output resources with full provenance, with the script execution running inside a pinned containerised environment.
- **Reproducibility primitives** — every dependency a re-run needs is a graph resource or an OCI image addressable by digest. There is no "external file the trace points at" that could be lost or tampered with.
- **A type-correspondence machinery** that lets language-side code dispatch on Eigon-class-shaped types, with the correspondence content-anchored to specific ontology states.
- **The substrate trait** that per-language crates implement to plug in. A language integration is a new crate, a Dockerfile template, a mirror generator, and a worker bootstrap; everything else is shared.

### 1.1 Why one substrate and not per-language ad-hoc integrations

Without a shared substrate every language reinvents: the worker pool, the image-build pipeline, the boundary-check protocol, the resource-class shape, the CLI publish/run/list/inspect verbs, the sandboxing model, the marshalling format, the precompilation discipline. The Julia integration done in isolation works fine, but it sets precedent for Python and R to copy-paste-and-tweak, and once three integrations exist that took independent design choices, harmonising them is harder than getting it right the first time. The substrate exists because the *boundary* between Eigenius and a hosted runtime is the same regardless of which runtime sits on the other side.

### 1.2 Non-goals

- The substrate does not produce *verified* knowledge. Its epistemic ceiling on hosted *computations* is *derived* with high-quality provenance. Stronger claims come from pairing substrate-hosted output with a verification institution. The Lean integration uses the substrate for its *authoring-side* workflows (proof export, environment instantiation, `EigonFFI` generation), but the proof-checking step itself stays in-process per the trust posture in [`d28-lean-4-as-institution.md`](d28-lean-4-as-institution.md) — that's not a substrate concern.
- The substrate is not a sandbox-as-a-service. It uses sandboxes (OS namespaces + cgroups, not WASM) but its job is provenance + dispatch, not containing untrusted code at scale. WASM remains the right host for fine-grained untrusted capabilities; this substrate is for trusted-but-tracked language toolchains.
- The substrate is not a replacement for the existing IO-component model. `CompleteText`, `CompleteJson`, etc. continue to dispatch through the orchestrator. The substrate adds new component families on top.
- This is not a privileged integration of any one language. Julia is the first concrete *derived*-side instance and Lean is the first concrete *authoring*-side instance, but the trait and the resource shape make no language-specific assumptions.

## 2. Architectural position

The integration is two-tier:

```
┌──────────────────────────────────────────────────────────────────────┐
│  Eigenius kernel — provenance, typing, layer chain, FiberReasoner    │
│  registry. No language knowledge.                                    │
└──────────────────────────────┬───────────────────────────────────────┘
                               │
   ┌───────────────────────────┴─────────────────────────────────────┐
   │  eigenius-runtime-substrate (this document)                      │
   │  • trait: LanguageRuntime                                        │
   │  • components: RunRuntimeScript, CallRuntimeMethod               │
   │  • parent resource classes: RuntimeScript, RuntimePackage,       │
   │    RuntimeEnvironment, RuntimePackageMirror, RuntimeInvocation   │
   │  • image-build pipeline + digest capture                         │
   │  • worker pool, sandbox, RPC framing                             │
   │  • boundary check (mirror anchor, signature match)               │
   └────┬─────────────────┬─────────────────┬─────────────────────┬──┘
        │                 │                 │                     │
   ┌────▼──────┐  ┌───────▼─────┐  ┌────────▼──────┐  ┌──────────▼─────┐
   │ eigenius- │  │ eigenius-   │  │ eigenius-r    │  │ eigenius-lean  │
   │ julia     │  │ python      │  │ (future)      │  │ (authoring     │
   │ (first)   │  │ (future)    │  │               │  │  side only)    │
   └────┬──────┘  └─────────────┘  └───────────────┘  └────────────────┘
        │
        ▼
   Specific institutions (Symbolics / JuMP / IntervalArithmetic / ...)
   wrap individual libraries within their substrate. See per-language
   docs for the details.
```

The kernel speaks `FiberReasoner` and resource graph; the substrate speaks `LanguageRuntime`; the language crates speak both. Institutions sit on top of language crates and translate institution-protocol calls into substrate dispatches.

`eigenius-lean` is the special case worth flagging: the crate has both an authoring side (containerised Lean toolchain hosting `lean4export`, environment instantiation, `EigonFFI` generation — a `LanguageRuntime` implementation) and a verification side (in-process nanoda_lib checker that does *not* go through the substrate). The split is explained in [`d28-lean-4-as-institution.md`](d28-lean-4-as-institution.md) §2.3. From the substrate's perspective, only the authoring side exists; the verification side is invisible.

### 2.1 Substrate vs institution

A **substrate component** runs hosted code with provenance — typed input, typed output, no fiber, no morphism validation. Its contribution is to *derived* knowledge.

An **institution** sits on top of the substrate, wraps a *specific* library that implements its own formal reasoning, and contributes typed morphism / `FiberQuery` machinery. The substrate is what an institution dispatches *into*; the substrate itself never claims institution status.

This factoring keeps each side honest: the substrate is a component family any team can use, while institutions are the locus of typed reasoning claims that downstream consumers can build on. See [`d27-julia-institutions.md`](d27-julia-institutions.md) for the institution-side discussion.

### 2.2 Why no in-process re-checker for substrate-hosted computations

Lean *verification* re-checks proof terms in-process via nanoda_lib because Lean proofs are closed mathematical objects that can be independently re-verified. A hosted-runtime computation isn't: it depends on the runtime, the BLAS lib, the FMA flag, and (if the script uses one) random state. Reproducibility is *operationally* checkable (re-run with the pinned environment, observe the same output) rather than *mathematically* checkable.

The substrate accepts this for hosted computations. It pins everything that can be pinned, records everything that can be recorded, and surfaces non-determinism explicitly when it occurs. The trust posture for substrate-hosted output is "we ran this exact pinned image with these exact pinned inputs and recorded the result"; the verdict is reproducible by re-running, not by re-checking.

The Lean integration is the asymmetric case: its authoring-side workflows (proof-term *export*, `EigonFFI` generation, environment instantiation) run on the substrate exactly because they are operationally-reproducible artifacts that benefit from image pinning, while its verification side (proof-term *checking*) stays in-process for the trust-surface reasons in [`d28-lean-4-as-institution.md`](d28-lean-4-as-institution.md) §2.1. Other languages don't have this split because they don't have a verification side to begin with.

### 2.3 Trusted computing base

- The Eigenius kernel: unchanged.
- The substrate's TCB:
  - The `eigenius-runtime-substrate` Rust crate (RPC, process management, marshalling, image pipeline, boundary check).
  - The container build pipeline (`buildah` / `crane` / equivalent) — produces deterministic image digests.
  - Per-language crates extend the TCB with their respective runtimes, mirror generators, and Dockerfile templates.
- Specific institutions extend their language's TCB with the wrapped library.

The blast radius of a bug anywhere in this stack is confined to *derived* outputs flowing through the substrate or claims flowing through a specific institution. *Verified* claims, declared resources, the ontology itself, and outputs from non-substrate components are unaffected.

This is intentionally a larger TCB than the Lean integration's. *Derived* knowledge does not require the same trust posture as *verified* knowledge — it does require strong provenance so that re-runs and audits are decisive.

## 3. The `LanguageRuntime` trait

Each per-language crate implements one trait:

```rust
pub trait LanguageRuntime: Send + Sync {
    /// Identifier — `"julia"`, `"python"`, etc. Used to namespace IRIs.
    fn language_id(&self) -> &str;

    /// Materialise a `RuntimeEnvironment` resource into a runnable
    /// container image. Returns the image digest captured at push time.
    fn build_environment_image(
        &self,
        env: &RuntimeEnvironment,
        packages: &[RuntimePackage],
        mirror: &RuntimePackageMirror,
    ) -> Result<ImageDigest, BuildError>;

    /// Spawn a worker against a built image. Returns a worker handle.
    fn spawn_worker(&self, image_digest: &ImageDigest) -> Result<WorkerHandle, SpawnError>;

    /// Run a script inside an existing worker. The substrate has already
    /// resolved `script` and `inputs` from the graph; this call passes
    /// them across the worker RPC and waits for the output.
    fn run_script(
        &self,
        worker: &WorkerHandle,
        script: &RuntimeScript,
        inputs: &[Resource],
    ) -> Result<Resource, RunError>;

    /// Call a specific method by signature. Same shape as `run_script`
    /// but with a declared `RuntimeMethodSignature` instead of a script
    /// body.
    fn call_method(
        &self,
        worker: &WorkerHandle,
        signature: &RuntimeMethodSignature,
        inputs: &[Resource],
    ) -> Result<Resource, RunError>;

    /// Emit Dockerfile fragments for the build pipeline. The substrate
    /// composes them with shared base layers; per-language fragments
    /// install the runtime, instantiate dependencies, register the
    /// mirror, and bake build-time provenance.
    fn dockerfile_fragments(&self, env: &RuntimeEnvironment) -> DockerfileFragments;
}
```

The trait is small. Most of the work each language crate does is in `build_environment_image` (the language-specific Dockerfile shape) and the worker-side bootstrap script the substrate spawns. The substrate provides the rest — RPC framing, image push/pull, boundary check, provenance assembly.

## 4. Substrate components and morphisms

### 4.1 Components

- **`RunRuntimeScript`** — runs a `RuntimeScript` resource in a pinned `RuntimeEnvironment` against typed input resources, returns one typed output resource. Catch-all for user-authored scripts.
- **`CallRuntimeMethod`** — calls a single declared method (by `RuntimeMethodSignature`) from an installed package. Sharper surface for the "library call" use case; internally implemented as a one-line script.

Both components are `IO`-tagged. The substrate enforces a runtime sandbox for these effects (§9.4).

### 4.2 Substrate-level morphism: `DispatchedTo`

The substrate contributes one morphism type: `DispatchedTo(invocation, method_signature)`. Records *which* method (in a multiple-dispatch language like Julia) handled the invocation. Re-running must dispatch the same method or it's flagged as a reproducibility failure.

This is technically a morphism but it is not a fiber-internal claim; it's structural metadata. The substrate does not declare itself an institution.

## 5. Resource classes — the parent shape

Eigon's subclass mechanism factors language-agnostic shape from language-specific extensions. The substrate commits parent classes; per-language crates commit their subclasses.

### 5.1 `RuntimeScript`

A single executable artifact (a script file) authored by a user.

| Property | Purpose |
|---|---|
| `language` | Language identifier (`"julia"`, `"python"`, etc.). **Required.** |
| `source` | Script source text, embedded as a string. **Required.** |
| `requires_environment` | IRI of a `RuntimeEnvironment` the script declares as compatible. **Required.** |
| `entry_point` | Declared entry point name. *Optional.* |
| `entry_point_signature` | IRI of a `RuntimeMethodSignature` describing input/output types in mirror-struct terms. *Optional.* |
| `requires_mirror_classes` | List of Eigon class IRIs the script's mirror-struct usage covers. *Optional.* |
| `description` | Human-readable. *Optional.* |

The canonical `RunRuntimeScript` dispatch (§4.1, §6.3) evaluates `source` top-level: the worker loads the script, the script reads `eigenius_inputs` and emits its output resource. That path needs only `language` + `source` + `requires_environment`, which is why those three are required and `entry_point` / `entry_point_signature` are not. A script declares `entry_point` + `entry_point_signature` only when it exposes a *typed* entry point — the `CallRuntimeMethod` surface (§4.1), where the worker dispatches into the named function with marshalled, signature-checked arguments. Requiring an entry point on every script would conflate the two surfaces; the substrate's `LanguageRuntime::run_script` reads only `source`.

Content-addressed IRI is a function of the above (`language`, `source`, `entry_point`, `entry_point_signature`, `requires_environment`). Two notebooks publishing the same script body with the same declared signature and environment produce the same IRI.

### 5.2 `RuntimePackage`

A user-authored library — a directory tree the user maintains separately from any registry, intended for reuse across scripts.

| Property | Purpose |
|---|---|
| `language` | Language identifier. |
| `name` | Package name in the language's namespace. |
| `version` | Internal version string. |
| `manifest` | Language-specific manifest content (`Project.toml` for Julia, `pyproject.toml` for Python, etc.). |
| `source_tree` | Either an embedded archive (CBOR-encoded `Vec<{path, content}>`) for small packages, or a content-addressed reference to external storage for large ones. |
| `entry_points` | List of `RuntimeMethodSignature` IRIs the package exports. |
| `description` | Human-readable. |

Registry packages (`Symbolics.jl`, `numpy`, etc.) are *not* `RuntimePackage` resources — they're pinned by the language-specific lockfile inside `RuntimeEnvironment`. The graph carries the lockfile; the registry carries the bytes.

### 5.3 `RuntimeEnvironment`

The unit of pinned reproducibility. References everything baked into the worker image.

| Property | Purpose |
|---|---|
| `language` | Language identifier. |
| `runtime_version` | Exact runtime version (Julia version, Python version, etc.). |
| `manifest` | Language-specific lockfile content (`Manifest.toml`, `uv.lock`, etc.) — pins registry packages. Verbatim bytes; the round-trip anchor for re-instantiation. |
| `pinned_packages` | List of `RuntimePackagePin` IRIs (§5.6) — parsed Eigon view of the lockfile, optional but supports graph-side queries without re-parsing the manifest. Derived projection of `manifest`; can't drift. |
| `included_packages` | List of `RuntimePackage` IRIs (user-authored libraries) baked into the image. |
| `mirror_dependency` | IRI of the `RuntimePackageMirror` baked into the image. |
| `image_digest` | OCI image digest (e.g. `sha256:abc123…`). The primary reproducibility anchor at runtime. Empty for the dev-path (deployment shape (c), §10.1). |
| `image_reference` | Optional human-readable tag like `registry.eigenius.io/runtime/julia-symbolics:1.4`. Advisory. |
| `lifecycle` | One of `Service` or `Job`. Selects the worker lifecycle (§8) and is required for `CallRuntimeMethod` envs. See §5.3.1. |

The IRI is content-addressed from the above. The image digest is itself a function of the other fields plus a pinned base-image digest, so given a `RuntimeEnvironment` resource an auditor can re-derive the digest from the inputs. See §10 for the build pipeline.

#### 5.3.1 `lifecycle` — Service vs Job

`lifecycle` partitions envs along the dimension that matters for cloud deployment: long-lived request-routed workers (Service) versus run-to-completion ephemeral workers (Job). The substrate maps each onto distinct cloud primitives — Service to a Container App service / k8s Deployment / serverless platform; Job to a Container App Job / k8s Job. Local Docker Compose runs Service envs as compose services or DooD-launched warm sibling containers, and Job envs as DooD-launched per-invocation siblings.

The two values:

- **`Service`** — the env runs as a long-lived service. Workers are kept warm across many dispatches via the pool layered above the [`ServiceSpawner`](#82-spawner-traits-jobspawner-and-servicespawner) trait. Required for any env that backs `CallRuntimeMethod` (§4.1) — methods are typed library calls whose dispatch rate is too high to absorb a cold start each time. `RunRuntimeScript` also dispatches into a Service env; short scripts naturally land on a warm worker without any extra substrate machinery.
- **`Job`** — the env spawns a fresh worker per dispatch via [`JobSpawner`](#82-spawner-traits-jobspawner-and-servicespawner). Only valid for `RunRuntimeScript`; rejected for `CallRuntimeMethod` at the boundary check. Right shape for genuinely long-running scripts (model training, multi-hour simulations) where the cold start is rounding error, and for one-off ad-hoc invocations where there is no warm worker to amortise into.

**Serverless deployments are a Service env with min replicas ≤ 0** plus aggressive scale-to-zero rules. The substrate's code path is identical to a "regular" Service env — what changes is the operator's scaling configuration on the underlying cloud resource. The substrate stays at two lifecycles; cost-mode tuning belongs to the deployment layer.

**Mixed-use envs.** A Service env may serve both `CallRuntimeMethod` and `RunRuntimeScript` simultaneously. Methods reach the warm pool; scripts dispatch into the same pool. This is the common case for institution-bearing envs (e.g. a Julia env serving both `Symbolics.simplify` method calls and ad-hoc bash-equivalent Julia scripts). The substrate does not require partitioning the worker pool by surface — methods and scripts share warm workers keyed on `image_digest`.

**Why this is a property of the env, not the surface call.** The lifecycle decision is a function of the *environment's role* in the deployment topology — how often it's hit, what its cold-start cost is, what the cost profile of keeping it warm is. Different methods on the same env share the same answer; different scripts on the same env share the same answer. Tying the decision to the env keeps the substrate's surface area small and gives operators one knob per env rather than per call.

### 5.4 `RuntimePackageMirror`

The generated bridge between Eigon classes and the language's type system. A `GeneratedLibrary` specialisation, sharing the model from the Lean doc §5.3.

| Property | Purpose |
|---|---|
| `language` | Language identifier. |
| `source_layer` | IRI of the layer the mirror was generated from. |
| `generator_identifier` | The `eigon-<lang>-gen` tool identifier. |
| `generator_version` | Tool version string. |
| `generator_content_hash` | Hash of the generator binary, pinning the exact tool. |
| `library_content_hash` | Hash of the generated library source. |
| `library_content` | Embedded source (small) or content-addressed external reference (large). |
| `mirrored_classes` | The Eigon class IRIs the library mirrors. |
| `generated_at` | Timestamp, advisory. |

The generator is content-hashed and that hash is recorded in every `RuntimePackageMirror`, closing the integrity chain the same way the Lean and Julia generators do.

### 5.5 `RuntimeInvocation`

The provenance closure for one execution.

| Property | Purpose |
|---|---|
| `language` | Language identifier. |
| `script` | IRI of the `RuntimeScript` (or `RuntimeMethodSignature` for `CallRuntimeMethod`). |
| `environment` | IRI of the `RuntimeEnvironment` resource. |
| `image_digest` | Echoed copy of `RuntimeEnvironment.image_digest` at dispatch — self-contained for audit. |
| `mirror` | IRI of the `RuntimePackageMirror` used at the boundary. |
| `inputs` | Ordered list of input resource IRIs. |
| `output` | IRI of the output resource. |
| `dispatched_to` | The specific `RuntimeMethodSignature` resolved by language-side dispatch. |
| `started_at` / `completed_at` | Timestamps. |
| `random_seed` | If set or recorded. |
| `numerical_metadata` | BLAS lib, FMA flag, GPU determinism flags, host kernel — anything affecting reproducibility that isn't pinned by the image digest. |
| `runtime_metrics` | Wall time, allocations, peak memory; advisory. |

The invocation is reproducible — same output bytes — iff `script + environment + inputs + random_seed` resolve to the same content-hashed values on a re-run, against a host with matching `numerical_metadata`. The image digest pin makes "same environment" objective.

### 5.6 `RuntimePackagePin` — the parsed lockfile view

Every language's lockfile (Julia's `Manifest.toml`, Python's `uv.lock`, Cargo's `Cargo.lock`, etc.) is generated by the language's own resolver and stored verbatim in `RuntimeEnvironment.manifest` for round-trip fidelity. The *content* of that lockfile is structured information — pinned packages, versions, git tree hashes, compatibility constraints, dependency edges — that's natural to query through EigenQL.

To support that without re-parsing the lockfile bytes on every query, the substrate adopts a **dual-representation pattern**:

1. **Verbatim bytes** in `RuntimeEnvironment.manifest` — the audit-and-replay anchor; feeds back into `Pkg.instantiate` / `uv sync` / `cargo` deterministically.
2. **Parsed Eigon view** as a set of per-package `RuntimePackagePin` resources committed alongside.

```
RuntimePackagePin
├── language: "julia" / "python" / "rust" / ...
├── package_name: e.g. "Symbolics"
├── package_identifier: language-specific (UUID for Julia, name+version for npm, etc.)
├── pinned_version: e.g. "5.4.2"
├── source_hash: git tree hash, sdist hash, registry digest — language-specific
├── source_origin: registry URL, git URL, path; advisory
└── depended_on_by: [<other RuntimePackagePin IRIs>]
```

`RuntimeEnvironment.pinned_packages` references this set as IRIs (each pin content-addressed). Per-language crates parse the lockfile once at `env create` time and commit both representations. Drift between them isn't possible because both come from the same source bytes — the parsed view is a derived projection of the verbatim manifest.

Use cases this enables:

- "Find every environment that pins Symbolics 5.4.x" — single EigenQL query.
- "Is package P with hash H present in this environment?" — graph membership check, no TOML parsing.
- "Show me the dependency tree of this environment" — graph traversal via `depended_on_by`.

The pattern is optional in v1: Phase A and B ship without parsed pins; the projection lands when the first query that needs it gets written. The substrate's contract is "the bytes are sufficient for re-instantiation"; the graph view is a layered convenience.

### 5.7 `RuntimeMethodSignature`

A declared dispatch contract: a method name plus typed input and output Eigon class IRIs.

| Property | Purpose |
|---|---|
| `language` | Language identifier. |
| `method_name` | The method's name in the language. |
| `input_types` | Ordered list of Eigon class IRIs. |
| `output_type` | Eigon class IRI of the return type. |
| `package` | Optional IRI of the `RuntimePackage` (or registry package name) the method belongs to. |

Used by `CallRuntimeMethod` to constrain dispatch and by the boundary check (§7.5) to verify resolved methods match the declared signature.

## 6. The image-vs-graph boundary

Stable, infrequently-changing artifacts are baked into the image. Per-invocation artifacts come from the graph at run time. The split tracks change velocity.

### 6.1 What's in the image

| Artifact | Why baked | Source at build time |
|---|---|---|
| Language runtime binary | Pinned by image; never per-invocation | Per-language Dockerfile installs (`juliaup`, `pyenv`, etc.) |
| Registry packages | Pinned by lockfile; expensive to instantiate | Lockfile-driven install at build time |
| Precompiled bytecode | First-call latency would otherwise be seconds-to-minutes | Language-specific precompilation step |
| `RuntimePackageMirror` | Stable per generated revision; loaded as a package alongside registry deps | Built mirror archive copied in, registered with the language's package manager |
| User-authored `RuntimePackage`s referenced by the env | Stable like a library; expensive to materialise repeatedly | `source_tree` fetched from graph at build time, written to disk, registered |
| System libs (BLAS, libc, kernel ABI surface) | The reproducibility unit | Base image plus apt/yum installs |

### 6.2 What comes from the graph at run time

| Artifact | Why not baked |
|---|---|
| `RuntimeScript` | Per-invocation; users publish many; can't rebuild images per script |
| Input resources | Per-invocation by definition |
| Output resources | Created by the invocation, committed back |
| `RuntimeInvocation` provenance | Created by the invocation |

The substrate's worker has a small kernel client at startup. When an invocation arrives carrying `RuntimeScript` IRI + input IRIs, the worker resolves them against the graph (cheap if the kernel is in-process; one IPC hop if remote), materialises the script source into a per-invocation tempdir, and runs.

### 6.3 Why packages are baked but scripts aren't

A package is a *library* — used by many scripts, stable across invocations, expensive to load and precompile. If fetched per invocation, every call would pay the load cost.

A script is a *call site* — often unique to one invocation, kilobytes of source, no precompile cost (loaded into an already-warm session in milliseconds).

Mapped onto velocity:
- Packages change weekly or monthly. Image rebuild is acceptable.
- Scripts change every cell evaluation in a notebook. Image rebuild is impossible.

### 6.4 Implications for the workflow

Two CLI verbs with different costs:

```bash
# Cheap: just a graph commit. Same shape as `eigenius load`.
eigenius script publish my-analysis.jl --env <env-iri>

# Expensive: triggers an image rebuild + push.
eigenius env create --lang julia \
    --project-toml ./Project.toml \
    --manifest-toml ./Manifest.toml \
    --mirror <mirror-iri> \
    --include-package <package-iri> ...
```

`eigenius env create` is the heavy operation. It runs the deterministic Dockerfile pipeline (§10.2), reads referenced `RuntimePackage` resources from the graph, materialises their source trees into the build context, and bakes them into the image. The resulting `RuntimeEnvironment` references the image digest and the constituent `RuntimePackage` IRIs — the chain is closed both ways: pull the image to run; resolve the packages to audit what's inside it.

Day-to-day, scientists run `script publish` constantly and `env create` when their dependency graph changes meaningfully.

### 6.5 Reproducibility cache key

Given the boundary above, a `RuntimeInvocation` reproduces deterministically iff:

```
(script_iri, env_iri, input_iris, random_seed)
```

resolves to the same content-hashed values, against a host with `numerical_metadata`-equivalent hardware. The env IRI implicitly captures the runtime version + registry packages + user packages + mirror via the image digest, so the cache key stays compact.

## 7. Type correspondence — the mirror generator pattern

Each language has a deterministic generator (`eigon-julia-gen`, `eigon-python-gen`, …) that produces a language-side mirror of Eigon class structure. The shape is the same across languages; the rendering differs.

### 7.1 What the mirror contains

For each Eigon class a user might call into the runtime about, the generated package provides:

- A language-side struct / class / dataclass with one field per required Eigon property.
- Static / parametric typing where Eigon properties are resource-typed.
- Constructor / validator functions that enforce format constraints at construction time.
- Conversion functions to/from Eigon-JSON / CBOR.
- Inheritance / supertype declarations reflecting Eigon's `subclass_of` relationships, where the language supports them.

### 7.2 What the mirror does NOT contain

- Constraint *predicates* as language-level theorems. Format constraints, `requires` / `recommends`, conditional requirements — checked at construction (validation), not encoded as refinement types.
- Behavioural specifications.

The mirror is **structural, not propositional**. This makes the faithful-translation specification much smaller than `EigonFFI`'s in the Lean integration. Users who want to *prove* things about Eigon-shaped data use the Lean integration; users who want to *compute* over them use this substrate.

### 7.3 Anchoring

Each `RuntimePackageMirror` carries an anchor: `(source_layer, generator_version, content_hash)`. The generator is deterministic; given the same layer and generator version, the output is byte-identical. This makes independent provenance verification possible: an auditor with the generator binary and the layer chain re-runs `eigon-<lang>-gen --layer L`, checks the content hash matches, confirms the mirror is authentic.

Compositionality: a mirror anchored to layer L₀ remains valid for invocations against any descendant layer L₁ ⊒ L₀ provided the mirrored classes haven't changed in L₁. When they have, the substrate rejects the invocation with `MirrorVersionMismatch`, naming the offending class.

### 7.4 The faithful-translation specification

Each language's faithful-translation specification is a finite document mapping Eigon constructs to language-side constructs. A representative skeleton (filled in differently per language):

| Eigon construct | Language-side construct |
|---|---|
| Class with required properties | Struct / class with corresponding fields |
| Class with recommended properties | Same with optional fields |
| Subclass relationship | Language's inheritance / subtype mechanism |
| `data_type: resource` | Field type is the referenced class's mirror struct |
| `data_type: resource_array` | Field type is `Array<mirror>` / equivalent |
| `data_type: integer` / `float` / `boolean` / `string` | Language primitives |
| Format constraints | Constructor-level validation that raises on violation |

Per-language docs flesh this out for their own translation. Julia: [D29](d29-eigon-julia-mirror-spec.md). Lean: D30 (planned alongside `eigon-ffi-gen`).

### 7.5 The boundary check

When a `RunRuntimeScript` or `CallRuntimeMethod` invocation enters the substrate:

1. **Mirror resolution.** Resolve the `RuntimePackageMirror` IRI. Read its anchor. Verify the source layer is ancestral to or identical with the invocation's claim layer. Otherwise: `MirrorVersionMismatch`.
2. **Input shape check.** For each input resource IRI, look up its declared class. Confirm a corresponding mirror struct exists in the resolved package. Otherwise: `MissingMirrorStruct(class_iri)`.
3. **Method-signature check** (only for `CallRuntimeMethod`). Confirm the declared `RuntimeMethodSignature` exists in the pinned environment and that its argument types match the input mirror structs. Otherwise: `MethodSignatureMismatch`.

The full invocation flow then proceeds: marshall inputs into mirror struct values, dispatch into the worker, marshall the output back, construct the `RuntimeInvocation` provenance record.

## 8. Worker process model

### 8.1 Process model — two lifecycles, declared per env

Workers run inside an OS-level sandbox restricting filesystem access to a per-invocation working directory and a read-only mount of the runtime depot. Network access is governed by a per-environment policy.

The substrate exposes **two worker lifecycles**, selected by [`RuntimeEnvironment.lifecycle`](#531-lifecycle--service-vs-job) and mapped to distinct cloud primitives:

- **Service-backed dispatch** (`lifecycle: Service`) — long-lived workers per warm `RuntimeEnvironment`, kept around across many dispatches and request-routed. The pool layered above [`ServiceSpawner`](#82-spawner-traits-jobspawner-and-servicespawner) caches workers by `image_digest`, with idle timeout, max size, and health-check eviction. Required for any env that backs `CallRuntimeMethod` (D26 §4.1) — the dispatch rate of typed library calls (institution morphisms, ExportFormat / ImportFormat handlers, AutoOnLoad QueryClass invocations, `NativeDecide` reductions during type-checking) makes per-call cold-start untenable for languages with non-trivial startup. `RunRuntimeScript` against a Service env *also* dispatches through the warm pool — short scripts get pre-warmed worker latency without any extra substrate machinery.
- **Job-backed dispatch** (`lifecycle: Job`) — fresh worker per dispatch, runs to completion, exits. Provided by [`JobSpawner`](#82-spawner-traits-jobspawner-and-servicespawner). Simpler fault model (no inter-invocation state leak), no pool accounting, no idle bookkeeping. Right shape for genuinely long-running scripts (model training, multi-hour simulations) where the cold start is rounding error, and for ad-hoc one-offs where there is no warm worker to amortise into. Only valid for `RunRuntimeScript`; `CallRuntimeMethod` against a Job env is rejected at the boundary check (D26 §7.5).

**Serverless** is a Service env with `min_replicas: 0` plus aggressive scale-to-zero rules — same substrate code path, different operator-side scaling configuration. The substrate stays at two lifecycles; cost-mode tuning belongs to the deployment layer.

**Mixed-use envs.** A Service env may serve `CallRuntimeMethod` *and* `RunRuntimeScript` simultaneously and share workers across both — methods and scripts dispatch into the same warm pool, keyed on `image_digest`. No partitioning by surface.

**Cloud mapping.**

| Lifecycle | Local Docker Compose | Azure Container Apps | k8s |
|---|---|---|---|
| `Service` | docker-compose service or DooD-launched warm sibling | Container App **service** + autoscaling rules | Deployment + Service (HPA / KEDA) |
| `Job` | DooD-launched per call | Container App **Job** | Job |

Workers communicate with the orchestrator via a small RPC protocol over a Unix domain socket (containers) or local TCP (development). The protocol carries: `instantiate`, `register_mirror`, `dispatch_method`, `evict`, `health`. Marshalling on the wire is **CBOR** ([RFC 8949](https://www.rfc-editor.org/rfc/rfc8949)), using **RFC 8746 typed-array tags** for large numerical arrays so FP / integer matrices avoid per-element type tags. (Note: `evict` is meaningful only for Service envs — Job envs exit on their own when the dispatched work completes.)

CBOR is the rest of Eigenius's serialization format — `LayerHandle`, `BloomFilter`, `MigrationRecord`, every persisted resource is CBOR. Carrying that through to the wire eliminates a translation step at the boundary and lets `RuntimeInvocation` provenance records embed wire bytes directly without re-encoding. CBOR's deterministic-encoding mode (§4.2.1) also matches the kernel's content-addressing posture: byte-equal inputs produce byte-equal output, so the on-disk and in-flight representations of a value are identical.

Cross-language coverage: `ciborium` (Rust, already used internally), [`CBOR.jl`](https://github.com/saolsen/CBOR.jl) (Julia, maintained), [`cbor2`](https://pypi.org/project/cbor2/) (Python, mature). R support (`cbor` package) is weaker; the day R lands as a substrate the integration may need a Rust-side helper or a Cap'n-Proto-style sidecar. AWS DAX uses CBOR for the same cross-language reasons; the production envelope is well-charted.

### 8.2 Spawner traits — `JobSpawner` and `ServiceSpawner`

Container/process lifecycle sits behind two traits inside `eigenius-runtime-substrate`, one per worker lifecycle (§8.1). The split keeps the surface area honest: a Job backend should not be reachable through a path that expects a long-lived service, and vice versa.

```rust
/// One-shot worker: spawn → run-to-completion → exit. Drives
/// `RunRuntimeScript` against `lifecycle: Job` envs.
pub trait JobSpawner: Send + Sync {
    fn spawn(&self, spec: WorkerSpec) -> Result<WorkerHandle, SpawnError>;
    fn wait(&self, handle: &WorkerHandle) -> Result<ExitStatus, SpawnError>;
    fn kill(&self, handle: &WorkerHandle) -> Result<(), SpawnError>;
    fn attach_uds(&self, handle: &WorkerHandle) -> Result<UnixStream, SpawnError>;
}

/// Long-lived service: get-or-start a service for an env, attach a
/// connection per dispatch, drain on shutdown. Drives
/// `CallRuntimeMethod` and `RunRuntimeScript` against
/// `lifecycle: Service` envs.
pub trait ServiceSpawner: Send + Sync {
    /// Get-or-start the service backing an env. Idempotent: repeated
    /// calls for the same `image_digest` return the same service.
    fn ensure_service(&self, spec: WorkerSpec) -> Result<ServiceHandle, SpawnError>;
    /// Open a CBOR RPC connection to the service for one dispatch.
    /// The connection is short-lived; the service is long-lived.
    /// Future K8s / ACA backends will route through their platform's
    /// service endpoint; the dev-side Local / Docker backends use a
    /// UDS path.
    fn attach_uds(&self, service: &ServiceHandle) -> Result<UnixStream, SpawnError>;
    /// Graceful tear-down of the service. Used at orchestrator
    /// shutdown and env retirement.
    fn drain(&self, service: &ServiceHandle) -> Result<(), SpawnError>;
}
```

**Pooling deferred.** Production-target backends (`K8sDeploymentSpawner`, `AzureContainerAppsSpawner`) handle scaling, max-replica enforcement, idle eviction, and liveness/readiness probing at the platform level (HPA / KEDA / ACA scale rules). A substrate-side pool would duplicate and potentially conflict with the platform's decisions. Dev-side backends (`LocalServiceSpawner`, `DockerServiceSpawner`) keep one long-lived worker per env; concurrent dispatches share it via the worker's accept loop. The trait shape above generalises cleanly to all four backends — none of them lease/release.

`WorkerSpec` carries the `image_digest`, the per-invocation tempdir host path, the runtime-depot mount, env vars (including the cross-check digest, §9.3), resource limits, and the seccomp profile. Per-language `LanguageRuntime::spawn_worker` impls call into whichever spawner matches the env's `lifecycle` indirectly — they don't see backend specifics.

Backends:

| Backend | Realises | Use |
|---|---|---|
| `LocalJobSpawner` | `JobSpawner` | Dev, CI, smoke tests. Host subprocess, no container. Reduced sandbox. |
| `DockerJobSpawner` | `JobSpawner` | Production Job envs on Linux via DooD (§9.5). |
| `LocalServiceSpawner` | `ServiceSpawner` | Dev / CI Service envs. Long-lived host subprocess pool. |
| `DockerServiceSpawner` | `ServiceSpawner` | Production Service envs on Linux. DooD-launched persistent service container per env. |
| `K8sJobSpawner` | `JobSpawner` | Cloud — k8s Job per dispatch. Deferred. |
| `K8sDeploymentSpawner` | `ServiceSpawner` | Cloud — k8s Deployment + Service per env, with HPA / KEDA. Deferred. |
| `PodmanJobSpawner` / `PodmanServiceSpawner` | both | Rootless, no-daemon alternatives to the Docker backends. Deferred. |

A backend may realise both traits where it makes sense (the `Local*` and `Docker*` variants share lower-level spawn logic). The substrate's `LanguageRuntime` consumers see one consistent dispatch surface regardless of backend.

### 8.3 Sandboxing

Hosted-runtime scripts can be arbitrary code. The substrate constrains:

- **Filesystem.** Read-only access to the runtime depot; read-write only to a per-invocation tempdir; no other paths visible.
- **Network.** Blocked unless the invocation is `IO`-tagged and the institution / environment registration permits it.
- **Time.** Per-invocation wall-clock limit (declared in the contract; default a few minutes; institutions can raise it).
- **Memory.** Per-invocation memory limit, enforced by cgroup; default a few GiB; institutions can raise it.
- **Syscalls.** A small allow-list (open-relative, read, write to tempdir, mmap for BLAS, etc.). For `DockerSpawner`, applied as a custom seccomp profile via `HostConfig.security_opt` — Docker's default profile is too permissive for the substrate's allow-list.
- **Capabilities.** For `DockerSpawner`, `CapDrop: ["ALL"]` then re-add only what is needed. AppArmor profile applied where available.

Defaults are restrictive; institutions and deployments relax them as needed. On Linux with `DockerSpawner`: namespaces (mnt, pid, net, user) + cgroups v2 + seccomp + capability drop + AppArmor. With `LocalSpawner`: rlimits + per-invocation tempdir only — no namespacing, no syscall filtering. The orchestrator emits a one-line warning at every dispatch under `LocalSpawner` so it cannot be silently used in production. On macOS / Windows: weaker analogs (sandbox-exec, app-containers); coverage is reduced and the operator gets a warning.

### 8.4 Determinism and numerical reproducibility

Default: **deterministic-by-environment**. Same image + inputs + seed on the same host → bit-identical output.
Default: **bit-different across hardware**. Same image + inputs + seed on different hosts → semantically equivalent, possibly bit-different (FMA, BLAS lib, reduction order). Recorded in `numerical_metadata`.

Strict bit-identical-across-hardware is a deployment policy: pin BLAS to a no-FMA build inside the image, refuse to run on hosts whose CPU features would diverge. The substrate supports this mode but doesn't make it the default.

## 9. Container deployment and image identity

The substrate ships hosted runtimes in containers. The unit of "an environment" at deploy time is an OCI image, and that image is what `RuntimeEnvironment.image_digest` anchors against.

### 9.1 Three deployment shapes

(a) **Pre-built image per `RuntimeEnvironment`** — each environment is a distinct OCI image carrying its pinned runtime, instantiated lockfile, precompiled packages, and the mirror package preloaded. The substrate spawns the matching image when an invocation references the environment.

(b) **Single container, environment switched at runtime** — one image; lockfile-driven install per switch. Loses precompilation benefits between switches.

(c) **Runtime bundled in the orchestrator image** — simplest ops; orchestrator image fattens by hundreds of MB to several GiB depending on the dep set.

Production uses **(a)**: it gives the strongest reproducibility (the image digest pins everything below the lockfile, including BLAS, libc, kernel ABI surface, and precompiled bytecode) and matches how production scientific runtimes are typically deployed. The dev / docker-compose path uses (b) or (c) for simplicity — `RuntimeEnvironment.image_digest` is empty in those cases, and reproducibility falls back to "lockfile plus the host runtime install."

### 9.2 Image build pipeline

Deterministic by construction. Inputs are `(base_digest, runtime_version, manifest, included_packages, mirror_iri)`; output is an OCI image whose digest is a function of those inputs.

The pipeline:

1. Compose a Dockerfile from the language's fragments (`LanguageRuntime::dockerfile_fragments`) plus the substrate's shared layers.
2. Materialise the `included_packages` source trees into the build context by resolving their IRIs against the graph.
3. Materialise the mirror archive from `RuntimePackageMirror.library_content`.
4. Invoke `buildah` (or `crane`-built layered tarball) deterministically.
5. Push to the configured registry, capture the digest from the registry's response.
6. Commit the `RuntimeEnvironment` resource carrying the digest.

The build path is `buildah`-driven — never via the run-side container client (§8.2). The two paths are independent: `buildah` produces images deterministically (`--timestamp 0`, ordered layer assembly); the run-side `WorkerSpawner` consumes them by digest. Conflating the two (e.g. driving builds through a Docker daemon client) sacrifices determinism for no operational gain.

The image carries build-time provenance baked into `/etc/eigenius-runtime-env/`:

```
/etc/eigenius-runtime-env/manifest-hash    # content hash of the lockfile
/etc/eigenius-runtime-env/mirror-iri       # IRI of the mirror baked in
/etc/eigenius-runtime-env/included-pkgs    # IRIs of the user packages baked in
/etc/eigenius-runtime-env/built-at         # build timestamp, advisory
```

The image digest itself doesn't appear inside the image (digests are computed after layer assembly), but every input that determines it does.

### 9.3 Identifying the image from inside the container

Three signals, layered:

1. **In-image build provenance** — the files at `/etc/eigenius-runtime-env/`. Read on worker startup; cross-checked against the lockfile the runtime actually sees and against the mirror package loaded.

2. **Start-time environment variable** — the substrate, which spawned the container by digest, passes `EIGENIUS_RUNTIME_ENV_DIGEST=sha256:<digest>` and `EIGENIUS_RUNTIME_ENV_MANIFEST_HASH=<hash>` as env vars. The worker's bootstrap reads these, treats `EIGENIUS_RUNTIME_ENV_DIGEST` as the authoritative digest for self-reporting, and asserts `EIGENIUS_RUNTIME_ENV_MANIFEST_HASH` matches the in-image file.

3. **Runtime-source detection** — Linux `/proc/self/cgroup`, Kubernetes downward-API annotations, Docker socket access. Available but brittle, platform-specific, and (for socket access) a security antipattern. Used only as a fallback diagnostic if (1) and (2) disagree.

The cross-check is the load-bearing piece. If the env var says digest X and the in-image manifest-hash doesn't correspond to the manifest registered for that digest, the worker refuses to start. This catches misconfigurations and tampering before any invocation runs.

### 9.4 What this gives the integration

- **Audit closure.** Given a `RuntimeInvocation`, you have script + image digest + inputs + outputs. The image digest pulls a complete, byte-identical runtime + dependency stack; combined with the script and inputs, the invocation reproduces.
- **Operational independence.** An auditor needs nothing from Eigenius's running infrastructure — the image is in an accessible OCI registry, the inputs and outputs are content-addressed Eigon resources, verification is a local pull + run.
- **Upgrade discipline.** A new `RuntimeEnvironment` (different image digest) is a new resource. Existing `RuntimeInvocation`s remain reproducible against their original image even after the registry serves newer ones; the substrate doesn't implicitly upgrade.

### 9.5 DooD bind-mount discipline and the orchestrator security boundary

When the orchestrator runs in a container and spawns workers via `DockerSpawner`, the spawn pattern is **Docker-outside-of-Docker**: the orchestrator container talks to the host's Docker daemon (via a mounted `/var/run/docker.sock`) to spawn *sibling* workers — not nested ones. Sibling workers are cheap and observable from host tooling, but every path in the spawn request is interpreted against the *host* filesystem, not the orchestrator's container-local view. Without discipline this silently breaks bind mounts: a tempdir path the orchestrator constructs against its own filesystem is meaningless to the daemon.

The substrate's discipline is three rules:

1. **Single stable host path.** The substrate writes per-invocation tempdirs, the runtime depot, mirror archives, and any other worker-visible artifact under one well-known host path (e.g. `/var/lib/eigenius-runtime/`).
2. **Same path inside the orchestrator.** That host path is bind-mounted into the orchestrator container at the *same* path. Paths the orchestrator constructs under that prefix are valid both in its own filesystem and on the host with no translation.
3. **Refuse to start otherwise.** At `DockerSpawner` construction the substrate stats the depot path and verifies the bind-mount is present and points to the expected host inode. If it does not, the substrate refuses to come up rather than spawning workers that will silently see the wrong data.

The depot path choice is a spec-level decision, not a deployment knob: changing it post-hoc breaks every `RuntimeInvocation`'s reproducibility against its original sandbox layout.

**Security boundary acknowledgement.** Granting the orchestrator process access to `/var/run/docker.sock` is root-equivalent on the host: anyone who can drive RPCs into the orchestrator can spawn a privileged container that bind-mounts `/` from the host and pivots. Membership in the `docker` group is the same primitive — it is not a mitigation. The orchestrator host is therefore the substrate's security boundary: no multi-tenant co-tenancy, no untrusted RPC surfaces forwarded into it, no exposed substrate APIs that let an unauthenticated caller trigger arbitrary `RunRuntimeScript` invocations. The substrate enforces this in code at one place — the orchestrator startup logs the active spawner backend and prints a one-line security-posture reminder when `DockerSpawner` is selected — and in deployment docs everywhere else.

## 10. CLI surface

Language-agnostic where it can be, language-aware where it must be.

```bash
# Language-agnostic verbs — auto-detect language from extension on publish.
eigenius script publish my-analysis.jl --env <env-iri>      # → JuliaScript
eigenius script publish my-analysis.py --env <env-iri>      # → PythonScript (future)
eigenius script list [--lang <name>]
eigenius script inspect <iri>
eigenius script run <iri> --inputs <iri,iri,...>

eigenius env list [--lang <name>]
eigenius env inspect <iri>

# Language-aware where the manifests differ.
eigenius env create --lang julia \
    --project-toml ./Project.toml \
    --manifest-toml ./Manifest.toml \
    --mirror <mirror-iri> \
    --include-package <pkg-iri>...

eigenius env create --lang python \                          # (future)
    --pyproject-toml ./pyproject.toml \
    --lock ./uv.lock \
    --mirror <mirror-iri>
```

Top-level operations don't care about language; lower-level operations that handle language-specific artifacts do. Mirrors how `git` factors verbs over file types and how `kubectl` factors over resource types.

## 11. Boundary contract

The substrate components instantiate `BoundaryContract`. Per-language extensions and per-institution refinements are documented in their own design docs.

### 11.1 Substrate-component error taxonomy

Beyond the baseline:

- `MirrorVersionMismatch` — input class not in the resolved mirror, or mirror anchored to an incompatible layer.
- `MethodSignatureMismatch` — declared `RuntimeMethodSignature` doesn't exist in the environment, or its argument types don't match.
- `EnvironmentBuildFailed` — image build failed (network, registry, lockfile conflict).
- `EnvironmentImageUnavailable` — the referenced image digest isn't pullable.
- `RuntimeError` — language-side exception. Diagnostic preserves the stack trace.
- `ResourceLimitExceeded` — wall-clock or memory cap hit.
- `SandboxViolation` — the script attempted a forbidden syscall.
- `WorkerCrossCheckFailed` — the in-image provenance disagrees with the substrate-supplied digest. The worker refuses to start.

### 11.2 Declared properties

- **Determinism:** `DeterministicModuloHardware`. Same environment + inputs + seed → semantically equivalent output; bit-identical depends on hardware metadata.
- **Idempotence:** `Idempotent` modulo numerical noise.
- **Effects:** `IO` for the standard variants; pure variants exist where the institution forbids network/filesystem access beyond tempdir.
- **Resource bounds:** per-invocation `max_wall_time_ms` and `max_memory_bytes`.

### 11.3 Lifecycle

Each substrate component is registered against a specific `BoundaryContract` version. Upgrades produce new registrations in later layers; prior registrations remain valid for their trace history.

## 12. Integration touchpoints in the existing code

### 12.1 Kernel-side changes

- None to EigenTT, the layer system, or the validation engine.
- Minor: `InstitutionRegistry` already accepts arbitrary registrations; per-language institutions plug in through the existing slot.

### 12.2 Orchestrator-side additions

- **Shared crate**: `eigenius-runtime-substrate` — owns `LanguageRuntime` trait, `RuntimeWorker` RPC framing, image-build orchestration, the cross-check pattern, the boundary check, the `RuntimeInvocation` provenance assembly, the parent ontology classes.
- **Per-language crates**: `eigenius-julia`, `eigenius-python` (future), `eigenius-r` (future) — each implements `LanguageRuntime`, owns its language-specific subclasses, mirror generator, Dockerfile template, and worker bootstrap.
- gRPC dispatch surface for the new components reuses existing `ComponentExecutor` plumbing.
- A lifecycle hook in the orchestrator's startup code that warms a default worker pool if a `--runtime-warm <env-iri>` flag is passed.

### 12.3 Ontology additions

Parent classes (committed where the substrate is registered):

- `RuntimeScript`, `RuntimePackage`, `RuntimeEnvironment`, `RuntimePackageMirror`, `RuntimeInvocation`, `RuntimeMethodSignature`.
- `DispatchedTo` morphism class.
- Error classes.

Per-language subclasses live in language-specific layers; institutions add their own morphism / query classes on top.

### 12.4 Mirror generators

Each `eigon-<lang>-gen` is a separate crate (or a feature within the language crate). Part of that language's TCB. Generator binaries are content-hashed and that hash is recorded in every `RuntimePackageMirror` they produce.

The faithful-translation specification per language is a long-lived artifact authored alongside the generator and lives in the design directory.

## 13. Phased plan

T-shirt sizes, ordered by dependency. Per-language phases live in language-specific docs; this section covers substrate work.

### Phase A — Substrate skeleton

`eigenius-runtime-substrate` crate with the `LanguageRuntime` trait, parent resource classes, basic worker RPC, and the orchestrator-side wiring. No language implementations yet; `eigenius-julia` Phase A (in [`d27-julia-institutions.md`](d27-julia-institutions.md)) lands against this.

**Scope:** Small. Scaffolding plus the RPC and provenance shape.

### Phase B — Mirror anchoring + boundary check

The boundary check (§7.5), mirror anchor verification, and the integration with per-language mirror generators. Per-language generator implementations land in the language docs.

**Scope:** Medium.

### Phase C — Image build pipeline + spawn-per-invocation + sandbox

The deterministic build pipeline (§9.2), digest capture, in-image provenance, worker bootstrap cross-check. `DockerSpawner` implementation of `WorkerSpawner` (§8.2) using DooD per §9.5. OS-level sandbox per worker. Resource-limit enforcement (cgroup-based on Linux). `numerical_metadata` recording.

**Spawn-per-invocation in v1.** The substrate ships fresh-spawn-per-call as the only worker shape. The warm-worker pool (§8.1) defers to a per-language phase — Julia's cold-start cost (D27 Phase C) is the first concrete trigger.

**Scope:** Medium. The sandbox and image pipeline are independent workstreams; both need to land before production deployment is viable.

**Acceptance: end-to-end capstone.** Phase C closes with a hello-world round-trip against a *substrate-built* image that extends an [official Julia image](https://hub.docker.com/_/julia) digest as its base. The substrate's build pipeline composes a Dockerfile on top of the upstream digest — adding a minimal Julia worker (`JuliaWorker.jl`, ~100 lines, speaks the substrate's CBOR RPC) and the in-image provenance files — and `buildah` produces a deterministic OCI image. The capstone runtime spawns *that* digest (not the upstream Julia digest), and the round-trip exercises every architectural piece in one test: fragment composition, build context materialisation, deterministic image digest, provenance cross-check, DooD bind-mount discipline, spawn, RPC, sandbox, dispatch. Detailed scope and criteria in [implementation-plan.md](implementation-plan.md) Phase 18d.

### Phase D — Cross-language readiness

Once Julia is mature (per [`d27-julia-institutions.md`](d27-julia-institutions.md) Phase D-E), bring up a second language (Python is the obvious next target) using the substrate. Validates that the substrate's abstractions are right. Phase D's deliverable is the second-language proof-of-concept (a Python `RunRuntimeScript` working end-to-end), not full institution coverage.

**Scope:** Medium. Mostly the language-specific work in a Python crate; substrate changes are lightweight.

## 14. Open questions

1. **Worker pool eviction policy.** LRU is the working assumption; LFU and memory-pressure-driven are alternatives. Operational tuning, not architecture.

2. **Marshalling format.** Resolved: **CBOR** (RFC 8949) with RFC 8746 typed-array tags. Uniform with the rest of Eigenius's serialization, eliminates a wire-vs-storage translation step at the boundary, and the cross-language coverage is solid enough to support DAX-grade workloads in production. JSON remains the Phase A bootstrap option for human-debuggable end-to-end testing before CBOR worker codecs land.

3. **Scope of the mirror.** Mirror every Eigon class on each generator run, or only the classes the user's code references? Lean answered "scoped, with regeneration on demand"; same answer is probably right here, but the policy needs to be explicit per language.

4. **Random seeds and stochastic invocations.** Refuse to run if no seed declared (strict reproducibility), or set one and record it (permissive)? Permissive matches typical scientific workflow expectations and is the working default; strict is a deployment-policy override.

5. **Output classification.** Every substrate output is *derived* by default. Specific institutions (e.g. interval-arithmetic) produce morphisms whose semantics are operationally stronger than ordinary derived data. Should the kernel allow institutions to declare a custom epistemic sub-category, or do they remain *derived* with a stronger morphism shape? The latter keeps the four-categories design clean; the former gives downstream consumers a sharper pivot point. Probably the latter wins.

6. **Mirror generator governance.** Each `eigon-<lang>-gen` is in its language's TCB. Maintenance model and faithful-translation specification governance need to be explicit per language.

7. **Numerical determinism policy.** Best-effort with hardware metadata recorded (working default), or strict refusal on non-conforming hosts? Probably best-effort by default with strict as an institution-level opt-in.

8. **Sandboxing depth.** Linux namespaces + cgroups are the working choice on Linux. macOS / Windows have weaker analogs; coverage there is reduced. Acceptable on dev hosts with a flag warning?

9. **Stale image cleanup.** When user-authored packages are superseded, the old env images still exist with the old version baked. Old `RuntimeInvocation`s reference those old envs and need the old images to remain pullable. The registry needs a retention policy mirroring the graph's reachability GC at the image-registry level. Probably not Phase A.

10. **Environment-build privileges.** `eigenius env create` builds an image. On the user's host, a CI worker, a dedicated build service? Each has different security implications. Substrate should be agnostic; deployment chooses.

11. **Cross-language coexistence.** The substrate factoring is the answer (this document). Each language gets its own crate, all sharing the substrate. Once two languages exist, factor out language-agnostic resource-class fields into the parent classes if the duplication is noticeable.

12. **Abstracting the substrate further.** If a third language exposes a fundamentally different shape (e.g. MATLAB with its session-based model), the trait may need refactoring. Defer until at least two languages exist.

---

*This outline is a starting point. The next step is turning Sections 3 through 11 into a concrete `RuntimeSubstrateContract v1` document with the open questions resolved, at which point Phase A can begin.*

---

## Appendix A: Why a substrate, not WASM

Eigenius already has WASM as the host for fine-grained untrusted capabilities (D12). Why a separate substrate model for hosted scientific runtimes?

- **Performance.** Scientific computing routinely involves tens-to-hundreds of millions of FP operations per invocation. WASM with current SIMD support is competitive but not dominant; native runtimes with optimised BLAS / LAPACK / GPU bindings are dramatically faster. The substrate is for workloads where that gap matters.
- **Ecosystem reach.** `Symbolics.jl` / `JuMP` / `IntervalArithmetic.jl` (Julia), `numpy` / `scipy` / `pytorch` (Python), `MathOptInterface` (Julia), `Stan` / `JAGS` (R) — these are not portable to WASM, and won't be without years of porting effort. The substrate meets the ecosystems where they are.
- **Deployment shape.** Scientific computing infrastructure already runs in containers. A substrate that ships in containers fits the deployment model; a substrate that requires a WASM runtime everywhere doesn't.
- **Trust posture.** WASM's fine-grained sandbox is overkill for trusted-but-tracked scientific code, and underkill for arbitrary user-uploaded code (which the substrate does not host — that goes through WASM). The substrate's posture is "trusted runtime, tracked invocations, deterministic environment."

WASM remains the right choice for arbitrary untrusted user code, fine-grained capability composition, and small-deployment scenarios where pulling a 2 GiB Julia image is impractical. The substrate is the right choice for scientific computing at the rigour expected of the domain.

## Appendix B: Comparison with the Lean integration

| Dimension | Lean integration | Runtime substrate |
|---|---|---|
| Epistemic contribution | *Verified* | *Derived* |
| Trust mechanism | Re-check the proof term in-process (nanoda_lib) | Pin everything that runs (image digest + lockfile + script content hash) |
| Reproducibility model | Mathematical (pure function of inputs) | Operational (deterministic-by-environment, hardware-recorded) |
| Closed object | Proof term + environment + claim layer | Script + environment + image + inputs |
| Mirror generator | `eigon-ffi-gen` (refinement-typed Lean) | `eigon-<lang>-gen` (structural mirror in target language) |
| Worker process | Rust crate linked into the orchestrator | Per-language container worker |
| TCB | Term checker + generator + correspondence | Substrate + per-language crate + per-institution wrapper + image pipeline |
| Failure mode | Refuses bad proofs (binary verdict) | Reports invocation failure with diagnostics; reproducibility divergence visible in metadata |

The two integrations complement each other. Substrate-hosted computations produce *derived* knowledge that Lean proofs can later assert properties about. The first concrete bridge is sketched in [`d27-julia-institutions.md`](d27-julia-institutions.md) §6.

This appendix should be revisited when a third major integration lands (a second-language substrate, a second proof system, or a substantially different verification primitive) to verify the comparison framework still holds.
