# D61 — LLM-based encoding methodology + a mechanized faithfulness oracle

## Context

`docs/notes/llm-based-encoding.md` is an untracked deep-research survey of the SOTA for
turning informal thought into formal typed knowledge (LLM ontology learning / SPIRES /
Text2KGBench; neural autoformalization into Lean/Coq; NL→FOL semantic parsing; and the
RDF↔dependent-type bridge, with Lai et al. 2020 as the closest precedent). The user asked
what to **incorporate** and what to **explore further**.

The survey's single decisive lesson is the **faithfulness gap**: across all three streams
the winning pattern is *generate with an LLM → check against a formal oracle → refine*, but
**the oracle guarantees structural/logical validity, NOT semantic faithfulness to the
original intent.** The evidence is blunt — Herald reports 96.7% by an LLM-judge vs **66%**
on human review (34.8% end-to-end), and even human "ground-truth" formal statements carry
16.4%–38.5% semantic errors (ReForm). *Checker-passing ≠ faithful.*

This maps directly onto Eigenius. The kernel's AutoOnLoad commit gate is **oracle #1**: a
`reasoning:ReasoningSentence` commits (`Holds`) iff its certificate type-checks against an
admitted witness. That proves the claim *follows from admitted evidence* — it does **not**
prove the encoded proposition faithfully captures the informal intent of the source it came
from. Eigenius today has no **oracle #2** (semantic faithfulness). The user chose the **full
build**: a complete, mechanized faithfulness oracle, plus a design doc (D61) capturing the
methodology, plus adopt-now edits and verified prior-art anchors.

