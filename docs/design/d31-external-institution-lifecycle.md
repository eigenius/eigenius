# External Institution Authoring & Dispatch Lifecycle

**Status:** Implemented v1.1 (Phase 19a.4 carry-over; `mirror create → env build → env create → institution install` lifecycle live in the CLI and orchestrator)
**Changes from v1 → v1.1:** Multi-input dispatch via EigenTT Sigma (§6.5); Comorphism evaluation via two boundary `DispatchExternalRequest` calls + in-kernel EigenTT middle (§6.6); Verdict IRI simplified to `urn:eigenius:invocation:<inv-id>:verdict` (§6.3 — drop the QueryClass-short suffix); substrate handle injection clarified as a napi addon method, not new plumbing.
**Scope:** The end-to-end lifecycle for institutions whose `runtime` is `external` — i.e. dispatched into a substrate-hosted worker (Julia, Lean 4, future Python). Covers mirror generation, registration, and dispatch routing. The pure-evaluation institutions (WASM, in-process Rust) are not in scope; their lifecycle is settled by [D14](d14-institution-realisation.md) and the existing kernel runtime.
**Related:** [D14 — Institution Realisation](d14-institution-realisation.md) (the institution protocol — typed declarations, trait surface, dispatch model, Verdict, Comorphism), [D26 — Runtime Substrate](d26-runtime-substrate.md) (the language-agnostic substrate for hosting external runtimes), [D29 — Faithful Translation Specification: `eigon-julia-gen`](d29-eigon-julia-mirror-spec.md) (the Julia-specific mirror generator), [D6 — Execution Architecture](d6-execution-architecture.md) (the IO/pure separation between kernel and orchestrator), [D12 — WASM Extensibility](d12-wasm-extensibility.md) (the existing in-kernel WASM institution model).

---

## 1. Purpose

D14 defines the protocol every institution conforms to (declarations on the chain, `Institution` trait, AutoOnLoad commit gating, Verdict result class, Comorphism shape). D26 defines the substrate that hosts external-language workers. D29 defines the faithful translation contract for Julia. The remaining gap is the *operational lifecycle* that connects them: how does a developer **author** an institution that runs against an external substrate, and how does the kernel **dispatch** to it at runtime?

This document covers exactly that gap. It does *not* re-derive D14, D26, or D29 — it cites them and pins the missing operational pieces.

The lifecycle has five phases, each with a clear owner. The CLI surfaces what the user touches; the kernel and orchestrator handle the rest.

| Phase | Owner | Artifact |
|---|---|---|
| 1. Mirror generation | CLI → kernel + substrate | `RuntimePackageMirror` resource on chain, mirror source files locally |
| 2. Development | Author (outside Eigenius) | Handler module / institution code referencing the mirror |
| 3. Packaging | Author (outside Eigenius) | OCI image, image_digest |
| 4. Registration | CLI → kernel | Institution declaration on chain (Institution + QueryClass + ExportFormat + ImportFormat) |
| 5. Dispatch | Kernel → orchestrator → substrate | RuntimeInvocation + Verdict resources on chain |

---

## 2. Lifecycle overview

Concretely, what a developer does end-to-end:

```
# Phase 1 — generate the mirror against a layer
$ eigenius mirror create \
    --layer urn:eigenius:demo:layer:l3 \
    --filter-file ./institution-classes.eigenql \
    --language julia \
    --output ./EigeniusMirror

# Output:
#   ./EigeniusMirror/Project.toml
#   ./EigeniusMirror/src/EigeniusMirror.jl
#   committed RuntimePackageMirror IRI: urn:eigenius:runtime:mirror:julia:8a7b6c5d4e3f2a1b

# Phase 2 — write the institution code (developer work, outside Eigenius)
$ cd EigeniusIntervals/
$ # ...edit src/EigeniusIntervals.jl, add validators against the mirror...

# Phase 3 — package as a Docker image (developer work, outside Eigenius)
$ docker build -t my-registry/eigenius-intervals:v1 .
$ # capture the resulting image digest

# Phase 4 — register the institution on the chain
$ eigenius institution install \
    --definition ./institution.eigon-json \
    --image sha256:abc123... \
    --mirror urn:eigenius:runtime:mirror:julia:8a7b6c5d4e3f2a1b

# Phase 5 — dispatch happens automatically when committing resources of a
# class the institution's QueryClass declarations cover. AutoOnLoad
# fires; kernel emits a DispatchExternal request to the orchestrator;
# orchestrator services it via the substrate; Verdict comes back; gate
# applies.
```

The rest of this doc unpacks each phase.

---

## 3. Phase 1 — Mirror generation

### 3.1 CLI surface

```
eigenius mirror create
    --layer <layer-iri>
    [--filter <eigenql> | --filter-file <path>]
    --language julia                    # eventually: rust | lean | python
    --output <directory>
    [--endpoint <kernel-endpoint>]
    [--json]

eigenius mirror get
    --iri <mirror-iri>
    --output <directory>
    [--endpoint <kernel-endpoint>]
    [--json]

eigenius mirror list
    [--language <name>]
    [--layer <layer-iri>]
    [--endpoint <kernel-endpoint>]
    [--json]

eigenius mirror inspect <iri>
    [--endpoint <kernel-endpoint>]
    [--json]
```

`mirror create` is the authoring path. `mirror get` is the recovery path — fetch a previously-created mirror's source files without re-generating (useful when developers clone an institution repo and want the mirror code that matches the chain-pinned IRI). `list` and `inspect` are the inspection verbs.

