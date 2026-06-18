# D54 — Reasoning Lemma Citation (sentence-as-lemma)

*Status: design memo · **implemented** June 2026*

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

## 3. Mechanics — *implemented*

The realization turned out to need **one** localized change, not two, because the soundness already lives at the commit boundary (see below).

1. **Witness admission (the whole change).** `build_witness_index` gains one branch (`kernel/src/layer/witness_index.rs`): a resource whose `is_a` includes `reasoning:ReasoningSentence` admits a witness keyed on **its own IRI** with the hash of its `reasoning:proposition` — emitting the **`Verified`** category (§4.2), via a helper `emit_from_reasoning_sentence` mirroring `emit_from_institution_derivation`. The proposition is read straight off the sentence (no separate `canonical_proposition` stamping needed); the D47 `Value::Json` hashes identically to the consumer's `verified(iri, P)` term, so the key matches. The consumer side (`JustifiedBy.verified`, and `.derived` via the existing `IsVerifiedAs → IsDerivedAs` coercion) is unchanged.
2. **Soundness — at the commit boundary, not in the index.** Only a sentence that **passed its `ValidateJustification` gate** is on chain: the commit pipeline (`autoonload_dispatch`, `kernel/src/commit/phases.rs`) rejects `Fails` verdicts, so any *committed* `ReasoningSentence` `Holds`. `build_witness_index` therefore **trusts committed resources** — exactly the model that already lets it admit `InstitutionEmittedDerivation`s without re-running their institution. No gate-stamping or `canonical_proposition`-copy is required; the witness reads the sentence's `proposition` directly and relies on commit-rejects-`Fails` for soundness. (The only way to admit an un-gated sentence is to bypass the commit pipeline with raw `LayerBuilder` — a test/internal construction primitive, never a production commit path.)

This is *purely additive*: a certificate that cites no sentence is unaffected (the extra `Verified` self-witnesses sit unread in the index). No change to the type checker, the `JustificationTerm` constructors, the D47 codec, the reasoning gate, or the `ReasoningSentence` / `Verdict` class definitions.

> **Note on the earlier draft.** A previous version of this section preferred a "gate stamps `canonical_proposition` at commit" exposure. That was dropped: the WRN reasoning tests (and any direct `LayerBuilder` use) never run the commit gate, so a gate-stamp would be untestable there; and it is unnecessary, since soundness already holds at the commit boundary. Reading `reasoning:proposition` directly is simpler and works in every harness.

## 4. Open design decisions

### 4.1 How the cited proposition is supplied — the real axis - Resolved

The choice is usually framed as "direct `IsDerivedAs(S, P)` vs. `IsDerivedAs(S, Asserts(S))`," but that is surface syntax. The decision underneath is:

> **Does the citer restate P (so the kernel works only with hashes), or does the kernel resolve S to recover P (so type-checking dereferences cited content)?**

**Form A — citer restates P.** S's `canonical_proposition` *is* P (the domain proposition). `build_witness_index` reads it once at emission and emits the key `(Verified, iri=S, prop_hash = hash(P))` (category per §4.2). A citation writes P explicitly:

```
verified("urn:…:concl_mmr", onco:ContributesToDependence("dMMR","WRN"))
```

The kernel forms `(S, hash(P))` and looks it up in the witness *index*. It **never reads `concl_mmr`'s content** — checking stays a hash-index lookup, the property the whole D49 design rests on. (A `derived(...)` citation also resolves, via the `IsVerifiedAs → IsDerivedAs` coercion — §4.2.) This axis is independent of the witness *category* (§4.2): Form A vs. B is about how the *proposition* is supplied, Verified vs. Derived about what the witness *certifies*.

**Form B — kernel resolves S (`IsDerivedAs(S, Asserts(S))`).** `Asserts : iri → Prop`; S's `canonical_proposition` defaults to `Asserts(S)` — keyed on the IRI, not on P's content. A citation names only the IRI:

```
derived("urn:…:concl_mmr", Asserts("urn:…:concl_mmr"))
```

You **never write P**. But the consumer site is typed at the actual proposition (e.g. a rule antecedent `ContributesToDependence("dMMR","WRN")`), so for modus ponens to fire the type-checker must know

