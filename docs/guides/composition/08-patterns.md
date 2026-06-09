# 8. Composition patterns

This chapter is the decision space: **when X applies, prefer Y**, with the
reasoning grounded in the trade-offs the previous chapters introduced. The
patterns here are extracted from the platform's v1 surface (the kinase
notebook + the dock-assay WASM demo); as more comorphisms land, more
patterns will join them.

> **Note on maturity.** v1 has three comorphisms in the kinase notebook
> plus the dock-assay WASM trio. That's enough material for the basic
> shape decisions in §7.1–§7.5; the multi-step chain pattern in §7.6 is
> projected from the architecture, not yet exercised end-to-end. Treat
> this chapter as a working set of recommendations that will tighten as
> more compositions land.

## 8.1. Sharing a payload vs. declaring a converter

**Use a shared payload language when** multiple institutions naturally
operate on the same kind of data (numerical expressions, planning trees,
logical clauses). The cost is up-front coordination — every consuming
institution has to agree on the encoding — and the saving is structural:
O(N) boundaries instead of O(N²) per-bridge converters, and identity
comorphisms become possible (chapter 3 §3.3).

**Use a per-bridge converter when** institutions genuinely operate on
different shapes — one stream-based, one record-based, one with explicit
error sentinels and another with total functions. Forcing a shared payload
in this case produces an awkwardly-general type that nobody finds natural,
and every institution writes ad-hoc adapters from the shared shape into
its preferred form.

**Three signals you're in shared-payload territory:**