### 3.2 Mirror creation semantics

`eigenius mirror create` is the only path that produces a mirror. Four steps:

1. **Resolve the seed**. The optional `--filter` / `--filter-file` is an EigenQL query whose result rows must include an `iri` column. Each row's `iri` value joins the seed class set. Without a filter, an empty seed is rejected (the user must specify at least one class to mirror, either via filter or — *future* — a `--seed <iri>` flag).

2. **Walk the closure** at the named layer per the language's faithful-translation spec ([D29 §3](d29-eigon-julia-mirror-spec.md#3-closure-walk) for Julia). Resolves each seed class through the layer chain, transitively pulls in referenced classes via `class_types` and `subclass_of`, builds the mirrored-class set.

3. **Generate** the mirror via the language-specific generator (`JuliaMirrorGenerator`, `RustMirrorGenerator` (planned, §7), …). Output is a `MirrorGenerationOutput` carrying:
   - `mirrored_classes: Vec<Iri>` — the resolved closure.
   - `library: LibraryContent::Embedded(Vec<LibraryFile>)` — the source files.

4. **Commit** the `RuntimePackageMirror` resource to the chain. The resource carries everything in the output plus metadata (generator identifier, generator version, content hashes, `generated_at` timestamp). The IRI is content-addressed (first 16 hex chars of the `library_content_hash` per [D29 §10.3](d29-eigon-julia-mirror-spec.md#103-mirror-iri)).

5. **Write** the source files to `--output`. Same bytes as `library_content`'s embedded entries; same `library_content_hash`. The local files and the chain commit are byte-identical by construction.

### 3.3 Why commit always

Mirror generation always commits to the chain — never write-only-locally. Two reasons:

- **Provenance closure.** The chain is the authoritative record. A local mirror file with no chain entry can't be audited; nobody can verify it matches what the institution depends on. Always-commit avoids the entire class of "the local source drifted from what's pinned" bugs.
- **Idempotence is free.** Mirror generation is deterministic (D29 §10.1): same `(generator_content_hash, source_layer, seed)` produces a byte-identical archive. If a developer runs `mirror create` twice with the same inputs, the second run commits the same `library_content_hash` → same IRI → kernel deduplicates. No churn, no orphaned mirrors.

The flip side — committed mirrors that nobody ends up using — is fine. They're a few KB on the chain; they're provenance. The chain is supposed to record what the system knew, not just what got used.

### 3.4 Filter shape

`--filter <q>` and `--filter-file <p>` are mutually exclusive alternatives. The query must return rows with an `iri` column whose values are class IRIs:

```eigenql
MATCH "urn:eigenius:core:Class"(?c) {
    ...predicates...
}
RETURN [] { iri: ?c }
```

Common patterns:

```eigenql
# Namespace-scoped: every class in a namespace.
MATCH "urn:eigenius:core:Class"(?c) {
    "urn:eigenius:core:short_name": ?name
}
WHERE iri-prefix-matches(?c, "urn:eigenius:demo:assay:")
RETURN [] { iri: ?c }

# Subclass-of: every class derived from a parent.
MATCH "urn:eigenius:core:Class"(?c) {
    "urn:eigenius:core:subclass_of": "urn:eigenius:demo:Animal"
}
RETURN [] { iri: ?c }

# Explicit list (degenerate filter — could be done via repeated --seed).
MATCH "urn:eigenius:core:Class"(?c) {}
WHERE ?c IN ["urn:eigenius:demo:assay:Compound", "urn:eigenius:demo:assay:Target"]
RETURN [] { iri: ?c }
```

The query runs against the layer named by `--layer`. If the query returns zero rows, the CLI errors out (empty seed → no mirror).

Future: a `--seed <iri>` flag (repeatable) for cases where an inline EigenQL query is overkill. Out of scope for v1.

### 3.5 Mirror retrieval (`mirror get`)

`mirror get --iri <mirror-iri> --output <dir>` reads the `RuntimePackageMirror` resource by IRI from the chain, decodes its `library_content` (per [D29 §8.5](d29-eigon-julia-mirror-spec.md#85-codec-registries) — the `Embedded(files)` encoding), and writes the files to `<dir>`. No new commit. The local files are byte-identical to what's pinned on the chain.

Use cases:
- Developer clones an institution's source repo; the mirror IRI is pinned in `Cargo.toml` / `Project.toml` / similar; `mirror get` materialises it into the working directory.
- Verification: a mirror IRI reference on the chain → fetch → diff against developer's local copy → confirm no drift.

The kernel rejects `mirror get` against a non-existent IRI with a typed error.

---

## 4. Phase 2-3 — Development and packaging

The author's deliverable is a typed handler package + a built env image. The substrate owns the rest — `JuliaWorker.jl`, `EigeniusJuliaCommon`, the mirror, the cross-check provenance, the Dockerfile composition, and the image build are all bundled by `eigenius env create` (D26 §10). The author touches only their handler-package source.

### 4.1 Phase 2 — Author the handler package

The handler package is a vanilla Julia (Lean / Python / Rust) package that depends on the generated mirror and `EigeniusJuliaCommon`, plus the institution's third-party dependencies (e.g. `IntervalArithmetic`). No Eigenius-specific framework, no boilerplate beyond standard package layout.

For IntervalArithmetic — the institution we're bringing up alongside this lifecycle — the handler package is:

```
EigeniusIntervals/
├── Project.toml
└── src/
    └── EigeniusIntervals.jl
```

`Project.toml`:

```toml
name = "EigeniusIntervals"
uuid = "<author-supplied UUID>"
authors = ["..."]
version = "0.1.0"

[deps]
EigeniusMirror       = "8a7b6c5d-4e3f-4a1b-9c8d-7e6f5a4b3c2d"
EigeniusJuliaCommon  = "9c8e7a4e-1f2b-4c3d-9e5f-6a7b8c9d0e1f"
IntervalArithmetic   = "d1acc4aa-44c8-5952-acd4-ba5d80a2a253"

[compat]
julia = "1.10"
IntervalArithmetic = "0.20"
```

`src/EigeniusIntervals.jl` declares one handler per `QueryClass.query_handler` procedure IRI. Convention: procedure IRI tail = exported function name. `urn:eigenius:julia:intervals:proc:validate_bounded_by` → `Main.validate_bounded_by` after `using EigeniusIntervals`.

```julia
module EigeniusIntervals

using EigeniusMirror             # Generated mirror with the BoundedBy struct
import IntervalArithmetic
const IA = IntervalArithmetic

export validate_bounded_by

"""
    validate_bounded_by(b::BoundedBy)::Dict{String, Any}

AutoOnLoad validator for `BoundedBy` resources. Verifies
`b.value ∈ [b.lower, b.upper]` using rigorous interval arithmetic.
Returns a Verdict resource — `Holds` when the inclusion is rigorously
proved, `Fails` when it's rigorously refuted, `Undecidable` when the
interval check is inconclusive (rounding, NaN, …).
"""
function validate_bounded_by(b::EigeniusMirror.BoundedBy)::Dict{String, Any}
    bounds = IA.interval(b.lower, b.upper)
    point  = IA.interval(b.value)
    ctor = if IA.issubset_interval(point, bounds)
        "Holds"
    elseif IA.isdisjoint_interval(point, bounds)
        "Fails"
    else
        "Undecidable"
    end
    return Dict{String, Any}(
        "urn:eigenius:core:is_a"        => ["urn:eigenius:institution:Verdict"],
        "urn:eigenius:core:ctor_name"   => ctor,
    )
end

end # module
```

Notes on the Verdict shape:
- The kernel's `parse_verdict` ([kernel/src/institution/dispatch.rs:166](../../kernel/src/institution/dispatch.rs#L166)) accepts two forms: `core:ctor_name` carrying the inductive ctor name (`"Holds"` / `"Fails"` / `"Undecidable"`), or `is_a` containing one of the `urn:eigenius:institution:verdicts:*` class IRIs. The handler can use either; the ctor-name form is simpler and works without committing fresh class IRIs.
- The Verdict gets stamped with `urn:eigenius:reflection:DerivedResource` by the substrate's commit pipeline before reaching the chain (D29 §8.4 cross-link).
- The substrate adds `dispatched_to`, `runtime_invocation`, and the verdict's IRI scaffolding when it commits the resource — handlers don't construct invocation IRIs.

The author's authoring loop is purely Julia-native — `julia --project=. -e 'using EigeniusIntervals; validate_bounded_by(...)'` works for unit-testing the handler against synthesized mirror struct instances. No substrate, no kernel, no env image needed for handler unit tests.

### 4.2 Phase 3 — Build the env image with `eigenius env create`

`eigenius env create` is the substrate's primary author-facing build command. Bakes the worker, common package, mirror, handler package, and any extra path-deps into a deterministic OCI image; commits a `RuntimeEnvironment` resource carrying the image_digest.

```
eigenius env create
    --lang <name>                       # julia | lean | python (planned) | rust (planned)
    --handler-package <path>            # path to author's handler-package directory
    --mirror <mirror-iri>               # references a previously-created RuntimePackageMirror
    [--include-package <path>]          # repeatable: extra package directories to bake in (path-deps)
    [--as-iri <env-iri>]                # IRI to commit the RuntimeEnvironment under
    [--base-image <ref>]                # override the language's default base; defaults pinned per language
    [--push-to <registry>]              # OCI registry; defaults to local docker daemon for dev
    [--endpoint <kernel>] [--json]
```

Concrete invocation for the IntervalArithmetic institution:

```bash
$ eigenius env create --lang julia \
    --handler-package ./EigeniusIntervals \
    --mirror urn:eigenius:runtime:mirror:julia:8a7b6c5d4e3f2a1b \
    --as-iri urn:eigenius:demo:env:intervals-v1

# Output:
#   committed RuntimeEnvironment: urn:eigenius:demo:env:intervals-v1
#   built image: eigenius-julia-intervals:sha256-abc123...
#   image_digest: sha256:abc123...
```

What the substrate bakes in, by layer:

| Layer | Source | Owner |
|---|---|---|
| Julia base | `julia:1.12-bookworm` (or pinned digest, default per `--lang`) | Substrate (default) |
| Worker package | `julia/runtime-worker/Project.toml` + `Manifest.toml` + `src/JuliaWorker.jl` | Substrate (bundled into eigenius-julia crate via `include_str!`) |
| `EigeniusJuliaCommon` | `julia/common/EigeniusJuliaCommon/` | Substrate (bundled identically) |
| Mirror (`EigeniusMirror`) | `RuntimePackageMirror.library_content` resolved from the `--mirror` IRI | Author indirectly (generated in Phase 1) |
| Handler package (`EigeniusIntervals`) | `--handler-package` path | Author |
| Extra packages | `--include-package` paths (zero or more) | Author |
| Cross-check provenance | `/etc/eigenius-runtime-env/{manifest-hash, mirror-iri, included-pkgs, built-at}` | Substrate |
| `CMD` | `julia --project=/opt/eigenius/julia-worker /opt/eigenius/julia-worker/src/JuliaWorker.jl` | Substrate |

`manifest_hash` covers all of the above — any change to worker, common, mirror, or handler bumps the digest. The cross-check (D26 §9.3) verifies the in-image manifest-hash matches the substrate-supplied one at spawn time, so a tampered image is caught before any dispatch runs.

The worker's `Project.toml` declares `[deps]` on `EigeniusJuliaCommon`, `EigeniusMirror`, **and the handler package** so all three land in `Base.loaded_modules` after `Pkg.instantiate` + `Pkg.precompile` at image-build time. The worker's discovery walk (D29 §8.5) then finds `_eigenius_decoders` / `_eigenius_encoders` (from the mirror) and the handler functions (from the handler package's exports).

### 4.3 What the author does NOT touch

To make the boundary explicit:

- ❌ No Dockerfile authoring. `env create` composes it.
- ❌ No `JuliaWorker.jl` editing. Substrate-owned, manifest-hashed.
- ❌ No `EigeniusMirror` source changes. Generated; if the ontology changes, regenerate via `eigenius mirror create`.
- ❌ No `EigeniusJuliaCommon` modifications. Substrate-owned validator helpers.
- ❌ No worker `Project.toml` editing — only the handler-package's `Project.toml`.
- ❌ No cross-check provenance file authoring. Substrate writes them.
- ❌ No image push tooling. `--push-to <registry>` (or default local docker) is one flag.

The author surface is: write a handler package, run `env create`, get an image_digest. Three commands across the lifecycle.

### 4.4 What's still out of scope

- **Image registry authentication.** `--push-to <registry>` assumes the operator has configured registry credentials separately. Eigenius doesn't manage credentials.
- **Multi-handler env images.** v1 supports one handler package per env via `--handler-package`. Multiple handlers can ride along via `--include-package`. Whether to formalise multi-handler envs (one image hosting multiple institutions' handlers) is a future design call — operationally cheaper but couples institution lifecycles.
- **Hot iteration loop.** `env create` is currently a full image rebuild. A faster dev loop ("rebuild only the handler package") is future-work; production reproducibility forbids it.

---

## 5. Phase 4 — Registration

By Phase 4 the chain already holds:

- The institution's resource classes (e.g. `BoundedBy`) — committed as part of a foundational ontology layer the user loaded earlier (via `eigenius load` or similar).
- The `RuntimePackageMirror` for those classes — committed by Phase 1's `mirror create`.
- The `RuntimeEnvironment` carrying the env image_digest — committed by Phase 3's `env create`.

Phase 4 commits the institution's *institutional* declarations on top: `Institution`, `QueryClass`, `ExportFormat`, `ImportFormat`, `Comorphism` resources that reference the artifacts already pinned.

### 5.1 CLI surface

```
eigenius institution install
    --definition <file>              # Eigon-JSON / ESL declaring the institution
    [--endpoint <kernel-endpoint>]
    [--json]
```

Sends a `LoadRequest` (existing kernel RPC) with `auto_commit: true`. No `--image` flag — the env IRI in the definition already pins the digest. No `--mirror` flag — the mirror IRI in the definition already pins the mirror. Validation cross-checks both at commit time.

If a developer wants to **change** the env (new image build, e.g. updated handler) or **change** the mirror (regenerated against a new layer), they:
1. Run `env create` / `mirror create` again to commit the new env / mirror.
2. Update the institution's definition file with the new IRIs.
3. Run `institution install` again. The kernel commits a new layer with the updated Institution resource.

The institution IRI stays the same; the references it carries change. Old `RuntimeInvocation` resources continue to reference the old env IRI; the institution's *current* declaration points at the new one. Per [D26 §9.4](d26-runtime-substrate.md#94-what-this-gives-the-integration), older invocations remain reproducible against their original env image.

### 5.2 Definition file structure

For IntervalArithmetic, the definition file commits the institution-shape resources (Institution + QueryClass + the typed-format declarations). The `BoundedBy` resource class itself is *not* in this file — it's part of the foundational ontology layer committed before mirror generation.

```json
[
    {
      "@id": "urn:eigenius:julia:intervals",
      "urn:eigenius:core:is_a": ["urn:eigenius:institution:Institution"],
      "urn:eigenius:institution:institution_iri": "urn:eigenius:julia:intervals",
      "urn:eigenius:institution:institution_name": "IntervalArithmetic",
      "urn:eigenius:institution:runtime": "urn:eigenius:institution:runtimes:external",
      "urn:eigenius:institution:runtime_environment": "urn:eigenius:demo:env:intervals-v1",
      "urn:eigenius:institution:mirror": "urn:eigenius:runtime:mirror:julia:8a7b6c5d4e3f2a1b"
    },
    {
      "@id": "urn:eigenius:julia:intervals:qc_validate_bounded_by",
      "urn:eigenius:core:is_a": ["urn:eigenius:institution:QueryClass"],
      "urn:eigenius:institution:query_class": "urn:eigenius:julia:intervals:BoundedBy",
      "urn:eigenius:institution:result_class": "urn:eigenius:institution:Verdict",
      "urn:eigenius:institution:dispatch_role": ["urn:eigenius:institution:dispatch_roles:auto_on_load"],
      "urn:eigenius:institution:query_handler": "urn:eigenius:julia:intervals:proc:validate_bounded_by",
      "urn:eigenius:institution:institution_ref": "urn:eigenius:julia:intervals"
    }
    // ... ExportFormat / ImportFormat / Comorphism declarations follow same pattern
]
```

**Two new properties on `Institution`** for the external case:

- **`institution:runtime_environment`** — IRI of the `RuntimeEnvironment` carrying the image_digest. Required when `runtime: external`; absent for WASM and in-process.
- **`institution:mirror`** — IRI of the `RuntimePackageMirror` the institution depends on. Required when `runtime: external`; absent (or generator-output-only) for WASM and in-process.

The kernel cross-checks both references at commit time — install fails if the mirror or env IRI doesn't resolve in the layer chain.

**Procedure IRI ↔ handler function convention.** `QueryClass.query_handler` (and `ExportFormat.procedure`, `ImportFormat.procedure`) carry a procedure IRI. The convention: procedure IRI's local tail = handler function name in the worker's `Main`. So `urn:eigenius:julia:intervals:proc:validate_bounded_by` resolves to `Main.validate_bounded_by` after the env image's `using EigeniusIntervals` lands the handler in scope.

The kernel emits this function name in the dispatch request (§6.2); the worker looks it up via `getfield(Main, Symbol(method_name))` and invokes via `Base.invokelatest` per [D29 §8.5](d29-eigon-julia-mirror-spec.md#85-codec-registries).

### 5.3 Kernel-side processing

On commit:

1. Validate the resources against the ontology + chain.
2. Cross-check references: `Institution.runtime_environment` → must resolve to a `RuntimeEnvironment`; `Institution.mirror` → must resolve to a `RuntimePackageMirror`; `QueryClass.query_handler` procedure IRIs are unique within the institution.
3. Commit the layer.
4. Rebuild `InstitutionIndex` (existing behavior).
5. **Skip runtime registration for `runtime: external`** (existing behavior — already by design).
6. (No push to the orchestrator. The dispatch info travels per-request, §6.)

The orchestrator never learns "an external institution was registered" out-of-band. It learns when the kernel emits a `DispatchExternalRequest` for that institution.

---

## 6. Phase 5 — Dispatch

### 6.1 Architecture: kernel-emits-request, orchestrator-services-IO

The substrate is IO. By the kernel-orchestrator separation in [D6](d6-execution-architecture.md) and [D12](d12-wasm-extensibility.md), IO lives orchestrator-side. The kernel doesn't drive Docker daemons, doesn't open UDS sockets to spawned workers, doesn't pull OCI images. The substrate stays out of the kernel.

This is the same split that puts `CompleteText` (LLM call), `CompleteJson`, `RunRuntimeScript`, and `CallRuntimeMethod` in the orchestrator's component handlers ([orchestration/src/components/](../../orchestration/src/components/)) while `WasmComponent` and `WasmInstitution` execute in-kernel via Wasmtime. **External institution dispatch is the same shape, instantiated for the institution surface instead of the component surface.**

The dispatch flow:

```
[orchestrator]                 [kernel]                       [orchestrator]                  [substrate]                 [worker]
       |                          |                                  |                            |                          |
       | Load(institution decls)  |                                  |                            |                          |
       |───── grpc ──────────────>|                                  |                            |                          |
       |                          | commit, rebuild index            |                            |                          |
       |<──── LoadResponse ───────|                                  |                            |                          |
       |                          |                                  |                            |                          |
       | Load(resource of class C)|                                  |                            |                          |
       |───── grpc ──────────────>|                                  |                            |                          |
       |                          | walk AutoOnLoad QueryClasses     |                            |                          |
       |                          | one is runtime: external         |                            |                          |
       |                          |                                  |                            |                          |
       |                          | DispatchExternalRequest          |                            |                          |
       |                          |───── kernel→orchestrator ───────>|                            |                          |
       |                          |                                  | substrate.call_method()    |                          |
       |                          |                                  |───────────────────────────>|                          |
       |                          |                                  |                            | UDS RPC                  |
       |                          |                                  |                            |─────────────────────────>|
       |                          |                                  |                            |<──── result ─────────────|
       |                          |                                  |<──── output_resource ──────|                          |
       |                          | DispatchExternalResponse         |                            |                          |
       |                          |<──── orchestrator→kernel ────────|                            |                          |
       |                          | apply Verdict gate, commit       |                            |                          |
       |<──── LoadResponse ───────|                                  |                            |                          |
```

The kernel's `Institution::query` for `runtime: external` is *not* a Rust trait impl that does work in-kernel. It's a marker that says "this institution dispatches via the orchestrator." When the kernel's commit pipeline encounters one, the kernel makes an outbound gRPC call to the orchestrator's server (`orchestrator_client.dispatch_external(...)`), waits for the response, applies the gate, and continues the commit. Same pattern the kernel already uses for IO WASM components ([kernel/src/server/mod.rs:1051](../../kernel/src/server/mod.rs#L1051) — the `orchestrator_client` field — and `RegisterWasmComponent` / IO component execution use it). Regular request/response gRPC; no streaming, no polling, no two-phase commit semantics.

### 6.2 Wire shape

Single RPC per dispatch firing. Stateless on both sides:

```protobuf
message DispatchExternalRequest {
    string invocation_id = 1;       // substrate-assigned correlation
    string institution_iri = 2;     // for logging / RuntimeInvocation provenance
    string env_iri = 3;             // RuntimeEnvironment to spawn against
    string image_digest = 4;        // sha256:... — the spawn target
    string method_name = 5;         // worker function name to invoke
    string signature_iri = 6;       // RuntimeMethodSignature; goes on RuntimeInvocation.script
    repeated bytes input_resource_cbors = 7;  // one entry per Sigma-component (§6.5); single-element for AutoOnLoad/Decidable
}

message DispatchExternalResponse {
    bytes output_resource_cbor = 1;            // Verdict for AutoOnLoad/Decidable; otherwise instance of QueryClass.result_class
    bytes runtime_invocation_partial_cbor = 2; // dispatched_to, image_digest, started/completed_at, numerical_metadata
}

message DispatchExternalError {
    enum Kind {
        INSTITUTION_NOT_DISPATCHABLE = 0;  // orchestrator can't service this institution
        SUBSTRATE_DISPATCH_FAILED = 1;     // worker spawn / RPC failed
        WORKER_RUNTIME_ERROR = 2;          // worker raised; carries language-side diagnostic
        MALFORMED_OUTPUT = 3;              // worker returned non-decoded output
    }
    Kind kind = 1;
    string message = 2;
}
```

**Why the kernel sends `image_digest` per request** rather than maintaining an orchestrator-side registry:
- Dispatch info is a few KB, dwarfed by the substrate UDS RPC + worker latency. Per-request inclusion is a wash.
- Orchestrator stays stateless: no registry, no cache invalidation, no startup sync. Pure RPC handler.
- Kernel is the source of truth (the chain has the dispatch info via `InstitutionIndex`); centralizing reads on the kernel side avoids drift.
- ServiceSpawner in the substrate already caches by `image_digest`, so warm-pool reuse is automatic without orchestrator-side state.

The orchestrator's handler routes by `image_digest` to the matching `JuliaLanguageRuntime` (or future `LeanLanguageRuntime` etc.) instance, then calls `run_script` / `call_method` per the `method_name` shape.

### 6.3 Verdict commit semantics

For AutoOnLoad firings (and Decidable, where applicable), every dispatch produces a `Verdict` resource that gets committed alongside the gated resource:

- **Holds**: gated resource committed, Verdict committed (carries dispatched_to, runtime_invocation reference).
- **Fails**: gated resource **rejected**, Verdict committed nonetheless. The rejection produces a typed error to the original `Load` caller; the Verdict is the audit anchor explaining *why* the rejection occurred.
- **Undecidable**: gated resource committed, Verdict committed (audit-only; no semantic effect).

Verdict resources carry:

```
{
  "@id": "urn:eigenius:invocation:<inv-id>:verdict",
  "is_a": ["urn:eigenius:institution:Verdict",
           "urn:eigenius:reflection:DerivedResource"],
  "ctor_name": "Holds" | "Fails" | "Undecidable",
  "verdict_subject": "<gated resource IRI>",
  "verdict_query_class": "<QueryClass IRI that fired>",
  "runtime_invocation": "<RuntimeInvocation IRI that produced this Verdict>",
  "dispatched_to": "<which() output>",
  "diagnostic": "<optional language-side message, on Fails>"
}
```

**IRI scheme**. The `@id` is deterministically derived from the invocation IRI: `urn:eigenius:invocation:<inv-id>:verdict`. One AutoOnLoad firing produces one invocation, which produces one Verdict — the suffix doesn't need to discriminate further. Queryability ("all Verdicts for QueryClass X", "all failing Verdicts on subject S", "the Verdict for invocation V") is property-based against `verdict_query_class`, `verdict_subject`, `runtime_invocation`, and `ctor_name`; the IRI scheme contributes uniqueness + readability, not query primitives.

**`ctor_name`** matches the kernel's `parse_verdict` ([kernel/src/institution/dispatch.rs:166](../../kernel/src/institution/dispatch.rs#L166)) — the inductive ctor name the worker stamps in its handler return value (D31 §4.1). The AutoOnLoad gate reads this property to apply the Holds/Fails/Undecidable rule.

The `is_a` includes `DerivedResource` (per the existing substrate stamping rule, [D29 §8.4](d29-eigon-julia-mirror-spec.md#84-information-preservation-across-decode-encode)) — Verdicts are runtime-produced, like every other typed-method output.

Commit ordering within the kernel transaction:

1. RuntimeInvocation resource committed (provenance for the dispatch).
2. Verdict resource committed (audit anchor).
3. **If Holds/Undecidable**: gated resource committed.
4. **If Fails**: transaction aborts — gated resource not committed; RuntimeInvocation + Verdict do persist.

All three commits are within the same kernel commit transaction. There's no partial-state window where a Verdict references a gated resource that didn't make it.

### 6.4 Failure paths

| Failure | Effect |
|---|---|
| Orchestrator unreachable (RPC channel down) | Kernel's commit aborts with `ExternalDispatchUnavailable`; no Verdict committed; gated resource not committed. |
| Substrate spawn failed | Verdict committed as `Fails` with diagnostic; gated resource rejected. |
| Worker runtime error (handler raised) | Verdict committed as `Fails` with the language-side diagnostic; gated resource rejected. |
| Worker returned malformed output | Verdict committed as `Fails`; gated resource rejected. |
| Verdict.Fails | Per §6.3 — commit Verdict, reject gated resource. |
| Verdict.Holds | Per §6.3 — commit both. |
| Verdict.Undecidable | Per §6.3 — commit both. |

The "orchestrator unreachable" path is the only one that doesn't commit a Verdict — by the time the kernel knows the orchestrator is unreachable, it can't dispatch the validator at all, so there's nothing to record. The `Load` request errors out with a transient-ish error code so the caller can retry.

### 6.5 Multi-input dispatch via EigenTT Sigma

`CallRuntimeMethod` and `OnDemand` QueryClass surfaces take multiple typed inputs (e.g. `compute_selectivity_index(c::Compound, t1::Target, t2::Target)`). In EigenTT, the input is a Sigma type:

```
Σ c:Compound. Σ t1:Target. Σ t2:Target. Unit
```

In-kernel, the inputs are typed Sigma values produced by `nbe::eval`. At the institution boundary, the kernel splits the Sigma into its components, then serializes each component as a separate Eigon resource via the institution's `extract_typed` ExportFormat for that component's class. The components feed into a single `DispatchExternalRequest` as the `input_resource_cbors` list (§6.2).

The worker's typed-method dispatch ([D29 §8.5](d29-eigon-julia-mirror-spec.md#85-codec-registries)) already accepts a list of inputs and decodes each by `is_a` — the existing CallRuntimeMethod path is multi-input on the wire from day one.

For AutoOnLoad / Decidable QueryClasses, the input is a single resource (the gated subject); `input_resource_cbors` is a single-element list. The Sigma framing collapses to a degenerate case but the wire shape is uniform.

### 6.6 Comorphism evaluation

D14 §5's `Comorphism(s, m, t)` triple is a cross-institution translation: extract from a source institution via ExportFormat `s`, evaluate the EigenTT term `m: S → T`, reify into a target institution via ImportFormat `t`. For two external institutions, the kernel orchestrates all three steps:

```
   [kernel commit pipeline]
            |
            | 1. evaluate Comorphism(s, m, t) on input resource R
            v
   [kernel] DispatchExternalRequest { procedure_iri = s.procedure, ... }
            |
            v
   [orchestrator → substrate → worker] returns extracted typed payload
            |
            v
   [kernel] decode result → EigenTT Val (typed payload of S type)
            |
            | 2. evaluate `m : S → T` via nbe::eval (in-kernel, pure)
            v
   [kernel] EigenTT Val (typed payload of T type)
            |
            v
   [kernel] DispatchExternalRequest { procedure_iri = t.procedure, ... }
            |
            v
   [orchestrator → substrate → worker] returns reified target Resource
            |
            v
   [kernel commit pipeline] commits target resource + RuntimeInvocations
```

Two `DispatchExternalRequest` calls per Comorphism traversal — one per institution boundary. The EigenTT term `m`'s evaluation stays in-kernel (pure, no IO), reusing the existing `nbe::eval` machinery. The orchestrator services only the IO at the boundaries.

The wire protocol from §6.2 is **already general enough** for this — `procedure_iri` distinguishes `extract_typed` calls from `reify` calls; the kernel knows which to emit at each step. No new RPC, no special "comorphism dispatch" verb.

For mixed Comorphisms (one external institution + one in-process or WASM institution), the in-process / WASM steps go through the existing `Institution::query` trait registry; only the external steps hit `DispatchExternalRequest`. The kernel routes per institution's `runtime` kind.

---

## 7. Per-language mirror generators

D31's lifecycle is generator-agnostic. The CLI dispatches on `--language` to the matching generator implementation. Generators implement the `MirrorGenerator` trait from `eigenius-runtime-substrate`. Each language gets its own faithful-translation specification.

| Language | Generator | Faithful translation spec | Status |
|---|---|---|---|
| Julia | `JuliaMirrorGenerator` (`crates/eigenius-julia/src/mirror_gen.rs`) | [D29 v1.2](d29-eigon-julia-mirror-spec.md) | Shipped (Phase 19a.4) |
| Rust (for WASM institutions) | `RustMirrorGenerator` (planned) | D32 (planned) | Tracked in [issue #41](https://github.com/eigenius/eigenius/issues/41) |
| Lean 4 | `eigon-ffi-gen` | [D30](https://example.invalid/) (planned) | Phase 20 |
| Python | `PythonMirrorGenerator` (future) | TBD | Future |

Each generator owns:
- The closure walk over the chain layer.
- Language-specific type translation rules (D29 for Julia is the template).
- The codec emit (per-language `decode_*` / `encode_*`).
- The integrity-chain anchors (`generator_content_hash`, `library_content_hash`).
- The `_eigenius_decoders` / `_eigenius_encoders` registries (or language-equivalent — Rust uses trait dispatch, Lean uses typeclass instances).

The lifecycle in this doc is **identical** across generators. CLI verbs, registration flow, and dispatch semantics don't change with the language.

---

## 8. CLI consolidation

D31 introduces three new CLI verbs (`mirror create`, `mirror get`, `institution install` for external) that should harmonise with the existing surfaces. Today's CLI has:

- `eigenius capability install <wasm_file> --kind component|institution` — WASM-only.
- `eigenius capability list`, `inspect`, `test`.

The proposed consolidated surface:

```
eigenius component install <wasm_file> --definition <file>
eigenius component list / inspect / test

eigenius institution install --definition <file> [--wasm <file>] [--image <digest>] [--mirror <iri>]
eigenius institution list / inspect / test

eigenius script publish <file> --env <iri>           # D26 §10
eigenius script list / inspect / run

eigenius mirror create --layer <iri> [--filter <q> | --filter-file <p>] --language <name> --output <dir>
eigenius mirror get --iri <iri> --output <dir>
eigenius mirror list / inspect

eigenius env create --lang <name> --project-toml ... --manifest-toml ...   # D26 §10
eigenius env list / inspect
```

`eigenius capability install` would deprecate to an alias for the right `component` / `institution install`. Tracked in [issue #42](https://github.com/eigenius/eigenius/issues/42).

The consolidation is operational; it doesn't affect the lifecycle in this doc. The verbs may rename without affecting registration or dispatch.

---

## 9. Substrate placement: orchestrator, not kernel

The choice to host the substrate in the orchestrator rather than the kernel is **structural, not operational**. It follows from the IO/pure separation in [D6](d6-execution-architecture.md):

- **Kernel**: pure typed evaluation + storage. WASM components and WASM institutions sandboxed (no IO imports linked), synchronous, in-process.
- **Orchestrator**: IO. CompleteText, CompleteJson, file I/O, network calls, anything touching external systems.

The substrate is unambiguously IO:
- Spawning Docker containers.
- UDS RPC to spawned workers.
- Bind-mount setup, depot management.
- Image build via `buildah`, registry pulls, `docker load`.

Hosting the substrate in the kernel would link the kernel binary against Bollard, Tokio, the substrate's spawn machinery, and would put the kernel binary's life in the hands of every substrate failure mode (container leak, hung worker, network timeout). The IO/pure rule rules this out.

A side benefit is fault isolation — if the substrate hangs, the kernel doesn't — but that's incidental. The principled reason is the rule.

In production deployments where K8s/ACA owns image lifecycle, the substrate is thinned to platform RPCs but is still IO. The placement decision doesn't change.

---

## 10. What's not in scope

- **Multi-tenant deployment shape**. Orchestrator-host security boundary, per-tenant orchestrator processes, tenant-isolated substrate handles. Operator concern, not D31.
- **Production K8s/ACA platform integration**. Future operator concern; the substrate's `ServiceSpawner` trait is already structured for it but no backend exists. Lands when an operator scope demands it.
- **Hot-reload of registered institutions**. Today's flow is "install creates a new layer." Hot-reload would let a developer push a new image without a layer commit. Out of scope; the layer commit is the source of truth.
- **Streaming / async dispatch**. AutoOnLoad firings are synchronous-with-commit; long-running queries (model training, multi-hour solvers) can't run as AutoOnLoad. Their pattern is `RunRuntimeScript` / `CallRuntimeMethod` directly, not via the institution gating path.
- **Per-language generator specs**. D29 covers Julia. D30 / D32 / future docs cover other languages.

---

## 11. Decisions log + open questions

### Decisions

| Decision | Resolution |
|---|---|
| Mirror commit timing | On `mirror create` (always commits to chain). |
| Filter shape | `--filter <q>` or `--filter-file <p>`, mutually exclusive. |
| Mirror retrieval | `eigenius mirror get --iri <iri> --output <dir>` (read-only, no commit). |
| Per-language generators | Tracked in [issue #41](https://github.com/eigenius/eigenius/issues/41). |
| CLI consolidation | Tracked in [issue #42](https://github.com/eigenius/eigenius/issues/42). |
| Substrate placement | Orchestrator (IO/pure rule per D6/D12). |
| Push mechanism | None — kernel includes dispatch info per-request; orchestrator stays stateless. |
| Dispatch routing | Kernel-emits-request, orchestrator-services-IO. Same pattern as IO components. |
| Verdict commit | Always committed (Holds, Fails, Undecidable), within the same transaction as the gated resource. |
| Failure shape | Typed error per failure mode (orchestrator-unreachable, substrate-failed, worker-error, malformed-output). |

### Open questions

The four open questions raised in the v1 draft are resolved in this revision:

- ~~**Multi-input dispatch.**~~ → Resolved (§6.5). EigenTT Sigma types in the kernel; lowered to a list of Eigon resources at the boundary. `DispatchExternalRequest.input_resource_cbors` is a list from day one. AutoOnLoad/Decidable are single-element list; OnDemand and CallRuntimeMethod fill multiple components.
- ~~**Verdict resource IRI scheme.**~~ → Resolved (§6.3). `urn:eigenius:invocation:<inv-id>:verdict` is sufficient; the suffix-with-QueryClass-short was unmotivated. Queryability is property-based via `verdict_query_class`, `verdict_subject`, `runtime_invocation`, `ctor_name`.
- ~~**Substrate handle injection.**~~ → Resolved. The substrate is owned by the existing `runtime-substrate-native` napi addon; the new dispatch surface is an additional addon method (`addon.dispatchExternal(...)`). The TS handler in `orchestration/src/components/` calls it. No new injection plumbing.
- ~~**Comorphism dispatch.**~~ → Resolved (§6.6). Kernel orchestrates the three-step `extract → m → reify` traversal: two `DispatchExternalRequest` calls per traversal (one per institution boundary), EigenTT term evaluated in-kernel between. The wire protocol is general enough; no new RPC.

Open items that remain (not blocking v1):

- **Hot iteration loop for institution authoring.** Today every handler change requires a full `env create` rebuild. Faster dev loops (rebuild only the handler package) are operationally appealing but coupled to reproducibility — production-built images can't have an alternate fast-path. Future-work.
- **Multi-handler env images.** §4.4 sketches: one env hosting multiple institutions' handlers. Operationally cheaper but couples lifecycles. Defer until an operator actually wants this.

### Spec versioning

D31 v1 covers the lifecycle for v1.2 of the Julia mirror spec ([D29 v1.2](d29-eigon-julia-mirror-spec.md)). Bumps follow as additional lifecycle features land:

- **v1.x** — additive changes (multi-input dispatch, hot-reload, new failure modes).
- **v2.0** — breaking changes to the dispatch wire shape or registration flow.

Each language's faithful-translation spec evolves independently; D31 only changes when the cross-language lifecycle does.
