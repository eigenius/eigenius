# D21: Task Traces and Checkpointing

*Design document for the Eigenius project — April 2026*

**Status:** Implemented (Phase 9b-iii)
**Required before:** Phase 9b-iii (task model) implementation
**Depends on:** D6 (execution architecture), D13 (durable kernel state)
**Companion to:** D11 (codata + streams)

---

## 1. Motivation

D11 and D13 together promise resumable execution: a program that crashes
mid-way should restart and pick up where it left off, backed by the
`ComponentTrace` cache that D13 persists in RocksDB. Phase 9a wired the
trace store through. In principle, a killed-and-restarted kernel should
replay a program and have every already-completed IO call short-circuit
from the cache.

Two problems surface the moment we try to apply that to a real stream
or long-running task:

1. **The cache collides on repeated calls.** `ComponentTrace` is keyed
   by `SHA-256(component_iri ‖ CBOR(input))`. A program that calls
   `dequeue({})` a thousand times hits the *same* cache entry every
   time. Memoization was a correct design for deterministic Pure /
   Read components, but it is wrong for a stream consumer — the cache
   silently collapses distinct IO events into one.

2. **Replay cost grows without bound.** Even if the key collision were
   fixed by scoping the trace per call, a restart still re-evaluates
   the whole program from the root. The cache turns each IO step from
   "seconds" into "microseconds", but the walk is O(N) in the number
   of traced steps. A 10k-event stream that's been running for days
   replays 10k cache lookups before making any new progress, and the
   trace store grows forever.

This document specifies how traces are identified, checkpointed, and
pruned so that both problems are solved before Phase 9b-iii starts
building on top of them.

The scope is deliberately narrow: **how persistent tasks identify and
compress their trace history.** It does not specify:

- Codata type theory (D11 §2–4)
- The task-table gRPC surface (D11 §5, Phase 9b-iii)
- The "Task as codata" evaluator refactor (D11 §5.1, not in Phase 9b)
- Orchestrator-side durability (D12b)

---

## 2. Today's state (honest inventory)

### 2.1 Trace keying

`kernel/src/program/trace.rs`:

```rust
pub fn compute_trace_key(component: &str, input: &Resource) -> [u8; 32] {
    // SHA-256(component_iri ‖ CBOR(input))
}
```

One global namespace, content-addressed. Correct for `Pure` / `Read`
components because they are deterministic functions of their input.
Incorrect for any IO component whose output depends on external state
(queues, timers, the outside world).

### 2.2 Trace lookup in the evaluator

`kernel/src/nbe/eval.rs` (around line 300):

```rust
let cache_key = compute_trace_key(component_iri, &input);
if let Some(cached) = store.get_component_trace(&cache_key) {
    // Return the cached result as the component's output.
} else {
    // Dispatch, then store the resulting ComponentTrace under the same key.
    store.put_component_trace(cache_key, trace);
}
```

Cache hits are indistinguishable from fresh dispatches to the caller.
There is no notion of "this is the third call in program P, execution
instance I" — every call with the same input is the same call, by
construction.

### 2.3 What "resume" would look like today

If we killed the kernel mid-program and restarted it with the same DB,
the trace store would be present but there is no concept of a program
*instance*. `RunProgram` is synchronous; it has no identity that
survives beyond the gRPC call. There is nothing on disk that says
"program P was running with input I, got this far." The trace cache is
a memoization side-table, not a task log.

---

## 3. Proposed model: tasks as first-class trace owners

### 3.1 Identity

A **task** is a single execution of a program — a program run. Every
task belongs to exactly one session (§3.7), which determines the
layer it pins at entry and where its eventual result layer lives on
the chain. In 9b-iii the enclosing session is always the single
hardwired one (`session_id = Uuid::nil()`); Phase 14 generalizes to
multiple sessions without changing the task model.

Every task has:

- `session_id: Uuid` — the enclosing session. Always `Uuid::nil()`
  in 9b-iii; carried in the record from day one.
- `task_id: Uuid` — kernel-assigned on `RunProgram` entry.
- `program_iri` — the IRI of the program being executed.
- `input_iri` — the IRI of the input resource.
- `layer_head: LayerId` — the session's active top at task entry
  (§8.1 pin-at-entry decision); the task reads against this layer for
  its lifetime.
- `created_at: Timestamp`.
- `status`: `Running | Suspended | Completed | Failed | Cancelled`.
- `step_seq: u64` — monotonic counter, incremented on each IO
  dispatch. The counter lives on disk so it survives restarts.
- `last_checkpoint: Option<CheckpointId>` — latest committed
  checkpoint (§4). `None` until the program produces its first.
- `latest_trace_seq: u64` — the seq of the most recent
  `ComponentTrace` written, used to drive pruning (§5) and replay
  (§6).
- `result_layer_head: Option<LayerId>` — on completion, the layer
  the task committed as its result (§3.7); `None` while running.

Tasks live under the session's keyspace on the persistent backend:

```
session:<session_id>:task:<task_id>:meta      → CBOR-encoded TaskRecord
session:<session_id>:task:<task_id>:trace:<N> → CBOR-encoded ComponentTrace
session:<session_id>:task:<task_id>:ckpt:<N>  → CBOR-encoded Checkpoint
```

