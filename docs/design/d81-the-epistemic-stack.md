# D81 — The epistemic stack: justification logic, witnesses, traces, grades

**Status: in progress.** An organized description of the current architecture, with code
references. Plan: [`docs/notes/d81-epistemic-stack-analysis-plan.md`](../notes/d81-epistemic-stack-analysis-plan.md).

**The implementation is the guiding artifact.** Everything below is read from code, ontologies and
tests. Design documents are consulted only in §4's provenance pass, and only to explain a shape the
code leaves unexplained; they record intentions at the time a feature was planned and have no
authority over what the system now is.

| § | content | phase | state |
|---|---|---|---|
| 1 | the concept inventory | P0 | **done** |
| 2 | four lifecycles | P1 | **done** |
| 3 | the boundary map | P2 | **done** |
| 4 | provenance — why the shapes exist | P3 | **done** |
| 5 | findings | P4 | **done** |
| 6 | the encoding ontology — a counter-example | — | **done** |

---

## 0. Orientation — two stacks, not one

The seven encodings of §1.1 are not one abstraction seen seven ways. They fall into **two groups
that never touch each other.**

### The proving stack — does the work

| abstraction | role | produced by | consumed by |
|---|---|---|---|
| `reflection:*Trace` (the 5 attestation classes) | **the evidence.** A chain record that an event happened, pointing at the resource it attests | authors (`DeclarationTrace` in ESL); the reasoning institution mints `VerificationTrace` on a passing gate | `trace_category` → witness admission |
| `WitnessCategory` + `WitnessKey` | **the lookup key** — `(category, grounded IRI, proposition hash)`. Kernel-internal, **never persisted** | computed from traces, plus two hard-coded self-attesting classes | `layer_admits_witness` |
| `witness:Is{Declared,Observed,Derived,Verified}As` | **the proposition.** `Prop`-valued inductives with **zero constructors** — unconstructible at the surface | declared in `ontologies/justification/justification.esl` | the declared type of `JustifiedBy`'s evidence argument |
| `JustifiedBy.{declared,observed,derived,verified}` | **the certificate.** The term an author writes and the kernel must check | authored ESL; built by `ClaimGrader` | the kernel type checker |

This runs end to end: a trace grounds a key, the key inhabits the proposition, the proposition is the
argument the certificate needs. Break any link and certificates stop type-checking. **This is where
"verified" means something.**

### The labelling stack — is read by nothing

| abstraction | role | produced by | consumed by |
|---|---|---|---|
| `reflection:{Declared,Observed,Derived,Verified}Resource` | **self-description**, and the hook that pulls in `requires` obligations | **eight** writers — the ESL compiler stamps *every* compiled resource, plus institution dispatch, importers, graders, bootstrap (§1.6) | the validator's `requires` / `recommends` — **and nothing else** |
| `reflection:epistemic_status` + `reflection:epistemic:*` individuals | **a value**, for slots where `is_a` is already spent | one Rust writer (`kernel/src/server/programs.rs:193`), plus authored ESL | other *ontologies*, via `allows_only` (`lexicon:grade`, objective milestones). **Zero Rust readers** |
| `Grade` (`crates/eigenius-reasoning`) | intended as a construction-time projection | four sites | **nothing** — write-only (§5.3) |

No reader anywhere grants an entitlement on the strength of an epistemic class. `is_a
VerifiedResource` is an unbacked self-description, not a forged capability — which is why §5.1 rates
the class/witness split *fine* rather than dangerous.

### The two are written together and diverge silently

`kernel/src/server/programs.rs:185-195` sets **three encodings at once** on one resource:

```rust
types.push(wk::DERIVED_RESOURCE);                    // the class
output.set(DERIVATION, trace_iri);                   // the evidence pointer
output.set(EPISTEMIC_STATUS, wk::EPISTEMIC_DERIVED); // the value
```

That is the clearest evidence the seven are **parallel projections written by producers**, not
alternative views computed from one source. Nothing recomputes one from another, and nothing checks
they agree — because only one of them is ever read.

**Read §§1–5 with this split in view.** Most of what looks like redundancy is the labelling stack,
which is inert; most of what carries risk is the proving stack, whose entire unifying concept lives
in three Rust lists (§5.2).

---

## 1. The concept inventory

### 1.1 The four-way distinction is encoded nine times

Every row below is a distinct artifact naming *declared / observed / derived / verified*. They are
not synonyms — each occupies a different syntactic category, listed in the third column — but
nothing in the system states the mapping between them in one place.

| # | encoding | kind | defined at |
|---|---|---|---|
| 1 | `reflection:{Declared,Observed,Derived,Verified}Resource` | **class** — `is_a` membership | `ontologies/reflection/reflection-ontology.json` |
| 2 | `reflection:epistemic:{declared,observed,derived,verified}` | **individuals** of `reflection:EpistemicStatus`, held by the `reflection:epistemic_status` property | same; instances pinned by `kernel/src/bootstrap/mod.rs:1280` |
| 3 | `reflection:{Declaration,Observation,Program,Verification,ExternalExecution}Trace` | **event kind** — what happened | same; **five** classes |
| 4 | `WitnessCategory::{Declared,Observed,Derived,Verified}` | **kernel enum**, a `WitnessKey` component | `kernel/src/witness/mod.rs:47` |
| 5 | `witness:Is{Declared,Observed,Derived,Verified}As` | **inductive predicate** — the proposition | `ontologies/justification/justification.esl` |
| 6 | `JustifiedBy.{declared,observed,derived,verified}` | **constructor** — the certificate | same |
| 7 | `Grade::{Declared,Observed,Derived,Verified}` | **crate enum** | `crates/eigenius-reasoning/src/grade.rs:69` |
| 8 | `JustificationTerm.{Declared,Observed,Derived,Verified}Evidence` | **constructor** — the evidence term | `ontologies/justification/justification.esl` |
| 9 | `Ground::{Declared,Observed,Derived,Verified}` | **crate enum**, mapping *from* row 8's ctor names | `crates/eigenius-reasoning/src/project.rs:71` |

Rows 8 and 9 were missed by the first pass and found by asking what *other* epistemic categories
exist. Row 8 matters: `JustifiedBy`'s grade-carrying constructors (row 6) each take a `witness:Is*As`
(row 5), while `JustificationTerm`'s evidence constructors (row 8) are the *justification* side of
the same four-way. Row 9 is a second dead-ish projection alongside row 7, reading row 8 by
constructor **name**.

### 1.1a Axes that are *not* repeats of the four

Distinguished here because conflating them with §1.1 is how a redundancy count inflates.

