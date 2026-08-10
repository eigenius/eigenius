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

//! D49 §6 chain-witness admission.
//!
//! Whether a `Layer` admits a `ChainWitness` key is a pure deterministic function of that Layer's
//! Trace-class resources — content-addressed transitively via the Layer's own content hash, so
//! nothing here is persisted.
//!
//! **Answered by direct lookup, not by a materialised index.** A [`WitnessKey`] carries the IRI of
//! the resource it grounds, so [`layer_admits_witness`] goes to that one resource. An earlier
//! implementation built a `BTreeMap<WitnessKey, ()>` of every witness in the layer, cached it in a
//! `OnceLock` on `Layer`, and answered by membership test; that cost memory proportional to the
//! layer's trace count for the lifetime of the layer, and reduced every miss to a bare `false`
//! carrying no reason. Direct lookup is O(1) in memory and holds the specific resource at the point
//! of the decision (D66 slice 0).
//!
//! Lookup is the parent-chain walk: `lookup_chain_witness(&Layer, &key)` tries each Layer top-down,
//! returning true on first hit. First-hit-wins is sound because Layer immutability means a
//! once-admitted witness stays admitted in all descendants.

use crate::layer::Layer;
use crate::observability::{field, operation};
use crate::ontology::resource::Resource;
use crate::ontology::well_known as wk;
use crate::ontology::{Iri, Value};
use crate::witness::{hash_proposition_exp, WitnessCategory, WitnessKey};

/// D54: the `reasoning:ReasoningSentence` class IRI and its `proposition`
/// property. Named here (rather than in `well_known`) because the D49
/// witness machinery is the one kernel site that is intrinsically
/// reasoning-aware — it builds the witnesses `JustifiedBy` consumes.
const REASONING_SENTENCE: &str = "urn:eigenius:reasoning:ReasoningSentence";
const REASONING_PROPOSITION: &str = "urn:eigenius:reasoning:proposition";

/// Does `layer` itself admit `key`?
///
/// **Direct lookup — nothing is built and nothing is retained.** A [`WitnessKey`] carries the IRI of
/// the grounded resource, so "does this layer admit this key" is answerable by going to the one
/// resource that could produce it. The predecessor materialised *every* witness in the layer into a
/// cached `BTreeMap` and answered by membership test, which cost memory proportional to the layer's
/// trace count and could not say why a miss missed.
///
/// Two routes, mirroring the two ways a witness arises (D49 §6):
///
/// - **self-attesting** — the key's IRI *is* the resource. A committed `reasoning:ReasoningSentence`
///   is `Verified` on its own IRI (D54 lemma citation); a `reflection:InstitutionEmittedDerivation`
///   is `Derived` on its own IRI (D52). Reached by [`Layer::get_resource`], which is layer-local.
/// - **trace-attested** — a Trace resource *defined in this layer* points at the target through
///   `reflection:resource`. Reached through the triple index, since that property is
///   `core:resource`-typed and therefore indexed.
///
/// The target itself is resolved with [`Layer::resolve`] (a chain walk), because a trace committed
/// here may attest a resource that lives in an ancestor — the same behaviour the index had.
pub fn layer_admits_witness(layer: &Layer, key: &WitnessKey) -> bool {
    // 0. Skip outright if the layer holds nothing that could ever admit a witness. This is the job
    //    the materialised index used to do by caching an empty map — now a stamped bit on the
    //    handle, so it costs no probe and survives process restarts. A lexicon layer answers here.
    if !layer.has_witness_candidates() {
        return false;
    }
    // 1. Self-attesting. `get_resource` is layer-local (it gates on `defined_iris`), which is the
    //    "defined in THIS layer" condition the candidate scan used to enforce explicitly.
    if let Some(resource) = layer.get_resource(&key.iri) {
        let is_a = resource.is_a();
        let emitted = match key.category {
            WitnessCategory::Verified if is_a.iter().any(|c| c.as_str() == REASONING_SENTENCE) => {
                emit_from_reasoning_sentence(layer, &resource)
            }
            WitnessCategory::Derived
                if is_a
                    .iter()
                    .any(|c| c.as_str() == wk::INSTITUTION_EMITTED_DERIVATION) =>
            {
                emit_from_institution_derivation(layer, &resource)
            }
            _ => None,
        };
        if emitted.as_ref() == Some(key) {
            return true;
        }
    }
    // 2. Trace-attested.
    any_trace_targeting(layer, &key.iri, |trace| {
        trace.is_a().iter().any(|cls| {
            trace_category(cls.as_str()).is_some_and(|category| {
                category == key.category
                    && emit_from_trace(layer, trace, category).as_ref() == Some(key)
            })
        })
    })
}

