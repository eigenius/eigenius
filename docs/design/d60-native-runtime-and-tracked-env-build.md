# D60 — Generic OCI tool runtime, kernel-tracked environment build, and the D57 generator lift (Level 2)

*Status: **implemented** (2026-06-20). Implements Level 2 of the D57 mechanical-evidence
plan ([d57-mechanical-evidence-plan.md](../notes/d57-mechanical-evidence-plan.md) §3):
run the schema.org generator as a program **through the kernel** so `concl_generator`
discharges `GeneratorConforms` via `derived(...)` instead of `declared(...)`. Builds on
D26 (runtime substrate), D55 (R runtime — the closest precedent), D56 (component
execution → `ProgramTrace → IsDerivedAs`), D53 (`PinnedExternalFile` provisioning).
References the user requirement (2026-06-20): the image build goes through the existing
`eigenius env build` CLI feature, and the build command/recipe is **kernel-tracked**
(on-chain), not an ad-hoc shell script.*

> **As built.** New crates `eigenius-schemaorg-worker` (the converter as a plain-Rust
> RPC worker — no FFI, sets its own `canonical_proposition`) and `eigenius-oci`
> (`OciToolRuntime` = `TestLanguageRuntimeDocker`'s build half + `eigenius-r`'s
> dispatch half, spawner-agnostic). `runtime:BuildRecipe` added (ontology + generic
> builder + `OciToolRuntime::build_recipe`). napi `register_oci_tool_runtime` + TS
> `registerOciToolRuntime` (`EIGENIUS_OCI_*`); `eigenius env build --language oci`;
> the orchestrator image stages the worker. `04-generator.esl`'s `concl_generator`
> now composes `observed(gen_input) ∧ observed(gen_output) ∧
> **derived(generate_result, GeneratorConforms)**`.
>
> **Verified — including the live clean-DB run.** `cargo test -p eigenius-oci --test
> oci_e2e -- --ignored` (real buildah image + sibling container + real conversion →
> report carries `GeneratorConforms("schema_org")`); `cargo test -p eigenius-schemaorg
> --test d57_chain_validates`; plus unit/bootstrap/clippy. **And end-to-end on a fresh
> DB** (`demo/d57-schema-org/run.sh`): `docker compose down -v && up` → `eigenius run`
> dispatches the generator through the `oci` runtime (the worker converts the real
> V30.0 vocabulary in a pinned sibling container), commits `generate_result` + a
> `ProgramTrace`, and the five conclusions — `concl_discipline`, `concl_probe`,
> **`concl_generator` (via `derived(...)`)**, `concl_cut`, **`concl_main` (the thesis)**
> — all return kernel-checked `Holds`. The chain is split into `04a-evidence.esl`
> (pre-run pins/rule) + `04b-conclusions.esl` (post-run), since `gen_input` must commit
> before the run. One deployment detail the live run surfaced: the `oci` image must be
> built from the *same* worker binary the orchestrator stages (boot cross-check, D26
> §9.3) — the demo `docker cp`s the staged binary before `eigenius env build`.

## 1. Goal

D57 Level 1 made the generator's *output* an Observed artifact and its conformance a
Declared-but-test-backed claim. Level 2 upgrades the conformance to a genuine **Derived**
witness: the kernel runs the real `eigenius-schemaorg` converter as a pinned, reproducible
program; the run commits a `ProgramTrace` over a `DerivedResource`; the witness index mints
`IsDerivedAs(result, GeneratorConforms("schema_org"))`; `concl_generator` cites it via
`derived(...)`. No new institution — the program-execution subsystem is the driver (D56 §2).

## 2. Epistemic classification (why program-run, not institution)

The generator is pure + deterministic, which superficially fits D52 *institution
recomputation* ("the kernel re-derived this"). We deliberately use the **D56 component-
execution** path instead, for three reasons:

1. **It reads an external 1.5 MB file** (`PinnedExternalFile`, provisioned by content
   hash) — side-effecting/external input, not chain-resident data. That is the
   component-execution profile (D56 §1), not the pure-from-chain institution profile.
2. **Keep domain logic out of the kernel.** The WRN family runs analysis via the generic
   `RunRuntimeScript`, never a bespoke kernel gate. A schema.org-specific institution or
   `BuiltinComponent` would cut against that (D56 §1 trap #2).
3. **The provenance is the point** (D56 §3.2): the warrant is anchored by a
   `RuntimeInvocation` (script/input/output hashes + `image_digest`) recording *pinned-
   executed*, not *re-derived*. Faithful to what actually happened.

The reasoning-layer grade is identical either way — `derived(result, P)` discharges the
same — but the provenance honestly records a pinned run.

## 3. The mechanism gap

The realized component-execution path (D56) dispatches `RunRuntimeScript` → orchestrator →
a substrate-spawned **language** container (r/julia/lean), each with a worker that bridges
Eigon-CBOR over UDS via an in-language FFI shim (R's `libeigenius_r_worker.so`, etc.).
**There is no runtime that runs an arbitrary pinned tool** — and pinning external tools is
exactly D56's model. So Level 2 adds one, and it should be **language-agnostic**: the unit
is a *container image*, not a Rust binary.

The seam is small and already proven: [`LanguageRuntime`](../../crates/runtime-substrate/src/language_runtime.rs)
(`language_id` / `build_environment_image` / `dockerfile_fragments` / `run_script` /
`call_method`). The substrate already provides the `buildah` image-build pipeline, the
`DockerSpawner`, content-hash file provisioning (D53 — how the WRN R worker reads the
1.6 GB `.rds`), and a build/spawn exemplar:
[`TestLanguageRuntimeDocker`](../../crates/runtime-substrate/src/test_runtime_docker.rs)
builds a real OCI image, spawns a container, and dispatches.

## 4. Design

### 4.1 A generic `oci` tool runtime (not Rust-specific)

The new runtime runs **anything packageable into a container image** — a Rust binary, a
Python script, a C program, a shell pipeline. The tool stays *vanilla*; the **substrate**
does the Eigon marshalling. This is strictly more general than a per-language worker and is
the faithful "pinned external tool" model (D56 §1): the reproducibility anchor is the whole
**`image_digest`**, not a per-binary manifest-hash.

- **`OciToolRuntime` : `LanguageRuntime`** (in `runtime-substrate`, generalizing
  `TestLanguageRuntimeDocker` out of the test gate): `language_id() = "oci"`.
  - **Build** — `build_environment_image` bakes whatever the `BuildRecipe` (§4.2) declares
    (e.g. `COPY` the cargo-built `schemaorg-import` over a `debian:bookworm-slim` base).
  - **Dispatch** — the kernel/substrate **invokes the tool inside the pinned image as a
    one-shot program** (run-to-completion, the `lifecycle:Job` the ontology already
    defines). The substrate provisions each input `PinnedExternalFile` by `content_hash`
    into the container and runs the declared command; **the tool returns its result as
    Eigon-CBOR** (the same wire format the R/Julia workers already use) — `inputs → CBOR
    result resource(s)`, nothing more.
- **Division of labor.** The tool is a *pure transform*: read inputs, emit the result
  resource(s) as Eigon-CBOR. **The kernel applies everything else** — it commits the
  output + the `ProgramTrace` + `RuntimeInvocation` (script/input/output hashes +
  `image_digest`), the witness index mints `IsDerivedAs`, and any AutoOnLoad institution
  **verdicts** fire on commit. The tool never touches traces, witnesses, or verdicts.
- **The invocation contract** (on a `RuntimeScript`/`ToolInvocation`): `command` (argv
  template) + input bindings (`PinnedExternalFile` → path/arg). The result comes back as
  CBOR; the kernel wraps it.
- **The proposition is invocation-declared, kernel-stamped** (the generic path). The
  `canonical_proposition` is the D47-encoding of `GeneratorConforms("schema_org")` — a
  *Prop*, not data. A generic containerized tool cannot build it (it would have to embed
  the kernel's `Exp` AST + the `obj:d57` IRI — defeating genericity; R only manages it
  because its worker links the kernel via FFI). So the **ESL chain declares it** on the
  invocation/program (`type_expr( obj:GeneratorConforms("schema_org") )`, which ESL already
  compiles to the D47 value, `kernel/src/program/eigentt_type_mirror.rs::encode_type`), and
  the **kernel stamps it onto the tool's result** before committing the `ProgramTrace`, so
  the witness index reads it. The tool returns only the **bare data** (coverage + output
  `content_hash`) as Eigon-CBOR.
  - *Kernel feature this implies (P2/P4):* the program-run path stamps the
    invocation-declared `canonical_proposition` onto the output resource. Today the R path
    relies on the worker setting it (`r_eigon_set_proposition`); the `oci` path moves that
    to the kernel, which is also where "the kernel applies traces + verdicts" lands.
  - *Optional:* an Eigenius-aware Rust tool linking the kernel may still set the proposition
    itself; for genericity schema.org uses the invocation-declared path.
- **schema.org tool change** is small: an output mode on `schemaorg-import` that builds the
  conversion-report `Resource` (coverage + output `content_hash`) and writes it as
  **Eigon-CBOR** (stdout / a result fd). Because the tool is Rust linking `eigenius-kernel`
  it serializes Eigon-CBOR via `eigon_cbor::serialize_resource` directly — no FFI.
- **napi + orchestrator**: `register_oci_tool_runtime(...)` in `runtime-substrate-native`,
  registered at orchestrator startup alongside r/julia/lean.

The existing per-language runtimes (worker-RPC + Eigon mirrors, for tight typed
`CallRuntimeMethod` integration) are unchanged; `oci` is the complementary one-shot path.

### 4.2 Kernel-tracked environment build recipe (the user's requirement)

Today only `lockfile` + `image_digest` are on-chain; the Dockerfile/recipe lives in
compiled-in per-language code + ad-hoc scripts (`demo/wrn-helicase/build-r-image.sh`). The
build is reproducible-in-principle but the *recipe is not auditable from the chain*. Add a
**`runtime:BuildRecipe`** (or `runtime:build_recipe` field on `RuntimeEnvironment`) carrying:

- `base_image` (digest-pinned),
- `artifact_hashes` (SHA-256 of each baked-in artifact — e.g. the `schemaorg-import`
  binary; the cross-check anchor),
- `dockerfile` (the composed Dockerfile text, or its content hash + the fragment inputs),
- `build_command` (the exact `eigenius env build …` invocation that produced the image),
- `builder` + `builder_version` (e.g. `buildah` + version).

`eigenius env build` **emits** the recipe and commits it with the `RuntimeEnvironment`;
`eigenius env build --verify <env>` (or the worker bootstrap cross-check, D26 §9.3) can
**reproduce the digest from the recipe** and fail closed on drift. This makes "how the
image was built" a chain-resident, content-verified fact — the same discipline D57 applied
to the generator's input/output, now applied to the *environment* it runs in. It
generalizes to every runtime (r/julia/lean *and* `oci`), not just this one.

### 4.3 The D57 chain encoding (replaces the Level-1 Declared leg)

On the `obj-d57` chain:

- `obj:schemaorg_env : runtime:RuntimeEnvironment` (`language="oci"`, the pinned
  schema.org converter image, `image_digest`, `build_recipe`).
- `obj:schemaorg_script : runtime:RuntimeScript` (`requires_environment = schemaorg_env`,
  source = the command + file-I/O contract).
- `obj:generate_program : program:Program` whose body applies `RunRuntimeScript` to
  `obj:gen_input` (the existing Observed input pin) with `schemaorg_script` as the
  component argument.
- `eigenius run` (or `eigenius script run`) executes it → commits `obj:generate_result`
  (`DerivedResource`, `canonical_proposition = GeneratorConforms("schema_org")`) under a
  `ProgramTrace` → `IsDerivedAs`.
- `04-generator.esl`: `concl_generator` now composes `observed(gen_input) ∧
  observed(gen_output) ∧ **derived(generate_result, GeneratorConforms)**` — the Level-1
  `declared(generator_checks)` leg becomes `derived(...)`. The cargo test suite stays as
  the CI-enforced cross-check; the on-chain warrant is now kernel-attested.

## 5. CLI surface (reuse, don't reinvent)

- `eigenius env build --language oci …` (baking the `schemaorg-import` binary per the
  recipe) → builds the image, emits the `BuildRecipe`, prints `image_digest` + recipe IRI.
- `eigenius env create --language oci --image-digest … --recipe …` → commits the env.
- `eigenius script publish` / `eigenius script run <script> --inputs <gen_input>` → the
  D26 §10 path, already wired for r/julia/lean.

## 6. Verification

Docker + buildah + compose are available in the dev environment, so the lift is verified
end-to-end exactly as WRN's `concl_vivo` was (D56 §9):

1. `eigenius env build --language native …` builds the schema.org worker image; the recipe
   commits; `--verify` reproduces the digest.
2. Clean-DB `docker compose` up (kernel + orchestrator + substrate).
3. `eigenius run` dispatches the generate program to the spawned `oci` tool container;
   observe `obj:generate_result` + its `ProgramTrace` commit and `IsDerivedAs` mint.
4. Load `04-generator.esl` with the `derived(...)` certificate; `concl_generator` Holds.
   The `d57_chain_validates` harness is extended to assert the derived discharge once the
   trace is present (or a focused integration test drives the real `eigenius run`).

Where a leg can be exercised in-process (the converter, the worker's convert+propose
round-trip), keep a `cargo test`; the genuine Derived lift is the docker e2e.

## 7. Phasing

- **P1 — tool output mode.** Add an Eigon-CBOR result mode to `schemaorg-import`: build the
  conversion-report `DerivedResource` (coverage + output `content_hash` +
  `canonical_proposition = GeneratorConforms`) and emit it as Eigon-CBOR. In-process test
  (no docker): convert → build report → CBOR round-trip → assert the proposition decodes.
- **P2 — `oci` tool runtime.** `OciToolRuntime : LanguageRuntime` (command + file-I/O
  dispatch, `lifecycle:Job`), generalized from `TestLanguageRuntimeDocker`; napi +
  orchestrator registration; substrate e2e test (docker).
- **P3 — tracked build recipe.** `runtime:BuildRecipe` ontology + `eigenius env build`
  emits/commits it + `--verify`.
- **P4 — D57 encoding + lift.** The env/script/program resources; run; re-discharge
  `concl_generator` via `derived(...)`; verify the chain Holds.

## 8. Decisions needed before P1

1. **Proposition assignment** — *invocation-declared + kernel-stamped* (generic; the tool
   emits bare data) vs. *tool-set* (only for kernel-linked Eigenius-aware tools).
   *Recommend: invocation-declared for schema.org and the generic `oci` contract; tool-set
   stays an optional path.* Implies the small kernel feature in §4.1.
2. **`oci` runtime scope** — generic "run anything packaged into a container image"
   (command + file-I/O contract) vs. a narrower Rust-worker runtime. *Recommend: generic
   `oci` (this section's design).*
3. **Build recipe shape** — a first-class `runtime:BuildRecipe` resource referenced by
   `RuntimeEnvironment`, vs. inline fields on `RuntimeEnvironment`. *Recommend: a
   first-class resource* (content-addressed, reusable across envs, queryable).
