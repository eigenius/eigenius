# D83 — Judgements, warrants, and logics

**Status: design. No implementation.** This document specifies how the system records the grounds on
which a proposition is asserted. It is written for replacement of the existing mechanism, not
migration.

**Self-contained.** Every concept is defined here or anchored in §9. Two internal companion documents
exist — a description of the current implementation and a derivation record — but neither is required.
Where they diverge from this document, this document is current.

**Thesis.** The existing design conflates two distinct objects: a *proof* that `P` holds, and a
*record* of the grounds on which `P` is asserted. Both are required. They obey different rules. This
design separates them, gives each a checkable form, and defines every epistemic grade as a function
computed from stored evidence rather than a label an author applies.

---

## Context

The following establishes the minimum required to read this document.

**The store.** The system is a typed knowledge graph. Its store is an immutable chain of layers. Each
layer contains **resources**: records carrying a URI identity, a set of classes (`is_a`), and typed
properties. A layer may declare classes and properties; a class may state which properties its
instances require. Adding a layer is a **commit**, and validation runs at commit time. A layer that
fails validation does not land. Nothing mutates after commit.

**The type theory.** The kernel implements a dependent type theory, EigenTT, in the Martin-Löf
tradition [MLTT]: Π and Σ types, inductive families, a universe hierarchy, and
normalization-by-evaluation for definitional equality. `Prop` is its universe of propositions. Under
propositions-as-types, a proof of `P` is a term `t` such that `t : P`.

**The fusion.** EigenTT terms are storable as resource property values. A proposition is therefore
chain data, and the type checker operates over terms the graph holds. The system already stores
propositions and stores artifacts intended as evidence for them; this document specifies what that
evidence licenses.

**External logics.** External logics participate as **institutions** (§5), in the Goguen–Burstall
sense [GB92]. Two exist:

- a **verification institution** for Lean 4, in which Lean proof terms commit as resources and the
  kernel re-checks them in-process via `nanoda_lib` [ND], a Lean kernel reimplementation;
- a **statistics institution**, which executes a declared analysis plan against committed
  observations.

A prose-to-proposition **encoding pipeline** produces claims but is not an institution. It was
reassigned from the institution protocol to a service operation on independent grounds prior to this
design. §5's criterion agrees with that reassignment.

**Justification logic.** The system implements justification logic [AF19] as chain vocabulary: an
inductive family `JustifiedBy(j, P)` whose constructors are the introduction rules, over an algebra
of justification terms supporting application and sum. §2 and §4 specify what that layer does and
does not establish.

---

## 0. Glossary

Entries are grouped by the distinction each pair exists to enforce. Each states what the term denotes
and, where a confusion is likely, what it does not.

### The proof layer

**Term** (`eigentt:Term`) — a syntax tree mirroring the kernel's `Exp`. One syntactic category: types,
propositions, lambdas, literals and inductive values are all terms. A term alone is not checkable: a
bare lambda has no inferable type.

**Judgement** (`eigentt:Judgement`) — the triple `⊢_L t : T`: a term, a type, and the **logic** in
which a checker established that the term inhabits the type. This is the smallest checkable unit and
the reification of justification logic's `!` operator (§1).

**Verdict** — the tri-state result (`Holds`, `Fails`, `Undecidable`) an institution returns for a
subject. A verdict **asserts**; it carries no term, so no party can re-check it. A judgement is an
**artifact**; it carries the term, so any party holding the checker can re-execute the check. The
verification institution returns both. The statistics institution returns a verdict only. **A verdict
does not establish `Verified`; a judgement does.**

**Logic** — a system with a criterion for when its sentences hold. Every judgement is a judgement in
a logic.

**Proof system** — a logic supplying three components, identified by `reflection:proof_system`:

1. a term syntax — proof objects as transmissible data;
2. a formula syntax — for what a proof establishes;
3. a decidable checking relation `⊢_L t : T`.

That is the complete definition. Two conditions appearing in earlier drafts — that checking be
independent of finding, and that the checker consult the theory but not the evidence — are withdrawn.
The first excludes proofs by computation, which are proofs. The second is unfounded: a dataset
committed as a term is a term, and `p(D, spec) < α` is then a closed proposition the kernel decides by
evaluation, yielding a proof term and `Verified` status.

**What separates the statistics institution from the verification institution is the proposition
proved, not the proof.** A term establishing `p(D, spec) < α` establishes a fact about `D`. It does
not establish that a treatment effect exists. See §4.

The kernel is a proof system and is not an institution.

**Comorphism** — a translation carrying an institution's proposition into an EigenTT `Prop`. Required
exactly when a logic supplies its own proposition language.

### The justification layer

**`JustifiedBy(j, P)`** — the reification of justification logic's `j:P`: *`j` grounds a claim to
`P`*. An inductive family whose constructors are the introduction rules of the logic.

**J** — basic justification logic: application, sum, grounding constants, and no factivity axiom.
`JustifiedBy` implements J. LP = J + factivity + `!`.

**Factivity** — the axiom `t:F → F`. LP includes it; J does not; this design excludes it. A
certificate records grounds and does not assert its proposition.

