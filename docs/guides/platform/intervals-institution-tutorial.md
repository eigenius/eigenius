# Tutorial: an external Julia institution end-to-end

This walkthrough builds the simplest possible *external-runtime* institution: a one-class predicate gate that uses Julia's [`IntervalArithmetic.jl`](https://github.com/JuliaIntervals/IntervalArithmetic.jl) to verify whether a scalar value lies inside a closed interval. By the end you will have:

- Defined a typed shape (`BoundedBy`) on the chain.
- Generated a Julia mirror package that maps `BoundedBy` to a Julia struct and back.
- Built and pinned an OCI image containing your handler + the mirror + Julia 1.12.
- Wired the handler as an `AutoOnLoad` gate so that **every** `BoundedBy` instance committed to the chain is automatically validated, with a `Verdict` resource committed alongside as the audit anchor.

The script form of this tutorial lives at [`demo/intervals/run.sh`](../../../demo/intervals/run.sh) — `just demo-intervals` runs the same sequence non-interactively. Read this if you want to understand *why* each step matters and *what the platform is doing behind the scenes*.

## Prerequisites

- The compose stack running: `EIGENIUS_MOCK_LLM=true docker compose up -d`. This brings up the kernel (port 50051) and the orchestrator (port 8080), with the orchestrator's substrate addon pre-registered for Julia ([§7](07-orchestrator.md)).
- A reachable Docker daemon on the host. The orchestrator's runtime substrate spawns Julia worker containers as **siblings** of the orchestrator container via the bind-mounted Docker socket — Docker-out-of-Docker, not nested. The shared depot path (`/var/lib/eigenius/substrate-depot/`) is bind-mounted into the orchestrator at the *same* path it appears on the host so worker UDS sockets are reachable from both sides.
- `jq` for parsing the JSON output `eigenius env build` emits.
- The workspace built once: the demo invokes `cargo build -q -p eigenius-cli` and runs the result from `target/debug/eigenius`.

The institution sources used throughout live at [`julia/institutions/intervals/`](../../../julia/institutions/intervals/):

```
julia/institutions/intervals/
├── declarations/
│   ├── intervals-ontology.eigon.json      # BoundedBy class + properties
│   └── intervals-institution.eigon.json   # Institution / QueryClass / RuntimeMethodSignature
└── EigeniusIntervals/
    ├── Project.toml                       # dep on IntervalArithmetic.jl + EigeniusMirror
    └── src/EigeniusIntervals.jl           # `validate_bounded_by(b::BoundedBy)` returning a Verdict
```

## Step 1 — Load the typed shape

The institution gates resources of class `BoundedBy`. Before anything else can reference that class, it has to exist on the chain.

```bash
eigenius --endpoint http://localhost:50051 \
    load julia/institutions/intervals/declarations/intervals-ontology.eigon.json
```

You should see:

```
Loaded 4 resource(s) into branch main. Layer: <hex>
```

### What just happened

The ontology file declares one class (`BoundedBy`) with three required `Float` properties (`value`, `lower`, `upper`). `eigenius load` ships the resources to the kernel as Eigon-JSON; the kernel parses, validates against the parent layer's ontology (the embedded core/runtime/institution layers, available since startup), and commits a new layer with these four resources on top of `main`. The new layer's hex ID is what gets printed.

Why this matters: the chain ontology defines the **typed boundary** between Eigon and the Julia handler. The handler will eventually receive `BoundedBy` instances as values; the chain is responsible for guaranteeing those instances actually have a `value`, `lower`, and `upper` of type `Float`. No handler-side defensive type checks are needed.

## Step 2 — Anchor the mirror to a layer

The next step generates Julia source from the chain. That source is *only meaningful relative to a specific version of the `BoundedBy` class definition*: if the class were redefined later (say a fourth required property added), the generated struct would no longer faithfully represent the class. We therefore pin the mirror to the current head of `main`.

