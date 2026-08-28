# D82 — Propositions, witnesses, and where logics live

**Status: superseded as a design by [*Judgements, Warrants, and Logics*](judgements-and-warrants.tex);
retained as the derivation record.** That paper states the target shape. This document is how it was
reached — including several readings that were tried and withdrawn, each marked in place. Read it for
*why* a decision went the way it did, not for what to build.

**Two conclusions here are superseded, not merely restated.** The paper rejects this document's
institution criterion (an institution is *not* defined by having a satisfaction relation the kernel
cannot evaluate; proof-theoretic institutions exist, and the kernel is a degenerate one), and refutes
its constructive/classical conjecture (classicality is a property of a theory's axioms, which behave
as typed constants during checking, so it does not determine which logics supply terms).

**No code.** Rearchitecture of the epistemic machinery, taking
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

**The determinism argument against it is sound, and bounds what P7 may claim.**
`witness_index.rs:17-20` states the case: admission is *"a pure deterministic function of that
Layer's Trace-class resources — content-addressed transitively via the Layer's own content hash, so
nothing here is persisted."* That is correct. For a fixed chain, recomputation cannot disagree with
what was asserted, so **P7 buys no soundness** — and in particular it does **not** detect D80 §2's
environment-blindness. Storing the discharge stores the same environment-blind `WitnessKey`;
recomputing it against a rebound chain hits the same ancestor by first-hit-wins and returns the same
answer. **S1 fixes that; P7 does not, and the two are independent.**

**What persistence buys is that the evidence exists.** A recomputable witness is not an object —
it is a promise that an object could be reconstructed, redeemable only where the chain is and only
by the kernel. Three concrete consequences:

- **Nothing can cite it.** A certificate records that it was accepted, not what accepted it. There
  is no IRI for "the warrant this certificate rests on", so no resource can point at one, and §5a.3's
  obligations have nothing to attach to.
- **Nothing can transport it.** Merge, export, and a collaborator's institution all cross a boundary
  the recomputation cannot: off this chain, the witness is unavailable, not false.
- **The admitting layer is unrecoverable.** Recomputation answers *whether* some ancestor admits the
  key, never *which* — first-hit-wins discards it. "What warranted this?" is currently unanswerable
  even in principle.

**And it makes the oracle auditable.** §2's P5 establishes that a witness is an *axiom the kernel
asserts* — `Val::ChainWitness(key)`, an inhabitant of a zero-constructor type introduced by fiat. The
system persists the trace, the verdict and the derivation, and keeps no record of the one step that
is postulated rather than proved. That is backwards. A stored witness is the assertion on the chain
with its stated reason, in the same form as everything else.

**Cost accepted:** widening `WitnessKey` later (§7 Q1) becomes a migration rather than a no-op. Under
the pre-production posture that is a reseed, and §5a.5's relation vocabulary should therefore land
*before* P7 rather than after — sequencing, not dependency.

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
| **institution** (has ⊨) | evaluating its own satisfaction relation | `Derived`; or `Verified` **only** by surrendering a checkable proof term (§5b) | statistics and κ–τ (`Derived`); Lean (`Verified`) |
| **selection producer** (no ⊨) | recording a constrained choice and its authority | `Declared`, with an auditable record | the encoding pipeline |

**Corrected by §5b.** An earlier draft of this row read *"a declared refinement projecting onto one
of the four"*, which would have let an institution nominate its own grade. It cannot: `Verified` is
reached only through a proof term the kernel checks, and everything else an institution produces is
`Derived`. Lean is on the middle tier and reaches `Verified` because it hands over a Lean 4 term —
not because it declared that it could.

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

> **Superseded by [*Judgements, Warrants, and Logics*](judgements-and-warrants.tex) §7, which places
> κ–τ in the finished framework.** Under the `Verified`-means-proof-term decision it is a `Derived` institution needing
> no protocol change. This section stands as the analysis that established that; it no longer
> motivates S3.

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

