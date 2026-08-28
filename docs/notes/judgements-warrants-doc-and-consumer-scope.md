# Scope — documents, institutions, and demos under the judgements/warrants plan

**Status: scope analysis.** Written `2026-08-28`. Companion to
[`judgements-warrants-build-plan.md`](judgements-warrants-build-plan.md), which sizes the kernel and
ontology work. This note sizes everything the plan's phases invalidate outside those two: design
documents, the ACP specification, user guides, agent skills, the website, the three institutions, and
the demos and fixtures. Counts taken the same day against the working tree.

---

## 0. What makes a document affected

Five claims are load-bearing in the current corpus. Four of them the plan falsifies; the fifth is a
substitution.

| # | claim as currently written | what removes it |
|---|---|---|
| 1 | a resource carries an epistemic grade, declared by `is_a` on one of four classes | P5 deletes the classes and `reflection:epistemic_status` |
| 2 | the four grades form a progression of increasing strength, with `Verified` a specialization of `Derived` | P5 breaks the subclass; the paper rejects the lattice |
| 3 | a trace admits a witness — `ProgramTrace → IsDerivedAs` is how a computation becomes citable | P4 deletes `IsDerivedAs` and `DerivedEvidence` |
| 4 | an institution's `Verdict::Holds` promotes a resource's grade | the plan's disposition section removes the licensing role |
| 5 | `eigentt:TypeExpr` names the type-level fragment | P1 renames it to `eigentt:Term` |

Claims 1–4 require rewriting; each is a sentence about what the system *means*. Claim 5 is
`sed`-scale: **538 occurrences in 76 files** tree-wide, of which 36 files are under `docs/`.

**Corpus totals.** 111 of 242 files under `docs/` mention affected vocabulary: **56 of 90** design
documents, **23 of 80** guide chapters, **27 of 65** notes, **2 of 2** spec files, **3 of 4** agent
skills.

---

## 1. Design documents

### 1.1 Retire — record only

Four documents already carry supersession headers or describe a state the plan ends. None needs
rewriting; each needs a status line pointing at the paper, and D39 needs one pointing two hops.

| doc | lines | hits | current status | action |
|---|---|---|---|---|
| [D39](../design/d39-justification-logic.md) justification logic | 441 | 121 | superseded `2026-08-21` by D73 | second supersession; the term algebra it records survives with **6 constructors, not 7** |
| [D73](../design/d73-justification-logic-witnesses-and-traces.md) proof polynomials, witnesses, the epistemic lattice | 462 | 49 | supersedes D39 | superseded by the paper; its §1 — *the term is retained whole and every category is a query over it* — survives verbatim, the lattice in its title does not |
| [D82](../design/d82-propositions-witnesses-and-logics.md) propositions, witnesses, logics | 883 | 83 | already "superseded as a design, retained as the derivation record" | add the plan as the successor; no content change |
| [D81](../design/d81-the-epistemic-stack.md) the epistemic stack | 907 | 86 | in progress | on P7 it becomes the measured description of the *previous* architecture; keep as the baseline the plan closes against |

### 1.2 Rewrite — the thesis survives, the mechanism does not

Ordered by how much of the document the change reaches.

