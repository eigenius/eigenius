# D80 — Witness and institution machinery

**Status: design.** Split out of [D77](d77-merge-as-a-pushout-of-environments.md) on `2026-08-25`.
Depends on [D79](d79-the-representation-of-inductive-types.md).

**Two facts earned under a binding survive that binding changing.** Witness credit (§2) and
institution verdicts (§3) are both computed against a chain, recorded, and never rechecked when the
chain rebinds a name they rest on. Both are witnessed by standing tests or by reading the commit path.
Neither is a merge defect — both fire on an ordinary linear commit.

**Why this comes before D77.** D77 makes merge check that a rebinding did not invalidate what was
checked against the old binding. That requires knowing what "recheck" means for each kind of thing the
chain holds. For resources it is settled: revalidate, or re-type-check. For witnesses and verdicts it
is not — and the answers turn out to be different from each other and different from resources
(§2.1 and §3.2 are both corrections of a first answer that looked obvious). Building the merge pass
on top of an unsettled notion of rechecking would wire it to the wrong recheckers.

---

## 1. The shared shape

Both defects instantiate one pattern:

1. a fact is established against the chain as it stands — witness credit admitted, a verdict computed;
2. a later layer **rebinds** a name that fact rests on;
3. the rebinding **widens** what the name admits, so the fact is now a claim about something stronger;
4. nothing rechecks.

Step 3 is the discriminator and it is the same predicate in both cases — D78's `conjunction_entails`.
D75 §3.4 states it for witnesses: *"Narrowing the class shrinks the domain and leaves stale credit
sound by accident; only widening exhibits the unsoundness."*

Where they diverge is step 4's remedy, and neither remedy is the obvious one.

---

## 2. Witness credit survives a rebinding

D75 §3.4 and §3.5 are siblings, and only §3.5 was given a follow-on:

| | §3.4 — witness credit | §3.5 — merge (#225) |
|---|---|---|
| trigger | a **descendant layer** rebinds a name | a **merge** rebinds a name |
| what was checked against the old binding | witness credit for `Π(x : Dog). P` | a resource neither branch changed |
| direction that makes it unsound | `Dog` **widens** | the binding **weakens** |
| what rechecks it | nothing | nothing |
| follow-on | **none assigned** | this document |

The direction criterion is the same predicate. D75 §3.4: *"Narrowing the class shrinks the domain and
leaves stale credit sound by accident; only widening exhibits the unsoundness."* That is
`conjunction_entails` (§2.1), reached from the other side.

**This falsifies §2.5's heading.** "The linear analogue is complete" is true of `retroactive_validate`
for **shape** dependents and false for **term** dependents — witness credit is a term-level fact about
what a proposition mentions, and no linear pass rechecks it. So the two-relations split of §3.2 is not
merge-specific: the linear path has the same hole, and the witness index is where it shows.

That strengthens the case for D79 §2.2 rather than complicating it. Once `core:mentions` exists, *"which
witnessed propositions mention `Dog`"* is the same range query as *"which resources mention `i`"*, and
the fix has the same two halves — enumerate, then invalidate credit — discharged at commit through
`retroactive_validate` rather than at `lookup_chain_witness`, which is a hot path and the wrong place
to put a recheck.

### 2.1 Who owns a witness, and why the obvious fix does not work

The `JustifiedBy` vocabulary belongs to the **Justification Logic institution** (D39) —
`reasoning:reasoning_institution`, whose `ValidateJustification` QueryClass is **AutoOnLoad** and
fires on every `ReasoningSentence` commit. But the institution declares
`institution:runtime = runtimes:in_process` and says why: *"The validator is the kernel — no external
runtime — so verification is a direct function call with TCB bounded by the kernel's type-theory
implementation."*

| | owner |
|---|---|
| the `JustifiedBy` / `JustificationTerm` vocabulary | the reasoning ontology (the institution's) |
| **when** a witness is demanded | the institution's `ValidateJustification`, AutoOnLoad at commit |
| **synthesis** — inhabiting a `JustifiedBy.*` argument position | the **kernel type checker** (`nbe/check/witness.rs` → `EffectHooks::synthesize_chain_witness`) |
| **admission** — does the chain admit this key | the **kernel** (`layer/witness_index.rs`), a pure function of Trace-class resources — *nothing persisted* |

So the institution owns the vocabulary and the trigger; the kernel owns creation and checking. That
places the reasoning institution squarely in §3.3's row-2 cell — in-process, AutoOnLoad, no
`runtime_invocation` — which invites the conclusion that §3's re-dispatch covers it.

**It does not, and this is the correction.** The first answer written here was "invalidate credit at
commit." Both that and §3's re-dispatch fail, for the same reason:

- **Re-dispatching `ValidateJustification` against `Γ_merge` returns the same answer.** It decodes the
  proposition, builds `JustifiedBy(j, p)`, type-checks the certificate, and synthesis calls
  `lookup_chain_witness` — which walks to the ancestor, matches the key, and says yes. The key is
  environment-blind (D75 §3.4), so re-running the check reproduces the wrong verdict rather than
  correcting it. **The defect is not that the check was skipped; it is that re-running it does not
  help.**
- **Invalidating the trace is wrong on the merits.** The declaration trace attested something true —
  `Π(x : Dog). P` *was* declared, under `Dog`-v1. What fails is the inference from that trace to
  credit for the *stronger* proposition `Dog`-v2 induces. The trace is not the defective party.

**So the fix is at the lookup.** `lookup_chain_witness` must refuse a hit from a layer that binds the
proposition's names differently from the querying layer. The two pieces exist: D79 §2.2's `core:mentions`
supplies the name set for the attested proposition without decoding it, and D77 §2.1's
`conjunction_entails` decides whether the difference is a widening. Cost is confined to hits from a
layer *other than* the querying one — first-hit-wins within a layer is unaffected — so the hot path
stays hot.

This is still not making the environment part of proposition identity (§5): the *key* is unchanged and
existing witnesses are not reforked. Only the walk that consumes it becomes binding-aware.

## 3. Institution verdicts survive a rebinding

D79 §1.2 split the dependency relation in two. There is a third, and it is the one an institution verdict
lives on. A `reflection:InstitutionEmittedDerivation` — the statistics institution emits one per ANOVA
effect — is a verdict computed from a gated analysis spec and its data. Rebind the bound dataset or an
experimental-design parameter and the verdict no longer follows, **while every resource on the path
stays structurally valid and type-correct**. Nothing is invalid. The verdict is *unsupported*.

| relation | reaches `i` via | closure | recheck |
|---|---|---|---|
| **shape** | `is_a`, property value, property key | one hop — validity is local | `validate_resource` |
| **term** | `ConstRef` in an encoded term | one hop — the term is checked in `Γ` | re-type-check in `Γ_merge` |
| **support** | `from_subject`, `runtime_invocation`, `runtime:inputs`, `derivation_trace` | **transitive** | **§3.2** |

**It is transitive, and the other two are not.** One hop suffices for shape and term because validity
and type-correctness are *local*: revalidating `R` against the merged chain settles `R`, and `R`'s own
dependents are unaffected because `R` did not change. Support does not work that way — a rebound
dataset invalidates a derivation through `invocation → inputs`, and that derivation may itself be an
input to another. `enumerate_dependents` is a single pass over the new layer's `defined_iris()`
(`retroactive.rs:91`) with no fixpoint, so it cannot reach past one hop by construction.

### 3.1 The staleness question is decidable from the index

`runtime:inputs` is `core:resource_array` ("ordered list of input resource IRIs"); `from_subject`,
`runtime_invocation`, `runtime:script` and `environment` are `core:resource`; the invocation pins
`image_digest`, and D53 §6.1 file-backed observations carry a `content_hash` the kernel verifies. Every
provenance edge is therefore an indexed triple under the existing rule — no D79 §2.2 extension is needed
for this relation, only a transitive closure over a **named edge set** rather than over all reference
edges, which would reach the whole chain.

So *"was this verdict computed from something the rebinding moved"* is answerable by closure over the
index, for every institution, without running anything.

### 3.2 Whether it can be *rechecked* is declared, not assumed

An earlier draft of this section asserted that a verdict's recheck is re-execution and that a merge
commit therefore cannot perform it — "the verdict is the institution's to issue, not the kernel's to
recompute." **That is false for the institution this section is about.** `eigenius-statistics` is a
kernel crate whose Cargo manifest states the design directly: *"The verifier is in-process and reads
SampleSets from the chain via the kernel's resource/value machinery; no external prover or worker."*
It is `ndarray` + `statrs`, deterministic, with chain-resident or content-hashed inputs.

The distinction is **declared on the verdict itself**. `institution:runtime_invocation` is documented
as *"Set when the dispatching institution was external-runtime (D31 §6.3); absent for in-process /
WASM dispatches whose provenance is program-trace-only."* Its presence or absence decides whether
re-execution is admissible inside a commit — so this is a property read, not a hard-coded list of
institutions.

**And re-execution is already a commit phase.** `dispatch_auto_on_load_for_layer`
(`commit/phases.rs:411`) fires every AutoOnLoad QueryClass for the layer being committed. For an
in-process institution, recomputing the verdict against the merged chain is not new machinery; it is
the pipeline doing what it already does.

### 3.3 Three cells, and what serves each

Crossing "can it re-execute in-process" with D77 §2.4's materialisation invariant — the merge layer holds
what *either branch changed*, and nothing else:

| verdict | carrier in the merge layer | disposition |
|---|---|---|
| in-process (no `runtime_invocation`) | **yes** — a branch changed the spec | re-fires under D77 §4(a) **for free**; skipped entirely under (b) |
| in-process | **no** — spec unchanged, its *data* was rebound | W3 enumerates it, then **re-dispatches** AutoOnLoad for that subject |
| external-runtime | either | **mark** — `InvalidatedTrace`; re-execution is out of scope |

Row 2 is #225's shape exactly: the
at-risk carrier is one whose own resource did not change, so no amount of pipeline routing reaches it.
It needs the enumeration (W3).

**Row 2 does not subsume the witness case.** The reasoning institution sits in this cell — in-process,
AutoOnLoad, no `runtime_invocation` — but re-dispatching it reproduces the wrong verdict rather than
correcting it, because the witness key is environment-blind. §2.1 gives the reason and W1 the fix.
Re-dispatch repairs a verdict whose *inputs* moved; it cannot repair one whose *lookup* is
binding-blind.

**Row 1 is a present defect on the merge path.** Merge today ends at `store_layer` (D77 §4, failure 2),
so `dispatch_auto_on_load_for_layer` never runs for a merge — an analysis spec contributed by a branch
produces no verdict at all, and one whose inputs moved keeps the old one. D77 §4 argues for (a) on the
structural ground that the checking path and the resolution path should not be two paths; this is the
same argument with a witness attached, and it is not hypothetical.

Row 3 is where the original "mark, don't recompute" answer stands, and it stands for the right reason:
not that the kernel lacks authority, but that a merge commit cannot be unbounded in time or depend on
a foreign runtime. That is `CascadeItem::InvalidatedTrace { trace, reason }`, which D20 §8 reserved for
exactly this — *"A trace references content that becomes inconsistent."*

**What is out of scope**, stated so it is not mistaken for an oversight: dispatching *external-runtime*
institutions, scheduling their re-execution, and any notion of a verdict's numerical stability. D77
re-dispatches only what the commit pipeline already re-dispatches on an ordinary load.


---

## 4. Phases

- **W0 — what does revocation mean?** Shared by §2 and §3, and the reason neither phase starts with
  code. When credit is withdrawn or a verdict marked stale: does the carrying claim survive as
  unsupported, does `JustifiedBy` start failing, is anything scheduled for re-execution? Output is a
  decision recorded here. **No code.**
- **W1 — binding-aware witness lookup** (§2.1), gated behind D79 P3. `lookup_chain_witness` refuses a
  hit whose layer binds a name the attested proposition mentions more widely than the querying layer
  does — name set from `core:mentions`, comparison by `conjunction_entails`. **Not** re-dispatch (it
  reproduces the wrong answer) and **not** trace invalidation (the trace attested a true thing).
  Gates: `witness_credit_survives_redefinition_of_a_class_the_proposition_quantifies_over`
  (`witness_index.rs:1184`) **flips** and is renamed, closing D75 §3.4; a *narrowing* redefinition
  still admits, since only widening is unsound; same-layer first-hit-wins is untouched, asserted by a
  lookup benchmark on the unchanged path.
  **`redefining_a_class_does_not_change_the_hash_of_a_proposition_over_it` (`:1133`) must NOT flip** —
  it guards proposition *identity*, which §5 keeps environment-blind. W1 changes the walk, not the
  key. A W1 that flips both has changed the wrong thing.
- **W2 — AutoOnLoad on the linear path.** Establish the baseline §3.3 assumes: that
  `dispatch_auto_on_load_for_layer` (`commit/phases.rs:411`) re-fires for a resource whose *inputs*
  were rebound by the layer being committed, not only for one the layer defines. Gate: rebinding a
  dataset under an unchanged analysis spec recomputes the verdict on an ordinary commit, or is
  recorded as not doing so — which is a finding, not a failure.
- **W3 — provenance closure.** Transitive closure over `from_subject` / `runtime_invocation` /
  `runtime:inputs` / `derivation_trace`, bounded to that named edge set. Then split on the verdict's
  `institution:runtime_invocation`: **absent** ⇒ re-dispatch the AutoOnLoad QueryClass for that
  subject; **present** ⇒ mark. Gates: rebinding a dataset two hops below an in-process
  `InstitutionEmittedDerivation` **recomputes** it, and a `Fails` verdict surfaces rather than being
  silently replaced; the same shape with an external-runtime verdict **marks** and dispatches
  nothing; the closure is bounded — a chain lacking the edge set enumerates nothing, asserted so it
  cannot degrade into a full reference walk.

**W2 before W3, deliberately.** §3.3 reasons about which cells the commit pipeline already covers. If
W2 finds it covers fewer than assumed, W3's scope grows and the merge-side conclusion in D77 changes
with it.

---

## 5. What this does not cover

- **Merge.** Both defects fire on a linear commit and are fixed there. D77 consumes the result: once
  "recheck" has a definition for witnesses and verdicts, the merge pass has recheckers to call.
- **Proposition identity stays environment-blind.** W1 changes the lookup walk; it does not put the
  layer into `hash_proposition_exp`. Making the environment part of proposition identity is the
  alternative fix for D75 §3.4 and a much larger change — it forks every existing witness key.
  `witness_index.rs:1136` already names the assertion that would have to move.
- **External-runtime institution dispatch.** W3 re-dispatches *in-process* verdicts only. Verdicts
  produced by an external runtime are marked, not recomputed: re-execution there is unbounded in time
  and depends on a foreign runtime. Scheduling it is a separate surface.
- **Numerical stability.** Whether a recomputed verdict *should* differ is the institution's question.

## 6. References

- D75 §3.4 (proposition identity is environment-blind — witnessed), §3.5 (the merge sibling)
- D39 (justification logic), D49 §6 (chain-witness admission), D52 (institution-emitted derivations)
- D20 §8 (`CascadeItem::InvalidatedTrace`), D78 §4 (`conjunction_entails`)
- D79 (`core:mentions`, which W1 and W3 both consume)
- `layer/witness_index.rs`, `nbe/check/witness.rs`, `program/check_hooks.rs`,
  `commit/phases.rs` (`dispatch_auto_on_load_for_layer`), `crates/eigenius-reasoning/src/validate.rs`