## 5b. The bridge: the design is mostly already declared

**Decided `2026-08-26`.** An institution produces exactly two grades: **Derived** or **Verified**.
`Verified` means the kernel holds a proof term it can check — an **EigenTT** term, or a **Lean 4**
term accepted as a substitute and checked in-process via `nanoda_lib`. Nothing else is `Verified`.
`Declared` and `Observed` are not institution outputs at all.

This is not new design. `reflection:VerificationTrace` already declares it
(`ontologies/reflection/reflection-ontology.json`), `requires` **`proof_term`** and
**`proof_system`**, and its description states the two-verifier rule verbatim:

> *"TWO VERIFIERS PRODUCE THIS, distinguished by `proof_system`, not by class: an external prover
> (lean4, coq, agda) whose exported proof blob is checked, and the kernel itself, whose type-checked
> `JustifiedBy` certificate IS the proof term. Kernel-checking is verification"* (eigenius#200).

D28 built the external half — proof terms commit as chain resources and the kernel re-checks them
in-process. The rule is declared, validator-enforced through `requires`, and has a working
precedent.

### 5b.1 The method this suggests

Both findings examined so far have the same shape, and it is not the shape D82 §1 assumed:

| finding | the rule, already declared | what the code does |
|---|---|---|
| §5.2 — *"three semantic relations are kernel-only"* | `reflection:epistemic_status`, `allows_only` over exactly the four grades, attached to `ProgramTrace`'s `recommends` | `trace_category`'s hard-coded match; `grep epistemic_status` over `*.rs` returns **zero hits** |
| §5.2 — *"`Verified` rests on a conjunction nothing records"* | `VerificationTrace requires proof_term + proof_system` | `emit_from_reasoning_sentence` (`witness_index.rs:262`) mints `Verified` from `is_a == ReasoningSentence` plus a hashable proposition — **no proof term, no trace, no `proof_system`** |

So the bridge from findings to design is not *"invent the right shape"*. For each finding, ask:
**is the correct rule already declared in the chain vocabulary, and is Rust routing around it?**
Twice out of twice, it is. The revised design is then **make the declarations load-bearing and
delete the shortcuts** — which is §5a.5's move (*"taking the shape that already works and putting it
where other logics can reach it"*) generalised into the working method.

This is also the only bridge consistent with the project's posture. The ontology is executable and
validator-checked; a design document is neither. Where the two disagree the ontology is the design,
and D82's remaining job is to say which Rust shortcuts contradict it.

### 5b.2 What the decision does to this document

- **P3 narrows.** Institutions do not declare witness kinds with a projection. They land in `Derived`
  or `Verified`, and `Verified` is *defined* by kernel-checkability — not chosen by the producer.
- **P4 is promoted from a lane to a definition.** "Where the kernel can type-check a term against a
  proposition, that *is* verification" stops describing the kernel's own corner and becomes the
  admission rule for everyone.
- **S3 largely evaporates.** There is nothing for an institution to supply a *synthesis* for: it
  either hands over a checkable proof term, or it is `Derived`. The `CheckHooks` extension S3
  proposed is not needed for the cases in hand.
- **κ–τ needs no protocol change.** Its score is not a proof term, so it is a `Derived` institution
  and the existing `InstitutionEmittedDerivation` path already carries it. §4's forcing case turns
  out to force nothing — which is the honest outcome, and better than extending a protocol for it.
- **The defect list gets one hard entry.** `emit_from_reasoning_sentence` violates the declared rule.
  The certificate it should be keyed to *does* exist — a `ReasoningSentence` only commits if
  AutoOnLoad's `ValidateJustification` type-checked its `JustifiedBy` certificate, and the ontology
  says that certificate **is** the proof term. The arm is not unfounded; it is **unrecorded**, which
  is exactly what D81 §5.2 found and could not name. Keying it to the checked certificate rather
  than to `is_a` closes the finding and makes the relation intrinsic (§3.5).

