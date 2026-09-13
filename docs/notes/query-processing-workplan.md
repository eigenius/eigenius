# Query processing — workplan

**Written `2026-09-13`**, against `main` at `9de9fa2`. Every claim below was traced to the source
as it stands; where an issue's own citation has gone stale, the current location is given and the
staleness is called out, because a verification step that returns nothing reads as "fixed" to the
next person.

## Scope

Seventeen open issues concern query processing: the EigenQL language and its evaluation, retrieval
and ranking, index lifecycle and declaration, query memory, and the query RPC surface.

Deliberately excluded, with the reason, because the boundary is part of the plan:

| # | why not |
|---|---|
| #117 | Retroactive **validation** carrier enumeration. It would add a triple-index method the query side would also use, but the defect and its measurement are on the commit path. |
| #51, #134 | `RunProgram` cancellation. Same shape as #132 — a cancel flag nothing flips — in a different subsystem. |
| #127 | `TikvStore`'s four `todo!()`s: a storage-backend honesty problem. |
| #176 | Making WRN traceability queryable is ontology modelling. |
| #85 | AutoOnLoad two-phase gating: the load path. |
| #97, #212 | DCG chart pruning and the sense reranker: the natural-language parser, not query processing. |
| #40, #121, #110, #82, #165 | Validation, merge CLI, importer, statistics modelling, institution dispatch. |

## Three things the issues do not say

Found by reading the code rather than the issues, and each changes what the plan should do.

**`ORDER BY` over anything but a bare variable is a silent no-op.** `extract_sort_value` matches
only `Expression::Variable`; every other expression returns `None` for both operands, the
comparison is skipped, and the order is left untouched. D2 §8.8's own worked example —
`GROUP BY ?breed … ORDER BY COUNT(?d) DESC LIMIT 10` — therefore returns an unordered result with
no error. Same family as #123, same two files.

**D2 already decided #124, in the opposite direction from the implementation.** §6.2 says a
pattern may match a resource while a specific property has no value, leaving the variable unbound
for `NOT EXISTS` to detect, and §8.7 gives a worked example that depends on it. `MATCH` is strictly
conjunctive, so that example returns zero rows against any data. #124 and #33 are one decision.

**The similarity pre-pass result never reaches candidate enumeration.** The pre-pass computes a
subject-to-score map before matching runs, and `collect_candidates` never consults it — with no
class in the pattern it falls through to a full-chain scan. That is the cost the D43 notes
attribute to "the pattern-match scan, not the BM25 probe", and the narrowing set is already sitting
in the runtime when the scan starts.

## The work

Grouped so each item is one merge request, not one issue.

### A. Silent wrong answers in the evaluator — **do this first**

Four defects where a query returns a wrong number, a wrong order, or an empty set with no
diagnostic. Closes #123, #126, #172, and the unfiled `ORDER BY COUNT(…)` no-op.

One design decision, on **#126**: the fix is not to log and continue. Evaluation must distinguish
"this resource does not carry that property", which is a legitimate filter drop, from every other
error, which is a query failure. That is a variant on the error type, not a guard — wrapping the
existing swallow in a warning and calling it observability is the Band-Aid to avoid.

**Do #126 before the rest of A.** Until those two cases are distinguishable, every dot-path mistake
in every later item presents as an empty result set, which makes C and B materially harder to
debug. This is the cheap thing that unblocks the others.

Two to three days. No design note. No manifest move.

### B. `MATCH` optionality — needs its own design note

Resolves the contradiction between D2 §6.2/§8.7 and a strictly conjunctive `MATCH`. Closes #33;
closes or redefines #124.

The item **is** the decision, and there are three shapes: brace variables bind optionally, which is
what D2 already says and would silently widen every existing query's results; an explicit optional
block per #33, leaving §6.2 to be rewritten; or drop `NOT EXISTS` and keep only pattern negation.

The hard sub-problem is the result document. D2 states there is no null literal and Eigon-JSON has
none, so an unbound column has nowhere to land. Decide that before touching the parser, and do not
emit a sentinel for it.

