// Copyright 2026 The Eigenius Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! D49 §6 per-`Layer` witness index.
//!
//! Each `Layer` carries a materialised `BTreeMap<WitnessKey, ()>` projecting
//! its Trace-class resources into admitted `ChainWitness` keys. The index is
//! a pure deterministic function of the Layer's resources — recomputable on
//! load, content-addressed transitively via the Layer's own content hash.
//!
//! Lookup is the parent-chain walk: `lookup_chain_witness(&Layer, &key)`
//! tries each Layer top-down, returning true on first hit. First-hit-wins
//! is sound because Layer immutability means a once-admitted witness stays
//! admitted in all descendants.
//!
//! This module hosts the index-build function. The `OnceLock` field on
//! `Layer` and the lookup walker live in `kernel/src/layer/mod.rs`.

use crate::layer::Layer;
use crate::ontology::resource::Resource;
use crate::ontology::well_known as wk;
use crate::ontology::{Iri, Value};
use crate::witness::{hash_proposition_value, WitnessCategory, WitnessKey};
use std::collections::BTreeMap;

/// D54: the `reasoning:ReasoningSentence` class IRI and its `proposition`
/// property. Named here (rather than in `well_known`) because the D49
/// witness machinery is the one kernel site that is intrinsically
/// reasoning-aware — it builds the witnesses `JustifiedBy` consumes.
const REASONING_SENTENCE: &str = "urn:eigenius:reasoning:ReasoningSentence";
const REASONING_PROPOSITION: &str = "urn:eigenius:reasoning:proposition";

/// Build the per-`Layer` witness index by walking the Layer's local
/// resources and dispatching each Trace-class resource to the
/// corresponding witness emission.
///
/// **Per D49 §6**: three of the four Trace classes are static reads of a
/// resource's `canonical_proposition`. The fourth (`VerificationTrace`)
/// is admitted via a comorphism-reified `VerifiedPropositionView`
/// (D49 §7) and is handled the same way once that view exists. The
/// implementation here covers all four uniformly: the witness emitter
/// reads `canonical_proposition` from the trace's target resource (or,
/// for `VerificationTrace`, from the `VerifiedPropositionView` resource
/// keyed by `source_verified_resource`).
///
/// **What this Phase-4 implementation handles**:
/// - `DeclarationTrace` → `IsDeclaredAs target_iri P`
/// - `ObservationTrace` → `IsObservedAs target_iri P`
/// - `ProgramTrace` → `IsDerivedAs output_iri P`
///
/// `VerificationTrace` dispatch is deferred to the Phase-7 / D49 §7
/// integration, which depends on the `reasoning:VerifiedPropositionView`
/// class (produced by the Lean → Reasoning comorphism). When that lands,
/// the dispatch becomes a fourth match arm here.
///
/// **Default proposition**: per D49 §6 / D39 §4.1, an absent
/// `reflection:canonical_proposition` on the target resource defaults to
/// the EigenTT term `Asserts(iri)`. Authoring the `Asserts` inductive +
/// the witness-emission default is Phase 5; the Phase-4 implementation
/// emits a witness only when `canonical_proposition` is present on the
/// target. This is sufficient to exercise the witness machinery against
/// hand-built test fixtures; the `Asserts(iri)` default lands in Phase 5
/// without changing this module's signature.
pub fn build_witness_index(layer: &Layer) -> BTreeMap<WitnessKey, ()> {
    let mut index: BTreeMap<WitnessKey, ()> = BTreeMap::new();
    for (_iri, resource) in layer.iter_resources() {
        let is_a = resource.is_a();
        for cls in &is_a {
            let cls_str = cls.as_str();
            let category = if cls_str == wk::DECLARATION_TRACE {
                WitnessCategory::Declared
            } else if cls_str == wk::OBSERVATION_TRACE {
                WitnessCategory::Observed
            } else if cls_str == wk::PROGRAM_TRACE {
                WitnessCategory::Derived
            } else {
                continue;
            };
            // The trace's target IRI is in `reflection:resource` for
            // Declaration / Observation traces, and (per D49 §6) for
            // ProgramTrace's output we'd traditionally read the
            // `output` property — but since both surface here as the
            // same reflection:resource carrier, treat them uniformly.
            if let Some(key) = emit_from_trace(layer, &resource, category) {
                index.insert(key, ());
            }
        }
        // D52 institution-emitted-derivation shape: AutoOnLoad-emitted
        // derivations (`reflection:InstitutionEmittedDerivation`) are
        // self-attesting — the kernel produced them deterministically
        // from a decidable institution running against the gated
        // subject, no separate ProgramTrace is needed to certify their
        // existence. Walk these directly and admit
        // `IsDerivedAs(derivation_iri, P)` against the derivation's
        // own IRI. The verdict resource itself doesn't carry a
        // canonical_proposition under the new shape — only derivations
        // do (D52 verdict-vs-derivation split).
        if is_a
            .iter()
            .any(|c| c.as_str() == wk::INSTITUTION_EMITTED_DERIVATION)
        {
            if let Some(key) = emit_from_institution_derivation(&resource) {
                index.insert(key, ());
            }
        }
        // D54 reasoning-sentence lemma citation: a committed
        // `reasoning:ReasoningSentence` is a kernel-checked proof of its
        // `proposition` — the `ValidateJustification` gate Held, and the
        // commit pipeline rejects `Fails` sentences, so any *committed*
        // sentence Held (the same trust-committed model that lets us admit
        // institution derivations without re-running them). Admit it as a
        // `Verified` witness keyed on its own IRI, so a later sentence can
        // cite it as a lemma via `JustifiedBy.verified` (or `.derived`, via
        // the `IsVerifiedAs → IsDerivedAs` coercion in `lookup_chain_witness`).
        if is_a.iter().any(|c| c.as_str() == REASONING_SENTENCE) {
            if let Some(key) = emit_from_reasoning_sentence(&resource) {
                index.insert(key, ());
            }
        }
    }
    index
}