### 5b.3 Q1 answered by derivation

§7 Q1 asked whether the witnessed relation belongs in `WitnessKey` or beside it. Under this
decision it belongs in **neither**, per lane:

- **`Verified`** — the relation is carried by the **proof term's type**. Checking the term against
  the proposition *is* establishing the relation, so there is nothing to record separately and no
  subject problem: a mismatched subject fails the check.
- **`Derived`** — the relation is a property of the emitted derivation resource, where
  `epistemic_status` already sits unread.
- **`Declared` / `Observed`** — trace-grounded, same vocabulary on the trace class, replacing
  `trace_category`'s hard-coded lists.

So `WitnessKey` is not widened, and S5a's migration cost (§7 Q1) does not arise.

---


### 5b.4 Where the declarations are themselves wrong

§5b.1's method — *find where Rust routes around the declared rule* — is half right. It found a real
defect (S4a), but applied uniformly it **launders the ontology's mistakes into the design**. The
declarations are evidence of intent, not authority; each one still has to survive the §5b
definition. Three do not.

**(i) `VerifiedResource subclass_of DerivedResource` is backwards.** `DerivedResource` is *"produced
by a typed program from other resources"*; `VerifiedResource` is *"carries a machine-checked formal
proof"*. A proof term is **checked, not produced** — a hand-written Lean proof was computed by no
program. The subclass forces `VerifiedResource requires derivation`, i.e. a `ProgramTrace` pointer
for something no program ran.

The relation exists to make `IsVerifiedAs` coerce to `IsDerivedAs` at lookup (`witness_index.rs:1024`,
D49 §4) — a **citation convenience encoded as a subsumption**. Under §5b the two answer different
questions: `Derived` = what computed it; `Verified` = does the kernel hold a term that checks.
Neither entails the other. Breaking the subclass costs the coercion, and the question that decides
it is whether *verified ⇒ derived* is a real entailment or a shortcut. It is a shortcut: `Derived`
requires a producing program and a proof term has none.

**(ii) `reflection:epistemic_status` should be deleted, not wired up.** It carries `allows_only` over
the four grade individuals, sits on `ProgramTrace`'s `recommends`, and is described as *"Epistemic
status of the traced output"* — so **a trace declares which grade its output has**. That is exactly
the self-nomination §5b rules out, one level down: an institution cannot nominate its own grade, and
neither should its trace.

So D81 §5.2's finding (*"the vocabulary exists, is chain-resident, and no Rust file reads it"*) has
the **opposite** resolution from the one §5b.1 proposed. **The zero readers are correct; the
declaration is the mistake.** The grade follows from the shape of the evidence — a checked proof
term ⇒ `Verified`, a recorded kernel invocation ⇒ `Derived` — and needs no vocabulary, because a
derived grade cannot be lied about.

**S4 is therefore reversed**: delete `epistemic_status` and derive the grade from evidence, rather
than replace `trace_category`'s match with a lookup that reintroduces the hole S4a closes.

**(iii) The four grades answer three different questions.**

| grade | `requires` | the question it answers |
|---|---|---|
| `DeclaredResource` | `declared_by` | **origin** — who asserted it |
| `ObservedResource` | `source` | **origin** — where it came from |
| `DerivedResource` | — | **process** — what computed it |
| `VerifiedResource` | `derivation`, `verification` | **evidence** — does a checked term exist |

Origin, process, evidence. One enum, three axes — which is what produced (i)'s bad subsumption and
plausibly what D81 §1 was seeing when it found the four-way encoded nine times. §5b makes the split
visible: institutions touch only the last two, and the first two are not institution outputs
**because they are not derivations at all — they are attributions**.

**The follow-through — corrected.** An earlier version of this paragraph said
`JustifiedBy.declared(w, P)` *"asserts `P` on the strength of someone having said so, which is not
justification in Artemov's sense."* Both halves are wrong, and the declaration says why.

