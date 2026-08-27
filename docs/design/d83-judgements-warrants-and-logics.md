# D83 — Judgements, warrants, and logics

**Status: design. No code yet.** The target shape for how this system records *why* something is
believed, to be built by replacement rather than migration.

**Self-contained.** Everything needed to read this is defined here or anchored in §9's references.
Two companion documents exist internally — a description of the current implementation, and the
derivation record showing how this shape was reached, including readings tried and withdrawn — but
neither is required, and where they differ from this document, this one is current.

**The thesis.** Two things have been conflated: a **proof** that `P` holds, and a **record of why**
`P` is believed. Both are wanted and they obey different rules. The design separates them, gives each
a checkable form, and makes every epistemic grade a **computed function of what the system holds**
rather than a label anyone applies.

---

## Context: the system this describes

Enough to read the rest; no further familiarity is assumed.

**Eigenius is a typed knowledge graph.** Its store is an **immutable chain of layers**, each layer a
set of **resources** — records with a URI identity, a set of classes (`is_a`), and typed properties.
A layer may declare classes and properties, and a class may state which properties its instances
`requires`. Adding a layer is a **commit**, and validation runs at commit time; a rejected layer does
not land. Nothing is mutated, so a resource is identified by content and everything is auditable
after the fact.

**The kernel implements a dependent type theory** — called EigenTT here — in the Martin-Löf tradition
[MLTT]: Π and Σ types, inductive families, a universe hierarchy, and normalization-by-evaluation for
definitional equality. **`Prop` is its universe of propositions**, and under propositions-as-types a
proof of `P` is a term `t` with `t : P`.

**The two are fused**: EigenTT terms can be stored as resource property values, so a proposition is
chain data, and the type checker runs over terms the graph holds. This is what makes the questions in
this document concrete rather than architectural taste — the system already stores propositions,
already stores things that are supposed to be evidence for them, and has to decide what that evidence
licenses.

**External logics participate as institutions** (§6), in the Goguen–Burstall sense [GB92]. Two are
built: a **Lean 4 verification institution**, where Lean proof terms are committed as resources and
re-checked in-process by `nanoda_lib` [ND], a Lean kernel reimplementation; and a **statistics
institution**, which runs a declared analysis plan against committed observations.

**Not everything that produces claims is an institution.** A prose-to-proposition **encoding
pipeline** turns documents into typed propositions, and was reassigned from the institution protocol
to a *service operation* on independent grounds before this design was written. §6's criterion agrees:
the pipeline has no satisfaction relation of its own, so it is a producer of claims rather than a
logic, and §4 places its outputs — `Computed` for the deterministic parse, `Sampled` for the
LLM-mediated steps within it. This document takes that downgrade as settled and explains it, rather
than proposing it.

**The system already implements justification logic** [AF19] as chain vocabulary: an inductive family
`JustifiedBy(j, P)` whose constructors are the introduction rules, over an algebra of justification
terms with application and sum. §3 and §5 are largely about what that layer does and does not mean.

---

## 0. Glossary

Organised around the pairs this design exists to separate. Each entry says what the thing is and,
where it matters, what it is **not**.

### The proof layer

**Term** (`eigentt:Term`) — a syntax tree mirroring the kernel's `Exp`. **One** category: types,
propositions, lambdas and literals are all terms. *Not checkable on its own* — a bare `Lam` has no
inferable type, which is why a term alone can carry no obligation.

**Judgement** (`eigentt:Judgement`) — `⊢_L t : T`: a term, a type, and the **logic** in which a
checker established that the one inhabits the other. The smallest checkable unit, and the reification
of justification logic's `!` operator (§1).

**Verdict** — the tri-state answer (`Holds` / `Fails` / `Undecidable`) an institution returns about a
subject. **An assertion, backed by the institution's authority**, participating in the commit
protocol. *Not a judgement*: it carries no term, so nothing can re-check it, and it is therefore
trusted rather than verified. Lean returns both a verdict and a judgement; statistics returns only a
verdict. **This is why a verdict never earns `Verified` and a judgement does.**

**Logic** — a system with its own notion of when a sentence holds. A judgement is always *in* a
logic.

**Proof system** — a logic supplying three things, named by `reflection:proof_system`:

1. **a term syntax** — proof objects as transmissible data;
2. **a formula syntax** — for what a proof proves (in EigenTT and Lean, the same syntax as 1);
3. **a decidable checking relation `⊢_L t : T`**.

That is the whole definition. Earlier drafts added *"checking must be independent of finding"* and
*"the checker may consult the theory but not the evidence"*; both are wrong. The first excludes
proofs by computation, which are proofs. The second draws a line that does not exist — **a dataset
committed as a term is a term**, and `p(D, spec) < α` is then a closed proposition the kernel can
decide by evaluation, yielding a real proof term and a real `Verified`.

**What separates statistics from Lean is therefore not the proof — it is the proposition proved.**
`refl : p(D,spec) < α` establishes a fact *about D*, not *"the effect is real"*, and no computation
closes that gap. See §4: the lattice measures the distance between the proposition established and
the proposition claimed.

The kernel is a proof system and *not* an institution.

### The justification layer

**`JustifiedBy(j, P)`** — the reification of justification logic's `j:P`, *"`j` grounds a claim to
`P`"*. An inductive whose constructors are JL's introduction rules.

