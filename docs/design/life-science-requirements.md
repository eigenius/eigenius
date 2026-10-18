# Life-Science Propositions in Eigenius: Mapping, Current Capability, and Required Extensions

**Status:** Draft — working design document
**Scope:** How claims, propositions, and relationships in biopharmaceutical research are represented in Eigenius; which of these representations the current kernel (EigenTT as implemented in `nbe/term.rs`, `nbe/eval.rs`, `nbe/check.rs`) already supports; and where extensions are needed. nanoda_lib's Lean 4 kernel is used as a reference for the richer end of the expression language where EigenTT would need to grow.
**Related:** `boundary-contracts.md`, `lean4-institution.md`, and the Eigenius institutions paper.

## 1. Purpose

Life-science research — drug discovery specifically — exercises the full range of epistemic shapes Eigenius is designed to represent. Unlike engineering workflows where mathematical verification plays a central role, drug discovery rests predominantly on *observed* experimental data and *derived* computational predictions, with formal verification confined to structural properties of models rather than to biological claims themselves. This distribution matters for what the expression language needs to do well.

This document has three parts. Part one lays out how research propositions in a pharmaceutical workflow map to type-theoretic shapes and to the four institutions the system supports ($\mathcal{I}_{\text{Dock}}$, $\mathcal{I}_{\text{ADMET}}$, $\mathcal{I}_{\text{Assay}}$, $\mathcal{I}_{\text{PK}}$). Part two audits the current EigenTT implementation against these shapes and identifies what can already be represented. Part three specifies the extensions the expression language will need, using nanoda_lib as the reference for how richer constructs are typically realized.

The goal is to make the gap between "what the framework promises" and "what the current implementation actually supports" explicit and tractable, rather than letting the gap close only through accumulated implementation accidents.

## 2. The institutions and their fiber structure

The life-science example registers four institutions contributing structured fibers to a shared compound knowledge graph. Each is summarized briefly; the full specification lives in the Grothendieck institutions paper.

**$\mathcal{I}_{\text{Dock}}$ — Molecular docking.** Signatures are protein-ligand systems (target structure, compound library, force field parameters). Sentences are binding hypotheses. Models are binding poses with scoring. Fiber morphisms: *conformational proximity* (RMSD between poses), *re-scoring* (same pose under different scoring functions), *ensemble clustering* (poses grouped by binding mode).

**$\mathcal{I}_{\text{ADMET}}$ — ADMET prediction.** Signatures are compound structures with descriptor sets. Sentences are pharmacokinetic bounds. Models are predicted ADMET profiles. Fiber morphisms: *model uncertainty* (ensemble agreement or disagreement), *descriptor sensitivity* (how profile changes with structural modifications).

**$\mathcal{I}_{\text{Assay}}$ — Experimental assay.** Signatures are assay protocols. Sentences are activity thresholds. Models are dose-response curves. Fiber morphisms: *replicate relationships*, *protocol variations*, *curve fitting* (raw data → fitted IC₅₀).

**$\mathcal{I}_{\text{PK}}$ — Pharmacokinetic modeling.** Signatures are compartmental models with physiological parameters. Sentences are PK targets. Models are concentration-time profiles. Fiber morphisms: *compartment refinement*, *parameter sensitivity*.

Connected by comorphisms including $\rho_{\text{Dock} \to \text{Assay}}$ (predicted binding affinity → expected IC₅₀) and $\rho_{\text{ADMET} \to \text{PK}}$ (predicted ADMET properties → compartmental model parameters).

## Part One — Representation Catalogue

This part catalogues the shapes of claims that arise in life-science workflows and shows how each maps to type-theoretic constructs. Each shape is illustrated with a concrete example from drug discovery.

## 3. Atomic observations

**Example claim.** "Compound EIG-0042 inhibits kinase K with IC₅₀ = 42 nM, 95% CI [38, 47], measured in assay protocol P, by operator O on date D."

**Type-theoretic shape.** Inhabitant of a dependent record type (nested Σ-types):

```
Σ(compound : Compound)
Σ(target : Target)
Σ(protocol : AssayProtocol)
Σ(value : PositiveReal)
Σ(ci : ConfidenceInterval)
Σ(replicates : List Measurement)
Σ(operator : Operator)
Σ(date : Date)
Unit
```