```bash
HEAD_HEX=$(eigenius --endpoint http://localhost:50051 branch show main | awk '{print $2}')
LAYER_IRI="urn:eigenius:layer:$HEAD_HEX"
echo "$LAYER_IRI"
```

### What just happened

`eigenius branch show main` returns the hex ID of the layer the `main` branch currently points at. Layer IRIs follow the convention `urn:eigenius:layer:<sha256-hex>`; that's what the rest of the substrate machinery uses. The mirror that the next step commits will carry this IRI as its `runtime:source_layer` property — a permanent record of *which* version of `BoundedBy` it mirrors.

## Step 3 — Generate and commit the mirror

```bash
eigenius --endpoint http://localhost:50051 mirror create \
    --layer "$LAYER_IRI" \
    --filter 'MATCH "urn:eigenius:core:Class"(?iri) { "urn:eigenius:core:short_name": "BoundedBy" } RETURN [] { iri: ?iri }' \
    --language julia \
    --output /tmp/intervals-mirror \
    --json
```

```
{"success":true,"mirror_iri":"urn:eigenius:runtime:mirror:julia:<hex>","file_count":2,"output_dir":"/tmp/intervals-mirror"}
```

Capture `mirror_iri` for the next step.

### What just happened

`mirror create` does three things at once:

1. **Resolve the seed set.** The `--filter` query selects every `Class` resource whose `short_name` is `"BoundedBy"`. That returns one IRI: `urn:eigenius:intervals:BoundedBy`.
2. **Walk the closure on the chain side.** The Julia mirror generator visits each seed class, follows its `requires` properties to property definitions, follows each property's `data_type`/`class_types` to referenced classes, and recurses. For `BoundedBy` the closure stops at three primitive `Float` properties.
3. **Emit Julia source.** Each visited class becomes a `struct BoundedBy ...` in `EigeniusMirror.jl`, with `encode_BoundedBy(b::BoundedBy)::Dict` and `decode_BoundedBy(d::Dict)::BoundedBy` paired alongside it. A registry binds the class IRI to the decoder so the worker can dispatch on `is_a` arrays at runtime. The two emitted files (`Project.toml` and `src/EigeniusMirror.jl`) get written to `/tmp/intervals-mirror/` *and* committed to the chain as a `RuntimePackageMirror` resource — content-addressed by source hash so two mirrors of byte-identical inputs produce the same IRI.

The mirror is the **typed bridge** between Eigon and Julia. It's why your handler can write `b.value` without first parsing JSON or walking property maps — the substrate decodes the wire payload into a fully typed Julia struct before the handler sees it.

## Step 4 — Read the handler

Before baking anything into an image, take a moment to look at what the handler actually does. The full source is at [`julia/institutions/intervals/EigeniusIntervals/src/EigeniusIntervals.jl`](../../../julia/institutions/intervals/EigeniusIntervals/src/EigeniusIntervals.jl); the load-bearing parts are:

```julia
module EigeniusIntervals

using IntervalArithmetic
using EigeniusMirror: BoundedBy

export validate_bounded_by

const VERDICT_CLASS_IRI = "urn:eigenius:institution:Verdict"
const IS_A_PROP         = "urn:eigenius:core:is_a"
const CTOR_NAME_PROP    = "urn:eigenius:core:ctor_name"

function validate_bounded_by(b::BoundedBy)
    target = interval(b.lower, b.upper)
    point  = interval(b.value)
    if issubset_interval(point, target)
        return _verdict("Holds")
    elseif isdisjoint_interval(point, target)
        return _verdict("Fails")
    else
        return _verdict("Undecidable")
    end
end

_verdict(ctor::AbstractString) = Dict{String,Any}(
    IS_A_PROP      => [VERDICT_CLASS_IRI],
    CTOR_NAME_PROP => ctor,
)

end # module
```

### What this implementation is doing