**J** — basic justification logic: application, sum, grounding constants, and **no factivity**. What
`JustifiedBy` implements. LP = J + factivity + `!`.

**Factivity** — the axiom `t:F → F`. Present in LP, absent in J, and **deliberately absent here**: a
certificate records grounds and never asserts its proposition.

**Certificate** — a term inhabiting `JustifiedBy(j, P)`. A proof **about `P`'s grounds**. *Not a
proof term*: no rewriting turns it into a proof of `P`, and conflating the two is the defect §2
exists to make unstatable.

**Proof term** — a term inhabiting **`P` itself**.

**Justification term / proof polynomial** (`JustificationTerm`) — the audit structure: `App`, `Sum`,
`SpecStr`, and the four grounding leaves. Keeps *why*, where a scalar grade would keep only *what*.

**Support** — the disjunctive normal form of a justification term: the alternative minimal leaf-sets,
any one of which carries the conclusion. `App` is conjunctive, `Sum` disjunctive.

### Evidence about the chain

**Trace** — the chain-resident record of an **event**: something happened, here is what and when.

**Witness** — a proof of a proposition **about the chain**: `IsDeclaredAs(iri, P)` says the chain
contains evidence that `iri` is declared as `P`. It does **not** say `P`. *Not a trace*: a trace is
the evidence, a witness is the entitlement the evidence licenses.

**Proof constant / constant specification** — LP's mechanism for justifying an axiom by stipulation:
`c:A` is postulated because `A` cannot be proved from below. The kernel's witness admission for
`Declared` and `Observed` *is* a constant specification, and its soundness condition is that it be
*axiomatically appropriate* — constants only for what genuinely holds.

**TCB** — what must be correct for a `Verified` claim to be sound: the kernel's type checker, each
proof checker we host, each comorphism, and the constant specification for attributions. Nothing
else — not a prover, not a verdict, not a class.

### Grading

**Provenance** — how an **artifact** came to exist. Every resource has one.

**Warrant** — what evidence exists for a resource's **proposition**. Only resources carrying a
proposition have one. *Independent of provenance*: a hand-authored claim with a checked proof is
`Verified`; a machine-generated one without a proof is not.

**Verified** — a judgement `⊢_L t : P` exists in a logic we check. Entails `P`.

**Observed** — something was recorded: an instrument reading, an assay result, a model's output. The
*protocol or instrument that produced it is provenance*, not part of the warrant. Entails only that
the recording happened.

**Declared** — an agent asserted it. Entails only that they did.

**Computed** and **Sampled** are **not grounds and not stored** — they are names for whether an
application could be formed (§4). `Computed` names the shape `App(Declared(f : I → O),
Observed(input))`: a plan declared *as a function*, applied to an observed input. `Sampled` names
having no such shape available, because a stochastic protocol cannot carry that specification — so
you are left holding the observation. **You compute a function; you sample a process**, and which one
you have is structural rather than assigned.

**Reproducible** — a procedure denotes a function: same inputs, same output. **Not a separate
premise**: declaring a plan's specification as `I → O` *is* the reproducibility claim, since a
stochastic protocol cannot carry that type.

### Institutions

**Institution** — a logic with a **satisfaction relation the kernel cannot evaluate**. If the kernel
can evaluate it — that is type checking — it is not an institution.

**Satisfaction condition** — Goguen–Burstall's central axiom: `M' ⊨_Σ' σ(φ) ⟺ Mod(σ)(M') ⊨_Σ φ`.
*Truth is invariant under change of notation.* An institution in the original sense is
**model-theoretic** — signatures, sentences, models, ⊨ — with **no proofs, terms or checkers in the
definition**. The ⊢ side is separate work (Fiadeiro–Sernadas π-institutions; Meseguer's *logical
systems* = institution + entailment system + a soundness condition linking them). So *institution*
and *proof system* here are two axes rather than a subset relation, which is why statistics is one
and not the other.

**Satisfaction relation (⊨)** — an institution's own criterion for when its sentences hold:
`p < α` for statistics, `sc_S(φ) ≥ τ` for κ–τ, Lean's type theory for Lean. Having one is what makes
a warrant-producer a logic; the encoding pipeline has none and is correctly not an institution.

**Veto** — a `Fails` verdict blocking a commit. An institution may veto on its own authority
(wrong-direction-safe) but **may not verify** on its own authority.

---

## 1. Terms and judgements

**`eigentt:Term`** — one syntactic category, mirroring the kernel's `Exp`. Types, propositions,
lambdas, literals and inductive values are all terms; EigenTT has no separate type syntax and neither
does its chain mirror. (Replaces a class named for *type* expressions and documented as covering
only *"the type-level subset"*, whose 20 constructors — lambda, pair, projections, literals —
long ago stopped matching that description.)

**A term alone is syntax and cannot be checked.** A bare `Lam` has no inferable type; inference is
the wrong mode for exactly the terms that matter, which are lambdas — definitions and proofs.

**`eigentt:Judgement`** — the checkable unit:

```
data eigentt:Judgement {
    holds(logic : eigentt:Logic, term : eigentt:Term, type : eigentt:Term),
}
```

`⊢_L t : T`, reified. **This is justification logic's `!` operator.** LP's positive-introspection
axiom is `t:F → !t:(t:F)` — *"`!t` is evidence that `t` is a proof of `F`"* — which is exactly what a
proof checker returns when it runs `t` against `F`; Artemov names the operator after it. Our
`JustificationTerm` has `App` and `Sum` but no `!`, so the system today **runs the checker at commit
and discards its result**. A `Judgement` is where that result is kept.

It also places the system precisely: no `!` and no factivity is **J**; adding judgements supplies `!`
(**J4**); and `Verified` is where factivity genuinely holds because a real `t : P` exists. §4's
lattice is, in these terms, *which fragment each grounding lives in*.

The `logic` parameter is what makes it general:

| `logic` | checked by | comorphism needed |
|---|---|---|
| `eigentt` | the kernel's type checker | none — the type *is* the proposition |
| `lean4` | `nanoda_lib`, in-process | yes: Lean's `P'` → the EigenTT `P` |