**Institutional location.** $\mathcal{I}_{\text{Assay}}$.

**Epistemic status.** *Observed*. Provenance records the lab, instrument, operator, and date.

**Why the structure matters.** The replicate list is not a scalar — it's a structured collection (three measurements at 38, 44, 45 nM) whose elements are related by replicateRelationship morphisms *within* the assay fiber. Flat storage would collapse the replicates into a single IC₅₀ value and lose the individual measurements. The dependent-record representation preserves them.

## 4. Universal claims over finite ensembles

**Example claim.** "For all 50 poses in docking ensemble E for compound EIG-0042, the docking score is below −7 kcal/mol."

**Type-theoretic shape.** A Π-type quantifying over the ensemble:

```
Π(p : Pose) (p ∈ E) → (score(p) < −7)
```

The ensemble is finite and enumerable, so the quantifier is discharged by iterating over the pose list — conceptually a `Map` primitive applied to the ensemble, with the type system ensuring each element satisfies the bound.

**Institutional location.** $\mathcal{I}_{\text{Dock}}$.

**Epistemic status.** *Derived*. Each pose's score comes from a scoring function calculation.

**Why the structure matters.** The quantifier ranges only over poses related by the ensemble's internal structure — not over all possible poses in conformation space. The fiber morphism `conformationalProximity` gives the ensemble its geometric cohesion; without it, the "for all poses in E" quantifier would reduce to a flat list and lose the relationships between neighboring conformations.

## 5. Existential claims

**Example claim.** "There exists a pose of ligand L within 2 Å RMSD of the crystal-bound conformation that scores below −8 kcal/mol."

**Type-theoretic shape.** A Σ-type used propositionally, where inhabiting it requires producing the witness:

```
Σ(p : Pose)
  (ConformationalProximity p crystal_pose 2.0)
× (score(p) < −8)
```

**Institutional location.** $\mathcal{I}_{\text{Dock}}$.

**Epistemic status.** *Derived*, with the witness produced by the docking run.

**Why the structure matters.** The `ConformationalProximity` morphism is a first-class relationship in the fiber, not an annotation. Claiming "a pose exists within 2 Å" requires naming both the witness pose and the morphism relating it to the reference — both of which are typed inhabitants, not loose metadata.

## 6. Conditional claims (the workhorse shape)

**Example claim.** "If the docking ensemble converges (sampling sufficiency satisfied), then the predicted binding mode is reliable up to RMSD tolerance ε."

**Type-theoretic shape.** A Π-type with a propositional antecedent:

```
Π(E : DockingEnsemble)
  (ConvergenceCondition E)
→ (ModeStability E ε)
```

**Institutional location.** $\mathcal{I}_{\text{Dock}}$, with applicability via comorphism to $\mathcal{I}_{\text{Assay}}$.

**Epistemic status.** *Derived* typically; could be promoted to *verified* if a Lean proof discharges the implication under stated assumptions.

**Why the structure matters.** This is the workhorse shape in scientific reasoning. Most meaningful pharmacological claims are of the form "if model assumptions hold, then conclusion follows." The antecedent captures the assumptions explicitly; the type system forces them to appear in the signature rather than lurking as implicit background. Chained reasoning composes by function composition in the type theory, which is the type-theoretic version of chained syllogistic reasoning — crossing institution boundaries via comorphisms preserves this compositionality.

## 7. Quantitative bounds (honest typing of extrapolation)

**Example claim.** "For all compound concentrations up to 50 μM, hERG channel activity is reduced by less than 10%."

**Type-theoretic shape — the honest version.** Rather than pretending to bound over a continuous concentration range, the type makes explicit that the observation is at tested points and the extrapolation is a separate assumption:

```
(tested : Π(c : TestedConcentration) (InhibitionFraction c < 0.1))
× (extrapolation_assumption : ExtrapolationValid(TestedConcentration, [0, 50_uM]))
```

**Institutional location.** $\mathcal{I}_{\text{Assay}}$.

**Epistemic status.** The tested component is *observed*. The extrapolation assumption is *declared* (a modeling judgment) or *derived* (if a pharmacological model justifies the extrapolation).

