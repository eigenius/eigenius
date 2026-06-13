# D54 — Reasoning Lemma Citation (sentence-as-lemma)

*Status: design memo · June 2026*

*Companion documents: [D39 justification logic](d39-justification-logic.md), [D46 Prop universe + proof irrelevance](d46-prop-universe-and-proof-irrelevance.md), [D47 chain-mirrored EigenTT type fragment](d47-chain-mirrored-eigentt-type-fragment.md), [D49 ChainWitness machinery](d49-chainwitness-machinery.md). Background: Artemov & Fitting, *Justification Logic* (2020), in `references/publications/`.*

*This memo specifies a small, foundational capability: letting a proven `ReasoningSentence` be cited as a **lemma** in a later sentence's justification, instead of re-inlining its sub-proof. It adds no logical power — inlining and lemma-citation are equivalent — but it is the prerequisite for layered proofs (lemmas → theorems) at any scale. It also fixes a concrete gap: the witness emitter does not currently admit a bare `ReasoningSentence`, so a sentence cannot cite another today. The memo's second half answers a deeper question raised in scoping — consistency checking is logic-dependent, so what does the institution's choice of justification logic actually give us?*

---

## 1. Motivation — the gap, and the concrete trigger

D49's `build_witness_index` admits an `IsDerivedAs` witness only two ways: from a *trace* (`DeclarationTrace`/`ObservationTrace`/`ProgramTrace`) pointing at a resource, or from an `InstitutionEmittedDerivation`-marked resource carrying `reflection:canonical_proposition`. A bare `reasoning:ReasoningSentence` has neither — its derivation requirement is satisfied by the certificate field, not a trace — and the `ValidateJustification` gate emits only a `Verdict`, stamping nothing on the sentence. So although `ReasoningSentence : reflection:DerivedResource` *intends* (per its class comment) that a later sentence's `DerivedEvidence(prior_iri)` resolve, in practice it does not: citing a prior conclusion fails with