`JustifiedBy` has **no factivity rule**. Its seven constructors are the four groundings, `app`,
`sum_l`/`sum_r` and `spec_poly` (`ontologies/reasoning/reasoning.esl:112-190`) — there is no
`JustifiedBy(j, P) → P`, and no positive introspection. So the system implements **J**, the
*non-factive* base of the justification-logic family, not **LP** (= JT4). A certificate never asserts
its proposition; it records that `j` grounds a claim to it. And non-factive justification is squarely
Artemov's — J, JD and J4 are his systems too. `declared` sitting in J is exactly right.

**A second retraction, on the same paragraph.** It went on to claim that `app` *"will compose a
declared rule with a verified sentence and hand back a certificate whose weakest link is a
declaration, unrecorded"*, and that a factivity predicate *"becomes definable for the first time"*.
Both false. **The polynomial is the recording**, which is the point of proof polynomials: `App(
DeclaredEvidence(rule), VerifiedEvidence(sentence))` carries both leaves, labelled by family, and
the weakest link is a question *about the term*.

**And the fold is built.** `crates/eigenius-reasoning/src/project.rs` computes `support(t)` — the
disjunctive normal form, the set of alternative minimal leaf-sets any one of which carries the
conclusion:

| term | support |
|---|---|
| leaf `L` | `{{L}}` |
| `App(a, b)` | `{ sa ∪ sb : … }` — conjunctive, both needed |
| `Sum(a, b)` | `support(a) ∪ support(b)` — **disjunctive, either suffices** |
| `SpecStr(j, tag)` | `support(j)` |

`is_fully_verified`, `survives_without`, `leaves_of` and `cited_iris` read it, and
`qc_project_justification` exposes it as a query. `Sum`'s disjunctivity is handled exactly right —
`Sum(VerifiedEvidence(a), DeclaredEvidence(b))` **is** verified because the `a` branch alone carries
it, and the module notes that reading `Sum` conjunctively is the error D39 §8 made. The cap is an
error rather than a silent truncation.

**The real point, which is narrower and sharper than either wrong version.** `is_fully_verified`
trusts the **label** on the leaf: `VerifiedEvidence(iri)` is factive only if whatever graded `iri`
as `Verified` was entitled to. That is precisely §5b.1's defect —
`emit_from_reasoning_sentence` grading from class membership. So the projection algebra is correct
and complete, sitting on an admission layer that can be fooled, and **S4a is what makes
`is_fully_verified` mean what its name says.** The analysis layer was built right; its inputs are
what needs fixing.

The open question §2a never asked therefore stands, but smaller: not whether the four categories are
*enough*, and not whether the polynomial records them — it does — but whether `Declared` and
`Observed`, being attributions rather than proofs, should carry the same *admission* discipline as
the two that are checkable.

**And the oracle has a name in this literature.** LP justifies axioms by **proof constants** under a
**constant specification** — you stipulate `c:A` because `A` cannot be proved from below. `Val::ChainWitness`
*is* a proof constant and `layer_admits_witness` *is* the constant specification. LP's soundness
condition is that the specification be **axiomatically appropriate**: constants only for genuine
axioms. `emit_from_reasoning_sentence` minting `Verified` from class membership (§5b.1) is therefore
not merely unrecorded — it is an **inappropriate constant specification**, which is the precise name
for the defect D81 §5.2 found and could not characterise.

---

### 5b.5 The implication, concretely: declared content launders into `Verified`

The pieces above compose into a **live wrong answer**, reachable on the designed citation path:

1. `DeclaredClaimGrader` writes a `reasoning:ReasoningSentence` whose own justification is
   `DeclaredEvidence(declaring)` — someone asserted it (`grade.rs:260-270`).
2. `emit_from_reasoning_sentence` mints `IsVerifiedAs(sentence_iri, P)` for **any** `ReasoningSentence`
   with a hashable proposition (`witness_index.rs:262`). It never inspects that sentence's own
   justification.
