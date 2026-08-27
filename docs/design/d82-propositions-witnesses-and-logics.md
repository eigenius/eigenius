# D82 — Propositions, witnesses, and where logics live

**Status: design.** No code. Rearchitecture of the epistemic machinery, taking
[D81](d81-the-epistemic-stack.md) as its evidence base and the κ–τ pilot (arXiv:2608.08192) as its
first external forcing case.

**The thesis in one sentence.** The system was anchored on **resources** and is actually about
**propositions and how they came to be warranted**; every finding in D81 is a consequence of that
drift, and the fix is to make the proposition the subject, the witness the property of the logic
that produced it, and the kernel the owner of exactly one witness kind — the proof term it can
check.

---

## 1. The drift, and why it explains everything D81 found

D81 §0's central result is that the epistemic machinery is **two stacks that never touch**:

| stack | anchored on | read by |
|---|---|---|
| `reflection:*Resource` classes, `epistemic_status`, `Grade` | **a resource** — "this thing is derived" | nothing |
| `WitnessKey` = (category, IRI, **proposition hash**) | **a proposition** | the type checker |

The inert half is exactly the resource-anchored half. The working half was born proposition-keyed
and never needed the reframe. That is the drift, visible as a split.

Four D81 findings fall out of it directly:

- **Eight writers, one trait, no reader** (§1.6, §5.1). Nothing reads an epistemic class because the
  class describes the wrong subject. A grade is not a property of a resource; it is a property of
  *the relation between a proposition and its warrant*.
- **The unifying concept exists only as a function** (§1.3). `trace_category` names the set of trace
  kinds that ground a witness. There is no class for it because the concept it wants is *"what kind
  of warrant does this evidence supply"* — a statement about warrants, which a resource-anchored
  vocabulary has no place to put.
- **`Verified` has two producers and one works** (§2.4). The Lean route was specified as an inverse
  translation recovering a proposition *from a resource*; D49 §7 superseded it with
  externalize-and-check, which starts from **the claim's own proposition**. That supersession is the
  drift being corrected in one place, by hand.
- **The encoding ontology compensates in prose** (§6). It needed to say which of three propositions a
  trace witnesses and had nowhere to say it, so it said it in comments — and had the bug anyway
  until someone read them.

---

## 2. Principles

**P1 — The proposition is the subject.** The unit of epistemic accounting is a proposition together
with the warrant that licenses asserting it. Resources carry propositions; they are not themselves
the thing warranted.

**P2 — A witness is a generalized proof term.** In intuitionistic type theory the argument position
of a proof-carrying type holds a term. Justification logic generalizes that position: *declared*,
*observed*, *derived* and *verified* are four ways of discharging one argument, of which mechanical
checking is one. `witness:Is*As` are already `Prop`-valued inductives with **zero constructors**
sitting in exactly that position — the design is present; only its ownership is wrong.

**P3 — The four categories are kernel-core; a logic owns *refinements* of them.** An earlier draft
of this principle read *"a logic owns its witnesses"*, which would have let each institution mint its
own stratification. The evidence does not support that, and §2a sets it out: **every extension the
system has actually attempted is a refinement that projects onto the four**, and nothing anywhere
asks for a fifth.

So an institution declares a witness kind **together with the category it projects to**. The
projection is declared, checkable, and is precisely the *"which relation"* §3.5 says is missing — a
refinement **is** a named relation between a proposition and its evidence.

The kernel keeps the four; it does not keep the list of ways to reach them.

**P4 — The kernel owns exactly one witness kind.** Where the kernel can type-check a term against a
proposition in EigenTT, that *is* verification, and needs no institution. Everything else is a
logic's business.

**P5 — The generalized-witness *mechanism* is a kernel capability; justification logic is not.**
These are separable and D82 separates them, because conflating them is what put the four categories
in a kernel enum.