For brevity, later sections of this document drop the
`session:<nil>:` prefix in inline references and write
`task:<task_id>:trace:<N>`. See §7 for the full storage schema.

### 3.2 Trace keying under tasks

The per-IO-call trace written to `task:<task_id>:trace:<N>` is keyed
*positionally* by `step_seq`, not by content hash. This is the key
change from today's model:

```rust
pub struct TraceLookup {
    task_id: Uuid,
    step_seq: u64,
}
```

When the evaluator is about to dispatch an IO component on behalf of
task `T`:

1. Read the current `step_seq` for task `T` (call it `N`).
2. Look up `task:T:trace:N` in the backend.
3. **Hit:** return the cached trace's result (this is a replay).
4. **Miss:** dispatch the component, store the resulting
   `ComponentTrace` under `task:T:trace:N`, increment `step_seq`.

Repeated calls to the same component with the same input now produce
distinct trace entries because each one occupies its own `step_seq`
slot. Streams work.

### 3.3 Cross-task memoization for deterministic components

The SHA-256 content-address cache is not thrown away — it is demoted.
Components whose `capability_level` is `Pure` or `Read` are
deterministic by construction. For those, we keep the content-address
cache as an optional *second* lookup:

```
On IO dispatch:
  1. Check task:T:trace:N (per-task replay).
     Hit → return.
  2. If the component is Pure/Read, check memo:<content_hash>
     (cross-task memoization).
     Hit → copy the result into task:T:trace:N and return.
  3. Dispatch. Store under task:T:trace:N, and if Pure/Read, also
     write-through to memo:<content_hash>.
```

Under this split:

- Per-task traces are a **log** (ordered, per-task, used for replay).
- The content-address cache is a **side-table** (unordered, shared,
  used for optimization across tasks).

IO components with external side effects *never* write or read the
content-address side-table — they are strictly per-task.

### 3.4 Component classification

This design requires knowing which components are deterministic. Three
options, ordered by implementation cost:

1. **Reuse existing `capability_level`.** `Pure` and `Read` → eligible
   for cross-task memo; `IO` → not. This is what we have already and
   requires zero surface change.
2. **Add an `idempotent: bool` property** on `Component` resources.
   Defaults to `false` for IO, `true` for Pure/Read. Institutions can
   mark specific IO components as idempotent (e.g. a content-addressed
   GET with an ETag).
3. **Full effect system.** Out of scope for 9b.

Phase 9b-iii ships with option 1. Option 2 is a clean later addition.

### 3.5 Lineage, observations, and derived state

A task's on-disk footprint sorts into three tiers with very different
durability requirements:

1. **Program lineage.** The program resource itself, its input IRI,
   and the layer chain it runs against. All already durable via D13 —
   this is the recipe and the proof structure for what the task is
   *doing*. Never part of the task keyspace; never eligible for
   pruning through this document.
2. **Observations (nondeterministic IO traces).** The outputs of
   `IO`-capability component calls: dequeued events, LLM responses,
   timestamps, sensor readings, remote HTTP bodies. These are not
   reconstructible — once lost, we cannot faithfully replay or audit
   the task. They are the only record of what the outside world said
   to us. Retention for these is an audit / compliance decision, not
   a performance decision (§5).
3. **Derived state.** Pure/Read component traces (deterministic
   functions of layer state + input) and `Checkpoint` snapshots. Both
   are caches — reconstructible from tier 1 + tier 2 by re-running
   the program. They exist to bound replay cost, not to preserve
   information.

An important asymmetry inside tier 3: derived data computed downstream
of observations is *operationally* observation-like — you can't get it
back without keeping something, because you need the upstream
observations to re-derive it. The difference is that the program
supplies strictly more provenance than the raw observations alone. An
observation trace says "the outside world said X at step N." The
program says exactly what was done with X to produce every
intermediate resource. Given both, any derived value is uniquely
determined. This is why we keep (program + observations) rather than
materializing downstream derivations into storage — the pair is a
tighter audit trail than the derivations themselves would be, and it
scales better: the program stays one resource even as the observation
log grows.

Every retention, pruning, and compaction decision in §5 follows from
this tiering. The only invariant that bites is tier 2: as long as the
IO trace log is intact (plus the program in the layer chain), the task
can be faithfully resumed or audited — all other data is a cache we
can afford to lose.

### 3.6 Task-scoped layer reads (new primitive)

Head-pinning (§8.1) makes one implicit assumption explicit: **the
kernel must be able to evaluate against a layer that is not its
current head.** This is a new primitive. Today, `ExecutionContext`
advances its `head` linearly on every commit, and every evaluator
invocation implicitly runs against that one head. Tasks break that
assumption: if task T₁ is pinned to `H₁` and a `Load` RPC meanwhile
advances the kernel's head to `H₂`, then T₁ (if resumed or still
running in the background) must read against `H₁`, while a new query
in the foreground reads against `H₂`. Two concurrent tasks pinned to
different heads compound the need — each requires its own read-view
over a different `Arc<Layer>` chain.

The underlying mechanism already exists in the evaluator:
`EvalCtx::IO` and `EvalCtx::Read` both hold an owned `Arc<Layer>`,
so the NbE layer is agnostic to whether that layer is the kernel's
current head or a historical snapshot. What's missing is higher up:

- **Backend lookup for arbitrary layers.** D13 already stores every
  committed layer under its `LayerId` and preserves them indefinitely.
  A task with a pinned head needs to re-hydrate the chain rooted at
  that head — D13's `load_chain(head_id)` is exactly the primitive, it
  just has to be called for each task on resume, not only for the
  kernel's current head.
- **Task-local evaluator construction.** The server's `RunProgram`
  handler today grabs `self.context.head()` implicitly. Tasks need to
  build a dedicated `EvalCtx` rooted at their pinned head — concretely,
  a `TaskEvalContext { head: Arc<Layer>, trace_adapter, … }` that is
  independent of the kernel's global context.
- **No new storage surface.** Multiple "tops" already coexist in the
  store as a natural consequence of D13 never pruning old heads; we
  just haven't used that structure as anything other than the single
  linear chain before.

What D21 does **not** introduce is any mechanism for *switching* the
kernel's live head between these tops within a session. The kernel's
head advances monotonically as loads commit; tasks simply opt into a
historical snapshot for their own lifetime. Richer scenarios —
programs that create layers, branch resolution, cross-top
reconciliation — are Phase 14 concerns (D20) and build on the
task-scoped read primitive introduced here.

**Tasks can also pin to non-top layers.** D13 retains every committed
layer indefinitely, so "top of some chain" and "mid-chain" are
indistinguishable from the backend's point of view — both are just
`LayerId`s resolvable via `load_chain`. A natural extension of the
pinning primitive is letting a user start a task rooted at an arbitrary
historical layer, not only the current kernel head. This enables:

- *Branching / exploration.* "Run the same program against the
  pre-institution state to see what changed."
- *Historical replay.* "Re-audit a decision made three months ago by
  running the then-version of the program against the then-version of
  the ontology."
- *Counterfactual analysis.* "What would this task have done if we
  hadn't loaded the experimental institution last week?"

From the kernel's perspective these are all the same operation:
construct a `TaskEvalContext` rooted at a caller-supplied `LayerId`
instead of the live head. The RPC surface for opting into a non-top
pin is deferred (9b-iii ships with "pin to current head" as the only
exposed option), but the internal architecture is identical and we
shouldn't design our way out of it. A later `RunProgram` variant that
accepts an optional `layer_head: LayerId` field is the natural way to
expose this.

### 3.7 Sessions and active-top selection

A **session** is the unit that program runs (tasks, §3.1) attach to.
It is a client-scoped handle to an *active top* layer — the layer
that new `Load` commits extend and that task pinning (§8.1) defaults
to. Without it, task results that commit as new layers with a pinned
head as parent are stranded: they exist in the store but no read path
reaches them from the kernel's notion of "current head."

Program *definitions* are Resources in the layer chain and are not
session-attached; program *runs* (tasks) are.

**9b-iii scope: single hardwired session.** The kernel hosts exactly
one session with `session_id = Uuid::nil()` (the all-zero UUID). All
RPCs are implicitly scoped to it. The session's active top *is* the
kernel's current head — they are synonyms in v1. `Load` commits
advance the active top linearly, exactly as today; `RunProgram`
accepts tasks pinned to the active top at entry.

**Data model commits to session from day one.** Every `TaskRecord`,
every IO trace, every `Checkpoint` carries a `session_id: Uuid`
field, always `nil()` in 9b-iii. Storage keyspace reflects this:

```
session:<session_id>:task:<task_id>:meta
session:<session_id>:task:<task_id>:trace:<N>
session:<session_id>:task:<task_id>:ckpt:<N>
```

In v1 the `session:<nil>:` prefix is constant and could be inferred,
but writing it explicitly means the Phase 14 multi-session rollout is
a surface expansion (adding RPCs, populating non-nil IDs) rather than
a data migration. No existing v1 data has to move when real sessions
arrive.

**Auto-advance vs fork on task completion.** When a task completes
and commits a result layer with `parent = TaskRecord.layer_head`, the
session's active top may or may not auto-advance:

- *Fast-forward case.* If the session's active top has not moved
  since the task started (`pinned_head == session.active_top`), the
  session auto-advances its active top to the new result layer. The
  timeline stays linear; the user's next query sees the result
  naturally, no `at_layer` hint needed.
- *Fork case.* If other commits landed against the session's active
  top while the task was running (another `Load`, another completed
  task), the result layer is a new top in the DAG — its parent is no
  longer the live active top. The session does **not** auto-advance.
  The forked result is reachable via `at_layer` reads (§3.6 read
  extension, ships in 9b-iii). In Phase 14 the user will be able to
  `SwitchActiveTop` onto it explicitly.

This mirrors how version control handles concurrent writes versus
fast-forward merges and keeps the session's timeline honest. A single
hardwired session with no concurrent writers will almost always
fast-forward, so the fork path is latent in v1 but still correct if
it triggers (e.g. two long-running tasks racing).

