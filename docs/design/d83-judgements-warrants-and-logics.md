# D83 — Judgements, warrants, and logics

**Status: design.** No code. The target shape for the epistemic machinery, to be built by replacement
rather than migration. [D81](d81-the-epistemic-stack.md) is the description of what exists;
[D82](d82-propositions-witnesses-and-logics.md) is the derivation record — how this shape was
reached, including the readings that were tried and withdrawn. **This document states the design and
nothing else.** Where it and D82 differ, this one is current.

**The thesis.** Two things were conflated: a *proof* that `P` holds, and a *record* of why `P` is
believed. Both are wanted, they obey different rules, and every defect in D81 follows from expressing
them in one vocabulary. The design separates them, gives each a checkable form, and makes every
epistemic grade a **computed function of what exists** rather than a label anyone applies.

---

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
3. **a decidable checking relation `⊢_L t : T`**, computable from `t` and `T` alone, **against the
   theory only**.

The third is the criterion, and two clauses carry it. **Checking must be independent of finding** —
validity is established without reproducing the search that produced the object (de Bruijn). And
**the checker may consult the theory but not the evidence**: declarations, definitions and prior
judgements are fine, since any checker needs an environment; measurements, samples and external state
are not, because a relation needing an oracle is not a checking relation.

The kernel is a proof system and *not* an institution. Statistics is an institution and *not* a proof
system: it has no term syntax, borrows our `Prop`, and its check needs the dataset — so checking
would *be* recomputing, and the recomputation establishes an arithmetic fact rather than the claim.
"Is this a proof language?" and "what warrant does it yield?" are therefore one question, answered by
§4's lattice.

**Comorphism** — the translation carrying an institution's proposition into an EigenTT `Prop`
(`reasoning:VerifiedPropositionView` is the Lean one). Needed exactly when a logic brings its own
proposition language.

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

**Derived** — a **reproducible** procedure applied to inputs: `App(Declared(proc), Observed(input))`.
Entails `P` relative to the declared procedure.

**Sampled** — a declared protocol and an observed outcome, with **no `f : I → O`**. Entails nothing.
*Not a weaker `Derived`* — a different claim, and the distinction is reproducibility.

**Declared / Observed** — attribution: who asserted it, where it came from. Entails nothing.

**Reproducible** — a procedure is reproducible when it denotes a function: same inputs, same output.
Declared by the procedure, not inferred from who invoked it.

### Institutions

**Institution** — a logic with a **satisfaction relation the kernel cannot evaluate**. If the kernel
can evaluate it — that is type checking — it is not an institution.

**Satisfaction relation (⊨)** — an institution's own criterion for when its sentences hold:
`p < α` for statistics, `sc_S(φ) ≥ τ` for κ–τ, Lean's type theory for Lean. Having one is what makes
a warrant-producer a logic; the encoding pipeline has none and is correctly not an institution.

**Veto** — a `Fails` verdict blocking a commit. An institution may veto on its own authority
(wrong-direction-safe) but **may not verify** on its own authority.

---

## 1. Terms and judgements