| doc | hits | what breaks |
|---|---|---|
| [D56](../design/d56-component-execution-and-derivation-materialization.md) component execution | 11 | **the most fully invalidated document outside the epistemic core.** Its central claim is that a component execution becomes a chain-resident witness-bearing derivation *through* `ProgramTrace → IsDerivedAs`. P4 deletes the witness; P5 makes `ProgramTrace` a `prov:Activity` grounding nothing. §1's "different epistemic kinds" distinction between recomputation and execution is the distinction §4.1 of the paper draws on the plan, not the actor — the same correction `ExternalExecutionTrace` gets |
| [D49](../design/d49-chainwitness-machinery.md) ChainWitness machinery | 44 | the admission algorithm survives; §4's `IsVerifiedAs → IsDerivedAs` coercion is deleted (P4), the four-category enum becomes three, §7's Lean route is replaced by P3's judgement, and P7 moves the witness types into kernel base vocabulary — reversing the memo's table-location decision |
| [D54](../design/d54-reasoning-lemma-citation.md) reasoning lemma citation | 36 | the capability (a sentence cited as a lemma) survives; its mechanism does not — it rests on `ReasoningSentence : DerivedResource` and `DerivedEvidence(prior_iri)`, both deleted |
| [D6b](../design/d6b-reasoning-trace-schema.md) reasoning trace schema | 34 | "epistemic status computation" leaves its **Resolves** line. 5 of 19 trace classes change; the other 14 are untouched, which the rewrite must say explicitly |
| [D28](../design/d28-lean-4-as-institution.md) Lean 4 as institution | 16 | five sites assert that `Verdict::Holds` promotes the resource's status to *verified* (lines 4, 61, 91, 277, 526). Under P3 `Verified` is reachable only through a checked judgement, so the Lean institution must supply one; §5.7's closed audit chain is re-drawn |
| [D72](../design/d72-declaration-provenance.md) declaration provenance: agent and warrant | 18 | its `agent`/`warrant` split is the name collision P5 resolves. `Warrant`'s distinction is provenance; the document's §0 decision table keeps its content under changed names |
| [D52](../design/d52-measurement-statistics-institution.md) statistics institution | 24 | its output contract — `InstitutionEmittedDerivation` + `canonical_proposition` → `IsDerivedAs` — becomes P4's composite `App(Declared(plan), Observed(inputs))` |
| [D80](../design/d80-witness-and-institution-machinery.md) witness and institution machinery | 5 | §3's verdict question is retired rather than answered by the plan's disposition section; §2's witness-credit analysis survives |
| [D14](../design/d14-institution-realisation.md) institution realisation | 2 | line 305 — *"the institutions provide the reasoning that promotes resources between them"* — and line 334's derived→verified promotion are claim 4. [D10](../design/d10-grothendieck-institution-protocol.md) is a redirect into D14, so both move together. P7's operational protocol statement lands here |
| [D58](../design/d58-objective-framing-and-obligation-graphs.md) objective framing | 12 | §H2 records reusing `reflection:EpistemicStatus` for `objective:acceptance_grade`. See §6.1 below — this is a gap in the plan, not only a doc edit |
| [D53](../design/d53-large-data-tracking.md) large-data tracking | 8 | `PinnedExternalFile : ObservedResource` is on the retype list; §5's *"small `DerivedResource` carrying an `IsDerivedAs` witness"* is the deleted mechanism |
| [D66](../design/d66-definitional-lifting-and-witness-normalization.md) definitional lifting | 10 | its measurement — 61 `Declared` bridges for 62 sentences — is stated in grade-class terms, and `build_shape_rule` writes `DeclaredResource`. The measurement stands; its vocabulary changes |
| [D51](../design/d51-benchmark-implementation-gaps.md) benchmark implementation gaps | 35 | a status ledger against D39/D49 surfaces that stop existing; gap 2 (the Lean → Reasoning comorphism) is re-scoped by P3 |

### 1.3 The narrative documents

Three documents state the four-category framing as the platform's thesis, in prose written for
readers outside the project.

| doc | site |
|---|---|
| [architecture-v0.3.md](../design/architecture-v0.3.md), labelled *"the authoritative reference for all design decisions"* | §1: *"The progression from declared → observed → derived → verified maps to increasing epistemic strength… Resources declare their epistemic status via base classes"* — claims 1 and 2 in one sentence |
| [vision.md](../design/vision.md) | *"tracked provenance and queryable epistemic status"* |
| [manifesto.md](../design/manifesto.md) | *"Eigenius maintains four epistemic categories as a first-class architectural concern"* |
| [guides/README.md](../guides/README.md) | the same four bullets, as the platform's opening |