**Read path with sessions.** Reads default to the session's active
top. An optional `at_layer: LayerId` hint on `Query`, `Inspect`,
`ListInstitutions`, and `GetSchema` lets clients pin a specific read
to any committed layer — forked task results, historical snapshots,
audit inspection. This is the read-side slice of the §3.6 non-top-pin
extension and is **in scope for 9b-iii** because otherwise forked
task results are stranded. Write-side non-top pinning (issuing
`RunProgram` against a historical layer) and the full session RPC
surface are still deferred.

**Phase 14 opens it up.** The multi-session work adds:

- `CreateSession`, `CloseSession`, `SwitchActiveTop`, `ListSessions`
  RPCs and their CLI verbs.
- An optional `session_id` field on every session-scoped RPC; default
  value `nil()` = the v1 hardwired session, preserving backward
  compatibility.
- Independent active tops per session; concurrent sessions read and
  extend their own chains without interfering.
- Policy decisions postponed: concurrent writes to the same top
  (single-writer versus merge-on-commit, which ties into D20
  comorphism reconciliation); multi-user permissions (D14 Security
  Model); cross-restart session persistence (probably not — sessions
  are transient client handles).

---

## 4. Checkpoints

### 4.1 What a checkpoint is

A **checkpoint** is a snapshot of a task's logical state at a safe
resumption boundary. The program declares its state shape; the kernel
stores the snapshot:

```
struct Checkpoint {
    task_id: Uuid,
    step_seq: u64,           // the step this checkpoint covers through
    state: Resource,         // the task's declared state at this point
    created_at: Timestamp,
}
```

A checkpoint says: "At step N, the task's state was this Resource. If
you resume, start from here; traces before step N are no longer
needed."

### 4.2 Who commits checkpoints

The program commits them explicitly via a new built-in component:

```
urn:eigenius:program:components:Checkpoint
  input_type:  urn:eigenius:core:resource
  output_type: urn:eigenius:core:resource     (echoes input)
  capability_level: urn:eigenius:program:capability_levels:io
```

Semantics: `Checkpoint(state)` persists `state` as the task's current
checkpoint and returns `state` unchanged, so callers can write it
inline inside an expression. It is an IO-level primitive so that
commit-to-disk happens synchronously with the program's view of its
own state.

Programs decide when to check in:

```esl
program demo:fold_stream : demo:Event -> demo:Accumulator {
  let acc_in     = input;
  let event      = components:dequeue(queue);
  let acc_next   = components:apply(event, acc_in);
  let _          = components:Checkpoint(acc_next);
  acc_next
}
```

Rate limiting is the program's concern: check in every N events, every
N seconds, etc. The kernel provides the primitive, not the policy.

### 4.3 Codata tasks (D11 §5)

When the task model grows into codata `Task` values, the observation
boundary (each `.step` call) is the natural checkpoint point — the
kernel can checkpoint automatically after each observation. That is a
D11 §5 concern; D21 specifies the explicit-commit primitive that works
today and that codata tasks will continue to use for coarser-grained
checkpointing.

### 4.4 Resume with a checkpoint

On resume (see §6), the kernel locates the latest checkpoint, feeds
the saved `state` resource back into the program as if it were the
original `input`, and replays traces with `step_seq > ckpt.step_seq`
only. Traces below the checkpoint are pruneable (§5).

---

## 5. Retention and pruning

Retention is driven by the tiering in §3.5: observations (tier 2)
preserve information the outside world gave us; derived state (tier 3)
is reconstructible from tier 1 + tier 2 and therefore disposable.

### 5.1 Per-trace retention class

Each `task:<id>:trace:<N>` entry carries a classification derived from
the component's `capability_level`:

- **Observation traces** (`IO`): outputs of nondeterministic
  components. Durability policy is an audit decision, not a
  performance one — keeping them is what lets us answer "what did the
  outside world actually say to this task?" later.
- **Derived traces** (`Pure`, `Read`): outputs of deterministic
  components. Reconstructible by re-running the program against the
  same layer head plus the observation log. Free to drop any time.

Checkpoints (`task:<id>:ckpt:<N>`) are always derived — they are a
precomputed replay shortcut, not primary data. A lost checkpoint
reconstructs from (program + input + observation log) at the cost of
longer replay.

### 5.2 Resume invariants

For a task to resume correctly from the latest checkpoint at step `M`,
the kernel requires:

- `task:T:meta` and `task:T:ckpt:M` intact.
- `task:T:trace:N` intact for every **observation** with `N > M` (we
  need to re-feed those to the program during live resume — they are
  not reproducible by re-dispatching).

Everything else — derived traces at any `N`, observation traces at
`N ≤ M` — is optional for resume. It's either in the checkpoint (by
having been folded in) or it's an audit artifact whose retention is
governed separately (§5.4).

### 5.3 Pruning on checkpoint commit

When a new checkpoint is committed at step `N`:

1. Write `task:T:ckpt:N` with the new state.
2. Update the task record: `last_checkpoint = N`.
3. Schedule a background delete of every **derived** trace at any
   step. They're a pure cache; the checkpoint didn't need them to
   exist, and future resume won't either.
4. Retain **observation** traces at `K ≤ N` subject to the audit
   policy (§5.4). Default v1 policy: keep them until the task itself
   is pruned.

The pruning is best-effort and can lag. Stale derived entries are
inert — the replay logic never consults them once the checkpoint
covers them.

