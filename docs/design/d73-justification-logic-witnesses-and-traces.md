# D73 — Justification logic: proof polynomials, witnesses, and the epistemic lattice

*Status: design memo · `2026-08-21`. **Supersedes [D39](d39-justification-logic.md).***

*Reference: Artemov & Fitting, *Justification Logic*, 2020 (`references/publications/justification-logic-artemov-fitting-2020.txt`).
Depends on D46 (Prop + proof irrelevance), D47 (the TypeExpr codec), D48 (indexed inductive families),
[D49](d49-chainwitness-machinery.md) (witness machinery), [D6b](d6b-reasoning-trace-schema.md) (the epistemic cluster),
[D68](../notes/d68-claim-kinds.md) (the two-axis claim), [D72](d72-declaration-provenance.md) (agent vs warrant).*

*Open defects this document explains: [#137](https://github.com/eigenius/eigenius/issues/137),
[#159](https://github.com/eigenius/eigenius/issues/159), [#175](https://github.com/eigenius/eigenius/issues/175),
[#191](https://github.com/eigenius/eigenius/issues/191).*

---

## 0. What changes from D39

D39 got the hard part right. `reasoning:JustifiedBy(j, P)` **is** Artemov's `t:F` — not an analogy; the constructors
are his axioms, correctly typed as an indexed family whose conclusions the checker derives. That stands, along with
the four grounding constructors, the kernel-internal witness model, and the institution packaging.

One thing is wrong, and it is structural rather than a detail.

**D39 §8 collapses the justification term into a four-valued scalar.** D39 §1 rejects modal epistemic logic on the
grounds that `K(A → B) → (KA → KB)` *"does not specify dependencies"*. §8 then takes the term that does specify them
and projects it onto `Declared | Observed | Derived | Verified`. That is the same collapse, one level up, performed on
the object the logic exists to preserve.

**This document's central commitment:** the justification term — Artemov's **proof polynomial** — is the primitive.
It is retained whole, its type is the proposition it justifies, and every epistemic summary is a *query over it*
rather than a value stored beside it.

Everything else here follows from that, or repairs something the collapse concealed.

## 1. The polynomial is the primitive

Artemov's own term (p. 4869): terms built from constants and variables by application `·` and sum `+` are **proof
polynomials**. `JustificationTerm` is exactly that algebra, and `JustifiedBy : JustificationTerm -> Prop -> Type 0`
types it: the polynomial's type *is* the associated proposition.

So the chain already carries the right object. §8 of D39 adds a projection on top of it, stores the projection, and
invites consumers to read the projection instead of the term.

### 1.1 The collapse produced a wrong rule

D39 §8:

```
App(j1, j2) | Sum(j1, j2):
  if category(j1) = Verified and category(j2) = Verified: Verified
  else: Derived
```

`App` is conjunctive — Artemov's `s:(A→B) → (t:A → [s·t]:B)` needs both operands. `Sum` is **disjunctive**:

```
sum_l : ∀(P, j1, j2) => JustifiedBy(j1, P) -> JustifiedBy(Sum(j1, j2), P)
sum_r : ∀(P, j1, j2) => JustifiedBy(j2, P) -> JustifiedBy(Sum(j1, j2), P)
```

In `sum_l`, `j2` is universally quantified and **completely unconstrained** — it need not justify anything, and need
not be related to `P` at all. Yet §8 grades the sum by both operands. Consequences:

- `Sum(VerifiedEvidence(a), DeclaredEvidence(b))`, built by `sum_l` and genuinely verified via `a`, grades `Derived`.
- The grade depends on a term that carries no weight in the derivation.

This is not a rounding error in a summary. Once you are folding a number over a tree, `App` and `Sum` have the same
shape, and the fact that one is conjunctive and the other disjunctive stops being visible. **Keeping the polynomial
makes the distinction unloseable, because it is the distinction between two constructors.**

### 1.2 Category becomes a query

Different consumers want different projections, and a scalar answers only one of them:

| question | projection over the polynomial |
|---|---|
| is this fully verified? | every leaf is `VerifiedEvidence` on some spanning sub-polynomial |
| what does this rest on that nobody proved? | the set of `DeclaredEvidence` leaves |
| which agents are we trusting? | `declared_by` of each `DeclaredEvidence` leaf |
| which measurements? | the `ObservedEvidence` leaves |
| would this survive losing instrument X? | recompute over the polynomial with X's leaves removed |

The last one is the point. A stored category cannot answer a counterfactual; a retained polynomial can. This is what
justification logic buys over modal epistemic logic, and D39 §8 spends it.

**`reflection:epistemic_status` on a reasoning sentence is therefore denormalization** — a cached projection that can
drift from the term it summarizes. It may be kept as a materialized query result; it must not be the source of truth.

One distinction to preserve: the four **epistemic resource classes** are not the category-of-a-term.
`DeclaredResource requires declared_by` is a well-formedness rule about a resource and its trace, and it stands
unchanged. What is withdrawn is the collapse of a *justification term* to a scalar.

## 2. The typing of witnesses

A witness is an inhabitant of `JustifiedBy(j, P)`. Four requirements, and until this week two of them failed.

| # | requirement | status |
|---|---|---|
| (a) | the type is **expressible** in the surface language | **BROKEN** — §2.1 |
| (b) | different `P` gives a **different type** | fixed, #137 |
| (c) | `P` is a **proposition** | fixed, #175 + #191 |
| (d) | conclusions are **derived by rules**, not asserted | already true |

Losing (b) is precisely the collapse of Artemov's (1.1) into (1.2): with indices ignored in conversion,
`JustifiedBy(j, P)` and `JustifiedBy(j, Q)` are the same type, so a certificate for one claim discharges another.

(b) and (c) interact, and neither closes the hole alone — measured on branch: #175's gate is defeated by #191's arm via
`Exp::Ann` (which returns its ascription as the inferred type), while #191 alone leaves a chain with a non-proposition
`canonical_proposition` and no citing sentence committing clean.

### 2.1 (a): the type cannot be written

Index types of an ESL-declared inductive lower to `EigonClass`, while their inhabitants are `InductiveType` values. So
`reasoning:JustifiedBy(j, P)` written as a type fails `check_type` with `InductiveType(…) ≠ EigonClass(…)`.

It is invisible today only because every node of a `reasoning:certificate` is a **constructor**, and the constructor
path builds the expected type from the declaration rather than checking index arguments against declared index types.

**The one relation carrying the platform's guarantee is the one relation whose type the surface language cannot
express** — which is why the conversion rule protecting it went essentially unexercised, and why #137 survived.
Fixing (a) is a prerequisite for testing any of this at the ESL level.

## 3. The Constant Specification

Artemov's CS is a set of pairings `c:A` — constants justifying axioms — and it is what makes internalization work
(Thm 2.14). The four grounding constructors play that role here, each admitted only when the chain carries the pairing
as a `ChainWitness`.

### 3.1 Leaves are where the chain stops explaining itself

> The leaves of a proof polynomial are exactly the points where the chain stops explaining itself. Each must name
> someone or something that takes responsibility.

For a **declared** leaf that responsibility is an **agent**. The structure already encodes it: a Declared witness is
admitted from a `DeclarationTrace`, which *requires* `resource`, `declared_by` and `timestamp`. So the leaf denotes a
**speech act** — this agent asserted this proposition at this time.

**[D72](d72-declaration-provenance.md) made that recoverable for the first time.** Before it, `declared_by` was an
unvalidated string; on the WRN chain 74 of them were the literal `"esl-compiler"`, so every `DeclaredEvidence` leaf
bottomed out in a name for the compiler. Rule 8 and Rule 22 now force it to resolve to a `reflection:Agent` present in
the chain. That work was done as provenance hygiene; it was repairing the base case of this logic.

The count of `DeclaredEvidence` leaves under a conclusion is therefore a real measure: **how much of this is still
assumed, and by whom.**

### 3.2 Our CS is not axiomatically appropriate, so internalization is restated — and wanted

In Artemov, constants justify *axioms*, and Thm 2.14 requires the CS to be **axiomatically appropriate**. Ours is
different in kind: a grounding constant is admitted for whatever proposition sits on a committed resource, which is an
arbitrary domain claim, not an axiom. So **internalization in Artemov's sense cannot hold** — nothing guarantees every
axiom has a constant. D39 §10 notes the difference without drawing this consequence.

**Decision: we want the chain analogue, scoped to compositions.**

> **Chain internalization.** If the chain *derives* or *verifies* `P`, a chain-resident witness pairs a term with `P`.

Scoped deliberately. `Declared` and `Observed` are the **assumed base** — the leaves of §3.1, where the chain stops
explaining itself. Their witnesses *are* the assumption, and there is no prior act to internalize. `Derived` and
`Verified` are the cases where the chain actually did something, and an act that establishes something should leave a
citable reason behind.

Why it matters: **a proposition nobody can cite is a dead end.** Witnesses are what make a fact usable as a premise, so
an establishing act that emits no witness forces the next agent to re-assert the fact as a fresh `Declared` leaf —
converting something the system *knew* into something it merely *assumes*.

The invariant carries its own escape hatch, because §3.3 shows not every derivation has a proposition to internalize:

> Every `Derived` or `Verified` resource either carries a witness for its stated proposition, or is explicitly marked
> **provenance-only** — derived in fact, uncitable as a reason.

That keeps the property checkable and makes the uninternalized set countable. Known violations today:
`VerificationTrace` emits nothing (§4.1); D6b hedges the same way about EigenQL `FIBER … INTO` commits and comorphism
reify outputs. Each is a place where the chain plausibly knows something it cannot cite.

### 3.3 What a derivation may witness is bounded by its specification

A program is a computation `f : I → O`. Applying it establishes `f(i) : O` — application at the **type** level. That is
a real derivation, but its content is *inhabitation*: this artifact is a well-formed `O`, produced from `i` by `f`. As
a proposition it is about the **artifact**, close to `Asserts(output_iri)` plus provenance. Thin, and correctly thin.

EigenTT terms are the other case. Where `P : A -> Prop` is applied to `a : A`, the result `P(a)` is a genuine
proposition, and `App` / `spec_str` do real justification work. Institution inference rules live here.

Two consequences.

**`DerivedEvidence(iri)` is correctly a constant, not a polynomial.** Where the specification is a *type* rather than a
*Prop*, there is no propositional composition to record. The term algebra is also right to have **no constructor for
"program applied to input"** — program application is not a justification step. (An earlier draft of this document
argued the opposite; withdrawn.)

**The rule:**

> A derivation may witness only a proposition its specification entails. For `f : I -> O` that is an inhabitation fact
> about the output. A domain proposition requires either a Prop-valued rule, or a declared leaf naming who asserts it.

The live violation is `enc:EncodedClaim`. It *requires* `canonical_proposition`, is a `DerivedResource`, and is produced
by the parser. What the parser establishes is *"this text parses to this well-typed term"*. What the claim carries is a
domain proposition about the world. `IsDerivedAs(iri, P)` is admitted for a `P` the program never established, and a
certificate can cite it. §6 is the resolution.

## 4. The four grounding families and their traces

| class | trace | what it records | witness admitted from |
|---|---|---|---|
| `DeclaredResource` | `DeclarationTrace` | **who** asserted it + when | the trace |
| `ObservedResource` | `ObservationTrace` | **where** it came from | the trace |
| `DerivedResource` | `ProgramTrace` | **which program**, from which inputs | the trace |
| `VerifiedResource` | `VerificationTrace` | `proof_system`, `proof_term`, `derivation_trace` | **nothing** — §4.1 |

For the first three the pairing is an *assumption about the chain*, discharged by the resource existing with its
trace. That is legitimate and matches Artemov: a CS is assumed, not proved.

### 4.1 Verified: two crossed notions, and a designed-but-unbuilt path

`trace_category` has arms for `DeclarationTrace`, `ObservationTrace` and `ProgramTrace`. **There is no arm for
`VerificationTrace`.** A resource carrying an actual Lean proof therefore emits no witness and cannot be cited.

What *does* emit a Verified witness is `emit_from_reasoning_sentence` — a `ReasoningSentence` whose `JustifiedBy`
certificate type-checked. So two different things are both called Verified:

| | means | witness? |
|---|---|---|
| `reflection:VerifiedResource` | an external prover's proof blob is attached | **none** |
| `IsVerifiedAs` as emitted today | the kernel type-checked a certificate | yes |

The second is sound but is a different claim — the kernel checked a derivation, not an external prover.

**D39 §10's factivity sentence describes the first path**: *"`VerifiedEvidence`-grounded justifications imply truth
(the Lean checker validated the proof, so the proposition holds)."* That path emits nothing. The sentence is not
merely unearned; it describes a route that does not exist.

**The fix is designed and unbuilt.** [D49](d49-chainwitness-machinery.md) §7 specifies it: a Lean → Reasoning
comorphism reifies a `reasoning:VerifiedPropositionView` carrying `canonical_proposition` — the EigenTT-form
proposition obtained by inverting D30's translation — and the witness emitter reads it through the same uniform path
as the other three families, keyed on the source `VerifiedResource`'s IRI. The kernel comment at
`witness_index.rs:168` says exactly this: *"becomes a fourth arm when that view exists."*

So [#159](https://github.com/eigenius/eigenius/issues/159) is **D49 §7 not implemented**, not an open design question.

### 4.2 Factivity is relative, and should say so

Even with D49 §7 built, "`VerifiedEvidence` implies truth" is factive **relative to** trusting the external prover's
kernel and D30's translation. D39 states it unconditionally. This is the one place the platform's guarantees bottom
out in trusting something outside it, and the document should name that rather than imply absolute factivity.

## 5. Warrants are proto-justifications

D72 split `declared_by` (who asserted) from `warranted_by` (what grounds it). The warrant axis is the informal
precursor of a justification term, and the bridge is concrete:

> A warrant resource that acquires a `canonical_proposition` becomes citable as `DeclaredEvidence(iri)`.

Nothing else has to change. Which makes the 48 warrant stubs D72 minted — `wrn:warrant_selective_essentiality_criterion`
and siblings, deliberately content-free — the **proto-axioms of the WRN encoding**. Filling them in is exactly the work
of turning that paper's informal criteria into citable declared evidence.

That also states the migration in the polynomial's own terms: **formalizing a warrant turns a leaf into an interior
node.** The chain explains one step more of itself, and the count in §3.1 goes down.

Consequence for vocabulary: `warranted_by` must not acquire semantics that compete with `JustifiedBy`. A claim
carrying both is one formalized reason and one not-yet.

## 6. Boundary: formulation is not justification

*Outside the scope of justification logic. Recorded here because it decides which epistemic class a pipeline output
takes, and §3.3 leaves that question open.*

The parsing pipeline is a **formulation instrument**. It produces a well-formed proposition and a fidelity record. It
does not produce a warrant, because **fidelity is not covered by this document** — "this encoding is faithful to what
the author wrote" is its own proposition, D61's subject, and unbuilt.

Since the parse cannot warrant fidelity, the only honest status for its output is **Declared**: some agent takes
responsibility for the proposition. Which agent depends on the use, and the two uses differ *only* in that:

| use | asserted by |
|---|---|
| encoding a source document | the document's authors — e.g. `wrn:authors` |
| an agent formulating its own claim | the agent running the pipeline |

In both, the parse contributes **form and fidelity, never warrant**. The `ProgramTrace` stays attached to the encoding
artifact and witnesses the inhabitation fact of §3.3; the claim cites an agent.

D71's architecture already has the joint: generation is decoupled from commitment, and the notebook's `land` flag is
the agent's act of taking responsibility — the moment a formulation becomes an assertion.

**This supersedes the Stage-3 settlement of `2026-08-10`** ("parsed sentences land Derived; the Declared cluster
reserved for curator-pinned rules"). That split the world on *parsed vs curated*; the operative axis is **who asserts**.
D72 supplies what makes the change possible: `wrn:authors` is a resolvable `reflection:Agent`, so a claim from that
paper can cite the people who made it rather than the program that transcribed it.

Three propositions remain distinct and must not collapse into one witness:

1. *this text parses to this well-typed term* — the artifact fact, witnessed by the `ProgramTrace` (§3.3)
2. *this encoding is faithful to what the author wrote* — D61, unwarranted today
3. *what the author wrote is true* — never established by any of the above; only ever declared, by a named agent

## 7. Two lattices, deliberately independent

**The epistemic axis** (`reflection`) answers *where knowledge came from* and selects the grounding constructor.

**The discourse axis** (`encoding`) answers *what kind of assertion this is* — `enc:Claim` and its closed kinds. Its
own description says it "names the root the reflection: source lattice deliberately lacks, at the enc: level where
discourse needs it": the resource a demonstrative («these findings») can bind, whatever its epistemic source.

`enc:EncodedClaim : reflection:DerivedResource` sits on both — Derived by construction, carrying its discourse kind as
a second `is_a` (D68 §2). The axes are orthogonal and must stay so: a Finding can be Declared, Derived or Verified,
and the discourse kind says nothing about the warrant.

## 8. Invariants

1. The justification term is retained whole; every epistemic summary is a query over it. (§1)
2. `JustifiedBy(j, P)` and `JustifiedBy(j, Q)` are the same type only when `P` and `Q` are convertible. (#137)
3. Every proposition slot holds a `Prop`. (#175, #191)
4. The type is writable in the surface language. (§2.1, unfixed)
5. A constructor's conclusion is computed by the checker from its arguments. (already true)
6. Every `DeclaredEvidence` leaf resolves to a `reflection:Agent`. (D72)
7. `IsVerifiedAs(iri, P)` holds only when the attached proof's statement translates to `P`. (§4.1, unfixed)
8. The epistemic and discourse axes are independent. (§6)

## 9. What D39 said that survives

Carried forward unchanged: the motivation (§1), the term algebra (§3), the Reasoning institution and
`ReasoningSentence` (§4), the three-layer constraint on what counts as a justification (§5), the reasoning patterns
(§6), the comorphisms (§7), belief revision via `refutes` (§9), and the non-goals (§11) — including the deliberate
exclusion of `!` (positive introspection), of a `Refutation` constructor, and of agent-extensibility of the ADT.

On `!` specifically: an earlier draft of this document argued that Verified wants it. That conflates a meta-claim
about the justification relation with an admission check that simply does not run. D39's exclusion stands.

Withdrawn: **§8 in its entirety**, and §10's factivity parenthetical.

## 10. Build order

1. **(a), the unwritable type** — prerequisite for testing anything else at the ESL level.
2. **D49 §7**, the `VerifiedPropositionView` comorphism — closes #159 and makes `VerifiedEvidence` mean what D39 said.
3. **Withdraw §8's stored category**; expose the projections of §1.2 as queries over the retained term.
4. **Warrant formalization** as an ongoing activity, measured by §3.1's leaf count.

Steps 1 and 2 are independent. Step 3 depends on nothing but is a vocabulary change with consumers.

## 11. Open questions

1. **Is chain internalization (§3.2) a goal?** If yes it is a gate on commit paths that establish propositions without
   emitting a witness. If no, say so, because the Artemov reading invites the assumption that it holds.
2. **Where does the Lean/EigenTT statement comparison happen**, and is D30's translation trusted or checked? D49 §7
   specifies inverting the forward translation; whether the inverse is verified or assumed is unsettled.
3. **Should `spec_str` generalize beyond `core:string`?** Monomorphic today; numeric and structural specialization
   were deferred to the measurement-statistics institution.
4. **Does `reflection:epistemic_status` survive as a materialized query result**, or is it removed? §1.2 says it must
   not be the source of truth; it does not say it must not exist.