**The claim these documents actually want survives the change and is strengthened by it.** *We will
never pretend that probable means certain* does not depend on a stored grade; under P5 it depends on
a warrant computed from relations, which is the stronger form. The rewrite is a re-derivation of the
narrative from the new mechanism, not a retraction.

### 1.4 Rename only

`eigentt:TypeExpr` → `eigentt:Term`, no other change: D47 (6), D61 (5), D37 (4), D48 (3), D63 (3),
D74 (2), D75 (2), D79 (4), plus `notes/p2-n4-eigentt-representation-layer.md` (**33**, the largest
single doc site).

### 1.5 Untouched

The EigenQL guide (13 chapters), the formula guide (9 chapters), and the five Julia institution
tutorials carry **zero** occurrences. The Julia institutions coordinate over `formulas:FormulaTerm`
and declared comorphisms, not over the epistemic vocabulary; nothing in the plan reaches them.

### 1.6 The index

`docs/design/README.md` stops at D66. D70–D82 are unindexed, including every document this note
retires or rewrites. The index is rebuilt as part of the retirement pass.

---

## 2. The ACP specification — separate, because it is normative and external

[`docs/spec/ai-computed-provenance-1.0.md`](../spec/ai-computed-provenance-1.0.md), 1563 lines, dated
12 August 2026, is an editor's draft written as input to a proposed W3C community group. It carries
**128 numbered normative assertions**. **53 of them sit in the sections the plan rewrites** — §5
(epistemic grades), §6 (traces and witnesses), §7 (the justification calculus), and Appendices A.7
and A.8, the normative Eigon/EigenTT binding.

Four assertions are contradicted outright:

| assertion | text | removed by |
|---|---|---|
| `ACP-5-1` | an implementation MUST support all four grades, and MUST NOT use a grade vocabulary of its own | P5 |
| `ACP-5-2` | `Verified` MUST be a specialization of `Derived` | P5 breaks the subclass |
| `ACP-A-22` | the four grades MUST be realised as `reflection:{Declared,Observed,Derived,Verified}Resource` | P5 |
| `ACP-A-31` | the certificate relation MUST be `reasoning:JustifiedBy` with the four grounding constructors listed | P4 removes `derived` |

**Two assertions already state the plan's position.** `ACP-5-7` — *a grade recorded on a claim MUST
NOT be treated as evidence* — and `ACP-5-3` are the computed-not-asserted rule P5 implements. The
spec's §5.3 reached the conclusion before the design did; what it kept was the stored grade as a
redundant, checkable label, and `ACP-5-8` requires the stored and computed grades to agree.

**Decision the rewrite forces: version or revise.** The draft binds conformance to class IRIs that
cease to exist. Editing in place silently changes what a conformance claim means. A `1.1` that
supersedes `1.0`, with `1.0` retained, matches how the design corpus already handles supersession and
is the only option that leaves an external reader able to tell which document a claim was made
against. The explainer (316 lines, 5 hits) follows the spec.

---

## 3. User guides

23 of 80 chapters. Three are structural rewrites, not edits.

| chapter | lines | affected | scope |
|---|---|---|---|
| [esl/09-institutions.md](../guides/esl/09-institutions.md) | 426 | 45 | **§9.10 is lines 256–426 — 171 lines, 40% of the chapter.** It teaches the seven `JustificationTerm` constructors with a per-constructor epistemic gloss, the four `JustifiedBy` grounding constructors, and the `Verified → Derived` coercion, each of which changes |
| [platform/reasoning-institution/README.md](../guides/platform/reasoning-institution/README.md) | 282 | 44 | the whole document is the closed-audit-chain walkthrough: verdict → certificate → witnesses → traces → artifacts. Every hop but the first changes. Its table of "what admits a witness" is five rows of deleted vocabulary |
| [composition/07-stats-and-reasoning-walkthrough.md](../guides/composition/07-stats-and-reasoning-walkthrough.md) | 313 | 36 | the eight-step end-to-end fixture walkthrough. Steps 3–5 are `DerivedResource` + `ProgramTrace` + `canonical_proposition` → `IsDerivedAs` → `App(DeclaredEvidence, DerivedEvidence)`. line 19's stated reason the composition works — *"both institutions honour the same chain artifact shape"* — names the shape that is deleted |