A logic with no checker we hold cannot produce judgements here, whatever it produces elsewhere.

### 1.1 The EigenTT case is already built

The kernel's term language has an **annotation** constructor, `Ann(e, T)` — *"`e`, at type `T`"* —
and its typing rule is exactly the rule proposed above:

```
check_infer(Ann(e, T))  =  infer T, require it to be a Sort;
                           check e against T;          ← check mode, not infer
                           return T
```

It exists because a Curry-style lambda has no synthesizable type: `λx. x` is not inferable bare but
is inferable as `(λx. x : Prop → Prop)`. **Annotation is the bidirectional mode switch**, and it is
runtime-erased — `eval(Ann(e, _)) = eval(e)` — so normal forms never contain it and it costs nothing
semantically.

**So for EigenTT, the fix is "require annotation", not "introduce a pairing".** The per-property
exemptions §7 describes exist precisely because some values are stored as *bare* lambdas with their
type in a neighbouring field; stored annotated, the existing inference path would have checked them
with no new rule at all.

**What annotation cannot do, and what `Judgement` is therefore for:**

- **It cannot name a logic.** `Ann` is an EigenTT term, so it can express *"this EigenTT term at this
  EigenTT type"* and nothing else. A Lean proof term checked by a Lean kernel is not an EigenTT term,
  and the `logic` parameter is exactly the dimension annotation does not reach. §5's account of
  external proof systems needs it.
- **It cannot be cited.** An annotation is syntax inside a term and is erased on evaluation. §7 wants
  a committed judgement to be a *citable object* — something a witness constructor can take as an
  argument, and something that can be transported. That requires reification, which annotation is
  designed to avoid.

**`Ann` is therefore the EigenTT-internal case of `Judgement`, already implemented and already in
check mode.** The design generalises it in one dimension rather than introducing a parallel notion.

**A judgement carrying its own type is not a new storage cost.** An annotated term already holds its
type inline, and `Ann` is erased on evaluation, so whatever duplication a judgement introduces is the
duplication annotation has always had. It is not an argument against the design.

**One validation rule.** Every `Judgement`-ranged slot is decoded, its `type` checked as a type, and
its `term` **checked against that type** — check mode, never infer. This replaces the
proposition-slot special case, the separate definition-body rule, and every exemption carved out
between them. A property declares that its value is a judgement and what it means; the kernel
discharges the obligation. That is safe in a way that *declaring a grade* is not: the property states
what it must satisfy, not what it thereby receives.

## 2. The two layers

| layer | form | reading | factive |
|---|---|---|---|
| **proof** | `Judgement(L, t, T)` | `t` inhabits `T`, and we checked it | **yes** |
| **justification** | `JustifiedBy(j, P)` | `j` grounds a claim to `P` | **no** |

`JustifiedBy` is justification logic **J** — application, sum, specialisation, and the grounding
constants. It has no factivity axiom and must not acquire one: a certificate records grounds, it
never asserts its proposition.

The two compose in one direction only:

```
Judgement(eigentt, c, JustifiedBy(j, P))    -- the kernel checked the certificate
JustifiedBy(j, P)                            -- j grounds P                [object]
P                                            -- the proposition
```

**No rewriting turns that into `Judgement(eigentt, t, P)`.** Making this unstatable is the point of
the separation: a proof *about* `P`'s grounds is not a proof *of* `P`, and the two must not share a
slot, a name, or a field.

`Verified` is the case where the middle layer is absent: `Judgement(L, t, P)` directly.

## 3. Provenance and warrant are independent

Two questions, currently one enum:

| axis | question | applies to |
|---|---|---|
| **provenance** | how did this artifact come to exist? | **every** resource |
| **warrant** | what evidence exists for its proposition? | only resources carrying one |

A lexicon entry, a class declaration, an imported concept has provenance and **no warrant** — *"what
proves this?"* is not an under-answered question about it, it is not a question. The mechanical test
is whether the resource carries a proposition.

They are independent, not ordered: a hand-authored claim with a checked proof is `Verified` with
`Declared` provenance; a machine-generated claim without one is not `Verified` at all. **Authorship
is irrelevant to warrant.** A human writing an EigenTT term is exactly as good as a prover emitting
one.

### 3.1 The provenance axis is W3C PROV, and the warrant axis is deliberately outside it

**The split is not this project's invention.** W3C PROV [PROV-O] standardises the provenance axis and
stops exactly where warrant begins — `prov:Entity` is *deliberately opaque*, modelling identity and
lineage rather than semantic content. An external standard drew the same line and stayed on one side
of it, which is the strongest corroboration §3 has.

**Provenance should map down to PROV**, for interoperability on the half where a collaborator most
likely already has data:

| here | PROV |
|---|---|
| who asserted it | `prov:wasAttributedTo` (Entity → Agent) |
| a run that produced it | `prov:Activity`, with `prov:used` / `prov:wasGeneratedBy` |
| where an observation came from | `prov:hadPrimarySource` |
| **the declared procedure of §4's `Computed` row** | **`prov:Plan`**, via `prov:qualifiedAssociation` |

The last row is close enough to be worth stating: *"an activity carried out by an agent following a
plan"* is `App(Declared(proc), Observed(input))` in PROV's vocabulary. What PROV cannot add is what
§4 turns on — **nothing states the plan's logic or checks conformance to it.**

**Warrant cannot be expressed in PROV, for four structural reasons**, not vocabulary gaps:

- **PROV graphs are producer-writable.** A system can assert `prov:wasDerivedFrom` *without ever
  executing the derivation*, and the statement remains structurally valid. That is precisely the
  self-nomination §5 forbids.
- **`prov:wasDerivedFrom` covers truth-preserving and guessed derivations equally**, so it cannot
  express §4's `Computed`/`Sampled` distinction — the one that decides whether anything is entailed.
- **PROV has no proof objects and no validation relation**, so §2's proof layer has no counterpart.
- **Multiple independent justifications for one proposition** tend to duplicate entities rather than
  yield one claim with alternative support — the structure §3's justification terms exist to carry.

**And this is why §4's row is called `Computed`.** An earlier draft named it *"Derived"*, one word
away from `prov:wasDerivedFrom` and meaning something materially different — a warrant versus a
lineage relation. The collision was live: the current implementation's `Derived` grade is glossed as
*"produced by a typed program from other resources"*, which is PROV's sense, not §4's. Renaming the
row removes the trap rather than documenting it.

## 4. The warrant lattice is distance from proved to claimed

**Every warrant proves something.** The rows differ by how far that something is from what is being
claimed, and each gap is closed — if at all — by a *declared* premise:

**There are three grounds:**

| ground | what is actually established | bridge to the claim |
|---|---|---|
| **Verified** | `t : P` | **none** — the same proposition |
| **Observed** | this was recorded | whatever premise you supply |
| **Declared** | `agent a asserted P` | trust in `a` |

**`Computed` and `Sampled` are not among them.** They name *whether an application could be formed*:

| name | shape | why |
|---|---|---|
| **Computed** | `App(Declared(f : I → O), Observed(input))` | the plan is declared **as a function**, so it is an implication and can be applied |
| **Sampled** | a bare `Observed` leaf | a stochastic protocol cannot carry an `I → O` specification, so there is nothing to apply |

This is structural, not assigned. `App` requires `j₁ : (A → B)`; you can only apply a plan that *is*
an implication. That single fact carries the whole distinction, and neither name is stored anywhere.

**The grounds are named for the act that produced the evidence** — an agent *declared*, something was
*observed*. `Verified` is the deliberate exception, naming a status rather than an act, which is
defensible because it is the one ground whose bridge has length zero. The two shape-names are natural
opposites in the vocabulary itself: **you compute a function; you sample a process.**

**Why `Computed` is a shape and not a ground.** Two alternatives were considered and both fail. Making
it an opaque ground leaves the declared specification nowhere to sit, so `survives_without(input)`
answers wrongly — the defect this section exists to name. Giving it a constructor that takes the
sub-warrants collapses into `App`, which the term algebra already has; the only non-redundant part is
a check that the cited plan and inputs match a recorded run, and that belongs in validation.

**And the decisive argument: a computed conclusion does not depend on the run having happened.** `App`
needs a declared plan *as an implication* and an observed input. If the plan is a function, the output
is determined **whether or not anyone ran it** — so the run is provenance, not warrant (§3). For a
stochastic process the outcome is *not* determined by the input, so the record of the actual run
**is** the evidence: an observation, not an application. That asymmetry is the whole of the
`Computed`/`Sampled` distinction, and it is why they cannot share a ground.

**The design rule this yields, which replaces several:** *the chain records the proposition actually
established, and every bridge is a declared premise someone owns by name.*

That one rule covers what were three separate defects — a certificate proving `JustifiedBy(j,P)` and
being recorded as warrant for `P`; a statistical test proving `p < α` and being recorded as warrant
for the scientific claim; κ–τ establishing `Commits(τ,φ)` and being recorded as warrant for `φ`.
They are one error with three instances: **warranting `P` and recording it against `Q`.**

**`Sampled` is not the row without a proof.** `⊢ run r produced X` is perfectly provable, and should
be recorded. What is missing is any licence to get from there to `X` — the run is an event, and
re-running draws another sample rather than reproducing.

The current implementation states this criterion exactly right in one place — a trace class for
externally-run computations, documented as *"no `f : I -> O`, so no specification, so nothing
entailed"* — but then **tests for it with the wrong predicate**: whether the system itself initiated
the run. Initiation is a proxy for functionhood, and the two come apart on any nondeterministic call
the system *does* initiate, which is every LLM invocation in the pipeline.

**So the operative question is whether the plan carries an `I → O` specification**, which someone
declares (§4.1). When it does, the term takes the `App(Declared(f : I → O), Observed(input))` form and
the conclusion follows relative to that specification; when it does not, no application can be formed
and the outcome stays an observation. *"Is this reproducible?"* is then a question about the
polynomial — is there an application, and what is at its head — answered by the same projection
algebra as everything else. A sampled step cannot silently inherit a computed one's entailment,
because it cannot form the application at all.

