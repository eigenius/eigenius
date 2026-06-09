# 10. Appendix

> **STATUS:** outline only. To be filled in.

## What this chapter covers

References, source index, and cross-links. No new material — just the
machinery a reader uses to follow up on the rest of the guide.

## Section outline

### §10.1. Source index

- [`kernel/src/institution/`](../../../kernel/src/institution/) — `Institution`
  trait, `InstitutionRuntime`, `InstitutionIndex`, AutoOnLoad dispatch
- [`kernel/src/nbe/eval.rs`](../../../kernel/src/nbe/eval.rs) — the
  `Exp::InstitutionInvoke` arm and the four-step pipeline
- [`kernel/src/program/trace.rs`](../../../kernel/src/program/trace.rs) —
  `Trace::Comorphism` audit variant
- [`crates/runtime-substrate/`](../../../crates/runtime-substrate/) —
  substrate hosting for external-runtime institutions
- [`julia/comorphisms/`](../../../julia/comorphisms/) — the v1
  comorphism declarations
- [`notebooks/examples/kinase-institutions.json`](../../../notebooks/examples/kinase-institutions.json)
  — the canonical worked example

### §10.2. Related design documents

- [**D14** — Institution Realisation](../../design/d14-institution-realisation.md)
  — institution mechanism (supersedes D10); §4 the resource shapes,
  §6 the dispatch roles, §9.3 the chain-reinsertion contract
- [**D26** — Runtime substrate](../../design/d26-runtime-substrate.md)
  — substrate hosting layer
- [**D27** — Julia institutions](../../design/d27-julia-institutions.md)
  — the v1 Julia institution suite
- [**D29** — Mirror generator](../../design/d29-runtime-mirror-generator.md)
  — closure walker over chain shapes
- [**D31** — Institution lifecycle](../../design/d31-runtime-language-substrate-institution-lifecycle.md)
  — install + audit lifecycle
- [**D32** — Chain-mirrored EigenTT inductives](../../design/d32-chain-mirrored-mini-tt-inductives.md)
  — `formulas:FormulaTerm` and the inductive-types-on-the-chain
  mechanism

### §10.3. Companion notes

- [Note for a SHACL user](../../notes/note-for-a-shacl-user.md) — the
  conceptual pitch for someone coming from the W3C semantic-web
  stack. Frames why composition matters in narrative form.
- [Enterprise supply-chain scenario](../../notes/enterprise-supply-chain-scenario.md)
  — the same machinery applied to an enterprise setting; useful as a
  domain-transfer exercise.

### §10.4. Cross-language guides

- [ESL](../esl/README.md) — surface syntax for ontologies and programs
- [EigenQL](../eigenql/README.md) — surface syntax for queries
- [Formula language](../formula/README.md) — chain-mirrored EigenTT
  fragment used as the v1 cross-institution payload

### §10.5. Per-host implementer chapters

- [Platform §10 — Building WASM institutions](../platform/10-wasm-institutions.md)
- [Platform §11 — Runtime substrate](../platform/11-runtime-substrate.md)
- [`platform/julia-institutions/`](../platform/julia-institutions/) —
  per-institution Julia tutorials

### §10.6. Phase status

The composition surface is complete through Phase 19i (D14 §9.3 chain
reinsertion through both ESL and EigenQL surfaces). Tracked next:

- Per-payload sharing beyond `FormulaTerm` (planning).
- Lean-4 as a verification institution (D28; not yet wired).
- The first non-Julia substrate runtime (Python tracked at
  [issue #41](https://github.com/eigenius/eigenius/issues/41)).
- The first cross-host comorphism (a Julia substrate institution
  bridging to a WASM-hosted institution).
- A formal translation of institution theory (Goguen & Burstall, set +
  model-theoretic) into constructive type theory, replacing models with
  typed witnesses under Curry–Howard. Background context in
  [§3.9](03-comorphisms.md). Open research direction; widely believed
  feasible — the kernel's EigenTT already carries the load-bearing
  pieces (`Pi`/`Sigma` types, typed inductive verdicts), the gap is
  the meta-theoretic equivalence proof.

---

Return to **[README](README.md)**.