**Why the structure matters.** Most bounds stated in drug discovery are implicitly extrapolative. Typing them honestly forces the extrapolation into the signature as an explicit assumption rather than letting it hide in the sentence's pragmatics. This is what makes the claim auditable: a regulatory reviewer can ask "what justifies the extrapolation assumption?" and get back a specific typed resource.

## 8. Statistical and ML-based claims

**Example claim.** "The ML ensemble M predicts oral bioavailability of 45% with 95% confidence interval [38%, 52%]."

**Type-theoretic shape.** A `Prediction` type that makes the model a first-class component of the claim:

```
Prediction
  (claim : BioavailabilityClaim)
  (interval : ConfidenceInterval)
  (ensemble : MLEnsemble)
```

**Institutional location.** $\mathcal{I}_{\text{ADMET}}$.

**Epistemic status.** *Derived*, permanently. No ML prediction is promoted to *verified* regardless of confidence level.

**Why the structure matters.** Two structural points. First, the type refuses to let "P = 0.95" be confused with "proved" — the ensemble parameter makes the statistical nature of the claim part of its type, not a footnote. Second, the fiber morphism `modelAgreement` within $\mathcal{I}_{\text{ADMET}}$ captures disagreements between sub-models of the ensemble as first-class queryable relationships. When the paper's example has three sub-models predicting "safe" on CYP3A4 and two predicting "risk," the disagreement is structurally visible rather than buried in a combined confidence interval.

What *can* be verified via Lean is the procedure producing the interval — bounds on generalization error under stated training-distribution assumptions, coverage guarantees for the confidence-interval calibration. The individual prediction stays statistical; the procedure producing it may carry formal guarantees.

## 9. Model-dependent claims

**Example claim.** "Under a two-compartment PK model with clearance Cl = 5 L/h and volume V_d = 30 L, C_max at 100 mg oral dose is predicted to be 3.2 μM."

**Type-theoretic shape.** A Σ-type where the model is part of the claim:

```
Σ(M : PKModel)
  (M.clearance = 5_L_per_h)
× (M.volume = 30_L)
× (M.compartments = TwoCompartment)
× (Cmax M 100_mg_oral = 3.2_uM)
```

**Institutional location.** $\mathcal{I}_{\text{PK}}$.

**Epistemic status.** *Derived*.

**Why the structure matters.** The model isn't ambient context — it's a component of the claim, and disputing the claim means disputing either the second conjunct (the computation is wrong) or the first (the model is inappropriate). The one-compartment version of the same data predicts C_max = 4.1 μM, and both predictions coexist in the PK fiber connected by a `compartmentRefinement` morphism. The morphism is neither provenance (neither model caused the other) nor a schema relationship (both are PKModel instances) — it is structural content of the PK fiber that flat storage would discard.

## 10. Relational claims as fiber morphism inhabitants

**Example claim.** "Pose p₁ and pose p₂ are conformationally proximate with RMSD 1.4 Å, computed via heavy-atom alignment of the ligand core."

**Type-theoretic shape.** An inhabitant of an inductive type representing the morphism class:

```
ConformationalProximity p₁ p₂ (rmsd : PositiveReal) (method : AlignmentMethod)
```

The inhabitant carries the RMSD value and the alignment method as data.

**Institutional location.** $\mathcal{I}_{\text{Dock}}$.

**Epistemic status.** *Derived*, with the trace recording the alignment algorithm and any parameters.

**Why the structure matters.** Relationships between scientific objects are typed inhabitants of typed morphism classes, not flat graph edges. The RMSD value and alignment method travel *with* the relationship, not as separately-queried properties. This is the structural generalization the institutions paper makes over flat knowledge graphs — every fiber morphism type in every institution follows this pattern.

## 11. Cross-institution claims via comorphism

**Example claim.** "Docking predicted a binding affinity of ΔG = −9.2 kcal/mol for EIG-0042, which corresponds to an expected IC₅₀ of approximately 180 nM under the assay protocol."

**Type-theoretic shape.** A comorphism application, recorded as its own typed resource:

```
ComorphismApplication
  (source : DockingResult)
  (comorphism : ρ_Dock_to_Assay)
  (result : AssayPrediction)
```

Then the predicted assay claim is `Prediction(IC50 ≈ 180_nM, ...)` in $\mathcal{I}_{\text{Assay}}$'s fiber.

