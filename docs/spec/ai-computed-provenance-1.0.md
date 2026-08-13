# AI Computed Provenance 1.0

**Core data model, justification calculus, and conformance**

Draft — 12 August 2026

---

## Status of this document

This is an **unofficial editor's draft**. It is not a W3C publication, it has not been reviewed or
endorsed by any W3C group, and it carries no standing of any kind. It is written as input to the
proposed **AI Computed Provenance Community Group** (proposed 11 August 2026), whose first committed
deliverable is *"a reference specification for capturing cryptographic provenance in AI-assisted
scientific and technical workflows."*

Nothing here has been agreed by that group. Sections marked **AT RISK** name decisions the editors
believe are unsettled. Sections marked **UNVERIFIED** contain claims about external specifications or
about the state of other standards work that the editors have not yet confirmed against primary
sources; they must not be relied on until that check is done.

The normative content of this draft is derived from a working implementation (see
[Appendix A](#appendix-a-the-eigon-eigentt-binding-normative)). Where this draft states a requirement
that no implementation yet satisfies, it says so explicitly at the point of the requirement.

---

## 1. Introduction

### 1.1 The problem

An AI-assisted scientific workflow produces conclusions. A reader who wants to know whether a
conclusion is sound has to ask three questions: what does it depend on, how was each dependency
established, and can any of that be checked without trusting the system that produced it.

Systems today answer the first question partially, the second in prose, and the third not at all.
Provenance metadata is written by the same component that produced the conclusion, so it records
what that component chose to record. A field saying `derived_by: "model-x"` is an assertion about a
computation, not evidence that the computation happened. When the producer is a proprietary service,
the reader's only recourse is to trust it.

This is not a gap in diligence. It is a structural property of provenance formats in which every
field is writable by the producer.

### 1.2 Computed ≠ Asserted

This specification is organised around one distinction:

> A record may **assert** that something was computed, or it may be **structurally incapable of
> stating that something was computed unless it was.**

Only the second is worth anything to a reader who does not trust the producer. Achieving it requires
that some part of the record not be author-writable — that there exist a class of statements which
an implementation admits *only* as a consequence of having performed and checked the corresponding
work, and for which no authoring syntax exists at all.

This specification calls those statements **witnesses** ([§6](#6-traces-and-witnesses)), and the
requirement that they be unforgeable ([ACP-6-1](#acp-6-1)) is the requirement every other part of the
document serves. A conforming implementation may be closed-source and still produce records whose
grounding claims a third party can independently re-check, because re-checking a witness does not
consult the producer — it recomputes a hash and looks for an admitting trace.

### 1.3 What this specification defines

1. An **abstract data model** for provenance records: resources, layers, and the immutable chain they
   form ([§3](#3-abstract-data-model)).
2. **Canonicalization and content addressing** — the requirements that make a record's identity a
   function of its content, and one concrete hash profile ([§4](#4-canonicalization-and-content-addressing)).
3. The **epistemic grades** — Declared, Observed, Derived, Verified — as a computed projection rather
   than an author-supplied label ([§5](#5-epistemic-grades)).
4. **Traces and witnesses**, including the non-forgeability requirement and the binding of a witness
   to the exact proposition it attests ([§6](#6-traces-and-witnesses)).
5. A **justification calculus** in which warrants compose and grades propagate
   ([§7](#7-the-justification-calculus)).
6. Requirements on the **proposition language** a binding must supply ([§8](#8-the-proposition-language)).
7. **AI decision records** — how a non-deterministic choice made by a model is recorded, bounded, and
   replayed, and the requirement that a record account for input it failed to process
   ([§9](#9-ai-decision-records)).
8. **Conformance classes** and the assertion index ([§10](#10-conformance)).

### 1.4 What this specification does not define

- **Authenticity.** This specification covers integrity and reproducibility. It does not say who
  produced a record. See [§12](#12-out-of-scope-authenticity-and-attribution), which states the
  boundary and its consequences rather than leaving it implicit.
- **A model of correctness.** A conforming record can faithfully document a wrong conclusion. The
  grades report how a claim was established, not whether it is true. Only the `Verified`
  grade carries a truth guarantee, and only relative to the proof system that produced it.
- **Which vocabulary a domain should use.** The vocabulary for a scientific domain is authored, not
  standardised here.
- **How models are invoked**, prompted, or selected. [§9](#9-ai-decision-records) constrains what must
  be *recorded* about a model's choice, not how the choice is made.

### 1.5 Ontology-first

This specification defines its vocabulary as **ontology definitions** — classes, properties, and
inductive types, each an addressable resource in the data model of [§3](#3-abstract-data-model) — and
not as a syntax.

The consequence is that the specification's own vocabulary is expressed in the data model it
specifies, and is therefore inspectable, versionable, and extensible by the same mechanisms as any
other content. An implementation agrees with another implementation by agreeing on resources at IRIs,
not by parsing the same files.

Concrete authoring syntaxes exist and are out of scope. Where this document shows a definition it
shows the resource, not the surface form someone typed to produce it.

---

## 2. Terminology and conventions

### 2.1 Normative language

The key words MUST, MUST NOT, REQUIRED, SHALL, SHALL NOT, SHOULD, SHOULD NOT, RECOMMENDED, MAY, and
OPTIONAL in this document are to be interpreted as described in BCP 14 [RFC 2119] [RFC 8174] when,
and only when, they appear in all capitals.

Text in sections or paragraphs marked *(informative)* is non-normative. Examples, notes, and
rationale are non-normative.

### 2.2 Assertion identifiers

Every normative statement carries a stable identifier of the form **`ACP-<section>-<n>`**, where
`<section>` is the number of the section it appears in and `<n>` is its position within that section.

Identifiers are stable across revisions. A requirement that is withdrawn keeps its identifier, marked
withdrawn; a new requirement takes the next unused `<n>` in its section rather than renumbering its
neighbours. The conformance test suite cites these identifiers, so renumbering would silently
invalidate test provenance — which is the failure mode this scheme exists to prevent.

### 2.3 Terms

**resource**
: An identified unit of content: an IRI together with a set of typed properties. The unit of
  identity, reference, and validation.

**property**
: A named, typed attribute a resource may carry. Properties are themselves resources; a property
  resource declares the type of its values and the classes it may appear on.

**class**
: A type whose instances are resources. A class resource declares which properties its instances
  must and should carry, and which classes it specialises. Class membership is stated by a resource's
  `is_a` property and MAY be multiple.

**inductive type**
: A type defined by a finite, closed list of constructors. Used for the term languages this
  specification defines — the justification terms of [§7](#7-the-justification-calculus) and the
  propositions of [§8](#8-the-proposition-language). A constructor is a resource; the constructor list
  of a type is therefore an inspectable part of the record rather than a property of an
  implementation.

**layer**
: An immutable set of resources committed atomically, together with references to zero or more parent
  layers. Readers coming from version control or from dataset-versioning systems may read *layer* as
  *revision*, *commit*, or *generation*; the term is used here because layers stack and later layers
  shadow earlier ones.

**chain**
: The directed acyclic graph of layers reachable from a given layer through parent references. The
  chain is the scope in which an IRI is resolved.

**proposition**
: A statement, expressed as a term in the proposition language of [§8](#8-the-proposition-language),
  about which a claim can be made. Propositions are content, not prose: two propositions are the same
  proposition when the language's equivalence relation says so, not when their renderings match.

**claim**
: A resource that carries a proposition and is the subject of a grade.

**grade**
: One of the four epistemic categories of [§5](#5-epistemic-grades): `Declared`,
  `Observed`, `Derived`, `Verified`. A grade describes how a claim was established.

**trace**
: A resource recording that a specific claim was established in a specific way, at a specific time,
  by a named agent, program, or source. The audit-facing artifact.

**witness**
: A checkable attestation that a named resource carries a named proposition under a named grade. The
  machine-facing projection of a trace. Witnesses are not author-writable
  ([ACP-6-1](#acp-6-1)).

**justification term**
: The structure of a warrant: which evidence it grounds in and how those groundings compose. A term
  by itself makes no claim; it is paired with a proposition.

**certificate**
: A checkable object establishing that a given justification term justifies a given proposition.

**authority**
: The named source of a choice that was not determined by the input — a human pin, a model, a replayed
  recording, or a deterministic rule. See [§9](#9-ai-decision-records).

**omission record**
: A record that a unit of input produced no claim, together with the reason class. See
  [ACP-9-6](#acp-9-6).

**producer**
: An implementation that emits provenance records.

**verifier**
: An implementation that checks provenance records without trusting the producer.

### 2.4 Notational conventions *(informative)*

Resources are shown in the JSON serialization of
[Appendix A](#appendix-a-the-eigon-eigentt-binding-normative), abbreviated with the prefixes below.
Prefixes are a presentation device of this document only; the data model has no prefix mechanism and
IRIs always appear in full.

| Prefix | IRI base |
|---|---|
| `core:` | `urn:eigenius:core:` |
| `reflection:` | `urn:eigenius:reflection:` |
| `reasoning:` | `urn:eigenius:reasoning:` |
| `witness:` | `urn:eigenius:reasoning:ChainWitness:` |
| `enc:` | `urn:eigenius:encoding:` |

> **Note.** These bases are the working implementation's. Whether the specification adopts them,
> mints its own, or defines a registry is **AT RISK** and is a question for the group. Nothing in the
> abstract core depends on the choice.

---

## 3. Abstract data model

### 3.1 Resources

**ACP-3-1.** A **top-level resource** MUST be identified by an IRI. That IRI is the resource's
identity: two top-level resources with the same IRI in the same chain scope are the same resource.

**ACP-3-2.** A resource's content MUST be a set of **properties**, each keyed by an IRI. A property
key MUST NOT be a bare name; names are not globally unique and this specification's records are
merged across authorities that never coordinated.

**ACP-3-3.** An **embedded resource** MAY appear as a property value. An embedded resource MUST NOT
carry an identity of its own and MUST be addressable only through the top-level resource that
contains it.

**ACP-3-4.** A resource MUST declare its class membership through the `is_a` property. Membership MAY
be multiple; a resource MAY instantiate several classes at once.

**ACP-3-5.** An absent property and a property present with a null value MUST be treated as the same
condition. A binding MUST NOT distinguish them, because a canonical form that admitted both would
give one resource two content addresses.

### 3.2 The metamodel: classes, properties, inductive types

This specification's own vocabulary is expressed as resources ([§1.5](#15-ontology-first)). Three
metamodel classes are needed to do so.

**ACP-3-6.** A **class** resource MUST declare the properties its instances are required to carry and
SHOULD declare those they are recommended to carry. A class MAY declare one or more parent classes;
an instance of a class is an instance of every ancestor, and inherits every ancestor's requirements.

**ACP-3-7.** A **property** resource MUST declare the type of its values. It MAY additionally
constrain values to instances of named classes, restrict them to an explicit set of resources, and
name the classes on which the property may appear.

**ACP-3-8.** A property that restricts its values to an explicit set of resources defines a **closed
enumeration**. An implementation MUST reject a value outside that set. Extending a closed enumeration
MUST be an edit to the property definition — that is, a versioned change to a resource in the record —
and MUST NOT be achievable by minting a new value at use site.

> *(informative)* ACP-3-8 is load-bearing for [§9](#9-ai-decision-records). The authorities that may
> make a non-deterministic choice are a closed enumeration precisely so that a new kind of
> decision-maker cannot appear in records without a visible change to the vocabulary.

**ACP-3-9.** An **inductive type** resource MUST declare a finite, ordered list of **constructors**,
each declaring an ordered list of argument types. The constructor list is closed: a value of the type
MUST be built by one of the declared constructors.

**ACP-3-10.** Because constructor lists are resources, an implementation MUST be able to enumerate the
constructors of any term language this specification defines by reading the record, without consulting
implementation-internal tables.

### 3.3 Layers and chains

**ACP-3-11.** A **layer** MUST be an immutable set of resources. Once a layer is admitted, an
implementation MUST NOT alter its content.

**ACP-3-12.** A layer MUST record references to its parent layers. A layer MAY have zero parents (a
root), one parent, or several (a merge).

**ACP-3-13.** The parent relation MUST be acyclic.

**ACP-3-14.** An IRI MUST be resolved against a layer by searching that layer and then its ancestors.
Where several layers in the chain define the same IRI, the definition nearest the starting layer MUST
win, and the shadowed definitions MUST remain present and reachable.

**ACP-3-15.** A layer MAY **tombstone** an IRI, making it unresolvable from that layer onward without
removing the resource that defined it. A tombstone MUST be part of the layer's content for the
purposes of [§4](#4-canonicalization-and-content-addressing): two layers with identical resources and
different tombstones present different chains to a reader and MUST NOT share a content address.

### 3.4 Validation and admission

**ACP-3-16.** An implementation MUST validate a layer before admitting it. Validation MUST at minimum
confirm that every resource satisfies the required properties of every class it claims, that every
property value satisfies its declared type and constraints, and that every reference resolves in the
chain the layer is being admitted onto.

**ACP-3-17.** A layer that fails validation MUST NOT be admitted, and MUST NOT emit witnesses
([§6](#6-traces-and-witnesses)). Partial admission of a layer MUST NOT occur.

> *(informative)* ACP-3-17 is what connects validation to the guarantee in §6. A witness exists
> because a commit was validated; if a failed commit could emit witnesses, the guarantee would be
> vacuous.

---

## 4. Canonicalization and content addressing

### 4.1 Requirements

**ACP-4-1.** A binding MUST define a **canonical form**: a total function from a resource to a byte
sequence. Two resources equal in the data model MUST canonicalize to identical bytes. Two resources
that differ MUST canonicalize to different bytes.

**ACP-4-2.** A binding MUST define a **content address**: the output of a collision-resistant
cryptographic hash over canonical bytes. The content address of a layer MUST be a function of that
layer's content alone — its resources and its tombstones — and MUST NOT depend on the layer's position
in a chain, the time of its creation, or its author.

**ACP-4-3.** A binding MUST define a **position address** for a layer, binding its content address
together with the position addresses of its parents. The position address is what makes history
tamper-evident: altering any ancestor changes every descendant's position address.

**ACP-4-4.** The position address MUST be a function of the content address and the *set* of parent
position addresses. It MUST NOT depend on the order in which parents are presented, so that a merge of
two layers has one identity regardless of which was named first.

**ACP-4-5.** Distinct hash constructions defined by a binding MUST use distinct domain separators, and
each construction MUST unambiguously delimit its inputs. Concatenating variable-length inputs without
delimitation MUST NOT occur.

> *(informative)* ACP-4-5 guards against a construction in which two different input sequences produce
> the same byte string before hashing. The hash function's collision resistance is the real guarantee;
> delimitation removes the class of collisions an implementation would otherwise create for itself.

**ACP-4-6.** A binding MUST specify its hash function by name and MUST state the consequences of
changing it. A record MUST carry enough information for a verifier to know which construction was used.

**ACP-4-7.** A verifier MUST be able to recompute every address in a record from the record's own
content ([§10.1.2](#1012-record-verifier)). A binding whose addresses cannot be recomputed without
implementation-private state does not satisfy this specification.

### 4.2 Hash profile

The abstract core fixes no hash function and no serialization; a binding does. The concrete
constructions of this document's binding — canonical JSON per [RFC 8785], the `content:v1:` and
`position:v1:` SHA-256 constructions, and the proposition-hash construction of
[§6](#6-traces-and-witnesses) — are specified byte-for-byte in
[Appendix A](#appendix-a-the-eigon-eigentt-binding-normative) and are normative for implementations of
that binding.

---

## 5. Epistemic grades

### 5.1 The four grades

This specification defines four grades. A grade answers *how a claim was established*, not whether it
is true.

**Declared**
: Someone asserted it. The record vouches that the declaration is well-formed and names who made it.
  It does not vouch for its content. Axioms, conventions, policy thresholds, and any bridge between
  two vocabularies are Declared.

**Observed**
: It was recorded from outside the system, and the record names the source.

**Derived**
: A program produced it from other content, and the record identifies the program and its inputs.

**Verified**
: A machine-checked proof establishes it, relative to a named proof system.

**ACP-5-1.** An implementation MUST support all four grades. A record MUST NOT use a grade vocabulary
of its own in place of these.

**ACP-5-2.** `Verified` MUST be a specialization of `Derived`: anything verified is also derived, and
a `Verified` claim MUST satisfy any obligation that calls for a `Derived` one. No other specialization
relation holds among the four; `Declared` and `Observed` are not ordered with respect to each other or
to the rest.

> *(informative)* The four grades are therefore not a chain of increasing strength, and this
> specification deliberately does not define one. `Observed` is not "better than" `Declared`; they
> answer different questions. Only the `Verified`/`Derived` edge is a genuine specialization.

### 5.2 Per-grade obligations

**ACP-5-3.** A `Declared` claim MUST name the party that declared it, and SHOULD carry a rationale and
a timestamp.

**ACP-5-4.** An `Observed` claim MUST name its source, and SHOULD carry the time of observation.

**ACP-5-5.** A `Derived` claim SHOULD carry a reference to the record of the computation that produced
it.

**ACP-5-6.** A `Verified` claim MUST carry both a reference to the derivation and a reference to the
verification — the proof system and the proof term or its address.

### 5.3 Grades are computed, not asserted

**ACP-5-7.** A grade recorded on a claim MUST NOT be treated as evidence. An implementation MUST NOT
rely on a claim's grade in a justification unless a witness for that claim at that grade is admitted
([§6](#6-traces-and-witnesses)).

> *(informative)* ACP-5-7 is why ACP-5-5 is a SHOULD rather than a MUST. The `derivation` reference is
> a convenience for a human reader; it is not what makes the claim citable. What makes it citable is
> the witness, and the witness exists only because a validated commit emitted a trace. A record can
> therefore state a grade it cannot support — and a conforming verifier will find that it cannot
> support it, which is the intended behaviour.

**ACP-5-8.** Where a claim carries an explicit justification ([§7](#7-the-justification-calculus)),
its grade MUST equal the grade computed from that justification by
[ACP-7-10](#74-grade-propagation). An implementation MUST reject a claim whose recorded grade differs
from the computed one.

---

## 6. Traces and witnesses

### 6.1 Two projections of one event

When an implementation validates a commit that establishes a claim, it produces two things from that
one event:

- a **trace** — a resource, part of the record, readable by a person, saying that this claim was
  established this way, by this party or program, at this time;
- a **witness** — a checkable attestation, consumed by the calculus of
  [§7](#7-the-justification-calculus), saying that this resource carries this proposition at this
  grade.

The trace is the audit artifact. The witness is what a certificate consumes. They are not
independent: the witness exists because the trace was emitted by a validated commit.

### 6.2 Non-forgeability

<a id="acp-6-1"></a>
**ACP-6-1 (witness non-forgeability).** An implementation MUST NOT provide any means by which
author-supplied content can introduce a witness. An implementation MUST admit a witness only as a
consequence of a commit that it validated and that emitted the corresponding trace.

**ACP-6-2.** The record format MUST NOT define a syntax for witnesses, and the implementation MUST NOT
expose an API, configuration setting, import path, or debug facility through which a caller can supply
one. ACP-6-1 is a requirement on the implementation's whole surface, not on its file format.

**ACP-6-3.** A binding MUST realise witnesses as a type with no constructors, or by an equivalent
mechanism that makes an inhabitant unconstructable by an author. The absence of constructors MUST be
readable from the record ([ACP-3-10](#32-the-metamodel-classes-properties-inductive-types)), so that a
verifier can confirm from the record itself that no author-supplied inhabitant was possible.

> *(informative)* ACP-6-3 is what lets a verifier check the guarantee rather than assume it. The
> witness types are ordinary inductive-type resources with an empty constructor list; anyone reading
> the record can see that the list is empty.

### 6.3 Witness identity

**ACP-6-4.** A witness MUST be identified by exactly three components: the **grade** it attests, the
**IRI** of the resource it attests about, and a **binding of the proposition** that resource carries.

**ACP-6-5.** The proposition binding MUST be a collision-resistant hash of the proposition's canonical
encoding, not the proposition's rendering and not a reference to it.

> *(informative)* ACP-6-4 and ACP-6-5 together give the property that makes citation safe: a citation
> names an IRI *and* the proposition that IRI is being cited for. Changing what a resource says
> changes its witness identity, so a citation written against the old proposition no longer resolves.
> A record cannot silently acquire a different meaning under a stable reference.

**ACP-6-6.** Propositions that differ only by the names of bound variables MUST produce the same
binding. A binding MUST specify a canonicalization of bound-variable names and MUST apply it before
hashing.

> *(informative)* Without ACP-6-6, a proposition written with human-chosen variable names would never
> match the same proposition after an implementation renamed binders internally, and every citation
> would fail for a reason unrelated to its content.

### 6.4 Which proposition a witness attests

**ACP-6-7.** The proposition a witness attests MUST be determined by the resource the trace targets,
by the following rules applied in order:

1. if the resource carries an explicit canonical-proposition property, that property's value;
2. otherwise, the atomic proposition asserting the resource's own IRI.

**ACP-6-8.** A resource MUST carry at most one canonical proposition. A party needing to ground
several propositions MUST declare several resources.

**ACP-6-9.** A binding MAY define additional resolution rules, ahead of rule 1, for resources whose
proposition is computed rather than declared — for example the statement a machine-checked proof
establishes. Such a rule MUST be deterministic and MUST be recomputable by a verifier.

**ACP-6-10.** The implementation MUST derive the proposition binding from the proposition as
*interpreted*, not as *written*, wherever the two can differ. Where a binding permits definitions that
expand, an implementation MUST expand them before hashing on both the emitting and the consuming side.

> *(informative)* ACP-6-10 addresses a concrete failure. If an author writes a folded name and the
> checker sees the expanded body, hashing the stored form on one side and the interpreted form on the
> other yields two different bindings for one proposition, and every citation misses. The requirement
> is that both ends hash the same term.

### 6.5 Trace classes

**ACP-6-11.** A binding MUST define one trace class per grade, and MUST specify for each the
properties it requires. A trace MUST identify the resource it attests about, and MUST record when it
was emitted.

**ACP-6-12.** A trace MUST NOT be emitted by an author. A trace present in a record that was not
emitted by a validated commit MUST NOT cause a witness to be admitted; an implementation MUST NOT admit
a witness merely because a well-formed trace resource is present.

> **Implementation status.** In the reference implementation the `Declared`, `Observed`, and `Derived`
> trace routes are implemented. The `Verified` route through a verification trace is **not**: the
> `Verified` grade is currently admitted only through a self-attesting route
> ([§6.6](#66-self-attesting-witnesses)), pending the translation of external proof statements into
> the proposition language. ACP-6-11 leads the implementation on this point.

### 6.6 Self-attesting witnesses

**ACP-6-13.** A binding MAY define resource classes that attest themselves — that is, whose validated
commit admits a witness keyed on the committing resource's own IRI, with no separate trace. A binding
that does so MUST enumerate those classes, and each MUST derive its proposition by
[ACP-6-7](#64-which-proposition-a-witness-attests) or [ACP-6-9](#64-which-proposition-a-witness-attests).

### 6.7 Admission scope

**ACP-6-14.** A witness MUST be admitted if any layer in the chain reachable from the current layer
admits it. An implementation MUST search the chain, and MUST NOT restrict admission to the layer in
which the citing content appears.

**ACP-6-15.** A witness lookup that fails MUST produce a diagnostic naming the grade, the IRI, and the
proposition that was sought. An implementation MUST NOT report a failed lookup as a generic type error.

> *(informative)* ACP-6-15 exists because the failure mode it prevents is the common one. "No admitted
> `IsDerivedAs` witness for `X` with proposition `P`" tells an author what to fix; a bare mismatch does
> not, and the two failures — no such resource, versus that resource says something else — are
> indistinguishable without it.

**ACP-6-16.** A witness at a specialized grade MUST satisfy an obligation stated at the grade it
specializes. In particular, a `Verified` witness MUST satisfy a `Derived` obligation
([ACP-5-2](#51-the-four-grades)).

---

## 7. The justification calculus

### 7.1 Justification terms

A **justification term** records the shape of a warrant: what it grounds in, and how the groundings
compose. It carries no propositional content on its own.

**ACP-7-1.** A binding MUST define the justification term language as an inductive type resource
([ACP-3-9](#32-the-metamodel-classes-properties-inductive-types)) with a closed constructor list. It
MUST include a grounding constructor for each of the four grades, each taking the IRI of the resource
it grounds in.

**ACP-7-2.** The constructor list MUST be closed against extension at use site. Adding a constructor
MUST be a versioned change to the type's resource.

**ACP-7-3.** A justification term MUST be first order: no constructor argument may be a proposition, a
certificate, or a function. A term is an audit object and MUST remain comparable and storable without
evaluating anything.

> *(informative)* ACP-7-3 has a visible consequence in the binding. Specializing a universally
> quantified warrant needs the instance on the proof side, but the term records only an opaque tag
> naming the instantiation. The tag is for the auditor; the instance lives in the certificate.

### 7.2 Certificates

**ACP-7-4.** A binding MUST define a **certificate** relation between a justification term and a
proposition, and MUST define it as an inductive type resource whose constructors are its inference
rules — so that the rules of the calculus are readable from the record rather than being a property of
the implementation.

**ACP-7-5.** For each grade, the calculus MUST include a rule that consumes a witness at that grade and
produces a certificate for the corresponding grounding term at the witnessed proposition. These rules
MUST be the only way a grounding term acquires a certificate.

**ACP-7-6 (application).** The calculus MUST include a rule that, from a certificate that one term
justifies an implication and a certificate that another justifies its antecedent, produces a
certificate that the application of the two justifies the consequent.

**ACP-7-7 (alternatives).** The calculus MUST include rules by which a certificate for either of two
terms yields a certificate for their combination at the same proposition. It MUST NOT include an
elimination rule for that combination: combining alternatives is a packaging operation, not a
decomposable structure.

**ACP-7-8 (specialization).** The calculus MUST include a rule that eliminates a universal quantifier
in a justified proposition, producing a certificate for the proposition at a specific instance.

### 7.3 No implication introduction

<a id="acp-7-no-impl-intro"></a>
**ACP-7-9.** The calculus MUST NOT include any rule whose conclusion is a certificate at an
implication. Every rule's conclusion MUST be at a proposition that is not an implication introduced by
that rule.

> *(informative)* This is the most consequential structural decision in the calculus, and it is a
> deliberate weakening. There is no deduction theorem: an implication cannot be *derived* by assuming
> its antecedent and concluding its consequent. An implication can only enter a warrant by being
> **grounded** — that is, by being carried by some resource, at some grade, attested by some trace.
>
> For provenance this is exactly the desired property. Every bridge between two vocabularies, every
> domain rule of the form "if this measurement, then that conclusion", is visible in the record as a
> claim someone or something stands behind, with a grade and a trace naming them. A system that could
> derive its own bridging implications could manufacture warrants for conclusions no party ever
> asserted, and the audit trail would not show it. Counting the Declared implications in a record is
> therefore a meaningful measure of what the record takes on authority.

### 7.4 Grade propagation

**ACP-7-10.** The grade of a claim carrying a justification MUST be computed from the justification
term by structural recursion:

| Term | Computed grade |
|---|---|
| a grounding constructor | the grade that constructor grounds in |
| a composition of sub-terms | `Verified` if **every** sub-term computes to `Verified`; otherwise `Derived` |
| a specialization of a sub-term | the grade of that sub-term |

**ACP-7-11.** `Verified` MUST NOT propagate through a composition in which any sub-term — including
the term justifying the inference rule itself, not only the terms justifying premises — computes to a
grade other than `Verified`.

> *(informative)* A verified premise combined by a merely declared inference rule yields a `Derived`
> conclusion. This is the rule that keeps `Verified` meaning what [§5](#5-epistemic-grades) says it
> means: a machine-checked proof of the whole claim, not a machine-checked proof of one of its parts.

> **Implementation status.** ACP-7-10 and ACP-7-11 **lead the reference implementation.** The
> reference implementation projects a claim's grade from the warrant recorded at landing time rather
> than by recursion over the justification term, so a composed justification's grade is not currently
> computed by these rules. The witness-level part of ACP-7-11 is implemented: a `Verified` witness
> satisfies a `Derived` obligation ([ACP-6-16](#67-admission-scope)). The recursive projection is
> specified here because [ACP-5-8](#53-grades-are-computed-not-asserted) depends on it and because a
> verifier cannot otherwise check a recorded grade.

### 7.5 Independent justifications

**ACP-7-12.** A record MUST be able to carry more than one justification for the same proposition, and
those justifications MUST be independent: invalidating one MUST NOT invalidate another that does not
depend on it.

> *(informative)* This is what lets a record distinguish "the document says so" from "it also follows
> from a measurement and a published rule". Removing the measurement removes the second warrant and
> leaves the first standing, and the record shows which.

---

## 8. The proposition language

### 8.1 What a binding must supply

**ACP-8-1.** A binding MUST specify a **proposition language**: the set of well-formed propositions
about which claims may be made.

**ACP-8-2.** The language MUST have a **canonical encoding** into the data model of
[§3](#3-abstract-data-model), so that a proposition is content in the record rather than a string an
implementation interprets privately.

**ACP-8-3.** The encoding MUST round-trip: decoding an encoded proposition MUST yield a proposition
equivalent to the original under [ACP-8-4](#81-what-a-binding-must-supply).

**ACP-8-4.** The language MUST have an **equivalence relation** that is decidable, and that is
insensitive to the names of bound variables. This relation is what [ACP-6-6](#63-witness-identity)
canonicalizes for and what makes "the same proposition" checkable rather than textual.

**ACP-8-5.** The language MUST have a **checking relation** by which an implementation decides whether
a given object establishes a given proposition. This is what [§7](#7-the-justification-calculus)'s
certificates are checked against.

**ACP-8-6.** The language MUST be able to express, at minimum: an atomic proposition naming a resource;
conjunction; disjunction; implication; negation; universal and existential quantification; and equality
between terms.

**ACP-8-7.** The atomic proposition naming a resource MUST have no inhabitants of its own. An
implementation MUST NOT permit a proof of an atomic proposition to be constructed structurally; it MUST
arise only from a witness ([§6](#6-traces-and-witnesses)) or from a proof supplied by a verification
system.

> *(informative)* ACP-8-7 is the proposition-language counterpart of
> [ACP-6-1](#62-non-forgeability). If atomic propositions had constructors, an author could prove
> `Asserts(X)` for any `X` and the grounding rules of §7 would be decoration.

### 8.2 What this specification does not require

This specification does not require a particular type theory, logic, or proof system. It requires the
four properties above. A binding satisfying them with a different substrate conforms.

> *(informative)* **AT RISK.** Whether ACP-8-1 through ACP-8-7 are sufficient to guarantee that two
> independently built bindings agree on what a proposition means is not settled. They are sufficient
> for a verifier to check a record *within* one binding, which is what [§10](#10-conformance) requires.
> Cross-binding proposition equivalence is a harder problem and is not addressed here.

---

## 9. AI decision records

[§6](#6-traces-and-witnesses) and [§7](#7-the-justification-calculus) constrain what a record can
claim. This section constrains what a record must **disclose** about how it was produced, when
production involved steps whose outcome the input did not determine.

The requirements here apply to any such step. A step performed by a language model is the motivating
case, but nothing below is specific to models: a heuristic, a ranked retrieval, a sampling procedure,
and a human choosing from a menu are all non-deterministic steps in this sense and are all covered.

### 9.1 What must be recorded

**ACP-9-1.** For each step whose outcome was not determined by the step's input, an implementation
conforming as an Encoding Pipeline MUST emit a **decision record** identifying the unit of input the
step applied to, the outcome that was selected, and how many candidates were available.

**ACP-9-2.** A decision record MUST name the **authority** that produced the outcome.

**ACP-9-3.** The set of authorities MUST be a closed enumeration
([ACP-3-8](#32-the-metamodel-classes-properties-inductive-types)). An implementation MUST NOT record
an authority outside the enumeration, and MUST NOT mint one at use site. Introducing a new kind of
decision-maker MUST be a versioned change to the vocabulary.

> *(informative)* The point of ACP-9-3 is that "what kinds of thing were allowed to make choices in
> this record" is answerable by reading the vocabulary, and changes to that answer are visible as
> vocabulary edits rather than as new strings appearing in data.

### 9.2 Vetoed and unvetoed choices

Non-deterministic steps divide into two kinds, and a reader cannot assess a record without knowing
which kind each step was.

A step is **vetoed** when the implementation independently rejects unacceptable outcomes, so the
non-deterministic component only proposes and a mechanical check disposes. A step is **unvetoed** when
every candidate is acceptable to the implementation and the choice among them is not checkable by it.

**ACP-9-4.** A decision record MUST disclose whether the step was vetoed or unvetoed. For a vetoed
step, the record MUST identify what performed the veto. For an unvetoed step, the record MUST NOT
imply that the outcome was checked.

> *(informative)* This distinction is the difference between two very different guarantees, and
> collapsing them is the most likely way for a record to mislead. In a vetoed step — resolving a
> reference to an antecedent, say — a model may rank candidates freely, and a type check rejects any
> ranking that produces an ill-formed result; the model cannot introduce an error the implementation
> would accept. In an unvetoed step — choosing among readings that all type-check — the model's choice
> stands, and the only controls are the ones in [§9.3](#93-recording-a-model-choice) plus whatever
> external adjudication the producer maintains. A record that presents both as "AI-assisted with
> provenance" tells the reader nothing.

### 9.3 Recording a model choice

**ACP-9-5.** Where the authority is a model, the decision record MUST mark the choice as untrusted and
MUST record:

1. the alternatives the model chose against, in the order the model ranked them where an order exists;
2. the model's stated rationale, where the model supplies one;
3. enough of the context the model was given to identify what question it answered.

An implementation MUST record an abstention as an outcome in its own right. It MUST NOT silently
substitute a default for an abstention, and MUST NOT omit the record.

> *(informative)* Recording abstentions matters more than it appears. A pipeline that drops them
> reports a higher decision rate than it achieved and hides the cases where the model declined —
> which are the cases most worth reviewing.

### 9.4 Record completeness

<a id="acp-9-6"></a>
**ACP-9-6 (omission records).** An implementation conforming as an Encoding Pipeline MUST account for
every unit of its input. A unit that produced no claim MUST appear in the record as an **omission
record** identifying the unit and naming the reason class. Input that could not be processed MUST NOT
be absent from the record.

**ACP-9-7.** Reason classes MUST be a closed enumeration
([ACP-3-8](#32-the-metamodel-classes-properties-inductive-types)) and MUST distinguish at minimum:
input outside the intended scope of processing; failure to interpret the vocabulary; failure to
interpret the structure; and failure to resolve a choice the pipeline was unable to make.

**ACP-9-8.** An omission record MUST NOT be a grade. A unit that produced no claim has no grade,
because there is no claim to grade.

> *(informative)* ACP-9-6 is the requirement that turns coverage into a measurable quantity. A record
> containing only the units that succeeded reports complete coverage by construction, and no reader can
> tell a pipeline that processed everything from one that processed a third and discarded the rest. The
> distinctions ACP-9-7 requires are the ones that make the residue actionable: a vocabulary failure and
> an unresolved choice call for entirely different responses, and a single "failed" class would conceal
> which one occurred.

### 9.5 Replay

**ACP-9-9.** An implementation MUST support **replay**: re-running the pipeline from recorded decisions
rather than by re-invoking the non-deterministic components, and producing the same outcomes.

**ACP-9-10.** The key under which a decision is recorded and retrieved MUST cover the context that was
presented to the authority, not only the unit identifier. A change in the presented context MUST
produce a lookup failure rather than the reuse of a decision made under different context.

> *(informative)* ACP-9-10 is what makes replay evidence rather than decoration. If the key covered
> only the unit, editing the surrounding document would silently reuse decisions made about a different
> question, and the replayed run would appear to reproduce the original while answering something else.

**ACP-9-11.** A replay lookup that fails MUST fail closed — the affected unit MUST NOT be encoded by
substituting a live invocation or a default — and the failure MUST be counted and reported.

**ACP-9-12.** Replay MUST NOT change any outcome that was determined by the input. An implementation
MUST produce identical results for every deterministic stage whether or not decisions are replayed.

> *(informative)* ACP-9-12 makes replay usable as a regression instrument: a difference between a live
> run and a replayed run localises to the non-deterministic steps, because nothing else was permitted
> to vary.

### 9.6 Confidence

**ACP-9-13.** A confidence value, where recorded, MUST be advisory. An implementation MUST NOT treat
confidence as a grade, MUST NOT accept it in place of a witness, and MUST NOT let it determine whether
a claim is admitted.

> *(informative)* Confidence and grade answer different questions and are not comparable. A grade says
> how a claim was established; a confidence says how sure a component was. A pipeline may reasonably
> route low-confidence results to human review, which is a workflow decision — but a claim admitted on
> confidence alone has no witness, and a verifier would correctly refuse it.

---

## 10. Conformance

### 10.1 Conformance classes

This specification defines four conformance classes. An implementation MAY conform to more than one.
An implementation claiming conformance MUST state which classes it claims
([ACP-10-1](#acp-10-1)).

The classes are separated so that the ability to *check* a record does not depend on the ability to
*produce* one. A verifier that shares no code with a producer is what makes a record's claims
independent of the producer's trustworthiness; a conformance story that could not describe such a
verifier would not support the reproducibility this specification exists to provide.

#### 10.1.1 Record Producer

An implementation that emits provenance records.

A Record Producer MUST emit records satisfying the data model of [§3](#3-abstract-data-model), the
canonicalization and content-addressing requirements of
[§4](#4-canonicalization-and-content-addressing), and the grade rules of
[§5](#5-epistemic-grades). It MUST NOT emit a record in which a grade is stated but not
supported by a trace admitting the corresponding witness.

A Record Producer need not be able to check certificates; a producer that composes justifications MUST
also conform as a Witness Authority.

#### 10.1.2 Record Verifier

An implementation that checks a record without trusting its producer.

A Record Verifier MUST, given a record and no other input from the producer:

1. recompute every content address in the record and confirm it matches ([§4](#4-canonicalization-and-content-addressing));
2. recompute the proposition binding of every witness the record relies on and confirm each is admitted
   by a trace present in the record ([§6](#6-traces-and-witnesses));
3. re-check every certificate against the proposition it certifies ([§7](#7-the-justification-calculus));
4. recompute every grade from justification structure and confirm it matches the grade recorded
   ([§5](#5-epistemic-grades), [§7](#7-the-justification-calculus));
5. report a specific failure — identifying the resource and the requirement violated — rather than a
   boolean, for each check that does not pass.

A Record Verifier MUST NOT consult the producer, a network service, or any authority outside the
record and this specification in order to complete checks 1 through 4. A verifier that cannot complete
a check with the material in the record MUST report the record as incomplete rather than as valid.

> *(informative)* This class is where "auditing becomes an independently reproducible process"
> is cashed out. Everything else in the specification exists so that this class is implementable.

#### 10.1.3 Witness Authority

An implementation that admits witnesses — in practice, the component that validates commits.

A Witness Authority MUST satisfy [ACP-6-1](#acp-6-1). It MUST admit a witness only for a proposition
resolved by the rules of [§6](#6-traces-and-witnesses), and MUST NOT admit a witness whose proposition
differs from the one carried by the resource the trace targets.

A Witness Authority MUST expose no interface — authoring syntax, API, configuration, or import path —
through which a caller can supply a witness directly. This is a requirement on the implementation's
entire surface, not on its record format: a format with no syntax for witnesses, served by an API that
accepts them, does not conform.

#### 10.1.4 Encoding Pipeline

An implementation that derives claims from source documents, with one or more steps whose outcome is
not determined by the input — including any step performed by a language model.

An Encoding Pipeline MUST conform as a Record Producer, and MUST additionally satisfy the
requirements of [§9](#9-ai-decision-records): authority disclosure, the untrusted-but-recorded rule,
replay, and record completeness.

An Encoding Pipeline MUST account for every unit of its input. A unit that produced no claim MUST
appear in the record as an omission record with a reason class ([ACP-9-6](#acp-9-6)). Silently
dropping input that could not be processed is the specific failure this class exists to exclude: a
record covering only the units that succeeded reports a coverage rate of 100% by construction.

### 10.2 Claiming conformance

<a id="acp-10-1"></a>
**ACP-10-1.** An implementation claiming conformance MUST state which conformance classes it claims.

<a id="acp-10-2"></a>
**ACP-10-2.** An implementation claiming conformance MUST state which binding
([Appendix A](#appendix-a-the-eigon-eigentt-binding-normative), or another) it implements. A
conformance claim that does not name a binding is not checkable, because the abstract core does not
fix a serialization, a hash construction, or a proposition language.

<a id="acp-10-3"></a>
**ACP-10-3.** An implementation MUST NOT describe a record as conforming when any requirement it
depends on is satisfied only by a component the verifier cannot inspect.

### 10.3 Assertion index

Every normative statement in this specification, with the conformance classes that bear it. Summaries
are abbreviated; the numbered statement in the body is authoritative.

**Classes:** `P` Record Producer · `V` Record Verifier · `W` Witness Authority · `E` Encoding Pipeline
· `binding` applies to implementations of [Appendix A](#appendix-a-the-eigon-eigentt-binding-normative)
· `all` applies to any conformance claim.

| ID | Requirement | Classes |
|---|---|---|
| `ACP-3-1` | A top-level resource MUST be identified by an IRI. That IRI is the resource's identity: two… | P V |
| `ACP-3-2` | A resource's content MUST be a set of properties, each keyed by an IRI. A property key MUST… | P V |
| `ACP-3-3` | An embedded resource MAY appear as a property value | P V |
| `ACP-3-4` | A resource MUST declare its class membership through the is_a property | P V |
| `ACP-3-5` | An absent property and a property present with a null value MUST be treated as the same… | P V |
| `ACP-3-6` | A class resource MUST declare the properties its instances are required to carry and SHOULD… | P V |
| `ACP-3-7` | A property resource MUST declare the type of its values | P V |
| `ACP-3-8` | A property that restricts its values to an explicit set of resources defines a closed… | P V |
| `ACP-3-9` | An inductive type resource MUST declare a finite, ordered list of constructors, each… | P V |
| `ACP-3-10` | Because constructor lists are resources, an implementation MUST be able to enumerate the… | P V |
| `ACP-3-11` | A layer MUST be an immutable set of resources | P V |
| `ACP-3-12` | A layer MUST record references to its parent layers | P V |
| `ACP-3-13` | The parent relation MUST be acyclic. | P V |
| `ACP-3-14` | An IRI MUST be resolved against a layer by searching that layer and then its ancestors | P V |
| `ACP-3-15` | A layer MAY tombstone an IRI, making it unresolvable from that layer onward without… | P V |
| `ACP-3-16` | An implementation MUST validate a layer before admitting it | W |
| `ACP-3-17` | A layer that fails validation MUST NOT be admitted, and MUST NOT emit witnesses (§6) | W |
| `ACP-4-1` | A binding MUST define a canonical form: a total function from a resource to a byte sequence | P V |
| `ACP-4-2` | A binding MUST define a content address: the output of a collision-resistant cryptographic… | P V |
| `ACP-4-3` | A binding MUST define a position address for a layer, binding its content address together… | P V |
| `ACP-4-4` | The position address MUST be a function of the content address and the *set* of parent… | P V |
| `ACP-4-5` | Distinct hash constructions defined by a binding MUST use distinct domain separators, and… | P V |
| `ACP-4-6` | A binding MUST specify its hash function by name and MUST state the consequences of changing it | P V |
| `ACP-4-7` | A verifier MUST be able to recompute every address in a record from the record's own… | P V |
| `ACP-5-1` | An implementation MUST support all four grades | P V |
| `ACP-5-2` | Verified MUST be a specialization of Derived: anything verified is also derived, and a… | P V |
| `ACP-5-3` | A Declared claim MUST name the party that declared it, and SHOULD carry a rationale and a… | P V |
| `ACP-5-4` | An Observed claim MUST name its source, and SHOULD carry the time of observation. | P V |
| `ACP-5-5` | A Derived claim SHOULD carry a reference to the record of the computation that produced it. | P V |
| `ACP-5-6` | A Verified claim MUST carry both a reference to the derivation and a reference to the… | P V |
| `ACP-5-7` | A grade recorded on a claim MUST NOT be treated as evidence | P V |
| `ACP-5-8` | Where a claim carries an explicit justification (§7), its grade MUST equal the grade… | P V |
| `ACP-6-1` *(witness non-forgeability)* | An implementation MUST NOT provide any means by which author-supplied content can introduce… | W |
| `ACP-6-2` | The record format MUST NOT define a syntax for witnesses, and the implementation MUST NOT… | W |
| `ACP-6-3` | A binding MUST realise witnesses as a type with no constructors, or by an equivalent… | W V |
| `ACP-6-4` | A witness MUST be identified by exactly three components: the grade it attests, the IRI of… | W V |
| `ACP-6-5` | The proposition binding MUST be a collision-resistant hash of the proposition's canonical… | W V |
| `ACP-6-6` | Propositions that differ only by the names of bound variables MUST produce the same binding | W V |
| `ACP-6-7` | The proposition a witness attests MUST be determined by the resource the trace targets, by… | W V |
| `ACP-6-8` | A resource MUST carry at most one canonical proposition | W V |
| `ACP-6-9` | A binding MAY define additional resolution rules, ahead of rule 1, for resources whose… | W V |
| `ACP-6-10` | The implementation MUST derive the proposition binding from the proposition as… | W V |
| `ACP-6-11` | A binding MUST define one trace class per grade, and MUST specify for each the properties… | W V |
| `ACP-6-12` | A trace MUST NOT be emitted by an author | W V |
| `ACP-6-13` | A binding MAY define resource classes that attest themselves — that is, whose validated… | W V |
| `ACP-6-14` | A witness MUST be admitted if any layer in the chain reachable from the current layer admits it | W V |
| `ACP-6-15` | A witness lookup that fails MUST produce a diagnostic naming the grade, the IRI, and the… | W V |
| `ACP-6-16` | A witness at a specialized grade MUST satisfy an obligation stated at the grade it specializes | W V |
| `ACP-7-1` | A binding MUST define the justification term language as an inductive type resource… | W V |
| `ACP-7-2` | The constructor list MUST be closed against extension at use site | W V |
| `ACP-7-3` | A justification term MUST be first order: no constructor argument may be a proposition, a… | W V |
| `ACP-7-4` | A binding MUST define a certificate relation between a justification term and a… | W V |
| `ACP-7-5` | For each grade, the calculus MUST include a rule that consumes a witness at that grade and… | W V |
| `ACP-7-6` *(application)* | The calculus MUST include a rule that, from a certificate that one term justifies an… | W V |
| `ACP-7-7` *(alternatives)* | The calculus MUST include rules by which a certificate for either of two terms yields a… | W V |
| `ACP-7-8` *(specialization)* | The calculus MUST include a rule that eliminates a universal quantifier in a justified… | W V |
| `ACP-7-9` | The calculus MUST NOT include any rule whose conclusion is a certificate at an implication | W V |
| `ACP-7-10` | The grade of a claim carrying a justification MUST be computed from the justification term… | W V |
| `ACP-7-11` | Verified MUST NOT propagate through a composition in which any sub-term — including the… | W V |
| `ACP-7-12` | A record MUST be able to carry more than one justification for the same proposition, and… | W V |
| `ACP-8-1` | A binding MUST specify a proposition language: the set of well-formed propositions about… | P W V |
| `ACP-8-2` | The language MUST have a canonical encoding into the data model of §3, so that a… | P W V |
| `ACP-8-3` | The encoding MUST round-trip: decoding an encoded proposition MUST yield a proposition… | P W V |
| `ACP-8-4` | The language MUST have an equivalence relation that is decidable, and that is insensitive… | P W V |
| `ACP-8-5` | The language MUST have a checking relation by which an implementation decides whether a… | P W V |
| `ACP-8-6` | The language MUST be able to express, at minimum: an atomic proposition naming a resource;… | P W V |
| `ACP-8-7` | The atomic proposition naming a resource MUST have no inhabitants of its own | P W V |
| `ACP-9-1` | For each step whose outcome was not determined by the step's input, an implementation… | E |
| `ACP-9-2` | A decision record MUST name the authority that produced the outcome. | E |
| `ACP-9-3` | The set of authorities MUST be a closed enumeration (ACP-3-8) | E |
| `ACP-9-4` | A decision record MUST disclose whether the step was vetoed or unvetoed | E |
| `ACP-9-5` | Where the authority is a model, the decision record MUST mark the choice as untrusted and… | E |
| `ACP-9-6` *(omission records)* | An implementation conforming as an Encoding Pipeline MUST account for every unit of its input | E |
| `ACP-9-7` | Reason classes MUST be a closed enumeration (ACP-3-8) and MUST distinguish at minimum:… | E |
| `ACP-9-8` | An omission record MUST NOT be a grade | E |
| `ACP-9-9` | An implementation MUST support replay: re-running the pipeline from recorded decisions… | E |
| `ACP-9-10` | The key under which a decision is recorded and retrieved MUST cover the context that was… | E |
| `ACP-9-11` | A replay lookup that fails MUST fail closed — the affected unit MUST NOT be encoded by… | E |
| `ACP-9-12` | Replay MUST NOT change any outcome that was determined by the input | E |
| `ACP-9-13` | A confidence value, where recorded, MUST be advisory | E |
| `ACP-10-1` | An implementation claiming conformance MUST state which conformance classes it claims. | all |
| `ACP-10-2` | An implementation claiming conformance MUST state which binding (Appendix A, or another) it… | all |
| `ACP-10-3` | An implementation MUST NOT describe a record as conforming when any requirement it depends… | all |
| `ACP-11-1` | A record MAY omit source text and model rationales while remaining conforming, provided… | P |
| `ACP-11-2` | Where a record is redacted, its content addresses MUST be recomputed over the redacted… | P |
| `ACP-11-3` | Implementers MUST NOT treat a content address as opaque with respect to the content | P |
| `ACP-12-1` | An implementation MUST NOT describe a record as authenticated, attributed, or signed on the… | all |
| `ACP-12-2` | A specification layering attribution over this one MUST bind its signatures to content… | all |
| `ACP-A-1` | A resource MUST be serialized as a JSON object | binding |
| `ACP-A-2` | A JSON object without @id appearing as a property value MUST be an embedded resource (ACP-3-3). | binding |
| `ACP-A-3` | A document MAY carry a single resource as an object, or several as an array of objects. | binding |
| `ACP-A-4` | The media type for this serialization is application/eigon+json. | binding |
| `ACP-A-5` | The canonical JSON form MUST be [RFC 8785] (JSON Canonicalization Scheme): keys sorted by… | binding |
| `ACP-A-6` | Empty objects, empty arrays, and null values MUST NOT appear in canonical form (ACP-3-5). | binding |
| `ACP-A-7` | For hashing, a resource MUST be encoded in CBOR [RFC 8949] using Core Deterministic… | binding |
| `ACP-A-8` | A property value that is an opaque structured payload rather than an embedded resource MUST… | binding |
| `ACP-A-9` | The content address of a layer MUST be computed as follows, where ‖ is concatenation,… | binding |
| `ACP-A-10` | Both sections MUST be emitted unconditionally | binding |
| `ACP-A-11` | The content address MUST NOT incorporate the layer's parents, name, position, creation… | binding |
| `ACP-A-12` | The position address of a layer — its identity within a chain — MUST be computed as: | binding |
| `ACP-A-13` | Parent addresses MUST be sorted before concatenation, so that the position address does not… | binding |
| `ACP-A-14` | Propositions MUST be terms of the EigenTT type theory — a fragment of the Calculus of… | binding |
| `ACP-A-15` | A proposition MUST be encoded on the chain as a tree of tagged objects of the form {"ctor":… | binding |
| `ACP-A-16` | The connectives required by ACP-8-6 MUST be expressed with these constructors rather than… | binding |
| `ACP-A-17` | The atomic proposition of ACP-8-7 MUST be the chain-declared inductive type core:Asserts,… | binding |
| `ACP-A-18` | The encoding MUST round-trip (ACP-8-3). | binding |
| `ACP-A-19` | The proposition binding of ACP-6-5 MUST be computed as: | binding |
| `ACP-A-20` | alpha_canonicalize MUST rewrite bound-variable names to a positional scheme, as follows | binding |
| `ACP-A-21` | An implementation MUST decode a stored proposition and hash the decoded term, rather than… | binding |
| `ACP-A-22` | The four grades MUST be realised as the classes reflection:DeclaredResource,… | binding |
| `ACP-A-23` | Per-grade obligations (§5.2) MUST be realised as the classes' required and recommended… | binding |
| `ACP-A-24` | The witness predicates MUST be realised as the four inductive types witness:IsDeclaredAs,… | binding |
| `ACP-A-25` | Trace classes MUST be reflection:DeclarationTrace, reflection:ObservationTrace,… | binding |
| `ACP-A-26` | A validated commit MUST admit witnesses from traces as follows: DeclarationTrace admits… | binding |
| `ACP-A-27` | The self-attesting classes of ACP-6-13 MUST be exactly: reasoning:ReasoningSentence,… | binding |
| `ACP-A-28` | The canonical-proposition property of ACP-6-7 MUST be reflection:canonical_proposition,… | binding |
| `ACP-A-29` | The justification term language MUST be the inductive type reasoning:JustificationTerm with… | binding |
| `ACP-A-30` | SpecStr's second argument MUST be an audit tag naming the instantiation, and MUST NOT carry… | binding |
| `ACP-A-31` | The certificate relation MUST be the indexed inductive type reasoning:JustifiedBy, indexed… | binding |
| `ACP-A-32` | There MUST be no elimination rule for Sum (ACP-7-7). | binding |
| `ACP-A-33` | No constructor's conclusion may be at an implication (ACP-7-9) | binding |
| `ACP-A-34` | Each constructor's full dependent signature MUST be carried on the chain, encoded per… | binding |
| `ACP-A-35` | A claim carrying a justification MUST be a reasoning:ReasoningSentence, requiring the… | binding |
| `ACP-A-36` | The vocabulary of §9 MUST be realised by the enc: classes below | binding |
| `ACP-A-37` | The selection authorities MUST be exactly: a human-supplied pin; the reading ranker; and… | binding |
| `ACP-A-38` | The binding authorities MUST be exactly: the deterministic recency ordering; the live… | binding |
| `ACP-A-39` | The omission reason classes MUST be exactly: out of scope; vocabulary gap; grammar gap;… | binding |
| `ACP-A-40` | The reference-resolution step MUST be vetoed (ACP-9-4): a proposed antecedent that does not… | binding |

<!-- 128 assertions -->

> *The mapping from assertions to conformance tests is added once the suite exists; the test-vector
> requirements in [Appendix C](#appendix-c-test-vectors) name the assertions each vector exercises and
> are the starting point.*

---

## 11. Security and privacy considerations

### 11.1 What a verifier establishes, and what it does not

A Record Verifier ([§10.1.2](#1012-record-verifier)) establishes that a record is **internally
consistent**: every address matches its content, every certificate checks against the proposition it
certifies, every grade matches the justification it is computed from, and every witness a certificate
consumes is admitted by a trace present in the record.

It does not establish that the traces are true.

A `DeclarationTrace` says that a party declared something. A `ProgramTrace` says that a program
produced something from named inputs. A verifier checking a record cannot re-run the world: it cannot
confirm that the declaration was made or that the program ran. What it confirms is that the claim is
**grounded** — that it terminates in a trace rather than in nothing — and that the grounding structure
is complete and typed.

**The value of this is that the trust is localised and enumerable.** Before, every field in a
provenance record was a place the producer could have lied. After, the places are exactly the traces,
and a verifier can list them. A reader who wants to know what a record takes on authority reads the
Declared groundings, and that set is finite and usually small.

[§7.3](#73-no-implication-introduction) is what keeps it small. Because no rule introduces an
implication, every inferential bridge in the record — every "if this, then that" — is itself a
grounded claim with a trace and a named party, rather than something the system derived for itself. A
record cannot manufacture a warrant whose bridging premise nobody asserted.

### 11.2 Threat model

**Addressed.**

- *Silent substitution.* Changing what a resource says changes its witness identity
  ([ACP-6-5](#63-witness-identity)), so citations written against the old content stop resolving. A
  record cannot acquire a new meaning under a stable reference.
- *Undetected alteration of history.* The position address binds a layer to its ancestors
  ([ACP-4-3](#41-requirements)), so altering an ancestor changes every descendant's identity.
- *Grade inflation.* A grade recorded without a witness to support it is rejected
  ([ACP-5-7](#53-grades-are-computed-not-asserted)); a grade inconsistent with its justification is
  rejected ([ACP-5-8](#53-grades-are-computed-not-asserted)).
- *Asserted computation.* A conforming implementation cannot emit a witness for work it did not
  perform and validate ([ACP-6-1](#62-non-forgeability)).
- *Coverage inflation.* A pipeline cannot report success on input it silently discarded
  ([ACP-9-6](#94-record-completeness)).

**Not addressed.**

- *A non-conforming producer.* [ACP-6-1](#62-non-forgeability) constrains implementations that
  conform. An implementation that claims conformance and emits witnesses freely produces records that
  a verifier will find internally consistent, because the inconsistency is not in the record. Nothing
  in this specification detects it. This is the gap that attribution
  ([§12](#12-out-of-scope-authenticity-and-attribution)) would narrow and that its absence leaves open.
- *A conforming implementation with a defect.* The guarantee is no stronger than the implementation
  of the check.
- *A correct record of a wrong conclusion.* [§1.4](#14-what-this-specification-does-not-define).
- *Confidentiality of the record.* See [§11.3](#113-what-a-complete-record-discloses).
- *Availability.* Nothing here addresses denial of service, and the chain walk of
  [ACP-6-14](#67-admission-scope) has a cost that a hostile record can inflate.

### 11.3 What a complete record discloses

A record produced by an Encoding Pipeline is far more disclosive than a summary of its conclusions.
Conforming to [§9](#9-ai-decision-records) requires it to carry, at minimum: the identity of each unit
of source input and its character span; the reason each unprocessed unit failed; the alternatives
considered at each decision; and any rationale a model supplied.

For a clinical, commercial, or embargoed corpus, this may include material that cannot be shared even
when the conclusions can. Implementers MUST NOT assume a record is safe to publish because its
conclusions are.

**ACP-11-1.** A record MAY omit source text and model rationales while remaining conforming, provided
every omission is itself recorded. An implementation MUST NOT silently drop disclosive fields: an
omitted field MUST be distinguishable from an absent one.

**ACP-11-2.** Where a record is redacted, its content addresses MUST be recomputed over the redacted
content, and the record MUST state that it is redacted. An implementation MUST NOT present a content
address computed over unredacted content alongside redacted content.

> *(informative)* ACP-11-2 forecloses an attractive mistake — keeping the original addresses so the
> redacted record still "matches" the original. A content address attests to the bytes it was computed
> over. An address that does not match the content beside it is worse than no address, because a
> verifier following ACP-4-7 will report a mismatch it cannot explain.

> *(informative)* **AT RISK.** A redaction scheme under which a third party can verify that a
> redacted record is a faithful redaction of a specific original — rather than merely internally
> consistent — is not specified here. Hash-tree constructions supporting selective disclosure are the
> obvious direction and are the natural companion to [§12](#12-out-of-scope-authenticity-and-attribution).

### 11.4 Content addressing reveals content equality

Content addressing is deterministic over content, so two parties holding the same content compute the
same address. Publishing an address therefore discloses that one holds that exact content, to anyone
who holds it or can guess it.

**ACP-11-3.** Implementers MUST NOT treat a content address as opaque with respect to the content. For
low-entropy or enumerable content, an address is equivalent to a commitment an adversary can confirm
by trial.

> *(informative)* This cuts both ways and is not only a hazard. Two laboratories can determine whether
> they hold the same dataset by exchanging addresses and nothing else. The same mechanism lets an
> observer confirm a suspicion about content that was never disclosed.

---

## 12. Out of scope: authenticity and attribution

The mechanisms in this specification provide **integrity** — content cannot change without its address
changing — and **reproducibility** — an independent party can recompute every address and re-check
every certificate. They do not provide **attribution**: nothing here establishes who produced a record.

This is a deliberate boundary, stated rather than implied, because the difference matters at exactly
the point where records are exchanged between parties who do not trust each other.

**What follows from the boundary.** A trace names its originator as data: a string identifying a
declaring party, a source, or a program. Nothing binds that string to a real party. A record's traces
can name anyone. The consistency a verifier establishes ([§11.1](#111-what-a-verifier-establishes-and-what-it-does-not))
is consistency of the record with itself, and it holds equally for a record whose traces are fiction.

**ACP-12-1.** An implementation MUST NOT describe a record as authenticated, attributed, or signed on
the basis of conformance to this specification.

**ACP-12-2.** A specification layering attribution over this one MUST bind its signatures to content
addresses ([§4](#4-canonicalization-and-content-addressing)) rather than to a serialization, so that
attribution survives re-serialization and so that the two layers agree on what was attributed.

> *(informative)* **UNVERIFIED.** The Verifiable Credentials Data Model, C2PA, and in-toto each carry
> an issuer-and-signature model that could supply the missing layer. Which is appropriate — or whether
> the group should specify its own — is an open question, and the editors have not checked the current
> state of any of them. See [Appendix B](#appendix-b-relation-to-prior-art-informative).

> **Implementation status.** The reference implementation performs no signing and manages no keys. It
> content-addresses and hash-links; it does not attest. This section describes a boundary the
> implementation actually has, not one it has chosen to expose selectively.

---

## Appendix A. The Eigon / EigenTT binding *(normative)*

This appendix is a **binding**: a concrete realisation of the abstract core, complete enough to
implement against and to test. An implementation claiming conformance names the binding it implements
([ACP-10-2](#102-claiming-conformance)).

Requirements in this appendix are numbered `ACP-A-n` and apply only to implementations of this
binding.

### A.1 Serialization

**ACP-A-1.** A resource MUST be serialized as a JSON object. The key `@id` MUST carry the resource's
IRI and MUST be the only reserved key. Every other key MUST be the full IRI of a property.

**ACP-A-2.** A JSON object without `@id` appearing as a property value MUST be an embedded resource
([ACP-3-3](#31-resources)).

**ACP-A-3.** A document MAY carry a single resource as an object, or several as an array of objects.

**ACP-A-4.** The media type for this serialization is `application/eigon+json`.

> *(informative)* **AT RISK.** The media type is not registered. Registration, and whether the
> specification should mint its own type, are questions for the group.

### A.2 Canonical form

**ACP-A-5.** The canonical JSON form MUST be [RFC 8785] (JSON Canonicalization Scheme): keys sorted by
Unicode code point, no insignificant whitespace, and the RFC 8785 number representation.

**ACP-A-6.** Empty objects, empty arrays, and null values MUST NOT appear in canonical form
([ACP-3-5](#31-resources)).

**ACP-A-7.** For hashing, a resource MUST be encoded in CBOR [RFC 8949] using Core Deterministic
Encoding (RFC 8949 §4.2): map keys sorted by their encoded byte string, and the shortest encoding for
every value.

**ACP-A-8.** A property value that is an opaque structured payload rather than an embedded resource
MUST be distinguished on the wire by CBOR tag `27182`. Without the tag a payload object and an
embedded resource encode to the same CBOR map and cannot be told apart on decode.

> *(informative)* **AT RISK.** Tag `27182` is presently drawn from IANA's unassigned range. A
> standardised binding requires a registered tag.

### A.3 Content address

**ACP-A-9.** The content address of a layer MUST be computed as follows, where `‖` is concatenation,
`u64le(n)` is `n` as eight bytes little-endian, and `utf8(s)` is the UTF-8 encoding of `s` with no
terminator unless one is shown:

```
content_address = SHA-256(
    "content:v1:"
  ‖ "::resources::"
  ‖ u64le( number of resources )
  ‖ for each (iri, resource), in ascending IRI order:
        utf8(iri) ‖ deterministic_cbor(resource)
  ‖ "::tombstones::"
  ‖ u64le( number of tombstoned IRIs )
  ‖ for each iri, in ascending order:
        utf8(iri) ‖ 0x00
)
```

**ACP-A-10.** Both sections MUST be emitted unconditionally. A layer with no tombstones MUST still
contribute the `::tombstones::` separator and a zero count, so that the address is a total function of
`(resources, tombstones)` rather than of resources alone.

**ACP-A-11.** The content address MUST NOT incorporate the layer's parents, name, position, creation
time, or author ([ACP-4-2](#41-requirements)).

### A.4 Position address

**ACP-A-12.** The position address of a layer — its identity within a chain — MUST be computed as:

```
position_address = SHA-256(
    "position:v1:"
  ‖ content_address                       (32 bytes)
  ‖ u64le( number of parents )
  ‖ concat( parent position addresses, sorted ascending as 32-byte strings )
)
```

**ACP-A-13.** Parent addresses MUST be sorted before concatenation, so that the position address does
not depend on the order in which parents were presented ([ACP-4-4](#41-requirements)).

### A.5 The proposition language

**ACP-A-14.** Propositions MUST be terms of the EigenTT type theory — a fragment of the Calculus of
Inductive Constructions — inhabiting the impredicative universe `Prop`.

**ACP-A-15.** A proposition MUST be encoded on the chain as a tree of tagged objects of the form
`{"ctor": <name>, "args": [ ... ]}`, using exactly these nine constructors:

| Constructor | Arguments | Denotes |
|---|---|---|
| `Sort` | `level` | A universe. Level 0 is `Prop`, level 1 is `Set`. |
| `Var` | `name` | A bound-variable reference. |
| `ConstRef` | `iri` | A reference to a chain-declared type former. Always nullary; multi-argument references are built by currying through `App`. |
| `App` | `head`, `arg` | Application. |
| `Pi` | `name`, `domain`, `body` | Dependent function type. An empty `name` denotes an anonymous binder, giving ordinary implication. |
| `Sig` | `name`, `domain`, `body` | Dependent pair type. An empty `name` denotes an anonymous binder. |
| `Lam` | `name`, `domain`, `body` | Type-level abstraction, used for motives and parametric definitions. |
| `One` | — | The unit type. |
| `Id` | `type`, `lhs`, `rhs` | Propositional equality. |

**ACP-A-16.** The connectives required by [ACP-8-6](#81-what-a-binding-must-supply) MUST be expressed
with these constructors rather than by adding primitives: conjunction as `Sig` with both components in
`Prop`; implication as `Pi` with an anonymous binder; negation as implication into the empty type;
universal and existential quantification as `Pi` and `Sig`; disjunction as the chain-declared sum type
at `Prop`; equality as `Id`.

**ACP-A-17.** The atomic proposition of [ACP-8-7](#81-what-a-binding-must-supply) MUST be the
chain-declared inductive type `core:Asserts`, taking the asserted resource's IRI as a uniform
parameter and declaring **no constructors**.

**ACP-A-18.** The encoding MUST round-trip ([ACP-8-3](#81-what-a-binding-must-supply)).

### A.6 Proposition binding and α-canonicalization

**ACP-A-19.** The proposition binding of [ACP-6-5](#63-witness-identity) MUST be computed as:

```
proposition_binding = SHA-256( deterministic_cbor( alpha_canonicalize( encode(P) ) ) )
```

**ACP-A-20.** `alpha_canonicalize` MUST rewrite bound-variable names to a positional scheme, as
follows. Walk the encoded tree maintaining a stack of `(original name, canonical name)` pairs. Each
`Pi`, `Sig`, or `Lam` node pushes its binder with canonical name `_b<depth>`, where `<depth>` is the
binder's position counted from the outside. Each `Var` node's name is rewritten to the canonical name
of the nearest enclosing binder that matches, so that inner binders shadow outer ones. A binder whose
name is the empty string is anonymous: it MUST be preserved as-is and MUST NOT push an entry. A `Var`
with no matching binder is free and MUST be preserved unchanged.

**ACP-A-21.** An implementation MUST decode a stored proposition and hash the decoded term, rather
than hashing the stored form, wherever the binding permits definitions that expand
([ACP-6-10](#64-which-proposition-a-witness-attests)).

### A.7 Grades, traces, and witnesses

**ACP-A-22.** The four grades MUST be realised as the classes `reflection:DeclaredResource`,
`reflection:ObservedResource`, `reflection:DerivedResource`, and `reflection:VerifiedResource`, with
`VerifiedResource` declaring `DerivedResource` as a parent class ([ACP-5-2](#51-the-four-grades)).

**ACP-A-23.** Per-grade obligations ([§5.2](#52-per-grade-obligations)) MUST be realised as the
classes' required and recommended property lists:

| Class | Required | Recommended |
|---|---|---|
| `DeclaredResource` | `declared_by` | `rationale`, `timestamp` |
| `ObservedResource` | `source` | `source_irl`, `observed_at`, `timestamp` |
| `DerivedResource` | — | `derivation` |
| `VerifiedResource` | `derivation`, `verification` | — |

**ACP-A-24.** The witness predicates MUST be realised as the four inductive types
`witness:IsDeclaredAs`, `witness:IsObservedAs`, `witness:IsDerivedAs`, and `witness:IsVerifiedAs`,
each indexed by a string (the IRI) and a `Prop` (the proposition), each declaring **no constructors**,
and each inhabiting `Prop`.

> *(informative)* The empty constructor list is what satisfies
> [ACP-6-3](#62-non-forgeability), and it is directly readable: the resource at
> `urn:eigenius:reasoning:ChainWitness:IsDeclaredAs` carries `"urn:eigenius:core:ctors": []`. A
> verifier confirms the guarantee by reading the record, not by trusting the implementation.

**ACP-A-25.** Trace classes MUST be `reflection:DeclarationTrace`, `reflection:ObservationTrace`,
`reflection:ProgramTrace`, and `reflection:VerificationTrace`. Each MUST require a reference to the
resource it attests and a timestamp; each MUST additionally require the property that names its
originator — `declared_by` for declaration, `source` for observation and for program traces, and
`proof_system` with `proof_term` for verification.

**ACP-A-26.** A validated commit MUST admit witnesses from traces as follows: `DeclarationTrace`
admits `IsDeclaredAs`; `ObservationTrace` admits `IsObservedAs`; `ProgramTrace` admits `IsDerivedAs`.

> **Implementation status.** `VerificationTrace` does **not** currently admit `IsVerifiedAs` in the
> reference implementation. The `Verified` grade is admitted only through the self-attesting route of
> [ACP-A-27](#a7-grades-traces-and-witnesses), pending the reification of external proof statements
> into the proposition language. ACP-A-26's fourth arm is therefore specified but unimplemented, per
> the note at [ACP-6-11](#65-trace-classes).

**ACP-A-27.** The self-attesting classes of [ACP-6-13](#66-self-attesting-witnesses) MUST be exactly:
`reasoning:ReasoningSentence`, admitting `IsVerifiedAs` on its own IRI; and
`reflection:InstitutionEmittedDerivation`, admitting `IsDerivedAs` on its own IRI.

**ACP-A-28.** The canonical-proposition property of
[ACP-6-7](#64-which-proposition-a-witness-attests) MUST be `reflection:canonical_proposition`, carrying
a proposition encoded per [ACP-A-15](#a5-the-proposition-language). Where absent, the attested
proposition MUST be `core:Asserts(<target IRI>)`.

### A.8 Justification terms and certificates

**ACP-A-29.** The justification term language MUST be the inductive type
`reasoning:JustificationTerm` with exactly seven constructors:

| Constructor | Arguments |
|---|---|
| `DeclaredEvidence` | `string` (an IRI) |
| `ObservedEvidence` | `string` |
| `DerivedEvidence` | `string` |
| `VerifiedEvidence` | `string` |
| `App` | `JustificationTerm`, `JustificationTerm` |
| `Sum` | `JustificationTerm`, `JustificationTerm` |
| `SpecStr` | `JustificationTerm`, `string` |

**ACP-A-30.** `SpecStr`'s second argument MUST be an audit tag naming the instantiation, and MUST NOT
carry the instance itself. The instance is bound on the certificate side. This keeps the term algebra
first order ([ACP-7-3](#71-justification-terms)).

**ACP-A-31.** The certificate relation MUST be the indexed inductive type `reasoning:JustifiedBy`,
indexed by a `JustificationTerm` and a `Prop`, inhabiting `Type 0` rather than `Prop` so that a
certificate is stored and re-checkable rather than erased. It MUST declare exactly nine constructors:

| Rule | Premises | Conclusion |
|---|---|---|
| `declared` | `IsDeclaredAs(iri, P)` | `JustifiedBy(DeclaredEvidence(iri), P)` |
| `observed` | `IsObservedAs(iri, P)` | `JustifiedBy(ObservedEvidence(iri), P)` |
| `derived` | `IsDerivedAs(iri, P)` | `JustifiedBy(DerivedEvidence(iri), P)` |
| `verified` | `IsVerifiedAs(iri, P)` | `JustifiedBy(VerifiedEvidence(iri), P)` |
| `app` | `JustifiedBy(j₁, A → B)`, `JustifiedBy(j₂, A)` | `JustifiedBy(App(j₁, j₂), B)` |
| `sum_l` | `JustifiedBy(j₁, P)` | `JustifiedBy(Sum(j₁, j₂), P)` |
| `sum_r` | `JustifiedBy(j₂, P)` | `JustifiedBy(Sum(j₁, j₂), P)` |
| `spec_str` | `JustifiedBy(j, ∀ x : string. P(x))` | `JustifiedBy(SpecStr(j, t), P(t))` |
| `spec_poly` | `JustifiedBy(j, ∀ y : T. P(y))` for any domain `T : Set` | `JustifiedBy(SpecStr(j, tag), P(x))` |

**ACP-A-32.** There MUST be no elimination rule for `Sum` ([ACP-7-7](#72-certificates)).

**ACP-A-33.** No constructor's conclusion may be at an implication
([ACP-7-9](#73-no-implication-introduction)). Inspection of the table above confirms this: `app`
concludes at `B`, `sum_l` and `sum_r` at `P`, `spec_str` and `spec_poly` at an instance of `P`, and
the four grounding rules at the witnessed proposition.

**ACP-A-34.** Each constructor's full dependent signature MUST be carried on the chain, encoded per
[ACP-A-15](#a5-the-proposition-language), so that the rules of the calculus are readable from the
record ([ACP-7-4](#72-certificates)).

**ACP-A-35.** A claim carrying a justification MUST be a `reasoning:ReasoningSentence`, requiring the
proposition, the justification term, and the certificate. The implementation MUST type-check the
certificate against the proposition at commit, and MUST reject the commit if it does not check.

### A.9 Decision records

**ACP-A-36.** The vocabulary of [§9](#9-ai-decision-records) MUST be realised by the `enc:` classes
below. Each authority and reason enumeration MUST be closed by an explicit value restriction on the
property that carries it ([ACP-3-8](#32-the-metamodel-classes-properties-inductive-types)).

| Concept | Class | Key properties |
|---|---|---|
| Unit of input | `enc:DiscourseUnit` | source document, section, character span |
| Unit in context | `enc:ScopedUnit` | the unit, its scope |
| Selection among readings | `enc:DecisionPoint` | the unit, the selected claim, candidate count, rationale, authority, ranked alternatives |
| Resolution of a reference | `enc:AnaphorBinding` | the unit, the hole, the accepted antecedent, the surface form, authority, confidence |
| Omission | `enc:CutItem` | the unit, the reason class |
| Landed claim | `enc:EncodedClaim` | a `reflection:DerivedResource` carrying the proposition |

**ACP-A-37.** The selection authorities MUST be exactly: a human-supplied pin; the reading ranker; and
the case where a single candidate survived and no choice existed.

**ACP-A-38.** The binding authorities MUST be exactly: the deterministic recency ordering; the live
proposer; and a replayed recording.

**ACP-A-39.** The omission reason classes MUST be exactly: out of scope; vocabulary gap; grammar gap;
unresolved selection; and unresolved reference ([ACP-9-7](#94-record-completeness)).

**ACP-A-40.** The reference-resolution step MUST be vetoed ([ACP-9-4](#92-vetoed-and-unvetoed-choices)):
a proposed antecedent that does not type-check against the hole's declared restriction MUST be
rejected, and rejection MUST remove the reading rather than substituting another antecedent. The
selection step is unvetoed and MUST be recorded as such — every candidate reading type-checks by
construction, so no mechanical check discriminates among them.

### A.10 Requirement realisation

| Abstract requirement | Realised by |
|---|---|
| [ACP-4-1](#41-requirements) canonical form | [ACP-A-5](#a2-canonical-form), [ACP-A-7](#a2-canonical-form) |
| [ACP-4-2](#41-requirements) content address | [ACP-A-9](#a3-content-address) |
| [ACP-4-3](#41-requirements) position address | [ACP-A-12](#a4-position-address) |
| [ACP-4-4](#41-requirements) order independence | [ACP-A-13](#a4-position-address) |
| [ACP-4-5](#41-requirements) domain separation | [ACP-A-9](#a3-content-address), [ACP-A-12](#a4-position-address) |
| [ACP-6-1](#62-non-forgeability) non-forgeability | [ACP-A-24](#a7-grades-traces-and-witnesses) |
| [ACP-6-3](#62-non-forgeability) readable from record | [ACP-A-24](#a7-grades-traces-and-witnesses) |
| [ACP-6-5](#63-witness-identity) proposition binding | [ACP-A-19](#a6-proposition-binding-and-α-canonicalization) |
| [ACP-6-6](#63-witness-identity) binder insensitivity | [ACP-A-20](#a6-proposition-binding-and-α-canonicalization) |
| [ACP-6-7](#64-which-proposition-a-witness-attests) resolution order | [ACP-A-28](#a7-grades-traces-and-witnesses) |
| [ACP-7-1](#71-justification-terms) term language | [ACP-A-29](#a8-justification-terms-and-certificates) |
| [ACP-7-4](#72-certificates) calculus readable | [ACP-A-34](#a8-justification-terms-and-certificates) |
| [ACP-7-9](#73-no-implication-introduction) no implication introduction | [ACP-A-33](#a8-justification-terms-and-certificates) |
| [ACP-8-7](#81-what-a-binding-must-supply) atomic propositions uninhabited | [ACP-A-17](#a5-the-proposition-language) |
| [ACP-9-3](#91-what-must-be-recorded) closed authorities | [ACP-A-37](#a9-decision-records), [ACP-A-38](#a9-decision-records) |
| [ACP-9-7](#94-record-completeness) reason classes | [ACP-A-39](#a9-decision-records) |

Requirements not listed are realised directly by the abstract core and need no binding-specific
statement.

---

## Appendix B. Relation to prior art *(informative)*

> **UNVERIFIED — do not rely on this section.** The editors have not completed a primary-source pass
> over the specifications named below, and the current state of related standards work has not been
> confirmed. This section records what must be checked, not conclusions.
>
> The comparison to make, for each: what it identifies, what it signs or hashes, whether its
> provenance assertions are producer-writable, and whether it has a notion of a claim being *checked*
> rather than *stated*.
>
> - **W3C PROV** — the existing W3C provenance recommendation, and the first comparison any reviewer
>   will ask for. The question to answer precisely is whether PROV's activity/entity/agent model can
>   express the Declared/Observed/Derived/Verified distinction, and whether anything in PROV
>   corresponds to a non-forgeable witness. The editors' expectation is that PROV describes provenance
>   and this specification constrains who may write it, but that must be argued from the text.
> - **Verifiable Credentials Data Model** — the closest existing model of cryptographically-anchored
>   claims with issuers. Relevant to [§12](#12-out-of-scope-authenticity-and-attribution).
> - **C2PA** — content provenance with cryptographic manifests, and the closest neighbour on
>   media-provenance for AI-generated content.
> - **in-toto / SLSA** — supply-chain attestation. SLSA's provenance thesis is close to this
>   specification's: an attestation about how an artifact was built, produced by the build system
>   rather than asserted by the publisher. The relationship between SLSA's trusted-builder model and
>   [ACP-6-1](#62-non-forgeability) is the most substantive comparison in this list.
> - **RO-Crate** — research-object packaging; the closest neighbour on describing a scientific
>   workflow's outputs as a unit.

---

## Appendix C. Test vectors

**Status: not yet generated.** Vectors MUST be produced by running a conforming implementation and
recording its output. This appendix specifies which vectors are required; it does not contain
hand-computed values, and no value should be added here that was not produced by an implementation.

### C.1 Required vectors

| # | Exercises | Input | Expected |
|---|---|---|---|
| 1 | [ACP-A-9](#a3-content-address) | A layer with one resource, no tombstones | Content address |
| 2 | [ACP-A-10](#a3-content-address) | The same resources, one tombstone added | A **different** content address |
| 3 | [ACP-A-12](#a4-position-address) | A layer with one parent | Position address |
| 4 | [ACP-A-13](#a4-position-address) | A merge layer, parents presented in both orders | The **same** position address both ways |
| 5 | [ACP-A-19](#a6-proposition-binding-and-α-canonicalization) | A closed proposition | Proposition binding |
| 6 | [ACP-A-20](#a6-proposition-binding-and-α-canonicalization) | The same proposition with binders renamed | The **same** binding as #5 |
| 7 | [ACP-6-5](#63-witness-identity) | A proposition and its negation | **Different** bindings |
| 8 | [ACP-6-1](#62-non-forgeability) | A record whose author supplies a witness | Rejected |
| 9 | [ACP-6-7](#64-which-proposition-a-witness-attests) | A resource with no canonical proposition | Binding equals that of `Asserts(iri)` |
| 10 | [ACP-A-21](#a6-proposition-binding-and-α-canonicalization) | A proposition written folded, checked expanded | Emit and check sides agree |
| 11 | [ACP-7-9](#73-no-implication-introduction) | An attempt to certify an implication without grounding it | Rejected |
| 12 | [ACP-9-10](#95-replay) | A recorded decision replayed under changed context | A counted miss, not a reuse |
| 13 | [ACP-9-6](#94-record-completeness) | A document with unprocessable units | Every unit present, each with a reason class |

### C.2 Generation

Vectors 1 through 11 exercise the kernel and require no corpus. Vectors 12 and 13 exercise an
Encoding Pipeline and require a lexicon-backed store.

> *(informative)* Vectors 12 and 13 could not be generated on the machine this draft was written on:
> the store snapshot the pipeline requires was not present, and the affected tests skip rather than
> fail when it is absent. Generating them requires a machine holding the corresponding snapshot.
> Vectors 1–11 have no such dependency and are the ones to produce first.

---

## References

**[RFC 2119]** S. Bradner. *Key words for use in RFCs to Indicate Requirement Levels.* BCP 14, RFC 2119, March 1997.

**[RFC 8174]** B. Leiba. *Ambiguity of Uppercase vs Lowercase in RFC 2119 Key Words.* BCP 14, RFC 8174, May 2017.

**[RFC 8785]** A. Rundgren, B. Jordan, S. Erdtman. *JSON Canonicalization Scheme (JCS).* RFC 8785, June 2020.

> *Further references — justification logic, the type-theoretic foundations, and the prior art of
> Appendix B — are added with the sections that cite them.*