**Nothing on this axis is nominated.** Every ground is a function of what the chain holds, and
`Computed`/`Sampled` are readings of term shape. No institution, trace, class or importer assigns a
warrant.

### 4.1 The plan is provenance of the observation, and its specification is the premise

**A protocol is not a warrant leaf.** How a recording came about — under which protocol, on which
instrument, in which run — is *provenance* (`prov:hadPlan`, §3.1). So a sampled outcome is a **single
ground**, `Observed(the run produced X)`, with the protocol hanging off it as provenance. An earlier
draft made the protocol a second leaf beside the observation; that was a category error, putting a
provenance relation into the justification term.

**Which means `Sampled` and `Observed` are the same ground.** An instrument reading and a model's
output are both *something was recorded*, each with a plan in its provenance. Nothing at the warrant
level separates them — §4.1a reaches this from the domain side.

**Reproducibility is not a separate premise.** For `App` to apply a plan, the plan must be an
implication: `Declared(f : I → O)`. **Declaring that specification *is* the reproducibility claim**,
because a stochastic protocol cannot carry that type — the claim and the specification are one
assertion, not two. So the support set for a computed conclusion is two leaves, not three:

```
{ Declared(f : I → O),  Observed(input) }
```

`survives_without(the specification)` still answers *"what if `f` is not a function?"*, because the
specification is a leaf.

**It cannot be inferred, only asserted.** Whether a procedure denotes a function is not decidable by
inspecting it, and for a model-mediated step it is a fact about the world rather than the code. So
someone declares it and is on record — *"temperature 0, fixed seed"* is a claim of exactly this kind,
not a property the system reads off a configuration.

**About the plan, not the code.** The parse pipeline forces this: one binary is deterministic in its
grammar and not when a model-mediated ranking step is enabled. The specification belongs to
*procedure plus configuration* — `prov:Plan`.

**And it is subject to §4 like anything else**, landing in different grounds by how it was
established: `Declared` when the plan's author asserts it (the common case); `Verified` when the plan
*is* an EigenTT term, since determinism is then definitional; and resting on `Observed` runs when it
comes from repeated executions agreeing, which is evidence about those runs and worth exactly that.

**The regress terminates** at `Declared` and `Observed`. Neither carries a bridge of its own: a
declaration establishes that an agent asserted something, an observation that a recording happened,
and any premise licensing more than that is **supplied by whoever wants the conclusion**, as a further
`Declared` leaf. So the chain of premises bottoms out in attributions — the same place §6's witness
oracle does. One termination mechanism, not two.

### 4.1a Sampling is substrate-independent

**An LLM invocation and a wet-lab assay are the same shape.** A declared protocol, a run under it, an
observed outcome, and no `f : I → O` — so nothing about the protocol tells you what the next run
gives. They differ in substrate and in nothing the warrant depends on.

**So the statistics apparatus applies to model runs unchanged.** Replication structure, the
sample-level/population-level scope marker, α, effect size — that machinery exists to get from
individual samples to a population claim, and a model run is a sample. *"The reranker improves
resolution"* needs what *"the compound is efficacious"* needs: many runs, a declared scope, a test.
One run establishes it no better than one mouse does. Measuring a pipeline component is therefore an
**institution job**, not an ad-hoc harness.

**"Temperature 0, fixed seed" is a claim, not a fact the system can read off.** It is §4.1's
`Declared(f : I → O)`, asserted by someone who is then on record — the same kind of statement as
*"this assay gives the same reading every time"*.

**The route out of `Sampled` is the same in both cases, and lands in `Computed`:**

```
App( Declared(statistical bridge),  [ Observed(run 1), …, Observed(run N) ] )
```

The test is a function of the samples, so the population claim has the `Computed` shape, with the
inductive bridge declared and owned. This is §5's three-level analysis reappearing for model
evaluation. Note the runs enter as `Observed` grounds and the protocol does not enter the term at
all — it is their provenance (§4.1).

**And the dividing line is not computational versus physical — it is whether the protocol pins a
function.** A deterministic computation does; a stochastic one does not; a physical protocol
essentially never does. Substrate drops out of the taxonomy.

One asymmetry, which does not change the structure: a computational protocol can in principle pin
everything — weights hash, prompt bytes, temperature, seed — which makes the `I → O` specification
*assertable*. A physical protocol cannot pin everything, so it is not. Same shape; different
reachability of the specification.

### 4.2 Store the relations; compute both summaries

Neither axis is a field on a resource.

**Stored:** the provenance relations of §3.1 (`wasAttributedTo`, `wasGeneratedBy`, `used`, `hadPlan`,
`hadPrimarySource`); the justification term; any committed judgements; and the declared premises those
terms cite, including §4.1's.

**Computed:** the *provenance summary* as a pattern over which relations are present — *declared* is
attributed to an agent with no generating activity, *observed* has a primary source outside the
system, *computed* was generated by an activity that used inputs and followed a plan. And the
*warrant row* as a query over the justification term, per §4's table.

**The property this buys: a summary cannot disagree with its evidence, because it is not stored.** A
resource carrying a `Verified` stamp with no proof anywhere is a state the current representation
permits and a rule must catch. Here there is no field to put it in — the bad state is not forbidden,
it is **inexpressible**.

