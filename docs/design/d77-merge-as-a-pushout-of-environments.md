# D77 — Merge as a pushout of environments

**Status: design.** No code. Closes [#225](https://github.com/eigenius/eigenius/issues/225), a
soundness defect reachable on a live RPC.

Implements the third of D75's three follow-ons, after
[D78](d78-resources-as-records.md) (Seam B) and [D76](d76-the-typing-environment.md) (Seam A), both
complete. D75 §7 ordered it last *"deliberately — both predecessors change what it checks and what it
is able to check"*. §1 below records what they changed, because it is more than that sentence
anticipated.

**Scope note.** The document runs wider than its title suggests. Making merge check the pushout
obligation requires enumerating what depends on a rebinding, and after D76 half of that dependency
relation — what a *term* mentions — is not represented anywhere the chain can query. §4 fixes the
representation: it normalises the three declarations now carrying D47-encoded terms onto
`core:inductive`, adds the indexer arm that projects term references, and seals the constructor
vocabulary so that projection stays affordable. One of those is a bootstrap edit and needs a reseed.
That work is here rather than in its own note because it is not separable from the argument: it is
the answer to "why can't merge see this dependency", and stating the reason in one document and the
decision in another would leave neither complete.

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
class survives that class being widened. §4.4 states the relationship.

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

### 3.2 The dependency relation is *two* relations, and only one is enumerable today

*(Two for the purposes of §3. §4.5 adds a third — **support** — which behaves differently enough that
it is stated after the machinery it depends on.)*

This is what changes when the chain becomes `Γ_env`, and it is the correction that reshaped this
section.

Before D76 a layer was a store of resource **shapes**, and "`R` depends on `i`" meant `R`'s property
graph points at `i` — an `is_a` target, a property value, a property key. All three of
`enumerate_dependents`' triggers walk exactly that, and the triple index is built from it
(`extract_indexable_triples`, `layer/index.rs:294`).

After D76 the chain binds **names to declarations**, and a resource carrying a proposition was
type-checked in that environment. So "`R` depends on `i`" *also* means **`R`'s term mentions `i`** — a
`ConstRef` inside an encoded proposition, an inductive named in a `ctor_type`, an axiom cited in a
justification.

| dependency | how `R` reaches `i` | enumerable today |
|---|---|---|
| **shape** | `is_a`, property value, property key | yes — the three triggers |
| **term** | a `ConstRef` inside an encoded proposition | **no** |

**Term dependencies are invisible to every mechanism the linear scan uses.** They live inside
`Value::Json`, and `extract_indexable_triples` emits triples only for `Value::ResourceRef` under
`resource` / `resource_array` predicates (`index.rs:306-340`) — an encoded term contributes **no
triples at all**. Trigger 3 ("referenced as an IRI value") therefore cannot see them.

**The tree already knows this distinction, and has been bitten by it.** D76 Phase A needed its own
reference walker for `declaration_order`, documented at `layer/declaration_order.rs:113`:

> *"Descends into `Value::Json`, which the walker in `layer::supporting` does not. That one documents
> JSON as never carrying typed-reference semantics, which is true for its purpose and false here: an
> inductive's constructor argument types are stored as D47-encoded JSON, so a walker that skips `Json`
> finds **no inductive-to-inductive edges at all**. Reusing it would produce an empty graph for
> precisely the case `OrderError::MutualInductives` exists to catch, **and would look like it
> worked**."*

The same sentence applies here with "merge hazard" substituted for "mutual inductives" — a second
consumer forced to hand-roll the same descent. **That is the signal to fix the representation rather
than write a third walker**, which §4 does: after it, term dependents are an index range, not a scan.

### 3.3 Rechecking means two different things

Symmetrically:

- **shape dependents** — `Validator::validate_resource`, which is `revalidate_pending` unchanged;
- **term dependents** — **re-type-check the term in `Γ_merge`**, which is `check` against the merged
  environment, not a validation rule.

The second is only expressible because D76 put the environment in the judgment, and it is why D20
called the kind `InvalidatedSignature` **(type-checker driven)** rather than validator-driven. It is
not a scoping variant of the rule-driven scan — **it is a different pass over a different dependency
relation**, and an earlier draft of this section had it as the former.

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

## 4. Making term dependencies enumerable

§3.2 established that term dependencies are invisible. This section is why, and what to change. The
short version: the dedicated data type already exists, two of its three declarations are wrong, and
sealing the constructor vocabulary makes indexing the rest affordable.

### 4.1 The data type exists; three declarations are in use for it

`core:inductive` is a declared `core:DataType` whose `class_types` must name exactly one
`InductiveType` (D32 §3.5). It is not opaque to the kernel: Rule 16 walks the tagged-dict tree against
the type's ctors and `arg_types`, and Rule 21 (`check_type_expr_well_typed`) decodes and
**type-checks** every `eigentt:TypeExpr`-ranged value, rejecting unresolved `ConstRef`s
(`validation/rules/inductive.rs:171`, `validation/rules/eigentt_value.rs`).

Three different declarations carry D47-encoded terms today:

| declaration | properties | validated | indexed |
|---|---|---|---|
| `core:inductive` + `class_types eigentt:TypeExpr` | `eigentt:axiom_statement`, `definition_type`, `definition_body` | Rules 16 + 21 | no — and says so |
| `core:resource` + `class_types eigentt:TypeExpr` | `lexicon:cat`, `lexicon:sem_type`, `reflection:canonical_proposition`, `core:type_name` | Rule 21 | **claims to be, isn't** |
| `core:json` | `core:ctor_type` | **nothing** | no |

Row 1 is honest — `core:inductive`'s own description says *"the wire shape is opaque JSON to
surrounding chain queries."* That sentence was a decision, and this document is the case against it.

**Row 2 is the defect.** Declaring a term-valued property `core:resource` makes both reference
consumers accept it and then do nothing:

- `extract_indexable_triples`' `wk::RESOURCE` arm matches `Value::String | Value::ResourceRef` and
  drops `Value::Json` into `_ => {}` (`index.rs:321-328`) — **zero triples**;
- Rule 22(b)'s filter `dt != RESOURCE && dt != RESOURCE_ARRAY` **passes**, then `iris_of` calls
  `as_iri()`, which returns `None` for `Value::Json` (`resource.rs:140`) — **zero checks**.

Neither errors. `lexicon:cat` and `lexicon:sem_type` are declared reference-typed on every lexicon
entry and contribute nothing to either mechanism. Nothing is *unchecked* — Rule 21 covers those slots
— but not by the mechanism the declaration names, which is the same "looks like it worked" shape as
the `declaration_order` walker.

Row 3 is what forced that walker: `ctor_type` is `core:json`, so no rule validates it.

**Decision: normalise rows 2 and 3 onto row 1.** `lexicon:cat`, `lexicon:sem_type`,
`reflection:canonical_proposition`, `core:type_name`, `core:ctor_type` become `core:inductive` +
`class_types eigentt:TypeExpr`. This is a bootstrap-ontology edit and therefore requires a reseed.

### 4.2 The indexer arm

**No new `Value` variant is needed.** Indexability is decided by the *predicate's declared data type*,
not the value's variant — `extract_indexable_triples` already resolves `prop_def.data_type` per
predicate. `core:json` and `core:inductive` share `Value::Json` on the wire and the predicate
distinguishes them.

So: a `wk::INDUCTIVE` arm that harvests the term's `ConstRef` IRIs, deduplicated per subject, emitted
under a synthetic **`core:mentions`** predicate rather than the carrying property. The question the
merge pass asks is *"which resources mention declaration `i`"*, not *"which mention it in slot
`cat`"*, so one predicate answers it in a single `scan_predicate_object(core:mentions, i)` range.

Consequences: §3.2's term-dependent enumeration becomes an index lookup rather than a chain walk, and
`declaration_order`'s bespoke walker can be deleted in favour of the same extraction.

### 4.3 Sealing the constructor vocabulary

Indexing every mention naively is expensive at lexicon scale, and expensive in a specific,
diagnosable way. A lexicon entry's `cat` is `cat_np(wn:n00001740, num_sg)`; multiplied over a
7.6M-entry chain, the posting list for `lexicon:cat_np` has millions of members. It is not *wrong* —
rebinding `cat_np` genuinely would invalidate every entry — but nothing can rebind `cat_np`, and
nothing should be able to.

**The rule: an `InductiveType` or one of its constructors may not be redefined.** D76 made the reason
literal — the chain binds names to declarations and `Env::lookup` returns *one* `InductiveDecl`. For a
class, "add a parent" is a monotone edit that the alignment layers depend on. For an inductive there
is no monotone edit: changing constructors changes the type, and every committed term mentioning it
silently means something else.

Two static scans over the repo's ontologies, experiments and demo files (approximate — ESL
constructor extraction undercounts bare-enum arms):

