---
name: reasoning
description: The standing method for substantive science/engineering tasks — capture reasoning live as a typed Eigenius chain so every load-bearing claim is a graded, witnessed proposition the kernel gate accepts or rejects. Covers stating the thesis, anchoring new territory in real cited sources, planning as typed warrants, executing with evidence-as-you-go, failing closed on unsupported claims, and composing to the thesis. TRIGGER at the start of any non-trivial task that involves a claim worth defending — analysis, reproduction, design decisions, debugging conclusions, research. Complements the `eigenius` skill (which drives the platform mechanics). Requires the kernel/orchestrator stack up.
---

# Reasoning protocol (typed reasoning, live on the chain)

The default way to do substantive work here: **don't assert — witness.** Every
load-bearing claim becomes a typed proposition with a graded warrant, committed to
an Eigenius chain whose commit gate *rejects* any conclusion that doesn't actually
follow from admitted evidence. The chain is the working memory of the reasoning,
not a write-up made afterward.

## Why (the failure modes this exists to prevent)

Unaided, reasoning fails in characteristic ways — all seen, repeatedly, in this
repo's own WRN work:

- **Unwitnessed assertion** — stating an empirical claim as established with
  nothing behind it ("γH2AX foci are redundant / invalid").
- **Conclude-before-check** — asserting a direction/result before running it
  (pATM "activates DDR" computed from data that showed the opposite).
- **Silent divergence** — quietly dropping or changing a planned step.
- **Unanchored leap** — reasoning in unfamiliar territory without pulling in the
  prior knowledge that fixes the ground.

The kernel makes each one a *hard stop* instead of a thing noticed five turns
later: a `ReasoningSentence` only commits if its certificate type-checks against an
**admitted witness**; a `Fails` verdict blocks the layer (AutoOnLoad gate). You
cannot record "X holds" without the thing that makes X hold.

## Prerequisites

- Stack up: `docker compose up -d` (kernel :50051 +
  orchestrator :8080). Mechanics (load/query/run, MCP tools, ESL/EigenQL) are in
  the **`eigenius`** skill — read it for the *how*; this skill is the *method*.
- One **objective branch** so the reasoning is isolated and inspectable:
  `eigenius branch create <obj-slug> --from <main-head>`; commit everything below
  onto it.

## The epistemic contract — grade every claim

Every claim lands as one of four grades; each grade has an admission cost the
kernel enforces. Never let a claim float ungraded.

| Grade | Means | What it requires on chain | Witness |
|---|---|---|---|
| **Observed** | recorded from reality | `reflection:ObservedResource` + provenance (a pinned source / content hash) + an `ObservationTrace` | `IsObservedAs` |
| **Declared** | asserted on authority/design | `reflection:DeclaredResource` + `reflection:rationale` + a `DeclarationTrace` | `IsDeclaredAs` |
| **Derived** | computed | a `DerivedResource` a program/institution emits, carrying `canonical_proposition`, under a `ProgramTrace` | `IsDerivedAs` |
| **Verified** | kernel-checked reasoning | a `ReasoningSentence` whose certificate type-checks → `Holds` | `IsVerifiedAs` |

Each of the first three witnesses is emitted by the per-layer witness index **from a
trace resource** (`ObservationTrace` / `DeclarationTrace` / `ProgramTrace`) whose
`reflection:resource` points at the target and whose target carries
`reflection:canonical_proposition` — so `observed(iri, P)` / `declared(iri, P)` /
`derived(iri, P)` only resolve when that trace exists in an **ancestor layer** of the
citing sentence (load emitters before consumers — the
`04a-evidence`-before-`04b-conclusions` / recompute-plans-before-conclusions split).

A bare opinion is, at most, a **Declared hypothesis** — and it must say so, with a
rationale. If you want it to count as fact, it must become Derived (run it) or
Verified (prove it). "I think the bug is X" is a Declared hypothesis until a
Derived witness (a reproduction, a test) discharges it.

