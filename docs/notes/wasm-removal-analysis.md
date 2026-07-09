# Removing WebAssembly support — scope analysis

Working note. Answers: what does it take to strip WASM component support, and
what can be removed. Read-only analysis — no code changed. Branch `more-nbe-cleanup`.

## TL;DR

- **WASM and institutions are separable.** WASM is *one of three* institution
  backends (`RuntimeKind::Wasm | External | InProcess`, `institution/registry.rs:55`).
  Removing it leaves the institution framework fully intact.
- **The "not used" premise is half right.** *WASM* is genuinely unused in
  production — **zero** ontologies declare `runtime: wasm` (only tests/demos
  exercise it). But *institutions* are heavily used and **load-bearing on the
  commit path** (AutoOnLoad Verdict gating) — they must **not** be removed.
- **The weight is `wasmtime`.** The kernel depends on `wasmtime` v45
  (`component-model`) **unconditionally** (no feature gate), pulling ~39–56
  wasmtime/Cranelift/wit crates — one of the heaviest subtrees in the workspace
  and a major compile-time cost.
- **Recommendation: remove WASM, keep institutions.** A bounded teardown
  (delete ~9k LOC + 21 MB fixtures + the Cranelift dep; edit ~6 shared files).
  The production institutions (Reasoning, Statistics, Lean in-process; Julia
  family external via Docker) don't touch WASM.

## Why this is safe — evidence

**Backend census across `ontologies/`** (grade: Observed):
`external` ×6 (Julia symbolics/jump/catalyst/diffeq/intervals + encoding),
`in_process` ×5 (Lean, Reasoning, Statistics + the two dock-assay examples),
**`wasm` ×0**. `runtimes:wasm` appears only as a *definition*
(`ontologies/institution/institution-ontology.json:149`), never used.

**Production institution wiring** (`cli/src/main.rs:2229` → `server/lifecycle.rs:457`):
the in-process vector is `[LeanInstitution, ReasoningInstitution, StatisticsInstitution]`
— all in-process Rust. The Julia family dispatches via `ExternalInstitution`
(`capability/external_institution.rs:104`) → gRPC → orchestrator → Docker
(`runtime-substrate`). None of these touch `wasmtime`.

**`WasmInstitution` is constructed only in tests/demos**: `wasm_institution.rs:379`
(unit test) and `kernel/tests/dock_assay_demo_wasm.rs:180`. The production WASM
registration pass (`server/hooks.rs:73`, `build_wasm_institution_runtime_indexed`)
runs on every commit but **returns an empty runtime** because no chain declares
a WASM institution.

