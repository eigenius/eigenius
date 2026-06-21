# 11. Runtime substrate

[Chapter 10](10-wasm-institutions.md) covered the WASM extension surface.
This chapter covers the **other** extension surface: the **runtime
substrate** — the orchestrator-spawned, container-hosted runtime that lets
institutions live in their natural language ecosystem (Julia, R, Lean;
Python and others tracked) instead of inside a WASM sandbox. It also hosts a
language-agnostic **`oci` tool runtime** (§11.7, D60) that runs any pinned
containerized tool as an attested, replayable derivation.

The two surfaces are peers, not alternatives. WASM institutions are right
when sandboxing matters, when the institution is hot-installable, or when
the dependency tree is small. Substrate-hosted institutions are right when
the institution wants to use a heavy native library (a SAT solver, an ODE
integrator, a quantum-chemistry engine) that doesn't compile to WASM
cleanly. Both speak the same chain protocol; the same `Institution`,
`QueryClass`, `ExportFormat`, `ImportFormat`, and `Comorphism` resources
work for both.

The substrate is specified by [D26 — Runtime
Substrate](../../design/d26-runtime-substrate.md), [D29 — Mirror
Generator](../../design/d29-runtime-mirror-generator.md), and [D31 —
Institution Lifecycle](../../design/d31-runtime-language-substrate-institution-lifecycle.md);
the implementation lives at [`crates/runtime-substrate/`](../../../crates/runtime-substrate/).

Cross-link: this chapter is the **implementer** view from the substrate
side. The user view (calling institutions from queries and programs) is
shared with WASM institutions and lives in [ESL §9](../esl/09-institutions.md)
and [EigenQL §8](../eigenql/08-institutions.md). The Julia v1
instantiation has dedicated tutorials under
[`julia-institutions/`](julia-institutions/) — start with the
[intervals tutorial](julia-institutions/intervals-institution-tutorial.md)
for the full slow-walk.

## 11.1. The substrate concept

The runtime substrate is a small Rust crate
([`crates/runtime-substrate/`](../../../crates/runtime-substrate/))
loaded into the orchestrator process. Its job is to host *external
runtimes* — workers that aren't WASM-sandboxed and aren't in-process Rust,
but instead run as **sibling containers** of the orchestrator (Docker-out-of-Docker
via a bind-mounted Docker socket, not nested) and communicate over a Unix
domain socket using Eigon-CBOR.

```
┌──────────────┐        ┌──────────────────┐          ┌──────────────────┐
│   Kernel     │ ──RPC─▶│  Orchestrator    │  spawns  │  Worker          │
│   (Rust)     │  call  │  (Deno + napi-rs │ ────────▶│  (Julia / etc)   │
│              │   ◀────┤  substrate addon)│          │  Eigon-CBOR / UDS│
└──────────────┘ result └──────────────────┘          └──────────────────┘
                            depot bind-mount  ──────  /var/lib/eigenius/
                            (host ⇆ orch ⇆ worker)    substrate-depot/
```

**Three pieces** worth naming up front:

