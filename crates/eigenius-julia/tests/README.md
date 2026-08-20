# `eigenius-julia` integration tests

Coverage for the Julia language-runtime crate's external surface, organised by what each suite anchors. The unit tests inside `src/mirror_gen.rs` etc. cover the generator's emit shape against golden snapshots; this directory is exclusively for tests that exercise the substrate's image-build pipeline, the worker's bootstrap, or the substrate's RPC dispatch path against a live Julia process.

## Map

Seventeen integration-test files plus a shared `common/` helper module. They
fall into three groups.

### Chain-validation (cheap, no Docker)

Load the declarations against a real chain, assert the resources commit and
the index resolves them. Each of the five institutions and each of the three
comorphisms has one.

| File | Anchors | Cost |
|---|---|---|
| [`catalyst_chain_validation.rs`](catalyst_chain_validation.rs) | Catalyst's `Institution`, signatures, `conservation_law_validity` and `qc_cat_to_ode` query classes, `ef_cat_to_ode_input`. | <1s |
| [`diffeq_chain_validation.rs`](diffeq_chain_validation.rs) | DiffEq's declarations, including `if_diffeq_problem`. | <1s |
| [`jump_highs_chain_validation.rs`](jump_highs_chain_validation.rs) | JuMP-HiGHS's declarations, including `if_jump_optimisation_problem`. | <1s |
| [`comorphism_chain_validation.rs`](comorphism_chain_validation.rs) | The Symbolics → IntervalArithmetic triple `(ef_symb_expr, m_id_formula_term, if_intv_function)`. | <1s |
| [`catalyst_to_diffeq_chain_validation.rs`](catalyst_to_diffeq_chain_validation.rs) | The Catalyst → DiffEq triple, middle `m_id_ode_problem`. | <1s |
| [`symbolics_to_jump_chain_validation.rs`](symbolics_to_jump_chain_validation.rs) | The Symbolics → JuMP triple, middle `m_id_optimisation_problem`. | <1s |
| [`intervals_e2e_stage1.rs`](intervals_e2e_stage1.rs) | Chain-side install lifecycle for IntervalArithmetic. Pure chain-state. | <1s |

### Mirror generator and marshalling

| File | Anchors | Cost |
|---|---|---|
| [`mirror_regeneration_test.rs`](mirror_regeneration_test.rs) | The chain-side determinism guarantee D31 §3.3 makes: the same ontology layer regenerates a byte-identical mirror, and any class-shape edit produces a different mirror IRI. No Docker. | <100ms |
| [`inductive_mirror_round_trip.rs`](inductive_mirror_round_trip.rs) | An inductive value survives encode → wire → decode against the generated per-ctor structs. | ~1 min |
| [`mirror_image_build_integration.rs`](mirror_image_build_integration.rs) | Mirror Resource carries the substrate's required properties; the generated archive round-trips through the image-build pipeline byte-for-byte; a typed-mirror-struct handler dispatches via `CallRuntimeMethod` end-to-end (single-input, `Demo` test class). | ~1 min cold, ~30s warm |

### End-to-end against a live worker

Every file in this group is `#[ignore]`d **and** gated on a Docker + buildah
probe, so a default `cargo test` runs none of them.

| File | Anchors | Cost |
|---|---|---|
| [`e2e_kinase.rs`](e2e_kinase.rs) | Multi-input typed dispatch (`Compound`, `Target`, `Target`) against the canonical kinase ontology; `dispatched_to` carries the multi-arg `which()` shape; warm dispatch is substantially faster than cold. | ~25-90s |
| [`intervals_e2e_substrate.rs`](intervals_e2e_substrate.rs) | IntervalArithmetic's substrate-side e2e: mirror + handler package + env image + multi-input `dispatch_external_institution` with `BoundedBy` inputs, asserting `Holds`/`Fails` verdicts. | ~1-2 min cold |
| [`intervals_on_demand_e2e.rs`](intervals_on_demand_e2e.rs) | The OnDemand kernel-side dispatch path — `qc_compute_bounds` via FIBER, returning a `BoundedBy`. | ~2 min cold |
| [`cross_institution_probe.rs`](cross_institution_probe.rs) | D32 §6 — the same `FormulaTerm` read by two institutions' walkers. The operational claim `symbolics_to_intervals`' declaration cites. | ~2 min cold |
| [`catalyst_to_diffeq_e2e.rs`](catalyst_to_diffeq_e2e.rs) | The Catalyst → DiffEq pipeline: `qc_cat_to_ode` compiles a network to a FormulaTerm-typed `OdeProblem`, DiffEq's gate re-integrates. | ~3 min cold |
| [`symbolics_to_jump_e2e.rs`](symbolics_to_jump_e2e.rs) | The Symbolics → JuMP pipeline: objective authored as a `SymbolicExpression`, framed, solved, re-validated. | ~3 min cold |
| [`jump_highs_e2e.rs`](jump_highs_e2e.rs) | JuMP-HiGHS LP and QP solves plus the `optimum_validity` gate on both. | ~2-3 min cold |

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
1b. **Chain-validation test** here, modelled on `diffeq_chain_validation.rs` — the cheapest useful coverage and the first thing to write: it catches a declaration that names a resource the chain can't resolve.
2. **Substrate-only e2e** here, modelled on `intervals_e2e_substrate.rs` — mirror + handler package + image build + dispatch — when the goal is "the institution's handler dispatches end-to-end against the substrate". No kernel / orchestrator gRPC.
3. **Chain-side install lifecycle test** modelled on `intervals_e2e_stage1.rs` when the goal is "the chain commits the institution's resources, indexes the QueryClass, and AutoOnLoad fires on a matching commit".
4. **Full end-to-end demo** as a `demo/<institution>/run.sh` script (see [`demo/intervals/run.sh`](../../../demo/intervals/run.sh)) when the goal is "developer can drive the whole thing from `eigenius` CLI commands against the compose stack".

Use the existing IntervalArithmetic trio (`intervals_e2e_stage1.rs`, `intervals_e2e_substrate.rs`, `intervals_on_demand_e2e.rs`) as the reference template — they collectively cover every layer 19a.6's plan calls out.

Keep this map in sync when you add a file. It went stale once already: it documented five files while seventeen were present.