**A `Holds` is only the *first* oracle.** A `Holds` proves the certificate
**type-checks against an admitted witness** — *structural/logical* validity (D61's
**oracle #1**). It does **not** prove the right grounding was *discovered*, nor that
an encoding is **faithful** to its source (D61's **oracle #2** — semantic
faithfulness, which the gate cannot give). Two encodings can both type-check while
only one is faithful (D57 #9: `core:domain` vs `core:recommends` — both well-formed,
only the latter true to the spec). So a `Holds` is necessary, not sufficient: still
verify intent/grounding. A *mechanized* faithfulness/grounding check is graded
**Derived** (a program scored it — the LLM-judge inflates, D61), **never
auto-Verified**; only a human spot-check or a proof-level correspondence reaches
**Verified**.

**Reach for the strongest grade the mechanics allow — don't settle for Declared.**
A claim you'd write as Declared often has a stronger *mechanical* witness available:
content-hash a file → **Observed**; run the producer as a program *through the kernel*
(the D60 `oci` tool runtime / D56 wrapped-program → `ProgramTrace → IsDerivedAs`) →
**Derived**; a load that validates (0 errors) or a query that returns the expected
result → **Verified-by-check**. Auditing your Declared claims for these upgrade paths
is the method of the D57 mechanical-evidence pass — and the act of producing the
witness routinely *catches a bug the assertion hid* (it found two real generator bugs).

## The loop

### 0. Frame ⇄ Ground — make the objective well-posed (the assessment phase)
Frame the task as an **obligation graph** (D58): a **thesis** proposition + the
**axioms** it may assume (Observed data / Declared rules / Cited anchors) + the
**milestones** to derive, each with an acceptance grade. You usually can't state the
axioms or milestones until you have grounded enough to *express* them — so framing
and grounding are **co-recursive**. Iterate until the graph is well-posed, then
execute.

Draft the graph as typed `objective:` resources, then check four admissibility
gates. Three are **enforced by the type system at commit** — a non-well-posed frame
simply won't load — and one is a query you run:
- **Expressible** — every proposition compiles (an undefined predicate ⇒ vocabulary
  gap → `grounding`: import/align/declare terms). *The frame loads = passes.*
- **Checkable** — every `objective:Milestone` carries `acceptance_grade` +
  `witness_kind` + `falsifier` (all `requires`d; values `allows_only`-constrained).
  A milestone with undefined acceptance won't commit.
- **Anchored (presence)** — every `objective:Axiom` carries a `witness` (required);
  an unanchored axiom won't commit. Its referential half (does the witness resolve?)
  is the query below.
- **Reachable** + **Anchored (referential)** — the two runtime checks: run
  [`experiments/objectives/well-posed-reachable.eigenql`](../../experiments/objectives/well-posed-reachable.eigenql)
  and [`well-posed-anchored.eigenql`](../../experiments/objectives/well-posed-anchored.eigenql)
  against the objective's branch. **Empty result = passes;** any row is a node
  disconnected from the thesis / an axiom with a dangling witness → `grounding`
  (cite/observe), decompose, or record **blocked**.

Loop frame⇄ground until all gates pass (`grounding`'s retrieve-first shrinks the
frontier each pass, so it converges). **Loop budget:** cap reframing at a few
rounds — if a gate still can't be closed after ~3 passes (no evidence, no path, no
grounding), stop and record the finding: the objective is ill-posed/blocked *here*;
don't proceed on faith. Full spec + the objective ontology: **D58**. (Distinct from
kernel D21 tasks / `bench:TaskOutput`, which are program-run execution — D58 §6.)

**The frame is a living artifact — re-enter it at every subgoal boundary (D58 §4.1).**
A frame seeded from a high-level goal cannot anticipate what execution teaches, so
`EXECUTE` is not a one-way exit: after each milestone lands, or whenever an insight
surfaces, **re-assess the frame and fold the learning back in** — *sharpen* a
downstream milestone now that you can express it precisely, *re-grade* it when a
stronger witness turns out mechanical (the grade-climbing below — do it as the
evidence lands, not as a cleanup pass), *decompose* it, or *reframe* the goal when a
learning shows the original cut was wrong. Do this **proactively**, as part of the
work — the anti-pattern (seen in D57) is executing against a stale high-level frame
and then *retrofitting* the formalization afterward under manual steering. Each such
change is a recorded, versioned frame revision (a structural diff, not silent prose).

Commit the thesis + milestones now (the goal posts) as typed `objective:` resources
in the objective's own namespace — the propositions they target as `Prop` decls,
wrapped in `objective:Milestone`/`Axiom` nodes (worked example:
`experiments/objectives/d57-schema-org/chain/`):
```esl
namespace obj = "urn:eigenius:obj:<slug>";
data obj:ThesisHolds : core:string -> Prop { }
// Each Prop is the TARGET of an objective:Milestone (acceptance_grade +
// witness_kind + falsifier + depends_on edges); it only Holds once its
// antecedents do. The thesis is the root Milestone.
```

