# D49 — `ChainWitness` Machinery

*Status: design memo · June 2026*

*Companion documents: [D39 justification logic (v2 draft)](d39-justification-logic.md), [D46 Prop universe + axiom framework](d46-prop-universe-and-proof-irrelevance.md), [D47 chain-mirrored EigenTT type fragment](d47-chain-mirrored-eigentt-type-fragment.md), [D48 indexed inductive families](d48-indexed-inductive-families.md), [D41 commit pipeline](d41-commit-pipeline.md).*

*This memo settles the implementation shape of the `ChainWitness` predicate family that D39 v2 §5 introduces. It is the soundness boundary for the Reasoning institution — every grounding fact entering the type system passes through these witnesses — so getting the table location, the synthesis algorithm, the trace hook points, and the Lean checker integration right matters more than getting them done quickly. D39 implementation depends on this design landing first.*

---

## 1. Scope

In scope:

- The data structure backing the four `ChainWitness.IsXxAs` predicate families and where it lives in the kernel.
- The witness-synthesis algorithm: lookup key, parent-chain walk, hit/miss handling.
- The trace-emission relationship: how `DeclarationTrace` / `ObservationTrace` / `ProgramTrace` / `VerificationTrace` commits surface as admitted witnesses without requiring new D41 pipeline hooks.
- The `IsVerifiedAs` admission path — the one witness whose proposition `P` cannot be read directly from the user-authored verified resource because it is what the verifier's proof actually inhabits, expressed in the verifier's own type theory. §7 settles this via a Lean → Reasoning comorphism that reifies a chain-resident `VerifiedPropositionView` carrying the EigenTT-form proposition; the witness emitter then reads it uniformly with the other three families.

Out of scope:

- The `JustifiedBy` indexed inductive itself (covered by D39 §5 and authored via eigenius#72 Layer 2).
- The Reasoning institution's own D14 implementation — `extract_typed` / `reify` / the three query classes (`ValidateJustification` / `EntailmentQuery` / `ConsistencyCheck`) per D39 §4.3. The Reasoning institution is a normal D14 institution declared as an ontology resource and dispatched through the existing `InstitutionIndex` machinery; D49 only specifies the kernel-internal `ChainWitness` predicates its grounding constructors consume.
- Belief-revision semantics for `ReasoningSentence.refutes` (deferred to chain-merge work per D39 §9).
- The internals of each verification institution's backward translation (Lean → EigenTT, future Coq → EigenTT, etc.). Each institution owns the implementation of its outbound comorphism's transformation step; D49 specifies only that the comorphism exists and reifies a `VerifiedPropositionView` whose `canonical_proposition` the witness emitter reads.

## 2. What the witnesses are (recap from D39 §5)

Four `Prop`-typed predicate families, indexed by IRI and by the asserted proposition:

```
ChainWitness.IsDeclaredAs : core:iri → Prop → Prop
ChainWitness.IsObservedAs : core:iri → Prop → Prop
ChainWitness.IsDerivedAs  : core:iri → Prop → Prop
ChainWitness.IsVerifiedAs : core:iri → Prop → Prop
```

Each `JustifiedBy` grounding constructor consumes a witness; the composition constructors (`app`, `sum_l`, `sum_r`) are pure type-theoretic combinators.

The witnesses are **kernel-internal**: ESL has no constructors for them, and there is no introduction or elimination rule visible at the surface. The kernel admits an inhabitant *as a consequence* of a Trace-emitting commit succeeding (the resource passed its reflection-ontology validation; its trace was committed; the witness comes along for free). Proof irrelevance (D46) makes any two witnesses of the same `Prop`-typed predicate definitionally equal at that type, so the witness inhabitants need carry no internal content — they are pure proofs-of-existence.

The `VerifiedResource subclass_of DerivedResource` relation in the reflection ontology propagates as a witness coercion: `IsVerifiedAs iri P → IsDerivedAs iri P` is admitted automatically when the `IsVerifiedAs` witness is.

## 3. Where the witness table lives

**Decision: per-`Layer`, derived from the Layer's Trace resources.**

The witness table is *not* a separate persisted artifact. It is a deterministic function of the Layer's resources — specifically, of the subset of resources whose class is one of the four Trace classes plus the related grounding resources. The kernel materialises this as an in-memory index attached to `Layer` for fast lookup; the index is computed once at Layer-load time (or lazily on first witness lookup) and discarded when the Layer is dropped.

Why per-Layer rather than per-branch, per-chain, or kernel-global:

- **Layers are the unit of immutable commitment.** Witnesses are facts about what got committed; the natural home is the artifact that records the commitment. Branches and tags are mutable head pointers; they would be the wrong scope.
- **Witness lookup composes through the existing parent-chain walk.** Resource resolution already walks `Arc<Layer>` parent chains top-down; witness lookup uses the same walk, examining each Layer's index in turn. No new traversal machinery.
- **Content-addressing stays honest.** Witnesses are pure functions of the Layer's resources, so they are reproducible from the Layer alone. The Layer's content hash already covers them transitively (because it covers the Trace resources they are derived from); no separate hash is needed and no drift is possible.
- **Persistence comes for free.** The Trace resources persist via the existing storage backend; the witness index is recomputable on Layer load. Nothing new to store in RocksDB.

Conceptually the table *is* the Layer's Trace resources, projected through the witness key. The materialised index is purely an efficiency device — a `BTreeMap<WitnessKey, ()>` on the Layer, populated at construction time.

## 4. The witness key

```rust
pub struct WitnessKey {
    pub category: WitnessCategory,
    pub iri: Iri,
    pub prop_hash: [u8; 32],   // SHA-256 of the D47-encoded proposition
}

pub enum WitnessCategory {
    Declared,
    Observed,
    Derived,
    Verified,
}
```

Three fields:

- `category` — distinguishes the four predicate families. Required because the same IRI can carry multiple different witnesses (a `DerivedResource` could in principle also be a `VerifiedResource` via the `subclass_of` relation; the `IsVerifiedAs` witness must be distinguishable from the `IsDerivedAs` witness for elimination purposes).
- `iri` — the IRI of the resource being grounded. The trace's `resource` property (reflection ontology).
- `prop_hash` — SHA-256 of the D47-encoded proposition. The proposition itself can be arbitrarily large; hashing it gives a fixed-size lookup key and makes the table a `BTreeMap` rather than a more expensive structure. Hash collisions are negligible at SHA-256 strength.

Per D39 §3 Decision 3c, the (category, iri) pair determines exactly one canonical proposition for the resource at any given Layer, so the `prop_hash` is effectively redundant — but keeping it in the key catches the case where a `JustifiedBy.declared` constructor is type-checked with a proposition that doesn't match the resource's canonical proposition (a type error at type-check time rather than a silent acceptance of the wrong proposition).

The `subclass_of`-induced coercion `IsVerifiedAs → IsDerivedAs` is implemented at lookup time, not at table-population time: when looking up `(Derived, iri, hash)`, the kernel additionally checks for `(Verified, iri, hash)` and admits the witness via the coercion if found.

## 5. The synthesis algorithm

When the type checker encounters a `JustifiedBy.declared` (or `.observed`, `.derived`, `.verified`) constructor with arguments `iri : core:iri` and a target type `JustifiedBy (DeclaredEvidence iri) P`, the witness is *implicit* — the user does not write it, the kernel synthesises it. The algorithm:

```
synthesise_chain_witness(category, iri, P, ctx) → Result<WitnessVal, TypeError>:
    1. encoded_P = encode_type(P)             // D47 codec
    2. prop_hash = sha256(canonical_cbor(encoded_P))
    3. key = WitnessKey { category, iri, prop_hash }
    4. for layer in ctx.layer_chain top-down:
         if layer.witness_index.contains(&key):
             return Ok(opaque_witness(key))
         // Coercion: IsVerifiedAs subsumes IsDerivedAs.
         if category == Derived:
             verified_key = WitnessKey { category: Verified, iri, prop_hash }
             if layer.witness_index.contains(&verified_key):
                 return Ok(opaque_witness(verified_key))
    5. return Err(TypeError::NoAdmittedChainWitness {
         category, iri, prop_hash,
         hint: "the resource at <iri> must be committed with
                canonical_proposition = <P> (or with the default
                Asserts(<iri>)) in a Layer reachable from the
                current context before this JustifiedBy.<category>
                constructor is well-typed",
       })
```

Notes on the algorithm:

- **The walk is the existing parent-chain walk.** No new traversal abstraction; reuses `Arc<Layer>` parent pointers.
- **First hit wins**, matching how resource resolution works. Once a witness is admitted in a Layer, it is admitted in all descendant Layers transitively (immutability).
- **Negative results are type errors at type-check time**, not runtime failures. The user sees a precise diagnostic naming the missing witness, the expected canonical proposition, and how to make it admissible (commit the resource, or correct the proposition the `JustifiedBy` constructor claims).
- **`opaque_witness(key)`** produces a kernel `Val::Neut` representing the admitted witness. Two witnesses for the same key are definitionally equal by proof irrelevance (D46) — the kernel does not need to track per-witness identity beyond the key.
- **The algorithm is decidable and total** under the assumption that encoding is deterministic (D47 §3.7) and SHA-256 is collision-free at the relevant scale. It is also independent of the order in which Trace resources were committed — only the set of committed traces matters, not their interleaving.

## 6. Trace-emission relationship (no new D41 hooks)

Witnesses are derived from Trace resources by walking the Layer's resource set. The existing D41 commit pipeline already validates and persists every Resource, including Trace classes; the witness index is rebuilt as a side effect of Layer construction (or on first lookup, lazily). No new commit-pipeline hooks are required.

The trace-to-witness mapping is uniform across all four families — the witness emitter reads `canonical_proposition` from a chain resource in every case. The only variation is *which* resource carries the proposition:

| Trace class | Witness key `iri` | Source resource for `P` | Witness emitted |
|---|---|---|---|
| `reflection:DeclarationTrace` | `reflection:resource` (target of the declaration) | the declared resource itself; reads `reflection:canonical_proposition` (default `Asserts(iri)`) | `IsDeclaredAs iri P` |
| `reflection:ObservationTrace` | `reflection:resource` (target of the observation) | the observed resource itself; same default rule | `IsObservedAs iri P` |
| `reflection:ProgramTrace` | the `output` resource's IRI | the output resource itself; same default rule | `IsDerivedAs output_iri P` |
| `reflection:VerificationTrace` | the `reflection:resource` (the original `VerifiedResource`) | the `reasoning:VerifiedPropositionView` reified by the Lean → Reasoning comorphism (looked up by `source_verified_resource = trace.resource`); reads its `canonical_proposition`. See §7 for the comorphism. | `IsVerifiedAs iri P` |

In all four cases the witness emitter performs the same operation: locate the `canonical_proposition`-carrying chain resource, read the property, hash the encoded form, populate the witness index entry. The three non-verifier cases read directly from the trace's target; the verifier case reads from the comorphism-reified view at a content-hash-derived IRI distinct from the original `VerifiedResource`. The view's existence is a precondition for `IsVerifiedAs` admission — if the comorphism's reify failed (proposition outside the v1 translatable fragment), no view exists and no witness is admitted. This makes "non-exportable" a uniform observable: it is the absence of a chain resource, not a special trait return value.

**Per D39 §4.2, `ReasoningSentence` is `subclass_of reflection:DerivedResource`** and its `proposition` field serves as the resource's `canonical_proposition`. As a consequence, a `ReasoningSentence` commit emits an `IsDerivedAs sentence_iri proposition` witness via the `ProgramTrace` row above (the trace produced by the agent's reasoning step is a `ProgramTrace` whose `output` is the sentence itself, and the witness emitter reads `proposition` as the canonical proposition). This is what lets a later `ReasoningSentence`'s `JustificationTerm` cite the prior via `DerivedEvidence(prior_sentence_iri)` — the witness is admitted by the same per-Layer index, looked up by the same algorithm, with no Reasoning-institution-specific dispatch in the witness emitter.

