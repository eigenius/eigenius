# D41 — Kernel Commit Pipeline

**Status:** Implemented (2026-05-30; `kernel/src/commit/` — CommitPipeline + CommitOrchestrator + LayerPersister live, all six commit-shaped RPCs routed through it)
**Phase:** structural refactor of the kernel commit path
**Companion docs:** D14 (AutoOnLoad institutions and `Verdict`/`RuntimeInvocation` provenance), D23 §5.4 (`update_branch`, `ConflictPolicy`, `UpdateOutcome`), D25 (chain consolidation; uses the same persist boundary), D31 (institution dispatch surface), D33 §6 (anchored-commit cache — the cache probe currently lives in `persist_layer_if_backend`), D34 §G (the gaps in trivial-merge surfacing and `branch_advanced` plumbing that motivated the unification)
**Supersedes:** the implicit "commit is whatever each module does" assumption that runs through `kernel/src/context/mod.rs`, `kernel/src/lattice.rs`, and the Load handler in `kernel/src/server/mod.rs`

## 1. Motivation

A *commit-shaped RPC* is any RPC that ends by materialising a new layer on a branch. Today the platform has six of them — `Load`, `RunProgram`, `RunProgramByIri`, `Reflect`, `Query` (INTO), `SubmitResolution`, `CapabilityInstall` — and each is responsible for the same conceptual pipeline. That pipeline currently has no module home. Its phases are smeared across three modules and the gRPC handler:

- `commit_with_validation` in [`kernel/src/context/mod.rs:313`](../../kernel/src/context/mod.rs) does build + structural validate + AutoOnLoad institution dispatch.
- `commit_layer` in [`kernel/src/lattice.rs:496`](../../kernel/src/lattice.rs) (the newer one with `CommitPolicy` / retroactive validation / cascade) does build + structural validate + retroactive + cascade + `backend.store_layer`.
- `persist_layer_if_backend` in [`kernel/src/server/mod.rs:898`](../../kernel/src/server/mod.rs) does the anchored-commit cache probe (D33 §6) + `backend.store_layer` + branch CAS.
- The Load handler itself ([`kernel/src/server/mod.rs:2054`](../../kernel/src/server/mod.rs)) does WASM-component-registration + institution-index rebuild + roughly 250 lines of revert-on-not-advanced state-machine bookkeeping coordinating user / provenance / institution-classes layer persists.

The symptoms confirming this is structural, not aesthetic:

- **Two distinct `CommitOutcome` types** ([`lattice.rs:196`](../../kernel/src/lattice.rs), [`context/mod.rs:102`](../../kernel/src/context/mod.rs)) for the same conceptual thing. The lattice one carries cascade tombstones; the context one carries the optional provenance follow-up layer and the prior head needed for revert. Neither subsumes the other.
- **Two `backend.store_layer` callers** fighting over who owns persist. `commit_layer` writes through the backend directly; `persist_layer_if_backend` also writes through the backend. The Load handler picks the second; nothing reaches the first today.
- **Three near-identical revert blocks** (user / provenance / institution_classes) in the Load handler, each implementing the same "if the branch didn't advance, walk `ctx.head` back to the prior layer" pattern.
- **`commit_layer` is unreachable from any RPC.** The retroactive validation and cascade machinery built into `commit_layer` is dead code on the RPC paths — `Load` goes through `commit_with_validation`, which doesn't know about retroactive validation. Retroactive validation is currently only invoked by tests and the `commit_layer_default` helper.
- **No home for institutional retroactive cascade.** AutoOnLoad institution dispatch runs only on the new layer's resources. The same "lower-layer impact of a new declaration" question that drove structural retroactive validation also applies to institutional gates. There is no place in the current shape to put that work — the structural-validation cascade and the institution dispatch live in different modules with no shared sequencing primitive.

This is a structural smear, not three local cleanups. The fix is not to add a guard or coalesce two of the three; it is to give the commit pipeline a single home with explicit ordering and explicit phase boundaries, so that:

- Each phase is named and reachable from a single call site.
- The handler shrinks to a translator between RPC types and kernel types.
- Adding a new phase (e.g. institutional retroactive cascade) means appending to one shape, not threading state through three modules.
- The two `CommitOutcome` shapes collapse into one.
- `commit_layer`'s retroactive machinery becomes reachable from every commit-shaped RPC that wants it.

## 2. Two-level structure

The pipeline factors into two levels, each with one job:

### 2.1 `CommitPipeline` — one layer

Runs a single commit. Phases are stored as `&'static [Phase]` data — zero allocation, four canned shapes. The pipeline reads from a `LayerBuilder` and writes to a `CommitState` arena, returning a `LayerCommitOutcome`. It does not know about emissions in any active sense; it only knows that phases may append to `state.emissions` for the orchestrator to drain later.

### 2.2 `CommitOrchestrator` — multi-layer routing

Owns the FIFO emission drain. Every commit-shaped RPC goes `handler → orchestrator → pipeline`, including the single-layer ones. A `Query INTO` that produces no emissions runs through the orchestrator as the degenerate case (one pipeline run, empty emission queue, returns immediately). This makes the handler shape uniform: build a root `LayerEmission` from the RPC inputs, call `orchestrator.run(root)`, translate the `MultiLayerOutcome` back into RPC response fields.

The boundary between the two levels is **single-layer correctness** (pipeline) versus **multi-layer ordering, revert, and depth-capping** (orchestrator). The pipeline never sees the queue; the orchestrator never sees a phase.

## 3. Phases and hooks

The commit pipeline distinguishes two kinds of work: **phases**, which run before persist and can abort the commit, and **hooks**, which run after a successful persist and cannot. Hooks exist because some work — registering WASM components, rebuilding the institution dispatch index — is logically "post-commit": it observes a layer that is already on disk and updates kernel-side caches/runtimes accordingly. Folding that work into the pre-persist phase list would create the false impression that it can unwind the commit; it cannot.

| | Phases | `didPersist` hooks | `didDrain` hooks |
|---|---|---|---|
| When | Before persist | After successful persist (per pipeline) | After drain completes (per orchestrator) |
| Can abort commit? | Yes (return `Err`) | No — layer is on disk | No — all layers landed |
| Error handling | Unwinds | Best-effort: collected, surfaced, commit stands | Same |
| Can emit follow-ups? | Yes | Yes (push to `state.emissions`) | No — drain is over |
| Can side-effect kernel state? | Discouraged (might unwind) | Yes | Yes |