### 1. Anchor — when entering new territory, fix the ground in real sources
Run the **`grounding`** skill: **retrieve-first** (ask the kernel what it already
knows via D43 `~` search — reuse existing conclusions/witnesses/vocabulary, don't
re-derive), then fill the gap with external research, then **map results back into
the kernel** as retrievable anchors and aligned standard vocabulary. Each
load-bearing external fact becomes a **CiTO-typed citation carrying the imported
claim** (the template below) — these are the admissible premises. **Sources must be
real and verified (DOIs/PMIDs that resolve); never fabricate a citation.** When a
standard vocabulary types the domain, adopt it (OBO / schema.org) rather than
reinventing terms — see `grounding`.

**Consult the authoritative documentation, not just the data — and cite it as you
decide, not when prompted.** A standard's machine artifact (JSON-LD, OWL) gives you
the *terms*; its prose spec (data-model / conformance docs) gives you the *semantics*
that govern how to map them — and every load-bearing design choice you make from that
spec is itself an anchor. The failure mode (seen in D57): the mapping was built from
schema.org's JSON-LD without reading its data-model doc, and the conformance fact the
key decision rested on (`domainIncludes` is advisory → `recommends`, not the
restrictive `core:domain`) was only cited as a `reference:Citation` once a human asked.
The discipline is **proactive**: before mapping a standard, read its spec; the moment a
decision turns on a documented fact, commit the citation carrying that fact in the same
step — the agent does this itself, it is not a thing the human should have to request.
```esl
namespace lit = "urn:eigenius:obj:<slug>:lit";
resource lit:smith_2020 : reference:Reference {
    reference:creator = "Smith J, et al."; reference:title = "...";
    reference:container_title = "Nature"; reference:issued_year = 2020;
    reference:doi = "10.1038/..."; reference:pmid = "12345678";
}
resource lit:cite_smith : reference:Citation {
    reference:cites          = lit:smith_2020;
    reference:citation_type  = reference:cites_as_authority;   // or uses_method_in / cites_as_evidence / ...
    reflection:canonical_proposition = type_expr( obj:KnownFact("x") );
    core:description = "what this work establishes that we build on";
}
resource lit:cite_smith_trace : reflection:DeclarationTrace {
    reflection:resource = lit:cite_smith; reflection:declared_by = "lit:smith-2020";
    reflection:timestamp = "<iso>";
}
```
An anchor is a *premise you are allowed to build on*. Everything else must be
derived, or declared with a rationale. Distinguish anchors (cited prior knowledge)
from your own claims — never let an assumption pass as established fact.

### 2. Plan — express the plan as a typed warrant graph
Author the intended `ReasoningSentence`s (the dependency graph), even as stubs, so
the plan lives on chain. Then any later deviation is a **structural diff** (plan
declares warrant W; chain lacks a resource discharging it), not a silent prose
change. Name what evidence kind will discharge each (Observed/Derived/Declared).

### 3. Execute — produce evidence, then the proposition, in that order
For each step: make the evidence first, commit the witness, then the sentence that
cites it. Three shapes:

- **Observed** — commit an `ObservedResource` (a pinned source, a content-hashed
  `ingest:PinnedExternalFile`) **plus an `ObservationTrace`** pointing at it
  (`reflection:resource = <iri>`); the trace is what makes the witness index emit
  `IsObservedAs`, so `observed(iri, P)` resolves. Without the trace the resource
  loads but cannot be cited.
- **Derived** — run a program / institution (see `eigenius` skill: `run`,
  `RunRuntimeScript`, the statistics institution); it emits a `DerivedResource`
  with `canonical_proposition` set **only when the computation supports it** (e.g.
  `if (direction & significance) set_proposition`), committed under a `ProgramTrace`.
  To get a Derived witness from *any* pinned tool (not just R/an institution), run it
  through the D60 generic `oci` tool runtime — `eigenius env build --language oci` +
  `eigenius run` — the WRN wrapped-program pattern, no new institution. Then:
```esl
resource obj:concl_x : reasoning:ReasoningSentence {
    reasoning:subject_iri   = "urn:eigenius:obj:<slug>:subject";
    reasoning:proposition   = type_expr( obj:Result("x") );
    reasoning:justification = DerivedEvidence("urn:eigenius:obj:<slug>:x:result");
    reasoning:certificate   = type_expr( derived("urn:eigenius:obj:<slug>:x:result", obj:Result("x")) );
}
```
- **Declared** rule/judgment — `DeclaredResource` + rationale + `DeclarationTrace`
  (the anchor shape, minus the citation), carrying the rule as
  `canonical_proposition`.