| candidate rule | scope | files that violate it today |
|---|---|---|
| seal **all** bootstrap IRIs (~1211) | 21 compiled-in ontologies | **6** |
| seal **inductive types + constructors** (193 repo-wide) | the term vocabulary | **0** |

The blanket rule is too broad, and the six say why. Five are parsing probes redefining closed-class
*entries* (`resource lexicon:among_prep : lexicon:LexicalEntry`) — instances. The sixth is
`ontologies/encoding/claim-kind-alignment.esl`, which is chain-loaded in the demo and redeclares
`enc:Finding` / `Observation` / `Classification` with lexicon parents added; its header names the
idiom as *"the layered-resolution pattern the wordnet↔umls alignment established."* Sealing all of
bootstrap breaks that. Sealing the constructor vocabulary costs nothing — zero violations across the
whole tree — which is the right time to impose a rule.

**What the seal buys the index.** Mentions whose object is sealed are dropped: they can never enter a
`rebound` set, so their posting lists can never be queried. In `cat_np(wn:n00001740, num_sg)` that
removes two of three. `wn:n00001740` survives — and must, because the alignment layer redefines
WordNet sense classes, which is exactly the live merge hazard. Per-entry mentions fall from ~5 to
~1-2, and every retained edge is one a merge can actually break.