**Institutions are load-bearing (do NOT remove):** commit AutoOnLoad gating
(`commit/phases.rs:411` — a `Fails` Verdict rejects the commit), FIBER
(`query/evaluate/fiber.rs:591`), `InstitutionInvoke`/`NativeDecide`
(`nbe/eval/mod.rs:422,458`), and the reasoning institution's
`validate_justification` ("Load-bearing — every committed ReasoningSentence
triggers it", `eigenius-reasoning/src/institution.rs:48`). These are on the
hot/core path, backend-agnostic.

## What WASM is here

The D12 "WASM extensibility" mechanism (`docs/design/d12-wasm-extensibility.md`,
status *Implemented*): third parties author sandboxed Component-Model binaries
that the kernel hosts for pure/read work and the orchestrator hosts for IO. It's
a real cross-cutting subsystem (kernel + orchestrator + guest SDK + WIT + examples),
just one nobody's shipping components against.

## Teardown inventory

### Delete wholesale (WASM-only)

| Target | Size | Notes |
|---|---|---|
| `kernel/src/capability/wasm_component.rs` | 248 LOC | wasmtime Component hosting (pure/read) |
| `kernel/src/capability/wasm_institution.rs` | 490 LOC | `WasmInstitution` host bridge |
| `kernel/src/capability/tests.rs` | 274 LOC | loads `doc_validator.wasm` (WASM-only) |
| `crates/wasm-runtime/` | ~160 LOC crate | shared wasmtime plumbing; deps: `wasmtime` |
| `sdk/wasm-sdk/` | ~1,099 LOC crate | guest SDK; consumed only by examples |
| `examples/wasm-*` (8 crates) | ~4,534 LOC | echo/dock/assay/arrhenius/cbor-echo/doc-validator/http-shout/read-query-probe (all workspace-excluded) |
| `wit/eigenius-component.wit` | 195 LOC | the only WIT file; all 4 worlds are WASM |
| `kernel/tests/fixtures/*.wasm` | 21 MB | 5 prebuilt component binaries |
| `kernel/tests/dock_assay_demo_wasm.rs` | 413 LOC | WASM mirror of `dock_assay_demo.rs` (non-WASM stays) |
| `orchestration/native/` | ~1,121 LOC | napi-rs + wasmtime IO-host addon (excluded) |
| `orchestration/src/wasm/` | 458 LOC | TS glue: loadAddon/registry/hostBridge/cbor |
| `demo/wasm/run.sh` | — | WASM extensibility demo |

### Edit (shared — WASM branch interleaved with backend-general code)

| File | Change |
|---|---|
| `kernel/src/capability/registration.rs` (1074) | Remove **most** of it — the whole WASM cluster: `scan_and_register` + `ScanResult`/`WasmComponent` (only scans `implementation="wasm"`; called solely from `server/hooks.rs`), `build_wasm_institution_runtime[_indexed]`, `build_wasm_runtime_from`, `load_wasm_institution`, `load_wasm_component`, `extract_wasm_bytes`, `decode_wasm_binary`, `base64_decode`, `classify_component_capability`, `extract_capability_level`, `extract_config`, and the `wasm_binary`/`wasm_binary_ref` fields. Keep only `validate_external_institution_chain`, `register_external_institutions`, `register_in_process_institutions`, and shared helpers (`resolve_via_layer`, `check_namespace`). |
| `kernel/src/server/hooks.rs` (494) | Drop the WASM passes in `rebuild_institution_index` (`:73`, `build_wasm_institution_runtime_indexed`) and the `scan_and_register` calls (`:145,:231`), plus `register_wasm_from_layer` / `rehydrate_wasm_*` / `register_io_wasm`. Keep external + in-process passes. |
| `kernel/src/commit/hooks.rs` (521) | Remove the `register_wasm_components` commit hook + its tests; unregister it from the hook pipeline. |
| `kernel/src/institution/registry.rs` | Drop the `RuntimeKind::Wasm` arm + its `parse_runtime_kind` mapping (leaves `External`/`InProcess`). |
| `proto/eigenius.proto` | Remove `ComponentExecutor.RegisterWasmComponent` RPC + `RegisterWasmComponentRequest/Response` + `RUNTIME_KIND_WASM = 2` (wire-compat note below). |

(Verified: there are **no** WASM term forms in `nbe/term.rs` — the `wasm_binary`/`wasm_binary_ref` payload handling lives entirely in `registration.rs`, so the AST is untouched.)
| `orchestration/src/main.ts` | Remove the `tryLoadWasmAddon`/`WasmComponentRegistry`/`createHostBridge` wiring (`:41-43,146-154`). |

### Dependency / build config

- Root `Cargo.toml`: drop `wasmtime` + `eigenius-wasm-runtime` from
  `[workspace.dependencies]`; remove `crates/wasm-runtime`, `sdk/wasm-sdk` from
  `members`; remove `examples/wasm-*` and `orchestration/native` from `exclude`.
- `kernel/Cargo.toml`: drop `wasmtime.workspace` + `eigenius-wasm-runtime.workspace`.
- `justfile`: delete the `build-wasm` recipe and its `orchestration/native` build;
  remove the `build-wasm` dependency from `build`/`build-gpu`/`build-metal`.
- `.github/workflows/ci.yml`: remove the wasm32 targets, `cargo-component` install,
  the "Build WASM fixtures" step, and the fixture cache. **Already broken** — it
  references `examples/wasm-d14-*`, which were renamed to `wasm-*`; this removal
  fixes the staleness by deletion.
- Docs: mark `docs/design/d12-wasm-extensibility.md` + `d12b-orchestrator-wasm-plan.md`
  superseded/removed; scrub WASM refs from D8/D56.

## What stays (do NOT touch)

The entire institution framework — `kernel/src/institution/{mod,runtime,registry,
dispatch,eval_hooks,marshal,in_process_registry,error}.rs`; the in-process backend;
the external backend (`capability/external_institution.rs`); `crates/runtime-substrate`
(Docker/OCI — **not** WASM); the Reasoning / Statistics / Lean / Julia / R crates;
comorphisms, FIBER, AutoOnLoad, InstitutionInvoke/NativeDecide. The item-6 hook
refactor (`EffectHooks`/`InstitutionEngine`) stays as-is — it's backend-agnostic.

## Risks & gotchas

- **`registration.rs` is the one genuinely interleaved file** — the WASM and
  External/in-process paths share `scan_and_register` and the `RegistrationReport`
  types. Split carefully; it's ~half the file (99 wasm/component-binary mentions).
- **Proto wire-compat.** Removing `RUNTIME_KIND_WASM = 2` and the RPC changes the
  generated bindings on both sides (kernel gRPC + orchestrator TS). Safe only if
  nothing persisted/serialized uses value `2`. Lower-risk alternative: keep the
  enum value reserved and delete only the RPC + handler. Confirm no chain data or
  orchestrator config references it before dropping.
- **The commit-hook pipeline** registers `register_wasm_components` by name in
  `commit/` wiring — removing the hook fn requires unregistering it, not just
  deleting the fn (else a dangling reference).
- **`in_process_registry.rs` lives in `institution/`, not `capability/`** — don't
  delete it with the capability WASM files; it's the backend that stays.
- **Legacy WIT world** `eigenius-institution` (the retired `FiberReasoner`) is in
  the same `.wit` file — it goes with the wholesale WIT deletion; verify nothing
  else binds it (the item-6 rename already confirmed no host binds it).

## Effort & sequencing

Bounded, mechanical, verifiable by `cargo test --workspace` staying green
(behavior-preserving — the WASM paths were dead in production):

1. **Kernel core** — split `registration.rs`, drop the WASM passes in
   `server/hooks.rs` + `commit/hooks.rs`, remove `RuntimeKind::Wasm`, delete
   `wasm_component.rs`/`wasm_institution.rs`/`capability/tests.rs`, remove the deps.
   Verify green (institutions still dispatch via external/in-process).
2. **Crates & examples** — delete `wasm-runtime`, `wasm-sdk`, `examples/wasm-*`,
   `wit/`, fixtures, `dock_assay_demo_wasm.rs`; fix workspace `members`/`exclude`.
3. **Orchestrator** — delete `orchestration/native/` + `orchestration/src/wasm/`,
   unwire `main.ts`; prune the proto RPC.
4. **Build/CI/docs** — `justfile`, `ci.yml`, D12/D12b.

Rough size: ~9,000 LOC + 21 MB fixtures deleted, ~6 files edited, `wasmtime`/
Cranelift subtree gone (the compile-time win). ~1 focused session for the kernel
core (step 1), the rest mechanical.

## Recommendation

Proceed with the WASM removal; keep institutions. The clean cut line is
`RuntimeKind::Wasm` — everything reachable only through it is removable, and the
framework's other two backends carry all real usage. Do step 1 first as its own
reviewable, green commit (it's where the interleaving risk lives); steps 2–4 are
mechanical follow-ups. If wire-compat on the proto enum is uncertain, reserve the
value rather than delete it.
