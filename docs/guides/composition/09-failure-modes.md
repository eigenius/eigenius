# 9. Failure modes across compositions

Multi-institution flows have failure modes single-institution flows don't.
Validation cascades through nested resources; comorphism dispatch can fail
at any of the four pipeline steps; AutoOnLoad gates can race against
OnDemand calls; provenance gaps appear under mixed hosting. This chapter
is the survival guide.

For per-domain failure modes (FormulaTerm encoding errors, operator-arity
mismatches, validator rule rejections), see the
[formula language guide §7](../formula/07-failure-modes.md). For
operational failures of the platform itself (kernel won't start, env
images won't build), see [platform §13](../platform/13-troubleshooting.md).
This chapter covers the *cross-composition* failure modes those don't.

## 9.1. Validation cascade failures

Chain validation walks nested resources. When a comorphism's reify output
is committed, it's validated against the target class's `requires`,
`recommends`, and any constraints (Decidable predicates attached to
properties). A failure deep in the cascade can be hard to read because
the error message points at the leaf, not at the comorphism that caused
the leaf to land.

### Reading a cascade failure

A typical error from `eigenius load` after a comorphism dispatch:

```
Load failed:
  StructuralValidation: missing required property
    resource: urn:eigenius:comorphism-output:foo:abc123
    class:    urn:eigenius:target:Bar
    property: urn:eigenius:target:critical_field
```

What this is telling you: the target institution's `reify` handler
produced a resource that's missing a required property of its target
class. Three plausible causes:

1. The institution's `reify` handler has a bug — it forgot to set
   `critical_field`.
2. The chain's class declaration changed (a property got promoted from
   `recommends` to `requires`) without the institution being rebuilt
   against the new declaration.
3. The transformation Component returned a payload missing a field the
   `reify` expected (a typed-mismatch deeper in the pipeline that
   surfaced only when reify tried to construct the target).

### Tracing back through the pipeline

Walk the trace tree from the failed commit:

1. The failed commit's `Trace::Comorphism` event names the comorphism
   IRI. Run `eigenius inspect <comorphism-iri>` — confirm the triple
   is what you expect (export / transformation / import).
2. The `Trace::Comorphism.source_trace` field carries the trace of the
   source-expression evaluation. Walk it to see what input the
   comorphism received.
3. If the comorphism's institutions are external-runtime hosted, the
   `RuntimeInvocation` provenance carries the env image digest. Confirm
   the image is the expected version — a stale env image running an
   old `reify` is a common cause.

For substrate-hosted institutions, the worker container's stdout may
carry a more specific error than the kernel's commit-rejection message.
`docker logs <orchestrator>` after a failed dispatch usually surfaces
the worker's panic or error.

### The locality-of-blame property

The platform tries to push validation to *commit time* of the
comorphism *declaration*, not the dispatch. So:

- Type-aligned comorphism + bad reify implementation → fails at
  dispatch; error points at the reify output's missing property.
- Misaligned comorphism (transformation output type ≠ import payload
  type) → fails at chain commit of the comorphism declaration; error
  points at the comorphism resource itself (chapter 3 §3.6).

The first case is the harder one to diagnose because the error is in
the institution's runtime behaviour, not in the chain's static
structure. Rule of thumb: if the error fires consistently for the same
input, it's a reify bug; if it fires non-deterministically, it's a
race or environment issue (§9.3).

## 9.2. Comorphism dispatch failures (extract / transform / reify)

Each of the four pipeline steps (chapter 3 §3.2) has its own failure
shape:

### Extract failures

The source institution's `extract_typed` handler errors. Typical
causes:

- Missing source-side property (the source resource doesn't have the
  field the export expects).
- Wrong type in the source-side property (the validator should catch
  this at the resource's commit time, but a stale validator + a fresh
  resource can slip through).
- Source institution is in-process / WASM and panics during extract;
  in-process panics often produce less-informative error messages than
  external-runtime panics (no RuntimeInvocation captured).

**Diagnose by:** running the extract handler against the same source
input via the per-institution test harness (e.g.
[`crates/eigenius-julia/tests/`](../../../crates/eigenius-julia/tests/)
for Julia institutions; the WASM SDK's test harness for WASM
institutions).

### Transform failures

The transformation Component's evaluator panics. Rare in v1 because
the v1 restriction is `capability_level ∈ {Pure, Read}` — the
evaluator can't dispatch IO Components — and pure / read components
are normally just data shuffling. The most common cause is a typed
mismatch the chain didn't catch: the extract returned a value whose
runtime shape doesn't match its declared `payload_type`.

**Diagnose by:** running the comorphism's transformation Component in
isolation against the extract output. The kernel's NbE evaluator can
be invoked from Rust against a known input; the per-institution
tutorials show the harness shape.

### Reify failures

The target institution's `reify` handler errors. Typical causes:

