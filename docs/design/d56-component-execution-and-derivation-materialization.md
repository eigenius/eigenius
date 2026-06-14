# D56 — Component execution & derivation materialization

*Status: design (2026-06-13, revised same day). Establishes that a **component execution** (side-effecting, external — D26 language runtimes) becomes a chain-resident, witness-bearing derivation **through the existing program-execution subsystem** (`kernel/src/program`, the `RunProgram` RPC / `eigenius run`), not a bespoke "materializer." Distinguishes this from **institution recomputation** (pure, decidable, implicit at commit — D52/AutoOnLoad). Motivated by the WRN `concl_vivo` lift (wrapping lme4 via D55). References D6b (program trace), D8 (components), D26 (RuntimeInvocation), D41 (commit pipeline), D49 (ChainWitness), D52 (statistics institution), D54 (reasoning lemma citation), D55 (R runtime).*

> **Revision note.** The first draft proposed a standalone *materializer* + a *pending-RuntimeInvocation* marker as a new commit-adjacent operation. That was over-built. Eigenius already has the driver (program execution) and already mints the witness (`ProgramTrace → IsDerivedAs`). The corrected model below replaces it: a component execution is **a program invoking a RuntimeScript component**, and `eigenius run` is the demanded-execution driver. §3, §5, §6 carry the corrected design; §1–§2, §4 (the tension and the kind distinction) stand unchanged.

## 1. The tension

Two ways to put a computed warrant on the chain exist, and they look superficially alike — both end in an `IsDerivedAs` witness a reasoning certificate can `derived(RESULT, P)` against (D49/D54). But they are different epistemic kinds:

- **Institution recomputation (D52).** The judgment is **pure, total, decidable from chain-resident data.** A `SampleSet` + `AnalysisPlan` are fully declared; the statistics institution *re-derives* the verdict; identical inputs always yield identical results, no side effects. The kernel vouches *"I recomputed this."* Safe to fire **implicitly at commit** via AutoOnLoad (D41 §3.4), safe to register as a type-level `QueryClass`.

- **Component execution (D26 / D55).** Running limma / fgsea / lme4 (or any pinned external tool) is **side-effecting, external, on-demand.** The kernel does **not** re-derive it; it vouches only that the pinned image ran the declared script on the declared inputs and produced this output (D55 §2: *"faithful, reproducible execution of a pinned external tool"*). A genuinely weaker, *different* warrant grade.

The trap is making a component execution masquerade as an institution gate (an AutoOnLoad `QueryClass` whose handler shells out to R). That collapses the distinction three ways:

1. **It mislabels the warrant** — the `IsDerivedAs` would be indistinguishable from a kernel-recomputed one, silently upgrading *"we ran the authors' lme4"* to *"the kernel decided this."*
2. **It registers a one-off as a global capability** — institutions are keyed by `QueryClass`; every analysis script becomes a type-level extension.
3. **It couples chain loading to tool availability** — AutoOnLoad runs at commit; an external, possibly-unavailable execution must be *demanded*, never implicitly triggered by loading a file.

| | Institution recomputation | Component execution |
|---|---|---|
| Judgment | pure, total, decidable | side-effecting, external |
| Trigger | implicit at commit (AutoOnLoad) | **on-demand (`eigenius run`)** |
| Registration | type-level `QueryClass` | a component the program applies |
| Warrant grade | proven by re-derivation (D52) | faithful pinned execution (D55 §2) |
| Provenance anchor | `Verdict` | `ProgramTrace` + `RuntimeInvocation` + `ImageDigest` |
| Driver | the commit pipeline | the program-execution subsystem |

## 2. The asymmetry this exposes

For institutions, **AutoOnLoad is the driver** — it lives in the commit pipeline (D41) and chains *execution → derivation → witness* automatically on load. The corrected insight (this revision) is that component executions already have the symmetric driver: **program execution**. A program body applies a component; the run commits a `ProgramTrace` over the output; and the witness index already mints `IsDerivedAs` from that trace. The gap was never a missing operation — it was a missing *component kind* (§6).

## 3. The model: a RuntimeScript component in the program subsystem

`eigenius run program input` lands in `kernel/src/program/eval_io.rs::execute_program_nbe` (via the `RunProgram` RPC, `server/programs.rs`). It evaluates the program body (an EigenTT expression) in NbE **IO mode**, dispatching **components** (`ComponentRegistry`) and institutions, and commits the output + `ProgramTrace` + produced resources as a **program-run trace layer**.

Component execution today has two kinds (D8): **CompleteJson** (LLM, orchestrator-side, registered) and **WASM** (host-bridged). Both reach the orchestrator from the kernel via `RemoteComponent` (`kernel/src/program/remote.rs`) → `component_executor.ts`. The substrate's `run_script` is already exposed to the orchestrator (`runtime-substrate-native::dispatchRunRuntimeScript`, used by Julia/Lean) — but **no component kind routes a program's component invocation to it.**