Edits rather than rewrites: `esl/06` (11 — §6.4a witness predicates), `esl/11-appendix` (19, mostly
`TypeExpr`), `platform/statistics-institution/README.md` (18), `composition/02` (14, `TypeExpr`),
`esl/04` (7), `esl/05` (6), `composition/04` (5), `esl/07` (4), `composition/01` (4),
`platform/README` (4), `platform/lean-institution/README.md` (2), and six single-hit files.

---

## 4. Agent skills — `docs/method/`

Three of the four files under `docs/method/` are Claude skills with `name`/`description`/TRIGGER
frontmatter, not prose documentation. **They instruct an agent to author chains in the deleted
shape**, so leaving them stale reproduces the removed vocabulary on every new chain.

| skill | site |
|---|---|
| [`reasoning.md`](../method/reasoning.md) | §"The epistemic contract — grade every claim" is a four-row table mapping each grade to the class, the trace, and the witness it admits. §"Execute with evidence" instructs `reasoning:justification = DerivedEvidence(...)`. Both are P4/P5 deletions |
| [`eigenius.md`](../method/eigenius.md) | its `description` — the string that decides when the skill fires — names *"epistemic statuses, witnesses, justification certificates"*. §"Reasoning & provenance surface" lists the classes and the four `Is*As` families |
| [`grounding.md`](../method/grounding.md) | two sites stamping imported nodes `is_a [..., DeclaredResource]` |

These rank ahead of the guides in urgency: a stale guide misinforms a reader, a stale skill writes
data.

---

## 5. Website

`website/src/content/docs/` is a **hand-maintained copy**, not a generated one — the guide chapters
differ from `docs/guides/` by frontmatter, absolute GitHub links, and, in places, content that has
drifted (`composition/03`'s comorphism diagram differs in substance from the guide's). **28 files
carry affected vocabulary**, of which 4 have no counterpart under `docs/`:

- `concepts/justification-logic.md` — a standalone concept page on the D39 surface
- `concepts/institutions.mdx`, `concepts/domain-bridges.mdx`
- `examples/drug-screening.mdx` — the `App(DeclaredEvidence, DerivedEvidence)` composition as the
  public example
- `research/papers/typed-knowledge-graph-dbms.md`

The copy relationship means every guide edit is made twice, and the four originals are edited once.

---

## 6. Institutions

Three institutions register in process (`crates/eigenius-{reasoning,statistics,lean}`); the Julia and
R hosts reach the kernel through `ExternalInstitution` over the D26 substrate.

### 6.1 Statistics — the largest institution change

**P4 rewrites its output contract.** Today `validate.rs` emits, per effect, an
`InstitutionEmittedDerivation` carrying a `canonical_proposition`, and the witness emitter reads that
resource to admit `IsDerivedAs` on its own IRI — the self-attesting path. P4 deletes both
`emit_from_institution_derivation` and `IsDerivedAs`; the institution must instead emit the composite
`App(Declared(plan), Observed(sample_set))`.

The emission is per-dispatch-position (single-sample, two-sample, correlation, classification), so
the change is not one call site. Surface: 3 source files (`validate.rs`, `institution.rs`, `lib.rs`), **8 test files**, 5 fixtures. The plan's
P4 exit gate — `leaves_of(term, Observed)` returns the sample set, `survives_without(dataset)`
returns false — is a statement about *this* institution's output, and cannot be met without it.

### 6.2 Lean — P3 changes what a passing proof establishes