The `reflection:canonical_proposition` property is a new optional addition to `DeclaredResource` / `ObservedResource` / `DerivedResource`, of `data_type: resource` carrying an `eigentt:TypeExpr` payload. Absent value defaults to the `Asserts(iri)` term (built by the witness-emission path; not stored on the resource). The validator type-checks the `canonical_proposition` at commit — this is **Rule 21** (`check_type_expr_well_typed`, `kernel/src/validation/rules/eigentt_value.rs`): it decodes the D47 payload (→ `TypeExprMalformed` on failure) then runs `nbe::check_infer` against the chain (→ `TypeExprIllTyped` on failure); a mis-typed proposition rejects the resource entirely (and therefore the Trace, and therefore the witness). Rule 21 is **not** canonical-specific — it keys off the declared range (`class_types ∋ eigentt:TypeExpr`), so *every* eigentt-valued slot (`objective:proposition`, `lexicon:prop`, …) is decoded + type-checked uniformly: the type system decides what is a checkable proposition, not a property name. It **consolidated** what were three overlapping checks — the earlier decode-only `canonical_proposition` check, `check_inductive_value`'s parallel `ConstRef`/`CtorApp` resolution walk for `eigentt:TypeExpr`, and the type-check itself — into one validator (decode + `check_infer`), with no duplicate diagnostics. (v1 asserts well-typedness, not strictly `: Prop`; tightening canonical slots to exactly `Prop` is an additive refinement — the load-bearing case, an ill-typed proposition, is already rejected.) For `ReasoningSentence` specifically (D39 §4.2), the validator reads `proposition` in place of `canonical_proposition` (since `ReasoningSentence` already requires the field by §4.2's invariants) — no duplicate storage required. For `VerifiedPropositionView` (§7 below), the property is *required* (not just optional) — the view's whole purpose is to surface the EigenTT-encoded proposition that the Lean → Reasoning comorphism produced, and a view without it is malformed.

