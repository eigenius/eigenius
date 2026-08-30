# Build plan — Judgements, Warrants, and Logics

**Status: plan.** Written `2026-08-28`. Implements
[`docs/design/judgements-and-warrants.tex`](../design/judgements-and-warrants.tex). The derivation
record is [D82](../design/d82-propositions-witnesses-and-logics.md); the description of what exists
is [D81](../design/d81-the-epistemic-stack.md), whose findings this plan closes.

**Posture.** The paper specifies replacement, not migration. No backwards compatibility is required:
each reseed rewrites the chain. Phases are ordered so that every gate is runnable and every phase
leaves the tree green.

**Naming convention in this document.** P1.3 renames the justification vocabulary, and it lands
first, so **P2 onward is written in the new names** — `justification:Term`, `justification:Certificate`,
`justification:Sentence`, and the `Declared` / `Observed` / `Verified` leaves. §0, P0, P1, the removal
inventory and the retype table name what is in the tree **today** (`reasoning:JustificationTerm`,
`reasoning:JustifiedBy`, `DeclaredEvidence`, …) because their job is to say where to find it.

Companion: [`judgements-warrants-doc-and-consumer-scope.md`](judgements-warrants-doc-and-consumer-scope.md)
sizes the documentation, institution and demo surface these phases invalidate.

---

## 0. Surface, measured

Counts re-measured `2026-08-29` against the working tree. They size the work; they are not the design.
**Method**, so a later re-measure reproduces them: Rust file counts are over `kernel/src` and
`crates/*/src` (excluding `tests/`); ontology counts are over `ontologies/`; authored-artifact counts
are over `ontologies/`, `demo/`, `experiments/`.

| area | surface |
|---|---|
| grade classes (`{Declared,Observed,Derived,Verified}Resource`) | **26 Rust source files**, **9 ontologies** |
| `eigentt:TypeExpr` | **21 Rust source files**, **52 ontology sites** |
| `urn:eigenius:reasoning` namespace | **~650 occurrences in 103 files** (131 full IRIs, 521 short-form) |
| `*Evidence` grounding-constructor names | **480 occurrences in 56 files** |
| witness machinery | `witness/mod.rs` 397 lines (≈145 of them tests), `layer/witness_index.rs` 521, `program/check_hooks.rs` 101 |
| reasoning crate (post-move, `2026-08-29`) | `extract.rs` 1025, `project.rs` 605, `validate.rs` 265, `institution.rs` 182, `entailment.rs` 113, `consistency.rs` 79, `startup.rs` 33 — **2369 lines in 8 files** |
| `DerivedEvidence` / `IsDerivedAs` in authored artifacts | **153 occurrences** (90 + 63) in **25 files** |
| validation rules | 15 files under `kernel/src/validation/rules/` |

**The reasoning crate was decomposed on `2026-08-29`, before the phases.** It held three unrelated
things. The encoding pipeline's claim machinery — `ClaimGrader`, `ParsedClaimGrader`,
`DerivedClaimLander`, `UNATTRIBUTED_AGENT`, `ClaimSource`, `GradedClaim`, `GradeError`, `Grade`,
`Warrant`, and the whole `KindClassifier` family — moved to `crates/eigenius-encoding`
(`grade.rs` 290, `claim_kind.rs` 436, `land.rs` 171). Its `enc:`-prefixed constants and its
consumers (`emit.rs`, `pipeline.rs`, `formalize.rs`) were always there; `eigenius-encoding` no
longer depends on `eigenius-reasoning` at build time.

A third set was **drift and was deleted**: `DeclaredClaimGrader`, `ProseModusPonens`,
`ChainRuleApplication`, and `DocumentIngestion` / `InProcessIngestion` — 382 lines of `ingest.rs`
plus 778 lines of tests, all reachable only from their own tests. D67 §0 named `ingest.rs` as one of
three overlapping pipeline constructions and D67 is retired (`2026-08-19`); the parsing pipeline
superseded it and the loser was never removed. `DeclaredClaimGrader` was sediment twice over — D67
called it wrong against the then-settled Derived landing, and D73 §6 then replaced that axis.

**This changes two things in the phases below.** `Grade` and `Warrant` now live at
`crates/eigenius-encoding/src/grade.rs:60` and `:74`, so P5 deletes them from there. And P3's exit
gate can no longer be stated through `DeclaredClaimGrader`, which is gone — see P3.

**Three counts were wrong in the `2026-08-28` draft** and are corrected above: grade classes read
21 Rust files (26 under the stated method), `TypeExpr` read 51 ontology sites (52), and the authored
`DerivedEvidence` / `IsDerivedAs` count read 106 (153). P0 re-measures with the method recorded, and
nothing downstream should quote a count without it.

**The 9 ontologies are not 9 edits.** Two are generated: `ontologies/schema-org/schema-org.eigon.json`
carries **2114** of the tree's grade-class occurrences — more than everywhere else combined — and the
OBO imports carry more, both stamped by `crates/eigenius-schemaorg/src/convert.rs` and
`crates/eigenius-obograph/src/convert.rs`. Those are two converter changes plus a regeneration with
moving golden tests, not hand edits; **7 ontologies are authored.**

**Two findings change the shape of the work.**

- **Cycle detection exists.** `kernel/src/layer/declaration_order.rs` already computes a topological
  order over the layer's declaration dependency graph and reports `OrderError::Cycle`, and it already
  reasons about `core:mentions`. §5.3's well-foundedness check extends this rather than building it.
- **The grade classes have writers and no structural readers.** D81 §5.1 established that no reader
  grants an entitlement on an epistemic class. The 21 files above are overwhelmingly *writers* and
  doc comments. P5 is therefore a deletion with a computed replacement, not a consumer migration —
  but see P0, which must confirm this rather than inherit it.

**Bootstrap-touching phases force a reseed** (~12 min, per `bootstrap_drift_reseed`): P1, P2, P5, P7.
Batch bootstrap edits within a phase; do not spread one edit across two.

---

## Disposition: traces and verdicts

The plan's phases move these; this section states where each lands, because neither is named in the
paper and both are load-bearing today.

### Traces

**Only 5 of the 19 trace classes are in scope.** The other 14 — `ComponentTrace`, `PureTrace`,
`MapTrace`, `ReduceTrace`, `CaseTrace`, `SeqTrace`, `LetTrace`, `ProjectTrace`, `ConstructTrace`,
`FieldTrace`, `EmptyTrace`, `ComorphismTrace`, `ProductionTrace` and `Trace` itself — record the
structure of a program run. They are provenance already and nothing here disputes that.

The 5 that feed `trace_category` are the epistemic ones, and they are the ones that stop being a
parallel grade vocabulary:

| trace | requires | disposition | phase |
|---|---|---|---|
| `DeclarationTrace` | `resource`, `declared_by`, `timestamp` | becomes PROV attribution; still grounds `IsDeclaredAs` — the constant specification needs an input | P5 |
| `ObservationTrace` | `resource`, `source`, `timestamp` | becomes PROV primary-source; still grounds `IsObservedAs` | P5 |
| `ProgramTrace` | `resource`, `source`, `timestamp` | becomes a `prov:Activity` and **grounds nothing** — `IsDerivedAs` is deleted | P4 |
| `VerificationTrace` | `resource`, `proof_system`, `proof_term`, `timestamp` | **stops being the licensor**; the judgement grounds `IsVerifiedAs`, and `proof_term` finally names a proof of `P` | P3 |
| `ExternalExecutionTrace` | `resource`, `declared_by`, `source` | **dissolves**; what remains is an `Observed` recording with a plan in provenance | P5 |

`ExternalExecutionTrace`'s dissolution is the one worth arguing. Its criterion is right — *no
`f : I -> O`, so no specification, so nothing entailed* — but it tests for it by asking whether the
system initiated the run. Under §4.1 the question is whether the plan carries an `I → O`
specification, which is a property of the plan and not of who invoked it. The class encodes a proxy
for a question the design asks directly.

**`trace_category` shrinks from five entries to two**, and P5 may retire it entirely in favour of
reading the provenance shape, per its computed-summary rule.

### Verdicts

**Verdicts stay stored, and that is not a change.** A verdict records that an institution was asked a
question and gave an answer at a time, which is provenance, and §3 stores provenance. It is also
already provenance-only in practice: the verdict is read **in flight** — `dispatch.rs:125` matches
`VerdictReading::Fails` to decide whether to reject — and nothing reads the committed resource back.

| verdict | stored | why |
|---|---|---|
| `Fails` | **no** | the commit is blocked, so no layer lands; the rejection reason returns to the caller. A fact cannot be recorded in a layer that was refused. |
| `Holds` | yes | provenance of the institutional act |
| `Undecidable` | yes | the verdict resource **is** the record of a suspension; without it, a below-threshold κ–τ result is invisible |

**What the design removes is the licensing role, not the record.** A `Fails` keeps its one
load-bearing effect — blocking a commit on the institution's authority, which is
wrong-direction-safe, since an incorrect `Fails` loses data and an incorrect `Holds` corrupts.

**A `Holds` grants nothing, and therefore needs no declared post-condition.** The warrant comes from
what the institution *emitted* — a derivation carrying a composite justification term — not from the
verdict. `Undecidable` commits the institution's resources without rejecting the subject, unchanged.

**This retires a conclusion the derivation record treats as central.** D81 §5.6 identified *"whether
AutoOnLoad should have a declared post-condition on success"* as its one design question, and
D82 scheduled it as S2, calling it the root the other findings hang from. Under this design the
question dissolves rather than being answered: a passing gate entitles nothing, so there is nothing
for a post-condition to declare. What D81 actually found — that `Holds` had undefined consequences
while `Fails` had defined ones — is fixed by removing the consequences rather than by declaring them.

**`InstitutionEmittedDerivation` survives** as the institution's output resource. Its warrant comes
from its justification term (composite after P4), not from its class, and
`emit_from_institution_derivation` retires with `IsDerivedAs`.

---

## Disposition: the sentence class