*The kernel cannot delegate the judgment that a term is well-typed.* It follows that it cannot
delegate the judgment that a witness position has been discharged — that judgment is part of the
type check. So **an argument position discharged by an oracle rather than by a term is a kernel
concept**, and must be.

What is *not* a kernel concept: which logics have witnesses, what those witnesses are called, what
discharges them, and how they stratify. The J-family, `JustifiedBy`'s connectives, and the four
categories *as a stratification* are justification logic — one logic among the possible ones, and
the reasoning institution's business.

**The kernel already has the mechanism, in degenerate form.** `CheckHooks::synthesize_chain_witness`
is generic in shape, but:

- it recognises a witness position by matching **four hard-coded short names**
  (`chain_witness_category_for_short_name`, `kernel/src/program/check_hooks.rs:34`) — so *any*
  inductive anywhere named `IsVerifiedAs` enters the path (D81 §5.5), a rule looser than the IRIs
  everything else uses;
- the categories it yields come from the kernel's own `WitnessCategory`;
- there is exactly one implementor.

Generalising it — recognition by **declared** witness type, categories owned by the declaring logic,
N implementors — is the kernel change this document needs. It is also the **only** one.

**P7 — A discharged witness is persisted.** In a system whose subject is propositions and their
warrants, *the inhabitant of the proposition is the evidence* — the trace is only the raw material it
was derived from. Storing the material and recomputing the conclusion on every read is what a
database does; a proof system keeps the proof.

**What was rejected was not this.** D66 slice 0 removed a materialised
`OnceLock<BTreeMap<WitnessKey, ()>>` holding *every witness the layer could admit*, because it "cost
memory proportional to the layer's trace count for the lifetime of the layer". That is an **eager
index of all possible witnesses**. P7 persists the ones actually **discharged** — one per witness
argument the checker filled — which is bounded by the number of certificates, not by the trace count.
Different objects, different cost.

**Persistence does not replace recomputation; it enables comparison.** This is what makes it worth
doing rather than merely tidy:

- today a witness is recomputed against the chain *as it now stands*, so the same key silently
  yields the same answer in a chain where the names it depends on were rebound — D80 §2's
  environment-blindness, undetectable because nothing recorded what was asserted;
- with the discharge stored, the asserted witness and the recomputed one can be **compared**, and a
  divergence is exactly the signal D77's rebound-set pass is trying to construct. Locally, per
  certificate, without a chain walk.

**And it makes the oracle auditable.** §2's P5 establishes that a witness is an *axiom the kernel
asserts* — `Val::ChainWitness(key)`, an inhabitant of a zero-constructor type introduced by fiat. The
system persists the trace, the verdict and the derivation, and keeps no record of the one step that
is postulated rather than proved. That is backwards. A stored witness is the assertion on the chain
with its stated reason, in the same form as everything else.

**Cost accepted:** widening `WitnessKey` later (§7 Q1) becomes a migration rather than a no-op. Under
the pre-production posture that is a reseed, and §5a.5's relation vocabulary should therefore land
*before* P7 rather than after.

**P6 — Not every warrant-producer is a logic.** An institution has a satisfaction relation.
Abductive selection does not, unless one is supplied. Both produce chain-resident warrant records;
only one is an institution.

### 2a. Do the four need to expand?

**No — on the evidence, and the refinement model is what the tree has been reaching for.**