**Certificate** — a term inhabiting `JustifiedBy(j, P)`. It establishes a proposition **about `P`'s
grounds**. It is not a proof term: no rewriting converts it into a proof of `P`.

**Proof term** — a term inhabiting `P`.

**Justification term** — the audit structure: application, sum, specialisation, and the grounding
leaves. Artemov's *proof polynomial*. It retains the grounds, where a scalar grade retains only the
conclusion.

**Support** — the disjunctive normal form of a justification term: the set of alternative minimal
leaf-sets, any one of which carries the conclusion. Application is conjunctive; sum is disjunctive.

### Evidence about the chain

**Trace** — a chain-resident record of an event.

**Witness** — a proof of a proposition **about the chain**. `IsDeclaredAs(iri, P)` establishes that
the chain contains evidence that `iri` is declared as `P`. It does not establish `P`. A trace is the
evidence; a witness is the entitlement that evidence licenses.

**Proof constant, constant specification** — LP's mechanism for justifying an axiom by stipulation:
`c:A` is postulated because `A` is not provable from below. The kernel's witness admission for
`Declared` and `Observed` is a constant specification. Its soundness condition is that the
specification be *axiomatically appropriate*: constants issued only for propositions that hold.

**Trusted computing base** — the set of components that must be correct for a `Verified` claim to be
sound: the kernel's type checker, each hosted proof checker, each comorphism, and the constant
specification for attributions. It excludes the prover that found a proof, any institution's verdict,
and class membership.

### Grading

**Provenance** — how an artifact came to exist. Every resource has provenance.

**Warrant** — what evidence exists for a resource's proposition. Only resources carrying a
proposition have warrant. Warrant is independent of provenance: a hand-authored claim accompanied by
a checked proof is `Verified`; a machine-generated claim without one is not.

**Verified** — a judgement `⊢_L t : P` exists in a logic the system checks. Establishes `P`.

**Observed** — a recording occurred: an instrument reading, an assay result, a model output. The
protocol or instrument producing it is provenance, not warrant. Establishes only that the recording
occurred.

**Declared** — an agent asserted the proposition. Establishes only that the agent asserted it.

**Computed** and **Sampled** — not grounds, and not stored. They name whether the justification term
admits an application (§4):

- **Computed** names the shape `App(Declared(f : I → O), Observed(input))`. The plan is declared as a
  function, is therefore an implication, and applies.
- **Sampled** names the absence of that shape. A stochastic protocol cannot carry an `I → O`
  specification, so no application forms and the term is a bare `Observed` leaf.

**Reproducible** — a procedure denotes a function: identical inputs yield identical outputs. This is
not a separate premise. Declaring a plan's specification as `I → O` **is** the reproducibility claim,
because a stochastic protocol cannot carry that type.

### Institutions

**Institution** — a logic with a satisfaction relation the kernel cannot evaluate. A logic whose
satisfaction the kernel can evaluate — that is, type checking — is not an institution.

**Satisfaction relation (`⊨`)** — an institution's criterion for when its sentences hold: `p < α` for
the statistics institution, Lean's type theory for the verification institution. Possession of one
distinguishes a logic from a producer of claims. The encoding pipeline has none.

**Satisfaction condition** — Goguen–Burstall's axiom: `M' ⊨_Σ' σ(φ) ⟺ Mod(σ)(M') ⊨_Σ φ`. Truth is
invariant under change of notation. An institution in the original sense is model-theoretic —
signatures, sentences, models, `⊨` — and contains no proofs, terms or checkers. The `⊢` side is
separate work: π-institutions [FS88], and Meseguer's *logical systems* = institution + entailment
system + a soundness condition linking them [Mes89]. *Institution* and *proof system* are therefore
independent axes, not a subset relation.

**Veto** — a `Fails` verdict blocking a commit. An institution may veto on its own authority. It may
not establish `Verified` on its own authority.

---

## 1. Terms and judgements

**`eigentt:Term` replaces the current `TypeExpr` class.** It is one syntactic category mirroring the
kernel's `Exp`. Types, propositions, lambdas, literals and inductive values are all terms; EigenTT has
no separate type syntax and its chain mirror requires none. The class it replaces is named for *type*
expressions and documented as covering the type-level subset; its 20 constructors — lambda, pair,
projections, literals — do not match that description.

**A term is not checkable in isolation.** A bare lambda has no inferable type. Inference is therefore
the wrong mode for the terms that matter: definitions and proofs.

**`eigentt:Judgement` is the checkable unit:**

```
data eigentt:Judgement {
    holds(logic : eigentt:Logic, term : eigentt:Term, type : eigentt:Term),
}
```

**A judgement is justification logic's `!` operator.** LP's positive-introspection axiom is
`t:F → !t:(t:F)` — *`!t` is evidence that `t` is a proof of `F`* — which is what a proof checker
returns when it runs `t` against `F`. Artemov names the operator for that function. The current
justification-term algebra provides application and sum and omits `!`, so the system executes the
checker at commit and discards its result. A judgement retains that result.

This locates the system precisely:

| system | operators | this design |
|---|---|---|
| **J** | application, sum; no `!`, no factivity | the current state |
| **J4** | J + `!` | adding judgements |
| **JT / LP** | + factivity | `Verified` only, where a term `t : P` exists |

§4's lattice states which fragment each ground occupies.

**The `logic` parameter generalises the judgement across checkers:**

| `logic` | checker | comorphism |
|---|---|---|
| `eigentt` | the kernel's type checker | none — the type is the proposition |
| `lean4` | `nanoda_lib`, in-process | required: Lean's `P'` to the EigenTT `P` |