- The transformed payload doesn't satisfy the target class's
  invariants (the target's `reify` does its own validation).
- A property-level constraint on the target class fires — e.g. a
  Decidable predicate constraining a numerical range, applied to the
  reified value.
- Target institution panics during construction.

**Diagnose by:** the target institution's logs (its handler usually
surfaces a more specific error than the kernel's rejection message).

### Reinsert failures

Chain commit of the reified resource fails after reify returned. This
is the case §9.1 covered: the produced resource passed reify but
fails the chain's structural validation. Rare when the chain ontology
and the institution implementation are in sync; common when they've
drifted.

**Diagnose by:** comparing the chain's current view of the target
class (`eigenius inspect <class-iri>`) against the institution's
build-time view (the env image's mirror, which has a snapshot of the
class definition the institution was compiled against).

## 9.3. Chain-state races and stale Verdicts

AutoOnLoad gates fire synchronously per commit, but a multi-cell
notebook (or a multi-call program) can queue commits faster than
gates complete. Subtle but real:

### The setup

Cell 1 commits resource A, which triggers AutoOnLoad gate G_A.
Cell 2 commits resource B, which depends on A and triggers AutoOnLoad
gate G_B. The gates each take seconds; the user clicks "Run All" and
the cells fire back-to-back.

What can go wrong:

- **Cell 2's commit dispatch races against cell 1's gate completion.**
  In v1 the kernel serialises commits, so cell 2 *waits* for cell 1's
  gate to complete before validating cell 2's commit. The race doesn't
  produce a chain inconsistency, but it does produce visible delay
  ("cell 2 is hanging" — actually it's waiting on cell 1's gate).
- **A stale Verdict from a previous run.** If the chain has a Verdict
  on resource A from a previous run, cell 2's query of "what does the
  gate say about A?" can read the old Verdict before cell 1's new
  Verdict commits. The chain doesn't have transactional snapshots in
  v1, so reads can see in-flight state.

### What "stale Verdict" means

A Verdict is *stale* when it's bound to an old version of its
`verdict_subject` — the subject has been re-committed since the
Verdict was produced, but the chain hasn't re-fired the gate (because
nothing triggered it).

In v1 this isn't really a problem because Verdicts are bound to
content-addressable layer IDs — re-committing a resource produces a
new layer, and the old Verdict points at the old layer. Queries that
join Verdicts to subjects via IRI will see *both* (the old and the
new), with the chain's recency model deciding which to surface.

The pitfall: if a downstream query filters Verdicts by
`verdict_subject = <iri>` and doesn't account for layer recency, it
can return both Verdicts and the consumer has to decide which one is
authoritative. Best practice: include a layer-recency filter in the
query, or use the kernel's `inspect <iri>` which always returns the
top-of-stack view.

## 9.4. Provenance gaps under mixed hosting

A composition can mix WASM-hosted institutions, substrate-hosted
institutions, and in-process institutions. Each host kind produces
different provenance:

| Host kind | RuntimeInvocation | Trace::Comorphism | Notes |
|---|---|---|---|
| WASM | No | Yes | Wasmtime fuel/memory metrics may be available via the trace, but no env image digest because the binary is the institution. |
| Substrate (Julia) | Yes | Yes | Full provenance: image digest, runtime version, numerical metadata, started_at/completed_at. |
| In-process Rust | No | Yes | The trace says which Component ran; no RuntimeInvocation because there's no separate runtime. |

A *mixed comorphism* — say, source institution is in-process and target
institution is substrate-hosted — produces a partial provenance
closure. The trace records both endpoints; the RuntimeInvocation
captures only the substrate dispatch.

For audit-critical workflows (compliance, regulatory reporting),
**require external-runtime hosting on both sides** of any comorphism
that gates a covenant-level claim. The `RuntimeInvocation` is what
makes the dispatch reproducible; without it, a reviewer can't
re-execute and confirm the verdict.

For research / experimentation workflows, mixed hosting is fine —
the trace tree is enough to walk back what happened, even if a
reviewer can't bit-for-bit reproduce the dispatch.

## 9.5. Cross-host failure mode classification

A quick reference for "which host kind owns this symptom":

| Symptom | Likely host kind | What to check |
|---|---|---|
| `Wasmtime fuel exhausted` / `out of memory` | WASM | Component's fuel limit; SDK's WASM build flags. |
| `worker RPC failed: connect to worker UDS: No such file or directory` | Substrate | Depot bind-mount path mismatch (host vs. orchestrator container); orchestrator's substrate addon health. |
| `Pkg.precompile` errors during env build | Substrate (Julia) | Handler package's Project.toml; pinned dep versions; Julia version mismatch. |
| `manifest-hash mismatch` from worker | Substrate | The orchestrator container's bundled worker source is stale relative to the host's `eigenius env build`. |
| `panic` with no further context | In-process Rust | Kernel logs; the panic location is in the institution's Rust crate. |
| `NotImplemented` from a handler | Any | The institution declared a procedure (in `ExportFormat.procedure` etc.) but didn't implement the handler. Cross-check the institution declaration vs. the handler source. |
| `OperatorArityMismatch` at chain commit | Any | A FormulaTerm's App-spine has more args than the operator's signature has Pi binders. See [formula §7](../formula/07-failure-modes.md). |
| `seed manifest drift` on docker compose up | Substrate | Persistent DB volume seeded with old embedded ontologies that have since changed. `docker compose down -v` to wipe. |

For symptoms not in this table, the trace tree (`eigenius inspect <invocation-iri>`)
is usually the next step — it carries enough context to localise the
failure to a single institution and a single handler call.

---

Next: **[10. Appendix →](10-appendix.md)**
