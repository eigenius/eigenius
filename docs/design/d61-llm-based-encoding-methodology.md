# D61 — Faithful encoding of reasoning: the faithfulness gap and the two oracles

*Status: **partly withdrawn `2026-09-03`** · design specification.*

*This document specified two things: the **faithfulness boundary** — what an LLM-proposed
encoding does and does not establish — and a **typed decision layer** extending the `objective:`
ontology. The second is gone: `ontologies/objective/`, D58, and the decision layer's sections
(§3–§6, §8) were removed on `2026-09-03` because nothing exercised them — no runtime path, one
test that only checked they compiled, three gate queries nothing ran, and a central premise
(`acceptance_grade` over the four-value epistemic lattice) that P5 had already invalidated by
deleting the lattice.*

*What remains is what D62, D63, D64, D71 and D18 cite this document for: the faithfulness gap
(§1), the two oracles (§2), the grading rule for a grounding verdict (§7), the survey (§9) and
the prior art (§10). Section numbering is unchanged so those citations still resolve —
`D61 §10` still means the prior-art section.*

*Companion documents: [D39 justification
logic](d39-justification-logic.md) + [D49 ChainWitness](d49-chainwitness-machinery.md)
(certificates / witnesses — the answer-as-witness backbone), [D59 EigenQL array patterns &
derived joins](d59-eigenql-array-patterns-and-derived-joins.md) (the gate-query machinery),
[D43 retrieval](d43-text-and-vector-retrieval.md) (discovery), [D32 FormulaTerm](d32-chain-mirrored-mini-tt-inductives.md)
+ [D47 EigenTT fragment](d47-chain-mirrored-eigentt-type-fragment.md) (the content notation),
[D57 schema.org mapping](d57-schema-org-vocabulary-mapping.md) (the dogfood this is harvested
from + validated against), [D30 Eigon→Lean](d30-eigon-to-lean-faithful-translation.md) /
[D28 Lean institution](d28-lean-4-as-institution.md) (the proof-carrying ideal, §8). Operationalized
by the `reasoning` / `grounding` skills.*

---

## 1. Motivation — the faithfulness gap

**The faithfulness gap.** Across the LLM-formalization literature (§9) the pattern is *generate
→ check against a formal oracle → refine*, but **the oracle proves structural/logical validity,
never that the formalization captures intent.** *Checker-passing ≠ faithful*: an
autoformalization pipeline whose back-translation LLM-judge rated it ~97 % accurate was ~66 % on
human review (~34.8 % end-to-end); even human-written formal statements carry 16.4 %/38.5 %
semantic errors (§9).

**In Eigenius terms.** The kernel commit gate is **oracle #1**: a `reasoning:ReasoningSentence`
`Holds` iff its certificate type-checks against an admitted witness (D39/D49). That proves the
claim *follows from admitted evidence* — not that it was grounded in the *right discovered fact*.

**D57 is the proof.** Its load-bearing call — #9: schema.org's `domainIncludes` is *advisory*, so
→ `core:recommends`, never the restrictive `core:domain` — was a **discovery failure**. The
conformance fact lived in schema.org's prose spec; it surfaced reactively, by a human `[steer]`,
cited only when asked (`d57-mapping-decisions.md`). Both encodings **type-check** — `core:domain`
is well-formed — so oracle #1 cannot distinguish the faithful one. Only the *discovered fact*
can.

**Why type-checking is not enough.** The gap is not that the encoding is malformed — both
encodings above are well-formed. It is that *what would refute this* stays prose. A free-text
falsifier is neither runnable nor checkable, so "is this decision grounded?" remains a judgement
rather than a query, and the judgement is made by the same process that produced the encoding.

The typed decision layer this document originally proposed as the answer was removed on
`2026-09-03` (see the status note above) — it was never exercised. The gap it named is real and
open; what closes it is undecided.

## 2. Thesis and the two oracles

The goal is **not** a new "encoding" discipline. Eigenius already encodes reasoning (the
`reasoning` protocol). The lever is **doing `grounding` better — including the *discovery* of the
material the reasoning rests on** — and **typing the content of that discovery so it is checkable.**

> **Thesis.** *Reasoning is faithfully encoded only when it rests on properly **discovered**
> grounding.* Make discovery a first-class, **gated** phase whose targets and outcomes are
> **typed, runnable content** — so a conclusion cannot commit while its grounding is undiscovered,
> and "is this grounded?" is a query.

| Oracle | Question | Mechanism | Status |
|---|---|---|---|
| **#1 structural** | does the claim follow from admitted evidence? | the kernel commit gate (D39/D49) | exists |
| **#2 grounding** | is every load-bearing claim grounded in *discovered* fact? | undecided (the typed decision layer proposed here was withdrawn) | the **Discovered** gate (§6) + the back-stop check (§7) | this doc |

Oracle #2 is **purely additive** — it never weakens or routes around the kernel gate.

## 7. Grading the grounding verdict

A grounding check that passes is **Derived** (a program computed a CQ pass-rate / a back-translation
score) — **never auto-Verified.** The LLM-judge inflates (§9); the strongest *automatic* grade is
Derived. Only a **human spot-check** or a **proof-level correspondence** elevates toward
Verified. (A check grounded purely by an EigenQL query returning an expected answer is
**Verified-by-check** — a kernel-evaluated query over the real output, not an LLM judgement. An
LLM back-stop is the Derived case.)