Phases are free functions, not trait objects. Each phase reads and writes named fields of `CommitState`. The signature:

```rust
type Phase = fn(&mut CommitState<'_>) -> Result<PhaseControl, CommitError>;

pub enum PhaseControl {
    Continue,
    /// The builder is empty (no resources, no tombstones). The pipeline
    /// returns `LayerCommitOutcome::Skipped` immediately; subsequent
    /// phases do not run. Distinguished from `Continue` so callers can
    /// tell "we ran but the layer was a no-op" apart from "we ran and
    /// landed a layer."
    SkipEmptyCommit,
}
```

All phases live in `kernel/src/commit/phases.rs`. The phase set:

### 3.1 `build`

- **Reads:** `state.builder` (consumed).
- **Writes:** `state.layer` (`Some(Arc<Layer>)`).
- **Emits:** nothing.
- Materialises the `LayerBuilder` into an `Arc<Layer>`. The cascade phase, if present, may rebuild the layer later from a cloned builder; `build` only handles the first construction. Returns `SkipEmptyCommit` if the builder is empty.

### 3.2 `structural_validate`

- **Reads:** `state.layer`, `state.storage`.
- **Writes:** nothing on the happy path; returns `CommitError::Validation { errors, total_violations }` otherwise.
- **Emits:** nothing.
- Runs `Validator::validate` against the just-built layer. This is the structural check (referential integrity, type shape, constraint satisfaction at the level of `Decidable-QC`).
- **Scope.** This phase runs only on pipelines that accept **user-authored content**: `structural_only`, `with_retroactive`, `with_institutions`. It is deliberately omitted from `structural_followup`, whose layers carry kernel-emitted content (`verdict_provenance`, `institution_classes`) whose well-formedness is guaranteed by the emitter. See §5 for the rationale.

### 3.3 `retroactive_with_cascade`

- **Reads:** `state.layer`, `state.policy`, `state.storage`.
- **Writes:** rebuilds `state.layer` if the cascade tombstoned IRIs; appends to `state.cascade_tombstones`; increments `state.cascade_iterations`.
- **Emits:** nothing. The cascade is internal — the outer pipeline does not see individual iterations.
- Runs the fixpoint loop currently spelled `commit_cascade_path` in `lattice.rs`. Each iteration: probe lower layers for retroactive constraint violations against the new layer's declarations; if `policy` is `CascadeTombstone`, tombstone the offenders and rebuild the layer with the tombstones; iterate until the fixpoint is reached. Under `policy: Reject`, the phase fails the first time it finds a retroactive violation.
- The cascade does not produce emissions; it produces a different *content* for the same layer being committed.

### 3.4 `autoonload_dispatch`

- **Reads:** `state.layer`, `state.institutions`, `state.storage`.
- **Writes:** `state.dispatched_verdicts`, `state.provenance_resources`.
- **Emits:** queues exactly one `LayerEmission { name: "verdict_provenance", pipeline: StructuralFollowup, kind: EmissionKind::Sibling, resources, tombstones: empty }` whenever any verdict was produced — **regardless of reading** (Holds, Undecidable, *and* Fails). The emission is queued *before* the phase decides whether to return `Ok` or `Err`.
- The D14 / D31 institution gate. For each AutoOnLoad institution declaration covering an IRI committed by the new layer, dispatches the gate; for every dispatch that produced a verdict (any reading), generates a `RuntimeInvocation` + `Verdict` resource pair into a local accumulator that becomes the emission's `resources`. The accumulator also flows into `state.dispatched_verdicts` for surfacing to the handler.
- Returns `Err(CommitError::Validation { errors, total_violations })` if any verdict was `Fails`, where `errors` carries one entry per `Fails` dispatch. Returns `Ok(Continue)` otherwise.
- Phase is absent unless `state.institutions` is `Some` — the pipeline kind controls whether it runs.

The structural framing here matters and supersedes the earlier draft. AutoOnLoad does not produce a "rejection provenance layer" only on `Fails`. Per D31 §6.3, AutoOnLoad **always** produces verdict provenance — one `RuntimeInvocation` + `Verdict` resource pair per dispatched QueryClass, for every verdict reading. The split is in routing, not in production:

- On `Holds` / `Undecidable`: the audit layer commits as a follow-up to the user layer.
- On `Fails`: the audit layer commits *in lieu of* the user layer — the user layer is rejected, the audit lands anyway.

Both are the same emission, queued the same way, with the same `EmissionKind::Sibling`. The orchestrator's drain loop is what makes the routing fall out naturally (§6.1): `Sibling` emissions are preserved when their queuing pipeline returns `Err`, while `Child` emissions are discarded. The phase itself only has to do one thing: queue the audit unconditionally and let the drain do the routing.

**On the shape of `CommitError`.** The `autoonload_dispatch` phase's `Err` does not carry the audit layer; the audit travels via `Sibling` emission. This keeps `CommitError` shape narrow and routes provenance through the same channel as all other follow-up layers. There is no `CommitError::ValidationFailed { provenance_layer }` variant — the lattice's existing `CommitError::Validation { errors, total_violations }` is sufficient because audit routing is orthogonal to the error: `Sibling` emissions carry the audit, the error carries the `Fails`. Phase 2 of D41 implementation re-exports `lattice::CommitError` from `commit::Error` unchanged.

**On the audit Sibling's pipeline.** The queued `verdict_provenance` Sibling specifies `pipeline: PipelineKind::StructuralFollowup`. Because `StructuralFollowup` omits `structural_validate` (§5), the drain run that lands the audit layer will **not** re-validate the `Verdict` + `RuntimeInvocation` resources produced here. The well-formedness contract for those resources lives at the emitter — `build_verdict_resource` and `build_runtime_invocation_resource` are jointly responsible for producing resources whose `is_a`, properties, and constraint shape are consistent with the institution ontology declarations of `Verdict` and `RuntimeInvocation`. The phase rebuild therefore needs no additional ontology accommodation for audit-resource shape; if a future audit-emitter change produces ill-formed content, the fix is at the emitter, not by re-enabling validation here.

