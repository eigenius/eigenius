# Explainer: AI Computed Provenance

Draft — 12 August 2026 · Companion to
[AI Computed Provenance 1.0](ai-computed-provenance-1.0.md) · Non-normative

---

## Participants

Editors: *to be named.* Input to the proposed **AI Computed Provenance Community Group** (proposed
11 August 2026).

**Stakeholder feedback: none yet.** The group is proposed, not chartered. This document has not been
reviewed by W3C, by the group, or by implementers other than the authors of the reference
implementation. Nothing below should be read as reflecting consensus.

---

## Introduction

An AI-assisted research system produces a conclusion. You want to know whether to act on it.

Today you get a provenance record: this was derived from that, by this model, at this time. It is a
graph of assertions — nodes and edges, each atomic and opaque. Such a graph can record *that* things
are related. It cannot record *why a claim holds*, because it has no notion of what a claim says and
no notion of validity. A sound derivation and an invented one have the same shape in it.

That is the first gap, and it is representational, not cryptographic. The second gap compounds it:
every field in the record was written by the system that produced the conclusion, so even the
relationships it can express are worth exactly the producer's trustworthiness.

This specification closes both, and it does so by **changing levels** — from flat data to reasoning.

In the record we propose, a claim has **content**: a proposition in a formal language, with a
decidable notion of when two claims are the same one. A warrant has **structure**: a term recording
which evidence it grounds in and how those groundings compose. And validity is **decidable**: whether
a warrant justifies a claim is settled by type-checking, not by inspection.

That level shift is what makes the second gap closable. Once a warrant is a checkable object, its
leaves can be made unforgeable — statements that exist only because an implementation performed and
validated specific work, with no syntax, API, or configuration through which anyone can supply one.
A third party re-checks them by recomputing hashes and looking for the work that admits them, without
asking the producer anything.

We call that second property **Computed ≠ Asserted**. It is the headline, but it rests on the level
shift: you cannot make a warrant unforgeable until you have a warrant.

---

## The user-facing problem

Existing provenance formats sit at the artifact level: what came from what, what ran, who signed it.
That is the right level for the questions they were built for, and the wrong level for "is this
conclusion warranted, and by what?" — a question about claims, not artifacts. Asking it of a
lineage graph gets an answer about file ancestry.

Three people are hurt by this, concretely.

**The reviewer** reads a conclusion and wants its dependencies. They can get a list of cited
documents. They cannot get an answer to "which steps here were *checked* and which were *chosen*,
and by what?" — because the record does not distinguish them. A retrieval that found the right
paper and a model that picked one reading of an ambiguous sentence appear in the record as the same
kind of event.

**The second laboratory** wants to reproduce. They can re-run the pipeline and compare outputs. When
the outputs differ they cannot localise the difference, because the non-deterministic steps were not
recorded in a form that can be replayed independently of the components that made them.

**The regulator** asks what a submission takes on authority. The honest answer is a number: how many
claims in this record rest on somebody's say-so rather than on measurement or proof, and which ones.
No current format can produce that number, because "somebody's say-so" is not a distinguished
category and inferential steps are not represented at all.

The volume makes all three worse. A system generating thousands of claims exceeds the capacity to
spot-check, so the properties have to be structural or they are not there.

---

## Goals

1. **Make grounding non-forgeable.** A record cannot state that something was computed unless it was.
2. **Make audit independently reproducible.** A verifier sharing no code with the producer can
   re-check every address, every certificate, and every grade, using only the record.
3. **Make authority enumerable.** A reader can list exactly what a record takes on somebody's word,
   and that list is finite and small.
4. **Record AI choices honestly.** Every step whose outcome the input did not determine is recorded
   with its authority, its alternatives, and a key that replays it.
5. **Account for failure.** A record covers every unit of input, including the units that produced
   nothing.
6. **Stay implementable by anyone.** No component of the guarantee may depend on a specific vendor,
   service, or hardware root of trust.

## Non-goals

1. **Not attribution.** This specification does not establish *who* produced a record. It provides
   integrity and reproducibility. Signing belongs in a layer above, and the specification says so
   rather than implying otherwise.
2. **Not a correctness oracle.** A conforming record can faithfully document a wrong conclusion. The
   grades say how a claim was established, not whether it is true.
3. **Not a domain vocabulary.** What a biology claim means is authored, not standardised here.
4. **Not model evaluation.** Nothing here scores a model, and confidence values are explicitly not
   grades.