**Institutional location.** Spans $\mathcal{I}_{\text{Dock}}$ and $\mathcal{I}_{\text{Assay}}$ via the comorphism boundary.

**Epistemic status.** *Derived* through the comorphism. The comorphism itself is *declared* — it's a mathematical relationship between the two institutions established by the institution designers.

**Why the structure matters.** This is where cross-institution contradictions become surfaceable. Step 4 of the worked example has an *observed* IC₅₀ = 42 nM for the same compound. The fourfold discrepancy between predicted (180 nM) and measured (42 nM) is a cross-fiber query result — not something a medicinal chemist has to notice. Whether this means docking overestimated binding, or the assay conditions differed from the docking's implicit assumptions, is a judgment the chemist makes; the *existence* of the discrepancy is structural and automatic.

## 12. Negative claims

**Example claim.** "No CYP3A4 inhibition was observed at concentrations up to 50 μM."

**Type-theoretic shape.** Honest typing avoids pretending to prove absolute absence:

```
Π(c : Concentration) (c ≤ 50_uM) (c ∈ TestedPoints)
→ (InhibitionFraction c < DetectionLimit)
```

Not the stronger `Π(c : Concentration) → ¬Inhibits(c)` — which the data does not support.

**Institutional location.** $\mathcal{I}_{\text{Assay}}$.

**Epistemic status.** *Observed* at tested points; any stronger claim requires an explicit extrapolation assumption (§7).

**Why the structure matters.** Negative claims are treacherous because their natural-language form ("no inhibition") suggests absolute absence, which is rarely what the experiment actually shows. The type forces the domain of quantification to match the actual observation: tested points within a bounded range, below the assay's detection limit. Stronger claims require explicit extrapolation.

## 13. Meta-level claims

**Example claim.** "The derivation of EIG-0042's predicted therapeutic window depends on three unverified ML predictions and one mechanistic model."

**Type-theoretic shape.** Quantification over the trace tree:

```
Σ(t : Trace)
  (ProducesOutput t TherapeuticWindow)
× (count(UnverifiedMLSteps t) = 3)
× (count(MechanisticModelSteps t) = 1)
```

**Institutional location.** Not domain-specific — this is a claim *about* the derivation graph itself.

**Epistemic status.** *Derived*, produced by queries over the trace tree.

**Why the structure matters.** Because traces mirror the expression tree and are themselves typed Eigon resources, meta-claims are just claims about another layer of the same graph. The same EigenQL that queries domain content queries reasoning structure. This is what makes regulatory questions like "what unverified assumptions does this conclusion rest on?" tractable without special-purpose tooling.

## Part Two — Audit of Current Kernel Capability

The EigenTT kernel as currently implemented (`nbe/term.rs`, `nbe/eval.rs`, `nbe/check.rs`) supports a subset of the representations catalogued above. This part identifies what is directly expressible today and what requires the extensions specified in Part Three.

## 14. Directly representable in the current kernel

The following claim shapes can be represented with the EigenTT expression forms already implemented, assuming the ontology-as-types layer-chain plumbing (tracked separately) is complete.

### 14.1 Atomic observations (§3)

**Representable.** The `Construct` term form builds dependent records from class definitions; `PropAccess` retrieves fields by IRI. An `IC50Measurement` class declared in the assay ontology, with required properties for compound, target, protocol, value, CI, replicates, etc., becomes a Σ-type at check time. A specific measurement is a `Construct` expression producing a `ResourceVal` of that class.

The replicate list is `Exp::list(Measurement)` (the helper that desugars to the appropriate Data form). Measurements themselves are nested resources.

**Caveats.** The `find_sigma_field` function currently returns `Val::Set` as a fallback for unresolved class references — this means property access on `EigonClass` values is not fully type-checked until the layer-chain plumbing is completed. The representation is sound once that plumbing lands; until then, the check is effectively runtime-only.

### 14.2 Existential claims (§5)

**Representable** as Σ-types with `Pair` introduction and `Fst`/`Snd` elimination. The docking run produces a witness pose; the pose is paired with its score-bound certificate; the resulting term inhabits the Σ-type. All primitive forms required (`Sig`, `Pair`, `Fst`, `Snd`) are in the kernel.