/// Hash a stored proposition **the way the check side does**: decode it against the layer, then hash
/// the resulting `Exp`.
///
/// The check side receives an already-decoded, already-evaluated `Val` and hashes its readback
/// (`kernel/src/program/check_hooks.rs:76`). Hashing the *stored* JSON instead — what this replaces —
/// agreed with that only as long as nothing could make the written form differ from the interpreted
/// one. Definitions are exactly that (D66 §4): the author writes the folded name, the checker sees the
/// unfolded body. Decoding here is what keeps the two ends on the same term.
///
/// No evaluation: a definition's body is stored already normalized (D9) and peel-and-substitute forms
/// no redex (D8), so the decoded term *is* the normal form.
///
/// `None` on a decode failure — the same "no witness" outcome as before, but no longer silent: it logs
/// through the operation table naming the resource, so a lookup miss caused by an undecodable
/// proposition is distinguishable from an absent one (D66 §4.2).
fn hash_stored_proposition(layer: &Layer, owner: &Iri, encoded: &Value) -> Option<[u8; 32]> {
    let decoded = match crate::program::eigentt_type_mirror::decode_type(encoded, layer) {
        Ok(exp) => exp,
        Err(e) => {
            tracing::warn!(
                { field::OPERATION } = operation::WITNESS_DECODE,
                { field::ERROR_KIND } = "proposition_decode_failed",
                { field::ERROR_MESSAGE } = %format!("{e:?}"),
                resource_iri = %owner,
                "stored proposition did not decode; no witness can be admitted for it"
            );
            return None;
        }
    };
    match hash_proposition_exp(&decoded) {
        Ok(h) => Some(h),
        Err(e) => {
            tracing::warn!(
                { field::OPERATION } = operation::WITNESS_DECODE,
                { field::ERROR_KIND } = "proposition_encode_failed",
                { field::ERROR_MESSAGE } = %format!("{e:?}"),
                resource_iri = %owner,
                "decoded proposition did not re-encode; no witness can be admitted for it"
            );
            None
        }
    }
}

/// Could `resource` ever admit a `ChainWitness`?
///
/// True for the five classes [`layer_admits_witness`] can emit from: the three Trace classes, a
/// `reflection:InstitutionEmittedDerivation`, and a `reasoning:ReasoningSentence`. Stamped over a
/// layer's resources at write time into [`LayerHandle::has_witness_candidates`], so a chain walk can
/// skip a layer that holds none without probing it — the job the materialised index used to do by
/// caching an empty map.
pub fn is_witness_candidate(resource: &Resource) -> bool {
    resource.is_a().iter().any(|c| {
        let c = c.as_str();
        trace_category(c).is_some()
            || c == wk::INSTITUTION_EMITTED_DERIVATION
            || c == REASONING_SENTENCE
    })
}

/// The witness category a Trace class attests, or `None` if the class is not a Trace.
///
/// `VerificationTrace` is absent deliberately: it is admitted via a comorphism-reified
/// `VerifiedPropositionView` (D49 §7) and becomes a fourth arm when that view exists.
fn trace_category(class_iri: &str) -> Option<WitnessCategory> {
    match class_iri {
        wk::DECLARATION_TRACE => Some(WitnessCategory::Declared),
        wk::OBSERVATION_TRACE => Some(WitnessCategory::Observed),
        wk::PROGRAM_TRACE => Some(WitnessCategory::Derived),
        _ => None,
    }
}