**And warrant tracks evidence as evidence changes.** Withdraw the declared `f : I → O` and every
application headed by it loses its head — the conclusions fall back to the observations they rest on,
with no resource edited and none to hunt down. That is
epistemically right: learning that a procedure is not a function *should* downgrade what it produced.
A stored grade would need a migration, and one you would have to know to run. The same holds for a
retracted dataset or a withdrawn bridge premise.

**On cost:** summaries can be indexed, and the distinction that matters is that **an index is a cache
rebuildable from the relations, where a stamp is not**. If an index and the relations disagree, the
index is wrong by definition; if a stamp and the relations disagree, nothing says which is wrong —
which is the current situation, and why grade-writing sites with no readers went unnoticed.

**And recomputation is what creates §4.3's exposure.** Because a warrant is read at the current head
rather than frozen at commit, adding a premise later can upgrade the evidence that was used to
justify that very premise. The layer chain stratifies *citations* — a claim may only cite resources
at or below its own layer — but it does not stratify *warrants*, because warrants are not stored.
§4.3 is the condition that closes the gap.

### 4.3 Well-foundedness: a premise may not rest on what it licenses

**The exposure.** Suppose a claim `C` in layer 3 rests on runs of plan `f`, with `f : I → O` not yet
declared — so no application can be formed and `C` is a bare observation. In layer 5 someone declares
`f : I → O`, citing `C` as their evidence. `C` now supports an application, while the specification's
own support contains `C`, whose reading depends on that specification. Nothing is established, and the
layer order did not prevent it.

**The condition:** *a premise's support may not transitively include the premise.* A justification
that violates it is rejected at commit — a genuine well-formedness condition on justification terms,
of the same kind as a positivity check on an inductive declaration.

**It is cheaper than it sounds.** The general form needs a transitive expansion the support algebra
does not have — support is the normal form of *one* term and does not chase a leaf into the
justification of the claim it names. But the case that actually arises is **one step**: *does this
premise's support contain a claim whose reading depends on this premise?* Full closure is not
required to catch a premise cited by the claims it licenses.

**Unsettled, and worth trying to break before building:** whether one step is genuinely sufficient, or
whether a constructible case needs the full expansion. This is the one place in the design where the
argument is "no counterexample came to mind" rather than a construction.

**And it is vacuous exactly where justification logic requires self-reference to be legal.** A
`Declared` premise has no support to inspect — its bridge is trust in an agent, not a further
proposition — so the condition never bites on it. That is the right carve-out rather than a
convenient one: Artemov's constant specifications may be **self-referential**, `c : A(c)`, and
Kuznets showed self-referentiality is *unavoidable* for realising some S4 theorems in LP. Postulated
self-reference is sound; **derived** circularity is not, and only the latter has support to inspect.

**Mutual justification is not the escape hatch.** A mutual *inductive definition* is sound because a
least fixed point makes the block denote something — an external construction. Mutual *justification*
has no analogous construction: claims supporting each other establish nothing new. The kernel has no
mutual inductive blocks in any case, but the analogy would fail even if it did, at exactly the point
that makes mutual definition legitimate.

## 5. Institutions

**An institution is a logic with a satisfaction relation the kernel cannot evaluate.** If the kernel
can evaluate it — that is type checking — it is not an institution.

**Contributes:** vocabulary for its sentences; a decision procedure yielding a tri-state verdict;
derivation resources recording what it computed, with its analysis spec and invocation pinned;
optionally a judgement in its own logic.

**Never:** assigns a warrant, admits a witness, or asserts `Verified`.

**A verdict never earns `Verified`; a judgement does.** An institution reaches `Verified` only by
surrendering a term in a logic we hold a checker for, together with a comorphism to the EigenTT
proposition. The kernel does not trust the institution's answer — **it runs the institution's checker
itself**, in-process. Nothing else is verification, however rigorous the institution.

**Stated as trust:**

| | trust required | why it is safe |
|---|---|---|
| `Verified` | none — the kernel re-checks the term | that is the definition |
| `Computed` / `Sampled` | bounded and attributed: which institution, which invocation, which subject | recorded, so a wrong answer is traceable |
| a `Fails` verdict blocking a commit | full, on the institution's own authority | wrong-direction-safe: a bad `Fails` loses data, a bad `Holds` corrupts |

**An institution may veto on its own authority; it may not verify on its own authority.**

**`proof_system` is not a synonym for institution.** It names a logic *we hold a checker for*. The
kernel is a proof system and not an institution. A statistical institution has a real satisfaction
relation and no proof language — it expresses conclusions in *our* propositions and brings only a
procedure, so it needs no comorphism and has no judgement form. Its `p < α` is evidence bearing on
`P`, not a derivation of `P`, and recomputing it is declining to trust the first run rather than
checking a proof. **The restriction of `Verified` to EigenTT and Lean 4 is therefore forced, not
stipulated:** those are the logics in which we hold something to check.

A logic that brings a proof language we hold no checker for lands `Computed`. A proof we cannot check
is not a proof we hold.

**An institution's authority ends at its declared scope, and there are three levels, not two.**
Statistics already implements this and the vocabulary should not blur it:

| | what it is | how it is carried |
|---|---|---|
| **numerics** | `(statistic, p_value)` | audit fields on the result resource |
| **the immediate statement** | the `canonical_proposition` — a **domain** claim at a declared epistemic scope, warranted when p crosses α | an `IsComputedAs` witness, gated by the scope check |
| **the translation** | what it means for, say, the efficacy of a compound | a further claim across a **declared bridge**, owned by someone else |