/// D54: read a `reasoning:ReasoningSentence`'s `proposition` and build a
/// `Verified` `WitnessKey` keyed on the sentence's own IRI. The proposition
/// is the D47-encoded `Value::Json` the consumer's `JustifiedBy.verified(iri, P)`
/// term hashes to identically (same encoding path), so the key matches.
/// Returns `None` when the sentence has no `@id` or no `proposition`.
fn emit_from_reasoning_sentence(sentence: &Resource) -> Option<WitnessKey> {
    let sentence_iri = sentence.id().cloned()?;
    let prop_iri = Iri::parse(REASONING_PROPOSITION).ok()?;
    let encoded_prop = sentence.get(&prop_iri)?;
    let prop_hash = hash_proposition_value(encoded_prop);
    Some(WitnessKey {
        category: WitnessCategory::Verified,
        iri: sentence_iri,
        prop_hash,
    })
}

/// D52 institution-emitted derivation: read `canonical_proposition`
/// directly off a kernel-emitted derivation resource and build a
/// `WitnessKey` keyed against the derivation's own IRI. Returns `None`
/// when the derivation has no `canonical_proposition` set (kernel
/// merge dropped it, or the institution didn't supply one).
fn emit_from_institution_derivation(derivation: &Resource) -> Option<WitnessKey> {
    let derivation_iri = derivation.id().cloned()?;
    let prop_iri = Iri::parse(wk::CANONICAL_PROPOSITION).ok()?;
    let encoded_prop = derivation.get(&prop_iri)?;
    let prop_hash = hash_proposition_value(encoded_prop);
    Some(WitnessKey {
        category: WitnessCategory::Derived,
        iri: derivation_iri,
        prop_hash,
    })
}

/// Read a Trace resource's target IRI and the target's
/// `canonical_proposition`; build a `WitnessKey`. When
/// `canonical_proposition` is absent on the target resource, fall back
/// to the D39 §4.1 default proposition `Asserts(target_iri)` — built
/// via [`default_asserts_proposition`]. The fallback path requires the
/// chain to provide `core:Asserts` (it does once core ontology has
/// loaded); pre-bootstrap chains where `core:Asserts` doesn't resolve
/// fail silently (returning `None`) so witness-index construction
/// can't deadlock the bootstrap path.
fn emit_from_trace(
    layer: &Layer,
    trace: &Resource,
    category: WitnessCategory,
) -> Option<WitnessKey> {
    let target_iri = resolve_target_iri(trace)?;
    let target_resource = layer.resolve(&target_iri)?;
    let prop_iri = Iri::parse(wk::CANONICAL_PROPOSITION).ok()?;
    let prop_hash = match target_resource.get(&prop_iri) {
        Some(encoded_prop) => hash_proposition_value(encoded_prop),
        None => default_asserts_proposition_hash(layer, &target_iri)?,
    };
    Some(WitnessKey {
        category,
        iri: target_iri,
        prop_hash,
    })
}

