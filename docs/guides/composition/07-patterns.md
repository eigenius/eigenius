# 7. Composition patterns

> **STATUS:** outline only. To be filled in.

## What this chapter covers

The decision space when you're authoring a new cross-institution flow.
Each pattern below is a "when X applies, prefer Y" rule rooted in
trade-offs the previous chapters introduced.

Six patterns to cover:

1. **When to share a payload language** vs. when to declare a
   converter. Sharing is cheap once it lands but expensive to
   negotiate up-front (every consuming institution has to agree on
   the encoding). Converters add per-bridge translation work but
   keep institutions independent.

2. **Identity comorphism vs. structural comorphism.** Identity
   middles cost zero at runtime; structural middles do real work
   but capture genuine semantic translation (e.g. compiling a
   reaction network into an ODE). When `exact: true` is honest and
   when `exact: false` is honest.

3. **AutoOnLoad gating vs. OnDemand FIBER.** AutoOnLoad gates every
   commit; OnDemand fires only when asked. Use AutoOnLoad when the
   claim is the *result* you want to record; OnDemand when you want
   to *probe* without committing.

4. **Chain reinsertion vs. transient overlay.** EigenQL `FIBER`
   without `INTO` is overlay-only; with `INTO` it reinserts.
   Reinsertion is right when the produced resource is an interesting
   chain entity in its own right (gates downstream of it should fire);
   overlay-only is right when you just want the response in the
   query result set.

5. **Decidable predicates as constraints.** The "constraint attached
   to a property" pattern from ESL §9.6: an institution-decided
   constraint fires during type-check reduction and rejects the
   program if the predicate `Fails`. Composes well with chain
   reinsertion (a comorphism's reinserted output gets type-checked
   against the chain's constraints).

6. **Multi-step comorphism chains.** When you want
   `Catalyst → DiffEq → IntervalArithmetic` in sequence, you can
   either author a single composite comorphism or chain two
   comorphisms by reinserting the intermediate. Trade-offs: composite
   is one chain commit but harder to inspect; chained is N commits
   with full per-step audit but more chain noise.

## Section outline

- **§7.1.** Sharing a payload vs. declaring a converter
- **§7.2.** Identity vs. structural comorphism
- **§7.3.** AutoOnLoad vs. OnDemand
- **§7.4.** Chain reinsertion vs. overlay
- **§7.5.** Decidable predicates as constraints
- **§7.6.** Multi-step comorphism chains
- **§7.7.** Anti-patterns (common mistakes and why they're wrong)

---

Next: **[8. Failure modes across compositions →](08-failure-modes.md)**