A logic for which the system holds no checker produces no judgements here.

### 1.1 The EigenTT case is already implemented

**The kernel's term language provides an annotation constructor `Ann(e, T)` whose typing rule is the
rule this section specifies:**

```
check_infer(Ann(e, T))  =  infer T, require a Sort;
                           check e against T;          ← check mode
                           return T
```

Annotation exists because a Curry-style lambda has no synthesizable type: `λx. x` is not inferable
bare and is inferable as `(λx. x : Prop → Prop)`. It is the bidirectional mode switch, and it is
runtime-erased — `eval(Ann(e, _)) = eval(e)` — so normal forms never contain it.

**For EigenTT the required change is to mandate annotation, not to introduce a pairing.** The
per-property exemptions §7 describes exist because certain values are stored as bare lambdas with
their type in a neighbouring field. Stored annotated, the existing inference path checks them without
a new rule.

**Annotation cannot serve two functions that `Judgement` must:**

- **It cannot name a logic.** `Ann` is an EigenTT term and expresses *this EigenTT term at this
  EigenTT type*. A Lean proof term checked by a Lean kernel is not an EigenTT term. The `logic`
  parameter is the dimension annotation does not reach, and §5 requires it.
- **It cannot be cited.** An annotation is syntax within a term and is erased on evaluation. §6
  requires a committed judgement to be an object a witness constructor accepts as an argument and
  that transports off-chain. That requires reification, which annotation is designed to avoid.

`Ann` is therefore the EigenTT-internal case of `Judgement`. The design generalises it along one
dimension rather than introducing a parallel notion.

**A judgement carrying its own type imposes no new storage cost.** An annotated term already holds its
type inline and `Ann` is erased on evaluation, so any duplication a judgement introduces is
duplication annotation already carries.

### 1.2 One validation rule replaces three mechanisms

**Every `Judgement`-ranged slot is decoded, its type checked as a type, and its term checked against
that type — in check mode, never inference.** This replaces:

- the hardcoded list of slots required to hold propositions;
- the separate rule checking a definition body against its declared type;
- every per-property exemption carved out between them.

A property declares that its value is a judgement and what the judgement means; the kernel discharges
the obligation. This is safe in a way that declaring a *grade* is not: the property states what it
must satisfy, not what it thereby receives.

---

## 2. The two layers

**The design separates a factive proof layer from a non-factive justification layer.**

| layer | form | reading | factive |
|---|---|---|---|
| proof | `Judgement(L, t, T)` | `t` inhabits `T`, and a checker verified it | yes |
| justification | `JustifiedBy(j, P)` | `j` grounds a claim to `P` | no |

**`JustifiedBy` implements J and must not acquire factivity.** A certificate records grounds; it does
not assert its proposition.

**The layers compose in one direction:**

```
Judgement(eigentt, c, JustifiedBy(j, P))    the kernel checked the certificate
JustifiedBy(j, P)                            j grounds P                [object level]
P                                            the proposition
```

**No rewriting converts `Judgement(eigentt, c, JustifiedBy(j,P))` into `Judgement(eigentt, t, P)`.**
The two have different types and the system provides no rule connecting them. Making the substitution
inexpressible is the purpose of the separation: a proof about `P`'s grounds is not a proof of `P`, and
the two must not share a slot, a field or a name.

**`Verified` is the case in which the middle layer is absent:** `Judgement(L, t, P)` directly.

---

## 3. Provenance and warrant are independent axes

**The current design encodes two independent questions in one enumeration.**

| axis | question | applies to |
|---|---|---|
| provenance | how did this artifact come to exist? | every resource |
| warrant | what evidence exists for its proposition? | only resources carrying a proposition |

**Most resources have provenance and no warrant.** A lexicon entry, a class declaration or an imported
concept carries no proposition, so *what proves this?* is not an under-answered question about it; it
is not a question. The test is mechanical: does the resource carry a proposition.

**The axes are independent, not ordered.** A hand-authored claim accompanied by a checked proof is
`Verified` with `Declared` provenance. A machine-generated claim without a proof is not `Verified`.
Authorship does not determine warrant: a human writing an EigenTT term and a prover emitting one
produce the same result.

### 3.1 The provenance axis maps to W3C PROV

**W3C PROV [PROV-O] standardises the provenance axis and terminates where warrant begins.**
`prov:Entity` is deliberately opaque, modelling identity and lineage rather than semantic content. An
external standard drew this boundary independently and remained on one side of it.

**Provenance maps down to PROV for interoperability:**

| this design | PROV |
|---|---|
| the agent that asserted a claim | `prov:wasAttributedTo` |
| a run that produced a resource | `prov:Activity`, with `prov:used` and `prov:wasGeneratedBy` |
| the origin of an observation | `prov:hadPrimarySource` |
| the declared procedure of §4's `Computed` shape | `prov:Plan`, via `prov:qualifiedAssociation` |

