# 7. Statistics + reasoning walkthrough

The [kinase walkthrough (chapter 6)](06-kinase-walkthrough.md) traces the platform's first composition shape: five Julia institutions coordinating over `formulas:FormulaTerm`, bridged by declared comorphisms, AutoOnLoad gates firing as data flows through the typed pipeline. This chapter traces the second composition shape: the [D52 measurement-statistics institution](../platform/statistics-institution/README.md) and the [D39 reasoning institution](../platform/reasoning-institution/README.md), bridged by the [D49 chain-witness index](../esl/06-resources-types-and-the-layer.md#6-4a-witness-predicates-admitting-propositions-from-layer-state) over a shared `core:EigenTTType` proposition slot. The two shapes use the same chain primitives — typed resources, AutoOnLoad cascades, deterministic verifiers — but with a different bridge mechanism. Reading both walkthroughs side by side surfaces the choice space: comorphism-mediated translation when one runtime's output needs to be reshaped for another's consumption, witness-index admission when one institution's verdict is being cited as evidence rather than re-processed.

The fixture this chapter traces lives at [`crates/eigenius-reasoning/tests/fixtures/drug_screening.esl`](../../../crates/eigenius-reasoning/tests/fixtures/drug_screening.esl). The end-to-end test that exercises it is [`crates/eigenius-reasoning/tests/drug_screening.rs`](../../../crates/eigenius-reasoning/tests/drug_screening.rs).

## 7.1. The scenario

A drug-screening lab has three IC50 readings for compound EIG_0291 (72, 85, 100 nM) and a literature rule asserting that compounds with IC50 < 100 nM at a kinase target are strong inhibitors. The author wants the chain to attest the conclusion `StrongInhibitor(EIG_0291)` — but only by mechanical derivation from the raw data plus the cited rule, not by author assertion.

Five chain commits accomplish this:

1. Declare the domain predicates (`HasLowIC50`, `StrongInhibitor`) in the chain ontology, both marked `is_a stats:PopulationLevel` per [D52 §7.4](../platform/statistics-institution/README.md#7-4-opinionated-stance-technicalonly-replicates-cannot-support-populationlevel-propositions).
2. Commit a `stats:SampleSetResource` carrying the three raw IC50 readings via `stats:SingleSampleEstimate(...)`, paired with an `ObservationTrace`.
3. Commit a `stats:StatisticalAnalysisPlan` against the SampleSet asserting the 100 nM threshold (alpha = 0.05, TwoSided, WelchUnequal, Identity exclusion), paired with a `ProgramTrace`. The D52 institution's AutoOnLoad gate recomputes the claim and emits a `Verdict::Holds`; the chain-witness index admits `IsDerivedAs(claim_iri, HasLowIC50(EIG_0291))`.
4. Commit the literature rule as a `reflection:DeclaredResource` whose `canonical_proposition` is `HasLowIC50(EIG_0291) -> StrongInhibitor(EIG_0291)`, paired with a `DeclarationTrace`. The chain-witness index admits `IsDeclaredAs(rule_iri, HasLowIC50 -> StrongInhibitor)`.
5. Commit a `reasoning:ReasoningSentence` whose justification is `App(DeclaredEvidence(rule_iri), DerivedEvidence(claim_iri))` and whose certificate is the matching `JustifiedBy.app` term. The D39 institution's AutoOnLoad gate type-checks the certificate against `JustifiedBy(justification, StrongInhibitor(EIG_0291))`; both grounding constructors consume the admitted witnesses; verdict is Holds; the sentence is admitted.

No comorphism is declared between the two institutions. No bridge code runs to translate the statistics verdict into a reasoning input. The composition works because both institutions honour the same chain artifact shape (`DerivedResource` + `ProgramTrace` + `canonical_proposition`), and the witness index reads from that shape uniformly.

## 7.2. The five chain shapes at a glance

| Resource class | What it contributes |
|---|---|
| `screen:HasLowIC50`, `screen:StrongInhibitor` (`data : ... -> Prop, stats:PopulationLevel`) | Domain predicates. The multi-class header puts both `is_a InductiveType` (implicit) and `is_a stats:PopulationLevel` on the resource so D52's §7.4 check admits the claim under BiologicalReplication. |
| `screen:m_eig0291_sampleset` (`stats:SampleSetResource`) | Holds the three raw IC50 reads. `stats:SingleSampleEstimate([72.0, 85.0, 100.0], BiologicalReplication())` lands at the SingleSampleEstimate dispatch position. |
| `screen:m_eig0291_sampleset_trace` (`reflection:ObservationTrace`) | Pairs the SampleSet with its bench provenance. Admits `IsObservedAs` for downstream auditability (D49 §6). |
| `screen:claim_eig0291_lowic50` (`stats:StatisticalAnalysisPlan`) | The universal-claim schema: alpha, effect_size, directionality, variance_assumption, outlier_exclusion. Its `canonical_proposition` is `HasLowIC50(EIG_0291)`. AutoOnLoad-gated by D52. |
| `screen:claim_eig0291_lowic50_trace` (`reflection:ProgramTrace`) | Pairs the claim with the statistics-institution validator. Admits `IsDerivedAs(claim_iri, HasLowIC50(EIG_0291))` once the claim's canonical_proposition is on chain. |
| `screen:rule_strong` (`reflection:DeclaredResource`) | The literature rule. `canonical_proposition` is `HasLowIC50 -> StrongInhibitor`. |
| `screen:rule_strong_trace` (`reflection:DeclarationTrace`) | Admits `IsDeclaredAs(rule_iri, HasLowIC50 -> StrongInhibitor)`. |
| `screen:concl_eig0291_strong` (`reasoning:ReasoningSentence`) | The reasoning step. Proposition: `StrongInhibitor(EIG_0291)`. Justification: `App(DeclaredEvidence(rule), DerivedEvidence(claim))`. Certificate: `JustifiedBy.app(declared(rule, ...), derived(claim, ...))`. AutoOnLoad-gated by D39. |

The fixture commits all of these in one ESL document; the AutoOnLoad cascades fire in commit order ([§4.2](04-dispatch-roles-in-concert.md#42-autoonload-cascades-single-commit-multiple-gates)).

## 7.3. Step-by-step: data flowing through the pipeline

### Step 1 — Predicate declarations land in the layer

```esl
data screen:HasLowIC50 : core:string -> Prop, stats:PopulationLevel { }
data screen:StrongInhibitor : core:string -> Prop, stats:PopulationLevel { }
```

Each `data` declaration commits a `core:InductiveType` resource whose `core:is_a` array carries `[core:InductiveType, stats:PopulationLevel]`. No AutoOnLoad fires — these are pure ontology declarations.

The `stats:PopulationLevel` marker is what D52's §7.4 epistemic-scope check will read at StatisticalAnalysisPlan admissibility time to decide whether a `TechnicalWithinRun` replication would be rejected. (Here the SampleSet uses `BiologicalReplication`, so the check passes regardless of the marker; the marker is load-bearing in the parallel `TechnicalWithinRun` fixture variant.)

### Step 2 — SampleSet + ObservationTrace land in the layer

```esl
resource screen:m_eig0291_sampleset : stats:SampleSetResource {
    reflection:source      = "instrument-log:kinase-glo-plate-2026-03-04-A1";
    reflection:observed_at = "2026-03-04T14:22:11Z";

    stats:sample_set_value = stats:SingleSampleEstimate(
        [72.0, 85.0, 100.0],
        BiologicalReplication(),
    );
}

resource screen:m_eig0291_sampleset_trace : reflection:ObservationTrace {
    reflection:resource  = screen:m_eig0291_sampleset;
    reflection:source    = "instrument-log:kinase-glo-plate-2026-03-04-A1";
    reflection:timestamp = "2026-03-04T14:22:11Z";
}
```

`stats:SingleSampleEstimate(...)` is a [macro](../esl/04-declarations.md#4-9-macro-compile-time-smart-constructors-d52-12) declared in the statistics ontology layer; the compiler picks it up via `compile_against_layer` ([§6.5.4](../esl/06-resources-types-and-the-layer.md#6-5-4-cross-file-macro-and-axiom-visibility-compile_against_layer)) and expands it at compile time to a `Bundle(CompleteRandom(), Unblocked(), NoFactor(), BiologicalReplication(), CrossSectional(), Units([]), Columns([]), Entries([]), [72.0, 85.0, 100.0])`. That's the chain wire shape the SampleSetResource carries.

No AutoOnLoad fires on the SampleSetResource itself — D52's gate is on `StatisticalAnalysisPlan`, not on `SampleSetResource`. The trace pairing it admits `IsObservedAs(sampleset_iri, ...)` in the witness index, but no D39 sentence in this fixture cites the SampleSet directly via `ObservedEvidence` (the reasoning chain goes through the claim's `DerivedEvidence` instead), so the IsObservedAs witness is unused.

### Step 3 — StatisticalAnalysisPlan + ProgramTrace land; D52 AutoOnLoad fires

```esl
resource screen:claim_eig0291_lowic50 : stats:StatisticalAnalysisPlan {
    stats:sample_set = screen:m_eig0291_sampleset;

    stats:null_hypothesis = type_expr(
        screen:HasLowIC50("urn:eigenius:demo:screen:EIG_0291")
    );
    stats:alternative_hypothesis = type_expr(
        screen:HasLowIC50("urn:eigenius:demo:screen:EIG_0291")
    );
    reflection:canonical_proposition = type_expr(
        screen:HasLowIC50("urn:eigenius:demo:screen:EIG_0291")
    );

    stats:alpha = 0.05;
    stats:effect_size = Absolute(100.0, "nM");
    stats:directionality = TwoSided();
    stats:variance_assumption = WelchUnequal();
    stats:outlier_exclusion = Identity();
}

resource screen:claim_eig0291_lowic50_trace : reflection:ProgramTrace {
    reflection:resource  = screen:claim_eig0291_lowic50;
    reflection:source    = "statistics-institution:validate_analysis_plan";
    reflection:timestamp = "2026-03-04T14:22:11Z";
}
```

The StatisticalAnalysisPlan commit triggers D52's `validate_analysis_plan` AutoOnLoad gate. The gate runs the [four-step check](../platform/statistics-institution/README.md#the-four-step-validate_analysis_plan-check):

1. Resolve `sample_set` → `m_eig0291_sampleset` → decode the `Bundle(...)` ctor.
2. Read claim parameters; `OneSidedWitnessed` not asserted, so no impossibility-witness lookup.
3. Dispatch on `(CompleteRandom, Unblocked, NoFactor, _, CrossSectional)` → SingleSampleEstimate → run `one_sample_t_test([72.0, 85.0, 100.0], 100.0)`. Result: `t = -1.776`, `p_two_sided ≈ 0.218` (the SD across (72, 85, 100) is too large for n = 3 to reject at α = 0.05).
4. §7.4 epistemic scope: BiologicalReplication admits PopulationLevel ✓. Compare `p < alpha`: `0.218 < 0.05` → False → `Verdict::Fails`.

For the running narrative we use the *failing* claim — that's what the fixture commits — but the same shape works for the confirmatory dataset in the fixture's parallel claim (n = 6 tightly clustered around 85 nM), which produces `Verdict::Holds` and admits the witness for downstream D39 use. The drug-screening fixture in the reasoning crate uses the failing n=3 dataset for the *initial* claim and shows the D49 witness mechanism is layer-scoped — voiding the claim's layer removes the witness from descendant resolutions. For an end-to-end Holds variant, see the IC50 fixture in [`crates/eigenius-statistics/tests/fixtures/ic50_measurement.esl`](../../../crates/eigenius-statistics/tests/fixtures/ic50_measurement.esl), which the D39 reasoning fixture transitively layers on top of via `compile_against_layer`.

When the claim *does* pass (the confirmatory case), the witness index admits:

```
WitnessKey {
    category: Derived,
    iri: "urn:eigenius:demo:screen:claim_eig0291_lowic50",
    prop_hash: sha256(canonical_cbor(encode_type(HasLowIC50("urn:...:EIG_0291")))),
}
```

This entry is what the D39 sentence's `JustifiedBy.derived` constructor will consume in step 5.

### Step 4 — Rule + DeclarationTrace land

```esl
resource screen:rule_strong : reflection:DeclaredResource {
    reflection:declared_by = "literature:smith_et_al_2024_3.2";
    reflection:rationale   = "IC50 < 100 nM at a kinase target is the standard strong-inhibitor threshold.";

    reflection:canonical_proposition = type_expr(
        screen:HasLowIC50("urn:eigenius:demo:screen:EIG_0291")
        ->
        screen:StrongInhibitor("urn:eigenius:demo:screen:EIG_0291")
    );
}

resource screen:rule_strong_trace : reflection:DeclarationTrace {
    reflection:resource    = screen:rule_strong;
    reflection:declared_by = "literature:smith_et_al_2024_3.2";
    reflection:timestamp   = "2026-04-10T09:00:00Z";
}
```

No AutoOnLoad fires — `DeclaredResource` is a chain-shape class, not an institution input. The trace pairs it with the witness index, which admits:

```
WitnessKey {
    category: Declared,
    iri: "urn:eigenius:demo:screen:rule_strong",
    prop_hash: sha256(canonical_cbor(encode_type(
        HasLowIC50("urn:...:EIG_0291") -> StrongInhibitor("urn:...:EIG_0291")
    ))),
}
```

Two witness keys are now in the index, with corresponding entries for the `IsDerivedAs` (from step 3, in the Holds case) and `IsDeclaredAs` predicates.

### Step 5 — ReasoningSentence lands; D39 AutoOnLoad fires

```esl
resource screen:concl_eig0291_strong : reasoning:ReasoningSentence {
    reasoning:subject_iri = "urn:eigenius:demo:screen:EIG_0291";

    reasoning:proposition = type_expr(
        screen:StrongInhibitor("urn:eigenius:demo:screen:EIG_0291")
    );

    reasoning:justification = App(
        DeclaredEvidence("urn:eigenius:demo:screen:rule_strong"),
        DerivedEvidence("urn:eigenius:demo:screen:claim_eig0291_lowic50")
    );

    reasoning:certificate = type_expr(
        app(
            screen:HasLowIC50("urn:eigenius:demo:screen:EIG_0291"),       // A
            screen:StrongInhibitor("urn:eigenius:demo:screen:EIG_0291"),  // B
            DeclaredEvidence("urn:eigenius:demo:screen:rule_strong"),     // j1
            DerivedEvidence("urn:eigenius:demo:screen:claim_eig0291_lowic50"), // j2
            declared(
                "urn:eigenius:demo:screen:rule_strong",
                screen:HasLowIC50("urn:eigenius:demo:screen:EIG_0291")
                ->
                screen:StrongInhibitor("urn:eigenius:demo:screen:EIG_0291"),
                screen:HasLowIC50("urn:eigenius:demo:screen:EIG_0291")
            ),
            derived(
                "urn:eigenius:demo:screen:claim_eig0291_lowic50",
                screen:HasLowIC50("urn:eigenius:demo:screen:EIG_0291"),
                screen:HasLowIC50("urn:eigenius:demo:screen:EIG_0291")
            )
        )
    );
}
```

The commit triggers D39's `validate_justification` AutoOnLoad gate. The gate's check:

1. **Decode** `proposition`, `justification`, `certificate` from the three property slots into kernel `Exp` values.
2. **Type-check the proposition.** `StrongInhibitor("urn:...:EIG_0291")` has type `Prop` ✓.
3. **Type-check the certificate.** Construct the expected type `JustifiedBy(justification, proposition)` = `JustifiedBy(App(DeclaredEvidence("...rule_strong"), DerivedEvidence("...claim_eig0291_lowic50")), StrongInhibitor("...EIG_0291"))`. Walk the certificate's `app(...)` constructor; it requires sub-certificates for:
   - `JustifiedBy(DeclaredEvidence("...rule_strong"), HasLowIC50 -> StrongInhibitor)` — matched by `declared(...)`. The constructor consumes `IsDeclaredAs("...rule_strong", HasLowIC50 -> StrongInhibitor)`; the kernel hashes the proposition, looks up the witness key in the layer's index, finds the entry admitted in step 4, returns the opaque witness value.
   - `JustifiedBy(DerivedEvidence("...claim_eig0291_lowic50"), HasLowIC50)` — matched by `derived(...)`. The constructor consumes `IsDerivedAs("...claim_eig0291_lowic50", HasLowIC50)`; the kernel hashes, looks up, finds the entry admitted in step 3 (in the Holds case), returns the opaque witness.
   Both witnesses admit, the `app(...)` constructor type-checks against `JustifiedBy(App(j1, j2), B)`, the outer certificate type-checks against the expected `JustifiedBy(...)` type ✓.
4. **Emit the verdict.** `Verdict::Holds`. The sentence is admitted; the chain has attested `StrongInhibitor(EIG_0291)`.

The reasoning sentence is itself a `DerivedResource` (the `ReasoningSentence : DerivedResource` declaration in the reasoning ontology), so its `canonical_proposition` (defaulted from `reasoning:proposition`) lands in the witness index as `IsDerivedAs("...concl_eig0291_strong", StrongInhibitor("...EIG_0291"))`. A downstream reasoning sentence can cite *this* one via `DerivedEvidence` and consume the witness — proof composition is just another form of derivation composition.

## 7.4. The AutoOnLoad cascade in this scenario

Two AutoOnLoad gates fire in this commit sequence:

1. **D52 fires on StatisticalAnalysisPlan commit (step 3).** Recomputes the claim, emits Verdict + RuntimeInvocation. As a side effect of the trace + canonical_proposition pair being on chain, the witness index admits the `IsDerivedAs` entry.
2. **D39 fires on ReasoningSentence commit (step 5).** Type-checks the certificate, consults the witness index for the `IsDeclaredAs` (from step 4) and `IsDerivedAs` (from step 3) entries, finds both, admits the certificate, emits Verdict + RuntimeInvocation.

The cascade is mechanical, not coordinated. D52 doesn't know D39 is about to fire; D39 doesn't know D52 ran. They share the chain artifact shape (`DerivedResource` + `ProgramTrace` + `canonical_proposition`) that the witness index reads from. The composition emerges from each institution honouring the shared chain shape independently. See [§4.2 "The D52 → D39 cascade"](04-dispatch-roles-in-concert.md#the-d52--d39-cascade) for the dispatch-role framing of this.

If the order is reversed (the D39 sentence commits before the D52 claim that grounds it), the sentence's `derived(claim_iri, ...)` constructor would fail to admit the witness — the index entry doesn't exist yet — and the commit would be rejected with a `NoAdmittedChainWitness` diagnostic. The fixture commits in topological order to avoid this, but the platform's transactional commit semantics ensure no partial state is observable; either the whole commit succeeds or none of its resources land.

## 7.5. Where this composition differs from the kinase one

| Dimension | Kinase walkthrough (chapter 6) | Stats + reasoning walkthrough (this chapter) |
|---|---|---|
| **Shared payload** | `formulas:FormulaTerm` — the chain-mirrored EigenTT numerical-expression fragment. | `core:EigenTTType` — the chain-mirrored EigenTT type-expression fragment ([D47](../../design/d47-chain-mirrored-eigentt-type-fragment.md)). |
| **Bridge mechanism** | Declared comorphisms (Symbolics → JuMP, Catalyst → DiffEq, Symbolics → IntervalArithmetic). Active translation: extract → transform → reify. | Per-layer chain-witness index over `canonical_proposition` slots ([D49](../../design/d49-chainwitness-machinery.md)). Passive admission: producer emits, consumer reads. |
| **Coupling** | Each comorphism is a typed bridge with its own implementation. Adding a new institution to the composition requires authoring new comorphisms to/from it. | Each institution honouring the shared chain shape (`DerivedResource` + `ProgramTrace` + `canonical_proposition`) participates automatically. Adding a new institution to the composition requires no new bridge code. |
| **What runs when** | Comorphism dispatch invokes the source institution's runtime via the extract → transform pipeline; the reify step commits a new chain resource. | The producer's AutoOnLoad gate runs at commit; the witness index is materialized as a side effect; the consumer's AutoOnLoad gate reads the index when type-checking the cited grounding constructor. No reify step. |
| **Use case** | "Translate this numerical expression from one institution's view into another's, then have the target institution process it." | "Cite this institution's verdict as evidence inside a reasoning chain, without re-processing the value itself." |
| **Failure mode** | Comorphism failure (extract reject, transform error, reify fail). Surfaces as institutional-error tuples per [§8.2](09-failure-modes.md#9-2-comorphism-dispatch-failures-extract--transform--reify). | `NoAdmittedChainWitness` from the consumer's gate, naming the missing (category, iri, proposition) triple. Surfaces as a type-checking diagnostic. |

Both shapes are first-class. Which one applies depends on whether the downstream institution needs the input *value translated* (comorphism) or just *cited as evidence* (witness index). A future institution could participate in both: produce a `core:EigenTTType` proposition for witness-index consumers AND register comorphisms that translate its outputs into other institutions' payloads for runtime consumption. The platform doesn't mandate one composition shape per institution; the composition is per-edge.

## 7.6. Inspecting the audit chain from EigenQL

The verdict + witness chain is queryable from EigenQL. A typical inspection sequence after the fixture commits successfully:

```eigenql
USING "urn:eigenius:reasoning"

// Find the reasoning verdict
MATCH "urn:eigenius:institution:Verdict"(?v) {
    "urn:eigenius:institution:verdict_subject": "urn:eigenius:demo:screen:concl_eig0291_strong",
    "urn:eigenius:core:ctor_name": ?c
}
RETURN [] { verdict: ?c }
```

→ one row, `verdict = "Holds"`.

```eigenql
// Walk back to the claim the sentence's DerivedEvidence cited
MATCH "urn:eigenius:reasoning:ReasoningSentence"(?s) {
    "@id": "urn:eigenius:demo:screen:concl_eig0291_strong",
    "urn:eigenius:reasoning:justification": ?j
}
RETURN [] { justification: ?j }
```

→ one row, `justification = {ctor: "App", args: [{ctor: "DeclaredEvidence", args: ["...rule_strong"]}, {ctor: "DerivedEvidence", args: ["...claim_eig0291_lowic50"]}]}`.

```eigenql
// Walk back to the statistics verdict that admitted the IsDerivedAs witness
MATCH "urn:eigenius:institution:Verdict"(?v) {
    "urn:eigenius:institution:verdict_subject": "urn:eigenius:demo:screen:claim_eig0291_lowic50",
    "urn:eigenius:core:ctor_name": ?c,
    "urn:eigenius:measurements:computed_statistic": ?t,
    "urn:eigenius:measurements:computed_p_value": ?p
}
RETURN [] { verdict: ?c, t: ?t, p: ?p }
```

→ one row, `verdict = "Holds"`, `t = -7.5`, `p = 6.4e-4` (for the n=6 confirmatory dataset).

```eigenql
// Walk back to the raw IC50 readings
MATCH "urn:eigenius:measurements:StatisticalAnalysisPlan"(?c) {
    "@id": "urn:eigenius:demo:screen:claim_eig0291_lowic50",
    "urn:eigenius:measurements:sample_set": ?ss
}
RETURN [] { sample_set: ?ss }
```

→ one row, `sample_set = "urn:eigenius:demo:screen:m_eig0291_sampleset"`.

```eigenql
// Read the raw observations off the SampleSet
MATCH "urn:eigenius:measurements:SampleSetResource"(?ss) {
    "@id": "urn:eigenius:demo:screen:m_eig0291_sampleset",
    "urn:eigenius:measurements:sample_set_value": ?bundle
}
RETURN [] { bundle: ?bundle }
```

→ one row carrying the full `Bundle(...)` ctor value with the raw `[72.0, 85.0, 100.0]` observations payload.

Every byte that went into the verification — the raw readings, the claim parameters, the statistics-institution recomputation, the literature rule, the reasoning certificate, both verdicts — sits on the chain as a typed, queryable, content-addressed resource. Nothing in the chain is *trusted*; everything is *anchored*.

## 7.7. Patterns this walkthrough exercises

Each of the next chapter's patterns ([chapter 8](08-patterns.md)) has a concrete instance in this walkthrough:

- **Sharing a payload vs. declaring a converter** ([§8.1](08-patterns.md#8-1-sharing-a-payload-vs-declaring-a-converter)) — `core:EigenTTType` as a shared payload between D52 and D39; no comorphism declared.
- **AutoOnLoad vs. OnDemand** ([§8.3](08-patterns.md#8-3-autoonload-vs-ondemand)) — both D52 and D39 fire AutoOnLoad; the cascade is the composition.
- **Chain reinsertion vs. transient overlay** ([§8.4](08-patterns.md#8-4-chain-reinsertion-vs-transient-overlay)) — the D52 verdict is reinserted as a chain resource (the StatisticalAnalysisPlan is itself the DerivedResource), making it citable by D39. Without reinsertion, the witness index would have nothing to read.

The corresponding failure modes ([chapter 9](09-failure-modes.md)) also have direct instances:

- **Validation cascade failures** ([§9.1](09-failure-modes.md#9-1-validation-cascade-failures)) — if the D52 verdict fails, the cascade stops before D39 runs; the reasoning sentence's commit is rejected because the `IsDerivedAs` witness isn't in the index.
- **Chain-state races and stale Verdicts** ([§9.3](09-failure-modes.md#9-3-chain-state-races-and-stale-verdicts)) — voiding the claim's layer removes the witness from descendant resolutions; reasoning sentences that cited it become unadmissible in those resolutions.

---

Next: **[8. Composition patterns →](08-patterns.md)**