/// Build the default proposition `Asserts(target_iri)` per D39 §4.1
/// and return its hash. Resolves `core:Asserts` from the layer chain,
/// constructs `Exp::InductiveType(asserts_decl, [Exp::LitString(target_iri)])`,
/// encodes via the D47 codec, and hashes.
///
/// **Both ends of the witness machinery use the same construction.**
/// When a future `JustifiedBy.declared(iri, Asserts(iri))` constructor
/// is type-checked, the consumer side (D49 §5 / `synthesize_chain_witness`)
/// receives the same `Exp` from the user's proof term, encodes it via
/// the same `encode_type` path, and arrives at the same hash. The
/// hash-matching is the soundness guarantee; the explicit shared
/// helper is the maintainability guarantee.
///
/// Returns `None` if `core:Asserts` isn't resolvable in the chain
/// (typically: pre-bootstrap construction, or a malformed chain).
/// Callers treat absence as "no witness emitted" — same outer behaviour
/// as the missing-`canonical_proposition` no-Asserts case.
pub fn default_asserts_proposition_hash(layer: &Layer, target_iri: &Iri) -> Option<[u8; 32]> {
    let asserts_iri = Iri::parse(wk::ASSERTS).ok()?;
    let asserts_resource = layer.resolve(&asserts_iri)?;
    let val =
        crate::program::ground::resolve_inductive_type(&asserts_iri, &asserts_resource, layer)
            .ok()?;
    let decl = match val {
        crate::nbe::val::Val::InductiveType { decl, .. } => decl,
        _ => return None,
    };
    let proposition = crate::nbe::term::Exp::InductiveType(
        decl,
        vec![crate::nbe::term::Exp::LitString(
            target_iri.as_str().to_string(),
        )],
    );
    crate::witness::hash_proposition_exp(&proposition).ok()
}

/// Public synthesis variant of [`default_asserts_proposition_hash`]
/// that returns the full `Exp` rather than the hash. Used by the
/// `synthesize_chain_witness` consumer site when the agent's
/// `JustifiedBy.declared` constructor doesn't carry an explicit
/// proposition (i.e. the consumer wants the default to compare
/// against). Same `Asserts(iri)` shape; same Exp; same hash.
pub fn default_asserts_proposition(
    layer: &Layer,
    target_iri: &Iri,
) -> Option<crate::nbe::term::Exp> {
    let asserts_iri = Iri::parse(wk::ASSERTS).ok()?;
    let asserts_resource = layer.resolve(&asserts_iri)?;
    let val =
        crate::program::ground::resolve_inductive_type(&asserts_iri, &asserts_resource, layer)
            .ok()?;
    let decl = match val {
        crate::nbe::val::Val::InductiveType { decl, .. } => decl,
        _ => return None,
    };
    Some(crate::nbe::term::Exp::InductiveType(
        decl,
        vec![crate::nbe::term::Exp::LitString(
            target_iri.as_str().to_string(),
        )],
    ))
}

/// Read the `reflection:resource` property from a Trace resource and
/// parse it as an `Iri`. Returns `None` if the property is missing or
/// malformed.
fn resolve_target_iri(trace: &Resource) -> Option<Iri> {
    let resource_iri = Iri::parse(wk::REFLECTION_RESOURCE).ok()?;
    let value = trace.get(&resource_iri)?;
    match value {
        Value::ResourceRef(iri) => Some(iri.clone()),
        Value::String(s) => Iri::parse(s).ok(),
        _ => None,
    }
}

/// Walk the parent chain top-down, returning true on the first Layer
/// whose witness index contains `key`. Implements the §5 synthesis
/// algorithm's lookup step. The `IsVerifiedAs → IsDerivedAs` coercion
/// (D49 §4) is handled at this layer: a `Derived`-category lookup also
/// succeeds when a corresponding `Verified` entry exists at the same
/// `(iri, prop_hash)`.
pub fn lookup_chain_witness(layer: &Layer, key: &WitnessKey) -> bool {
    if check_layer_with_coercion(layer, key) {
        return true;
    }
    let mut cursor = layer.parent().cloned();
    while let Some(parent) = cursor {
        if check_layer_with_coercion(&parent, key) {
            return true;
        }
        cursor = parent.parent().cloned();
    }
    false
}