## 9. Explore further
- **Lean correspondence** — lift a stable, proof-relevant typed core to Lean (the Lai et al.
  queries-as-types/answers-as-proof-terms ideal at full strength; D28/D30/D40). Research-grade.
- **The prose→trees encoding engine** — schema-constrained extraction (SPIRES-style) extending
  D8 `CompleteJson` against a typed contract — **now designed in
  [D62](d62-encoding-engine-prose-to-trees.md)** (the generation front-end this check guards;
  realized as an on-demand *institution*).
- **Encoding/grounding benchmarking** extending D50 (Text2KGBench-style conformance/hallucination
  metrics).

## 10. Prior art (primary-read & verified)

Read from primaries during this design (the `grounding` discipline applied to this doc); reading
them **corrected the secondary survey** — corrections noted. Full entries →
`docs/references/eigenius_related_work.bib`.

**Dependent type theory as a KG/ontology substrate** (→ D30/D39, D18)
- Cooper, R. *From Perception to Communication: A Theory of Types for Action and Meaning.* OUP,
  2023 (open access; DOI 10.1093/oso/9780192871312.001.0001). **TTR** — records-first: a record
  type *is* the Class-as-record-signature (`requires`/`recommends` = fields; `Resource` = record =
  witness); types not possible worlds; first-class types + reflection. The closest external
  substrate match (D62 §3). Primary-read.
- Lai, Z.; Ng, A. B.; Wong, L. Z.; See, S.; Lin, S. *Dependently Typed Knowledge Graphs.*
  arXiv:2003.03785 (2020). RDF + SPARQL in CIC/Coq; *"explainability in answers to queries through
  witnesses … compositionality and automation in the construction of witnesses"*; explicitly *"a
  proof of concept."* The precedent for reusing the reasoning stack rather than paralleling it.
- Barlatier, P.; Dapoigny, R. *A type-theoretical approach for ontologies: the case of roles.*
  Applied Ontology 7(3), 2012 (DOI 10.3233/AO-2012-0113); *Modeling Contexts with Dependent
  Types*, Fundamenta Informaticae, 2010. CIC/CCω + Dependent Record Types. *(Correction: drop the
  secondary "SUMO→GF" detail — not theirs.)*
- Luo, Z. *Common Nouns as Types*, LACL 2012 (DOI 10.1007/978-3-642-31262-5_12); *Formal Semantics
  in Modern Type Theories with Coercive Subtyping*, Linguistics & Philosophy, 2012. Types +
  coercive subtyping for subsumption — the treatment D18 parallels.
- Chatzikyriakidis, S.; Luo, Z. *Formal Semantics in Modern Type Theories.* ISTE/Wiley, 2020
  (DOI 10.1002/9781119489252). The comprehensive MTT-semantics reference: dependent types +
  coercive subtyping, **both model- and proof-theoretic**, Coq-verified NL semantics, dependent
  event types; impredicative `Prop` ≈ Eigenius D46. (Main chapters paywalled; characterized from
  abstract + TOC + free appendices.)

**LLM ontology learning / typed KG construction** (→ D50, D8)
- Mihindukulasooriya, N.; Tiwari, S.; Enguix, C. F.; Lata, K. *Text2KGBench.* arXiv:2308.02357
  (2023). Ontology-conformance + subject/relation/object hallucination metrics.
- Babaei Giglou, H.; D'Souza, J.; Auer, S. *LLMs4OL.* ISWC 2023, arXiv:2307.16648. Term typing /
  taxonomy / non-taxonomic relations; foundational LLMs alone insufficient.
- Caufield, J. H.; et al. *SPIRES: … populating knowledge bases using zero-shot learning.*
  Bioinformatics 40(3), btae104 (2024). LinkML-schema-constrained recursive extraction grounded to
  ontology IDs.

**The faithfulness gap** (→ this doc, D30/D39)
- Gao, G.; et al. *Herald: A Natural Language Annotated Lean 4 Dataset.* arXiv:2410.10878, ICLR
  2025. Back-translation + LLM-judge as the faithfulness check.
- Ospanov, A.; Farnia, F.; Yousefzadeh, R. *miniF2F-Lean Revisited.* NeurIPS 2025, arXiv:2511.03108.
  Herald **~97 % (LLM-judge) → ~66 % (human)**, **~34.8 % end-to-end**. *(Correction: this audit's
  figures, not Herald's own claims.)*
- Chen, G.; et al. *ReForm: Reflective Autoformalization …* ICLR 2026, arXiv:2510.24592. **16.4 % /
  38.5 %** semantic errors in *human-written* miniF2F / ProofNet statements; LLM-judge ceiling
  **85.8 %**. *(Correction: human-statement error rates, not autoformalizer output.)*

## 11. Out of scope
- A production RDF↔CIC toolchain or HoTT-on-KG (research-grade; §9's Lean correspondence is the
  stepping stone).
- Anything that weakens or routes around the kernel commit gate — oracle #2 is additive.
- schema.org mapping rules (D57).
```
