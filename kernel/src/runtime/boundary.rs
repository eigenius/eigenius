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

//! Kernel-side boundary check for substrate IO components per
//! [D26 §7.5](../../../../docs/design/d26-runtime-substrate.md).
//! Phase 18b.
//!
//! Conceptually this is the same check the substrate crate has in
//! `boundary.rs` — same three rules (mirror anchor ancestral; required
//! classes mirrored; mirrored classes unchanged) — but the kernel
//! version operates directly on `&Layer` rather than going through a
//! `ChainAccessor` trait. The substrate-side abstraction is good for
//! orchestrator-side use and synthetic-chain testing; the kernel side
//! has the real chain at hand and can avoid the indirection. The
//! duplication is small (~150 lines) and the two paths happen to be
//! the only two consumers, so a shared crate is overkill — revisit if
//! a third caller emerges.
//!
//! ## ⚠️ Keep this in sync with `crates/runtime-substrate/src/boundary.rs`
//!
//! Both files implement the same D26 §7.5 specification against
//! different chain-access primitives (this one against `&Layer`, the
//! substrate's against the `ChainAccessor` trait). When one changes,
//! the other almost certainly needs the matching change:
//!
//! - **Property IRIs** (the `PROP_*` constants): mirrored.
//! - **Three-rule order** (anchor ancestral → mirror present →
//!   class unchanged): identical.
//! - **Error variants**: substrate uses `RunError::*`; kernel uses
//!   `BoundaryError::*`. Same semantic shape, different names.
//! - **Empty-mirror short-circuit**: env without `mirror_dependency`
//!   returns Ok in both.
//!
//! The justification for the duplication is in the substrate-side
//! file's matching note. If you find yourself adding logic to one
//! and not the other, ask why — most additions to the boundary check
//! affect both.
//!
//! ## Layer-IRI convention
//!
//! `RuntimePackageMirror.source_layer` is declared as a string-form
//! IRI (`runtime-substrate-ontology.json` §5.4). This module adopts
//! the convention `urn:eigenius:layer:<sha256-hex>` — the layer's
//! content hash rendered lowercase hex, prefixed. Mirror generators
//! (Phase 19b/20b) and the kernel-side boundary check both emit /
//! parse this form.

use crate::layer::{Layer, LayerId};
use crate::ontology::iri::Iri;
use crate::ontology::resource::{Resource, Value};
use std::sync::Arc;
use thiserror::Error;

// Property IRIs the boundary check reads. Mirror the substrate-side
// constants in `crates/runtime-substrate/src/boundary.rs` — kept
// duplicated so the kernel's check has zero substrate dep.
const PROP_REQUIRES_ENVIRONMENT: &str = "urn:eigenius:runtime:requires_environment";
const PROP_REQUIRES_MIRROR_CLASSES: &str = "urn:eigenius:runtime:requires_mirror_classes";
const PROP_MIRROR_DEPENDENCY: &str = "urn:eigenius:runtime:mirror_dependency";
const PROP_SOURCE_LAYER: &str = "urn:eigenius:runtime:source_layer";
const PROP_MIRRORED_CLASSES: &str = "urn:eigenius:runtime:mirrored_classes";
const PROP_INPUT_TYPES: &str = "urn:eigenius:runtime:input_types";
const PROP_OUTPUT_TYPE: &str = "urn:eigenius:runtime:output_type";

const LAYER_IRI_PREFIX: &str = "urn:eigenius:layer:";

/// Failure modes of the kernel-side boundary check. Mirror the
/// substrate's `RunError` variants but stay kernel-internal — the
/// dispatch path that calls this maps these into whatever shape the
/// caller surfaces (today: a string error from `RemoteComponent`).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum BoundaryError {
    #[error("RuntimeEnvironment `{0}` not found in chain at the claim layer")]
    EnvNotFound(String),

    #[error("RuntimePackageMirror `{0}` not found in chain at the claim layer")]
    MirrorNotFound(String),

    #[error("argument is malformed: {0}")]
    Malformed(String),

    #[error("mirror anchor not ancestral: anchor {mirror_layer} is not an ancestor of or equal to claim {claim_layer}")]
    MirrorAnchorNotAncestral {
        mirror_layer: String,
        claim_layer: String,
    },

    #[error("missing mirror struct for class `{class_iri}`")]
    MissingMirrorStruct { class_iri: String },

    #[error("mirror version mismatch: class `{class_iri}` (mirror anchor: {mirror_layer}, claim: {claim_layer})")]
    MirrorVersionMismatch {
        class_iri: String,
        mirror_layer: String,
        claim_layer: String,
    },
}