D56 adds that third kind: a **RuntimeScript component**. A program body applies it to an input table (the component's static config — `component_argument` — names the `RuntimeScript` + `RuntimeEnvironment`); dispatch runs `run_script`; the returned Eigon `DerivedResource` carries the `canonical_proposition` the script set via `r_eigon_set_proposition` (D55). That output is the component's result, and the program commits it under a `ProgramTrace`.

### 3.1 The witness is already minted — no new machinery

`build_witness_index` (witness_index.rs) emits `IsDerivedAs(output_iri, P)` for any committed `ProgramTrace` whose output resource carries `canonical_proposition` (`emit_from_trace`, D49 §6 / D6b §6). So a program that runs the wrapped-R script produces — purely by committing its trace — exactly the witness a downstream `derived(output_iri, P)` certificate discharges against. **No `InstitutionEmittedDerivation` stamping, no pending-invocation marker, no second commit path.** The statistics institution's `IsDerivedAs` (via `InstitutionEmittedDerivation`) and the program's `IsDerivedAs` (via `ProgramTrace`) are two producers feeding the one witness index the reasoning checker reads.

### 3.2 Why the provenance difference is the point

At the **reasoning** layer the witness is uniform — `derived(RESULT, P)` discharges identically whether `RESULT` came from the statistics institution or a program run. At the **provenance** layer the program's derivation is anchored by a `ProgramTrace` + a `RuntimeInvocation` (script/input/output hashes + `image_digest` + timestamps), **not** a decidable `Verdict`. That records *how* the warrant was earned — pinned-executed, not re-derived — without weakening the proof term. Faking an institution would erase that signal.

## 4. How this composes with — and stays distinct from — AutoOnLoad

| Step | AutoOnLoad institution (D41/D52) | Program run (D56) |
|---|---|---|
| trigger | commit, every load | explicit (`eigenius run`) |
| produces | `Verdict` + `InstitutionEmittedDerivation`s | output + `ProgramTrace` + produced resources |
| witness source | `emit_from_institution_derivation` | `emit_from_trace` (`ProgramTrace`) |
| availability dependence | none (pure Rust) | requires the runtime's image/tool |

Both feed the single witness index consumed by D49/D54. D56 adds no consumer and changes neither the index nor the reasoning checker.

## 5. The "program" question — resolved

The first draft asked *what drives the chaining* and reached for a new operation. It's already answered: **program execution is the driver**, and `eigenius run` is its entry point. A notebook chains program runs interactively today; a server-side / headless notebook runner (a future capability) would schedule the same `RunProgram` calls. The DAG of component executions is expressed as ordinary program data flow (a component's input is another's output) — the evaluator already handles that, with trace memoization (D6b) giving idempotent re-runs.

## 6. The mechanism already exists: D26's `RuntimeScript` + `RunRuntimeScript`

An earlier draft of this section proposed a *new* component declaration (a `program:Component` with `implementation = "runtime"`, scanned into a `PendingRuntimeComponent`). That was a reinvention — D26 already specifies the whole surface, and most of it is already coded. Use it:

- **`RuntimeScript`** (D26 §5.1) is the graph resource carrying `source` + `requires_environment` (→ a `RuntimeEnvironment` with `image_digest` + `language`). Content-addressed; published via `eigenius script publish` (D26 §10). This *is* the "declared in the graph like an institution" shape — no new class.
- **`RunRuntimeScript`** (D26 §4.1) is the single, generic, IO-tagged substrate component that runs *any* `RuntimeScript`. It already exists end to end: the orchestrator handler (`orchestration/src/components/run_runtime_script.ts`), the napi bridge (`runtime-substrate-native::dispatch_run_runtime_script`), and the `SubstrateDispatcher` routing by `language` to a registered `LanguageRuntime`.
- A program invokes `RunRuntimeScript` with the input table as the component input and the `RuntimeScript` (+ env) as the component `argument`; `eigenius run` commits the `ProgramTrace` over the output → `IsDerivedAs` (§3.1). The output carries its `canonical_proposition` from `r_eigon_set_proposition` (D55).

So there is **no new component kind, no per-script registration, no kernel scan, no witness change.** The actual gaps are three small wirings + the encoding:

1. **Kernel** — add `urn:eigenius:program:components:RunRuntimeScript` to `REMOTE_COMPONENTS` (`server/lifecycle.rs`; today only CompleteText/CompleteJson/HttpRequest), so a program can dispatch it to the orchestrator like the other built-in remote components.
2. **Substrate** — register `RLanguageRuntime` in the napi `SubstrateDispatcher` (`runtime-substrate-native`), mirroring the Julia/Lean registration fns, so `language = "r"` resolves.
3. **CLI** — D26 §10's `eigenius script publish/run/list/inspect` surface (currently absent) is the home for publishing + managing runtime scripts — *not* `capability install` (which is the WASM-component/institution installer).

`RuntimeInvocation`/`ProgramTrace` already carry the provenance; the witness emitter already reads it.

## 7. Open questions (need deliberation before Phase B)

1. **Component-invocation surface.** How a program declares "apply the RuntimeScript component with this script + env" — reuse the existing `component_argument` mechanism (the script/env as static config) vs. a dedicated component-declaration shape. Lean toward `component_argument`, consistent with CompleteJson's prompt-template config.
2. **Determinism contract.** lme4/limma are deterministic; fgsea needs a pinned seed (D55 §5). The RuntimeScript component must refuse to mint a witness for a non-seed-pinned permutation tool — encode this as a component-level precondition, fail closed (D55 §11).
3. **Re-verification.** D55 §11 verify = re-run S on I in image D, check output hash H. Program-run trace memoization (D6b) already content-addresses; is re-verification just a cache-miss re-run with hash comparison, or an explicit verify mode? Likely the former.
4. **`bench:ToolArtifact` migration.** The WRN linked-external artifacts that become program-materializable (lme4, fgsea, limma) move to program-produced `DerivedResource`s; the genuinely non-recomputable ones (wet-lab IF foci) stay declared/agent-attested. Per the §8 decision, `concl_vivo` *replaces* its linked-external artifact rather than stacking additively.

## 8. The WRN `concl_vivo` lift (first consumer)

- A **program** whose body applies the `RunRuntimeScript` component to the chain-resident Fig 2d xenograft table (73 obs), with the lme4 model (`xenograft_lme4.rs`'s source) carried as a `RuntimeScript` (+ R `RuntimeEnvironment`) in the component `argument`.
- `eigenius run` executes lme4, commits the LRT-p `DerivedResource` carrying `canonical_proposition = onco:InVivoDependence("WRN","MSI")` under a `ProgramTrace`; the witness index mints `IsDerivedAs(vivo_result, InVivoDependence(...))`.
- A Declared bridge + a `concl_vivo` ReasoningSentence `derived(...)` it into the conclusion — identical shape to every recomputed conclusion in wrn-phase1-recompute-conclusions.esl. Per the §7.4 decision, the existing linked-external `wrn:vivo_xenograft` `bench:ToolArtifact` is **replaced** by the program-produced derivation.

The `r_eigon_set_proposition` primitive + its round-trip proof (`marshalling_round_trip.rs`), the real lme4 LRT (`xenograft_lme4.rs`), and the `ProgramTrace → IsDerivedAs` witness emission (`build_witness_index`) are each already verified independently. The lift's correctness therefore rests not on re-composing them in a test (that composition follows by construction — an early "Phase A" doing so was dropped as a tautology test that would have used non-production glue and de-risked nothing), but on building and running the **real** transport below and observing `concl_vivo` lift through an actual `eigenius run`.

## 9. Phasing

The `RunRuntimeScript` component + orchestrator handler + napi bridge + `SubstrateDispatcher` already exist (D26 §4.1). The remaining work is wiring R into them and authoring the WRN encoding; verification is a real `eigenius run` against the Docker Compose stack (kernel + Deno orchestrator + DooD substrate spawning the R container — the production `DockerServiceSpawner` path, which no in-repo test exercises).

- **B.2a (kernel)** — add `RunRuntimeScript` to `REMOTE_COMPONENTS` (`server/lifecycle.rs`). Rust.
- **B.2b (substrate)** — register `RLanguageRuntime` in `runtime-substrate-native`'s dispatcher, mirroring Julia/Lean. Rust + napi.
- **B.3 (WRN)** — the xenograft `RuntimeScript` (lme4 source + R `RuntimeEnvironment` w/ `image_digest`) + the input table + a program invoking `RunRuntimeScript`; replace `wrn:vivo_xenograft`; `concl_vivo` derives against the program output. Build + tag the R+lme4 image (D55 §6) so the DooD spawner can run it.
- **B.4 (verify)** — `docker compose build kernel orchestrator && docker compose up`; `eigenius run` the xenograft program; confirm the sibling R container runs lme4 and `concl_vivo` lifts. First real dispatch leg for a substrate-built R image (P3.2 built it + loaded the cdylib in-container but skipped dispatch under rootful docker).
- **B.5 (CLI)** — D26 §10's `eigenius script publish/run/list/inspect`.
- **Phase C** — generalize: GSEA (fgsea, seed-pinned) → `mech_rule` antecedent; D-DIFF (limma, Oxen-resolved inputs) → `dd_achilles`/`dd_drive`.