**Why `Sibling` is the only emission kind warranting always-commit content.** Of the seven rejection causes the pipeline can produce (structural-validation failure, retroactive violation, cascade abort, AutoOnLoad `Fails`, persist I/O error, `NeedsWitnessedMerge`, pipeline-internal error), only AutoOnLoad `Fails` produces durable institutional facts worth anchoring to the chain. The other six produce only error responses to the caller. Institutional dispatch creates the fact "institution X dispatched against subject Y, returned `Fails` because Z, at time T" — and future chain inspection (audit queries, retroactive replays, regulatory review) wants that fact present regardless of whether the user-layer commit succeeded. User errors, chain-incompatibility errors, I/O, and concurrency taxonomy don't carry that property.

### 3.5 `persist`

- **Reads:** `state.layer`, `state.persist`, `state.branch`.
- **Writes:** `state.persisted` (`PersistedLayerInfo`).
- **Emits:** nothing.
- Calls `LayerPersister::persist(branch, &layer)`. The persister's body is today's `persist_layer_if_backend`: anchored-commit cache probe (D33 §6) → `backend.store_layer` → branch CAS. The phase does not interpret the result; the orchestrator does (`PersistedLayerInfo.branch_advanced` controls whether subsequent emissions are queued or descendants are dropped).

The phase set is deliberately small. Each phase has exactly one purpose and one place to read about it. Work that has to observe a successfully-persisted layer lives in hooks, covered next.

### 3.6 Hooks

Hooks are the post-persist counterpart to phases. They run with a layer that is already on disk, so they cannot abort the commit; errors they raise are surfaced to the caller but the commit stands.

**Signatures.**

```rust
type DidPersistHook = fn(&mut CommitState<'_>) -> HookOutcome;
type DidDrainHook   = fn(&mut DrainState<'_>) -> HookOutcome;

pub struct HookOutcome {
    pub errors: Vec<ValidationError>,   // surfaced but non-unwinding
    // didPersist pushes emissions via &mut CommitState; didDrain can't emit.
}
```

`didPersist` hooks live alongside the phase list on `CommitPipeline` and run after the `persist` phase has populated `state.persisted` — but only when the persist actually advanced the branch (§6.1). The hook receives the same `CommitState` the phases used, so it can read the just-persisted layer and push follow-up `LayerEmission`s onto `state.emissions` for the orchestrator to drain.

`didDrain` hooks live on `CommitOrchestrator` and run once, after the FIFO drain loop finishes, against a `DrainState` carrying the `MultiLayerOutcome` accumulated so far plus the final top layer. They cannot emit — the drain is over — but they can mutate kernel state derived from the full set of landed layers.

**Hook-specific state.** `CommitState` grows a `hook_errors: Vec<ValidationError>` accumulator (§4) for errors raised by `didPersist` hooks; these flow into `LayerCommitOutcome.hook_errors`. The orchestrator carries an analogous `drain_hook_errors` that flows into `MultiLayerOutcome.drain_hook_errors`.

**Concrete hooks today.**

- `register_wasm_components` — `didPersist` hook on the `with_institutions` pipeline. Reads the just-persisted user layer (the WASM components are part of its content), registers components against the institution runtime, and queues a `LayerEmission { name: "institution_classes", pipeline: StructuralFollowup, kind: EmissionKind::Child, … }` carrying the registered classes for the institution-classes follow-up layer. Lifts the logic currently in `register_wasm_from_layer` ([`kernel/src/server/mod.rs:2201`](../../kernel/src/server/mod.rs)). The handler today calls `register_wasm_from_layer` and manually constructs the institution-classes follow-up layer; under D41 the hook does both. Errors registering components are recorded on `state.hook_errors`; the user-layer commit stands either way.

  Because the queued emission specifies `pipeline: StructuralFollowup`, the institution-classes drain run will not re-validate its content (§5). The resources in that emission are produced by the WASM-registration extraction inside the hook; that extraction is responsible for emitting well-formed institution-class resources. If `register_wasm_from_layer` (or its successor inside the hook) is ever changed to produce resources whose shape the chain ontology cannot represent, that is a contract violation at the extractor — fix the extractor, not the pipeline.

- `rebuild_institution_index` — `didDrain` hook on the orchestrator. Runs once after the FIFO drain completes, with the final top layer in hand. Replaces today's three intra-Load rebuild calls ([`server/mod.rs:2191`](../../kernel/src/server/mod.rs), [`server/mod.rs:2197`](../../kernel/src/server/mod.rs), [`server/mod.rs:2232`](../../kernel/src/server/mod.rs)). The collapse from three rebuilds to one is semantically equivalent because nothing inside a single Load actually consumes the rebuilt index; only the *next* RPC's `InstitutionContext` snapshot reads it. Errors here are recorded on `MultiLayerOutcome.drain_hook_errors`.

**Why best-effort, non-unwinding is structurally correct.** Once `persist` has returned `branch_advanced = true`, the layer is on disk and reachable from the branch tip. Treating a downstream registration failure as a commit failure would either (a) lie to the storage layer by pretending the layer isn't there, or (b) require a transactional retract that the persistent backend does not support. Surfacing the error and letting the commit stand is the only honest shape: the layer is durable, the hook side-effect is not, and callers see both facts.

**Hooks queue `Child` emissions exclusively.** By current design, all always-commit (`Sibling`) content originates from *phases*, not hooks. The structural reason: hooks only run after `persist` succeeded, so by the time a hook queues an emission, the parent layer is already on disk and the `Child` semantics are the right ones — drain if and only if the parent landed (which it did). There is no shape in which a hook would want `Sibling` routing, because a hook never runs in a context where the parent did not land. The phase-only origin of `Sibling` emissions is enforced socially (by code review) rather than by the type system; a future hook that violates this invariant would still drain correctly under `Ok` paths but its `Sibling` routing would be dead semantics.

## 4. `CommitState` arena

```rust
pub struct CommitState<'a> {
    // Inputs — set once at orchestrator entry, read by phases.
    storage:      LayerStorage,
    persist:      &'a dyn LayerPersister,
    policy:       CommitPolicy,
    branch:       &'a str,
    institutions: Option<InstitutionContext<'a>>,

    // Transient — rewritten across cascade iterations and across phases.
    builder: LayerBuilder,
    layer:   Option<Arc<Layer>>,

    // Accumulators — written by phases, read by the outcome construction
    // at the end of pipeline run.
    cascade_tombstones:   BTreeSet<Iri>,
    cascade_iterations:   u32,
    dispatched_verdicts:  Vec<DispatchEntry>,
    provenance_resources: Vec<Resource>,
    emissions:            Vec<LayerEmission>,
    hook_errors:          Vec<ValidationError>,  // populated by didPersist hooks

    // Working buffers — borrowed; not owned.
    working_set: &'a mut CommitWorkingSet,

    // Persist result — set by the persist phase, read at outcome construction.
    persisted: Option<PersistedLayerInfo>,
}
```