fn check_layer_with_coercion(layer: &Layer, key: &WitnessKey) -> bool {
    let index = layer.chain_witness_index();
    if index.contains_key(key) {
        return true;
    }
    if key.category == WitnessCategory::Derived {
        let verified_key = WitnessKey {
            category: WitnessCategory::Verified,
            iri: key.iri.clone(),
            prop_hash: key.prop_hash,
        };
        if index.contains_key(&verified_key) {
            return true;
        }
    }
    false
}

/// **D49 §5 synthesis algorithm — Phase 6 foundation.** Look up a
/// `ChainWitness` inhabitant for `(category, iri, proposition)` and, on
/// hit, return a `Val::ChainWitness(key)` value the kernel's NbE checker
/// can use as the synthesised witness argument to a `JustifiedBy.*`
/// constructor. On miss, surface the precise diagnostic D49 §5
/// specifies — naming the missing predicate family, the IRI, and what
/// the chain needs to admit for this `JustifiedBy.*` constructor to
/// become well-typed.
///
/// This function is the kernel-side surface the D39 Reasoning
/// institution's `JustifiedBy` constructor type-checker calls into. The
/// integration site — where `check_infer` in `nbe/check.rs` recognises a
/// `JustifiedBy.declared` / `.observed` / `.derived` / `.verified`
/// constructor and dispatches here — lands during D39 implementation
/// (per D51 gap 3); this function is the stable contract that integration
/// can call against starting today.
///
/// `proposition` is the EigenTT `Exp` extracted from the constructor's
/// `P` argument at the call site. The key's `prop_hash` is computed via
/// D47 encoding + SHA-256 to match what `build_witness_index` produced.
///
/// Crate-internal `crate::witness::Val::ChainWitness` is returned wrapped
/// in `Ok`; callers can pass it directly to where the constructor
/// expects a `ChainWitness.IsXxAs iri P` inhabitant.
pub fn synthesize_chain_witness(
    layer: &Layer,
    category: WitnessCategory,
    iri: &Iri,
    proposition: &crate::nbe::term::Exp,
) -> Result<crate::nbe::val::Val, String> {
    let key = WitnessKey::from_exp(category, iri.clone(), proposition).map_err(|e| {
        format!(
            "synthesize_chain_witness: failed to encode proposition for {} witness on {}: {e}",
            category.label(),
            iri,
        )
    })?;
    if lookup_chain_witness(layer, &key) {
        Ok(crate::nbe::val::Val::ChainWitness(key))
    } else {
        Err(format!(
            "no admitted {} witness for IRI {} with the supplied proposition; \
             the resource at {} must be committed with reflection:canonical_proposition \
             matching the proposition (or the proposition must be Asserts(<iri>) — the \
             default; the Asserts default lands in Phase 5b once D39's core-ontology \
             Asserts class is authored) before this JustifiedBy.{} constructor is well-typed",
            category.label(),
            iri,
            iri,
            match category {
                WitnessCategory::Declared => "declared",
                WitnessCategory::Observed => "observed",
                WitnessCategory::Derived => "derived",
                WitnessCategory::Verified => "verified",
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::{storage::LayerStorage, LayerBuilder};
    use crate::nbe::term::Exp;
    use crate::ontology::resource::Resource;
    use crate::ontology::{Iri, Value};
    use crate::program::eigentt_type_mirror::encode_type;
    use std::sync::Arc;

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn target_resource_with_canonical_prop(target_iri: &str, prop: &Exp) -> Resource {
        let mut r = Resource::new(iri(target_iri));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(wk::DECLARED_RESOURCE.to_string())]),
        );
        let encoded = encode_type(prop).unwrap();
        r.set(iri(wk::CANONICAL_PROPOSITION), encoded);
        r
    }

    fn declaration_trace(target_iri: &str, trace_iri: &str) -> Resource {
        let mut r = Resource::new(iri(trace_iri));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(wk::DECLARATION_TRACE.to_string())]),
        );
        r.set(
            iri(wk::REFLECTION_RESOURCE),
            Value::ResourceRef(iri(target_iri)),
        );
        r
    }

    #[test]
    fn build_witness_index_emits_declared_for_declaration_trace() {
        let mut b = LayerBuilder::new("test", None);
        let target = "urn:eigenius:example:thing";
        let prop = Exp::Sort(0);
        b.add_resource(target_resource_with_canonical_prop(target, &prop))
            .unwrap();
        b.add_resource(declaration_trace(
            target,
            "urn:eigenius:example:thing-decl-trace",
        ))
        .unwrap();
        let layer = b.build(LayerStorage::in_memory());
        let index = layer.chain_witness_index();
        let expected = WitnessKey::from_exp(WitnessCategory::Declared, iri(target), &prop).unwrap();
        assert!(
            index.contains_key(&expected),
            "expected IsDeclaredAs witness for target; got keys {:?}",
            index.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn build_witness_index_no_emission_when_canonical_prop_missing() {
        // Phase-4 behaviour: no Asserts(iri) default yet (deferred to
        // Phase 5). When the target lacks `canonical_proposition`, the
        // witness emitter skips emission.
        let mut b = LayerBuilder::new("test", None);
        let target = "urn:eigenius:example:bare";
        let mut bare = Resource::new(iri(target));
        bare.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(wk::DECLARED_RESOURCE.to_string())]),
        );
        b.add_resource(bare).unwrap();
        b.add_resource(declaration_trace(
            target,
            "urn:eigenius:example:bare-decl-trace",
        ))
        .unwrap();
        let layer = b.build(LayerStorage::in_memory());
        let index = layer.chain_witness_index();
        assert!(
            index.is_empty(),
            "Phase 4 emits nothing when canonical_proposition is absent (got {:?})",
            index.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn lookup_chain_witness_walks_parent_chain() {
        // Layer A defines the trace + target with canonical_prop.
        // Layer B (child of A) defines nothing. Lookup against B for
        // the witness key admitted by A succeeds (parent-chain walk).
        let mut a = LayerBuilder::new("parent", None);
        let target = "urn:eigenius:example:thing";
        let prop = Exp::Sort(0);
        a.add_resource(target_resource_with_canonical_prop(target, &prop))
            .unwrap();
        a.add_resource(declaration_trace(
            target,
            "urn:eigenius:example:thing-decl-trace",
        ))
        .unwrap();
        let layer_a = Arc::new(a.build(LayerStorage::in_memory()));

        let b = LayerBuilder::new("child", Some(layer_a.clone()));
        let layer_b = b.build(LayerStorage::in_memory());

        let key = WitnessKey::from_exp(WitnessCategory::Declared, iri(target), &prop).unwrap();
        assert!(
            lookup_chain_witness(&layer_b, &key),
            "lookup must walk parent chain and find the witness in layer A"
        );

        // Lookup of a witness that doesn't exist anywhere correctly misses.
        let other_prop = Exp::Sort(1);
        let other_key =
            WitnessKey::from_exp(WitnessCategory::Declared, iri(target), &other_prop).unwrap();
        assert!(
            !lookup_chain_witness(&layer_b, &other_key),
            "lookup must miss when the (iri, prop) pair was never admitted"
        );
    }

    // --- D39 Phase 2 — Asserts(iri) default when canonical_proposition is absent ---

    fn layer_with_core_ontology() -> Arc<crate::layer::Layer> {
        // Load the real core ontology so `core:Asserts` resolves.
        use crate::ontology::eigon_json;
        let core_json = include_str!("../../../ontologies/core/core-ontology.json");
        let core_resources = eigon_json::parse_document(core_json).unwrap();
        let mut core_builder = LayerBuilder::new("core", None);
        for r in core_resources {
            core_builder.add_resource(r).unwrap();
        }
        Arc::new(core_builder.build(LayerStorage::in_memory()))
    }

    #[test]
    fn default_asserts_proposition_hash_resolves_when_core_loaded() {
        let core_layer = layer_with_core_ontology();
        let target = iri("urn:eigenius:example:thing");
        let hash = default_asserts_proposition_hash(&core_layer, &target)
            .expect("Asserts default must resolve once core ontology is loaded");
        // Two calls with the same target produce the same hash.
        let hash2 = default_asserts_proposition_hash(&core_layer, &target).unwrap();
        assert_eq!(hash, hash2, "hash must be deterministic");
        // Different target → different hash.
        let other_target = iri("urn:eigenius:example:thing-2");
        let other_hash = default_asserts_proposition_hash(&core_layer, &other_target).unwrap();
        assert_ne!(hash, other_hash, "different iris hash to different keys");
    }

    #[test]
    fn build_witness_index_emits_asserts_default_when_canonical_prop_missing() {
        // With core ontology loaded, a DeclarationTrace pointing at a
        // target that lacks canonical_proposition still emits a witness
        // — the witness key uses Asserts(target_iri) as the proposition.
        let core_layer = layer_with_core_ontology();
        let target = "urn:eigenius:example:bare";

        // Build a user layer with the target (no canonical_proposition)
        // and a DeclarationTrace for it.
        let mut user = LayerBuilder::new("user", Some(core_layer.clone()));
        let mut bare = Resource::new(iri(target));
        bare.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(wk::DECLARED_RESOURCE.to_string())]),
        );
        user.add_resource(bare).unwrap();
        user.add_resource(declaration_trace(
            target,
            "urn:eigenius:example:bare-decl-trace",
        ))
        .unwrap();
        let user_layer = user.build(LayerStorage::in_memory());

        let index = user_layer.chain_witness_index();
        // Witness should now exist with the Asserts(target) default proposition.
        let expected_hash = default_asserts_proposition_hash(&core_layer, &iri(target))
            .expect("Asserts default must resolve");
        let expected = WitnessKey {
            category: WitnessCategory::Declared,
            iri: iri(target),
            prop_hash: expected_hash,
        };
        assert!(
            index.contains_key(&expected),
            "default Asserts witness must be emitted when canonical_proposition is absent; \
             got keys {:?}",
            index.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn explicit_canonical_proposition_overrides_asserts_default() {
        // When canonical_proposition IS present, the witness emitter
        // uses it instead of the Asserts default. The resulting hash
        // differs from what the default would produce.
        let core_layer = layer_with_core_ontology();
        let target = "urn:eigenius:example:explicit";
        let explicit_prop = Exp::Sort(0); // Prop sort

        let mut user = LayerBuilder::new("user", Some(core_layer.clone()));
        user.add_resource(target_resource_with_canonical_prop(target, &explicit_prop))
            .unwrap();
        user.add_resource(declaration_trace(
            target,
            "urn:eigenius:example:explicit-decl-trace",
        ))
        .unwrap();
        let user_layer = user.build(LayerStorage::in_memory());

        let index = user_layer.chain_witness_index();
        let explicit_key =
            WitnessKey::from_exp(WitnessCategory::Declared, iri(target), &explicit_prop).unwrap();
        assert!(
            index.contains_key(&explicit_key),
            "explicit canonical_proposition witness must be in index"
        );
        // The Asserts default key must NOT be in the index — the
        // emitter picked the explicit proposition.
        let default_hash = default_asserts_proposition_hash(&core_layer, &iri(target)).unwrap();
        let default_key = WitnessKey {
            category: WitnessCategory::Declared,
            iri: iri(target),
            prop_hash: default_hash,
        };
        assert_ne!(
            explicit_key, default_key,
            "explicit Prop must hash differently from default Asserts(iri)"
        );
        assert!(
            !index.contains_key(&default_key),
            "default Asserts witness must NOT appear when explicit canonical_proposition is set"
        );
    }

    // --- Phase 6 foundation — synthesize_chain_witness ---

    #[test]
    fn synthesize_chain_witness_succeeds_when_admitted() {
        let mut b = LayerBuilder::new("test", None);
        let target = "urn:eigenius:example:thing";
        let prop = Exp::Sort(0);
        b.add_resource(target_resource_with_canonical_prop(target, &prop))
            .unwrap();
        b.add_resource(declaration_trace(
            target,
            "urn:eigenius:example:thing-decl-trace",
        ))
        .unwrap();
        let layer = b.build(LayerStorage::in_memory());
        let target_iri = iri(target);
        let val = synthesize_chain_witness(&layer, WitnessCategory::Declared, &target_iri, &prop)
            .expect("witness should be admissible");
        // The returned value carries the synthesised witness.
        match val {
            crate::nbe::val::Val::ChainWitness(k) => {
                assert_eq!(k.category, WitnessCategory::Declared);
                assert_eq!(k.iri, target_iri);
            }
            other => panic!("expected Val::ChainWitness, got {other:?}"),
        }
    }

    #[test]
    fn synthesize_chain_witness_fails_with_diagnostic_when_missing() {
        let layer = LayerBuilder::new("test", None).build(LayerStorage::in_memory());
        let target_iri = iri("urn:eigenius:example:unfounded");
        let prop = Exp::Sort(0);
        let err = synthesize_chain_witness(&layer, WitnessCategory::Declared, &target_iri, &prop)
            .expect_err("witness must miss when nothing admits it");
        // Diagnostic shape — names the predicate family, the IRI, what
        // the user needs to do.
        assert!(err.contains("IsDeclaredAs"), "diagnostic: {err}");
        assert!(err.contains(target_iri.as_str()), "diagnostic: {err}");
        assert!(
            err.contains("canonical_proposition"),
            "diagnostic should hint at canonical_proposition: {err}"
        );
        assert!(
            err.contains("JustifiedBy.declared"),
            "diagnostic should name the consuming constructor: {err}"
        );
    }

    #[test]
    fn synthesize_chain_witness_walks_parent_chain() {
        let mut parent = LayerBuilder::new("parent", None);
        let target = "urn:eigenius:example:thing";
        let prop = Exp::Sort(0);
        parent
            .add_resource(target_resource_with_canonical_prop(target, &prop))
            .unwrap();
        parent
            .add_resource(declaration_trace(
                target,
                "urn:eigenius:example:thing-decl-trace",
            ))
            .unwrap();
        let parent_layer = Arc::new(parent.build(LayerStorage::in_memory()));

        let child = LayerBuilder::new("child", Some(parent_layer.clone()));
        let child_layer = child.build(LayerStorage::in_memory());

        let target_iri = iri(target);
        assert!(
            synthesize_chain_witness(&child_layer, WitnessCategory::Declared, &target_iri, &prop,)
                .is_ok(),
            "synthesis must walk the parent chain to find the witness in parent layer"
        );
    }

    #[test]
    fn verified_witness_coerces_to_derived_at_lookup() {
        // D49 §4 coercion: VerifiedResource subclass_of DerivedResource
        // means an `IsVerifiedAs iri P` witness in the index makes
        // `IsDerivedAs iri P` lookups succeed via the lookup-time
        // coercion, even though the index doesn't carry the Derived key
        // directly.
        //
        // Setup: we *manually* construct a Layer whose witness index
        // contains a Verified key (since Phase 4's build_witness_index
        // doesn't emit VerificationTrace witnesses yet — that's Phase 7).
        let target = "urn:eigenius:example:proof";
        let prop = Exp::Sort(0);
        let verified_key =
            WitnessKey::from_exp(WitnessCategory::Verified, iri(target), &prop).unwrap();

        // Build a Layer normally (no traces). Then inject a witness via
        // the OnceLock's set() interface — this is test-only access; the
        // production path uses build_witness_index.
        let layer = LayerBuilder::new("test", None).build(LayerStorage::in_memory());
        let mut idx = std::collections::BTreeMap::new();
        idx.insert(verified_key.clone(), ());
        layer
            .chain_witness_index_for_test_set(idx)
            .expect("OnceLock not yet initialised in fresh layer");

        // Direct Verified lookup hits.
        assert!(lookup_chain_witness(&layer, &verified_key));

        // Coerced Derived lookup at the same (iri, prop_hash) also hits.
        let derived_key =
            WitnessKey::from_exp(WitnessCategory::Derived, iri(target), &prop).unwrap();
        assert!(
            lookup_chain_witness(&layer, &derived_key),
            "IsVerifiedAs should coerce to IsDerivedAs at lookup time per D49 §4"
        );

        // But a Declared lookup at the same prop does NOT coerce.
        let declared_key =
            WitnessKey::from_exp(WitnessCategory::Declared, iri(target), &prop).unwrap();
        assert!(
            !lookup_chain_witness(&layer, &declared_key),
            "IsVerifiedAs must not coerce to IsDeclaredAs (no such subclass relation)"
        );
    }
}