The final row is exact: *an activity carried out by an agent following a plan* is
`App(Declared(f : I → O), Observed(input))` in PROV's vocabulary. PROV supplies no statement of the
plan's logic and no conformance check, which is what §4 requires.

**Warrant cannot be expressed in PROV. The reasons are structural, not lexical:**

- **PROV graphs are producer-writable.** A system may assert `prov:wasDerivedFrom` without executing
  the derivation, and the statement remains structurally valid. §4 forbids exactly this.
- **`prov:wasDerivedFrom` covers truth-preserving and stochastic derivations identically**, so it
  cannot express §4's `Computed`/`Sampled` distinction.
- **PROV defines no proof objects and no validation relation**, so §2's proof layer has no counterpart.
- **Multiple independent justifications for one proposition duplicate entities** rather than yielding
  one claim with alternative support.

**§4's shape is named `Computed` rather than `Derived` for this reason.** *Derived* is one word from
`prov:wasDerivedFrom` and denotes a materially different relation — lineage rather than warrant. The
collision is live: the current implementation's `Derived` grade is documented as *produced by a typed
program from other resources*, which is PROV's sense.

---

## 4. The warrant lattice measures distance from proved to claimed

**Every warrant establishes some proposition. The grounds differ by the distance between that
proposition and the one claimed.** Each gap is closed, if at all, by a declared premise.

**Three grounds exist:**

| ground | what it establishes | bridge to the claim |
|---|---|---|
| **Verified** | `t : P` | none — the same proposition |
| **Observed** | a recording occurred | supplied by the consumer |
| **Declared** | agent `a` asserted `P` | trust in `a` |

**`Computed` and `Sampled` are not grounds. They name whether the term admits an application:**

| name | shape | condition |
|---|---|---|
| **Computed** | `App(Declared(f : I → O), Observed(input))` | the plan is declared as a function, is an implication, and applies |
| **Sampled** | a bare `Observed` leaf | a stochastic protocol cannot carry an `I → O` specification, so no application forms |

**This is structural.** Application requires `j₁ : (A → B)`. A plan is applicable exactly when it is
an implication. Neither name is stored.

**Two alternatives were considered and both fail:**

- Making `Computed` an opaque ground leaves the declared specification with no position in the term,
  so `survives_without(input)` returns an incorrect result.
- Giving it a constructor taking the sub-warrants collapses into application, which the term algebra
  provides. The only non-redundant component is a check that the cited plan and inputs correspond to
  a recorded run, which belongs in validation.

**The decisive argument: a computed conclusion does not depend on the run having occurred.**
Application requires a declared plan as an implication and an observed input. If the plan is a
function, the output is determined whether or not any party executed it — the run is provenance, not
warrant (§3). For a stochastic process the output is not determined by the input, so the record of the
run is the evidence: an observation, not an application. That asymmetry constitutes the
`Computed`/`Sampled` distinction and is why they cannot share a ground.

**The design rule, replacing several:** *the chain records the proposition actually established, and
every bridge is a declared premise attributed to an owner.*

**That rule covers three defects previously treated separately**, all instances of establishing `P`
and recording it against `Q`:

| establishes | recorded as warrant for |
|---|---|
| `JustifiedBy(j, P)` | `P` |
| `p(D, spec) < α` | a treatment effect exists |
| `Commits(τ, φ)` | `φ` |

**`Sampled` is not the ground without a proof.** `⊢ run r produced X` is provable and should be
recorded. What is absent is any licence to proceed from that to `X`.

**The current implementation states the criterion correctly in one location and tests for it with the
wrong predicate.** A trace class for externally-run computations is documented as *no `f : I → O`, so
no specification, so nothing entailed*. The test applied is whether the system initiated the run.
Initiation is a proxy for functionhood; the two diverge on any nondeterministic call the system does
initiate, which includes every model invocation in the pipeline.

**Nothing on this axis is nominated.** Each ground is a function of stored evidence, and
`Computed`/`Sampled` are readings of term shape. No institution, trace, class or importer assigns a
warrant.

### 4.1 The plan is provenance; its specification is the premise

**A protocol is not a warrant leaf.** How a recording came about — under which protocol, on which
instrument, in which run — is provenance (`prov:hadPlan`, §3.1). A sampled outcome is therefore a
single ground, `Observed(the run produced X)`, with the protocol attached as provenance. Treating the
protocol as a second leaf places a provenance relation in the justification term.

**`Sampled` and `Observed` are consequently the same ground.** An instrument reading and a model
output are both recordings, each with a plan in provenance. Nothing at the warrant level separates
them. §4.2 reaches this result from the domain side.

**Reproducibility is not a separate premise.** Application requires the plan to be an implication:
`Declared(f : I → O)`. Declaring that specification **is** the reproducibility claim, because a
stochastic protocol cannot carry that type. The support set for a computed conclusion is two leaves:

```
{ Declared(f : I → O),  Observed(input) }
```

`survives_without(the specification)` answers *what if `f` is not a function?* because the
specification is a leaf.

