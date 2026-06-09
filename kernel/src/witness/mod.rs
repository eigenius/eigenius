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

//! D49 — `ChainWitness` machinery.
//!
//! Soundness boundary for the D39 Reasoning institution: the four
//! `ChainWitness.IsXxAs : core:iri → Prop → Prop` predicate families are
//! consumed by the `JustifiedBy` indexed inductive's grounding constructors
//! to project the chain's existing class-membership + Trace-emission facts
//! into the type system. Witnesses are kernel-internal — ESL has no
//! constructor for them; the kernel synthesises inhabitants at
//! `JustifiedBy.declared` / `.observed` / `.derived` / `.verified`
//! type-check time by looking up a per-`Layer` witness index that the
//! Layer builds from its Trace resources.
//!
//! This module hosts the keying / hashing / category primitives. The
//! per-Layer index lives in [`crate::layer`] (see `witness_index.rs`); the
//! type-checker synthesis hook lives in [`crate::nbe::check`].
//!
//! Specification: `docs/design/d49-chainwitness-machinery.md` §3-§6.

use crate::nbe::term::Exp;
use crate::ontology::{eigon_cbor, Iri, Value};
use crate::program::eigentt_type_mirror::{encode_type, EncodeError};
use sha2::{Digest, Sha256};

/// Which of the four epistemic-category predicate families a witness
/// belongs to. The four families are independent — the kernel does not
/// silently coerce `IsObservedAs` to `IsDeclaredAs` even when the IRIs
/// match — but `IsVerifiedAs` propagates to `IsDerivedAs` per the
/// reflection ontology's `VerifiedResource subclass_of DerivedResource`
/// relation. The coercion is implemented at lookup time
/// (see `crate::layer::lookup_chain_witness`), not by populating both
/// keys in the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WitnessCategory {
    Declared,
    Observed,
    Derived,
    Verified,
}

impl WitnessCategory {
    /// Human-readable label, used in diagnostics when a witness lookup
    /// misses (`"no admitted IsDeclaredAs witness for iri X with
    /// proposition P"`).
    pub fn label(self) -> &'static str {
        match self {
            WitnessCategory::Declared => "IsDeclaredAs",
            WitnessCategory::Observed => "IsObservedAs",
            WitnessCategory::Derived => "IsDerivedAs",
            WitnessCategory::Verified => "IsVerifiedAs",
        }
    }
}

/// The lookup key for an admitted `ChainWitness` inhabitant.
///
/// Three fields keyed by deterministic, content-addressed values:
/// - `category` — which predicate family.
/// - `iri` — the IRI of the grounded chain resource.
/// - `prop_hash` — SHA-256 of the deterministic CBOR encoding of the
///   D47-encoded proposition. Hashing rather than storing the proposition
///   inline keeps the index a `BTreeMap`-of-fixed-size-keys (cheap parent
///   walk; cheap collision dedup); D47's codec output is canonical (its
///   inner JSON tree comes from `serde_json::Value`'s default
///   `BTreeMap`-sorted `Map`, and `value_to_cbor` walks it deterministically)
///   so two encodings of the same `Exp` hash to the same key.
///
/// The `(category, iri)` pair determines exactly one canonical proposition
/// per resource per D49 §4 / D39 §4.1's `canonical_proposition` semantics
/// (default `Asserts(iri)`; explicit value via the optional
/// `reflection:canonical_proposition` property; for `VerifiedResource`,
/// derived from the reified `VerifiedPropositionView`). Keeping
/// `prop_hash` in the key still matters: it surfaces "the
/// `JustifiedBy.declared` constructor was instantiated with the wrong
/// proposition for this IRI" as a type error at type-check time, rather
/// than silently admitting a witness for a mismatched proposition.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WitnessKey {
    pub category: WitnessCategory,
    pub iri: Iri,
    pub prop_hash: [u8; 32],
}