Implementation site: `kernel/src/layer/witness_index.rs` (new). One function, `build_witness_index(&Layer) -> BTreeMap<WitnessKey, ()>`, called from `Layer::build_post_resources` (or wherever the Layer's post-commit derived state is computed). The function walks `layer.resources`, dispatches on Trace class, and populates the map.

Calling this lazily on first witness lookup (rather than eagerly at Layer construction) avoids paying the cost for Layers that no `JustifiedBy` certificate ever references. Use `OnceLock<BTreeMap<WitnessKey, ()>>` on the Layer.

## 7. `IsVerifiedAs` via the Lean → Reasoning comorphism

`IsVerifiedAs` is the one case where the proposition `P` is not declared on the resource — it is what the verifier's proof actually inhabits, and the verifier is the only party that knows the answer in its own type theory. An earlier draft of this memo introduced a new `Institution::export_proposition` trait method to surface this; that shape is over-engineered. The cross-institution translation is exactly what a D14 *comorphism* already does, and the witness emitter does not need a special path: it reads `canonical_proposition` from a chain-resident view that the comorphism reifies, identically to the other three witness families.

**Decision: comorphism-reify pattern, no new trait surface.** D39 §7 already declares a Reasoning ↔ Lean comorphism family; the Lean → Reasoning direction is what we need. The witness emitter for `VerificationTrace` (§6) is *uniform across all four witness families* — it always reads `canonical_proposition` from a chain resource. The Lean case is no longer special; it simply reads the property from a comorphism-reified `VerifiedPropositionView` resource rather than from a user-authored `VerifiedResource` directly.

The pattern:

1. **The user commits a `VerifiedResource`** in the verifier's native vocabulary. For Lean, this is a `LeanProofTerm` carrying verbatim `lean4export` bytes plus the chain-mirrored `lean:LeanExpr` proposition (D40). No parallel EigenTT proposition is required.
2. **The verifier's existing AutoOnLoad gate validates the proof.** For Lean, this is the existing D28 `nanoda_lib` re-check that already produces a `Verdict` resource on commit.
3. **The Lean → Reasoning comorphism dispatches as a second AutoOnLoad gate on the same commit** (D14 §9.3 supports stacked AutoOnLoad dispatch). Its *transform* step runs the inverse of the D30 forward translation on the proposition; its *reify* step commits a new derived resource at a content-hash-derived IRI per D14 §9.3 step 4, of class `reasoning:VerifiedPropositionView`, carrying `canonical_proposition` = the EigenTT-form proposition and `source_verified_resource` = the original VerifiedResource's IRI. On transform failure (proposition outside the v1 translatable fragment), the comorphism reify emits `Verdict::Fails` with a diagnostic naming the inexpressible feature, and no `VerifiedPropositionView` is reified.
4. **The witness emitter (§6) reads `canonical_proposition` from the `VerifiedPropositionView`** at witness-table build time, exactly the same code path as for the other three witness families. `IsVerifiedAs iri P` keys to `(Verified, source_verified_resource_iri, sha256(canonical_cbor(canonical_proposition)))`. The `source_verified_resource_iri` (not the view's own IRI) is the witness key's `iri` slot, so `JustifiedBy.verified` constructors over the user-authored `VerifiedResource`'s IRI lookup successfully.

The chain artifact and its dispatch in concrete shape:

```
class reasoning:VerifiedPropositionView {
    is_a reflection:DerivedResource;
    requires reasoning:source_verified_resource;     // IRI of the user-authored VerifiedResource
    requires reflection:canonical_proposition;       // D47-encoded EigenTT Prop term
    // Inherited derivation invariant satisfied by the comorphism's
    // reify trace (the comorphism's ProgramTrace).
}

// Comorphism declaration (D14 §3-§5 shape):
institution:Comorphism reasoning:lean_to_reasoning {
    source = institution:LeanInstitution;
    target = institution:ReasoningInstitution;
    export_format = lean:LeanProofTerm;              // source-side class
    transformation = reasoning:lean_to_eigentt_transform;  // the inverse-D30 lambda
    import_format = reasoning:VerifiedPropositionView;     // target-side class
    exact = false;   // not faithful for the full Lean fragment (universe polymorphism, etc.)
    dispatch_role = AutoOnLoad;
    fires_on = lean:LeanProofTerm;   // any committed proof triggers it
}
```

Why this shape:

- **No new D14 trait surface.** D14's three-method `Institution` trait (`extract_typed` / `reify` / `query`) stays unchanged. The comorphism mechanism that already handles cross-institution dispatch absorbs the requirement.
- **The witness emitter is uniform across all four families.** §6's algorithm reads `canonical_proposition` from a chain resource in every case. The Lean case differs only in *how the resource came into existence* (via comorphism reify rather than user commit) — invisible to the emitter.
- **The translation is the comorphism's transformation step.** The inverse-D30 logic lives where it belongs — inside the Lean institution's comorphism declaration, behind the existing D14 transformation surface. Symmetric with D30's forward direction, which is implemented as the source-side export of the Reasoning → Lean comorphism.
- **Single source of truth for the proposition** (in the user's authoring step). The user commits the Lean-native form; the EigenTT form is derived deterministically via the comorphism. No two-version-and-check pattern; no user-facing consistency check.
- **Graceful unexportability surfaces through existing diagnostic shapes.** Lean propositions outside the v1 fragment (universe polymorphism, Lean-specific definitional unfolding) cause the comorphism's reify to fail with a `Verdict::Fails` carrying the diagnostic. The `VerifiedResource` stays valid as a Lean-native artifact (and remains citable from other Lean-aware contexts) but no `VerifiedPropositionView` is reified — `JustifiedBy.verified` constructors over it fail to type-check at the next witness lookup, with the chain-resident Verdict resource as a discoverable explanation. v1 ships with the trivially-mappable fragment; broader coverage is a future comorphism-transformation refinement, transparent to the Reasoning institution.
- **The chain artifact for "this Lean proof verifies this EigenTT proposition" is first-class** — content-addressed, queryable via plain EigenQL, traceable through the standard reflection-ontology provenance. Auditors can `MATCH VerifiedPropositionView(?v) { source_verified_resource: <X> } RETURN canonical_proposition` directly.
- **Generalises to future verifiers.** Adding Coq, Agda, or Idris is a matter of declaring `coq_to_reasoning` / `agda_to_reasoning` / … comorphisms, each with its own transformation implementing the relevant inverse translation. The Reasoning institution and the witness machinery are untouched.

Implementation sites:
- `ontologies/reasoning/reasoning-ontology.json` — adds the `VerifiedPropositionView` class declaration with the two required properties.
- `ontologies/lean/lean-ontology.json` (or wherever the Lean comorphisms are declared) — adds the `lean_to_reasoning` comorphism declaration.
- `crates/eigenius-lean-worker/src/lean_to_reasoning.rs` (new) — the comorphism transformation implementation. Reads the chain-mirrored `lean:LeanExpr` proposition, calls `nanoda_lib`'s type accessor on the proof to confirm it inhabits the proposition (already done by the existing AutoOnLoad), runs the inverse-D30 translation, returns an EigenTT `Exp`. The comorphism's reify wrapping (committing the view resource) uses the existing D14 §9.3 step 4 chain-reinsertion path — no new infrastructure.
- `kernel/src/layer/witness_index.rs` — the `VerificationTrace` branch of the witness emitter reads `canonical_proposition` from the *reified* `VerifiedPropositionView` (looked up by `source_verified_resource = trace.resource`) rather than from the user-authored VerifiedResource. Same code path as the other three families once the view exists.

The witness emission is layer-deterministic because the comorphism reify is deterministic for a given proof term and a given Lean institution version. If the Lean institution's translation surface changes between versions, the affected `VerifiedPropositionView` resources at older content hashes remain valid — re-verification produces new views at new content hashes, and the old IRI references still resolve to the old propositions. This matches how D24 schema-migration semantics handle versioned chain artifacts.

## 8. The opaque witness value

When the synthesis algorithm returns, the kernel needs a concrete `Val` that represents the admitted witness. The options:

- **A new `Val::ChainWitness { key: WitnessKey }` variant.** Honest about what it represents; carries the lookup key so debug output can name the admitted fact.
- **Reuse `Val::Neut` with a designated marker.** Less invasive but conflates witnesses with general neutral terms.

**Decision: a new `Val::ChainWitness` variant.** It clarifies the value's provenance for both kernel-internal code and debug introspection. The variant has no eliminator (witnesses are opaque); definitional equality treats two `ChainWitness` values as equal iff their keys match (proof irrelevance reduces this further: two witnesses of the same `Prop`-typed type are equal regardless of key, but the per-key equality is useful for the rare cases where the kernel inspects two witness values outside the proof-irrelevance fast path).

`Exp` does not need a corresponding constructor — `ChainWitness` values are never readback into surface syntax because they are never authored by the user. Type-level uses of the predicate (`ChainWitness.IsDeclaredAs iri P`) appear in `JustifiedBy` constructor signatures as ordinary inductive-type references, but the *inhabitants* of those types appear only as kernel-internal `Val::ChainWitness` values.

## 9. Open questions

The implementation will likely surface refinements in these areas; calling them out so they don't get reinvented under time pressure.

**Witness garbage collection.** When a Layer is GC'd (D24), its witness index is discarded along with the Layer itself. Witnesses admitted by ancestors persist as long as the ancestors persist. This matches the GC model exactly — no new policy required. Worth verifying explicitly that no `JustifiedBy` certificate can be type-checked against a witness whose underlying Trace resource has been GC-collected (it cannot, because the Layer chain walk only touches reachable Layers).

**Layer-merge witness composition.** When two branches are merged (D20/D36/D38), the resulting Layer's witness index is the union of both parents' indices — composing through the existing parent-chain walk handles this automatically. No new merge logic. Worth verifying with a merge-resolution test that witnesses from both parents are admissible in the merged branch.

**EntailmentQuery efficiency.** D39 §4.3's `EntailmentQuery` asks "does some `JustificationTerm` over a set of committed sentences justify proposition `A`?" — implemented by elaborating a `JustifiedBy` term against the query proposition. Witness lookup happens for each elaboration step; for deeply structured proofs this becomes the dominant cost. The per-Layer materialised index plus the OnceLock makes individual lookups O(log n); the total cost scales with the proof's depth. No special optimisation expected to be needed in v1; revisit if `EntailmentQuery` becomes a hot path.

**Multiple Layers admitting the same witness.** The algorithm returns the first hit walking top-down; ties are resolved by the most-recent admitting Layer. This matters for content-addressing (the witness inhabitant carries the discovering Layer's identity in debug output) but not for soundness (the witness is the same fact regardless of which Layer surfaced it first).

**Intra-load emitter/consumer ordering (eigenius#85).** §6 builds the witness index lazily by walking `layer.resources`, but commit is sequential within a load unit: a consumer's commit-time justification gate (D54 `qc_validate_justification`) fires before a later-in-file emitter has committed its witness-bearing Trace into the layer. A single file mixing emitters and consumers therefore fails on first load and succeeds on retry (the emitters' witnesses persist to an ancestor layer on the partial first pass). This is an ordering artifact, not a missing warrant. Current workaround: author emitters and consumers in separate, ordered files. Proper fix is a two-phase AutoOnLoad (index all derivations across the load unit, then gate consumers) — tracked in eigenius#85, which also flags the partial-apply-on-failure behaviour as a related D41 smell.

## 10. Relationship to other documents

- **[D39 v2 justification logic](d39-justification-logic.md)** — D49 is the implementation memo for D39 §5's `ChainWitness` predicate family. D39 specifies the predicate shape, indexing decisions, and JustifiedBy constructor signatures; D49 specifies where the table lives, how witnesses are synthesised, and how the Lean checker integrates.
- **[D41 commit pipeline](d41-commit-pipeline.md)** — D49 deliberately uses no new D41 hooks. Witnesses are derived from Trace resources that the existing pipeline validates and persists; the per-Layer witness index is a post-Layer-construction derived structure, not a pipeline event.
- **[D46 Prop + proof irrelevance](d46-prop-universe-and-proof-irrelevance.md)** — D46's proof irrelevance is what makes opaque witness values sound: two `IsDeclaredAs iri P` witnesses are definitionally equal at the `Prop`-typed predicate without any internal-structure comparison.
- **[D47 type-fragment codec](d47-chain-mirrored-eigentt-type-fragment.md)** — D49 uses `encode_type` to canonicalise propositions for hashing in the witness key. The proposition's content hash *is* the codec output's SHA-256.
- **[D48 indexed inductive families](d48-indexed-inductive-families.md)** — `ChainWitness.IsXxAs` is an indexed inductive (indexed by both IRI and proposition) authored using D48's machinery; eigenius#72 Layer 2 is the surface for declaring it.
- **[D14 institution realisation](d14-institution-realisation.md)** — the Reasoning institution that consumes `ChainWitness` witnesses is a normal D14 institution, not a kernel built-in. Its `extract_typed` / `reify` / `query` methods follow the standard trait. **No new D14 trait surface is required** for `IsVerifiedAs` witness emission — the Lean → Reasoning comorphism pattern (§7) uses D14's existing comorphism machinery and chain-reinsertion path (D14 §9.3 step 4) without modification. An earlier draft of this memo proposed a new `Institution::export_proposition` trait method; that shape was over-engineered and is dropped in favour of the comorphism pattern.
- **[D28 Lean 4 as institution](d28-lean-4-as-institution.md), [D30 Eigon → Lean faithful translation](d30-eigon-to-lean-faithful-translation.md), [D40 chain-mirrored Lean expressions](d40-chain-mirrored-lean-expressions.md)** — together provide the Lean side of §7's comorphism. D28 is the verifier that validates the proof via `nanoda_lib` at the user's `VerifiedResource` commit; D40 is the chain-mirrored Lean proposition format the user commits; D30 is the EigenTT → Lean forward translation whose inverse the new Lean → Reasoning comorphism's transformation step implements. The translation pair is symmetric — D30 forward for inputs (the source side of the Reasoning → Lean comorphism), the new inverse for outputs (the source side of the Lean → Reasoning comorphism). Both directions live in the Lean institution's comorphism declarations.
- **reflection ontology** — the four Trace classes (`DeclarationTrace`, `ObservationTrace`, `ProgramTrace`, `VerificationTrace`) are the chain substrate witnesses are derived from. The one extension D49 requires is the optional `reflection:canonical_proposition` property on `DeclaredResource` / `ObservedResource` / `DerivedResource`; this is the single new chain-vocabulary item.

---

*Implementation order (informative). The witness machinery is the soundness boundary, so build it first: §3-5 (witness key + table + synthesis) without any Trace-class integration, smoke-tested against a hand-built Layer carrying mock Trace resources. Then §6 (the four-Trace dispatch — all four families read `canonical_proposition` uniformly; the Lean case reads from a `VerifiedPropositionView` that does not yet exist, so the lookup correctly returns "no witness" until §7 lands). Then §7: declare the `VerifiedPropositionView` class in `ontologies/reasoning/`, declare the Lean → Reasoning comorphism in the Lean institution's ontology, implement the comorphism's transformation as the inverse of D30 for the trivially-mappable Prop fragment. Then layer onto the rest of D39: the `JustifiedBy` inductive, `ReasoningSentence` Resource class, and the Reasoning institution's own D14 dispatch (`extract_typed` / `reify` / `ValidateJustification` / `EntailmentQuery` / `ConsistencyCheck`). The first two sections (§3-6) are roughly two weeks of kernel work; the comorphism (§7) is a third week largely spent on the Lean → EigenTT translation; the D39 institutional dispatch on top is a fourth week of mostly D14-shaped boilerplate.*