5. **Not a replacement for W3C PROV.** PROV answers artifact-level questions and answers them well.
   This sits a level above and maps down to PROV for tooling interop, rather than competing with it.
   See [alternatives](#considered-alternatives).
6. **Not prescriptive about AI.** How a model is invoked, prompted, or chosen is out of scope; only
   what must be *recorded* about its choices is in scope.
7. **Not defeasible reasoning.** Belief revision is a structural marker, not a non-monotonic logic.

---

## The level shift: three theories, three jobs

Nothing in the foundations below is invented here. Each is decades-old, well-understood mathematics
with textbooks, proof assistants, and independent implementations behind it. The contribution is the
assembly and the interfaces — which is the right posture for standards work, because implementers can
reason about established theory and check our use of it against sources we do not control.

Each of the three does one job that the other two cannot.

### Constructive type theory — gives claims content

A proposition is a type; a proof is a term inhabiting it; checking a proof is type-checking. The
constructive commitment is that to assert something exists you must **exhibit it**: a proof is not a
certificate that a witness could be found, it is the witness.

Two decidable questions follow, and everything else depends on them:

- **Are these the same claim?** Settled by the language's equivalence relation, not by string
  comparison. This is what lets a citation bind to a proposition rather than to a name.
- **Does this warrant justify this claim?** Settled by type-checking.

*Without it:* claims are strings, sameness is textual, and nothing is checkable. This is the position
flat-data provenance is in.

### Justification logic — makes the warrant an object

Artemov's Logic of Proofs treats the warrant as first-class syntax: `t : A` reads "*t* is a
justification for *A*", with explicit operators for composing justifications.

The practical consequence is that a warrant is **stored, inspected, and audited** rather than
discarded once checked. An auditor reads *how* a claim was justified. Several independent warrants for
one claim coexist without collapsing into one. And — the part no proof system gives on its own — a
single calculus expresses *justified by authority* alongside *justified by proof*, so the epistemic
grade is computed from the warrant's shape rather than attached as a label.

*Without it:* you would have proofs but no audit surface, and no uniform way to represent a claim
resting on somebody's say-so next to one resting on a machine-checked proof.

### Institution theory — lets many logics coexist

Goguen and Burstall's institutions formalise what a *logical system* is — signatures, sentences,
models, satisfaction — together with the satisfaction condition: truth is invariant under change of
notation. **Comorphisms** are truth-preserving translations between logical systems.

This is what makes the platform extensible without a universal ontology. A statistics system, a proof
assistant, a solver each keep their own sentences and their own notion of a verdict, and are related
by *declared* translations with stated semantics rather than by ad-hoc glue.

*Without it:* either one global logic — which no real scientific domain has — or bridges between
systems whose meaning nobody can state.

**This is also the formal content of the group's neutrality commitment.** "Immune to single-vendor
enclosure" is usually a governance aspiration. Here it has a mathematical counterpart: the framework
does not privilege one logic, and adding one is a declared institution with declared comorphisms — an
extension, not a change to the core. A vendor cannot enclose a framework whose extension mechanism is
a published translation with a satisfaction condition attached.

> **Implementation status.** Institutions are built, not aspirational: three implementations exist
> (reasoning, statistics, and a Lean proof-checking institution), two are registered on the chain, and
> the kernel carries the comorphism registry. The reasoning institution's validation gate is what
> type-checks the certificates described below.

---

## Proposed approach

Seven moves. Each is small; the guarantee comes from their combination.

**1. Content, not files.** Everything is a resource with an IRI, in immutable layers forming a
hash-linked graph. A layer's identity is a hash of its content; its position identity folds in its
parents, so altering history changes every descendant.

**2. A claim carries a proposition.** Not a string — a term in a language with a decidable notion of
"the same proposition", and a canonical encoding. This matters for move 4.

**3. Validated commits emit traces.** When an implementation validates a commit that establishes a
claim, it writes a *trace*: this claim, established this way, by this party or program, at this time.

**4. Traces admit witnesses, and witnesses cannot be written.** A *witness* is the machine-checkable
side of a trace. It is identified by three things: the grade, the IRI, and **a hash of the
proposition**. Including the proposition hash is what stops a citation from drifting: change what a
resource says, and every citation written against the old content stops resolving.

Witnesses are realised as types with **no constructors**. There is no syntax for them. The author of
a proof writes a hole:

```
derived("urn:…:claim_1",  HasActivity(msi, WRN),  ())
                                                   ↑
                            the author writes a hole here — and cannot write anything else.
                            The implementation fills it by finding a trace that admits
                            (Derived, claim_1, hash of that proposition), or refuses the commit.
```

That hole is the whole mechanism. You cannot assert the grounding; you can only ask whether it
exists.

**5. Warrants compose, and only witnesses are leaves.** A small calculus builds composite warrants —
apply a rule to a premise, offer alternatives, specialise a general claim to a case. Every leaf is a
witness. A certificate is stored and re-checkable, so a verifier redoes the check rather than
trusting that it happened.

**6. No rule introduces an implication.** This is the least obvious move and the most consequential.
The calculus has no deduction theorem: you cannot *derive* "if A then B" by assuming A and
concluding B. An implication can only enter a warrant by being **grounded** — carried by some
resource, at some grade, with a trace naming a party.

The effect is that every inferential bridge in a record is visible as a claim somebody stands behind.
A system cannot manufacture a warrant whose bridging premise nobody asserted. Goal 3 falls out of
this: to find what a record takes on authority, list its Declared groundings.

**7. Record the choices, and the failures.** Every non-deterministic step — a model's, a heuristic's,
a human's — is recorded with the authority that made it (from a closed list), the alternatives it was
chosen against, any rationale, and a replay key that covers the *context presented*, so that changing
the context is a counted miss rather than a silent reuse. Every input unit that produced no claim
gets an omission record naming why.

The record also distinguishes **vetoed** from **unvetoed** choices. Where a mechanical check rejects
unacceptable outcomes, the model only proposes and cannot introduce an error the implementation would
accept. Where every candidate is acceptable, the model's choice stands and only the audit trail
constrains it. Collapsing these two is the most likely way for an "AI provenance" record to mislead,
so disclosure of which one applies is required.

---

## Key scenarios

### Scenario 1 — A dependency that is live, not narrated

This one runs today in the reference implementation.

A paragraph of a paper is encoded. Sentence 1 establishes a measurement. A rule pinned from the
literature says that anything with that property requires another. Sentence 2 asserts a conclusion —
and the *same* conclusion also follows from sentence 1 plus the rule.

So the conclusion ends up justified twice, by two independent warrants: the document says it, and it
follows from a measurement and a published rule.

Now negate sentence 1 in the source text. It parses to a different proposition, which hashes to a
different witness key. The inferential warrant has nothing to apply the rule to, and the commit is
refused with a diagnostic naming the missing witness:

```
no admitted IsDerivedAs witness for urn:…:claim_1 with proposition …
```

Sentence 2's own claim is untouched and still commits. The document still says the conclusion; it is
simply no longer *derived*.

Nothing compared the two texts. No rule fired about negation. The dependency did the work, because
the citation was bound to the proposition rather than to the name.

> The literature rule in the demo is illustrative and invented for it — it is not a finding of the
> paper. The demo is about how a claim becomes justified, not about the biology.

### Scenario 2 — "What does this rest on?"

A reviewer wants the authority surface of a 500-claim record. They filter for Declared groundings.
Because of move 6, that filter is exhaustive: no implication reached the record any other way. The
answer is a list, typically short, each entry naming a party and a rationale.

Without move 6 the same filter returns the same list and it means nothing, because the system could
have derived bridges of its own.

### Scenario 3 — Independent re-checking

A second party receives the record and no access to the producer. They recompute every content
address, re-check every certificate against its proposition, recompute every grade from its
justification, and confirm every witness is admitted by a trace present in the record.

What this establishes is that the record is internally consistent and that every claim terminates in
a trace rather than in nothing. What it does *not* establish is that the traces are true — a verifier
cannot re-run the world.

The gain is that the trust is **localised and enumerable**. Before, every field was a place to lie.
After, the places are exactly the traces, and they can be listed.

### Scenario 4 — Honest coverage

A pipeline processes 62 units of a document and encodes 50. A record containing only the 50 reports
100% coverage by construction, and no reader can distinguish it from a pipeline that processed
everything.

Conformance requires all 62 present: 50 claims and 12 omission records, each naming a reason class —
vocabulary gap, grammar gap, unresolved selection, unresolved reference, out of scope. Those
distinctions are what make the residue actionable, since a vocabulary failure and an unresolved
choice call for entirely different responses.

---

## Considered alternatives

A note on the first three. They are not really alternatives — they operate at the **artifact level**
(what came from what, what ran, who signed it) and this operates at the **claim level** (why does this
hold, and can I check it). They compose with this specification rather than competing with it, and the
sections below say in which direction. They are listed as alternatives because they are what a
reviewer will reasonably ask about first.

### Profile W3C PROV-O

The obvious move, and the first question any reviewer will ask. There are two reasons it does not
work, and the representational one is the more fundamental.

**PROV cannot express reasoning.** RDF is open, so one can always mint classes — but the gap is not
vocabulary, and adding terms does not close it:

| Reasoning requires | PROV's nearest construct | What is missing |
|---|---|---|
| The content of a claim | `prov:Entity` | Deliberately opaque. PROV models identity and lineage, not meaning. |
| "P follows from Q by rule R" | Activity `used` Q, `generated` P, `wasAssociatedWith` an Agent that `hadPlan` R | Records that a step occurred. R is an opaque `prov:Plan`; nothing states it and nothing checks conformance to it. |
| A checkable warrant | — | PROV has no proof objects of any kind. |
| Composing warrants | Activity chaining | Chains *activities*, not *justifications*. No notion of a composite warrant being valid because its parts are. |
| Truth-preserving vs. guessed | — | `wasDerivedFrom` is deliberately general and covers both. |
| Validity | PROV-CONSTRAINTS | Constrains the provenance graph's well-formedness — ordering, uniqueness — not the logical validity of anything. |

Closing this needs two things RDF vocabulary cannot supply: content with a decidable identity, and a
checking relation. That is what [the three theories](#the-level-shift-three-theories-three-jobs) are
for.

There is also a structural mismatch at exactly our motivating case. PROV's entity/generation model is
oriented to "this artifact was produced by this process", so one proposition warranted two
*independent* ways has no natural home — one gets two entities and an `alternateOf`, losing the fact
that they are the same claim.

**And separately, everything in PROV is producer-writable.** Even the lineage PROV *can* express is
asserted by whoever writes the graph; a `wasDerivedFrom` from a producer that never ran the derivation
is a perfectly well-formed PROV statement.

**What we should do rather than dismiss it:** publish a downward mapping, so records are consumable by
existing PROV tooling. The mapping is heavily lossy and worth stating precisely — a PROV export
flattens every certificate to `wasDerivedFrom` edges, so the calculus does not survive translation at
all. That is the point rather than an embarrassment: the flattening is what makes the extra layer
legible to someone who only knows PROV. Not yet drafted.

> **Prior art to verify — this matters.** The genuine precedent for reasoning-level provenance is the
> **Proof Markup Language** (PML), from the Inference Web project (McGuinness, Pinheiro da Silva,
> mid-2000s), whose justification layer modelled inference steps with antecedents and named inference
> rules. Our recollection is that PML fed into the W3C Provenance Incubator Group, and that the
> resulting PROV work deliberately narrowed scope from justification toward lineage. If that history
> holds it is a far better argument than "PROV did not consider this": the W3C looked at
> reasoning-level provenance and scoped it out, and the question for this group is whether twenty
> intervening years of proof assistants and machine-generated claims change the calculation.
> **Unverified — check before circulating.**

### Signed attestations (in-toto / SLSA)

The closest neighbour, and the comparison most worth getting right. SLSA's provenance thesis is
nearly ours: an attestation about how an artifact was built, produced by the build platform rather
than asserted by the publisher.

The difference is where the non-forgeability comes from. SLSA gets it by **trusting the builder** —
the guarantee is "this attestation came from a build platform you have decided to trust", carried by
a signature. Ours comes from the **record's own structure**: the witness types have no constructors,
and a verifier confirms that by reading the record.

These are complementary, not competing. Signing answers *who*; this answers *was it computed*. The
specification explicitly leaves the first to a layer above.

### Verifiable Credentials

Issuer-signed claims with a mature revocation and key-discovery story. Same analysis as above: VCs
establish authenticity, not computation. A strong candidate for the attribution layer the
specification defers.

### Log every model call

Store prompts and responses; let auditors read them.

Logs are producer-writable, so they inherit the original problem. They are also enormous, they do not
compose — a log of ten calls does not tell you what claim 7 depends on — and they leak far more than a
decision record does. Recording the decision, its alternatives, and a replay key is smaller, more
useful, and less disclosive.

### Trusted execution / remote attestation

Hardware attestation proves that a particular binary ran in a particular enclave. It does not say what
the binary concluded or why, and it requires trusting a vendor's root of trust — which is the
single-vendor enclosure the group's mission specifically disclaims.

### Require formal proof everywhere

Admit only machine-checked claims. This yields a system with almost nothing in it, and it is why four
grades exist rather than one. The record is useful before any formal proof exists, and it always shows
where the boundary between checked and unchecked lies. Verification deepens over time on the
conclusions that warrant the cost.

### Confidence scores plus human review

The current de-facto practice. Confidence does not compose across inference steps, is not comparable
between components, and says nothing about dependencies. Review capacity does not grow with generation
volume. The specification permits confidence values and forbids treating them as grades.

---

## Security and privacy considerations

Summarised from [§11 of the specification](ai-computed-provenance-1.0.md#11-security-and-privacy-considerations).

**What is addressed:** silent substitution of a proposition under a stable citation; undetected
alteration of history; grades recorded without support; asserted-but-not-performed computation; and
coverage inflation by dropping unprocessable input.

**What is not:** a producer that claims conformance and does not conform will emit records a verifier
finds internally consistent, because the inconsistency is not in the record. This is precisely the gap
attribution would narrow, and its absence leaves open. Also unaddressed: implementation defects,
correct records of wrong conclusions, and availability.

**Disclosure is a real cost.** A conforming record carries source spans, extracted text, model
rationales, and the alternatives considered at every decision. For clinical or embargoed corpora this
may be unshareable even when the conclusions are shareable. Redaction is permitted, must be declared,
and requires recomputing addresses over the redacted content — an address that does not match the
content beside it is worse than no address.

**Content addressing reveals content equality.** Two parties holding the same data compute the same
address. That lets laboratories confirm they hold the same dataset without disclosing it, and lets an
observer confirm a guess about data never disclosed.

---

## Open questions

1. **Cross-binding agreement.** The specification states four properties a proposition language must
   have. Whether they suffice to make two independently built implementations agree on what a
   proposition *means* is unsettled. They suffice for verification within one binding, which is what
   conformance requires today.

2. **The PROV mapping.** Should be written. Not yet drafted.

3. **Does attribution layer, or integrate?** The specification requires that any signing layer bind to
   content addresses rather than to a serialization. Whether that is enough, and which existing
   attribution model to adopt, is open.

4. **Registrations.** The binding currently uses an unregistered media type and a CBOR tag from IANA's
   unassigned range. Both need resolving before anything is standardised.

5. **Identifiers.** The reference implementation's IRIs are vendor-namespaced. Whether the group mints
   its own, adopts these, or defines a registry is undecided. Nothing in the abstract core depends on
   the answer.

6. **Grade propagation is specified but unimplemented.** The rule that `Verified` survives composition
   only when every sub-warrant is `Verified` — including the inference rule itself — is normative in
   the specification and does not yet exist in the reference implementation, which projects grades from
   the landing warrant instead. Flagged in place rather than quietly omitted.

7. **Selective disclosure.** Proving that a redacted record is a faithful redaction of a specific
   original, rather than merely internally consistent, is not specified. Hash-tree constructions are the
   obvious direction.

8. **What the third deliverable is.** The group's charter promises "architectural guidelines ensuring
   the protocol remains inspectable and protected from commercial capture." W3C's actual anti-capture
   mechanisms are procedural — the Royalty-Free patent policy, and for a Community Group the Final
   Specification Agreement — rather than documentary. A document asserting neutrality does not create
   it. This is likely two deliverables that were named as one, and separating them early matters because
   only one of them has deadlines.

---

## References

**The specification.**

- [AI Computed Provenance 1.0](ai-computed-provenance-1.0.md) — the specification this explains.

**Foundations.**

- Martin-Löf, P. (1984). *Intuitionistic Type Theory.* Bibliopolis. And Coquand, T. and Huet, G.
  (1988), "The Calculus of Constructions", *Information and Computation* 76(2–3). The
  propositions-as-types substrate; the binding's type theory is a fragment of the Calculus of
  Inductive Constructions, the same family underlying Rocq and Lean.
- Artemov, S. (2008). "The Logic of Justification", *Review of Symbolic Logic* 1(4). And Artemov, S.
  and Fitting, M. (2020), *Justification Logic: Reasoning with Reasons*, Cambridge University Press.
  The warrant calculus is a fragment of the Logic of Proofs.
- Goguen, J. and Burstall, R. (1992). "Institutions: Abstract Model Theory for Specification and
  Programming", *Journal of the ACM* 39(1). Signatures, sentences, models, satisfaction, and the
  satisfaction condition; comorphisms as truth-preserving translation between logical systems.

**Prior art in the alternatives section.**

- W3C PROV (PROV-DM, PROV-O, PROV-CONSTRAINTS); Verifiable Credentials Data Model; C2PA; in-toto and
  SLSA; RO-Crate.
- McGuinness, D. and Pinheiro da Silva, P. — the Proof Markup Language and the Inference Web project.
  The closest precedent for reasoning-level provenance, and the one whose relationship to the W3C
  provenance work most needs establishing.

> **Unverified.** Every characterisation in the alternatives section — W3C PROV, the Verifiable
> Credentials Data Model, C2PA, in-toto/SLSA, RO-Crate, and the PML history — is written from the
> editors' understanding and has **not** been checked against primary sources or against those
> specifications' current state. Citation details above are approximate where publication data was not
> confirmed. All of it must be verified before this document is circulated.
