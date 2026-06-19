# D58 — Objective Framing & Obligation Graphs

*Status: **stub** · design memo · June 2026*

*Companion documents: [D39 justification logic](d39-justification-logic.md) (the warrant calculus), [D43 text & vector retrieval](d43-text-and-vector-retrieval.md) + [D57 schema.org mapping](d57-schema-org-vocabulary-mapping.md) (grounding), [D49 chain-witness machinery](d49-chainwitness-machinery.md), [D54 lemma citation](d54-reasoning-lemma-citation.md). Operationalized by the `reasoning` + `grounding` skills. **Not** to be confused with D21 kernel tasks or `bench:TaskOutput` — see §6.*

*This memo specifies how a unit of science/engineering work — an **objective** — is **framed in Eigenius before work begins**: as a typed **obligation graph** (a thesis, the axioms it may assume, and the milestone propositions to derive), made **well-posed** through a frame⇄ground iteration loop. The reasoning protocol assumes a well-posed objective; this is the missing assessment phase that produces one. **Stub:** the shape and the admissibility gates are settled below; the exact ontology and the gate-query encodings are open. (The object is `objective:Objective` to avoid collision with the kernel's execution "task" notion — §6; "objective" reads naturally across science and engineering disciplines.)*

---

## 1. Motivation

The `reasoning` protocol turns claims into graded, witnessed propositions — but it
**starts from an already-stated thesis and known premises.** Real work rarely does.
You usually cannot write the axioms or milestones until you have grounded enough to
*express* them, and what to ground is steered by the half-formed framing. Framing
and grounding are **co-recursive**. Without an explicit assessment phase, this
bootstrapping happens implicitly in prose — exactly where the framing-level failures
hide: starting on an **ill-posed** objective, assuming **unanchored** premises as
axioms, or pursuing a milestone with **no evidence path**. (Both occurred in the WRN
work: a plan written from panel names before the data was understood; "redundant"
asserted before the measure was checked.)

So: make the objective itself a first-class typed object, and make "is it well-posed?"
a mechanical question.

## 2. The obligation graph (settled shape)

An objective is a DAG of propositions with acceptance criteria:

- **Thesis** — the root proposition + the **grade** it must reach (Observed /
  Declared / Derived / Verified) to count as done.
- **Axioms** — leaves: premises admitted *without* derivation, each an admitted
  witness in exactly one of three legal kinds — **Observed** (data),
  **Declared** (definition/design rule, with rationale + `DeclarationTrace`),
  **Cited** (`reference`/CiTO anchor). Anything not derivable and not one of these
  is an *unjustified assumption*, not an axiom.
- **Milestones** — internal nodes: intermediate propositions to **derive** from
  axioms or lower milestones; each a sub-thesis with its own acceptance grade. A
  milestone completes when a `Holds` `ReasoningSentence` (or admitted witness)
  carries its proposition.
- **Edges** — the *intended* warrant: which axioms/milestones discharge which.
- **Acceptance criteria** (per node) — grade + the **witness kind** that satisfies
  it + a **falsifiability** note (what would count as evidence / refute it).

**Completion is a query**, not a judgment: the thesis `ReasoningSentence` Holds,
which by the D39 commit gate entails every milestone Holds and every axiom is
admitted. (A completed objective may emit a cited summary/report resource referencing
those conclusions — but that is *not* a `bench:TaskOutput`; §6.)

## 3. Well-posedness — the four admissibility gates (settled)

Assessment = build the graph and check it passes four gates, each mechanically
checkable over the objective's layer:

| Gate | Question | Check | Failure → action |
|---|---|---|---|
| **Expressible** | every proposition writable in the available vocabulary? | ESL type-checks / compiles (an undefined predicate fails) | **ground**: import/align/declare terms |
| **Anchored** | every axiom has an admitted witness? | EigenQL: axioms lacking `IsObservedAs`/`IsDeclaredAs`/a citation | **ground** (cite/observe) or demote to a flagged Declared hypothesis |
| **Reachable** | every milestone has a candidate evidence path? | an incoming warrant edge / plausible witness kind exists | decompose, or record **blocked** |
| **Checkable** | every proposition's acceptance defined? | node states grade + witness kind + falsifier | sharpen until you can say what evidence satisfies it |

All four pass for the whole DAG ⇒ **well-posed** ⇒ the linear execute phases begin.
This is the framing-level analogue of "don't assert unwitnessed": *don't start
deriving until the obligations are expressible, anchored, reachable, and checkable.*

## 4. The frame⇄ground fixpoint (settled)

```
seed goal + seed context
  └─► draft obligation graph (provisional thesis/milestones/axioms; mark
  │     not-yet-expressible / unanchored nodes OPEN — the grounding frontier)
  └─► assess the 4 gates ──► all pass? ─yes─► well-posed; EXECUTE (reasoning loop)
        │ no — each failing gate names a grounding/reframing action
        ▼
      Expressible ✗ → grounding: import/align vocabulary so the term exists
      Anchored    ✗ → grounding: retrieve/research/cite the missing premise
      Reachable   ✗ → reframe: decompose the milestone, or record "blocked"
      Checkable   ✗ → reframe: sharpen the acceptance criterion
        └─► re-draft (sharper, with the new ground) ──► reassess
```

A genuine fixpoint: `frame` depends on `ground`, `ground` is steered by `frame`;
iterate until the graph stops changing *and* is admissible. `grounding`'s
retrieve-first means the frontier shrinks each pass (later passes reuse earlier
imports), so it converges. The bottom of the recursion is a vague goal + seed
context; the top is a well-posed obligation DAG. When even the *thesis* isn't yet
expressible, the outer loop is explore/ground → form a provisional thesis →
decompose.

**Termination is honest.** The loop ends either *admissible* (execute) or with a
gate that **cannot** be closed — no evidence for an axiom, no path to a milestone,
a term with no grounding. That is not license to proceed on faith; it is a
**recorded finding**: the objective is (currently) ill-posed or blocked here, with the
specific gap. Catching that before sinking effort is the phase's main payoff.

## 5. Open questions

- **Ontology shape.** New `objective:Objective` / `objective:Milestone` /
  `objective:Axiom` classes vs. reuse: milestones as `ReasoningSentence` stubs
  (proposition set, no justification yet) + an `OPEN`/frontier status; axioms as
  plain Observed/Declared/Citation resources tagged into the objective; the thesis as
  the root milestone. How to represent the *intended warrant edge* before the
  certificate exists (a declared `intends`/`depends_on` link?).
- **Acceptance-criterion encoding** — grade + witness-kind + falsifier as
  properties; how the gate dispatch reads them.
- **The gate queries** — the exact EigenQL for Anchored / Reachable / Checkable
  (Expressible is just "does it compile"); whether an AutoOnLoad `QueryClass`
  should emit a `WellPosed` / `Blocked` verdict on an `Objective` resource.
- **Objective isolation** — branch-per-objective (`branch create <slug>`) vs an
  objective-rooted layer; how the obligation graph is GC'd or archived when done.
- **Frontier representation** — how OPEN nodes + their failing gate are marked so
  the loop (and a human) can see the remaining grounding work at a glance.
- **Completion artifact** — whether a completed objective emits a dedicated
  deliverable resource (distinct from `bench:TaskOutput`), or completion is purely
  the thesis-Holds query.
- **Loop budget / convergence** — guard against non-terminating reframing; when to
  declare "ill-posed" vs keep grounding.

## 6. Relationship to kernel tasks (D21) and `bench:TaskOutput` — distinct concepts

The word "task" is overloaded; this memo deliberately uses **objective** instead.

- **D21 tasks** are *kernel execution units* — a program run, foreground or
  background, observable via `tasks list` / `get_task_status`. About *running code*.
- **`bench:TaskOutput`** (`experiments/benchmark/harness-ontology.esl`) is the
  chain-resident **deliverable handle for a program run** — the artifact a run
  produces, with its `reasoning_chain`. About a *run's output*.
- **An `objective:Objective`** (this memo) is the **reasoning frame** for a unit of
  work — the obligation graph (thesis + axioms + milestones) that says *what we are
  trying to establish and what would establish it*. About *the question and its
  proof obligations*, not about running anything.

They compose but do not coincide: executing an objective's milestones may **spawn**
D21 program-run tasks (and produce `bench:TaskOutput`s) as the *means* of
discharging a Derived obligation — but the objective is the frame, not the run. Keep
the namespaces separate (`objective:` ≠ kernel task IDs ≠ `bench:`).

## 7. Out of scope

- Execution scheduling / running tasks — D21; this memo is about *well-posedness*,
  not task management.
- The grounding mechanics themselves — D43 (retrieval), D57 (vocabulary), the
  `reference` ontology (anchors); this memo only *invokes* them via the loop.
- The warrant calculus — D39; obligations become `ReasoningSentence`s discharged by
  its certificates.
