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
later: a `justification:Conclusion` only commits if its judgement type-checks against an
**admitted witness**; a `Fails` verdict blocks the layer (AutoOnLoad gate). You
cannot record "X holds" without the thing that makes X hold.

## Prerequisites

- Stack up: `docker compose up -d` (kernel :50051 +
  orchestrator :8080). Mechanics (load/query/run, MCP tools, ESL/EigenQL) are in
  the **`eigenius`** skill — read it for the *how*; this skill is the *method*.
- One **objective branch** so the reasoning is isolated and inspectable:
  `eigenius branch create <obj-slug> --from <main-head>`; commit everything below
  onto it.

## The epistemic contract — two axes, three grounds

**Provenance and warrant are orthogonal.** Do not conflate them; the vocabulary
used to, and the conflation is what this method most often gets wrong.

| axis | question | applies to |
|---|---|---|
| provenance | how did this artifact come to exist? | **every** resource |
| warrant | what evidence exists for its proposition? | only resources carrying a proposition |

Most resources have provenance and no warrant. A lexicon entry, a class
declaration, an imported concept carries no proposition, so asking what proves it
is a **category error**, not an unanswered question. The test is mechanical: does
the resource carry a `reflection:canonical_proposition`.

**There are three grounds, not four.** A ground is what a certificate cites.

| Ground | Means | What it needs on chain | Witness |
|---|---|---|---|
| **Declared** | asserted on authority/design | a `justification:Claim` carrying the proposition + a `prov:DeclarationTrace` naming the agent | `IsDeclaredAs` |
| **Observed** | read off the world | the observed resource + a `prov:ObservationTrace` naming the `prov:Activity` that produced it | `IsObservedAs` |
| **Verified** | kernel-checked | a `justification:Conclusion` carrying a `justification:proof` — the judgement `holds(logic, t, P)` | `IsVerifiedAs` |

**`Computed` is not a fourth ground; it is a term shape.** A computed claim is
`App(Declared(plan), Observed(inputs))`: the plan is DECLARED to denote a function
of its input — which an accountable agent asserts and no execution can establish,
because determinism is a fact about the environment rather than something
recoverable from a run record — and the input is OBSERVED. `Sampled` is likewise
just a bare `Observed` leaf.

**A `prov:ProgramTrace` grounds NOTHING.** It records that a run happened. If you
want a computed claim to stand, commit the plan's reproducibility declaration and
the input's observation; the run record is provenance and cites nothing.

**Nothing stores a grade.** There is no `DeclaredResource` / `ObservedResource` /
`DerivedResource` / `VerifiedResource` class and no `epistemic_status` — a stored
grade let the thing being graded nominate its own grade. Warrant is COMPUTED from
the justification term (`kernel/src/justification/`: `support`,
`leaves_of`, `is_fully_verified`, `survives_without`), and it is a Rust-API answer,
not an EigenQL one. Provenance IS an EigenQL query — `prov:was_attributed_to`,
`prov:was_generated_by`, `prov:used`, `prov:had_primary_source` are all
resource-typed, so *which claims rest on this instrument* is a join.

Each witness is emitted by the per-layer witness index **from a trace resource**
whose `prov:resource` points at the target and whose target carries
`reflection:canonical_proposition` — so `declared(iri, P)` / `observed(iri, P)`
only resolve when that trace exists in an **ancestor layer** of the citing
conclusion (load emitters before consumers — the recompute-plans-before-conclusions
split).

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

**Reach for the strongest ground the mechanics allow — don't settle for a bare
Declared.** A claim you'd write as Declared often has a stronger *mechanical*
witness available: content-hash a file and give it a `prov:ObservationTrace` →
**Observed**; run the producer through the kernel (the D60 `oci` tool runtime / D56
wrapped-program) and declare the plan's reproducibility → the composite
`App(Declared(plan), Observed(input))`; a load that validates (0 errors) or a query
returning the expected result → a checked judgement. Auditing your Declared claims
for these upgrade paths is the method of the D57 mechanical-evidence pass — and
producing the witness routinely *catches a bug the assertion hid* (it found two
real generator bugs).

Note what running the program does NOT buy you: the run alone grounds nothing. The
upgrade is the plan DECLARATION plus the input OBSERVATION, and both need an
accountable agent behind them.

## The loop

### 0. Frame ⇄ Ground — state the thesis before deriving toward it
Say what you are trying to establish, what you may assume, and what would falsify
each step — before deriving. You usually cannot state the assumptions until you have
grounded enough to *express* them, so framing and grounding are co-recursive: iterate
until the thesis and its steps are expressible, then execute.

Two disciplines carry the weight, and neither needs machinery:

- **Every step names what would falsify it.** A step with no falsifier has no
  acceptance criterion, so it cannot fail and therefore cannot be evidence.
- **Re-enter the frame at every subgoal boundary.** Execution teaches things the
  original cut could not anticipate — sharpen a downstream step once you can express
  it precisely, decompose it, or reframe when a learning shows the cut was wrong. Do
  it as the work lands. The anti-pattern, seen in D57, is executing against a stale
  frame and retrofitting the formalization afterward.

