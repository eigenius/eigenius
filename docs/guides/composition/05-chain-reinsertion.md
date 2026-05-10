# 5. Chain reinsertion of comorphism outputs

> **STATUS:** outline only. To be filled in.

## What this chapter covers

[D14 §9.3](../../design/d14-institution-realisation.md) step 4: the
comorphism's reify output isn't a transport-only return value — it
becomes a **first-class chain resource** that downstream queries can
match, gates can fire on, and provenance can trace back to. This
chapter is the practical reference for how chain reinsertion works
through both the ESL and EigenQL surfaces.

Three things to internalise:

1. **Two surfaces, one mechanism.** ESL programs invoke a comorphism
   as a qualified-name function call (`comorphisms:foo(input)`), which
   lowers to `Exp::InstitutionInvoke`. EigenQL invokes it as a
   `FIBER ... AS ?var INTO "<iri>"` clause. Both go through the same
   `commit_with_validation` machinery; they differ in *who picks the
   IRI* (deterministic content-hash from the kernel vs. caller-named).

2. **Deterministic content-hash IRIs.** The default IRI for an ESL
   `Exp::InstitutionInvoke` invocation is
   `urn:eigenius:comorphism-output:<comorphism-tail>:<hex16>` where
   the hex16 is SHA-256 over the canonical Eigon-CBOR of the produced
   resource (with `@id` cleared). Re-running the same invocation
   dedupes to the same IRI — the cross-fibre identity property the
   Grothendieck construction wants.

3. **The audit trail.** Every chain-reinserted resource carries a
   `Trace::Comorphism { comorphism_iri, source_trace, target_iri,
   target_class }` audit variant on the program trace, plus the
   standard `RuntimeInvocation` provenance closure when the
   comorphism's institutions are external-runtime hosted.

The chapter closes with a section on *why this matters for
composition*: chain-reinserted comorphism outputs participate in all
the downstream machinery (AutoOnLoad gates, OnDemand FIBER lookups,
Decidable predicates, EigenQL queries) as if they had been authored
by hand. That's what turns a one-step translation into a building
block for multi-step pipelines.

## Section outline

- **§5.1.** Why chain reinsertion matters for composition
- **§5.2.** ESL surface: `comorphisms:foo(input)` → `Exp::InstitutionInvoke`
- **§5.3.** EigenQL surface: `FIBER ... AS ?var INTO "<iri>"`
- **§5.4.** Deterministic content-hash IRIs
- **§5.5.** Audit trail: `Trace::Comorphism` + `RuntimeInvocation`
- **§5.6.** Worked example: the kinase notebook's Part C end to end
- **§5.7.** Idempotence and the cross-fibre identity property

## Cross-references

- [D14 §9.3](../../design/d14-institution-realisation.md) — chain
  reinsertion contract
- [EigenQL §7.6](../eigenql/07-fiber-clauses.md) — `FIBER ... INTO`
  reference
- [ESL §9.5](../esl/09-institutions.md) — invoking comorphisms from
  ESL programs

---

Next: **[6. Walkthrough: reading the kinase notebook end-to-end →](06-kinase-walkthrough.md)**