The lifetime story: `'a` is the lifetime of the orchestrator call. The persister, branch name, institution context, and working set are all borrowed for the duration of one `orchestrator.run(root)` invocation. The state itself is constructed fresh per pipeline run (one per layer); the working set is the only borrowed mutable that survives across pipeline runs in the same orchestrator invocation (re-used to amortise allocation across user / provenance / institution_classes layers).

The fields split cleanly into four groups:

- **Inputs.** Set at the orchestrator boundary; phases read only. The `institutions` field is the only one that varies per pipeline kind — it's `Some` for `with_institutions`, `None` otherwise.
- **Transient.** Mutated heavily by `build` and `retroactive_with_cascade`. Held in `Option` because phases run in a known order and the next phase relies on the previous one having populated the field.
- **Accumulators.** Append-only across the pipeline run. Read once at outcome construction. `hook_errors` is populated by `didPersist` hooks (§3.6) and flows into `LayerCommitOutcome.hook_errors`; the orchestrator carries an analogous `drain_hook_errors` accumulator for `didDrain` hooks that flows into `MultiLayerOutcome.drain_hook_errors`.
- **Persist result.** Set exactly once by the `persist` phase; the orchestrator inspects `PersistedLayerInfo.branch_advanced` to decide whether to drain emissions or skip descendants.

Phases that don't need a field simply don't touch it. The arena is intentionally a single struct (not a per-phase typed input/output) because the phase ordering is small, fixed, and known at compile time; the cost of typed handoffs is not worth the gain.

## 5. Canned pipelines

```rust
pub struct CommitPipeline {
    phases:      &'static [Phase],
    did_persist: &'static [DidPersistHook],
}

impl CommitPipeline {
    pub const fn structural_only() -> Self;       // build, structural_validate, persist
    pub const fn with_retroactive() -> Self;      // + retroactive_with_cascade
    pub const fn with_institutions() -> Self;     // + autoonload_dispatch; didPersist: register_wasm_components
    pub const fn structural_followup() -> Self;   // build, persist (provenance / institution_classes) — no structural_validate
}
```

| Pipeline | build | structural_validate | retroactive_with_cascade | autoonload_dispatch | persist | `didPersist` hooks |
|---|---|---|---|---|---|---|
| `structural_only` | yes | yes | — | — | yes | — |
| `with_retroactive` | yes | yes | yes | — | yes | — |
| `with_institutions` | yes | yes | yes | yes | yes | `register_wasm_components` |
| `structural_followup` | yes | — | — | — | yes | — |

Pipeline run signature:

```rust
pub struct PipelineRunOk {
    pub outcome: LayerCommitOutcome,
}

pub struct PipelineRunErr {
    pub error:             CommitError,
    /// Sibling emissions queued by phases that ran before the failing
    /// phase. Surfaced separately from `CommitError` so the audit
    /// routing channel stays uniform across `Ok` and `Err` returns:
    /// the orchestrator handles `Sibling` emissions the same way in
    /// both cases (re-queue at depth 0, parent at `ctx.head`), and the
    /// error itself is unchanged from the lattice's existing
    /// `CommitError::Validation { ... }`.
    pub sibling_emissions: Vec<LayerEmission>,
}

pub fn run(
    &self,
    builder: LayerBuilder,
    cfg: PipelineConfig<'_>,
    ws: &mut CommitWorkingSet,
) -> Result<PipelineRunOk, PipelineRunErr>;
```

`PipelineConfig` carries the inputs that vary per orchestrator invocation but stay constant across pipeline runs in one orchestrator call: the persister, branch name, policy, optional institution context, and a reference to the storage view. The pipeline's `run` constructs a fresh `CommitState`, opens a `COMMIT_PIPELINE_RUN` span, walks the phase array, runs the `did_persist` hook list under a `COMMIT_DID_PERSIST` span if and only if the `persist` phase set `branch_advanced = true`, and constructs the `LayerCommitOutcome` from the accumulators (including `hook_errors`). On `Err` from any phase, the run halts the phase walk, skips `did_persist` hooks, and partitions `state.emissions` by `EmissionKind`: `Sibling` entries are surfaced via `PipelineRunErr.sibling_emissions`; `Child` entries are dropped (their intended parent did not land).

The pipeline run is the granularity at which `tracing` spans are scoped. Within the run, individual phase telemetry uses `tracing::info!` with the per-phase operation constant (§12).

Distinguishing `structural_only` from `structural_followup`: they are *not* "the same phase list under different names." `structural_followup` deliberately omits `structural_validate`. The structural reason: followup layers (`verdict_provenance`, `institution_classes`) carry **kernel-emitted content** — the resources in those layers come from `build_verdict_resource` / `build_runtime_invocation_resource` (audit content) or the WASM-registration extraction in `register_wasm_from_layer` (institution-classes content), not from user input. Well-formedness of that content is the **emitter's contract**, not something the pipeline re-checks. Re-running `structural_validate` here would be redundant *and* would force the chain ontology to be permissive enough to validate every shape the kernel emits — a coupling that runs the wrong direction (the ontology should describe the domain, not chase the emitter's output shape). If a kernel emitter ever produces content the chain ontology cannot represent, that is a contract violation at the emitter; fix the emitter, not the pipeline.

Concretely, `structural_only` runs `[build, structural_validate, persist]` because it processes user-authored content; `structural_followup` runs `[build, persist]` because it processes kernel-emitted content with guaranteed well-formedness.

## 6. Orchestrator drain loop

```rust
pub struct CommitOrchestrator<'a> {
    ctx:          &'a mut ExecutionContext,
    pool:         &'a CommitWorkingSetPool,
    persister:    &'a dyn LayerPersister,
    branch:       &'a str,
    policy:       CommitPolicy,
    institutions: Option<InstitutionContext<'a>>,
    did_drain:    &'static [DidDrainHook],
}

impl CommitOrchestrator<'_> {
    pub fn run(self, root: LayerEmission) -> Result<MultiLayerOutcome, CommitError>;
}
```

`LayerEmission`:

```rust
pub struct LayerEmission {
    pub name:       &'static str,    // "verdict_provenance", "institution_classes", ...
    pub pipeline:   PipelineKind,
    pub kind:       EmissionKind,
    pub resources:  Vec<Resource>,
    pub tombstones: BTreeSet<Iri>,
}

pub enum EmissionKind {
    /// Drained iff the emission's parent emission's pipeline run
    /// returned `Ok` and `branch_advanced = true`. Parent of the
    /// queued layer is the freshly-landed parent layer. This is the
    /// default and matches every emission today (institution-classes
    /// follow-up, etc.).
    Child,
    /// Drained unconditionally — even if the queuing pipeline's
    /// outer commit returned `Err`. Parent of the queued layer is
    /// `ctx.head` at drain time (the head as it stood when the
    /// queuing emission *started*, not the failed-to-advance layer).
    /// Used for audit-anchor content that must land regardless of
    /// whether the gated commit succeeded — today only the AutoOnLoad
    /// `Verdict` + `RuntimeInvocation` provenance.
    Sibling,
}

pub enum PipelineKind {
    StructuralOnly,
    WithRetroactive,
    WithInstitutions,
    StructuralFollowup,
}
```

`EmissionKind` distinguishes two routing modes for follow-up layers. The vast majority of follow-up content is `Child`: it only makes sense after its parent has successfully landed (e.g., the `institution_classes` follow-up off a user layer is meaningless if the user layer was rejected). A small, structurally identified category is `Sibling`: always-commit content that anchors institutional facts to the chain regardless of whether the gated user-layer commit succeeded. Today the sole `Sibling` emission is the AutoOnLoad `verdict_provenance` layer — see §3.4 for the rationale.

### 6.1 Drain algorithm

The orchestrator owns a FIFO queue of `(depth, LayerEmission)`. Pseudocode:

```
queue ← [(0, root)]
outcomes ← []
working_set ← pool.acquire()
last_advanced ← ctx.head()
first_err ← None

while queue not empty:
    (depth, emission) ← queue.pop_front()

    if depth >= MAX_EMISSION_DEPTH:
        return Err(CommitError::EmissionDepthExceeded { name: emission.name, depth })

    pre_run_head ← ctx.head()                            // for Sibling parenting on Err
    builder ← ctx.take_working(emission.name)            // fresh, parented at ctx.head()
    for r in emission.resources:  builder.add(r)
    for t in emission.tombstones: builder.tombstone(t)

    pipeline ← canned_for(emission.pipeline)
    cfg ← PipelineConfig { persister, branch, policy, institutions, storage: ctx.storage_view() }
    result ← pipeline.run(builder, cfg, &mut working_set)
    //                                  ^ pipeline.run internally executes
    //                                    pipeline.did_persist hooks if and only
    //                                    if the persist phase set
    //                                    branch_advanced = true. Hook errors
    //                                    land on state.hook_errors and flow
    //                                    into outcome.hook_errors. On Err,
    //                                    pipeline.run returns PipelineRunErr,
    //                                    which partitions state.emissions and
    //                                    surfaces only the Sibling subset
    //                                    (queued by phases that ran before
    //                                    the failing phase — notably
    //                                    autoonload_dispatch's verdict_
    //                                    provenance Sibling).

    match result:
        Ok(PipelineRunOk { outcome }) if outcome.persist.branch_advanced:
            ctx.advance_head(outcome.layer.clone(), emission.name)
            last_advanced ← outcome.layer.clone()
            // Child and Sibling alike — when the parent landed, the
            // routing distinction is irrelevant; both drain as children.
            for child in outcome.emissions:
                queue.push_back((depth + 1, child))
            outcomes.push(outcome)

        Ok(PipelineRunOk { outcome }) /* !branch_advanced */:
            ctx.revert_head(last_advanced.clone())
            // descendants are dropped: their parent didn't land.
            // older sibling emissions in the queue remain valid because their
            // parent is `last_advanced`, which is still in storage.
            // didPersist hooks were not run for this pipeline (see §6.5).
            // !branch_advanced is *not* a rejection — see §6.4. Sibling
            // emissions sitting on outcome.emissions are also dropped (see
            // §6.4 for why audit content has no semantic home in the
            // !branch_advanced case).
            outcomes.push(outcome)

        Err(PipelineRunErr { error, sibling_emissions }):
            // Rescue Sibling emissions queued by phases that ran before
            // the failing phase. ctx.head did not advance (this pipeline
            // failed), so pre_run_head == ctx.head() now. Re-queue at
            // depth 0; the next iteration's builder will parent them at
            // ctx.head(), which is pre_run_head.
            for sib in sibling_emissions:
                queue.push_back((0, sib))
            // Child emissions from the failed run were already partitioned
            // off and dropped by pipeline.run.
            first_err ← first_err.or(Some(error))
            // Continue draining so rescued Siblings (and any Children
            // they queue) get committed before we return the error.

pool.release(working_set)

if first_err is Some(e):
    return Err(e)

// Post-drain hook stage (§6.5). Only runs on the all-Ok path.
multi ← MultiLayerOutcome { layers: outcomes, drain_hook_errors: vec![] }
drain_state ← DrainState { top_layer: last_advanced.clone(), multi: &mut multi, ctx }
for hook in self.did_drain:
    let HookOutcome { errors } = hook(&mut drain_state);
    multi.drain_hook_errors.extend(errors);

return Ok(multi)
```

The `didPersist` hooks are executed inside `pipeline.run` (not in the orchestrator loop body) so that pipelines remain self-contained: a pipeline knows its phases and its hooks together. The orchestrator only needs to know the result. Hooks **do not run** when persist did not advance the branch — there is no successfully-persisted layer to hook off, and any side-effect the hook performs would attach to a layer that isn't there.

**The `Err` arm is the heart of the AutoOnLoad-`Fails` audit path.** When `autoonload_dispatch` returns `Err` on a `Fails` verdict, the persist phase never runs, so the user layer is not on disk — but `autoonload_dispatch` queued the `verdict_provenance` Sibling emission *before* returning `Err`. The drain loop rescues it, re-queues it at depth 0 parented at the (unchanged) `ctx.head`, and processes it as a normal `StructuralFollowup` pipeline run. That follow-up pipeline persists the audit layer, advances `ctx.head` to the audit layer, and lands the institutional fact on the chain. After the drain finishes, the orchestrator returns the original `Err` to the caller, but the audit has landed.

The same `Err` arm covers structural-validation failures, retroactive violations, cascade aborts, persist I/O errors, and `NeedsWitnessedMerge` (when surfaced as Err) — none of those phases queue Sibling emissions, so the rescue loop is a no-op and the drain ends with an error response and no audit content. That asymmetry is by design (see §3.4).