**What the seal does not do.** An earlier draft cited `layer/witness_index.rs:1133` — where witness
credit survives redefinition of a class a proposition quantifies over — as evidence that the seal has
standing beyond merge, and said sealing "removes it for the constructor vocabulary." That reads as if
the seal addresses the cited case. It does not: the rebound name in that test is `Dog`, a **class**,
and classes stay redefinable by design (§7). The seal covers only the sub-case where the rebound name
is a constructor, which is not what that test exhibits.

The witness index is a real defect and a closely related one, but it is related as a *sibling*, not
as something the seal fixes. §4.4 states the relationship.

**It is not a substitute for the pass.** #225's rebound IRI is a domain class, and domain classes are
unsealed by construction — that is where modeling happens. The seal shrinks the index and removes the
largest posting lists; it does not remove the need for §3's rebound-set pass.

### 4.4 The witness index is the linear-chain instance of this document's defect

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

That strengthens the case for §4.2 rather than complicating it. Once `core:mentions` exists, *"which
witnessed propositions mention `Dog`"* is the same range query as *"which resources mention `i`"*, and
the fix has the same two halves — enumerate, then invalidate credit — discharged at commit through
`retroactive_validate` rather than at `lookup_chain_witness`, which is a hot path and the wrong place
to put a recheck.

**Scope.** §4.1-4.3 are prerequisites for §3 and are in this document because §3 cannot be built
without them. Wiring the witness invalidation is not: it is a second consumer of machinery §4 builds,
on a different trigger, and it needs its own decision about what "revoking credit" means for
already-committed `JustifiedBy` results. It is F5 in §6, gated behind F2c, and it is called out here
so that the tool is not built and left unwired.

### 4.5 Support is a third relation, and re-execution is sometimes the right recheck