The `ConformationalProximity` component is the challenging part — not the Σ-shape but the inductive type representing the morphism. See §16.1.

### 14.3 Model-dependent claims (§9)

**Representable** as nested Σ-types where the model is the outer binding. The type expresses "there exists a PKModel such that it has these parameters and this prediction holds." `Construct` produces the model; a downstream `Construct` embeds the model and its prediction together.

### 14.4 Relational claims at the simplest level (§10, shallow)

**Partially representable.** A fiber morphism whose instance is a dependent record of the source, target, and some scalar properties (like a `ConformationalProximity` carrying two pose references and an RMSD value) can be represented as a `Construct` of an ontology class. This is flat-edge-shaped representation with the morphism as a typed resource class.

**Caveats.** The *inductive-type-with-eliminators* aspect of fiber morphisms — where the morphism carries domain-specific structure that can be pattern-matched and reduced — is not yet representable. See §16.1.

### 14.5 Statistical predictions (§8) at the value level

**Representable** as `Construct` of a `Prediction` class with the ensemble, claim, and interval as fields. The kernel supports this straightforwardly — it's a dependent record. The EigenTT type system correctly maintains the distinction between a `Prediction(bioavailability = 45%, ...)` and a bare `45%`, so the former cannot be confused with the latter at the type level.

**Caveats.** The kernel cannot yet reason about *properties* of the ensemble (bounds on error rates, coverage guarantees). Those require proof-level verification via the Lean institution, not kernel-level type-checking.

### 14.6 Meta-level claims at the value level (§13)

**Representable** for simple predicates over trace resources. Because traces are ordinary Eigon resources, a query of the form "for this trace, what is the count of unverified ML steps" is a PropAccess chain followed by a native aggregation — within reach of EigenQL and the kernel's existing dispatch.

**Caveats.** Quantification *over* trace structure (rather than querying a specific trace) needs the universal-quantification extensions discussed in §15.

## 15. Representable with straightforward additions

The following require extensions to EigenTT, but of a kind where the underlying design is understood and the work is additive rather than architecturally novel.

### 15.1 Universal claims over finite ensembles (§4)

**What's missing.** Bounded universal quantification discharged by iteration. The Π-type is in the kernel; what's missing is a `Map`/`Reduce` primitive that the type checker understands at the *type level* — not just as a library function at the value level.

**Approach.** Add `Map` and `Reduce` as primitive expression forms (distinct from ordinary function application). When the type checker sees `Map f xs` where `xs : List A`, and `f : Π(a : A) (a ∈ xs) → P a`, the resulting term has type `Π(a : A) (a ∈ xs) → P a` — i.e., the map's result is a proof that the predicate holds for every element. Reduction fires by iterating over the list.

**nanoda_lib reference.** nanoda handles bounded universal quantification via the recursor on `List`, which Lean's kernel generates automatically when the inductive type is declared. The pattern is the same in principle — iterate over the inductive's constructors and discharge the predicate constructor-by-constructor. Adapting this for EigenTT requires the inductive-type infrastructure from §16.1.

### 15.2 Conditional claims beyond the trivial shape (§6)

**What's missing.** Π-types with propositional antecedents work today in the kernel. What's missing is the *reduction story* — when a user has a term of type `P → Q` and a proof `p : P`, the kernel needs to reduce the application to a term of type `Q` cleanly. This works for simple cases but interacts with `NativeDecide` and `IdJ` in ways that the current reducer handles with some subtle edge-case bugs (see the IdJ-on-neutral correctness issue).

**Approach.** Tighten the reducer's handling of neutral terms in conditional reduction. Specifically, the J-on-neutral case should carry all the arguments through the neutral wrapping so that later substitution under known proofs can fire the reduction. Currently the other J arguments (motive, d, x, y) are discarded.

**nanoda_lib reference.** nanoda's reducer handles conditional reduction correctly by maintaining the full spine of the blocked application in the neutral representation. This is the pattern EigenTT's reducer should adopt. The code in `src/expr.rs` and `src/tc.rs` of nanoda shows the idiom.

### 15.3 Quantitative bounds (§7)