| pressure point | what it did |
|---|---|
| `Warrant::{Declared, Parsed}` | two *different* relations — the source asserts it; the parser produced it from a span — **both projecting to `Declared`** via `Warrant::grade()` |
| `ExternalExecutionTrace` (eigenius#205) | a **fifth trace kind** added, mapped onto an **existing** grade. The refinement pattern already working, in a Rust `match` |
| the literature-warrant climb (`Warrant`'s own doc) | a `reference:Citation` keeps the grade at *"Declared-but-attested"* — a qualified `Declared`, not a new one |
| `objective:acceptance_grade` | reuses `reflection:EpistemicStatus` with `allows_only`, and says so: *"not a parallel enum"* |
| `lexicon:grade` | same |
| κ–τ (arXiv:2608.08192) | *"commit-worthy under Π at τ"* — a **qualified** warrant; and its suspension is a **non-assertion**, not a fifth grade |
| the encoding pipeline's three propositions | a **subject** problem (§3.5), not a category problem — it needs *which relation*, not *which fifth grade* |

Two negative checks:

- **Nobody minted a fifth individual.** `grep epistemic:` over the whole ontology corpus returns
  exactly four.
- **No defeat, retraction or contestation status exists** — the usual pressure point for a fifth in
  defeasible systems. The only `defeat`/`superseded` hits in the corpus are grammar prose and
  schema.org boilerplate.

D73 §1.2 already settled the matter from the other side, after the one attempt to make something
else canonical was withdrawn: *"the four epistemic resource classes … `DeclaredResource requires
declared_by` is a well-formedness rule about a resource and its trace, and **it stands unchanged**."*

**The refinement model is safe precisely because it does not assume completeness.** If a fifth
category is ever genuinely needed, the signal is a declared witness kind whose author cannot state a
projection — a declaration that fails rather than a stratification that quietly forks. The
architecture makes the need for expansion **detectable**, which is more than the current one does.

---

## 3. The target shape

### 3.1 Three tiers, distinguished by what discharges the witness

| tier | discharges a witness by | grade it can reach | examples |
|---|---|---|---|
| **kernel** | type-checking a term against the proposition | `Verified` | a `JustifiedBy` certificate; any EigenTT derivation |
| **institution** (has ⊨) | evaluating its own satisfaction relation | a **declared refinement** projecting onto one of the four | statistics, Lean, κ–τ |
| **selection producer** (no ⊨) | recording a constrained choice and its authority | `Declared`, with an auditable record | the encoding pipeline |

The middle tier is where the current protocol lives and the outer two are the ones it does not
model. Tier 1 is currently *dressed* as an institution — `reasoning:reasoning_institution` declares
`runtime = in_process` and states plainly *"the validator is the kernel"*. Tier 3 was correctly
refused institution status by D71 and given no protocol in exchange.

### 3.2 What the kernel keeps

- the proposition, and its identity (`hash_proposition_exp`);
- `Exp` / `Val` / conversion / the type checker;
- **one** witness rule: *a term that type-checks against P at `Prop` discharges a witness for P*;
- the commit pipeline, dispatch, and the tri-state verdict.

**The generalized-witness mechanism (P5) sits here**, not with the institutions: recognising that an
argument position expects a witness, and admitting an inhabitant obtained from outside the term
language. What the kernel must *not* own is which logic supplies it.

**This subsumes the reasoning institution's actual work.** D81 §2.4 route A is already the kernel
checking a certificate; the institution supplies vocabulary and a trigger. Under P4 that becomes
explicit: `JustifiedBy` checking is a kernel capability, and what remains institution-side is the
`JustificationTerm` *algebra* — the connectives (`app`, `sum_l`, `sum_r`, `spec_poly`) and whatever
future J-family axioms are wanted.

### 3.3 What an institution gains

Two things it does not have today.

**(a) A declared witness kind, and the right to discharge it.** `CheckHooks` is already
`Arc<dyn CheckHooks>` with `synthesize_chain_witness` on it — **and exactly one implementor**,
hard-wired at construction (D81 §5.3). The seam for institution-supplied witness synthesis exists
and is unused. What is needed:

- an institution declares its witness inductive(s) in its own ontology, as reasoning already does
  with `witness:Is*As`;
- it registers a synthesis handler for them;
- the checker dispatches on the *declared* witness type rather than on two hard-coded class IRIs and
  a Rust `match`.

This replaces D81 §5.2's three kernel-only lists with declared relations. The vocabulary for the
last of them already exists and is dead: `reflection:epistemic_status` carries `allows_only` over
exactly the four grade individuals, is attached to `ProgramTrace`'s `recommends`, and **no Rust file
reads it**.

**(b) A declared post-condition on success.** Today `Fails` has defined consequences and `Holds` has
none, so what a passing gate may assert is each handler's private decision — reasoning mints a
trace, Lean mints nothing, and nothing detects the difference (D81 §2.5, §3.1.3). D14 §7.2 and D52
§6 both assigned promotion to *the kernel* and it was never built (D81 §4.3).

A `QueryClass` should declare what a `Holds` entitles: nothing, a named witness kind, or a grade.
The kernel enforces the declaration rather than trusting the handler.

### 3.4 What a selection producer gets

A protocol that is **not** the institution protocol: a record of *candidates, choice, authority,
rationale, residue*, warranting **that a choice was made under constraint and by whom** — never that
the choice is right. `enc:DecisionPoint` / `AnaphorBinding` / `LexicalGap` / `CutItem` are that
record, hand-rolled. Generalizing them is cheap and makes D71's refusal principled rather than
merely correct.

### 3.5 The subject problem, which is the hard one

`WitnessKey` is `(category, IRI, proposition hash)`. It can say *this resource is Derived with
respect to that proposition*. It cannot say **with respect to which of several propositions about
it** — and the encoding ontology needed exactly that:

> *1. this text parses to this well-typed term — the artifact fact
> 2. this encoding is faithful to what the author wrote — unwarranted
> 3. what the author wrote is true — only ever declared*

It got this wrong in production until 2026-08-22, minting `IsDerivedAs(claim, P)` with P a
proposition about the world when the run established something about the artifact. **The witnesses
were well-formed; nothing detected it.** The κ–τ pilot walks into the same shape — a
`SuspendedDerivation` warrants *commitment-status under Π at τ*, not the hypothesis.

So a witness must carry **the relation it witnesses**, not only the proposition. This is the one
genuinely new piece of vocabulary D82 proposes, and the one that cannot be deferred: every other
change is a relocation of something that exists.

---

## 4. κ–τ as the forcing case

The pilot is a good test because it needs (a) and (b) and needs the kernel unchanged.

| what it needs | status |
|---|---|
| its own satisfaction relation | **has one** — `S ⊩ C_τφ ⟺ sc_S(φ) ≥ τ`; this is the paper's contribution, and what our own abductive pipeline lacks |
| a witness that pins parameters | needs (a). A verdict is meaningless without `w`, `κ`, `λ`, `τ`, `ε`, `δ` — the same discipline as `image_digest` and `random_seed` |
| to record suspension | **already possible**: `Undecidable` commits the institution's resources and does not reject the subject; only `Fails` drops them |
| honest provenance for neural-estimated `κ` | already right in the proposal — each estimate committed as a resource with its own grade |
| to promote nothing | correct under today's protocol, and the reason it needs no kernel change |

It also demonstrates P6 from the other side: **abduction becomes an institution exactly when given a
satisfaction relation.** Our encoding pipeline has no ⊨ and is correctly not an institution; κ–τ
supplies one by replacing *best explanation* with *score over threshold*, and qualifies.

---

## 5. What changes, what does not

**Does not change:** the four grades (composition, not replacement); `Exp`/`Val`/conversion;
the tri-state verdict; the commit pipeline's shape; any chain data.

**One kernel change, and it is the only one** (P5): the checker's *recognition* of a witness
position generalises from four hard-coded short names to a declared witness type. The **dispatch
stays kernel-side** — it is part of the type check and cannot leave. Only the *handlers* relocate.
Everything else below is a relocation of something that already exists.

**Relocations** — each moves something that already exists:

| from | to |
|---|---|
| `trace_category`, the self-attesting arms (Rust) | declared relations, using `epistemic_status`'s existing vocabulary |
| witness synthesis *handlers* (kernel-only, one impl) | per-institution, behind the existing `CheckHooks` seam |
| promotion (handler convention) | a declared `QueryClass` post-condition the kernel enforces |
| the reasoning institution's checking role | acknowledged as kernel capability; the institution keeps the `JustificationTerm` algebra |

**Genuinely new:** the witnessed *relation* (§3.5), and a selection-record protocol (§3.4).

**Deletions available immediately**, from D81 §5.3: `chain_witness_category_for_iri` (0 callers),
`Grade`/`GradedClaim.grade` (write-only), `default_asserts_proposition{,_hash}` (0 consumers),
`runtimes:wasm` (declared, unimplemented).

---

## 5a. Implications that follow from §2a

The refinement model settles things §3 left open, and forces two conclusions D82's first draft did
not reach.

### 5a.1 A trace kind *is* a refinement declaration

This is the one that changes most. `trace_category`'s five arms are already refinement declarations
written as a Rust `match`: `DeclarationTrace → Declared`, and `ExternalExecutionTrace → Declared`
with a five-line rationale for why the projection is *that* one (eigenius#205).

Under §2a a trace class declares its own projection, exactly as an institution's witness kind does.
The two mechanisms unify: **a trace class and an institution witness kind are the same thing — a
named relation between a proposition and its evidence, projecting onto one of the four.** Adding a
trace kind stops being a kernel edit.

That retires `trace_category` and the self-attesting arms together (D81 §5.2), and the vocabulary is
already chain-resident and dead: `reflection:epistemic_status` carries `allows_only` over exactly the
four individuals and no Rust reads it.

### 5a.2 `reflection:Trace` should split

D81 §1.2 found three families under one name. The refinement model makes the split necessary rather
than merely tidy:

| family | what it is | under §2a |
|---|---|---|
| `Let`/`Map`/`Case`/`Seq`/… (11 classes) | the **inside** of one program run — a data structure | **not evidence.** Declares no projection, grounds no witness |
| `ProgramTrace`, `ExternalExecutionTrace` | a record that a run happened | a refinement, declaring its projection |
| `Declaration`/`Observation`/`VerificationTrace` | standalone attestations | refinements, declaring theirs |

Only the last two carry a projection; the first cannot, and the fact that it cannot is the test that
tells them apart. Calling all three "Trace" is the same category error as the two senses of
"witness" — one word, unrelated denotations, and here it is load-bearing rather than cosmetic.

### 5a.3 The obligations belong to the warrant, not the resource

`DeclaredResource requires declared_by`; `ObservedResource requires source`. Read under P1 these are
not statements about resources at all — they are *"a Declared warrant requires a named agent"* and
*"an Observed warrant requires a source"*. The obligation attaches to the **relation**, and it is
carried by the resource class only because that was the available hook.

This dissolves D81 §2.0's asymmetry without needing the refutation. `DerivedResource` requires
nothing *as a class* while every concrete derived path carries its own requirement — because the
requirement was never a property of the class. Under §2a each refinement declares its own
obligations, and the base categories need none.

It also explains why the labelling stack is inert. Once the obligations move, `is_a
reflection:DeclaredResource` carries no information a reader could act on — which is already true
today, and would become visibly so.

### 5a.4 `witness:Is*As` is kernel vocabulary, not the reasoning institution's

Those four inductives *are* the four categories expressed as propositions. Under §2a they belong
with the kernel's base vocabulary, not in `ontologies/reasoning/reasoning.esl` where they sit today.

What stays with the reasoning institution is `JustifiedBy` and `JustificationTerm` — the certificate
type and the J-family algebra (`app`, `sum_l`, `sum_r`, `spec_poly`, the evidence constructors). An
institution referencing base vocabulary is the normal direction (D14 §1.3: institutions build fibres
over the base), so nothing about the split is unusual — it is only currently inverted.

### 5a.5 `Warrant` is the prototype and should become the declared form

`Warrant::{Declared, Parsed}` with `Warrant::grade()` is exactly §2a's mechanism, built once, in one
crate, in Rust, `#[non_exhaustive]`, on an axis its own documentation says is expected to grow — and
which nothing outside `crates/eigenius-reasoning` can extend.

Generalising it is not new design. It is taking the shape that already works and putting it where
other logics can reach it.

---

## 6. Sequencing

- **S0 — the deletions.** No design content; clears noise before anything moves.
- **S1 — the witnessed relation** (§3.5). Everything else can be expressed once this exists; nothing
  should be built on a key that cannot say what it witnesses.
- **S2 — declared post-condition on `Holds`** (§3.3b). Smallest change with the largest reach: it
  makes the Lean gap (#160) a declaration error rather than an omission nobody notices.
- **S3 — institution-supplied witness synthesis** (§3.3a), through the existing `CheckHooks` seam.
  κ–τ is the acceptance test.
- **S4 — retire the kernel lists** (§3.3a) once S3 gives institutions somewhere to declare them.
- **S5 — the selection-record protocol** (§3.4). Independent; can run any time after S1.
- **S5a — persist discharged witnesses** (P7). After S1, because the relation belongs in the key
  before anything is stored under it. Unlocks the drift comparison D77/D80 both need.
- **S6 — split `reflection:Trace`** (§5a.2) and move the obligations onto refinements (§5a.3).
  Chain-vocabulary work, gated on S4 having somewhere to declare projections.

S1 before S2 is the only hard ordering. S2 alone would already close #160.

**S1 is smaller than it looks now.** §5a.5 says the relation vocabulary exists as `Warrant`, and
§5a.1 says trace kinds are the same mechanism — so S1 is generalising one enum and one `match` into
a declared form, not inventing a notion.

---

## 7. Open questions

1. **Does the witnessed relation belong in the key or beside it?** Widening `WitnessKey` re-forks
   every existing witness; a companion resource does not, but weakens the "witness is a term" story.
   **P7 raises the stakes**: once discharges are persisted, this is a migration. Sequence the
   relation vocabulary before persistence.
2. **Can an institution's witness be trusted, or must the kernel re-check it?** Statistics is
   recompute-checkable, Lean's proof term is re-checkable, κ–τ's score is recomputable — but nothing
   in the protocol *requires* a witness to be either. D54 §4.3's principle
   (*"lemma-citability ⇔ proposition-bearing + kernel-warranted"*) suggests it must.
3. ~~**Does `Verified` stay a grade, or become "witnessed by the kernel"?**~~ **Resolved by P3/§2a.**
   The four are kernel-core, so `Verified` stays a grade *and* the kernel's own term-checking is a
   refinement projecting onto it. No conflict.
3a. ~~**What does the kernel call its own witness kind?**~~ **Resolved with 3.** The kernel's
   term-checked position is the refinement `Verified` is reached by when the kernel itself
   discharges it — one refinement among the possible ones, and the only one the kernel supplies.
4. **What happens to `epistemic_status` once relations are declared?** It is currently written by one
   site and read by none; either it becomes the declared vocabulary of §3.3a or it should be deleted.

---

## 8. References

- [D81](d81-the-epistemic-stack.md) — the evidence base; every claim above traces to a section there
- D14 §1.2 (what an institution is), §7.2 (promotion assigned to the kernel)
- D39 §8 and D73 §9 (the canonical-encoding intention, and its withdrawal)
- D49 §6–§7 (the uniform emitter; the superseded Lean inverse translation)
- D52 §6 (promotion assigned to the kernel, again), §10 (the multi-resource channel recorded missing)
- D54 §4.2 (the class/witness decoupling decision), §4.3 (lemma-citability)
- D71 (why prose formalization is not an institution)
- Pareschi, *A Minimal κ–τ Logic for Risk-Sensitive Abduction*, arXiv:2608.08192