The statistics institution already enforces this: before running a test it compares the claimed
proposition's head predicate against a **scope marker** (sample-level versus population-level) and
fails the gate when the replication structure does not support the scope claimed. An unmarked
predicate defaults to the more restrictive reading. **The institution already refuses to warrant past
its design** — this design generalises that, it does not introduce it.

**`Computed` attaches to the immediate statement**, as `App(Declared(analysis plan), Observed(sample
set))` — the plan's specification is §4's bridge, scope marker included. **The translation is not a
row.** It is one more `App` out, with its own declared leaf, and `is_fully_verified` reports false
because that leaf is `Declared`. Calling a bridged claim `Computed` would re-collapse it into the
opaque atom §4 exists to eliminate.

**And an institution may verify the propositions it actually proves.** With its data committed as
terms the numerics become decidable closed propositions, genuinely `Verified` — for *those*
propositions. That changes nothing above: the immediate statement is still `Computed` relative to the
plan, and the translation still needs its bridge. The same holds for κ–τ (§8).

## 6. Witnesses

`witness:Is*As` is kernel vocabulary — the four categories expressed as propositions **about the
chain**: `IsDeclaredAs(iri, P)` says the chain contains evidence that `iri` is declared as `P`. It
does not say `P`.

- **`Verified` becomes provable.** A committed `Judgement(L, t, P)` at `iri` discharges
  `IsVerifiedAs(iri, P)` through a real constructor. The kernel proves it instead of postulating it.
- **`Declared` and `Observed` stay postulated.** The kernel asserts them as proof constants under a
  constant specification, which is honest: there is nothing there to check, and that is what an
  attribution is.

**The trusted base is therefore exactly:** the kernel's type checker, each proof checker we host
(`nanoda_lib`), each comorphism, and the constant specification for attributions. Not the prover that
found a proof, not an institution's verdict, not any class membership.

**What admitting a new proof system requires**, stated in the institution literature's terms — these
are the two things that could turn a false `P'` into an accepted `P`:

1. **Soundness of its `⊢` with respect to its `⊨`** — if the checker accepts `t : P'`, then `P'`
   holds in that logic's models. Meseguer's linking condition, assumed per hosted checker and
   argued, not presumed.
2. **Its comorphism satisfies the satisfaction condition** — translation preserves truth. This is
   Goguen–Burstall's axiom, and it is the whole reason a `Verified` established elsewhere transfers
   here at all.

Hosting a checker is therefore not a packaging decision. It adds both obligations to the trusted
base, and the argument for each belongs with the institution that brings them.

## 7. What this replaces

Each item is a pattern the current implementation contains; the point is what the design does
instead, which stands without knowing the details.

- **Grades assigned by class membership, by a trace declaring its own grade, or by whichever importer
  wrote the resource.** Replaced by computation from what the system holds (§5). Nothing nominates
  its own warrant, so there is no path by which asserting a class confers evidential standing.
- **One artifact serving as proof term, derivation record and justification at once.** Replaced by
  §2's layering, which makes the substitution *unstatable* rather than merely discouraged: a proof of
  `JustifiedBy(j,P)` and a proof of `P` have different types and no rewriting between them.
