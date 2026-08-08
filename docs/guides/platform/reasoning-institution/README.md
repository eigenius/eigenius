# Reasoning institution tutorial

Slow-walk worked example of the platform's first reasoning institution: D39 Justification Logic. Walks the closed audit chain end-to-end against a concrete drug-screening scenario — from the chain-committed `Verdict::Holds` back through the `JustifiedBy` certificate, the chain witnesses that admitted the grounding constructors, the trace resources that admitted the witnesses, and the raw chain artifacts those traces point at.

Read this if you want to know what a `ReasoningSentence` commit actually does, how the `App(DeclaredEvidence, DerivedEvidence)` composition picks up groundings from anywhere on the chain, or how a D52 statistics verdict becomes a citable evidence node in a D39 proof.

Surface reference: [**ESL §9.10 — D39 reasoning institution**](../../esl/09-institutions.md#9-10-the-reasoning-institution-d39-justification-logic). Design spec: [**D39 Justification Logic**](../../../design/d39-justification-logic.md). Companion design: [**D49 chain-witness machinery**](../../../design/d49-chainwitness-machinery.md). Implementation: [`crates/eigenius-reasoning/`](../../../../crates/eigenius-reasoning/). Ontology: [`ontologies/reasoning/reasoning.esl`](../../../../ontologies/reasoning/reasoning.esl).

## Why reasoning is different from the Lean institution

[Lean](../lean-institution/README.md) is a *verification* institution: chain authors commit a Lean proof term and the institution re-checks the proof against an exported theorem statement. The proof is its own thing, authored in Lean, exported as bytes, re-checked by a bundled `nanoda_lib`. The chain attests that the proof checks.

D39 is a *reasoning* institution: chain authors commit a triple of (proposition, justification term, certificate) where the certificate is a `JustifiedBy(justification, proposition)` term — an inhabitant of an indexed inductive family declared in the chain's own type theory. The grounding terms reference chain artifacts (an axiom, an observed measurement, a derived claim, a verified Lean proof) and the kernel admits the corresponding chain witnesses from a per-layer index materialized at layer construction. The chain attests both that the certificate type-checks *and* that every cited chain artifact actually exists.

Two consequences of this difference shape the rest of the tutorial:

1. **The validator is the kernel.** No bundled external checker. No mirror-anchor consistency check. The validator for a D39 reasoning sentence is a direct call to the kernel's NbE checker — the same one that type-checks every other ESL program. Implementation lives in [`crates/eigenius-reasoning/src/validate.rs`](../../../../crates/eigenius-reasoning/src/validate.rs) and is ~200 lines: decode three properties from the sentence resource, construct the expected `JustifiedBy(justification, proposition)` type, call `check`. TCB is bounded by the kernel.
2. **Composition is the point.** A reasoning sentence's `App` and `SpecStr` combinators let one chain artifact's verdict ground a downstream conclusion. The motivating composition: a [D52 StatisticalAnalysisPlan](../statistics-institution/README.md) verdict (`HasLowIC50(EIG_0291)`) grounds a `DerivedEvidence` ctor, a literature rule (`HasLowIC50(c) -> StrongInhibitor(c)`) grounds a `DeclaredEvidence` ctor, and `App` composes them into `StrongInhibitor(EIG_0291)`. The composition is mechanical — every step type-checks against the kernel, no out-of-band convention bridges the layers.

This means a reasoning institution is registered with `runtime: in_process`. There is no env-image build, no orchestrator round-trip; the verifier runs synchronously inside the kernel process at commit time.

## The five chain shapes the reasoning audit trail touches

Reasoning leaves a typed audit trail. The institution itself emits only the verdict; the other shapes are pre-existing chain artifacts the reasoning sentence cites or that the witness index reads.

| Resource | Role |
|---|---|
| `axiom` declaration → `eigentt:Axiom` | Author-asserted propositional statement ([ESL §4.4a](../../esl/04-declarations.md#4-4a-axiom-postulated-propositions-d46-10)). Paired with a `DeclarationTrace` to admit `IsDeclaredAs`. |
| `reflection:DeclaredResource` + `DeclarationTrace` | Any chain-resident declared assertion (literature rule, asserted theorem, marker resource). The matching trace admits `IsDeclaredAs(iri, canonical_proposition)`. |
| `reflection:ObservedResource` + `ObservationTrace` | Bench measurement, instrument log entry. The matching trace admits `IsObservedAs(iri, canonical_proposition)`. |
| `reflection:DerivedResource` + `ProgramTrace` | Institution-computed derivation — typically a [D52 StatisticalAnalysisPlan](../statistics-institution/README.md) whose verifier returned Holds, or a prior `ReasoningSentence` (which is itself a `DerivedResource`). Admits `IsDerivedAs(iri, canonical_proposition)`. |
| `reflection:VerifiedResource` + `ProgramTrace` | Formal-proof artifact (e.g., from the [Lean institution](../lean-institution/README.md)). Admits `IsVerifiedAs(iri, canonical_proposition)`, which cumulates into `IsDerivedAs` via the subclass coercion. |

The reasoning institution then emits a sixth shape:

| Resource | Role |
|---|---|
| `reasoning:ReasoningSentence` | The chain-resident reasoning step: proposition + justification + certificate. AutoOnLoad fires `ValidateJustification` on commit; type-checks the certificate against `JustifiedBy(justification, proposition)`. |
| `Verdict` (`ctor_name: "Holds" / "Fails"`) | The institution's outcome. Committed alongside a `RuntimeInvocation` provenance record. Failed verdicts reject the commit. |

The `ReasoningSentence` is *itself* a `DerivedResource` (it carries a `ProgramTrace` and a `canonical_proposition`), so a downstream sentence can cite a prior one via `DerivedEvidence(prior_iri)` and the witness index admits the `IsDerivedAs` witness uniformly — proof composition is just another form of derivation composition.

## The D49 witness index — how the kernel admits grounding witnesses

When the type-checker elaborates a `JustifiedBy.declared` / `.observed` / `.derived` / `.verified` grounding constructor, it needs to produce a value of the corresponding `IsDeclaredAs(iri, P)` / `IsObservedAs(iri, P)` / `IsDerivedAs(iri, P)` / `IsVerifiedAs(iri, P)` predicate. These predicates have **zero surface constructors** — the kernel admits inhabitants only through the per-layer **chain witness index**.

### Index construction (layer lifecycle)

Each `Layer` ([`kernel/src/layer/mod.rs`](../../../../kernel/src/layer/mod.rs)) materializes its `chain_witness_index` deterministically from the layer's resource set. The index is a `BTreeMap<WitnessKey, ()>` keyed by `(category, iri, prop_hash)`:

```rust
pub struct WitnessKey {
    pub category: WitnessCategory,   // Declared / Observed / Derived / Verified
    pub iri: Iri,                    // the trace's `resource` property
    pub prop_hash: [u8; 32],         // SHA-256 of D47-encoded canonical_proposition
}
```

Construction walks the layer's trace resources (`DeclarationTrace` / `ObservationTrace` / `ProgramTrace`), looks up each trace's `resource` IRI to find the matching declared / observed / derived / verified resource, reads `canonical_proposition` from that resource, computes the SHA-256 prop_hash, and inserts the `WitnessKey`. The index is purely derived state — the layer's content hash already covers the trace resources transitively, so the index doesn't need its own persistence.

The index lives on the `Layer` in memory; materialization is lazy (`Layer::chain_witness_index()` builds it on first call) and the result is cached for subsequent lookups. Dropping the `Layer` drops the index — there is no kernel-global witness state.

### Lookup at type-check time

When the kernel encounters `JustifiedBy.derived(iri, P, witness)` and needs to fill in `witness : IsDerivedAs(iri, P)`, the synthesis algorithm runs ([D49 §5](../../../design/d49-chainwitness-machinery.md)):

```
1. encoded_P = encode_type(P)              // D47 codec
2. prop_hash = sha256(canonical_cbor(encoded_P))
3. key = WitnessKey { category: Derived, iri, prop_hash }
4. for layer in ctx.layer_chain top-down:
     if layer.witness_index.contains(&key):
         return Ok(opaque_witness(key))
     // VerifiedResource subclass_of DerivedResource: IsVerifiedAs subsumes IsDerivedAs.
     if category == Derived:
         verified_key = WitnessKey { category: Verified, iri, prop_hash }
         if layer.witness_index.contains(&verified_key):
             return Ok(opaque_witness(verified_key))
5. return Err(NoAdmittedChainWitness { ... })
```

The walk reuses the existing `Arc<Layer>` parent-chain walk (the one resource resolution uses) — no new traversal abstraction. First hit wins. The `opaque_witness` value carries no payload beyond the key; proof irrelevance ([ESL §7.1](../../esl/07-type-theory-primer.md#7-1-universes-the-unified-sortn-ladder-with-prop-at-the-bottom)) makes any two witnesses of the same `(category, iri, P)` definitionally equal.

### Voiding semantics

Witnesses are derived state. Voiding a layer removes its trace resources from any chain resolution that excludes the voided layer; the witness derived from those traces becomes unadmissible in that resolution. A reasoning sentence whose grounding constructor cited the voided witness fails to type-check through that resolution but remains admissible through any resolution that still includes the layer. This is the same provenance discipline `class` and `property` resources follow.

## The four-step `ValidateJustification` check

[D39 §4.3](../../../design/d39-justification-logic.md) specifies what the institution verifies for every `ReasoningSentence` that commits. AutoOnLoad fires it; the kernel rejects the commit if any step fails.

1. **Property decoding.** Read `reasoning:proposition`, `reasoning:justification`, `reasoning:certificate` from the sentence resource. The proposition and certificate are D47-encoded type-expression values; the justification is a chain-mirrored inductive value over the seven `JustificationTerm` constructors. Decode each into a kernel `Exp` via the standard codecs. Missing or malformed properties produce structured `MalformedSentence` diagnostics.

2. **Proposition typing.** Type-check the decoded proposition at `Prop` ([ESL §7.1](../../esl/07-type-theory-primer.md#7-1-universes-the-unified-sortn-ladder-with-prop-at-the-bottom)). A proposition that doesn't live in `Prop` (e.g., the author wrote a `Set`-typed expression by accident) is rejected with `PropositionNotInProp`.

3. **Certificate type-checking.** Construct the expected type `JustifiedBy(justification, proposition)` and run the kernel's NbE checker against it. The check walks the certificate's constructor tree; at every grounding constructor it consults the witness index to synthesize the implicit `ChainWitness` argument. Failures from this step:
   - `NoAdmittedChainWitness { category, iri, prop_hash, expected_canonical_proposition }` — the witness index has no entry for the cited (category, iri, proposition) triple. The diagnostic names the trace shape the author needs to commit.
   - `CertificateTypeMismatch { expected, got }` — the certificate's type doesn't match the expected `JustifiedBy(...)` type. Usually means the certificate references a constructor that doesn't match the justification's shape (e.g., using `JustifiedBy.observed` to ground a `DeclaredEvidence` term).
   - `IndexMismatch` — the kernel's pattern unification rejected one of the indexed-family elaborations; typically a constructor's conclusion indices don't match the expected target.

4. **Verdict emission.** On success, the institution emits a `Verdict::Holds` resource paired with the sentence. Subsequent reasoning sentences can cite the sentence via `DerivedEvidence(sentence_iri)` and the witness index admits `IsDerivedAs(sentence_iri, sentence_proposition)` automatically — the sentence's `ProgramTrace` companion is what populates the witness index entry.

All four must pass for `Verdict::Holds`. Any failure produces `Verdict::Fails` with a typed diagnostic; the commit is rejected. The structured diagnostic shape lives in [`crates/eigenius-reasoning/src/diagnostic.rs`](../../../../crates/eigenius-reasoning/src/diagnostic.rs).

## Walking a worked example — drug-screening end-to-end

The capstone fixture at [`crates/eigenius-reasoning/tests/fixtures/drug_screening.esl`](../../../../crates/eigenius-reasoning/tests/fixtures/drug_screening.esl) walks the cycle that closes between the verdict and the raw measurement readings the proof transitively depends on. Read forward, the chain is:

```text
HasLowIC50, StrongInhibitor classes        [PopulationLevel-marked predicates]
  ↑ canonical_proposition
rule_strong                                [DeclaredResource — literature rule]
  │ ↑ resource
  │  rule_strong_trace                     [DeclarationTrace — admits IsDeclaredAs]
  │
  ↑ canonical_proposition
claim_eig0291_lowic50                      [StatisticalAnalysisPlan (DerivedResource subclass)]
  │ ↑ resource
  │  claim_eig0291_lowic50_trace           [ProgramTrace — admits IsDerivedAs]
  │ ↑ sample_set
  │  m_eig0291_sampleset                   [SampleSetResource holding raw IC50 reads]
  │  │ ↑ sample_set_value
  │  │  stats:SingleSampleEstimate([72.0, 85.0, 100.0], BiologicalReplication())
  │  ↑ resource
  │   m_eig0291_sampleset_trace            [ObservationTrace — admits IsObservedAs]
  │
  ↑ App(DeclaredEvidence, DerivedEvidence)
concl_eig0291_strong                       [ReasoningSentence]
  ↑ ValidateJustification AutoOnLoad
Verdict("Holds")                           [the verdict]
```

The reasoning sentence's certificate uses [`JustifiedBy.app`](../../esl/09-institutions.md#9102-the-justifiedby-certificate-predicate) to compose two sub-certificates — `declared(rule_strong, ...)` consuming the `IsDeclaredAs` witness admitted from `rule_strong_trace`, and `derived(claim_eig0291_lowic50, ...)` consuming the `IsDerivedAs` witness admitted from `claim_eig0291_lowic50_trace`. At commit, the kernel walks the certificate, hits both grounding constructors, queries the layer's witness index, finds both keys, and admits the witnesses. The certificate type-checks; the verdict is Holds.

If you void the layer containing `rule_strong_trace` (or `claim_eig0291_lowic50_trace`), the witness index loses the corresponding key in any chain resolution that excludes the voided layer, and the certificate fails to type-check through that resolution with a `NoAdmittedChainWitness` diagnostic. The sentence remains valid through any resolution that still includes the layer — the proof's validity is layer-scoped, not absolute.

Every byte that went into the verification — the literature rule's text, the three raw IC50 readings, the statistics-institution recomputation that turned them into a claim verdict, the certificate's tree of grounding constructors — sits on the chain as a typed, queryable, content-addressed resource. The audit trail is mechanical: you can run the certificate through the kernel offline and confirm it type-checks against the same witnesses without trusting any of the actors that produced the chain.

## Authoring your own reasoning sentence

The high-level shape, modeled on the drug-screening fixture:

1. **Author the domain vocabulary.** Declare the propositional predicates the reasoning will use. Mark scope where relevant ([ESL §4.5a multi-class data](../../esl/04-declarations.md#4-5a-multi-class-data-declarations-marker-classes-d52-12)):

   ```esl
   data screen:HasLowIC50 : core:string -> Prop, stats:PopulationLevel { }
   data screen:StrongInhibitor : core:string -> Prop, stats:PopulationLevel { }
   ```

2. **Commit the grounding artifacts.** Each grounding constructor needs a chain artifact (axiom, observed resource, derived resource, verified resource) plus its matching trace. For a literature rule:

   ```esl
   resource screen:rule_strong : reflection:DeclaredResource {
       reflection:declared_by = "literature:smith_et_al_2024";
       reflection:rationale   = "IC50 < 100 nM is the standard threshold.";
       reflection:canonical_proposition = type_expr(
           screen:HasLowIC50("urn:eigenius:demo:screen:EIG_0291")
           ->
           screen:StrongInhibitor("urn:eigenius:demo:screen:EIG_0291")
       );
   }

   resource screen:rule_strong_trace : reflection:DeclarationTrace {
       reflection:resource    = screen:rule_strong;
       reflection:declared_by = "literature:smith_et_al_2024";
       reflection:timestamp   = "2026-04-10T09:00:00Z";
   }
   ```

   For a `DerivedEvidence` target, a [D52 StatisticalAnalysisPlan](../statistics-institution/README.md) is the typical shape; its `canonical_proposition` is what the witness key hashes.

3. **Author the reasoning sentence.** Three required slots — proposition, justification, certificate — all D47-encoded via [`type_expr(...)`](../../esl/05-expressions.md#5-14a-type_expr-eigentt-type-expressions):

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
               screen:HasLowIC50("urn:eigenius:demo:screen:EIG_0291"),
               screen:StrongInhibitor("urn:eigenius:demo:screen:EIG_0291"),
               DeclaredEvidence("urn:eigenius:demo:screen:rule_strong"),
               DerivedEvidence("urn:eigenius:demo:screen:claim_eig0291_lowic50"),
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

4. **Commit.** Load the fixture (`eigenius load <doc>`). The reasoning institution's `ValidateJustification` AutoOnLoad gate fires automatically; success → admit, failure → reject with a structured diagnostic naming the failing clause.

## Composition with the statistics institution

The `DerivedEvidence(claim_iri)` ctor in the example above cites a [D52 StatisticalAnalysisPlan](../statistics-institution/README.md). The statistics institution's `validate_analysis_plan` AutoOnLoad gate has already fired on the claim at commit, recomputed it from raw replicates, and emitted a `Verdict::Holds`. The claim resource carries `canonical_proposition = HasLowIC50(EIG_0291)`; its `ProgramTrace` admits the `IsDerivedAs` witness in the witness index; the reasoning sentence's `derived(...)` certificate constructor consumes that witness mechanically.

No bridge code, no manual handoff. The composition works because D52's emitted artifact is shaped like every other chain-resident derived resource — it carries `canonical_proposition` in the same slot, gets a `ProgramTrace` of the same shape, lands in the witness index by the same path. The reasoning institution doesn't know D52 exists; it just sees an `IsDerivedAs` witness with the right hash.

This is the load-bearing composition pattern: **D52 turns raw data into a propositional verdict; D39 grounds that verdict in a reasoning chain**. The full walkthrough — committing raw IC50 readings, watching D52 produce the claim verdict, then committing a reasoning sentence that grounds the claim in `DerivedEvidence` — is the [composition guide §7 stats+reasoning walkthrough](../../composition/07-stats-and-reasoning-walkthrough.md).

## Troubleshooting

- **`Verdict::Fails` with `NoAdmittedChainWitness`** — the witness index has no entry for the cited (category, iri, proposition). Three common causes:
  1. The grounding resource was never committed (`rule_strong` IRI is wrong, or the resource is in a layer outside the current chain resolution);
  2. The companion trace was never committed (a `DeclaredResource` without its `DeclarationTrace` doesn't admit `IsDeclaredAs`);
  3. The proposition shape doesn't match — the resource's `canonical_proposition` is structurally different from what the certificate constructor declares. The diagnostic names the expected canonical proposition; compare it field by field against what the resource actually carries.
- **`Verdict::Fails` with `CertificateTypeMismatch`** — usually means the wrong certificate constructor was used. `JustifiedBy.observed` consumes `IsObservedAs`, which only `ObservedResource`s admit; using it to ground a `DeclaredEvidence(axiom_iri)` term is a category mismatch. Match the certificate constructor name to the justification's grounding-ctor name (`declared` for `DeclaredEvidence`, `derived` for `DerivedEvidence`, etc.).
- **`Verdict::Fails` with `PropositionNotInProp`** — the `proposition` slot's `type_expr(...)` body lowered to a `Set`/`Type(n)`-typed expression instead of `Prop`. Common cause: the predicate's `data` declaration was declared with `: Set` (or no result-sort clause) instead of `: ... -> Prop`. Re-declare the predicate with a `Prop` result sort.
- **`Verdict::Fails` with `MalformedSentence`** — a required property is missing or has the wrong shape. Check that `proposition` and `certificate` are `type_expr(...)` values (not raw JSON) and that `justification` is a chain-inductive constructor application matching one of the seven `JustificationTerm` ctors.
- **Sentence type-checks in one chain resolution but fails in another** — voiding semantics. The witness index is derived from the layer's trace resources; voiding a layer removes its traces from the witness index in any resolution that excludes it. Confirm the resolution includes every layer containing a grounding artifact the certificate cites.

## Cross-references

- [**ESL §9.10 — D39 reasoning institution surface**](../../esl/09-institutions.md#9-10-the-reasoning-institution-d39-justification-logic) — surface syntax reference: the seven `JustificationTerm` constructors, the nine `JustifiedBy` certificate constructors, the `ReasoningSentence` resource shape, and the worked example this sub-guide expands on.
- [**ESL §6.4a — Witness predicates**](../../esl/06-resources-types-and-the-layer.md#6-4a-witness-predicates-admitting-propositions-from-layer-state) — the kernel-side view of the four `ChainWitness.Is*As` families.
- [**ESL §7.1 — Universes**](../../esl/07-type-theory-primer.md#7-1-universes-the-unified-sortn-ladder-with-prop-at-the-bottom) — `Prop`, proof irrelevance, and why distinct evidence chains for the same proposition produce judgmentally-equal certificates.
- [**Statistics institution tutorial**](../statistics-institution/README.md) — the D52 institution whose verdicts are the canonical `DerivedEvidence` targets for D39 reasoning sentences.
- [**Composition guide §7**](../../composition/07-stats-and-reasoning-walkthrough.md) — full statistics-plus-reasoning walkthrough showing the pipeline raw data → D52 verdict → D39 reasoning composition.
- [**D39 Justification Logic**](../../../design/d39-justification-logic.md) — design spec.
- [**D49 Chain-witness machinery**](../../../design/d49-chainwitness-machinery.md) — companion spec for the per-layer witness index this tutorial walks through.
- [**D46 Prop universe and proof irrelevance**](../../../design/d46-prop-universe-and-proof-irrelevance.md) — the universe-formation rules the reasoning predicates depend on.
- [**D47 Chain-mirrored EigenTT type fragment**](../../../design/d47-chain-mirrored-eigentt-type-fragment.md) — the codec the proposition and certificate slots ride on.
- [**D48 Indexed inductive families**](../../../design/d48-indexed-inductive-families.md) — the type theory that makes `JustifiedBy : JustificationTerm -> Prop -> Type 0` expressible.
- [`crates/eigenius-reasoning/`](../../../../crates/eigenius-reasoning/) — institution implementation.
- [`ontologies/reasoning/reasoning.esl`](../../../../ontologies/reasoning/reasoning.esl) — ontology source.
- [`crates/eigenius-reasoning/tests/fixtures/drug_screening.esl`](../../../../crates/eigenius-reasoning/tests/fixtures/drug_screening.esl) — the worked example this tutorial walks.