/// Run the boundary check for a `RunRuntimeScript` invocation.
///
/// `script` is the resolved `RuntimeScript` resource being dispatched.
/// `claim_layer` is the layer the kernel is about to commit the
/// invocation's output against — typically the head of the
/// `ExecutionContext`.
pub fn check_run_script(script: &Resource, claim_layer: &Layer) -> Result<(), BoundaryError> {
    let env_iri = read_iri_property(script, PROP_REQUIRES_ENVIRONMENT)?;
    let required = read_iri_array(script, PROP_REQUIRES_MIRROR_CLASSES);
    run_check(&required, &env_iri, claim_layer)
}

/// Run the boundary check for a `CallRuntimeMethod` invocation.
///
/// `signature` is the resolved `RuntimeMethodSignature`. `env_iri`
/// is supplied by the caller — typically resolved from the signature's
/// `method_package` → package's environment, or passed explicitly by
/// the substrate facade.
pub fn check_call_method(
    signature: &Resource,
    env_iri: &Iri,
    claim_layer: &Layer,
) -> Result<(), BoundaryError> {
    let mut required = read_iri_array(signature, PROP_INPUT_TYPES);
    if let Some(out) = read_optional_iri_property(signature, PROP_OUTPUT_TYPE) {
        if !required.contains(&out) {
            required.push(out);
        }
    }
    run_check(&required, env_iri, claim_layer)
}

fn run_check(required: &[Iri], env_iri: &Iri, claim_layer: &Layer) -> Result<(), BoundaryError> {
    let env = claim_layer
        .resolve(env_iri)
        .ok_or_else(|| BoundaryError::EnvNotFound(env_iri.as_str().to_string()))?;

    // Envs without a mirror are valid (e.g. dev-path / test-runtime
    // environments without typed Eigon dispatch). Nothing to check.
    let mirror_iri = match read_optional_iri_property(&env, PROP_MIRROR_DEPENDENCY) {
        Some(iri) => iri,
        None => return Ok(()),
    };

    let mirror = claim_layer
        .resolve(&mirror_iri)
        .ok_or_else(|| BoundaryError::MirrorNotFound(mirror_iri.as_str().to_string()))?;

    let mirror_layer_iri = read_iri_property(&mirror, PROP_SOURCE_LAYER)?;
    let mirror_layer = find_layer_in_ancestry(claim_layer, &mirror_layer_iri).ok_or_else(|| {
        BoundaryError::MirrorAnchorNotAncestral {
            mirror_layer: mirror_layer_iri.as_str().to_string(),
            claim_layer: layer_iri(claim_layer).as_str().to_string(),
        }
    })?;

    let mirrored = read_iri_array(&mirror, PROP_MIRRORED_CLASSES);

    for class_iri in required {
        if !mirrored.contains(class_iri) {
            return Err(BoundaryError::MissingMirrorStruct {
                class_iri: class_iri.as_str().to_string(),
            });
        }
        let claim_def = claim_layer.resolve(class_iri);
        let mirror_def = mirror_layer.resolve(class_iri);
        if !resources_equal(&claim_def, &mirror_def) {
            return Err(BoundaryError::MirrorVersionMismatch {
                class_iri: class_iri.as_str().to_string(),
                mirror_layer: mirror_layer_iri.as_str().to_string(),
                claim_layer: layer_iri(claim_layer).as_str().to_string(),
            });
        }
    }

    Ok(())
}

