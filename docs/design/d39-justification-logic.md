# D39 — Justification Logic as a First-Class Institution

*Status: v2 design proposal · June 2026*

*Companion documents: [D14 institution realisation](d14-institution-realisation.md), [D28 Lean 4 as institution](d28-lean-4-as-institution.md), [D32 chain-mirrored EigenTT inductives](d32-chain-mirrored-mini-tt-inductives.md), [D46 Prop universe + axiom framework](d46-prop-universe-and-proof-irrelevance.md), [D47 chain-mirrored EigenTT type fragment](d47-chain-mirrored-eigentt-type-fragment.md), [D48 indexed inductive families](d48-indexed-inductive-families.md), [D49 `ChainWitness` machinery](d49-chainwitness-machinery.md), [D6 execution architecture](d6-execution-architecture.md).*

*Foundation note: this design consumes four substrates that landed before this revision — D46 (impredicative `Prop` universe with proof irrelevance, plus the `eigentt:Axiom` chain class hosting built-in `propext` and `Quot.sound`), D47 (chain-mirrored EigenTT type fragment with the `eigentt:TypeExpr` codec, extended by eigenius#71 for term-level constructors), D48 (indexed inductive families with first-order pattern unification, dependent constructor checking, and singleton-elim for propositional indices), and the existing `reflection` ontology's epistemic-category base classes and Trace event classes. Propositions live in `Prop`; the relation between a `JustificationTerm` and the proposition it justifies is captured by a type-theoretic indexed inductive predicate `JustifiedBy : JustificationTerm → Prop → Type`; the chain-side grounding facts that predicate consumes are projected from the reflection ontology's existing class-membership and Trace-emission events via opaque `ChainWitness` predicates. Earlier drafts of this document sketched a separate procedural-validator path as a v1 fallback; that path is dropped in favour of the unified type-theoretic design throughout.*

---

## 1. Motivation

Eigenius represents data, computation, and verified knowledge cleanly. Resources carry typed properties; Programs are typed expressions in EigenTT; numerical institutions handle domain-specific reasoning via dispatch with content-addressed Verdicts; the Lean 4 institution validates constructive proof terms in-kernel. What the platform does *not* yet represent first-class is **the agent's reasoning itself** — the arguments, hypotheses, and conclusions that connect observations and derivations into warranted claims.

Today an agent reasoning over the chain leaves a trail of structured artifacts (typed resource commits, Component invocation traces, institutional Verdicts) but no formal record of the *argument structure* that justified each commit. A reviewer asking "why does the agent believe X?" must reconstruct the warrant by walking provenance edges manually and inferring the inference rules the agent applied. The provenance is structurally complete; the reasoning over it is not.

This document proposes to close that gap by treating agent reasoning as a logic — specifically, Artemov-style justification logic — and packaging it as an institution. The Reasoning institution joins the existing numerical institutions and the Lean 4 institution as a first-class extension, with its own typed payload (`JustificationTerm`), its own kernel-checked validation predicate (`JustifiedBy`), and its own comorphisms into the existing institutional surface. The deepest commitment: the four epistemic categories (`Declared`, `Observed`, `Derived`, `Verified`) become *structural projections* from the shape of an agent's justification terms, rather than separate annotations applied by the user.

The choice of justification logic — specifically, the Logic of Proofs introduced by Artemov (1995, 2008) — over alternatives (classical FOL, intuitionistic logic, modal epistemic logic, abstract argumentation) is deliberate. Justification logic treats the warrant as a first-class syntactic object: `t : A` reads "t is a justification for A." Justifications compose explicitly via typed operators, multiple distinct justifications can support the same claim, and the logic internalises reasoning about its own justifications. These properties match what an agent reasoning over a chain actually does: composing institutional verdicts, cited observations, and prior conclusions into warranted further claims. See §13 for full references.

## 2. Scope

In scope:

- A `JustificationTerm` chain-mirrored inductive ADT (D32 encoding) with a closed six-constructor set: four categorical groundings plus two composition operators (`App`, `Sum`).
- The propositional language for Reasoning institution sentences: EigenTT terms in the impredicative `Prop` universe (D46), encoded via the D47 `eigentt:TypeExpr` codec, with `Asserts(iri) : Prop` as the atomic-proposition constructor.
- A kernel-level indexed inductive predicate `JustifiedBy : JustificationTerm → Prop → Type` (D48 indexed family) whose inhabitants are the type-theoretic certificates that a given `JustificationTerm` justifies a given proposition.
- A `ChainWitness` opaque-predicate family that projects the reflection ontology's existing class-membership and Trace-emission facts into the type system. Witnesses are kernel-internal — inhabitants are admitted as a consequence of the corresponding Trace-emitting commit succeeding, never constructed via ESL.
- A Reasoning institution per D14's three-method trait. Its `ValidateJustification` `AutoOnLoad` gate is implemented by type-checking the embedded `JustifiedBy` certificate against the embedded proposition; no procedural axiom-walking.
- A `ReasoningSentence` Resource class carrying the proposition, the `JustificationTerm`, the `JustifiedBy` certificate (chain-mirrored), an optional `subject_iri`, and an optional `refutes` pointer for belief revision (full semantics in chain-merge work).
- Four comorphisms: Reasoning → Lean (forward, EigenTT → Lean translation per D30), Lean → Reasoning (backward, the v1 inverse of D30; produces the `VerifiedPropositionView` resources that make `ChainWitness.IsVerifiedAs` admissible per D49 §7), Reasoning → numerical institutions, Reasoning → observed-resource fibre.
- Structural propagation rules that compute the four epistemic categories from `JustificationTerm` shape.
- One small reflection-ontology extension: an optional `reflection:canonical_proposition` property on `DeclaredResource` / `ObservedResource` / `DerivedResource` for resources advertising an explicit `Prop` statement beyond the default `Asserts(iri)`.

Out of scope:

- Modifications to EigenTT beyond the eigenius#69 no-confusion enhancement (filed against D48; helpful for some `JustifiedBy` eliminations but not blocking).
- A first-order logic institution. Separate work.
- Modal extensions (Fitting semantics, dynamic epistemic logic). Follow-up document if needed.
- Defeasible / non-monotonic reasoning. Separate logical foundation; out of this scope.
- Migration of existing chain data. New resources commit explicit `JustifiedBy` certificates; existing Derived resources keep their current provenance-edge representation. A bulk-lifting pass is a separate operational decision.
- Agent-extensibility of the `JustificationTerm` ADT itself. Constructors are versioned with the same review discipline as `FormulaTerm`.
- Full belief-revision semantics for the `refutes` pointer. Sketched here; precise interaction with chain-merge resolution is deferred to that work.

## 3. The `JustificationTerm` interlingua

`JustificationTerm` is a chain-mirrored inductive type, encoded under [D32](d32-chain-mirrored-mini-tt-inductives.md)'s convention. The constructor set is small, closed, and structurally aligned with the platform's four epistemic categories. Adding a new constructor is a versioned change to the ADT declaration, subject to the same review discipline as `FormulaTerm` evolution.

The six constructors partition into two groups: four **categorical groundings** (one per epistemic category) that anchor a justification in a typed chain resource, and two **composition operators** that combine sub-justifications under Artemov-style rules.

**Categorical groundings.** Each constructor references a resource of the matching reflection-ontology base class. The kernel's existing `is_a` enforcement plus the per-class `requires` invariants in the reflection ontology handle all referential validation.

| Constructor | Signature | Semantics |
|---|---|---|
| `DeclaredEvidence(iri)` | `core:iri → JustificationTerm` | IRI must resolve to a `reflection:DeclaredResource` (axiom, hypothesis, convention, regulatory threshold; an `eigentt:Axiom` per D46 is the canonical statement-bearing case). Justifies a claim by authority without further evidence. |
| `ObservedEvidence(iri)` | `core:iri → JustificationTerm` | IRI must resolve to a `reflection:ObservedResource` (measurement, citation, recorded data; the resource's `source` property anchors external provenance). |
| `DerivedEvidence(iri)` | `core:iri → JustificationTerm` | IRI must resolve to a `reflection:DerivedResource` (a computation output, model prediction, or institution-dispatched derivation; the originating institution's `Verdict` is attached to the resource as provenance). |
| `VerifiedEvidence(iri)` | `core:iri → JustificationTerm` | IRI must resolve to a `reflection:VerifiedResource` (typically a `LeanProofTerm` per [D28](d28-lean-4-as-institution.md)). Since `VerifiedResource` is `subclass_of` `DerivedResource` in the reflection ontology, a VerifiedResource also satisfies any `DerivedEvidence`-shaped grounding obligation. |

**Composition operators.** Category-agnostic; epistemic effect determined by sub-justifications per §8.

| Constructor | Signature | Semantics |
|---|---|---|
| `App(j1, j2)` | `JustificationTerm × JustificationTerm → JustificationTerm` | Artemov's application operator (`·`). If `j1 : (A → B)` and `j2 : A`, then `App(j1, j2) : B`. Direct rendering of the Application axiom (Artemov & Fitting 2020, Definition 2.3): `s:(X → Y) → (t:X → [s · t]:Y)`. The type-theoretic content is captured by `JustifiedBy.app` per §5. |
| `Sum(j1, j2)` | `JustificationTerm × JustificationTerm → JustificationTerm` | Artemov's sum operator (`+`). If `j1 : A` *or* `j2 : A` — either side alone suffices — then `Sum(j1, j2) : A`. Direct rendering of the Sum axiom (Artemov & Fitting 2020, Definition 2.3): `s:X → [s + t]:X` and `t:X → [s + t]:X`. Captures "I have multiple possible justifications for the same claim; this term packages them and either witness will do." Proof irrelevance (D46) means the two underlying proof terms (when both are present) need not be definitionally equal. |

The four-and-two partition is load-bearing: every justification term grounds in some combination of the four categorical evidence types, composed via the two operators. There is no "untyped" or "categoryless" grounding; nothing in a justification escapes the platform's epistemic vocabulary. Belief revision — the case earlier drafts handled with a `Refutation` constructor — is structural rather than logical: it lives on the surrounding `ReasoningSentence` as a `refutes` pointer (§4.2, §9), not as a `JustificationTerm` constructor. Deriving `¬P` is just composition with a negation-producing inference rule.

A `JustificationTerm` carries no propositional content on its own — it is the term half of a `t : A` pair. The proposition `A` is a `Prop`-typed EigenTT term and lives in the surrounding `ReasoningSentence` Resource that the term is embedded in, paired with a `JustifiedBy t A` certificate (§4.2).

The encoding follows D32 §3.7's `{"ctor": "<ctor_name>", "args": [...]}` value shape — for example, `App(j1, j2)` is `{"ctor": "App", "args": [j1, j2]}`. The kernel's inductive-value walker validates it against the ADT's schema at commit (D32 §3.5). Well-formed terms are accepted; ill-typed terms (wrong constructor arity, undeclared constructor name) are rejected before any institution-specific reasoning runs.

Where a constructor's argument is itself a chain-mirrored EigenTT term — most notably the `proposition` and `certificate` fields of `ReasoningSentence`, and any inference rule whose statement needs to be cited explicitly — the encoding is the D47 `eigentt:TypeExpr` codec extended by eigenius#71 to cover term-level constructors (`UnitVal`, `Pair`, `CtorApp`, and the literal payloads). D47 is what makes the propositions in §4.1 and the certificates in §4.2 chain-resident: an `Exp` such as `Π (x : A), B[x]` or `JustifiedBy.app cert_1 cert_2` round-trips between kernel `Exp` and chain `Value::Json` losslessly, without a parallel ADT. The same codec is what `eigentt:Axiom` (D46 §10) uses to carry its `axiom_statement` on the chain.

## 4. The Reasoning institution

The Reasoning institution is registered per [D14](d14-institution-realisation.md)'s three-method trait — `extract_typed`, `reify`, `query`. Its signature category includes the `JustificationTerm` constructors and the set of inference-rule resources visible at the queried layer.

### 4.1 Propositions and what the kernel knows about them

**Propositions are EigenTT terms in the impredicative `Prop` universe.** Per [D46](d46-prop-universe-and-proof-irrelevance.md), `Prop = Sort(0)` is impredicative (a `Π` whose codomain is in `Prop` is itself in `Prop`, regardless of the domain's universe) and supports proof irrelevance (any two proofs of the same `Prop`-typed proposition are definitionally equal at that type). No new chain-mirrored ADT for propositions; the D47 codec (extended by eigenius#71) carries them on the chain as `eigentt:TypeExpr` payloads. Propositions are encoded using constructors EigenTT already supports:

- **Atomic propositions** use a single core-ontology declaration: `Asserts(iri) : Prop`, where `Asserts` is a uniform-parameter inductive type with **no constructors**, declared in `Sort(0)`. Different IRIs produce distinct propositions; no structural inhabitation is possible; the only ways to inhabit `Asserts(iri)` are institutional dispatch (typically Lean producing a proof term that the in-kernel checker validates) or introduction as an `eigentt:Axiom` per D46 §10.
- **Conjunction** is `Σ` with both projections in `Prop` (impredicative `Prop` keeps the resulting Σ in `Prop`).
- **Disjunction** is the standard `Sum` inductive at `Prop`.
- **Implication** is `→` (non-dependent Π in `Prop`).
- **Negation** is `→ Empty`, with `Empty` taken at `Prop`.
- **Universal / existential quantification** when needed: `Π` / `Σ` (impredicativity keeps the result in `Prop` when the body is in `Prop`).
- **Equality** between FormulaTerm-typed values: `Id` / `Refl` / `IdJ` (already primitive). When both endpoints are `Prop`-typed proof terms, proof irrelevance discharges the equality definitionally.

That's the full propositional language, expressible against EigenTT as it stands today after D46/D47/D48 landed.

**What the kernel does with these propositions.** Standard EigenTT term operations, with two D46-specific additions:

- **Typecheck them** — verify each is well-formed as a `Prop`-typed EigenTT type.
- **Normalize them** — reduce to canonical form using the existing NbE evaluator.
- **Decide definitional equality** between two propositions, within standard CIC bounds (β, ι, η where applicable), *and* — when comparing two proof terms at a `Prop`-typed proposition — short-circuit to definitionally equal via D46's proof-irrelevance rule, without normalising the proof terms.
- **Check inhabitation** when a candidate proof term is presented — the standard `t : P` judgment. This is what the Lean institution's in-process term checker exercises against `LeanProofTerm` resources, and what `ValidateJustification` (§4.3) exercises against the embedded `JustifiedBy` certificate.
- **Persist propositions as chain-mirrored resources** — propositions become content-addressed, queryable, and referenceable like any other chain artifact via the D47 codec rather than a parallel ADT.

### 4.2 The `ReasoningSentence` Resource

A `ReasoningSentence` is the chain-resident pairing of a proposition `A` with a `JustificationTerm` `t` and a `JustifiedBy t A` certificate — the agent's claim that `t : A`, together with a kernel-checkable proof of that claim. **The class is declared `subclass_of reflection:DerivedResource`** so that prior reasoning sentences are first-class citations: a later sentence's `JustificationTerm` can cite a prior one via `DerivedEvidence(prior_sentence_iri)`, and `IsDerivedAs prior_sentence_iri prop` witnesses are emitted at commit (D49 §6) on exactly the same footing as for any other derived resource. Properties:

| Property | Type | Required? | Reading |
|---|---|---|---|
| `is_a` (inherited) | `[reflection:DerivedResource, …]` | yes | Subclassing `DerivedResource` is what makes the sentence a citable derived artifact and what triggers `IsDerivedAs` witness emission. |
| `proposition` | `eigentt:TypeExpr` payload encoding a `Prop`-typed EigenTT term (D47 codec) | yes | The proposition being asserted (using the grammar in §4.1). Also serves as the sentence's `reflection:canonical_proposition` (per D49 §6) so that downstream `DerivedEvidence` citations resolve to the right `P` in the witness key. |
| `justification` | `JustificationTerm` | yes | The agent's warrant (using the constructors in §3). |
| `certificate` | `eigentt:TypeExpr` payload encoding a `JustifiedBy justification proposition` proof term | yes | The kernel-checkable certificate that the JustificationTerm justifies the proposition. The `ValidateJustification` gate (§4.3) type-checks it at commit. |
| `derivation` (inherited from `DerivedResource`) | reference to a chain-resident trace | yes (by `DerivedResource`'s `requires` list) | Points at the certificate field above — the certificate *is* the derivation. The reflection-ontology invariant is satisfied without introducing a separate `ReasoningTrace` class. |
| `subject_iri` | `core:iri` | optional | The principal Resource the sentence is *about*. **First-class EigenQL index** (the Reasoning institution declares it as a primary query key) — agents querying "what have I concluded about subject X?" hit this index directly. For decision-shaped reasoning (§6.4 below), `subject_iri` is the IRI of the decision being made; all alternative-consideration sentences and the final pick-sentence share it. |
| `refutes` | `core:iri` referencing a prior `ReasoningSentence` | optional | Marks this sentence as a belief-revision step superseding the named prior commitment. Full semantics — including which prior commitment is superseded when multiple cover the same claim, and how chain-merge resolution composes refutation chains — is the subject of the chain-merge work; this document fixes only the structural marker. |

The implicit semantic claim of a `ReasoningSentence` is "this `JustificationTerm` justifies this proposition." The `ValidateJustification` AutoOnLoad gate (§4.3) checks the certificate at commit; no auxiliary procedural step is required. Because the sentence is a `DerivedResource`, any reasoning that cites it can flow through the standard `DerivedEvidence` constructor — there is no separate "cite a prior sentence" machinery.

### 4.3 Models, satisfaction, and query classes

**Models.** The semantic framework is basic justification-logic models (Artemov & Fitting 2020 §3.1): a propositional valuation paired with a *justification evaluation* mapping each justification term to the set of formulas it justifies, with closure conditions corresponding to the Application and Sum axioms. Mkrtychev models (Mkrtychev 1997; Artemov & Fitting 2020 §3.4) are the factivity-aware variant whose truth condition bakes in `|=∗ t:F iff F ∈ t∗ and |=∗ F`; they apply specifically to the `VerifiedEvidence`-grounded fragment, where the Lean delegation guarantees that a checked proof of `F` entails `F`. The institutional dispatch does not enumerate models directly — the type-theoretic `JustifiedBy` certificate is sufficient: if it type-checks, the term well-justifies the proposition under any admissible model, by soundness of the kernel's type theory.

**Satisfaction.** Standard for justification logic: `M ⊨ t : A` iff `A` is in the justification evaluation's image at `t` under the model `M`'s closure conditions (with the Mkrtychev factivity refinement for `VerifiedEvidence`-grounded terms). The kernel realises this judgement type-theoretically: `JustifiedBy t A` is inhabited iff `t` justifies `A`.

**Query classes.** Three, with the standard dispatch roles per D14:

| Query class | Dispatch role | Behaviour |
|---|---|---|
| `ValidateJustification` | `AutoOnLoad` | Fires on every `ReasoningSentence` commit. Decodes the `proposition` and `certificate` via the D47 codec; type-checks the proposition at `Prop`; type-checks the certificate against `JustifiedBy justification proposition`. Returns `Verdict::Holds` if the certificate type-checks, `Verdict::Fails` (with the kernel's type error pinned to a specific subterm) otherwise. Cross-institution Lean delegation is embedded in `ChainWitness.IsVerifiedAs` inhabitation, not in a separate procedural step: the Lean term-checker produces the witness when the underlying `VerifiedResource`'s proof is checked at *that* resource's commit, and `JustifiedBy.verified` consumes the witness during certificate type-checking. |
| `EntailmentQuery` | `OnDemand` | Given a set of committed sentences `Γ` and a candidate proposition `A`, returns whether some `JustificationTerm` over `Γ` exists for which `JustifiedBy t A` is inhabited. Used by agents and queries to ask "does the chain warrant this conclusion?" |
| `ConsistencyCheck` | `Decidable` | Returns whether a set of committed sentences is internally consistent under the institution's logic. Decidable for the propositional fragment; reports `Undecidable` for richer fragments. |

The `AutoOnLoad` gate is the load-bearing piece. Its `Verdict` becomes a first-class chain resource alongside the `ReasoningSentence` it validated, traceable via the same provenance machinery used by every other institution. A `Fails` verdict rejects the commit (consistent with D14 §6's general gating semantics); a `Holds` verdict admits the sentence with the verdict attached as evidence that the gate has spoken.

**Agent-facing dispatch.** The two `OnDemand` / `Decidable` query classes are not exposed as per-query-class MCP tools. Instead, a single generic `eigenius_institution_dispatch(institution_iri, query_class_iri, payload, …)` MCP tool dispatches into the kernel's existing `InstitutionIndex` for any D14 institution. The Reasoning institution's queries thread through the same surface — keeping the MCP tool count lean and forward-compatible for future institutions' query classes. Agent skill documentation (a separate memo, not part of this design) covers the canonical EigenQL patterns for "what have I concluded about subject X?" (`MATCH ReasoningSentence(?s) { subject_iri: <X>, … }`), which is the most common self-recall query and doesn't require an institutional dispatch at all.

### 4.4 The `TaskOutput` Resource — relocated to D50

`TaskOutput` was originally specified here as the deliverable-handle for the discipline-thesis benchmark, citing the `ReasoningSentence` chain that justified its content. On review (during D39 Phase 4 implementation), the class is justified entirely by the benchmark-evaluation work (D50/D51) and not by anything the Reasoning institution itself needs — every property (`task`, `deliverable_kind`, `payload`, `reasoning_chain`) is benchmark-shaped. Keeping it here would pollute the foundational Reasoning ontology with downstream-consumer concerns.

The class and its properties are deferred to the benchmark harness, where they belong. See D50 (benchmark evaluation approach) for the up-to-date specification.

### 4.5 The two-phase agent surface: model, then reason

The `ReasoningSentence` shape and the `JustificationTerm` constructor set both presuppose a *vocabulary*: classes and predicates that propositions are framed in, and inference rules that compositions cite. None of this vocabulary is built-in beyond the spanning core / reflection / institution ontologies and the `Asserts(iri) : Prop` default — the agent's first move on any task that requires reasoning is *to author the task's vocabulary*, then commit reasoning sentences using that vocabulary.

Concretely, the agent's structured-reasoning loop has two phases:

1. **Model.** Emit ESL `class`, `property`, `axiom`, and (where useful) indexed `data` declarations for the task-specific entities, predicates, and inference rules. Commit them as a vocabulary layer. The validator catches malformed declarations at this stage.
2. **Reason.** Commit `ReasoningSentence`s using the declared vocabulary. The `ValidateJustification` gate fires per sentence.

The kernel does not distinguish "vocabulary authoring" from "reasoning" — both are chain commits, both go through validation, both contribute to the audit trail. But the *discipline pattern* the agent skill teaches is explicit about the order: trying to reason in untyped prose first and lift to ESL later would defeat the discipline (the agent would already have made its decisions before the typing constraint engages). For tasks that span an established domain (chemistry, GIS, etc.), per-family base ontologies cover the spanning concepts so the agent only authors the task-specific specifics; see the benchmark approach document for the concrete spanning vocabularies the pilot uses.

This vocabulary-authoring phase is part of what the discipline thesis measures. An agent that authors parsimonious, well-formed task vocabularies and then reasons over them is exercising the discipline at both levels; an agent that floods the vocabulary with thirty ad-hoc predicates is exercising it poorly. Tracking the size and shape of agent-authored ontologies per task is itself a derived experimental metric.

## 5. What counts as a justification

A `JustificationTerm` is constrained at three independent layers. All three must hold; failure at any level rejects the commit.

**Structural constraint.** The term must be well-typed at the `JustificationTerm` ADT level. The kernel's existing inductive-type validation machinery (the same machinery that handles `FormulaTerm`) checks constructor arity, constructor-name validity, and structural shape. The Reasoning institution does not need to participate.

**Referential constraint.** Each categorical-grounding constructor requires its target IRI to resolve to a resource of the matching reflection-ontology base class. The reflection ontology declares the per-class `requires` lists (`DeclaredResource` requires `declared_by`; `ObservedResource` requires `source`; `VerifiedResource` requires `derivation` and `verification`); the existing class-membership check enforces them at the resource's own commit, before any `JustificationTerm` cites it. The Reasoning institution delegates to this existing enforcement — a justification cannot reference an `ObservedResource` that itself lacks a valid `source`, nor an `eigentt:Axiom` whose statement fails to type-check in `Prop`, because the underlying resource commits would have been rejected first.

The referential constraint surfaces type-theoretically as the `ChainWitness` predicate family ([D49](d49-chainwitness-machinery.md) settles the implementation shape — per-Layer witness index derived from Trace resources, kernel-internal witness synthesis at type-check time, and the EigenTT ↔ Lean consistency check for `IsVerifiedAs`). Four `Prop`-typed predicates indexed by IRI and by the asserted proposition:

```
ChainWitness.IsDeclaredAs : core:iri → Prop → Prop
ChainWitness.IsObservedAs : core:iri → Prop → Prop
ChainWitness.IsDerivedAs  : core:iri → Prop → Prop
ChainWitness.IsVerifiedAs : core:iri → Prop → Prop
```

Witnesses are kernel-internal: ESL has no constructor for them; the kernel admits inhabitants as a consequence of the corresponding Trace-emitting commit (`DeclarationTrace` / `ObservationTrace` / `ProgramTrace` / `VerificationTrace`) succeeding. The witness and the trace are two projections of the same validator event: the trace is the chain-side audit artifact; the witness is the type-theoretic handle the kernel synthesises when type-checking a `JustifiedBy` grounding constructor. The `VerifiedResource subclass_of DerivedResource` relation in the reflection ontology propagates as a witness coercion: `IsVerifiedAs iri P → IsDerivedAs iri P`.

The proposition `P` carried by each witness is determined by the resource:

- For an `eigentt:Axiom`, `P` is the `axiom_statement` (a D47-encoded `Prop` term type-checked at the axiom's commit).
- For a `VerifiedResource`, `P` is what the proof term inhabits, computed by the kernel's Lean checker at the resource's commit. This is the one case where `P` is determined by kernel computation rather than by a declared property.
- For any other `DeclaredResource` / `ObservedResource` / `DerivedResource` that carries the optional `reflection:canonical_proposition` property, `P` is the value of that property.
- Otherwise, `P` defaults to `Asserts(iri)` — the resource asserts itself.

One canonical proposition per resource; an agent needing to ground multiple propositions declares multiple Resources.

**Semantic constraint.** The certificate must type-check as a `JustifiedBy justification proposition` inhabitant. `JustifiedBy : JustificationTerm → Prop → Type` is an indexed inductive predicate (D48) whose constructors are:

```
JustifiedBy.declared : ChainWitness.IsDeclaredAs iri P → JustifiedBy (DeclaredEvidence iri) P
JustifiedBy.observed : ChainWitness.IsObservedAs iri P → JustifiedBy (ObservedEvidence iri) P
JustifiedBy.derived  : ChainWitness.IsDerivedAs  iri P → JustifiedBy (DerivedEvidence  iri) P
JustifiedBy.verified : ChainWitness.IsVerifiedAs iri P → JustifiedBy (VerifiedEvidence iri) P
JustifiedBy.app      : JustifiedBy j1 (A → B) → JustifiedBy j2 A → JustifiedBy (App j1 j2) B
JustifiedBy.sum_l    : JustifiedBy j1 P → JustifiedBy (Sum j1 j2) P
JustifiedBy.sum_r    : JustifiedBy j2 P → JustifiedBy (Sum j1 j2) P
```

The grounding constructors consume `ChainWitness` witnesses (admitted opaquely by the chain validator); the composition constructors are pure type-theoretic combinators. `JustifiedBy.sum_l` and `JustifiedBy.sum_r` jointly realise Artemov's Sum axiom — either alternative is sufficient to inhabit `JustifiedBy (Sum j1 j2) P`. There is deliberately no Sum *eliminator*: `Sum` is a packaging operation in the term language, not a decomposable structure. `JustifiedBy` inhabits `Type` (not `Prop`) so that an explicit certificate can be stored, inspected, and re-checked, distinguishing the certificate's structure from the propositions it certifies. Proof irrelevance (D46) applies to the underlying propositional content but not to the certificate's structure — auditors can read the certificate to see *how* the justification was constructed, while the kernel ignores irrelevant proof-term variance when checking equality of propositions inside the certificate.

Together, the three constraints form a chain: the structural check ensures the `JustificationTerm` is well-formed; the referential check (delegated to reflection-ontology class invariants) ensures every grounding IRI resolves to a valid resource and produces the corresponding `ChainWitness` inhabitant; the semantic check ensures the `JustifiedBy` certificate type-checks against the proposition. Failure at any layer rejects the commit; success admits the sentence into the chain with the certificate attached as durable evidence of the warrant.

## 6. The three reasoning patterns

The constructor set above realises three patterns that cover the bulk of what agents actually do over the chain. Each maps to one of the four categorical groundings (plus optional inference structure). In every pattern the conclusion `X` is a `Prop` term per §4.1; the certificate is a `JustifiedBy t X` inhabitant the kernel type-checks at commit.

**Pattern 1 — "I observed this, hence I conclude X."** The agent grounds in an `ObservedResource` and draws a further conclusion via an inference rule:

```
JustificationTerm:  App(inference_rule, ObservedEvidence(O))
Proposition:        X
Certificate:        JustifiedBy.app  rule_cert  obs_cert
   where
     obs_cert  : JustifiedBy (ObservedEvidence O) (Asserts O)
     rule_cert : JustifiedBy inference_rule (Asserts O → X)
```

`obs_cert` is built from `JustifiedBy.observed witness` where `witness : ChainWitness.IsObservedAs O (Asserts O)` was admitted when `O` was committed. The inference rule is itself a categorically-grounded justification — `DeclaredEvidence` for a registered methodological convention (typically an `eigentt:Axiom` whose `axiom_statement` is the `→`-shaped `Prop` term `Asserts O → X`), `VerifiedEvidence` for a proved inference principle. The conclusion `X` is `Derived` because the `App` adds inferential content beyond the strict observation (see §8 for the propagation rule).

**Pattern 2 — "I derived this, hence I conclude X."** The most common shape. The agent grounds in a `DerivedResource` (the output of a prior Component invocation or institutional dispatch):

```
JustificationTerm:  App(inference_rule, DerivedEvidence(derived_iri))
Proposition:        X
Certificate:        JustifiedBy.app  rule_cert  derived_cert
```

The conclusion stays `Derived`. Long inferential chains nest these `App`-spines arbitrarily deep, just as EigenTT terms nest applications; each step has its own sub-certificate, all collapsed into a single root certificate the kernel checks once.

**Pattern 3 — "I proved this, hence I conclude X."** Two sub-cases that must be kept distinct.

If `X` is exactly the proposition the verified resource asserts:

```
JustificationTerm:  VerifiedEvidence(verified_iri)
Proposition:        X
Certificate:        JustifiedBy.verified witness
   where  witness : ChainWitness.IsVerifiedAs verified_iri X
```

The Lean term-checker produced the witness when the `VerifiedResource` was committed; no inference rule is needed; the conclusion is `Verified`.

If `X` is some further inferential consequence of the verified claim:

```
JustificationTerm:  App(inference_rule, VerifiedEvidence(verified_iri))
Proposition:        X
Certificate:        JustifiedBy.app rule_cert verified_cert
```

The conclusion is `Verified` iff the inference rule is itself grounded in `VerifiedEvidence` (transitively); otherwise `Derived`. This is the most important propagation rule: **the `Verified` category propagates only when every link in the justification — leaves and inference rules alike — is grounded in `VerifiedEvidence`**. A single non-verified link downgrades the conclusion to `Derived`.

**Inference rules are recursively grounded.** The "hence" in each pattern is itself a categorically-grounded justification, and the same propagation rule applies to it. A `DeclaredEvidence` inference rule yields a `Derived` conclusion no matter how strong the premise's justification is. A `VerifiedEvidence` inference rule applied to a `VerifiedEvidence` premise yields a `Verified` conclusion. The Reasoning institution validates each rule's grounding just as it validates every other constructor; the recursion is bounded by the chain's finite depth.

### 6.4 Trade-off reasoning as an authoring pattern (not a new ctor)

The benchmark surveys surface a recurring reasoning shape that the three patterns above don't address head-on: *"I considered alternatives A, B, C; picked B because of criteria K."* The temptation is to add a fourth `JustificationTerm` constructor like `Choice(alts, picked, rule)`. The design deliberately resists this — the 6-ctor closed set (4 groundings + `App` + `Sum`) is one of the design's strengths, and `Choice` would collapse to syntactic sugar for the pattern below anyway. Instead, the discipline is encoded as an authoring pattern using existing constructors plus a declared decision-rule axiom:

```
// Step 1 (vocabulary, Phase 1 of §4.5): declare the decision rule as an axiom.
axiom decision_rule_K :
    forall (a : alternatives:T) =>
        criteria_satisfied_better_by(a) -> is_chosen(a)

// Step 2 (reasoning, one ReasoningSentence per considered alternative):
ReasoningSentence sentence_A:
    subject_iri = decision_iri,
    proposition = "alternative_A has property_P_A",
    justification = <App-spine grounded in evidence for property_P_A>

ReasoningSentence sentence_B:
    subject_iri = decision_iri,
    proposition = "alternative_B has property_P_B",
    justification = <App-spine grounded in evidence for property_P_B>

ReasoningSentence sentence_C:
    subject_iri = decision_iri,
    proposition = "alternative_C has property_P_C",
    justification = <App-spine grounded in evidence for property_P_C>

// Step 3 (the pick):
ReasoningSentence pick:
    subject_iri = decision_iri,
    proposition = "is_chosen(alternative_B)",
    justification = App(
        DeclaredEvidence(decision_rule_K),
        // the premise: B satisfies the criteria better than A or C —
        // a derived conclusion grounded in the per-alternative sentences:
        App( App( <K-comparison-rule>, DerivedEvidence(sentence_A) ),
             App( DerivedEvidence(sentence_B), DerivedEvidence(sentence_C) ) )
    )
```

The `subject_iri = decision_iri` shared across all four sentences is what makes the cluster queryable as a unit. The query "what alternatives did the agent consider for decision D?" is plain EigenQL: `MATCH ReasoningSentence(?s) { subject_iri: <D> } RETURN [] { ?s, proposition }` returns all of `sentence_A` / `sentence_B` / `sentence_C` / `pick`. Auditors see the deliberation; the pick-sentence's `JustificationTerm` records exactly which criteria justified the choice and which alternative-evidence sentences fed in.

Two structural commitments worth noting:

- **`ReasoningSentence` is a `DerivedResource` (§4.2).** This is what lets the pick-sentence cite the per-alternative sentences via `DerivedEvidence(sentence_X_iri)` — no separate mechanism. The witness `IsDerivedAs sentence_X_iri property_P_X` is emitted at each per-alternative commit per D49 §6.
- **The decision rule lives in the chain as an `eigentt:Axiom`** (D46 §10), authored in Phase 1 of the agent's loop (§4.5). The same axiom is reusable across decisions of the same shape — *if* the agent recognises the reuse opportunity. The discipline thesis predicts that authoring decision rules explicitly (and being able to cite them across tasks) is part of what produces compounding gains.

This pattern is verbose compared to a built-in `Choice` ctor, but it composes: the same machinery handles trade-offs, hypothetical reasoning ("if H, then E follows"), abductive reasoning ("E is observed, H best explains it"), and any future decision shape we haven't anticipated. Adding a per-shape ctor for each would balloon the closed set and tie the chain's audit story to a specific decision vocabulary. The pattern keeps the closed set closed.

## 7. Comorphisms

The Reasoning institution participates in three comorphisms, each declared per D14's triadic structure. All three have identity-like middles on the constructor that carries the IRI — there is no transformation needed because the `JustificationTerm` constructor already carries the typed reference into the target institution's space.

**Reasoning ↔ Lean (bidirectional comorphism pair).** Two comorphisms — one per direction — together close the loop between EigenTT-native propositions and Lean-native proofs.

*Reasoning → Lean.* Source class: `ReasoningSentence` whose `JustificationTerm` is a root `VerifiedEvidence(verified_iri)` referencing a `LeanProofTerm`. Target class: the `LeanProofTerm` referenced by the IRI, with the proved proposition matching the sentence's proposition. The propositional alignment is direct: both institutions speak EigenTT `Prop` terms after D46, so there is no propositional translation to do. The comorphism's source-export step is the EigenTT → Lean translation specified by D30 (the forward direction).

*Lean → Reasoning.* Source class: any `lean:LeanProofTerm` that the Lean checker has validated. Target class: `reasoning:VerifiedPropositionView` (per D49 §7), reified at a content-hash-derived IRI carrying the Lean-proposition translated back into EigenTT. The comorphism's transformation step is the inverse of D30 — implemented by the Lean institution, restricted in v1 to the trivially-mappable `Prop` fragment. Dispatch role: `AutoOnLoad` on `LeanProofTerm` commits. This is the comorphism that makes `ChainWitness.IsVerifiedAs` admissible: the witness emitter (D49 §6) reads `canonical_proposition` from the reified view exactly as it reads it from any other Trace target. Lean propositions outside the v1 fragment fail the transformation; the resource remains valid as a Lean-native artifact, but no view is reified and no witness becomes admissible, with the failing `Verdict` resource carrying the diagnostic.

Together the pair makes the type-system enforcement bidirectional: Reasoning sentences citing Lean proofs are checked against Lean (forward), and Lean proofs become citable from Reasoning sentences through the EigenTT-form view (backward).

**Reasoning → numerical institutions.** Source class: `ReasoningSentence` whose `JustificationTerm` contains `DerivedEvidence(derived_iri)` constructors referencing resources produced by a numerical institution. Target class: the `DerivedResource`s from the originating institution (Symbolics, IntervalArithmetic, Catalyst, OrdinaryDiffEq, JuMP-HiGHS, and any others registered) — each carries the institution's `Verdict` as provenance, so citing the `DerivedResource` cites the verdict transitively.

**Reasoning → observed-resource fibre.** Source class: `ReasoningSentence` whose `JustificationTerm` contains `ObservedEvidence(observation_iri)` constructors. Target: the `ObservedResource` whose `source` property anchors the observation in the external world.

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
```

A `ReasoningSentence`'s epistemic category is the category of its `JustificationTerm`. The rule has two clauses worth highlighting:

**Bare grounding constructors preserve their category.** A justification consisting of just `ObservedEvidence(O)` produces an `Observed` sentence — direct citation of a measurement is observational. Similarly for the other three.

**Composition operators always produce at least `Derived`, except when every sub-justification is `Verified`.** An `App` adds inferential structure; that inference can only preserve `Verified` if every input (both the inference rule and the premise) is itself fully verified. Any non-verified leaf or non-verified inference rule anywhere in the tree downgrades the conclusion to `Derived` — verification is monotonic but does not survive non-verified composition.

The category vocabulary is unchanged from the architecture spec. What changes is that the categories become **structurally enforced projections** from the `JustificationTerm` shape, rather than separate tags applied by the user or computed from loose provenance edges. The Reasoning institution's `ValidateJustification` gate computes the category mechanically as part of admitting the term; the result is a typed Verdict resource alongside the sentence, queryable like any other chain artifact.

For resources committed without a `JustificationTerm` (which is most of the chain's existing data, and most non-reasoning commits going forward), the existing category-base-class enforcement applies as before. The new mechanism augments rather than replaces; explicit justification terms supersede provenance-based inference when both are present.

## 9. Belief, conclusion, and chain immutability

Two distinctions matter operationally.

**Belief vs conclusion.** *Belief* is the agent's provisional epistemic state — what the agent currently thinks, subject to revision as new evidence arrives. *Conclusion* is what the agent has *committed* to the chain as a `ReasoningSentence` Resource. Beliefs are not chain residents; conclusions are. The agent's working memory of beliefs is internal to the agent; only committed conclusions leave a trace.

This matters because the agent's "thinking" in the colloquial sense includes many beliefs that are never committed — hypotheses considered and rejected, partial arguments abandoned, intermediate calculations discarded. The chain records the agent's *durable* reasoning, not its *transient* reasoning. The `ValidateJustification` gate fires only on commit; intermediate beliefs need not satisfy the institution's validation rules.

**Chain immutability and belief revision.** The layer system is immutable: a commit cannot be retracted in place. An agent that changes its mind about a prior conclusion does not erase the prior commitment; it commits a new `ReasoningSentence` whose:

- `proposition` is whatever the agent now believes (often `¬P` where `P` was the prior claim),
- `justification` is built from the standard six `JustificationTerm` constructors — there is no special "refutation operator"; deriving `¬P` is just an `App` of an inference rule that produces a negation,
- `certificate` type-checks against the new proposition exactly as for any other sentence,
- `refutes` points at the prior `ReasoningSentence` it supersedes — the structural belief-revision marker.

The chain preserves both commitments. A future query (or future agent) can see both and the `refutes` link that connects them. Auditors can ask "when did the agent change its mind about X, and what specifically superseded the earlier conclusion?" — a typed, structural answer drawn from the chain itself.

The full semantics of `refutes` — which prior commitment is superseded when multiple cover the same claim, how `refutes` chains compose across layers, how chain-merge resolves conflicting refutation orderings — is the subject of the chain-merge work. This document fixes the structural marker; the precise resolution rules are scoped to that work.

This is what the platform's "debugging cycle for thinking" looks like when applied at the chain level rather than only within a single agent session. The cycle's evidence becomes durable: every gate firing, every conclusion repaired, every belief revised in light of new evidence is a chain artifact the platform preserves.

## 10. Open questions and risks

**Axiom system positioning.** The constructor set realises a fragment of the Artemov J-family (Artemov 1995, 2008; Artemov & Fitting 2020 Chapter 2). The composition operators `App` (`·`) and `Sum` (`+`) are the two basic binary operators of the core `J0` system (Definition 2.3); the four categorical-grounding constructors play the role of Artemov's *constants* — justifications attached to leaf claims — specialised to the four epistemic categories rather than ranging over arbitrary axioms. The system is *partially factive*: `VerifiedEvidence`-grounded justifications imply truth (the Lean checker validated the proof, so the proposition holds), but the other groundings do not — a declared axiom can be wrong; an observation can be inaccurate. Positive introspection (`!` from J4 and LP) is not included; justifications do not internalise meta-claims about themselves. The system therefore sits between `J` (with constant specifications) and `JT` (which adds general factivity), with constructor-specific factivity for `VerifiedEvidence`. Richer choices — full LP (J4 + factivity), modal extensions with Fitting-style semantics (Fitting 2005), multi-agent variants — buy more expressiveness at the cost of additional `JustifiedBy` constructors. The Artemov and Fitting (2020) monograph is the canonical survey. The minimal version is adequate for the agent reasoning patterns the platform expects in the near term; richer versions can be added by extending the `JustifiedBy` constructor set or by registering additional inference rules as `eigentt:Axiom` resources, depending on which level the extension belongs at.

**Inference rules as Declared and Verified resources.** The Reasoning institution itself should declare the basic Artemov-flavoured inference rules — application schema variants, factivity for evidence-bearing justifications, category-specific introduction rules — as `eigentt:Axiom`s at registration time, citable via `DeclaredEvidence`. The natural form is an axiom statement of the appropriate `→`-shaped `Prop` term, encoded via the D47 codec and admitted on the same footing as the kernel-built-in axioms (`propext`, `Quot.sound`). Other institutions may declare additional inference rules either as `eigentt:Axiom`s (when the rule's authority is methodological) or as `VerifiedResource`s carrying Lean proofs (when the rule is itself provable from a standing axiom system). The line between "logical axiom" and "domain inference rule" is conventionally drawn; the categorical-grounding constructors handle them uniformly. A `VerifiedEvidence` inference rule is what allows a chain of inferences to preserve the `Verified` category end-to-end.

**Validator performance.** Type-checking a deep `JustifiedBy` certificate at commit time scales with the certificate's size and the cost of NbE normalisation on the embedded propositions. For agent reasoning chains of plausible depth (tens to low hundreds of constructors), validation cost should be modest. For pathological cases, the kernel's existing memoisation and incremental-NbE strategies apply; the practical limit is an empirical question the implementation surfaces.

**Cross-fibre composition.** A `JustificationTerm` may reference resources from multiple institutions via `DerivedEvidence` and `VerifiedEvidence`. The Reasoning institution honours each institution's verdict according to its dispatch role and the chain's recorded status — the type-theoretic surface (`ChainWitness.IsDerivedAs` vs `IsVerifiedAs`) makes the distinction explicit; the validator does not need to encode the foreign institutions' logics.

**Belief-revision semantics.** The `refutes` property establishes the structural marker for superseding a prior commitment (§9), but the full resolution rules — which prior commitment is superseded when multiple cover the same claim, how chain-merge composes conflicting `refutes` orderings — are deferred to the chain-merge work.

**Migration of existing data.** Existing `Derived` and `Verified` resources do not carry explicit `JustifiedBy` certificates. A bulk migration would lift their provenance-edge structure into certificates, but is not strictly required — the new mechanism augments the existing one. Whether to invest in a migration is an operational decision separable from this document's structural commitments.

**Vocabulary engineering is part of the agent's discipline.** The two-phase agent surface (§4.5) puts vocabulary authoring on the chain alongside reasoning. This makes vocabulary quality a measurable dimension — agents that author parsimonious, well-formed task vocabularies exercise the discipline well; agents that flood the vocabulary with thirty ad-hoc predicates exercise it poorly. Open empirical questions: does the discipline produce convergence on parsimonious models across independent runs, or does each agent invent a new predicate per claim? Do per-family base ontologies (concrete vocabularies authored once per domain family) suppress drift enough to make cross-run comparison clean, or do agents fight the base ontology and re-invent? These resolve only through pilot runs; the design is intentionally non-prescriptive on the parsimony question so the experiment can measure it.

**No-confusion for indexed families (eigenius#69).** A no-confusion principle for indexed inductive families would give definitionally-correct disjointness for distinct constructors and would let `JustifiedBy`'s eliminator discharge "different inference rules cannot witness the same proposition shape" obligations without an additional explicit lemma. The issue is filed as a kernel enhancement (D48 §7.2 follow-up); it simplifies but does not block this design.

**Prior art for the witness model.** Beyond the justification-logic lineage (§13), the type-theoretic shape D39 commits to — an answer that carries its own proof — has external precedent in Lai et al.'s *Dependently Typed Knowledge Graphs* ([`lai2020dependently`]), where SPARQL answers over a CIC/Coq-encoded graph are proof-carrying witnesses. That "answers as proof-carrying witnesses" shape is exactly the `JustifiedBy` certificate / `ChainWitness` model here, arrived at independently from the justification-logic direction.

## 11. Non-goals

To be explicit:

- **No kernel changes driven by D39.** The kernel remains CIC-based (EigenTT fragment); D46 (Prop + proof irrelevance + axiom-as-Resource), D47 (chain-mirrored type fragment + codec), and D48 (indexed inductive families) all landed independently of this document and provide everything D39 needs. The kernel sees `JustificationTerm` and `JustifiedBy` as ordinary inductive types, not as foundational constructs. Any apparent privilege the Reasoning institution enjoys is institution-level, not kernel-level.
- **No separate propositional ADT.** Propositions are `Prop`-typed EigenTT terms encoded on the chain via the D47 codec rather than as a new chain-mirrored ADT. The only core-ontology addition for propositions is the `Asserts(iri) : Prop` declaration — a uniform-parameter inductive with no constructors, declared at `Sort(0)`. Standard connectives use EigenTT's existing Π, Σ, Sum, Empty, Id; impredicative `Prop` keeps them in `Prop` whenever their bodies are.
- **No external witness vocabulary.** `ChainWitness` predicates are kernel-internal; ESL has no constructors for them; inhabitants are admitted only by the chain validator as a consequence of the corresponding Trace-emitting commit succeeding. Audit lives in the existing Trace classes; the witness is the type-theoretic projection of the trace.
- **No `Refutation` constructor in `JustificationTerm`.** Earlier drafts included one; this design drops it. Deriving `¬P` is just composition with a negation-producing inference rule via the existing `App` constructor; the belief-revision marker lives on `ReasoningSentence` as the `refutes` property (§4.2, §9), not in the term language.
- **No replacement of provenance edges.** The existing chain-level provenance machinery continues to track resource-to-resource derivation. `JustificationTerm`s and `JustifiedBy` certificates supplement this with explicit warrant structure; they do not replace it.
- **No positive introspection.** The `!` operator (Artemov & Fitting 2020 §2.6 — present in `J4`, `LP`, `JT4`) is not included. Justifications do not internalise meta-claims about themselves; a justification term cannot witness "this term justifies this other term." If a future reasoning pattern requires it, it would be added as an additional `JustificationTerm` constructor with a matching `JustifiedBy` rule, lifting the system into the `J4`/`LP` family.
- **No modal or dynamic-epistemic extensions.** This document covers the basic propositional fragment of justification logic. Modal and dynamic extensions are follow-up work.
- **No defeasible / non-monotonic logic.** Belief revision is a structural chain marker, not a non-monotonic logical commitment. Logical frameworks like default logic and circumscription could be admitted as their own institutions with their own term languages, related to Reasoning via declared comorphisms.
- **No first-order logic institution.** The platform's existing absence of a FOL institution is not closed by this document.
- **No agent-extensibility of the `JustificationTerm` ADT.** Agents compose using existing constructors; new constructors require a versioned ADT update with associated design review. The agent's controlled-extension path is to author new institutions whose derived outputs the existing `DerivedEvidence` constructor can cite, and whose declared inference rules the existing `DeclaredEvidence` constructor can cite (typically as `eigentt:Axiom`s).

## 12. Relationship to other design documents

- **[D6 execution architecture](d6-execution-architecture.md)** — reasoning traces describe *what happened* during execution; justification terms describe *with what warrant* a claim is asserted. The two are related but distinct. A trace can be lifted into a justification term (the constructors that wrap institutional Verdicts and observations are exactly the trace's structural elements made into typed warrant-bearing claims), but the lifting is a separate operation; traces do not become justifications automatically.
- **[D14 institution realisation](d14-institution-realisation.md)** — the Reasoning institution is a normal institution per D14's three-method trait. Its query classes follow the standard `OnDemand` / `AutoOnLoad` / `Decidable` dispatch-role mechanism. Its Verdicts integrate into the chain's audit story without special-casing.
- **[D28 Lean 4 as institution](d28-lean-4-as-institution.md)** — the `VerifiedEvidence` constructor references resources of class `VerifiedResource`, typically `LeanProofTerm`s; the cross-institution delegation embeds in `ChainWitness.IsVerifiedAs` inhabitation, which is produced when the Lean checker validates a `LeanProofTerm` at commit. With D46 landed, the kernel's `Prop` universe is the shared substrate both institutions use for propositions, so there is no translation between disjoint type systems.
- **[D32 chain-mirrored EigenTT inductives](d32-chain-mirrored-mini-tt-inductives.md)** — `JustificationTerm` is another chain-mirrored inductive sitting alongside `FormulaTerm` in the chain's interlingua catalogue. The encoding follows D32's pattern; the kernel's validation machinery handles both uniformly. D32 establishes the constructor-shape vocabulary that subsequent type-fragment work (D47) generalises into a full Exp↔Value::Json codec.
- **[D46 Prop universe + axiom framework](d46-prop-universe-and-proof-irrelevance.md)** — D46 lets D39 declare propositions in `Prop`, gives proof irrelevance for free on `Prop`-typed proofs, and supplies the `eigentt:Axiom` chain class that hosts both built-in axioms (`propext`, `Quot.sound`) and the Reasoning institution's declared inference rules (§10).
- **[D47 chain-mirrored EigenTT type fragment](d47-chain-mirrored-eigentt-type-fragment.md)** — D47 (with eigenius#71's term-level extension) provides the chain-resident encoding for both the propositional language (§4.1) and the `certificate` field of `ReasoningSentence` (§4.2). The codec decodes both back to kernel `Exp`s for type-checking at commit.
- **[D48 indexed inductive families](d48-indexed-inductive-families.md)** — D48 is what makes the type-theoretic `JustifiedBy : JustificationTerm → Prop → Type` expressible: the first-order pattern unifier handles index unification during certificate type-checking; dependent constructor checking handles the `ChainWitness`-consuming grounding constructors; per-arm index-coherence and singleton-elim (Case B) for propositional indices clean up elaboration. D48's open follow-up (eigenius#69, no-confusion principle) is the one further kernel enhancement that would simplify some `JustifiedBy` eliminator derivations.
- **[D49 `ChainWitness` machinery](d49-chainwitness-machinery.md)** — implementation memo for the `ChainWitness` predicate family introduced in §5. Settles where the witness table lives (per-`Layer`, derived from the Layer's Trace resources, materialised lazily via `OnceLock`), the witness-synthesis algorithm (parent-chain walk by `(category, iri, prop_hash)` key; first hit wins; misses surface as type errors at type-check time with a precise diagnostic), the trace-emission relationship (no new D41 hooks — witnesses are projections of `DeclarationTrace` / `ObservationTrace` / `ProgramTrace` / `VerificationTrace` resources), and the Lean checker hook for `IsVerifiedAs` (a v1 consistency check between the chain-declared EigenTT proposition and the Lean proposition via the D30 forward translation, with backward translation deferred to v1.1). Required reading for D39 implementation.
- **`reflection` ontology (`ontologies/reflection/reflection-ontology.json`)** — the load-bearing chain substrate D39 sits on. The four epistemic-category base classes (`DeclaredResource` / `ObservedResource` / `DerivedResource` / `VerifiedResource`, with Verified `subclass_of` Derived), their per-class `requires` invariants, the `EpistemicStatus` value vocabulary, and the parallel Trace event classes (`DeclarationTrace`, `ObservationTrace`, `ProgramTrace`, `VerificationTrace`) are already declared here and are exactly what the Reasoning institution validates against. D39 does *not* introduce a parallel hierarchy: the `ChainWitness` predicate family (§5) is a type-theoretic projection of the reflection ontology's class-membership facts, with witness inhabitation triggered by Trace-emitting commits. The one extension D39 adds is an optional `reflection:canonical_proposition` property on `DeclaredResource` / `ObservedResource` / `DerivedResource` for resources that advertise an explicit `Prop` statement beyond the default `Asserts(iri)`. `EpistemicStatus` (the status-value vocabulary: `declared`, `observed`, `derived`, `verified`) is parallel to but distinct from class membership and remains a property-value vocabulary for trace annotations, not a justification witness.
- **The four epistemic categories** specified in the architecture documents and realised in the reflection ontology — D39 aligns the `JustificationTerm` interlingua structurally with the four categories: each grounding constructor (`DeclaredEvidence`, `ObservedEvidence`, `DerivedEvidence`, `VerifiedEvidence`) references a resource of the corresponding base class. The propagation rule in §8 computes a sentence's category mechanically by walking the term tree. The categories' meaning is unchanged from the architecture spec; the structural enforcement becomes stricter for resources that commit justification terms.

## 13. References

**Justification logic — primary sources.**

- S. Artemov (1995). *Operational modal logic.* Technical Report MSI 95-29, Mathematical Sciences Institute, Cornell University. The foundational paper introducing the Logic of Proofs (LP); first internalisation of evidence terms into a modal-style provability logic.
- S. Artemov (2001). "Explicit provability and constructive semantics." *Bulletin of Symbolic Logic*, 7(1), 1–36. Journal-published treatment of LP with semantic and proof-theoretic details; widely cited in lieu of the 1995 technical report.
- S. Artemov (2008). "The Logic of Justification." *The Review of Symbolic Logic*, 1(4), 477–513. Canonical modern reference for justification logic; basis for the `JustificationTerm` constructor set (§3) and the `JustifiedBy` predicate (§5).
- S. Artemov and M. Fitting (2020). *Justification Logic: Reasoning with Reasons.* Cambridge University Press, *Cambridge Tracts in Mathematics* 216. Comprehensive monograph covering LP, JT and JT4, factivity, modal extensions, multi-agent variants, and applications. The recommended deep reference. A local copy is available at `references/publications/dokumen.pub_justification-logic-reasoning-with-reasons-1108661106-9781108661102.pdf`, with a layout-preserving plain-text rendering alongside it at `references/publications/justification-logic-artemov-fitting-2020.txt` for in-repo grep / search.
- S. Artemov and M. Fitting. "Justification Logic." *Stanford Encyclopedia of Philosophy.* First published 2011, periodically revised. Survey article; recommended entry point for readers new to the area.

**Model theory.**

- A. Mkrtychev (1997). "Models for the logic of proofs." In S. Adian and A. Nerode (eds.), *Logical Foundations of Computer Science (LFCS '97)*, LNCS 1234, Springer, 266–275. Introduces the basic-model semantics referenced in §4.3 — propositional valuations paired with justification functions, sufficient for the validation gate's syntactic axiom-checking.
- M. Fitting (2005). "The logic of proofs, semantically." *Annals of Pure and Applied Logic*, 132(1), 1–25. The richer Kripke-style semantics for justification logic referenced in §10 as a foundation for potential modal extensions; supersedes Mkrtychev models where modal-frame structure is needed.

---

*This is a v2 design proposal. The structural commitments — justification logic as the foundation, the closed `JustificationTerm` constructor set, the type-theoretic `JustifiedBy` predicate with `ChainWitness`-mediated grounding, the three-layer constraint story, the epistemic-category propagation as a projection from justification structure — are the load-bearing design decisions and should be the focus of review. The specific choice of Artemov LP as the base axiom system, the constructor list in §3, and the open questions in §10 are open to revision.*
