# Analysis plan — the epistemic stack: justification logic, witnesses, traces, grades

**Status: plan.** Produces D81, an organized description of the current architecture with code
references. Written `2026-08-26`.

**Question.** How do the reasoning institution's justification logic, the witness machinery, the
trace classes, and the four epistemic resource classes actually interact — and where are the
boundaries between them unclear, overlapping, or unowned?

**Hypothesis to test, not assume.** That parts of this stack have unclear boundaries, overlapping
abstractions, and underspecified responsibilities. The analysis must be able to come back with
*"these four things are distinct for a reason and the reason is X"* as readily as with a merge
proposal. Reconnaissance below already found candidate evidence on both sides.

---

## 1. What the deliverable is

**D81 — a description, not a proposal.** Four sections:

1. **The concept inventory.** Every name the stack uses for an epistemic or evidential notion, what
   it denotes, where it is defined, and who may create or read it.
2. **The lifecycle.** For each of the four grades, the complete path from *something happens* to *a
   resource carries that grade and a certificate cites it*, as a sequence of named functions.
3. **The boundary map.** For each seam — kernel/institution, kernel/validator, institution/chain,
   compiler/kernel — what crosses it, in which direction, and what the crossing is allowed to
   assume.
4. **Findings.** Where the above is genuinely redundant, genuinely ambiguous, or genuinely fine.
   Each finding carries the evidence that makes it a finding.

A cleanup proposal is **out of scope** for D81 and belongs to a successor. Mixing them is what
makes an analysis argue for its conclusion instead of reporting.

## 2. Method

**Read the code, then the design docs — in that order.** The design corpus for this area is large
(D6b, D14, D31, D39, D46, D49, D52, D54, D73) and partly aspirational. Reading it first would seed
the description with intentions rather than behaviour. Read it second, as a *diff*: where the docs
and the code disagree, that disagreement is itself a finding.

**Every claim carries a `file:line`.** A description without references cannot be checked and will
rot the way the README did.

**Distinguish three failure modes, because they want different fixes.**

| kind | signature | fix shape |
|---|---|---|
| **redundant** | two names, one denotation, no path where they differ | merge |
| **ambiguous** | one name, two denotations | split and rename |
| **unowned** | a decision made in N places with no authority | give it a home |

## 3. Starting evidence from reconnaissance

These are inputs, not conclusions. Each is a question the analysis must answer.

### 3.1 The four-way distinction is encoded five times

| # | encoding | where |
|---|---|---|
| 1 | `reflection:{Declared,Observed,Derived,Verified}Resource` | `ontologies/reflection/` |
| 2 | `reflection:{Declaration,Observation,Program,Verification,ExternalExecution}Trace` — **five** classes onto four grades | `kernel/src/layer/witness_index.rs:179` (`trace_category`) |
| 3 | `WitnessCategory::{Declared,Observed,Derived,Verified}` | `kernel/src/witness/mod.rs` |
| 4 | `witness:Is{Declared,Observed,Derived,Verified}As` | `ontologies/reasoning/reasoning.esl` |
| 5 | `JustifiedBy.{declared,observed,derived,verified}` | same |

**To answer:** are these five projections of one concept, or do they differ somewhere? Specifically
— can a resource be `DerivedResource` while the only admissible witness is `IsDeclaredAs`? Does
`trace_category`'s 5→4 collapse (`ExternalExecutionTrace → Declared`, eigenius#205) lose anything a
consumer needs? Is there a lawful mapping, and if so is it written down anywhere or re-derived at
each site?

### 3.2 Epistemic status is assigned in at least eight non-test places

`crates/eigenius-obograph/src/convert.rs`, `crates/eigenius-schemaorg/src/{convert,report}.rs`,
`crates/eigenius-reasoning/src/grade.rs`, `crates/runtime-substrate/src/facade.rs`,
`kernel/src/bootstrap/mod.rs`, `kernel/src/esl/compile.rs`,
`kernel/src/institution/dispatch.rs`, `kernel/src/layer/index.rs`.

`ClaimGrader` (`crates/eigenius-reasoning/src/grade.rs:197`) looks like the intended authority, but
most of those sites do not go through it.