- **A `Verified` resource class declared a subclass of a `Derived` resource class** (the current
  implementation's names), so that a verified thing inherits a derived thing's obligations. Replaced by §3: the two answer different questions — *what evidence exists*
  versus *what produced this artifact* — so no subsumption exists in either direction.
- **Type inference plus a hardcoded list of "these slots must be propositions" plus per-property
  exemptions for the slots inference cannot handle.** Replaced by §1's single check-mode rule.
  Inference is the wrong mode for exactly the terms that matter — a bare lambda has no inferable
  type — and the exemption list is where that broke down, patched one property at a time.
- **A protocol for institutions to supply their own witness kinds.** Unnecessary under §6: an
  institution hands over a judgement in a logic we can check, or its output is `Computed`. There is no
  third thing to extend the type system with.

**What §4's three grounds cost concretely**, since it is the largest single change here:

| | before | after |
|---|---|---|
| justification-term constructors | 7 | **6** — the derived/computed leaf is deleted |
| the witness family `Is*As` | 4 | **3** |
| the certificate's grounding constructors | 4 | **3** |
| the projection algebra's ground enum | 4 | **3** |
| what an institution emits | one atom | **a composite application** |

Plus a reseed: every existing composite-as-atom leaf becomes invalid. Under the project's
pre-production posture that is acceptable, and it is why this design is stated as replacement rather
than migration.

## 8. Worked example: the κ–τ pilot

The first outside logic proposed for the platform (arXiv:2608.08192, *rival-sensitive commitment*
over the WRN evidence graph). A design that cannot place its first external case is not finished, so
this section places it.

**Institution: yes.** `S ⊩ C_τφ ⟺ sc_S(φ) ≥ τ` is a genuine satisfaction relation and the kernel
cannot evaluate it.

**Proof system: not as proposed, and it could become one — for the wrong proposition.** With the
evidence graph and the parameters committed as terms, `sc_S(φ) ≥ τ` is decidable by evaluation, so
κ–τ could hand over a real proof term and earn `Verified` **for `Commits(τ, φ)`**. That is correct
and worth having. It does not make `φ` verified, and the pilot does not claim otherwise — which is
exactly why it is a good first external case.

**It establishes `Commits(τ, φ)`, not `φ`.** This is the design's principal demand on it, and it is
the pilot's own contribution restated — making the commitment threshold explicit. `Commits(τ, φ)` is
a *different proposition* from `φ`, so the derivation's `canonical_proposition` must be the former.
Recording it against `φ` would launder a commitment into a claim: the certificate/proof-term level
error of §2, one floor up. The gap is crossed by a **declared** bridge `Commits(τ,φ) → φ` that
someone owns by name, and the polynomial then shows that declared leaf rather than absorbing it.

**Its warrant is composite and spans two rows of §4:**

```
App( Declared(κ–τ spec: w, κ, λ, τ, ε, δ),
     [ Observed(evidence graph), Sampled(κ estimates) ] )
```

Scoring and threshold comparison are reproducible — a function of the graph and the parameters.
The **neural κ estimates are not**: declared protocol, observed outcome, nothing entailed. The
proposal anticipated this (*"each estimate committed as a resource with its own grade"*); §4 supplies
the name and stops the sampled part from inheriting the reproducible part's entailment.

**The projection then answers the pilot's own questions.** `survives_without(κ_estimate)` — does the
commitment stand without this estimate? `leaves_of(term, Sampled)` — every neural estimate the
conclusion rests on. Rival-sensitivity becomes a query over the polynomial, which works only because
the term is composite rather than the single opaque leaf an institution emits when it collapses
design and data into one node.

**It has veto power and should not use it.** A `Fails` verdict blocks a commit; below-threshold means
*"do not commit to φ"*, not *"this chain is invalid"*. So: `Holds` on `Commits(τ,φ)` above threshold,
`Undecidable` below — which commits its resources without rejecting the subject.

**What it needs from us: nothing new.** An ontology declaring its analysis-spec class with the six
parameters as `requires`; `Commits` as a chain-declared `Prop` constructor; an institution
declaration with a verifier; derivations on the existing path with `from_subject` pinning the spec.
No protocol change and no kernel change — which is the test this section exists to run.

**What this design asks of it that the present one would not:** declare which of its steps are
reproducible (§4); emit a composite justification term rather than an atom; and state `Commits(τ,φ)`
rather than `φ`. The third is the one to raise early — it is the framework declining to absorb the
commitment/truth gap, which is what the pilot is *about*, so it should read as agreement.

---

## 9. References

**Institutions and general logics**

- **[GB84]** J. A. Goguen and R. M. Burstall. *Introducing institutions.* Logics of Programs
  Workshop, LNCS 164, 1984.
- **[GB92]** J. A. Goguen and R. M. Burstall. *Institutions: abstract model theory for specification
  and programming.* Journal of the ACM 39(1), 1992. — signatures, sentences, models, `⊨`, and the
  satisfaction condition.
- **[Mes89]** J. Meseguer. *General logics.* Logic Colloquium '87, North-Holland, 1989. — a *logical
  system* as an institution plus an entailment system plus the soundness condition linking them; the
  `⊨`/`⊢` separation §5.1 turns on.
- **[Dia08]** R. Diaconescu. *Institution-independent Model Theory.* Birkhäuser, 2008.
- **[MML07]** T. Mossakowski, C. Maeder and K. Lüttich. *The heterogeneous tool set, Hets.* TACAS
  2007, LNCS 4424. — comorphisms between logics, in practice.

**Justification logic**

- **[Art95]** S. Artemov. *Operational modal logic.* Technical report, 1995. — the Logic of Proofs.
- **[Art08]** S. Artemov. *The logic of justification.* Review of Symbolic Logic 1(4), 2008.
- **[AF19]** S. Artemov and M. Fitting. *Justification Logic: Reasoning with Reasons.* Cambridge
  University Press, 2019. — the `J` / `JT` / `J4` / `LP` family, application `·`, sum `+`, the proof
  checker `!`, factivity, and constant specifications. §3's vocabulary is this book's.

**Type theory and constructive semantics**

- **[MLTT]** P. Martin-Löf. *Intuitionistic Type Theory.* Bibliopolis, 1984.
- **[TvD88]** A. S. Troelstra and D. van Dalen. *Constructivism in Mathematics: An Introduction.*
  North-Holland, 1988. — the BHK interpretation, on which §5.1 rests.
- **[BG01]** H. Barendregt and H. Geuvers. *Proof-assistants using dependent type systems.* Handbook
  of Automated Reasoning, 2001. — the de Bruijn criterion: a proof object checkable by a small,
  independent kernel.

**Provenance**

- **[PROV-O]** W3C. *PROV-O: The PROV Ontology.* W3C Recommendation, 2013.
  <https://www.w3.org/TR/prov-o/>. Entities, activities, agents, and the qualified-association
  pattern. §3.1 maps this design's provenance axis onto it, and says why the warrant axis cannot be.

**Systems**

- **[ND]** `nanoda_lib` — a Lean 4 kernel reimplementation in Rust.
  <https://github.com/ammkrn/nanoda_lib>. What makes re-checking a Lean proof term an in-process
  operation rather than a round trip to Lean.
- **[KT]** The κ–τ pilot, arXiv:2608.08192 — rival-sensitive commitment. §8's account of it comes
  from the collaborator's pilot proposal rather than from an independent reading.
