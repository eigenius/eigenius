# 1. Introduction

> **STATUS:** outline only — drafted as part of the composition-guide
> structural pass. To be filled in.

## What this chapter covers

The framing for the rest of the guide. Three things:

1. **What "composition" means here.** Not "how to write *one* institution"
   (the per-host chapters cover that) but how *several* institutions
   share data, dispatch, and verdicts through the chain. The platform
   has WASM-hosted institutions and substrate-hosted institutions;
   composition is host-independent and works across the boundary.

2. **The three layers composition operates at.** Each chapter that
   follows lives at one of these layers:
   - **Shared payload language** (chapter 2) — a typed value shape
     multiple institutions consume directly. `formulas:FormulaTerm` in
     v1; the kernel's inductive-types machinery makes the principle
     general.
   - **Declared comorphisms** (chapter 3) — chain-resident bridge
     declarations that translate from one institution's vocabulary into
     another's, with the kernel statically type-checking the alignment.
   - **Coordinated dispatch roles** (chapter 4) — AutoOnLoad gates,
     OnDemand FIBER calls, and Decidable predicates working together.

3. **The kinase-institutions notebook as the running example.** Five
   institutions, three comorphisms, two storylines. Every subsequent
   chapter cites specific cells from this notebook. Read at least the
   notebook's top-level overview before continuing.

## Section outline

- **§1.1.** What this guide is for
- **§1.2.** The three layers of composition
- **§1.3.** The kinase-institutions notebook at a glance
- **§1.4.** Surface vocabulary you should already have
- **§1.5.** What this guide is *not* for (mirrors the README list).
  Includes a one-line forward-pointer to [§3.9](03-comorphisms.md) for
  readers looking for the theoretical lineage — institution theory's
  set + model-theoretic origins, Eigenius's constructive realisation,
  and the open research direction of formalising the translation.

---

Next: **[2. Shared payload languages →](02-shared-payload-languages.md)**