3. A later sentence cites it with `JustifiedBy.verified` — `ChainRuleApplication`'s documented
   lemma-citation path, *"the prior sentence is cited with `verified`"* (`grade.rs:542`).
4. Its support is `{VerifiedEvidence(sentence_iri)}`, so **`is_fully_verified` returns `true`** for a
   claim whose entire provenance is a declaration.

Nothing in `project.rs` is at fault: it answers correctly about the leaves it is given, and the leaf
is labelled `Verified` before it arrives. Every projection reads wrong in the **reassuring**
direction — `is_fully_verified` says yes, and `leaves_of(term, Declared)` returns empty for a claim
resting entirely on declarations.

**Why this was not statable before §5b.** Under the old reading — `Verified` = *emitted by something
we trust* — step 2 is defensible: a `ReasoningSentence` commits only if AutoOnLoad type-checked its
certificate, so something *was* checked. What was checked is that the certificate is **well-formed**,
which `JustifiedBy(DeclaredEvidence(x), P)` is. Once `Verified` means *the kernel holds a proof term
for `P`*, step 2 is simply false, and the defect has a name (§5b.4: an inappropriate constant
specification).

**Priority.** S4a is the only item in §6 that closes an incorrect answer rather than tidying
structure, and it needs no new vocabulary. It goes first.

---

### 5b.6 The root: the kernel's "proof term" proves the wrong proposition

A hand-authored proposition supplied with a hand-authored **matching** proof term would be
`Verified`, correctly and with no further machinery. Authorship is irrelevant to §5b — a human
writing an EigenTT term is exactly as good as Lean producing one. *Matching* is the whole content:
the term must inhabit **`P`**.

The kernel's verification lane does not do that. `validate.rs:186` mints the `VerificationTrace` on a
passing `ValidateJustification` with:

- `proof_system = urn:eigenius:kernel`
- `proof_term = <the sentence's own IRI>` — *"the certificate lives on the sentence, so the sentence
  IS the proof term's location"*

The certificate has type `JustifiedBy(j, P)`. The proposition is `P : Prop`. `JustifiedBy` is
declared `JustificationTerm -> Prop -> Type 2`, and §5b.4 established there is **no factivity rule**
— no `JustifiedBy(j, P) → P`. So the certificate is not a proof of `P` and cannot be turned into
one. **Kernel-verification certifies a different proposition than the one it names**: it proves
*"`j` justifies `P`"*, and when `j = DeclaredEvidence(x)` that is a perfectly valid proof that
someone declared `P`.

This is the root of §5b.5's laundering, one level below `emit_from_reasoning_sentence`. The
`VerificationTrace` is not missing — it is minted, and its `proof_term` points at a proof of the
wrong statement. The ontology encodes the same confusion, so §5b.1's *"the rule is already
declared"* was too generous here: *"the kernel itself, whose type-checked `JustifiedBy` certificate
**IS the proof term**"* is the error, in the declaration.

**What the fix requires.** For `proof_system = kernel`, `proof_term` must name a term `t` that the
kernel type-checked at `t : P`, where `P` is the sentence's `proposition`. Today **there is nowhere
to put such a term**: `reasoning:certificate` holds a `JustifiedBy` value, and no field carries an
EigenTT inhabitant of `P`. The Lean lane has this right — `LeanProofTerm` carries an actual proof of
the mirrored proposition — so the gap is the kernel lane only.

So the change is additive and small: a field on `ReasoningSentence` (or a sibling class) carrying an
EigenTT term, checked against `proposition` at commit. A sentence that supplies one is `Verified`;
one that does not is graded by its justification — `Declared` for `DeclaredEvidence`, and so on.
That subsumes S4a: keying `Verified` to the `VerificationTrace` is only correct once the trace's
`proof_term` means what it says.