**The specification is asserted, not inferred.** Whether a procedure denotes a function is not
decidable by inspection, and for a model-mediated step it is a fact about the world rather than the
code. An author declares it and is attributed. *Temperature 0, fixed seed* is a claim of this kind,
not a property readable from a configuration.

**The specification attaches to the plan, not the code.** One binary is deterministic in its grammar
and nondeterministic when a model-mediated ranking step is enabled. The specification therefore
belongs to procedure plus configuration — `prov:Plan`.

**The specification is itself subject to §4**, and occupies different grounds by how it was
established:

| how established | ground |
|---|---|
| the plan's author asserts it | `Declared` — the common case |
| the plan is an EigenTT term | `Verified` — determinism is definitional |
| repeated executions agree | rests on `Observed` runs; establishes only what those runs establish |

**The regress terminates at `Declared` and `Observed`.** Neither carries a bridge of its own: a
declaration establishes that an agent asserted something, an observation that a recording occurred,
and any premise licensing more is supplied by the consumer as a further `Declared` leaf. The chain of
premises therefore terminates in attributions — the same termination as §6's witness oracle. One
mechanism, not two.

### 4.2 Sampling is substrate-independent

**A model invocation and a laboratory assay have the same warrant structure:** a declared protocol, a
run under it, an observed outcome, and no `f : I → O`. They differ in substrate and in nothing the
warrant depends on.

**The statistics institution's apparatus therefore applies to model runs without modification.**
Replication structure, the sample-level/population-level scope marker, α and effect size exist to move
from individual samples to a population claim, and a model run is a sample. *The reranker improves
resolution* requires what *the compound is efficacious* requires: replication, a declared scope, a
test. One run establishes it no better than one animal does. **Measuring a pipeline component is an
institution operation, not an ad-hoc harness.**

**The route out of `Sampled` is identical in both cases and produces the `Computed` shape:**

```
App( Declared(statistical bridge),  [ Observed(run 1), …, Observed(run N) ] )
```

The test is a function of the samples, so the population claim has the `Computed` shape with the
inductive bridge declared and attributed. The runs enter as `Observed` grounds; the protocol does not
enter the term, being their provenance (§4.1).

**The dividing line is whether the protocol pins a function, not whether execution is computational.**
A deterministic computation pins one; a stochastic computation does not; a physical protocol does not.
Substrate is not a term in the taxonomy.

**One asymmetry does not affect the structure.** A computational protocol can in principle pin every
input — weights hash, prompt bytes, temperature, seed — which makes the `I → O` specification
assertable. A physical protocol cannot pin every input, so it is not assertable. The shape is
identical; the reachability of the specification differs.

### 4.3 Storage and computation

**Neither axis is a field on a resource.**

**Stored:**

- the provenance relations of §3.1 — `wasAttributedTo`, `wasGeneratedBy`, `used`, `hadPlan`,
  `hadPrimarySource`;
- the justification term;
- committed judgements;
- the declared premises those terms cite, including §4.1's specification.

**Computed:**

- the **provenance summary**, as a pattern over which relations are present: *declared* is attributed
  to an agent with no generating activity; *observed* has a primary source outside the system;
  *computed* was generated by an activity that used inputs and followed a plan;
- the **warrant**, as a query over the justification term, per §4's tables.

**A computed summary cannot disagree with its evidence, because it is not stored.** A resource
carrying a `Verified` stamp with no proof is a state the current representation permits and a
validation rule must detect. Under this design no field holds such a stamp: the state is
inexpressible rather than prohibited.

**Warrant tracks evidence as evidence changes.** Withdrawing a declared `f : I → O` removes the head
of every application it headed; those conclusions fall back to the observations they rest on, with no
resource edited. A retracted dataset or a withdrawn bridge premise behaves identically. A stored grade
would require a migration, and one a party would have to know to run.

**Cost.** Summaries are indexable. **An index is a cache rebuildable from the relations; a stamp is
not.** If an index and the relations disagree, the index is incorrect by definition. If a stamp and
the relations disagree, nothing determines which is incorrect — the current situation, and why
grade-writing sites with no readers persisted undetected.

### 4.4 Well-foundedness

**Recomputation at head creates an exposure the layer order does not close.** A warrant is read
against the current head rather than frozen at commit, so adding a premise can upgrade the evidence
used to justify that premise:

1. Claim `C` in layer 3 rests on runs of plan `f`. No `f : I → O` is declared, so no application forms
   and `C` is a bare observation.
2. In layer 5 an author declares `f : I → O`, citing `C` as evidence.
3. `C` now supports an application, while the specification's support contains `C`, whose reading
   depends on that specification.

The layer chain stratifies **citations** — a claim may cite only resources at or below its own layer —
and does not stratify **warrants**, because warrants are not stored.

**The condition: a premise's support may not transitively include the premise.** A justification
violating it is rejected at commit. This is a well-formedness condition on justification terms, of the
same kind as a positivity check on an inductive declaration.

**The check is one step for the case that arises:** *does this premise's support contain a claim whose
reading depends on this premise?* The general form requires a transitive expansion the support algebra
does not provide — support is the normal form of one term and does not follow a leaf into the
justification of the claim it names. Full closure is not required to detect a premise cited by the
claims it licenses.