### 6.2 Ordering — FIFO across siblings

Emissions queued from the same pipeline run drain in the order they were queued. For the canonical Load flow within a single `with_institutions` pipeline run:

1. The `autoonload_dispatch` phase runs first and queues `verdict_provenance` as a `Sibling`.
2. The `register_wasm_components` `didPersist` hook runs after `persist` succeeds and queues `institution_classes` as a `Child`.

The FIFO order matches today's `user → provenance → institution_classes` sequence in the Load handler. This is not accidental — it's what makes the orchestrator a behaviour-preserving refactor of the existing handler logic. Phases queue before hooks because hooks only run after `persist`, and `persist` is the last phase.

`Sibling` emissions rescued on `Err` (§6.1) are inserted at the back of the queue, preserving FIFO with whatever else is pending. Any successfully-queued `Child` emissions from earlier in the drain still process before the rescued `Sibling`. In the canonical `Load`-with-`Fails` flow there are no earlier `Child` emissions pending (the user-layer pipeline failed before the `register_wasm_components` hook could run, so no `institution_classes` got queued), and the rescued `Sibling` is the only thing in the queue. But the FIFO invariant holds either way: provenance commits before institution_classes if both happen, matching today's ordering.

### 6.3 Depth cap — `MAX_EMISSION_DEPTH = 4`

Termination has two arguments:

- **Static, by construction.** Only `with_institutions` emits. The two emission sites (the `autoonload_dispatch` phase and the `register_wasm_components` `didPersist` hook) both target `StructuralFollowup` pipelines. `StructuralFollowup` does not include any emitting phase or hook. `WithRetroactive` likewise does not. Today, depth is at most 1 on the `Ok` path. On the `Err`-with-Sibling-rescue path, the rescued Sibling is re-queued at depth 0 and runs `StructuralFollowup`, which itself emits nothing — so depth is still at most 1 on that path too.
- **Dynamic safety net.** `MAX_EMISSION_DEPTH = 4` catches a future bug where a phase or hook is added to a followup pipeline and produces emissions. The cap is generous (today's depth is 1; conceivable near-future is 2–3, e.g. an institution-classes layer triggers a follow-up validation that itself emits, or an audit-classes follow-up off the audit layer — architecturally fine, just not a thing today). Sibling emissions can themselves queue Child emissions; those count toward depth from their Sibling root (the Sibling is depth 0; its Child is depth 1). Hitting the cap produces a structured `CommitError::EmissionDepthExceeded { name, depth }` that names the offending emission for debuggability.

### 6.4 Revert semantics

When a pipeline's persist phase reports `branch_advanced = false` (anchored-commit cache hit at a different position per D33 §6; `NeedsWitnessedMerge` per D34 §G.1; any other CAS-loss case), the orchestrator reverts `ctx.head` to `last_advanced`. The contract:

- **The current emission's descendants are dropped.** Their parent (the just-failed emission's layer) is not in storage; queueing them would commit children with a missing parent. They are discarded silently — the user's commit, which was a no-op or a CAS loss, is reported via the eventual `MultiLayerOutcome` (the handler decides whether to surface that as success or a recoverable condition).
- **Older sibling emissions in the queue remain valid.** Their parent is `last_advanced` (or an earlier advanced layer), which is in storage. They drain normally.
- **`ctx.head` is restored exactly once per drain.** The `last_advanced` variable tracks the most recent successfully-advanced layer; revert points there. Repeated reverts in a single drain (multiple consecutive non-advancing emissions) all point at the same `last_advanced` — there is no compounding state.

This is a direct replacement for the three near-identical revert blocks in today's Load handler, with the additional property that the revert is centralised — adding a new commit-shaped RPC does not require re-implementing the revert.

**`!branch_advanced` is not a rejection.** A different-position cache hit (D33 §6) or `NeedsWitnessedMerge` (D34 §G.1) is a no-op CAS outcome, not a structural or institutional failure. The pipeline ran cleanly through all its phases, including `autoonload_dispatch`; `persist` merely observed that the branch tip moved or that an anchored cache entry already covered this content. The semantic difference from a rejection is:

- A rejection (`Err`) means some phase decided the commit must not stand — structural-validation failure, retroactive violation, AutoOnLoad `Fails`, cascade abort, I/O error. The user-layer commit is being refused.
- A no-op (`!branch_advanced`) means the commit's content is either already on the chain or pending witnessed-merge resolution. Nothing was refused.

Because no rejection occurred, no institutional fact worth auditing was created. **`Sibling` emissions are *not* drained on `!branch_advanced`** — only on `Ok`-with-`branch_advanced` (where they queue as children of the landed layer) or on `Err` (where they are rescued and re-rooted, §6.1). If `autoonload_dispatch` had already populated `state.emissions` with a `verdict_provenance` Sibling before persist returned `!branch_advanced`, those entries are visible on `outcome.emissions` but the orchestrator does not enqueue them — same fate as the Child emissions on this branch. The audit content has no semantic home in the `!branch_advanced` case: the layer being audited is either already present (so duplicate provenance is noise) or pending witnessed-merge (so provenance attached to an un-merged state is incoherent). The distinction matters: a different-position cache hit doesn't represent an institutional fact worth auditing; an AutoOnLoad `Fails` does.

### 6.5 Post-drain hook stage

After the drain loop exits, the orchestrator runs `did_drain` hooks against a `DrainState`:

```rust
pub struct DrainState<'a> {
    pub top_layer: Arc<Layer>,            // final top of branch after drain
    pub multi:     &'a mut MultiLayerOutcome,
    pub ctx:       &'a mut ExecutionContext,
}
```

`DrainState` deliberately does *not* expose `state.emissions` — by the time `didDrain` runs, no further pipelines will execute and queuing more work is meaningless. Hooks receive `&mut MultiLayerOutcome` so they can read the full set of landed layers if needed; errors they raise go into `multi.drain_hook_errors`.

The canonical `didDrain` hook is `rebuild_institution_index`. It walks the institution declarations reachable from `top_layer` and rebuilds the dispatch index that lives on the kernel's institution runtime. Today the Load handler does this three times per Load (once per landed layer); under D41 the orchestrator does it once at the end of the drain, which is semantically equivalent because no work *within* the drain reads the index — only the next RPC's `InstitutionContext` snapshot reads it. The collapse from three rebuilds to one is the efficiency win called out in §11.