The Lean institution runs the D28 §5.5 three-part correspondence check and emits `Verdict::Holds`.
`Verified` then arrives by the D49 §7 route: a `VerifiedPropositionView` plus a
`reflection:VerificationTrace`, which `trace_category` maps to `WitnessCategory::Verified`.

**Under P3 the verdict grants nothing and `Verified` is reachable only through a checked judgement**,
so the Lean institution must supply a judgement whose `proof_term` names a term checked at `t : P`.
That is the same correction P3 makes to the reasoning institution's `verification_trace`, applied to
the one institution whose proof term is a real proof rather than a self-reference. D51 gap 2 records
the Lean → Reasoning comorphism as partial and deferred; P3 changes what remains to be built.

Surface: `crates/eigenius-lean` 5 files (1 source, 3 tests, 1 example generator), plus the
`lean-verification` notebook and its fixture.

### 6.3 Reasoning — P3 and P7

P3 rewrites `validate.rs`'s `verification_trace` and `emit_from_reasoning_sentence`; P4 removes a
`Ground` variant from `project.rs`; P5 retires `grade.rs` entirely (607 lines, 46 matching lines — the
largest single-file concentration in the tree); P7 absorbs `ValidateJustification` into uniform validation,
which removes the institution's AutoOnLoad dispatch entirely.

Surface: 8 source files, **15 test files**, 4 fixtures.

### 6.4 Encoding, ingest, and the two generated ontologies

- `crates/eigenius-encoding/src/emit.rs` stamps `enc:EncodedClaim` as a `DeclaredResource` and
  requires the `DerivedResource` trace pointer on `enc:ReasoningStructure`. Both classes are on the
  retype list.
- **`crates/eigenius-schemaorg/src/convert.rs` and `crates/eigenius-obograph/src/convert.rs` stamp
  `DeclaredResource` on every imported node.** This is why `ontologies/schema-org/schema-org.eigon.json`
  holds **2114 occurrences** — more than the rest of the tree combined. It is a regeneration, not
  2114 edits, but the two converters must change before the regeneration and both have a golden test.

### 6.5 The R runtime stamps a grade from inside the R script

`crates/eigenius-r`'s marshalling convention has the R script build its own output resource:
`.Call("r_eigon_add_class", b, "urn:eigenius:reflection:DerivedResource")`. The grade IRI is a string
literal inside R source embedded in Rust (`conventions.rs:33`, `runtime.rs:451`, two test files), so
no Rust type change catches it — a grep over R script strings is the only way it surfaces. The same
convention is what the WRN wrapped-R family (lme4, limma, fgsea, emmeans) emits, which is why the
D56 rewrite and this crate move together.

### 6.6 Unaffected

The five Julia institutions and the WASM examples carry no epistemic vocabulary beyond `TypeExpr`.

---

## 7. Demos, notebooks, fixtures, experiments

| artifact | occurrences | disposition |
|---|---|---|
| `experiments/publications/wrn-helicase` (29 files) | **327** | the plan's end-to-end verification gate. Its chain files 03–08 are authored `DerivedResource` + `ProgramTrace` + `ReasoningSentence` triples; `04-phase1-recompute-conclusions.esl` alone has 56 |
| `notebooks/examples/stats-and-reasoning.json` | 41 | the composition demo the guide chapter walks. P5's exit gate calls it the one consumer filtering on a grade class; **all 41 occurrences are in ESL authoring cells and markdown prose.** Its three EigenQL queries pivot on `Verdict` + `verdict_subject` + `ctor_name` and touch no proposition, justification, or grade — see §8.7 |
| `demo/prose-to-formulas` + `-v2` | 67 | `inference.esl` and `literature-rules.esl` author justification terms; the README documents them |
| `experiments/objectives` (13 files) | 87 | `acceptance_grade` / `axiom_kind` consumers — see §8.1 |
| `experiments/objectives/d57-schema-org` (12 files) | 85 | regenerated with the schema.org importer |
| `experiments/benchmark` (7 files) | 63 | `bench:TaskOutput` / `bench:Deliverable` — see §8.2 |
| `notebooks/examples/lean-verification*` (4 files) | 11 | the audit chain P3 re-draws |
| `demo/wrn-helicase`, `demo/d57-schema-org` run scripts | 9 | driver scripts, mechanical |
| test fixtures under `crates/*/tests/fixtures/` | 9 files | authored ESL chains in the deleted shape |
| `demo/{catalyst,diffeq,intervals,jump-highs,symbolics,patent,d41-commit-pipeline}`, `notebooks/examples/{kinase-institutions,patent-analysis,d36-merge}` | 0 | untouched |