**The condition is vacuous exactly where justification logic requires self-reference to remain legal.**
A `Declared` premise has no support to inspect, its bridge being trust rather than a further
proposition, so the condition never applies to it. This carve-out is principled: Artemov's constant
specifications may be self-referential, `c : A(c)`, and self-referentiality is unavoidable for
realising certain S4 theorems in LP. Postulated self-reference is sound; derived circularity is not,
and only derived circularity has support to inspect.

**Mutual justification is not an exemption.** A mutual inductive definition is sound because a least
fixed point makes the block denote — an external construction. Mutual justification has no analogue:
claims supporting one another establish nothing. The kernel provides no mutual inductive blocks, and
the analogy would fail regardless.

**Unsettled.** Whether the one-step check is sufficient, or whether a constructible case requires the
full expansion, is not established. This is the one condition in this design supported by absence of a
counterexample rather than by construction.

---

## 5. Institutions

**An institution is a logic with a satisfaction relation the kernel cannot evaluate.** A logic whose
satisfaction the kernel can evaluate — type checking — is not an institution.

**An institution contributes:**

- vocabulary for its sentences;
- a decision procedure yielding a tri-state verdict;
- derivation resources recording what it computed, with its analysis plan and invocation identified;
- optionally, a judgement in its own logic.

**An institution does not:**

- assign a warrant — warrants are computed from stored evidence (§4.3);
- admit a witness — that is the kernel's constant specification (§6);
- establish `Verified`.

**A verdict does not establish `Verified`; a judgement does.** The verification institution
demonstrates the separation: it is an institution *and* a proof-term source, and the roles are
independent. Its verdict does not establish `Verified`; its term does. A verification institution
returning `Holds` and shipping no term produces `Computed`.

**Trust, by direction:**

| direction | trust required | why the direction is safe |
|---|---|---|
| `Verified` | none — the kernel re-checks the term | that is the definition |
| `Computed` / `Sampled` | bounded and attributed: which institution, which invocation, which subject | recorded, so an incorrect result is traceable |
| a `Fails` verdict blocking a commit | full, on the institution's authority | an incorrect `Fails` loses data; an incorrect `Holds` corrupts |

**An institution may veto on its own authority and may not establish `Verified` on its own
authority.**

**`proof_system` is not a synonym for institution.** It identifies a logic for which the system holds
a checker. The kernel is a proof system and not an institution. The statistics institution has a
satisfaction relation and no proof language: it expresses conclusions in the system's own `Prop` and
supplies only a procedure, so it requires no comorphism and has no judgement form. Its `p < α` is
evidence bearing on a claim, not a derivation of it.

**A logic supplying a proof language for which the system holds no checker produces `Computed`.** A
proof the system cannot check is not a proof the system holds.

**An institution may produce judgements for the propositions it proves.** With its data committed as
terms, the statistics institution's numerics become decidable closed propositions and are `Verified`
for those propositions. This changes nothing above: the immediate statement retains the `Computed`
shape relative to the plan, and the translation requires its bridge.

### 5.1 The authority boundary: three levels

**An institution's authority terminates at its declared scope. Three levels exist, and the vocabulary
must not merge them:**

| level | content | how carried |
|---|---|---|
| **numerics** | `(statistic, p_value)` | audit fields on the result resource |
| **immediate statement** | a domain claim at a declared epistemic scope, warranted when p crosses α | an `IsComputedAs` witness, gated by the scope check |
| **translation** | what the statement implies for, e.g., compound efficacy | a further claim across a declared bridge, owned by another party |

**The statistics institution already enforces this boundary.** Before executing a test it compares the
claimed proposition's head predicate against a scope marker — sample-level versus population-level —
and fails the gate when the replication structure does not support the scope claimed. An unmarked
predicate defaults to the more restrictive reading. This design generalises that enforcement; it does
not introduce it.

**The `Computed` shape attaches to the immediate statement**, as
`App(Declared(analysis plan), Observed(sample set))`, where the plan's specification is §4's bridge
and includes the scope marker. **The translation is not a shape.** It is a further application with
its own declared leaf, and `is_fully_verified` returns false because that leaf is `Declared`.

### 5.2 Conjecture: the proof-system boundary is the constructive/classical boundary

**Stated as a conjecture. It is consistent with every case in this document, which is the condition
under which it requires checking rather than adoption.**

**Institution theory was formulated classically** [GB92]: `⊨` relates models to sentences.
**Constructively, truth is inhabitation** — under the BHK interpretation [TvD88] and
propositions-as-types [MLTT], *`P` holds* is *`⟦P⟧` has an inhabitant*, categorically a global element
`1 → ⟦P⟧`. For the initial (term) model, `⊢` and `⊨` coincide by construction: a type is inhabited
there exactly when a term inhabits it.

**Three consequences follow, in increasing order of consequence:**

- **§5's criterion acquires a reason.** The kernel is a proof system and not an institution because
  its `⊨` *is* inhabitation, and inhabitation is what its type checker decides. It is not an
  institution because it is the term model of its own logic.