/// Compute the layer IRI for a Layer using the
/// `urn:eigenius:layer:<sha256-hex>` convention. Always succeeds —
/// the IRI is constructed from the layer's content hash, which is
/// guaranteed valid hex.
pub fn layer_iri(layer: &Layer) -> Iri {
    Iri::parse(&format!("{LAYER_IRI_PREFIX}{}", layer.id()))
        .expect("layer IRI is well-formed by construction")
}

/// Walk `start`'s primary-parent chain looking for a layer whose
/// content-hash matches `target_iri`. Returns `None` if `target_iri`
/// is not a valid layer IRI or no ancestor matches.
fn find_layer_in_ancestry<'a>(start: &'a Layer, target_iri: &Iri) -> Option<&'a Layer> {
    let target_id = parse_layer_iri(target_iri)?;
    if start.id() == &target_id {
        return Some(start);
    }
    let mut cur = start.parent()?;
    loop {
        if cur.id() == &target_id {
            return Some(cur.as_ref());
        }
        cur = cur.parent()?;
    }
}

fn parse_layer_iri(iri: &Iri) -> Option<LayerId> {
    let s = iri.as_str().strip_prefix(LAYER_IRI_PREFIX)?;
    if s.len() != 64 {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (i, byte_chunk) in s.as_bytes().chunks_exact(2).enumerate() {
        let hi = hex_digit(byte_chunk[0])?;
        let lo = hex_digit(byte_chunk[1])?;
        bytes[i] = (hi << 4) | lo;
    }
    Some(LayerId(bytes))
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn resources_equal(a: &Option<Arc<Resource>>, b: &Option<Arc<Resource>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => **a == **b,
        (None, None) => true,
        _ => false,
    }
}

fn read_iri_property(r: &Resource, prop_iri: &str) -> Result<Iri, BoundaryError> {
    let prop = Iri::parse(prop_iri).expect("static IRI is well-formed");
    let value = r.get(&prop).ok_or_else(|| {
        BoundaryError::Malformed(format!("missing required property `{prop_iri}`"))
    })?;
    match value {
        Value::String(s) => Iri::parse(s)
            .map_err(|e| BoundaryError::Malformed(format!("malformed IRI in `{prop_iri}`: {e}"))),
        Value::ResourceRef(iri) => Ok(iri.clone()),
        _ => Err(BoundaryError::Malformed(format!(
            "property `{prop_iri}` has wrong type: expected IRI string"
        ))),
    }
}

fn read_optional_iri_property(r: &Resource, prop_iri: &str) -> Option<Iri> {
    let prop = Iri::parse(prop_iri).ok()?;
    match r.get(&prop)? {
        Value::String(s) => Iri::parse(s).ok(),
        Value::ResourceRef(iri) => Some(iri.clone()),
        _ => None,
    }
}

fn read_iri_array(r: &Resource, prop_iri: &str) -> Vec<Iri> {
    let prop = match Iri::parse(prop_iri) {
        Ok(i) => i,
        Err(_) => return vec![],
    };
    match r.get(&prop) {
        Some(v) => v.as_iri_array(),
        None => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::{LayerBuilder, LayerStorage};

    fn iri(s: &str) -> Iri {
        Iri::parse(s).unwrap()
    }

    fn class_resource(name: &str, marker: &str) -> Resource {
        let mut r = Resource::new(iri(name));
        r.set(
            iri("urn:eigenius:test:marker"),
            Value::String(marker.to_string()),
        );
        r
    }

    fn env_resource(env_iri: &str, mirror_iri: Option<&str>) -> Resource {
        let mut r = Resource::new(iri(env_iri));
        if let Some(m) = mirror_iri {
            r.set(iri(PROP_MIRROR_DEPENDENCY), Value::ResourceRef(iri(m)));
        }
        r
    }

    fn mirror_resource(
        mirror_iri: &str,
        source_layer_iri: &str,
        mirrored_classes: &[&str],
    ) -> Resource {
        let mut r = Resource::new(iri(mirror_iri));
        r.set(
            iri(PROP_SOURCE_LAYER),
            Value::String(source_layer_iri.to_string()),
        );
        r.set(
            iri(PROP_MIRRORED_CLASSES),
            Value::Array(
                mirrored_classes
                    .iter()
                    .map(|c| Value::ResourceRef(iri(c)))
                    .collect(),
            ),
        );
        r
    }

    fn script_resource(env_iri: &str, requires_classes: &[&str]) -> Resource {
        let mut r = Resource::new(iri("urn:eigenius:test:script:s1"));
        r.set(
            iri(PROP_REQUIRES_ENVIRONMENT),
            Value::ResourceRef(iri(env_iri)),
        );
        r.set(
            iri(PROP_REQUIRES_MIRROR_CLASSES),
            Value::Array(
                requires_classes
                    .iter()
                    .map(|c| Value::ResourceRef(iri(c)))
                    .collect(),
            ),
        );
        r
    }

    fn build_layer(name: &str, parent: Option<Arc<Layer>>, resources: &[Resource]) -> Arc<Layer> {
        let storage = LayerStorage::in_memory();
        let mut builder = LayerBuilder::new(name, parent);
        for r in resources {
            builder.add_resource(r.clone()).expect("add resource");
        }
        Arc::new(builder.build(storage))
    }

    /// Build a three-layer fixture: L0 defines the named classes; L1
    /// adds the env + mirror anchored to L0; L2 optionally redefines
    /// classes to test version-mismatch.
    ///
    /// This factoring sidesteps the fixed-point trap: putting a
    /// mirror inside the layer it references would change that
    /// layer's content hash (and thus its IRI), but the mirror's
    /// `source_layer` was already serialised with the pre-mirror
    /// hash. Anchoring to L0 from L1 keeps L0's hash stable.
    fn three_layer_chain(
        class_at_l0: &[(&str, &str)],
        class_at_l2: &[(&str, &str)],
        mirrored_classes: &[&str],
    ) -> Arc<Layer> {
        let l0_resources: Vec<Resource> = class_at_l0
            .iter()
            .map(|(name, marker)| class_resource(name, marker))
            .collect();
        let l0 = build_layer("l0", None, &l0_resources);
        let l0_iri = layer_iri(&l0);

        let l1 = build_layer(
            "l1",
            Some(Arc::clone(&l0)),
            &[
                env_resource(
                    "urn:eigenius:test:env:e1",
                    Some("urn:eigenius:test:mirror:m1"),
                ),
                mirror_resource(
                    "urn:eigenius:test:mirror:m1",
                    l0_iri.as_str(),
                    mirrored_classes,
                ),
            ],
        );

        let l2_resources: Vec<Resource> = class_at_l2
            .iter()
            .map(|(name, marker)| class_resource(name, marker))
            .collect();
        build_layer("l2", Some(Arc::clone(&l1)), &l2_resources)
    }

    /// L0 (class C v1) ⊏ L1 (env + mirror anchored to L0) ⊏ L2 (no
    /// changes); claim at L2 → boundary check passes.
    #[test]
    fn passes_when_chain_compatible_and_classes_unchanged() {
        let l2 = three_layer_chain(
            &[("urn:eigenius:test:class:C", "v1")],
            &[],
            &["urn:eigenius:test:class:C"],
        );
        let script = script_resource("urn:eigenius:test:env:e1", &["urn:eigenius:test:class:C"]);
        check_run_script(&script, &l2).expect("should pass");
    }

    /// Identical redefinition at L2 (same marker, different layer)
    /// should still pass — the equality check uses Resource value
    /// equality, not identity.
    #[test]
    fn passes_when_class_redefined_identically() {
        let l2 = three_layer_chain(
            &[("urn:eigenius:test:class:C", "v1")],
            &[("urn:eigenius:test:class:C", "v1")],
            &["urn:eigenius:test:class:C"],
        );
        let script = script_resource("urn:eigenius:test:env:e1", &["urn:eigenius:test:class:C"]);
        check_run_script(&script, &l2).expect("should pass");
    }

    /// L2 redefines C with a different marker → MirrorVersionMismatch
    /// pinpointing C.
    #[test]
    fn mirror_version_mismatch_when_class_redefined_in_descendant() {
        let l2 = three_layer_chain(
            &[("urn:eigenius:test:class:C", "v1")],
            &[("urn:eigenius:test:class:C", "v2")],
            &["urn:eigenius:test:class:C"],
        );
        let script = script_resource("urn:eigenius:test:env:e1", &["urn:eigenius:test:class:C"]);
        let err = check_run_script(&script, &l2).expect_err("expected MirrorVersionMismatch");
        match err {
            BoundaryError::MirrorVersionMismatch { class_iri, .. } => {
                assert_eq!(class_iri, "urn:eigenius:test:class:C");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    /// Mirror anchored to a layer not on the claim's chain →
    /// MirrorAnchorNotAncestral.
    #[test]
    fn mirror_anchor_not_ancestral() {
        let l0 = build_layer(
            "l0",
            None,
            &[
                class_resource("urn:eigenius:test:class:C", "v1"),
                env_resource("urn:eigenius:test:env:e1", Some("urn:eigenius:test:mirror:m1")),
                mirror_resource(
                    "urn:eigenius:test:mirror:m1",
                    "urn:eigenius:layer:0000000000000000000000000000000000000000000000000000000000000000",
                    &["urn:eigenius:test:class:C"],
                ),
            ],
        );

        let script = script_resource("urn:eigenius:test:env:e1", &["urn:eigenius:test:class:C"]);
        let err = check_run_script(&script, &l0).expect_err("expected MirrorAnchorNotAncestral");
        match err {
            BoundaryError::MirrorAnchorNotAncestral { mirror_layer, .. } => {
                assert_eq!(
                    mirror_layer,
                    "urn:eigenius:layer:0000000000000000000000000000000000000000000000000000000000000000"
                );
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    /// Required class not in mirror's mirrored_classes →
    /// MissingMirrorStruct pinpointing the missing class.
    #[test]
    fn missing_mirror_struct() {
        let l2 = three_layer_chain(
            &[("urn:eigenius:test:class:A", "v1")],
            &[],
            &["urn:eigenius:test:class:A"], // mirror covers A but script also asks for B
        );
        let script = script_resource(
            "urn:eigenius:test:env:e1",
            &["urn:eigenius:test:class:A", "urn:eigenius:test:class:B"],
        );
        let err = check_run_script(&script, &l2).expect_err("expected MissingMirrorStruct");
        match err {
            BoundaryError::MissingMirrorStruct { class_iri } => {
                assert_eq!(class_iri, "urn:eigenius:test:class:B");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    /// Env without a mirror → check passes (no work to do).
    #[test]
    fn passes_when_env_has_no_mirror() {
        let l0 = build_layer(
            "l0",
            None,
            &[env_resource("urn:eigenius:test:env:e1", None)],
        );

        let script = script_resource(
            "urn:eigenius:test:env:e1",
            &["urn:eigenius:test:class:NeverMirrored"],
        );
        check_run_script(&script, &l0).expect("should pass");
    }

    /// Layer-IRI round-trip: build a Layer, derive its IRI, parse it,
    /// confirm the LayerId round-trips.
    #[test]
    fn layer_iri_roundtrip() {
        let l0 = build_layer("l0", None, &[]);
        let iri_str = layer_iri(&l0);
        let parsed = parse_layer_iri(&iri_str).expect("parse");
        assert_eq!(parsed, *l0.id());
    }

    /// Malformed layer IRI (wrong prefix, wrong length, non-hex) →
    /// parse returns None.
    #[test]
    fn parse_layer_iri_rejects_malformed() {
        assert!(parse_layer_iri(&iri("urn:wrong:prefix:0123")).is_none());
        assert!(parse_layer_iri(&iri("urn:eigenius:layer:tooshort")).is_none());
        assert!(parse_layer_iri(&iri(
            "urn:eigenius:layer:0000000000000000000000000000000000000000000000000000000000000000zz"
        ))
        .is_none());
    }
}