§3.2 split the dependency relation in two. There is a third, and it is the one an institution verdict
lives on. A `reflection:InstitutionEmittedDerivation` — the statistics institution emits one per ANOVA
effect — is a verdict computed from a gated analysis spec and its data. Rebind the bound dataset or an
experimental-design parameter and the verdict no longer follows, **while every resource on the path
stays structurally valid and type-correct**. Nothing is invalid. The verdict is *unsupported*.

| relation | reaches `i` via | closure | recheck |
|---|---|---|---|
| **shape** | `is_a`, property value, property key | one hop — validity is local | `validate_resource` |
| **term** | `ConstRef` in an encoded term | one hop — the term is checked in `Γ` | re-type-check in `Γ_merge` |
| **support** | `from_subject`, `runtime_invocation`, `runtime:inputs`, `derivation_trace` | **transitive** | **§4.5.2** |

**It is transitive, and the other two are not.** One hop suffices for shape and term because validity
and type-correctness are *local*: revalidating `R` against the merged chain settles `R`, and `R`'s own
dependents are unaffected because `R` did not change. Support does not work that way — a rebound
dataset invalidates a derivation through `invocation → inputs`, and that derivation may itself be an
input to another. `enumerate_dependents` is a single pass over the new layer's `defined_iris()`
(`retroactive.rs:91`) with no fixpoint, so it cannot reach past one hop by construction.

#### 4.5.1 The staleness question is decidable from the index

`runtime:inputs` is `core:resource_array` ("ordered list of input resource IRIs"); `from_subject`,
`runtime_invocation`, `runtime:script` and `environment` are `core:resource`; the invocation pins
`image_digest`, and D53 §6.1 file-backed observations carry a `content_hash` the kernel verifies. Every
provenance edge is therefore an indexed triple under the existing rule — no §4.2 extension is needed
for this relation, only a transitive closure over a **named edge set** rather than over all reference
edges, which would reach the whole chain.

So *"was this verdict computed from something the rebinding moved"* is answerable by closure over the
index, for every institution, without running anything.

#### 4.5.2 Whether it can be *rechecked* is declared, not assumed

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

#### 4.5.3 Three cells, and merge currently serves none of them

Crossing "can it re-execute in-process" with §2.4's materialisation invariant — the merge layer holds
what *either branch changed*, and nothing else:

| verdict | carrier in the merge layer | disposition |
|---|---|---|
| in-process (no `runtime_invocation`) | **yes** — a branch changed the spec | re-fires under §5(a) **for free**; skipped entirely under (b) |
| in-process | **no** — spec unchanged, its *data* was rebound | F6 enumerates it, then **re-dispatches** AutoOnLoad for that subject |
| external-runtime | either | **mark** — `InvalidatedTrace`; re-execution is out of scope |

Row 2 is the case in the question that prompted this section, and it is #225's shape exactly: the
at-risk carrier is one *neither branch changed*, so it is not in the merge layer and no amount of
pipeline routing reaches it. It needs the enumeration.

**Row 1 is a present defect, and it sharpens §5.** Merge today ends at `store_layer` (§5, failure 2),
so `dispatch_auto_on_load_for_layer` never runs for a merge — an analysis spec contributed by a branch
produces no verdict at all, and one whose inputs moved keeps the old one. §5 argues for (a) on the
structural ground that the checking path and the resolution path should not be two paths; this is the
same argument with a witness attached, and it is not hypothetical.

Row 3 is where the original "mark, don't recompute" answer stands, and it stands for the right reason:
not that the kernel lacks authority, but that a merge commit cannot be unbounded in time or depend on
a foreign runtime. That is `CascadeItem::InvalidatedTrace { trace, reason }`, which D20 §8 reserved for
exactly this — *"A trace references content that becomes inconsistent."*

**What is out of scope**, stated so it is not mistaken for an oversight: dispatching *external-runtime*
institutions, scheduling their re-execution, and any notion of a verdict's numerical stability. D77
re-dispatches only what the commit pipeline already re-dispatches on an ordinary load.