- **§6's first obligation is discharged by construction for type-theoretic checkers.** Where `⊨` is
  Tarskian, soundness of `⊢` with respect to `⊨` is a theorem requiring proof [Mes89]. Where `⊨` is
  inhabitation and the checker is a type checker, *the checker accepts `t : P'`* and *`P'` holds* are
  the same statement. **All obligation transfers to the comorphism**, which matches practice: the risk in
  admitting a Lean proof is not that Lean's kernel accepts falsehoods but that `P'` does not denote
  `P`.
- **The satisfaction condition becomes inhabitation-preservation** — `α(φ)` inhabited iff `φ`
  inhabited. This is sharper than model-preservation and closer to exhibitable, though not executable:
  the two inhabitants occupy different type theories, and the comorphism carries the proposition, not
  the proof.

**The conjecture.** A logic whose satisfaction is inhabitation supplies terms, because its
truth-witnesses are objects. A logic whose satisfaction relates sentences to something external — a
dataset, a population, the world — has no witness object to supply and can only report. If this holds,
*institution or proof system* is a question about a logic's semantics rather than about what a
component implements, and the statistics institution falls on the far side for reasons unrelated to
its implementation.

---

## 6. Witnesses and the trusted computing base

**`witness:Is*As` is kernel vocabulary: the grounds expressed as propositions about the chain.**
`IsDeclaredAs(iri, P)` establishes that the chain contains evidence that `iri` is declared as `P`. It
does not establish `P`.

| ground | status | mechanism |
|---|---|---|
| `Verified` | **provable** | a committed `Judgement(L, t, P)` at `iri` discharges `IsVerifiedAs(iri, P)` through a constructor |
| `Declared`, `Observed` | **postulated** | the kernel asserts them as proof constants under a constant specification |

**Postulation is correct for attributions.** There is nothing to check: an attribution asserts that an
agent asserted, or that a recording occurred.

**The trusted computing base is:** the kernel's type checker, each hosted proof checker, each
comorphism, and the constant specification for attributions. It excludes the prover that found a
proof, any institution's verdict, and class membership.

**Admitting a new proof system requires two arguments**, both named in the institution literature.
These are the two failure modes that convert a false `P'` into an accepted `P`:

1. **Soundness of its `⊢` with respect to its `⊨`** — if the checker accepts `t : P'`, then `P'` holds
   in that logic's models. Meseguer's linking condition [Mes89], argued per hosted checker.
2. **Satisfaction-preservation by its comorphism** — translation preserves truth. Goguen–Burstall's
   axiom [GB92], and the reason a `Verified` established elsewhere transfers.

**Hosting a checker is not a packaging decision.** It adds both obligations to the trusted computing
base, and the argument for each belongs with the institution supplying them.

---

## 7. What this design replaces

Each item names a pattern in the current implementation. The replacement stands without knowledge of
the implementation's details.

- **Grades assigned by class membership, by a trace declaring its own grade, or by the importer that
  wrote the resource.** Replaced by computation from stored evidence (§4.3). No path exists by which
  asserting a class confers evidential standing.
- **One artifact serving as proof term, derivation record and justification simultaneously.** Replaced
  by §2's layering, which renders the substitution inexpressible rather than discouraged: a proof of
  `JustifiedBy(j,P)` and a proof of `P` have different types and no rewriting between them.
- **A `Verified` resource class declared a subclass of a `Derived` resource class**, so that a verified
  resource inherits a derived resource's obligations. Replaced by §3: the two answer different
  questions, so no subsumption exists in either direction.
- **Type inference, plus a hardcoded list of slots required to hold propositions, plus per-property
  exemptions for slots inference cannot handle.** Replaced by §1.2's single check-mode rule. Inference
  is the wrong mode for the terms that matter, and the exemption list records where that failed.
- **A protocol for institutions to supply their own witness kinds.** Unnecessary under §5: an
  institution supplies a judgement in a logic the system checks, or its output is `Computed`.

**§4's three grounds impose the largest single change:**

| | current | this design |
|---|---|---|
| justification-term constructors | 7 | **6** — the derived leaf is removed |
| the witness family `Is*As` | 4 | **3** |
| the certificate's grounding constructors | 4 | **3** |
| the projection algebra's ground enumeration | 4 | **3** |
| institution output | one atom | **a composite application** |

Every existing composite-as-atom leaf becomes invalid, requiring a reseed. The project's
pre-production posture accepts this, which is why this document specifies replacement rather than
migration.

---

## 8. Worked example: the κ–τ pilot

The first external logic proposed for the platform (arXiv:2608.08192, *rival-sensitive commitment*).
This section places it in the framework; a design unable to place its first external case is
incomplete.

**Institution: yes.** `S ⊩ C_τφ ⟺ sc_S(φ) ≥ τ` is a satisfaction relation the kernel cannot evaluate.

**Proof system: not as proposed, and it could become one for a different proposition.** With the
evidence graph and parameters committed as terms, `sc_S(φ) ≥ τ` is decidable by evaluation, so the
pilot could supply a proof term and establish `Verified` **for `Commits(τ, φ)`**. That does not
establish `φ`.

