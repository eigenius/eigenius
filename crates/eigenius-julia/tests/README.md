# `eigenius-julia` integration tests

Coverage for the Julia language-runtime crate's external surface, organised by what each suite anchors. The unit tests inside `src/mirror_gen.rs` etc. cover the generator's emit shape against golden snapshots; this directory is exclusively for tests that exercise the substrate's image-build pipeline, the worker's bootstrap, or the substrate's RPC dispatch path against a live Julia process.

## Map

| File | Anchors | Cost |
|---|---|---|
| [`mirror_regeneration_test.rs`](mirror_regeneration_test.rs) | The chain-side determinism guarantee D31 §3.3 makes: the same ontology layer regenerates a byte-identical mirror, and any class-shape edit produces a different mirror IRI. Cheap — chain → generator → assert; no Docker. | <100ms |
| [`mirror_image_build_integration.rs`](mirror_image_build_integration.rs) | Mirror Resource carries the substrate's required properties; the generated archive round-trips through the substrate's image-build pipeline byte-for-byte; a typed-mirror-struct handler dispatches via `CallRuntimeMethod` end-to-end (single-input, `Demo` test class). | ~1 min cold, ~30s warm |
| [`e2e_kinase.rs`](e2e_kinase.rs) | The plan's "kinase-grounded e2e" anchor. Multi-input typed dispatch (`Compound`, `Target`, `Target`) against the workspace's canonical kinase ontology; verifies `dispatched_to` carries the multi-arg `which()` shape; verifies warm dispatch is substantially faster than cold (the warm-pool guarantee). | ~25-90s |
| [`intervals_e2e_substrate.rs`](intervals_e2e_substrate.rs) | The IntervalArithmetic institution's substrate-side e2e (Phase 19a.6 stage 2a-iii). Mirror + handler package + env image + multi-input `dispatch_external_institution` with `BoundedBy` inputs, asserting `Holds`/`Fails` verdicts. | ~1-2 min cold |
| [`intervals_e2e_stage1.rs`](intervals_e2e_stage1.rs) | Chain-side install lifecycle for the IntervalArithmetic institution (Phase 19a.6 stage 1). Pure chain-state — no Docker. | <1s |

## Coverage held elsewhere

A few pieces of 19a's plan-level coverage live in the runtime-substrate crate's `tests/` rather than here, because they exercise the substrate trait surface and only incidentally use Julia:

- [`crates/runtime-substrate/tests/julia_capstone_integration.rs`](../../runtime-substrate/tests/julia_capstone_integration.rs) — the 18d capstone path against the production `JuliaLanguageRuntime`, including the cross-check tampering case. Phase 19a.8's "regression_18d_capstone" coverage lives here; we don't duplicate it under `eigenius-julia/tests/` because the test already targets the production crate.
- [`crates/runtime-substrate/tests/service_spawner_integration.rs`](../../runtime-substrate/tests/service_spawner_integration.rs) — service lifecycle (warm reuse, drain semantics, `ensure_service` idempotence) at the `LocalServiceSpawner` level. Cheap to run (uses the bash test worker, no Julia base image). Idempotence at the `DockerServiceSpawner` level is covered by the warm-reuse assertion in `e2e_kinase.rs`.

## Skip gates

Every Docker / buildah-dependent test in this directory shares the same skip discipline:

- **Docker socket unreachable** at `/var/run/docker.sock` → skip with a printed reason.
- **`buildah` not on PATH** → skip.
- **Julia base image not pullable** (offline / no registry access) → skip.

When skipped, tests print a single `eprintln!` line explaining why and return `Ok` so CI doesn't flake on hosts without the full toolchain. Local dev runs pick up everything when the dev box has Docker + buildah installed.

## Adding a new test

When the next institution ships, the natural shape is:

1. **Generator-only assertion** in `crates/eigenius-julia/src/mirror_gen.rs`'s tests module if it's about the emit shape.
2. **Substrate-only e2e** here, modelled on `intervals_e2e_substrate.rs` — mirror + handler package + image build + dispatch — when the goal is "the institution's handler dispatches end-to-end against the substrate". No kernel / orchestrator gRPC.
3. **Chain-side install lifecycle test** modelled on `intervals_e2e_stage1.rs` when the goal is "the chain commits the institution's resources, indexes the QueryClass, and AutoOnLoad fires on a matching commit".
4. **Full end-to-end demo** as a `demo/<institution>/run.sh` script (see [`demo/intervals/run.sh`](../../../demo/intervals/run.sh)) when the goal is "developer can drive the whole thing from `eigenius` CLI commands against the compose stack".

Use the existing IntervalArithmetic test trio as the reference template — they collectively cover every layer 19a.6's plan calls out.