A few non-obvious choices are worth pausing on, because they're the conventions every external Julia institution will follow:

- **`using EigeniusMirror: BoundedBy`** is the line that ties the handler to the chain's typed shape. The mirror you committed in step 3 *is* the source of `BoundedBy`. The handler's signature `validate_bounded_by(b::BoundedBy)` therefore commits to a specific layer's view of the class — change `BoundedBy` on the chain (say, add a fourth required property) and a new mirror has to be generated, a new image built, and a new env Resource committed before the new shape can flow through. The chain pin propagates all the way to the Julia type system.
- **`interval(b.value)` is a degenerate (point) interval.** It's tempting to write `b.value ∈ target`, but that compares a `Float64` to an `Interval{Float64}` and silently drops you into ordinary float comparison — losing the rounding-mode discipline `IntervalArithmetic.jl` exists to provide. By wrapping the value as a degenerate interval first, both sides get the same rigorous treatment, so `Holds` is a *proof* of containment rather than a heuristic.
- **The three-valued return is structural, not a workaround.** `IntervalArithmetic.jl` distinguishes "proven subset" (`issubset_interval`) from "proven disjoint" (`isdisjoint_interval`) from the residual "overlap exists but isn't full subset" — typically the case where `value` lands exactly on a bound that has multiple `Float64` representations. That residual maps directly onto Eigenius's `Undecidable` verdict; the dispatch role decides what to do with it (an `auto_on_load` accepts the load on `Undecidable` but won't claim domain commitment, per [D14 §6.1](../../design/d14-institution-realisation.md#6-verdicts)).
- **The verdict is a `Dict`, not a typed struct.** `Verdict` is declared on the chain as an `InductiveType` (a sum type with `Holds`/`Fails`/`Undecidable` constructors), and the mirror generator only emits typed structs for `Class` resources. The handler therefore returns a Dict shaped exactly like a JSON-LD-flavoured Eigon resource — `is_a: [Verdict]` plus `ctor_name: "Holds"` — and the worker forwards it as-is. The kernel's lenient parser reads `ctor_name` to apply the gating decision.

The handler is small (about 20 lines of substantive code) because most of the work — typing the inputs, decoding from CBOR, dispatching the right method, encoding the output back to CBOR — happens in the substrate and the generated mirror. The handler is just the institution-specific reasoning step.

## Step 5 — Build the environment image

This is where the substrate produces a worker container image: Julia 1.12 base + `EigeniusJuliaCommon` (substrate utilities) + `EigeniusMirror` (the package we just generated) + `EigeniusIntervals` (your handler) + a baked `Pkg.precompile()` of all of them.

```bash
eigenius --endpoint http://localhost:50051 env build \
    --language julia \
    --package-path julia/institutions/intervals/EigeniusIntervals \
    --mirror "urn:eigenius:runtime:mirror:julia:<hex>" \
    --base-image docker.io/library/julia:1.12-bookworm \
    --json
```

```
{"image_digest":"sha256:<digest>","runtime_version":"1.12.6",
 "package_name":"EigeniusIntervals","mirror_iri":"urn:eigenius:runtime:mirror:julia:<hex>"}
```

### What just happened

`env build` runs entirely on **your host**, not in the orchestrator. It:

1. **Reads the handler package from disk.** `Project.toml` for the package name + dep manifest, every file under `src/` for the source tree.
2. **Fetches the mirror Resource from the chain** via the kernel's `Inspect` RPC, base64-decodes the embedded library, and writes the files into the build context.
3. **Composes a Dockerfile.** From the upstream `julia:1.12-bookworm` base, it `COPY`s the worker source (`JuliaWorker.jl`), the mirror, and the handler package, then runs `Pkg.develop` on each path package and `Pkg.instantiate; Pkg.precompile`. Two `RUN` layers — one for `Pkg.instantiate` of the worker project + `EigeniusJuliaCommon`, a second for the mirror + handler develop+precompile (the order matters: handlers depend on `EigeniusMirror`, so the mirror must develop first).
4. **Builds via buildah, loads into Docker.** The substrate uses `buildah bud` for reproducible builds, then `buildah push docker-archive: → docker load` so the resulting image is reachable from the orchestrator's daemon under its content-addressed digest.
5. **Captures the runtime version.** Right after the build, `env build` runs `docker run --rm <digest> julia --version` and parses `julia version 1.12.6` out of the output. That patch-level pin is what gets committed in the next step — coarser tags like `1.12-bookworm` aren't reproducible enough; D26 §5.3 wants the exact version Julia actually runs at.

The whole step takes 30–90 seconds cold (most of it `Pkg.precompile`); subsequent rebuilds without input changes hit buildah's layer cache and finish in seconds.

## Step 6 — Commit the environment Resource

The image now exists in the local Docker daemon, but the chain doesn't know about it yet. `env create` commits a `RuntimeEnvironment` resource that **pins** the digest, version, lockfile, and lifecycle — the audit anchor that says "this image is canonical for this institution".

```bash
eigenius --endpoint http://localhost:50051 env create \
    --language julia \
    --handler-package julia/institutions/intervals/EigeniusIntervals \
    --mirror "urn:eigenius:runtime:mirror:julia:<hex>" \
    --as-iri "urn:eigenius:intervals:env:v1" \
    --image-digest "sha256:<digest>" \
    --runtime-version "1.12.6"
```

### What just happened

A `RuntimeEnvironment` resource was committed at the IRI you passed to `--as-iri`. Its required properties:

| Property | Value |
|---|---|
| `runtime:language` | `"julia"` |
| `runtime:runtime_version` | `"1.12.6"` |
| `runtime:lockfile` | the verbatim `Project.toml` (v1 — the full Manifest.toml lives inside the image; chain-side projection lands in a follow-up) |
| `runtime:lifecycle` | `urn:eigenius:runtime:lifecycle:Service` (long-lived, pool-backed worker) |
| `runtime:image_digest` | `sha256:<digest>` |

The lifecycle declaration (`Service`) is what tells the substrate to spin up a single long-lived worker per dispatch target rather than firing up a fresh container per call — Julia's startup cost makes a per-call lifecycle prohibitively expensive.

## Step 7 — Install the institution

Three resources go on the chain in one commit: the `Institution` itself (a name and a runtime kind), a `RuntimeMethodSignature` (the typed shape of the Julia function — `validate_bounded_by(BoundedBy) → Verdict`), and a `QueryClass` (the rule that says "fire `validate_bounded_by` on every `BoundedBy` that lands").

```bash
eigenius --endpoint http://localhost:50051 institution install \
    --definition julia/institutions/intervals/declarations/intervals-institution.eigon.json
```

```
Installed 3 resource(s). Layer: <hex>
```

### What just happened

The kernel's commit pipeline did three things on top of the standard validation:

1. **Indexed the `QueryClass`.** Its `dispatch_role` is `auto_on_load`, and its `query_class` field references `BoundedBy`. The kernel maintains an `auto_on_load_by_class` map from query-class IRI → list of QueryClass IRIs that fire on it; this commit added an entry pointing `BoundedBy` at `bounded_by_validity`. Future `BoundedBy` loads now know to dispatch this gate.
2. **Resolved the runtime environment reference.** The `Institution` resource's `requires_environment` property points at the `urn:eigenius:intervals:env:v1` we committed in step 6. Validation walked the parent chain to confirm that env exists; if it didn't, the install would have been rejected.
3. **Cross-checked types.** The `RuntimeMethodSignature.output_type` and `QueryClass.result_class` both reference `urn:eigenius:institution:Verdict`, which is an `InductiveType` (a sum type with three constructors: `Holds`, `Fails`, `Undecidable`). The validator confirmed those references resolve and have a permitted kind.

After this commit, the kernel knows: when a `BoundedBy` resource lands, run `validate_bounded_by` against it via the external Julia runtime in env `v1`, and treat the returned `Verdict` as the gating decision.

## Step 8 — Trigger an evaluation

Now load an instance of `BoundedBy`. Nothing here mentions Julia, the institution, or the verdict — it's just a typed Eigon resource:

```bash
cat > /tmp/obs.json <<'JSON'
[
  {
    "@id": "urn:eigenius:demo:intervals:obs:1",
    "urn:eigenius:core:is_a": ["urn:eigenius:intervals:BoundedBy"],
    "urn:eigenius:core:short_name": "obs1",
    "urn:eigenius:intervals:value": 2.0,
    "urn:eigenius:intervals:lower": 1.0,
    "urn:eigenius:intervals:upper": 3.0
  }
]
JSON

eigenius --endpoint http://localhost:50051 load /tmp/obs.json
```

```
Loaded 1 resource(s) into branch main. Layer: <hex>
```

### What just happened

A single Eigon-JSON load triggered a five-stage cross-process flow:

1. **Kernel validation.** The kernel parsed the JSON, confirmed the resource matches the `BoundedBy` class shape (three Floats, all required, present), and prepared a tentative new layer.
2. **AutoOnLoad gate fired.** The kernel's commit pipeline checked the `auto_on_load_by_class` index, found `bounded_by_validity` pointing at this resource's class, and issued a `DispatchExternal` gRPC call to the orchestrator with `(env_iri, image_digest, method_name, signature_iri, [input_cbor])`.
3. **Orchestrator → substrate.** The orchestrator's substrate addon (the napi-rs binding to `eigenius-runtime-substrate` running inside the Deno process) called `JuliaLanguageRuntime::call_method`. The runtime read `image_digest` from the synthesized env Resource, looked up its cached `ServiceHandle` for that digest (none yet — first dispatch), and asked `DockerServiceSpawner` to start a worker.
4. **Worker bootstrap.** The spawner ran `docker create` against the image, started the container, opened a Unix domain socket at the agreed depot path, and waited for the worker's `JuliaWorker.jl` to bind. On boot the worker scans `/opt/eigenius/packages/*/Project.toml` and `/opt/eigenius/mirror/Project.toml` and runs `using <Name>` for each — that's what populates the `_eigenius_decoders` registry with the mirror's `BoundedBy` decoder and brings the handler's `validate_bounded_by` into `Main`.
5. **Dispatch.** The substrate sent a `DispatchMethod` request over the UDS with the method name and the input CBOR. The worker decoded the input via the registered decoder, called `validate_bounded_by(BoundedBy(2.0, 1.0, 3.0))`, which uses `IntervalArithmetic.jl` to compute `2.0 ∈ interval(1.0, 3.0)` — rigorously, with rounding-mode discipline — and returned `Holds`. The substrate marshalled the verdict back as Eigon-CBOR.
6. **Kernel commits the verdict.** The orchestrator returned the verdict + a partial `RuntimeInvocation` resource (timing, image digest, numerical metadata captured by the worker's health check). The kernel committed both alongside the original `BoundedBy` instance in the same layer. If the verdict had been `Fails`, the entire load would have been **rejected** — that's the gating contract.

The cold dispatch (first call ever) takes ~3 seconds, dominated by the worker's Julia startup. The warm worker stays alive in the orchestrator's per-digest service cache; subsequent dispatches against the same env reuse it and complete in tens of milliseconds.

## Step 9 — Inspect the verdict

```bash
eigenius --endpoint http://localhost:50051 query \
    'MATCH "urn:eigenius:institution:Verdict"(?v) {
        "urn:eigenius:core:ctor_name": ?ctor
     } RETURN [] { verdict: ?v, ctor: ?ctor }'
```

The result includes:

```json
{
  "urn:eigenius:query:gen:<hex>:row:ctor": "Holds",
  "urn:eigenius:query:gen:<hex>:row:verdict": "urn:eigenius:invocation:<uuid>:verdict"
}
```

A single `Verdict` resource was committed on the same layer as the `BoundedBy` instance, with `ctor_name = "Holds"`. Inspecting it directly (`eigenius inspect <verdict-iri>`) shows the full structure — including a back-reference to the `RuntimeInvocation` resource that carries the dispatch trace (`started_at`, `completed_at`, `image_digest`, `numerical_metadata`).

To see the falsifying case, load an instance with `value: 5.0, lower: 1.0, upper: 3.0` and re-run the query. You'll see a second verdict with `ctor_name = "Fails"`, and the load will report:

```
Load failed:
  InstitutionValidation: AutoOnLoad QueryClass `bounded_by_validity` returned Fails
```

The `BoundedBy` instance was rejected; the chain is unchanged on `main`, but the `Verdict + RuntimeInvocation` audit trail still committed (on a side layer) so the rejection has a verifiable cause.

## What now lives on the chain

After running through this tutorial, the following resources are committed (all reachable via `eigenius inspect <iri>` against the running kernel):

| Resource | IRI | Source |
|---|---|---|
| `BoundedBy` class | `urn:eigenius:intervals:BoundedBy` | step 1 |
| `value` / `lower` / `upper` properties | `urn:eigenius:intervals:{value,lower,upper}` | step 1 |
| `RuntimePackageMirror` | `urn:eigenius:runtime:mirror:julia:<hex>` | step 3 |
| `RuntimeEnvironment` | `urn:eigenius:intervals:env:v1` | step 6 |
| `Institution` | `urn:eigenius:institutions:intervals` | step 7 |
| `RuntimeMethodSignature` | `urn:eigenius:intervals:signatures:validate_bounded_by` | step 7 |
| `QueryClass` | `urn:eigenius:intervals:query_classes:bounded_by_validity` | step 7 |
| `BoundedBy` instance(s) | `urn:eigenius:demo:intervals:obs:*` | step 8 |
| `Verdict` per instance | `urn:eigenius:invocation:<uuid>:verdict` | step 8 |
| `RuntimeInvocation` per instance | `urn:eigenius:invocation:<uuid>` | step 8 |

Together they form the full audit closure D31 prescribes: every Verdict points at the RuntimeInvocation that produced it; every Invocation pins the image digest, the runtime version, and the input/output content hashes; every image digest is reachable from a chain-committed `RuntimeEnvironment`; every `RuntimeEnvironment` references a chain-committed `RuntimePackageMirror`. Re-running the institution against the same inputs is reproducible by construction.

## Common failure modes

| Symptom | Cause | Fix |
|---|---|---|
| `seed manifest drift — refusing to boot` on `docker compose up` | The persistent DB volume was seeded with older embedded ontologies and they've changed since. | `docker compose down -v` to wipe the volume; `up -d` again. |
| `failed to construct DockerServiceSpawner` from `env build` | Docker daemon unreachable or the depot bind-mount in `docker-compose.yml` doesn't match the host path. | Check `/var/lib/eigenius/substrate-depot/` exists on the host and the orchestrator container can read+write it. |
| `manifest-hash mismatch` from the worker | The orchestrator container has a stale copy of `julia/runtime-worker/JuliaWorker.jl` while `eigenius env build` ran against the freshly-edited file on the host. | `docker compose build orchestrator` to refresh the bundled worker source, then `up -d`. |
| `no mirror decoder registered for class X` | The handler package's `Project.toml` doesn't list `EigeniusMirror` as a dep, so the worker's auto-`using` walk can't bring the decoder in. | Add `EigeniusMirror = "8a7b6c5d-4e3f-4a1b-9c8d-7e6f5a4b3c2d"` to `[deps]` and rebuild the env image. |