**Added `2026-08-29`, after P1.** Names what `reasoning:ReasoningSentence` — `justification:Sentence`
after P1.3 — becomes. It is a replacement, not a retirement, and it lands in **P2**.

### What it is today

`justification:Sentence : reflection:DerivedResource`, requiring `proposition`, `term` and
`certificate`, recommending `subject_iri`, with `refutes` optional. **No Rust code constructs one** —
every code reference is a consumer, confirmed mechanically (0 sites setting `is_a` to it).

**Counts re-measured `2026-08-29`, post-P1, against `: justification:Sentence {` declarations**
rather than name mentions, which the P1 prose rename inflated:

| | resources | `proposition` | `term` | `certificate` | `subject_iri` | `refutes` |
|---|---|---|---|---|---|---|
| `experiments/` | 40 | | | | | |
| `demo/` | 1 | | | | | |
| `notebooks/` | 1 | | | | | |
| **authored total** | **42** | 45 | 43 | 43 | 42 | **0** |
| fixtures (`crates/*/tests`) | 6 | | | | | |

**The authoring edit is 131 slot writes across 42 resources**, not the 91 an earlier draft of this
section estimated. `refutes` has **no authored use at all** — its 2 occurrences are the ontology
declaration and one doc mention, so the "belief revision" slot is carried forward on its design
argument alone, not on use.

### P2 subsumes three slots into one

A sentence asserts that certificate `c` inhabits `Certificate(j, P)`. Once slots are
`Judgement`-ranged that is one value — `holds(kernel, c, Certificate(j, P))` — with `P` and `j`
appearing inside the type. Three slots checked by three paths become one slot checked by the
uniform rule. P3 then adds a second, optional judgement, `holds(lean4, t, P)`: the proof term.
**The two judgements must not be merged.** One says `j` grounds `P`; the other says `t` proves `P`.

### Two slots are not subsumed, and a third constraint keeps the class

- **`subject_iri`** (42 authored uses — one per sentence, so it is universal in practice) is
  *aboutness*, not logic. Its own description calls it the
  *"first-class EigenQL index … agents querying 'what have I concluded about X?' hit this
  directly."* No judgement carries it.
- **`refutes`** is belief revision — a structural marker for a supersession step, deferred to the
  chain-merge work. **Zero authored uses** (see the table): it is retained on the design argument,
  not on demand. If the chain-merge work does not arrive, this is the slot to drop.
- **Something must keep a resource IRI.** `objective:satisfied_by` (10 occurrences) names the
  sentence that discharged a Milestone; the competency-question answer slot and the
  Tension/Hypothesis resolution slot do the same. A judgement value inside a property is not
  addressable — only a resource is.

So a step reading *"delete ReasoningSentence"* would be wrong. The class becomes a resource
identity, one required judgement, one optional judgement, and two aboutness slots — three required
inductive slots down to one.

```
class justification:Conclusion {
    requires   justification:judgement;      // holds(kernel, c, Certificate(j, P))
    recommends justification:proof,          // holds(logic, t, P) — P3, when one exists
               justification:subject_iri;
    // justification:refutes stays optional
}
```

**The name is a decision, not a derivation.** *Sentence* came from D39's "reasoning sentence" and
stops describing the thing once the three slots collapse. *Claim* is taken by `enc:EncodedClaim`.
*Conclusion* distinguishes it from a parsed claim and says what it is.

### The one open check, and its answer

**Whether any consumer reads `proposition` or `certificate` separately rather than as a pair.**
The collapse assumes they are only ever used together to build `Certificate(j, P)`.

Measured `2026-08-29`. **Two readers take the proposition alone**, and neither blocks the collapse:

| reader | reads | disposition |
|---|---|---|
| `witness_index.rs:264` `emit_from_reasoning_sentence` | `proposition` alone, to build the `Verified` `WitnessKey` | see below |
| `entailment.rs:73` | `proposition` alone, scanning committed sentences | P7 deletes the file |

`validate.rs:65-66` reads `proposition` and `certificate` as a pair, which is the assumed use.

**The witness key survives the collapse, and the reason is that it never depended on the slot.**
`hash_stored_proposition` (`witness_index.rs`) does **not** hash the stored JSON — it runs
`decode_type` to an `Exp` first and hashes *that*, through `hash_proposition_exp`. The emit side is
already a decode-then-hash path. After the collapse it decodes the judgement, takes its `type`,
projects the second index argument of `Certificate(j, P)`, and hashes the same `Exp`. What is
hashed is unchanged; only where the `Exp` is read from moves. The α/δ agreement that
`emit_and_check_sides_agree_on_the_hash` pins is what keeps the emit and check sides equal, and it
is stated over `Exp`, not over the stored value.

**The new obligation is the projection step** — recognising `Certificate(j, P)` in the judgement's
type and taking `P`. That is a structural walk on a decoded `Exp`. Pin it with a test that the
projected hash equals the hash of the same proposition stored flat, or the two sides can drift
silently and a `Verified` witness simply fails to be admitted.

### Residue

| residue | disposition |
|---|---|
| `proposition`, `term`, `certificate` properties | collapse into `judgement` — **P2**, where `Judgement`-ranged slots and the uniform rule arrive |
| P3's proof-term slot | lands as the second, optional judgement, not a fourth slot |
| `emit_from_reasoning_sentence` (`witness_index.rs:262`) | already rekeyed to the checked judgement by P3; reads the new slot |
| `extract.rs`'s `extract_justification` | reads the justification slot today; must read the judgement. **Change the slot before P7 moves the file into the kernel**, so the move is a relocation and not a relocation plus a rewrite |
| `subclass_of DerivedResource` | already on P5's retype list; unchanged by this |
| `VerifiedPropositionView`, `EntailmentRequest`, `ConsistencyRequest` | reference sentences; P7 deletes all three with the institution |
| `entailment.rs`'s scan over committed sentences | P7 deletes |
| `objective:satisfied_by` (9 occurrences), the CQ answer slot, the Tension/Hypothesis resolution slot | retarget to the new class name only — they hold IRIs, so the shape is unchanged |
| 42 authored resources / 131 slot writes | each sentence rewrites 3 slots to 1; batch with P2's reseed |
| `docs/method/reasoning.md` | teaches authoring the 3-slot cluster. It is an executable skill, so **it must change before the ontology does** or agents keep writing the old shape |
| `docs/guides/esl/09-institutions.md` §9.10.3, `composition/07` | teach the resource; rewrite with P2 |
| D54 (sentence-as-lemma) | its subject is citing one of these; the capability survives, the shape changes |
| ACP spec | references the certificate relation in `ACP-A-31`; already inside the 53 assertions in scope |

### Sequencing

**The collapse belongs in P2**, not P3 or P5 — that is where the `Judgement` inductive becomes real
and the uniform rule replaces the three-step check. Doing it there means P3 adds one optional slot
to a class that already has the right shape, rather than a fourth slot to a class about to lose
three.

Two ordering constraints: update `docs/method/reasoning.md` **first**, or agents keep authoring the
deleted shape; and change `extract.rs`'s slot read **before** P7 moves the file into the kernel.

---

## P0 — Measure before building

**No code.** Produces numbers that determine whether P2 and P5 are small changes or data migrations.
Every later phase is sized by this one.

1. **Does the lexicon survive check mode?** Sample `lexicon:cat`, `lexicon:sem_type` and
   `lexicon:term` values across the loaded lexicon; decode each and `check` it against its intended
   type offline. Report the failure rate and the failure shapes. **This is the plan's largest
   unknown**: ~7.6M entries carry these slots, written by three producers
   (`dcg/glossary.rs`, `dcg/augment.rs`, `lexicon-align/emit.rs`), and Rule 21 has never checked that
   a `lexicon:cat` value is a `lexicon:Cat` — it infers a type and discards it.
2. **Confirm the grade classes have no structural readers.** Re-derive D81 §5.1 mechanically: for
   each of the **26** files, classify every non-test occurrence as writer, reader, or comment. A
   single genuine reader changes P5 from deletion to migration. Record the search method with the
   result — §0's three corrected counts came from methods nobody wrote down.
3. **Find every name the design reuses with a different meaning.** `Warrant` and `Grade` are
   already known to be swapped (see P5). Sweep for others before P5 renames anything, since a
   collision found mid-phase is a rename inside a rename.
4. **Count `Sum` and `SpecStr` on the persisted chain.** The tree has no authored `Sum` and its
   `SpecStr` uses are three fixtures, but P4 changes both and a committed chain may carry more. A
   `Sum` whose second branch never grounded stops committing under P4's strengthened rule.
5. **Inventory the persisted chain**, not just the tree: how many `DerivedEvidence` leaves,
   `IsDerivedAs` witnesses and grade-class stamps exist on the current chain. Sizes the reseed and
   the P4 invalidation.
6. **Establish the baseline.** Run the parse gate (`--release`, per `parse_sweep_must_be_release`)
   and the WRN demo end to end on the current branch, and record the numbers. Nothing later may
   regress against this baseline, and a regression must be attributable to a phase.

**Exit:** a numbers note under `docs/notes/`. No source change.

---

## P1 — `eigentt:Term`, the `Judgement` inductive, and the `justification:` namespace

**Bootstrap edit → reseed.** Three renames, batched into one reseed because each touches bootstrap.

- Rename `eigentt:TypeExpr` to `eigentt:Term` across `ontologies/` (51 sites) and Rust (21 files).
  The 20 constructors are unchanged; the class was named for the type-level fragment it originally
  carried and has held lambdas, pairs, projections and literals for some time.
- Declare `eigentt:Judgement` as an inductive with one constructor
  `holds(logic, term, type)`, and `eigentt:Logic` with the two inhabitants the system can check.
- **Move the justification calculus out of `reasoning:` into `urn:eigenius:justification`.**

### P1.3 — the `justification:` namespace

**`reasoning:` names an activity, not a subject matter.** Every other ontology in the tree names what
it declares — `statistics`, `lexicon`, `logic`, `reference`, `formulas`, `objective`. *Reasoning* is
what the whole platform does, and the word already has three referents: this ontology, the
`eigenius-reasoning` institution, and the `docs/method/reasoning.md` agent skill. The vocabulary it
declares is a justification calculus, which is the name it takes.

