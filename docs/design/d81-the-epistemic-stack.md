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
| 3 | the boundary map | P2 | **partial** — seam 1 done |
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

## 2. Four lifecycles

Each traced against an artifact that exists in the tree. The question is the same each time: from
*something happened* to *a certificate can cite it*, what runs?

### 2.0 Two carriers, computed from disjoint inputs

Every lifecycle has **two independent carriers** of its grade, and nothing reconciles them.

| carrier | how it is established | who reads it |
|---|---|---|
| **class membership** — `is_a reflection:*Resource` | asserted by the author, or inherited from a class that subclasses it | the validator's `requires` / `recommends` |
| **witness admission** — a `WitnessKey` | computed by `layer_admits_witness` from traces and two hard-coded self-attesting classes | the type checker, when a `JustifiedBy.*` argument needs inhabiting |

`layer_admits_witness` (`kernel/src/layer/witness_index.rs:66`) **never consults the epistemic
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
no required property. That `derivation` is recommended rather than required is deliberate and
documented at `kernel/src/validation/mod.rs:1518`: substrate-produced resources from `FIBER … INTO`
and post-translation comorphism reify outputs "are derived by construction but may not have a
kernel-generated `ProgramTrace` yet". The consequence stands regardless of the reason: nothing at
commit distinguishes those from a resource that simply claims the grade.

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
2. **Two axes on purpose.** `encoding:Observation` here is a **discourse kind** (`enc:Observation :
   enc:Claim`), not the epistemic `reflection:ObservedResource`. The ontology states the convention
   at `ontologies/encoding/encoding.esl:377`: *"A landed claim carries BOTH axes as classes: `is_a = [enc:EncodedClaim,
   enc:<Kind>]`"*. The word *Observation* therefore denotes two unrelated things one file apart.
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
   (`kernel/src/layer/witness_index.rs:272`) reads `canonical_proposition` off the derivation and
   keys `Derived` against the derivation's **own** IRI. No trace is consulted.
3. **Silent on absence.** `canonical_proposition` is only *recommended*. A derivation without it
   returns `None` — no witness, no error. The doc comment names two causes: "kernel merge dropped
   it, or the institution didn't supply one".

### 2.4 Verified — two intended producers, one of which does not exist

`Verified` is the only grade with more than one route, and the routes are in very different states.

#### Route A — the reasoning institution (works)

1. **Witness, self-attesting.** `emit_from_reasoning_sentence`
   (`kernel/src/layer/witness_index.rs:255`) grants `WitnessCategory::Verified` to **any**
   `ReasoningSentence` whose `reasoning:proposition` hashes. It performs **no check that the
   sentence's certificate validated.**
2. **The guard is elsewhere.** `qc_validate_justification` is declared AutoOnLoad
   (`ontologies/reasoning/reasoning.esl`), so it fires on every `ReasoningSentence` commit; a
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
| 3. the witness emitter looks the view up by `source_verified_resource` and reads its `canonical_proposition` | D49 §7 | **does not exist** — `target_proposition_hash` (`kernel/src/layer/witness_index.rs:317`) has three slots: `canonical_proposition`, `reasoning:proposition`, and the `Asserts(iri)` default. The view is not among them |

Nothing Lean-side stamps `reflection:VerifiedResource`, mints a `VerificationTrace`, or reifies a
view — `grep` over `crates/eigenius-lean/` for all three returns nothing. A `Holds` verdict is a
leaf resource. This is eigenius#160, open.

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
`VerificationTrace` arm (`kernel/src/layer/witness_index.rs:184`, added under eigenius#200), so the emitter half was
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

`finalize_emitted_derivation` (`kernel/src/institution/dispatch.rs:517`) runs over **every** element
of `derivations` and unconditionally adds two classes if absent:

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

---

## §4

Pending. Seams 2–4 of §3 also pending: kernel ↔ validator, compiler ↔ kernel, chain ↔ derived.