If a step cannot be closed after a few passes — no evidence, no path — stop and
record that it is blocked. Do not proceed on faith.

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
resource lit:cite_smith_trace : prov:DeclarationTrace {
    prov:resource = lit:cite_smith; prov:was_attributed_to = "lit:smith-2020";
    prov:timestamp = "<iso>";
}
```
An anchor is a *premise you are allowed to build on*. Everything else must be
derived, or declared with a rationale. Distinguish anchors (cited prior knowledge)
from your own claims — never let an assumption pass as established fact.

### 2. Plan — express the plan as a typed warrant graph
Author the intended `justification:Conclusion`s (the dependency graph), even as stubs, so
the plan lives on chain. Then any later deviation is a **structural diff** (plan
declares warrant W; chain lacks a resource discharging it), not a silent prose
change. Name what evidence kind will discharge each (Observed/Derived/Declared).

### 3. Execute — produce evidence, then the proposition, in that order
For each step: make the evidence first, commit the witness, then the sentence that
cites it. Three shapes:

- **Observed** — commit the observed resource (a pinned source, a content-hashed
  `ingest:PinnedExternalFile`) **plus a `prov:ObservationTrace`** pointing at it
  (`prov:resource = <iri>`) and naming the `prov:Activity` that produced it
  (`prov:was_generated_by`). The trace is what makes the witness index emit
  `IsObservedAs`, so `observed(iri, P)` resolves. Without it the resource loads but
  cannot be cited. `was_generated_by` is resource-typed: name the instrument run or
  data release as an Activity resource, never as a string.
- **Computed** — run a program / institution (see `eigenius` skill: `run`,
  `RunRuntimeScript`, the statistics institution); it emits a result carrying
  `canonical_proposition` **only when the computation supports it** (e.g.
  `if (direction & significance) set_proposition`), under a `prov:ProgramTrace`. For
  *any* pinned tool, use the D60 generic `oci` runtime — `eigenius env build
  --language oci` + `eigenius run` — the WRN wrapped-program pattern, no new
  institution.

  **The run is not the ground.** Commit a `justification:Claim` asserting that the
  plan denotes a function of its input, with a `prov:DeclarationTrace` behind it,
  and cite the composite:
```esl
resource obj:plan_yields_result : justification:Claim {
    prov:was_attributed_to  = agent:<who-vouches>;
    prov:had_primary_source = obj:warrant_plan_reproducibility;
    prov:rationale = "Applying <plan> to its recorded input yields <result>. A claim about the method, pinned at the input it is applied to.";
    reflection:canonical_proposition = type_expr(
        core:Asserts("urn:eigenius:obj:<slug>:input") -> obj:Result("x")
    );
}
resource obj:plan_yields_result_trace : prov:DeclarationTrace {
    prov:resource          = obj:plan_yields_result;
    prov:was_attributed_to = agent:<who-vouches>;
    prov:timestamp         = "<iso8601>";
}

resource obj:concl_x : justification:Conclusion {
    justification:subject_iri = "urn:eigenius:obj:<slug>:subject";
    justification:judgement   = type_expr(
        holds( eigentt:logic_kernel,
               app( core:Asserts("urn:eigenius:obj:<slug>:input"), obj:Result("x"),
                    Declared("urn:eigenius:obj:<slug>:plan_yields_result"),
                    Observed("urn:eigenius:obj:<slug>:input"),
                    declared("urn:eigenius:obj:<slug>:plan_yields_result",
                             core:Asserts("urn:eigenius:obj:<slug>:input") -> obj:Result("x")),
                    observed("urn:eigenius:obj:<slug>:input",
                             core:Asserts("urn:eigenius:obj:<slug>:input")) ),
               justification:Certificate(
                   justification:App(
                       Declared("urn:eigenius:obj:<slug>:plan_yields_result"),
                       Observed("urn:eigenius:obj:<slug>:input")),
                   obj:Result("x") ) )
    );
}
```
**One slot, not three.** The proposition and the justification term are no longer
separate fields — they appear inside the judgement's TYPE, where the kernel checks
that the certificate actually inhabits `Certificate(j, P)`. Previously `proposition`,
`term` and `certificate` were three fields checked by three paths, with nothing
requiring them to be about the same claim; a certificate for one proposition sat
happily beside a different `proposition`. Now the pairing is what gets checked.

The judgement reads: *the kernel verified that this certificate grounds this
proposition*. It does **not** say the proposition is true — that is the point of the
separation, and no rule turns one into the other.
- **Declared** rule/judgment — a `justification:Claim` carrying the rule as
  `reflection:canonical_proposition`, with `prov:rationale`, a
  `prov:DeclarationTrace`, and `prov:was_attributed_to` naming who stands behind it.
  A declaration with no agent asserts nothing anybody can be held to. If the reason
  it was asserted is itself a resource — a criterion, a convention, a citation —
  name it with `prov:had_primary_source`, whose target is a `prov:Source`.

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
`justification:Conclusion` is admitted as a Verified witness keyed on its IRI, so
`verified("...:concl_sub", obj:SubProp("x"))` discharges an antecedent. The
thesis Holds **only if every antecedent does** — the gate composes the warrant for
you. For the multi-antecedent modus-ponens spine, copy the worked pattern in
`experiments/publications/wrn-helicase/chain/08-phase3-invivo-mechanism.esl`
(`concl_mech`) and `09-phase5-synthesis.esl` (`concl_main`).

### 6. Audit — query the chain for integrity
Before declaring done, query: does every conclusion resolve to a witness? Is every
anchor a real, cited source? Are there dangling claims (no consumer) or ungraded
assertions? `eigenius_query` over `justification:Conclusion` / `Verdict` makes this
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