**What's missing.** The tested-points-plus-extrapolation pattern works today using existing Σ and Π types. What's worth adding at the library level (not the kernel) is an ontology class hierarchy for explicit-extrapolation patterns, so that `ExtrapolationAssumption` becomes a first-class declared-resource class in the assay ontology rather than an ad-hoc Σ-component.

**Approach.** This is ontology design, not kernel work. Document the pattern in the assay ontology specification.

### 15.4 Cross-institution comorphism applications (§11)

**What's missing.** The comorphism itself as a first-class resource. Currently comorphisms are discussed in the architecture paper as mathematical objects but don't have a corresponding Eigon ontology class. A `Comorphism` class with source/target institution references and a typed function body would let `ComorphismApplication` resources be committed and queried.

**Approach.** Specify `Comorphism` and `ComorphismApplication` as ontology classes. The kernel doesn't need to change — the application is a typed resource that references the comorphism's body, which is itself a EigenTT expression. What *does* need to land first is full type-checking of ontology-class-parameterized expressions, which depends on the ontology-as-types plumbing.

### 15.5 Negative claims (§12)

**What's missing.** Nothing at the kernel level. The honest typing pattern uses existing Π-types and decidable bounds. The issue is discipline, not kernel capability — authors need to not write `¬P` when they mean "P has not been observed at tested points."

**Approach.** Document the pattern. Possibly lint-check for stronger negation forms in ontology-declared claims and suggest the bounded version.

## 16. Extensions required for full coverage

The following require genuine extensions to EigenTT — new expression forms, new type-theoretic rules, and corresponding reducer work.

### 16.1 Inductive types for fiber morphisms (§10, deep)

**What's missing.** EigenTT's current `Data(summands)` form supports sum-of-products (finite sums with pair bodies) but not recursive inductive types with derived eliminators. The `ConformationalProximity`, `ReplicateRelationship`, `ModelAgreement`, and `CompartmentRefinement` morphism types can be represented as flat resource classes (as discussed in §14.4), but the deeper claim — that these are *inductive types* with reduction rules for their eliminators — requires inductive type infrastructure.

This matters because several useful queries depend on it. "Is this conformationalProximity the composition of two shorter-range proximities?" is an inductive question over the morphism structure. Without inductive types and eliminators, these questions can only be answered by traversing the trace tree at the resource level, which is slower and less composable.

**Required extensions to EigenTT:**

- A new expression form `Inductive` for declaring inductive types with constructors and parameters.
- A positivity checker that rejects non-strictly-positive declarations (to ensure decidability).
- Automatic derivation of eliminators from the inductive specification.
- Reduction rules for the eliminators (the `iota` reductions).
- Integration with the type checker's conversion algorithm so that terms reduced by eliminators are considered equal to their normal forms.

**Scope of work.** Substantial. This is the largest single extension needed for the life-science case. Estimated 4–6 weeks of focused work for a first version covering single (non-mutual, non-nested) strictly-positive inductive types.

**nanoda_lib reference.** nanoda implements this fully, including nested inductives (which lag in some versions — see the Appendix A caveat in `lean4-institution.md`). The relevant code is in `src/inductive.rs`. For EigenTT, only the single-inductive case is needed initially; the patterns in nanoda for constructor typing, recursor generation, and iota reduction translate cleanly even if the specific term representation differs.

**Life-science impact.** Unblocks §10 fully. Also improves §4 because bounded universal quantification becomes cleaner once the underlying `List` type is a properly recursive inductive rather than the simplified sum-of-products form currently used in `Exp::list`.

### 16.2 Universe stratification enforcement (§13, for meta-level soundness)

**What's missing.** The kernel has `Type(n)` as a term form and the basic `Type(n) : Type(n+1)` rule, but the three-level epistemic stratification ("a trace at level N can only reference resources at level N−1 or below") is not yet enforced. For meta-level claims over traces to be sound, this enforcement has to land.

**Required extensions.** Not to the expression language itself — the enforcement belongs at resource ingestion time and in the layer system, not in EigenTT term forms. What EigenTT needs is consistent handling of universe levels in its checker so that attempts to construct self-referential meta-claims are rejected with a clear error.

**Scope of work.** Moderate. The enforcement point is elsewhere but the checker's universe rules need to be tightened.