A defensive global cap still applies: `max_observations_per_task =
100_000` (configurable). If a task accumulates that many observation
traces without a completion or cancellation, the kernel refuses to run
further IO and marks the task `Failed`. The cap exists to bound disk
growth; a real workload that needs more should lean on the audit
policy to roll observations off into cold storage (out of scope here).

### 5.4 Retention of completed and cancelled tasks

When a task reaches `Completed`, `Failed`, or `Cancelled`:

- Keep `task:T:meta` and the final `task:T:ckpt:*` (if any) — these
  together with the program resource constitute the task's lineage.
- Keep the **observation** traces for the configured audit retention
  window (default: unlimited in v1, expected to become a 30-day TTL
  once a real audit policy is written). These are what an external
  reviewer needs to reconstruct what the task actually did.
- Prune all **derived** traces immediately. The task will never
  resume; derived state is pure cache.

Cross-task memo entries (`memo:*`) are unaffected by task termination
— they are a global side-table and live under their own eviction
policy (LRU once a size cap is implemented; for v1, unlimited).

---

## 6. Resume protocol

On kernel startup (after D13's seed/resume path completes), scan
`task:*:meta` for tasks with status `Running` or `Suspended`. For each:

1. Load `task_record` and (if present)
   `task:T:ckpt:<last_checkpoint.step_seq>`.
2. Build a replay `EvalCtx::IO` whose trace lookup routes through
   `task:T:trace:<step_seq>` and whose `step_seq` counter starts at
   `last_checkpoint.step_seq + 1` (or 0 if no checkpoint).
3. Enqueue the task on the task scheduler. It will re-enter the
   evaluator on the next tick.
4. The evaluator re-runs the program with `input = ckpt.state` (or
   `input = task_record.input_iri` if no checkpoint yet). Every IO
   dispatch checks `task:T:trace:N` first. **Observation** hits
   (tier 2) return the stored result — these are the only IO calls
   that we cannot safely re-dispatch, because the outside world may
   no longer be there. **Derived** hits (tier 3) are equally valid as
   an optimization but the call can also be re-run deterministically
   if the entry is missing. The first missing observation is where
   live execution resumes.

This gives O(M) replay where M is the number of observation traces
since the last checkpoint, not O(N) across the task's history.

Cancelled and failed tasks are not re-enqueued. Completed tasks'
results are served from their final checkpoint on demand.

---

## 7. Storage schema additions

On top of D13's keyspace:

```
session:<session_id>:task:<task_id>:meta
    TaskRecord = {
        session_id, task_id, program_iri, input_iri, status,
        layer_head, step_seq, last_checkpoint, latest_trace_seq,
        created_at, updated_at, result_layer_head?
    }

session:<session_id>:task:<task_id>:trace:<step_seq>
    ComponentTrace (existing type, positionally keyed)

session:<session_id>:task:<task_id>:ckpt:<step_seq>
    Checkpoint = { session_id, task_id, step_seq, state, created_at }

session:<session_id>:task:<task_id>:cancelled/trace:<step_seq>
    ComponentTrace (shadow keyspace for force-abort late arrivals, §8)

memo:<layer_head_hex>:<sha256_hex>
    ComponentTrace (cross-task memo for Pure/Read components only;
    layer_head scopes the memo to a fixed layer chain, §3.3)
```

In 9b-iii, `session_id` is always `Uuid::nil()` (§3.7). Elsewhere in
this document keys are written without the `session:<nil>:` prefix
for brevity — e.g. `task:T:trace:N` is shorthand for
`session:<nil>:task:T:trace:N`. The full form is what hits the
backend.

`session:<id>:task:*` keys sort together so per-session scans during
startup are cheap. `memo:*` keys are global and can be compacted
independently. Both use the `meta:` column family pattern Phase 9a
established via `PersistentBackend::put_meta`.

---

## 8. Decisions (walk-through)

The six questions below were worked through explicitly during the
draft review. Each bullet records the question, the chosen resolution
in a *Decided:* lead, and the full rationale. The one-line summary of
everything decided in this document lives in §10; use this section
when you want to know *why* a particular call was made.

- **Layer-head pinning.** *Decided: pin at entry to the session's
  active top.* A task records the session's active top in
  `TaskRecord.layer_head` when `RunProgram` accepts the request, and
  every Read-component dispatch during the task reads against that
  specific layer for the task's lifetime. Loads committed during the
  task — whether in the same session or another — are invisible to
  it. This gives Read-component determinism, aligns memo scope
  (§3.3) with task scope, and keeps the simpler default. In 9b-iii
  the session's active top is the kernel's single head (§3.7), so the
  pinned head is the same thing you'd have expected pre-sessions; in
  Phase 14 with multiple sessions, pinning is per-session. Cost:
  `TaskRecord` grows by `session_id: Uuid` and `layer_head: LayerId`;
  resume loads the specific layer rather than the current head.

- **Post-crash head advance.** *Decided: keep the pinned head across
  crash.* Resume rehydrates the task's chain from its pinned
  `layer_head` via `load_chain(head_id)`, regardless of where the
  kernel's live head is now. If the pinned layer is no longer in the
  store (D13's invariants guarantee it is, unless a future Phase-14
  reconciliation flow has rewritten history), resume fails loudly
  rather than silently substituting a different head — the task is
  marked `Failed` with a `PinnedLayerMissing` error and a
  human-readable remediation hint.

- **Atomicity of trace + checkpoint + meta writes.** *Decided:
  per-step RocksDB `WriteBatch`.* Every task step batches the IO
  trace write (`task:T:trace:N`), the meta update (`task:T:meta`),
  and — on checkpoint steps — the checkpoint write (`task:T:ckpt:N`)
  into a single atomic commit. The `PersistentBackend` trait gains a
  `write_batch(ops: &[BatchOp]) -> Result<(), StorageError>` method
  with `BatchOp ∈ { Put(key, value), Delete(key) }`. `RocksStore`
  maps it to `rocksdb::WriteBatch`; the in-memory backend applies the
  ops sequentially under its existing lock (trivially atomic with no
  concurrent observer). All of D21's multi-key updates go through this
  primitive, making step-level crash atomicity a property of the
  backend layer rather than something the task code has to enforce.

- **Resume scheduling.** *Decided: bounded-parallel, background,
  FIFO, config-tunable retry cap.* On startup, the kernel scans for
  `Running` / `Suspended` tasks, opens the gRPC server immediately
  (resume does not block the listener), and spawns up to
  `max_parallel_resumes` (default 4) resumption workers. Workers pull
  from the resume queue in `created_at` order. Each task gets up to
  `max_resume_attempts` (default 1) attempts per kernel startup; on
  exhaustion, the task transitions to `Failed` with the underlying
  cause recorded. Both limits are server-config knobs — a laptop
  keeps 4/1, an Azure deployment can raise them.

  **Observability requirement (derived).** Because resume runs in the
  background and may produce new outputs (task results, committed
  layers on the pinned chain, external side-effects) after the user
  is already interacting with the kernel again, the server must
  surface that background work clearly:

  - `Health` reports a `resume_in_progress: bool` and a
    `tasks_resuming: u32` so clients can tell that startup isn't yet
    quiet.
  - `ListTasks` continues to be the authoritative status view; a
    filter for `status = Running \| Suspended` lets clients poll the
    tail of the resume queue.
  - A session (§3.7) that opens while resume is in flight should be
    shown the set of resuming tasks whose pinned layer is an ancestor
    of the session's chosen active top — those are the ones whose
    eventual commits will be visible in that session. The exact UX is
    a §3.7 concern; D21 only guarantees the data is available.

  Task-level streaming events (notify-on-completion) are a later
  addition; polling `ListTasks` is adequate for v1.

- **Cancellation semantics.** *Decided: cooperative with deadline and
  shadow keyspace.* `CancelTask(task_id)` transitions the task to
  `Cancelling` and sets an `AtomicBool` flag that the evaluator
  checks between IO dispatches. If the task stops cleanly within
  `cancel_grace` seconds (default 30s, config-tunable), it
  transitions to `Cancelled`. If not, the kernel force-aborts the
  tokio task. Any late-arriving IO traces from dispatches that
  completed after the force-abort land in a `cancelled/<task_id>/trace:<N>`
  shadow keyspace rather than `task:<task_id>/trace:<N>`, preserving
  audit data without corrupting the main task log or the resume path.
  Shadow-keyspace entries are not consulted during resume (the task
  never resumes) and are retained under the same audit policy as
  observation traces (§5.4).

- **Audit retention.** *Decided: one TTL knob + per-task override;
  factory default is `unlimited` with a loud startup warning.* The
  machinery is a single setting, `--audit-retention
  <duration|unlimited>` (env `EIGENIUS_AUDIT_RETENTION`), plus an
  optional `retain_forever: bool` on `RunProgram` that overrides the
  server default for compliance-critical tasks. Different deployment
  profiles set different defaults — the laptop binary / dev Docker
  Compose ships with `unlimited` (paranoid, never silently deletes),
  an Azure production deployment can override to a conservative TTL
  in its Bicep config. When retention is `unlimited`, the kernel
  emits a `WARN`-level log line on startup: *"Audit retention
  unbounded; observation traces will accumulate without automatic
  cleanup. Set --audit-retention to bound disk growth."* — so the
  operator sees the knob exists and can choose. This keeps the design
  one mechanism, surfaces the tradeoff, and lets the deployment layer
  (not D21) decide what's right for a given environment.

---

## 9. Clarifications

These are not open questions — they're consequences of the design
called out here so readers don't have to re-derive them.

- **Multiple tasks with the same (program, input).** The behaviour
  falls out of the program's capability level, not a policy knob. A
  program composed entirely of Pure/Read components is deterministic:
  two tasks over the same input produce identical traces, and the
  cross-task memo side-table collapses the redundant work to one
  dispatch per content hash. Any program containing IO components is
  nondeterministic by construction — each task gets its own trace log
  under `task:<id>:trace:*`, and repeated runs may diverge legitimately
  (a `dequeue` returns different events; a `Now()` returns different
  timestamps). The per-task log exists precisely so those divergences
  can coexist without colliding in the cache.

- **Checkpoint size.** Checkpoints are a replay-cost optimization,
  not a correctness requirement — §3.5 tier 3. If a task's accumulator
  grows large enough that snapshotting it hurts, the program can
  simply checkpoint less often (or stop checkpointing entirely);
  resume still works from the observation log, just with more replay.
  Two future optimizations when a real workload hits the limit:
  content-addressable blob storage (checkpoint stores state hashes
  into a dedup table) or delta-encoded checkpoints (each ckpt records
  the diff against its predecessor). Neither is in scope for 9b-iii.

- **Cross-task memo correctness under institution installs.** A Pure
  component's output can in principle change after a new layer commits
  (it reads from the layer stack). The memo key must include the
  `head_id` at dispatch time, or memo must be invalidated on every
  commit. Simplest is to *include the head LayerId in the memo key* —
  this scopes memoization to a fixed layer chain. Memoization within a
  session works; across commits it just re-fires. Good enough for 9b.

- **Checkpoint commit inside non-IO contexts.** `Checkpoint` as an IO
  component means it can only appear in IO programs. Pure programs
  don't need checkpoints (they can re-run deterministically). Read
  programs *might* want them for long cross-layer analyses; we can lift
  `Checkpoint` to Read later if needed.

- **Interaction with tracing formats.** The `ComponentTrace` wire
  format doesn't change. Only the key changes. That keeps the trace
  schema backward-compatible with existing `ComponentTrace` consumers
  (the `Reflect` RPC, D6b trace viewers).

---

## 10. Decisions summary

All decisions from §8 plus the architectural choices woven through
§§3–7, in table form for quick reference.

| Question | Decision | Rationale |
|----------|----------|-----------|
| Trace identity | Per-task positional `(task_id, step_seq)` | Streams don't work under content-address; positional keys match the "log of observations" mental model |
| Content-address cache | Retained as a cross-task memo side-table, restricted to Pure/Read | Preserves the Phase-9a speedup for deterministic components; can't mis-apply to IO |
| Three-tier durability | Program (in layers) + observations (IO traces) + derived state (Pure/Read traces + checkpoints) | Only observations are irreplaceable; everything else is reconstructible, and retention follows from that |
| Checkpoint role | Replay-cost optimization, not primary data | Drop-in checkpoint = more replay; correctness unaffected |
| Checkpoint commit | Explicit via `components:Checkpoint` built-in | Policy belongs in the program; kernel provides the primitive |
| Pruning | Drop derived traces on checkpoint commit; keep observation traces per audit policy; hard cap on observation count | Observations are the audit trail; derived state is free to collapse |
| Resume input | Latest checkpoint's `state`, or the original `input_iri` if none | Simple rule, covers both newly-started and long-running tasks |
| Component determinism gate | `capability_level` ∈ {Pure, Read} | Uses what we already have; no new ontology surface |
| Task storage | New `task:<id>:*` keyspace on the existing PersistentBackend | Sits next to layers and meta; no new backend trait needed |
| Layer-head pinning | Pin at `RunProgram` entry to the session's active top, store in `TaskRecord.layer_head` | Read-component determinism; aligns memo scope with task scope |
| Post-crash head on resume | Use pinned head; loud failure if unresolvable | Crashes must not silently change the task's world-view |
| Task-scoped layer reads | New primitive: evaluator runs against a caller-supplied `Arc<Layer>` | Required by pinning; already supported by `EvalCtx`, needs exposure |
| Sessions (9b-iii) | Single hardwired session `session_id = Uuid::nil()`; data model and keyspace carry `session_id` from day one | Avoids a Phase-14 data migration; session == kernel's single head in v1 |
| Sessions (Phase 14) | Multi-session: `CreateSession`/`SwitchActiveTop`/etc., independent active tops, optional `session_id` on RPCs | Pull expansion when branching + multi-user work lands |
| Task result layers | Task commits a result layer with `parent = pinned head` on completion | Results are first-class artifacts on the chain, auditable, queryable |
| Auto-advance vs fork | Session auto-advances active top to result layer iff `pinned_head == session.active_top`; otherwise the result is a fork reachable via `at_layer` | Keeps single-session linear timeline; preserves correctness under rare concurrency |
| Read-path non-top pin | `at_layer: LayerId` optional field on `Query`/`Inspect`/`ListInstitutions`/`GetSchema`; CLI `--at-layer` flag | Without it, forked task results are stranded; pulls read-side of §3.6 into 9b-iii |
| Write-path non-top pin | Deferred past 9b-iii | Not needed to access results; architecture doesn't preclude it |
| Step atomicity | Per-step `WriteBatch` via new `PersistentBackend::write_batch` | Crash can't leave partial trace/meta/ckpt state |
| Resume scheduling | Bounded-parallel (`max_parallel_resumes=4`), background, FIFO by `created_at`, `max_resume_attempts=1` | Fast recovery without thundering; gRPC stays responsive during resume |
| Resume observability | `Health` counters + `ListTasks` filter; session-aware surfacing via §3.7 | Users must be able to see background work after a crash |
| Cancellation | Cooperative with `cancel_grace=30s` deadline; force-abort spills to `cancelled/` shadow keyspace | Safe default; escape hatch for stuck IO; audit preserved |
| Audit retention | Single `--audit-retention` knob + per-task `retain_forever`; factory default `unlimited` with startup WARN | One mechanism; never silently deletes; deploy profiles override |

---

## 11. Implementation plan (for Phase 9b-iii)

1. Extend `PersistentBackend` with (a) a `write_batch(ops)` primitive
   for step-atomic multi-key writes (§8 decision), and (b) task-
   keyspace helpers or a thin `TaskStore` trait wrapping the backend
   (mirror of `BackendTraceStore`). `RocksStore::write_batch` maps to
   `rocksdb::WriteBatch`; in-memory backend applies sequentially.
2. Introduce the `Session` type: `{ session_id: Uuid, active_top:
   LayerId }`. v1 instantiates exactly one — `session_id = Uuid::nil()`
   — inside the kernel server, seeded from the current head. `Load`
   commit-through advances `session.active_top` linearly.
3. Add `TaskRecord` (with `session_id: Uuid`, `layer_head: LayerId`,
   `step_seq`, `last_checkpoint`, `status`, `result_layer_head:
   Option<LayerId>`, etc.) and `Checkpoint` types + CBOR encoding.
   All keyspace writes include the `session:<nil>:` prefix from day
   one.
4. Build `TaskEvalContext` that wraps an `Arc<Layer>` resolved from a
   caller-supplied `LayerId` (pin-at-entry, §3.6). The existing
   `EvalCtx::IO` composition already takes `Arc<Layer>`; the new
   piece is the higher-level constructor and its `load_chain(head_id)`
   call.
5. Re-key the evaluator's IO trace lookup through
   `(session_id, task_id, step_seq)`. Existing content-address
   lookups become the Pure/Read memo fallback (§3.3), with the memo
   key including `layer_head` for correctness under institution
   installs.
6. Add the `components:Checkpoint` built-in and wire it through
   `write_batch` so the checkpoint, trace, and meta update land as
   one atomic step.
7. Add `RunProgram` async path: spawn a tokio task, pin the session's
   active top into `TaskRecord.layer_head`, return `task_id`
   immediately, record the task on entry and on every status change
   (always via `write_batch`).
8. On task completion, commit the result as a new layer with
   `parent = TaskRecord.layer_head` and store its id in
   `TaskRecord.result_layer_head`. If `pinned_head ==
   session.active_top`, auto-advance `session.active_top` to the
   result layer (§3.7 fast-forward); otherwise leave the session
   active top untouched and flag the task's result as a fork.
9. Add `at_layer: LayerId` optional field to `Query`, `Inspect`,
   `ListInstitutions`, and `GetSchema` request messages (proto
   change). Server resolves the requested layer via `load_chain` and
   runs the read against it; falls back to `session.active_top` when
   the field is absent. CLI exposes `--at-layer <id>` on the
   corresponding verbs.
10. Add `ListTasks` / `GetTaskStatus` / `CancelTask` RPCs.
    `GetTaskStatus` returns `result_layer_head` for completed tasks
    so clients know where to point `at_layer` reads. `CancelTask`
    implements the cooperative-with-grace-deadline semantics (§8
    decision): flip `AtomicBool`, transition to `Cancelling`,
    force-abort after `cancel_grace`, route any late-arriving traces
    to the `session:<nil>:task:<id>:cancelled/trace:<N>` shadow
    keyspace.
11. Add the resume sweep to `start_server` after D13's seed/resume
    path. Scan `session:<nil>:task:*:meta`, filter
    `Running|Suspended`, enqueue by `created_at`, drive up to
    `max_parallel_resumes` workers in background. Each task gets up
    to `max_resume_attempts` tries; exhaustion → `Failed`. The gRPC
    server starts listening before resume completes.
12. Expose resume-in-progress via `Health.resume_in_progress` +
    `Health.tasks_resuming` counters and an enhanced `ListTasks`
    filter for active status. (Full §3.7 session-aware surfacing
    lands with multi-session in Phase 14.)
13. Pruning pass on checkpoint commit (drop derived traces) and on
    task completion (drop derived; retain observations per
    `--audit-retention` / per-task `retain_forever`).
14. Surface config knobs: `--max-parallel-resumes` (default 4),
    `--max-resume-attempts` (default 1), `--cancel-grace-seconds`
    (default 30), `--audit-retention` (default `unlimited`, with
    startup WARN when unbounded).
15. Integration test: start a long-ish IO-driven task, kill the
    kernel mid-flight, restart, verify (a) resume from the latest
    checkpoint, (b) pre-checkpoint derived traces are pruned, (c)
    observation traces are retained, (d) `Health` exposes
    resume-in-progress during the ramp, (e) task result layer is
    committed on completion with correct parent, (f) session
    auto-advances on fast-forward, stays put on fork, (g) `at_layer`
    read reaches a forked result, (h) cancellation of a stuck task
    force-aborts after grace and spills to shadow keyspace.

---

## 12. References

- D11 §5 — Resumable Execution
- D13 §5–7 — Durable kernel state and commit-through
- `kernel/src/program/trace.rs` — existing `ComponentTrace`,
  `TraceStore`, `compute_trace_key`
- `kernel/src/server/mod.rs` — `BackendTraceStore` (Phase 9a)
- Xia et al., *Interaction Trees* (2020) — positional trace semantics
  for coinductive programs
- Hancock & Setzer, *Interactive Programs in Dependent Type Theory*
  (2000) — observations as the unit of IO identity
