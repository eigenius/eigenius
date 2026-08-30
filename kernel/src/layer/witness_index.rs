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
//! **What is being decided here is whether to assert an axiom.** The `witness:Is*As` types have zero
//! constructors (`ontologies/justification/justification.esl:52`), so no term inhabits them and this
//! function is the only way one comes into existence — see `Val::ChainWitness` in `nbe/val.rs` for
//! the full anatomy. Consequently **this module is inside the TCB**: everything above a witness is
//! type-checked, the witness itself is postulated, and a wrong admission cannot be caught
//! downstream because an axiom has no proof to re-check.
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

/// D54: the `justification:Conclusion` class IRI and its `proposition`
/// property. Named here (rather than in `well_known`) because the D49
/// witness machinery is the one kernel site that is intrinsically
/// reasoning-aware — it builds the witnesses `justification:Certificate` consumes.
const REASONING_SENTENCE: &str = "urn:eigenius:justification:Conclusion";
const CONCLUSION_JUDGEMENT: &str = "urn:eigenius:justification:judgement";
/// The conclusion's optional PROOF judgement — `holds(logic, t, P)`. This, and
/// not the certificate judgement, is what establishes `Verified`.
const CONCLUSION_PROOF: &str = "urn:eigenius:justification:proof";

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
/// - **self-attesting** — the key's IRI *is* the resource. A committed `justification:Conclusion`
///   whose judgement carries a proof is `Verified` on its own IRI. Reached by
///   [`Layer::get_resource`], which is layer-local. A `reflection:InstitutionEmittedDerivation`
///   used to be `Derived` on its own IRI (D52); it now attests nothing, because a program's
///   output is not a ground — see `trace_category`.
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
/// True for the seven classes [`layer_admits_witness`] can emit from: the five Trace classes, a
/// `reflection:InstitutionEmittedDerivation`, and a `justification:Conclusion`. Stamped over a
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
/// All four grounding families are here (eigenius#200). `VerificationTrace` was absent until
/// `2026-08-21` on the reasoning that it would arrive with D49 §7's comorphism-reified
/// `VerifiedPropositionView`. That deferred the wrong half: the view is how a LEAN proof's
/// proposition reaches the trace, not what makes a `VerificationTrace` admit a witness. The trace
/// already names its target through `reflection:resource`, so `emit_from_trace` reads the target's
/// `canonical_proposition` for it exactly as it does for the other three — nothing about the
/// Verified category needs special handling here.
///
/// The consequence of the omission was a witness with no artifact: `emit_from_reasoning_sentence`
/// synthesised a Verified key straight from the sentence, so every Verified witness on every chain
/// was traceless, breaking D39 §5's invariant that the trace and the witness are two projections of
/// one validator event.
fn trace_category(class_iri: &str) -> Option<WitnessCategory> {
    match class_iri {
        wk::DECLARATION_TRACE => Some(WitnessCategory::Declared),
        wk::OBSERVATION_TRACE => Some(WitnessCategory::Observed),
        // A `ProgramTrace` grounds NOTHING. It records that a run happened —
        // provenance — and a computed claim does not rest on the fact that a
        // computation ran. It rests on the assertion that the plan denotes a
        // function `I -> O`, which is Declared by an accountable agent and which
        // no execution can establish, and on the inputs, which are Observed. The
        // composite `App(Declared(plan), Observed(inputs))` is built from those
        // two, so the run record is not a third ground.
        wk::PROGRAM_TRACE => None,
        wk::VERIFICATION_TRACE => Some(WitnessCategory::Verified),
        // An author recording that a program ran elsewhere (eigenius#205). A transcription
        // establishes only that someone asserts the run happened, so it is DECLARED, and
        // `declared_by` is required on the class, so the assertion always has an agent behind
        // it. This arm was the one place the old four-category split had the right instinct:
        // it already refused to call a run record a ground of its own kind. Now that
        // `ProgramTrace` grounds nothing either, the two agree.
        wk::EXTERNAL_EXECUTION_TRACE => Some(WitnessCategory::Declared),
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

/// D54: admit a `justification:Conclusion` as a `Verified` witness — but ONLY
/// on the strength of a proof term.
///
/// The conclusion carries up to two judgements, and they say different things:
///
/// - `justification:judgement` is `holds(kernel, c, Certificate(j, P))` — *a
///   checker verified the certificate c*. It does **not** say `P`. A
///   certificate records the grounds a claim rests on; it is not factive.
/// - `justification:proof` is `holds(logic, t, P)` — *a checker verified `t`
///   against `P` itself*. That is factive, and it is what `Verified` means.
///
/// **Only the second admits a witness.** Minting `Verified` from the first was
/// the substitution the two-layer separation exists to forbid: the witness is
/// what a later `Certificate.verified(iri, P)` consumes, so a conclusion
/// resting on nothing but `Declared("…")` laundered into a proof exactly one
/// citation downstream, and `is_fully_verified` answered true for it.
///
/// A conclusion with no proof term therefore admits NO witness here. That is a
/// deliberate tightening of D54 lemma citation: a lemma can be cited as
/// `verified` only if it was proved, not merely justified.
///
/// The hash must match what a citing certificate produces. A consumer supplies
/// `P` directly; `hash_proposition_exp` hashes the decoded `Exp`, so both sides
/// hash the same term — see
/// `a_projected_proposition_hashes_as_the_same_proposition_stored_flat`.
fn emit_from_reasoning_sentence(layer: &Layer, sentence: &Resource) -> Option<WitnessKey> {
    let sentence_iri = sentence.id().cloned()?;
    let proof_iri = Iri::parse(CONCLUSION_PROOF).ok()?;
    let stored = sentence.get(&proof_iri)?;
    let proof = crate::program::eigentt_type_mirror::decode_judgement(stored, layer).ok()?;

    // The proof's type IS the proposition — no certificate to unwrap. If it is
    // a `Certificate(...)`, the slot holds a certificate judgement rather than
    // a proof, and it establishes nothing about the proposition.
    if crate::program::eigentt_type_mirror::certificate_indices(&proof.typ).is_some() {
        tracing::warn!(
            { field::OPERATION } = operation::WITNESS_DECODE,
            { field::ERROR_KIND } = "proof_is_a_certificate",
            resource_iri = %sentence_iri,
            "justification:proof holds a certificate judgement, not a proof of the \
             proposition; no Verified witness admitted"
        );
        return None;
    }

    let prop_hash = hash_proposition_exp(&proof.typ).ok()?;
    Some(WitnessKey {
        category: WitnessCategory::Verified,
        iri: sentence_iri,
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
    let prop_hash = target_proposition_hash(layer, &target_iri, &target_resource)?;
    Some(WitnessKey {
        category,
        iri: target_iri,
        prop_hash,
    })
}

/// The proposition a trace's target canonically asserts, hashed.
///
/// Three slots can hold it, tried in order:
///
/// 1. `reflection:canonical_proposition` — the general slot.
/// 2. `justification:proposition` — where a `justification:Conclusion` keeps the same thing under a different
///    name. **Required for correctness, not convenience** (eigenius#200): the self-attesting path
///    [`emit_from_reasoning_sentence`] reads slot 2, so without this arm a `VerificationTrace`
///    targeting a sentence would fall through to slot 3 and key the witness against
///    `Asserts(sentence_iri)` — a DIFFERENT hash from the one the sentence itself emits, and the
///    one no certificate cites.
/// 3. the D39 §4.1 default `Asserts(target_iri)`.
fn target_proposition_hash(layer: &Layer, target_iri: &Iri, target: &Resource) -> Option<[u8; 32]> {
    if let Some(encoded) = Iri::parse(wk::CANONICAL_PROPOSITION)
        .ok()
        .and_then(|i| target.get(&i))
    {
        return hash_stored_proposition(layer, target_iri, encoded);
    }
    // A conclusion carries its proposition inside its judgement rather than in
    // a slot, so it is projected out — and it MUST be, for the reason slot 2
    // exists at all: without it a trace targeting a conclusion falls through to
    // `Asserts(iri)`, a different hash from the one the conclusion itself
    // emits, and no certificate cites that.
    if target
        .is_a()
        .iter()
        .any(|c| c.as_str() == REASONING_SENTENCE)
    {
        if let Some(stored) = Iri::parse(CONCLUSION_JUDGEMENT)
            .ok()
            .and_then(|i| target.get(&i))
        {
            if let Ok(j) = crate::program::eigentt_type_mirror::decode_judgement(stored, layer) {
                if let Some((_, prop)) =
                    crate::program::eigentt_type_mirror::certificate_indices(&j.typ)
                {
                    return hash_proposition_exp(prop).ok();
                }
            }
        }
    }
    default_asserts_proposition_hash(layer, target_iri)
}

/// Build the default proposition `Asserts(target_iri)` per D39 §4.1
/// and return its hash. Resolves `core:Asserts` from the layer chain,
/// constructs `Exp::InductiveType(asserts_decl, [Exp::LitString(target_iri)])`,
/// encodes via the D47 codec, and hashes.
///
/// **Both ends of the witness machinery use the same construction.**
/// When a future `justification:Certificate.declared(iri, Asserts(iri))` constructor
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
    // The declaration need only *exist* — D76 Phase B: the term names it rather
    // than carrying it, so the full decode this used to do was dropped into a slot
    // that no longer exists. The encoded form is unchanged (`ConstRef` + an `App`
    // spine either way), so the witness hash is too — `witness_hash_agreement` is
    // the gate on that.
    if !crate::program::ground::is_inductive_type(&asserts_resource) {
        return None;
    }
    let proposition = crate::nbe::term::Exp::const_applied(
        asserts_iri,
        Vec::new(),
        vec![crate::nbe::term::Exp::LitString(
            target_iri.as_str().to_string(),
        )],
    );
    crate::witness::hash_proposition_exp(&proposition).ok()
}

/// Public synthesis variant of [`default_asserts_proposition_hash`]
/// that returns the full `Exp` rather than the hash. Used by the
/// `synthesize_chain_witness` consumer site when the agent's
/// `justification:Certificate.declared` constructor doesn't carry an explicit
/// proposition (i.e. the consumer wants the default to compare
/// against). Same `Asserts(iri)` shape; same Exp; same hash.
pub fn default_asserts_proposition(
    layer: &Layer,
    target_iri: &Iri,
) -> Option<crate::nbe::term::Exp> {
    let asserts_iri = Iri::parse(wk::ASSERTS).ok()?;
    let asserts_resource = layer.resolve(&asserts_iri)?;
    if !crate::program::ground::is_inductive_type(&asserts_resource) {
        return None;
    }
    Some(crate::nbe::term::Exp::const_applied(
        asserts_iri,
        Vec::new(),
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
/// algorithm's lookup step.
///
/// **No coercion between categories.** A `check_layer_with_coercion` helper sat
/// here and let a `Derived`-category lookup succeed on a `Verified` entry at the
/// same `(iri, prop_hash)`, on the authority of the reflection ontology's
/// `VerifiedResource subclass_of DerivedResource`. It was the second laundering
/// path: P3 narrowed what MINTS a Verified witness, and this is what SPENT one —
/// a `derived(…)` citation satisfied by a proof-checked conclusion, so the
/// distinction between "a program produced this" and "the kernel verified this"
/// collapsed at the lookup. It also implemented a lattice over the categories
/// that the design rejects, and it did so as a match arm rather than by reading
/// `subclass_of`, so the ontology could not have disagreed with it. Gone with
/// the `Derived` category itself.
pub fn lookup_chain_witness(layer: &Layer, key: &WitnessKey) -> bool {
    if layer_admits_witness(layer, key) {
        return true;
    }
    let mut cursor = layer.parent().cloned();
    while let Some(parent) = cursor {
        if layer_admits_witness(&parent, key) {
            return true;
        }
        cursor = parent.parent().cloned();
    }
    false
}

/// **D49 §5 synthesis algorithm — Phase 6 foundation.** Look up a
/// `ChainWitness` inhabitant for `(category, iri, proposition)` and, on
/// hit, return a `Val::ChainWitness(key)` value the kernel's NbE checker
/// can use as the synthesised witness argument to a `justification:Certificate.*`
/// constructor. On miss, surface the precise diagnostic D49 §5
/// specifies — naming the missing predicate family, the IRI, and what
/// the chain needs to admit for this `justification:Certificate.*` constructor to
/// become well-typed.
///
/// This function is the kernel-side surface the D39 Reasoning
/// institution's `justification:Certificate` constructor type-checker calls into. The
/// integration site — where `check_infer` in `nbe/check.rs` recognises a
/// `justification:Certificate.declared` / `.observed` / `.derived` / `.verified`
/// constructor and dispatches here — lands during D39 implementation
/// (per D51 gap 3); this function is the stable contract that integration
/// can call against starting today.
///
/// `proposition` is the EigenTT `Exp` extracted from the constructor's
/// `P` argument at the call site. The key's `prop_hash` is computed via
/// D47 encoding + SHA-256 to match what the emit side of
/// [`layer_admits_witness`] produces.
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
             Asserts class is authored) before this justification:Certificate.{} constructor is well-typed",
            category.label(),
            iri,
            iri,
            match category {
                WitnessCategory::Declared => "declared",
                WitnessCategory::Observed => "observed",
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

    /// A committed `justification:Conclusion` carrying a PROOF — the only shape
    /// admitted as a `Verified` witness on its own IRI (D54 lemma citation).
    ///
    /// It used to carry only the certificate judgement, and that was enough.
    /// It is not any more, and the change is the point of P3: a certificate
    /// records grounds, a proof establishes the proposition, and only the
    /// second admits `Verified`. A fixture still built the old way would test
    /// a shape the emitter no longer honours.
    fn reasoning_sentence(sentence_iri: &str, prop: &Exp) -> Resource {
        let mut r = Resource::new(iri(sentence_iri));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(REASONING_SENTENCE.to_string())]),
        );
        // `holds(kernel, t, P)` — the proof's TYPE is the proposition itself,
        // with no certificate to unwrap. That is what makes it factive.
        let p = encode_type(prop).unwrap();
        let t = crate::program::eigentt_type_mirror::encode_type(&Exp::LitString(
            "urn:eigenius:test:proof-term".into(),
        ))
        .unwrap();
        let proof = crate::program::eigentt_type_mirror::encode_judgement(
            "urn:eigenius:eigentt:logic_kernel",
            &t,
            &p,
        )
        .unwrap();
        r.set(iri(CONCLUSION_PROOF), proof);
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
        let prop = Exp::sort(0);
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
            WitnessKey::from_exp(WitnessCategory::Declared, iri(target), &Exp::sort(0)).unwrap();
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
        let prop = Exp::sort(0);
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
        let other_prop = Exp::sort(1);
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
        let explicit_prop = Exp::sort(0); // Prop sort

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
        let prop = Exp::sort(0);
        assert!(
            is_witness_candidate(&declaration_trace(
                "urn:eigenius:example:t",
                "urn:eigenius:example:tr"
            )),
            "DeclarationTrace must be a candidate"
        );
        assert!(
            is_witness_candidate(&reasoning_sentence("urn:eigenius:example:s", &prop)),
            "justification:Conclusion must be a candidate (D54)"
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
    /// **P3's gate.** Written failing, then closed.
    ///
    /// A conclusion grounded only by a DECLARATION must not be admitted as a
    /// `Verified` witness. It used to be: the emitter read the certificate
    /// judgement, took the proposition out of `Certificate(j, P)`, and minted
    /// `WitnessCategory::Verified` without ever looking at `j` — the binding
    /// was literally `let (_j, prop) = …`.
    ///
    /// Why that is a soundness defect rather than a cosmetic one: the witness
    /// it mints is what a LATER conclusion's `Certificate.verified(iri, P)`
    /// consumes. So a claim resting on nothing but "an agent asserted it"
    /// launders into `Verified` one citation downstream, and
    /// `is_fully_verified` on the citing term answers true. That is the
    /// substitution of grounds for a proof that the two-layer separation
    /// exists to make inexpressible.
    ///
    /// `Judgement(kernel, c, Certificate(j, P))` says *a checker verified the
    /// certificate c*. It does NOT say `P`. Only `Judgement(L, t, P)` — a
    /// proof term checked against the proposition itself, which is what
    /// `justification:proof` carries — establishes `Verified`.
    ///
    /// Closed by keying the `Verified` witness off `justification:proof` —
    /// a proof of the proposition — rather than off the certificate judgement.
    #[test]
    fn a_declared_grounded_conclusion_is_not_admitted_as_verified() {
        use crate::program::eigentt_type_mirror::{
            certificate_type, encode_judgement, encode_type,
        };

        let head = std::sync::Arc::clone(crate::bootstrap::bootstrap().expect("bootstrap").head());
        let conclusion_iri = "urn:eigenius:test:p3:concl";
        let prop = Exp::sort(0);

        // The justification term is a bare DECLARATION — an agent asserted the
        // premise. Nothing here is proved.
        let j = encode_type(&Exp::InductiveCtor(
            iri("urn:eigenius:justification:Term"),
            "Declared".into(),
            vec![Exp::LitString("urn:eigenius:test:p3:premise".into())],
        ))
        .unwrap();
        let p = encode_type(&prop).unwrap();
        let typ = certificate_type(&j, &p).unwrap();
        let judgement = encode_judgement("urn:eigenius:eigentt:logic_kernel", &j, &typ).unwrap();

        let mut r = Resource::new(iri(conclusion_iri));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::String(REASONING_SENTENCE.to_string())]),
        );
        r.set(iri(CONCLUSION_JUDGEMENT), judgement);

        let mut b = LayerBuilder::new("p3_gate", Some(head));
        b.add_resource(r).unwrap();
        let layer = b.build(LayerStorage::in_memory());

        let verified =
            WitnessKey::from_exp(WitnessCategory::Verified, iri(conclusion_iri), &prop).unwrap();
        assert!(
            !lookup_chain_witness(&layer, &verified),
            "a conclusion whose only ground is Declared must NOT be admitted as Verified — \
             the witness it mints is what a later `Certificate.verified(iri, P)` consumes, so \
             admitting it launders a declaration into a proof one citation downstream"
        );
    }

    #[test]
    fn skip_hint_short_circuits_the_lookup() {
        let target = "urn:eigenius:example:thing";
        let prop = Exp::sort(0);
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
    /// The projection out of a judgement must hash IDENTICALLY to the same
    /// proposition stored flat.
    ///
    /// This is the one failure the collapse can produce silently. A citing
    /// certificate's `verified(iri, P)` supplies `P` directly, while the emit
    /// side now recovers `P` by walking `holds(kernel, c, Certificate(j, P))`.
    /// If those two ever disagree — a stray annotation, a different binder
    /// name, an encoding that does not round-trip — nothing errors: the
    /// witness-index lookup simply misses, no `IsVerifiedAs` is admitted, and
    /// a conclusion that should Hold reports an unsatisfied citation with no
    /// indication that the proposition was the problem.
    ///
    /// Asserting hash equality is what turns that into a test failure.
    #[test]
    fn a_projected_proposition_hashes_as_the_same_proposition_stored_flat() {
        use crate::program::eigentt_type_mirror::{
            certificate_indices, certificate_type, decode_judgement, encode_judgement, encode_type,
        };

        let head = std::sync::Arc::clone(crate::bootstrap::bootstrap().expect("bootstrap").head());
        let layer = LayerBuilder::new("projection", Some(head)).build(LayerStorage::in_memory());

        // Shapes with structure worth losing: binders, arrows, a literal.
        let cases: Vec<(&str, Exp)> = vec![
            ("bare sort", Exp::sort(0)),
            (
                "arrow",
                Exp::Arrow(Box::new(Exp::sort(0)), Box::new(Exp::sort(0))),
            ),
            (
                "pi with a named binder",
                Exp::Pi(
                    Patt::Var("x".into()),
                    Box::new(Exp::sort(1)),
                    Box::new(Exp::sort(0)),
                ),
            ),
        ];

        for (label, prop) in cases {
            // The check side: the proposition as a certificate would supply it.
            let flat = hash_proposition_exp(&prop).expect("flat proposition hashes");

            // The emit side: the same proposition, reached through a judgement.
            let p = encode_type(&prop).unwrap();
            let j = encode_type(&Exp::InductiveCtor(
                iri("urn:eigenius:justification:Term"),
                "Declared".into(),
                vec![Exp::LitString("urn:eigenius:test:premise".into())],
            ))
            .unwrap();
            let typ = certificate_type(&j, &p).expect("certificate type encodes");
            let stored = encode_judgement("urn:eigenius:eigentt:logic_kernel", &j, &typ)
                .expect("judgement encodes");

            let judgement = decode_judgement(&stored, &layer)
                .unwrap_or_else(|e| panic!("{label}: judgement must decode: {e}"));
            let (_, projected) = certificate_indices(&judgement.typ)
                .unwrap_or_else(|| panic!("{label}: judgement type must be a Certificate"));
            let via_judgement =
                hash_proposition_exp(projected).expect("projected proposition hashes");

            assert_eq!(
                flat, via_judgement,
                "{label}: a proposition projected out of a judgement must hash as the same \
                 proposition stored flat, or the emit and check sides drift and the witness \
                 silently fails to be admitted"
            );
        }
    }

    #[test]
    fn emit_and_check_sides_agree_on_the_hash() {
        use crate::nbe::env::Rho;
        use crate::nbe::eval::eval;
        use crate::nbe::readback::readback_val;
        use crate::program::eigentt_type_mirror::decode_type;

        let layer = layer_with_core_ontology();
        let s = || Exp::sort(0);
        let cls = |i: &str| Exp::EigonClass(iri(i));

        let cases: Vec<(&str, Exp)> = vec![
            ("bare sort", s()),
            ("Set", Exp::sort(1)),
            (
                "arrow — the negation shape",
                Exp::Arrow(Box::new(s()), Box::new(s())),
            ),
            (
                "pi with a named binder",
                Exp::Pi(Patt::Var("x".into()), Box::new(Exp::sort(1)), Box::new(s())),
            ),
            (
                "sigma — the `exists` binder",
                Exp::Sig(
                    Patt::Var("x0".into()),
                    Box::new(Exp::sort(1)),
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
        let lam = Exp::Lam(Patt::Var("x".into()), Box::new(Exp::sort(0)));
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
        let prop = Exp::sort(0);
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
        let prop = Exp::sort(0);
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
            err.contains("justification:Certificate.declared"),
            "diagnostic should name the consuming constructor: {err}"
        );
    }

    #[test]
    fn synthesize_chain_witness_walks_parent_chain() {
        let mut parent = LayerBuilder::new("parent", None);
        let target = "urn:eigenius:example:thing";
        let prop = Exp::sort(0);
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
    fn witness_categories_do_not_coerce_into_one_another() {
        // The categories are independent families. A `check_layer_with_coercion`
        // helper used to make a `Derived`-category lookup succeed on a `Verified`
        // entry at the same `(iri, prop_hash)`, justified by the reflection
        // ontology's `VerifiedResource subclass_of DerivedResource`. That was the
        // spend half of the laundering P3 closed the mint half of, and it is gone
        // along with the `Derived` category. What remains to assert is that no
        // OTHER pair coerces either — the property the removed helper's existence
        // made easy to lose sight of.
        //
        // A committed `justification:Conclusion` is admitted as a `Verified`
        // witness on its own IRI, so this runs against the real emission path.
        let target = "urn:eigenius:example:proof";
        let prop = Exp::sort(0);

        // Parented on the bootstrap: a conclusion's judgement names
        // `eigentt:logic_kernel` and `justification:Certificate` by reference,
        // and the emitter resolves both through the chain. A parent-less layer
        // could carry the old flat proposition (a bare `Sort`, resolving
        // nothing) but cannot carry a judgement.
        let head = std::sync::Arc::clone(crate::bootstrap::bootstrap().expect("bootstrap").head());
        let mut b = LayerBuilder::new("test", Some(head));
        b.add_resource(reasoning_sentence(target, &prop)).unwrap();
        let layer = b.build(LayerStorage::in_memory());

        // The witness that IS admitted.
        let verified_key =
            WitnessKey::from_exp(WitnessCategory::Verified, iri(target), &prop).unwrap();
        assert!(lookup_chain_witness(&layer, &verified_key));

        // Neither remaining category is reachable from it at the same
        // (iri, prop_hash).
        for category in [WitnessCategory::Declared, WitnessCategory::Observed] {
            let key = WitnessKey::from_exp(category, iri(target), &prop).unwrap();
            assert!(
                !lookup_chain_witness(&layer, &key),
                "IsVerifiedAs must not coerce to {} — the families are independent",
                category.label()
            );
        }
    }

    // ─── Environment-blindness of proposition identity (see
    //     docs/design/d75-fusing-eigentt-and-the-knowledge-graph.md §3.4) ──────────────────────

    /// A class resource requiring the listed properties.
    fn class_requiring(class_iri: &str, requires: &[&str]) -> Resource {
        let mut r = Resource::new(iri(class_iri));
        r.set(
            iri(wk::IS_A),
            Value::Array(vec![Value::ResourceRef(iri(wk::CLASS))]),
        );
        r.set(
            iri(wk::REQUIRES),
            Value::Array(
                requires
                    .iter()
                    .map(|p| Value::ResourceRef(iri(p)))
                    .collect(),
            ),
        );
        r
    }

    const SUBJECT_CLASS: &str = "urn:eigenius:example:Dog";

    /// Two versions of `Dog`, differing in *extension*. `WIDE` drops a required
    /// property, so strictly more things are Dogs under it.
    const NARROW: &[&str] = &["urn:eigenius:example:name", "urn:eigenius:example:owner"];
    const WIDE: &[&str] = &["urn:eigenius:example:name"];

    /// `Π(x : Dog). Prop` — a proposition quantifying over a class, so its
    /// meaning depends on what `Dog` is.
    fn quantified_over_subject_class() -> Exp {
        Exp::Pi(
            crate::nbe::term::Patt::Var("x".into()),
            Box::new(Exp::EigonClass(iri(SUBJECT_CLASS))),
            Box::new(Exp::sort(0)),
        )
    }

    #[test]
    fn redefining_a_class_does_not_change_the_hash_of_a_proposition_over_it() {
        // The defining layer is not an input to the hash: `hash_proposition_exp`
        // takes `&Exp`, and a class reference encodes as a bare `ConstRef(iri)`.
        // So a proposition quantifying over `Dog` hashes identically before and
        // after `Dog` is redefined — the term is unchanged while its meaning is
        // not.
        let prop = quantified_over_subject_class();
        let encoded = encode_type(&prop).unwrap();
        let owner = iri("urn:eigenius:example:claim");

        let mut b1 = LayerBuilder::new("dog-v1", None);
        b1.add_resource(class_requiring(SUBJECT_CLASS, NARROW))
            .unwrap();
        let l1 = Arc::new(b1.build(LayerStorage::in_memory()));

        let mut b2 = LayerBuilder::new("dog-v2", Some(Arc::clone(&l1)));
        b2.add_resource(class_requiring(SUBJECT_CLASS, WIDE))
            .unwrap();
        let l2 = Arc::new(b2.build(LayerStorage::in_memory()));

        // Precondition: the redefinition is real — `Dog` resolves differently
        // in the two layers. Without this the hash comparison proves nothing.
        assert_ne!(
            l1.resolve(&iri(SUBJECT_CLASS))
                .unwrap()
                .get(&iri(wk::REQUIRES)),
            l2.resolve(&iri(SUBJECT_CLASS))
                .unwrap()
                .get(&iri(wk::REQUIRES)),
            "test setup: Dog must differ between the two layers"
        );

        let h1 = hash_stored_proposition(&l1, &owner, &encoded)
            .expect("proposition must hash against dog-v1");
        let h2 = hash_stored_proposition(&l2, &owner, &encoded)
            .expect("proposition must hash against dog-v2");

        assert_eq!(
            h1, h2,
            "proposition identity is environment-blind: `Π(x : Dog). Prop` hashes the same \
             after Dog is redefined. This is the current behaviour, not the desired one — see \
             docs/design/d75-fusing-eigentt-and-the-knowledge-graph.md §3.4. If this assertion starts failing, the \
             environment has become part of proposition identity and §6.2 needs revisiting."
        );
    }

    #[test]
    fn witness_credit_survives_redefinition_of_a_class_the_proposition_quantifies_over() {
        // The module doc argues first-hit-wins is sound "because Layer
        // immutability means a once-admitted witness stays admitted in all
        // descendants". Immutability makes the *record* stable; it does not
        // make the *meaning* of what was recorded stable, because a descendant
        // can rebind a name the proposition mentions.
        //
        // The direction of the rebinding is what makes this unsound rather than
        // merely stale. `Dog` here is *widened* — a required property is
        // dropped, so more things are Dogs in v2 than in v1. `Π(x : Dog). P` is
        // therefore a strictly stronger claim in v2, and the credit was earned
        // against the weaker one. (Narrowing the class would shrink the domain
        // and leave the stale credit sound by accident, which is why this test
        // does not narrow.)
        let prop = quantified_over_subject_class();
        let target = "urn:eigenius:example:every-dog-claim";

        let mut b1 = LayerBuilder::new("credit-v1", None);
        b1.add_resource(class_requiring(SUBJECT_CLASS, NARROW))
            .unwrap();
        b1.add_resource(target_resource_with_canonical_prop(target, &prop))
            .unwrap();
        b1.add_resource(declaration_trace(
            target,
            "urn:eigenius:example:every-dog-claim-decl-trace",
        ))
        .unwrap();
        let l1 = Arc::new(b1.build(LayerStorage::in_memory()));

        let key = WitnessKey::from_exp(WitnessCategory::Declared, iri(target), &prop).unwrap();
        assert!(
            layer_admits_witness(&l1, &key),
            "test setup: the witness must be admitted against Dog-v1"
        );

        // A descendant widens the class the proposition quantifies over.
        let mut b2 = LayerBuilder::new("credit-v2", Some(Arc::clone(&l1)));
        b2.add_resource(class_requiring(SUBJECT_CLASS, WIDE))
            .unwrap();
        let l2 = Arc::new(b2.build(LayerStorage::in_memory()));

        assert!(
            lookup_chain_witness(&l2, &key),
            "current behaviour: credit granted under the narrower Dog is still found from a \
             layer where Dog is wider, so `Π(x : Dog). P` is now a stronger claim than the one \
             that earned the credit. Nothing rechecks the proposition against the rebinding. \
             See docs/design/d75-fusing-eigentt-and-the-knowledge-graph.md §3.4."
        );
    }
}