What is **already done** (don't redo): the WRN reorg (merged, #89) and schema.org import
(#90) are on `main`; the skills already carry the D57/D58 *process* disciplines —
`reasoning.md` discipline 5 (proactive cite) & 6 (refine-don't-retrofit), `grounding.md`
discipline 7 (spec-over-data). The faithfulness gap and the two-oracle model are **absent**
from the skills — that is the new material here.

Outcome: Eigenius gains a typed, mechanized semantic-faithfulness check for any LLM-based
encoding; a faithfulness verdict that grades **Derived** (computed), never auto-Verified;
and a design doc that anchors the whole approach in the real prior art.

## Guiding design decisions (the spine of the build)

- **Two oracles, both grade their result, neither alone is sufficient.** Oracle #1
  (structural, the kernel gate — exists). Oracle #2 (semantic faithfulness — new). Oracle #2
  is **purely additive**; it never weakens or routes around the kernel gate.
- **A passing faithfulness check is `Derived`, not `Verified`.** A program computed a
  CQ-pass rate + a back-translation similarity; per the survey the LLM-judge is inflated, so
  the strongest *automatic* grade is Derived. Only a **human spot-check** or a **Lean-proof
  correspondence** elevates toward Verified. Encoding this honestly in the grade stack is the
  structural point — mechanization is a *screen that surfaces residual for review*, not a
  proof of meaning.
- **Reuse, don't mint parallel structure.** A competency question is a specialization of the
  objective ontology's existing `falsifier` / `wk_query` mechanism, grading via the existing
  `reflection:EpistemicStatus` enum — not a new epistemics. The back-translation check is a
  sibling of the existing `complete_json.ts` component, run **through the kernel** (D56
  component execution → `ProgramTrace → IsDerivedAs`) so its verdict is a first-class witness
  on the chain, not an out-of-band script.
- **Dogfood it on itself.** The whole effort is framed as an Eigenius objective (D58) and the
  faithfulness oracle is run against a real encoding (the D57 schema-org chain, and the
  encoding of `llm-based-encoding.md`'s own claims as anchors).

## Build plan (multi-session; phased)

### Phase 0 — Frame the effort as an objective (dogfood D58)
Create the objective branch and commit the obligation graph: thesis *"Eigenius has a
mechanized semantic-faithfulness oracle (oracle #2) for LLM-based encoding, and
checker-passing ≠ faithful is a recorded, addressed gap"*, decomposed into the milestones
below, each with `acceptance_grade` + `witness_kind` + `falsifier`. Axioms = the verified
prior-art citations (Phase 1). This makes the rest of the work fail-closed against its own
frame, and exercises D58 at the scale it was designed for.

### Phase 1 — D61 design doc + verified prior-art anchors
- **New `docs/design/d61-llm-based-encoding-and-faithfulness.md`** — sections:
  (1) the faithfulness gap, stated in Eigenius terms (oracle #1 vs #2);
  (2) the two-oracle methodology and the four-grade mapping (faithfulness verdict = Derived);
  (3) the LLM-based-encoding pipeline (informal source → schema-constrained typed encoding →
  oracle #1 commit → oracle #2 faithfulness → human residual), positioned against the three
  research streams (LLM ontology learning, neural autoformalization, NL→FOL parsing);
  (4) the mechanized oracle #2 spec (CQ battery, back-translation component, multi-candidate
  scoring, human-review surface — Phases 2–5);
  (5) prior art + (6) explore-further + (7) out-of-scope.
- **Verify every citation resolves, then anchor** (grounding discipline — never fabricate;
  the .bib header already mandates a provenance pass). Add to
  `docs/references/eigenius_related_work.bib`: Lai et al. (arXiv:2003.03785), Dapoigny &
  Barlatier, Luo (coercive subtyping), Text2KGBench (arXiv:2308.02357), LLMs4OL
  (arXiv:2307.16648), SPIRES/OntoGPT (Caufield et al., *Bioinformatics* 2024, btae104),
  Herald (ICLR 2025), miniF2F-Lean Revisited (arXiv:2511.03108), ReForm (arXiv:2510.24592),
  Li et al. (arXiv:2410.20936), Draft-Sketch-Prove (Jiang et al., ICLR 2023). Any that can't
  be verified get the existing `note`-flag convention and are **not** used as load-bearing
  anchors.
- **Graft anchors into the docs they support** (one-paragraph "prior art" notes, not
  rewrites): Lai et al. → D30 / D39 / D28 (queries-as-types, answers-as-witnesses ≈ the
  certificate/witness model); Dapoigny & Barlatier + Luo → D18 (ontology-as-types); Text2KG /
  LLMs4OL / SPIRES → D50 + D8.
- Add D61 to `docs/design/README.md`.

### Phase 2 — Competency-question battery (oracle #2, mechanism 1)
- Add `objective:CompetencyQuestion` to `ontologies/objective/objective-ontology.esl`: a
  typed falsifier carrying the NL question, an EigenQL query, the expected answer/predicate,
  and the source-intent it probes. Witnessed via the existing `objective:wk_query`. Add to
  the bootstrap/load set.
- Build a **CQ-runner**: executes the battery against an encoded graph, emits per-CQ
  pass/fail + an aggregate; a CQ that doesn't return its expected answer is a **fail-closed
  finding** (the encoding missed that intent). The battery is an obligation sub-graph: "E is
  faithful to S" ⇒ "every CQ over E returns its expected answer."
- Dogfood: author a CQ battery for the D57 schema-org chain; run it.

### Phase 3 — Back-translation faithfulness component (oracle #2, mechanism 2)
- New orchestrator component `orchestration/src/components/faithfulness_check.ts` (sibling of
  `complete_json.ts`): back-translates an encoded fragment to NL via an LLM completion, scores
  semantic consistency against the original source (embedding similarity + LLM-judge).
- Run it **through the kernel** (D56 component execution; D60 `oci` runtime for any pinned
  tooling) so it emits a `DerivedResource` with `canonical_proposition =
  EncodingIsFaithful(source_id)` **only** when the score clears a *declared* threshold; below
  threshold it **Fails closed** and is recorded as a finding. The verdict commits as a
  Derived witness (`ProgramTrace → IsDerivedAs`).

### Phase 4 — Multi-candidate scoring + human-review surface (oracle #2, mechanisms 3–4)
- Sample *k* independent encodings of one source (vary encoder prompt/seed; orchestrate as a
  parallel fan-out); rank by CQ-pass-rate + back-translation similarity. Inter-candidate
  agreement is itself a faithfulness signal (disagreement ⇒ ambiguous source / unstable
  encoding).
- **Human-review surface**: present the residual — CQ failures, low-similarity fragments,
  inter-candidate disagreements — for adjudication. A human sign-off is recorded as a
  Declared/Citation witness (or a ReasoningSentence) that **elevates the faithfulness grade
  from Derived toward Verified**. This is the non-negotiable step the survey insists on.

### Phase 5 — Adopt-now edits + audit
- `.claude/skills/reasoning.md`: add the faithfulness caveat to the epistemic contract (a
  `Holds` = oracle #1 structural validity, **not** faithfulness → name oracle #2); add a
  discipline ("Checker-passing ≠ faithful — verify intent, not just type; a mechanized
  faithfulness check is Derived, not Verified, until a human or proof confirms it"); extend
  the Audit step (§6) with a faithfulness pass (run the CQ battery).
- `.claude/skills/grounding.md`: add **competency-question-driven design** to the Anchor/frame
  loop (CQs define what an encoding must answer *before* extraction — OntoChat/NeOn-GPT/RevOnt;
  ties to D58).
- `.claude/skills/eigenius.md`: document the new mechanics (run the CQ battery; run the
  faithfulness component).
- D58 clarifying note: a `CompetencyQuestion` is the concrete `wk_query` falsifier for a
  Milestone; "encoding E faithful to source S" is itself an objective sub-graph.
- Audit the whole effort's objective chain (Phase 0) — every milestone resolves to a witness;
  every anchor is a real, verified citation.

## Explore further (documented in D61, NOT built this round)
- **Lean-correspondence oracle #3**: lift a stable, proof-relevant typed core to Lean per Lai
  et al. (queries-as-types, answers-as-witnesses); the Lean type checker becomes the oracle
  for the lifted fragment (D28/D30/D40). Research-grade.
- **Schema-constrained extraction pipeline** extending D8 `CompleteJson` (SPIRES-style
  recursive entity-then-relation extraction; constrained decoding; RAG of schema fragments).
- **Encoding benchmarking** extending D50 (Text2KGBench-style metrics: ontology conformance,
  hallucination rate, CQ-pass on a held-out note set).

## Out of scope
- A production RDF↔CIC toolchain or HoTT-on-KG (the survey flags the bridge as research-grade;
  oracle #3 above is the documented-only stepping stone).
- Any change that weakens or routes around the kernel commit gate — oracle #2 is additive.

## Verification (dogfood, end-to-end)
1. `cargo build`, `cargo test --workspace`, `cargo fmt --all -- --check`,
   `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets` — clean; the ontology
   bootstrap test passes with the new `objective:CompetencyQuestion` class.
2. CQ battery runs against the D57 schema-org chain and returns expected answers (a planted
   wrong-answer CQ Fails closed → proves the falsifier actually falsifies).
3. The back-translation component commits a **Derived** `EncodingIsFaithful` verdict for a
   faithful fragment, and **Fails closed** (recorded finding) for a deliberately mis-encoded
   fragment.
4. Multi-candidate scoring produces ranked candidates + a disagreement set; the human-review
   surface lists the residual.
5. Every new `.bib` entry verified resolvable; every load-bearing D61 anchor is a real
   Citation on the chain.
6. The effort's own objective chain (Phase 0) audits clean — the methodology validated on
   itself.

## Critical files
- **NEW** `docs/design/d61-llm-based-encoding-and-faithfulness.md`; index it in
  `docs/design/README.md`.
- `ontologies/objective/objective-ontology.esl` (+ `CompetencyQuestion` class/properties; add
  to bootstrap).
- **NEW** `orchestration/src/components/faithfulness_check.ts` (pattern: `complete_json.ts`);
  the CQ-runner (EigenQL over the chain).
- `.claude/skills/{reasoning,grounding,eigenius}.md` (adopt-now disciplines + mechanics).
- Prior-art notes in `docs/design/{d18,d28,d30,d39,d50,d58}-*.md`.
- `docs/references/eigenius_related_work.bib` (verified anchors).
- `docs/notes/llm-based-encoding.md` (commit the research note that prompted this).
