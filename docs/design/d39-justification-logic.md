# D39 — Justification Logic as a First-Class Institution

*Status: draft proposal · May 2026*

*Companion documents: [D14 institution realisation](d14-institution-realisation.md), [D28 Lean 4 as institution](d28-lean-4-as-institution.md), [D32 chain-mirrored Mini-TT inductives](d32-chain-mirrored-mini-tt-inductives.md), [D6 execution architecture](d6-execution-architecture.md).*

---

## 1. Motivation

Eigenius represents data, computation, and verified knowledge cleanly. Resources carry typed properties; Programs are typed expressions in Mini-TT; numerical institutions handle domain-specific reasoning via dispatch with content-addressed Verdicts; the Lean 4 institution validates constructive proof terms in-kernel. What the platform does *not* yet represent first-class is **the agent's reasoning itself** — the arguments, hypotheses, and conclusions that connect observations and derivations into warranted claims.

Today an agent reasoning over the chain leaves a trail of structured artifacts (typed resource commits, Component invocation traces, institutional Verdicts) but no formal record of the *argument structure* that justified each commit. A reviewer asking "why does the agent believe X?" must reconstruct the warrant by walking provenance edges manually and inferring the inference rules the agent applied. The provenance is structurally complete; the reasoning over it is not.

This document proposes to close that gap by treating agent reasoning as a logic — specifically, Artemov-style justification logic — and packaging it as an institution. The Reasoning institution joins the existing numerical institutions and the Lean 4 institution as a first-class extension, with its own typed payload (JustificationTerm), its own validation gates, and its own comorphisms into the existing institutional surface. The deepest commitment: the four epistemic categories (`Declared`, `Observed`, `Derived`, `Verified`) become *structural projections* from the shape of an agent's justification terms, rather than separate annotations applied by the user.

The choice of justification logic — specifically, the Logic of Proofs introduced by Artemov (1995, 2008) — over alternatives (classical FOL, intuitionistic logic, modal epistemic logic, abstract argumentation) is deliberate. Justification logic treats the warrant as a first-class syntactic object: `t : A` reads "t is a justification for A." Justifications compose explicitly via typed operators, multiple distinct justifications can support the same claim, and the logic internalises reasoning about its own justifications. These properties match what an agent reasoning over a chain actually does: composing institutional verdicts, cited observations, and prior conclusions into warranted further claims. See §13 for full references.

## 2. Scope

In scope:

- A `JustificationTerm` chain-mirrored inductive ADT, analogous to `FormulaTerm` per [D32](d32-chain-mirrored-mini-tt-inductives.md). Closed constructor set; well-formedness validated by the kernel through the existing inductive-type machinery.
- The propositional language for Reasoning institution sentences: Mini-TT terms of type `Type(n)`, encoded using the kernel's existing CIC constructors (Π, Σ, Sum, Empty, Id) plus a single new core-ontology declaration `Asserts(iri) : Type` for atomic propositions. No new chain-mirrored ADT for propositions; propositions ride on D32's existing Mini-TT term mirroring.
- A Reasoning institution per [D14](d14-institution-realisation.md)'s three-method trait. Validates justification-logic well-formedness and composition *procedurally* (not as a type-theoretic predicate). Registered via either the in-kernel path (analogous to Lean 4 per [D28](d28-lean-4-as-institution.md)) or the runtime substrate (WASM); the choice is operational.
- A `ReasoningSentence` Resource class that carries a proposition (a Mini-TT term), its `JustificationTerm`, and an optional back-reference to whatever it justifies.
- Three comorphisms: Reasoning → Lean (the `VerifiedEvidence` constructor wraps a Lean-produced verified resource; the propositional language matches because both institutions speak Mini-TT types directly), Reasoning → numerical institutions (`DerivedEvidence` cites an institution-produced derivation), Reasoning → observed-resource fibre (`ObservedEvidence` cites a provenance-anchored observation).
- Structural propagation rules that compute the four epistemic categories from JustificationTerm shape.

Out of scope:

- Modifications to the kernel's type theory. CIC (Mini-TT fragment) stays as-is; nothing about justification logic is "special" at the kernel level. The kernel sees an inductive type, not "justifications specifically."
- Modifications to Mini-TT itself.
- A first-order logic institution. Separate work; this document specifies justification logic, not general predicate logic.
- Modal extensions (Fitting semantics, dynamic epistemic logic). Deferred to a follow-up document if the need surfaces.
- Defeasible / non-monotonic reasoning. Would require a different logical foundation (default logic, argumentation frameworks); separate from this work.
- Migration of existing chain data. New resources commit explicit JustificationTerms; existing Derived resources are left with their current provenance-edge representation. A bulk-lifting pass is a separate operational decision.
- Agent-extensibility of the `JustificationTerm` ADT itself. The agent composes justifications using existing constructors; adding new constructors is a chain-mirrored ADT change, with the same scope as modifying `FormulaTerm`.