impl WitnessKey {
    /// Build a witness key from category, IRI, and proposition `Exp`.
    /// Computes the `prop_hash` via [`hash_proposition_exp`].
    pub fn from_exp(
        category: WitnessCategory,
        iri: Iri,
        proposition: &Exp,
    ) -> Result<Self, EncodeError> {
        Ok(Self {
            category,
            iri,
            prop_hash: hash_proposition_exp(proposition)?,
        })
    }

    /// Build a witness key from category, IRI, and a pre-encoded
    /// proposition `Value` (the D47 codec output — `Value::Json(...)`).
    /// Skips re-encoding, useful when the witness emitter already holds
    /// the encoded form (the trace path reads `canonical_proposition`
    /// directly from a chain resource).
    pub fn from_encoded(category: WitnessCategory, iri: Iri, encoded: &Value) -> Self {
        Self {
            category,
            iri,
            prop_hash: hash_proposition_value(encoded),
        }
    }
}

/// SHA-256 of the deterministic CBOR encoding of an `Exp`'s D47
/// representation. Combines D47's `encode_type` with
/// `eigon_cbor::serialize_value` to produce a fixed-size content hash.
///
/// The encoded JSON is alpha-canonicalized before CBOR encoding (see
/// [`alpha_canonicalize_proposition_json`]) so that propositions equal
/// up to binder renaming hash to the same key. Required because the
/// kernel's NbE readback freshens binder names (`Pi (c : T) => ...`
/// reads back as `Pi (G#0 : T) => ...`); without canonicalization, a
/// `canonical_proposition` stored on the chain with author-supplied
/// binder names would never match the synthesise-side hash.
pub fn hash_proposition_exp(proposition: &Exp) -> Result<[u8; 32], EncodeError> {
    let encoded = encode_type(proposition)?;
    Ok(hash_proposition_value(&encoded))
}

/// SHA-256 of the deterministic CBOR encoding of an already-D47-encoded
/// proposition `Value` (`Value::Json(...)`). Used when the witness
/// emitter reads `canonical_proposition` from a chain resource and has
/// the encoded form in hand.
///
/// Alpha-canonicalizes the JSON before hashing — see
/// [`hash_proposition_exp`] for why.
pub fn hash_proposition_value(encoded: &Value) -> [u8; 32] {
    let canonical = match encoded {
        Value::Json(j) => {
            let normalized = alpha_canonicalize_proposition_json(j);
            Value::Json(normalized)
        }
        // Non-JSON values aren't D47-encoded propositions; hash as-is so
        // pre-existing call sites (if any) stay byte-stable.
        other => other.clone(),
    };
    let bytes = eigon_cbor::serialize_value(&canonical);
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    hasher.finalize().into()
}

/// Rewrite a D47-encoded proposition JSON tree to use a canonical
/// scheme for binder names so that alpha-equivalent propositions hash
/// to the same value.
///
/// Walks the tree maintaining a stack of `(original_name,
/// canonical_name)` pairs; each `Pi` / `Sig` / `Lam` ctor pushes its
/// binder (canonical name = `"_b{depth}"` where depth is the binder's
/// position from the outside), and each `Var` ctor's name is rewritten
/// to the topmost matching canonical name on the stack (later
/// binders shadow earlier ones). Empty binder strings (`Patt::Unit`
/// encoded as `""`) preserve as-is and add no entry to the stack.
///
/// Free `Var`s (no matching binder on the stack) are preserved
/// unchanged — they're either author-level free variables that the
/// kernel's type-checker will reject independently, or references to
/// chain identifiers encoded via `ConstRef` rather than `Var`.
pub fn alpha_canonicalize_proposition_json(value: &serde_json::Value) -> serde_json::Value {
    let mut env: Vec<(String, String)> = Vec::new();
    canonicalize_inner(value, &mut env)
}