**The framework's principal requirement: the pilot establishes `Commits(τ, φ)`, not `φ`.** This
restates the pilot's own contribution — making the commitment threshold explicit. `Commits(τ, φ)` is a
different proposition from `φ`, so the derivation's `canonical_proposition` is the former. Recording it
against `φ` converts a commitment into a claim: §2's level error, one level higher. The gap is crossed
by a declared bridge `Commits(τ,φ) → φ` attributed to an owner, and the justification term exhibits
that declared leaf.

**Its warrant is composite and spans §4's `Computed` shape and `Sampled` grounds:**

```
App( Declared(κ–τ spec : w, κ, λ, τ, ε, δ),
     [ Observed(evidence graph), Observed(κ estimates) ] )
```

Scoring and threshold comparison are reproducible — a function of the graph and the parameters. The
neural κ estimates are not: they are recordings under a declared protocol, so no application forms over
them and nothing is entailed about subsequent estimates. The pilot proposal anticipated this
(*each estimate committed as a resource with its own grade*); §4 supplies the vocabulary and prevents
the sampled component from inheriting the reproducible component's entailment.

**The projection answers the pilot's own questions.** `survives_without(κ_estimate)` determines
whether the commitment stands without a given estimate; `leaves_of(term, Observed)` enumerates every
estimate the conclusion rests on. Rival-sensitivity becomes a query over the justification term, which
requires the term to be composite rather than the single opaque leaf an institution emits when it
collapses plan and data into one node.

**It holds veto power and should not exercise it.** A `Fails` verdict blocks a commit; a
below-threshold score means *do not commit to φ*, not *this chain is invalid*. The correct behaviour is
`Holds` on `Commits(τ,φ)` above threshold and `Undecidable` below, which commits its resources without
rejecting the subject.

**It requires nothing new from the platform:**

- an ontology declaring its analysis-plan class with the six parameters as required properties;
- `Commits` as a chain-declared `Prop` constructor;
- an institution declaration with a verifier;
- derivations on the existing path, with the plan identified.

No protocol change and no kernel change. That is the test this section performs.

**This design imposes three requirements the current one would not:** declare which steps are
reproducible (§4.1); emit a composite justification term rather than an atom; and state
`Commits(τ,φ)` rather than `φ`. The third warrants early discussion with the collaborator — it is the
framework declining to absorb the commitment/truth gap, which is the pilot's subject.

---

## 9. References

**Institutions and general logics**

- **[GB84]** J. A. Goguen and R. M. Burstall. *Introducing institutions.* Logics of Programs Workshop,
  LNCS 164, 1984.
- **[GB92]** J. A. Goguen and R. M. Burstall. *Institutions: abstract model theory for specification
  and programming.* Journal of the ACM 39(1), 1992. Signatures, sentences, models, `⊨`, and the
  satisfaction condition.
- **[FS88]** J. Fiadeiro and A. Sernadas. *Structuring theories on consequence.* Recent Trends in Data
  Type Specification, LNCS 332, 1988. Abstract consequence relations (π-institutions).
- **[Mes89]** J. Meseguer. *General logics.* Logic Colloquium '87, North-Holland, 1989. A *logical
  system* as an institution plus an entailment system plus the soundness condition linking them; the
  `⊨`/`⊢` separation §5.2 turns on.
- **[Dia08]** R. Diaconescu. *Institution-independent Model Theory.* Birkhäuser, 2008.
- **[MML07]** T. Mossakowski, C. Maeder and K. Lüttich. *The heterogeneous tool set, Hets.* TACAS
  2007, LNCS 4424. Comorphisms between logics in practice.

**Justification logic**

- **[Art95]** S. Artemov. *Operational modal logic.* Technical report, 1995. The Logic of Proofs.
- **[Art08]** S. Artemov. *The logic of justification.* Review of Symbolic Logic 1(4), 2008.
- **[AF19]** S. Artemov and M. Fitting. *Justification Logic: Reasoning with Reasons.* Cambridge
  University Press, 2019. The J / JT / J4 / LP family, application, sum, the proof checker `!`,
  factivity, and constant specifications. §2's vocabulary follows this text.

**Type theory and constructive semantics**

- **[MLTT]** P. Martin-Löf. *Intuitionistic Type Theory.* Bibliopolis, 1984.
- **[TvD88]** A. S. Troelstra and D. van Dalen. *Constructivism in Mathematics: An Introduction.*
  North-Holland, 1988. The BHK interpretation, on which §5.2 rests.
- **[BG01]** H. Barendregt and H. Geuvers. *Proof-assistants using dependent type systems.* Handbook
  of Automated Reasoning, 2001.

**Provenance**

- **[PROV-O]** W3C. *PROV-O: The PROV Ontology.* W3C Recommendation, 2013.
  <https://www.w3.org/TR/prov-o/>. Entities, activities, agents, and the qualified-association
  pattern. §3.1 maps this design's provenance axis onto it and states why the warrant axis cannot be.

**Systems**

- **[ND]** `nanoda_lib` — a Lean 4 kernel reimplementation in Rust.
  <https://github.com/ammkrn/nanoda_lib>. Makes re-checking a Lean proof term an in-process operation.
- **[KT]** The κ–τ pilot, arXiv:2608.08192. §8's account derives from the collaborator's pilot
  proposal rather than an independent reading of the paper.