**And it makes the hand-authored path first-class**, which it is not today. A human can currently
reach `Verified` only by routing through Lean. With a checked EigenTT term on the sentence, an
author can discharge `P` directly in the kernel's own language — which is what P4 always implied and
what the `proof_system` field was already shaped to record.

---

### 5b.7 The institution boundary under the revised model

**Criterion: an institution is a logic with a satisfaction relation the kernel cannot evaluate.** If
the kernel can evaluate it — that is type checking — it is not an institution, it is the kernel.

**Contributes:** vocabulary for its sentences; a decision procedure yielding the tri-state verdict;
derivation resources recording what it computed (`InstitutionEmittedDerivation`, with `from_subject`
pinning the spec it ran); optionally a proof term in EigenTT or Lean 4.

**Never:** assigns a grade (computed by the kernel from evidence, §5b.6); admits a witness (the
kernel's constant specification, §5b.4); asserts `Verified`.

**Lean shows why the last one generalises.** Lean is an institution *and* a proof-term source, and
the roles are separable. Its **verdict** does not earn `Verified`; its **term** does. Lean answering
`Holds` while shipping no term would be `Derived`.

**The boundary stated as trust:**

| direction | trust required | why it is safe |
|---|---|---|
| `Verified` | none — the kernel re-checks the term | that is the definition |
| `Derived` | bounded and attributed — which institution, which invocation, which subject | recorded, so a wrong answer is traceable |
| `Fails` blocks a commit | full, on the institution's own authority | wrong-direction-safe: a bad `Fails` loses data, a bad `Holds` corrupts |

So: **an institution may veto on its own authority, but may not verify on its own authority.**

**Consequence — the reasoning institution is not one.** `reasoning:reasoning_institution` declares
`runtime = in_process` and states *"the validator is the kernel"*. It has no satisfaction relation of
its own: `JustifiedBy` checking is type checking. Under the criterion it dissolves —

- `JustifiedBy` checking → kernel (already true in fact)
- `witness:Is*As` → kernel vocabulary (§5a.4 reached this from the other side)
- the `JustificationTerm` algebra → the justification layer, part of the kernel's type theory once
  `JustifiedBy` is a kernel inductive
- `project.rs`'s support algebra → a **query** over retained terms, not a logic
- the AutoOnLoad trigger → a kernel dispatch mechanism

Statistics, Lean and κ–τ remain institutions: each has a ⊨ the kernel cannot evaluate (`p < α`,
Lean's type theory, `sc_S(φ) ≥ τ`). The encoding pipeline correctly is not one — no ⊨, and it
produces assumptions with attribution (§5b.6's third case), which is what D71 refused institution
status for and gave no protocol in exchange.

---

### 5b.8 Resources that carry no proposition

**The enum carries two independent questions, and only one is about propositions.**

| axis | question | applies to |
|---|---|---|
| **provenance** | how did this artifact come to exist? | **every** resource |
| **warrant** | what evidence exists for its proposition? | only resources carrying one |

`Declared` / `Observed` / `Derived` are provenance — a person authored it, it was imported from a
source, a program produced it — with `declared_by` / `source` / `derivation` as their attributions.
`Verified` is not a fourth member of that series; it answers the other question. §5b.4(iii) reached
this by inspecting the `requires` clauses; the encoding ontology reached it by hitting the limit.

**The encoding ontology had to split one artifact into two resources to say both things:**

- `enc:EncodedClaim : reflection:DeclaredResource` — *"the parser establishes that the text parses to
  this well-typed term, not that the term is faithful to what the author wrote nor that what the
  author wrote is true"*
- `enc:ReasoningStructure : reflection:DerivedResource` — *"the output of a program run over hashed
  input"*, owning the single `ProgramTrace`

One claim, two facts: a program produced it, and an agent vouches for it. `is_a` holds one grade, so
the provenance was pushed onto a second resource. D81 §6 called this *"where the modelling was done
carefully"* — it is, and the split is the workaround a careful modeller writes when one field must
answer two questions.

**So a regular resource has provenance and no warrant.** For the ~9.4M lexicon entries, class
declarations and imported concepts on the chain, *"what proves this?"* is not an under-answered
question — it is not a question. They are not in the epistemic system at all.

**Three consequences.**

- **It explains D81 §5.1.** *"No reader grants an entitlement on the strength of an epistemic class"*
  — for nearly every resource the class is provenance, which is descriptive and was never meant to
  entitle anything.
- **It dissolves the importer worry.** D81 asked what stops an importer stamping `VerifiedResource`
  on anything. Under the split it cannot: `Verified` is not on the axis importers write to. The
  missing guard becomes an unnecessary one.
- **It kills the subsumption independently of §5b.4(i).** `VerifiedResource subclass_of
  DerivedResource` relates two different axes, so no subsumption is available in either direction.

**The test for which axis applies is mechanical and already in the tree:** does the resource carry
`reflection:canonical_proposition`? If yes, warrant applies. If no, provenance only.

---

### 5b.9 `eigentt:TypeExpr` is untyped, and `PROPOSITION_SLOTS` is the patch

Rule 21's third step is a hardcoded special case:

```rust
if wk::PROPOSITION_SLOTS.contains(&prop_iri.as_str())
    && !matches!(&inferred, Val::Sort(l) if l.is_nat(0))
```

**The oddness is the symptom; the cause is that a property's range cannot say what type its
EigenTT value must have.** `class_types ∋ eigentt:TypeExpr` says only *"an EigenTT tree"* — and that
range covers propositions, types and terms alike, so the one thing the kernel is best at computing
is exactly the thing the ontology cannot express. It is then patched back in Rust, **for one case
out of five**.

**The other four are silently unchecked.** Rule 21 runs `check_infer` and, for any property not in
the list, **discards the inferred type**:

| property | declared intent | actually checked |
|---|---|---|
| `canonical_proposition`, `reasoning:proposition`, … | inhabits `Prop` | yes — via the Rust list |
| `lexicon:cat` | *"a value of the inductive `lexicon:Cat` … Kernel-checked"* | **no** |
| `lexicon:sem_type` | the EigenTT type `⟦cat⟧` | **no** |
| `eigentt:axiom_statement` / `definition_type` | `Sort(1)` / `Sort(2)` | **no** |

`type_expr(42)` in a `lexicon:cat` slot passes Rule 21 today. `lexicon:cat` is written by
`dcg/glossary.rs`, `dcg/augment.rs` and `lexicon-align/emit.rs`, and read by the parser
(`dcg/lexicon.rs`), so the ontology's *"Kernel-checked"* is half true: the value is well-formed
EigenTT, but nothing establishes it is a `Cat`. `PROPOSITION_SLOTS` is therefore not *"the
propositions we are strict about"* — it is **the one typed obligation that got encoded**.

**The fix, and why it subsumes §5b.6.** Let an EigenTT-valued property declare the type its values
must inhabit; Rule 21 becomes `check(ctx, exp, expected)` — the entry point already exists at
`nbe/check/mod.rs:523` — instead of `check_infer` plus a list. The rows above become declarations.
And **the proof term is the same mechanism with a dependent expected type**: not a constant, but the
value of the sibling `proposition` slot on the same resource. One extra form, and §5b.6's
"nowhere to put a proof term" stops being new machinery and becomes the general case of a rule that
should have been uniform from the start.

**It also deletes a kernel-only list rather than adding a fourth**, which is the direction §5b.4
argues for — and the distinction matters: an *obligation declared by the property and discharged by
the kernel* is safe, unlike a *grade declared by a resource and thereby received* (§5b.4(ii)). The
first is a typing constraint the kernel enforces; the second is self-nomination.

---

## 6. Sequencing

**Re-scoped by §5b.** The steps below were written before the `Verified`-means-proof-term decision;
S3 and S4 are superseded by it, and the ordering now follows what is *earned* rather than what is
architecturally tidy. D81 §5.6's own handoff — *"three dead artifacts to delete, three stale
assertions to correct, three untested claims to pin, and one design question"* — is strand one, and
is the part backed by measurement.

- **S4b — typed ranges for `eigentt:TypeExpr` properties** (§5b.9). Precedes S4a: declare the
  expected type per property, switch Rule 21 to `check`, retire `PROPOSITION_SLOTS`. Closes four
  silently-unchecked obligations and makes S4a a dependent instance rather than new machinery.
- **S4a — give the kernel lane a real proof term** (§5b.6), then key `Verified` to it. **First**:
  the only step that closes a live wrong answer. A field on `ReasoningSentence` carrying an EigenTT
  term checked against `proposition`; `Verified` iff that check passed. Subsumes the earlier
  "key `Verified` to the `VerificationTrace`" formulation, which is correct only once the trace's
  `proof_term` proves `P` rather than `JustifiedBy(j, P)`.
- **S0 — the deletions.** No design content; clears noise before anything moves.
- **S1 — the witnessed relation** (§3.5). Everything else can be expressed once this exists; nothing
  should be built on a key that cannot say what it witnesses.
- **S2 — declared post-condition on `Holds`** (§3.3b). Smallest change with the largest reach: it
  makes the Lean gap (#160) a declaration error rather than an omission nobody notices.
- ~~**S3 — institution-supplied witness synthesis.**~~ **Superseded by §5b.2.** An institution
  hands over a checkable proof term or it is `Derived`; there is no synthesis to supply.
- **S4 — derive the grade from the evidence** (§5b.4(ii)). Reversed twice over: not "let
  institutions declare projections" (S3, struck), and not "read `epistemic_status`" either — that
  property lets a trace nominate its own grade and should be **deleted**. `trace_category`'s match
  goes, replaced by the shape of the evidence, not by a lookup.
- **S4a — key `Verified` to the checked certificate.** Replace
  `emit_from_reasoning_sentence`'s `is_a` test (`witness_index.rs:262`) with the `VerificationTrace`
  the ontology already requires. Closes D81 §5.2's standing finding and the §3.5 subject problem in
  the one lane where a subject mismatch is detectable.
- **S5 — the selection-record protocol** (§3.4). Independent; can run any time after S1.
- **S5a — persist discharged witnesses** (P7). After S1 to avoid storing under a key about to
  change — but S1 does not depend on it. Gives §5a.3's obligations something to attach to.
- **S6 — split `reflection:Trace`** (§5a.2) and move the obligations onto refinements (§5a.3).
  Chain-vocabulary work, gated on S4 having somewhere to declare projections.

S1 before S2 is the only hard ordering. S2 alone would already close #160.

**S1 is smaller than it looks now.** §5a.5 says the relation vocabulary exists as `Warrant`, and
§5a.1 says trace kinds are the same mechanism — so S1 is generalising one enum and one `match` into
a declared form, not inventing a notion.

---

## 7. Open questions

1. ~~**Does the witnessed relation belong in the key or beside it?**~~ **Resolved by §5b.3** —
   neither. Per lane: carried by the proof term's type (`Verified`), by the emitted derivation
   resource (`Derived`), or by the trace class (`Declared`/`Observed`). `WitnessKey` is not widened,
   so P7's migration cost does not arise.
2. ~~**Can an institution's witness be trusted, or must the kernel re-check it?**~~ **Resolved by
   §5b** — an institution produces `Derived` or `Verified`; `Verified` requires a proof term the
   kernel checks (EigenTT, or Lean 4 via `nanoda_lib`). Recomputability is *not* sufficient, so
   statistics and κ–τ are `Derived`. This is D54 §4.3's *"lemma-citability ⇔ proposition-bearing +
   kernel-warranted"* made into an admission rule.
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