fn canonicalize_inner(v: &serde_json::Value, env: &mut Vec<(String, String)>) -> serde_json::Value {
    use serde_json::json;
    let obj = match v.as_object() {
        Some(o) => o,
        None => return v.clone(),
    };
    let ctor = obj.get("ctor").and_then(|c| c.as_str());
    let args = obj.get("args").and_then(|a| a.as_array());
    let (ctor, args) = match (ctor, args) {
        (Some(c), Some(a)) => (c, a),
        _ => return v.clone(),
    };
    match (ctor, args.len()) {
        ("Pi", 3) | ("Sig", 3) | ("Lam", 3) => {
            let binder = args[0].as_str().unwrap_or("").to_string();
            // Dom is evaluated in the *outer* scope (before this binder
            // is in scope), so canonicalize it first without pushing.
            let dom_canon = canonicalize_inner(&args[1], env);
            // Push the binder mapping for the body. Anonymous binders
            // (empty string) push an empty mapping so the depth counter
            // still advances — required so a later Var lookup sees the
            // right scoping even when intermediate binders are
            // anonymous.
            let depth = env.len();
            let canonical_binder = if binder.is_empty() {
                String::new()
            } else {
                format!("_b{depth}")
            };
            env.push((binder, canonical_binder.clone()));
            let body_canon = canonicalize_inner(&args[2], env);
            env.pop();
            json!({
                "ctor": ctor,
                "args": [canonical_binder, dom_canon, body_canon],
            })
        }
        ("Var", 1) => {
            let name = args[0].as_str().unwrap_or("");
            // Search the stack top-down (most recent binder wins for
            // shadowing). The level we read is the binder's depth from
            // the outside, so it's stable across the whole tree.
            let resolved = env
                .iter()
                .rev()
                .find(|(orig, _)| orig == name && !orig.is_empty())
                .map(|(_, canon)| canon.clone())
                .unwrap_or_else(|| name.to_string());
            json!({
                "ctor": "Var",
                "args": [resolved],
            })
        }
        _ => {
            // Other ctors: recurse on each arg without changing the env.
            let canon_args: Vec<serde_json::Value> =
                args.iter().map(|a| canonicalize_inner(a, env)).collect();
            json!({
                "ctor": ctor,
                "args": canon_args,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbe::term::{Exp, Patt};

    fn nat_iri() -> Iri {
        Iri::parse("urn:eigenius:example:Nat").unwrap()
    }

    fn ex_iri() -> Iri {
        Iri::parse("urn:eigenius:example:thing").unwrap()
    }

    #[test]
    fn category_label_round_trips() {
        for cat in [
            WitnessCategory::Declared,
            WitnessCategory::Observed,
            WitnessCategory::Derived,
            WitnessCategory::Verified,
        ] {
            assert!(cat.label().starts_with("Is"));
            assert!(cat.label().ends_with("As"));
        }
    }

    #[test]
    fn same_proposition_hashes_to_same_key() {
        let p1 = Exp::Sort(0);
        let p2 = Exp::Sort(0);
        let k1 = WitnessKey::from_exp(WitnessCategory::Declared, ex_iri(), &p1).unwrap();
        let k2 = WitnessKey::from_exp(WitnessCategory::Declared, ex_iri(), &p2).unwrap();
        assert_eq!(k1, k2);
        assert_eq!(k1.prop_hash, k2.prop_hash);
    }

    #[test]
    fn different_proposition_hashes_differ() {
        let p_prop = Exp::Sort(0); // Prop
        let p_set = Exp::Sort(1); // Set
        let k1 = WitnessKey::from_exp(WitnessCategory::Declared, ex_iri(), &p_prop).unwrap();
        let k2 = WitnessKey::from_exp(WitnessCategory::Declared, ex_iri(), &p_set).unwrap();
        assert_ne!(k1.prop_hash, k2.prop_hash);
    }

    #[test]
    fn different_category_distinct_keys() {
        let p = Exp::Sort(0);
        let k1 = WitnessKey::from_exp(WitnessCategory::Declared, ex_iri(), &p).unwrap();
        let k2 = WitnessKey::from_exp(WitnessCategory::Observed, ex_iri(), &p).unwrap();
        assert_ne!(k1, k2);
        assert_eq!(k1.prop_hash, k2.prop_hash); // hash same; category differs
    }

    #[test]
    fn different_iri_distinct_keys() {
        let p = Exp::Sort(0);
        let k1 = WitnessKey::from_exp(WitnessCategory::Declared, ex_iri(), &p).unwrap();
        let k2 = WitnessKey::from_exp(WitnessCategory::Declared, nat_iri(), &p).unwrap();
        assert_ne!(k1, k2);
    }

    #[test]
    fn from_exp_and_from_encoded_agree() {
        let p = Exp::Pi(Patt::Unit, Box::new(Exp::Sort(0)), Box::new(Exp::Sort(0)));
        let encoded = encode_type(&p).unwrap();
        let k_exp = WitnessKey::from_exp(WitnessCategory::Derived, ex_iri(), &p).unwrap();
        let k_enc = WitnessKey::from_encoded(WitnessCategory::Derived, ex_iri(), &encoded);
        assert_eq!(k_exp, k_enc);
    }

    #[test]
    fn key_is_ord_for_btreemap_use() {
        // Sanity: the BTreeMap-as-witness-index pattern in
        // kernel/src/layer/witness_index.rs needs WitnessKey: Ord.
        // Just confirm the impl exists by exercising it.
        let mut keys: std::collections::BTreeMap<WitnessKey, ()> = Default::default();
        let p = Exp::Sort(0);
        let k = WitnessKey::from_exp(WitnessCategory::Declared, ex_iri(), &p).unwrap();
        keys.insert(k.clone(), ());
        assert!(keys.contains_key(&k));
    }

    // --- Phase 2 — Val::ChainWitness equality (D49 §8) ---

    #[test]
    fn val_chain_witness_same_key_definitionally_equal() {
        use crate::nbe::check::eq_nf;
        use crate::nbe::val::Val;
        let p = Exp::Sort(0);
        let k1 = WitnessKey::from_exp(WitnessCategory::Declared, ex_iri(), &p).unwrap();
        let k2 = WitnessKey::from_exp(WitnessCategory::Declared, ex_iri(), &p).unwrap();
        let v1 = Val::ChainWitness(k1);
        let v2 = Val::ChainWitness(k2);
        assert!(
            eq_nf(0, &v1, &v2).is_ok(),
            "same-key ChainWitness values must be definitionally equal"
        );
    }

    #[test]
    fn val_chain_witness_different_iri_not_equal() {
        use crate::nbe::check::eq_nf;
        use crate::nbe::val::Val;
        let p = Exp::Sort(0);
        let k1 = WitnessKey::from_exp(WitnessCategory::Declared, ex_iri(), &p).unwrap();
        let k2 = WitnessKey::from_exp(WitnessCategory::Declared, nat_iri(), &p).unwrap();
        let v1 = Val::ChainWitness(k1);
        let v2 = Val::ChainWitness(k2);
        assert!(
            eq_nf(0, &v1, &v2).is_err(),
            "different-iri ChainWitness values must not be definitionally equal"
        );
    }

    #[test]
    fn val_chain_witness_vs_non_witness_not_equal() {
        use crate::nbe::check::eq_nf;
        use crate::nbe::val::Val;
        let p = Exp::Sort(0);
        let k = WitnessKey::from_exp(WitnessCategory::Declared, ex_iri(), &p).unwrap();
        let v_witness = Val::ChainWitness(k);
        let v_other = Val::Sort(0);
        assert!(
            eq_nf(0, &v_witness, &v_other).is_err(),
            "ChainWitness vs non-witness must not be equal"
        );
        assert!(
            eq_nf(0, &v_other, &v_witness).is_err(),
            "non-witness vs ChainWitness must not be equal (symmetric)"
        );
    }

    #[test]
    #[should_panic(expected = "ChainWitness")]
    fn val_chain_witness_readback_panics() {
        use crate::nbe::readback::readback_val;
        use crate::nbe::val::Val;
        let p = Exp::Sort(0);
        let k = WitnessKey::from_exp(WitnessCategory::Declared, ex_iri(), &p).unwrap();
        let v = Val::ChainWitness(k);
        // Witnesses never round-trip to surface syntax — readback panics.
        // This is the contract: any code path that would readback a
        // ChainWitness is a bug (it should be consumed at type-check time).
        let _ = readback_val(0, &v);
    }
}
