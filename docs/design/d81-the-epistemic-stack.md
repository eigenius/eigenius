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
| 2 | four lifecycles | P1 | pending |
| 3 | the boundary map | P2 | pending |
| 4 | findings | P3 / P4 | pending |

---

## 1. The concept inventory

### 1.1 The four-way distinction is encoded seven times

Every row below is a distinct artifact naming *declared / observed / derived / verified*. They are
not synonyms — each occupies a different syntactic category, listed in the third column — but
nothing in the system states the mapping between them in one place.

| # | encoding | kind | defined at |
|---|---|---|---|
| 1 | `reflection:{Declared,Observed,Derived,Verified}Resource` | **class** — `is_a` membership | `ontologies/reflection/reflection-ontology.json` |
| 2 | `reflection:epistemic:{declared,observed,derived,verified}` | **individuals** of `reflection:EpistemicStatus`, held by the `reflection:epistemic_status` property | same; instances pinned by `kernel/src/bootstrap/mod.rs:1280` |
| 3 | `reflection:{Declaration,Observation,Program,Verification,ExternalExecution}Trace` | **event kind** — what happened | same; **five** classes |
| 4 | `WitnessCategory::{Declared,Observed,Derived,Verified}` | **kernel enum**, a `WitnessKey` component | `kernel/src/witness/mod.rs:47` |
| 5 | `witness:Is{Declared,Observed,Derived,Verified}As` | **inductive predicate** — the proposition | `ontologies/reasoning/reasoning.esl` |
| 6 | `JustifiedBy.{declared,observed,derived,verified}` | **constructor** — the certificate | same |
| 7 | `Grade::{Declared,Observed,Derived,Verified}` | **crate enum** | `crates/eigenius-reasoning/src/grade.rs:69` |

Two observations, both mechanical:

**The 5→4 collapse in row 3 lives in a Rust `match`, not in the ontology.** `trace_category`
(`kernel/src/layer/witness_index.rs:179`) maps the five trace classes onto four categories, sending
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

`is_witness_candidate` (`kernel/src/layer/witness_index.rs:156`) and `trace_category` (`:179`) are
where the concept is defined. Both are Rust.

### 1.4 "Witness" names two unrelated things — **settled**

| | `ChainWitness` | merge `Witness` |
|---|---|---|
| what it is | evidence inhabiting a `JustifiedBy.*` argument | a `MergeComorphism` realising the **universal arrow** at a conflicting IRI |
| shape | a `WitnessKey` — `(category, iri, proposition hash)` | a function `(A, A, Option<A>) → A` |
| where | `kernel/src/witness/mod.rs`, `kernel/src/layer/witness_index.rs`, `kernel/src/nbe/check/witness.rs` | `kernel/src/layer/merge/witnessed.rs` (D20 §6.1) |
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

**Kernel — `kernel/src/layer/witness_index.rs`** *(despite the name, no index is materialised)*

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

**Reasoning ontology — `ontologies/reasoning/reasoning.esl`**

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

## §§2–4

Pending. See the plan.