**nanoda_lib reference.** Lean's universe polymorphism is more complex than Eigenius needs. Three fixed levels with concrete stratification is simpler than Lean's treatment. nanoda's universe-checking code (`src/level.rs`) is nevertheless the reference for how universe checking integrates with type equality — the pattern is the same even though the feature set is smaller.

**Life-science impact.** Unblocks §13 soundness. Meta-claims about traces become reliable rather than merely convenient.

### 16.3 Constraint reduction at type-check time for domain predicates

**What's missing.** The kernel has `NativeDecide` for range bounds, length checks, regex patterns, and format validation. What's missing is domain-specific constraint reduction for life-science-specific predicates — RMSD thresholds, concentration bounds, confidence-interval containment, compound-structure validity.

**Required extensions.** Extend the `Constraint` enum in `nbe/term.rs` to include domain-specific variants, or (better) generalize the `NativeDecide` machinery to dispatch to institution-registered decision procedures. The latter is more architecturally consistent: domain constraints belong in the institution, not the kernel.

**Scope of work.** Small to moderate. The architectural pattern (institution-registered decision procedures) is the cleaner path and fits the existing capability-dispatch machinery.

**Life-science impact.** Improves §5, §6, §7, §12 by allowing domain predicates like "RMSD below 2 Å" or "concentration in therapeutic range" to reduce at check time rather than requiring runtime verification.

### 16.4 The `verified_in` witness extension to EigonClass

**What's missing.** Currently a `StressResult`-class claim and a `StressResult`-class claim are indistinguishable to the type checker, whether one was derived via simulation and one was verified via Lean proof. Extending `EigonClass(iri)` to `EigonClass(iri, verified_in: E)` lets the type system distinguish verified from derived resources at compile time.

**Required extensions.** New term form `VerifiedClass(iri, environment)` (or parameterize `EigonClass`); coercion rules relating verified to unverified types; NbE equality handling.

**Scope of work.** Non-trivial. Discussed at length as open question 9 in `lean4-institution.md`.

**Life-science impact.** Lower than for engineering. Most life-science claims stay *derived* permanently, so the added expressiveness has fewer consumers in this domain. The extension is still valuable where verification *does* apply — properties of the PK model's solution, consistency of cross-route derivations, calibration of ensemble confidence intervals. Worth tracking but probably not the first extension life-science users will ask for.

## 17. Summary table

A compact reference for planning purposes:

| Shape | Examples in §| Kernel today | With planned extensions |
|---|---|---|---|
| Atomic observations | 3 | ✓ (pending plumbing) | ✓ |
| Finite universal quantification | 4 | Partial | ✓ (with Map/Reduce) |
| Existential claims | 5 | ✓ | ✓ |
| Conditional claims | 6 | Partial | ✓ (with reducer fixes) |
| Quantitative bounds (honest) | 7 | ✓ | ✓ |
| Statistical predictions | 8 | ✓ (at value level) | ✓ (at value level) |
| Model-dependent claims | 9 | ✓ | ✓ |
| Relational claims (flat) | 10 | ✓ | ✓ |
| Relational claims (inductive) | 10 | ✗ | ✓ (with inductive types) |
| Cross-institution comorphisms | 11 | Partial | ✓ (with ontology additions) |
| Negative claims | 12 | ✓ (with discipline) | ✓ |
| Meta-level claims (shallow) | 13 | ✓ | ✓ |
| Meta-level claims (sound) | 13 | ✗ | ✓ (with universe enforcement) |
| Verified-status distinction | — | ✗ | ✓ (with witness extension) |

## Part Three — Consolidated Extension Plan

## 18. Prioritization

The extensions required for life-science coverage fall into three tiers by leverage:

**Tier 1 — Unblocks substantial coverage.** Inductive types for fiber morphisms (§16.1) is the single highest-leverage extension. It unblocks deep fiber-morphism representation, improves bounded universal quantification, and lands a general-purpose capability the kernel will need across all domains.

**Tier 2 — Completes known gaps.** Universe stratification enforcement (§16.2), constraint reduction for domain predicates (§16.3), and comorphism ontology classes (§15.4). Each is additive, each has a clear implementation path, and together they complete the coverage of the shapes life-science users need.