**Rust test churn**: 37 test files under `kernel/` and `crates/` reference affected vocabulary — 15
reasoning, 8 statistics, 6 kernel, 3 Lean, 5 elsewhere — against 56 source files.

---

## 8. What the build plan's inventory does not cover

Seven findings. Each is a decision the plan has to make, not a document to edit.

### 8.1 `reflection:EpistemicStatus` has two consumers the removal inventory does not list

The inventory deletes `reflection:epistemic_status` and the four `epistemic:{declared,observed,derived,verified}`
individuals. It does not mention the class `reflection:EpistemicStatus` itself, and **two properties
in a different ontology are typed at it with `allows_only` enforcement at commit**:

| property | file | enforcement |
|---|---|---|
| `objective:acceptance_grade` | `objective-ontology.esl:163` | `class_types reflection:EpistemicStatus`, `allows_only` all four individuals |
| `objective:axiom_kind` | `objective-ontology.esl:194` | same class, `allows_only` observed \| declared |

Deleting the four individuals leaves both properties pointing at an empty enumeration. The property
comment states the distinction the design now removes — *"this is the TARGET grade of an open goal;
`epistemic_status` is the ACTUAL grade of a produced resource"* — and D58 §H2 records the reuse as a
deliberate decision to avoid a parallel epistemics.

**The question P5 must answer**: a Milestone's acceptance criterion is a *target* warrant, and warrant
becomes a query. Either `acceptance_grade` names a warrant predicate the query evaluates, or the
Milestone check is restated over the justification term directly. `objective-ontology.esl` and its 13
files of experiment chains belong in P5's scope; today they are in neither the removal inventory nor
the retype list.

### 8.2 `ExternalExecutionTrace`'s dissolution reopens the problem that created it

The plan argues the class encodes a proxy — who initiated the run — for a question about the plan, and
the argument holds. But eigenius#205 added the class *and* widened `reflection:derivation` from
`ProgramTrace` to the new parent `reflection:ProductionTrace`, because `bench:TaskOutput` requires a
derivation and a declared-external production was unlinkable on any class that requires one.
`bench:Deliverable`'s description states the resulting neutrality explicitly.

Dissolving the class leaves `ProductionTrace` with a single subclass and returns the linkability
question the widening answered. P5 should say what a declared-external production links through —
most likely `wasGeneratedBy` on an activity with no `I → O` plan, which is the paper's own criterion.

### 8.3 The `Warrant` / `Grade` collision has a third occupant

P0 item 3 sweeps for names the design reuses with a different meaning. `objective:acceptance_grade`
and `objective:axiom_kind` both use *grade* for the paper's **grounds**, in a second ontology, with a
third spelling (`kind`). The sweep must cover ontologies, not only Rust.

### 8.4 The 9-ontology count mixes authored with generated

P0 item 2 counts 9 ontologies carrying grade classes. Two of them — `schema-org.eigon.json` (2114
occurrences) and the OBO imports — are emitter output. Separating them changes the estimate: 8
authored files to edit, 2 converters to change, and a regeneration whose golden tests move.

### 8.5 The ACP spec is a downstream consumer with an external audience

Nothing in the build plan names `docs/spec/`. 53 of its 128 normative assertions are in scope and
four are contradicted outright. This is the one affected artifact where the correct action may be a
new version rather than an edit, and that decision should be made before P4 rather than after P5.