## 3. The `JustificationTerm` interlingua

`JustificationTerm` is a chain-mirrored inductive type, encoded under [D32](d32-chain-mirrored-mini-tt-inductives.md)'s convention. The constructor set is small, closed, and structurally aligned with the platform's four epistemic categories. Adding a new constructor is a versioned change to the ADT declaration, subject to the same review discipline as `FormulaTerm` evolution.

The seven constructors partition into two groups: four **categorical groundings** (one per epistemic category) that anchor a justification in a typed chain resource, and three **composition operators** that combine sub-justifications under Artemov-style rules.

**Categorical groundings.** Each constructor references a resource of the matching epistemic-category base class. The kernel's existing category-class enforcement handles all referential validation uniformly.

| Constructor | Signature | Semantics |
|---|---|---|
| `DeclaredEvidence(iri)` | `core:iri → JustificationTerm` | IRI must resolve to a `DeclaredResource` (axiom, hypothesis, convention, regulatory threshold). Justifies a claim by authority without further evidence. |
| `ObservedEvidence(iri)` | `core:iri → JustificationTerm` | IRI must resolve to an `ObservedResource` (measurement, citation, recorded data). Justifies a claim by direct observation with external provenance. |
| `DerivedEvidence(iri)` | `core:iri → JustificationTerm` | IRI must resolve to a `DerivedResource` (a computation output, model prediction, or institution-dispatched derivation; the originating institution's `Verdict` is attached to the resource as provenance). Justifies a claim by citing a typed derivation. |
| `VerifiedEvidence(iri)` | `core:iri → JustificationTerm` | IRI must resolve to a `VerifiedResource` (typically a `LeanProofTerm` per [D28](d28-lean-4-as-institution.md), but any institution-validated verified resource qualifies). Justifies a claim by citing a checked proof. |

**Composition operators.** Category-agnostic; epistemic effect determined by sub-justifications per §8.

| Constructor | Signature | Semantics |
|---|---|---|
| `App(j1, j2)` | `JustificationTerm × JustificationTerm → JustificationTerm` | Artemov's application operator. If `j1 : (A → B)` and `j2 : A`, then `App(j1, j2) : B`. |
| `Sum(j1, j2)` | `JustificationTerm × JustificationTerm → JustificationTerm` | Sum of justifications. If `j1 : A` and `j2 : A`, then `Sum(j1, j2) : A`. Captures multiple converging justifications. |
| `Refutation(j)` | `JustificationTerm → JustificationTerm` | If `j : A`, then `Refutation(j) : ¬A`. Used for explicit refutation and belief revision. |

The four-and-three partition is load-bearing: every justification term grounds in some combination of the four categorical evidence types, composed via the three operators. There is no "untyped" or "categoryless" grounding; nothing in a justification escapes the platform's epistemic vocabulary.

A `JustificationTerm` carries no propositional content on its own — it is the term half of a `t : A` pair. The proposition `A` is a Mini-TT term of type `Type(n)` and lives in the surrounding `ReasoningSentence` Resource that the term is embedded in. The propositional language and what the kernel does with it are specified in §4.1.

The encoding follows D32: each constructor is a ctor call (`{ctor: "App", args: [j1, j2]}` in Eigon-JSON), validated by the kernel's inductive-value walker against this ADT's schema at commit. Well-formed terms are accepted; ill-typed terms (wrong constructor arity, undeclared constructor name) are rejected before any institution-specific reasoning runs.

## 4. The Reasoning institution

The Reasoning institution is registered per [D14](d14-institution-realisation.md)'s three-method trait — `extract_typed`, `reify`, `query`. Its signature category includes the `JustificationTerm` constructors and the set of inference-rule resources visible at the queried layer.

### 4.1 Propositions and what the kernel knows about them

**Propositions are Mini-TT terms of type `Type(n)`.** No new chain-mirrored ADT for propositions; the kernel's existing CIC fragment provides the full propositional grammar. Propositions are encoded using constructors Mini-TT already supports:

- **Atomic propositions** use a single core-ontology declaration: `Asserts(iri) : Type`, where `Asserts` is a uniform-parameter inductive type with **no constructors**. Different IRIs produce distinct propositions; no structural inhabitation is possible (the type has no constructors); the only way to produce a proof of `Asserts(iri)` is through institutional dispatch (typically Lean producing a proof term that the in-kernel checker validates). Uniform parameter rather than index sidesteps the indexed-family requirement.
- **Conjunction** is `Σ` (already present).
- **Disjunction** is the standard `Sum` inductive (already present).
- **Implication** is `→` (non-dependent Π, already present).
- **Negation** is `→ Empty` (Empty is the no-constructor inductive; already supported).
- **Universal / existential quantification** when needed: `Π` / `Σ` (already present).
- **Equality** between FormulaTerm-typed values: `Id` / `Refl` / `IdJ` (already primitive).

That's the full propositional language for v1, expressible against Mini-TT as it stands today — no `Prop` universe, no indexed families, no new constructors.

**What the kernel can do and say about these propositions.** The kernel's role is exactly its role for any Mini-TT term:

- **Typecheck them** — verify each is well-formed as a Mini-TT type.
- **Normalize them** — reduce to canonical form using the existing NbE evaluator.
- **Decide definitional equality** between two propositions, within standard CIC bounds (β, ι, η where applicable, plus the decidable-equality machinery).
- **Check inhabitation** when a candidate proof term is presented — the standard `t : P` judgment. This is exactly what the Lean institution's in-process term checker exercises against `LeanProofTerm` resources.
- **Persist propositions as chain-mirrored resources** — propositions become content-addressed, queryable, and referenceable like any other chain artifact via the existing D32 machinery.

**What the kernel does not do, and does not need to.** There is no inductive predicate `JustifiedBy : JustificationTerm → Prop → Type` in the type system, and the kernel does not derive which justification justifies which proposition. That validation is the Reasoning institution's job, performed procedurally on commit via the `AutoOnLoad` gate (§4.3). The relation between justification and proposition is a *chain-level* relation, not a type-theoretic one — matching how the four epistemic categories are already validated on the platform (procedurally by base-class + AutoOnLoad enforcement, not type-theoretically).

The kernel also does not construct atomic-proposition inhabitants. `Asserts(iri)` has no constructors; the only way it is inhabited is via institutional dispatch — most commonly via Lean producing a proof term that the in-kernel checker validates. For non-Lean evidence paths (observed measurements, institutional derivations, declared axioms), there is no Mini-TT-level inhabitant; the warrant lives in the `JustificationTerm` and the Reasoning institution's procedural verdict.

### 4.2 The `ReasoningSentence` Resource

A `ReasoningSentence` is the chain-resident pairing of a proposition `A` with a `JustificationTerm` `t` — the agent's claim that `t : A`. Properties:

| Property | Type | Required? | Reading |
|---|---|---|---|
| `proposition` | reference to a chain-mirrored Mini-TT term of type `Type(n)` | yes | The proposition being asserted (using the grammar in §4.1). |
| `justification` | `JustificationTerm` | yes | The agent's warrant for the proposition (using the constructors in §3). |
| `subject_iri` | `core:iri` | optional | For atomic claims, the principal Resource the sentence is *about* — used for query indexing and back-reference. |

The implicit semantic claim of a `ReasoningSentence` is "this `JustificationTerm` justifies this proposition." The `ValidateJustification` AutoOnLoad gate (§4.3) checks exactly this at commit.

### 4.3 Models, satisfaction, and query classes

**Models.** Models are Mkrtychev-style interpretations (Mkrtychev 1997): a propositional valuation paired with a *justification function* mapping justification terms to the set of propositions they justify. The institutional dispatch does not enumerate models directly — it validates that the committed justification term satisfies the Artemov axioms (Artemov 2008) under any admissible model, which is sufficient for the validation gate's needs.

**Satisfaction.** Standard for justification logic: `M, w ⊨ t : A` iff `t` is in the justification function's image at `A` under the model `M`'s admissibility constraints. For the validation gate, the relevant test is the syntactic one (Artemov's axiom-checking) rather than the model-theoretic one.

**Query classes.** Three, with the standard dispatch roles per D14:

| Query class | Dispatch role | Behaviour |
|---|---|---|
| `ValidateJustification` | `AutoOnLoad` | Fires on every `ReasoningSentence` commit. The handler walks the embedded `JustificationTerm` against the embedded proposition (a Mini-TT type), checks that each categorical-evidence constructor's IRI resolves to a resource of the matching epistemic-category base class, validates each composition step against the Artemov axioms, and returns `Verdict::Holds` if the term well-justifies the proposition, `Verdict::Fails` (with a specific failure point) otherwise, `Verdict::Undecidable` if the proof obligation exceeds the validator's decision procedure. The validation is *procedural* — the validator pattern-matches on JustificationTerm constructors and proposition shapes; it does not invoke kernel type-checking of an internal `JustifiedBy` predicate (which does not exist; see §4.1). For the `VerifiedEvidence` constructor specifically, the validator delegates to the kernel's in-process Lean term checker to verify the wrapped proof term inhabits the proposition. |
| `EntailmentQuery` | `OnDemand` | Given a set of committed sentences `Γ` and a candidate proposition `A`, returns whether some justification term over `Γ` can be constructed for `A`. Used by agents and queries to ask "does the chain warrant this conclusion?" |
| `ConsistencyCheck` | `Decidable` | Returns whether a set of committed sentences is internally consistent under the institution's logic. Decidable for the propositional fragment; reports `Undecidable` for richer fragments. |

The `AutoOnLoad` gate is the load-bearing piece. Its `Verdict` becomes a first-class chain resource alongside the `ReasoningSentence` it validated, traceable via the same provenance machinery used by every other institution. A `Fails` verdict rejects the commit (consistent with D14 §6's general gating semantics); a `Holds` verdict admits the sentence with the verdict attached as evidence that the gate has spoken.

## 5. What counts as a justification

A `JustificationTerm` is constrained at three independent layers. All three must hold; failure at any level rejects the commit.

**Structural constraint.** The term must be well-typed at the `JustificationTerm` ADT level. The kernel's existing inductive-type validation machinery (the same machinery that handles `FormulaTerm`) checks constructor arity, constructor-name validity, and structural shape. This is the same kind of check the chain runs against any chain-mirrored inductive value; the Reasoning institution does not need to participate.

**Referential constraint.** Each categorical-grounding constructor requires its target IRI to resolve to a resource of the matching epistemic-category base class: `DeclaredEvidence` requires a `DeclaredResource`; `ObservedEvidence` requires an `ObservedResource`; `DerivedEvidence` requires a `DerivedResource`; `VerifiedEvidence` requires a `VerifiedResource`. The kernel resolves IRIs against the chain at commit, checks class membership via the standard `is_a` mechanism, and rejects the commit on either miss. The category base classes themselves carry the kernel's existing per-category invariants (`ObservedResource` requires a `source`; `VerifiedResource` requires a checked proof term; etc.); the JustificationTerm machinery inherits this enforcement transitively — a justification cannot reference an `ObservedResource` that itself lacks a valid `source`.

This constraint is what makes "the agent points at any IRI" impossible. A justification cannot reference imaginary resources, nor mis-typed ones. Every grounding edge in a justification chain terminates in a typed chain artifact the auditor can independently inspect.

**Semantic constraint.** Composite constructors require their sub-justifications to be type-compatible under Artemov's axioms. `App(j1, j2)` requires `j1` to justify an implication (a Mini-TT `→` type) and `j2` to justify its antecedent; the resulting term justifies the consequent. `Sum(j1, j2)` requires both sub-justifications to support the same proposition. `Refutation(j)` requires `j` to justify the proposition being refuted. The `ValidateJustification` AutoOnLoad gate walks the term against the proposition and checks each composition step; the gate is where this layer's validation actually fires.

Without the semantic layer, the term could be syntactically well-formed and refer to real resources while making nonsense claims. With it, the institution is doing real work: it rejects justifications whose composition steps don't satisfy the logic's rules.

## 6. The three reasoning patterns

The constructor set above realises three patterns that cover the bulk of what agents actually do over the chain. Each maps to one of the four categorical groundings (plus optional inference structure). In every pattern the conclusion `X` is a Mini-TT type per §4.1; the notation `t : X` reads "the JustificationTerm `t` justifies the proposition `X`" (validated procedurally by the Reasoning institution, not type-theoretically by the kernel).

**Pattern 1 — "I observed this, hence I conclude X."** The agent grounds in an `ObservedResource` and draws a further conclusion via an inference rule:

```
App(
  inference_rule,            // the "hence"
  ObservedEvidence(O)        // grounds the premise
)  :  X
```

The inference rule is itself a categorically-grounded justification — `DeclaredEvidence` for a registered methodological convention, `VerifiedEvidence` for a proved inference principle, `DerivedEvidence` for a rule established by prior reasoning. The conclusion `X` is `Derived` because the `App` adds inferential content beyond the strict observation (see §8 for the propagation rule).

**Pattern 2 — "I derived this, hence I conclude X."** The most common shape. The agent grounds in a `DerivedResource` (the output of a prior Component invocation or institutional dispatch — the originating institution's `Verdict` is attached to the resource as provenance):

```
App(inference_rule, DerivedEvidence(derived_resource_iri))  :  X
```

The conclusion stays `Derived`. Long inferential chains nest these `App`-spines arbitrarily deep, just as Mini-TT terms nest applications; each step has its own sub-justification, and the validator walks the tree at commit.

**Pattern 3 — "I proved this, hence I conclude X."** Two sub-cases that must be kept distinct:

If `X` is exactly the proposition the verified resource asserts:

```
VerifiedEvidence(verified_resource_iri)  :  X
```

The verified resource IS the justification; no inference rule is needed; the conclusion is `Verified` (the institution that produced the resource — typically Lean per [D28](d28-lean-4-as-institution.md) — has already validated the proof term it carries).

If `X` is some further inferential consequence of the verified claim:

```
App(inference_rule, VerifiedEvidence(verified_resource_iri))  :  X
```

The conclusion is `Verified` iff the inference rule is itself grounded in `VerifiedEvidence` (transitively); otherwise `Derived`. This is the most important propagation rule: **the `Verified` category propagates only when every link in the justification — leaves and inference rules alike — is grounded in `VerifiedEvidence`**. A single non-verified link downgrades the conclusion to `Derived`.

**Inference rules are recursively grounded.** The "hence" in each pattern is itself a categorically-grounded justification, and the same propagation rule applies to it. A `DeclaredEvidence` inference rule ("I take this as convention") yields a `Derived` conclusion no matter how strong the premise's justification is. A `VerifiedEvidence` inference rule applied to a `VerifiedEvidence` premise yields a `Verified` conclusion. The Reasoning institution validates each rule's grounding just as it validates every other constructor; the recursion is bounded by the chain's finite depth.

## 7. Comorphisms

The Reasoning institution participates in three comorphisms, each declared per D14's triadic structure. All three have identity-like middles on the constructor that carries the IRI — there is no transformation needed because the JustificationTerm constructor already carries the typed reference into the target institution's space.

**Reasoning → Lean.** Source class: `ReasoningSentence` whose `JustificationTerm` is a root `VerifiedEvidence(verified_resource_iri)` referencing a `LeanProofTerm`. Target class: the `LeanProofTerm` referenced by the IRI, with the proved proposition matching the sentence's proposition. The propositional alignment is direct: both institutions speak Mini-TT types as propositions (per §4.1), so the comorphism's identity middle on the verified-resource reference also collapses the propositional translation to identity — there is no separate propositional language to translate between. The comorphism establishes that a `ReasoningSentence` whose root justification is a closed Lean-produced verified resource is exactly a Lean-verified claim.

**Reasoning → numerical institutions.** Source class: `ReasoningSentence` whose `JustificationTerm` contains `DerivedEvidence(derived_resource_iri)` constructors referencing resources produced by a numerical institution. Target class: the `DerivedResource`s from the originating institution (Symbolics, IntervalArithmetic, Catalyst, OrdinaryDiffEq, JuMP-HiGHS, and any others registered) — each carries the institution's `Verdict` as provenance, so citing the `DerivedResource` cites the verdict transitively. The comorphism establishes that the agent's reasoning step properly grounds in a typed institutional derivation; identity middle on the derived-resource reference.

**Reasoning → observed-resource fibre.** Source class: `ReasoningSentence` whose `JustificationTerm` contains `ObservedEvidence(observation_iri)` constructors. Target: the `ObservedResource` whose `source` property anchors the observation in the external world. Identity middle on the observed-resource reference; the comorphism establishes that the agent's reasoning grounds in observations the chain itself anchors.

The deeper point: these comorphisms make the Reasoning institution a *meta-institution* in a precise sense. Other institutions produce typed `Verdict`s on their own logics; the Reasoning institution composes those verdicts (and observations, and proofs) into composite warrants. The comorphisms are the connective tissue.

## 8. Epistemic category propagation

With the categorical groundings aligned one-to-one with the four epistemic categories, the propagation rule reduces to a single recursive definition over the `JustificationTerm` tree:

```
category(JustificationTerm) =
  case JustificationTerm:
    DeclaredEvidence(_)  → Declared
    ObservedEvidence(_)  → Observed
    DerivedEvidence(_)   → Derived
    VerifiedEvidence(_)  → Verified
    App(j1, j2) | Sum(j1, j2):
      if category(j1) = Verified and category(j2) = Verified:
        Verified
      else:
        Derived
    Refutation(j):
      if category(j) = Verified: Verified else: Derived
```

A `ReasoningSentence`'s epistemic category is the category of its `JustificationTerm`. The rule has two clauses worth highlighting:

**Bare grounding constructors preserve their category.** A justification consisting of just `ObservedEvidence(O)` produces an `Observed` sentence — direct citation of a measurement is observational. Similarly for the other three.

**Composition operators always produce at least `Derived`, except when every sub-justification is `Verified`.** An `App` adds inferential structure; that inference can only preserve `Verified` if every input (both the inference rule and the premise) is itself fully verified. Any non-verified leaf or non-verified inference rule anywhere in the tree downgrades the conclusion to `Derived` — verification is monotonic but does not survive non-verified composition.

The category vocabulary is unchanged from the architecture spec. What changes is that the categories become **structurally enforced projections** from the JustificationTerm shape, rather than separate tags applied by the user or computed from loose provenance edges. The Reasoning institution's `ValidateJustification` gate computes the category mechanically as part of validating the term; the result is a typed Verdict resource alongside the sentence, queryable like any other chain artifact.

For resources committed without a `JustificationTerm` (which is most of the chain's existing data, and most non-reasoning commits going forward), the existing category-base-class enforcement applies as before. The new mechanism augments rather than replaces; explicit justification terms supersede provenance-based inference when both are present.

## 9. Belief, conclusion, and chain immutability

Two distinctions matter operationally.

**Belief vs conclusion.** *Belief* is the agent's provisional epistemic state — what the agent currently thinks, subject to revision as new evidence arrives. *Conclusion* is what the agent has *committed* to the chain as a `ReasoningSentence` Resource. Beliefs are not chain residents; conclusions are. The agent's working memory of beliefs is internal to the agent; only committed conclusions leave a trace.

This matters because the agent's "thinking" in the colloquial sense includes many beliefs that are never committed — hypotheses considered and rejected, partial arguments abandoned, intermediate calculations discarded. The chain records the agent's *durable* reasoning, not its *transient* reasoning. The `ValidateJustification` gate fires only on commit; intermediate beliefs need not satisfy the institution's validation rules.

**Chain immutability and belief revision.** The layer system is immutable: a commit cannot be retracted in place. An agent that changes its mind about a prior conclusion does not erase the prior commitment; it adds a new `ReasoningSentence` to a subsequent layer, typically with a `Refutation`-containing justification that explicitly refutes the prior. The chain preserves both:

- Layer N: `ReasoningSentence` asserting `X` with `JustificationTerm` `J1 : X`
- Layer N+k: `ReasoningSentence` asserting `¬X` with `JustificationTerm` `J2 : ¬X` where `J2 = App(refutation_rule, Refutation(J1))`

A future query (or future agent) can see both commitments and the chain of reasoning that led from one to the other. Auditors can ask "when did the agent change its mind about X, and what specifically refuted the earlier conclusion?" — a typed, structural answer drawn from the chain itself, not narrative reconstruction.

This is what the platform's "debugging cycle for thinking" looks like when applied at the chain level rather than only within a single agent session. The cycle's evidence becomes durable: every gate firing, every conclusion repaired, every belief revised in light of new evidence is a chain artifact the platform preserves.

## 10. Open questions and risks

**Axiom system choice.** This document assumes Artemov's basic LP system (Logic of Proofs; Artemov 1995, 2008) augmented with `Sum` and `Refutation`. Richer choices — JT4 (with iterated application), justification logic with factivity (where `t : A` implies `A`), modal extensions with Fitting-style semantics (Fitting 2005) — buy more expressiveness at the cost of validator complexity. The Artemov and Fitting (2020) monograph is the canonical survey of options. The minimal version is adequate for the agent reasoning patterns the platform expects in the near term; richer versions can be added by registering additional inference rules as chain resources without modifying the constructor set.

**Inference rules as Declared and Verified resources.** The Reasoning institution itself should declare the basic Artemov axioms — application, sum, constructor preservation, factivity for evidence-bearing justifications — as `DeclaredResource`s at registration time, citable via `DeclaredEvidence`. Other institutions may declare additional inference rules either as `DeclaredResource`s (when the rule's authority is methodological) or as `VerifiedResource`s carrying Lean proofs (when the rule is itself provable from a standing axiom system). The line between "logical axiom" and "domain inference rule" is conventionally drawn; the categorical-grounding constructors handle them uniformly. A `VerifiedEvidence` inference rule is what allows a chain of inferences to preserve the `Verified` category end-to-end.

**Validator performance.** Walking a deep justification term at commit time scales with the term's size. For agent reasoning chains of plausible depth (tens to low hundreds of constructors), validation cost should be modest. For pathological cases (deeply nested `App`-spines from a long inferential chain), the validator may need memoisation or iterative-deepening strategies; the practical limit is an empirical question the implementation surfaces.

**Migration of existing data.** Existing `Derived` and `Verified` resources do not carry explicit `JustificationTerm`s. A bulk migration would lift their provenance-edge structure into justification terms, but is not strictly required — the new mechanism augments the existing one. Whether to invest in a migration is an operational decision separable from this document's structural commitments.

**Belief revision and superseded layers.** The `Refutation` constructor supports explicit refutation, but the question of *which* prior commitment a refutation supersedes is non-trivial when multiple prior commitments cover the same claim. The naive rule is "the most recent prior commitment of the same sentence is superseded"; more sophisticated semantics (preferential refutation, contextual scoping) may be needed in practice and are deferred.

**Cross-fibre composition.** A justification term may reference resources from multiple institutions via `DerivedEvidence` (institution-produced derived resources) and `VerifiedEvidence` (typically Lean-produced verified resources). The Reasoning institution's validator must understand that a `DerivedEvidence` pointing at a JuMP-HiGHS-produced resource carries a different evidential weight than a `VerifiedEvidence` pointing at a Lean-produced resource. The validator does not need to encode the foreign institutions' logics; it needs to honour each institution's verdict according to its dispatch role and the chain's recorded status. The honest framing: this is the chain's existing audit story, made formal.

**Future kernel features that would enable richer integration.** Two kernel-level capabilities are absent from the current Mini-TT implementation and would, if added, allow the Reasoning institution to be promoted from procedural validation to type-theoretic validation. Neither is needed for v1; both are noted here as future enhancements rather than blockers:

- *Indexed inductive families* (D19's deferred work, issue #22) would allow `JustifiedBy : JustificationTerm → Proposition → Type` to be expressed as a proper indexed inductive predicate inside the type system, with elimination that refines indices. The current uniform-parameter-only inductives cannot express this directly. With indexed families, the Reasoning institution's `ValidateJustification` gate could be implemented as type-checking `JustifiedBy J P` for inhabitation rather than as procedural axiom-walking — substantively the same check, but unified with the kernel's type theory. Substantial kernel work (4–8 weeks) touching `term.rs`, `check.rs`, `eval.rs`, `recursor.rs`, `positivity.rs`. Architecturally well-understood (Coq, Lean, Agda all have it; Dybjer's "Inductive Families" is the canonical treatment).
- *A separate `Prop` universe with proof irrelevance* would let propositions live in `Prop` rather than `Type(n)`, with proof irrelevance making two proofs of the same proposition definitionally equal. This benefits conversion-checking performance broadly (the checker doesn't have to compute under propositions) and gives cleaner Curry-Howard semantics. Moderate kernel work (3–6 weeks) touching `term.rs`, `check.rs`, `eval.rs`, conversion. Not on the roadmap as of this writing.

The current design (procedural validation, `Asserts(iri)` as a uniform-parameter inductive with no constructors) is what's buildable against the kernel as it stands today. Both enhancements would be additive — the procedural path stays as a baseline; the type-theoretic path becomes an alternative validation route — so adopting them later does not invalidate any commitment made here.

## 11. Non-goals

To be explicit:

- **No kernel changes.** The kernel remains CIC-based (Mini-TT fragment); it sees `JustificationTerm` as another inductive type, not as a foundational construct. Any apparent privilege the Reasoning institution enjoys is institution-level, not kernel-level.
- **No separate propositional ADT.** Propositions are Mini-TT terms of type `Type(n)` (per §4.1), not a new chain-mirrored ADT. The only core-ontology addition for propositions is the `Asserts(iri) : Type` declaration — a uniform-parameter inductive with no constructors — for atomic propositions. Standard connectives use Mini-TT's existing Π, Σ, Sum, Empty, Id.
- **No internal `JustifiedBy` predicate in the type system.** The relation between a `JustificationTerm` and the proposition it justifies is a chain-level relation validated procedurally by the Reasoning institution's AutoOnLoad gate. It is not a Mini-TT inductive predicate the kernel reasons about. (See §10 — indexed inductive families would enable a type-theoretic version, but that's deferred future work, not v1.)
- **No replacement of provenance edges.** The existing chain-level provenance machinery continues to track resource-to-resource derivation. JustificationTerms supplement this with explicit warrant structure; they do not replace it.
- **No modal or dynamic-epistemic extensions in v1.** The first version covers the basic propositional fragment of justification logic, with the constructors documented above. Modal and dynamic extensions are follow-up work.
- **No defeasible / non-monotonic logic.** Refutation supports explicit retraction, but the framework does not commit to a particular non-monotonic semantics (default logic, circumscription, argumentation frameworks). These are separate logical frameworks; they could be admitted as their own institutions with their own term languages, related to Reasoning via declared comorphisms.
- **No first-order logic institution.** The platform's existing absence of a FOL institution (noted in the manifesto) is not closed by this document. FOL is a separate logic and a separate institution.
- **No agent-extensibility of the `JustificationTerm` ADT.** Agents compose using existing constructors; new constructors require a versioned ADT update with associated design review. The agent's controlled-extension path is to author new institutions (per the bonus-capability discussion in the experiment brief) whose derived outputs the existing `DerivedEvidence` constructor can cite, and whose declared inference rules the existing `DeclaredEvidence` constructor can cite.

## 12. Relationship to other design documents

- **[D6 execution architecture](d6-execution-architecture.md)** — reasoning traces describe *what happened* during execution; justification terms describe *with what warrant* a claim is asserted. The two are related but distinct. A trace can be lifted into a justification term (the constructors that wrap institutional Verdicts and observations are exactly the trace's structural elements made into typed warrant-bearing claims), but the lifting is a separate operation; traces do not become justifications automatically.
- **[D14 institution realisation](d14-institution-realisation.md)** — the Reasoning institution is a normal institution per D14's three-method trait. Its query classes follow the standard `OnDemand` / `AutoOnLoad` / `Decidable` dispatch-role mechanism. Its Verdicts integrate into the chain's audit story without special-casing.
- **[D28 Lean 4 as institution](d28-lean-4-as-institution.md)** — the `VerifiedEvidence` constructor references resources of class `VerifiedResource`, typically `LeanProofTerm`s; the Reasoning → Lean comorphism establishes how verified-citing justifications connect to constructive verification. The Lean institution keeps its kernel-linked in-process term-checker privilege; the Reasoning institution does not inherit it.
- **[D32 chain-mirrored Mini-TT inductives](d32-chain-mirrored-mini-tt-inductives.md)** — `JustificationTerm` is another chain-mirrored inductive sitting alongside `FormulaTerm` in the chain's interlingua catalogue. The encoding follows D32's pattern; the kernel's validation machinery handles both uniformly. D32 also covers the chain-mirroring of arbitrary Mini-TT terms, which is what hosts the propositional language (§4.1) — propositions are Mini-TT types stored as standard D32-mirrored terms, not a new ADT. The `Asserts(iri) : Type` declaration is a single core-ontology addition (a uniform-parameter inductive with no constructors) that lifts Resources into atomic propositions.
- **The four epistemic categories** specified in the architecture documents — this proposal aligns the `JustificationTerm` interlingua structurally with the four categories: each grounding constructor (`DeclaredEvidence`, `ObservedEvidence`, `DerivedEvidence`, `VerifiedEvidence`) references a resource of the corresponding base class. The propagation rule in §8 computes a sentence's category mechanically by walking the term tree. The categories' meaning is unchanged from the architecture spec; the structural enforcement becomes stricter for resources that commit justification terms.

## 13. References

**Justification logic — primary sources.**

- S. Artemov (1995). *Operational modal logic.* Technical Report MSI 95-29, Mathematical Sciences Institute, Cornell University. The foundational paper introducing the Logic of Proofs (LP); first internalisation of evidence terms into a modal-style provability logic.
- S. Artemov (2001). "Explicit provability and constructive semantics." *Bulletin of Symbolic Logic*, 7(1), 1–36. Journal-published treatment of LP with semantic and proof-theoretic details; widely cited in lieu of the 1995 technical report.
- S. Artemov (2008). "The Logic of Justification." *The Review of Symbolic Logic*, 1(4), 477–513. Canonical modern reference for justification logic; basis for the `JustificationTerm` constructor set (§3) and for the Artemov axioms the `ValidateJustification` AutoOnLoad gate checks (§4.3).
- S. Artemov and M. Fitting (2020). *Justification Logic: Reasoning with Reasons.* Cambridge University Press, *Cambridge Tracts in Mathematics* 216. Comprehensive monograph covering LP, JT and JT4, factivity, modal extensions, multi-agent variants, and applications. The recommended deep reference.
- S. Artemov and M. Fitting. "Justification Logic." *Stanford Encyclopedia of Philosophy.* First published 2011, periodically revised. Survey article; recommended entry point for readers new to the area.

**Model theory.**

- A. Mkrtychev (1997). "Models for the logic of proofs." In S. Adian and A. Nerode (eds.), *Logical Foundations of Computer Science (LFCS '97)*, LNCS 1234, Springer, 266–275. Introduces the basic-model semantics referenced in §4.3 — propositional valuations paired with justification functions, sufficient for the validation gate's syntactic axiom-checking.
- M. Fitting (2005). "The logic of proofs, semantically." *Annals of Pure and Applied Logic*, 132(1), 1–25. The richer Kripke-style semantics for justification logic referenced in §10 as a foundation for potential modal extensions; supersedes Mkrtychev models where modal-frame structure is needed.

---

*This is a draft proposal. The structural commitments — justification logic as the foundation, the closed `JustificationTerm` constructor set, the three-layer constraint story, the epistemic-category propagation as a projection from justification structure — are the load-bearing design decisions and should be the focus of review. The specific choice of Artemov LP as the base axiom system, the constructor list in §3, and the open questions in §10 are open to revision.*