## 5. Does merge gain a validation pass, or *is* the pushout obligation the pass?

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

## 6. Phases

Each phase begins with a code audit, per the discipline D76's phases established — every one of its
seven audits corrected something the design had asserted.

- **F0 — can a merge route through the commit pipeline?** Read `commit/pipeline.rs`,
  `commit/phases.rs` and `layer/handle.rs` against the two-parent question. **Include
  `dispatch_auto_on_load_for_layer` (`phases.rs:411`) in the audit** — §4.5.3 row 1 shows merge
  currently skips AutoOnLoad entirely, which is a live hole (a) closes and (b) does not. Output is a
  decision between §5's (a) and (b), with the cost of each measured rather than estimated. **No code.**
- **F1 — the rebound set.** `rebound_A` / `rebound_B` from `MergeSpan` + the LCA, with
  `conjunction_entails` deciding weakening. Pure function, unit-testable against the #225 fixture.
  Gate: `a_reference_meeting_a_redefinition_raises_no_conflict` **flips** — renamed, since after this
  it raises one.
- **F2a — the seal** (§4.3). A validation rule refusing a layer that redefines an `InductiveType` or
  constructor. Measured at zero violations, so this lands before anything depends on it. Gate: the
  rule fires on a hand-built violating layer; the full workspace suite, the demo and the parse gate
  are unperturbed. **No reseed** — it adds a rule, not an ontology edit.
- **F2b — the declarations** (§4.1). `lexicon:cat`, `lexicon:sem_type`,
  `reflection:canonical_proposition`, `core:type_name`, `core:ctor_type` → `core:inductive` +
  `class_types eigentt:TypeExpr`. **Bootstrap edit ⇒ reseed**; batch with any other pending bootstrap
  change. Gate: reseed completes at the current resource count with 0 errors, and `ctor_type` now
  reaches Rules 16/21 — asserted by a malformed `ctor_type` that previously loaded and now does not.
- **F2c — the indexer arm** (§4.2). `wk::INDUCTIVE` in `extract_indexable_triples`, emitting deduped
  `core:mentions` triples and skipping sealed objects. Gate: index growth on the lexicon chain is
  **measured, not estimated**, against the ~1-2-per-entry prediction; `declaration_order`'s bespoke
  walker is deleted and its `MutualInductives` tests still pass on the shared extraction.
- **F2d — enumeration, both relations.** Shape dependents reuse `enumerate_dependents`' three triggers
  scoped to the opposite side; term dependents are `scan_predicate_object(core:mentions, i)`. Gate: a
  resource whose *only* link to the rebound IRI is inside an encoded proposition is enumerated — the
  case the existing triggers provably miss, and the case #225 is made of. Plus: the scan is
  proportional to `|rebound|`, asserted against a large chain with a one-IRI rebound.
- **F3 — the pass, both recheckers.** Shape dependents through `revalidate_pending`; term dependents
  through `check` in `Γ_merge` (§3.3). Wire per §5's decision. Gate: the #225 scenario is **refused**,
  and a merge whose bindings did not weaken still succeeds.
- **F4 — `InvalidatedSignature` fires.** The cascade variant carries the finding, so the resolution UI
  can surface it. Gate: the variant is constructed somewhere other than a test.

- **F5 — witness credit under a widening redefinition** (§4.4), gated behind F2c. Enumerate witnessed
  propositions mentioning a rebound IRI via `core:mentions`; invalidate credit at commit rather than
  rechecking at `lookup_chain_witness`, which is a hot path. Needs its own decision on what revocation
  means for already-committed `JustifiedBy` results — that decision is the phase's first output.
  Gate: `witness_credit_survives_redefinition_of_a_class_the_proposition_quantifies_over`
  (`witness_index.rs:1184`) **flips** and is renamed, closing D75 §3.4.
  **`redefining_a_class_does_not_change_the_hash_of_a_proposition_over_it` (`:1133`) must NOT flip** —
  it guards proposition *identity*, which §7 keeps environment-blind. F5 revokes credit; it does not
  refork every existing witness key. An F5 that flips both has changed the wrong thing.