| current | after |
|---|---|
| `reasoning:JustificationTerm` | `justification:Term` |
| `reasoning:JustifiedBy` | `justification:Certificate` |
| `reasoning:ReasoningSentence` | `justification:Sentence` |
| `reasoning:JustificationProjection` | `justification:Projection` |
| `reasoning:ProjectionRequest` | `justification:ProjectionRequest` |
| `reasoning:proposition`, `reasoning:certificate` | `justification:proposition`, `justification:certificate` |
| `reasoning:justification` | `justification:term` |
| `reasoning:VerifiedPropositionView`, `Entailment`/`ConsistencyRequest` | `justification:` prefix, otherwise unchanged |
| `DeclaredEvidence`, `ObservedEvidence`, `VerifiedEvidence` | `Declared`, `Observed`, `Verified` — see below |

**The grounding constructors are renamed too, and this is not cosmetic.** The paper's taxonomy is a
table of three **grounds**, and it is explicit that a declaration is not evidence for `P`: *Declared*
establishes *"agent `a` asserted `P`"*, is **postulated** rather than proved, and *"when the
established proposition differs from the claim, an accountable party must declare the premise bridging
the two."* Naming the leaf `DeclaredEvidence` asserts the thing the design is most careful to deny.

The rename also removes a split the code carries today: the term side says `DeclaredEvidence` while
the reading side says `Ground::Declared`, for the same object. After it, `Ground::from_ctor` maps
`"Declared" → Ground::Declared` instead of translating, and P4's exit gate
(`leaves_of(term, Observed)`) reads against a constructor of that name.

**This plan already assumed the rename without declaring it.** P4 states the institution's new output
as `App(Declared(plan), Observed(inputs))` — the new names — while P3 twenty lines earlier says
`DeclaredEvidence`. The tree has `DeclaredEvidence`. Declaring it here makes the two consistent.

**`justification:Term`'s constructors after P1.3 and P4**: `Declared`, `Observed`, `Verified`, `App`,
`Sum` — five. P4 removes `DerivedEvidence` with the grounds change and `SpecStr` with the algebra.
Surface for the three renames: **480 occurrences in 56 files**, overlapping the namespace pass, so
both land in the same edit. `justification:Certificate`'s constructor names
(`declared`, `observed`, `verified`, `app`, `sum_l`, `sum_r`, `spec_poly`) already match and do not
change — though P4 alters the signatures of `sum_l`, `sum_r` and `spec_poly`.

**`SpecStr` is not renamed because P4 deletes it** — the naming question turned out to be a
structural one. See P4.

**`urn:eigenius:reasoning` retires entirely — there is no institution left to keep it.** An earlier
draft of this section split the namespace, leaving the institution resource and its QueryClasses under
`reasoning:`. That split does not survive P7: the ExportFormat's only caller retires, consistency is a
stub, entailment and projection have no callers, and the paper names no reasoning institution. P7
deletes all of them. **Move every surviving declaration to `justification:` in this phase and delete
the institution resources in P7**, rather than renaming them at P1 to delete them six phases later.

**Surface**: the namespace itself is ~650 occurrences in 103 files — 131 full IRIs (crates 17, experiments 15, kernel 6,
ontologies 2, docs 2, demo 2, notebooks 1) and 521 short-form `reasoning:` prefixes resolved through
per-file `namespace` declarations, so most ESL files change one line plus their bodies.

**Two moves this forces, both already required by later phases.**

1. **`witness:` must leave.** It is `urn:eigenius:reasoning:ChainWitness` (16 sites) — a child of the
   namespace being vacated. P7 moves `Is*As` into kernel base vocabulary independently, so the IRI
   has to move regardless; P1 should not park it under `justification:` on the way. Land it at its
   P7 destination directly.
2. **`reflection:warranted_by` must be settled before the word *warrant* is used anywhere else.**
   It is D72's warrant axis, the same word P5 rules is provenance, and it is in active use at **161
   occurrences** across every WRN chain file, the objective and benchmark experiments, and eight
   fixtures. It appears nowhere else in this plan. See P5.

**`warrant:` was the runner-up and is rejected for now.** It is the paper's headline axis and reads
better, but it collides with `warranted_by` above, and P5 establishes that warrant is computed and
stored nowhere — so an ontology named for it would declare the algebra warrant is computed *over*,
not warrants. `justification:` names what is actually in the file.

**No behaviour change.** Nothing yet ranges over `Judgement`; validation is untouched; the renames are
IRI substitutions with no semantic content.

**Exit:** reseed clean at the P0 resource count; `cargo test --workspace`, clippy and fmt green;
parse gate and demo unchanged against the P0 baseline; no `urn:eigenius:reasoning:` IRI names a class,
property or inductive. The institution resources still carry the old prefix at this point and leave
with P7.

---

## P2 — Uniform check-mode validation

**Depends on P1. Bootstrap edit → reseed.**