**To answer:** is `ClaimGrader` the intended single authority, an authority for one route only, or a
convenience? What stops an importer from stamping `VerifiedResource` on anything? Is the
`is_a`-based grade enforced anywhere, or is it descriptive?

### 3.3 "Witness" names at least two unrelated things

- **`ChainWitness`** — evidence for a `JustifiedBy` certificate, synthesized by the kernel type
  checker (`kernel/src/nbe/check/witness.rs` → `EffectHooks::synthesize_chain_witness` →
  `kernel/src/layer/witness_index.rs`).
- **Merge `Witness` resolution** — a D20 §6.1 merge *strategy* taking a witness **function**
  (`kernel/src/layer/merge/witnessed.rs`, `MergeError::WitnessTypeMismatch`).

**To answer:** confirm these are unrelated, and if so whether the collision is costing anything
beyond grep noise. This is the cheapest finding to confirm or dismiss and should be done first, so
it stops contaminating searches.

### 3.4 The institution abstraction has a dead branch

`ontologies/institution/` declares `runtimes:{in_process, external, wasm}`. WASM was removed in
eigenius#101; no kernel code implements it.

**To answer:** is `runtimes:wasm` reachable? What else in the institution surface is declared but
unbacked? This bears directly on "what is an institution, really" — the answer differs sharply
between an in-process institution whose validator *is* the kernel (`reasoning:reasoning_institution`
declares `runtime = in_process`, *"the validator is the kernel"*) and an external one behind a
container.

### 3.5 The reasoning institution's relationship to the kernel is unusual

D80 §2.1 already established, for the witness case: the institution owns the **vocabulary** and the
**trigger** (`ValidateJustification`, AutoOnLoad), the kernel owns **synthesis** and **checking**,
and nothing is persisted. That is a specific and defensible split.

**To answer:** does it generalise? Is the statistics institution (in-process, `ndarray`/`statrs`,
`crates/eigenius-statistics/`) split the same way, or differently? Is "in-process institution" one
pattern or several wearing one label?

## 4. Phases

Each phase produces a section of D81 and can be stopped at.

- **P0 — the name census.** Every type, class, enum variant and property in the stack, with its
  definition site and its readers. Mechanical; the input to everything else. Settles §3.3
  immediately. **Output:** D81 §1.
- **P1 — four lifecycles.** For each grade, trace the concrete path end to end against a real
  example already in the tree: a Declared claim from the WRN demo, an Observed one, an
  `InstitutionEmittedDerivation` from statistics, and a Lean `Verdict`. Name every function.
  **Output:** D81 §2. **Gate:** each lifecycle is walked against a test or a demo artifact that
  actually exists — no path is described from the design docs alone.
- **P2 — the boundary map.** kernel ↔ institution (both runtimes), kernel ↔ validator (Rules 16,
  21, 22, and the AutoOnLoad phase at `kernel/src/commit/phases.rs:411`), compiler ↔ kernel (what
  `kernel/src/esl/compile.rs` stamps and why), chain ↔ everything (what is persisted vs derived).
  **Output:** D81 §3.
- **P3 — the design-doc diff.** Read D6b, D14, D31, D39, D46, D49, D52, D54, D73 against P0–P2.
  Record every disagreement. **Output:** feeds D81 §4.
- **P4 — findings.** Classify each candidate as redundant / ambiguous / unowned / fine, with
  evidence. **Output:** D81 §4.

## 5. What would make this analysis fail

Named up front, because each is a way to produce a document that reads well and helps nobody.

- **Describing the design instead of the code.** Guarded by the read-order rule and by P1's gate.
- **Finding overlap because the hypothesis asked for it.** Guarded by requiring a "genuinely fine"
  verdict to be as reportable as a merge proposal, and by requiring evidence per finding.
- **Stopping at the vocabulary.** Five names for one concept is a finding only if the analysis also
  shows what it costs — a bug, a re-derivation, a place where two sites disagree. §3.1 is not a
  finding yet.
- **Sliding into a proposal.** D81 describes. The successor decides.

## 6. Sequencing

P0 first and alone — it is mechanical and it settles §3.3, which otherwise pollutes every search.
P1 and P2 can interleave. P3 last of the reading phases, deliberately. P4 only after P3.

Not gated on any code change, and produces none.