## 7. `LayerPersister` boundary

```rust
pub trait LayerPersister: Send + Sync {
    fn persist(
        &self,
        branch: &str,
        layer: &Arc<Layer>,
    ) -> Result<PersistedLayerInfo, ValidationError>;
}

pub struct PersistedLayerInfo {
    pub layer_id:        LayerId,
    pub branch_advanced: bool,
    pub cache_hit:       bool,           // D33 §6 anchored-commit cache
    pub update_outcome:  UpdateOutcome,  // FastForward / TrivialMerge / NeedsWitnessedMerge
}
```

`EigeniusService` implements `LayerPersister`. Its implementation is exactly today's `persist_layer_if_backend` body — the anchored-commit cache probe, the `backend.store_layer` call, the `update_branch` CAS, the trivial-merge handling. None of that logic changes; it moves modules and gains a name. Tests use an in-memory implementation that records calls and returns canned `PersistedLayerInfo` values.

The pipeline depends on the trait, not the impl. This is the cleanest place to put the seam: it gives tests injection without exposing the orchestrator to test-specific abstractions, and it gives the pipeline a single persist call site instead of the current "the pipeline calls store_layer; the handler calls store_layer too" split.

Trait location: `kernel/src/commit/persister.rs`. The trait depends on `PersistentBackend` from `kernel/src/storage/` because the impl in `EigeniusService` reaches into storage; that's the natural direction (commit knows about storage, not vice versa).

## 8. `CommitPolicy` plumbing

There is one `CommitPolicy` per orchestrator run. It threads into every `CommitState` constructed in the drain. Followup pipelines (`structural_followup`) do not run the retroactive phase; the policy is unused there. A single global policy is harmless in that case — there is no situation today where the user layer and the provenance follow-up want different policies.

Per-phase or per-pipeline policy granularity is a forward option (§13), not part of D41. The phases read `state.policy` directly; switching to per-phase later means changing one field type, not threading new state.

## 9. `ExecutionContext` interaction

`ExecutionContext` keeps every existing field. It loses two methods and gains two:

- **Removed:** `commit`, `commit_with_validation`. The work both methods did is split across the phases.
- **Added:**
  - `take_working(&mut self, fresh_name: &str) -> LayerBuilder` — produces a fresh, empty `LayerBuilder` parented at `ctx.head()` and named `fresh_name`. The orchestrator calls this once per emission.
  - `advance_head(&mut self, layer: Arc<Layer>, fresh_name: &str)` — installs `layer` as the new `ctx.head` and resets the working builder name. The orchestrator calls this when `persist.branch_advanced` is true.

`revert_head` already exists; the orchestrator calls it on the `branch_advanced = false` path.

The role of `ExecutionContext` shifts from "session that also commits" to "session that delegates commit to the pipeline." No data fields change.

`ExecutionMode::ReadOnly` enforcement remains a runtime check inside the new pipeline-interop methods (`take_working` and `advance_head` reject in read-only mode). A type-level `ReadSession` / `WriteSession` split was explicitly rejected: the caller-side churn outweighs the gain, and the runtime check is correct.

## 10. RPC mapping

Every commit-shaped RPC follows the same handler shape: translate the request into a root `LayerEmission`, call `orchestrator.run(root)`, translate the `MultiLayerOutcome` into the response. The pipeline kind for the root emission depends on the RPC:

| RPC | Root pipeline | Notes |
|---|---|---|
| `Load` (with `auto_commit`) | `WithInstitutions` | Followups emitted: `verdict_provenance` (if AutoOnLoad fired), `institution_classes` (if WASM components registered). |
| `Query` (FIBER INTO) | `WithRetroactive` | No institutional gate today; revisit if INTO ever wants AutoOnLoad. |
| `RunProgram` / `RunProgramByIri` | `WithRetroactive` | Same. |
| `Reflect` | `WithRetroactive` | Same. |
| `SubmitResolution` | `StructuralOnly` | User has already chosen this resolution in the witnessed-merge UX; a cascade would surprise them. |
| `CapabilityInstall` | `WithRetroactive` | Standard commit shape. |

`Load` is the only RPC today that produces emissions. All other RPCs run the orchestrator's degenerate single-pipeline case — one drain iteration, empty emission queue, returns. The uniformity is the point: the handler shape is identical across all six RPCs.

### 10.1 Explicit tombstones

`LoadRequest` (and any other RPC needing it) gains an `explicit_tombstones: Vec<Iri>` field. The handler routes them to the orchestrator, which applies them to the *initial* root emission's builder (via `LayerBuilder::tombstone`) before running the user-layer pipeline. They participate in the cascade fixpoint like any other tombstone. Followup pipelines do not receive explicit tombstones; their tombstones come from emission state, not from the user.

## 11. Migration — what collapses

| Today | After |
|---|---|
| `commit_layer` in `lattice.rs` (calls `store_layer` directly) | `CommitPipeline::with_retroactive()` (calls `LayerPersister::persist`) |
| `commit_with_validation` in `context/mod.rs` | `CommitPipeline::with_institutions()`, driven by orchestrator |
| `commit_layer_default` | thin wrapper around `CommitPipeline::with_retroactive()` + in-memory `CommitWorkingSet` + default policy |
| `persist_layer_if_backend` in `server/mod.rs` | `EigeniusService as LayerPersister` impl |
| Two `CommitOutcome` types | one `LayerCommitOutcome` |
| Load handler revert state machine (~250 lines) | `CommitOrchestrator::run` |
| Handler-side `register_wasm_from_layer` invocation | `register_wasm_components` `didPersist` hook on `with_institutions` |
| Handler-side `rebuild_institution_index` invocations (×3 per Load) | `rebuild_institution_index` `didDrain` hook on orchestrator (×1 per Load) |
| Handler-side `working.build(storage)` for provenance / institution-classes layers, with no re-validation ([`kernel/src/context/mod.rs:476-491`](../../kernel/src/context/mod.rs)) | `structural_followup` pipeline = `[build, persist]`, no `structural_validate` (§5) |

The collapse of the institution-index rebuild from three calls per Load to one is an efficiency win the new shape gets for free: today's handler rebuilds after each of the user, provenance, and institution-classes layers lands, but nothing inside a single Load actually consumes the index between those rebuilds — only the next RPC's `InstitutionContext` snapshot reads it. The `didDrain` hook position makes the single-rebuild shape natural rather than requiring careful argument about which rebuilds can be elided.