Start the note in parallel with the items below — it is the long pole — but land it after A,
because it rewrites the same two functions A touches.

### C. Bound the probes, and seed candidates from them

Makes `TOP N` reach the text and vector probes, and makes the already-computed similarity subject
set replace the full-chain candidate scan. Closes #62's substance and #125.

**Do not add a planner.** #62 asks for a field on "the planner's logical-plan node" and there is no
planner; inventing a logical-plan layer for one pushdown is the wrong shape. The pushdown belongs
in the existing pre-pass.

Two small decisions: where the probe bound comes from when `TOP N` is absent, and the soundness
argument for candidate seeding, which holds only because similarity sits in a conjunctive `WHERE`.
Write that argument into the merge request rather than leaving it implied.

Three to four days, including a re-run of the D43 performance bench. This carries the only measured
performance number in the set.

### D. Vector-index lifecycle: call the code that already exists

Three entry points that are written, tested and never invoked. Closes #133 and #132.

One small decision: D43 names layer deletion as the cancellation point and four source comments
repeat it, so cancelling from the caller would leave those comments false and any other delete path
uncovered. Plumb the registry into the collector so the cancel happens where the delete is decided.

One to two days.

### D2. Crash-resilient sweep

Closes #59: persist the task record, checkpoint per batch, resume on startup. Depends on D, same
files. Days.

### E. HNSW build quality

Closes #61, and picks up the unfiled build-time finding of roughly quadratic growth recorded in the
D43 notes. Adds the paper's §4.3 neighbour heuristic beside the current simple selection, behind a
build-config flag. No decision. One to two days plus bench wall-clock.

### F. Sweep throughput

Closes #63: length-bucket before chunking in the vector sweep. The existing
batch-size-independence test is the invariant that must still hold. No decision. Hours.

### G. Index declaration honesty

Two surfaces that promise behaviour the kernel does not implement. Closes #140 and #171.

**Both of the obvious fixes are wedges, and this item exists to avoid them.** For #140, the
production ontology already uses the resource form and nothing in the tree uses the keyword, so the
keyword and its AST nodes either go or get their lowering implemented — rejecting earlier with a
better message keeps the dead grammar and adds a guard. For #171, `allows_only` narrows to the one
policy that exists, or the other two get implemented; **option (c), a kernel-side rejection that
contradicts the committed ontology, is not on the table.**

#140 costs hours and nothing else. **#171 moves the bootstrap manifest** — the `allows_only` edit
rehashes the core layer and every layer below it, so every persisted store refuses to resume until
a reseed, followed by a parse measurement. Bundle #171 with the next bootstrap edit that is
happening anyway rather than paying a reseed for it alone.

## Order

1. **A**, and inside it #126 first.
2. **F** — hours, independent, no decision.
3. **C** — the largest measured number, and what turns the ranking module from dead code into the
   live path.
4. **D**.
5. **B**, with its design note started in parallel from the beginning.
6. **E**, then **G**/#140, then **D2**, then **G**/#171 when a reseed is already being paid.

If only two could be funded: **A**, for wrong answers cheaply fixed, and **C**, for the one measured
performance number and the structural finding sitting unused in the runtime.

## Set aside

Agreed `2026-09-13`, not deferred by default:

- **#129 — out-of-core query execution.** Seven undecided sections is a phase, not a merge request.
  No out-of-memory failure has been witnessed on the query-operator path; the ones this repo has
  actually hit were full-chain scans on the validation side. Item C removes the largest unbounded
  materialisation a query can produce. Revisit when a query operator actually kills the process.
- **#83 — read-pin indicator.** The substantive half shipped: the pin is visible with a one-click
  return on every destination, and setting it clears stale cell state. What remains is a user-
  experience policy question, not the silent inconsistency the issue was filed about. Retitle to
  that question or close.
- **#45 — FIBER multi-query-class.** No defect; it removes a duplicate declaration pattern in one
  demo notebook, at the cost of AST, parser, type-checker and evaluator changes that would collide
  with whatever B does. Revisit if a second institution needs it.
- **#62's literal ask** — the planner node. The substance is in C.
