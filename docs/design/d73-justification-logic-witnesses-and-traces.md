# D73 — Justification logic: proof polynomials, witnesses, and the epistemic lattice

*Status: design memo · `2026-08-21`. **Supersedes [D39](d39-justification-logic.md).***

*Reference: Artemov & Fitting, *Justification Logic*, 2019 (`references/publications/justification-logic-artemov-fitting-2020.txt`).
Depends on D46 (Prop + proof irrelevance), D47 (the Term codec), D48 (indexed inductive families),
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

**Built `2026-08-22` (eigenius#204)**, as `reasoning:qc_project_justification` — an OnDemand query class on the
Reasoning institution, not EigenQL. EigenQL was the natural home and cannot host it: a `JustificationTerm` is a
recursive D47 tagged-dict inside a single property value, `Clause` is `Pattern | Fiber`, patterns match triples, and
the AST has no recursion construct. The term is opaque to it.

> **Reshaped `2026-08-31` (P7).** The capability stands; its housing does not. The algebra moved to
> `kernel/src/justification/` at P6.0 — `support`, `is_fully_verified`, `leaves_of`,
> `survives_without`, `cited_iris`, called directly on a retained term — and P7 deleted the
> QueryClass with the Reasoning institution, along with `justification:ProjectionRequest` and
> `justification:Projection`, which existed only to carry that query's input and output. Every
> question §1.2 lists is answerable today; what is gone is the chain-resident request/report pair,
> which nothing ever wrote. The paragraph above still holds on why EigenQL cannot host it.

Everything above is one function plus readings of its output. A term's **support** is its disjunctive normal form —
the alternative minimal ground-sets, any one of which carries the conclusion:

| term | support |
|---|---|
| a grounding leaf `L` | `{{L}}` |
| `App(a, b)` | `{ sa ∪ sb : sa ∈ support(a), sb ∈ support(b) }` — conjunctive |
| `Sum(a, b)` | `support(a) ∪ support(b)` — **disjunctive** |
| `SpecStr(j, tag)` | `support(j)` — specialization changes what is concluded, not what it rests on |

*"Every leaf is `VerifiedEvidence` on some spanning sub-polynomial"* is then an **existential over alternatives**, and
that existential is load-bearing: a conclusion resting on `Sum(VerifiedEvidence(a), DeclaredEvidence(b))` IS fully
verified, because the `a` branch carries it alone. Reading `Sum` conjunctively understates every conclusion that has a
fallback — which is the shape a careful author writes, and precisely what D39 §8's propagation rule got wrong.

The exposure questions (*what does it rest on*, *which measurements*) read as a **union** across alternatives: a ground
appearing on any branch is one the conclusion may rest on.

`App` over `Sum` multiplies, so support is exponential in nested alternatives. Real terms are small — the largest on
the WRN chain is three grounds — but the bound is real, so the projection **refuses past a cap rather than truncating**:
every one of these answers reads as exhaustive, and a quiet truncation would make each of them wrong in the
safe-looking direction.

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

**This is already in use for kernel-checked derivations, under another name.** Measured on the WRN chain
(`2026-08-21`): all 22 `VerifiedEvidence` citations target `reasoning:ReasoningSentence` resources — a conclusion
whose polynomial type-checked becomes a citable constant, so later certificates need not re-derive it. That is
Artemov's internalization step, in production. So the decision above is partly a *description* of existing practice;
what is missing is only the external-prover case (§4.1).

The invariant carries its own escape hatch, because §3.3 shows not every derivation has a proposition to internalize:

> Every `Derived` or `Verified` resource either carries a witness for its stated proposition, or is explicitly marked
> **provenance-only** — derived in fact, uncitable as a reason.

That keeps the property checkable and makes the uninternalized set countable. Known violations: `VerificationTrace`
emitted nothing (§4.1) until eigenius#200 closed it on `2026-08-21`; D6b hedges the same way about EigenQL
`FIBER … INTO` commits and comorphism reify outputs, which #206 tracks. Each is a place where the chain plausibly
knows something it cannot cite.

### 3.3 What a derivation may witness is bounded by its specification

A program is a computation `f : I → O`. Applying it establishes `f(i) : O` — application at the **type** level. That is
a real derivation, but its content is *inhabitation*: this artifact is a well-formed `O`, produced from `i` by `f`. As
a proposition it is about the **artifact**, close to `Asserts(output_iri)` plus provenance. Thin, and correctly thin.

EigenTT terms are the other case. Where `P : A -> Prop` is applied to `a : A`, the result `P(a)` is a genuine
proposition, and `App` / `spec_poly` do real justification work. Institution inference rules live here.

Two consequences.

**`DerivedEvidence(iri)` is correctly a constant, not a polynomial.** Where the specification is a *type* rather than a
*Prop*, there is no propositional composition to record. The term algebra is also right to have **no constructor for
"program applied to input"** — program application is not a justification step. (An earlier draft of this document
argued the opposite; withdrawn.)

**The rule:**

> A derivation may witness only a proposition its specification entails. For `f : I -> O` that is an inhabitation fact
> about the output. A domain proposition requires either a Prop-valued rule, or a declared leaf naming who asserts it.

The live violation was `enc:EncodedClaim`. It *requires* `canonical_proposition`, was a `DerivedResource`, and is
produced by the parser. **Fixed `2026-08-21` by eigenius#201**, which is §6 applied; the statement of the violation
stands as the reason.

What the parser establishes is *"this text parses to this well-typed term"*. What the claim carries is a domain
proposition about the world. `IsDerivedAs(iri, P)` was admitted for a `P` the program never established, and a
certificate could cite it.

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

**Closed `2026-08-21` by eigenius#200; the analysis below is what led there.** `trace_category` now has all four
arms, and the reasoning institution mints a `VerificationTrace` when a certificate checks. The resolution was not to
separate the two notions but to recognise them as one: kernel-checking IS verification, and `proof_system`
distinguishes the verifier. Historical statement of the defect follows.

`trace_category` had arms for `DeclarationTrace`, `ObservationTrace` and `ProgramTrace`. **There was no arm for
`VerificationTrace`.** A resource carrying an actual Lean proof therefore emitted no witness and could not be cited.

What *does* emit a Verified witness is `emit_from_reasoning_sentence` — a `ReasoningSentence` whose `JustifiedBy`
certificate type-checked. So two different things are both called Verified:

| | means | witness? |
|---|---|---|
| `reflection:VerifiedResource` | an external prover's proof blob is attached | **none** |
| `IsVerifiedAs` as emitted today | the kernel type-checked a certificate | yes |

The second is sound but is a different claim — the kernel checked a derivation, not an external prover.

**And the second is what real content means.** Measured across the WRN encoding chain (`2026-08-21`): 22
`VerifiedEvidence` citations, **all** targeting `ReasoningSentence` resources, and **not one** `VerificationTrace`,
`proof_term` or `proof_system` in the entire chain. The flagship experiment uses `VerifiedEvidence` to mean
*"the kernel type-checked this conclusion's certificate"*, and it works precisely because
`emit_from_reasoning_sentence` is the only path that emits.

So this is not a latent hazard to be resolved before someone trips on it. One side is established usage; the other is
documentation for a path that has never run. The naming is the defect — see §11.4.

**D39 §10's factivity sentence describes the first path**: *"`VerifiedEvidence`-grounded justifications imply truth
(the Lean checker validated the proof, so the proposition holds)."* When this was written that path emitted nothing,
and the sentence did not merely lack warrant — it described a route that did not exist.

> **The route exists as of `2026-09-03`** (eigenius#159, eigenius#160). The statement check is mandatory: the claim's
> `canonical_proposition` is externalized to Lean and compared to the target declaration's type with `def_eq`
> (`crates/eigenius-lean/src/externalize.rs`), and a claim carrying no proposition is refused rather than falling back
> to the name-level check. On `Holds` the institution emits a `prov:VerificationTrace` naming the claim, which is what
> makes `layer_admits_witness` answer `Verified` for that claim's own proposition. D39 §10's sentence is now earned —
> subject to §4.2, which is the part that does not change.
>
> The measurement above still holds for the WRN chain: those 22 `VerifiedEvidence` citations target
> `ReasoningSentence` resources and are kernel-checked certificates, the second row of the table. What changed is that
> the first row is no longer documentation for a path that has never run.

**The fix was designed here, and the design changed on `2026-08-21`.** [D49](d49-chainwitness-machinery.md) §7 specified
recovering the EigenTT proposition by **inverting** D30's translation and reifying a
`reasoning:VerifiedPropositionView`. That section is now superseded. The replacement is **externalize-and-check**:
the Lean institution translates the claim's existing `canonical_proposition` *into* Lean, Lean returns a proof term,
and the baked-in `nanoda_lib` checks it against the **externalized statement** — so `IsVerifiedAs(iri, P)` is admitted
with `P` the claim's own proposition and no inverse exists.

Better on three counts. The forward translation is **total** on the domain that matters, while the inverse is partial
over Lean's larger language — D49 §7 concedes its own failure mode. It leaves **one** trusted translation instead of
two, which is what §4.2 is about. And it removes the reified view entirely, so the witness keys on the claim's own
proposition hash.

D49 §7 chose the inverse because `eigentt:Term` and the `Prop` universe did not exist when the Lean institution
was built; D46 and D47 removed that constraint. Same shape of correction this document makes to D39 §8 — a design
right for its premises, outliving them.

The gap is also one level lower than [#159](https://github.com/eigenius/eigenius/issues/159) states: `checker.rs`
takes a **`target_name`** and `Holds` means *"every declaration type-checks and the target name resolves"*. Nothing
compares the named theorem's **statement** to anything. Externalization is what supplies a statement to compare
against.

### 4.2 Factivity is relative, and should say so

"`VerifiedEvidence` implies truth" is factive **relative to** trusting the external prover's kernel and the
translation. D39 states it unconditionally. This is the one place the platform's guarantees bottom out in trusting
something outside it, and the document should name that rather than imply absolute factivity.

**Unchanged by eigenius#159, and narrowed by it.** Externalize-and-check leaves *one* trusted translation instead of
two, and relocates it: the trusted artifact is no longer D30's class mirror alone but
[D74](d74-eigentt-to-lean-externalization.md)'s forward externalization, whose fragment is fixed by an exhaustive
match over all 43 `Exp` variants — 22 translated, 21 refused with typed errors. D74 §5 states the consequence in its
own words: *"If this document's mapping is wrong, the system proves the wrong theorem soundly."* The TCB is
`nanoda_lib` + D30 + D74.

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

In both, the parse contributes **form, never warrant**. The `ProgramTrace` stays attached to the encoding artifact and
witnesses the inhabitation fact of §3.3; the claim cites an agent.

### 6.1 Which artifact, and how many traces

*"Attached to the encoding artifact"* was under-specified, and the first implementation pass read it as *"attached to
the claim"* — the only per-sentence resource in reach — and so deleted the trace rather than moving it. Settled
`2026-08-22`: **encoding a document produces two objects, and they take opposite categories.**

| object | class | category | trace | how many |
|---|---|---|---|---|
| the run's output | `enc:ReasoningStructure` | **Derived** | `ProgramTrace` → structure | **one per run** |
| each proposition | `enc:EncodedClaim` | **Declared** | `DeclarationTrace` → claim | one per claim |

`enc:ReasoningStructure` is the right target because it already *is* the run's output: it carries `enc:source_path`
("the run's input location") and `enc:source_sha256` ("the exact bytes this structure was derived from"). It is a
function of (engine, bytes) → structure, which is Derived in the plain sense of §4.

The old shape was wrong twice over, and the second error hid the first. It minted **N** `ProgramTrace`s for **one**
program execution — one per sentence — so the cardinality never matched the process; and each of them keyed
`IsDerivedAs(claim, P)` on a `P` about the world, so a certificate citing one read as *a program established P*.

Nothing needs a new predicate. A certificate cites a claim's warrant; nothing cites *"the run happened"* — that is
provenance — so the §4.1 default `Asserts(structure_iri)` suffices, the structure's identity having already pinned the
engine, the bytes and the claim set.

This also places the parse inside the rule of eigenius#205: a run through D71's service is **kernel-initiated**, so its
`ProgramTrace` is a legitimately kernel-minted Derived witness rather than one an author wrote down.

D71's architecture already has the joint: generation is decoupled from commitment, and the notebook's `land` flag is
the agent's act of taking responsibility — the moment a formulation becomes an assertion.

**This supersedes the Stage-3 settlement of `2026-08-10`** ("parsed sentences land Derived; the Declared cluster
reserved for curator-pinned rules"). That split the world on *parsed vs curated*; the operative axis is **who asserts**.
D72 supplies what makes the change possible: `wrn:authors` is a resolvable `reflection:Agent`, so a claim from that
paper can cite the people who made it rather than the program that transcribed it.

**Scope of the change, measured (`2026-08-21`): three citations.** Only claims produced by the *parser* move.
`demo/prose-to-formulas-v2/inference.esl` cites `DerivedEvidence("…:claim_1")` three times, one of them commented
"THE LIVE DEPENDENCY". The WRN chain is hand-authored and contains **zero** `enc:EncodedClaim`; its 79
`DerivedEvidence` citations target `*_plan:result` statistics-institution outputs, which are genuine program outputs
and stay Derived under §3.3. An earlier estimate of this scope was wrong by counting every `DerivedEvidence` in the
tree; the WRN chain uses unqualified constructor names via a namespace binding, so a `reasoning:`-prefixed grep both
missed 177 citations and then over-attributed them.

Three propositions remain distinct and must not collapse into one witness:

1. *this text parses to this well-typed term* — the artifact fact, witnessed by the `ProgramTrace` (§3.3)
2. *this encoding is faithful to what the author wrote* — D61, unwarranted today
3. *what the author wrote is true* — never established by any of the above; only ever declared, by a named agent

## 7. Two lattices, deliberately independent

**The epistemic axis** (`reflection`) answers *where knowledge came from* and selects the grounding constructor.

**The discourse axis** (`encoding`) answers *what kind of assertion this is* — `enc:Claim` and its closed kinds. Its
own description says it "names the root the reflection: source lattice deliberately lacks, at the enc: level where
discourse needs it": the resource a demonstrative («these findings») can bind, whatever its epistemic source.

`enc:EncodedClaim : reflection:DeclaredResource` sits on both — Declared by construction (eigenius#201), carrying its
discourse kind as a second `is_a` (D68 §2). The axes are orthogonal and must stay so: a Finding can be Declared, Derived or Verified,
and the discourse kind says nothing about the warrant.

## 7a. Why proof irrelevance and justification tracking coexist

D46 gives `Prop` proof irrelevance: for any `P : Prop` and `t1, t2 : P`, `def_eq(t1, t2)` succeeds without
comparing them, and §6 lets the strong reducer skip `Prop`-typed subterms outright. D73 says the justification
term must be **retained whole**, because every epistemic summary is a query over it (§1.2). Read quickly these
look opposed: one erases proofs, the other insists on keeping them.

They are about different objects, and the confusion is worth naming because both live in `Prop`.

**Proof irrelevance is about inhabitants.** `t1` and `t2` are two derivations of the same proposition *inside*
the type theory. Nothing downstream may branch on which one it got, so the kernel is free to treat them as
equal — that is what makes `Prop` a proposition universe rather than a data type. Erasing the distinction
loses nothing, because there was nothing there to lose: a second proof of `P` tells a consumer nothing a first
proof did not.

**Justification tracking is about the index.** In `JustifiedBy : JustificationTerm -> Prop -> Type`, the
polynomial `j` is not an inhabitant of the proposition — it is an **argument of the type**, and by §2's
requirement (b) a different `j` gives a *different type*. Its leaves are chain IRIs naming agents, traces and
checkers outside the theory. `DeclaredEvidence(iri)` and `VerifiedEvidence(iri)` are not two proofs of one
thing; they are two different claims about what the world contains.

So the axes are orthogonal in the same way §7's two lattices are:

| | proof irrelevance applies | why |
|---|---|---|
| a proof of `JustifiedBy(j, P)` | **yes** | two derivations that `j` warrants `P` are interchangeable |
| the polynomial `j` itself | **no** | it indexes the type; erasing it changes which proposition is being asserted |
| the proposition `P` | **no** | same reason — it is the other index |

The practical consequence is that irrelevance costs the audit chain nothing. A consumer asking *"which agents
are we trusting?"* walks `j`'s `DeclaredEvidence` leaves; nothing in that walk inspects a proof object, so §6's
reducer gate never reaches it. And the reverse: retaining `j` whole imposes no obligation to retain the
derivations, which is why `JustifiedBy` can be a `Prop` at all.

This is also why D39 §8's collapse was a real loss rather than an instance of the same erasure. Projecting `j`
to a four-valued scalar discards *indices*, not inhabitants — the summary is strictly less informative than the
term, and §1.1 records the wrong rule that followed from treating it as sufficient.

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
3. **Withdraw §8's stored category**; expose the projections of §1.2 as queries over the retained term. — *Done: the
   withdrawal was free (§8 was never implemented); the queries landed `2026-08-22` as
   `reasoning:qc_project_justification` (eigenius#204), and are reached as Rust functions in
   `kernel/src/justification/` since P7 deleted that QueryClass with the institution hosting it.*
4. **Warrant formalization** as an ongoing activity, measured by §3.1's leaf count.

Steps 1 and 2 are independent. Step 3 depends on nothing but is a vocabulary change with consumers.

## 11. Open questions

1. **Is chain internalization (§3.2) a goal?** If yes it is a gate on commit paths that establish propositions without
   emitting a witness. If no, say so, because the Artemov reading invites the assumption that it holds.
2. **Where does the Lean/EigenTT statement comparison happen**, and is D30's translation trusted or checked? D49 §7
   specifies inverting the forward translation; whether the inverse is verified or assumed is unsettled.
3. ~~**Should `spec_str` generalize beyond `core:string`?**~~ **Decided `2026-08-21` (eigenius#203): the question
   was stale and the answer is the other direction.** `spec_str` had already been generalized — by `spec_poly`,
   which strictly subsumes it — so the live question was whether to keep two rules for one thing. It was retired.
   `SpecStr` the constructor stays; `JustificationTerm` remains at seven constructors.
4. ~~**Rename or split `VerifiedEvidence`?**~~ **Decided `2026-08-21` (eigenius#200): neither.** The premise was
   that "external prover proved it" and "the kernel checked a certificate" are different kinds. They are the same
   kind by different verifiers, and `proof_system` already records which. The defect was a missing ARTIFACT, not a
   bad name: nothing in the kernel ever created a `VerificationTrace`. Fixed by giving `trace_category` its fourth
   arm and having the reasoning institution mint a trace on a passing check. Cost: no constructor change, no
   migration of the 22 WRN citations, `JustificationTerm` unchanged at seven constructors.
5. ~~**Does `reflection:epistemic_status` survive as a materialized query result?**~~ **Decided `2026-08-22`
   (eigenius#204): it stays what it already is and the projections are separate.** Nothing ever computed it from a
   term — it is written in one place, for program outputs — so there was no cached projection to keep or remove, and
   `qc_project_justification` returns a `JustificationProjection` rather than writing a scalar back. A materialized
   projection can be added later against a built query; adding vocabulary for one first would have been speculative.