| vocabulary | axis it measures | relation to the four |
|---|---|---|
| **`Warrant::{Declared, Parsed}`** (`crates/eigenius-reasoning/src/grade.rs:83`) | **what warrants the assertion** | **finer than the grade** — both project to `Declared` via `Warrant::grade()`. `#[non_exhaustive]`, documented as *"the growth axis"* |
| `enc:Claim` kinds — `Finding`, `Observation`, `Classification`, `Hypothesis`, `Suggestion`, `Assertion` | **discourse role** | orthogonal by design (§2.1) |
| `institution:Verdict` — `Holds` / `Fails` / `Undecidable`, plus `VerdictReading`, `VerdictPredicate`, `ClaimVerdict` | **a gate decision** | not a grade; one tri-state in four representations |
| `objective:WitnessKind` | how a milestone is *expected* to be witnessed | *"operational planning metadata with no reflection analog"* — a third sense of "witness" |
| `objective:acceptance_grade` | the **target** grade of an open goal | reuses `reflection:EpistemicStatus` with `allows_only`; explicitly distinguished from `epistemic_status`, the **actual** grade |
| `lexicon:grade` | the grade of a lexical entry | reuses the same individuals — *"not a parallel enum"* |
| `enc:confidence` | pipeline confidence, 0..1 | *"Advisory; not a grade"* |
| `enc:SelectionAuthority`, `BindingAuthority`, `GapDisposition`, `CutKind` | who chose, and what became of what could not be explained | the abductive record (§6) |

**`Warrant` is the one that matters**, because it is the vocabulary §3.5 of D82 says is missing,
already prototyped: two distinct *relations between a proposition and its evidence* — the source
asserts it, versus the parser produced it from a span — collapsing to one grade because the grade
cannot express the difference. It lives in one crate, in a Rust enum, marked as an axis expected to
grow.

Two observations, both mechanical:

**The 5→4 collapse in row 3 lives in a Rust `match`, not in the ontology.** `trace_category`
(`kernel/src/layer/witness_admission.rs:179`) maps the five trace classes onto four categories, sending
`ExternalExecutionTrace → Declared` (eigenius#205). The ontology carries no class, property or
relation expressing *"this trace kind grounds that grade"* — see §1.3.

**Rows 4 and 7 are the same four variants in two crates, and never meet.** `grep WitnessCategory`
over `crates/eigenius-reasoning/src/` returns nothing: there is no conversion in either direction.
Whether this matters is §4's question; that they are unconnected is a fact.

### 1.2 `reflection:Trace` names three unrelated families

The 28 reflection classes include 17 whose name ends in `Trace`. They do not form one hierarchy.

| family | members | parent |
|---|---|---|
| **execution-tree nodes** — the *inside* of one program run | `Empty`, `Component`, `Pure`, `Comorphism`, `Let`, `Map`, `Reduce`, `Case`, `Construct`, `Project`, `Seq` | `reflection:Trace` |
| **production records** — that a run happened | `ProgramTrace`, `ExternalExecutionTrace` | `reflection:ProductionTrace` |
| **standalone attestations** | `DeclarationTrace`, `ObservationTrace`, `VerificationTrace` | **none** |
| unplaced | `FieldTrace` | none |

`reflection:Trace` is therefore *not* the root of the trace concept — it is the root of the
execution-node family only. `ProductionTrace` has no parent either, and the three standalone
attestations are siblings of nothing.

Verified by walking `subclass_of` across the ontology: eleven classes point at `reflection:Trace`,
two at `reflection:ProductionTrace`, and eleven classes in the file have no `subclass_of` at all —
among them `DeclarationTrace`, `ObservationTrace`, `VerificationTrace`, `ProductionTrace`, `Trace`
and `FieldTrace`.

### 1.3 The unifying concept exists only as a function

The set that matters to the witness machinery — *trace kinds that ground a witness* — is exactly
`trace_category`'s five match arms. It has:

- no class,
- no `subclass_of` edge joining its members (they span two families and a parentless group),
- no property marking membership.

`is_witness_candidate` (`kernel/src/layer/witness_admission.rs:156`) and `trace_category` (`:179`) are
where the concept is defined. Both are Rust.

### 1.4 "Witness" names two unrelated things — **settled**

| | `ChainWitness` | merge `Witness` |
|---|---|---|
| what it is | evidence inhabiting a `JustifiedBy.*` argument | a `MergeComorphism` realising the **universal arrow** at a conflicting IRI |
| shape | a `WitnessKey` — `(category, iri, proposition hash)` | a function `(A, A, Option<A>) → A` |
| where | `kernel/src/witness/mod.rs`, `kernel/src/layer/witness_admission.rs`, `kernel/src/nbe/check/witness.rs` | `kernel/src/layer/merge/witnessed.rs` (D20 §6.1) |
| persisted | **no** — derived from Trace-class resources on demand | the comorphism resource is; the application's result is |
| errors | `WitnessTypeMismatch`, `WitnessTargetNotResolvable`, `WitnessTermNotAFunction` — all `MergeError` | *(same list — they belong to merge)* |

**They share no code.** `kernel/src/layer/merge/witnessed.rs` imports `conflict`, `lca`,
`MergeError`, `LayerTopology`, `LayerId`, `Iri`, `Resource`, `well_known` — and nothing from
`crate::witness`. The collision is two disciplines' senses of the English word: *evidence* in
justification logic, *universal arrow* in category theory.

One consequence worth recording now: every `Witness*` error variant in the tree belongs to **merge**,
not to the justification stack. A reader searching `WitnessTypeMismatch` for a certificate failure
finds merge code.

### 1.5 Name census — the artifacts

**Kernel — `kernel/src/witness/mod.rs`**

| name | line | role |
|---|---|---|
| `WitnessCategory` | 47 | the four grades, as a key component |
| `WitnessKey` | 91 | `(category, grounded IRI, proposition hash)` |
| `hash_proposition_exp` | 137 | hash from a term |
| `hash_proposition_value` | 149 | hash from an encoded chain value |
| `alpha_canonicalize_proposition_json` | 181 | α-normalisation before hashing |

**Kernel — `kernel/src/layer/witness_admission.rs`** *(despite the name, no index is materialised)*

| name | line | role |
|---|---|---|
| `layer_admits_witness` | 66 | does **this** layer admit the key |
| `is_witness_candidate` | 156 | can this resource ever admit one |
| `trace_category` | 179 | **the 5→4 map** (private) |
| `default_asserts_proposition_hash` | 359 | the `Asserts(iri)` default |
| `default_asserts_proposition` | 386 | as above, as a term |
| `lookup_chain_witness` | 423 | chain walk, first-hit-wins |
| `synthesize_chain_witness` | 479 | build the value the checker splices in |

**Kernel — the checker seam:** `kernel/src/nbe/check/witness.rs::try_synthesize_chain_witness` →
`EffectHooks::synthesize_chain_witness` → `kernel/src/program/check_hooks.rs:86` → the above.

**Reasoning ontology — `ontologies/justification/justification.esl`**

Inductives: `reasoning:JustificationTerm`, `reasoning:JustifiedBy`,
`witness:Is{Declared,Observed,Derived,Verified}As`.
Classes: `ReasoningSentence`, `VerifiedPropositionView`, `JustificationProjection`,
`ConsistencyRequest`, `EntailmentRequest`, `ProjectionRequest`.
Institution resources: `reasoning_institution`, `ef_justification`, and four query classes —
`qc_validate_justification`, `qc_entailment_query`, `qc_consistency_check`,
`qc_project_justification`.

`JustifiedBy` has **eight** constructors: the four grade-carrying ones plus `app`, `sum_l`, `sum_r`,
`spec_poly` — the logical connectives. Only the first four take a `witness:Is*As` argument.

**Reasoning crate — `crates/eigenius-reasoning/src/`**

`Grade`, `Ground`, `Warrant`, `KindVerdict`, `ClaimVerdict`, `ClaimSource`, `GradedClaim`,
`GradeError`, `KindRecord`; traits `ClaimGrader`, `KindClassifier`, `DocumentIngestion`;
graders `DeclaredClaimGrader`, `ParsedClaimGrader`; `DerivedClaimLander`, `ChainRuleApplication`,
`ProseModusPonens`, `InProcessIngestion`, `ReasoningInstitution`.

### 1.6 Where a grade is written

`ClaimGrader::grade` (`crates/eigenius-reasoning/src/grade.rs:197`) is the only *abstraction* for
producing a graded claim. It is not the only *writer*. These non-test sites set a
`reflection:*Resource` class directly:

`kernel/src/bootstrap/mod.rs` · `kernel/src/esl/compile.rs` · `kernel/src/institution/dispatch.rs`
· `kernel/src/layer/index.rs` · `crates/eigenius-reasoning/src/grade.rs` ·
`crates/eigenius-schemaorg/src/{convert,report}.rs` · `crates/eigenius-obograph/src/convert.rs` ·
`crates/runtime-substrate/src/facade.rs`

Whether any of these are *wrong* is §2's and §4's question — several are plainly legitimate
(`kernel/src/esl/compile.rs` stamping author-declared resources, `kernel/src/institution/dispatch.rs` stamping a
derivation it just produced). What §1 records is that the decision has **eight** implementations
and one trait, and the trait is used by one of them.

---

## 2. Four lifecycles

Each traced against an artifact that exists in the tree. The question is the same each time: from
*something happened* to *a certificate can cite it*, what runs?

### 2.0 Two carriers, computed from disjoint inputs

Every lifecycle has **two independent carriers** of its grade, and nothing reconciles them.

| carrier | how it is established | who reads it |
|---|---|---|
| **class membership** — `is_a reflection:*Resource` | asserted by the author, or inherited from a class that subclasses it | the validator's `requires` / `recommends` |
| **witness admission** — a `WitnessKey` | computed by `layer_admits_witness` from traces and two hard-coded self-attesting classes | the type checker, when a `JustifiedBy.*` argument needs inhabiting |

`layer_admits_witness` (`kernel/src/layer/witness_admission.rs:66`) **never consults the epistemic
classes.** Its self-attesting route matches on exactly two class IRIs —
`reasoning:ReasoningSentence → Verified` and `reflection:InstitutionEmittedDerivation → Derived` —
and its trace route matches on `trace_category`. `reflection:DeclaredResource` and its three
siblings appear nowhere in `kernel/src/validation/` except test fixtures.

So a resource may be `is_a VerifiedResource` and admit no witness, or admit a `Verified` witness
while carrying no epistemic class at all. Both states validate.

**The obligations the classes do impose are asymmetric**, and inverted with respect to how much
evidence the grade implies:

| class | `requires` | `recommends` |
|---|---|---|
| `DeclaredResource` | `declared_by` | `rationale`, `timestamp` |
| `ObservedResource` | `source` | `source_irl`, `observed_at`, `timestamp` |
| **`DerivedResource`** | **— nothing —** | `derivation` |
| `VerifiedResource` *(⊂ Derived)* | `derivation`, `verification` | — |

`DerivedResource` — the grade meaning *the kernel computed this* — is the only one of the four with
no required property. That `derivation` is recommended rather than required is deliberate, is
documented in the ontology itself (`ontologies/reflection/reflection-ontology.json:32`) and at
`kernel/src/validation/mod.rs:1518`, and is pinned by a test named for the behaviour:
substrate-produced resources from `FIBER … INTO` and comorphism reify outputs "are derived by
construction but may not have a kernel-generated `ProgramTrace` yet".

**An earlier draft called this an "inverted obligation table". §5 refutes that framing**, and the
refutation is worth keeping in view while reading the rest of this section: the *abstract* class is
permissive, but **every concrete derived-by-kernel path carries a requirement of its own** —
`InstitutionEmittedDerivation` requires `from_subject`, `ReasoningSentence` requires proposition +
justification + certificate, `VerifiedResource` requires `derivation` + `verification`. Requiring
`derivation` on the base class would force `ReasoningSentence` and `VerificationTrace` to point the
slot at themselves, a fiction the tree declines twice in writing.

### 2.1 Declared — `demo/prose-to-formulas-v2/claims-intact.esl`

```esl
resource v2:claim_1 : encoding:EncodedClaim, encoding:Observation {
    reflection:canonical_proposition = type_expr( … );
    reflection:declared_by = "urn:eigenius:reflection:agent:unattributed";
}
resource v2:trace_1 : reflection:DeclarationTrace {
    reflection:resource = "urn:eigenius:demo:v2:claim_1";
    …
}
```

1. **Grade by class.** `enc:EncodedClaim : reflection:DeclaredResource`
   (`ontologies/encoding/encoding.esl:366`) — inherited, not stated. `declared_by` is required and
   present.
2. **Two axes on purpose, and correctly separated.** `encoding:Observation` here is a **discourse
   kind** (`enc:Observation : enc:Claim`), not an epistemic grade. An earlier draft of this section
   called that a harmful name collision with `reflection:ObservedResource`; **§5 refutes it** — the
   `reflection` ontology declares no class named `Observation`, the two identifiers differ in
   namespace *and* in string, and the orthogonality is stated at the point of definition
   (`ontologies/encoding/encoding.esl:369-379`) and again per member: `enc:Finding` reads *"Discourse
   kind, not epistemic grade: a finding may land Derived or Observed"*. The convention is working as
   designed.
3. **Witness by trace.** `v2:trace_1` is a `DeclarationTrace` whose `reflection:resource` points at
   the claim. `layer_admits_witness` takes the trace-attested route: `any_trace_targeting` finds it
   (`reflection:resource` is `core:resource`-typed, hence in the triple index), `trace_category`
   maps `DeclarationTrace → Declared`, and `emit_from_trace` builds the key from the *target's*
   `canonical_proposition`.
4. **Consumed by** `JustifiedBy.declared`, whose `witness:IsDeclaredAs(iri, P)` argument the checker
   inhabits via `try_synthesize_chain_witness`.

**Nothing requires step 1 and step 3 to agree.** The claim would validate without the trace, and the
trace would grant its witness even if the claim carried no epistemic class.

### 2.2 Observed — `experiments/publications/wrn-helicase/chain/08-phase3-invivo-mechanism.esl`

```esl
class wrn:XenograftTable : reflection:ObservedResource { … }
```

Same shape as Declared, one rung along: the grade arrives by subclassing, `source` is the required
property, and admission is trace-attested through `ObservationTrace`.

Both instance-level and class-level assertion are in use in the same file —
`wrn:vivo_seed_control : bench:ToolArtifact, reflection:DeclaredResource` states its grade directly.
Neither form is privileged.

### 2.3 Derived — `reflection:InstitutionEmittedDerivation`, statistics

The only lifecycle where the kernel both produces the resource and grants the witness.

1. **Production.** `dispatch_auto_on_load_for_layer` (`kernel/src/commit/phases.rs:411`) fires the
   institution's AutoOnLoad QueryClass during commit; statistics emits one derivation per ANOVA
   effect, carrying `from_subject` (required) and `canonical_proposition` (recommended).
2. **Witness, self-attesting.** `emit_from_institution_derivation`
   (`kernel/src/layer/witness_admission.rs:272`) reads `canonical_proposition` off the derivation and
   keys `Derived` against the derivation's **own** IRI. No trace is consulted.
3. **Silent on absence.** `canonical_proposition` is only *recommended*. A derivation without it
   returns `None` — no witness, no error. The doc comment names two causes: "kernel merge dropped
   it, or the institution didn't supply one".

### 2.4 Verified — two intended producers, one of which does not exist

`Verified` is the only grade with more than one route, and the routes are in very different states.

#### Route A — the reasoning institution (works)

1. **Witness, self-attesting.** `emit_from_reasoning_sentence`
   (`kernel/src/layer/witness_admission.rs:255`) grants `WitnessCategory::Verified` to **any**
   `ReasoningSentence` whose `reasoning:proposition` hashes. It performs **no check that the
   sentence's certificate validated.**
2. **The guard is elsewhere.** `qc_validate_justification` is declared AutoOnLoad
   (`ontologies/justification/justification.esl`), so it fires on every `ReasoningSentence` commit; a
   `Fails` verdict becomes a `ValidationError { rule: InstitutionValidation }`
   (`kernel/src/commit/phases.rs:460`) and blocks the commit. **Therefore any committed
   `ReasoningSentence` has a validated certificate**, and step 1 may skip re-checking.

**That invariant is load-bearing and is stated in no single place.** It is the conjunction of four
facts in four files: the QueryClass's `dispatch_role`, the dispatch phase running it, `Fails`
blocking the commit, and `emit_from_reasoning_sentence` relying on all three by omission. Nothing
links them; nothing fails if a link is removed.

3. **The other trace producer.** A passing `ValidateJustification` also mints a
   `reflection:VerificationTrace` (`crates/eigenius-reasoning/src/validate.rs:151`). This is the
   *only* producer of that class in the tree.

One consequence is already known and belongs to a sibling document rather than here: merge ends at
`store_layer` and never runs `dispatch_auto_on_load_for_layer` (D80 §3.3), so a `ReasoningSentence`
materialised into a merge layer is not re-validated against the merged chain. It *was* validated —
on its branch, against a different environment. That is D77's subject, not a defect of this
lifecycle.

#### Route B — the Lean institution (absent in the middle)

The canonical verification story — a machine-checked proof — reaches the grade through a three-step
path of which **only the first step exists**.

**It is in the commit sequence.** `lean:qc_proof_check` declares **both** dispatch roles —
`auto_on_load` and `on_demand` (`ontologies/lean/lean-institution.eigon.json`) — gated on
`query_class = lean:LeanProofTerm`. So committing a proof term fires the checker through the same
`dispatch_auto_on_load_for_layer` phase the reasoning institution uses, and a `Fails` verdict blocks
the commit by the same mechanism (§2.4 route A step 2). Lean is not an on-demand-only query.

| step | intended | actual |
|---|---|---|
| 1. Lean checks the proof term, returns a `Verdict` | `do_proof_check` (`crates/eigenius-lean/src/institution.rs:306`) | **exists**, and runs at commit |
| 2. a `lean_to_reasoning` comorphism reifies a `reasoning:VerifiedPropositionView` | D49 §7 | **does not exist** — the identifier appears nowhere in the tree except one ontology comment |
| 3. the witness emitter looks the view up by `source_verified_resource` and reads its `canonical_proposition` | D49 §7 | **does not exist** — `target_proposition_hash` (`kernel/src/layer/witness_admission.rs:317`) has three slots: `canonical_proposition`, `reasoning:proposition`, and the `Asserts(iri)` default. The view is not among them |

Nothing Lean-side stamps `reflection:VerifiedResource`, mints a `VerificationTrace`, or reifies a
view — `grep` over `crates/eigenius-lean/` for all three returns nothing. A `Holds` verdict is a
leaf resource. This was eigenius#160.

**Closed `2026-09-03`, and rows 2 and 3 stayed unbuilt.** The route the table assumes — reify a
view, then have the emitter read it — was replaced by D74's forward externalization: the claim's
`canonical_proposition` is already in EigenTT before the check runs, so there is nothing to
translate back. `do_proof_check` emits a `prov:VerificationTrace` naming the claim, the kernel
commits it beside the Verdict, and `target_proposition_hash`'s **first** slot —
`canonical_proposition` on the claim — is the one that answers. The view remains declared and
unused.

**So the sharper statement is that the AutoOnLoad gate is used as a veto and not as a promoter.**
Two institutions share the dispatch role and diverge on what a *pass* means:

| | on `Fails` | on `Holds` |
|---|---|---|
| `reasoning:qc_validate_justification` | blocks the commit | **mints a `VerificationTrace`** (`crates/eigenius-reasoning/src/validate.rs:151`) |
| `lean:qc_proof_check` | blocks the commit | commits the `Verdict` as provenance and **nothing else** |

The asymmetry is not in the dispatch machinery, which is identical. It is that AutoOnLoad has no
declared notion of a *post-condition on success* — what a passing gate is entitled to assert is
decided by each handler's own code, and one of the two handlers decides nothing.

**The receiving end is ready and nothing arrives on it.** `trace_category` *does* have its
`VerificationTrace` arm (`kernel/src/layer/witness_admission.rs:184`, added under eigenius#200), so the emitter half was
completed while the producer half was not. The two halves were fixed in opposite order.

**The ontology asserts the missing path as fact.** `reasoning:VerifiedPropositionView`'s own
description reads:

> *"The witness emitter for `reflection:VerificationTrace` **looks up** the view by
> `source_verified_resource` and **reads** `canonical_proposition` to build the `IsVerifiedAs`
> witness (D49 §7)."*

and the comment above it: *"Created automatically by the `lean_to_reasoning` comorphism"*. Both
describe behaviour that does not exist.

This is a different failure mode from a stale design document, and a worse one. A design document is
dated by construction and read as history. An **ontology is chain-resident** — it is loaded, it is
queryable, its descriptions are what `eigenius get-schema` returns to an agent, and it reads as
current. Intent embedded there is indistinguishable from specification.

### 2.5 What §2 establishes

- The grade is carried twice, by mechanisms with **no common input and no reconciliation** (§2.0).
- The obligation table is **inverted**: the grade implying the most machinery requires the least
  (§2.0).
- `Observation` denotes a discourse kind and an epistemic grade, one file apart (§2.1).
- Two of the four grades are granted by **hard-coded class IRIs** in a kernel match, not by any
  declared relation (§2.3, §2.4).
- The soundness of `Verified` rests on a **four-file conjunction** that nothing records (§2.4).
- `Verified` has **two intended producers and one working one**; the Lean route's middle step was
  never built, and its consumer half was completed anyway (§2.4 route B).
- **AutoOnLoad is a veto with no declared post-condition on success.** Two institutions share the
  role and one promotes on `Holds` while the other does not; nothing in the mechanism says which is
  correct (§2.4 route B).
- An **ontology description asserts behaviour that does not exist** (§2.4 route B). Chain-resident
  artifacts read as current in a way design documents do not.

---

## 3. The boundary map

### 3.1 Kernel ↔ institution — what an institution may assert

**The trait is three operations.** `Institution` (`kernel/src/institution/runtime.rs:126`) exposes
`extract_typed` (resource → `Val`), `reify` (`Val` → resource), and `query` (resource →
`QueryOutcome`). There is no method for stamping a class on an existing resource, minting a trace,
or writing more than its own results.

**The whole write channel is `QueryOutcome`**, which has exactly two resource-bearing fields:

| field | committed when | read by |
|---|---|---|
| `output: Resource` | always (as provenance) | the gate — `Holds` admits the commit, `Fails` rejects it |
| `derivations: Vec<Resource>` | **only on `Holds`**; dropped on `Fails` | nothing at commit; later, the witness emitter |

That is the entire surface. An institution's power over the chain is: *veto the commit*, and *emit N
resources that survive only if it does not veto*.

#### 3.1.1 The channel is untyped, and two unrelated things ride it

`derivations` is documented (`kernel/src/institution/runtime.rs:67`) as carrying *derived results* — statistics emits one
`InstitutionEmittedDerivation` per ANOVA effect, whose `canonical_proposition` grounds an
`IsDerivedAs` witness. The same comment states it is *"Empty for institutions whose only job is the
pass/fail gate (e.g. Reasoning / Lean)"*.

**That is no longer true of Reasoning.** Since eigenius#200 a passing `ValidateJustification` rides
a `reflection:VerificationTrace` on the same field
(`crates/eigenius-reasoning/src/validate.rs:161-166`), precisely *because* the field's
commit-on-Holds semantics are what it wants: *"a `Fails` mints nothing, which is the point."*

So one untyped channel now carries two semantically different things:

| | statistics | reasoning |
|---|---|---|
| what rides | a **result** the institution computed | an **audit record** that a check happened |
| carries `canonical_proposition` | yes — it is the point | no |
| grounds a witness | `IsDerivedAs`, self-attesting | `IsVerifiedAs`, trace-attested |

#### 3.1.2 The kernel stamps every passenger as a derivation

*As of `2026-09-03` it does not — see the note at the end of this section. What follows is the state this analysis found.*

`finalize_emitted_derivation` (`kernel/src/institution/dispatch.rs:517`, since renamed) ran over **every** element
of `derivations` and unconditionally added two classes if absent:

```rust
if !has_class(&classes, wk::DERIVED_RESOURCE)             { classes.push(DERIVED_RESOURCE) }
if !has_class(&classes, wk::INSTITUTION_EMITTED_DERIVATION) { classes.push(INSTITUTION_EMITTED_DERIVATION) }
derivation.set(FROM_SUBJECT, subject_iri)
```

The reasoning institution's `VerificationTrace` therefore lands on the chain carrying
`is_a = [VerificationTrace, DerivedResource, InstitutionEmittedDerivation]` — an audit record
labelled as a computed result. `from_subject` is stamped too, so
`InstitutionEmittedDerivation`'s required property is satisfied and the resource validates.

**No live defect follows, for one contingent reason.** The self-attesting Derived route
(`emit_from_institution_derivation`) reads `canonical_proposition`, and `verification_trace()`
(`crates/eigenius-reasoning/src/validate.rs`) does not set one — so the emitter returns `None` and
no spurious `Derived` witness appears. The trace is spared a second, wrong witness by an omission,
not by a check.

**Fixed `2026-09-03`**, when eigenius#160 made the Lean institution a second producer of traces on
this channel. `finalize_emitted_resource` — the same function, renamed off "derivation" — now asks
`Layer::is_subclass_of(class, prov:Trace)` and withholds the marker from anything under it, so a
`VerificationTrace` lands as `is_a = [VerificationTrace]` alone. Subsumption, not a list of trace
IRIs: a new `prov:Trace` subclass is covered by declaring it. The same branch drops a trace whose
dispatch returned `Undecidable`, which the outer loop lets through (it drops only `Fails`).

The channel still carries both kinds, which §3.1's table says is the seam. What changed is that
the kernel now reads the class instead of assuming one.

#### 3.1.3 There is no declared post-condition on success

§2.4 established that `reasoning:qc_validate_justification` and `lean:qc_proof_check` share the
`auto_on_load` role and diverge on what a pass means — one mints a trace, the other emits nothing.
§3.1 explains why nothing detects that: **the dispatch protocol has a typed channel for *rejecting*
and an untyped one for *everything else*.**

`VerdictReading::{Holds, Fails, Undecidable}` is read by the pipeline and has defined consequences.
What an institution is *entitled to assert* on `Holds` is not modelled anywhere — not on the
`QueryClass` resource, not on the `Institution` trait, not in `QueryOutcome`'s type. It is whatever
each handler happens to put in a `Vec<Resource>`.

That is the shape of the seam: **rejection is a contract; promotion is a convention.**

#### 3.1.4 in_process vs external is thinner than it looks

`institution:runtime` admits `in_process`, `external` and `wasm`. Against the trait, the distinction
reduces to one field: external-runtime institutions populate
`QueryOutcome.partial_invocation` so the kernel can fold a `RuntimeInvocation` into the commit
(D31 §6.3); in-process ones return `None`. Nothing else in the trait differs, and `wasm` has no
implementation at all (eigenius#101 removed it; the ontology still declares it — §1.1's pattern of
a declared-but-unbacked value, here in the institution vocabulary rather than the epistemic one).

### 3.2 Kernel ↔ validator — three semantic relations that live in Rust

The validator enforces this stack through three hard-coded lists. Each encodes a relation the
ontology cannot express, and each is the sole definition of that relation.

| list | where | what it decides |
|---|---|---|
| `PROPOSITION_SLOTS` | `kernel/src/ontology/well_known.rs:545` | which of the 28 `eigentt:Term`-ranged properties must inhabit `Prop`, not merely type-check — **6 of them** |
| `trace_category` | `kernel/src/layer/witness_admission.rs:179` | which trace class grounds which grade — the 5→4 map |
| the self-attesting arms | `kernel/src/layer/witness_admission.rs:74-88` | which classes ground a witness *without* a trace — exactly `reasoning:ReasoningSentence` and `reflection:InstitutionEmittedDerivation` |

**The first is documented as a deliberate compensation for what the range cannot say:**

> *"`eigentt:Term` is the range of every D47-encoded EigenTT tree, and most of those trees are
> legitimately not propositions … The range alone therefore cannot carry the obligation; membership
> here is what distinguishes a slot that asserts something from a slot that merely holds a term."*

That reasoning is sound. What it does not address is why the distinction is a Rust array rather than
a property on the property — the ontology already annotates properties (`core:class_types`,
`core:domain`, `core:data_type` are all property-on-property), so *"this slot asserts"* is
expressible in the vocabulary that exists. Whether it should be is §4's question.

**The consequence is uniform across all three:** adding a proposition slot, a trace kind, or a
self-attesting class is a **kernel edit**, not an ontology edit. An institution or a domain ontology
cannot introduce one. The extensibility the institution mechanism provides stops at this boundary.

### 3.3 Compiler ↔ kernel — the compiler is a grade author

`stamp_declared` (`kernel/src/esl/compile.rs:3638`) appends `reflection:DeclaredResource` to **every
resource compiled from ESL**, at seven call sites covering every declaration form.

This is the largest single producer of epistemic grades in the system, and it is neither an
institution nor a `ClaimGrader` — it is the surface compiler. It is also self-consistent in a way
the other producers are not: because `DeclaredResource` `requires declared_by`, and a stamp without
one would fail `MissingRequired` at commit, the compiler also supplies
`reflection:agent:unattributed` when the source names no declarer
(`kernel/src/esl/compile.rs:3588-3600`).

That care is worth noting precisely because it is local. The compiler satisfies the obligation its
own stamp creates; nothing checks that the other seven producers (§1.6) do the same.

### 3.4 Chain ↔ derived — what is persisted

| artifact | persisted | rebuilt from |
|---|---|---|
| epistemic class (`is_a`) | **yes** — part of the resource | — |
| `reflection:*Trace` resources | **yes** | — |
| institution `Verdict` | **yes** (provenance) | — |
| `InstitutionEmittedDerivation` | **yes**, on `Holds` only | — |
| **`ChainWitness` / `WitnessKey`** | **no** | recomputed per lookup from Trace-class resources and the two self-attesting classes |
| the witness *index* | **no** — despite the file name, nothing is materialised | direct lookup per key (`kernel/src/layer/witness_admission.rs:20-28`) |

The asymmetry that matters: **the evidence is persisted and the entitlement is not.** A witness is a
function of the chain, recomputed on demand.

**That is not the cause of D80 §2's environment-blindness**, though an earlier draft of this section
said it was. The cause is that `WitnessKey` records the category, the IRI and a hash of the
proposition **term** — and nothing about the environment those names resolve in
(`kernel/src/witness/mod.rs`). The same key therefore denotes a different proposition after a
rebinding while hashing identically. Persisting the discharge would not change that: it stores the
same blind key, and recomputing it against the rebound chain still hits the same ancestor by
first-hit-wins and still returns the same answer. Non-persistence and environment-blindness are
independent facts, and only the second is a soundness problem.

What non-persistence *does* cost is that the entitlement has no existence outside the recomputation:
nothing can cite a witness (there is no IRI for the warrant a certificate rests on), nothing can
transport one across a merge or an export, and the admitting layer is unrecoverable — first-hit-wins
answers *whether* some ancestor admits the key, never *which*. D82 P7 proposes persisting discharged
witnesses on those grounds; D82 S1 is the separate fix for the blindness.

---

## 4. Provenance

**Everything in this section is quotation from the design corpus, not description of the system.**
Per the method rule, a claim supported only by a design document is marked as one. §§1–3 stand
without this section; it exists to answer *why* a shape is as it is, and — more usefully — to record
which intentions were **abandoned**, since an abandoned intention usually marks something the plan
had not anticipated.

### 4.1 One encoding *was* canonical, and that was explicitly withdrawn

D39 §8 made the justification term the canonical carrier and demoted the classes to a legacy path:
the categories were to become *"structurally enforced projections from the `JustificationTerm`
shape, rather than separate tags"*.

**D73 §9 withdrew §8 in its entirety**, and replaced unification with deliberate separation:

> *"One distinction to preserve: the four epistemic resource classes are **not** the category-of-a-term.
> `DeclaredResource requires declared_by` is a well-formedness rule about a resource and its trace,
> and it stands unchanged. What is withdrawn is the collapse of a justification term to a scalar."*
> (D73 §1.2)

The class-vs-`WitnessCategory` decoupling that §2.0 records as unreconciled is likewise a **recorded
decision**, not an oversight — D54 §4.2 chose it because making `ReasoningSentence` a
`VerifiedResource` would give it *"trace requirements it shouldn't have"*.

**Rows 2 and 7 of §1.1 are unexplained anywhere in the corpus.** Neither the
`reflection:epistemic:*` individuals nor `Grade`/`ClaimGrader` appears in any of the nine documents.

### 4.2 The corpus specifies the opposite of §2.0's obligation table

D6b §6.2 states `DerivedResource` **requires** `derivation`. The relaxation the code carries has no
counterpart in the corpus, and D73 §3.2 treats the cases that motivate it as an **open defect rather
than a design**: *"Each is a place where the chain plausibly knows something it cannot cite."*

This does not make the code wrong — the implementation is the guiding artifact. It changes the
character of the finding: the empty `requires` list is a **local relaxation that contradicts the
spec**, not an evolved position.

### 4.3 Promotion was specified — as a *kernel* responsibility, and never built

§3.1.3 called the asymmetry "rejection is a contract, promotion is a convention". The corpus shows
that division was deliberate, and that the missing half was assigned elsewhere:

> *"If the Lean server accepts, **the kernel attaches the proof term as the resource's reasoning
> trace and promotes the resource's epistemic status from derived → verified**."* (D14 §7.2)

> *"On `Holds`, **the kernel emits**: 1. A `DerivedResource` … 2. A `ProgramTrace` resource (per D49
> §6) pointing at the `DerivedResource`, so the witness index admits an `IsDerivedAs` entry."*
> (D52 §6)

The AutoOnLoad role owned rejection (D14 §9.1, D31 §6.3); the **kernel** was to own promotion. The
kernel side was never built. The convention that fills the gap is each handler's own code.

### 4.4 Abandoned intentions

The most informative output of this pass. Each explains a seam §§1–3 found.

**Uniformity was the design, and it is what was lost.** D49 §6 specified one emitter for all four
families: *"In all four cases the witness emitter performs the same operation: locate the
`canonical_proposition`-carrying chain resource, read the property, hash the encoded form, populate
the witness index entry."* D49 §6 and D39 §4.2 explicitly intended **no** class-keyed arm for
`ReasoningSentence` — its witness was to come from a `ProgramTrace` *"with no
Reasoning-institution-specific dispatch in the witness emitter"*. That `ProgramTrace` was never
produced (D54 §1), so the uniform emitter became three disjoint routes and two hard-coded class arms.

**This explains §1.3.** The concept the ontology lacks — *trace kinds that ground a witness* — is the
**residue of a uniformity that was supposed to make naming it unnecessary**. Nothing needed to say
"these classes ground witnesses" while all of them did so identically through one property.

**Statistics was designed with the kernel minting the trace** (D52 §6, §8, §9): a `Decidable`
QueryClass whose `Holds` made the *kernel* emit a `DerivedResource` plus a `ProgramTrace`. The built
system runs it AutoOnLoad, has the *institution* put derivations on `QueryOutcome.derivations`, and
mints no `ProgramTrace`. **This is the origin of two seams at once** — the untyped channel (§3.1.1)
and the self-attesting `Derived` arm (§2.3) — both exist because the trace the design routed the
witness through was never produced.

**The witness index was to be materialised**: *"Use `OnceLock<BTreeMap<WitnessKey, ()>>` on the
Layer"* (D49 §3, §6). Nothing is. The filename `kernel/src/layer/witness_admission.rs` is the residue, which is why
§3.4 has to say "despite the name, no index is materialised".

**WASM was the primary intended runtime**, not a speculative third value — D14 §12 specifies
institutions as WASM guest components against a WIT world. `runtimes:wasm` surviving with no
implementation (§3.1.4) is the remains of the main plan, not an unfinished extra.

**`QueryOutcome.derivations` post-dates the corpus.** D14 §8's trait returns a single `Resource`;
D31 §6.3 enumerates one artifact per firing; D52 §10 records the multi-resource channel as *missing*
— *"that fuller commit shape is the natural Phase 5.1 follow-on once the institution API supports
multi-resource output cleanly"*. Nothing in the corpus discusses **typing** it. D54 §4.3 does state
the principle the untyped channel now violates: *"lemma-citability ⇔ proposition-bearing +
kernel-warranted"*, and *"Institution `Verdict`s are not lemmas."*

**The only reconciliation ever proposed was withdrawn rather than replaced.** D39 §8 had
`ValidateJustification` compute the epistemic category as part of admitting the term. D73 §9
withdrew it; D73 §11.5 records it was never implemented. §2.0's "two carriers, no reconciliation" is
the state left behind by that withdrawal.

**`declared_by` was prose.** D6b §4.2's example carries `"declared_by": "Eigenius core team"`.
D73 §3.1 records what changed it: on the WRN chain *"74 of them were the literal `\"esl-compiler\"`,
so every `DeclaredEvidence` leaf bottomed out in a name for the compiler."* Rules 8 and 22 now force
it to resolve to a `reflection:Agent`. **That tightening is what made the compiler a grade author**
(§3.3): `stamp_declared` must synthesise `reflection:agent:unattributed` because its own stamp
requires a resolvable agent.

---

## 5. Findings

Every candidate from §§1–3 was put to an adversarial pass instructed to **default to "not a
defect"** and to concede only where it had looked for a defence and failed. **Two were refuted, six
weakened, two stand.** The refutations are recorded as corrections in §2.0 and §2.1 rather than
buried here.

Classified as the method requires: *redundant* (two names, one denotation) · *ambiguous* (one name,
two denotations) · *unowned* (a decision made in N places with no authority) · *fine*.

### 5.1 Fine — and the hypothesis was wrong about them

| candidate | why it stands as-is |
|---|---|
| `DerivedResource` requires nothing | Deliberate, documented in the chain-resident ontology, pinned by a named test — and every *concrete* derived path carries its own requirement. Requiring `derivation` on the base would force `ReasoningSentence` and `VerificationTrace` to point the slot at themselves |
| `encoding:Observation` vs `reflection:ObservedResource` | No collision exists. Different namespace, different string; orthogonality stated at the point of definition and per member |
| seven encodings, rows 1 · 3 · 5 · 6 | Non-substitutable syntactic categories. Row 5 is a `Prop`-valued inductive, row 6 a constructor **taking row 5 as an argument** — a type and its argument cannot be one artifact. The 5→4 collapse carries a five-line rationale and a test |
| row 2, `reflection:epistemic:*` individuals | Load-bearing where `is_a` is already spent: `lexicon:grade` uses them with `allows_only`, and says why — *"the SAME `reflection:EpistemicStatus` the rest of the stack uses (not a parallel enum)"* |
| two senses of "witness" | Zero mechanical risk. Every `Witness*` error is a `MergeError`; the chain-side senses live under different namespace prefixes. A readability cost only — and the overload is in fact **three-way** (`objective:witness`), the third pre-empting confusion in its own description |

**§1.1's headline was too strong.** Seven encodings is not seven-way redundancy. What survives is
two dead artifacts (§5.3).

### 5.2 Unowned — the two that stand

**`Verified` rests on a conjunction nothing records.** `emit_from_reasoning_sentence` grants the
grade to any `ReasoningSentence` with a hashable proposition; that is sound only because the
AutoOnLoad gate blocks a failing commit. The skeptic found two partial defences — the ontology link
is protected by the manifest pin, and the gate's status is recorded once on an IRI constant
(*"Load-bearing — every committed ReasoningSentence triggers it"*,
`crates/eigenius-reasoning/src/institution.rs:50`) — and neither closes it: the emitter's own doc
never mentions the gate. It also found the conjunction **weaker than §2.4 credited**:
`dispatch_auto_on_load_for_layer` has one call site and **no test**, and the only test of
`Fails → InstitutionValidation` goes through the *per-resource* entry point, not the layer path the
commit actually uses (`kernel/tests/dock_assay_demo.rs:699`).

**Three semantic relations are kernel-only, and one needn't be.** `PROPOSITION_SLOTS` is well
argued and commit-enforced. The self-attesting arms have a *plausible* soundness reason — an
ontology that could declare its own class self-attesting could mint `Verified` at will — but the
skeptic searched `kernel/src/layer/witness_admission.rs`, `kernel/src/witness/mod.rs`, `kernel/src/ontology/well_known.rs` and D49 and **found that
argument stated nowhere**. Against `trace_category` the candidate got *stronger*: the ontology
already declares `reflection:epistemic_status` with `allows_only` over exactly the four grade
individuals, and already attaches it to `ProgramTrace`'s `recommends` as *"Epistemic status of the
traced output"* — **the vocabulary for "this trace grounds that grade" exists, is chain-resident,
and no Rust file reads it.** `grep epistemic_status` over `*.rs`: zero hits.

### 5.3 Dead — three artifacts with no consumer

Found by the adversarial pass while looking for defences, and the clearest cleanup targets.

| artifact | state |
|---|---|
| ~~`chain_witness_category_for_iri` (`kernel/src/ontology/well_known.rs:578`)~~ | **resolved at P7.** The hook now calls it; the short-name duplicate is deleted |
| `Grade` / `GradedClaim.grade` (`crates/eigenius-reasoning/src/grade.rs`) | **write-only** — set at four sites, read at exactly one, in a test. "Not two enums needing reconciliation; one live enum and one dead field. The remedy is deletion, not a `From` impl" |
| `runtimes:wasm` | declared, no implementation (§3.1.4) |
| `default_asserts_proposition` and `default_asserts_proposition_hash` | **public API with zero consumers** — re-exported from `kernel/src/layer/mod.rs:48` and called from nowhere in the tree, tests included. The `Asserts(iri)` fallback they expose is reached only through the module-private path inside `emit_from_trace` |

**The module's whole production surface is two entry points.** `synthesize_chain_witness` is called
from exactly one place — `kernel/src/program/check_hooks.rs:86`, implementing
`EffectHooks::synthesize_chain_witness` — and `is_witness_candidate` from exactly one —
`kernel/src/storage/memory.rs:242`, computing the `has_witness_candidates` bit that lets a lexicon
layer be skipped without probing. `layer_admits_witness` and `lookup_chain_witness` are re-exported
and used **only from test files** (`crates/eigenius-reasoning/tests/`,
`crates/eigenius-statistics/tests/`).

`Grade`'s defence is worth keeping even though the field is dead: its doc calls it *"a structural
projection of the `JustificationTerm` constructor — not a stored field"*, i.e. a **pre-commit
construction-time** label against `WitnessCategory`'s **post-commit chain-derived** key. Different
inputs, different times; a conversion between them would be a category error.

### 5.4 Stale — assertions the code has falsified

| where | says | actual |
|---|---|---|
| `kernel/src/institution/runtime.rs:69` | derivations are *"Empty for institutions whose only job is the pass/fail gate (e.g. Reasoning / Lean)"* | falsified by `crates/eigenius-reasoning/src/validate.rs:163` since eigenius#200 |
| `reasoning:VerifiedPropositionView`'s description | the emitter *"looks up the view … and reads `canonical_proposition`"* | no such lookup exists (§2.4 route B) |
| `kernel/src/layer/witness_admission.rs` — the filename | an index | nothing is materialised (§3.4, §4.4) |

The second is the serious one, for the reason §2.4 gives: an ontology is chain-resident and reads as
current in a way a code comment does not.

### 5.5 Untested — where a claim rests on nothing executable

- ~~The **committed shape** of a stamped `VerificationTrace` is untested.~~ **Pinned
  `2026-09-03`.** `notebook_fixture_test::a_holds_verdict_admits_a_verified_witness` reads the
  trace out of the committed provenance layer, so it sees the resource exactly as it lands. The
  observation that a stamped trace carries `InstitutionEmittedDerivation` is what prompted the
  fix rather than the test: `finalize_emitted_resource` now withholds that marker from anything
  under `prov:Trace`, because the class asserts "grounds nothing" and a trace is a ground.
- `dispatch_auto_on_load_for_layer` — the commit's actual path — has **no test** (§5.2).
- ~~`kernel/src/program/check_hooks.rs:93` dispatches witness synthesis on an inductive's **short
  name**, so any inductive anywhere named `IsVerifiedAs` enters the path.~~ **Closed at P7.** The
  hook keys on `decl.iri` through the `chain_witness_category_for_iri` that was already written
  and unused, and `synthesis_hook_ignores_a_foreign_inductive_carrying_a_witness_short_name`
  pins it — an inductive named `IsVerifiedAs` under a foreign IRI no longer enters the path.

### 5.6 What the analysis concludes

The hypothesis was *unclear boundaries, overlapping abstractions, unclear responsibilities*. Against
the code:

**Overlap: mostly refuted.** The seven encodings are seven syntactic categories doing seven jobs, and
the pass defended five of them from the code. What is genuinely duplicated is small and dead (§5.3).

**Unclear boundaries: one, and it is real.** *Rejection is a contract; promotion is a convention*
(§3.1.3) — with §4.3 showing this is the corpus's own division, the kernel half simply never built.
Everything downstream of it (the untyped channel, the self-attesting arms, the missing Lean route)
descends from that one gap.

**Unclear responsibility: one, and it is narrower than expected.** Not *"who assigns a grade"* —
eight writers with one trait sounds worse than it is, since no reader grants anything on a class
(§5.1). It is *"who may say that a trace kind grounds a grade"*: the answer is the kernel, in three
Rust lists, while the ontology already has the vocabulary and no code reads it (§5.2).

**The most useful single sentence in the analysis is §4.4's**: the concept the ontology lacks is the
residue of a uniformity that was supposed to make naming it unnecessary. Every kernel-only list in
§5.2 is a shard of one emitter that was specified to be uniform and never was, because the
`ProgramTrace` it routed through was never produced.

A cleanup proposal is out of scope here, as §1 of the plan states. What this section hands a
successor is: three dead artifacts to delete, three stale assertions to correct, three untested
claims to pin, and **one design question** — whether AutoOnLoad should have a declared
post-condition on success, which is the root the other findings hang from.


---

## 6. The encoding ontology — where the modelling was done carefully

`ontologies/encoding/encoding.esl` (550 lines) is the chain vocabulary of the prose→propositions
pipeline, and therefore **the largest producer into this stack**: one `EncodedClaim` per parsed
sentence. It is also the one place in the tree that reasons explicitly about the distinctions
§§1–5 find missing elsewhere — which makes it both a counter-example to the hypothesis and the
sharpest evidence for §5.2.

### 6.1 It splits its output into two objects with opposite grades

| class | grade | why |
|---|---|---|
| `enc:ReasoningStructure` | `reflection:DerivedResource` | *"Applying the parse engine to bytes is a program run: a function of (engine, source_sha256) → structure. That is Derived in the plain sense, witnessed by ONE `reflection:ProgramTrace` for the run."* |
| `enc:EncodedClaim` | `reflection:DeclaredResource` | *"The parser chooses the FORM; it does not assert the content."* |

`ReasoningStructure` **requires `reflection:derivation`** — so unlike the base `DerivedResource`
(§2.0), this concrete derived class does carry an evidential obligation, and the validator enforces
it.

### 6.2 It states the distinction the vocabulary cannot

> *"Three propositions stay distinct and must not collapse into one witness:*
> *1. this text parses to this well-typed term — the artifact fact, witnessed by the `ProgramTrace`
> on the ENCODING artifact, bounded by the program's type*
> *2. this encoding is faithful to what the author wrote — D61, unwarranted today*
> *3. what the author wrote is true — never established by either; only ever declared"*

The parser is *"a FORMULATION INSTRUMENT: it produces a well-formed proposition and a fidelity
record. It does not produce a warrant … Since the parse cannot warrant fidelity, the only honest
status for its output is Declared: some agent takes responsibility."*

**This is three-way, and the stack's vocabulary is one-way.** A `WitnessKey` is
`(category, iri, proposition hash)` — it can say *this resource is Derived with respect to that
proposition*, and cannot say *with respect to which of three propositions about it*. The
distinction survives here as a **prose comment**, because there is nowhere else to put it.

### 6.3 It records having had the bug this analysis hunts — and fixed it

> *"Until 2026-08-22 the artifact had this exactly inverted at the level that mattered: no trace on
> the structure at all, and N `ProgramTrace`s on the CLAIMS — one per sentence — each minting
> `IsDerivedAs(claim, P)` where P was a proposition about the world. **Wrong on both counts.** Wrong
> CARDINALITY, because one parse run is one program execution, not one per sentence; and wrong
> PROPOSITION, because what the run establishes is that this structure came out of this engine over
> these bytes, never that any claim in it is true."*

A producer minting witnesses at the wrong cardinality, whose proposition was about the wrong thing,
and nothing in the stack detected it — the witnesses were well-formed. It was caught by a person
reasoning about the ontology, and the fix was to re-shape what the producer emits.

That is the concrete cost §5.1 said the seven encodings had not yet been shown to have. It is not a
cost of the *encodings*; it is a cost of the **proposition slot being unconstrained**. Nothing
anywhere states which proposition a given trace kind is entitled to witness.

### 6.4 It practises "reuse, don't mint" — and shows where reuse runs out

The file opens with the rule (`ontologies/encoding/encoding.esl:15`): *"REUSE, DON'T MINT. Grades
reuse `reflection:EpistemicStatus`; the encoded proposition reuses
`reflection:canonical_proposition`."* It follows it — and the two-axis claim model (§2.1) is the
result of following it into a place the epistemic vocabulary does not reach: the discourse kind
needed its own axis because `is_a` was already carrying the grade.

**The pattern across §6 is one thing.** Every distinction this ontology needed and could express, it
expressed in the shared vocabulary. Every distinction it needed and could *not* express — which
proposition a trace witnesses, why a parse warrants form but not content — it wrote in comments.
§5.2's finding is the same observation from the other end: the vocabulary for *"this trace kind
grounds that grade"* is absent, and here is the ontology that most needed it, compensating in prose.