Match the measure to the claim. Reproducing a published result means matching the
*reported* number, not just the sign — a wrong measure that merely agrees in
direction is still a recorded **divergence** (a finding), not a pass.

### 4. Fail closed — a non-supporting result is a stop, not a drop
If the sentence Fails (witness doesn't carry the proposition; computation went the
wrong way; the gate rejects the layer) — **stop and investigate; record a
finding.** Do not quietly drop the step, weaken the claim, or route around it. A
`Fails` is the protocol working: it caught a wrong belief before it propagated.
Every divergence between expected and obtained is an explicit recorded finding
with a rationale (the `recompute-findings.md` discipline, generalized).

### 5. Compose — lemma-cite sub-results up to the thesis
Once sub-conclusions Hold, the capstone cites them as lemmas (D54): a Holds
`ReasoningSentence` is admitted as a Verified witness keyed on its IRI, so
`verified("...:concl_sub", obj:SubProp("x"))` discharges an antecedent. The
thesis Holds **only if every antecedent does** — the gate composes the warrant for
you. For the multi-antecedent modus-ponens spine, copy the worked pattern in
`experiments/publications/wrn-helicase/chain/08-phase3-invivo-mechanism.esl`
(`concl_mech`) and `09-phase5-synthesis.esl` (`concl_main`).

### 6. Audit — query the chain for integrity
Before declaring done, query: does every conclusion resolve to a witness? Is every
anchor a real, cited source? Are there dangling claims (no consumer) or ungraded
assertions? `eigenius_query` over `ReasoningSentence` / `Verdict` makes this
mechanical.

## Disciplines (the rules, each against a failure mode)

1. **No unwitnessed assertion.** Empirical claims are Derived or they are not
   asserted — at most a graded Declared hypothesis. (The kernel won't commit a
   sentence without an admitted witness.)
2. **Check before you conclude.** The witness *is* the check; produce it before
   the claim, never after.
3. **Checker-passing ≠ faithful — verify intent/grounding, not just type.** A
   `Holds` is oracle #1 (the certificate type-checks); it is not evidence the right
   grounding was *discovered* or that an encoding is faithful (oracle #2). A
   mechanized faithfulness/grounding check is **Derived**, never auto-Verified —
   only human spot-check or proof reaches Verified. (D61.)
4. **Fail closed.** A `Fails` / mismatch ⇒ investigate + record, never silently
   route around.
5. **Anchor new territory.** Don't reason unaided in unfamiliar ground; bring
   real, CiTO-cited prior knowledge first. Never fabricate a source.
6. **Ground in the documentation, cite as you decide — proactively.** When mapping a
   standard, read its prose spec (not just its data), and the moment a decision turns
   on a documented fact, commit the citation carrying it *in the same step*. Don't wait
   to be asked (D57: the schema.org data-model conformance fact was cited only on
   prompt).
7. **Refine the frame as you go; don't retrofit.** Re-enter the frame at each subgoal
   boundary — sharpen / re-grade / decompose / reframe as you learn (D58 §4.1) — rather
   than executing against a stale high-level frame and formalizing after the fact. Reach
   for the strongest grade the mechanics allow *as the evidence lands*, not in a cleanup
   pass.
8. **Plan on chain.** Deviations must be structural diffs, not prose drift.
9. **Same claim vs distinct evidence is inspectable.** Two witnesses for one
   `canonical_proposition` is corroboration; two propositions is distinct
   evidence. Don't call distinct evidence "redundant" — the types tell you which.
10. **Match measure to claim; reproduce the number, not just the sign.**

## Weight & escape hatch

This is the default for tasks with a claim worth defending. It is **not** for
trivial mechanical work (a rename, a formatting fix, a one-line lookup) — those
don't have a thesis. Minimum viable capture for an in-scope task: the **thesis**,
the **anchors**, and each **load-bearing conclusion** go on chain; purely
mechanical intermediate steps may stay inline. When in doubt, grade the claim: if
you'd be embarrassed to be wrong about it, it gets a witness.

## Going deeper

- `eigenius` skill — platform mechanics (load/query/run, MCP tools).
- Worked exemplars of every shape above: `experiments/publications/wrn-helicase/`
  (chain/ = the warrant graph; docs/03-recompute-findings.md = fail-closed
  findings; docs/02-dependency-graph.md = the four-grade graph; chain/02-literature.esl
  = CiTO anchors).
- Specs: D39 (justification logic / certificates), D49 (chain-witness index),
  D52 (statistics recompute), D54 (lemma citation), the `reference` ontology.
