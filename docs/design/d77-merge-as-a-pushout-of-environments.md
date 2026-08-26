# D77 — Merge as a pushout of environments

**Status: design.** No code. Closes [#225](https://github.com/eigenius/eigenius/issues/225), a
soundness defect reachable on a live RPC.

Implements the third of D75's three follow-ons, after
[D78](d78-resources-as-records.md) (Seam B) and [D76](d76-the-typing-environment.md) (Seam A), both
complete. D75 §7 ordered it last *"deliberately — both predecessors change what it checks and what it
is able to check"*. §1 below records what they changed, because it is more than that sentence
anticipated.

**Split, `2026-08-25`.** An earlier draft carried three projects. Writing them together was how their
dependency order became visible, and the order is the reverse of the order they were written in:

| | | |
|---|---|---|
| **[D79](d79-the-representation-of-inductive-types.md)** | the representation of inductive types | **buildable now**; prerequisite for both others |
| **[D80](d80-witness-and-institution-machinery.md)** | witness and institution machinery | defines what "recheck" means for a witness and a verdict |
| **D77** (this) | merge as a pushout | needs both |

D77 is last because it needs recheckers to call. For a *resource*, "recheck" is settled — revalidate,
or re-type-check. For a witness and for an institution verdict it was not, and both answers turned out
to differ from the resource one and from each other (D80 §2.1, §3.2). Building this pass first would
have wired it to the wrong recheckers.

---

## 1. The defect, and why it is not a missing guard

`MergeBranches` (`proto/eigenius.proto:120`, `server/branches.rs:427`) admits a merge in which a
resource's environment silently changed under it.

**The scenario**, witnessed by `a_reference_meeting_a_redefinition_raises_no_conflict`
(`layer/merge/conflict.rs:781`):

- the LCA defines class `C` requiring `{name, owner}`;
- branch **A widens** it to `{name}` — strictly more things are `C`;
- branch **B adds `R`**, which *references* `C` and does not define it.

`R` was validated in `Γ_B` against `C_LCA`. In the merge `C` resolves to `C_A`. Along `R`'s own path
`C` was never rebound, so Rule 22's retroactive scan never fires either.

**Three independent failures stack**, and the third is this document's subject:

| # | failure | where |
|---|---|---|
| 1 | nothing **detects** it — `shared_iris` is a set *intersection*, `{C} ∩ {R} = ∅` | `merge/conflict.rs:222-231`, `:689` |
| 2 | nothing **validates** it — the merge layer is built by verbatim copy and `store_layer` | `merge/resolve.rs:929-945`, `:1360` |
| 3 | the **backstop is unimplemented** — `InvalidatedSignature` is forward-compat only | `merge/cascade.rs:24` |

**Widening is the direction that matters.** Narrowing shrinks the class's extension and leaves
anything universally quantified over it sound by accident. The test widens deliberately.

**Not a guard on `shared_iris`.** Detection today is definition-vs-definition; the hazard is a
*reference* meeting a *redefinition*, which an intersection cannot express no matter how it is
patched. D75 §5 derives the shape instead: a merge is a **pushout of environments**, and *"recheck
what the pushout rebound"* is part of taking the pushout rather than an optional cascade a user
acknowledges. `InvalidatedSignature` stops being a reserved enum variant and becomes what merge *is*.

---

## 2. What D78 and D76 changed — anchored in the code as it now stands

D75 §7 predicted both predecessors would reshape this work. They did, and in one case they supplied
the algorithm outright.

### 2.1 D78 gave the widening check a name: `conjunction_entails`

The #225 scenario is a class widening from `{name, owner}` to `{name}`. That is exactly the relation
D78 §4 defines and `program/ground.rs:184` implements:

```rust
pub fn conjunction_entails(constraints: &BTreeSet<Iri>, sup: &Iri, layer: &Layer) -> Result<bool, String>
```

`fields(sup) ⊆ ⋃fields(constraints)`. So *"did this binding weaken?"* is not a new judgment D77 must
invent — it is `entails` evaluated across the two environments:

> `C` **weakened** between `Γ_side` and `Γ_merge` iff `C_side ⊨ C_merge` and not `C_merge ⊨ C_side`.

Widening is the failing direction because a term quantified over `C` covers strictly more after the
merge than the proof constructed for `C_side` established.

### 2.2 D76 made the check a type-level one, which is what D20 asked for

D20 named the missing kind `InvalidatedSignature` **(type-checker driven)**. Before D76 that was not
buildable: `subtype_of` and `eq_nf` took no context, so nothing in conversion could resolve a class
IRI. After D76 Phase D they carry `Γ_env`, and `subtype_of_inner`'s `Refine` arm decides entailment
directly (`check/conv.rs:418-437`).

So the backstop D20 designed and could not build is now one environment away from expressible.

### 2.3 D78 changed what a rescan would run

D75 §7 flagged this and it held: validation is now clause-8 evaluation against `Val::Record`
(`validation/mod.rs`, `effective_record_fields`). A merge-scoped rescan invokes *those* rules, not the
transitive `subclass_of` walk that existed when D20 was written. Building the rescan before D78 would
have wired it to rules that no longer exist.

### 2.4 The resolution invariant — load-bearing, and stated nowhere in the code

`Layer` carries `parents: Vec<Arc<Layer>>` and merges with three parents exist
(`lattice.rs:2475`), but `resolve_uncached` advances via `parents.first()`
(`layer/mod.rs:748`) — resolution walks **one** parent. `collect_ancestors`
(`layer/index.rs:700`) walks **all** of them.

That is not a disagreement, and reading it as one was a mistake worth recording, because the
resolution of it is what makes this document's central question well-posed. The two answer different
questions — the index needs the ancestor *set* for triple-index coverage; resolution needs the
*binding* — and **the binding is the same down either path**:

> `commit_resolutions_as_merge_layer` materialises `sources_a`, `sources_b` and every resolved
> conflict into the merge layer itself (`merge/resolve.rs:929-966`). So anything either branch changed
> since the LCA is found *in the merge layer*, before any parent walk begins; and anything neither
> changed is unchanged since the LCA, which is reachable down either parent and identical either way.

**So `Γ_merge` is well-defined**, `binding_M(i)` in §3 means something, and `revalidate_pending`'s
reuse below is sound — it resolves through `new_layer.resolve()`, which is exactly this walk.

**The distinction the invariant does *not* cross is this document's subject.** It says which body `i`
resolves to. It says nothing about whether a resource that mentions `i` is still *valid* given what
`i` now binds to. #225 is precisely a resource **neither branch changed** becoming unsound because the
other side rebound something under it — a merge that is correct by the invariant and wrong by the
judgment.

Worth writing down because nothing in the code states it: the correctness of first-parent resolution
rests on the materialisation above, and a future change that made the merge layer hold *references*
rather than bodies would break it silently.

### 2.5 The linear analogue is the template — and is complete only for shape dependents

`retroactive_validate` (`validation/retroactive.rs:91`) discharges the same obligation for a **linear**
commit:

```rust
pub fn retroactive_validate(new_layer: &Arc<Layer>, ws: &mut CommitWorkingSet)
    -> Result<(), WorkingSetExhausted>
{
    enumerate_dependents(new_layer, ws)?;
    revalidate_pending(new_layer, ws)
}
```

Two halves — *enumerate what the change could have broken*, then *revalidate it* — and D77 needs the
same two halves with a different enumeration. The second half is reusable as-is.

**It is complete for *shape* dependents only.** The linear path has the same term-dependency hole
§3.2 identifies for merge, and D75 §3.4 witnessed it: witness credit earned against one binding of a
class survives that class being widened. [D80](d80-witness-and-institution-machinery.md) §2 fixes it.

**Its scoping lesson is the one to inherit.** That scan produced an OOM on a 7.6M chain and was fixed
by gating to *redefinitions* (`redefines_ancestor`). D75 §7 names this as the trap D77 most risks
repeating.

---

## 3. What `InvalidatedSignature` computes

**The rebound set, its dependents, then rechecking.** Stated as the pushout obligation: for a merge
`M` of branches `A` and `B` over LCA `L`, a resource `R` contributed by side `S` is at risk iff some
IRI `i` that `R` depends on satisfies

```
binding_M(i)  ≠  binding_S(i)
```

— `i` resolves differently in the merged chain than in the chain `R` was checked against.

### 3.1 The rebound set

Computable from the existing `MergeSpan`. `sources_a` and `sources_b` record which IRIs each side
contributed and from which layer, so

```
rebound_B = { i ∈ sources_a : binding_M(i) ≠ binding_L(i) }
```

and symmetrically. **Note what this is not:** `shared_iris` is `sources_a ∩ sources_b`, and `rebound_B`
is drawn from `sources_a` **alone** — the IRIs the *other* side changed. An intersection asks "did both
sides touch this?"; the pushout asks "did the other side move something under me?" That asymmetry is
the whole defect, and no patch to an intersection expresses it.

**Weakening is decidable by D78's `conjunction_entails`** (§2.1): a rebinding matters when the new
binding admits strictly more than the old.

### 3.2 The dependency relation is *two* relations

Before D76 a layer was a store of resource **shapes**, and "`R` depends on `i`" meant `R`'s property
graph points at `i`. All three of `enumerate_dependents`' triggers walk exactly that, and the triple
index is built from it.

After D76 the chain binds **names to declarations**, so "`R` depends on `i`" *also* means **`R`'s term
mentions `i`** — a `ConstRef` inside an encoded proposition.

| dependency | how `R` reaches `i` | enumerable |
|---|---|---|
| **shape** | `is_a`, property value, property key | yes — the three triggers |
| **term** | a `ConstRef` inside an encoded term | **only after D79** |

**Term dependencies are invisible to every mechanism this scan uses**, because an encoded term
contributes no triples at all. That is not a merge problem and is not fixed here:
[D79](d79-the-representation-of-inductive-types.md) is the correction, and after it the enumeration
below is `scan_predicate_object(core:mentions, i)` — one index range, not a new full-chain walker.
**This document is blocked on D79 §2.2 for exactly that reason.**

### 3.3 Rechecking means two different things

Symmetrically:

- **shape dependents** — `Validator::validate_resource`, which is `revalidate_pending` unchanged;
- **term dependents** — **re-type-check the term in `Γ_merge`**, which is `check` against the merged
  environment, not a validation rule.

The second is only expressible because D76 put the environment in the judgment, and it is why D20
called the kind `InvalidatedSignature` **(type-checker driven)** rather than validator-driven. It is
not a scoping variant of the rule-driven scan — **it is a different pass over a different dependency
relation**, and an earlier draft of this section had it as the former.

**Two, not three.** Witness credit and institution verdicts are also facts that a rebinding can
invalidate, and neither is repaired by revalidation or re-type-checking. They are not listed here
because they are not merge-specific: both fire on a linear commit, and D80 fixes them there. This
pass calls what D80 defines rather than defining a third recheck of its own.

### 3.4 Why not simply revalidate the whole merge layer

The obvious answer, and wrong for the reason Rule 22's scan was gated: a merge layer is the union of
two branches, which over a lexicon chain is millions of resources. The scan must be proportional to
*what changed*. `rebound` is typically a handful of IRIs.

### 3.5 Cost, corrected

An earlier draft called trigger 2 — the `O(chain)` carrier scan — the cost driver. Two corrections:

- **it is narrower than it looks.** Gated on `is_property && redefines`, so it fires only when the
  rebound IRI is a redefined `core:Property` *declaration*. #225's rebound IRI is a **class**, which
  takes the indexed trigger 1.
- **and `predicate`-alone is a prefix range, not a missing capability.** `MemoryTripleIndex` keys
  `pos: BTreeSet<Vec<u8>>` on `pos_key(p, o, s, layer)` (`index.rs:360-374`), so ranging on the `p`
  prefix would answer "which subjects carry `P`" in `O(carriers)`. The trait exposes only
  `scan_predicate_object(p, o)`; the *method* is absent, not the capability. `retroactive.rs:309`
  presents this as an index limitation alongside the genuine one, and only the genuine one stands:
  a **literal-valued** property has no triples to range over, because `is_indexable_predicate` indexes
  only `resource` / `resource_array`.

Neither changes this document's scope. Adding `scan_predicate` improves the linear commit path
identically and belongs there, not here — it is a missing *range method* over triples that already
exist, unrelated to §4, which is about triples that are never emitted.

### 3.6 A rename resolution does not rewrite term references — and cannot be fixed before D79

Found while landing D79 P3. Two functions in `layer/merge/resolve.rs` decide and perform an
IRI rename, and both stop at `Value::Json`:

| function | line | `Value::Json` |
|---|---|---|
| `value_mentions_iri` — *does this resource reference the IRI?* | `:448` | `_ => false` |
| `substitute_iri_in_value` — *rewrite every reference* | `:~478` | `other => other.clone()` |

So a resource whose only reference to a renamed IRI is inside an encoded term is **not selected**
for rewriting, and would not be rewritten if it were. The merge commits reporting a completed rename
while the term still names the old IRI. That is worse than the missed invalidation §3 is about — it
is a dangling reference written into the chain, not a stale check.

**The fix is gated on D79 §2.1, and would have been unsafe before it.** The correct rule is *descend
into `Value::Json` only when the carrying property is declared `core:inductive`* — a `core:json`
value is an opaque payload (a solver result, a `*_kv` map) whose IRI-shaped strings are **data**, and
rewriting one would corrupt it. Before D79 P2 the carrier's declared data type could not make that
distinction: twenty-two term-valued properties were declared `core:resource` and `core:ctor_type` was
`core:json`. A term-aware rename written then would have had to choose between missing terms and
corrupting blobs.

`json_mentions` (D79 §2.2) answers the first column. The second needs a *rewriting* walker, which
D79 does not build — detection and substitution are different traversals. F1's audit should size it.

## 4. Does merge gain a validation pass, or *is* the pushout obligation the pass?

D75 §7 left this open. **The answer is that it gains one, and the pushout obligation is its
enumeration.**

The distinction matters because it decides where the code goes. Failure 2 above is that
`commit_resolutions_as_merge_layer` ends at `builder.build(storage)` → `backend.store_layer(&layer)`
— the `store_layer`-only adapter, described at `commit/backend_persister.rs:26` as serving *"callers
that don't have branch semantics"*. A merge manifestly has branch semantics; it is using the adapter
for callers that do not.

So there are two candidate shapes:

| shape | what it means |
|---|---|
| **(a)** merge routes through the commit pipeline | it gains `Validator::validate` **and** `retroactive_validate` for free, with `CommitPolicy` deciding reject-vs-cascade |
| **(b)** merge keeps its own path and adds a pushout pass | one new pass, but the merge path stays a second implementation of committing |

**(a) is the structural answer and (b) is the guard.** D75 §3.5's finding is precisely that *the
checking path and the resolution path are different paths*; keeping them different and bolting a pass
onto one of them preserves the defect's cause while removing this instance of it. The project posture
names that shape directly.

**But (a) needs a decision this document cannot make alone.** Not for the reason first written here —
that *"the commit pipeline is built around a single parent, and a merge layer has two"* — because
`Layer` already carries `parents: Vec<Arc<Layer>>` and §2.4's invariant makes resolution through such
a layer well-defined. The data model is not the obstacle.

What is unestablished is narrower: whether `CommitPipeline`'s **phases** hold assumptions a merge
breaks — `already_validated`'s anchored-commit cache, the branch-advance gating, and whether
`retroactive_validate`'s enumeration means the same thing when "the new layer" is a union of two
branches rather than a delta over one. `commit/pipeline.rs` and `commit/phases.rs` need reading
against *that*. **That is the first work item, and it is research, not code.**

---

## 5. Phases

Each phase begins with a code audit, per the discipline D76's phases established — every one of its
seven audits corrected something the design had asserted.

- **F0 — can a merge route through the commit pipeline?** Read `commit/pipeline.rs`,
  `commit/phases.rs` and `layer/handle.rs` against the two-parent question. **Include
  `dispatch_auto_on_load_for_layer` (`phases.rs:411`) in the audit** — D80 §3.3 row 1 shows merge
  currently skips AutoOnLoad entirely, which is a live hole (a) closes and (b) does not. Output is a
  decision between §4's (a) and (b), with the cost of each measured rather than estimated. **No code.**
- **F1 — the rebound set**, and audit §3.6's rename path while in this file. `rebound_A` /
  `rebound_B` `rebound_A` / `rebound_B` from `MergeSpan` + the LCA, with
  `conjunction_entails` deciding weakening. Pure function, unit-testable against the #225 fixture.
  Gate: `a_reference_meeting_a_redefinition_raises_no_conflict` **flips** — renamed, since after this
  it raises one.
- **F2 — enumeration, both relations.** Shape dependents reuse `enumerate_dependents`' three triggers
  scoped to the opposite side; term dependents are `scan_predicate_object(core:mentions, i)`, which
  **requires D79 §2.2**. Gate: a resource whose *only* link to the rebound IRI is inside an encoded
  proposition is enumerated — the case the existing triggers provably miss, and the case #225 is made
  of. Plus: the scan is proportional to `|rebound|`, asserted against a large chain with a one-IRI
  rebound.
- **F3 — the pass, both recheckers.** Shape dependents through `revalidate_pending`; term dependents
  through `check` in `Γ_merge` (§3.3). Wire per §4's decision. Gate: the #225 scenario is **refused**,
  and a merge whose bindings did not weaken still succeeds.
- **F4 — `InvalidatedSignature` fires.** The cascade variant carries the finding, so the resolution UI
  can surface it. Gate: the variant is constructed somewhere other than a test.

**Not gated on a reseed.** Nothing here changes the chain format or any ontology — D79 P2 carries the
only bootstrap edit in the three documents. F0 and F1 depend on neither predecessor and can start
immediately; F2 onward wait on D79 P3 and D80 W0.

---

## 6. What this does not cover

- **Witness credit and institution verdicts.** Both are facts earned under a binding that survive it
  changing, and both fire on an ordinary *linear* commit, so neither is a merge defect.
  [D80](d80-witness-and-institution-machinery.md) fixes them there; this pass calls the recheckers it
  defines.
- **`InvalidatedTrace`.** The other reserved cascade variant. D80 §3 populates it; the merge-side
  surfacing follows once it exists.
- **The asymmetric tombstone case.** `DeletionConflict` is documented as never raised: tombstones are
  honoured on lookup but written only by D20 §6.2/§6.3 merge resolutions (`layer/handle.rs:156`), so
  the case needs a branch that is itself a merge. D75 §3.5 records it as uncovered by any conflict
  kind; it stays uncovered here.
- **Sealing more than inductives.** D79 §2.3 seals `InductiveType` declarations; classes and
  properties stay redefinable, because redefining a class to add parents is a load-bearing modeling
  idiom. That is deliberate — a redefinable class is exactly what §3's pass exists to check.
- **Narrowing.** A binding that *strengthens* leaves prior proofs sound, so it is not a soundness
  defect. It may still be a *usability* one — a resource that no longer satisfies a tightened class —
  but that is Rule 22's existing linear behaviour and needs no merge-specific answer.

## 7. References

- #225 (the defect, with the witness), #215 (P2 tracker)
- D75 §3.5 (the witnessed hazard), §5 (merge as a pushout), §7 (sequencing and the rework argument)
- D20 §7.2 (merge span), §8 (cascade analysis and the four kinds)
- D76 §2.4 (the inbound obligation), Phase D (`Γ_env` in conversion)
- D78 §3 (the `Refine` subtyping rule), §4 (entailment)
- D79 (`core:mentions`, which F2 consumes), D80 (the recheckers F3 calls)
- `layer/merge/conflict.rs`, `layer/merge/resolve.rs`, `layer/merge/cascade.rs`,
  `validation/retroactive.rs`
- `commit/pipeline.rs`, `commit/phases.rs`, `commit/backend_persister.rs` (F0's reading list)