1. The institutions you want to bridge already have a common ancestor in
   the literature (e.g. anything that consumes "an algebraic expression
   over real numbers" lives downstream of a EigenTT-shaped term language).
2. Cross-institution comorphisms are easy to imagine (S → I, S → J, I → J,
   etc. — the combinatorics matter).
3. The shared shape has a natural home in a *peer namespace* — not under
   any one institution's IRI. `urn:eigenius:formulas:` is the v1 example;
   a hypothetical `urn:eigenius:planning:` would be the next.

**Three signals you're not:**

1. The "shared" type carries fields only one consumer cares about — every
   other institution just sets them to default values.
2. Bridges keep growing custom validation rules that try to constrain the
   shared shape down to what each institution actually expects.
3. Adding a new institution requires extending the shared type, and
   existing consumers either ignore the extension or break.

## 8.2. Identity vs. structural comorphism

**Identity comorphism** when both endpoints share a payload language. The
transformation Component is `Lam(t. Var(t))`; the chain bytes flow through
unchanged; runtime cost is one β-step. The payoff is `exact: true` — the
comorphism preserves every sentence's truth, and the audit trail says so.

The kinase notebook's `symbolics_to_intervals` and `symbolics_to_jump` are
both identity comorphisms (modulo framing wrappers — the latter wraps a
`SymbolicExpression` in a `SymbolicsToJuMPInput` composite to carry JuMP-side
metadata; the FormulaTerm payload itself flows through unchanged).

**Structural comorphism** when the transformation does real work. Catalyst
→ DiffEq is the worked example: compiling a reaction network into an ODE
right-hand side is a real translation that captures *the deterministic
limit* of the chemical kinetics. The transformation is honest about what
it loses (the stochastic structure of the master equation), which is what
`exact: false` declares.

**The right choice is rarely a free hand.** If both endpoints already
speak the same payload, identity is cheap and honest — there's no
structural argument for adding work. If they don't, identity is a lie —
you'd need to either coerce one side's data into the other's ad-hoc, or
decline the bridge. Structural is the right answer for genuinely different
shapes.

**A useful question to ask:** would two domain experts argue about the
right way to do the translation? If yes, structural. If no, identity.

## 8.3. AutoOnLoad vs. OnDemand

**AutoOnLoad** when the claim *is* the result you want to record. Every
commit of the gated class fires the gate; the Verdict + RuntimeInvocation
commit alongside; the chain has a permanent record. Right for:

- *Claims you author and want verified* — `OdeSolution` claiming a
  specific final state, `OptimisesTo` claiming a specific optimum.
- *Constraints you want enforced* — `BoundedBy` claiming a value lies in
  an interval; rejection on `Fails` keeps the chain consistent.
- *Audit requirements* — every chain entry of a gated class has a
  Verdict on the chain proving the institution agreed (or didn't).

**OnDemand FIBER** when you want to *probe* without committing. The
FIBER call queries the institution; the response lives in the transient
overlay (or, with `INTO`, commits at a caller-named IRI). Right for:

- *Exploratory queries* — "what does Symbolics simplify this to?",
  without polluting the chain with intermediate `SimplifiesTo` claims.
- *Computed properties for use within a single query* — FIBER produces a
  result; subsequent clauses match against it; the response evaporates
  when the query returns.
- *Generating a candidate to commit later* — call OnDemand to compute,
  inspect the result, decide whether to commit a claim built around it.

**The middle case** is OnDemand FIBER `... INTO`. Same call as plain
OnDemand, but the response commits to the chain at the named IRI. Use
`INTO` when:

- The OnDemand call's result is interesting in its own right, not just
  as an intermediate.
- Downstream queries (or cells, or programs) need to find the result by
  IRI rather than re-executing the call.
- You want the gate cascade — committing the response triggers any
  AutoOnLoad gate bound to its class.

**Common mistake:** using AutoOnLoad to gate an *exploratory* commit
(e.g. "I want to see what the institution thinks of this hypothetical
claim before committing it"). The gate's `Fails` aborts the commit, so
the chain has no record of the exploration. OnDemand FIBER is the right
tool — call it, look at the response, then decide whether to commit a
claim afterwards.

## 8.4. Chain reinsertion vs. transient overlay

EigenQL `FIBER ... AS ?var` has two flavours: with `INTO` (chain
reinsertion) and without (transient overlay). The difference matters
when a downstream consumer wants to find the response *outside the
current query's scope*.

**Use `INTO` (chain reinsertion) when:**

- The produced resource is an interesting chain entity in its own right.
- Gates downstream of it should fire (the AutoOnLoad cascade pattern).
- The IRI matters — for stable references, external system keys, or
  cross-cell lookups in a notebook.
- Multiple queries need to read the same response without re-executing.

**Use overlay-only (no `INTO`) when:**

- The response is intermediate machinery — a value the current query
  consumes once and discards.
- The institution call is read-only and side-effect-free; commits would
  be noise.
- Repeated runs of the same query should not accumulate chain residents.

**A subtle case:** when an OnDemand call is *deterministic* (same input,
same output), `INTO` is safe to use for caching — re-running the cell
finds the existing committed resource and skips re-execution. When it's
*non-deterministic* (e.g. uses a random seed; depends on host BLAS
behaviour), `INTO` overwrites or errors depending on validator state.
The kinase notebook's comorphism dispatches are deterministic (EigenTT
evaluation + Pkg-pinned Julia), so `INTO` is safe. The kinase
notebook's hypothetical Monte-Carlo extensions wouldn't be.

## 8.5. Decidable predicates as constraints

The "constraint attached to a property" pattern from
[ESL §9.6](../esl/09-institutions.md#96-constraints-attached-to-properties):
a Decidable QueryClass IRI carried as a constraint on a property fires
during type-check reduction (Check mode) and rejects the program if the
predicate `Fails`.

**Use it when:**

- A program's correctness depends on a value satisfying a domain-specific
  invariant (e.g. "the optimised Kᵢ must be within the screened range of
  the compound" — a treasury covenant, a regulatory bound).
- The constraint is checkable by an institution that already exists; you
  don't want to duplicate the check inside the program.
- You want type-check time enforcement, not runtime — the constraint
  rejects the program at compile time if it provably fails.

**Composes well with chain reinsertion:** a comorphism's reified output
is type-checked against the chain's constraints exactly the way a
hand-authored resource would be. So you can layer:

1. ESL program invokes a comorphism (chain-reinserts the produced
   resource).
2. The produced resource has a Decidable constraint on one of its
   properties.
3. The constraint's QueryClass dispatches at commit time; if `Holds`,
   the commit goes through; if `Fails`, the commit is rejected and the
   program reports the violation.

The shape generalises to multi-step pipelines: each step's output
participates in the next step's constraint check. The chain's role as
the source of truth means each step sees a fully-validated upstream.

## 8.6. Multi-step comorphism chains

When you want a sequence of translations — Catalyst → DiffEq →
IntervalArithmetic, or Symbolics → JuMP → Solver Verdict — there are two
shapes to choose between:

**Composite comorphism.** Author one new comorphism whose `transformation`
is the *composition* of the two intermediate transformations. Pros: one
chain commit; one user-facing dispatch; intermediate state is opaque
(might be desirable for consumers who don't care about it). Cons: harder
to inspect and audit (the intermediate value isn't on the chain); harder
to reuse pieces (can't extract just the first stage); requires authoring
the composite Component if either intermediate is structural.

**Chained comorphisms (with chain reinsertion).** Author one ESL program
that invokes the first comorphism, captures its reify output, then
invokes the second comorphism on that output. Pros: each intermediate is
a chain resident with full audit (Trace::Comorphism per step); each
intermediate can be inspected, queried, and gated independently;
pieces are reusable across different chained pipelines. Cons: N chain
commits for N steps; more chain noise; the intermediate IRIs
(content-hash) need to be navigated by downstream consumers.

**Trade-off principle:** chained comorphisms favour *transparency and
reusability*; composite comorphisms favour *brevity and opacity*. Most
new pipelines should start chained — you can always collapse to a
composite later when the intermediate becomes uninteresting.

A worked-shape example (not yet exercised):

```esl
program nb:fit_with_bounds
    : symbolics:SymbolicsToJuMPInput -> tighter:KiBoundedAbove
{
    // Step 1: fit the optimum.
    let optimum : jump:OptimisesTo =
        comorphisms:symbolics_to_jump(input) |> run_solver;
    // Step 2: bound the result.
    comorphisms:optimum_to_bounded_ki(optimum)
}
```

Each step's output reinserts at a content-hash IRI; the program's
return value is the second step's reify output; the audit trail walks
back through both `Trace::Comorphism` events. The chain's identity
property (chapter 5 §5.7) means re-running the program with the same
input dedupes both intermediate commits *and* the final commit.

## 8.7. Anti-patterns

Three common mistakes worth flagging:

### Sharing a payload that "almost" matches

Trying to fit two institutions into a shared payload by extending the
shared type with optional fields that only one cares about. Symptom:
the shared type accumulates `Option<...>` fields, with each consumer
ignoring the ones it doesn't recognise. The result is a payload language
that's lying about the structure — every consumer needs runtime checks
that the fields it cares about are present.

**Fix:** declare a per-bridge converter. The two institutions don't
genuinely share a payload; pretending they do hides the work in
defensive runtime checks.

### Using AutoOnLoad to gate everything

Putting an AutoOnLoad gate on every interesting class because "audit is
good." Symptom: every chain commit fires N gates; each commit becomes
slow; many gates produce `Undecidable` because the institution can't
actually decide most claims it sees.

**Fix:** AutoOnLoad is for claims you *want verified*. If the institution
doesn't have a meaningful answer for a class, don't gate the class.
OnDemand is the right shape for "answer if asked."

### Comorphism without `exact` consideration

Authoring a comorphism with `exact: true` because "it preserves the
data, mostly." Symptom: downstream audit code can't distinguish a truly
exact comorphism from a "mostly exact" one; verdicts that travel
through lossy chains are reported as authoritative.

**Fix:** `exact: true` is a substantive claim. If the comorphism does
*any* structural collapse — coerces representation choices, drops
metadata, projects to a smaller domain — set `exact: false` and let
downstream code decide whether the loss matters for its purpose.

### Chain reinsertion without an audit consumer

Using `FIBER ... INTO` to commit comorphism outputs that no downstream
code ever reads. Symptom: chain bloat with content-hash IRIs nothing
references.

**Fix:** if the response is exploratory, drop `INTO` — the overlay form
is the right tool. Reserve chain reinsertion for resources downstream
queries actually look up.

---

Next: **[9. Failure modes across compositions →](09-failure-modes.md)**