> *no admitted IsDerivedAs witness for IRI `…` … must be committed with `reflection:canonical_proposition` matching the proposition (or the proposition must be `Asserts(<iri>)` — the default; the `Asserts` default lands in Phase 5b once D39's core-ontology `Asserts` class is authored).*

The trigger is the WRN encoding's `C-MAIN` capstone (`docs`: `experiments/publications/wrn-helicase/`). `C-MAIN` is the thesis `SyntheticLethal(WRN, MSI)`, reached by modus ponens over a Declared synthesis implication applied to five phase findings. Lacking lemma citation, `C-MAIN` discharges each antecedent by **inlining** that finding's leaf proof — correct, but it re-states ~10 sub-proofs. With this mechanism it would instead cite the five phase conclusions. The inline form is the valid interim and needs no rework once this lands; it just gets shorter.

## 2. What it is — and what it is not

**It is:** a committed, `Holds` `ReasoningSentence` S (proposition P, kernel-checked certificate) becomes citable as the fact P in a later sentence's justification term, via the existing evidence-atom surface.

**It is not new logical power.** In justification-logic terms (D39), a justification term composes by *application*: from `r : (P → G)` and `s : P` build `(r · s) : G`. A lemma citation just supplies `s : P` by reference to S instead of re-deriving it. Inlining S's proof and citing S are the same proof term modulo sharing — exactly Artemov's application closure. So this is a **structural sharing / DRY + readability** feature, not an extension of the calculus, the type checker, or `JustifiedBy`.

The boundary matters: the mechanism makes an *already-proven* proposition *referenceable*. It never makes anything provable that the leaf warrants didn't already establish.

## 3. Mechanics

Two localized pieces, plus one hard rule.

1. **Witness exposure.** S's proven proposition P must be readable as S's `reflection:canonical_proposition` (today S sets only `reasoning:proposition`). Options: (a) ESL/compile copies `reasoning:proposition → reflection:canonical_proposition` for `ReasoningSentence`; (b) the `ValidateJustification` gate stamps it at commit on `Holds`. (b) is preferable because it ties exposure to the gate (see the soundness rule).
2. **Witness admission.** `build_witness_index` gains one branch: a `ReasoningSentence` carrying a canonical proposition admits a witness keyed on **its own IRI** with P's hash — the analogue of `emit_from_institution_derivation`, for reasoning sentences. The consumer side (`JustifiedBy.derived`/`verified`) is unchanged.
3. **Soundness rule (hard).** Only a sentence that **passed its gate** may be admitted as a lemma. In the commit pipeline this is automatic — a `Fails` sentence is rejected and never committed, so every committed sentence `Holds`. The risk is paths that bypass the gate (building layers directly, as some tests do): admission must be gate-gated, not "any sentence resource on the layer." Tying exposure (piece 1b) to the gate enforces this by construction.

No change to the type checker, the `JustificationTerm` constructors, or the D47 codec.

## 4. Open design decisions

1. **Direct vs. `Asserts` wrapper.** Admit `IsDerivedAs(S, P)` — cite S, get P directly (what `C-MAIN` needs) — versus `IsDerivedAs(S, Asserts(S))`, a reification you then unwrap. Direct is simpler; the `Asserts` wrapper earns its place only if "asserted-by-a-sentence" must be provenance-distinguishable from "derived-by-an-institution" at the proposition level. **Recommendation: direct**, with `Asserts` reserved should provenance-typing of conclusions later prove necessary.
2. **Witness category: Derived vs. Verified.** A sentence proven by a kernel-checked certificate is arguably *Verified* (`IsVerifiedAs`), not *Derived*. The class comment says `DerivedEvidence`/`IsDerivedAs`; but a checked proof is closer in spirit to a Lean `VerifiedResource`. This is a real taxonomy call (it fixes which `JustifiedBy` constructor cites a conclusion) and interacts with factivity (§5). **Recommendation: revisit alongside §5's factivity assignment** — a factive, kernel-checked conclusion is most honestly *Verified*.
3. **All sentences vs. opt-in.** Is every `Holds` sentence a citable lemma (uniform — "a proven fact is a fact"), or only ones flagged? **Recommendation: uniform**; opt-in adds surface without a clear payoff.

## 5. Consistency, and what justification logic gives us

Consistency checking was raised in scoping as *logic-dependent* — it "depends on the underlying logic and thus the reasoning institution." That is exactly right, and it is why this memo **scopes consistency out** of the lemma mechanism while characterizing what the institution's logic affords. The lemma mechanism is pure term composition; it changes no consistency property. But because lemmas *chain*, the institution's logic choices become more consequential, so they are worth stating.

### 5.1 Two different "consistency" questions

- **Term-level (decidable, already done).** Every committed `t : F` is a valid proof — the per-sentence `ValidateJustification` gate. "The chain is locally consistent" = every certificate type-checks. The lemma mechanism preserves this exactly (a cited lemma's own certificate was checked).
- **Propositional consistency of the asserted set (the hard one).** Is `{ F : some committed t : F }` jointly satisfiable? This is the classical SAT/validity question, and it is where the underlying logic decides everything. The existing `qc_consistency_check` returns `Undecidable` for non-trivial input — and §5.3 shows that is *principled*, not merely unimplemented.

### 5.2 What justification logic specifically gives us

Justification logic (D39's basis) replaces modal □F ("F is provable/known") with explicit terms `t:F` ("t justifies F"), with application (`·`), sum (`+`, monotone evidence-combining), and a proof checker (`!`). Three consequences bear directly on consistency:

1. **Factivity is an explicit, tunable axis.** The axiom `t:F → F` (factivity) is what separates the family: with it, the logic is LP-like and realizes **S4** (justified ⇒ true); without it, the basic logic J realizes **K** (justified *belief*, not necessarily true). Eigenius can — and should — assign factivity **per evidence category**, which is the real payoff *here*:
   - `VerifiedEvidence` (a Lean/Coq proof): **factive** — `t:F → F` (veridical modulo the checker + axioms). LP-like.
   - `DerivedEvidence` (a recomputed statistic/program): **conditionally factive** — true relative to the *content-addressed* data + deterministic method; the conditioning is explicit, so it is strong but defeasible only if the pinned data is wrong.
   - `ObservedEvidence`: **defeasible** — an observation can be erroneous. J-like.
   - `DeclaredEvidence`: **explicitly non-factive** — `t:F` here means "F is *assumed*" (an axiom/rule/threshold the author declares); emphatically not `→ F`.
2. **Conflicting justifications need not explode.** With non-factive categories, `s:F` and `t:¬F` can coexist as *conflicting evidence* without collapsing to ⊥ — there is no global ex falso. (Artemov-Fitting devote a chapter to paraconsistency.) For a multi-agent knowledge graph this is a feature: conflicts are **localized and visible** (both justification terms are on the chain) rather than detonating the whole layer. Consistency becomes a *queryable property of a named set*, not a fragile global invariant — which is why `qc_consistency_check` takes an explicit set as input.
3. **Conflict is provenance-explainable.** Every `F` carries its term, which bottoms out in Declared/Observed/Derived/Verified atoms. A detected `s:F` vs `t:¬F` conflict is therefore *traceable to its evidence* and adjudicable (this is what `reflection:refutes` + belief revision act on). Classical SAT says only "unsat"; JL says *which warrants* collide and at which factivity grade — so adjudication can prefer Verified over Declared, recomputed-Derived over Observed, etc.

**Where the consistency exposure actually lives.** Because Observed/Derived/Verified are anchored to data and proofs, the dominant source of potential propositional inconsistency in an Eigenius chain is the **Declared layer** — the assumptions, domain rules, and thresholds. A consistency institution's real job is checking that the *declared* set does not jointly entail ⊥ (under whatever factivity the other categories carry). That reframes consistency from "scan everything" to "audit the assumptions," which is both more tractable and more meaningful.

### 5.3 The decidability boundary (why `Undecidable` is correct)

Artemov's **realization theorem** connects LP to S4: every S4 theorem has an LP realization (□ replaced by explicit terms) and every LP theorem forgets to an S4 theorem. So propositional consistency of a JL-asserted set reduces to the modal consistency of its forgetful projection:

- **Propositional fragment:** decidable (S4-SAT is PSPACE-complete). A consistency institution *can* decide this sub-case — the natural v1+ target.
- **First-order fragment (quantifiers):** D39's `SpecStr` is ∀-instantiation; once propositions quantify, the relevant system is **FOLP/FOS4**, and consistency is **undecidable** in general (first-order modal logic is). So `qc_consistency_check` returning `Undecidable` for non-trivial (quantified) input is the *correct* answer, not a stub — it is honestly reporting the boundary the logic imposes.

So the institution's logic choice is not cosmetic: it fixes (a) what inconsistency *means* (via per-category factivity), (b) whether conflicts explode or localize (factive vs. paraconsistent), and (c) what a checker can *decide* (propositional yes, first-order no). A future consistency institution should therefore be explicit about its factivity assignment and advertise decidability only on the propositional fragment.

## 6. Out of scope

- **Consistency / contradiction checking** itself (§5 characterizes it; the lemma mechanism does not implement it). That is a separate institution, gated by the factivity/decidability analysis above.
- **Cross-institution citation** — statistics results, Lean proofs, etc. already have their own `IsDerivedAs`/`IsVerifiedAs` paths; this is sentence-cites-sentence within reasoning.
- **Belief revision / refutation** — `reflection:refutes` is a separate marker; lemma citation does not supersede prior sentences.
- **The leaf warrants** — a lemma's own proof still rests on its admitted evidence.

## 7. Footprint

One admission branch in `build_witness_index` (kernel), one stamping point in the `ValidateJustification` gate (reasoning) to expose the proven proposition as `reflection:canonical_proposition`, the gate-gated soundness rule, and tests (including a sentence-cites-sentence case and a negative test that a `Fails`/ungated sentence is *not* admitted). No type-checker, codec, or constructor changes. Once landed, the WRN `C-MAIN` certificate collapses from inlined leaf proofs to five lemma citations, and layered proof — the lemmas → theorems pattern the platform needs everywhere — becomes available.
