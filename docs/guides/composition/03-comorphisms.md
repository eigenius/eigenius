# 3. Comorphisms — bridges between domains

> **STATUS:** outline only. To be filled in.

## What this chapter covers

The load-bearing concept of the guide. A comorphism is the *declared,
type-checked bridge* between two institutions — chain-resident, not a
hand-coded ETL pipeline. This chapter is the structured reference for
how comorphisms work, what they guarantee, and how to read one
end-to-end.

Five things to internalise:

1. **The triadic structure.** `ExportFormat` (source side) +
   `transformation` Mini-TT Component + `ImportFormat` (target side).
   The kernel statically type-checks at commit time that the
   transformation's signature matches `payload_type(export) →
   payload_type(import)` (D14 §4.5). Mismatches reject the chain
   commit, not the dispatch.

2. **The four-step dispatch pipeline (D14 §9.3).** extract → transform
   → reify → optionally chain-reinsert. Walked through with the
   `symbolics_to_jump` comorphism from the kinase notebook.

3. **Identity vs. structural transformations.** Two of the kinase
   notebook's comorphisms have identity middles (because both
   endpoints share `FormulaTerm`); the third (Catalyst → DiffEq) has
   a structural middle that compiles a reaction network into an ODE
   right-hand side. What changes about the type-check, the runtime
   cost, and the audit story.

4. **The Satisfaction-Condition (`exact: bool`).** What `exact: true`
   actually claims (bit-for-bit payload preservation, no semantic
   loss); when `exact: false` is appropriate and what the contract
   means in that case (the kinase notebook's `catalyst_to_diffeq`
   comorphism is `exact: false` because mass-action ODE compilation
   is faithful only to the deterministic limit).

5. **Authoring a new comorphism.** What chain resources to commit,
   how to validate them locally, what the type-checker will refuse
   to commit and why.

## Section outline

- **§3.1.** Triadic structure: ExportFormat, transformation, ImportFormat
- **§3.2.** The four-step dispatch pipeline (extract → transform →
  reify → reinsert)
- **§3.3.** Identity transformations — when both endpoints share a
  payload
- **§3.4.** Structural transformations — Catalyst → DiffEq as the
  worked example
- **§3.5.** The `exact` flag and Satisfaction-Conditions
- **§3.6.** Static type-checking of the triple at commit time
- **§3.7.** Authoring a new comorphism (worked example: a hypothetical
  Symbolics → IntervalArithmetic-with-bounds)
- **§3.8.** Failure modes (mismatched payload types, missing
  ExportFormat, IO-capability transformation rejected in v1)
- **§3.9.** Note: theoretical foundations and a research direction

## §3.9 placeholder body

Institution theory was developed by Joseph Goguen and Rod Burstall in
the 1980s as a model-theoretic formalism for *abstract logical
systems* — a way to talk about "a logic" without committing to any
particular syntax, semantic carrier, or proof system. The original
framework lives squarely within classical set theory and Tarskian
model theory: an institution is given by a category of signatures, a
functor producing sentences over each signature, a functor producing
**models**, and a satisfaction relation linking sentences to models.
Comorphisms are the structure-preserving translations between
institutions in that classical setting.

Eigenius implements this framework in a *constructive* setting. The
kernel's small dependent-type theory (Mini-TT) plays the role of the
meta-language, and an institution's typed `Verdict` becomes a
chain-resident witness rather than a model-theoretic satisfaction
relation. The platform realises enough of the structure to work in
practice — declared comorphisms, type-checked transformations,
chain-resident verdicts, audit-traceable provenance — without claiming
to discharge the meta-theoretic equivalence between the model-theoretic
original and the type-theoretic realisation.

Closing that gap — formulating institution theory cleanly in
*constructive type theory*, with models replaced by **typed witnesses**
under the propositions-as-types reading (the Curry–Howard correspondence
introduced in [formula §2.2](../formula/02-mini-tt-fragment.md#22-why-pi-and-lam-are-chain-resident))
— is an open research direction. It is widely believed feasible:
Mini-TT's `Pi`-types already correspond to universal quantification,
its `Sigma`-types to existential quantification, and the kernel
already carries typed claims and typed verdicts as data. What's
missing is the meta-theoretic story tying the constructive realisation
back to the original model-theoretic framework with full equivalence
proofs. For the platform's purposes the practical realisation is
sufficient; for a type theorist who wants to *prove* that what
Eigenius implements is "really" institution theory, the translation
remains to be written.

## Cross-references

- [D14 §4.5](../../design/d14-institution-realisation.md) — comorphism
  type-check
- [D14 §9.3](../../design/d14-institution-realisation.md) — four-step
  pipeline
- [Formula guide §6.4](../formula/06-sharing-across-institutions.md#64-the-kinase-notebooks-three-comorphisms)
  — the three v1 comorphisms in summary form
- [`julia/comorphisms/`](../../../julia/comorphisms/) — the v1
  comorphism declarations

---

Next: **[4. The three dispatch roles in concert →](04-dispatch-roles-in-concert.md)**