- **`LanguageRuntime` trait**
  ([`language_runtime.rs`](../../../crates/runtime-substrate/src/language_runtime.rs))
  — the trait every substrate-hosted runtime implements. The Julia
  v1 implementation lives in [`crates/eigenius-julia/`](../../../crates/eigenius-julia/);
  Python and others are tracked in
  [issue #41](https://github.com/eigenius/eigenius/issues/41). One
  trait, one method per chain operation (`build_environment_image`,
  `call_method`, etc.).

- **`Spawner`** — the per-runtime container lifecycle. The Julia spawner
  uses `docker create` + `docker start` against the env image's content-addressed
  digest. The first call against a new env-image digest spawns a worker;
  subsequent calls against the same digest reuse the cached `ServiceHandle`.
  Workers stay alive between calls (Julia's startup cost makes per-call
  containers prohibitively expensive); the substrate keeps a per-image
  pool in memory.

- **The depot bind-mount** — the directory `/var/lib/eigenius/substrate-depot/`
  is bind-mounted into the orchestrator container at the *same* path it
  appears on the host. Workers write their UDS sockets into this directory,
  and the orchestrator reads them at the same absolute path. This is the
  one piece of host configuration the substrate depends on; without it,
  the orchestrator can't reach worker sockets.

The kernel itself doesn't know any of this. It dispatches via the existing
`DispatchExternal` gRPC, the orchestrator routes the dispatch to the
substrate, and the substrate marshals through the env image and worker
container. The kernel sees a `ResourceCBOR → ResourceCBOR` operation; the
substrate handles the runtime-specific plumbing.

## 11.2. The four chain resources

Substrate institutions are *declared* on the chain through four typed
resources. Each is committed via the same `eigenius load` /
`eigenius institution install` flow that any other ontology declaration
uses; nothing about the substrate is operationally privileged.

| Resource | Purpose |
|---|---|
| `RuntimePackageMirror` | Auto-generated, language-specific source code that mirrors a slice of the chain (selected `Class` and `InductiveType` resources) into typed structs the worker can decode and dispatch on. Content-addressed by source hash. |
| `RuntimeEnvironment` | Pinned identity of a worker image: `image_digest` (sha256), `runtime_version` (e.g. `1.12.6`), `lockfile`, `lifecycle: Service`. Refers to the mirror it bakes in. |
| `RuntimeMethodSignature` | Typed input/output contract for a single handler method — `input_types: [Class₁, …]`, `output_type: Class`. Used by the kernel to marshal arguments and validate returns; used by the mirror generator's closure walker to discover cross-institution classes. |
| `Institution { runtime: external, requires_environment: <env-iri>, … }` | The chain-side identity of the institution; refers to the env it dispatches against. |

The first three (mirror, env, signatures) are lifecycle resources; the
fourth (Institution) is the same resource type WASM and in-process
institutions also use, distinguished by its `runtime` property.

A walk through the contract:

1. The mirror takes a snapshot of the chain shapes the institution needs
   to encode/decode. For Julia, this becomes `EigeniusMirror.jl` with
   `struct BoundedBy ... end` + `decode_BoundedBy(::Dict)::BoundedBy` +
   `encode_BoundedBy(::BoundedBy)::Dict` per class. The chain commit is
   content-addressed — re-running the generator over the same seed
   produces the same IRI.
2. The env image bakes the mirror plus the handler package plus the
   language runtime plus the worker library. `eigenius env build` runs the
   buildah-driven build on the host and prints the resulting digest; you
   then commit the `RuntimeEnvironment` resource pinning that digest plus
   the runtime version.
3. The institution declaration ties everything together — names the
   environment it requires, declares its `RuntimeMethodSignature`s, and
   declares its `QueryClass`es / `ExportFormat`s / `ImportFormat`s.
4. At dispatch time, the kernel resolves the QC's `query_handler`
   procedure to a method name on a signature; the substrate looks up the
   env's image digest, ensures a worker is running (cold-spawn or warm-pool),
   marshals the input as Eigon-CBOR, sends a `DispatchMethod` over UDS,
   and returns the worker's typed response.

## 11.3. Install flow end-to-end

The CLI surface lives in [chapter 4 §4.7–§4.9](04-cli-reference.md#47-mirror-commands).
At a high level, installing a substrate-hosted institution is four steps:

```bash
# 1. Make sure the chain has the institution's ontology loaded.
eigenius load <institution-ontology.eigon.json>

# 2. Generate the mirror against the head layer; commit a
#    `RuntimePackageMirror` resource and write the source files locally.
eigenius mirror create \
    --layer "$(eigenius branch show main | awk '{print "urn:eigenius:layer:"$2}')" \
    --filter '<EigenQL query selecting seed classes>' \
    --institution-file <institution-declaration.eigon.json> \
    --language julia \
    --output /tmp/my-institution-mirror \
    --json

# 3. Build the env image with the handler package + mirror baked in.
#    Prints the image digest + runtime version.
eigenius env build \
    --language julia \
    --package-path <handler-package-dir> \
    --mirror <mirror-iri-from-step-2> \
    --base-image docker.io/library/julia:1.12-bookworm \
    --json

# 4. Commit the `RuntimeEnvironment` resource pinning the digest.
eigenius env create \
    --language julia \
    --handler-package <handler-package-dir> \
    --mirror <mirror-iri-from-step-2> \
    --as-iri urn:my-org:my-institution:env:v1 \
    --image-digest <digest-from-step-3> \
    --runtime-version <version-from-step-3>

# 5. Install the institution declaration (commits Institution +
#    RuntimeMethodSignature x N + QueryClass x N + ExportFormat / ImportFormat).
eigenius institution install \
    --definition <institution-declaration.eigon.json>
```

The slow-walk version of this exact flow — for the `IntervalArithmetic.jl`
institution — lives at
[`julia-institutions/intervals-institution-tutorial.md`](julia-institutions/intervals-institution-tutorial.md).
That's the recommended first reading.

### The `--institution-file` closure walker

Step 2's `--institution-file` flag is worth flagging. The mirror generator
walks the closure of every reachable class from the seed query — properties'
`class_types`, `requires`, etc. — but cross-institution *return* classes
aren't reachable that way. If a Symbolics handler returns an `OptimisationProblem`
(declared by JuMP, not Symbolics), the closure walker can't find it from a
Symbolics-rooted seed.

`--institution-file <path>` augments the seed by parsing the institution
declaration directly and pulling every class IRI mentioned in
`RuntimeMethodSignature.input_types` / `output_type`. The flag reads the
file rather than querying the chain because, in the install order above,
the institution declaration only commits in step 5 — *after* the mirror
generator runs. The file is the source of truth at this stage of the
pipeline.

## 11.4. Worked example — see the intervals tutorial

The smallest viable substrate institution is `IntervalArithmetic.jl`
(rigorous numerical bounds). It's the canonical worked example for the
substrate plumbing:

- One typed shape on the chain (`BoundedBy { value, lower, upper }`).
- One handler method (`validate_bounded_by`) returning a `Verdict`.
- One `QueryClass` with `dispatch_role: AutoOnLoad`.
- The full install flow exercised step-by-step against a live kernel.

The slow-walk lives at
[`julia-institutions/intervals-institution-tutorial.md`](julia-institutions/intervals-institution-tutorial.md).
Reading it once is the fastest way to internalise the substrate's surface
— every concept in this chapter is grounded against a live install in that
walkthrough.

For richer surfaces, the same subdirectory has tutorials for the four
other institutions in the kinase notebook (Symbolics, Catalyst, DiffEq,
JuMP-HiGHS) — each goes one level deeper:

- [Symbolics tutorial](julia-institutions/symbolics-institution-tutorial.md)
  — three dispatch roles in one institution; the FormulaTerm-as-EigenTT-fragment
  story end-to-end.
- [Catalyst tutorial](julia-institutions/catalyst-institution-tutorial.md)
  + [DiffEq tutorial](julia-institutions/diffeq-institution-tutorial.md)
  — the comorphism that compiles a chemical reaction network into an ODE.
- [JuMP-HiGHS tutorial](julia-institutions/jump-highs-institution-tutorial.md)
  — the smart-pow walker rule that keeps quadratic objectives in `QuadExpr`
  rather than `NonlinearExpr` territory.

## 11.5. Substrate vs. WASM trade-off

| Aspect | WASM institution (chapter 10) | Substrate institution (this chapter) |
|---|---|---|
| Sandbox | Yes (Wasmtime) | No (sibling container) |
| Hot-installable | Yes (auto-registration on commit) | Requires `env build` step on the host |
| Dependency tree | Restricted (anything that compiles to WASM) | Full language ecosystem (Julia, Python, …) |
| Native libs | None (or compiled to WASM) | Full access (BLAS, MPI, HiGHS, etc.) |
| Cold-start | Sub-millisecond | Seconds (container spawn + language warm-up) |
| Steady-state cost per call | Wasmtime overhead per call | UDS round-trip per call |
| Trust model | Untrusted authors OK (sandboxed) | First-party / vetted authors |
| Best for | Cap-as-predicate validation; small reasoners; 3rd-party extensions | Numerical / scientific libraries; large precompilation tails; first-party reasoning systems |

The kernel dispatches identically against both — same `Institution::query`
trait surface, same Verdict shape, same FIBER param coercion semantics for
comorphisms. The difference is operational, not protocol.

## 11.6. Where the implementation lives

| File | Purpose |
|---|---|
| [`crates/runtime-substrate/src/language_runtime.rs`](../../../crates/runtime-substrate/src/language_runtime.rs) | `LanguageRuntime` trait |
| [`crates/runtime-substrate/src/spawner/`](../../../crates/runtime-substrate/src/spawner/) | Container-spawning logic |
| [`crates/runtime-substrate/src/image_build/`](../../../crates/runtime-substrate/src/image_build/) | buildah-driven env image construction |
| [`crates/runtime-substrate/src/rpc/`](../../../crates/runtime-substrate/src/rpc/) | Eigon-CBOR-over-UDS protocol |
| [`crates/runtime-substrate/src/mirror_generator.rs`](../../../crates/runtime-substrate/src/mirror_generator.rs) | Closure walker for chain shapes |
| [`crates/runtime-substrate/src/boundary.rs`](../../../crates/runtime-substrate/src/boundary.rs) | Typed boundary marshalling |
| [`crates/runtime-substrate/src/chain.rs`](../../../crates/runtime-substrate/src/chain.rs) | Chain accessor abstraction |
| [`crates/runtime-substrate/src/cross_check.rs`](../../../crates/runtime-substrate/src/cross_check.rs) | Type validation across the boundary |
| [`crates/runtime-substrate/src/facade.rs`](../../../crates/runtime-substrate/src/facade.rs) | Substrate facade exposed to the orchestrator |
| [`crates/runtime-substrate/src/external_file.rs`](../../../crates/runtime-substrate/src/external_file.rs) | External-file resolver — content-addressed IRI minting, `resolve_and_materialize` (fetch + content-verify into the depot cache), `DatasetSchema` parsing / layout validation (D53) |
| [`crates/runtime-substrate/src/oxen.rs`](../../../crates/runtime-substrate/src/oxen.rs) | `oxen://` reference parsing + prebuilt-client fetch (D53 §2) |
| [`crates/eigenius-julia/`](../../../crates/eigenius-julia/) | Julia `LanguageRuntime` instantiation |
| [`julia/runtime-worker/`](../../../julia/runtime-worker/) | The Julia worker source loaded into the env image |
| [`julia/common/EigeniusJuliaCommon/`](../../../julia/common/EigeniusJuliaCommon/) | Substrate-side Julia utilities |
| [`julia/institutions/`](../../../julia/institutions/) | The five v1 Julia institutions |
| [`julia/comorphisms/`](../../../julia/comorphisms/) | Cross-institution comorphism declarations |
| [`crates/eigenius-oci/`](../../../crates/eigenius-oci/) | The generic `oci` tool `LanguageRuntime` + `runtime:BuildRecipe` builder (§11.7) |
| [`crates/eigenius-schemaorg-worker/`](../../../crates/eigenius-schemaorg-worker/) | First `oci` tool worker — the schema.org converter as an Eigenius RPC worker |

## 11.7. Generic OCI tool runtime (D60)

The language runtimes above (Julia, R, Lean) each ship a worker that *speaks Eigon*
via an in-language FFI mirror. The **`oci` tool runtime** is the complementary,
language-agnostic path: it runs **any pinned containerized tool** as a one-shot Job
(`lifecycle:Job`) — a Rust/C binary, a Python script, a shell pipeline. The tool is a
pure transform; the kernel does everything else.

- **Dispatch.** `eigenius run` → kernel `RunRuntimeScript` → the `oci` runtime spawns
  the tool in its `image_digest`-pinned image (the orchestrator resolves the digest the
  env carries — it does **not** build at dispatch). The substrate provisions inputs by
  `content_hash` (§11 external-file resolver); the tool returns its result as
  **Eigon-CBOR**, and the kernel commits it under a `ProgramTrace → IsDerivedAs`. A
  downstream reasoning certificate then discharges `derived(result, P)` over it — the
  WRN wrapped-program pattern, **no new institution**.
- **Kernel-tracked build recipe.** `eigenius env build --language oci
  --worker-source-dir <binary> --base-image <pinned>` bakes the tool into an image
  *and* emits a **`runtime:BuildRecipe`** — base image, baked-artifact content hashes,
  the composed Dockerfile, the exact build command, and the builder version — committed
  with the `RuntimeEnvironment` (`runtime:build_recipe`). "How the image was built" is
  thus a chain-resident, content-verified fact rather than an ad-hoc shell script; it
  generalizes to every runtime.
- **Boot cross-check (D26 §9.3).** The image must be baked from the *same* worker binary
  the orchestrator stages (`EIGENIUS_OCI_WORKER_BINARY_PATH`), or the in-image
  manifest-hash won't match the one the runtime computes at dispatch and the worker
  exits 78.
- **Worked example.** The D57 schema.org generator lift — `demo/d57-schema-org/run.sh`
  runs the generator through the `oci` runtime on a clean DB and lands every conclusion
  (incl. `concl_generator` via `derived(...)` and the thesis) as a kernel-checked
  `Holds`. See [D60](../../design/d60-native-runtime-and-tracked-env-build.md).

## 11.8. Cross-references

- [**Chapter 4 — CLI reference**](04-cli-reference.md) — the `mirror`,
  `env`, and `institution` subcommand groups walked through above, plus
  the `data` subcommand group (§4.12) for attaching, verifying, and
  provisioning the external data files this crate resolves.
- [**Chapter 10 — Building WASM institutions**](10-wasm-institutions.md)
  — the peer surface for sandboxed institutions.
- [**ESL §9 — Institutions in ESL**](../esl/09-institutions.md),
  [**EigenQL §8 — Institutions in EigenQL**](../eigenql/08-institutions.md)
  — the user-facing dispatch surface, identical for WASM- and
  substrate-hosted institutions.
- [**The formula language guide**](../formula/README.md) — the
  shared payload language for cross-institution numerical work, used
  pervasively by the Julia v1 institutions.
- [**D26 — Runtime Substrate**](../../design/d26-runtime-substrate.md),
  [**D29 — Mirror Generator**](../../design/d29-runtime-mirror-generator.md),
  [**D31 — Institution Lifecycle**](../../design/d31-runtime-language-substrate-institution-lifecycle.md)
  — the protocol specs.

---

Next: **[12. Deployment →](12-deployment.md)**
