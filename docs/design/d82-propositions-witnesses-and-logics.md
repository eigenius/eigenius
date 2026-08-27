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

**P3 — A logic owns its witnesses.** The rules by which a warrant is admitted belong to the logic
that defines the warrant. The kernel's job is to hold the proposition and to check terms in the one
language it owns.

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

**P6 — Not every warrant-producer is a logic.** An institution has a satisfaction relation.
Abductive selection does not, unless one is supplied. Both produce chain-resident warrant records;
only one is an institution.

---

## 3. The target shape

### 3.1 Three tiers, distinguished by what discharges the witness

| tier | discharges a witness by | grade it can reach | examples |
|---|---|---|---|
| **kernel** | type-checking a term against the proposition | `Verified` | a `JustifiedBy` certificate; any EigenTT derivation |
| **institution** (has ⊨) | evaluating its own satisfaction relation | its own, declared | statistics, Lean, κ–τ |
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

S1 before S2 is the only hard ordering. S2 alone would already close #160.

---

## 7. Open questions

1. **Does the witnessed relation belong in the key or beside it?** Widening `WitnessKey` re-forks
   every existing witness; a companion resource does not, but weakens the "witness is a term" story.
2. **Can an institution's witness be trusted, or must the kernel re-check it?** Statistics is
   recompute-checkable, Lean's proof term is re-checkable, κ–τ's score is recomputable — but nothing
   in the protocol *requires* a witness to be either. D54 §4.3's principle
   (*"lemma-citability ⇔ proposition-bearing + kernel-warranted"*) suggests it must.
3. **Does `Verified` stay a grade, or become "witnessed by the kernel"?** Under P4 they are the same
   thing, which would make the fourth grade a derived notion rather than a primitive.
3a. **If the four categories move to the reasoning institution (P5), what does the kernel call its
   own witness kind?** It needs one — the term-checked position of P4 — and calling it `Verified`
   re-imports the stratification P5 just exported.
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