**`eigentt:Term`** — one syntactic category, mirroring the kernel's `Exp`. Types, propositions,
lambdas, literals and inductive values are all terms; EigenTT has no separate type syntax and neither
does its chain mirror. (Replaces `eigentt:TypeExpr`, whose name and stated scope — *"the type-level
subset"* — stopped matching its 20 constructors.)

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

## 4. The warrant lattice is entailment strength

| warrant | form | entailed |
|---|---|---|
| **Verified** | `Judgement(L, t, P)` in a logic we check | `P`, deductively |
| **Derived** | `App(Declared(procedure), Observed(input))` | `P`, relative to the declared procedure |
| **Sampled** | `Declared(protocol)` + `Observed(outcome)` | **nothing** |
| **Declared** / **Observed** | attribution | nothing |

**`Derived` requires a function.** *"I ran this analysis over this dataset"* composes: the spec is a
declared implication, the data an observed antecedent, and the conclusion is Artemov application. A
statistical verdict is this shape, and so is a deterministic parse.

**`Sampled` is what remains when there is no `f : I → O`.** *"I ran this protocol — prompt, model,
parameters — and this is what I observed."* Re-running draws another sample rather than reproducing.
The protocol is declared, the outcome observed, and **nothing is entailed**; the run is an event, not
a derivation. `reflection:ExternalExecutionTrace` already states the criterion — *"no `f : I -> O`,
so no specification, so nothing entailed"* — but tests for it with *"the kernel did not initiate
it"*, which is a proxy that fails on any nondeterministic in-kernel call.

**So the test is reproducibility, and a procedure declares it.** The justification term takes the
`App` form only when the procedure is a function; otherwise the outcome is a leaf. *"Is this
reproducible?"* then becomes a question about the polynomial, answerable by the same projection
algebra as everything else, and a sampled step cannot silently inherit a derived step's entailment.

**Nothing on this axis is nominated.** Each row is a function of what the chain holds — a checked
judgement, a declared-function procedure with its operands, a protocol with an outcome, an
attribution. No institution, trace, class or importer assigns a warrant.

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
| `Derived` / `Sampled` | bounded and attributed: which institution, which invocation, which subject | recorded, so a wrong answer is traceable |
| a `Fails` verdict blocking a commit | full, on the institution's own authority | wrong-direction-safe: a bad `Fails` loses data, a bad `Holds` corrupts |

**An institution may veto on its own authority; it may not verify on its own authority.**

**`proof_system` is not a synonym for institution.** It names a logic *we hold a checker for*. The
kernel is a proof system and not an institution. A statistical institution has a real satisfaction
relation and no proof language — it expresses conclusions in *our* propositions and brings only a
procedure, so it needs no comorphism and has no judgement form. Its `p < α` is evidence bearing on
`P`, not a derivation of `P`, and recomputing it is declining to trust the first run rather than
checking a proof. **The restriction of `Verified` to EigenTT and Lean 4 is therefore forced, not
stipulated:** those are the logics in which we hold something to check.

A logic that brings a proof language we hold no checker for lands `Derived`. A proof we cannot check
is not a proof we hold.

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

## 7. What this replaces

- grades assigned by class membership, by trace declaration, or by importer — replaced by computation
  from evidence;
- a certificate serving as proof term, derivation and justification at once — replaced by §2's
  layering, which makes the substitution unstatable;
- `VerifiedResource subclass_of DerivedResource` — the two are different axes (§3), so no subsumption
  exists in either direction;
- inference plus a hardcoded slot list plus per-property exemptions — replaced by §1's single
  check-mode rule;
- an institution-supplied witness-synthesis protocol — unnecessary: an institution hands over a
  judgement or it is `Derived`.

## 8. Worked example: the κ–τ pilot

The first outside logic proposed for the platform (arXiv:2608.08192, *rival-sensitive commitment*
over the WRN evidence graph). A design that cannot place its first external case is not finished, so
this section places it.

**Institution: yes. Proof system: no.** `S ⊩ C_τφ ⟺ sc_S(φ) ≥ τ` is a genuine satisfaction relation
and the kernel cannot evaluate it. But there is no term syntax, no proof object, and the check would
need the evidence graph — so it fails §0's third condition. **Its ceiling is `Derived`
structurally**, not by policy: there is no term for any checker to check.

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

## 9. Open

1. **`Judgement` versus `Ann`.** `Ann(term, typ)` already exists inside `Term` and has the same
   shape. The difference is obligation, not structure — a `Judgement`-ranged slot must carry the
   pair. Whether that warrants a distinct inductive, or whether the rule should simply require `Ann`
   in those slots, is undecided.
2. **Duplication.** A self-contained judgement stores its type, so a proof of a large proposition
   stores that proposition twice. The alternative — naming a sibling slot — reintroduces the
   cross-slot dependency §1 removes. Unresolved.
3. **Where the reproducibility declaration lives** (§4) — on the procedure resource, on the
   institution, or on the trace kind.
4. **How provenance and warrant are carried** now that they are independent: two fields, or `is_a`
   for provenance and a property for warrant.
