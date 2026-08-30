# Reasoning institution tutorial

Slow-walk worked example of the platform's first reasoning institution: D39 Justification Logic. Walks the closed audit chain end-to-end against a concrete drug-screening scenario — from the chain-committed `Verdict::Holds` back through the `justification:Certificate` certificate, the chain witnesses that admitted the grounding constructors, the trace resources that admitted the witnesses, and the raw chain artifacts those traces point at.

Read this if you want to know what a `justification:Conclusion` commit actually does, how the `App(Declared, DerivedEvidence)` composition picks up groundings from anywhere on the chain, or how a D52 statistics verdict becomes a citable evidence node in a D39 proof.

Surface reference: [**ESL §9.10 — D39 reasoning institution**](../../esl/09-institutions.md#9-10-the-reasoning-institution-d39-justification-logic). Design spec: [**D39 Justification Logic**](../../../design/d39-justification-logic.md). Companion design: [**D49 chain-witness machinery**](../../../design/d49-chainwitness-machinery.md). Implementation: [`crates/eigenius-reasoning/`](../../../../crates/eigenius-reasoning/). Ontology: [`ontologies/justification/justification.esl`](../../../../ontologies/justification/justification.esl).

## Why reasoning is different from the Lean institution

[Lean](../lean-institution/README.md) is a *verification* institution: chain authors commit a Lean proof term and the institution re-checks the proof against an exported theorem statement. The proof is its own thing, authored in Lean, exported as bytes, re-checked by a bundled `nanoda_lib`. The chain attests that the proof checks.

D39 is a *reasoning* institution: chain authors commit a triple of (proposition, justification term, certificate) where the certificate is a `justification:Certificate(justification, proposition)` term — an inhabitant of an indexed inductive family declared in the chain's own type theory. The grounding terms reference chain artifacts (an axiom, an observed measurement, a derived claim, a verified Lean proof) and the kernel admits the corresponding chain witnesses by resolving, at type-check time, the one chain resource each cited IRI names. The chain attests both that the certificate type-checks *and* that every cited chain artifact actually exists.

Two consequences of this difference shape the rest of the tutorial:

1. **The validator is the kernel.** No bundled external checker. No mirror-anchor consistency check. The validator for a D39 reasoning sentence is a direct call to the kernel's NbE checker — the same one that type-checks every other ESL program. Implementation lives in [`crates/eigenius-reasoning/src/validate.rs`](../../../../crates/eigenius-reasoning/src/validate.rs) and is ~200 lines: decode three properties from the sentence resource, construct the expected `justification:Certificate(justification, proposition)` type, call `check`. TCB is bounded by the kernel.
2. **Composition is the point.** A reasoning sentence's `App` and `SpecStr` combinators let one chain artifact's verdict ground a downstream conclusion. The motivating composition: a [D52 StatisticalAnalysisPlan](../statistics-institution/README.md) verdict (`HasLowIC50(EIG_0291)`) grounds a `DerivedEvidence` ctor, a literature rule (`HasLowIC50(c) -> StrongInhibitor(c)`) grounds a `Declared` ctor, and `App` composes them into `StrongInhibitor(EIG_0291)`. The composition is mechanical — every step type-checks against the kernel, no out-of-band convention bridges the layers.

This means a reasoning institution is registered with `runtime: in_process`. There is no env-image build, no orchestrator round-trip; the verifier runs synchronously inside the kernel process at commit time.

## The chain shapes the reasoning audit trail touches

Reasoning leaves a typed audit trail. The institution itself emits only the verdict; the other shapes are pre-existing chain artifacts the reasoning sentence cites or that witness admission reads.

| Resource | Role |
|---|---|
| `axiom` declaration → `eigentt:Axiom` | Author-asserted propositional statement ([ESL §4.4a](../../esl/04-declarations.md#4-4a-axiom-postulated-propositions-d46-10)). Paired with a `DeclarationTrace` to admit `IsDeclaredAs`. |
| `reflection:DeclaredResource` + `DeclarationTrace` | Any chain-resident declared assertion (literature rule, asserted theorem, marker resource). The matching trace admits `IsDeclaredAs(iri, canonical_proposition)`. |
| `reflection:ObservedResource` + `ObservationTrace` | Bench measurement, instrument log entry. The matching trace admits `IsObservedAs(iri, canonical_proposition)`. |
| `reflection:DerivedResource` + `ProgramTrace` | Any derived artifact committed together with a program trace pointing at it. The trace admits `IsDerivedAs(iri, canonical_proposition)` on the target. |
| `reflection:InstitutionEmittedDerivation` | Institution output the kernel stamped on dispatch — a [D52 `StatisticalAnalysisResult`](../statistics-institution/README.md) is the canonical case. **Self-attesting**: it admits `IsDerivedAs(iri, canonical_proposition)` on its own IRI, reading the property off itself, with no companion trace. Note that this is the *result*, not the `StatisticalAnalysisPlan`: the plan carries no propositional content of its own. |

**No trace class admits `IsVerifiedAs`.** `trace_category` (`kernel/src/layer/witness_index.rs`) has exactly three arms — `DeclarationTrace → Declared`, `ObservationTrace → Observed`, `ProgramTrace → Derived` — and no `VerificationTrace` arm. `reflection:VerifiedResource` is `subclass_of DerivedResource`, so a `VerifiedResource` paired with a `ProgramTrace` admits `IsDerivedAs` like any other derived artifact; nothing promotes that to `IsVerifiedAs`. A [Lean institution](../lean-institution/README.md) proof therefore admits no witness at all at this revision: D49 §7 routes it through a comorphism-reified `VerifiedPropositionView`, and that comorphism is not built. The only `IsVerifiedAs` the kernel admits today is a committed `justification:Conclusion` self-attesting (below). The one coercion runs in the opposite direction to the sentence in the table this note replaced: a `Derived` lookup *also* tries the matching `Verified` key, so a `Verified` entry satisfies a `derived(...)` citation — never the reverse.

The reasoning institution then emits two more shapes:

| Resource | Role |
|---|---|
| `justification:Conclusion` | The chain-resident reasoning step: proposition + justification + certificate. AutoOnLoad fires `ValidateJustification` on commit; type-checks the certificate against `justification:Certificate(justification, proposition)`. |
| `Verdict` (`ctor_name: "Holds" / "Fails"`) | The institution's outcome. Committed alongside a `RuntimeInvocation` provenance record. Failed verdicts reject the commit. |

`justification:Conclusion` is declared `subclass_of reflection:DerivedResource`, but it carries **neither** a `ProgramTrace` **nor** a `reflection:canonical_proposition`. Its `requires` list is `proposition`, `justification`, `certificate`, on the class's own reasoning that the certificate *is* the derivation. [D54 §1](../../../design/d54-reasoning-lemma-citation.md) opens on precisely that: because a bare sentence had neither of the two shapes witness admission recognised, citing a prior conclusion used to fail with `no admitted IsDerivedAs witness`.

What closed it is a third recognition route rather than a stamped trace. `layer_admits_witness` matches a resource whose `is_a` includes `justification:Conclusion` and emits a **`Verified`** witness keyed on the sentence's own IRI, hashing the sentence's own `justification:proposition` (`emit_from_reasoning_sentence`). A later `verified(prior_iri, P)` citation matches it directly; a `derived(prior_iri, P)` citation matches through the `Verified → Derived` coercion. Soundness sits at the commit boundary, not in the emitter: `autoonload_dispatch` rejects a `Fails` verdict, so every committed sentence passed its gate (D54 §3).

## D49 witness admission — how the kernel admits grounding witnesses

When the type-checker elaborates a `justification:Certificate.declared` / `.observed` / `.derived` / `.verified` grounding constructor, it needs to produce a value of the corresponding `IsDeclaredAs(iri, P)` / `IsObservedAs(iri, P)` / `IsDerivedAs(iri, P)` / `IsVerifiedAs(iri, P)` predicate. These predicates have **zero surface constructors** — the kernel admits inhabitants only by consulting layer state.

### Admission is a direct lookup — nothing is materialized

A `WitnessKey` names the resource it grounds, so "does this layer admit this key" is answerable by going to that one resource:

```rust
pub struct WitnessKey {
    pub category: WitnessCategory,   // Declared / Observed / Derived / Verified
    pub iri: Iri,                    // the grounded resource
    pub prop_hash: [u8; 32],         // SHA-256 of the D47-encoded proposition
}
```

`layer_admits_witness(&Layer, &WitnessKey) -> bool` ([`kernel/src/layer/witness_index.rs`](../../../../kernel/src/layer/witness_index.rs)) answers in three steps and builds nothing:

1. **Skip.** `LayerHandle::has_witness_candidates` is stamped at write time over the layer's resources. A layer holding no Trace, no `InstitutionEmittedDerivation` and no `justification:Conclusion` answers `false` with no probe at all — a lexicon layer stops here.
2. **Self-attesting.** `Layer::get_resource` on the key's IRI, which is layer-local. If that resource is a `justification:Conclusion` and the key's category is `Verified`, or an `InstitutionEmittedDerivation` and the category is `Derived`, build the key it would emit and compare it to the key asked for.
3. **Trace-attested.** Find a Trace resource *defined in this layer* whose `reflection:resource` points at the key's IRI — through the triple index when the layer is already stored, by iterating the layer when it is still in flight, which is the case during `autoonload_dispatch`. Resolve the target (a chain walk, since a trace here may attest a resource in an ancestor), read its `reflection:canonical_proposition` — or fall back to the D39 §4.1 default `Asserts(target_iri)` when it carries none — hash it, and compare.

An earlier implementation did materialize an index: `build_witness_index` walked the layer at construction and cached a `BTreeMap<WitnessKey, ()>` in a `OnceLock` on the `Layer`, and lookup was a membership test. **D66 slice 0 removed all of it.** The map cost memory proportional to the layer's trace count for the layer's whole lifetime and reduced every miss to a bare `false` carrying no reason; direct lookup is O(1) in memory and holds the specific resource at the point of the decision. There is no `Layer::chain_witness_index` method and nothing is cached.

Both ends of the key hash the proposition the same way. The emitter *decodes* the stored `Value::Json` against the layer before hashing (`hash_stored_proposition`), so a folded definition name and its unfolded body land on the same hash as the checker's readback of the term the author wrote (D66 §4). A stored proposition that fails to decode emits no witness, and logs why, rather than failing silently.

### Lookup at type-check time

When the kernel encounters `justification:Certificate.derived(iri, P)` and needs to fill in `witness : IsDerivedAs(iri, P)`, `synthesize_chain_witness` runs the D49 §5 algorithm:

```text
1. prop_hash = sha256(canonical_cbor(encode_type(P)))     // D47 codec
2. key       = WitnessKey { category: Derived, iri, prop_hash }
3. for layer in the parent chain, top-down:
     if layer_admits_witness(layer, &key):
         return Ok(Val::ChainWitness(key))
     // D49 §4: a Verified entry satisfies a Derived lookup, not the reverse.
     if key.category == Derived
        && layer_admits_witness(layer, &WitnessKey { category: Verified, ..key }):
         return Ok(Val::ChainWitness(key))
4. return Err("no admitted IsDerivedAs witness for IRI ... ")
```

The walk reuses the existing `Arc<Layer>` parent-chain walk (the one resource resolution uses) — no new traversal abstraction. First hit wins, which is sound because layer immutability means a once-admitted witness stays admitted in all descendants. The `Val::ChainWitness` value carries no payload beyond the key; proof irrelevance ([ESL §7.1](../../esl/07-type-theory-primer.md#7-1-universes-the-unified-sortn-ladder-with-prop-at-the-bottom)) makes any two witnesses of the same `(category, iri, P)` definitionally equal.

A miss returns a **free-form `String`**, not a structured error value. It names the predicate family (`IsDeclaredAs` / `IsObservedAs` / `IsDerivedAs` / `IsVerifiedAs`), the IRI, the property the resource would have to carry, and the `justification:Certificate.*` constructor that would become well-typed. There is no diagnostic enum anywhere on this path — see [the check](#the-four-step-validatejustification-check) below.

### Voiding semantics

Witnesses are derived state, recomputed at every lookup. Voiding a layer removes its trace resources from any chain resolution that excludes the voided layer, and the witness those traces admitted becomes unadmissible in that resolution. A reasoning sentence whose grounding constructor cited the voided witness fails to type-check through that resolution but remains admissible through any resolution that still includes the layer. This is the same provenance discipline `class` and `property` resources follow.

## The four-step `ValidateJustification` check

[D39 §4.3](../../../design/d39-justification-logic.md) specifies what the institution verifies for every `justification:Conclusion` that commits. AutoOnLoad fires it; the kernel rejects the commit if any step fails.

1. **Property decoding.** Read `justification:proposition`, `justification:term` and `justification:certificate` from the sentence resource. The proposition and certificate are D47-encoded type-expression values, decoded to a kernel `Exp` by `decode_type`; the justification is lifted into a `Val::InductiveVal` typed at `justification:Term` through the institution's own `extract_typed` route (`extract_justification`), so the chain-to-`Val` translation rides the standard institution surface. A property that is present but does not decode yields `Verdict::Fails` carrying `malformed proposition: ...` or `malformed certificate: ...`. A property that is *absent* does not reach a verdict at all: `required_property` returns an `InstitutionError::ComputationFailed`, on the reasoning that the class's `requires` enforcement has already rejected such a sentence at commit.

2. **Proposition typing.** Type-check the decoded proposition at `Prop` (= `Sort(0)`, [ESL §7.1](../../esl/07-type-theory-primer.md#7-1-universes-the-unified-sortn-ladder-with-prop-at-the-bottom)), then `eval` it for the index slot of the expected type. A proposition that does not live in `Prop` — the author wrote a `Set`-typed expression by accident — yields `Verdict::Fails` carrying `proposition does not type-check at Prop: ...` with the checker's own error appended.

3. **Certificate type-checking.** Resolve the `justification:Certificate` inductive from the layer and build the expected type directly at the `Val` layer: `Val::InductiveType { decl: justification:Certificate, params: [], indices: [justification_val, proposition_val] }`, no `Exp` round-trip. Run the kernel's NbE `check` against it. The check walks the certificate's constructor tree and, at every grounding constructor, synthesizes the implicit `ChainWitness` argument by the lookup above. **Every** failure of this step — a witness that no layer admits, a certificate constructor that does not match the justification's shape, an indexed-family elaboration the pattern unifier rejects — arrives as one string inside `certificate does not type-check against \`justification:Certificate(justification, proposition)\`: ...`. The kernel's type error is what distinguishes them.

4. **Verdict emission.** On success the institution emits a `Verdict` resource with `ctor_name = "Holds"`. The gate stamps nothing on the sentence itself; a later sentence can nonetheless cite it via `DerivedEvidence(sentence_iri)` or `Verified(sentence_iri)`, because a committed `justification:Conclusion` self-attests a `Verified` witness at its own IRI and the `Verified → Derived` coercion covers the `derived` form.

All four must pass for `Verdict::Holds`. Any failure produces `Verdict::Fails` carrying a single `urn:eigenius:institution:diagnostic` **string**, and the commit is rejected.

**There is no structured diagnostic taxonomy.** The whole handler is ~200 lines in [`crates/eigenius-reasoning/src/validate.rs`](../../../../crates/eigenius-reasoning/src/validate.rs); the crate has no `diagnostic` module, and no `NoAdmittedChainWitness`, `CertificateTypeMismatch`, `IndexMismatch`, `PropositionNotInProp` or `MalformedSentence` type exists anywhere in the workspace. Those names appear in [D49 §5](../../../design/d49-chainwitness-machinery.md) and D51 as *specified* shapes; they were never built. Tooling that reads a failed verdict must match on the message text, not on a variant. (This guide asserted the taxonomy, and named a `crates/eigenius-reasoning/src/diagnostic.rs` that has never existed, until the section was corrected on 2026-08-20.)

## Walking a worked example — drug-screening end-to-end

The capstone fixture at [`crates/eigenius-reasoning/tests/fixtures/drug_screening.esl`](../../../../crates/eigenius-reasoning/tests/fixtures/drug_screening.esl) walks the cycle that closes between the verdict and the raw measurement readings the proof transitively depends on. Read forward, the chain is:

```text
HasLowIC50, StrongInhibitor                     [PopulationLevel-marked predicates in Prop]
  ↑ canonical_proposition
rule_strong                                     [DeclaredResource — literature rule]
  │ ↑ reflection:resource
  │  rule_strong_trace                          [DeclarationTrace — admits IsDeclaredAs]
  │
  ↑ canonical_proposition
bridge_eig0291_lowic50                          [DeclaredResource — statistical → domain]
  │ ↑ reflection:resource
  │  bridge_eig0291_lowic50_trace               [DeclarationTrace — admits IsDeclaredAs]
  │
claim_eig0291_lowic50                           [StatisticalAnalysisPlan — carries no
  │                                              proposition of its own]
  │ ↑ sample_set
  │  m_eig0291_sampleset                        [SampleSetResource — raw IC50 reads
  │  │                                           72, 85, 100 nM]
  │  ↑ reflection:resource
  │   m_eig0291_sampleset_trace                 [ObservationTrace — admits IsObservedAs]
  │ ↑ directionality
  │  witness_kinaseglo_floor                    [ImpossibilityWitness — licenses the
  │                                              one-sided path]
  ↓ per-effect output of the D52 verifier
claim_eig0291_lowic50:result:main_effect        [StatisticalAnalysisResult +
  │                                              InstitutionEmittedDerivation —
  │                                              self-admits IsDerivedAs at its own IRI]
  │
  ↑ App(Declared(rule),
  │     App(Declared(bridge), DerivedEvidence(result)))
concl_eig0291_strong                            [justification:Conclusion]
  ↑ ValidateJustification AutoOnLoad
Verdict("Holds")                                [the verdict]
```

The reasoning sentence's certificate is two nested [`justification:Certificate.app`](../../esl/09-institutions.md#9102-the-justifiedby-certificate-predicate) calls composing three sub-certificates. The inner `app` applies the bridge to the statistical result: `declared(bridge, lt(mean_of(s), 100.0) -> HasLowIC50(EIG_0291))` against `derived(result, lt(mean_of(s), 100.0))`, yielding `HasLowIC50(EIG_0291)`. The outer `app` applies the literature rule to that, yielding `StrongInhibitor(EIG_0291)`. Both grounding constructors are written with the trailing witness slot elided; the kernel fills each in.

Two details of the shape are worth naming, because both are easy to get wrong when authoring:

- **The `DerivedEvidence` target is the per-effect result, not the plan.** `claim_eig0291_lowic50` is a `StatisticalAnalysisPlan` and carries no `canonical_proposition` — the verifier derives the proposition from `(dispatch, effect_size, directionality)` and emits it on a `StatisticalAnalysisResult` at `{plan_iri}:result:{effect_name}`, which the kernel stamps `reflection:InstitutionEmittedDerivation`. That marker is what makes the result self-attest `IsDerivedAs`. Citing the plan's IRI admits no witness.
- **The statistical proposition and the domain proposition are different propositions,** and the bridge between them is a chain-resident `DeclaredResource` rather than something the statistics author folded into the plan. The chain attests only what the verifier proved — `lt(mean_of(s), 100.0)` — and the translation into `HasLowIC50` is itself citable and auditable.

The fixture pre-authors the `StatisticalAnalysisResult` rather than dispatching the statistics institution, because it exercises witness admission directly; institution dispatch is covered in the `eigenius-statistics` crate's own end-to-end tests.

At commit, the kernel walks the certificate, hits the three grounding constructors, resolves each cited IRI against the layer chain, and admits the two `IsDeclaredAs` witnesses from the declaration traces and the one `IsDerivedAs` witness from the result's self-attestation. The certificate type-checks; the verdict is Holds.

If you void the layer containing `rule_strong_trace` (or `bridge_eig0291_lowic50_trace`), the corresponding key stops being admissible in any chain resolution that excludes the voided layer, and the certificate fails to type-check through that resolution. The sentence remains valid through any resolution that still includes the layer — the proof's validity is layer-scoped, not absolute.

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

   For a `DerivedEvidence` target you need one of two shapes: a derived resource carrying `reflection:canonical_proposition` **plus** a `ProgramTrace` pointing at it, or an institution-emitted derivation, which the kernel stamps `reflection:InstitutionEmittedDerivation` and which self-attests off its own `canonical_proposition` with no trace. Either way it is *that resource's* `canonical_proposition` the witness key hashes, so the proposition your `derived(...)` constructor writes has to be that one and not a restatement of it. A [D52 StatisticalAnalysisPlan](../statistics-institution/README.md) is not itself such a shape: the plan carries no proposition, and the citable artifact is the per-effect `StatisticalAnalysisResult` at `{plan_iri}:result:{effect_name}` that the verifier emits.

3. **Author the reasoning sentence.** Three required slots — proposition, justification, certificate — all D47-encoded via [`type_expr(...)`](../../esl/05-expressions.md#5-14a-type_expr-eigentt-type-expressions):

   ```esl
   resource screen:concl_eig0291_strong : justification:Conclusion {
       justification:subject_iri = "urn:eigenius:demo:screen:EIG_0291";

       justification:proposition = type_expr(
           screen:StrongInhibitor("urn:eigenius:demo:screen:EIG_0291")
       );

       justification:term = App(
           Declared("urn:eigenius:demo:screen:rule_strong"),
           DerivedEvidence("urn:eigenius:demo:screen:derivation_lowic50")
       );

       justification:certificate = type_expr(
           app(
               screen:HasLowIC50("urn:eigenius:demo:screen:EIG_0291"),
               screen:StrongInhibitor("urn:eigenius:demo:screen:EIG_0291"),
               Declared("urn:eigenius:demo:screen:rule_strong"),
               DerivedEvidence("urn:eigenius:demo:screen:derivation_lowic50"),
               declared(
                   "urn:eigenius:demo:screen:rule_strong",
                   screen:HasLowIC50("urn:eigenius:demo:screen:EIG_0291")
                   ->
                   screen:StrongInhibitor("urn:eigenius:demo:screen:EIG_0291"),
                   screen:HasLowIC50("urn:eigenius:demo:screen:EIG_0291")
               ),
               derived(
                   "urn:eigenius:demo:screen:derivation_lowic50",
                   screen:HasLowIC50("urn:eigenius:demo:screen:EIG_0291"),
                   screen:HasLowIC50("urn:eigenius:demo:screen:EIG_0291")
               )
           )
       );
   }
   ```

   The example is abbreviated to a single `app`, and `derivation_lowic50` stands for a derived artifact whose `canonical_proposition` is already the domain proposition `HasLowIC50(EIG_0291)`. When the derived artifact is a D52 result its proposition is statistical — `lt(mean_of(s), 100.0)` — and a second `app` over a declared statistical-to-domain bridge is what carries it into domain vocabulary. That is the shape the drug-screening fixture uses.

4. **Commit.** Load the fixture (`eigenius load <doc>`). The reasoning institution's `ValidateJustification` AutoOnLoad gate fires automatically; success → admit, failure → reject with a `Verdict::Fails` whose diagnostic string names the failing step.

## Composition with the statistics institution

The `DerivedEvidence(...)` ctor in the worked example cites the output of a [D52 StatisticalAnalysisPlan](../statistics-institution/README.md). The statistics institution's `validate_analysis_plan` AutoOnLoad gate has already fired on the plan at commit, recomputed it from raw replicates, and emitted two things: a `Verdict`, and one `StatisticalAnalysisResult` per effect carrying the derived `canonical_proposition`. The kernel stamps that result `reflection:DerivedResource` + `reflection:InstitutionEmittedDerivation` and sets `reflection:from_subject` to the plan. The marker class is what `layer_admits_witness` matches on its self-attesting route, so the result admits `IsDerivedAs(result_iri, canonical_proposition)` off its own IRI — no `ProgramTrace` is emitted, and none is needed. The reasoning sentence's `derived(...)` constructor consumes that witness mechanically.

The `Verdict` itself is not citable: under the D52 verdict-versus-derivation split it carries no `canonical_proposition`, so no witness key can be built for it. The citable artifact is always the proposition-bearer.

No bridge code, no manual handoff. The composition works because D52's emitted artifact is shaped like every other institution-emitted derivation — it carries `canonical_proposition` in the same slot and is recognised by the same marker class. The reasoning institution doesn't know D52 exists; it just sees an `IsDerivedAs` witness with the right hash.

This is the load-bearing composition pattern: **D52 turns raw data into a propositional verdict; D39 grounds that verdict in a reasoning chain**. The full walkthrough — committing raw IC50 readings, watching D52 produce the claim verdict, then committing a reasoning sentence that grounds the claim in `DerivedEvidence` — is the [composition guide §7 stats+reasoning walkthrough](../../composition/07-stats-and-reasoning-walkthrough.md).

## Troubleshooting

A failed gate gives you one string on the `Verdict`, under `urn:eigenius:institution:diagnostic`. Match it by prefix.

- **`certificate does not type-check against justification:Certificate(justification, proposition): no admitted Is…As witness for IRI …`** — no layer in the resolution admits the cited `(category, iri, proposition)`. Four common causes:
  1. The grounding resource was never committed, or the IRI is wrong, or it sits in a layer outside the current chain resolution.
  2. The companion trace was never committed — a `DeclaredResource` without its `DeclarationTrace` admits nothing — or the trace is defined in a different layer from the one holding it, since the trace-attested route requires the trace to be *defined* in the layer where it is found.
  3. The proposition does not match: the resource's `canonical_proposition` is structurally different from what the certificate constructor writes. The message names the property the resource must carry; compare the two term by term.
  4. The cited IRI names a plan rather than the derivation the verifier emitted (see [composition](#composition-with-the-statistics-institution)), or a `Verdict` rather than the proposition-bearer.
- **The same prefix, with the kernel's own type error after it** — the certificate's shape does not match the justification's. Every mismatch of constructor, index or type arrives through this one path, so read the kernel's error: `justification:Certificate.observed` consumes `IsObservedAs`, which only an `ObservationTrace` admits, and using it to ground a `Declared(iri)` term is a category mismatch. Match the certificate constructor name to the justification's grounding-ctor name — `declared` for `Declared`, `derived` for `DerivedEvidence`, and so on.
- **`proposition does not type-check at Prop: …`** — the `proposition` slot's `type_expr(...)` body lowered to a `Set`/`Type(n)`-typed expression instead of `Prop`. Common cause: the predicate's `data` declaration was written with `: Set`, or with no result-sort clause, instead of `: … -> Prop`. Re-declare the predicate with a `Prop` result sort.
- **`malformed proposition: …` / `malformed certificate: …`** — the D47 decode failed. Check that `proposition` and `certificate` are `type_expr(...)` values rather than raw JSON.
- **`justification:Conclusion missing required … property`, arriving as an institution error rather than a verdict** — a required slot is absent. The class's `requires` enforcement should have rejected this at commit; reaching the handler means the institution was dispatched against a resource that did not come through the commit path.
- **The gate is slow to reject** — admission keeps no cached index, so a *miss* walks to the root of the chain, and on a layer still in flight (which is the case during `autoonload_dispatch`) the trace-attested route iterates the layer rather than using the triple index. A measured case on `demo/prose-to-formulas` took 0.75 s to commit and 127 s to reject the same certificate shape.
- **Sentence type-checks in one chain resolution but fails in another** — voiding semantics. Admission is recomputed against the resolution's layers at every lookup; voiding a layer removes its traces from every resolution that excludes it. Confirm the resolution includes every layer holding a grounding artifact the certificate cites.

## Cross-references

- [**ESL §9.10 — D39 reasoning institution surface**](../../esl/09-institutions.md#9-10-the-reasoning-institution-d39-justification-logic) — surface syntax reference: the seven `justification:Term` constructors, the nine `justification:Certificate` certificate constructors, the `justification:Conclusion` resource shape, and the worked example this sub-guide expands on.
- [**ESL §6.4a — Witness predicates**](../../esl/06-resources-types-and-the-layer.md#6-4a-witness-predicates-admitting-propositions-from-layer-state) — the kernel-side view of the four `ChainWitness.Is*As` families.
- [**ESL §7.1 — Universes**](../../esl/07-type-theory-primer.md#7-1-universes-the-unified-sortn-ladder-with-prop-at-the-bottom) — `Prop`, proof irrelevance, and why distinct evidence chains for the same proposition produce judgmentally-equal certificates.
- [**Statistics institution tutorial**](../statistics-institution/README.md) — the D52 institution whose verdicts are the canonical `DerivedEvidence` targets for D39 reasoning sentences.
- [**Composition guide §7**](../../composition/07-stats-and-reasoning-walkthrough.md) — full statistics-plus-reasoning walkthrough showing the pipeline raw data → D52 verdict → D39 reasoning composition.
- [**D39 Justification Logic**](../../../design/d39-justification-logic.md) — design spec.
- [**D49 Chain-witness machinery**](../../../design/d49-chainwitness-machinery.md) — companion spec for the witness machinery this tutorial walks. Read it as design intent: it specifies the materialized per-layer index that D66 slice 0 replaced with direct lookup, and a structured diagnostic taxonomy that was never built.
- [**D46 Prop universe and proof irrelevance**](../../../design/d46-prop-universe-and-proof-irrelevance.md) — the universe-formation rules the reasoning predicates depend on.
- [**D47 Chain-mirrored EigenTT type fragment**](../../../design/d47-chain-mirrored-eigentt-type-fragment.md) — the codec the proposition and certificate slots ride on.
- [**D48 Indexed inductive families**](../../../design/d48-indexed-inductive-families.md) — the type theory that makes `justification:Certificate : justification:Term -> Prop -> Type 0` expressible.
- [`crates/eigenius-reasoning/`](../../../../crates/eigenius-reasoning/) — institution implementation.
- [`ontologies/justification/justification.esl`](../../../../ontologies/justification/justification.esl) — ontology source.
- [`crates/eigenius-reasoning/tests/fixtures/drug_screening.esl`](../../../../crates/eigenius-reasoning/tests/fixtures/drug_screening.esl) — the worked example this tutorial walks.