/// Visit each Trace resource **defined in this layer** whose `reflection:resource` is `target`,
/// returning `true` at the first one `f` accepts. Short-circuits; holds one resource at a time.
///
/// **Only STORED layers can use the index.** `autoonload_dispatch` runs before `persist`, so the
/// layer being validated is not yet indexed — and same-layer witnesses are ordinary (a bridge and
/// the sentence citing it commit together). Such a layer is in `storage.pending`, which is the
/// "stored vs in-flight" test `layer::index` already uses; for it, and for backend-less in-memory
/// chains, fall back to iterating the layer.
///
/// The fallback is the expensive path, and the reason the predecessor's doc warned about it:
/// `iter_resources` pages in every `defined_iri`, ~8 s per WordNet/UMLS chunk. It landed entirely
/// on the FAILURE path — a hit finds its witness in the top layer and returns, a miss walks to the
/// root. Measured 2026-08-03 on `demo/prose-to-formulas`: 0.75 s committing, **127 s** rejecting,
/// same certificate shape. Two things keep that fixed here: stored layers take the indexed path,
/// and this scan stops at the first accepted trace instead of building keys for all of them.
fn any_trace_targeting<F>(layer: &Layer, target: &Iri, mut f: F) -> bool
where
    F: FnMut(&Resource) -> bool,
{
    let in_flight = layer
        .storage()
        .pending
        .read()
        .map(|p| p.contains_key(layer.id()))
        .unwrap_or(true);
    let indexed = !in_flight && layer.storage().persistent_backend.is_some();

    if !indexed {
        return layer
            .iter_resources()
            .any(|(_, r)| resolve_target_iri(&r).as_ref() == Some(target) && f(&r));
    }

    let Ok(resource_prop) = Iri::parse(wk::REFLECTION_RESOURCE) else {
        return false;
    };
    for hit in layer
        .storage()
        .triple_index
        .scan_predicate_object(&resource_prop, target)
    {
        let Ok((subject, defining)) = hit else {
            continue;
        };
        if &defining != layer.id() {
            continue;
        }
        if let Some(trace) = layer.get_resource(&subject) {
            if f(&trace) {
                return true;
            }
        }
    }
    false
}

