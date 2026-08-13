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

Today you get a provenance record: this was derived from that, by this model, at this time. Every
field in that record was written by the system that produced the conclusion. If the system is wrong,
the record is wrong in exactly the same way, and it will look just as complete. If the system is a
proprietary service, you have no recourse but to trust it.

This is not a quality problem that better engineering fixes. It is a structural property: when every
field is writable by the producer, the record's value equals the producer's trustworthiness, and no
amount of detail changes that.

This specification proposes a different arrangement. Some statements in the record are **not
writable at all**. They exist only because an implementation performed and checked a specific piece
of work, and there is no syntax, API, or configuration through which anyone can supply one. Claims
ground out in those statements. A third party re-checks them by recomputing hashes and looking for
the work that admits them — without asking the producer anything.

We call the distinction **Computed ≠ Asserted**.

---

## The user-facing problem

Three people are hurt today, concretely.

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
5. **Not a replacement for W3C PROV.** PROV describes provenance. This constrains who may write it.
   See [alternatives](#considered-alternatives).
6. **Not prescriptive about AI.** How a model is invoked, prompted, or chosen is out of scope; only
   what must be *recorded* about its choices is in scope.
7. **Not defeasible reasoning.** Belief revision is a structural marker, not a non-monotonic logic.

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

### Profile W3C PROV-O

The obvious move, and the first question any reviewer will ask.

PROV gives a mature, widely-understood model of entities, activities and agents, and `wasDerivedFrom`
expresses most of the graph shape we need. What it does not have is a place for the distinction this
specification is about: in PROV, `wasDerivedFrom` is an *assertion*, and a producer that never ran the
derivation can write it. PROV describes provenance; it does not constrain who may write which parts.

We could profile PROV and add a constraint layer on top. But the constraint layer is the entire
contribution, and PROV's model has no seam to attach it to — there is nothing in PROV that is
admitted by a validator rather than asserted by an author.

**What we should do instead of dismissing it:** publish a mapping, so records can be consumed by
existing PROV tooling. Grades map onto PROV agents and activities readily. The mapping is lossy in
one direction — PROV cannot express non-forgeability — and stating exactly where it loses information
is a useful contribution in itself. This is not yet drafted.

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

- [AI Computed Provenance 1.0](ai-computed-provenance-1.0.md) — the specification this explains.
- Artemov, S. and Fitting, M. (2020). *Justification Logic: Reasoning with Reasons.* Cambridge
  University Press. The calculus of moves 5 and 6 is a fragment of the Logic of Proofs.

> **Unverified.** Characterisations of W3C PROV, the Verifiable Credentials Data Model, C2PA, in-toto
> and SLSA in the alternatives section are written from the editors' understanding and have **not**
> been checked against primary sources or against those specifications' current state. They must be
> verified before this document is circulated.