```
Asserts("urn:…:concl_mmr") ≡ resolve(concl_mmr).proposition = ContributesToDependence("dMMR","WRN")
```

i.e. **`Asserts` must unfold by chain lookup during conversion.** Form B buys "don't restate P" at the price of making *definitional equality chain-aware* — conversion now resolves cited resources, coupling the type-checker to chain state (a real complication for the metatheory and for D46's conversion / proof-irrelevance story). A `lemma(S)` constructor whose typing rule looks up S's proposition is the same trade in different clothing: it too resolves S at check time.

**The "default = proposition" option.** The ergonomic goal behind `Asserts` — *sentences citable out of the box, without the author setting `canonical_proposition`* — does **not** require `Asserts`. Default a `ReasoningSentence`'s `canonical_proposition` to its own `proposition`. That yields citable-by-default **and** keeps Form A's hash-only checking; no chain-aware conversion. The phase conclusions then need no author change at all (see Appendix A).

The redundancy that motivates Form B — restating P at each citation — is already absorbed by the ESL `alias` construct: bind P once, reuse it in `derived(S, P)`. Drift is benign: a wrong restatement fails to type-check, never silently unsound.

**Recommendation: Form A, with `canonical_proposition` defaulting to `proposition`.** It preserves resolution-free, hash-index type-checking; needs no new definitional-equality machinery; and its only cost is one `alias`-bound proposition per citation. Reserve `Asserts` for if/when conclusions must be *provenance-typed* ("this fact is a lemma reference to S" readable in the proposition itself) — accepting that it pulls chain resolution into conversion. Appendix A is the worked Form-A `C-MAIN`.
### 4.2 Witness category — *resolved: Verified*

**Decision: the witness is `Verified` (`IsVerifiedAs(S, P)`), cited via `VerifiedEvidence` / `JustifiedBy.verified`.** A `ReasoningSentence` that `Holds` did so because the kernel **proof-checked its `JustifiedBy` certificate against the proposition** — the certificate *is* a proof term, the gate *is* a checker. That is a verification event, structurally the same as nanoda checking a Lean proof, and categorically distinct from `Derived` (a reproducible *computation* trace). The decisive marker is not "there is a witness on chain" — every category has one — but *what the witness certifies*: a checked proof, not a recomputation. This also lines up with factivity (§5.2): `Verified` is the factive category; a conclusion proven from its leaves is factive *relative to its declared-assumption base*, exactly as a Lean proof is factive relative to its axiom base.

**Bonus — it is strictly dominant.** `build_witness_index` already carries an `IsVerifiedAs → IsDerivedAs` coercion (a `.derived` citation succeeds when a matching `Verified` entry exists at the same key). So emitting `Verified` satisfies **both** `verified(S, P)` *and* `derived(S, P)` citations — a proof legitimately standing in for a derivation citation. There is no downside to choosing the stronger category.

**Caveat — witness category vs. resource *class* are decoupled here.** `reflection:VerifiedResource` is `subclass_of DerivedResource` and `requires [derivation, verification]` — i.e. it is shaped for *external-prover* proofs (a `ProgramTrace` + a `VerificationTrace`/`VerifiedPropositionView`, the Lean comorphism path). A `ReasoningSentence` has **neither** trace: its certificate is an inline field ("the certificate IS the derivation"). So the class **cannot** simply become `reflection:VerifiedResource` without acquiring trace requirements it shouldn't have. Resolution: keep `ReasoningSentence : reflection:DerivedResource` (true, since `VerifiedResource ⊑ DerivedResource`) and have the new admission branch (§3.2) emit a **`Verified`** witness from the *certificate-check* directly — recognizing **in-chain-certificate verification** as a verification modality distinct from external-prover `VerificationTrace`. This is the one structural detail to settle at build time; it surfaces that the reflection ontology currently models verification only via external-prover traces, and a reasoning conclusion is a second, self-contained kind. (Generalizing `VerifiedResource` to admit certificate-verified resources is the alternative; the decoupled-admission route is lighter and keeps the external-prover shape intact.)

### 4.3 What counts as a citable lemma (and what does not) - Resolved

**"`Holds` sentence" means a `reasoning:ReasoningSentence` that passed its `ValidateJustification` gate** (hence was committed); the cited proposition is its `proposition`. **Recommendation: uniform** — every committed `ReasoningSentence` is a lemma; opt-in adds surface without a clear payoff.

**Institution `Verdict`s are *not* lemmas.** Under the D52 **verdict-vs-derivation split**, a `Verdict` (the `Holds`/`Fails`/`Undecidable` output of *any* gate, reasoning included) carries **no `canonical_proposition`** — it attests "the gate ran and reached this verdict," not a proposition. `emit_from_institution_derivation` reads `canonical_proposition`; the verdict resource has none, so there is nothing to cite. Note the parallel: a `ReasoningSentence`'s own gate also emits a Verdict, and that Verdict is *not* the citable thing — the **sentence** (which bears the proposition) is. Uniformly: cite the proposition-bearer, never the Verdict.

**The principle: lemma-citability ⇔ proposition-bearing + kernel-warranted.** That sorts the three kinds of gate/institution output:

| Output | Carries a proposition? | Citable? | Via |
|---|---|---|---|
| `Verdict` (any institution) | No (D52 split) | **No** | — |
| `InstitutionEmittedDerivation` (e.g. `StatisticalAnalysisResult`) | Yes (`canonical_proposition`) | Yes — *already* | `IsDerivedAs` (existing) |
| `ReasoningSentence` (`Holds`) | Yes (`proposition`) | Yes — *new* | `IsVerifiedAs` (§4.2, this memo) |

This mechanism adds only the third row: institution **derivations** were already citable via the existing `IsDerivedAs` path (the WRN statistics results C-WRN/D-REFINE/D-RECQ/D-BIOM cite exactly these), and `Verdict`s never are.

**Out of scope: reasoning *about* verdicts.** Asserting "institution X approved subject Y" as a proposition (e.g. `Approved(Y)`) would require *proposition-ifying* verdicts — giving the `Verdict` a `canonical_proposition` — which is a distinct feature, not the lemma mechanism. Verdicts stay operational, not propositional.

## 5. Consistency, and what justification logic gives us

Consistency checking was raised in scoping as *logic-dependent* — it "depends on the underlying logic and thus the reasoning institution." That is exactly right, and it is why this memo **scopes consistency out** of the lemma mechanism while characterizing what the institution's logic affords. The lemma mechanism is pure term composition; it changes no consistency property. But because lemmas *chain*, the institution's logic choices become more consequential, so they are worth stating.

### 5.1 Two different "consistency" questions

- **Term-level (decidable, already done).** Every committed `t : F` is a valid proof — the per-sentence `ValidateJustification` gate. "The chain is locally consistent" = every certificate type-checks. The lemma mechanism preserves this exactly (a cited lemma's own certificate was checked).
- **Propositional consistency of the asserted set (the hard one).** Is `{ F : some committed t : F }` jointly satisfiable? This is the classical SAT/validity question, and it is where the underlying logic decides everything. The existing `qc_consistency_check` returns `Undecidable` for non-trivial input — and §5.3 shows that is *principled*, not merely unimplemented.

### 5.2 What justification logic specifically gives us

Justification logic (D39's basis) replaces modal □F ("F is provable/known") with explicit terms `t:F` ("t justifies F"), with application (`·`), sum (`+`, monotone evidence-combining), and a proof checker (`!`). Three consequences bear directly on consistency:

1. **Factivity is an explicit, tunable axis.** The axiom `t:F → F` (factivity) is what separates the family: with it, the logic is LP-like and realizes **S4** (justified ⇒ true); without it, the basic logic J realizes **K** (justified *belief*, not necessarily true). Eigenius can — and should — assign factivity **per evidence category**, which is the real payoff *here*:
   - `VerifiedEvidence` (a Lean/Coq proof, **or — per §4.2 — a kernel-checked `ReasoningSentence`**): **factive** — `t:F → F` (veridical modulo the checker + axioms; for a reasoning conclusion, modulo its *declared-assumption* base). LP-like.
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

## 7. Footprint — *as built*

**One branch + one helper** in `build_witness_index` (`kernel/src/layer/witness_index.rs`, ~25 lines): admit a `reasoning:ReasoningSentence` as a `Verified` witness keyed on its IRI, reading `reasoning:proposition`. Soundness is the existing commit-rejects-`Fails` boundary (§3.2) — no gate change, no `canonical_proposition` stamping. **No** changes to the type checker, NbE, the D47 codec, the `JustificationTerm` constructors, the reasoning gate, or the `ReasoningSentence` / `Verdict` classes.

**Verified end-to-end:** the WRN `C-MAIN` was converted from inlined leaf proofs to five lemma citations (`verified(concl_X, P)`) and still type-checks to `Holds` (`crates/eigenius-reasoning/tests/wrn_phase5.rs::wrn_phase5_cmmr_and_cmain_validate`); kernel, reasoning, and statistics suites stay green; the change is additive (certificates citing no sentence are unaffected). Appendix A is now the *live* `C-MAIN`, not a sketch. Layered proof — the lemmas → theorems pattern — is available platform-wide.

---

## Appendix A — Worked example: Form A on the WRN `C-MAIN`

The capstone `SyntheticLethal(WRN, MSI)` is reached by modus ponens over the
Declared synthesis implication applied to the five phase findings (C-VAL,
C-VIVO, D-HELICASE, C-MECH, C-MMR). **This is now the live `C-MAIN`** (the
mechanism is built; `wrn_phase5.rs` validates it) — Form A, lemma citations.

**Category note (§4.2).** A lemma citation of a conclusion is a `Verified`
witness, so the canonical constructors are `VerifiedEvidence(concl_X)` /
`verified(concl_X, P)`. The focal snippet in A.2 shows that resolved form;
the full listing in A.3 is written with `DerivedEvidence` / `derived` for
readability — those *also* type-check, via the `IsVerifiedAs → IsDerivedAs`
coercion, so both spellings are valid. Citations of *artifacts* (ToolArtifacts
in the "inline" comparison) stay `Derived` — those genuinely are.

### A.1 Conclusion side — no author change

With `canonical_proposition` defaulting to `proposition` (§4.1), each phase
conclusion is citable as-is; the author writes nothing extra:

```
resource wrn:concl_mmr : reasoning:ReasoningSentence {
    reasoning:subject_iri = "urn:eigenius:pub:wrn:gene_WRN";
    reasoning:proposition = type_expr(onco:ContributesToDependence("dMMR", "WRN"));
    // canonical_proposition defaults to the proposition above ⇒
    //   build_witness_index emits:
    //   IsDerivedAs(concl_mmr, ContributesToDependence("dMMR","WRN"))
    reasoning:justification = App(DeclaredEvidence("…:mmr_rule"),
                                  DerivedEvidence("…:mmr_restoration"));
    reasoning:certificate   = type_expr( /* unchanged C-MMR cert */ );
}
```

(Without the default, the only addition per conclusion is one line:
`reflection:canonical_proposition = type_expr(onco:ContributesToDependence("dMMR","WRN"));`.)

### A.2 The per-antecedent collapse

Each antecedent's proof drops from an inlined re-derivation to a one-line
lemma citation. The helicase antecedent, **inline (today)**:

```
// justification:  App(DeclaredEvidence("…:helicase_rule"), DerivedEvidence("…:va_fail_k577m"))
// certificate:
app(FR, RA, DeclaredEvidence("…:helicase_rule"), DerivedEvidence("…:va_fail_k577m"),
    declared("…:helicase_rule", FR -> RA), derived("…:va_fail_k577m", FR))
```

**Form A (cite the lemma — Verified, §4.2):**

```
// justification:  VerifiedEvidence("…:concl_helicase_required")
// certificate:    verified("…:concl_helicase_required", RA)
```

One atom instead of an `app` node + the `FR -> RA` implication + the artifact
reference. The only restated proposition is `RA`, `alias`-bound once. (`derived(…)`
would also resolve, via the coercion — §4.2.)

### A.3 Full Form-A `C-MAIN`

```
resource wrn:concl_main : reasoning:ReasoningSentence {
    reasoning:subject_iri = "urn:eigenius:pub:wrn:gene_WRN";
    reasoning:proposition = type_expr(onco:SyntheticLethal("WRN", "MSI"));

    // Modus ponens over the synthesis rule — every antecedent is now a
    // lemma citation (DerivedEvidence of the phase conclusion).
    reasoning:justification = App(App(App(App(App(
        DeclaredEvidence("urn:eigenius:pub:wrn:synthesis_rule"),
        DerivedEvidence("urn:eigenius:pub:wrn:concl_val")),
        DerivedEvidence("urn:eigenius:pub:wrn:concl_vivo")),
        DerivedEvidence("urn:eigenius:pub:wrn:concl_helicase_required")),
        DerivedEvidence("urn:eigenius:pub:wrn:concl_mech")),
        DerivedEvidence("urn:eigenius:pub:wrn:concl_mmr"));

    reasoning:certificate = type_expr(
        alias
            RULE = "urn:eigenius:pub:wrn:synthesis_rule",
            SVD = onco:SelectiveViabilityDependence("WRN", "MSI"),
            IVD = onco:InVivoDependence("WRN", "MSI"),
            RA  = onco:RequiresActivity("WRN", "helicase"),
            DL  = onco:DSBDrivenLethality("WRN", "MSI"),
            CD  = onco:ContributesToDependence("dMMR", "WRN"),
            SL  = onco:SyntheticLethal("WRN", "MSI"),
            RULE_P = SVD -> IVD -> RA -> DL -> CD -> SL,
            // antecedent justification terms — all lemma citations:
            jVAL  = DerivedEvidence("urn:eigenius:pub:wrn:concl_val"),
            jVIVO = DerivedEvidence("urn:eigenius:pub:wrn:concl_vivo"),
            jHEL  = DerivedEvidence("urn:eigenius:pub:wrn:concl_helicase_required"),
            jMECH = DerivedEvidence("urn:eigenius:pub:wrn:concl_mech"),
            jMMR  = DerivedEvidence("urn:eigenius:pub:wrn:concl_mmr"),
            // modus-ponens spine — each antecedent cert is one `derived(...)`:
            c1 = app(SVD, IVD -> RA -> DL -> CD -> SL,
                     DeclaredEvidence(RULE), jVAL,
                     declared(RULE, RULE_P),
                     derived("urn:eigenius:pub:wrn:concl_val", SVD)),
            c2 = app(IVD, RA -> DL -> CD -> SL,
                     reasoning:App(DeclaredEvidence(RULE), jVAL), jVIVO,
                     c1, derived("urn:eigenius:pub:wrn:concl_vivo", IVD)),
            c3 = app(RA, DL -> CD -> SL,
                     reasoning:App(reasoning:App(DeclaredEvidence(RULE), jVAL), jVIVO), jHEL,
                     c2, derived("urn:eigenius:pub:wrn:concl_helicase_required", RA)),
            c4 = app(DL, CD -> SL,
                     reasoning:App(reasoning:App(reasoning:App(DeclaredEvidence(RULE), jVAL), jVIVO), jHEL), jMECH,
                     c3, derived("urn:eigenius:pub:wrn:concl_mech", DL))
        in
        app(CD, SL,
            reasoning:App(reasoning:App(reasoning:App(reasoning:App(DeclaredEvidence(RULE), jVAL), jVIVO), jHEL), jMECH),
            jMMR, c4, derived("urn:eigenius:pub:wrn:concl_mmr", CD))
    );
}
```

### A.4 What changed vs. the inline `C-MAIN`

- **Antecedent terms:** `App(rule, artifact)` re-derivations → bare `DerivedEvidence(concl_X)`.
- **Antecedent certs:** multi-node `app(...)` sub-proofs → one `derived(concl_X, P)` each.
- **The modus-ponens spine (`c1…c5`) is unchanged** — the "from A follows B" backbone is identical; only *how each antecedent is discharged* changed.
- The `va_competition` / `vivo_xenograft` / `helicase_rule + va_fail_k577m` / `mech_rule + mech_dsb` / `mmr_rule + mmr_restoration` references **leave `C-MAIN`** — they live in the phase conclusions, cited by name.
- Kernel-side: each Held conclusion contributes `IsDerivedAs(concl_X, P)` to the witness index (because `canonical_proposition = proposition`), so each `derived(concl_X, P)` resolves **by hash** — no resolution of `concl_X`'s content at check time. That is the Form-A invariant.
- The spine still restates each antecedent proposition (`SVD`, `IVD`, …), but those are `alias`-bound once — the entirety of the "restate P" cost.