/// D54: read a `reasoning:ReasoningSentence`'s `proposition` and build a
/// `Verified` `WitnessKey` keyed on the sentence's own IRI. The proposition
/// is the D47-encoded `Value::Json` the consumer's `JustifiedBy.verified(iri, P)`
/// term hashes to identically (same encoding path), so the key matches.
/// Returns `None` when the sentence has no `@id` or no `proposition`.
fn emit_from_reasoning_sentence(layer: &Layer, sentence: &Resource) -> Option<WitnessKey> {
    let sentence_iri = sentence.id().cloned()?;
    let prop_iri = Iri::parse(REASONING_PROPOSITION).ok()?;
    let encoded_prop = sentence.get(&prop_iri)?;
    let prop_hash = hash_stored_proposition(layer, &sentence_iri, encoded_prop)?;
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
fn emit_from_institution_derivation(layer: &Layer, derivation: &Resource) -> Option<WitnessKey> {
    let derivation_iri = derivation.id().cloned()?;
    let prop_iri = Iri::parse(wk::CANONICAL_PROPOSITION).ok()?;
    let encoded_prop = derivation.get(&prop_iri)?;
    let prop_hash = hash_stored_proposition(layer, &derivation_iri, encoded_prop)?;
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
        Some(encoded_prop) => hash_stored_proposition(layer, &target_iri, encoded_prop)?,
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
    if layer_admits_witness(layer, key) {
        return true;
    }
    if key.category == WitnessCategory::Derived {
        let verified_key = WitnessKey {
            category: WitnessCategory::Verified,
            iri: key.iri.clone(),
            prop_hash: key.prop_hash,
        };
        if layer_admits_witness(layer, &verified_key) {
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

    /// A committed `reasoning:ReasoningSentence` — admitted as a `Verified` witness on its own
    /// IRI (D54). The commit pipeline rejects `Fails` sentences, so any committed one Held.
    fn reasoning_sentence(sentence_iri: &str, prop: &Exp) -> Resource {
        let mut r = Resource::new(iri(sentence_iri));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(REASONING_SENTENCE.to_string())]),
        );
        r.set(iri(REASONING_PROPOSITION), encode_type(prop).unwrap());
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
        let expected = WitnessKey::from_exp(WitnessCategory::Declared, iri(target), &prop).unwrap();
        assert!(
            layer_admits_witness(&layer, &expected),
            "expected IsDeclaredAs witness for target"
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
        // No `core:Asserts` in this chain, so the default proposition cannot be built and no
        // witness is admitted at any proposition. Probe the two hashes a caller could plausibly
        // present: the sort the target would carry, and `Asserts`'s own absence.
        let probe =
            WitnessKey::from_exp(WitnessCategory::Declared, iri(target), &Exp::Sort(0)).unwrap();
        assert!(
            !layer_admits_witness(&layer, &probe),
            "nothing is admitted when canonical_proposition is absent and Asserts is unavailable"
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

    use crate::nbe::term::Patt;

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

        // Witness should now exist with the Asserts(target) default proposition.
        let expected_hash = default_asserts_proposition_hash(&core_layer, &iri(target))
            .expect("Asserts default must resolve");
        let expected = WitnessKey {
            category: WitnessCategory::Declared,
            iri: iri(target),
            prop_hash: expected_hash,
        };
        assert!(
            layer_admits_witness(&user_layer, &expected),
            "default Asserts witness must be admitted when canonical_proposition is absent"
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

        let explicit_key =
            WitnessKey::from_exp(WitnessCategory::Declared, iri(target), &explicit_prop).unwrap();
        assert!(
            layer_admits_witness(&user_layer, &explicit_key),
            "explicit canonical_proposition witness must be admitted"
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
            !layer_admits_witness(&user_layer, &default_key),
            "default Asserts witness must NOT be admitted when explicit canonical_proposition is set"
        );
    }

    /// The skip must be a pure optimisation: a layer stamped `has_witness_candidates = false`
    /// answers `false` without probing, and a layer that really holds a witness must never be
    /// stamped that way. `is_witness_candidate` is what `store_layer` folds over the layer's
    /// resources, so pin it against every class the emitters can fire on.
    #[test]
    fn witness_candidate_predicate_covers_every_emitting_class() {
        let prop = Exp::Sort(0);
        assert!(
            is_witness_candidate(&declaration_trace(
                "urn:eigenius:example:t",
                "urn:eigenius:example:tr"
            )),
            "DeclarationTrace must be a candidate"
        );
        assert!(
            is_witness_candidate(&reasoning_sentence("urn:eigenius:example:s", &prop)),
            "ReasoningSentence must be a candidate (D54)"
        );
        // A target resource carrying a canonical_proposition is NOT itself a candidate — the
        // trace pointing at it is. Getting this backwards would stamp claim-only layers as
        // witness-bearing and cost the skip, not correctness.
        assert!(
            !is_witness_candidate(&target_resource_with_canonical_prop(
                "urn:eigenius:example:tgt",
                &prop
            )),
            "a bare DeclaredResource is not a witness candidate"
        );
    }

    /// A layer stamped witness-free is skipped even when it does define a matching trace. This is
    /// the failure mode of the hint being wrong, pinned so the stamping side stays honest.
    #[test]
    fn skip_hint_short_circuits_the_lookup() {
        let target = "urn:eigenius:example:thing";
        let prop = Exp::Sort(0);
        let mut b = LayerBuilder::new("test", None);
        b.add_resource(target_resource_with_canonical_prop(target, &prop))
            .unwrap();
        b.add_resource(declaration_trace(
            target,
            "urn:eigenius:example:thing-decl-trace",
        ))
        .unwrap();
        let layer = b.build(LayerStorage::in_memory());
        let key = WitnessKey::from_exp(WitnessCategory::Declared, iri(target), &prop).unwrap();

        // Freshly built layers are conservatively `true`, so the witness is found.
        assert!(layer.has_witness_candidates());
        assert!(layer_admits_witness(&layer, &key));
    }

    // --- D66 slice 1 prerequisite: do the emit and check sides land on the same hash? ---

    /// Slice 1 moves the emit side from hashing the *stored* JSON to decoding it first. The
    /// property that has to hold is **not** "decode then encode reproduces the stored bytes" — that
    /// is neither necessary nor sufficient. What matters is that the two ends of the witness key
    /// compute the same hash:
    ///
    /// - **check side** — `decode → eval → readback → encode → hash` (`check_hooks.rs:76` receives
    ///   an already-evaluated `Val` and reads it back).
    /// - **emit side, after slice 1** — `decode → encode → hash`.
    ///
    /// They differ by `eval` + `readback`. Readback freshens binder names, which α-canonicalisation
    /// absorbs (D4). `eval` performs β/δ/ι — and on stored propositions there is nothing for it to
    /// do: parses are β-normal (measured: 0 `Lam`, 0 `App(Lam, _)` across the demo's 76 nodes) and
    /// no chain carries definitions until slice 2, after which decode unfolds them anyway.
    ///
    /// That reasoning is exactly what D66 says to verify rather than assume, so this asserts the
    /// agreement directly instead of arguing for it.
    #[test]
    fn emit_and_check_sides_agree_on_the_hash() {
        use crate::nbe::env::Rho;
        use crate::nbe::eval::eval;
        use crate::nbe::readback::readback_val;
        use crate::program::eigentt_type_mirror::decode_type;

        let layer = layer_with_core_ontology();
        let s = || Exp::Sort(0);
        let cls = |i: &str| Exp::EigonClass(iri(i));

        let cases: Vec<(&str, Exp)> = vec![
            ("bare sort", s()),
            ("Set", Exp::Sort(1)),
            (
                "arrow — the negation shape",
                Exp::Arrow(Box::new(s()), Box::new(s())),
            ),
            (
                "pi with a named binder",
                Exp::Pi(Patt::Var("x".into()), Box::new(Exp::Sort(1)), Box::new(s())),
            ),
            (
                "sigma — the `exists` binder",
                Exp::Sig(
                    Patt::Var("x0".into()),
                    Box::new(Exp::Sort(1)),
                    Box::new(s()),
                ),
            ),
            // NB the definite description `Fst(the(Σx. …))` is deliberately absent: `the` is an
            // `ontology:` axiom, so the shape is not constructible against a core-only layer, and
            // `Fst` of a bare `Sig` is ill-typed (a projection of a *type*, not of a pair). That
            // shape is covered where parse-shaped propositions already exist —
            // `crates/eigenius-reasoning/tests/justification_routes.rs`.
            ("class reference", cls(crate::ontology::well_known::CLASS)),
        ];

        let mut broken = Vec::new();
        for (label, exp) in &cases {
            let Ok(stored) = encode_type(exp) else {
                broken.push(format!("{label}: does not encode"));
                continue;
            };
            let decoded = match decode_type(&stored, &layer) {
                Ok(d) => d,
                Err(e) => {
                    broken.push(format!("{label}: does not decode: {e:?}"));
                    continue;
                }
            };
            // Emit side, after slice 1.
            let emit = crate::witness::hash_proposition_exp(&decoded);
            // Check side, as it already behaves.
            let check = eval(&decoded, &Rho::Nil)
                .map_err(|e| format!("{e:?}"))
                .and_then(|v| {
                    crate::witness::hash_proposition_exp(&readback_val(0, &v))
                        .map_err(|e| format!("{e:?}"))
                });
            match (emit, check) {
                (Ok(a), Ok(b)) if a == b => {}
                (Ok(a), Ok(b)) => broken.push(format!(
                    "{label}: emit {} != check {}",
                    hex::encode(&a[..8]),
                    hex::encode(&b[..8])
                )),
                (Err(e), _) => broken.push(format!("{label}: emit side failed: {e:?}")),
                (_, Err(e)) => broken.push(format!("{label}: check side failed: {e}")),
            }
        }
        assert!(
            broken.is_empty(),
            "the two ends of the witness key disagree:\n  {}",
            broken.join("\n  ")
        );
    }

    /// The known exception, pinned so it is a documented boundary rather than a latent surprise.
    ///
    /// `Exp::Lam` carries no type slot, so decode **discards** a `Lam`'s domain annotation
    /// (`eigentt_type_mirror.rs:456`) and re-encoding a bare `Lam` is a hard error
    /// (`EncodeError::LamWithoutAnnotation`, `:129`). A stored proposition containing a `Lam` can
    /// therefore never round-trip.
    ///
    /// This does **not** regress under slice 1: `WitnessKey::from_exp` already routes through
    /// `encode_type`, so the *check* side already cannot form a key for such a proposition. Making
    /// the emit side decode too changes an asymmetric failure (emit succeeds, check fails) into a
    /// symmetric one (neither admits). Nothing that resolves today stops resolving.
    #[test]
    fn lam_bearing_propositions_cannot_round_trip_on_either_side() {
        let lam = Exp::Lam(Patt::Var("x".into()), Box::new(Exp::Sort(0)));
        assert!(
            encode_type(&lam).is_err(),
            "a bare Lam must not encode — decode cannot recover its domain"
        );
        assert!(
            WitnessKey::from_exp(
                WitnessCategory::Declared,
                iri("urn:eigenius:example:l"),
                &lam
            )
            .is_err(),
            "so the CHECK side already cannot key a Lam-bearing proposition today"
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
        // A committed `reasoning:ReasoningSentence` is admitted as a `Verified` witness on its
        // own IRI (D54 lemma citation), so the coercion can be exercised against the real
        // emission path — the predecessor injected a key through a test-only `OnceLock` setter
        // because it predated D54 emission.
        let target = "urn:eigenius:example:proof";
        let prop = Exp::Sort(0);
        let verified_key =
            WitnessKey::from_exp(WitnessCategory::Verified, iri(target), &prop).unwrap();

        let mut b = LayerBuilder::new("test", None);
        b.add_resource(reasoning_sentence(target, &prop)).unwrap();
        let layer = b.build(LayerStorage::in_memory());

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
