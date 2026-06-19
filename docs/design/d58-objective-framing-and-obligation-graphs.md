# D58 — Objective Framing & Obligation Graphs

*Status: **ontology designed** (§5, from the D57 dogfood harvest) · design memo · June 2026 · gate-query wiring + loop budget remaining*

*Companion documents: [D39 justification logic](d39-justification-logic.md) (the warrant calculus), [D43 text & vector retrieval](d43-text-and-vector-retrieval.md) + [D57 schema.org mapping](d57-schema-org-vocabulary-mapping.md) (grounding), [D49 chain-witness machinery](d49-chainwitness-machinery.md), [D54 lemma citation](d54-reasoning-lemma-citation.md). Operationalized by the `reasoning` + `grounding` skills. **Not** to be confused with D21 kernel tasks or `bench:TaskOutput` — see §6.*

*This memo specifies how a unit of science/engineering work — an **objective** — is **framed in Eigenius before work begins**: as a typed **obligation graph** (a thesis, the axioms it may assume, and the milestone propositions to derive), made **well-posed** through a frame⇄ground iteration loop. The reasoning protocol assumes a well-posed objective; this is the missing assessment phase that produces one. The shape, the admissibility gates, **and the `objective:` ontology (§5)** are settled — the ontology designed from the first real dogfood (D57; harvest in `experiments/objectives/d57-schema-org/HARVEST-d58.md`), not from speculation. The gate-query wiring and the loop budget remain open. (The object is `objective:Objective` to avoid collision with the kernel's execution "task" notion — §6; "objective" reads naturally across science and engineering disciplines.)*

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

## 5. The objective ontology (`objective:`)

Designed from the D57 dogfood, where the obligation graph was first run
"lightweight" (bare `Prop` declarations + prose comments). Each construct below
answers a place that forced prose where a typed node belonged
(`HARVEST-d58.md`, findings H1–H8). Realized as
`ontologies/objective/objective-ontology.esl` (loads — *Expressible* by
construction).

**The three planning nodes are structural, not epistemic.** They reference
propositions and witnesses; the *claims* stay where the epistemic stack already
puts them (the `Prop`, the `ReasoningSentence` that discharges it, the
Observed/Declared/Citation witness). This keeps `objective:` a thin planning layer
over the reasoning stack rather than a parallel epistemics.

### 5.1 Classes

- **`objective:Objective`** — the root planning node for a unit of work. Holds the
  **thesis** (→ a Milestone), a **version** + a `supersedes` edge to the prior
  version (H3 — reframing is the normal case; a revision points back and the gates
  re-run), the working **branch**, and an overall **status**.
- **`objective:Milestone`** — a proposition to establish, with acceptance criteria
  (H1, H2). Carries `proposition` (the `Prop`, same value the discharging
  `ReasoningSentence` will carry — H6), `acceptance_grade`, `witness_kind`,
  `falsifier`, the intended-warrant edges `depends_on` (→ Milestones/Axioms), a
  `status` (`open | blocked | admissible | satisfied`) and, while open, the
  `frontier_gate` that is pending (H4). **Completion is a query** (§2): the
  milestone is satisfied iff a `Holds` `ReasoningSentence` carries its
  `proposition` (`satisfied_by` records which). The thesis is just the root
  Milestone.
- **`objective:Axiom`** — a premise admitted without derivation: a *pointer* to an
  already-admitted witness, not the witness itself. Carries `proposition`,
  `axiom_kind` (`observed | declared | cited`), and `witness` (the
  `IsObservedAs` / `IsDeclaredAs` / `reference:Citation` IRI). Anything lacking a
  resolvable `witness` is an unjustified assumption, not an axiom (the *Anchored*
  gate).

### 5.2 Key properties

| Property | On | Type | Role |
|---|---|---|---|
| `objective:thesis` | Objective | resource → Milestone | the root goal |
| `objective:version` / `objective:supersedes` | Objective | integer / IRI | frame revision (H3) |
| `objective:branch` | Objective | string | branch-per-objective (H7) |
| `objective:proposition` | Milestone, Axiom | *(as `reasoning:proposition`)* | the `Prop` |
| `objective:acceptance_grade` | Milestone | string | `observed\|declared\|derived\|verified` (H2) |
| `objective:witness_kind` | Milestone | string | `layer-commit\|query\|generator-output\|citation` (H2) |
| `objective:falsifier` | Milestone | string | what would refute it (H2, *Checkable*) |
| `objective:depends_on` | Milestone | resource[] → Milestone/Axiom | intended-warrant edge (H1) |
| `objective:status` | Objective, Milestone | string | `open\|blocked\|admissible\|satisfied` (H4) |
| `objective:frontier_gate` | Milestone | string | pending gate while open (H4) |
| `objective:satisfied_by` | Milestone | string | the discharging `ReasoningSentence` IRI (H6) |
| `objective:axiom_kind` / `objective:witness` | Axiom | string / IRI | admitted-witness pointer |

### 5.3 The four gates as `QueryClass`es

The §3 gates become committed, mechanically-checkable queries over the objective's
layer (H5) — three are real EigenQL; *Expressible* is just "the frame loaded":

- **Anchored** — every `objective:Axiom` whose `witness` does **not** resolve to an
  `IsObservedAs`/`IsDeclaredAs`/`reference:Citation` is a violation.
- **Reachable** — every non-thesis Milestone is reachable along `depends_on` from
  the thesis, and every `depends_on` target resolves (no dangling/cyclic edge).
- **Checkable** — every Milestone has `acceptance_grade` + `witness_kind` +
  `falsifier` set.

An AutoOnLoad `QueryClass` on `objective:Objective` emits a **`WellPosed`** /
**`Blocked`** verdict (mirroring the reasoning institution's `Holds`/`Fails`), and
**re-fires on each new version** (H3). Completion is the separate thesis-Holds
query (§2).

### 5.4 Remaining open

- **Gate-query wiring** — §5.3 specifies the gate semantics + EigenQL sketches;
  registering them as a `QueryClass`/institution emitting `WellPosed`/`Blocked` is
  the implementation step.
- **Completion artifact** — whether a satisfied Objective emits a dedicated
  deliverable resource (distinct from `bench:TaskOutput`; §6) or completion is
  purely the thesis-Holds query. Leaning: query only, no new artifact.
- **Loop budget / convergence** — guard against non-terminating reframing; when to
  declare "ill-posed" vs. keep grounding. (Out of the ontology; a protocol policy.)
- **Archival / GC** — how a finished objective's branch is archived.

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
