# 2. Shared payload languages

> **STATUS:** outline only. To be filled in.

## What this chapter covers

The first and cheapest layer of composition: agreeing on a shared *data
shape*. When two institutions both consume the same typed payload,
bridging them via a comorphism becomes nearly trivial. When they
don't, every comorphism has to do real translation work.

The chapter establishes three claims, all grounded in
[`formulas:FormulaTerm`](../formula/README.md) as the v1 example:

1. A shared payload is a *coordination mechanism*, not a domain
   vocabulary. FormulaTerm doesn't belong to any one institution; it
   lives in the kernel bootstrap layer at `urn:eigenius:formulas:`.
2. With a shared payload, comorphisms collapse to identity (or
   near-identity) transformations. Two of the kinase notebook's three
   comorphisms have identity middles for exactly this reason.
3. The principle generalises beyond formulas. Any chain-mirrored
   inductive type can play the same role; a future "shared
   process-graph language" or "shared logical-clause language" would
   coordinate the same way.

## Section outline

- **§2.1.** Why payload-shape agreement matters
- **§2.2.** `FormulaTerm` as a coordination mechanism (not a domain
  vocabulary)
- **§2.3.** Five institutions, one payload — the kinase setup at a
  glance
- **§2.4.** Identity-comorphism collapse: the structural payoff
- **§2.5.** When *not* to share a payload (when domains genuinely
  diverge)
- **§2.6.** What other shared payloads might look like (forward look:
  process graphs, logical clauses, codata)

## Cross-references

- [Formula language guide §6 — Sharing across institutions](../formula/06-sharing-across-institutions.md)
  — the FormulaTerm-specific deep dive that grounds this chapter.

---

Next: **[3. Comorphisms — bridges between domains →](03-comorphisms.md)**