**Tier 3 — Deferred to consumer demand.** The `verified_in` witness extension (§16.4) is valuable in principle but of limited leverage for life sciences specifically. Worth tracking as an open question and implementing when a concrete consumer requests it, per the open-questions discipline.

## 19. Recommended sequencing

Given the prioritization, a coherent sequencing:

1. **Complete the ontology-as-types layer-chain plumbing first.** This is tracked separately and is a prerequisite for most of what follows. Nothing in the life-science representation works cleanly until `find_sigma_field` resolves `EigonClass` to proper dependent records.
2. **Add `Map` and `Reduce` as type-level primitives**, unblocking §4 and improving §13.
3. **Add inductive types**, unblocking §10 (deep) and enabling the recursor-based treatment of bounded quantification more cleanly.
4. **Tighten universe stratification**, making meta-level claims (§13) sound.
5. **Generalize `NativeDecide` to institution-registered decision procedures**, improving several shape coverages.
6. **Specify `Comorphism` as an ontology class**, unblocking §11 at the representation level.
7. **Defer the `verified_in` witness extension** until a life-science consumer makes a specific case for it.

Each step produces independent value. The kernel is never in a state where the life-science representation is broken in novel ways; it's in a state where some shapes are fully supported, some are partially supported via workarounds, and the extensions fill in the remaining gaps according to the above sequence.

## 20. The reference role of nanoda_lib

Throughout the extensions above, nanoda_lib serves as a reference rather than a dependency for EigenTT itself. The pattern is consistent: when implementing an extension to EigenTT, consult the analogous construct in nanoda, understand the design decisions and edge-case handling, and implement a simpler version for EigenTT that respects Eigenius's architectural constraints (capability modes, tracing, ontology-as-types resolution) while not adopting Lean-specific complexity (universe polymorphism, nested inductives, η for structures).

The companion document *Type Checking in Lean 4* (Chris Bailey) is the more valuable artifact than nanoda's source alone — it is an extended specification of kernel design at exactly the right level of abstraction for someone implementing a related kernel. Reading it during the extension work substantially reduces the time spent on "wait, how does this interaction work?" questions.

No code from nanoda is imported into EigenTT. The reference relationship is one of *consulted design*, not code sharing. This preserves EigenTT's identity as an Eigenius-specific kernel rather than a derivative of Lean's kernel, while letting the extension work benefit from decades of dependent-type-checker design wisdom.

## 21. Open questions specific to life-science representation

1. **Assay protocol variation modeling.** The fiber morphism `protocolVariations` relates assay results measured under different protocols. How is protocol equivalence established — as a declared relation, as a comorphism within $\mathcal{I}_{\text{Assay}}$, or via an inductive type of protocol transformations? The choice affects how "the same compound measured in two protocols" is typed.

2. **Ensemble-level properties as quantified claims.** A claim like "the docking ensemble is reliable" is natively a universal claim over the ensemble, but the ensemble itself has structure (morphisms between its poses). Should reliability be typed at the ensemble-object level (a property of the ensemble as a whole) or at the element level (a property of every pose)? Probably the former, but the specification needs to pin this down.

3. **Time-varying claims.** PK models produce concentration-time profiles, which are functions of time. How are function-valued claims typed? As `Π(t : Time) (t ∈ Dosing_Interval) → Concentration`, or with a more structured `TemporalProfile` inductive type? The choice affects how queries like "what is the C_max and when does it occur?" are expressed.

4. **Uncertainty propagation across comorphism.** When $\rho_{\text{ADMET} \to \text{PK}}$ translates an ADMET prediction (with confidence interval) into a PK model parameter, how is the confidence interval propagated? Linearly, by Monte Carlo sampling of the PK model, or by some structured transformation? The comorphism's type should reflect this.

5. **Regulatory evidence packaging.** A regulatory submission is a bundle of claims of mixed epistemic status. Is there a canonical "evidence bundle" class that packages observed data, derived predictions, declared assumptions, and verified properties together? If yes, its structure is worth specifying explicitly rather than letting each submission re-invent it.

---

*This document is a working specification. It captures the current understanding of how life-science research propositions map into Eigenius and what extensions the expression language needs. As the extensions land and as domain users provide feedback, both the catalogue and the extension plan should be revised. The prioritization in §18 and the sequencing in §19 are best-current-guess and should be revisited after Tier 1 is complete.*