### 8.6 EigenQL cannot express the warrant query that replaces the stored grade

P5 makes warrant *"a query over the justification term"*; P7 keeps `project.rs`'s support algebra
*"as a query surface."* **EigenQL is named nowhere in the plan, the paper, D81, or D82** — "query"
there means the Rust API (`support`, `leaves_of`, `survives_without`, `is_fully_verified`).

EigenQL cannot reach it. A proposition or justification term lands as `Value::Json`
(`kernel/src/ontology/resource.rs:29` has no inductive variant), and:

- `values_equal` (`kernel/src/query/functions.rs:134`) has no `Json` arm and falls through to
  `_ => false`. Two identical propositions never compare equal; an equi-join on a proposition slot
  returns zero rows rather than an error.
- `ValueOrVariable` admits a scalar `Literal` or an array pattern — no destructuring into a term, so
  no filter by head predicate, argument, or subterm.
- `DotPath` walks resource → IRI → resource; a `Json` value ends the walk.
- `call_function` has six built-ins (`DATE`, `TIMESTAMP`, `REGEX`, `LENGTH`, `CONTAINS`, `CONCAT`) —
  none decodes, normalizes, or α-canonicalizes a term.

The one accommodation is the postfix `HOLDS` / `FAILS` / `UNDECIDABLE`, which reads `ctor_name` off a
`Verdict`. D2 §3.8 records it as Verdict-specific in v1, with generalisation to any inductive named as
a future revision.

The escape hatch exists and is unused: `reasoning:qc_project_justification` is an OnDemand QueryClass
reachable over `FIBER`, returning a `JustificationProjection` of integers, booleans, and IRI string
arrays — the term flattened into EigenQL's value model. **No notebook, demo, experiment or guide
invokes it.** Its `reasoning:derived_grounds` slot is itself a P4 casualty.

**Why this lands on the plan rather than beside it.** `is_a` membership is an array of IRI strings, so
filtering `MATCH "…:DerivedResource"(?r)` works today — the stored grade is the one epistemic fact
EigenQL can filter on. P5 removes it and replaces it with something computed that EigenQL cannot
compute. Either the plan states that warrant is a Rust-API answer and EigenQL loses grade filtering,
or `qc_project_justification` (or a successor) becomes the supported route and P5 has a consumer to
build against.

### 8.7 The agent skills write the deleted shape

`docs/method/{reasoning,eigenius,grounding}.md` are executable methodology, not documentation. Until
they are updated, any agent following the reasoning protocol authors `DeclaredResource` +
`DeclarationTrace` + `DerivedEvidence` chains — the shape P4 and P5 delete — onto a tree that no
longer accepts it.

---

## 9. Sequencing

Documents follow their phase; nothing here needs to lead. Two exceptions.

| when | what | why |
|---|---|---|
| **before P4** | the ACP spec's version decision (§8.5) | a conformance claim's meaning must not change silently while the phases run |
| **before P4** | `docs/method/` skills (§8.7) | they author chains; a stale skill produces data the new tree rejects |
| with P1 | the `TypeExpr` → `Term` rename across 36 doc files | one mechanical pass, batched with the code rename |
| with P4 | D52, D56, D53, D51; the statistics institution and its 8 test files | the grounds vocabulary change |
| with P5 | the narrative documents (§1.3), the objective ontology (§8.1), guides/README | the four-category framing |
| with P7 | D49, D14/D10, D80, the three structural guide chapters, the website mirror | the kernel/chain boundary and the operational protocol |
| after P7 | D39, D73, D81, D82 status headers; the D70–D82 index gap | retirement is last, so each document is retired against a finished state |

**The guides are the natural last gate.** `composition/07` and `platform/reasoning-institution/README.md`
are both walkthroughs of a fixture that must run; if the rewritten chapter's steps do not reproduce,
the phase is not done.