- **F6 — institution verdicts under a rebound input** (§4.5), gated behind F1 — the provenance edges
  are already indexed, so this needs the rebound set but not the term arm. Transitive closure over
  `from_subject` / `runtime_invocation` / `runtime:inputs` / `derivation_trace`, bounded to that named
  edge set. Then split on the verdict's `institution:runtime_invocation`: **absent** ⇒ re-dispatch the
  AutoOnLoad QueryClass for that subject against `Γ_merge`; **present** ⇒ emit `InvalidatedTrace`.
  Gates: rebinding a dataset two hops below an in-process `InstitutionEmittedDerivation` **recomputes**
  it, and a `Fails` verdict surfaces rather than being silently replaced; the same shape with an
  external-runtime verdict **marks** and dispatches nothing; the closure is bounded — a chain lacking
  the edge set enumerates nothing, asserted so it cannot degrade into a full reference walk.

**F2b is the one reseed.** Everything else leaves the chain format and the ontologies alone; F2b is a
bootstrap edit and should batch with any other pending one. F2a and F2c are ordered around it
deliberately — the seal first (so the indexer can rely on it), the declarations next (so the indexer
has a `core:inductive` predicate to match), the arm last. F1 does not depend on any of them and can
run in parallel.

---

## 7. What this does not cover

- **External-runtime institution dispatch.** F6 re-dispatches *in-process* verdicts (§4.5.2 — the
  distinction is read off `institution:runtime_invocation`, and `dispatch_auto_on_load_for_layer` is
  already a commit phase). Verdicts produced by an external runtime are **marked, not recomputed**:
  re-execution there is unbounded in time and depends on a foreign runtime. Scheduling it is a
  separate surface.
- **Proposition identity stays environment-blind.** F5 invalidates *credit* under a widening
  rebinding; it does not put the layer into `hash_proposition_exp`. Making the environment part of
  proposition identity is the alternative fix for D75 §3.4 and a much larger change — it forks every
  existing witness key. `witness_index.rs:1136` already names the assertion that would have to move.
- **The asymmetric tombstone case.** `DeletionConflict` is documented as never raised: tombstones are
  honoured on lookup but written only by D20 §6.2/§6.3 merge resolutions (`layer/handle.rs:156`), so
  the case needs a branch that is itself a merge. D75 §3.5 records it as uncovered by any conflict
  kind; it stays uncovered here.
- **Classes and properties stay redefinable.** §4.3 seals inductive types and constructors only.
  Redefining a class to add parents is a load-bearing modeling idiom (the wordnet↔umls alignment,
  `claim-kind-alignment.esl`) and is exactly the case §3's pass exists to check. Sealing more would
  trade a checkable hazard for a blocked workflow.
- **Narrowing.** A binding that *strengthens* leaves prior proofs sound, so it is not a soundness
  defect. It may still be a *usability* one — a resource that no longer satisfies a tightened class —
  but that is Rule 22's existing linear behaviour and needs no merge-specific answer.

## 8. References

- #225 (the defect, with the witness), #215 (P2 tracker)
- D75 §3.5 (the witnessed hazard), §5 (merge as a pushout), §7 (sequencing and the rework argument)
- D20 §7.2 (merge span), §8 (cascade analysis and the four kinds)
- D76 §2.4 (the inbound obligation), Phase D (`Γ_env` in conversion)
- D78 §3 (the `Refine` subtyping rule), §4 (entailment)
- D32 §3.5 (`core:inductive` and singleton `class_types`), D47 §5 (`ConstRef` resolution)
- `layer/merge/conflict.rs`, `layer/merge/resolve.rs`, `layer/merge/cascade.rs`,
  `validation/retroactive.rs`
- `layer/index.rs` (`extract_indexable_triples`), `layer/declaration_order.rs` (the second walker),
  `validation/rules/inductive.rs`, `validation/rules/eigentt_value.rs`,
  `layer/witness_index.rs:1133` (the standing unsoundness assertion)