**Read [*Disposition: the sentence class*](#disposition-the-sentence-class) with this phase.** The
sentence's three required inductive slots collapse into one `Judgement`-ranged slot here, not in a
later phase — it is the same edit as the rule change, and deferring it means P3 adds a fourth slot
to a class about to lose three.

Replace Rule 21's three-step shape (decode, `check_infer`, plus a `PROPOSITION_SLOTS` special case)
with one rule over `Judgement`-ranged slots: decode both fields, check the type is a type, check the
term against it in **check mode**.

Retire, in the same phase:

- `wk::PROPOSITION_SLOTS` — the hardcoded list of slots required to hold propositions;
- the `eigentt:definition_body` exemption (Rule 24's separate check subsumes into the uniform rule);
- the `core:param_kind` / `core:type_name` exemption (Rule 23's telescope-scoped check);
- the `check_infer`-then-discard path for every other `Term`-ranged slot.

**P0's measurement decides the migration.** Slots holding bare lambdas with their type in a
neighbouring field become single `Judgement` values. Slots whose values are already self-describing
may instead require `Ann`, which the kernel's term language already provides and whose typing rule is
exactly this rule (`check_infer(Ann(e,T))` checks `e` against `T`).

**Exit:** the four obligations D81 recorded as declared-but-unchecked — `lexicon:cat` against
`lexicon:Cat`, `lexicon:sem_type`, `eigentt:axiom_statement`, `eigentt:definition_type` — are
enforced, with a test each. No exemption list remains in `eigentt_value.rs`.

---

## P3 — The proof term, and the two-layer fix

**Depends on P2.** The soundness fix; it closes D81 §5.2.

- `justification:Sentence` gains a `Judgement`-ranged slot carrying a term checked against the
  sentence's `proposition`.
- `verification_trace` (`crates/eigenius-reasoning/src/validate.rs`) stops writing the sentence's own
  IRI into `reflection:proof_term`. For `proof_system = kernel`, `proof_term` names a term the kernel
  checked at `t : P`. A certificate has type `Certificate(j, P)`, not `P`, and no rule
  connects them.
- `emit_from_reasoning_sentence` (`kernel/src/layer/witness_index.rs`) stops minting `Verified` from
  `is_a` membership plus a hashable proposition. It keys off the checked judgement.

**Exit gate — write this test first and watch it fail.** Build a `Declared` claim, cite it from a
second sentence with `Certificate.verified`, and assert `is_fully_verified` returns **false**. On the
current kernel it returns true: the witness emitter mints `IsVerifiedAs` from `is_a` membership plus
a hashable proposition regardless of what grounded the sentence, and the citation path is the
documented one.

**State the gate against the live path.** An earlier draft demonstrated this through
`DeclaredClaimGrader` — which was deleted on `2026-08-29` as drift, reachable only from its own
tests. A gate written against dead code demonstrates a defect on a route nothing takes. Build the
`Declared` claim through `ParsedClaimGrader` (`crates/eigenius-encoding/src/grade.rs`), the grader
the parsing pipeline actually uses, or against `emit_from_reasoning_sentence` directly. The test must fail before P3 and pass after.

---

## P4 — Three grounds

**Depends on P3.** Invalidates authored artifacts; batch with P5's reseed if convenient.

### Landed `2026-08-30`

Three layers moved (`justification`, `statistics`, `reflection`); the pin in
`kernel/tests/bootstrap_manifest_pinned.rs` is updated and carries the account. Workspace green.

What the phase found that the plan did not predict, in the order it mattered:

- **The out-of-band programs had no provenance at all.** All 12 program resources already declared
  `input_type -> output_type` and their inputs were content-addressed, but **0 programs carried a
  `DeclarationTrace` and 19 of 21 inputs carried no `ObservationTrace`**. `emit_from_trace` resolves
  a trace's target on the chain, so both had to be committed before any citation could be rewritten.
  New chain file `08a-program-provenance.esl`: 8 program claims, 8 input observations, one warrant.

- **The plan declarations as first written were false.** They read
  `forall s. Asserts(s) -> STAT(s)` — a claim about the method, and the shape a universal wants.
  It asserts that *every* recorded sample set has the stated statistic, which is a claim about data
  nobody has seen. Narrowed to pin the input. Side effect: `SpecStr` occurrences in the WRN chain
  fell 45 → 3.

- **`ProgramTrace` and `InstitutionEmittedDerivation` now emit NO witness.** The plan said to change
  the two producers to emit a composite; what they actually had to do is stop. A run record grounds
  nothing, and the removal is forced rather than chosen: `WitnessCategory::Derived` could only ever
  be consumed by `Certificate.derived`, so once that constructor goes no lookup can ask for it.

- **Seven tests stopped needing Docker.** `wrn_phase3`'s R-runtime `#[ignore]`s are gone and the
  `R_RUNTIME` pending allowance is deleted from both `wrn_phase3.rs` and `wrn_phase5.rs`. Measured,
  not assumed: emptying the allowance and running is what showed all seven now Hold. They needed a
  runtime because `DerivedEvidence(<program>:result)` cited the program's OUTPUT resource, which
  only R could commit; `App(Declared(plan), Observed(input))` cites two chain-resident facts, so
  type-checking no longer requires having run the analysis. The demo still runs lme4 for real and
  that is still what checks the numbers.

- **The `Sum` strengthening cost zero, as predicted, and is now tested at commit.**
  `crates/eigenius-reasoning/tests/sum_requires_both_branches.rs` — a `Sum` over two grounded
  branches Holds; one whose fallback cites an ungroundable IRI is refused with a missing-`IsDeclaredAs`
  diagnostic naming the branch.

- **The notebook `notebooks/examples/stats-and-reasoning.json` was already broken**, independent of
  this phase: it used `spec_str`, retired `2026-08-21` by eigenius#203. Nothing executes it, so
  nothing noticed. Updated to `spec_poly` and the composite shape, plus the plan declaration cell it
  now needs.

- **`spec_poly` is 5-ary at all 8 call sites.** The audit tag is gone and the result index is `j`.
  Two sites were missed on the first pass because the rewriter skipped calls NESTED inside one it had
  already rewritten; a third-pass check by arity caught it. Comments containing a comma
  (`// tag : audit label, same IRI as the instance`) also broke the argument splitter until it
  learned to skip line comments.

**Exit criteria, met.** `leaves_of(term, Observed)` returns the sample set and
`survives_without(dataset)` returns false — `a_computed_ground_projects_to_its_plan_and_its_input`
in `project.rs`. Both answered wrongly before, in the reassuring direction.

### Prerequisite found `2026-08-29` — the analysis plans are not declared

**`App(Declared(plan), Observed(inputs))` needs BOTH witnesses to resolve, and the plan half does
not exist.** Measured against the WRN chain:

| resource | trace it carries | witness | composite half |
|---|---|---|---|
| `wrn_dep_sampleset` | `ObservationTrace` | `IsObservedAs` | `Observed(inputs)` ✅ |
| `bridge_msi_selective` | `DeclarationTrace` | `IsDeclaredAs` | ✅ |
| `wrn_dep_plan` | **`ProgramTrace`** | `IsDerivedAs` — deleted by this phase | `Declared(plan)` ❌ |

**0 of 21 `stats:StatisticalAnalysisPlan` resources carry a `DeclarationTrace`.** Every one is
traced only by the `ProgramTrace` of its run.

**That is the right defect to find, and it is the design's central claim in miniature.** A
`ProgramTrace` records *that a run happened* — provenance. What the composite needs is the
assertion *that this plan denotes a function `I → O`*, which is a claim an accountable agent makes
and which no execution can establish: determinism is an empirical fact about the environment, not a
property recoverable from a run record. So the plans must be **declared**, and by someone.

**P4's shape changes accordingly**: author 21 `DeclarationTrace`s (with agents and rationales)
BEFORE the citation rewrite, or every rewritten `App(Declared(plan), …)` fails to resolve and the
chains go from wrong to uncommittable. Sequence: declare the plans → rewrite the 32
`DerivedEvidence` sites → rewrite the ~30 `Verified(…)` citations → delete the constructors.

**This is also the first place the refactor asks for a judgement no one has recorded.** Twenty-one
plans need an agent willing to assert reproducibility. That is not a mechanical edit, and pretending
otherwise by minting stub declarations would reproduce exactly the pattern P5 deletes — a grade
conferred by the importer that wrote the resource.

### Scope grew after P3 — the citation spine

**Added `2026-08-29`, from P3.** P3 closed its gate by minting `Verified` only from a proof
judgement. That exposed a pattern the plan did not size: **D54 lemma citation is the laundering
step, and it is how the project composes conclusions.**

| file | `Verified(…)` citations |
|---|---|
| `wrn-helicase/chain/09-phase5-synthesis.esl` | **14** |
| `wrn-helicase/chain/07-phase2-validation.esl` | 4 |
| `wrn-helicase/chain/08-phase3-invivo-mechanism.esl` | 4 |
| `d57-schema-org/chain/05-synthesis.esl` | 8 |

Not one cited conclusion is proved. They rest on `Declared` and `DerivedEvidence`:
`concl_val_recomputed` (Declared + DerivedEvidence), `concl_vivo` (DerivedEvidence),
`concl_helicase_required` (Declared), `concl_mech` and `concl_mmr` (Declared + Verified).
Citing them as `Verified(iri)` **is** the reduction of `Judgement(kernel, c, Certificate(j,P))` to
`Judgement(kernel, t, P)` that the design names as inexpressible. It passed for months because the
emitter minted `Verified` from `is_a` membership without inspecting what grounded the conclusion.

**The repair belongs here, not in P3, for three reasons that are P4's to settle.**

1. **P4 decides what a cited conclusion decomposes INTO.** Deleting `DerivedEvidence` takes the
   grounds to three, so a computed conclusion becomes `App(Declared(plan), Observed(inputs))`.
   `concl_vivo` rests on a `Derived` leaf today and on an application after P4 — the citation's
   replacement differs before and after, and only the second is worth writing.
2. **`08-phase3-invivo-mechanism.esl` composes with the `derived(…)` certificate constructor**,
   which P4 removes. Rewriting its spine earlier guarantees rewriting it twice.
3. **There is a second laundering path and P4 closes it.** `check_layer_with_coercion` lets a
   `Verified` witness satisfy a `derived(…)` citation. P3 narrowed what MINTS a Verified witness;
   P4 removes the coercion that SPENDS one. Repairing citations between the two aims at a moving
   target.

**The capability is not lost.** `Certificate.app` already composes certificates: a synthesis takes
the cited conclusion's certificate as its antecedent instead of a fresh `Verified` leaf. What goes is
citation *by IRI* for unproved conclusions — which is the point, since the IRI was standing in for a
proof that does not exist.

**Cost of the deferral, stated:** `d57_chain_validates` and the WRN demo stay red between P3 and P4.
That is the honest state — those chains assert `Verified` for conclusions never proved — and making
the tests green before the data is right would be the wrong order.

**Also carried into P4 from P3:** `reflection:proof_term`'s description still reads *"for the kernel,
the chain-resident IRI of the `justification:Conclusion` whose CERTIFICATE type-checked"* — the
defect stated in the ontology. Correcting it changes the bootstrap manifest, so it rides P4's
reseed rather than invalidating P2's mid-flight.

| | current | after |
|---|---|---|
| `justification:Term` constructors | 7 | **5** — `DerivedEvidence` and `SpecStr` removed |
| `witness:Is*As` | 4 | 3 — `IsDerivedAs` removed |
| `justification:Certificate` grounding constructors | 4 | 3 |
| `justification:Certificate` constructors total | 9 | 8 — `spec_poly` stays, its index changes |
| `project.rs`'s `Ground` | 4 | 3 |

- Institutions emit a **composite** justification term — `App(Declared(plan), Observed(inputs))` —
  in place of a single opaque leaf. The leaf names are P1.3's; before that rename they read
  `DeclaredEvidence` / `ObservedEvidence`. `emit_from_institution_derivation` and the statistics institution
  are the two producers to change.
- **Remove `SpecStr` from the term algebra; keep `spec_poly` as a rule.**

  The paper's algebra has two operations — *"an algebra of justification terms that supports
  application and sum."* `SpecStr` is D39's third, and its second field is the **only unchecked
  argument in the algebra**: `spec_poly` binds the instance `x : T` and the tag independently, with
  nothing relating them, so the tag is a free string the author picks and no rule validates. Every
  other constructor argument is checked — a leaf's IRI is consumed by the matching `Is*As`, `App`
  and `Sum` are structural.

  Nothing is lost. `support` already discards it (`SpecStr(j, tag) → support(j)`), and the instance
  survives in two places that *are* checked: `spec_poly`'s `x : T` binding and the proposition `P(x)`.

  The rule stays and loses its index change:
  ```
  spec_poly : forall (T, P, j, x : T) =>
              Certificate(j, forall (y : T) => P(y)) -> Certificate(j, P(x))
  ```
  Universal literature rules are load-bearing — D66 rests on them and the DCG parser emits
  class-quantified rules — so only the term record goes, not the capability. `Certificate` stays at
  `Type 2`: the universe is forced by `spec_poly` binding `T : Type 1`, which is unchanged.

  **Two consequences, both stated rather than discovered later.** `spec_poly` becomes a certificate
  constructor with no corresponding term constructor, so `spec_poly` and `declared` can both target
  `Certificate(Declared(rule), P(x))` — the term stops determining which rule applies at that node.
  Checking is unaffected because the certificate names its own constructor, and `app`'s intermediate
  `A` already made inferring a certificate from a term undecidable, so this adds no new limit. And it
  **resolves P6's open filter choice** — see there.

- **Strengthen `sum_l` / `sum_r` to require the other branch.**

  Today `sum_l : forall (P, j1, j2) => Certificate(j1, P) -> Certificate(Sum(j1, j2), P)` leaves `j2`
  bound and **unconstrained**: nothing requires the unused summand to be justified, groundable, or to
  name a resource that exists. That is faithful to Artemov, whose axiom `t:F → (t+s):F` quantifies
  over an arbitrary `s`.

  **But `support` reads `Sum` disjunctively and reports the unchecked branch as an alternative.**
  `Sum(real_evidence, Declared("urn:does-not-exist"))` type-checks through `sum_l`, and
  `survives_without(real_evidence_iri)` returns **true** — the conclusion "survives" losing its only
  grounded evidence, by way of a branch nothing ever grounded. That is the counterfactual D73 §1.2
  calls the whole argument for retaining the polynomial, answered in the reassuring direction. The
  existing test cannot catch it: `a_second_source_would_make_a_recompute_droppable` builds raw `Exp`
  with no chain and *asserts* "the DRIVE branch carries it alone" as a premise.

  Both constructors take a derivation for each branch; which was preferred is still recorded by the
  choice of `sum_l` against `sum_r`:
  ```
  sum_l : forall (P, j1, j2) => Certificate(j1, P) -> Certificate(j2, P) -> Certificate(Sum(j1,j2), P)
  ```
  **This departs from LP's axiom deliberately.** The paper makes the system's position in the
  J / J4 / JT family an ontology edit, and under this design an unverified alternative is not a
  ground — asserting a fallback should oblige you to show the fallback works. It also makes the two
  objects agree about `Sum`: the term claims two grounds are available, and the certificate now
  demonstrates both, so `support`'s term-only reading is sound and
  `kernel/src/justification/` stays pure (P6.0's fork, resolved toward A).

  **Cost, measured: zero today.** Authoring a `Sum` becomes more expensive — both branches must
  ground — but **no authored artifact in the tree uses `Sum`**. The only occurrences are the
  declaration itself and two synthetic `Exp` values in `project.rs` and `tests/projection.rs`. So the
  strengthening breaks nothing now, which is the argument for doing it now rather than after the
  first real `Sum` is committed.

- Remove the hardcoded `IsVerifiedAs → IsDerivedAs` coercion in `check_layer_with_coercion`. It
  implements a lattice the paper rejects, and it is not driven by the ontology's `subclass_of`.
- Repair the 106 authored occurrences in `ontologies/`, `demo/` and `experiments/`.

**Exit:** for a statistics-derived claim, `leaves_of(term, Observed)` returns the sample set and
`survives_without(dataset)` returns false. Both answer wrongly today, in the reassuring direction.
Plus a test that a `Sum` whose second branch cites an ungroundable IRI is **rejected at commit** —
today it commits and `survives_without` reports the real ground as droppable.

---

## P5 — Provenance and warrant as independent axes

**The largest phase. Bootstrap edit → reseed.** Depends on P4 for the grounds vocabulary.

### Landed `2026-08-30` (reseed still owed)

**Provenance became its own namespace and its own layer**, `ontologies/prov/prov.esl`, mapped
onto W3C PROV-O rather than adopting its IRIs. That was not in the plan; the plan had the
vocabulary staying in `reflection`. The reason to move it: `reflection` held two unrelated families
under one word — `reflection:Trace` with `LetTrace` / `MapTrace` / `CaseTrace` records how a PROGRAM
EVALUATED, while the parentless `DeclarationTrace` / `ObservationTrace` / `ProductionTrace` /
`VerificationTrace` record HOW A RESOURCE CAME TO EXIST. `prov` sits ABOVE `reflection` and the
direction is forced: `prov:ProgramTrace` reaches down through `prov:trace_tree` and
`reflection:output`, and nothing reaches back.

**`prov:Activity` was the axis's missing middle.** 153 sites named the event that produced an
observation in a free-text string, so the origin was not a node and *which claims rest on this
instrument* had no answer. 50 distinct WRN strings collapsed to 25 activities on one reading: the
activity is the measurement occasion, and the rest of each string described the slice taken from it,
which belongs on the entity. `runtime:RuntimeInvocation` now subclasses `prov:Activity` — it was
already one, requiring inputs, an output, and start/end times.

**Renamed where the old name was wrong**, not merely re-namespaced: `declared_by` →
`prov:was_attributed_to` (2,461), `source` → `prov:was_generated_by` (192, retyped from string),
`warranted_by` → `prov:had_primary_source` (206, retyped). `objective:warrant` →
`objective:rests_on` with it, which the plan required be settled together.

**The 144 resources using a grade class AS their class split by the design's own mechanical test** —
does the resource carry a proposition: 83 → `justification:Claim` (which REQUIRES
`canonical_proposition`), 58 → `prov:Source`. `Claim` is ground-neutral: two of them are instrument
readings, and `Certificate.declared` and `.observed` both cite this kind, so which ground applies is
which trace the resource carries. Making the ground a KIND OF RESOURCE is exactly what the grade
classes did wrong.

**`lexicon:grade` is the thesis at maximum cardinality**, as P0 predicted: 2,641,713 stamps on the
converted chain, every one `epistemic:declared`. A lexical entry carries no proposition, so it has no
warrant to grade — and the single value it ever took says so, because nothing ever climbed.

#### What a textual rename does not reach

Four classes of site, each found by a failing test rather than by grep, and now recorded because the
next namespace move will hit all four again: IRIs built by interpolation (`format!("{REFL}:rationale")`);
non-canonical aliases (`namespace ref = "urn:eigenius:reflection"`); files using the new prefix
without declaring it, in `.esl` AND in ESL embedded in Rust string literals; and hand-built layer
chains in tests. Use `cargo test --no-fail-fast` so all of them report in one pass.

#### The gap that hid most of this

**`esl::compile_against_layer` compiles and does not validate, and `LayerBuilder::build` does not
either** — so the authored chains were checked for their conclusions' certificates and never for
structural validity. `00-wrn-vocabulary.esl`'s own header records the same gap costing two months in
2026-06. All four WRN harnesses now assert `Validator::validate()`, and it immediately found six
things: `prov` / `reference` / `ingest` missing from the test chains; **`eigentt:Judgement` slots
validated by nobody** (Rule 21 selected only on `eigentt:Term`, so judgements fell to Rule 16's D32
walk while being D47-encoded — every judgement on every chain reported "ctor `App` not declared");
`ExternalExecutionTrace` dissolving into `DeclarationTrace` rather than ProgramTrace; eigenius#205's
linkability question answered by requiring the ACTIVITY instead of a ProductionTrace; **08a's eight
`PinnedExternalFile`s being hash-less stand-ins whose IRIs were truncated prefixes of files that
already existed content-addressed**; and `wrn_phase1_recompute` never loading `02-literature.esl`.

**The notebook was already broken and nothing ran it** — `spec_str`, retired 2026-08-21.
`kernel/tests/stats_notebook_cells_compile.rs` now compiles its cells cumulatively.

**Exit criteria met**, except the reseed: no resource carries a stored epistemic grade; provenance is
relational and queryable in EigenQL; warrant is a Rust-API answer and `justification.esl` now says so
plainly rather than leaving "warrant becomes a query" to be read as an EigenQL query; the notebook is
updated and compiles.

**Still owed:** the reseed (P4's and P5's, batched), the ACP spec, and `docs/method/`.

- **Delete the grade classes** `{Declared,Observed,Derived,Verified}Resource` and
  `reflection:epistemic_status`. The latter lets a trace nominate the grade of its own output, which
  is the self-nomination the design forbids; its zero readers are correct and the declaration is the
  defect.
- **Break `VerifiedResource subclass_of DerivedResource`.** Verified is not a special case of
  derived. The relation carries no `requires` inheritance in practice — `DerivedResource` requires
  nothing — and nothing constrains a property to `DerivedResource` via `allows_only` or
  `class_types`, so the removal is free.
- **Provenance becomes relations.** The starting points mostly exist: `reflection:declared_by` is
  already resource-typed and ranged on `reflection:Agent`, which is `prov:wasAttributedTo` in all but
  name, so the `Declared` constant specification can read it unchanged.

  **`reflection:source` is the one real gap: it is a `string`.** The origin of an observation is
  therefore not a traversable relation, so *which claims rest on this instrument* is unanswerable and
  a warrant computed from relations has nothing to walk. **Retyping it to a resource reference is on
  this phase's critical path**, independent of any PROV decision. Its consumers must be found first —
  a string-valued property is read as text.

- **`reflection:warranted_by` is D72's warrant axis, and this phase decides what it becomes.** Its
  description reads *"the criterion, convention, source, or prior result that grounds this
  declaration — what warrants it, as opposed to who asserted it."* Under this design that is not
  warrant: warrant is computed from the justification term, and what `warranted_by` records is the
  declared reason a declaration was made — provenance. It carries **no `class_types`** (D72 §3.3:
  warranting resources are heterogeneous and no class covers them), so it is an untyped resource
  pointer, the same defect `reflection:source` has one step removed.

  It is also the most-used epistemic property outside the grade classes: **161 occurrences**, across
  every WRN chain file, the objective and benchmark experiment chains, `demo/prose-to-formulas-v2`,
  and eight test fixtures. Three options, in order of preference: retype it as a provenance relation
  under the PROV mapping and rename it to say provenance; keep it as an author-facing rationale
  pointer and rename it out of the warrant vocabulary; or delete it and let `rationale` +
  `declared_by` carry the load. **The name must be resolved either way** — P1.3 declines `warrant:`
  as a namespace only because this property still holds the word.

- **`reflection:EpistemicStatus` has two consumers in a different ontology, and they are not in the
  removal inventory.** Deleting the four `epistemic:*` individuals leaves `objective:acceptance_grade`
  (`objective-ontology.esl:163`) and `objective:axiom_kind` (`:194`) typed at an empty enumeration —
  both carry `class_types reflection:EpistemicStatus` with `allows_only` enforced at commit, and D58
  §H2 records the reuse as a deliberate decision to avoid a parallel epistemics.

  A Milestone's `acceptance_grade` is a **target warrant**, and warrant becomes a query. Either it
  names a warrant predicate the query evaluates, or the Milestone check is restated over the
  justification term. `objective-ontology.esl` and the 13 experiment-chain files under
  `experiments/objectives/` (87 occurrences) belong in this phase; today they are in neither the
  removal inventory nor the retype table. The class `reflection:EpistemicStatus` goes with them.

- **`ExternalExecutionTrace`'s dissolution reopens the problem that created it.** eigenius#205 added
  the class *and* widened `reflection:derivation` from `ProgramTrace` to the new parent
  `reflection:ProductionTrace`, because `bench:TaskOutput` requires a derivation and a
  declared-external production was unlinkable on any class that requires one;
  `bench:Deliverable`'s description states the resulting neutrality explicitly. Dissolving the class
  leaves `ProductionTrace` with a single subclass and returns the linkability question. State what a
  declared-external production links through — most likely `wasGeneratedBy` on an activity carrying
  no `I → O` plan, which is §4.1's own criterion.

- **EigenQL cannot compute the replacement for the stored grade, and this phase should say so.**
  `is_a` is an array of IRI strings, so `MATCH "…:DerivedResource"(?r)` works today; the grade is the
  one epistemic fact the query language can filter on. Its replacement is a query over the
  justification term, and EigenQL cannot express one: a term lands as `Value::Json`
  (`kernel/src/ontology/resource.rs:29` has no inductive variant), `values_equal`
  (`kernel/src/query/functions.rs:134`) has no `Json` arm and falls through to `_ => false`, patterns
  admit only scalars and array patterns, and no built-in decodes a term. **Warrant is a Rust-API
  answer in the interim** — `kernel::justification` after P6.0 — and this phase must say that plainly
  rather than leave "warrant becomes a query" to be read as an EigenQL query. The real fix is the
  λProlog extension over EigenQL's Datalog foundation, tracked separately; see the companion scope
  note §8.6 for the constraints it inherits (δ and ι are not in αβη; full HOU is undecidable).

- **The PROV mapping is interop, not correctness, and can land after this phase.** The minimum is
  three classes (`Entity`, `Agent`, `Activity`) and two to five properties: `wasAttributedTo` for
  `Declared`; `hadPrimarySource`, or `wasGeneratedBy` + `used` + `wasAssociatedWith`, for `Observed`;
  and a time term. **Map rather than adopt**, following D57's schema.org precedent: PROV is OWL with
  open-world semantics while these resources are typed with `requires` enforced at commit, so
  adopting the IRIs means redeclaring them in this type system anyway. Note also that
  `prov:hadPrimarySource` is a subproperty of `prov:wasDerivedFrom`, the relation §3.1 names as the
  trap — harmless, since the warning concerns using it as warrant, but it is an entanglement rather
  than a clean borrow.
- **Warrant becomes a query** over the justification term. Nothing stores it. Index it if the cost
  requires; an index is a cache rebuildable from the relations, which a stamp is not.
- **Resolve the `Warrant` / `Grade` name collision, which is a swap.**
  `crates/eigenius-encoding/src/grade.rs` declares `Grade {Declared, Observed, Derived, Verified}`
  — the paper's **grounds** — and `Warrant {Declared, Parsed}`, documented as *"the axis along which
  the grade climbs"* and projecting onto a `Grade`. The paper uses *warrant* for the axis whose
  values are grounds, so the two words currently mean each other's referent.

  **`Warrant`'s distinction is provenance, not warrant.** Both variants project to `Grade::Declared`;
  what separates them is that a parse run produced one. The code already states this correctly on the
  projection — *"the parser is a formulation instrument, not a warrant"* — so the insight is present
  and only the name is wrong. Under this phase the distinction becomes `wasGeneratedBy(parse run)`
  against its absence, with `wasAttributedTo(agent)` on both, and the enum has nothing left to carry.
  `grade()` retires with `Grade`.

  Its `#[non_exhaustive]` growth path — *"the `Observed`/`Verified` climbs are the next increments"* —
  is superseded: those are grounds, not refinements of a ground.

- Update the 21 Rust writers and 9 ontologies accordingly.

**Exit:** no resource carries a stored epistemic grade; provenance and warrant are answerable as
queries; `notebooks/examples/stats-and-reasoning.json` is updated and runs.

**A correction to an earlier draft of this line.** It called that notebook *"the one consumer found
filtering on a grade class."* It is not: all 41 of its grade-class occurrences are in ESL authoring
cells and markdown prose, and its three EigenQL queries pivot on `Verdict` + `verdict_subject` +
`ctor_name`, touching no grade, proposition or justification. It is an **author** of the deleted
shape, like the WRN chain — which is a larger job than updating a filter, and it means **no EigenQL
consumer of a grade class was ever found.**

---

## P6 — Well-foundedness

**Depends on P4.** Independent of P5.

Reject at commit any justification whose premise's support transitively includes the premise. Extend
`declaration_order.rs`'s existing topological pass rather than adding a second graph walk: it already
detects cycles over the declaration dependency graph and already consumes `core:mentions`.

**A justification term's premise citations do reach the index**, which is not obvious from the code
and is now pinned by a test (`a_grounding_leafs_string_iri_becomes_a_mentions_triple`). The grounding
leaves take the premise IRI as a `core:string` argument rather than a `ConstRef`, but
`justification:term` is declared `core:inductive`, the indexer walks any such property, and
`json_mentions` matches any `urn:`-prefixed string at any depth. Only `InductiveType` objects are
dropped, by the D79 seal.

### P6.0 — the support algebra moves into the kernel first

**`core:mentions` is too coarse for this check in two independent ways, and only one of them is a
filtering problem.**

The first is the superset problem below: proposition edges and justification edges leave the same
subject, and the index does not record which slot a mention came from. That is fixable by filtering.

**The second is `Sum`, and filtering cannot fix it.** This phase's condition is stated over a term's
*support* — its disjunctive normal form — and support reads `Sum` disjunctively: `Sum(a, b)` is
carried by either branch alone. A cycle through `a` where `b` is acyclic leaves the conclusion
well-founded, because the `b` alternative still carries it. `core:mentions` records both branches'
edges undifferentiated, so a cycle walk over the edge set rejects that commit. **That is a false
rejection**, and by this plan's own wrong-direction-safe reasoning it is the losing direction: an
incorrect reject destroys data, where an incorrect admit is caught by the next check. Reference edges
cannot distinguish a conjunctive `App` from a disjunctive `Sum`, and no predicate filter recovers the
distinction, because it is not in the edges at all.

**So the kernel needs the algebra, not the edge set** — which is the case P7's load-bearing assumption
named in advance.

- **`crates/eigenius-reasoning/src/project.rs` becomes `kernel/src/justification/`**, a sibling of
  `kernel/src/witness/`, mirroring the namespace split P1.3 makes. The two modules answer different
  questions and stay apart: `witness/` answers *does the chain admit this ground* (keys, hashes,
  α-canonicalisation, and after P7 the `Is*As` types); `justification/` answers *what does this term
  rest on* (`support`, `is_fully_verified`, `leaves_of`, `survives_without`, `cited_iris`, `Ground`,
  `Leaf`). `nbe/` is the wrong home — this is a reading of one particular inductive, not type theory.
- **Move it once, after P4.** P4 takes `Ground` from four variants to three; relocating before that
  means editing the module in its new home immediately after moving it.
- **The `do_project_justification` dispatch wrapper does not move** — P7 deletes it with the rest of
  the institution. What moves is the algebra and its five readings.
- `crates/eigenius-reasoning` keeps `tests/projection.rs` or the tests move with the module; either
  way P3's and P4's exit gates are stated in this module's vocabulary (`is_fully_verified`,
  `leaves_of`, `survives_without`) and must keep running across the move.

**The mentions graph is a superset of the premise graph, and P6 must still filter.** A resource's
`justification:proposition` and `justification:term` are both `core:inductive`, so both contribute
edges from the same subject: the proposition's references to classes sit in the same edge set as the
justification's citations of premises. Cycle detection over raw `core:mentions` would report cycles
that are not justification cycles. **The check restricts to edges originating in the
justification-bearing slots** — which the index does not currently distinguish, since a
`core:mentions` triple records subject and object and not the predicate the term came from.

**P4 closes this choice.** Two options stood here — carry the originating predicate on the projected
triple, or re-read and decode each candidate's justification slot — with *measure before choosing*.
They no longer measure the same thing, because `SpecStr`'s removal changes what a justification slot
contains.

Today the tag is a free `core:string` written as a `urn:`-prefixed value in practice —
`SpecStr(DeclaredEvidence(rule_strong), "urn:eigenius:demo:screen:EIG_0291")` in
`crates/eigenius-reasoning/tests/fixtures/universal_rule.esl:107`. `json_mentions` matches any
`urn:`-prefixed string at any depth, so **the tag becomes a mention edge inside the justification
slot and is not a premise citation.** A predicate-on-the-triple filter cannot exclude it: the tag and
the premise IRIs share a slot, so slot identity does not separate them. Only decoding does.

After P4 removes `SpecStr`, `justification:Term` is `{Declared(iri), Observed(iri), Verified(iri),
App, Sum}` and every string argument is a premise IRI consumed by a witness. **Every `urn:` string in
the justification slot is then a premise citation**, and the predicate-on-the-triple filter is exactly
correct — the cheaper option, and the one that serves future consumers needing to know why a mention
exists. Take it, and record the invariant it depends on: *no constructor of `justification:Term`
carries a string that is not a premise IRI.* A future constructor that broke that would silently
break this check.

The failure direction matters for the ordering. A spurious edge produces a **false rejection** of a
well-founded commit, which loses data; so P6 must not land before P4.

The condition is vacuous on `Declared` premises, which have no support to inspect. That carve-out is
required, not convenient: constant specifications may be self-referential, and self-referentiality is
unavoidable for realising some S4 theorems in LP.

**Exit:** a test constructing the two-layer retroactive-upgrade cycle from the paper and asserting the
commit is rejected; and a test asserting that a claim whose proposition and premise reference the same
class is **not** rejected.

---

## P7 — Relocate what the kernel owns; state the boundary operationally

**Bootstrap edit → reseed.** Depends on P3 and P4.

**The kernel owns what it must construct; the chain owns what the kernel only has to check.** That
line places the witness types inside the kernel and leaves the certificate vocabulary outside it.

- **`witness:Is*As` moves to kernel base vocabulary.** `synthesize_chain_witness` produces inhabitants
  of those otherwise-empty types — by constant specification for attributions, and from a committed
  judgement for `Verified` after P3. A type the kernel inhabits cannot be owned by a layer above it.
  This also replaces `check_hooks.rs`'s recognition of witness positions by four hardcoded short
  names, which admits any inductive anywhere carrying one of those names, with IRI resolution.
- **`justification:Certificate` and `justification:Term` stay chain-declared.** The kernel verifies a constructor
  application against its declared type; one argument's type is a witness type it recognises, and it
  needs no knowledge that the constructor belongs to `Certificate`. Chain-declared inductives exist
  so the kernel can check terms of types it does not carry, and this is the case they were built for.
  Keeping the algebra in a layer also keeps the system's position in the J / J4 / JT family an
  ontology edit rather than a kernel change.

  **The kernel knows the algebra but does not own the type.** This reads as a contradiction and is
  not one. After P6.0 the kernel knows how to *read* a term of this shape — which constructors are
  conjunctive, which disjunctive, what a leaf is — because P6 cannot be correct without it. It still
  does not own the *declaration*: certificates are type-checked generically against whatever the layer
  declares, so adding a constructor or changing the family remains an ontology edit. What the kernel
  gains is a reading; what the chain keeps is the vocabulary. The two are separable precisely because
  the reading is total on any term built from the constructors it recognises and errs (`ProjectError`)
  on anything else, rather than silently mis-reading it.
- **`extract.rs` no longer needs to move — P2 deleted the reason.** *Updated `2026-08-29`,
  after the sentence collapse landed.* The bullet below is the original plan and is retained
  because its reasoning is still the record of why the move was scheduled; what changed is its
  premise. The lift it turns on — *"nothing else performs that lift"* — **is now performed by
  nothing at all.**

  A `justification:Term` used to sit in its own slot as a plain D32 §3.7 tagged dict, a shape no
  codec read, so the file carried a bespoke decoder for it: `chain_value_to_exp` plus its error
  type, argument walker and diagnostics. After the collapse the term rides inside the judgement,
  which is an `eigentt:Term`-ranged value, so the D47 codec decodes it and `justification_exp`
  projects an `Exp` straight out. **One encoding where there were two.**

  Measured: `extract.rs` went from **1035 lines to 147**, with 888 removed as dead — 330 lines of
  decoder and ~550 of tests that existed only to exercise it. The two public functions survive.
  P7's disposition for this file is therefore a deletion with its two functions rehomed, not a
  1025-line relocation, and the crate's 2369-line total resolves accordingly.

- **[superseded] `extract.rs` moves to the kernel with the check it serves.** Its own module doc explains why it
  sits outside today: *"the Reasoning institution is different because its 'runtime' is the kernel's
  NbE checker — there's no external worker to reify into, and the validate handler needs a `Val`."*
  That rationale inverts here. Once `ValidateJustification` is absorbed into P2's uniform validation,
  the kernel is the party that needs the `Val`, so the D32 §3.7 tagged-dict → `Val::InductiveVal`
  lift goes with it. It cannot simply be deleted — nothing else performs that lift. At **1025 lines**
  it is the largest file in the crate, and an earlier draft of this plan did not place it anywhere.
- **The support algebra is already in the kernel** — P6.0 moved it to `kernel/src/justification/`
  because P6's check is unsound over reference edges. It stays a query surface over retained terms;
  P4 changed its ground enumeration and nothing else.
- **`ValidateJustification` stops being a dispatched AutoOnLoad query**, absorbed by P2's uniform
  check-mode validation: checking a certificate is type checking, which the kernel does not delegate.
  D81 recorded that `dispatch_auto_on_load_for_layer` has one call site and no test; write that test
  against whatever the check becomes, once.
- **The reasoning institution dissolves with it.** Removing `ValidateJustification` removes the only
  thing the institution did at commit, and what remains does not constitute one.

  **The paper does not have a reasoning institution.** It names two — *"a verification institution for
  Lean 4 … and a statistics institution"* (§Participating logics) — and describes justification logic
  as chain vocabulary, not as a participating logic. It says so zero times elsewhere. The guide states
  the reason plainly: *"The validator is the kernel. No bundled external checker."* An institution
  hosts a logic the kernel cannot evaluate; this one dispatches the kernel to itself.

  | resource | disposition |
  |---|---|
  | `reasoning:reasoning_institution` | delete — no hosted logic |
  | `reasoning:ef_justification` | delete — its only caller is `validate.rs:83`, which retires here; no comorphism declares it |
  | `reasoning:qc_validate_justification` | delete — absorbed above |
  | `reasoning:qc_consistency_check` + `consistency.rs` | delete — returns `Undecidable` for every non-empty input; its only mention outside its own tests is `demo/prose-to-formulas/README.md` explaining that it does not work. A reserved IRI for an unbuilt decision procedure is the follow-up-issue pattern the project's posture rejects |
  | `reasoning:qc_entailment_query` + `entailment.rs` | delete — a real 113-line lookup, not a stub, but the question it answers (*has a sentence claiming `P` been committed?*) is a witness-index lookup once P5 makes the index the answer |
  | `reasoning:qc_project_justification` | delete **the QueryClass and its dispatch wrapper**; `project.rs`'s algebra stays per the bullet above. The QueryClass has zero callers and the ontology already states it is not an institutional act — *"returns a JustificationProjection, not a Verdict: this reports what the term rests on, it does not judge it"* |
  | `urn:eigenius:reasoning:proc:*` | delete — four opaque handles with no handlers left |

  With `project.rs` gone at P6.0 and `extract.rs` gone here, **`crates/eigenius-reasoning` has nothing
  left**: `institution.rs`, `startup.rs`, `entailment.rs`, `consistency.rs` and `validate.rs` are all
  deleted by this phase. The crate is removed, not renamed. Its 2369 lines resolve as 1630 to the
  kernel and 739 deleted. **The seed pins these IRIs**: `kernel/src/esl/compile.rs:6223-6244` asserts
  `reasoning.esl` contains the ExportFormat and the QueryClasses, and `bootstrap/mod.rs:1348-1350`
  lists them. Both must land in this edit or the reseed fails.
- **State the protocol operationally.** The question is not whether a participating logic satisfies
  the definition of an institution, but whether the system can hold and re-check a witness for the
  claims it establishes. A logic supplies vocabulary, a decision procedure yielding a verdict,
  derivation resources, and optionally a judgement. It does not assign a warrant, define a witness
  kind, or establish `Verified`. Admitting a new hosted checker requires the two arguments the paper
  names: soundness of its `⊢` against its `⊨`, and satisfaction-preservation by its comorphism.

**The load-bearing assumption was discharged, and it failed.** Earlier revisions of this phase
assumed P6 could enforce well-foundedness over `core:mentions` edges, detecting cycles without
distinguishing `App` from `Sum`, and noted that if a case arose requiring the term algebra's
semantics the division above would move. P6.0 is that case: reference edges reject a `Sum` whose
other branch is acyclic, which is a false rejection. The division moved — the kernel holds the
algebra, the chain holds the declaration — and the boundary is now the one stated two bullets above.

**Exit:** `Verified` is reachable only through a checked judgement; the kernel owns every type it
inhabits; and hosting a checker is documented as adding both obligations, and the checker's
implementation, to the trusted computing base.

---

## Open after P7 — the role of the witness

**Deferred deliberately.** Three phases each remove part of what a witness was for, and the residue
should be reviewed once, at the end, rather than re-argued in each phase.

What the phases leave:

| phase | effect on witnesses |
|---|---|
| P3 | `Verified` comes from a checked judgement. The witness index stops being how `Verified` is established |
| P4 | three families, not four — `IsDerivedAs` is gone, and with it the lookup-time coercion |
| P5 | `Declared` and `Observed` become constant specifications over provenance relations — `declared_by` is already resource-typed and `source` is retyped to match |
| P6.0 / P7 | the `Is*As` types move into the kernel, alongside a support algebra that reads terms directly |

**The question the review has to answer.** A predicate the kernel inhabits by constant specification,
computed from a relation it can read at any time, is a decision procedure rather than a witness. If
all three surviving families are that, the index is a cache over relations — rebuildable, droppable,
and not a soundness boundary. D49 called the witness machinery *"the soundness boundary for the
Reasoning institution — every grounding fact entering the type system passes through these
witnesses"*; after P3 the institution is gone and the one family that carried a genuine admission
decision is served by a judgement instead.

Three things are known to survive regardless and should not be swept up in the review:
`hash_proposition_exp` and `alpha_canonicalize_proposition_json` (proposition identity, needed by
anything that compares propositions), the α/δ agreement between the emit and check sides that
`emit_and_check_sides_agree_on_the_hash` pins, and the diagnostic surface — a lookup miss naming the
family, the IRI and the property is the system's most-used error message.

**Do not act on this during P0–P7.** It is a question about the shape that P7 leaves, and answering
it early would fix the answer against a tree that is still changing.

---

## Removal inventory

Every declaration this plan deletes, verified present in the tree `2026-08-28`. **Names here are
pre-P1.3** — this section says where to find each declaration today, not what it is called after the
rename. Renames and
reworks are listed separately below, because they are not deletions and must not be treated as such.

### Rust — removed

| declaration | file | phase |
|---|---|---|
| `enum Grade` (4 variants) | `crates/eigenius-encoding/src/grade.rs:60` | P5 |
| `enum Warrant` (2 variants) and `Warrant::grade()` | `eigenius-encoding/src/grade.rs:74`, `:86` | P5 |
| `WitnessCategory::Derived` variant | `kernel/src/witness/mod.rs:47` | P4 |
| `Ground::Derived` variant | `crates/eigenius-reasoning/src/project.rs:71` | P4 |
| `PROPOSITION_SLOTS` | `kernel/src/ontology/well_known.rs:544` | P2 |
| `DECLARED_RESOURCE`, `OBSERVED_RESOURCE`, `DERIVED_RESOURCE`, `VERIFIED_RESOURCE` | `well_known.rs:449-452` | P5 |
| `emit_from_institution_derivation` | `kernel/src/layer/witness_index.rs:279` | P4 |
| the coercion branch of `check_layer_with_coercion` | `witness_index.rs:444` | P4 |
| `chain_witness_category_for_short_name` | `kernel/src/program/check_hooks.rs:93` | P7 |
| `impl Institution for ReasoningInstitution` | `crates/eigenius-reasoning/src/institution.rs:131` | P7 |
| `entailment.rs` (113 lines), `consistency.rs` (79 lines) | `crates/eigenius-reasoning/src/` | P7 |
| `do_project_justification` — the dispatch wrapper only | `crates/eigenius-reasoning/src/project.rs:271` | P7 |
| the `reasoning.esl` seed assertions | `kernel/src/esl/compile.rs:6223-6244`, `bootstrap/mod.rs:1348-1350` | P7 |
| Rule 21's two exemption branches | `kernel/src/validation/rules/eigentt_value.rs` | P2 |

### Rust — reworked, not removed

- `trace_category` (`witness_index.rs:186`) — five arms to two; P5 may retire it in favour of reading
  the provenance shape.
- `emit_from_reasoning_sentence` (`witness_index.rs:262`) — keyed to the checked judgement instead of
  `is_a` membership. P3.
- `verification_trace` (`crates/eigenius-reasoning/src/validate.rs:186`) — `proof_term` names a proof
  of `P` rather than the sentence's own IRI. P3.
- Rule 23 and Rule 24 — absorbed into P2's uniform rule rather than deleted outright; confirm no
  behaviour is lost before removing either file.

### Ontology — reworked, not removed

- `JustifiedBy.spec_poly` (`reasoning.esl:204`) — the rule survives; its result index drops `SpecStr`
  and becomes `JustifiedBy(j, P(x))`. P4.
- `JustifiedBy.sum_l` / `sum_r` (`reasoning.esl:143`, `:151`) — each gains a second premise so both
  branches must be justified. P4.

### Ontology — removed

| declaration | file | phase |
|---|---|---|
| `reflection:DeclaredResource`, `ObservedResource`, `DerivedResource`, `VerifiedResource` | `reflection-ontology.json` | P5 |
| `reflection:epistemic_status` | `reflection-ontology.json` | P5 |
| `reflection:epistemic:{declared,observed,derived,verified}` — the four `allows_only` individuals | `reflection-ontology.json` | P5 |
| `reflection:EpistemicStatus` — the class the four inhabit | `reflection-ontology.json:123` | P5 |
| `objective:acceptance_grade`, `objective:axiom_kind` | `objective-ontology.esl:163`, `:194` | P5 |
| `reflection:ExternalExecutionTrace` | `reflection-ontology.json` | P5 |
| `witness:IsDerivedAs` | `ontologies/justification/justification.esl:60` | P4 |
| `reasoning:reasoning_institution`, `ef_justification`, the four QueryClasses, `proc:*` | `reasoning.esl:323-393, 487` | P7 |
| `JustificationTerm.DerivedEvidence` constructor | `reasoning.esl:76` | P4 |
| `JustificationTerm.SpecStr` constructor | `reasoning.esl:90` | P4 |
| `JustifiedBy.derived` constructor | `reasoning.esl:122` | P4 |
| `VerifiedResource subclass_of DerivedResource` | `reflection-ontology.json` | P5 |

### Ontology — retyped

**Ten classes currently subclass a grade class and must be retyped when the grade classes go.**
This is the dependency P5 must handle first, and it spans six ontologies:

| class | currently subclasses |
|---|---|
| `reference:Citation` | `DeclaredResource` |
| `reasoning:ReasoningSentence` | `DerivedResource` |
| `reasoning:VerifiedPropositionView` | `DerivedResource` |
| `reasoning:EntailmentRequest` | `DerivedResource` |
| `reasoning:ConsistencyRequest` | `DerivedResource` |
| `stats:SampleSetResource` | `ObservedResource` |
| `enc:EncodedClaim` | `DeclaredResource` |
| `enc:ReasoningStructure` | `DerivedResource` |
| `ingest:PinnedExternalFile` | `ObservedResource` |
| `reflection:InstitutionEmittedDerivation` | `DerivedResource` |

**Two properties, not classes, also point at the grade vocabulary** and are easy to miss because they
live in a different ontology: `objective:acceptance_grade` and `objective:axiom_kind`, both
`class_types reflection:EpistemicStatus` with `allows_only` over the four individuals. See P5.

`reasoning:ReasoningSentence`'s subclassing carries a stated rationale — *"subclassing DerivedResource
lets prior sentences be cited via DerivedEvidence; the inherited derivation requirement is satisfied
by the certificate field"*. Both halves are void: `DerivedResource` requires nothing, so there is no
inherited requirement, and `DerivedEvidence` is deleted in P4.

### Renamed, not removed

- `eigentt:TypeExpr` → `eigentt:Term` (P1) — 51 ontology sites, 21 Rust files. The 20 constructors
  are unchanged.
- `urn:eigenius:reasoning` → `urn:eigenius:justification` for the calculus, the carrier and the
  report (P1.3) — ~650 occurrences in 103 files. The institution resources keep the old prefix until
  P7 deletes them, after which `urn:eigenius:reasoning` names nothing.
- `urn:eigenius:reasoning:ChainWitness` → its P7 destination in kernel base vocabulary (16 sites).
  Moved once, at P1, not twice.

---

## Two things to settle before P4

Neither is a phase; both are decisions that go stale badly if left until the phases have run. Both
are sized in the companion scope note.

1. **The ACP specification** (`docs/spec/ai-computed-provenance-1.0.md`, 1563 lines, an editor's draft
   for a proposed W3C community group). **53 of its 128 normative assertions** sit in the sections
   these phases rewrite, and four are contradicted outright — `ACP-5-1` (four grades MUST be
   supported), `ACP-5-2` (`Verified` MUST specialize `Derived`), `ACP-A-22` (the grades MUST be the
   four classes), `ACP-A-31` (the certificate MUST be `JustifiedBy` with four grounding constructors).
   Editing in place silently changes what a conformance claim means; a `1.1` superseding a retained
   `1.0` is the option that leaves an external reader able to tell which document a claim was made
   against.
2. **The agent skills** under `docs/method/`. `reasoning.md`, `eigenius.md` and `grounding.md` are
   executable methodology with `name`/`description`/TRIGGER frontmatter, not documentation.
   `reasoning.md` §"The epistemic contract" is a four-row table instructing an agent to author
   `DeclaredResource` + `DeclarationTrace` + `DerivedEvidence` chains. Until they are updated they
   keep producing the shape P4 and P5 delete.

---

## Verification, every phase

- `cargo test --workspace`; `cargo fmt --all -- --check`; `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets`.
- The parse gate, **`--release`** — a debug build fakes grammar gaps through NbE stack overflow.
  Baseline from P0; grammar-gap 0 and the hit count must not regress.

  **The tracked ranks are STALE against current importers, and that is a pre-existing condition
  this refactor surfaced rather than caused.** Measured `2026-08-29` on the first post-P2 reseed:
  a replay of `ranks/2026-07-29-demonstratives.json` reported **61 hits, 1 miss**, and the harness
  voids its own run on that — *"each falls back to seed order, disabling sense elimination for that
  sentence, so this is NOT a faithful replay and its per-unit numbers are not comparable."*
  Downstream, `expected-hits` read 60 against the baseline's 62.

  **Not attributable to P1-P3.** The whole of P1+P2 changes exactly one line under
  `kernel/src/dcg/` — a doc comment renaming `eigentt:TypeExpr` to `eigentt:Term` in
  `lexicon.rs`. The rank keys are sentence text, word surfaces and `wn:` / `umls:` sense IDs; the
  refactor touches none of them.

  **The cause is an axis the reseed script does not expose.** A rank key embeds the candidate
  sense list, which the IMPORTERS produce. The ranks were recorded `2026-07-29` and their snapshot
  built `2026-08-02`; `dcg/augment.rs` and `crates/eigenius-umls` changed `2026-08-20` (D71) and
  `dcg/glossary.rs` changed `2026-08-26` (#229). Any reseed at HEAD reproduces the mismatch. The
  baseline's own provenance note records this happening once before, when the D70 reseed changed
  candidate sense lists and one key stopped matching.

  **Matching baseline provenance therefore has one more axis than the flags cover.** UMLS scope,
  drops, atom-overrides and countability are all selectable; the importer code that built the
  lexicon is not. A faithful gate needs the ranks re-recorded whenever the importers move —
  which is a live-LLM cost and a fresh draw, so it is a deliberate act rather than a step in a
  phase.

  **Coverage still gates, and it held**: `grammar-gap 0`, `missing-lexeme 0`, 62 units. That
  criterion is the non-negotiable one and needs no rank replay to read.

  **Decision (`2026-08-29`): P1-P3 are verified on COVERAGE ONLY.** The faithfulness number is
  unreadable until the ranks are re-recorded, and re-recording is a live-LLM draw that buys nothing
  for phases which provably do not touch the parse path. It becomes necessary the first time a
  phase does.
- The WRN demo end to end. Take the stack down first; staging removes the store directory under a
  live RocksDB otherwise.
- After any bootstrap edit: reseed, and check the resource count against P0.
- The three gates this plan writes as failing tests first: P3's `Declared`-cited-as-`verified`
  (`is_fully_verified` must return false), P4's ungroundable `Sum` branch (must be rejected at
  commit), and P6's two-layer retroactive-upgrade cycle (must be rejected) alongside its
  same-class-mention case (must **not** be).

## Risks

| risk | phase | mitigation |
|---|---|---|
| the lexicon does not survive check mode | P2 | P0 measures it offline first; if the failure rate is material, P2 splits into a repair pass and a rule change |
| a genuine structural reader of a grade class exists | P5 | P0 classifies all 26 files before any deletion |
| reseed count grows | P1, P2, P5, P7 | batch bootstrap edits within a phase; P4 and P5 may share one |
| the projection algebra changes under consumers | P4 | `project.rs` itself is correct and stays; only the `Ground` enumeration and what institutions emit change |
| one-step cycle checking proves insufficient | P6 | the paper already concludes it is; P6 implements transitive closure over `core:mentions` from the start |
| P6 lands before P4 and rejects well-founded commits | P6 | a `SpecStr` tag is a non-premise `urn:` string in the justification slot; ordering is a hard dependency, not a preference |
| the ACP spec's conformance meaning changes silently | before P4 | 53 of its 128 normative assertions are in scope and four are contradicted outright; decide version-vs-revise before the phases run, not after |
| agents keep authoring the deleted shape | before P4 | `docs/method/{reasoning,eigenius,grounding}.md` are executable skills, not prose; a stale skill writes data the new tree rejects |
| the objective ontology's grade properties are missed | P5 | they are properties in a different ontology, not classes; now in the removal inventory and the retype table |
| counts in this document are quoted without their method | all | §0 records the method and lists the three counts that were wrong in the first draft |