`kernel/src/lattice.rs` retains `update_branch`, `ConflictPolicy`, `UpdateOutcome`, `BranchUpdateError`. The branch CAS algebra is distinct from commit-pipeline orchestration; it lives downstream of the persister and stays in `lattice.rs`. The `CommitPolicy` enum and `commit_layer_default` helper move to `kernel/src/commit/`.

### 11.1 Module layout

```
kernel/src/commit/
  mod.rs          -- re-exports
  pipeline.rs     -- CommitPipeline, PipelineKind, Phase, PhaseControl, PipelineConfig
  state.rs        -- CommitState arena, DrainState
  phases.rs       -- the five phase functions
  hooks.rs        -- DidPersistHook, DidDrainHook, HookOutcome, the two concrete hooks
  orchestrator.rs -- CommitOrchestrator + drain loop + post-drain hook stage,
                     LayerEmission, EmissionKind
  persister.rs    -- LayerPersister trait, PersistedLayerInfo
  outcome.rs      -- LayerCommitOutcome, MultiLayerOutcome, CommitError
```

`kernel/src/commit/` is a sibling of `kernel/src/lattice.rs` and `kernel/src/storage/`. It depends on both; neither depends on it. Test-only helpers (in-memory persister, fixture `CommitWorkingSet`) live in `kernel/src/commit/test_support.rs` gated behind `#[cfg(test)]`.

### 11.2 `CommitWorkingSetPool` location

Owned by `EigeniusService` — one pool per server. Single-branch commits acquire one pooled working set per orchestrator run; the orchestrator passes it through every pipeline run in that operation (re-using it across user / provenance / institution-classes layers). Per-branch pools were considered and rejected: branches commit serially today (per-branch lock), so per-server is sufficient.

## 12. Telemetry

Each phase emits a `tracing::info!` with a new operation constant in `kernel/src/observability/operation.rs`. Hooks log under their own ad-hoc operation names too (`COMMIT_REGISTER_WASM` is the existing one, preserved from today's handler-side telemetry):

```rust
pub const COMMIT_BUILD: &str                = "kernel.commit.build";                 // phase
pub const COMMIT_STRUCTURAL_VALIDATE: &str  = "kernel.commit.structural_validate";   // phase
pub const COMMIT_RETROACTIVE: &str          = "kernel.commit.retroactive";           // phase
pub const COMMIT_CASCADE: &str              = "kernel.commit.cascade";               // per cascade iteration
pub const COMMIT_AUTOONLOAD: &str           = "kernel.commit.autoonload";            // phase
pub const COMMIT_PERSIST: &str              = "kernel.commit.persist";               // phase
pub const COMMIT_REGISTER_WASM: &str        = "kernel.commit.register_wasm";         // didPersist hook
pub const COMMIT_PIPELINE_RUN: &str         = "kernel.commit.pipeline_run";          // span
pub const COMMIT_ORCHESTRATOR_RUN: &str     = "kernel.commit.orchestrator_run";      // span
pub const COMMIT_DID_PERSIST: &str          = "kernel.commit.did_persist";           // span (per pipeline)
pub const COMMIT_DID_DRAIN: &str            = "kernel.commit.did_drain";             // span (per orchestrator)
```

The orchestrator opens `COMMIT_ORCHESTRATOR_RUN` as a `tracing::info_span!` around the entire drain loop. Each pipeline run opens `COMMIT_PIPELINE_RUN` as a nested span. Individual phases emit `tracing::info!` events with their operation constant — not spans, because phases run synchronously and don't nest. The cascade phase emits one `COMMIT_CASCADE` event per fixpoint iteration so cascade depth is visible in trace output.

Hook execution gets its own span layer. Inside each pipeline run, after the `persist` phase succeeds, the pipeline opens `COMMIT_DID_PERSIST` as a span around the full `did_persist` hook list; individual hooks log under their own ad-hoc operation names if they want (`register_wasm_components` already has telemetry today and keeps it). The orchestrator opens `COMMIT_DID_DRAIN` once, around the `did_drain` hook list, nested inside `COMMIT_ORCHESTRATOR_RUN` but after every `COMMIT_PIPELINE_RUN` has closed.

This gives operators three nesting levels in trace output (orchestrator → pipeline → phase events), plus the hook spans as siblings of the phase-event sequence within each level, matching the conceptual hierarchy.

## 13. Future work

### 13.1 Institutional retroactive cascade

Today's `autoonload_dispatch` runs only on resources defined in the new layer. The analogous "lower-layer impact of a new institutional declaration" pass — when a new layer declares an institution gate covering an IRI that already exists in lower layers — has no current implementation. D41's `CommitPolicy` shape generalises naturally: a future `CommitPolicy::CascadeInstitutional` variant would tell a `institutional_retroactive_cascade` phase to scan lower layers, run gates over historical IRIs, and queue provenance for any verdicts. The phase slots into `with_institutions` after `autoonload_dispatch` and before `persist`; the `register_wasm_components` `didPersist` hook continues to run after persist as it does today.

### 13.2 Per-phase policies

A single global `CommitPolicy` per orchestrator run is sufficient today. If a future workload needs e.g. `Reject` for structural cascade but `CascadeTombstone` for institutional retroactive, the policy becomes a struct of per-phase variants. The phases read their slice of it; the orchestrator threads the struct rather than a single enum value. This is a one-field change in `CommitState`.

### 13.3 Per-branch working-set pool

Per-server pool suffices because branches commit serially. If multi-branch concurrent commits ever land (D33 v2 sketches this), the pool partitions per branch. The orchestrator already takes `&CommitWorkingSetPool`; switching to a sharded pool is invisible to it.

### 13.4 Witnessed-merge resolution as an orchestrator entry point

`SubmitResolution` currently goes through the `StructuralOnly` pipeline because the user has manually chosen the resolution. A future witnessed-merge UX might run the resolution through `WithRetroactive` to catch downstream effects of the chosen merge. The shape supports this — only the RPC mapping in §10 changes.

## 14. Open questions

None at present. The earlier open questions about `register_wasm_components` / persist ordering and `rebuild_institution_index` placement are resolved by the hooks shape introduced in §3.6 and §6.5: both pieces of work are post-persist hooks, not phases, and the pipeline/orchestrator split makes their positions unambiguous (`didPersist` on the `with_institutions` pipeline; `didDrain` on the orchestrator).
